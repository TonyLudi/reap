use std::collections::{BTreeMap, HashMap, HashSet};

use reap_core::{FillKey, NormalizedEvent, OrderIntent, SystemEvent, SystemEventKind, TimeMs};
use reap_engine::{ChaosEngineOutput, SafetyCancelCandidate, TradingEngine};
use reap_feed::{FeedOutput, RecoveryRequest};
#[cfg(test)]
use reap_order::SubmitOutcome;
use reap_order::{
    ApprovedRegularCancel, ClientOrderIdGenerator, OwnedRegularOrders, PrivateOrderIdentityError,
    PrivateStateReducer, RegularApprovalScope, RegularExecutionPolicy, RegularExecutionPolicyError,
    ReservedRegularSubmit,
};
use reap_risk::{RiskDecision, RiskGate};
#[cfg(test)]
use reap_storage::OrderOperation;
use reap_storage::{
    ProvenRegularOrderBinding, ProvenRegularSubmitRequest, SafetyLatchRecord, SafetyLatchScope,
    SafetyLatchSource, StorageRecord,
};
use reap_strategy::{ChaosExecutionIntent, ChaosStrategy};
use thiserror::Error;

use crate::forbidden_orders::ForbiddenOrderEvent;
use crate::regular_execution::regular_execution_policy;
use crate::{LiveConfig, ReadinessSnapshot, StartupError, StartupGate, VerifiedBootstrap};

mod account_reconciliation;
mod private_feed;

#[derive(Debug)]
pub(crate) struct SubmitAction {
    ts_ms: TimeMs,
    idempotency_key: String,
    reserved: ReservedRegularSubmit,
}

impl SubmitAction {
    pub(crate) fn new(
        ts_ms: TimeMs,
        idempotency_key: String,
        reserved: ReservedRegularSubmit,
    ) -> Self {
        Self {
            ts_ms,
            idempotency_key,
            reserved,
        }
    }

    pub(crate) fn ts_ms(&self) -> TimeMs {
        self.ts_ms
    }

    pub(crate) fn account_id(&self) -> &str {
        self.reserved.account_id()
    }

    pub(crate) fn idempotency_key(&self) -> &str {
        &self.idempotency_key
    }

    pub(crate) fn client_order_id(&self) -> &str {
        self.reserved.client_order_id()
    }

    pub(crate) fn order(&self) -> &reap_core::NewOrder {
        self.reserved.order()
    }

    pub(crate) fn into_parts(self) -> (String, ReservedRegularSubmit) {
        (self.idempotency_key, self.reserved)
    }
}

#[derive(Debug)]
pub(crate) struct CancelAction {
    ts_ms: TimeMs,
    approved: ApprovedRegularCancel,
}

impl CancelAction {
    pub(crate) fn new(ts_ms: TimeMs, approved: ApprovedRegularCancel) -> Self {
        Self { ts_ms, approved }
    }

    pub(crate) fn ts_ms(&self) -> TimeMs {
        self.ts_ms
    }

    pub(crate) fn account_id(&self) -> &str {
        self.approved.account_id()
    }

    pub(crate) fn symbol(&self) -> &str {
        self.approved.symbol()
    }

    pub(crate) fn client_order_id(&self) -> &str {
        self.approved.client_order_id()
    }

    pub(crate) fn reason(&self) -> &str {
        self.approved.reason()
    }

    pub(crate) fn into_approved(self) -> ApprovedRegularCancel {
        self.approved
    }
}

#[derive(Debug, Clone)]
pub(crate) struct ReconcileAction {
    pub(crate) ts_ms: TimeMs,
    pub(crate) account_id: String,
    pub(crate) reason: String,
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

#[derive(Debug)]
pub(crate) enum LiveAction {
    Submit(SubmitAction),
    Cancel(CancelAction),
    RecoverBook(RecoveryRequest),
    Reconcile(ReconcileAction),
}

#[derive(Debug, Default)]
pub struct CoordinatorOutput {
    pub(crate) actions: Vec<LiveAction>,
    pub records: Vec<StorageRecord>,
}

impl CoordinatorOutput {
    pub fn action_count(&self) -> usize {
        self.actions.len()
    }

    pub fn record_count(&self) -> usize {
        self.records.len()
    }

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
    #[error("durable regular-submit proof does not match recovered order {order_id}: {message}")]
    RecoveredOrderProof { order_id: String, message: String },
    #[error(transparent)]
    Startup(#[from] StartupError),
}

impl From<RegularExecutionPolicyError> for CoordinatorError {
    fn from(error: RegularExecutionPolicyError) -> Self {
        match error {
            RegularExecutionPolicyError::ClientIdSetup { account_id, source } => {
                Self::ClientIdSetup {
                    account_id,
                    message: source.to_string(),
                }
            }
            error => Self::RegularExecutionPolicy(error.to_string()),
        }
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
        approval_scopes: HashMap<String, RegularApprovalScope>,
        session_id: impl Into<String>,
    ) -> Result<Self, CoordinatorError> {
        Self::new_with_order_transports(config, verified, approval_scopes, session_id)
    }

