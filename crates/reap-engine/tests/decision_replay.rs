#[path = "support/decision_replay.rs"]
mod support;

use std::collections::BTreeMap;

use reap_core::{
    AccountUpdate, Balance, FillLiquidity, Level, MarginSnapshot, MarketEvent, NormalizedEvent,
    OrderBook, OrderEvent, OrderIntent, OrderStatus, OrderUpdate, Position, PositionMarginMode,
    Side, SystemEvent, SystemEventKind, TimeInForce, TimerEvent, Venue,
};
use reap_engine::{ChaosEngineOutput, TradingEngine};
use reap_risk::{InstrumentOrderLimits, InstrumentRiskModel, RiskLimits, StablecoinGuardConfig};
use reap_strategy::{ChaosConfig, ChaosStrategy};

use support::{
    AccountEquityState, AccountInitialization, DeclaredDecisionState, FeedHealthState,
    InitializationArtifactV1, InstrumentInitialization, LiveInitialization, NumericSymbolState,
    PrivateHealthState, ReplayEnvelope, ReplayInput, SeedEvent, SeedRoute, SourceClockState,
    StablecoinRateState, StrategyReferenceState, build_engine, canonical_jsonl,
    parse_initialization, parse_replay_jsonl, replay_engine, risk_limit_keys,
};

const INITIALIZATION_BYTES: &[u8] =
    include_bytes!("../../../fixtures/decision_parity/risk_initialization_v1.json");
const REPLAY_BYTES: &[u8] =
    include_bytes!("../../../fixtures/decision_parity/replay_events_v1.jsonl");
const EXPECTED_ENGINE_BYTES: &[u8] =
    include_bytes!("../../../fixtures/decision_parity/expected_engine_v1.jsonl");

