use std::time::{Duration, Instant};

use reap_core::SystemEventKind;
use reap_storage::{OrderOperation, OrderRequestRecord, StorageRecord};
use reap_telemetry::{AlertDeliveryFailure, AlertEvent, AlertSeverity};
use tokio::sync::mpsc;

use crate::CoordinatorOutput;
use crate::coordinator::LiveAction;

use super::dispatch::OrderTaskCommand;
use super::{LiveRuntime, LiveRuntimeError, unix_time_ms};

pub(super) fn alert_for_storage_record(record: &StorageRecord) -> Option<AlertEvent> {
    let StorageRecord::System(event) = record else {
        return None;
    };
    let (severity, component, code) = match event.kind {
        SystemEventKind::FeedStale => (AlertSeverity::Warning, "market_data", "feed_stale"),
        SystemEventKind::FeedGap => (AlertSeverity::Warning, "market_data", "feed_gap"),
        SystemEventKind::BookRecoveryStarted => (
            AlertSeverity::Warning,
            "market_data",
            "book_recovery_started",
        ),
        SystemEventKind::PrivateStreamStale => (
            AlertSeverity::Warning,
            "account_data",
            "private_stream_stale",
        ),
        SystemEventKind::OrderTransportStale => (
            AlertSeverity::Critical,
            "order_gateway",
            "order_transport_stale",
        ),
        SystemEventKind::BookRecoveryFailed => (
            AlertSeverity::Critical,
            "market_data",
            "book_recovery_failed",
        ),
        SystemEventKind::ReconcileDrift => {
            (AlertSeverity::Critical, "order_state", "reconcile_drift")
        }
        SystemEventKind::RiskBreach => (AlertSeverity::Critical, "risk", "risk_breach"),
        SystemEventKind::KillSwitchActivated => {
            (AlertSeverity::Critical, "risk", "kill_switch_activated")
        }
        SystemEventKind::AccountHalted => (AlertSeverity::Critical, "risk", "account_halted"),
        SystemEventKind::SymbolHalted => (AlertSeverity::Critical, "risk", "symbol_halted"),
        SystemEventKind::FeedHeartbeat
        | SystemEventKind::FeedRecovered
        | SystemEventKind::PrivateStreamHeartbeat
        | SystemEventKind::PrivateStreamRecovered
        | SystemEventKind::OrderTransportHeartbeat
        | SystemEventKind::OrderTransportRecovered
        | SystemEventKind::KillSwitchReset
        | SystemEventKind::SymbolResumed => return None,
    };
    let mut alert = AlertEvent::new(severity, component, code, event.reason.clone());
    alert.ts_ms = event.ts_ms;
    if let Some(venue) = &event.venue {
        alert = alert.with_attribute("venue", format!("{venue:?}").to_ascii_lowercase());
    }
    if let Some(account_id) = &event.account_id {
        alert = alert.with_attribute("account_id", account_id);
    }
    if let Some(symbol) = &event.symbol {
        alert = alert.with_attribute("symbol", symbol);
    }
    Some(alert)
}

impl LiveRuntime {
    pub(super) async fn commit_output(
        &mut self,
        output: CoordinatorOutput,
    ) -> Result<(), LiveRuntimeError> {
        self.observe_order_convergence(&output, unix_time_ms());
        let alerts = if self.dispatch.alert_sink.is_some() {
            output
                .records
                .iter()
                .filter_map(alert_for_storage_record)
                .collect::<Vec<_>>()
        } else {
            Vec::new()
        };
        for record in output.records {
            if matches!(record, StorageRecord::SafetyLatch(_)) {
                self.record_durable_storage(record).await?;
            } else {
                self.record_storage(record)?;
            }
        }
        for alert in alerts {
            self.emit_alert(alert)?;
        }
        for action in output.actions {
            self.dispatch_action(action)?;
        }
        Ok(())
    }

    pub(super) fn emit_alert(&self, alert: AlertEvent) -> Result<(), LiveRuntimeError> {
        if let Some(sink) = &self.dispatch.alert_sink {
            sink.try_emit(alert)?;
        }
        Ok(())
    }

    pub(super) fn emit_runtime_failure_alert(
        &self,
        error: &LiveRuntimeError,
    ) -> Result<(), LiveRuntimeError> {
        if matches!(
            error,
            LiveRuntimeError::Alert(_)
                | LiveRuntimeError::AlertDelivery { .. }
                | LiveRuntimeError::AlertMonitorClosed
                | LiveRuntimeError::AlertShutdownTimeout(_)
                | LiveRuntimeError::AlertFailuresDuringShutdown(_)
        ) {
            return Ok(());
        }
        let mut alert = AlertEvent::new(
            AlertSeverity::Critical,
            "runtime",
            "runtime_failure",
            error.to_string(),
        );
        if let LiveRuntimeError::Host(host_error) = error {
            alert.code = host_error.code().to_string();
            if let Some(snapshot) = host_error.snapshot() {
                alert = alert
                    .with_attribute(
                        "disk_available_bytes",
                        snapshot.disk_available_bytes.to_string(),
                    )
                    .with_attribute(
                        "memory_available_bytes",
                        snapshot.memory_available_bytes.to_string(),
                    )
                    .with_attribute(
                        "clock_synchronized",
                        snapshot.clock_synchronized.to_string(),
                    );
            }
        }
        self.emit_alert(alert)
    }

