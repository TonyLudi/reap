use serde::Serialize;
use sha2::{Digest, Sha256};

use crate::journal::PmJournalRecovery;

#[derive(Debug, Clone, Serialize)]
pub(crate) struct ActionPathReport {
    pub schema_version: u16,
    pub benchmark: &'static str,
    pub fixture_revision: &'static str,
    pub build_revision: String,
    pub rustc: String,
    pub host: String,
    pub timer: TimerMetadata,
    pub boundary: BoundaryMetadata,
    pub capacities: CapacityReport,
    pub warmup_setup: SetupCounters,
    pub recorded_setup: SetupCounters,
    pub warmup_input_mix: InputMixReport,
    pub measured_input_mix: InputMixReport,
    pub warmup: NominalCounters,
    pub measured: NominalCounters,
    pub repeated_passes: Vec<PassProjection>,
    pub action_latency_ns: LatencySummary,
    pub parser: ParserReport,
    pub total_elapsed_ns: u128,
    pub external_observations_per_second: f64,
    pub owner_reductions_per_second: f64,
    pub owner_allocations: AllocationReport,
    pub production_order_entry_authorized: bool,
}

#[derive(Debug, Clone, Serialize)]
pub(crate) struct ActionPathSuiteReport {
    pub schema_version: u16,
    pub benchmark: &'static str,
    pub warmup_runs: usize,
    pub recorded_runs: Vec<ActionPathReport>,
    pub production_order_entry_authorized: bool,
}

#[derive(Debug, Clone, Serialize)]
pub(crate) struct CombinedReplayReport {
    pub schema_version: u16,
    pub target: &'static str,
    pub fixture_revision: &'static str,
    pub build_revision: String,
    pub rustc: String,
    pub host: String,
    pub replay_working_limit_bytes: usize,
    pub artifact_bytes: u64,
    pub artifact_lines: u64,
    pub artifact_sha256: String,
    pub setup: SetupCounters,
    pub input_mix: InputMixReport,
    pub measured: NominalCounters,
    pub first_recovery: RecoveryProjection,
    pub second_recovery: RecoveryProjection,
    pub byte_identical_projection: bool,
    pub production_order_entry_authorized: bool,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
pub(crate) struct TimerMetadata {
    pub source: &'static str,
    pub reads: u64,
    pub read_overhead_p50_ns: u64,
    pub read_overhead_p95_ns: u64,
    pub read_overhead_p99_ns: u64,
    pub read_overhead_p99_9_ns: u64,
    pub read_overhead_max_ns: u64,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
pub(crate) struct BoundaryMetadata {
    pub starts_at: &'static str,
    pub ends_at: &'static str,
    pub includes: &'static [&'static str],
    pub excludes: &'static [&'static str],
    pub parser_is_separate: bool,
    pub sealed_ack_is_benchmark_only: bool,
}

