use std::collections::{BTreeMap, BTreeSet};

use reap_core::{PositionMarginMode, Side};
use reap_storage::FillRecord;
use reap_strategy::InstrumentConfig;
use reap_venue::RemoteFill;
use reap_venue::okx::OkxBillMarginMode;

use super::support::{close_abs, expected_bill_margin_mode, instrument, is_lower_sha256, issue};
use super::{
    BoundEconomicSources, EconomicIssueSource, EconomicReconciliationFailure,
    EconomicReconciliationOptions, IssueSink, JournalAuthoritativeAccountSnapshot,
    JournalDerivativePnlEvidence, JournalFillObservation, JournalRuntimeSession,
    JournalTradeEvidence, PositionBasis,
};

pub(super) fn validate_runtime_sessions<'a>(
    sources: &'a BoundEconomicSources,
    failures: &mut BTreeSet<EconomicReconciliationFailure>,
    issues: &mut IssueSink,
) -> Vec<&'a JournalRuntimeSession> {
    let mut seen = BTreeSet::new();
    let mut valid = Vec::new();
    for session in &sources.runtime_sessions {
        let identity_valid = session.started_at_ms > 0
            && is_runtime_session_id(&session.session_id)
            && !session.account_id.is_empty()
            && session.account_id.trim() == session.account_id
            && sources.config.account(&session.account_id).is_some()
            && !session.strategy_name.is_empty()
            && session.strategy_name == sources.config.strategy.strategy_name
            && session.config_fingerprint == sources.config_fingerprint
            && is_lower_sha256(&session.config_fingerprint)
            && is_lower_sha256(&session.account_identity_sha256);
        if !identity_valid {
            issues.push(
                EconomicReconciliationFailure::InvalidOrDuplicateRuntimeSessions,
                issue(
                    EconomicIssueSource::Journal,
                    None,
                    None,
                    None,
                    "runtime_session",
                    "configured account/strategy and valid session/config/account identities",
                    &format!(
                        "line {}, started_at={}, session_id={}, account={}",
                        session.line, session.started_at_ms, session.session_id, session.account_id
                    ),
                    "journal contains a malformed or foreign runtime-session boundary",
                ),
                failures,
            );
            continue;
        }
        if !seen.insert((session.account_id.clone(), session.session_id.clone())) {
            issues.push(
                EconomicReconciliationFailure::InvalidOrDuplicateRuntimeSessions,
                issue(
                    EconomicIssueSource::Journal,
                    None,
                    None,
                    None,
                    "runtime_session",
                    "one session-start record per account/session id",
                    &format!("duplicate at line {}", session.line),
                    "journal contains a duplicate runtime-session boundary",
                ),
                failures,
            );
            continue;
        }
        valid.push(session);
    }
    valid
}

fn is_runtime_session_id(value: &str) -> bool {
    !value.is_empty()
        && value.len() <= 32
        && value
            .bytes()
            .all(|byte| byte.is_ascii_digit() || (b'a'..=b'f').contains(&byte))
}

pub(super) fn runtime_session_for_line<'a>(
    sessions: &[&'a JournalRuntimeSession],
    account_id: &str,
    line: u64,
) -> Option<&'a JournalRuntimeSession> {
    sessions
        .iter()
        .copied()
        .filter(|session| session.account_id == account_id && session.line < line)
        .max_by_key(|session| session.line)
}

