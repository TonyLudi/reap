use std::collections::BTreeMap;
use std::sync::Arc;
use std::sync::atomic::{AtomicU64, AtomicUsize, Ordering};
use std::time::{Duration, SystemTime, UNIX_EPOCH};

use reqwest::{Client, StatusCode, Url};
use serde::{Deserialize, Serialize};
use thiserror::Error;
use tokio::sync::mpsc;
use tokio::task::JoinHandle;

static ALERT_SEQUENCE: AtomicU64 = AtomicU64::new(1);
const MAX_COMPONENT_BYTES: usize = 64;
const MAX_CODE_BYTES: usize = 64;
const MAX_EVENT_ID_BYTES: usize = 128;
const MAX_MESSAGE_BYTES: usize = 4_096;
const MAX_ATTRIBUTE_KEY_BYTES: usize = 64;
const MAX_ATTRIBUTE_VALUE_BYTES: usize = 512;
const MAX_ATTRIBUTES: usize = 16;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum AlertSeverity {
    Warning,
    Critical,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct AlertEvent {
    pub schema_version: u16,
    pub event_id: String,
    pub ts_ms: u64,
    pub severity: AlertSeverity,
    pub component: String,
    pub code: String,
    pub message: String,
    #[serde(default, skip_serializing_if = "BTreeMap::is_empty")]
    pub attributes: BTreeMap<String, String>,
}

impl AlertEvent {
    pub fn new(
        severity: AlertSeverity,
        component: impl Into<String>,
        code: impl Into<String>,
        message: impl Into<String>,
    ) -> Self {
        let ts_ms = unix_time_ms();
        let sequence = ALERT_SEQUENCE.fetch_add(1, Ordering::Relaxed);
        Self {
            schema_version: 1,
            event_id: format!("{ts_ms:x}-{:x}-{sequence:x}", std::process::id()),
            ts_ms,
            severity,
            component: truncate_utf8(component.into(), MAX_COMPONENT_BYTES),
            code: truncate_utf8(code.into(), MAX_CODE_BYTES),
            message: truncate_utf8(message.into(), MAX_MESSAGE_BYTES),
            attributes: BTreeMap::new(),
        }
    }

    pub fn with_attribute(mut self, key: impl Into<String>, value: impl Into<String>) -> Self {
        self.attributes.insert(
            truncate_utf8(key.into(), MAX_ATTRIBUTE_KEY_BYTES),
            truncate_utf8(value.into(), MAX_ATTRIBUTE_VALUE_BYTES),
        );
        self
    }

    fn enforce_bounds(&mut self) {
        self.event_id = truncate_utf8(std::mem::take(&mut self.event_id), MAX_EVENT_ID_BYTES);
        self.component = truncate_utf8(std::mem::take(&mut self.component), MAX_COMPONENT_BYTES);
        self.code = truncate_utf8(std::mem::take(&mut self.code), MAX_CODE_BYTES);
        self.message = truncate_utf8(std::mem::take(&mut self.message), MAX_MESSAGE_BYTES);
        self.attributes = std::mem::take(&mut self.attributes)
            .into_iter()
            .take(MAX_ATTRIBUTES)
            .map(|(key, value)| {
                (
                    truncate_utf8(key, MAX_ATTRIBUTE_KEY_BYTES),
                    truncate_utf8(value, MAX_ATTRIBUTE_VALUE_BYTES),
                )
            })
            .collect();
    }
}

fn truncate_utf8(mut value: String, max_bytes: usize) -> String {
    if value.len() <= max_bytes {
        return value;
    }
    let mut boundary = max_bytes;
    while !value.is_char_boundary(boundary) {
        boundary -= 1;
    }
    value.truncate(boundary);
    value
}

#[derive(Debug, Clone)]
pub struct WebhookAlertConfig {
    pub endpoint: String,
    pub bearer_token: Option<String>,
    pub channel_capacity: usize,
    pub failure_channel_capacity: usize,
    pub request_timeout: Duration,
    pub connect_timeout: Duration,
    pub max_attempts: usize,
    pub retry_backoff: Duration,
}

impl WebhookAlertConfig {
    pub fn new(endpoint: impl Into<String>) -> Self {
        Self {
            endpoint: endpoint.into(),
            bearer_token: None,
            channel_capacity: 256,
            failure_channel_capacity: 64,
            request_timeout: Duration::from_secs(2),
            connect_timeout: Duration::from_secs(1),
            max_attempts: 3,
            retry_backoff: Duration::from_millis(250),
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct AlertDeliveryFailure {
    pub event_id: String,
    pub code: String,
    pub attempts: usize,
    pub reason: String,
}

#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct AlertStats {
    pub delivered: u64,
    pub failed: u64,
    pub failure_notifications_dropped: u64,
    pub max_queue_depth: usize,
}

#[derive(Debug, Error)]
pub enum AlertError {
    #[error("alert endpoint is invalid: {0}")]
    InvalidEndpoint(String),
    #[error("alert HTTP client setup failed: {0}")]
    Client(String),
    #[error("alert queue is full")]
    QueueFull,
    #[error("alert worker is closed")]
    Closed,
    #[error("alert worker task failed: {0}")]
    Join(#[from] tokio::task::JoinError),
}

enum AlertCommand {
    Deliver(AlertEvent),
    Shutdown,
}

#[derive(Clone)]
pub struct AlertSink {
    sender: mpsc::Sender<AlertCommand>,
    queued: Arc<AtomicUsize>,
    max_queue_depth: Arc<AtomicUsize>,
}

impl AlertSink {
    pub fn try_emit(&self, mut event: AlertEvent) -> Result<(), AlertError> {
        event.enforce_bounds();
        let depth = self.queued.fetch_add(1, Ordering::Relaxed) + 1;
        self.max_queue_depth.fetch_max(depth, Ordering::Relaxed);
        match self.sender.try_send(AlertCommand::Deliver(event)) {
            Ok(()) => Ok(()),
            Err(mpsc::error::TrySendError::Full(_)) => {
                self.queued.fetch_sub(1, Ordering::Relaxed);
                Err(AlertError::QueueFull)
            }
            Err(mpsc::error::TrySendError::Closed(_)) => {
                self.queued.fetch_sub(1, Ordering::Relaxed);
                Err(AlertError::Closed)
            }
        }
    }

    pub fn queue_depth(&self) -> usize {
        self.queued.load(Ordering::Relaxed)
    }
}

pub struct AlertRuntime {
    sink: AlertSink,
    failures: Option<mpsc::Receiver<AlertDeliveryFailure>>,
    task: JoinHandle<Result<(), AlertError>>,
    delivered: Arc<AtomicU64>,
    failed: Arc<AtomicU64>,
    failure_notifications_dropped: Arc<AtomicU64>,
}

impl AlertRuntime {
    pub fn sink(&self) -> AlertSink {
        self.sink.clone()
    }

    pub fn take_failures(&mut self) -> mpsc::Receiver<AlertDeliveryFailure> {
        self.failures
            .take()
            .expect("alert failure receiver can only be taken once")
    }

    pub async fn shutdown(self) -> Result<AlertStats, AlertError> {
        self.sink
            .sender
            .send(AlertCommand::Shutdown)
            .await
            .map_err(|_| AlertError::Closed)?;
        self.task.await??;
        Ok(AlertStats {
            delivered: self.delivered.load(Ordering::Relaxed),
            failed: self.failed.load(Ordering::Relaxed),
            failure_notifications_dropped: self
                .failure_notifications_dropped
                .load(Ordering::Relaxed),
            max_queue_depth: self.sink.max_queue_depth.load(Ordering::Relaxed),
        })
    }
}

pub fn start_webhook_alerts(config: WebhookAlertConfig) -> Result<AlertRuntime, AlertError> {
    let endpoint = validate_endpoint(&config.endpoint)?;
    if config.channel_capacity == 0
        || config.failure_channel_capacity == 0
        || config.max_attempts == 0
        || config.request_timeout.is_zero()
        || config.connect_timeout.is_zero()
    {
        return Err(AlertError::Client(
            "alert capacities, attempts, and timeouts must be positive".to_string(),
        ));
    }
    let client = Client::builder()
        .connect_timeout(config.connect_timeout)
        .timeout(config.request_timeout)
        .redirect(reqwest::redirect::Policy::none())
        .build()
        .map_err(|error| AlertError::Client(error.to_string()))?;
    let (sender, receiver) = mpsc::channel(config.channel_capacity);
    let (failure_tx, failures) = mpsc::channel(config.failure_channel_capacity);
    let queued = Arc::new(AtomicUsize::new(0));
    let max_queue_depth = Arc::new(AtomicUsize::new(0));
    let delivered = Arc::new(AtomicU64::new(0));
    let failed = Arc::new(AtomicU64::new(0));
    let failure_notifications_dropped = Arc::new(AtomicU64::new(0));
    let task = tokio::spawn(run_alert_worker(
        client,
        endpoint,
        config,
        receiver,
        failure_tx,
        Arc::clone(&queued),
        Arc::clone(&delivered),
        Arc::clone(&failed),
        Arc::clone(&failure_notifications_dropped),
    ));
    Ok(AlertRuntime {
        sink: AlertSink {
            sender,
            queued,
            max_queue_depth,
        },
        failures: Some(failures),
        task,
        delivered,
        failed,
        failure_notifications_dropped,
    })
}

fn validate_endpoint(endpoint: &str) -> Result<Url, AlertError> {
    let url =
        Url::parse(endpoint).map_err(|error| AlertError::InvalidEndpoint(error.to_string()))?;
    let secure = url.scheme() == "https";
    let loopback_http = url.scheme() == "http"
        && url.host_str().is_some_and(|host| {
            host.eq_ignore_ascii_case("localhost") || host == "127.0.0.1" || host == "::1"
        });
    if !secure && !loopback_http {
        return Err(AlertError::InvalidEndpoint(
            "endpoint must use HTTPS or loopback HTTP".to_string(),
        ));
    }
    Ok(url)
}

#[allow(clippy::too_many_arguments)]
async fn run_alert_worker(
    client: Client,
    endpoint: Url,
    config: WebhookAlertConfig,
    mut commands: mpsc::Receiver<AlertCommand>,
    failures_tx: mpsc::Sender<AlertDeliveryFailure>,
    queued: Arc<AtomicUsize>,
    delivered: Arc<AtomicU64>,
    failed: Arc<AtomicU64>,
    failure_notifications_dropped: Arc<AtomicU64>,
) -> Result<(), AlertError> {
    while let Some(command) = commands.recv().await {
        match command {
            AlertCommand::Deliver(event) => {
                queued.fetch_sub(1, Ordering::Relaxed);
                match deliver_with_retry(&client, &endpoint, &config, &event).await {
                    Ok(()) => {
                        delivered.fetch_add(1, Ordering::Relaxed);
                    }
                    Err((attempts, reason)) => {
                        failed.fetch_add(1, Ordering::Relaxed);
                        if failures_tx
                            .try_send(AlertDeliveryFailure {
                                event_id: event.event_id,
                                code: event.code,
                                attempts,
                                reason,
                            })
                            .is_err()
                        {
                            failure_notifications_dropped.fetch_add(1, Ordering::Relaxed);
                        }
                    }
                }
            }
            AlertCommand::Shutdown => return Ok(()),
        }
    }
    Ok(())
}

async fn deliver_with_retry(
    client: &Client,
    endpoint: &Url,
    config: &WebhookAlertConfig,
    event: &AlertEvent,
) -> Result<(), (usize, String)> {
    let mut backoff = config.retry_backoff;
    for attempt in 1..=config.max_attempts {
        let mut request = client.post(endpoint.clone()).json(event);
        if let Some(token) = &config.bearer_token {
            request = request.bearer_auth(token);
        }
        match request.send().await {
            Ok(response) if response.status().is_success() => return Ok(()),
            Ok(response) => {
                let status = response.status();
                if attempt == config.max_attempts || !retryable_status(status) {
                    return Err((attempt, format!("webhook returned HTTP {status}")));
                }
            }
            Err(error) => {
                if attempt == config.max_attempts {
                    return Err((attempt, error.without_url().to_string()));
                }
            }
        }
        if !backoff.is_zero() {
            tokio::time::sleep(backoff).await;
            backoff = backoff.saturating_mul(2);
        }
    }
    unreachable!("positive max_attempts is validated")
}

fn retryable_status(status: StatusCode) -> bool {
    status.is_server_error()
        || status == StatusCode::REQUEST_TIMEOUT
        || status == StatusCode::TOO_MANY_REQUESTS
}

fn unix_time_ms() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap_or_default()
        .as_millis()
        .min(u64::MAX as u128) as u64
}

#[cfg(test)]
mod tests {
    use tokio::io::{AsyncReadExt, AsyncWriteExt};
    use tokio::net::TcpListener;

    use super::*;

    async fn receive_request(listener: &TcpListener, status: &str) -> String {
        let (mut socket, _) = listener.accept().await.unwrap();
        let mut request = Vec::new();
        let mut buffer = [0_u8; 4_096];
        loop {
            let read = socket.read(&mut buffer).await.unwrap();
            if read == 0 {
                break;
            }
            request.extend_from_slice(&buffer[..read]);
            if complete_http_request(&request) {
                break;
            }
        }
        socket
            .write_all(
                format!("HTTP/1.1 {status}\r\nContent-Length: 0\r\nConnection: close\r\n\r\n")
                    .as_bytes(),
            )
            .await
            .unwrap();
        String::from_utf8(request).unwrap()
    }

    fn complete_http_request(request: &[u8]) -> bool {
        let text = String::from_utf8_lossy(request);
        let Some(header_end) = text.find("\r\n\r\n") else {
            return false;
        };
        let content_length = text[..header_end]
            .lines()
            .find_map(|line| {
                let (name, value) = line.split_once(':')?;
                name.eq_ignore_ascii_case("content-length")
                    .then(|| value.trim().parse::<usize>().ok())
                    .flatten()
            })
            .unwrap_or(0);
        request.len() >= header_end + 4 + content_length
    }

    #[tokio::test]
    async fn webhook_delivers_bounded_json_with_bearer_auth() {
        let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
        let address = listener.local_addr().unwrap();
        let server =
            tokio::spawn(async move { receive_request(&listener, "204 No Content").await });
        let mut config = WebhookAlertConfig::new(format!("http://{address}/alerts"));
        config.bearer_token = Some("test-token".to_string());
        let runtime = start_webhook_alerts(config).unwrap();
        runtime
            .sink()
            .try_emit(AlertEvent::new(
                AlertSeverity::Critical,
                "risk",
                "risk_breach",
                "position limit exceeded",
            ))
            .unwrap();

        let stats = runtime.shutdown().await.unwrap();
        let request = server.await.unwrap();

        assert_eq!(stats.delivered, 1);
        assert_eq!(stats.failed, 0);
        assert!(
            request
                .to_ascii_lowercase()
                .contains("authorization: bearer test-token")
        );
        assert!(request.contains("\"code\":\"risk_breach\""));
    }

    #[tokio::test]
    async fn terminal_delivery_failure_is_reported_after_bounded_retries() {
        let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
        let address = listener.local_addr().unwrap();
        let server = tokio::spawn(async move {
            receive_request(&listener, "503 Service Unavailable").await;
            receive_request(&listener, "503 Service Unavailable").await;
        });
        let mut config = WebhookAlertConfig::new(format!("http://{address}/alerts"));
        config.max_attempts = 2;
        config.retry_backoff = Duration::from_millis(1);
        let mut runtime = start_webhook_alerts(config).unwrap();
        let mut failures = runtime.take_failures();
        runtime
            .sink()
            .try_emit(AlertEvent::new(
                AlertSeverity::Warning,
                "feed",
                "feed_gap",
                "gap",
            ))
            .unwrap();

        let failure = tokio::time::timeout(Duration::from_secs(1), failures.recv())
            .await
            .unwrap()
            .unwrap();
        let stats = runtime.shutdown().await.unwrap();
        server.await.unwrap();

        assert_eq!(failure.attempts, 2);
        assert_eq!(failure.code, "feed_gap");
        assert_eq!(stats.failed, 1);
        assert_eq!(stats.delivered, 0);
    }

    #[tokio::test]
    async fn transport_failures_do_not_expose_secret_endpoint_paths() {
        let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
        let address = listener.local_addr().unwrap();
        drop(listener);
        let mut config = WebhookAlertConfig::new(format!("http://{address}/secret-webhook-token"));
        config.max_attempts = 1;
        config.request_timeout = Duration::from_millis(100);
        let mut runtime = start_webhook_alerts(config).unwrap();
        let mut failures = runtime.take_failures();
        runtime
            .sink()
            .try_emit(AlertEvent::new(
                AlertSeverity::Critical,
                "runtime",
                "test_failure",
                "test",
            ))
            .unwrap();

        let failure = tokio::time::timeout(Duration::from_secs(1), failures.recv())
            .await
            .unwrap()
            .unwrap();
        runtime.shutdown().await.unwrap();

        assert!(!failure.reason.contains("secret-webhook-token"));
        assert!(!failure.reason.contains(&address.to_string()));
    }

    #[test]
    fn endpoint_requires_https_or_loopback_http() {
        assert!(matches!(
            start_webhook_alerts(WebhookAlertConfig::new("http://example.com/hook")),
            Err(AlertError::InvalidEndpoint(_))
        ));
    }

    #[test]
    fn alert_fields_are_utf8_safe_and_size_bounded() {
        let alert = AlertEvent::new(
            AlertSeverity::Critical,
            "x".repeat(MAX_COMPONENT_BYTES + 1),
            "y".repeat(MAX_CODE_BYTES + 1),
            "z".repeat(MAX_MESSAGE_BYTES - 1) + "\u{20ac}",
        )
        .with_attribute(
            "k".repeat(MAX_ATTRIBUTE_KEY_BYTES + 1),
            "v".repeat(MAX_ATTRIBUTE_VALUE_BYTES + 1),
        );

        assert_eq!(alert.component.len(), MAX_COMPONENT_BYTES);
        assert_eq!(alert.code.len(), MAX_CODE_BYTES);
        assert!(alert.message.len() <= MAX_MESSAGE_BYTES);
        let (key, value) = alert.attributes.first_key_value().unwrap();
        assert_eq!(key.len(), MAX_ATTRIBUTE_KEY_BYTES);
        assert_eq!(value.len(), MAX_ATTRIBUTE_VALUE_BYTES);
    }
}
