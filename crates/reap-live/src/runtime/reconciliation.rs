use std::collections::{HashMap, HashSet};
use std::time::{Duration, Instant};

use reap_core::{AccountUpdate, BacktestLatencyClass, NormalizedEvent, OrderStatus};
use reap_feed::FeedOutput;
use reap_order::{OkxReconciliationClient, reconcile_full_state};
use reap_storage::StorageRecord;
use reap_venue::{PrivateOrderState, RemoteFill, RemoteOrder};
use tokio::sync::{mpsc, oneshot};
use tokio::task::JoinHandle;

use crate::coordinator::ReconcileAction;
use crate::{CoordinatorError, CoordinatorOutput, LiveLatencySemantics, ReconciliationResult};

use super::dispatch::{OrderTaskCommand, ReconcileOrderRef, ReconcileTaskCommand, RuntimeEvent};
use super::recovery::{private_update_from_remote, remote_order_id};
use super::shutdown::is_zero_order_reconciliation;
use super::{
    FillConvergenceGuard, LiveRuntime, LiveRuntimeError, OrderStateConvergenceGuard, unix_time_ms,
    unix_time_ns,
};

pub(super) struct ReconciliationState {
    pub(super) senders: HashMap<String, mpsc::Sender<ReconcileTaskCommand>>,
    pub(super) tasks: Vec<JoinHandle<()>>,
    pub(super) inflight: HashSet<String>,
    pub(super) cancel_inflight: HashSet<(String, String)>,
    pub(super) last_attempt: HashMap<String, Instant>,
    pub(super) fill_convergence: FillConvergenceGuard,
    pub(super) order_convergence: OrderStateConvergenceGuard,
}

impl LiveRuntime {
    pub(super) fn observe_account_convergence(
        &mut self,
        account_id: &str,
        output: &CoordinatorOutput,
        observed_ns: u64,
    ) {
        for record in &output.records {
            if let StorageRecord::Normalized(NormalizedEvent::Account(update)) = record {
                let result = self.reconciliation.fill_convergence.observe_account_at(
                    account_id,
                    update,
                    observed_ns / 1_000_000,
                    observed_ns,
                );
                for observation in result.observations {
                    self.composition.latency.observe_ns(
                        BacktestLatencyClass::OrderFill,
                        &observation.symbol,
                        LiveLatencySemantics::FillToAccountStateVisibility,
                        observation.first_observed_ns,
                        observation.state_visible_ns,
                    );
                }
            }
        }
    }

    pub(super) async fn apply_remote_recovery(
        &mut self,
        account_id: &str,
        remote_orders: &[RemoteOrder],
        remote_fills: &[RemoteFill],
    ) -> Result<(), LiveRuntimeError> {
        for fill in remote_fills {
            let should_apply = self
                .coordinator
                .private_state(account_id)
                .is_some_and(|state| {
                    let order_id =
                        state.resolve_order_id(&fill.client_order_id, &fill.exchange_order_id);
                    state.order_reducer().contains_order(&order_id)
                        && !state.has_seen_fill(&fill.symbol, &fill.fill_id)
                });
            if should_apply {
                let output = self.coordinator.process_feed(FeedOutput::PrivateFill {
                    account_id: Some(account_id.to_string()),
                    fill: fill.clone(),
                })?;
                self.observe_fill_convergence(&output, unix_time_ns(), false);
                self.commit_output(output).await?;
            }
        }
        for remote in remote_orders {
            let known = self
                .coordinator
                .private_state(account_id)
                .is_some_and(|state| {
                    let order_id =
                        state.resolve_order_id(&remote.client_order_id, &remote.exchange_order_id);
                    state.order_reducer().contains_order(&order_id)
                });
            if known {
                let output = self.coordinator.process_feed(FeedOutput::PrivateOrder {
                    account_id: Some(account_id.to_string()),
                    update: private_update_from_remote(remote.clone()),
                })?;
                self.observe_fill_convergence(&output, unix_time_ns(), false);
                self.commit_output(output).await?;
            }
        }
        Ok(())
    }

