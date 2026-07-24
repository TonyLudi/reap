use std::path::PathBuf;
use std::process::Command;
use std::time::Instant;

use super::PmEvidenceError;
use super::contract::{
    FIXTURE_REVISION, MAX_REPLAY_WORKING_BYTES, MAX_RESERVED_CAPACITY_BYTES,
    MEASURED_EXTERNAL_OBSERVATIONS, MEASURED_OWNER_REDUCTIONS, PHYSICAL_JOURNAL_LINES,
};
use super::parser::run_parser_segment;
use super::report::{
    ActionPathReport, ActionPathSuiteReport, BoundaryMetadata, CombinedReplayReport,
    LatencySummary, RecoveryProjection, TimerMetadata, sha256_hex,
};
use super::workload::{
    BenchmarkWarmup, run_benchmark_warmup, run_benchmark_workload, run_real_writer_workload,
};
use crate::journal::{PmJournalLineV1, PmJournalRecordV1, PmJournalScopeV1};
use crate::recover_pm_mutation_journal;

const EXCLUDED_BOUNDARY: &[&str] = &[
    "socket receive",
    "websocket framing",
    "JSON parsing",
    "filesystem serialization and fsync",
    "network IO",
    "fake exchange service time",
];
const INCLUDED_BOUNDARY: &[&str] = &[
    "immutable normalized event delivery",
    "exact reducers and private/public readiness",
    "risk and fixture-model evaluation",
    "checked quote conversion",
    "ownership and bounded effect reservation",
    "PM mutation-record construction and bounded enqueue",
    "injected durable acknowledgement consumption",
    "prepared fake-effect enqueue",
];

pub(crate) fn run_action_path() -> Result<ActionPathSuiteReport, PmEvidenceError> {
    let warmup = run_benchmark_warmup()?;
    let mut recorded_runs = Vec::with_capacity(3);
    for _ in 0..3 {
        recorded_runs.push(run_recorded_action_path(warmup, timer_metadata())?);
    }
    validate_recorded_determinism(&recorded_runs)?;
    Ok(ActionPathSuiteReport {
        schema_version: 1,
        benchmark: "pm_action_path",
        warmup_runs: 1,
        recorded_runs,
        production_order_entry_authorized: false,
    })
}

fn run_recorded_action_path(
    warmup: BenchmarkWarmup,
    timer: TimerMetadata,
) -> Result<ActionPathReport, PmEvidenceError> {
    validate_timer_metadata(timer)?;
    let mut outcome = run_benchmark_workload(warmup)?;
    let mut parser = run_parser_segment()?;
    parser.matches_owner_corpus =
        parser.projection_sha256 == outcome.owner_public_projection_sha256;
    if !parser.matches_owner_corpus {
        return Err(PmEvidenceError::invariant(format!(
            "parser projection {} differs from owner-fed projection {}",
            parser.projection_sha256, outcome.owner_public_projection_sha256
        )));
    }
    if outcome.capacities.reserved_capacity_bytes > MAX_RESERVED_CAPACITY_BYTES {
        return Err(PmEvidenceError::invariant(format!(
            "reserved capacity {} exceeds {}",
            outcome.capacities.reserved_capacity_bytes, MAX_RESERVED_CAPACITY_BYTES
        )));
    }
    let seconds = outcome.total_elapsed_ns as f64 / 1_000_000_000_f64;
    let external_observations_per_second = MEASURED_EXTERNAL_OBSERVATIONS as f64 / seconds;
    let owner_reductions_per_second = MEASURED_OWNER_REDUCTIONS as f64 / seconds;
    let action_latency_ns = LatencySummary::from_samples(&mut outcome.action_latencies_ns);
    if action_latency_ns.samples != 15_000
        || action_latency_ns.p50 > 25_000
        || action_latency_ns.p99_9 > 250_000
    {
        return Err(PmEvidenceError::invariant(format!(
            "recorded latency contract failed: {action_latency_ns:?}"
        )));
    }
    Ok(ActionPathReport {
        schema_version: 1,
        benchmark: "pm_action_path",
        fixture_revision: FIXTURE_REVISION,
        build_revision: command_output("git", &["rev-parse", "HEAD"])?,
        rustc: command_output("rustc", &["--version"])?,
        host: command_output("uname", &["-a"])?,
        timer,
        boundary: BoundaryMetadata {
            starts_at: "immutable normalized event delivery to the PM coordinator",
            ends_at: "durable acknowledgement consumed and prepared fake effect enqueued",
            includes: INCLUDED_BOUNDARY,
            excludes: EXCLUDED_BOUNDARY,
            parser_is_separate: true,
            sealed_ack_is_benchmark_only: true,
        },
        capacities: outcome.capacities,
        warmup_setup: outcome.warmup.setup,
        recorded_setup: outcome.recorded_setup,
        warmup_input_mix: outcome.warmup.input_mix,
        measured_input_mix: outcome.measured_input_mix,
        warmup: outcome.warmup.counters,
        measured: outcome.measured,
        repeated_passes: std::mem::take(&mut outcome.repeated_passes),
        action_latency_ns,
        parser,
        total_elapsed_ns: outcome.total_elapsed_ns,
        external_observations_per_second,
        owner_reductions_per_second,
        owner_allocations: outcome.owner_allocations,
        production_order_entry_authorized: false,
    })
}

