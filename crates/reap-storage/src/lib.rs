use std::collections::{BTreeMap, HashMap, HashSet};
use std::path::{Path, PathBuf};

use reap_core::{
    AccountUpdate, FillFee, FillKey, FillLiquidity, NormalizedEvent, OrderIntent, OrderUpdate,
    Price, Quantity, RawEnvelope, Side, Symbol, SystemEvent, SystemEventKind, TimeMs,
};
use reap_durable_writer::{
    BoundedSink, DeliveryClass, DurableLease, DurableWriterConfig, DurableWriterRuntime,
    EnqueueError, EnqueueOutcome, JournalCodec, LeaseError, WriterError,
    start_durable_writer_with_lease,
};
use serde::{Deserialize, Serialize};
use thiserror::Error;

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

/// Account-scoped client identity for a regular order whose submit request was
/// proven from the durable journal.
///
/// Construction remains private to recovery so a caller cannot turn an
/// arbitrary client-order ID into ownership authority.
#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct ProvenRegularClientOrderKey {
    account_id: String,
    client_order_id: String,
}

impl ProvenRegularClientOrderKey {
    pub fn account_id(&self) -> &str {
        &self.account_id
    }

    pub fn client_order_id(&self) -> &str {
        &self.client_order_id
    }
}

/// Account-scoped exchange identity bound to a proven regular submit request.
///
/// Construction remains private to recovery for the same reason as
/// [`ProvenRegularClientOrderKey`].
#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct ProvenRegularExchangeOrderKey {
    account_id: String,
    exchange_order_id: String,
}

impl ProvenRegularExchangeOrderKey {
    pub fn account_id(&self) -> &str {
        &self.account_id
    }

    pub fn exchange_order_id(&self) -> &str {
        &self.exchange_order_id
    }
}

/// A well-formed regular submit request recovered before any matching
/// acknowledgement.
///
/// This is an in-memory ownership proof only. It deliberately is not
/// serializable and does not change the journal schema.
#[derive(Debug, PartialEq, Eq)]
pub struct ProvenRegularSubmitRequest {
    account_id: String,
    symbol: Symbol,
    client_order_id: String,
    idempotency_key: String,
}

impl ProvenRegularSubmitRequest {
    pub fn account_id(&self) -> &str {
        &self.account_id
    }

    pub fn symbol(&self) -> &str {
        &self.symbol
    }

    pub fn client_order_id(&self) -> &str {
        &self.client_order_id
    }

    pub fn idempotency_key(&self) -> &str {
        &self.idempotency_key
    }
}

/// An accepted or duplicate regular submit acknowledgement bound to its prior
/// proven request.
///
/// This type is also recovery-only, non-serialized ownership state.
#[derive(Debug, PartialEq, Eq)]
pub struct ProvenRegularOrderBinding {
    request: ProvenRegularSubmitRequest,
    exchange_order_id: String,
}

impl ProvenRegularOrderBinding {
    pub fn account_id(&self) -> &str {
        self.request.account_id()
    }

    pub fn symbol(&self) -> &str {
        self.request.symbol()
    }

    pub fn client_order_id(&self) -> &str {
        self.request.client_order_id()
    }

    pub fn idempotency_key(&self) -> &str {
        self.request.idempotency_key()
    }

    pub fn exchange_order_id(&self) -> &str {
        &self.exchange_order_id
    }

    pub fn request(&self) -> &ProvenRegularSubmitRequest {
        &self.request
    }
}