    pub(super) fn observe_fill_convergence(
        &mut self,
        output: &CoordinatorOutput,
        observed_ns: u64,
        collect_latency: bool,
    ) {
        for record in &output.records {
            if let StorageRecord::Order {
                account_id: Some(account_id),
                update,
            } = record
            {
                let result = self.reconciliation.fill_convergence.observe_fill_at(
                    account_id,
                    update,
                    observed_ns / 1_000_000,
                    observed_ns,
                    collect_latency,
                );
                if collect_latency {
                    if result.dropped_latency_observation {
                        self.composition.latency.observe_dropped_observation();
                    }
                    for observation in result.observations {
                        self.composition.latency.observe_ns(
                            BacktestLatencyClass::OrderFill,
                            &observation.symbol,
                            LiveLatencySemantics::FillToAccountStateVisibility,
                            observation.first_observed_ns,
                            observation.state_visible_ns,
                        );
                    }
                }
            }
        }
    }

    pub(super) fn observe_order_convergence(
        &mut self,
        output: &CoordinatorOutput,
        observed_ms: u64,
    ) {
        for record in &output.records {
            if let StorageRecord::Order {
                account_id: Some(account_id),
                update,
            } = record
            {
                self.reconciliation.order_convergence.observe_order(
                    account_id,
                    update,
                    observed_ms,
                );
                if matches!(
                    update.status,
                    OrderStatus::Filled | OrderStatus::Cancelled | OrderStatus::Rejected
                ) {
                    self.reconciliation
                        .cancel_inflight
                        .remove(&(account_id.clone(), update.order_id.clone()));
                }
            }
        }
    }

    pub(super) fn dispatch_reconcile(
        &mut self,
        action: ReconcileAction,
    ) -> Result<(), LiveRuntimeError> {
        tracing::debug!(
            account_id = action.account_id,
            requested_at_ms = action.ts_ms,
            reason = action.reason,
            "dispatching reconciliation"
        );
        if !self
            .reconciliation
            .inflight
            .insert(action.account_id.clone())
        {
            return Ok(());
        }
        self.reconciliation
            .last_attempt
            .insert(action.account_id.clone(), Instant::now());
        let orders = match self.reconciliation_order_refs(&action.account_id) {
            Ok(orders) => orders,
            Err(error) => {
                self.reconciliation.inflight.remove(&action.account_id);
                return Err(error);
            }
        };
        let sender = match self.reconcile_sender(&action.account_id) {
            Ok(sender) => sender.clone(),
            Err(error) => {
                self.reconciliation.inflight.remove(&action.account_id);
                return Err(error);
            }
        };
        sender
            .try_send(ReconcileTaskCommand::Reconcile {
                restored_orders: orders,
                command_flush: None,
            })
            .map_err(|_| {
                self.reconciliation.inflight.remove(&action.account_id);
                LiveRuntimeError::OrderQueueUnavailable(action.account_id)
            })
    }