pub(super) fn build_journal_trade_evidence(
    sources: &BoundEconomicSources,
    runtime_sessions: &[&JournalRuntimeSession],
    options: &EconomicReconciliationOptions,
    failures: &mut BTreeSet<EconomicReconciliationFailure>,
    issues: &mut IssueSink,
) -> BTreeMap<(String, String), Vec<JournalTradeEvidence>> {
    enum TimelineEvent<'a> {
        Snapshot(&'a JournalAuthoritativeAccountSnapshot),
        Fill(&'a JournalFillObservation),
    }

    impl TimelineEvent<'_> {
        fn line(&self) -> u64 {
            match self {
                Self::Snapshot(snapshot) => snapshot.line,
                Self::Fill(fill) => fill.line,
            }
        }
    }

    let mut timeline = sources
        .authoritative_account_snapshots
        .iter()
        .map(TimelineEvent::Snapshot)
        .chain(sources.journal_fills.iter().map(TimelineEvent::Fill))
        .collect::<Vec<_>>();
    timeline.sort_by_key(TimelineEvent::line);
    let mut verified_fills = BTreeMap::<(String, String), Vec<&RemoteFill>>::new();
    for fill in &sources.fills {
        verified_fills
            .entry((fill.symbol.clone(), fill.fill_id.clone()))
            .or_default()
            .push(fill);
    }

    let mut state_session_id = None::<String>;
    let mut positions = BTreeMap::<String, PositionBasis>::new();
    let mut by_key = BTreeMap::<(String, String), Vec<JournalTradeEvidence>>::new();
    let mut seen_fill_keys = BTreeSet::new();

    for event in timeline {
        match event {
            TimelineEvent::Snapshot(snapshot) => {
                if snapshot.account_id != sources.account_id {
                    continue;
                }
                let Some(session) =
                    runtime_session_for_line(runtime_sessions, &sources.account_id, snapshot.line)
                else {
                    push_invalid_snapshot_issue(
                        snapshot,
                        "same-account runtime session before the snapshot",
                        "missing",
                        failures,
                        issues,
                    );
                    state_session_id = None;
                    positions.clear();
                    continue;
                };
                if session.account_identity_sha256 != sources.account_identity_sha256 {
                    push_invalid_snapshot_issue(
                        snapshot,
                        "runtime session with the exact collected account identity",
                        &session.account_identity_sha256,
                        failures,
                        issues,
                    );
                    state_session_id = None;
                    positions.clear();
                    continue;
                }
                if snapshot.event_ts_ms == 0 || snapshot.event_ts_ms != snapshot.update_ts_ms {
                    push_invalid_snapshot_issue(
                        snapshot,
                        "matching positive record/update timestamps",
                        &format!(
                            "record={}, update={}",
                            snapshot.event_ts_ms, snapshot.update_ts_ms
                        ),
                        failures,
                        issues,
                    );
                    state_session_id = None;
                    positions.clear();
                    continue;
                }

                let mut snapshot_positions = BTreeMap::new();
                let mut invalid = None;
                for position in &snapshot.positions {
                    let configured = instrument(&sources.config, &position.symbol);
                    let owned = sources
                        .config
                        .account_for_symbol(&position.symbol)
                        .is_some_and(|account| account.id == sources.account_id);
                    let margin_mode_valid = if position.qty == 0.0 {
                        true
                    } else {
                        matches!(
                            (
                                expected_bill_margin_mode(
                                    &sources.config,
                                    &sources.account_id,
                                    &position.symbol,
                                ),
                                position.margin_mode,
                            ),
                            (
                                Some(OkxBillMarginMode::Cross),
                                Some(PositionMarginMode::Cross)
                            ) | (
                                Some(OkxBillMarginMode::Isolated),
                                Some(PositionMarginMode::Isolated)
                            )
                        )
                    };
                    if position.symbol.is_empty()
                        || !position.qty.is_finite()
                        || !position.avg_price.is_finite()
                        || (position.qty != 0.0
                            && (!owned
                                || !margin_mode_valid
                                || !configured.is_some_and(|instrument| {
                                    instrument.kind.is_derivative() && position.avg_price > 0.0
                                })))
                    {
                        invalid = Some(format!(
                            "invalid position {} qty={} avgPx={}",
                            position.symbol, position.qty, position.avg_price
                        ));
                        break;
                    }
                    if snapshot_positions
                        .insert(position.symbol.clone(), position)
                        .is_some()
                    {
                        invalid = Some(format!("duplicate position {}", position.symbol));
                        break;
                    }
                }
                if let Some(invalid) = invalid {
                    push_invalid_snapshot_issue(
                        snapshot,
                        "unique finite configured derivative positions with positive avgPx when non-zero",
                        &invalid,
                        failures,
                        issues,
                    );
                    state_session_id = None;
                    positions.clear();
                    continue;
                }

                positions.clear();
                for configured in sources.config.instruments_for_account(&sources.account_id) {
                    if !configured.kind.is_derivative() {
                        continue;
                    }
                    let (quantity, avg_price) =
                        snapshot_positions
                            .get(&configured.symbol)
                            .map_or((0.0, 0.0), |position| {
                                if position.qty == 0.0 {
                                    (0.0, 0.0)
                                } else {
                                    (position.qty, position.avg_price)
                                }
                            });
                    positions.insert(
                        configured.symbol.clone(),
                        PositionBasis {
                            quantity,
                            avg_price,
                            snapshot_line: snapshot.line,
                            snapshot_time_ms: snapshot.event_ts_ms,
                        },
                    );
                }
                state_session_id = Some(session.session_id.clone());
            }
            TimelineEvent::Fill(observation) => {
                if observation.fill.account_id.as_deref() != Some(sources.account_id.as_str()) {
                    continue;
                }
                let fill_key = (
                    observation.fill.symbol.clone(),
                    observation.fill.fill_id.clone(),
                );
                if observation.fill.fill_id.is_empty() || !seen_fill_keys.insert(fill_key.clone()) {
                    issues.push(
                        EconomicReconciliationFailure::TradeJournalFillMismatches,
                        issue(
                            EconomicIssueSource::Journal,
                            None,
                            Some(&observation.fill.symbol),
                            (!observation.fill.fill_id.is_empty())
                                .then_some(observation.fill.fill_id.as_str()),
                            "journal_fill_identity",
                            "unique non-empty (symbol, fill_id) for the account",
                            &format!("duplicate or empty at line {}", observation.line),
                            "critical fill journal contains an ambiguous trade identity",
                        ),
                        failures,
                    );
                    positions.remove(&observation.fill.symbol);
                    by_key
                        .entry(fill_key)
                        .or_default()
                        .push(JournalTradeEvidence {
                            observation: observation.clone(),
                            derivative: None,
                        });
                    continue;
                }
                let session = runtime_session_for_line(
                    runtime_sessions,
                    &sources.account_id,
                    observation.line,
                );
                if state_session_id.as_deref() != session.map(|session| session.session_id.as_str())
                {
                    state_session_id = None;
                    positions.clear();
                }
                let derivative_instrument = instrument(&sources.config, &observation.fill.symbol)
                    .filter(|instrument| instrument.kind.is_derivative());
                let derivative = derivative_instrument.and_then(|instrument| {
                    let session = session?;
                    let basis = *positions.get(&observation.fill.symbol)?;
                    let [exchange_fill] = verified_fills.get(&fill_key)?.as_slice() else {
                        return None;
                    };
                    if basis.snapshot_time_ms >= observation.fill.ts_ms
                        || exchange_fill.ts_ms != observation.fill.ts_ms
                        || exchange_fill.side != observation.fill.side
                        || !close_abs(
                            exchange_fill.price,
                            observation.fill.price,
                            options.tolerances.price_abs,
                        )
                        || !close_abs(
                            exchange_fill.qty,
                            observation.fill.qty,
                            options.tolerances.quantity_abs,
                        )
                    {
                        return None;
                    }
                    let calculation = apply_derivative_fill(
                        basis,
                        &observation.fill,
                        instrument,
                        options.tolerances.quantity_abs,
                    )?;
                    positions.insert(
                        observation.fill.symbol.clone(),
                        PositionBasis {
                            quantity: calculation.post_quantity,
                            avg_price: calculation.post_avg_price,
                            ..basis
                        },
                    );
                    Some(JournalDerivativePnlEvidence {
                        fill_line: observation.line,
                        runtime_session_id: session.session_id.clone(),
                        runtime_session_start_line: session.line,
                        basis,
                        close_quantity: calculation.close_quantity,
                        post_quantity: calculation.post_quantity,
                        post_avg_price: calculation.post_avg_price,
                        expected_sub_type: calculation.expected_sub_type,
                        expected_pnl: calculation.expected_pnl,
                    })
                });
                if derivative_instrument.is_some() && derivative.is_none() {
                    positions.remove(&observation.fill.symbol);
                }
                by_key
                    .entry(fill_key)
                    .or_default()
                    .push(JournalTradeEvidence {
                        observation: observation.clone(),
                        derivative,
                    });
            }
        }
    }
    by_key
}

