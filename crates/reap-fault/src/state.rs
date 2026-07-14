use std::collections::{BTreeMap, HashMap, HashSet, VecDeque};
use std::fs::{self, OpenOptions};
use std::io::Write;
use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::sync::atomic::{AtomicU64, Ordering};
use std::time::{Duration, SystemTime, UNIX_EPOCH};

use hyper::header::{HeaderName, HeaderValue};
use serde_json::Value;
use sha2::{Digest, Sha256};
use tokio::sync::{Mutex, mpsc, oneshot, watch};
use tokio_tungstenite::tungstenite::Message;

use crate::protocol::{
    CONTROL_FORMAT_VERSION, FaultCommandSummary, FaultEffect, FaultEvidenceContext,
    FaultInjectorEvidence, FaultProxyControlResponse, FaultProxyStatus, InjectedHttpResponse,
    RestRequestMatcher, WebSocketDirection, WebSocketFrameKind, WebSocketFrameMatcher,
    WebSocketJsonMatcher, WebSocketTarget,
};

#[derive(Debug, Default)]
pub(crate) struct ProxyCounters {
    pub rest_requests_forwarded: AtomicU64,
    pub rest_responses_injected: AtomicU64,
    pub websocket_connections_accepted: AtomicU64,
    pub websocket_frames_forwarded: AtomicU64,
    pub websocket_frames_dropped: AtomicU64,
    pub websocket_disconnects_injected: AtomicU64,
    pub completed_faults: AtomicU64,
    pub proxy_errors: AtomicU64,
}

#[derive(Debug)]
pub(crate) struct ProxyState {
    pub session_id: String,
    pub config_fingerprint: String,
    pub evidence_directory: PathBuf,
    pub max_pending_faults: usize,
    pub max_http_body_bytes: usize,
    pub counters: ProxyCounters,
    errors: std::sync::Mutex<VecDeque<String>>,
    registry: Mutex<FaultRegistry>,
    connections: Mutex<BTreeMap<u64, ConnectionControl>>,
    next_connection_id: AtomicU64,
    pub shutdown: watch::Sender<Option<String>>,
}

#[derive(Debug)]
struct FaultRegistry {
    command_ids: HashSet<String>,
    evidence_files: HashSet<String>,
    rest: Vec<ArmedRestFault>,
    websocket: Vec<ArmedWebSocketFault>,
}

#[derive(Debug)]
struct ArmedRestFault {
    command_id: String,
    evidence_file: String,
    matcher: RestRequestMatcher,
    response: InjectedHttpResponse,
    requested: u32,
    remaining: u32,
    armed_at_ms: u64,
    effects: Vec<FaultEffect>,
}

#[derive(Debug)]
struct ArmedWebSocketFault {
    command_id: String,
    evidence_file: String,
    target: WebSocketTarget,
    direction: WebSocketDirection,
    matcher: WebSocketFrameMatcher,
    requested: u32,
    remaining: u32,
    armed_at_ms: u64,
    effects: Vec<FaultEffect>,
}

#[derive(Debug, Clone)]
struct ConnectionControl {
    target: WebSocketTarget,
    disconnect: mpsc::Sender<DisconnectSignal>,
}

#[derive(Debug)]
pub(crate) struct DisconnectSignal {
    pub command_id: String,
    pub acknowledgement: oneshot::Sender<u64>,
}

#[derive(Debug)]
pub(crate) struct RestInjection {
    pub response: InjectedHttpResponse,
    pub completion: Option<FaultCompletion>,
}

#[derive(Debug)]
pub(crate) struct WebSocketDrop {
    pub completion: Option<FaultCompletion>,
}

#[derive(Debug)]
pub(crate) struct FaultCompletion {
    evidence_file: String,
    evidence: FaultInjectorEvidence,
}

