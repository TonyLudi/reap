use std::path::{Path, PathBuf};
use std::time::{Duration, SystemTime, UNIX_EPOCH};

pub use reap_core::{
    HostGuardConfig, MAX_HOST_GUARD_CHECK_INTERVAL_MS, PRODUCTION_HOST_GUARD_MAX_CHECK_INTERVAL_MS,
    PRODUCTION_HOST_GUARD_MIN_DISK_AVAILABLE_BYTES,
    PRODUCTION_HOST_GUARD_MIN_MEMORY_AVAILABLE_BYTES,
};
use serde::{Deserialize, Serialize};
use thiserror::Error;
use tokio::sync::{mpsc, oneshot};
use tokio::task::JoinHandle;

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct HostHealthSnapshot {
    pub checked_at_ms: u64,
    pub disk_available_bytes: u64,
    pub memory_available_bytes: u64,
    pub clock_synchronized: bool,
}

#[derive(Debug, Clone, Error)]
pub enum HostHealthError {
    #[error("host health probe failed: {0}")]
    Probe(String),
    #[error("host health threshold breached: {message}")]
    Unhealthy {
        code: String,
        message: String,
        snapshot: HostHealthSnapshot,
    },
}

impl HostHealthError {
    pub fn code(&self) -> &str {
        match self {
            Self::Probe(_) => "host_probe_failed",
            Self::Unhealthy { code, .. } => code,
        }
    }

    pub fn snapshot(&self) -> Option<&HostHealthSnapshot> {
        match self {
            Self::Probe(_) => None,
            Self::Unhealthy { snapshot, .. } => Some(snapshot),
        }
    }
}

#[derive(Debug, Clone, Default)]
pub struct HostGuardStats {
    pub checks: u64,
    pub last_snapshot: Option<HostHealthSnapshot>,
}

pub struct HostGuardRuntime {
    failures: Option<mpsc::Receiver<HostHealthError>>,
    shutdown_tx: Option<oneshot::Sender<()>>,
    task: Option<JoinHandle<HostGuardStats>>,
}

impl HostGuardRuntime {
    pub fn take_failures(&mut self) -> mpsc::Receiver<HostHealthError> {
        self.failures
            .take()
            .expect("host guard failure receiver can only be taken once")
    }

    pub fn request_shutdown(&mut self) {
        if let Some(shutdown_tx) = self.shutdown_tx.take() {
            let _ = shutdown_tx.send(());
        }
    }

    pub async fn shutdown(mut self) -> Result<HostGuardStats, tokio::task::JoinError> {
        self.request_shutdown();
        let result = match self.task.as_mut() {
            Some(task) => task.await,
            None => Ok(HostGuardStats::default()),
        };
        self.task.take();
        result
    }
}

impl Drop for HostGuardRuntime {
    fn drop(&mut self) {
        self.request_shutdown();
        if let Some(task) = &self.task {
            task.abort();
        }
    }
}

pub fn check_host_health(
    config: &HostGuardConfig,
    storage_path: &Path,
) -> Result<HostHealthSnapshot, HostHealthError> {
    let snapshot = probe_host(storage_path)?;
    evaluate_host_health(config, snapshot)
}

pub fn start_host_guard(config: HostGuardConfig, storage_path: PathBuf) -> HostGuardRuntime {
    let (failure_tx, failures) = mpsc::channel(1);
    let (shutdown_tx, mut shutdown_rx) = oneshot::channel();
    let task = tokio::spawn(async move {
        let mut stats = HostGuardStats::default();
        let interval = Duration::from_millis(config.check_interval_ms);
        loop {
            tokio::select! {
                _ = tokio::time::sleep(interval) => {
                    let check_config = config.clone();
                    let check_path = storage_path.clone();
                    let result = tokio::task::spawn_blocking(move || {
                        check_host_health(&check_config, &check_path)
                    }).await;
                    stats.checks = stats.checks.saturating_add(1);
                    match result {
                        Ok(Ok(snapshot)) => stats.last_snapshot = Some(snapshot),
                        Ok(Err(error)) => {
                            let _ = failure_tx.send(error).await;
                            return stats;
                        }
                        Err(error) => {
                            let _ = failure_tx.send(HostHealthError::Probe(format!(
                                "host guard blocking task failed: {error}"
                            ))).await;
                            return stats;
                        }
                    }
                }
                _ = &mut shutdown_rx => return stats,
            }
        }
    });
    HostGuardRuntime {
        failures: Some(failures),
        shutdown_tx: Some(shutdown_tx),
        task: Some(task),
    }
}

