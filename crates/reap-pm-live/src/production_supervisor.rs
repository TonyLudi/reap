//! Fail-closed supervision for continuously running Polymarket execution.
//!
//! The supervisor deliberately does not contain a strategy. It joins four
//! purpose-specific production roles around one canonical actor: heartbeat,
//! private WebSocket, authoritative polling, and exact mutation. Every place
//! intent is durable before dispatch, fills are deduplicated into an internal
//! position, and each complete poll reconciles that projection with venue
//! positions. Startup remains closed until a complete poll repairs recovery;
//! shutdown cancels only supervisor-owned orders and requires a terminal
//! complete zero-open-order cut.

use std::{
    collections::{BTreeMap, BTreeSet},
    fmt,
    path::PathBuf,
    time::Duration,
};

use async_trait::async_trait;
use reap_pm_core::{PmOrderSide, U256};
use serde::{Deserialize, Serialize};
use thiserror::Error;
use tokio::{
    sync::{mpsc, oneshot, watch},
    task::{JoinError, JoinHandle},
};

pub use crate::production_supervisor_journal::{
    PmSupervisorJournal, PmSupervisorJournalError, PmSupervisorJournalFill,
    PmSupervisorJournalRecord, PmSupervisorJournalRecovery,
};
pub use crate::production_supervisor_roles::{
    PmSupervisorFixedHeartbeatRole, PmSupervisorFixedMutationRole,
    PmSupervisorProductionMutationRole,
};

pub const MAX_PM_SUPERVISOR_ORDERS: usize = 4_096;
pub const MAX_PM_SUPERVISOR_FILLS: usize = 65_536;
pub const MAX_PM_SUPERVISOR_TOKENS: usize = 32;
/// The strategy-neutral production supervisor and concrete fixed heartbeat /
/// mutation adapters are available. This does not authorize a strategy or a
/// continuously composed product executable by itself.
pub const PRODUCTION_SUPERVISOR_INFRA_AVAILABLE: bool = true;
const MAX_PM_SUPERVISOR_INGRESS: usize = 1_024;
const MAX_PM_SUPERVISOR_COMMANDS: usize = 128;

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct PmSupervisorScope {
    condition_id: String,
    token_ids: Box<[String]>,
}

impl PmSupervisorScope {
    pub fn new(
        condition_id: impl Into<String>,
        token_ids: impl IntoIterator<Item = String>,
    ) -> Result<Self, PmProductionSupervisorError> {
        let condition_id = condition_id.into();
        let token_ids = token_ids.into_iter().collect::<BTreeSet<_>>();
        if condition_id.is_empty()
            || condition_id.len() > 256
            || token_ids.is_empty()
            || token_ids.len() > MAX_PM_SUPERVISOR_TOKENS
            || token_ids
                .iter()
                .any(|token| token.is_empty() || token.len() > 128)
        {
            return Err(PmProductionSupervisorError::InvalidConfiguration);
        }
        Ok(Self {
            condition_id,
            token_ids: token_ids.into_iter().collect::<Vec<_>>().into_boxed_slice(),
        })
    }

    #[must_use]
    pub fn condition_id(&self) -> &str {
        &self.condition_id
    }

    #[must_use]
    pub fn token_ids(&self) -> &[String] {
        &self.token_ids
    }

    fn contains_token(&self, token: &str) -> bool {
        self.token_ids
            .binary_search_by(|candidate| candidate.as_str().cmp(token))
            .is_ok()
    }
}

#[derive(Debug, Clone)]
pub struct PmProductionSupervisorConfig {
    scope: PmSupervisorScope,
    poll_interval: Duration,
    heartbeat_interval: Duration,
    shutdown_timeout: Duration,
    maximum_orders: usize,
    maximum_fills: usize,
}

impl PmProductionSupervisorConfig {
    pub fn new(
        scope: PmSupervisorScope,
        poll_interval: Duration,
        heartbeat_interval: Duration,
        shutdown_timeout: Duration,
    ) -> Result<Self, PmProductionSupervisorError> {
        if poll_interval.is_zero()
            || heartbeat_interval.is_zero()
            || shutdown_timeout.is_zero()
            || poll_interval > Duration::from_secs(60)
            || heartbeat_interval > Duration::from_secs(10)
            || shutdown_timeout > Duration::from_secs(300)
        {
            return Err(PmProductionSupervisorError::InvalidConfiguration);
        }
        Ok(Self {
            scope,
            poll_interval,
            heartbeat_interval,
            shutdown_timeout,
            maximum_orders: MAX_PM_SUPERVISOR_ORDERS,
            maximum_fills: MAX_PM_SUPERVISOR_FILLS,
        })
    }

