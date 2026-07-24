use std::cmp::Ordering;

use reap_pm_core::{
    ConnectionEpoch, PmAccountScope, PmClientOrderKey, PmFillKey, PmInstrumentHandle,
    PmOrderIdentity, PmOrderProgress, PmOrderSide, PmOrderStatus, PmPrice, PmQuantity,
    PmVenueOrderKey, SnapshotRevision, U256,
};
use thiserror::Error;

use crate::{PmExactReservation, PmPrivateOccurrence, PmReservationBasis};

pub const MAX_PM_OWNED_ORDER_HISTORY: usize = 1_024;
pub const MAX_PM_OWNED_FILL_KEYS: usize = reap_pm_core::MAX_PM_RECONCILIATION_FILLS;

/// Collision-free ordering assigned by the one aggregate owner as it reduces
/// effects and private observations.
///
/// This sequence is deliberately distinct from venue ingress ordering:
/// immediate acknowledgements are not private-connection events.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct PmOwnedReductionSequence(u64);

impl PmOwnedReductionSequence {
    pub fn new(value: u64) -> Result<Self, PmOwnedOrderLifecycleError> {
        if value == 0 {
            Err(PmOwnedOrderLifecycleError::ZeroReductionSequence)
        } else {
            Ok(Self(value))
        }
    }

    #[must_use]
    pub const fn value(self) -> u64 {
        self.0
    }
}

/// Exact owned-reducer occurrence.
///
/// `reduction_sequence` orders every mutation handled by the aggregate.
/// `private_occurrence` and `snapshot_revision` retain original venue evidence
/// when one exists without fabricating it for immediate acknowledgements.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct PmOwnedObservationOccurrence {
    reduction_sequence: PmOwnedReductionSequence,
    private_occurrence: Option<PmPrivateOccurrence>,
    snapshot_revision: Option<SnapshotRevision>,
}

impl PmOwnedObservationOccurrence {
    pub fn new(
        reduction_sequence: PmOwnedReductionSequence,
        private_occurrence: Option<PmPrivateOccurrence>,
        snapshot_revision: Option<SnapshotRevision>,
    ) -> Result<Self, PmOwnedOrderLifecycleError> {
        if snapshot_revision.is_some() && private_occurrence.is_none() {
            return Err(PmOwnedOrderLifecycleError::SnapshotWithoutPrivateOccurrence);
        }
        Ok(Self {
            reduction_sequence,
            private_occurrence,
            snapshot_revision,
        })
    }

    #[must_use]
    pub const fn immediate(reduction_sequence: PmOwnedReductionSequence) -> Self {
        Self {
            reduction_sequence,
            private_occurrence: None,
            snapshot_revision: None,
        }
    }

    #[must_use]
    pub const fn reduction_sequence(self) -> PmOwnedReductionSequence {
        self.reduction_sequence
    }

    #[must_use]
    pub const fn private_occurrence(self) -> Option<PmPrivateOccurrence> {
        self.private_occurrence
    }

    #[must_use]
    pub const fn snapshot_revision(self) -> Option<SnapshotRevision> {
        self.snapshot_revision
    }

    pub(crate) fn causal_cmp(self, other: Self) -> Ordering {
        match (self.private_occurrence, other.private_occurrence) {
            (Some(left), Some(right)) => left.cmp(&right),
            (None, None) | (Some(_), None) | (None, Some(_)) => {
                self.reduction_sequence.cmp(&other.reduction_sequence)
            }
        }
    }

    pub(crate) fn causal_max(self, other: Self) -> Self {
        match self.causal_cmp(other) {
            Ordering::Less => other,
            Ordering::Equal | Ordering::Greater => self,
        }
    }

    pub(crate) const fn with_private_context(
        self,
        private_occurrence: PmPrivateOccurrence,
        snapshot_revision: Option<SnapshotRevision>,
    ) -> Self {
        Self {
            reduction_sequence: self.reduction_sequence,
            private_occurrence: Some(private_occurrence),
            snapshot_revision,
        }
    }

