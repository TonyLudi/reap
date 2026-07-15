use std::collections::{HashMap, HashSet};
use std::fs::{File, OpenOptions};
use std::io::{Read, Seek, SeekFrom, Write};
use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex as StdMutex};
use std::time::{Duration, Instant};

#[cfg(unix)]
use std::os::unix::fs::{MetadataExt, OpenOptionsExt};

use reap_core::{ConnId, RawEnvelope, Symbol, Venue};
use reap_venue::{VenueAdapter, okx::OkxSigner};
use thiserror::Error;
use tokio::sync::{Mutex, mpsc, watch};
use tokio::task::JoinHandle;

use crate::{
    ConnectionError, ConnectionStatus, ConnectionStatusKind, RecoveryRequest, SocketPlan,
    run_connection_once,
};

/// OKX documents at most three WebSocket connection requests per second per IP.
pub const OKX_MIN_CONNECTION_ATTEMPT_INTERVAL_MS: u64 = 334;
pub const DEFAULT_OKX_CONNECTION_ATTEMPT_PACER_PATH: &str = "var/reap/okx-connection-attempt.pacer";

type RecoveryStreamKey = (Venue, Symbol);
type RecoveryRoute = (ConnId, watch::Sender<u64>);
type RecoveryRoutes = HashMap<RecoveryStreamKey, Vec<RecoveryRoute>>;

const SHARED_PACER_STATE_MAGIC: &str = "reap-okx-connect-pacer-v1";
const MAX_SHARED_PACER_STATE_BYTES: u64 = 128;
const SHARED_PACER_TIMESTAMP_WIDTH: usize = 39;
const MAX_SHARED_RESERVATION_AHEAD: Duration = Duration::from_secs(15 * 60);
const MAX_SHARED_PACER_LOCK_WAIT: Duration = Duration::from_secs(1);
const SHARED_PACER_LOCK_RETRY: Duration = Duration::from_millis(5);

#[derive(Debug, Clone)]
pub struct ReconnectPolicy {
    pub initial_delay: Duration,
    pub max_delay: Duration,
    pub multiplier: u32,
}

/// Pacing for connection handshakes across independent feed groups and, when
/// configured, every Reap process using the same state file.
#[derive(Debug, Clone)]
pub struct ConnectionAttemptPacer {
    interval: Duration,
    state: Arc<Mutex<ConnectionAttemptPacerState>>,
    shared: Option<Arc<SharedConnectionAttemptPacer>>,
}

#[derive(Debug, Default)]
struct ConnectionAttemptPacerState {
    next_attempt: Option<Instant>,
}

#[derive(Debug)]
struct SharedConnectionAttemptPacer {
    path: PathBuf,
    file: File,
    local_lock: StdMutex<()>,
    clock_id: String,
}

#[derive(Debug)]
struct SharedPacerState {
    clock_id: String,
    next_attempt_ns: u128,
}

#[derive(Debug, Error)]
pub enum ConnectionAttemptPacerError {
    #[error("failed to {operation} process-shared connection pacer {path}: {source}")]
    Io {
        operation: &'static str,
        path: PathBuf,
        #[source]
        source: std::io::Error,
    },
    #[error("unsafe process-shared connection pacer {path}: {reason}")]
    UnsafeFile { path: PathBuf, reason: String },
    #[error("invalid process-shared connection pacer state in {path}: {reason}")]
    InvalidState { path: PathBuf, reason: String },
    #[error("process-shared connection pacer clock failed: {0}")]
    Clock(String),
    #[error("process-shared connection pacing requires Linux CLOCK_BOOTTIME")]
    UnsupportedPlatform,
    #[error(
        "process-shared connection pacer {path} reserved {ahead_ms}ms ahead; maximum is {maximum_ms}ms"
    )]
    ReservationTooFar {
        path: PathBuf,
        ahead_ms: u128,
        maximum_ms: u128,
    },
    #[error("process-shared connection pacer lock is poisoned")]
    LockPoisoned,
    #[error("process-shared connection pacer {path} lock remained busy for {maximum_wait_ms}ms")]
    LockTimeout {
        path: PathBuf,
        maximum_wait_ms: u128,
    },
    #[error("process-shared connection pacer task failed: {0}")]
    Task(String),
}

impl ConnectionAttemptPacer {
    pub fn new(interval: Duration) -> Self {
        Self {
            interval,
            state: Arc::new(Mutex::new(ConnectionAttemptPacerState::default())),
            shared: None,
        }
    }

    pub fn process_shared(
        interval: Duration,
        path: impl AsRef<Path>,
    ) -> Result<Self, ConnectionAttemptPacerError> {
        ensure_process_shared_pacing_supported()?;
        Ok(Self {
            interval,
            state: Arc::new(Mutex::new(ConnectionAttemptPacerState::default())),
            shared: Some(Arc::new(SharedConnectionAttemptPacer::open(path.as_ref())?)),
        })
    }

