use super::super::*;
use super::support::*;

#[test]
fn funding_formula_tamper_fails_even_when_balance_equation_is_self_consistent() {
    let mut sources = sources();
    sources.bills[1].pnl = Some(-3.0);
    sources.bills[1].balance_change = -3.0;

    let report = build_report(sources, options(), "c".repeat(64));

    assert!(!report.passed);
    assert!(
        report
            .failures
            .contains(&EconomicReconciliationFailure::FundingFormulaMismatches)
    );
    assert!(
        report
            .failures
            .contains(&EconomicReconciliationFailure::MinimumFundingBillsNotMet)
    );
    assert_eq!(report.funding_formula_samples[0].expected_pnl_absolute, 4.0);
    assert_eq!(report.funding_formula_samples[0].observed_pnl, -3.0);
}

#[test]
fn duplicate_journal_settlement_is_rejected_before_formula_acceptance() {
    let mut sources = sources();
    sources.settlements.push(sources.settlements[0].clone());

    let report = build_report(sources, options(), "c".repeat(64));

    assert!(!report.passed);
    assert!(
        report
            .failures
            .contains(&EconomicReconciliationFailure::InvalidOrDuplicateFundingSettlements)
    );
}

#[test]
fn settled_rate_replay_after_restart_does_not_duplicate_prior_session() {
    let mut sources = sources();
    sources.runtime_sessions.push(JournalRuntimeSession {
        line: 10,
        started_at_ms: FUNDING_MS + 110,
        session_id: "4d5e6f".to_string(),
        account_id: "main".to_string(),
        strategy_name: sources.config.strategy.strategy_name.clone(),
        config_fingerprint: sources.config_fingerprint.clone(),
        account_identity_sha256: sources.account_identity_sha256.clone(),
    });
    let mut replay = sources.settlements[0].clone();
    replay.line = 11;
    replay.event_ts_ms = FUNDING_MS + 150;
    sources.settlements.push(replay);

    let report = build_report(sources, options(), "c".repeat(64));

    assert!(report.passed, "{:?}", report.issues);
    assert_eq!(report.counts.funding_bills_validated, 1);
    assert_eq!(
        report.funding_formula_samples[0].runtime_session_id,
        "1a2b3c"
    );
}

#[test]
fn funding_sign_is_recomputed_from_the_journaled_position() {
    let mut sources = sources();
    sources.position_observations[1].quantity = -8.0;

    let report = build_report(sources, options(), "c".repeat(64));

    assert!(!report.passed);
    assert!(
        report
            .failures
            .contains(&EconomicReconciliationFailure::FundingFormulaMismatches)
    );
    assert_eq!(
        report.funding_formula_samples[0].expected_pnl_at_bill_mark,
        4.0
    );
    assert_eq!(report.funding_formula_samples[0].observed_pnl, -4.0);
}

#[test]
fn funding_requires_a_matching_pre_assessment_journal_position() {
    let mut sources = sources();
    sources.position_observations.clear();

    let report = build_report(sources, options(), "c".repeat(64));

    assert!(!report.passed);
    assert!(
        report
            .failures
            .contains(&EconomicReconciliationFailure::FundingPositionMismatches)
    );
    assert!(report.funding_formula_samples.is_empty());
}

#[test]
fn funding_requires_marks_on_both_sides_of_the_assessment() {
    let mut sources = sources();
    sources.mark_price_observations.pop();

    let report = build_report(sources, options(), "c".repeat(64));

    assert!(!report.passed);
    assert!(
        report
            .failures
            .contains(&EconomicReconciliationFailure::FundingMarkBracketsMissing)
    );
    assert!(report.funding_formula_samples.is_empty());
}

#[test]
fn funding_bill_mark_must_lie_inside_the_journaled_bracket() {
    let mut sources = sources();
    sources.bills[1].price = Some(51_000.0);

    let report = build_report(sources, options(), "c".repeat(64));

    assert!(!report.passed);
    assert!(
        report
            .failures
            .contains(&EconomicReconciliationFailure::FundingMarkMismatches)
    );
    assert!(!report.funding_formula_samples[0].mark_validated);
    assert_eq!(report.funding_formula_samples[0].absolute_difference, 0.0);
}

#[test]
fn duplicate_journal_mark_timestamp_fails_closed() {
    let mut sources = sources();
    let mut duplicate = sources.mark_price_observations[0].clone();
    duplicate.line = 10;
    sources.mark_price_observations.push(duplicate);

    let report = build_report(sources, options(), "c".repeat(64));

    assert!(!report.passed);
    assert!(
        report
            .failures
            .contains(&EconomicReconciliationFailure::InvalidOrDuplicateFundingMarks)
    );
}

#[test]
fn mark_replay_after_restart_does_not_duplicate_prior_session() {
    let mut sources = sources();
    sources.runtime_sessions.push(JournalRuntimeSession {
        line: 10,
        started_at_ms: FUNDING_MS + 110,
        session_id: "4d5e6f".to_string(),
        account_id: "main".to_string(),
        strategy_name: sources.config.strategy.strategy_name.clone(),
        config_fingerprint: sources.config_fingerprint.clone(),
        account_identity_sha256: sources.account_identity_sha256.clone(),
    });
    let mut replay = sources.mark_price_observations[1].clone();
    replay.line = 11;
    sources.mark_price_observations.push(replay);

    let report = build_report(sources, options(), "c".repeat(64));

    assert!(report.passed, "{:?}", report.issues);
    assert_eq!(report.counts.funding_bills_validated, 1);
    assert_eq!(
        report.funding_formula_samples[0].runtime_session_id,
        "1a2b3c"
    );
}

#[test]
fn funding_requires_an_explicit_matching_runtime_session() {
    let mut sources = sources();
    sources.runtime_sessions.clear();

    let report = build_report(sources, options(), "c".repeat(64));

    assert!(!report.passed);
    assert!(
        report
            .failures
            .contains(&EconomicReconciliationFailure::FundingSessionBoundaryMissing)
    );
    assert!(report.funding_formula_samples.is_empty());
}

#[test]
fn funding_mark_bracket_cannot_cross_a_runtime_restart() {
    let mut sources = sources();
    sources.mark_price_observations.pop();
    sources.runtime_sessions.push(JournalRuntimeSession {
        line: 9,
        started_at_ms: FUNDING_MS + 105,
        session_id: "4d5e6f".to_string(),
        account_id: "main".to_string(),
        strategy_name: sources.config.strategy.strategy_name.clone(),
        config_fingerprint: sources.config_fingerprint.clone(),
        account_identity_sha256: sources.account_identity_sha256.clone(),
    });
    sources
        .mark_price_observations
        .push(JournalMarkPriceObservation {
            line: 10,
            event_ts_ms: FUNDING_MS + 110,
            symbol: "BTC-USDT-SWAP".to_string(),
            price: 50_000.0,
        });

    let report = build_report(sources, options(), "c".repeat(64));

    assert!(!report.passed);
    assert!(
        report
            .failures
            .contains(&EconomicReconciliationFailure::FundingMarkBracketsMissing)
    );
    assert!(report.funding_formula_samples.is_empty());
}
