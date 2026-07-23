use reap_pm_core::{
    CLOB_V2_LOT_UNITS, PM_PROTOCOL_SCALE, PmInstrumentHandle, PmMarketMetadata, SnapshotRevision,
};
use thiserror::Error;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct PmMetadataFingerprint([u8; 32]);

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct PmDomainFingerprint([u8; 32]);

#[derive(Debug, Clone, Copy, PartialEq, Eq, Error)]
pub enum PmMetadataContractError {
    #[error("metadata fingerprint must not be all zeroes")]
    ZeroMetadataFingerprint,
    #[error("domain fingerprint must not be all zeroes")]
    ZeroDomainFingerprint,
    #[error("metadata unit values must all be positive")]
    ZeroUnit,
    #[error("metadata freshness limits must both be positive")]
    ZeroFreshnessLimit,
    #[error("metadata revision must be positive")]
    ZeroMetadataRevision,
    #[error("metadata observation time must be positive")]
    ZeroObservationTime,
}

impl PmMetadataFingerprint {
    pub fn new(bytes: [u8; 32]) -> Result<Self, PmMetadataContractError> {
        if bytes == [0; 32] {
            Err(PmMetadataContractError::ZeroMetadataFingerprint)
        } else {
            Ok(Self(bytes))
        }
    }

    #[must_use]
    pub const fn bytes(self) -> [u8; 32] {
        self.0
    }
}

impl PmDomainFingerprint {
    pub fn new(bytes: [u8; 32]) -> Result<Self, PmMetadataContractError> {
        if bytes == [0; 32] {
            Err(PmMetadataContractError::ZeroDomainFingerprint)
        } else {
            Ok(Self(bytes))
        }
    }

