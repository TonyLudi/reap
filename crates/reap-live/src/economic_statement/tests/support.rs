use reap_core::FillFee;

use super::super::*;

pub(super) const BEGIN_MS: u64 = 1_000_000;
pub(super) const END_MS: u64 = 1_200_000;
pub(super) const TRADE_MS: u64 = 1_050_000;
pub(super) const FUNDING_MS: u64 = 1_100_000;

pub(super) fn config() -> LiveConfig {
    LiveConfig::from_toml(include_str!("../../../../../examples/live-okx-demo.toml")).unwrap()
}

pub(super) fn options() -> EconomicReconciliationOptions {
    EconomicReconciliationOptions {
        account_id: "main".to_string(),
        begin_ms: BEGIN_MS,
        end_ms: END_MS,
        minimum_trade_bills: 1,
        minimum_derivative_close_bills: 1,
        minimum_funding_bills: 1,
        maximum_trade_bill_delay_ms: 10_000,
        maximum_funding_bill_delay_ms: 10_000,
        maximum_funding_mark_bracket_distance_ms: 1_000,
        maximum_account_boundary_gap_ms: 10_000,
        tolerances: EconomicReconciliationTolerances {
            price_abs: 0.0,
            quantity_abs: 1e-9,
            fee_abs: 1e-12,
            balance_abs: 1e-12,
            trade_pnl_abs: 1e-12,
            trade_pnl_relative: 1e-12,
            funding_pnl_abs: 1e-12,
            funding_pnl_relative: 1e-12,
            funding_mark_abs: 0.0,
            funding_mark_relative: 0.0,
        },
    }
}

pub(super) fn evidence(path: &str) -> FillCollectionFileEvidence {
    FillCollectionFileEvidence {
        path: path.to_string(),
        bytes: 1,
        sha256: "a".repeat(64),
    }
}

pub(super) fn account_boundary(
    path: &str,
    fingerprint: &str,
    start_server_ms: u64,
    finish_server_ms: u64,
    window_gap_ms: u64,
    cash_balance: f64,
) -> BoundAccountBoundary {
    let detail = OkxBalanceDetail {
        currency: "USDT".to_string(),
        update_time_ms: finish_server_ms,
        cash_balance: Some(cash_balance),
        available_balance: Some(cash_balance),
        equity: Some(cash_balance),
        equity_usd: Some(cash_balance),
        discounted_equity_usd: Some(cash_balance),
        unrealized_pnl: Some(0.0),
        liability: Some(0.0),
        cross_liability: Some(0.0),
        isolated_liability: None,
        unrealized_loss_liability: None,
        accrued_interest: Some(0.0),
        borrow_frozen_usd: Some(0.0),
        max_loan: Some(0.0),
        forced_repayment_indicator: Some(0),
    };
    BoundAccountBoundary {
        evidence: EconomicAccountBoundaryEvidence {
            certification_file: evidence(path),
            certification_schema_version: crate::ACCOUNT_CERTIFICATION_SCHEMA_VERSION,
            collector_reap_version: env!("CARGO_PKG_VERSION").to_string(),
            collector_executable_sha256: "c".repeat(64),
            collector_host_identity_sha256: "d".repeat(64),
            start_server_ms,
            finish_server_ms,
            window_gap_ms,
            total_equity_usd: cash_balance,
            balance_currencies: 1,
        },
        account_id: "main".to_string(),
        environment: TradingEnvironment::Demo,
        account_identity_sha256: "b".repeat(64),
        config_fingerprint: fingerprint.to_string(),
        config_source_path: "/config".to_string(),
        config_sha256: "a".repeat(64),
        passed: true,
        balance: OkxAccountBalanceSnapshot {
            update_time_ms: finish_server_ms,
            total_equity_usd: Some(cash_balance),
            adjusted_equity_usd: Some(cash_balance),
            borrow_frozen_usd: Some(0.0),
            notional_usd_for_borrow: Some(0.0),
            margin_ratio: None,
            notional_usd: Some(0.0),
            details: vec![detail],
        },
    }
}

pub(super) fn set_boundary_cash(boundary: &mut BoundAccountBoundary, value: f64) {
    boundary.evidence.total_equity_usd = value;
    boundary.balance.total_equity_usd = Some(value);
    boundary.balance.adjusted_equity_usd = Some(value);
    boundary.balance.details[0].cash_balance = Some(value);
    boundary.balance.details[0].available_balance = Some(value);
    boundary.balance.details[0].equity = Some(value);
    boundary.balance.details[0].equity_usd = Some(value);
    boundary.balance.details[0].discounted_equity_usd = Some(value);
}

pub(super) fn swap_fill() -> RemoteFill {
    RemoteFill {
        fill_id: "trade-1".to_string(),
        exchange_order_id: "exchange-1".to_string(),
        client_order_id: "reap-1".to_string(),
        symbol: "BTC-USDT-SWAP".to_string(),
        side: Side::Sell,
        price: 50_000.0,
        qty: 2.0,
        liquidity: FillLiquidity::Taker,
        fee: Some(FillFee {
            amount: -0.5,
            currency: "USDT".to_string(),
        }),
        ts_ms: TRADE_MS,
    }
}

pub(super) fn trade_bill() -> OkxBill {
    OkxBill {
        bill_id: "100".to_string(),
        bill_type: "2".to_string(),
        sub_type: "5".to_string(),
        timestamp_ms: TRADE_MS + 1,
        currency: "USDT".to_string(),
        balance_change: 19.5,
        balance: Some(1_000.0),
        position_balance_change: Some(0.0),
        position_balance: Some(0.0),
        quantity: Some(2.0),
        price: Some(50_000.0),
        pnl: Some(20.0),
        fee: Some(-0.5),
        interest: Some(0.0),
        instrument_type: Some(OkxInstrumentType::Swap),
        symbol: "BTC-USDT-SWAP".to_string(),
        margin_mode: Some(OkxBillMarginMode::Cross),
        order_id: "exchange-1".to_string(),
        client_order_id: "reap-1".to_string(),
        trade_id: "trade-1".to_string(),
        fill_time_ms: Some(TRADE_MS),
        execution_type: Some(OkxBillExecutionType::Taker),
        from_account: None,
        to_account: None,
        notes: String::new(),
    }
}