    pub async fn wait_for_turn(
        &self,
        shutdown: &mut watch::Receiver<bool>,
    ) -> Result<bool, ConnectionAttemptPacerError> {
        if self.interval.is_zero() {
            return Ok(!*shutdown.borrow());
        }
        if *shutdown.borrow() {
            return Ok(false);
        }
        if let Some(shared) = &self.shared {
            let shared = Arc::clone(shared);
            let interval = self.interval;
            let delay = tokio::task::spawn_blocking(move || shared.reserve(interval))
                .await
                .map_err(|error| ConnectionAttemptPacerError::Task(error.to_string()))??;
            if !delay.is_zero() {
                tokio::select! {
                    _ = tokio::time::sleep(delay) => {}
                    changed = shutdown.changed() => {
                        if changed.is_err() || *shutdown.borrow() {
                            return Ok(false);
                        }
                    }
                }
            }
            return Ok(!*shutdown.borrow());
        }
        'wait: loop {
            let mut state = tokio::select! {
                state = self.state.lock() => state,
                changed = shutdown.changed() => {
                    if changed.is_err() || *shutdown.borrow() {
                        return Ok(false);
                    }
                    continue 'wait;
                }
            };
            if *shutdown.borrow() {
                return Ok(false);
            }
            let delay = state.delay_at(Instant::now());
            if !delay.is_zero() {
                tokio::select! {
                    _ = tokio::time::sleep(delay) => {}
                    changed = shutdown.changed() => {
                        if changed.is_err() || *shutdown.borrow() {
                            return Ok(false);
                        }
                        continue 'wait;
                    }
                }
            }
            if *shutdown.borrow() {
                return Ok(false);
            }
            state.record_attempt_at(Instant::now(), self.interval);
            return Ok(true);
        }
    }
}

impl SharedConnectionAttemptPacer {
    fn open(path: &Path) -> Result<Self, ConnectionAttemptPacerError> {
        if path.as_os_str().is_empty() {
            return Err(ConnectionAttemptPacerError::UnsafeFile {
                path: path.to_path_buf(),
                reason: "path must not be empty".to_string(),
            });
        }
        let mut options = OpenOptions::new();
        options.read(true).write(true).create(true);
        #[cfg(unix)]
        options
            .mode(0o600)
            .custom_flags(libc::O_CLOEXEC | libc::O_NOFOLLOW);
        let file = options
            .open(path)
            .map_err(|source| ConnectionAttemptPacerError::Io {
                operation: "open",
                path: path.to_path_buf(),
                source,
            })?;
        let metadata = file
            .metadata()
            .map_err(|source| ConnectionAttemptPacerError::Io {
                operation: "inspect",
                path: path.to_path_buf(),
                source,
            })?;
        if !metadata.file_type().is_file() {
            return Err(ConnectionAttemptPacerError::UnsafeFile {
                path: path.to_path_buf(),
                reason: "path is not a regular file".to_string(),
            });
        }
        #[cfg(unix)]
        {
            let effective_uid = unsafe { libc::geteuid() };
            if metadata.uid() != effective_uid {
                return Err(ConnectionAttemptPacerError::UnsafeFile {
                    path: path.to_path_buf(),
                    reason: format!(
                        "file owner uid {} differs from effective uid {effective_uid}",
                        metadata.uid()
                    ),
                });
            }
            if metadata.nlink() != 1 {
                return Err(ConnectionAttemptPacerError::UnsafeFile {
                    path: path.to_path_buf(),
                    reason: format!("file has {} hard links; expected one", metadata.nlink()),
                });
            }
            let mode = metadata.mode() & 0o777;
            if mode & 0o077 != 0 {
                return Err(ConnectionAttemptPacerError::UnsafeFile {
                    path: path.to_path_buf(),
                    reason: format!("file mode {mode:03o} permits group or other access"),
                });
            }
        }
        let shared = Self {
            path: path.to_path_buf(),
            file,
            local_lock: StdMutex::new(()),
            clock_id: shared_clock_identity()?,
        };
        shared.with_locked(|file| {
            shared.read_next_attempt_ns(file)?;
            Ok(())
        })?;
        Ok(shared)
    }

    fn reserve(&self, interval: Duration) -> Result<Duration, ConnectionAttemptPacerError> {
        self.with_locked(|file| {
            let now_ns = shared_clock_ns()?;
            let next_ns = self
                .read_next_attempt_ns(file)?
                .filter(|state| state.clock_id == self.clock_id)
                .map(|state| state.next_attempt_ns)
                .unwrap_or(now_ns);
            let reserved_ns = next_ns.max(now_ns);
            let ahead_ns = reserved_ns.saturating_sub(now_ns);
            if ahead_ns > MAX_SHARED_RESERVATION_AHEAD.as_nanos() {
                return Err(ConnectionAttemptPacerError::ReservationTooFar {
                    path: self.path.clone(),
                    ahead_ms: ahead_ns / 1_000_000,
                    maximum_ms: MAX_SHARED_RESERVATION_AHEAD.as_millis(),
                });
            }
            let following_ns = reserved_ns
                .checked_add(interval.as_nanos())
                .ok_or_else(|| ConnectionAttemptPacerError::InvalidState {
                    path: self.path.clone(),
                    reason: "next reservation timestamp overflowed".to_string(),
                })?;
            self.write_next_attempt_ns(file, following_ns)?;
            let ahead_ns =
                u64::try_from(ahead_ns).map_err(|_| ConnectionAttemptPacerError::InvalidState {
                    path: self.path.clone(),
                    reason: "reservation delay does not fit Duration".to_string(),
                })?;
            Ok(Duration::from_nanos(ahead_ns))
        })
    }

