use reap_core::{Level, NewOrder, OrderBook, OrderStatus, Side, TimeInForce, TimerEvent};
use reap_strategy::{InstrumentConfig, InstrumentKindConfig, RiskGroupConfig};

use super::*;

fn config() -> ChaosConfig {
    ChaosConfig {
        ref_symbol: "BTC-USDT".to_string(),
        active_hedge_threshold_usd: 500.0,
        min_hedge_interval_ms: 0,
        risk_groups: vec![RiskGroupConfig {
            name: "main".to_string(),
            symbols: vec!["BTC-USDT".to_string(), "BTC-PERP".to_string()],
            soft_delta_limit_usd: 50_000.0,
            hard_delta_limit_usd: 75_000.0,
            delta_stop_limit_usd: 100_000.0,
            live_order_limit_usd: 100_000.0,
            ..RiskGroupConfig::default()
        }],
        instruments: vec![
            InstrumentConfig {
                symbol: "BTC-USDT".to_string(),
                risk_group: "main".to_string(),
                kind: InstrumentKindConfig::Spot,
                max_order_size_usd: 5_000.0,
                min_order_size_usd: 100.0,
                max_order_size: 1.0,
                tick_size: 0.1,
                lot_size: 0.0001,
                ..InstrumentConfig::default()
            },
            InstrumentConfig {
                symbol: "BTC-PERP".to_string(),
                risk_group: "main".to_string(),
                kind: InstrumentKindConfig::Future,
                contract_value: 0.001,
                max_order_size_usd: 10_000.0,
                min_order_size_usd: 100.0,
                max_order_size: 10_000.0,
                min_trade_size: 1.0,
                lot_size: 1.0,
                min_position: -100_000.0,
                max_position: 100_000.0,
                ..InstrumentConfig::default()
            },
        ],
        ..ChaosConfig::default()
    }
}

fn initial_books() -> Vec<NormalizedEvent> {
    vec![
        NormalizedEvent::from(MarketEvent::Depth(OrderBook::one_level(
            "BTC-USDT",
            1,
            Level::new(50_000.0, 2.0),
            Level::new(50_001.0, 2.0),
        ))),
        NormalizedEvent::from(MarketEvent::Depth(OrderBook::one_level(
            "BTC-PERP",
            1,
            Level::new(50_003.0, 10_000.0),
            Level::new(50_004.0, 10_000.0),
        ))),
    ]
}

fn usdt_config() -> ChaosConfig {
    let mut config = config();
    for instrument in &mut config.instruments {
        instrument.base_currency = "BTC".to_string();
        instrument.quote_currency = "USDT".to_string();
        instrument.taker_fee = 0.0;
        if instrument.kind.is_derivative() {
            instrument.settle_currency = "USDT".to_string();
        }
    }
    config
}

fn usdt_execution(market_data_latency_ms: u64, max_age_ms: u64) -> BacktestExecutionConfig {
    BacktestExecutionConfig {
        market_data_latency_ms,
        currency_rates: vec![BacktestCurrencyRateConfig {
            currency: "USDT".to_string(),
            index_symbol: "USDT-USD".to_string(),
            max_age_ms,
        }],
        ..BacktestExecutionConfig::default()
    }
}

fn external_spot_fill(ts_ms: u64, price: f64) -> NormalizedEvent {
    NormalizedEvent::Order(OrderUpdate {
        ts_ms,
        order_id: "external-fill".to_string(),
        symbol: "BTC-USDT".to_string(),
        side: Side::Buy,
        event: OrderEvent::FullyFilled,
        status: OrderStatus::Filled,
        price,
        time_in_force: Some(TimeInForce::Ioc),
        qty: 1.0,
        open_qty: 0.0,
        filled_qty: 1.0,
        avg_fill_price: price,
        last_fill_qty: 1.0,
        last_fill_price: price,
        last_fill_liquidity: Some(FillLiquidity::Taker),
        last_fill_fee: None,
        reason: "external-test-fill".to_string(),
    })
}

fn seed_perp_matcher(runner: &mut BacktestRunner, ts_ms: u64) {
    runner.matcher_mut("BTC-PERP").unwrap().on_depth_at(
        OrderBook::one_level(
            "BTC-PERP",
            ts_ms,
            Level::new(100.0, 10_000.0),
            Level::new(101.0, 10_000.0),
        ),
        ts_ms,
    );
}

mod carry;
mod funding;
mod orders;
mod public_trade;
mod replay;
mod structure;
mod valuation;
