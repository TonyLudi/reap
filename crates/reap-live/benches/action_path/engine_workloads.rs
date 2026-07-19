use super::*;

struct EnginePreparationScenario {
    engine: TradingEngine<ChaosStrategy>,
    event: NormalizedEvent,
    preparation: PreparationRig,
}

pub(super) fn quote_creation_workload() -> WorkloadResult {
    let result = run_workload(
        "quote_creation_decision_and_preparation",
        "owned normalized event delivery; production ChaosStrategy; \
         TradingEngine<ChaosStrategy>; RiskGate decisions; typed intent traversal; regular \
         policy; canonical reservation; gateway idempotency; PreparedRegularSubmit for the first \
         production-emitted quote purpose",
        ENGINE_EXCLUDED,
        || {
            let mut engine = benchmark_engine(RiskLimits {
                require_feed_health: false,
                require_private_health: false,
                ..RiskLimits::default()
            });
            black_box(engine.on_chaos_event(depth_event("BTC-USDT", 1, 50_000.0, 50_001.0, 2.0)));
            EnginePreparationScenario {
                engine,
                event: depth_event("BTC-PERP", 1, 50_003.0, 50_004.0, 10_000.0),
                preparation: PreparationRig::new(),
            }
        },
        |scenario, index| {
            let output = scenario.engine.on_chaos_event(scenario.event.clone());
            let (mut counters, selected) =
                count_engine_output_and_select(output, ChaosExecutionPurpose::Quote, None);
            if let Some(intent) = selected {
                scenario.preparation.prepare_submit(intent, index);
                counters.prepared_submits = 1;
            }
            Observation {
                counters,
                queue_age_ns: None,
            }
        },
    );
    assert!(result.counters.quote_intents >= TIMED_OBSERVATIONS as u64);
    assert_eq!(result.counters.prepared_submits, TIMED_OBSERVATIONS as u64);
    result
}

struct ReplacementScenario {
    engine: TradingEngine<ChaosStrategy>,
    event: NormalizedEvent,
    owned_id: String,
    preparation: PreparationRig,
}

pub(super) fn quote_replacement_workload() -> WorkloadResult {
    let result = run_workload(
        "quote_replacement_owned_cancel_preparation",
        "owned normalized event delivery; production ChaosStrategy; \
         TradingEngine<ChaosStrategy>; RiskGate decisions; typed intent traversal; a \
         production-created quote registered PendingNew; typed CancelOwned replacement; \
         ownership policy proof; PreparedRegularCancel",
        ENGINE_EXCLUDED,
        || {
            let mut engine = benchmark_engine(permissive_risk_limits());
            black_box(engine.on_chaos_event(depth_event("BTC-USDT", 1, 50_000.0, 50_001.0, 2.0)));
            let initial =
                engine.on_chaos_event(depth_event("BTC-PERP", 1, 50_003.0, 50_004.0, 10_000.0));
            let quote = initial
                .intents
                .into_iter()
                .find(|intent| {
                    intent.as_quote().is_some_and(|quote| {
                        quote.symbol() == "BTC-USDT" && quote.side() == Side::Sell
                    })
                })
                .expect("fixture must emit a BTC-USDT sell quote");
            let mut preparation = PreparationRig::new();
            let seeded = preparation.seed_submit(quote, 0);
            black_box(engine.on_chaos_event(NormalizedEvent::Order(pending_update(
                &seeded.client_order_id,
                &seeded.order,
                2,
            ))));
            ReplacementScenario {
                engine,
                event: depth_event("BTC-PERP", 3, 51_003.0, 51_004.0, 10_000.0),
                owned_id: seeded.client_order_id,
                preparation,
            }
        },
        |scenario, _| {
            let output = scenario.engine.on_chaos_event(scenario.event.clone());
            let (mut counters, selected) = count_engine_output_and_select(
                output,
                ChaosExecutionPurpose::CancelOwned,
                Some(&scenario.owned_id),
            );
            if let Some(ChaosExecutionIntent::CancelOwned(cancel)) = selected {
                scenario
                    .preparation
                    .prepare_cancel(cancel.order_id(), cancel.reason());
                counters.prepared_cancels = 1;
            }
            Observation {
                counters,
                queue_age_ns: None,
            }
        },
    );
    assert!(result.counters.cancel_owned_intents >= TIMED_OBSERVATIONS as u64);
    assert_eq!(result.counters.prepared_cancels, TIMED_OBSERVATIONS as u64);
    result
}

struct HedgeScenario {
    engine: TradingEngine<ChaosStrategy>,
    account_positive: NormalizedEvent,
    account_negative: NormalizedEvent,
    preparation: PreparationRig,
}

