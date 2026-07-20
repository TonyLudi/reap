use std::collections::HashSet;

use reap_core::{NormalizedEvent, OrderIntent, TimeMs};
use reap_engine::SafetyCancelCandidate;
use reap_order::RegularExecutionPolicyError;
use reap_storage::StorageRecord;
use reap_strategy::ChaosExecutionIntent;

use super::{CancelAction, CoordinatorOutput, LiveAction, LiveCoordinator, SubmitAction};

impl LiveCoordinator {
    pub(super) fn route_chaos_intent(
        &mut self,
        now_ms: TimeMs,
        observed_now_ms: TimeMs,
        intent: ChaosExecutionIntent,
        legacy: OrderIntent,
        local_send_clock: &mut dyn FnMut() -> TimeMs,
        output: &mut CoordinatorOutput,
    ) {
        match intent {
            intent @ (ChaosExecutionIntent::Quote(_) | ChaosExecutionIntent::Hedge(_)) => {
                let symbol = match &intent {
                    ChaosExecutionIntent::Quote(quote) => quote.symbol(),
                    ChaosExecutionIntent::Hedge(hedge) => hedge.symbol(),
                    ChaosExecutionIntent::CancelOwned(_) => unreachable!(),
                };
                let Some(account_id) = self
                    .config
                    .account_for_symbol(symbol)
                    .map(|account| account.id.clone())
                else {
                    output.records.push(StorageRecord::IntentRejected {
                        ts_ms: now_ms,
                        intent: legacy,
                        reason: "symbol has no account route".to_string(),
                    });
                    self.startup.mark_runtime_health(
                        "routing",
                        false,
                        "strategy emitted an order for an unmapped symbol",
                    );
                    return;
                };
                let gateway_actions_enabled = self.gateway_action_accounts.contains(&account_id);
                let submit_enabled = gateway_actions_enabled && self.order_entry_enabled;
                if let Some(reason) = self.halted_accounts.get(&account_id) {
                    output.records.push(StorageRecord::IntentRejected {
                        ts_ms: now_ms,
                        intent: legacy,
                        reason: format!("account {account_id} is halted: {reason}"),
                    });
                    return;
                }
                if !self.startup.can_submit_new(submit_enabled) {
                    output.records.push(StorageRecord::IntentRejected {
                        ts_ms: now_ms,
                        intent: legacy,
                        reason: format!(
                            "live gate is {:?}; gateway actions enabled={}; new order entry enabled={}",
                            self.startup.phase(),
                            gateway_actions_enabled,
                            self.order_entry_enabled
                        ),
                    });
                    return;
                }
                let reservation = {
                    let regular_execution = &self.regular_execution;
                    let decision_sequence = &mut self.decision_sequence;
                    let strategy_name = &self.config.strategy.strategy_name;
                    let session_id = &self.session_id;
                    let client_ids = &self.client_ids;
                    let owned_regular_orders = &mut self.owned_regular_orders;
                    let private_states = &mut self.private_states;
                    self.engine.with_locally_sent_chaos_intent(
                        intent,
                        &mut *local_send_clock,
                        |intent| {
                            let approved = regular_execution.authorize_submit(intent)?;
                            if approved.account_id() != account_id {
                                return Err(RegularExecutionPolicyError::OwnerMismatch {
                                    symbol: approved.order().symbol.clone(),
                                    actual: approved.account_id().to_string(),
                                    expected: account_id.clone(),
                                });
                            }
                            *decision_sequence = decision_sequence.wrapping_add(1);
                            let idempotency_key =
                                format!("{strategy_name}:{session_id}:{}", *decision_sequence);
                            let client_order_id = client_ids
                                .get(&account_id)
                                .expect("validated account must have a client id generator")
                                .next(now_ms);
                            let (pending, reserved) = owned_regular_orders.reserve_local(
                                approved,
                                client_order_id,
                                private_states
                                    .get_mut(&account_id)
                                    .expect("validated account must have private state"),
                                now_ms,
                            )?;
                            Ok((pending, reserved, idempotency_key))
                        },
                    )
                };
                let (pending, reserved, idempotency_key) = match reservation {
                    Ok(reservation) => reservation,
                    Err(error) => {
                        self.reject_execution_policy(now_ms, legacy, error, output);
                        return;
                    }
                };
                output.actions.push(LiveAction::Submit(SubmitAction::new(
                    now_ms,
                    idempotency_key,
                    reserved,
                )));
                output.extend(self.process_normalized_at_with_clock(
                    NormalizedEvent::Order(pending),
                    observed_now_ms,
                    &mut *local_send_clock,
                ));
            }
            ChaosExecutionIntent::CancelOwned(cancel) => {
                let order_id = cancel.order_id().to_string();
                let reason = cancel.reason().to_string();
                self.route_cancel_owned(now_ms, &order_id, &reason, legacy, output);
            }
        }
    }

