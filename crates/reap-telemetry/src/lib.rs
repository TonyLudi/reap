mod alerts;

pub use alerts::{
    AlertDeliveryFailure, AlertError, AlertEvent, AlertRuntime, AlertSeverity, AlertSink,
    AlertStats, WebhookAlertConfig, start_webhook_alerts,
};

use std::collections::{BTreeMap, HashMap};
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::{Arc, RwLock};

use serde::{Deserialize, Serialize};
use tracing_subscriber::EnvFilter;
use tracing_subscriber::util::SubscriberInitExt;

const LATENCY_BUCKETS_US: [u64; 12] = [
    10,
    25,
    50,
    100,
    250,
    500,
    1_000,
    2_500,
    5_000,
    10_000,
    50_000,
    u64::MAX,
];

#[derive(Debug, Clone, Copy)]
pub enum Counter {
    RawMessages,
    NormalizedEvents,
    Duplicates,
    Gaps,
    Recoveries,
    Reconnects,
    OrdersSubmitted,
    OrdersRejected,
    Fills,
    RiskRejects,
    StorageDropped,
}

#[derive(Debug, Clone, Copy)]
pub enum Gauge {
    FeedLagUs,
    BookAgeMs,
    PrivateAgeMs,
    MarketQueueDepth,
    PrivateQueueDepth,
    StorageQueueDepth,
}

