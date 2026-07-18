use std::collections::HashSet;
use std::time::{Duration, Instant};

use reap_core::{NormalizedEvent, SystemEvent, SystemEventKind};
use reap_order::ReconcileReport;
use reap_storage::{OrderOperation, OrderRequestRecord, StorageRecord};
use reap_telemetry::{AlertEvent, AlertSeverity};
use tokio::sync::{mpsc, oneshot};
use tokio::task::JoinHandle;

use crate::coordinator::{CancelAction, LiveAction};
use crate::{CoordinatorOutput, LiveMode};

use super::composition::combine_lifecycle_errors;
use super::dispatch::{OrderTaskCommand, ReconcileTaskCommand, SafetyTaskCommand};
use super::{LiveRuntime, LiveRuntimeError, unix_time_ms};

pub(super) struct ShutdownState {
    pub(super) timeout_ms: u64,
    pub(super) teardown_timeout_ms: u64,
    pub(super) safety_latch_sync_timeout_ms: u64,
    pub(super) in_progress: bool,
    pub(super) storage_error: Option<String>,
    pub(super) preserve_deadman: bool,
    pub(super) reconciliation_requested: HashSet<String>,
    pub(super) reconciled_accounts: HashSet<String>,
}

pub(super) fn is_zero_order_reconciliation(report: &ReconcileReport) -> bool {
    report.is_clean() && report.local_live_orders == 0 && report.remote_live_orders == 0
}

/// Owns spawned tasks until the runtime has adopted them. Dropping a partially
/// constructed runtime cannot orphan a task that still has exchange authority.
#[derive(Default)]
pub(super) struct StartupTaskGroup(Vec<JoinHandle<()>>);

impl StartupTaskGroup {
    pub(super) fn take(&mut self) -> Vec<JoinHandle<()>> {
        std::mem::take(&mut self.0)
    }
}

impl std::ops::Deref for StartupTaskGroup {
    type Target = Vec<JoinHandle<()>>;

    fn deref(&self) -> &Self::Target {
        &self.0
    }
}

impl std::ops::DerefMut for StartupTaskGroup {
    fn deref_mut(&mut self) -> &mut Self::Target {
        &mut self.0
    }
}

impl Drop for StartupTaskGroup {
    fn drop(&mut self) {
        for task in &self.0 {
            task.abort();
        }
    }
}

#[cfg(unix)]
pub(super) async fn shutdown_signal() -> Result<(), std::io::Error> {
    let mut terminate = tokio::signal::unix::signal(tokio::signal::unix::SignalKind::terminate())?;
    tokio::select! {
        result = tokio::signal::ctrl_c() => result,
        _ = terminate.recv() => Ok(()),
    }
}

#[cfg(not(unix))]
pub(super) async fn shutdown_signal() -> Result<(), std::io::Error> {
    tokio::signal::ctrl_c().await
}

impl LiveRuntime {
    pub(super) async fn graceful_stop(&mut self, reason: &str) -> Result<(), LiveRuntimeError> {
        if self.composition.mode != LiveMode::Demo {
            return Ok(());
        }
        self.shutdown.in_progress = true;
        let result = match tokio::time::timeout(
            Duration::from_millis(self.shutdown.timeout_ms),
            self.graceful_stop_inner(reason),
        )
        .await
        {
            Ok(result) => result,
            Err(_) => Err(self.shutdown_unresolved_error()),
        };
        match (result, self.shutdown.storage_error.take()) {
            (Ok(()), None) => Ok(()),
            (Ok(()), Some(error)) => Err(LiveRuntimeError::ShutdownStorage(error)),
            (Err(primary), None) => Err(primary),
            (Err(primary), Some(error)) => Err(combine_lifecycle_errors(
                primary,
                vec![("shutdown storage", LiveRuntimeError::ShutdownStorage(error))],
            )),
        }
    }

