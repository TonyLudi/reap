use std::collections::BTreeSet;

use reap_venue::okx::{OkxBill, OkxInstrumentType};

use super::position_basis::runtime_session_for_line;
use super::support::{
    close_abs, expected_bill_margin_mode, instrument, issue, issue_for_bill, margin_mode_name,
    push_bill_issue,
};
use super::{
    BoundEconomicSources, EconomicIssueSource, EconomicReconciliationCounts,
    EconomicReconciliationFailure, EconomicReconciliationOptions, FundingFormulaSample, IssueSink,
    JournalFundingSettlement, JournalMarkPriceObservation, JournalPositionObservation,
    JournalRuntimeSession,
};
use crate::LiveConfig;

pub(super) fn validate_funding_settlements<'a>(
    sources: &'a BoundEconomicSources,
    runtime_sessions: &[&JournalRuntimeSession],
    options: &EconomicReconciliationOptions,
    counts: &mut EconomicReconciliationCounts,
    failures: &mut BTreeSet<EconomicReconciliationFailure>,
    issues: &mut IssueSink,
) -> Vec<&'a JournalFundingSettlement> {
    let mut seen = BTreeSet::new();
    let mut valid = Vec::new();
    let relevant_begin = options
        .begin_ms
        .saturating_sub(options.maximum_funding_bill_delay_ms);
    for settlement in &sources.settlements {
        let configured_swap = instrument(&sources.config, &settlement.symbol)
            .is_some_and(|instrument| instrument.kind.is_swap());
        if configured_swap
            && (relevant_begin..=options.end_ms).contains(&settlement.funding_time_ms)
        {
            counts.funding_settlements_relevant += 1;
        }
        if settlement.symbol.is_empty()
            || settlement.funding_time_ms == 0
            || !settlement.rate.is_finite()
            || settlement.event_ts_ms == 0
            || settlement.event_ts_ms < settlement.funding_time_ms
            || settlement.event_ts_ms - settlement.funding_time_ms
                > options.maximum_funding_bill_delay_ms
        {
            issues.push(
                EconomicReconciliationFailure::InvalidOrDuplicateFundingSettlements,
                issue(
                    EconomicIssueSource::Journal,
                    None,
                    Some(&settlement.symbol),
                    None,
                    "funding_settlement",
                    "non-empty symbol, finite rate, and observation inside the post-settlement delay",
                    &format!(
                        "line {}, event_ts={}, funding_time={}, rate={}",
                        settlement.line,
                        settlement.event_ts_ms,
                        settlement.funding_time_ms,
                        settlement.rate
                    ),
                    "journal contains an invalid settled funding observation",
                ),
                failures,
            );
            continue;
        }
        let session_id =
            runtime_session_for_line(runtime_sessions, &sources.account_id, settlement.line)
                .map_or("legacy", |session| session.session_id.as_str());
        let key = (
            session_id.to_string(),
            settlement.symbol.clone(),
            settlement.funding_time_ms,
        );
        if !seen.insert(key) {
            issues.push(
                EconomicReconciliationFailure::InvalidOrDuplicateFundingSettlements,
                issue(
                    EconomicIssueSource::Journal,
                    None,
                    Some(&settlement.symbol),
                    None,
                    "funding_settlement",
                    "one normalized settlement per runtime session/symbol/time",
                    &format!("duplicate at line {}", settlement.line),
                    "journal funding deduplication did not produce a unique settlement",
                ),
                failures,
            );
            continue;
        }
        valid.push(settlement);
    }
    valid
}

