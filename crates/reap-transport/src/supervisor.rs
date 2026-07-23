use std::fs::{File, OpenOptions};
use std::io::{Read, Seek, SeekFrom, Write};
use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex as StdMutex};
use std::time::{Duration, Instant};

#[cfg(unix)]
use std::os::unix::fs::{MetadataExt, OpenOptionsExt};

use thiserror::Error;
use tokio::sync::{Mutex, mpsc};

use crate::backoff::{ReconnectBackoff, ReconnectPolicy};
use crate::bounded::bounded_channel;
use crate::health::ConnectionStatusKind;
use crate::shutdown::{ShutdownReceiver, ShutdownSender, shutdown_channel, shutdown_requested};

const MAX_SHARED_PACER_STATE_BYTES: u64 = 128;
const SHARED_PACER_TIMESTAMP_WIDTH: usize = 39;
const MAX_SHARED_RESERVATION_AHEAD: Duration = Duration::from_secs(15 * 60);
const MAX_SHARED_PACER_LOCK_WAIT: Duration = Duration::from_secs(1);
const SHARED_PACER_LOCK_RETRY: Duration = Duration::from_millis(5);
const MAX_STATE_MAGIC_BYTES: usize = 48;

/// Pacing for connection handshakes within a process and, when configured,
/// across cooperating processes sharing one state file.
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
    state_magic: &'static str,
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
    #[must_use]
    pub fn new(interval: Duration) -> Self {
        Self {
            interval,
            state: Arc::new(Mutex::new(ConnectionAttemptPacerState::default())),
            shared: None,
        }
    }

    /// Opens a process-shared pacer under a caller-owned, schema-stable state
    /// identity. The identity is mechanics metadata, not a protocol selector.
    pub fn process_shared(
        interval: Duration,
        path: impl AsRef<Path>,
        state_magic: &'static str,
    ) -> Result<Self, ConnectionAttemptPacerError> {
        validate_state_magic(path.as_ref(), state_magic)?;
        ensure_process_shared_pacing_supported()?;
        Ok(Self {
            interval,
            state: Arc::new(Mutex::new(ConnectionAttemptPacerState::default())),
            shared: Some(Arc::new(SharedConnectionAttemptPacer::open(
                path.as_ref(),
                state_magic,
            )?)),
        })
    }

    #[must_use]
    pub const fn interval(&self) -> Duration {
        self.interval
    }

    #[must_use]
    pub const fn is_process_shared(&self) -> bool {
        self.shared.is_some()
    }

    pub async fn wait_for_turn(
        &self,
        shutdown: &mut ShutdownReceiver,
    ) -> Result<bool, ConnectionAttemptPacerError> {
        if self.interval.is_zero() {
            return Ok(!shutdown_requested(shutdown));
        }
        if shutdown_requested(shutdown) {
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
                        if changed.is_err() || shutdown_requested(shutdown) {
                            return Ok(false);
                        }
                    }
                }
            }
            return Ok(!shutdown_requested(shutdown));
        }
        'wait: loop {
            let mut state = tokio::select! {
                state = self.state.lock() => state,
                changed = shutdown.changed() => {
                    if changed.is_err() || shutdown_requested(shutdown) {
                        return Ok(false);
                    }
                    continue 'wait;
                }
            };
            if shutdown_requested(shutdown) {
                return Ok(false);
            }
            let delay = state.delay_at(Instant::now());
            if !delay.is_zero() {
                tokio::select! {
                    _ = tokio::time::sleep(delay) => {}
                    changed = shutdown.changed() => {
                        if changed.is_err() || shutdown_requested(shutdown) {
                            return Ok(false);
                        }
                        continue 'wait;
                    }
                }
            }
            if shutdown_requested(shutdown) {
                return Ok(false);
            }
            state.record_attempt_at(Instant::now(), self.interval);
            return Ok(true);
        }
    }
}

impl SharedConnectionAttemptPacer {
    fn open(path: &Path, state_magic: &'static str) -> Result<Self, ConnectionAttemptPacerError> {
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
            state_magic,
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
        if fields.next() != Some(self.state_magic) {
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
            "{} {} {next_ns:0SHARED_PACER_TIMESTAMP_WIDTH$}\n",
            self.state_magic, self.clock_id
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

fn validate_state_magic(path: &Path, state_magic: &str) -> Result<(), ConnectionAttemptPacerError> {
    if state_magic.is_empty()
        || state_magic.len() > MAX_STATE_MAGIC_BYTES
        || !state_magic
            .bytes()
            .all(|byte| byte.is_ascii_lowercase() || byte.is_ascii_digit() || byte == b'-')
    {
        return Err(ConnectionAttemptPacerError::InvalidState {
            path: path.to_path_buf(),
            reason: "state magic must be 1..=48 lowercase ASCII letters, digits, or hyphens"
                .to_string(),
        });
    }
    Ok(())
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

/// Shared bounded channels and one cooperative shutdown signal for a
/// supervisor owned by a higher-level session protocol.
pub struct SupervisionChannels<T, H> {
    pub output_sender: mpsc::Sender<T>,
    pub output_receiver: mpsc::Receiver<T>,
    pub health_sender: mpsc::Sender<H>,
    pub health_receiver: mpsc::Receiver<H>,
    pub shutdown_sender: ShutdownSender,
    pub shutdown_receiver: ShutdownReceiver,
}

#[must_use]
pub fn supervision_channels<T, H>(requested_capacity: usize) -> SupervisionChannels<T, H> {
    let (output_sender, output_receiver) = bounded_channel(requested_capacity);
    let (health_sender, health_receiver) = bounded_channel(requested_capacity);
    let (shutdown_sender, shutdown_receiver) = shutdown_channel();
    SupervisionChannels {
        output_sender,
        output_receiver,
        health_sender,
        health_receiver,
        shutdown_sender,
        shutdown_receiver,
    }
}

/// Venue-neutral retry and health state for one session supervisor.
#[derive(Debug)]
pub struct SupervisorState {
    backoff: ReconnectBackoff,
    health: ConnectionStatusKind,
}

impl SupervisorState {
    #[must_use]
    pub fn new(policy: ReconnectPolicy) -> Self {
        Self {
            backoff: ReconnectBackoff::new(policy),
            health: ConnectionStatusKind::Disconnected,
        }
    }

    #[must_use]
    pub const fn health(&self) -> ConnectionStatusKind {
        self.health
    }

    pub fn mark_ready(&mut self) {
        self.health = ConnectionStatusKind::Ready;
    }

    pub fn mark_heartbeat(&mut self) {
        self.health = ConnectionStatusKind::Heartbeat;
    }

    pub fn mark_disconnected(&mut self) {
        self.health = ConnectionStatusKind::Disconnected;
    }

    pub fn mark_fatal(&mut self) {
        self.health = ConnectionStatusKind::Fatal;
    }

    pub fn reset_for_recovery(&mut self) {
        self.backoff.reset();
        self.health = ConnectionStatusKind::Disconnected;
    }

    #[must_use]
    pub fn preview_after_failure(&self, reached_ready: bool) -> Duration {
        self.backoff.preview_after_failure(reached_ready)
    }

    pub fn after_failure(&mut self, reached_ready: bool) -> Duration {
        self.health = ConnectionStatusKind::Disconnected;
        self.backoff.after_failure(reached_ready)
    }

    #[must_use]
    pub fn should_stop(&self, shutdown: &ShutdownReceiver) -> bool {
        self.health == ConnectionStatusKind::Fatal || shutdown_requested(shutdown)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn local_attempt_state_spaces_attempts_and_recovers_after_idle_time() {
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
}
