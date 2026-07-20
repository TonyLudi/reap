use super::*;

struct CoordinatorActionScenario {
    coordinator: LiveCoordinator,
    account_positive: NormalizedEvent,
    account_negative: NormalizedEvent,
}

pub(super) fn coordinator_reduction_workload() -> WorkloadResult {
    let result = run_workload(
        "coordinator_normalized_storage_reduction",
        "public LiveCoordinator::process_event reduction, its production engine/risk path, and \
         canonical StorageRecord projection for alternating account-position inputs; exact \
         zero-action counters are retained rather than inferring unavailable authority",
        "socket/feed parsing; production runtime channel scheduling; storage enqueue/disk; \
         regular gateway preparation; adapter serialization; network; acknowledgement",
        || {
            let mut coordinator = benchmark_coordinator();
            seed_coordinator_references(&mut coordinator);
            black_box(
                coordinator.process_event(depth_event("BTC-USDT", 2, 50_000.0, 50_001.0, 2.0)),
            );
            black_box(coordinator.process_event(depth_event(
                "BTC-USDT-SWAP",
                2,
                50_003.0,
                50_004.0,
                100.0,
            )));
            CoordinatorActionScenario {
                coordinator,
                account_positive: account_position_event(3, 0.001),
                account_negative: account_position_event(3, -0.001),
            }
        },
        |scenario, index| {
            let event = if index.is_multiple_of(2) {
                &scenario.account_positive
            } else {
                &scenario.account_negative
            };
            let output = scenario.coordinator.process_event(event.clone());
            let mut counters = counters_from_coordinator_output(&output.records);
            counters.inputs = 1;
            counters.normalized_outputs = 1;
            counters.coordinator_actions = output.action_count() as u64;
            counters.storage_records = output.record_count() as u64;
            black_box(output);
            Observation {
                counters,
                queue_age_ns: None,
            }
        },
    );
    assert!(result.counters.storage_records >= TIMED_OBSERVATIONS as u64);
    assert_eq!(result.counters.coordinator_actions, 0);
    result
}

#[derive(Debug, Clone, Copy)]
enum StormKind {
    Control,
    Feed,
}

#[derive(Debug)]
struct QueuedStormEvent {
    kind: StormKind,
    enqueued_at: Instant,
}

struct StormScenario {
    control_tx: mpsc::Sender<QueuedStormEvent>,
    control_rx: mpsc::Receiver<QueuedStormEvent>,
    feed_tx: mpsc::Sender<QueuedStormEvent>,
    feed_rx: mpsc::Receiver<QueuedStormEvent>,
}

impl StormScenario {
    const CONTROL_CAPACITY: usize = 16;
    const FEED_CAPACITY: usize = 64;

    fn new() -> Self {
        let (control_tx, control_rx) = mpsc::channel(Self::CONTROL_CAPACITY);
        let (feed_tx, feed_rx) = mpsc::channel(Self::FEED_CAPACITY);
        Self {
            control_tx,
            control_rx,
            feed_tx,
            feed_rx,
        }
    }

    fn observe(&mut self) -> Observation {
        let mut counters = LogicalCounters {
            inputs: 1,
            queue_capacity: (Self::CONTROL_CAPACITY + Self::FEED_CAPACITY) as u64,
            ..LogicalCounters::default()
        };
        if self.control_rx.is_empty() && self.feed_rx.is_empty() {
            for _ in 0..24 {
                if self
                    .control_tx
                    .try_send(QueuedStormEvent {
                        kind: StormKind::Control,
                        enqueued_at: Instant::now(),
                    })
                    .is_err()
                {
                    counters.queue_saturations += 1;
                }
            }
            for _ in 0..80 {
                if self
                    .feed_tx
                    .try_send(QueuedStormEvent {
                        kind: StormKind::Feed,
                        enqueued_at: Instant::now(),
                    })
                    .is_err()
                {
                    counters.queue_saturations += 1;
                }
            }
        }

        let control_depth = self.control_rx.len();
        let feed_depth = self.feed_rx.len();
        counters.queue_high_water = (control_depth + feed_depth) as u64;
        let both_ready = control_depth > 0 && feed_depth > 0;
        let event = self
            .control_rx
            .try_recv()
            .or_else(|_| self.feed_rx.try_recv())
            .expect("a storm refill must leave a queued event");
        match event.kind {
            StormKind::Control => {
                counters.control_dequeues = 1;
                counters.biased_control_preemptions = u64::from(both_ready);
            }
            StormKind::Feed => counters.feed_dequeues = 1,
        }
        black_box(event.kind);
        Observation {
            counters,
            queue_age_ns: Some(duration_ns(event.enqueued_at.elapsed())),
        }
    }
}

