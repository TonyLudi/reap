use reap_core::{NormalizedEvent, OrderIntent, PINNED_JAVA_REVISION};
use reap_strategy::{ChaosConfig, InstrumentConfig, InstrumentKindConfig, RiskGroupConfig};
use serde_json::{Value, json};

use super::*;

fn fixture_strategy_config() -> ChaosConfig {
    let mut config = ChaosConfig {
        ref_symbol: "BTC-USDT".to_string(),
        delta_limit_usd: 50_000.0,
        active_hedge_threshold_usd: 1_000.0,
        min_hedge_interval_ms: 0,
        risk_groups: vec![RiskGroupConfig {
            name: "main".to_string(),
            account_id: Some("main".to_string()),
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
    };
    for instrument in &mut config.instruments {
        instrument.debounce_width = 0.0;
        instrument.debounce_size_usd = 0.0;
        instrument.debounce_ms = 0;
    }
    config
}

fn fixture_coordinator() -> LiveCoordinator {
    let mut coordinator = coordinator_with_strategy_and_risk(
        true,
        RiskLimits {
            require_feed_health: false,
            require_private_health: false,
            ..RiskLimits::default()
        },
        fixture_strategy_config(),
    );
    ready(&mut coordinator);
    for (symbol, mark_price) in [("BTC-USDT", 0.0), ("BTC-PERP", 50_003.5)] {
        coordinator
            .process_feed_at(
                FeedOutput::Event(NormalizedEvent::Market(MarketEvent::PriceLimits {
                    ts_ms: 2,
                    symbol: symbol.to_string(),
                    mark_price,
                    limit_down: 1.0,
                    limit_up: 1_000_000_000.0,
                })),
                2,
            )
            .unwrap();
    }
    assert!(
        std::mem::take(&mut coordinator.chaos_intent_trace).is_empty(),
        "readiness setup must not pre-consume fixture pricing state"
    );
    coordinator
}

fn normalized_fixture() -> (Vec<NormalizedEvent>, Value) {
    let events = include_str!("../../../../fixtures/normalized/chaos_trade_implied_depth.jsonl")
        .lines()
        .filter(|line| !line.starts_with('#') && !line.trim().is_empty())
        .map(|line| serde_json::from_str::<NormalizedEvent>(line).unwrap())
        .collect::<Vec<_>>();
    let golden = serde_json::from_str(include_str!(
        "../../../../fixtures/normalized/chaos_trade_implied_depth_intents_v2.json"
    ))
    .unwrap();
    (events, golden)
}

fn take_projection(coordinator: &mut LiveCoordinator) -> (Value, Value) {
    let trace = std::mem::take(&mut coordinator.chaos_intent_trace);
    let typed = json!(
        trace
            .iter()
            .map(|(purpose, legacy)| {
                json!({
                    "purpose": purpose.as_str(),
                    "legacy": legacy,
                })
            })
            .collect::<Vec<_>>()
    );
    let legacy = json!(
        trace
            .into_iter()
            .map(|(_, legacy)| legacy)
            .collect::<Vec<OrderIntent>>()
    );
    (typed, legacy)
}

fn assert_projection(coordinator: &mut LiveCoordinator, expected: &Value, context: &str) {
    let (typed, legacy) = take_projection(coordinator);
    assert_eq!(typed, expected["typed"], "typed {context}");
    assert_eq!(legacy, expected["legacy"], "legacy {context}");
}

fn drain_private_wakes_before(coordinator: &mut LiveCoordinator, arrival_ns: u64) -> Vec<u64> {
    let mut serviced = Vec::new();
    while let Some(due_ns) = coordinator
        .next_trade_reprice_due_ns()
        .filter(|due_ns| *due_ns < arrival_ns)
    {
        coordinator.service_one_due_trade_reprice(due_ns, due_ns / 1_000_000);
        let (typed, legacy) = take_projection(coordinator);
        assert_eq!(typed, json!([]), "typed internal wake at {due_ns}ns");
        assert_eq!(legacy, json!([]), "legacy internal wake at {due_ns}ns");
        serviced.push(due_ns);
    }
    serviced
}

#[test]
fn checked_in_trade_fixture_drives_live_reduction_through_inclusive_horizon() {
    let (events, golden) = normalized_fixture();
    assert_eq!(golden["schema_version"], 2);
    assert_eq!(golden["java_revision"], PINNED_JAVA_REVISION);
    assert_eq!(
        golden["scenario_fixture"],
        "chaos_trade_implied_depth.jsonl"
    );

    let lifecycle = &golden["lifecycle"];
    assert_eq!(lifecycle["strategy_is_live"], true);
    let arrival_ns = lifecycle["event_arrival_ns"].as_array().unwrap();
    let observed_now_ms = lifecycle["event_observed_now_ms"].as_array().unwrap();
    let expected_events = golden["event_outputs"].as_array().unwrap();
    assert_eq!(events.len(), arrival_ns.len());
    assert_eq!(events.len(), observed_now_ms.len());
    assert_eq!(events.len(), expected_events.len());

    let mut coordinator = fixture_coordinator();
    let mut internal_service_deadlines = Vec::new();
    for (index, event) in events.into_iter().enumerate() {
        internal_service_deadlines.extend(drain_private_wakes_before(
            &mut coordinator,
            arrival_ns[index].as_u64().unwrap(),
        ));
        coordinator
            .process_feed_arrived_at(
                FeedOutput::Event(event),
                observed_now_ms[index].as_u64().unwrap(),
                arrival_ns[index].as_u64().unwrap(),
            )
            .unwrap();
        assert_projection(
            &mut coordinator,
            &expected_events[index],
            &format!("event output {index}"),
        );
    }
    assert_eq!(
        internal_service_deadlines,
        Vec::<u64>::new(),
        "the fixture's depth arrivals reach immediate worker boundaries"
    );

    let service_horizon_ns = lifecycle["service_horizon_ns"].as_u64().unwrap();
    let service_points = lifecycle["service_points"].as_array().unwrap();
    for (index, point) in service_points.iter().enumerate() {
        let now_ns = point["now_ns"].as_u64().unwrap();
        assert!(now_ns <= service_horizon_ns);
        coordinator
            .service_one_due_trade_reprice(now_ns, point["observed_now_ms"].as_u64().unwrap());
        assert_projection(
            &mut coordinator,
            &point["output"],
            &format!("service output {index}"),
        );
        assert_eq!(
            coordinator.next_trade_reprice_due_ns(),
            point["expected_next_due_ns"].as_u64(),
            "next deadline after service point {index}"
        );
    }

    assert_eq!(
        service_points.last().unwrap()["now_ns"].as_u64(),
        Some(service_horizon_ns),
        "the production live reduction must be serviced through the fixture's inclusive horizon"
    );
    assert_eq!(
        coordinator.next_trade_reprice_due_ns(),
        golden["final_state"]["next_due_ns"].as_u64()
    );
}

#[test]
fn risk_kill_suppresses_but_consumes_a_retained_trade_callback() {
    let (events, golden) = normalized_fixture();
    let arrivals = golden["lifecycle"]["event_arrival_ns"].as_array().unwrap();
    let observed = golden["lifecycle"]["event_observed_now_ms"]
        .as_array()
        .unwrap();
    let mut coordinator = fixture_coordinator();
    for (index, event) in events.iter().take(2).cloned().enumerate() {
        coordinator
            .process_feed_arrived_at(
                FeedOutput::Event(event),
                observed[index].as_u64().unwrap(),
                arrivals[index].as_u64().unwrap(),
            )
            .unwrap();
        let _ = take_projection(&mut coordinator);
    }
    assert_eq!(
        drain_private_wakes_before(&mut coordinator, 1_000_000_000),
        Vec::<u64>::new()
    );

    let trade = events.last().unwrap().clone();
    coordinator
        .process_feed_arrived_at(FeedOutput::Event(trade.clone()), 1_000, 1_000_000_000)
        .unwrap();
    assert_eq!(coordinator.next_trade_reprice_due_ns(), Some(1_000_100_000));

    coordinator.process_event(NormalizedEvent::System(SystemEvent {
        ts_ms: 1_000,
        kind: SystemEventKind::KillSwitchActivated,
        venue: None,
        account_id: None,
        symbol: None,
        reason: "fixture risk kill".to_string(),
    }));
    assert!(coordinator.kill_switch_active());
    let _ = take_projection(&mut coordinator);

    coordinator.service_one_due_trade_reprice(1_000_100_000, 1_000);
    assert_projection(
        &mut coordinator,
        &json!({"typed": [], "legacy": []}),
        "risk-killed retained callback",
    );
    assert_eq!(
        coordinator.next_trade_reprice_due_ns(),
        None,
        "the retained callback must be consumed even though its output is live-gated"
    );

    coordinator.process_event(NormalizedEvent::System(SystemEvent {
        ts_ms: 1_000,
        kind: SystemEventKind::KillSwitchReset,
        venue: None,
        account_id: None,
        symbol: None,
        reason: "fixture reset".to_string(),
    }));
    assert!(!coordinator.kill_switch_active());
    assert!(coordinator.readiness().is_ready());
    let _ = take_projection(&mut coordinator);

    coordinator
        .process_feed_arrived_at(FeedOutput::Event(trade), 1_001, 1_001_000_000)
        .unwrap();
    assert_eq!(coordinator.next_trade_reprice_due_ns(), Some(1_001_100_000));
    coordinator.service_one_due_trade_reprice(1_001_100_000, 1_001);
    assert_projection(
        &mut coordinator,
        &json!({"typed": [], "legacy": []}),
        "post-reset callback within the worker interval",
    );
    assert_eq!(
        coordinator.next_trade_reprice_due_ns(),
        Some(1_005_100_000),
        "the killed callback must still advance the shared worker clock"
    );
}
