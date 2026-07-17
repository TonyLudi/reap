use std::collections::{HashMap, HashSet};
use std::sync::Arc;
use std::sync::atomic::{AtomicU64, Ordering};
use std::time::{Duration, Instant, SystemTime, UNIX_EPOCH};

use futures_util::{SinkExt, StreamExt};
use reap_feed::{ConnectionAttemptPacer, OKX_MIN_CONNECTION_ATTEMPT_INTERVAL_MS, ReconnectPolicy};
use reap_order::{
    CancelOrderTransportError, OkxOrderGateway, OrderTransportError, PreparedRegularCancel,
    PreparedRegularSubmit,
};
use reap_venue::okx::{
    OkxOrderAck, OkxWsOrderOperation, OkxWsOrderResult, okx_capability_registration,
    parse_okx_ws_order_response,
};
use serde_json::Value;
use tokio::sync::{mpsc, oneshot, watch};
use tokio::task::{JoinError, JoinHandle};
use tokio_tungstenite::tungstenite::Message;

use crate::{
    AdapterError, BoundRegularOrderGateway, OrderCommandTransportSlot, RegularOrderSessionFactory,
    is_loopback_host, validate_private_websocket_url,
};

const LOGIN_TIMEOUT: Duration = Duration::from_secs(10);
const CONTROL_WRITE_TIMEOUT: Duration = Duration::from_secs(5);
const PING_INTERVAL: Duration = Duration::from_secs(15);
const IDLE_TIMEOUT: Duration = Duration::from_secs(30);
const EXPIRY_SCAN_INTERVAL: Duration = Duration::from_millis(10);
const ORDER_COMMAND_SESSION_COUNT: usize = 1;
const ORDER_COMMAND_SESSION_INDEX: usize = 0;
const STATUS_CHANNEL_CAPACITY: usize = 64;

pub struct OrderCommandWebsocketConfig {
    account_id: String,
    websocket_url: String,
    command_capacity: usize,
    request_expiry: Duration,
    acknowledgement_timeout: Duration,
    connection_attempt_pacer: ConnectionAttemptPacer,
    reconnect: ReconnectPolicy,
}

