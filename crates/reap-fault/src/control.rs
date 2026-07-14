use std::path::Path;
use std::sync::Arc;
use std::time::Duration;

use tokio::io::{AsyncReadExt, AsyncWriteExt};
use tokio::net::{UnixListener, UnixStream};

use crate::protocol::{CONTROL_FORMAT_VERSION, FaultProxyCommand, FaultProxyControlResponse};
use crate::state::ProxyState;

const MAX_CONTROL_BYTES: u64 = 1024 * 1024;

pub(crate) async fn run_control_listener(
    listener: UnixListener,
    state: Arc<ProxyState>,
    acknowledgement_timeout: Duration,
) {
    let mut shutdown = state.shutdown.subscribe();
    let mut connections = tokio::task::JoinSet::new();
    loop {
        tokio::select! {
            accepted = listener.accept() => {
                match accepted {
                    Ok((stream, _)) => {
                        let state = Arc::clone(&state);
                        connections.spawn(async move {
                            if let Err(error) = handle_control_stream(stream, state.clone(), acknowledgement_timeout).await {
                                state.record_error(format!("control connection failed: {error}"));
                            }
                        });
                    }
                    Err(error) => {
                        state.record_error(format!("control accept failed: {error}"));
                        tokio::time::sleep(Duration::from_millis(50)).await;
                    }
                }
            }
            joined = connections.join_next(), if !connections.is_empty() => {
                if let Some(Err(error)) = joined {
                    state.record_error(format!("control connection task failed: {error}"));
                }
            }
            changed = shutdown.changed() => {
                if changed.is_err() || shutdown.borrow().is_some() {
                    break;
                }
            }
        }
    }
    while let Some(joined) = connections.join_next().await {
        if let Err(error) = joined {
            state.record_error(format!(
                "control connection task failed during shutdown: {error}"
            ));
        }
    }
}

async fn handle_control_stream(
    stream: UnixStream,
    state: Arc<ProxyState>,
    acknowledgement_timeout: Duration,
) -> Result<(), std::io::Error> {
    let (reader, mut writer) = stream.into_split();
    let mut bytes = Vec::new();
    let response = match tokio::time::timeout(
        acknowledgement_timeout,
        reader.take(MAX_CONTROL_BYTES + 1).read_to_end(&mut bytes),
    )
    .await
    {
        Err(_) => rejected("control command read timed out"),
        Ok(Err(error)) => return Err(error),
        Ok(Ok(_)) if bytes.len() as u64 > MAX_CONTROL_BYTES => {
            rejected("control command exceeds the 1 MiB limit")
        }
        Ok(Ok(_)) => match serde_json::from_slice::<FaultProxyCommand>(&bytes) {
            Ok(command) => dispatch_command(command, &state, acknowledgement_timeout).await,
            Err(error) => rejected(format!("invalid control command: {error}")),
        },
    };
    let mut response_bytes = serde_json::to_vec(&response).unwrap_or_else(|error| {
        format!(
            "{{\"format_version\":1,\"accepted\":false,\"message\":\"serialization failure: {error}\"}}"
        )
        .into_bytes()
    });
    response_bytes.push(b'\n');
    tokio::time::timeout(acknowledgement_timeout, writer.write_all(&response_bytes))
        .await
        .map_err(|_| timed_out("control response write"))??;
    tokio::time::timeout(acknowledgement_timeout, writer.shutdown())
        .await
        .map_err(|_| timed_out("control response shutdown"))?
}

fn timed_out(operation: &'static str) -> std::io::Error {
    std::io::Error::new(
        std::io::ErrorKind::TimedOut,
        format!("{operation} timed out"),
    )
}

async fn dispatch_command(
    command: FaultProxyCommand,
    state: &Arc<ProxyState>,
    acknowledgement_timeout: Duration,
) -> FaultProxyControlResponse {
    match command {
        FaultProxyCommand::Status => FaultProxyControlResponse {
            format_version: CONTROL_FORMAT_VERSION,
            accepted: true,
            message: "status".to_string(),
            evidence_path: None,
            status: Some(state.status().await),
        },
        FaultProxyCommand::DisconnectWebsockets {
            command_id,
            evidence_file,
            target,
            connections,
        } => {
            state
                .disconnect_websockets(
                    command_id,
                    evidence_file,
                    target,
                    connections,
                    acknowledgement_timeout,
                )
                .await
        }
        FaultProxyCommand::ArmRestResponse {
            command_id,
            evidence_file,
            matcher,
            response,
            times,
        } => {
            state
                .arm_rest(command_id, evidence_file, matcher, response, times)
                .await
        }
        FaultProxyCommand::ArmWebsocketDrop {
            command_id,
            evidence_file,
            target,
            direction,
            matcher,
            frames,
        } => {
            state
                .arm_websocket_drop(
                    command_id,
                    evidence_file,
                    target,
                    direction,
                    matcher,
                    frames,
                )
                .await
        }
        FaultProxyCommand::Shutdown { reason } => {
            if reason.trim().is_empty() || reason.len() > 256 {
                return rejected("shutdown reason must contain 1-256 bytes");
            }
            let _ = state.shutdown.send(Some(format!("control: {reason}")));
            FaultProxyControlResponse {
                format_version: CONTROL_FORMAT_VERSION,
                accepted: true,
                message: "shutdown requested".to_string(),
                evidence_path: None,
                status: None,
            }
        }
    }
}