#[derive(Debug, Default)]
pub struct RecoveredStorage {
    pub latest_orders: HashMap<String, OrderUpdate>,
    /// Legacy acknowledgement-derived bindings retained for compatibility.
    /// New live ownership decisions must use the proven regular indexes below.
    pub order_bindings: HashMap<String, HashMap<String, String>>,
    pub proven_regular_submit_requests:
        BTreeMap<ProvenRegularClientOrderKey, ProvenRegularSubmitRequest>,
    pub proven_regular_order_bindings:
        BTreeMap<ProvenRegularExchangeOrderKey, ProvenRegularOrderBinding>,
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

/// A read-only, process-local view of bounded storage-writer progress.
///
/// `records_outstanding` counts accepted records until their write attempt
/// completes, preserving the historical [`StorageSink::queue_depth`] metric.
/// `queue_depth` counts records currently held by the bounded channel and is
/// therefore never greater than `queue_capacity`. The monotonic timestamp uses
/// a private process-local origin and must not be compared across processes.
/// Its corresponding age is computed against that same origin.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct StorageProgressSnapshot {
    pub records_enqueued: u64,
    pub records_written: u64,
    pub durable_sync_completions: u64,
    pub write_failures: u64,
    pub sync_failures: u64,
    pub dropped_records: u64,
    pub records_outstanding: usize,
    pub queue_capacity: usize,
    pub queue_depth: usize,
    pub queue_high_water: usize,
    pub last_writer_progress_ns: u64,
    pub last_writer_progress_age_ns: u64,
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
    #[error("regular-order authority was already recovered from leased journal {path}")]
    AuthorityAlreadyRecovered { path: PathBuf },
    #[error("durable storage write failed: {0}")]
    Durability(String),
    #[error("storage recovery found an invalid record on line {line}: {message}")]
    Corrupt { line: usize, message: String },
    #[error("storage writer task failed: {0}")]
    Join(#[from] tokio::task::JoinError),
}

#[derive(Clone)]
pub struct StorageSink {
    inner: BoundedSink<StorageRecord>,
}

impl StorageSink {
    pub async fn record(&self, record: StorageRecord) -> Result<StorageWriteOutcome, StorageError> {
        let class = if record.is_critical() {
            DeliveryClass::Critical
        } else {
            DeliveryClass::BestEffort
        };
        self.inner
            .enqueue(record, class)
            .await
            .map(map_enqueue_outcome)
            .map_err(map_enqueue_error)
    }

    pub async fn record_durable(&self, record: StorageRecord) -> Result<(), StorageError> {
        self.inner
            .enqueue_durable(record)
            .await
            .map_err(map_enqueue_error)
    }

    pub fn try_record(&self, record: StorageRecord) -> Result<StorageWriteOutcome, StorageError> {
        let class = if record.is_critical() {
            DeliveryClass::Critical
        } else {
            DeliveryClass::BestEffort
        };
        self.inner
            .try_enqueue(record, class)
            .map(map_enqueue_outcome)
            .map_err(map_enqueue_error)
    }

    pub fn dropped_records(&self) -> u64 {
        self.inner.dropped_records()
    }

    pub fn queue_depth(&self) -> usize {
        self.inner.records_outstanding()
    }

    /// Returns a numeric, observation-only snapshot of storage writer progress.
    pub fn progress_snapshot(&self) -> StorageProgressSnapshot {
        let snapshot = self.inner.progress_snapshot();
        StorageProgressSnapshot {
            records_enqueued: snapshot.records_enqueued,
            records_written: snapshot.records_written,
            durable_sync_completions: snapshot.durable_sync_completions,
            write_failures: snapshot.write_failures,
            sync_failures: snapshot.sync_failures,
            dropped_records: snapshot.dropped_records,
            records_outstanding: snapshot.records_outstanding,
            queue_capacity: snapshot.queue_capacity,
            queue_depth: snapshot.queue_depth,
            queue_high_water: snapshot.queue_high_water,
            last_writer_progress_ns: snapshot.last_writer_progress_ns,
            last_writer_progress_age_ns: snapshot.last_writer_progress_age_ns,
        }
    }
}

pub struct StorageRuntime {
    inner: DurableWriterRuntime<StorageRecord, Schema7Codec>,
}

impl StorageRuntime {
    pub fn sink(&self) -> StorageSink {
        StorageSink {
            inner: self.inner.sink(),
        }
    }

    pub fn request_shutdown(&mut self) {
        self.inner.request_shutdown();
    }

    pub async fn stop_writer(&mut self) -> Result<(), StorageError> {
        self.inner.stop_writer().await.map_err(map_writer_error)
    }

    pub async fn shutdown(mut self) -> Result<(), StorageError> {
        self.stop_writer().await
    }
}

#[derive(Debug)]
pub struct StorageLease {
    inner: DurableLease,
    authority_recovered: bool,
}

impl StorageLease {
    pub fn journal_path(&self) -> &Path {
        self.inner.journal_path()
    }

    pub fn lock_path(&self) -> &Path {
        self.inner.lock_path()
    }
}

pub fn acquire_storage_lease(path: impl AsRef<Path>) -> Result<StorageLease, StorageError> {
    let inner = DurableLease::acquire(path).map_err(map_lease_error)?;
    Ok(StorageLease {
        inner,
        authority_recovered: false,
    })
}

#[derive(Debug, Serialize, Deserialize)]
struct StoredEnvelope {
    schema_version: u16,
    record: StorageRecord,
}

const CURRENT_SCHEMA_VERSION: u16 = 7;

struct Schema7Codec;

#[derive(Serialize)]
struct StoredEnvelopeRef<'a> {
    schema_version: u16,
    record: &'a StorageRecord,
}