impl OrderCommandWebsocketConfig {
    #[allow(clippy::too_many_arguments)]
    pub fn new(
        account_id: impl Into<String>,
        websocket_url: impl Into<String>,
        command_capacity: usize,
        request_expiry: Duration,
        acknowledgement_timeout: Duration,
        connection_attempt_pacer: ConnectionAttemptPacer,
        reconnect: ReconnectPolicy,
    ) -> Result<Self, AdapterError> {
        let account_id = account_id.into();
        if account_id.trim().is_empty() || account_id.trim() != account_id {
            return Err(AdapterError::InvalidConfiguration(
                "order command websocket account id must be non-empty and trimmed".to_string(),
            ));
        }
        let websocket_url = websocket_url.into();
        if websocket_url.trim().is_empty() || websocket_url.trim() != websocket_url {
            return Err(AdapterError::InvalidConfiguration(
                "order command websocket URL must be non-empty and trimmed".to_string(),
            ));
        }
        if command_capacity == 0 {
            return Err(AdapterError::InvalidConfiguration(
                "order command websocket capacity must be positive".to_string(),
            ));
        }
        if request_expiry.is_zero() || acknowledgement_timeout.is_zero() {
            return Err(AdapterError::InvalidConfiguration(
                "order command websocket request and acknowledgement timeouts must be positive"
                    .to_string(),
            ));
        }
        if reconnect.initial_delay.is_zero()
            || reconnect.max_delay.is_zero()
            || reconnect.max_delay < reconnect.initial_delay
            || reconnect.multiplier == 0
        {
            return Err(AdapterError::InvalidConfiguration(
                "order command websocket reconnect policy requires positive ordered delays and a positive multiplier"
                    .to_string(),
            ));
        }
        Ok(Self {
            account_id,
            websocket_url,
            command_capacity,
            request_expiry,
            acknowledgement_timeout,
            connection_attempt_pacer,
            reconnect,
        })
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum OrderCommandWebsocketStatusKind {
    Ready,
    Heartbeat,
    Disconnected,
    Fatal,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct OrderCommandWebsocketStatus {
    pub account_id: String,
    pub ts_ms: u64,
    pub kind: OrderCommandWebsocketStatusKind,
    pub ready_sessions: usize,
    pub total_sessions: usize,
    pub reason: String,
}

#[derive(Clone)]
pub(crate) struct OrderCommandWebsocketTransport {
    sessions: Arc<Vec<SessionHandle>>,
    session_factory: Arc<dyn OrderSessionOperations>,
    request_sequence: Arc<AtomicU64>,
    request_expiry: Duration,
}

pub struct OrderCommandWebsocketLifecycle {
    shutdown: watch::Sender<bool>,
    tasks: Vec<JoinHandle<()>>,
}

impl OrderCommandWebsocketLifecycle {
    pub fn request_shutdown(&self) {
        let _ = self.shutdown.send(true);
    }

    pub async fn shutdown(mut self) -> Result<(), JoinError> {
        self.request_shutdown();
        for task in &mut self.tasks {
            task.await?;
        }
        self.tasks.clear();
        Ok(())
    }
}

impl Drop for OrderCommandWebsocketLifecycle {
    fn drop(&mut self) {
        self.request_shutdown();
        for task in &self.tasks {
            task.abort();
        }
    }
}

#[derive(Clone)]
struct SessionHandle {
    commands: mpsc::Sender<SessionCommand>,
    ready: watch::Receiver<bool>,
}

struct ExpectedOrderIdentity {
    account_id: String,
    symbol: String,
    client_order_id: String,
}

struct SessionCommand {
    request_id: String,
    operation: OkxWsOrderOperation,
    expected_account_id: String,
    expected_symbol: String,
    expected_client_order_id: String,
    payload: String,
    send_deadline: Instant,
    response: oneshot::Sender<Result<OkxOrderAck, OrderTransportError>>,
}

struct PendingRequest {
    operation: OkxWsOrderOperation,
    expected_account_id: String,
    expected_symbol: String,
    expected_client_order_id: String,
    acknowledgement_deadline: Instant,
    response: oneshot::Sender<Result<OkxOrderAck, OrderTransportError>>,
}

pub(crate) trait OrderSessionOperations: Send + Sync {
    fn login_message(&self) -> Result<String, String>;
    fn place_request(
        &self,
        request_id: &str,
        expiry_ms: u64,
        order: PreparedRegularSubmit,
    ) -> Result<String, String>;
    fn cancel_request(
        &self,
        request_id: &str,
        order: &PreparedRegularCancel,
    ) -> Result<String, String>;
}

impl OrderSessionOperations for RegularOrderSessionFactory {
    fn login_message(&self) -> Result<String, String> {
        RegularOrderSessionFactory::login_message(self)
    }

    fn place_request(
        &self,
        request_id: &str,
        expiry_ms: u64,
        order: PreparedRegularSubmit,
    ) -> Result<String, String> {
        RegularOrderSessionFactory::place_request(self, request_id, expiry_ms, order)
            .map_err(|error| error.to_string())
    }

    fn cancel_request(
        &self,
        request_id: &str,
        order: &PreparedRegularCancel,
    ) -> Result<String, String> {
        RegularOrderSessionFactory::cancel_request(self, request_id, order)
            .map_err(|error| error.to_string())
    }
}

impl BoundRegularOrderGateway {
    pub fn start_and_install(
        self,
        config: OrderCommandWebsocketConfig,
    ) -> Result<
        (
            OkxOrderGateway,
            OrderCommandWebsocketLifecycle,
            mpsc::Receiver<OrderCommandWebsocketStatus>,
        ),
        AdapterError,
    > {
        let Self {
            gateway,
            order_sessions,
            order_transport,
        } = self;
        order_sessions.validate_start(&config)?;
        let (lifecycle, status) =
            start_and_install_order_command_websocket(order_sessions, order_transport, config)?;
        Ok((gateway, lifecycle, status))
    }
}

impl RegularOrderSessionFactory {
    pub(crate) fn validate_start(
        &self,
        config: &OrderCommandWebsocketConfig,
    ) -> Result<(), AdapterError> {
        if !self.demo_trading {
            return Err(AdapterError::InvalidConfiguration(
                "order command websocket authority is demo-trading only".to_string(),
            ));
        }
        if config.account_id != self.expected_account_id {
            return Err(AdapterError::InvalidConfiguration(format!(
                "order command websocket account {} does not match credential role account {}",
                config.account_id, self.expected_account_id
            )));
        }
        validate_private_websocket_url(&config.websocket_url, self.demo_trading)?;
        let endpoint = url::Url::parse(&config.websocket_url).map_err(|error| {
            AdapterError::InvalidConfiguration(format!(
                "order command websocket URL is invalid: {error}"
            ))
        })?;
        let host = endpoint.host_str().ok_or_else(|| {
            AdapterError::InvalidConfiguration(
                "order command websocket URL must contain a host".to_string(),
            )
        })?;
        let demo_loopback = self.demo_trading && is_loopback_host(host);
        if !demo_loopback {
            let minimum = Duration::from_millis(OKX_MIN_CONNECTION_ATTEMPT_INTERVAL_MS);
            if config.connection_attempt_pacer.interval() < minimum {
                return Err(AdapterError::InvalidConfiguration(format!(
                    "official order command websocket pacing must be at least {}ms",
                    OKX_MIN_CONNECTION_ATTEMPT_INTERVAL_MS
                )));
            }
            if !config.connection_attempt_pacer.is_process_shared() {
                return Err(AdapterError::InvalidConfiguration(
                    "official order command websocket pacing must be process-shared".to_string(),
                ));
            }
        }
        Ok(())
    }
}

#[derive(Debug)]
struct SessionStatus {
    index: usize,
    kind: OrderCommandWebsocketStatusKind,
    reason: String,
}

fn start_and_install_order_command_websocket(
    session_factory: RegularOrderSessionFactory,
    order_transport: Arc<OrderCommandTransportSlot>,
    config: OrderCommandWebsocketConfig,
) -> Result<
    (
        OrderCommandWebsocketLifecycle,
        mpsc::Receiver<OrderCommandWebsocketStatus>,
    ),
    AdapterError,
> {
    let session_factory: Arc<dyn OrderSessionOperations> = Arc::new(session_factory);
    start_order_command_websocket(session_factory, config, move |transport| {
        order_transport.install(transport.clone())
    })
}

fn start_order_command_websocket(
    session_factory: Arc<dyn OrderSessionOperations>,
    config: OrderCommandWebsocketConfig,
    before_spawn: impl FnOnce(&OrderCommandWebsocketTransport) -> Result<(), AdapterError>,
) -> Result<
    (
        OrderCommandWebsocketLifecycle,
        mpsc::Receiver<OrderCommandWebsocketStatus>,
    ),
    AdapterError,
> {
    let _connection_capability = okx_capability_registration("OKX-CONNECTION-ORDER-COMMAND")
        .expect("order command connection must remain in the OKX capability registry");
    let (shutdown_tx, shutdown_rx) = watch::channel(false);
    let (session_status_tx, session_status_rx) = mpsc::channel(ORDER_COMMAND_SESSION_COUNT * 4);
    let (aggregate_status_tx, aggregate_status_rx) = mpsc::channel(STATUS_CHANNEL_CAPACITY);
    let mut handles = Vec::with_capacity(ORDER_COMMAND_SESSION_COUNT);
    let mut session_starts = Vec::with_capacity(ORDER_COMMAND_SESSION_COUNT);

    for index in 0..ORDER_COMMAND_SESSION_COUNT {
        let (command_tx, command_rx) = mpsc::channel(config.command_capacity);
        let (ready_tx, ready_rx) = watch::channel(false);
        handles.push(SessionHandle {
            commands: command_tx,
            ready: ready_rx,
        });
        session_starts.push((index, command_rx, ready_tx));
    }

    let transport = OrderCommandWebsocketTransport {
        sessions: Arc::new(handles),
        session_factory: Arc::clone(&session_factory),
        request_sequence: Arc::new(AtomicU64::new(unix_time_ns())),
        request_expiry: config.request_expiry,
    };
    before_spawn(&transport)?;

    let mut tasks = Vec::with_capacity(ORDER_COMMAND_SESSION_COUNT + 1);
    for (index, command_rx, ready_tx) in session_starts {
        tasks.push(tokio::spawn(supervise_session(
            index,
            config.websocket_url.clone(),
            Arc::clone(&session_factory),
            command_rx,
            ready_tx,
            session_status_tx.clone(),
            shutdown_rx.clone(),
            config.connection_attempt_pacer.clone(),
            config.reconnect.clone(),
            config.acknowledgement_timeout,
        )));
    }
    drop(session_status_tx);
    tasks.push(tokio::spawn(aggregate_status(
        config.account_id,
        ORDER_COMMAND_SESSION_COUNT,
        session_status_rx,
        aggregate_status_tx,
        shutdown_rx,
    )));

    Ok((
        OrderCommandWebsocketLifecycle {
            shutdown: shutdown_tx,
            tasks,
        },
        aggregate_status_rx,
    ))
}

impl OrderCommandWebsocketTransport {
    pub(crate) async fn place_order(
        &self,
        order: PreparedRegularSubmit,
    ) -> Result<OkxOrderAck, OrderTransportError> {
        let session_index = ORDER_COMMAND_SESSION_INDEX;
        let request_id = self.next_request_id(session_index);
        let expiry_ms = unix_time_ms().saturating_add(duration_ms(self.request_expiry));
        let expected_identity = ExpectedOrderIdentity {
            account_id: order.account_id().to_string(),
            symbol: order.order().symbol.clone(),
            client_order_id: order.client_order_id().to_string(),
        };
        let payload = self
            .session_factory
            .place_request(&request_id, expiry_ms, order)
            .map_err(|error| OrderTransportError::InvalidRequest(error.to_string()))?;
        self.execute(
            session_index,
            request_id,
            OkxWsOrderOperation::Place,
            expected_identity,
            payload,
        )
        .await
    }

    pub(crate) async fn cancel_order(
        &self,
        order: PreparedRegularCancel,
    ) -> Result<OkxOrderAck, CancelOrderTransportError> {
        let session_index = ORDER_COMMAND_SESSION_INDEX;
        if !*self.sessions[session_index].ready.borrow() {
            return Err(CancelOrderTransportError::pre_send_unavailable(
                format!("session {session_index} is not authenticated"),
                order,
            ));
        }
        let request_id = self.next_request_id(session_index);
        let expected_identity = ExpectedOrderIdentity {
            account_id: order.account_id().to_string(),
            symbol: order.symbol().to_string(),
            client_order_id: order.client_order_id().to_string(),
        };
        let payload = self
            .session_factory
            .cancel_request(&request_id, &order)
            .map_err(|error| {
                CancelOrderTransportError::failed(OrderTransportError::InvalidRequest(
                    error.to_string(),
                ))
            })?;
        match self
            .execute(
                session_index,
                request_id,
                OkxWsOrderOperation::Cancel,
                expected_identity,
                payload,
            )
            .await
        {
            Ok(ack) => Ok(ack),
            Err(OrderTransportError::Unavailable(message)) => Err(
                CancelOrderTransportError::pre_send_unavailable(message, order),
            ),
            Err(error) => Err(CancelOrderTransportError::failed(error)),
        }
    }
}

impl OrderCommandWebsocketTransport {
    fn next_request_id(&self, session_index: usize) -> String {
        let sequence = self.request_sequence.fetch_add(1, Ordering::Relaxed);
        format!("r{session_index:02x}{sequence:016x}")
    }

    async fn execute(
        &self,
        session_index: usize,
        request_id: String,
        operation: OkxWsOrderOperation,
        expected_identity: ExpectedOrderIdentity,
        payload: String,
    ) -> Result<OkxOrderAck, OrderTransportError> {
        let session = &self.sessions[session_index];
        if !*session.ready.borrow() {
            return Err(OrderTransportError::Unavailable(format!(
                "session {session_index} is not authenticated"
            )));
        }
        let (response_tx, response_rx) = oneshot::channel();
        let ExpectedOrderIdentity {
            account_id: expected_account_id,
            symbol: expected_symbol,
            client_order_id: expected_client_order_id,
        } = expected_identity;
        let command = SessionCommand {
            request_id,
            operation,
            expected_account_id,
            expected_symbol,
            expected_client_order_id,
            payload,
            send_deadline: Instant::now() + self.request_expiry,
            response: response_tx,
        };
        session
            .commands
            .try_send(command)
            .map_err(|error| match error {
                mpsc::error::TrySendError::Full(_) => OrderTransportError::Unavailable(format!(
                    "session {session_index} command queue is full"
                )),
                mpsc::error::TrySendError::Closed(_) => OrderTransportError::Unavailable(format!(
                    "session {session_index} command queue is closed"
                )),
            })?;
        response_rx.await.unwrap_or_else(|_| {
            Err(OrderTransportError::Ambiguous(format!(
                "session {session_index} ended without classifying the request"
            )))
        })
    }
}

#[allow(clippy::too_many_arguments)]
async fn supervise_session(
    index: usize,
    websocket_url: String,
    session_factory: Arc<dyn OrderSessionOperations>,
    mut commands: mpsc::Receiver<SessionCommand>,
    ready: watch::Sender<bool>,
    status: mpsc::Sender<SessionStatus>,
    mut shutdown: watch::Receiver<bool>,
    connection_attempt_pacer: ConnectionAttemptPacer,
    reconnect: ReconnectPolicy,
    acknowledgement_timeout: Duration,
) {
    let mut delay = reconnect.initial_delay;
    loop {
        reject_queued(&mut commands, "order websocket is reconnecting");
        if *shutdown.borrow() {
            reject_queued(&mut commands, "order websocket is shutting down");
            return;
        }
        match connection_attempt_pacer.wait_for_turn(&mut shutdown).await {
            Ok(true) => {}
            Ok(false) => {
                reject_queued(&mut commands, "order websocket is shutting down");
                return;
            }
            Err(error) => {
                let reason = format!("order websocket connection pacer failed: {error}");
                let _ = ready.send(false);
                let _ = status
                    .send(SessionStatus {
                        index,
                        kind: OrderCommandWebsocketStatusKind::Fatal,
                        reason: reason.clone(),
                    })
                    .await;
                reject_queued(&mut commands, &reason);
                tracing::error!(session = index, %error, "order websocket connection pacer failed");
                return;
            }
        }
        let mut authenticated = false;
        let result = run_authenticated_session(
            index,
            &websocket_url,
            session_factory.as_ref(),
            &mut commands,
            &ready,
            &status,
            &mut shutdown,
            acknowledgement_timeout,
            &mut authenticated,
        )
        .await;
        if authenticated {
            delay = reconnect.initial_delay;
        }
        let _ = ready.send(false);
        if *shutdown.borrow() {
            reject_queued(&mut commands, "order websocket is shutting down");
            return;
        }
        let reason = result
            .err()
            .unwrap_or_else(|| "order websocket ended unexpectedly".to_string());
        if status
            .send(SessionStatus {
                index,
                kind: OrderCommandWebsocketStatusKind::Disconnected,
                reason: reason.clone(),
            })
            .await
            .is_err()
        {
            reject_queued(&mut commands, "order websocket status monitor closed");
            return;
        }
        tracing::warn!(session = index, ?delay, %reason, "order websocket reconnecting");
        tokio::select! {
            _ = tokio::time::sleep(delay) => {}
            changed = shutdown.changed() => {
                if changed.is_err() || *shutdown.borrow() {
                    reject_queued(&mut commands, "order websocket is shutting down");
                    return;
                }
            }
        }
        delay = reconnect.next_delay(delay);
    }
}

#[allow(clippy::too_many_arguments)]
async fn run_authenticated_session(
    index: usize,
    websocket_url: &str,
    session_factory: &dyn OrderSessionOperations,
    commands: &mut mpsc::Receiver<SessionCommand>,
    ready: &watch::Sender<bool>,
    status: &mpsc::Sender<SessionStatus>,
    shutdown: &mut watch::Receiver<bool>,
    acknowledgement_timeout: Duration,
    authenticated: &mut bool,
) -> Result<(), String> {
    let connection = tokio_tungstenite::connect_async(websocket_url);
    tokio::pin!(connection);
    let connection_timeout = tokio::time::sleep(LOGIN_TIMEOUT);
    tokio::pin!(connection_timeout);
    let (socket, _) = tokio::select! {
        result = &mut connection => {
            result.map_err(|error| format!("connection failed: {error}"))?
        }
        _ = &mut connection_timeout => {
            return Err("order websocket connection timed out".to_string());
        }
        changed = shutdown.changed() => {
            if changed.is_err() || *shutdown.borrow() {
                return Err("order websocket shutdown during connection".to_string());
            }
            return Err("order websocket shutdown state changed during connection".to_string());
        }
    };
    let (mut writer, mut reader) = socket.split();
    let login = session_factory
        .login_message()
        .map_err(|error| format!("login generation failed: {error}"))?;
    send_control_message(&mut writer, Message::Text(login.into()), "login").await?;
    await_login(&mut writer, &mut reader, shutdown).await?;
    let _ = ready.send(true);
    status
        .send(SessionStatus {
            index,
            kind: OrderCommandWebsocketStatusKind::Ready,
            reason: "authenticated".to_string(),
        })
        .await
        .map_err(|_| "order websocket status monitor closed".to_string())?;
    *authenticated = true;

    let mut pending = HashMap::new();
    let result = run_connected_loop(
        index,
        &mut writer,
        &mut reader,
        commands,
        status,
        shutdown,
        acknowledgement_timeout,
        &mut pending,
    )
    .await;
    if result.is_err() {
        fail_pending(
            &mut pending,
            "order websocket disconnected before acknowledgement",
        );
    }
    result
}

#[allow(clippy::too_many_arguments)]
async fn run_connected_loop<S>(
    index: usize,
    writer: &mut futures_util::stream::SplitSink<tokio_tungstenite::WebSocketStream<S>, Message>,
    reader: &mut futures_util::stream::SplitStream<tokio_tungstenite::WebSocketStream<S>>,
    commands: &mut mpsc::Receiver<SessionCommand>,
    status: &mpsc::Sender<SessionStatus>,
    shutdown: &mut watch::Receiver<bool>,
    acknowledgement_timeout: Duration,
    pending: &mut HashMap<String, PendingRequest>,
) -> Result<(), String>
where
    S: tokio::io::AsyncRead + tokio::io::AsyncWrite + Unpin,
{
    let mut ping = tokio::time::interval(PING_INTERVAL);
    ping.set_missed_tick_behavior(tokio::time::MissedTickBehavior::Delay);
    ping.tick().await;
    let mut expiry_scan = tokio::time::interval(EXPIRY_SCAN_INTERVAL);
    expiry_scan.set_missed_tick_behavior(tokio::time::MissedTickBehavior::Skip);
    expiry_scan.tick().await;
    let mut last_received = Instant::now();

    loop {
        tokio::select! {
            biased;
            changed = shutdown.changed() => {
                if changed.is_err() || *shutdown.borrow() {
                    let _ = send_control_message(writer, Message::Close(None), "close").await;
                    fail_pending(pending, "order websocket shut down before acknowledgement");
                    return Ok(());
                }
            }
            message = reader.next() => {
                let message = message
                    .ok_or_else(|| "peer closed the order websocket".to_string())?
                    .map_err(|error| format!("order websocket receive failed: {error}"))?;
                last_received = Instant::now();
                match message {
                    Message::Text(payload) => {
                        if payload.as_str() == "pong" {
                            try_send_heartbeat(index, status)?;
                        } else {
                            process_order_response(payload.as_str(), pending)?;
                        }
                    }
                    Message::Binary(payload) => {
                        let payload = std::str::from_utf8(payload.as_ref())
                            .map_err(|_| "order websocket returned non-UTF8 data".to_string())?;
                        if payload == "pong" {
                            try_send_heartbeat(index, status)?;
                        } else {
                            process_order_response(payload, pending)?;
                        }
                    }
                    Message::Ping(payload) => {
                        send_control_message(writer, Message::Pong(payload), "pong").await?
                    }
                    Message::Pong(_) => try_send_heartbeat(index, status)?,
                    Message::Close(_) => return Err("peer closed the order websocket".to_string()),
                    Message::Frame(_) => {}
                }
            }
            command = commands.recv() => {
                let Some(command) = command else {
                    return Err("order websocket command channel closed".to_string());
                };
                if Instant::now() >= command.send_deadline {
                    let _ = command.response.send(Err(OrderTransportError::Unavailable(
                        "request expired in the local queue before send".to_string(),
                    )));
                    continue;
                }
                let SessionCommand {
                    request_id,
                    operation,
                    expected_account_id,
                    expected_symbol,
                    expected_client_order_id,
                    payload,
                    send_deadline,
                    response,
                } = command;
                if pending.contains_key(&request_id) {
                    let _ = response.send(Err(OrderTransportError::InvalidRequest(
                        format!("duplicate request id {request_id}"),
                    )));
                    continue;
                }
                let write_budget = send_deadline.saturating_duration_since(Instant::now());
                match tokio::time::timeout(
                    write_budget,
                    writer.send(Message::Text(payload.into())),
                )
                .await
                {
                    Ok(Ok(())) => {}
                    Ok(Err(error)) => {
                        let _ = response.send(Err(OrderTransportError::Ambiguous(format!(
                            "websocket write failed: {error}"
                        ))));
                        return Err(format!("order websocket write failed: {error}"));
                    }
                    Err(_) => {
                        let _ = response.send(Err(OrderTransportError::Ambiguous(
                            "websocket write exceeded the request expiry".to_string(),
                        )));
                        return Err("order websocket write timed out".to_string());
                    }
                }
                pending.insert(
                    request_id,
                    PendingRequest {
                        operation,
                        expected_account_id,
                        expected_symbol,
                        expected_client_order_id,
                        acknowledgement_deadline: Instant::now() + acknowledgement_timeout,
                        response,
                    },
                );
            }
            _ = ping.tick() => {
                if last_received.elapsed() > IDLE_TIMEOUT {
                    return Err("order websocket received no data before idle timeout".to_string());
                }
                send_control_message(writer, Message::Text("ping".into()), "ping").await?;
            }
            _ = expiry_scan.tick(), if !pending.is_empty() => {
                let now = Instant::now();
                let expired = pending
                    .iter()
                    .filter(|(_, request)| now >= request.acknowledgement_deadline)
                    .map(|(request_id, _)| request_id.clone())
                    .collect::<Vec<_>>();
                if !expired.is_empty() {
                    for request_id in expired {
                        if let Some(request) = pending.remove(&request_id) {
                            let _ = request.response.send(Err(OrderTransportError::Ambiguous(
                                format!("request {request_id} acknowledgement timed out"),
                            )));
                        }
                    }
                    return Err("order websocket acknowledgement timeout".to_string());
                }
            }
        }
    }
}

async fn await_login<S>(
    writer: &mut futures_util::stream::SplitSink<tokio_tungstenite::WebSocketStream<S>, Message>,
    reader: &mut futures_util::stream::SplitStream<tokio_tungstenite::WebSocketStream<S>>,
    shutdown: &mut watch::Receiver<bool>,
) -> Result<(), String>
where
    S: tokio::io::AsyncRead + tokio::io::AsyncWrite + Unpin,
{
    let timeout = tokio::time::sleep(LOGIN_TIMEOUT);
    tokio::pin!(timeout);
    loop {
        tokio::select! {
            _ = &mut timeout => return Err("order websocket login timed out".to_string()),
            changed = shutdown.changed() => {
                if changed.is_err() || *shutdown.borrow() {
                    return Err("order websocket shutdown during login".to_string());
                }
            }
            message = reader.next() => {
                let message = message
                    .ok_or_else(|| "peer closed during order websocket login".to_string())?
                    .map_err(|error| format!("order websocket login receive failed: {error}"))?;
                match message {
                    Message::Text(payload) => {
                        if login_succeeded(payload.as_str())? {
                            return Ok(());
                        }
                    }
                    Message::Binary(payload) => {
                        let payload = std::str::from_utf8(payload.as_ref())
                            .map_err(|_| "order websocket login returned non-UTF8 data".to_string())?;
                        if login_succeeded(payload)? {
                            return Ok(());
                        }
                    }
                    Message::Ping(payload) => {
                        send_control_message(writer, Message::Pong(payload), "login pong").await?
                    }
                    Message::Pong(_) => {}
                    Message::Close(_) => return Err("peer closed during order websocket login".to_string()),
                    Message::Frame(_) => {}
                }
            }
        }
    }
}

async fn send_control_message<S>(
    writer: &mut futures_util::stream::SplitSink<tokio_tungstenite::WebSocketStream<S>, Message>,
    message: Message,
    operation: &'static str,
) -> Result<(), String>
where
    S: tokio::io::AsyncRead + tokio::io::AsyncWrite + Unpin,
{
    tokio::time::timeout(CONTROL_WRITE_TIMEOUT, writer.send(message))
        .await
        .map_err(|_| format!("order websocket {operation} timed out"))?
        .map_err(|error| format!("order websocket {operation} failed: {error}"))
}

fn login_succeeded(payload: &str) -> Result<bool, String> {
    if payload == "pong" {
        return Ok(false);
    }
    let value: Value = serde_json::from_str(payload)
        .map_err(|error| format!("malformed order websocket login response: {error}"))?;
    match value.get("event").and_then(Value::as_str) {
        Some("login") if value.get("code").and_then(Value::as_str) == Some("0") => Ok(true),
        Some("login") | Some("error") | Some("notice") => Err(format!(
            "order websocket login failed: code={} message={}",
            value.get("code").and_then(Value::as_str).unwrap_or(""),
            value.get("msg").and_then(Value::as_str).unwrap_or("")
        )),
        _ => Err("unexpected message during order websocket login".to_string()),
    }
}

fn process_order_response(
    payload: &str,
    pending: &mut HashMap<String, PendingRequest>,
) -> Result<(), String> {
    let value: Value = serde_json::from_str(payload)
        .map_err(|error| format!("malformed order websocket response: {error}"))?;
    if value.get("id").and_then(Value::as_str).is_none() {
        return match value.get("event").and_then(Value::as_str) {
            Some("error") | Some("notice") => Err(format!(
                "order websocket server event: code={} message={}",
                value.get("code").and_then(Value::as_str).unwrap_or(""),
                value.get("msg").and_then(Value::as_str).unwrap_or("")
            )),
            _ => Err("unexpected uncorrelated order websocket message".to_string()),
        };
    }
    let result = parse_okx_ws_order_response(payload)
        .map_err(|error| format!("malformed correlated order response: {error}"))?;
    let request_id = result.request_id().to_string();
    let request = pending
        .remove(&request_id)
        .ok_or_else(|| format!("response references unknown request id {request_id}"))?;
    if request.operation != result.operation() {
        let _ = request
            .response
            .send(Err(OrderTransportError::Ambiguous(format!(
                "request {request_id} expected op {} but received {}",
                request.operation.as_str(),
                result.operation().as_str()
            ))));
        return Err(format!("request {request_id} operation mismatch"));
    }
    if let Err(message) = validate_acknowledgement_identity(&value, &request) {
        let _ = request
            .response
            .send(Err(OrderTransportError::Ambiguous(message.clone())));
        return Err(message);
    }
    match result {
        OkxWsOrderResult::Accepted {
            mut acknowledgement,
            ..
        } => {
            if acknowledgement.exchange_order_id.trim().is_empty()
                || acknowledgement.exchange_order_id.trim() != acknowledgement.exchange_order_id
                || acknowledgement.exchange_order_id == "0"
            {
                let message = format!(
                    "request {request_id} returned invalid exchange order id {:?}",
                    acknowledgement.exchange_order_id
                );
                let _ = request
                    .response
                    .send(Err(OrderTransportError::Ambiguous(message.clone())));
                return Err(message);
            }
            if acknowledgement.client_order_id.is_empty() || acknowledgement.client_order_id == "0"
            {
                acknowledgement.client_order_id = request.expected_client_order_id;
            }
            let _ = request.response.send(Ok(acknowledgement));
        }
        OkxWsOrderResult::Rejected { code, message, .. } => {
            let _ = request
                .response
                .send(Err(OrderTransportError::Rejected { code, message }));
        }
    }
    Ok(())
}

fn validate_acknowledgement_identity(
    response: &Value,
    request: &PendingRequest,
) -> Result<(), String> {
    for (field, expected, aliases, allows_zero_placeholder) in [
        (
            "account",
            request.expected_account_id.as_str(),
            &["accountId", "acctId", "account_id"][..],
            false,
        ),
        (
            "symbol",
            request.expected_symbol.as_str(),
            &["instId", "symbol"][..],
            false,
        ),
        (
            "client order id",
            request.expected_client_order_id.as_str(),
            &["clOrdId", "clientOrderId", "client_order_id"][..],
            true,
        ),
    ] {
        let row = response
            .get("data")
            .and_then(Value::as_array)
            .and_then(|rows| rows.first());
        for container in std::iter::once(response).chain(row) {
            for alias in aliases {
                let Some(value) = container.get(*alias) else {
                    continue;
                };
                let actual = value.as_str().ok_or_else(|| {
                    format!(
                        "order websocket {field} acknowledgement field {alias} must be a string"
                    )
                })?;
                if actual.is_empty()
                    || (allows_zero_placeholder && actual == "0")
                    || (actual == expected && actual != "0")
                {
                    continue;
                }
                return Err(format!(
                    "order websocket {field} acknowledgement mismatch: expected {expected:?}, received {actual:?}"
                ));
            }
        }
    }
    Ok(())
}

fn try_send_heartbeat(index: usize, status: &mpsc::Sender<SessionStatus>) -> Result<(), String> {
    match status.try_send(SessionStatus {
        index,
        kind: OrderCommandWebsocketStatusKind::Heartbeat,
        reason: "pong received".to_string(),
    }) {
        Ok(()) | Err(mpsc::error::TrySendError::Full(_)) => Ok(()),
        Err(mpsc::error::TrySendError::Closed(_)) => {
            Err("order websocket status monitor closed".to_string())
        }
    }
}

fn reject_queued(commands: &mut mpsc::Receiver<SessionCommand>, reason: &str) {
    while let Ok(command) = commands.try_recv() {
        let _ = command
            .response
            .send(Err(OrderTransportError::Unavailable(reason.to_string())));
    }
}

fn fail_pending(pending: &mut HashMap<String, PendingRequest>, reason: &str) {
    for (request_id, request) in pending.drain() {
        let _ = request
            .response
            .send(Err(OrderTransportError::Ambiguous(format!(
                "request {request_id}: {reason}"
            ))));
    }
}

async fn aggregate_status(
    account_id: String,
    total_sessions: usize,
    mut session_status: mpsc::Receiver<SessionStatus>,
    output: mpsc::Sender<OrderCommandWebsocketStatus>,
    mut shutdown: watch::Receiver<bool>,
) {
    let mut ready_sessions = HashSet::new();
    let mut aggregate_ready = false;
    let mut disconnected_reported = false;
    loop {
        tokio::select! {
            changed = shutdown.changed() => {
                if changed.is_err() || *shutdown.borrow() {
                    return;
                }
            }
            status = session_status.recv() => {
                let Some(status) = status else { return; };
                match status.kind {
                    OrderCommandWebsocketStatusKind::Ready | OrderCommandWebsocketStatusKind::Heartbeat => {
                        ready_sessions.insert(status.index);
                    }
                    OrderCommandWebsocketStatusKind::Disconnected | OrderCommandWebsocketStatusKind::Fatal => {
                        ready_sessions.remove(&status.index);
                    }
                }
                let all_ready = ready_sessions.len() == total_sessions;
                let aggregate_kind = match status.kind {
                    OrderCommandWebsocketStatusKind::Disconnected
                        if aggregate_ready || !disconnected_reported =>
                    {
                        disconnected_reported = true;
                        Some(OrderCommandWebsocketStatusKind::Disconnected)
                    }
                    OrderCommandWebsocketStatusKind::Ready if all_ready && !aggregate_ready => {
                        disconnected_reported = false;
                        Some(OrderCommandWebsocketStatusKind::Ready)
                    }
                    OrderCommandWebsocketStatusKind::Heartbeat if all_ready => {
                        Some(OrderCommandWebsocketStatusKind::Heartbeat)
                    }
                    OrderCommandWebsocketStatusKind::Fatal => Some(OrderCommandWebsocketStatusKind::Fatal),
                    _ => None,
                };
                aggregate_ready = all_ready;
                let Some(kind) = aggregate_kind else { continue; };
                let terminal = kind == OrderCommandWebsocketStatusKind::Fatal;
                let reason = match kind {
                    OrderCommandWebsocketStatusKind::Ready => {
                        "every order websocket session is authenticated".to_string()
                    }
                    OrderCommandWebsocketStatusKind::Heartbeat => {
                        "every order websocket session is authenticated and responsive".to_string()
                    }
                    OrderCommandWebsocketStatusKind::Disconnected => format!(
                        "order websocket session {} disconnected: {}",
                        status.index, status.reason
                    ),
                    OrderCommandWebsocketStatusKind::Fatal => format!(
                        "order websocket session {} failed permanently: {}",
                        status.index, status.reason
                    ),
                };
                let aggregate = OrderCommandWebsocketStatus {
                    account_id: account_id.clone(),
                    ts_ms: unix_time_ms(),
                    kind,
                    ready_sessions: ready_sessions.len(),
                    total_sessions,
                    reason,
                };
                let output_closed = if kind == OrderCommandWebsocketStatusKind::Heartbeat {
                    matches!(
                        output.try_send(aggregate),
                        Err(mpsc::error::TrySendError::Closed(_))
                    )
                } else {
                    output.send(aggregate).await.is_err()
                };
                if output_closed {
                    return;
                }
                if terminal {
                    return;
                }
            }
        }
    }
}

fn unix_time_ms() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap_or_default()
        .as_millis()
        .min(u64::MAX as u128) as u64
}

fn unix_time_ns() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap_or_default()
        .as_nanos()
        .min(u64::MAX as u128) as u64
}

fn duration_ms(duration: Duration) -> u64 {
    duration.as_millis().min(u64::MAX as u128) as u64
}

#[cfg(test)]
mod tests {
    use reap_venue::okx::{OKX_WS_CANCEL_ORDER_OP, OKX_WS_PLACE_ORDER_OP};
    use tokio::net::TcpListener;
    use tokio_tungstenite::accept_async;

