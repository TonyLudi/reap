use std::collections::{BTreeMap, HashMap, VecDeque};
use std::path::PathBuf;
use std::sync::atomic::{AtomicU64, Ordering};
use std::time::{Duration, SystemTime, UNIX_EPOCH};

use base64::Engine;
use base64::engine::general_purpose::URL_SAFE_NO_PAD;
use hmac::{Hmac, Mac};
use serde::{Deserialize, Serialize};
use sha2::Sha256;
use thiserror::Error;
use tokio::sync::{mpsc, oneshot};

use crate::{LiveConfigError, OperatorConfig, ReadinessSnapshot};

const PROTOCOL_VERSION: u16 = 2;
const MIN_SECRET_BYTES: usize = 32;
const MAX_RESPONSE_BYTES: usize = 65_536;
static REQUEST_SEQUENCE: AtomicU64 = AtomicU64::new(0);

type HmacSha256 = Hmac<Sha256>;

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "command", rename_all = "snake_case")]
pub enum OperatorCommand {
    Status,
    KillSwitch { reason: String },
    KillAccount { account_id: String, reason: String },
    HaltSymbol { symbol: String, reason: String },
    ResumeSymbol { symbol: String, reason: String },
    Shutdown { reason: String },
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct OperatorStatus {
    pub readiness: ReadinessSnapshot,
    pub active_orders: usize,
    pub kill_switch_active: bool,
    pub halted_accounts: BTreeMap<String, String>,
    pub shutdown_in_progress: bool,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct OperatorResponse {
    pub version: u16,
    pub request_id: String,
    pub ok: bool,
    pub message: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub status: Option<OperatorStatus>,
}

impl OperatorResponse {
    pub(crate) fn accepted(
        request_id: impl Into<String>,
        message: impl Into<String>,
        status: Option<OperatorStatus>,
    ) -> Self {
        Self {
            version: PROTOCOL_VERSION,
            request_id: request_id.into(),
            ok: true,
            message: message.into(),
            status,
        }
    }

    pub(crate) fn rejected(request_id: impl Into<String>, message: impl Into<String>) -> Self {
        Self {
            version: PROTOCOL_VERSION,
            request_id: request_id.into(),
            ok: false,
            message: message.into(),
            status: None,
        }
    }
}

#[derive(Debug)]
pub(crate) struct OperatorEnvelope {
    pub request_id: String,
    pub command: OperatorCommand,
    pub response: oneshot::Sender<OperatorResponse>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
struct SignedOperatorRequest {
    version: u16,
    request_id: String,
    timestamp_ms: u64,
    nonce: String,
    command: OperatorCommand,
    signature: String,
}

#[derive(Serialize)]
struct SigningPayload<'a> {
    version: u16,
    request_id: &'a str,
    timestamp_ms: u64,
    nonce: &'a str,
    command: &'a OperatorCommand,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
struct SignedOperatorResponse {
    response: OperatorResponse,
    signature: String,
}

#[derive(Serialize)]
struct ResponseSigningPayload<'a> {
    version: u16,
    request_id: &'a str,
    ok: bool,
    message: &'a str,
    status: &'a Option<OperatorStatus>,
}

#[derive(Debug, Error)]
pub enum OperatorError {
    #[error(transparent)]
    Config(#[from] LiveConfigError),
    #[error("operator service is disabled")]
    Disabled,
    #[error("operator service is supported only on Unix platforms")]
    UnsupportedPlatform,
    #[error("operator socket path already exists and is not a socket: {0}")]
    UnsafeSocketPath(PathBuf),
    #[error("operator socket already has an active listener: {0}")]
    SocketInUse(PathBuf),
    #[error("operator request exceeded the configured size limit")]
    RequestTooLarge,
    #[error("operator response exceeded the protocol size limit")]
    ResponseTooLarge,
    #[error("operator request framing is invalid")]
    InvalidFraming,
    #[error("operator request is invalid: {0}")]
    InvalidRequest(String),
    #[error("operator request authentication failed")]
    Authentication,
    #[error("operator request timestamp is outside the allowed clock skew")]
    StaleRequest,
    #[error("operator request nonce has already been used")]
    Replay,
    #[error("operator runtime command channel is unavailable")]
    RuntimeUnavailable,
    #[error("operator request timed out")]
    Timeout,
    #[error("operator response channel closed")]
    ResponseClosed,
    #[error("operator response request id did not match")]
    ResponseMismatch,
    #[error("operator IO failed: {0}")]
    Io(#[from] std::io::Error),
    #[error("operator JSON failed: {0}")]
    Json(#[from] serde_json::Error),
    #[error("operator task failed: {0}")]
    Join(#[from] tokio::task::JoinError),
}

#[derive(Debug)]
struct ReplayCache {
    entries: HashMap<String, u64>,
    order: VecDeque<(u64, String)>,
    ttl_ms: u64,
    capacity: usize,
}

impl ReplayCache {
    fn new(ttl_ms: u64, capacity: usize) -> Self {
        Self {
            entries: HashMap::new(),
            order: VecDeque::new(),
            ttl_ms,
            capacity,
        }
    }

    fn check_and_insert(&mut self, nonce: &str, now_ms: u64) -> Result<(), OperatorError> {
        while let Some((inserted_ms, nonce)) = self.order.front() {
            if now_ms.saturating_sub(*inserted_ms) <= self.ttl_ms {
                break;
            }
            let nonce = nonce.clone();
            self.order.pop_front();
            self.entries.remove(&nonce);
        }
        if self.entries.contains_key(nonce) {
            return Err(OperatorError::Replay);
        }
        while self.entries.len() >= self.capacity {
            let Some((_, oldest)) = self.order.pop_front() else {
                break;
            };
            self.entries.remove(&oldest);
        }
        self.entries.insert(nonce.to_string(), now_ms);
        self.order.push_back((now_ms, nonce.to_string()));
        Ok(())
    }
}

fn sign_request(
    command: OperatorCommand,
    secret: &[u8],
    request_id: String,
    timestamp_ms: u64,
    nonce: String,
) -> Result<SignedOperatorRequest, OperatorError> {
    validate_secret(secret)?;
    validate_request_fields(&request_id, &nonce, &command)?;
    let payload = SigningPayload {
        version: PROTOCOL_VERSION,
        request_id: &request_id,
        timestamp_ms,
        nonce: &nonce,
        command: &command,
    };
    let bytes = serde_json::to_vec(&payload)?;
    let mut mac = HmacSha256::new_from_slice(secret).map_err(|_| OperatorError::Authentication)?;
    mac.update(&bytes);
    let signature = URL_SAFE_NO_PAD.encode(mac.finalize().into_bytes());
    Ok(SignedOperatorRequest {
        version: PROTOCOL_VERSION,
        request_id,
        timestamp_ms,
        nonce,
        command,
        signature,
    })
}

fn authenticate_request(
    request: &SignedOperatorRequest,
    secret: &[u8],
    now_ms: u64,
    max_clock_skew_ms: u64,
    replay: &mut ReplayCache,
) -> Result<(), OperatorError> {
    validate_secret(secret)?;
    if request.version != PROTOCOL_VERSION {
        return Err(OperatorError::InvalidRequest(
            "unsupported protocol version".to_string(),
        ));
    }
    validate_request_fields(&request.request_id, &request.nonce, &request.command)?;
    if now_ms.abs_diff(request.timestamp_ms) > max_clock_skew_ms {
        return Err(OperatorError::StaleRequest);
    }
    let payload = SigningPayload {
        version: request.version,
        request_id: &request.request_id,
        timestamp_ms: request.timestamp_ms,
        nonce: &request.nonce,
        command: &request.command,
    };
    let bytes = serde_json::to_vec(&payload)?;
    let signature = URL_SAFE_NO_PAD
        .decode(request.signature.as_bytes())
        .map_err(|_| OperatorError::Authentication)?;
    let mut mac = HmacSha256::new_from_slice(secret).map_err(|_| OperatorError::Authentication)?;
    mac.update(&bytes);
    mac.verify_slice(&signature)
        .map_err(|_| OperatorError::Authentication)?;
    replay.check_and_insert(&request.nonce, now_ms)
}

fn sign_response(
    response: OperatorResponse,
    secret: &[u8],
) -> Result<SignedOperatorResponse, OperatorError> {
    validate_secret(secret)?;
    let payload = ResponseSigningPayload {
        version: response.version,
        request_id: &response.request_id,
        ok: response.ok,
        message: &response.message,
        status: &response.status,
    };
    let bytes = serde_json::to_vec(&payload)?;
    let mut mac = HmacSha256::new_from_slice(secret).map_err(|_| OperatorError::Authentication)?;
    mac.update(&bytes);
    Ok(SignedOperatorResponse {
        response,
        signature: URL_SAFE_NO_PAD.encode(mac.finalize().into_bytes()),
    })
}

fn verify_response(
    signed: SignedOperatorResponse,
    secret: &[u8],
) -> Result<OperatorResponse, OperatorError> {
    validate_secret(secret)?;
    let payload = ResponseSigningPayload {
        version: signed.response.version,
        request_id: &signed.response.request_id,
        ok: signed.response.ok,
        message: &signed.response.message,
        status: &signed.response.status,
    };
    let bytes = serde_json::to_vec(&payload)?;
    let signature = URL_SAFE_NO_PAD
        .decode(signed.signature.as_bytes())
        .map_err(|_| OperatorError::Authentication)?;
    let mut mac = HmacSha256::new_from_slice(secret).map_err(|_| OperatorError::Authentication)?;
    mac.update(&bytes);
    mac.verify_slice(&signature)
        .map_err(|_| OperatorError::Authentication)?;
    Ok(signed.response)
}

fn validate_secret(secret: &[u8]) -> Result<(), OperatorError> {
    if secret.len() < MIN_SECRET_BYTES {
        return Err(OperatorError::InvalidRequest(
            "operator secret must contain at least 32 bytes".to_string(),
        ));
    }
    Ok(())
}

fn validate_request_fields(
    request_id: &str,
    nonce: &str,
    command: &OperatorCommand,
) -> Result<(), OperatorError> {
    validate_identifier("request_id", request_id)?;
    validate_identifier("nonce", nonce)?;
    match command {
        OperatorCommand::Status => {}
        OperatorCommand::KillSwitch { reason } | OperatorCommand::Shutdown { reason } => {
            validate_reason(reason)?;
        }
        OperatorCommand::KillAccount { account_id, reason } => {
            validate_account_id(account_id)?;
            validate_reason(reason)?;
        }
        OperatorCommand::HaltSymbol { symbol, reason }
        | OperatorCommand::ResumeSymbol { symbol, reason } => {
            if symbol.is_empty()
                || symbol.len() > 64
                || !symbol
                    .bytes()
                    .all(|byte| byte.is_ascii_alphanumeric() || matches!(byte, b'-' | b'_' | b'/'))
            {
                return Err(OperatorError::InvalidRequest(
                    "symbol must contain 1-64 safe ASCII characters".to_string(),
                ));
            }
            validate_reason(reason)?;
        }
    }
    Ok(())
}

fn validate_identifier(name: &str, value: &str) -> Result<(), OperatorError> {
    if value.is_empty()
        || value.len() > 128
        || !value
            .bytes()
            .all(|byte| byte.is_ascii_alphanumeric() || matches!(byte, b'-' | b'_' | b'.'))
    {
        return Err(OperatorError::InvalidRequest(format!(
            "{name} must contain 1-128 safe ASCII characters"
        )));
    }
    Ok(())
}

fn validate_account_id(value: &str) -> Result<(), OperatorError> {
    if value.trim().is_empty() || value.len() > 128 || value.chars().any(char::is_control) {
        return Err(OperatorError::InvalidRequest(
            "account_id must contain 1-128 printable characters".to_string(),
        ));
    }
    Ok(())
}

fn validate_reason(reason: &str) -> Result<(), OperatorError> {
    if reason.trim().is_empty() || reason.len() > 256 || reason.chars().any(char::is_control) {
        return Err(OperatorError::InvalidRequest(
            "reason must contain 1-256 printable characters".to_string(),
        ));
    }
    Ok(())
}

fn request_identity() -> (String, String) {
    let sequence = REQUEST_SEQUENCE.fetch_add(1, Ordering::Relaxed);
    let nanos = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap_or_default()
        .as_nanos();
    let identity = format!("{:x}-{:x}-{:x}", std::process::id(), nanos, sequence);
    (format!("request-{identity}"), format!("nonce-{identity}"))
}

fn unix_time_ms() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap_or_default()
        .as_millis()
        .min(u64::MAX as u128) as u64
}

#[cfg(unix)]
mod unix {
    use std::os::unix::fs::{FileTypeExt, PermissionsExt};
    use std::os::unix::net::UnixStream as StdUnixStream;
    use std::path::Path;
    use std::sync::Arc;

    use tokio::io::{AsyncBufReadExt, AsyncReadExt, AsyncWriteExt, BufReader};
    use tokio::net::{UnixListener, UnixStream};
    use tokio::sync::Mutex;
    use tokio::task::{JoinHandle, JoinSet};
    use tokio::time::timeout;

    use super::*;

    pub(crate) struct OperatorService {
        socket_path: PathBuf,
        shutdown: Option<oneshot::Sender<()>>,
        task: Option<JoinHandle<Result<(), OperatorError>>>,
    }

    impl OperatorService {
        pub(crate) fn request_shutdown(&mut self) {
            if let Some(shutdown) = self.shutdown.take() {
                let _ = shutdown.send(());
            }
        }

        pub(crate) async fn shutdown(mut self) -> Result<(), OperatorError> {
            self.request_shutdown();
            let task_result = match self.task.as_mut() {
                Some(task) => match task.await {
                    Ok(result) => result,
                    Err(error) => Err(error.into()),
                },
                None => Ok(()),
            };
            self.task.take();
            let remove_result = remove_socket_if_present(&self.socket_path);
            task_result?;
            remove_result
        }
    }

    impl Drop for OperatorService {
        fn drop(&mut self) {
            self.request_shutdown();
            if let Some(task) = &self.task {
                task.abort();
            }
            let _ = remove_socket_if_present(&self.socket_path);
        }
    }

    pub(crate) async fn start_operator_service(
        config: &OperatorConfig,
        secret: Vec<u8>,
        events: mpsc::Sender<OperatorEnvelope>,
    ) -> Result<OperatorService, OperatorError> {
        if !config.enabled {
            return Err(OperatorError::Disabled);
        }
        validate_secret(&secret)?;
        prepare_socket_path(&config.socket_path)?;
        let listener = UnixListener::bind(&config.socket_path)?;
        std::fs::set_permissions(&config.socket_path, std::fs::Permissions::from_mode(0o600))?;
        let socket_path = config.socket_path.clone();
        let task_socket_path = socket_path.clone();
        let request_timeout = Duration::from_millis(config.request_timeout_ms);
        let max_request_bytes = config.max_request_bytes;
        let max_clock_skew_ms = config.max_clock_skew_ms;
        let replay = Arc::new(Mutex::new(ReplayCache::new(
            config.nonce_ttl_ms,
            config.nonce_capacity,
        )));
        let secret: Arc<[u8]> = secret.into();
        let (shutdown_tx, mut shutdown_rx) = oneshot::channel();
        let task = tokio::spawn(async move {
            let mut connections = JoinSet::new();
            loop {
                tokio::select! {
                    biased;
                    _ = &mut shutdown_rx => break,
                    Some(result) = connections.join_next(), if !connections.is_empty() => {
                        if let Err(error) = result {
                            tracing::warn!(%error, "operator connection task failed");
                        }
                    }
                    accepted = listener.accept() => {
                        let (stream, _) = accepted?;
                        let secret = Arc::clone(&secret);
                        let replay = Arc::clone(&replay);
                        let events = events.clone();
                        connections.spawn(async move {
                            if let Err(error) = handle_connection(
                                stream,
                                &secret,
                                replay,
                                events,
                                request_timeout,
                                max_request_bytes,
                                max_clock_skew_ms,
                            ).await {
                                tracing::warn!(%error, "operator request failed");
                            }
                        });
                    }
                }
            }
            let drained = timeout(request_timeout, async {
                while let Some(result) = connections.join_next().await {
                    if let Err(error) = result {
                        tracing::warn!(%error, "operator connection task failed during shutdown");
                    }
                }
            })
            .await
            .is_ok();
            if !drained {
                connections.abort_all();
                while connections.join_next().await.is_some() {}
            }
            remove_socket_if_present(&task_socket_path)?;
            Ok(())
        });
        Ok(OperatorService {
            socket_path,
            shutdown: Some(shutdown_tx),
            task: Some(task),
        })
    }

    async fn handle_connection(
        stream: UnixStream,
        secret: &[u8],
        replay: Arc<Mutex<ReplayCache>>,
        events: mpsc::Sender<OperatorEnvelope>,
        request_timeout: Duration,
        max_request_bytes: usize,
        max_clock_skew_ms: u64,
    ) -> Result<(), OperatorError> {
        let mut reader = BufReader::new(stream);
        let bytes = timeout(
            request_timeout,
            read_bounded_line(&mut reader, max_request_bytes),
        )
        .await
        .map_err(|_| OperatorError::Timeout)??;
        let parsed = serde_json::from_slice::<SignedOperatorRequest>(&bytes);
        let response = match parsed {
            Ok(request) => {
                let authentication = {
                    let mut replay = replay.lock().await;
                    authenticate_request(
                        &request,
                        secret,
                        unix_time_ms(),
                        max_clock_skew_ms,
                        &mut replay,
                    )
                };
                match authentication {
                    Ok(()) => {
                        let (response_tx, response_rx) = oneshot::channel();
                        let request_id = request.request_id.clone();
                        timeout(
                            request_timeout,
                            events.send(OperatorEnvelope {
                                request_id: request.request_id,
                                command: request.command,
                                response: response_tx,
                            }),
                        )
                        .await
                        .map_err(|_| OperatorError::Timeout)?
                        .map_err(|_| OperatorError::RuntimeUnavailable)?;
                        let response = timeout(request_timeout, response_rx)
                            .await
                            .map_err(|_| OperatorError::Timeout)?
                            .map_err(|_| OperatorError::ResponseClosed)?;
                        if response.request_id != request_id {
                            return Err(OperatorError::ResponseMismatch);
                        }
                        response
                    }
                    Err(error) => {
                        tracing::warn!(
                            request_id = %request.request_id,
                            %error,
                            "operator authentication rejected"
                        );
                        OperatorResponse::rejected(request.request_id, error.to_string())
                    }
                }
            }
            Err(_) => OperatorResponse::rejected("unknown", "invalid operator request"),
        };
        let response = sign_response(response, secret)?;
        timeout(request_timeout, write_response(reader.get_mut(), &response))
            .await
            .map_err(|_| OperatorError::Timeout)?
    }

    async fn read_bounded_line(
        reader: &mut BufReader<UnixStream>,
        max_request_bytes: usize,
    ) -> Result<Vec<u8>, OperatorError> {
        let mut bytes = Vec::with_capacity(max_request_bytes.min(4_096));
        let mut limited = reader.take((max_request_bytes + 1) as u64);
        let read = limited.read_until(b'\n', &mut bytes).await?;
        if read == 0 || bytes.last() != Some(&b'\n') {
            return Err(OperatorError::InvalidFraming);
        }
        if bytes.len() > max_request_bytes {
            return Err(OperatorError::RequestTooLarge);
        }
        bytes.pop();
        Ok(bytes)
    }

    async fn write_response(
        stream: &mut UnixStream,
        response: &SignedOperatorResponse,
    ) -> Result<(), OperatorError> {
        let mut bytes = serde_json::to_vec(response)?;
        if bytes.len() + 1 > MAX_RESPONSE_BYTES {
            return Err(OperatorError::ResponseTooLarge);
        }
        bytes.push(b'\n');
        stream.write_all(&bytes).await?;
        stream.shutdown().await?;
        Ok(())
    }

    pub async fn send_operator_command_with_secret(
        config: &OperatorConfig,
        secret: &[u8],
        command: OperatorCommand,
    ) -> Result<OperatorResponse, OperatorError> {
        if !config.enabled {
            return Err(OperatorError::Disabled);
        }
        let (request_id, nonce) = request_identity();
        let request = sign_request(command, secret, request_id.clone(), unix_time_ms(), nonce)?;
        let request_timeout = Duration::from_millis(config.request_timeout_ms);
        let stream = timeout(request_timeout, UnixStream::connect(&config.socket_path))
            .await
            .map_err(|_| OperatorError::Timeout)??;
        let mut reader = BufReader::new(stream);
        let mut bytes = serde_json::to_vec(&request)?;
        bytes.push(b'\n');
        timeout(request_timeout, reader.get_mut().write_all(&bytes))
            .await
            .map_err(|_| OperatorError::Timeout)??;
        let response = timeout(
            request_timeout,
            read_bounded_line(&mut reader, MAX_RESPONSE_BYTES),
        )
        .await
        .map_err(|_| OperatorError::Timeout)??;
        let response: SignedOperatorResponse = serde_json::from_slice(&response)?;
        let response = verify_response(response, secret)?;
        if response.version != PROTOCOL_VERSION || response.request_id != request_id {
            return Err(OperatorError::ResponseMismatch);
        }
        Ok(response)
    }

    fn prepare_socket_path(path: &Path) -> Result<(), OperatorError> {
        if let Some(parent) = path.parent()
            && !parent.as_os_str().is_empty()
        {
            std::fs::create_dir_all(parent)?;
        }
        match std::fs::symlink_metadata(path) {
            Ok(metadata) if metadata.file_type().is_socket() => {
                match StdUnixStream::connect(path) {
                    Ok(_) => return Err(OperatorError::SocketInUse(path.to_path_buf())),
                    Err(error)
                        if matches!(
                            error.kind(),
                            std::io::ErrorKind::ConnectionRefused | std::io::ErrorKind::NotFound
                        ) =>
                    {
                        std::fs::remove_file(path)?;
                    }
                    Err(error) => return Err(error.into()),
                }
            }
            Ok(_) => return Err(OperatorError::UnsafeSocketPath(path.to_path_buf())),
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => {}
            Err(error) => return Err(error.into()),
        }
        Ok(())
    }

    fn remove_socket_if_present(path: &Path) -> Result<(), OperatorError> {
        match std::fs::symlink_metadata(path) {
            Ok(metadata) if metadata.file_type().is_socket() => {
                std::fs::remove_file(path)?;
            }
            Ok(_) => return Err(OperatorError::UnsafeSocketPath(path.to_path_buf())),
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => {}
            Err(error) => return Err(error.into()),
        }
        Ok(())
    }
}

#[cfg(all(test, unix))]
pub(crate) use unix::send_operator_command_with_secret;
#[cfg(unix)]
pub(crate) use unix::{OperatorService, start_operator_service};

#[cfg(not(unix))]
pub(crate) struct OperatorService;

#[cfg(not(unix))]
impl OperatorService {
    pub(crate) fn request_shutdown(&mut self) {}

    pub(crate) async fn shutdown(self) -> Result<(), OperatorError> {
        Err(OperatorError::UnsupportedPlatform)
    }
}

#[cfg(not(unix))]
pub(crate) async fn start_operator_service(
    _config: &OperatorConfig,
    _secret: Vec<u8>,
    _events: mpsc::Sender<OperatorEnvelope>,
) -> Result<OperatorService, OperatorError> {
    Err(OperatorError::UnsupportedPlatform)
}

#[cfg(unix)]
pub async fn send_operator_command(
    config: &OperatorConfig,
    command: OperatorCommand,
) -> Result<OperatorResponse, OperatorError> {
    let secret = config.secret_from_env()?.ok_or(OperatorError::Disabled)?;
    unix::send_operator_command_with_secret(config, &secret, command).await
}

#[cfg(not(unix))]
pub async fn send_operator_command(
    _config: &OperatorConfig,
    _command: OperatorCommand,
) -> Result<OperatorResponse, OperatorError> {
    Err(OperatorError::UnsupportedPlatform)
}

#[cfg(test)]
mod tests {
    use super::*;

    const SECRET: &[u8] = b"0123456789abcdef0123456789abcdef";

    #[test]
    fn signatures_detect_tampering_staleness_and_replay() {
        let now_ms = 10_000;
        let mut request = sign_request(
            OperatorCommand::KillSwitch {
                reason: "test".to_string(),
            },
            SECRET,
            "request-1".to_string(),
            now_ms,
            "nonce-1".to_string(),
        )
        .unwrap();
        let mut replay = ReplayCache::new(60_000, 16);

        authenticate_request(&request, SECRET, now_ms, 1_000, &mut replay).unwrap();
        assert!(matches!(
            authenticate_request(&request, SECRET, now_ms, 1_000, &mut replay),
            Err(OperatorError::Replay)
        ));

        request.nonce = "nonce-2".to_string();
        let mut replay = ReplayCache::new(60_000, 16);
        assert!(matches!(
            authenticate_request(&request, SECRET, now_ms, 1_000, &mut replay),
            Err(OperatorError::Authentication)
        ));

        let stale = sign_request(
            OperatorCommand::Status,
            SECRET,
            "request-2".to_string(),
            1,
            "nonce-3".to_string(),
        )
        .unwrap();
        assert!(matches!(
            authenticate_request(&stale, SECRET, now_ms, 1_000, &mut replay),
            Err(OperatorError::StaleRequest)
        ));

        let mut response = sign_response(
            OperatorResponse::accepted("request-3", "accepted", None),
            SECRET,
        )
        .unwrap();
        response.response.message = "tampered".to_string();
        assert!(matches!(
            verify_response(response, SECRET),
            Err(OperatorError::Authentication)
        ));

        assert!(matches!(
            sign_request(
                OperatorCommand::KillAccount {
                    account_id: "unsafe\naccount".to_string(),
                    reason: "test".to_string(),
                },
                SECRET,
                "request-4".to_string(),
                now_ms,
                "nonce-4".to_string(),
            ),
            Err(OperatorError::InvalidRequest(_))
        ));
    }

    #[cfg(unix)]
    #[tokio::test]
    async fn unix_service_authenticates_and_round_trips_one_command() {
        let socket_path = std::env::temp_dir().join(format!(
            "reap-operator-{}-{}.sock",
            std::process::id(),
            unix_time_ms()
        ));
        let config = OperatorConfig {
            enabled: true,
            socket_path: socket_path.clone(),
            request_timeout_ms: 1_000,
            ..OperatorConfig::default()
        };
        let (events_tx, mut events_rx) = mpsc::channel(4);
        let service = start_operator_service(&config, SECRET.to_vec(), events_tx)
            .await
            .unwrap();
        let (second_events, _second_rx) = mpsc::channel(1);
        assert!(matches!(
            start_operator_service(&config, SECRET.to_vec(), second_events).await,
            Err(OperatorError::SocketInUse(path)) if path == socket_path
        ));
        let rejected = unix::send_operator_command_with_secret(
            &config,
            b"abcdef0123456789abcdef0123456789",
            OperatorCommand::Status,
        )
        .await
        .unwrap_err();
        assert!(matches!(rejected, OperatorError::Authentication));
        assert!(matches!(
            events_rx.try_recv(),
            Err(mpsc::error::TryRecvError::Empty)
        ));
        let responder = tokio::spawn(async move {
            let envelope = events_rx.recv().await.unwrap();
            assert_eq!(envelope.command, OperatorCommand::Status);
            envelope
                .response
                .send(OperatorResponse::accepted(
                    envelope.request_id,
                    "ready",
                    None,
                ))
                .unwrap();
        });

        let response =
            unix::send_operator_command_with_secret(&config, SECRET, OperatorCommand::Status)
                .await
                .unwrap();

        assert!(response.ok);
        assert_eq!(response.message, "ready");
        responder.await.unwrap();
        service.shutdown().await.unwrap();
        assert!(!socket_path.exists());
    }
}
