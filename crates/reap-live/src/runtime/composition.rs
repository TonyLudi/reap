use std::collections::BTreeMap;
use std::time::Duration;

use reap_storage::{OrderAckStatus, OrderOperation, StorageRecord, StorageRuntime, StorageSink};

use crate::safety_contracts::LiveCleanSoakInputs;

use super::{
    LiveConfigFileEvidence, LiveFailureEvidence, LiveLatencyCollector, LiveMode, LiveRuntimeError,
    LiveStopReason, MAX_LIVE_FAILURE_MESSAGE_BYTES, OrderStatus, ReadinessSnapshot,
    SystemEventKind,
};

pub(super) struct CompositionState {
    pub(super) session_id: String,
    pub(super) session_started_at_ms: u64,
    pub(super) config_source: Option<LiveConfigFileEvidence>,
    pub(super) config_fingerprint: String,
    pub(super) evidence_config_fingerprint: String,
    pub(super) executable_sha256: String,
    pub(super) host_identity_sha256: Option<String>,
    pub(super) account_identity_sha256s: BTreeMap<String, String>,
    pub(super) mode: LiveMode,
    pub(super) run_duration: Option<Duration>,
    pub(super) storage: Option<StorageRuntime>,
    pub(super) storage_sink: StorageSink,
    pub(super) evidence: RuntimeEvidence,
    pub(super) latency: LiveLatencyCollector,
}

#[derive(Debug, Clone, Copy, Default)]
pub(super) struct RuntimeEvidence {
    pub(super) reconciliation_drift_events: u64,
    pub(super) book_recovery_events: u64,
    pub(super) stream_stale_events: u64,
    pub(super) connection_disconnect_events: u64,
    pub(super) public_connection_disconnect_events: u64,
    pub(super) private_connection_disconnect_events: u64,
    pub(super) order_transport_disconnect_events: u64,
    pub(super) order_transport_stale_events: u64,
    pub(super) ambiguous_submit_events: u64,
    pub(super) ambiguous_cancel_events: u64,
    pub(super) partial_fill_events: u64,
    pub(super) fill_convergence_timeout_events: u64,
    pub(super) order_convergence_timeout_events: u64,
    pub(super) restored_safety_latches: u64,
    pub(super) operator_commands: u64,
    pub(super) operator_mutations: u64,
    pub(super) max_storage_queue_depth: usize,
}

impl RuntimeEvidence {
    pub(super) fn begin_live_session(&mut self, restored_safety_latches: u64) {
        *self = Self {
            restored_safety_latches,
            max_storage_queue_depth: self.max_storage_queue_depth,
            ..Self::default()
        };
    }

    pub(super) fn observe_disconnect(&mut self, private: bool) {
        self.connection_disconnect_events = self.connection_disconnect_events.saturating_add(1);
        if private {
            self.private_connection_disconnect_events =
                self.private_connection_disconnect_events.saturating_add(1);
        } else {
            self.public_connection_disconnect_events =
                self.public_connection_disconnect_events.saturating_add(1);
        }
    }

    pub(super) fn observe_order_transport_disconnect(&mut self) {
        self.connection_disconnect_events = self.connection_disconnect_events.saturating_add(1);
        self.order_transport_disconnect_events =
            self.order_transport_disconnect_events.saturating_add(1);
    }

    pub(super) fn observe_record(&mut self, record: &StorageRecord) {
        match record {
            StorageRecord::System(event) => match event.kind {
                SystemEventKind::ReconcileDrift => {
                    self.reconciliation_drift_events =
                        self.reconciliation_drift_events.saturating_add(1);
                }
                SystemEventKind::BookRecoveryStarted => {
                    self.book_recovery_events = self.book_recovery_events.saturating_add(1);
                }
                SystemEventKind::FeedStale | SystemEventKind::PrivateStreamStale => {
                    self.stream_stale_events = self.stream_stale_events.saturating_add(1);
                }
                SystemEventKind::OrderTransportStale => {
                    self.order_transport_stale_events =
                        self.order_transport_stale_events.saturating_add(1);
                }
                _ => {}
            },
            StorageRecord::OrderAck(ack) if matches!(ack.status, OrderAckStatus::Ambiguous) => {
                match ack.operation {
                    OrderOperation::Submit => {
                        self.ambiguous_submit_events =
                            self.ambiguous_submit_events.saturating_add(1);
                    }
                    OrderOperation::Cancel => {
                        self.ambiguous_cancel_events =
                            self.ambiguous_cancel_events.saturating_add(1);
                    }
                }
            }
            StorageRecord::Order { update, .. }
                if update.status == OrderStatus::PartiallyFilled =>
            {
                self.partial_fill_events = self.partial_fill_events.saturating_add(1);
            }
            _ => {}
        }
    }

    pub(super) fn observe_fill_convergence_timeout(&mut self) {
        self.fill_convergence_timeout_events =
            self.fill_convergence_timeout_events.saturating_add(1);
    }

    pub(super) fn observe_order_convergence_timeout(&mut self) {
        self.order_convergence_timeout_events =
            self.order_convergence_timeout_events.saturating_add(1);
    }
}

