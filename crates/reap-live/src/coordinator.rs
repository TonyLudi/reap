use std::collections::{BTreeMap, HashMap, HashSet};

use reap_core::{
    FillKey, NormalizedEvent, OrderIntent, OrderStatus, SystemEvent, SystemEventKind, TimeMs, Venue,
};
use reap_engine::{ChaosEngineOutput, SafetyCancelCandidate, TradingEngine};
use reap_feed::{FeedOutput, RecoveryRequest};
use reap_order::{
    CancelOutcome, ClientOrderIdGenerator, PrivateOrderIdentityError, PrivateStateReducer,
    SubmitOutcome,
};
use reap_risk::{RiskDecision, RiskGate};
use reap_storage::{
    AccountSnapshotRecord, FillRecord, OrderAckRecord, OrderAckStatus, OrderOperation,
    ReconciliationRecord, SafetyLatchRecord, SafetyLatchScope, SafetyLatchSource, StorageRecord,
};
use reap_strategy::{ChaosExecutionIntent, ChaosStrategy};
use thiserror::Error;

use crate::forbidden_orders::ForbiddenOrderEvent;
use crate::regular_execution::{
    OwnedRegularOrders, RegularExecutionPolicy, RegularExecutionPolicyError,
};
use crate::{LiveConfig, ReadinessSnapshot, StartupError, StartupGate, VerifiedBootstrap};

#[derive(Debug, Clone)]
pub struct SubmitAction {
    pub ts_ms: TimeMs,
    pub account_id: String,
    pub idempotency_key: String,
    pub client_order_id: String,
    pub order: reap_core::NewOrder,
}

#[derive(Debug, Clone)]
pub struct CancelAction {
    pub ts_ms: TimeMs,
    pub account_id: String,
    pub symbol: String,
    pub client_order_id: String,
    pub reason: String,
}

#[derive(Debug, Clone)]
pub struct ReconcileAction {
    pub ts_ms: TimeMs,
    pub account_id: String,
    pub reason: String,
}

#[derive(Debug, Clone)]
pub struct ReconciliationResult {
    pub account_id: String,
    pub ts_ms: TimeMs,
    pub clean: bool,
    pub local_live_orders: usize,
    pub remote_live_orders: usize,
    pub remote_recent_fills: usize,
    pub reason: String,
}

#[derive(Debug, Clone)]
pub enum LiveAction {
    Submit(SubmitAction),
    Cancel(CancelAction),
    RecoverBook(RecoveryRequest),
    Reconcile(ReconcileAction),
}

#[derive(Debug, Default)]
pub struct CoordinatorOutput {
    pub actions: Vec<LiveAction>,
    pub records: Vec<StorageRecord>,
}

impl CoordinatorOutput {
    fn extend(&mut self, other: Self) {
        self.actions.extend(other.actions);
        self.records.extend(other.records);
    }
}

#[derive(Debug, Error)]
pub enum CoordinatorError {
    #[error("strategy configuration is invalid: {0}")]
    Strategy(String),
    #[error("live session id must not be empty")]
    EmptySessionId,
    #[error("client order id setup failed for account {account_id}: {message}")]
    ClientIdSetup { account_id: String, message: String },
    #[error("private event has no account identity")]
    MissingAccountIdentity,
    #[error("private event references unknown account {0}")]
    UnknownAccount(String),
    #[error("order {order_id} for {symbol} was routed to account {actual}, expected {expected}")]
    WrongOrderAccount {
        order_id: String,
        symbol: String,
        actual: String,
        expected: String,
    },
    #[error("private order identity violation on account {account_id}: {source}")]
    PrivateOrderIdentity {
        account_id: String,
        #[source]
        source: PrivateOrderIdentityError,
    },
    #[error("account {account_id} state policy violation: {message}")]
    AccountStatePolicy { account_id: String, message: String },
    #[error("regular execution policy failed: {0}")]
    RegularExecutionPolicy(String),
    #[error(transparent)]
    Startup(#[from] StartupError),
}

impl From<RegularExecutionPolicyError> for CoordinatorError {
    fn from(error: RegularExecutionPolicyError) -> Self {
        Self::RegularExecutionPolicy(error.to_string())
    }
}

pub struct LiveCoordinator {
    config: LiveConfig,
    engine: TradingEngine<ChaosStrategy>,
    startup: StartupGate,
    private_states: HashMap<String, PrivateStateReducer>,
    regular_execution: RegularExecutionPolicy,
    owned_regular_orders: OwnedRegularOrders,
    client_ids: HashMap<String, ClientOrderIdGenerator>,
    gateway_action_accounts: HashSet<String>,
    order_entry_enabled: bool,
    halted_accounts: BTreeMap<String, String>,
    journal_fill_keys_by_account: HashMap<String, HashSet<FillKey>>,
    session_id: String,
    decision_sequence: u64,
}

impl LiveCoordinator {
    pub fn new(
        config: LiveConfig,
        verified: VerifiedBootstrap,
        gateway_actions_enabled: bool,
        session_id: impl Into<String>,
    ) -> Result<Self, CoordinatorError> {
        let gateway_action_accounts = if gateway_actions_enabled {
            config.required_accounts()
        } else {
            HashSet::new()
        };
        Self::new_with_order_transports(config, verified, gateway_action_accounts, session_id)
    }

    pub fn new_with_order_transports(
        config: LiveConfig,
        verified: VerifiedBootstrap,
        gateway_action_accounts: HashSet<String>,
        session_id: impl Into<String>,
    ) -> Result<Self, CoordinatorError> {
        let session_id = session_id.into();
        if session_id.trim().is_empty() {
            return Err(CoordinatorError::EmptySessionId);
        }
        let strategy = ChaosStrategy::new(config.strategy.clone())
            .map_err(|error| CoordinatorError::Strategy(error.to_string()))?;
        let regular_execution = RegularExecutionPolicy::from_verified(&config, &verified)?;
        let mut risk = RiskGate::new(config.risk.clone());
        for instrument in verified.instruments.values() {
            risk.set_instrument_model(instrument.symbol.clone(), instrument.risk_model);
            risk.set_instrument_order_limits(instrument.symbol.clone(), instrument.order_limits);
        }
        let mut startup =
            StartupGate::new_with_order_transports(&config, gateway_action_accounts.clone())?;
        startup.mark_metadata_verified();
        let order_entry_enabled = !gateway_action_accounts.is_empty();
        let mut private_states = HashMap::new();
        let mut client_ids = HashMap::new();
        // Baseline keys predate this session and recovered fill keys are already
        // durable, so neither should be appended again when private streams race.
        let journal_fill_keys_by_account = verified.baseline_fill_ids.clone();
        for account in &config.accounts {
            let mut state = PrivateStateReducer::new();
            if let Some(fill_ids) = verified.baseline_fill_ids.get(&account.id) {
                state.seed_fill_keys(fill_ids.iter().cloned());
            }
            if let Some(update) = verified.account_updates.get(&account.id) {
                state.apply_account(update.clone());
            }
            private_states.insert(account.id.clone(), state);
            client_ids.insert(
                account.id.clone(),
                ClientOrderIdGenerator::new(&account.id_prefix, account.node_id).map_err(
                    |error| CoordinatorError::ClientIdSetup {
                        account_id: account.id.clone(),
                        message: error.to_string(),
                    },
                )?,
            );
        }
        Ok(Self {
            config,
            engine: TradingEngine::new(strategy, risk),
            startup,
            private_states,
            regular_execution,
            owned_regular_orders: OwnedRegularOrders::default(),
            client_ids,
            gateway_action_accounts,
            order_entry_enabled,
            halted_accounts: BTreeMap::new(),
            journal_fill_keys_by_account,
            session_id,
            decision_sequence: 0,
        })
    }

    pub fn readiness(&self) -> ReadinessSnapshot {
        self.startup.snapshot()
    }

    pub fn set_order_entry_enabled(&mut self, enabled: bool) {
        self.order_entry_enabled = enabled;
    }

    pub fn manages_symbol(&self, symbol: &str) -> bool {
        self.config.account_for_symbol(symbol).is_some()
    }

    pub fn manages_account(&self, account_id: &str) -> bool {
        self.private_states.contains_key(account_id)
    }

    pub fn halted_accounts(&self) -> &BTreeMap<String, String> {
        &self.halted_accounts
    }

    pub fn halted_account_for_symbol(&self, symbol: &str) -> Option<&str> {
        let account_id = self.config.account_for_symbol(symbol)?.id.as_str();
        self.halted_accounts
            .contains_key(account_id)
            .then_some(account_id)
    }

    pub fn kill_switch_active(&self) -> bool {
        self.engine.risk().is_killed()
    }

    pub fn is_symbol_halted(&self, symbol: &str) -> bool {
        self.engine.risk().is_symbol_halted(symbol)
    }

    pub fn halt_account(
        &mut self,
        ts_ms: TimeMs,
        account_id: &str,
        reason: impl Into<String>,
    ) -> Result<CoordinatorOutput, CoordinatorError> {
        if !self.manages_account(account_id) {
            return Err(CoordinatorError::UnknownAccount(account_id.to_string()));
        }
        Ok(
            self.process_normalized(NormalizedEvent::System(SystemEvent {
                ts_ms,
                kind: SystemEventKind::AccountHalted,
                venue: None,
                account_id: Some(account_id.to_string()),
                symbol: None,
                reason: reason.into(),
            })),
        )
    }

    pub fn mark_storage_ready(&mut self, ready: bool, reason: impl Into<String>) {
        self.startup.mark_storage(ready, reason);
    }

    pub fn mark_public_connectivity(&mut self, ready: bool, reason: impl Into<String>) {
        self.startup.mark_public_connectivity(ready, reason);
    }

    pub(crate) fn on_forbidden_order_event(
        &mut self,
        event: ForbiddenOrderEvent,
    ) -> Result<CoordinatorOutput, CoordinatorError> {
        if !self.manages_account(&event.account_id) {
            return Err(CoordinatorError::UnknownAccount(event.account_id));
        }
        let verified_zero = event.state.is_verified_zero();
        let reason = event
            .state
            .failure_reason()
            .unwrap_or_else(|| "fresh complete forbidden-order zero proof".to_string());
        self.startup.mark_forbidden_order_proof(
            &event.account_id,
            verified_zero,
            reason.clone(),
        )?;
        if verified_zero {
            return Ok(CoordinatorOutput::default());
        }

        self.startup
            .mark_reconciled(&event.account_id, false, reason.clone())?;
        let mut output = CoordinatorOutput::default();
        self.ensure_account_cancels(
            event.observed_at_ms,
            &event.account_id,
            &reason,
            &mut output,
        );
        output.actions.push(LiveAction::Reconcile(ReconcileAction {
            ts_ms: event.observed_at_ms,
            account_id: event.account_id,
            reason,
        }));
        Ok(output)
    }

    pub fn private_state(&self, account_id: &str) -> Option<&PrivateStateReducer> {
        self.private_states.get(account_id)
    }

    pub(crate) fn restore_owned_order(
        &mut self,
        account_id: &str,
        update: reap_core::OrderUpdate,
    ) -> Result<CoordinatorOutput, CoordinatorError> {
        let expected = self
            .config
            .account_for_symbol(&update.symbol)
            .map(|account| account.id.clone())
            .unwrap_or_default();
        if expected != account_id {
            return Err(CoordinatorError::WrongOrderAccount {
                order_id: update.order_id.clone(),
                symbol: update.symbol.clone(),
                actual: account_id.to_string(),
                expected,
            });
        }
        self.regular_execution
            .validate_recovered_identity(account_id, &update.symbol)?;
        self.owned_regular_orders.register_recovered(
            account_id,
            &update.symbol,
            &update.order_id,
            None,
        )?;
        self.private_state_mut(account_id)?
            .restore_order_update(update.clone());
        Ok(self.process_normalized(NormalizedEvent::Order(update)))
    }

    pub fn restore_order_binding(
        &mut self,
        account_id: &str,
        client_order_id: &str,
        exchange_order_id: &str,
    ) -> Result<(), CoordinatorError> {
        self.owned_regular_orders.bind_exchange_order_id(
            account_id,
            client_order_id,
            exchange_order_id,
        )?;
        self.private_state_mut(account_id)?
            .bind_exchange_order_id(client_order_id, exchange_order_id)
            .map_err(|source| CoordinatorError::PrivateOrderIdentity {
                account_id: account_id.to_string(),
                source,
            })
    }