    use crate::OrderCommandTransportSlot;

    use super::*;

    struct TestSessionOperations;

    impl OrderSessionOperations for TestSessionOperations {
        fn login_message(&self) -> Result<String, String> {
            Ok(r#"{"op":"login","args":[{"apiKey":"test"}]}"#.to_string())
        }

        fn place_request(
            &self,
            request_id: &str,
            expiry_ms: u64,
            order: PreparedRegularSubmit,
        ) -> Result<String, String> {
            let regular_order = order.order();
            Ok(serde_json::json!({
                "id": request_id,
                "op": OKX_WS_PLACE_ORDER_OP,
                "expTime": expiry_ms.to_string(),
                "args": [{
                    "instId": regular_order.symbol,
                    "tdMode": "cross",
                    "side": "buy",
                    "ordType": "post_only",
                    "px": regular_order.price.to_string(),
                    "sz": regular_order.qty.to_string(),
                    "clOrdId": order.client_order_id(),
                }],
            })
            .to_string())
        }

        fn cancel_request(
            &self,
            request_id: &str,
            order: &PreparedRegularCancel,
        ) -> Result<String, String> {
            Ok(serde_json::json!({
                "id": request_id,
                "op": OKX_WS_CANCEL_ORDER_OP,
                "args": [{
                    "instId": order.symbol(),
                    "clOrdId": order.client_order_id(),
                }],
            })
            .to_string())
        }
    }