    fn with_locked<T>(
        &self,
        operation: impl FnOnce(&File) -> Result<T, ConnectionAttemptPacerError>,
    ) -> Result<T, ConnectionAttemptPacerError> {
        let _local = self
            .local_lock
            .lock()
            .map_err(|_| ConnectionAttemptPacerError::LockPoisoned)?;
        let lock_started = Instant::now();
        loop {
            match self.file.try_lock() {
                Ok(()) => break,
                Err(std::fs::TryLockError::WouldBlock)
                    if lock_started.elapsed() < MAX_SHARED_PACER_LOCK_WAIT =>
                {
                    std::thread::sleep(SHARED_PACER_LOCK_RETRY);
                }
                Err(std::fs::TryLockError::WouldBlock) => {
                    return Err(ConnectionAttemptPacerError::LockTimeout {
                        path: self.path.clone(),
                        maximum_wait_ms: MAX_SHARED_PACER_LOCK_WAIT.as_millis(),
                    });
                }
                Err(std::fs::TryLockError::Error(source)) => {
                    return Err(ConnectionAttemptPacerError::Io {
                        operation: "lock",
                        path: self.path.clone(),
                        source,
                    });
                }
            }
        }
        let result = operation(&self.file);
        let unlock = self
            .file
            .unlock()
            .map_err(|source| ConnectionAttemptPacerError::Io {
                operation: "unlock",
                path: self.path.clone(),
                source,
            });
        match (result, unlock) {
            (Ok(value), Ok(())) => Ok(value),
            (Err(error), _) => Err(error),
            (Ok(_), Err(error)) => Err(error),
        }
    }

    fn read_next_attempt_ns(
        &self,
        file: &File,
    ) -> Result<Option<SharedPacerState>, ConnectionAttemptPacerError> {
        let mut handle = file;
        handle
            .seek(SeekFrom::Start(0))
            .map_err(|source| ConnectionAttemptPacerError::Io {
                operation: "seek",
                path: self.path.clone(),
                source,
            })?;
        let mut bytes = Vec::new();
        handle
            .take(MAX_SHARED_PACER_STATE_BYTES + 1)
            .read_to_end(&mut bytes)
            .map_err(|source| ConnectionAttemptPacerError::Io {
                operation: "read",
                path: self.path.clone(),
                source,
            })?;
        if bytes.len() as u64 > MAX_SHARED_PACER_STATE_BYTES {
            return Err(ConnectionAttemptPacerError::InvalidState {
                path: self.path.clone(),
                reason: format!("state exceeds {MAX_SHARED_PACER_STATE_BYTES} bytes"),
            });
        }
        if bytes.is_empty() {
            return Ok(None);
        }
        let text = std::str::from_utf8(&bytes).map_err(|error| {
            ConnectionAttemptPacerError::InvalidState {
                path: self.path.clone(),
                reason: format!("state is not UTF-8: {error}"),
            }
        })?;
        let mut fields = text.split_whitespace();
        if fields.next() != Some(SHARED_PACER_STATE_MAGIC) {
            return Err(ConnectionAttemptPacerError::InvalidState {
                path: self.path.clone(),
                reason: "state magic does not match".to_string(),
            });
        }
        let clock_id = fields
            .next()
            .ok_or_else(|| ConnectionAttemptPacerError::InvalidState {
                path: self.path.clone(),
                reason: "clock identity is missing".to_string(),
            })?;
        if !valid_clock_identity(clock_id) {
            return Err(ConnectionAttemptPacerError::InvalidState {
                path: self.path.clone(),
                reason: "clock identity is invalid".to_string(),
            });
        }
        let timestamp_text =
            fields
                .next()
                .ok_or_else(|| ConnectionAttemptPacerError::InvalidState {
                    path: self.path.clone(),
                    reason: "next-attempt timestamp is missing".to_string(),
                })?;
        if timestamp_text.len() != SHARED_PACER_TIMESTAMP_WIDTH
            || !timestamp_text.bytes().all(|byte| byte.is_ascii_digit())
        {
            return Err(ConnectionAttemptPacerError::InvalidState {
                path: self.path.clone(),
                reason: format!(
                    "next-attempt timestamp must contain exactly {SHARED_PACER_TIMESTAMP_WIDTH} decimal digits"
                ),
            });
        }
        let timestamp = timestamp_text.parse::<u128>().map_err(|error| {
            ConnectionAttemptPacerError::InvalidState {
                path: self.path.clone(),
                reason: format!("next-attempt timestamp is invalid: {error}"),
            }
        })?;
        if fields.next().is_some() {
            return Err(ConnectionAttemptPacerError::InvalidState {
                path: self.path.clone(),
                reason: "state has trailing fields".to_string(),
            });
        }
        Ok(Some(SharedPacerState {
            clock_id: clock_id.to_string(),
            next_attempt_ns: timestamp,
        }))
    }