    pub(super) async fn dispatch_shutdown_reconcile(
        &mut self,
        action: ReconcileAction,
    ) -> Result<(), LiveRuntimeError> {
        tracing::debug!(
            account_id = action.account_id,
            requested_at_ms = action.ts_ms,
            reason = action.reason,
            "dispatching shutdown reconciliation"
        );
        if !self
            .reconciliation
            .inflight
            .insert(action.account_id.clone())
        {
            return Ok(());
        }
        self.reconciliation
            .last_attempt
            .insert(action.account_id.clone(), Instant::now());
        let order_sender = match self.order_sender(&action.account_id) {
            Ok(sender) => sender.clone(),
            Err(error) => {
                self.reconciliation.inflight.remove(&action.account_id);
                return Err(error);
            }
        };
        let (flushed_tx, flushed_rx) = oneshot::channel();
        if order_sender
            .send(OrderTaskCommand::Flush(flushed_tx))
            .await
            .is_err()
        {
            self.reconciliation.inflight.remove(&action.account_id);
            return Err(LiveRuntimeError::OrderQueueUnavailable(action.account_id));
        }
        let orders = match self.reconciliation_order_refs(&action.account_id) {
            Ok(orders) => orders,
            Err(error) => {
                self.reconciliation.inflight.remove(&action.account_id);
                return Err(error);
            }
        };
        let sender = match self.reconcile_sender(&action.account_id) {
            Ok(sender) => sender.clone(),
            Err(error) => {
                self.reconciliation.inflight.remove(&action.account_id);
                return Err(error);
            }
        };
        if sender
            .send(ReconcileTaskCommand::Reconcile {
                restored_orders: orders,
                command_flush: Some(flushed_rx),
            })
            .await
            .is_err()
        {
            self.reconciliation.inflight.remove(&action.account_id);
            return Err(LiveRuntimeError::OrderQueueUnavailable(action.account_id));
        }
        Ok(())
    }

    pub(super) fn reconciliation_order_refs(
        &self,
        account_id: &str,
    ) -> Result<Vec<ReconcileOrderRef>, LiveRuntimeError> {
        let state = self
            .coordinator
            .private_state(account_id)
            .ok_or_else(|| CoordinatorError::UnknownAccount(account_id.to_string()))?;
        Ok(state
            .order_reducer()
            .orders()
            .filter(|(_, order)| {
                matches!(
                    order.status,
                    OrderStatus::PendingNew | OrderStatus::Live | OrderStatus::PartiallyFilled
                )
            })
            .map(|(order_id, order)| ReconcileOrderRef {
                order_id: order_id.to_string(),
                symbol: order.symbol.clone(),
                side: order.side,
                price: order.price,
                qty: order.qty,
                filled_qty: order.filled_qty,
                average_fill_price: order.avg_fill_price,
                last_update_ms: state.last_order_update_ms(order_id).unwrap_or(0),
            })
            .collect())
    }

    pub(super) async fn request_shutdown_reconciliation(
        &mut self,
        ts_ms: u64,
        force: bool,
    ) -> Result<(), LiveRuntimeError> {
        let accounts = self
            .dispatch
            .order_senders
            .keys()
            .filter(|account_id| !self.shutdown.reconciled_accounts.contains(*account_id))
            .filter(|account_id| !self.reconciliation.inflight.contains(*account_id))
            .filter(|account_id| {
                !self.shutdown.reconciliation_requested.contains(*account_id)
                    || force
                    || self
                        .reconciliation
                        .last_attempt
                        .get(*account_id)
                        .is_none_or(|last| last.elapsed() >= Duration::from_secs(2))
            })
            .cloned()
            .collect::<Vec<_>>();
        for account_id in accounts {
            self.dispatch_shutdown_reconcile(ReconcileAction {
                ts_ms,
                account_id: account_id.clone(),
                reason: "verify zero exchange orders during graceful shutdown".to_string(),
            })
            .await?;
            self.shutdown.reconciliation_requested.insert(account_id);
        }
        Ok(())
    }

    pub(super) fn retry_reconciliation(&mut self, ts_ms: u64) -> Result<(), LiveRuntimeError> {
        let readiness = self.coordinator.readiness();
        let accounts = readiness
            .missing_reconciliation
            .into_iter()
            .filter(|account_id| !self.reconciliation.inflight.contains(account_id))
            .filter(|account_id| {
                self.reconciliation
                    .last_attempt
                    .get(account_id)
                    .is_none_or(|last| last.elapsed() >= Duration::from_secs(2))
            })
            .collect::<Vec<_>>();
        for account_id in accounts {
            self.dispatch_reconcile(ReconcileAction {
                ts_ms,
                account_id,
                reason: "retry degraded reconciliation".to_string(),
            })?;
        }
        Ok(())
    }

