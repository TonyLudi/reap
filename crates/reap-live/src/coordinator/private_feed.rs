use reap_core::{AccountUpdate, FillKey, NormalizedEvent, OrderStatus, TimeMs};
use reap_storage::{FillRecord, StorageRecord};
use reap_venue::{PrivateOrderUpdate, RemoteFill};

use super::{CoordinatorError, CoordinatorOutput, LiveCoordinator, scope_account_update};

impl LiveCoordinator {
    pub(super) fn process_private_account(
        &mut self,
        account_id: Option<String>,
        update: AccountUpdate,
    ) -> Result<CoordinatorOutput, CoordinatorError> {
        let observed_now_ms = update.ts_ms;
        self.process_private_account_at(account_id, update, observed_now_ms)
    }

    pub(super) fn process_private_account_at(
        &mut self,
        account_id: Option<String>,
        update: AccountUpdate,
        observed_now_ms: TimeMs,
    ) -> Result<CoordinatorOutput, CoordinatorError> {
        let mut local_send_clock = || observed_now_ms;
        self.process_private_account_at_with_clock(
            account_id,
            update,
            observed_now_ms,
            &mut local_send_clock,
        )
    }

    pub(super) fn process_private_account_at_with_clock(
        &mut self,
        account_id: Option<String>,
        update: AccountUpdate,
        observed_now_ms: TimeMs,
        local_send_clock: &mut dyn FnMut() -> TimeMs,
    ) -> Result<CoordinatorOutput, CoordinatorError> {
        let account_id = self.require_account_id(account_id)?;
        self.ensure_account_state_policy(&account_id, &update)?;
        let mut update = update;
        scope_account_update(&account_id, &mut update);
        let Some(update) = self.private_state_mut(&account_id)?.reduce_account(update) else {
            return Ok(CoordinatorOutput::default());
        };
        let output = self.process_normalized_at_with_clock(
            NormalizedEvent::Account(update),
            observed_now_ms,
            local_send_clock,
        );
        self.startup.mark_account_snapshot(
            &account_id,
            true,
            "account snapshot applied to strategy and risk engine",
        )?;
        Ok(output)
    }

    pub(super) fn process_private_order(
        &mut self,
        account_id: Option<String>,
        update: PrivateOrderUpdate,
    ) -> Result<CoordinatorOutput, CoordinatorError> {
        let observed_now_ms = update.ts_ms;
        self.process_private_order_at(account_id, update, observed_now_ms)
    }

    pub(super) fn process_private_order_at(
        &mut self,
        account_id: Option<String>,
        update: PrivateOrderUpdate,
        observed_now_ms: TimeMs,
    ) -> Result<CoordinatorOutput, CoordinatorError> {
        let mut local_send_clock = || observed_now_ms;
        self.process_private_order_at_with_clock(
            account_id,
            update,
            observed_now_ms,
            &mut local_send_clock,
        )
    }

    pub(super) fn process_private_order_at_with_clock(
        &mut self,
        account_id: Option<String>,
        update: PrivateOrderUpdate,
        observed_now_ms: TimeMs,
        local_send_clock: &mut dyn FnMut() -> TimeMs,
    ) -> Result<CoordinatorOutput, CoordinatorError> {
        let account_id = self.require_account_id(account_id)?;
        let reported_order_id =
            if update.client_order_id.is_empty() || update.client_order_id == "0" {
                update.exchange_order_id.as_str()
            } else {
                update.client_order_id.as_str()
            };
        self.ensure_private_order_account(&account_id, reported_order_id, &update.symbol)?;
        let (canonical_id, known) = {
            let state = self
                .private_state(&account_id)
                .expect("validated account must have private state");
            let canonical_id =
                state.resolve_order_id(&update.client_order_id, &update.exchange_order_id);
            let known = state.order_reducer().contains_order(&canonical_id);
            (canonical_id, known)
        };
        let fill_id = update.fill_id.clone();
        let fill_key = fill_id
            .as_ref()
            .map(|fill_id| FillKey::new(&update.symbol, fill_id));
        let fill_was_journaled = fill_key.as_ref().is_some_and(|fill_key| {
            self.journal_fill_keys_by_account
                .get(&account_id)
                .is_some_and(|fill_keys| fill_keys.contains(fill_key))
        });
        let raw_fill_record =
            if !fill_was_journaled && update.last_fill_qty > 0.0 && update.last_fill_price > 0.0 {
                fill_id.clone().map(|fill_id| FillRecord {
                    ts_ms: update.ts_ms,
                    account_id: Some(account_id.clone()),
                    fill_id,
                    order_id: canonical_id.clone(),
                    symbol: update.symbol.clone(),
                    side: update.side,
                    price: update.last_fill_price,
                    qty: update.last_fill_qty,
                    liquidity: update.liquidity,
                    fee: update.last_fill_fee.clone(),
                })
            } else {
                None
            };
        let ts_ms = update.ts_ms;
        let symbol = update.symbol.clone();
        let canonical = self
            .private_state_mut(&account_id)?
            .apply_order(update)
            .map_err(|source| CoordinatorError::PrivateOrderIdentity {
                account_id: account_id.clone(),
                source,
            })?;
        let canonical_identity = self
            .private_state(&account_id)
            .and_then(|state| state.order_reducer().get(&canonical_id))
            .map(|order| (order.symbol.clone(), order.status));
        let proven_owned = canonical_identity.as_ref().is_some_and(|(symbol, _)| {
            self.owned_regular_orders
                .proves_identity(&canonical_id, &account_id, symbol)
        });
        if !proven_owned {
            let active = canonical_identity.as_ref().is_some_and(|(_, status)| {
                matches!(
                    status,
                    OrderStatus::PendingNew | OrderStatus::Live | OrderStatus::PartiallyFilled
                )
            });
            self.startup.mark_runtime_health(
                        &format!("foreign_regular_order:{account_id}:{canonical_id}"),
                        !active,
                        if active {
                            format!(
                                "unproven regular order {canonical_id} is live on account {account_id}; operator handling is required"
                            )
                        } else {
                            format!(
                                "unproven regular order {canonical_id} is terminal on account {account_id}"
                            )
                        },
                    );
        }
        let mut output = CoordinatorOutput::default();
        if !known {
            output.extend(self.reconciliation_fault(
                &account_id,
                ts_ms,
                Some(symbol),
                format!("private update for unknown order {canonical_id}"),
            )?);
        }
        let canonical_fill_record = canonical.as_ref().and_then(|update| {
            if !fill_was_journaled && update.has_fill() {
                fill_id.map(|fill_id| FillRecord {
                    ts_ms: update.ts_ms,
                    account_id: Some(account_id.clone()),
                    fill_id,
                    order_id: update.order_id.clone(),
                    symbol: update.symbol.clone(),
                    side: update.side,
                    price: update.last_fill_price,
                    qty: update.last_fill_qty,
                    liquidity: update.last_fill_liquidity,
                    fee: update.last_fill_fee.clone(),
                })
            } else {
                None
            }
        });
        if let Some(update) = canonical {
            output.extend(self.process_normalized_at_with_clock(
                NormalizedEvent::Order(update),
                observed_now_ms,
                local_send_clock,
            ));
        }
        if let Some(fill_record) = canonical_fill_record.or(raw_fill_record) {
            self.journal_fill_keys_by_account
                .entry(account_id)
                .or_default()
                .insert(FillKey::new(&fill_record.symbol, &fill_record.fill_id));
            output.records.push(StorageRecord::Fill(fill_record));
        }
        Ok(output)
    }

