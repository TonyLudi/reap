use reap_core::FillFee;

use super::super::*;
use super::support::*;

#[test]
fn derivative_pnl_tamper_fails_even_when_balance_equation_is_self_consistent() {
    let mut sources = sources();
    sources.bills[0].pnl = Some(19.0);
    sources.bills[0].balance_change = 18.5;

    let report = build_report(sources, options(), "c".repeat(64));

    assert!(!report.passed);
    assert!(
        report
            .failures
            .contains(&EconomicReconciliationFailure::DerivativePnlFormulaMismatches)
    );
    assert!(
        report
            .failures
            .contains(&EconomicReconciliationFailure::MinimumDerivativeCloseBillsNotMet)
    );
    assert_eq!(report.derivative_pnl_formula_samples[0].expected_pnl, 20.0);
    assert_eq!(report.derivative_pnl_formula_samples[0].observed_pnl, 19.0);
    assert!(!report.derivative_pnl_formula_samples[0].validated);
}

#[test]
fn trade_bill_margin_mode_must_match_the_account_configuration() {
    let mut sources = sources();
    sources.bills[0].margin_mode = Some(OkxBillMarginMode::Isolated);

    let report = build_report(sources, options(), "c".repeat(64));

    assert!(!report.passed);
    assert!(
        report
            .failures
            .contains(&EconomicReconciliationFailure::InvalidTradeBills)
    );
    assert!(report.issues.iter().any(|issue| issue.field == "mgnMode"));
}

#[test]
fn spot_sell_uses_quote_currency_quantity_and_balance_change() {
    let mut sources = sources();
    sources.fills[0] = RemoteFill {
        fill_id: "trade-spot".to_string(),
        exchange_order_id: "exchange-spot".to_string(),
        client_order_id: "reap-spot".to_string(),
        symbol: "BTC-USDT".to_string(),
        side: Side::Sell,
        price: 50_000.0,
        qty: 0.01,
        liquidity: FillLiquidity::Maker,
        fee: Some(FillFee {
            amount: -0.05,
            currency: "USDT".to_string(),
        }),
        ts_ms: TRADE_MS,
    };
    sources.bills[0] = OkxBill {
        bill_id: "100".to_string(),
        bill_type: "2".to_string(),
        sub_type: "2".to_string(),
        timestamp_ms: TRADE_MS + 1,
        currency: "USDT".to_string(),
        balance_change: 499.95,
        balance: Some(1_000.0),
        position_balance_change: Some(0.0),
        position_balance: Some(0.0),
        quantity: Some(500.0),
        price: Some(50_000.0),
        pnl: Some(0.0),
        fee: Some(-0.05),
        interest: Some(0.0),
        instrument_type: Some(OkxInstrumentType::Spot),
        symbol: "BTC-USDT".to_string(),
        margin_mode: Some(OkxBillMarginMode::Cash),
        order_id: "exchange-spot".to_string(),
        client_order_id: "reap-spot".to_string(),
        trade_id: "trade-spot".to_string(),
        fill_time_ms: Some(TRADE_MS),
        execution_type: Some(OkxBillExecutionType::Maker),
        from_account: None,
        to_account: None,
        notes: String::new(),
    };
    sources.journal_fills[0] = JournalFillObservation {
        line: 4,
        fill: FillRecord {
            ts_ms: TRADE_MS,
            account_id: Some("main".to_string()),
            fill_id: "trade-spot".to_string(),
            order_id: "reap-spot".to_string(),
            symbol: "BTC-USDT".to_string(),
            side: Side::Sell,
            price: 50_000.0,
            qty: 0.01,
            liquidity: Some(FillLiquidity::Maker),
            fee: Some(FillFee {
                amount: -0.05,
                currency: "USDT".to_string(),
            }),
        },
    };
    let mut options = options();
    options.minimum_derivative_close_bills = 0;
    set_boundary_cash(&mut sources.opening_account_boundary, 500.05);

    let report = build_report(sources, options, "c".repeat(64));

    assert!(report.passed, "{:?}", report.issues);
    assert_eq!(report.counts.trade_bills_validated, 1);
}
