use reap_pm_core::{
    PmAccountScope, PmClientOrderKey, PmInstrumentHandle, PmInstrumentId, PmOrderSalt, PmOrderSide,
    PmPrice, PmQuantity, PmVenueOrderKey, SnapshotRevision, U256, exact_order_amounts,
};
use reap_pm_state::{
    PmExactReservation, PmOwnedCancelRequestApply, PmOwnedIntentId, PmOwnedOrderProjection,
    PmOwnedQuoteAdmission, PmOwnedQuoteIntent, PmPrivateReady, PmReservationBasis, PmRiskDecision,
};
use reap_pm_strategy::PmValidatedQuoteCandidate;
use reap_polymarket_adapter::{
    PmCancelOwnedPurpose, PmFakeCancelCommand, PmFakeExecutionError, PmFakePlaceCommand,
    PmFixtureInstrumentScope, PmFixtureOwnedExecution, PmGtcPostOnlyProfile,
};
use thiserror::Error;

use crate::journal::{
    PmCancelIntentDurablyAcknowledged, PmJournalCancelIntentV1, PmJournalCancelReasonV1,
    PmJournalQuoteIntentV1, PmJournalQuoteProfileV1, PmJournalSideV1,
    PmQuoteIntentDurablyAcknowledged,
};

/// Exact revisions against which one PM mutation was approved.
///
/// This is copied diagnostic data, not mutation authority.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct PmAuthorityRevisions {
    metadata: SnapshotRevision,
    book: SnapshotRevision,
    model: u64,
    book_readiness: u64,
    private_readiness: u64,
}

impl PmAuthorityRevisions {
    pub fn new(
        metadata: SnapshotRevision,
        book: SnapshotRevision,
        model: u64,
        book_readiness: u64,
        private_readiness: u64,
    ) -> Result<Self, PmAuthorityError> {
        if metadata.value() == 0
            || book.value() == 0
            || model == 0
            || book_readiness == 0
            || private_readiness == 0
        {
            return Err(PmAuthorityError::ZeroRevision);
        }
        Ok(Self {
            metadata,
            book,
            model,
            book_readiness,
            private_readiness,
        })
    }

    #[must_use]
    pub const fn metadata(self) -> SnapshotRevision {
        self.metadata
    }

    #[must_use]
    pub const fn book(self) -> SnapshotRevision {
        self.book
    }

    #[must_use]
    pub const fn model(self) -> u64 {
        self.model
    }

    #[must_use]
    pub const fn book_readiness(self) -> u64 {
        self.book_readiness
    }

