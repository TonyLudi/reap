use std::path::Path;
use std::time::Duration;

use base64::Engine;
use base64::engine::general_purpose::STANDARD as BASE64;
use reap_capture_framing::{
    BoundedJsonlFrameError, ByteBoundedWriterError, JsonlFileScan, JsonlVerifyError,
    JsonlWriterError, jsonl_payload, scan_jsonl_file_bounded_total, sha256_hex,
};
use reap_okx_public_source::OkxPublicSessionFault;
use reap_pm_core::{
    CLOB_V2_LOT_UNITS, ConnectionEpoch, IngressSequence, OkxReferenceHandle,
    OkxReferenceInstrument, PM_PROTOCOL_SCALE, PmConditionId, PmConnectionId, PmInstrumentHandle,
    PmInstrumentId, PmMarketId, PmMarketMetadata, PmPrice, PmProductSource,
    PmPublicObservationGrant, PmQuantity, PmTick, PmTokenId, SnapshotRevision,
};
use reap_pm_live_contracts::PmPublicConnectivityConfig;
use reap_pm_state::PmBookFreshness;
use reap_polymarket_adapter::{
    MAX_PM_PUBLIC_RAW_FRAME_BYTES, PmAuthoritativeMetadata, PmMetadataJoinError,
    PmMetadataRevisionInput, PmPublicHeartbeatConfig, PmPublicSessionFault,
    PmRecordedMetadataEvidence,
};
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use thiserror::Error;

mod validation;
mod verify;
mod writer;

use validation::{validate_header, validate_provenance, validate_scope};
pub use verify::verify_pm_public_capture;
#[cfg(test)]
pub(crate) use writer::Phase6RawCapacityProbe;
pub(crate) use writer::PmPublicCaptureWriter;

pub const PM_PUBLIC_CAPTURE_SCHEMA_VERSION: u16 = 1;
pub const PM_PUBLIC_CAPTURE_PRODUCT: &str = "okx_reference_polymarket";
pub const MAX_PM_PUBLIC_CAPTURE_RAW_FRAMES: u64 = 8_192;
pub const MAX_PM_PUBLIC_CAPTURE_RECORDS: u64 = 16_384;
pub const MAX_PM_PUBLIC_CAPTURE_RAW_PAYLOAD_BYTES: u64 = 32 * 1024 * 1024;
pub const MAX_PM_PUBLIC_CAPTURE_ENCODED_BYTES: u64 = 48 * 1024 * 1024;
pub const MAX_PM_PUBLIC_CAPTURE_PENDING_AGE_NS: u64 = 500_000_000;
/// The compact JSONL envelope bound. A one-MiB raw frame expands to at most
/// 1,398,104 base64 bytes; the remaining margin covers fixed typed fields.
pub const MAX_PM_PUBLIC_CAPTURE_FRAME_BYTES: usize = 3 * 512 * 1024;
pub const MAX_PM_RAW_PUBLIC_FRAME_BYTES: usize = MAX_PM_PUBLIC_RAW_FRAME_BYTES;
pub const MAX_PM_PUBLIC_CAPTURE_BASE64_FRAME_BYTES: usize =
    MAX_PM_RAW_PUBLIC_FRAME_BYTES.div_ceil(3) * 4;
/// Worst-case live bytes for one schema wrapper plus its one encoded writer
/// frame. The caller-owned raw receive buffer is accounted separately.
pub const MAX_PM_PUBLIC_CAPTURE_RECORD_WORKING_BYTES: usize =
    MAX_PM_PUBLIC_CAPTURE_BASE64_FRAME_BYTES + MAX_PM_PUBLIC_CAPTURE_FRAME_BYTES;

const DIGEST_HEX_BYTES: usize = 64;
const MAX_PROVENANCE_TEXT_BYTES: usize = 128;
const PREDARB_REFERENCE_COMMIT: &str = "8222273a9c72033b760e1d2fec813bc77144556d";
const PREDARB_REFERENCE_SEED_SHA256: &str =
    "8e671f14c4b1e8137b1dc1b0bd7d39c79d9c8f961a8483daa32151df99cbdf81";