pub(super) struct DerivativeFillCalculation {
    pub(super) close_quantity: f64,
    pub(super) post_quantity: f64,
    pub(super) post_avg_price: f64,
    pub(super) expected_sub_type: String,
    pub(super) expected_pnl: f64,
}

pub(super) fn apply_derivative_fill(
    basis: PositionBasis,
    fill: &FillRecord,
    instrument: &InstrumentConfig,
    quantity_tolerance: f64,
) -> Option<DerivativeFillCalculation> {
    if !basis.quantity.is_finite()
        || !basis.avg_price.is_finite()
        || !fill.price.is_finite()
        || fill.price <= 0.0
        || !fill.qty.is_finite()
        || fill.qty <= 0.0
        || !instrument.contract_value.is_finite()
        || instrument.contract_value <= 0.0
    {
        return None;
    }
    let pre_quantity = if basis.quantity.abs() <= quantity_tolerance {
        0.0
    } else {
        basis.quantity
    };
    if pre_quantity != 0.0 && basis.avg_price <= 0.0 {
        return None;
    }
    let delta = match fill.side {
        Side::Buy => fill.qty,
        Side::Sell => -fill.qty,
    };
    let closes_position = pre_quantity != 0.0 && pre_quantity.signum() != delta.signum();
    let close_quantity = if closes_position {
        pre_quantity.abs().min(fill.qty)
    } else {
        0.0
    };
    let expected_sub_type = match (closes_position, pre_quantity.is_sign_positive(), fill.side) {
        (true, true, Side::Sell) => "5",
        (true, false, Side::Buy) => "6",
        (false, _, Side::Buy) => "3",
        (false, _, Side::Sell) => "4",
        _ => return None,
    }
    .to_string();
    let expected_pnl = if close_quantity == 0.0 {
        0.0
    } else if instrument.kind.is_inverse() {
        pre_quantity.signum()
            * (1.0 / basis.avg_price - 1.0 / fill.price)
            * close_quantity
            * instrument.contract_value
    } else {
        pre_quantity.signum()
            * (fill.price - basis.avg_price)
            * close_quantity
            * instrument.contract_value
    };

    let raw_post_quantity = pre_quantity + delta;
    let post_quantity = if raw_post_quantity.abs() <= quantity_tolerance {
        0.0
    } else {
        raw_post_quantity
    };
    let post_avg_price = if post_quantity == 0.0 {
        0.0
    } else if pre_quantity == 0.0 || pre_quantity.signum() != post_quantity.signum() {
        fill.price
    } else if post_quantity.abs() < pre_quantity.abs() {
        basis.avg_price
    } else if instrument.kind.is_inverse() {
        let base_value = pre_quantity.abs() / basis.avg_price + fill.qty / fill.price;
        if base_value <= 0.0 || !base_value.is_finite() {
            return None;
        }
        post_quantity.abs() / base_value
    } else {
        (basis.avg_price * pre_quantity.abs() + fill.price * fill.qty) / post_quantity.abs()
    };
    if !expected_pnl.is_finite() || !post_avg_price.is_finite() || post_avg_price < 0.0 {
        return None;
    }
    Some(DerivativeFillCalculation {
        close_quantity,
        post_quantity,
        post_avg_price,
        expected_sub_type,
        expected_pnl,
    })
}

fn push_invalid_snapshot_issue(
    snapshot: &JournalAuthoritativeAccountSnapshot,
    expected: &str,
    observed: &str,
    failures: &mut BTreeSet<EconomicReconciliationFailure>,
    issues: &mut IssueSink,
) {
    issues.push(
        EconomicReconciliationFailure::InvalidAuthoritativeAccountSnapshots,
        issue(
            EconomicIssueSource::Journal,
            None,
            None,
            None,
            "authoritative_account_snapshot",
            expected,
            &format!("line {}: {observed}", snapshot.line),
            "journaled REST account snapshot cannot establish an opening-cost basis",
        ),
        failures,
    );
}