    #[must_use]
    pub const fn bytes(self) -> [u8; 32] {
        self.0
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum PmProtocolProfile {
    ClobV2,
    Unsupported(u16),
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct PmUnitContract {
    price_units_per_one: u32,
    quantity_units_per_one: u32,
    collateral_units_per_one: u32,
    lot_units: u32,
}

impl PmUnitContract {
    pub fn new(
        price_units_per_one: u32,
        quantity_units_per_one: u32,
        collateral_units_per_one: u32,
        lot_units: u32,
    ) -> Result<Self, PmMetadataContractError> {
        if price_units_per_one == 0
            || quantity_units_per_one == 0
            || collateral_units_per_one == 0
            || lot_units == 0
        {
            return Err(PmMetadataContractError::ZeroUnit);
        }
        Ok(Self {
            price_units_per_one,
            quantity_units_per_one,
            collateral_units_per_one,
            lot_units,
        })
    }

    #[must_use]
    pub const fn goal_f_clob_v2() -> Self {
        Self {
            price_units_per_one: PM_PROTOCOL_SCALE,
            quantity_units_per_one: PM_PROTOCOL_SCALE,
            collateral_units_per_one: PM_PROTOCOL_SCALE,
            lot_units: CLOB_V2_LOT_UNITS,
        }
    }

    #[must_use]
    pub const fn price_units_per_one(self) -> u32 {
        self.price_units_per_one
    }

    #[must_use]
    pub const fn quantity_units_per_one(self) -> u32 {
        self.quantity_units_per_one
    }

    #[must_use]
    pub const fn collateral_units_per_one(self) -> u32 {
        self.collateral_units_per_one
    }

    #[must_use]
    pub const fn lot_units(self) -> u32 {
        self.lot_units
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct PmMetadataContract {
    market: PmMarketMetadata,
    protocol: PmProtocolProfile,
    units: PmUnitContract,
    domain: PmDomainFingerprint,
}

impl PmMetadataContract {
    #[must_use]
    pub const fn new(
        market: PmMarketMetadata,
        protocol: PmProtocolProfile,
        units: PmUnitContract,
        domain: PmDomainFingerprint,
    ) -> Self {
        Self {
            market,
            protocol,
            units,
            domain,
        }
    }

    #[must_use]
    pub const fn goal_f_clob_v2(market: PmMarketMetadata, domain: PmDomainFingerprint) -> Self {
        Self::new(
            market,
            PmProtocolProfile::ClobV2,
            PmUnitContract::goal_f_clob_v2(),
            domain,
        )
    }

    #[must_use]
    pub const fn market(self) -> PmMarketMetadata {
        self.market
    }

    #[must_use]
    pub const fn protocol(self) -> PmProtocolProfile {
        self.protocol
    }

    #[must_use]
    pub const fn units(self) -> PmUnitContract {
        self.units
    }

    #[must_use]
    pub const fn domain(self) -> PmDomainFingerprint {
        self.domain
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct PmMetadataObservation {
    instrument: PmInstrumentHandle,
    revision: SnapshotRevision,
    fingerprint: PmMetadataFingerprint,
    contract: PmMetadataContract,
    monotonic_receive_ns: u64,
}

impl PmMetadataObservation {
    pub fn new(
        instrument: PmInstrumentHandle,
        revision: SnapshotRevision,
        fingerprint: PmMetadataFingerprint,
        contract: PmMetadataContract,
        monotonic_receive_ns: u64,
    ) -> Result<Self, PmMetadataContractError> {
        if revision.value() == 0 {
            return Err(PmMetadataContractError::ZeroMetadataRevision);
        }
        if monotonic_receive_ns == 0 {
            return Err(PmMetadataContractError::ZeroObservationTime);
        }
        Ok(Self {
            instrument,
            revision,
            fingerprint,
            contract,
            monotonic_receive_ns,
        })
    }

    #[must_use]
    pub const fn instrument(self) -> PmInstrumentHandle {
        self.instrument
    }

    #[must_use]
    pub const fn revision(self) -> SnapshotRevision {
        self.revision
    }

    #[must_use]
    pub const fn fingerprint(self) -> PmMetadataFingerprint {
        self.fingerprint
    }

    #[must_use]
    pub const fn contract(self) -> PmMetadataContract {
        self.contract
    }

    #[must_use]
    pub const fn monotonic_receive_ns(self) -> u64 {
        self.monotonic_receive_ns
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct PmBookFreshness {
    metadata_max_age_ns: u64,
    book_max_age_ns: u64,
}

impl PmBookFreshness {
    pub fn new(
        metadata_max_age_ns: u64,
        book_max_age_ns: u64,
    ) -> Result<Self, PmMetadataContractError> {
        if metadata_max_age_ns == 0 || book_max_age_ns == 0 {
            Err(PmMetadataContractError::ZeroFreshnessLimit)
        } else {
            Ok(Self {
                metadata_max_age_ns,
                book_max_age_ns,
            })
        }
    }

    #[must_use]
    pub const fn metadata_max_age_ns(self) -> u64 {
        self.metadata_max_age_ns
    }

    #[must_use]
    pub const fn book_max_age_ns(self) -> u64 {
        self.book_max_age_ns
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum PmMetadataDrift {
    Instrument,
    Identity,
    OutcomeLabel,
    Protocol,
    Units,
    Lot,
    Grid,
    Minimum,
    NegativeRisk,
    Domain,
    RequiredSpenders,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Error)]
pub enum PmPublicReadinessReason {
    #[error("authoritative metadata has not been accepted")]
    MetadataMissing,
    #[error("metadata fingerprint differs from the configured fingerprint")]
    MetadataFingerprintMismatch,
    #[error("metadata revision did not advance")]
    MetadataRevisionNotIncreasing,
    #[error("metadata drifted from the configured contract: {0:?}")]
    MetadataDrift(PmMetadataDrift),
    #[error("market is inactive")]
    MarketInactive,
    #[error("market is closed")]
    MarketClosed,
    #[error("market is archived")]
    MarketArchived,
    #[error("market is not accepting orders")]
    OrdersNotAccepted,
    #[error("market order book is disabled")]
    OrderBookDisabled,
    #[error("public connection is unavailable")]
    ConnectionUnavailable,
    #[error("a complete current snapshot has not been committed")]
    SnapshotMissing,
    #[error("connection epoch is invalid or did not advance exactly once")]
    ConnectionEpochInvalid,
    #[error("book event belongs to another connection epoch")]
    ConnectionEpochMismatch,
    #[error("book event names a stale or different metadata revision")]
    MetadataRevisionMismatch,
    #[error("book event names an invalid snapshot revision")]
    SnapshotRevisionMismatch,
    #[error("local ingress sequence was duplicated")]
    DuplicateIngress,
    #[error("local ingress sequence moved backwards")]
    ReorderedIngress,
    #[error("book input exceeds the fixed level bound")]
    TooManyLevels,
    #[error("snapshot contains an explicit delete")]
    DeleteInSnapshot,
    #[error("book batch contains a duplicate side/price key")]
    DuplicateLevel,
    #[error("book price is off the current metadata tick")]
    PriceOffTick,
    #[error("book snapshot or delta leaves an empty side")]
    EmptyBook,
    #[error("book snapshot or delta is crossed")]
    CrossedBook,
    #[error("delta attempted to delete a level that does not exist")]
    MissingDeleteLevel,
    #[error("reduced best bid/ask does not match canonical full-book state")]
    BboMismatch,
    #[error("venue integrity validation failed")]
    HashMismatch,
    #[error("public stream disconnected")]
    Disconnected,
    #[error("public stream heartbeat timed out")]
    HeartbeatTimeout,
    #[error("public stream has an explicit integrity gap")]
    Gap,
    #[error("public stream or capture overflowed")]
    Overflow,
    #[error("public lane overflow is pending exact lifecycle enactment")]
    PendingOverflow,
    #[error("public-lane staleness is pending exact lifecycle enactment")]
    PendingBookStale,
    #[error("public stream made an invalid transition")]
    InvalidTransition,
    #[error("metadata is stale")]
    MetadataStale,
    #[error("book is stale")]
    BookStale,
    #[error("monotonic freshness clock regressed")]
    ClockRegression,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct PmBookReadiness {
    reason: Option<PmPublicReadinessReason>,
    metadata_revision: Option<SnapshotRevision>,
    snapshot_revision: Option<SnapshotRevision>,
}

impl PmBookReadiness {
    pub(crate) const fn unavailable(
        reason: PmPublicReadinessReason,
        metadata_revision: Option<SnapshotRevision>,
        snapshot_revision: Option<SnapshotRevision>,
    ) -> Self {
        Self {
            reason: Some(reason),
            metadata_revision,
            snapshot_revision,
        }
    }

    pub(crate) const fn ready(
        metadata_revision: SnapshotRevision,
        snapshot_revision: SnapshotRevision,
    ) -> Self {
        Self {
            reason: None,
            metadata_revision: Some(metadata_revision),
            snapshot_revision: Some(snapshot_revision),
        }
    }

    #[must_use]
    pub const fn is_ready(self) -> bool {
        self.reason.is_none()
    }

    #[must_use]
    pub const fn reason(self) -> Option<PmPublicReadinessReason> {
        self.reason
    }

    #[must_use]
    pub const fn metadata_revision(self) -> Option<SnapshotRevision> {
        self.metadata_revision
    }

    #[must_use]
    pub const fn snapshot_revision(self) -> Option<SnapshotRevision> {
        self.snapshot_revision
    }
}
