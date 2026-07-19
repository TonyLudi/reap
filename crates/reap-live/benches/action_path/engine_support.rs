use super::*;

pub(super) fn count_engine_output_and_select(
    output: ChaosEngineOutput,
    purpose: ChaosExecutionPurpose,
    cancel_order_id: Option<&str>,
) -> (LogicalCounters, Option<ChaosExecutionIntent>) {
    let mut counters = count_engine_output(&output);
    let mut selected = None;
    for intent in output.intents {
        let matches_id = cancel_order_id.is_none_or(|expected| {
            intent
                .as_cancel_owned()
                .is_some_and(|cancel| cancel.order_id() == expected)
        });
        if selected.is_none() && intent.purpose() == purpose && matches_id {
            selected = Some(intent);
        }
    }
    counters.inputs = 1;
    counters.normalized_outputs = 1;
    (counters, selected)
}

pub(super) fn count_engine_output(output: &ChaosEngineOutput) -> LogicalCounters {
    let produced_actions = (output.intents.len() + output.safety_cancel_candidates.len()) as u64;
    let mut counters = LogicalCounters {
        inputs: 1,
        normalized_outputs: 1,
        typed_intents: output.intents.len() as u64,
        risk_rejections: output.rejected.len() as u64,
        system_events: output.system_events.len() as u64,
        safety_cancel_candidates: output.safety_cancel_candidates.len() as u64,
        produced_actions,
        ..LogicalCounters::default()
    };
    for intent in &output.intents {
        match intent.purpose() {
            ChaosExecutionPurpose::Quote => counters.quote_intents += 1,
            ChaosExecutionPurpose::Hedge => counters.hedge_intents += 1,
            ChaosExecutionPurpose::CancelOwned => counters.cancel_owned_intents += 1,
        }
    }
    counters
}

pub(super) fn counters_from_coordinator_output(records: &[StorageRecord]) -> LogicalCounters {
    let mut counters = LogicalCounters::default();
    for record in records {
        match record {
            StorageRecord::Intent { intent, .. } => {
                counters.typed_intents += 1;
                match intent {
                    reap_core::OrderIntent::NewOrder(order)
                        if order.time_in_force == TimeInForce::Ioc =>
                    {
                        counters.hedge_intents += 1;
                    }
                    reap_core::OrderIntent::NewOrder(_) => counters.quote_intents += 1,
                    reap_core::OrderIntent::CancelOrder { .. } => {
                        counters.cancel_owned_intents += 1;
                    }
                }
            }
            StorageRecord::IntentRejected { .. } => counters.risk_rejections += 1,
            StorageRecord::System(_) => counters.system_events += 1,
            StorageRecord::SafetyLatch(_) => counters.safety_cancel_candidates += 1,
            StorageRecord::Raw { .. }
            | StorageRecord::Normalized(_)
            | StorageRecord::Bootstrap(_)
            | StorageRecord::SessionStart(_)
            | StorageRecord::AccountSnapshot(_)
            | StorageRecord::OrderRequest(_)
            | StorageRecord::OrderAck(_)
            | StorageRecord::Order { .. }
            | StorageRecord::Fill(_)
            | StorageRecord::Reconciliation(_) => {}
        }
    }
    counters
}

pub(super) fn permissive_risk_limits() -> RiskLimits {
    RiskLimits {
        require_feed_health: false,
        require_private_health: false,
        ..RiskLimits::default()
    }
}

pub(super) fn benchmark_engine(limits: RiskLimits) -> TradingEngine<ChaosStrategy> {
    let config: ChaosConfig = toml::from_str(include_str!("../../../../examples/iarb2-basic.toml"))
        .expect("benchmark strategy configuration");
    let strategy = ChaosStrategy::new(config).expect("benchmark strategy must validate");
    let mut risk = RiskGate::new(limits);
    assert!(risk.set_instrument_model("BTC-USDT", InstrumentRiskModel::Spot));
    assert!(risk.set_instrument_model(
        "BTC-PERP",
        InstrumentRiskModel::LinearDerivative {
            contract_value: 0.001,
        },
    ));
    assert!(risk.set_instrument_order_limits(
        "BTC-USDT",
        InstrumentOrderLimits {
            max_limit_quantity: 100.0,
            max_limit_notional_usd: Some(1_000_000.0),
        },
    ));
    assert!(risk.set_instrument_order_limits(
        "BTC-PERP",
        InstrumentOrderLimits {
            max_limit_quantity: 1_000_000.0,
            max_limit_notional_usd: None,
        },
    ));
    TradingEngine::new(strategy, risk)
}

pub(super) fn depth_event(
    symbol: &str,
    ts_ms: u64,
    bid: f64,
    ask: f64,
    quantity: f64,
) -> NormalizedEvent {
    NormalizedEvent::Market(MarketEvent::Depth(OrderBook::one_level(
        symbol,
        ts_ms,
        Level::new(bid, quantity),
        Level::new(ask, quantity),
    )))
}

pub(super) fn account_position_event(ts_ms: u64, quantity: f64) -> NormalizedEvent {
    NormalizedEvent::Account(AccountUpdate {
        ts_ms,
        balances: Vec::new(),
        positions: vec![Position {
            symbol: "BTC-USDT".to_string(),
            qty: quantity,
            avg_price: 50_000.0,
            margin_mode: None,
        }],
        margins: Vec::new(),
    })
}

pub(super) fn pending_update(client_order_id: &str, order: &NewOrder, ts_ms: u64) -> OrderUpdate {
    OrderUpdate {
        ts_ms,
        order_id: client_order_id.to_string(),
        symbol: order.symbol.clone(),
        side: order.side,
        event: OrderEvent::PendingNew,
        status: OrderStatus::PendingNew,
        price: order.price,
        time_in_force: Some(order.time_in_force),
        qty: order.qty,
        open_qty: order.qty,
        filled_qty: 0.0,
        avg_fill_price: 0.0,
        last_fill_qty: 0.0,
        last_fill_price: 0.0,
        last_fill_liquidity: None,
        last_fill_fee: None,
        reason: order.reason.clone(),
    }
}