#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize)]
pub(crate) struct CapacityReport {
    pub reserved_capacity_bytes: usize,
    pub reserved_capacity_limit_bytes: usize,
    pub replay_working_limit_bytes: usize,
    pub persistence_capacity: usize,
    pub persistence_depth: usize,
    pub persistence_nominal_high_water: usize,
    pub fake_effect_capacity: usize,
    pub schedule_capacity: usize,
    pub schedule_nominal_high_water: usize,
    pub schedule_depth: usize,
    pub schedule_high_water: usize,
    pub schedule_admitted: u64,
    pub schedule_duplicate_suppressed: u64,
    pub schedule_rescheduled: u64,
    pub schedule_removed: u64,
    pub schedule_serviced: u64,
    pub schedule_rejected_full: u64,
    pub schedule_clock_regressions: u64,
    pub schedule_current_due_age_ns: u64,
    pub schedule_maximum_due_age_ns: u64,
    pub schedule_maximum_permitted_due_age_ns: u64,
    pub schedule_fail_closed: bool,
    pub copied_correlation_capacity: usize,
    pub copied_correlation_high_water: usize,
    pub copied_output_capacity: usize,
    pub copied_output_depth: usize,
    pub copied_output_high_water: usize,
    pub copied_output_rejected_full: u64,
    pub copied_output_age_faults: u64,
    pub copied_output_saturation_action: &'static str,
    pub persistence_high_water: usize,
    pub persistence_maximum_age_limit_ns: Option<u64>,
    pub persistence_maximum_age_ns: u64,
    pub persistence_admitted: u64,
    pub persistence_acknowledged: u64,
    pub persistence_saturations: u64,
    pub persistence_age_faults: u64,
    pub persistence_globally_stopped: bool,
    pub persistence_saturation_action: &'static str,
    pub fake_effect_depth: usize,
    pub fake_effect_nominal_high_water: usize,
    pub fake_effect_high_water: usize,
    pub fake_effect_maximum_age_limit_ns: Option<u64>,
    pub fake_effect_maximum_age_ns: u64,
    pub fake_effect_queued: usize,
    pub fake_effect_blocked: usize,
    pub fake_effect_retained: usize,
    pub fake_effect_serviced: u64,
    pub fake_effect_saturations: u64,
    pub fake_effect_age_faults: u64,
    pub fake_effect_clock_regressions: u64,
    pub fake_effect_saturation_action: &'static str,
    pub refresh: RefreshPressureReport,
    pub raw_entry_capacity: usize,
    pub raw_entry_high_water: usize,
    pub raw_entry_rejections: u64,
    pub raw_payload_byte_capacity: usize,
    pub raw_payload_byte_high_water: usize,
    pub raw_payload_rejections: u64,
    pub raw_oversize_rejections: u64,
    pub raw_maximum_age_limit_ns: Option<u64>,
    pub raw_age_faults: u64,
    pub raw_saturation_action: &'static str,
    pub lanes: Vec<LanePressureReport>,
}

#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize)]
pub(crate) struct LanePressureReport {
    pub lane: &'static str,
    pub depth: usize,
    pub high_water: usize,
    pub capacity: usize,
    pub nominal_high_water: usize,
    pub maximum_age_limit_ns: Option<u64>,
    pub maximum_observed_age_ns: u64,
    pub saturation_action: &'static str,
    pub serviced: u64,
    pub age_faults: u64,
    pub rejected_full: u64,
    pub coalesced: u64,
    pub invalidated_purged: u64,
}

#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize)]
pub(crate) struct RefreshPressureReport {
    pub capacity: usize,
    pub total_pending: usize,
    pub total_in_flight: usize,
    pub ambiguous_order_pending: usize,
    pub ambiguous_order_in_flight: usize,
    pub fill_observed_pending: usize,
    pub fill_observed_in_flight: usize,
    pub fill_observed_high_water: usize,
    pub external_ingress_pending: usize,
    pub external_ingress_in_flight: usize,
    pub external_ingress_high_water: usize,
    pub oldest_in_flight_age_ns: u64,
    pub maximum_observed_age_ns: u64,
    pub maximum_age_limit_ns: Option<u64>,
    pub retry_effects: u64,
    pub duplicate_or_superseded_admissions: u64,
    pub saturation_action: &'static str,
}

#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize)]
pub(crate) struct BootstrapInputMix {
    pub private_connection_completion: u64,
    pub open_orders_snapshot: u64,
    pub initial_market_metadata: u64,
    pub initial_pm_book_snapshot: u64,
}

#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize)]
pub(crate) struct SetupCounters {
    pub bootstrap: BootstrapInputMix,
    pub journal_header_records: u64,
    pub w0_paired_reconciliations: u64,
    pub w0_external_observations: u64,
    pub w0_internal_fact_acknowledgements: u64,
    pub w0_owner_reductions: u64,
    pub w0_journal_records: u64,
    pub w0_watermark_advances: u64,
    pub physical_journal_lines: Option<u64>,
}