fn expected_initialization() -> InitializationArtifactV1 {
    let mut strategy: ChaosConfig =
        toml::from_str(include_str!("../../../examples/iarb2-basic.toml")).unwrap();
    strategy.reference_data_stale_threshold_ms = Some(120_000);
    strategy.risk_groups[0].account_id = Some("main".to_string());
    for instrument in &mut strategy.instruments {
        instrument.debounce_width = 0.0;
        instrument.debounce_size_usd = 0.0;
        instrument.debounce_ms = 0;
    }
    let strategy = strategy.effective();
    let risk_limits = RiskLimits {
        max_order_notional_usd: 25_000.0,
        max_abs_position_notional_usd: 100_000.0,
        max_live_order_notional_usd: 100_000.0,
        max_live_order_count: 256,
        max_live_order_count_per_symbol: 64,
        order_reject_count_limit: 10,
        order_reject_count_per_symbol_limit: 5,
        order_reject_window_ms: 60_000,
        unfilled_ioc_cancel_count_per_symbol_limit: 10,
        unfilled_ioc_cancel_window_ms: 60_000,
        max_turnover_usd: 1_000_000.0,
        max_drawdown_usd: 10_000.0,
        max_feed_age_ms: 1_000,
        max_private_age_ms: 2_000,
        require_feed_health: true,
        require_private_health: true,
        stablecoin_guards: vec![StablecoinGuardConfig {
            symbol: "USDT-USD".to_string(),
            max_downside_deviation: 0.01,
        }],
        stablecoin_max_age_ms: 75_000,
        stablecoin_breach_debounce_ms: 5_000,
        forced_repayment_indicator_limit: 1,
    };
    let account_update = AccountUpdate {
        ts_ms: 1,
        balances: vec![Balance {
            account_id: Some("main".to_string()),
            currency: "USDT".to_string(),
            total: 10_000.0,
            available: 10_000.0,
            equity: 10_000.0,
            liability: 0.0,
            max_loan: 0.0,
            forced_repayment_indicator: None,
        }],
        positions: vec![
            Position {
                symbol: "BTC-USDT".to_string(),
                qty: 0.0,
                avg_price: 0.0,
                margin_mode: None,
            },
            Position {
                symbol: "BTC-PERP".to_string(),
                qty: 0.0,
                avg_price: 0.0,
                margin_mode: Some(PositionMarginMode::Cross),
            },
        ],
        margins: vec![MarginSnapshot {
            account_id: Some("main".to_string()),
            ratio: Some(100.0),
            exchange_ratio: Some(100.0),
            adjusted_equity_usd: Some(10_000.0),
            notional_usd: Some(0.0),
        }],
    };
    let seed_events = vec![
        seed(
            1,
            1_000_000,
            1,
            SeedRoute::PrivateAccount,
            NormalizedEvent::Account(account_update.clone()),
        ),
        seed(
            2,
            2_000_000,
            2,
            SeedRoute::Normalized,
            NormalizedEvent::Market(MarketEvent::IndexPrice {
                ts_ms: 2,
                symbol: "USDT-USD".to_string(),
                price: 1.0,
            }),
        ),
        seed(
            3,
            2_100_000,
            2,
            SeedRoute::Normalized,
            NormalizedEvent::Market(MarketEvent::PriceLimits {
                ts_ms: 2,
                symbol: "BTC-USDT".to_string(),
                mark_price: 50_000.5,
                limit_down: 1.0,
                limit_up: 1_000_000.0,
            }),
        ),
        seed(
            4,
            2_200_000,
            2,
            SeedRoute::Normalized,
            NormalizedEvent::Market(MarketEvent::PriceLimits {
                ts_ms: 2,
                symbol: "BTC-PERP".to_string(),
                mark_price: 50_003.5,
                limit_down: 1.0,
                limit_up: 1_000_000.0,
            }),
        ),
        seed(
            5,
            3_000_000,
            3,
            SeedRoute::Normalized,
            NormalizedEvent::Market(MarketEvent::Depth(OrderBook {
                symbol: "BTC-USDT".to_string(),
                ts_ms: 3,
                bids: vec![Level {
                    px: 50_000.0,
                    qty: 2.0,
                }],
                asks: vec![Level {
                    px: 50_001.0,
                    qty: 2.0,
                }],
            })),
        ),
        seed(
            6,
            3_100_000,
            3,
            SeedRoute::Normalized,
            NormalizedEvent::Market(MarketEvent::Depth(OrderBook {
                symbol: "BTC-PERP".to_string(),
                ts_ms: 3,
                bids: vec![Level {
                    px: 50_003.0,
                    qty: 10_000.0,
                }],
                asks: vec![Level {
                    px: 50_004.0,
                    qty: 10_000.0,
                }],
            })),
        ),
        seed(
            7,
            4_000_000,
            4,
            SeedRoute::Normalized,
            NormalizedEvent::System(SystemEvent {
                ts_ms: 4,
                kind: SystemEventKind::FeedRecovered,
                venue: Some(Venue::Okx),
                account_id: None,
                symbol: Some("BTC-USDT".to_string()),
                reason: "initial snapshot".to_string(),
            }),
        ),
        seed(
            8,
            4_100_000,
            4,
            SeedRoute::Normalized,
            NormalizedEvent::System(SystemEvent {
                ts_ms: 4,
                kind: SystemEventKind::FeedRecovered,
                venue: Some(Venue::Okx),
                account_id: None,
                symbol: Some("BTC-PERP".to_string()),
                reason: "initial snapshot".to_string(),
            }),
        ),
        seed(
            9,
            4_200_000,
            4,
            SeedRoute::Normalized,
            NormalizedEvent::System(SystemEvent {
                ts_ms: 4,
                kind: SystemEventKind::PrivateStreamRecovered,
                venue: Some(Venue::Okx),
                account_id: Some("main".to_string()),
                symbol: None,
                reason: "initial private snapshot".to_string(),
            }),
        ),
        seed(
            10,
            4_300_000,
            4,
            SeedRoute::Normalized,
            NormalizedEvent::System(SystemEvent {
                ts_ms: 4,
                kind: SystemEventKind::OrderTransportRecovered,
                venue: Some(Venue::Okx),
                account_id: Some("main".to_string()),
                symbol: None,
                reason: "order session authenticated".to_string(),
            }),
        ),
    ];
    InitializationArtifactV1 {
        schema_version: 1,
        reap_commit: "1d35c02cd74892ec0cadb7d60c2839827fff3e3b".to_string(),
        java_revision: reap_core::PINNED_JAVA_REVISION.to_string(),
        strategy,
        risk_limits,
        instruments: vec![
            InstrumentInitialization {
                account_id: "main".to_string(),
                symbol: "BTC-USDT".to_string(),
                instrument_type: "spot".to_string(),
                trade_mode: "cash".to_string(),
                risk_model: InstrumentRiskModel::Spot,
                order_limits: InstrumentOrderLimits {
                    max_limit_quantity: 100.0,
                    max_limit_notional_usd: Some(1_000_000.0),
                },
                tick_size: 0.1,
                lot_size: 0.0001,
                min_size: 0.0001,
                contract_value: None,
            },
            InstrumentInitialization {
                account_id: "main".to_string(),
                symbol: "BTC-PERP".to_string(),
                instrument_type: "futures".to_string(),
                trade_mode: "cross".to_string(),
                risk_model: InstrumentRiskModel::LinearDerivative {
                    contract_value: 0.001,
                },
                order_limits: InstrumentOrderLimits {
                    max_limit_quantity: 1_000_000.0,
                    max_limit_notional_usd: None,
                },
                tick_size: 0.1,
                lot_size: 1.0,
                min_size: 1.0,
                contract_value: Some(0.001),
            },
        ],
        accounts: vec![AccountInitialization {
            id: "main".to_string(),
            id_prefix: "reap".to_string(),
            node_id: 1,
            expected_account_level: "single_currency_margin".to_string(),
            expected_position_mode: "net_mode".to_string(),
            trade_modes: BTreeMap::from([
                ("BTC-PERP".to_string(), "cross".to_string()),
                ("BTC-USDT".to_string(), "cash".to_string()),
            ]),
            bootstrap_update: account_update,
            baseline_fill_ids: Vec::new(),
            quote_stp_verified: true,
        }],
        declared_state: DeclaredDecisionState {
            kill_switch_reason: None,
            halted_symbols: Vec::new(),
            feed_health: vec![
                FeedHealthState {
                    venue: Venue::Okx,
                    symbol: "BTC-PERP".to_string(),
                    last_ready_ms: 4,
                    stale: false,
                },
                FeedHealthState {
                    venue: Venue::Okx,
                    symbol: "BTC-USDT".to_string(),
                    last_ready_ms: 4,
                    stale: false,
                },
            ],
            private_health: vec![PrivateHealthState {
                venue: Venue::Okx,
                account_id: Some("main".to_string()),
                last_ready_ms: 4,
                stale: false,
            }],
            marks: vec![
                NumericSymbolState {
                    symbol: "BTC-PERP".to_string(),
                    value: 50_003.5,
                },
                NumericSymbolState {
                    symbol: "BTC-USDT".to_string(),
                    value: 50_000.5,
                },
            ],
            positions: vec![
                NumericSymbolState {
                    symbol: "BTC-PERP".to_string(),
                    value: 0.0,
                },
                NumericSymbolState {
                    symbol: "BTC-USDT".to_string(),
                    value: 0.0,
                },
            ],
            live_orders: Vec::new(),
            order_rejections: Vec::new(),
            rejected_order_ids: Vec::new(),
            last_order_rejection_ms: 0,
            unfilled_ioc_cancellations: Vec::new(),
            unfilled_ioc_cancelled_order_ids: Vec::new(),
            last_unfilled_ioc_cancel_ms: 0,
            turnover_usd: 0.0,
            equity_usd: 10_000.0,
            equity_by_account: vec![AccountEquityState {
                account_id: Some("main".to_string()),
                equity_usd: 10_000.0,
            }],
            peak_equity_usd: 10_000.0,
            seen_fills: Vec::new(),
            stablecoin_rates: vec![StablecoinRateState {
                symbol: "USDT-USD".to_string(),
                ts_ms: 2,
                price: 1.0,
                conflict: false,
            }],
            stablecoin_missing_symbols: Vec::new(),
            stablecoin_breach_since: Vec::new(),
            strategy_references: vec![
                StrategyReferenceState {
                    kind: "mark_price".to_string(),
                    symbol: "BTC-PERP".to_string(),
                    source_ts_ms: 2,
                    observed_now_ms: 2,
                },
                StrategyReferenceState {
                    kind: "price_limits".to_string(),
                    symbol: "BTC-PERP".to_string(),
                    source_ts_ms: 2,
                    observed_now_ms: 2,
                },
                StrategyReferenceState {
                    kind: "price_limits".to_string(),
                    symbol: "BTC-USDT".to_string(),
                    source_ts_ms: 2,
                    observed_now_ms: 2,
                },
            ],
            source_clock: SourceClockState {
                seed_now_ms: 4,
                seed_arrival_ns: 4_300_000,
            },
        },
        live: LiveInitialization {
            session_id: "goal-d-phase3".to_string(),
            storage_ready: true,
            public_connectivity_ready: true,
            reconciled_accounts: vec!["main".to_string()],
            forbidden_zero_accounts: vec!["main".to_string()],
            order_transport_ready_accounts: vec!["main".to_string()],
            gateway_action_accounts: vec!["main".to_string()],
            order_entry_enabled: true,
            halted_accounts: Vec::new(),
            decision_sequence: 0,
        },
        seed_events,
    }
}