    #[must_use]
    pub fn scope(&self) -> &PmSupervisorScope {
        &self.scope
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum PmSupervisorMutationClassification {
    DefinitelyNotDispatched,
    Accepted,
    Rejected,
    AcknowledgementUnknown,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum PmSupervisorOrderStatus {
    PendingNew,
    Live,
    PartiallyFilled,
    Filled,
    PendingCancel,
    Cancelled,
    Rejected,
    Expired,
    ReconciliationRequired,
}

impl PmSupervisorOrderStatus {
    const fn terminal(self) -> bool {
        matches!(
            self,
            Self::Filled | Self::Cancelled | Self::Rejected | Self::Expired
        )
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct PmSupervisorOrderFacts {
    client_order_id: String,
    expected_venue_order_id: String,
    token_id: String,
    #[serde(with = "crate::production_supervisor_serde")]
    side: PmOrderSide,
    quantity: U256,
}

impl PmSupervisorOrderFacts {
    pub fn new(
        client_order_id: impl Into<String>,
        expected_venue_order_id: impl Into<String>,
        token_id: impl Into<String>,
        side: PmOrderSide,
        quantity: U256,
    ) -> Result<Self, PmProductionSupervisorError> {
        let facts = Self {
            client_order_id: client_order_id.into(),
            expected_venue_order_id: expected_venue_order_id.into(),
            token_id: token_id.into(),
            side,
            quantity,
        };
        if facts.client_order_id.is_empty()
            || facts.client_order_id.len() > 128
            || facts.expected_venue_order_id.is_empty()
            || facts.expected_venue_order_id.len() > 128
            || facts.token_id.is_empty()
            || facts.token_id.len() > 128
            || facts.quantity.is_zero()
        {
            return Err(PmProductionSupervisorError::InvalidOrderFacts);
        }
        Ok(facts)
    }

    #[must_use]
    pub fn client_order_id(&self) -> &str {
        &self.client_order_id
    }

    #[must_use]
    pub fn expected_venue_order_id(&self) -> &str {
        &self.expected_venue_order_id
    }

    #[must_use]
    pub fn token_id(&self) -> &str {
        &self.token_id
    }

    #[must_use]
    pub const fn side(&self) -> PmOrderSide {
        self.side
    }

    #[must_use]
    pub const fn quantity(&self) -> U256 {
        self.quantity
    }
}

pub struct PmSupervisorPlaceCommand<Request> {
    facts: PmSupervisorOrderFacts,
    request: Request,
}

impl<Request> PmSupervisorPlaceCommand<Request> {
    #[must_use]
    pub const fn new(facts: PmSupervisorOrderFacts, request: Request) -> Self {
        Self { facts, request }
    }
}

impl<Request> fmt::Debug for PmSupervisorPlaceCommand<Request> {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("PmSupervisorPlaceCommand")
            .field("facts", &self.facts)
            .field("request", &"<opaque; move-only>")
            .finish()
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PmSupervisorPlaceResult {
    pub classification: PmSupervisorMutationClassification,
    pub observed_venue_order_id: Option<String>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct PmSupervisorCancelResult {
    pub classification: PmSupervisorMutationClassification,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PmSupervisorOpenOrder {
    pub venue_order_id: String,
    pub token_id: String,
    pub status: PmSupervisorOrderStatus,
    pub cumulative_filled: U256,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PmSupervisorFill {
    pub fill_id: String,
    pub venue_order_id: String,
    pub token_id: String,
    pub side: PmOrderSide,
    pub quantity: U256,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PmSupervisorPosition {
    pub token_id: String,
    pub quantity: U256,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PmSupervisorPollCut {
    pub sequence: u64,
    pub open_orders: Box<[PmSupervisorOpenOrder]>,
    pub fills: Box<[PmSupervisorFill]>,
    pub positions: Box<[PmSupervisorPosition]>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum PmSupervisorWsEvent {
    /// The authenticated subscription was written on a live connection. A
    /// later complete poll is still required before order entry can reopen.
    Connected,
    /// The private stream retired, exhausted retries, or began shutdown.
    Disconnected,
    /// A private lifecycle transition cannot be represented as an
    /// irreversible fill and requires a fresh authoritative poll.
    ReconciliationRequired,
    Order(PmSupervisorOpenOrder),
    Fill(PmSupervisorFill),
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PmSupervisorPositionReconciliation {
    pub token_id: String,
    pub baseline: U256,
    pub fill_based: U256,
    pub authoritative: U256,
    pub converged: bool,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PmSupervisorOrderProjection {
    pub facts: PmSupervisorOrderFacts,
    pub status: PmSupervisorOrderStatus,
    pub cumulative_filled: U256,
    /// Sum of unique fill records applied by the supervisor. Readiness closes
    /// whenever a venue order snapshot advances beyond this value.
    pub known_filled: U256,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Error)]
pub enum PmSupervisorEdgeError {
    #[error("production edge is unavailable")]
    Unavailable,
    #[error("production edge returned an invalid or out-of-scope observation")]
    InvalidObservation,
}

#[async_trait]
pub trait PmSupervisorHeartbeatRole: Send + 'static {
    async fn heartbeat(&mut self) -> Result<(), PmSupervisorEdgeError>;
}

#[async_trait]
pub trait PmSupervisorPollRole: Send + 'static {
    async fn complete_poll(&mut self) -> Result<PmSupervisorPollCut, PmSupervisorEdgeError>;
}

#[async_trait]
pub trait PmSupervisorWsRole: Send + 'static {
    async fn next_event(&mut self) -> Result<PmSupervisorWsEvent, PmSupervisorEdgeError>;

    async fn shutdown(&mut self) -> Result<(), PmSupervisorEdgeError> {
        Ok(())
    }
}

#[async_trait]
pub trait PmSupervisorMutationRole: Send + 'static {
    type PlaceRequest: Send + 'static;

    /// Validate the public command facts against the opaque mutation request
    /// before the durable intent is written or any signing can occur.
    fn validate_place(&self, facts: &PmSupervisorOrderFacts, request: &Self::PlaceRequest) -> bool;

    async fn place(
        &mut self,
        request: Self::PlaceRequest,
    ) -> Result<PmSupervisorPlaceResult, PmSupervisorEdgeError>;

    async fn cancel_exact(
        &mut self,
        venue_order_id: &str,
        token_id: &str,
    ) -> Result<PmSupervisorCancelResult, PmSupervisorEdgeError>;
}

pub struct PmProductionSupervisorRoles<H, P, W, M> {
    pub heartbeat: H,
    pub poll: P,
    pub user_ws: W,
    pub mutation: M,
}

#[derive(Debug, Error)]
pub enum PmProductionSupervisorError {
    #[error("production supervisor configuration is invalid")]
    InvalidConfiguration,
    #[error("production supervisor order facts are invalid")]
    InvalidOrderFacts,
    #[error("production supervisor recovery is inconsistent")]
    Recovery,
    #[error("production supervisor journal failed")]
    Journal(#[from] PmSupervisorJournalError),
    #[error("a supervised production role failed")]
    RoleFailed,
    #[error("a supervised production task failed")]
    TaskFailed,
    #[error("production supervisor controlled shutdown timed out")]
    ShutdownTimeout,
    #[error("production supervisor shutdown did not reach a complete zero-open-order cut")]
    ShutdownUnreconciled,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Error)]
pub enum PmSupervisorCommandError {
    #[error("production supervisor is not ready")]
    NotReady,
    #[error("production supervisor is closed")]
    Closed,
    #[error("production supervisor scope or identity does not match")]
    ScopeMismatch,
    #[error("production supervisor capacity is exhausted")]
    Capacity,
    #[error("production mutation failed")]
    Mutation,
    #[error("durable mutation intent or outcome failed")]
    Durability,
    #[error("production order state became contradictory")]
    Contradiction,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PmSupervisorShutdownReport {
    /// Number of durable exact-cancel intents in this journal lifetime. An
    /// intent recovered without a result is deliberately counted because the
    /// prior process may have crossed the dispatch boundary.
    pub durable_exact_cancel_intents: usize,
    pub terminal_poll_sequence: u64,
    pub positions: Box<[PmSupervisorPositionReconciliation]>,
}

pub struct PmProductionSupervisor;

pub struct PmProductionSupervisorHandle<Request> {
    commands: mpsc::Sender<Command<Request>>,
    task: Option<JoinHandle<Result<PmSupervisorShutdownReport, PmProductionSupervisorError>>>,
    shutdown_timeout: Duration,
}

impl<Request: Send + 'static> PmProductionSupervisorHandle<Request> {
    pub async fn place(
        &self,
        command: PmSupervisorPlaceCommand<Request>,
    ) -> Result<PmSupervisorOrderProjection, PmSupervisorCommandError> {
        let (send, receive) = oneshot::channel();
        self.commands
            .send(Command::Place(command, send))
            .await
            .map_err(|_| PmSupervisorCommandError::Closed)?;
        receive
            .await
            .map_err(|_| PmSupervisorCommandError::Closed)?
    }

    pub async fn cancel_exact(
        &self,
        expected_venue_order_id: impl Into<String>,
    ) -> Result<PmSupervisorOrderProjection, PmSupervisorCommandError> {
        let (send, receive) = oneshot::channel();
        self.commands
            .send(Command::Cancel(expected_venue_order_id.into(), send))
            .await
            .map_err(|_| PmSupervisorCommandError::Closed)?;
        receive
            .await
            .map_err(|_| PmSupervisorCommandError::Closed)?
    }

    pub async fn shutdown(
        mut self,
    ) -> Result<PmSupervisorShutdownReport, PmProductionSupervisorError> {
        let (send, receive) = oneshot::channel();
        self.commands
            .send(Command::Shutdown(send))
            .await
            .map_err(|_| PmProductionSupervisorError::TaskFailed)?;
        // The actor task owns the authoritative error. The response exists so
        // command handling remains uniform, but awaiting it first would hide a
        // specific actor failure behind a generic channel error.
        drop(receive);
        let mut task = self
            .task
            .take()
            .ok_or(PmProductionSupervisorError::TaskFailed)?;
        match tokio::time::timeout(self.shutdown_timeout, &mut task).await {
            Ok(joined) => joined.map_err(|_| PmProductionSupervisorError::TaskFailed)?,
            Err(_) => {
                // Every mutation intent is already durable before dispatch;
                // aborting at this bound is therefore recovered as an
                // ambiguous exact-place/cancel state on the next start.
                task.abort();
                let _ = task.await;
                Err(PmProductionSupervisorError::ShutdownTimeout)
            }
        }
    }
}

impl<Request> Drop for PmProductionSupervisorHandle<Request> {
    fn drop(&mut self) {
        if let Some(task) = self.task.take() {
            task.abort();
        }
    }
}

impl PmProductionSupervisor {
    pub async fn start<H, P, W, M>(
        config: PmProductionSupervisorConfig,
        journal_path: PathBuf,
        roles: PmProductionSupervisorRoles<H, P, W, M>,
    ) -> Result<PmProductionSupervisorHandle<M::PlaceRequest>, PmProductionSupervisorError>
    where
        H: PmSupervisorHeartbeatRole,
        P: PmSupervisorPollRole,
        W: PmSupervisorWsRole,
        M: PmSupervisorMutationRole,
    {
        let (journal, recovery) = PmSupervisorJournal::open(journal_path, &config.scope).await?;
        let state = SupervisorState::recover(&config, recovery)?;
        let (ingress_send, ingress_receive) = mpsc::channel(MAX_PM_SUPERVISOR_INGRESS);
        let (stop_send, stop_receive) = watch::channel(false);
        let heartbeat = spawn_heartbeat(
            roles.heartbeat,
            config.heartbeat_interval,
            ingress_send.clone(),
            stop_receive.clone(),
        );
        let poll = spawn_poll(
            roles.poll,
            config.poll_interval,
            ingress_send.clone(),
            stop_receive.clone(),
        );
        let user_ws = spawn_ws(roles.user_ws, ingress_send, stop_receive);
        let (commands, command_receive) = mpsc::channel(MAX_PM_SUPERVISOR_COMMANDS);
        let shutdown_timeout = config.shutdown_timeout;
        let task = tokio::spawn(run_actor(
            config,
            state,
            journal,
            roles.mutation,
            ingress_receive,
            command_receive,
            stop_send,
            [heartbeat, poll, user_ws],
        ));
        Ok(PmProductionSupervisorHandle {
            commands,
            task: Some(task),
            shutdown_timeout,
        })
    }
}

enum Command<Request> {
    Place(
        PmSupervisorPlaceCommand<Request>,
        oneshot::Sender<Result<PmSupervisorOrderProjection, PmSupervisorCommandError>>,
    ),
    Cancel(
        String,
        oneshot::Sender<Result<PmSupervisorOrderProjection, PmSupervisorCommandError>>,
    ),
    Shutdown(oneshot::Sender<Result<PmSupervisorShutdownReport, PmProductionSupervisorError>>),
}

enum Ingress {
    Poll(PmSupervisorPollCut),
    Ws(PmSupervisorWsEvent),
    Fatal,
}

struct TaskGuard {
    task: Option<JoinHandle<()>>,
}

impl TaskGuard {
    fn new(task: JoinHandle<()>) -> Self {
        Self { task: Some(task) }
    }

    async fn join(mut self) -> Result<(), JoinError> {
        let result = self.task.as_mut().expect("task is consumed once").await;
        self.task.take();
        result
    }
}

impl Drop for TaskGuard {
    fn drop(&mut self) {
        if let Some(task) = self.task.take() {
            task.abort();
        }
    }
}

type TaskArray = [TaskGuard; 3];

fn spawn_heartbeat<H: PmSupervisorHeartbeatRole>(
    mut role: H,
    interval: Duration,
    ingress: mpsc::Sender<Ingress>,
    mut stop: watch::Receiver<bool>,
) -> TaskGuard {
    TaskGuard::new(tokio::spawn(async move {
        let mut ticker = tokio::time::interval(interval);
        ticker.set_missed_tick_behavior(tokio::time::MissedTickBehavior::Delay);
        loop {
            tokio::select! {
                changed = stop.changed() => {
                    if changed.is_err() || *stop.borrow() { break; }
                }
                _ = ticker.tick() => {
                    tokio::select! {
                        changed = stop.changed() => {
                            if changed.is_err() || *stop.borrow() { break; }
                        }
                        result = role.heartbeat() => {
                            if result.is_err() {
                                let _ = ingress.send(Ingress::Fatal).await;
                                break;
                            }
                        }
                    }
                }
            }
        }
    }))
}

fn spawn_poll<P: PmSupervisorPollRole>(
    mut role: P,
    interval: Duration,
    ingress: mpsc::Sender<Ingress>,
    mut stop: watch::Receiver<bool>,
) -> TaskGuard {
    TaskGuard::new(tokio::spawn(async move {
        let mut ticker = tokio::time::interval(interval);
        ticker.set_missed_tick_behavior(tokio::time::MissedTickBehavior::Delay);
        loop {
            tokio::select! {
                changed = stop.changed() => {
                    if changed.is_err() || *stop.borrow() { break; }
                }
                _ = ticker.tick() => {
                    tokio::select! {
                        changed = stop.changed() => {
                            if changed.is_err() || *stop.borrow() { break; }
                        }
                        result = role.complete_poll() => match result {
                            Ok(cut) => {
                                if ingress.send(Ingress::Poll(cut)).await.is_err() {
                                    break;
                                }
                            }
                            Err(_) => {
                                let _ = ingress.send(Ingress::Fatal).await;
                                break;
                            }
                        }
                    }
                }
            }
        }
    }))
}

fn spawn_ws<W: PmSupervisorWsRole>(
    mut role: W,
    ingress: mpsc::Sender<Ingress>,
    mut stop: watch::Receiver<bool>,
) -> TaskGuard {
    TaskGuard::new(tokio::spawn(async move {
        loop {
            tokio::select! {
                changed = stop.changed() => {
                    if changed.is_err() || *stop.borrow() { break; }
                }
                event = role.next_event() => {
                    match event {
                        Ok(event) => {
                            if ingress.send(Ingress::Ws(event)).await.is_err() {
                                break;
                            }
                        }
                        Err(_) => {
                            let _ = ingress.send(Ingress::Fatal).await;
                            break;
                        }
                    }
                }
            }
        }
        assert!(
            role.shutdown().await.is_ok(),
            "supervised private WebSocket shutdown failed"
        );
    }))
}

struct OrderState {
    facts: PmSupervisorOrderFacts,
    status: PmSupervisorOrderStatus,
    cumulative_filled: U256,
    known_filled: U256,
    cancel_requested: bool,
}

struct PositionState {
    baseline: U256,
    bought: U256,
    sold: U256,
    authoritative: Option<U256>,
}

struct SupervisorState {
    orders: BTreeMap<String, OrderState>,
    fills: BTreeSet<(String, String)>,
    positions: BTreeMap<String, PositionState>,
    durable_exact_cancel_intents: usize,
    ready: bool,
    private_ws_connected: bool,
    last_poll_sequence: Option<u64>,
}

impl SupervisorState {
    fn recover(
        config: &PmProductionSupervisorConfig,
        recovery: PmSupervisorJournalRecovery,
    ) -> Result<Self, PmProductionSupervisorError> {
        let mut state = Self {
            orders: BTreeMap::new(),
            fills: BTreeSet::new(),
            positions: BTreeMap::new(),
            durable_exact_cancel_intents: 0,
            ready: false,
            private_ws_connected: false,
            last_poll_sequence: None,
        };
        for record in recovery.records.into_vec() {
            match record {
                PmSupervisorJournalRecord::Header { .. } => {}
                PmSupervisorJournalRecord::PositionBaseline { token_id, quantity } => {
                    if !config.scope.contains_token(&token_id)
                        || state
                            .positions
                            .insert(
                                token_id,
                                PositionState {
                                    baseline: quantity,
                                    bought: U256::ZERO,
                                    sold: U256::ZERO,
                                    authoritative: None,
                                },
                            )
                            .is_some()
                    {
                        return Err(PmProductionSupervisorError::Recovery);
                    }
                }
                PmSupervisorJournalRecord::PlaceIntent { facts } => {
                    if !config.scope.contains_token(&facts.token_id)
                        || state.orders.len() >= config.maximum_orders
                        || state
                            .orders
                            .insert(
                                facts.expected_venue_order_id.clone(),
                                OrderState {
                                    facts,
                                    status: PmSupervisorOrderStatus::PendingNew,
                                    cumulative_filled: U256::ZERO,
                                    known_filled: U256::ZERO,
                                    cancel_requested: false,
                                },
                            )
                            .is_some()
                    {
                        return Err(PmProductionSupervisorError::Recovery);
                    }
                }
                PmSupervisorJournalRecord::PlaceResult {
                    expected_venue_order_id,
                    classification,
                } => {
                    let order = state
                        .orders
                        .get_mut(&expected_venue_order_id)
                        .ok_or(PmProductionSupervisorError::Recovery)?;
                    order.status = place_status(classification);
                }
                PmSupervisorJournalRecord::CancelIntent { venue_order_id } => {
                    state.durable_exact_cancel_intents = state
                        .durable_exact_cancel_intents
                        .checked_add(1)
                        .ok_or(PmProductionSupervisorError::Recovery)?;
                    let order = state
                        .orders
                        .get_mut(&venue_order_id)
                        .ok_or(PmProductionSupervisorError::Recovery)?;
                    if !order.status.terminal() {
                        order.status = PmSupervisorOrderStatus::PendingCancel;
                        order.cancel_requested = true;
                    }
                }
                PmSupervisorJournalRecord::CancelResult {
                    venue_order_id,
                    classification,
                } => {
                    let order = state
                        .orders
                        .get_mut(&venue_order_id)
                        .ok_or(PmProductionSupervisorError::Recovery)?;
                    order.cancel_requested = matches!(
                        classification,
                        PmSupervisorMutationClassification::Accepted
                            | PmSupervisorMutationClassification::AcknowledgementUnknown
                    );
                    order.status = cancel_status(order.status, classification);
                }
                PmSupervisorJournalRecord::FillApplied { fill } => {
                    state.apply_fill(config, fill.into(), true)?;
                }
                PmSupervisorJournalRecord::PollReconciled { sequence } => {
                    state.last_poll_sequence = Some(sequence);
                }
                PmSupervisorJournalRecord::CleanShutdown { .. } => {}
            }
        }
        Ok(state)
    }

    fn projection(&self, venue_order_id: &str) -> Option<PmSupervisorOrderProjection> {
        self.orders
            .get(venue_order_id)
            .map(|order| PmSupervisorOrderProjection {
                facts: order.facts.clone(),
                status: order.status,
                cumulative_filled: order.cumulative_filled,
                known_filled: order.known_filled,
            })
    }

    fn apply_fill(
        &mut self,
        config: &PmProductionSupervisorConfig,
        fill: PmSupervisorFill,
        require_known_order: bool,
    ) -> Result<bool, PmProductionSupervisorError> {
        if fill.fill_id.is_empty()
            || fill.fill_id.len() > 128
            || fill.quantity.is_zero()
            || !config.scope.contains_token(&fill.token_id)
        {
            return Err(PmProductionSupervisorError::RoleFailed);
        }
        let fill_key = (fill.fill_id.clone(), fill.venue_order_id.clone());
        if self.fills.contains(&fill_key) {
            return Ok(false);
        }
        if self.fills.len() >= config.maximum_fills {
            return Err(PmProductionSupervisorError::RoleFailed);
        }
        let Some(order) = self.orders.get_mut(&fill.venue_order_id) else {
            return if require_known_order {
                Err(PmProductionSupervisorError::RoleFailed)
            } else {
                Ok(false)
            };
        };
        if order.facts.token_id != fill.token_id || order.facts.side != fill.side {
            return Err(PmProductionSupervisorError::RoleFailed);
        }
        order.known_filled = order
            .known_filled
            .checked_add(fill.quantity)
            .map_err(|_| PmProductionSupervisorError::RoleFailed)?;
        if order.known_filled > order.facts.quantity {
            return Err(PmProductionSupervisorError::RoleFailed);
        }
        order.cumulative_filled = order.cumulative_filled.max(order.known_filled);
        order.status = if order.cumulative_filled == order.facts.quantity {
            PmSupervisorOrderStatus::Filled
        } else {
            PmSupervisorOrderStatus::PartiallyFilled
        };
        let position = self
            .positions
            .get_mut(&fill.token_id)
            .ok_or(PmProductionSupervisorError::RoleFailed)?;
        match fill.side {
            PmOrderSide::Buy => {
                position.bought = position
                    .bought
                    .checked_add(fill.quantity)
                    .map_err(|_| PmProductionSupervisorError::RoleFailed)?;
            }
            PmOrderSide::Sell => {
                position.sold = position
                    .sold
                    .checked_add(fill.quantity)
                    .map_err(|_| PmProductionSupervisorError::RoleFailed)?;
            }
        }
        self.fills.insert(fill_key);
        Ok(true)
    }

    fn fill_based(position: &PositionState) -> Result<U256, PmProductionSupervisorError> {
        position
            .baseline
            .checked_add(position.bought)
            .and_then(|value| value.checked_sub(position.sold))
            .map_err(|_| PmProductionSupervisorError::RoleFailed)
    }

    fn position_report(
        &self,
    ) -> Result<Box<[PmSupervisorPositionReconciliation]>, PmProductionSupervisorError> {
        self.positions
            .iter()
            .map(|(token_id, position)| {
                let fill_based = Self::fill_based(position)?;
                let authoritative = position
                    .authoritative
                    .ok_or(PmProductionSupervisorError::ShutdownUnreconciled)?;
                Ok(PmSupervisorPositionReconciliation {
                    token_id: token_id.clone(),
                    baseline: position.baseline,
                    fill_based,
                    authoritative,
                    converged: fill_based == authoritative,
                })
            })
            .collect::<Result<Vec<_>, _>>()
            .map(Vec::into_boxed_slice)
    }
}

fn place_status(classification: PmSupervisorMutationClassification) -> PmSupervisorOrderStatus {
    match classification {
        PmSupervisorMutationClassification::Accepted => PmSupervisorOrderStatus::Live,
        PmSupervisorMutationClassification::DefinitelyNotDispatched
        | PmSupervisorMutationClassification::Rejected => PmSupervisorOrderStatus::Rejected,
        PmSupervisorMutationClassification::AcknowledgementUnknown => {
            PmSupervisorOrderStatus::ReconciliationRequired
        }
    }
}

fn cancel_status(
    _current: PmSupervisorOrderStatus,
    classification: PmSupervisorMutationClassification,
) -> PmSupervisorOrderStatus {
    match classification {
        PmSupervisorMutationClassification::Accepted => PmSupervisorOrderStatus::PendingCancel,
        PmSupervisorMutationClassification::DefinitelyNotDispatched
        | PmSupervisorMutationClassification::Rejected
        | PmSupervisorMutationClassification::AcknowledgementUnknown => {
            PmSupervisorOrderStatus::ReconciliationRequired
        }
    }
}

#[allow(clippy::too_many_arguments)]
async fn run_actor<M: PmSupervisorMutationRole>(
    config: PmProductionSupervisorConfig,
    mut state: SupervisorState,
    mut journal: PmSupervisorJournal,
    mut mutation: M,
    mut ingress: mpsc::Receiver<Ingress>,
    mut commands: mpsc::Receiver<Command<M::PlaceRequest>>,
    stop: watch::Sender<bool>,
    tasks: TaskArray,
) -> Result<PmSupervisorShutdownReport, PmProductionSupervisorError> {
    let mut shutdown_response = None;
    let result = loop {
        tokio::select! {
            biased;
            incoming = ingress.recv() => match incoming {
                Some(Ingress::Poll(cut)) => {
                    if let Err(error) = apply_poll(&config, &mut state, &mut journal, cut).await {
                        state.ready = false;
                        let _ = cancel_each_owned(&mut state, &mut journal, &mut mutation).await;
                        break Err(error);
                    }
                    if shutdown_response.is_some() {
                        match terminal_shutdown_report(&state) {
                            Ok(report) => break Ok(report),
                            Err(PmProductionSupervisorError::ShutdownUnreconciled) => {}
                            Err(error) => break Err(error),
                        }
                    }
                }
                Some(Ingress::Ws(event)) => {
                    if let Err(error) = apply_ws(&config, &mut state, &mut journal, event).await {
                        state.ready = false;
                        let _ = cancel_each_owned(&mut state, &mut journal, &mut mutation).await;
                        break Err(error);
                    }
                }
                Some(Ingress::Fatal) | None => {
                    state.ready = false;
                    // A liveness/read-edge failure closes entry immediately.
                    // Exact cancellation is best-effort because the failed
                    // role may prevent terminal convergence proof.
                    let _ = cancel_each_owned(&mut state, &mut journal, &mut mutation).await;
                    break Err(PmProductionSupervisorError::RoleFailed);
                }
            },
            command = commands.recv(), if shutdown_response.is_none() => match command {
                Some(Command::Place(command, response)) => {
                    let result = apply_place(&config, &mut state, &mut journal, &mut mutation, command).await;
                    if result.is_err() && !state.ready {
                        let _ = cancel_each_owned(&mut state, &mut journal, &mut mutation).await;
                    }
                    let _ = response.send(result);
                }
                Some(Command::Cancel(order_id, response)) => {
                    let result = apply_cancel(&mut state, &mut journal, &mut mutation, &order_id).await;
                    if result.is_err() && !state.ready {
                        let _ = cancel_each_owned(&mut state, &mut journal, &mut mutation).await;
                    }
                    let _ = response.send(result);
                }
                Some(Command::Shutdown(response)) => {
                    state.ready = false;
                    shutdown_response = Some(response);
                    if let Err(error) = cancel_each_owned(&mut state, &mut journal, &mut mutation).await {
                        break Err(error);
                    }
                }
                None => break Err(PmProductionSupervisorError::TaskFailed),
            }
        }
    };

    let _ = stop.send(true);
    let task_result = join_tasks(tasks).await;
    let final_result = match (result, task_result) {
        (Ok(report), Ok(())) => {
            journal
                .append_durable(&PmSupervisorJournalRecord::CleanShutdown {
                    terminal_poll_sequence: report.terminal_poll_sequence,
                })
                .await?;
            Ok(report)
        }
        (Err(error), _) => Err(error),
        (Ok(_), Err(error)) => Err(error),
    };
    if let Some(response) = shutdown_response {
        let response_value = match &final_result {
            Ok(report) => Ok(report.clone()),
            Err(_) => Err(PmProductionSupervisorError::TaskFailed),
        };
        let _ = response.send(response_value);
    }
    final_result
}

async fn join_tasks(tasks: TaskArray) -> Result<(), PmProductionSupervisorError> {
    for task in tasks {
        task.join()
            .await
            .map_err(|_| PmProductionSupervisorError::TaskFailed)?;
    }
    Ok(())
}

async fn apply_place<M: PmSupervisorMutationRole>(
    config: &PmProductionSupervisorConfig,
    state: &mut SupervisorState,
    journal: &mut PmSupervisorJournal,
    mutation: &mut M,
    command: PmSupervisorPlaceCommand<M::PlaceRequest>,
) -> Result<PmSupervisorOrderProjection, PmSupervisorCommandError> {
    if !state.ready {
        return Err(PmSupervisorCommandError::NotReady);
    }
    let PmSupervisorPlaceCommand { facts, request } = command;
    if !config.scope.contains_token(&facts.token_id) || !mutation.validate_place(&facts, &request) {
        return Err(PmSupervisorCommandError::ScopeMismatch);
    }
    if state.orders.len() >= config.maximum_orders
        || state.orders.contains_key(&facts.expected_venue_order_id)
        || state
            .orders
            .values()
            .any(|order| order.facts.client_order_id == facts.client_order_id)
    {
        return Err(PmSupervisorCommandError::Capacity);
    }
    if journal
        .append_durable(&PmSupervisorJournalRecord::PlaceIntent {
            facts: facts.clone(),
        })
        .await
        .is_err()
    {
        state.ready = false;
        return Err(PmSupervisorCommandError::Durability);
    }
    let order_id = facts.expected_venue_order_id.clone();
    state.orders.insert(
        order_id.clone(),
        OrderState {
            facts,
            status: PmSupervisorOrderStatus::PendingNew,
            cumulative_filled: U256::ZERO,
            known_filled: U256::ZERO,
            cancel_requested: false,
        },
    );
    let result = match mutation.place(request).await {
        Ok(result) => result,
        Err(_) => {
            state.ready = false;
            state
                .orders
                .get_mut(&order_id)
                .expect("inserted order")
                .status = PmSupervisorOrderStatus::ReconciliationRequired;
            return Err(PmSupervisorCommandError::Mutation);
        }
    };
    if result
        .observed_venue_order_id
        .as_deref()
        .is_some_and(|observed| observed != order_id)
    {
        state.ready = false;
        state
            .orders
            .get_mut(&order_id)
            .expect("inserted order")
            .status = PmSupervisorOrderStatus::ReconciliationRequired;
        return Err(PmSupervisorCommandError::Contradiction);
    }
    if journal
        .append_durable(&PmSupervisorJournalRecord::PlaceResult {
            expected_venue_order_id: order_id.clone(),
            classification: result.classification,
        })
        .await
        .is_err()
    {
        state.ready = false;
        state
            .orders
            .get_mut(&order_id)
            .expect("inserted order")
            .status = PmSupervisorOrderStatus::ReconciliationRequired;
        return Err(PmSupervisorCommandError::Durability);
    }
    let order = state.orders.get_mut(&order_id).expect("inserted order");
    order.status = place_status(result.classification);
    if order.status == PmSupervisorOrderStatus::ReconciliationRequired {
        state.ready = false;
    }
    state
        .projection(&order_id)
        .ok_or(PmSupervisorCommandError::Contradiction)
}

async fn apply_cancel<M: PmSupervisorMutationRole>(
    state: &mut SupervisorState,
    journal: &mut PmSupervisorJournal,
    mutation: &mut M,
    order_id: &str,
) -> Result<PmSupervisorOrderProjection, PmSupervisorCommandError> {
    if !state.ready {
        return Err(PmSupervisorCommandError::NotReady);
    }
    let order = state
        .orders
        .get_mut(order_id)
        .ok_or(PmSupervisorCommandError::ScopeMismatch)?;
    if order.status.terminal() {
        return state
            .projection(order_id)
            .ok_or(PmSupervisorCommandError::Contradiction);
    }
    if journal
        .append_durable(&PmSupervisorJournalRecord::CancelIntent {
            venue_order_id: order_id.to_owned(),
        })
        .await
        .is_err()
    {
        state.ready = false;
        return Err(PmSupervisorCommandError::Durability);
    }
    order.status = PmSupervisorOrderStatus::PendingCancel;
    order.cancel_requested = true;
    state.durable_exact_cancel_intents = state
        .durable_exact_cancel_intents
        .checked_add(1)
        .ok_or(PmSupervisorCommandError::Capacity)?;
    let token_id = order.facts.token_id.clone();
    let result = match mutation.cancel_exact(order_id, &token_id).await {
        Ok(result) => result,
        Err(_) => {
            state.ready = false;
            order.status = PmSupervisorOrderStatus::ReconciliationRequired;
            return Err(PmSupervisorCommandError::Mutation);
        }
    };
    if journal
        .append_durable(&PmSupervisorJournalRecord::CancelResult {
            venue_order_id: order_id.to_owned(),
            classification: result.classification,
        })
        .await
        .is_err()
    {
        state.ready = false;
        order.status = PmSupervisorOrderStatus::ReconciliationRequired;
        return Err(PmSupervisorCommandError::Durability);
    }
    order.cancel_requested = matches!(
        result.classification,
        PmSupervisorMutationClassification::Accepted
            | PmSupervisorMutationClassification::AcknowledgementUnknown
    );
    order.status = cancel_status(order.status, result.classification);
    if order.status == PmSupervisorOrderStatus::ReconciliationRequired {
        state.ready = false;
    }
    state
        .projection(order_id)
        .ok_or(PmSupervisorCommandError::Contradiction)
}

async fn cancel_each_owned<M: PmSupervisorMutationRole>(
    state: &mut SupervisorState,
    journal: &mut PmSupervisorJournal,
    mutation: &mut M,
) -> Result<(), PmProductionSupervisorError> {
    let order_ids = state
        .orders
        .iter()
        .filter_map(|(id, order)| (!order.status.terminal()).then_some(id.clone()))
        .collect::<Vec<_>>();
    for order_id in order_ids {
        journal
            .append_durable(&PmSupervisorJournalRecord::CancelIntent {
                venue_order_id: order_id.clone(),
            })
            .await?;
        let order = state.orders.get_mut(&order_id).expect("known owned order");
        order.status = PmSupervisorOrderStatus::PendingCancel;
        order.cancel_requested = true;
        state.durable_exact_cancel_intents = state
            .durable_exact_cancel_intents
            .checked_add(1)
            .ok_or(PmProductionSupervisorError::RoleFailed)?;
        let token_id = order.facts.token_id.clone();
        let classification = match mutation.cancel_exact(&order_id, &token_id).await {
            Ok(result) => result.classification,
            Err(_) => {
                order.status = PmSupervisorOrderStatus::ReconciliationRequired;
                return Err(PmProductionSupervisorError::RoleFailed);
            }
        };
        journal
            .append_durable(&PmSupervisorJournalRecord::CancelResult {
                venue_order_id: order_id,
                classification,
            })
            .await?;
        order.cancel_requested = matches!(
            classification,
            PmSupervisorMutationClassification::Accepted
                | PmSupervisorMutationClassification::AcknowledgementUnknown
        );
        order.status = cancel_status(order.status, classification);
    }
    Ok(())
}

async fn apply_ws(
    config: &PmProductionSupervisorConfig,
    state: &mut SupervisorState,
    journal: &mut PmSupervisorJournal,
    event: PmSupervisorWsEvent,
) -> Result<(), PmProductionSupervisorError> {
    // A private edge always invalidates the last composite REST cut. The next
    // complete poll is responsible for reopening readiness after reconciling
    // order progress, confirmed fills, and venue positions.
    state.ready = false;
    match event {
        PmSupervisorWsEvent::Connected => {
            state.private_ws_connected = true;
            Ok(())
        }
        PmSupervisorWsEvent::Disconnected => {
            state.private_ws_connected = false;
            Ok(())
        }
        PmSupervisorWsEvent::ReconciliationRequired => Ok(()),
        PmSupervisorWsEvent::Order(observation) => apply_order_observation(state, observation),
        PmSupervisorWsEvent::Fill(fill) => {
            if state.apply_fill(config, fill.clone(), true)? {
                journal
                    .append_durable(&PmSupervisorJournalRecord::FillApplied {
                        fill: (&fill).into(),
                    })
                    .await?;
            }
            Ok(())
        }
    }
}

fn apply_order_observation(
    state: &mut SupervisorState,
    observation: PmSupervisorOpenOrder,
) -> Result<(), PmProductionSupervisorError> {
    let order = state
        .orders
        .get_mut(&observation.venue_order_id)
        .ok_or(PmProductionSupervisorError::RoleFailed)?;
    if order.facts.token_id != observation.token_id
        || observation.cumulative_filled > order.facts.quantity
    {
        return Err(PmProductionSupervisorError::RoleFailed);
    }
    // WS and HTTP are independent venue edges. A complete HTTP walk can have
    // begun before a newer private event was reduced, so an otherwise valid
    // lower cumulative value is stale rather than contradictory.
    if observation.cumulative_filled < order.cumulative_filled
        || (observation.cumulative_filled == order.cumulative_filled
            && matches!(
                order.status,
                PmSupervisorOrderStatus::Filled | PmSupervisorOrderStatus::Cancelled
            )
            && !observation.status.terminal())
    {
        return Ok(());
    }
    if matches!(
        order.status,
        PmSupervisorOrderStatus::Rejected | PmSupervisorOrderStatus::Expired
    ) && observation.status != order.status
    {
        return Err(PmProductionSupervisorError::RoleFailed);
    }
    order.cumulative_filled = observation.cumulative_filled;
    order.status = observation.status;
    Ok(())
}

async fn apply_poll(
    config: &PmProductionSupervisorConfig,
    state: &mut SupervisorState,
    journal: &mut PmSupervisorJournal,
    cut: PmSupervisorPollCut,
) -> Result<(), PmProductionSupervisorError> {
    if cut.sequence == 0
        || state
            .last_poll_sequence
            .is_some_and(|last| cut.sequence <= last)
    {
        return Err(PmProductionSupervisorError::RoleFailed);
    }
    let mut position_ids = BTreeSet::new();
    for position in cut.positions {
        if !config.scope.contains_token(&position.token_id)
            || !position_ids.insert(position.token_id.clone())
        {
            return Err(PmProductionSupervisorError::RoleFailed);
        }
        if !state.positions.contains_key(&position.token_id) {
            journal
                .append_durable(&PmSupervisorJournalRecord::PositionBaseline {
                    token_id: position.token_id.clone(),
                    quantity: position.quantity,
                })
                .await?;
            state.positions.insert(
                position.token_id.clone(),
                PositionState {
                    baseline: position.quantity,
                    bought: U256::ZERO,
                    sold: U256::ZERO,
                    authoritative: None,
                },
            );
        }
        state
            .positions
            .get_mut(&position.token_id)
            .expect("inserted position")
            .authoritative = Some(position.quantity);
    }
    if position_ids.len() != config.scope.token_ids.len() {
        return Err(PmProductionSupervisorError::RoleFailed);
    }
    let open_ids = cut
        .open_orders
        .iter()
        .map(|order| order.venue_order_id.clone())
        .collect::<BTreeSet<_>>();
    if open_ids.len() != cut.open_orders.len() {
        return Err(PmProductionSupervisorError::RoleFailed);
    }
    for observation in cut.open_orders {
        apply_order_observation(state, observation)?;
    }
    // The concrete poll walks trades before orders. Apply the later order
    // snapshot first, then let confirmed fills advance it. A fill that lands
    // between those two HTTP walks therefore cannot make a valid cut look
    // internally regressive.
    for fill in cut.fills {
        if state.apply_fill(config, fill.clone(), false)? {
            journal
                .append_durable(&PmSupervisorJournalRecord::FillApplied {
                    fill: (&fill).into(),
                })
                .await?;
        }
    }
    for (order_id, order) in &mut state.orders {
        if !order.status.terminal()
            && !open_ids.contains(order_id)
            && order.cumulative_filled == order.facts.quantity
        {
            order.status = PmSupervisorOrderStatus::Filled;
        } else if !open_ids.contains(order_id) && order.cancel_requested {
            order.status = PmSupervisorOrderStatus::Cancelled;
        } else if !order.status.terminal() && !open_ids.contains(order_id) {
            // A complete open-order cut proves only that the order is no
            // longer open. Without a matched fill total or a prior accepted
            // exact cancel, it cannot distinguish filled/cancelled/rejected.
            order.status = PmSupervisorOrderStatus::ReconciliationRequired;
        }
    }
    state.last_poll_sequence = Some(cut.sequence);
    state.ready = state.private_ws_connected
        && state
            .position_report()?
            .iter()
            .all(|position| position.converged)
        && state.orders.values().all(|order| {
            order.cumulative_filled == order.known_filled
                && !matches!(
                    order.status,
                    PmSupervisorOrderStatus::PendingNew
                        | PmSupervisorOrderStatus::ReconciliationRequired
                )
        });
    journal
        .append_durable(&PmSupervisorJournalRecord::PollReconciled {
            sequence: cut.sequence,
        })
        .await?;
    Ok(())
}

fn terminal_shutdown_report(
    state: &SupervisorState,
) -> Result<PmSupervisorShutdownReport, PmProductionSupervisorError> {
    if state.orders.values().any(|order| !order.status.terminal()) {
        return Err(PmProductionSupervisorError::ShutdownUnreconciled);
    }
    let positions = state.position_report()?;
    if positions.iter().any(|position| !position.converged) {
        return Err(PmProductionSupervisorError::ShutdownUnreconciled);
    }
    Ok(PmSupervisorShutdownReport {
        durable_exact_cancel_intents: state.durable_exact_cancel_intents,
        terminal_poll_sequence: state
            .last_poll_sequence
            .ok_or(PmProductionSupervisorError::ShutdownUnreconciled)?,
        positions,
    })
}

#[cfg(test)]
mod tests {
    use std::{collections::VecDeque, path::Path, sync::Arc};

    use tokio::sync::{Mutex, Notify};

    use super::*;

    struct Heartbeat;

    #[async_trait]
    impl PmSupervisorHeartbeatRole for Heartbeat {
        async fn heartbeat(&mut self) -> Result<(), PmSupervisorEdgeError> {
            Ok(())
        }
    }

    struct Poll {
        cuts: Arc<Mutex<VecDeque<PmSupervisorPollCut>>>,
    }

    #[async_trait]
    impl PmSupervisorPollRole for Poll {
        async fn complete_poll(&mut self) -> Result<PmSupervisorPollCut, PmSupervisorEdgeError> {
            loop {
                if let Some(cut) = self.cuts.lock().await.pop_front() {
                    return Ok(cut);
                }
                tokio::task::yield_now().await;
            }
        }
    }

    struct Ws {
        notify: Arc<Notify>,
        connected: bool,
    }

    #[async_trait]
    impl PmSupervisorWsRole for Ws {
        async fn next_event(&mut self) -> Result<PmSupervisorWsEvent, PmSupervisorEdgeError> {
            if !self.connected {
                self.connected = true;
                return Ok(PmSupervisorWsEvent::Connected);
            }
            self.notify.notified().await;
            Err(PmSupervisorEdgeError::Unavailable)
        }
    }

    struct Mutation;

    #[async_trait]
    impl PmSupervisorMutationRole for Mutation {
        type PlaceRequest = ();

        fn validate_place(
            &self,
            _facts: &PmSupervisorOrderFacts,
            _request: &Self::PlaceRequest,
        ) -> bool {
            true
        }

        async fn place(
            &mut self,
            (): (),
        ) -> Result<PmSupervisorPlaceResult, PmSupervisorEdgeError> {
            Ok(PmSupervisorPlaceResult {
                classification: PmSupervisorMutationClassification::Accepted,
                observed_venue_order_id: Some("venue-1".to_owned()),
            })
        }

        async fn cancel_exact(
            &mut self,
            _venue_order_id: &str,
            _token_id: &str,
        ) -> Result<PmSupervisorCancelResult, PmSupervisorEdgeError> {
            Ok(PmSupervisorCancelResult {
                classification: PmSupervisorMutationClassification::Accepted,
            })
        }
    }

    fn cut(sequence: u64, open: bool, quantity: u64) -> PmSupervisorPollCut {
        PmSupervisorPollCut {
            sequence,
            open_orders: if open {
                vec![PmSupervisorOpenOrder {
                    venue_order_id: "venue-1".to_owned(),
                    token_id: "up".to_owned(),
                    status: PmSupervisorOrderStatus::Live,
                    cumulative_filled: U256::ZERO,
                }]
                .into_boxed_slice()
            } else {
                Box::new([])
            },
            fills: Box::new([]),
            positions: vec![PmSupervisorPosition {
                token_id: "up".to_owned(),
                quantity: U256::from_u64(quantity),
            }]
            .into_boxed_slice(),
        }
    }

    fn test_path(name: &str) -> PathBuf {
        let directory = Path::new(env!("CARGO_MANIFEST_DIR")).join("../../target/tmp");
        std::fs::create_dir_all(&directory).unwrap();
        directory.join(format!("{name}-{}.jsonl", std::process::id()))
    }

    fn remove_test_journal(path: &Path) {
        let _ = std::fs::remove_file(path);
        let mut lock_name = path.as_os_str().to_owned();
        lock_name.push(".lock");
        let _ = std::fs::remove_file(PathBuf::from(lock_name));
    }

    #[tokio::test]
    async fn startup_gates_place_and_shutdown_requires_terminal_poll() {
        let path = test_path("pm-production-supervisor");
        remove_test_journal(&path);
        let cuts = Arc::new(Mutex::new(VecDeque::from([cut(1, false, 0)])));
        let config = PmProductionSupervisorConfig::new(
            PmSupervisorScope::new("condition", ["up".to_owned()]).unwrap(),
            Duration::from_millis(2),
            Duration::from_millis(2),
            Duration::from_secs(2),
        )
        .unwrap();
        let handle = PmProductionSupervisor::start(
            config,
            path.clone(),
            PmProductionSupervisorRoles {
                heartbeat: Heartbeat,
                poll: Poll {
                    cuts: Arc::clone(&cuts),
                },
                user_ws: Ws {
                    notify: Arc::new(Notify::new()),
                    connected: false,
                },
                mutation: Mutation,
            },
        )
        .await
        .unwrap();
        tokio::time::sleep(Duration::from_millis(20)).await;
        let facts = PmSupervisorOrderFacts::new(
            "client-1",
            "venue-1",
            "up",
            PmOrderSide::Buy,
            U256::from_u64(1_000_000),
        )
        .unwrap();
        let placed = handle
            .place(PmSupervisorPlaceCommand::new(facts, ()))
            .await
            .unwrap();
        assert_eq!(placed.status, PmSupervisorOrderStatus::Live);
        cuts.lock().await.push_back(cut(2, true, 0));
        let cancelled = handle.cancel_exact("venue-1").await.unwrap();
        assert_eq!(cancelled.status, PmSupervisorOrderStatus::PendingCancel);
        let terminal_cuts = Arc::clone(&cuts);
        tokio::spawn(async move {
            tokio::time::sleep(Duration::from_millis(20)).await;
            terminal_cuts.lock().await.push_back(cut(3, false, 0));
        });
        let stopped = handle.shutdown().await.unwrap();
        assert_eq!(stopped.terminal_poll_sequence, 3);
        assert!(stopped.positions.iter().all(|position| position.converged));
        remove_test_journal(&path);
    }

    #[tokio::test]
    async fn polled_fill_uses_order_leg_identity_and_mismatch_keeps_gate_closed() {
        let config = PmProductionSupervisorConfig::new(
            PmSupervisorScope::new("condition", ["up".to_owned()]).unwrap(),
            Duration::from_secs(1),
            Duration::from_secs(1),
            Duration::from_secs(1),
        )
        .unwrap();
        let facts = PmSupervisorOrderFacts::new(
            "client-1",
            "venue-1",
            "up",
            PmOrderSide::Buy,
            U256::from_u64(1_000_000),
        )
        .unwrap();
        let second_facts = PmSupervisorOrderFacts::new(
            "client-2",
            "venue-2",
            "up",
            PmOrderSide::Buy,
            U256::from_u64(1_000_000),
        )
        .unwrap();
        let mut state = SupervisorState {
            orders: BTreeMap::from([
                (
                    "venue-1".to_owned(),
                    OrderState {
                        facts,
                        status: PmSupervisorOrderStatus::Live,
                        cumulative_filled: U256::ZERO,
                        known_filled: U256::ZERO,
                        cancel_requested: false,
                    },
                ),
                (
                    "venue-2".to_owned(),
                    OrderState {
                        facts: second_facts,
                        status: PmSupervisorOrderStatus::Live,
                        cumulative_filled: U256::ZERO,
                        known_filled: U256::ZERO,
                        cancel_requested: false,
                    },
                ),
            ]),
            fills: BTreeSet::new(),
            positions: BTreeMap::from([(
                "up".to_owned(),
                PositionState {
                    baseline: U256::ZERO,
                    bought: U256::ZERO,
                    sold: U256::ZERO,
                    authoritative: Some(U256::ZERO),
                },
            )]),
            durable_exact_cancel_intents: 0,
            ready: true,
            private_ws_connected: true,
            last_poll_sequence: None,
        };
        let fill = PmSupervisorFill {
            fill_id: "fill-1".to_owned(),
            venue_order_id: "venue-1".to_owned(),
            token_id: "up".to_owned(),
            side: PmOrderSide::Buy,
            quantity: U256::from_u64(1_000_000),
        };
        assert!(state.apply_fill(&config, fill.clone(), true).unwrap());
        assert!(!state.apply_fill(&config, fill, true).unwrap());
        let second_leg = PmSupervisorFill {
            fill_id: "fill-1".to_owned(),
            venue_order_id: "venue-2".to_owned(),
            token_id: "up".to_owned(),
            side: PmOrderSide::Buy,
            quantity: U256::from_u64(1_000_000),
        };
        assert!(state.apply_fill(&config, second_leg.clone(), true).unwrap());
        assert!(!state.apply_fill(&config, second_leg, true).unwrap());
        let report = state.position_report().unwrap();
        assert_eq!(report[0].fill_based, U256::from_u64(2_000_000));
        assert!(!report[0].converged);
    }

    #[tokio::test]
    async fn private_fill_closes_readiness_until_a_later_complete_poll() {
        let path = test_path("pm-production-supervisor-private-fill-gate");
        remove_test_journal(&path);
        let config = PmProductionSupervisorConfig::new(
            PmSupervisorScope::new("condition", ["up".to_owned()]).unwrap(),
            Duration::from_secs(1),
            Duration::from_secs(1),
            Duration::from_secs(1),
        )
        .unwrap();
        let facts = PmSupervisorOrderFacts::new(
            "client-1",
            "venue-1",
            "up",
            PmOrderSide::Buy,
            U256::from_u64(1_000_000),
        )
        .unwrap();
        let mut state = SupervisorState {
            orders: BTreeMap::from([(
                "venue-1".to_owned(),
                OrderState {
                    facts,
                    status: PmSupervisorOrderStatus::Live,
                    cumulative_filled: U256::ZERO,
                    known_filled: U256::ZERO,
                    cancel_requested: false,
                },
            )]),
            fills: BTreeSet::new(),
            positions: BTreeMap::from([(
                "up".to_owned(),
                PositionState {
                    baseline: U256::ZERO,
                    bought: U256::ZERO,
                    sold: U256::ZERO,
                    authoritative: Some(U256::ZERO),
                },
            )]),
            durable_exact_cancel_intents: 0,
            ready: true,
            private_ws_connected: true,
            last_poll_sequence: Some(1),
        };
        let (mut journal, _) = PmSupervisorJournal::open(path.clone(), config.scope())
            .await
            .unwrap();
        apply_ws(
            &config,
            &mut state,
            &mut journal,
            PmSupervisorWsEvent::Fill(PmSupervisorFill {
                fill_id: "fill-1".to_owned(),
                venue_order_id: "venue-1".to_owned(),
                token_id: "up".to_owned(),
                side: PmOrderSide::Buy,
                quantity: U256::from_u64(1_000_000),
            }),
        )
        .await
        .unwrap();
        assert!(!state.ready);
        drop(journal);
        remove_test_journal(&path);
    }

    #[tokio::test]
    async fn later_private_fill_dominates_an_older_open_order_snapshot() {
        let path = test_path("pm-production-supervisor-interleaved-poll-fill");
        remove_test_journal(&path);
        let config = PmProductionSupervisorConfig::new(
            PmSupervisorScope::new("condition", ["up".to_owned()]).unwrap(),
            Duration::from_secs(1),
            Duration::from_secs(1),
            Duration::from_secs(1),
        )
        .unwrap();
        let facts = PmSupervisorOrderFacts::new(
            "client-1",
            "venue-1",
            "up",
            PmOrderSide::Buy,
            U256::from_u64(1_000_000),
        )
        .unwrap();
        let mut state = SupervisorState {
            orders: BTreeMap::from([(
                "venue-1".to_owned(),
                OrderState {
                    facts,
                    status: PmSupervisorOrderStatus::Live,
                    cumulative_filled: U256::ZERO,
                    known_filled: U256::ZERO,
                    cancel_requested: false,
                },
            )]),
            fills: BTreeSet::new(),
            positions: BTreeMap::from([(
                "up".to_owned(),
                PositionState {
                    baseline: U256::ZERO,
                    bought: U256::ZERO,
                    sold: U256::ZERO,
                    authoritative: Some(U256::ZERO),
                },
            )]),
            durable_exact_cancel_intents: 0,
            ready: false,
            private_ws_connected: true,
            last_poll_sequence: None,
        };
        let (mut journal, _) = PmSupervisorJournal::open(path.clone(), config.scope())
            .await
            .unwrap();
        let fill = PmSupervisorFill {
            fill_id: "fill-1".to_owned(),
            venue_order_id: "venue-1".to_owned(),
            token_id: "up".to_owned(),
            side: PmOrderSide::Buy,
            quantity: U256::from_u64(1_000_000),
        };
        apply_ws(
            &config,
            &mut state,
            &mut journal,
            PmSupervisorWsEvent::Fill(fill.clone()),
        )
        .await
        .unwrap();
        apply_poll(
            &config,
            &mut state,
            &mut journal,
            PmSupervisorPollCut {
                sequence: 1,
                open_orders: vec![PmSupervisorOpenOrder {
                    venue_order_id: "venue-1".to_owned(),
                    token_id: "up".to_owned(),
                    status: PmSupervisorOrderStatus::Live,
                    cumulative_filled: U256::ZERO,
                }]
                .into_boxed_slice(),
                fills: vec![fill].into_boxed_slice(),
                positions: vec![PmSupervisorPosition {
                    token_id: "up".to_owned(),
                    quantity: U256::from_u64(1_000_000),
                }]
                .into_boxed_slice(),
            },
        )
        .await
        .unwrap();
        assert!(state.ready);
        assert_eq!(
            state.orders["venue-1"].status,
            PmSupervisorOrderStatus::Filled
        );
        assert_eq!(
            state.orders["venue-1"].known_filled,
            U256::from_u64(1_000_000)
        );
        drop(journal);
        remove_test_journal(&path);
    }

    #[tokio::test]
    async fn order_cumulative_fill_closes_readiness_until_fill_ledger_catches_up() {
        let path = test_path("pm-production-supervisor-fill-ledger");
        remove_test_journal(&path);
        let config = PmProductionSupervisorConfig::new(
            PmSupervisorScope::new("condition", ["up".to_owned()]).unwrap(),
            Duration::from_secs(1),
            Duration::from_secs(1),
            Duration::from_secs(1),
        )
        .unwrap();
        let facts = PmSupervisorOrderFacts::new(
            "client-1",
            "venue-1",
            "up",
            PmOrderSide::Buy,
            U256::from_u64(2_000_000),
        )
        .unwrap();
        let mut state = SupervisorState {
            orders: BTreeMap::from([(
                "venue-1".to_owned(),
                OrderState {
                    facts,
                    status: PmSupervisorOrderStatus::Live,
                    cumulative_filled: U256::ZERO,
                    known_filled: U256::ZERO,
                    cancel_requested: false,
                },
            )]),
            fills: BTreeSet::new(),
            positions: BTreeMap::new(),
            durable_exact_cancel_intents: 0,
            ready: false,
            private_ws_connected: true,
            last_poll_sequence: None,
        };
        let (mut journal, _) = PmSupervisorJournal::open(path.clone(), config.scope())
            .await
            .unwrap();
        apply_poll(
            &config,
            &mut state,
            &mut journal,
            PmSupervisorPollCut {
                sequence: 1,
                open_orders: vec![PmSupervisorOpenOrder {
                    venue_order_id: "venue-1".to_owned(),
                    token_id: "up".to_owned(),
                    status: PmSupervisorOrderStatus::PartiallyFilled,
                    cumulative_filled: U256::from_u64(1_000_000),
                }]
                .into_boxed_slice(),
                fills: Box::new([]),
                positions: vec![PmSupervisorPosition {
                    token_id: "up".to_owned(),
                    quantity: U256::ZERO,
                }]
                .into_boxed_slice(),
            },
        )
        .await
        .unwrap();
        assert!(!state.ready);

        apply_poll(
            &config,
            &mut state,
            &mut journal,
            PmSupervisorPollCut {
                sequence: 2,
                open_orders: vec![PmSupervisorOpenOrder {
                    venue_order_id: "venue-1".to_owned(),
                    token_id: "up".to_owned(),
                    status: PmSupervisorOrderStatus::PartiallyFilled,
                    cumulative_filled: U256::from_u64(1_000_000),
                }]
                .into_boxed_slice(),
                fills: vec![PmSupervisorFill {
                    fill_id: "fill-1".to_owned(),
                    venue_order_id: "venue-1".to_owned(),
                    token_id: "up".to_owned(),
                    side: PmOrderSide::Buy,
                    quantity: U256::from_u64(1_000_000),
                }]
                .into_boxed_slice(),
                positions: vec![PmSupervisorPosition {
                    token_id: "up".to_owned(),
                    quantity: U256::from_u64(1_000_000),
                }]
                .into_boxed_slice(),
            },
        )
        .await
        .unwrap();
        assert!(state.ready);
        assert_eq!(
            state.orders["venue-1"].known_filled,
            U256::from_u64(1_000_000)
        );
        drop(journal);
        remove_test_journal(&path);
    }

    #[tokio::test]
    async fn restart_replays_durable_order_and_requires_poll_before_exact_cancel() {
        let path = test_path("pm-production-supervisor-recovery");
        remove_test_journal(&path);
        let scope = PmSupervisorScope::new("condition", ["up".to_owned()]).unwrap();
        let facts = PmSupervisorOrderFacts::new(
            "client-1",
            "venue-1",
            "up",
            PmOrderSide::Buy,
            U256::from_u64(1_000_000),
        )
        .unwrap();
        let (mut journal, _) = PmSupervisorJournal::open(path.clone(), &scope)
            .await
            .unwrap();
        journal
            .append_durable(&PmSupervisorJournalRecord::PositionBaseline {
                token_id: "up".to_owned(),
                quantity: U256::ZERO,
            })
            .await
            .unwrap();
        journal
            .append_durable(&PmSupervisorJournalRecord::PlaceIntent {
                facts: facts.clone(),
            })
            .await
            .unwrap();
        journal
            .append_durable(&PmSupervisorJournalRecord::PlaceResult {
                expected_venue_order_id: "venue-1".to_owned(),
                classification: PmSupervisorMutationClassification::Accepted,
            })
            .await
            .unwrap();
        drop(journal);

        let cuts = Arc::new(Mutex::new(VecDeque::new()));
        let config = PmProductionSupervisorConfig::new(
            scope,
            Duration::from_millis(2),
            Duration::from_millis(2),
            Duration::from_secs(2),
        )
        .unwrap();
        let handle = PmProductionSupervisor::start(
            config,
            path.clone(),
            PmProductionSupervisorRoles {
                heartbeat: Heartbeat,
                poll: Poll {
                    cuts: Arc::clone(&cuts),
                },
                user_ws: Ws {
                    notify: Arc::new(Notify::new()),
                    connected: false,
                },
                mutation: Mutation,
            },
        )
        .await
        .unwrap();
        assert_eq!(
            handle.cancel_exact("venue-1").await.unwrap_err(),
            PmSupervisorCommandError::NotReady
        );
        cuts.lock().await.push_back(cut(1, true, 0));
        tokio::time::sleep(Duration::from_millis(20)).await;
        assert_eq!(
            handle.cancel_exact("venue-1").await.unwrap().status,
            PmSupervisorOrderStatus::PendingCancel
        );
        let terminal_cuts = Arc::clone(&cuts);
        tokio::spawn(async move {
            tokio::time::sleep(Duration::from_millis(20)).await;
            terminal_cuts.lock().await.push_back(cut(2, false, 0));
        });
        assert_eq!(handle.shutdown().await.unwrap().terminal_poll_sequence, 2);
        remove_test_journal(&path);
    }
}
