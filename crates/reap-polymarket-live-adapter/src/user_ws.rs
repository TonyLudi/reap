use std::fmt;
use std::sync::{
    Arc, OnceLock,
    atomic::{AtomicU64, Ordering},
};
use std::time::{Duration, Instant as StdInstant, SystemTime, UNIX_EPOCH};

use async_trait::async_trait;
use futures_util::{SinkExt, StreamExt};
use reap_pm_core::{ConnectionEpoch, PmConditionId, ReceivedEventClock};
use reap_polymarket_auth::{
    AuthenticatedUserSubscriptionSink, CredentialOwnedUserFrame, PmAuthError,
};
use reap_polymarket_wire::{PmLiveUserEvent, parse_live_user_frame};
use thiserror::Error;
use tokio::sync::{mpsc, watch};
use tokio::time::{Instant, sleep_until, timeout};
use tokio_tungstenite::tungstenite::{Error as WebSocketError, Message, protocol::WebSocketConfig};
use zeroize::Zeroizing;

use crate::{
    PmLiveAdapterError, PmUserWsConfig,
    read_authority::PmUserWsReadAuthorityProvider,
    task_guard::AbortOnDropTask,
    ws_transport::{
        PmDefaultWsDialer, PmFixedWsRoute, PmWsDialRequest, PmWsDialStrategy, PmWsSocket,
    },
};

const APPLICATION_PING: &str = "PING";
const APPLICATION_PONG: &str = "PONG";

struct PmRetainedUserSubscription(Zeroizing<Vec<u8>>);

impl PmRetainedUserSubscription {
    fn as_bytes(&self) -> &[u8] {
        self.0.as_slice()
    }
}

struct RetainSubscriptionSink;

impl AuthenticatedUserSubscriptionSink for RetainSubscriptionSink {
    type Output = PmRetainedUserSubscription;
    type Error = PmLiveAdapterError;

    fn send_user_subscription(&mut self, exact_frame: &[u8]) -> Result<Self::Output, Self::Error> {
        if exact_frame.is_empty() || exact_frame.len() > 1_024 {
            return Err(PmLiveAdapterError::InvalidUserSubscription);
        }
        Ok(PmRetainedUserSubscription(Zeroizing::new(
            exact_frame.to_vec(),
        )))
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Error)]
pub enum PmUserWsClockError {
    #[error("user WebSocket clock reading is invalid")]
    InvalidReading,
    #[error("user WebSocket system clock is unavailable")]
    SystemClockUnavailable,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct PmUserWsEdgeClock {
    received: ReceivedEventClock,
}

impl PmUserWsEdgeClock {
    pub fn new(
        local_wall_receive_ns: u64,
        monotonic_receive_ns: u64,
    ) -> Result<Self, PmUserWsClockError> {
        Ok(Self {
            received: ReceivedEventClock::new(None, local_wall_receive_ns, monotonic_receive_ns)
                .map_err(|_| PmUserWsClockError::InvalidReading)?,
        })
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

pub trait PmUserWsClockSource: Send + 'static {
    fn observe_user_ws_edge(&mut self) -> Result<PmUserWsEdgeClock, PmUserWsClockError>;
}

struct SystemUserWsClock;

impl PmUserWsClockSource for SystemUserWsClock {
    fn observe_user_ws_edge(&mut self) -> Result<PmUserWsEdgeClock, PmUserWsClockError> {
        static MONOTONIC_ORIGIN: OnceLock<StdInstant> = OnceLock::new();
        let wall = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .map_err(|_| PmUserWsClockError::SystemClockUnavailable)?
            .as_nanos()
            .try_into()
            .map_err(|_| PmUserWsClockError::SystemClockUnavailable)?;
        let monotonic = MONOTONIC_ORIGIN
            .get_or_init(StdInstant::now)
            .elapsed()
            .as_nanos()
            .saturating_add(1)
            .try_into()
            .map_err(|_| PmUserWsClockError::SystemClockUnavailable)?;
        PmUserWsEdgeClock::new(wall, monotonic)
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct PmUserWsConnection {
    condition: PmConditionId,
    connection_epoch: ConnectionEpoch,
}

/// Cloneable read-only high-water view of authenticated user-stream activity.
///
/// Every socket or lifecycle edge reserves a checked generation before any
/// parse, owner binding, or queue handoff. A caller holding an older value can
/// therefore detect queued, in-flight, rejected, or retirement activity even
/// when no corresponding event was successfully delivered yet.
#[derive(Clone)]
pub struct PmUserWsActivityView {
    generation: Arc<AtomicU64>,
}

impl PmUserWsActivityView {
    fn new() -> Self {
        Self {
            generation: Arc::new(AtomicU64::new(0)),
        }
    }

    fn advance(&self) -> Result<u64, PmUserWsTransportError> {
        self.generation
            .fetch_update(Ordering::AcqRel, Ordering::Acquire, |generation| {
                generation.checked_add(1)
            })
            .map(|previous| previous + 1)
            .map_err(|_| PmUserWsTransportError::ActivityGenerationOverflow)
    }

    #[must_use]
    pub fn generation(&self) -> u64 {
        self.generation.load(Ordering::Acquire)
    }

    #[must_use]
    pub fn high_water(&self) -> u64 {
        self.generation()
    }
}

impl fmt::Debug for PmUserWsActivityView {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("PmUserWsActivityView")
            .field("generation", &self.generation())
            .finish()
    }
}

impl PmUserWsConnection {
    #[must_use]
    pub const fn condition(self) -> PmConditionId {
        self.condition
    }

    #[must_use]
    pub const fn connection_epoch(self) -> ConnectionEpoch {
        self.connection_epoch
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct PmUserWsObservation {
    connection: PmUserWsConnection,
    clock: PmUserWsEdgeClock,
    activity_generation: u64,
}

impl PmUserWsObservation {
    #[must_use]
    pub const fn connection(self) -> PmUserWsConnection {
        self.connection
    }

    #[must_use]
    pub const fn clock(self) -> PmUserWsEdgeClock {
        self.clock
    }

    #[must_use]
    pub const fn activity_generation(self) -> u64 {
        self.activity_generation
    }
}

/// Parsed user evidence already bound to the sole credential authority.
pub struct PmUserWsBoundFrame {
    observation: PmUserWsObservation,
    frame: CredentialOwnedUserFrame,
}

impl PmUserWsBoundFrame {
    #[must_use]
    pub const fn observation(&self) -> PmUserWsObservation {
        self.observation
    }

    #[must_use]
    pub fn events(&self) -> &[PmLiveUserEvent] {
        self.frame.events()
    }

    /// Consume the transport envelope into the credential-bound frame needed
    /// by the sealed private-normalization ingress. No raw venue bytes or
    /// credential material are exposed.
    #[must_use]
    pub fn into_credential_owned_frame(self) -> CredentialOwnedUserFrame {
        self.frame
    }
}

impl fmt::Debug for PmUserWsBoundFrame {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("PmUserWsBoundFrame")
            .field("observation", &self.observation)
            .field("frame", &"[REDACTED]")
            .finish()
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Error)]
pub enum PmUserWsDisconnectReason {
    #[error("connection attempt timed out")]
    ConnectTimeout,
    #[error("connection attempt failed")]
    ConnectFailed,
    #[error("fresh authenticated subscription could not be produced")]
    SubscriptionAuthenticationFailed,
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
    #[error("binary authenticated frame is forbidden")]
    BinaryFrame,
    #[error("authenticated frame exceeded its configured bound")]
    FrameTooLarge,
    #[error("authenticated frame was malformed")]
    MalformedFrame,
    #[error("authenticated frame did not belong to the configured credential owner")]
    CredentialOwnerMismatch,
    #[error("private credential authority became unavailable")]
    CredentialAuthorityUnavailable,
    #[error("authenticated connection became idle")]
    IdleTimeout,
    #[error("application-level PONG was not received in time")]
    PongTimeout,
    #[error("unexpected raw WebSocket protocol frame")]
    UnexpectedProtocolFrame,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct PmUserWsRetirement {
    observation: PmUserWsObservation,
    reason: PmUserWsDisconnectReason,
}

impl PmUserWsRetirement {
    #[must_use]
    pub const fn observation(self) -> PmUserWsObservation {
        self.observation
    }