pub(super) fn bounded_biased_control_feed_storm_workload() -> WorkloadResult {
    let result = run_workload(
        "bounded_biased_control_feed_storm",
        "bench-private Tokio bounded mpsc channels; deterministic control-first try-receive \
         priority; monotonic enqueue/dequeue queue age; depth, high-water, and saturation counts",
        "production LiveRuntime select loop; socket/feed parsing; strategy/engine/coordinator; \
         storage; gateway; adapter serialization; disk/network/acknowledgement",
        StormScenario::new,
        |scenario, _| scenario.observe(),
    );
    assert_eq!(
        result.counters.control_dequeues + result.counters.feed_dequeues,
        TIMED_OBSERVATIONS as u64
    );
    assert!(result.counters.biased_control_preemptions > 0);
    assert_eq!(
        result.counters.queue_capacity,
        (StormScenario::CONTROL_CAPACITY + StormScenario::FEED_CAPACITY) as u64
    );
    assert_eq!(
        result.counters.queue_high_water,
        (StormScenario::CONTROL_CAPACITY + StormScenario::FEED_CAPACITY) as u64
    );
    assert!(result.counters.queue_saturations > 0);
    assert_eq!(
        result
            .queue_age
            .expect("storm must report queue age")
            .samples,
        TIMED_OBSERVATIONS
    );
    result
}

pub(super) fn benchmark_coordinator() -> LiveCoordinator {
    let config = LiveConfig::from_toml(include_str!("../../../../examples/live-okx-demo.toml"))
        .expect("benchmark live config");
    let instruments = config
        .strategy
        .instruments
        .iter()
        .map(|instrument| {
            let account = config
                .account_for_symbol(&instrument.symbol)
                .expect("configured symbol must have an account");
            let risk_model = if instrument.kind.is_spot() {
                InstrumentRiskModel::Spot
            } else if instrument.kind.is_inverse() {
                InstrumentRiskModel::InverseDerivative {
                    contract_value: instrument.contract_value,
                }
            } else {
                InstrumentRiskModel::LinearDerivative {
                    contract_value: instrument.contract_value,
                }
            };
            let instrument_type = match instrument.kind {
                InstrumentKindConfig::Spot => OkxInstrumentType::Spot,
                InstrumentKindConfig::LinearSwap | InstrumentKindConfig::InverseSwap => {
                    OkxInstrumentType::Swap
                }
                InstrumentKindConfig::Future
                | InstrumentKindConfig::LinearFuture
                | InstrumentKindConfig::InverseFuture => OkxInstrumentType::Futures,
            };
            let tick_size = instrument.tick_size.to_string();
            let lot_size = instrument.lot_size.to_string();
            let min_size = instrument.min_trade_size.to_string();
            (
                instrument.symbol.clone(),
                VerifiedInstrument::new(
                    account.id.clone(),
                    instrument.symbol.clone(),
                    instrument_type,
                    account.trade_modes[&instrument.symbol],
                    risk_model,
                    InstrumentOrderLimits {
                        max_limit_quantity: 1_000_000.0,
                        max_limit_notional_usd: instrument.kind.is_spot().then_some(1_000_000.0),
                    },
                    instrument.tick_size,
                    instrument.lot_size,
                    instrument.min_trade_size,
                    instrument
                        .kind
                        .is_derivative()
                        .then_some(instrument.contract_value),
                    reap_venue::okx::OkxRegularOrderRules::from_exchange_decimals(
                        &tick_size, &lot_size, &min_size,
                    )
                    .expect("benchmark exact order rules"),
                ),
            )
        })
        .collect();
    let account_update = AccountUpdate {
        ts_ms: BASE_TS_MS,
        balances: vec![Balance {
            account_id: Some(ACCOUNT_ID.to_string()),
            currency: "USDT".to_string(),
            total: 100_000.0,
            available: 100_000.0,
            equity: 100_000.0,
            liability: 0.0,
            max_loan: 0.0,
            forced_repayment_indicator: None,
        }],
        positions: Vec::new(),
        margins: Vec::new(),
    };
    let verified = VerifiedBootstrap {
        instruments,
        account_updates: HashMap::from([(ACCOUNT_ID.to_string(), account_update.clone())]),
        baseline_fill_ids: HashMap::from([(ACCOUNT_ID.to_string(), HashSet::new())]),
        quote_stp_verified_accounts: HashSet::from([ACCOUNT_ID.to_string()]),
    };
    let mut coordinator =
        LiveCoordinator::new(config, verified, HashMap::new(), "action-benchmark")
            .expect("benchmark coordinator");
    coordinator.mark_storage_ready(true, "benchmark sink ready");
    coordinator.mark_public_connectivity(true, "benchmark public connectivity ready");
    coordinator
        .process_feed(FeedOutput::PrivateAccount {
            account_id: Some(ACCOUNT_ID.to_string()),
            update: account_update,
        })
        .expect("benchmark account state");
    coordinator
        .on_reconciliation(ReconciliationResult {
            account_id: ACCOUNT_ID.to_string(),
            ts_ms: BASE_TS_MS,
            clean: true,
            local_live_orders: 0,
            remote_live_orders: 0,
            remote_recent_fills: 0,
            reason: "benchmark bootstrap is clean".to_string(),
        })
        .expect("benchmark reconciliation");
    coordinator
}

