use std::collections::HashSet;
use std::convert::Infallible;
use std::fs;
use std::path::Path;
use std::sync::Arc;
use std::sync::atomic::Ordering;
use std::time::{Duration, Instant};

use bytes::{Bytes, BytesMut};
use futures_util::{SinkExt, StreamExt};
use http_body_util::{BodyExt, Full, Limited};
use hyper::body::Incoming;
use hyper::header::{CONNECTION, CONTENT_LENGTH, HeaderMap, HeaderName, HeaderValue};
use hyper::server::conn::http1;
use hyper::service::service_fn;
use hyper::{Request, Response, StatusCode};
use hyper_util::rt::TokioIo;
use reap_core::PINNED_JAVA_REVISION;
use reap_live::{current_executable_sha256, host_identity_sha256};
use tokio::net::{TcpListener, TcpStream, UnixListener};
use tokio::sync::mpsc;
use tokio::task::JoinSet;
use tokio_tungstenite::tungstenite::Message;
use tokio_tungstenite::tungstenite::handshake::server::{
    Callback, ErrorResponse as WebSocketErrorResponse, Request as WebSocketRequest,
    Response as WebSocketResponse,
};
use tokio_tungstenite::tungstenite::protocol::{CloseFrame, frame::coding::CloseCode};
use tokio_tungstenite::{WebSocketStream, accept_hdr_async, connect_async};

use crate::config::{FaultProxyConfig, FaultProxyConfigEvidence};
use crate::control::run_control_listener;
use crate::protocol::{
    FaultProxyRunReport, InjectedHttpResponse, RUN_REPORT_FORMAT_VERSION, WebSocketDirection,
    WebSocketTarget,
};
use crate::state::{DisconnectSignal, ProxyState, now_ms};

#[derive(Debug, Clone, Default)]
pub struct FaultProxyRunOptions {
    pub duration: Option<Duration>,
}