pub async fn send_fault_proxy_command(
    socket_path: impl AsRef<Path>,
    command: &FaultProxyCommand,
) -> Result<FaultProxyControlResponse, FaultProxyControlError> {
    let socket_path = socket_path.as_ref();
    let mut stream = UnixStream::connect(socket_path).await.map_err(|source| {
        FaultProxyControlError::Connect {
            path: socket_path.to_path_buf(),
            source,
        }
    })?;
    let bytes = serde_json::to_vec(command)?;
    if bytes.len() as u64 > MAX_CONTROL_BYTES {
        return Err(FaultProxyControlError::TooLarge(bytes.len() as u64));
    }
    stream.write_all(&bytes).await?;
    stream.shutdown().await?;
    let mut response = Vec::new();
    stream
        .take(MAX_CONTROL_BYTES + 1)
        .read_to_end(&mut response)
        .await?;
    if response.len() as u64 > MAX_CONTROL_BYTES {
        return Err(FaultProxyControlError::ResponseTooLarge(
            response.len() as u64
        ));
    }
    Ok(serde_json::from_slice(&response)?)
}

fn rejected(message: impl Into<String>) -> FaultProxyControlResponse {
    FaultProxyControlResponse {
        format_version: CONTROL_FORMAT_VERSION,
        accepted: false,
        message: message.into(),
        evidence_path: None,
        status: None,
    }
}

#[derive(Debug, thiserror::Error)]
pub enum FaultProxyControlError {
    #[error("failed to connect to fault-proxy control socket {path}: {source}")]
    Connect {
        path: std::path::PathBuf,
        source: std::io::Error,
    },
    #[error("control command is {0} bytes; maximum is 1 MiB")]
    TooLarge(u64),
    #[error("control response is {0} bytes; maximum is 1 MiB")]
    ResponseTooLarge(u64),
    #[error("fault-proxy control I/O failed: {0}")]
    Io(#[from] std::io::Error),
    #[error("fault-proxy control JSON failed: {0}")]
    Json(#[from] serde_json::Error),
}

#[cfg(test)]
mod tests {
    use std::collections::BTreeMap;

    use crate::protocol::{InjectedHttpResponse, RestRequestMatcher};

    use super::*;

    #[tokio::test]
    async fn unix_control_round_trip_arms_once_reports_status_and_shuts_down() {
        let directory = tempfile::tempdir().unwrap();
        let evidence = directory.path().join("evidence");
        std::fs::create_dir(&evidence).unwrap();
        let socket = directory.path().join("control.sock");
        let listener = UnixListener::bind(&socket).unwrap();
        let state = ProxyState::new("fingerprint".to_string(), evidence, 8, 1024 * 1024);
        let task = tokio::spawn(run_control_listener(
            listener,
            Arc::clone(&state),
            Duration::from_secs(1),
        ));

        let status = send_fault_proxy_command(&socket, &FaultProxyCommand::Status)
            .await
            .unwrap();
        assert!(status.accepted);
        assert_eq!(status.status.unwrap().proxy_session_id, state.session_id);

        let arm = FaultProxyCommand::ArmRestResponse {
            command_id: "clock-failure".to_string(),
            evidence_file: "clock-failure.json".to_string(),
            matcher: RestRequestMatcher {
                method: "GET".to_string(),
                path: "/api/v5/public/time".to_string(),
                query: BTreeMap::new(),
            },
            response: InjectedHttpResponse {
                status: 503,
                headers: BTreeMap::new(),
                body: String::new(),
            },
            times: 1,
        };
        assert!(
            send_fault_proxy_command(&socket, &arm)
                .await
                .unwrap()
                .accepted
        );
        let duplicate = send_fault_proxy_command(&socket, &arm).await.unwrap();
        assert!(!duplicate.accepted);
        assert!(duplicate.message.contains("already used"));

        let status = send_fault_proxy_command(&socket, &FaultProxyCommand::Status)
            .await
            .unwrap()
            .status
            .unwrap();
        assert_eq!(status.pending_rest_faults, 1);
        let shutdown = send_fault_proxy_command(
            &socket,
            &FaultProxyCommand::Shutdown {
                reason: "test complete".to_string(),
            },
        )
        .await
        .unwrap();
        assert!(shutdown.accepted);
        task.await.unwrap();
    }
}