fn seed(
    sequence: u64,
    arrival_ns: u64,
    observed_now_ms: u64,
    route: SeedRoute,
    event: NormalizedEvent,
) -> SeedEvent {
    SeedEvent {
        sequence,
        arrival_ns,
        observed_now_ms,
        route,
        event,
    }
}

#[test]
fn initialization_fixture_is_complete_strict_and_self_contained() {
    let parsed = parse_initialization(INITIALIZATION_BYTES).expect("strict initialization fixture");
    assert_eq!(
        serde_json::to_value(parsed).unwrap(),
        serde_json::to_value(expected_initialization()).unwrap()
    );

    let raw: serde_json::Value = serde_json::from_slice(INITIALIZATION_BYTES).unwrap();
    for key in risk_limit_keys() {
        let mut missing = raw.clone();
        missing["risk_limits"].as_object_mut().unwrap().remove(*key);
        let error = parse_initialization(&serde_json::to_vec(&missing).unwrap())
            .expect_err("every effective risk limit is required");
        assert!(
            error.contains("missing=[") && error.contains(key),
            "missing {key}: {error}"
        );
    }

    let mut defaultable_strategy_field_missing = raw.clone();
    defaultable_strategy_field_missing["strategy"]
        .as_object_mut()
        .unwrap()
        .remove("reference_data_stale_threshold_ms");
    assert!(
        parse_initialization(&serde_json::to_vec(&defaultable_strategy_field_missing).unwrap())
            .unwrap_err()
            .contains("reference_data_stale_threshold_ms")
    );

    let mut wrong_java = raw.clone();
    wrong_java["java_revision"] = serde_json::Value::String("0".repeat(40));
    assert!(
        parse_initialization(&serde_json::to_vec(&wrong_java).unwrap())
            .unwrap_err()
            .contains("Java revision")
    );

    let mut uppercase_commit = raw;
    uppercase_commit["reap_commit"] =
        serde_json::Value::String("ABCDEFABCDEFABCDEFABCDEFABCDEFABCDEFABCD".to_string());
    assert!(
        parse_initialization(&serde_json::to_vec(&uppercase_commit).unwrap())
            .unwrap_err()
            .contains("lowercase SHA")
    );
}