    pub fn process_feed(
        &mut self,
        output: FeedOutput,
    ) -> Result<CoordinatorOutput, CoordinatorError> {
        match output {
            FeedOutput::Event(event) => Ok(self.process_normalized(event)),
            FeedOutput::System(event) => {
                Ok(self.process_normalized(NormalizedEvent::System(event)))
            }
            FeedOutput::Duplicate(_) => Ok(CoordinatorOutput::default()),
            FeedOutput::RecoveryRequired(request) => Ok(CoordinatorOutput {
                actions: vec![LiveAction::RecoverBook(request)],
                records: Vec::new(),
            }),
            FeedOutput::PrivateAccount { account_id, update } => {
                let account_id = self.require_account_id(account_id)?;
                self.ensure_account_state_policy(&account_id, &update)?;
                let mut update = update;
                scope_account_update(&account_id, &mut update);
                let Some(update) = self.private_state_mut(&account_id)?.reduce_account(update)
                else {
                    return Ok(CoordinatorOutput::default());
                };
                let output = self.process_normalized(NormalizedEvent::Account(update));
                self.startup.mark_account_snapshot(
                    &account_id,
                    true,
                    "account snapshot applied to strategy and risk engine",
                )?;
                Ok(output)
            }
            FeedOutput::PrivateOrder { account_id, update } => {
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
                let raw_fill_record = if !fill_was_journaled
                    && update.last_fill_qty > 0.0
                    && update.last_fill_price > 0.0
                {
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
                        .get(&canonical_id)
                        .is_some_and(|owned| {
                            owned.account_id() == account_id && owned.symbol() == symbol
                        })
                });
                if !proven_owned {
                    let active = canonical_identity.as_ref().is_some_and(|(_, status)| {
                        matches!(
                            status,
                            OrderStatus::PendingNew
                                | OrderStatus::Live
                                | OrderStatus::PartiallyFilled
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
                    output.extend(self.process_normalized(NormalizedEvent::Order(update)));
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
            FeedOutput::PrivateFill { account_id, fill } => {
                let account_id = self.require_account_id(account_id)?;
                let reported_order_id =
                    if fill.client_order_id.is_empty() || fill.client_order_id == "0" {
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
                    output.extend(self.process_normalized(NormalizedEvent::Order(update)));
                }
                Ok(output)
            }
        }
    }

    pub fn process_feed_at(
        &mut self,
        output: FeedOutput,
        observed_now_ms: TimeMs,
    ) -> Result<CoordinatorOutput, CoordinatorError> {
        match output {
            FeedOutput::Event(event) => Ok(self.process_normalized_at(event, observed_now_ms)),
            output => self.process_feed(output),
        }
    }

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

    pub fn process_event(&mut self, event: NormalizedEvent) -> CoordinatorOutput {
        self.process_normalized(event)
    }

    fn ensure_account_state_policy(
        &self,
        account_id: &str,
        update: &reap_core::AccountUpdate,
    ) -> Result<(), CoordinatorError> {
        let errors = self
            .config
            .evaluate_account_state_policy(account_id, update);
        if errors.is_empty() {
            Ok(())
        } else {
            Err(CoordinatorError::AccountStatePolicy {
                account_id: account_id.to_string(),
                message: errors.join(", "),
            })
        }
    }

    fn ensure_private_order_account(
        &self,
        account_id: &str,
        order_id: &str,
        symbol: &str,
    ) -> Result<(), CoordinatorError> {
        let expected = self
            .config
            .account_for_symbol(symbol)
            .map(|account| account.id.as_str());
        if expected == Some(account_id) {
            return Ok(());
        }
        Err(CoordinatorError::WrongOrderAccount {
            order_id: order_id.to_string(),
            symbol: symbol.to_string(),
            actual: account_id.to_string(),
            expected: expected.unwrap_or("<unmapped>").to_string(),
        })
    }

    #[cfg(test)]
    pub fn register_local_order(
        &mut self,
        account_id: &str,
        client_order_id: &str,
        order: reap_core::NewOrder,
        ts_ms: TimeMs,
    ) -> Result<CoordinatorOutput, CoordinatorError> {
        let expected = self
            .config
            .account_for_symbol(&order.symbol)
            .map(|account| account.id.clone())
            .unwrap_or_default();
        if expected != account_id {
            return Err(CoordinatorError::WrongOrderAccount {
                order_id: client_order_id.to_string(),
                symbol: order.symbol,
                actual: account_id.to_string(),
                expected,
            });
        }
        self.regular_execution
            .validate_recovered_identity(account_id, &order.symbol)?;
        self.owned_regular_orders.register_recovered(
            account_id,
            &order.symbol,
            client_order_id,
            None,
        )?;
        let update = self.private_state_mut(account_id)?.register_local_order_at(
            client_order_id,
            order,
            ts_ms,
        );
        Ok(update
            .map(|update| self.process_normalized(NormalizedEvent::Order(update)))
            .unwrap_or_default())
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

    fn process_normalized(&mut self, event: NormalizedEvent) -> CoordinatorOutput {
        let observed_now_ms = event.ts_ms();
        self.process_normalized_at(event, observed_now_ms)
    }

    fn process_normalized_at(
        &mut self,
        event: NormalizedEvent,
        observed_now_ms: TimeMs,
    ) -> CoordinatorOutput {
        let strategy_references_were_ready = self.startup.strategy_references_ready();
        let account_halt = match &event {
            NormalizedEvent::System(system)
                if system.kind == SystemEventKind::AccountHalted
                    && system
                        .account_id
                        .as_deref()
                        .is_some_and(|account_id| self.private_states.contains_key(account_id)) =>
            {
                let account_id = system
                    .account_id
                    .clone()
                    .expect("checked account halt must have an account id");
                self.halted_accounts
                    .insert(account_id.clone(), system.reason.clone());
                Some((account_id, system.reason.clone()))
            }
            _ => None,
        };
        let order_transport_stale = match &event {
            NormalizedEvent::System(system)
                if system.kind == SystemEventKind::OrderTransportStale
                    && system
                        .account_id
                        .as_deref()
                        .is_some_and(|account_id| self.private_states.contains_key(account_id)) =>
            {
                Some((
                    system
                        .account_id
                        .clone()
                        .expect("checked order transport event must have an account id"),
                    system.reason.clone(),
                ))
            }
            _ => None,
        };
        if let NormalizedEvent::System(system) = &event {
            self.apply_system_to_startup(system);
        }
        match &event {
            NormalizedEvent::Market(market) => self
                .startup
                .observe_strategy_market(market, observed_now_ms),
            NormalizedEvent::Timer(_) => self.startup.refresh_strategy_references(observed_now_ms),
            NormalizedEvent::Order(_)
            | NormalizedEvent::Account(_)
            | NormalizedEvent::Control(_)
            | NormalizedEvent::System(_) => {}
        }
        let strategy_reference_readiness_lost =
            strategy_references_were_ready && !self.startup.strategy_references_ready();
        let now_ms = event.ts_ms();
        let mut records = vec![StorageRecord::Normalized(event.clone())];
        match &event {
            NormalizedEvent::Order(update) => records.push(StorageRecord::Order {
                account_id: self
                    .config
                    .account_for_symbol(&update.symbol)
                    .map(|account| account.id.clone()),
                update: update.clone(),
            }),
            NormalizedEvent::System(system) => records.push(StorageRecord::System(system.clone())),
            _ => {}
        }
        let sync_stablecoin_readiness = self.event_updates_stablecoin_readiness(&event);
        let engine_output = self.engine.on_chaos_event(event);
        if sync_stablecoin_readiness {
            self.sync_stablecoin_readiness(now_ms);
        }
        let mut output = CoordinatorOutput {
            actions: Vec::new(),
            records,
        };
        self.handle_engine_output(now_ms, engine_output, &mut output);
        if let Some((account_id, reason)) = account_halt {
            self.ensure_account_cancels(
                now_ms,
                &account_id,
                &format!("account {account_id} halted: {reason}"),
                &mut output,
            );
        }
        if let Some((account_id, reason)) = order_transport_stale {
            self.ensure_account_cancels(
                now_ms,
                &account_id,
                &format!("order transport stale for account {account_id}: {reason}"),
                &mut output,
            );
            output.actions.push(LiveAction::Reconcile(ReconcileAction {
                ts_ms: now_ms,
                account_id,
                reason: format!("order transport disconnected: {reason}"),
            }));
        }
        if strategy_reference_readiness_lost {
            let missing = self
                .startup
                .snapshot()
                .missing_strategy_references
                .join(", ");
            let account_ids = self
                .config
                .accounts
                .iter()
                .map(|account| account.id.clone())
                .collect::<Vec<_>>();
            for account_id in account_ids {
                self.ensure_account_cancels(
                    observed_now_ms,
                    &account_id,
                    &format!("strategy reference data stale: {missing}"),
                    &mut output,
                );
            }
        }
        output
    }

    fn event_updates_stablecoin_readiness(&self, event: &NormalizedEvent) -> bool {
        match event {
            NormalizedEvent::Timer(_) => !self.config.risk.stablecoin_guards.is_empty(),
            NormalizedEvent::Market(reap_core::MarketEvent::IndexPrice { symbol, .. }) => self
                .config
                .risk
                .stablecoin_guards
                .iter()
                .any(|guard| guard.symbol == *symbol),
            NormalizedEvent::System(system) => {
                system.kind == SystemEventKind::KillSwitchReset
                    && !self.config.risk.stablecoin_guards.is_empty()
            }
            NormalizedEvent::Order(_)
            | NormalizedEvent::Account(_)
            | NormalizedEvent::Control(_)
            | NormalizedEvent::Market(_) => false,
        }
    }

    fn sync_stablecoin_readiness(&mut self, now_ms: TimeMs) {
        let health = self.engine.risk().stablecoin_guard_health(now_ms);
        for guard in health {
            if let Err(error) =
                self.startup
                    .mark_stablecoin_rate(&guard.symbol, guard.healthy, guard.reason)
            {
                self.startup.mark_runtime_health(
                    "stablecoin_guard",
                    false,
                    format!("stablecoin readiness configuration mismatch: {error}"),
                );
            }
        }
    }

    fn handle_engine_output(
        &mut self,
        now_ms: TimeMs,
        engine_output: ChaosEngineOutput,
        output: &mut CoordinatorOutput,
    ) {
        for system in engine_output.system_events {
            if system.kind == SystemEventKind::RiskBreach {
                output
                    .records
                    .push(StorageRecord::SafetyLatch(SafetyLatchRecord {
                        ts_ms: system.ts_ms,
                        scope: SafetyLatchScope::Global,
                        active: true,
                        source: SafetyLatchSource::Risk,
                        request_id: None,
                        reason: system.reason.clone(),
                    }));
            }
            self.apply_system_to_startup(&system);
            output.records.push(StorageRecord::System(system));
        }
        for rejection in engine_output.rejected {
            let RiskDecision::Rejected { intent, reason } = rejection else {
                continue;
            };
            output.records.push(StorageRecord::IntentRejected {
                ts_ms: now_ms,
                intent,
                reason: format!("{reason:?}"),
            });
        }
        for intent in engine_output.intents {
            let legacy = intent.to_order_intent();
            output.records.push(StorageRecord::Intent {
                ts_ms: now_ms,
                intent: legacy.clone(),
            });
            self.route_chaos_intent(now_ms, intent, legacy, output);
        }
        for candidate in engine_output.safety_cancel_candidates {
            let legacy = candidate.to_order_intent();
            output.records.push(StorageRecord::Intent {
                ts_ms: now_ms,
                intent: legacy.clone(),
            });
            self.route_safety_cancel(now_ms, candidate, legacy, output);
        }
    }

    fn route_chaos_intent(
        &mut self,
        now_ms: TimeMs,
        intent: ChaosExecutionIntent,
        legacy: OrderIntent,
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
                let approved = match self.regular_execution.authorize_submit(intent) {
                    Ok(approved) => approved,
                    Err(error) => {
                        self.reject_execution_policy(now_ms, legacy, error, output);
                        return;
                    }
                };
                if approved.account_id() != account_id {
                    self.reject_execution_policy(
                        now_ms,
                        legacy,
                        RegularExecutionPolicyError::OwnerMismatch {
                            symbol: approved.order().symbol.clone(),
                            actual: approved.account_id().to_string(),
                            expected: account_id,
                        },
                        output,
                    );
                    return;
                }
                self.decision_sequence = self.decision_sequence.wrapping_add(1);
                let idempotency_key = format!(
                    "{}:{}:{}",
                    self.config.strategy.strategy_name, self.session_id, self.decision_sequence
                );
                let client_order_id = self
                    .client_ids
                    .get(&account_id)
                    .expect("validated account must have a client id generator")
                    .next(now_ms);
                let pending = match self.owned_regular_orders.reserve_local(
                    &approved,
                    &client_order_id,
                    self.private_states
                        .get_mut(&account_id)
                        .expect("validated account must have private state"),
                    now_ms,
                ) {
                    Ok(pending) => pending,
                    Err(error) => {
                        self.reject_execution_policy(now_ms, legacy, error, output);
                        return;
                    }
                };
                let (approved_account_id, order) = approved.into_parts();
                output.actions.push(LiveAction::Submit(SubmitAction {
                    ts_ms: now_ms,
                    account_id: approved_account_id,
                    idempotency_key,
                    client_order_id: client_order_id.clone(),
                    order: order.clone(),
                }));
                output.extend(self.process_normalized(NormalizedEvent::Order(pending)));
            }
            ChaosExecutionIntent::CancelOwned(cancel) => {
                let order_id = cancel.order_id().to_string();
                let reason = cancel.reason().to_string();
                self.route_cancel_owned(now_ms, &order_id, &reason, legacy, output);
            }
        }
    }

    fn route_safety_cancel(
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
        let (account_id, symbol, client_order_id, reason) = approved.into_parts();
        if !self.gateway_action_accounts.contains(&account_id) {
            output.records.push(StorageRecord::IntentRejected {
                ts_ms: now_ms,
                intent: legacy,
                reason: format!("account {account_id} has no planned regular order transport"),
            });
            return;
        }
        output.actions.push(LiveAction::Cancel(CancelAction {
            ts_ms: now_ms,
            account_id,
            symbol,
            client_order_id,
            reason,
        }));
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

    /// Raw serialized intents are evidence/backtest records, never live
    /// authority. This test-only seam proves that direct legacy injection is
    /// rejected instead of being promoted by field or reason inference.
    #[cfg(test)]
    fn route_intent(
        &mut self,
        now_ms: TimeMs,
        intent: OrderIntent,
        output: &mut CoordinatorOutput,
    ) {
        output.records.push(StorageRecord::IntentRejected {
            ts_ms: now_ms,
            intent,
            reason: "legacy serialized OrderIntent has no live execution authority".to_string(),
        });
    }

    fn ensure_account_cancels(
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
                LiveAction::Cancel(cancel) if cancel.account_id == account_id => {
                    Some(cancel.client_order_id.clone())
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
                    .get(order_id)
                    .is_some_and(|owned| owned.account_id() == account_id)
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

    pub fn active_order_count(&self) -> usize {
        self.private_states
            .values()
            .map(|state| {
                state
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
                    .count()
            })
            .sum()
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

    fn apply_system_to_startup(&mut self, event: &SystemEvent) {
        match event.kind {
            SystemEventKind::FeedHeartbeat | SystemEventKind::FeedRecovered => {
                if let Some(symbol) = event.symbol.as_deref() {
                    let _ = self.startup.mark_book(symbol, true, &event.reason);
                }
            }
            SystemEventKind::FeedStale
            | SystemEventKind::FeedGap
            | SystemEventKind::BookRecoveryStarted
            | SystemEventKind::BookRecoveryFailed => {
                if let Some(symbol) = event.symbol.as_deref() {
                    let _ = self.startup.mark_book(symbol, false, &event.reason);
                }
            }
            SystemEventKind::PrivateStreamHeartbeat | SystemEventKind::PrivateStreamRecovered => {
                if let Some(account_id) = event.account_id.as_deref() {
                    let _ = self
                        .startup
                        .mark_private_stream(account_id, true, &event.reason);
                }
            }
            SystemEventKind::PrivateStreamStale => {
                if let Some(account_id) = event.account_id.as_deref() {
                    let _ = self
                        .startup
                        .mark_private_stream(account_id, false, &event.reason);
                }
            }
            SystemEventKind::OrderTransportHeartbeat | SystemEventKind::OrderTransportRecovered => {
                if let Some(account_id) = event.account_id.as_deref() {
                    let _ = self
                        .startup
                        .mark_order_transport(account_id, true, &event.reason);
                }
            }
            SystemEventKind::OrderTransportStale => {
                if let Some(account_id) = event.account_id.as_deref() {
                    let _ = self
                        .startup
                        .mark_order_transport(account_id, false, &event.reason);
                    let _ = self
                        .startup
                        .mark_reconciled(account_id, false, &event.reason);
                }
            }
            SystemEventKind::ReconcileDrift => {
                if let Some(account_id) = event.account_id.as_deref() {
                    let _ = self
                        .startup
                        .mark_reconciled(account_id, false, &event.reason);
                }
            }
            SystemEventKind::RiskBreach | SystemEventKind::KillSwitchActivated => {
                self.startup
                    .mark_runtime_health("risk", false, &event.reason);
            }
            SystemEventKind::KillSwitchReset => {
                self.startup
                    .mark_runtime_health("risk", true, &event.reason);
            }
            SystemEventKind::AccountHalted
            | SystemEventKind::SymbolHalted
            | SystemEventKind::SymbolResumed => {}
        }
    }

    fn require_account_id(&self, account_id: Option<String>) -> Result<String, CoordinatorError> {
        let account_id = account_id.ok_or(CoordinatorError::MissingAccountIdentity)?;
        if self.private_states.contains_key(&account_id) {
            Ok(account_id)
        } else {
            Err(CoordinatorError::UnknownAccount(account_id))
        }
    }

    fn private_state_mut(
        &mut self,
        account_id: &str,
    ) -> Result<&mut PrivateStateReducer, CoordinatorError> {
        self.private_states
            .get_mut(account_id)
            .ok_or_else(|| CoordinatorError::UnknownAccount(account_id.to_string()))
    }
}

fn scope_account_update(account_id: &str, update: &mut reap_core::AccountUpdate) {
    for balance in &mut update.balances {
        balance.account_id = Some(account_id.to_string());
    }
    for margin in &mut update.margins {
        margin.account_id = Some(account_id.to_string());
    }
}

#[cfg(test)]
mod tests {
    use std::collections::{HashMap, HashSet};

    use reap_core::{
        AccountUpdate, Balance, FillFee, FillKey, FillLiquidity, Level, MarketEvent, NewOrder,
        OrderBook, OrderEvent, OrderStatus, OrderUpdate, Position, PositionMarginMode, Side,
        SystemEvent, SystemEventKind, TimeInForce, TimeMs, Venue,
    };
    use reap_feed::FeedOutput;
    use reap_order::{ReconcileIssue, reconcile, reconcile_full_state};
    use reap_risk::{
        InstrumentOrderLimits, InstrumentRiskModel, RiskDecision, RiskLimits, RiskRejectReason,
        StablecoinGuardConfig,
    };
    use reap_strategy::{ChaosConfig, ReferenceDataKind};
    use reap_venue::okx::{OkxAccountLevel, OkxInstrumentType, OkxPositionMode};
    use reap_venue::{PrivateOrderState, PrivateOrderUpdate, RemoteFill, RemoteOrder};

    use crate::forbidden_orders::{ForbiddenOrderEvent, ForbiddenOrderState};
    use crate::{
        LiveAccountConfig, LiveStorageConfig, OkxTradeModeConfig, OkxVenueConfig, RuntimeConfig,
        VerifiedInstrument,
    };

    use super::*;

    fn coordinator_with_gateway_actions(gateway_actions_enabled: bool) -> LiveCoordinator {
        coordinator_with_risk(
            gateway_actions_enabled,
            RiskLimits {
                require_feed_health: false,
                require_private_health: false,
                ..RiskLimits::default()
            },
        )
    }

    fn coordinator_with_risk(gateway_actions_enabled: bool, risk: RiskLimits) -> LiveCoordinator {
        let mut strategy: ChaosConfig =
            toml::from_str(include_str!("../../../examples/iarb2-basic.toml")).unwrap();
        strategy.reference_data_stale_threshold_ms = Some(120_000);
        strategy.risk_groups[0].account_id = Some("main".to_string());
        let config = LiveConfig {
            strategy,
            risk,
            venue: OkxVenueConfig::default(),
            runtime: RuntimeConfig::default(),
            storage: LiveStorageConfig::default(),
            operator: crate::OperatorConfig::default(),
            alerts: crate::AlertConfig::default(),
            host_guard: crate::HostGuardConfig::default(),
            accounts: vec![LiveAccountConfig {
                id: "main".to_string(),
                api_key_env: "KEY".to_string(),
                secret_key_env: "SECRET".to_string(),
                passphrase_env: "PASS".to_string(),
                expected_account_level: OkxAccountLevel::SingleCurrencyMargin,
                expected_position_mode: OkxPositionMode::NetMode,
                api_key_policy: crate::OkxApiKeyPolicyConfig::default(),
                id_prefix: "reap".to_string(),
                node_id: 1,
                trade_modes: HashMap::from([
                    ("BTC-USDT".to_string(), OkxTradeModeConfig::Cash),
                    ("BTC-PERP".to_string(), OkxTradeModeConfig::Cross),
                ]),
            }],
        };
        let verified = VerifiedBootstrap {
            instruments: HashMap::from([
                (
                    "BTC-USDT".to_string(),
                    VerifiedInstrument {
                        account_id: "main".to_string(),
                        symbol: "BTC-USDT".to_string(),
                        instrument_type: OkxInstrumentType::Spot,
                        trade_mode: OkxTradeModeConfig::Cash,
                        risk_model: InstrumentRiskModel::Spot,
                        order_limits: InstrumentOrderLimits {
                            max_limit_quantity: 100.0,
                            max_limit_notional_usd: Some(1_000_000.0),
                        },
                        tick_size: 0.1,
                        lot_size: 0.0001,
                        min_size: 0.0001,
                        contract_value: None,
                    },
                ),
                (
                    "BTC-PERP".to_string(),
                    VerifiedInstrument {
                        account_id: "main".to_string(),
                        symbol: "BTC-PERP".to_string(),
                        instrument_type: OkxInstrumentType::Futures,
                        trade_mode: OkxTradeModeConfig::Cross,
                        risk_model: InstrumentRiskModel::LinearDerivative {
                            contract_value: 0.001,
                        },
                        order_limits: InstrumentOrderLimits {
                            max_limit_quantity: 1_000_000.0,
                            max_limit_notional_usd: None,
                        },
                        tick_size: 0.1,
                        lot_size: 1.0,
                        min_size: 1.0,
                        contract_value: Some(0.001),
                    },
                ),
            ]),
            account_updates: HashMap::from([(
                "main".to_string(),
                AccountUpdate {
                    ts_ms: 1,
                    balances: vec![Balance {
                        account_id: Some("main".to_string()),
                        currency: "USDT".to_string(),
                        total: 10_000.0,
                        available: 10_000.0,
                        equity: 10_000.0,
                        liability: 0.0,
                        max_loan: 0.0,
                        forced_repayment_indicator: None,
                    }],
                    positions: Vec::new(),
                    margins: Vec::new(),
                },
            )]),
            baseline_fill_ids: HashMap::from([("main".to_string(), HashSet::new())]),
            quote_stp_verified_accounts: HashSet::from(["main".to_string()]),
        };
        LiveCoordinator::new(config, verified, gateway_actions_enabled, "test-session").unwrap()
    }

    fn coordinator() -> LiveCoordinator {
        coordinator_with_gateway_actions(true)
    }

    fn two_account_coordinator() -> LiveCoordinator {
        let mut config = coordinator().config.clone();
        let mut hedge_group = config.strategy.risk_groups[0].clone();
        config.strategy.risk_groups[0].symbols = vec!["BTC-USDT".to_string()];
        hedge_group.name = "hedge".to_string();
        hedge_group.account_id = Some("hedge".to_string());
        hedge_group.symbols = vec!["BTC-PERP".to_string()];
        config.strategy.risk_groups.push(hedge_group);
        config.strategy.instruments[1].risk_group = "hedge".to_string();
        config.accounts[0].trade_modes.remove("BTC-PERP");
        config.accounts.push(LiveAccountConfig {
            id: "hedge".to_string(),
            api_key_env: "HEDGE_KEY".to_string(),
            secret_key_env: "HEDGE_SECRET".to_string(),
            passphrase_env: "HEDGE_PASS".to_string(),
            expected_account_level: OkxAccountLevel::SingleCurrencyMargin,
            expected_position_mode: OkxPositionMode::NetMode,
            api_key_policy: crate::OkxApiKeyPolicyConfig::default(),
            id_prefix: "hedge".to_string(),
            node_id: 2,
            trade_modes: HashMap::from([("BTC-PERP".to_string(), OkxTradeModeConfig::Cross)]),
        });
        let verified = VerifiedBootstrap {
            instruments: HashMap::from([
                (
                    "BTC-USDT".to_string(),
                    VerifiedInstrument {
                        account_id: "main".to_string(),
                        symbol: "BTC-USDT".to_string(),
                        instrument_type: OkxInstrumentType::Spot,
                        trade_mode: OkxTradeModeConfig::Cash,
                        risk_model: InstrumentRiskModel::Spot,
                        order_limits: InstrumentOrderLimits {
                            max_limit_quantity: 100.0,
                            max_limit_notional_usd: Some(1_000_000.0),
                        },
                        tick_size: 0.1,
                        lot_size: 0.0001,
                        min_size: 0.0001,
                        contract_value: None,
                    },
                ),
                (
                    "BTC-PERP".to_string(),
                    VerifiedInstrument {
                        account_id: "hedge".to_string(),
                        symbol: "BTC-PERP".to_string(),
                        instrument_type: OkxInstrumentType::Futures,
                        trade_mode: OkxTradeModeConfig::Cross,
                        risk_model: InstrumentRiskModel::LinearDerivative {
                            contract_value: 0.001,
                        },
                        order_limits: InstrumentOrderLimits {
                            max_limit_quantity: 1_000_000.0,
                            max_limit_notional_usd: None,
                        },
                        tick_size: 0.1,
                        lot_size: 1.0,
                        min_size: 1.0,
                        contract_value: Some(0.001),
                    },
                ),
            ]),
            account_updates: HashMap::from([
                ("main".to_string(), account_update("main", 1)),
                ("hedge".to_string(), account_update("hedge", 1)),
            ]),
            baseline_fill_ids: HashMap::from([
                ("main".to_string(), HashSet::new()),
                ("hedge".to_string(), HashSet::new()),
            ]),
            quote_stp_verified_accounts: HashSet::from(["main".to_string(), "hedge".to_string()]),
        };
        LiveCoordinator::new(config, verified, true, "two-account-test").unwrap()
    }

    fn account_update(account_id: &str, ts_ms: TimeMs) -> AccountUpdate {
        AccountUpdate {
            ts_ms,
            balances: vec![Balance {
                account_id: Some(account_id.to_string()),
                currency: "USDT".to_string(),
                total: 10_000.0,
                available: 10_000.0,
                equity: 10_000.0,
                liability: 0.0,
                max_loan: 0.0,
                forced_repayment_indicator: None,
            }],
            positions: Vec::new(),
            margins: Vec::new(),
        }
    }

    fn bootstrap_readiness(coordinator: &mut LiveCoordinator) {
        coordinator.mark_storage_ready(true, "open");
        coordinator.mark_public_connectivity(true, "connected");
        coordinator
            .process_feed(FeedOutput::PrivateAccount {
                account_id: Some("main".to_string()),
                update: AccountUpdate {
                    ts_ms: 1,
                    balances: vec![Balance {
                        account_id: Some("main".to_string()),
                        currency: "USDT".to_string(),
                        total: 10_000.0,
                        available: 10_000.0,
                        equity: 10_000.0,
                        liability: 0.0,
                        max_loan: 0.0,
                        forced_repayment_indicator: None,
                    }],
                    positions: Vec::new(),
                    margins: Vec::new(),
                },
            })
            .unwrap();
        coordinator
            .on_reconciliation(ReconciliationResult {
                account_id: "main".to_string(),
                ts_ms: 2,
                clean: true,
                local_live_orders: 0,
                remote_live_orders: 0,
                remote_recent_fills: 0,
                reason: "clean".to_string(),
            })
            .unwrap();
        for symbol in ["BTC-USDT", "BTC-PERP"] {
            coordinator.process_event(NormalizedEvent::System(SystemEvent {
                ts_ms: 2,
                kind: SystemEventKind::FeedRecovered,
                venue: Some(Venue::Okx),
                account_id: None,
                symbol: Some(symbol.to_string()),
                reason: "snapshot".to_string(),
            }));
        }
        coordinator.process_event(NormalizedEvent::System(SystemEvent {
            ts_ms: 2,
            kind: SystemEventKind::PrivateStreamRecovered,
            venue: Some(Venue::Okx),
            account_id: Some("main".to_string()),
            symbol: None,
            reason: "connected".to_string(),
        }));
        coordinator.process_event(NormalizedEvent::System(SystemEvent {
            ts_ms: 2,
            kind: SystemEventKind::OrderTransportRecovered,
            venue: Some(Venue::Okx),
            account_id: Some("main".to_string()),
            symbol: None,
            reason: "all sessions authenticated".to_string(),
        }));
        coordinator
            .startup
            .mark_forbidden_order_proof("main", true, "complete zero proof")
            .unwrap();
        seed_strategy_references(coordinator, 2);
    }

    fn seed_strategy_references(coordinator: &mut LiveCoordinator, ts_ms: TimeMs) {
        let requirements = coordinator.config.strategy.reference_data_requirements();
        for requirement in requirements {
            let event = match requirement.kind {
                ReferenceDataKind::IndexPrice => MarketEvent::IndexPrice {
                    ts_ms,
                    symbol: requirement.symbol,
                    price: 100.0,
                },
                ReferenceDataKind::FundingRate => MarketEvent::FundingRate {
                    ts_ms,
                    symbol: requirement.symbol,
                    rate: 0.0001,
                    funding_time_ms: ts_ms + 28_800_000,
                    settlement: None,
                },
                ReferenceDataKind::MarkPrice => MarketEvent::PriceLimits {
                    ts_ms,
                    symbol: requirement.symbol,
                    mark_price: 100.0,
                    limit_down: 0.0,
                    limit_up: 0.0,
                },
                ReferenceDataKind::PriceLimits => MarketEvent::PriceLimits {
                    ts_ms,
                    symbol: requirement.symbol,
                    mark_price: 0.0,
                    limit_down: 50.0,
                    limit_up: 150.0,
                },
            };
            coordinator.process_event(NormalizedEvent::Market(event));
        }
    }

    fn ready(coordinator: &mut LiveCoordinator) {
        bootstrap_readiness(coordinator);
        assert!(coordinator.readiness().is_ready());
    }

    fn ready_two_accounts(coordinator: &mut LiveCoordinator) {
        coordinator.mark_storage_ready(true, "open");
        coordinator.mark_public_connectivity(true, "connected");
        for account_id in ["main", "hedge"] {
            coordinator
                .process_feed(FeedOutput::PrivateAccount {
                    account_id: Some(account_id.to_string()),
                    update: account_update(account_id, 1),
                })
                .unwrap();
            coordinator
                .on_reconciliation(ReconciliationResult {
                    account_id: account_id.to_string(),
                    ts_ms: 2,
                    clean: true,
                    local_live_orders: 0,
                    remote_live_orders: 0,
                    remote_recent_fills: 0,
                    reason: "clean".to_string(),
                })
                .unwrap();
            coordinator
                .startup
                .mark_forbidden_order_proof(account_id, true, "complete zero proof")
                .unwrap();
        }
        for symbol in ["BTC-USDT", "BTC-PERP"] {
            coordinator.process_event(NormalizedEvent::System(SystemEvent {
                ts_ms: 2,
                kind: SystemEventKind::FeedRecovered,
                venue: Some(Venue::Okx),
                account_id: None,
                symbol: Some(symbol.to_string()),
                reason: "snapshot".to_string(),
            }));
        }
        for account_id in ["main", "hedge"] {
            coordinator.process_event(NormalizedEvent::System(SystemEvent {
                ts_ms: 2,
                kind: SystemEventKind::PrivateStreamRecovered,
                venue: Some(Venue::Okx),
                account_id: Some(account_id.to_string()),
                symbol: None,
                reason: "connected".to_string(),
            }));
            coordinator.process_event(NormalizedEvent::System(SystemEvent {
                ts_ms: 2,
                kind: SystemEventKind::OrderTransportRecovered,
                venue: Some(Venue::Okx),
                account_id: Some(account_id.to_string()),
                symbol: None,
                reason: "all sessions authenticated".to_string(),
            }));
        }
        seed_strategy_references(coordinator, 2);
        assert!(coordinator.readiness().is_ready());
    }

    fn order() -> NewOrder {
        NewOrder {
            symbol: "BTC-USDT".to_string(),
            side: Side::Buy,
            qty: 0.1,
            price: 100.0,
            time_in_force: TimeInForce::PostOnly,
            reduce_only: false,
            self_trade_prevention: None,
            reason: "quote".to_string(),
        }
    }

    fn cancelled_private_order(
        client_order_id: &str,
        exchange_order_id: &str,
        ts_ms: TimeMs,
    ) -> PrivateOrderUpdate {
        PrivateOrderUpdate {
            ts_ms,
            exchange_order_id: exchange_order_id.to_string(),
            client_order_id: client_order_id.to_string(),
            symbol: "BTC-USDT".to_string(),
            side: Side::Buy,
            state: PrivateOrderState::Cancelled,
            price: 100.0,
            qty: 0.1,
            cumulative_filled_qty: 0.0,
            average_fill_price: 0.0,
            last_fill_qty: 0.0,
            last_fill_price: 0.0,
            liquidity: None,
            last_fill_fee: None,
            fill_id: None,
            reject_reason: String::new(),
        }
    }

    #[test]
    fn account_snapshot_is_ready_only_after_the_engine_consumes_it() {
        let mut coordinator = coordinator();
        assert_eq!(
            coordinator.readiness().missing_account_snapshots,
            vec!["main".to_string()]
        );
        assert!(
            coordinator
                .private_state("main")
                .is_some_and(|state| !state.balances().is_empty())
        );

        coordinator.mark_storage_ready(true, "open");
        coordinator.mark_public_connectivity(true, "connected");
        coordinator
            .on_reconciliation(ReconciliationResult {
                account_id: "main".to_string(),
                ts_ms: 2,
                clean: true,
                local_live_orders: 0,
                remote_live_orders: 0,
                remote_recent_fills: 0,
                reason: "clean".to_string(),
            })
            .unwrap();
        for symbol in ["BTC-USDT", "BTC-PERP"] {
            coordinator.process_event(NormalizedEvent::System(SystemEvent {
                ts_ms: 2,
                kind: SystemEventKind::FeedRecovered,
                venue: Some(Venue::Okx),
                account_id: None,
                symbol: Some(symbol.to_string()),
                reason: "snapshot".to_string(),
            }));
        }
        coordinator.process_event(NormalizedEvent::System(SystemEvent {
            ts_ms: 2,
            kind: SystemEventKind::PrivateStreamRecovered,
            venue: Some(Venue::Okx),
            account_id: Some("main".to_string()),
            symbol: None,
            reason: "connected".to_string(),
        }));
        coordinator.process_event(NormalizedEvent::System(SystemEvent {
            ts_ms: 2,
            kind: SystemEventKind::OrderTransportRecovered,
            venue: Some(Venue::Okx),
            account_id: Some("main".to_string()),
            symbol: None,
            reason: "all sessions authenticated".to_string(),
        }));
        coordinator
            .startup
            .mark_forbidden_order_proof("main", true, "complete zero proof")
            .unwrap();
        seed_strategy_references(&mut coordinator, 2);

        assert!(!coordinator.readiness().is_ready());
        assert_eq!(coordinator.readiness().phase, crate::LivePhase::Reconciling);

        coordinator
            .process_feed(FeedOutput::PrivateAccount {
                account_id: Some("main".to_string()),
                update: account_update("main", 3),
            })
            .unwrap();

        assert!(coordinator.readiness().is_ready());
        assert!(coordinator.readiness().missing_account_snapshots.is_empty());
    }

    #[test]
    fn position_margin_mode_drift_is_rejected_before_state_application() {
        let mut coordinator = coordinator();
        let wrong_mode = AccountUpdate {
            ts_ms: 3,
            balances: Vec::new(),
            positions: vec![Position {
                symbol: "BTC-PERP".to_string(),
                qty: 2.0,
                avg_price: 50_000.0,
                margin_mode: Some(PositionMarginMode::Isolated),
            }],
            margins: Vec::new(),
        };

        let error = coordinator
            .process_feed(FeedOutput::PrivateAccount {
                account_id: Some("main".to_string()),
                update: wrong_mode.clone(),
            })
            .unwrap_err();

        assert!(matches!(
            error,
            CoordinatorError::AccountStatePolicy { ref account_id, ref message }
                if account_id == "main"
                    && message.contains("BTC-PERP expected Cross, received Isolated")
        ));
        assert!(
            coordinator
                .private_state("main")
                .unwrap()
                .positions()
                .is_empty()
        );
        assert!(matches!(
            coordinator.apply_authoritative_account_snapshot("main", wrong_mode),
            Err(CoordinatorError::AccountStatePolicy { .. })
        ));

        coordinator
            .process_feed(FeedOutput::PrivateAccount {
                account_id: Some("main".to_string()),
                update: AccountUpdate {
                    ts_ms: 4,
                    balances: Vec::new(),
                    positions: vec![Position {
                        symbol: "BTC-PERP".to_string(),
                        qty: 0.0,
                        avg_price: 0.0,
                        margin_mode: Some(PositionMarginMode::Isolated),
                    }],
                    margins: Vec::new(),
                },
            })
            .unwrap();

        let error = coordinator
            .process_feed(FeedOutput::PrivateAccount {
                account_id: Some("main".to_string()),
                update: AccountUpdate {
                    ts_ms: 5,
                    balances: Vec::new(),
                    positions: vec![Position {
                        symbol: "ETH-USDT-SWAP".to_string(),
                        qty: 1.0,
                        avg_price: 3_000.0,
                        margin_mode: Some(PositionMarginMode::Cross),
                    }],
                    margins: Vec::new(),
                },
            })
            .unwrap_err();
        assert!(matches!(
            error,
            CoordinatorError::AccountStatePolicy { ref message, .. }
                if message.contains("unmanaged nonzero position ETH-USDT-SWAP qty=1")
        ));

        coordinator
            .process_feed(FeedOutput::PrivateAccount {
                account_id: Some("main".to_string()),
                update: AccountUpdate {
                    ts_ms: 6,
                    balances: Vec::new(),
                    positions: vec![Position {
                        symbol: "ETH-USDT-SWAP".to_string(),
                        qty: 0.0,
                        avg_price: 0.0,
                        margin_mode: Some(PositionMarginMode::Isolated),
                    }],
                    margins: Vec::new(),
                },
            })
            .unwrap();

        let mut borrowed = account_update("main", 7);
        borrowed.balances[0].liability = 0.01;
        let error = coordinator
            .process_feed(FeedOutput::PrivateAccount {
                account_id: Some("main".to_string()),
                update: borrowed,
            })
            .unwrap_err();
        assert!(matches!(
            error,
            CoordinatorError::AccountStatePolicy { ref message, .. }
                if message.contains("liability 0.01 is nonzero")
        ));

        let mut forced_repayment = account_update("main", 8);
        forced_repayment.balances[0].total = 9_000.0;
        forced_repayment.balances[0].forced_repayment_indicator = Some(1);
        let error = coordinator
            .process_feed(FeedOutput::PrivateAccount {
                account_id: Some("main".to_string()),
                update: forced_repayment,
            })
            .unwrap_err();
        assert!(matches!(
            error,
            CoordinatorError::AccountStatePolicy { ref message, .. }
                if message.contains(
                    "currency USDT forced repayment indicator 1 reached limit 1"
                )
        ));
        assert_eq!(
            coordinator.private_state("main").unwrap().balances()["USDT"].total,
            10_000.0
        );
    }

    #[test]
    fn post_connect_reconciliation_blocks_readiness_until_clean() {
        let mut coordinator = coordinator();
        ready(&mut coordinator);

        let output = coordinator
            .require_reconciliation("main", 3, "private stream connected")
            .unwrap();

        assert!(!coordinator.readiness().is_ready());
        assert!(
            coordinator
                .readiness()
                .missing_reconciliation
                .contains(&"main".to_string())
        );
        assert!(output.actions.iter().any(
            |action| matches!(action, LiveAction::Reconcile(action) if action.account_id == "main")
        ));

        coordinator
            .on_reconciliation(ReconciliationResult {
                account_id: "main".to_string(),
                ts_ms: 4,
                clean: true,
                local_live_orders: 0,
                remote_live_orders: 0,
                remote_recent_fills: 0,
                reason: "post-connect REST state is clean".to_string(),
            })
            .unwrap();
        assert!(coordinator.readiness().is_ready());
    }

    #[test]
    fn retained_reference_frame_is_aged_and_cancelled_while_already_degraded() {
        let mut coordinator = coordinator();
        ready(&mut coordinator);
        coordinator
            .register_local_order("main", "reference-q1", order(), 3)
            .unwrap();
        coordinator
            .startup
            .mark_runtime_health("test_fault", false, "already degraded");
        assert_eq!(coordinator.readiness().phase, crate::LivePhase::Degraded);
        assert!(coordinator.startup.strategy_references_ready());

        let output = coordinator
            .process_feed_at(
                FeedOutput::Event(NormalizedEvent::Market(MarketEvent::PriceLimits {
                    ts_ms: 3,
                    symbol: "BTC-USDT".to_string(),
                    mark_price: 0.0,
                    limit_down: 50.0,
                    limit_up: 150.0,
                })),
                120_004,
            )
            .unwrap();

        let readiness = coordinator.readiness();
        assert_eq!(readiness.phase, crate::LivePhase::Degraded);
        assert!(
            readiness
                .missing_strategy_references
                .contains(&"price_limits:BTC-USDT".to_string())
        );
        assert!(
            readiness
                .faults
                .contains_key("strategy_reference:price_limits:BTC-USDT")
        );
        assert!(output.actions.iter().any(|action| {
            matches!(action, LiveAction::Cancel(cancel) if cancel.client_order_id == "reference-q1")
        }));
    }

    #[test]
    fn order_transport_loss_blocks_entry_cancels_and_requires_reconciliation() {
        let mut coordinator = coordinator();
        ready(&mut coordinator);
        coordinator
            .register_local_order("main", "client-1", order(), 3)
            .unwrap();

        let output = coordinator.process_event(NormalizedEvent::System(SystemEvent {
            ts_ms: 4,
            kind: SystemEventKind::OrderTransportStale,
            venue: Some(Venue::Okx),
            account_id: Some("main".to_string()),
            symbol: None,
            reason: "session 3 disconnected".to_string(),
        }));

        let readiness = coordinator.readiness();
        assert_eq!(readiness.phase, crate::LivePhase::Degraded);
        assert_eq!(readiness.missing_order_transports, vec!["main"]);
        assert_eq!(readiness.missing_reconciliation, vec!["main"]);
        assert!(output.actions.iter().any(|action| matches!(
            action,
            LiveAction::Cancel(cancel) if cancel.client_order_id == "client-1"
        )));
        assert!(output.actions.iter().any(|action| matches!(
            action,
            LiveAction::Reconcile(reconcile) if reconcile.account_id == "main"
        )));

        let mut blocked = CoordinatorOutput::default();
        coordinator.route_intent(5, OrderIntent::NewOrder(order()), &mut blocked);
        assert!(blocked.actions.is_empty());
    }

    #[test]
    fn authoritative_reconciliation_clears_closed_position_before_clean_retry() {
        let mut coordinator = coordinator();
        ready(&mut coordinator);
        coordinator
            .process_feed(FeedOutput::PrivateAccount {
                account_id: Some("main".to_string()),
                update: AccountUpdate {
                    ts_ms: 3,
                    balances: account_update("main", 3).balances,
                    positions: vec![Position {
                        symbol: "BTC-PERP".to_string(),
                        qty: 4.0,
                        avg_price: 50_000.0,
                        margin_mode: Some(PositionMarginMode::Cross),
                    }],
                    margins: Vec::new(),
                },
            })
            .unwrap();
        assert_eq!(
            coordinator
                .engine
                .strategy()
                .entity("BTC-PERP")
                .unwrap()
                .position_qty,
            4.0
        );

        coordinator
            .require_reconciliation("main", 4, "private stream recovered")
            .unwrap();
        let remote_account = account_update("main", 5);
        let first = reconcile_full_state(
            coordinator.private_state("main").unwrap(),
            &[],
            &[],
            &remote_account,
        );
        assert!(first.issues.iter().any(|issue| matches!(
            issue,
            ReconcileIssue::PositionMissingRemote { symbol, .. } if symbol == "BTC-PERP"
        )));

        let snapshot_output = coordinator
            .apply_authoritative_account_snapshot("main", remote_account.clone())
            .unwrap();
        assert!(matches!(
            snapshot_output.records.first(),
            Some(StorageRecord::AccountSnapshot(snapshot))
                if snapshot.account_id == "main"
                    && snapshot.ts_ms == 5
                    && snapshot.update.positions.iter().any(|position|
                        position.symbol == "BTC-PERP" && position.qty == 0.0)
        ));
        assert!(
            coordinator
                .private_state("main")
                .unwrap()
                .positions()
                .is_empty()
        );
        assert_eq!(
            coordinator
                .engine
                .strategy()
                .entity("BTC-PERP")
                .unwrap()
                .position_qty,
            0.0
        );
        let stale = coordinator
            .process_feed(FeedOutput::PrivateAccount {
                account_id: Some("main".to_string()),
                update: AccountUpdate {
                    ts_ms: 4,
                    balances: Vec::new(),
                    positions: vec![Position {
                        symbol: "BTC-PERP".to_string(),
                        qty: 8.0,
                        avg_price: 49_000.0,
                        margin_mode: Some(PositionMarginMode::Cross),
                    }],
                    margins: Vec::new(),
                },
            })
            .unwrap();
        assert!(stale.records.is_empty());
        assert_eq!(
            coordinator
                .engine
                .strategy()
                .entity("BTC-PERP")
                .unwrap()
                .position_qty,
            0.0
        );
        coordinator
            .on_reconciliation(ReconciliationResult {
                account_id: "main".to_string(),
                ts_ms: 5,
                clean: first.is_clean(),
                local_live_orders: first.local_live_orders,
                remote_live_orders: first.remote_live_orders,
                remote_recent_fills: first.remote_fills,
                reason: format!("{:?}", first.issues),
            })
            .unwrap();
        assert_eq!(
            coordinator.readiness().missing_reconciliation,
            vec!["main".to_string()]
        );

        let second = reconcile_full_state(
            coordinator.private_state("main").unwrap(),
            &[],
            &[],
            &remote_account,
        );
        assert!(second.is_clean());
        coordinator
            .on_reconciliation(ReconciliationResult {
                account_id: "main".to_string(),
                ts_ms: 6,
                clean: true,
                local_live_orders: 0,
                remote_live_orders: 0,
                remote_recent_fills: 0,
                reason: "second authoritative pass is clean".to_string(),
            })
            .unwrap();
        assert!(coordinator.readiness().missing_reconciliation.is_empty());
    }

    #[test]
    fn authenticated_instrument_order_limits_seed_live_pre_trade_risk() {
        let coordinator = coordinator();
        let mut oversized = order();
        oversized.qty = 101.0;

        assert!(matches!(
            coordinator
                .engine
                .risk()
                .pre_trade(1, OrderIntent::NewOrder(oversized)),
            RiskDecision::Rejected {
                reason: RiskRejectReason::InstrumentOrderQuantity {
                    symbol,
                    value: 101.0,
                    limit: 100.0,
                },
                ..
            } if symbol == "BTC-USDT"
        ));

        let mut oversized_amount = order();
        oversized_amount.qty = 1.0;
        oversized_amount.price = 1_000_001.0;
        assert!(matches!(
            coordinator
                .engine
                .risk()
                .pre_trade(2, OrderIntent::NewOrder(oversized_amount)),
            RiskDecision::Rejected {
                reason: RiskRejectReason::InstrumentOrderNotional {
                    value: 1_000_001.0,
                    limit: 1_000_000.0,
                    ..
                },
                ..
            }
        ));
    }

    #[test]
    fn stablecoin_breach_blocks_entry_then_latches_and_cancels() {
        let mut coordinator = coordinator_with_risk(
            true,
            RiskLimits {
                require_feed_health: false,
                require_private_health: false,
                stablecoin_guards: vec![StablecoinGuardConfig {
                    symbol: "USDT-USD".to_string(),
                    max_downside_deviation: 0.01,
                }],
                stablecoin_max_age_ms: 10_000,
                stablecoin_breach_debounce_ms: 5_000,
                ..RiskLimits::default()
            },
        );
        bootstrap_readiness(&mut coordinator);
        assert_eq!(
            coordinator.readiness().phase,
            crate::LivePhase::AwaitingStreams
        );
        assert_eq!(
            coordinator.readiness().missing_stablecoin_rates,
            vec!["USDT-USD".to_string()]
        );

        coordinator.process_event(NormalizedEvent::Market(MarketEvent::IndexPrice {
            ts_ms: 2,
            symbol: "USDT-USD".to_string(),
            price: 1.0,
        }));
        assert!(coordinator.readiness().is_ready());
        coordinator
            .register_local_order("main", "live-1", order(), 3)
            .unwrap();

        let transient =
            coordinator.process_event(NormalizedEvent::Market(MarketEvent::IndexPrice {
                ts_ms: 10,
                symbol: "USDT-USD".to_string(),
                price: 0.98,
            }));
        assert_eq!(coordinator.readiness().phase, crate::LivePhase::Degraded);
        assert!(!coordinator.kill_switch_active());
        assert!(
            !transient
                .records
                .iter()
                .any(|record| { matches!(record, StorageRecord::SafetyLatch(_)) })
        );
        assert!(matches!(
            coordinator
                .engine
                .risk()
                .pre_trade(10, OrderIntent::NewOrder(order())),
            RiskDecision::Rejected {
                reason: RiskRejectReason::StablecoinDepeg { .. },
                ..
            }
        ));
        let mut blocked = CoordinatorOutput::default();
        coordinator.route_intent(10, OrderIntent::NewOrder(order()), &mut blocked);
        assert!(blocked.actions.is_empty());

        let latched = coordinator.process_event(NormalizedEvent::Market(MarketEvent::IndexPrice {
            ts_ms: 5_010,
            symbol: "USDT-USD".to_string(),
            price: 0.98,
        }));
        assert!(coordinator.kill_switch_active());
        assert!(latched.records.iter().any(|record| {
            matches!(
                record,
                StorageRecord::SafetyLatch(latch)
                    if latch.active
                        && latch.scope == SafetyLatchScope::Global
                        && latch.source == SafetyLatchSource::Risk
            )
        }));
        assert!(latched.actions.iter().any(|action| {
            matches!(
                action,
                LiveAction::Cancel(cancel) if cancel.client_order_id == "live-1"
            )
        }));
    }

    #[test]
    fn registration_is_canonical_before_rest_result() {
        let mut coordinator = coordinator();
        ready(&mut coordinator);
        let output = coordinator
            .register_local_order("main", "client-1", order(), 3)
            .unwrap();

        assert_eq!(
            coordinator
                .private_state("main")
                .unwrap()
                .order_reducer()
                .get("client-1")
                .unwrap()
                .status,
            OrderStatus::PendingNew
        );
        assert!(output.records.iter().any(
            |record| matches!(record, StorageRecord::Order { update, .. } if update.order_id == "client-1")
        ));
    }

    #[test]
    fn explicit_submit_failure_is_terminal_but_ambiguous_failure_degrades() {
        let mut explicit = coordinator();
        ready(&mut explicit);
        explicit
            .register_local_order("main", "client-1", order(), 3)
            .unwrap();
        explicit
            .on_submit_error("main", "client-1", 4, false, "rejected")
            .unwrap();
        assert_eq!(
            explicit
                .private_state("main")
                .unwrap()
                .order_reducer()
                .get("client-1")
                .unwrap()
                .status,
            OrderStatus::Rejected
        );

        let mut ambiguous = coordinator();
        ready(&mut ambiguous);
        ambiguous
            .register_local_order("main", "client-2", order(), 3)
            .unwrap();
        let output = ambiguous
            .on_submit_error("main", "client-2", 4, true, "timeout")
            .unwrap();
        assert!(!ambiguous.readiness().is_ready());
        assert!(output.actions.iter().any(
            |action| matches!(action, LiveAction::Reconcile(action) if action.account_id == "main")
        ));
    }

    #[test]
    fn submit_ack_binding_resolves_missing_private_client_ids() {
        let mut coordinator = coordinator();
        ready(&mut coordinator);
        coordinator
            .register_local_order("main", "client-1", order(), 3)
            .unwrap();
        coordinator
            .on_submit_outcome(
                "main",
                SubmitOutcome::Submitted {
                    client_order_id: "client-1".to_string(),
                    exchange_order_id: "exchange-1".to_string(),
                },
                4,
            )
            .unwrap();
        let private_order = coordinator
            .process_feed(FeedOutput::PrivateOrder {
                account_id: Some("main".to_string()),
                update: PrivateOrderUpdate {
                    ts_ms: 5,
                    exchange_order_id: "exchange-1".to_string(),
                    client_order_id: String::new(),
                    symbol: "BTC-USDT".to_string(),
                    side: Side::Buy,
                    state: PrivateOrderState::Live,
                    price: 100.0,
                    qty: 0.1,
                    cumulative_filled_qty: 0.0,
                    average_fill_price: 0.0,
                    last_fill_qty: 0.0,
                    last_fill_price: 0.0,
                    liquidity: None,
                    last_fill_fee: None,
                    fill_id: None,
                    reject_reason: String::new(),
                },
            })
            .unwrap();

        assert!(private_order.records.iter().any(|record| matches!(
            record,
            StorageRecord::Order { update, .. } if update.order_id == "client-1"
        )));
        assert!(
            !private_order
                .actions
                .iter()
                .any(|action| matches!(action, LiveAction::Reconcile(_)))
        );
        let fill = coordinator
            .process_feed(FeedOutput::PrivateFill {
                account_id: Some("main".to_string()),
                fill: RemoteFill {
                    fill_id: "fill-1".to_string(),
                    exchange_order_id: "exchange-1".to_string(),
                    client_order_id: "0".to_string(),
                    symbol: "BTC-USDT".to_string(),
                    side: Side::Buy,
                    price: 100.0,
                    qty: 0.05,
                    liquidity: FillLiquidity::Taker,
                    fee: Some(FillFee {
                        amount: -0.005,
                        currency: "USDT".to_string(),
                    }),
                    ts_ms: 6,
                },
            })
            .unwrap();

        assert!(fill.records.iter().any(|record| matches!(
            record,
            StorageRecord::Fill(fill)
                if fill.order_id == "client-1"
                    && fill.fee.as_ref().is_some_and(|fee|
                        fee.amount == -0.005 && fee.currency == "USDT")
        )));
        assert!(
            !fill
                .actions
                .iter()
                .any(|action| matches!(action, LiveAction::Reconcile(_)))
        );
        let state = coordinator.private_state("main").unwrap();
        assert_eq!(state.order_reducer().orders().count(), 1);
        assert!(state.order_reducer().get("exchange-1").is_none());
    }

    #[test]
    fn order_channel_fill_is_persisted_once_across_private_channels() {
        let mut coordinator = coordinator();
        ready(&mut coordinator);
        coordinator
            .register_local_order("main", "client-1", order(), 3)
            .unwrap();
        coordinator
            .on_submit_outcome(
                "main",
                SubmitOutcome::Submitted {
                    client_order_id: "client-1".to_string(),
                    exchange_order_id: "exchange-1".to_string(),
                },
                4,
            )
            .unwrap();

        let order_fill = coordinator
            .process_feed(FeedOutput::PrivateOrder {
                account_id: Some("main".to_string()),
                update: PrivateOrderUpdate {
                    ts_ms: 5,
                    exchange_order_id: "exchange-1".to_string(),
                    client_order_id: "client-1".to_string(),
                    symbol: "BTC-USDT".to_string(),
                    side: Side::Buy,
                    state: PrivateOrderState::PartiallyFilled,
                    price: 100.0,
                    qty: 0.1,
                    cumulative_filled_qty: 0.05,
                    average_fill_price: 100.0,
                    last_fill_qty: 0.05,
                    last_fill_price: 100.0,
                    liquidity: None,
                    last_fill_fee: Some(FillFee {
                        amount: -0.005,
                        currency: "USDT".to_string(),
                    }),
                    fill_id: Some("fill-1".to_string()),
                    reject_reason: String::new(),
                },
            })
            .unwrap();

        let persisted = order_fill
            .records
            .iter()
            .filter_map(|record| match record {
                StorageRecord::Fill(fill) => Some(fill),
                _ => None,
            })
            .collect::<Vec<_>>();
        assert_eq!(persisted.len(), 1);
        assert_eq!(persisted[0].fill_id, "fill-1");
        assert_eq!(persisted[0].order_id, "client-1");
        assert_eq!(persisted[0].liquidity, None);
        assert_eq!(
            persisted[0].fee,
            Some(FillFee {
                amount: -0.005,
                currency: "USDT".to_string(),
            })
        );

        let duplicate = coordinator
            .process_feed(FeedOutput::PrivateFill {
                account_id: Some("main".to_string()),
                fill: RemoteFill {
                    fill_id: "fill-1".to_string(),
                    exchange_order_id: "exchange-1".to_string(),
                    client_order_id: "client-1".to_string(),
                    symbol: "BTC-USDT".to_string(),
                    side: Side::Buy,
                    price: 100.0,
                    qty: 0.05,
                    liquidity: FillLiquidity::Maker,
                    fee: Some(FillFee {
                        amount: -0.005,
                        currency: "USDT".to_string(),
                    }),
                    ts_ms: 6,
                },
            })
            .unwrap();

        assert!(
            duplicate
                .records
                .iter()
                .all(|record| !matches!(record, StorageRecord::Fill(_)))
        );
        assert_eq!(
            coordinator
                .private_state("main")
                .unwrap()
                .order_reducer()
                .get("client-1")
                .unwrap()
                .filled_qty,
            0.05
        );
    }

    #[test]
    fn fee_less_fill_channel_does_not_hide_later_exact_order_fill() {
        let mut coordinator = coordinator();
        ready(&mut coordinator);
        coordinator
            .register_local_order("main", "client-1", order(), 3)
            .unwrap();
        coordinator
            .on_submit_outcome(
                "main",
                SubmitOutcome::Submitted {
                    client_order_id: "client-1".to_string(),
                    exchange_order_id: "exchange-1".to_string(),
                },
                4,
            )
            .unwrap();

        let early_fill = coordinator
            .process_feed(FeedOutput::PrivateFill {
                account_id: Some("main".to_string()),
                fill: RemoteFill {
                    fill_id: "fill-1".to_string(),
                    exchange_order_id: "exchange-1".to_string(),
                    client_order_id: "client-1".to_string(),
                    symbol: "BTC-USDT".to_string(),
                    side: Side::Buy,
                    price: 100.0,
                    qty: 0.05,
                    liquidity: FillLiquidity::Maker,
                    fee: None,
                    ts_ms: 5,
                },
            })
            .unwrap();
        assert!(
            early_fill
                .records
                .iter()
                .all(|record| !matches!(record, StorageRecord::Fill(_)))
        );

        let exact_fill = coordinator
            .process_feed(FeedOutput::PrivateOrder {
                account_id: Some("main".to_string()),
                update: PrivateOrderUpdate {
                    ts_ms: 6,
                    exchange_order_id: "exchange-1".to_string(),
                    client_order_id: "client-1".to_string(),
                    symbol: "BTC-USDT".to_string(),
                    side: Side::Buy,
                    state: PrivateOrderState::PartiallyFilled,
                    price: 100.0,
                    qty: 0.1,
                    cumulative_filled_qty: 0.05,
                    average_fill_price: 100.0,
                    last_fill_qty: 0.05,
                    last_fill_price: 100.0,
                    liquidity: Some(FillLiquidity::Maker),
                    last_fill_fee: Some(FillFee {
                        amount: -0.005,
                        currency: "USDT".to_string(),
                    }),
                    fill_id: Some("fill-1".to_string()),
                    reject_reason: String::new(),
                },
            })
            .unwrap();

        let persisted = exact_fill
            .records
            .iter()
            .filter_map(|record| match record {
                StorageRecord::Fill(fill) => Some(fill),
                _ => None,
            })
            .collect::<Vec<_>>();
        assert_eq!(persisted.len(), 1);
        assert_eq!(persisted[0].fill_id, "fill-1");
        assert_eq!(persisted[0].order_id, "client-1");
        assert_eq!(persisted[0].liquidity, Some(FillLiquidity::Maker));
        assert_eq!(
            persisted[0].fee,
            Some(FillFee {
                amount: -0.005,
                currency: "USDT".to_string(),
            })
        );
        assert_eq!(
            coordinator
                .private_state("main")
                .unwrap()
                .order_reducer()
                .get("client-1")
                .unwrap()
                .filled_qty,
            0.05
        );
    }

    #[test]
    fn wrong_account_private_order_and_fill_fail_before_state_mutation() {
        let mut coordinator = two_account_coordinator();
        ready_two_accounts(&mut coordinator);
        let order_error = coordinator
            .process_feed(FeedOutput::PrivateOrder {
                account_id: Some("hedge".to_string()),
                update: cancelled_private_order("wrong-order", "wrong-exchange", 3),
            })
            .unwrap_err();
        assert!(matches!(
            order_error,
            CoordinatorError::WrongOrderAccount {
                actual,
                expected,
                ..
            } if actual == "hedge" && expected == "main"
        ));
        assert!(
            !coordinator
                .private_state("hedge")
                .unwrap()
                .order_reducer()
                .contains_order("wrong-order")
        );

        let fill_error = coordinator
            .process_feed(FeedOutput::PrivateFill {
                account_id: Some("hedge".to_string()),
                fill: RemoteFill {
                    fill_id: "wrong-fill".to_string(),
                    exchange_order_id: "wrong-exchange".to_string(),
                    client_order_id: "wrong-order".to_string(),
                    symbol: "BTC-USDT".to_string(),
                    side: Side::Buy,
                    price: 100.0,
                    qty: 0.1,
                    liquidity: FillLiquidity::Taker,
                    fee: None,
                    ts_ms: 4,
                },
            })
            .unwrap_err();
        assert!(matches!(
            fill_error,
            CoordinatorError::WrongOrderAccount { actual, .. } if actual == "hedge"
        ));
        assert!(
            !coordinator
                .private_state("hedge")
                .unwrap()
                .has_seen_fill("BTC-USDT", "wrong-fill")
        );
    }

    #[test]
    fn unproven_private_orders_are_observed_but_never_become_cancel_authority() {
        for foreign_id in ["reap-prefix-foreign", "algo-order-7", "spread-order-9"] {
            let mut coordinator = coordinator();
            ready(&mut coordinator);

            let output = coordinator
                .process_feed(FeedOutput::PrivateOrder {
                    account_id: Some("main".to_string()),
                    update: PrivateOrderUpdate {
                        ts_ms: 4,
                        exchange_order_id: format!("exchange-{foreign_id}"),
                        client_order_id: foreign_id.to_string(),
                        symbol: "BTC-USDT".to_string(),
                        side: Side::Buy,
                        state: PrivateOrderState::Live,
                        price: 100.0,
                        qty: 0.1,
                        cumulative_filled_qty: 0.0,
                        average_fill_price: 0.0,
                        last_fill_qty: 0.0,
                        last_fill_price: 0.0,
                        liquidity: None,
                        last_fill_fee: None,
                        fill_id: None,
                        reject_reason: String::new(),
                    },
                })
                .unwrap();

            assert!(
                coordinator
                    .private_state("main")
                    .unwrap()
                    .order_reducer()
                    .contains_order(foreign_id),
                "foreign exposure must remain observable"
            );
            assert!(!coordinator.readiness().is_ready());
            assert!(output.actions.iter().all(|action| {
                !matches!(action, LiveAction::Cancel(cancel) if cancel.client_order_id == foreign_id)
            }));
            assert!(output.actions.iter().any(|action| {
                matches!(action, LiveAction::Reconcile(reconcile) if reconcile.account_id == "main")
            }));
            assert!(coordinator.readiness().faults.keys().any(|fault| {
                fault == &format!("runtime:foreign_regular_order:main:{foreign_id}")
            }));

            let stale_terminal = coordinator
                .process_feed(FeedOutput::PrivateOrder {
                    account_id: Some("main".to_string()),
                    update: cancelled_private_order(
                        foreign_id,
                        &format!("exchange-{foreign_id}"),
                        3,
                    ),
                })
                .unwrap();
            assert!(stale_terminal.actions.is_empty());
            assert_eq!(
                coordinator
                    .private_state("main")
                    .unwrap()
                    .order_reducer()
                    .get(foreign_id)
                    .unwrap()
                    .status,
                OrderStatus::Live
            );
            coordinator
                .on_reconciliation(ReconciliationResult {
                    account_id: "main".to_string(),
                    ts_ms: 5,
                    clean: true,
                    local_live_orders: 1,
                    remote_live_orders: 1,
                    remote_recent_fills: 0,
                    reason: "foreign order still requires operator handling".to_string(),
                })
                .unwrap();
            assert!(!coordinator.readiness().is_ready());
            assert!(coordinator.readiness().faults.keys().any(|fault| {
                fault == &format!("runtime:foreign_regular_order:main:{foreign_id}")
            }));

            let safety = coordinator.process_event(NormalizedEvent::System(SystemEvent {
                ts_ms: 6,
                kind: SystemEventKind::KillSwitchActivated,
                venue: None,
                account_id: None,
                symbol: None,
                reason: "test fail-closed cancellation".to_string(),
            }));
            assert!(safety.actions.iter().all(|action| {
                !matches!(action, LiveAction::Cancel(cancel) if cancel.client_order_id == foreign_id)
            }));
            assert!(safety.records.iter().any(|record| matches!(
                record,
                StorageRecord::IntentRejected { reason, .. }
                    if reason.contains("not a proven owned regular order")
            )));
        }
    }

    #[test]
    fn known_order_identity_mismatch_fails_before_mapping_or_fill_mutation() {
        let mut coordinator = coordinator();
        coordinator
            .register_local_order("main", "client-1", order(), 3)
            .unwrap();
        let order_error = coordinator
            .process_feed(FeedOutput::PrivateOrder {
                account_id: Some("main".to_string()),
                update: PrivateOrderUpdate {
                    ts_ms: 4,
                    exchange_order_id: "exchange-1".to_string(),
                    client_order_id: "client-1".to_string(),
                    symbol: "BTC-PERP".to_string(),
                    side: Side::Buy,
                    state: PrivateOrderState::Live,
                    price: 100.0,
                    qty: 0.1,
                    cumulative_filled_qty: 0.0,
                    average_fill_price: 0.0,
                    last_fill_qty: 0.0,
                    last_fill_price: 0.0,
                    liquidity: None,
                    last_fill_fee: None,
                    fill_id: None,
                    reject_reason: String::new(),
                },
            })
            .unwrap_err();
        assert!(matches!(
            order_error,
            CoordinatorError::PrivateOrderIdentity {
                source: PrivateOrderIdentityError::SymbolMismatch { .. },
                ..
            }
        ));
        assert!(
            coordinator
                .private_state("main")
                .unwrap()
                .canonical_order_id("exchange-1")
                .is_none()
        );

        let fill_error = coordinator
            .process_feed(FeedOutput::PrivateFill {
                account_id: Some("main".to_string()),
                fill: RemoteFill {
                    fill_id: "fill-wrong-side".to_string(),
                    exchange_order_id: String::new(),
                    client_order_id: "client-1".to_string(),
                    symbol: "BTC-USDT".to_string(),
                    side: Side::Sell,
                    price: 100.0,
                    qty: 0.05,
                    liquidity: FillLiquidity::Taker,
                    fee: None,
                    ts_ms: 5,
                },
            })
            .unwrap_err();
        assert!(matches!(
            fill_error,
            CoordinatorError::PrivateOrderIdentity {
                source: PrivateOrderIdentityError::SideMismatch { .. },
                ..
            }
        ));
        assert!(
            !coordinator
                .private_state("main")
                .unwrap()
                .has_seen_fill("BTC-USDT", "fill-wrong-side")
        );
    }

    #[test]
    fn repeated_submit_rejections_persist_risk_latch_and_cancel_live_orders() {
        let mut coordinator = coordinator_with_risk(
            true,
            RiskLimits {
                require_feed_health: false,
                require_private_health: false,
                order_reject_count_limit: 2,
                order_reject_count_per_symbol_limit: 2,
                order_reject_window_ms: 60_000,
                ..RiskLimits::default()
            },
        );
        ready(&mut coordinator);
        coordinator
            .restore_owned_order(
                "main",
                OrderUpdate {
                    ts_ms: 2,
                    order_id: "live-order".to_string(),
                    symbol: "BTC-USDT".to_string(),
                    side: Side::Buy,
                    event: OrderEvent::New,
                    status: OrderStatus::Live,
                    price: 100.0,
                    time_in_force: Some(TimeInForce::PostOnly),
                    qty: 1.0,
                    open_qty: 1.0,
                    filled_qty: 0.0,
                    avg_fill_price: 0.0,
                    last_fill_qty: 0.0,
                    last_fill_price: 0.0,
                    last_fill_liquidity: None,
                    last_fill_fee: None,
                    reason: "quote".to_string(),
                },
            )
            .unwrap();
        coordinator
            .register_local_order("main", "reject-1", order(), 3)
            .unwrap();
        let first = coordinator
            .on_submit_error("main", "reject-1", 4, false, "exchange rejected")
            .unwrap();
        assert!(!coordinator.kill_switch_active());
        assert!(
            !first
                .records
                .iter()
                .any(|record| matches!(record, StorageRecord::SafetyLatch(_)))
        );

        coordinator
            .register_local_order("main", "reject-2", order(), 5)
            .unwrap();
        let second = coordinator
            .on_submit_error("main", "reject-2", 6, false, "exchange rejected")
            .unwrap();

        assert!(coordinator.kill_switch_active());
        assert!(second.records.iter().any(|record| matches!(
            record,
            StorageRecord::SafetyLatch(latch)
                if latch.active
                    && latch.scope == SafetyLatchScope::Global
                    && latch.source == SafetyLatchSource::Risk
                    && latch.reason.contains("order rejection count 2 reached limit 2")
        )));
        assert!(second.actions.iter().any(|action| matches!(
            action,
            LiveAction::Cancel(cancel) if cancel.client_order_id == "live-order"
        )));
    }

    #[test]
    fn repeated_unfilled_ioc_cancels_persist_risk_latch_and_cancel_live_orders() {
        let mut coordinator = coordinator_with_risk(
            true,
            RiskLimits {
                require_feed_health: false,
                require_private_health: false,
                unfilled_ioc_cancel_count_per_symbol_limit: 2,
                unfilled_ioc_cancel_window_ms: 60_000,
                ..RiskLimits::default()
            },
        );
        ready(&mut coordinator);
        coordinator
            .restore_owned_order(
                "main",
                OrderUpdate {
                    ts_ms: 2,
                    order_id: "live-order".to_string(),
                    symbol: "BTC-USDT".to_string(),
                    side: Side::Sell,
                    event: OrderEvent::New,
                    status: OrderStatus::Live,
                    price: 101.0,
                    time_in_force: Some(TimeInForce::PostOnly),
                    qty: 0.1,
                    open_qty: 0.1,
                    filled_qty: 0.0,
                    avg_fill_price: 0.0,
                    last_fill_qty: 0.0,
                    last_fill_price: 0.0,
                    last_fill_liquidity: None,
                    last_fill_fee: None,
                    reason: "quote".to_string(),
                },
            )
            .unwrap();

        let mut ioc = order();
        ioc.time_in_force = TimeInForce::Ioc;
        ioc.reason = "hedge:BTC-USDT:100".to_string();
        coordinator
            .register_local_order("main", "ioc-1", ioc.clone(), 3)
            .unwrap();
        let first_cancel = cancelled_private_order("ioc-1", "exchange-ioc-1", 4);
        let first = coordinator
            .process_feed(FeedOutput::PrivateOrder {
                account_id: Some("main".to_string()),
                update: first_cancel.clone(),
            })
            .unwrap();
        assert!(!coordinator.kill_switch_active());
        assert!(first.records.iter().any(|record| matches!(
            record,
            StorageRecord::Order { update, .. }
                if update.order_id == "ioc-1"
                    && update.status == OrderStatus::Cancelled
                    && update.time_in_force == Some(TimeInForce::Ioc)
        )));
        let duplicate = coordinator
            .process_feed(FeedOutput::PrivateOrder {
                account_id: Some("main".to_string()),
                update: first_cancel,
            })
            .unwrap();
        assert!(!coordinator.kill_switch_active());
        assert!(
            !duplicate
                .records
                .iter()
                .any(|record| matches!(record, StorageRecord::SafetyLatch(_)))
        );

        coordinator
            .register_local_order("main", "ioc-2", ioc, 5)
            .unwrap();
        let second = coordinator
            .process_feed(FeedOutput::PrivateOrder {
                account_id: Some("main".to_string()),
                update: cancelled_private_order("ioc-2", "exchange-ioc-2", 6),
            })
            .unwrap();

        assert!(coordinator.kill_switch_active());
        assert!(second.records.iter().any(|record| matches!(
            record,
            StorageRecord::SafetyLatch(latch)
                if latch.active
                    && latch.scope == SafetyLatchScope::Global
                    && latch.source == SafetyLatchSource::Risk
                    && latch.reason.contains(
                        "symbol BTC-USDT unfilled IOC cancellation count 2 reached limit 2"
                    )
        )));
        assert!(second.actions.iter().any(|action| matches!(
            action,
            LiveAction::Cancel(cancel) if cancel.client_order_id == "live-order"
        )));
    }

    #[test]
    fn chaos_internal_halt_persists_risk_latch_and_cancels_live_orders() {
        let mut coordinator = coordinator_with_risk(
            true,
            RiskLimits {
                max_abs_position_notional_usd: 1_000_000_000.0,
                require_feed_health: false,
                require_private_health: false,
                ..RiskLimits::default()
            },
        );
        ready(&mut coordinator);
        coordinator.set_order_entry_enabled(false);
        coordinator
            .restore_owned_order(
                "main",
                OrderUpdate {
                    ts_ms: 2,
                    order_id: "live-order".to_string(),
                    symbol: "BTC-USDT".to_string(),
                    side: Side::Sell,
                    event: OrderEvent::New,
                    status: OrderStatus::Live,
                    price: 101.0,
                    time_in_force: Some(TimeInForce::PostOnly),
                    qty: 0.1,
                    open_qty: 0.1,
                    filled_qty: 0.0,
                    avg_fill_price: 0.0,
                    last_fill_qty: 0.0,
                    last_fill_price: 0.0,
                    last_fill_liquidity: None,
                    last_fill_fee: None,
                    reason: "quote".to_string(),
                },
            )
            .unwrap();
        for symbol in ["BTC-USDT", "BTC-PERP"] {
            coordinator.process_event(NormalizedEvent::Market(MarketEvent::Depth(
                OrderBook::one_level(
                    symbol,
                    3,
                    Level::new(99.0, 10_000.0),
                    Level::new(101.0, 10_000.0),
                ),
            )));
        }
        assert!(!coordinator.kill_switch_active());

        let output = coordinator.process_event(NormalizedEvent::Account(AccountUpdate {
            ts_ms: 4,
            balances: Vec::new(),
            positions: vec![Position {
                symbol: "BTC-USDT".to_string(),
                qty: 1_000.0,
                avg_price: 100.0,
                margin_mode: None,
            }],
            margins: Vec::new(),
        }));

        assert!(coordinator.kill_switch_active());
        assert!(output.records.iter().any(|record| matches!(
            record,
            StorageRecord::SafetyLatch(latch)
                if latch.active
                    && latch.scope == SafetyLatchScope::Global
                    && latch.source == SafetyLatchSource::Risk
                    && latch.reason.contains("strategy halted: strategy delta")
        )));
        assert!(output.actions.iter().any(|action| matches!(
            action,
            LiveAction::Cancel(cancel) if cancel.client_order_id == "live-order"
        )));
    }

    #[test]
    fn fill_convergence_fault_cancels_live_orders_and_reconciles_account() {
        let mut coordinator = coordinator();
        ready(&mut coordinator);
        coordinator
            .register_local_order("main", "client-1", order(), 3)
            .unwrap();
        coordinator
            .process_feed(FeedOutput::PrivateOrder {
                account_id: Some("main".to_string()),
                update: PrivateOrderUpdate {
                    ts_ms: 4,
                    exchange_order_id: "exchange-1".to_string(),
                    client_order_id: "client-1".to_string(),
                    symbol: "BTC-USDT".to_string(),
                    side: Side::Buy,
                    state: PrivateOrderState::Live,
                    price: 100.0,
                    qty: 0.1,
                    cumulative_filled_qty: 0.0,
                    average_fill_price: 0.0,
                    last_fill_qty: 0.0,
                    last_fill_price: 0.0,
                    liquidity: None,
                    last_fill_fee: None,
                    fill_id: None,
                    reject_reason: String::new(),
                },
            })
            .unwrap();

        let output = coordinator
            .reconciliation_fault(
                "main",
                5,
                Some("BTC-USDT".to_string()),
                "fill-to-account-state convergence exceeded 2000ms".to_string(),
            )
            .unwrap();

        assert!(!coordinator.readiness().is_ready());
        assert!(output.actions.iter().any(|action| matches!(
            action,
            LiveAction::Cancel(cancel) if cancel.client_order_id == "client-1"
        )));
        assert!(output.actions.iter().any(|action| matches!(
            action,
            LiveAction::Reconcile(reconcile) if reconcile.account_id == "main"
        )));
        assert!(output.records.iter().any(|record| matches!(
            record,
            StorageRecord::System(event)
                if event.kind == SystemEventKind::ReconcileDrift
                    && event.account_id.as_deref() == Some("main")
        )));
    }

    #[test]
    fn forbidden_state_blocks_placement_cancels_owned_regular_and_requires_clean_reconcile() {
        let mut coordinator = coordinator();
        ready(&mut coordinator);
        coordinator
            .restore_owned_order(
                "main",
                OrderUpdate {
                    ts_ms: 2,
                    order_id: "live-order".to_string(),
                    symbol: "BTC-USDT".to_string(),
                    side: Side::Sell,
                    event: OrderEvent::New,
                    status: OrderStatus::Live,
                    price: 101.0,
                    time_in_force: Some(TimeInForce::PostOnly),
                    qty: 0.1,
                    open_qty: 0.1,
                    filled_qty: 0.0,
                    avg_fill_price: 0.0,
                    last_fill_qty: 0.0,
                    last_fill_price: 0.0,
                    last_fill_liquidity: None,
                    last_fill_fee: None,
                    reason: "quote".to_string(),
                },
            )
            .unwrap();

        let output = coordinator
            .on_forbidden_order_event(ForbiddenOrderEvent {
                account_id: "main".to_string(),
                observed_at_ms: 3,
                state: ForbiddenOrderState::NonZero {
                    algo_orders_observed: Some(1),
                    spread_orders_observed: Some(0),
                },
            })
            .unwrap();
        assert!(!coordinator.readiness().is_ready());
        assert!(output.actions.iter().any(|action| matches!(
            action,
            LiveAction::Cancel(cancel) if cancel.client_order_id == "live-order"
        )));
        assert!(output.actions.iter().any(|action| matches!(
            action,
            LiveAction::Reconcile(reconcile) if reconcile.account_id == "main"
        )));

        coordinator
            .on_forbidden_order_event(ForbiddenOrderEvent {
                account_id: "main".to_string(),
                observed_at_ms: 4,
                state: ForbiddenOrderState::VerifiedZero {
                    expires_at_ms: 30_004,
                },
            })
            .unwrap();
        assert!(
            !coordinator.readiness().is_ready(),
            "a fresh zero proof must not bypass the clean-reconciliation requirement"
        );
        coordinator
            .on_reconciliation(ReconciliationResult {
                account_id: "main".to_string(),
                ts_ms: 5,
                clean: true,
                local_live_orders: 1,
                remote_live_orders: 1,
                remote_recent_fills: 0,
                reason: "regular state is clean".to_string(),
            })
            .unwrap();
        assert!(coordinator.readiness().is_ready());
    }

    #[test]
    fn strategy_submit_is_registered_in_same_event_loop_turn() {
        let mut coordinator = coordinator();
        ready(&mut coordinator);
        let mut actions = Vec::new();
        for ts_ms in [1_000, 2_000] {
            for (symbol, bid, ask) in [("BTC-USDT", 99.9, 100.1), ("BTC-PERP", 99.8, 100.2)] {
                let output = coordinator.process_event(NormalizedEvent::Market(
                    MarketEvent::Depth(OrderBook::one_level(
                        symbol,
                        ts_ms,
                        Level::new(bid, 100.0),
                        Level::new(ask, 100.0),
                    )),
                ));
                actions.extend(output.actions);
            }
        }
        let submits = actions
            .iter()
            .filter_map(|action| match action {
                LiveAction::Submit(action) => Some(action),
                _ => None,
            })
            .collect::<Vec<_>>();

        assert!(!submits.is_empty());
        for submit in submits {
            assert_eq!(
                coordinator
                    .private_state(&submit.account_id)
                    .unwrap()
                    .order_reducer()
                    .get(&submit.client_order_id)
                    .unwrap()
                    .status,
                OrderStatus::PendingNew
            );
        }
    }

    #[test]
    fn restart_replays_missed_fill_and_terminal_order_before_reconciliation() {
        let mut coordinator = coordinator();
        let restored = OrderUpdate {
            ts_ms: 10,
            order_id: "client-1".to_string(),
            symbol: "BTC-USDT".to_string(),
            side: Side::Buy,
            event: OrderEvent::New,
            status: OrderStatus::Live,
            price: 100.0,
            time_in_force: Some(TimeInForce::PostOnly),
            qty: 1.0,
            open_qty: 1.0,
            filled_qty: 0.0,
            avg_fill_price: 0.0,
            last_fill_qty: 0.0,
            last_fill_price: 0.0,
            last_fill_liquidity: None,
            last_fill_fee: None,
            reason: "quote".to_string(),
        };
        coordinator
            .restore_owned_order("main", restored)
            .expect("checkpoint order should restore");
        let fill = RemoteFill {
            fill_id: "fill-1".to_string(),
            exchange_order_id: "exchange-1".to_string(),
            client_order_id: "client-1".to_string(),
            symbol: "BTC-USDT".to_string(),
            side: Side::Buy,
            price: 99.9,
            qty: 1.0,
            liquidity: FillLiquidity::Taker,
            fee: None,
            ts_ms: 11,
        };
        coordinator
            .process_feed(FeedOutput::PrivateFill {
                account_id: Some("main".to_string()),
                fill: fill.clone(),
            })
            .unwrap();
        coordinator
            .process_feed(FeedOutput::PrivateOrder {
                account_id: Some("main".to_string()),
                update: PrivateOrderUpdate {
                    ts_ms: 12,
                    exchange_order_id: "exchange-1".to_string(),
                    client_order_id: "client-1".to_string(),
                    symbol: "BTC-USDT".to_string(),
                    side: Side::Buy,
                    state: PrivateOrderState::Filled,
                    price: 100.0,
                    qty: 1.0,
                    cumulative_filled_qty: 1.0,
                    average_fill_price: 99.9,
                    last_fill_qty: 0.0,
                    last_fill_price: 0.0,
                    liquidity: None,
                    last_fill_fee: None,
                    fill_id: None,
                    reject_reason: String::new(),
                },
            })
            .unwrap();
        let remote = RemoteOrder {
            exchange_order_id: "exchange-1".to_string(),
            client_order_id: "client-1".to_string(),
            symbol: "BTC-USDT".to_string(),
            side: Side::Buy,
            state: PrivateOrderState::Filled,
            price: 100.0,
            qty: 1.0,
            cumulative_filled_qty: 1.0,
            average_fill_price: 99.9,
            update_time_ms: 12,
        };
        let state = coordinator.private_state("main").unwrap();
        let report = reconcile(
            state.order_reducer(),
            &HashSet::from([FillKey::new("BTC-USDT", "fill-1")]),
            &[remote],
            &[fill],
        );

        assert!(report.is_clean(), "{:?}", report.issues);
        assert_eq!(
            state.order_reducer().get("client-1").unwrap().status,
            OrderStatus::Filled
        );
    }

    #[test]
    fn account_halt_cancels_and_blocks_only_the_target_account() {
        let mut coordinator = two_account_coordinator();
        ready_two_accounts(&mut coordinator);
        let mut main_order = order();
        main_order.reason = "external".to_string();
        let mut hedge_order = order();
        hedge_order.symbol = "BTC-PERP".to_string();
        hedge_order.qty = 1.0;
        hedge_order.reason = "external".to_string();
        coordinator
            .register_local_order("main", "client-main", main_order.clone(), 3)
            .unwrap();
        coordinator
            .register_local_order("hedge", "client-hedge", hedge_order.clone(), 3)
            .unwrap();

        let output = coordinator
            .halt_account(4, "main", "unexpected exposure")
            .unwrap();

        let cancels = output
            .actions
            .iter()
            .filter_map(|action| match action {
                LiveAction::Cancel(cancel) => {
                    Some((cancel.account_id.as_str(), cancel.client_order_id.as_str()))
                }
                _ => None,
            })
            .collect::<Vec<_>>();
        assert_eq!(cancels, vec![("main", "client-main")]);
        assert_eq!(
            coordinator
                .halted_accounts()
                .get("main")
                .map(String::as_str),
            Some("unexpected exposure")
        );
        assert!(output.records.iter().any(|record| matches!(
            record,
            StorageRecord::System(SystemEvent {
                kind: SystemEventKind::AccountHalted,
                account_id: Some(account_id),
                ..
            }) if account_id == "main"
        )));

        let mut blocked = CoordinatorOutput::default();
        coordinator.route_intent(5, OrderIntent::NewOrder(main_order), &mut blocked);
        assert!(blocked.actions.is_empty());
        assert!(blocked.records.iter().any(|record| matches!(
            record,
            StorageRecord::IntentRejected { reason, .. }
                if reason.contains("legacy serialized OrderIntent has no live execution authority")
        )));

        let mut healthy = CoordinatorOutput::default();
        coordinator.route_intent(5, OrderIntent::NewOrder(hedge_order), &mut healthy);
        assert!(healthy.actions.is_empty());
        assert!(healthy.records.iter().any(|record| matches!(
            record,
            StorageRecord::IntentRejected { reason, .. }
                if reason.contains("legacy serialized OrderIntent has no live execution authority")
        )));
    }

    #[test]
    fn observe_mode_never_dispatches_gateway_cancels() {
        let mut coordinator = coordinator_with_gateway_actions(false);
        ready(&mut coordinator);
        coordinator
            .register_local_order("main", "client-1", order(), 3)
            .unwrap();
        let output = coordinator.process_event(NormalizedEvent::System(SystemEvent {
            ts_ms: 4,
            kind: SystemEventKind::KillSwitchActivated,
            venue: None,
            account_id: None,
            symbol: None,
            reason: "observe".to_string(),
        }));

        assert!(
            !output
                .actions
                .iter()
                .any(|action| matches!(action, LiveAction::Cancel(_)))
        );
        assert!(output.records.iter().any(|record| matches!(
            record,
            StorageRecord::IntentRejected { reason, .. }
                if reason.contains("observe mode")
        )));
    }

    #[test]
    fn disabling_new_order_entry_preserves_kill_switch_cancels() {
        let mut coordinator = coordinator();
        ready(&mut coordinator);
        coordinator
            .register_local_order("main", "client-1", order(), 3)
            .unwrap();
        coordinator.set_order_entry_enabled(false);

        let output = coordinator.process_event(NormalizedEvent::System(SystemEvent {
            ts_ms: 4,
            kind: SystemEventKind::KillSwitchActivated,
            venue: None,
            account_id: None,
            symbol: None,
            reason: "shutdown".to_string(),
        }));

        assert!(output.actions.iter().any(|action| matches!(
            action,
            LiveAction::Cancel(CancelAction {
                client_order_id,
                ..
            }) if client_order_id == "client-1"
        )));
        assert!(
            !output
                .actions
                .iter()
                .any(|action| matches!(action, LiveAction::Submit(_)))
        );
    }
}