    #[must_use]
    pub const fn reason(self) -> PmUserWsDisconnectReason {
        self.reason
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct PmUserWsReconnect {
    retired: PmUserWsRetirement,
    replacement_epoch: ConnectionEpoch,
    reconnect_attempt: u8,
    backoff: Duration,
    scheduled_clock: PmUserWsEdgeClock,
    activity_generation: u64,
}

impl PmUserWsReconnect {
    #[must_use]
    pub const fn retired(self) -> PmUserWsRetirement {
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
    pub const fn scheduled_clock(self) -> PmUserWsEdgeClock {
        self.scheduled_clock
    }

    #[must_use]
    pub const fn activity_generation(self) -> u64 {
        self.activity_generation
    }
}

#[derive(Debug)]
pub enum PmUserWsEvent {
    ConnectionOpened(PmUserWsObservation),
    SubscriptionSent(PmUserWsObservation),
    PingSent(PmUserWsObservation),
    Pong(PmUserWsObservation),
    BoundFrame(PmUserWsBoundFrame),
    ConnectionRetired(PmUserWsRetirement),
    ReconnectScheduled(PmUserWsReconnect),
    RetryExhausted(PmUserWsRetirement),
    Shutdown(PmUserWsObservation),
}

impl PmUserWsEvent {
    #[must_use]
    pub const fn activity_generation(&self) -> u64 {
        match self {
            Self::ConnectionOpened(observation)
            | Self::SubscriptionSent(observation)
            | Self::PingSent(observation)
            | Self::Pong(observation)
            | Self::Shutdown(observation) => observation.activity_generation(),
            Self::BoundFrame(frame) => frame.observation().activity_generation(),
            Self::ConnectionRetired(retirement) | Self::RetryExhausted(retirement) => {
                retirement.observation().activity_generation()
            }
            Self::ReconnectScheduled(reconnect) => reconnect.activity_generation(),
        }
    }
}

#[async_trait]
pub trait PmUserWsEventSink: Send {
    type Error;

    async fn deliver_user_ws_event(&mut self, event: PmUserWsEvent) -> Result<(), Self::Error>;
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Error)]
pub enum PmUserWsTransportError {
    #[error(
        "user WebSocket exhausted {attempts} bounded connection attempts after: {final_reason}"
    )]
    RetryExhausted {
        attempts: u8,
        final_reason: PmUserWsDisconnectReason,
    },
    #[error("user WebSocket connection epoch overflowed")]
    ConnectionEpochOverflow,
    #[error("user WebSocket activity generation overflowed")]
    ActivityGenerationOverflow,
    #[error("user WebSocket evidence channel closed")]
    EventChannelClosed,
    #[error("user WebSocket evidence channel saturated")]
    EventChannelSaturated,
    #[error("user WebSocket worker failed")]
    WorkerFailed,
    #[error(transparent)]
    Clock(#[from] PmUserWsClockError),
}

#[derive(Debug, Error)]
pub enum PmUserWsRunError<E> {
    #[error(transparent)]
    Transport(#[from] PmUserWsTransportError),
    #[error("user WebSocket sink rejected typed evidence: {0}")]
    Sink(E),
}

#[derive(Debug)]
pub struct PmUserWsShutdownHandle {
    sender: watch::Sender<bool>,
}

impl PmUserWsShutdownHandle {
    pub fn request_shutdown(&self) {
        self.sender.send_replace(true);
    }
}

#[derive(Debug)]
pub struct PmUserWsShutdownSignal {
    receiver: watch::Receiver<bool>,
}

#[must_use]
pub fn pm_user_ws_shutdown_channel() -> (PmUserWsShutdownHandle, PmUserWsShutdownSignal) {
    let (sender, receiver) = watch::channel(false);
    (
        PmUserWsShutdownHandle { sender },
        PmUserWsShutdownSignal { receiver },
    )
}

/// Read-only authenticated user-stream role. Subscription authentication is
/// freshly minted by the sole credential authority on every connection.
pub struct PmAuthenticatedUserWsRole {
    config: PmUserWsConfig,
    credentials: Box<dyn PmUserWsReadAuthorityProvider>,
    clock: Box<dyn PmUserWsClockSource>,
    activity: PmUserWsActivityView,
}

impl PmAuthenticatedUserWsRole {
    pub(crate) fn from_authority(
        config: PmUserWsConfig,
        credentials: crate::private_credentials::PmUserWsCredentialRole,
    ) -> Self {
        Self::from_external_authority(config, Box::new(credentials))
    }

    pub(crate) fn from_external_authority(
        config: PmUserWsConfig,
        credentials: Box<dyn PmUserWsReadAuthorityProvider>,
    ) -> Self {
        Self {
            config,
            credentials,
            clock: Box::new(SystemUserWsClock),
            activity: PmUserWsActivityView::new(),
        }
    }

    /// Replace the convenience process clock with the composition's shared
    /// receive-edge clock origin before production use.
    #[must_use]
    pub fn with_clock_source<C>(mut self, clock: C) -> Self
    where
        C: PmUserWsClockSource,
    {
        self.clock = Box::new(clock);
        self
    }

    #[must_use]
    pub const fn condition(&self) -> PmConditionId {
        self.config.condition()
    }

    #[must_use]
    pub fn activity_view(&self) -> PmUserWsActivityView {
        self.activity.clone()
    }