    fn config(url: String) -> OrderCommandWebsocketConfig {
        OrderCommandWebsocketConfig::new(
            "account-a",
            url,
            8,
            Duration::from_millis(1_000),
            Duration::from_millis(500),
            ConnectionAttemptPacer::new(Duration::ZERO),
            ReconnectPolicy {
                initial_delay: Duration::from_millis(10),
                max_delay: Duration::from_millis(20),
                multiplier: 2,
            },
        )
        .unwrap()
    }

    fn start_test_order_command_websocket(
        config: OrderCommandWebsocketConfig,
    ) -> (
        OrderCommandWebsocketTransport,
        OrderCommandWebsocketLifecycle,
        mpsc::Receiver<OrderCommandWebsocketStatus>,
    ) {
        let mut installed = None;
        let (runtime, status) =
            start_order_command_websocket(Arc::new(TestSessionOperations), config, |transport| {
                installed = Some(transport.clone());
                Ok(())
            })
            .unwrap();
        (
            installed.expect("test transport must be installed before tasks start"),
            runtime,
            status,
        )
    }

    impl OrderCommandWebsocketTransport {
        async fn place_test_order(&self) -> Result<OkxOrderAck, OrderTransportError> {
            let session_index = ORDER_COMMAND_SESSION_INDEX;
            let request_id = self.next_request_id(session_index);
            let expiry_ms = unix_time_ms().saturating_add(duration_ms(self.request_expiry));
            let payload = serde_json::json!({
                "id": request_id,
                "op": OKX_WS_PLACE_ORDER_OP,
                "expTime": expiry_ms.to_string(),
                "args": [{
                    "instId": "BTC-USDT-SWAP",
                    "tdMode": "cross",
                    "side": "buy",
                    "ordType": "post_only",
                    "px": "50000",
                    "sz": "0.01",
                    "clOrdId": "reap1",
                }],
            })
            .to_string();
            self.execute(
                session_index,
                request_id,
                OkxWsOrderOperation::Place,
                ExpectedOrderIdentity {
                    account_id: "account-a".to_string(),
                    symbol: "BTC-USDT-SWAP".to_string(),
                    client_order_id: "reap1".to_string(),
                },
                payload,
            )
            .await
        }

