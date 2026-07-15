use std::collections::{BTreeMap, HashMap, HashSet};
use std::io::{Seek, SeekFrom, Write};
use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::sync::atomic::{AtomicU64, AtomicUsize, Ordering};

use reap_core::{
    AccountUpdate, FillFee, FillKey, FillLiquidity, NormalizedEvent, OrderIntent, OrderUpdate,
    Price, Quantity, RawEnvelope, Side, Symbol, SystemEvent, SystemEventKind, TimeMs,
};
use serde::{Deserialize, Serialize};
use thiserror::Error;
use tokio::io::AsyncWriteExt;
use tokio::sync::{mpsc, oneshot};
use tokio::task::JoinHandle;

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(tag = "kind", content = "data", rename_all = "snake_case")]
pub enum StorageRecord {
    Raw {
        account_id: Option<String>,
        envelope: RawEnvelope,
    },
    Normalized(NormalizedEvent),
    Intent {
        ts_ms: TimeMs,
        intent: OrderIntent,
    },
    IntentRejected {
        ts_ms: TimeMs,
        intent: OrderIntent,
        reason: String,
    },
    Bootstrap(BootstrapRecord),
    SessionStart(SessionStartRecord),
    AccountSnapshot(AccountSnapshotRecord),
    OrderRequest(OrderRequestRecord),
    OrderAck(OrderAckRecord),
    Order {
        account_id: Option<String>,
        update: OrderUpdate,
    },
    Fill(FillRecord),
    System(SystemEvent),
    SafetyLatch(SafetyLatchRecord),
    Reconciliation(ReconciliationRecord),
}

