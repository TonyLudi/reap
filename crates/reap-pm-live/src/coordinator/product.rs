//! Static single-owner PM product coordinator seam.
//!
//! The coordinator owns only decision state in addition to the existing
//! `PmMutationOwner`: one exact OKX reference projection, one copied
//! canonical-book projection, the pure model, fixed copied correlations, and
//! counters. It is not a second public/private/book/schedule/mutation owner.

#![allow(
    clippy::result_large_err,
    reason = "exact inline PM failures remain allocation-free on the owner path"
)]

use std::cmp::Ordering;

use reap_pm_core::{
    ConnectionEpoch, IngressSequence, OkxReferenceHandle, OkxReferencePrice, PmAccountScope,
    PmClientOrderKey, PmConnectionId, PmInstrumentHandle, PmMarketMetadata, PmOrderSalt,
    PmOrderSide, PmPrice, PmProductSource, SnapshotRevision, U256,
};
use reap_pm_live_contracts::PmConnectivityConfig;
use reap_pm_state::{PmExactReservation, PmOwnedSubmitState};
use reap_pm_strategy::{
    PmQuoteModel, PmQuoteModelError, PmQuoteModelInput, PmQuotePolicyError,
    PmValidatedQuoteCandidate, validate_passive_quote_candidate,
};
use thiserror::Error;

use super::effect_queue::PmFakeEffectMetrics;
use super::effects::{
    MAX_PM_EFFECTS_PER_INPUT, PmCancelIntentReason, PmDurableRecordEffect, PmDurableRecordKind,
    PmEffectCapacityError, PmFailClosedEffect, PmFakeCancelEffect, PmFakeEffectStage,
    PmFakeQuoteEffect, PmHealthMetricEffect, PmHealthMetricKind, PmProductEffect,
    PmProductEffectBatch, PmProductEffectMetrics, PmProductEffectOutput, PmRefreshEffect,
    PmRefreshEffectKind,
};
use super::input::{
    PmBookInput, PmControlReason, PmMarketInput, PmOkxReferenceInput, PmTimerInput,
};
use super::mutation::{
    PmCancelMutationAdmission, PmCancelMutationRequest, PmMutationCounters, PmMutationError,
    PmMutationHalt, PmMutationOwner, PmPersistenceService, PmQuoteMutationAdmission,
    PmQuoteMutationRequest,
};
use super::persistence::{PmPersistenceError, PmPersistenceIntentIdentity, PmPersistenceMetrics};
use super::{PmAuthorityError, PmAuthorityRevisions};
use crate::journal::PmJournalCancelReasonV1;
use crate::lanes::{
    PmCompleteIngress, PmCompleteInputLanes, PmCompleteServiced, PmCriticalInput,
    PmPersistenceInput as PmLanePersistenceInput, PmPrivateInput, PmReconciliationInput,
};
use crate::schedule::{PmScheduleMetrics, PmScheduledActionKey, PmScheduledActionKind};

pub(crate) const MAX_COPIED_EFFECT_CORRELATIONS: usize = 256;

mod cancel;
mod evidence;
mod helpers;
mod lane_failure;
#[cfg(test)]
mod overload_evidence;
mod refresh_obligations;
mod service;
mod start;

pub(crate) use evidence::PmEvidenceTerminalLengths;
use helpers::*;
#[cfg(test)]
pub(crate) use overload_evidence::PmTelemetryOverloadState;
#[cfg(test)]
pub(crate) use refresh_obligations::Phase6RefreshAllocationProbe;
pub use refresh_obligations::PmRefreshObligationMetrics;
pub(crate) use start::{PmCoordinatorShutdownError, PmCoordinatorStartError};

/// Explicit freshness and authority lifetime policy. There are no permissive
/// defaults.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct PmCoordinatorPolicy {
    maximum_reference_age_ns: u64,
    maximum_book_age_ns: u64,
    approval_lifetime_ns: u64,
}

impl PmCoordinatorPolicy {
    pub fn new(
        maximum_reference_age_ns: u64,
        maximum_book_age_ns: u64,
        approval_lifetime_ns: u64,
    ) -> Result<Self, PmCoordinatorPolicyError> {
        if maximum_reference_age_ns == 0 || maximum_book_age_ns == 0 || approval_lifetime_ns == 0 {
            return Err(PmCoordinatorPolicyError::ZeroLimit);
        }
        Ok(Self {
            maximum_reference_age_ns,
            maximum_book_age_ns,
            approval_lifetime_ns,
        })
    }

    #[must_use]
    pub const fn maximum_reference_age_ns(self) -> u64 {
        self.maximum_reference_age_ns
    }

    #[must_use]
    pub const fn maximum_book_age_ns(self) -> u64 {
        self.maximum_book_age_ns
    }