pub async fn run_fault_proxy(
    config: FaultProxyConfig,
    config_evidence: FaultProxyConfigEvidence,
    options: FaultProxyRunOptions,
) -> Result<FaultProxyRunReport, FaultProxyRuntimeError> {
    config.validate()?;
    prepare_private_directory(&config.evidence_directory)?;
    prepare_control_parent(&config.control_socket)?;
    if config.control_socket.exists() {
        return Err(FaultProxyRuntimeError::ControlSocketExists(
            config.control_socket.clone(),
        ));
    }
    let executable_sha256 =
        current_executable_sha256().map_err(FaultProxyRuntimeError::Provenance)?;
    let host_identity_sha256 =
        host_identity_sha256().map_err(FaultProxyRuntimeError::Provenance)?;

    let rest_listener = TcpListener::bind(config.rest_listen)
        .await
        .map_err(|source| FaultProxyRuntimeError::BindTcp {
            name: "rest",
            address: config.rest_listen,
            source,
        })?;
    let public_listener = bind_websocket("public", config.public_ws_listen).await?;
    let private_listener = bind_websocket("private", config.private_ws_listen).await?;
    let order_listener = bind_websocket("order", config.order_ws_listen).await?;
    let control_listener = UnixListener::bind(&config.control_socket).map_err(|source| {
        FaultProxyRuntimeError::BindControl {
            path: config.control_socket.clone(),
            source,
        }
    })?;
    tighten_socket_permissions(&config.control_socket)?;

    let state = ProxyState::new(
        config_evidence.effective_fingerprint.clone(),
        config.evidence_directory.clone(),
        config.max_pending_faults,
        config.max_http_body_bytes,
    );
    let client = reqwest::Client::builder()
        .connect_timeout(Duration::from_millis(config.request_timeout_ms))
        .timeout(Duration::from_millis(config.request_timeout_ms))
        .build()
        .map_err(FaultProxyRuntimeError::HttpClient)?;
    let started_at_ms = now_ms();
    let started = Instant::now();
    let mut tasks = JoinSet::new();
    tasks.spawn(run_rest_listener(
        rest_listener,
        Arc::clone(&state),
        client,
        config.upstream.rest_url.clone(),
        config.max_http_body_bytes,
    ));
    tasks.spawn(run_websocket_listener(
        public_listener,
        Arc::clone(&state),
        WebSocketTarget::Public,
        config.upstream.public_ws_url.clone(),
    ));
    tasks.spawn(run_websocket_listener(
        private_listener,
        Arc::clone(&state),
        WebSocketTarget::Private,
        config.upstream.private_ws_url.clone(),
    ));
    tasks.spawn(run_websocket_listener(
        order_listener,
        Arc::clone(&state),
        WebSocketTarget::Order,
        config.upstream.private_ws_url.clone(),
    ));
    tasks.spawn(run_control_listener(
        control_listener,
        Arc::clone(&state),
        Duration::from_millis(config.request_timeout_ms),
    ));

    let mut shutdown = state.shutdown.subscribe();
    let stop_reason = match options.duration {
        Some(duration) => {
            tokio::select! {
                _ = tokio::time::sleep(duration) => "duration_elapsed".to_string(),
                result = tokio::signal::ctrl_c() => {
                    result.map_err(FaultProxyRuntimeError::Signal)?;
                    "interrupt".to_string()
                }
                changed = shutdown.changed() => {
                    if changed.is_err() {
                        "control_channel_closed".to_string()
                    } else {
                        shutdown.borrow().clone().unwrap_or_else(|| "control".to_string())
                    }
                }
                joined = tasks.join_next(), if !tasks.is_empty() => {
                    listener_exit_stop_reason(joined, &state, &shutdown)
                }
            }
        }
        None => {
            tokio::select! {
                result = tokio::signal::ctrl_c() => {
                    result.map_err(FaultProxyRuntimeError::Signal)?;
                    "interrupt".to_string()
                }
                changed = shutdown.changed() => {
                    if changed.is_err() {
                        "control_channel_closed".to_string()
                    } else {
                        shutdown.borrow().clone().unwrap_or_else(|| "control".to_string())
                    }
                }
                joined = tasks.join_next(), if !tasks.is_empty() => {
                    listener_exit_stop_reason(joined, &state, &shutdown)
                }
            }
        }
    };
    let _ = state.shutdown.send(Some(stop_reason.clone()));

    let shutdown_deadline = tokio::time::sleep(Duration::from_millis(config.shutdown_timeout_ms));
    tokio::pin!(shutdown_deadline);
    let mut joined_cleanly = true;
    loop {
        tokio::select! {
            joined = tasks.join_next(), if !tasks.is_empty() => {
                if let Some(Err(error)) = joined {
                    joined_cleanly = false;
                    state.record_error(format!("proxy listener task failed: {error}"));
                }
            }
            _ = &mut shutdown_deadline => {
                if !tasks.is_empty() {
                    joined_cleanly = false;
                    state.record_error("proxy shutdown timed out with active listener tasks");
                    tasks.abort_all();
                    while tasks.join_next().await.is_some() {}
                }
                break;
            }
            else => break,
        }
        if tasks.is_empty() {
            break;
        }
    }
    let socket_removed = match remove_control_socket(&config.control_socket) {
        Ok(()) => true,
        Err(error) => {
            state.record_error(format!("failed to remove control socket: {error}"));
            false
        }
    };
    let status = state.status().await;
    let clean_shutdown = joined_cleanly
        && socket_removed
        && status.proxy_errors == 0
        && status.pending_rest_faults == 0
        && status.pending_websocket_faults == 0
        && status
            .websocket_connections_active
            .values()
            .all(|count| *count == 0);
    Ok(FaultProxyRunReport {
        format_version: RUN_REPORT_FORMAT_VERSION,
        proxy_session_id: state.session_id.clone(),
        config: config_evidence,
        java_reference_revision: PINNED_JAVA_REVISION.to_string(),
        reap_version: env!("CARGO_PKG_VERSION").to_string(),
        executable_sha256,
        host_identity_sha256,
        started_at_ms,
        stopped_at_ms: now_ms(),
        elapsed_ms: started.elapsed().as_millis().try_into().unwrap_or(u64::MAX),
        stop_reason,
        status,
        listener_tasks_joined_cleanly: joined_cleanly,
        control_socket_removed: socket_removed,
        clean_shutdown,
    })
}

fn listener_exit_stop_reason(
    joined: Option<Result<(), tokio::task::JoinError>>,
    state: &ProxyState,
    shutdown: &tokio::sync::watch::Receiver<Option<String>>,
) -> String {
    match joined {
        Some(Ok(())) => {
            if let Some(reason) = shutdown.borrow().clone() {
                reason
            } else {
                state.record_error("proxy listener task exited before shutdown");
                "listener_task_exited".to_string()
            }
        }
        Some(Err(error)) => {
            state.record_error(format!("proxy listener task failed: {error}"));
            "listener_task_failed".to_string()
        }
        None => {
            state.record_error("proxy listener set became empty before shutdown");
            "listener_set_empty".to_string()
        }
    }
}

async fn bind_websocket(
    name: &'static str,
    address: std::net::SocketAddr,
) -> Result<TcpListener, FaultProxyRuntimeError> {
    TcpListener::bind(address)
        .await
        .map_err(|source| FaultProxyRuntimeError::BindTcp {
            name,
            address,
            source,
        })
}

