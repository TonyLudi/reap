use super::*;

pub(super) fn command_output(program: &str, arguments: &[&str]) -> String {
    Command::new(program)
        .args(arguments)
        .output()
        .ok()
        .filter(|output| output.status.success())
        .and_then(|output| String::from_utf8(output.stdout).ok())
        .map(|output| output.trim().to_string())
        .unwrap_or_else(|| "unknown".to_string())
}

pub(super) fn timer_overhead() -> Distribution {
    let mut samples = Vec::with_capacity(TIMED_OBSERVATIONS);
    for _ in 0..TIMED_OBSERVATIONS {
        let started = Instant::now();
        samples.push(duration_ns(started.elapsed()));
    }
    Distribution::from_samples(samples)
}

pub(super) fn duration_ns(duration: Duration) -> u64 {
    u64::try_from(duration.as_nanos()).unwrap_or(u64::MAX)
}

pub(super) fn run_workload<State>(
    name: &'static str,
    included_boundary: &'static str,
    excluded_boundary: &'static str,
    setup: impl Fn() -> State,
    mut observe: impl FnMut(&mut State, usize) -> Observation,
) -> WorkloadResult {
    let mut timing_state = setup();
    for index in 0..WARMUP_OBSERVATIONS {
        black_box(observe(&mut timing_state, index));
    }

    let mut elapsed_samples = Vec::with_capacity(TIMED_OBSERVATIONS);
    let mut queue_age_samples = Vec::with_capacity(TIMED_OBSERVATIONS);
    let mut counters = LogicalCounters::default();
    for offset in 0..TIMED_OBSERVATIONS {
        let index = WARMUP_OBSERVATIONS + offset;
        let started = Instant::now();
        let observation = observe(&mut timing_state, index);
        elapsed_samples.push(duration_ns(started.elapsed()));
        if let Some(queue_age_ns) = observation.queue_age_ns {
            queue_age_samples.push(queue_age_ns);
        }
        counters.merge(observation.counters);
    }
    drop(timing_state);

    let mut allocation_state = setup();
    for index in 0..WARMUP_OBSERVATIONS {
        black_box(observe(&mut allocation_state, index));
    }
    let mut allocation_counters = LogicalCounters::default();
    start_allocation_tracking();
    for offset in 0..TIMED_OBSERVATIONS {
        let index = WARMUP_OBSERVATIONS + offset;
        let observation = observe(&mut allocation_state, index);
        allocation_counters.merge(observation.counters);
        black_box(observation.queue_age_ns);
    }
    let allocations = stop_allocation_tracking();
    assert_eq!(
        allocation_counters, counters,
        "{name} logical counters must match between timing and allocation passes"
    );
    assert_eq!(
        counters.inputs, TIMED_OBSERVATIONS as u64,
        "{name} must count exactly one input per timed observation"
    );
    let produced_actions = counters.produced_actions();
    let inputs = counters.inputs as f64;
    let allocations = AllocationRates {
        total: allocations,
        calls_per_input: allocations.calls as f64 / inputs,
        requested_bytes_per_input: allocations.requested_bytes as f64 / inputs,
        calls_per_produced_action: (produced_actions > 0)
            .then_some(allocations.calls as f64 / produced_actions as f64),
        requested_bytes_per_produced_action: (produced_actions > 0)
            .then_some(allocations.requested_bytes as f64 / produced_actions as f64),
    };

    WorkloadResult {
        name,
        warmup_observations: WARMUP_OBSERVATIONS,
        timed_observations: TIMED_OBSERVATIONS,
        percentile_algorithm: "exact nearest-rank",
        elapsed: Distribution::from_samples(elapsed_samples),
        queue_age: (!queue_age_samples.is_empty())
            .then(|| Distribution::from_samples(queue_age_samples)),
        counters,
        allocations,
        included_boundary,
        excluded_boundary,
    }
}

pub(super) fn print_human_result(result: &WorkloadResult) {
    let elapsed = result.elapsed;
    let allocation = &result.allocations;
    println!(
        "{}: observations={} p50_ns={} p95_ns={} p99_ns={} p99.9_ns={} max_ns={} \
         allocation_calls={} requested_bytes={} calls/input={:.3} bytes/input={:.1} \
         frames={} parsed={} feed_outputs={} normalized={} typed={} rejected={} \
         safety_candidates={} prepared_submits={} prepared_cancels={} actions={} \
         produced_actions={} storage_records={} queue_capacity={} queue_high_water={} \
         queue_saturations={}",
        result.name,
        result.timed_observations,
        elapsed.p50_ns,
        elapsed.p95_ns,
        elapsed.p99_ns,
        elapsed.p99_9_ns,
        elapsed.max_ns,
        allocation.total.calls,
        allocation.total.requested_bytes,
        allocation.calls_per_input,
        allocation.requested_bytes_per_input,
        result.counters.frames,
        result.counters.parsed_events,
        result.counters.feed_outputs,
        result.counters.normalized_outputs,
        result.counters.typed_intents,
        result.counters.risk_rejections,
        result.counters.safety_cancel_candidates,
        result.counters.prepared_submits,
        result.counters.prepared_cancels,
        result.counters.coordinator_actions,
        result.counters.produced_actions,
        result.counters.storage_records,
        result.counters.queue_capacity,
        result.counters.queue_high_water,
        result.counters.queue_saturations,
    );
    if let Some(age) = result.queue_age {
        println!(
            "{} queue_age: samples={} p50_ns={} p95_ns={} p99_ns={} p99.9_ns={} max_ns={}",
            result.name, age.samples, age.p50_ns, age.p95_ns, age.p99_ns, age.p99_9_ns, age.max_ns,
        );
    }
}