    pub(super) fn reconcile_sender(
        &self,
        account_id: &str,
    ) -> Result<&mpsc::Sender<ReconcileTaskCommand>, LiveRuntimeError> {
        self.reconciliation
            .senders
            .get(account_id)
            .ok_or_else(|| LiveRuntimeError::OrderQueueUnavailable(account_id.to_string()))
    }
}

pub(super) async fn handle_runtime_event(
    runtime: &mut LiveRuntime,
    event: RuntimeEvent,
) -> Result<(), LiveRuntimeError> {
    match event {
        RuntimeEvent::RemoteState {
            account_id,
            remote_orders,
            remote_fills,
            remote_account,
            ts_ms,
        } => {
            runtime.reconciliation.inflight.remove(&account_id);
            runtime
                .apply_remote_recovery(&account_id, &remote_orders, &remote_fills)
                .await?;
            let order_convergence = &runtime.reconciliation.order_convergence;
            runtime
                .reconciliation
                .cancel_inflight
                .retain(|(cancel_account, order_id)| {
                    cancel_account != &account_id
                        || order_convergence.has_pending_cancel(cancel_account, order_id)
                });
            let report = {
                let state = runtime
                    .coordinator
                    .private_state(&account_id)
                    .ok_or_else(|| CoordinatorError::UnknownAccount(account_id.clone()))?;
                reconcile_full_state(state, &remote_orders, &remote_fills, &remote_account)
            };
            let remote_account_ts_ms = remote_account.ts_ms;
            let account_output = runtime
                .coordinator
                .apply_authoritative_account_snapshot(&account_id, remote_account)?;
            let censored_fill_latencies = runtime
                .reconciliation
                .fill_convergence
                .observe_authoritative(&account_id, remote_account_ts_ms);
            runtime
                .composition
                .latency
                .observe_dropped_observations(censored_fill_latencies as u64);
            runtime.commit_output(account_output).await?;
            let pending_order_state = runtime
                .reconciliation
                .order_convergence
                .pending_reason(&account_id);
            let clean = report.is_clean() && pending_order_state.is_none();
            if runtime.shutdown.in_progress
                && runtime
                    .shutdown
                    .reconciliation_requested
                    .contains(&account_id)
            {
                if is_zero_order_reconciliation(&report) {
                    runtime
                        .shutdown
                        .reconciled_accounts
                        .insert(account_id.clone());
                } else {
                    runtime.shutdown.reconciled_accounts.remove(&account_id);
                }
            }
            let reason = if clean {
                "REST orders, fills, balances, positions, and canonical private state agree"
                    .to_string()
            } else if report.is_clean() {
                pending_order_state
                    .expect("non-clean order convergence must include a pending reason")
            } else {
                let mut reason = format!("{:?}", report.issues);
                if let Some(pending) = pending_order_state {
                    reason.push_str("; ");
                    reason.push_str(&pending);
                }
                reason
            };
            let output = runtime
                .coordinator
                .on_reconciliation(ReconciliationResult {
                    account_id,
                    ts_ms,
                    clean,
                    local_live_orders: report.local_live_orders,
                    remote_live_orders: report.remote_live_orders,
                    remote_recent_fills: report.remote_fills,
                    reason,
                })?;
            runtime.commit_output(output).await?;
        }
        RuntimeEvent::ReconcileFailed {
            account_id,
            ts_ms,
            reason,
        } => {
            runtime.reconciliation.inflight.remove(&account_id);
            runtime.shutdown.reconciled_accounts.remove(&account_id);
            let output = runtime
                .coordinator
                .on_reconciliation(ReconciliationResult {
                    account_id,
                    ts_ms,
                    clean: false,
                    local_live_orders: 0,
                    remote_live_orders: 0,
                    remote_recent_fills: 0,
                    reason: format!("REST reconciliation request failed: {reason}"),
                })?;
            runtime.commit_output(output).await?;
        }
        _ => unreachable!("non-reconciliation event sent to reconciliation handler"),
    }
    Ok(())
}