    pub(crate) const fn with_reduction_sequence(
        self,
        reduction_sequence: PmOwnedReductionSequence,
    ) -> Self {
        Self {
            reduction_sequence,
            private_occurrence: self.private_occurrence,
            snapshot_revision: self.snapshot_revision,
        }
    }

    pub(crate) fn same_source_occurrence(self, other: Self) -> bool {
        match (self.private_occurrence, other.private_occurrence) {
            (Some(left), Some(right)) => {
                left == right && self.snapshot_revision == other.snapshot_revision
            }
            (None, None) => self.reduction_sequence == other.reduction_sequence,
            (Some(_), None) | (None, Some(_)) => false,
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct PmOwnedIntentId(u64);

impl PmOwnedIntentId {
    pub fn new(value: u64) -> Result<Self, PmOwnedOrderLifecycleError> {
        if value == 0 {
            Err(PmOwnedOrderLifecycleError::ZeroIntentIdentity)
        } else {
            Ok(Self(value))
        }
    }

    #[must_use]
    pub const fn value(self) -> u64 {
        self.0
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct PmOwnedQuoteSlotKey {
    account_scope: PmAccountScope,
    instrument: PmInstrumentHandle,
    side: PmOrderSide,
}

impl PmOwnedQuoteSlotKey {
    #[must_use]
    pub const fn new(
        account_scope: PmAccountScope,
        instrument: PmInstrumentHandle,
        side: PmOrderSide,
    ) -> Self {
        Self {
            account_scope,
            instrument,
            side,
        }
    }

    #[must_use]
    pub const fn account_scope(self) -> PmAccountScope {
        self.account_scope
    }

    #[must_use]
    pub const fn instrument(self) -> PmInstrumentHandle {
        self.instrument
    }

    #[must_use]
    pub const fn side(self) -> PmOrderSide {
        self.side
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct PmOwnedQuoteIntent {
    intent: PmOwnedIntentId,
    slot: PmOwnedQuoteSlotKey,
    client_order: PmClientOrderKey,
    price: PmPrice,
    quantity: PmQuantity,
    reservation: PmExactReservation,
}

impl PmOwnedQuoteIntent {
    pub fn new(
        intent: PmOwnedIntentId,
        slot: PmOwnedQuoteSlotKey,
        client_order: PmClientOrderKey,
        price: PmPrice,
        quantity: PmQuantity,
        reservation: PmExactReservation,
    ) -> Result<Self, PmOwnedOrderLifecycleError> {
        if client_order.account() != slot.account_scope().handle() {
            return Err(PmOwnedOrderLifecycleError::ScopeMismatch);
        }
        if reservation.basis() != PmReservationBasis::PolicyApprovedWorstCase
            || reservation
                .validate_for(slot.side(), price, quantity)
                .is_err()
        {
            return Err(PmOwnedOrderLifecycleError::InvalidReservation);
        }
        Ok(Self {
            intent,
            slot,
            client_order,
            price,
            quantity,
            reservation,
        })
    }

    #[must_use]
    pub const fn intent(self) -> PmOwnedIntentId {
        self.intent
    }

    #[must_use]
    pub const fn slot(self) -> PmOwnedQuoteSlotKey {
        self.slot
    }

    #[must_use]
    pub const fn client_order(self) -> PmClientOrderKey {
        self.client_order
    }

    #[must_use]
    pub const fn price(self) -> PmPrice {
        self.price
    }

    #[must_use]
    pub const fn quantity(self) -> PmQuantity {
        self.quantity
    }

    #[must_use]
    pub const fn reservation(self) -> PmExactReservation {
        self.reservation
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum PmOwnedSubmitState {
    Pending,
    Accepted,
    Rejected,
    Ambiguous,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum PmOwnedCancelState {
    None,
    Pending,
    Rejected,
    Accepted,
    Ambiguous,
    FilledRace,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum PmOwnedObservationSource {
    ImmediateAcknowledgement,
    PrivateWebSocket,
    RestReconciliation,
}

impl PmOwnedObservationSource {
    const fn bit(self) -> u8 {
        match self {
            Self::ImmediateAcknowledgement => 1,
            Self::PrivateWebSocket => 2,
            Self::RestReconciliation => 4,
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct PmOwnedOrderProjection {
    intent: PmOwnedIntentId,
    slot: PmOwnedQuoteSlotKey,
    client_order: PmClientOrderKey,
    venue_order: Option<PmVenueOrderKey>,
    price: PmPrice,
    quantity: PmQuantity,
    reservation: PmExactReservation,
    submit: PmOwnedSubmitState,
    status: Option<PmOrderStatus>,
    cumulative_filled: U256,
    known_fill_total: U256,
    cancel: PmOwnedCancelState,
    reconciliation_required: bool,
}

impl PmOwnedOrderProjection {
    #[must_use]
    pub const fn intent(self) -> PmOwnedIntentId {
        self.intent
    }

    #[must_use]
    pub const fn slot(self) -> PmOwnedQuoteSlotKey {
        self.slot
    }

    #[must_use]
    pub const fn client_order(self) -> PmClientOrderKey {
        self.client_order
    }

    #[must_use]
    pub const fn venue_order(self) -> Option<PmVenueOrderKey> {
        self.venue_order
    }

    #[must_use]
    pub const fn price(self) -> PmPrice {
        self.price
    }

    #[must_use]
    pub const fn quantity(self) -> PmQuantity {
        self.quantity
    }

    #[must_use]
    pub const fn reservation(self) -> PmExactReservation {
        self.reservation
    }

    #[must_use]
    pub const fn submit(self) -> PmOwnedSubmitState {
        self.submit
    }

    #[must_use]
    pub const fn status(self) -> Option<PmOrderStatus> {
        self.status
    }

    #[must_use]
    pub const fn cumulative_filled(self) -> U256 {
        self.cumulative_filled
    }

    #[must_use]
    pub fn remaining(self) -> U256 {
        self.quantity
            .protocol_units()
            .checked_sub(self.cumulative_filled)
            .expect("owned progress never exceeds original")
    }

    #[must_use]
    pub const fn known_fill_total(self) -> U256 {
        self.known_fill_total
    }

    #[must_use]
    pub const fn cancel(self) -> PmOwnedCancelState {
        self.cancel
    }

    #[must_use]
    pub const fn reconciliation_required(self) -> bool {
        self.reconciliation_required
    }

    #[must_use]
    pub const fn is_terminal(self) -> bool {
        matches!(self.status, Some(status) if status.is_terminal())
            || matches!(self.submit, PmOwnedSubmitState::Rejected)
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct PmOwnedQuoteSlotProjection {
    key: PmOwnedQuoteSlotKey,
    current: Option<PmClientOrderKey>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct PmOwnedTerminalCompaction {
    client_order: PmClientOrderKey,
    intent: PmOwnedIntentId,
    fill_keys_removed: usize,
}

impl PmOwnedTerminalCompaction {
    #[must_use]
    pub const fn client_order(self) -> PmClientOrderKey {
        self.client_order
    }

    #[must_use]
    pub const fn intent(self) -> PmOwnedIntentId {
        self.intent
    }

    #[must_use]
    pub const fn fill_keys_removed(self) -> usize {
        self.fill_keys_removed
    }
}

impl PmOwnedQuoteSlotProjection {
    #[must_use]
    pub const fn key(self) -> PmOwnedQuoteSlotKey {
        self.key
    }

    #[must_use]
    pub const fn current(self) -> Option<PmClientOrderKey> {
        self.current
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct PmOwnedCancelIntent {
    client_order: PmClientOrderKey,
    venue_order: PmVenueOrderKey,
}

impl PmOwnedCancelIntent {
    #[must_use]
    pub const fn client_order(self) -> PmClientOrderKey {
        self.client_order
    }

    #[must_use]
    pub const fn venue_order(self) -> PmVenueOrderKey {
        self.venue_order
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum PmOwnedReplacementBlock {
    SubmitPending,
    SubmitAmbiguous,
    CancelAmbiguous,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum PmOwnedQuoteAdmission {
    Admitted(PmClientOrderKey),
    DuplicateIntent(PmClientOrderKey),
    DuplicateQuote(PmClientOrderKey),
    CancelBeforeReplace(PmOwnedCancelIntent),
    ReplacementBlocked {
        current: PmClientOrderKey,
        reason: PmOwnedReplacementBlock,
    },
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum PmOwnedSubmitResult {
    Accepted(PmVenueOrderKey),
    Rejected,
    Ambiguous,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum PmOwnedSubmitApply {
    Accepted,
    LateAccepted,
    Rejected,
    MarkedAmbiguous,
    Duplicate,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct PmOwnedFillObservation {
    key: PmFillKey,
    quantity: PmQuantity,
    reported_cumulative: Option<U256>,
    occurrence: PmOwnedObservationOccurrence,
    source: PmOwnedObservationSource,
}

impl PmOwnedFillObservation {
    pub fn new(
        key: PmFillKey,
        quantity: PmQuantity,
        reported_cumulative: Option<U256>,
        occurrence: PmOwnedObservationOccurrence,
        source: PmOwnedObservationSource,
    ) -> Result<Self, PmOwnedOrderLifecycleError> {
        if reported_cumulative.is_some_and(|cumulative| cumulative < quantity.protocol_units()) {
            return Err(PmOwnedOrderLifecycleError::FillCumulativeBelowQuantity);
        }
        Ok(Self {
            key,
            quantity,
            reported_cumulative,
            occurrence,
            source,
        })
    }

    #[must_use]
    pub const fn key(self) -> PmFillKey {
        self.key
    }

    #[must_use]
    pub const fn quantity(self) -> PmQuantity {
        self.quantity
    }

    #[must_use]
    pub const fn reported_cumulative(self) -> Option<U256> {
        self.reported_cumulative
    }

    #[must_use]
    pub const fn occurrence(self) -> PmOwnedObservationOccurrence {
        self.occurrence
    }

    #[must_use]
    pub const fn source(self) -> PmOwnedObservationSource {
        self.source
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct PmOwnedFillProjection {
    key: PmFillKey,
    client_order: PmClientOrderKey,
    quantity: PmQuantity,
    sources: u8,
    first_occurrence: PmOwnedObservationOccurrence,
    last_occurrence: PmOwnedObservationOccurrence,
}

impl PmOwnedFillProjection {
    #[must_use]
    pub const fn key(self) -> PmFillKey {
        self.key
    }

    #[must_use]
    pub const fn client_order(self) -> PmClientOrderKey {
        self.client_order
    }

    #[must_use]
    pub const fn quantity(self) -> PmQuantity {
        self.quantity
    }

    #[must_use]
    pub const fn first_occurrence(self) -> PmOwnedObservationOccurrence {
        self.first_occurrence
    }

    #[must_use]
    pub const fn last_occurrence(self) -> PmOwnedObservationOccurrence {
        self.last_occurrence
    }

    #[must_use]
    pub const fn observed_from(self, source: PmOwnedObservationSource) -> bool {
        self.sources & source.bit() != 0
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum PmOwnedFillApply {
    Applied {
        client_order: PmClientOrderKey,
        cumulative_filled: U256,
        remaining: U256,
    },
    Duplicate {
        client_order: PmClientOrderKey,
        cumulative_filled: U256,
        remaining: U256,
        source_added: bool,
        cumulative_advanced: bool,
    },
    IgnoredOldEpoch,
}

/// Exact retained fill fact required to rebuild canonical provisional exposure
/// and the owned dedup ledger from a mutation journal.
///
/// A fill key/client aggregate is intentionally insufficient: replay also
/// requires the execution principal, fee, settlement, cumulative evidence,
/// original observation source, and occurrence context carried here. Recovery
/// is the authority/delivery path and never replaces that original source.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct PmOwnedRecoveryFill {
    event: reap_pm_core::PmFillEvent,
    reported_cumulative: Option<U256>,
    occurrence: PmOwnedObservationOccurrence,
    source: PmOwnedObservationSource,
}

impl PmOwnedRecoveryFill {
    #[must_use]
    pub const fn new(
        event: reap_pm_core::PmFillEvent,
        reported_cumulative: Option<U256>,
        occurrence: PmOwnedObservationOccurrence,
        source: PmOwnedObservationSource,
    ) -> Self {
        Self {
            event,
            reported_cumulative,
            occurrence,
            source,
        }
    }

    #[must_use]
    pub const fn event(self) -> reap_pm_core::PmFillEvent {
        self.event
    }

    #[must_use]
    pub const fn reported_cumulative(self) -> Option<U256> {
        self.reported_cumulative
    }

    #[must_use]
    pub const fn occurrence(self) -> PmOwnedObservationOccurrence {
        self.occurrence
    }

    #[must_use]
    pub const fn source(self) -> PmOwnedObservationSource {
        self.source
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct PmOwnedOrderProgressObservation {
    client_order: PmClientOrderKey,
    venue_order: PmVenueOrderKey,
    progress: PmOrderProgress,
    occurrence: PmOwnedObservationOccurrence,
    source: PmOwnedObservationSource,
}

impl PmOwnedOrderProgressObservation {
    #[must_use]
    pub const fn new(
        client_order: PmClientOrderKey,
        venue_order: PmVenueOrderKey,
        progress: PmOrderProgress,
        occurrence: PmOwnedObservationOccurrence,
        source: PmOwnedObservationSource,
    ) -> Self {
        Self {
            client_order,
            venue_order,
            progress,
            occurrence,
            source,
        }
    }

    #[must_use]
    pub const fn occurrence(self) -> PmOwnedObservationOccurrence {
        self.occurrence
    }

    #[must_use]
    pub const fn client_order(self) -> PmClientOrderKey {
        self.client_order
    }

    #[must_use]
    pub const fn venue_order(self) -> PmVenueOrderKey {
        self.venue_order
    }

    #[must_use]
    pub const fn progress(self) -> PmOrderProgress {
        self.progress
    }

    #[must_use]
    pub const fn source(self) -> PmOwnedObservationSource {
        self.source
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum PmOwnedProgressApply {
    Applied {
        status: PmOrderStatus,
        cumulative_filled: U256,
        remaining: U256,
    },
    Duplicate,
    IgnoredOutOfOrder,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum PmOwnedRemoteOrderApply {
    Matched(PmClientOrderKey),
    AmbiguousRemote,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum PmOwnedDetailAbsenceApply {
    SettledAcceptedCancel(PmClientOrderKey),
    Unmatched,
    Unsafe,
    IgnoredOutOfOrder,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum PmOwnedCancelRequestApply {
    Issued(PmOwnedCancelIntent),
    Duplicate(PmOwnedCancelIntent),
    AlreadyTerminal,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum PmOwnedCancelOutcome {
    Accepted,
    Rejected,
    AlreadyFilled,
    Ambiguous,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum PmOwnedCancelApply {
    Cancelled,
    Rejected,
    Filled,
    MarkedAmbiguous,
    ConvergedFilled,
    Duplicate,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub struct PmOwnedLifecycleCounters {
    admissions: u64,
    duplicate_quotes: u64,
    cancel_before_replace: u64,
    submit_accepts: u64,
    submit_rejections: u64,
    submit_ambiguous: u64,
    fills: u64,
    fill_duplicates: u64,
    cancel_requests: u64,
    cancel_results: u64,
    reconnects: u64,
    terminal_compactions: u64,
    contract_violations: u64,
    order_capacity_failures: u64,
    fill_capacity_failures: u64,
}

impl PmOwnedLifecycleCounters {
    #[must_use]
    pub const fn admissions(self) -> u64 {
        self.admissions
    }

    #[must_use]
    pub const fn duplicate_quotes(self) -> u64 {
        self.duplicate_quotes
    }

    #[must_use]
    pub const fn cancel_before_replace(self) -> u64 {
        self.cancel_before_replace
    }

    #[must_use]
    pub const fn submit_accepts(self) -> u64 {
        self.submit_accepts
    }

    #[must_use]
    pub const fn submit_rejections(self) -> u64 {
        self.submit_rejections
    }

    #[must_use]
    pub const fn submit_ambiguous(self) -> u64 {
        self.submit_ambiguous
    }

    #[must_use]
    pub const fn fills(self) -> u64 {
        self.fills
    }

    #[must_use]
    pub const fn fill_duplicates(self) -> u64 {
        self.fill_duplicates
    }

    #[must_use]
    pub const fn cancel_requests(self) -> u64 {
        self.cancel_requests
    }

    #[must_use]
    pub const fn cancel_results(self) -> u64 {
        self.cancel_results
    }

    #[must_use]
    pub const fn reconnects(self) -> u64 {
        self.reconnects
    }

    #[must_use]
    pub const fn terminal_compactions(self) -> u64 {
        self.terminal_compactions
    }

    #[must_use]
    pub const fn contract_violations(self) -> u64 {
        self.contract_violations
    }

    #[must_use]
    pub const fn order_capacity_failures(self) -> u64 {
        self.order_capacity_failures
    }

    #[must_use]
    pub const fn fill_capacity_failures(self) -> u64 {
        self.fill_capacity_failures
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Error)]
pub enum PmOwnedOrderLifecycleError {
    #[error("owned local intent identity must be nonzero")]
    ZeroIntentIdentity,
    #[error("owned reducer sequence must be nonzero")]
    ZeroReductionSequence,
    #[error("snapshot revision requires original private occurrence evidence")]
    SnapshotWithoutPrivateOccurrence,
    #[error("owned quote/order belongs to another exact account, instrument, or side slot")]
    ScopeMismatch,
    #[error("owned quote requires an exact policy-approved reservation")]
    InvalidReservation,
    #[error("client-order identity is already bound to another local intent or quote")]
    ClientIdentityConflict,
    #[error("local intent identity is already bound to another client order")]
    IntentIdentityConflict,
    #[error("local intent identity is at or below the compacted monotonic high-watermark")]
    CompactedIntentIdentity,
    #[error("venue-order identity is already bound to another client order")]
    VenueBindingConflict,
    #[error("owned client-order identity is unknown")]
    UnknownClientOrder,
    #[error("venue order has no exact proven owned client binding")]
    UnboundVenueOrder,
    #[error("submit lifecycle transition is invalid")]
    InvalidSubmitTransition,
    #[error("terminal owned order cannot be resurrected")]
    TerminalNonResurrection,
    #[error("fill identity carries conflicting exact quantity or ownership")]
    FillConflict,
    #[error("fill-reported cumulative quantity is below its exact fill quantity")]
    FillCumulativeBelowQuantity,
    #[error("owned cumulative fill exceeds original quantity")]
    Overfill,
    #[error("owned cumulative fill moved backwards")]
    BackwardsCumulative,
    #[error("order progress original quantity differs from the owned intent")]
    ProgressOriginalMismatch,
    #[error("the same private occurrence carries conflicting owned progress")]
    SameOccurrenceConflict,
    #[error("cancel requires a live accepted order with an exact venue binding")]
    CancelUnavailable,
    #[error("cancel result arrived without a pending or ambiguous cancel")]
    CancelResultWithoutIntent,
    #[error("owned order is not proven terminal and safe to compact")]
    TerminalCompactionUnavailable,
    #[error("owned lifecycle has no active private epoch")]
    MissingEpoch,
    #[error("owned lifecycle private epoch did not advance")]
    EpochDidNotAdvance,
    #[error("owned observation belongs to an older private epoch")]
    OldEpoch,
    #[error("owned-order history reached its fixed capacity")]
    OrderCapacity,
    #[error("owned fill-key ledger reached its fixed capacity")]
    FillCapacity,
    #[error("owned exact cumulative arithmetic overflowed")]
    ArithmeticOverflow,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
struct OwnedOrderEntry {
    intent: PmOwnedQuoteIntent,
    venue_order: Option<PmVenueOrderKey>,
    submit: PmOwnedSubmitState,
    status: Option<PmOrderStatus>,
    cumulative_filled: U256,
    known_fill_total: U256,
    cancel: PmOwnedCancelState,
    reconciliation_required: bool,
    compaction_generation: Option<u64>,
    last_occurrence: Option<PmOwnedObservationOccurrence>,
    last_progress: Option<(PmOwnedObservationOccurrence, PmOrderProgress)>,
}

impl OwnedOrderEntry {
    const fn projection(self) -> PmOwnedOrderProjection {
        PmOwnedOrderProjection {
            intent: self.intent.intent(),
            slot: self.intent.slot(),
            client_order: self.intent.client_order(),
            venue_order: self.venue_order,
            price: self.intent.price(),
            quantity: self.intent.quantity(),
            reservation: self.intent.reservation(),
            submit: self.submit,
            status: self.status,
            cumulative_filled: self.cumulative_filled,
            known_fill_total: self.known_fill_total,
            cancel: self.cancel,
            reconciliation_required: self.reconciliation_required,
        }
    }

    const fn is_terminal(self) -> bool {
        matches!(self.submit, PmOwnedSubmitState::Rejected)
            || matches!(self.status, Some(status) if status.is_terminal())
    }

    const fn is_live(self) -> bool {
        !self.is_terminal()
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
struct OwnedFillEntry {
    key: PmFillKey,
    client_order: PmClientOrderKey,
    quantity: PmQuantity,
    sources: u8,
    first_occurrence: PmOwnedObservationOccurrence,
    last_occurrence: PmOwnedObservationOccurrence,
}

impl OwnedFillEntry {
    const fn projection(self) -> PmOwnedFillProjection {
        PmOwnedFillProjection {
            key: self.key,
            client_order: self.client_order,
            quantity: self.quantity,
            sources: self.sources,
            first_occurrence: self.first_occurrence,
            last_occurrence: self.last_occurrence,
        }
    }
}

pub struct PmOwnedOrderLifecycle {
    account_scope: PmAccountScope,
    instrument: PmInstrumentHandle,
    slots: [Option<PmClientOrderKey>; 2],
    entries: Vec<OwnedOrderEntry>,
    client_order_index: Vec<u16>,
    intent_index: Vec<u16>,
    fills: Vec<OwnedFillEntry>,
    compacted_intent_high_watermark: Option<PmOwnedIntentId>,
    current_epoch: Option<ConnectionEpoch>,
    counters: PmOwnedLifecycleCounters,
}

pub(crate) struct PmOwnedQuoteAdmissionPlan {
    outcome: PmOwnedQuoteAdmission,
    action: PmOwnedQuoteAdmissionAction,
}

impl PmOwnedQuoteAdmissionPlan {
    const fn new(outcome: PmOwnedQuoteAdmission, action: PmOwnedQuoteAdmissionAction) -> Self {
        Self { outcome, action }
    }

    pub(crate) const fn outcome(&self) -> PmOwnedQuoteAdmission {
        self.outcome
    }
}

#[allow(
    clippy::large_enum_variant,
    reason = "owned-order admission carries the prebuilt entry inline to keep the owner loop allocation-free"
)]
enum PmOwnedQuoteAdmissionAction {
    None,
    CountDuplicateQuote,
    MarkCancelBeforeReplace {
        index: usize,
    },
    Insert {
        client_position: usize,
        intent_position: usize,
        slot_index: usize,
        entry: OwnedOrderEntry,
    },
}

mod dense_index;
mod reducer;