    async fn graceful_stop_inner(&mut self, reason: &str) -> Result<(), LiveRuntimeError> {
        self.coordinator.set_order_entry_enabled(false);
        let now_ms = unix_time_ms();
        let output = self
            .coordinator
            .process_event(NormalizedEvent::System(SystemEvent {
                ts_ms: now_ms,
                kind: SystemEventKind::KillSwitchActivated,
                venue: None,
                account_id: None,
                symbol: None,
                reason: reason.to_string(),
            }));
        self.commit_shutdown_output(output).await?;
        self.request_shutdown_reconciliation(now_ms, true).await?;

        let mut retry = tokio::time::interval(Duration::from_millis(100));
        retry.set_missed_tick_behavior(tokio::time::MissedTickBehavior::Skip);
        loop {
            self.drain_shutdown_events().await?;
            let readiness = self.coordinator.readiness();
            if self.coordinator.active_order_count() == 0
                && readiness.missing_reconciliation.is_empty()
                && self.reconciliation.inflight.is_empty()
                && self.shutdown.reconciled_accounts.len() == self.dispatch.order_senders.len()
            {
                if !self.shutdown.preserve_deadman {
                    self.disable_deadman_all().await?;
                }
                return Ok(());
            }
            tokio::select! {
                biased;
                event = self.dispatch.control_rx.recv(), if !self.dispatch.control_rx.is_closed() => {
                    if let Some(event) = event {
                        self.handle_runtime_event(event).await?;
                    }
                }
                event = self.readiness_safety.forbidden_rx.recv(), if !self.readiness_safety.forbidden_rx.is_closed() => {
                    if let Some(event) = event {
                        self.handle_forbidden_order_event(event).await?;
                    }
                }
                event = self.connectivity.feed_rx.recv(), if !self.connectivity.feed_rx.is_closed() => {
                    if let Some(event) = event {
                        self.handle_runtime_event(event).await?;
                    }
                }
                _ = retry.tick() => {
                    self.request_shutdown_reconciliation(unix_time_ms(), false).await?;
                }
            }
        }
    }

    fn shutdown_unresolved_error(&self) -> LiveRuntimeError {
        LiveRuntimeError::ShutdownUnresolved {
            active_orders: self.coordinator.active_order_count(),
            unreconciled_accounts: self
                .coordinator
                .readiness()
                .missing_reconciliation
                .into_iter()
                .chain(self.reconciliation.inflight.iter().cloned())
                .chain(
                    self.dispatch
                        .order_senders
                        .keys()
                        .filter(|account_id| {
                            !self.shutdown.reconciled_accounts.contains(*account_id)
                        })
                        .cloned(),
                )
                .collect::<HashSet<_>>()
                .len(),
        }
    }

    async fn drain_shutdown_events(&mut self) -> Result<(), LiveRuntimeError> {
        let pending_control = self.dispatch.control_rx.len();
        for _ in 0..pending_control {
            match self.dispatch.control_rx.try_recv() {
                Ok(event) => self.handle_runtime_event(event).await?,
                Err(mpsc::error::TryRecvError::Empty) => break,
                Err(mpsc::error::TryRecvError::Disconnected) => break,
            }
        }
        let pending_forbidden = self.readiness_safety.forbidden_rx.len();
        for _ in 0..pending_forbidden {
            match self.readiness_safety.forbidden_rx.try_recv() {
                Ok(event) => self.handle_forbidden_order_event(event).await?,
                Err(mpsc::error::TryRecvError::Empty) => break,
                Err(mpsc::error::TryRecvError::Disconnected) => break,
            }
        }
        let pending_feed = self.connectivity.feed_rx.len();
        for _ in 0..pending_feed {
            match self.connectivity.feed_rx.try_recv() {
                Ok(event) => self.handle_runtime_event(event).await?,
                Err(mpsc::error::TryRecvError::Empty) => break,
                Err(mpsc::error::TryRecvError::Disconnected) => break,
            }
        }
        Ok(())
    }