    pub(super) fn route_safety_cancel(
        &mut self,
        now_ms: TimeMs,
        candidate: SafetyCancelCandidate,
        legacy: OrderIntent,
        output: &mut CoordinatorOutput,
    ) {
        self.route_cancel_owned(
            now_ms,
            candidate.order_id(),
            candidate.reason(),
            legacy,
            output,
        );
    }

    fn route_cancel_owned(
        &mut self,
        now_ms: TimeMs,
        order_id: &str,
        reason: &str,
        legacy: OrderIntent,
        output: &mut CoordinatorOutput,
    ) {
        if self.gateway_action_accounts.is_empty() || !self.startup.can_cancel() {
            let rejection_reason = if self.gateway_action_accounts.is_empty() {
                "live gateway actions are disabled in observe mode".to_string()
            } else {
                format!(
                    "live gate is {:?}; cancellation is unavailable",
                    self.startup.phase()
                )
            };
            output.records.push(StorageRecord::IntentRejected {
                ts_ms: now_ms,
                intent: legacy,
                reason: rejection_reason,
            });
            return;
        }
        let approved = match self.regular_execution.authorize_cancel(
            order_id,
            reason,
            &self.owned_regular_orders,
            &self.private_states,
        ) {
            Ok(approved) => approved,
            Err(error) => {
                self.reject_execution_policy(now_ms, legacy, error, output);
                return;
            }
        };
        if !self.gateway_action_accounts.contains(approved.account_id()) {
            output.records.push(StorageRecord::IntentRejected {
                ts_ms: now_ms,
                intent: legacy,
                reason: format!(
                    "account {} has no planned regular order transport",
                    approved.account_id()
                ),
            });
            return;
        }
        output
            .actions
            .push(LiveAction::Cancel(CancelAction::new(now_ms, approved)));
    }

    fn reject_execution_policy(
        &mut self,
        now_ms: TimeMs,
        intent: OrderIntent,
        error: RegularExecutionPolicyError,
        output: &mut CoordinatorOutput,
    ) {
        let reason = error.to_string();
        self.startup
            .mark_runtime_health("regular_execution_policy", false, reason.clone());
        output.records.push(StorageRecord::IntentRejected {
            ts_ms: now_ms,
            intent,
            reason,
        });
    }

    pub(super) fn ensure_account_cancels(
        &mut self,
        now_ms: TimeMs,
        account_id: &str,
        cancel_reason: &str,
        output: &mut CoordinatorOutput,
    ) {
        let existing = output
            .actions
            .iter()
            .filter_map(|action| match action {
                LiveAction::Cancel(cancel) if cancel.account_id() == account_id => {
                    Some(cancel.client_order_id().to_string())
                }
                _ => None,
            })
            .collect::<HashSet<_>>();
        let active_orders = self
            .private_states
            .get(account_id)
            .expect("validated account halt must have private state")
            .order_reducer()
            .orders()
            .filter(|(_, order)| {
                matches!(
                    order.status,
                    reap_core::OrderStatus::PendingNew
                        | reap_core::OrderStatus::Live
                        | reap_core::OrderStatus::PartiallyFilled
                )
            })
            .filter(|(order_id, _)| {
                self.owned_regular_orders
                    .proves_account(order_id, account_id)
            })
            .map(|(order_id, _)| order_id.to_string())
            .filter(|order_id| !existing.contains(order_id))
            .collect::<Vec<_>>();
        for order_id in active_orders {
            let intent = OrderIntent::CancelOrder {
                order_id: order_id.clone(),
                reason: cancel_reason.to_string(),
            };
            output.records.push(StorageRecord::Intent {
                ts_ms: now_ms,
                intent: intent.clone(),
            });
            self.route_cancel_owned(now_ms, &order_id, cancel_reason, intent, output);
        }
    }
}