    #[must_use]
    pub const fn private_readiness(self) -> u64 {
        self.private_readiness
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
struct PmQuoteAuthorityFacts {
    account_scope: PmAccountScope,
    instrument: PmInstrumentHandle,
    instrument_id: PmInstrumentId,
    intent: PmOwnedIntentId,
    client_order: PmClientOrderKey,
    side: PmOrderSide,
    price: PmPrice,
    quantity: PmQuantity,
    maker_amount: U256,
    taker_amount: U256,
    reservation: PmExactReservation,
    profile: PmGtcPostOnlyProfile,
    salt: PmOrderSalt,
    timestamp_ms: u64,
    revisions: PmAuthorityRevisions,
    approved_at_monotonic_ns: u64,
    expires_at_monotonic_ns: u64,
}

/// Risk/readiness-approved quote authority.
///
/// Its fields and constructor are private, and the value is deliberately not
/// `Clone` or `Copy`.
#[derive(Debug)]
pub struct ApprovedPmQuote {
    facts: PmQuoteAuthorityFacts,
}

/// Quote authority whose exact reservation and quote slot were admitted by
/// the canonical state owner.
#[derive(Debug)]
pub struct ReservedPmQuote {
    facts: PmQuoteAuthorityFacts,
}

/// Durably journaled quote authority containing one exact fake command.
///
/// Only the crate-private fake effect role can extract and consume the
/// command.
#[derive(Debug)]
pub struct PreparedPmQuote {
    facts: PmQuoteAuthorityFacts,
    journal_sequence: u64,
    command: PmFakePlaceCommand,
}

macro_rules! quote_accessors {
    ($stage:ty) => {
        impl $stage {
            #[must_use]
            pub const fn account_scope(&self) -> PmAccountScope {
                self.facts.account_scope
            }

            #[must_use]
            pub const fn instrument(&self) -> PmInstrumentHandle {
                self.facts.instrument
            }

            #[must_use]
            pub const fn instrument_id(&self) -> PmInstrumentId {
                self.facts.instrument_id
            }

            #[must_use]
            pub const fn intent(&self) -> PmOwnedIntentId {
                self.facts.intent
            }

            #[must_use]
            pub const fn client_order(&self) -> PmClientOrderKey {
                self.facts.client_order
            }

            #[must_use]
            pub const fn side(&self) -> PmOrderSide {
                self.facts.side
            }

            #[must_use]
            pub const fn price(&self) -> PmPrice {
                self.facts.price
            }

            #[must_use]
            pub const fn quantity(&self) -> PmQuantity {
                self.facts.quantity
            }

            #[must_use]
            pub const fn maker_amount(&self) -> U256 {
                self.facts.maker_amount
            }

            #[must_use]
            pub const fn taker_amount(&self) -> U256 {
                self.facts.taker_amount
            }

            #[must_use]
            pub const fn reservation(&self) -> PmExactReservation {
                self.facts.reservation
            }

            #[must_use]
            pub const fn profile(&self) -> PmGtcPostOnlyProfile {
                self.facts.profile
            }

            #[must_use]
            pub const fn salt(&self) -> PmOrderSalt {
                self.facts.salt
            }

            #[must_use]
            pub const fn timestamp_ms(&self) -> u64 {
                self.facts.timestamp_ms
            }

            #[must_use]
            pub const fn revisions(&self) -> PmAuthorityRevisions {
                self.facts.revisions
            }

            #[must_use]
            pub const fn approved_at_monotonic_ns(&self) -> u64 {
                self.facts.approved_at_monotonic_ns
            }

            #[must_use]
            pub const fn expires_at_monotonic_ns(&self) -> u64 {
                self.facts.expires_at_monotonic_ns
            }
        }
    };
}

quote_accessors!(ApprovedPmQuote);
quote_accessors!(ReservedPmQuote);
quote_accessors!(PreparedPmQuote);

impl ApprovedPmQuote {
    pub(crate) fn reserve(
        self,
        intent: PmOwnedQuoteIntent,
        admission: PmOwnedQuoteAdmission,
    ) -> Result<ReservedPmQuote, PmAuthorityError> {
        let facts = self.facts;
        let exact_intent = intent.intent() == facts.intent
            && intent.slot().account_scope() == facts.account_scope
            && intent.slot().instrument() == facts.instrument
            && intent.slot().side() == facts.side
            && intent.client_order() == facts.client_order
            && intent.price() == facts.price
            && intent.quantity() == facts.quantity
            && intent.reservation() == facts.reservation;
        if !exact_intent || admission != PmOwnedQuoteAdmission::Admitted(facts.client_order) {
            return Err(PmAuthorityError::ReservationAdmissionMismatch);
        }
        Ok(ReservedPmQuote { facts })
    }
}

impl ReservedPmQuote {
    #[must_use]
    pub(crate) fn journal_intent(&self) -> PmJournalQuoteIntentV1 {
        quote_journal_intent(self.facts)
    }
}

impl PreparedPmQuote {
    #[must_use]
    pub const fn journal_sequence(&self) -> u64 {
        self.journal_sequence
    }
}

#[allow(clippy::too_many_arguments)]
pub(crate) fn approve_pm_quote(
    account_scope: PmAccountScope,
    instrument_id: PmInstrumentId,
    intent: PmOwnedIntentId,
    client_order: PmClientOrderKey,
    candidate: PmValidatedQuoteCandidate,
    reservation: PmExactReservation,
    profile: PmGtcPostOnlyProfile,
    salt: PmOrderSalt,
    timestamp_ms: u64,
    revisions: PmAuthorityRevisions,
    approved_at_monotonic_ns: u64,
    expires_at_monotonic_ns: u64,
    private_ready: PmPrivateReady,
    risk_decision: PmRiskDecision,
) -> Result<ApprovedPmQuote, PmAuthorityError> {
    if client_order.account() != account_scope.handle() {
        return Err(PmAuthorityError::ScopeMismatch);
    }
    if candidate.instrument_id() != instrument_id {
        return Err(PmAuthorityError::ScopeMismatch);
    }
    if account_scope.signer().address() != account_scope.funder().address() {
        return Err(PmAuthorityError::EoaIdentityMismatch);
    }
    validate_profile(profile)?;
    validate_window(
        timestamp_ms,
        approved_at_monotonic_ns,
        expires_at_monotonic_ns,
    )?;
    if reservation.basis() != PmReservationBasis::PolicyApprovedWorstCase
        || reservation
            .validate_for(candidate.side(), candidate.price(), candidate.quantity())
            .is_err()
    {
        return Err(PmAuthorityError::InvalidReservation);
    }
    if private_ready.candidate_collateral() != reservation.collateral()
        || private_ready.candidate_outcome() != reservation.outcome()
    {
        return Err(PmAuthorityError::ReadinessReservationMismatch);
    }
    if !matches!(risk_decision, PmRiskDecision::Approved { .. }) {
        return Err(PmAuthorityError::RiskNotApproved);
    }

    Ok(ApprovedPmQuote {
        facts: PmQuoteAuthorityFacts {
            account_scope,
            instrument: candidate.instrument(),
            instrument_id,
            intent,
            client_order,
            side: candidate.side(),
            price: candidate.price(),
            quantity: candidate.quantity(),
            maker_amount: candidate.maker_amount(),
            taker_amount: candidate.taker_amount(),
            reservation,
            profile,
            salt,
            timestamp_ms,
            revisions,
            approved_at_monotonic_ns,
            expires_at_monotonic_ns,
        },
    })
}

#[allow(clippy::too_many_arguments)]
pub(crate) fn prepare_pm_quote(
    execution: &PmFixtureOwnedExecution,
    expected_instrument_id: PmInstrumentId,
    reserved: ReservedPmQuote,
    current_scope: PmFixtureInstrumentScope,
    current_revisions: PmAuthorityRevisions,
    monotonic_now_ns: u64,
    acknowledged: PmQuoteIntentDurablyAcknowledged,
) -> Result<PreparedPmQuote, PmAuthorityError> {
    let facts = reserved.facts;
    validate_execution_scope(execution, expected_instrument_id, current_scope, facts)?;
    validate_current(
        facts.revisions,
        current_revisions,
        monotonic_now_ns,
        facts.expires_at_monotonic_ns,
    )?;

    let expected_intent = quote_journal_intent(facts);
    let (journal_sequence, durable_intent) = consume_quote_acknowledgement(acknowledged);
    if journal_sequence == 0 || durable_intent != expected_intent {
        return Err(PmAuthorityError::DurableIntentMismatch);
    }

    let command = execution.place_command(
        current_scope,
        facts.client_order,
        facts.salt,
        facts.side,
        facts.price,
        facts.quantity,
        facts.timestamp_ms,
    )?;
    let unsigned = command.unsigned_order();
    if unsigned.maker_amount() != facts.maker_amount
        || unsigned.taker_amount() != facts.taker_amount
        || command.profile() != facts.profile
    {
        return Err(PmAuthorityError::LoweringMismatch);
    }
    Ok(PreparedPmQuote {
        facts,
        journal_sequence,
        command,
    })
}

pub(crate) fn consume_prepared_quote(
    prepared: PreparedPmQuote,
    account_scope: PmAccountScope,
    instrument: PmInstrumentHandle,
    instrument_id: PmInstrumentId,
) -> Result<PmFakePlaceCommand, PmAuthorityError> {
    if prepared.facts.account_scope != account_scope
        || prepared.facts.instrument != instrument
        || prepared.facts.instrument_id != instrument_id
    {
        return Err(PmAuthorityError::ScopeMismatch);
    }
    Ok(prepared.command)
}

fn quote_journal_intent(facts: PmQuoteAuthorityFacts) -> PmJournalQuoteIntentV1 {
    PmJournalQuoteIntentV1 {
        intent_id: facts.intent.value(),
        client_order: facts.client_order,
        instrument: facts.instrument_id,
        side: PmJournalSideV1::from(facts.side),
        price_units: facts.price.units(),
        quantity: facts.quantity,
        reserved_collateral: facts.reservation.collateral(),
        reserved_outcome: facts.reservation.outcome(),
        profile: PmJournalQuoteProfileV1::PassiveGtcPostOnlyEoa,
        metadata_revision: facts.revisions.metadata().value(),
        book_revision: facts.revisions.book().value(),
        model_revision: facts.revisions.model(),
        book_readiness_revision: facts.revisions.book_readiness(),
        private_readiness_revision: facts.revisions.private_readiness(),
        expires_at_monotonic_ns: facts.expires_at_monotonic_ns,
        salt: facts.salt,
        timestamp_ms: facts.timestamp_ms,
        maker: facts.account_scope.funder().address(),
        signer: facts.account_scope.signer().address(),
        maker_amount: facts.maker_amount,
        taker_amount: facts.taker_amount,
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
struct PmCancelAuthorityFacts {
    account_scope: PmAccountScope,
    instrument: PmInstrumentHandle,
    instrument_id: PmInstrumentId,
    intent: PmOwnedIntentId,
    client_order: PmClientOrderKey,
    venue_order: PmVenueOrderKey,
    side: PmOrderSide,
    price: PmPrice,
    quantity: PmQuantity,
    maker_amount: U256,
    taker_amount: U256,
    reservation: PmExactReservation,
    quote_profile: PmGtcPostOnlyProfile,
    cancel_purpose: PmCancelOwnedPurpose,
    salt: PmOrderSalt,
    timestamp_ms: u64,
    approved_at_monotonic_ns: u64,
    expires_at_monotonic_ns: u64,
    reason: PmJournalCancelReasonV1,
}

#[derive(Debug)]
pub struct ApprovedPmCancel {
    facts: PmCancelAuthorityFacts,
}

#[derive(Debug)]
pub struct ReservedPmCancel {
    facts: PmCancelAuthorityFacts,
}

#[derive(Debug)]
pub struct PreparedPmCancel {
    facts: PmCancelAuthorityFacts,
    journal_sequence: u64,
    command: PmFakeCancelCommand,
}

macro_rules! cancel_accessors {
    ($stage:ty) => {
        impl $stage {
            #[must_use]
            pub const fn account_scope(&self) -> PmAccountScope {
                self.facts.account_scope
            }

            #[must_use]
            pub const fn instrument(&self) -> PmInstrumentHandle {
                self.facts.instrument
            }

            #[must_use]
            pub const fn instrument_id(&self) -> PmInstrumentId {
                self.facts.instrument_id
            }

            #[must_use]
            pub const fn intent(&self) -> PmOwnedIntentId {
                self.facts.intent
            }

            #[must_use]
            pub const fn client_order(&self) -> PmClientOrderKey {
                self.facts.client_order
            }

            #[must_use]
            pub const fn venue_order(&self) -> PmVenueOrderKey {
                self.facts.venue_order
            }

            #[must_use]
            pub const fn side(&self) -> PmOrderSide {
                self.facts.side
            }

            #[must_use]
            pub const fn price(&self) -> PmPrice {
                self.facts.price
            }

            #[must_use]
            pub const fn quantity(&self) -> PmQuantity {
                self.facts.quantity
            }

            #[must_use]
            pub const fn maker_amount(&self) -> U256 {
                self.facts.maker_amount
            }

            #[must_use]
            pub const fn taker_amount(&self) -> U256 {
                self.facts.taker_amount
            }

            #[must_use]
            pub const fn reservation(&self) -> PmExactReservation {
                self.facts.reservation
            }

            #[must_use]
            pub const fn quote_profile(&self) -> PmGtcPostOnlyProfile {
                self.facts.quote_profile
            }

            #[must_use]
            pub const fn cancel_purpose(&self) -> PmCancelOwnedPurpose {
                self.facts.cancel_purpose
            }

            #[must_use]
            pub const fn salt(&self) -> PmOrderSalt {
                self.facts.salt
            }

            #[must_use]
            pub const fn timestamp_ms(&self) -> u64 {
                self.facts.timestamp_ms
            }

            #[must_use]
            pub const fn approved_at_monotonic_ns(&self) -> u64 {
                self.facts.approved_at_monotonic_ns
            }

            #[must_use]
            pub const fn expires_at_monotonic_ns(&self) -> u64 {
                self.facts.expires_at_monotonic_ns
            }

            #[must_use]
            pub const fn reason(&self) -> PmJournalCancelReasonV1 {
                self.facts.reason
            }
        }
    };
}

cancel_accessors!(ApprovedPmCancel);
cancel_accessors!(ReservedPmCancel);
cancel_accessors!(PreparedPmCancel);

impl ApprovedPmCancel {
    pub(crate) fn reserve(
        self,
        request: PmOwnedCancelRequestApply,
    ) -> Result<ReservedPmCancel, PmAuthorityError> {
        let facts = self.facts;
        let PmOwnedCancelRequestApply::Issued(intent) = request else {
            return Err(PmAuthorityError::CancelAdmissionMismatch);
        };
        if intent.client_order() != facts.client_order || intent.venue_order() != facts.venue_order
        {
            return Err(PmAuthorityError::CancelAdmissionMismatch);
        }
        Ok(ReservedPmCancel { facts })
    }
}

impl ReservedPmCancel {
    #[must_use]
    pub(crate) fn journal_intent(&self) -> PmJournalCancelIntentV1 {
        cancel_journal_intent(self.facts)
    }
}

impl PreparedPmCancel {
    #[must_use]
    pub const fn journal_sequence(&self) -> u64 {
        self.journal_sequence
    }
}

#[allow(clippy::too_many_arguments)]
pub(crate) fn approve_pm_cancel(
    account_scope: PmAccountScope,
    instrument_id: PmInstrumentId,
    order: PmOwnedOrderProjection,
    quote_profile: PmGtcPostOnlyProfile,
    cancel_purpose: PmCancelOwnedPurpose,
    salt: PmOrderSalt,
    timestamp_ms: u64,
    approved_at_monotonic_ns: u64,
    expires_at_monotonic_ns: u64,
    reason: PmJournalCancelReasonV1,
) -> Result<ApprovedPmCancel, PmAuthorityError> {
    let slot = order.slot();
    let venue_order = order
        .venue_order()
        .ok_or(PmAuthorityError::MissingVenueOrder)?;
    if slot.account_scope() != account_scope
        || order.client_order().account() != account_scope.handle()
        || venue_order.account() != account_scope.handle()
    {
        return Err(PmAuthorityError::ScopeMismatch);
    }
    if account_scope.signer().address() != account_scope.funder().address() {
        return Err(PmAuthorityError::EoaIdentityMismatch);
    }
    validate_profile(quote_profile)?;
    validate_window(
        timestamp_ms,
        approved_at_monotonic_ns,
        expires_at_monotonic_ns,
    )?;
    if order.reservation().basis() != PmReservationBasis::PolicyApprovedWorstCase
        || order
            .reservation()
            .validate_for(slot.side(), order.price(), order.quantity())
            .is_err()
    {
        return Err(PmAuthorityError::InvalidReservation);
    }
    let amounts = exact_order_amounts(slot.side(), order.price(), order.quantity())
        .map_err(|_| PmAuthorityError::LoweringMismatch)?;
    Ok(ApprovedPmCancel {
        facts: PmCancelAuthorityFacts {
            account_scope,
            instrument: slot.instrument(),
            instrument_id,
            intent: order.intent(),
            client_order: order.client_order(),
            venue_order,
            side: slot.side(),
            price: order.price(),
            quantity: order.quantity(),
            maker_amount: amounts.maker(),
            taker_amount: amounts.taker(),
            reservation: order.reservation(),
            quote_profile,
            cancel_purpose,
            salt,
            timestamp_ms,
            approved_at_monotonic_ns,
            expires_at_monotonic_ns,
            reason,
        },
    })
}

#[allow(clippy::too_many_arguments)]
pub(crate) fn prepare_pm_cancel(
    execution: &PmFixtureOwnedExecution,
    expected_instrument_id: PmInstrumentId,
    reserved: ReservedPmCancel,
    current_scope: PmFixtureInstrumentScope,
    monotonic_now_ns: u64,
    acknowledged: PmCancelIntentDurablyAcknowledged,
) -> Result<PreparedPmCancel, PmAuthorityError> {
    let facts = reserved.facts;
    validate_cancel_execution_scope(execution, expected_instrument_id, current_scope, facts)?;
    validate_not_expired(monotonic_now_ns, facts.expires_at_monotonic_ns)?;
    let expected_intent = cancel_journal_intent(facts);
    let (journal_sequence, durable_intent) = consume_cancel_acknowledgement(acknowledged);
    if journal_sequence == 0 || durable_intent != expected_intent {
        return Err(PmAuthorityError::DurableIntentMismatch);
    }

    let command = execution.cancel_command(current_scope, facts.client_order, facts.venue_order)?;
    if command.purpose() != facts.cancel_purpose {
        return Err(PmAuthorityError::LoweringMismatch);
    }
    Ok(PreparedPmCancel {
        facts,
        journal_sequence,
        command,
    })
}

pub(crate) fn consume_prepared_cancel(
    prepared: PreparedPmCancel,
    account_scope: PmAccountScope,
    instrument: PmInstrumentHandle,
    instrument_id: PmInstrumentId,
) -> Result<PmFakeCancelCommand, PmAuthorityError> {
    if prepared.facts.account_scope != account_scope
        || prepared.facts.instrument != instrument
        || prepared.facts.instrument_id != instrument_id
    {
        return Err(PmAuthorityError::ScopeMismatch);
    }
    Ok(prepared.command)
}

fn cancel_journal_intent(facts: PmCancelAuthorityFacts) -> PmJournalCancelIntentV1 {
    PmJournalCancelIntentV1 {
        client_order: facts.client_order,
        venue_order: facts.venue_order,
        reason: facts.reason,
    }
}

fn consume_quote_acknowledgement(
    acknowledged: PmQuoteIntentDurablyAcknowledged,
) -> (u64, PmJournalQuoteIntentV1) {
    (acknowledged.sequence(), acknowledged.intent())
}

fn consume_cancel_acknowledgement(
    acknowledged: PmCancelIntentDurablyAcknowledged,
) -> (u64, PmJournalCancelIntentV1) {
    (acknowledged.sequence(), acknowledged.intent())
}

fn validate_profile(profile: PmGtcPostOnlyProfile) -> Result<(), PmAuthorityError> {
    if profile.order_type() != reap_polymarket_adapter::PmFakeOrderType::Gtc
        || !profile.post_only()
        || profile.defer_exec()
        || profile.expiration() != 0
    {
        Err(PmAuthorityError::ProfileMismatch)
    } else {
        Ok(())
    }
}

fn validate_window(
    timestamp_ms: u64,
    approved_at_monotonic_ns: u64,
    expires_at_monotonic_ns: u64,
) -> Result<(), PmAuthorityError> {
    if timestamp_ms == 0
        || approved_at_monotonic_ns == 0
        || expires_at_monotonic_ns <= approved_at_monotonic_ns
    {
        Err(PmAuthorityError::InvalidTimeWindow)
    } else {
        Ok(())
    }
}

fn validate_current(
    approved: PmAuthorityRevisions,
    current: PmAuthorityRevisions,
    monotonic_now_ns: u64,
    expires_at_monotonic_ns: u64,
) -> Result<(), PmAuthorityError> {
    if approved != current {
        return Err(PmAuthorityError::RevisionChanged);
    }
    validate_not_expired(monotonic_now_ns, expires_at_monotonic_ns)
}

fn validate_not_expired(
    monotonic_now_ns: u64,
    expires_at_monotonic_ns: u64,
) -> Result<(), PmAuthorityError> {
    if monotonic_now_ns == 0 || monotonic_now_ns >= expires_at_monotonic_ns {
        return Err(PmAuthorityError::ApprovalExpired);
    }
    Ok(())
}

fn validate_execution_scope(
    execution: &PmFixtureOwnedExecution,
    expected_instrument_id: PmInstrumentId,
    current_scope: PmFixtureInstrumentScope,
    facts: PmQuoteAuthorityFacts,
) -> Result<(), PmAuthorityError> {
    if execution.account_scope() != facts.account_scope
        || execution.instrument() != facts.instrument
        || expected_instrument_id != facts.instrument_id
        || current_scope.handle() != facts.instrument
        || current_scope.id() != facts.instrument_id
        || execution.place_profile() != facts.profile
    {
        return Err(PmAuthorityError::ScopeMismatch);
    }
    Ok(())
}

fn validate_cancel_execution_scope(
    execution: &PmFixtureOwnedExecution,
    expected_instrument_id: PmInstrumentId,
    current_scope: PmFixtureInstrumentScope,
    facts: PmCancelAuthorityFacts,
) -> Result<(), PmAuthorityError> {
    if execution.account_scope() != facts.account_scope
        || execution.instrument() != facts.instrument
        || expected_instrument_id != facts.instrument_id
        || current_scope.handle() != facts.instrument
        || current_scope.id() != facts.instrument_id
        || execution.place_profile() != facts.quote_profile
        || execution.cancel_purpose() != facts.cancel_purpose
    {
        return Err(PmAuthorityError::ScopeMismatch);
    }
    Ok(())
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Error)]
pub enum PmAuthorityError {
    #[error("PM mutation authority requires nonzero exact revisions")]
    ZeroRevision,
    #[error("PM mutation authority scope or identity does not match")]
    ScopeMismatch,
    #[error("PM mutation authority requires the fixed fake execution profile")]
    ProfileMismatch,
    #[error("PM mutation authority requires maker, signer, and funder to be one EOA")]
    EoaIdentityMismatch,
    #[error("PM mutation authority has an invalid timestamp or monotonic expiry")]
    InvalidTimeWindow,
    #[error("PM quote reservation is not the exact policy-approved requirement")]
    InvalidReservation,
    #[error("PM private readiness does not bind the exact reservation")]
    ReadinessReservationMismatch,
    #[error("PM risk decision did not approve the exact coordinator turn")]
    RiskNotApproved,
    #[error("PM quote slot did not admit the exact approved intent and reservation")]
    ReservationAdmissionMismatch,
    #[error("PM cancel state did not issue the exact owned cancel")]
    CancelAdmissionMismatch,
    #[error("PM owned cancel has no exact venue-order identity")]
    MissingVenueOrder,
    #[error("PM metadata, book, model, or readiness revision changed before preparation")]
    RevisionChanged,
    #[error("PM approval expired before preparation")]
    ApprovalExpired,
    #[error("durable acknowledgement does not name the exact mutation intent")]
    DurableIntentMismatch,
    #[error("exact command lowering differs from the approved authority")]
    LoweringMismatch,
    #[error(transparent)]
    FakeExecution(#[from] PmFakeExecutionError),
}

#[cfg(test)]
mod tests;