fn evaluate_host_health(
    config: &HostGuardConfig,
    snapshot: HostHealthSnapshot,
) -> Result<HostHealthSnapshot, HostHealthError> {
    let mut codes = Vec::new();
    let mut reasons = Vec::new();
    if snapshot.disk_available_bytes < config.min_disk_available_bytes {
        codes.push("disk_low");
        reasons.push(format!(
            "storage filesystem has {} bytes available, below {}",
            snapshot.disk_available_bytes, config.min_disk_available_bytes
        ));
    }
    if snapshot.memory_available_bytes < config.min_memory_available_bytes {
        codes.push("memory_low");
        reasons.push(format!(
            "host has {} bytes available memory, below {}",
            snapshot.memory_available_bytes, config.min_memory_available_bytes
        ));
    }
    if config.require_clock_synchronized && !snapshot.clock_synchronized {
        codes.push("clock_unsynchronized");
        reasons.push("kernel reports the host clock as unsynchronized".to_string());
    }
    if reasons.is_empty() {
        Ok(snapshot)
    } else {
        Err(HostHealthError::Unhealthy {
            code: codes.join("+"),
            message: reasons.join("; "),
            snapshot,
        })
    }
}

#[cfg(target_os = "linux")]
fn probe_host(storage_path: &Path) -> Result<HostHealthSnapshot, HostHealthError> {
    let filesystem_path = storage_path.parent().unwrap_or_else(|| Path::new("."));
    Ok(HostHealthSnapshot {
        checked_at_ms: unix_time_ms(),
        disk_available_bytes: disk_available_bytes(filesystem_path)?,
        memory_available_bytes: memory_available_bytes()?,
        clock_synchronized: clock_synchronized()?,
    })
}

#[cfg(not(target_os = "linux"))]
fn probe_host(_storage_path: &Path) -> Result<HostHealthSnapshot, HostHealthError> {
    Err(HostHealthError::Probe(
        "host guard is currently supported only on Linux".to_string(),
    ))
}

#[cfg(target_os = "linux")]
fn disk_available_bytes(path: &Path) -> Result<u64, HostHealthError> {
    use std::ffi::CString;
    use std::mem::MaybeUninit;
    use std::os::unix::ffi::OsStrExt;

    let path = CString::new(path.as_os_str().as_bytes())
        .map_err(|_| HostHealthError::Probe("storage path contains a NUL byte".to_string()))?;
    let mut stats = MaybeUninit::<libc::statvfs>::uninit();
    // SAFETY: `path` is NUL-terminated and `stats` points to writable storage for statvfs.
    let result = unsafe { libc::statvfs(path.as_ptr(), stats.as_mut_ptr()) };
    if result != 0 {
        return Err(HostHealthError::Probe(format!(
            "statvfs failed for storage filesystem: {}",
            std::io::Error::last_os_error()
        )));
    }
    // SAFETY: statvfs returned success and initialized `stats`.
    let stats = unsafe { stats.assume_init() };
    let fragment_size = if stats.f_frsize == 0 {
        stats.f_bsize
    } else {
        stats.f_frsize
    };
    let available_blocks = unsigned_to_u64(stats.f_bavail);
    let fragment_size = unsigned_to_u64(fragment_size);
    Ok(available_blocks.saturating_mul(fragment_size))
}

#[cfg(target_os = "linux")]
fn unsigned_to_u64<T>(value: T) -> u64
where
    T: TryInto<u64>,
{
    value.try_into().unwrap_or(u64::MAX)
}

#[cfg(target_os = "linux")]
fn memory_available_bytes() -> Result<u64, HostHealthError> {
    let meminfo = std::fs::read_to_string("/proc/meminfo").map_err(|error| {
        HostHealthError::Probe(format!("failed to read /proc/meminfo: {error}"))
    })?;
    parse_memory_available_bytes(&meminfo)
}