impl ProxyState {
    pub fn new(
        config_fingerprint: String,
        evidence_directory: PathBuf,
        max_pending_faults: usize,
        max_http_body_bytes: usize,
    ) -> Arc<Self> {
        let (shutdown, _) = watch::channel(None);
        Arc::new(Self {
            session_id: new_session_id(),
            config_fingerprint,
            evidence_directory,
            max_pending_faults,
            max_http_body_bytes,
            counters: ProxyCounters::default(),
            errors: std::sync::Mutex::new(VecDeque::with_capacity(32)),
            registry: Mutex::new(FaultRegistry {
                command_ids: HashSet::new(),
                evidence_files: HashSet::new(),
                rest: Vec::new(),
                websocket: Vec::new(),
            }),
            connections: Mutex::new(BTreeMap::new()),
            next_connection_id: AtomicU64::new(1),
            shutdown,
        })
    }

    pub async fn register_connection(
        &self,
        target: WebSocketTarget,
        disconnect: mpsc::Sender<DisconnectSignal>,
    ) -> u64 {
        let id = self.next_connection_id.fetch_add(1, Ordering::Relaxed);
        self.connections
            .lock()
            .await
            .insert(id, ConnectionControl { target, disconnect });
        self.counters
            .websocket_connections_accepted
            .fetch_add(1, Ordering::Relaxed);
        id
    }

    pub async fn unregister_connection(&self, connection_id: u64) {
        self.connections.lock().await.remove(&connection_id);
    }

    pub async fn arm_rest(
        &self,
        command_id: String,
        evidence_file: String,
        matcher: RestRequestMatcher,
        response: InjectedHttpResponse,
        times: u32,
    ) -> FaultProxyControlResponse {
        if let Err(message) =
            validate_rest_fault(&matcher, &response, times, self.max_http_body_bytes)
        {
            return rejected(message);
        }
        let mut registry = self.registry.lock().await;
        if let Err(message) = self.reserve(&mut registry, &command_id, &evidence_file) {
            return rejected(message);
        }
        let evidence_path = self.evidence_directory.join(&evidence_file);
        registry.rest.push(ArmedRestFault {
            command_id,
            evidence_file,
            matcher,
            response,
            requested: times,
            remaining: times,
            armed_at_ms: now_ms(),
            effects: Vec::with_capacity(times as usize),
        });
        accepted(
            "REST response fault armed; evidence is created after every requested match",
            Some(evidence_path),
        )
    }

    #[allow(clippy::too_many_arguments)]
    pub async fn arm_websocket_drop(
        &self,
        command_id: String,
        evidence_file: String,
        target: WebSocketTarget,
        direction: WebSocketDirection,
        matcher: WebSocketFrameMatcher,
        frames: u32,
    ) -> FaultProxyControlResponse {
        if let Err(message) = validate_websocket_fault(&matcher, frames) {
            return rejected(message);
        }
        let mut registry = self.registry.lock().await;
        if let Err(message) = self.reserve(&mut registry, &command_id, &evidence_file) {
            return rejected(message);
        }
        let evidence_path = self.evidence_directory.join(&evidence_file);
        registry.websocket.push(ArmedWebSocketFault {
            command_id,
            evidence_file,
            target,
            direction,
            matcher,
            requested: frames,
            remaining: frames,
            armed_at_ms: now_ms(),
            effects: Vec::with_capacity(frames as usize),
        });
        accepted(
            "websocket frame-drop fault armed; evidence is created after every requested match",
            Some(evidence_path),
        )
    }