    pub async fn run<S>(
        self,
        shutdown: PmUserWsShutdownSignal,
        sink: &mut S,
    ) -> Result<(), PmUserWsRunError<S::Error>>
    where
        S: PmUserWsEventSink,
    {
        let (sender, mut receiver) = mpsc::channel(self.config.event_channel_capacity());
        let worker = AbortOnDropTask::new(tokio::spawn(run_worker(
            self.config,
            self.credentials,
            self.clock,
            self.activity,
            shutdown.receiver,
            sender,
            PmDefaultWsDialer,
        )));
        serve_worker_events(worker, &mut receiver, sink).await
    }
}

async fn serve_worker_events<S>(
    mut worker: AbortOnDropTask<Result<(), PmUserWsTransportError>>,
    receiver: &mut mpsc::Receiver<PmUserWsEvent>,
    sink: &mut S,
) -> Result<(), PmUserWsRunError<S::Error>>
where
    S: PmUserWsEventSink,
{
    while let Some(event) = receiver.recv().await {
        if let Err(error) = sink.deliver_user_ws_event(event).await {
            let _ = worker.abort_and_join().await;
            return Err(PmUserWsRunError::Sink(error));
        }
    }
    worker
        .join()
        .await
        .map_err(|_| PmUserWsTransportError::WorkerFailed)??;
    Ok(())
}

impl fmt::Debug for PmAuthenticatedUserWsRole {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("PmAuthenticatedUserWsRole")
            .field("condition", &self.config.condition())
            .field("credentials", &"[REDACTED]")
            .finish()
    }
}

async fn run_worker<D>(
    config: PmUserWsConfig,
    mut credentials: Box<dyn PmUserWsReadAuthorityProvider>,
    mut clock: Box<dyn PmUserWsClockSource>,
    activity: PmUserWsActivityView,
    mut shutdown: watch::Receiver<bool>,
    events: mpsc::Sender<PmUserWsEvent>,
    mut dialer: D,
) -> Result<(), PmUserWsTransportError>
where
    D: PmWsDialStrategy,
{
    let mut epoch = config.initial_connection_epoch();
    let mut reconnects = 0_u8;
    loop {
        let connection = PmUserWsConnection {
            condition: config.condition(),
            connection_epoch: epoch,
        };
        let outcome = run_attempt(
            &config,
            credentials.as_mut(),
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
        let retired = match outcome {
            AttemptOutcome::Shutdown(observation) => {
                emit(&events, PmUserWsEvent::Shutdown(observation)).await?;
                return Ok(());
            }
            AttemptOutcome::Retired(retired) => retired,
        };
        emit(&events, PmUserWsEvent::ConnectionRetired(retired)).await?;
        if reconnects >= config.max_reconnect_attempts() {
            let exhausted = retirement(&activity, clock.as_mut(), connection, retired.reason)?;
            emit(&events, PmUserWsEvent::RetryExhausted(exhausted)).await?;
            return Err(PmUserWsTransportError::RetryExhausted {
                attempts: reconnects.saturating_add(1),
                final_reason: retired.reason,
            });
        }
        reconnects += 1;
        let replacement_epoch = ConnectionEpoch::new(
            epoch
                .value()
                .checked_add(1)
                .ok_or(PmUserWsTransportError::ConnectionEpochOverflow)?,
        );
        let reconnect_generation = activity.advance()?;
        emit(
            &events,
            PmUserWsEvent::ReconnectScheduled(PmUserWsReconnect {
                retired,
                replacement_epoch,
                reconnect_attempt: reconnects,
                backoff: config.reconnect_backoff(),
                scheduled_clock: observe(clock.as_mut())?,
                activity_generation: reconnect_generation,
            }),
        )
        .await?;
        let deadline = Instant::now() + config.reconnect_backoff();
        tokio::select! {
            () = wait_for_shutdown(&mut shutdown) => {
                let observation = reserve_observation(&activity, connection, clock.as_mut())?;
                emit(&events, PmUserWsEvent::Shutdown(observation)).await?;
                return Ok(());
            }
            () = sleep_until(deadline) => {}
        }
        epoch = replacement_epoch;
    }
}

enum AttemptOutcome {
    Shutdown(PmUserWsObservation),
    Retired(PmUserWsRetirement),
}

struct AttemptControl<'a> {
    shutdown: &'a mut watch::Receiver<bool>,
    events: &'a mpsc::Sender<PmUserWsEvent>,
}

async fn run_attempt<D>(
    config: &PmUserWsConfig,
    credentials: &mut dyn PmUserWsReadAuthorityProvider,
    connection: PmUserWsConnection,
    clock: &mut dyn PmUserWsClockSource,
    activity: &PmUserWsActivityView,
    control: AttemptControl<'_>,
    dialer: &mut D,
) -> Result<AttemptOutcome, PmUserWsTransportError>
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
        PmFixedWsRoute::AuthenticatedUser,
        config.endpoint().as_str(),
        websocket_config,
    ));
    let socket = tokio::select! {
        () = wait_for_shutdown(shutdown) => {
            return Ok(AttemptOutcome::Shutdown(reserve_observation(activity, connection, clock)?));
        }
        result = timeout(config.connect_timeout(), connect) => match result {
            Err(_) => return retired(activity, clock, connection, PmUserWsDisconnectReason::ConnectTimeout),
            Ok(Err(_)) => return retired(activity, clock, connection, PmUserWsDisconnectReason::ConnectFailed),
            Ok(Ok(socket)) => socket,
        },
    };
    let opened = reserve_observation(activity, connection, clock)?;
    emit(events, PmUserWsEvent::ConnectionOpened(opened)).await?;
    run_connected(
        ConnectedContext {
            config,
            credentials,
            connection,
            clock,
            activity,
            shutdown,
            events,
        },
        socket,
    )
    .await
}

struct ConnectedContext<'a> {
    config: &'a PmUserWsConfig,
    credentials: &'a mut dyn PmUserWsReadAuthorityProvider,
    connection: PmUserWsConnection,
    clock: &'a mut dyn PmUserWsClockSource,
    activity: &'a PmUserWsActivityView,
    shutdown: &'a mut watch::Receiver<bool>,
    events: &'a mpsc::Sender<PmUserWsEvent>,
}

async fn run_connected(
    context: ConnectedContext<'_>,
    mut socket: PmWsSocket,
) -> Result<AttemptOutcome, PmUserWsTransportError> {
    let ConnectedContext {
        config,
        credentials,
        connection,
        clock,
        activity,
        shutdown,
        events,
    } = context;
    let subscription = match credentials
        .authenticate_user_subscription(config.condition())
        .await
    {
        Ok(subscription) => subscription,
        Err(_) => {
            return retired(
                activity,
                clock,
                connection,
                PmUserWsDisconnectReason::SubscriptionAuthenticationFailed,
            );
        }
    };
    let subscription = match subscription.dispatch(&mut RetainSubscriptionSink) {
        Ok(subscription) => subscription,
        Err(_) => {
            return retired(
                activity,
                clock,
                connection,
                PmUserWsDisconnectReason::SubscriptionAuthenticationFailed,
            );
        }
    };
    let subscription_text = match std::str::from_utf8(subscription.as_bytes()) {
        Ok(text) => Zeroizing::new(text.to_owned()),
        Err(_) => {
            return retired(
                activity,
                clock,
                connection,
                PmUserWsDisconnectReason::SubscriptionAuthenticationFailed,
            );
        }
    };
    match timeout(
        config.connect_timeout(),
        socket.send(Message::text(subscription_text.as_str())),
    )
    .await
    {
        Err(_) => {
            return retired(
                activity,
                clock,
                connection,
                PmUserWsDisconnectReason::SubscriptionWriteTimeout,
            );
        }
        Ok(Err(_)) => {
            return retired(
                activity,
                clock,
                connection,
                PmUserWsDisconnectReason::SubscriptionWriteFailed,
            );
        }
        Ok(Ok(())) => {}
    }
    let subscribed = reserve_observation(activity, connection, clock)?;
    emit(events, PmUserWsEvent::SubscriptionSent(subscribed)).await?;

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
                return Ok(AttemptOutcome::Shutdown(reserve_observation(activity, connection, clock)?));
            }
            () = sleep_until(next_heartbeat), if outstanding_pong.is_none() => {
                match timeout(config.pong_timeout(), socket.send(Message::text(APPLICATION_PING))).await {
                    Err(_) => return retired(activity, clock, connection, PmUserWsDisconnectReason::SocketWriteTimeout),
                    Ok(Err(_)) => return retired(activity, clock, connection, PmUserWsDisconnectReason::SocketWriteFailed),
                    Ok(Ok(())) => {}
                }
                let sent = Instant::now();
                outstanding_pong = Some(sent + config.pong_timeout());
                next_heartbeat = sent + config.heartbeat_interval();
                let ping = reserve_observation(activity, connection, clock)?;
                emit(events, PmUserWsEvent::PingSent(ping)).await?;
            }
            () = sleep_until(pong_deadline), if outstanding_pong.is_some() => {
                return retired(activity, clock, connection, PmUserWsDisconnectReason::PongTimeout);
            }
            () = sleep_until(idle_deadline) => {
                return retired(activity, clock, connection, PmUserWsDisconnectReason::IdleTimeout);
            }
            message = socket.next() => {
                // The completed socket-read edge invalidates an older cut
                // even when it reports EOF or a transport/capacity error.
                let received_generation = activity.advance()?;
                let Some(message) = message else {
                    return retired(activity, clock, connection, PmUserWsDisconnectReason::SocketClosed);
                };
                let message = match message {
                    Ok(message) => message,
                    Err(error) => return retired(activity, clock, connection, classify_read_error(&error)),
                };
                // Reserve high-water and sample immediately after the socket
                // read, before parsing, credential binding, or queue service.
                // Failures never roll this generation back. Raw protocol
                // Ping/Pong frames intentionally consume an un-emitted
                // generation as a conservative invalidation interval.
                let received_clock = observe(clock)?;
                last_inbound = Instant::now();
                match message {
                    Message::Text(text) if text.as_str() == APPLICATION_PONG => {
                        if outstanding_pong.take().is_none() {
                            return retired(
                                activity,
                                clock,
                                connection,
                                PmUserWsDisconnectReason::UnexpectedProtocolFrame,
                            );
                        }
                        emit(events, PmUserWsEvent::Pong(observation_at(
                            connection,
                            received_clock,
                            received_generation,
                        ))).await?;
                    }
                    Message::Text(text) => {
                        if text.len() > config.max_frame_bytes() {
                            return retired(activity, clock, connection, PmUserWsDisconnectReason::FrameTooLarge);
                        }
                        let raw = Zeroizing::new(text.as_str().as_bytes().to_vec());
                        let frame = match parse_live_user_frame(raw.as_slice()) {
                            Ok(frame) => frame,
                            Err(_) => return retired(activity, clock, connection, PmUserWsDisconnectReason::MalformedFrame),
                        };
                        let frame = match credentials.bind_user_frame(frame).await {
                            Ok(frame) => frame,
                            Err(PmLiveAdapterError::Auth(
                                PmAuthError::UserOrderOwnerMismatch
                                | PmAuthError::UserOrderOrderOwnerMismatch
                                | PmAuthError::UserTradeOwnerMismatch
                                | PmAuthError::UserTradeTradeOwnerMismatch,
                            ) | PmLiveAdapterError::CredentialOwnerMismatch) => return retired(activity, clock, connection, PmUserWsDisconnectReason::CredentialOwnerMismatch),
                            Err(_) => return retired(activity, clock, connection, PmUserWsDisconnectReason::CredentialAuthorityUnavailable),
                        };
                        emit(events, PmUserWsEvent::BoundFrame(PmUserWsBoundFrame {
                            observation: observation_at(
                                connection,
                                received_clock,
                                received_generation,
                            ),
                            frame,
                        })).await?;
                    }
                    Message::Binary(_) => return retired(activity, clock, connection, PmUserWsDisconnectReason::BinaryFrame),
                    Message::Ping(_) | Message::Pong(_) => {
                        match timeout(config.pong_timeout(), socket.flush()).await {
                            Err(_) => return retired(activity, clock, connection, PmUserWsDisconnectReason::SocketWriteTimeout),
                            Ok(Err(_)) => return retired(activity, clock, connection, PmUserWsDisconnectReason::SocketWriteFailed),
                            Ok(Ok(())) => {}
                        }
                    }
                    Message::Close(_) => return retired(activity, clock, connection, PmUserWsDisconnectReason::SocketClosed),
                    Message::Frame(_) => return retired(activity, clock, connection, PmUserWsDisconnectReason::UnexpectedProtocolFrame),
                }
            }
        }
    }
}