    fn write_next_attempt_ns(
        &self,
        file: &File,
        next_ns: u128,
    ) -> Result<(), ConnectionAttemptPacerError> {
        let payload = format!(
            "{SHARED_PACER_STATE_MAGIC} {} {next_ns:0SHARED_PACER_TIMESTAMP_WIDTH$}\n",
            self.clock_id
        );
        let mut handle = file;
        handle
            .seek(SeekFrom::Start(0))
            .and_then(|_| handle.write_all(payload.as_bytes()))
            .and_then(|_| handle.flush())
            .and_then(|_| file.set_len(payload.len() as u64))
            .map_err(|source| ConnectionAttemptPacerError::Io {
                operation: "write",
                path: self.path.clone(),
                source,
            })
    }
}

fn valid_clock_identity(value: &str) -> bool {
    !value.is_empty()
        && value.len() <= 64
        && value
            .bytes()
            .all(|byte| byte.is_ascii_lowercase() || byte.is_ascii_digit() || byte == b'-')
}

#[cfg(target_os = "linux")]
fn ensure_process_shared_pacing_supported() -> Result<(), ConnectionAttemptPacerError> {
    Ok(())
}

#[cfg(not(target_os = "linux"))]
fn ensure_process_shared_pacing_supported() -> Result<(), ConnectionAttemptPacerError> {
    Err(ConnectionAttemptPacerError::UnsupportedPlatform)
}

#[cfg(target_os = "linux")]
fn shared_clock_identity() -> Result<String, ConnectionAttemptPacerError> {
    let value = std::fs::read_to_string("/proc/sys/kernel/random/boot_id")
        .map_err(|error| ConnectionAttemptPacerError::Clock(error.to_string()))?;
    let value = value.trim();
    let valid = value.len() == 36
        && value.bytes().enumerate().all(|(index, byte)| {
            if matches!(index, 8 | 13 | 18 | 23) {
                byte == b'-'
            } else {
                byte.is_ascii_digit() || (b'a'..=b'f').contains(&byte)
            }
        });
    if !valid {
        return Err(ConnectionAttemptPacerError::Clock(
            "Linux boot_id is not a canonical lowercase UUID".to_string(),
        ));
    }
    Ok(value.to_string())
}

#[cfg(not(target_os = "linux"))]
fn shared_clock_identity() -> Result<String, ConnectionAttemptPacerError> {
    Err(ConnectionAttemptPacerError::UnsupportedPlatform)
}

#[cfg(target_os = "linux")]
fn shared_clock_ns() -> Result<u128, ConnectionAttemptPacerError> {
    let mut timestamp = std::mem::MaybeUninit::<libc::timespec>::uninit();
    let result = unsafe { libc::clock_gettime(libc::CLOCK_BOOTTIME, timestamp.as_mut_ptr()) };
    if result != 0 {
        return Err(ConnectionAttemptPacerError::Clock(
            std::io::Error::last_os_error().to_string(),
        ));
    }
    let timestamp = unsafe { timestamp.assume_init() };
    if timestamp.tv_sec < 0 || !(0..1_000_000_000).contains(&timestamp.tv_nsec) {
        return Err(ConnectionAttemptPacerError::Clock(
            "CLOCK_BOOTTIME returned an invalid timespec".to_string(),
        ));
    }
    Ok((timestamp.tv_sec as u128) * 1_000_000_000 + timestamp.tv_nsec as u128)
}

#[cfg(not(target_os = "linux"))]
fn shared_clock_ns() -> Result<u128, ConnectionAttemptPacerError> {
    Err(ConnectionAttemptPacerError::UnsupportedPlatform)
}

impl ConnectionAttemptPacerState {
    fn delay_at(&self, now: Instant) -> Duration {
        self.next_attempt
            .map(|next| next.saturating_duration_since(now))
            .unwrap_or_default()
    }

    fn record_attempt_at(&mut self, now: Instant, interval: Duration) {
        self.next_attempt = Some(now.checked_add(interval).unwrap_or(now));
    }
}

pub type BootstrapFactory =
    Arc<dyn Fn(&SocketPlan) -> Result<Vec<String>, ConnectionError> + Send + Sync>;

pub fn no_bootstrap() -> BootstrapFactory {
    Arc::new(|_| Ok(Vec::new()))
}

pub fn okx_login_bootstrap(signer: OkxSigner) -> BootstrapFactory {
    Arc::new(move |plan| {
        if !plan.private {
            return Ok(Vec::new());
        }
        let timestamp = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap_or_default()
            .as_secs()
            .to_string();
        signer
            .websocket_login(&timestamp)
            .map(|message| vec![message])
            .map_err(|error| ConnectionError::LoginFailed(error.to_string()))
    })
}

impl Default for ReconnectPolicy {
    fn default() -> Self {
        Self {
            initial_delay: Duration::from_millis(250),
            max_delay: Duration::from_secs(30),
            multiplier: 2,
        }
    }
}

impl ReconnectPolicy {
    pub fn next_delay(&self, current: Duration) -> Duration {
        current
            .saturating_mul(self.multiplier.max(1))
            .min(self.max_delay)
    }
}

pub struct SupervisedFeed {
    pub raw: mpsc::Receiver<RawEnvelope>,
    pub status: mpsc::Receiver<ConnectionStatus>,
    shutdown: watch::Sender<bool>,
    recovery_routes: RecoveryRoutes,
    tasks: Vec<JoinHandle<()>>,
}