#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize)]
pub(crate) struct InputMixReport {
    pub pm_book_observations: u64,
    pub okx_reference_observations: u64,
    pub quote_evaluation_timers: u64,
    pub quote_intent_acknowledgements: u64,
    pub fake_place_acceptances: u64,
    pub private_unique_fills: u64,
    pub private_duplicate_fills: u64,
    pub fill_order_detail_absences: u64,
    pub paired_reconciliations: u64,
    pub fill_freshness_timers: u64,
    pub replace_timers: u64,
    pub cancel_intent_acknowledgements: u64,
    pub fake_cancel_acceptances: u64,
    pub cancel_order_detail_absences: u64,
    pub cancel_freshness_timers: u64,
}

impl InputMixReport {
    pub(crate) const fn total(self) -> u64 {
        self.pm_book_observations
            .saturating_add(self.okx_reference_observations)
            .saturating_add(self.quote_evaluation_timers)
            .saturating_add(self.quote_intent_acknowledgements)
            .saturating_add(self.fake_place_acceptances)
            .saturating_add(self.private_unique_fills)
            .saturating_add(self.private_duplicate_fills)
            .saturating_add(self.fill_order_detail_absences)
            .saturating_add(self.paired_reconciliations)
            .saturating_add(self.fill_freshness_timers)
            .saturating_add(self.replace_timers)
            .saturating_add(self.cancel_intent_acknowledgements)
            .saturating_add(self.fake_cancel_acceptances)
            .saturating_add(self.cancel_order_detail_absences)
            .saturating_add(self.cancel_freshness_timers)
    }
}