fn validate_recorded_determinism(reports: &[ActionPathReport]) -> Result<(), PmEvidenceError> {
    if reports.len() != 3 {
        return Err(PmEvidenceError::invariant(format!(
            "recorded report list contains {} runs, expected 3",
            reports.len()
        )));
    }
    let Some(first) = reports.first() else {
        return Err(PmEvidenceError::invariant("recorded report list is empty"));
    };
    if first.production_order_entry_authorized {
        return Err(PmEvidenceError::invariant(
            "action-path evidence cannot authorize production order entry",
        ));
    }
    if first.owner_allocations.allocation_calls != 0
        || first.owner_allocations.allocated_bytes != 0
        || first.owner_allocations.live_bytes_delta > 0
    {
        return Err(PmEvidenceError::invariant(
            "first recorded invocation violated the zero-allocation, non-growing owner boundary",
        ));
    }
    for report in reports.iter().skip(1) {
        if report.fixture_revision != first.fixture_revision
            || report.build_revision != first.build_revision
            || report.rustc != first.rustc
            || report.host != first.host
            || report.timer.source != first.timer.source
            || report.timer.reads != first.timer.reads
            || report.boundary != first.boundary
            || report.capacities != first.capacities
            || report.warmup != first.warmup
            || report.measured != first.measured
            || report.warmup_setup != first.warmup_setup
            || report.recorded_setup != first.recorded_setup
            || report.warmup_input_mix != first.warmup_input_mix
            || report.measured_input_mix != first.measured_input_mix
            || report.parser.warmup_pm_frames != first.parser.warmup_pm_frames
            || report.parser.warmup_okx_frames != first.parser.warmup_okx_frames
            || report.parser.measured_pm_frames != first.parser.measured_pm_frames
            || report.parser.measured_okx_frames != first.parser.measured_okx_frames
            || report.parser.warmup_pm_bytes != first.parser.warmup_pm_bytes
            || report.parser.warmup_okx_bytes != first.parser.warmup_okx_bytes
            || report.parser.measured_pm_bytes != first.parser.measured_pm_bytes
            || report.parser.measured_okx_bytes != first.parser.measured_okx_bytes
            || report.parser.fixture_sha256 != first.parser.fixture_sha256
            || report.parser.projection_sha256 != first.parser.projection_sha256
            || report.parser.matches_owner_corpus != first.parser.matches_owner_corpus
            || report.production_order_entry_authorized
        {
            return Err(PmEvidenceError::invariant(
                "fresh recorded invocation changed frozen metadata, counters, capacities, or corpus",
            ));
        }
        if report.owner_allocations != first.owner_allocations
            || report.owner_allocations.allocation_calls != 0
            || report.owner_allocations.allocated_bytes != 0
            || report.owner_allocations.live_bytes_delta > 0
        {
            return Err(PmEvidenceError::invariant(
                "fresh recorded invocation changed or violated zero-allocation, non-growing owner evidence",
            ));
        }
        let equal = report
            .repeated_passes
            .iter()
            .zip(&first.repeated_passes)
            .all(|(actual, expected)| {
                actual.journal_hash == expected.journal_hash
                    && actual.logical_hash == expected.logical_hash
                    && actual.input_mix == expected.input_mix
                    && actual.counters == expected.counters
                    && actual.terminal_state_lengths == expected.terminal_state_lengths
            });
        if report.repeated_passes.len() != first.repeated_passes.len() || !equal {
            return Err(PmEvidenceError::invariant(
                "fresh recorded invocations changed journal or normalized owner projections",
            ));
        }
    }
    Ok(())
}