        async fn cancel_test_order(&self) -> Result<OkxOrderAck, OrderTransportError> {
            let session_index = ORDER_COMMAND_SESSION_INDEX;
            let request_id = self.next_request_id(session_index);
            let payload = serde_json::json!({
                "id": request_id,
                "op": OKX_WS_CANCEL_ORDER_OP,
                "args": [{
                    "instId": "BTC-USDT-SWAP",
                    "clOrdId": "reap1",
                }],
            })
            .to_string();
            self.execute(
                session_index,
                request_id,
                OkxWsOrderOperation::Cancel,
                ExpectedOrderIdentity {
                    account_id: "account-a".to_string(),
                    symbol: "BTC-USDT-SWAP".to_string(),
                    client_order_id: "reap1".to_string(),
                },
                payload,
            )
            .await
        }
    }

    async fn next_text<S>(socket: &mut tokio_tungstenite::WebSocketStream<S>) -> String
    where
        S: tokio::io::AsyncRead + tokio::io::AsyncWrite + Unpin,
    {
        match socket.next().await.unwrap().unwrap() {
            Message::Text(payload) => payload.to_string(),
            message => panic!("expected text, received {message:?}"),
        }
    }

    async fn authenticate<S>(socket: &mut tokio_tungstenite::WebSocketStream<S>)
    where
        S: tokio::io::AsyncRead + tokio::io::AsyncWrite + Unpin,
    {
        let login: Value = serde_json::from_str(&next_text(socket).await).unwrap();
        assert_eq!(login["op"], "login");
        socket
            .send(Message::Text(
                r#"{"event":"login","code":"0","msg":""}"#.into(),
            ))
            .await
            .unwrap();
    }

