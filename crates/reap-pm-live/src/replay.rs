use std::io::Write;
use std::mem::size_of;
use std::path::Path;

use reap_capture_framing::{
    BoundedJsonlFrameError, digest_hex, jsonl_payload, measure_jsonl_frame_bounded,
    scan_jsonl_file_bounded_total,
};
use reap_okx_public_source::{
    OkxPublicSession, OkxPublicSessionError, OkxPublicSessionEvent, OkxPublicSessionFault,
};
use reap_pm_core::{PmBookSide, PmBookUpdate};
use reap_pm_state::{
    PmBookBatchEvidence, PmBookCounters, PmBookReducer, PmBookTransition, PmDomainFingerprint,
    PmExternalBookFault, PmMetadataContract, PmMetadataDrift, PmMetadataFingerprint,
    PmMetadataObservation, PmPublicReadinessReason,
};
use reap_polymarket_adapter::{
    PmPublicHeartbeatAction, PmPublicRole, PmPublicRoleError, PmPublicSession,
    PmPublicSessionError, PmPublicSessionFault,
};
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use thiserror::Error;

use crate::capture::{
    MAX_PM_PUBLIC_CAPTURE_ENCODED_BYTES, MAX_PM_PUBLIC_CAPTURE_FRAME_BYTES,
    MAX_PM_PUBLIC_CAPTURE_RAW_FRAMES, MAX_PM_PUBLIC_CAPTURE_RECORDS, OkxCaptureDisconnectReason,
    OkxCaptureLifecycle, PmCaptureDisconnectReason, PmCaptureHeader, PmCaptureLifecycle,
    PmCaptureVerification, PmPublicCaptureRecord, PmRawPublicFrame, verify_pm_public_capture,
};

const MAX_PM_REPLAY_PROJECTION_BYTES: usize = 16 * 1024 * 1024;
const MAX_PM_REPLAY_FIXED_BYTES: usize = 64 * 1024;
const MAX_PM_REPLAY_LOGICAL_EVENT_BYTES: usize =
    MAX_PM_REPLAY_PROJECTION_BYTES - MAX_PM_REPLAY_FIXED_BYTES;
const MAX_PM_REPLAY_LOGICAL_EVENTS: usize =
    MAX_PM_PUBLIC_CAPTURE_RAW_FRAMES as usize * 64 + MAX_PM_PUBLIC_CAPTURE_RECORDS as usize;
const PM_REPLAY_EVENT_RESERVE_CHUNK: usize = 1_024;

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "event", rename_all = "snake_case", deny_unknown_fields)]
pub enum PmReplayLogicalEvent {
    ConnectionStarted {
        epoch: u64,
    },
    SubscriptionSent {
        epoch: u64,
    },
    HeartbeatPingSent {
        epoch: u64,
    },
    HeartbeatPong {
        epoch: u64,
        monotonic_receive_ns: u64,
    },
    SnapshotCommitted {
        epoch: u64,
        snapshot_revision: u64,
        local_ingress_sequence: u64,
        venue_timestamp_ns: Option<u64>,
        levels: u16,
        best_bid_units: u32,
        best_ask_units: u32,
    },
    DeltaBatchCommitted {
        epoch: u64,
        snapshot_revision: u64,
        local_ingress_sequence: u64,
        venue_timestamp_ns: Option<u64>,
        changes: u16,
        venue_change_hashes_present: u16,
        ordered_change_hashes_sha256: String,
        best_bid_units: u32,
        best_ask_units: u32,
    },
    TopConfirmed {
        epoch: u64,
        snapshot_revision: u64,
        local_ingress_sequence: u64,
        venue_timestamp_ns: Option<u64>,
        best_bid_units: u32,
        best_ask_units: u32,
    },
    PublicTradeIgnored {
        epoch: u64,
    },
    Disconnected {
        epoch: u64,
        reason: PmCaptureDisconnectReason,
    },
    ReconnectScheduled {
        prior_epoch: u64,
        next_epoch: u64,
        delay_ns: u64,
    },
    FreshnessConfirmed {
        epoch: u64,
        snapshot_revision: u64,
        monotonic_ns: u64,
    },
    FreshnessInvalidated {
        epoch: u64,
        snapshot_revision: u64,
        monotonic_ns: u64,
        reason: PmReplayFreshnessInvalidation,
    },
    OkxSubscriptionAcknowledged {
        epoch: u64,
    },
    OkxConnectionStarted {
        epoch: u64,
    },
    OkxSubscriptionSent {
        epoch: u64,
    },
    OkxDisconnected {
        epoch: u64,
        reason: OkxCaptureDisconnectReason,
    },
    OkxReconnectScheduled {
        prior_epoch: u64,
        next_epoch: u64,
        delay_ns: u64,
    },
    OkxHeartbeat {
        epoch: u64,
    },
    OkxControl {
        epoch: u64,
    },
    OkxReference {
        epoch: u64,
        instrument: String,
        index_price_lexeme: String,
        venue_timestamp_ms: u64,
        wall_receive_ns: u64,
        monotonic_receive_ns: u64,
        raw_hash: u64,
    },
    TickSizeInvalidated {
        epoch: u64,
        snapshot_revision: u64,
        local_ingress_sequence: u64,
        old_tick_units: u32,
        new_tick_units: u32,
    },
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum PmReplayFreshnessInvalidation {
    MetadataStale,
    BookStale,
}

#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct PmReplayCounters {
    pub capture_records: u64,
    pub raw_public_frames: u64,
    pub okx_raw_public_frames: u64,
    pub lifecycle_records: u64,
    pub okx_lifecycle_records: u64,
    pub freshness_timers: u64,
    pub normalized_book_batches: u64,
    pub public_trades_ignored: u64,
    pub heartbeat_pongs: u64,
    pub snapshots_committed: u64,
    pub snapshot_levels_committed: u64,
    pub resync_snapshots: u64,
    pub delta_batches_committed: u64,
    pub delta_changes_committed: u64,
    pub delta_top_checks_confirmed: u64,
    pub top_checks_confirmed: u64,
    pub freshness_checks: u64,
    pub freshness_confirmed: u64,
    pub stale_invalidations: u64,
    pub unavailable_transitions: u64,
    pub external_faults: u64,
    pub disconnects: u64,
    pub heartbeat_timeouts: u64,
    pub backlog_aged_faults: u64,
    pub gaps: u64,
    pub overflows: u64,
    pub reconnects: u64,
    pub invalidations: u64,
    pub integrity_batches_coalesced: u64,
    pub okx_subscription_acknowledgements: u64,
    pub okx_references: u64,
    pub okx_disconnects: u64,
    pub okx_reconnects: u64,
    pub tick_size_invalidations: u64,
    pub projection_event_capacity_bytes: u64,
    pub projection_payload_capacity_bytes: u64,
    pub projection_reserved_capacity_bytes: u64,
    pub metadata_refresh_required: bool,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct PmReplayProjection {
    schema_version: u16,
    product: String,
    capture_artifact_sha256: String,
    capture_verification_sha256: String,
    logical_events: Vec<PmReplayLogicalEvent>,
    counters: PmReplayCounters,
    projection_sha256: String,
    production_order_entry_authorized: bool,
}

impl PmReplayProjection {
    #[must_use]
    pub fn logical_events(&self) -> &[PmReplayLogicalEvent] {
        &self.logical_events
    }