#[test]
fn initialization_rejects_nested_omissions_and_state_contradictions() {
    let raw: serde_json::Value = serde_json::from_slice(INITIALIZATION_BYTES).unwrap();
    let reject = |label: &str, value: &serde_json::Value| {
        assert!(
            parse_initialization(&serde_json::to_vec(value).unwrap()).is_err(),
            "{label} must fail closed"
        );
    };

    let mut missing_margin_array = raw.clone();
    missing_margin_array["accounts"][0]["bootstrap_update"]
        .as_object_mut()
        .unwrap()
        .remove("margins");
    reject("missing nested margin array", &missing_margin_array);

    let mut missing_balance_equity = raw.clone();
    missing_balance_equity["accounts"][0]["bootstrap_update"]["balances"][0]
        .as_object_mut()
        .unwrap()
        .remove("equity");
    reject("missing nested balance equity", &missing_balance_equity);

    let mut unknown_nested = raw.clone();
    unknown_nested["seed_events"][0]["event"]["Account"]["balances"][0]["permissive"] =
        serde_json::Value::Bool(true);
    reject("unknown nested field", &unknown_nested);

    let mut contradictory_mark = raw.clone();
    contradictory_mark["declared_state"]["marks"][0]["value"] = serde_json::Value::from(1.0);
    reject("declared mark contradiction", &contradictory_mark);

    let mut contradictory_equity = raw.clone();
    contradictory_equity["declared_state"]["equity_usd"] = serde_json::Value::from(9_999.0);
    reject("declared equity contradiction", &contradictory_equity);

    let mut contradictory_feed = raw.clone();
    contradictory_feed["declared_state"]["feed_health"][0]["last_ready_ms"] =
        serde_json::Value::from(3);
    reject("declared feed contradiction", &contradictory_feed);

    let mut contradictory_stablecoin = raw.clone();
    contradictory_stablecoin["declared_state"]["stablecoin_rates"][0]["price"] =
        serde_json::Value::from(0.99);
    reject(
        "declared stablecoin contradiction",
        &contradictory_stablecoin,
    );

    let mut contradictory_reference = raw.clone();
    contradictory_reference["declared_state"]["strategy_references"][0]["observed_now_ms"] =
        serde_json::Value::from(3);
    reject(
        "declared strategy-reference contradiction",
        &contradictory_reference,
    );

    let mut contradictory_clock = raw.clone();
    contradictory_clock["declared_state"]["source_clock"]["seed_arrival_ns"] =
        serde_json::Value::from(4_299_999);
    reject("declared source-clock contradiction", &contradictory_clock);

    let mut nonclean_history = raw.clone();
    nonclean_history["declared_state"]["turnover_usd"] = serde_json::Value::from(1.0);
    reject("non-clean transition history", &nonclean_history);

    let mut contradictory_metadata = raw.clone();
    contradictory_metadata["instruments"][0]["tick_size"] = serde_json::Value::from(0.01);
    reject(
        "strategy/live metadata contradiction",
        &contradictory_metadata,
    );

    let mut contradictory_bootstrap = raw.clone();
    contradictory_bootstrap["accounts"][0]["bootstrap_update"]["balances"][0]["available"] =
        serde_json::Value::from(9_999.0);
    reject("bootstrap/seed contradiction", &contradictory_bootstrap);

    let mut non_effective_strategy = raw.clone();
    non_effective_strategy["strategy"]["risk_multiplier"] = serde_json::Value::from(2.0);
    reject(
        "strategy requiring a second effective-config transform",
        &non_effective_strategy,
    );

    let mut unscoped_private_row = raw.clone();
    unscoped_private_row["seed_events"][0]["event"]["Account"]["balances"][0]["account_id"] =
        serde_json::Value::Null;
    reject("unscoped private-account row", &unscoped_private_row);

    let mut duplicate_account_rows = raw.clone();
    for pointer in [
        "/accounts/0/bootstrap_update/balances",
        "/seed_events/0/event/Account/balances",
    ] {
        let rows = duplicate_account_rows
            .pointer_mut(pointer)
            .unwrap()
            .as_array_mut()
            .unwrap();
        let duplicate = rows[0].clone();
        rows.push(duplicate);
    }
    reject("duplicate account snapshot rows", &duplicate_account_rows);

    let mut contradictory_transport = raw.clone();
    contradictory_transport["seed_events"][9]["event"]["System"]["account_id"] =
        serde_json::Value::String("other".to_string());
    reject(
        "order-transport readiness/seed contradiction",
        &contradictory_transport,
    );

    let mut invalid_depth = raw.clone();
    let depth_seed = invalid_depth["seed_events"]
        .as_array_mut()
        .unwrap()
        .iter_mut()
        .find(|seed| seed["event"]["Market"]["Depth"].is_object())
        .unwrap();
    depth_seed["event"]["Market"]["Depth"]["bids"][0]["qty"] = serde_json::Value::from(-1.0);
    reject("invalid seeded book numeric state", &invalid_depth);

    let mut invalid_prefix = raw.clone();
    invalid_prefix["accounts"][0]["id_prefix"] = serde_json::Value::String("too-long!".to_string());
    reject("invalid client-order prefix", &invalid_prefix);

    let mut missing_quote_stp = raw.clone();
    missing_quote_stp["accounts"][0]["quote_stp_verified"] = serde_json::Value::Bool(false);
    reject("missing quote STP verification", &missing_quote_stp);

    let mut duplicate_feed = raw;
    let duplicate = duplicate_feed["declared_state"]["feed_health"][0].clone();
    duplicate_feed["declared_state"]["feed_health"]
        .as_array_mut()
        .unwrap()
        .insert(1, duplicate);
    reject("duplicate declared feed row", &duplicate_feed);
}

