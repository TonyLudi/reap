use std::sync::atomic::{AtomicU64, Ordering};

use reap_pm_core::{PmAccountHandle, PmInstrumentHandle};
use thiserror::Error;

pub const MAX_PM_REFRESH_OBLIGATIONS: usize = 128;

/// Stable identity of the private-state owner that issued a refresh ticket.
///
/// The composition root supplies one nonzero identity per owner incarnation.
/// Binding it into every ticket prevents an old or sibling state owner with
/// the same account/instrument scope from admitting or completing work.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct PmRefreshOwnerId(u64);

impl PmRefreshOwnerId {
    pub(crate) fn allocate() -> Result<Self, PmRefreshError> {
        static NEXT_OWNER: AtomicU64 = AtomicU64::new(1);
        NEXT_OWNER
            .fetch_update(Ordering::Relaxed, Ordering::Relaxed, |current| {
                current.checked_add(1)
            })
            .map(Self)
            .map_err(|_| PmRefreshError::OwnerIdentityExhausted)
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub enum PmRefreshReason {
    PrivateReconnect,
    FillObserved,
    FillFeeUnknown,
    FillConflict,
    FillSettlementRetrying,
    FillSettlementFailed,
    UnresolvedFill,
    UnmanagedOrder,
    AmbiguousOrder,
    MissingOrderDetail,
    AccountSnapshotStale,
    AccountDivergence,
    PositionUnavailable,
    AllowanceUnavailable,
    ExternalIngressFault,
    StateCapacity,
    RiskBreach,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct PmRefreshKey {
    account: PmAccountHandle,
    instrument: PmInstrumentHandle,
    reason: PmRefreshReason,
}

impl PmRefreshKey {
    #[must_use]
    pub const fn new(
        account: PmAccountHandle,
        instrument: PmInstrumentHandle,
        reason: PmRefreshReason,
    ) -> Self {
        Self {
            account,
            instrument,
            reason,
        }
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
    pub const fn reason(self) -> PmRefreshReason {
        self.reason
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct PmRefreshGeneration(u64);

impl PmRefreshGeneration {
    #[must_use]
    pub const fn value(self) -> u64 {
        self.0
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct PmRefreshTicket {
    owner: PmRefreshOwnerId,
    key: PmRefreshKey,
    generation: PmRefreshGeneration,
}

impl PmRefreshTicket {
    #[must_use]
    pub const fn owner(self) -> PmRefreshOwnerId {
        self.owner
    }

    #[must_use]
    pub const fn key(self) -> PmRefreshKey {
        self.key
    }

    #[must_use]
    pub const fn generation(self) -> PmRefreshGeneration {
        self.generation
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum PmRefreshRequired {
    Inserted {
        ticket: PmRefreshTicket,
    },
    AlreadyRequired {
        ticket: PmRefreshTicket,
    },
    SupersededInFlight {
        required: PmRefreshTicket,
        in_flight: PmRefreshTicket,
    },
    Saturated {
        key: PmRefreshKey,
    },
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum PmRefreshAdmission {
    Admitted(PmRefreshTicket),
    AlreadyInFlight(PmRefreshTicket),
    NotRequired(PmRefreshKey),
    Stale(PmRefreshTicket),
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum PmRefreshCompletion {
    Cleared(PmRefreshTicket),
    NewerRequirementRetained {
        completed: PmRefreshTicket,
        required: PmRefreshTicket,
    },
    Stale(PmRefreshTicket),
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub struct PmRefreshCounters {
    requirements: u64,
    deduplicated: u64,
    superseded_in_flight: u64,
    admissions: u64,
    completions: u64,
    stale_completions: u64,
    saturations: u64,
    high_water: usize,
}

impl PmRefreshCounters {
    #[must_use]
    pub const fn requirements(self) -> u64 {
        self.requirements
    }

    #[must_use]
    pub const fn deduplicated(self) -> u64 {
        self.deduplicated
    }

    #[must_use]
    pub const fn superseded_in_flight(self) -> u64 {
        self.superseded_in_flight
    }

    #[must_use]
    pub const fn admissions(self) -> u64 {
        self.admissions
    }

    #[must_use]
    pub const fn completions(self) -> u64 {
        self.completions
    }

    #[must_use]
    pub const fn stale_completions(self) -> u64 {
        self.stale_completions
    }

    #[must_use]
    pub const fn saturations(self) -> u64 {
        self.saturations
    }

    #[must_use]
    pub const fn high_water(self) -> usize {
        self.high_water
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Error)]
pub enum PmRefreshError {
    #[error("refresh owner identity space exhausted")]
    OwnerIdentityExhausted,
    #[error("refresh-obligation generation exhausted")]
    GenerationExhausted,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
struct PmRefreshEntry {
    key: PmRefreshKey,
    required_generation: PmRefreshGeneration,
    in_flight_generation: Option<PmRefreshGeneration>,
}

impl PmRefreshEntry {
    const fn required_ticket(self, owner: PmRefreshOwnerId) -> PmRefreshTicket {
        PmRefreshTicket {
            owner,
            key: self.key,
            generation: self.required_generation,
        }
    }

    const fn in_flight_ticket(self, owner: PmRefreshOwnerId) -> Option<PmRefreshTicket> {
        match self.in_flight_generation {
            Some(generation) => Some(PmRefreshTicket {
                owner,
                key: self.key,
                generation,
            }),
            None => None,
        }
    }
}

pub(crate) struct PmRefreshState {
    owner: PmRefreshOwnerId,
    entries: [Option<PmRefreshEntry>; MAX_PM_REFRESH_OBLIGATIONS],
    length: usize,
    next_generation: u64,
    full_reconcile_required: bool,
    counters: PmRefreshCounters,
}

impl PmRefreshState {
    pub(crate) const fn new(owner: PmRefreshOwnerId) -> Self {
        Self {
            owner,
            entries: [None; MAX_PM_REFRESH_OBLIGATIONS],
            length: 0,
            next_generation: 1,
            full_reconcile_required: false,
            counters: PmRefreshCounters {
                requirements: 0,
                deduplicated: 0,
                superseded_in_flight: 0,
                admissions: 0,
                completions: 0,
                stale_completions: 0,
                saturations: 0,
                high_water: 0,
            },
        }
    }

    pub(crate) fn reserved_capacity_bytes(&self) -> usize {
        std::mem::size_of_val(&self.entries)
    }

    pub(crate) fn require(
        &mut self,
        key: PmRefreshKey,
    ) -> Result<PmRefreshRequired, PmRefreshError> {
        self.counters.requirements = self.counters.requirements.saturating_add(1);
        match self.search(key) {
            Ok(index) => {
                let entry = self.entries[index].expect("occupied refresh entry");
                if let Some(in_flight) = entry.in_flight_ticket(self.owner) {
                    let generation = self.issue_generation()?;
                    let updated = PmRefreshEntry {
                        required_generation: generation,
                        ..entry
                    };
                    self.entries[index] = Some(updated);
                    self.counters.superseded_in_flight =
                        self.counters.superseded_in_flight.saturating_add(1);
                    Ok(PmRefreshRequired::SupersededInFlight {
                        required: updated.required_ticket(self.owner),
                        in_flight,
                    })
                } else {
                    self.counters.deduplicated = self.counters.deduplicated.saturating_add(1);
                    Ok(PmRefreshRequired::AlreadyRequired {
                        ticket: entry.required_ticket(self.owner),
                    })
                }
            }
            Err(index) if self.length < MAX_PM_REFRESH_OBLIGATIONS => {
                let generation = self.issue_generation()?;
                for cursor in (index..self.length).rev() {
                    self.entries[cursor + 1] = self.entries[cursor];
                }
                let entry = PmRefreshEntry {
                    key,
                    required_generation: generation,
                    in_flight_generation: None,
                };
                self.entries[index] = Some(entry);
                self.length += 1;
                self.counters.high_water = self.counters.high_water.max(self.length);
                Ok(PmRefreshRequired::Inserted {
                    ticket: entry.required_ticket(self.owner),
                })
            }
            Err(_) => {
                self.full_reconcile_required = true;
                self.counters.saturations = self.counters.saturations.saturating_add(1);
                Ok(PmRefreshRequired::Saturated { key })
            }
        }
    }

    pub(crate) fn mark_admitted(&mut self, ticket: PmRefreshTicket) -> PmRefreshAdmission {
        if ticket.owner != self.owner {
            return PmRefreshAdmission::Stale(ticket);
        }
        let Ok(index) = self.search(ticket.key) else {
            return PmRefreshAdmission::NotRequired(ticket.key);
        };
        let mut entry = self.entries[index].expect("occupied refresh entry");
        if entry.required_generation != ticket.generation {
            return PmRefreshAdmission::Stale(ticket);
        }
        if let Some(in_flight) = entry.in_flight_ticket(self.owner) {
            return PmRefreshAdmission::AlreadyInFlight(in_flight);
        }
        entry.in_flight_generation = Some(entry.required_generation);
        self.entries[index] = Some(entry);
        self.counters.admissions = self.counters.admissions.saturating_add(1);
        PmRefreshAdmission::Admitted(entry.required_ticket(self.owner))
    }

    pub(crate) fn complete(&mut self, ticket: PmRefreshTicket) -> PmRefreshCompletion {
        if ticket.owner != self.owner {
            self.counters.stale_completions = self.counters.stale_completions.saturating_add(1);
            return PmRefreshCompletion::Stale(ticket);
        }
        let Ok(index) = self.search(ticket.key) else {
            self.counters.stale_completions = self.counters.stale_completions.saturating_add(1);
            return PmRefreshCompletion::Stale(ticket);
        };
        let mut entry = self.entries[index].expect("occupied refresh entry");
        if entry.in_flight_generation != Some(ticket.generation) {
            self.counters.stale_completions = self.counters.stale_completions.saturating_add(1);
            return PmRefreshCompletion::Stale(ticket);
        }
        self.counters.completions = self.counters.completions.saturating_add(1);
        if entry.required_generation == ticket.generation {
            self.remove(index);
            PmRefreshCompletion::Cleared(ticket)
        } else {
            entry.in_flight_generation = None;
            self.entries[index] = Some(entry);
            PmRefreshCompletion::NewerRequirementRetained {
                completed: ticket,
                required: entry.required_ticket(self.owner),
            }
        }
    }

    pub(crate) fn pending(&self) -> impl Iterator<Item = PmRefreshTicket> + '_ {
        self.entries[..self.length]
            .iter()
            .flatten()
            .filter(|entry| entry.in_flight_generation.is_none())
            .map(|entry| entry.required_ticket(self.owner))
    }

    pub(crate) fn pending_keys(&self) -> impl Iterator<Item = PmRefreshKey> + '_ {
        self.entries[..self.length]
            .iter()
            .flatten()
            .filter(|entry| entry.in_flight_generation.is_none())
            .map(|entry| entry.key)
    }

    pub(crate) const fn len(&self) -> usize {
        self.length
    }

    pub(crate) const fn full_reconcile_required(&self) -> bool {
        self.full_reconcile_required
    }

    pub(crate) fn complete_full_reconciliation(&mut self) {
        self.full_reconcile_required = false;
    }

    pub(crate) const fn counters(&self) -> PmRefreshCounters {
        self.counters
    }

    fn issue_generation(&mut self) -> Result<PmRefreshGeneration, PmRefreshError> {
        let generation = self.next_generation;
        let Some(next_generation) = self.next_generation.checked_add(1) else {
            self.full_reconcile_required = true;
            return Err(PmRefreshError::GenerationExhausted);
        };
        self.next_generation = next_generation;
        Ok(PmRefreshGeneration(generation))
    }

    fn search(&self, key: PmRefreshKey) -> Result<usize, usize> {
        self.entries[..self.length]
            .binary_search_by_key(&key, |entry| entry.expect("occupied refresh prefix").key)
    }

    fn remove(&mut self, index: usize) {
        for cursor in index..self.length - 1 {
            self.entries[cursor] = self.entries[cursor + 1];
        }
        self.length -= 1;
        self.entries[self.length] = None;
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use reap_pm_core::{PmMarketHandle, PmTokenHandle};

    fn owner() -> PmRefreshOwnerId {
        PmRefreshOwnerId::allocate().unwrap()
    }

    fn key(index: u16, reason: PmRefreshReason) -> PmRefreshKey {
        PmRefreshKey::new(
            PmAccountHandle::from_ordinal(index),
            PmInstrumentHandle::new(
                PmMarketHandle::from_ordinal(index),
                PmTokenHandle::from_ordinal(index),
            ),
            reason,
        )
    }

    #[test]
    fn duplicate_requirements_coalesce_until_admission() {
        let mut state = PmRefreshState::new(owner());
        let key = key(7, PmRefreshReason::FillObserved);
        let inserted = state.require(key).unwrap();
        let repeated = state.require(key).unwrap();

        let PmRefreshRequired::Inserted { ticket } = inserted else {
            panic!("first requirement must insert");
        };
        assert_eq!(repeated, PmRefreshRequired::AlreadyRequired { ticket });
        assert_eq!(state.pending().collect::<Vec<_>>(), vec![ticket]);
        assert_eq!(state.counters().requirements(), 2);
        assert_eq!(state.counters().deduplicated(), 1);
    }

    #[test]
    fn a_new_occurrence_while_in_flight_survives_old_completion() {
        let mut state = PmRefreshState::new(owner());
        let key = key(2, PmRefreshReason::FillObserved);
        let PmRefreshRequired::Inserted { ticket } = state.require(key).unwrap() else {
            panic!("insert");
        };
        let PmRefreshAdmission::Admitted(first) = state.mark_admitted(ticket) else {
            panic!("required refresh must admit");
        };
        let PmRefreshRequired::SupersededInFlight {
            required,
            in_flight,
        } = state.require(key).unwrap()
        else {
            panic!("new occurrence must supersede in-flight generation");
        };
        assert_eq!(first, in_flight);
        assert_ne!(first, required);
        assert_eq!(
            state.complete(first),
            PmRefreshCompletion::NewerRequirementRetained {
                completed: first,
                required,
            }
        );
        assert_eq!(state.pending().collect::<Vec<_>>(), vec![required]);
        assert_eq!(
            state.mark_admitted(required),
            PmRefreshAdmission::Admitted(required)
        );
        assert_eq!(
            state.complete(required),
            PmRefreshCompletion::Cleared(required)
        );
        assert_eq!(state.len(), 0);
    }

    #[test]
    fn stale_or_wrong_completion_never_clears_current_work() {
        let mut state = PmRefreshState::new(owner());
        let key = key(3, PmRefreshReason::AccountDivergence);
        let PmRefreshRequired::Inserted { ticket } = state.require(key).unwrap() else {
            panic!("insert");
        };
        assert_eq!(state.complete(ticket), PmRefreshCompletion::Stale(ticket));
        assert_eq!(state.len(), 1);
        let PmRefreshAdmission::Admitted(admitted) = state.mark_admitted(ticket) else {
            panic!("admit");
        };
        assert_eq!(
            state.complete(admitted),
            PmRefreshCompletion::Cleared(admitted)
        );
    }

    #[test]
    fn phase6_refresh_mechanism_row_is_exactly_129_attempts_until_full_reconciliation() {
        let mut state = PmRefreshState::new(owner());
        let mut inserted = 0;
        for index in 0..MAX_PM_REFRESH_OBLIGATIONS {
            let index = u16::try_from(index).unwrap();
            assert!(matches!(
                state
                    .require(key(index, PmRefreshReason::FillObserved))
                    .unwrap(),
                PmRefreshRequired::Inserted { .. }
            ));
            inserted += 1;
        }
        let rejected_key = key(900, PmRefreshReason::FillObserved);
        assert_eq!(
            state.require(rejected_key).unwrap(),
            PmRefreshRequired::Saturated { key: rejected_key }
        );
        assert_eq!(inserted, 128);
        assert_eq!(state.len(), MAX_PM_REFRESH_OBLIGATIONS);
        assert_eq!(state.pending().count(), MAX_PM_REFRESH_OBLIGATIONS);
        assert!(!state.pending_keys().any(|key| key == rejected_key));
        // The one coarse bit is the mechanism's fail-closed readiness result
        // for the exact rejected scope; no public constructor or runtime
        // authority is introduced for this test.
        let rejected_scope_is_unready = state.full_reconcile_required();
        assert!(rejected_scope_is_unready);
        assert_eq!(state.counters().high_water(), MAX_PM_REFRESH_OBLIGATIONS);
        assert_eq!(state.counters().saturations(), 1);
        assert_eq!(state.counters().requirements(), 129);
        state.complete_full_reconciliation();
        assert!(!state.full_reconcile_required());
        assert_eq!(state.len(), MAX_PM_REFRESH_OBLIGATIONS);
        assert_eq!(state.pending().count(), MAX_PM_REFRESH_OBLIGATIONS);
    }

    #[test]
    fn pending_iteration_is_in_canonical_key_order() {
        let mut state = PmRefreshState::new(owner());
        let high = key(9, PmRefreshReason::RiskBreach);
        let low = key(1, PmRefreshReason::FillObserved);
        state.require(high).unwrap();
        state.require(low).unwrap();
        assert_eq!(
            state
                .pending()
                .map(PmRefreshTicket::key)
                .collect::<Vec<_>>(),
            vec![low, high]
        );
    }

    #[test]
    fn sibling_owner_and_old_generation_cannot_admit_work() {
        let mut state = PmRefreshState::new(owner());
        let key = key(4, PmRefreshReason::PrivateReconnect);
        let PmRefreshRequired::Inserted { ticket } = state.require(key).unwrap() else {
            panic!("insert");
        };
        let sibling = PmRefreshTicket {
            owner: owner(),
            ..ticket
        };
        assert_eq!(
            state.mark_admitted(sibling),
            PmRefreshAdmission::Stale(sibling)
        );
        assert_eq!(
            state.mark_admitted(ticket),
            PmRefreshAdmission::Admitted(ticket)
        );
        let PmRefreshRequired::SupersededInFlight { required, .. } = state.require(key).unwrap()
        else {
            panic!("supersede");
        };
        assert_eq!(
            state.mark_admitted(ticket),
            PmRefreshAdmission::Stale(ticket)
        );
        assert_eq!(state.complete(sibling), PmRefreshCompletion::Stale(sibling));
        assert_eq!(
            state.complete(ticket),
            PmRefreshCompletion::NewerRequirementRetained {
                completed: ticket,
                required,
            }
        );
    }
}