    pub async fn disconnect_websockets(
        &self,
        command_id: String,
        evidence_file: String,
        target: WebSocketTarget,
        requested: usize,
        acknowledgement_timeout: Duration,
    ) -> FaultProxyControlResponse {
        if requested == 0 || requested > 1024 {
            return rejected("connections must be in 1..=1024");
        }
        if let Err(message) = validate_identity(&command_id, &evidence_file) {
            return rejected(message);
        }
        let controls = self
            .connections
            .lock()
            .await
            .iter()
            .filter(|(_, connection)| connection.target == target)
            .take(requested)
            .map(|(id, connection)| (*id, connection.disconnect.clone()))
            .collect::<Vec<_>>();
        if controls.len() != requested {
            return rejected(format!(
                "requested {requested} {target:?} disconnects but only {} connections are active",
                controls.len()
            ));
        }
        {
            let mut registry = self.registry.lock().await;
            if let Err(message) = self.reserve(&mut registry, &command_id, &evidence_file) {
                return rejected(message);
            }
        }

        let armed_at_ms = now_ms();
        let mut acknowledgements = Vec::with_capacity(requested);
        for (connection_id, sender) in controls {
            let (acknowledgement, receiver) = oneshot::channel();
            if sender
                .send(DisconnectSignal {
                    command_id: command_id.clone(),
                    acknowledgement,
                })
                .await
                .is_ok()
            {
                acknowledgements.push((connection_id, receiver));
            }
        }
        let mut effects = Vec::with_capacity(requested);
        let collect_acknowledgements = async {
            for (connection_id, acknowledgement) in acknowledgements {
                if let Ok(applied_at_ms) = acknowledgement.await {
                    effects.push(FaultEffect::WebsocketDisconnected {
                        sequence: effects.len() as u32 + 1,
                        applied_at_ms,
                        connection_id,
                        target,
                    });
                }
            }
        };
        let _ = tokio::time::timeout(acknowledgement_timeout, collect_acknowledgements).await;
        let passed = effects.len() == requested;
        let completion = FaultCompletion {
            evidence_file: evidence_file.clone(),
            evidence: FaultInjectorEvidence::new(
                FaultEvidenceContext {
                    proxy_session_id: self.session_id.clone(),
                    proxy_config_fingerprint: self.config_fingerprint.clone(),
                    armed_at_ms,
                    completed_at_ms: now_ms(),
                },
                command_id,
                FaultCommandSummary::DisconnectWebsockets {
                    target,
                    connections: requested,
                },
                effects,
                passed,
            ),
        };
        match self.write_completion(completion).await {
            Ok(path) if passed => accepted("websocket disconnect fault completed", Some(path)),
            Ok(path) => FaultProxyControlResponse {
                format_version: CONTROL_FORMAT_VERSION,
                accepted: false,
                message:
                    "websocket disconnect fault was not acknowledged by every selected connection"
                        .to_string(),
                evidence_path: Some(path),
                status: None,
            },
            Err(message) => rejected(message),
        }
    }

    pub async fn consume_rest(
        &self,
        method: &str,
        path: &str,
        query: Option<&str>,
    ) -> Option<RestInjection> {
        let mut registry = self.registry.lock().await;
        let index = registry
            .rest
            .iter()
            .position(|fault| rest_matches(&fault.matcher, method, path, query))?;
        let fault = &mut registry.rest[index];
        let sequence = fault.requested - fault.remaining + 1;
        fault.remaining -= 1;
        fault.effects.push(FaultEffect::RestResponseInjected {
            sequence,
            applied_at_ms: now_ms(),
            method: method.to_string(),
            path: path.to_string(),
            query_sha256: hex_sha256(query.unwrap_or_default().as_bytes()),
        });
        let response = fault.response.clone();
        let completion = if fault.remaining == 0 {
            let fault = registry.rest.remove(index);
            Some(FaultCompletion {
                evidence_file: fault.evidence_file,
                evidence: FaultInjectorEvidence::new(
                    FaultEvidenceContext {
                        proxy_session_id: self.session_id.clone(),
                        proxy_config_fingerprint: self.config_fingerprint.clone(),
                        armed_at_ms: fault.armed_at_ms,
                        completed_at_ms: now_ms(),
                    },
                    fault.command_id,
                    FaultCommandSummary::RestResponse {
                        matcher: fault.matcher,
                        status: fault.response.status,
                        response_headers: fault.response.headers,
                        response_body_bytes: fault.response.body.len() as u64,
                        response_body_sha256: hex_sha256(fault.response.body.as_bytes()),
                        times: fault.requested,
                    },
                    fault.effects,
                    true,
                ),
            })
        } else {
            None
        };
        Some(RestInjection {
            response,
            completion,
        })
    }