/// Exact, secret-free scope bound into every PM public capture.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct PmCaptureScope {
    okx_reference: OkxReferenceHandle,
    okx_reference_instrument: OkxReferenceInstrument,
    okx_source: PmProductSource,
    okx_connection_id: PmConnectionId,
    instrument: PmInstrumentHandle,
    raw_pm_instrument: PmInstrumentId,
    identity_configuration_sha256: String,
    source: PmProductSource,
    connection_id: PmConnectionId,
    metadata: PmMarketMetadata,
    metadata_revision: SnapshotRevision,
    metadata_monotonic_receive_ns: u64,
    metadata_sha256: String,
    domain_sha256: String,
    condition: PmConditionId,
    market: PmMarketId,
    outcome_token: PmTokenId,
    tick: PmTick,
    minimum_order_size: PmQuantity,
    negative_risk: bool,
    price_units_per_one: u32,
    quantity_units_per_one: u32,
    collateral_units_per_one: u32,
    lot_units: u32,
}

impl PmCaptureScope {
    pub fn new(
        config: &PmPublicConnectivityConfig,
        authoritative: &PmAuthoritativeMetadata,
    ) -> Result<Self, PmCaptureVerifyError> {
        let event = authoritative.event();
        if event.instrument() != config.instrument()
            || event.source() != config.polymarket_route().source()
            || event.metadata() != config.expected_metadata()
            || authoritative.parser_config().scope().token()
                != config.polymarket_instrument_id().token()
        {
            return Err(PmCaptureVerifyError::InvalidHeader(
                "checked public config and authoritative metadata differ",
            ));
        }
        let metadata = event.metadata();
        let value = Self {
            okx_reference: config.okx_reference(),
            okx_reference_instrument: config.okx_reference_instrument(),
            okx_source: config.okx_route().source(),
            okx_connection_id: config.okx_route().connection(),
            instrument: event.instrument(),
            raw_pm_instrument: config.polymarket_instrument_id(),
            identity_configuration_sha256: bytes_to_hex(
                &config.configuration_fingerprint().bytes(),
            ),
            source: event.source(),
            connection_id: config.polymarket_route().connection(),
            metadata,
            metadata_revision: event.metadata_revision(),
            metadata_monotonic_receive_ns: authoritative.monotonic_receive_ns(),
            metadata_sha256: bytes_to_hex(&authoritative.metadata_fingerprint()),
            domain_sha256: bytes_to_hex(&authoritative.domain_fingerprint()),
            condition: metadata.condition(),
            market: metadata.market(),
            outcome_token: metadata.outcome().token(),
            tick: metadata.tick(),
            minimum_order_size: metadata.minimum_order_size(),
            negative_risk: metadata.negative_risk(),
            price_units_per_one: PM_PROTOCOL_SCALE,
            quantity_units_per_one: PM_PROTOCOL_SCALE,
            collateral_units_per_one: PM_PROTOCOL_SCALE,
            lot_units: CLOB_V2_LOT_UNITS,
        };
        validate_scope(&value)?;
        Ok(value)
    }

    #[must_use]
    pub const fn instrument(&self) -> PmInstrumentHandle {
        self.instrument
    }

    #[must_use]
    pub const fn okx_reference_instrument(&self) -> OkxReferenceInstrument {
        self.okx_reference_instrument
    }

    #[must_use]
    pub const fn okx_reference(&self) -> OkxReferenceHandle {
        self.okx_reference
    }

    #[must_use]
    pub const fn okx_source(&self) -> PmProductSource {
        self.okx_source
    }

    #[must_use]
    pub const fn okx_connection_id(&self) -> PmConnectionId {
        self.okx_connection_id
    }

    #[must_use]
    pub const fn source(&self) -> PmProductSource {
        self.source
    }

    #[must_use]
    pub const fn connection_id(&self) -> PmConnectionId {
        self.connection_id
    }

    #[must_use]
    pub const fn condition(&self) -> PmConditionId {
        self.condition
    }

    #[must_use]
    pub const fn market(&self) -> PmMarketId {
        self.market
    }

    #[must_use]
    pub const fn outcome_token(&self) -> PmTokenId {
        self.outcome_token
    }

    #[must_use]
    pub const fn tick(&self) -> PmTick {
        self.tick
    }

    #[must_use]
    pub const fn minimum_order_size(&self) -> PmQuantity {
        self.minimum_order_size
    }

    #[must_use]
    pub const fn negative_risk(&self) -> bool {
        self.negative_risk
    }

    #[must_use]
    pub const fn metadata(&self) -> PmMarketMetadata {
        self.metadata
    }

