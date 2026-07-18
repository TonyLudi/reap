use super::super::*;
use super::support::*;

#[test]
fn validates_normal_trade_and_linear_funding_from_exact_sources() {
    let report = build_report(sources(), options(), "c".repeat(64));

    assert!(report.passed, "{:?}", report.issues);
    assert!(report.failures.is_empty());
    assert_eq!(report.counts.trade_bills_validated, 1);
    assert_eq!(report.counts.derivative_close_bills_recomputed, 1);
    assert_eq!(report.counts.funding_bills_validated, 1);
    assert_eq!(report.counts.eligible_fills_missing_bill, 0);
    assert_eq!(report.funding_formula_samples.len(), 1);
    assert_eq!(report.derivative_pnl_formula_samples.len(), 1);
    assert_eq!(
        report
            .journal_recovery
            .authoritative_account_snapshot_records,
        1
    );
    assert_eq!(report.journal_recovery.journal_fill_records, 1);
    assert_eq!(report.journal_recovery.position_observation_records, 2);
    assert_eq!(report.journal_recovery.mark_price_observation_records, 2);
    assert_eq!(report.journal_recovery.runtime_session_records, 1);
    assert_eq!(report.counts.funding_mark_brackets_validated, 1);
    assert_eq!(report.counts.cash_balance_currencies, 1);
    assert_eq!(report.counts.cash_balance_currencies_validated, 1);
    assert_eq!(report.counts.cash_balance_chain_links, 3);
    assert_eq!(report.counts.cash_balance_chain_links_validated, 3);
    assert_eq!(report.currency_balance_continuity.len(), 1);
    assert!(report.currency_balance_continuity[0].validated);
    assert_eq!(report.currency_balance_continuity[0].bill_count, 2);
    assert_eq!(
        report.currency_balance_continuity[0].summed_balance_change,
        15.5
    );
    assert_eq!(report.total_equity_change_usd, 15.5);
    assert_eq!(
        report.funding_formula_samples[0].expected_pnl_at_bill_mark,
        -4.0
    );
    assert_eq!(report.funding_formula_samples[0].expected_pnl_absolute, 4.0);
    assert_eq!(
        report.funding_formula_samples[0].journal_position_quantity,
        8.0
    );
    assert_eq!(
        report.funding_formula_samples[0].position_observation_line,
        7
    );
    assert_eq!(
        report.funding_formula_samples[0].runtime_session_id,
        "1a2b3c"
    );
    assert_eq!(
        report.funding_formula_samples[0].runtime_session_start_line,
        2
    );
    assert!(report.funding_formula_samples[0].mark_validated);
    assert!(report.funding_formula_samples[0].validated);
    assert_eq!(report.derivative_pnl_formula_samples[0].pre_quantity, 10.0);
    assert_eq!(
        report.derivative_pnl_formula_samples[0].pre_avg_price,
        49_000.0
    );
    assert_eq!(report.derivative_pnl_formula_samples[0].close_quantity, 2.0);
    assert_eq!(report.derivative_pnl_formula_samples[0].expected_pnl, 20.0);
    assert_eq!(report.derivative_pnl_formula_samples[0].post_quantity, 8.0);
    assert!(report.derivative_pnl_formula_samples[0].validated);
}

#[test]
fn unexplained_balance_changing_bill_fails_closed() {
    let mut sources = sources();
    let mut transfer = funding_bill();
    transfer.bill_id = "300".to_string();
    transfer.bill_type = "1".to_string();
    transfer.sub_type = "11".to_string();
    sources.bills.push(transfer);

    let report = build_report(sources, options(), "c".repeat(64));

    assert!(!report.passed);
    assert_eq!(report.counts.unsupported_bills, 1);
    assert!(
        report
            .failures
            .contains(&EconomicReconciliationFailure::UnsupportedBills)
    );
}