    pub(super) fn record_storage(&mut self, record: StorageRecord) -> Result<(), LiveRuntimeError> {
        self.composition.evidence.observe_record(&record);
        if let Err(error) = self.composition.storage_sink.try_record(record) {
            if !self.shutdown.in_progress {
                return Err(error.into());
            }
            tracing::error!(%error, "storage unavailable during fail-closed shutdown");
            self.shutdown
                .storage_error
                .get_or_insert_with(|| error.to_string());
            return Ok(());
        }
        self.composition.evidence.max_storage_queue_depth = self
            .composition
            .evidence
            .max_storage_queue_depth
            .max(self.composition.storage_sink.queue_depth());
        Ok(())
    }

    pub(super) async fn record_durable_storage(
        &mut self,
        record: StorageRecord,
    ) -> Result<(), LiveRuntimeError> {
        self.composition.evidence.observe_record(&record);
        self.composition.evidence.max_storage_queue_depth =
            self.composition.evidence.max_storage_queue_depth.max(
                self.composition
                    .storage_sink
                    .queue_depth()
                    .saturating_add(1),
            );
        let result = tokio::time::timeout(
            Duration::from_millis(self.shutdown.safety_latch_sync_timeout_ms),
            self.composition.storage_sink.record_durable(record),
        )
        .await;
        match result {
            Ok(Ok(())) => Ok(()),
            Ok(Err(error)) => {
                self.shutdown.preserve_deadman = true;
                Err(error.into())
            }
            Err(_) => {
                self.shutdown.preserve_deadman = true;
                Err(LiveRuntimeError::SafetyLatchSyncTimeout(
                    self.shutdown.safety_latch_sync_timeout_ms,
                ))
            }
        }
    }

    pub(super) fn dispatch_action(&mut self, action: LiveAction) -> Result<(), LiveRuntimeError> {
        match action {
            LiveAction::Submit(action) => {
                let enqueued_at = Instant::now();
                self.record_storage(StorageRecord::OrderRequest(OrderRequestRecord {
                    ts_ms: action.ts_ms(),
                    account_id: action.account_id().to_string(),
                    operation: OrderOperation::Submit,
                    idempotency_key: Some(action.idempotency_key().to_string()),
                    client_order_id: Some(action.client_order_id().to_string()),
                    exchange_order_id: None,
                    symbol: action.order().symbol.clone(),
                }))?;
                self.order_sender(action.account_id())?
                    .try_send(OrderTaskCommand::Submit {
                        action,
                        enqueued_at,
                    })
                    .map_err(|_| {
                        LiveRuntimeError::OrderQueueUnavailable("submit account queue".to_string())
                    })?;
            }
            LiveAction::Cancel(action) => {
                let enqueued_at = Instant::now();
                let cancel_key = (
                    action.account_id().to_string(),
                    action.client_order_id().to_string(),
                );
                let symbol = action.symbol().to_string();
                if !self
                    .reconciliation
                    .cancel_inflight
                    .insert(cancel_key.clone())
                {
                    return Ok(());
                }
                if let Err(error) =
                    self.record_storage(StorageRecord::OrderRequest(OrderRequestRecord {
                        ts_ms: action.ts_ms(),
                        account_id: action.account_id().to_string(),
                        operation: OrderOperation::Cancel,
                        idempotency_key: None,
                        client_order_id: Some(action.client_order_id().to_string()),
                        exchange_order_id: None,
                        symbol: action.symbol().to_string(),
                    }))
                {
                    self.reconciliation.cancel_inflight.remove(&cancel_key);
                    return Err(error);
                }
                let sender = match self.order_sender(action.account_id()) {
                    Ok(sender) => sender.clone(),
                    Err(error) => {
                        self.reconciliation.cancel_inflight.remove(&cancel_key);
                        return Err(error);
                    }
                };
                if sender
                    .try_send(OrderTaskCommand::Cancel {
                        action,
                        enqueued_at,
                    })
                    .is_err()
                {
                    self.reconciliation.cancel_inflight.remove(&cancel_key);
                    return Err(LiveRuntimeError::OrderQueueUnavailable(cancel_key.0));
                }
                self.reconciliation.order_convergence.observe_cancel(
                    &cancel_key.0,
                    &cancel_key.1,
                    &symbol,
                    unix_time_ms(),
                );
            }
            LiveAction::RecoverBook(request) => {
                let routes = self.connectivity.feeds[self.connectivity.public_feed_index]
                    .request_recovery(&request);
                if routes == 0 {
                    return Err(LiveRuntimeError::MissingRecoveryRoute(
                        request.stream.symbol,
                    ));
                }
            }
            LiveAction::Reconcile(action) => self.dispatch_reconcile(action)?,
        }
        Ok(())
    }
}

pub(super) async fn receive_alert_failure(
    receiver: &mut Option<mpsc::Receiver<AlertDeliveryFailure>>,
) -> Option<AlertDeliveryFailure> {
    match receiver {
        Some(receiver) => receiver.recv().await,
        None => std::future::pending().await,
    }
}