impl SupervisedFeed {
    pub fn take_raw(&mut self) -> mpsc::Receiver<RawEnvelope> {
        let (_sender, replacement) = mpsc::channel(1);
        std::mem::replace(&mut self.raw, replacement)
    }

    pub fn take_status(&mut self) -> mpsc::Receiver<ConnectionStatus> {
        let (_sender, replacement) = mpsc::channel(1);
        std::mem::replace(&mut self.status, replacement)
    }

    pub fn request_recovery(&self, request: &RecoveryRequest) -> usize {
        let Some(routes) = self
            .recovery_routes
            .get(&(request.stream.venue, request.stream.symbol.clone()))
        else {
            return 0;
        };
        routes
            .iter()
            .filter(|(conn_id, route)| {
                if request
                    .source_conn_id
                    .as_ref()
                    .is_some_and(|source| source != conn_id)
                {
                    return false;
                }
                let next = route.borrow().wrapping_add(1);
                route.send(next).is_ok()
            })
            .count()
    }

    pub async fn shutdown(self) {
        let _ = self.shutdown.send(true);
        for task in self.tasks {
            let _ = task.await;
        }
    }
}

pub fn spawn_supervised_feed(
    adapter: Arc<dyn VenueAdapter>,
    plans: Vec<SocketPlan>,
    bootstrap: BootstrapFactory,
    channel_capacity: usize,
    connection_attempt_pacer: ConnectionAttemptPacer,
    reconnect: ReconnectPolicy,
) -> SupervisedFeed {
    let (raw_tx, raw_rx) = mpsc::channel(channel_capacity.max(1));
    let (status_tx, status_rx) = mpsc::channel(channel_capacity.max(1));
    let (shutdown_tx, shutdown_rx) = watch::channel(false);
    let mut tasks = Vec::new();
    let mut recovery_routes = RecoveryRoutes::new();
    for plan in plans {
        let (recovery_tx, recovery_rx) = watch::channel(0_u64);
        let mut routed_symbols = HashSet::new();
        for subscription in &plan.subscriptions {
            if subscription.channel.is_book()
                && let Some(symbol) = &subscription.symbol
                && routed_symbols.insert(symbol.clone())
            {
                recovery_routes
                    .entry((plan.venue, symbol.clone()))
                    .or_default()
                    .push((plan.conn_id.clone(), recovery_tx.clone()));
            }
        }
        let adapter = Arc::clone(&adapter);
        let output = raw_tx.clone();
        let status = status_tx.clone();
        let bootstrap = Arc::clone(&bootstrap);
        let shutdown = shutdown_rx.clone();
        let connection_attempt_pacer = connection_attempt_pacer.clone();
        let reconnect = reconnect.clone();
        tasks.push(tokio::spawn(async move {
            supervise_connection(
                adapter,
                plan,
                bootstrap,
                ConnectionChannels {
                    output,
                    status,
                    shutdown,
                    recovery: recovery_rx,
                },
                connection_attempt_pacer,
                reconnect,
            )
            .await;
        }));
    }
    drop(raw_tx);
    drop(status_tx);
    SupervisedFeed {
        raw: raw_rx,
        status: status_rx,
        shutdown: shutdown_tx,
        recovery_routes,
        tasks,
    }
}

struct ConnectionChannels {
    output: mpsc::Sender<RawEnvelope>,
    status: mpsc::Sender<ConnectionStatus>,
    shutdown: watch::Receiver<bool>,
    recovery: watch::Receiver<u64>,
}