struct CaseAuthor {
    case: &'static str,
    next_sequence: u64,
    next_client_id: u64,
    engine: TradingEngine<ChaosStrategy>,
    rows: Vec<ReplayEnvelope>,
}

impl CaseAuthor {
    fn new(case: &'static str, initialization: &InitializationArtifactV1) -> Self {
        Self {
            case,
            next_sequence: 1,
            next_client_id: 1,
            engine: build_engine(initialization).unwrap(),
            rows: Vec::new(),
        }
    }

    fn push_input(&mut self, input: ReplayInput) {
        self.rows.push(ReplayEnvelope {
            schema_version: 1,
            case: self.case.to_string(),
            sequence: self.next_sequence,
            input,
        });
        self.next_sequence += 1;
    }

    fn normalized(
        &mut self,
        receipt_ns: u64,
        observed_now_ms: u64,
        event: NormalizedEvent,
    ) -> ChaosEngineOutput {
        self.push_input(ReplayInput::Normalized {
            receipt_ns: Some(receipt_ns),
            observed_now_ms,
            event: event.clone(),
        });
        self.engine
            .on_chaos_event_at(event, receipt_ns, observed_now_ms, true)
    }

    fn register_submits(
        &mut self,
        output: ChaosEngineOutput,
        reservation_ts_ms: u64,
        local_send_ms: u64,
    ) -> Vec<OrderUpdate> {
        let pending = output
            .intents
            .iter()
            .filter_map(|intent| match intent.to_order_intent() {
                OrderIntent::NewOrder(order) => {
                    let update = OrderUpdate {
                        ts_ms: reservation_ts_ms,
                        order_id: format!("client#{}", self.next_client_id),
                        symbol: order.symbol,
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
                        reason: if order.reason.is_empty() {
                            "pending_new".to_string()
                        } else {
                            format!("{}:pending_new", order.reason)
                        },
                    };
                    self.next_client_id += 1;
                    Some(update)
                }
                OrderIntent::CancelOrder { .. } => None,
            })
            .collect::<Vec<_>>();
        for intent in output.intents {
            if matches!(intent.to_order_intent(), OrderIntent::NewOrder(_)) {
                self.engine
                    .with_locally_sent_chaos_intent(
                        intent,
                        || local_send_ms,
                        |_| Ok::<(), String>(()),
                    )
                    .unwrap();
            }
        }
        for update in &pending {
            self.push_input(ReplayInput::PendingFeedback {
                event: update.clone(),
            });
            let _ = self
                .engine
                .on_chaos_event(NormalizedEvent::Order(update.clone()));
        }
        pending
    }

