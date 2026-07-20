use super::*;
use crate::Strategy;
use reap_core::{
    AccountUpdate, Balance, FillLiquidity, Level, MarginSnapshot, MarketEvent, NormalizedEvent,
    OrderBook, OrderEvent, OrderIntent, OrderStatus, OrderUpdate, SelfTradePrevention,
    StrategyEvent, SystemEvent, SystemEventKind, TimeInForce,
};

fn legacy_intents(intents: Vec<ChaosExecutionIntent>) -> Vec<OrderIntent> {
    intents
        .into_iter()
        .map(ChaosExecutionIntent::into_order_intent)
        .collect()
}

fn config() -> ChaosConfig {
    ChaosConfig {
        ref_symbol: "BTC-USDT".to_string(),
        delta_limit_usd: 50_000.0,
        active_hedge_threshold_usd: 1_000.0,
        min_hedge_interval_ms: 0,
        risk_groups: vec![RiskGroupConfig {
            name: "main".to_string(),
            symbols: vec!["BTC-USDT".to_string(), "BTC-PERP".to_string()],
            soft_delta_limit_usd: 25_000.0,
            hard_delta_limit_usd: 40_000.0,
            live_order_limit_usd: 100_000.0,
            ..RiskGroupConfig::default()
        }],
        instruments: vec![
            InstrumentConfig {
                symbol: "BTC-USDT".to_string(),
                risk_group: "main".to_string(),
                kind: InstrumentKindConfig::Spot,
                tick_size: 0.1,
                lot_size: 0.0001,
                min_trade_size: 0.0001,
                max_order_size_usd: 5_000.0,
                min_order_size_usd: 100.0,
                max_order_size: 1.0,
                ..InstrumentConfig::default()
            },
            InstrumentConfig {
                symbol: "BTC-PERP".to_string(),
                risk_group: "main".to_string(),
                kind: InstrumentKindConfig::Future,
                tick_size: 0.1,
                lot_size: 1.0,
                min_trade_size: 1.0,
                contract_value: 0.001,
                max_order_size_usd: 5_000.0,
                min_order_size_usd: 100.0,
                max_order_size: 200.0,
                min_position: -10_000.0,
                max_position: 10_000.0,
                ..InstrumentConfig::default()
            },
        ],
        ..ChaosConfig::default()
    }
}

