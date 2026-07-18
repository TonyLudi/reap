use std::collections::{HashMap, HashSet};
use std::time::Instant;

use reap_core::AccountUpdate;
use reap_order::{OkxReconciliationClient, reconcile_full_state};
use reap_venue::{PrivateOrderState, RemoteFill, RemoteOrder};
use tokio::sync::mpsc;
use tokio::task::JoinHandle;

use crate::{CoordinatorError, ReconciliationResult};

use super::recovery::remote_order_id;
use super::shutdown::is_zero_order_reconciliation;
use super::{
    FillConvergenceGuard, LiveRuntime, LiveRuntimeError, OrderStateConvergenceGuard,
    ReconcileOrderRef, ReconcileTaskCommand, RuntimeEvent, unix_time_ms,
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
