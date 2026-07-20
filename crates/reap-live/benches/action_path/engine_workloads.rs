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
        PREPARED_ACTION_EXCLUDED,
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
        PREPARED_ACTION_EXCLUDED,
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
        PREPARED_ACTION_EXCLUDED,
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
        PREPARED_ACTION_EXCLUDED,
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

pub(super) fn public_trade_reprice_workload() -> WorkloadResult {
    const BASE_TS_MS: u64 = 1_000;
    let result = run_workload(
        "public_trade_implied_depth_reprice",
        "bench-declared monotonic replay arrival and arrival+100,000ns deadline; production \
         TradingEngine<ChaosStrategy>/RiskGate delivery of a taker-Buy trade that strictly \
         crosses a three-level raw ask book and whose exact-half quantity moves Java's implied \
         first-valid level from 50,001 to 50,003; private bounded callback scheduling and exact-due \
         service through the production strategy/engine/risk path",
        "wire parsing; feed dedup/reduction; live runtime queues and select; \
         coordinator/storage; gateway preparation; adapter serialization; \
        disk/network/acknowledgement",
        || {
            let mut engine = benchmark_engine_with_config(permissive_risk_limits(), |config| {
                // The synthetic clock advances for 100,000 independent
                // observations without replaying depth. Disable only that
                // benchmark artifact so the workload continues to time
                // public-trade scheduling and repricing rather than
                // manufactured book staleness.
                for instrument in &mut config.instruments {
                    instrument.depth_stale_threshold_ms = u64::MAX;
                }
            });
            black_box(
                engine.on_chaos_event_at(
                    NormalizedEvent::Market(MarketEvent::Depth(OrderBook {
                        symbol: "BTC-USDT".to_string(),
                        ts_ms: BASE_TS_MS,
                        bids: [50_000.0, 49_999.0, 49_998.0]
                            .map(|px| Level::new(px, 10.0))
                            .to_vec(),
                        asks: [50_001.0, 50_002.0, 50_003.0]
                            .map(|px| Level::new(px, 10.0))
                            .to_vec(),
                    })),
                    900_000_000,
                    BASE_TS_MS,
                    true,
                ),
            );
            black_box(engine.on_chaos_event_at(
                depth_event("BTC-PERP", BASE_TS_MS + 5, 50_003.0, 50_004.0, 10_000.0),
                905_000_000,
                BASE_TS_MS + 5,
                true,
            ));
            TradeRepriceBaselineScenario {
                engine,
                aggressive_trade: NormalizedEvent::Market(MarketEvent::Trade {
                    ts_ms: BASE_TS_MS + 6,
                    symbol: "BTC-USDT".to_string(),
                    price: 50_002.0,
                    qty: 5.0,
                    taker_side: Side::Buy,
                }),
            }
        },
        |scenario, index| {
            let arrival_ns = 1_000_000_000_u64
                .checked_add((index as u64).saturating_mul(10_000_000))
                .expect("benchmark arrival timeline");
            let expected_due_ns = arrival_ns
                .checked_add(100_000)
                .expect("trade-reprice deadline");
            let observed_now_ms = BASE_TS_MS
                .checked_add(10)
                .and_then(|base| base.checked_add((index as u64).saturating_mul(10)))
                .expect("benchmark millisecond timeline");
            let delivered = scenario.engine.on_chaos_event_at(
                scenario.aggressive_trade.clone(),
                arrival_ns,
                observed_now_ms,
                true,
            );
            assert!(delivered.intents.is_empty());
            assert_eq!(
                scenario.engine.next_chaos_trade_reprice_due_ns(),
                Some(expected_due_ns)
            );

            let repriced = scenario.engine.service_one_due_chaos_trade_reprice(
                expected_due_ns,
                observed_now_ms,
                true,
            );
            let mut counters = count_engine_output(&delivered);
            let mut reprice_counters = count_engine_output(&repriced);
            reprice_counters.inputs = 0;
            reprice_counters.normalized_outputs = 0;
            counters.merge(reprice_counters);
            counters.trade_reprice_actions = 1;
            black_box((delivered, repriced));
            Observation {
                counters,
                queue_age_ns: None,
            }
        },
    );
    assert_eq!(
        result.counters.trade_reprice_actions, TIMED_OBSERVATIONS as u64,
        "Phase 2 must service exactly one due trade-reprice action per input"
    );
    assert!(
        result.counters.typed_intents >= TIMED_OBSERVATIONS as u64,
        "the pinned crossing fixture must produce a nonzero reprice result"
    );
    result
}