    #[must_use]
    pub const fn approval_lifetime_ns(self) -> u64 {
        self.approval_lifetime_ns
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Error)]
pub enum PmCoordinatorPolicyError {
    #[error("coordinator freshness and lifetime limits must be positive")]
    ZeroLimit,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum PmQuoteSuppression {
    MissingReference,
    MissingMarket,
    MissingBook,
    PrivateUnavailable,
    RiskRejected,
    MarketUnavailable,
    ReferenceStale,
    BookStale,
    BookUnavailable,
    RevisionMismatch,
    CoordinatorHalted,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub struct PmCoordinatorCounters {
    inputs: u64,
    references_applied: u64,
    references_ignored: u64,
    markets_applied: u64,
    books_applied: u64,
    quote_evaluations: u64,
    quote_candidates: u64,
    quote_suppressions: u64,
    quote_policy_rejections: u64,
    durable_record_effects: u64,
    fake_quote_effects: u64,
    fake_cancel_effects: u64,
    refresh_effects: u64,
    control_halts: u64,
    maximum_effects_per_input: u8,
    correlation_high_water: u16,
}

impl PmCoordinatorCounters {
    #[must_use]
    pub const fn inputs(self) -> u64 {
        self.inputs
    }

    #[must_use]
    pub const fn references_applied(self) -> u64 {
        self.references_applied
    }

    #[must_use]
    pub const fn references_ignored(self) -> u64 {
        self.references_ignored
    }

    #[must_use]
    pub const fn markets_applied(self) -> u64 {
        self.markets_applied
    }

    #[must_use]
    pub const fn books_applied(self) -> u64 {
        self.books_applied
    }

    #[must_use]
    pub const fn quote_evaluations(self) -> u64 {
        self.quote_evaluations
    }

    #[must_use]
    pub const fn quote_candidates(self) -> u64 {
        self.quote_candidates
    }

    #[must_use]
    pub const fn quote_suppressions(self) -> u64 {
        self.quote_suppressions
    }

    #[must_use]
    pub const fn quote_policy_rejections(self) -> u64 {
        self.quote_policy_rejections
    }

    #[must_use]
    pub const fn durable_record_effects(self) -> u64 {
        self.durable_record_effects
    }

    #[must_use]
    pub const fn fake_quote_effects(self) -> u64 {
        self.fake_quote_effects
    }

    #[must_use]
    pub const fn fake_cancel_effects(self) -> u64 {
        self.fake_cancel_effects
    }

    #[must_use]
    pub const fn refresh_effects(self) -> u64 {
        self.refresh_effects
    }

    #[must_use]
    pub const fn control_halts(self) -> u64 {
        self.control_halts
    }

    #[must_use]
    pub const fn maximum_effects_per_input(self) -> u8 {
        self.maximum_effects_per_input
    }

    #[must_use]
    pub const fn correlation_high_water(self) -> u16 {
        self.correlation_high_water
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
struct ExactOccurrence {
    epoch: ConnectionEpoch,
    ingress: IngressSequence,
}

impl ExactOccurrence {
    const fn new(epoch: ConnectionEpoch, ingress: IngressSequence) -> Self {
        Self { epoch, ingress }
    }

    fn cmp(self, other: Self) -> Ordering {
        (self.epoch, self.ingress).cmp(&(other.epoch, other.ingress))
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
struct ReferenceState {
    occurrence: ExactOccurrence,
    price: OkxReferencePrice,
    observed_monotonic_ns: u64,
    revision: u64,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
struct MarketState {
    occurrence: ExactOccurrence,
    metadata: PmMarketMetadata,
    metadata_revision: SnapshotRevision,
    observed_monotonic_ns: u64,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
struct BookState {
    occurrence: ExactOccurrence,
    metadata_revision: SnapshotRevision,
    snapshot_revision: Option<SnapshotRevision>,
    readiness_revision: u64,
    best_bid: Option<PmPrice>,
    best_ask: Option<PmPrice>,
    observed_monotonic_ns: u64,
    ready: bool,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
struct DecisionCandidateBatch {
    candidates: [Option<PmValidatedQuoteCandidate>; 2],
    policy_errors: [Option<PmQuotePolicyError>; 2],
    model_revision: u64,
    metadata_revision: SnapshotRevision,
    book_revision: SnapshotRevision,
    book_readiness_revision: u64,
    reference_observed_ns: u64,
    book_observed_ns: u64,
}

#[allow(
    clippy::large_enum_variant,
    reason = "the allocation-free decision path keeps its bounded candidate batch inline"
)]
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum DecisionOutcome {
    Candidates(DecisionCandidateBatch),
    Suppressed(PmQuoteSuppression),
}

#[derive(Debug)]
struct PmDecisionState<M> {
    model: M,
    reference_handle: OkxReferenceHandle,
    instrument: PmInstrumentHandle,
    expected_metadata: PmMarketMetadata,
    policy: PmCoordinatorPolicy,
    reference: Option<ReferenceState>,
    market: Option<MarketState>,
    book: Option<BookState>,
    next_reference_revision: u64,
    next_model_revision: u64,
}

impl<M: PmQuoteModel> PmDecisionState<M> {
    fn new(
        config: &PmConnectivityConfig,
        model: M,
        policy: PmCoordinatorPolicy,
    ) -> Result<Self, PmCoordinatorError> {
        let requirements = model.input_requirements();
        if requirements.reference() != config.public().okx_reference()
            || requirements.instrument() != config.public().instrument()
        {
            return Err(PmCoordinatorError::ModelRequirementsMismatch);
        }
        Ok(Self {
            model,
            reference_handle: config.public().okx_reference(),
            instrument: config.public().instrument(),
            expected_metadata: config.public().expected_metadata(),
            policy,
            reference: None,
            market: None,
            book: None,
            next_reference_revision: 1,
            next_model_revision: 1,
        })
    }

    fn observe_reference(
        &mut self,
        input: PmOkxReferenceInput,
    ) -> Result<bool, PmCoordinatorError> {
        let event = input.event();
        if event.reference() != self.reference_handle {
            return Err(PmCoordinatorError::ReferenceScopeMismatch);
        }
        let ordering = input.ordering();
        let occurrence = ExactOccurrence::new(
            ordering.connection_epoch(),
            ordering.local_ingress_sequence(),
        );
        if let Some(current) = self.reference {
            match occurrence.cmp(current.occurrence) {
                Ordering::Less => return Ok(false),
                Ordering::Equal if current.price == event.price() => return Ok(false),
                Ordering::Equal => return Err(PmCoordinatorError::ConflictingOccurrence),
                Ordering::Greater => {}
            }
        }
        let revision = self.next_reference_revision;
        self.next_reference_revision = revision
            .checked_add(1)
            .ok_or(PmCoordinatorError::RevisionExhausted)?;
        self.reference = Some(ReferenceState {
            occurrence,
            price: event.price(),
            observed_monotonic_ns: input.clock().monotonic_receive_ns(),
            revision,
        });
        Ok(true)
    }

    fn observe_market(&mut self, input: PmMarketInput) -> Result<bool, PmCoordinatorError> {
        let event = input.event();
        if event.instrument() != self.instrument
            || event.metadata().market() != self.expected_metadata.market()
            || event.metadata().outcome().token() != self.expected_metadata.outcome().token()
        {
            return Err(PmCoordinatorError::MarketScopeMismatch);
        }
        let ordering = input.ordering();
        let occurrence = ExactOccurrence::new(
            ordering.connection_epoch(),
            ordering.local_ingress_sequence(),
        );
        if let Some(current) = self.market {
            match occurrence.cmp(current.occurrence) {
                Ordering::Less => return Ok(false),
                Ordering::Equal
                    if current.metadata == event.metadata()
                        && current.metadata_revision == event.metadata_revision() =>
                {
                    return Ok(false);
                }
                Ordering::Equal => return Err(PmCoordinatorError::ConflictingOccurrence),
                Ordering::Greater => {}
            }
        }
        self.market = Some(MarketState {
            occurrence,
            metadata: event.metadata(),
            metadata_revision: event.metadata_revision(),
            observed_monotonic_ns: input.clock().monotonic_receive_ns(),
        });
        Ok(true)
    }

    fn observe_book(&mut self, input: PmBookInput) -> Result<bool, PmCoordinatorError> {
        if input.event().instrument() != self.instrument
            || input.projection().instrument() != self.instrument
        {
            return Err(PmCoordinatorError::BookScopeMismatch);
        }
        let ordering = input.ordering();
        let occurrence = ExactOccurrence::new(
            ordering.connection_epoch(),
            ordering.local_ingress_sequence(),
        );
        let projection = input.projection();
        let top = projection.top();
        let next = BookState {
            occurrence,
            metadata_revision: projection.metadata_revision(),
            snapshot_revision: projection.snapshot_revision(),
            readiness_revision: projection.readiness_revision(),
            best_bid: top
                .and_then(reap_pm_core::PmBookTop::bid)
                .map(|point| point.price()),
            best_ask: top
                .and_then(reap_pm_core::PmBookTop::ask)
                .map(|point| point.price()),
            observed_monotonic_ns: projection.observed_monotonic_ns(),
            ready: projection.is_ready(),
        };
        if let Some(current) = self.book {
            match occurrence.cmp(current.occurrence) {
                Ordering::Less => return Ok(false),
                Ordering::Equal if current == next => return Ok(false),
                Ordering::Equal => return Err(PmCoordinatorError::ConflictingOccurrence),
                Ordering::Greater => {}
            }
        }
        self.book = Some(next);
        Ok(true)
    }

    fn invalidate_okx_public(&mut self) {
        self.reference = None;
    }

    fn invalidate_pm_public(&mut self) {
        self.market = None;
        self.book = None;
    }

    fn evaluate(&mut self, monotonic_now_ns: u64) -> Result<DecisionOutcome, PmCoordinatorError> {
        let Some(reference) = self.reference else {
            return Ok(DecisionOutcome::Suppressed(
                PmQuoteSuppression::MissingReference,
            ));
        };
        let Some(market) = self.market else {
            return Ok(DecisionOutcome::Suppressed(
                PmQuoteSuppression::MissingMarket,
            ));
        };
        let Some(book) = self.book else {
            return Ok(DecisionOutcome::Suppressed(PmQuoteSuppression::MissingBook));
        };
        if !market_is_tradable(market.metadata) {
            return Ok(DecisionOutcome::Suppressed(
                PmQuoteSuppression::MarketUnavailable,
            ));
        }
        let reference_age = age(monotonic_now_ns, reference.observed_monotonic_ns)?;
        if reference_age > self.policy.maximum_reference_age_ns {
            return Ok(DecisionOutcome::Suppressed(
                PmQuoteSuppression::ReferenceStale,
            ));
        }
        let book_age = age(monotonic_now_ns, book.observed_monotonic_ns)?;
        if book_age > self.policy.maximum_book_age_ns {
            return Ok(DecisionOutcome::Suppressed(PmQuoteSuppression::BookStale));
        }
        if !book.ready {
            return Ok(DecisionOutcome::Suppressed(
                PmQuoteSuppression::BookUnavailable,
            ));
        }
        let Some(book_revision) = book.snapshot_revision else {
            return Ok(DecisionOutcome::Suppressed(
                PmQuoteSuppression::BookUnavailable,
            ));
        };
        if market.metadata_revision != book.metadata_revision {
            return Ok(DecisionOutcome::Suppressed(
                PmQuoteSuppression::RevisionMismatch,
            ));
        }

        let model_revision = self.next_model_revision;
        let model_input = PmQuoteModelInput::new(
            reference.price,
            reference.revision,
            self.instrument,
            monotonic_now_ns,
        )?;
        let output = self.model.evaluate(model_input);
        self.next_model_revision = model_revision
            .checked_add(1)
            .ok_or(PmCoordinatorError::RevisionExhausted)?;

        let mut candidates = [None; 2];
        let mut policy_errors = [None; 2];
        for (index, side) in output.sides().ordered().into_iter().enumerate() {
            let Some(side) = side else {
                continue;
            };
            match validate_passive_quote_candidate(reap_pm_strategy::PmQuotePolicyInput::new(
                self.instrument,
                market.metadata,
                side,
                output.fair_probability(),
                output.quantity(),
                book.best_bid,
                book.best_ask,
            )) {
                Ok(candidate) => candidates[index] = Some(candidate),
                Err(error) => policy_errors[index] = Some(error),
            }
        }
        Ok(DecisionOutcome::Candidates(DecisionCandidateBatch {
            candidates,
            policy_errors,
            model_revision,
            metadata_revision: market.metadata_revision,
            book_revision,
            book_readiness_revision: book.readiness_revision,
            reference_observed_ns: reference.observed_monotonic_ns,
            book_observed_ns: book.observed_monotonic_ns,
        }))
    }
}

fn age(now_ns: u64, observed_ns: u64) -> Result<u64, PmCoordinatorError> {
    now_ns
        .checked_sub(observed_ns)
        .ok_or(PmCoordinatorError::ClockRegression)
}

const fn market_is_tradable(metadata: PmMarketMetadata) -> bool {
    let lifecycle = metadata.lifecycle();
    lifecycle.active()
        && !lifecycle.closed()
        && !lifecycle.archived()
        && lifecycle.accepting_orders()
        && lifecycle.order_book_enabled()
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum CopiedEffectCorrelation {
    Quote {
        intent: u64,
        effect: PmFakeQuoteEffect,
    },
    Cancel(PmFakeCancelEffect),
}

#[derive(Debug)]
struct CorrelationRing {
    values: Box<[Option<CopiedEffectCorrelation>]>,
    head: u16,
    len: u16,
    high_water: u16,
}

impl CorrelationRing {
    fn new() -> Self {
        Self {
            values: vec![None; MAX_COPIED_EFFECT_CORRELATIONS].into_boxed_slice(),
            head: 0,
            len: 0,
            high_water: 0,
        }
    }

    fn push(&mut self, value: CopiedEffectCorrelation) -> Result<(), PmCoordinatorError> {
        if usize::from(self.len) == MAX_COPIED_EFFECT_CORRELATIONS {
            return Err(PmCoordinatorError::CopiedCorrelationSaturated);
        }
        let index =
            (usize::from(self.head) + usize::from(self.len)) % MAX_COPIED_EFFECT_CORRELATIONS;
        self.values[index] = Some(value);
        self.len += 1;
        self.high_water = self.high_water.max(self.len);
        Ok(())
    }

    const fn high_water(&self) -> u16 {
        self.high_water
    }

    fn reserved_capacity_bytes(&self) -> usize {
        std::mem::size_of_val(&*self.values)
    }

    fn remove_identity(
        &mut self,
        identity: PmPersistenceIntentIdentity,
    ) -> Result<CopiedEffectCorrelation, PmCoordinatorError> {
        self.remove_matching(|value| correlation_matches(value, identity))
    }

    fn remove_quote(
        &mut self,
        client_order: PmClientOrderKey,
    ) -> Result<PmFakeQuoteEffect, PmCoordinatorError> {
        match self.remove_matching(|value| {
            matches!(
                value,
                CopiedEffectCorrelation::Quote { effect, .. }
                    if effect.client_order() == client_order
            )
        })? {
            CopiedEffectCorrelation::Quote { effect, .. } => Ok(effect),
            CopiedEffectCorrelation::Cancel(_) => Err(PmCoordinatorError::CorrelationKindMismatch),
        }
    }

    fn remove_cancel(
        &mut self,
        client_order: PmClientOrderKey,
        venue_order: reap_pm_core::PmVenueOrderKey,
    ) -> Result<PmFakeCancelEffect, PmCoordinatorError> {
        match self.remove_matching(|value| {
            matches!(
                value,
                CopiedEffectCorrelation::Cancel(effect)
                    if effect.client_order() == client_order
                        && effect.venue_order() == venue_order
            )
        })? {
            CopiedEffectCorrelation::Cancel(effect) => Ok(effect),
            CopiedEffectCorrelation::Quote { .. } => {
                Err(PmCoordinatorError::CorrelationKindMismatch)
            }
        }
    }

    fn remove_matching(
        &mut self,
        mut predicate: impl FnMut(CopiedEffectCorrelation) -> bool,
    ) -> Result<CopiedEffectCorrelation, PmCoordinatorError> {
        let Some(offset) = (0..usize::from(self.len)).find(|offset| {
            let index = (usize::from(self.head) + offset) % MAX_COPIED_EFFECT_CORRELATIONS;
            self.values[index].is_some_and(&mut predicate)
        }) else {
            return Err(PmCoordinatorError::MissingCopiedCorrelation);
        };
        let index = (usize::from(self.head) + offset) % MAX_COPIED_EFFECT_CORRELATIONS;
        let value = self.values[index]
            .take()
            .ok_or(PmCoordinatorError::MissingCopiedCorrelation)?;
        for shift in offset..usize::from(self.len) - 1 {
            let from = (usize::from(self.head) + shift + 1) % MAX_COPIED_EFFECT_CORRELATIONS;
            let to = (usize::from(self.head) + shift) % MAX_COPIED_EFFECT_CORRELATIONS;
            self.values[to] = self.values[from].take();
        }
        self.len -= 1;
        Ok(value)
    }
}

fn correlation_matches(
    correlation: CopiedEffectCorrelation,
    identity: PmPersistenceIntentIdentity,
) -> bool {
    match (correlation, identity) {
        (
            CopiedEffectCorrelation::Quote { intent, effect },
            PmPersistenceIntentIdentity::Quote {
                intent: expected_intent,
                client_order,
            },
        ) => intent == expected_intent.value() && effect.client_order() == client_order,
        (
            CopiedEffectCorrelation::Cancel(effect),
            PmPersistenceIntentIdentity::Cancel {
                client_order,
                venue_order,
            },
        ) => effect.client_order() == client_order && effect.venue_order() == venue_order,
        _ => false,
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum TrackedQuoteStage {
    PersistencePending,
    PreparedLocal,
    DispatchedWithoutVenue,
    RemotelyLive,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
struct TrackedQuote {
    client_order: PmClientOrderKey,
    side: PmOrderSide,
    stage: TrackedQuoteStage,
}

struct RetainedLaneInput<T> {
    ingress: PmCompleteIngress,
    input: T,
}

#[derive(Debug, Clone, Copy)]
struct PendingSchedule {
    key: PmScheduledActionKey,
    deadline_ns: u64,
    scheduled_at_ns: u64,
    decision_wall_timestamp_ms: u64,
}

struct PendingSchedules {
    values: [Option<PendingSchedule>; 8],
}

impl PendingSchedules {
    const fn new() -> Self {
        Self { values: [None; 8] }
    }

    fn insert(
        &mut self,
        key: PmScheduledActionKey,
        deadline_ns: u64,
        scheduled_at_ns: u64,
        decision_wall_timestamp_ms: u64,
    ) -> Result<(), PmCoordinatorError> {
        if let Some(existing) = self
            .values
            .iter_mut()
            .flatten()
            .find(|pending| pending.key == key)
        {
            existing.deadline_ns = deadline_ns;
            existing.scheduled_at_ns = scheduled_at_ns;
            existing.decision_wall_timestamp_ms = decision_wall_timestamp_ms;
            return Ok(());
        }
        let Some(slot) = self.values.iter_mut().find(|slot| slot.is_none()) else {
            return Err(PmCoordinatorError::PendingScheduleSaturated);
        };
        *slot = Some(PendingSchedule {
            key,
            deadline_ns,
            scheduled_at_ns,
            decision_wall_timestamp_ms,
        });
        Ok(())
    }

    fn take_next(&mut self) -> Option<PendingSchedule> {
        let index = self
            .values
            .iter()
            .enumerate()
            .filter_map(|(index, value)| value.map(|value| (index, value)))
            .min_by_key(|(_, value)| (value.deadline_ns, value.key))
            .map(|(index, _)| index)?;
        self.values[index].take()
    }

    fn clear(&mut self) {
        self.values.fill(None);
    }
}

/// Static generic product coordinator.
pub(crate) struct PmCoordinator<M> {
    decision: PmDecisionState<M>,
    account_source: PmProductSource,
    account_connection: PmConnectionId,
    mutation: Box<PmMutationOwner>,
    lanes: Option<Box<PmCompleteInputLanes>>,
    outputs: PmProductEffectOutput,
    account_scope: PmAccountScope,
    instrument: PmInstrumentHandle,
    private_readiness_revision: u64,
    last_action_sequence: u64,
    pending_correlations: CorrelationRing,
    prepared_correlations: CorrelationRing,
    tracked_quotes: [Option<TrackedQuote>; 2],
    halt: Option<PmControlReason>,
    counters: PmCoordinatorCounters,
    callback_error: Option<PmCoordinatorError>,
    retained_critical: Option<RetainedLaneInput<PmCriticalInput>>,
    retained_persistence: Option<RetainedLaneInput<PmLanePersistenceInput>>,
    retained_private_admission: Option<RetainedLaneInput<PmPrivateInput>>,
    retained_reconciliation_admission: Option<RetainedLaneInput<PmReconciliationInput>>,
    pending_schedules: PendingSchedules,
    refresh_obligations: refresh_obligations::PmRefreshObligations,
    reconciliation_gate: bool,
    reconciliation_recovered: bool,
}

impl<M: PmQuoteModel> PmCoordinator<M> {
    #[must_use]
    pub(crate) const fn counters(&self) -> PmCoordinatorCounters {
        self.counters
    }

    #[must_use]
    pub(crate) const fn halt(&self) -> Option<PmControlReason> {
        self.halt
    }

    pub(crate) const fn mutation_halt(&self) -> Option<PmMutationHalt> {
        self.mutation.halt()
    }

    pub(crate) const fn mutation_counters(&self) -> PmMutationCounters {
        self.mutation.counters()
    }

    pub(crate) fn persistence_metrics(&self) -> PmPersistenceMetrics {
        self.mutation.persistence_metrics()
    }

    pub(crate) fn fake_effect_metrics(&self) -> PmFakeEffectMetrics {
        self.mutation.fake_effect_metrics()
    }

    pub(crate) const fn product_effect_metrics(&self) -> PmProductEffectMetrics {
        self.outputs.metrics()
    }

    pub(crate) fn reserved_capacity_bytes(&self) -> usize {
        self.transitive_reserved_capacity_bytes()
            .saturating_add(self.inline_bounded_capacity_bytes())
    }

    pub(crate) fn boxed_reserved_capacity_bytes(&self) -> usize {
        self.transitive_reserved_capacity_bytes()
            .saturating_add(std::mem::size_of_val(self))
    }

    #[cfg(test)]
    pub(crate) fn tracked_quote_slots_for_test(&self) -> usize {
        self.tracked_quotes.iter().flatten().count()
    }

    fn transitive_reserved_capacity_bytes(&self) -> usize {
        self.mutation
            .reserved_capacity_bytes()
            .saturating_add(std::mem::size_of::<PmMutationOwner>())
            .saturating_add(self.lanes.as_ref().map_or(0, |lanes| {
                lanes
                    .reserved_capacity_bytes()
                    .saturating_add(std::mem::size_of::<PmCompleteInputLanes>())
            }))
            .saturating_add(self.outputs.reserved_capacity_bytes())
            .saturating_add(self.pending_correlations.reserved_capacity_bytes())
            .saturating_add(self.prepared_correlations.reserved_capacity_bytes())
    }

    fn inline_bounded_capacity_bytes(&self) -> usize {
        std::mem::size_of_val(&self.decision)
            .saturating_add(std::mem::size_of_val(&self.tracked_quotes))
            .saturating_add(std::mem::size_of_val(&self.pending_schedules))
            .saturating_add(std::mem::size_of_val(&self.refresh_obligations))
            .saturating_add(std::mem::size_of_val(&self.retained_critical))
            .saturating_add(std::mem::size_of_val(&self.retained_persistence))
            .saturating_add(std::mem::size_of_val(&self.retained_private_admission))
            .saturating_add(std::mem::size_of_val(
                &self.retained_reconciliation_admission,
            ))
    }

    fn service_timer(
        &mut self,
        input: PmTimerInput,
        effects: &mut PmProductEffectBatch,
    ) -> Result<(), PmCoordinatorError> {
        self.validate_timer(&input)?;
        match input.kind() {
            PmScheduledActionKind::QuoteEvaluation => self.evaluate_quote(input, effects),
            PmScheduledActionKind::CancelOwnedQuote => self.cancel_tracked(
                input,
                PmJournalCancelReasonV1::SafetyHalt,
                PmControlReason::RequestedShutdown,
                effects,
            ),
            PmScheduledActionKind::ReconciliationRefresh => {
                let admitted = self.admit_next_refresh(input.monotonic_service_ns(), effects)?;
                push_metric(
                    effects,
                    PmHealthMetricKind::RefreshRequested,
                    u64::from(admitted),
                )
            }
            PmScheduledActionKind::Freshness => self.service_freshness(input, effects),
        }
    }

    fn validate_timer(&mut self, input: &PmTimerInput) -> Result<(), PmCoordinatorError> {
        if input.key().account_scope() != self.account_scope
            || input.key().instrument() != self.instrument
        {
            return Err(PmCoordinatorError::TimerScopeMismatch);
        }
        if input.local_action_sequence() <= self.last_action_sequence {
            return Err(PmCoordinatorError::TimerSequenceRegression);
        }
        if input.monotonic_service_ns() < input.deadline_ns() {
            return Err(PmCoordinatorError::ClockRegression);
        }
        self.last_action_sequence = input.local_action_sequence();
        Ok(())
    }

    fn evaluate_quote(
        &mut self,
        input: PmTimerInput,
        effects: &mut PmProductEffectBatch,
    ) -> Result<(), PmCoordinatorError> {
        if self.halt.is_some() || self.reconciliation_gate {
            return self.record_suppression(PmQuoteSuppression::CoordinatorHalted, effects);
        }
        self.counters.quote_evaluations = self.counters.quote_evaluations.saturating_add(1);
        match self.decision.evaluate(input.decision_monotonic_ns())? {
            DecisionOutcome::Suppressed(reason) => {
                self.record_suppression(reason, effects)?;
                match reason {
                    PmQuoteSuppression::ReferenceStale => self.cancel_tracked_owned_orders(
                        input,
                        PmJournalCancelReasonV1::StaleReference,
                        PmControlReason::PublicUnavailable,
                        effects,
                    ),
                    PmQuoteSuppression::BookStale
                    | PmQuoteSuppression::BookUnavailable
                    | PmQuoteSuppression::MarketUnavailable
                    | PmQuoteSuppression::RevisionMismatch => self.cancel_tracked_owned_orders(
                        input,
                        PmJournalCancelReasonV1::StaleBook,
                        PmControlReason::PublicUnavailable,
                        effects,
                    ),
                    PmQuoteSuppression::MissingReference
                    | PmQuoteSuppression::MissingMarket
                    | PmQuoteSuppression::MissingBook
                    | PmQuoteSuppression::PrivateUnavailable
                    | PmQuoteSuppression::RiskRejected
                    | PmQuoteSuppression::CoordinatorHalted => Ok(()),
                }
            }
            DecisionOutcome::Candidates(batch) => {
                self.counters.quote_policy_rejections = self
                    .counters
                    .quote_policy_rejections
                    .saturating_add(batch.policy_errors.iter().flatten().count() as u64);
                let candidates = batch.candidates;
                for candidate in candidates.into_iter().flatten() {
                    self.begin_candidate(input, batch, candidate, effects)?;
                }
                Ok(())
            }
        }
    }

    fn begin_candidate(
        &mut self,
        timer: PmTimerInput,
        batch: DecisionCandidateBatch,
        candidate: PmValidatedQuoteCandidate,
        effects: &mut PmProductEffectBatch,
    ) -> Result<(), PmCoordinatorError> {
        self.counters.quote_candidates = self.counters.quote_candidates.saturating_add(1);
        let revisions = PmAuthorityRevisions::new(
            batch.metadata_revision,
            batch.book_revision,
            batch.model_revision,
            batch.book_readiness_revision,
            self.private_readiness_revision,
        )?;
        self.mutation.update_revisions(revisions);
        let reservation = reservation_for(candidate)?;
        let request = PmQuoteMutationRequest::new(
            candidate,
            reservation,
            salt_for(timer.local_action_sequence(), candidate.side())?,
            timer.decision_wall_timestamp_ms(),
            timer.decision_monotonic_ns(),
            timer
                .decision_monotonic_ns()
                .checked_add(self.decision.policy.approval_lifetime_ns)
                .ok_or(PmCoordinatorError::ClockOverflow)?,
            batch.reference_observed_ns,
            batch.book_observed_ns,
        );
        let admission = match self.mutation.begin_quote(request) {
            Ok(admission) => admission,
            Err(PmMutationError::PrivateNotReady(_)) => {
                return self.record_suppression(PmQuoteSuppression::PrivateUnavailable, effects);
            }
            Err(PmMutationError::RiskRejected { halt, .. }) => {
                return self.handle_risk_rejection(timer, halt, effects);
            }
            Err(error) => return Err(error.into()),
        };
        match admission {
            PmQuoteMutationAdmission::JournalPending {
                intent_id,
                client_order,
            } => {
                let projection = PmFakeQuoteEffect::new(
                    self.account_scope,
                    self.instrument,
                    client_order,
                    candidate.side(),
                    candidate.price(),
                    candidate.quantity(),
                    PmFakeEffectStage::PreparedAfterDurability,
                );
                self.pending_correlations
                    .push(CopiedEffectCorrelation::Quote {
                        intent: intent_id,
                        effect: projection,
                    })?;
                self.track_quote(client_order, candidate.side());
                effects.push(PmProductEffect::DurableRecord(PmDurableRecordEffect::new(
                    PmDurableRecordKind::QuoteIntent,
                    Some(client_order),
                    intent_id,
                )))?;
                self.counters.durable_record_effects =
                    self.counters.durable_record_effects.saturating_add(1);
                push_metric(effects, PmHealthMetricKind::QuoteDecision, 1)
            }
            PmQuoteMutationAdmission::Duplicate { .. } => {
                push_metric(effects, PmHealthMetricKind::DuplicateQuote, 1)
            }
            PmQuoteMutationAdmission::CancelBeforeReplace { current, .. } => self.begin_cancel(
                current,
                candidate.side(),
                timer,
                PmJournalCancelReasonV1::Replacement,
                PmControlReason::RequestedShutdown,
                effects,
            ),
            PmQuoteMutationAdmission::ReplacementBlocked { .. } => {
                self.record_suppression(PmQuoteSuppression::CoordinatorHalted, effects)
            }
        }
    }

    fn service_freshness(
        &mut self,
        input: PmTimerInput,
        effects: &mut PmProductEffectBatch,
    ) -> Result<(), PmCoordinatorError> {
        match self.decision.evaluate(input.decision_monotonic_ns())? {
            DecisionOutcome::Suppressed(PmQuoteSuppression::ReferenceStale) => {
                self.record_suppression(PmQuoteSuppression::ReferenceStale, effects)?;
                self.cancel_tracked_owned_orders(
                    input,
                    PmJournalCancelReasonV1::StaleReference,
                    PmControlReason::PublicUnavailable,
                    effects,
                )
            }
            DecisionOutcome::Suppressed(
                reason @ (PmQuoteSuppression::BookStale
                | PmQuoteSuppression::BookUnavailable
                | PmQuoteSuppression::MarketUnavailable
                | PmQuoteSuppression::RevisionMismatch),
            ) => {
                self.record_suppression(reason, effects)?;
                self.cancel_tracked_owned_orders(
                    input,
                    PmJournalCancelReasonV1::StaleBook,
                    PmControlReason::PublicUnavailable,
                    effects,
                )
            }
            DecisionOutcome::Suppressed(reason) => self.record_suppression(reason, effects),
            DecisionOutcome::Candidates(_) => Ok(()),
        }
    }

    fn record_persistence_service(
        &mut self,
        service: PmPersistenceService,
        effects: &mut PmProductEffectBatch,
    ) -> Result<(), PmCoordinatorError> {
        match service {
            PmPersistenceService::Empty | PmPersistenceService::Pending => {
                push_metric(effects, PmHealthMetricKind::PersistencePending, 1)
            }
            PmPersistenceService::PreparedQuote { identity } => {
                let correlation = self.pending_correlations.remove_identity(identity)?;
                let CopiedEffectCorrelation::Quote {
                    intent,
                    effect: projection,
                } = correlation
                else {
                    return Err(PmCoordinatorError::CorrelationKindMismatch);
                };
                self.prepared_correlations
                    .push(CopiedEffectCorrelation::Quote {
                        intent,
                        effect: projection,
                    })?;
                self.mark_tracked_prepared(projection.client_order());
                effects.push(PmProductEffect::FakePassiveQuote(projection))?;
                self.counters.fake_quote_effects =
                    self.counters.fake_quote_effects.saturating_add(1);
                push_metric(effects, PmHealthMetricKind::PersistenceAcknowledged, 1)
            }
            PmPersistenceService::PreparedCancel { identity } => {
                let correlation = self.pending_correlations.remove_identity(identity)?;
                let CopiedEffectCorrelation::Cancel(projection) = correlation else {
                    return Err(PmCoordinatorError::CorrelationKindMismatch);
                };
                self.prepared_correlations
                    .push(CopiedEffectCorrelation::Cancel(projection))?;
                effects.push(PmProductEffect::FakeCancelOwned(projection))?;
                self.counters.fake_cancel_effects =
                    self.counters.fake_cancel_effects.saturating_add(1);
                push_metric(effects, PmHealthMetricKind::PersistenceAcknowledged, 1)
            }
            PmPersistenceService::QuoteInvalidated { identity } => {
                let correlation = self.pending_correlations.remove_identity(identity)?;
                let CopiedEffectCorrelation::Quote { effect, .. } = correlation else {
                    return Err(PmCoordinatorError::CorrelationKindMismatch);
                };
                self.clear_tracked_quote(effect.client_order());
                self.record_suppression(PmQuoteSuppression::RevisionMismatch, effects)
            }
            PmPersistenceService::FactAcknowledged { sequence } => push_metric(
                effects,
                PmHealthMetricKind::PersistenceAcknowledged,
                sequence,
            ),
            PmPersistenceService::IntentFailed { identity } => {
                let failed = self.pending_correlations.remove_identity(identity)?;
                if let CopiedEffectCorrelation::Quote { effect, .. } = failed {
                    self.clear_tracked_quote(effect.client_order());
                }
                self.latch_halt(PmControlReason::PersistenceUnavailable);
                effects.push(PmProductEffect::FailClosedHaltOrCancel(
                    PmFailClosedEffect::halt(
                        self.account_scope,
                        self.instrument,
                        PmControlReason::PersistenceUnavailable,
                    ),
                ))?;
                Ok(())
            }
            PmPersistenceService::FactFailed => {
                self.latch_halt(PmControlReason::PersistenceUnavailable);
                effects.push(PmProductEffect::FailClosedHaltOrCancel(
                    PmFailClosedEffect::halt(
                        self.account_scope,
                        self.instrument,
                        PmControlReason::PersistenceUnavailable,
                    ),
                ))?;
                Ok(())
            }
        }
    }

    fn record_suppression(
        &mut self,
        _reason: PmQuoteSuppression,
        effects: &mut PmProductEffectBatch,
    ) -> Result<(), PmCoordinatorError> {
        self.counters.quote_suppressions = self.counters.quote_suppressions.saturating_add(1);
        push_metric(effects, PmHealthMetricKind::QuoteSuppressed, 1)
    }

    fn track_quote(&mut self, client_order: PmClientOrderKey, side: PmOrderSide) {
        self.tracked_quotes[side_index(side)] = Some(TrackedQuote {
            client_order,
            side,
            stage: TrackedQuoteStage::PersistencePending,
        });
    }

    fn mark_tracked_prepared(&mut self, client_order: PmClientOrderKey) {
        for tracked in &mut self.tracked_quotes {
            if let Some(tracked) = tracked
                && tracked.client_order == client_order
            {
                tracked.stage = TrackedQuoteStage::PreparedLocal;
            }
        }
    }

    fn refresh_tracked_quote(&mut self, client_order: PmClientOrderKey) {
        let Some(projection) = self.mutation.private_mut().owned_order(client_order) else {
            self.clear_tracked_quote(client_order);
            return;
        };
        if projection.submit() == PmOwnedSubmitState::Rejected
            || projection
                .status()
                .is_some_and(|status| status.is_terminal())
        {
            self.clear_tracked_quote(client_order);
            return;
        }
        for tracked in &mut self.tracked_quotes {
            if let Some(tracked) = tracked
                && tracked.client_order == client_order
            {
                tracked.stage = if projection.venue_order().is_some() {
                    TrackedQuoteStage::RemotelyLive
                } else {
                    TrackedQuoteStage::DispatchedWithoutVenue
                };
            }
        }
    }

    fn clear_tracked_quote(&mut self, client_order: PmClientOrderKey) {
        for tracked in &mut self.tracked_quotes {
            if tracked.is_some_and(|value| value.client_order == client_order) {
                *tracked = None;
            }
        }
    }

    fn publish_effect_batch(
        &mut self,
        effects: PmProductEffectBatch,
    ) -> Result<(), PmCoordinatorError> {
        self.counters.maximum_effects_per_input = self
            .counters
            .maximum_effects_per_input
            .max(u8::try_from(effects.len()).expect("fixed effect batch fits u8"));
        self.counters.correlation_high_water = self
            .pending_correlations
            .high_water()
            .max(self.prepared_correlations.high_water());
        self.outputs.push_batch(effects)?;
        Ok(())
    }
}

impl From<PmEffectCapacityError> for PmCoordinatorError {
    fn from(_: PmEffectCapacityError) -> Self {
        Self::EffectProjectionSaturated
    }
}

#[derive(Debug, Error)]
pub(crate) enum PmCoordinatorError {
    #[error("model requirements do not match the configured reference mapping")]
    ModelRequirementsMismatch,
    #[error("OKX reference scope does not match the configured model")]
    ReferenceScopeMismatch,
    #[error("PM market scope does not match the configured instrument")]
    MarketScopeMismatch,
    #[error("PM book scope does not match the configured instrument")]
    BookScopeMismatch,
    #[error("one exact occurrence carried conflicting facts")]
    ConflictingOccurrence,
    #[error("a coordinator revision is exhausted")]
    RevisionExhausted,
    #[error("monotonic clock regressed")]
    ClockRegression,
    #[error("monotonic clock arithmetic overflowed")]
    ClockOverflow,
    #[error("scheduled action scope does not match the product")]
    TimerScopeMismatch,
    #[error("scheduled local action sequence regressed or repeated")]
    TimerSequenceRegression,
    #[error("deterministic order salt arithmetic overflowed")]
    SaltOverflow,
    #[error("copied effect-correlation capacity is saturated")]
    CopiedCorrelationSaturated,
    #[error("prepared effect has no copied correlation")]
    MissingCopiedCorrelation,
    #[error("prepared effect kind conflicts with copied correlation")]
    CorrelationKindMismatch,
    #[error("tracked prepared quote is absent from the sole mutation effect owner")]
    PreparedQuoteAuthorityMismatch,
    #[error("fixed per-input copied-effect output is saturated")]
    EffectProjectionSaturated,
    #[error("fixed retained refresh-obligation storage is saturated")]
    RefreshRetentionSaturated,
    #[error("canonical refresh admission/completion disagrees with retained correlation")]
    RefreshAdmissionMismatch,
    #[error("product service clocks must be positive")]
    ZeroServiceClock,
    #[error("captured wall receive evidence cannot represent a positive millisecond timestamp")]
    InvalidCapturedWallTimestamp,
    #[error("complete scheduler re-entered its sole coordinator consumer")]
    SchedulerReentrant,
    #[error("critical input remains retained after a failed lane admission")]
    CriticalAdmissionRetained,
    #[error("persistence input remains retained after a failed lane admission")]
    PersistenceAdmissionRetained,
    #[error("private input remains retained after a failed lane admission")]
    PrivateAdmissionRetained,
    #[error("reconciliation input remains retained after a failed lane admission")]
    ReconciliationAdmissionRetained,
    #[error("critical input lane rejected an exact input")]
    CriticalLaneRejected,
    #[error("persistence input lane rejected an exact input")]
    PersistenceLaneRejected,
    #[error("private input lane rejected an exact input")]
    PrivateLaneRejected,
    #[error("reconciliation input lane rejected an exact input")]
    ReconciliationLaneRejected,
    #[error("telemetry input lane rejected an input")]
    TelemetryLaneRejected,
    #[error("the fixed pending schedule set is saturated")]
    PendingScheduleSaturated,
    #[error("the product public callback lacks the canonical PM book projection")]
    MissingCanonicalBookProjection,
    #[error("a PM account input requires the configured product source")]
    AccountSourceRequired,
    #[error("quote model input is invalid: {0:?}")]
    Model(PmQuoteModelError),
    #[error(transparent)]
    Authority(#[from] PmAuthorityError),
    #[error(transparent)]
    Numeric(#[from] reap_pm_core::PmNumericError),
    #[error(transparent)]
    State(#[from] reap_pm_state::PmOrderStateError),
    #[error(transparent)]
    Mutation(#[from] PmMutationError),
    #[error("complete scheduler failed: {0}")]
    CompleteScheduler(crate::lanes::PmCompleteServiceError),
    #[error("quote schedule failed: {0:?}")]
    Schedule(crate::schedule::PmScheduleError),
    #[error(transparent)]
    ProductInput(#[from] super::input::PmProductInputError),
    #[error(transparent)]
    PrivateMonitor(#[from] crate::private_monitor::PmPrivateMonitorError),
    #[error(transparent)]
    Envelope(#[from] reap_pm_core::EnvelopeError),
}

impl From<PmQuoteModelError> for PmCoordinatorError {
    fn from(error: PmQuoteModelError) -> Self {
        Self::Model(error)
    }
}

impl PmCoordinatorError {
    const fn callback_requires_global_halt(&self) -> bool {
        !matches!(
            self,
            Self::Mutation(
                PmMutationError::PrivateNotReady(_)
                    | PmMutationError::RiskRejected { .. }
                    | PmMutationError::DurableConsequenceSaturated
                    | PmMutationError::Persistence(PmPersistenceError::Full)
            )
        )
    }
}

#[cfg(test)]
mod control_tests;
#[cfg(test)]
mod tests;