async fn run_rest_listener(
    listener: TcpListener,
    state: Arc<ProxyState>,
    client: reqwest::Client,
    upstream_rest_url: String,
    max_body_bytes: usize,
) {
    let mut shutdown = state.shutdown.subscribe();
    let mut connections = JoinSet::new();
    loop {
        tokio::select! {
            accepted = listener.accept() => {
                match accepted {
                    Ok((stream, _)) => {
                        let state = Arc::clone(&state);
                        let service_state = Arc::clone(&state);
                        let client = client.clone();
                        let upstream = upstream_rest_url.clone();
                        connections.spawn(async move {
                            let mut connection_shutdown = state.shutdown.subscribe();
                            let service = service_fn(move |request| {
                                handle_rest_request(
                                    request,
                                    Arc::clone(&service_state),
                                    client.clone(),
                                    upstream.clone(),
                                    max_body_bytes,
                                )
                            });
                            let connection = http1::Builder::new()
                                .serve_connection(TokioIo::new(stream), service);
                            tokio::pin!(connection);
                            tokio::select! {
                                result = &mut connection => {
                                    if let Err(error) = result {
                                        state.record_error(format!("REST client connection failed: {error}"));
                                    }
                                }
                                changed = connection_shutdown.changed() => {
                                    if changed.is_err() || connection_shutdown.borrow().is_some() {
                                        connection.as_mut().graceful_shutdown();
                                        if let Err(error) = connection.await {
                                            state.record_error(format!(
                                                "REST client connection failed during shutdown: {error}"
                                            ));
                                        }
                                    }
                                }
                            }
                        });
                    }
                    Err(error) => {
                        state.record_error(format!("REST accept failed: {error}"));
                        tokio::time::sleep(Duration::from_millis(50)).await;
                    }
                }
            }
            joined = connections.join_next(), if !connections.is_empty() => {
                if let Some(Err(error)) = joined {
                    state.record_error(format!("REST connection task failed: {error}"));
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
                "REST connection task failed during shutdown: {error}"
            ));
        }
    }
}

async fn handle_rest_request(
    request: Request<Incoming>,
    state: Arc<ProxyState>,
    client: reqwest::Client,
    upstream_rest_url: String,
    max_body_bytes: usize,
) -> Result<Response<Full<Bytes>>, Infallible> {
    let method = request.method().clone();
    let uri = request.uri().clone();
    let headers = request.headers().clone();
    if let Some(injection) = state
        .consume_rest(method.as_str(), uri.path(), uri.query())
        .await
    {
        state
            .counters
            .rest_responses_injected
            .fetch_add(1, Ordering::Relaxed);
        if let Some(completion) = injection.completion
            && let Err(error) = state.write_completion(completion).await
        {
            state.record_error(error);
        }
        return Ok(injected_response(injection.response));
    }

    if headers
        .get(CONTENT_LENGTH)
        .and_then(|value| value.to_str().ok())
        .and_then(|value| value.parse::<usize>().ok())
        .is_some_and(|length| length > max_body_bytes)
    {
        return Ok(error_response(
            StatusCode::PAYLOAD_TOO_LARGE,
            "request body exceeds proxy limit",
        ));
    }
    let body = match Limited::new(request.into_body(), max_body_bytes)
        .collect()
        .await
    {
        Ok(body) => body.to_bytes(),
        Err(_) => {
            return Ok(error_response(
                StatusCode::PAYLOAD_TOO_LARGE,
                "request body exceeds proxy limit",
            ));
        }
    };
    let path_and_query = uri
        .path_and_query()
        .map(|value| value.as_str())
        .unwrap_or("/");
    let url = format!(
        "{}{}",
        upstream_rest_url.trim_end_matches('/'),
        path_and_query
    );
    let mut forwarded_headers = HeaderMap::new();
    copy_forward_headers(&headers, &mut forwarded_headers);
    state
        .counters
        .rest_requests_forwarded
        .fetch_add(1, Ordering::Relaxed);
    let mut upstream = match client
        .request(method, url)
        .headers(forwarded_headers)
        .body(body)
        .send()
        .await
    {
        Ok(response) => response,
        Err(error) => {
            state.record_error(format!("upstream REST request failed: {error}"));
            return Ok(error_response(
                StatusCode::BAD_GATEWAY,
                "upstream REST request failed",
            ));
        }
    };
    let status = upstream.status();
    let upstream_headers = upstream.headers().clone();
    if upstream
        .content_length()
        .is_some_and(|length| length > max_body_bytes as u64)
    {
        state.record_error("upstream REST response exceeds proxy limit");
        return Ok(error_response(
            StatusCode::BAD_GATEWAY,
            "upstream REST response exceeds proxy limit",
        ));
    }
    let mut body = BytesMut::new();
    loop {
        match upstream.chunk().await {
            Ok(Some(chunk)) if body.len().saturating_add(chunk.len()) <= max_body_bytes => {
                body.extend_from_slice(&chunk);
            }
            Ok(Some(_)) => {
                state.record_error("upstream REST response exceeds proxy limit");
                return Ok(error_response(
                    StatusCode::BAD_GATEWAY,
                    "upstream REST response could not be read within proxy limits",
                ));
            }
            Ok(None) => break,
            Err(error) => {
                state.record_error(format!("upstream REST response read failed: {error}"));
                return Ok(error_response(
                    StatusCode::BAD_GATEWAY,
                    "upstream REST response could not be read within proxy limits",
                ));
            }
        }
    }
    let mut response = Response::new(Full::new(body.freeze()));
    *response.status_mut() = status;
    copy_forward_headers(&upstream_headers, response.headers_mut());
    Ok(response)
}