async fn supervise_connection(
    adapter: Arc<dyn VenueAdapter>,
    plan: SocketPlan,
    bootstrap: BootstrapFactory,
    channels: ConnectionChannels,
    connection_attempt_pacer: ConnectionAttemptPacer,
    reconnect: ReconnectPolicy,
) {
    let ConnectionChannels {
        output,
        status,
        mut shutdown,
        mut recovery,
    } = channels;
    let mut delay = reconnect.initial_delay;
    loop {
        if *shutdown.borrow() {
            return;
        }
        match connection_attempt_pacer.wait_for_turn(&mut shutdown).await {
            Ok(true) => {}
            Ok(false) => return,
            Err(error) => {
                tracing::error!(conn_id = %plan.conn_id, %error, "feed connection pacer failed");
                let _ = status
                    .send(ConnectionStatus {
                        conn_id: plan.conn_id.clone(),
                        venue: plan.venue,
                        private: plan.private,
                        ts_ms: crate::unix_time_ns() / 1_000_000,
                        kind: ConnectionStatusKind::Fatal,
                        reason: error.to_string(),
                    })
                    .await;
                return;
            }
        }
        let bootstrap_messages = match bootstrap(&plan) {
            Ok(messages) => messages,
            Err(error) => {
                tracing::warn!(conn_id = %plan.conn_id, ?error, ?delay, "feed bootstrap generation failed");
                tokio::select! {
                    _ = tokio::time::sleep(delay) => {}
                    changed = shutdown.changed() => {
                        if changed.is_err() || *shutdown.borrow() {
                            return;
                        }
                    }
                }
                delay = reconnect.next_delay(delay);
                continue;
            }
        };
        let result = run_connection_once(
            adapter.as_ref(),
            &plan,
            &bootstrap_messages,
            &output,
            &status,
            &mut shutdown,
            &mut recovery,
        )
        .await;
        if *shutdown.borrow() || matches!(result, Ok(())) {
            return;
        }
        let error = result.expect_err("non-success result must contain an error");
        let fatal = matches!(&error, ConnectionError::InvalidSubscriptionPlan(_));
        let disconnected = status.send(ConnectionStatus {
            conn_id: plan.conn_id.clone(),
            venue: plan.venue,
            private: plan.private,
            ts_ms: crate::unix_time_ns() / 1_000_000,
            kind: if fatal {
                ConnectionStatusKind::Fatal
            } else {
                ConnectionStatusKind::Disconnected
            },
            reason: error.to_string(),
        });
        tokio::select! {
            result = disconnected => {
                if result.is_err() {
                    return;
                }
            }
            changed = shutdown.changed() => {
                if changed.is_err() || *shutdown.borrow() {
                    return;
                }
            }
        }
        if fatal {
            tracing::error!(conn_id = %plan.conn_id, ?error, "feed connection plan is invalid");
            return;
        }
        if matches!(error, ConnectionError::RecoveryRequested) {
            delay = reconnect.initial_delay;
            tracing::info!(conn_id = %plan.conn_id, "feed connection restarting for snapshot recovery");
            continue;
        }
        tracing::warn!(conn_id = %plan.conn_id, ?error, ?delay, "feed connection restarting");
        if matches!(error, ConnectionError::OutputClosed) {
            return;
        }
        tokio::select! {
            _ = tokio::time::sleep(delay) => {}
            changed = shutdown.changed() => {
                if changed.is_err() || *shutdown.borrow() {
                    return;
                }
            }
        }
        delay = reconnect.next_delay(delay);
    }
}

#[cfg(test)]
mod tests {
    use reap_core::{Channel, FeedPriority, Subscription};
    use reap_venue::okx::{OkxAdapter, OkxCredentials};

    use super::*;

    #[test]
    fn reconnect_backoff_is_bounded() {
        let policy = ReconnectPolicy {
            initial_delay: Duration::from_millis(10),
            max_delay: Duration::from_millis(25),
            multiplier: 2,
        };
        assert_eq!(
            policy.next_delay(Duration::from_millis(10)),
            Duration::from_millis(20)
        );
        assert_eq!(
            policy.next_delay(Duration::from_millis(20)),
            Duration::from_millis(25)
        );
    }

    #[tokio::test]
    async fn invalid_subscription_plan_is_fatal_without_reconnect() {
        let subscription = Subscription::public(
            Venue::Okx,
            Channel::Books,
            "BTC-USDT",
            FeedPriority::Critical,
        );
        let plan = SocketPlan {
            conn_id: reap_core::ConnId::new("duplicate-plan"),
            venue: Venue::Okx,
            private: false,
            subscriptions: vec![subscription.clone(), subscription],
        };
        let (output, _output_rx) = mpsc::channel(1);
        let (status, mut status_rx) = mpsc::channel(1);
        let (_shutdown_tx, shutdown) = watch::channel(false);
        let (_recovery_tx, recovery) = watch::channel(0_u64);

        supervise_connection(
            Arc::new(OkxAdapter::new("ws://127.0.0.1:9", "ws://127.0.0.1:9")),
            plan,
            no_bootstrap(),
            ConnectionChannels {
                output,
                status,
                shutdown,
                recovery,
            },
            ConnectionAttemptPacer::new(Duration::ZERO),
            ReconnectPolicy::default(),
        )
        .await;

        let fatal = status_rx.recv().await.unwrap();
        assert_eq!(fatal.kind, ConnectionStatusKind::Fatal);
        assert!(fatal.reason.contains("repeats subscription books/BTC-USDT"));
        assert!(status_rx.recv().await.is_none());
    }

    #[test]
    fn connection_attempt_pacer_state_spaces_attempts_and_recovers_after_idle_time() {
        let interval = Duration::from_millis(400);
        let mut state = ConnectionAttemptPacerState::default();
        let start = Instant::now();

        assert_eq!(state.delay_at(start), Duration::ZERO);
        state.record_attempt_at(start, interval);
        assert_eq!(state.delay_at(start), interval);
        assert_eq!(
            state.delay_at(start + Duration::from_millis(100)),
            Duration::from_millis(300)
        );
        state.record_attempt_at(start + interval, interval);
        assert_eq!(
            state.delay_at(start + Duration::from_secs(2)),
            Duration::ZERO
        );
    }

    #[tokio::test]
    async fn process_shared_pacers_reserve_distinct_handshake_slots() {
        let directory = tempfile::tempdir().unwrap();
        let path = directory.path().join("connect.pacer");
        let interval = Duration::from_millis(50);
        let first = ConnectionAttemptPacer::process_shared(interval, &path).unwrap();
        let second = ConnectionAttemptPacer::process_shared(interval, &path).unwrap();
        let (_first_shutdown, mut first_shutdown) = watch::channel(false);
        let (_second_shutdown, mut second_shutdown) = watch::channel(false);

        assert!(first.wait_for_turn(&mut first_shutdown).await.unwrap());
        let started = Instant::now();
        assert!(second.wait_for_turn(&mut second_shutdown).await.unwrap());

        assert!(started.elapsed() >= Duration::from_millis(40));
        let state = std::fs::read_to_string(path).unwrap();
        assert!(state.starts_with(SHARED_PACER_STATE_MAGIC));
    }