    pub fn new_with_order_transports(
        config: LiveConfig,
        verified: VerifiedBootstrap,
        approval_scopes: HashMap<String, RegularApprovalScope>,
        session_id: impl Into<String>,
    ) -> Result<Self, CoordinatorError> {
        let session_id = session_id.into();
        if session_id.trim().is_empty() {
            return Err(CoordinatorError::EmptySessionId);
        }
        let strategy = ChaosStrategy::new(config.strategy.clone())
            .map_err(|error| CoordinatorError::Strategy(error.to_string()))?;
        let gateway_action_accounts = approval_scopes.keys().cloned().collect::<HashSet<_>>();
        let (regular_execution, client_ids) =
            regular_execution_policy(&config, &verified, approval_scopes)?;
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
        proof: ProvenRegularSubmitRequest,
        update: reap_core::OrderUpdate,
    ) -> Result<CoordinatorOutput, CoordinatorError> {
        let account_id = proof.account_id().to_string();
        if proof.symbol() != update.symbol || proof.client_order_id() != update.order_id {
            return Err(CoordinatorError::RecoveredOrderProof {
                order_id: update.order_id.clone(),
                message: format!(
                    "proof identifies {}/{} but update identifies {}/{}",
                    proof.symbol(),
                    proof.client_order_id(),
                    update.symbol,
                    update.order_id
                ),
            });
        }
        let expected = self
            .config
            .account_for_symbol(&update.symbol)
            .map(|account| account.id.clone())
            .unwrap_or_default();
        if expected != account_id {
            return Err(CoordinatorError::WrongOrderAccount {
                order_id: update.order_id.clone(),
                symbol: update.symbol.clone(),
                actual: account_id,
                expected,
            });
        }
        self.owned_regular_orders
            .register_recovered(&self.regular_execution, proof)?;
        self.private_state_mut(&account_id)?
            .restore_order_update(update.clone());
        Ok(self.process_normalized(NormalizedEvent::Order(update)))
    }

    pub(crate) fn restore_order_binding(
        &mut self,
        binding: ProvenRegularOrderBinding,
    ) -> Result<(), CoordinatorError> {
        let account_id = binding.account_id().to_string();
        let client_order_id = binding.client_order_id().to_string();
        let exchange_order_id = binding.exchange_order_id().to_string();
        if !self.owned_regular_orders.proves_identity(
            &client_order_id,
            &account_id,
            binding.symbol(),
        ) {
            return Err(CoordinatorError::RecoveredOrderProof {
                order_id: client_order_id.to_string(),
                message: "durable exchange binding does not match restored owned identity"
                    .to_string(),
            });
        }
        self.owned_regular_orders.bind_exchange_order_id(
            &account_id,
            &client_order_id,
            &exchange_order_id,
        )?;
        self.private_state_mut(&account_id)?
            .bind_exchange_order_id(&client_order_id, &exchange_order_id)
            .map_err(|source| CoordinatorError::PrivateOrderIdentity { account_id, source })
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
                self.process_private_account(account_id, update)
            }
            FeedOutput::PrivateOrder { account_id, update } => {
                self.process_private_order(account_id, update)
            }
            FeedOutput::PrivateFill { account_id, fill } => {
                self.process_private_fill(account_id, fill)
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
        let proof = test_recovered_submit_proof(account_id, &order.symbol, client_order_id);
        self.owned_regular_orders
            .register_recovered(&self.regular_execution, proof)?;
        let update = self.private_state_mut(account_id)?.register_local_order_at(
            client_order_id,
            order,
            ts_ms,
        );
        Ok(update
            .map(|update| self.process_normalized(NormalizedEvent::Order(update)))
            .unwrap_or_default())
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
                let (pending, reserved) = match self.owned_regular_orders.reserve_local(
                    approved,
                    client_order_id,
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
                output.actions.push(LiveAction::Submit(SubmitAction::new(
                    now_ms,
                    idempotency_key,
                    reserved,
                )));
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
fn test_recovered_submit_proof(
    account_id: &str,
    symbol: &str,
    client_order_id: &str,
) -> ProvenRegularSubmitRequest {
    let request = StorageRecord::OrderRequest(reap_storage::OrderRequestRecord {
        ts_ms: 1,
        account_id: account_id.to_string(),
        operation: OrderOperation::Submit,
        idempotency_key: Some(format!("test:{account_id}:{client_order_id}")),
        client_order_id: Some(client_order_id.to_string()),
        exchange_order_id: None,
        symbol: symbol.to_string(),
    });
    let mut journal = serde_json::to_vec(&serde_json::json!({
        "schema_version": 7,
        "record": request,
    }))
    .expect("test regular-submit request must serialize");
    journal.push(b'\n');
    let directory = tempfile::tempdir().expect("test journal directory must exist");
    let path = directory.path().join("coordinator-proof.jsonl");
    std::fs::write(&path, journal).expect("test journal must be written");
    let mut lease =
        reap_storage::acquire_storage_lease(&path).expect("test journal must be leased");
    reap_storage::recover_leased_jsonl(&mut lease)
        .expect("test regular-submit request must recover under its lease")
        .proven_regular_submit_requests
        .into_values()
        .next()
        .expect("test recovery must produce a regular-submit proof")
}

#[cfg(test)]
#[path = "../tests/coordinator_unit/mod.rs"]
mod tests;