fn injected_response(injected: InjectedHttpResponse) -> Response<Full<Bytes>> {
    let mut response = Response::new(Full::new(Bytes::from(injected.body)));
    *response.status_mut() =
        StatusCode::from_u16(injected.status).unwrap_or(StatusCode::INTERNAL_SERVER_ERROR);
    for (name, value) in injected.headers {
        let Ok(name) = HeaderName::from_bytes(name.as_bytes()) else {
            continue;
        };
        let Ok(value) = HeaderValue::from_str(&value) else {
            continue;
        };
        if !hop_by_hop_header(&name) {
            response.headers_mut().insert(name, value);
        }
    }
    response
}

fn error_response(status: StatusCode, message: &str) -> Response<Full<Bytes>> {
    let body = serde_json::to_vec(&serde_json::json!({
        "code": "reap_fault_proxy",
        "message": message,
    }))
    .unwrap_or_else(|_| b"{\"code\":\"reap_fault_proxy\"}".to_vec());
    let mut response = Response::new(Full::new(Bytes::from(body)));
    *response.status_mut() = status;
    response.headers_mut().insert(
        hyper::header::CONTENT_TYPE,
        HeaderValue::from_static("application/json"),
    );
    response
}

fn copy_forward_headers(source: &HeaderMap, target: &mut HeaderMap) {
    let connection_scoped = connection_scoped_headers(source);
    for (name, value) in source {
        if !hop_by_hop_header(name) && !connection_scoped.contains(name) {
            target.append(name.clone(), value.clone());
        }
    }
}

fn hop_by_hop_header(name: &HeaderName) -> bool {
    matches!(
        name.as_str(),
        "host"
            | "connection"
            | "content-length"
            | "transfer-encoding"
            | "keep-alive"
            | "proxy-authenticate"
            | "proxy-authorization"
            | "te"
            | "trailer"
            | "upgrade"
    )
}

fn connection_scoped_headers(headers: &HeaderMap) -> HashSet<HeaderName> {
    headers
        .get_all(CONNECTION)
        .iter()
        .filter_map(|value| value.to_str().ok())
        .flat_map(|value| value.split(','))
        .filter_map(|name| HeaderName::from_bytes(name.trim().as_bytes()).ok())
        .collect()
}

async fn run_websocket_listener(
    listener: TcpListener,
    state: Arc<ProxyState>,
    target: WebSocketTarget,
    upstream_url: String,
) {
    let mut shutdown = state.shutdown.subscribe();
    let mut connections = JoinSet::new();
    loop {
        tokio::select! {
            accepted = listener.accept() => {
                match accepted {
                    Ok((stream, _)) => {
                        let state = Arc::clone(&state);
                        let upstream_url = upstream_url.clone();
                        connections.spawn(async move {
                            if let Err(error) = bridge_websocket(stream, state.clone(), target, &upstream_url).await {
                                state.record_error(format!("{target:?} websocket bridge failed: {error}"));
                            }
                        });
                    }
                    Err(error) => {
                        state.record_error(format!("{target:?} websocket accept failed: {error}"));
                        tokio::time::sleep(Duration::from_millis(50)).await;
                    }
                }
            }
            joined = connections.join_next(), if !connections.is_empty() => {
                if let Some(Err(error)) = joined {
                    state.record_error(format!("{target:?} websocket connection task failed: {error}"));
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
                "{target:?} websocket connection task failed during shutdown: {error}"
            ));
        }
    }
}

async fn bridge_websocket(
    stream: TcpStream,
    state: Arc<ProxyState>,
    target: WebSocketTarget,
    upstream_url: &str,
) -> Result<(), WebSocketBridgeError> {
    let client = accept_hdr_async(
        stream,
        ExpectedPathCallback {
            expected_path: target.expected_path(),
        },
    )
    .await?;
    let (upstream, _) = connect_async(upstream_url).await?;
    bridge_websocket_streams(client, upstream, state, target).await
}

struct ExpectedPathCallback {
    expected_path: &'static str,
}

impl Callback for ExpectedPathCallback {
    fn on_request(
        self,
        request: &WebSocketRequest,
        response: WebSocketResponse,
    ) -> Result<WebSocketResponse, WebSocketErrorResponse> {
        if request.uri().path() == self.expected_path && request.uri().query().is_none() {
            Ok(response)
        } else {
            Err(hyper::Response::builder()
                .status(StatusCode::BAD_REQUEST)
                .body(Some(format!(
                    "expected exact websocket path {}",
                    self.expected_path
                )))
                .expect("static websocket error response is valid"))
        }
    }
}