    async fn ready(
        status: &mut mpsc::Receiver<OrderCommandWebsocketStatus>,
    ) -> OrderCommandWebsocketStatus {
        tokio::time::timeout(Duration::from_secs(2), async {
            loop {
                let status = status.recv().await.expect("aggregate status");
                if status.kind == OrderCommandWebsocketStatusKind::Ready {
                    return status;
                }
            }
        })
        .await
        .expect("order websocket should become ready")
    }

    fn pending_request() -> (
        HashMap<String, PendingRequest>,
        oneshot::Receiver<Result<OkxOrderAck, OrderTransportError>>,
    ) {
        let (response, receiver) = oneshot::channel();
        let pending = HashMap::from([(
            "request1".to_string(),
            PendingRequest {
                operation: OkxWsOrderOperation::Place,
                expected_account_id: "account-a".to_string(),
                expected_symbol: "BTC-USDT-SWAP".to_string(),
                expected_client_order_id: "client-a".to_string(),
                acknowledgement_deadline: Instant::now() + Duration::from_secs(1),
                response,
            },
        )]);
        (pending, receiver)
    }

    fn accepted_response(row: Value) -> String {
        serde_json::json!({
            "id": "request1",
            "op": OKX_WS_PLACE_ORDER_OP,
            "code": "0",
            "msg": "",
            "data": [row],
        })
        .to_string()
    }