    #[must_use]
    pub const fn metadata_revision(&self) -> SnapshotRevision {
        self.metadata_revision
    }

    #[must_use]
    pub const fn metadata_monotonic_receive_ns(&self) -> u64 {
        self.metadata_monotonic_receive_ns
    }

    #[must_use]
    pub fn metadata_sha256(&self) -> &str {
        &self.metadata_sha256
    }

    #[must_use]
    pub fn domain_sha256(&self) -> &str {
        &self.domain_sha256
    }

    pub fn recorded_metadata(&self) -> Result<PmRecordedMetadataEvidence, PmCaptureVerifyError> {
        let metadata_fingerprint = decode_digest(&self.metadata_sha256)?;
        let domain_fingerprint = decode_digest(&self.domain_sha256)?;
        let revision = PmMetadataRevisionInput::new(
            self.metadata_revision,
            self.metadata_monotonic_receive_ns,
        )
        .map_err(PmCaptureVerifyError::Metadata)?;
        PmRecordedMetadataEvidence::verify(
            self.instrument,
            self.source,
            self.metadata,
            revision,
            metadata_fingerprint,
            domain_fingerprint,
        )
        .map_err(PmCaptureVerifyError::Metadata)
    }

    pub fn observation_grant(&self) -> Result<PmPublicObservationGrant, PmCaptureVerifyError> {
        let grant = PmPublicObservationGrant::derive_goal_f(
            self.okx_reference_instrument,
            self.raw_pm_instrument,
        );
        if grant.okx_reference() != self.okx_reference
            || grant.instrument() != self.instrument
            || grant.configuration_fingerprint().bytes()
                != decode_digest(&self.identity_configuration_sha256)?
        {
            return Err(PmCaptureVerifyError::InvalidHeader(
                "recorded compact/raw observation grant mismatched",
            ));
        }
        Ok(grant)
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct PmCaptureProvenance {
    reference_commit: String,
    reference_blob_oid: String,
    reference_seed_sha256: String,
    fixture_sha256: String,
}

impl PmCaptureProvenance {
    pub fn new(
        reference_commit: impl Into<String>,
        reference_blob_oid: impl Into<String>,
        reference_seed_sha256: impl Into<String>,
        fixture_sha256: impl Into<String>,
    ) -> Result<Self, PmCaptureVerifyError> {
        let value = Self {
            reference_commit: reference_commit.into(),
            reference_blob_oid: reference_blob_oid.into(),
            reference_seed_sha256: reference_seed_sha256.into(),
            fixture_sha256: fixture_sha256.into(),
        };
        validate_provenance(&value)?;
        Ok(value)
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct PmCaptureReconnectPolicy {
    initial_delay_ns: u64,
    max_delay_ns: u64,
    multiplier: u32,
}

impl PmCaptureReconnectPolicy {
    pub fn new(
        initial_delay: Duration,
        max_delay: Duration,
        multiplier: u32,
    ) -> Result<Self, PmCaptureVerifyError> {
        let initial_delay_ns = u64::try_from(initial_delay.as_nanos())
            .map_err(|_| PmCaptureVerifyError::InvalidHeader("reconnect delay overflow"))?;
        let max_delay_ns = u64::try_from(max_delay.as_nanos())
            .map_err(|_| PmCaptureVerifyError::InvalidHeader("reconnect delay overflow"))?;
        let value = Self {
            initial_delay_ns,
            max_delay_ns,
            multiplier,
        };
        value.validate()?;
        Ok(value)
    }

    #[must_use]
    pub fn as_transport(self) -> reap_transport::ReconnectPolicy {
        reap_transport::ReconnectPolicy {
            initial_delay: Duration::from_nanos(self.initial_delay_ns),
            max_delay: Duration::from_nanos(self.max_delay_ns),
            multiplier: self.multiplier,
        }
    }

    fn validate(self) -> Result<(), PmCaptureVerifyError> {
        if self.initial_delay_ns == 0
            || self.max_delay_ns < self.initial_delay_ns
            || self.multiplier < 2
        {
            Err(PmCaptureVerifyError::InvalidHeader(
                "reconnect policy is invalid",
            ))
        } else {
            Ok(())
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct PmCaptureSessionPolicy {
    pm_initial_epoch: ConnectionEpoch,
    pm_last_snapshot_revision: Option<SnapshotRevision>,
    pm_reconnect: PmCaptureReconnectPolicy,
    pm_ping_interval_ns: u64,
    pm_pong_timeout_ns: u64,
    metadata_max_age_ns: u64,
    book_max_age_ns: u64,
    okx_initial_epoch: u64,
    okx_reconnect: PmCaptureReconnectPolicy,
}

impl PmCaptureSessionPolicy {
    #[allow(clippy::too_many_arguments)]
    pub fn new(
        pm_initial_epoch: ConnectionEpoch,
        pm_last_snapshot_revision: Option<SnapshotRevision>,
        pm_reconnect: PmCaptureReconnectPolicy,
        pm_heartbeat: PmPublicHeartbeatConfig,
        freshness: PmBookFreshness,
        okx_initial_epoch: u64,
        okx_reconnect: PmCaptureReconnectPolicy,
    ) -> Result<Self, PmCaptureVerifyError> {
        let value = Self {
            pm_initial_epoch,
            pm_last_snapshot_revision,
            pm_reconnect,
            pm_ping_interval_ns: pm_heartbeat.ping_interval_ns(),
            pm_pong_timeout_ns: pm_heartbeat.pong_timeout_ns(),
            metadata_max_age_ns: freshness.metadata_max_age_ns(),
            book_max_age_ns: freshness.book_max_age_ns(),
            okx_initial_epoch,
            okx_reconnect,
        };
        value.validate()?;
        Ok(value)
    }

    pub fn pm_heartbeat(self) -> Result<PmPublicHeartbeatConfig, PmCaptureVerifyError> {
        PmPublicHeartbeatConfig::new(self.pm_ping_interval_ns, self.pm_pong_timeout_ns)
            .map_err(|_| PmCaptureVerifyError::InvalidHeader("heartbeat policy is invalid"))
    }

    pub fn freshness(self) -> Result<PmBookFreshness, PmCaptureVerifyError> {
        PmBookFreshness::new(self.metadata_max_age_ns, self.book_max_age_ns)
            .map_err(|_| PmCaptureVerifyError::InvalidHeader("freshness policy is invalid"))
    }

    #[must_use]
    pub const fn pm_initial_epoch(self) -> ConnectionEpoch {
        self.pm_initial_epoch
    }

    #[must_use]
    pub const fn pm_last_snapshot_revision(self) -> Option<SnapshotRevision> {
        self.pm_last_snapshot_revision
    }

    #[must_use]
    pub const fn pm_reconnect(self) -> PmCaptureReconnectPolicy {
        self.pm_reconnect
    }

    #[must_use]
    pub const fn okx_initial_epoch(self) -> u64 {
        self.okx_initial_epoch
    }

    #[must_use]
    pub const fn okx_reconnect(self) -> PmCaptureReconnectPolicy {
        self.okx_reconnect
    }

    fn validate(self) -> Result<(), PmCaptureVerifyError> {
        if self.pm_initial_epoch.value() == 0
            || self
                .pm_last_snapshot_revision
                .is_some_and(|value| value.value() == 0)
            || self.okx_initial_epoch == 0
        {
            return Err(PmCaptureVerifyError::InvalidHeader(
                "session epochs and revisions must be positive",
            ));
        }
        self.pm_reconnect.validate()?;
        self.okx_reconnect.validate()?;
        self.pm_heartbeat()?;
        self.freshness()?;
        Ok(())
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct PmCaptureHeader {
    schema_version: u16,
    product: String,
    configuration_sha256: String,
    structural_scope_sha256: String,
    scope: PmCaptureScope,
    session_policy: PmCaptureSessionPolicy,
    provenance: PmCaptureProvenance,
    authenticated: bool,
    production_order_entry_authorized: bool,
}

impl PmCaptureHeader {
    pub fn new(
        scope: PmCaptureScope,
        session_policy: PmCaptureSessionPolicy,
        provenance: PmCaptureProvenance,
    ) -> Result<Self, PmCaptureVerifyError> {
        let scope_bytes =
            serde_json::to_vec(&scope).map_err(PmCaptureVerifyError::SerializeHeader)?;
        let configuration_bytes = serde_json::to_vec(&(&scope, session_policy))
            .map_err(PmCaptureVerifyError::SerializeHeader)?;
        let header = Self {
            schema_version: PM_PUBLIC_CAPTURE_SCHEMA_VERSION,
            product: PM_PUBLIC_CAPTURE_PRODUCT.to_string(),
            configuration_sha256: sha256_hex(&configuration_bytes),
            structural_scope_sha256: sha256_hex(&scope_bytes),
            scope,
            session_policy,
            provenance,
            authenticated: false,
            production_order_entry_authorized: false,
        };
        validate_header(&header)?;
        Ok(header)
    }

    #[must_use]
    pub const fn scope(&self) -> &PmCaptureScope {
        &self.scope
    }

    #[must_use]
    pub fn configuration_sha256(&self) -> &str {
        &self.configuration_sha256
    }

    #[must_use]
    pub fn structural_scope_sha256(&self) -> &str {
        &self.structural_scope_sha256
    }

    #[must_use]
    pub const fn session_policy(&self) -> PmCaptureSessionPolicy {
        self.session_policy
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case", deny_unknown_fields)]
pub enum PmCaptureLifecycle {
    ConnectionStarted,
    SubscriptionSent,
    HeartbeatPingSent,
    Disconnected {
        local_wall_receive_ns: u64,
        reason: PmCaptureDisconnectReason,
    },
    ReconnectScheduled {
        next_epoch: ConnectionEpoch,
        delay_ns: u64,
    },
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case", deny_unknown_fields)]
pub enum PmCaptureDisconnectReason {
    Disconnect,
    Gap,
    Overflow,
    Stale,
    HeartbeatTimeout,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case", deny_unknown_fields)]
pub enum OkxCaptureLifecycle {
    ConnectionStarted,
    SubscriptionSent,
    Disconnected {
        local_wall_receive_ns: u64,
        reason: OkxCaptureDisconnectReason,
    },
    ReconnectScheduled {
        next_epoch: u64,
        delay_ns: u64,
    },
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case", deny_unknown_fields)]
pub enum OkxCaptureDisconnectReason {
    Disconnect,
    Overflow,
    Stale,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct PmRawPublicFrame {
    source: PmProductSource,
    connection_id: PmConnectionId,
    connection_epoch: ConnectionEpoch,
    local_ingress_sequence: IngressSequence,
    local_wall_receive_ns: u64,
    monotonic_receive_ns: u64,
    outcome_token: PmTokenId,
    raw_sha256: String,
    raw_length: u32,
    raw_base64: String,
}

impl PmRawPublicFrame {
    #[allow(clippy::too_many_arguments)]
    fn new(
        scope: &PmCaptureScope,
        connection_epoch: ConnectionEpoch,
        local_ingress_sequence: IngressSequence,
        local_wall_receive_ns: u64,
        monotonic_receive_ns: u64,
        raw_bytes: &[u8],
    ) -> Result<Self, PmCaptureVerifyError> {
        if raw_bytes.len() > MAX_PM_RAW_PUBLIC_FRAME_BYTES {
            return Err(PmCaptureVerifyError::RawFrameTooLarge);
        }
        if raw_bytes.is_empty() {
            return Err(PmCaptureVerifyError::InvalidRawFrame("raw frame is empty"));
        }
        if connection_epoch.value() == 0
            || local_ingress_sequence.value() == 0
            || local_wall_receive_ns == 0
            || monotonic_receive_ns == 0
        {
            return Err(PmCaptureVerifyError::InvalidRawFrame(
                "raw frame ordering and clocks must be positive",
            ));
        }
        Ok(Self {
            source: scope.source,
            connection_id: scope.connection_id,
            connection_epoch,
            local_ingress_sequence,
            local_wall_receive_ns,
            monotonic_receive_ns,
            outcome_token: scope.outcome_token,
            raw_sha256: sha256_hex(raw_bytes),
            raw_length: u32::try_from(raw_bytes.len()).expect("one-MiB raw frame length"),
            raw_base64: BASE64.encode(raw_bytes),
        })
    }

    #[must_use]
    pub const fn connection_epoch(&self) -> ConnectionEpoch {
        self.connection_epoch
    }

    #[must_use]
    pub const fn local_ingress_sequence(&self) -> IngressSequence {
        self.local_ingress_sequence
    }

    #[must_use]
    pub const fn local_wall_receive_ns(&self) -> u64 {
        self.local_wall_receive_ns
    }

    #[must_use]
    pub const fn monotonic_receive_ns(&self) -> u64 {
        self.monotonic_receive_ns
    }

    pub fn decode_raw(&self) -> Result<Vec<u8>, PmCaptureVerifyError> {
        let decoded = BASE64
            .decode(self.raw_base64.as_bytes())
            .map_err(|_| PmCaptureVerifyError::InvalidRawFrame("invalid base64 payload"))?;
        if decoded.len() != self.raw_length as usize
            || decoded.is_empty()
            || decoded.len() > MAX_PM_RAW_PUBLIC_FRAME_BYTES
            || self.raw_sha256 != sha256_hex(&decoded)
        {
            return Err(PmCaptureVerifyError::InvalidRawFrame(
                "raw length or SHA-256 mismatched",
            ));
        }
        Ok(decoded)
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct OkxRawPublicFrame {
    source: PmProductSource,
    connection_id: PmConnectionId,
    reference_instrument: OkxReferenceInstrument,
    connection_epoch: u64,
    local_ingress_sequence: u64,
    local_wall_receive_ns: u64,
    monotonic_receive_ns: u64,
    raw_hash: u64,
    raw_sha256: String,
    raw_length: u32,
    raw_base64: String,
}

impl OkxRawPublicFrame {
    #[allow(clippy::too_many_arguments)]
    fn new(
        scope: &PmCaptureScope,
        connection_epoch: u64,
        local_ingress_sequence: u64,
        local_wall_receive_ns: u64,
        monotonic_receive_ns: u64,
        raw_bytes: &[u8],
    ) -> Result<Self, PmCaptureVerifyError> {
        if raw_bytes.len() > MAX_PM_RAW_PUBLIC_FRAME_BYTES {
            return Err(PmCaptureVerifyError::RawFrameTooLarge);
        }
        if raw_bytes.is_empty() {
            return Err(PmCaptureVerifyError::InvalidRawFrame(
                "OKX raw frame is empty",
            ));
        }
        if connection_epoch == 0
            || local_ingress_sequence == 0
            || local_wall_receive_ns == 0
            || monotonic_receive_ns == 0
        {
            return Err(PmCaptureVerifyError::InvalidRawFrame(
                "OKX raw frame ordering and clocks must be positive",
            ));
        }
        let (raw_sha256, raw_hash) = raw_sha256_and_hash(raw_bytes);
        Ok(Self {
            source: scope.okx_source,
            connection_id: scope.okx_connection_id,
            reference_instrument: scope.okx_reference_instrument,
            connection_epoch,
            local_ingress_sequence,
            local_wall_receive_ns,
            monotonic_receive_ns,
            raw_hash,
            raw_sha256,
            raw_length: u32::try_from(raw_bytes.len()).expect("one-MiB raw frame length"),
            raw_base64: BASE64.encode(raw_bytes),
        })
    }

    pub fn decode_raw(&self) -> Result<Vec<u8>, PmCaptureVerifyError> {
        let decoded = BASE64
            .decode(self.raw_base64.as_bytes())
            .map_err(|_| PmCaptureVerifyError::InvalidRawFrame("invalid OKX base64 payload"))?;
        let (raw_sha256, raw_hash) = raw_sha256_and_hash(&decoded);
        if decoded.len() != self.raw_length as usize
            || decoded.is_empty()
            || decoded.len() > MAX_PM_RAW_PUBLIC_FRAME_BYTES
            || self.raw_sha256 != raw_sha256
            || self.raw_hash != raw_hash
        {
            return Err(PmCaptureVerifyError::InvalidRawFrame(
                "OKX raw length or SHA-256 mismatched",
            ));
        }
        Ok(decoded)
    }

    #[must_use]
    pub const fn connection_epoch(&self) -> u64 {
        self.connection_epoch
    }

    #[must_use]
    pub const fn local_wall_receive_ns(&self) -> u64 {
        self.local_wall_receive_ns
    }

    #[must_use]
    pub const fn monotonic_receive_ns(&self) -> u64 {
        self.monotonic_receive_ns
    }

    #[must_use]
    pub const fn raw_hash(&self) -> u64 {
        self.raw_hash
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "record_type", rename_all = "snake_case", deny_unknown_fields)]
pub enum PmPublicCaptureRecord {
    Header {
        sequence: u64,
        header: Box<PmCaptureHeader>,
    },
    RawPublicFrame {
        sequence: u64,
        frame: PmRawPublicFrame,
    },
    OkxRawPublicFrame {
        sequence: u64,
        frame: OkxRawPublicFrame,
    },
    Lifecycle {
        sequence: u64,
        source: PmProductSource,
        connection_id: PmConnectionId,
        connection_epoch: ConnectionEpoch,
        monotonic_ns: u64,
        event: PmCaptureLifecycle,
    },
    OkxLifecycle {
        sequence: u64,
        source: PmProductSource,
        connection_id: PmConnectionId,
        connection_epoch: u64,
        monotonic_ns: u64,
        event: OkxCaptureLifecycle,
    },
    FreshnessTimer {
        sequence: u64,
        monotonic_ns: u64,
    },
}

impl PmPublicCaptureRecord {
    const fn sequence(&self) -> u64 {
        match self {
            Self::Header { sequence, .. }
            | Self::RawPublicFrame { sequence, .. }
            | Self::OkxRawPublicFrame { sequence, .. }
            | Self::Lifecycle { sequence, .. }
            | Self::OkxLifecycle { sequence, .. }
            | Self::FreshnessTimer { sequence, .. } => *sequence,
        }
    }

    const fn monotonic_ns(&self) -> Option<u64> {
        match self {
            Self::Header { .. } => None,
            Self::RawPublicFrame { frame, .. } => Some(frame.monotonic_receive_ns),
            Self::OkxRawPublicFrame { frame, .. } => Some(frame.monotonic_receive_ns),
            Self::Lifecycle { monotonic_ns, .. }
            | Self::OkxLifecycle { monotonic_ns, .. }
            | Self::FreshnessTimer { monotonic_ns, .. } => Some(*monotonic_ns),
        }
    }
}

#[derive(Debug, Error)]
pub enum PmCaptureWriteError {
    #[error(transparent)]
    Framing(#[from] ByteBoundedWriterError),
    #[error(transparent)]
    Frame(#[from] BoundedJsonlFrameError),
    #[error(transparent)]
    Contract(#[from] PmCaptureVerifyError),
    #[error("oldest pending capture record age {observed_age_ns}ns exceeded {maximum_age_ns}ns")]
    CaptureAged {
        observed_age_ns: u64,
        maximum_age_ns: u64,
    },
    #[error(
        "capture writer entry evidence exceeded its tracked timestamp FIFO: writer depth {writer_depth}, tracked depth {tracked_depth}"
    )]
    CaptureQueueEvidenceMismatch {
        writer_depth: usize,
        tracked_depth: usize,
    },
    #[error(
        "capture writer service clock {observed_monotonic_ns}ns regressed behind oldest pending record {oldest_monotonic_ns}ns"
    )]
    CaptureQueueClockRegression {
        observed_monotonic_ns: u64,
        oldest_monotonic_ns: u64,
    },
}

impl PmCaptureWriteError {
    pub(crate) fn session_fault(&self) -> PmPublicSessionFault {
        if matches!(self, Self::CaptureAged { .. }) {
            PmPublicSessionFault::Stale
        } else if self.is_capacity_failure() {
            PmPublicSessionFault::Overflow
        } else {
            PmPublicSessionFault::InvalidTransition
        }
    }

    pub(crate) fn okx_session_fault(&self) -> OkxPublicSessionFault {
        if matches!(self, Self::CaptureAged { .. }) {
            OkxPublicSessionFault::Stale
        } else if self.is_capacity_failure() {
            OkxPublicSessionFault::Overflow
        } else {
            OkxPublicSessionFault::InvalidTransition
        }
    }

    fn is_capacity_failure(&self) -> bool {
        let capacity_contract = matches!(
            self,
            Self::Contract(
                PmCaptureVerifyError::TooManyRecords
                    | PmCaptureVerifyError::TooManyRawFrames
                    | PmCaptureVerifyError::RawPayloadTooLarge
                    | PmCaptureVerifyError::CaptureTooLarge
                    | PmCaptureVerifyError::RawFrameTooLarge
            )
        );
        let capacity_frame = matches!(
            self,
            Self::Frame(BoundedJsonlFrameError::FrameTooLarge { .. })
                | Self::Framing(
                    ByteBoundedWriterError::FrameTooLarge { .. }
                        | ByteBoundedWriterError::ByteBackpressure { .. }
                        | ByteBoundedWriterError::Writer(JsonlWriterError::Backpressure { .. })
                )
        );
        capacity_contract || capacity_frame
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct PmCaptureVerification {
    pub schema_version: u16,
    pub product: String,
    pub artifact_sha256: String,
    pub structural_scope_sha256: String,
    pub records: u64,
    pub bytes: u64,
    pub raw_public_frames: u64,
    pub okx_raw_public_frames: u64,
    pub raw_payload_bytes: u64,
    pub lifecycle_records: u64,
    pub okx_lifecycle_records: u64,
    pub freshness_timers: u64,
    pub production_order_entry_authorized: bool,
}

#[derive(Debug, Error)]
pub enum PmCaptureVerifyError {
    #[error("invalid PM public capture header: {0}")]
    InvalidHeader(&'static str),
    #[error("failed to serialize PM public capture header: {0}")]
    SerializeHeader(serde_json::Error),
    #[error("recorded authoritative metadata is invalid: {0}")]
    Metadata(#[source] PmMetadataJoinError),
    #[error("invalid PM public capture provenance")]
    InvalidProvenance,
    #[error("invalid PM raw public frame: {0}")]
    InvalidRawFrame(&'static str),
    #[error("PM raw public frame exceeds one MiB")]
    RawFrameTooLarge,
    #[error("invalid PM public lifecycle record")]
    InvalidLifecycle,
    #[error("invalid PM public freshness timer")]
    InvalidFreshnessTimer,
    #[error("PM public capture exceeds its 16384 total-record bound")]
    TooManyRecords,
    #[error("PM public capture exceeds 8192 raw public frames")]
    TooManyRawFrames,
    #[error("PM public capture raw payload total exceeds 32 MiB")]
    RawPayloadTooLarge,
    #[error("PM public capture encoded file exceeds 48 MiB")]
    CaptureTooLarge,
    #[error("PM public capture is malformed at record {0}")]
    MalformedRecord(u64),
    #[error("PM public capture sequence is not contiguous at record {0}")]
    NonContiguousSequence(u64),
    #[error("PM public capture must begin with the one expected header")]
    HeaderMismatch,
    #[error("PM public capture record {0} is outside the configured route or token scope")]
    ScopeMismatch(u64),
    #[error("PM public capture raw hash mismatched at record {0}")]
    RawHashMismatch(u64),
    #[error("PM public capture local ingress is not contiguous within epoch at record {0}")]
    IngressMismatch(u64),
    #[error("PM public capture has a trailing partial record")]
    TrailingPartialRecord,
    #[error("PM public capture changed while it was being verified")]
    ChangedWhileReading,
    #[error("PM public capture contains invalid records")]
    InvalidRecords,
    #[error(transparent)]
    Framing(#[from] JsonlVerifyError),
}

fn valid_digest(value: &str) -> bool {
    value.len() == DIGEST_HEX_BYTES
        && value
            .bytes()
            .all(|byte| byte.is_ascii_hexdigit() && !byte.is_ascii_uppercase())
        && value.bytes().any(|byte| byte != b'0')
}

fn bytes_to_hex(bytes: &[u8]) -> String {
    bytes.iter().map(|byte| format!("{byte:02x}")).collect()
}

fn raw_sha256_and_hash(bytes: &[u8]) -> (String, u64) {
    let digest = Sha256::digest(bytes);
    let mut prefix = [0_u8; 8];
    prefix.copy_from_slice(&digest[..8]);
    (bytes_to_hex(&digest), u64::from_be_bytes(prefix))
}

fn decode_digest(value: &str) -> Result<[u8; 32], PmCaptureVerifyError> {
    if !valid_digest(value) {
        return Err(PmCaptureVerifyError::InvalidHeader(
            "metadata digest is invalid",
        ));
    }
    let mut output = [0_u8; 32];
    for (index, byte) in output.iter_mut().enumerate() {
        let high = decode_hex(value.as_bytes()[index * 2])?;
        let low = decode_hex(value.as_bytes()[index * 2 + 1])?;
        *byte = (high << 4) | low;
    }
    Ok(output)
}

fn decode_hex(byte: u8) -> Result<u8, PmCaptureVerifyError> {
    match byte {
        b'0'..=b'9' => Ok(byte - b'0'),
        b'a'..=b'f' => Ok(byte - b'a' + 10),
        _ => Err(PmCaptureVerifyError::InvalidHeader(
            "digest is not lowercase hexadecimal",
        )),
    }
}