async fn bridge_websocket_streams<S1, S2>(
    client: WebSocketStream<S1>,
    upstream: WebSocketStream<S2>,
    state: Arc<ProxyState>,
    target: WebSocketTarget,
) -> Result<(), WebSocketBridgeError>
where
    S1: tokio::io::AsyncRead + tokio::io::AsyncWrite + Unpin,
    S2: tokio::io::AsyncRead + tokio::io::AsyncWrite + Unpin,
{
    let (mut client_sink, mut client_stream) = client.split();
    let (mut upstream_sink, mut upstream_stream) = upstream.split();
    let (disconnect_tx, mut disconnect_rx) = mpsc::channel::<DisconnectSignal>(1);
    let connection_id = state.register_connection(target, disconnect_tx).await;
    let mut shutdown = state.shutdown.subscribe();
    let result = loop {
        tokio::select! {
            message = client_stream.next() => {
                let Some(message) = message else { break Ok(()); };
                let message = message?;
                if let Some(drop) = state.consume_websocket(
                    connection_id,
                    target,
                    WebSocketDirection::ClientToExchange,
                    &message,
                ).await {
                    state.counters.websocket_frames_dropped.fetch_add(1, Ordering::Relaxed);
                    if let Some(completion) = drop.completion
                        && let Err(error) = state.write_completion(completion).await
                    {
                        state.record_error(error);
                    }
                    continue;
                }
                upstream_sink.send(message).await?;
                state.counters.websocket_frames_forwarded.fetch_add(1, Ordering::Relaxed);
            }
            message = upstream_stream.next() => {
                let Some(message) = message else { break Ok(()); };
                let message = message?;
                if let Some(drop) = state.consume_websocket(
                    connection_id,
                    target,
                    WebSocketDirection::ExchangeToClient,
                    &message,
                ).await {
                    state.counters.websocket_frames_dropped.fetch_add(1, Ordering::Relaxed);
                    if let Some(completion) = drop.completion
                        && let Err(error) = state.write_completion(completion).await
                    {
                        state.record_error(error);
                    }
                    continue;
                }
                client_sink.send(message).await?;
                state.counters.websocket_frames_forwarded.fetch_add(1, Ordering::Relaxed);
            }
            signal = disconnect_rx.recv() => {
                let Some(signal) = signal else { break Ok(()); };
                let _command_id = signal.command_id;
                let close = Message::Close(Some(CloseFrame {
                    code: CloseCode::Away,
                    reason: "reap fault proxy injected disconnect".into(),
                }));
                let _ = client_sink.send(close.clone()).await;
                let _ = upstream_sink.send(close).await;
                let applied_at_ms = now_ms();
                let _ = signal.acknowledgement.send(applied_at_ms);
                state.counters.websocket_disconnects_injected.fetch_add(1, Ordering::Relaxed);
                break Ok(());
            }
            changed = shutdown.changed() => {
                if changed.is_err() || shutdown.borrow().is_some() {
                    let close = Message::Close(Some(CloseFrame {
                        code: CloseCode::Away,
                        reason: "reap fault proxy shutdown".into(),
                    }));
                    let _ = client_sink.send(close.clone()).await;
                    let _ = upstream_sink.send(close).await;
                    break Ok(());
                }
            }
        }
    };
    state.unregister_connection(connection_id).await;
    result
}

fn prepare_private_directory(path: &Path) -> Result<(), FaultProxyRuntimeError> {
    let existed = path.exists();
    if existed {
        let metadata =
            fs::symlink_metadata(path).map_err(|source| FaultProxyRuntimeError::PreparePath {
                path: path.to_path_buf(),
                source,
            })?;
        if metadata.file_type().is_symlink() || !metadata.is_dir() {
            return Err(FaultProxyRuntimeError::UnsafeEvidencePath(
                path.to_path_buf(),
            ));
        }
    }
    fs::create_dir_all(path).map_err(|source| FaultProxyRuntimeError::PreparePath {
        path: path.to_path_buf(),
        source,
    })?;
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        if existed {
            let mode = fs::metadata(path)
                .map_err(|source| FaultProxyRuntimeError::PreparePath {
                    path: path.to_path_buf(),
                    source,
                })?
                .permissions()
                .mode();
            if mode & 0o077 != 0 {
                return Err(FaultProxyRuntimeError::InsecureEvidenceDirectory {
                    path: path.to_path_buf(),
                    mode: mode & 0o777,
                });
            }
        } else {
            fs::set_permissions(path, fs::Permissions::from_mode(0o700)).map_err(|source| {
                FaultProxyRuntimeError::PreparePath {
                    path: path.to_path_buf(),
                    source,
                }
            })?;
        }
    }
    Ok(())
}

