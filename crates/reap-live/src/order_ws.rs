use std::collections::{HashMap, HashSet};
use std::sync::Arc;
use std::sync::atomic::{AtomicU64, Ordering};
use std::time::{Duration, Instant, SystemTime, UNIX_EPOCH};

use async_trait::async_trait;
use futures_util::{SinkExt, StreamExt};
use reap_feed::{ConnectionAttemptPacer, ReconnectPolicy};
use reap_order::{OkxOrderTransport, OrderTransportError, okx_order_dispatch_key};
use reap_venue::okx::{
    OkxCancelOrder, OkxOrderAck, OkxPlaceOrder, OkxSigner, OkxWsOrderOperation, OkxWsOrderResult,
    build_okx_ws_cancel_order_request, build_okx_ws_place_order_request,
    parse_okx_ws_order_response,
};
use serde_json::Value;
use tokio::sync::{mpsc, oneshot, watch};
use tokio::task::{JoinError, JoinHandle};
use tokio_tungstenite::tungstenite::Message;

const LOGIN_TIMEOUT: Duration = Duration::from_secs(10);
const CONTROL_WRITE_TIMEOUT: Duration = Duration::from_secs(5);
const PING_INTERVAL: Duration = Duration::from_secs(15);
const IDLE_TIMEOUT: Duration = Duration::from_secs(30);
const EXPIRY_SCAN_INTERVAL: Duration = Duration::from_millis(10);