pub(crate) async fn run_combined_replay(
    journal_path: PathBuf,
) -> Result<CombinedReplayReport, PmEvidenceError> {
    if journal_path.exists() {
        return Err(PmEvidenceError::invariant(
            "combined replay requires a fresh journal path",
        ));
    }
    let outcome = run_real_writer_workload(journal_path.clone()).await?;
    let bytes = std::fs::read(&journal_path)
        .map_err(|error| PmEvidenceError::invariant(error.to_string()))?;
    let artifact_lines = bytes.iter().filter(|byte| **byte == b'\n').count() as u64;
    if artifact_lines != PHYSICAL_JOURNAL_LINES {
        return Err(PmEvidenceError::invariant(format!(
            "real artifact has {artifact_lines} lines, expected {PHYSICAL_JOURNAL_LINES}"
        )));
    }
    let scope = super::workload::journal_scope()?;
    validate_real_journal_prefix(&bytes, &scope)?;
    let first_recovery = measured_recovery(&journal_path, &scope)?;
    let second_recovery = measured_recovery(&journal_path, &scope)?;
    for (label, recovery) in [("first", &first_recovery), ("second", &second_recovery)] {
        if recovery.record_count != PHYSICAL_JOURNAL_LINES as usize
            || recovery.owned_orders != 0
            || recovery.fill_keys != 0
            || recovery.unresolved_orders != 0
            || recovery.safety_halted
            || recovery.requires_reconciliation
            || recovery.last_intent_id != 10_000
            || recovery.compacted_intent_id != 10_000
            || recovery.fill_watermark.is_none()
        {
            return Err(PmEvidenceError::invariant(format!(
                "{label} recovery did not reach the exact terminal ten-cut projection"
            )));
        }
        if recovery.peak_working_bytes > MAX_REPLAY_WORKING_BYTES {
            return Err(PmEvidenceError::invariant(format!(
                "{label} recovery peak working bytes {} exceed {}",
                recovery.peak_working_bytes, MAX_REPLAY_WORKING_BYTES
            )));
        }
    }
    if outcome.counters.watermark_advances != 10 {
        return Err(PmEvidenceError::invariant(format!(
            "combined replay recorded {} watermark cuts, expected 10",
            outcome.counters.watermark_advances
        )));
    }
    if !first_recovery.has_same_logical_projection(&second_recovery) {
        return Err(PmEvidenceError::invariant(
            "two independent recoveries produced different logical projections",
        ));
    }
    Ok(CombinedReplayReport {
        schema_version: 1,
        target: "combined_replay",
        fixture_revision: FIXTURE_REVISION,
        build_revision: command_output("git", &["rev-parse", "HEAD"])?,
        rustc: command_output("rustc", &["--version"])?,
        host: command_output("uname", &["-a"])?,
        replay_working_limit_bytes: MAX_REPLAY_WORKING_BYTES,
        artifact_bytes: bytes.len() as u64,
        artifact_lines,
        artifact_sha256: sha256_hex(&bytes),
        setup: outcome.setup,
        input_mix: outcome.input_mix,
        measured: outcome.counters,
        byte_identical_projection: first_recovery.canonical_sha256
            == second_recovery.canonical_sha256,
        first_recovery,
        second_recovery,
        production_order_entry_authorized: false,
    })
}

fn validate_real_journal_prefix(
    bytes: &[u8],
    expected_scope: &PmJournalScopeV1,
) -> Result<(), PmEvidenceError> {
    let mut lines = bytes.split(|byte| *byte == b'\n');
    let header = decode_prefix_line(lines.next(), "sequence-zero header")?;
    let watermark = decode_prefix_line(lines.next(), "W0 fill watermark")?;
    for (label, sequence, line) in [
        ("sequence-zero header", 0_u64, &header),
        ("W0 fill watermark", 1_u64, &watermark),
    ] {
        if line.scope() != expected_scope.fingerprint() || line.sequence() != sequence {
            return Err(PmEvidenceError::invariant(format!(
                "real journal {label} has the wrong scope or sequence"
            )));
        }
        line.record().validate(expected_scope).map_err(|error| {
            PmEvidenceError::invariant(format!(
                "real journal {label} failed schema validation: {error}"
            ))
        })?;
    }
    if !matches!(header.record(), PmJournalRecordV1::Header(_)) {
        return Err(PmEvidenceError::invariant(
            "real journal first physical record is not the sequence-zero header",
        ));
    }
    match watermark.record() {
        PmJournalRecordV1::FillWatermarkAdvanced(watermark)
            if watermark.cursor.account_scope == expected_scope.account_scope()
                && watermark.cursor.opaque.bytes() == [1; 32] => {}
        _ => {
            return Err(PmEvidenceError::invariant(
                "real journal second physical record is not the exact fixture W0 fill watermark",
            ));
        }
    }
    Ok(())
}