fn prepare_control_parent(path: &Path) -> Result<(), FaultProxyRuntimeError> {
    let parent = path.parent().unwrap_or_else(|| Path::new("."));
    fs::create_dir_all(parent).map_err(|source| FaultProxyRuntimeError::PreparePath {
        path: parent.to_path_buf(),
        source,
    })
}

fn tighten_socket_permissions(path: &Path) -> Result<(), FaultProxyRuntimeError> {
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        fs::set_permissions(path, fs::Permissions::from_mode(0o600)).map_err(|source| {
            FaultProxyRuntimeError::PreparePath {
                path: path.to_path_buf(),
                source,
            }
        })?;
    }
    Ok(())
}

fn remove_control_socket(path: &Path) -> Result<(), std::io::Error> {
    match fs::remove_file(path) {
        Ok(()) => Ok(()),
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => Ok(()),
        Err(error) => Err(error),
    }
}

#[derive(Debug, thiserror::Error)]
pub enum FaultProxyRuntimeError {
    #[error(transparent)]
    Config(#[from] crate::config::FaultProxyConfigError),
    #[error("failed to prepare private path {path}: {source}")]
    PreparePath {
        path: std::path::PathBuf,
        source: std::io::Error,
    },
    #[error("fault-proxy control socket already exists: {0}")]
    ControlSocketExists(std::path::PathBuf),
    #[error(
        "fault-proxy evidence directory {path} has insecure mode {mode:o}; require no group/other permissions"
    )]
    InsecureEvidenceDirectory { path: std::path::PathBuf, mode: u32 },
    #[error("fault-proxy evidence path must be a real directory, not a file or symlink: {0}")]
    UnsafeEvidencePath(std::path::PathBuf),
    #[error("failed to bind {name} fault-proxy listener {address}: {source}")]
    BindTcp {
        name: &'static str,
        address: std::net::SocketAddr,
        source: std::io::Error,
    },
    #[error("failed to bind fault-proxy control socket {path}: {source}")]
    BindControl {
        path: std::path::PathBuf,
        source: std::io::Error,
    },
    #[error("failed to build fault-proxy HTTP client: {0}")]
    HttpClient(reqwest::Error),
    #[error("failed to install fault-proxy signal handler: {0}")]
    Signal(std::io::Error),
    #[error("failed to fingerprint fault-proxy provenance: {0}")]
    Provenance(String),
}

#[derive(Debug, thiserror::Error)]
enum WebSocketBridgeError {
    #[error("websocket protocol failed: {0}")]
    Protocol(#[from] tokio_tungstenite::tungstenite::Error),
}

#[cfg(test)]
mod tests {
    use std::collections::BTreeMap;

    use http_body_util::StreamBody;
    use hyper::body::Frame;
    use hyper::header::HeaderValue;
    use tokio_tungstenite::{accept_async, connect_async};

    use super::*;
    use crate::protocol::{
        FaultInjectorEvidence, RestRequestMatcher, WebSocketFrameKind, WebSocketFrameMatcher,
        WebSocketJsonMatcher,
    };

    fn private_evidence_directory(root: &Path) -> std::path::PathBuf {
        let path = root.join("evidence");
        fs::create_dir(&path).unwrap();
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            fs::set_permissions(&path, fs::Permissions::from_mode(0o700)).unwrap();
        }
        path
    }

    #[test]
    fn forwarded_headers_remove_standard_and_connection_scoped_hops() {
        let mut source = HeaderMap::new();
        source.insert(CONNECTION, HeaderValue::from_static("keep-alive, x-hop"));
        source.insert("keep-alive", HeaderValue::from_static("timeout=5"));
        source.insert("x-hop", HeaderValue::from_static("remove"));
        source.insert("x-end-to-end", HeaderValue::from_static("retain"));
        let mut target = HeaderMap::new();

        copy_forward_headers(&source, &mut target);

        assert!(!target.contains_key(CONNECTION));
        assert!(!target.contains_key("keep-alive"));
        assert!(!target.contains_key("x-hop"));
        assert_eq!(target["x-end-to-end"], "retain");
    }

    #[test]
    fn listener_exit_is_fatal_unless_shutdown_was_already_requested() {
        let directory = tempfile::tempdir().unwrap();
        let state = ProxyState::new(
            "fingerprint".to_string(),
            directory.path().to_path_buf(),
            8,
            1024,
        );
        let shutdown = state.shutdown.subscribe();
        assert_eq!(
            listener_exit_stop_reason(Some(Ok(())), &state, &shutdown),
            "listener_task_exited"
        );
        assert_eq!(state.counters.proxy_errors.load(Ordering::Relaxed), 1);

        let state = ProxyState::new(
            "fingerprint".to_string(),
            directory.path().to_path_buf(),
            8,
            1024,
        );
        let shutdown = state.shutdown.subscribe();
        let _ = state.shutdown.send(Some("control: test".to_string()));
        assert_eq!(
            listener_exit_stop_reason(Some(Ok(())), &state, &shutdown),
            "control: test"
        );
        assert_eq!(state.counters.proxy_errors.load(Ordering::Relaxed), 0);
    }

