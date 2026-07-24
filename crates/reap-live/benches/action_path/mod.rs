use std::alloc::{GlobalAlloc, Layout, System};
use std::collections::{HashMap, HashSet};
use std::hint::black_box;
use std::process::Command;
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use std::time::{Duration, Instant};

use async_trait::async_trait;
use reap_core::{
    AccountUpdate, Balance, Channel, ConnId, Level, MarketEvent, NewOrder, NormalizedEvent,
    OkxVenue, OrderBook, OrderEvent, OrderStatus, OrderUpdate, Position, RawEnvelope, Side,
    SystemEvent, SystemEventKind, TimeInForce, Venue,
};
use reap_engine::{ChaosEngineOutput, TradingEngine};
use reap_feed::{FeedOutput, FeedProcessor, payload_hash};
use reap_live::{
    LiveConfig, LiveCoordinator, ReconciliationResult, VerifiedBootstrap, VerifiedInstrument,
};
use reap_order::{
    CancelOrderTransportError, OkxOrderGateway, OrderTransportError, OwnedRegularOrders,
    PacingPolicy, PreparedRegularCancel, PreparedRegularSubmit, PrivateStateReducer,
    RegularExecution, RegularExecutionPolicy, RegularExecutionProfile, RegularReconciliation,
    SubmitPreparation,
};
use reap_risk::{InstrumentOrderLimits, InstrumentRiskModel, RiskGate, RiskLimits};
use reap_storage::StorageRecord;
use reap_strategy::{
    ChaosConfig, ChaosExecutionIntent, ChaosExecutionPurpose, ChaosStrategy, InstrumentKindConfig,
    ReferenceDataKind,
};
use reap_venue::okx::{
    OkxAdapter, OkxFillPage, OkxInstrumentType, OkxOrderAck, OkxRegularOrderPage, OkxTradeMode,
    RestError,
};
use reap_venue::{RemoteOrder, VenueAdapter};
use serde::Serialize;
use serde_json::json;
use tokio::sync::mpsc;

mod coordinator_workloads;
mod engine_support;
mod engine_workloads;
mod harness;
mod preparation;
mod raw_action_workload;

use coordinator_workloads::*;
use engine_support::*;
use engine_workloads::*;
use harness::*;
use preparation::*;
use raw_action_workload::*;

const WARMUP_OBSERVATIONS: usize = 10_000;
const TIMED_OBSERVATIONS: usize = 100_000;
const BASE_TS_MS: u64 = 1_700_000_000_000;
const ACCOUNT_ID: &str = "main";
const ENGINE_INCLUDED: &str = "owned normalized event delivery; production ChaosStrategy; \
    TradingEngine<ChaosStrategy>; RiskGate post/pre-trade decisions; typed intent traversal";
const ENGINE_EXCLUDED: &str = "socket receive; websocket frame decoding; OKX wire parsing; feed \
    deduplication/book reduction; LiveCoordinator reduction; production runtime channel \
    scheduling; regular execution policy/gateway preparation; storage enqueue and disk IO; \
    network IO; exchange acknowledgement; adapter REST/websocket serialization";
const PREPARED_ACTION_EXCLUDED: &str = "socket receive; websocket frame decoding; OKX wire \
    parsing; feed deduplication/book reduction; LiveCoordinator reduction; production runtime \
    channel scheduling; adapter REST/websocket serialization; storage enqueue and disk IO; \
    network IO; exchange acknowledgement";
const PREPARED_BOUNDARY: &str = "regular execution policy authorization; generated canonical \
    client-order identity; same-turn PrivateStateReducer and OwnedRegularOrders reservation; \
    OkxOrderGateway idempotency and lowering through PreparedRegularSubmit/PreparedRegularCancel; \
    adapter serialization remains private, is excluded from this binary, and is measured by the \
    separate adapter-private goal_d_prepared_serializer_benchmark";

static TRACK_ALLOCATIONS: AtomicBool = AtomicBool::new(false);
static ALLOCATION_CALLS: AtomicU64 = AtomicU64::new(0);
static ALLOCATED_BYTES: AtomicU64 = AtomicU64::new(0);

struct TrackingAllocator;

#[global_allocator]
static GLOBAL_ALLOCATOR: TrackingAllocator = TrackingAllocator;

unsafe impl GlobalAlloc for TrackingAllocator {
    unsafe fn alloc(&self, layout: Layout) -> *mut u8 {
        // SAFETY: the request is delegated unchanged to the system allocator.
        let pointer = unsafe { System.alloc(layout) };
        track_allocation(layout.size());
        pointer
    }

