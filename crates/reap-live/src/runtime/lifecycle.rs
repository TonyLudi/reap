use reap_core::PINNED_JAVA_REVISION;

use super::composition::{
    RunLoopFailure, RunLoopOutcome, combine_lifecycle_errors, live_failure_evidence,
    qualifies_as_clean_soak,
};
use super::{
    LIVE_RUN_REPORT_SCHEMA_VERSION, LiveFailureEvidence, LiveRunReport, LiveRuntime,
    LiveRuntimeError, LiveStopReason, unix_time_ms,
};

impl LiveRuntime {
    pub(super) async fn close_after_error(
        &mut self,
        primary: LiveRuntimeError,
        context: &str,
    ) -> LiveRuntimeError {
        let readiness_at_stop = self.coordinator.readiness();
        let elapsed_ms = unix_time_ms().saturating_sub(self.composition.session_started_at_ms);
        let stop_result = self.graceful_stop(context).await;
        let shutdown_result = self.shutdown().await;
        self.emit_final_health_snapshot();
        let mut additional = Vec::new();
        if let Err(error) = stop_result {
            additional.push(("fail-closed cleanup", error));
        }
        if let Err(error) = shutdown_result {
            additional.push(("runtime teardown", error));
        }
        let error = combine_lifecycle_errors(primary, additional);
        let reached_ready = readiness_at_stop.is_ready();
        let outcome = RunLoopOutcome {
            stop_reason: LiveStopReason::RuntimeFailure,
            elapsed_ms,
            reached_ready,
            time_to_ready_ms: reached_ready.then_some(elapsed_ms),
            readiness_loss_count: 0,
            max_readiness_outage_ms: 0,
            readiness_at_stop,
        };
        let report = self.build_report(outcome, Some(live_failure_evidence(&error)));
        LiveRuntimeError::ReportedFailure {
            source: Box::new(error),
            report: Box::new(report),
        }
    }

    pub(super) async fn run(mut self) -> Result<LiveRunReport, LiveRuntimeError> {
        let loop_result = self.run_loop().await;
        let runtime_alert_result = match &loop_result {
            Ok(_) => Ok(()),
            Err(failure) => self.emit_runtime_failure_alert(&failure.error),
        };
        let stop_context = match &loop_result {
            Ok(outcome) => match outcome.stop_reason {
                LiveStopReason::OperatorSignal => "operator signal".to_string(),
                LiveStopReason::OperatorCommand => self
                    .dispatch
                    .operator_shutdown_reason
                    .clone()
                    .unwrap_or_else(|| "authenticated operator command".to_string()),
                LiveStopReason::DurationElapsed => "bounded duration elapsed".to_string(),
                LiveStopReason::ReadinessTimeout => "bounded readiness timeout".to_string(),
                LiveStopReason::Validation => "validation".to_string(),
                LiveStopReason::RuntimeFailure => "runtime failure".to_string(),
            },
            Err(failure) => format!("runtime failure: {}", failure.error),
        };
        let stop_result = self.graceful_stop(&stop_context).await;
        let shutdown_result = self.shutdown().await;
        self.emit_final_health_snapshot();
        let (mut outcome, failure) = match loop_result {
            Ok(outcome) => {
                let failure = match stop_result {
                    Ok(()) => shutdown_result.err(),
                    Err(primary) => {
                        let additional = shutdown_result
                            .err()
                            .map(|error| vec![("runtime teardown", error)])
                            .unwrap_or_default();
                        Some(combine_lifecycle_errors(primary, additional))
                    }
                };
                (outcome, failure)
            }
            Err(RunLoopFailure {
                error: primary,
                outcome,
            }) => {
                let mut additional = Vec::new();
                if let Err(error) = runtime_alert_result {
                    additional.push(("runtime alert enqueue", error));
                }
                if let Err(error) = stop_result {
                    additional.push(("fail-closed cleanup", error));
                }
                if let Err(error) = shutdown_result {
                    additional.push(("runtime teardown", error));
                }
                (outcome, Some(combine_lifecycle_errors(primary, additional)))
            }
        };
        if failure.is_some() {
            outcome.stop_reason = LiveStopReason::RuntimeFailure;
        }
        let failure_evidence = failure.as_ref().map(live_failure_evidence);
        let report = self.build_report(outcome, failure_evidence);
        match failure {
            Some(source) => Err(LiveRuntimeError::ReportedFailure {
                source: Box::new(source),
                report: Box::new(report),
            }),
            None => Ok(report),
        }
    }

