use futures_util::{SinkExt, StreamExt};
use reap_core::{Channel, RawEnvelope};
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

    loop {
        tokio::select! {
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
                match message {
                    Message::Text(payload) => {
                        forward_payload(payload.as_str(), plan, output).await?;
                    }
                    Message::Binary(payload) => {
                        let payload = std::str::from_utf8(payload.as_ref())
                            .map_err(|_| ConnectionError::NonUtf8Payload)?;
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
}
