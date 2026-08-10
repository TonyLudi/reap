use std::sync::{
    Arc, OnceLock,
    atomic::{AtomicU64, Ordering as AtomicOrdering},
};
use std::time::{Duration, Instant as StdInstant, SystemTime, UNIX_EPOCH};
use std::{fmt, future::Future, pin::Pin};

use async_trait::async_trait;
use futures_util::{SinkExt, StreamExt};
use reap_pm_core::{ConnectionEpoch, ReceivedEventClock};
use reap_polymarket_wire::{PmMarketSubscription, PmWireScope};
use thiserror::Error;
use tokio::sync::{mpsc, oneshot, watch};
use tokio::time::{Instant, sleep_until, timeout};
use tokio_tungstenite::tungstenite::{Error as WebSocketError, Message, protocol::WebSocketConfig};

use crate::{
    PmLiveAdapterError, PmPublicWsConfig, PmSelectedWsSocketFacts,
    selected_ws::PmProductionSelectedWsRouteBinding,
    task_guard::AbortOnDropTask,
    ws_transport::{
        PmDefaultWsDialer, PmFixedWsRoute, PmProductionSelectedWsDialer, PmWsDialFailure,
        PmWsDialRequest, PmWsDialStrategy, PmWsSocket,
    },
};

const APPLICATION_PING: &str = "PING";
const APPLICATION_PONG: &str = "PONG";

#[derive(Debug, Clone, Copy, PartialEq, Eq, Error)]
pub enum PmPublicWsClockError {
    #[error("public WebSocket clock reading is invalid")]
    InvalidReading,
    #[error("public WebSocket system clock is unavailable")]
    SystemClockUnavailable,
}

/// Receive-edge clock with no venue timestamp and no queue-service timestamp.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct PmPublicWsEdgeClock {
    received: ReceivedEventClock,
}

impl PmPublicWsEdgeClock {
    pub fn new(
        local_wall_receive_ns: u64,
        monotonic_receive_ns: u64,
    ) -> Result<Self, PmPublicWsClockError> {
        let received = ReceivedEventClock::new(None, local_wall_receive_ns, monotonic_receive_ns)
            .map_err(|_| PmPublicWsClockError::InvalidReading)?;
        Ok(Self { received })
    }

    #[must_use]
    pub const fn local_wall_receive_ns(self) -> u64 {
        self.received.local_wall_receive_ns()
    }

    #[must_use]
    pub const fn monotonic_receive_ns(self) -> u64 {
        self.received.monotonic_receive_ns()
    }
}

/// Purpose-specific source sampled by the transport at socket/lifecycle edges.
pub trait PmPublicWsClockSource: Send + 'static {
    fn observe_public_ws_edge(&mut self) -> Result<PmPublicWsEdgeClock, PmPublicWsClockError>;
}

/// Cloneable, read-only view of the latest activity generation issued by one
/// concrete public-WebSocket source.
///
/// The transport advances the shared high-water before attempting each
/// socket/lifecycle handoff. Callers can therefore compare this value with
/// the generation on the last event they fully admitted. There is no public
/// constructor or mutation method, and the view grants no socket authority.
#[derive(Clone)]
pub struct PmPublicWsActivityView {
    generation: Arc<AtomicU64>,
}

impl PmPublicWsActivityView {
    #[must_use]
    pub fn generation(&self) -> u64 {
        self.generation.load(AtomicOrdering::Acquire)
    }
}

impl fmt::Debug for PmPublicWsActivityView {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("PmPublicWsActivityView")
            .field("generation", &self.generation())
            .finish()
    }
}

struct PmPublicWsActivitySource {
    generation: Arc<AtomicU64>,
}

impl PmPublicWsActivitySource {
    fn new() -> (Self, PmPublicWsActivityView) {
        let generation = Arc::new(AtomicU64::new(0));
        (
            Self {
                generation: Arc::clone(&generation),
            },
            PmPublicWsActivityView { generation },
        )
    }

    fn issue(&self) -> Result<u64, PmPublicWsTransportError> {
        let success = AtomicOrdering::AcqRel;
        let failure = AtomicOrdering::Acquire;
        self.generation
            .fetch_update(success, failure, |current| current.checked_add(1))
            .map(|previous| {
                previous
                    .checked_add(1)
                    .expect("successful checked activity update cannot overflow")
            })
            .map_err(|_| PmPublicWsTransportError::ActivityGenerationOverflow)
    }
}

struct SystemPublicWsClock;

