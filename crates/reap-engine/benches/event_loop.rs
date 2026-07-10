use std::hint::black_box;
use std::time::Instant;

use reap_core::{Level, MarketEvent, NormalizedEvent, OrderBook};
use reap_engine::TradingEngine;
use reap_risk::{RiskGate, RiskLimits};
use reap_strategy::{ChaosConfig, ChaosStrategy};

fn main() {
    const EVENTS: u64 = 250_000;
    let config: ChaosConfig = toml::from_str(include_str!("../../../examples/iarb2-basic.toml"))
        .expect("benchmark config");
    let limits = RiskLimits {
        require_feed_health: false,
        require_private_health: false,
        max_order_notional_usd: f64::MAX,
        max_abs_position_notional_usd: f64::MAX,
        max_live_order_notional_usd: f64::MAX,
        max_turnover_usd: f64::MAX,
        ..RiskLimits::default()
    };
    let mut engine = TradingEngine::new(ChaosStrategy::new(config), RiskGate::new(limits));

    let started = Instant::now();
    let mut intents = 0_usize;
    for index in 0..EVENTS {
        let symbol = if index % 2 == 0 {
            "BTC-USDT"
        } else {
            "BTC-PERP"
        };
        let mid = 45_000.0 + (index % 11) as f64 * 0.1;
        let output = engine.on_event(NormalizedEvent::from(MarketEvent::Depth(
            OrderBook::one_level(
                symbol,
                index,
                Level::new(mid - 0.1, 10.0),
                Level::new(mid + 0.1, 10.0),
            ),
        )));
        intents += black_box(output.intents.len());
    }
    let elapsed = started.elapsed();
    let nanos_per_event = elapsed.as_nanos() as f64 / EVENTS as f64;
    println!(
        "event_loop: events={EVENTS} elapsed_ms={:.3} ns_per_event={nanos_per_event:.1} intents={intents}",
        elapsed.as_secs_f64() * 1_000.0
    );
    assert!(intents > 0);
}