#[cfg(target_os = "linux")]
fn parse_memory_available_bytes(meminfo: &str) -> Result<u64, HostHealthError> {
    let line = meminfo
        .lines()
        .find(|line| line.starts_with("MemAvailable:"))
        .ok_or_else(|| HostHealthError::Probe("/proc/meminfo has no MemAvailable".to_string()))?;
    let mut fields = line.split_ascii_whitespace();
    let _label = fields.next();
    let kib = fields
        .next()
        .ok_or_else(|| HostHealthError::Probe("MemAvailable has no value".to_string()))?
        .parse::<u64>()
        .map_err(|error| HostHealthError::Probe(format!("invalid MemAvailable value: {error}")))?;
    if fields.next() != Some("kB") {
        return Err(HostHealthError::Probe(
            "MemAvailable does not use kB units".to_string(),
        ));
    }
    kib.checked_mul(1024)
        .ok_or_else(|| HostHealthError::Probe("MemAvailable overflows bytes".to_string()))
}

#[cfg(target_os = "linux")]
fn clock_synchronized() -> Result<bool, HostHealthError> {
    // SAFETY: a zeroed timex with modes=0 is the documented read-only adjtimex query.
    let mut state: libc::timex = unsafe { std::mem::zeroed() };
    // SAFETY: `state` is a valid mutable timex pointer for the duration of the call.
    let result = unsafe { libc::adjtimex(&mut state) };
    if result < 0 {
        return Err(HostHealthError::Probe(format!(
            "adjtimex failed: {}",
            std::io::Error::last_os_error()
        )));
    }
    Ok(result != libc::TIME_ERROR && state.status & libc::STA_UNSYNC == 0)
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
    use super::*;

    fn config() -> HostGuardConfig {
        HostGuardConfig {
            enabled: true,
            check_interval_ms: 1,
            min_disk_available_bytes: 1_000,
            min_memory_available_bytes: 2_000,
            require_clock_synchronized: true,
        }
    }

    #[cfg(target_os = "linux")]
    #[test]
    fn parses_linux_available_memory_in_bytes() {
        assert_eq!(
            parse_memory_available_bytes("MemTotal: 42 kB\nMemAvailable: 1234 kB\n").unwrap(),
            1_263_616
        );
        assert!(parse_memory_available_bytes("MemTotal: 42 kB\n").is_err());
    }

    #[test]
    fn evaluates_every_host_threshold_fail_closed() {
        let error = evaluate_host_health(
            &config(),
            HostHealthSnapshot {
                checked_at_ms: 1,
                disk_available_bytes: 999,
                memory_available_bytes: 1_999,
                clock_synchronized: false,
            },
        )
        .unwrap_err();
        assert_eq!(error.code(), "disk_low+memory_low+clock_unsynchronized");
        assert_eq!(error.snapshot().unwrap().checked_at_ms, 1);
        assert!(error.to_string().contains("storage filesystem"));
        assert!(error.to_string().contains("available memory"));
        assert!(error.to_string().contains("unsynchronized"));
    }

    #[test]
    fn healthy_snapshot_is_preserved_as_evidence() {
        let snapshot = HostHealthSnapshot {
            checked_at_ms: 1,
            disk_available_bytes: 1_000,
            memory_available_bytes: 2_000,
            clock_synchronized: true,
        };
        assert_eq!(
            evaluate_host_health(&config(), snapshot.clone()).unwrap(),
            snapshot
        );
    }

    #[cfg(target_os = "linux")]
    #[tokio::test]
    async fn periodic_guard_reports_first_failure_and_stops() {
        let mut config = config();
        config.min_disk_available_bytes = u64::MAX;
        config.min_memory_available_bytes = 1;
        config.require_clock_synchronized = false;
        let path = std::env::temp_dir().join("reap-host-guard-test.jsonl");
        let mut guard = start_host_guard(config, path);
        let mut failures = guard.take_failures();

        let failure = tokio::time::timeout(Duration::from_secs(1), failures.recv())
            .await
            .unwrap()
            .unwrap();
        assert_eq!(failure.code(), "disk_low");
        let stats = guard.shutdown().await.unwrap();
        assert_eq!(stats.checks, 1);
    }
}