    fn due_trade_reprice(&mut self, now_ns: u64, observed_now_ms: u64) -> ChaosEngineOutput {
        self.push_input(ReplayInput::DueTradeReprice {
            now_ns,
            observed_now_ms,
        });
        self.engine
            .service_one_due_chaos_trade_reprice(now_ns, observed_now_ms, true)
    }
}

fn authored_replay(initialization: &InitializationArtifactV1) -> Vec<ReplayEnvelope> {
    let timer = |ts_ms, name: &str| {
        NormalizedEvent::Timer(TimerEvent {
            ts_ms,
            name: name.to_string(),
        })
    };
    let system = |ts_ms, kind, symbol: Option<&str>, reason: &str| {
        NormalizedEvent::System(SystemEvent {
            ts_ms,
            kind,
            venue: Some(Venue::Okx),
            account_id: None,
            symbol: symbol.map(str::to_string),
            reason: reason.to_string(),
        })
    };
    let mut all = Vec::new();

    let mut quote = CaseAuthor::new("01_quote", initialization);
    let quote_output = quote.normalized(12_000_000, 12, timer(10, "quote"));
    quote.register_submits(quote_output, 10, 12);
    all.extend(quote.rows);

    let mut hedge = CaseAuthor::new("02_hedge", initialization);
    let hedge_output = hedge.normalized(
        10_000_000,
        10,
        NormalizedEvent::Account(AccountUpdate {
            ts_ms: 10,
            balances: Vec::new(),
            positions: vec![Position {
                symbol: "BTC-USDT".to_string(),
                qty: 0.1,
                avg_price: 50_000.0,
                margin_mode: None,
            }],
            margins: Vec::new(),
        }),
    );
    hedge.register_submits(hedge_output, 10, 10);
    all.extend(hedge.rows);

    let mut trade = CaseAuthor::new("03_trade_reprice", initialization);
    let depth_output = trade.normalized(
        10_000_000,
        10,
        NormalizedEvent::Market(MarketEvent::Depth(OrderBook {
            symbol: "BTC-USDT".to_string(),
            ts_ms: 10,
            bids: [50_000.0, 49_999.0, 49_998.0]
                .map(|px| Level::new(px, 10.0))
                .to_vec(),
            asks: [50_001.0, 50_002.0, 50_003.0]
                .map(|px| Level::new(px, 10.0))
                .to_vec(),
        })),
    );
    trade.register_submits(depth_output, 10, 10);
    let trade_output = trade.normalized(
        15_000_000,
        15,
        NormalizedEvent::Market(MarketEvent::Trade {
            ts_ms: 15,
            symbol: "BTC-USDT".to_string(),
            price: 50_002.0,
            qty: 5.0,
            taker_side: Side::Buy,
        }),
    );
    trade.register_submits(trade_output, 15, 15);
    let repriced = trade.due_trade_reprice(15_100_000, 15);
    trade.register_submits(repriced, 15, 15);
    all.extend(trade.rows);

    let mut rejected = CaseAuthor::new("04_risk_rejection", initialization);
    let stale = rejected.normalized(
        10_000_000,
        10,
        NormalizedEvent::System(SystemEvent {
            ts_ms: 10,
            kind: SystemEventKind::PrivateStreamStale,
            venue: Some(Venue::Okx),
            account_id: Some("main".to_string()),
            symbol: None,
            reason: "fixture private stream stale".to_string(),
        }),
    );
    rejected.register_submits(stale, 10, 10);
    let rejected_output = rejected.normalized(11_000_000, 11, timer(11, "risk rejection"));
    rejected.register_submits(rejected_output, 11, 11);
    all.extend(rejected.rows);

    let mut symbol_halt = CaseAuthor::new("05_symbol_halt", initialization);
    let quotes = symbol_halt.normalized(10_000_000, 10, timer(10, "symbol halt seed"));
    symbol_halt.register_submits(quotes, 10, 10);
    let _ = symbol_halt.normalized(
        11_000_000,
        11,
        system(
            11,
            SystemEventKind::SymbolHalted,
            Some("BTC-USDT"),
            "fixture symbol halt",
        ),
    );
    all.extend(symbol_halt.rows);

    let mut global_kill = CaseAuthor::new("06_global_kill", initialization);
    let quotes = global_kill.normalized(10_000_000, 10, timer(10, "global kill seed"));
    global_kill.register_submits(quotes, 10, 10);
    let _ = global_kill.normalized(
        11_000_000,
        11,
        system(
            11,
            SystemEventKind::KillSwitchActivated,
            None,
            "fixture global kill",
        ),
    );
    all.extend(global_kill.rows);

    let mut fill = CaseAuthor::new("07_fill_order", initialization);
    let quotes = fill.normalized(10_000_000, 10, timer(10, "fill seed"));
    let pending = fill.register_submits(quotes, 10, 10);
    let seeded = pending.first().expect("quote case must create an order");
    let filled = fill.normalized(
        11_000_000,
        11,
        NormalizedEvent::Order(OrderUpdate {
            ts_ms: 11,
            order_id: seeded.order_id.clone(),
            symbol: seeded.symbol.clone(),
            side: seeded.side,
            event: OrderEvent::FullyFilled,
            status: OrderStatus::Filled,
            price: seeded.price,
            time_in_force: Some(TimeInForce::PostOnly),
            qty: seeded.qty,
            open_qty: 0.0,
            filled_qty: seeded.qty,
            avg_fill_price: seeded.price,
            last_fill_qty: seeded.qty,
            last_fill_price: seeded.price,
            last_fill_liquidity: Some(FillLiquidity::Maker),
            last_fill_fee: None,
            reason: "fixture fill".to_string(),
        }),
    );
    fill.register_submits(filled, 11, 11);
    let post_fill_probe = fill.normalized(
        12_000_000,
        12,
        system(
            12,
            SystemEventKind::SymbolHalted,
            Some("BTC-PERP"),
            "post-fill live-order probe",
        ),
    );
    fill.register_submits(post_fill_probe, 12, 12);
    all.extend(fill.rows);

    let mut system_event = CaseAuthor::new("08_system_event", initialization);
    let mut forced_repayment = initialization.accounts[0].bootstrap_update.clone();
    forced_repayment.ts_ms = 10;
    forced_repayment.balances[0].forced_repayment_indicator = Some(1);
    let _ = system_event.normalized(10_000_000, 10, NormalizedEvent::Account(forced_repayment));
    all.extend(system_event.rows);

    all
}

