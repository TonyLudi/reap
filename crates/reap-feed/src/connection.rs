use std::collections::{BTreeMap, BTreeSet};

use futures_util::{SinkExt, StreamExt};
use reap_core::{Channel, ConnId, RawEnvelope, Venue};
use reap_venue::{VenueAdapter, VenueError};
use thiserror::Error;
use tokio::sync::{mpsc, watch};
use tokio_tungstenite::{connect_async, tungstenite::Message};

use crate::{SocketPlan, payload_hash, unix_time_ns};

#[derive(Debug, Error)]
pub enum ConnectionError {
    #[error("websocket transport failed: {0}")]
    Websocket(#[source] Box<tokio_tungstenite::tungstenite::Error>),
    #[error("venue adapter failed: {0}")]
    Venue(#[from] VenueError),
    #[error("raw feed output channel is closed")]
    OutputClosed,
    #[error("feed connection-status channel is closed")]
    StatusClosed,
    #[error("public feed output channel is full; connection must recover")]
    Backpressure,
    #[error("websocket peer closed the connection")]
    PeerClosed,
    #[error("received non-UTF8 websocket payload")]
    NonUtf8Payload,
    #[error("private websocket requires a login bootstrap message")]
    MissingPrivateLogin,
    #[error("private websocket login timed out")]
    LoginTimeout,
    #[error("private websocket login failed: {0}")]
    LoginFailed(String),
    #[error("connection restart requested for snapshot recovery")]
    RecoveryRequested,
    #[error("connection shutdown requested")]
    ShutdownRequested,
    #[error("websocket received no data before the idle timeout")]
    IdleTimeout,
    #[error("websocket subscription acknowledgement timed out")]
    SubscriptionTimeout,
    #[error("websocket subscription failed: {0}")]
    SubscriptionFailed(String),
    #[error("invalid websocket subscription plan: {0}")]
    InvalidSubscriptionPlan(String),
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ConnectionStatusKind {
    Ready,
    Heartbeat,
    Disconnected,
    Fatal,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ConnectionStatus {
    pub conn_id: ConnId,
    pub venue: Venue,
    pub private: bool,
    pub ts_ms: u64,
    pub kind: ConnectionStatusKind,
    pub reason: String,
}

impl From<tokio_tungstenite::tungstenite::Error> for ConnectionError {
    fn from(error: tokio_tungstenite::tungstenite::Error) -> Self {
        Self::Websocket(Box::new(error))
    }
}

pub async fn run_connection_once(
    adapter: &dyn VenueAdapter,
    plan: &SocketPlan,
    bootstrap_messages: &[String],
    output: &mpsc::Sender<RawEnvelope>,
    status: &mpsc::Sender<ConnectionStatus>,
    shutdown: &mut watch::Receiver<bool>,
    recovery: &mut watch::Receiver<u64>,
) -> Result<(), ConnectionError> {
    if plan.private && bootstrap_messages.is_empty() {
        return Err(ConnectionError::MissingPrivateLogin);
    }
    let subscription = adapter.subscription_message(&plan.subscriptions)?;
    let subscription_readiness =
        SubscriptionReadiness::from_request(&subscription, plan.subscriptions.len())?;
    let (socket, _) = connect_async(adapter.websocket_url(plan.private)).await?;
    let (mut writer, mut reader) = socket.split();

    if plan.private {
        for message in bootstrap_messages {
            writer.send(Message::Text(message.clone().into())).await?;
        }
        await_private_login(&mut writer, &mut reader, shutdown, recovery).await?;
    }
    writer.send(Message::Text(subscription.into())).await?;
    await_subscriptions(
        &mut writer,
        &mut reader,
        subscription_readiness,
        plan,
        output,
        shutdown,
        recovery,
    )
    .await?;
    tokio::select! {
        result = send_status(
            status,
            plan,
            ConnectionStatusKind::Ready,
            "subscriptions acknowledged",
        ) => result?,
        changed = shutdown.changed() => {
            if changed.is_err() || *shutdown.borrow() {
                return Err(ConnectionError::ShutdownRequested);
            }
        }
    }

    let mut ping = tokio::time::interval(std::time::Duration::from_secs(15));
    ping.set_missed_tick_behavior(tokio::time::MissedTickBehavior::Delay);
    ping.tick().await;
    let mut last_received = tokio::time::Instant::now();

    loop {
        tokio::select! {
            _ = ping.tick() => {
                if last_received.elapsed() > std::time::Duration::from_secs(30) {
                    return Err(ConnectionError::IdleTimeout);
                }
                writer.send(Message::Text("ping".into())).await?;
            }
            changed = shutdown.changed() => {
                if changed.is_err() || *shutdown.borrow() {
                    let _ = writer.send(Message::Close(None)).await;
                    return Ok(());
                }
            }
            changed = recovery.changed() => {
                if changed.is_ok() {
                    let _ = writer.send(Message::Close(None)).await;
                    return Err(ConnectionError::RecoveryRequested);
                }
            }
            message = reader.next() => {
                let message = message.ok_or(ConnectionError::PeerClosed)??;
                last_received = tokio::time::Instant::now();
                match message {
                    Message::Text(payload) => {
                        if payload.as_str() != "pong" {
                            reject_server_error(payload.as_str())?;
                            forward_payload(payload.as_str(), plan, output).await?;
                        }
                    }
                    Message::Binary(payload) => {
                        let payload = std::str::from_utf8(payload.as_ref())
                            .map_err(|_| ConnectionError::NonUtf8Payload)?;
                        reject_server_error(payload)?;
                        forward_payload(payload, plan, output).await?;
                    }
                    Message::Ping(payload) => writer.send(Message::Pong(payload)).await?,
                    Message::Pong(_) => {}
                    Message::Close(_) => return Err(ConnectionError::PeerClosed),
                    Message::Frame(_) => {}
                }
            }
        }
    }
}

async fn await_subscriptions<S>(
    writer: &mut futures_util::stream::SplitSink<tokio_tungstenite::WebSocketStream<S>, Message>,
    reader: &mut futures_util::stream::SplitStream<tokio_tungstenite::WebSocketStream<S>>,
    mut readiness: SubscriptionReadiness,
    plan: &SocketPlan,
    output: &mpsc::Sender<RawEnvelope>,
    shutdown: &mut watch::Receiver<bool>,
    recovery: &mut watch::Receiver<u64>,
) -> Result<(), ConnectionError>
where
    S: tokio::io::AsyncRead + tokio::io::AsyncWrite + Unpin,
{
    if readiness.is_complete() {
        return Ok(());
    }
    let deadline = tokio::time::sleep(std::time::Duration::from_secs(10));
    tokio::pin!(deadline);
    loop {
        tokio::select! {
            _ = &mut deadline => return Err(ConnectionError::SubscriptionTimeout),
            changed = shutdown.changed() => {
                if changed.is_err() || *shutdown.borrow() {
                    return Err(ConnectionError::ShutdownRequested);
                }
            }
            changed = recovery.changed() => {
                if changed.is_ok() {
                    return Err(ConnectionError::RecoveryRequested);
                }
            }
            message = reader.next() => {
                let message = message.ok_or(ConnectionError::PeerClosed)??;
                match message {
                    Message::Text(payload) => {
                        match subscription_message(payload.as_str())? {
                            SubscriptionMessage::Acknowledged(identity) => {
                                readiness.acknowledge(identity)?;
                            }
                            SubscriptionMessage::Data => {
                                forward_payload(payload.as_str(), plan, output).await?;
                            }
                            SubscriptionMessage::Ignore => {}
                        }
                    }
                    Message::Binary(payload) => {
                        let payload = std::str::from_utf8(payload.as_ref())
                            .map_err(|_| ConnectionError::NonUtf8Payload)?;
                        match subscription_message(payload)? {
                            SubscriptionMessage::Acknowledged(identity) => {
                                readiness.acknowledge(identity)?;
                            }
                            SubscriptionMessage::Data => forward_payload(payload, plan, output).await?,
                            SubscriptionMessage::Ignore => {}
                        }
                    }
                    Message::Ping(payload) => writer.send(Message::Pong(payload)).await?,
                    Message::Pong(_) => {}
                    Message::Close(_) => return Err(ConnectionError::PeerClosed),
                    Message::Frame(_) => {}
                }
                if readiness.is_complete() {
                    return Ok(());
                }
            }
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord)]
struct SubscriptionIdentity {
    arguments: BTreeMap<String, String>,
}

impl SubscriptionIdentity {
    fn from_argument(value: &serde_json::Value) -> Result<Self, String> {
        let argument = value
            .as_object()
            .ok_or_else(|| "subscription argument is not an object".to_string())?;
        let mut arguments = BTreeMap::new();
        for (field, value) in argument {
            let value = value
                .as_str()
                .filter(|value| !value.is_empty())
                .ok_or_else(|| {
                    format!("subscription argument field {field} is not a non-empty string")
                })?;
            arguments.insert(field.clone(), value.to_string());
        }
        if !arguments.contains_key("channel") {
            return Err("subscription argument is missing channel".to_string());
        }
        Ok(Self { arguments })
    }

    fn from_acknowledgement(value: &serde_json::Value) -> Result<Self, ConnectionError> {
        let arg = value.get("arg").ok_or_else(|| {
            ConnectionError::SubscriptionFailed(
                "successful subscription acknowledgement is missing arg".to_string(),
            )
        })?;
        Self::from_argument(arg).map_err(ConnectionError::SubscriptionFailed)
    }

    fn label(&self) -> String {
        let channel = self
            .arguments
            .get("channel")
            .expect("validated subscription identity must contain channel");
        let mut label = self
            .arguments
            .get("instId")
            .map_or_else(|| channel.clone(), |symbol| format!("{channel}/{symbol}"));
        let selectors = self
            .arguments
            .iter()
            .filter(|(field, _)| !matches!(field.as_str(), "channel" | "instId"))
            .map(|(field, value)| format!("{field}={value}"))
            .collect::<Vec<_>>();
        if !selectors.is_empty() {
            label.push_str(&format!("[{}]", selectors.join(",")));
        }
        label
    }
}

struct SubscriptionReadiness {
    expected: BTreeSet<SubscriptionIdentity>,
    acknowledged: BTreeSet<SubscriptionIdentity>,
}

impl SubscriptionReadiness {
    fn from_request(payload: &str, planned_count: usize) -> Result<Self, ConnectionError> {
        if planned_count == 0 {
            return Err(ConnectionError::InvalidSubscriptionPlan(
                "socket plan has no subscriptions".to_string(),
            ));
        }
        let request: serde_json::Value = serde_json::from_str(payload).map_err(|error| {
            ConnectionError::InvalidSubscriptionPlan(format!(
                "subscription request is not valid JSON: {error}"
            ))
        })?;
        if request.get("op").and_then(serde_json::Value::as_str) != Some("subscribe") {
            return Err(ConnectionError::InvalidSubscriptionPlan(
                "subscription request has no subscribe operation".to_string(),
            ));
        }
        let arguments = request
            .get("args")
            .and_then(serde_json::Value::as_array)
            .ok_or_else(|| {
                ConnectionError::InvalidSubscriptionPlan(
                    "subscription request has no argument array".to_string(),
                )
            })?;
        if arguments.len() != planned_count {
            return Err(ConnectionError::InvalidSubscriptionPlan(format!(
                "subscription request contains {} arguments for {planned_count} planned subscriptions",
                arguments.len()
            )));
        }
        let mut expected = BTreeSet::new();
        for argument in arguments {
            let identity = SubscriptionIdentity::from_argument(argument).map_err(|error| {
                ConnectionError::InvalidSubscriptionPlan(format!(
                    "invalid subscription request argument: {error}"
                ))
            })?;
            if !expected.insert(identity.clone()) {
                return Err(ConnectionError::InvalidSubscriptionPlan(format!(
                    "socket plan repeats subscription {}",
                    identity.label()
                )));
            }
        }
        Ok(Self {
            expected,
            acknowledged: BTreeSet::new(),
        })
    }

    fn acknowledge(&mut self, identity: SubscriptionIdentity) -> Result<(), ConnectionError> {
        if !self.expected.contains(&identity) {
            return Err(ConnectionError::SubscriptionFailed(format!(
                "unexpected subscription acknowledgement {}",
                identity.label()
            )));
        }
        self.acknowledged.insert(identity);
        Ok(())
    }

    fn is_complete(&self) -> bool {
        self.acknowledged == self.expected
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
enum SubscriptionMessage {
    Acknowledged(SubscriptionIdentity),
    Data,
    Ignore,
}

fn subscription_message(payload: &str) -> Result<SubscriptionMessage, ConnectionError> {
    if payload == "pong" {
        return Ok(SubscriptionMessage::Ignore);
    }
    let value: serde_json::Value = serde_json::from_str(payload)
        .map_err(|error| ConnectionError::SubscriptionFailed(error.to_string()))?;
    match value.get("event").and_then(serde_json::Value::as_str) {
        Some("subscribe") if successful_optional_code(&value) => Ok(
            SubscriptionMessage::Acknowledged(SubscriptionIdentity::from_acknowledgement(&value)?),
        ),
        Some("error") | Some("channel-conn-count-error") => {
            Err(ConnectionError::SubscriptionFailed(server_error(&value)))
        }
        Some("subscribe") | Some("notice") => {
            Err(ConnectionError::SubscriptionFailed(server_error(&value)))
        }
        Some(_) => Ok(SubscriptionMessage::Ignore),
        None => Ok(SubscriptionMessage::Data),
    }
}

fn successful_optional_code(value: &serde_json::Value) -> bool {
    match value.get("code") {
        None | Some(serde_json::Value::Null) => true,
        Some(serde_json::Value::String(code)) => code == "0",
        Some(_) => false,
    }
}

fn reject_server_error(payload: &str) -> Result<(), ConnectionError> {
    let Ok(value) = serde_json::from_str::<serde_json::Value>(payload) else {
        return Ok(());
    };
    match value.get("event").and_then(serde_json::Value::as_str) {
        Some("error") | Some("channel-conn-count-error") | Some("notice") => {
            Err(ConnectionError::SubscriptionFailed(server_error(&value)))
        }
        _ => Ok(()),
    }
}

fn server_error(value: &serde_json::Value) -> String {
    format!(
        "event={} code={} message={}",
        value
            .get("event")
            .and_then(serde_json::Value::as_str)
            .unwrap_or(""),
        value
            .get("code")
            .and_then(serde_json::Value::as_str)
            .unwrap_or(""),
        value
            .get("msg")
            .and_then(serde_json::Value::as_str)
            .unwrap_or("")
    )
}

async fn send_status(
    status: &mpsc::Sender<ConnectionStatus>,
    plan: &SocketPlan,
    kind: ConnectionStatusKind,
    reason: &str,
) -> Result<(), ConnectionError> {
    status
        .send(ConnectionStatus {
            conn_id: plan.conn_id.clone(),
            venue: plan.venue,
            private: plan.private,
            ts_ms: unix_time_ns() / 1_000_000,
            kind,
            reason: reason.to_string(),
        })
        .await
        .map_err(|_| ConnectionError::StatusClosed)
}

async fn await_private_login<S>(
    writer: &mut futures_util::stream::SplitSink<tokio_tungstenite::WebSocketStream<S>, Message>,
    reader: &mut futures_util::stream::SplitStream<tokio_tungstenite::WebSocketStream<S>>,
    shutdown: &mut watch::Receiver<bool>,
    recovery: &mut watch::Receiver<u64>,
) -> Result<(), ConnectionError>
where
    S: tokio::io::AsyncRead + tokio::io::AsyncWrite + Unpin,
{
    let deadline = tokio::time::sleep(std::time::Duration::from_secs(10));
    tokio::pin!(deadline);
    loop {
        tokio::select! {
            _ = &mut deadline => return Err(ConnectionError::LoginTimeout),
            changed = shutdown.changed() => {
                if changed.is_err() || *shutdown.borrow() {
                    return Err(ConnectionError::ShutdownRequested);
                }
            }
            changed = recovery.changed() => {
                if changed.is_ok() {
                    return Err(ConnectionError::RecoveryRequested);
                }
            }
            message = reader.next() => {
                let message = message.ok_or(ConnectionError::PeerClosed)??;
                match message {
                    Message::Text(payload) => {
                        if login_response(payload.as_str())? {
                            return Ok(());
                        }
                    }
                    Message::Binary(payload) => {
                        let payload = std::str::from_utf8(payload.as_ref())
                            .map_err(|_| ConnectionError::NonUtf8Payload)?;
                        if login_response(payload)? {
                            return Ok(());
                        }
                    }
                    Message::Ping(payload) => writer.send(Message::Pong(payload)).await?,
                    Message::Pong(_) => {}
                    Message::Close(_) => return Err(ConnectionError::PeerClosed),
                    Message::Frame(_) => {}
                }
            }
        }
    }
}

fn login_response(payload: &str) -> Result<bool, ConnectionError> {
    let value: serde_json::Value = serde_json::from_str(payload)
        .map_err(|error| ConnectionError::LoginFailed(error.to_string()))?;
    match value.get("event").and_then(serde_json::Value::as_str) {
        Some("login") if value.get("code").and_then(serde_json::Value::as_str) == Some("0") => {
            Ok(true)
        }
        Some("login") | Some("error") => Err(ConnectionError::LoginFailed(format!(
            "code={} message={}",
            value
                .get("code")
                .and_then(serde_json::Value::as_str)
                .unwrap_or(""),
            value
                .get("msg")
                .and_then(serde_json::Value::as_str)
                .unwrap_or("")
        ))),
        _ => Ok(false),
    }
}

async fn forward_payload(
    payload: &str,
    plan: &SocketPlan,
    output: &mpsc::Sender<RawEnvelope>,
) -> Result<(), ConnectionError> {
    let channel = plan
        .subscriptions
        .first()
        .map(|subscription| subscription.channel.clone())
        .unwrap_or_else(|| Channel::Custom("unsubscribed".to_string()));
    let symbol = (plan.subscriptions.len() == 1)
        .then(|| plan.subscriptions[0].symbol.clone())
        .flatten();
    let envelope = RawEnvelope {
        venue: plan.venue,
        conn_id: plan.conn_id.clone(),
        channel,
        symbol,
        recv_ts_ns: unix_time_ns(),
        raw_hash: payload_hash(payload.as_bytes()),
        payload: payload.to_string(),
    };

    if plan.private {
        output
            .send(envelope)
            .await
            .map_err(|_| ConnectionError::OutputClosed)
    } else {
        output.try_send(envelope).map_err(|error| match error {
            mpsc::error::TrySendError::Full(_) => ConnectionError::Backpressure,
            mpsc::error::TrySendError::Closed(_) => ConnectionError::OutputClosed,
        })
    }
}

#[cfg(test)]
mod tests {
    use reap_core::{Channel, FeedPriority, Subscription};
    use tokio::net::TcpListener;
    use tokio::sync::oneshot;
    use tokio_tungstenite::{accept_async, tungstenite::Message};

    use super::*;

    fn readiness_for(
        subscriptions: &[Subscription],
    ) -> Result<SubscriptionReadiness, ConnectionError> {
        let adapter = reap_venue::okx::OkxAdapter::default();
        let request = adapter.subscription_message(subscriptions).unwrap();
        SubscriptionReadiness::from_request(&request, subscriptions.len())
    }

    #[test]
    fn login_ack_must_be_successful() {
        assert!(login_response(r#"{"event":"login","code":"0","msg":""}"#).unwrap());
        assert!(matches!(
            login_response(r#"{"event":"error","code":"60009","msg":"Login failed"}"#),
            Err(ConnectionError::LoginFailed(_))
        ));
    }

    #[test]
    fn subscription_readiness_requires_successful_ack() {
        assert_eq!(
            subscription_message(
                r#"{"event":"subscribe","arg":{"channel":"orders","instType":"ANY"},"connId":"one"}"#
            )
            .unwrap(),
            SubscriptionMessage::Acknowledged(
                SubscriptionIdentity::from_argument(&serde_json::json!({
                    "channel": "orders",
                    "instType": "ANY",
                }))
                .unwrap()
            )
        );
        assert!(matches!(
            subscription_message(
                r#"{"event":"channel-conn-count-error","code":"60012","msg":"too many"}"#
            ),
            Err(ConnectionError::SubscriptionFailed(_))
        ));
    }

    #[test]
    fn subscription_readiness_requires_each_exact_identity() {
        let subscriptions = [
            Subscription::public(
                Venue::Okx,
                Channel::Books,
                "BTC-USDT",
                FeedPriority::Critical,
            ),
            Subscription::public(
                Venue::Okx,
                Channel::Books,
                "BTC-USDT-SWAP",
                FeedPriority::Critical,
            ),
        ];
        let mut readiness = readiness_for(&subscriptions).unwrap();

        let spot = match subscription_message(
            r#"{"event":"subscribe","arg":{"channel":"books","instId":"BTC-USDT"}}"#,
        )
        .unwrap()
        {
            SubscriptionMessage::Acknowledged(identity) => identity,
            message => panic!("unexpected message {message:?}"),
        };
        readiness.acknowledge(spot.clone()).unwrap();
        assert!(!readiness.is_complete());

        readiness.acknowledge(spot).unwrap();
        assert!(!readiness.is_complete());

        let swap = match subscription_message(
            r#"{"event":"subscribe","arg":{"channel":"books","instId":"BTC-USDT-SWAP"}}"#,
        )
        .unwrap()
        {
            SubscriptionMessage::Acknowledged(identity) => identity,
            message => panic!("unexpected message {message:?}"),
        };
        readiness.acknowledge(swap).unwrap();
        assert!(readiness.is_complete());
    }

    #[test]
    fn subscription_readiness_rejects_malformed_unexpected_and_duplicate_plans() {
        for payload in [
            r#"{"event":"subscribe"}"#,
            r#"{"event":"subscribe","arg":{"instId":"BTC-USDT"}}"#,
            r#"{"event":"subscribe","code":0,"arg":{"channel":"books","instId":"BTC-USDT"}}"#,
            r#"{"event":"subscribe","code":"1","arg":{"channel":"books","instId":"BTC-USDT"}}"#,
        ] {
            assert!(matches!(
                subscription_message(payload),
                Err(ConnectionError::SubscriptionFailed(_))
            ));
        }

        let subscription = Subscription::public(
            Venue::Okx,
            Channel::Books,
            "BTC-USDT",
            FeedPriority::Critical,
        );
        let mut readiness = readiness_for(std::slice::from_ref(&subscription)).unwrap();
        let unexpected = SubscriptionIdentity::from_argument(&serde_json::json!({
            "channel": "books",
            "instId": "ETH-USDT",
        }))
        .unwrap();
        assert!(matches!(
            readiness.acknowledge(unexpected),
            Err(ConnectionError::SubscriptionFailed(_))
        ));
        assert!(matches!(
            readiness_for(&[subscription.clone(), subscription]),
            Err(ConnectionError::InvalidSubscriptionPlan(_))
        ));

        let orders = Subscription::private(Venue::Okx, Channel::Orders, FeedPriority::Critical);
        let mut private_readiness = readiness_for(&[orders]).unwrap();
        let wrong_scope = SubscriptionIdentity::from_argument(&serde_json::json!({
            "channel": "orders",
            "instType": "SWAP",
        }))
        .unwrap();
        assert!(matches!(
            private_readiness.acknowledge(wrong_scope),
            Err(ConnectionError::SubscriptionFailed(_))
        ));
        let exact_scope = SubscriptionIdentity::from_argument(&serde_json::json!({
            "channel": "orders",
            "instType": "ANY",
        }))
        .unwrap();
        private_readiness.acknowledge(exact_scope).unwrap();
        assert!(private_readiness.is_complete());
    }

    #[tokio::test]
    async fn duplicate_ack_cannot_hide_a_missing_subscription() {
        let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
        let url = format!("ws://{}", listener.local_addr().unwrap());
        let (duplicates_sent, duplicates_received) = oneshot::channel();
        let (release_missing, release_missing_received) = oneshot::channel();
        let server = tokio::spawn(async move {
            let (stream, _) = listener.accept().await.unwrap();
            let mut socket = accept_async(stream).await.unwrap();
            let request = socket.next().await.unwrap().unwrap().into_text().unwrap();
            let request: serde_json::Value = serde_json::from_str(&request).unwrap();
            assert_eq!(request["op"], "subscribe");
            assert_eq!(request["args"].as_array().unwrap().len(), 2);

            let spot_ack = Message::Text(
                r#"{"event":"subscribe","arg":{"channel":"books","instId":"BTC-USDT"}}"#.into(),
            );
            socket.send(spot_ack.clone()).await.unwrap();
            socket.send(spot_ack).await.unwrap();
            duplicates_sent.send(()).unwrap();
            release_missing_received.await.unwrap();
            socket
                .send(Message::Text(
                    r#"{"event":"subscribe","arg":{"channel":"books","instId":"BTC-USDT-SWAP"}}"#
                        .into(),
                ))
                .await
                .unwrap();
            while let Some(message) = socket.next().await {
                if matches!(message.unwrap(), Message::Close(_)) {
                    break;
                }
            }
        });

        let plan = SocketPlan {
            conn_id: ConnId::new("books"),
            venue: Venue::Okx,
            private: false,
            subscriptions: vec![
                Subscription::public(
                    Venue::Okx,
                    Channel::Books,
                    "BTC-USDT",
                    reap_core::FeedPriority::Critical,
                ),
                Subscription::public(
                    Venue::Okx,
                    Channel::Books,
                    "BTC-USDT-SWAP",
                    reap_core::FeedPriority::Critical,
                ),
            ],
        };
        let adapter = reap_venue::okx::OkxAdapter::new(&url, &url);
        let (output_tx, _output_rx) = mpsc::channel(8);
        let (status_tx, mut status_rx) = mpsc::channel(8);
        let (shutdown_tx, mut shutdown_rx) = watch::channel(false);
        let (_recovery_tx, mut recovery_rx) = watch::channel(0_u64);
        let client = tokio::spawn(async move {
            run_connection_once(
                &adapter,
                &plan,
                &[],
                &output_tx,
                &status_tx,
                &mut shutdown_rx,
                &mut recovery_rx,
            )
            .await
        });

        duplicates_received.await.unwrap();
        assert!(
            tokio::time::timeout(std::time::Duration::from_millis(50), status_rx.recv())
                .await
                .is_err()
        );
        release_missing.send(()).unwrap();
        let ready = tokio::time::timeout(std::time::Duration::from_secs(1), status_rx.recv())
            .await
            .unwrap()
            .unwrap();
        assert_eq!(ready.kind, ConnectionStatusKind::Ready);
        assert_eq!(ready.reason, "subscriptions acknowledged");

        shutdown_tx.send(true).unwrap();
        tokio::time::timeout(std::time::Duration::from_secs(1), client)
            .await
            .unwrap()
            .unwrap()
            .unwrap();
        server.await.unwrap();
    }

    #[tokio::test]
    async fn critical_status_transition_waits_for_bounded_capacity() {
        let (status_tx, mut status_rx) = mpsc::channel(1);
        status_tx
            .send(ConnectionStatus {
                conn_id: ConnId::new("existing"),
                venue: Venue::Okx,
                private: false,
                ts_ms: 1,
                kind: ConnectionStatusKind::Ready,
                reason: "existing".to_string(),
            })
            .await
            .unwrap();
        let plan = SocketPlan {
            conn_id: ConnId::new("book-1"),
            venue: Venue::Okx,
            private: false,
            subscriptions: Vec::new(),
        };
        let pending = send_status(
            &status_tx,
            &plan,
            ConnectionStatusKind::Disconnected,
            "peer closed",
        );
        tokio::pin!(pending);

        assert!(
            tokio::time::timeout(std::time::Duration::from_millis(10), &mut pending)
                .await
                .is_err()
        );
        assert_eq!(
            status_rx.recv().await.unwrap().conn_id,
            ConnId::new("existing")
        );
        pending.await.unwrap();
        let delivered = status_rx.recv().await.unwrap();
        assert_eq!(delivered.conn_id, ConnId::new("book-1"));
        assert_eq!(delivered.kind, ConnectionStatusKind::Disconnected);
    }
}
