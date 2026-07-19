use std::alloc::{GlobalAlloc, Layout, System};
use std::hint::black_box;
use std::process::Command;
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use std::time::Instant;

use super::*;

const WARMUP_OBSERVATIONS: usize = 10_000;
const TIMED_OBSERVATIONS: usize = 100_000;
const WS_REQUEST_ID: &str = "goalDbench1";
const WS_EXPIRY_MS: u64 = 123_456;

struct TrackingAllocator;

static ALLOCATION_TRACKING_ENABLED: AtomicBool = AtomicBool::new(false);
static ALLOCATION_CALLS: AtomicU64 = AtomicU64::new(0);
static ALLOCATION_BYTES: AtomicU64 = AtomicU64::new(0);

#[global_allocator]
static TRACKING_ALLOCATOR: TrackingAllocator = TrackingAllocator;

unsafe impl GlobalAlloc for TrackingAllocator {
    unsafe fn alloc(&self, layout: Layout) -> *mut u8 {
        if ALLOCATION_TRACKING_ENABLED.load(Ordering::Relaxed) {
            ALLOCATION_CALLS.fetch_add(1, Ordering::Relaxed);
            ALLOCATION_BYTES.fetch_add(layout.size() as u64, Ordering::Relaxed);
        }
        unsafe { System.alloc(layout) }
    }

    unsafe fn alloc_zeroed(&self, layout: Layout) -> *mut u8 {
        if ALLOCATION_TRACKING_ENABLED.load(Ordering::Relaxed) {
            ALLOCATION_CALLS.fetch_add(1, Ordering::Relaxed);
            ALLOCATION_BYTES.fetch_add(layout.size() as u64, Ordering::Relaxed);
        }
        unsafe { System.alloc_zeroed(layout) }
    }

    unsafe fn dealloc(&self, pointer: *mut u8, layout: Layout) {
        unsafe { System.dealloc(pointer, layout) }
    }

    unsafe fn realloc(&self, pointer: *mut u8, layout: Layout, new_size: usize) -> *mut u8 {
        if ALLOCATION_TRACKING_ENABLED.load(Ordering::Relaxed) {
            ALLOCATION_CALLS.fetch_add(1, Ordering::Relaxed);
            ALLOCATION_BYTES.fetch_add(new_size as u64, Ordering::Relaxed);
        }
        unsafe { System.realloc(pointer, layout, new_size) }
    }
}

#[derive(Clone, Copy)]
struct AllocationTotals {
    calls: u64,
    bytes: u64,
}

fn start_allocation_tracking() {
    ALLOCATION_TRACKING_ENABLED.store(false, Ordering::Relaxed);
    ALLOCATION_CALLS.store(0, Ordering::Relaxed);
    ALLOCATION_BYTES.store(0, Ordering::Relaxed);
    ALLOCATION_TRACKING_ENABLED.store(true, Ordering::Relaxed);
}

fn stop_allocation_tracking() -> AllocationTotals {
    ALLOCATION_TRACKING_ENABLED.store(false, Ordering::Relaxed);
    AllocationTotals {
        calls: ALLOCATION_CALLS.load(Ordering::Relaxed),
        bytes: ALLOCATION_BYTES.load(Ordering::Relaxed),
    }
}

#[derive(serde::Serialize)]
struct LatencySummary {
    samples: usize,
    p50_ns: u64,
    p95_ns: u64,
    p99_ns: u64,
    p99_9_ns: u64,
    max_ns: u64,
    dropped_or_overflowed_samples: u64,
}

impl LatencySummary {
    fn from_samples(mut samples: Vec<u64>) -> Self {
        assert!(!samples.is_empty(), "latency sample set must be non-empty");
        samples.sort_unstable();
        Self {
            samples: samples.len(),
            p50_ns: nearest_rank(&samples, 500, 1_000),
            p95_ns: nearest_rank(&samples, 950, 1_000),
            p99_ns: nearest_rank(&samples, 990, 1_000),
            p99_9_ns: nearest_rank(&samples, 999, 1_000),
            max_ns: *samples.last().unwrap(),
            dropped_or_overflowed_samples: 0,
        }
    }
}

#[derive(serde::Serialize)]
struct ExactRate {
    numerator: u64,
    denominator: u64,
}

#[derive(serde::Serialize)]
struct AllocationSummary {
    calls_total: u64,
    requested_bytes_total: u64,
    calls_per_input: ExactRate,
    requested_bytes_per_input: ExactRate,
    calls_per_action: ExactRate,
    requested_bytes_per_action: ExactRate,
}

#[derive(serde::Serialize)]
struct LogicalCounts {
    prepared_submit_inputs: u64,
    regular_place_orders: u64,
    rest_inner_bodies: u64,
    websocket_order_requests: u64,
    serialized_order_actions: u64,
    serialized_output_bytes: u64,
}

#[derive(serde::Serialize)]
struct Workload {
    name: &'static str,
    operation: &'static str,
    observations: u64,
    actions: u64,
    logical_counts: LogicalCounts,
    latency: LatencySummary,
    allocations: AllocationSummary,
}