fn reserve_observation(
    activity: &PmUserWsActivityView,
    connection: PmUserWsConnection,
    clock: &mut dyn PmUserWsClockSource,
) -> Result<PmUserWsObservation, PmUserWsTransportError> {
    let activity_generation = activity.advance()?;
    let clock = observe(clock)?;
    Ok(observation_at(connection, clock, activity_generation))
}

const fn observation_at(
    connection: PmUserWsConnection,
    clock: PmUserWsEdgeClock,
    activity_generation: u64,
) -> PmUserWsObservation {
    PmUserWsObservation {
        connection,
        clock,
        activity_generation,
    }
}

fn observe(
    clock: &mut dyn PmUserWsClockSource,
) -> Result<PmUserWsEdgeClock, PmUserWsTransportError> {
    clock
        .observe_user_ws_edge()
        .map_err(PmUserWsTransportError::Clock)
}

fn retired(
    activity: &PmUserWsActivityView,
    clock: &mut dyn PmUserWsClockSource,
    connection: PmUserWsConnection,
    reason: PmUserWsDisconnectReason,
) -> Result<AttemptOutcome, PmUserWsTransportError> {
    Ok(AttemptOutcome::Retired(retirement(
        activity, clock, connection, reason,
    )?))
}

fn retirement(
    activity: &PmUserWsActivityView,
    clock: &mut dyn PmUserWsClockSource,
    connection: PmUserWsConnection,
    reason: PmUserWsDisconnectReason,
) -> Result<PmUserWsRetirement, PmUserWsTransportError> {
    Ok(PmUserWsRetirement {
        observation: reserve_observation(activity, connection, clock)?,
        reason,
    })
}

fn classify_read_error(error: &WebSocketError) -> PmUserWsDisconnectReason {
    if matches!(error, WebSocketError::Capacity(_)) {
        PmUserWsDisconnectReason::FrameTooLarge
    } else {
        PmUserWsDisconnectReason::SocketReadFailed
    }
}

#[cfg(test)]
impl PmAuthenticatedUserWsRole {
    async fn run_with_test_selected_loopback<S>(
        self,
        shutdown: PmUserWsShutdownSignal,
        sink: &mut S,
        dialer: crate::ws_transport::PmTestSelectedLoopbackWsDialer,
    ) -> Result<(), PmUserWsRunError<S::Error>>
    where
        S: PmUserWsEventSink,
    {
        let (sender, mut receiver) = mpsc::channel(self.config.event_channel_capacity());
        let worker = AbortOnDropTask::new(tokio::task::spawn_local(run_worker(
            self.config,
            self.credentials,
            self.clock,
            self.activity,
            shutdown.receiver,
            sender,
            dialer,
        )));
        serve_worker_events(worker, &mut receiver, sink).await
    }
}