    pub async fn consume_websocket(
        &self,
        connection_id: u64,
        target: WebSocketTarget,
        direction: WebSocketDirection,
        message: &Message,
    ) -> Option<WebSocketDrop> {
        let mut registry = self.registry.lock().await;
        let index = registry.websocket.iter().position(|fault| {
            fault.target == target
                && fault.direction == direction
                && websocket_matches(&fault.matcher, message)
        })?;
        let fault = &mut registry.websocket[index];
        let sequence = fault.requested - fault.remaining + 1;
        fault.remaining -= 1;
        let bytes = message_bytes(message);
        fault.effects.push(FaultEffect::WebsocketFrameDropped {
            sequence,
            applied_at_ms: now_ms(),
            connection_id,
            target,
            direction,
            frame_kind: message_kind(message),
            frame_bytes: bytes.len() as u64,
            frame_sha256: hex_sha256(bytes),
        });
        let completion = if fault.remaining == 0 {
            let fault = registry.websocket.remove(index);
            Some(FaultCompletion {
                evidence_file: fault.evidence_file,
                evidence: FaultInjectorEvidence::new(
                    FaultEvidenceContext {
                        proxy_session_id: self.session_id.clone(),
                        proxy_config_fingerprint: self.config_fingerprint.clone(),
                        armed_at_ms: fault.armed_at_ms,
                        completed_at_ms: now_ms(),
                    },
                    fault.command_id,
                    FaultCommandSummary::WebsocketDrop {
                        target: fault.target,
                        direction: fault.direction,
                        matcher: fault.matcher,
                        frames: fault.requested,
                    },
                    fault.effects,
                    true,
                ),
            })
        } else {
            None
        };
        Some(WebSocketDrop { completion })
    }

    pub async fn write_completion(&self, completion: FaultCompletion) -> Result<PathBuf, String> {
        let path = self.evidence_directory.join(completion.evidence_file);
        let bytes = serde_json::to_vec_pretty(&completion.evidence)
            .map_err(|error| format!("failed to serialize injector evidence: {error}"))?;
        let path_for_write = path.clone();
        tokio::task::spawn_blocking(move || write_create_new(&path_for_write, &bytes))
            .await
            .map_err(|error| format!("injector evidence writer task failed: {error}"))?
            .map_err(|error| {
                format!(
                    "failed to write injector evidence {}: {error}",
                    path.display()
                )
            })?;
        self.counters
            .completed_faults
            .fetch_add(1, Ordering::Relaxed);
        Ok(path)
    }

    pub async fn status(&self) -> FaultProxyStatus {
        let registry = self.registry.lock().await;
        let connections = self.connections.lock().await;
        let mut active = BTreeMap::from([
            ("public".to_string(), 0),
            ("private".to_string(), 0),
            ("order".to_string(), 0),
        ]);
        for connection in connections.values() {
            let key = match connection.target {
                WebSocketTarget::Public => "public",
                WebSocketTarget::Private => "private",
                WebSocketTarget::Order => "order",
            };
            *active.get_mut(key).expect("all target keys are present") += 1;
        }
        FaultProxyStatus {
            proxy_session_id: self.session_id.clone(),
            rest_requests_forwarded: self
                .counters
                .rest_requests_forwarded
                .load(Ordering::Relaxed),
            rest_responses_injected: self
                .counters
                .rest_responses_injected
                .load(Ordering::Relaxed),
            websocket_connections_accepted: self
                .counters
                .websocket_connections_accepted
                .load(Ordering::Relaxed),
            websocket_connections_active: active,
            websocket_frames_forwarded: self
                .counters
                .websocket_frames_forwarded
                .load(Ordering::Relaxed),
            websocket_frames_dropped: self
                .counters
                .websocket_frames_dropped
                .load(Ordering::Relaxed),
            websocket_disconnects_injected: self
                .counters
                .websocket_disconnects_injected
                .load(Ordering::Relaxed),
            pending_rest_faults: registry.rest.len(),
            pending_websocket_faults: registry.websocket.len(),
            completed_faults: self.counters.completed_faults.load(Ordering::Relaxed),
            proxy_errors: self.counters.proxy_errors.load(Ordering::Relaxed),
            recent_errors: self
                .errors
                .lock()
                .unwrap_or_else(|poisoned| poisoned.into_inner())
                .iter()
                .cloned()
                .collect(),
        }
    }