    #[test]
    fn command_config_rejects_zero_capacity_timeouts_and_invalid_reconnect() {
        let build = |capacity, request_expiry, acknowledgement_timeout, reconnect| {
            OrderCommandWebsocketConfig::new(
                "account-a",
                "ws://127.0.0.1:1/ws/v5/private",
                capacity,
                request_expiry,
                acknowledgement_timeout,
                ConnectionAttemptPacer::new(Duration::ZERO),
                reconnect,
            )
        };
        let reconnect = ReconnectPolicy {
            initial_delay: Duration::from_millis(10),
            max_delay: Duration::from_millis(20),
            multiplier: 2,
        };

        for error in [
            build(
                0,
                Duration::from_secs(1),
                Duration::from_secs(1),
                reconnect.clone(),
            )
            .err()
            .expect("zero capacity must be rejected"),
            build(1, Duration::ZERO, Duration::from_secs(1), reconnect.clone())
                .err()
                .expect("zero request expiry must be rejected"),
            build(1, Duration::from_secs(1), Duration::ZERO, reconnect.clone())
                .err()
                .expect("zero acknowledgement timeout must be rejected"),
            build(
                1,
                Duration::from_secs(1),
                Duration::from_secs(1),
                ReconnectPolicy {
                    initial_delay: Duration::from_millis(20),
                    max_delay: Duration::from_millis(10),
                    multiplier: 2,
                },
            )
            .err()
            .expect("reversed reconnect delays must be rejected"),
            build(
                1,
                Duration::from_secs(1),
                Duration::from_secs(1),
                ReconnectPolicy {
                    initial_delay: Duration::from_millis(10),
                    max_delay: Duration::from_millis(20),
                    multiplier: 0,
                },
            )
            .err()
            .expect("zero reconnect multiplier must be rejected"),
        ] {
            assert!(matches!(error, AdapterError::InvalidConfiguration(_)));
        }
    }

    #[tokio::test]
    async fn shared_command_transport_slot_installs_exactly_once_before_spawn() {
        let slot = Arc::new(OrderCommandTransportSlot::default());
        let first_slot = Arc::clone(&slot);
        let (lifecycle, _status) = start_order_command_websocket(
            Arc::new(TestSessionOperations),
            config("ws://127.0.0.1:1/ws/v5/private".to_string()),
            move |transport| first_slot.install(transport.clone()),
        )
        .unwrap();

        let second_slot = Arc::clone(&slot);
        let error = start_order_command_websocket(
            Arc::new(TestSessionOperations),
            config("ws://127.0.0.1:1/ws/v5/private".to_string()),
            move |transport| second_slot.install(transport.clone()),
        )
        .err()
        .expect("the shared command transport slot must reject a second install");

        assert!(error.to_string().contains("already installed"), "{error}");
        lifecycle.shutdown().await.unwrap();
    }

    #[tokio::test]
    async fn mismatching_acknowledgement_identity_is_ambiguous_and_disconnects() {
        for (field, actual, expected_dimension) in [
            ("accountId", serde_json::json!("account-b"), "account"),
            ("accountId", serde_json::json!("0"), "account"),
            ("acctId", serde_json::json!(7), "account"),
            ("instId", serde_json::json!("ETH-USDT-SWAP"), "symbol"),
            ("instId", serde_json::json!("0"), "symbol"),
            ("symbol", serde_json::json!(7), "symbol"),
            ("clOrdId", serde_json::json!("client-b"), "client order id"),
            ("clientOrderId", serde_json::json!(7), "client order id"),
        ] {
            let (mut pending, receiver) = pending_request();
            let mut row = serde_json::json!({
                "ordId": "42",
                "clOrdId": "client-a",
                "sCode": "0",
                "sMsg": "",
            });
            row[field] = actual;

            let error = process_order_response(&accepted_response(row), &mut pending).unwrap_err();
            assert!(error.contains(expected_dimension), "{error}");
            assert!(pending.is_empty());
            assert!(matches!(
                receiver.await.unwrap(),
                Err(OrderTransportError::Ambiguous(message))
                    if message.contains(expected_dimension)
            ));
        }
    }

    #[tokio::test]
    async fn client_placeholder_acknowledgement_normalizes_to_expected_client_id() {
        for client_order_id in ["", "0"] {
            let (mut pending, receiver) = pending_request();
            let response = accepted_response(serde_json::json!({
                "ordId": "42",
                "clOrdId": client_order_id,
                "sCode": "0",
                "sMsg": "",
            }));

            process_order_response(&response, &mut pending).unwrap();
            let acknowledgement = receiver.await.unwrap().unwrap();
            assert_eq!(acknowledgement.exchange_order_id, "42");
            assert_eq!(acknowledgement.client_order_id, "client-a");
        }
    }

    #[tokio::test]
    async fn absent_or_empty_optional_account_and_symbol_aliases_are_tolerated() {
        for optional_identity in [
            serde_json::json!({}),
            serde_json::json!({"accountId": "", "instId": ""}),
        ] {
            let (mut pending, receiver) = pending_request();
            let mut row = serde_json::json!({
                "ordId": "42",
                "clOrdId": "client-a",
                "sCode": "0",
                "sMsg": "",
            });
            row.as_object_mut()
                .unwrap()
                .extend(optional_identity.as_object().unwrap().clone());

            process_order_response(&accepted_response(row), &mut pending).unwrap();
            assert_eq!(
                receiver.await.unwrap().unwrap(),
                OkxOrderAck {
                    exchange_order_id: "42".to_string(),
                    client_order_id: "client-a".to_string(),
                }
            );
        }
    }

    #[tokio::test]
    async fn a_non_string_top_level_identity_alias_is_ambiguous() {
        let (mut pending, receiver) = pending_request();
        let mut response: Value = serde_json::from_str(&accepted_response(serde_json::json!({
            "ordId": "42",
            "clOrdId": "client-a",
            "sCode": "0",
            "sMsg": "",
        })))
        .unwrap();
        response["account_id"] = serde_json::json!(7);

        let error = process_order_response(&response.to_string(), &mut pending).unwrap_err();
        assert!(error.contains("account"), "{error}");
        assert!(matches!(
            receiver.await.unwrap(),
            Err(OrderTransportError::Ambiguous(message)) if message.contains("must be a string")
        ));
    }

    #[tokio::test]
    async fn zero_or_untrimmed_exchange_order_id_is_ambiguous_and_disconnects() {
        for exchange_order_id in ["0", " 42", "42 "] {
            let (mut pending, receiver) = pending_request();
            let response = accepted_response(serde_json::json!({
                "ordId": exchange_order_id,
                "clOrdId": "client-a",
                "sCode": "0",
                "sMsg": "",
            }));

            let error = process_order_response(&response, &mut pending).unwrap_err();
            assert!(error.contains("invalid exchange order id"), "{error}");
            assert!(pending.is_empty());
            assert!(matches!(
                receiver.await.unwrap(),
                Err(OrderTransportError::Ambiguous(message))
                    if message.contains("invalid exchange order id")
            ));
        }
    }

    #[test]
    fn session_heartbeat_is_best_effort_under_status_backpressure() {
        let (status, mut receiver) = mpsc::channel(1);
        status
            .try_send(SessionStatus {
                index: 0,
                kind: OrderCommandWebsocketStatusKind::Ready,
                reason: "authenticated".to_string(),
            })
            .unwrap();

        try_send_heartbeat(0, &status).unwrap();
        assert_eq!(
            receiver.try_recv().unwrap().kind,
            OrderCommandWebsocketStatusKind::Ready
        );
        drop(receiver);
        assert_eq!(
            try_send_heartbeat(0, &status).unwrap_err(),
            "order websocket status monitor closed"
        );
    }