    pub(super) fn process_private_fill(
        &mut self,
        account_id: Option<String>,
        fill: RemoteFill,
    ) -> Result<CoordinatorOutput, CoordinatorError> {
        let observed_now_ms = fill.ts_ms;
        self.process_private_fill_at(account_id, fill, observed_now_ms)
    }

    pub(super) fn process_private_fill_at(
        &mut self,
        account_id: Option<String>,
        fill: RemoteFill,
        observed_now_ms: TimeMs,
    ) -> Result<CoordinatorOutput, CoordinatorError> {
        let mut local_send_clock = || observed_now_ms;
        self.process_private_fill_at_with_clock(
            account_id,
            fill,
            observed_now_ms,
            &mut local_send_clock,
        )
    }

    pub(super) fn process_private_fill_at_with_clock(
        &mut self,
        account_id: Option<String>,
        fill: RemoteFill,
        observed_now_ms: TimeMs,
        local_send_clock: &mut dyn FnMut() -> TimeMs,
    ) -> Result<CoordinatorOutput, CoordinatorError> {
        let account_id = self.require_account_id(account_id)?;
        let reported_order_id = if fill.client_order_id.is_empty() || fill.client_order_id == "0" {
            fill.exchange_order_id.as_str()
        } else {
            fill.client_order_id.as_str()
        };
        self.ensure_private_order_account(&account_id, reported_order_id, &fill.symbol)?;
        let (canonical_id, known) = {
            let state = self
                .private_state(&account_id)
                .expect("validated account must have private state");
            let canonical_id =
                state.resolve_order_id(&fill.client_order_id, &fill.exchange_order_id);
            let known = state.order_reducer().contains_order(&canonical_id);
            (canonical_id, known)
        };
        let fill_key = FillKey::new(&fill.symbol, &fill.fill_id);
        let fill_was_journaled = self
            .journal_fill_keys_by_account
            .get(&account_id)
            .is_some_and(|fill_keys| fill_keys.contains(&fill_key));
        let fill_record = FillRecord {
            ts_ms: fill.ts_ms,
            account_id: Some(account_id.clone()),
            fill_id: fill.fill_id.clone(),
            order_id: canonical_id.clone(),
            symbol: fill.symbol.clone(),
            side: fill.side,
            price: fill.price,
            qty: fill.qty,
            liquidity: Some(fill.liquidity),
            fee: fill.fee.clone(),
        };
        let ts_ms = fill.ts_ms;
        let symbol = fill.symbol.clone();
        let canonical = self
            .private_state_mut(&account_id)?
            .apply_fill(fill)
            .map_err(|source| CoordinatorError::PrivateOrderIdentity {
                account_id: account_id.clone(),
                source,
            })?;
        // The VIP fills channel currently omits per-fill fees. Let its
        // earlier state update race without consuming the journal key;
        // the required orders channel can then persist exact evidence.
        let journal_fill = (!fill_was_journaled && fill_record.fee.is_some())
            .then_some(StorageRecord::Fill(fill_record));
        if journal_fill.is_some() {
            self.journal_fill_keys_by_account
                .entry(account_id.clone())
                .or_default()
                .insert(fill_key);
        }
        let mut output = CoordinatorOutput {
            actions: Vec::new(),
            records: journal_fill.into_iter().collect(),
        };
        if !known {
            output.extend(self.reconciliation_fault(
                &account_id,
                ts_ms,
                Some(symbol),
                format!("fill for unknown order {canonical_id}"),
            )?);
        }
        if let Some(update) = canonical {
            output.extend(self.process_normalized_at_with_clock(
                NormalizedEvent::Order(update),
                observed_now_ms,
                local_send_clock,
            ));
        }
        Ok(output)
    }
}