pub(super) fn ioc_hedge_workload() -> WorkloadResult {
    let result = run_workload(
        "ioc_hedge_decision_and_preparation",
        "owned normalized event delivery; production ChaosStrategy; \
         TradingEngine<ChaosStrategy>; RiskGate decisions; typed intent traversal; alternating \
         production account-position reductions; IOC CancelMaker hedge purpose; regular policy; \
         canonical reservation; gateway idempotency; PreparedRegularSubmit",
        ENGINE_EXCLUDED,
        || {
            let mut engine = benchmark_engine(permissive_risk_limits());
            black_box(engine.on_chaos_event(depth_event("BTC-USDT", 1, 50_000.0, 50_001.0, 2.0)));
            black_box(
                engine.on_chaos_event(depth_event("BTC-PERP", 1, 50_003.0, 50_004.0, 10_000.0)),
            );
            HedgeScenario {
                engine,
                account_positive: account_position_event(2, 0.1),
                account_negative: account_position_event(2, -0.1),
                preparation: PreparationRig::new(),
            }
        },
        |scenario, index| {
            let event = if index.is_multiple_of(2) {
                &scenario.account_positive
            } else {
                &scenario.account_negative
            };
            let output = scenario.engine.on_chaos_event(event.clone());
            let (mut counters, selected) =
                count_engine_output_and_select(output, ChaosExecutionPurpose::Hedge, None);
            if let Some(intent) = selected {
                scenario.preparation.prepare_submit(intent, index);
                counters.prepared_submits = 1;
            }
            Observation {
                counters,
                queue_age_ns: None,
            }
        },
    );
    assert!(result.counters.hedge_intents >= TIMED_OBSERVATIONS as u64);
    assert_eq!(result.counters.prepared_submits, TIMED_OBSERVATIONS as u64);
    result
}

pub(super) fn risk_rejection_workload() -> WorkloadResult {
    let result = run_workload(
        "risk_rejection",
        ENGINE_INCLUDED,
        ENGINE_EXCLUDED,
        || {
            let mut engine = benchmark_engine(RiskLimits {
                max_order_notional_usd: 1.0,
                require_feed_health: false,
                require_private_health: false,
                ..RiskLimits::default()
            });
            black_box(engine.on_chaos_event(depth_event("BTC-USDT", 1, 50_000.0, 50_001.0, 2.0)));
            (
                engine,
                depth_event("BTC-PERP", 1, 50_003.0, 50_004.0, 10_000.0),
            )
        },
        |(engine, event), _| {
            let output = engine.on_chaos_event(event.clone());
            let (counters, _) =
                count_engine_output_and_select(output, ChaosExecutionPurpose::Quote, None);
            Observation {
                counters,
                queue_age_ns: None,
            }
        },
    );
    assert!(result.counters.risk_rejections >= TIMED_OBSERVATIONS as u64);
    assert_eq!(result.counters.typed_intents, 0);
    result
}

struct FailCloseScenario {
    engine: TradingEngine<ChaosStrategy>,
    event: NormalizedEvent,
    preparation: PreparationRig,
}

pub(super) fn symbol_fail_close_workload() -> WorkloadResult {
    let result = fail_close_workload(
        "symbol_fail_close_owned_cancel_preparation",
        SystemEventKind::SymbolHalted,
        Some("BTC-USDT"),
    );
    assert_eq!(
        result.counters.safety_cancel_candidates,
        TIMED_OBSERVATIONS as u64
    );
    assert_eq!(result.counters.prepared_cancels, TIMED_OBSERVATIONS as u64);
    result
}

pub(super) fn global_fail_close_workload() -> WorkloadResult {
    let result = fail_close_workload(
        "global_fail_close_owned_cancel_preparation",
        SystemEventKind::KillSwitchActivated,
        None,
    );
    assert_eq!(
        result.counters.safety_cancel_candidates,
        (TIMED_OBSERVATIONS * 2) as u64
    );
    assert_eq!(
        result.counters.prepared_cancels,
        (TIMED_OBSERVATIONS * 2) as u64
    );
    result
}