    #[tokio::test]
    async fn aggregate_heartbeat_cannot_displace_a_disconnect_transition() {
        let (session_tx, session_rx) = mpsc::channel(4);
        let (output_tx, mut output_rx) = mpsc::channel(1);
        let output_probe = output_tx.clone();
        let (shutdown_tx, shutdown_rx) = watch::channel(false);
        let task = tokio::spawn(aggregate_status(
            "account-a".to_string(),
            1,
            session_rx,
            output_tx,
            shutdown_rx,
        ));

        session_tx
            .send(SessionStatus {
                index: 0,
                kind: OrderCommandWebsocketStatusKind::Ready,
                reason: "authenticated".to_string(),
            })
            .await
            .unwrap();
        tokio::time::timeout(Duration::from_secs(1), async {
            while output_probe.capacity() != 0 || session_tx.capacity() != 4 {
                tokio::task::yield_now().await;
            }
        })
        .await
        .unwrap();
        assert_eq!(output_probe.capacity(), 0);

        session_tx
            .send(SessionStatus {
                index: 0,
                kind: OrderCommandWebsocketStatusKind::Heartbeat,
                reason: "pong received".to_string(),
            })
            .await
            .unwrap();
        tokio::time::timeout(Duration::from_secs(1), async {
            while session_tx.capacity() != 4 {
                tokio::task::yield_now().await;
            }
        })
        .await
        .unwrap();
        assert_eq!(session_tx.capacity(), 4);
        assert_eq!(output_probe.capacity(), 0);

        session_tx
            .send(SessionStatus {
                index: 0,
                kind: OrderCommandWebsocketStatusKind::Disconnected,
                reason: "transport lost".to_string(),
            })
            .await
            .unwrap();

        assert_eq!(
            output_rx.recv().await.unwrap().kind,
            OrderCommandWebsocketStatusKind::Ready
        );
        let disconnected = tokio::time::timeout(Duration::from_secs(1), output_rx.recv())
            .await
            .unwrap()
            .unwrap();
        assert_eq!(
            disconnected.kind,
            OrderCommandWebsocketStatusKind::Disconnected
        );
        shutdown_tx.send(true).unwrap();
        task.await.unwrap();
    }

    #[tokio::test]
    async fn aggregate_status_propagates_a_fatal_session_failure_and_stops() {
        let (session_tx, session_rx) = mpsc::channel(1);
        let (output_tx, mut output_rx) = mpsc::channel(1);
        let (_shutdown_tx, shutdown_rx) = watch::channel(false);
        let task = tokio::spawn(aggregate_status(
            "account-a".to_string(),
            2,
            session_rx,
            output_tx,
            shutdown_rx,
        ));

        session_tx
            .send(SessionStatus {
                index: 1,
                kind: OrderCommandWebsocketStatusKind::Fatal,
                reason: "connection pacer failed".to_string(),
            })
            .await
            .unwrap();

        let status = output_rx.recv().await.unwrap();
        assert_eq!(status.kind, OrderCommandWebsocketStatusKind::Fatal);
        assert_eq!(status.ready_sessions, 0);
        assert!(status.reason.contains("connection pacer failed"));
        task.await.unwrap();
    }

    #[tokio::test]
    async fn authenticated_place_and_cancel_are_correlated() {
        let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
        let url = format!("ws://{}", listener.local_addr().unwrap());
        let server = tokio::spawn(async move {
            let (stream, _) = listener.accept().await.unwrap();
            let mut socket = accept_async(stream).await.unwrap();
            authenticate(&mut socket).await;

            let place: Value = serde_json::from_str(&next_text(&mut socket).await).unwrap();
            assert_eq!(place["op"], OKX_WS_PLACE_ORDER_OP);
            assert!(place["expTime"].as_str().unwrap().parse::<u64>().unwrap() > unix_time_ms());
            let place_id = place["id"].as_str().unwrap();
            socket
                .send(Message::Text(
                    serde_json::json!({
                        "id": place_id,
                        "op": OKX_WS_PLACE_ORDER_OP,
                        "code": "0",
                        "msg": "",
                        "data": [{"ordId": "42", "clOrdId": "reap1", "sCode": "0", "sMsg": ""}]
                    })
                    .to_string()
                    .into(),
                ))
                .await
                .unwrap();

            let cancel: Value = serde_json::from_str(&next_text(&mut socket).await).unwrap();
            assert_eq!(cancel["op"], OKX_WS_CANCEL_ORDER_OP);
            assert!(cancel.get("expTime").is_none());
            let cancel_id = cancel["id"].as_str().unwrap();
            socket
                .send(Message::Text(
                    serde_json::json!({
                        "id": cancel_id,
                        "op": OKX_WS_CANCEL_ORDER_OP,
                        "code": "0",
                        "msg": "",
                        "data": [{"ordId": "42", "clOrdId": "reap1", "sCode": "0", "sMsg": ""}]
                    })
                    .to_string()
                    .into(),
                ))
                .await
                .unwrap();
        });

        let (transport, runtime, mut status) = start_test_order_command_websocket(config(url));
        let ready = ready(&mut status).await;
        assert_eq!(ready.ready_sessions, 1);
        let acknowledgement = transport.place_test_order().await.unwrap();
        assert_eq!(acknowledgement.exchange_order_id, "42");
        let cancellation = transport.cancel_test_order().await.unwrap();
        assert_eq!(cancellation.client_order_id, "reap1");
        runtime.shutdown().await.unwrap();
        server.await.unwrap();
    }

    #[tokio::test]
    async fn disconnect_after_write_is_ambiguous() {
        let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
        let url = format!("ws://{}", listener.local_addr().unwrap());
        let server = tokio::spawn(async move {
            let (stream, _) = listener.accept().await.unwrap();
            let mut socket = accept_async(stream).await.unwrap();
            authenticate(&mut socket).await;
            let request: Value = serde_json::from_str(&next_text(&mut socket).await).unwrap();
            assert_eq!(request["op"], OKX_WS_PLACE_ORDER_OP);
            socket.close(None).await.unwrap();
        });

        let (transport, runtime, mut status) = start_test_order_command_websocket(config(url));
        ready(&mut status).await;
        let error = transport.place_test_order().await.unwrap_err();
        assert!(error.is_ambiguous(), "{error}");
        runtime.shutdown().await.unwrap();
        server.await.unwrap();
    }

    #[tokio::test]
    async fn unauthenticated_session_rejects_before_send() {
        let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
        let url = format!("ws://{}", listener.local_addr().unwrap());
        drop(listener);
        let (transport, runtime, _status) = start_test_order_command_websocket(config(url));

        let error = transport.place_test_order().await.unwrap_err();
        assert!(error.is_unavailable(), "{error}");
        runtime.shutdown().await.unwrap();
    }

    #[tokio::test]
    async fn shutdown_interrupts_a_stalled_websocket_handshake() {
        let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
        let url = format!("ws://{}", listener.local_addr().unwrap());
        let (accepted_tx, accepted_rx) = oneshot::channel();
        let server = tokio::spawn(async move {
            let (stream, _) = listener.accept().await.unwrap();
            let _ = accepted_tx.send(());
            let _stream = stream;
            std::future::pending::<()>().await;
        });

        let (_transport, runtime, _status) = start_test_order_command_websocket(config(url));
        tokio::time::timeout(Duration::from_secs(1), accepted_rx)
            .await
            .expect("client should connect")
            .expect("server should report the connection");
        tokio::time::timeout(Duration::from_secs(1), runtime.shutdown())
            .await
            .expect("shutdown should interrupt the handshake")
            .unwrap();
        server.abort();
        let _ = server.await;
    }

    #[tokio::test]
    async fn configured_authority_uses_exactly_one_session() {
        let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
        let url = format!("ws://{}", listener.local_addr().unwrap());
        let server = tokio::spawn(async move {
            let (stream, _) = listener.accept().await.unwrap();
            let mut socket = accept_async(stream).await.unwrap();
            authenticate(&mut socket).await;
            while socket.next().await.is_some() {}
        });

        let (_transport, runtime, mut status) = start_test_order_command_websocket(config(url));
        let ready = ready(&mut status).await;
        assert_eq!(ready.ready_sessions, 1);
        assert_eq!(ready.total_sessions, 1);
        runtime.shutdown().await.unwrap();
        server.await.unwrap();
    }
}