impl JournalCodec<StorageRecord> for Schema7Codec {
    type Error = serde_json::Error;

    fn encode(&self, record: &StorageRecord, output: &mut Vec<u8>) -> Result<(), Self::Error> {
        serde_json::to_writer(
            output,
            &StoredEnvelopeRef {
                schema_version: CURRENT_SCHEMA_VERSION,
                record,
            },
        )
    }

    fn durability_error(&self, error: &WriterError<Self::Error>) -> String {
        match error {
            WriterError::Io(error) => format!("storage IO failed: {error}"),
            WriterError::Codec(error) => format!("storage serialization failed: {error}"),
            other => other.to_string(),
        }
    }
}

fn map_enqueue_outcome(outcome: EnqueueOutcome) -> StorageWriteOutcome {
    match outcome {
        EnqueueOutcome::Queued => StorageWriteOutcome::Queued,
        EnqueueOutcome::DroppedBestEffort => StorageWriteOutcome::DroppedBestEffort,
    }
}

fn map_enqueue_error(error: EnqueueError) -> StorageError {
    match error {
        EnqueueError::Closed => StorageError::Closed,
        EnqueueError::Full => StorageError::Backpressure,
        EnqueueError::Durability(message) => StorageError::Durability(message),
    }
}

fn map_lease_error(error: LeaseError) -> StorageError {
    match error {
        LeaseError::Io(error) => StorageError::Io(error),
        LeaseError::InvalidPath { path, message } => StorageError::InvalidPath { path, message },
        LeaseError::AlreadyLocked { path, lock_path } => {
            StorageError::AlreadyLocked { path, lock_path }
        }
    }
}

fn map_writer_error(error: WriterError<serde_json::Error>) -> StorageError {
    match error {
        WriterError::Lease(error) => map_lease_error(error),
        WriterError::Io(error) => StorageError::Io(error),
        WriterError::Codec(error) => StorageError::Serialization(error),
        WriterError::LeaseMismatch {
            config_path,
            lease_path,
        } => StorageError::LeaseMismatch {
            config_path,
            lease_path,
        },
        WriterError::Join(error) => StorageError::Join(error),
    }
}

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

/// Recovers live regular-order authority from the journal protected by an
/// exclusively borrowed storage lease.
///
/// Ordinary path/byte recovery intentionally strips authority-bearing proofs.
/// Live startup must already own the journal lease and cannot select different
/// bytes from those protected by that lease.
pub fn recover_leased_jsonl(lease: &mut StorageLease) -> Result<RecoveredStorage, StorageError> {
    if std::mem::replace(&mut lease.authority_recovered, true) {
        return Err(StorageError::AuthorityAlreadyRecovered {
            path: lease.journal_path().to_path_buf(),
        });
    }
    let bytes = match std::fs::read(lease.journal_path()) {
        Ok(bytes) => bytes,
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => {
            return Ok(RecoveredStorage::default());
        }
        Err(error) => return Err(error.into()),
    };
    recover_jsonl_bytes_with_visitor_inner(&bytes, |_, _| {}, true)
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
    visitor: F,
) -> Result<RecoveredStorage, StorageError>
where
    F: FnMut(u64, &StorageRecord),
{
    recover_jsonl_bytes_with_visitor_inner(bytes, visitor, false)
}