    async fn upstream_http() -> (std::net::SocketAddr, tokio::task::JoinHandle<()>) {
        let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
        let address = listener.local_addr().unwrap();
        let task = tokio::spawn(async move {
            let (stream, _) = listener.accept().await.unwrap();
            let service = service_fn(|request: Request<Incoming>| async move {
                assert_eq!(request.uri().path(), "/passthrough");
                assert_eq!(request.headers()["ok-access-key"], "opaque-key");
                let mut response = Response::new(Full::new(Bytes::from_static(b"upstream")));
                response
                    .headers_mut()
                    .insert("x-upstream", HeaderValue::from_static("yes"));
                Ok::<_, Infallible>(response)
            });
            http1::Builder::new()
                .keep_alive(false)
                .serve_connection(TokioIo::new(stream), service)
                .await
                .unwrap();
        });
        (address, task)
    }

    async fn upstream_chunked_http() -> (std::net::SocketAddr, tokio::task::JoinHandle<()>) {
        let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
        let address = listener.local_addr().unwrap();
        let task = tokio::spawn(async move {
            let (stream, _) = listener.accept().await.unwrap();
            let service = service_fn(|_request: Request<Incoming>| async move {
                let frames = vec![
                    Ok::<_, Infallible>(Frame::data(Bytes::from_static(b"123456"))),
                    Ok::<_, Infallible>(Frame::data(Bytes::from_static(b"789012"))),
                ];
                Ok::<_, Infallible>(Response::new(StreamBody::new(futures_util::stream::iter(
                    frames,
                ))))
            });
            http1::Builder::new()
                .keep_alive(false)
                .serve_connection(TokioIo::new(stream), service)
                .await
                .unwrap();
        });
        (address, task)
    }

    #[tokio::test]
    async fn rest_proxy_injects_once_writes_evidence_and_forwards_other_requests() {
        let directory = tempfile::tempdir().unwrap();
        let evidence_directory = private_evidence_directory(directory.path());
        let state = ProxyState::new(
            "fingerprint".to_string(),
            evidence_directory.clone(),
            8,
            1024 * 1024,
        );
        let (upstream_address, upstream_task) = upstream_http().await;
        let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
        let proxy_address = listener.local_addr().unwrap();
        let proxy_task = tokio::spawn(run_rest_listener(
            listener,
            Arc::clone(&state),
            reqwest::Client::new(),
            format!("http://{upstream_address}"),
            1024 * 1024,
        ));

        let response = state
            .arm_rest(
                "clock-failure".to_string(),
                "clock-failure.json".to_string(),
                RestRequestMatcher {
                    method: "GET".to_string(),
                    path: "/api/v5/public/time".to_string(),
                    query: BTreeMap::new(),
                },
                InjectedHttpResponse {
                    status: 503,
                    headers: BTreeMap::from([(
                        "content-type".to_string(),
                        "application/json".to_string(),
                    )]),
                    body: r#"{"code":"50000","msg":"injected"}"#.to_string(),
                },
                1,
            )
            .await;
        assert!(response.accepted);

        let client = reqwest::Client::new();
        let injected = client
            .get(format!("http://{proxy_address}/api/v5/public/time"))
            .send()
            .await
            .unwrap();
        assert_eq!(injected.status(), StatusCode::SERVICE_UNAVAILABLE);
        assert!(injected.text().await.unwrap().contains("injected"));
        let evidence_path = evidence_directory.join("clock-failure.json");
        let evidence: FaultInjectorEvidence =
            serde_json::from_slice(&fs::read(evidence_path).unwrap()).unwrap();
        assert!(evidence.passed);
        assert_eq!(evidence.effects.len(), 1);
        assert_eq!(evidence.java_reference_revision, PINNED_JAVA_REVISION);

        let forwarded = client
            .get(format!("http://{proxy_address}/passthrough"))
            .header("ok-access-key", "opaque-key")
            .send()
            .await
            .unwrap();
        assert_eq!(forwarded.headers()["x-upstream"], "yes");
        assert_eq!(forwarded.text().await.unwrap(), "upstream");
        assert_eq!(
            state
                .counters
                .rest_responses_injected
                .load(Ordering::Relaxed),
            1
        );
        assert_eq!(
            state
                .counters
                .rest_requests_forwarded
                .load(Ordering::Relaxed),
            1
        );

        let _ = state.shutdown.send(Some("test".to_string()));
        proxy_task.await.unwrap();
        upstream_task.await.unwrap();
    }