    #[tokio::test]
    async fn process_shared_pacer_wait_remains_shutdown_cancellable() {
        let directory = tempfile::tempdir().unwrap();
        let path = directory.path().join("connect.pacer");
        let interval = Duration::from_millis(250);
        let first = ConnectionAttemptPacer::process_shared(interval, &path).unwrap();
        let second = ConnectionAttemptPacer::process_shared(interval, &path).unwrap();
        let (_first_shutdown, mut first_shutdown) = watch::channel(false);
        assert!(first.wait_for_turn(&mut first_shutdown).await.unwrap());
        let (shutdown_tx, mut shutdown_rx) = watch::channel(false);

        let waiter = tokio::spawn(async move { second.wait_for_turn(&mut shutdown_rx).await });
        tokio::time::sleep(Duration::from_millis(20)).await;
        shutdown_tx.send(true).unwrap();

        assert!(!waiter.await.unwrap().unwrap());
    }

    #[tokio::test]
    async fn process_shared_pacer_does_not_reserve_after_shutdown() {
        let directory = tempfile::tempdir().unwrap();
        let path = directory.path().join("connect.pacer");
        let pacer =
            ConnectionAttemptPacer::process_shared(Duration::from_millis(400), &path).unwrap();
        let (_shutdown_tx, mut shutdown_rx) = watch::channel(true);

        assert!(!pacer.wait_for_turn(&mut shutdown_rx).await.unwrap());
        assert!(std::fs::read(path).unwrap().is_empty());
    }

    #[tokio::test]
    async fn process_shared_pacer_fails_closed_on_a_stuck_file_lock() {
        let directory = tempfile::tempdir().unwrap();
        let path = directory.path().join("connect.pacer");
        let pacer =
            ConnectionAttemptPacer::process_shared(Duration::from_millis(400), &path).unwrap();
        let blocker = OpenOptions::new()
            .read(true)
            .write(true)
            .open(&path)
            .unwrap();
        blocker.lock().unwrap();
        let (_shutdown_tx, mut shutdown_rx) = watch::channel(false);
        let started = Instant::now();

        let error = pacer.wait_for_turn(&mut shutdown_rx).await.unwrap_err();

        blocker.unlock().unwrap();
        assert!(matches!(
            error,
            ConnectionAttemptPacerError::LockTimeout { .. }
        ));
        assert!(started.elapsed() >= MAX_SHARED_PACER_LOCK_WAIT);
        assert!(started.elapsed() < Duration::from_secs(2));
    }

    #[tokio::test]
    async fn process_shared_pacer_resets_state_from_another_boot() {
        let directory = tempfile::tempdir().unwrap();
        let path = directory.path().join("connect.pacer");
        let stale = format!(
            "{SHARED_PACER_STATE_MAGIC} 00000000-0000-0000-0000-000000000000 {:039}\n",
            u128::MAX
        );
        std::fs::write(&path, stale).unwrap();
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            std::fs::set_permissions(&path, std::fs::Permissions::from_mode(0o600)).unwrap();
        }
        let pacer =
            ConnectionAttemptPacer::process_shared(Duration::from_millis(400), &path).unwrap();
        let (_shutdown_tx, mut shutdown_rx) = watch::channel(false);
        let started = Instant::now();

        assert!(pacer.wait_for_turn(&mut shutdown_rx).await.unwrap());