impl StorageRecord {
    fn is_critical(&self) -> bool {
        !matches!(self, Self::Raw { .. } | Self::Normalized(_))
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct OrderRequestRecord {
    pub ts_ms: TimeMs,
    pub account_id: String,
    pub operation: OrderOperation,
    pub idempotency_key: Option<String>,
    pub client_order_id: Option<String>,
    pub exchange_order_id: Option<String>,
    pub symbol: Symbol,
}

#[derive(Debug, Clone, Copy, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum OrderOperation {
    Submit,
    Cancel,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct OrderAckRecord {
    pub ts_ms: TimeMs,
    pub account_id: String,
    pub operation: OrderOperation,
    pub client_order_id: String,
    pub exchange_order_id: Option<String>,
    pub status: OrderAckStatus,
    pub message: String,
}

#[derive(Debug, Clone, Copy, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum OrderAckStatus {
    Accepted,
    Duplicate,
    PendingReconciliation,
    Rejected,
    Ambiguous,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ReconciliationRecord {
    pub ts_ms: TimeMs,
    pub account_id: String,
    pub clean: bool,
    pub local_live_orders: usize,
    pub remote_live_orders: usize,
    pub remote_recent_fills: usize,
    pub reason: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(tag = "scope", rename_all = "snake_case")]
pub enum SafetyLatchScope {
    Global,
    Account { account_id: String },
    Symbol { symbol: Symbol },
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum SafetyLatchSource {
    Operator,
    Risk,
    LegacySystemEvent,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct SafetyLatchRecord {
    pub ts_ms: TimeMs,
    pub scope: SafetyLatchScope,
    pub active: bool,
    pub source: SafetyLatchSource,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub request_id: Option<String>,
    pub reason: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct BootstrapRecord {
    pub ts_ms: TimeMs,
    pub account_id: String,
    pub strategy_name: String,
    pub config_fingerprint: String,
    pub baseline_fill_ids: Vec<FillKey>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SessionStartRecord {
    pub ts_ms: TimeMs,
    pub session_id: String,
    pub account_id: String,
    pub strategy_name: String,
    pub config_fingerprint: String,
    pub account_identity_sha256: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct AccountSnapshotRecord {
    pub ts_ms: TimeMs,
    pub account_id: String,
    pub update: AccountUpdate,
}

#[derive(Debug, Clone, Default)]
pub struct RecoveredStorage {
    pub latest_orders: HashMap<String, OrderUpdate>,
    pub order_bindings: HashMap<String, HashMap<String, String>>,
    pub fills: Vec<FillRecord>,
    pub seen_fill_keys: HashSet<FillKey>,
    pub baseline_fill_ids: HashMap<String, HashSet<FillKey>>,
    pub bootstrap_identities: HashMap<String, (String, String)>,
    pub global_safety_latch: Option<SafetyLatchRecord>,
    pub account_safety_latches: BTreeMap<String, SafetyLatchRecord>,
    pub symbol_safety_latches: BTreeMap<Symbol, SafetyLatchRecord>,
    pub last_ts_ms: TimeMs,
    pub records: u64,
    pub ignored_truncated_tail: bool,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct FillRecord {
    pub ts_ms: TimeMs,
    pub account_id: Option<String>,
    pub fill_id: String,
    pub order_id: String,
    pub symbol: Symbol,
    pub side: Side,
    pub price: Price,
    pub qty: Quantity,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub liquidity: Option<FillLiquidity>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub fee: Option<FillFee>,
}

#[derive(Debug, Clone)]
pub struct StorageConfig {
    pub path: PathBuf,
    pub channel_capacity: usize,
    pub flush_every_records: usize,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum StorageWriteOutcome {
    Queued,
    DroppedBestEffort,
}

#[derive(Debug, Error)]
pub enum StorageError {
    #[error("storage writer is closed")]
    Closed,
    #[error("storage queue is full for a critical record")]
    Backpressure,
    #[error("storage IO failed: {0}")]
    Io(#[from] std::io::Error),
    #[error("storage serialization failed: {0}")]
    Serialization(#[from] serde_json::Error),
    #[error("storage path {path} is invalid: {message}")]
    InvalidPath { path: PathBuf, message: String },
    #[error("storage journal {path} is already owned by another process via {lock_path}")]
    AlreadyLocked { path: PathBuf, lock_path: PathBuf },
    #[error("storage lease for {lease_path} does not match configured journal {config_path}")]
    LeaseMismatch {
        config_path: PathBuf,
        lease_path: PathBuf,
    },
    #[error("durable storage write failed: {0}")]
    Durability(String),
    #[error("storage recovery found an invalid record on line {line}: {message}")]
    Corrupt { line: usize, message: String },
    #[error("storage writer task failed: {0}")]
    Join(#[from] tokio::task::JoinError),
}

#[derive(Clone)]
pub struct StorageSink {
    sender: mpsc::Sender<PendingRecord>,
    dropped: Arc<AtomicU64>,
    queued: Arc<AtomicUsize>,
}

struct PendingRecord {
    record: StorageRecord,
    durable_ack: Option<oneshot::Sender<Result<(), String>>>,
}

impl StorageSink {
    pub async fn record(&self, record: StorageRecord) -> Result<StorageWriteOutcome, StorageError> {
        let pending = PendingRecord {
            record,
            durable_ack: None,
        };
        if pending.record.is_critical() {
            self.queued.fetch_add(1, Ordering::Relaxed);
            if self.sender.send(pending).await.is_err() {
                self.queued.fetch_sub(1, Ordering::Relaxed);
                return Err(StorageError::Closed);
            }
            return Ok(StorageWriteOutcome::Queued);
        }
        self.queued.fetch_add(1, Ordering::Relaxed);
        match self.sender.try_send(pending) {
            Ok(()) => Ok(StorageWriteOutcome::Queued),
            Err(mpsc::error::TrySendError::Full(_)) => {
                self.queued.fetch_sub(1, Ordering::Relaxed);
                self.dropped.fetch_add(1, Ordering::Relaxed);
                Ok(StorageWriteOutcome::DroppedBestEffort)
            }
            Err(mpsc::error::TrySendError::Closed(_)) => {
                self.queued.fetch_sub(1, Ordering::Relaxed);
                Err(StorageError::Closed)
            }
        }
    }

    pub async fn record_durable(&self, record: StorageRecord) -> Result<(), StorageError> {
        let (durable_ack, ack) = oneshot::channel();
        self.queued.fetch_add(1, Ordering::Relaxed);
        if self
            .sender
            .send(PendingRecord {
                record,
                durable_ack: Some(durable_ack),
            })
            .await
            .is_err()
        {
            self.queued.fetch_sub(1, Ordering::Relaxed);
            return Err(StorageError::Closed);
        }
        match ack.await {
            Ok(Ok(())) => Ok(()),
            Ok(Err(message)) => Err(StorageError::Durability(message)),
            Err(_) => Err(StorageError::Closed),
        }
    }

    pub fn try_record(&self, record: StorageRecord) -> Result<StorageWriteOutcome, StorageError> {
        let critical = record.is_critical();
        self.queued.fetch_add(1, Ordering::Relaxed);
        match self.sender.try_send(PendingRecord {
            record,
            durable_ack: None,
        }) {
            Ok(()) => Ok(StorageWriteOutcome::Queued),
            Err(mpsc::error::TrySendError::Full(_)) if critical => {
                self.queued.fetch_sub(1, Ordering::Relaxed);
                Err(StorageError::Backpressure)
            }
            Err(mpsc::error::TrySendError::Full(_)) => {
                self.queued.fetch_sub(1, Ordering::Relaxed);
                self.dropped.fetch_add(1, Ordering::Relaxed);
                Ok(StorageWriteOutcome::DroppedBestEffort)
            }
            Err(mpsc::error::TrySendError::Closed(_)) => {
                self.queued.fetch_sub(1, Ordering::Relaxed);
                Err(StorageError::Closed)
            }
        }
    }

    pub fn dropped_records(&self) -> u64 {
        self.dropped.load(Ordering::Relaxed)
    }

    pub fn queue_depth(&self) -> usize {
        self.queued.load(Ordering::Relaxed)
    }
}

pub struct StorageRuntime {
    sink: StorageSink,
    shutdown: Option<oneshot::Sender<()>>,
    task: Option<JoinHandle<Result<(), StorageError>>>,
    _lease: Arc<StorageLease>,
}

impl StorageRuntime {
    pub fn sink(&self) -> StorageSink {
        self.sink.clone()
    }

    pub async fn stop_writer(&mut self) -> Result<(), StorageError> {
        if let Some(shutdown) = self.shutdown.take() {
            let _ = shutdown.send(());
        }
        if let Some(task) = self.task.take() {
            task.await??;
        }
        Ok(())
    }

    pub async fn shutdown(mut self) -> Result<(), StorageError> {
        self.stop_writer().await?;
        Ok(())
    }
}

#[derive(Debug)]
pub struct StorageLease {
    journal_path: PathBuf,
    lock_path: PathBuf,
    lock_file: std::fs::File,
}

impl StorageLease {
    pub fn journal_path(&self) -> &Path {
        &self.journal_path
    }

    pub fn lock_path(&self) -> &Path {
        &self.lock_path
    }
}

impl Drop for StorageLease {
    fn drop(&mut self) {
        let _ = self.lock_file.unlock();
    }
}

pub fn acquire_storage_lease(path: impl AsRef<Path>) -> Result<StorageLease, StorageError> {
    let journal_path = normalize_journal_path(path.as_ref())?;
    let lock_path = lock_path_for(&journal_path)?;
    validate_regular_file_if_present(&lock_path, "lock path")?;

    let mut options = std::fs::OpenOptions::new();
    options.create(true).read(true).write(true);
    #[cfg(unix)]
    {
        use std::os::unix::fs::OpenOptionsExt;
        options.mode(0o600);
    }
    let mut lock_file = options.open(&lock_path)?;
    match lock_file.try_lock() {
        Ok(()) => {}
        Err(std::fs::TryLockError::WouldBlock) => {
            return Err(StorageError::AlreadyLocked {
                path: journal_path,
                lock_path,
            });
        }
        Err(std::fs::TryLockError::Error(error)) => return Err(error.into()),
    }
    #[cfg(unix)]
    std::fs::set_permissions(&lock_path, unix_private_permissions())?;
    lock_file.set_len(0)?;
    lock_file.seek(SeekFrom::Start(0))?;
    writeln!(
        lock_file,
        "pid={} acquired_at_ms={}",
        std::process::id(),
        unix_time_ms()
    )?;
    lock_file.sync_data()?;

    Ok(StorageLease {
        journal_path,
        lock_path,
        lock_file,
    })
}

fn normalize_journal_path(path: &Path) -> Result<PathBuf, StorageError> {
    let file_name = path.file_name().ok_or_else(|| StorageError::InvalidPath {
        path: path.to_path_buf(),
        message: "journal path must name a file".to_string(),
    })?;
    let parent = path
        .parent()
        .filter(|parent| !parent.as_os_str().is_empty())
        .unwrap_or_else(|| Path::new("."));
    std::fs::create_dir_all(parent)?;
    let canonical_parent = std::fs::canonicalize(parent)?;
    let journal_path = canonical_parent.join(file_name);
    validate_regular_file_if_present(&journal_path, "journal path")?;
    Ok(journal_path)
}

fn lock_path_for(journal_path: &Path) -> Result<PathBuf, StorageError> {
    let file_name = journal_path
        .file_name()
        .ok_or_else(|| StorageError::InvalidPath {
            path: journal_path.to_path_buf(),
            message: "journal path must name a file".to_string(),
        })?;
    let mut lock_name = file_name.to_os_string();
    lock_name.push(".lock");
    Ok(journal_path.with_file_name(lock_name))
}

fn validate_regular_file_if_present(path: &Path, label: &str) -> Result<(), StorageError> {
    match std::fs::symlink_metadata(path) {
        Ok(metadata) if metadata.file_type().is_symlink() => Err(StorageError::InvalidPath {
            path: path.to_path_buf(),
            message: format!("{label} must not be a symbolic link"),
        }),
        Ok(metadata) if !metadata.is_file() => Err(StorageError::InvalidPath {
            path: path.to_path_buf(),
            message: format!("{label} must be a regular file"),
        }),
        Ok(_) => Ok(()),
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => Ok(()),
        Err(error) => Err(error.into()),
    }
}

#[cfg(unix)]
fn unix_private_permissions() -> std::fs::Permissions {
    use std::os::unix::fs::PermissionsExt;
    std::fs::Permissions::from_mode(0o600)
}

fn unix_time_ms() -> u64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap_or_default()
        .as_millis()
        .min(u64::MAX as u128) as u64
}

#[derive(Debug, Serialize, Deserialize)]
struct StoredEnvelope {
    schema_version: u16,
    record: StorageRecord,
}

const CURRENT_SCHEMA_VERSION: u16 = 7;

pub fn recover_jsonl(path: impl AsRef<Path>) -> Result<RecoveredStorage, StorageError> {
    let path = path.as_ref();
    let bytes = match std::fs::read(path) {
        Ok(bytes) => bytes,
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => {
            return Ok(RecoveredStorage::default());
        }
        Err(error) => return Err(error.into()),
    };
    recover_jsonl_bytes(&bytes)
}

/// Recovers exactly the supplied journal bytes.
///
/// Evidence tooling uses this entry point so the bytes it fingerprints are the
/// same bytes used to reconstruct fills and checkpoints.
pub fn recover_jsonl_bytes(bytes: &[u8]) -> Result<RecoveredStorage, StorageError> {
    recover_jsonl_bytes_with_visitor(bytes, |_, _| {})
}

/// Recovers exactly the supplied journal bytes while streaming each validated
/// record to an evidence visitor before normal recovery consumes it.
///
/// The visitor receives borrowed records, so callers can retain only the small
/// subset needed for offline evidence without adding normalized market traffic
/// to [`RecoveredStorage`] or live startup memory.
pub fn recover_jsonl_bytes_with_visitor<F>(
    bytes: &[u8],
    mut visitor: F,
) -> Result<RecoveredStorage, StorageError>
where
    F: FnMut(u64, &StorageRecord),
{
    let text = std::str::from_utf8(bytes).map_err(|error| {
        StorageError::Io(std::io::Error::new(std::io::ErrorKind::InvalidData, error))
    })?;
    let trailing_newline = text.ends_with('\n');
    let lines = text.lines().collect::<Vec<_>>();
    let mut recovered = RecoveredStorage::default();
    let mut explicit_latch_scopes = HashSet::new();
    for (index, line) in lines.iter().enumerate() {
        if line.trim().is_empty() {
            continue;
        }
        let envelope: StoredEnvelope = match serde_json::from_str(line) {
            Ok(envelope) => envelope,
            Err(_) if index + 1 == lines.len() && !trailing_newline => {
                recovered.ignored_truncated_tail = true;
                break;
            }
            Err(error) => {
                return Err(StorageError::Corrupt {
                    line: index + 1,
                    message: error.to_string(),
                });
            }
        };
        if !matches!(
            envelope.schema_version,
            2 | 3 | 4 | 5 | 6 | CURRENT_SCHEMA_VERSION
        ) {
            return Err(StorageError::Corrupt {
                line: index + 1,
                message: format!("unsupported schema version {}", envelope.schema_version),
            });
        }
        visitor((index + 1) as u64, &envelope.record);
        recovered.records += 1;
        recovered.last_ts_ms = recovered
            .last_ts_ms
            .max(storage_record_ts_ms(&envelope.record));
        match envelope.record {
            StorageRecord::Bootstrap(bootstrap) => {
                recovered.bootstrap_identities.insert(
                    bootstrap.account_id.clone(),
                    (bootstrap.strategy_name, bootstrap.config_fingerprint),
                );
                recovered.baseline_fill_ids.insert(
                    bootstrap.account_id,
                    bootstrap.baseline_fill_ids.into_iter().collect(),
                );
            }
            StorageRecord::Order { update, .. } => {
                recovered
                    .latest_orders
                    .insert(update.order_id.clone(), update);
            }
            StorageRecord::OrderAck(ack) => {
                let Some(exchange_order_id) = ack.exchange_order_id else {
                    continue;
                };
                if ack.client_order_id.is_empty()
                    || ack.client_order_id == "0"
                    || exchange_order_id.is_empty()
                    || exchange_order_id == "0"
                {
                    continue;
                }
                let bindings = recovered
                    .order_bindings
                    .entry(ack.account_id.clone())
                    .or_default();
                if let Some(existing_client_order_id) = bindings.get(&exchange_order_id)
                    && existing_client_order_id != &ack.client_order_id
                {
                    return Err(StorageError::Corrupt {
                        line: index + 1,
                        message: format!(
                            "account {} exchange order {} is bound to both client orders {} and {}",
                            ack.account_id,
                            exchange_order_id,
                            existing_client_order_id,
                            ack.client_order_id
                        ),
                    });
                }
                if let Some(existing_exchange_order_id) = bindings.iter().find_map(
                    |(existing_exchange_order_id, existing_client_order_id)| {
                        (existing_client_order_id == &ack.client_order_id
                            && existing_exchange_order_id != &exchange_order_id)
                            .then_some(existing_exchange_order_id)
                    },
                ) {
                    return Err(StorageError::Corrupt {
                        line: index + 1,
                        message: format!(
                            "account {} client order {} is bound to both exchange orders {} and {}",
                            ack.account_id,
                            ack.client_order_id,
                            existing_exchange_order_id,
                            exchange_order_id
                        ),
                    });
                }
                bindings.insert(exchange_order_id, ack.client_order_id);
            }
            StorageRecord::Fill(fill) => {
                recovered
                    .seen_fill_keys
                    .insert(FillKey::new(fill.symbol.clone(), fill.fill_id.clone()));
                recovered.fills.push(fill);
            }
            StorageRecord::SafetyLatch(latch) => {
                explicit_latch_scopes.insert(latch.scope.clone());
                apply_recovered_latch(&mut recovered, latch);
            }
            StorageRecord::System(system) => {
                if let Some(latch) = legacy_system_latch(&system)
                    && !explicit_latch_scopes.contains(&latch.scope)
                {
                    apply_recovered_latch(&mut recovered, latch);
                }
            }
            _ => {}
        }
    }
    Ok(recovered)
}

fn apply_recovered_latch(recovered: &mut RecoveredStorage, latch: SafetyLatchRecord) {
    match &latch.scope {
        SafetyLatchScope::Global => {
            recovered.global_safety_latch = latch.active.then_some(latch);
        }
        SafetyLatchScope::Account { account_id } => {
            if latch.active {
                recovered
                    .account_safety_latches
                    .insert(account_id.clone(), latch);
            } else {
                recovered.account_safety_latches.remove(account_id);
            }
        }
        SafetyLatchScope::Symbol { symbol } => {
            if latch.active {
                recovered
                    .symbol_safety_latches
                    .insert(symbol.clone(), latch);
            } else {
                recovered.symbol_safety_latches.remove(symbol);
            }
        }
    }
}

fn legacy_system_latch(system: &SystemEvent) -> Option<SafetyLatchRecord> {
    const OPERATOR_REASON_PREFIX: &str = "authenticated operator request ";

    let (scope, active, source) = match system.kind {
        SystemEventKind::RiskBreach => (SafetyLatchScope::Global, true, SafetyLatchSource::Risk),
        SystemEventKind::KillSwitchActivated
            if system.reason.starts_with(OPERATOR_REASON_PREFIX) =>
        {
            (
                SafetyLatchScope::Global,
                true,
                SafetyLatchSource::LegacySystemEvent,
            )
        }
        SystemEventKind::AccountHalted if system.reason.starts_with(OPERATOR_REASON_PREFIX) => (
            SafetyLatchScope::Account {
                account_id: system.account_id.clone()?,
            },
            true,
            SafetyLatchSource::LegacySystemEvent,
        ),
        SystemEventKind::SymbolHalted if system.reason.starts_with(OPERATOR_REASON_PREFIX) => (
            SafetyLatchScope::Symbol {
                symbol: system.symbol.clone()?,
            },
            true,
            SafetyLatchSource::LegacySystemEvent,
        ),
        SystemEventKind::SymbolResumed if system.reason.starts_with(OPERATOR_REASON_PREFIX) => (
            SafetyLatchScope::Symbol {
                symbol: system.symbol.clone()?,
            },
            false,
            SafetyLatchSource::LegacySystemEvent,
        ),
        _ => return None,
    };
    Some(SafetyLatchRecord {
        ts_ms: system.ts_ms,
        scope,
        active,
        source,
        request_id: None,
        reason: system.reason.clone(),
    })
}

fn storage_record_ts_ms(record: &StorageRecord) -> TimeMs {
    match record {
        StorageRecord::Raw { envelope, .. } => envelope.recv_ts_ns / 1_000_000,
        StorageRecord::Normalized(event) => event.ts_ms(),
        StorageRecord::Intent { ts_ms, .. } | StorageRecord::IntentRejected { ts_ms, .. } => *ts_ms,
        StorageRecord::Bootstrap(bootstrap) => bootstrap.ts_ms,
        StorageRecord::SessionStart(session) => session.ts_ms,
        StorageRecord::AccountSnapshot(snapshot) => snapshot.ts_ms,
        StorageRecord::OrderRequest(request) => request.ts_ms,
        StorageRecord::OrderAck(ack) => ack.ts_ms,
        StorageRecord::Order { update, .. } => update.ts_ms,
        StorageRecord::Fill(fill) => fill.ts_ms,
        StorageRecord::System(system) => system.ts_ms,
        StorageRecord::SafetyLatch(latch) => latch.ts_ms,
        StorageRecord::Reconciliation(reconciliation) => reconciliation.ts_ms,
    }
}

pub async fn start_jsonl_storage(config: StorageConfig) -> Result<StorageRuntime, StorageError> {
    let lease = acquire_storage_lease(&config.path)?;
    start_jsonl_storage_with_lease(config, lease).await
}

pub async fn start_jsonl_storage_with_lease(
    mut config: StorageConfig,
    lease: StorageLease,
) -> Result<StorageRuntime, StorageError> {
    let normalized_config_path = normalize_journal_path(&config.path)?;
    if normalized_config_path != lease.journal_path {
        return Err(StorageError::LeaseMismatch {
            config_path: normalized_config_path,
            lease_path: lease.journal_path.clone(),
        });
    }
    config.path = lease.journal_path.clone();
    let file = open_storage_file(&config).await?;
    let lease = Arc::new(lease);
    let (sender, receiver) = mpsc::channel(config.channel_capacity.max(1));
    let (shutdown, shutdown_rx) = oneshot::channel();
    let dropped = Arc::new(AtomicU64::new(0));
    let queued = Arc::new(AtomicUsize::new(0));
    let sink = StorageSink {
        sender,
        dropped,
        queued: Arc::clone(&queued),
    };
    let writer_lease = Arc::clone(&lease);
    let task = tokio::spawn(async move {
        let _lease = writer_lease;
        run_writer_with_file(config, file, receiver, shutdown_rx, queued).await
    });
    Ok(StorageRuntime {
        sink,
        shutdown: Some(shutdown),
        task: Some(task),
        _lease: lease,
    })
}

async fn open_storage_file(config: &StorageConfig) -> Result<tokio::fs::File, StorageError> {
    if let Some(parent) = config.path.parent() {
        tokio::fs::create_dir_all(parent).await?;
    }
    let file = tokio::fs::OpenOptions::new()
        .create(true)
        .append(true)
        .open(&config.path)
        .await?;
    #[cfg(unix)]
    tokio::fs::set_permissions(&config.path, unix_private_permissions()).await?;
    Ok(file)
}

async fn run_writer_with_file(
    config: StorageConfig,
    mut file: tokio::fs::File,
    mut receiver: mpsc::Receiver<PendingRecord>,
    mut shutdown: oneshot::Receiver<()>,
    queued: Arc<AtomicUsize>,
) -> Result<(), StorageError> {
    let flush_every = config.flush_every_records.max(1);
    let mut since_flush = 0_usize;

    loop {
        tokio::select! {
            biased;
            _ = &mut shutdown => {
                receiver.close();
                while let Some(pending) = receiver.recv().await {
                    let result = write_pending(&mut file, pending).await;
                    queued.fetch_sub(1, Ordering::Relaxed);
                    result?;
                }
                break;
            }
            pending = receiver.recv() => {
                let Some(pending) = pending else { break; };
                let result = write_pending(&mut file, pending).await;
                queued.fetch_sub(1, Ordering::Relaxed);
                let durable = result?;
                if durable {
                    since_flush = 0;
                } else {
                    since_flush += 1;
                }
                if !durable && since_flush >= flush_every {
                    file.flush().await?;
                    since_flush = 0;
                }
            }
        }
    }
    file.flush().await?;
    Ok(())
}

async fn write_pending(
    file: &mut tokio::fs::File,
    pending: PendingRecord,
) -> Result<bool, StorageError> {
    let PendingRecord {
        record,
        durable_ack,
    } = pending;
    let durable = durable_ack.is_some();
    let result = async {
        write_record(file, record).await?;
        if durable {
            file.flush().await?;
            file.sync_data().await?;
        }
        Ok::<(), StorageError>(())
    }
    .await;
    if let Some(ack) = durable_ack {
        let acknowledgement = match &result {
            Ok(()) => Ok(()),
            Err(error) => Err(error.to_string()),
        };
        let _ = ack.send(acknowledgement);
    }
    result.map(|()| durable)
}

async fn write_record(
    file: &mut tokio::fs::File,
    record: StorageRecord,
) -> Result<(), StorageError> {
    let mut line = serde_json::to_vec(&StoredEnvelope {
        schema_version: CURRENT_SCHEMA_VERSION,
        record,
    })?;
    line.push(b'\n');
    file.write_all(&line).await?;
    Ok(())
}

#[cfg(test)]
mod tests {
    use reap_core::{
        ConnId, ControlEvent, FundingSettlement, MarketEvent, OrderEvent, OrderStatus, OrderUpdate,
        Venue,
    };

    use super::*;

    fn raw() -> StorageRecord {
        StorageRecord::Raw {
            account_id: None,
            envelope: RawEnvelope {
                venue: Venue::Okx,
                conn_id: ConnId::new("test"),
                channel: reap_core::Channel::Books,
                symbol: Some("BTC-USDT".to_string()),
                recv_ts_ns: 1,
                raw_hash: 2,
                payload: "{}".to_string(),
            },
        }
    }

    #[tokio::test]
    async fn best_effort_records_drop_when_bounded_queue_is_full() {
        let (sender, _receiver) = mpsc::channel(1);
        let sink = StorageSink {
            sender,
            dropped: Arc::new(AtomicU64::new(0)),
            queued: Arc::new(AtomicUsize::new(0)),
        };
        assert_eq!(
            sink.record(raw()).await.unwrap(),
            StorageWriteOutcome::Queued
        );
        assert_eq!(
            sink.record(raw()).await.unwrap(),
            StorageWriteOutcome::DroppedBestEffort
        );
        assert_eq!(sink.dropped_records(), 1);
    }

    #[test]
    fn critical_record_backpressure_is_fail_stop_not_drop() {
        let (sender, _receiver) = mpsc::channel(1);
        let sink = StorageSink {
            sender,
            dropped: Arc::new(AtomicU64::new(0)),
            queued: Arc::new(AtomicUsize::new(0)),
        };
        sink.try_record(StorageRecord::Intent {
            ts_ms: 1,
            intent: OrderIntent::CancelOrder {
                order_id: "one".to_string(),
                reason: "test".to_string(),
            },
        })
        .unwrap();
        assert!(matches!(
            sink.try_record(StorageRecord::Intent {
                ts_ms: 2,
                intent: OrderIntent::CancelOrder {
                    order_id: "two".to_string(),
                    reason: "test".to_string(),
                },
            }),
            Err(StorageError::Backpressure)
        ));
        assert_eq!(sink.dropped_records(), 0);
    }

    #[test]
    fn recovery_remains_compatible_with_v2_journals() {
        let directory = tempfile::tempdir().unwrap();
        let path = directory.path().join("events.jsonl");
        let line = serde_json::to_string(&StoredEnvelope {
            schema_version: 2,
            record: raw(),
        })
        .unwrap();
        std::fs::write(&path, format!("{line}\n")).unwrap();

        let recovered = recover_jsonl(path).unwrap();

        assert_eq!(recovered.records, 1);
    }

    #[test]
    fn recovery_migrates_v3_unscoped_bootstrap_fill_ids() {
        let directory = tempfile::tempdir().unwrap();
        let path = directory.path().join("events.jsonl");
        let line = r#"{"schema_version":3,"record":{"kind":"bootstrap","data":{"ts_ms":1,"account_id":"main","strategy_name":"chaos","config_fingerprint":"fingerprint","baseline_fill_ids":["legacy-fill"]}}}"#;
        std::fs::write(&path, format!("{line}\n")).unwrap();

        let recovered = recover_jsonl(path).unwrap();

        assert_eq!(recovered.records, 1);
        assert!(
            recovered.baseline_fill_ids["main"].contains(&FillKey::legacy_unscoped("legacy-fill"))
        );
        assert!(
            recovered.baseline_fill_ids["main"]
                .iter()
                .any(|key| key.matches("BTC-USDT", "legacy-fill"))
        );
    }

    #[test]
    fn recovery_migrates_v4_fill_liquidity_to_optional_field() {
        let directory = tempfile::tempdir().unwrap();
        let path = directory.path().join("events.jsonl");
        let line = r#"{"schema_version":4,"record":{"kind":"fill","data":{"ts_ms":1,"account_id":"main","fill_id":"fill-1","order_id":"order-1","symbol":"BTC-USDT","side":"buy","price":100.0,"qty":0.1,"liquidity":"maker"}}}"#;
        std::fs::write(&path, format!("{line}\n")).unwrap();

        let recovered = recover_jsonl(path).unwrap();

        assert_eq!(recovered.records, 1);
        assert_eq!(recovered.fills.len(), 1);
        assert_eq!(recovered.fills[0].liquidity, Some(FillLiquidity::Maker));
    }

    #[test]
    fn recovery_remains_compatible_with_v5_journals() {
        let line = serde_json::to_string(&StoredEnvelope {
            schema_version: 5,
            record: raw(),
        })
        .unwrap();

        let recovered = recover_jsonl_bytes(format!("{line}\n").as_bytes()).unwrap();

        assert_eq!(recovered.records, 1);
    }

    #[test]
    fn recovery_visitor_streams_v6_runtime_session_boundaries() {
        let session = StorageRecord::SessionStart(SessionStartRecord {
            ts_ms: 1_000,
            session_id: "1a2b3c".to_string(),
            account_id: "main".to_string(),
            strategy_name: "iarb2".to_string(),
            config_fingerprint: "a".repeat(64),
            account_identity_sha256: "b".repeat(64),
        });
        let line = serde_json::to_string(&StoredEnvelope {
            schema_version: CURRENT_SCHEMA_VERSION,
            record: session,
        })
        .unwrap();
        let mut observed = None;

        let recovered =
            recover_jsonl_bytes_with_visitor(format!("{line}\n").as_bytes(), |line, record| {
                if let StorageRecord::SessionStart(session) = record {
                    observed = Some((line, session.clone()));
                }
            })
            .unwrap();

        assert_eq!(recovered.records, 1);
        let (line, session) = observed.unwrap();
        assert_eq!(line, 1);
        assert_eq!(session.session_id, "1a2b3c");
        assert_eq!(session.account_id, "main");
        assert_eq!(session.account_identity_sha256, "b".repeat(64));
    }

    #[test]
    fn recovery_visitor_streams_v7_authoritative_account_snapshots() {
        let snapshot = StorageRecord::AccountSnapshot(AccountSnapshotRecord {
            ts_ms: 1_001,
            account_id: "main".to_string(),
            update: AccountUpdate {
                ts_ms: 1_001,
                balances: Vec::new(),
                positions: vec![reap_core::Position {
                    symbol: "BTC-USDT-SWAP".to_string(),
                    qty: -2.0,
                    avg_price: 50_000.0,
                    margin_mode: Some(reap_core::PositionMarginMode::Cross),
                }],
                margins: Vec::new(),
            },
        });
        assert!(snapshot.is_critical());
        let line = serde_json::to_string(&StoredEnvelope {
            schema_version: CURRENT_SCHEMA_VERSION,
            record: snapshot,
        })
        .unwrap();
        let mut observed = None;

        let recovered =
            recover_jsonl_bytes_with_visitor(format!("{line}\n").as_bytes(), |line, record| {
                if let StorageRecord::AccountSnapshot(snapshot) = record {
                    observed = Some((line, snapshot.clone()));
                }
            })
            .unwrap();

        assert_eq!(recovered.records, 1);
        let (line, snapshot) = observed.unwrap();
        assert_eq!(line, 1);
        assert_eq!(snapshot.account_id, "main");
        assert_eq!(snapshot.update.positions[0].qty, -2.0);
        assert_eq!(snapshot.update.positions[0].avg_price, 50_000.0);
    }

    #[test]
    fn recovery_visitor_streams_funding_without_retaining_normalized_traffic() {
        let funding =
            StorageRecord::Normalized(NormalizedEvent::Market(MarketEvent::FundingRate {
                ts_ms: 1_001,
                symbol: "BTC-USDT-SWAP".to_string(),
                rate: 0.0001,
                funding_time_ms: 2_000,
                settlement: Some(FundingSettlement {
                    funding_time_ms: 2_000,
                    rate: 0.0002,
                }),
            }));
        let line = serde_json::to_string(&StoredEnvelope {
            schema_version: CURRENT_SCHEMA_VERSION,
            record: funding,
        })
        .unwrap();
        let bytes = format!("{line}\n");
        let mut settlements = Vec::new();

        let recovered = recover_jsonl_bytes_with_visitor(bytes.as_bytes(), |line, record| {
            if let StorageRecord::Normalized(NormalizedEvent::Market(MarketEvent::FundingRate {
                symbol,
                settlement: Some(settlement),
                ..
            })) = record
            {
                settlements.push((line, symbol.clone(), *settlement));
            }
        })
        .unwrap();

        assert_eq!(recovered.records, 1);
        assert!(recovered.fills.is_empty());
        assert_eq!(settlements.len(), 1);
        assert_eq!(settlements[0].0, 1);
        assert_eq!(settlements[0].1, "BTC-USDT-SWAP");
        assert_eq!(settlements[0].2.funding_time_ms, 2_000);
        assert_eq!(settlements[0].2.rate, 0.0002);
    }

    #[tokio::test]
    async fn writer_persists_all_record_classes_as_jsonl() {
        let directory = tempfile::tempdir().unwrap();
        let path = directory.path().join("events.jsonl");
        let runtime = start_jsonl_storage(StorageConfig {
            path: path.clone(),
            channel_capacity: 16,
            flush_every_records: 1,
        })
        .await
        .unwrap();
        let sink = runtime.sink();
        sink.record(raw()).await.unwrap();
        sink.record(StorageRecord::Normalized(NormalizedEvent::Control(
            ControlEvent {
                ts_ms: 1,
                reason: "test".to_string(),
            },
        )))
        .await
        .unwrap();
        sink.record(StorageRecord::Intent {
            ts_ms: 1,
            intent: OrderIntent::CancelOrder {
                order_id: "order-1".to_string(),
                reason: "test".to_string(),
            },
        })
        .await
        .unwrap();
        sink.record(StorageRecord::Order {
            account_id: None,
            update: OrderUpdate {
                ts_ms: 1,
                order_id: "order-1".to_string(),
                symbol: "BTC-USDT".to_string(),
                side: Side::Buy,
                event: OrderEvent::New,
                status: OrderStatus::Live,
                price: 100.0,
                time_in_force: None,
                qty: 1.0,
                open_qty: 1.0,
                filled_qty: 0.0,
                avg_fill_price: 0.0,
                last_fill_qty: 0.0,
                last_fill_price: 0.0,
                last_fill_liquidity: None,
                last_fill_fee: None,
                reason: "test".to_string(),
            },
        })
        .await
        .unwrap();
        sink.record(StorageRecord::Fill(FillRecord {
            ts_ms: 1,
            account_id: None,
            fill_id: "fill-1".to_string(),
            order_id: "order-1".to_string(),
            symbol: "BTC-USDT".to_string(),
            side: Side::Buy,
            price: 100.0,
            qty: 1.0,
            liquidity: Some(FillLiquidity::Maker),
            fee: None,
        }))
        .await
        .unwrap();
        runtime.shutdown().await.unwrap();

        let text = std::fs::read_to_string(path).unwrap();
        let lines = text.lines().collect::<Vec<_>>();
        assert_eq!(lines.len(), 5);
        assert!(
            lines
                .iter()
                .all(|line| serde_json::from_str::<serde_json::Value>(line).is_ok())
        );
        assert!(text.contains("normalized"));
        assert!(text.contains("intent"));
        assert!(text.contains("order"));
        assert!(text.contains("fill"));
    }

    #[test]
    fn storage_lease_is_exclusive_canonical_and_released_on_drop() {
        let directory = tempfile::tempdir().unwrap();
        let nested = directory.path().join("nested");
        std::fs::create_dir_all(&nested).unwrap();
        let direct = directory.path().join("events.jsonl");
        let alias = nested.join("..").join("events.jsonl");

        let lease = acquire_storage_lease(&direct).unwrap();
        assert_eq!(
            lease.journal_path(),
            directory
                .path()
                .canonicalize()
                .unwrap()
                .join("events.jsonl")
        );
        let second = acquire_storage_lease(&alias).unwrap_err();
        assert!(matches!(second, StorageError::AlreadyLocked { .. }));
        let owner = std::fs::read_to_string(lease.lock_path()).unwrap();
        assert!(owner.contains(&format!("pid={}", std::process::id())));
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            assert_eq!(
                std::fs::metadata(lease.lock_path())
                    .unwrap()
                    .permissions()
                    .mode()
                    & 0o777,
                0o600
            );
        }

        drop(lease);
        acquire_storage_lease(alias).unwrap();
    }

    #[tokio::test]
    async fn storage_runtime_holds_lease_until_shutdown() {
        let directory = tempfile::tempdir().unwrap();
        let path = directory.path().join("events.jsonl");
        let config = StorageConfig {
            path: path.clone(),
            channel_capacity: 4,
            flush_every_records: 1,
        };
        let mut runtime = start_jsonl_storage(config.clone()).await.unwrap();
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            assert_eq!(
                std::fs::metadata(&path).unwrap().permissions().mode() & 0o777,
                0o600
            );
        }

        let second = start_jsonl_storage(config.clone()).await.err().unwrap();
        assert!(matches!(second, StorageError::AlreadyLocked { .. }));

        runtime.stop_writer().await.unwrap();
        let second = acquire_storage_lease(&path).unwrap_err();
        assert!(matches!(second, StorageError::AlreadyLocked { .. }));
        drop(runtime);
        start_jsonl_storage(config)
            .await
            .unwrap()
            .shutdown()
            .await
            .unwrap();
    }

    #[tokio::test]
    async fn storage_rejects_a_lease_for_a_different_journal() {
        let directory = tempfile::tempdir().unwrap();
        let leased_path = directory.path().join("leased.jsonl");
        let configured_path = directory.path().join("configured.jsonl");
        let lease = acquire_storage_lease(&leased_path).unwrap();

        let error = start_jsonl_storage_with_lease(
            StorageConfig {
                path: configured_path,
                channel_capacity: 4,
                flush_every_records: 1,
            },
            lease,
        )
        .await
        .err()
        .unwrap();

        assert!(matches!(error, StorageError::LeaseMismatch { .. }));
        acquire_storage_lease(leased_path).unwrap();
    }

    #[tokio::test]
    async fn durable_record_is_synced_before_acknowledgement() {
        let directory = tempfile::tempdir().unwrap();
        let path = directory.path().join("events.jsonl");
        let runtime = start_jsonl_storage(StorageConfig {
            path: path.clone(),
            channel_capacity: 4,
            flush_every_records: 1_000,
        })
        .await
        .unwrap();
        runtime
            .sink()
            .record_durable(StorageRecord::SafetyLatch(SafetyLatchRecord {
                ts_ms: 1,
                scope: SafetyLatchScope::Global,
                active: true,
                source: SafetyLatchSource::Operator,
                request_id: Some("request-1".to_string()),
                reason: "operator".to_string(),
            }))
            .await
            .unwrap();

        let text = std::fs::read_to_string(&path).unwrap();
        assert!(text.contains("safety_latch"));
        assert!(text.contains("request-1"));
        runtime.shutdown().await.unwrap();
    }

    #[tokio::test]
    async fn recovery_restores_latest_orders_and_fill_identity() {
        let directory = tempfile::tempdir().unwrap();
        let path = directory.path().join("events.jsonl");
        let runtime = start_jsonl_storage(StorageConfig {
            path: path.clone(),
            channel_capacity: 16,
            flush_every_records: 1,
        })
        .await
        .unwrap();
        let sink = runtime.sink();
        sink.record(StorageRecord::Bootstrap(BootstrapRecord {
            ts_ms: 1,
            account_id: "main".to_string(),
            strategy_name: "test".to_string(),
            config_fingerprint: "fingerprint".to_string(),
            baseline_fill_ids: vec![FillKey::new("BTC-USDT", "historical-fill")],
        }))
        .await
        .unwrap();
        sink.record(StorageRecord::OrderAck(OrderAckRecord {
            ts_ms: 1,
            account_id: "main".to_string(),
            operation: OrderOperation::Submit,
            client_order_id: "order-1".to_string(),
            exchange_order_id: Some("exchange-1".to_string()),
            status: OrderAckStatus::Accepted,
            message: "accepted".to_string(),
        }))
        .await
        .unwrap();
        let mut order = OrderUpdate {
            ts_ms: 1,
            order_id: "order-1".to_string(),
            symbol: "BTC-USDT".to_string(),
            side: Side::Buy,
            event: OrderEvent::New,
            status: OrderStatus::Live,
            price: 100.0,
            time_in_force: None,
            qty: 1.0,
            open_qty: 1.0,
            filled_qty: 0.0,
            avg_fill_price: 0.0,
            last_fill_qty: 0.0,
            last_fill_price: 0.0,
            last_fill_liquidity: None,
            last_fill_fee: None,
            reason: "test".to_string(),
        };
        sink.record(StorageRecord::Order {
            account_id: Some("main".to_string()),
            update: order.clone(),
        })
        .await
        .unwrap();
        order.ts_ms = 2;
        order.event = OrderEvent::Cancelled;
        order.status = OrderStatus::Cancelled;
        sink.record(StorageRecord::Order {
            account_id: Some("main".to_string()),
            update: order.clone(),
        })
        .await
        .unwrap();
        sink.record(StorageRecord::Fill(FillRecord {
            ts_ms: 2,
            account_id: Some("main".to_string()),
            fill_id: "fill-1".to_string(),
            order_id: "order-1".to_string(),
            symbol: "BTC-USDT".to_string(),
            side: Side::Buy,
            price: 100.0,
            qty: 0.5,
            liquidity: Some(FillLiquidity::Maker),
            fee: Some(FillFee {
                amount: -0.0005,
                currency: "BTC".to_string(),
            }),
        }))
        .await
        .unwrap();
        runtime.shutdown().await.unwrap();

        let recovered = recover_jsonl(&path).unwrap();
        assert_eq!(
            recovered.latest_orders["order-1"].status,
            OrderStatus::Cancelled
        );
        assert!(
            recovered
                .seen_fill_keys
                .contains(&FillKey::new("BTC-USDT", "fill-1"))
        );
        assert_eq!(
            recovered.fills[0].fee,
            Some(FillFee {
                amount: -0.0005,
                currency: "BTC".to_string(),
            })
        );
        assert!(
            recovered.baseline_fill_ids["main"]
                .contains(&FillKey::new("BTC-USDT", "historical-fill"))
        );
        assert_eq!(recovered.order_bindings["main"]["exchange-1"], "order-1");
        assert_eq!(recovered.last_ts_ms, 2);
    }

    #[test]
    fn recovery_rejects_conflicting_order_ack_bindings() {
        for (client_order_id, exchange_order_id) in
            [("order-1", "exchange-2"), ("order-2", "exchange-1")]
        {
            let directory = tempfile::tempdir().unwrap();
            let path = directory.path().join("events.jsonl");
            let records = [
                StorageRecord::OrderAck(OrderAckRecord {
                    ts_ms: 1,
                    account_id: "main".to_string(),
                    operation: OrderOperation::Submit,
                    client_order_id: "order-1".to_string(),
                    exchange_order_id: Some("exchange-1".to_string()),
                    status: OrderAckStatus::Accepted,
                    message: "accepted".to_string(),
                }),
                StorageRecord::OrderAck(OrderAckRecord {
                    ts_ms: 2,
                    account_id: "main".to_string(),
                    operation: OrderOperation::Submit,
                    client_order_id: client_order_id.to_string(),
                    exchange_order_id: Some(exchange_order_id.to_string()),
                    status: OrderAckStatus::Duplicate,
                    message: "duplicate".to_string(),
                }),
            ];
            let journal = records
                .into_iter()
                .map(|record| {
                    serde_json::to_string(&StoredEnvelope {
                        schema_version: CURRENT_SCHEMA_VERSION,
                        record,
                    })
                    .unwrap()
                })
                .collect::<Vec<_>>()
                .join("\n");
            std::fs::write(&path, format!("{journal}\n")).unwrap();

            let error = recover_jsonl(path).unwrap_err();

            assert!(matches!(&error, StorageError::Corrupt { line: 2, .. }));
            assert!(error.to_string().contains("bound to both"));
        }
    }

    #[tokio::test]
    async fn recovery_reduces_explicit_and_legacy_safety_latches() {
        let directory = tempfile::tempdir().unwrap();
        let path = directory.path().join("events.jsonl");
        let runtime = start_jsonl_storage(StorageConfig {
            path: path.clone(),
            channel_capacity: 16,
            flush_every_records: 1,
        })
        .await
        .unwrap();
        let sink = runtime.sink();
        sink.record(StorageRecord::System(SystemEvent {
            ts_ms: 1,
            kind: SystemEventKind::KillSwitchActivated,
            venue: None,
            account_id: None,
            symbol: None,
            reason: "bounded run completed".to_string(),
        }))
        .await
        .unwrap();
        sink.record(StorageRecord::System(SystemEvent {
            ts_ms: 2,
            kind: SystemEventKind::KillSwitchActivated,
            venue: None,
            account_id: None,
            symbol: None,
            reason: "authenticated operator request legacy: halt".to_string(),
        }))
        .await
        .unwrap();
        sink.record(StorageRecord::SafetyLatch(SafetyLatchRecord {
            ts_ms: 3,
            scope: SafetyLatchScope::Account {
                account_id: "main".to_string(),
            },
            active: true,
            source: SafetyLatchSource::Operator,
            request_id: Some("request-2".to_string()),
            reason: "account exposure".to_string(),
        }))
        .await
        .unwrap();
        sink.record(StorageRecord::SafetyLatch(SafetyLatchRecord {
            ts_ms: 4,
            scope: SafetyLatchScope::Symbol {
                symbol: "BTC-USDT".to_string(),
            },
            active: true,
            source: SafetyLatchSource::Operator,
            request_id: Some("request-3".to_string()),
            reason: "bad book".to_string(),
        }))
        .await
        .unwrap();
        sink.record(StorageRecord::SafetyLatch(SafetyLatchRecord {
            ts_ms: 5,
            scope: SafetyLatchScope::Symbol {
                symbol: "BTC-USDT".to_string(),
            },
            active: false,
            source: SafetyLatchSource::Operator,
            request_id: Some("request-4".to_string()),
            reason: "reviewed".to_string(),
        }))
        .await
        .unwrap();
        runtime.shutdown().await.unwrap();

        let recovered = recover_jsonl(path).unwrap();
        assert_eq!(
            recovered
                .global_safety_latch
                .as_ref()
                .map(|latch| latch.source),
            Some(SafetyLatchSource::LegacySystemEvent)
        );
        assert_eq!(
            recovered.account_safety_latches["main"]
                .request_id
                .as_deref(),
            Some("request-2")
        );
        assert!(recovered.symbol_safety_latches.is_empty());
    }
}