pub(super) fn funding_bill() -> OkxBill {
    OkxBill {
        bill_id: "200".to_string(),
        bill_type: "8".to_string(),
        sub_type: "173".to_string(),
        timestamp_ms: FUNDING_MS + 100,
        currency: "USDT".to_string(),
        balance_change: -4.0,
        balance: Some(996.0),
        position_balance_change: Some(0.0),
        position_balance: Some(0.0),
        quantity: Some(8.0),
        price: Some(50_000.0),
        pnl: Some(-4.0),
        fee: Some(0.0),
        interest: Some(0.0),
        instrument_type: Some(OkxInstrumentType::Swap),
        symbol: "BTC-USDT-SWAP".to_string(),
        margin_mode: Some(OkxBillMarginMode::Cross),
        order_id: String::new(),
        client_order_id: String::new(),
        trade_id: String::new(),
        fill_time_ms: Some(FUNDING_MS + 100),
        execution_type: None,
        from_account: None,
        to_account: None,
        notes: String::new(),
    }
}

pub(super) fn sources() -> BoundEconomicSources {
    let config = config();
    let fingerprint = config.fingerprint().unwrap();
    let strategy_name = config.strategy.strategy_name.clone();
    let mut recovered = RecoveredStorage {
        records: 9,
        ..RecoveredStorage::default()
    };
    recovered.bootstrap_identities.insert(
        "main".to_string(),
        (config.strategy.strategy_name.clone(), fingerprint.clone()),
    );
    BoundEconomicSources {
        account_id: "main".to_string(),
        config,
        config_file: evidence("/config"),
        journal: evidence("/journal"),
        recovered,
        account_bootstrap_records: 1,
        runtime_sessions: vec![JournalRuntimeSession {
            line: 2,
            started_at_ms: BEGIN_MS - 1_000,
            session_id: "1a2b3c".to_string(),
            account_id: "main".to_string(),
            strategy_name,
            config_fingerprint: fingerprint.clone(),
            account_identity_sha256: "b".repeat(64),
        }],
        authoritative_account_snapshots: vec![JournalAuthoritativeAccountSnapshot {
            line: 3,
            event_ts_ms: TRADE_MS - 100,
            update_ts_ms: TRADE_MS - 100,
            account_id: "main".to_string(),
            positions: vec![Position {
                symbol: "BTC-USDT-SWAP".to_string(),
                qty: 10.0,
                avg_price: 49_000.0,
                margin_mode: Some(reap_core::PositionMarginMode::Cross),
            }],
        }],
        journal_fills: vec![JournalFillObservation {
            line: 4,
            fill: FillRecord {
                ts_ms: TRADE_MS,
                account_id: Some("main".to_string()),
                fill_id: "trade-1".to_string(),
                order_id: "reap-1".to_string(),
                symbol: "BTC-USDT-SWAP".to_string(),
                side: Side::Sell,
                price: 50_000.0,
                qty: 2.0,
                liquidity: Some(FillLiquidity::Taker),
                fee: Some(FillFee {
                    amount: -0.5,
                    currency: "USDT".to_string(),
                }),
            },
        }],
        settlements: vec![JournalFundingSettlement {
            line: 6,
            event_ts_ms: FUNDING_MS + 50,
            symbol: "BTC-USDT-SWAP".to_string(),
            funding_time_ms: FUNDING_MS,
            rate: 0.001,
        }],
        position_observations: vec![
            JournalPositionObservation {
                line: 5,
                event_ts_ms: FUNDING_MS - 100,
                symbol: "BTC-USDT-SWAP".to_string(),
                quantity: 9.0,
            },
            JournalPositionObservation {
                line: 7,
                event_ts_ms: FUNDING_MS + 75,
                symbol: "BTC-USDT-SWAP".to_string(),
                quantity: 8.0,
            },
        ],
        mark_price_observations: vec![
            JournalMarkPriceObservation {
                line: 8,
                event_ts_ms: FUNDING_MS + 90,
                symbol: "BTC-USDT-SWAP".to_string(),
                price: 50_000.0,
            },
            JournalMarkPriceObservation {
                line: 9,
                event_ts_ms: FUNDING_MS + 110,
                symbol: "BTC-USDT-SWAP".to_string(),
                price: 50_000.0,
            },
        ],
        fill_manifest_file: evidence("/fills"),
        bill_manifest_file: evidence("/bills"),
        fills: vec![swap_fill()],
        bills: vec![trade_bill(), funding_bill()],
        environment: TradingEnvironment::Demo,
        account_identity_sha256: "b".repeat(64),
        config_fingerprint: fingerprint.clone(),
        window: BillCollectionWindow {
            begin_ms: BEGIN_MS,
            end_ms: END_MS,
            endpoints_inclusive: true,
            minimum_close_delay_ms: 1,
        },
        opening_account_boundary: account_boundary(
            "/opening-account",
            &fingerprint,
            BEGIN_MS - 2_000,
            BEGIN_MS - 1_000,
            1_000,
            980.5,
        ),
        closing_account_boundary: account_boundary(
            "/closing-account",
            &fingerprint,
            END_MS + 1_000,
            END_MS + 2_000,
            1_000,
            996.0,
        ),
    }
}
