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
    let (socket, _) = connect_async(adapter.websocket_url(plan.private)).await?;
    let (mut writer, mut reader) = socket.split();

    if plan.private {
        for message in bootstrap_messages {
            writer.send(Message::Text(message.clone().into())).await?;
        }
        await_private_login(&mut writer, &mut reader, shutdown, recovery).await?;
    }
    let subscription = adapter.subscription_message(&plan.subscriptions)?;
    writer.send(Message::Text(subscription.into())).await?;
    await_subscriptions(
        &mut writer,
        &mut reader,
        plan.subscriptions.len(),
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
    expected: usize,
    plan: &SocketPlan,
    output: &mpsc::Sender<RawEnvelope>,
    shutdown: &mut watch::Receiver<bool>,
    recovery: &mut watch::Receiver<u64>,
) -> Result<(), ConnectionError>
where
    S: tokio::io::AsyncRead + tokio::io::AsyncWrite + Unpin,
{
    if expected == 0 {
        return Ok(());
    }
    let deadline = tokio::time::sleep(std::time::Duration::from_secs(10));
    tokio::pin!(deadline);
    let mut acknowledged = 0_usize;
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
                            SubscriptionMessage::Acknowledged => acknowledged += 1,
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
                            SubscriptionMessage::Acknowledged => acknowledged += 1,
                            SubscriptionMessage::Data => forward_payload(payload, plan, output).await?,
                            SubscriptionMessage::Ignore => {}
                        }
                    }
                    Message::Ping(payload) => writer.send(Message::Pong(payload)).await?,
                    Message::Pong(_) => {}
                    Message::Close(_) => return Err(ConnectionError::PeerClosed),
                    Message::Frame(_) => {}
                }
                if acknowledged >= expected {
                    return Ok(());
                }
            }
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum SubscriptionMessage {
    Acknowledged,
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
        Some("subscribe")
            if value
                .get("code")
                .and_then(serde_json::Value::as_str)
                .is_none_or(|code| code == "0") =>
        {
            Ok(SubscriptionMessage::Acknowledged)
        }
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
    use super::*;

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
                r#"{"event":"subscribe","arg":{"channel":"orders"},"connId":"one"}"#
            )
            .unwrap(),
            SubscriptionMessage::Acknowledged
        );
        assert!(matches!(
            subscription_message(
                r#"{"event":"channel-conn-count-error","code":"60012","msg":"too many"}"#
            ),
            Err(ConnectionError::SubscriptionFailed(_))
        ));
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