    pub(super) async fn drain_queued_events(&mut self) -> Result<(), LiveRuntimeError> {
        let pending_control = self.dispatch.control_rx.len();
        for _ in 0..pending_control {
            match self.dispatch.control_rx.try_recv() {
                Ok(event) => self.handle_runtime_event(event).await?,
                Err(mpsc::error::TryRecvError::Empty) => break,
                Err(mpsc::error::TryRecvError::Disconnected) => {
                    return Err(LiveRuntimeError::EventChannelClosed);
                }
            }
        }
        let pending_forbidden = self.readiness_safety.forbidden_rx.len();
        for _ in 0..pending_forbidden {
            match self.readiness_safety.forbidden_rx.try_recv() {
                Ok(event) => self.handle_forbidden_order_event(event).await?,
                Err(mpsc::error::TryRecvError::Empty) => break,
                Err(mpsc::error::TryRecvError::Disconnected) => {
                    return Err(LiveRuntimeError::EventChannelClosed);
                }
            }
        }
        let pending_feed = self.connectivity.feed_rx.len();
        for _ in 0..pending_feed {
            match self.connectivity.feed_rx.try_recv() {
                Ok(event) => self.handle_runtime_event(event).await?,
                Err(mpsc::error::TryRecvError::Empty) => break,
                Err(mpsc::error::TryRecvError::Disconnected) => {
                    return Err(LiveRuntimeError::EventChannelClosed);
                }
            }
        }
        Ok(())
    }

    async fn commit_shutdown_output(
        &mut self,
        output: CoordinatorOutput,
    ) -> Result<(), LiveRuntimeError> {
        self.observe_order_convergence(&output, unix_time_ms());
        for record in output.records {
            if matches!(record, StorageRecord::SafetyLatch(_)) {
                self.record_durable_storage(record).await?;
            } else {
                self.record_storage(record)?;
            }
        }
        for action in output.actions {
            match action {
                LiveAction::Submit(_) => return Err(LiveRuntimeError::UnsafeShutdownSubmit),
                LiveAction::Cancel(action) => self.dispatch_shutdown_cancel(action).await?,
                LiveAction::RecoverBook(_) => {}
                LiveAction::Reconcile(action) => {
                    self.dispatch_shutdown_reconcile(action).await?;
                }
            }
        }
        Ok(())
    }