#[derive(serde::Serialize)]
struct Report {
    schema_version: u8,
    benchmark: &'static str,
    command: &'static str,
    rustc: String,
    host: HostDescription,
    monotonic_clock: &'static str,
    latency_timer_overhead_adjusted: bool,
    measurement_passes: &'static str,
    allocation_scope: &'static str,
    percentile_algorithm: &'static str,
    sample_precision: &'static str,
    included_boundary: &'static str,
    excluded_boundary: &'static str,
    warmup_per_workload: u64,
    observations_per_workload: u64,
    timer_overhead_observations: u64,
    timer_read_overhead: LatencySummary,
    workloads: [Workload; 2],
}

#[derive(serde::Serialize)]
struct HostDescription {
    os: &'static str,
    architecture: &'static str,
    hostname: String,
    available_parallelism: usize,
}

fn command_output(program: &str, arguments: &[&str]) -> String {
    Command::new(program)
        .args(arguments)
        .output()
        .ok()
        .filter(|output| output.status.success())
        .and_then(|output| String::from_utf8(output.stdout).ok())
        .map(|output| output.trim().to_string())
        .unwrap_or_else(|| "unknown".to_string())
}

fn nearest_rank(samples: &[u64], numerator: usize, denominator: usize) -> u64 {
    assert!(numerator > 0 && numerator <= denominator);
    let rank = samples
        .len()
        .checked_mul(numerator)
        .and_then(|value| value.checked_add(denominator - 1))
        .expect("nearest-rank calculation overflowed")
        / denominator;
    samples[rank - 1]
}

fn elapsed_ns(started: Instant) -> u64 {
    u64::try_from(started.elapsed().as_nanos()).unwrap_or(u64::MAX)
}

fn timer_overhead() -> LatencySummary {
    let mut samples = Vec::with_capacity(TIMED_OBSERVATIONS);
    for _ in 0..TIMED_OBSERVATIONS {
        let started = Instant::now();
        black_box(());
        samples.push(elapsed_ns(started));
    }
    LatencySummary::from_samples(samples)
}

fn allocation_summary(totals: AllocationTotals, units: u64) -> AllocationSummary {
    AllocationSummary {
        calls_total: totals.calls,
        requested_bytes_total: totals.bytes,
        calls_per_input: ExactRate {
            numerator: totals.calls,
            denominator: units,
        },
        requested_bytes_per_input: ExactRate {
            numerator: totals.bytes,
            denominator: units,
        },
        calls_per_action: ExactRate {
            numerator: totals.calls,
            denominator: units,
        },
        requested_bytes_per_action: ExactRate {
            numerator: totals.bytes,
            denominator: units,
        },
    }
}

fn measure(
    name: &'static str,
    prepared: &PreparedRegularSubmit,
    rest_inner_bodies_per_input: u64,
    websocket_requests_per_input: u64,
    mut serialize: impl FnMut(&PreparedRegularSubmit) -> String,
) -> Workload {
    for _ in 0..WARMUP_OBSERVATIONS {
        black_box(serialize(black_box(prepared)));
    }

    let mut samples = Vec::with_capacity(TIMED_OBSERVATIONS);
    let mut serialized_output_bytes = 0_u64;
    for _ in 0..TIMED_OBSERVATIONS {
        let started = Instant::now();
        let output = serialize(black_box(prepared));
        black_box(&output);
        samples.push(elapsed_ns(started));
        serialized_output_bytes = serialized_output_bytes
            .checked_add(output.len() as u64)
            .expect("serialized output byte count overflowed");
    }

    for _ in 0..WARMUP_OBSERVATIONS {
        black_box(serialize(black_box(prepared)));
    }
    let mut allocation_output_bytes = 0_u64;
    start_allocation_tracking();
    for _ in 0..TIMED_OBSERVATIONS {
        let output = serialize(black_box(prepared));
        black_box(&output);
        allocation_output_bytes = allocation_output_bytes
            .checked_add(output.len() as u64)
            .expect("allocation-pass output byte count overflowed");
    }
    let allocations = stop_allocation_tracking();
    assert_eq!(
        allocation_output_bytes, serialized_output_bytes,
        "{name} output changed between timing and allocation passes"
    );

    let observations = TIMED_OBSERVATIONS as u64;
    Workload {
        name,
        operation: "order",
        observations,
        actions: observations,
        logical_counts: LogicalCounts {
            prepared_submit_inputs: observations,
            regular_place_orders: observations,
            rest_inner_bodies: observations * rest_inner_bodies_per_input,
            websocket_order_requests: observations * websocket_requests_per_input,
            serialized_order_actions: observations,
            serialized_output_bytes,
        },
        latency: LatencySummary::from_samples(samples),
        allocations: allocation_summary(allocations, observations),
    }
}