fn seed_coordinator_references(coordinator: &mut LiveCoordinator) {
    for symbol in ["BTC-USDT", "BTC-USDT-SWAP"] {
        black_box(
            coordinator.process_event(NormalizedEvent::System(SystemEvent {
                ts_ms: 1,
                kind: SystemEventKind::FeedRecovered,
                venue: Some(Venue::Okx),
                account_id: None,
                symbol: Some(symbol.to_string()),
                reason: "benchmark snapshot".to_string(),
            })),
        );
    }
    black_box(
        coordinator.process_event(NormalizedEvent::System(SystemEvent {
            ts_ms: 1,
            kind: SystemEventKind::PrivateStreamRecovered,
            venue: Some(Venue::Okx),
            account_id: Some(ACCOUNT_ID.to_string()),
            symbol: None,
            reason: "benchmark private stream".to_string(),
        })),
    );
    for symbol in ["USDT-USD", "USDC-USD"] {
        black_box(
            coordinator.process_event(NormalizedEvent::Market(MarketEvent::IndexPrice {
                ts_ms: 1,
                symbol: symbol.to_string(),
                price: 1.0,
            })),
        );
    }

    let config = LiveConfig::from_toml(include_str!("../../../../examples/live-okx-demo.toml"))
        .expect("benchmark live config");
    for requirement in config.strategy.reference_data_requirements() {
        let event = match requirement.kind {
            ReferenceDataKind::IndexPrice => MarketEvent::IndexPrice {
                ts_ms: 1,
                symbol: requirement.symbol,
                price: 50_000.0,
            },
            ReferenceDataKind::FundingRate => MarketEvent::FundingRate {
                ts_ms: 1,
                symbol: requirement.symbol,
                rate: 0.0001,
                funding_time_ms: 28_800_001,
                settlement: None,
            },
            ReferenceDataKind::MarkPrice => MarketEvent::PriceLimits {
                ts_ms: 1,
                symbol: requirement.symbol,
                mark_price: 50_000.0,
                limit_down: 0.0,
                limit_up: 0.0,
            },
            ReferenceDataKind::PriceLimits => MarketEvent::PriceLimits {
                ts_ms: 1,
                symbol: requirement.symbol,
                mark_price: 0.0,
                limit_down: 40_000.0,
                limit_up: 60_000.0,
            },
        };
        black_box(coordinator.process_event(NormalizedEvent::Market(event)));
    }
}
