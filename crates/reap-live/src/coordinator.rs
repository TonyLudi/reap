use std::collections::{BTreeMap, HashMap, HashSet};

use reap_core::{NormalizedEvent, OrderIntent, SystemEvent, SystemEventKind, TimeMs, Venue};
use reap_engine::{EngineOutput, TradingEngine};
use reap_feed::{FeedOutput, RecoveryRequest};
use reap_order::{CancelOutcome, ClientOrderIdGenerator, PrivateStateReducer, SubmitOutcome};
use reap_risk::{RiskDecision, RiskGate};
use reap_storage::{
    FillRecord, OrderAckRecord, OrderAckStatus, OrderOperation, ReconciliationRecord,
    SafetyLatchRecord, SafetyLatchScope, SafetyLatchSource, StorageRecord,
};
use reap_strategy::ChaosStrategy;
use thiserror::Error;

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
    #[error(transparent)]
    Startup(#[from] StartupError),
}

pub struct LiveCoordinator {
    config: LiveConfig,
    engine: TradingEngine<ChaosStrategy>,
    startup: StartupGate,
    private_states: HashMap<String, PrivateStateReducer>,
    client_ids: HashMap<String, ClientOrderIdGenerator>,
    gateway_actions_enabled: bool,
    order_entry_enabled: bool,
    halted_accounts: BTreeMap<String, String>,
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
        let session_id = session_id.into();
        if session_id.trim().is_empty() {
            return Err(CoordinatorError::EmptySessionId);
        }
        let strategy = ChaosStrategy::new(config.strategy.clone())
            .map_err(|error| CoordinatorError::Strategy(error.to_string()))?;
        let mut risk = RiskGate::new(config.risk.clone());
        for instrument in verified.instruments.values() {
            risk.set_instrument_model(instrument.symbol.clone(), instrument.risk_model);
        }
        let mut startup = StartupGate::new(&config);
        startup.mark_metadata_verified();
        let mut private_states = HashMap::new();
        let mut client_ids = HashMap::new();
        for account in &config.accounts {
            let mut state = PrivateStateReducer::new();
            if let Some(fill_ids) = verified.baseline_fill_ids.get(&account.id) {
                state.seed_fill_ids(fill_ids.iter().cloned());
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
            startup.mark_account_snapshot(&account.id, true, "initial account snapshot loaded")?;
        }
        Ok(Self {
            config,
            engine: TradingEngine::new(strategy, risk),
            startup,
            private_states,
            client_ids,
            gateway_actions_enabled,
            order_entry_enabled: gateway_actions_enabled,
            halted_accounts: BTreeMap::new(),
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

    pub fn private_state(&self, account_id: &str) -> Option<&PrivateStateReducer> {
        self.private_states.get(account_id)
    }

    pub fn restore_order(
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
        self.private_state_mut(account_id)?
            .restore_order_update(update.clone());
        Ok(self.process_normalized(NormalizedEvent::Order(update)))
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
                let mut update = update;
                scope_account_update(&account_id, &mut update);
                self.private_state_mut(&account_id)?
                    .apply_account(update.clone());
                Ok(self.process_normalized(NormalizedEvent::Account(update)))
            }
            FeedOutput::PrivateOrder { account_id, update } => {
                let account_id = self.require_account_id(account_id)?;
                let canonical_id = if update.client_order_id.is_empty() {
                    update.exchange_order_id.clone()
                } else {
                    update.client_order_id.clone()
                };
                let known = {
                    let state = self.private_state_mut(&account_id)?;
                    state.order_reducer().contains_order(&canonical_id)
                        || state
                            .canonical_order_id(&update.exchange_order_id)
                            .is_some()
                };
                let canonical = self.private_state_mut(&account_id)?.apply_order(update);
                let mut output = CoordinatorOutput::default();
                if !known {
                    output.extend(self.reconciliation_fault(
                        &account_id,
                        canonical.as_ref().map(|order| order.ts_ms).unwrap_or(0),
                        canonical.as_ref().map(|order| order.symbol.clone()),
                        format!("private update for unknown order {canonical_id}"),
                    )?);
                }
                if let Some(update) = canonical {
                    output.extend(self.process_normalized(NormalizedEvent::Order(update)));
                }
                Ok(output)
            }
            FeedOutput::PrivateFill { account_id, fill } => {
                let account_id = self.require_account_id(account_id)?;
                let canonical_id = if fill.client_order_id.is_empty() {
                    fill.exchange_order_id.clone()
                } else {
                    fill.client_order_id.clone()
                };
                let known = self
                    .private_state_mut(&account_id)?
                    .order_reducer()
                    .contains_order(&canonical_id);
                let fill_record = FillRecord {
                    ts_ms: fill.ts_ms,
                    account_id: Some(account_id.clone()),
                    fill_id: fill.fill_id.clone(),
                    order_id: canonical_id.clone(),
                    symbol: fill.symbol.clone(),
                    side: fill.side,
                    price: fill.price,
                    qty: fill.qty,
                    liquidity: fill.liquidity,
                };
                let ts_ms = fill.ts_ms;
                let symbol = fill.symbol.clone();
                let canonical = self.private_state_mut(&account_id)?.apply_fill(fill);
                let mut output = CoordinatorOutput {
                    actions: Vec::new(),
                    records: vec![StorageRecord::Fill(fill_record)],
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

    pub fn process_event(&mut self, event: NormalizedEvent) -> CoordinatorOutput {
        self.process_normalized(event)
    }

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
        Ok(CoordinatorOutput {
            actions: Vec::new(),
            records: vec![StorageRecord::OrderAck(OrderAckRecord {
                ts_ms,
                account_id: account_id.to_string(),
                operation: OrderOperation::Submit,
                client_order_id,
                exchange_order_id,
                status,
                message: "REST acknowledgement received; awaiting private stream".to_string(),
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
        self.private_state_mut(account_id)?;
        Ok(CoordinatorOutput {
            actions: Vec::new(),
            records: vec![StorageRecord::OrderAck(OrderAckRecord {
                ts_ms,
                account_id: account_id.to_string(),
                operation: OrderOperation::Cancel,
                client_order_id: outcome.client_order_id,
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
        if let NormalizedEvent::System(system) = &event {
            self.apply_system_to_startup(system);
        }
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
        let engine_output = self.engine.on_event(event);
        let mut output = CoordinatorOutput {
            actions: Vec::new(),
            records,
        };
        self.handle_engine_output(now_ms, engine_output, &mut output);
        if let Some((account_id, reason)) = account_halt {
            self.ensure_account_cancels(now_ms, &account_id, &reason, &mut output);
        }
        output
    }

    fn handle_engine_output(
        &mut self,
        now_ms: TimeMs,
        engine_output: EngineOutput,
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
            output.records.push(StorageRecord::Intent {
                ts_ms: now_ms,
                intent: intent.clone(),
            });
            self.route_intent(now_ms, intent, output);
        }
    }

    fn route_intent(
        &mut self,
        now_ms: TimeMs,
        intent: OrderIntent,
        output: &mut CoordinatorOutput,
    ) {
        match intent {
            OrderIntent::NewOrder(order) => {
                let submit_enabled = self.gateway_actions_enabled && self.order_entry_enabled;
                let Some(account_id) = self
                    .config
                    .account_for_symbol(&order.symbol)
                    .map(|account| account.id.clone())
                else {
                    output.records.push(StorageRecord::IntentRejected {
                        ts_ms: now_ms,
                        intent: OrderIntent::NewOrder(order),
                        reason: "symbol has no account route".to_string(),
                    });
                    self.startup.mark_runtime_health(
                        "routing",
                        false,
                        "strategy emitted an order for an unmapped symbol",
                    );
                    return;
                };
                if let Some(reason) = self.halted_accounts.get(&account_id) {
                    output.records.push(StorageRecord::IntentRejected {
                        ts_ms: now_ms,
                        intent: OrderIntent::NewOrder(order),
                        reason: format!("account {account_id} is halted: {reason}"),
                    });
                    return;
                }
                if !self.startup.can_submit_new(submit_enabled) {
                    output.records.push(StorageRecord::IntentRejected {
                        ts_ms: now_ms,
                        intent: OrderIntent::NewOrder(order),
                        reason: format!(
                            "live gate is {:?}; gateway actions enabled={}; new order entry enabled={}",
                            self.startup.phase(),
                            self.gateway_actions_enabled,
                            self.order_entry_enabled
                        ),
                    });
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
                output.actions.push(LiveAction::Submit(SubmitAction {
                    ts_ms: now_ms,
                    account_id: account_id.clone(),
                    idempotency_key,
                    client_order_id: client_order_id.clone(),
                    order: order.clone(),
                }));
                let pending = self
                    .private_states
                    .get_mut(&account_id)
                    .expect("validated account must have private state")
                    .register_local_order_at(&client_order_id, order, now_ms);
                if let Some(update) = pending {
                    output.extend(self.process_normalized(NormalizedEvent::Order(update)));
                }
            }
            OrderIntent::CancelOrder { order_id, reason } => {
                if !self.gateway_actions_enabled || !self.startup.can_cancel() {
                    let rejection_reason = if self.gateway_actions_enabled {
                        format!(
                            "live gate is {:?}; cancellation is unavailable",
                            self.startup.phase()
                        )
                    } else {
                        "live gateway actions are disabled in observe mode".to_string()
                    };
                    output.records.push(StorageRecord::IntentRejected {
                        ts_ms: now_ms,
                        intent: OrderIntent::CancelOrder { order_id, reason },
                        reason: rejection_reason,
                    });
                    return;
                }
                let route = self.private_states.iter().find_map(|(account_id, state)| {
                    state
                        .order_reducer()
                        .get(&order_id)
                        .map(|order| (account_id.clone(), order.symbol.clone()))
                });
                if let Some((account_id, symbol)) = route {
                    output.actions.push(LiveAction::Cancel(CancelAction {
                        ts_ms: now_ms,
                        account_id,
                        symbol,
                        client_order_id: order_id,
                        reason,
                    }));
                } else {
                    output.records.push(StorageRecord::IntentRejected {
                        ts_ms: now_ms,
                        intent: OrderIntent::CancelOrder { order_id, reason },
                        reason: "cancel target is not present in canonical private state"
                            .to_string(),
                    });
                }
            }
        }
    }

    fn ensure_account_cancels(
        &mut self,
        now_ms: TimeMs,
        account_id: &str,
        halt_reason: &str,
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
            .map(|(order_id, _)| order_id.to_string())
            .filter(|order_id| !existing.contains(order_id))
            .collect::<Vec<_>>();
        for order_id in active_orders {
            let intent = OrderIntent::CancelOrder {
                order_id,
                reason: format!("account {account_id} halted: {halt_reason}"),
            };
            output.records.push(StorageRecord::Intent {
                ts_ms: now_ms,
                intent: intent.clone(),
            });
            self.route_intent(now_ms, intent, output);
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

    fn reconciliation_fault(
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
        AccountUpdate, Balance, FillLiquidity, Level, MarketEvent, NewOrder, OrderBook, OrderEvent,
        OrderStatus, OrderUpdate, Side, SystemEvent, SystemEventKind, TimeInForce, Venue,
    };
    use reap_feed::FeedOutput;
    use reap_order::reconcile;
    use reap_risk::{InstrumentRiskModel, RiskLimits};
    use reap_strategy::ChaosConfig;
    use reap_venue::okx::{OkxAccountLevel, OkxInstrumentType, OkxPositionMode};
    use reap_venue::{PrivateOrderState, PrivateOrderUpdate, RemoteFill, RemoteOrder};

    use crate::{
        LiveAccountConfig, LiveStorageConfig, OkxTradeModeConfig, OkxVenueConfig, RuntimeConfig,
        VerifiedInstrument,
    };

    use super::*;

    fn coordinator_with_gateway_actions(gateway_actions_enabled: bool) -> LiveCoordinator {
        let mut strategy: ChaosConfig =
            toml::from_str(include_str!("../../../examples/iarb2-basic.toml")).unwrap();
        strategy.risk_groups[0].account_id = Some("main".to_string());
        let config = LiveConfig {
            strategy,
            risk: RiskLimits {
                require_feed_health: false,
                require_private_health: false,
                ..RiskLimits::default()
            },
            venue: OkxVenueConfig::default(),
            runtime: RuntimeConfig::default(),
            storage: LiveStorageConfig::default(),
            operator: crate::OperatorConfig::default(),
            accounts: vec![LiveAccountConfig {
                id: "main".to_string(),
                api_key_env: "KEY".to_string(),
                secret_key_env: "SECRET".to_string(),
                passphrase_env: "PASS".to_string(),
                expected_account_level: OkxAccountLevel::SingleCurrencyMargin,
                expected_position_mode: OkxPositionMode::NetMode,
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
                    }],
                    positions: Vec::new(),
                    margins: Vec::new(),
                },
            )]),
            baseline_fill_ids: HashMap::from([("main".to_string(), HashSet::new())]),
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
            }],
            positions: Vec::new(),
            margins: Vec::new(),
        }
    }

    fn ready(coordinator: &mut LiveCoordinator) {
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
        }
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
            qty: 1.0,
            open_qty: 1.0,
            filled_qty: 0.0,
            avg_fill_price: 0.0,
            last_fill_qty: 0.0,
            last_fill_price: 0.0,
            last_fill_liquidity: None,
            reason: "quote".to_string(),
        };
        coordinator
            .restore_order("main", restored)
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
            state.seen_fill_ids(),
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
                if reason.contains("account main is halted")
        )));

        let mut healthy = CoordinatorOutput::default();
        coordinator.route_intent(5, OrderIntent::NewOrder(hedge_order), &mut healthy);
        assert!(healthy.actions.iter().any(|action| matches!(
            action,
            LiveAction::Submit(submit) if submit.account_id == "hedge"
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