impl PmPublicWsClockSource for SystemPublicWsClock {
    fn observe_public_ws_edge(&mut self) -> Result<PmPublicWsEdgeClock, PmPublicWsClockError> {
        static MONOTONIC_ORIGIN: OnceLock<StdInstant> = OnceLock::new();
        let local_wall_receive_ns = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .map_err(|_| PmPublicWsClockError::SystemClockUnavailable)?
            .as_nanos()
            .try_into()
            .map_err(|_| PmPublicWsClockError::SystemClockUnavailable)?;
        let monotonic_receive_ns = MONOTONIC_ORIGIN
            .get_or_init(StdInstant::now)
            .elapsed()
            .as_nanos()
            .saturating_add(1)
            .try_into()
            .map_err(|_| PmPublicWsClockError::SystemClockUnavailable)?;
        PmPublicWsEdgeClock::new(local_wall_receive_ns, monotonic_receive_ns)
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct PmPublicWsConnection {
    scope: PmWireScope,
    connection_epoch: ConnectionEpoch,
    selected_socket_facts: Option<PmSelectedWsSocketFacts>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct PmPublicWsObservation {
    connection: PmPublicWsConnection,
    clock: PmPublicWsEdgeClock,
    activity_generation: u64,
}

impl PmPublicWsObservation {
    #[must_use]
    pub const fn connection(self) -> PmPublicWsConnection {
        self.connection
    }

    #[must_use]
    pub const fn clock(self) -> PmPublicWsEdgeClock {
        self.clock
    }

    #[must_use]
    pub const fn activity_generation(self) -> u64 {
        self.activity_generation
    }
}

impl PmPublicWsConnection {
    #[must_use]
    pub const fn scope(self) -> PmWireScope {
        self.scope
    }

    #[must_use]
    pub const fn connection_epoch(self) -> ConnectionEpoch {
        self.connection_epoch
    }

    /// Socket facts are present only after a production-selected or explicit
    /// loopback-evidence dial completed its full post-handshake validation.
    #[must_use]
    pub const fn selected_socket_facts(self) -> Option<PmSelectedWsSocketFacts> {
        self.selected_socket_facts
    }
}

#[derive(Debug, PartialEq, Eq)]
pub struct PmPublicWsRawData {
    observation: PmPublicWsObservation,
    bytes: Box<[u8]>,
}

impl PmPublicWsRawData {
    #[must_use]
    pub const fn connection(&self) -> PmPublicWsConnection {
        self.observation.connection
    }

    #[must_use]
    pub const fn clock(&self) -> PmPublicWsEdgeClock {
        self.observation.clock
    }

    #[must_use]
    pub const fn activity_generation(&self) -> u64 {
        self.observation.activity_generation
    }

    #[must_use]
    pub fn bytes(&self) -> &[u8] {
        &self.bytes
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Error)]
pub enum PmPublicWsDisconnectReason {
    #[error("connection attempt timed out")]
    ConnectTimeout,
    #[error("connection attempt failed")]
    ConnectFailed,
    #[error("subscription write timed out")]
    SubscriptionWriteTimeout,
    #[error("subscription write failed")]
    SubscriptionWriteFailed,
    #[error("socket read failed")]
    SocketReadFailed,
    #[error("socket closed")]
    SocketClosed,
    #[error("socket write timed out")]
    SocketWriteTimeout,
    #[error("socket write failed")]
    SocketWriteFailed,
    #[error("binary public-data frame is forbidden")]
    BinaryFrame,
    #[error("public-data frame exceeded its configured bound")]
    FrameTooLarge,
    #[error("public-data connection became idle")]
    IdleTimeout,
    #[error("application-level PONG was not received in time")]
    PongTimeout,
    #[error("unexpected raw WebSocket protocol frame")]
    UnexpectedProtocolFrame,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct PmPublicWsRetirement {
    observation: PmPublicWsObservation,
    reason: PmPublicWsDisconnectReason,
}

impl PmPublicWsRetirement {
    #[must_use]
    pub const fn connection(self) -> PmPublicWsConnection {
        self.observation.connection
    }

    #[must_use]
    pub const fn clock(self) -> PmPublicWsEdgeClock {
        self.observation.clock
    }

    #[must_use]
    pub const fn activity_generation(self) -> u64 {
        self.observation.activity_generation
    }

    #[must_use]
    pub const fn reason(self) -> PmPublicWsDisconnectReason {
        self.reason
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
/// Evidence of the exact reconnect transition already authorized by the
/// composition-owned public session after durable retirement handling.
pub struct PmPublicWsReconnect {
    retired: PmPublicWsRetirement,
    replacement_epoch: ConnectionEpoch,
    reconnect_attempt: u8,
    backoff: Duration,
    scheduled_clock: PmPublicWsEdgeClock,
    activity_generation: u64,
}

/// Reconnect authority returned by the composition-owned public session.
///
/// The transport never derives this decision: composition first durably
/// records the retirement and the session's reconnect transition, then
/// authorizes either one exact replacement or a terminal stop.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum PmPublicWsReconnectDirective {
    Reconnect {
        retired_epoch: ConnectionEpoch,
        replacement_epoch: ConnectionEpoch,
        reconnect_attempt: u8,
        backoff: Duration,
    },
    Stop,
}

impl PmPublicWsReconnectDirective {
    #[must_use]
    pub const fn reconnect(
        retired_epoch: ConnectionEpoch,
        replacement_epoch: ConnectionEpoch,
        reconnect_attempt: u8,
        backoff: Duration,
    ) -> Self {
        Self::Reconnect {
            retired_epoch,
            replacement_epoch,
            reconnect_attempt,
            backoff,
        }
    }

    #[must_use]
    pub const fn stop() -> Self {
        Self::Stop
    }
}

impl PmPublicWsReconnect {
    #[must_use]
    pub const fn retired(self) -> PmPublicWsRetirement {
        self.retired
    }

    #[must_use]
    pub const fn replacement_epoch(self) -> ConnectionEpoch {
        self.replacement_epoch
    }

    #[must_use]
    pub const fn reconnect_attempt(self) -> u8 {
        self.reconnect_attempt
    }

    #[must_use]
    pub const fn backoff(self) -> Duration {
        self.backoff
    }

    #[must_use]
    pub const fn scheduled_clock(self) -> PmPublicWsEdgeClock {
        self.scheduled_clock
    }

    #[must_use]
    pub const fn activity_generation(self) -> u64 {
        self.activity_generation
    }
}

/// Purpose-specific public market transport evidence.
///
/// Raw text is intentionally not parsed here: capture must retain malformed
/// bounded venue input so the downstream public session can classify it. A
/// binary message never crosses this boundary. The current official text
/// heartbeat is represented separately as `Pong`.
#[derive(Debug, PartialEq, Eq)]
pub enum PmPublicWsEvent {
    ConnectionOpened(PmPublicWsObservation),
    SubscriptionSent(PmPublicWsObservation),
    /// Successful application text `PING` write. Composition processes this
    /// ordered edge with `PmPublicSession::poll_heartbeat` before any later
    /// queued `Pong`.
    PingSent(PmPublicWsObservation),
    RawData(PmPublicWsRawData),
    Pong(PmPublicWsObservation),
    ConnectionRetired(PmPublicWsRetirement),
    ReconnectScheduled(PmPublicWsReconnect),
    ReconnectStopped(PmPublicWsRetirement),
    Shutdown(PmPublicWsObservation),
}

impl PmPublicWsEvent {
    /// Source-issued high-water stamped before this event's bounded handoff.
    #[must_use]
    pub const fn activity_generation(&self) -> u64 {
        match self {
            Self::ConnectionOpened(observation)
            | Self::SubscriptionSent(observation)
            | Self::PingSent(observation)
            | Self::Pong(observation)
            | Self::Shutdown(observation) => observation.activity_generation(),
            Self::RawData(raw) => raw.activity_generation(),
            Self::ConnectionRetired(retired) | Self::ReconnectStopped(retired) => {
                retired.activity_generation()
            }
            Self::ReconnectScheduled(reconnect) => reconnect.activity_generation(),
        }
    }
}

#[async_trait]
pub trait PmPublicWsEventSink: Send {
    type Error;

    async fn deliver_public_ws_event(&mut self, event: PmPublicWsEvent) -> Result<(), Self::Error>;

    /// Authorize the next transport epoch only after retirement evidence and
    /// the session-owned reconnect policy have been durably committed.
    async fn authorize_public_ws_reconnect(
        &mut self,
        _retired: PmPublicWsRetirement,
    ) -> Result<PmPublicWsReconnectDirective, Self::Error> {
        Ok(PmPublicWsReconnectDirective::Stop)
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Error)]
pub enum PmPublicWsTransportError {
    #[error("public market WebSocket reconnect directive was stale, malformed, or out of bounds")]
    InvalidReconnectDirective,
    #[error("public market WebSocket event channel closed")]
    EventChannelClosed,
    #[error("public market WebSocket event channel saturated")]
    EventChannelSaturated,
    #[error("public market WebSocket worker task failed")]
    WorkerFailed,
    #[error("public market WebSocket activity generation overflowed")]
    ActivityGenerationOverflow,
    #[error(transparent)]
    Clock(#[from] PmPublicWsClockError),
}

#[derive(Debug, Error)]
pub enum PmPublicWsRunError<E> {
    #[error(transparent)]
    Transport(#[from] PmPublicWsTransportError),
    #[error("public market WebSocket event sink rejected evidence: {0}")]
    Sink(E),
}

#[derive(Debug)]
pub struct PmPublicWsShutdownHandle {
    sender: watch::Sender<bool>,
}

impl PmPublicWsShutdownHandle {
    pub fn request_shutdown(&self) {
        self.sender.send_replace(true);
    }
}

#[derive(Debug)]
pub struct PmPublicWsShutdownSignal {
    receiver: watch::Receiver<bool>,
}

#[must_use]
pub fn pm_public_ws_shutdown_channel() -> (PmPublicWsShutdownHandle, PmPublicWsShutdownSignal) {
    let (sender, receiver) = watch::channel(false);
    (
        PmPublicWsShutdownHandle { sender },
        PmPublicWsShutdownSignal { receiver },
    )
}

/// Sole network sender for the current public market subscription and PING.
pub struct PmPublicMarketWsRole {
    config: PmPublicWsConfig,
    subscription: String,
    clock: Box<dyn PmPublicWsClockSource>,
    activity_source: PmPublicWsActivitySource,
    activity_view: PmPublicWsActivityView,
}

impl fmt::Debug for PmPublicMarketWsRole {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("PmPublicMarketWsRole")
            .field("scope", &self.config.scope())
            .finish_non_exhaustive()
    }
}

impl PmPublicMarketWsRole {
    /// Builds with a process-local fallback clock.
    ///
    /// Production composition must prefer [`Self::with_clock_source`] and
    /// inject the shared monotonic origin used by metadata, PM book, and OKX
    /// ingress; otherwise cross-source age/order comparisons are meaningless.
    pub fn new(config: PmPublicWsConfig) -> Result<Self, PmLiveAdapterError> {
        Self::with_clock_source(config, SystemPublicWsClock)
    }

    pub fn with_clock_source<C>(
        config: PmPublicWsConfig,
        clock: C,
    ) -> Result<Self, PmLiveAdapterError>
    where
        C: PmPublicWsClockSource,
    {
        let subscription = PmMarketSubscription::new(config.scope().token()).to_json()?;
        let (activity_source, activity_view) = PmPublicWsActivitySource::new();
        Ok(Self {
            config,
            subscription,
            clock: Box::new(clock),
            activity_source,
            activity_view,
        })
    }

    #[must_use]
    pub const fn scope(&self) -> PmWireScope {
        self.config.scope()
    }

    /// Returns only the nonsecret transport bounds needed to prove that the
    /// canonical session can honestly drive this role.
    #[must_use]
    pub const fn transport_policy(&self) -> crate::PmPublicWsTransportPolicy {
        self.config.transport_policy()
    }

    /// Read-only source high-water for detecting socket/lifecycle activity
    /// that has been stamped but not yet admitted by an event sink.
    #[must_use]
    pub fn activity_view(&self) -> PmPublicWsActivityView {
        self.activity_view.clone()
    }

    pub(crate) const fn is_production(&self) -> bool {
        self.config.is_production()
    }

    pub async fn run<S>(
        self,
        shutdown: PmPublicWsShutdownSignal,
        sink: &mut S,
    ) -> Result<(), PmPublicWsRunError<S::Error>>
    where
        S: PmPublicWsEventSink,
    {
        let (event_sender, mut event_receiver) =
            mpsc::channel(self.config.event_channel_capacity());
        let config = self.config;
        let subscription = self.subscription;
        let clock = self.clock;
        let activity_source = self.activity_source;
        let worker = AbortOnDropTask::new(tokio::spawn(async move {
            run_worker(
                config,
                subscription,
                clock,
                activity_source,
                shutdown.receiver,
                event_sender,
                PmDefaultWsDialer,
            )
            .await
        }));

        serve_worker_events(worker, &mut event_receiver, sink).await
    }
}

/// Public market WebSocket role fixed to one production peer and selected
/// Linux interface/source pair by [`crate::PmProductionSelectedWsOwner`].
///
/// This move-only value is thread-confined configuration and transport
/// custody. It is not actor-generation, namespace, DNS, NAT, or authorization
/// evidence.
pub struct PmProductionSelectedPublicWsRole {
    role: PmPublicMarketWsRole,
    binding: PmProductionSelectedWsRouteBinding,
}

impl PmProductionSelectedPublicWsRole {
    pub(crate) const fn from_role_and_binding(
        role: PmPublicMarketWsRole,
        binding: PmProductionSelectedWsRouteBinding,
    ) -> Self {
        Self { role, binding }
    }

    #[must_use]
    pub const fn scope(&self) -> PmWireScope {
        self.role.scope()
    }

    #[must_use]
    pub const fn transport_policy(&self) -> crate::PmPublicWsTransportPolicy {
        self.role.transport_policy()
    }

    #[must_use]
    pub fn activity_view(&self) -> PmPublicWsActivityView {
        self.role.activity_view()
    }

    /// A selected public read transport never authorizes order entry.
    #[must_use]
    pub const fn production_order_entry_authorized(&self) -> bool {
        false
    }

    pub async fn run<S>(
        self,
        shutdown: PmPublicWsShutdownSignal,
        sink: &mut S,
    ) -> Result<(), PmPublicWsRunError<S::Error>>
    where
        S: PmPublicWsEventSink,
    {
        let Self { role, binding } = self;
        let PmPublicMarketWsRole {
            config,
            subscription,
            clock,
            activity_source,
            activity_view: _,
        } = role;
        let (event_sender, mut event_receiver) = mpsc::channel(config.event_channel_capacity());
        let worker = run_worker(
            config,
            subscription,
            clock,
            activity_source,
            shutdown.receiver,
            event_sender,
            PmProductionSelectedWsDialer::new(binding),
        );
        serve_inline_worker_events(worker, &mut event_receiver, sink).await
    }
}

impl fmt::Debug for PmProductionSelectedPublicWsRole {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("PmProductionSelectedPublicWsRole")
            .field("scope", &self.scope())
            .finish_non_exhaustive()
    }
}

async fn serve_inline_worker_events<F, S>(
    worker: F,
    event_receiver: &mut mpsc::Receiver<WorkerEvent>,
    sink: &mut S,
) -> Result<(), PmPublicWsRunError<S::Error>>
where
    F: Future<Output = Result<(), PmPublicWsTransportError>>,
    S: PmPublicWsEventSink,
{
    tokio::pin!(worker);
    loop {
        tokio::select! {
            event = event_receiver.recv() => {
                let Some(event) = event else {
                    return worker.await.map_err(PmPublicWsRunError::Transport);
                };
                if let InlineWorkerCompletion::Completed(result) = deliver_inline_worker_event_while_polling(
                    worker.as_mut(),
                    event,
                    sink,
                )
                .await?
                {
                    while let Some(event) = event_receiver.recv().await {
                        deliver_inline_worker_event(event, sink).await?;
                    }
                    return result.map_err(PmPublicWsRunError::Transport);
                }
            }
            result = &mut worker => {
                while let Some(event) = event_receiver.recv().await {
                    deliver_inline_worker_event(event, sink).await?;
                }
                return result.map_err(PmPublicWsRunError::Transport);
            }
        }
    }
}

enum InlineWorkerCompletion {
    Running,
    Completed(Result<(), PmPublicWsTransportError>),
}

async fn deliver_inline_worker_event_while_polling<F, S>(
    mut worker: Pin<&mut F>,
    event: WorkerEvent,
    sink: &mut S,
) -> Result<InlineWorkerCompletion, PmPublicWsRunError<S::Error>>
where
    F: Future<Output = Result<(), PmPublicWsTransportError>>,
    S: PmPublicWsEventSink,
{
    // Keep the source-owned worker live while admission is pending. If it
    // finishes first, retain that result without cancelling the already
    // admitted delivery; the caller drains all queued evidence afterward.
    let delivery = deliver_inline_worker_event(event, sink);
    tokio::pin!(delivery);
    tokio::select! {
        result = delivery.as_mut() => {
            result?;
            Ok(InlineWorkerCompletion::Running)
        }
        worker_result = worker.as_mut() => {
            delivery.await?;
            Ok(InlineWorkerCompletion::Completed(worker_result))
        }
    }
}

async fn deliver_inline_worker_event<S>(
    event: WorkerEvent,
    sink: &mut S,
) -> Result<(), PmPublicWsRunError<S::Error>>
where
    S: PmPublicWsEventSink,
{
    match event {
        WorkerEvent::Evidence(event) => sink
            .deliver_public_ws_event(event)
            .await
            .map_err(PmPublicWsRunError::Sink),
        WorkerEvent::ReconnectAuthority { retired, response } => {
            let directive = sink
                .authorize_public_ws_reconnect(retired)
                .await
                .map_err(PmPublicWsRunError::Sink)?;
            response
                .send(directive)
                .map_err(|_| PmPublicWsRunError::Transport(PmPublicWsTransportError::WorkerFailed))
        }
    }
}

async fn serve_worker_events<S>(
    mut worker: AbortOnDropTask<Result<(), PmPublicWsTransportError>>,
    event_receiver: &mut mpsc::Receiver<WorkerEvent>,
    sink: &mut S,
) -> Result<(), PmPublicWsRunError<S::Error>>
where
    S: PmPublicWsEventSink,
{
    while let Some(event) = event_receiver.recv().await {
        match event {
            WorkerEvent::Evidence(event) => {
                if let Err(error) = sink.deliver_public_ws_event(event).await {
                    let _ = worker.abort_and_join().await;
                    return Err(PmPublicWsRunError::Sink(error));
                }
            }
            WorkerEvent::ReconnectAuthority { retired, response } => {
                let directive = sink.authorize_public_ws_reconnect(retired).await;
                match directive {
                    Ok(directive) => {
                        if response.send(directive).is_err() {
                            let _ = worker.abort_and_join().await;
                            return Err(PmPublicWsRunError::Transport(
                                PmPublicWsTransportError::WorkerFailed,
                            ));
                        }
                    }
                    Err(error) => {
                        let _ = worker.abort_and_join().await;
                        return Err(PmPublicWsRunError::Sink(error));
                    }
                }
            }
        }
    }

    worker
        .join()
        .await
        .map_err(|_| PmPublicWsTransportError::WorkerFailed)??;
    Ok(())
}

async fn run_worker<D>(
    config: PmPublicWsConfig,
    subscription: String,
    mut clock: Box<dyn PmPublicWsClockSource>,
    activity: PmPublicWsActivitySource,
    mut shutdown: watch::Receiver<bool>,
    events: mpsc::Sender<WorkerEvent>,
    mut dialer: D,
) -> Result<(), PmPublicWsTransportError>
where
    D: PmWsDialStrategy,
{
    let mut connection_epoch = config.initial_connection_epoch();

    loop {
        let connection = PmPublicWsConnection {
            scope: config.scope(),
            connection_epoch,
            selected_socket_facts: None,
        };
        let outcome = run_attempt(
            &config,
            &subscription,
            connection,
            clock.as_mut(),
            &activity,
            AttemptControl {
                shutdown: &mut shutdown,
                events: &events,
            },
            &mut dialer,
        )
        .await?;
        let (retired, terminal) = match outcome {
            AttemptOutcome::Shutdown(observation) => {
                emit(&events, PmPublicWsEvent::Shutdown(observation)).await?;
                return Ok(());
            }
            AttemptOutcome::Retired(retired) => (
                retired,
                dialer.uses_selected_reconnect_classification()
                    && selected_public_retirement_is_terminal(retired.reason()),
            ),
            AttemptOutcome::Terminal(retired) => (retired, true),
        };
        emit(&events, PmPublicWsEvent::ConnectionRetired(retired)).await?;

        if terminal {
            let stopped = restamp_retirement(&activity, retired)?;
            emit(&events, PmPublicWsEvent::ReconnectStopped(stopped)).await?;
            return Err(PmPublicWsTransportError::WorkerFailed);
        }

        let directive = request_reconnect_authority(&activity, &events, retired).await?;
        let PmPublicWsReconnectDirective::Reconnect {
            retired_epoch,
            replacement_epoch,
            reconnect_attempt,
            backoff,
        } = directive
        else {
            let stopped = restamp_retirement(&activity, retired)?;
            emit(&events, PmPublicWsEvent::ReconnectStopped(stopped)).await?;
            return Ok(());
        };
        if retired_epoch != connection_epoch
            || replacement_epoch.value() <= connection_epoch.value()
            || reconnect_attempt == 0
            || reconnect_attempt > config.max_reconnect_attempts()
            || backoff.is_zero()
            || backoff > config.reconnect_backoff()
        {
            return Err(PmPublicWsTransportError::InvalidReconnectDirective);
        }
        let scheduled = source_observation(&activity, connection, clock.as_mut())?;
        emit(
            &events,
            PmPublicWsEvent::ReconnectScheduled(PmPublicWsReconnect {
                retired,
                replacement_epoch,
                reconnect_attempt,
                backoff,
                scheduled_clock: scheduled.clock(),
                activity_generation: scheduled.activity_generation(),
            }),
        )
        .await?;

        let backoff_deadline = Instant::now() + backoff;
        tokio::select! {
            () = wait_for_shutdown(&mut shutdown) => {
                emit(
                    &events,
                    PmPublicWsEvent::Shutdown(source_observation(
                        &activity,
                        retired.connection(),
                        clock.as_mut(),
                    )?),
                )
                .await?;
                return Ok(());
            }
            () = sleep_until(backoff_deadline) => {}
        }
        connection_epoch = replacement_epoch;
    }
}

enum AttemptOutcome {
    Shutdown(PmPublicWsObservation),
    Retired(PmPublicWsRetirement),
    Terminal(PmPublicWsRetirement),
}

struct AttemptControl<'a> {
    shutdown: &'a mut watch::Receiver<bool>,
    events: &'a mpsc::Sender<WorkerEvent>,
}

async fn run_attempt<D>(
    config: &PmPublicWsConfig,
    subscription: &str,
    connection: PmPublicWsConnection,
    clock: &mut dyn PmPublicWsClockSource,
    activity: &PmPublicWsActivitySource,
    control: AttemptControl<'_>,
    dialer: &mut D,
) -> Result<AttemptOutcome, PmPublicWsTransportError>
where
    D: PmWsDialStrategy,
{
    let AttemptControl { shutdown, events } = control;
    let websocket_config = WebSocketConfig::default()
        .read_buffer_size(config.max_frame_bytes().clamp(1_024, 64 * 1_024))
        .write_buffer_size(1_024)
        .max_write_buffer_size(8 * 1_024)
        .max_message_size(Some(config.max_frame_bytes()))
        .max_frame_size(Some(config.max_frame_bytes()));
    let connect = dialer.dial(PmWsDialRequest::new(
        PmFixedWsRoute::PublicMarket,
        config.endpoint().as_str(),
        websocket_config,
    ));
    let dial_outcome = tokio::select! {
        () = wait_for_shutdown(shutdown) => {
            return Ok(AttemptOutcome::Shutdown(source_observation(
                activity,
                connection,
                clock,
            )?));
        }
        result = timeout(config.connect_timeout(), connect) => match result {
            Err(_) => return retired(activity, clock, connection, PmPublicWsDisconnectReason::ConnectTimeout),
            Ok(Err(PmWsDialFailure::RetryableConnect)) => {
                return retired(activity, clock, connection, PmPublicWsDisconnectReason::ConnectFailed);
            }
            Ok(Err(PmWsDialFailure::TerminalInvariant)) => {
                return terminal_retired(
                    activity,
                    clock,
                    connection,
                    PmPublicWsDisconnectReason::ConnectFailed,
                );
            }
            Ok(Ok(outcome)) => outcome,
        },
    };
    let (socket, selected_socket_facts) = dial_outcome.into_parts();
    let connection = PmPublicWsConnection {
        selected_socket_facts,
        ..connection
    };

    emit(
        events,
        PmPublicWsEvent::ConnectionOpened(source_observation(activity, connection, clock)?),
    )
    .await?;
    run_connected(
        config,
        subscription,
        connection,
        clock,
        activity,
        socket,
        shutdown,
        events,
    )
    .await
}

#[allow(clippy::too_many_arguments)]
async fn run_connected(
    config: &PmPublicWsConfig,
    subscription: &str,
    connection: PmPublicWsConnection,
    clock: &mut dyn PmPublicWsClockSource,
    activity: &PmPublicWsActivitySource,
    mut socket: PmWsSocket,
    shutdown: &mut watch::Receiver<bool>,
    events: &mpsc::Sender<WorkerEvent>,
) -> Result<AttemptOutcome, PmPublicWsTransportError> {
    match timeout(
        config.connect_timeout(),
        socket.send(Message::text(subscription.to_owned())),
    )
    .await
    {
        Err(_) => {
            return retired(
                activity,
                clock,
                connection,
                PmPublicWsDisconnectReason::SubscriptionWriteTimeout,
            );
        }
        Ok(Err(_)) => {
            return retired(
                activity,
                clock,
                connection,
                PmPublicWsDisconnectReason::SubscriptionWriteFailed,
            );
        }
        Ok(Ok(())) => {}
    }
    emit(
        events,
        PmPublicWsEvent::SubscriptionSent(source_observation(activity, connection, clock)?),
    )
    .await?;

    let now = Instant::now();
    let mut last_inbound = now;
    let mut next_heartbeat = now + config.heartbeat_interval();
    let mut outstanding_pong: Option<Instant> = None;

    loop {
        let idle_deadline = last_inbound + config.idle_timeout();
        let pong_deadline = outstanding_pong.unwrap_or(idle_deadline);
        tokio::select! {
            () = wait_for_shutdown(shutdown) => {
                let _ = timeout(config.pong_timeout(), socket.close(None)).await;
                return Ok(AttemptOutcome::Shutdown(source_observation(
                    activity,
                    connection,
                    clock,
                )?));
            }
            () = sleep_until(next_heartbeat), if outstanding_pong.is_none() => {
                match timeout(
                    config.pong_timeout(),
                    socket.send(Message::text(APPLICATION_PING)),
                ).await {
                    Err(_) => return retired(activity, clock, connection, PmPublicWsDisconnectReason::SocketWriteTimeout),
                    Ok(Err(_)) => return retired(activity, clock, connection, PmPublicWsDisconnectReason::SocketWriteFailed),
                    Ok(Ok(())) => {}
                }
                let sent_at = Instant::now();
                outstanding_pong = Some(sent_at + config.pong_timeout());
                next_heartbeat = sent_at + config.heartbeat_interval();
                emit(
                    events,
                    PmPublicWsEvent::PingSent(source_observation(activity, connection, clock)?),
                )
                .await?;
            }
            () = sleep_until(pong_deadline), if outstanding_pong.is_some() => {
                return retired(activity, clock, connection, PmPublicWsDisconnectReason::PongTimeout);
            }
            () = sleep_until(idle_deadline) => {
                return retired(activity, clock, connection, PmPublicWsDisconnectReason::IdleTimeout);
            }
            message = socket.next() => {
                let Some(message) = message else {
                    return retired(activity, clock, connection, PmPublicWsDisconnectReason::SocketClosed);
                };
                let message = match message {
                    Ok(message) => message,
                    Err(error) => return retired(activity, clock, connection, classify_read_error(&error)),
                };
                let received_at = Instant::now();
                last_inbound = received_at;
                match message {
                    Message::Text(text) if text.as_str() == APPLICATION_PONG => {
                        let observation = source_observation(activity, connection, clock)?;
                        outstanding_pong = None;
                        // The canonical public session rebases its next ping
                        // from the application-PONG receive edge. Keep the
                        // transport timer on that same edge so it cannot emit
                        // an earlier PingSent from the prior send schedule.
                        next_heartbeat = received_at + config.heartbeat_interval();
                        emit(
                            events,
                            PmPublicWsEvent::Pong(observation),
                        )
                        .await?;
                    }
                    Message::Text(text) => {
                        // Stamp the source edge before validation or owned-byte
                        // allocation. A later rejection/failed handoff leaves
                        // the read-only high-water ahead and invalidates old
                        // runtime evidence.
                        let observation = source_observation(activity, connection, clock)?;
                        if text.len() > config.max_frame_bytes() {
                            return retired(activity, clock, connection, PmPublicWsDisconnectReason::FrameTooLarge);
                        }
                        emit(
                            events,
                            PmPublicWsEvent::RawData(PmPublicWsRawData {
                                observation,
                                bytes: text.as_str().as_bytes().to_vec().into_boxed_slice(),
                            }),
                        )
                        .await?;
                    }
                    Message::Binary(_) => {
                        return retired(activity, clock, connection, PmPublicWsDisconnectReason::BinaryFrame);
                    }
                    Message::Ping(_) | Message::Pong(_) => {
                        match timeout(config.pong_timeout(), socket.flush()).await {
                            Err(_) => return retired(activity, clock, connection, PmPublicWsDisconnectReason::SocketWriteTimeout),
                            Ok(Err(_)) => return retired(activity, clock, connection, PmPublicWsDisconnectReason::SocketWriteFailed),
                            Ok(Ok(())) => {}
                        }
                    }
                    Message::Close(_) => {
                        return retired(activity, clock, connection, PmPublicWsDisconnectReason::SocketClosed);
                    }
                    Message::Frame(_) => {
                        return retired(activity, clock, connection, PmPublicWsDisconnectReason::UnexpectedProtocolFrame);
                    }
                }
            }
        }
    }
}

fn classify_read_error(error: &WebSocketError) -> PmPublicWsDisconnectReason {
    if matches!(error, WebSocketError::Capacity(_)) {
        PmPublicWsDisconnectReason::FrameTooLarge
    } else {
        PmPublicWsDisconnectReason::SocketReadFailed
    }
}

#[cfg(test)]
impl PmPublicMarketWsRole {
    async fn run_with_test_selected_loopback<S>(
        self,
        shutdown: PmPublicWsShutdownSignal,
        sink: &mut S,
        dialer: crate::ws_transport::PmTestSelectedLoopbackWsDialer,
    ) -> Result<(), PmPublicWsRunError<S::Error>>
    where
        S: PmPublicWsEventSink,
    {
        let (event_sender, mut event_receiver) =
            mpsc::channel(self.config.event_channel_capacity());
        let worker = run_worker(
            self.config,
            self.subscription,
            self.clock,
            self.activity_source,
            shutdown.receiver,
            event_sender,
            dialer,
        );
        serve_inline_worker_events(worker, &mut event_receiver, sink).await
    }
}

fn observe(
    clock: &mut dyn PmPublicWsClockSource,
) -> Result<PmPublicWsEdgeClock, PmPublicWsTransportError> {
    clock
        .observe_public_ws_edge()
        .map_err(PmPublicWsTransportError::Clock)
}

fn source_observation(
    activity: &PmPublicWsActivitySource,
    connection: PmPublicWsConnection,
    clock: &mut dyn PmPublicWsClockSource,
) -> Result<PmPublicWsObservation, PmPublicWsTransportError> {
    // Allocate first. If clocking or the eventual bounded handoff fails, the
    // externally retained view stays ahead of the last admitted observation.
    let activity_generation = activity.issue()?;
    let clock = observe(clock)?;
    Ok(PmPublicWsObservation {
        connection,
        clock,
        activity_generation,
    })
}

fn restamp_retirement(
    activity: &PmPublicWsActivitySource,
    retired: PmPublicWsRetirement,
) -> Result<PmPublicWsRetirement, PmPublicWsTransportError> {
    Ok(PmPublicWsRetirement {
        observation: PmPublicWsObservation {
            connection: retired.connection(),
            clock: retired.clock(),
            activity_generation: activity.issue()?,
        },
        reason: retired.reason(),
    })
}

fn retired(
    activity: &PmPublicWsActivitySource,
    clock: &mut dyn PmPublicWsClockSource,
    connection: PmPublicWsConnection,
    reason: PmPublicWsDisconnectReason,
) -> Result<AttemptOutcome, PmPublicWsTransportError> {
    Ok(AttemptOutcome::Retired(PmPublicWsRetirement {
        observation: source_observation(activity, connection, clock)?,
        reason,
    }))
}

fn terminal_retired(
    activity: &PmPublicWsActivitySource,
    clock: &mut dyn PmPublicWsClockSource,
    connection: PmPublicWsConnection,
    reason: PmPublicWsDisconnectReason,
) -> Result<AttemptOutcome, PmPublicWsTransportError> {
    Ok(AttemptOutcome::Terminal(PmPublicWsRetirement {
        observation: source_observation(activity, connection, clock)?,
        reason,
    }))
}

const fn selected_public_retirement_is_terminal(reason: PmPublicWsDisconnectReason) -> bool {
    matches!(
        reason,
        PmPublicWsDisconnectReason::BinaryFrame
            | PmPublicWsDisconnectReason::FrameTooLarge
            | PmPublicWsDisconnectReason::UnexpectedProtocolFrame
    )
}

async fn emit(
    events: &mpsc::Sender<WorkerEvent>,
    event: PmPublicWsEvent,
) -> Result<(), PmPublicWsTransportError> {
    debug_assert_ne!(event.activity_generation(), 0);
    events
        .try_send(WorkerEvent::Evidence(event))
        .map_err(classify_event_send_error)
}

enum WorkerEvent {
    Evidence(PmPublicWsEvent),
    ReconnectAuthority {
        retired: PmPublicWsRetirement,
        response: oneshot::Sender<PmPublicWsReconnectDirective>,
    },
}

async fn request_reconnect_authority(
    activity: &PmPublicWsActivitySource,
    events: &mpsc::Sender<WorkerEvent>,
    retired: PmPublicWsRetirement,
) -> Result<PmPublicWsReconnectDirective, PmPublicWsTransportError> {
    let (response, decision) = oneshot::channel();
    let retired = restamp_retirement(activity, retired)?;
    events
        .try_send(WorkerEvent::ReconnectAuthority { retired, response })
        .map_err(classify_event_send_error)?;
    decision
        .await
        .map_err(|_| PmPublicWsTransportError::EventChannelClosed)
}

fn classify_event_send_error(
    error: mpsc::error::TrySendError<WorkerEvent>,
) -> PmPublicWsTransportError {
    match error {
        mpsc::error::TrySendError::Full(_) => PmPublicWsTransportError::EventChannelSaturated,
        mpsc::error::TrySendError::Closed(_) => PmPublicWsTransportError::EventChannelClosed,
    }
}

async fn wait_for_shutdown(shutdown: &mut watch::Receiver<bool>) {
    loop {
        if *shutdown.borrow() {
            return;
        }
        if shutdown.changed().await.is_err() {
            return;
        }
    }
}

#[cfg(test)]
mod tests {
    use std::collections::VecDeque;
    use std::sync::{
        Arc, Condvar, Mutex,
        atomic::{AtomicU64, Ordering},
    };

    use reap_pm_core::{
        EvmAddress, MAX_REQUIRED_SPENDERS, OkxInstrumentId, OkxReferenceInstrument, PmAssetId,
        PmChainId, PmConditionId, PmConnectionId, PmInstrumentHandle, PmInstrumentId,
        PmMarketHandle, PmMarketId, PmMarketLifecycle, PmMarketMetadata, PmOutcomeLabel,
        PmOutcomeMetadata, PmProductSource, PmPublicObservationGrant, PmQuantity, PmSourceHandle,
        PmSpenderDomain, PmSpenderRequirement, PmTick, PmTokenHandle, PmTokenId, SnapshotRevision,
        U256,
    };
    use reap_polymarket_adapter::{
        PM_PUBLIC_PONG_BYTES, PmAuthoritativeMetadata, PmMetadataRevisionInput,
        PmPublicHeartbeatAction, PmPublicHeartbeatConfig, PmPublicRole, PmPublicSession,
    };
    use reap_polymarket_wire::PmBookParserConfig;
    use reap_transport::ReconnectPolicy;
    use tokio::net::TcpListener;
    #[cfg(target_os = "linux")]
    use tokio::task::LocalSet;
    use tokio::{sync::Notify, task::JoinHandle};
    use tokio_tungstenite::tungstenite::handshake::server::{Request, Response};
    use tokio_tungstenite::{WebSocketStream, accept_async, accept_hdr_async};

    use super::*;
    #[cfg(target_os = "linux")]
    use crate::ws_transport::PmTestSelectedLoopbackWsDialer;

    const CURRENT_SUBSCRIPTION: &str =
        r#"{"assets_ids":["123"],"custom_feature_enabled":true,"type":"market"}"#;

    #[derive(Debug, PartialEq, Eq)]
    enum Seen {
        Open(u64),
        Subscription(u64),
        Ping(u64),
        Raw(u64, Vec<u8>),
        Pong(u64),
        Retired(u64, PmPublicWsDisconnectReason),
        Reconnect(u64, u64, u8),
        Stopped(u64, PmPublicWsDisconnectReason),
        Shutdown(u64),
    }

    #[derive(Debug, PartialEq, Eq)]
    struct SeenClock {
        edge: &'static str,
        local_wall_receive_ns: u64,
        monotonic_receive_ns: u64,
        activity_generation: u64,
    }

    struct TestClock {
        next: u64,
    }

    impl PmPublicWsClockSource for TestClock {
        fn observe_public_ws_edge(&mut self) -> Result<PmPublicWsEdgeClock, PmPublicWsClockError> {
            let next = self.next;
            self.next = self.next.checked_add(1).expect("test clock overflow");
            PmPublicWsEdgeClock::new(1_000_000 + next, next)
        }
    }

    struct TokioInstantClock {
        origin: Instant,
    }

    impl PmPublicWsClockSource for TokioInstantClock {
        fn observe_public_ws_edge(&mut self) -> Result<PmPublicWsEdgeClock, PmPublicWsClockError> {
            let monotonic_receive_ns: u64 = Instant::now()
                .duration_since(self.origin)
                .as_nanos()
                .saturating_add(100)
                .try_into()
                .map_err(|_| PmPublicWsClockError::InvalidReading)?;
            PmPublicWsEdgeClock::new(
                1_700_000_000_000_000_000_u64.saturating_add(monotonic_receive_ns),
                monotonic_receive_ns,
            )
        }
    }

    struct QueueClock {
        next: Arc<AtomicU64>,
        raw_two_sampled: Arc<(Mutex<bool>, Condvar)>,
    }

    impl PmPublicWsClockSource for QueueClock {
        fn observe_public_ws_edge(&mut self) -> Result<PmPublicWsEdgeClock, PmPublicWsClockError> {
            let next = self.next.fetch_add(1, Ordering::SeqCst);
            if next == 4 {
                let (sampled, changed) = &*self.raw_two_sampled;
                *sampled.lock().unwrap() = true;
                changed.notify_all();
            }
            PmPublicWsEdgeClock::new(2_000_000 + next, next)
        }
    }

    #[derive(Default)]
    struct RawSinkGate {
        entered_first_raw: bool,
        release_first_raw: bool,
    }

    struct BlockingRawSink {
        gate: Arc<(Mutex<RawSinkGate>, Condvar)>,
        blocked_once: bool,
    }

    struct BlockingOpenedSink {
        entered: Arc<Notify>,
        release: Arc<Notify>,
    }

    #[cfg(target_os = "linux")]
    struct BlockingSelectedInlineSink {
        entered: Arc<Notify>,
        release: Arc<Notify>,
        delivered: Arc<Mutex<Vec<&'static str>>>,
    }

    struct BlockingRawDeliverySink {
        entered: Arc<Notify>,
        release: Arc<Notify>,
    }

    struct BlockingReconnectAuthoritySink {
        entered: Arc<Notify>,
        release: Arc<Notify>,
    }

    #[async_trait]
    impl PmPublicWsEventSink for BlockingOpenedSink {
        type Error = &'static str;

        async fn deliver_public_ws_event(
            &mut self,
            event: PmPublicWsEvent,
        ) -> Result<(), Self::Error> {
            if matches!(event, PmPublicWsEvent::ConnectionOpened(_)) {
                self.entered.notify_one();
                self.release.notified().await;
            }
            Ok(())
        }
    }

    #[cfg(target_os = "linux")]
    #[async_trait]
    impl PmPublicWsEventSink for BlockingSelectedInlineSink {
        type Error = &'static str;

        async fn deliver_public_ws_event(
            &mut self,
            event: PmPublicWsEvent,
        ) -> Result<(), Self::Error> {
            let edge = match &event {
                PmPublicWsEvent::ConnectionOpened(_) => "opened",
                PmPublicWsEvent::SubscriptionSent(_) => "subscription",
                PmPublicWsEvent::RawData(_) => "raw",
                PmPublicWsEvent::PingSent(_) => "ping",
                PmPublicWsEvent::Pong(_) => "pong",
                PmPublicWsEvent::ConnectionRetired(_) => "retired",
                PmPublicWsEvent::ReconnectScheduled(_) => "reconnect",
                PmPublicWsEvent::ReconnectStopped(_) => "stopped",
                PmPublicWsEvent::Shutdown(_) => "shutdown",
            };
            if matches!(event, PmPublicWsEvent::ConnectionOpened(_)) {
                self.entered.notify_one();
                self.release.notified().await;
            }
            self.delivered.lock().unwrap().push(edge);
            Ok(())
        }
    }

    #[async_trait]
    impl PmPublicWsEventSink for BlockingRawDeliverySink {
        type Error = &'static str;

        async fn deliver_public_ws_event(
            &mut self,
            event: PmPublicWsEvent,
        ) -> Result<(), Self::Error> {
            if matches!(event, PmPublicWsEvent::RawData(_)) {
                self.entered.notify_one();
                self.release.notified().await;
            }
            Ok(())
        }
    }

    #[async_trait]
    impl PmPublicWsEventSink for BlockingReconnectAuthoritySink {
        type Error = &'static str;

        async fn deliver_public_ws_event(
            &mut self,
            _event: PmPublicWsEvent,
        ) -> Result<(), Self::Error> {
            Ok(())
        }

        async fn authorize_public_ws_reconnect(
            &mut self,
            _retired: PmPublicWsRetirement,
        ) -> Result<PmPublicWsReconnectDirective, Self::Error> {
            self.entered.notify_one();
            self.release.notified().await;
            Ok(PmPublicWsReconnectDirective::stop())
        }
    }

    #[async_trait]
    impl PmPublicWsEventSink for BlockingRawSink {
        type Error = &'static str;

        async fn deliver_public_ws_event(
            &mut self,
            event: PmPublicWsEvent,
        ) -> Result<(), Self::Error> {
            if matches!(event, PmPublicWsEvent::RawData(_)) && !self.blocked_once {
                self.blocked_once = true;
                let (state, changed) = &*self.gate;
                let mut state = state.lock().unwrap();
                state.entered_first_raw = true;
                changed.notify_all();
                while !state.release_first_raw {
                    let (next, wait) = changed
                        .wait_timeout(state, Duration::from_secs(15))
                        .unwrap();
                    state = next;
                    if wait.timed_out() && !state.release_first_raw {
                        return Err("first raw sink gate timed out");
                    }
                }
            }
            Ok(())
        }
    }

    type SelectedSocketFactsLog = Arc<Mutex<Vec<(u64, Option<PmSelectedWsSocketFacts>)>>>;

    struct TestSink {
        sender: mpsc::UnboundedSender<Seen>,
        clocks: Arc<Mutex<Vec<SeenClock>>>,
        selected_facts: Option<SelectedSocketFactsLog>,
        fail_at: Option<usize>,
        delivered: usize,
        reconnect_authority_calls: u64,
        authorized_reconnects: u8,
        max_authorized_reconnects: u8,
        authorized_backoff: Duration,
        planned_directives: Option<VecDeque<PmPublicWsReconnectDirective>>,
    }

    struct CanonicalHeartbeatSink {
        inner: TestSink,
        session: PmPublicSession,
    }

    type TestRoleTask = JoinHandle<Result<(), PmPublicWsRunError<&'static str>>>;
    type SpawnedTestRole = (
        PmPublicWsShutdownHandle,
        mpsc::UnboundedReceiver<Seen>,
        Arc<Mutex<Vec<SeenClock>>>,
        TestRoleTask,
    );

    #[async_trait]
    impl PmPublicWsEventSink for TestSink {
        type Error = &'static str;

        async fn deliver_public_ws_event(
            &mut self,
            event: PmPublicWsEvent,
        ) -> Result<(), Self::Error> {
            if self.fail_at == Some(self.delivered) {
                return Err("synthetic sink rejection");
            }
            self.delivered += 1;
            let connection = match &event {
                PmPublicWsEvent::ConnectionOpened(observation)
                | PmPublicWsEvent::SubscriptionSent(observation)
                | PmPublicWsEvent::PingSent(observation)
                | PmPublicWsEvent::Pong(observation)
                | PmPublicWsEvent::Shutdown(observation) => observation.connection(),
                PmPublicWsEvent::RawData(data) => data.connection(),
                PmPublicWsEvent::ConnectionRetired(retired)
                | PmPublicWsEvent::ReconnectStopped(retired) => retired.connection(),
                PmPublicWsEvent::ReconnectScheduled(reconnect) => reconnect.retired().connection(),
            };
            if let Some(selected_facts) = &self.selected_facts {
                selected_facts.lock().unwrap().push((
                    connection.connection_epoch().value(),
                    connection.selected_socket_facts(),
                ));
            }
            let activity_generation = event.activity_generation();
            let (edge, clock) = match &event {
                PmPublicWsEvent::ConnectionOpened(observation) => ("opened", observation.clock()),
                PmPublicWsEvent::SubscriptionSent(observation) => {
                    ("subscription", observation.clock())
                }
                PmPublicWsEvent::PingSent(observation) => ("ping", observation.clock()),
                PmPublicWsEvent::RawData(data) => ("raw", data.clock()),
                PmPublicWsEvent::Pong(observation) => ("pong", observation.clock()),
                PmPublicWsEvent::ConnectionRetired(retired) => ("retired", retired.clock()),
                PmPublicWsEvent::ReconnectScheduled(reconnect) => {
                    ("reconnect", reconnect.scheduled_clock())
                }
                PmPublicWsEvent::ReconnectStopped(retired) => ("stopped", retired.clock()),
                PmPublicWsEvent::Shutdown(observation) => ("shutdown", observation.clock()),
            };
            self.clocks.lock().unwrap().push(SeenClock {
                edge,
                local_wall_receive_ns: clock.local_wall_receive_ns(),
                monotonic_receive_ns: clock.monotonic_receive_ns(),
                activity_generation,
            });
            let seen = match event {
                PmPublicWsEvent::ConnectionOpened(observation) => {
                    Seen::Open(observation.connection().connection_epoch().value())
                }
                PmPublicWsEvent::SubscriptionSent(observation) => {
                    Seen::Subscription(observation.connection().connection_epoch().value())
                }
                PmPublicWsEvent::PingSent(observation) => {
                    Seen::Ping(observation.connection().connection_epoch().value())
                }
                PmPublicWsEvent::RawData(data) => Seen::Raw(
                    data.connection().connection_epoch().value(),
                    data.bytes().to_vec(),
                ),
                PmPublicWsEvent::Pong(observation) => {
                    Seen::Pong(observation.connection().connection_epoch().value())
                }
                PmPublicWsEvent::ConnectionRetired(retired) => Seen::Retired(
                    retired.connection().connection_epoch().value(),
                    retired.reason(),
                ),
                PmPublicWsEvent::ReconnectScheduled(reconnect) => Seen::Reconnect(
                    reconnect.retired().connection().connection_epoch().value(),
                    reconnect.replacement_epoch().value(),
                    reconnect.reconnect_attempt(),
                ),
                PmPublicWsEvent::ReconnectStopped(retired) => Seen::Stopped(
                    retired.connection().connection_epoch().value(),
                    retired.reason(),
                ),
                PmPublicWsEvent::Shutdown(observation) => {
                    Seen::Shutdown(observation.connection().connection_epoch().value())
                }
            };
            self.sender
                .send(seen)
                .map_err(|_| "test observation receiver closed")
        }

        async fn authorize_public_ws_reconnect(
            &mut self,
            retired: PmPublicWsRetirement,
        ) -> Result<PmPublicWsReconnectDirective, Self::Error> {
            self.reconnect_authority_calls += 1;
            if let Some(directives) = &mut self.planned_directives {
                return Ok(directives
                    .pop_front()
                    .unwrap_or_else(PmPublicWsReconnectDirective::stop));
            }
            if self.authorized_reconnects >= self.max_authorized_reconnects {
                return Ok(PmPublicWsReconnectDirective::stop());
            }
            self.authorized_reconnects += 1;
            let retired_epoch = retired.connection().connection_epoch();
            Ok(PmPublicWsReconnectDirective::reconnect(
                retired_epoch,
                ConnectionEpoch::new(retired_epoch.value() + 1),
                self.authorized_reconnects,
                self.authorized_backoff,
            ))
        }
    }

    #[async_trait]
    impl PmPublicWsEventSink for CanonicalHeartbeatSink {
        type Error = &'static str;

        async fn deliver_public_ws_event(
            &mut self,
            event: PmPublicWsEvent,
        ) -> Result<(), Self::Error> {
            match &event {
                PmPublicWsEvent::SubscriptionSent(observation) => self
                    .session
                    .mark_subscription_sent(observation.clock().monotonic_receive_ns())
                    .expect("transport subscription edge is canonical-session admissible"),
                PmPublicWsEvent::PingSent(observation) => assert_eq!(
                    self.session
                        .poll_heartbeat(observation.clock().monotonic_receive_ns())
                        .expect("transport ping clock is canonical-session admissible"),
                    PmPublicHeartbeatAction::SendPing,
                ),
                PmPublicWsEvent::Pong(observation) => {
                    let batch = self
                        .session
                        .classify(
                            PM_PUBLIC_PONG_BYTES,
                            observation.clock().local_wall_receive_ns(),
                            observation.clock().monotonic_receive_ns(),
                        )
                        .expect("transport pong clock is canonical-session admissible");
                    assert!(batch.heartbeat().is_some());
                }
                _ => {}
            }
            self.inner.deliver_public_ws_event(event).await
        }

        async fn authorize_public_ws_reconnect(
            &mut self,
            retired: PmPublicWsRetirement,
        ) -> Result<PmPublicWsReconnectDirective, Self::Error> {
            self.inner.authorize_public_ws_reconnect(retired).await
        }
    }

    fn scope() -> PmWireScope {
        PmWireScope::new(
            PmConditionId::from_bytes([0x11; 32]).unwrap(),
            PmMarketId::from_bytes([0x22; 32]).unwrap(),
            PmTokenId::new(U256::from_u64(123)).unwrap(),
        )
    }

    fn canonical_heartbeat_session(
        heartbeat_interval: Duration,
        pong_timeout: Duration,
        connection_epoch: ConnectionEpoch,
    ) -> PmPublicSession {
        let scope = scope();
        let instrument = PmInstrumentHandle::new(
            PmMarketHandle::from_ordinal(0),
            PmTokenHandle::from_ordinal(0),
        );
        let parser = PmBookParserConfig::new(
            scope,
            PmTick::parse_decimal("0.01").unwrap(),
            PmQuantity::parse_decimal("5").unwrap(),
            false,
        );
        let source =
            PmProductSource::polymarket_market(PmSourceHandle::from_ordinal(4), instrument.token());
        let grant = PmPublicObservationGrant::derive_goal_f(
            OkxReferenceInstrument::index(OkxInstrumentId::new("BTC-USDT").unwrap()),
            PmInstrumentId::new(scope.market(), scope.token()),
        );
        let role = PmPublicRole::new(
            grant,
            instrument,
            parser,
            source,
            PmConnectionId::new("live-adapter-heartbeat-test").unwrap(),
        )
        .unwrap();

        let chain = PmChainId::new(137).unwrap();
        let exchange = EvmAddress::parse("0xE111180000d2663C0091e4f400237545B87B996B").unwrap();
        let collateral = EvmAddress::parse("0xC011a7E12a19f7B1f670d46F03B03f3342E82DFB").unwrap();
        let conditional_tokens =
            EvmAddress::parse("0x4D97DCd97eC945f40cF65F87097ACe5EA0476045").unwrap();
        let mut spenders = [None; MAX_REQUIRED_SPENDERS];
        spenders[0] = Some(PmSpenderRequirement::new(
            chain,
            exchange,
            PmSpenderDomain::Standard,
            PmAssetId::collateral(collateral),
        ));
        spenders[1] = Some(PmSpenderRequirement::new(
            chain,
            exchange,
            PmSpenderDomain::Standard,
            PmAssetId::outcome(conditional_tokens, scope.token()),
        ));
        let expected = PmMarketMetadata::new(
            scope.condition(),
            scope.market(),
            PmOutcomeMetadata::new(scope.token(), PmOutcomeLabel::new("Yes").unwrap()),
            PmMarketLifecycle::new(true, false, false, true, true),
            parser.tick(),
            parser.minimum_order_size(),
            false,
            chain,
            exchange,
            spenders,
            2,
        )
        .unwrap();
        let lifecycle = format!(
            r#"{{"condition_id":"{}","market_id":"{}","active":true,"closed":false,"archived":false,"accepting_orders":true,"enable_order_book":true}}"#,
            scope.condition(),
            scope.market(),
        );
        let clob = format!(
            r#"{{"condition_id":"{}","market_id":"{}","minimum_tick_size":"0.01","minimum_order_size":"5","neg_risk":false,"tokens":[{{"token_id":"123","outcome":"Yes"}},{{"token_id":"456","outcome":"No"}}]}}"#,
            scope.condition(),
            scope.market(),
        );
        let authoritative = PmAuthoritativeMetadata::join_raw(
            instrument,
            source,
            expected,
            lifecycle.as_bytes(),
            clob.as_bytes(),
            PmMetadataRevisionInput::new(SnapshotRevision::new(7), 50).unwrap(),
        )
        .unwrap();
        let heartbeat = PmPublicHeartbeatConfig::new(
            heartbeat_interval.as_nanos().try_into().unwrap(),
            pong_timeout.as_nanos().try_into().unwrap(),
        )
        .unwrap();
        PmPublicSession::new(
            role,
            authoritative,
            connection_epoch,
            None,
            ReconnectPolicy {
                initial_delay: Duration::from_millis(1),
                max_delay: Duration::from_millis(4),
                multiplier: 2,
            },
            heartbeat,
        )
        .unwrap()
    }

    #[allow(clippy::too_many_arguments)]
    fn local_config(
        address: std::net::SocketAddr,
        connect_timeout: Duration,
        idle_timeout: Duration,
        heartbeat_interval: Duration,
        pong_timeout: Duration,
        max_frame_bytes: usize,
        max_reconnect_attempts: u8,
        reconnect_backoff: Duration,
        initial_epoch: u64,
    ) -> PmPublicWsConfig {
        PmPublicWsConfig::local_evidence(
            &format!("ws://{address}/ws/market"),
            scope(),
            connect_timeout,
            idle_timeout,
            heartbeat_interval,
            pong_timeout,
            max_frame_bytes,
            max_reconnect_attempts,
            reconnect_backoff,
            8,
            ConnectionEpoch::new(initial_epoch),
        )
        .expect("valid loopback configuration")
    }

    fn standard_config(
        address: std::net::SocketAddr,
        max_frame_bytes: usize,
        max_reconnect_attempts: u8,
        initial_epoch: u64,
    ) -> PmPublicWsConfig {
        local_config(
            address,
            Duration::from_millis(200),
            Duration::from_millis(500),
            Duration::from_millis(50),
            Duration::from_millis(20),
            max_frame_bytes,
            max_reconnect_attempts,
            Duration::from_millis(5),
            initial_epoch,
        )
    }

    fn saturation_config(address: std::net::SocketAddr) -> PmPublicWsConfig {
        PmPublicWsConfig::local_evidence(
            &format!("ws://{address}/ws/market"),
            scope(),
            Duration::from_millis(200),
            Duration::from_millis(500),
            Duration::from_millis(50),
            Duration::from_millis(20),
            1_024,
            0,
            Duration::from_millis(5),
            1,
            ConnectionEpoch::new(70),
        )
        .expect("valid saturation configuration")
    }

    fn spawn_role(config: PmPublicWsConfig, fail_at: Option<usize>) -> SpawnedTestRole {
        let max_authorized_reconnects = config.max_reconnect_attempts();
        let authorized_backoff = config.reconnect_backoff();
        let role = PmPublicMarketWsRole::with_clock_source(config, TestClock { next: 1 })
            .expect("public WS role");
        let (shutdown_handle, shutdown_signal) = pm_public_ws_shutdown_channel();
        let (sender, receiver) = mpsc::unbounded_channel();
        let clocks = Arc::new(Mutex::new(Vec::new()));
        let sink_clocks = Arc::clone(&clocks);
        let task = tokio::spawn(async move {
            let mut sink = TestSink {
                sender,
                clocks: sink_clocks,
                selected_facts: None,
                fail_at,
                delivered: 0,
                reconnect_authority_calls: 0,
                authorized_reconnects: 0,
                max_authorized_reconnects,
                authorized_backoff,
                planned_directives: None,
            };
            role.run(shutdown_signal, &mut sink).await
        });
        (shutdown_handle, receiver, clocks, task)
    }

    fn spawn_role_with_directives(
        config: PmPublicWsConfig,
        directives: Vec<PmPublicWsReconnectDirective>,
    ) -> SpawnedTestRole {
        let role = PmPublicMarketWsRole::with_clock_source(config, TestClock { next: 1 })
            .expect("public WS role");
        let (shutdown_handle, shutdown_signal) = pm_public_ws_shutdown_channel();
        let (sender, receiver) = mpsc::unbounded_channel();
        let clocks = Arc::new(Mutex::new(Vec::new()));
        let sink_clocks = Arc::clone(&clocks);
        let task = tokio::spawn(async move {
            let mut sink = TestSink {
                sender,
                clocks: sink_clocks,
                selected_facts: None,
                fail_at: None,
                delivered: 0,
                reconnect_authority_calls: 0,
                authorized_reconnects: 0,
                max_authorized_reconnects: 0,
                authorized_backoff: Duration::from_millis(1),
                planned_directives: Some(directives.into()),
            };
            role.run(shutdown_signal, &mut sink).await
        });
        (shutdown_handle, receiver, clocks, task)
    }

    async fn next_seen(receiver: &mut mpsc::UnboundedReceiver<Seen>) -> Seen {
        timeout(Duration::from_secs(2), receiver.recv())
            .await
            .expect("event timeout")
            .expect("event channel closed")
    }

    async fn collect_remaining(receiver: &mut mpsc::UnboundedReceiver<Seen>) -> Vec<Seen> {
        let mut events = Vec::new();
        while let Some(event) = receiver.recv().await {
            events.push(event);
        }
        events
    }

    async fn wait_for_condvar(pair: Arc<(Mutex<bool>, Condvar)>, message: &'static str) {
        let observed = tokio::task::spawn_blocking(move || {
            let (state, changed) = &*pair;
            let state = state.lock().unwrap();
            let (state, _) = changed
                .wait_timeout_while(state, Duration::from_secs(15), |observed| !*observed)
                .unwrap();
            *state
        })
        .await
        .expect("condition waiter task");
        assert!(observed, "{message}");
    }

    async fn read_subscription<S>(socket: &mut WebSocketStream<S>)
    where
        S: tokio::io::AsyncRead + tokio::io::AsyncWrite + Unpin,
    {
        let message = timeout(Duration::from_secs(1), socket.next())
            .await
            .expect("subscription timeout")
            .expect("subscription stream ended")
            .expect("subscription frame");
        assert_eq!(message, Message::text(CURRENT_SUBSCRIPTION));
        let text = message.into_text().unwrap();
        assert!(!text.contains("initial_dump"));
        assert!(!text.contains("operation"));
    }

    #[cfg(target_os = "linux")]
    #[tokio::test(flavor = "current_thread")]
    async fn selected_loopback_dialer_preserves_public_worker_protocol_across_reconnect() {
        LocalSet::new()
            .run_until(async {
                let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
                let address = listener.local_addr().unwrap();
                let exact_peer_ip: std::net::IpAddr = "127.0.0.1".parse().unwrap();
                let selected_source: std::net::IpAddr = "127.0.0.2".parse().unwrap();
                let decoy_ip: std::net::IpAddr = "127.0.0.3".parse().unwrap();
                let decoy = TcpListener::bind(format!("127.0.0.3:{}", address.port()))
                    .await
                    .unwrap();
                assert_eq!(address.ip(), exact_peer_ip);
                assert_eq!(decoy.local_addr().unwrap().ip(), decoy_ip);
                let server = tokio::task::spawn_local(async move {
                    for attempt in 0..2 {
                        let (stream, accepted_peer) = listener.accept().await.unwrap();
                        assert_eq!(accepted_peer.ip(), selected_source);
                        let mut socket = accept_async(stream).await.unwrap();
                        read_subscription(&mut socket).await;
                        if attempt == 0 {
                            socket.send(Message::Close(None)).await.unwrap();
                        } else {
                            while let Some(message) = socket.next().await {
                                match message {
                                    Ok(Message::Close(_)) | Err(_) => break,
                                    Ok(_) => {}
                                }
                            }
                        }
                    }
                });

                let endpoint = format!("ws://{address}/ws/market");
                let config = standard_config(address, 1_024, 1, 91);
                let role =
                    PmPublicMarketWsRole::with_clock_source(config, TestClock { next: 1 }).unwrap();
                let dialer = PmTestSelectedLoopbackWsDialer::new(
                    PmFixedWsRoute::PublicMarket,
                    &endpoint,
                    address,
                    "lo",
                    selected_source,
                )
                .unwrap();
                let (shutdown, signal) = pm_public_ws_shutdown_channel();
                let (sender, mut receiver) = mpsc::unbounded_channel();
                let clocks = Arc::new(Mutex::new(Vec::new()));
                let sink_clocks = Arc::clone(&clocks);
                let selected_facts = Arc::new(Mutex::new(Vec::new()));
                let sink_selected_facts = Arc::clone(&selected_facts);
                let task = tokio::task::spawn_local(async move {
                    let mut sink = TestSink {
                        sender,
                        clocks: sink_clocks,
                        selected_facts: Some(sink_selected_facts),
                        fail_at: None,
                        delivered: 0,
                        reconnect_authority_calls: 0,
                        authorized_reconnects: 0,
                        max_authorized_reconnects: 1,
                        authorized_backoff: Duration::from_millis(1),
                        planned_directives: None,
                    };
                    role.run_with_test_selected_loopback(signal, &mut sink, dialer)
                        .await
                });

                assert_eq!(next_seen(&mut receiver).await, Seen::Open(91));
                assert_eq!(next_seen(&mut receiver).await, Seen::Subscription(91));
                assert_eq!(
                    next_seen(&mut receiver).await,
                    Seen::Retired(91, PmPublicWsDisconnectReason::SocketClosed),
                );
                assert_eq!(next_seen(&mut receiver).await, Seen::Reconnect(91, 92, 1));
                assert_eq!(next_seen(&mut receiver).await, Seen::Open(92));
                assert_eq!(next_seen(&mut receiver).await, Seen::Subscription(92));
                shutdown.request_shutdown();
                assert_eq!(next_seen(&mut receiver).await, Seen::Shutdown(92));
                task.await.unwrap().unwrap();
                server.await.unwrap();
                assert!(
                    timeout(Duration::from_millis(50), decoy.accept())
                        .await
                        .is_err(),
                    "selected WebSocket dialer must leave the same-port decoy idle"
                );
                let selected_facts = selected_facts.lock().unwrap();
                assert_eq!(selected_facts.len(), 7);
                assert!(selected_facts.iter().all(|(_, facts)| facts.is_some()));
                let first_epoch_facts = selected_facts[0].1.unwrap();
                assert!(
                    selected_facts[..4]
                        .iter()
                        .all(|(epoch, facts)| *epoch == 91 && *facts == Some(first_epoch_facts))
                );
                let second_epoch_facts = selected_facts[4].1.unwrap();
                assert!(
                    selected_facts[4..]
                        .iter()
                        .all(|(epoch, facts)| *epoch == 92 && *facts == Some(second_epoch_facts))
                );
                assert_eq!(first_epoch_facts.interface_name(), "lo");
                assert_eq!(first_epoch_facts.peer_addr(), address);
                assert_eq!(first_epoch_facts.local_addr().ip(), selected_source);
                assert_eq!(second_epoch_facts.interface_name(), "lo");
                assert_eq!(second_epoch_facts.peer_addr(), address);
                assert_eq!(second_epoch_facts.local_addr().ip(), selected_source);
                assert_eq!(
                    clocks
                        .lock()
                        .unwrap()
                        .iter()
                        .map(|edge| edge.edge)
                        .collect::<Vec<_>>(),
                    [
                        "opened",
                        "subscription",
                        "retired",
                        "reconnect",
                        "opened",
                        "subscription",
                        "shutdown",
                    ],
                );
            })
            .await;
    }

    #[cfg(target_os = "linux")]
    #[tokio::test(flavor = "current_thread")]
    async fn selected_inline_public_sink_keeps_worker_live_and_drains_after_completion() {
        LocalSet::new()
            .run_until(async {
                let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
                let address = listener.local_addr().unwrap();
                let selected_source: std::net::IpAddr = "127.0.0.2".parse().unwrap();
                let allow_worker_progress = Arc::new(Notify::new());
                let server_allow_worker_progress = Arc::clone(&allow_worker_progress);
                let worker_completed = Arc::new(Notify::new());
                let server_completed = Arc::clone(&worker_completed);
                let server = tokio::task::spawn_local(async move {
                    let (stream, accepted_peer) = listener.accept().await.unwrap();
                    assert_eq!(accepted_peer.ip(), selected_source);
                    let mut socket = accept_async(stream).await.unwrap();
                    read_subscription(&mut socket).await;
                    server_allow_worker_progress.notified().await;
                    socket.feed(Message::text(r#"{"inline":1}"#)).await.unwrap();
                    socket.feed(Message::text(r#"{"inline":2}"#)).await.unwrap();
                    socket.flush().await.unwrap();
                    timeout(Duration::from_secs(2), async {
                        loop {
                            match socket.next().await {
                                None | Some(Err(_)) | Some(Ok(Message::Close(_))) => break,
                                Some(Ok(_)) => {}
                            }
                        }
                    })
                    .await
                    .expect("selected public worker did not close after saturated handoff");
                    server_completed.notify_one();
                });

                let endpoint = format!("ws://{address}/ws/market");
                let config = PmPublicWsConfig::local_evidence(
                    &endpoint,
                    scope(),
                    Duration::from_millis(200),
                    Duration::from_secs(30),
                    Duration::from_secs(10),
                    Duration::from_secs(2),
                    1_024,
                    0,
                    Duration::from_millis(5),
                    2,
                    ConnectionEpoch::new(131),
                )
                .unwrap();
                let role =
                    PmPublicMarketWsRole::with_clock_source(config, TestClock { next: 1 }).unwrap();
                let activity = role.activity_view();
                let dialer = PmTestSelectedLoopbackWsDialer::new(
                    PmFixedWsRoute::PublicMarket,
                    &endpoint,
                    address,
                    "lo",
                    selected_source,
                )
                .unwrap();
                let (_shutdown, signal) = pm_public_ws_shutdown_channel();
                let entered = Arc::new(Notify::new());
                let release = Arc::new(Notify::new());
                let delivered = Arc::new(Mutex::new(Vec::new()));
                let sink_entered = Arc::clone(&entered);
                let sink_release = Arc::clone(&release);
                let sink_delivered = Arc::clone(&delivered);
                let mut task = tokio::task::spawn_local(async move {
                    let mut sink = BlockingSelectedInlineSink {
                        entered: sink_entered,
                        release: sink_release,
                        delivered: sink_delivered,
                    };
                    role.run_with_test_selected_loopback(signal, &mut sink, dialer)
                        .await
                });

                timeout(Duration::from_secs(2), entered.notified())
                    .await
                    .expect("selected public Open never entered the sink barrier");
                allow_worker_progress.notify_one();
                timeout(Duration::from_secs(2), worker_completed.notified())
                    .await
                    .expect("selected public worker stopped polling behind the sink barrier");
                assert!(activity.generation() >= 4);
                assert!(timeout(Duration::from_millis(50), &mut task).await.is_err());
                release.notify_one();
                assert!(matches!(
                    timeout(Duration::from_secs(2), task)
                        .await
                        .expect("selected public inline pump did not return")
                        .unwrap(),
                    Err(PmPublicWsRunError::Transport(
                        PmPublicWsTransportError::EventChannelSaturated
                    ))
                ));
                assert_eq!(
                    delivered.lock().unwrap().as_slice(),
                    ["opened", "subscription", "raw"]
                );
                server.await.unwrap();
            })
            .await;
    }

    #[cfg(target_os = "linux")]
    #[tokio::test(flavor = "current_thread")]
    async fn selected_public_backoff_shutdown_retains_retired_epoch_facts() {
        LocalSet::new()
            .run_until(async {
                let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
                let address = listener.local_addr().unwrap();
                let selected_source: std::net::IpAddr = "127.0.0.2".parse().unwrap();
                let server = tokio::task::spawn_local(async move {
                    let (stream, accepted_peer) = listener.accept().await.unwrap();
                    assert_eq!(accepted_peer.ip(), selected_source);
                    let mut socket = accept_async(stream).await.unwrap();
                    read_subscription(&mut socket).await;
                    socket.send(Message::Close(None)).await.unwrap();
                });
                let endpoint = format!("ws://{address}/ws/market");
                let config = local_config(
                    address,
                    Duration::from_millis(200),
                    Duration::from_millis(500),
                    Duration::from_millis(50),
                    Duration::from_millis(20),
                    1_024,
                    1,
                    Duration::from_secs(5),
                    121,
                );
                let role =
                    PmPublicMarketWsRole::with_clock_source(config, TestClock { next: 1 }).unwrap();
                let dialer = PmTestSelectedLoopbackWsDialer::new(
                    PmFixedWsRoute::PublicMarket,
                    &endpoint,
                    address,
                    "lo",
                    selected_source,
                )
                .unwrap();
                let (shutdown, signal) = pm_public_ws_shutdown_channel();
                let (sender, mut receiver) = mpsc::unbounded_channel();
                let facts = Arc::new(Mutex::new(Vec::new()));
                let sink_facts = Arc::clone(&facts);
                let task = tokio::task::spawn_local(async move {
                    let mut sink = TestSink {
                        sender,
                        clocks: Arc::new(Mutex::new(Vec::new())),
                        selected_facts: Some(sink_facts),
                        fail_at: None,
                        delivered: 0,
                        reconnect_authority_calls: 0,
                        authorized_reconnects: 0,
                        max_authorized_reconnects: 1,
                        authorized_backoff: Duration::from_secs(5),
                        planned_directives: None,
                    };
                    role.run_with_test_selected_loopback(signal, &mut sink, dialer)
                        .await
                });
                assert_eq!(next_seen(&mut receiver).await, Seen::Open(121));
                assert_eq!(next_seen(&mut receiver).await, Seen::Subscription(121));
                assert_eq!(
                    next_seen(&mut receiver).await,
                    Seen::Retired(121, PmPublicWsDisconnectReason::SocketClosed)
                );
                assert_eq!(next_seen(&mut receiver).await, Seen::Reconnect(121, 122, 1));
                shutdown.request_shutdown();
                assert_eq!(next_seen(&mut receiver).await, Seen::Shutdown(121));
                task.await.unwrap().unwrap();
                server.await.unwrap();
                let facts = facts.lock().unwrap();
                assert_eq!(facts.len(), 5);
                let exact = facts[0].1.expect("selected facts");
                assert!(
                    facts
                        .iter()
                        .all(|(epoch, observed)| *epoch == 121 && *observed == Some(exact))
                );
            })
            .await;
    }

    #[cfg(target_os = "linux")]
    #[tokio::test(flavor = "current_thread")]
    async fn selected_binding_failure_is_terminal_before_public_reconnect_authority() {
        let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
        let address = listener.local_addr().unwrap();
        let selected_source = "127.0.0.2".parse().unwrap();
        let endpoint = format!("ws://{address}/ws/market");
        let config = standard_config(address, 1_024, 3, 101);
        let role = PmPublicMarketWsRole::with_clock_source(config, TestClock { next: 1 }).unwrap();
        let dialer = PmTestSelectedLoopbackWsDialer::new(
            PmFixedWsRoute::PublicMarket,
            &endpoint,
            address,
            "missing0",
            selected_source,
        )
        .unwrap();
        let (_shutdown, signal) = pm_public_ws_shutdown_channel();
        let (sender, mut receiver) = mpsc::unbounded_channel();
        let facts = Arc::new(Mutex::new(Vec::new()));
        let mut sink = TestSink {
            sender,
            clocks: Arc::new(Mutex::new(Vec::new())),
            selected_facts: Some(Arc::clone(&facts)),
            fail_at: None,
            delivered: 0,
            reconnect_authority_calls: 0,
            authorized_reconnects: 0,
            max_authorized_reconnects: 3,
            authorized_backoff: Duration::from_millis(1),
            planned_directives: None,
        };
        let result = role
            .run_with_test_selected_loopback(signal, &mut sink, dialer)
            .await;
        assert!(matches!(
            result,
            Err(PmPublicWsRunError::Transport(
                PmPublicWsTransportError::WorkerFailed
            ))
        ));
        assert_eq!(sink.reconnect_authority_calls, 0);
        assert_eq!(
            next_seen(&mut receiver).await,
            Seen::Retired(101, PmPublicWsDisconnectReason::ConnectFailed)
        );
        assert_eq!(
            next_seen(&mut receiver).await,
            Seen::Stopped(101, PmPublicWsDisconnectReason::ConnectFailed)
        );
        assert!(matches!(
            receiver.try_recv(),
            Err(mpsc::error::TryRecvError::Empty)
        ));
        assert_eq!(facts.lock().unwrap().as_slice(), [(101, None), (101, None)]);
        assert!(
            timeout(Duration::from_millis(50), listener.accept())
                .await
                .is_err(),
            "terminal device binding must not fall back to an unbound connect"
        );
    }

    #[cfg(target_os = "linux")]
    #[tokio::test(flavor = "current_thread")]
    async fn selected_exact_peer_refusal_remains_publicly_authorized_retryable() {
        let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
        let address = listener.local_addr().unwrap();
        let selected_source = "127.0.0.2".parse().unwrap();
        drop(listener);
        let endpoint = format!("ws://{address}/ws/market");
        let config = standard_config(address, 1_024, 1, 111);
        let role = PmPublicMarketWsRole::with_clock_source(config, TestClock { next: 1 }).unwrap();
        let dialer = PmTestSelectedLoopbackWsDialer::new(
            PmFixedWsRoute::PublicMarket,
            &endpoint,
            address,
            "lo",
            selected_source,
        )
        .unwrap();
        let (_shutdown, signal) = pm_public_ws_shutdown_channel();
        let (sender, mut receiver) = mpsc::unbounded_channel();
        let mut sink = TestSink {
            sender,
            clocks: Arc::new(Mutex::new(Vec::new())),
            selected_facts: None,
            fail_at: None,
            delivered: 0,
            reconnect_authority_calls: 0,
            authorized_reconnects: 0,
            max_authorized_reconnects: 0,
            authorized_backoff: Duration::from_millis(1),
            planned_directives: None,
        };
        role.run_with_test_selected_loopback(signal, &mut sink, dialer)
            .await
            .unwrap();
        assert_eq!(sink.reconnect_authority_calls, 1);
        assert_eq!(
            next_seen(&mut receiver).await,
            Seen::Retired(111, PmPublicWsDisconnectReason::ConnectFailed)
        );
        assert_eq!(
            next_seen(&mut receiver).await,
            Seen::Stopped(111, PmPublicWsDisconnectReason::ConnectFailed)
        );
    }

    #[allow(clippy::result_large_err)]
    #[tokio::test]
    async fn delayed_application_pong_rebases_two_cycle_transport_and_canonical_session() {
        const HEARTBEAT: Duration = Duration::from_millis(800);
        const PONG_TIMEOUT: Duration = Duration::from_millis(600);
        const DELAYED_PONG: Duration = Duration::from_millis(200);
        const NO_EARLY_PING_WINDOW: Duration = Duration::from_millis(700);

        let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
        let address = listener.local_addr().unwrap();
        let (wire_edges, mut observed_wire_edges) = mpsc::unbounded_channel();
        let server = tokio::spawn(async move {
            let (stream, _) = listener.accept().await.unwrap();
            let mut socket = accept_hdr_async(stream, |request: &Request, response: Response| {
                assert_eq!(request.uri().path(), "/ws/market");
                Ok(response)
            })
            .await
            .unwrap();
            read_subscription(&mut socket).await;
            assert_eq!(
                socket.next().await.unwrap().unwrap(),
                Message::text(APPLICATION_PING)
            );
            wire_edges.send(1_u8).unwrap();
            tokio::time::sleep(DELAYED_PONG).await;
            socket.send(Message::text(APPLICATION_PONG)).await.unwrap();
            wire_edges.send(2_u8).unwrap();
            assert_eq!(
                socket.next().await.unwrap().unwrap(),
                Message::text(APPLICATION_PING)
            );
            wire_edges.send(3_u8).unwrap();
            socket.send(Message::text(APPLICATION_PONG)).await.unwrap();
            wire_edges.send(4_u8).unwrap();
            let _ = socket.next().await;
        });

        let config = local_config(
            address,
            Duration::from_secs(2),
            Duration::from_secs(5),
            HEARTBEAT,
            PONG_TIMEOUT,
            1_024,
            0,
            Duration::from_millis(1),
            7,
        );
        let origin = Instant::now();
        let role = PmPublicMarketWsRole::with_clock_source(config, TokioInstantClock { origin })
            .expect("public WS role");
        let (shutdown, shutdown_signal) = pm_public_ws_shutdown_channel();
        let (sender, mut events) = mpsc::unbounded_channel();
        let clocks = Arc::new(Mutex::new(Vec::new()));
        let sink_clocks = Arc::clone(&clocks);
        let task = tokio::spawn(async move {
            let mut sink = CanonicalHeartbeatSink {
                inner: TestSink {
                    sender,
                    clocks: sink_clocks,
                    selected_facts: None,
                    fail_at: None,
                    delivered: 0,
                    reconnect_authority_calls: 0,
                    authorized_reconnects: 0,
                    max_authorized_reconnects: 0,
                    authorized_backoff: Duration::from_millis(1),
                    planned_directives: None,
                },
                session: canonical_heartbeat_session(
                    HEARTBEAT,
                    PONG_TIMEOUT,
                    ConnectionEpoch::new(7),
                ),
            };
            role.run(shutdown_signal, &mut sink).await
        });

        assert_eq!(next_seen(&mut events).await, Seen::Open(7));
        assert_eq!(next_seen(&mut events).await, Seen::Subscription(7));
        assert_eq!(observed_wire_edges.recv().await, Some(1));
        assert_eq!(next_seen(&mut events).await, Seen::Ping(7));

        assert_eq!(observed_wire_edges.recv().await, Some(2));
        assert_eq!(next_seen(&mut events).await, Seen::Pong(7));

        // The old send-based schedule becomes due 600 ms after this delayed
        // PONG. The canonical PONG-based schedule remains quiet for 800 ms.
        assert!(timeout(NO_EARLY_PING_WINDOW, events.recv()).await.is_err());
        assert!(matches!(
            observed_wire_edges.try_recv(),
            Err(mpsc::error::TryRecvError::Empty)
        ));

        assert_eq!(
            timeout(Duration::from_secs(2), observed_wire_edges.recv())
                .await
                .expect("second heartbeat wire timeout"),
            Some(3),
        );
        assert_eq!(next_seen(&mut events).await, Seen::Ping(7));
        assert_eq!(observed_wire_edges.recv().await, Some(4));
        assert_eq!(next_seen(&mut events).await, Seen::Pong(7));

        shutdown.request_shutdown();
        assert_eq!(next_seen(&mut events).await, Seen::Shutdown(7));
        task.await.unwrap().unwrap();
        server.await.unwrap();

        let clocks = clocks.lock().unwrap();
        let first_ping = clocks.iter().find(|edge| edge.edge == "ping").unwrap();
        let first_pong = clocks.iter().find(|edge| edge.edge == "pong").unwrap();
        let second_ping = clocks
            .iter()
            .filter(|edge| edge.edge == "ping")
            .nth(1)
            .unwrap();
        assert!(
            second_ping.monotonic_receive_ns >= first_pong.monotonic_receive_ns + 800_000_000,
            "the next ping must not precede the PONG-based canonical deadline",
        );
        assert!(
            second_ping.monotonic_receive_ns > first_ping.monotonic_receive_ns + 800_000_000,
            "the delayed PONG must move the next ping beyond the prior send schedule",
        );
    }

    #[allow(clippy::result_large_err)]
    #[tokio::test]
    async fn exact_current_subscription_raw_malformed_pong_and_shutdown_are_preserved() {
        let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
        let address = listener.local_addr().unwrap();
        let server = tokio::spawn(async move {
            let (stream, _) = listener.accept().await.unwrap();
            let mut socket = accept_hdr_async(stream, |request: &Request, response: Response| {
                assert_eq!(request.uri().path(), "/ws/market");
                assert!(request.uri().query().is_none());
                Ok(response)
            })
            .await
            .unwrap();
            read_subscription(&mut socket).await;
            socket.send(Message::text("{not-json")).await.unwrap();
            match socket.next().await.unwrap().unwrap() {
                Message::Text(text) if text.as_str() == APPLICATION_PING => {
                    socket.send(Message::text(APPLICATION_PONG)).await.unwrap();
                }
                other => panic!("unexpected client frame: {other:?}"),
            }
            let _ = socket.next().await;
        });
        let config = standard_config(address, 1_024, 0, 7);
        let (shutdown, mut events, clocks, role) = spawn_role(config, None);

        assert_eq!(next_seen(&mut events).await, Seen::Open(7));
        assert_eq!(next_seen(&mut events).await, Seen::Subscription(7));
        assert_eq!(
            next_seen(&mut events).await,
            Seen::Raw(7, b"{not-json".to_vec())
        );
        assert_eq!(next_seen(&mut events).await, Seen::Ping(7));
        assert_eq!(next_seen(&mut events).await, Seen::Pong(7));
        shutdown.request_shutdown();
        assert_eq!(next_seen(&mut events).await, Seen::Shutdown(7));
        role.await.unwrap().unwrap();
        server.await.unwrap();
        assert_eq!(
            *clocks.lock().unwrap(),
            [
                SeenClock {
                    edge: "opened",
                    local_wall_receive_ns: 1_000_001,
                    monotonic_receive_ns: 1,
                    activity_generation: 1,
                },
                SeenClock {
                    edge: "subscription",
                    local_wall_receive_ns: 1_000_002,
                    monotonic_receive_ns: 2,
                    activity_generation: 2,
                },
                SeenClock {
                    edge: "raw",
                    local_wall_receive_ns: 1_000_003,
                    monotonic_receive_ns: 3,
                    activity_generation: 3,
                },
                SeenClock {
                    edge: "ping",
                    local_wall_receive_ns: 1_000_004,
                    monotonic_receive_ns: 4,
                    activity_generation: 4,
                },
                SeenClock {
                    edge: "pong",
                    local_wall_receive_ns: 1_000_005,
                    monotonic_receive_ns: 5,
                    activity_generation: 5,
                },
                SeenClock {
                    edge: "shutdown",
                    local_wall_receive_ns: 1_000_006,
                    monotonic_receive_ns: 6,
                    activity_generation: 6,
                },
            ]
        );
    }

    #[tokio::test(flavor = "multi_thread", worker_threads = 4)]
    async fn second_receive_edge_is_stamped_while_first_raw_sink_service_is_blocked() {
        let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
        let address = listener.local_addr().unwrap();
        let server = tokio::spawn(async move {
            let (stream, _) = listener.accept().await.unwrap();
            let mut socket = accept_async(stream).await.unwrap();
            read_subscription(&mut socket).await;
            socket.feed(Message::text(r#"{"n":1}"#)).await.unwrap();
            socket.feed(Message::text(r#"{"n":2}"#)).await.unwrap();
            socket.flush().await.unwrap();
            while let Some(Ok(message)) = socket.next().await {
                match message {
                    Message::Text(text) if text.as_str() == APPLICATION_PING => {
                        socket.send(Message::text(APPLICATION_PONG)).await.unwrap();
                    }
                    Message::Close(_) => break,
                    _ => {}
                }
            }
        });

        let raw_two_sampled = Arc::new((Mutex::new(false), Condvar::new()));
        let next_clock = Arc::new(AtomicU64::new(1));
        let role = PmPublicMarketWsRole::with_clock_source(
            local_config(
                address,
                Duration::from_secs(5),
                Duration::from_secs(30),
                Duration::from_secs(10),
                Duration::from_secs(5),
                1_024,
                0,
                Duration::from_secs(1),
                1,
            ),
            QueueClock {
                next: Arc::clone(&next_clock),
                raw_two_sampled: Arc::clone(&raw_two_sampled),
            },
        )
        .unwrap();
        let activity_view = role.activity_view();
        let sink_gate = Arc::new((Mutex::new(RawSinkGate::default()), Condvar::new()));
        let sink_task_gate = Arc::clone(&sink_gate);
        let (shutdown, signal) = pm_public_ws_shutdown_channel();
        let role_task = tokio::spawn(async move {
            let mut sink = BlockingRawSink {
                gate: sink_task_gate,
                blocked_once: false,
            };
            role.run(signal, &mut sink).await
        });

        let gate_wait = Arc::clone(&sink_gate);
        let entered = tokio::task::spawn_blocking(move || {
            let (state, changed) = &*gate_wait;
            let state = state.lock().unwrap();
            let (state, _) = changed
                .wait_timeout_while(state, Duration::from_secs(15), |state| {
                    !state.entered_first_raw
                })
                .unwrap();
            state.entered_first_raw
        })
        .await
        .expect("first raw waiter task");
        assert!(entered, "first raw never reached the blocking sink");
        wait_for_condvar(
            Arc::clone(&raw_two_sampled),
            "second raw was not receive-edge stamped behind the bounded queue",
        )
        .await;
        let sampled_clock_count = next_clock.load(Ordering::SeqCst);
        assert!(
            activity_view.generation() >= 4,
            "second queued raw must advance the source high-water beyond the admitted first raw",
        );

        {
            let (state, changed) = &*sink_gate;
            let mut state = state.lock().unwrap();
            state.release_first_raw = true;
            changed.notify_all();
        }
        shutdown.request_shutdown();
        timeout(Duration::from_secs(15), role_task)
            .await
            .expect("role did not stop after releasing the sink")
            .unwrap()
            .unwrap();
        timeout(Duration::from_secs(15), server)
            .await
            .expect("loopback server did not observe shutdown")
            .unwrap();
        assert!(sampled_clock_count >= 5);
    }

    #[tokio::test(flavor = "multi_thread", worker_threads = 4)]
    async fn queued_retirement_advances_activity_before_the_blocked_sink_admits_it() {
        let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
        let address = listener.local_addr().unwrap();
        let server = tokio::spawn(async move {
            let (stream, _) = listener.accept().await.unwrap();
            let mut socket = accept_async(stream).await.unwrap();
            read_subscription(&mut socket).await;
            socket.close(None).await.unwrap();
        });

        let entered = Arc::new(Notify::new());
        let release = Arc::new(Notify::new());
        let role = PmPublicMarketWsRole::with_clock_source(
            standard_config(address, 1_024, 0, 19),
            TestClock { next: 1 },
        )
        .unwrap();
        let activity = role.activity_view();
        let (_shutdown, signal) = pm_public_ws_shutdown_channel();
        let sink_entered = Arc::clone(&entered);
        let sink_release = Arc::clone(&release);
        let task = tokio::spawn(async move {
            let mut sink = BlockingOpenedSink {
                entered: sink_entered,
                release: sink_release,
            };
            role.run(signal, &mut sink).await
        });

        timeout(Duration::from_secs(5), entered.notified())
            .await
            .expect("opened evidence never reached blocked sink");
        timeout(Duration::from_secs(5), async {
            while activity.generation() < 3 {
                tokio::task::yield_now().await;
            }
        })
        .await
        .expect("retirement was not source-stamped behind the blocked sink");
        assert!(
            activity.generation() > 1,
            "queued lifecycle evidence must invalidate the last admitted open generation",
        );

        release.notify_one();
        timeout(Duration::from_secs(5), task)
            .await
            .expect("role did not finish after releasing lifecycle sink")
            .unwrap()
            .unwrap();
        server.await.unwrap();
    }

    #[test]
    fn activity_generation_overflow_is_closed_and_never_wraps() {
        let (source, view) = PmPublicWsActivitySource::new();
        source.generation.store(u64::MAX, AtomicOrdering::Release);
        assert_eq!(view.generation(), u64::MAX);
        assert_eq!(
            source.issue(),
            Err(PmPublicWsTransportError::ActivityGenerationOverflow)
        );
        assert_eq!(view.generation(), u64::MAX);
    }

    #[tokio::test]
    async fn binary_frame_is_never_delivered_as_public_data() {
        let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
        let address = listener.local_addr().unwrap();
        let server = tokio::spawn(async move {
            let (stream, _) = listener.accept().await.unwrap();
            let mut socket = accept_async(stream).await.unwrap();
            read_subscription(&mut socket).await;
            socket
                .send(Message::binary(vec![1_u8, 2, 3]))
                .await
                .unwrap();
        });
        let (_shutdown, mut events, _clocks, role) =
            spawn_role(standard_config(address, 64, 0, 1), None);
        role.await.unwrap().unwrap();
        let seen = collect_remaining(&mut events).await;
        assert!(seen.contains(&Seen::Retired(1, PmPublicWsDisconnectReason::BinaryFrame)));
        assert!(seen.contains(&Seen::Stopped(1, PmPublicWsDisconnectReason::BinaryFrame)));
        assert!(!seen.iter().any(|event| matches!(event, Seen::Raw(..))));
        server.await.unwrap();
    }

    #[tokio::test]
    async fn oversized_frame_is_rejected_before_raw_delivery() {
        let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
        let address = listener.local_addr().unwrap();
        let server = tokio::spawn(async move {
            let (stream, _) = listener.accept().await.unwrap();
            let mut socket = accept_async(stream).await.unwrap();
            read_subscription(&mut socket).await;
            socket.send(Message::text("x".repeat(65))).await.unwrap();
        });
        let (_shutdown, mut events, _clocks, role) =
            spawn_role(standard_config(address, 64, 0, 2), None);
        role.await.unwrap().unwrap();
        let seen = collect_remaining(&mut events).await;
        assert!(seen.contains(&Seen::Stopped(2, PmPublicWsDisconnectReason::FrameTooLarge)));
        assert!(!seen.iter().any(|event| matches!(event, Seen::Raw(..))));
        server.await.unwrap();
    }

    #[tokio::test]
    async fn disconnect_reconnect_replaces_epoch_and_resends_exact_subscription() {
        let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
        let address = listener.local_addr().unwrap();
        let server = tokio::spawn(async move {
            let (first, _) = listener.accept().await.unwrap();
            let mut first = accept_async(first).await.unwrap();
            read_subscription(&mut first).await;
            first.close(None).await.unwrap();

            let (second, _) = listener.accept().await.unwrap();
            let mut second = accept_async(second).await.unwrap();
            read_subscription(&mut second).await;
            second
                .send(Message::text(r#"{"event_type":"book"}"#))
                .await
                .unwrap();
            let _ = second.next().await;
        });
        let (shutdown, mut events, _clocks, role) =
            spawn_role(standard_config(address, 1_024, 1, 41), None);

        let mut observed = Vec::new();
        loop {
            let event = next_seen(&mut events).await;
            let done = event == Seen::Raw(42, br#"{"event_type":"book"}"#.to_vec());
            observed.push(event);
            if done {
                break;
            }
        }
        assert!(observed.contains(&Seen::Retired(41, PmPublicWsDisconnectReason::SocketClosed)));
        assert!(observed.contains(&Seen::Reconnect(41, 42, 1)));
        assert!(observed.contains(&Seen::Open(42)));
        assert!(observed.contains(&Seen::Subscription(42)));
        shutdown.request_shutdown();
        assert_eq!(next_seen(&mut events).await, Seen::Shutdown(42));
        role.await.unwrap().unwrap();
        server.await.unwrap();
    }

    #[tokio::test]
    async fn refused_connections_consume_the_exact_bounded_retry_budget() {
        let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
        let address = listener.local_addr().unwrap();
        drop(listener);
        let (_shutdown, mut events, _clocks, role) = spawn_role(
            local_config(
                address,
                Duration::from_millis(50),
                Duration::from_millis(500),
                Duration::from_millis(50),
                Duration::from_millis(20),
                1_024,
                2,
                Duration::from_millis(5),
                8,
            ),
            None,
        );
        role.await.unwrap().unwrap();
        let seen = collect_remaining(&mut events).await;
        assert_eq!(
            seen.iter()
                .filter(|event| matches!(event, Seen::Retired(..)))
                .count(),
            3
        );
        assert!(seen.contains(&Seen::Reconnect(8, 9, 1)));
        assert!(seen.contains(&Seen::Reconnect(9, 10, 2)));
        assert!(seen.contains(&Seen::Stopped(
            10,
            PmPublicWsDisconnectReason::ConnectFailed
        )));
    }

    #[tokio::test]
    async fn handshake_and_application_pong_timeouts_are_classified() {
        let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
        let address = listener.local_addr().unwrap();
        let server = tokio::spawn(async move {
            let (_stream, _) = listener.accept().await.unwrap();
            tokio::time::sleep(Duration::from_millis(150)).await;
        });
        let config = local_config(
            address,
            Duration::from_millis(30),
            Duration::from_millis(500),
            Duration::from_millis(50),
            Duration::from_millis(20),
            1_024,
            0,
            Duration::from_millis(5),
            1,
        );
        let (_shutdown, _events, _clocks, role) = spawn_role(config, None);
        role.await.unwrap().unwrap();
        server.await.unwrap();

        let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
        let address = listener.local_addr().unwrap();
        let server = tokio::spawn(async move {
            let (stream, _) = listener.accept().await.unwrap();
            let mut socket = accept_async(stream).await.unwrap();
            read_subscription(&mut socket).await;
            let ping = socket.next().await.unwrap().unwrap();
            assert_eq!(ping, Message::text(APPLICATION_PING));
            tokio::time::sleep(Duration::from_millis(100)).await;
        });
        let (_shutdown, _events, _clocks, role) =
            spawn_role(standard_config(address, 1_024, 0, 3), None);
        role.await.unwrap().unwrap();
        server.await.unwrap();
    }

    #[tokio::test]
    async fn reconnect_authority_controls_pre_ready_attempts_and_post_ready_reset() {
        let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
        let address = listener.local_addr().unwrap();
        let server = tokio::spawn(async move {
            for index in 0..4 {
                let (stream, _) = listener.accept().await.unwrap();
                let mut socket = accept_async(stream).await.unwrap();
                read_subscription(&mut socket).await;
                if index == 2 {
                    socket
                        .send(Message::text(r#"{"flow":"open"}"#))
                        .await
                        .unwrap();
                }
                if index == 3 {
                    socket
                        .send(Message::text(r#"{"after":"reset"}"#))
                        .await
                        .unwrap();
                    let _ = socket.next().await;
                } else {
                    socket.close(None).await.unwrap();
                }
            }
        });
        let directives = vec![
            PmPublicWsReconnectDirective::reconnect(
                ConnectionEpoch::new(10),
                ConnectionEpoch::new(11),
                1,
                Duration::from_millis(1),
            ),
            PmPublicWsReconnectDirective::reconnect(
                ConnectionEpoch::new(11),
                ConnectionEpoch::new(12),
                2,
                Duration::from_millis(2),
            ),
            // Composition observed flow-open on epoch 12 and reset its
            // session-owned reconnect policy. Transport obeys attempt 1; it
            // neither infers readiness nor insists on attempt 3.
            PmPublicWsReconnectDirective::reconnect(
                ConnectionEpoch::new(12),
                ConnectionEpoch::new(13),
                1,
                Duration::from_millis(1),
            ),
        ];
        let (shutdown, mut events, _clocks, role) =
            spawn_role_with_directives(standard_config(address, 1_024, 3, 10), directives);
        let mut seen = Vec::new();
        loop {
            let event = next_seen(&mut events).await;
            let finished = event == Seen::Raw(13, br#"{"after":"reset"}"#.to_vec());
            seen.push(event);
            if finished {
                break;
            }
        }
        assert!(seen.contains(&Seen::Reconnect(10, 11, 1)));
        assert!(seen.contains(&Seen::Reconnect(11, 12, 2)));
        assert!(seen.contains(&Seen::Raw(12, br#"{"flow":"open"}"#.to_vec())));
        assert!(seen.contains(&Seen::Reconnect(12, 13, 1)));
        shutdown.request_shutdown();
        assert_eq!(next_seen(&mut events).await, Seen::Shutdown(13));
        role.await.unwrap().unwrap();
        server.await.unwrap();
    }

    #[tokio::test]
    async fn stale_or_out_of_bounds_reconnect_directive_is_rejected() {
        let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
        let address = listener.local_addr().unwrap();
        let server = tokio::spawn(async move {
            let (stream, _) = listener.accept().await.unwrap();
            let mut socket = accept_async(stream).await.unwrap();
            read_subscription(&mut socket).await;
            socket.close(None).await.unwrap();
        });
        let (_shutdown, _events, _clocks, role) = spawn_role_with_directives(
            standard_config(address, 1_024, 2, 7),
            vec![PmPublicWsReconnectDirective::reconnect(
                ConnectionEpoch::new(6),
                ConnectionEpoch::new(8),
                1,
                Duration::from_millis(1),
            )],
        );
        assert!(matches!(
            role.await.unwrap().unwrap_err(),
            PmPublicWsRunError::Transport(PmPublicWsTransportError::InvalidReconnectDirective)
        ));
        server.await.unwrap();
    }

    #[tokio::test(flavor = "multi_thread", worker_threads = 4)]
    async fn saturated_public_evidence_queue_fails_closed_without_trapping_shutdown() {
        let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
        let address = listener.local_addr().unwrap();
        let (socket_closed, observed_socket_close) = oneshot::channel();
        let server = tokio::spawn(async move {
            let (stream, _) = listener.accept().await.unwrap();
            let mut socket = accept_async(stream).await.unwrap();
            read_subscription(&mut socket).await;
            let _ = socket.send(Message::text(r#"{"queue":"full"}"#)).await;
            let _ = timeout(Duration::from_secs(5), socket.next()).await;
            let _ = socket_closed.send(());
        });

        let entered = Arc::new(Notify::new());
        let release = Arc::new(Notify::new());
        let role = PmPublicMarketWsRole::with_clock_source(
            saturation_config(address),
            TestClock { next: 1 },
        )
        .unwrap();
        let activity = role.activity_view();
        let (_shutdown, signal) = pm_public_ws_shutdown_channel();
        let sink_entered = Arc::clone(&entered);
        let sink_release = Arc::clone(&release);
        let task = tokio::spawn(async move {
            let mut sink = BlockingOpenedSink {
                entered: sink_entered,
                release: sink_release,
            };
            role.run(signal, &mut sink).await
        });

        timeout(Duration::from_secs(5), entered.notified())
            .await
            .expect("opened evidence never reached sink");
        timeout(Duration::from_secs(6), observed_socket_close)
            .await
            .expect("saturated worker did not close its socket")
            .expect("server close observer dropped");
        release.notify_one();
        assert!(matches!(
            timeout(Duration::from_secs(5), task)
                .await
                .expect("public role remained trapped behind blocked sink")
                .unwrap(),
            Err(PmPublicWsRunError::Transport(
                PmPublicWsTransportError::EventChannelSaturated
            ))
        ));
        assert!(
            activity.generation() > 1,
            "failed handoff must leave the source high-water ahead of admitted evidence",
        );
        server.await.unwrap();
    }

    #[tokio::test(flavor = "multi_thread", worker_threads = 4)]
    async fn shutdown_waits_for_an_admitted_public_sink_delivery() {
        let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
        let address = listener.local_addr().unwrap();
        let server = tokio::spawn(async move {
            let (stream, _) = listener.accept().await.unwrap();
            let mut socket = accept_async(stream).await.unwrap();
            let _ = timeout(Duration::from_secs(5), socket.next()).await;
            let _ = timeout(Duration::from_secs(5), socket.next()).await;
        });

        let entered = Arc::new(Notify::new());
        let release = Arc::new(Notify::new());
        let role = PmPublicMarketWsRole::with_clock_source(
            standard_config(address, 1_024, 0, 71),
            TestClock { next: 1 },
        )
        .unwrap();
        let (shutdown, signal) = pm_public_ws_shutdown_channel();
        let sink_entered = Arc::clone(&entered);
        let sink_release = Arc::clone(&release);
        let mut task = tokio::spawn(async move {
            let mut sink = BlockingOpenedSink {
                entered: sink_entered,
                release: sink_release,
            };
            role.run(signal, &mut sink).await
        });

        timeout(Duration::from_secs(5), entered.notified())
            .await
            .expect("opened evidence never reached blocked sink");
        shutdown.request_shutdown();
        assert!(timeout(Duration::from_millis(50), &mut task).await.is_err());
        release.notify_one();
        timeout(Duration::from_secs(5), task)
            .await
            .expect("public run did not finish after admitted sink completed")
            .unwrap()
            .unwrap();
        timeout(Duration::from_secs(6), server)
            .await
            .expect("server did not observe public socket teardown")
            .unwrap();
    }

    #[tokio::test(flavor = "multi_thread", worker_threads = 4)]
    async fn shutdown_does_not_cancel_an_admitted_raw_capture_barrier() {
        let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
        let address = listener.local_addr().unwrap();
        let server = tokio::spawn(async move {
            let (stream, _) = listener.accept().await.unwrap();
            let mut socket = accept_async(stream).await.unwrap();
            read_subscription(&mut socket).await;
            socket
                .send(Message::text(r#"{"shutdown":"barrier"}"#))
                .await
                .unwrap();
            let _ = timeout(Duration::from_secs(5), socket.next()).await;
        });

        let entered = Arc::new(Notify::new());
        let release = Arc::new(Notify::new());
        let role = PmPublicMarketWsRole::with_clock_source(
            standard_config(address, 1_024, 0, 73),
            TestClock { next: 1 },
        )
        .unwrap();
        let (shutdown, signal) = pm_public_ws_shutdown_channel();
        let sink_entered = Arc::clone(&entered);
        let sink_release = Arc::clone(&release);
        let mut task = tokio::spawn(async move {
            let mut sink = BlockingRawDeliverySink {
                entered: sink_entered,
                release: sink_release,
            };
            role.run(signal, &mut sink).await
        });

        timeout(Duration::from_secs(5), entered.notified())
            .await
            .expect("raw evidence never entered its capture barrier");
        shutdown.request_shutdown();
        assert!(timeout(Duration::from_millis(50), &mut task).await.is_err());
        release.notify_one();
        timeout(Duration::from_secs(5), task)
            .await
            .expect("public run did not finish after raw capture barrier")
            .unwrap()
            .unwrap();
        server.await.unwrap();
    }

    #[tokio::test(flavor = "multi_thread", worker_threads = 4)]
    async fn shutdown_does_not_cancel_admitted_reconnect_authorization() {
        let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
        let address = listener.local_addr().unwrap();
        let server = tokio::spawn(async move {
            let (stream, _) = listener.accept().await.unwrap();
            let mut socket = accept_async(stream).await.unwrap();
            read_subscription(&mut socket).await;
            socket.close(None).await.unwrap();
        });

        let entered = Arc::new(Notify::new());
        let release = Arc::new(Notify::new());
        let role = PmPublicMarketWsRole::with_clock_source(
            standard_config(address, 1_024, 1, 74),
            TestClock { next: 1 },
        )
        .unwrap();
        let (shutdown, signal) = pm_public_ws_shutdown_channel();
        let sink_entered = Arc::clone(&entered);
        let sink_release = Arc::clone(&release);
        let mut task = tokio::spawn(async move {
            let mut sink = BlockingReconnectAuthoritySink {
                entered: sink_entered,
                release: sink_release,
            };
            role.run(signal, &mut sink).await
        });

        timeout(Duration::from_secs(5), entered.notified())
            .await
            .expect("reconnect authority barrier was never entered");
        shutdown.request_shutdown();
        assert!(timeout(Duration::from_millis(50), &mut task).await.is_err());
        release.notify_one();
        timeout(Duration::from_secs(5), task)
            .await
            .expect("public run did not finish after reconnect authority barrier")
            .unwrap()
            .unwrap();
        server.await.unwrap();
    }

    #[tokio::test(flavor = "multi_thread", worker_threads = 4)]
    async fn aborting_the_outer_public_run_cannot_detach_its_socket_worker() {
        let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
        let address = listener.local_addr().unwrap();
        let (closed, observed_close) = oneshot::channel();
        let server = tokio::spawn(async move {
            let (stream, _) = listener.accept().await.unwrap();
            let mut socket = accept_async(stream).await.unwrap();
            read_subscription(&mut socket).await;
            let closed_before_timeout = timeout(Duration::from_secs(5), async {
                loop {
                    match socket.next().await {
                        None | Some(Err(_)) | Some(Ok(Message::Close(_))) => break true,
                        Some(Ok(_)) => {}
                    }
                }
            })
            .await
            .unwrap_or(false);
            let _ = closed.send(closed_before_timeout);
        });

        let entered = Arc::new(Notify::new());
        let role = PmPublicMarketWsRole::with_clock_source(
            standard_config(address, 1_024, 0, 72),
            TestClock { next: 1 },
        )
        .unwrap();
        let (_shutdown, signal) = pm_public_ws_shutdown_channel();
        let sink_entered = Arc::clone(&entered);
        let outer = tokio::spawn(async move {
            let mut sink = BlockingOpenedSink {
                entered: sink_entered,
                release: Arc::new(Notify::new()),
            };
            role.run(signal, &mut sink).await
        });

        timeout(Duration::from_secs(5), entered.notified())
            .await
            .expect("opened evidence never reached blocked sink");
        outer.abort();
        assert!(outer.await.unwrap_err().is_cancelled());
        assert!(
            timeout(Duration::from_secs(6), observed_close)
                .await
                .expect("server never completed close observation")
                .expect("server close observer dropped"),
            "outer public-run cancellation detached the socket worker",
        );
        server.await.unwrap();
    }

    #[tokio::test]
    async fn sink_failure_aborts_the_transport_without_reinterpreting_it() {
        let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
        let address = listener.local_addr().unwrap();
        let server = tokio::spawn(async move {
            let (stream, _) = listener.accept().await.unwrap();
            let mut socket = accept_async(stream).await.unwrap();
            let _ = timeout(Duration::from_secs(1), socket.next()).await;
        });
        let (_shutdown, _events, _clocks, role) =
            spawn_role(standard_config(address, 1_024, 0, 1), Some(0));
        assert!(matches!(
            role.await.unwrap().unwrap_err(),
            PmPublicWsRunError::Sink("synthetic sink rejection")
        ));
        server.await.unwrap();
    }
}