pub(super) fn validate_funding_mark_prices<'a>(
    sources: &'a BoundEconomicSources,
    runtime_sessions: &[&JournalRuntimeSession],
    failures: &mut BTreeSet<EconomicReconciliationFailure>,
    issues: &mut IssueSink,
) -> Vec<&'a JournalMarkPriceObservation> {
    let mut seen = BTreeSet::new();
    let mut valid = Vec::new();
    for observation in &sources.mark_price_observations {
        if observation.symbol.is_empty()
            || observation.event_ts_ms == 0
            || !observation.price.is_finite()
            || observation.price <= 0.0
        {
            issues.push(
                EconomicReconciliationFailure::InvalidOrDuplicateFundingMarks,
                issue(
                    EconomicIssueSource::Journal,
                    None,
                    Some(&observation.symbol),
                    None,
                    "mark_price",
                    "non-empty symbol and positive finite exchange-time mark",
                    &format!(
                        "line {}, event_ts={}, price={}",
                        observation.line, observation.event_ts_ms, observation.price
                    ),
                    "journal contains an invalid mark-price observation",
                ),
                failures,
            );
            continue;
        }
        let session_id =
            runtime_session_for_line(runtime_sessions, &sources.account_id, observation.line)
                .map_or("legacy", |session| session.session_id.as_str());
        let key = (
            session_id.to_string(),
            observation.symbol.clone(),
            observation.event_ts_ms,
        );
        if !seen.insert(key) {
            issues.push(
                EconomicReconciliationFailure::InvalidOrDuplicateFundingMarks,
                issue(
                    EconomicIssueSource::Journal,
                    None,
                    Some(&observation.symbol),
                    None,
                    "mark_price",
                    "one normalized mark per runtime session/symbol/exchange timestamp",
                    &format!("duplicate at line {}", observation.line),
                    "journal mark-price deduplication did not produce a unique observation",
                ),
                failures,
            );
            continue;
        }
        valid.push(observation);
    }
    valid
}

