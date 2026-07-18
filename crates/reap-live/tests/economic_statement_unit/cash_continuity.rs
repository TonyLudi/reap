use super::super::*;
use super::support::*;

#[test]
fn cash_continuity_requires_every_bill_post_balance() {
    let mut sources = sources();
    sources.bills[0].balance = None;

    let report = build_report(sources, options(), "c".repeat(64));

    assert!(!report.passed);
    assert!(
        report
            .failures
            .contains(&EconomicReconciliationFailure::InvalidBillBalanceChain)
    );
    assert!(report.issues.iter().any(|issue| issue.field == "bal"));
    assert_eq!(report.counts.cash_balance_chain_links, 3);
    assert!(report.counts.cash_balance_chain_links_validated < 3);
}

#[test]
fn cash_continuity_rejects_a_broken_intermediate_bill_link() {
    let mut sources = sources();
    sources.bills[1].balance = Some(995.0);

    let report = build_report(sources, options(), "c".repeat(64));

    assert!(!report.passed);
    assert!(
        report
            .failures
            .contains(&EconomicReconciliationFailure::InvalidBillBalanceChain)
    );
    assert!(
        report
            .issues
            .iter()
            .any(|issue| issue.field == "bill_balance_chain")
    );
}

#[test]
fn cash_continuity_rejects_a_certified_endpoint_delta_mismatch() {
    let mut sources = sources();
    set_boundary_cash(&mut sources.closing_account_boundary, 997.0);

    let report = build_report(sources, options(), "c".repeat(64));

    assert!(!report.passed);
    assert!(
        report
            .failures
            .contains(&EconomicReconciliationFailure::CashBalanceContinuityMismatches)
    );
    assert!(!report.currency_balance_continuity[0].validated);
    assert_eq!(
        report.currency_balance_continuity[0].expected_closing_cash_balance,
        996.0
    );
}

#[test]
fn account_boundary_timing_and_numeric_bill_ids_fail_closed() {
    let mut timing = sources();
    timing.opening_account_boundary.evidence.finish_server_ms = BEGIN_MS + 1;
    timing.opening_account_boundary.evidence.window_gap_ms = 0;
    let report = build_report(timing, options(), "c".repeat(64));
    assert!(
        report
            .failures
            .contains(&EconomicReconciliationFailure::InvalidAccountBoundaries)
    );

    let mut nonnumeric = sources();
    nonnumeric.bills[0].bill_id = "not-numeric".to_string();
    let report = build_report(nonnumeric, options(), "c".repeat(64));
    assert!(
        report
            .failures
            .contains(&EconomicReconciliationFailure::InvalidBillBalanceChain)
    );
}
