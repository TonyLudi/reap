use reap_core::{NormalizedEvent, SystemEvent, SystemEventKind, TimeMs, Venue};
use reap_order::{CancelOutcome, SubmitOutcome};
use reap_storage::{
    AccountSnapshotRecord, OrderAckRecord, OrderAckStatus, OrderOperation, ReconciliationRecord,
    StorageRecord,
};

use super::{
    CoordinatorError, CoordinatorOutput, LiveAction, LiveCoordinator, ReconcileAction,
    ReconciliationResult, scope_account_update,
};

impl LiveCoordinator {
    pub fn apply_authoritative_account_snapshot(
        &mut self,
        account_id: &str,
        mut update: reap_core::AccountUpdate,
    ) -> Result<CoordinatorOutput, CoordinatorError> {
        if !self.manages_account(account_id) {
            return Err(CoordinatorError::UnknownAccount(account_id.to_string()));
        }
        self.ensure_account_state_policy(account_id, &update)?;
        scope_account_update(account_id, &mut update);
        let update = self
            .private_state_mut(account_id)?
            .replace_account_snapshot(update);
        let snapshot = AccountSnapshotRecord {
            ts_ms: update.ts_ms,
            account_id: account_id.to_string(),
            update: update.clone(),
        };
        let mut output = self.process_normalized(NormalizedEvent::Account(update));
        output
            .records
            .insert(0, StorageRecord::AccountSnapshot(snapshot));
        self.startup.mark_account_snapshot(
            account_id,
            true,
            "authoritative REST account snapshot applied to strategy and risk engine",
        )?;
        Ok(output)
    }

    pub fn on_submit_outcome(
        &mut self,
        account_id: &str,
        outcome: SubmitOutcome,
        ts_ms: TimeMs,
    ) -> Result<CoordinatorOutput, CoordinatorError> {
        self.private_state_mut(account_id)?;
        let (client_order_id, exchange_order_id, status) = match outcome {
            SubmitOutcome::Submitted {
                client_order_id,
                exchange_order_id,
            } => (
                client_order_id,
                Some(exchange_order_id),
                OrderAckStatus::Accepted,
            ),
            SubmitOutcome::Duplicate {
                client_order_id,
                exchange_order_id,
            } => (
                client_order_id,
                Some(exchange_order_id),
                OrderAckStatus::Duplicate,
            ),
            SubmitOutcome::PendingReconciliation { client_order_id } => {
                let mut output = self.reconciliation_fault(
                    account_id,
                    ts_ms,
                    None,
                    format!("submit {client_order_id} is pending reconciliation"),
                )?;
                output.records.push(StorageRecord::OrderAck(OrderAckRecord {
                    ts_ms,
                    account_id: account_id.to_string(),
                    operation: OrderOperation::Submit,
                    client_order_id,
                    exchange_order_id: None,
                    status: OrderAckStatus::PendingReconciliation,
                    message: "idempotency key remains pending".to_string(),
                }));
                return Ok(output);
            }
        };
        if let Some(exchange_order_id) = exchange_order_id.as_deref() {
            self.owned_regular_orders.bind_exchange_order_id(
                account_id,
                &client_order_id,
                exchange_order_id,
            )?;
            self.private_state_mut(account_id)?
                .bind_exchange_order_id(&client_order_id, exchange_order_id)
                .map_err(|source| CoordinatorError::PrivateOrderIdentity {
                    account_id: account_id.to_string(),
                    source,
                })?;
        }
        Ok(CoordinatorOutput {
            actions: Vec::new(),
            records: vec![StorageRecord::OrderAck(OrderAckRecord {
                ts_ms,
                account_id: account_id.to_string(),
                operation: OrderOperation::Submit,
                client_order_id,
                exchange_order_id,
                status,
                message: "exchange order acknowledgement received; awaiting private stream"
                    .to_string(),
            })],
        })
    }

    pub fn on_submit_error(
        &mut self,
        account_id: &str,
        client_order_id: &str,
        ts_ms: TimeMs,
        ambiguous: bool,
        reason: impl Into<String>,
    ) -> Result<CoordinatorOutput, CoordinatorError> {
        let reason = reason.into();
        if ambiguous {
            let mut output = self.reconciliation_fault(
                account_id,
                ts_ms,
                None,
                format!("ambiguous submit {client_order_id}: {reason}"),
            )?;
            output.records.push(StorageRecord::OrderAck(OrderAckRecord {
                ts_ms,
                account_id: account_id.to_string(),
                operation: OrderOperation::Submit,
                client_order_id: client_order_id.to_string(),
                exchange_order_id: None,
                status: OrderAckStatus::Ambiguous,
                message: reason,
            }));
            return Ok(output);
        }
        let update =
            self.private_state_mut(account_id)?
                .reject_local_order(client_order_id, ts_ms, &reason);
        let mut output = CoordinatorOutput {
            actions: Vec::new(),
            records: vec![StorageRecord::OrderAck(OrderAckRecord {
                ts_ms,
                account_id: account_id.to_string(),
                operation: OrderOperation::Submit,
                client_order_id: client_order_id.to_string(),
                exchange_order_id: None,
                status: OrderAckStatus::Rejected,
                message: reason,
            })],
        };
        if let Some(update) = update {
            output.extend(self.process_normalized(NormalizedEvent::Order(update)));
        }
        Ok(output)
    }