    unsafe fn alloc_zeroed(&self, layout: Layout) -> *mut u8 {
        // SAFETY: the request is delegated unchanged to the system allocator.
        let pointer = unsafe { System.alloc_zeroed(layout) };
        track_allocation(layout.size());
        pointer
    }

    unsafe fn dealloc(&self, pointer: *mut u8, layout: Layout) {
        // SAFETY: the pointer and layout came from the system allocator above.
        unsafe { System.dealloc(pointer, layout) };
    }

    unsafe fn realloc(&self, pointer: *mut u8, layout: Layout, new_size: usize) -> *mut u8 {
        // SAFETY: the pointer and layout came from the system allocator above.
        let pointer = unsafe { System.realloc(pointer, layout, new_size) };
        track_allocation(new_size);
        pointer
    }
}

fn track_allocation(bytes: usize) {
    if TRACK_ALLOCATIONS.load(Ordering::Relaxed) {
        ALLOCATION_CALLS.fetch_add(1, Ordering::Relaxed);
        ALLOCATED_BYTES.fetch_add(bytes as u64, Ordering::Relaxed);
    }
}

#[derive(Debug, Clone, Copy, Serialize)]
struct AllocationSnapshot {
    calls: u64,
    requested_bytes: u64,
}

fn start_allocation_tracking() {
    ALLOCATION_CALLS.store(0, Ordering::Relaxed);
    ALLOCATED_BYTES.store(0, Ordering::Relaxed);
    TRACK_ALLOCATIONS.store(true, Ordering::SeqCst);
}

fn stop_allocation_tracking() -> AllocationSnapshot {
    TRACK_ALLOCATIONS.store(false, Ordering::SeqCst);
    AllocationSnapshot {
        calls: ALLOCATION_CALLS.load(Ordering::Relaxed),
        requested_bytes: ALLOCATED_BYTES.load(Ordering::Relaxed),
    }
}

#[derive(Debug, Default, Clone, Copy, PartialEq, Eq, Serialize)]
struct LogicalCounters {
    inputs: u64,
    frames: u64,
    parsed_events: u64,
    feed_outputs: u64,
    normalized_outputs: u64,
    typed_intents: u64,
    quote_intents: u64,
    hedge_intents: u64,
    cancel_owned_intents: u64,
    risk_rejections: u64,
    system_events: u64,
    safety_cancel_candidates: u64,
    prepared_submits: u64,
    prepared_cancels: u64,
    coordinator_actions: u64,
    storage_records: u64,
    trade_reprice_actions: u64,
    control_dequeues: u64,
    feed_dequeues: u64,
    biased_control_preemptions: u64,
    queue_capacity: u64,
    queue_high_water: u64,
    queue_saturations: u64,
    produced_actions: u64,
}

impl LogicalCounters {
    fn merge(&mut self, other: Self) {
        self.inputs += other.inputs;
        self.frames += other.frames;
        self.parsed_events += other.parsed_events;
        self.feed_outputs += other.feed_outputs;
        self.normalized_outputs += other.normalized_outputs;
        self.typed_intents += other.typed_intents;
        self.quote_intents += other.quote_intents;
        self.hedge_intents += other.hedge_intents;
        self.cancel_owned_intents += other.cancel_owned_intents;
        self.risk_rejections += other.risk_rejections;
        self.system_events += other.system_events;
        self.safety_cancel_candidates += other.safety_cancel_candidates;
        self.prepared_submits += other.prepared_submits;
        self.prepared_cancels += other.prepared_cancels;
        self.coordinator_actions += other.coordinator_actions;
        self.storage_records += other.storage_records;
        self.trade_reprice_actions += other.trade_reprice_actions;
        self.control_dequeues += other.control_dequeues;
        self.feed_dequeues += other.feed_dequeues;
        self.biased_control_preemptions += other.biased_control_preemptions;
        self.queue_capacity = self.queue_capacity.max(other.queue_capacity);
        self.queue_high_water = self.queue_high_water.max(other.queue_high_water);
        self.queue_saturations += other.queue_saturations;
        self.produced_actions += other.produced_actions;
    }

    fn produced_actions(self) -> u64 {
        self.produced_actions
    }
}

#[derive(Debug, Default)]
struct Observation {
    counters: LogicalCounters,
    queue_age_ns: Option<u64>,
}

#[derive(Debug, Clone, Copy, Serialize)]
struct Distribution {
    samples: usize,
    p50_ns: u64,
    p95_ns: u64,
    p99_ns: u64,
    p99_9_ns: u64,
    max_ns: u64,
    dropped_or_overflowed_samples: u64,
}