#[derive(Debug)]
pub(super) struct RunLoopOutcome {
    pub(super) stop_reason: LiveStopReason,
    pub(super) elapsed_ms: u64,
    pub(super) reached_ready: bool,
    pub(super) time_to_ready_ms: Option<u64>,
    pub(super) readiness_loss_count: u64,
    pub(super) max_readiness_outage_ms: u64,
    pub(super) readiness_at_stop: ReadinessSnapshot,
}

#[derive(Debug)]
pub(super) struct RunLoopFailure {
    pub(super) error: LiveRuntimeError,
    pub(super) outcome: RunLoopOutcome,
}

#[derive(Debug, Clone, Default)]
pub(super) struct ReadinessTracker {
    pub(super) reached_ready: bool,
    time_to_ready_ms: Option<u64>,
    readiness_loss_count: u64,
    outage_started_ms: Option<u64>,
    max_readiness_outage_ms: u64,
}

impl ReadinessTracker {
    pub(super) fn observe(&mut self, elapsed_ms: u64, readiness: &ReadinessSnapshot) {
        if readiness.is_ready() {
            if !self.reached_ready {
                self.reached_ready = true;
                self.time_to_ready_ms = Some(elapsed_ms);
            }
            if let Some(started_ms) = self.outage_started_ms.take() {
                self.max_readiness_outage_ms = self
                    .max_readiness_outage_ms
                    .max(elapsed_ms.saturating_sub(started_ms));
            }
        } else if self.reached_ready && self.outage_started_ms.is_none() {
            self.readiness_loss_count += 1;
            self.outage_started_ms = Some(elapsed_ms);
        }
    }

    pub(super) fn finish(
        &self,
        stop_reason: LiveStopReason,
        elapsed_ms: u64,
        readiness: ReadinessSnapshot,
    ) -> RunLoopOutcome {
        let mut tracker = self.clone();
        tracker.observe(elapsed_ms, &readiness);
        if let Some(started_ms) = tracker.outage_started_ms {
            tracker.max_readiness_outage_ms = tracker
                .max_readiness_outage_ms
                .max(elapsed_ms.saturating_sub(started_ms));
        }
        RunLoopOutcome {
            stop_reason,
            elapsed_ms,
            reached_ready: tracker.reached_ready,
            time_to_ready_ms: tracker.time_to_ready_ms,
            readiness_loss_count: tracker.readiness_loss_count,
            max_readiness_outage_ms: tracker.max_readiness_outage_ms,
            readiness_at_stop: readiness,
        }
    }
}

pub(super) fn qualifies_as_clean_soak(
    outcome: &RunLoopOutcome,
    evidence: RuntimeEvidence,
    dropped_storage_records: u64,
    active_orders_after_shutdown: usize,
    alert_delivery_failures: u64,
) -> bool {
    LiveCleanSoakInputs {
        duration_elapsed: outcome.stop_reason == LiveStopReason::DurationElapsed,
        reached_ready: outcome.reached_ready,
        readiness_at_stop_ready: outcome.readiness_at_stop.is_ready(),
        reconciliation_drift_free: evidence.reconciliation_drift_events == 0,
        operator_mutation_free: evidence.operator_mutations == 0,
        storage_records_complete: dropped_storage_records == 0,
        no_active_orders_after_shutdown: active_orders_after_shutdown == 0,
        alert_delivery_failure_free: alert_delivery_failures == 0,
    }
    .qualifies_as_clean_soak()
}

pub(super) fn combine_lifecycle_errors(
    primary: LiveRuntimeError,
    additional: Vec<(&'static str, LiveRuntimeError)>,
) -> LiveRuntimeError {
    if additional.is_empty() {
        return primary;
    }
    let secondary = additional
        .into_iter()
        .map(|(stage, error)| format!("{stage}: {error}"))
        .collect::<Vec<_>>()
        .join("; ");
    LiveRuntimeError::LifecycleFailure {
        primary: Box::new(primary),
        secondary,
    }
}

pub(super) fn live_failure_evidence(error: &LiveRuntimeError) -> LiveFailureEvidence {
    LiveFailureEvidence {
        code: error.stable_code().to_string(),
        message: truncate_utf8(error.to_string(), MAX_LIVE_FAILURE_MESSAGE_BYTES),
    }
}

pub(super) fn live_startup_failure_evidence(error: &LiveRuntimeError) -> LiveFailureEvidence {
    LiveFailureEvidence {
        code: error.stable_code().to_string(),
        message: truncate_utf8(
            format!(
                "startup failed before a reportable runtime session was established; zero-valued runtime counters are not exchange-zero proof: {error}"
            ),
            MAX_LIVE_FAILURE_MESSAGE_BYTES,
        ),
    }
}

pub(super) fn truncate_utf8(mut value: String, maximum_bytes: usize) -> String {
    if value.len() <= maximum_bytes {
        return value;
    }
    let mut boundary = maximum_bytes;
    while !value.is_char_boundary(boundary) {
        boundary -= 1;
    }
    value.truncate(boundary);
    value
}