    async fn dispatch_shutdown_cancel(
        &mut self,
        action: CancelAction,
    ) -> Result<(), LiveRuntimeError> {
        tracing::debug!(
            account_id = action.account_id(),
            symbol = action.symbol(),
            client_order_id = action.client_order_id(),
            reason = action.reason(),
            "dispatching approved shutdown cancellation"
        );
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
        if let Err(error) = self.record_storage(StorageRecord::OrderRequest(OrderRequestRecord {
            ts_ms: action.ts_ms(),
            account_id: action.account_id().to_string(),
            operation: OrderOperation::Cancel,
            idempotency_key: None,
            client_order_id: Some(action.client_order_id().to_string()),
            exchange_order_id: None,
            symbol: action.symbol().to_string(),
        })) {
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
            .send(OrderTaskCommand::Cancel {
                action,
                enqueued_at,
            })
            .await
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
        Ok(())
    }

    async fn disable_deadman_all(&mut self) -> Result<(), LiveRuntimeError> {
        let mut acknowledgements = Vec::new();
        for (account_id, sender) in &self.readiness_safety.safety_senders {
            let (result_tx, result_rx) = oneshot::channel();
            sender
                .send(SafetyTaskCommand::DisableDeadMan { result: result_tx })
                .await
                .map_err(|_| {
                    LiveRuntimeError::GatewayTask(format!(
                        "account {account_id} safety task is unavailable"
                    ))
                })?;
            acknowledgements.push((account_id.clone(), result_rx));
        }
        for (account_id, result) in acknowledgements {
            let result = result.await.map_err(|_| {
                LiveRuntimeError::GatewayTask(format!(
                    "account {account_id} safety task dropped disable acknowledgement"
                ))
            })?;
            result.map_err(|message| {
                LiveRuntimeError::GatewayTask(format!(
                    "account {account_id} failed to disable Cancel All After: {message}"
                ))
            })?;
        }
        Ok(())
    }

    pub(super) async fn shutdown(&mut self) -> Result<(), LiveRuntimeError> {
        let timeout_ms = self.shutdown.teardown_timeout_ms;
        match tokio::time::timeout(Duration::from_millis(timeout_ms), self.shutdown_inner()).await {
            Ok(result) => result,
            Err(_) => {
                self.abort_remaining_teardown();
                tokio::task::yield_now().await;
                Err(LiveRuntimeError::TeardownTimeout(timeout_ms))
            }
        }
    }

    async fn shutdown_inner(&mut self) -> Result<(), LiveRuntimeError> {
        let mut errors = Vec::new();
        if let Some(host_guard) = self.readiness_safety.host_guard.as_mut() {
            host_guard.request_shutdown();
        }
        if let Some(service) = self.dispatch.operator_service.as_mut() {
            service.request_shutdown();
        }
        for feed in &self.connectivity.feeds {
            feed.request_shutdown();
        }
        for runtime in &self.connectivity.order_ws_runtimes {
            runtime.request_shutdown();
        }
        for sender in self.dispatch.order_senders.values() {
            let _ = sender.try_send(OrderTaskCommand::Shutdown);
        }
        self.dispatch.order_senders.clear();
        for sender in self.reconciliation.senders.values() {
            let _ = sender.try_send(ReconcileTaskCommand::Shutdown);
        }
        self.reconciliation.senders.clear();
        for sender in self.readiness_safety.safety_senders.values() {
            let _ = sender.try_send(SafetyTaskCommand::Shutdown);
        }
        self.readiness_safety.safety_senders.clear();
        self.dispatch.control_rx.close();
        self.readiness_safety.forbidden_rx.close();
        self.connectivity.feed_rx.close();
        for task in &self.readiness_safety.forbidden_tasks {
            task.abort();
        }
        if let Some(host_guard) = self.readiness_safety.host_guard.take() {
            match host_guard.shutdown().await {
                Ok(stats) => {
                    self.readiness_safety.host_checks = self
                        .readiness_safety
                        .host_checks
                        .saturating_add(stats.checks);
                    if let Some(snapshot) = stats.last_snapshot {
                        self.readiness_safety.host_last_snapshot = Some(snapshot);
                    }
                }
                Err(error) => errors.push(("host guard", LiveRuntimeError::Join(error))),
            }
        }
        if let Some(failures) = &mut self.readiness_safety.host_failures
            && let Ok(error) = failures.try_recv()
        {
            errors.push(("host health", error.into()));
        }
        self.readiness_safety.host_failures.take();
        if let Some(service) = self.dispatch.operator_service.take()
            && let Err(error) = service.shutdown().await
        {
            errors.push(("operator service", LiveRuntimeError::Operator(error)));
        }
        self.dispatch.operator_rx.take();
        for feed in self.connectivity.feeds.drain(..) {
            feed.shutdown().await;
        }
        for task in &mut self.connectivity.feed_tasks {
            if let Err(error) = task.await {
                errors.push(("feed task", LiveRuntimeError::Join(error)));
            }
        }
        self.connectivity.feed_tasks.clear();
        for task in &mut self.dispatch.order_tasks {
            if let Err(error) = task.await {
                errors.push(("order task", LiveRuntimeError::Join(error)));
            }
        }
        self.dispatch.order_tasks.clear();
        for task in &mut self.reconciliation.tasks {
            if let Err(error) = task.await {
                errors.push(("reconciliation task", LiveRuntimeError::Join(error)));
            }
        }
        self.reconciliation.tasks.clear();
        for runtime in self.connectivity.order_ws_runtimes.drain(..) {
            if let Err(error) = runtime.shutdown().await {
                errors.push(("order websocket", LiveRuntimeError::Join(error)));
            }
        }
        for task in &mut self.connectivity.order_ws_status_tasks {
            if let Err(error) = task.await {
                errors.push(("order websocket status", LiveRuntimeError::Join(error)));
            }
        }
        self.connectivity.order_ws_status_tasks.clear();
        for task in &mut self.readiness_safety.safety_tasks {
            if let Err(error) = task.await {
                errors.push(("safety task", LiveRuntimeError::Join(error)));
            }
        }
        self.readiness_safety.safety_tasks.clear();
        for task in &mut self.readiness_safety.forbidden_tasks {
            if let Err(error) = task.await
                && !error.is_cancelled()
            {
                errors.push(("forbidden-order sentinel", LiveRuntimeError::Join(error)));
            }
        }
        self.readiness_safety.forbidden_tasks.clear();
        if let Some(storage) = self.composition.storage.as_mut()
            && let Err(error) = storage.stop_writer().await
        {
            errors.push(("storage", LiveRuntimeError::Storage(error)));
        }
        if !errors.is_empty() {
            let message = errors
                .iter()
                .map(|(stage, error)| format!("{stage}: {error}"))
                .collect::<Vec<_>>()
                .join("; ");
            if let Err(error) = self.emit_alert(AlertEvent::new(
                AlertSeverity::Critical,
                "runtime",
                "teardown_failure",
                message,
            )) {
                errors.push(("teardown alert enqueue", error));
            }
        }
        self.dispatch.alert_sink.take();
        if let Some(alert_runtime) = self.dispatch.alert_runtime.take() {
            match tokio::time::timeout(
                Duration::from_millis(self.dispatch.alert_shutdown_timeout_ms),
                alert_runtime.shutdown(),
            )
            .await
            {
                Ok(Ok(stats)) => {
                    self.dispatch.alert_stats = stats;
                    let unobserved_failures = stats
                        .failed
                        .saturating_sub(self.dispatch.observed_alert_delivery_failures);
                    if self.dispatch.alert_delivery_failure_is_fatal && unobserved_failures > 0 {
                        errors.push((
                            "alert delivery",
                            LiveRuntimeError::AlertFailuresDuringShutdown(unobserved_failures),
                        ));
                    }
                }
                Ok(Err(error)) => errors.push(("alert service", error.into())),
                Err(_) => errors.push((
                    "alert service",
                    LiveRuntimeError::AlertShutdownTimeout(self.dispatch.alert_shutdown_timeout_ms),
                )),
            }
        }
        self.dispatch.alert_failures.take();
        if errors.is_empty() {
            return Ok(());
        }
        let (_, first) = errors.remove(0);
        let additional = errors;
        Err(combine_lifecycle_errors(first, additional))
    }

    fn abort_remaining_teardown(&mut self) {
        self.shutdown.preserve_deadman = true;
        self.dispatch.order_senders.clear();
        self.reconciliation.senders.clear();
        self.readiness_safety.safety_senders.clear();
        self.dispatch.control_rx.close();
        self.readiness_safety.forbidden_rx.close();
        self.connectivity.feed_rx.close();

        for task in self
            .connectivity
            .feed_tasks
            .iter()
            .chain(self.dispatch.order_tasks.iter())
            .chain(self.reconciliation.tasks.iter())
            .chain(self.connectivity.order_ws_status_tasks.iter())
            .chain(self.readiness_safety.safety_tasks.iter())
            .chain(self.readiness_safety.forbidden_tasks.iter())
        {
            task.abort();
        }
        self.connectivity.feed_tasks.clear();
        self.dispatch.order_tasks.clear();
        self.reconciliation.tasks.clear();
        self.connectivity.order_ws_status_tasks.clear();
        self.readiness_safety.safety_tasks.clear();
        self.readiness_safety.forbidden_tasks.clear();

        self.connectivity.feeds.clear();
        self.connectivity.order_ws_runtimes.clear();
        self.dispatch.operator_service.take();
        self.dispatch.operator_rx.take();
        self.readiness_safety.host_guard.take();
        self.readiness_safety.host_failures.take();
        self.composition.storage.take();
        self.dispatch.alert_sink.take();
        self.dispatch.alert_runtime.take();
        self.dispatch.alert_failures.take();
    }
}