fn recover_jsonl_bytes_with_visitor_inner<F>(
    bytes: &[u8],
    mut visitor: F,
    retain_regular_authority: bool,
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
            StorageRecord::OrderRequest(request) => {
                apply_proven_regular_submit_request(&mut recovered, &request, index + 1)?;
            }
            StorageRecord::OrderAck(ack) => {
                apply_legacy_order_binding(&mut recovered, &ack, index + 1)?;
                apply_proven_regular_submit_ack(&mut recovered, &ack, index + 1)?;
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
    if !retain_regular_authority {
        recovered.proven_regular_submit_requests.clear();
        recovered.proven_regular_order_bindings.clear();
    }
    Ok(recovered)
}

fn nonempty_recovered_field(value: &str) -> bool {
    !value.trim().is_empty()
}

fn valid_recovered_order_id(value: &str) -> bool {
    nonempty_recovered_field(value) && value != "0"
}

fn apply_proven_regular_submit_request(
    recovered: &mut RecoveredStorage,
    request: &OrderRequestRecord,
    line: usize,
) -> Result<(), StorageError> {
    let (Some(idempotency_key), Some(client_order_id)) =
        (&request.idempotency_key, &request.client_order_id)
    else {
        return Ok(());
    };
    if !matches!(request.operation, OrderOperation::Submit)
        || !nonempty_recovered_field(&request.account_id)
        || !nonempty_recovered_field(&request.symbol)
        || !valid_recovered_order_id(client_order_id)
        || !nonempty_recovered_field(idempotency_key)
        || request.exchange_order_id.is_some()
    {
        return Ok(());
    }

    let key = ProvenRegularClientOrderKey {
        account_id: request.account_id.clone(),
        client_order_id: client_order_id.clone(),
    };
    let proof = ProvenRegularSubmitRequest {
        account_id: request.account_id.clone(),
        symbol: request.symbol.clone(),
        client_order_id: client_order_id.clone(),
        idempotency_key: idempotency_key.clone(),
    };
    if let Some(existing) = recovered.proven_regular_submit_requests.get(&key) {
        if existing != &proof {
            return Err(StorageError::Corrupt {
                line,
                message: format!(
                    "account {} client order {} has conflicting proven regular submit requests",
                    request.account_id, client_order_id
                ),
            });
        }
        return Ok(());
    }
    recovered.proven_regular_submit_requests.insert(key, proof);
    Ok(())
}

fn apply_legacy_order_binding(
    recovered: &mut RecoveredStorage,
    ack: &OrderAckRecord,
    line: usize,
) -> Result<(), StorageError> {
    let Some(exchange_order_id) = &ack.exchange_order_id else {
        return Ok(());
    };
    if ack.client_order_id.is_empty()
        || ack.client_order_id == "0"
        || exchange_order_id.is_empty()
        || exchange_order_id == "0"
    {
        return Ok(());
    }
    let bindings = recovered
        .order_bindings
        .entry(ack.account_id.clone())
        .or_default();
    if let Some(existing_client_order_id) = bindings.get(exchange_order_id)
        && existing_client_order_id != &ack.client_order_id
    {
        return Err(StorageError::Corrupt {
            line,
            message: format!(
                "account {} exchange order {} is bound to both client orders {} and {}",
                ack.account_id, exchange_order_id, existing_client_order_id, ack.client_order_id
            ),
        });
    }
    if let Some(existing_exchange_order_id) =
        bindings
            .iter()
            .find_map(|(existing_exchange_order_id, existing_client_order_id)| {
                (existing_client_order_id == &ack.client_order_id
                    && existing_exchange_order_id != exchange_order_id)
                    .then_some(existing_exchange_order_id)
            })
    {
        return Err(StorageError::Corrupt {
            line,
            message: format!(
                "account {} client order {} is bound to both exchange orders {} and {}",
                ack.account_id, ack.client_order_id, existing_exchange_order_id, exchange_order_id
            ),
        });
    }
    bindings.insert(exchange_order_id.clone(), ack.client_order_id.clone());
    Ok(())
}

fn apply_proven_regular_submit_ack(
    recovered: &mut RecoveredStorage,
    ack: &OrderAckRecord,
    line: usize,
) -> Result<(), StorageError> {
    if !matches!(ack.operation, OrderOperation::Submit)
        || !nonempty_recovered_field(&ack.account_id)
        || !valid_recovered_order_id(&ack.client_order_id)
    {
        return Ok(());
    }
    let client_key = ProvenRegularClientOrderKey {
        account_id: ack.account_id.clone(),
        client_order_id: ack.client_order_id.clone(),
    };
    if matches!(ack.status, OrderAckStatus::Rejected) {
        recovered.proven_regular_submit_requests.remove(&client_key);
        recovered
            .proven_regular_order_bindings
            .retain(|_, binding| {
                binding.account_id() != ack.account_id
                    || binding.client_order_id() != ack.client_order_id
            });
        return Ok(());
    }
    if !matches!(
        ack.status,
        OrderAckStatus::Accepted | OrderAckStatus::Duplicate
    ) {
        return Ok(());
    }
    let Some(exchange_order_id) = &ack.exchange_order_id else {
        return Ok(());
    };
    if !valid_recovered_order_id(exchange_order_id) {
        return Ok(());
    }
    let Some(request) = recovered.proven_regular_submit_requests.get(&client_key) else {
        return Ok(());
    };
    let exchange_key = ProvenRegularExchangeOrderKey {
        account_id: ack.account_id.clone(),
        exchange_order_id: exchange_order_id.clone(),
    };
    let binding = ProvenRegularOrderBinding {
        request: ProvenRegularSubmitRequest {
            account_id: request.account_id.clone(),
            symbol: request.symbol.clone(),
            client_order_id: request.client_order_id.clone(),
            idempotency_key: request.idempotency_key.clone(),
        },
        exchange_order_id: exchange_order_id.clone(),
    };
    if let Some(existing) = recovered.proven_regular_order_bindings.get(&exchange_key) {
        if existing != &binding {
            return Err(StorageError::Corrupt {
                line,
                message: format!(
                    "account {} exchange order {} has conflicting proven regular bindings",
                    ack.account_id, exchange_order_id
                ),
            });
        }
        return Ok(());
    }
    if let Some(existing) = recovered
        .proven_regular_order_bindings
        .values()
        .find(|existing| {
            existing.account_id() == ack.account_id
                && existing.client_order_id() == ack.client_order_id
                && existing.exchange_order_id() != exchange_order_id
        })
    {
        return Err(StorageError::Corrupt {
            line,
            message: format!(
                "account {} proven client order {} is bound to both exchange orders {} and {}",
                ack.account_id,
                ack.client_order_id,
                existing.exchange_order_id(),
                exchange_order_id
            ),
        });
    }
    recovered
        .proven_regular_order_bindings
        .insert(exchange_key, binding);
    Ok(())
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
    config: StorageConfig,
    lease: StorageLease,
) -> Result<StorageRuntime, StorageError> {
    let StorageLease { inner, .. } = lease;
    let inner = start_durable_writer_with_lease(
        DurableWriterConfig {
            path: config.path,
            channel_capacity: config.channel_capacity,
            flush_every_records: config.flush_every_records,
        },
        inner,
        Schema7Codec,
    )
    .await
    .map_err(map_writer_error)?;
    Ok(StorageRuntime { inner })
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

    #[test]
    fn schema_seven_codec_preserves_the_frozen_raw_envelope_bytes() {
        let mut encoded = Vec::new();
        Schema7Codec.encode(&raw(), &mut encoded).unwrap();
        assert_eq!(
            encoded,
            br#"{"schema_version":7,"record":{"kind":"raw","data":{"account_id":null,"envelope":{"venue":"okx","conn_id":"test","channel":"books","symbol":"BTC-USDT","recv_ts_ns":1,"raw_hash":2,"payload":"{}"}}}}"#
        );
    }

    #[test]
    fn chaos_recovery_rejects_the_separate_pm_tuple_family() {
        let pm_header = br#"["reap-pm-mutation-journal",1,"0000000000000000000000000000000000000000000000000000000000000000",0,{"kind":"header","body":{}}]
"#;
        assert!(matches!(
            recover_jsonl_bytes(pm_header),
            Err(StorageError::Corrupt { line: 1, .. })
        ));
    }

    fn journal_bytes(records: impl IntoIterator<Item = StorageRecord>) -> Vec<u8> {
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
        format!("{journal}\n").into_bytes()
    }

    fn recover_records(
        records: impl IntoIterator<Item = StorageRecord>,
    ) -> Result<RecoveredStorage, StorageError> {
        let directory = tempfile::tempdir().unwrap();
        let path = directory.path().join("leased-recovery.jsonl");
        std::fs::write(&path, journal_bytes(records)).unwrap();
        let mut lease = acquire_storage_lease(&path)?;
        recover_leased_jsonl(&mut lease)
    }

    fn regular_submit_request(
        account_id: &str,
        symbol: &str,
        client_order_id: &str,
        idempotency_key: &str,
    ) -> StorageRecord {
        StorageRecord::OrderRequest(OrderRequestRecord {
            ts_ms: 1,
            account_id: account_id.to_string(),
            operation: OrderOperation::Submit,
            idempotency_key: Some(idempotency_key.to_string()),
            client_order_id: Some(client_order_id.to_string()),
            exchange_order_id: None,
            symbol: symbol.to_string(),
        })
    }

    fn regular_submit_ack(
        account_id: &str,
        client_order_id: &str,
        exchange_order_id: Option<&str>,
        status: OrderAckStatus,
    ) -> StorageRecord {
        StorageRecord::OrderAck(OrderAckRecord {
            ts_ms: 2,
            account_id: account_id.to_string(),
            operation: OrderOperation::Submit,
            client_order_id: client_order_id.to_string(),
            exchange_order_id: exchange_order_id.map(str::to_string),
            status,
            message: "test acknowledgement".to_string(),
        })
    }

    fn private_order(account_id: &str, client_order_id: &str, symbol: &str) -> StorageRecord {
        StorageRecord::Order {
            account_id: Some(account_id.to_string()),
            update: OrderUpdate {
                ts_ms: 2,
                order_id: client_order_id.to_string(),
                symbol: symbol.to_string(),
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
                reason: "private order update".to_string(),
            },
        }
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
    async fn storage_progress_counts_enqueue_write_and_durable_sync() {
        let directory = tempfile::tempdir().unwrap();
        let path = directory.path().join("events.jsonl");
        let runtime = start_jsonl_storage(StorageConfig {
            path,
            channel_capacity: 2,
            flush_every_records: 1_000,
        })
        .await
        .unwrap();
        let sink = runtime.sink();

        let initial = sink.progress_snapshot();
        assert_eq!(initial.queue_capacity, 2);
        assert_eq!(initial.queue_depth, 0);
        assert_eq!(initial.queue_high_water, 0);
        assert_eq!(initial.records_enqueued, 0);
        assert_eq!(initial.records_written, 0);
        assert_eq!(initial.durable_sync_completions, 0);
        assert_eq!(initial.write_failures, 0);
        assert_eq!(initial.sync_failures, 0);
        assert_eq!(initial.records_outstanding, 0);
        assert_eq!(initial.last_writer_progress_ns, 0);
        assert_eq!(initial.last_writer_progress_age_ns, 0);

        sink.record_durable(StorageRecord::SafetyLatch(SafetyLatchRecord {
            ts_ms: 1,
            scope: SafetyLatchScope::Global,
            active: true,
            source: SafetyLatchSource::Operator,
            request_id: Some("progress-test".to_string()),
            reason: "operator".to_string(),
        }))
        .await
        .unwrap();

        let progress = sink.progress_snapshot();
        assert_eq!(progress.queue_capacity, 2);
        assert_eq!(progress.queue_depth, 0);
        assert_eq!(progress.queue_high_water, 1);
        assert_eq!(progress.records_enqueued, 1);
        assert_eq!(progress.records_written, 1);
        assert_eq!(progress.durable_sync_completions, 1);
        assert_eq!(progress.write_failures, 0);
        assert_eq!(progress.sync_failures, 0);
        assert_eq!(progress.records_outstanding, 0);
        assert!(progress.last_writer_progress_ns > 0);
        assert!(progress.last_writer_progress_age_ns < 1_000_000_000);

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
    fn ordinary_recovery_cannot_return_regular_authority_but_a_lease_can_once() {
        let records = [
            regular_submit_request("main", "BTC-USDT", "client-1", "idem-1"),
            regular_submit_ack(
                "main",
                "client-1",
                Some("exchange-1"),
                OrderAckStatus::Accepted,
            ),
        ];
        let bytes = journal_bytes(records.clone());

        let from_bytes = recover_jsonl_bytes(&bytes).unwrap();
        assert!(from_bytes.proven_regular_submit_requests.is_empty());
        assert!(from_bytes.proven_regular_order_bindings.is_empty());

        let directory = tempfile::tempdir().unwrap();
        let path = directory.path().join("authority.jsonl");
        std::fs::write(&path, &bytes).unwrap();
        let from_path = recover_jsonl(&path).unwrap();
        assert!(from_path.proven_regular_submit_requests.is_empty());
        assert!(from_path.proven_regular_order_bindings.is_empty());

        let mut lease = acquire_storage_lease(&path).unwrap();
        let leased = recover_leased_jsonl(&mut lease).unwrap();
        assert_eq!(leased.proven_regular_submit_requests.len(), 1);
        assert_eq!(leased.proven_regular_order_bindings.len(), 1);
        assert!(matches!(
            recover_leased_jsonl(&mut lease),
            Err(StorageError::AuthorityAlreadyRecovered { path: repeated }) if repeated == path
        ));
    }

    #[test]
    fn recovery_proves_regular_requests_before_selected_submit_acknowledgements() {
        let recovered = recover_records([
            regular_submit_request("pending", "BTC-USDT", "client-p", "idem-p"),
            regular_submit_ack(
                "pending",
                "client-p",
                None,
                OrderAckStatus::PendingReconciliation,
            ),
            regular_submit_request("ambiguous", "ETH-USDT", "client-a", "idem-a"),
            regular_submit_ack("ambiguous", "client-a", None, OrderAckStatus::Ambiguous),
            regular_submit_request("accepted", "SOL-USDT", "client-ok", "idem-ok"),
            regular_submit_ack(
                "accepted",
                "client-ok",
                Some("exchange-ok"),
                OrderAckStatus::Accepted,
            ),
            regular_submit_request("duplicate", "XRP-USDT", "client-dup", "idem-dup"),
            regular_submit_ack(
                "duplicate",
                "client-dup",
                Some("exchange-dup"),
                OrderAckStatus::Duplicate,
            ),
        ])
        .unwrap();

        assert_eq!(recovered.proven_regular_submit_requests.len(), 4);
        assert_eq!(recovered.proven_regular_order_bindings.len(), 2);
        let requests = recovered
            .proven_regular_submit_requests
            .iter()
            .map(|(key, request)| {
                (
                    key.account_id(),
                    key.client_order_id(),
                    request.account_id(),
                    request.symbol(),
                    request.client_order_id(),
                    request.idempotency_key(),
                )
            })
            .collect::<Vec<_>>();
        assert!(requests.contains(&(
            "pending", "client-p", "pending", "BTC-USDT", "client-p", "idem-p"
        )));
        let bindings = recovered
            .proven_regular_order_bindings
            .iter()
            .map(|(key, binding)| {
                (
                    key.account_id(),
                    key.exchange_order_id(),
                    binding.account_id(),
                    binding.symbol(),
                    binding.client_order_id(),
                    binding.idempotency_key(),
                    binding.exchange_order_id(),
                    binding.request().client_order_id(),
                )
            })
            .collect::<Vec<_>>();
        assert!(bindings.contains(&(
            "accepted",
            "exchange-ok",
            "accepted",
            "SOL-USDT",
            "client-ok",
            "idem-ok",
            "exchange-ok",
            "client-ok"
        )));
        assert!(bindings.contains(&(
            "duplicate",
            "exchange-dup",
            "duplicate",
            "XRP-USDT",
            "client-dup",
            "idem-dup",
            "exchange-dup",
            "client-dup"
        )));
    }

    #[test]
    fn recovery_does_not_backfill_binding_from_ack_before_request() {
        let recovered = recover_records([
            regular_submit_ack(
                "main",
                "client-1",
                Some("exchange-1"),
                OrderAckStatus::Accepted,
            ),
            regular_submit_request("main", "BTC-USDT", "client-1", "idem-1"),
        ])
        .unwrap();

        assert_eq!(recovered.proven_regular_submit_requests.len(), 1);
        assert!(recovered.proven_regular_order_bindings.is_empty());
        assert_eq!(recovered.order_bindings["main"]["exchange-1"], "client-1");
    }

    #[test]
    fn recovery_rejected_submit_revokes_proven_regular_ownership() {
        let recovered = recover_records([
            regular_submit_request("main", "BTC-USDT", "client-1", "idem-1"),
            regular_submit_ack(
                "main",
                "client-1",
                Some("exchange-1"),
                OrderAckStatus::Accepted,
            ),
            regular_submit_ack("main", "client-1", None, OrderAckStatus::Rejected),
        ])
        .unwrap();

        assert!(recovered.proven_regular_submit_requests.is_empty());
        assert!(recovered.proven_regular_order_bindings.is_empty());
        assert_eq!(recovered.order_bindings["main"]["exchange-1"], "client-1");
    }

    #[test]
    fn recovery_does_not_infer_regular_ownership_from_untrusted_records() {
        let malformed_requests = [
            OrderRequestRecord {
                ts_ms: 1,
                account_id: String::new(),
                operation: OrderOperation::Submit,
                idempotency_key: Some("idem-empty-account".to_string()),
                client_order_id: Some("client-empty-account".to_string()),
                exchange_order_id: None,
                symbol: "BTC-USDT".to_string(),
            },
            OrderRequestRecord {
                ts_ms: 1,
                account_id: "main".to_string(),
                operation: OrderOperation::Submit,
                idempotency_key: Some("idem-empty-symbol".to_string()),
                client_order_id: Some("client-empty-symbol".to_string()),
                exchange_order_id: None,
                symbol: "  ".to_string(),
            },
            OrderRequestRecord {
                ts_ms: 1,
                account_id: "main".to_string(),
                operation: OrderOperation::Submit,
                idempotency_key: Some("idem-empty-client".to_string()),
                client_order_id: Some(String::new()),
                exchange_order_id: None,
                symbol: "BTC-USDT".to_string(),
            },
            OrderRequestRecord {
                ts_ms: 1,
                account_id: "main".to_string(),
                operation: OrderOperation::Submit,
                idempotency_key: Some(String::new()),
                client_order_id: Some("client-empty-idempotency".to_string()),
                exchange_order_id: None,
                symbol: "BTC-USDT".to_string(),
            },
            OrderRequestRecord {
                ts_ms: 1,
                account_id: "main".to_string(),
                operation: OrderOperation::Submit,
                idempotency_key: Some("idem-prebound".to_string()),
                client_order_id: Some("client-prebound".to_string()),
                exchange_order_id: Some("exchange-prebound".to_string()),
                symbol: "BTC-USDT".to_string(),
            },
        ];
        let mut records = malformed_requests.map(StorageRecord::OrderRequest).to_vec();
        records.extend([
            regular_submit_ack(
                "ack-only",
                "client-ack-only",
                Some("exchange-ack-only"),
                OrderAckStatus::Accepted,
            ),
            StorageRecord::OrderRequest(OrderRequestRecord {
                ts_ms: 1,
                account_id: "cancel".to_string(),
                operation: OrderOperation::Cancel,
                idempotency_key: Some("idem-cancel".to_string()),
                client_order_id: Some("client-cancel".to_string()),
                exchange_order_id: None,
                symbol: "BTC-USDT".to_string(),
            }),
            StorageRecord::OrderAck(OrderAckRecord {
                ts_ms: 2,
                account_id: "cancel".to_string(),
                operation: OrderOperation::Cancel,
                client_order_id: "client-cancel".to_string(),
                exchange_order_id: Some("exchange-cancel".to_string()),
                status: OrderAckStatus::Accepted,
                message: "cancel accepted".to_string(),
            }),
            private_order("private", "client-private", "BTC-USDT"),
            regular_submit_ack(
                "private",
                "client-private",
                Some("exchange-private"),
                OrderAckStatus::Accepted,
            ),
            regular_submit_request("rejected", "BTC-USDT", "client-r", "idem-r"),
            regular_submit_ack(
                "rejected",
                "client-r",
                Some("exchange-r"),
                OrderAckStatus::Rejected,
            ),
        ]);

        let recovered = recover_records(records).unwrap();

        assert!(recovered.proven_regular_submit_requests.is_empty());
        assert!(recovered.proven_regular_order_bindings.is_empty());
        assert!(recovered.order_bindings.contains_key("ack-only"));
        assert!(recovered.order_bindings.contains_key("cancel"));
        assert!(recovered.order_bindings.contains_key("private"));
        assert!(recovered.order_bindings.contains_key("rejected"));
        assert!(recovered.latest_orders.contains_key("client-private"));
    }

    #[test]
    fn recovery_rejects_conflicting_proven_regular_submit_requests() {
        for conflicting in [
            regular_submit_request("main", "ETH-USDT", "client-1", "idem-1"),
            regular_submit_request("main", "BTC-USDT", "client-1", "idem-2"),
        ] {
            let error = recover_records([
                regular_submit_request("main", "BTC-USDT", "client-1", "idem-1"),
                conflicting,
            ])
            .unwrap_err();

            assert!(matches!(&error, StorageError::Corrupt { line: 2, .. }));
            assert!(error.to_string().contains("conflict") || error.to_string().contains("both"));
        }
    }

    #[test]
    fn proven_regular_indexes_have_deterministic_key_order() {
        let account_a = [
            regular_submit_request("account-a", "BTC-USDT", "client-a", "idem-a"),
            regular_submit_ack(
                "account-a",
                "client-a",
                Some("exchange-z"),
                OrderAckStatus::Accepted,
            ),
        ];
        let account_z = [
            regular_submit_request("account-z", "ETH-USDT", "client-z", "idem-z"),
            regular_submit_ack(
                "account-z",
                "client-z",
                Some("exchange-a"),
                OrderAckStatus::Duplicate,
            ),
        ];
        let left = recover_records(account_z.clone().into_iter().chain(account_a.clone())).unwrap();
        let right = recover_records(account_a.into_iter().chain(account_z)).unwrap();

        assert_eq!(
            left.proven_regular_submit_requests,
            right.proven_regular_submit_requests
        );
        assert_eq!(
            left.proven_regular_order_bindings,
            right.proven_regular_order_bindings
        );
        assert_eq!(
            left.proven_regular_submit_requests
                .keys()
                .map(ProvenRegularClientOrderKey::account_id)
                .collect::<Vec<_>>(),
            ["account-a", "account-z"]
        );
        assert_eq!(
            left.proven_regular_order_bindings
                .keys()
                .map(ProvenRegularExchangeOrderKey::account_id)
                .collect::<Vec<_>>(),
            ["account-a", "account-z"]
        );
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