    pub fn record_error(&self, message: impl Into<String>) {
        self.counters.proxy_errors.fetch_add(1, Ordering::Relaxed);
        let mut message = message.into().replace(['\r', '\n'], " ");
        if message.len() > 512 {
            let mut boundary = 512;
            while !message.is_char_boundary(boundary) {
                boundary -= 1;
            }
            message.truncate(boundary);
        }
        let mut errors = self
            .errors
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner());
        if errors.len() == 32 {
            errors.pop_front();
        }
        errors.push_back(message);
    }

    fn reserve(
        &self,
        registry: &mut FaultRegistry,
        command_id: &str,
        evidence_file: &str,
    ) -> Result<(), String> {
        validate_identity(command_id, evidence_file)?;
        if registry.rest.len() + registry.websocket.len() >= self.max_pending_faults {
            return Err("maximum pending fault count reached".to_string());
        }
        if registry.command_ids.contains(command_id) {
            return Err(format!("command_id {command_id:?} was already used"));
        }
        if registry.evidence_files.contains(evidence_file) {
            return Err(format!(
                "evidence_file {evidence_file:?} was already reserved"
            ));
        }
        let path = self.evidence_directory.join(evidence_file);
        if path.exists() {
            return Err(format!("evidence path {} already exists", path.display()));
        }
        registry.command_ids.insert(command_id.to_string());
        registry.evidence_files.insert(evidence_file.to_string());
        Ok(())
    }
}

fn validate_identity(command_id: &str, evidence_file: &str) -> Result<(), String> {
    if !valid_identifier(command_id) {
        return Err(
            "command_id must contain 1-128 ASCII letters, digits, '.', '_', or '-'".to_string(),
        );
    }
    if !valid_identifier(evidence_file)
        || !evidence_file.ends_with(".json")
        || evidence_file.starts_with('.')
    {
        return Err(
            "evidence_file must be a plain non-hidden .json filename using ASCII letters, digits, '.', '_', or '-'"
                .to_string(),
        );
    }
    Ok(())
}

fn valid_identifier(value: &str) -> bool {
    !value.is_empty()
        && value.len() <= 128
        && value
            .bytes()
            .all(|byte| byte.is_ascii_alphanumeric() || matches!(byte, b'.' | b'_' | b'-'))
}

fn validate_rest_fault(
    matcher: &RestRequestMatcher,
    response: &InjectedHttpResponse,
    times: u32,
    max_http_body_bytes: usize,
) -> Result<(), String> {
    if matcher.method.is_empty()
        || !matcher
            .method
            .bytes()
            .all(|byte| byte.is_ascii_uppercase() || byte == b'-')
    {
        return Err("REST matcher method must be an uppercase HTTP token".to_string());
    }
    if !matcher.path.starts_with('/') || matcher.path.contains('?') || matcher.path.contains('#') {
        return Err(
            "REST matcher path must be an exact path without query or fragment".to_string(),
        );
    }
    if !(200..=599).contains(&response.status) {
        return Err("injected HTTP status must be in 200..=599".to_string());
    }
    if times == 0 || times > 100 {
        return Err("times must be in 1..=100".to_string());
    }
    if response.body.len() > max_http_body_bytes {
        return Err(format!(
            "injected HTTP body exceeds configured {max_http_body_bytes}-byte limit"
        ));
    }
    for (name, value) in &response.headers {
        let name = HeaderName::from_bytes(name.as_bytes())
            .map_err(|_| "injected HTTP header name is invalid".to_string())?;
        HeaderValue::from_str(value)
            .map_err(|_| "injected HTTP header value is invalid".to_string())?;
        if forbidden_proxy_header(&name) {
            return Err(format!(
                "injected HTTP response must not set hop-by-hop header {name}"
            ));
        }
    }
    Ok(())
}