#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize)]
pub(crate) struct NominalCounters {
    pub external_observations: u64,
    pub internal_fact_acknowledgements: u64,
    pub owner_reductions: u64,
    pub journal_records: u64,
    pub quote_evaluations: u64,
    pub quote_candidates_evaluated: u64,
    pub quote_intents: u64,
    pub place_results: u64,
    pub prepared_quote_projections: u64,
    pub executed_quote_projections: u64,
    pub cancel_decisions: u64,
    pub cancel_intents: u64,
    pub cancel_results: u64,
    pub prepared_cancel_projections: u64,
    pub executed_cancel_projections: u64,
    pub unique_fills: u64,
    pub duplicate_fills: u64,
    pub filled_orders: u64,
    pub cancelled_orders: u64,
    pub paired_reconciliations: u64,
    pub watermark_advances: u64,
    pub owned_lifecycle_rows_compacted: u64,
    pub canonical_order_rows_compacted: u64,
    pub owned_fill_keys_compacted: u64,
    pub canonical_fill_rows_compacted: u64,
    pub refresh_tickets_inserted: u64,
    pub refresh_tickets_admitted: u64,
    pub refresh_effects: u64,
    pub refresh_tickets_completed: u64,
    pub refresh_ticket_high_water: usize,
    pub refresh_duplicate_or_superseded: u64,
    pub queue_saturations: u64,
    pub state_bearing_drops: u64,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub(crate) struct PassProjection {
    pub ordinal: usize,
    pub input_mix: InputMixReport,
    pub counters: NominalCounters,
    pub journal_record_delta: u64,
    pub journal_hash: String,
    pub logical_hash: String,
    pub reserved_capacity_bytes: usize,
    pub terminal_state_lengths_zero: bool,
    pub terminal_state_lengths: TerminalStateLengths,
    pub allocator_live_bytes: i64,
}

#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize)]
pub(crate) struct TerminalStateLengths {
    pub critical_lane: usize,
    pub persistence_lane: usize,
    pub private_lane: usize,
    pub scheduled_lane: usize,
    pub public_lane: usize,
    pub reconciliation_lane: usize,
    pub telemetry_lane: usize,
    pub pending_persistence: usize,
    pub pending_fake_effects: usize,
    pub retained_fake_effect_permits: usize,
    pub pending_durable_consequences: usize,
    pub reconciliation_reductions: usize,
    pub canonical_orders: usize,
    pub owned_orders: usize,
    pub owned_quote_slots: usize,
    pub canonical_fills: usize,
    pub owned_fill_keys: usize,
    pub unresolved_fills: usize,
    pub canonical_refresh: usize,
    pub fill_compaction_pending: usize,
    pub pending_correlations: usize,
    pub prepared_correlations: usize,
    pub tracked_quote_slots: usize,
    pub pending_schedules: usize,
    pub copied_outputs: usize,
    pub retained_admissions: usize,
    pub refresh_obligations: usize,
}

impl From<crate::coordinator::PmEvidenceTerminalLengths> for TerminalStateLengths {
    fn from(value: crate::coordinator::PmEvidenceTerminalLengths) -> Self {
        Self {
            critical_lane: value.critical_lane,
            persistence_lane: value.persistence_lane,
            private_lane: value.private_lane,
            scheduled_lane: value.scheduled_lane,
            public_lane: value.public_lane,
            reconciliation_lane: value.reconciliation_lane,
            telemetry_lane: value.telemetry_lane,
            pending_persistence: value.pending_persistence,
            pending_fake_effects: value.pending_fake_effects,
            retained_fake_effect_permits: value.retained_fake_effect_permits,
            pending_durable_consequences: value.pending_durable_consequences,
            reconciliation_reductions: value.reconciliation_reductions,
            canonical_orders: value.canonical_orders,
            owned_orders: value.owned_orders,
            owned_quote_slots: value.owned_quote_slots,
            canonical_fills: value.canonical_fills,
            owned_fill_keys: value.owned_fill_keys,
            unresolved_fills: value.unresolved_fills,
            canonical_refresh: value.canonical_refresh,
            fill_compaction_pending: value.fill_compaction_pending,
            pending_correlations: value.pending_correlations,
            prepared_correlations: value.prepared_correlations,
            tracked_quote_slots: value.tracked_quote_slots,
            pending_schedules: value.pending_schedules,
            copied_outputs: value.copied_outputs,
            retained_admissions: value.retained_admissions,
            refresh_obligations: value.refresh_obligations,
        }
    }
}

#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize)]
pub(crate) struct AllocationReport {
    pub allocation_calls: u64,
    pub allocated_bytes: u64,
    pub deallocation_calls: u64,
    pub deallocated_bytes: u64,
    pub live_bytes_delta: i64,
    pub peak_live_bytes_delta: u64,
}

impl From<reap_benchmark_allocator::AllocationSnapshot> for AllocationReport {
    fn from(snapshot: reap_benchmark_allocator::AllocationSnapshot) -> Self {
        Self {
            allocation_calls: snapshot.allocation_calls,
            allocated_bytes: snapshot.allocated_bytes,
            deallocation_calls: snapshot.deallocation_calls,
            deallocated_bytes: snapshot.deallocated_bytes,
            live_bytes_delta: snapshot.live_bytes_delta,
            peak_live_bytes_delta: snapshot.peak_live_bytes_delta,
        }
    }
}

#[derive(Debug, Clone, Copy, Default, Serialize)]
pub(crate) struct LatencySummary {
    pub samples: usize,
    pub p50: u64,
    pub p95: u64,
    pub p99: u64,
    pub p99_9: u64,
    pub max: u64,
}

impl LatencySummary {
    pub(crate) fn from_samples(samples: &mut [u64]) -> Self {
        if samples.is_empty() {
            return Self::default();
        }
        samples.sort_unstable();
        Self {
            samples: samples.len(),
            p50: nearest_rank(samples, 500, 1_000),
            p95: nearest_rank(samples, 950, 1_000),
            p99: nearest_rank(samples, 990, 1_000),
            p99_9: nearest_rank(samples, 999, 1_000),
            max: *samples.last().expect("nonempty latency samples"),
        }
    }
}

fn nearest_rank(samples: &[u64], numerator: usize, denominator: usize) -> u64 {
    let rank = samples
        .len()
        .saturating_mul(numerator)
        .saturating_add(denominator - 1)
        / denominator;
    samples[rank.saturating_sub(1).min(samples.len() - 1)]
}

#[derive(Debug, Clone, Serialize)]
pub(crate) struct ParserReport {
    pub warmup_pm_frames: usize,
    pub warmup_okx_frames: usize,
    pub warmup_pm_bytes: usize,
    pub warmup_okx_bytes: usize,
    pub measured_pm_frames: usize,
    pub measured_okx_frames: usize,
    pub measured_pm_bytes: usize,
    pub measured_okx_bytes: usize,
    pub pm_latency_ns: LatencySummary,
    pub okx_latency_ns: LatencySummary,
    pub allocation: AllocationReport,
    pub fixture_sha256: String,
    pub projection_sha256: String,
    pub matches_owner_corpus: bool,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub(crate) struct RecoveryProjection {
    pub record_count: usize,
    pub terminal_working_bytes: usize,
    pub allocator_window_baseline_live_delta_bytes: i64,
    pub allocator_window_peak_live_delta_bytes: u64,
    pub peak_working_bytes: usize,
    pub allocator_measurement: &'static str,
    pub last_sequence: u64,
    pub last_intent_id: u64,
    pub last_owned_observation_sequence: u64,
    pub compacted_intent_id: u64,
    pub owned_orders: usize,
    pub fill_keys: usize,
    pub unresolved_orders: usize,
    pub safety_halted: bool,
    pub requires_reconciliation: bool,
    pub fill_watermark: Option<String>,
    pub canonical_sha256: String,
}

impl RecoveryProjection {
    pub(crate) fn from_recovery(
        recovery: &PmJournalRecovery,
        allocator_window_baseline_live_delta_bytes: i64,
        allocator_window_peak_live_delta_bytes: u64,
        allocator_peak_working_bytes: usize,
    ) -> Self {
        let fill_watermark = recovery
            .fill_watermark()
            .map(|cursor| hex(cursor.opaque.bytes()));
        let canonical = (
            recovery.record_count(),
            recovery.last_sequence(),
            recovery.last_intent_id(),
            recovery.last_owned_observation_sequence(),
            recovery.compacted_intent_id(),
            recovery.owned_order_count(),
            recovery.fill_key_count(),
            recovery.unresolved_order_count(),
            recovery.safety_halted(),
            recovery.requires_reconciliation(),
            fill_watermark.as_deref(),
        );
        let bytes = serde_json::to_vec(&canonical).expect("recovery projection serializes");
        Self {
            record_count: recovery.record_count(),
            terminal_working_bytes: recovery.reserved_capacity_bytes(),
            allocator_window_baseline_live_delta_bytes,
            allocator_window_peak_live_delta_bytes,
            peak_working_bytes: allocator_peak_working_bytes,
            allocator_measurement: "recovery-window peak live delta minus the post-input-construction window baseline",
            last_sequence: recovery.last_sequence(),
            last_intent_id: recovery.last_intent_id(),
            last_owned_observation_sequence: recovery.last_owned_observation_sequence(),
            compacted_intent_id: recovery.compacted_intent_id(),
            owned_orders: recovery.owned_order_count(),
            fill_keys: recovery.fill_key_count(),
            unresolved_orders: recovery.unresolved_order_count(),
            safety_halted: recovery.safety_halted(),
            requires_reconciliation: recovery.requires_reconciliation(),
            fill_watermark,
            canonical_sha256: sha256_hex(&bytes),
        }
    }

    pub(crate) fn has_same_logical_projection(&self, other: &Self) -> bool {
        self.record_count == other.record_count
            && self.last_sequence == other.last_sequence
            && self.last_intent_id == other.last_intent_id
            && self.last_owned_observation_sequence == other.last_owned_observation_sequence
            && self.compacted_intent_id == other.compacted_intent_id
            && self.owned_orders == other.owned_orders
            && self.fill_keys == other.fill_keys
            && self.unresolved_orders == other.unresolved_orders
            && self.safety_halted == other.safety_halted
            && self.requires_reconciliation == other.requires_reconciliation
            && self.fill_watermark == other.fill_watermark
            && self.canonical_sha256 == other.canonical_sha256
    }
}

pub(crate) fn sha256_hex(bytes: &[u8]) -> String {
    let digest = Sha256::digest(bytes);
    hex(digest.into())
}

pub(crate) fn hex<const N: usize>(bytes: [u8; N]) -> String {
    let mut output = String::with_capacity(N * 2);
    for byte in bytes {
        use std::fmt::Write as _;
        write!(&mut output, "{byte:02x}").expect("writing to String cannot fail");
    }
    output
}
