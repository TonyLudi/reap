use reap_pm_core::{
    EventEnvelope, PmAccountHandle, PmFillId, PmFillSettlementStatus, PmInstrumentHandle,
    PmProductSource, PmSourceBound, PmVenueOrderId, PmVenueOrderKey,
};
use thiserror::Error;

use crate::fill_state::settlement_transition;
use crate::private_config::PmPrivateStateConfig;
use crate::private_occurrence::PmPrivateOccurrence;

pub const MAX_PM_UNRESOLVED_FILLS: usize = reap_pm_core::MAX_PM_RECONCILIATION_FILLS;

#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct PmUnresolvedFillKey {
    fill_id: PmFillId,
    exact_order: Option<PmVenueOrderKey>,
    candidate_order: Option<PmVenueOrderId>,
}

impl PmUnresolvedFillKey {
    #[must_use]
    pub const fn new(
        fill_id: PmFillId,
        exact_order: Option<PmVenueOrderKey>,
        candidate_order: Option<PmVenueOrderId>,
    ) -> Self {
        Self {
            fill_id,
            exact_order,
            candidate_order,
        }
    }

    #[must_use]
    pub const fn fill_id(self) -> PmFillId {
        self.fill_id
    }

    #[must_use]
    pub const fn exact_order(self) -> Option<PmVenueOrderKey> {
        self.exact_order
    }

    #[must_use]
    pub const fn candidate_order(self) -> Option<PmVenueOrderId> {
        self.candidate_order
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub enum PmUnresolvedFillReason {
    MissingExactOrderLinkage,
    MultipleOrderReferenceKinds,
    MissingDirectOrderRole,
    MissingLocalMakerOrderProof,
    ExternalMakerOrder,
}

/// A private trade observation that cannot safely become an economic fill.
///
/// This carrier deliberately contains no side, price, quantity, or ownership
/// claim. Canonical state retains the unresolved identity and lifecycle fact,
/// blocks quoting, and waits for an authoritative paired reconciliation cut.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct PmUnresolvedFillObservation {
    source: PmProductSource,
    account: PmAccountHandle,
    instrument: PmInstrumentHandle,
    key: PmUnresolvedFillKey,
    reason: PmUnresolvedFillReason,
    settlement: PmFillSettlementStatus,
}

impl PmUnresolvedFillObservation {
    #[allow(
        clippy::too_many_arguments,
        reason = "the quarantine carrier binds every exact scope and unresolved identity fact"
    )]
    pub fn new(
        source: PmProductSource,
        account: PmAccountHandle,
        instrument: PmInstrumentHandle,
        fill_id: PmFillId,
        exact_order: Option<PmVenueOrderKey>,
        candidate_order: Option<PmVenueOrderId>,
        reason: PmUnresolvedFillReason,
        settlement: PmFillSettlementStatus,
    ) -> Result<Self, PmUnresolvedFillStateError> {
        match source {
            PmProductSource::PolymarketAccount {
                account: source_account,
                ..
            } if source_account == account => {}
            _ => return Err(PmUnresolvedFillStateError::AccountSourceMismatch),
        }
        if exact_order.is_some_and(|order| order.account() != account) {
            return Err(PmUnresolvedFillStateError::ExactOrderAccountMismatch);
        }
        Ok(Self {
            source,
            account,
            instrument,
            key: PmUnresolvedFillKey::new(fill_id, exact_order, candidate_order),
            reason,
            settlement,
        })
    }

    #[must_use]
    pub const fn account(self) -> PmAccountHandle {
        self.account
    }

    #[must_use]
    pub const fn instrument(self) -> PmInstrumentHandle {
        self.instrument
    }

    #[must_use]
    pub const fn key(self) -> PmUnresolvedFillKey {
        self.key
    }

    #[must_use]
    pub const fn reason(self) -> PmUnresolvedFillReason {
        self.reason
    }

    #[must_use]
    pub const fn settlement(self) -> PmFillSettlementStatus {
        self.settlement
    }
}