    fn build_report(
        &self,
        outcome: RunLoopOutcome,
        failure: Option<LiveFailureEvidence>,
    ) -> LiveRunReport {
        let readiness = self.coordinator.readiness();
        let dropped_storage_records = self.composition.storage_sink.dropped_records();
        let active_orders_after_shutdown = self.coordinator.active_order_count();
        let evidence = self.composition.evidence;
        let clean_soak = qualifies_as_clean_soak(
            &outcome,
            evidence,
            dropped_storage_records,
            active_orders_after_shutdown,
            self.dispatch.alert_stats.failed,
        );
        LiveRunReport {
            schema_version: LIVE_RUN_REPORT_SCHEMA_VERSION,
            session_id: Some(self.composition.session_id.clone()),
            session_started_at_ms: self.composition.session_started_at_ms,
            config_source: self.composition.config_source.clone(),
            config_fingerprint: self.composition.config_fingerprint.clone(),
            evidence_config_fingerprint: self.composition.evidence_config_fingerprint.clone(),
            java_reference_revision: PINNED_JAVA_REVISION.to_string(),
            reap_version: env!("CARGO_PKG_VERSION").to_string(),
            executable_sha256: self.composition.executable_sha256.clone(),
            host_identity_sha256: self.composition.host_identity_sha256.clone(),
            account_identity_sha256s: self.composition.account_identity_sha256s.clone(),
            mode: self.composition.mode,
            stop_reason: outcome.stop_reason,
            failure,
            elapsed_ms: outcome.elapsed_ms,
            reached_ready: outcome.reached_ready,
            time_to_ready_ms: outcome.time_to_ready_ms,
            readiness_loss_count: outcome.readiness_loss_count,
            max_readiness_outage_ms: outcome.max_readiness_outage_ms,
            reconciliation_drift_events: evidence.reconciliation_drift_events,
            book_recovery_events: evidence.book_recovery_events,
            stream_stale_events: evidence.stream_stale_events,
            connection_disconnect_events: evidence.connection_disconnect_events,
            public_connection_disconnect_events: evidence.public_connection_disconnect_events,
            private_connection_disconnect_events: evidence.private_connection_disconnect_events,
            order_transport_disconnect_events: evidence.order_transport_disconnect_events,
            order_transport_stale_events: evidence.order_transport_stale_events,
            ambiguous_submit_events: evidence.ambiguous_submit_events,
            ambiguous_cancel_events: evidence.ambiguous_cancel_events,
            partial_fill_events: evidence.partial_fill_events,
            fill_convergence_timeout_events: evidence.fill_convergence_timeout_events,
            order_convergence_timeout_events: evidence.order_convergence_timeout_events,
            restored_safety_latches: evidence.restored_safety_latches,
            operator_commands: evidence.operator_commands,
            operator_mutations: evidence.operator_mutations,
            max_storage_queue_depth: evidence.max_storage_queue_depth,
            alerts_delivered: self.dispatch.alert_stats.delivered,
            alert_delivery_failures: self.dispatch.alert_stats.failed,
            alert_failure_notifications_dropped: self
                .dispatch
                .alert_stats
                .failure_notifications_dropped,
            max_alert_queue_depth: self.dispatch.alert_stats.max_queue_depth,
            host_preflight: self.readiness_safety.host_preflight.clone(),
            host_checks: self.readiness_safety.host_checks,
            host_last_snapshot: self.readiness_safety.host_last_snapshot.clone(),
            readiness_at_stop: outcome.readiness_at_stop,
            readiness,
            dropped_storage_records,
            active_orders_after_shutdown,
            latency_evidence: self.composition.latency.report(),
            clean_soak,
        }
    }
}
