use std::collections::{BTreeMap, BTreeSet};

use reap_core::PINNED_JAVA_REVISION;

use super::artifacts::validate_journal_identity;
use super::cash_continuity::validate_account_balance_continuity;
use super::funding_bills::{
    validate_funding_bill, validate_funding_mark_prices, validate_funding_settlements,
};
use super::position_basis::{build_journal_trade_evidence, validate_runtime_sessions};
use super::support::{instrument, issue, issue_for_bill};
use super::trade_bills::validate_trade_bill;
use super::{
    BoundEconomicSources, ECONOMIC_RECONCILIATION_SCHEMA_VERSION, EconomicIssueSource,
    EconomicJournalRecoveryEvidence, EconomicReconciliationCounts, EconomicReconciliationFailure,
    EconomicReconciliationOptions, EconomicReconciliationReport, EconomicReconciliationScope,
    IssueSink, MAX_ECONOMIC_DERIVATIVE_PNL_SAMPLES, MAX_ECONOMIC_FUNDING_SAMPLES,
};

pub(super) fn build_report(
    sources: BoundEconomicSources,
    options: EconomicReconciliationOptions,
    executable_sha256: String,
) -> EconomicReconciliationReport {
    let mut counts = EconomicReconciliationCounts {
        bills_total: sources.bills.len() as u64,
        fills_total: sources.fills.len() as u64,
        funding_settlements_total: sources.settlements.len() as u64,
        ..EconomicReconciliationCounts::default()
    };
    let mut failures = BTreeSet::new();
    let mut issues = IssueSink::default();
    let currency_balance_continuity = validate_account_balance_continuity(
        &sources,
        &options,
        &mut counts,
        &mut failures,
        &mut issues,
    );
    validate_journal_identity(&sources, &mut failures, &mut issues);

    let required_fill_begin = options
        .begin_ms
        .saturating_sub(options.maximum_trade_bill_delay_ms);
    let completeness_end = options
        .end_ms
        .saturating_sub(options.maximum_trade_bill_delay_ms);
    for fill in &sources.fills {
        if (required_fill_begin..=options.end_ms).contains(&fill.ts_ms) {
            counts.fills_in_required_collection_window += 1;
        }
        if (options.begin_ms..=completeness_end).contains(&fill.ts_ms) {
            counts.fills_eligible_for_completeness += 1;
        } else if fill.ts_ms > completeness_end && fill.ts_ms <= options.end_ms {
            counts.fills_in_end_guard += 1;
        }
    }

    let mut fill_by_key = BTreeMap::new();
    for fill in &sources.fills {
        let key = (fill.symbol.clone(), fill.fill_id.clone());
        if fill_by_key.insert(key.clone(), fill).is_some() {
            issues.push(
                EconomicReconciliationFailure::DuplicateFills,
                issue(
                    EconomicIssueSource::FillCollection,
                    None,
                    Some(&key.0),
                    Some(&key.1),
                    "trade_identity",
                    "unique (symbol, tradeId)",
                    "duplicate",
                    "verified fill pages contain a duplicate trade identity",
                ),
                &mut failures,
            );
        }
    }

    let valid_runtime_sessions = validate_runtime_sessions(&sources, &mut failures, &mut issues);
    let journal_trade_evidence = build_journal_trade_evidence(
        &sources,
        &valid_runtime_sessions,
        &options,
        &mut failures,
        &mut issues,
    );
    let valid_settlements = validate_funding_settlements(
        &sources,
        &valid_runtime_sessions,
        &options,
        &mut counts,
        &mut failures,
        &mut issues,
    );
    let valid_mark_prices = validate_funding_mark_prices(
        &sources,
        &valid_runtime_sessions,
        &mut failures,
        &mut issues,
    );
    let mut trade_bill_keys = BTreeSet::new();
    let mut matched_fill_keys = BTreeSet::new();
    let mut derivative_pnl_samples = Vec::new();
    let mut derivative_pnl_samples_omitted = 0_u64;
    let mut funding_samples = Vec::new();
    let mut funding_samples_omitted = 0_u64;

    for bill in &sources.bills {
        match bill.bill_type.as_str() {
            "2" => {
                counts.trade_bills += 1;
                let key = (bill.symbol.clone(), bill.trade_id.clone());
                if !trade_bill_keys.insert(key.clone()) {
                    issues.push(
                        EconomicReconciliationFailure::DuplicateTradeBills,
                        issue_for_bill(
                            EconomicIssueSource::BillCollection,
                            bill,
                            "trade_identity",
                            "unique (symbol, tradeId)",
                            "duplicate",
                            "multiple trade bills have the same exchange trade identity",
                        ),
                        &mut failures,
                    );
                    continue;
                }
                let Some(fill) = fill_by_key.get(&key).copied() else {
                    issues.push(
                        EconomicReconciliationFailure::TradeBillsMissingFills,
                        issue_for_bill(
                            EconomicIssueSource::BillCollection,
                            bill,
                            "trade_identity",
                            "matching verified fill",
                            "missing",
                            "trade bill has no matching fill in the guarded fill collection",
                        ),
                        &mut failures,
                    );
                    continue;
                };
                counts.trade_bills_matched += 1;
                matched_fill_keys.insert(key.clone());
                if instrument(&sources.config, &bill.symbol).is_some_and(|instrument| {
                    instrument.kind.is_derivative() && matches!(bill.sub_type.as_str(), "5" | "6")
                }) {
                    counts.derivative_close_bills += 1;
                }
                let validation = validate_trade_bill(
                    bill,
                    fill,
                    journal_trade_evidence
                        .get(&key)
                        .map(Vec::as_slice)
                        .unwrap_or_default(),
                    &sources.config,
                    &sources.account_id,
                    &options,
                    &mut failures,
                    &mut issues,
                );
                if validation.valid {
                    counts.trade_bills_validated += 1;
                }
                if let Some(sample) = validation.derivative_sample {
                    if validation.valid
                        && sample.close_quantity > options.tolerances.quantity_abs
                        && sample.validated
                    {
                        counts.derivative_close_bills_recomputed += 1;
                    }
                    if derivative_pnl_samples.len() < MAX_ECONOMIC_DERIVATIVE_PNL_SAMPLES {
                        derivative_pnl_samples.push(sample);
                    } else {
                        derivative_pnl_samples_omitted =
                            derivative_pnl_samples_omitted.saturating_add(1);
                    }
                }
            }
            "8" => {
                counts.funding_bills += 1;
                let sample = validate_funding_bill(
                    bill,
                    &valid_settlements,
                    &valid_runtime_sessions,
                    &sources.position_observations,
                    &valid_mark_prices,
                    &sources.config,
                    &sources.config_fingerprint,
                    &sources.account_identity_sha256,
                    &sources.account_id,
                    &options,
                    &mut counts,
                    &mut failures,
                    &mut issues,
                );
                if let Some(sample) = sample {
                    if funding_samples.len() < MAX_ECONOMIC_FUNDING_SAMPLES {
                        funding_samples.push(sample);
                    } else {
                        funding_samples_omitted = funding_samples_omitted.saturating_add(1);
                    }
                }
            }
            _ => {
                counts.unsupported_bills += 1;
                issues.push(
                    EconomicReconciliationFailure::UnsupportedBills,
                    issue_for_bill(
                        EconomicIssueSource::BillCollection,
                        bill,
                        "type",
                        "2 (trade) or 8 (funding)",
                        &bill.bill_type,
                        "controlled strategy window contains an unexplained balance-changing bill",
                    ),
                    &mut failures,
                );
            }
        }
    }

    for fill in &sources.fills {
        if !(options.begin_ms..=completeness_end).contains(&fill.ts_ms) {
            continue;
        }
        let key = (fill.symbol.clone(), fill.fill_id.clone());
        if !matched_fill_keys.contains(&key) {
            counts.eligible_fills_missing_bill += 1;
            issues.push(
                EconomicReconciliationFailure::EligibleFillsMissingBills,
                issue(
                    EconomicIssueSource::FillCollection,
                    None,
                    Some(&fill.symbol),
                    Some(&fill.fill_id),
                    "trade_bill",
                    "matching account bill inside the closed window",
                    "missing",
                    "interior fill has no matching account trade bill",
                ),
                &mut failures,
            );
        }
    }

    if counts.trade_bills_validated < options.minimum_trade_bills {
        failures.insert(EconomicReconciliationFailure::MinimumTradeBillsNotMet);
    }
    if counts.derivative_close_bills_recomputed < options.minimum_derivative_close_bills {
        failures.insert(EconomicReconciliationFailure::MinimumDerivativeCloseBillsNotMet);
    }
    if counts.funding_bills_validated < options.minimum_funding_bills {
        failures.insert(EconomicReconciliationFailure::MinimumFundingBillsNotMet);
    }
    counts.issues_total = issues.total;
    counts.issues_reported = issues.issues.len() as u64;
    let issues_truncated = issues.total > issues.issues.len() as u64;
    let failures = failures.into_iter().collect::<Vec<_>>();
    let passed = failures.is_empty();

    EconomicReconciliationReport {
        schema_version: ECONOMIC_RECONCILIATION_SCHEMA_VERSION,
        scope: EconomicReconciliationScope::NormalTradeAndFundingBills,
        java_reference_revision: PINNED_JAVA_REVISION.to_string(),
        reap_version: env!("CARGO_PKG_VERSION").to_string(),
        executable_sha256,
        account_id: options.account_id,
        environment: sources.environment,
        account_identity_sha256: sources.account_identity_sha256,
        strategy_name: sources.config.strategy.strategy_name.clone(),
        config_fingerprint: sources.config_fingerprint,
        window: sources.window,
        minimum_trade_bills: options.minimum_trade_bills,
        minimum_derivative_close_bills: options.minimum_derivative_close_bills,
        minimum_funding_bills: options.minimum_funding_bills,
        maximum_trade_bill_delay_ms: options.maximum_trade_bill_delay_ms,
        maximum_funding_bill_delay_ms: options.maximum_funding_bill_delay_ms,
        maximum_funding_mark_bracket_distance_ms: options
            .maximum_funding_mark_bracket_distance_ms,
        maximum_account_boundary_gap_ms: options.maximum_account_boundary_gap_ms,
        tolerances: options.tolerances,
        config_file: sources.config_file,
        journal: sources.journal,
        journal_recovery: EconomicJournalRecoveryEvidence {
            records: sources.recovered.records,
            ignored_truncated_tail: sources.recovered.ignored_truncated_tail,
            account_bootstrap_records: sources.account_bootstrap_records,
            runtime_session_records: sources.runtime_sessions.len() as u64,
            authoritative_account_snapshot_records: sources
                .authoritative_account_snapshots
                .len() as u64,
            journal_fill_records: sources.journal_fills.len() as u64,
            funding_settlement_records: sources.settlements.len() as u64,
            position_observation_records: sources.position_observations.len() as u64,
            mark_price_observation_records: sources.mark_price_observations.len() as u64,
            exclusive_lease_held_while_reading: true,
        },
        fill_collection_manifest: sources.fill_manifest_file,
        bill_collection_manifest: sources.bill_manifest_file,
        total_equity_change_usd: sources
            .closing_account_boundary
            .evidence
            .total_equity_usd
            - sources
                .opening_account_boundary
                .evidence
                .total_equity_usd,
        opening_account_boundary: sources.opening_account_boundary.evidence,
        closing_account_boundary: sources.closing_account_boundary.evidence,
        currency_balance_continuity,
        counts,
        derivative_pnl_formula_samples: derivative_pnl_samples,
        derivative_pnl_formula_samples_omitted: derivative_pnl_samples_omitted,
        funding_formula_samples: funding_samples,
        funding_formula_samples_omitted: funding_samples_omitted,
        issues: issues.issues,
        issues_truncated,
        limitations: vec![
            "derivative close PnL is reconstructed from same-session authoritative REST avgPx snapshots and every intervening critical journal fill; the snapshot exchange timestamp must strictly precede every replayed fill".to_string(),
            "expiry-futures avgPx can reset at settlement; controlled evidence windows containing unsupported settlement bills fail, but dedicated settlement-PnL reconstruction remains out of scope".to_string(),
            "funding checks the bill-reported mark against journaled observations bracketing the exchange-reported assessment time; the exact internal venue assessment tick is not reproduced".to_string(),
            "runtime-session boundaries are locally journaled provenance that prevents cross-restart evidence composition; they are not remote process attestation".to_string(),
            "settlements with no funding bill are not failures because a zero position legitimately produces no balance change; minimum matched funding evidence is required instead".to_string(),
            "the final trade-delay guard is excluded from fill-to-bill completeness because its bills may fall after the closed account-bill window".to_string(),
            "opening and closing account snapshots are sequential authenticated/public REST certifications rather than atomic venue valuation ticks".to_string(),
            "a currency absent from an unfiltered OKX balance response is treated as zero at that boundary; every intervening balance-changing bill must still be present".to_string(),
            "total-equity delta is reported but is not equated to cash bill changes because mark-to-market unrealized PnL can change between boundaries".to_string(),
        ],
        failures,
        passed,
    }
}