fn forbidden_proxy_header(name: &HeaderName) -> bool {
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

fn validate_websocket_fault(matcher: &WebSocketFrameMatcher, frames: u32) -> Result<(), String> {
    if frames == 0 || frames > 100 {
        return Err("frames must be in 1..=100".to_string());
    }
    if matcher
        .json
        .as_ref()
        .is_some_and(WebSocketJsonMatcher::is_empty)
    {
        return Err("websocket JSON matcher must contain at least one field".to_string());
    }
    if matcher.json.is_some()
        && !matches!(
            matcher.kind,
            WebSocketFrameKind::Text | WebSocketFrameKind::Binary
        )
    {
        return Err("websocket JSON matching is valid only for text or binary frames".to_string());
    }
    Ok(())
}

fn rest_matches(
    matcher: &RestRequestMatcher,
    method: &str,
    path: &str,
    query: Option<&str>,
) -> bool {
    if matcher.method != method || matcher.path != path {
        return false;
    }
    if matcher.query.is_empty() {
        return true;
    }
    let actual = url::form_urlencoded::parse(query.unwrap_or_default().as_bytes())
        .into_owned()
        .collect::<HashMap<_, _>>();
    matcher
        .query
        .iter()
        .all(|(key, value)| actual.get(key) == Some(value))
}

fn websocket_matches(matcher: &WebSocketFrameMatcher, message: &Message) -> bool {
    if matcher.kind != message_kind(message) {
        return false;
    }
    let Some(json_matcher) = &matcher.json else {
        return true;
    };
    let Ok(value) = serde_json::from_slice::<Value>(message_bytes(message)) else {
        return false;
    };
    optional_field_matches(&value, "op", json_matcher.op.as_deref())
        && optional_field_matches(&value, "event", json_matcher.event.as_deref())
        && optional_nested_field_matches(
            &value,
            &["arg", "channel"],
            json_matcher.channel.as_deref(),
        )
        && optional_recursive_field_matches(
            &value,
            "instType",
            json_matcher.instrument_type.as_deref(),
        )
        && optional_recursive_field_matches(&value, "instId", json_matcher.symbol.as_deref())
}

fn optional_field_matches(value: &Value, key: &str, expected: Option<&str>) -> bool {
    expected.is_none_or(|expected| value.get(key).and_then(Value::as_str) == Some(expected))
}

fn optional_nested_field_matches(value: &Value, path: &[&str], expected: Option<&str>) -> bool {
    expected.is_none_or(|expected| {
        path.iter()
            .try_fold(value, |current, key| current.get(key))
            .and_then(Value::as_str)
            == Some(expected)
    })
}

fn optional_recursive_field_matches(value: &Value, key: &str, expected: Option<&str>) -> bool {
    expected.is_none_or(|expected| recursive_field_matches(value, key, expected))
}

fn recursive_field_matches(value: &Value, key: &str, expected: &str) -> bool {
    match value {
        Value::Object(object) => {
            object.get(key).and_then(Value::as_str) == Some(expected)
                || object
                    .values()
                    .any(|value| recursive_field_matches(value, key, expected))
        }
        Value::Array(array) => array
            .iter()
            .any(|value| recursive_field_matches(value, key, expected)),
        _ => false,
    }
}

pub(crate) fn message_kind(message: &Message) -> WebSocketFrameKind {
    match message {
        Message::Text(_) => WebSocketFrameKind::Text,
        Message::Binary(_) => WebSocketFrameKind::Binary,
        Message::Ping(_) => WebSocketFrameKind::Ping,
        Message::Pong(_) => WebSocketFrameKind::Pong,
        Message::Close(_) | Message::Frame(_) => WebSocketFrameKind::Close,
    }
}

pub(crate) fn message_bytes(message: &Message) -> &[u8] {
    match message {
        Message::Text(value) => value.as_bytes(),
        Message::Binary(value) | Message::Ping(value) | Message::Pong(value) => value.as_ref(),
        Message::Close(_) | Message::Frame(_) => &[],
    }
}

fn accepted(
    message: impl Into<String>,
    evidence_path: Option<PathBuf>,
) -> FaultProxyControlResponse {
    FaultProxyControlResponse {
        format_version: CONTROL_FORMAT_VERSION,
        accepted: true,
        message: message.into(),
        evidence_path,
        status: None,
    }
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

fn write_create_new(path: &Path, bytes: &[u8]) -> Result<(), std::io::Error> {
    #[cfg(unix)]
    use std::os::unix::fs::OpenOptionsExt;

    let mut options = OpenOptions::new();
    options.write(true).create_new(true);
    #[cfg(unix)]
    options.mode(0o600);
    let mut file = options.open(path)?;
    file.write_all(bytes)?;
    file.write_all(b"\n")?;
    file.sync_all()?;
    sync_parent(path)
}

#[cfg(unix)]
fn sync_parent(path: &Path) -> Result<(), std::io::Error> {
    fs::File::open(path.parent().unwrap_or_else(|| Path::new(".")))?.sync_all()
}

#[cfg(not(unix))]
fn sync_parent(_path: &Path) -> Result<(), std::io::Error> {
    Ok(())
}

pub(crate) fn now_ms() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap_or_default()
        .as_millis()
        .try_into()
        .unwrap_or(u64::MAX)
}

fn new_session_id() -> String {
    let mut hasher = Sha256::new();
    hasher.update(now_ms().to_le_bytes());
    hasher.update(std::process::id().to_le_bytes());
    hasher.update(
        SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap_or_default()
            .as_nanos()
            .to_le_bytes(),
    );
    let digest = format!("{:x}", hasher.finalize());
    format!("fault-{}-{}", std::process::id(), &digest[..16])
}

fn hex_sha256(bytes: &[u8]) -> String {
    format!("{:x}", Sha256::digest(bytes))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn rest_matching_uses_exact_path_and_required_query_pairs() {
        let matcher = RestRequestMatcher {
            method: "GET".to_string(),
            path: "/api/v5/account/instruments".to_string(),
            query: BTreeMap::from([
                ("instId".to_string(), "BTC-USDT".to_string()),
                ("instType".to_string(), "SPOT".to_string()),
            ]),
        };
        assert!(rest_matches(
            &matcher,
            "GET",
            "/api/v5/account/instruments",
            Some("instType=SPOT&instId=BTC-USDT&extra=1")
        ));
        assert!(!rest_matches(
            &matcher,
            "GET",
            "/api/v5/account/instruments",
            Some("instType=SWAP&instId=BTC-USDT")
        ));
    }

    #[test]
    fn websocket_json_matching_covers_okx_channel_and_instrument_fields() {
        let matcher = WebSocketFrameMatcher {
            kind: WebSocketFrameKind::Text,
            json: Some(WebSocketJsonMatcher {
                channel: Some("positions".to_string()),
                instrument_type: Some("SWAP".to_string()),
                symbol: Some("BTC-USDT-SWAP".to_string()),
                ..WebSocketJsonMatcher::default()
            }),
        };
        let message = Message::Text(
            r#"{"arg":{"channel":"positions","instType":"ANY"},"data":[{"instType":"SWAP","instId":"BTC-USDT-SWAP"}]}"#
                .into(),
        );
        assert!(websocket_matches(&matcher, &message));
    }

    #[test]
    fn evidence_filename_rejects_paths_and_hidden_files() {
        assert!(validate_identity("public-reconnect", "public-reconnect.json").is_ok());
        assert!(validate_identity("public-reconnect", "../record.json").is_err());
        assert!(validate_identity("public-reconnect", ".record.json").is_err());
    }

    #[test]
    fn rest_fault_validation_rejects_oversized_bodies_and_transport_headers() {
        let matcher = RestRequestMatcher {
            method: "GET".to_string(),
            path: "/api/v5/public/time".to_string(),
            query: BTreeMap::new(),
        };
        let oversized = InjectedHttpResponse {
            status: 503,
            headers: BTreeMap::new(),
            body: "too large".to_string(),
        };
        assert!(validate_rest_fault(&matcher, &oversized, 1, 8).is_err());

        let unsafe_header = InjectedHttpResponse {
            status: 503,
            headers: BTreeMap::from([("content-length".to_string(), "1".to_string())]),
            body: String::new(),
        };
        assert!(validate_rest_fault(&matcher, &unsafe_header, 1, 1024).is_err());
    }
}
