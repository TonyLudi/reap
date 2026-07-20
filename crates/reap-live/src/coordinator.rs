use std::cell::Cell;
use std::collections::{BTreeMap, HashMap, HashSet};

#[cfg(test)]
use reap_core::OrderIntent;
use reap_core::{FillKey, NormalizedEvent, SystemEvent, SystemEventKind, TimeMs};
use reap_engine::TradingEngine;
use reap_feed::{FeedOutput, RecoveryRequest};
#[cfg(test)]
use reap_order::SubmitOutcome;
use reap_order::{
    ApprovedRegularCancel, ClientOrderIdGenerator, OwnedRegularOrders, PrivateOrderIdentityError,
    PrivateStateReducer, RegularApprovalScope, RegularExecutionPolicy, RegularExecutionPolicyError,
    ReservedRegularSubmit,
};
use reap_risk::RiskGate;
#[cfg(test)]
use reap_storage::{OrderOperation, SafetyLatchScope, SafetyLatchSource};
use reap_storage::{ProvenRegularOrderBinding, ProvenRegularSubmitRequest, StorageRecord};
#[cfg(test)]
use reap_strategy::ChaosExecutionPurpose;
use reap_strategy::ChaosStrategy;
use thiserror::Error;

use crate::forbidden_orders::ForbiddenOrderEvent;
use crate::regular_execution::regular_execution_policy;
use crate::{LiveConfig, ReadinessSnapshot, StartupError, StartupGate, VerifiedBootstrap};

mod account_reconciliation;
mod private_feed;
mod reduction;
mod routing;

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
    #[cfg(test)]
    chaos_intent_trace: Vec<(ChaosExecutionPurpose, OrderIntent)>,
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
            #[cfg(test)]
            chaos_intent_trace: Vec::new(),
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
            FeedOutput::System(event) => {
                Ok(self.process_normalized_at(NormalizedEvent::System(event), observed_now_ms))
            }
            FeedOutput::PrivateAccount { account_id, update } => {
                self.process_private_account_at(account_id, update, observed_now_ms)
            }
            FeedOutput::PrivateOrder { account_id, update } => {
                self.process_private_order_at(account_id, update, observed_now_ms)
            }
            FeedOutput::PrivateFill { account_id, fill } => {
                self.process_private_fill_at(account_id, fill, observed_now_ms)
            }
            FeedOutput::Duplicate(_) => Ok(CoordinatorOutput::default()),
            FeedOutput::RecoveryRequired(request) => Ok(CoordinatorOutput {
                actions: vec![LiveAction::RecoverBook(request)],
                records: Vec::new(),
            }),
        }
    }

    #[cfg(test)]
    pub(crate) fn process_feed_arrived_at(
        &mut self,
        output: FeedOutput,
        observed_now_ms: TimeMs,
        arrival_ns: u64,
    ) -> Result<CoordinatorOutput, CoordinatorError> {
        match output {
            FeedOutput::Event(event) => {
                Ok(self.process_normalized_arrived_at(event, observed_now_ms, arrival_ns))
            }
            output => self.process_feed_at(output, observed_now_ms),
        }
    }

    pub(crate) fn process_feed_received_at(
        &mut self,
        output: FeedOutput,
        observed_now_ms: TimeMs,
        receipt_ns: u64,
        processing_clock: impl FnOnce() -> (u64, TimeMs),
        mut finish_clock: impl FnMut() -> TimeMs,
    ) -> Result<CoordinatorOutput, CoordinatorError> {
        match output {
            FeedOutput::Event(event) => Ok(self.process_normalized_received_at(
                event,
                observed_now_ms,
                receipt_ns,
                processing_clock,
                finish_clock,
            )),
            FeedOutput::System(event) => Ok(self.process_normalized_at_with_clock(
                NormalizedEvent::System(event),
                observed_now_ms,
                &mut finish_clock,
            )),
            FeedOutput::PrivateAccount { account_id, update } => self
                .process_private_account_at_with_clock(
                    account_id,
                    update,
                    observed_now_ms,
                    &mut finish_clock,
                ),
            FeedOutput::PrivateOrder { account_id, update } => self
                .process_private_order_at_with_clock(
                    account_id,
                    update,
                    observed_now_ms,
                    &mut finish_clock,
                ),
            FeedOutput::PrivateFill { account_id, fill } => self
                .process_private_fill_at_with_clock(
                    account_id,
                    fill,
                    observed_now_ms,
                    &mut finish_clock,
                ),
            FeedOutput::Duplicate(_) => Ok(CoordinatorOutput::default()),
            FeedOutput::RecoveryRequired(request) => Ok(CoordinatorOutput {
                actions: vec![LiveAction::RecoverBook(request)],
                records: Vec::new(),
            }),
        }
    }

    pub(crate) fn next_trade_reprice_due_ns(&self) -> Option<u64> {
        self.engine.next_chaos_trade_reprice_due_ns()
    }

    #[cfg(test)]
    pub(crate) fn service_one_due_trade_reprice(
        &mut self,
        now_ns: u64,
        observed_now_ms: TimeMs,
    ) -> CoordinatorOutput {
        self.service_one_due_trade_reprice_with_finish_clock(now_ns, observed_now_ms, || {
            observed_now_ms
        })
    }

    #[cfg(test)]
    pub(crate) fn service_one_due_trade_reprice_with_finish_clock(
        &mut self,
        now_ns: u64,
        observed_now_ms: TimeMs,
        finish_clock: impl FnMut() -> TimeMs,
    ) -> CoordinatorOutput {
        self.service_one_due_trade_reprice_with_clocks(|| (now_ns, observed_now_ms), finish_clock)
    }

    pub(crate) fn service_one_due_trade_reprice_with_clocks(
        &mut self,
        start_clock: impl FnOnce() -> (u64, TimeMs),
        mut finish_clock: impl FnMut() -> TimeMs,
    ) -> CoordinatorOutput {
        let strategy_is_live = self.strategy_is_live();
        let sampled_start = Cell::new(None);
        let engine_output = self.engine.service_one_due_chaos_trade_reprice_with_clocks(
            || {
                let start = start_clock();
                sampled_start.set(Some(start));
                start
            },
            strategy_is_live,
            &mut finish_clock,
        );
        let (_, observed_now_ms) = sampled_start
            .get()
            .expect("due trade-reprice start clock must be sampled");
        let mut output = CoordinatorOutput::default();
        let routing_observed_now_ms = finish_clock();
        self.handle_engine_output(
            observed_now_ms,
            routing_observed_now_ms,
            engine_output,
            &mut finish_clock,
            &mut output,
        );
        output
    }

    pub fn process_event(&mut self, event: NormalizedEvent) -> CoordinatorOutput {
        self.process_normalized(event)
    }

    fn strategy_is_live(&self) -> bool {
        self.startup.can_submit_new(self.order_entry_enabled) && !self.engine.risk().is_killed()
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
    pub(crate) fn register_local_order(
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
