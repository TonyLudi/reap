use std::collections::HashMap;

use reap_core::{NormalizedEvent, OrderStatus, OrderUpdate, SystemEvent, SystemEventKind};
use reap_storage::{RecoveredStorage, SafetyLatchRecord, SafetyLatchSource};
use reap_venue::{PrivateOrderState, PrivateOrderUpdate, RemoteOrder};

use crate::{CoordinatorError, CoordinatorOutput, LiveConfig, LiveCoordinator};

use super::{LiveRuntimeError, unix_time_ms};

pub(super) fn remote_order_id(order: &RemoteOrder) -> String {
    if order.client_order_id.is_empty() {
        order.exchange_order_id.clone()
    } else {
        order.client_order_id.clone()
    }
}

pub(super) fn private_update_from_remote(order: RemoteOrder) -> PrivateOrderUpdate {
    PrivateOrderUpdate {
        ts_ms: if order.update_time_ms == 0 {
            unix_time_ms()
        } else {
            order.update_time_ms
        },
        exchange_order_id: order.exchange_order_id,
        client_order_id: order.client_order_id,
        symbol: order.symbol,
        side: order.side,
        state: order.state,
        price: order.price,
        qty: order.qty,
        cumulative_filled_qty: order.cumulative_filled_qty,
        average_fill_price: order.average_fill_price,
        last_fill_qty: 0.0,
        last_fill_price: 0.0,
        liquidity: None,
        last_fill_fee: None,
        fill_id: None,
        reject_reason: if order.state == PrivateOrderState::Rejected {
            "order not present during restart reconciliation".to_string()
        } else {
            String::new()
        },
    }
}

pub(super) fn validate_recovered_safety_latches(
    config: &LiveConfig,
    recovered: &RecoveredStorage,
) -> Result<(), LiveRuntimeError> {
    for account_id in recovered.account_safety_latches.keys() {
        if config.account(account_id).is_none() {
            return Err(LiveRuntimeError::BootstrapVerification(format!(
                "persistent safety latch references unknown account {account_id}; retain the journal and reconcile before changing account identity"
            )));
        }
    }
    for symbol in recovered.symbol_safety_latches.keys() {
        if !config.required_symbols().contains(symbol) {
            return Err(LiveRuntimeError::BootstrapVerification(format!(
                "persistent safety latch references unmanaged symbol {symbol}; retain the journal and reconcile before changing instrument identity"
            )));
        }
    }
    Ok(())
}

pub(super) fn recovered_safety_latch_count(recovered: &RecoveredStorage) -> u64 {
    let total = usize::from(recovered.global_safety_latch.is_some())
        .saturating_add(recovered.account_safety_latches.len())
        .saturating_add(recovered.symbol_safety_latches.len());
    u64::try_from(total).unwrap_or(u64::MAX)
}

pub(super) fn proven_active_recovered_orders(
    config: &LiveConfig,
    recovered: &mut RecoveredStorage,
) -> Vec<(
    reap_core::OrderUpdate,
    reap_storage::ProvenRegularSubmitRequest,
)> {
    let requests = std::mem::take(&mut recovered.proven_regular_submit_requests);
    let mut orders = requests
        .into_values()
        .filter_map(|proof| {
            let update = recovered.latest_orders.get(proof.client_order_id())?;
            if !matches!(
                update.status,
                OrderStatus::PendingNew | OrderStatus::Live | OrderStatus::PartiallyFilled
            ) {
                return None;
            }
            let account_id = config
                .account_for_symbol(&update.symbol)
                .map(|account| account.id.as_str())?;
            (proof.account_id() == account_id
                && proof.symbol() == update.symbol
                && proof.client_order_id() == update.order_id)
                .then(|| (update.clone(), proof))
        })
        .collect::<Vec<_>>();
    orders.sort_by(|(left, _), (right, _)| left.order_id.cmp(&right.order_id));
    orders
}

pub(super) struct RecoveredActiveOrders(
    Vec<(
        reap_core::OrderUpdate,
        reap_storage::ProvenRegularSubmitRequest,
    )>,
);

