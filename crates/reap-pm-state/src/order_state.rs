use std::cmp::Ordering;

use reap_pm_core::{
    EventEnvelope, PmClientOrderKey, PmCompleteOpenOrdersSnapshot, PmExactOrderDetail,
    PmInstrumentHandle, PmOrderEvent, PmOrderIdentity, PmOrderSide, PmOrderStatus, PmPrice,
    PmQuantity, PmVenueOrderKey, U256, exact_order_amounts,
};
use thiserror::Error;

use crate::private_config::PmPrivateStateConfig;
use crate::private_occurrence::PmPrivateOccurrence;

mod admission;
mod decision_summary;
mod dense_index;
pub(crate) use decision_summary::PmOrderDecisionSummary;

pub const MAX_PM_PRIVATE_ORDERS: usize = reap_pm_core::MAX_PM_RECONCILIATION_ORDERS;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum PmReservationBasis {
    /// An upstream exact policy explicitly includes the worst-case fee and
    /// collateral/token requirement reached by this product.
    PolicyApprovedWorstCase,
    /// A complete read proves the exact remaining sell-token requirement.
    AuthoritativeSellRemaining,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct PmExactReservation {
    collateral: U256,
    outcome: U256,
    basis: PmReservationBasis,
}

impl PmExactReservation {
    pub fn policy_approved(collateral: U256, outcome: U256) -> Result<Self, PmOrderStateError> {
        Self::checked(
            collateral,
            outcome,
            PmReservationBasis::PolicyApprovedWorstCase,
        )
    }

    pub fn authoritative_sell_remaining(outcome: U256) -> Result<Self, PmOrderStateError> {
        Self::checked(
            U256::ZERO,
            outcome,
            PmReservationBasis::AuthoritativeSellRemaining,
        )
    }

    fn checked(
        collateral: U256,
        outcome: U256,
        basis: PmReservationBasis,
    ) -> Result<Self, PmOrderStateError> {
        if collateral.is_zero() && outcome.is_zero() {
            return Err(PmOrderStateError::ZeroReservation);
        }
        Ok(Self {
            collateral,
            outcome,
            basis,
        })
    }

    #[must_use]
    pub const fn collateral(self) -> U256 {
        self.collateral
    }

    #[must_use]
    pub const fn outcome(self) -> U256 {
        self.outcome
    }

    #[must_use]
    pub const fn basis(self) -> PmReservationBasis {
        self.basis
    }

    pub fn validate_for(
        self,
        side: PmOrderSide,
        price: PmPrice,
        quantity: PmQuantity,
    ) -> Result<Self, PmOrderStateError> {
        validate_requirement(side, price, quantity.protocol_units(), self)?;
        Ok(self)
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum PmReservationKnowledge {
    Known(PmExactReservation),
    Unknown,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum PmRemoteOrderKnowledge {
    Unmanaged(PmReservationKnowledge),
    Ambiguous,
}

/// Structural local ownership registration.
///
/// This is not dispatch/cancel authority. Phase 5's journaled owner will
/// create it before an effect; the read model merely preserves that proof and
/// never infers ownership from matching economic fields.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct PmOwnedOrderRegistration {
    client_order: PmClientOrderKey,
    instrument: PmInstrumentHandle,
    side: PmOrderSide,
    price: PmPrice,
    quantity: PmQuantity,
    reservation: PmExactReservation,
}

pub(crate) struct PmOwnedRegistrationPlan {
    entry: OrderEntry,
    action: PmOwnedRegistrationAction,
}

enum PmOwnedRegistrationAction {
    Existing,
    Insert {
        canonical_position: usize,
        client_position: usize,
    },
}

impl PmOwnedOrderRegistration {
    pub fn new(
        client_order: PmClientOrderKey,
        instrument: PmInstrumentHandle,
        side: PmOrderSide,
        price: PmPrice,
        quantity: PmQuantity,
        reservation: PmExactReservation,
    ) -> Result<Self, PmOrderStateError> {
        if reservation.basis() != PmReservationBasis::PolicyApprovedWorstCase {
            return Err(PmOrderStateError::OwnedReservationRequiresPolicy);
        }
        validate_requirement(side, price, quantity.protocol_units(), reservation)?;
        Ok(Self {
            client_order,
            instrument,
            side,
            price,
            quantity,
            reservation,
        })
    }

    #[must_use]
    pub const fn client_order(self) -> PmClientOrderKey {
        self.client_order
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct PmOpenOrderReservation {
    venue_order: PmVenueOrderKey,
    reservation: PmReservationKnowledge,
}

impl PmOpenOrderReservation {
    #[must_use]
    pub const fn new(venue_order: PmVenueOrderKey, reservation: PmReservationKnowledge) -> Self {
        Self {
            venue_order,
            reservation,
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum PmOrderOwnership {
    ProvenOwned,
    Unmanaged,
    Ambiguous,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct PmOrderProjection {
    identity: PmOrderIdentity,
    instrument: PmInstrumentHandle,
    status: Option<PmOrderStatus>,
    ownership: PmOrderOwnership,
    reservation: Option<PmReservationKnowledge>,
    missing_from_complete_open_snapshot: bool,
    terminal_by_detail_absence: bool,
    last_occurrence: Option<PmPrivateOccurrence>,
}

impl PmOrderProjection {
    #[must_use]
    pub const fn identity(self) -> PmOrderIdentity {
        self.identity
    }

    #[must_use]
    pub const fn instrument(self) -> PmInstrumentHandle {
        self.instrument
    }

    #[must_use]
    pub const fn status(self) -> Option<PmOrderStatus> {
        self.status
    }

    #[must_use]
    pub const fn ownership(self) -> PmOrderOwnership {
        self.ownership
    }

    #[must_use]
    pub const fn reservation(self) -> Option<PmReservationKnowledge> {
        self.reservation
    }

    #[must_use]
    pub const fn missing_from_complete_open_snapshot(self) -> bool {
        self.missing_from_complete_open_snapshot
    }

    #[must_use]
    pub const fn terminal_by_detail_absence(self) -> bool {
        self.terminal_by_detail_absence
    }

    #[must_use]
    pub const fn last_occurrence(self) -> Option<PmPrivateOccurrence> {
        self.last_occurrence
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum PmOrderApply {
    Inserted,
    Updated,
    TerminalReservationReleased,
    IgnoredStale,
    Duplicate,
    DetailAbsenceTerminalized,
    DetailAbsenceIgnoredAfterLaterEvent,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum PmOpenOrdersApply {
    Applied {
        inserted: u16,
        updated: u16,
        retained_missing: u16,
    },
    Duplicate,
    IgnoredStale,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub struct PmOrderCounters {
    registrations: u64,
    observations: u64,
    duplicates: u64,
    stale: u64,
    terminal_releases: u64,
    retained_missing: u64,
    unmanaged: u64,
    ambiguous: u64,
    capacity_failures: u64,
    contract_violations: u64,
}

impl PmOrderCounters {
    #[must_use]
    pub const fn observations(self) -> u64 {
        self.observations
    }

    #[must_use]
    pub const fn terminal_releases(self) -> u64 {
        self.terminal_releases
    }

    #[must_use]
    pub const fn retained_missing(self) -> u64 {
        self.retained_missing
    }

    #[must_use]
    pub const fn capacity_failures(self) -> u64 {
        self.capacity_failures
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Error)]
pub enum PmOrderStateError {
    #[error("reservation amount must be positive")]
    ZeroReservation,
    #[error("owned reservation requires an explicit policy-approved worst-case bound")]
    OwnedReservationRequiresPolicy,
    #[error("order or reservation belongs to another configured scope")]
    ScopeMismatch,
    #[error("private order violates the locally configured tick, lot, or minimum")]
    MarketContractMismatch,
    #[error("reservation asset/side differs from the configured PM resource")]
    ReservationScopeMismatch,
    #[error("reservation understates the exact base collateral or outcome-token requirement")]
    ReservationUnderstatesBase,
    #[error("an authoritative-remaining reservation is valid only for a sell")]
    InvalidAuthoritativeReservation,
    #[error("owned order was observed before local ownership was registered")]
    OwnershipRegisteredTooLate,
    #[error("order identity overlaps a different canonical order")]
    IdentityConflict,
    #[error("the same ingress sequence carries conflicting order observations")]
    ObservationConflict,
    #[error("private order lifecycle moved backwards or changed immutable order terms")]
    LifecycleRegression,
    #[error("a complete open-orders snapshot contained a terminal order")]
    TerminalOrderInOpenSnapshot,
    #[error("canonical PM order storage reached its fixed bound")]
    Capacity,
    #[error("open-order snapshot envelope revision differs from aggregate revision")]
    EnvelopeRevisionMismatch,
    #[error("open-order/detail envelope ingress differs from reconciliation completion")]
    CompletionSequenceMismatch,
    #[error("open-order reservation companion is duplicated or not present in the snapshot")]
    ReservationCompanionMismatch,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum PmReservationTotalsError {
    Unknown(PmOrderIdentity),
    Overflow,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum OwnershipState {
    ProvenOwned,
    Unmanaged,
    Ambiguous,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
struct OrderEntry {
    identity: PmOrderIdentity,
    instrument: PmInstrumentHandle,
    event: Option<PmOrderEvent>,
    ownership: OwnershipState,
    registered_terms: Option<(PmOrderSide, PmPrice, PmQuantity)>,
    reservation: Option<PmReservationKnowledge>,
    missing_from_complete_open_snapshot: bool,
    terminal_by_detail_absence: bool,
    last_occurrence: Option<PmPrivateOccurrence>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum OrderOverlap {
    None,
    One(usize),
    Bridge(usize, usize),
}

impl OrderEntry {
    fn projection(self) -> PmOrderProjection {
        PmOrderProjection {
            identity: self.identity,
            instrument: self.instrument,
            status: self.event.map(|event| event.progress().status()),
            ownership: match self.ownership {
                OwnershipState::ProvenOwned => PmOrderOwnership::ProvenOwned,
                OwnershipState::Unmanaged => PmOrderOwnership::Unmanaged,
                OwnershipState::Ambiguous => PmOrderOwnership::Ambiguous,
            },
            reservation: self.reservation,
            missing_from_complete_open_snapshot: self.missing_from_complete_open_snapshot,
            terminal_by_detail_absence: self.terminal_by_detail_absence,
            last_occurrence: self.last_occurrence,
        }
    }

    fn is_live(self) -> bool {
        !self.terminal_by_detail_absence
            && self
                .event
                .is_none_or(|event| !event.progress().status().is_terminal())
    }
}

pub(crate) struct PmOrderState {
    entries: Vec<OrderEntry>,
    canonical_index: Vec<u16>,
    client_index: Vec<u16>,
    live_count: u16,
    latest_snapshot_revision: Option<reap_pm_core::SnapshotRevision>,
    latest_snapshot_completion: Option<PmPrivateOccurrence>,
    observed_monotonic_ns: Option<u64>,
    counters: PmOrderCounters,
}

impl PmOrderState {
    pub(crate) fn new() -> Self {
        Self {
            entries: Vec::with_capacity(MAX_PM_PRIVATE_ORDERS),
            canonical_index: Vec::with_capacity(MAX_PM_PRIVATE_ORDERS),
            client_index: Vec::with_capacity(MAX_PM_PRIVATE_ORDERS),
            live_count: 0,
            latest_snapshot_revision: None,
            latest_snapshot_completion: None,
            observed_monotonic_ns: None,
            counters: PmOrderCounters::default(),
        }
    }

    pub(crate) fn reserved_capacity_bytes(&self) -> usize {
        self.entries
            .capacity()
            .saturating_mul(std::mem::size_of::<OrderEntry>())
            .saturating_add(
                self.canonical_index
                    .capacity()
                    .saturating_mul(std::mem::size_of::<u16>()),
            )
            .saturating_add(
                self.client_index
                    .capacity()
                    .saturating_mul(std::mem::size_of::<u16>()),
            )
    }

    pub(crate) fn register_owned(
        &mut self,
        registration: PmOwnedOrderRegistration,
        config: &PmPrivateStateConfig,
    ) -> Result<(), PmOrderStateError> {
        validate_registration(registration, config)?;
        let identity = PmOrderIdentity::new(Some(registration.client_order), None)
            .expect("registration always carries a client key");
        if let Some(index) = self.find(identity) {
            let entry = self.entries[index];
            if entry.ownership != OwnershipState::ProvenOwned || entry.event.is_some() {
                return Err(PmOrderStateError::OwnershipRegisteredTooLate);
            }
            if entry.reservation == Some(PmReservationKnowledge::Known(registration.reservation)) {
                return Ok(());
            }
            return Err(PmOrderStateError::IdentityConflict);
        }
        self.insert(OrderEntry {
            identity,
            instrument: registration.instrument,
            event: None,
            ownership: OwnershipState::ProvenOwned,
            registered_terms: Some((registration.side, registration.price, registration.quantity)),
            reservation: Some(PmReservationKnowledge::Known(registration.reservation)),
            missing_from_complete_open_snapshot: false,
            terminal_by_detail_absence: false,
            last_occurrence: None,
        })?;
        self.counters.registrations = self.counters.registrations.saturating_add(1);
        Ok(())
    }

    pub(crate) fn preflight_register_owned(
        &self,
        registration: PmOwnedOrderRegistration,
        config: &PmPrivateStateConfig,
    ) -> Result<(), PmOrderStateError> {
        self.prepare_owned_registration(registration, config)
            .map(drop)
    }

    pub(crate) fn bind_owned_venue(
        &mut self,
        client_order: PmClientOrderKey,
        venue_order: PmVenueOrderKey,
        config: &PmPrivateStateConfig,
    ) -> Result<(), PmOrderStateError> {
        if client_order.account() != config.account() || venue_order.account() != config.account() {
            return Err(PmOrderStateError::ScopeMismatch);
        }
        let identity = PmOrderIdentity::new(Some(client_order), Some(venue_order))
            .expect("validated same-account owned identity");
        match self.overlaps(identity)? {
            OrderOverlap::None => return Err(PmOrderStateError::OwnershipRegisteredTooLate),
            OrderOverlap::Bridge(first, second) => self.coalesce_entries(first, second)?,
            OrderOverlap::One(_) => {}
        }
        let index = self
            .find(identity)
            .ok_or(PmOrderStateError::OwnershipRegisteredTooLate)?;
        let entry = self.entries[index];
        if entry.ownership != OwnershipState::ProvenOwned || entry.registered_terms.is_none() {
            return Err(PmOrderStateError::OwnershipRegisteredTooLate);
        }
        ensure_identity_merge(entry.identity, identity)?;
        let mut bound = entry;
        bound.identity = merge_identity(entry.identity, identity);
        self.replace_ordered(index, bound)?;
        Ok(())
    }

    pub(crate) fn preflight_bind_owned_venue(
        &self,
        client_order: PmClientOrderKey,
        venue_order: PmVenueOrderKey,
        config: &PmPrivateStateConfig,
    ) -> Result<(), PmOrderStateError> {
        if client_order.account() != config.account() || venue_order.account() != config.account() {
            return Err(PmOrderStateError::ScopeMismatch);
        }
        let identity = PmOrderIdentity::new(Some(client_order), Some(venue_order))
            .expect("validated same-account owned identity");
        let entry = match self.overlaps(identity)? {
            OrderOverlap::None => return Err(PmOrderStateError::OwnershipRegisteredTooLate),
            OrderOverlap::One(index) => self.entries[index],
            OrderOverlap::Bridge(first, second) => {
                merge_entries(self.entries[first], self.entries[second])?
            }
        };
        if entry.ownership != OwnershipState::ProvenOwned || entry.registered_terms.is_none() {
            return Err(PmOrderStateError::OwnershipRegisteredTooLate);
        }
        ensure_identity_merge(entry.identity, identity)
    }

    pub(crate) fn compact_proven_owned(
        &mut self,
        client_order: PmClientOrderKey,
    ) -> Result<(), PmOrderStateError> {
        let identity = PmOrderIdentity::new(Some(client_order), None)
            .expect("client order is a complete structural identity");
        let index = self
            .find(identity)
            .ok_or(PmOrderStateError::OwnershipRegisteredTooLate)?;
        let entry = self.entries[index];
        if entry.ownership != OwnershipState::ProvenOwned || entry.registered_terms.is_none() {
            return Err(PmOrderStateError::OwnershipRegisteredTooLate);
        }
        self.remove_dense(index);
        Ok(())
    }

    pub(crate) fn preflight_compact_proven_owned(
        &self,
        client_order: PmClientOrderKey,
    ) -> Result<(), PmOrderStateError> {
        let identity = PmOrderIdentity::new(Some(client_order), None)
            .expect("client order is a complete structural identity");
        let index = self
            .find(identity)
            .ok_or(PmOrderStateError::OwnershipRegisteredTooLate)?;
        let entry = self.entries[index];
        if entry.ownership != OwnershipState::ProvenOwned || entry.registered_terms.is_none() {
            Err(PmOrderStateError::OwnershipRegisteredTooLate)
        } else {
            Ok(())
        }
    }

    pub(crate) fn observe(
        &mut self,
        envelope: EventEnvelope<PmOrderEvent>,
        knowledge: PmRemoteOrderKnowledge,
        config: &PmPrivateStateConfig,
    ) -> Result<PmOrderApply, PmOrderStateError> {
        let event = *envelope.payload();
        validate_event(event, envelope.source(), config)?;
        let reservation = validate_remote_knowledge(event, knowledge, config)?;
        let occurrence = PmPrivateOccurrence::new(
            envelope.ordering().connection_epoch(),
            envelope.ordering().local_ingress_sequence(),
        );
        self.observed_monotonic_ns = Some(envelope.clock().monotonic_service_ns());
        self.counters.observations = self.counters.observations.saturating_add(1);
        match self.overlaps(event.order())? {
            OrderOverlap::None => {
                let ownership = ownership_from_remote(knowledge);
                self.insert(OrderEntry {
                    identity: event.order(),
                    instrument: event.instrument(),
                    event: Some(event),
                    ownership,
                    registered_terms: None,
                    reservation: live_reservation(event, reservation),
                    missing_from_complete_open_snapshot: false,
                    terminal_by_detail_absence: false,
                    last_occurrence: Some(occurrence),
                })?;
                self.count_remote(ownership);
                Ok(if event.progress().status().is_terminal() {
                    PmOrderApply::TerminalReservationReleased
                } else {
                    PmOrderApply::Inserted
                })
            }
            OrderOverlap::One(index) => {
                self.update_existing(index, event, occurrence, knowledge, reservation)
            }
            OrderOverlap::Bridge(first, second) => {
                let merged = merge_entries(self.entries[first], self.entries[second])?;
                preflight_existing_update(merged, event, occurrence)?;
                self.coalesce_entries(first, second)?;
                let index = self
                    .find(event.order())
                    .expect("coalesced order retains the bridge identity");
                self.update_existing(index, event, occurrence, knowledge, reservation)
            }
        }
    }

    pub(crate) fn apply_open_snapshot(
        &mut self,
        envelope: &EventEnvelope<PmCompleteOpenOrdersSnapshot>,
        reservations: &[PmOpenOrderReservation],
        config: &PmPrivateStateConfig,
    ) -> Result<PmOpenOrdersApply, PmOrderStateError> {
        validate_open_snapshot(envelope, reservations, config)?;
        let snapshot = envelope.payload();
        let epoch = envelope.ordering().connection_epoch();
        if let Some(outcome) = self.snapshot_stale_or_duplicate(snapshot, epoch) {
            return Ok(outcome);
        }
        self.preflight_fresh_open_snapshot(envelope, reservations, config)?;
        self.apply_fresh_open_snapshot(envelope, reservations, config)
    }

    fn preflight_fresh_open_snapshot(
        &mut self,
        envelope: &EventEnvelope<PmCompleteOpenOrdersSnapshot>,
        reservations: &[PmOpenOrderReservation],
        config: &PmPrivateStateConfig,
    ) -> Result<(), PmOrderStateError> {
        let snapshot = envelope.payload();
        let epoch = envelope.ordering().connection_epoch();
        self.preflight_snapshot_capacity(snapshot)?;
        let request = PmPrivateOccurrence::new(epoch, snapshot.boundary().request_sequence());
        for event in snapshot.orders().iter().copied() {
            let reservation = companion_knowledge(event, reservations);
            validate_event(event, config.source(), config)?;
            validate_reservation(event, reservation, config)?;
            match self.overlaps(event.order())? {
                OrderOverlap::None => {}
                OrderOverlap::One(index) => {
                    preflight_existing_update(self.entries[index], event, request)?;
                }
                OrderOverlap::Bridge(first, second) => {
                    let merged = merge_entries(self.entries[first], self.entries[second])?;
                    preflight_existing_update(merged, event, request)?;
                }
            }
        }
        Ok(())
    }

    fn apply_fresh_open_snapshot(
        &mut self,
        envelope: &EventEnvelope<PmCompleteOpenOrdersSnapshot>,
        reservations: &[PmOpenOrderReservation],
        config: &PmPrivateStateConfig,
    ) -> Result<PmOpenOrdersApply, PmOrderStateError> {
        let snapshot = envelope.payload();
        let epoch = envelope.ordering().connection_epoch();
        self.preflight_snapshot_capacity(snapshot)?;
        let request = PmPrivateOccurrence::new(epoch, snapshot.boundary().request_sequence());
        let completion = PmPrivateOccurrence::new(epoch, snapshot.boundary().completion_sequence());
        let monotonic_ns = envelope.clock().monotonic_service_ns();
        let mut inserted = 0_u16;
        let mut updated = 0_u16;
        for event in snapshot.orders().iter().copied() {
            let knowledge = companion_knowledge(event, reservations);
            match self.reduce_snapshot_row(event, request, knowledge, config)? {
                PmOrderApply::Inserted => inserted = inserted.saturating_add(1),
                PmOrderApply::Updated => updated = updated.saturating_add(1),
                PmOrderApply::IgnoredStale
                | PmOrderApply::Duplicate
                | PmOrderApply::TerminalReservationReleased
                | PmOrderApply::DetailAbsenceTerminalized
                | PmOrderApply::DetailAbsenceIgnoredAfterLaterEvent => {}
            }
        }
        let retained_missing = self.mark_missing(snapshot, request);
        self.latest_snapshot_revision = Some(snapshot.snapshot().revision());
        self.latest_snapshot_completion = Some(completion);
        self.observed_monotonic_ns = Some(monotonic_ns);
        self.counters.retained_missing = self
            .counters
            .retained_missing
            .saturating_add(u64::from(retained_missing));
        Ok(PmOpenOrdersApply::Applied {
            inserted,
            updated,
            retained_missing,
        })
    }

    pub(crate) fn apply_detail(
        &mut self,
        envelope: EventEnvelope<PmExactOrderDetail>,
        reservation: PmReservationKnowledge,
        config: &PmPrivateStateConfig,
    ) -> Result<PmOrderApply, PmOrderStateError> {
        validate_detail(&envelope, config)?;
        let detail = *envelope.payload();
        self.observed_monotonic_ns = Some(envelope.clock().monotonic_service_ns());
        if let Some(event) = detail.order() {
            let knowledge = PmRemoteOrderKnowledge::Unmanaged(reservation);
            let reservation = validate_remote_knowledge(event, knowledge, config)?;
            let request = PmPrivateOccurrence::new(
                envelope.ordering().connection_epoch(),
                detail.boundary().request_sequence(),
            );
            return self.reduce_snapshot_row(event, request, reservation, config);
        }
        let identity = PmOrderIdentity::new(None, Some(detail.requested_order()))
            .expect("detail carries venue key");
        let Some(index) = self.find(identity) else {
            return Ok(PmOrderApply::Duplicate);
        };
        let epoch = envelope.ordering().connection_epoch();
        let request = PmPrivateOccurrence::new(epoch, detail.boundary().request_sequence());
        if self.entries[index]
            .last_occurrence
            .is_some_and(|occurrence| occurrence > request)
        {
            return Ok(PmOrderApply::DetailAbsenceIgnoredAfterLaterEvent);
        }
        if !self.entries[index].is_live() {
            return Ok(PmOrderApply::Duplicate);
        }
        self.live_count = self
            .live_count
            .checked_sub(1)
            .expect("a live detail row contributes to the exact live count");
        self.entries[index].reservation = None;
        self.entries[index].terminal_by_detail_absence = true;
        self.entries[index].last_occurrence = Some(PmPrivateOccurrence::new(
            epoch,
            detail.boundary().completion_sequence(),
        ));
        self.counters.terminal_releases = self.counters.terminal_releases.saturating_add(1);
        Ok(PmOrderApply::DetailAbsenceTerminalized)
    }

    pub(crate) fn projections(&self) -> impl Iterator<Item = PmOrderProjection> + '_ {
        self.canonical_entries()
            .copied()
            .map(OrderEntry::projection)
    }

    pub(crate) fn ownership(&self, identity: PmOrderIdentity) -> Option<PmOrderOwnership> {
        self.find(identity)
            .map(|index| match self.entries[index].ownership {
                OwnershipState::ProvenOwned => PmOrderOwnership::ProvenOwned,
                OwnershipState::Unmanaged => PmOrderOwnership::Unmanaged,
                OwnershipState::Ambiguous => PmOrderOwnership::Ambiguous,
            })
    }

    pub(crate) fn owned_live_venue_orders(&self) -> impl Iterator<Item = PmVenueOrderKey> + '_ {
        self.canonical_entries()
            .filter(|entry| entry.ownership == OwnershipState::ProvenOwned && entry.is_live())
            .filter_map(|entry| entry.identity.venue_order_key())
    }

    pub(crate) const fn observed_monotonic_ns(&self) -> Option<u64> {
        self.observed_monotonic_ns
    }

    pub(crate) fn invalidate_freshness(&mut self) {
        self.observed_monotonic_ns = None;
    }

    pub(crate) const fn counters(&self) -> PmOrderCounters {
        self.counters
    }

    fn insert(&mut self, entry: OrderEntry) -> Result<(), PmOrderStateError> {
        if self.entries.len() == MAX_PM_PRIVATE_ORDERS {
            self.counters.capacity_failures = self.counters.capacity_failures.saturating_add(1);
            return Err(PmOrderStateError::Capacity);
        }
        self.insert_ordered(entry)
    }

    fn coalesce_entries(&mut self, first: usize, second: usize) -> Result<(), PmOrderStateError> {
        let left = self.entries[first];
        let right = self.entries[second];
        let merged = merge_entries(left, right)?;
        let (lower, upper) = if first < second {
            (first, second)
        } else {
            (second, first)
        };
        if self.entries.iter().enumerate().any(|(index, entry)| {
            index != lower && index != upper && compare_entries(entry, &merged) == Ordering::Equal
        }) {
            return Err(PmOrderStateError::IdentityConflict);
        }
        if let Some(merged_client) = merged.identity.client_order_key()
            && self.entries.iter().enumerate().any(|(index, entry)| {
                index != lower
                    && index != upper
                    && entry.identity.client_order_key() == Some(merged_client)
            })
        {
            return Err(PmOrderStateError::IdentityConflict);
        }
        self.remove_dense(upper);
        self.remove_dense(lower);
        self.insert_ordered(merged)
    }

    fn update_existing(
        &mut self,
        index: usize,
        event: PmOrderEvent,
        occurrence: PmPrivateOccurrence,
        knowledge: PmRemoteOrderKnowledge,
        reservation: PmReservationKnowledge,
    ) -> Result<PmOrderApply, PmOrderStateError> {
        let prior = self.entries[index];
        if prior
            .last_occurrence
            .is_some_and(|prior_occurrence| occurrence < prior_occurrence)
        {
            self.counters.stale = self.counters.stale.saturating_add(1);
            return Ok(PmOrderApply::IgnoredStale);
        }
        if prior.last_occurrence == Some(occurrence) && prior.event == Some(event) {
            self.counters.duplicates = self.counters.duplicates.saturating_add(1);
            return Ok(PmOrderApply::Duplicate);
        }
        if prior.last_occurrence == Some(occurrence) {
            self.counters.contract_violations = self.counters.contract_violations.saturating_add(1);
            return Err(PmOrderStateError::ObservationConflict);
        }
        ensure_identity_merge(prior.identity, event.order())?;
        if let Some(previous) = prior.event
            && validate_lifecycle(previous, event).is_err()
        {
            return Err(PmOrderStateError::LifecycleRegression);
        }
        if let Some((side, price, quantity)) = prior.registered_terms
            && (event.side() != side
                || event.price() != price
                || event.progress().original_quantity() != quantity)
        {
            return Err(PmOrderStateError::ObservationConflict);
        }
        let was_live = prior.is_live();
        let terminal = event.progress().status().is_terminal();
        let mut entry = prior;
        entry.identity = merge_identity(entry.identity, event.order());
        entry.event = Some(event);
        entry.last_occurrence = Some(occurrence);
        entry.missing_from_complete_open_snapshot = false;
        entry.terminal_by_detail_absence = false;
        if entry.ownership != OwnershipState::ProvenOwned {
            entry.ownership = merge_remote_ownership(entry.ownership, knowledge);
            entry.reservation = live_reservation(event, reservation);
        } else if terminal {
            entry.reservation = None;
        }
        self.replace_ordered(index, entry)?;
        if was_live && terminal {
            self.counters.terminal_releases = self.counters.terminal_releases.saturating_add(1);
            Ok(PmOrderApply::TerminalReservationReleased)
        } else {
            Ok(PmOrderApply::Updated)
        }
    }

    fn snapshot_stale_or_duplicate(
        &mut self,
        snapshot: &PmCompleteOpenOrdersSnapshot,
        epoch: reap_pm_core::ConnectionEpoch,
    ) -> Option<PmOpenOrdersApply> {
        let revision = snapshot.snapshot().revision();
        let completion = PmPrivateOccurrence::new(epoch, snapshot.boundary().completion_sequence());
        if self.latest_snapshot_revision == Some(revision)
            && self.latest_snapshot_completion == Some(completion)
        {
            self.counters.duplicates = self.counters.duplicates.saturating_add(1);
            return Some(PmOpenOrdersApply::Duplicate);
        }
        let stale_occurrence = self
            .latest_snapshot_completion
            .is_some_and(|prior| completion <= prior);
        let stale_revision_same_epoch = self.latest_snapshot_completion.is_some_and(|prior| {
            prior.epoch() == epoch
                && self
                    .latest_snapshot_revision
                    .is_some_and(|prior_revision| revision <= prior_revision)
        });
        if stale_occurrence || stale_revision_same_epoch {
            self.counters.stale = self.counters.stale.saturating_add(1);
            return Some(PmOpenOrdersApply::IgnoredStale);
        }
        None
    }

    fn preflight_snapshot_capacity(
        &mut self,
        snapshot: &PmCompleteOpenOrdersSnapshot,
    ) -> Result<(), PmOrderStateError> {
        let new = snapshot
            .orders()
            .iter()
            .filter(|event| self.find(event.order()).is_none())
            .count();
        if self.entries.len().saturating_add(new) > MAX_PM_PRIVATE_ORDERS {
            self.counters.capacity_failures = self.counters.capacity_failures.saturating_add(1);
            Err(PmOrderStateError::Capacity)
        } else {
            Ok(())
        }
    }

    fn reduce_snapshot_row(
        &mut self,
        event: PmOrderEvent,
        request: PmPrivateOccurrence,
        reservation: PmReservationKnowledge,
        config: &PmPrivateStateConfig,
    ) -> Result<PmOrderApply, PmOrderStateError> {
        validate_event(event, config.source(), config)?;
        validate_reservation(event, reservation, config)?;
        let knowledge = PmRemoteOrderKnowledge::Unmanaged(reservation);
        match self.overlaps(event.order())? {
            OrderOverlap::One(index) => {
                if self.entries[index]
                    .last_occurrence
                    .is_some_and(|occurrence| occurrence > request)
                {
                    return Ok(PmOrderApply::IgnoredStale);
                }
                return self.update_existing(index, event, request, knowledge, reservation);
            }
            OrderOverlap::Bridge(first, second) => {
                if [first, second].into_iter().any(|index| {
                    self.entries[index]
                        .last_occurrence
                        .is_some_and(|occurrence| occurrence > request)
                }) {
                    return Ok(PmOrderApply::IgnoredStale);
                }
                self.coalesce_entries(first, second)?;
                let index = self
                    .find(event.order())
                    .expect("coalesced order retains the bridge identity");
                return self.update_existing(index, event, request, knowledge, reservation);
            }
            OrderOverlap::None => {}
        }
        self.insert(OrderEntry {
            identity: event.order(),
            instrument: event.instrument(),
            event: Some(event),
            ownership: OwnershipState::Unmanaged,
            registered_terms: None,
            reservation: live_reservation(event, reservation),
            missing_from_complete_open_snapshot: false,
            terminal_by_detail_absence: false,
            last_occurrence: Some(request),
        })?;
        self.count_remote(OwnershipState::Unmanaged);
        Ok(PmOrderApply::Inserted)
    }

    fn mark_missing(
        &mut self,
        snapshot: &PmCompleteOpenOrdersSnapshot,
        request: PmPrivateOccurrence,
    ) -> u16 {
        let mut retained = 0_u16;
        for entry in &mut self.entries {
            if !entry.is_live()
                || entry
                    .last_occurrence
                    .is_some_and(|occurrence| occurrence > request)
                || snapshot
                    .orders()
                    .iter()
                    .any(|event| identities_overlap(entry.identity, event.order()))
            {
                continue;
            }
            entry.missing_from_complete_open_snapshot = true;
            retained = retained.saturating_add(1);
        }
        retained
    }

    fn count_remote(&mut self, ownership: OwnershipState) {
        match ownership {
            OwnershipState::ProvenOwned => {}
            OwnershipState::Unmanaged => {
                self.counters.unmanaged = self.counters.unmanaged.saturating_add(1);
            }
            OwnershipState::Ambiguous => {
                self.counters.ambiguous = self.counters.ambiguous.saturating_add(1);
            }
        }
    }
}

fn validate_registration(
    registration: PmOwnedOrderRegistration,
    config: &PmPrivateStateConfig,
) -> Result<(), PmOrderStateError> {
    if registration.client_order.account() != config.account()
        || registration.instrument != config.instrument()
    {
        return Err(PmOrderStateError::ScopeMismatch);
    }
    Ok(())
}

fn preflight_existing_update(
    prior: OrderEntry,
    event: PmOrderEvent,
    occurrence: PmPrivateOccurrence,
) -> Result<(), PmOrderStateError> {
    if prior
        .last_occurrence
        .is_some_and(|prior_occurrence| occurrence < prior_occurrence)
        || (prior.last_occurrence == Some(occurrence) && prior.event == Some(event))
    {
        return Ok(());
    }
    if prior.last_occurrence == Some(occurrence) {
        return Err(PmOrderStateError::ObservationConflict);
    }
    ensure_identity_merge(prior.identity, event.order())?;
    if let Some(previous) = prior.event {
        validate_lifecycle(previous, event)?;
    }
    if let Some((side, price, quantity)) = prior.registered_terms
        && (event.side() != side
            || event.price() != price
            || event.progress().original_quantity() != quantity)
    {
        return Err(PmOrderStateError::ObservationConflict);
    }
    Ok(())
}

fn validate_event(
    event: PmOrderEvent,
    source: reap_pm_core::PmProductSource,
    config: &PmPrivateStateConfig,
) -> Result<(), PmOrderStateError> {
    if source != config.source()
        || event.source() != config.source()
        || event.account() != config.account()
        || event.instrument() != config.instrument()
    {
        return Err(PmOrderStateError::ScopeMismatch);
    }
    event
        .price()
        .validate_tick(config.tick())
        .and_then(|_| {
            event
                .progress()
                .original_quantity()
                .validate_order(config.minimum_order_size())
        })
        .map_err(|_| PmOrderStateError::MarketContractMismatch)?;
    Ok(())
}

fn validate_remote_knowledge(
    event: PmOrderEvent,
    knowledge: PmRemoteOrderKnowledge,
    config: &PmPrivateStateConfig,
) -> Result<PmReservationKnowledge, PmOrderStateError> {
    match knowledge {
        PmRemoteOrderKnowledge::Ambiguous => Ok(PmReservationKnowledge::Unknown),
        PmRemoteOrderKnowledge::Unmanaged(reservation) => {
            validate_reservation(event, reservation, config)?;
            Ok(reservation)
        }
    }
}

fn validate_reservation(
    event: PmOrderEvent,
    knowledge: PmReservationKnowledge,
    _config: &PmPrivateStateConfig,
) -> Result<(), PmOrderStateError> {
    let PmReservationKnowledge::Known(reservation) = knowledge else {
        return Ok(());
    };
    if event.progress().status().is_terminal() {
        return Ok(());
    }
    validate_requirement(
        event.side(),
        event.price(),
        event.progress().remaining_quantity_units(),
        reservation,
    )
}

fn validate_requirement(
    side: PmOrderSide,
    price: PmPrice,
    remaining: U256,
    reservation: PmExactReservation,
) -> Result<(), PmOrderStateError> {
    let quantity = PmQuantity::from_protocol_units(remaining)
        .map_err(|_| PmOrderStateError::ZeroReservation)?;
    let amounts = exact_order_amounts(side, price, quantity)
        .map_err(|_| PmOrderStateError::ReservationUnderstatesBase)?;
    let (base_collateral, base_outcome) = match side {
        PmOrderSide::Buy => (amounts.maker(), U256::ZERO),
        PmOrderSide::Sell => (U256::ZERO, remaining),
    };
    if reservation.collateral() < base_collateral || reservation.outcome() < base_outcome {
        return Err(PmOrderStateError::ReservationUnderstatesBase);
    }
    if reservation.basis() == PmReservationBasis::AuthoritativeSellRemaining
        && (side != PmOrderSide::Sell
            || !reservation.collateral().is_zero()
            || reservation.outcome() != remaining)
    {
        return Err(PmOrderStateError::InvalidAuthoritativeReservation);
    }
    Ok(())
}

fn ownership_from_remote(knowledge: PmRemoteOrderKnowledge) -> OwnershipState {
    match knowledge {
        PmRemoteOrderKnowledge::Unmanaged(_) => OwnershipState::Unmanaged,
        PmRemoteOrderKnowledge::Ambiguous => OwnershipState::Ambiguous,
    }
}

fn merge_remote_ownership(
    current: OwnershipState,
    observed: PmRemoteOrderKnowledge,
) -> OwnershipState {
    match (current, observed) {
        (OwnershipState::ProvenOwned, _) => OwnershipState::ProvenOwned,
        (OwnershipState::Ambiguous, _) | (_, PmRemoteOrderKnowledge::Ambiguous) => {
            OwnershipState::Ambiguous
        }
        (OwnershipState::Unmanaged, PmRemoteOrderKnowledge::Unmanaged(_)) => {
            OwnershipState::Unmanaged
        }
    }
}

fn live_reservation(
    event: PmOrderEvent,
    reservation: PmReservationKnowledge,
) -> Option<PmReservationKnowledge> {
    (!event.progress().status().is_terminal()).then_some(reservation)
}

fn identities_overlap(left: PmOrderIdentity, right: PmOrderIdentity) -> bool {
    matches!(
        (left.client_order_key(), right.client_order_key()),
        (Some(left), Some(right)) if left == right
    ) || matches!(
        (left.venue_order_key(), right.venue_order_key()),
        (Some(left), Some(right)) if left == right
    )
}

fn ensure_identity_merge(
    left: PmOrderIdentity,
    right: PmOrderIdentity,
) -> Result<(), PmOrderStateError> {
    if let (Some(left), Some(right)) = (left.client_order_key(), right.client_order_key())
        && left != right
    {
        return Err(PmOrderStateError::IdentityConflict);
    }
    if let (Some(left), Some(right)) = (left.venue_order_key(), right.venue_order_key())
        && left != right
    {
        return Err(PmOrderStateError::IdentityConflict);
    }
    Ok(())
}

fn merge_entries(left: OrderEntry, right: OrderEntry) -> Result<OrderEntry, PmOrderStateError> {
    if left.instrument != right.instrument {
        return Err(PmOrderStateError::IdentityConflict);
    }
    ensure_identity_merge(left.identity, right.identity)?;
    let registered_terms = match (left.registered_terms, right.registered_terms) {
        (Some(left), Some(right)) if left != right => {
            return Err(PmOrderStateError::ObservationConflict);
        }
        (Some(terms), _) | (_, Some(terms)) => Some(terms),
        (None, None) => None,
    };
    let ownership = merge_entry_ownership(left.ownership, right.ownership);
    let ordering = left.last_occurrence.cmp(&right.last_occurrence);
    if let (Some(left_event), Some(right_event)) = (left.event, right.event) {
        match ordering {
            Ordering::Less => validate_lifecycle(left_event, right_event)?,
            Ordering::Greater => validate_lifecycle(right_event, left_event)?,
            Ordering::Equal if left_event != right_event => {
                return Err(PmOrderStateError::ObservationConflict);
            }
            Ordering::Equal => {}
        }
    }
    let selected = match ordering {
        Ordering::Less => right,
        Ordering::Greater => left,
        Ordering::Equal if right.event.is_some() && left.event.is_none() => right,
        Ordering::Equal => left,
    };
    let reservation = if ownership == OwnershipState::ProvenOwned {
        merge_owned_reservation(left, right)?
    } else if ordering == Ordering::Equal && left.reservation != right.reservation {
        Some(PmReservationKnowledge::Unknown)
    } else {
        selected.reservation
    };
    Ok(OrderEntry {
        identity: merge_identity(left.identity, right.identity),
        instrument: left.instrument,
        event: selected.event,
        ownership,
        registered_terms,
        reservation,
        missing_from_complete_open_snapshot: selected.missing_from_complete_open_snapshot,
        terminal_by_detail_absence: selected.terminal_by_detail_absence,
        last_occurrence: selected.last_occurrence,
    })
}

fn merge_entry_ownership(left: OwnershipState, right: OwnershipState) -> OwnershipState {
    match (left, right) {
        (OwnershipState::ProvenOwned, _) | (_, OwnershipState::ProvenOwned) => {
            OwnershipState::ProvenOwned
        }
        (OwnershipState::Ambiguous, _) | (_, OwnershipState::Ambiguous) => {
            OwnershipState::Ambiguous
        }
        (OwnershipState::Unmanaged, OwnershipState::Unmanaged) => OwnershipState::Unmanaged,
    }
}

fn merge_owned_reservation(
    left: OrderEntry,
    right: OrderEntry,
) -> Result<Option<PmReservationKnowledge>, PmOrderStateError> {
    match (
        (left.ownership == OwnershipState::ProvenOwned).then_some(left.reservation),
        (right.ownership == OwnershipState::ProvenOwned).then_some(right.reservation),
    ) {
        (Some(left), Some(right)) if left != right => Err(PmOrderStateError::IdentityConflict),
        (Some(reservation), _) | (_, Some(reservation)) => Ok(reservation),
        (None, None) => unreachable!("proven ownership must come from one merged entry"),
    }
}

fn validate_lifecycle(previous: PmOrderEvent, next: PmOrderEvent) -> Result<(), PmOrderStateError> {
    if previous.source() != next.source()
        || previous.instrument() != next.instrument()
        || previous.side() != next.side()
        || previous.price() != next.price()
        || previous.progress().original_quantity() != next.progress().original_quantity()
        || next.progress().cumulative_filled() < previous.progress().cumulative_filled()
        || (previous.progress().status().is_terminal() && previous.progress() != next.progress())
    {
        Err(PmOrderStateError::LifecycleRegression)
    } else {
        Ok(())
    }
}

fn merge_identity(left: PmOrderIdentity, right: PmOrderIdentity) -> PmOrderIdentity {
    PmOrderIdentity::new(
        left.client_order_key().or(right.client_order_key()),
        left.venue_order_key().or(right.venue_order_key()),
    )
    .expect("same-account order identities")
}

fn compare_entries(left: &OrderEntry, right: &OrderEntry) -> Ordering {
    left.identity
        .venue_order_key()
        .cmp(&right.identity.venue_order_key())
        .then_with(|| {
            left.identity
                .client_order_key()
                .cmp(&right.identity.client_order_key())
        })
}

fn validate_open_snapshot(
    envelope: &EventEnvelope<PmCompleteOpenOrdersSnapshot>,
    reservations: &[PmOpenOrderReservation],
    config: &PmPrivateStateConfig,
) -> Result<(), PmOrderStateError> {
    let snapshot = envelope.payload();
    if envelope.source() != config.source() || snapshot.account_scope() != config.account_scope() {
        return Err(PmOrderStateError::ScopeMismatch);
    }
    if envelope.ordering().snapshot_revision() != Some(snapshot.snapshot().revision()) {
        return Err(PmOrderStateError::EnvelopeRevisionMismatch);
    }
    if envelope.ordering().local_ingress_sequence() != snapshot.boundary().completion_sequence() {
        return Err(PmOrderStateError::CompletionSequenceMismatch);
    }
    for (index, reservation) in reservations.iter().enumerate() {
        if reservations[..index]
            .iter()
            .any(|prior| prior.venue_order == reservation.venue_order)
            || !snapshot
                .orders()
                .iter()
                .any(|event| event.order().venue_order_key() == Some(reservation.venue_order))
        {
            return Err(PmOrderStateError::ReservationCompanionMismatch);
        }
    }
    for event in snapshot.orders().iter().copied() {
        if event.progress().status().is_terminal() {
            return Err(PmOrderStateError::TerminalOrderInOpenSnapshot);
        }
        validate_event(event, envelope.source(), config)?;
        validate_reservation(event, companion_knowledge(event, reservations), config)?;
    }
    Ok(())
}

fn companion_knowledge(
    event: PmOrderEvent,
    reservations: &[PmOpenOrderReservation],
) -> PmReservationKnowledge {
    event
        .order()
        .venue_order_key()
        .and_then(|venue| reservations.iter().find(|entry| entry.venue_order == venue))
        .map_or(PmReservationKnowledge::Unknown, |entry| entry.reservation)
}

fn validate_detail(
    envelope: &EventEnvelope<PmExactOrderDetail>,
    config: &PmPrivateStateConfig,
) -> Result<(), PmOrderStateError> {
    let detail = envelope.payload();
    if envelope.source() != config.source() || detail.account_scope() != config.account_scope() {
        return Err(PmOrderStateError::ScopeMismatch);
    }
    if envelope.ordering().snapshot_revision() != Some(detail.snapshot().revision()) {
        return Err(PmOrderStateError::EnvelopeRevisionMismatch);
    }
    if envelope.ordering().local_ingress_sequence() != detail.boundary().completion_sequence() {
        return Err(PmOrderStateError::CompletionSequenceMismatch);
    }
    Ok(())
}

#[cfg(test)]
#[path = "order_state_tests.rs"]
mod tests;