async fn emit(
    events: &mpsc::Sender<PmUserWsEvent>,
    event: PmUserWsEvent,
) -> Result<(), PmUserWsTransportError> {
    events.try_send(event).map_err(|error| match error {
        mpsc::error::TrySendError::Full(_) => PmUserWsTransportError::EventChannelSaturated,
        mpsc::error::TrySendError::Closed(_) => PmUserWsTransportError::EventChannelClosed,
    })
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
    use std::sync::{
        Arc, Mutex,
        atomic::{AtomicU64, Ordering},
    };

    use reap_pm_core::{PmConditionId, PmMarketId, PmTokenId, U256};
    use reap_polymarket_auth::{L2CredentialInput, L2Credentials};
    use reap_polymarket_wire::PmWireScope;
    use tokio::net::TcpListener;
    use tokio::sync::Notify;
    use tokio::task::{JoinHandle, LocalSet};
    use tokio_tungstenite::{WebSocketStream, accept_async, tungstenite::Message};

    use super::*;
    use crate::{
        PmPrivateConnectivityOwner, PmPrivateHttpConfig, PmUserWsConfig,
        ws_transport::PmTestSelectedLoopbackWsDialer,
    };

    const ADDRESS: &str = "0xf39Fd6e51aad88F6F4ce6aB8827279cffFb92266";
    const API_KEY: &str = "00000000-0000-4000-8000-000000000001";
    const FOREIGN_API_KEY: &str = "00000000-0000-4000-8000-000000000002";
    const API_SECRET: &str = "AAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAA=";
    const PASSPHRASE: &str = "synthetic-passphrase";
    const CONDITION: &str = "0x1111111111111111111111111111111111111111111111111111111111111111";
    const QUESTION: &str = "0x2222222222222222222222222222222222222222222222222222222222222222";
    const ORDER: &str = "0xaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa";
    const EXPECTED_SUBSCRIPTION: &str = r#"{"auth":{"apiKey":"00000000-0000-4000-8000-000000000001","secret":"AAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAA=","passphrase":"synthetic-passphrase"},"markets":["0x1111111111111111111111111111111111111111111111111111111111111111"],"type":"user"}"#;

    struct TestClock(u64);

    impl PmUserWsClockSource for TestClock {
        fn observe_user_ws_edge(&mut self) -> Result<PmUserWsEdgeClock, PmUserWsClockError> {
            let value = self.0;
            self.0 += 1;
            PmUserWsEdgeClock::new(1_000_000 + value, value)
        }
    }

    struct QueueClock {
        next: Arc<AtomicU64>,
        second_raw_sampled: Arc<Notify>,
    }

    impl PmUserWsClockSource for QueueClock {
        fn observe_user_ws_edge(&mut self) -> Result<PmUserWsEdgeClock, PmUserWsClockError> {
            let value = self.next.fetch_add(1, Ordering::SeqCst);
            if value == 4 {
                self.second_raw_sampled.notify_one();
            }
            PmUserWsEdgeClock::new(2_000_000 + value, value)
        }
    }

    struct BlockingBoundSink {
        entered: Arc<Notify>,
        release: Arc<Notify>,
        blocked: bool,
    }

    struct BlockingOpenedSink {
        entered: Arc<Notify>,
        release: Arc<Notify>,
    }

    #[async_trait]
    impl PmUserWsEventSink for BlockingOpenedSink {
        type Error = &'static str;

        async fn deliver_user_ws_event(&mut self, event: PmUserWsEvent) -> Result<(), Self::Error> {
            if matches!(event, PmUserWsEvent::ConnectionOpened(_)) {
                self.entered.notify_one();
                self.release.notified().await;
            }
            Ok(())
        }
    }

    #[async_trait]
    impl PmUserWsEventSink for BlockingBoundSink {
        type Error = &'static str;

        async fn deliver_user_ws_event(&mut self, event: PmUserWsEvent) -> Result<(), Self::Error> {
            if matches!(event, PmUserWsEvent::BoundFrame(_)) && !self.blocked {
                self.blocked = true;
                self.entered.notify_one();
                self.release.notified().await;
            }
            Ok(())
        }
    }

    #[derive(Debug, PartialEq, Eq)]
    enum Seen {
        Open(u64),
        Subscription(u64),
        Ping(u64),
        Pong(u64),
        Bound(u64, usize, u64),
        Retired(u64, PmUserWsDisconnectReason),
        Reconnect(u64, u64, u8),
        Exhausted(u64, PmUserWsDisconnectReason),
        Shutdown(u64),
    }

    struct TestSink {
        sender: mpsc::UnboundedSender<Seen>,
        rendered: Arc<Mutex<Vec<String>>>,
        generations: Arc<Mutex<Vec<u64>>>,
    }

    #[async_trait]
    impl PmUserWsEventSink for TestSink {
        type Error = &'static str;

        async fn deliver_user_ws_event(&mut self, event: PmUserWsEvent) -> Result<(), Self::Error> {
            self.rendered.lock().unwrap().push(format!("{event:?}"));
            self.generations
                .lock()
                .unwrap()
                .push(event.activity_generation());
            let seen = match event {
                PmUserWsEvent::ConnectionOpened(value) => {
                    Seen::Open(value.connection().connection_epoch().value())
                }
                PmUserWsEvent::SubscriptionSent(value) => {
                    Seen::Subscription(value.connection().connection_epoch().value())
                }
                PmUserWsEvent::PingSent(value) => {
                    Seen::Ping(value.connection().connection_epoch().value())
                }
                PmUserWsEvent::Pong(value) => {
                    Seen::Pong(value.connection().connection_epoch().value())
                }
                PmUserWsEvent::BoundFrame(value) => {
                    let observation = value.observation();
                    let frame = value.into_credential_owned_frame();
                    Seen::Bound(
                        observation.connection().connection_epoch().value(),
                        frame.events().len(),
                        observation.clock().monotonic_receive_ns(),
                    )
                }
                PmUserWsEvent::ConnectionRetired(value) => Seen::Retired(
                    value.observation().connection().connection_epoch().value(),
                    value.reason(),
                ),
                PmUserWsEvent::ReconnectScheduled(value) => Seen::Reconnect(
                    value
                        .retired()
                        .observation()
                        .connection()
                        .connection_epoch()
                        .value(),
                    value.replacement_epoch().value(),
                    value.reconnect_attempt(),
                ),
                PmUserWsEvent::RetryExhausted(value) => Seen::Exhausted(
                    value.observation().connection().connection_epoch().value(),
                    value.reason(),
                ),
                PmUserWsEvent::Shutdown(value) => {
                    Seen::Shutdown(value.connection().connection_epoch().value())
                }
            };
            self.sender.send(seen).map_err(|_| "receiver closed")
        }
    }

    fn scope() -> PmWireScope {
        PmWireScope::new(
            PmConditionId::parse(CONDITION).unwrap(),
            PmMarketId::parse(QUESTION).unwrap(),
            PmTokenId::new(U256::from_u64(123)).unwrap(),
        )
    }

    fn credentials() -> L2Credentials {
        L2Credentials::bind(
            ADDRESS,
            L2CredentialInput::new(API_KEY.into(), API_SECRET.into(), PASSPHRASE.into()),
        )
        .unwrap()
    }

    fn local_user_config(address: std::net::SocketAddr, retries: u8) -> PmUserWsConfig {
        local_user_config_with_frame_bound(address, retries, 4 * 1_024)
    }

    fn local_user_config_with_frame_bound(
        address: std::net::SocketAddr,
        retries: u8,
        max_frame_bytes: usize,
    ) -> PmUserWsConfig {
        local_user_config_with_capacity(address, retries, max_frame_bytes, 8)
    }

    fn local_user_config_with_capacity(
        address: std::net::SocketAddr,
        retries: u8,
        max_frame_bytes: usize,
        event_channel_capacity: usize,
    ) -> PmUserWsConfig {
        PmUserWsConfig::loopback_evidence(
            &format!("ws://{address}/ws/user"),
            scope().condition(),
            Duration::from_millis(200),
            Duration::from_millis(500),
            Duration::from_millis(50),
            Duration::from_millis(20),
            max_frame_bytes,
            retries,
            Duration::from_millis(2),
            event_channel_capacity,
            ConnectionEpoch::new(5),
        )
        .unwrap()
    }

    fn role(
        config: PmUserWsConfig,
    ) -> (
        PmAuthenticatedUserWsRole,
        crate::PmCredentialAuthoritySupervisor,
    ) {
        let http = PmPrivateHttpConfig::local_evidence(
            "http://127.0.0.1:1",
            Duration::from_millis(100),
            Duration::from_millis(200),
            scope(),
        )
        .unwrap();
        let roles = PmPrivateConnectivityOwner::new(http, config, credentials())
            .unwrap()
            .split()
            .unwrap();
        let (_http, user, supervisor) = roles.into_read_roles();
        (user.with_clock_source(TestClock(1)), supervisor)
    }

    type RoleTask = JoinHandle<Result<(), PmUserWsRunError<&'static str>>>;
    type SpawnedRole = (
        PmUserWsShutdownHandle,
        mpsc::UnboundedReceiver<Seen>,
        Arc<Mutex<Vec<String>>>,
        Arc<Mutex<Vec<u64>>>,
        PmUserWsActivityView,
        RoleTask,
    );

    fn spawn_role(config: PmUserWsConfig) -> SpawnedRole {
        let (role, supervisor) = role(config);
        let activity = role.activity_view();
        let (shutdown, signal) = pm_user_ws_shutdown_channel();
        let (sender, receiver) = mpsc::unbounded_channel();
        let rendered = Arc::new(Mutex::new(Vec::new()));
        let sink_rendered = Arc::clone(&rendered);
        let generations = Arc::new(Mutex::new(Vec::new()));
        let sink_generations = Arc::clone(&generations);
        let task = tokio::spawn(async move {
            let mut sink = TestSink {
                sender,
                rendered: sink_rendered,
                generations: sink_generations,
            };
            let result = role.run(signal, &mut sink).await;
            supervisor.shutdown().await.unwrap();
            result
        });
        (shutdown, receiver, rendered, generations, activity, task)
    }

    async fn next(receiver: &mut mpsc::UnboundedReceiver<Seen>) -> Seen {
        timeout(Duration::from_secs(2), receiver.recv())
            .await
            .expect("event timeout")
            .expect("event channel closed")
    }

    fn order_frame(owner: &str) -> String {
        format!(
            r#"{{"event_type":"order","id":"{ORDER}","owner":"{owner}","market":"{CONDITION}","asset_id":"123","side":"BUY","original_size":"10","size_matched":"0","price":"0.42","type":"PLACEMENT","status":"LIVE","timestamp":"1782753357257"}}"#
        )
    }

    async fn read_exact_subscription<S>(socket: &mut WebSocketStream<S>)
    where
        S: tokio::io::AsyncRead + tokio::io::AsyncWrite + Unpin,
    {
        let message = timeout(Duration::from_secs(1), socket.next())
            .await
            .unwrap()
            .unwrap()
            .unwrap();
        assert_eq!(message, Message::text(EXPECTED_SUBSCRIPTION));
        let text = message.into_text().unwrap();
        assert!(!text.contains("initial_dump"));
        assert!(!text.contains("operation"));
    }

    #[tokio::test(flavor = "current_thread")]
    async fn selected_loopback_dialer_preserves_user_worker_protocol_on_local_set() {
        LocalSet::new()
            .run_until(async {
                let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
                let address = listener.local_addr().unwrap();
                let server = tokio::task::spawn_local(async move {
                    let (stream, _) = listener.accept().await.unwrap();
                    let mut socket = accept_async(stream).await.unwrap();
                    read_exact_subscription(&mut socket).await;
                    while let Some(message) = socket.next().await {
                        match message {
                            Ok(Message::Close(_)) | Err(_) => break,
                            Ok(_) => {}
                        }
                    }
                });

                let endpoint = format!("ws://{address}/ws/user");
                let (role, supervisor) = role(local_user_config(address, 0));
                let dialer = PmTestSelectedLoopbackWsDialer::new(
                    PmFixedWsRoute::AuthenticatedUser,
                    &endpoint,
                    address,
                    address.ip(),
                )
                .unwrap();
                let (shutdown, signal) = pm_user_ws_shutdown_channel();
                let (sender, mut receiver) = mpsc::unbounded_channel();
                let rendered = Arc::new(Mutex::new(Vec::new()));
                let sink_rendered = Arc::clone(&rendered);
                let generations = Arc::new(Mutex::new(Vec::new()));
                let sink_generations = Arc::clone(&generations);
                let task = tokio::task::spawn_local(async move {
                    let mut sink = TestSink {
                        sender,
                        rendered: sink_rendered,
                        generations: sink_generations,
                    };
                    role.run_with_test_selected_loopback(signal, &mut sink, dialer)
                        .await
                });

                assert_eq!(next(&mut receiver).await, Seen::Open(5));
                assert_eq!(next(&mut receiver).await, Seen::Subscription(5));
                shutdown.request_shutdown();
                assert_eq!(next(&mut receiver).await, Seen::Shutdown(5));
                task.await.unwrap().unwrap();
                supervisor.shutdown().await.unwrap();
                server.await.unwrap();
                assert_eq!(generations.lock().unwrap().as_slice(), [1, 2, 3]);
                assert!(
                    rendered
                        .lock()
                        .unwrap()
                        .iter()
                        .all(|event| !event.contains(API_SECRET)),
                );
            })
            .await;
    }

    #[tokio::test]
    async fn exact_fixed_subscription_bound_frame_ping_pong_and_shutdown() {
        let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
        let address = listener.local_addr().unwrap();
        let server = tokio::spawn(async move {
            let (stream, _) = listener.accept().await.unwrap();
            let mut socket = accept_async(stream).await.unwrap();
            read_exact_subscription(&mut socket).await;
            socket
                .send(Message::text(order_frame(API_KEY)))
                .await
                .unwrap();
            assert_eq!(
                socket.next().await.unwrap().unwrap(),
                Message::text(APPLICATION_PING)
            );
            socket.send(Message::text(APPLICATION_PONG)).await.unwrap();
            let _ = socket.next().await;
        });
        let (shutdown, mut events, rendered, generations, activity, task) =
            spawn_role(local_user_config(address, 0));
        assert_eq!(next(&mut events).await, Seen::Open(5));
        assert_eq!(next(&mut events).await, Seen::Subscription(5));
        assert_eq!(next(&mut events).await, Seen::Bound(5, 1, 3));
        assert_eq!(next(&mut events).await, Seen::Ping(5));
        assert_eq!(next(&mut events).await, Seen::Pong(5));
        shutdown.request_shutdown();
        assert_eq!(next(&mut events).await, Seen::Shutdown(5));
        task.await.unwrap().unwrap();
        server.await.unwrap();
        {
            let generations = generations.lock().unwrap();
            assert!(
                generations.windows(2).all(|pair| pair[0] < pair[1]),
                "every emitted user event must carry a strictly newer generation",
            );
            assert_eq!(activity.high_water(), *generations.last().unwrap());
        }
        for debug in rendered.lock().unwrap().iter() {
            assert!(!debug.contains(API_KEY));
            assert!(!debug.contains(API_SECRET));
            assert!(!debug.contains(PASSPHRASE));
        }
    }

    #[tokio::test]
    async fn foreign_owner_retires_without_emitting_a_bound_or_raw_frame() {
        let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
        let address = listener.local_addr().unwrap();
        let server = tokio::spawn(async move {
            let (stream, _) = listener.accept().await.unwrap();
            let mut socket = accept_async(stream).await.unwrap();
            read_exact_subscription(&mut socket).await;
            socket
                .send(Message::text(order_frame(FOREIGN_API_KEY)))
                .await
                .unwrap();
        });
        let (_shutdown, mut events, rendered, generations, activity, task) =
            spawn_role(local_user_config(address, 0));
        let error = task.await.unwrap().unwrap_err();
        assert!(matches!(
            error,
            PmUserWsRunError::Transport(PmUserWsTransportError::RetryExhausted {
                final_reason: PmUserWsDisconnectReason::CredentialOwnerMismatch,
                ..
            })
        ));
        let mut seen = Vec::new();
        while let Some(event) = events.recv().await {
            seen.push(event);
        }
        assert!(seen.contains(&Seen::Retired(
            5,
            PmUserWsDisconnectReason::CredentialOwnerMismatch
        )));
        assert!(!seen.iter().any(|event| matches!(event, Seen::Bound(..))));
        {
            let generations = generations.lock().unwrap();
            assert!(
                generations.windows(2).any(|pair| pair[1] > pair[0] + 1),
                "the rejected frame edge must invalidate the prior delivered cut",
            );
            assert_eq!(activity.high_water(), *generations.last().unwrap());
        }
        for debug in rendered.lock().unwrap().iter() {
            assert!(!debug.contains(FOREIGN_API_KEY));
        }
        server.await.unwrap();
    }

    #[tokio::test]
    async fn reconnect_reauthenticates_and_replaces_the_epoch() {
        let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
        let address = listener.local_addr().unwrap();
        let server = tokio::spawn(async move {
            let (first, _) = listener.accept().await.unwrap();
            let mut first = accept_async(first).await.unwrap();
            read_exact_subscription(&mut first).await;
            first.close(None).await.unwrap();
            let (second, _) = listener.accept().await.unwrap();
            let mut second = accept_async(second).await.unwrap();
            read_exact_subscription(&mut second).await;
            second
                .send(Message::text(order_frame(API_KEY)))
                .await
                .unwrap();
            let _ = second.next().await;
        });
        let (shutdown, mut events, _rendered, _generations, _activity, task) =
            spawn_role(local_user_config(address, 1));
        let mut seen = Vec::new();
        loop {
            let event = next(&mut events).await;
            let done = matches!(event, Seen::Bound(6, 1, _));
            seen.push(event);
            if done {
                break;
            }
        }
        assert!(seen.contains(&Seen::Reconnect(5, 6, 1)));
        shutdown.request_shutdown();
        assert_eq!(next(&mut events).await, Seen::Shutdown(6));
        task.await.unwrap().unwrap();
        server.await.unwrap();
    }

    async fn assert_forbidden_frame(
        message: Message,
        max_frame_bytes: usize,
        expected: PmUserWsDisconnectReason,
    ) {
        let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
        let address = listener.local_addr().unwrap();
        let server = tokio::spawn(async move {
            let (stream, _) = listener.accept().await.unwrap();
            let mut socket = accept_async(stream).await.unwrap();
            read_exact_subscription(&mut socket).await;
            socket.send(message).await.unwrap();
        });
        let (_shutdown, mut events, _rendered, generations, activity, task) = spawn_role(
            local_user_config_with_frame_bound(address, 0, max_frame_bytes),
        );
        let error = task.await.unwrap().unwrap_err();
        assert!(matches!(
            error,
            PmUserWsRunError::Transport(PmUserWsTransportError::RetryExhausted {
                final_reason,
                ..
            }) if final_reason == expected
        ));
        let mut saw_bound = false;
        while let Some(event) = events.recv().await {
            saw_bound |= matches!(event, Seen::Bound(..));
        }
        assert!(!saw_bound);
        {
            let generations = generations.lock().unwrap();
            assert!(
                generations.windows(2).any(|pair| pair[1] > pair[0] + 1),
                "a rejected socket frame must reserve an un-emitted generation",
            );
            assert_eq!(activity.high_water(), *generations.last().unwrap());
        }
        server.await.unwrap();
    }

    #[tokio::test]
    async fn malformed_binary_and_oversized_private_frames_retire_fail_closed() {
        assert_forbidden_frame(
            Message::text(APPLICATION_PONG),
            1_024,
            PmUserWsDisconnectReason::UnexpectedProtocolFrame,
        )
        .await;
        assert_forbidden_frame(
            Message::text("{not-json"),
            1_024,
            PmUserWsDisconnectReason::MalformedFrame,
        )
        .await;
        assert_forbidden_frame(
            Message::binary(vec![1_u8, 2, 3]),
            1_024,
            PmUserWsDisconnectReason::BinaryFrame,
        )
        .await;
        assert_forbidden_frame(
            Message::text("x".repeat(65)),
            64,
            PmUserWsDisconnectReason::FrameTooLarge,
        )
        .await;
    }

    #[tokio::test]
    async fn raw_protocol_ping_creates_an_intentional_unemitted_invalidation_interval() {
        let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
        let address = listener.local_addr().unwrap();
        let server = tokio::spawn(async move {
            let (stream, _) = listener.accept().await.unwrap();
            let mut socket = accept_async(stream).await.unwrap();
            read_exact_subscription(&mut socket).await;
            socket
                .send(Message::Ping(b"edge".to_vec().into()))
                .await
                .unwrap();
            socket.close(None).await.unwrap();
        });
        let (_shutdown, mut events, _rendered, generations, activity, task) =
            spawn_role(local_user_config(address, 0));
        let error = task.await.unwrap().unwrap_err();
        assert!(matches!(
            error,
            PmUserWsRunError::Transport(PmUserWsTransportError::RetryExhausted {
                final_reason: PmUserWsDisconnectReason::SocketClosed,
                ..
            })
        ));
        while events.recv().await.is_some() {}
        {
            let generations = generations.lock().unwrap();
            assert!(
                generations.windows(2).any(|pair| pair[1] > pair[0] + 1),
                "the raw control-frame edge must invalidate an older admitted cut",
            );
            assert_eq!(activity.generation(), *generations.last().unwrap());
        }
        server.await.unwrap();
    }

    #[tokio::test(flavor = "multi_thread", worker_threads = 4)]
    async fn second_private_receive_is_stamped_before_first_sink_service_releases() {
        let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
        let address = listener.local_addr().unwrap();
        let server = tokio::spawn(async move {
            let (stream, _) = listener.accept().await.unwrap();
            let mut socket = accept_async(stream).await.unwrap();
            read_exact_subscription(&mut socket).await;
            socket
                .feed(Message::text(order_frame(API_KEY)))
                .await
                .unwrap();
            socket
                .feed(Message::text(order_frame(API_KEY)))
                .await
                .unwrap();
            socket.flush().await.unwrap();
            let _ = socket.next().await;
        });

        let entered = Arc::new(Notify::new());
        let release = Arc::new(Notify::new());
        let second_raw_sampled = Arc::new(Notify::new());
        let next_clock = Arc::new(AtomicU64::new(1));
        let (role, supervisor) = role(local_user_config(address, 0));
        let activity = role.activity_view();
        let role = role.with_clock_source(QueueClock {
            next: Arc::clone(&next_clock),
            second_raw_sampled: Arc::clone(&second_raw_sampled),
        });
        let (shutdown, signal) = pm_user_ws_shutdown_channel();
        let sink_entered = Arc::clone(&entered);
        let sink_release = Arc::clone(&release);
        let task = tokio::spawn(async move {
            let mut sink = BlockingBoundSink {
                entered: sink_entered,
                release: sink_release,
                blocked: false,
            };
            let result = role.run(signal, &mut sink).await;
            supervisor.shutdown().await.unwrap();
            result
        });

        timeout(Duration::from_secs(5), entered.notified())
            .await
            .expect("first bound frame never reached sink");
        timeout(Duration::from_secs(5), second_raw_sampled.notified())
            .await
            .expect("second frame was not stamped while sink was blocked");
        assert!(next_clock.load(Ordering::SeqCst) >= 5);
        assert!(
            activity.high_water() >= 4,
            "queued second frame must invalidate a cut held at the first sink barrier",
        );
        release.notify_one();
        shutdown.request_shutdown();
        timeout(Duration::from_secs(5), task)
            .await
            .expect("user role did not stop")
            .unwrap()
            .unwrap();
        timeout(Duration::from_secs(5), server)
            .await
            .expect("server did not stop")
            .unwrap();
    }

    #[tokio::test(flavor = "multi_thread", worker_threads = 4)]
    async fn saturated_private_evidence_queue_fails_closed_without_trapping_shutdown() {
        let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
        let address = listener.local_addr().unwrap();
        let socket_closed = Arc::new(Notify::new());
        let server_closed = Arc::clone(&socket_closed);
        let server = tokio::spawn(async move {
            let (stream, _) = listener.accept().await.unwrap();
            let mut socket = accept_async(stream).await.unwrap();
            read_exact_subscription(&mut socket).await;
            let _ = socket.send(Message::text(order_frame(API_KEY))).await;
            let _ = timeout(Duration::from_secs(5), socket.next()).await;
            server_closed.notify_one();
        });

        let entered = Arc::new(Notify::new());
        let release = Arc::new(Notify::new());
        let (role, supervisor) = role(local_user_config_with_capacity(address, 0, 1_024, 1));
        let activity = role.activity_view();
        let (_shutdown, signal) = pm_user_ws_shutdown_channel();
        let sink_entered = Arc::clone(&entered);
        let sink_release = Arc::clone(&release);
        let task = tokio::spawn(async move {
            let mut sink = BlockingOpenedSink {
                entered: sink_entered,
                release: sink_release,
            };
            let result = role.run(signal, &mut sink).await;
            supervisor.shutdown().await.unwrap();
            result
        });

        timeout(Duration::from_secs(5), entered.notified())
            .await
            .expect("opened evidence never reached sink");
        timeout(Duration::from_secs(5), socket_closed.notified())
            .await
            .expect("saturated worker did not close its socket");
        assert!(
            activity.high_water() >= 2,
            "failed queue handoff must leave the activity high-water advanced",
        );
        release.notify_one();
        assert!(matches!(
            timeout(Duration::from_secs(5), task)
                .await
                .expect("user role remained trapped behind blocked sink")
                .unwrap(),
            Err(PmUserWsRunError::Transport(
                PmUserWsTransportError::EventChannelSaturated
            ))
        ));
        server.await.unwrap();
    }

    #[test]
    fn activity_generation_fails_closed_at_u64_max_without_wrapping() {
        let activity = PmUserWsActivityView {
            generation: Arc::new(AtomicU64::new(u64::MAX)),
        };
        assert_eq!(
            activity.advance(),
            Err(PmUserWsTransportError::ActivityGenerationOverflow),
        );
        assert_eq!(activity.high_water(), u64::MAX);
    }

    #[tokio::test(flavor = "multi_thread", worker_threads = 4)]
    async fn shutdown_waits_for_an_admitted_private_sink_delivery() {
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
        let (role, supervisor) = role(local_user_config(address, 0));
        let (shutdown, signal) = pm_user_ws_shutdown_channel();
        let sink_entered = Arc::clone(&entered);
        let sink_release = Arc::clone(&release);
        let mut task = tokio::spawn(async move {
            let mut sink = BlockingOpenedSink {
                entered: sink_entered,
                release: sink_release,
            };
            let result = role.run(signal, &mut sink).await;
            supervisor.shutdown().await.unwrap();
            result
        });

        timeout(Duration::from_secs(5), entered.notified())
            .await
            .expect("opened evidence never reached blocked sink");
        shutdown.request_shutdown();
        assert!(timeout(Duration::from_millis(50), &mut task).await.is_err());
        release.notify_one();
        timeout(Duration::from_secs(5), task)
            .await
            .expect("user run did not finish after admitted sink completed")
            .unwrap()
            .unwrap();
        timeout(Duration::from_secs(6), server)
            .await
            .expect("server did not observe private socket teardown")
            .unwrap();
    }

    #[tokio::test(flavor = "multi_thread", worker_threads = 4)]
    async fn shutdown_does_not_cancel_an_admitted_bound_frame_apply() {
        let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
        let address = listener.local_addr().unwrap();
        let server = tokio::spawn(async move {
            let (stream, _) = listener.accept().await.unwrap();
            let mut socket = accept_async(stream).await.unwrap();
            read_exact_subscription(&mut socket).await;
            socket
                .send(Message::text(order_frame(API_KEY)))
                .await
                .unwrap();
            let _ = timeout(Duration::from_secs(5), socket.next()).await;
        });

        let entered = Arc::new(Notify::new());
        let release = Arc::new(Notify::new());
        let (role, supervisor) = role(local_user_config(address, 0));
        let (shutdown, signal) = pm_user_ws_shutdown_channel();
        let sink_entered = Arc::clone(&entered);
        let sink_release = Arc::clone(&release);
        let mut task = tokio::spawn(async move {
            let mut sink = BlockingBoundSink {
                entered: sink_entered,
                release: sink_release,
                blocked: false,
            };
            role.run(signal, &mut sink).await
        });

        timeout(Duration::from_secs(5), entered.notified())
            .await
            .expect("bound frame never entered canonical apply barrier");
        shutdown.request_shutdown();
        assert!(timeout(Duration::from_millis(50), &mut task).await.is_err());
        release.notify_one();
        timeout(Duration::from_secs(5), task)
            .await
            .expect("user run did not finish after bound-frame apply barrier")
            .unwrap()
            .unwrap();
        supervisor.shutdown().await.unwrap();
        server.await.unwrap();
    }

    #[tokio::test(flavor = "multi_thread", worker_threads = 4)]
    async fn aborting_the_outer_user_run_cannot_detach_socket_or_credential_work() {
        let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
        let address = listener.local_addr().unwrap();
        let (closed, observed_close) = tokio::sync::oneshot::channel();
        let server = tokio::spawn(async move {
            let (stream, _) = listener.accept().await.unwrap();
            let mut socket = accept_async(stream).await.unwrap();
            // Connection-open evidence is intentionally delivered before the
            // credentialed subscription write. Aborting the outer task at
            // that barrier may therefore close before or after subscription;
            // either ordering must still tear down the socket worker.
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
        let (role, supervisor) = role(local_user_config(address, 0));
        let (_shutdown, signal) = pm_user_ws_shutdown_channel();
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
            "outer user-run cancellation detached the credentialed socket worker",
        );
        supervisor.shutdown().await.unwrap();
        server.await.unwrap();
    }
}