    #[must_use]
    pub const fn counters(&self) -> PmReplayCounters {
        self.counters
    }

    #[must_use]
    pub fn projection_sha256(&self) -> &str {
        &self.projection_sha256
    }

    pub fn canonical_bytes(&self) -> Result<Vec<u8>, PmReplayError> {
        canonical_json_bytes_bounded(self, MAX_PM_REPLAY_PROJECTION_BYTES)
    }
}

#[derive(Debug, Error)]
pub enum PmReplayError {
    #[error(transparent)]
    Capture(#[from] crate::capture::PmCaptureVerifyError),
    #[error(transparent)]
    Framing(#[from] reap_capture_framing::JsonlVerifyError),
    #[error(transparent)]
    Session(#[from] PmPublicSessionError),
    #[error(transparent)]
    Role(#[from] PmPublicRoleError),
    #[error(transparent)]
    OkxSession(#[from] OkxPublicSessionError),
    #[error("PM metadata replay contract is invalid: {0}")]
    MetadataContract(#[from] reap_pm_state::PmMetadataContractError),
    #[error("PM state reducer rejected replay input: {0}")]
    Reducer(#[from] PmPublicReadinessReason),
    #[error("PM replay capture record could not be decoded")]
    MalformedCaptureRecord,
    #[error("PM capture changed between verification and replay")]
    CaptureChangedBeforeReplay,
    #[error("PM replay header differs from the verified expected header")]
    ReplayHeaderMismatch,
    #[error("captured OKX public payload is not UTF-8 text")]
    OkxRawNotUtf8,
    #[error("PM replay session epoch does not match capture")]
    EpochMismatch,
    #[error("PM replay lifecycle transition does not match the session")]
    LifecycleMismatch,
    #[error("PM replay book event has no snapshot revision")]
    MissingSnapshotRevision,
    #[error("PM replay snapshot flow token did not match the ready reducer state")]
    SnapshotFlowMismatch,
    #[error("PM replay projection exceeds 16 MiB")]
    ProjectionTooLarge,
    #[error("PM replay projection serialization failed: {0}")]
    ProjectionSerialization(#[from] serde_json::Error),
    #[error("PM replay projection serialization changed size between bounded passes")]
    ProjectionSerializationSizeChanged,
    #[error("PM data resumed after tick drift without a new authoritative metadata artifact")]
    MetadataRefreshRequired,
}

pub fn replay_pm_public_capture(
    path: &Path,
    expected_header: &PmCaptureHeader,
) -> Result<PmReplayProjection, PmReplayError> {
    let verification = verify_pm_public_capture(path, expected_header)?;
    let (mut session, mut reducer, mut okx_session) = build_runtime(expected_header)?;
    let mut replay = ReplayState::new(verification, expected_header.clone());
    let mut replay_error = None;
    let scan = scan_jsonl_file_bounded_total(
        path,
        MAX_PM_PUBLIC_CAPTURE_FRAME_BYTES,
        MAX_PM_PUBLIC_CAPTURE_ENCODED_BYTES,
        |bytes| {
            if replay_error.is_some() {
                return true;
            }
            let record = match serde_json::from_slice::<PmPublicCaptureRecord>(jsonl_payload(bytes))
            {
                Ok(record) => record,
                Err(_) => {
                    replay_error = Some(PmReplayError::MalformedCaptureRecord);
                    return true;
                }
            };
            match replay.apply_record(record, &mut session, &mut reducer, &mut okx_session) {
                Ok(()) => true,
                Err(error) => {
                    replay_error = Some(error);
                    true
                }
            }
        },
    )?;
    if scan.sha256 != replay.verification.artifact_sha256
        || scan.bytes != replay.verification.bytes
        || scan.records != replay.verification.records
        || scan.invalid_records != 0
        || scan.has_trailing_partial_record
        || !scan.stable_while_reading
    {
        return Err(PmReplayError::CaptureChangedBeforeReplay);
    }
    if let Some(error) = replay_error {
        return Err(error);
    }
    replay.finish(reducer.counters())
}

fn build_runtime(
    header: &PmCaptureHeader,
) -> Result<(PmPublicSession, PmBookReducer, OkxPublicSession), PmReplayError> {
    let scope = header.scope();
    let policy = header.session_policy();
    let authoritative = scope.authoritative_metadata()?;
    let role = PmPublicRole::from_expected_metadata(
        scope.observation_grant()?,
        scope.metadata(),
        scope.source(),
        scope.connection_id(),
    )?;
    let session = PmPublicSession::new(
        role,
        authoritative,
        policy.pm_initial_epoch(),
        policy.pm_last_snapshot_revision(),
        policy.pm_reconnect().as_transport(),
        policy.pm_heartbeat()?,
    )?;
    let metadata_fingerprint = PmMetadataFingerprint::new(authoritative.metadata_fingerprint())?;
    let domain_fingerprint = PmDomainFingerprint::new(authoritative.domain_fingerprint())?;
    let contract = PmMetadataContract::goal_f_clob_v2(scope.metadata(), domain_fingerprint);
    let mut reducer = PmBookReducer::new(
        scope.instrument(),
        metadata_fingerprint,
        contract,
        policy.freshness()?,
    )?;
    let metadata_observation = PmMetadataObservation::new(
        scope.instrument(),
        scope.metadata_revision(),
        metadata_fingerprint,
        contract,
        scope.metadata_monotonic_receive_ns(),
    )?;
    if !matches!(
        reducer.apply_metadata(metadata_observation)?,
        PmBookTransition::MetadataAccepted { .. }
    ) {
        return Err(PmReplayError::LifecycleMismatch);
    }
    let okx_session = OkxPublicSession::new_configured_capture(
        scope.okx_reference_instrument().instrument_id().as_str(),
        scope.okx_connection_id().as_str(),
        policy.okx_initial_epoch(),
        policy.okx_reconnect().as_transport(),
    )?;
    Ok((session, reducer, okx_session))
}

struct ProjectionBudget {
    serialized_bytes: usize,
    events: usize,
    payload_capacity_bytes: usize,
    max_bytes: usize,
    max_events: usize,
}

#[derive(Debug, Clone, Copy)]
struct ProjectionAdmission {
    serialized_bytes: usize,
    payload_capacity_bytes: usize,
}

impl ProjectionBudget {
    const fn new(max_bytes: usize, max_events: usize) -> Self {
        Self {
            serialized_bytes: 0,
            events: 0,
            payload_capacity_bytes: 0,
            max_bytes,
            max_events,
        }
    }

    fn measure(&self, event: &PmReplayLogicalEvent) -> Result<ProjectionAdmission, PmReplayError> {
        if self.events >= self.max_events {
            return Err(PmReplayError::ProjectionTooLarge);
        }
        let remaining = self.max_bytes.saturating_sub(self.serialized_bytes);
        let mut counter = ProjectionByteCounter::new(remaining.saturating_sub(1));
        let serialization = serde_json::to_writer(&mut counter, event);
        if counter.overflowed {
            return Err(PmReplayError::ProjectionTooLarge);
        }
        serialization?;
        let serialized_bytes = counter.bytes.saturating_add(1);
        self.serialized_bytes
            .checked_add(serialized_bytes)
            .filter(|value| *value <= self.max_bytes)
            .ok_or(PmReplayError::ProjectionTooLarge)?;
        let payload_capacity_bytes = logical_event_payload_capacity(event);
        self.payload_capacity_bytes
            .checked_add(payload_capacity_bytes)
            .filter(|value| *value <= self.max_bytes)
            .ok_or(PmReplayError::ProjectionTooLarge)?;
        Ok(ProjectionAdmission {
            serialized_bytes,
            payload_capacity_bytes,
        })
    }

    fn commit(&mut self, admission: ProjectionAdmission) {
        self.serialized_bytes = self
            .serialized_bytes
            .saturating_add(admission.serialized_bytes);
        self.events = self.events.saturating_add(1);
        self.payload_capacity_bytes = self
            .payload_capacity_bytes
            .saturating_add(admission.payload_capacity_bytes);
    }

    fn resident_capacity_bytes(
        &self,
        event_capacity: usize,
        additional_payload_bytes: usize,
    ) -> Result<usize, PmReplayError> {
        event_capacity
            .checked_mul(size_of::<PmReplayLogicalEvent>())
            .and_then(|bytes| bytes.checked_add(self.payload_capacity_bytes))
            .and_then(|bytes| bytes.checked_add(additional_payload_bytes))
            .filter(|bytes| *bytes <= self.max_bytes)
            .ok_or(PmReplayError::ProjectionTooLarge)
    }
}

fn logical_event_payload_capacity(event: &PmReplayLogicalEvent) -> usize {
    match event {
        PmReplayLogicalEvent::OkxReference {
            instrument,
            index_price_lexeme,
            ..
        } => instrument
            .capacity()
            .saturating_add(index_price_lexeme.capacity()),
        PmReplayLogicalEvent::DeltaBatchCommitted {
            ordered_change_hashes_sha256,
            ..
        } => ordered_change_hashes_sha256.capacity(),
        _ => 0,
    }
}

fn reserve_logical_event_capacity(
    events: &mut Vec<PmReplayLogicalEvent>,
    budget: &ProjectionBudget,
    admission: ProjectionAdmission,
) -> Result<(), PmReplayError> {
    if events.len() < events.capacity() {
        budget.resident_capacity_bytes(events.capacity(), admission.payload_capacity_bytes)?;
        return Ok(());
    }
    let minimum_capacity = events
        .len()
        .checked_add(1)
        .ok_or(PmReplayError::ProjectionTooLarge)?;
    let available_inline_bytes = budget
        .max_bytes
        .checked_sub(
            budget
                .payload_capacity_bytes
                .saturating_add(admission.payload_capacity_bytes),
        )
        .ok_or(PmReplayError::ProjectionTooLarge)?;
    let maximum_capacity = available_inline_bytes / size_of::<PmReplayLogicalEvent>();
    if minimum_capacity > maximum_capacity {
        return Err(PmReplayError::ProjectionTooLarge);
    }
    let next_capacity = if events.capacity() == 0 {
        PM_REPLAY_EVENT_RESERVE_CHUNK.min(maximum_capacity)
    } else {
        events.capacity().saturating_mul(2).min(maximum_capacity)
    }
    .max(minimum_capacity);
    budget.resident_capacity_bytes(next_capacity, admission.payload_capacity_bytes)?;
    events
        .try_reserve_exact(next_capacity - events.capacity())
        .map_err(|_| PmReplayError::ProjectionTooLarge)?;
    budget.resident_capacity_bytes(events.capacity(), admission.payload_capacity_bytes)?;
    Ok(())
}

struct ProjectionByteCounter {
    bytes: usize,
    limit: usize,
    overflowed: bool,
}

impl ProjectionByteCounter {
    const fn new(limit: usize) -> Self {
        Self {
            bytes: 0,
            limit,
            overflowed: false,
        }
    }
}

impl Write for ProjectionByteCounter {
    fn write(&mut self, buffer: &[u8]) -> std::io::Result<usize> {
        let next = self.bytes.saturating_add(buffer.len());
        if next > self.limit {
            self.overflowed = true;
            return Err(std::io::Error::other(
                "PM replay logical-event budget exceeded",
            ));
        }
        self.bytes = next;
        Ok(buffer.len())
    }

    fn flush(&mut self) -> std::io::Result<()> {
        Ok(())
    }
}

struct BoundedSha256Writer {
    hasher: Sha256,
    bytes: usize,
    limit: usize,
    overflowed: bool,
}

struct FixedCapacityJsonWriter {
    bytes: Vec<u8>,
    expected_bytes: usize,
    observed_bytes: usize,
    overflowed: bool,
}

impl FixedCapacityJsonWriter {
    fn new(expected_bytes: usize) -> Self {
        Self {
            bytes: Vec::with_capacity(expected_bytes),
            expected_bytes,
            observed_bytes: 0,
            overflowed: false,
        }
    }
}

impl Write for FixedCapacityJsonWriter {
    fn write(&mut self, buffer: &[u8]) -> std::io::Result<usize> {
        self.observed_bytes = self.observed_bytes.saturating_add(buffer.len());
        if buffer.len() > self.expected_bytes.saturating_sub(self.bytes.len()) {
            self.overflowed = true;
            return Err(std::io::Error::other(
                "PM replay canonical serialization exceeded its measured capacity",
            ));
        }
        self.bytes.extend_from_slice(buffer);
        Ok(buffer.len())
    }

    fn flush(&mut self) -> std::io::Result<()> {
        Ok(())
    }
}

fn canonical_json_bytes_bounded(
    value: &impl Serialize,
    limit: usize,
) -> Result<Vec<u8>, PmReplayError> {
    let frame_limit = limit
        .checked_add(1)
        .ok_or(PmReplayError::ProjectionTooLarge)?;
    let measured_frame = match measure_jsonl_frame_bounded(value, frame_limit) {
        Ok(measured) => measured,
        Err(BoundedJsonlFrameError::FrameTooLarge { .. }) => {
            return Err(PmReplayError::ProjectionTooLarge);
        }
        Err(BoundedJsonlFrameError::Serialization(source)) => {
            return Err(PmReplayError::ProjectionSerialization(source));
        }
        Err(BoundedJsonlFrameError::SizeChanged { .. }) => {
            return Err(PmReplayError::ProjectionSerializationSizeChanged);
        }
    };
    let measured_bytes = measured_frame
        .checked_sub(1)
        .ok_or(PmReplayError::ProjectionSerializationSizeChanged)?;
    let mut writer = FixedCapacityJsonWriter::new(measured_bytes);
    let serialization = serde_json::to_writer(&mut writer, value);
    if writer.overflowed {
        return Err(PmReplayError::ProjectionSerializationSizeChanged);
    }
    serialization?;
    if writer.observed_bytes != measured_bytes || writer.bytes.len() != measured_bytes {
        return Err(PmReplayError::ProjectionSerializationSizeChanged);
    }
    Ok(writer.bytes)
}

impl BoundedSha256Writer {
    fn new(limit: usize) -> Self {
        Self {
            hasher: Sha256::new(),
            bytes: 0,
            limit,
            overflowed: false,
        }
    }

    fn finish(self) -> String {
        self.hasher
            .finalize()
            .iter()
            .map(|byte| format!("{byte:02x}"))
            .collect()
    }
}

impl Write for BoundedSha256Writer {
    fn write(&mut self, buffer: &[u8]) -> std::io::Result<usize> {
        let Some(next) = self.bytes.checked_add(buffer.len()) else {
            self.overflowed = true;
            return Err(std::io::Error::other(
                "PM replay canonical serialization overflowed",
            ));
        };
        if next > self.limit {
            self.overflowed = true;
            return Err(std::io::Error::other(
                "PM replay canonical serialization exceeded its bound",
            ));
        }
        self.hasher.update(buffer);
        self.bytes = next;
        Ok(buffer.len())
    }

    fn flush(&mut self) -> std::io::Result<()> {
        Ok(())
    }
}

fn canonical_json_sha256_bounded(
    value: &impl Serialize,
    limit: usize,
) -> Result<String, PmReplayError> {
    let mut writer = BoundedSha256Writer::new(limit);
    let serialization = serde_json::to_writer(&mut writer, value);
    if writer.overflowed {
        return Err(PmReplayError::ProjectionTooLarge);
    }
    serialization?;
    Ok(writer.finish())
}

struct ReplayState {
    verification: PmCaptureVerification,
    expected_header: PmCaptureHeader,
    logical_events: Vec<PmReplayLogicalEvent>,
    projection_budget: ProjectionBudget,
    counters: PmReplayCounters,
    metadata_refresh_required: bool,
}

impl ReplayState {
    fn new(verification: PmCaptureVerification, expected_header: PmCaptureHeader) -> Self {
        Self {
            logical_events: Vec::new(),
            projection_budget: ProjectionBudget::new(
                MAX_PM_REPLAY_LOGICAL_EVENT_BYTES,
                MAX_PM_REPLAY_LOGICAL_EVENTS,
            ),
            counters: PmReplayCounters {
                capture_records: verification.records,
                raw_public_frames: verification.raw_public_frames,
                okx_raw_public_frames: verification.okx_raw_public_frames,
                lifecycle_records: verification.lifecycle_records,
                okx_lifecycle_records: verification.okx_lifecycle_records,
                freshness_timers: verification.freshness_timers,
                ..PmReplayCounters::default()
            },
            verification,
            expected_header,
            metadata_refresh_required: false,
        }
    }

    fn push_logical(&mut self, event: PmReplayLogicalEvent) -> Result<(), PmReplayError> {
        let admission = self.projection_budget.measure(&event)?;
        reserve_logical_event_capacity(
            &mut self.logical_events,
            &self.projection_budget,
            admission,
        )?;
        self.projection_budget.commit(admission);
        self.logical_events.push(event);
        Ok(())
    }

    fn apply_record(
        &mut self,
        record: PmPublicCaptureRecord,
        session: &mut PmPublicSession,
        reducer: &mut PmBookReducer,
        okx_session: &mut OkxPublicSession,
    ) -> Result<(), PmReplayError> {
        match record {
            PmPublicCaptureRecord::Header { header, .. } => {
                if header.as_ref() == &self.expected_header {
                    Ok(())
                } else {
                    Err(PmReplayError::ReplayHeaderMismatch)
                }
            }
            PmPublicCaptureRecord::RawPublicFrame { frame, .. } => {
                if self.metadata_refresh_required {
                    return Err(PmReplayError::MetadataRefreshRequired);
                }
                self.apply_raw(frame, session, reducer)
            }
            PmPublicCaptureRecord::OkxRawPublicFrame { frame, .. } => {
                self.apply_okx_raw(frame, okx_session)
            }
            PmPublicCaptureRecord::Lifecycle {
                connection_epoch,
                monotonic_ns,
                event,
                ..
            } => self.apply_lifecycle(
                connection_epoch.value(),
                monotonic_ns,
                event,
                session,
                reducer,
            ),
            PmPublicCaptureRecord::OkxLifecycle {
                connection_epoch,
                monotonic_ns,
                event,
                ..
            } => self.apply_okx_lifecycle(connection_epoch, monotonic_ns, event, okx_session),
            PmPublicCaptureRecord::FreshnessTimer { monotonic_ns, .. } => {
                if self.metadata_refresh_required {
                    return Err(PmReplayError::MetadataRefreshRequired);
                }
                let readiness = reducer.readiness();
                let epoch = reducer
                    .connection_epoch()
                    .ok_or(PmReplayError::EpochMismatch)?
                    .value();
                let snapshot_revision = readiness
                    .snapshot_revision()
                    .ok_or(PmReplayError::MissingSnapshotRevision)?
                    .value();
                match reducer.check_freshness(monotonic_ns) {
                    Ok(PmBookTransition::FreshnessConfirmed) => {
                        self.push_logical(PmReplayLogicalEvent::FreshnessConfirmed {
                            epoch,
                            snapshot_revision,
                            monotonic_ns,
                        })
                    }
                    Err(PmPublicReadinessReason::MetadataStale) => {
                        self.push_logical(PmReplayLogicalEvent::FreshnessInvalidated {
                            epoch,
                            snapshot_revision,
                            monotonic_ns,
                            reason: PmReplayFreshnessInvalidation::MetadataStale,
                        })
                    }
                    Err(PmPublicReadinessReason::BookStale) => {
                        self.push_logical(PmReplayLogicalEvent::FreshnessInvalidated {
                            epoch,
                            snapshot_revision,
                            monotonic_ns,
                            reason: PmReplayFreshnessInvalidation::BookStale,
                        })
                    }
                    Ok(_) => Err(PmReplayError::LifecycleMismatch),
                    Err(reason) => Err(PmReplayError::Reducer(reason)),
                }
            }
        }
    }

    fn apply_raw(
        &mut self,
        frame: PmRawPublicFrame,
        session: &mut PmPublicSession,
        reducer: &mut PmBookReducer,
    ) -> Result<(), PmReplayError> {
        if session.connection_epoch() != frame.connection_epoch() {
            return Err(PmReplayError::EpochMismatch);
        }
        let raw = frame.decode_raw()?;
        let batch = session.classify(
            &raw,
            frame.local_wall_receive_ns(),
            frame.monotonic_receive_ns(),
        )?;
        if let Some(heartbeat) = batch.heartbeat() {
            self.counters.heartbeat_pongs = self.counters.heartbeat_pongs.saturating_add(1);
            self.push_logical(PmReplayLogicalEvent::HeartbeatPong {
                epoch: heartbeat.connection_epoch().value(),
                monotonic_receive_ns: heartbeat.monotonic_receive_ns(),
            })?;
        }
        for _ in batch.ignored() {
            self.counters.public_trades_ignored =
                self.counters.public_trades_ignored.saturating_add(1);
            self.push_logical(PmReplayLogicalEvent::PublicTradeIgnored {
                epoch: frame.connection_epoch().value(),
            })?;
        }

        let flow_token = batch.snapshot_flow_token();
        let mut applied_snapshot = None;
        for event in batch.events() {
            let ordering = event.ordering();
            let snapshot_revision = ordering
                .snapshot_revision()
                .ok_or(PmReplayError::MissingSnapshotRevision)?;
            let payload = event.payload();
            let evidence = PmBookBatchEvidence::new(
                payload.instrument(),
                ordering.connection_epoch(),
                payload.metadata_revision(),
                snapshot_revision,
                ordering.local_ingress_sequence(),
                event.received_clock().monotonic_receive_ns(),
                ordering.venue_hash(),
            )?;
            let transition = match reducer.apply_update(evidence, payload.update()) {
                Ok(transition) => transition,
                Err(PmPublicReadinessReason::MetadataDrift(PmMetadataDrift::Grid))
                    if matches!(payload.update(), PmBookUpdate::TickSizeChanged { .. }) =>
                {
                    let PmBookUpdate::TickSizeChanged { old, new } = payload.update() else {
                        unreachable!("guarded tick-size update");
                    };
                    self.counters.normalized_book_batches =
                        self.counters.normalized_book_batches.saturating_add(1);
                    self.counters.tick_size_invalidations =
                        self.counters.tick_size_invalidations.saturating_add(1);
                    self.metadata_refresh_required = true;
                    self.push_logical(PmReplayLogicalEvent::TickSizeInvalidated {
                        epoch: ordering.connection_epoch().value(),
                        snapshot_revision: snapshot_revision.value(),
                        local_ingress_sequence: ordering.local_ingress_sequence().value(),
                        old_tick_units: old.units(),
                        new_tick_units: new.units(),
                    })?;
                    continue;
                }
                Err(error) => {
                    session.invalidate(PmPublicSessionFault::ReducerRejected);
                    return Err(error.into());
                }
            };
            self.counters.normalized_book_batches =
                self.counters.normalized_book_batches.saturating_add(1);
            if let PmBookTransition::SnapshotCommitted {
                revision,
                levels,
                proof,
            } = &transition
            {
                let PmBookUpdate::Snapshot(snapshot) = payload.update() else {
                    return Err(PmReplayError::SnapshotFlowMismatch);
                };
                if *revision != snapshot_revision
                    || usize::from(*levels) != snapshot.levels().len()
                    || proof.instrument() != payload.instrument()
                    || proof.metadata_fingerprint() != reducer.expected_metadata_fingerprint()
                    || proof.connection_epoch() != ordering.connection_epoch()
                    || proof.metadata_revision() != payload.metadata_revision()
                    || proof.snapshot_revision() != snapshot_revision
                    || proof.local_ingress_sequence() != ordering.local_ingress_sequence()
                    || Some(proof.venue_hash()) != ordering.venue_hash()
                {
                    session.invalidate(PmPublicSessionFault::ReducerRejected);
                    let _ = reducer.apply_external_fault(
                        ordering.connection_epoch(),
                        PmExternalBookFault::InvalidTransition,
                    );
                    return Err(PmReplayError::SnapshotFlowMismatch);
                }
                applied_snapshot = Some((
                    ordering.connection_epoch(),
                    snapshot_revision,
                    ordering.local_ingress_sequence(),
                    ordering.venue_hash(),
                ));
            }
            self.push_book_transition(event.envelope(), transition, reducer)?;
        }
        if let Some(token) = flow_token {
            let readiness = reducer.readiness();
            if applied_snapshot
                != Some((
                    token.connection_epoch(),
                    token.snapshot_revision(),
                    token.local_ingress_sequence(),
                    Some(token.venue_hash()),
                ))
                || !readiness.is_ready()
                || reducer.connection_epoch() != Some(token.connection_epoch())
                || readiness.metadata_revision() != Some(session.metadata_revision())
                || readiness.snapshot_revision() != Some(token.snapshot_revision())
                || reducer.last_verified_snapshot_hash() != Some(token.venue_hash())
            {
                session.invalidate(PmPublicSessionFault::ReducerRejected);
                let _ = reducer.apply_external_fault(
                    token.connection_epoch(),
                    PmExternalBookFault::InvalidTransition,
                );
                return Err(PmReplayError::SnapshotFlowMismatch);
            }
            session.open_protocol_flow_after_snapshot(token)?;
        }
        Ok(())
    }

    fn apply_okx_raw(
        &mut self,
        frame: crate::capture::OkxRawPublicFrame,
        session: &mut OkxPublicSession,
    ) -> Result<(), PmReplayError> {
        if session.connection_epoch() != frame.connection_epoch() {
            return Err(PmReplayError::EpochMismatch);
        }
        let raw = frame.decode_raw()?;
        let payload = std::str::from_utf8(&raw).map_err(|_| PmReplayError::OkxRawNotUtf8)?;
        let delivery = session.classify_captured_payload(
            payload,
            frame.local_wall_receive_ns(),
            frame.monotonic_receive_ns(),
            frame.raw_hash(),
        )?;
        let logical = match delivery.payload() {
            OkxPublicSessionEvent::SubscriptionAcknowledged(evidence) => {
                self.counters.okx_subscription_acknowledgements = self
                    .counters
                    .okx_subscription_acknowledgements
                    .saturating_add(1);
                PmReplayLogicalEvent::OkxSubscriptionAcknowledged {
                    epoch: evidence.connection_epoch(),
                }
            }
            OkxPublicSessionEvent::Heartbeat(evidence) => PmReplayLogicalEvent::OkxHeartbeat {
                epoch: evidence.connection_epoch(),
            },
            OkxPublicSessionEvent::Control(control) => PmReplayLogicalEvent::OkxControl {
                epoch: control.source().connection_epoch(),
            },
            OkxPublicSessionEvent::Reference(reference) => {
                self.counters.okx_references = self.counters.okx_references.saturating_add(1);
                PmReplayLogicalEvent::OkxReference {
                    epoch: reference.connection_epoch(),
                    instrument: reference.instrument().to_string(),
                    index_price_lexeme: reference.index_price_lexeme().to_string(),
                    venue_timestamp_ms: reference.venue_ts_ms(),
                    wall_receive_ns: reference.wall_receive_ts_ns(),
                    monotonic_receive_ns: delivery.monotonic_receive_ns(),
                    raw_hash: reference.raw_hash(),
                }
            }
        };
        self.push_logical(logical)
    }

    fn push_book_transition(
        &mut self,
        event: &reap_pm_core::ReceivedEventEnvelope<reap_pm_core::PmBookEvent>,
        transition: PmBookTransition,
        reducer: &PmBookReducer,
    ) -> Result<(), PmReplayError> {
        let ordering = event.ordering();
        let revision = ordering
            .snapshot_revision()
            .ok_or(PmReplayError::MissingSnapshotRevision)?;
        let (best_bid_units, best_ask_units) = canonical_top_units(reducer)?;
        let common = (
            ordering.connection_epoch().value(),
            revision.value(),
            ordering.local_ingress_sequence().value(),
            event.received_clock().venue_event_timestamp_ns(),
        );
        let logical = match transition {
            PmBookTransition::SnapshotCommitted { levels, .. } => {
                PmReplayLogicalEvent::SnapshotCommitted {
                    epoch: common.0,
                    snapshot_revision: common.1,
                    local_ingress_sequence: common.2,
                    venue_timestamp_ns: common.3,
                    levels,
                    best_bid_units,
                    best_ask_units,
                }
            }
            PmBookTransition::DeltaBatchCommitted { changes, .. } => {
                let PmBookUpdate::DeltaBatch(batch) = event.payload().update() else {
                    return Err(PmReplayError::LifecycleMismatch);
                };
                let (venue_change_hashes_present, ordered_change_hashes_sha256) =
                    ordered_change_hash_evidence(batch);
                PmReplayLogicalEvent::DeltaBatchCommitted {
                    epoch: common.0,
                    snapshot_revision: common.1,
                    local_ingress_sequence: common.2,
                    venue_timestamp_ns: common.3,
                    changes,
                    venue_change_hashes_present,
                    ordered_change_hashes_sha256,
                    best_bid_units,
                    best_ask_units,
                }
            }
            PmBookTransition::TopConfirmed => PmReplayLogicalEvent::TopConfirmed {
                epoch: common.0,
                snapshot_revision: common.1,
                local_ingress_sequence: common.2,
                venue_timestamp_ns: common.3,
                best_bid_units,
                best_ask_units,
            },
            PmBookTransition::MetadataAccepted { .. }
            | PmBookTransition::EpochStarted { .. }
            | PmBookTransition::FreshnessConfirmed => {
                return Err(PmReplayError::LifecycleMismatch);
            }
        };
        self.push_logical(logical)
    }

    fn apply_okx_lifecycle(
        &mut self,
        epoch: u64,
        monotonic_ns: u64,
        event: OkxCaptureLifecycle,
        session: &mut OkxPublicSession,
    ) -> Result<(), PmReplayError> {
        if session.connection_epoch() != epoch {
            return Err(PmReplayError::EpochMismatch);
        }
        let logical = match event {
            OkxCaptureLifecycle::ConnectionStarted => {
                PmReplayLogicalEvent::OkxConnectionStarted { epoch }
            }
            OkxCaptureLifecycle::SubscriptionSent => {
                if session.subscription_ready() {
                    return Err(PmReplayError::LifecycleMismatch);
                }
                PmReplayLogicalEvent::OkxSubscriptionSent { epoch }
            }
            OkxCaptureLifecycle::Disconnected {
                local_wall_receive_ns,
                reason,
            } => {
                let fault = match reason {
                    OkxCaptureDisconnectReason::Disconnect => OkxPublicSessionFault::Disconnect,
                    OkxCaptureDisconnectReason::Overflow => OkxPublicSessionFault::Overflow,
                    OkxCaptureDisconnectReason::Stale => OkxPublicSessionFault::Stale,
                };
                session.invalidate_with_receive_evidence(
                    fault,
                    local_wall_receive_ns,
                    monotonic_ns,
                )?;
                session
                    .take_unavailable()
                    .ok_or(PmReplayError::LifecycleMismatch)?;
                self.counters.okx_disconnects = self.counters.okx_disconnects.saturating_add(1);
                PmReplayLogicalEvent::OkxDisconnected { epoch, reason }
            }
            OkxCaptureLifecycle::ReconnectScheduled {
                next_epoch,
                delay_ns,
            } => {
                let delay = session.after_failure()?;
                if session.connection_epoch() != next_epoch
                    || u64::try_from(delay.as_nanos()).ok() != Some(delay_ns)
                {
                    return Err(PmReplayError::LifecycleMismatch);
                }
                self.counters.okx_reconnects = self.counters.okx_reconnects.saturating_add(1);
                PmReplayLogicalEvent::OkxReconnectScheduled {
                    prior_epoch: epoch,
                    next_epoch,
                    delay_ns,
                }
            }
        };
        self.push_logical(logical)
    }

    #[allow(clippy::too_many_arguments)]
    fn apply_lifecycle(
        &mut self,
        epoch: u64,
        monotonic_ns: u64,
        event: PmCaptureLifecycle,
        session: &mut PmPublicSession,
        reducer: &mut PmBookReducer,
    ) -> Result<(), PmReplayError> {
        if session.connection_epoch().value() != epoch {
            return Err(PmReplayError::EpochMismatch);
        }
        match event {
            PmCaptureLifecycle::ConnectionStarted => {
                if reducer.connection_epoch().map(|value| value.value()) != Some(epoch) {
                    let transition = reducer.begin_epoch(session.connection_epoch())?;
                    if !matches!(transition, PmBookTransition::EpochStarted { .. }) {
                        return Err(PmReplayError::LifecycleMismatch);
                    }
                }
                self.push_logical(PmReplayLogicalEvent::ConnectionStarted { epoch })?;
            }
            PmCaptureLifecycle::SubscriptionSent => {
                session.mark_subscription_sent(monotonic_ns)?;
                self.push_logical(PmReplayLogicalEvent::SubscriptionSent { epoch })?;
            }
            PmCaptureLifecycle::HeartbeatPingSent => {
                if session.poll_heartbeat(monotonic_ns)? != PmPublicHeartbeatAction::SendPing {
                    return Err(PmReplayError::LifecycleMismatch);
                }
                self.push_logical(PmReplayLogicalEvent::HeartbeatPingSent { epoch })?;
            }
            PmCaptureLifecycle::Disconnected {
                local_wall_receive_ns,
                reason,
            } => {
                let (session_fault, reducer_fault, expected_reason) = match reason {
                    PmCaptureDisconnectReason::Disconnect => (
                        PmPublicSessionFault::Disconnect,
                        PmExternalBookFault::Disconnect,
                        PmPublicReadinessReason::Disconnected,
                    ),
                    PmCaptureDisconnectReason::Gap => (
                        PmPublicSessionFault::Gap,
                        PmExternalBookFault::Gap,
                        PmPublicReadinessReason::Gap,
                    ),
                    PmCaptureDisconnectReason::Overflow => (
                        PmPublicSessionFault::Overflow,
                        PmExternalBookFault::Overflow,
                        PmPublicReadinessReason::Overflow,
                    ),
                    PmCaptureDisconnectReason::Stale => (
                        PmPublicSessionFault::Stale,
                        PmExternalBookFault::BacklogAged,
                        PmPublicReadinessReason::BookStale,
                    ),
                    PmCaptureDisconnectReason::HeartbeatTimeout => (
                        PmPublicSessionFault::HeartbeatTimeout,
                        PmExternalBookFault::HeartbeatTimeout,
                        PmPublicReadinessReason::HeartbeatTimeout,
                    ),
                };
                if reason == PmCaptureDisconnectReason::HeartbeatTimeout {
                    match session
                        .poll_heartbeat_with_receive_evidence(local_wall_receive_ns, monotonic_ns)
                    {
                        Err(PmPublicSessionError::HeartbeatTimeout { .. }) => {}
                        Ok(_) | Err(_) => return Err(PmReplayError::LifecycleMismatch),
                    }
                } else {
                    session.invalidate_with_receive_evidence(
                        session_fault,
                        local_wall_receive_ns,
                        monotonic_ns,
                    )?;
                }
                session
                    .take_unavailable()
                    .ok_or(PmReplayError::LifecycleMismatch)?;
                let result =
                    reducer.apply_external_fault(session.connection_epoch(), reducer_fault);
                if result != Err(expected_reason) {
                    return Err(PmReplayError::LifecycleMismatch);
                }
                self.push_logical(PmReplayLogicalEvent::Disconnected { epoch, reason })?;
            }
            PmCaptureLifecycle::ReconnectScheduled {
                next_epoch,
                delay_ns,
            } => {
                let delay = session.after_failure()?;
                if session.connection_epoch() != next_epoch
                    || u64::try_from(delay.as_nanos()).ok() != Some(delay_ns)
                {
                    return Err(PmReplayError::LifecycleMismatch);
                }
                self.push_logical(PmReplayLogicalEvent::ReconnectScheduled {
                    prior_epoch: epoch,
                    next_epoch: next_epoch.value(),
                    delay_ns,
                })?;
            }
        }
        Ok(())
    }

    fn finish(mut self, reducer: PmBookCounters) -> Result<PmReplayProjection, PmReplayError> {
        self.counters.snapshots_committed = reducer.snapshots_committed;
        self.counters.snapshot_levels_committed = reducer.snapshot_levels_committed;
        self.counters.resync_snapshots = reducer.resync_snapshots;
        self.counters.delta_batches_committed = reducer.delta_batches_committed;
        self.counters.delta_changes_committed = reducer.delta_changes_committed;
        self.counters.delta_top_checks_confirmed = reducer.delta_top_checks_confirmed;
        self.counters.top_checks_confirmed = reducer.top_checks_confirmed;
        self.counters.freshness_checks = reducer.freshness_checks;
        self.counters.freshness_confirmed = reducer.freshness_confirmed;
        self.counters.stale_invalidations = reducer.stale_invalidations;
        self.counters.unavailable_transitions = reducer.unavailable_transitions;
        self.counters.external_faults = reducer.external_faults;
        self.counters.disconnects = reducer.disconnects;
        self.counters.heartbeat_timeouts = reducer.heartbeat_timeouts;
        self.counters.backlog_aged_faults = reducer.backlog_aged_faults;
        self.counters.gaps = reducer.gaps;
        self.counters.overflows = reducer.overflows;
        self.counters.reconnects = reducer.reconnects;
        self.counters.invalidations = reducer.invalidations;
        self.counters.metadata_refresh_required = self.metadata_refresh_required;
        let event_capacity_bytes = self
            .logical_events
            .capacity()
            .checked_mul(size_of::<PmReplayLogicalEvent>())
            .ok_or(PmReplayError::ProjectionTooLarge)?;
        let payload_capacity_bytes = self.projection_budget.payload_capacity_bytes;
        let reserved_capacity_bytes = self
            .projection_budget
            .resident_capacity_bytes(self.logical_events.capacity(), 0)?;
        self.counters.projection_event_capacity_bytes =
            u64::try_from(event_capacity_bytes).map_err(|_| PmReplayError::ProjectionTooLarge)?;
        self.counters.projection_payload_capacity_bytes =
            u64::try_from(payload_capacity_bytes).map_err(|_| PmReplayError::ProjectionTooLarge)?;
        self.counters.projection_reserved_capacity_bytes =
            u64::try_from(reserved_capacity_bytes)
                .map_err(|_| PmReplayError::ProjectionTooLarge)?;

        let capture_verification_sha256 =
            canonical_json_sha256_bounded(&self.verification, MAX_PM_REPLAY_FIXED_BYTES)?;
        let body = ProjectionBody {
            capture_artifact_sha256: &self.verification.artifact_sha256,
            logical_events: &self.logical_events,
            counters: self.counters,
        };
        let projection_sha256 =
            canonical_json_sha256_bounded(&body, MAX_PM_REPLAY_PROJECTION_BYTES)?;
        Ok(PmReplayProjection {
            schema_version: 1,
            product: "okx_reference_polymarket".to_string(),
            capture_artifact_sha256: self.verification.artifact_sha256,
            capture_verification_sha256,
            logical_events: self.logical_events,
            counters: self.counters,
            projection_sha256,
            production_order_entry_authorized: false,
        })
    }
}

#[derive(Serialize)]
struct ProjectionBody<'a> {
    capture_artifact_sha256: &'a str,
    logical_events: &'a [PmReplayLogicalEvent],
    counters: PmReplayCounters,
}

fn ordered_change_hash_evidence(batch: &reap_pm_core::PmBookDeltaBatch) -> (u16, String) {
    let mut present = 0_u16;
    let mut hasher = Sha256::new();
    hasher.update(b"reap-pm-ordered-change-hashes-v1");
    for change_hash in batch.venue_change_hashes() {
        match change_hash {
            None => hasher.update([0]),
            Some(change_hash) => {
                present = present.saturating_add(1);
                let bytes = change_hash.as_str().as_bytes();
                hasher.update([
                    1,
                    u8::try_from(bytes.len()).expect("venue hash is u8 bounded"),
                ]);
                hasher.update(bytes);
            }
        }
    }
    (present, digest_hex(hasher.finalize()))
}

fn canonical_top_units(reducer: &PmBookReducer) -> Result<(u32, u32), PmReplayError> {
    let bid = reducer
        .levels()
        .iter()
        .filter(|level| level.side() == PmBookSide::Bid)
        .map(|level| level.price().units())
        .max()
        .ok_or(PmPublicReadinessReason::EmptyBook)?;
    let ask = reducer
        .levels()
        .iter()
        .filter(|level| level.side() == PmBookSide::Ask)
        .map(|level| level.price().units())
        .min()
        .ok_or(PmPublicReadinessReason::EmptyBook)?;
    Ok((bid, ask))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn canonical_json_bytes_admits_exact_cap_and_rejects_one_byte_less() {
        let value = "123456";
        let exact = serde_json::to_vec(value).unwrap();

        let bounded = canonical_json_bytes_bounded(&value, exact.len()).unwrap();
        assert_eq!(bounded, exact);
        assert_eq!(bounded.capacity(), exact.len());
        assert!(matches!(
            canonical_json_bytes_bounded(&value, exact.len() - 1),
            Err(PmReplayError::ProjectionTooLarge)
        ));
    }

    #[test]
    fn projection_budget_rejects_bytes_and_event_count_before_accounting_growth() {
        let event = PmReplayLogicalEvent::PublicTradeIgnored { epoch: 1 };
        let charge = serde_json::to_vec(&event).unwrap().len() + 1;
        let mut exact = ProjectionBudget::new(charge * 2, 2);
        let first = exact.measure(&event).unwrap();
        exact.commit(first);
        let second = exact.measure(&event).unwrap();
        exact.commit(second);
        let bytes = exact.serialized_bytes;
        let events = exact.events;
        assert!(matches!(
            exact.measure(&event),
            Err(PmReplayError::ProjectionTooLarge)
        ));
        assert_eq!((exact.serialized_bytes, exact.events), (bytes, events));

        let short = ProjectionBudget::new(charge - 1, 1);
        assert!(matches!(
            short.measure(&event),
            Err(PmReplayError::ProjectionTooLarge)
        ));
        assert_eq!((short.serialized_bytes, short.events), (0, 0));
    }

    #[test]
    fn logical_event_vec_growth_is_admitted_with_capacity_and_payload_bytes() {
        let mut budget = ProjectionBudget::new(
            MAX_PM_REPLAY_LOGICAL_EVENT_BYTES,
            MAX_PM_REPLAY_LOGICAL_EVENTS,
        );
        let mut events = Vec::new();
        for _ in 0..=PM_REPLAY_EVENT_RESERVE_CHUNK {
            let event = PmReplayLogicalEvent::OkxReference {
                epoch: 1,
                instrument: String::from("BTC-USDT"),
                index_price_lexeme: String::from("50000.125"),
                venue_timestamp_ms: 1,
                wall_receive_ns: 1,
                monotonic_receive_ns: 1,
                raw_hash: 1,
            };
            let admission = budget.measure(&event).unwrap();
            reserve_logical_event_capacity(&mut events, &budget, admission).unwrap();
            budget.commit(admission);
            events.push(event);
        }
        let reserved = budget
            .resident_capacity_bytes(events.capacity(), 0)
            .unwrap();
        assert!(reserved <= MAX_PM_REPLAY_LOGICAL_EVENT_BYTES);
        assert!(events.capacity() > PM_REPLAY_EVENT_RESERVE_CHUNK);
        assert_eq!(
            budget.payload_capacity_bytes,
            events
                .iter()
                .map(logical_event_payload_capacity)
                .sum::<usize>()
        );
    }
}