impl RecoveredActiveOrders {
    pub(super) fn take(config: &LiveConfig, recovered: &mut RecoveredStorage) -> Self {
        Self(proven_active_recovered_orders(config, recovered))
    }

    pub(super) fn restored_by_account(
        &self,
        config: &LiveConfig,
    ) -> Result<HashMap<String, Vec<OrderUpdate>>, LiveRuntimeError> {
        let mut restored_by_account: HashMap<String, Vec<OrderUpdate>> = HashMap::new();
        for (update, _) in &self.0 {
            let account_id = config
                .account_for_symbol(&update.symbol)
                .map(|account| account.id.clone())
                .ok_or_else(|| {
                    LiveRuntimeError::BootstrapVerification(format!(
                        "recovered order {} has unmapped symbol {}",
                        update.order_id, update.symbol
                    ))
                })?;
            restored_by_account
                .entry(account_id)
                .or_default()
                .push(update.clone());
        }
        Ok(restored_by_account)
    }

    pub(super) fn restore_into(
        self,
        coordinator: &mut LiveCoordinator,
        outputs: &mut Vec<CoordinatorOutput>,
    ) -> Result<(), LiveRuntimeError> {
        for (update, proof) in self.0 {
            outputs.push(coordinator.restore_owned_order(proof, update)?);
        }
        Ok(())
    }
}

pub(super) fn restore_active_order_bindings(
    coordinator: &mut LiveCoordinator,
    recovered: &mut RecoveredStorage,
) -> Result<(), LiveRuntimeError> {
    let bindings = std::mem::take(&mut recovered.proven_regular_order_bindings);
    for binding in bindings.into_values() {
        let account_id = binding.account_id().to_string();
        if !coordinator.manages_account(&account_id) {
            return Err(LiveRuntimeError::BootstrapVerification(format!(
                "recovered order binding references unknown account {account_id}; retain the journal and reconcile before changing account identity"
            )));
        }
        let active_order_is_restored =
            coordinator.private_state(&account_id).is_some_and(|state| {
                state
                    .order_reducer()
                    .contains_order(binding.client_order_id())
            });
        if active_order_is_restored {
            coordinator.restore_order_binding(binding)?;
        }
    }
    Ok(())
}

fn restored_latch_reason(latch: &SafetyLatchRecord) -> String {
    let source = match latch.source {
        SafetyLatchSource::Operator => "operator",
        SafetyLatchSource::Risk => "risk",
        SafetyLatchSource::LegacySystemEvent => "legacy system-event",
    };
    format!(
        "restored persistent {source} safety latch: {}",
        latch.reason
    )
}

pub(super) fn restore_safety_latches(
    coordinator: &mut LiveCoordinator,
    recovered: &RecoveredStorage,
) -> Result<Vec<CoordinatorOutput>, CoordinatorError> {
    let mut outputs = Vec::new();
    let now_ms = unix_time_ms();
    if let Some(latch) = &recovered.global_safety_latch {
        coordinator.set_order_entry_enabled(false);
        outputs.push(
            coordinator.process_event(NormalizedEvent::System(SystemEvent {
                ts_ms: now_ms,
                kind: SystemEventKind::KillSwitchActivated,
                venue: None,
                account_id: None,
                symbol: None,
                reason: restored_latch_reason(latch),
            })),
        );
    }
    for (account_id, latch) in &recovered.account_safety_latches {
        outputs.push(coordinator.halt_account(now_ms, account_id, restored_latch_reason(latch))?);
    }
    for (symbol, latch) in &recovered.symbol_safety_latches {
        outputs.push(
            coordinator.process_event(NormalizedEvent::System(SystemEvent {
                ts_ms: now_ms,
                kind: SystemEventKind::SymbolHalted,
                venue: None,
                account_id: None,
                symbol: Some(symbol.clone()),
                reason: restored_latch_reason(latch),
            })),
        );
    }
    Ok(outputs)
}