    pub fn on_cancel_outcome(
        &mut self,
        account_id: &str,
        outcome: CancelOutcome,
        ts_ms: TimeMs,
    ) -> Result<CoordinatorOutput, CoordinatorError> {
        let client_order_id = {
            let state = self.private_state_mut(account_id)?;
            let client_order_id =
                state.resolve_order_id(&outcome.client_order_id, &outcome.exchange_order_id);
            state
                .bind_exchange_order_id(&client_order_id, &outcome.exchange_order_id)
                .map_err(|source| CoordinatorError::PrivateOrderIdentity {
                    account_id: account_id.to_string(),
                    source,
                })?;
            client_order_id
        };
        self.owned_regular_orders.bind_exchange_order_id(
            account_id,
            &client_order_id,
            &outcome.exchange_order_id,
        )?;
        Ok(CoordinatorOutput {
            actions: Vec::new(),
            records: vec![StorageRecord::OrderAck(OrderAckRecord {
                ts_ms,
                account_id: account_id.to_string(),
                operation: OrderOperation::Cancel,
                client_order_id,
                exchange_order_id: Some(outcome.exchange_order_id),
                status: OrderAckStatus::Accepted,
                message: "cancel acknowledgement received; awaiting private stream".to_string(),
            })],
        })
    }

    pub fn on_cancel_error(
        &mut self,
        account_id: &str,
        client_order_id: &str,
        ts_ms: TimeMs,
        ambiguous: bool,
        reason: impl Into<String>,
    ) -> Result<CoordinatorOutput, CoordinatorError> {
        let reason = reason.into();
        let mut output = self.reconciliation_fault(
            account_id,
            ts_ms,
            None,
            format!("cancel {client_order_id} failed: {reason}"),
        )?;
        output.records.push(StorageRecord::OrderAck(OrderAckRecord {
            ts_ms,
            account_id: account_id.to_string(),
            operation: OrderOperation::Cancel,
            client_order_id: client_order_id.to_string(),
            exchange_order_id: None,
            status: if ambiguous {
                OrderAckStatus::Ambiguous
            } else {
                OrderAckStatus::Rejected
            },
            message: reason,
        }));
        Ok(output)
    }

    pub fn on_reconciliation(
        &mut self,
        result: ReconciliationResult,
    ) -> Result<CoordinatorOutput, CoordinatorError> {
        let ReconciliationResult {
            account_id,
            ts_ms,
            clean,
            local_live_orders,
            remote_live_orders,
            remote_recent_fills,
            reason,
        } = result;
        self.startup
            .mark_reconciled(&account_id, clean, reason.clone())?;
        let mut output = CoordinatorOutput {
            actions: Vec::new(),
            records: vec![StorageRecord::Reconciliation(ReconciliationRecord {
                ts_ms,
                account_id: account_id.clone(),
                clean,
                local_live_orders,
                remote_live_orders,
                remote_recent_fills,
                reason: reason.clone(),
            })],
        };
        if clean {
            return Ok(output);
        }
        let event = SystemEvent {
            ts_ms,
            kind: SystemEventKind::ReconcileDrift,
            venue: Some(Venue::Okx),
            account_id: Some(account_id),
            symbol: None,
            reason,
        };
        output.extend(self.process_normalized(NormalizedEvent::System(event)));
        Ok(output)
    }

    pub fn require_reconciliation(
        &mut self,
        account_id: &str,
        ts_ms: TimeMs,
        reason: impl Into<String>,
    ) -> Result<CoordinatorOutput, CoordinatorError> {
        let reason = reason.into();
        self.startup
            .mark_reconciled(account_id, false, reason.clone())?;
        Ok(CoordinatorOutput {
            actions: vec![LiveAction::Reconcile(ReconcileAction {
                ts_ms,
                account_id: account_id.to_string(),
                reason,
            })],
            records: Vec::new(),
        })
    }

    pub(crate) fn reconciliation_fault(
        &mut self,
        account_id: &str,
        ts_ms: TimeMs,
        symbol: Option<String>,
        reason: String,
    ) -> Result<CoordinatorOutput, CoordinatorError> {
        self.startup
            .mark_reconciled(account_id, false, reason.clone())?;
        let event = SystemEvent {
            ts_ms,
            kind: SystemEventKind::ReconcileDrift,
            venue: Some(Venue::Okx),
            account_id: Some(account_id.to_string()),
            symbol,
            reason: reason.clone(),
        };
        let mut output = self.process_normalized(NormalizedEvent::System(event));
        output.actions.push(LiveAction::Reconcile(ReconcileAction {
            ts_ms,
            account_id: account_id.to_string(),
            reason,
        }));
        Ok(output)
    }
}
