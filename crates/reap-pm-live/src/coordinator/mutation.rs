#![allow(
    clippy::large_enum_variant,
    clippy::result_large_err,
    reason = "exact fail-closed PM readiness evidence stays inline so rejection and overload paths remain allocation-free"
)]

use std::collections::VecDeque;
use std::path::PathBuf;

use reap_pm_core::{
    EventClock, EventEnvelope, EventOrdering, PmClientOrderKey, PmCompleteAccountSnapshot,
    PmCompleteFillQuery, PmConnectionId, PmEventError, PmInstrumentId, PmNumericError, PmOrderSalt,
    PmProductSource, PmVenueOrderKey,
};
use reap_pm_live_contracts::PmConnectivityConfig;
use reap_pm_state::{
    PmExactReservation, PmOwnedCancelIntent, PmOwnedIntentId, PmOwnedQuoteAdmission,
    PmOwnedQuoteIntent, PmOwnedQuoteSlotKey, PmOwnedSubmitApply, PmOwnedSubmitResult,
    PmPrivateQuoteRequest, PmPrivateReadiness, PmPrivateReadinessReason, PmPrivateStateError,
    PmReconciliationApply, PmReconciliationFillReduction, PmReconciliationReductions,
    PmRiskDecision, PmRiskDependency, PmRiskHaltScope, PmRiskReason,
};
use reap_pm_strategy::PmValidatedQuoteCandidate;
use reap_polymarket_adapter::{
    PmFakeCancelResult, PmFakeCancelScript, PmFakePlaceResult, PmFakePlaceScript,
    PmFixtureInstrumentScope, PmFixturePrivateDelivery, PmPrivateLifecycleObservation,
};
use thiserror::Error;

use super::authority::{approve_pm_cancel, approve_pm_quote};
use super::effect_queue::{
    PmFakeEffectMetrics, PmFakeEffectPermit, PmFakeEffectQueue, PmFakeEffectQueueError,
    PmPreparedFakeEffect, PmPreparedFakeEffectKind,
};
use super::effects::PmDurableRecordKind;
use super::persistence::{
    PM_PENDING_PERSISTENCE_CAPACITY, PmPendingPersistence, PmPersistenceError,
    PmPersistenceFailure, PmPersistenceIntentIdentity, PmPersistenceMetrics, PmPersistencePoll,
    PmPersistenceQueue,
};
use super::private_reduction::PmPrivateReductionError;
use super::{PmAuthorityError, PmAuthorityRevisions};
use crate::fake_effect::PmFakeEffectRole;
use crate::journal::{
    PmJournalCancelReasonV1, PmJournalError, PmJournalImmediateFillsV1, PmJournalPlaceOutcomeV1,
    PmJournalPlaceRejectReasonV1, PmJournalPlaceResultV1, PmJournalRecordV1, PmJournalRecovery,
    PmJournalSafetyReasonV1, PmJournalScopeV1, PmMutationJournal,
};
use crate::private_monitor::{
    PmPrivateMonitorError, PmPrivateMonitorRuntime, PmServicedPrivateReduction,
    validate_private_batch,
};

mod terminal_safety;
#[allow(
    unused_imports,
    reason = "the typed transition is consumed by the pending product-level safety wiring"
)]
pub(crate) use terminal_safety::{PmTerminalSafetyAdmissionFailure, PmTerminalSafetyTransition};

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) struct PmQuoteMutationRequest {
    candidate: PmValidatedQuoteCandidate,
    reservation: PmExactReservation,
    salt: PmOrderSalt,
    timestamp_ms: u64,
    approved_at_monotonic_ns: u64,
    expires_at_monotonic_ns: u64,
    reference_observed_monotonic_ns: u64,
    book_observed_monotonic_ns: u64,
}