impl Distribution {
    fn from_samples(mut samples: Vec<u64>) -> Self {
        assert!(!samples.is_empty(), "a distribution needs samples");
        samples.sort_unstable();
        Self {
            samples: samples.len(),
            p50_ns: nearest_rank(&samples, 500, 1_000),
            p95_ns: nearest_rank(&samples, 950, 1_000),
            p99_ns: nearest_rank(&samples, 990, 1_000),
            p99_9_ns: nearest_rank(&samples, 999, 1_000),
            max_ns: *samples.last().expect("samples are nonempty"),
            dropped_or_overflowed_samples: 0,
        }
    }
}

fn nearest_rank(sorted: &[u64], numerator: usize, denominator: usize) -> u64 {
    let rank = sorted.len().saturating_mul(numerator).div_ceil(denominator);
    sorted[rank.saturating_sub(1).min(sorted.len() - 1)]
}

#[derive(Debug, Serialize)]
struct AllocationRates {
    total: AllocationSnapshot,
    calls_per_input: f64,
    requested_bytes_per_input: f64,
    calls_per_produced_action: Option<f64>,
    requested_bytes_per_produced_action: Option<f64>,
}

#[derive(Debug, Serialize)]
struct WorkloadResult {
    name: &'static str,
    warmup_observations: usize,
    timed_observations: usize,
    percentile_algorithm: &'static str,
    elapsed: Distribution,
    queue_age: Option<Distribution>,
    counters: LogicalCounters,
    allocations: AllocationRates,
    included_boundary: &'static str,
    excluded_boundary: &'static str,
}

#[derive(Debug, Serialize)]
struct BenchmarkReport {
    schema_version: u32,
    benchmark: &'static str,
    rustc: String,
    cargo: String,
    host: HostDescription,
    monotonic_clock: &'static str,
    sample_precision: &'static str,
    percentile_algorithm: &'static str,
    measurement_passes: &'static str,
    timer_read_overhead: Distribution,
    common_included_boundary: &'static str,
    common_excluded_boundary: &'static str,
    prepared_authority_boundary: &'static str,
    adapter_serialization_status: &'static str,
    workloads: Vec<WorkloadResult>,
}

#[derive(Debug, Serialize)]
struct HostDescription {
    os: &'static str,
    architecture: &'static str,
    hostname: String,
    available_parallelism: usize,
}

#[allow(clippy::assertions_on_constants)]
pub(crate) fn run() {
    assert!(
        TIMED_OBSERVATIONS >= 100_000,
        "Goal D requires at least 100,000 post-warmup observations"
    );

    let timer_read_overhead = timer_overhead();
    let workloads = vec![
        quote_creation_workload(),
        quote_replacement_workload(),
        ioc_hedge_workload(),
        risk_rejection_workload(),
        symbol_fail_close_workload(),
        global_fail_close_workload(),
        coordinator_reduction_workload(),
        raw_sequence_gap_action_record_workload(),
        public_trade_reprice_workload(),
        bounded_biased_control_feed_storm_workload(),
    ];
    for result in &workloads {
        print_human_result(result);
    }

    let report = BenchmarkReport {
        schema_version: 1,
        benchmark: "reap-live/action_path",
        rustc: command_output("rustc", &["--version"]),
        cargo: command_output("cargo", &["--version"]),
        host: HostDescription {
            os: std::env::consts::OS,
            architecture: std::env::consts::ARCH,
            hostname: std::fs::read_to_string("/etc/hostname")
                .unwrap_or_else(|_| "unknown".to_string())
                .trim()
                .to_string(),
            available_parallelism: std::thread::available_parallelism()
                .map(usize::from)
                .unwrap_or(1),
        },
        monotonic_clock: "std::time::Instant (process-local monotonic clock)",
        sample_precision: "one u64 nanosecond sample retained per timed observation; no histogram, \
            reservoir, downsampling, or interpolation",
        percentile_algorithm: "exact nearest-rank over all post-warmup observations",
        measurement_passes: "elapsed distributions are collected with allocation tracking off; \
            allocation calls/bytes are collected in a separate freshly initialized pass after \
            the same warm-up; exact logical counters must match between passes",
        timer_read_overhead,
        common_included_boundary: "benchmark harness, exact all-sample nearest-rank timing, and \
            a separate freshly initialized allocation pass only; production stages vary by \
            workload and are listed in each workload's included_boundary",
        common_excluded_boundary: "no production stage is globally excluded; consult each \
            workload's excluded_boundary",
        prepared_authority_boundary: PREPARED_BOUNDARY,
        adapter_serialization_status: "excluded from this binary and measured separately by the \
            adapter-private ignored release test goal_d_prepared_serializer_benchmark; no \
            serializer or authority constructor is made public",
        workloads,
    };
    println!(
        "ACTION_PATH_JSON={}",
        serde_json::to_string(&report).expect("benchmark report must serialize")
    );
}