#[test]
#[ignore = "one-shot fixture authoring helper"]
fn print_initialization_fixture() {
    println!(
        "{}",
        serde_json::to_string(&expected_initialization()).unwrap()
    );
}

#[test]
#[ignore = "one-shot fixture authoring helper"]
fn print_replay_fixture() {
    let initialization = parse_initialization(INITIALIZATION_BYTES).unwrap();
    print!(
        "{}",
        canonical_jsonl(&authored_replay(&initialization)).unwrap()
    );
}

#[test]
fn replay_fixture_is_strict_deterministic_and_covers_the_decision_boundary() {
    let initialization = parse_initialization(INITIALIZATION_BYTES).unwrap();
    let replay = parse_replay_jsonl(REPLAY_BYTES).unwrap();
    assert_eq!(
        canonical_jsonl(&replay).unwrap(),
        canonical_jsonl(&authored_replay(&initialization)).unwrap(),
        "checked-in input must equal the production-authored fixture"
    );

    let first = replay_engine(&initialization, &replay).unwrap();
    let second = replay_engine(&initialization, &replay).unwrap();
    let first = canonical_jsonl(&first).unwrap();
    assert_eq!(first, canonical_jsonl(&second).unwrap());
    assert_eq!(first.as_bytes(), EXPECTED_ENGINE_BYTES);

    let rows = first
        .lines()
        .map(|line| serde_json::from_str::<serde_json::Value>(line).unwrap())
        .collect::<Vec<_>>();
    let skewed_quote = rows
        .iter()
        .find(|row| row["case"] == "01_quote" && row["sequence"] == 1)
        .unwrap();
    assert_eq!(skewed_quote["input"]["observed_now_ms"], 12);
    assert_eq!(skewed_quote["input"]["event"]["Timer"]["ts_ms"], 10);
    let skewed_pending = rows
        .iter()
        .find(|row| row["case"] == "01_quote" && row["sequence"] == 2)
        .unwrap();
    assert_eq!(
        skewed_pending["input"]["event"]["ts_ms"], 10,
        "PendingNew reservation time follows source event time, not local-send time"
    );
    assert!(rows.iter().any(|row| {
        row["typed_intents"]
            .as_array()
            .unwrap()
            .iter()
            .any(|intent| intent["purpose"] == "quote")
    }));
    assert!(rows.iter().any(|row| {
        row["typed_intents"]
            .as_array()
            .unwrap()
            .iter()
            .any(|intent| intent["purpose"] == "hedge")
    }));
    assert!(rows.iter().any(|row| {
        row["case"] == "03_trade_reprice"
            && row["input"]["kind"] == "due_trade_reprice"
            && !row["typed_intents"].as_array().unwrap().is_empty()
    }));
    assert!(rows.iter().any(|row| {
        row["case"] == "04_risk_rejection" && !row["rejections"].as_array().unwrap().is_empty()
    }));
    assert!(rows.iter().any(|row| {
        row["case"] == "05_symbol_halt"
            && row["typed_intents"]
                .as_array()
                .unwrap()
                .iter()
                .any(|intent| intent["purpose"] == "cancel_owned")
    }));
    assert!(rows.iter().any(|row| {
        row["case"] == "06_global_kill"
            && !row["safety_cancel_candidates"]
                .as_array()
                .unwrap()
                .is_empty()
    }));
    let post_fill_probe = rows
        .iter()
        .find(|row| {
            row["case"] == "07_fill_order"
                && row["input"]["event"]["System"]["reason"] == "post-fill live-order probe"
        })
        .unwrap();
    let cancelled = post_fill_probe["typed_intents"]
        .as_array()
        .unwrap()
        .iter()
        .map(|intent| {
            intent["legacy"]["CancelOrder"]["order_id"]
                .as_str()
                .unwrap()
        })
        .collect::<Vec<_>>();
    assert_eq!(
        cancelled,
        ["client#2", "client#3", "client#4"],
        "the post-fill probe must prove client#1 was removed from live-order state"
    );
    assert!(rows.iter().any(|row| {
        row["case"] == "08_system_event" && !row["system_events"].as_array().unwrap().is_empty()
    }));
}