fn fail_close_workload(
    name: &'static str,
    kind: SystemEventKind,
    symbol: Option<&'static str>,
) -> WorkloadResult {
    run_workload(
        name,
        "owned normalized event delivery; production ChaosStrategy; \
         TradingEngine<ChaosStrategy>; RiskGate decisions; typed intent traversal; canonical \
         live-order risk state; risk-synthesized safety-cancel candidates; ownership proof; \
         PreparedRegularCancel",
        ENGINE_EXCLUDED,
        move || {
            let mut engine = benchmark_engine(permissive_risk_limits());
            black_box(engine.on_chaos_event(depth_event("BTC-USDT", 1, 50_000.0, 50_001.0, 2.0)));
            let initial =
                engine.on_chaos_event(depth_event("BTC-PERP", 1, 50_003.0, 50_004.0, 10_000.0));
            let mut quote_by_symbol = HashMap::new();
            for intent in initial.intents {
                if let Some(quote) = intent.as_quote()
                    && !quote_by_symbol.contains_key(quote.symbol())
                {
                    quote_by_symbol.insert(quote.symbol().to_string(), intent);
                }
            }
            let mut preparation = PreparationRig::new();
            for (ordinal, live_symbol) in ["BTC-USDT", "BTC-PERP"].into_iter().enumerate() {
                let intent = quote_by_symbol
                    .remove(live_symbol)
                    .expect("fixture must quote both symbols");
                let seeded = preparation.seed_submit(intent, ordinal);
                black_box(engine.on_chaos_event(NormalizedEvent::Order(pending_update(
                    &seeded.client_order_id,
                    &seeded.order,
                    2,
                ))));
            }
            FailCloseScenario {
                engine,
                event: NormalizedEvent::System(SystemEvent {
                    ts_ms: 3,
                    kind,
                    venue: None,
                    account_id: None,
                    symbol: symbol.map(str::to_string),
                    reason: "action-path deterministic fail-close workload".to_string(),
                }),
                preparation,
            }
        },
        |scenario, _| {
            let output = scenario.engine.on_chaos_event(scenario.event.clone());
            let mut counters = count_engine_output(&output);
            for candidate in output.safety_cancel_candidates {
                scenario
                    .preparation
                    .prepare_cancel(candidate.order_id(), candidate.reason());
                counters.prepared_cancels += 1;
            }
            Observation {
                counters,
                queue_age_ns: None,
            }
        },
    )
}

struct TradeRepriceBaselineScenario {
    engine: TradingEngine<ChaosStrategy>,
    aggressive_trade: NormalizedEvent,
}

pub(super) fn trade_reprice_zero_baseline_workload() -> WorkloadResult {
    let result = run_workload(
        "public_trade_reprice_zero_baseline",
        "bench-declared monotonic replay arrival and arrival+100,000ns deadline; production \
         TradingEngine<ChaosStrategy>/RiskGate delivery of a taker-Buy trade that strictly \
         crosses a three-level raw ask book and whose exact-half quantity moves Java's implied \
         first-valid level from 50,001 to 50,003",
        "wire parsing; feed dedup/reduction; private deferred-action scheduling and due-action \
         service (the missing Phase 0 behavior); live runtime queues; coordinator/storage; \
         gateway preparation; adapter serialization; disk/network/acknowledgement",
        || {
            let mut engine = benchmark_engine(permissive_risk_limits());
            black_box(
                engine.on_chaos_event(NormalizedEvent::Market(MarketEvent::Depth(OrderBook {
                    symbol: "BTC-USDT".to_string(),
                    ts_ms: 1,
                    bids: [50_000.0, 49_999.0, 49_998.0]
                        .map(|px| Level::new(px, 10.0))
                        .to_vec(),
                    asks: [50_001.0, 50_002.0, 50_003.0]
                        .map(|px| Level::new(px, 10.0))
                        .to_vec(),
                }))),
            );
            black_box(
                engine.on_chaos_event(depth_event("BTC-PERP", 1, 50_003.0, 50_004.0, 10_000.0)),
            );
            TradeRepriceBaselineScenario {
                engine,
                aggressive_trade: NormalizedEvent::Market(MarketEvent::Trade {
                    ts_ms: 2,
                    symbol: "BTC-USDT".to_string(),
                    price: 50_002.0,
                    qty: 5.0,
                    taker_side: Side::Buy,
                }),
            }
        },
        |scenario, index| {
            let arrival_ns = 1_000_000_000_u64
                .checked_add((index as u64).saturating_mul(1_000_000))
                .expect("benchmark arrival timeline");
            let expected_due_ns = arrival_ns
                .checked_add(100_000)
                .expect("trade-reprice deadline");
            black_box((arrival_ns, expected_due_ns));
            let output = scenario
                .engine
                .on_chaos_event(scenario.aggressive_trade.clone());
            let counters = count_engine_output(&output);
            black_box(output);
            Observation {
                counters,
                queue_age_ns: None,
            }
        },
    );
    assert_eq!(result.counters.typed_intents, 0);
    assert_eq!(
        result.counters.trade_reprice_actions, 0,
        "Phase 0 must preserve and record the missing due trade-reprice action"
    );
    result
}