#[derive(Clone)]
pub(crate) struct OkxOrderWsConfig {
    pub account_id: String,
    pub websocket_url: String,
    pub signer: OkxSigner,
    pub session_count: usize,
    pub command_capacity: usize,
    pub request_expiry: Duration,
    pub acknowledgement_timeout: Duration,
    pub connection_attempt_pacer: ConnectionAttemptPacer,
    pub reconnect: ReconnectPolicy,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum OkxOrderWsStatusKind {
    Ready,
    Heartbeat,
    Disconnected,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct OkxOrderWsStatus {
    pub account_id: String,
    pub ts_ms: u64,
    pub kind: OkxOrderWsStatusKind,
    pub ready_sessions: usize,
    pub total_sessions: usize,
    pub reason: String,
}

#[derive(Clone)]
pub(crate) struct OkxOrderWsTransport {
    sessions: Arc<Vec<SessionHandle>>,
    request_sequence: Arc<AtomicU64>,
    request_expiry: Duration,
}

pub(crate) struct OkxOrderWsRuntime {
    shutdown: watch::Sender<bool>,
    tasks: Vec<JoinHandle<()>>,
}

impl OkxOrderWsRuntime {
    pub async fn shutdown(self) -> Result<(), JoinError> {
        let _ = self.shutdown.send(true);
        for task in self.tasks {
            task.await?;
        }
        Ok(())
    }
}

#[derive(Clone)]
struct SessionHandle {
    commands: mpsc::Sender<SessionCommand>,
    ready: watch::Receiver<bool>,
}

struct SessionCommand {
    request_id: String,
    operation: OkxWsOrderOperation,
    payload: String,
    send_deadline: Instant,
    response: oneshot::Sender<Result<OkxOrderAck, OrderTransportError>>,
}

struct PendingRequest {
    operation: OkxWsOrderOperation,
    acknowledgement_deadline: Instant,
    response: oneshot::Sender<Result<OkxOrderAck, OrderTransportError>>,
}

#[derive(Debug)]
struct SessionStatus {
    index: usize,
    kind: OkxOrderWsStatusKind,
    reason: String,
}

pub(crate) fn spawn_okx_order_ws(
    config: OkxOrderWsConfig,
) -> (
    OkxOrderWsTransport,
    OkxOrderWsRuntime,
    mpsc::Receiver<OkxOrderWsStatus>,
) {
    assert!(config.session_count > 0, "validated session count");
    let (shutdown_tx, shutdown_rx) = watch::channel(false);
    let (session_status_tx, session_status_rx) = mpsc::channel(config.session_count.max(1) * 4);
    let (aggregate_status_tx, aggregate_status_rx) = mpsc::channel(64);
    let mut handles = Vec::with_capacity(config.session_count);
    let mut tasks = Vec::with_capacity(config.session_count + 1);

    for index in 0..config.session_count {
        let (command_tx, command_rx) = mpsc::channel(config.command_capacity.max(1));
        let (ready_tx, ready_rx) = watch::channel(false);
        handles.push(SessionHandle {
            commands: command_tx,
            ready: ready_rx,
        });
        tasks.push(tokio::spawn(supervise_session(
            index,
            config.websocket_url.clone(),
            config.signer.clone(),
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
        config.session_count,
        session_status_rx,
        aggregate_status_tx,
        shutdown_rx,
    )));

    (
        OkxOrderWsTransport {
            sessions: Arc::new(handles),
            request_sequence: Arc::new(AtomicU64::new(unix_time_ns())),
            request_expiry: config.request_expiry,
        },
        OkxOrderWsRuntime {
            shutdown: shutdown_tx,
            tasks,
        },
        aggregate_status_rx,
    )
}

#[async_trait]
impl OkxOrderTransport for OkxOrderWsTransport {
    async fn place_order(&self, order: &OkxPlaceOrder) -> Result<OkxOrderAck, OrderTransportError> {
        let session_index = route_session(&order.symbol, self.sessions.len());
        let request_id = self.next_request_id(session_index);
        let expiry_ms = unix_time_ms().saturating_add(duration_ms(self.request_expiry));
        let payload = build_okx_ws_place_order_request(&request_id, expiry_ms, order)
            .map_err(|error| OrderTransportError::InvalidRequest(error.to_string()))?;
        self.execute(
            session_index,
            request_id,
            OkxWsOrderOperation::Place,
            payload,
        )
        .await
    }

    async fn cancel_order(
        &self,
        order: &OkxCancelOrder,
    ) -> Result<OkxOrderAck, OrderTransportError> {
        let session_index = route_session(&order.symbol, self.sessions.len());
        let request_id = self.next_request_id(session_index);
        let payload = build_okx_ws_cancel_order_request(&request_id, order)
            .map_err(|error| OrderTransportError::InvalidRequest(error.to_string()))?;
        self.execute(
            session_index,
            request_id,
            OkxWsOrderOperation::Cancel,
            payload,
        )
        .await
    }
}

impl OkxOrderWsTransport {
    fn next_request_id(&self, session_index: usize) -> String {
        let sequence = self.request_sequence.fetch_add(1, Ordering::Relaxed);
        format!("r{session_index:02x}{sequence:016x}")
    }

    async fn execute(
        &self,
        session_index: usize,
        request_id: String,
        operation: OkxWsOrderOperation,
        payload: String,
    ) -> Result<OkxOrderAck, OrderTransportError> {
        let session = &self.sessions[session_index];
        if !*session.ready.borrow() {
            return Err(OrderTransportError::Unavailable(format!(
                "session {session_index} is not authenticated"
            )));
        }
        let (response_tx, response_rx) = oneshot::channel();
        let command = SessionCommand {
            request_id,
            operation,
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
    signer: OkxSigner,
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
        if *shutdown.borrow() || !connection_attempt_pacer.wait_for_turn(&mut shutdown).await {
            reject_queued(&mut commands, "order websocket is shutting down");
            return;
        }
        let mut authenticated = false;
        let result = run_authenticated_session(
            index,
            &websocket_url,
            &signer,
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
                kind: OkxOrderWsStatusKind::Disconnected,
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
    signer: &OkxSigner,
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
    let login = signer
        .websocket_login(&unix_time_seconds().to_string())
        .map_err(|error| format!("login generation failed: {error}"))?;
    send_control_message(&mut writer, Message::Text(login.into()), "login").await?;
    await_login(&mut writer, &mut reader, shutdown).await?;
    let _ = ready.send(true);
    status
        .send(SessionStatus {
            index,
            kind: OkxOrderWsStatusKind::Ready,
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
                    let _ = writer.send(Message::Close(None)).await;
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
                            send_heartbeat(index, status).await?;
                        } else {
                            process_order_response(payload.as_str(), pending)?;
                        }
                    }
                    Message::Binary(payload) => {
                        let payload = std::str::from_utf8(payload.as_ref())
                            .map_err(|_| "order websocket returned non-UTF8 data".to_string())?;
                        if payload == "pong" {
                            send_heartbeat(index, status).await?;
                        } else {
                            process_order_response(payload, pending)?;
                        }
                    }
                    Message::Ping(payload) => {
                        send_control_message(writer, Message::Pong(payload), "pong").await?
                    }
                    Message::Pong(_) => send_heartbeat(index, status).await?,
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
    match result {
        OkxWsOrderResult::Accepted {
            acknowledgement, ..
        } => {
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

async fn send_heartbeat(index: usize, status: &mpsc::Sender<SessionStatus>) -> Result<(), String> {
    status
        .send(SessionStatus {
            index,
            kind: OkxOrderWsStatusKind::Heartbeat,
            reason: "pong received".to_string(),
        })
        .await
        .map_err(|_| "order websocket status monitor closed".to_string())
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
    output: mpsc::Sender<OkxOrderWsStatus>,
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
                    OkxOrderWsStatusKind::Ready | OkxOrderWsStatusKind::Heartbeat => {
                        ready_sessions.insert(status.index);
                    }
                    OkxOrderWsStatusKind::Disconnected => {
                        ready_sessions.remove(&status.index);
                    }
                }
                let all_ready = ready_sessions.len() == total_sessions;
                let aggregate_kind = match status.kind {
                    OkxOrderWsStatusKind::Disconnected
                        if aggregate_ready || !disconnected_reported =>
                    {
                        disconnected_reported = true;
                        Some(OkxOrderWsStatusKind::Disconnected)
                    }
                    OkxOrderWsStatusKind::Ready if all_ready && !aggregate_ready => {
                        disconnected_reported = false;
                        Some(OkxOrderWsStatusKind::Ready)
                    }
                    OkxOrderWsStatusKind::Heartbeat if all_ready => {
                        Some(OkxOrderWsStatusKind::Heartbeat)
                    }
                    _ => None,
                };
                aggregate_ready = all_ready;
                let Some(kind) = aggregate_kind else { continue; };
                let reason = match kind {
                    OkxOrderWsStatusKind::Ready => {
                        "every order websocket session is authenticated".to_string()
                    }
                    OkxOrderWsStatusKind::Heartbeat => {
                        "every order websocket session is authenticated and responsive".to_string()
                    }
                    OkxOrderWsStatusKind::Disconnected => format!(
                        "order websocket session {} disconnected: {}",
                        status.index, status.reason
                    ),
                };
                if output
                    .send(OkxOrderWsStatus {
                        account_id: account_id.clone(),
                        ts_ms: unix_time_ms(),
                        kind,
                        ready_sessions: ready_sessions.len(),
                        total_sessions,
                        reason,
                    })
                    .await
                    .is_err()
                {
                    return;
                }
            }
        }
    }
}

fn route_session(symbol: &str, session_count: usize) -> usize {
    let mut hash = 0xcbf2_9ce4_8422_2325_u64;
    for byte in okx_order_dispatch_key(symbol).bytes() {
        hash ^= u64::from(byte);
        hash = hash.wrapping_mul(0x0000_0100_0000_01b3);
    }
    (hash as usize) % session_count
}

fn unix_time_seconds() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap_or_default()
        .as_secs()
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
    use reap_core::{Side, TimeInForce};
    use reap_venue::okx::{
        OKX_WS_CANCEL_ORDER_OP, OKX_WS_PLACE_ORDER_OP, OkxCredentials, OkxTradeMode,
    };
    use tokio::net::TcpListener;
    use tokio_tungstenite::accept_async;

    use super::*;

    fn signer() -> OkxSigner {
        OkxSigner::new(OkxCredentials::new("key", "secret", "pass"), true)
    }

    fn config(url: String, session_count: usize) -> OkxOrderWsConfig {
        OkxOrderWsConfig {
            account_id: "account-a".to_string(),
            websocket_url: url,
            signer: signer(),
            session_count,
            command_capacity: 8,
            request_expiry: Duration::from_millis(1_000),
            acknowledgement_timeout: Duration::from_millis(500),
            connection_attempt_pacer: ConnectionAttemptPacer::new(Duration::ZERO),
            reconnect: ReconnectPolicy {
                initial_delay: Duration::from_millis(10),
                max_delay: Duration::from_millis(20),
                multiplier: 2,
            },
        }
    }

    fn place_order() -> OkxPlaceOrder {
        OkxPlaceOrder {
            symbol: "BTC-USDT-SWAP".to_string(),
            trade_mode: OkxTradeMode::Cross,
            side: Side::Buy,
            time_in_force: TimeInForce::PostOnly,
            price: 50_000.0,
            qty: 0.01,
            client_order_id: "reap1".to_string(),
            reduce_only: false,
            self_trade_prevention: None,
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

    async fn ready(status: &mut mpsc::Receiver<OkxOrderWsStatus>) -> OkxOrderWsStatus {
        tokio::time::timeout(Duration::from_secs(2), async {
            loop {
                let status = status.recv().await.expect("aggregate status");
                if status.kind == OkxOrderWsStatusKind::Ready {
                    return status;
                }
            }
        })
        .await
        .expect("order websocket should become ready")
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

        let (transport, runtime, mut status) = spawn_okx_order_ws(config(url, 1));
        let ready = ready(&mut status).await;
        assert_eq!(ready.ready_sessions, 1);
        let acknowledgement = transport.place_order(&place_order()).await.unwrap();
        assert_eq!(acknowledgement.exchange_order_id, "42");
        let cancellation = transport
            .cancel_order(&OkxCancelOrder {
                symbol: "BTC-USDT-SWAP".to_string(),
                exchange_order_id: None,
                client_order_id: Some("reap1".to_string()),
            })
            .await
            .unwrap();
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

        let (transport, runtime, mut status) = spawn_okx_order_ws(config(url, 1));
        ready(&mut status).await;
        let error = transport.place_order(&place_order()).await.unwrap_err();
        assert!(error.is_ambiguous(), "{error}");
        runtime.shutdown().await.unwrap();
        server.await.unwrap();
    }

    #[tokio::test]
    async fn unauthenticated_session_rejects_before_send() {
        let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
        let url = format!("ws://{}", listener.local_addr().unwrap());
        drop(listener);
        let (transport, runtime, _status) = spawn_okx_order_ws(config(url, 1));

        let error = transport.place_order(&place_order()).await.unwrap_err();
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

        let (_transport, runtime, _status) = spawn_okx_order_ws(config(url, 1));
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
    async fn aggregate_readiness_requires_every_configured_session() {
        let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
        let url = format!("ws://{}", listener.local_addr().unwrap());
        let server = tokio::spawn(async move {
            let mut handlers = Vec::new();
            for _ in 0..2 {
                let (stream, _) = listener.accept().await.unwrap();
                handlers.push(tokio::spawn(async move {
                    let mut socket = accept_async(stream).await.unwrap();
                    authenticate(&mut socket).await;
                    while socket.next().await.is_some() {}
                }));
            }
            for handler in handlers {
                handler.await.unwrap();
            }
        });

        let (_transport, runtime, mut status) = spawn_okx_order_ws(config(url, 2));
        let ready = ready(&mut status).await;
        assert_eq!(ready.ready_sessions, 2);
        assert_eq!(ready.total_sessions, 2);
        runtime.shutdown().await.unwrap();
        server.await.unwrap();
    }

    #[test]
    fn stable_underlying_dispatches_spot_swap_and_future_to_one_session() {
        let spot = route_session("BTC-USDT", 8);
        assert_eq!(route_session("BTC-USDT-SWAP", 8), spot);
        assert_eq!(route_session("BTC-USDT-260925", 8), spot);
        assert_ne!(route_session("ETH-USDT-SWAP", 8), spot);
    }
}