    #[tokio::test]
    async fn rest_proxy_bounds_chunked_upstream_responses_while_reading() {
        let directory = tempfile::tempdir().unwrap();
        let state = ProxyState::new(
            "fingerprint".to_string(),
            private_evidence_directory(directory.path()),
            8,
            8,
        );
        let (upstream_address, upstream_task) = upstream_chunked_http().await;
        let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
        let proxy_address = listener.local_addr().unwrap();
        let proxy_task = tokio::spawn(run_rest_listener(
            listener,
            Arc::clone(&state),
            reqwest::Client::new(),
            format!("http://{upstream_address}"),
            8,
        ));

        let response = reqwest::get(format!("http://{proxy_address}/chunked"))
            .await
            .unwrap();

        assert_eq!(response.status(), StatusCode::BAD_GATEWAY);
        assert_eq!(state.counters.proxy_errors.load(Ordering::Relaxed), 1);
        let _ = state.shutdown.send(Some("test".to_string()));
        proxy_task.await.unwrap();
        upstream_task.await.unwrap();
    }

    #[tokio::test]
    async fn websocket_proxy_drops_matching_frame_then_disconnects_target() {
        let directory = tempfile::tempdir().unwrap();
        let evidence_directory = private_evidence_directory(directory.path());
        let state = ProxyState::new(
            "fingerprint".to_string(),
            evidence_directory.clone(),
            8,
            1024 * 1024,
        );
        let upstream_listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
        let upstream_address = upstream_listener.local_addr().unwrap();
        let upstream_task = tokio::spawn(async move {
            let (stream, _) = upstream_listener.accept().await.unwrap();
            let mut websocket = accept_async(stream).await.unwrap();
            while let Some(message) = websocket.next().await {
                let message = message.unwrap();
                if message.is_close() {
                    break;
                }
                websocket.send(message).await.unwrap();
            }
        });
        let proxy_listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
        let proxy_address = proxy_listener.local_addr().unwrap();
        let proxy_task = tokio::spawn(run_websocket_listener(
            proxy_listener,
            Arc::clone(&state),
            WebSocketTarget::Order,
            format!("ws://{upstream_address}/ws/v5/private"),
        ));
        let (mut client, _) = connect_async(format!(
            "ws://{proxy_address}{}",
            WebSocketTarget::Order.expected_path()
        ))
        .await
        .unwrap();
        for _ in 0..20 {
            if state.status().await.websocket_connections_active["order"] == 1 {
                break;
            }
            tokio::time::sleep(Duration::from_millis(10)).await;
        }
        assert_eq!(
            state.status().await.websocket_connections_active["order"],
            1
        );

        let armed = state
            .arm_websocket_drop(
                "ambiguous-submit".to_string(),
                "ambiguous-submit.json".to_string(),
                WebSocketTarget::Order,
                WebSocketDirection::ClientToExchange,
                WebSocketFrameMatcher {
                    kind: WebSocketFrameKind::Text,
                    json: Some(WebSocketJsonMatcher {
                        op: Some("order".to_string()),
                        ..WebSocketJsonMatcher::default()
                    }),
                },
                1,
            )
            .await;
        assert!(armed.accepted);
        client
            .send(Message::Text(r#"{"op":"order","id":"opaque"}"#.into()))
            .await
            .unwrap();
        for _ in 0..20 {
            if evidence_directory.join("ambiguous-submit.json").exists() {
                break;
            }
            tokio::time::sleep(Duration::from_millis(10)).await;
        }
        let evidence: FaultInjectorEvidence = serde_json::from_slice(
            &fs::read(evidence_directory.join("ambiguous-submit.json")).unwrap(),
        )
        .unwrap();
        assert!(evidence.passed);

        client
            .send(Message::Text(r#"{"op":"ping"}"#.into()))
            .await
            .unwrap();
        let echoed = tokio::time::timeout(Duration::from_secs(1), client.next())
            .await
            .unwrap()
            .unwrap()
            .unwrap();
        assert_eq!(echoed.into_text().unwrap(), r#"{"op":"ping"}"#);

        let disconnected = state
            .disconnect_websockets(
                "order-reconnect".to_string(),
                "order-reconnect.json".to_string(),
                WebSocketTarget::Order,
                1,
                Duration::from_secs(1),
            )
            .await;
        assert!(disconnected.accepted, "{}", disconnected.message);
        let evidence: FaultInjectorEvidence = serde_json::from_slice(
            &fs::read(evidence_directory.join("order-reconnect.json")).unwrap(),
        )
        .unwrap();
        assert!(evidence.passed);
        assert_eq!(evidence.effects.len(), 1);

        let close = tokio::time::timeout(Duration::from_secs(1), client.next())
            .await
            .unwrap()
            .unwrap()
            .unwrap();
        assert!(close.is_close());
        let _ = state.shutdown.send(Some("test".to_string()));
        proxy_task.await.unwrap();
        upstream_task.await.unwrap();
    }
}