        assert!(started.elapsed() < Duration::from_millis(100));
    }

    #[tokio::test]
    async fn process_shared_pacer_rejects_implausibly_distant_reservations() {
        let directory = tempfile::tempdir().unwrap();
        let path = directory.path().join("connect.pacer");
        let pacer =
            ConnectionAttemptPacer::process_shared(Duration::from_millis(400), &path).unwrap();
        let clock_id = pacer.shared.as_ref().unwrap().clock_id.clone();
        let distant = shared_clock_ns().unwrap()
            + MAX_SHARED_RESERVATION_AHEAD.as_nanos()
            + Duration::from_secs(1).as_nanos();
        std::fs::write(
            &path,
            format!("{SHARED_PACER_STATE_MAGIC} {clock_id} {distant:039}\n"),
        )
        .unwrap();
        let (_shutdown_tx, mut shutdown_rx) = watch::channel(false);

        let error = pacer.wait_for_turn(&mut shutdown_rx).await.unwrap_err();

        assert!(matches!(
            error,
            ConnectionAttemptPacerError::ReservationTooFar { .. }
        ));
    }

    #[test]
    fn process_shared_pacer_rejects_malformed_or_exposed_state_files() {
        let directory = tempfile::tempdir().unwrap();
        let malformed = directory.path().join("malformed.pacer");
        std::fs::write(&malformed, b"not-reap-state\n").unwrap();
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            std::fs::set_permissions(&malformed, std::fs::Permissions::from_mode(0o600)).unwrap();
        }
        assert!(matches!(
            ConnectionAttemptPacer::process_shared(Duration::from_millis(400), &malformed),
            Err(ConnectionAttemptPacerError::InvalidState { .. })
        ));

        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            let exposed = directory.path().join("exposed.pacer");
            std::fs::write(&exposed, b"").unwrap();
            std::fs::set_permissions(&exposed, std::fs::Permissions::from_mode(0o640)).unwrap();
            assert!(matches!(
                ConnectionAttemptPacer::process_shared(Duration::from_millis(400), &exposed),
                Err(ConnectionAttemptPacerError::UnsafeFile { .. })
            ));
        }
    }

    #[test]
    fn recovery_routes_include_every_redundant_book_socket() {
        let mut subscription = Subscription::public(
            Venue::Okx,
            Channel::Books,
            "BTC-USDT",
            FeedPriority::Critical,
        );
        subscription.connections = 2;
        let plans = crate::partition_subscriptions(&[subscription], 10).unwrap();
        let count = plans
            .iter()
            .filter(|plan| {
                plan.subscriptions
                    .iter()
                    .any(|subscription| subscription.symbol.as_deref() == Some("BTC-USDT"))
            })
            .count();
        assert_eq!(count, 2);
    }

    #[test]
    fn unscoped_recovery_notifies_every_registered_socket() {
        let (first_route, mut first_rx) = watch::channel(0_u64);
        let (second_route, mut second_rx) = watch::channel(0_u64);
        let (_raw_tx, raw_rx) = mpsc::channel(1);
        let (_status_tx, status_rx) = mpsc::channel(1);
        let (shutdown, _shutdown_rx) = watch::channel(false);
        let feed = SupervisedFeed {
            raw: raw_rx,
            status: status_rx,
            shutdown,
            recovery_routes: HashMap::from([(
                (Venue::Okx, "BTC-USDT".to_string()),
                vec![
                    (ConnId::new("book-0"), first_route),
                    (ConnId::new("book-1"), second_route),
                ],
            )]),
            tasks: Vec::new(),
        };
        let request = RecoveryRequest {
            stream: crate::FeedStreamId {
                venue: Venue::Okx,
                channel: Channel::Books,
                symbol: "BTC-USDT".to_string(),
            },
            source_conn_id: None,
            expected_prev: Some(10),
            received_prev: 11,
            received_seq: 12,
        };

        assert_eq!(feed.request_recovery(&request), 2);
        assert!(first_rx.has_changed().unwrap());
        assert!(second_rx.has_changed().unwrap());
        assert_eq!(*first_rx.borrow_and_update(), 1);
        assert_eq!(*second_rx.borrow_and_update(), 1);
    }

    #[test]
    fn source_scoped_recovery_only_notifies_failed_socket() {
        let (first_route, first_rx) = watch::channel(0_u64);
        let (second_route, mut second_rx) = watch::channel(0_u64);
        let (_raw_tx, raw_rx) = mpsc::channel(1);
        let (_status_tx, status_rx) = mpsc::channel(1);
        let (shutdown, _shutdown_rx) = watch::channel(false);
        let failed_source = ConnId::new("book-1");
        let feed = SupervisedFeed {
            raw: raw_rx,
            status: status_rx,
            shutdown,
            recovery_routes: HashMap::from([(
                (Venue::Okx, "BTC-USDT".to_string()),
                vec![
                    (ConnId::new("book-0"), first_route),
                    (failed_source.clone(), second_route),
                ],
            )]),
            tasks: Vec::new(),
        };
        let request = RecoveryRequest {
            stream: crate::FeedStreamId {
                venue: Venue::Okx,
                channel: Channel::Books,
                symbol: "BTC-USDT".to_string(),
            },
            source_conn_id: Some(failed_source),
            expected_prev: Some(10),
            received_prev: 11,
            received_seq: 12,
        };

        assert_eq!(feed.request_recovery(&request), 1);
        assert!(!first_rx.has_changed().unwrap());
        assert!(second_rx.has_changed().unwrap());
        assert_eq!(*second_rx.borrow_and_update(), 1);
    }

    #[test]
    fn okx_private_bootstrap_builds_login_per_attempt() {
        let factory = okx_login_bootstrap(OkxSigner::new(
            OkxCredentials::new("key", "secret", "pass"),
            true,
        ));
        let private = SocketPlan {
            conn_id: reap_core::ConnId::new("private"),
            venue: Venue::Okx,
            private: true,
            subscriptions: vec![Subscription::private(
                Venue::Okx,
                Channel::Orders,
                FeedPriority::Critical,
            )],
        };
        let public = SocketPlan {
            conn_id: reap_core::ConnId::new("public"),
            venue: Venue::Okx,
            private: false,
            subscriptions: vec![Subscription::public(
                Venue::Okx,
                Channel::Books,
                "BTC-USDT",
                FeedPriority::Critical,
            )],
        };

        let login: serde_json::Value =
            serde_json::from_str(&factory(&private).unwrap()[0]).unwrap();
        assert_eq!(login["op"], "login");
        assert!(login["args"][0]["timestamp"].as_str().is_some());
        assert!(factory(&public).unwrap().is_empty());
    }
}