#[derive(Debug, Clone, Copy)]
pub enum LatencyKind {
    Command,
    Ack,
    Fill,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct HistogramSnapshot {
    pub upper_bounds_us: Vec<u64>,
    pub counts: Vec<u64>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct MetricsSnapshot {
    pub counters: BTreeMap<String, u64>,
    pub gauges: BTreeMap<String, u64>,
    pub latencies: BTreeMap<String, HistogramSnapshot>,
}

#[derive(Debug, Default)]
struct Counters {
    raw_messages: AtomicU64,
    normalized_events: AtomicU64,
    duplicates: AtomicU64,
    gaps: AtomicU64,
    recoveries: AtomicU64,
    reconnects: AtomicU64,
    orders_submitted: AtomicU64,
    orders_rejected: AtomicU64,
    fills: AtomicU64,
    risk_rejects: AtomicU64,
    storage_dropped: AtomicU64,
}

#[derive(Debug, Default)]
struct Gauges {
    feed_lag_us: AtomicU64,
    book_age_ms: AtomicU64,
    private_age_ms: AtomicU64,
    market_queue_depth: AtomicU64,
    private_queue_depth: AtomicU64,
    storage_queue_depth: AtomicU64,
}

#[derive(Debug)]
struct LatencyHistogram {
    counts: [AtomicU64; LATENCY_BUCKETS_US.len()],
}

impl Default for LatencyHistogram {
    fn default() -> Self {
        Self {
            counts: std::array::from_fn(|_| AtomicU64::new(0)),
        }
    }
}

impl LatencyHistogram {
    fn record(&self, latency_us: u64) {
        let index = LATENCY_BUCKETS_US
            .iter()
            .position(|bound| latency_us <= *bound)
            .unwrap_or(LATENCY_BUCKETS_US.len() - 1);
        self.counts[index].fetch_add(1, Ordering::Relaxed);
    }

    fn snapshot(&self) -> HistogramSnapshot {
        HistogramSnapshot {
            upper_bounds_us: LATENCY_BUCKETS_US.to_vec(),
            counts: self
                .counts
                .iter()
                .map(|count| count.load(Ordering::Relaxed))
                .collect(),
        }
    }
}

#[derive(Debug, Default)]
pub struct Metrics {
    counters: Counters,
    gauges: Gauges,
    command_latency: LatencyHistogram,
    ack_latency: LatencyHistogram,
    fill_latency: LatencyHistogram,
}

impl Metrics {
    pub fn new() -> Arc<Self> {
        Arc::new(Self::default())
    }

    pub fn increment(&self, counter: Counter, amount: u64) {
        counter_atomic(&self.counters, counter).fetch_add(amount, Ordering::Relaxed);
    }

    pub fn set_gauge(&self, gauge: Gauge, value: u64) {
        gauge_atomic(&self.gauges, gauge).store(value, Ordering::Relaxed);
    }

    pub fn record_latency(&self, kind: LatencyKind, latency_us: u64) {
        match kind {
            LatencyKind::Command => &self.command_latency,
            LatencyKind::Ack => &self.ack_latency,
            LatencyKind::Fill => &self.fill_latency,
        }
        .record(latency_us);
    }

    pub fn snapshot(&self) -> MetricsSnapshot {
        MetricsSnapshot {
            counters: BTreeMap::from([
                (
                    "raw_messages".to_string(),
                    load(&self.counters.raw_messages),
                ),
                (
                    "normalized_events".to_string(),
                    load(&self.counters.normalized_events),
                ),
                ("duplicates".to_string(), load(&self.counters.duplicates)),
                ("gaps".to_string(), load(&self.counters.gaps)),
                ("recoveries".to_string(), load(&self.counters.recoveries)),
                ("reconnects".to_string(), load(&self.counters.reconnects)),
                (
                    "orders_submitted".to_string(),
                    load(&self.counters.orders_submitted),
                ),
                (
                    "orders_rejected".to_string(),
                    load(&self.counters.orders_rejected),
                ),
                ("fills".to_string(), load(&self.counters.fills)),
                (
                    "risk_rejects".to_string(),
                    load(&self.counters.risk_rejects),
                ),
                (
                    "storage_dropped".to_string(),
                    load(&self.counters.storage_dropped),
                ),
            ]),
            gauges: BTreeMap::from([
                ("feed_lag_us".to_string(), load(&self.gauges.feed_lag_us)),
                ("book_age_ms".to_string(), load(&self.gauges.book_age_ms)),
                (
                    "private_age_ms".to_string(),
                    load(&self.gauges.private_age_ms),
                ),
                (
                    "market_queue_depth".to_string(),
                    load(&self.gauges.market_queue_depth),
                ),
                (
                    "private_queue_depth".to_string(),
                    load(&self.gauges.private_queue_depth),
                ),
                (
                    "storage_queue_depth".to_string(),
                    load(&self.gauges.storage_queue_depth),
                ),
            ]),
            latencies: BTreeMap::from([
                ("command".to_string(), self.command_latency.snapshot()),
                ("ack".to_string(), self.ack_latency.snapshot()),
                ("fill".to_string(), self.fill_latency.snapshot()),
            ]),
        }
    }
}

fn counter_atomic(counters: &Counters, counter: Counter) -> &AtomicU64 {
    match counter {
        Counter::RawMessages => &counters.raw_messages,
        Counter::NormalizedEvents => &counters.normalized_events,
        Counter::Duplicates => &counters.duplicates,
        Counter::Gaps => &counters.gaps,
        Counter::Recoveries => &counters.recoveries,
        Counter::Reconnects => &counters.reconnects,
        Counter::OrdersSubmitted => &counters.orders_submitted,
        Counter::OrdersRejected => &counters.orders_rejected,
        Counter::Fills => &counters.fills,
        Counter::RiskRejects => &counters.risk_rejects,
        Counter::StorageDropped => &counters.storage_dropped,
    }
}

fn gauge_atomic(gauges: &Gauges, gauge: Gauge) -> &AtomicU64 {
    match gauge {
        Gauge::FeedLagUs => &gauges.feed_lag_us,
        Gauge::BookAgeMs => &gauges.book_age_ms,
        Gauge::PrivateAgeMs => &gauges.private_age_ms,
        Gauge::MarketQueueDepth => &gauges.market_queue_depth,
        Gauge::PrivateQueueDepth => &gauges.private_queue_depth,
        Gauge::StorageQueueDepth => &gauges.storage_queue_depth,
    }
}

fn load(value: &AtomicU64) -> u64 {
    value.load(Ordering::Relaxed)
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum HealthStatus {
    Healthy,
    Degraded,
    Unhealthy,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ComponentHealth {
    pub component: String,
    pub status: HealthStatus,
    pub updated_at_ms: u64,
    pub reason: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct HealthSnapshot {
    pub overall: HealthStatus,
    pub components: Vec<ComponentHealth>,
}

#[derive(Debug, Default)]
pub struct HealthRegistry {
    components: RwLock<HashMap<String, ComponentHealth>>,
}

impl HealthRegistry {
    pub fn new() -> Arc<Self> {
        Arc::new(Self::default())
    }

    pub fn set(
        &self,
        component: impl Into<String>,
        status: HealthStatus,
        updated_at_ms: u64,
        reason: impl Into<String>,
    ) {
        let component = component.into();
        self.components.write().unwrap().insert(
            component.clone(),
            ComponentHealth {
                component,
                status,
                updated_at_ms,
                reason: reason.into(),
            },
        );
    }

    pub fn snapshot(&self) -> HealthSnapshot {
        let mut components = self
            .components
            .read()
            .unwrap()
            .values()
            .cloned()
            .collect::<Vec<_>>();
        components.sort_by(|left, right| left.component.cmp(&right.component));
        let overall = components
            .iter()
            .map(|component| component.status)
            .max_by_key(|status| match status {
                HealthStatus::Healthy => 0,
                HealthStatus::Degraded => 1,
                HealthStatus::Unhealthy => 2,
            })
            .unwrap_or(HealthStatus::Healthy);
        HealthSnapshot {
            overall,
            components,
        }
    }
}

pub fn init_json_tracing(default_filter: &str) -> Result<(), String> {
    let filter =
        EnvFilter::try_from_default_env().unwrap_or_else(|_| EnvFilter::new(default_filter));
    tracing_subscriber::fmt()
        .json()
        .with_writer(std::io::stderr)
        .with_env_filter(filter)
        .finish()
        .try_init()
        .map_err(|error| error.to_string())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn metrics_are_atomic_and_latency_is_bucketed() {
        let metrics = Metrics::new();
        metrics.increment(Counter::Duplicates, 2);
        metrics.set_gauge(Gauge::MarketQueueDepth, 7);
        metrics.record_latency(LatencyKind::Ack, 80);
        let snapshot = metrics.snapshot();

        assert_eq!(snapshot.counters["duplicates"], 2);
        assert_eq!(snapshot.gauges["market_queue_depth"], 7);
        assert_eq!(snapshot.latencies["ack"].counts[3], 1);
    }

    #[test]
    fn health_rolls_up_worst_component() {
        let health = HealthRegistry::new();
        health.set("feed", HealthStatus::Healthy, 1, "ready");
        health.set("private", HealthStatus::Unhealthy, 2, "stale");

        let snapshot = health.snapshot();
        assert_eq!(snapshot.overall, HealthStatus::Unhealthy);
        assert_eq!(snapshot.components.len(), 2);
    }
}
