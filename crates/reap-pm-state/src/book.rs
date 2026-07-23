use std::sync::atomic::{AtomicU64, Ordering as AtomicOrdering};

use reap_pm_core::{
    ConnectionEpoch, IngressSequence, MAX_PM_BOOK_LEVELS, PmBookDeltaBatch, PmBookLevel,
    PmBookQuantity, PmBookSnapshot, PmBookUpdate, PmInstrumentHandle, SnapshotRevision,
    VenueEventHash, VenueEventHashAlgorithm,
};

use crate::readiness::{
    PmBookFreshness, PmBookReadiness, PmMetadataContract, PmMetadataDrift, PmMetadataFingerprint,
    PmMetadataObservation, PmPublicReadinessReason,
};

mod canonical;
mod metadata;

use canonical::{BookKey, apply_change, canonical_top, compare_keys, validate_and_sort_levels};
use metadata::{validate_expected_contract, validate_observed_contract};

pub use reap_pm_core::PmBookTopCheck;

const MAX_LEVELS: usize = MAX_PM_BOOK_LEVELS as usize;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
/// Ordering and integrity evidence supplied by the already route-checked
/// public ingress boundary.
///
/// `venue_hash` is contextual: snapshots require one verified 20-byte SHA-1;
/// delta batches, top checks, and tick changes require `None` because their
/// raw integrity evidence is not one synthetic frame hash.
pub struct PmBookBatchEvidence {
    instrument: PmInstrumentHandle,
    connection_epoch: ConnectionEpoch,
    metadata_revision: SnapshotRevision,
    snapshot_revision: SnapshotRevision,
    local_ingress_sequence: IngressSequence,
    monotonic_receive_ns: u64,
    venue_hash: Option<VenueEventHash>,
}

impl PmBookBatchEvidence {
    #[allow(clippy::too_many_arguments)]
    pub fn new(
        instrument: PmInstrumentHandle,
        connection_epoch: ConnectionEpoch,
        metadata_revision: SnapshotRevision,
        snapshot_revision: SnapshotRevision,
        local_ingress_sequence: IngressSequence,
        monotonic_receive_ns: u64,
        venue_hash: Option<VenueEventHash>,
    ) -> Result<Self, PmPublicReadinessReason> {
        if connection_epoch.value() == 0
            || metadata_revision.value() == 0
            || snapshot_revision.value() == 0
            || local_ingress_sequence.value() == 0
            || monotonic_receive_ns == 0
        {
            return Err(PmPublicReadinessReason::InvalidTransition);
        }
        Ok(Self {
            instrument,
            connection_epoch,
            metadata_revision,
            snapshot_revision,
            local_ingress_sequence,
            monotonic_receive_ns,
            venue_hash,
        })
    }

    #[must_use]
    pub const fn instrument(self) -> PmInstrumentHandle {
        self.instrument
    }

    #[must_use]
    pub const fn connection_epoch(self) -> ConnectionEpoch {
        self.connection_epoch
    }

    #[must_use]
    pub const fn metadata_revision(self) -> SnapshotRevision {
        self.metadata_revision
    }

    #[must_use]
    pub const fn snapshot_revision(self) -> SnapshotRevision {
        self.snapshot_revision
    }

    #[must_use]
    pub const fn local_ingress_sequence(self) -> IngressSequence {
        self.local_ingress_sequence
    }

    #[must_use]
    pub const fn monotonic_receive_ns(self) -> u64 {
        self.monotonic_receive_ns
    }