fn print_human(workload: &Workload) {
    println!(
        "{}: operation={} observations={} actions={} samples={} dropped_or_overflowed={} \
         p50_ns={} p95_ns={} p99_ns={} p99.9_ns={} max_ns={} allocation_calls={} \
         requested_bytes={} calls/input={:.6} bytes/input={:.6} calls/action={:.6} \
         bytes/action={:.6} output_bytes={}",
        workload.name,
        workload.operation,
        workload.observations,
        workload.actions,
        workload.latency.samples,
        workload.latency.dropped_or_overflowed_samples,
        workload.latency.p50_ns,
        workload.latency.p95_ns,
        workload.latency.p99_ns,
        workload.latency.p99_9_ns,
        workload.latency.max_ns,
        workload.allocations.calls_total,
        workload.allocations.requested_bytes_total,
        workload.allocations.calls_total as f64 / workload.observations as f64,
        workload.allocations.requested_bytes_total as f64 / workload.observations as f64,
        workload.allocations.calls_total as f64 / workload.actions as f64,
        workload.allocations.requested_bytes_total as f64 / workload.actions as f64,
        workload.logical_counts.serialized_output_bytes,
    );
}

pub(super) fn run() {
    let (prepared, wire, _) = typed_strategy_prepared_submit();
    assert_eq!(prepared.account_id(), "main");

    let order = regular_place_order(&prepared);
    let rest_body = serialize_place(&order).unwrap();
    let websocket_request = build_ws_place_request(WS_REQUEST_ID, WS_EXPIRY_MS, &order).unwrap();
    let rest_body: serde_json::Value = serde_json::from_str(&rest_body).unwrap();
    let websocket_request: serde_json::Value = serde_json::from_str(&websocket_request).unwrap();
    assert_eq!(websocket_request["op"], "order");
    assert_eq!(websocket_request["args"].as_array().unwrap().len(), 1);
    for field in ["px", "sz"] {
        let rest_field = rest_body[field]
            .as_str()
            .expect("REST-shaped field must be a string");
        let websocket_field = websocket_request["args"][0][field]
            .as_str()
            .expect("websocket argument field must be a string");
        assert_eq!(
            rest_field.as_bytes(),
            websocket_field.as_bytes(),
            "REST-shaped inner body and websocket argument diverged for {field}"
        );
    }
    assert_trace(&wire, &[]);

    let timer_read_overhead = timer_overhead();
    let workloads = [
        measure("prepared_to_rest_inner_body", &prepared, 1, 0, |prepared| {
            let order = regular_place_order(black_box(prepared));
            serialize_place(black_box(&order)).unwrap()
        }),
        measure(
            "prepared_to_websocket_order_request",
            &prepared,
            0,
            1,
            |prepared| {
                let order = regular_place_order(black_box(prepared));
                build_ws_place_request(WS_REQUEST_ID, WS_EXPIRY_MS, black_box(&order)).unwrap()
            },
        ),
    ];
    println!(
        "timer_read_overhead: observations={} samples={} dropped_or_overflowed={} p50_ns={} \
         p95_ns={} p99_ns={} p99.9_ns={} max_ns={} (workload latencies unadjusted)",
        TIMED_OBSERVATIONS,
        timer_read_overhead.samples,
        timer_read_overhead.dropped_or_overflowed_samples,
        timer_read_overhead.p50_ns,
        timer_read_overhead.p95_ns,
        timer_read_overhead.p99_ns,
        timer_read_overhead.p99_9_ns,
        timer_read_overhead.max_ns,
    );
    for workload in &workloads {
        print_human(workload);
    }

    let report = Report {
        schema_version: 1,
        benchmark: "reap-okx-live-adapter/prepared_serializer",
        command: concat!(
            "cargo test --release -p reap-okx-live-adapter --locked ",
            "tests::goal_d_prepared_serializer_benchmark -- --ignored --exact --nocapture"
        ),
        rustc: command_output("rustc", &["--version"]),
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
        latency_timer_overhead_adjusted: false,
        measurement_passes: "latency distributions are collected with allocation tracking off; \
            allocation calls and requested bytes are collected in a separate pass after the same \
            warm-up; serialized byte counts must match",
        allocation_scope: "process-global test allocator; run with the documented exact \
            single-test command",
        percentile_algorithm: "exact nearest-rank over all post-warmup observations",
        sample_precision: "one u64 nanosecond sample per timed observation; all samples retained; \
            no histogram, interpolation, downsampling, drops, or overflow",
        included_boundary: "already-PreparedRegularSubmit -> adapter-private regular_place_order \
            -> actual REST-shaped inner-body or websocket order serializer",
        excluded_boundary: "typed strategy, policy approval, reservation, and gateway preparation \
            run once as untimed fixture setup; credentials, signing, network I/O, transport \
            queues, exchange acknowledgement, and cancel/algo/spread operations are excluded",
        warmup_per_workload: WARMUP_OBSERVATIONS as u64,
        observations_per_workload: TIMED_OBSERVATIONS as u64,
        timer_overhead_observations: TIMED_OBSERVATIONS as u64,
        timer_read_overhead,
        workloads,
    };
    println!(
        "GOAL_D_PREPARED_SERIALIZER_JSON={}",
        serde_json::to_string(&report).unwrap()
    );
}