impl PmSourceBound for PmUnresolvedFillObservation {
    fn source(&self) -> PmProductSource {
        self.source
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct PmUnresolvedFillProjection {
    observation: PmUnresolvedFillObservation,
    first_occurrence: PmPrivateOccurrence,
    last_occurrence: PmPrivateOccurrence,
    covered_by_reconciliation: Option<PmPrivateOccurrence>,
}

impl PmUnresolvedFillProjection {
    #[must_use]
    pub const fn observation(self) -> PmUnresolvedFillObservation {
        self.observation
    }

    #[must_use]
    pub const fn key(self) -> PmUnresolvedFillKey {
        self.observation.key()
    }

    #[must_use]
    pub const fn reason(self) -> PmUnresolvedFillReason {
        self.observation.reason()
    }

    #[must_use]
    pub const fn settlement(self) -> PmFillSettlementStatus {
        self.observation.settlement()
    }

    #[must_use]
    pub const fn first_occurrence(self) -> PmPrivateOccurrence {
        self.first_occurrence
    }

    #[must_use]
    pub const fn last_occurrence(self) -> PmPrivateOccurrence {
        self.last_occurrence
    }

    #[must_use]
    pub const fn covered_by_reconciliation(self) -> Option<PmPrivateOccurrence> {
        self.covered_by_reconciliation
    }

    #[must_use]
    pub fn is_active(self) -> bool {
        match self.covered_by_reconciliation {
            Some(covered) => self.last_occurrence > covered,
            None => true,
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum PmUnresolvedFillApply {
    Inserted(PmUnresolvedFillKey),
    SettlementAdvanced {
        key: PmUnresolvedFillKey,
        settlement: PmFillSettlementStatus,
    },
    Duplicate(PmUnresolvedFillKey),
    IgnoredStale(PmUnresolvedFillKey),
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub struct PmUnresolvedFillCounters {
    insertions: u64,
    settlement_updates: u64,
    duplicates: u64,
    stale: u64,
    conflicts: u64,
    capacity_failures: u64,
    reconciled: u64,
}

impl PmUnresolvedFillCounters {
    #[must_use]
    pub const fn insertions(self) -> u64 {
        self.insertions
    }

    #[must_use]
    pub const fn settlement_updates(self) -> u64 {
        self.settlement_updates
    }

    #[must_use]
    pub const fn duplicates(self) -> u64 {
        self.duplicates
    }

    #[must_use]
    pub const fn stale(self) -> u64 {
        self.stale
    }

    #[must_use]
    pub const fn conflicts(self) -> u64 {
        self.conflicts
    }

    #[must_use]
    pub const fn capacity_failures(self) -> u64 {
        self.capacity_failures
    }

    #[must_use]
    pub const fn reconciled(self) -> u64 {
        self.reconciled
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Error)]
pub enum PmUnresolvedFillStateError {
    #[error("unresolved fill source is not the exact configured PM account source")]
    AccountSourceMismatch,
    #[error("unresolved fill exact order belongs to another account")]
    ExactOrderAccountMismatch,
    #[error("unresolved fill belongs to another configured PM source/account/instrument")]
    ScopeMismatch,
    #[error("duplicate unresolved fill identity carries conflicting facts or lifecycle")]
    Conflict,
    #[error("unresolved PM fill storage reached its fixed bound")]
    Capacity,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
struct Entry {
    observation: PmUnresolvedFillObservation,
    first_occurrence: PmPrivateOccurrence,
    last_occurrence: PmPrivateOccurrence,
    covered_by_reconciliation: Option<PmPrivateOccurrence>,
}

impl Entry {
    const fn projection(self) -> PmUnresolvedFillProjection {
        PmUnresolvedFillProjection {
            observation: self.observation,
            first_occurrence: self.first_occurrence,
            last_occurrence: self.last_occurrence,
            covered_by_reconciliation: self.covered_by_reconciliation,
        }
    }

    fn is_active(&self) -> bool {
        self.projection().is_active()
    }
}

pub(crate) struct PmUnresolvedFillState {
    entries: Vec<Entry>,
    observed_monotonic_ns: Option<u64>,
    counters: PmUnresolvedFillCounters,
}

impl PmUnresolvedFillState {
    pub(crate) fn new() -> Self {
        Self {
            entries: Vec::with_capacity(MAX_PM_UNRESOLVED_FILLS),
            observed_monotonic_ns: None,
            counters: PmUnresolvedFillCounters::default(),
        }
    }

    pub(crate) fn observe(
        &mut self,
        envelope: EventEnvelope<PmUnresolvedFillObservation>,
        config: &PmPrivateStateConfig,
    ) -> Result<PmUnresolvedFillApply, PmUnresolvedFillStateError> {
        let observation = *envelope.payload();
        if envelope.source() != config.source()
            || observation.source != config.source()
            || observation.account() != config.account()
            || observation.instrument() != config.instrument()
        {
            return Err(PmUnresolvedFillStateError::ScopeMismatch);
        }
        let occurrence = PmPrivateOccurrence::new(
            envelope.ordering().connection_epoch(),
            envelope.ordering().local_ingress_sequence(),
        );
        self.observed_monotonic_ns = Some(envelope.clock().monotonic_service_ns());
        match self.search(observation.key()) {
            Ok(index) => self.enrich(index, observation, occurrence),
            Err(index) => self.insert(index, observation, occurrence),
        }
    }

    pub(crate) fn cover_through(
        &mut self,
        request: PmPrivateOccurrence,
        completion: PmPrivateOccurrence,
    ) {
        for entry in &mut self.entries {
            if entry.is_active() && entry.last_occurrence <= request {
                entry.covered_by_reconciliation = Some(completion);
                self.counters.reconciled = self.counters.reconciled.saturating_add(1);
            }
        }
    }

    pub(crate) fn projections(&self) -> impl Iterator<Item = PmUnresolvedFillProjection> + '_ {
        self.entries.iter().copied().map(Entry::projection)
    }

    pub(crate) fn first_active(&self) -> Option<PmUnresolvedFillProjection> {
        self.entries
            .iter()
            .find(|entry| entry.is_active())
            .copied()
            .map(Entry::projection)
    }

    pub(crate) fn active_count(&self) -> u16 {
        u16::try_from(
            self.entries
                .iter()
                .filter(|entry| entry.is_active())
                .count(),
        )
        .expect("unresolved fill capacity fits u16")
    }

    pub(crate) const fn observed_monotonic_ns(&self) -> Option<u64> {
        self.observed_monotonic_ns
    }

    pub(crate) const fn counters(&self) -> PmUnresolvedFillCounters {
        self.counters
    }

    fn search(&self, key: PmUnresolvedFillKey) -> Result<usize, usize> {
        self.entries
            .binary_search_by_key(&key, |entry| entry.observation.key())
    }

    fn insert(
        &mut self,
        index: usize,
        observation: PmUnresolvedFillObservation,
        occurrence: PmPrivateOccurrence,
    ) -> Result<PmUnresolvedFillApply, PmUnresolvedFillStateError> {
        if self.entries.len() == MAX_PM_UNRESOLVED_FILLS {
            self.counters.capacity_failures = self.counters.capacity_failures.saturating_add(1);
            return Err(PmUnresolvedFillStateError::Capacity);
        }
        let key = observation.key();
        self.entries.insert(
            index,
            Entry {
                observation,
                first_occurrence: occurrence,
                last_occurrence: occurrence,
                covered_by_reconciliation: None,
            },
        );
        self.counters.insertions = self.counters.insertions.saturating_add(1);
        Ok(PmUnresolvedFillApply::Inserted(key))
    }

    fn enrich(
        &mut self,
        index: usize,
        observation: PmUnresolvedFillObservation,
        occurrence: PmPrivateOccurrence,
    ) -> Result<PmUnresolvedFillApply, PmUnresolvedFillStateError> {
        let prior = self.entries[index];
        let key = prior.observation.key();
        if occurrence < prior.last_occurrence {
            self.counters.stale = self.counters.stale.saturating_add(1);
            return Ok(PmUnresolvedFillApply::IgnoredStale(key));
        }
        if prior.observation.source != observation.source
            || prior.observation.account() != observation.account()
            || prior.observation.instrument() != observation.instrument()
            || prior.observation.reason() != observation.reason()
        {
            self.counters.conflicts = self.counters.conflicts.saturating_add(1);
            return Err(PmUnresolvedFillStateError::Conflict);
        }
        let transition =
            settlement_transition(prior.observation.settlement(), observation.settlement())
                .map_err(|_| {
                    self.counters.conflicts = self.counters.conflicts.saturating_add(1);
                    PmUnresolvedFillStateError::Conflict
                })?;
        let Some(settlement) = transition else {
            self.counters.duplicates = self.counters.duplicates.saturating_add(1);
            return Ok(PmUnresolvedFillApply::Duplicate(key));
        };
        let entry = &mut self.entries[index];
        entry.observation.settlement = settlement;
        entry.last_occurrence = occurrence;
        self.counters.settlement_updates = self.counters.settlement_updates.saturating_add(1);
        Ok(PmUnresolvedFillApply::SettlementAdvanced { key, settlement })
    }
}