fn java_calculator_config() -> ChaosConfig {
    let instrument = |symbol: &str,
                      kind: InstrumentKindConfig,
                      risk_group: &str,
                      contract_value: f64,
                      max_order_size: f64,
                      min_position: f64,
                      max_position: f64,
                      skew: f64| InstrumentConfig {
        symbol: symbol.to_string(),
        kind,
        risk_group: risk_group.to_string(),
        maker_fee: -0.001,
        taker_fee: 0.001,
        hedge_profit_margin: 0.0005,
        quote_profit_margin: 0.0005,
        hedge_aggression: 0.0003,
        min_order_size_usd: 1_000.0,
        max_order_size_usd: 4_000.0,
        max_order_size,
        min_trade_size: if kind.is_spot() { 0.01 } else { 1.0 },
        tick_size: 0.01,
        lot_size: if kind.is_spot() { 0.01 } else { 1.0 },
        contract_value,
        min_position,
        max_position,
        pos_skew: skew,
        neg_skew: skew,
        ..InstrumentConfig::default()
    };

    let mut instruments = vec![
        instrument(
            "BTC-USDT.OK",
            InstrumentKindConfig::Spot,
            "OKEX-Spot",
            1.0,
            0.08,
            -1_000.0,
            1_000.0,
            0.0,
        ),
        instrument(
            "BTC-USD-SWAP.OK",
            InstrumentKindConfig::InverseSwap,
            "OKEX-Invert",
            100.0,
            40.0,
            -1_000.0,
            1_000.0,
            0.000005,
        ),
        instrument(
            "BTC-USD-211231.OK",
            InstrumentKindConfig::InverseFuture,
            "OKEX-Invert",
            100.0,
            40.0,
            -1_000.0,
            1_000.0,
            0.000005,
        ),
        instrument(
            "BTC-USDT-SWAP.OK",
            InstrumentKindConfig::LinearSwap,
            "OKEX-Linear",
            0.01,
            8.0,
            -161.0,
            161.0,
            0.000031055900621118014,
        ),
        instrument(
            "BTC-USDT-211231.OK",
            InstrumentKindConfig::LinearFuture,
            "OKEX-Linear",
            0.01,
            8.0,
            -161.0,
            161.0,
            0.000031055900621118014,
        ),
    ];
    instruments
        .iter_mut()
        .find(|instrument| instrument.symbol == "BTC-USD-SWAP.OK")
        .unwrap()
        .hedge_priority = 1;
    instruments
        .iter_mut()
        .find(|instrument| instrument.symbol == "BTC-USDT-211231.OK")
        .unwrap()
        .hedge_priority = 1;

    ChaosConfig {
        strategy_name: "CalcTest".to_string(),
        underlying: "BTC".to_string(),
        ref_symbol: "BTC-USDT.OK".to_string(),
        delta_limit_usd: 50_000.0,
        active_hedge_threshold_usd: 800.0,
        min_hedge_interval_ms: 200,
        risk_groups: vec![
            RiskGroupConfig {
                name: "OKEX-Spot".to_string(),
                symbols: vec!["BTC-USDT.OK".to_string()],
                soft_delta_limit_usd: 20_000.0,
                hard_delta_limit_usd: 30_000.0,
                delta_stop_limit_usd: 50_000.0,
                live_order_limit_usd: 100_000.0,
                ..RiskGroupConfig::default()
            },
            RiskGroupConfig {
                name: "OKEX-Invert".to_string(),
                symbols: vec![
                    "BTC-USD-SWAP.OK".to_string(),
                    "BTC-USD-211231.OK".to_string(),
                ],
                soft_delta_limit_usd: 20_000.0,
                hard_delta_limit_usd: 30_000.0,
                delta_stop_limit_usd: 50_000.0,
                live_order_limit_usd: 100_000.0,
                ..RiskGroupConfig::default()
            },
            RiskGroupConfig {
                name: "OKEX-Linear".to_string(),
                symbols: vec![
                    "BTC-USDT-SWAP.OK".to_string(),
                    "BTC-USDT-211231.OK".to_string(),
                ],
                soft_delta_limit_usd: 20_000.0,
                hard_delta_limit_usd: 30_000.0,
                delta_stop_limit_usd: 50_000.0,
                live_order_limit_usd: 100_000.0,
                ..RiskGroupConfig::default()
            },
        ],
        instruments,
        ..ChaosConfig::default()
    }
}

fn seed_java_calculator_books(strategy: &mut ChaosStrategy) {
    for entity in strategy.entities.values_mut() {
        let qty = if entity.config.kind.is_spot() {
            0.2
        } else {
            20.0
        };
        entity.book = Some(OrderBook {
            symbol: entity.config.symbol.clone(),
            ts_ms: 1,
            bids: [45_000.0, 40_000.0, 35_000.0, 30_000.0, 25_000.0]
                .into_iter()
                .map(|px| Level::new(px, qty))
                .collect(),
            asks: [55_000.0, 60_000.0, 65_000.0, 70_000.0, 75_000.0]
                .into_iter()
                .map(|px| Level::new(px, qty))
                .collect(),
        });
    }
}

mod configuration;
mod execution_lifecycle;
mod hedge_selection;
mod public_trade;
mod quote_pricing;
mod reference_inputs;
mod risk_controls;

#[test]
fn normalized_fixture_typed_output_preserves_exact_ordered_intents() {
    let events = include_str!("../../../../fixtures/normalized/chaos_quote_hedge.jsonl")
        .lines()
        .filter(|line| !line.trim().is_empty())
        .map(|line| serde_json::from_str::<NormalizedEvent>(line).unwrap())
        .collect::<Vec<_>>();
    let mut strategy = ChaosStrategy::new(config()).unwrap();

    let mut typed_by_event = Vec::new();
    for event in events {
        typed_by_event.push(strategy.on_execution_event(&event.into_strategy_event()));
    }
    let purposes = typed_by_event
        .iter()
        .flatten()
        .map(ChaosExecutionIntent::purpose)
        .collect::<Vec<_>>();
    assert!(purposes.contains(&crate::ChaosExecutionPurpose::Quote));
    assert!(purposes.contains(&crate::ChaosExecutionPurpose::Hedge));

    let lowered = typed_by_event
        .into_iter()
        .map(legacy_intents)
        .collect::<Vec<_>>();
    let expected: serde_json::Value = serde_json::from_str(include_str!(
        "../../../../fixtures/normalized/chaos_quote_hedge_intents.json"
    ))
    .unwrap();
    assert_eq!(serde_json::to_value(lowered).unwrap(), expected);
}
