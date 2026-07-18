use reap_strategy::InstrumentKindConfig;

use super::super::*;
use super::support::*;

#[test]
fn derivative_close_requires_a_same_session_authoritative_basis() {
    let mut sources = sources();
    sources.authoritative_account_snapshots.clear();

    let report = build_report(sources, options(), "c".repeat(64));

    assert!(!report.passed);
    assert!(
        report
            .failures
            .contains(&EconomicReconciliationFailure::DerivativeOpeningBasisMissingOrInvalid)
    );
    assert!(report.derivative_pnl_formula_samples.is_empty());
    assert_eq!(report.counts.derivative_close_bills_recomputed, 0);
}

#[test]
fn derivative_basis_must_strictly_precede_the_fill_exchange_time() {
    let mut sources = sources();
    sources.authoritative_account_snapshots[0].event_ts_ms = TRADE_MS;
    sources.authoritative_account_snapshots[0].update_ts_ms = TRADE_MS;

    let report = build_report(sources, options(), "c".repeat(64));

    assert!(!report.passed);
    assert!(
        report
            .failures
            .contains(&EconomicReconciliationFailure::DerivativeOpeningBasisMissingOrInvalid)
    );
    assert!(report.derivative_pnl_formula_samples.is_empty());
    assert_eq!(report.counts.derivative_close_bills_recomputed, 0);
}

#[test]
fn derivative_basis_margin_mode_must_match_the_account_configuration() {
    let mut sources = sources();
    sources.authoritative_account_snapshots[0].positions[0].margin_mode =
        Some(PositionMarginMode::Isolated);

    let report = build_report(sources, options(), "c".repeat(64));

    assert!(!report.passed);
    assert!(
        report
            .failures
            .contains(&EconomicReconciliationFailure::InvalidAuthoritativeAccountSnapshots)
    );
    assert!(report.derivative_pnl_formula_samples.is_empty());
}

#[test]
fn derivative_basis_session_must_match_the_collected_account_identity() {
    let mut sources = sources();
    sources.runtime_sessions[0].account_identity_sha256 = "d".repeat(64);

    let report = build_report(sources, options(), "c".repeat(64));

    assert!(!report.passed);
    assert!(
        report
            .failures
            .contains(&EconomicReconciliationFailure::InvalidAuthoritativeAccountSnapshots)
    );
    assert!(report.derivative_pnl_formula_samples.is_empty());
}

#[test]
fn derivative_close_requires_the_exact_critical_journal_fill() {
    let mut sources = sources();
    sources.journal_fills[0].fill.qty = 1.0;

    let report = build_report(sources, options(), "c".repeat(64));

    assert!(!report.passed);
    assert!(
        report
            .failures
            .contains(&EconomicReconciliationFailure::TradeJournalFillMismatches)
    );
    assert!(report.derivative_pnl_formula_samples.is_empty());
}

#[test]
fn derivative_basis_rejects_an_uncollected_intervening_fill() {
    let mut sources = sources();
    sources.journal_fills[0].line = 5;
    sources.journal_fills.insert(
        0,
        JournalFillObservation {
            line: 4,
            fill: FillRecord {
                ts_ms: TRADE_MS - 50,
                account_id: Some("main".to_string()),
                fill_id: "uncollected".to_string(),
                order_id: "reap-uncollected".to_string(),
                symbol: "BTC-USDT-SWAP".to_string(),
                side: Side::Buy,
                price: 49_500.0,
                qty: 1.0,
                liquidity: Some(FillLiquidity::Maker),
                fee: None,
            },
        },
    );

    let report = build_report(sources, options(), "c".repeat(64));

    assert!(!report.passed);
    assert!(
        report
            .failures
            .contains(&EconomicReconciliationFailure::DerivativeOpeningBasisMissingOrInvalid)
    );
    assert!(report.derivative_pnl_formula_samples.is_empty());
    assert_eq!(report.counts.derivative_close_bills_recomputed, 0);
}

#[test]
fn duplicate_critical_journal_fill_identity_fails_closed() {
    let mut sources = sources();
    let mut duplicate = sources.journal_fills[0].clone();
    duplicate.line = 5;
    sources.journal_fills.push(duplicate);

    let report = build_report(sources, options(), "c".repeat(64));

    assert!(!report.passed);
    assert!(
        report
            .failures
            .contains(&EconomicReconciliationFailure::TradeJournalFillMismatches)
    );
    assert!(report.derivative_pnl_formula_samples.is_empty());
}

#[test]
fn inverse_position_basis_uses_pinned_java_harmonic_average() {
    let mut inverse = config()
        .strategy
        .instruments
        .into_iter()
        .find(|instrument| instrument.symbol == "BTC-USDT-SWAP")
        .unwrap();
    inverse.kind = InstrumentKindConfig::InverseSwap;
    inverse.contract_value = 100.0;
    let increase = FillRecord {
        ts_ms: 2,
        account_id: Some("main".to_string()),
        fill_id: "increase".to_string(),
        order_id: "reap-increase".to_string(),
        symbol: inverse.symbol.clone(),
        side: Side::Buy,
        price: 20_000.0,
        qty: 1.0,
        liquidity: Some(FillLiquidity::Maker),
        fee: None,
    };
    let increased = apply_derivative_fill(
        PositionBasis {
            quantity: 2.0,
            avg_price: 10_000.0,
            snapshot_line: 1,
            snapshot_time_ms: 1,
        },
        &increase,
        &inverse,
        1e-12,
    )
    .unwrap();
    assert!((increased.post_avg_price - 12_000.0).abs() < 1e-9);

    let close = FillRecord {
        ts_ms: 3,
        fill_id: "close".to_string(),
        order_id: "reap-close".to_string(),
        side: Side::Sell,
        price: 15_000.0,
        qty: 1.0,
        ..increase
    };
    let closed = apply_derivative_fill(
        PositionBasis {
            quantity: increased.post_quantity,
            avg_price: increased.post_avg_price,
            snapshot_line: 1,
            snapshot_time_ms: 1,
        },
        &close,
        &inverse,
        1e-12,
    )
    .unwrap();
    let expected = 100.0 * (1.0 / 12_000.0 - 1.0 / 15_000.0);
    assert!((closed.expected_pnl - expected).abs() < 1e-15);
    assert_eq!(closed.expected_sub_type, "5");
    assert_eq!(closed.post_quantity, 2.0);
    assert!((closed.post_avg_price - 12_000.0).abs() < 1e-9);
}