fn decode_prefix_line(
    line: Option<&[u8]>,
    label: &str,
) -> Result<PmJournalLineV1, PmEvidenceError> {
    let line = line.filter(|line| !line.is_empty()).ok_or_else(|| {
        PmEvidenceError::invariant(format!("real journal is missing its {label}"))
    })?;
    serde_json::from_slice(line).map_err(|error| {
        PmEvidenceError::invariant(format!("real journal {label} is not valid JSON: {error}"))
    })
}

fn measured_recovery(
    journal_path: &std::path::Path,
    scope: &crate::journal::PmJournalScopeV1,
) -> Result<RecoveryProjection, PmEvidenceError> {
    let window = reap_benchmark_allocator::start_measurement().map_err(|error| {
        PmEvidenceError::invariant(format!("recovery allocation window failed: {error}"))
    })?;
    let baseline = window.checkpoint().map_err(|error| {
        PmEvidenceError::invariant(format!("recovery allocation baseline failed: {error}"))
    })?;
    let recovery = recover_pm_mutation_journal(journal_path, scope)
        .map_err(|error| PmEvidenceError::invariant(error.to_string()))?;
    let snapshot = window.stop().map_err(|error| {
        PmEvidenceError::invariant(format!("recovery allocation snapshot failed: {error}"))
    })?;
    if snapshot.allocation_calls == 0 {
        return Err(PmEvidenceError::invariant(
            "recovery allocator instrumentation observed zero calls; evidence target did not install TrackingAllocator",
        ));
    }
    let nonnegative_window_baseline = u64::try_from(baseline.live_bytes_delta).unwrap_or_default();
    let allocator_window_peak_live_delta_bytes = snapshot.peak_live_bytes_delta;
    let peak = allocator_window_peak_live_delta_bytes.saturating_sub(nonnegative_window_baseline);
    Ok(RecoveryProjection::from_recovery(
        &recovery,
        baseline.live_bytes_delta,
        allocator_window_peak_live_delta_bytes,
        usize::try_from(peak).unwrap_or(usize::MAX),
    ))
}

fn timer_metadata() -> TimerMetadata {
    const READS: usize = 10_000;
    let mut samples = Vec::with_capacity(READS);
    for _ in 0..READS {
        let started = Instant::now();
        let _ = Instant::now();
        samples.push(u64::try_from(started.elapsed().as_nanos()).unwrap_or(u64::MAX));
    }
    let summary = LatencySummary::from_samples(&mut samples);
    TimerMetadata {
        source: "std::time::Instant",
        reads: READS as u64,
        read_overhead_p50_ns: summary.p50,
        read_overhead_p95_ns: summary.p95,
        read_overhead_p99_ns: summary.p99,
        read_overhead_p99_9_ns: summary.p99_9,
        read_overhead_max_ns: summary.max,
    }
}

fn validate_timer_metadata(timer: TimerMetadata) -> Result<(), PmEvidenceError> {
    if timer.source != "std::time::Instant"
        || timer.reads != 10_000
        || timer.read_overhead_p50_ns > timer.read_overhead_p95_ns
        || timer.read_overhead_p95_ns > timer.read_overhead_p99_ns
        || timer.read_overhead_p99_ns > timer.read_overhead_p99_9_ns
        || timer.read_overhead_p99_9_ns > timer.read_overhead_max_ns
    {
        return Err(PmEvidenceError::invariant(format!(
            "timer-read overhead metadata is incomplete or unordered: {timer:?}"
        )));
    }
    Ok(())
}

fn command_output(program: &str, arguments: &[&str]) -> Result<String, PmEvidenceError> {
    let output = Command::new(program)
        .args(arguments)
        .output()
        .map_err(|error| {
            PmEvidenceError::invariant(format!(
                "required evidence identity command {program:?} failed to start: {error}"
            ))
        })?;
    if !output.status.success() {
        return Err(PmEvidenceError::invariant(format!(
            "required evidence identity command {program:?} exited with {}",
            output.status
        )));
    }
    let output = String::from_utf8(output.stdout).map_err(|error| {
        PmEvidenceError::invariant(format!(
            "required evidence identity command {program:?} returned non-UTF-8 output: {error}"
        ))
    })?;
    let output = output.trim();
    if output.is_empty() {
        return Err(PmEvidenceError::invariant(format!(
            "required evidence identity command {program:?} returned empty output"
        )));
    }
    Ok(output.to_string())
}