    #[must_use]
    pub const fn venue_hash(self) -> Option<VenueEventHash> {
        self.venue_hash
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum PmExternalBookFault {
    Disconnect,
    HeartbeatTimeout,
    Gap,
    Overflow,
    BacklogAged,
    InvalidTransition,
    HashMismatch,
}

/// Opaque, process-local authority for finalizing one reducer fault that was
/// made unavailable before an external lifecycle write.
///
/// This token is move-only, has no public constructor, and is never serialized
/// into capture or replay evidence.
pub struct PmPendingExternalBookFaultAuthority {
    reducer_authority: PmBookReducerAuthorityId,
    generation: u64,
    epoch: ConnectionEpoch,
    fault: PmExternalBookFault,
}

impl std::fmt::Debug for PmPendingExternalBookFaultAuthority {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter.write_str("PmPendingExternalBookFaultAuthority(<opaque>)")
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
struct PendingExternalBookFault {
    generation: u64,
    epoch: ConnectionEpoch,
    fault: PmExternalBookFault,
}

#[derive(Debug, PartialEq, Eq)]
pub enum PmBookTransition {
    MetadataAccepted {
        revision: SnapshotRevision,
    },
    EpochStarted {
        epoch: ConnectionEpoch,
    },
    SnapshotCommitted {
        revision: SnapshotRevision,
        levels: u16,
        proof: PmSnapshotCommitProof,
    },
    DeltaBatchCommitted {
        revision: SnapshotRevision,
        changes: u16,
    },
    TopConfirmed,
    FreshnessConfirmed,
}

/// Move-only proof that the canonical reducer committed one exact snapshot.
///
/// Construction is private to [`PmBookReducer::apply_snapshot`]. The
/// composition owner inspects it only inside the atomic operation that
/// consumes the route-issued snapshot, reduces that exact payload, and opens
/// the session's correlated flow. The proof by itself grants no session or
/// product authority.
#[derive(Debug, PartialEq, Eq)]
pub struct PmSnapshotCommitProof {
    instrument: PmInstrumentHandle,
    metadata_fingerprint: PmMetadataFingerprint,
    connection_epoch: ConnectionEpoch,
    metadata_revision: SnapshotRevision,
    snapshot_revision: SnapshotRevision,
    local_ingress_sequence: IngressSequence,
    venue_hash: VenueEventHash,
}

impl PmSnapshotCommitProof {
    #[must_use]
    pub const fn instrument(&self) -> PmInstrumentHandle {
        self.instrument
    }

    #[must_use]
    pub const fn metadata_fingerprint(&self) -> PmMetadataFingerprint {
        self.metadata_fingerprint
    }

    #[must_use]
    pub const fn connection_epoch(&self) -> ConnectionEpoch {
        self.connection_epoch
    }

    #[must_use]
    pub const fn metadata_revision(&self) -> SnapshotRevision {
        self.metadata_revision
    }

    #[must_use]
    pub const fn snapshot_revision(&self) -> SnapshotRevision {
        self.snapshot_revision
    }

    #[must_use]
    pub const fn local_ingress_sequence(&self) -> IngressSequence {
        self.local_ingress_sequence
    }

    #[must_use]
    pub const fn venue_hash(&self) -> VenueEventHash {
        self.venue_hash
    }
}

#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct PmBookCounters {
    pub metadata_inputs: u64,
    pub metadata_accepted: u64,
    pub metadata_rejected: u64,
    pub epoch_attempts: u64,
    pub epochs_started: u64,
    pub reconnects: u64,
    pub snapshot_attempts: u64,
    pub snapshots_committed: u64,
    pub snapshot_levels_committed: u64,
    pub resync_snapshots: u64,
    pub delta_batch_attempts: u64,
    pub delta_batches_committed: u64,
    pub delta_changes_committed: u64,
    pub delta_top_checks: u64,
    pub delta_top_checks_confirmed: u64,
    pub top_checks: u64,
    pub top_checks_confirmed: u64,
    pub freshness_checks: u64,
    pub freshness_confirmed: u64,
    pub tick_size_changes: u64,
    pub external_faults: u64,
    pub duplicate_ingress: u64,
    pub reordered_ingress: u64,
    pub disconnects: u64,
    pub heartbeat_timeouts: u64,
    pub backlog_aged_faults: u64,
    pub gaps: u64,
    pub overflows: u64,
    pub hash_mismatches: u64,
    pub bbo_mismatches: u64,
    pub invalid_transitions: u64,
    pub clock_regressions: u64,
    pub invalidations: u64,
    pub unavailable_transitions: u64,
    pub stale_invalidations: u64,
}

/// Pure, single-owner public PM book and readiness reducer.
///
/// The reducer deliberately has no venue predecessor-sequence field. A jump
/// in local ingress sequence is legal; only duplicate or regressing local
/// order is rejected. Venue gap knowledge enters through
/// [`PmExternalBookFault::Gap`].
#[derive(Debug)]
pub struct PmBookReducer {
    authority_id: PmBookReducerAuthorityId,
    instrument: PmInstrumentHandle,
    expected_fingerprint: PmMetadataFingerprint,
    expected_contract: PmMetadataContract,
    freshness: PmBookFreshness,
    metadata_revision: Option<SnapshotRevision>,
    metadata_receive_ns: Option<u64>,
    metadata_valid: bool,
    connection_epoch: Option<ConnectionEpoch>,
    requires_reconnect: bool,
    last_ingress_sequence: Option<IngressSequence>,
    last_ingress_receive_ns: Option<u64>,
    snapshot_revision: Option<SnapshotRevision>,
    last_committed_snapshot_revision: Option<SnapshotRevision>,
    book_receive_ns: Option<u64>,
    last_verified_snapshot_hash: Option<VenueEventHash>,
    levels: Vec<PmBookLevel>,
    staging: Vec<PmBookLevel>,
    change_keys: Vec<BookKey>,
    book_ready: bool,
    reason: PmPublicReadinessReason,
    pending_external_fault: Option<PendingExternalBookFault>,
    pending_external_fault_generation: u64,
    counters: PmBookCounters,
    ever_committed_snapshot: bool,
}

impl PmBookReducer {
    pub fn new(
        instrument: PmInstrumentHandle,
        expected_fingerprint: PmMetadataFingerprint,
        expected_contract: PmMetadataContract,
        freshness: PmBookFreshness,
    ) -> Result<Self, PmPublicReadinessReason> {
        validate_expected_contract(expected_contract)?;
        Ok(Self {
            authority_id: PmBookReducerAuthorityId::allocate()?,
            instrument,
            expected_fingerprint,
            expected_contract,
            freshness,
            metadata_revision: None,
            metadata_receive_ns: None,
            metadata_valid: false,
            connection_epoch: None,
            requires_reconnect: false,
            last_ingress_sequence: None,
            last_ingress_receive_ns: None,
            snapshot_revision: None,
            last_committed_snapshot_revision: None,
            book_receive_ns: None,
            last_verified_snapshot_hash: None,
            levels: Vec::with_capacity(MAX_LEVELS),
            staging: Vec::with_capacity(MAX_LEVELS),
            change_keys: Vec::with_capacity(MAX_LEVELS),
            book_ready: false,
            reason: PmPublicReadinessReason::MetadataMissing,
            pending_external_fault: None,
            pending_external_fault_generation: 0,
            counters: PmBookCounters::default(),
            ever_committed_snapshot: false,
        })
    }

    /// Opaque in-process identity used to bind one active capture root to one
    /// concrete product reducer. It is never serialized into capture/replay
    /// evidence.
    #[must_use]
    pub const fn authority_id(&self) -> PmBookReducerAuthorityId {
        self.authority_id
    }

    #[must_use]
    pub const fn instrument(&self) -> PmInstrumentHandle {
        self.instrument
    }

    /// Returns the immutable metadata identity configured for this reducer.
    #[must_use]
    pub const fn expected_metadata_fingerprint(&self) -> PmMetadataFingerprint {
        self.expected_fingerprint
    }

    /// Returns the immutable market/protocol/unit/domain contract configured
    /// for this reducer.
    #[must_use]
    pub const fn expected_metadata_contract(&self) -> PmMetadataContract {
        self.expected_contract
    }

    #[must_use]
    pub fn levels(&self) -> &[PmBookLevel] {
        &self.levels
    }

    #[must_use]
    pub const fn connection_epoch(&self) -> Option<ConnectionEpoch> {
        self.connection_epoch
    }

    /// Returns the exact route-issued ingress sequence last admitted by the
    /// reducer. Composition owners use this only to verify an atomic commit;
    /// it does not grant ingress authority.
    #[must_use]
    pub const fn last_ingress_sequence(&self) -> Option<IngressSequence> {
        self.last_ingress_sequence
    }

    /// Returns the monotonic receive clock paired with the last admitted
    /// ingress sequence.
    #[must_use]
    pub const fn last_ingress_receive_ns(&self) -> Option<u64> {
        self.last_ingress_receive_ns
    }

    #[must_use]
    /// Returns the last checksum-bearing snapshot accepted by this reducer.
    ///
    /// Deltas and top checks never replace it. Consumers must still consult
    /// [`Self::readiness`] before treating the current book as available.
    pub const fn last_verified_snapshot_hash(&self) -> Option<VenueEventHash> {
        self.last_verified_snapshot_hash
    }

    #[must_use]
    pub const fn counters(&self) -> PmBookCounters {
        self.counters
    }

    #[must_use]
    pub const fn readiness(&self) -> PmBookReadiness {
        if self.metadata_valid
            && self.connection_epoch.is_some()
            && !self.requires_reconnect
            && self.book_ready
        {
            match (self.metadata_revision, self.snapshot_revision) {
                (Some(metadata), Some(snapshot)) => PmBookReadiness::ready(metadata, snapshot),
                _ => PmBookReadiness::unavailable(
                    PmPublicReadinessReason::SnapshotMissing,
                    self.metadata_revision,
                    self.snapshot_revision,
                ),
            }
        } else {
            PmBookReadiness::unavailable(
                self.reason,
                self.metadata_revision,
                self.snapshot_revision,
            )
        }
    }

    pub fn apply_metadata(
        &mut self,
        observation: PmMetadataObservation,
    ) -> Result<PmBookTransition, PmPublicReadinessReason> {
        if self.pending_external_fault.is_some() {
            return Err(self.reason);
        }
        self.counters.metadata_inputs = self.counters.metadata_inputs.saturating_add(1);
        if observation.instrument() != self.instrument {
            return self.reject_metadata(PmPublicReadinessReason::MetadataDrift(
                PmMetadataDrift::Instrument,
            ));
        }
        if self
            .metadata_revision
            .is_some_and(|current| observation.revision() <= current)
        {
            return self.reject_metadata(PmPublicReadinessReason::MetadataRevisionNotIncreasing);
        }
        if self
            .metadata_receive_ns
            .is_some_and(|current| observation.monotonic_receive_ns() < current)
        {
            return self.reject_metadata(PmPublicReadinessReason::ClockRegression);
        }
        if let Err(reason) =
            validate_observed_contract(self.expected_contract, observation.contract())
        {
            return self.reject_metadata(reason);
        }
        if observation.fingerprint() != self.expected_fingerprint {
            return self.reject_metadata(PmPublicReadinessReason::MetadataFingerprintMismatch);
        }

        self.note_unavailable_transition();
        self.metadata_revision = Some(observation.revision());
        self.metadata_receive_ns = Some(observation.monotonic_receive_ns());
        self.metadata_valid = true;
        self.book_ready = false;
        self.snapshot_revision = None;
        self.book_receive_ns = None;
        self.last_verified_snapshot_hash = None;
        self.reason = if self.connection_epoch.is_some() && !self.requires_reconnect {
            PmPublicReadinessReason::SnapshotMissing
        } else {
            PmPublicReadinessReason::ConnectionUnavailable
        };
        self.counters.metadata_accepted = self.counters.metadata_accepted.saturating_add(1);
        Ok(PmBookTransition::MetadataAccepted {
            revision: observation.revision(),
        })
    }

    pub fn begin_epoch(
        &mut self,
        epoch: ConnectionEpoch,
    ) -> Result<PmBookTransition, PmPublicReadinessReason> {
        if self.pending_external_fault.is_some() {
            return Err(self.reason);
        }
        self.counters.epoch_attempts = self.counters.epoch_attempts.saturating_add(1);
        if epoch.value() == 0 {
            return self.invalidate(PmPublicReadinessReason::ConnectionEpochInvalid);
        }
        let reconnect = if let Some(current) = self.connection_epoch {
            let Some(expected) = current.value().checked_add(1) else {
                return self.invalidate(PmPublicReadinessReason::ConnectionEpochInvalid);
            };
            if epoch.value() != expected {
                return self.invalidate(PmPublicReadinessReason::ConnectionEpochInvalid);
            }
            true
        } else {
            false
        };

        self.note_unavailable_transition();
        self.connection_epoch = Some(epoch);
        self.requires_reconnect = false;
        self.last_ingress_sequence = None;
        self.last_ingress_receive_ns = None;
        self.book_ready = false;
        self.snapshot_revision = None;
        self.book_receive_ns = None;
        self.last_verified_snapshot_hash = None;
        self.reason = if self.metadata_valid {
            PmPublicReadinessReason::SnapshotMissing
        } else {
            self.reason
        };
        self.counters.epochs_started = self.counters.epochs_started.saturating_add(1);
        if reconnect {
            self.counters.reconnects = self.counters.reconnects.saturating_add(1);
        }
        Ok(PmBookTransition::EpochStarted { epoch })
    }

    pub fn apply_update(
        &mut self,
        evidence: PmBookBatchEvidence,
        update: &PmBookUpdate,
    ) -> Result<PmBookTransition, PmPublicReadinessReason> {
        if self.pending_external_fault.is_some() {
            return Err(self.reason);
        }
        match update {
            PmBookUpdate::Snapshot(snapshot) => self.apply_snapshot(evidence, snapshot),
            PmBookUpdate::DeltaBatch(batch) => self.apply_delta_batch(evidence, batch),
            PmBookUpdate::TopCheck(top) => self.check_top(evidence, *top),
            PmBookUpdate::TickSizeChanged { old, new } => {
                self.tick_size_changed(evidence, *old, *new)
            }
        }
    }

    pub fn apply_snapshot(
        &mut self,
        evidence: PmBookBatchEvidence,
        snapshot: &PmBookSnapshot,
    ) -> Result<PmBookTransition, PmPublicReadinessReason> {
        if self.pending_external_fault.is_some() {
            return Err(self.reason);
        }
        self.counters.snapshot_attempts = self.counters.snapshot_attempts.saturating_add(1);
        self.validate_common_evidence(evidence)?;
        self.validate_snapshot_hash(evidence)?;
        let levels = snapshot.levels();
        if self
            .last_committed_snapshot_revision
            .is_some_and(|current| evidence.snapshot_revision() <= current)
        {
            return self.invalidate(PmPublicReadinessReason::SnapshotRevisionMismatch);
        }
        if levels.len() > MAX_LEVELS {
            return self.invalidate(PmPublicReadinessReason::TooManyLevels);
        }

        self.staging.clear();
        self.staging.extend_from_slice(levels);
        if let Err(reason) =
            validate_and_sort_levels(&mut self.staging, self.expected_contract.market(), true)
        {
            return self.invalidate(reason);
        }

        let was_resync = self.ever_committed_snapshot && !self.readiness().is_ready();
        std::mem::swap(&mut self.levels, &mut self.staging);
        self.snapshot_revision = Some(evidence.snapshot_revision());
        self.last_committed_snapshot_revision = Some(evidence.snapshot_revision());
        self.book_receive_ns = Some(evidence.monotonic_receive_ns());
        self.last_verified_snapshot_hash = evidence.venue_hash();
        self.book_ready = true;
        self.reason = PmPublicReadinessReason::SnapshotMissing;
        self.ever_committed_snapshot = true;
        self.counters.snapshots_committed = self.counters.snapshots_committed.saturating_add(1);
        self.counters.snapshot_levels_committed = self
            .counters
            .snapshot_levels_committed
            .saturating_add(levels.len() as u64);
        if was_resync {
            self.counters.resync_snapshots = self.counters.resync_snapshots.saturating_add(1);
        }
        let proof = PmSnapshotCommitProof {
            instrument: self.instrument,
            metadata_fingerprint: self.expected_fingerprint,
            connection_epoch: evidence.connection_epoch(),
            metadata_revision: evidence.metadata_revision(),
            snapshot_revision: evidence.snapshot_revision(),
            local_ingress_sequence: evidence.local_ingress_sequence(),
            venue_hash: evidence
                .venue_hash()
                .expect("snapshot hash validation requires exact SHA-1 evidence"),
        };
        Ok(PmBookTransition::SnapshotCommitted {
            revision: evidence.snapshot_revision(),
            levels: u16::try_from(levels.len()).expect("bounded snapshot length"),
            proof,
        })
    }

    pub fn apply_delta_batch(
        &mut self,
        evidence: PmBookBatchEvidence,
        batch: &PmBookDeltaBatch,
    ) -> Result<PmBookTransition, PmPublicReadinessReason> {
        if self.pending_external_fault.is_some() {
            return Err(self.reason);
        }
        self.counters.delta_batch_attempts = self.counters.delta_batch_attempts.saturating_add(1);
        self.validate_common_evidence(evidence)?;
        self.validate_absent_singular_hash(evidence)?;
        let changes = batch.changes();
        if !self.readiness().is_ready() {
            return self.invalidate(self.reason);
        }
        if self.snapshot_revision != Some(evidence.snapshot_revision()) {
            return self.invalidate(PmPublicReadinessReason::SnapshotRevisionMismatch);
        }
        if changes.is_empty() {
            return self.invalidate(PmPublicReadinessReason::InvalidTransition);
        }
        if changes.len() > MAX_LEVELS {
            return self.invalidate(PmPublicReadinessReason::TooManyLevels);
        }

        self.staging.clear();
        self.staging.extend_from_slice(&self.levels);
        self.change_keys.clear();
        for change in changes {
            if change
                .price()
                .validate_tick(self.expected_contract.market().tick())
                .is_err()
            {
                return self.invalidate(PmPublicReadinessReason::PriceOffTick);
            }
            let key = BookKey {
                side: change.side(),
                price: change.price(),
            };
            match self
                .change_keys
                .binary_search_by(|candidate| compare_keys(*candidate, key))
            {
                Ok(_) => return self.invalidate(PmPublicReadinessReason::DuplicateLevel),
                Err(index) => self.change_keys.insert(index, key),
            }
        }
        // A delta frame is one atomic set of unique side/price changes.
        // Apply deletes first so a valid full-capacity replacement does not
        // fail merely because the venue serialized its insert before delete.
        for change in changes
            .iter()
            .filter(|change| change.quantity() == PmBookQuantity::Delete)
        {
            if let Err(reason) = apply_change(&mut self.staging, *change) {
                return self.invalidate(reason);
            }
        }
        for change in changes
            .iter()
            .filter(|change| change.quantity() != PmBookQuantity::Delete)
        {
            if let Err(reason) = apply_change(&mut self.staging, *change) {
                return self.invalidate(reason);
            }
        }
        if let Err(reason) =
            validate_and_sort_levels(&mut self.staging, self.expected_contract.market(), false)
        {
            return self.invalidate(reason);
        }
        self.counters.delta_top_checks = self.counters.delta_top_checks.saturating_add(1);
        let (bid, ask) = canonical_top(&self.staging)?;
        let expected_top = batch.expected_top();
        if expected_top.bid() != Some(bid) || expected_top.ask() != Some(ask) {
            return self.invalidate(PmPublicReadinessReason::BboMismatch);
        }

        std::mem::swap(&mut self.levels, &mut self.staging);
        self.book_receive_ns = Some(evidence.monotonic_receive_ns());
        self.book_ready = true;
        self.reason = PmPublicReadinessReason::SnapshotMissing;
        self.counters.delta_batches_committed =
            self.counters.delta_batches_committed.saturating_add(1);
        self.counters.delta_changes_committed = self
            .counters
            .delta_changes_committed
            .saturating_add(changes.len() as u64);
        self.counters.delta_top_checks_confirmed =
            self.counters.delta_top_checks_confirmed.saturating_add(1);
        Ok(PmBookTransition::DeltaBatchCommitted {
            revision: evidence.snapshot_revision(),
            changes: u16::try_from(changes.len()).expect("bounded delta length"),
        })
    }

    pub fn check_top(
        &mut self,
        evidence: PmBookBatchEvidence,
        top: PmBookTopCheck,
    ) -> Result<PmBookTransition, PmPublicReadinessReason> {
        if self.pending_external_fault.is_some() {
            return Err(self.reason);
        }
        self.counters.top_checks = self.counters.top_checks.saturating_add(1);
        self.validate_common_evidence(evidence)?;
        self.validate_absent_singular_hash(evidence)?;
        if !self.readiness().is_ready() {
            return self.invalidate(self.reason);
        }
        if self.snapshot_revision != Some(evidence.snapshot_revision()) {
            return self.invalidate(PmPublicReadinessReason::SnapshotRevisionMismatch);
        }
        let (bid, ask) = canonical_top(&self.levels)?;
        if top.bid() != Some(bid) || top.ask() != Some(ask) {
            return self.invalidate(PmPublicReadinessReason::BboMismatch);
        }
        self.counters.top_checks_confirmed = self.counters.top_checks_confirmed.saturating_add(1);
        Ok(PmBookTransition::TopConfirmed)
    }

    pub fn tick_size_changed(
        &mut self,
        evidence: PmBookBatchEvidence,
        old: reap_pm_core::PmTick,
        new: reap_pm_core::PmTick,
    ) -> Result<PmBookTransition, PmPublicReadinessReason> {
        if self.pending_external_fault.is_some() {
            return Err(self.reason);
        }
        self.validate_common_evidence(evidence)?;
        self.validate_absent_singular_hash(evidence)?;
        self.counters.tick_size_changes = self.counters.tick_size_changes.saturating_add(1);
        let expected = self.expected_contract.market().tick();
        let reason = if old != expected || new == expected {
            PmPublicReadinessReason::InvalidTransition
        } else {
            PmPublicReadinessReason::MetadataDrift(PmMetadataDrift::Grid)
        };
        self.note_unavailable_transition();
        self.metadata_valid = false;
        self.book_ready = false;
        self.counters.metadata_rejected = self.counters.metadata_rejected.saturating_add(1);
        self.record_invalidation(reason)
    }

    pub fn apply_external_fault(
        &mut self,
        epoch: ConnectionEpoch,
        fault: PmExternalBookFault,
    ) -> Result<PmBookTransition, PmPublicReadinessReason> {
        if self.pending_external_fault.is_some() {
            return Err(self.reason);
        }
        self.apply_external_fault_final(epoch, fault)
    }

    /// Makes quote readiness unavailable before a bounded public-lane Full
    /// proof can escape the reducer-coupled pipeline.
    ///
    /// Fault counters are deferred until exact lifecycle enactment finalizes
    /// this pending state as either `Overflow` or `InvalidTransition`.
    pub fn begin_pending_external_fault(
        &mut self,
        epoch: ConnectionEpoch,
        fault: PmExternalBookFault,
    ) -> Result<PmPendingExternalBookFaultAuthority, PmPublicReadinessReason> {
        if self.pending_external_fault.is_some() {
            return Err(self.reason);
        }
        let pending_reason = match fault {
            PmExternalBookFault::Overflow => PmPublicReadinessReason::PendingOverflow,
            PmExternalBookFault::BacklogAged => PmPublicReadinessReason::PendingBookStale,
            _ => return self.invalidate(PmPublicReadinessReason::InvalidTransition),
        };
        if self.connection_epoch != Some(epoch) || self.requires_reconnect {
            return self.invalidate(PmPublicReadinessReason::ConnectionEpochMismatch);
        }
        let Some(generation) = self.pending_external_fault_generation.checked_add(1) else {
            return self.invalidate(PmPublicReadinessReason::InvalidTransition);
        };
        self.note_unavailable_transition();
        self.book_ready = false;
        self.reason = pending_reason;
        self.pending_external_fault_generation = generation;
        self.pending_external_fault = Some(PendingExternalBookFault {
            generation,
            epoch,
            fault,
        });
        Ok(PmPendingExternalBookFaultAuthority {
            reducer_authority: self.authority_id,
            generation,
            epoch,
            fault,
        })
    }

    /// Finalizes one exact pending reducer fault after its lifecycle evidence
    /// either committed successfully or failed closed.
    pub fn finalize_pending_external_fault(
        &mut self,
        authority: &PmPendingExternalBookFaultAuthority,
        final_fault: PmExternalBookFault,
    ) -> Result<PmBookTransition, PmPublicReadinessReason> {
        let expected = PendingExternalBookFault {
            generation: authority.generation,
            epoch: authority.epoch,
            fault: authority.fault,
        };
        if authority.reducer_authority != self.authority_id
            || self.pending_external_fault != Some(expected)
            || (final_fault != authority.fault
                && final_fault != PmExternalBookFault::InvalidTransition)
        {
            return Err(self.reason);
        }
        self.pending_external_fault = None;
        self.apply_external_fault_final(authority.epoch, final_fault)
    }

    #[must_use]
    pub const fn pending_external_fault(&self) -> Option<PmExternalBookFault> {
        match self.pending_external_fault {
            Some(pending) => Some(pending.fault),
            None => None,
        }
    }

    /// Returns whether this reducer is the exact pre-snapshot state accepted
    /// by a fresh active capture Run.
    ///
    /// Same-configuration reducers with prior snapshot, reconnect, ingress,
    /// fault, or staging history are deliberately rejected because that
    /// history is absent from the new capture artifact.
    #[must_use]
    pub fn is_pristine_pre_snapshot(&self) -> bool {
        let expected_counters = PmBookCounters {
            metadata_inputs: 1,
            metadata_accepted: 1,
            epoch_attempts: 1,
            epochs_started: 1,
            ..PmBookCounters::default()
        };
        self.metadata_valid
            && self.metadata_revision.is_some()
            && self.metadata_receive_ns.is_some()
            && self.connection_epoch.is_some()
            && !self.requires_reconnect
            && self.last_ingress_sequence.is_none()
            && self.last_ingress_receive_ns.is_none()
            && self.snapshot_revision.is_none()
            && self.last_committed_snapshot_revision.is_none()
            && self.book_receive_ns.is_none()
            && self.last_verified_snapshot_hash.is_none()
            && self.levels.is_empty()
            && self.staging.is_empty()
            && self.change_keys.is_empty()
            && !self.book_ready
            && self.reason == PmPublicReadinessReason::SnapshotMissing
            && self.pending_external_fault.is_none()
            && self.counters == expected_counters
            && !self.ever_committed_snapshot
    }

    fn apply_external_fault_final(
        &mut self,
        epoch: ConnectionEpoch,
        fault: PmExternalBookFault,
    ) -> Result<PmBookTransition, PmPublicReadinessReason> {
        if self.connection_epoch != Some(epoch) {
            return self.invalidate(PmPublicReadinessReason::ConnectionEpochMismatch);
        }
        self.counters.external_faults = self.counters.external_faults.saturating_add(1);
        let reason = match fault {
            PmExternalBookFault::Disconnect => {
                self.requires_reconnect = true;
                self.counters.disconnects = self.counters.disconnects.saturating_add(1);
                PmPublicReadinessReason::Disconnected
            }
            PmExternalBookFault::HeartbeatTimeout => {
                self.requires_reconnect = true;
                self.counters.heartbeat_timeouts =
                    self.counters.heartbeat_timeouts.saturating_add(1);
                PmPublicReadinessReason::HeartbeatTimeout
            }
            PmExternalBookFault::Gap => {
                self.counters.gaps = self.counters.gaps.saturating_add(1);
                PmPublicReadinessReason::Gap
            }
            PmExternalBookFault::Overflow => {
                self.counters.overflows = self.counters.overflows.saturating_add(1);
                PmPublicReadinessReason::Overflow
            }
            PmExternalBookFault::BacklogAged => {
                self.counters.backlog_aged_faults =
                    self.counters.backlog_aged_faults.saturating_add(1);
                self.counters.stale_invalidations =
                    self.counters.stale_invalidations.saturating_add(1);
                PmPublicReadinessReason::BookStale
            }
            PmExternalBookFault::InvalidTransition => PmPublicReadinessReason::InvalidTransition,
            PmExternalBookFault::HashMismatch => PmPublicReadinessReason::HashMismatch,
        };
        self.invalidate(reason)
    }

    pub fn check_freshness(
        &mut self,
        now_ns: u64,
    ) -> Result<PmBookTransition, PmPublicReadinessReason> {
        if self.pending_external_fault.is_some() {
            return Err(self.reason);
        }
        self.counters.freshness_checks = self.counters.freshness_checks.saturating_add(1);
        let Some(metadata_ns) = self.metadata_receive_ns else {
            return Err(self.reason);
        };
        let Some(book_ns) = self.book_receive_ns else {
            return Err(self.reason);
        };
        let Some(metadata_age) = now_ns.checked_sub(metadata_ns) else {
            return self.invalidate(PmPublicReadinessReason::ClockRegression);
        };
        let Some(book_age) = now_ns.checked_sub(book_ns) else {
            return self.invalidate(PmPublicReadinessReason::ClockRegression);
        };
        if metadata_age > self.freshness.metadata_max_age_ns() {
            self.note_unavailable_transition();
            self.metadata_valid = false;
            self.book_ready = false;
            self.counters.stale_invalidations = self.counters.stale_invalidations.saturating_add(1);
            return self.record_invalidation(PmPublicReadinessReason::MetadataStale);
        }
        if book_age > self.freshness.book_max_age_ns() {
            self.counters.stale_invalidations = self.counters.stale_invalidations.saturating_add(1);
            return self.invalidate(PmPublicReadinessReason::BookStale);
        }
        if !self.readiness().is_ready() {
            return Err(self.reason);
        }
        self.counters.freshness_confirmed = self.counters.freshness_confirmed.saturating_add(1);
        Ok(PmBookTransition::FreshnessConfirmed)
    }

    fn reject_metadata<T>(
        &mut self,
        reason: PmPublicReadinessReason,
    ) -> Result<T, PmPublicReadinessReason> {
        self.note_unavailable_transition();
        self.metadata_valid = false;
        self.book_ready = false;
        self.counters.metadata_rejected = self.counters.metadata_rejected.saturating_add(1);
        self.record_invalidation(reason)
    }

    fn validate_common_evidence(
        &mut self,
        evidence: PmBookBatchEvidence,
    ) -> Result<(), PmPublicReadinessReason> {
        if !self.metadata_valid {
            return Err(self.reason);
        }
        if evidence.instrument() != self.instrument {
            return self.invalidate(PmPublicReadinessReason::MetadataDrift(
                PmMetadataDrift::Instrument,
            ));
        }
        if self.connection_epoch != Some(evidence.connection_epoch()) || self.requires_reconnect {
            return self.invalidate(PmPublicReadinessReason::ConnectionEpochMismatch);
        }
        self.validate_ingress(evidence.local_ingress_sequence())?;
        if self
            .last_ingress_receive_ns
            .is_some_and(|current| evidence.monotonic_receive_ns() < current)
            || self
                .metadata_receive_ns
                .is_some_and(|metadata| evidence.monotonic_receive_ns() < metadata)
        {
            return self.invalidate(PmPublicReadinessReason::ClockRegression);
        }
        self.last_ingress_receive_ns = Some(evidence.monotonic_receive_ns());
        if self.metadata_revision != Some(evidence.metadata_revision()) {
            return self.invalidate(PmPublicReadinessReason::MetadataRevisionMismatch);
        }
        Ok(())
    }

    fn validate_snapshot_hash(
        &mut self,
        evidence: PmBookBatchEvidence,
    ) -> Result<(), PmPublicReadinessReason> {
        if !matches!(
            evidence.venue_hash(),
            Some(hash)
                if hash.algorithm() == VenueEventHashAlgorithm::Sha1 && hash.len() == 20
        ) {
            return self.invalidate(PmPublicReadinessReason::HashMismatch);
        }
        Ok(())
    }

    fn validate_absent_singular_hash(
        &mut self,
        evidence: PmBookBatchEvidence,
    ) -> Result<(), PmPublicReadinessReason> {
        if evidence.venue_hash().is_some() {
            return self.invalidate(PmPublicReadinessReason::HashMismatch);
        }
        Ok(())
    }

    fn validate_ingress(
        &mut self,
        sequence: IngressSequence,
    ) -> Result<(), PmPublicReadinessReason> {
        if let Some(last) = self.last_ingress_sequence {
            if sequence == last {
                self.counters.duplicate_ingress = self.counters.duplicate_ingress.saturating_add(1);
                return self.invalidate(PmPublicReadinessReason::DuplicateIngress);
            }
            if sequence < last {
                self.counters.reordered_ingress = self.counters.reordered_ingress.saturating_add(1);
                return self.invalidate(PmPublicReadinessReason::ReorderedIngress);
            }
        }
        self.last_ingress_sequence = Some(sequence);
        Ok(())
    }

    fn invalidate<T>(
        &mut self,
        reason: PmPublicReadinessReason,
    ) -> Result<T, PmPublicReadinessReason> {
        self.note_unavailable_transition();
        self.book_ready = false;
        self.record_invalidation(reason)
    }

    fn record_invalidation<T>(
        &mut self,
        reason: PmPublicReadinessReason,
    ) -> Result<T, PmPublicReadinessReason> {
        match reason {
            PmPublicReadinessReason::BboMismatch => {
                self.counters.bbo_mismatches = self.counters.bbo_mismatches.saturating_add(1);
            }
            PmPublicReadinessReason::ClockRegression => {
                self.counters.clock_regressions = self.counters.clock_regressions.saturating_add(1);
            }
            PmPublicReadinessReason::HashMismatch => {
                self.counters.hash_mismatches = self.counters.hash_mismatches.saturating_add(1);
            }
            PmPublicReadinessReason::InvalidTransition => {
                self.counters.invalid_transitions =
                    self.counters.invalid_transitions.saturating_add(1);
            }
            _ => {}
        }
        self.reason = reason;
        self.counters.invalidations = self.counters.invalidations.saturating_add(1);
        Err(reason)
    }

    fn note_unavailable_transition(&mut self) {
        if self.readiness().is_ready() {
            self.counters.unavailable_transitions =
                self.counters.unavailable_transitions.saturating_add(1);
        }
    }
}

/// Opaque, process-local identity for one concrete PM book reducer instance.
///
/// The numeric value is deliberately private and its `Debug` output is
/// redacted. Capture/replay determinism never depends on this identity.
#[derive(Clone, Copy, PartialEq, Eq, Hash)]
pub struct PmBookReducerAuthorityId(u64);

impl PmBookReducerAuthorityId {
    fn allocate() -> Result<Self, PmPublicReadinessReason> {
        static NEXT_AUTHORITY_ID: AtomicU64 = AtomicU64::new(1);
        NEXT_AUTHORITY_ID
            .fetch_update(
                AtomicOrdering::Relaxed,
                AtomicOrdering::Relaxed,
                |current| current.checked_add(1),
            )
            .map(Self)
            .map_err(|_| PmPublicReadinessReason::InvalidTransition)
    }
}

impl std::fmt::Debug for PmBookReducerAuthorityId {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter.write_str("PmBookReducerAuthorityId(<opaque>)")
    }
}