impl PmQuoteMutationRequest {
    #[allow(clippy::too_many_arguments)]
    pub(crate) const fn new(
        candidate: PmValidatedQuoteCandidate,
        reservation: PmExactReservation,
        salt: PmOrderSalt,
        timestamp_ms: u64,
        approved_at_monotonic_ns: u64,
        expires_at_monotonic_ns: u64,
        reference_observed_monotonic_ns: u64,
        book_observed_monotonic_ns: u64,
    ) -> Self {
        Self {
            candidate,
            reservation,
            salt,
            timestamp_ms,
            approved_at_monotonic_ns,
            expires_at_monotonic_ns,
            reference_observed_monotonic_ns,
            book_observed_monotonic_ns,
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) struct PmCancelMutationRequest {
    client_order: PmClientOrderKey,
    reason: PmJournalCancelReasonV1,
    salt: PmOrderSalt,
    timestamp_ms: u64,
    approved_at_monotonic_ns: u64,
    expires_at_monotonic_ns: u64,
}

impl PmCancelMutationRequest {
    pub(crate) const fn new(
        client_order: PmClientOrderKey,
        reason: PmJournalCancelReasonV1,
        salt: PmOrderSalt,
        timestamp_ms: u64,
        approved_at_monotonic_ns: u64,
        expires_at_monotonic_ns: u64,
    ) -> Self {
        Self {
            client_order,
            reason,
            salt,
            timestamp_ms,
            approved_at_monotonic_ns,
            expires_at_monotonic_ns,
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum PmQuoteMutationAdmission {
    JournalPending {
        intent_id: u64,
        client_order: PmClientOrderKey,
    },
    Duplicate {
        client_order: PmClientOrderKey,
    },
    CancelBeforeReplace {
        current: PmClientOrderKey,
        venue_order: PmVenueOrderKey,
    },
    ReplacementBlocked {
        current: PmClientOrderKey,
    },
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum PmCancelMutationAdmission {
    JournalPending {
        client_order: PmClientOrderKey,
        venue_order: PmVenueOrderKey,
    },
    AlreadyPending {
        client_order: PmClientOrderKey,
        venue_order: PmVenueOrderKey,
    },
    AlreadyTerminal,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum PmPersistenceService {
    Empty,
    Pending,
    PreparedQuote {
        identity: PmPersistenceIntentIdentity,
    },
    PreparedCancel {
        identity: PmPersistenceIntentIdentity,
    },
    QuoteInvalidated {
        identity: PmPersistenceIntentIdentity,
    },
    FactAcknowledged {
        sequence: u64,
    },
    IntentFailed {
        identity: PmPersistenceIntentIdentity,
    },
    FactFailed,
}

/// Exact copied projection of one fact admitted to the mutation journal.
///
/// This is observation-only and carries no receipt or execution authority.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) struct PmDurableConsequence {
    kind: PmDurableRecordKind,
    client_order: Option<PmClientOrderKey>,
    correlation: u64,
}

impl PmDurableConsequence {
    pub(crate) const fn kind(self) -> PmDurableRecordKind {
        self.kind
    }

    pub(crate) const fn client_order(self) -> Option<PmClientOrderKey> {
        self.client_order
    }

    pub(crate) const fn correlation(self) -> u64 {
        self.correlation
    }
}

/// Move-only exact fake-cancel completion awaiting critical-lane reduction.
///
/// The adapter result alone cannot reconstruct the canonical cancel intent;
/// this carrier keeps that locally owned proof paired with the result until
/// the scheduler returns both to the sole mutation owner.
#[derive(Debug)]
pub(crate) struct PmPendingFakeCancelResult {
    intent: PmOwnedCancelIntent,
    result: PmFakeCancelResult,
}

impl PmPendingFakeCancelResult {
    pub(crate) const fn client_order(&self) -> PmClientOrderKey {
        self.intent.client_order()
    }

    pub(crate) const fn venue_order(&self) -> PmVenueOrderKey {
        self.intent.venue_order()
    }

    fn into_parts(self) -> (PmOwnedCancelIntent, PmFakeCancelResult) {
        (self.intent, self.result)
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub struct PmMutationCounters {
    quote_attempts: u64,
    quote_intents: u64,
    quote_duplicates: u64,
    cancel_before_replace: u64,
    cancel_attempts: u64,
    cancel_intents: u64,
    prepared_quotes: u64,
    prepared_cancels: u64,
    durable_failures: u64,
    preparation_failures: u64,
    fact_records: u64,
    place_results: u64,
    cancel_results: u64,
    unique_fills: u64,
    duplicate_fills: u64,
}

impl PmMutationCounters {
    pub const fn quote_attempts(self) -> u64 {
        self.quote_attempts
    }

    pub const fn quote_intents(self) -> u64 {
        self.quote_intents
    }

    pub const fn quote_duplicates(self) -> u64 {
        self.quote_duplicates
    }

    pub const fn cancel_before_replace(self) -> u64 {
        self.cancel_before_replace
    }

    pub const fn cancel_attempts(self) -> u64 {
        self.cancel_attempts
    }

    pub const fn cancel_intents(self) -> u64 {
        self.cancel_intents
    }

    pub const fn prepared_quotes(self) -> u64 {
        self.prepared_quotes
    }

    pub const fn prepared_cancels(self) -> u64 {
        self.prepared_cancels
    }

    pub const fn durable_failures(self) -> u64 {
        self.durable_failures
    }

    pub const fn preparation_failures(self) -> u64 {
        self.preparation_failures
    }

    pub const fn fact_records(self) -> u64 {
        self.fact_records
    }

    pub const fn place_results(self) -> u64 {
        self.place_results
    }

    pub const fn cancel_results(self) -> u64 {
        self.cancel_results
    }

    pub const fn unique_fills(self) -> u64 {
        self.unique_fills
    }

    pub const fn duplicate_fills(self) -> u64 {
        self.duplicate_fills
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum PmMutationHalt {
    RecoveryReconciliationRequired,
    LiveSafetyHalt(PmJournalSafetyReasonV1),
    RecoveredSafetyHalt(PmJournalSafetyReasonV1),
    UnjournalableSafetyHalt(PmJournalSafetyReasonV1),
    JournalAdmissionFailed,
    DurableAcknowledgementFailed,
    PreparationFailed,
    PersistenceSaturated,
    PersistenceAgeExceeded,
    PersistenceClockRegression,
    FakeEffectSaturated,
    DurableConsequenceSaturated,
    InternalInvariant,
}

/// The sole Phase-5 owner of local PM mutation authority.
///
/// Public/private observation reducers remain focused subreducers inside
/// `PmPrivateMonitorRuntime`; this owner is the only object that can join
/// readiness/risk, local reservation, the PM journal, and fake execution.
pub(crate) struct PmMutationOwner {
    scope: PmJournalScopeV1,
    instrument_scope: PmFixtureInstrumentScope,
    instrument_id: PmInstrumentId,
    private: PmPrivateMonitorRuntime,
    fake: PmFakeEffectRole,
    journal: PmMutationJournal,
    persistence: PmPersistenceQueue,
    effects: PmFakeEffectQueue,
    durable_consequences: VecDeque<PmDurableConsequence>,
    reconciliation_reductions: PmReconciliationReductions,
    next_intent_id: u64,
    current_revisions: Option<PmAuthorityRevisions>,
    halt: Option<PmMutationHalt>,
    counters: PmMutationCounters,
}

impl PmMutationOwner {
    pub(crate) async fn start(
        config: &PmConnectivityConfig,
        mut private: PmPrivateMonitorRuntime,
        fake: PmFakeEffectRole,
        journal_path: PathBuf,
    ) -> Result<(Self, PmJournalRecovery), PmMutationError> {
        let scope = PmJournalScopeV1::from_config(config)?;
        let instrument_scope = PmFixtureInstrumentScope::from_metadata(
            config.account().instrument(),
            config.account().expected_metadata(),
        )?;
        let instrument_id = config.account().instrument_id();
        if private.account_scope() != scope.account_scope()
            || private.instrument() != config.account().instrument()
            || fake.account_scope() != scope.account_scope()
            || fake.instrument() != config.account().instrument()
            || fake.instrument_id() != instrument_id
        {
            return Err(PmMutationError::CompositionScopeMismatch);
        }
        let (journal, recovery) = PmMutationJournal::start(journal_path, scope.clone()).await?;
        if let Err(error) = super::mutation_recovery::recover_private_owner(&mut private, &recovery)
        {
            let _ = journal.shutdown().await;
            return Err(error);
        }
        let next_intent_id = recovery
            .last_intent_id()
            .max(recovery.compacted_intent_id())
            .checked_add(1)
            .ok_or(PmMutationError::IntentIdentityExhausted)?;
        let halt = if let Some(reason) = recovery.safety_reason() {
            Some(PmMutationHalt::RecoveredSafetyHalt(reason))
        } else {
            recovery
                .requires_reconciliation()
                .then_some(PmMutationHalt::RecoveryReconciliationRequired)
        };
        Ok((
            Self {
                scope,
                instrument_scope,
                instrument_id,
                private,
                fake,
                journal,
                persistence: PmPersistenceQueue::new(),
                effects: PmFakeEffectQueue::new()?,
                durable_consequences: VecDeque::with_capacity(PM_PENDING_PERSISTENCE_CAPACITY),
                reconciliation_reductions: PmReconciliationReductions::new(),
                next_intent_id,
                current_revisions: None,
                halt,
                counters: PmMutationCounters::default(),
            },
            recovery,
        ))
    }

    pub(crate) fn update_revisions(&mut self, revisions: PmAuthorityRevisions) {
        self.current_revisions = Some(revisions);
    }

    pub(crate) fn invalidate_revisions(&mut self) {
        self.current_revisions = None;
    }

    /// Durably abandons one prepared but never-dispatched quote after an
    /// authority-bearing dependency changed.
    ///
    /// The fake effect remains retained until the local rejection record is
    /// admitted, so a journal failure cannot lose executable authority.
    pub(crate) fn invalidate_prepared_quote(
        &mut self,
        client_order: PmClientOrderKey,
        monotonic_service_ns: u64,
    ) -> Result<bool, PmMutationError> {
        if !self.effects.contains_prepared_quote(client_order) {
            return Ok(false);
        }
        self.ensure_fact_capacity(1)?;
        self.reject_durable_never_dispatched_quote(client_order, monotonic_service_ns)?;
        self.effects
            .invalidate_prepared_quote(client_order)
            .map_err(|error| {
                self.halt = Some(PmMutationHalt::InternalInvariant);
                PmMutationError::EffectQueue(error)
            })?;
        Ok(true)
    }

    pub(crate) fn begin_quote(
        &mut self,
        request: PmQuoteMutationRequest,
    ) -> Result<PmQuoteMutationAdmission, PmMutationError> {
        self.ensure_quote_available()?;
        self.counters.quote_attempts = self.counters.quote_attempts.saturating_add(1);
        Self::validate_persistence_time(request.approved_at_monotonic_ns)?;
        self.persistence
            .ensure_capacity(1)
            .map_err(|error| self.persistence_failure(error))?;

        let candidate = request.candidate;
        let quote_request = PmPrivateQuoteRequest::new(
            request.approved_at_monotonic_ns,
            candidate.side(),
            candidate.price(),
            candidate.quantity(),
            request.reservation,
        );
        let ready = match self.private.quote_readiness(quote_request) {
            PmPrivateReadiness::Ready(ready) => ready,
            PmPrivateReadiness::Blocked(reason) => {
                return Err(PmMutationError::PrivateNotReady(reason));
            }
        };
        let risk = self.private.evaluate_risk_candidate(
            quote_request,
            PmRiskDependency::available(request.reference_observed_monotonic_ns),
            PmRiskDependency::available(request.book_observed_monotonic_ns),
        )?;
        if let PmRiskDecision::Rejected { reason, halt } = risk {
            return Err(PmMutationError::RiskRejected { reason, halt });
        }

        let intent = PmOwnedIntentId::new(self.next_intent_id)?;
        let next_intent_id = self
            .next_intent_id
            .checked_add(1)
            .ok_or(PmMutationError::IntentIdentityExhausted)?;
        let client_order = self.scope.client_order_for_intent(intent.value())?;
        let approved = approve_pm_quote(
            self.scope.account_scope(),
            self.instrument_id,
            intent,
            client_order,
            candidate,
            request.reservation,
            self.fake.place_profile(),
            request.salt,
            request.timestamp_ms,
            self.current_revisions
                .ok_or(PmMutationError::RevisionsUnavailable)?,
            request.approved_at_monotonic_ns,
            request.expires_at_monotonic_ns,
            ready,
            risk,
        )?;
        let owned_intent = PmOwnedQuoteIntent::new(
            intent,
            PmOwnedQuoteSlotKey::new(
                self.scope.account_scope(),
                candidate.instrument(),
                candidate.side(),
            ),
            client_order,
            candidate.price(),
            candidate.quantity(),
            request.reservation,
        )?;
        let effect_permit = self.reserve_effect_capacity()?;
        let admission = match self.private.admit_owned_quote(owned_intent) {
            Ok(admission) => admission,
            Err(error) => {
                self.effects.release_before_journal(effect_permit)?;
                return Err(error.into());
            }
        };
        let reserved = match admission {
            PmOwnedQuoteAdmission::Admitted(_) => match approved.reserve(owned_intent, admission) {
                Ok(reserved) => reserved,
                Err(error) => {
                    self.effects.release_before_journal(effect_permit)?;
                    self.reject_never_dispatched_quote(client_order);
                    self.halt = Some(PmMutationHalt::InternalInvariant);
                    return Err(error.into());
                }
            },
            PmOwnedQuoteAdmission::DuplicateIntent(client)
            | PmOwnedQuoteAdmission::DuplicateQuote(client) => {
                self.effects.release_before_journal(effect_permit)?;
                self.counters.quote_duplicates = self.counters.quote_duplicates.saturating_add(1);
                return Ok(PmQuoteMutationAdmission::Duplicate {
                    client_order: client,
                });
            }
            PmOwnedQuoteAdmission::CancelBeforeReplace(cancel) => {
                self.effects.release_before_journal(effect_permit)?;
                self.counters.cancel_before_replace =
                    self.counters.cancel_before_replace.saturating_add(1);
                return Ok(PmQuoteMutationAdmission::CancelBeforeReplace {
                    current: cancel.client_order(),
                    venue_order: cancel.venue_order(),
                });
            }
            PmOwnedQuoteAdmission::ReplacementBlocked { current, .. } => {
                self.effects.release_before_journal(effect_permit)?;
                return Ok(PmQuoteMutationAdmission::ReplacementBlocked { current });
            }
        };

        let journal_intent = reserved.journal_intent();
        let pending = match self.journal.try_quote_intent(journal_intent) {
            Ok(pending) => pending,
            Err(error) => {
                self.effects.release_before_journal(effect_permit)?;
                self.reject_never_dispatched_quote(client_order);
                self.halt = Some(PmMutationHalt::JournalAdmissionFailed);
                return Err(error.into());
            }
        };
        if let Err(error) = self.persistence.push(PmPendingPersistence::QuoteIntent {
            reserved,
            effect_permit,
            receipt: pending,
            enqueued_monotonic_ns: request.approved_at_monotonic_ns,
        }) {
            self.halt = Some(PmMutationHalt::InternalInvariant);
            return Err(error.into());
        }
        self.next_intent_id = next_intent_id;
        self.counters.quote_intents = self.counters.quote_intents.saturating_add(1);
        Ok(PmQuoteMutationAdmission::JournalPending {
            intent_id: intent.value(),
            client_order,
        })
    }

    pub(crate) fn begin_cancel(
        &mut self,
        request: PmCancelMutationRequest,
    ) -> Result<PmCancelMutationAdmission, PmMutationError> {
        self.ensure_cancel_available()?;
        self.counters.cancel_attempts = self.counters.cancel_attempts.saturating_add(1);
        Self::validate_persistence_time(request.approved_at_monotonic_ns)?;
        self.persistence
            .ensure_capacity(1)
            .map_err(|error| self.persistence_failure(error))?;
        let order = self
            .private
            .owned_order(request.client_order)
            .ok_or(PmMutationError::UnknownOwnedOrder)?;
        let approved = approve_pm_cancel(
            self.scope.account_scope(),
            self.instrument_id,
            order,
            self.fake.place_profile(),
            self.fake.cancel_purpose(),
            request.salt,
            request.timestamp_ms,
            request.approved_at_monotonic_ns,
            request.expires_at_monotonic_ns,
            request.reason,
        )?;
        let effect_permit = self.reserve_effect_capacity()?;
        let cancel_request = match self.private.request_owned_cancel(request.client_order) {
            Ok(cancel_request) => cancel_request,
            Err(error) => {
                self.effects.release_before_journal(effect_permit)?;
                return Err(error.into());
            }
        };
        let owned_intent = match cancel_request {
            reap_pm_state::PmOwnedCancelRequestApply::Issued(intent) => intent,
            reap_pm_state::PmOwnedCancelRequestApply::Duplicate(intent) => {
                self.effects.release_before_journal(effect_permit)?;
                return Ok(PmCancelMutationAdmission::AlreadyPending {
                    client_order: intent.client_order(),
                    venue_order: intent.venue_order(),
                });
            }
            reap_pm_state::PmOwnedCancelRequestApply::AlreadyTerminal => {
                self.effects.release_before_journal(effect_permit)?;
                return Ok(PmCancelMutationAdmission::AlreadyTerminal);
            }
        };
        let reserved = match approved.reserve(cancel_request) {
            Ok(reserved) => reserved,
            Err(error) => {
                self.effects.release_before_journal(effect_permit)?;
                let _ = self.private.apply_owned_cancel_result(
                    owned_intent,
                    reap_pm_state::PmOwnedCancelOutcome::Rejected,
                );
                self.halt = Some(PmMutationHalt::InternalInvariant);
                return Err(error.into());
            }
        };
        let pending = match self.journal.try_cancel_intent(reserved.journal_intent()) {
            Ok(pending) => pending,
            Err(error) => {
                self.effects.release_before_journal(effect_permit)?;
                let _ = self.private.apply_owned_cancel_result(
                    owned_intent,
                    reap_pm_state::PmOwnedCancelOutcome::Rejected,
                );
                self.halt = Some(PmMutationHalt::JournalAdmissionFailed);
                return Err(error.into());
            }
        };
        if let Err(error) = self.persistence.push(PmPendingPersistence::CancelIntent {
            reserved,
            owned_intent,
            effect_permit,
            receipt: pending,
            enqueued_monotonic_ns: request.approved_at_monotonic_ns,
        }) {
            self.halt = Some(PmMutationHalt::InternalInvariant);
            return Err(error.into());
        }
        self.counters.cancel_intents = self.counters.cancel_intents.saturating_add(1);
        Ok(PmCancelMutationAdmission::JournalPending {
            client_order: owned_intent.client_order(),
            venue_order: owned_intent.venue_order(),
        })
    }

    #[cfg(test)]
    pub(crate) fn service_persistence(
        &mut self,
        monotonic_now_ns: u64,
    ) -> Result<PmPersistenceService, PmMutationError> {
        let poll = self.poll_persistence(monotonic_now_ns)?;
        self.reduce_persistence_poll(poll, monotonic_now_ns)
    }

    /// Polls the bounded durable-writer edge without consuming the exact
    /// acknowledgement authority.
    ///
    /// The complete product scheduler carries non-empty/non-pending values
    /// through its persistence lane before returning them to
    /// [`Self::reduce_persistence_poll`]. This split keeps one mutation owner
    /// while making cross-lane ordering explicit and replayable.
    pub(crate) fn poll_persistence(
        &mut self,
        monotonic_now_ns: u64,
    ) -> Result<PmPersistencePoll, PmMutationError> {
        match self.persistence.poll_one(monotonic_now_ns) {
            Ok(poll) => Ok(poll),
            Err(PmPersistenceError::ClockRegression) => {
                self.halt = Some(PmMutationHalt::PersistenceClockRegression);
                Err(PmPersistenceError::ClockRegression.into())
            }
            Err(error) => {
                self.halt = Some(PmMutationHalt::PersistenceSaturated);
                Err(error.into())
            }
        }
    }

    /// Reduces one scheduler-serviced durable result exactly once.
    pub(crate) fn reduce_persistence_poll(
        &mut self,
        poll: PmPersistencePoll,
        monotonic_service_ns: u64,
    ) -> Result<PmPersistenceService, PmMutationError> {
        Self::validate_persistence_time(monotonic_service_ns)?;
        match poll {
            PmPersistencePoll::Empty => Ok(PmPersistenceService::Empty),
            PmPersistencePoll::Pending => Ok(PmPersistenceService::Pending),
            PmPersistencePoll::QuoteAcknowledged {
                reserved,
                effect_permit,
                acknowledgement,
            } => {
                let identity = PmPersistenceIntentIdentity::Quote {
                    intent: reserved.intent(),
                    client_order: reserved.client_order(),
                };
                let Some(revisions) = self.current_revisions else {
                    return self.invalidate_durable_quote(
                        identity,
                        effect_permit,
                        monotonic_service_ns,
                    );
                };
                match self.fake.prepare_quote(
                    reserved,
                    self.instrument_scope,
                    revisions,
                    monotonic_service_ns,
                    acknowledgement,
                ) {
                    Ok(authority) => {
                        self.effects.commit(
                            effect_permit,
                            PmPreparedFakeEffect::Quote { authority },
                            monotonic_service_ns,
                        )?;
                        self.counters.prepared_quotes =
                            self.counters.prepared_quotes.saturating_add(1);
                        Ok(PmPersistenceService::PreparedQuote { identity })
                    }
                    Err(
                        error @ (PmAuthorityError::RevisionChanged
                        | PmAuthorityError::ApprovalExpired),
                    ) => {
                        let _expected_invalidation = error;
                        self.invalidate_durable_quote(identity, effect_permit, monotonic_service_ns)
                    }
                    Err(error) => {
                        self.retain_failed_effect(
                            effect_permit,
                            PmMutationHalt::PreparationFailed,
                        )?;
                        self.counters.preparation_failures =
                            self.counters.preparation_failures.saturating_add(1);
                        Err(error.into())
                    }
                }
            }
            PmPersistencePoll::CancelAcknowledged {
                reserved,
                owned_intent,
                effect_permit,
                acknowledgement,
            } => {
                let identity = PmPersistenceIntentIdentity::Cancel {
                    client_order: owned_intent.client_order(),
                    venue_order: owned_intent.venue_order(),
                };
                match self.fake.prepare_cancel(
                    reserved,
                    self.instrument_scope,
                    monotonic_service_ns,
                    acknowledgement,
                ) {
                    Ok(authority) => {
                        self.effects.commit(
                            effect_permit,
                            PmPreparedFakeEffect::Cancel {
                                authority,
                                owned_intent,
                            },
                            monotonic_service_ns,
                        )?;
                        self.counters.prepared_cancels =
                            self.counters.prepared_cancels.saturating_add(1);
                        Ok(PmPersistenceService::PreparedCancel { identity })
                    }
                    Err(error) => {
                        self.retain_failed_effect(
                            effect_permit,
                            PmMutationHalt::PreparationFailed,
                        )?;
                        self.counters.preparation_failures =
                            self.counters.preparation_failures.saturating_add(1);
                        Err(error.into())
                    }
                }
            }
            PmPersistencePoll::FactAcknowledged(acknowledged) => {
                let sequence = acknowledged.consume();
                Ok(PmPersistenceService::FactAcknowledged { sequence })
            }
            PmPersistencePoll::IntentFailed {
                identity,
                effect_permit,
                reason,
            } => {
                let halt = Self::halt_for_persistence_failure(&reason);
                self.retain_failed_effect(effect_permit, halt)?;
                self.record_persistence_failure(&reason);
                Ok(PmPersistenceService::IntentFailed { identity })
            }
            PmPersistencePoll::FactFailed(reason) => {
                self.halt = Some(Self::halt_for_persistence_failure(&reason));
                self.record_persistence_failure(&reason);
                Ok(PmPersistenceService::FactFailed)
            }
        }
    }

    pub(crate) fn next_effect_kind(&self) -> Option<PmPreparedFakeEffectKind> {
        self.effects.next_kind()
    }

    #[cfg(test)]
    pub(crate) fn execute_next_quote(
        &mut self,
        script: PmFakePlaceScript,
        monotonic_service_ns: u64,
    ) -> Result<(), PmMutationError> {
        let result = self.execute_next_quote_to_result(script, monotonic_service_ns)?;
        self.reduce_serviced_fake_place(result, monotonic_service_ns)
    }

    /// Consumes one prepared quote only at the fixture edge and returns its
    /// exact result for critical-lane ordering.
    pub(crate) fn execute_next_quote_to_result(
        &mut self,
        script: PmFakePlaceScript,
        monotonic_service_ns: u64,
    ) -> Result<PmFakePlaceResult, PmMutationError> {
        if self.next_effect_kind() != Some(PmPreparedFakeEffectKind::Quote) {
            return Err(PmMutationError::EffectKindMismatch);
        }
        let effect = match self
            .effects
            .pop_quote_at(monotonic_service_ns, self.current_revisions)
        {
            Ok(effect) => effect,
            Err(error) => {
                self.halt = Some(
                    if error == PmFakeEffectQueueError::QuoteAuthorityInvalidated {
                        PmMutationHalt::PreparationFailed
                    } else {
                        PmMutationHalt::FakeEffectSaturated
                    },
                );
                return Err(error.into());
            }
        };
        let Some(PmPreparedFakeEffect::Quote { authority }) = effect else {
            return Err(PmMutationError::EffectKindMismatch);
        };
        Ok(self.fake.execute_quote(authority, script)?)
    }

    /// Applies one scheduler-serviced fake place result to canonical state and
    /// its journal.
    pub(crate) fn reduce_serviced_fake_place(
        &mut self,
        result: PmFakePlaceResult,
        monotonic_service_ns: u64,
    ) -> Result<(), PmMutationError> {
        super::reduction::reduce_fake_place(self, result, monotonic_service_ns)
    }

    #[cfg(test)]
    pub(crate) fn execute_next_cancel(
        &mut self,
        script: PmFakeCancelScript,
        monotonic_service_ns: u64,
    ) -> Result<(), PmMutationError> {
        let result = self.execute_next_cancel_to_result(script, monotonic_service_ns)?;
        self.reduce_serviced_fake_cancel(result, monotonic_service_ns)
    }

    /// Consumes one prepared owned cancel only at the fixture edge and returns
    /// the paired result/ownership proof for critical-lane ordering.
    pub(crate) fn execute_next_cancel_to_result(
        &mut self,
        script: PmFakeCancelScript,
        monotonic_service_ns: u64,
    ) -> Result<PmPendingFakeCancelResult, PmMutationError> {
        if self.next_effect_kind() != Some(PmPreparedFakeEffectKind::Cancel) {
            return Err(PmMutationError::EffectKindMismatch);
        }
        let effect = match self.effects.pop_at(monotonic_service_ns) {
            Ok(effect) => effect,
            Err(error) => {
                self.halt = Some(PmMutationHalt::FakeEffectSaturated);
                return Err(error.into());
            }
        };
        let Some(PmPreparedFakeEffect::Cancel {
            authority,
            owned_intent,
        }) = effect
        else {
            return Err(PmMutationError::EffectKindMismatch);
        };
        let result = self.fake.execute_cancel(authority, script)?;
        Ok(PmPendingFakeCancelResult {
            intent: owned_intent,
            result,
        })
    }

    /// Applies one scheduler-serviced fake cancel result to canonical state
    /// and its journal.
    pub(crate) fn reduce_serviced_fake_cancel(
        &mut self,
        pending: PmPendingFakeCancelResult,
        monotonic_service_ns: u64,
    ) -> Result<(), PmMutationError> {
        let (intent, result) = pending.into_parts();
        super::reduction::reduce_fake_cancel(self, intent, result, monotonic_service_ns)
    }

    /// Applies one scheduled private lifecycle observation and records its
    /// exact owned consequence before another fake effect may run.
    #[allow(clippy::too_many_arguments)]
    pub(crate) fn reduce_serviced_private_observation(
        &mut self,
        source: PmProductSource,
        connection: PmConnectionId,
        clock: EventClock,
        ordering: EventOrdering,
        observation: PmPrivateLifecycleObservation,
    ) -> Result<PmServicedPrivateReduction, PmMutationError> {
        super::private_reduction::reduce_private_observation(
            self,
            source,
            connection,
            clock,
            ordering,
            observation,
        )
    }

    /// Opens one exact role-bound fixture batch at scheduler service time,
    /// then journals every owned consequence through this sole mutation
    /// owner. The whole possible fact count is preflighted before the first
    /// canonical reduction.
    pub(crate) fn reduce_serviced_private_fixture(
        &mut self,
        delivery: PmFixturePrivateDelivery,
        monotonic_service_ns: u64,
    ) -> Result<usize, PmMutationError> {
        let envelope = self
            .private
            .open_product_private_fixture(delivery, monotonic_service_ns)?;
        validate_private_batch(envelope.payload().observations())?;
        let observation_count = envelope.payload().observations().len();
        self.ensure_fact_capacity(observation_count)?;
        let source = envelope.source();
        let connection = envelope.connection_id();
        let clock = envelope.clock();
        let ordering = envelope.ordering();
        for observation in envelope.payload().observations().iter().copied() {
            self.reduce_serviced_private_observation(
                source,
                connection,
                clock,
                ordering,
                observation,
            )?;
        }
        Ok(observation_count)
    }

    /// Applies one exact paired account-plus-fill reconciliation cut through
    /// the sole private owner and journals every newly owned fill consequence.
    pub(crate) fn reduce_serviced_reconciliation(
        &mut self,
        account: EventEnvelope<PmCompleteAccountSnapshot>,
        fills: EventEnvelope<PmCompleteFillQuery>,
    ) -> Result<PmReconciliationApply, PmMutationError> {
        let outcome = super::private_reduction::reduce_reconciliation(self, account, fills)?;
        if matches!(outcome, PmReconciliationApply::Applied { .. })
            && self.halt == Some(PmMutationHalt::RecoveryReconciliationRequired)
        {
            self.halt = None;
        }
        Ok(outcome)
    }

    pub(crate) async fn shutdown(self) -> Result<(), PmMutationError> {
        self.journal.shutdown().await?;
        Ok(())
    }

    pub(crate) const fn halt(&self) -> Option<PmMutationHalt> {
        self.halt
    }

    pub(crate) const fn counters(&self) -> PmMutationCounters {
        self.counters
    }

    pub(crate) fn persistence_metrics(&self) -> PmPersistenceMetrics {
        self.persistence.projection()
    }

    pub(crate) fn fake_effect_metrics(&self) -> PmFakeEffectMetrics {
        self.effects.projection()
    }

    pub(crate) fn reserved_capacity_bytes(&self) -> usize {
        self.persistence.reserved_capacity_bytes()
            + self.effects.reserved_capacity_bytes()
            + self.reconciliation_reductions.reserved_capacity_bytes()
            + self.durable_consequences.capacity() * std::mem::size_of::<PmDurableConsequence>()
    }

    #[cfg(test)]
    pub(crate) fn pending_persistence(&self) -> usize {
        self.persistence.len()
    }

    #[cfg(test)]
    pub(crate) fn pending_effects(&self) -> usize {
        self.effects.queued_len()
    }

    pub(crate) fn pop_durable_consequence(&mut self) -> Option<PmDurableConsequence> {
        self.durable_consequences.pop_front()
    }

    pub(crate) fn pending_durable_consequences(&self) -> usize {
        self.durable_consequences.len()
    }

    #[cfg(test)]
    pub(crate) fn retained_effect_permits(&self) -> usize {
        self.effects.retained_permits()
    }

    #[cfg(test)]
    pub(crate) fn persistence_capacity(&self) -> usize {
        PM_PENDING_PERSISTENCE_CAPACITY
    }

    pub(super) fn private_mut(&mut self) -> &mut PmPrivateMonitorRuntime {
        &mut self.private
    }

    pub(super) fn reduce_private_reconciliation(
        &mut self,
        account: EventEnvelope<PmCompleteAccountSnapshot>,
        fills: EventEnvelope<PmCompleteFillQuery>,
    ) -> Result<PmReconciliationApply, PmPrivateMonitorError> {
        self.private.reduce_serviced_reconciliation(
            account,
            fills,
            &mut self.reconciliation_reductions,
        )
    }

    pub(super) fn reconciliation_reduction_count(&self) -> usize {
        self.reconciliation_reductions.len()
    }

    pub(super) fn reconciliation_reduction(
        &self,
        index: usize,
    ) -> Option<PmReconciliationFillReduction> {
        self.reconciliation_reductions.get(index)
    }

    pub(super) const fn instrument_id(&self) -> PmInstrumentId {
        self.instrument_id
    }

    pub(super) const fn account_scope(&self) -> reap_pm_core::PmAccountScope {
        self.scope.account_scope()
    }

    pub(super) fn ensure_fact_capacity(
        &mut self,
        additional: usize,
    ) -> Result<(), PmMutationError> {
        if self.durable_consequences.len().saturating_add(additional)
            > PM_PENDING_PERSISTENCE_CAPACITY
        {
            self.halt = Some(PmMutationHalt::DurableConsequenceSaturated);
            return Err(PmMutationError::DurableConsequenceSaturated);
        }
        self.persistence
            .ensure_capacity(additional)
            .map_err(|error| self.persistence_failure(error))
    }

    pub(super) fn record_fact(
        &mut self,
        record: PmJournalRecordV1,
        monotonic_now_ns: u64,
    ) -> Result<(), PmMutationError> {
        Self::validate_persistence_time(monotonic_now_ns)?;
        self.ensure_fact_capacity(1)?;
        let (kind, client_order) = durable_consequence_projection(&record)?;
        let pending = match self.journal.try_record(record) {
            Ok(pending) => pending,
            Err(error) => {
                self.halt = Some(PmMutationHalt::JournalAdmissionFailed);
                return Err(error.into());
            }
        };
        if let Err(error) = self.persistence.push(PmPendingPersistence::Fact {
            receipt: pending,
            enqueued_monotonic_ns: monotonic_now_ns,
        }) {
            self.halt = Some(PmMutationHalt::InternalInvariant);
            return Err(error.into());
        }
        self.counters.fact_records = self.counters.fact_records.saturating_add(1);
        self.durable_consequences.push_back(PmDurableConsequence {
            kind,
            client_order,
            correlation: self.counters.fact_records,
        });
        Ok(())
    }

    pub(super) fn count_place_result(&mut self) {
        self.counters.place_results = self.counters.place_results.saturating_add(1);
    }

    pub(super) fn count_cancel_result(&mut self) {
        self.counters.cancel_results = self.counters.cancel_results.saturating_add(1);
    }

    pub(super) fn count_unique_fill(&mut self) {
        self.counters.unique_fills = self.counters.unique_fills.saturating_add(1);
    }

    pub(super) fn count_duplicate_fill(&mut self) {
        self.counters.duplicate_fills = self.counters.duplicate_fills.saturating_add(1);
    }

    pub(super) fn halt_contract(&mut self) {
        self.halt = Some(PmMutationHalt::InternalInvariant);
    }

    fn ensure_quote_available(&self) -> Result<(), PmMutationError> {
        if let Some(halt) = self.halt {
            Err(PmMutationError::Halted(halt))
        } else if self.effects.quote_suppressed() {
            Err(PmMutationError::Halted(PmMutationHalt::FakeEffectSaturated))
        } else {
            Ok(())
        }
    }

    fn ensure_cancel_available(&self) -> Result<(), PmMutationError> {
        match self.halt {
            None
            | Some(
                PmMutationHalt::RecoveryReconciliationRequired
                | PmMutationHalt::LiveSafetyHalt(_)
                | PmMutationHalt::RecoveredSafetyHalt(_)
                | PmMutationHalt::UnjournalableSafetyHalt(_)
                | PmMutationHalt::PreparationFailed
                | PmMutationHalt::PersistenceSaturated
                | PmMutationHalt::FakeEffectSaturated,
            ) => Ok(()),
            Some(halt) => Err(PmMutationError::Halted(halt)),
        }
    }

    fn invalidate_durable_quote(
        &mut self,
        identity: PmPersistenceIntentIdentity,
        effect_permit: PmFakeEffectPermit,
        monotonic_service_ns: u64,
    ) -> Result<PmPersistenceService, PmMutationError> {
        let PmPersistenceIntentIdentity::Quote { client_order, .. } = identity else {
            self.halt = Some(PmMutationHalt::InternalInvariant);
            return Err(PmMutationError::InvalidPersistenceIdentity);
        };
        self.ensure_fact_capacity(1)?;
        self.reject_durable_never_dispatched_quote(client_order, monotonic_service_ns)?;
        self.effects
            .invalidate_after_durability(effect_permit)
            .map_err(|error| {
                self.halt = Some(PmMutationHalt::InternalInvariant);
                PmMutationError::EffectQueue(error)
            })?;
        Ok(PmPersistenceService::QuoteInvalidated { identity })
    }

    fn reject_durable_never_dispatched_quote(
        &mut self,
        client_order: PmClientOrderKey,
        monotonic_service_ns: u64,
    ) -> Result<(), PmMutationError> {
        let apply = self
            .private
            .apply_owned_submit_result(client_order, PmOwnedSubmitResult::Rejected)?;
        if apply != PmOwnedSubmitApply::Rejected {
            self.halt = Some(PmMutationHalt::InternalInvariant);
            return Err(PmMutationError::InvalidLocalInvalidation);
        }
        self.record_fact(
            PmJournalRecordV1::PlaceResult(PmJournalPlaceResultV1 {
                client_order,
                outcome: PmJournalPlaceOutcomeV1::Rejected,
                reject_reason: Some(
                    PmJournalPlaceRejectReasonV1::AuthorityInvalidatedBeforeDispatch,
                ),
                venue_order: None,
                immediate_fills: PmJournalImmediateFillsV1::empty(),
            }),
            monotonic_service_ns,
        )?;
        self.private.compact_proven_owned_terminal(client_order)?;
        Ok(())
    }

    fn reject_never_dispatched_quote(&mut self, client_order: PmClientOrderKey) {
        if self
            .private
            .apply_owned_submit_result(client_order, PmOwnedSubmitResult::Rejected)
            .is_ok()
        {
            let _ = self.private.compact_proven_owned_terminal(client_order);
        }
    }

    fn retain_failed_effect(
        &mut self,
        permit: PmFakeEffectPermit,
        halt: PmMutationHalt,
    ) -> Result<(), PmMutationError> {
        self.effects.retain_after_durable_failure(permit)?;
        self.halt = Some(halt);
        Ok(())
    }

    fn record_persistence_failure(&mut self, reason: &PmPersistenceFailure) {
        if matches!(
            reason,
            PmPersistenceFailure::Durability(_) | PmPersistenceFailure::Closed
        ) {
            self.counters.durable_failures = self.counters.durable_failures.saturating_add(1);
        }
    }

    const fn halt_for_persistence_failure(reason: &PmPersistenceFailure) -> PmMutationHalt {
        match reason {
            PmPersistenceFailure::Durability(_) | PmPersistenceFailure::Closed => {
                PmMutationHalt::DurableAcknowledgementFailed
            }
            PmPersistenceFailure::AgeExceeded => PmMutationHalt::PersistenceAgeExceeded,
        }
    }

    fn reserve_effect_capacity(&mut self) -> Result<PmFakeEffectPermit, PmMutationError> {
        self.effects.try_reserve().map_err(|error| {
            self.halt = Some(PmMutationHalt::FakeEffectSaturated);
            PmMutationError::EffectQueue(error)
        })
    }

    const fn validate_persistence_time(monotonic_ns: u64) -> Result<(), PmMutationError> {
        if monotonic_ns == 0 {
            Err(PmMutationError::Persistence(
                PmPersistenceError::InvalidMonotonicTime,
            ))
        } else {
            Ok(())
        }
    }

    fn persistence_failure(&mut self, error: PmPersistenceError) -> PmMutationError {
        self.halt = Some(PmMutationHalt::PersistenceSaturated);
        PmMutationError::Persistence(error)
    }
}

fn durable_consequence_projection(
    record: &PmJournalRecordV1,
) -> Result<(PmDurableRecordKind, Option<PmClientOrderKey>), PmMutationError> {
    match record {
        PmJournalRecordV1::QuoteIntent(intent) => {
            Ok((PmDurableRecordKind::QuoteIntent, Some(intent.client_order)))
        }
        PmJournalRecordV1::PlaceResult(result) => {
            Ok((PmDurableRecordKind::PlaceResult, Some(result.client_order)))
        }
        PmJournalRecordV1::CancelIntent(intent) => {
            Ok((PmDurableRecordKind::CancelIntent, Some(intent.client_order)))
        }
        PmJournalRecordV1::CancelResult(result) => {
            Ok((PmDurableRecordKind::CancelResult, Some(result.client_order)))
        }
        PmJournalRecordV1::FillApplied(applied) => Ok((
            PmDurableRecordKind::FillApplied,
            Some(applied.fill.client_order),
        )),
        PmJournalRecordV1::OrderTerminal(terminal) => Ok((
            PmDurableRecordKind::OrderTerminal,
            Some(terminal.client_order),
        )),
        PmJournalRecordV1::SafetyHalt(_) => Ok((PmDurableRecordKind::SafetyHalt, None)),
        PmJournalRecordV1::FillWatermarkAdvanced(_) => {
            Ok((PmDurableRecordKind::FillWatermarkAdvanced, None))
        }
        PmJournalRecordV1::Header(_) => Err(PmMutationError::InvalidDurableConsequence),
    }
}

#[derive(Debug, Error)]
pub(crate) enum PmMutationError {
    #[error("PM mutation composition scope is inconsistent")]
    CompositionScopeMismatch,
    #[error("PM mutation intent identity is exhausted")]
    IntentIdentityExhausted,
    #[error("PM mutation revisions are not available")]
    RevisionsUnavailable,
    #[error("PM private state is not quote-ready: {0:?}")]
    PrivateNotReady(PmPrivateReadinessReason),
    #[error("PM exact risk gate rejected the candidate: {reason:?} ({halt:?})")]
    RiskRejected {
        reason: PmRiskReason,
        halt: PmRiskHaltScope,
    },
    #[error("PM canonical owned order is unknown")]
    UnknownOwnedOrder,
    #[error("PM fake-effect script does not match the next prepared effect")]
    EffectKindMismatch,
    #[error("PM copied durable-consequence queue is saturated")]
    DurableConsequenceSaturated,
    #[error("PM journal record has no valid product durable-consequence projection")]
    InvalidDurableConsequence,
    #[error("PM persistence identity did not name a quote")]
    InvalidPersistenceIdentity,
    #[error("PM locally invalidated quote was not pending fake dispatch")]
    InvalidLocalInvalidation,
    #[error("PM journal recovery could not reproduce canonical owned state")]
    RecoveryProjectionMismatch,
    #[error(transparent)]
    PrivateReduction(#[from] PmPrivateReductionError),
    #[error("PM mutation owner is halted: {0:?}")]
    Halted(PmMutationHalt),
    #[error(transparent)]
    Numeric(#[from] PmNumericError),
    #[error(transparent)]
    Event(#[from] PmEventError),
    #[error(transparent)]
    Metadata(#[from] reap_pm_core::PmMetadataError),
    #[error(transparent)]
    State(#[from] PmPrivateStateError),
    #[error(transparent)]
    OwnedLifecycle(#[from] reap_pm_state::PmOwnedOrderLifecycleError),
    #[error(transparent)]
    JournalSchema(#[from] crate::journal::PmJournalSchemaError),
    #[error(transparent)]
    Journal(#[from] PmJournalError),
    #[error(transparent)]
    Authority(#[from] PmAuthorityError),
    #[error(transparent)]
    EffectQueue(#[from] PmFakeEffectQueueError),
    #[error(transparent)]
    Persistence(#[from] PmPersistenceError),
    #[error(transparent)]
    Reduction(#[from] super::reduction::PmReductionError),
    #[error(transparent)]
    PrivateMonitor(#[from] PmPrivateMonitorError),
}

#[cfg(test)]
mod tests;