#[allow(clippy::too_many_arguments)]
pub(super) fn validate_funding_bill(
    bill: &OkxBill,
    settlements: &[&JournalFundingSettlement],
    runtime_sessions: &[&JournalRuntimeSession],
    position_observations: &[JournalPositionObservation],
    mark_price_observations: &[&JournalMarkPriceObservation],
    config: &LiveConfig,
    config_fingerprint: &str,
    account_identity_sha256: &str,
    account_id: &str,
    options: &EconomicReconciliationOptions,
    counts: &mut EconomicReconciliationCounts,
    failures: &mut BTreeSet<EconomicReconciliationFailure>,
    issues: &mut IssueSink,
) -> Option<FundingFormulaSample> {
    let before = issues.total;
    let Some(instrument) = instrument(config, &bill.symbol) else {
        push_bill_issue(
            failures,
            issues,
            EconomicReconciliationFailure::InvalidFundingBills,
            bill,
            "instId",
            "configured swap instrument",
            &bill.symbol,
            "funding bill references an instrument outside the exact live config",
        );
        return None;
    };
    if !instrument.kind.is_swap() || bill.instrument_type != Some(OkxInstrumentType::Swap) {
        push_bill_issue(
            failures,
            issues,
            EconomicReconciliationFailure::InvalidFundingBills,
            bill,
            "instType",
            "configured SWAP",
            bill.instrument_type
                .map_or("missing", OkxInstrumentType::as_str),
            "funding bill is not for a configured swap contract",
        );
    }
    if !matches!(bill.sub_type.as_str(), "173" | "174") {
        push_bill_issue(
            failures,
            issues,
            EconomicReconciliationFailure::InvalidFundingBills,
            bill,
            "subType",
            "173 (expense) or 174 (income)",
            &bill.sub_type,
            "funding bill subtype does not match the pinned Java mapping",
        );
    }
    let expected_currency = instrument.settle_currency.trim().to_ascii_uppercase();
    if expected_currency.is_empty() || bill.currency != expected_currency {
        let expected = if expected_currency.is_empty() {
            "configured settlement currency"
        } else {
            &expected_currency
        };
        push_bill_issue(
            failures,
            issues,
            EconomicReconciliationFailure::InvalidFundingBills,
            bill,
            "ccy",
            expected,
            &bill.currency,
            "funding bill currency does not match the configured settlement currency",
        );
    }
    if let Some(expected_margin) = expected_bill_margin_mode(config, account_id, &bill.symbol)
        && bill.margin_mode != Some(expected_margin)
    {
        push_bill_issue(
            failures,
            issues,
            EconomicReconciliationFailure::InvalidFundingBills,
            bill,
            "mgnMode",
            margin_mode_name(expected_margin),
            bill.margin_mode.map_or("missing", margin_mode_name),
            "funding bill margin mode does not match the configured account trade mode",
        );
    }
    if !bill.trade_id.is_empty()
        || !bill.order_id.is_empty()
        || !bill.client_order_id.is_empty()
        || bill.execution_type.is_some()
    {
        push_bill_issue(
            failures,
            issues,
            EconomicReconciliationFailure::InvalidFundingBills,
            bill,
            "trade_identity",
            "empty for funding",
            "populated",
            "funding bill unexpectedly carries a normal trade identity",
        );
    }
    if bill.from_account.is_some() || bill.to_account.is_some() {
        push_bill_issue(
            failures,
            issues,
            EconomicReconciliationFailure::InvalidFundingBills,
            bill,
            "from/to",
            "empty for funding",
            "populated",
            "funding bill unexpectedly identifies an account transfer",
        );
    }
    let Some(assessment_time_ms) = bill.fill_time_ms else {
        push_bill_issue(
            failures,
            issues,
            EconomicReconciliationFailure::InvalidFundingBills,
            bill,
            "fillTime",
            "funding assessment timestamp",
            "missing",
            "funding bill omits the timestamp needed to bind position and mark evidence",
        );
        return None;
    };
    if assessment_time_ms
        < bill
            .timestamp_ms
            .saturating_sub(options.maximum_funding_bill_delay_ms)
        || assessment_time_ms
            > bill
                .timestamp_ms
                .saturating_add(options.maximum_funding_bill_delay_ms)
    {
        push_bill_issue(
            failures,
            issues,
            EconomicReconciliationFailure::InvalidFundingBills,
            bill,
            "fillTime",
            &format!(
                "within {} ms of bill ts",
                options.maximum_funding_bill_delay_ms
            ),
            &assessment_time_ms.to_string(),
            "funding assessment and balance-update timestamps are not causally close",
        );
    }
    if bill
        .fee
        .is_some_and(|fee| !close_abs(fee, 0.0, options.tolerances.fee_abs))
        || bill
            .interest
            .is_some_and(|interest| !close_abs(interest, 0.0, options.tolerances.balance_abs))
    {
        push_bill_issue(
            failures,
            issues,
            EconomicReconciliationFailure::InvalidFundingBills,
            bill,
            "fee/interest",
            "0",
            &format!("fee={:?}, interest={:?}", bill.fee, bill.interest),
            "funding settlement unexpectedly contains a separate fee or interest charge",
        );
    }

    let candidates = settlements
        .iter()
        .copied()
        .filter(|settlement| {
            settlement.symbol == bill.symbol
                && settlement.funding_time_ms <= bill.timestamp_ms
                && bill.timestamp_ms - settlement.funding_time_ms
                    <= options.maximum_funding_bill_delay_ms
        })
        .collect::<Vec<_>>();
    let session_bound_candidates = candidates
        .iter()
        .copied()
        .filter(|settlement| {
            runtime_session_for_line(runtime_sessions, account_id, settlement.line).is_some_and(
                |session| {
                    session.config_fingerprint == config_fingerprint
                        && session.account_identity_sha256 == account_identity_sha256
                        && session.started_at_ms <= assessment_time_ms
                },
            )
        })
        .collect::<Vec<_>>();
    let settlement = match (candidates.as_slice(), session_bound_candidates.as_slice()) {
        ([settlement], _) | (_, [settlement]) => *settlement,
        _ => {
            issues.push(
                EconomicReconciliationFailure::FundingBillsMissingSettlements,
                issue_for_bill(
                    EconomicIssueSource::Journal,
                    bill,
                    "funding_settlement",
                    "exactly one session-bound journaled settled rate within the causal delay",
                    &format!(
                        "causal={}, session_bound={}",
                        candidates.len(),
                        session_bound_candidates.len()
                    ),
                    "funding bill cannot be bound to one normalized settled-rate source",
                ),
                failures,
            );
            return None;
        }
    };
    counts.funding_bills_matched += 1;
    if assessment_time_ms < settlement.funding_time_ms
        || assessment_time_ms - settlement.funding_time_ms > options.maximum_funding_bill_delay_ms
    {
        push_bill_issue(
            failures,
            issues,
            EconomicReconciliationFailure::InvalidFundingBills,
            bill,
            "fillTime",
            &format!(
                "settlement..=settlement+{}",
                options.maximum_funding_bill_delay_ms
            ),
            &assessment_time_ms.to_string(),
            "funding assessment timestamp is outside the scheduled settlement delay",
        );
    }
    let runtime_session = runtime_session_for_line(runtime_sessions, account_id, settlement.line);
    let Some(runtime_session) = runtime_session.filter(|session| {
        session.config_fingerprint == config_fingerprint
            && session.account_identity_sha256 == account_identity_sha256
            && session.started_at_ms <= assessment_time_ms
    }) else {
        issues.push(
            EconomicReconciliationFailure::FundingSessionBoundaryMissing,
            issue_for_bill(
                EconomicIssueSource::Journal,
                bill,
                "runtime_session",
                "matching account/config/account-identity session start before assessment",
                "missing",
                "funding evidence cannot be tied to one explicitly journaled runtime session",
            ),
            failures,
        );
        return None;
    };
    let next_session_line = runtime_sessions
        .iter()
        .copied()
        .filter(|session| session.account_id == account_id && session.line > runtime_session.line)
        .map(|session| session.line)
        .min()
        .unwrap_or(u64::MAX);
    let position = position_observations
        .iter()
        .filter(|position| {
            position.symbol == bill.symbol
                && position.line > runtime_session.line
                && position.line < next_session_line
                && position.event_ts_ms <= assessment_time_ms
        })
        .max_by_key(|position| (position.event_ts_ms, position.line));
    let Some(position) = position else {
        issues.push(
            EconomicReconciliationFailure::FundingPositionMismatches,
            issue_for_bill(
                EconomicIssueSource::Journal,
                bill,
                "position",
                "latest same-session journaled position at or before funding assessment",
                "missing",
                "funding payment cannot be bound to an independently journaled position",
            ),
            failures,
        );
        return None;
    };
    if !position.quantity.is_finite() || position.quantity == 0.0 {
        issues.push(
            EconomicReconciliationFailure::FundingPositionMismatches,
            issue_for_bill(
                EconomicIssueSource::Journal,
                bill,
                "position_quantity",
                "finite non-zero signed position",
                &position.quantity.to_string(),
                "journaled position cannot explain a non-zero funding bill",
            ),
            failures,
        );
    }
    let Some(quantity) = bill
        .quantity
        .filter(|value| value.is_finite() && *value > 0.0)
    else {
        push_bill_issue(
            failures,
            issues,
            EconomicReconciliationFailure::InvalidFundingBills,
            bill,
            "sz",
            "positive position quantity in contracts",
            &format!("{:?}", bill.quantity),
            "funding formula requires a positive contract quantity",
        );
        return None;
    };
    if !close_abs(
        position.quantity.abs(),
        quantity,
        options.tolerances.quantity_abs,
    ) {
        issues.push(
            EconomicReconciliationFailure::FundingPositionMismatches,
            issue_for_bill(
                EconomicIssueSource::Journal,
                bill,
                "position_quantity",
                &position.quantity.abs().to_string(),
                &quantity.to_string(),
                "funding bill quantity does not match the latest journaled position",
            ),
            failures,
        );
    }
    let Some(bill_mark_price) = bill.price.filter(|value| value.is_finite() && *value > 0.0) else {
        push_bill_issue(
            failures,
            issues,
            EconomicReconciliationFailure::InvalidFundingBills,
            bill,
            "px",
            "positive settlement mark price",
            &format!("{:?}", bill.price),
            "funding formula requires the exchange-reported assessment mark for independent comparison",
        );
        return None;
    };
    let mark_before = mark_price_observations
        .iter()
        .copied()
        .filter(|observation| {
            observation.symbol == bill.symbol
                && observation.line > runtime_session.line
                && observation.line < next_session_line
                && observation.event_ts_ms <= assessment_time_ms
                && assessment_time_ms - observation.event_ts_ms
                    <= options.maximum_funding_mark_bracket_distance_ms
        })
        .max_by_key(|observation| (observation.event_ts_ms, observation.line));
    let mark_after = mark_price_observations
        .iter()
        .copied()
        .filter(|observation| {
            observation.symbol == bill.symbol
                && observation.line > runtime_session.line
                && observation.line < next_session_line
                && observation.event_ts_ms >= assessment_time_ms
                && observation.event_ts_ms - assessment_time_ms
                    <= options.maximum_funding_mark_bracket_distance_ms
        })
        .min_by_key(|observation| (observation.event_ts_ms, observation.line));
    let (Some(mark_before), Some(mark_after)) = (mark_before, mark_after) else {
        issues.push(
            EconomicReconciliationFailure::FundingMarkBracketsMissing,
            issue_for_bill(
                EconomicIssueSource::Journal,
                bill,
                "mark_price_bracket",
                &format!(
                    "same-session marks on both sides of fillTime within {} ms",
                    options.maximum_funding_mark_bracket_distance_ms
                ),
                &format!(
                    "before={}, after={}",
                    mark_before.is_some(),
                    mark_after.is_some()
                ),
                "funding assessment cannot be compared with a two-sided journaled mark bracket",
            ),
            failures,
        );
        return None;
    };
    let mark_lower_bound = mark_before.price.min(mark_after.price);
    let mark_upper_bound = mark_before.price.max(mark_after.price);
    let mark_scale = bill_mark_price
        .abs()
        .max(mark_lower_bound.abs())
        .max(mark_upper_bound.abs());
    let mark_effective_tolerance = options
        .tolerances
        .funding_mark_abs
        .max(options.tolerances.funding_mark_relative * mark_scale);
    let mark_valid = mark_effective_tolerance.is_finite()
        && bill_mark_price >= mark_lower_bound - mark_effective_tolerance
        && bill_mark_price <= mark_upper_bound + mark_effective_tolerance;
    if mark_valid {
        counts.funding_mark_brackets_validated += 1;
    } else {
        push_bill_issue(
            failures,
            issues,
            EconomicReconciliationFailure::FundingMarkMismatches,
            bill,
            "px",
            &format!(
                "{}..={} +/- {} from journaled mark bracket",
                mark_lower_bound, mark_upper_bound, mark_effective_tolerance
            ),
            &bill_mark_price.to_string(),
            "funding bill mark lies outside the independently journaled assessment bracket",
        );
    }
    let Some(pnl) = bill.pnl.filter(|value| value.is_finite() && *value != 0.0) else {
        push_bill_issue(
            failures,
            issues,
            EconomicReconciliationFailure::InvalidFundingBills,
            bill,
            "pnl",
            "non-zero signed funding payment",
            &format!("{:?}", bill.pnl),
            "funding bill does not contain a signed payment",
        );
        return None;
    };
    if (bill.sub_type == "173" && pnl >= 0.0) || (bill.sub_type == "174" && pnl <= 0.0) {
        push_bill_issue(
            failures,
            issues,
            EconomicReconciliationFailure::InvalidFundingBills,
            bill,
            "pnl_sign",
            if bill.sub_type == "173" {
                "negative expense"
            } else {
                "positive income"
            },
            &pnl.to_string(),
            "funding payment sign contradicts the pinned Java/OKX subtype",
        );
    }
    if !close_abs(bill.balance_change, pnl, options.tolerances.balance_abs) {
        push_bill_issue(
            failures,
            issues,
            EconomicReconciliationFailure::InvalidFundingBills,
            bill,
            "balChg",
            &pnl.to_string(),
            &bill.balance_change.to_string(),
            "funding balance change does not equal the reported funding PnL",
        );
    }

    let expected_pnl_at_bill_mark = funding_pnl_at_mark(
        position.quantity,
        instrument.contract_value,
        settlement.rate,
        bill_mark_price,
        instrument.kind.is_inverse(),
    );
    let expected_pnl_at_mark_before = funding_pnl_at_mark(
        position.quantity,
        instrument.contract_value,
        settlement.rate,
        mark_before.price,
        instrument.kind.is_inverse(),
    );
    let expected_pnl_at_mark_after = funding_pnl_at_mark(
        position.quantity,
        instrument.contract_value,
        settlement.rate,
        mark_after.price,
        instrument.kind.is_inverse(),
    );
    let expected_pnl_lower_bound = expected_pnl_at_mark_before.min(expected_pnl_at_mark_after);
    let expected_pnl_upper_bound = expected_pnl_at_mark_before.max(expected_pnl_at_mark_after);
    let expected_pnl_absolute = expected_pnl_lower_bound
        .abs()
        .max(expected_pnl_upper_bound.abs());
    let absolute_difference = if pnl < expected_pnl_lower_bound {
        expected_pnl_lower_bound - pnl
    } else if pnl > expected_pnl_upper_bound {
        pnl - expected_pnl_upper_bound
    } else {
        0.0
    };
    let relative_difference =
        absolute_difference / expected_pnl_absolute.max(pnl.abs()).max(f64::MIN_POSITIVE);
    let effective_tolerance = options
        .tolerances
        .funding_pnl_abs
        .max(options.tolerances.funding_pnl_relative * expected_pnl_absolute);
    let formula_valid = expected_pnl_at_bill_mark.is_finite()
        && expected_pnl_lower_bound.is_finite()
        && expected_pnl_upper_bound.is_finite()
        && absolute_difference.is_finite()
        && absolute_difference <= effective_tolerance;
    if !formula_valid {
        push_bill_issue(
            failures,
            issues,
            EconomicReconciliationFailure::FundingFormulaMismatches,
            bill,
            "pnl_formula",
            &format!(
                "{}..={} +/- {}",
                expected_pnl_lower_bound, expected_pnl_upper_bound, effective_tolerance
            ),
            &pnl.to_string(),
            "funding payment does not match the configured contract formula, journaled signed position/rate, and independent mark bracket",
        );
    }
    let validated = before == issues.total && mark_valid && formula_valid;
    if validated {
        counts.funding_bills_validated += 1;
    }
    Some(FundingFormulaSample {
        bill_id: bill.bill_id.clone(),
        symbol: bill.symbol.clone(),
        runtime_session_id: runtime_session.session_id.clone(),
        runtime_session_start_line: runtime_session.line,
        runtime_session_started_at_ms: runtime_session.started_at_ms,
        bill_timestamp_ms: bill.timestamp_ms,
        settlement_time_ms: settlement.funding_time_ms,
        settlement_delay_ms: bill.timestamp_ms - settlement.funding_time_ms,
        assessment_time_ms,
        assessment_delay_ms: assessment_time_ms.saturating_sub(settlement.funding_time_ms),
        rate: settlement.rate,
        inverse: instrument.kind.is_inverse(),
        currency: bill.currency.clone(),
        quantity,
        journal_position_quantity: position.quantity,
        position_observation_line: position.line,
        position_observation_time_ms: position.event_ts_ms,
        contract_value: instrument.contract_value,
        bill_mark_price,
        mark_before_line: mark_before.line,
        mark_before_time_ms: mark_before.event_ts_ms,
        mark_before_price: mark_before.price,
        mark_after_line: mark_after.line,
        mark_after_time_ms: mark_after.event_ts_ms,
        mark_after_price: mark_after.price,
        mark_lower_bound,
        mark_upper_bound,
        mark_effective_tolerance,
        mark_validated: mark_valid,
        expected_pnl_at_bill_mark,
        expected_pnl_lower_bound,
        expected_pnl_upper_bound,
        expected_pnl_absolute,
        observed_pnl: pnl,
        absolute_difference,
        relative_difference,
        effective_tolerance,
        validated,
    })
}

fn funding_pnl_at_mark(
    signed_position: f64,
    contract_value: f64,
    rate: f64,
    mark_price: f64,
    inverse: bool,
) -> f64 {
    if inverse {
        -(signed_position * contract_value * rate / mark_price)
    } else {
        -(signed_position * contract_value * rate * mark_price)
    }
}