pub(super) async fn run_reconcile_task(
    account_id: String,
    io: OkxReconciliationClient,
    mut commands: mpsc::Receiver<ReconcileTaskCommand>,
    events: mpsc::Sender<RuntimeEvent>,
    ambiguous_submit_grace_ms: u64,
    max_order_reconciliation_pages: usize,
    max_fill_reconciliation_pages: usize,
) {
    while let Some(command) = commands.recv().await {
        let ReconcileTaskCommand::Reconcile {
            restored_orders,
            command_flush,
        } = command
        else {
            return;
        };
        if let Some(command_flush) = command_flush
            && command_flush.await.is_err()
        {
            if events
                .send(RuntimeEvent::ReconcileFailed {
                    account_id: account_id.clone(),
                    ts_ms: unix_time_ms(),
                    reason: "order command task closed before its shutdown flush".to_string(),
                })
                .await
                .is_err()
            {
                return;
            }
            continue;
        }
        let result = reconcile_remote_account(
            &io,
            restored_orders,
            ambiguous_submit_grace_ms,
            max_order_reconciliation_pages,
            max_fill_reconciliation_pages,
        )
        .await;
        let event = match result {
            Ok((remote_orders, remote_fills, remote_account)) => RuntimeEvent::RemoteState {
                account_id: account_id.clone(),
                remote_orders,
                remote_fills,
                remote_account,
                ts_ms: unix_time_ms(),
            },
            Err(reason) => RuntimeEvent::ReconcileFailed {
                account_id: account_id.clone(),
                ts_ms: unix_time_ms(),
                reason,
            },
        };
        if events.send(event).await.is_err() {
            return;
        }
    }
}

async fn reconcile_remote_account(
    io: &OkxReconciliationClient,
    restored_orders: Vec<ReconcileOrderRef>,
    ambiguous_submit_grace_ms: u64,
    max_order_reconciliation_pages: usize,
    max_fill_reconciliation_pages: usize,
) -> Result<(Vec<RemoteOrder>, Vec<RemoteFill>, AccountUpdate), String> {
    let (mut remote_orders, remote_fills) = io
        .fetch_remote_state(
            None,
            None,
            max_order_reconciliation_pages,
            max_fill_reconciliation_pages,
        )
        .await
        .map_err(|error| error.to_string())?;
    let mut remote_ids = remote_orders
        .iter()
        .map(remote_order_id)
        .collect::<HashSet<_>>();
    for restored in restored_orders {
        if remote_ids.contains(&restored.order_id) {
            continue;
        }
        let details = match io
            .fetch_order_details(&restored.symbol, &restored.order_id)
            .await
        {
            Ok(details) => details,
            Err(error)
                if error.is_order_not_found()
                    && unix_time_ms().saturating_sub(restored.last_update_ms)
                        < ambiguous_submit_grace_ms =>
            {
                return Err(format!(
                    "order {} is not visible within the ambiguous-submit grace period",
                    restored.order_id
                ));
            }
            Err(error) if error.is_order_not_found() => RemoteOrder {
                exchange_order_id: String::new(),
                client_order_id: restored.order_id.clone(),
                symbol: restored.symbol,
                side: restored.side,
                state: PrivateOrderState::Rejected,
                price: restored.price,
                qty: restored.qty,
                cumulative_filled_qty: restored.filled_qty,
                average_fill_price: restored.average_fill_price,
                update_time_ms: unix_time_ms(),
            },
            Err(error) => return Err(error.to_string()),
        };
        remote_ids.insert(remote_order_id(&details));
        remote_orders.push(details);
    }
    let remote_account = io
        .fetch_remote_account_state()
        .await
        .map_err(|error| error.to_string())?;
    Ok((remote_orders, remote_fills, remote_account))
}