#[test]
fn replay_fixture_rejects_shape_and_case_order_ambiguity() {
    let rows = std::str::from_utf8(REPLAY_BYTES)
        .unwrap()
        .lines()
        .map(|line| serde_json::from_str::<serde_json::Value>(line).unwrap())
        .collect::<Vec<_>>();

    let mut unknown = rows[0].clone();
    unknown["permissive"] = serde_json::Value::Bool(true);
    assert!(parse_replay_jsonl(serde_json::to_string(&unknown).unwrap().as_bytes()).is_err());

    let mut missing = rows[0].clone();
    missing["input"]
        .as_object_mut()
        .unwrap()
        .remove("observed_now_ms");
    assert!(parse_replay_jsonl(serde_json::to_string(&missing).unwrap().as_bytes()).is_err());

    let mut reordered = rows;
    let second_case = reordered
        .iter()
        .position(|row| row["case"] == "02_hedge")
        .unwrap();
    reordered.swap(0, second_case);
    let reordered = reordered
        .iter()
        .map(serde_json::to_string)
        .collect::<Result<Vec<_>, _>>()
        .unwrap()
        .join("\n");
    assert!(parse_replay_jsonl(reordered.as_bytes()).is_err());
}

#[test]
#[ignore = "one-shot fixture authoring helper"]
fn print_engine_projection_fixture() {
    let initialization = parse_initialization(INITIALIZATION_BYTES).unwrap();
    let replay = parse_replay_jsonl(REPLAY_BYTES).unwrap();
    print!(
        "{}",
        canonical_jsonl(&replay_engine(&initialization, &replay).unwrap()).unwrap()
    );
}
