use reap_pm_core::{
    ConnectionEpoch, EnvelopeError, EventOrdering, IngressSequence, PmConditionId, PmEventError,
    PmFillQueryCursor, PmSnapshotEvidence, PmVenueOrderKey, ReceivedEventClock, SnapshotRevision,
};
use reap_pm_state::{
    PmPrivateDependency, PmPrivateExternalIngressFailure, PmPrivateExternalIngressFault,
    PmPrivateExternalIngressLane,
};
use reap_polymarket_adapter::PmCompletionOccurrence;
use reap_polymarket_live_adapter::{
    PmAccountBalanceAllowance, PmCompleteOpenOrdersCut, PmCompleteTradesCut,
    PmExactOrderObservation, PmUserWsBoundFrame, PmUserWsDisconnectReason, PmUserWsObservation,
    PmUserWsRetirement,
};
use thiserror::Error;

use super::{
    PmLiveAccountInput, PmLiveOpenOrdersInput, PmLiveOrderDetailInput, PmLivePrivateInput,
    PmLiveReconciliationInput,
};
use crate::private_monitor::{PmFixtureQueryOccurrence, PmPrivateMonitorInputError};

pub(crate) trait PmLiveHttpQueryPurpose {}

#[derive(Debug)]
pub(crate) struct PmLiveOpenOrdersQuery;
impl PmLiveHttpQueryPurpose for PmLiveOpenOrdersQuery {}

#[derive(Debug)]
pub(crate) struct PmLiveOrderDetailQuery;
impl PmLiveHttpQueryPurpose for PmLiveOrderDetailQuery {}

#[derive(Debug)]
pub(crate) struct PmLiveAccountQuery;
impl PmLiveHttpQueryPurpose for PmLiveAccountQuery {}

#[derive(Debug)]
pub(crate) struct PmLiveReconciliationQuery;
impl PmLiveHttpQueryPurpose for PmLiveReconciliationQuery {}

/// Sealed connection occurrence issued directly from a typed user-WebSocket
/// open observation. It cannot be replaced with fixture evidence.
#[derive(Debug)]
pub(crate) struct PmLiveConnectionInput {
    occurrence: PmCompletionOccurrence,
}

impl PmLiveConnectionInput {
    pub(crate) fn into_occurrence(self) -> PmCompletionOccurrence {
        self.occurrence
    }
}

/// Sealed retirement occurrence and canonical unavailable classification.
#[derive(Debug)]
pub(crate) struct PmLiveRetirementInput {
    occurrence: PmCompletionOccurrence,
    fault: PmPrivateExternalIngressFault,
}

/// A failed connection attempt that never became canonically available.
/// There is no private-state disconnect to enqueue, but the issuer still
/// consumes the attempted epoch and edge clock before authorizing a newer
/// attempt.
#[derive(Debug)]
pub(crate) struct PmLivePreOpenRetirement {
    connection_epoch: ConnectionEpoch,
    received_clock: ReceivedEventClock,
    fault: PmPrivateExternalIngressFault,
}

impl PmLivePreOpenRetirement {
    #[must_use]
    pub(crate) const fn connection_epoch(&self) -> ConnectionEpoch {
        self.connection_epoch
    }

    #[must_use]
    pub(crate) const fn received_clock(&self) -> ReceivedEventClock {
        self.received_clock
    }

    #[must_use]
    pub(crate) const fn fault(&self) -> PmPrivateExternalIngressFault {
        self.fault
    }
}

#[derive(Debug)]
pub(crate) enum PmLiveRetirementOutcome {
    Active(PmLiveRetirementInput),
    PreOpen(PmLivePreOpenRetirement),
}

impl PmLiveRetirementInput {
    pub(crate) fn into_parts(self) -> (PmCompletionOccurrence, PmPrivateExternalIngressFault) {
        (self.occurrence, self.fault)
    }
}

/// Move-only request-side occurrence. Only [`PmLiveOccurrenceIssuer`] can
/// mint it and only the matching issuer can turn it into a completion cut.
#[derive(Debug)]
pub(crate) struct PmLiveHttpQueryTicket<P: PmLiveHttpQueryPurpose> {
    connection_epoch: ConnectionEpoch,
    request_sequence: IngressSequence,
    monotonic_request_ns: u64,
    purpose: std::marker::PhantomData<fn() -> P>,
}

pub(crate) type PmLiveOpenOrdersQueryTicket = PmLiveHttpQueryTicket<PmLiveOpenOrdersQuery>;
pub(crate) type PmLiveOrderDetailQueryTicket = PmLiveHttpQueryTicket<PmLiveOrderDetailQuery>;
pub(crate) type PmLiveAccountQueryTicket = PmLiveHttpQueryTicket<PmLiveAccountQuery>;
pub(crate) type PmLiveReconciliationQueryTicket = PmLiveHttpQueryTicket<PmLiveReconciliationQuery>;

/// Bounded, secret-free classification of a failed authenticated HTTP read.
/// Raw URLs, response bodies, credentials, and dynamically allocated error
/// text never cross the occurrence boundary.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum PmLiveHttpQueryFailure {
    Authentication,
    Transport,
    Timeout,
    HttpStatus,
    MalformedResponse,
    IncompleteResponse,
    PaginationCycle,
    ScopeMismatch,
    ContractViolation,
}

impl PmLiveHttpQueryFailure {
    const fn external_failure(self) -> PmPrivateExternalIngressFailure {
        match self {
            Self::Authentication | Self::Transport | Self::Timeout | Self::HttpStatus => {
                PmPrivateExternalIngressFailure::Service
            }
            Self::MalformedResponse | Self::IncompleteResponse | Self::PaginationCycle => {
                PmPrivateExternalIngressFailure::Normalization
            }
            Self::ScopeMismatch => PmPrivateExternalIngressFailure::Scope,
            Self::ContractViolation => PmPrivateExternalIngressFailure::Contract,
        }
    }
}

/// Common scheduler payload reached only after a purpose-specific failure
/// carrier has been consumed by its matching coordinator ingress.
#[derive(Debug)]
pub(crate) struct PmLiveHttpDependencyFailure {
    occurrence: PmCompletionOccurrence,
    request_sequence: IngressSequence,
    dependency: PmPrivateDependency,
    fault: PmPrivateExternalIngressFault,
}

impl PmLiveHttpDependencyFailure {
    pub(crate) const fn occurrence(&self) -> &PmCompletionOccurrence {
        &self.occurrence
    }

    pub(crate) const fn request_sequence(&self) -> IngressSequence {
        self.request_sequence
    }

    pub(crate) const fn dependency(&self) -> PmPrivateDependency {
        self.dependency
    }

    pub(crate) const fn fault(&self) -> PmPrivateExternalIngressFault {
        self.fault
    }
}

macro_rules! live_http_failure_input {
    ($name:ident) => {
        #[derive(Debug)]
        pub(crate) struct $name {
            failure: PmLiveHttpDependencyFailure,
        }

        impl $name {
            pub(crate) fn into_dependency_failure(self) -> PmLiveHttpDependencyFailure {
                self.failure
            }
        }
    };
}

live_http_failure_input!(PmLiveOpenOrdersFailureInput);
live_http_failure_input!(PmLiveOrderDetailFailureInput);
live_http_failure_input!(PmLiveAccountFailureInput);
live_http_failure_input!(PmLiveReconciliationFailureInput);

/// Sealed occurrence for an internal control command. It shares the issuer's
/// ordering domain and remains issuable after the active stream retires.
#[derive(Debug)]
pub(crate) struct PmLiveInternalControlOccurrence {
    occurrence: PmCompletionOccurrence,
}

impl PmLiveInternalControlOccurrence {
    pub(crate) fn into_occurrence(self) -> PmCompletionOccurrence {
        self.occurrence
    }
}

/// Sealed occurrence for one authenticated mutation completion. Mutation
/// workers cannot choose an epoch or ingress sequence independently.
#[derive(Debug)]
pub(crate) struct PmLiveMutationCompletionOccurrence {
    occurrence: PmCompletionOccurrence,
}

impl PmLiveMutationCompletionOccurrence {
    pub(crate) fn into_occurrence(self) -> PmCompletionOccurrence {
        self.occurrence
    }
}

/// Sealed occurrence for one Goal-F persistence poll performed by the live
/// product owner. Persistence polling shares the exact user/HTTP/mutation
/// sequence domain and cannot be relabeled from fixture evidence.
#[derive(Debug)]
pub(crate) struct PmLivePersistencePollOccurrence {
    occurrence: PmCompletionOccurrence,
}

impl PmLivePersistencePollOccurrence {
    pub(crate) fn into_occurrence(self) -> PmCompletionOccurrence {
        self.occurrence
    }
}

/// Sole crate-private issuer for live user-stream and authenticated HTTP
/// occurrence ordering.
///
/// Fixture constructors remain available for deterministic fixture/replay
/// tests, but live composition does not choose epochs, ingress sequences, or
/// snapshot revisions itself. User, HTTP, and actor observations share this
/// one connection-local admission sequence. Their raw receive clocks retain
/// independent producer-local high-water marks because bounded asynchronous
/// handoffs may legitimately deliver an older HTTP or user edge after a newer
/// actor-issued control edge.
#[derive(Debug)]
pub(crate) struct PmLiveOccurrenceIssuer {
    condition: PmConditionId,
    active_epoch: Option<ConnectionEpoch>,
    last_epoch: Option<ConnectionEpoch>,
    local_ingress_sequence: u64,
    last_snapshot_revision: u64,
    last_actor_monotonic_ns: Option<u64>,
    last_user_monotonic_ns: Option<u64>,
    last_http_monotonic_ns: Option<u64>,
}

impl PmLiveOccurrenceIssuer {
    pub(crate) const fn new(condition: PmConditionId) -> Self {
        Self {
            condition,
            active_epoch: None,
            last_epoch: None,
            local_ingress_sequence: 0,
            last_snapshot_revision: 0,
            last_actor_monotonic_ns: None,
            last_user_monotonic_ns: None,
            last_http_monotonic_ns: None,
        }
    }

    /// Mutation dispatch is unavailable until at least one real private
    /// connection epoch has been observed. Completion and persistence may
    /// continue against the last retired epoch.
    pub(crate) fn require_known_epoch(&self) -> Result<(), PmLiveOccurrenceError> {
        self.active_epoch
            .or(self.last_epoch)
            .map(|_| ())
            .ok_or(PmLiveOccurrenceError::NoKnownConnectionEpoch)
    }

    #[cfg(test)]
    pub(crate) fn start_for_test(
        &mut self,
        connection_epoch: ConnectionEpoch,
        received_clock: ReceivedEventClock,
    ) -> Result<PmLiveConnectionInput, PmLiveOccurrenceError> {
        self.start_exact(connection_epoch, received_clock)
    }

    #[cfg(test)]
    pub(crate) fn issue_user_frame_for_test(
        &mut self,
        connection_epoch: ConnectionEpoch,
        received_clock: ReceivedEventClock,
        frame: reap_polymarket_auth::CredentialOwnedUserFrame,
    ) -> Result<PmLivePrivateInput, PmLiveOccurrenceError> {
        let occurrence = self.issue_completion(connection_epoch, None, received_clock)?;
        Ok(PmLivePrivateInput::new(occurrence, frame))
    }

    #[cfg(test)]
    pub(crate) fn retire_for_test(
        &mut self,
        connection_epoch: ConnectionEpoch,
        received_clock: ReceivedEventClock,
        reason: PmUserWsDisconnectReason,
    ) -> Result<PmLiveRetirementInput, PmLiveOccurrenceError> {
        self.retire_exact(connection_epoch, received_clock, reason)
    }

    /// Starts one strictly newer authenticated user-stream epoch and returns
    /// its canonical connection occurrence.
    pub(crate) fn start_user_connection(
        &mut self,
        observation: PmUserWsObservation,
    ) -> Result<PmLiveConnectionInput, PmLiveOccurrenceError> {
        let connection = observation.connection();
        if connection.condition() != self.condition {
            return Err(PmLiveOccurrenceError::ConditionMismatch);
        }
        self.start_exact(
            connection.connection_epoch(),
            observation_clock(observation),
        )
    }

    /// Retires the exact active epoch, issues its final ordered occurrence,
    /// and prevents later user/HTTP input until a strictly newer open event.
    pub(crate) fn retire_user_connection(
        &mut self,
        retirement: PmUserWsRetirement,
    ) -> Result<PmLiveRetirementOutcome, PmLiveOccurrenceError> {
        let observation = retirement.observation();
        let connection = observation.connection();
        if connection.condition() != self.condition {
            return Err(PmLiveOccurrenceError::ConditionMismatch);
        }
        self.retire_observed(
            connection.connection_epoch(),
            observation_clock(observation),
            retirement.reason(),
        )
    }

    /// Attaches one already credential-bound user frame to the sole issued
    /// completion occurrence. No raw frame or credential material escapes.
    pub(crate) fn issue_user_frame(
        &mut self,
        frame: PmUserWsBoundFrame,
    ) -> Result<PmLivePrivateInput, PmLiveOccurrenceError> {
        let observation = frame.observation();
        let connection = observation.connection();
        if connection.condition() != self.condition {
            return Err(PmLiveOccurrenceError::ConditionMismatch);
        }
        let occurrence = self.issue_completion(
            connection.connection_epoch(),
            None,
            observation_clock(observation),
        )?;
        Ok(PmLivePrivateInput::new(
            occurrence,
            frame.into_credential_owned_frame(),
        ))
    }

    /// Reserves the next connection-local sequence for one exact HTTP
    /// request. Dropping the ticket consumes that sequence and cannot produce
    /// a false complete cut.
    pub(crate) fn begin_open_orders_query(
        &mut self,
        monotonic_request_ns: u64,
    ) -> Result<PmLiveOpenOrdersQueryTicket, PmLiveOccurrenceError> {
        self.begin_http_query(monotonic_request_ns)
    }

    pub(crate) fn begin_order_detail_query(
        &mut self,
        monotonic_request_ns: u64,
    ) -> Result<PmLiveOrderDetailQueryTicket, PmLiveOccurrenceError> {
        self.begin_http_query(monotonic_request_ns)
    }

    pub(crate) fn begin_account_query(
        &mut self,
        monotonic_request_ns: u64,
    ) -> Result<PmLiveAccountQueryTicket, PmLiveOccurrenceError> {
        self.begin_http_query(monotonic_request_ns)
    }

    pub(crate) fn begin_reconciliation_query(
        &mut self,
        monotonic_request_ns: u64,
    ) -> Result<PmLiveReconciliationQueryTicket, PmLiveOccurrenceError> {
        self.begin_http_query(monotonic_request_ns)
    }

    pub(crate) fn complete_open_orders_query(
        &mut self,
        ticket: PmLiveOpenOrdersQueryTicket,
        received_clock: ReceivedEventClock,
        monotonic_service_ns: u64,
        cut: PmCompleteOpenOrdersCut,
    ) -> Result<PmLiveOpenOrdersInput, PmLiveOccurrenceError> {
        let occurrence = self.complete_http_query(ticket, received_clock, monotonic_service_ns)?;
        Ok(PmLiveOpenOrdersInput::new(occurrence, cut))
    }

    pub(crate) fn complete_order_detail_query(
        &mut self,
        ticket: PmLiveOrderDetailQueryTicket,
        received_clock: ReceivedEventClock,
        monotonic_service_ns: u64,
        requested_order: PmVenueOrderKey,
        observation: PmExactOrderObservation,
    ) -> Result<PmLiveOrderDetailInput, PmLiveOccurrenceError> {
        let occurrence = self.complete_http_query(ticket, received_clock, monotonic_service_ns)?;
        Ok(PmLiveOrderDetailInput::new(
            occurrence,
            requested_order,
            observation,
        ))
    }

    pub(crate) fn complete_account_query(
        &mut self,
        ticket: PmLiveAccountQueryTicket,
        received_clock: ReceivedEventClock,
        monotonic_service_ns: u64,
        collateral: PmAccountBalanceAllowance,
        conditional: PmAccountBalanceAllowance,
    ) -> Result<PmLiveAccountInput, PmLiveOccurrenceError> {
        let occurrence = self.complete_http_query(ticket, received_clock, monotonic_service_ns)?;
        Ok(PmLiveAccountInput::new(
            occurrence,
            collateral,
            conditional,
        )?)
    }

    #[allow(
        clippy::too_many_arguments,
        reason = "the exact paired account/fill query retains its causal cut explicitly"
    )]
    pub(crate) fn complete_reconciliation_query(
        &mut self,
        ticket: PmLiveReconciliationQueryTicket,
        received_clock: ReceivedEventClock,
        monotonic_service_ns: u64,
        collateral: PmAccountBalanceAllowance,
        conditional: PmAccountBalanceAllowance,
        requested_after: Option<PmFillQueryCursor>,
        trades: PmCompleteTradesCut,
    ) -> Result<PmLiveReconciliationInput, PmLiveOccurrenceError> {
        let occurrence = self.complete_http_query(ticket, received_clock, monotonic_service_ns)?;
        Ok(PmLiveReconciliationInput::new(
            occurrence,
            collateral,
            conditional,
            requested_after,
            trades,
        )?)
    }

    pub(crate) fn fail_open_orders_query(
        &mut self,
        ticket: PmLiveOpenOrdersQueryTicket,
        received_clock: ReceivedEventClock,
        failure: PmLiveHttpQueryFailure,
    ) -> Result<PmLiveOpenOrdersFailureInput, PmLiveOccurrenceError> {
        Ok(PmLiveOpenOrdersFailureInput {
            failure: self.complete_http_failure(
                ticket,
                received_clock,
                PmPrivateDependency::OrderLifecycle,
                PmPrivateExternalIngressLane::OpenOrders,
                failure,
            )?,
        })
    }

    pub(crate) fn fail_order_detail_query(
        &mut self,
        ticket: PmLiveOrderDetailQueryTicket,
        received_clock: ReceivedEventClock,
        failure: PmLiveHttpQueryFailure,
    ) -> Result<PmLiveOrderDetailFailureInput, PmLiveOccurrenceError> {
        Ok(PmLiveOrderDetailFailureInput {
            failure: self.complete_http_failure(
                ticket,
                received_clock,
                PmPrivateDependency::OrderLifecycle,
                PmPrivateExternalIngressLane::OrderDetail,
                failure,
            )?,
        })
    }

    pub(crate) fn fail_account_query(
        &mut self,
        ticket: PmLiveAccountQueryTicket,
        received_clock: ReceivedEventClock,
        failure: PmLiveHttpQueryFailure,
    ) -> Result<PmLiveAccountFailureInput, PmLiveOccurrenceError> {
        Ok(PmLiveAccountFailureInput {
            failure: self.complete_http_failure(
                ticket,
                received_clock,
                PmPrivateDependency::AccountSnapshot,
                PmPrivateExternalIngressLane::AccountSnapshot,
                failure,
            )?,
        })
    }

    pub(crate) fn fail_reconciliation_query(
        &mut self,
        ticket: PmLiveReconciliationQueryTicket,
        received_clock: ReceivedEventClock,
        failure: PmLiveHttpQueryFailure,
    ) -> Result<PmLiveReconciliationFailureInput, PmLiveOccurrenceError> {
        Ok(PmLiveReconciliationFailureInput {
            failure: self.complete_http_failure(
                ticket,
                received_clock,
                PmPrivateDependency::Reconciliation,
                PmPrivateExternalIngressLane::Reconciliation,
                failure,
            )?,
        })
    }

    /// Issues one internal stop/control occurrence from the same strict
    /// sequence as user, HTTP, and mutation observations. The last retired
    /// epoch remains valid for shutdown so fail-closed control does not need a
    /// fabricated fixture occurrence.
    pub(crate) fn issue_internal_control(
        &mut self,
        received_clock: ReceivedEventClock,
    ) -> Result<PmLiveInternalControlOccurrence, PmLiveOccurrenceError> {
        let epoch = self
            .active_epoch
            .or(self.last_epoch)
            .ok_or(PmLiveOccurrenceError::NoKnownConnectionEpoch)?;
        let occurrence = self.issue_actor_ordered(epoch, None, received_clock)?;
        Ok(PmLiveInternalControlOccurrence { occurrence })
    }

    /// Issues a transport-originated shutdown control while retaining the
    /// user-WebSocket receive edge in that producer's monotonic domain.
    pub(crate) fn issue_user_shutdown_control(
        &mut self,
        received_clock: ReceivedEventClock,
    ) -> Result<PmLiveInternalControlOccurrence, PmLiveOccurrenceError> {
        let epoch = self
            .active_epoch
            .or(self.last_epoch)
            .ok_or(PmLiveOccurrenceError::NoKnownConnectionEpoch)?;
        let occurrence = self.issue_user_ordered(epoch, None, received_clock)?;
        Ok(PmLiveInternalControlOccurrence { occurrence })
    }

    pub(crate) fn issue_mutation_completion(
        &mut self,
        received_clock: ReceivedEventClock,
    ) -> Result<PmLiveMutationCompletionOccurrence, PmLiveOccurrenceError> {
        let epoch = self
            .active_epoch
            .or(self.last_epoch)
            .ok_or(PmLiveOccurrenceError::NoKnownConnectionEpoch)?;
        let occurrence = self.issue_actor_ordered(epoch, None, received_clock)?;
        Ok(PmLiveMutationCompletionOccurrence { occurrence })
    }

    /// Issues one live persistence-poll occurrence from the same strict
    /// sequence as user, HTTP, control, and mutation completion evidence.
    /// Polling remains possible after stream retirement so an already-sent
    /// mutation can finish its Goal-F durability bridge while private input is
    /// unavailable.
    pub(crate) fn issue_persistence_poll(
        &mut self,
        received_clock: ReceivedEventClock,
    ) -> Result<PmLivePersistencePollOccurrence, PmLiveOccurrenceError> {
        let epoch = self
            .active_epoch
            .or(self.last_epoch)
            .ok_or(PmLiveOccurrenceError::NoKnownConnectionEpoch)?;
        let occurrence = self.issue_actor_ordered(epoch, None, received_clock)?;
        Ok(PmLivePersistencePollOccurrence { occurrence })
    }

    fn begin_http_query<P: PmLiveHttpQueryPurpose>(
        &mut self,
        monotonic_request_ns: u64,
    ) -> Result<PmLiveHttpQueryTicket<P>, PmLiveOccurrenceError> {
        let connection_epoch = self
            .active_epoch
            .ok_or(PmLiveOccurrenceError::NoActiveConnection)?;
        Self::validate_monotonic(self.last_actor_monotonic_ns, monotonic_request_ns)?;
        let request_sequence = self.next_ingress_sequence()?;
        self.local_ingress_sequence = request_sequence.value();
        self.last_actor_monotonic_ns = Some(monotonic_request_ns);
        Ok(PmLiveHttpQueryTicket {
            connection_epoch,
            request_sequence,
            monotonic_request_ns,
            purpose: std::marker::PhantomData,
        })
    }

    /// Completes an authenticated HTTP request with a freshly assigned
    /// snapshot revision and completion occurrence.
    fn complete_http_query<P: PmLiveHttpQueryPurpose>(
        &mut self,
        ticket: PmLiveHttpQueryTicket<P>,
        received_clock: ReceivedEventClock,
        monotonic_service_ns: u64,
    ) -> Result<PmFixtureQueryOccurrence, PmLiveOccurrenceError> {
        if self.active_epoch != Some(ticket.connection_epoch) {
            return Err(PmLiveOccurrenceError::EpochMismatch);
        }
        if received_clock.monotonic_receive_ns() < ticket.monotonic_request_ns {
            return Err(PmLiveOccurrenceError::CompletionBeforeRequest);
        }
        Self::validate_monotonic(
            self.last_http_monotonic_ns,
            received_clock.monotonic_receive_ns(),
        )?;
        Self::validate_monotonic(self.last_actor_monotonic_ns, monotonic_service_ns)?;
        let completion_sequence = self.next_ingress_sequence()?;
        let revision_value = self
            .last_snapshot_revision
            .checked_add(1)
            .ok_or(PmLiveOccurrenceError::SnapshotRevisionOverflow)?;
        let snapshot = PmSnapshotEvidence::new(SnapshotRevision::new(revision_value))?;
        let ordering = EventOrdering::new(
            ticket.connection_epoch,
            Some(snapshot.revision()),
            None,
            None,
            completion_sequence,
        )?;
        let occurrence = PmCompletionOccurrence::new(received_clock, ordering);
        let query = PmFixtureQueryOccurrence::new(
            ticket.connection_epoch,
            ticket.request_sequence,
            snapshot,
            occurrence,
            monotonic_service_ns,
        )?;
        self.local_ingress_sequence = completion_sequence.value();
        self.last_snapshot_revision = revision_value;
        self.last_http_monotonic_ns = Some(received_clock.monotonic_receive_ns());
        self.last_actor_monotonic_ns = Some(monotonic_service_ns);
        Ok(query)
    }

    fn complete_http_failure<P: PmLiveHttpQueryPurpose>(
        &mut self,
        ticket: PmLiveHttpQueryTicket<P>,
        received_clock: ReceivedEventClock,
        dependency: PmPrivateDependency,
        lane: PmPrivateExternalIngressLane,
        failure: PmLiveHttpQueryFailure,
    ) -> Result<PmLiveHttpDependencyFailure, PmLiveOccurrenceError> {
        if self.active_epoch != Some(ticket.connection_epoch) {
            return Err(PmLiveOccurrenceError::EpochMismatch);
        }
        if received_clock.monotonic_receive_ns() < ticket.monotonic_request_ns {
            return Err(PmLiveOccurrenceError::CompletionBeforeRequest);
        }
        let occurrence = self.issue_http_ordered(ticket.connection_epoch, None, received_clock)?;
        Ok(PmLiveHttpDependencyFailure {
            occurrence,
            request_sequence: ticket.request_sequence,
            dependency,
            fault: PmPrivateExternalIngressFault::new(lane, failure.external_failure()),
        })
    }

    fn start_exact(
        &mut self,
        connection_epoch: ConnectionEpoch,
        received_clock: ReceivedEventClock,
    ) -> Result<PmLiveConnectionInput, PmLiveOccurrenceError> {
        if self.active_epoch.is_some() {
            return Err(PmLiveOccurrenceError::ActiveConnectionNotRetired);
        }
        if connection_epoch.value() == 0
            || self
                .last_epoch
                .is_some_and(|previous| connection_epoch <= previous)
        {
            return Err(PmLiveOccurrenceError::EpochDidNotAdvance);
        }
        Self::validate_monotonic(
            self.last_user_monotonic_ns,
            received_clock.monotonic_receive_ns(),
        )?;
        let ordering =
            EventOrdering::new(connection_epoch, None, None, None, IngressSequence::new(1))?;
        let occurrence = PmCompletionOccurrence::new(received_clock, ordering);
        self.active_epoch = Some(connection_epoch);
        self.last_epoch = Some(connection_epoch);
        self.local_ingress_sequence = 1;
        self.last_user_monotonic_ns = Some(received_clock.monotonic_receive_ns());
        Ok(PmLiveConnectionInput { occurrence })
    }

    fn retire_exact(
        &mut self,
        connection_epoch: ConnectionEpoch,
        received_clock: ReceivedEventClock,
        reason: PmUserWsDisconnectReason,
    ) -> Result<PmLiveRetirementInput, PmLiveOccurrenceError> {
        let occurrence = self.issue_completion(connection_epoch, None, received_clock)?;
        self.active_epoch = None;
        Ok(PmLiveRetirementInput {
            occurrence,
            fault: retirement_fault(reason),
        })
    }

    fn retire_observed(
        &mut self,
        connection_epoch: ConnectionEpoch,
        received_clock: ReceivedEventClock,
        reason: PmUserWsDisconnectReason,
    ) -> Result<PmLiveRetirementOutcome, PmLiveOccurrenceError> {
        if self.active_epoch == Some(connection_epoch) {
            return self
                .retire_exact(connection_epoch, received_clock, reason)
                .map(PmLiveRetirementOutcome::Active);
        }
        if self.active_epoch.is_some() {
            return Err(PmLiveOccurrenceError::EpochMismatch);
        }
        if connection_epoch.value() == 0
            || self
                .last_epoch
                .is_some_and(|previous| connection_epoch <= previous)
        {
            return Err(PmLiveOccurrenceError::EpochDidNotAdvance);
        }
        Self::validate_monotonic(
            self.last_user_monotonic_ns,
            received_clock.monotonic_receive_ns(),
        )?;
        self.last_epoch = Some(connection_epoch);
        self.local_ingress_sequence = 0;
        self.last_user_monotonic_ns = Some(received_clock.monotonic_receive_ns());
        Ok(PmLiveRetirementOutcome::PreOpen(PmLivePreOpenRetirement {
            connection_epoch,
            received_clock,
            fault: retirement_fault(reason),
        }))
    }

    fn issue_completion(
        &mut self,
        connection_epoch: ConnectionEpoch,
        snapshot_revision: Option<SnapshotRevision>,
        received_clock: ReceivedEventClock,
    ) -> Result<PmCompletionOccurrence, PmLiveOccurrenceError> {
        if self.active_epoch != Some(connection_epoch) {
            return Err(PmLiveOccurrenceError::EpochMismatch);
        }
        self.issue_user_ordered(connection_epoch, snapshot_revision, received_clock)
    }

    fn issue_actor_ordered(
        &mut self,
        connection_epoch: ConnectionEpoch,
        snapshot_revision: Option<SnapshotRevision>,
        received_clock: ReceivedEventClock,
    ) -> Result<PmCompletionOccurrence, PmLiveOccurrenceError> {
        Self::validate_monotonic(
            self.last_actor_monotonic_ns,
            received_clock.monotonic_receive_ns(),
        )?;
        let occurrence = self.issue_ordered(connection_epoch, snapshot_revision, received_clock)?;
        self.last_actor_monotonic_ns = Some(received_clock.monotonic_receive_ns());
        Ok(occurrence)
    }

    fn issue_user_ordered(
        &mut self,
        connection_epoch: ConnectionEpoch,
        snapshot_revision: Option<SnapshotRevision>,
        received_clock: ReceivedEventClock,
    ) -> Result<PmCompletionOccurrence, PmLiveOccurrenceError> {
        Self::validate_monotonic(
            self.last_user_monotonic_ns,
            received_clock.monotonic_receive_ns(),
        )?;
        let occurrence = self.issue_ordered(connection_epoch, snapshot_revision, received_clock)?;
        self.last_user_monotonic_ns = Some(received_clock.monotonic_receive_ns());
        Ok(occurrence)
    }

    fn issue_http_ordered(
        &mut self,
        connection_epoch: ConnectionEpoch,
        snapshot_revision: Option<SnapshotRevision>,
        received_clock: ReceivedEventClock,
    ) -> Result<PmCompletionOccurrence, PmLiveOccurrenceError> {
        Self::validate_monotonic(
            self.last_http_monotonic_ns,
            received_clock.monotonic_receive_ns(),
        )?;
        let occurrence = self.issue_ordered(connection_epoch, snapshot_revision, received_clock)?;
        self.last_http_monotonic_ns = Some(received_clock.monotonic_receive_ns());
        Ok(occurrence)
    }

    fn issue_ordered(
        &mut self,
        connection_epoch: ConnectionEpoch,
        snapshot_revision: Option<SnapshotRevision>,
        received_clock: ReceivedEventClock,
    ) -> Result<PmCompletionOccurrence, PmLiveOccurrenceError> {
        let sequence = self.next_ingress_sequence()?;
        let ordering =
            EventOrdering::new(connection_epoch, snapshot_revision, None, None, sequence)?;
        let occurrence = PmCompletionOccurrence::new(received_clock, ordering);
        self.local_ingress_sequence = sequence.value();
        Ok(occurrence)
    }

    fn next_ingress_sequence(&self) -> Result<IngressSequence, PmLiveOccurrenceError> {
        self.local_ingress_sequence
            .checked_add(1)
            .map(IngressSequence::new)
            .ok_or(PmLiveOccurrenceError::IngressSequenceOverflow)
    }

    fn validate_monotonic(
        previous_monotonic_ns: Option<u64>,
        monotonic_ns: u64,
    ) -> Result<(), PmLiveOccurrenceError> {
        if monotonic_ns == 0 {
            return Err(PmLiveOccurrenceError::ZeroMonotonicTimestamp);
        }
        if previous_monotonic_ns.is_some_and(|previous| monotonic_ns < previous) {
            return Err(PmLiveOccurrenceError::MonotonicClockRegression);
        }
        Ok(())
    }
}

fn observation_clock(observation: PmUserWsObservation) -> ReceivedEventClock {
    let clock = observation.clock();
    ReceivedEventClock::new(
        None,
        clock.local_wall_receive_ns(),
        clock.monotonic_receive_ns(),
    )
    .expect("PmUserWsEdgeClock already validates the same received-clock contract")
}

const fn retirement_fault(reason: PmUserWsDisconnectReason) -> PmPrivateExternalIngressFault {
    let failure = match reason {
        PmUserWsDisconnectReason::MalformedFrame => PmPrivateExternalIngressFailure::Normalization,
        PmUserWsDisconnectReason::CredentialOwnerMismatch => PmPrivateExternalIngressFailure::Scope,
        PmUserWsDisconnectReason::BinaryFrame
        | PmUserWsDisconnectReason::FrameTooLarge
        | PmUserWsDisconnectReason::UnexpectedProtocolFrame => {
            PmPrivateExternalIngressFailure::Contract
        }
        PmUserWsDisconnectReason::ConnectTimeout
        | PmUserWsDisconnectReason::ConnectFailed
        | PmUserWsDisconnectReason::SubscriptionAuthenticationFailed
        | PmUserWsDisconnectReason::SubscriptionWriteTimeout
        | PmUserWsDisconnectReason::SubscriptionWriteFailed
        | PmUserWsDisconnectReason::SocketReadFailed
        | PmUserWsDisconnectReason::SocketClosed
        | PmUserWsDisconnectReason::SocketWriteTimeout
        | PmUserWsDisconnectReason::SocketWriteFailed
        | PmUserWsDisconnectReason::CredentialAuthorityUnavailable
        | PmUserWsDisconnectReason::IdleTimeout
        | PmUserWsDisconnectReason::PongTimeout => PmPrivateExternalIngressFailure::Service,
    };
    PmPrivateExternalIngressFault::new(PmPrivateExternalIngressLane::PrivateLifecycle, failure)
}

#[derive(Debug, Error)]
pub(crate) enum PmLiveOccurrenceError {
    #[error("live occurrence names another configured condition")]
    ConditionMismatch,
    #[error("live private connection epoch did not strictly advance")]
    EpochDidNotAdvance,
    #[error("live private connection must retire before a replacement epoch can open")]
    ActiveConnectionNotRetired,
    #[error("live occurrence has no active private connection")]
    NoActiveConnection,
    #[error("live occurrence has no current or retired connection epoch")]
    NoKnownConnectionEpoch,
    #[error("live occurrence belongs to another private connection epoch")]
    EpochMismatch,
    #[error("live HTTP completion preceded its exact request")]
    CompletionBeforeRequest,
    #[error("live occurrence monotonic timestamp must be nonzero")]
    ZeroMonotonicTimestamp,
    #[error("live occurrence monotonic clock regressed")]
    MonotonicClockRegression,
    #[error("live occurrence ingress sequence overflowed")]
    IngressSequenceOverflow,
    #[error("live HTTP snapshot revision overflowed")]
    SnapshotRevisionOverflow,
    #[error(transparent)]
    Envelope(#[from] EnvelopeError),
    #[error(transparent)]
    Event(#[from] PmEventError),
    #[error(transparent)]
    Query(#[from] PmPrivateMonitorInputError),
}

#[cfg(test)]
mod tests {
    use super::*;

    const CONDITION: &str = "0xaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa";

    fn clock(monotonic_ns: u64) -> ReceivedEventClock {
        ReceivedEventClock::new(None, 1_700_000_000_000_000_000 + monotonic_ns, monotonic_ns)
            .unwrap()
    }

    fn issuer() -> PmLiveOccurrenceIssuer {
        PmLiveOccurrenceIssuer::new(PmConditionId::parse(CONDITION).unwrap())
    }

    #[test]
    fn user_and_http_occurrences_share_one_collision_free_epoch_sequence() {
        let mut issuer = issuer();
        let connected = issuer
            .start_exact(ConnectionEpoch::new(7), clock(10))
            .unwrap();
        let connected = connected.into_occurrence();
        assert_eq!(
            connected.ordering().local_ingress_sequence(),
            IngressSequence::new(1)
        );

        let ticket = issuer.begin_open_orders_query(11).unwrap();
        let query = issuer.complete_http_query(ticket, clock(12), 13).unwrap();
        assert_eq!(query.request_sequence(), IngressSequence::new(2));
        assert_eq!(
            query.completion().ordering().local_ingress_sequence(),
            IngressSequence::new(3)
        );
        assert_eq!(query.snapshot().revision(), SnapshotRevision::new(1));

        let next = issuer.begin_account_query(14).unwrap();
        let next = issuer.complete_http_query(next, clock(15), 16).unwrap();
        assert_eq!(next.request_sequence(), IngressSequence::new(4));
        assert_eq!(
            next.completion().ordering().local_ingress_sequence(),
            IngressSequence::new(5)
        );
        assert_eq!(next.snapshot().revision(), SnapshotRevision::new(2));
    }

    #[test]
    fn stale_epoch_and_actor_clock_fail_closed_without_claiming_completion() {
        let mut issuer = issuer();
        issuer
            .start_exact(ConnectionEpoch::new(7), clock(10))
            .unwrap();
        let stale = issuer.begin_reconciliation_query(11).unwrap();
        issuer
            .retire_exact(
                ConnectionEpoch::new(7),
                clock(12),
                PmUserWsDisconnectReason::SocketClosed,
            )
            .unwrap();
        issuer
            .start_exact(ConnectionEpoch::new(8), clock(13))
            .unwrap();
        assert!(matches!(
            issuer.complete_http_query(stale, clock(14), 15),
            Err(PmLiveOccurrenceError::EpochMismatch)
        ));
        assert!(issuer.issue_internal_control(clock(16)).is_ok());
        assert!(matches!(
            issuer.begin_order_detail_query(15),
            Err(PmLiveOccurrenceError::MonotonicClockRegression)
        ));
    }

    #[test]
    fn active_connection_cannot_be_silently_replaced() {
        let mut issuer = issuer();
        issuer
            .start_exact(ConnectionEpoch::new(7), clock(10))
            .unwrap();
        assert!(matches!(
            issuer.start_exact(ConnectionEpoch::new(8), clock(11)),
            Err(PmLiveOccurrenceError::ActiveConnectionNotRetired)
        ));
        let retired = issuer
            .retire_exact(
                ConnectionEpoch::new(7),
                clock(12),
                PmUserWsDisconnectReason::SocketClosed,
            )
            .unwrap();
        assert_eq!(
            retired.into_parts().0.ordering().local_ingress_sequence(),
            IngressSequence::new(2),
            "rejected replacement must not consume or reset the active sequence",
        );
        assert!(
            issuer
                .start_exact(ConnectionEpoch::new(8), clock(13))
                .is_ok()
        );
    }

    #[test]
    fn retirement_seals_the_epoch_and_classifies_transport_failure() {
        let mut issuer = issuer();
        issuer
            .start_exact(ConnectionEpoch::new(7), clock(10))
            .unwrap();
        let retired = issuer
            .retire_exact(
                ConnectionEpoch::new(7),
                clock(11),
                PmUserWsDisconnectReason::CredentialOwnerMismatch,
            )
            .unwrap();
        let (occurrence, fault) = retired.into_parts();
        assert_eq!(
            occurrence.ordering().local_ingress_sequence(),
            IngressSequence::new(2)
        );
        assert_eq!(fault.lane(), PmPrivateExternalIngressLane::PrivateLifecycle);
        assert_eq!(fault.failure(), PmPrivateExternalIngressFailure::Scope);
        assert!(matches!(
            issuer.begin_open_orders_query(12),
            Err(PmLiveOccurrenceError::NoActiveConnection)
        ));
        assert!(matches!(
            issuer.start_exact(ConnectionEpoch::new(7), clock(12)),
            Err(PmLiveOccurrenceError::EpochDidNotAdvance)
        ));
        let next = issuer
            .start_exact(ConnectionEpoch::new(8), clock(12))
            .unwrap()
            .into_occurrence();
        assert_eq!(
            next.ordering().local_ingress_sequence(),
            IngressSequence::new(1)
        );
    }

    #[test]
    fn http_purposes_are_distinct_move_only_ticket_types() {
        assert_ne!(
            std::any::TypeId::of::<PmLiveOpenOrdersQueryTicket>(),
            std::any::TypeId::of::<PmLiveAccountQueryTicket>()
        );
        assert_ne!(
            std::any::TypeId::of::<PmLiveOrderDetailQueryTicket>(),
            std::any::TypeId::of::<PmLiveReconciliationQueryTicket>()
        );
        let _: fn(
            &mut PmLiveOccurrenceIssuer,
            u64,
        ) -> Result<PmLiveOpenOrdersQueryTicket, PmLiveOccurrenceError> =
            PmLiveOccurrenceIssuer::begin_open_orders_query;
        let _: fn(
            &mut PmLiveOccurrenceIssuer,
            u64,
        ) -> Result<PmLiveAccountQueryTicket, PmLiveOccurrenceError> =
            PmLiveOccurrenceIssuer::begin_account_query;
    }

    #[test]
    fn pre_open_retirement_consumes_the_failed_attempt_without_fake_disconnect() {
        let mut issuer = issuer();
        let outcome = issuer
            .retire_observed(
                ConnectionEpoch::new(7),
                clock(10),
                PmUserWsDisconnectReason::ConnectFailed,
            )
            .unwrap();
        let PmLiveRetirementOutcome::PreOpen(retired) = outcome else {
            panic!("a failed connect cannot claim an active canonical disconnect");
        };
        assert_eq!(retired.connection_epoch(), ConnectionEpoch::new(7));
        assert_eq!(retired.received_clock(), clock(10));
        assert_eq!(
            retired.fault().failure(),
            PmPrivateExternalIngressFailure::Service
        );
        assert!(matches!(
            issuer.start_exact(ConnectionEpoch::new(7), clock(11)),
            Err(PmLiveOccurrenceError::EpochDidNotAdvance)
        ));
        assert!(
            issuer
                .start_exact(ConnectionEpoch::new(8), clock(11))
                .is_ok()
        );
    }

    #[test]
    fn http_failure_is_purpose_bound_and_does_not_claim_a_snapshot_revision() {
        let mut issuer = issuer();
        issuer
            .start_exact(ConnectionEpoch::new(7), clock(10))
            .unwrap();
        let account = issuer.begin_account_query(11).unwrap();
        let failure = issuer
            .fail_account_query(
                account,
                clock(12),
                PmLiveHttpQueryFailure::IncompleteResponse,
            )
            .unwrap()
            .into_dependency_failure();
        assert_eq!(failure.request_sequence(), IngressSequence::new(2));
        assert_eq!(failure.dependency(), PmPrivateDependency::AccountSnapshot);
        assert_eq!(
            failure.fault(),
            PmPrivateExternalIngressFault::new(
                PmPrivateExternalIngressLane::AccountSnapshot,
                PmPrivateExternalIngressFailure::Normalization,
            )
        );
        assert_eq!(
            failure.occurrence().ordering().local_ingress_sequence(),
            IngressSequence::new(3)
        );
        assert_eq!(failure.occurrence().ordering().snapshot_revision(), None);

        let next = issuer.begin_open_orders_query(13).unwrap();
        let completed = issuer.complete_http_query(next, clock(14), 15).unwrap();
        assert_eq!(completed.snapshot().revision(), SnapshotRevision::new(1));
        assert_ne!(
            std::any::TypeId::of::<PmLiveAccountFailureInput>(),
            std::any::TypeId::of::<PmLiveOpenOrdersFailureInput>()
        );
    }

    #[test]
    fn all_http_failure_tickets_preserve_purpose_classification_without_snapshot_authority() {
        let mut issuer = issuer();
        issuer
            .start_exact(ConnectionEpoch::new(7), clock(10))
            .unwrap();

        let open_orders = issuer.begin_open_orders_query(11).unwrap();
        let open_orders = issuer
            .fail_open_orders_query(open_orders, clock(12), PmLiveHttpQueryFailure::Transport)
            .unwrap()
            .into_dependency_failure();
        assert_eq!(open_orders.request_sequence(), IngressSequence::new(2));
        assert_eq!(
            open_orders.dependency(),
            PmPrivateDependency::OrderLifecycle
        );
        assert_eq!(
            open_orders.fault(),
            PmPrivateExternalIngressFault::new(
                PmPrivateExternalIngressLane::OpenOrders,
                PmPrivateExternalIngressFailure::Service,
            )
        );
        assert_eq!(
            open_orders.occurrence().ordering().snapshot_revision(),
            None
        );

        let detail = issuer.begin_order_detail_query(13).unwrap();
        let detail = issuer
            .fail_order_detail_query(detail, clock(14), PmLiveHttpQueryFailure::MalformedResponse)
            .unwrap()
            .into_dependency_failure();
        assert_eq!(detail.request_sequence(), IngressSequence::new(4));
        assert_eq!(detail.dependency(), PmPrivateDependency::OrderLifecycle);
        assert_eq!(
            detail.fault(),
            PmPrivateExternalIngressFault::new(
                PmPrivateExternalIngressLane::OrderDetail,
                PmPrivateExternalIngressFailure::Normalization,
            )
        );
        assert_eq!(detail.occurrence().ordering().snapshot_revision(), None);

        let account = issuer.begin_account_query(15).unwrap();
        let account = issuer
            .fail_account_query(account, clock(16), PmLiveHttpQueryFailure::ScopeMismatch)
            .unwrap()
            .into_dependency_failure();
        assert_eq!(account.request_sequence(), IngressSequence::new(6));
        assert_eq!(account.dependency(), PmPrivateDependency::AccountSnapshot);
        assert_eq!(
            account.fault(),
            PmPrivateExternalIngressFault::new(
                PmPrivateExternalIngressLane::AccountSnapshot,
                PmPrivateExternalIngressFailure::Scope,
            )
        );
        assert_eq!(account.occurrence().ordering().snapshot_revision(), None);

        let reconciliation = issuer.begin_reconciliation_query(17).unwrap();
        let reconciliation = issuer
            .fail_reconciliation_query(
                reconciliation,
                clock(18),
                PmLiveHttpQueryFailure::ContractViolation,
            )
            .unwrap()
            .into_dependency_failure();
        assert_eq!(reconciliation.request_sequence(), IngressSequence::new(8));
        assert_eq!(
            reconciliation.dependency(),
            PmPrivateDependency::Reconciliation
        );
        assert_eq!(
            reconciliation.fault(),
            PmPrivateExternalIngressFault::new(
                PmPrivateExternalIngressLane::Reconciliation,
                PmPrivateExternalIngressFailure::Contract,
            )
        );
        assert_eq!(
            reconciliation.occurrence().ordering().snapshot_revision(),
            None
        );

        let next = issuer.begin_open_orders_query(19).unwrap();
        let completed = issuer.complete_http_query(next, clock(20), 21).unwrap();
        assert_eq!(
            completed.snapshot().revision(),
            SnapshotRevision::new(1),
            "four failed reads must not consume or fabricate a snapshot revision",
        );
    }

    #[test]
    fn internal_control_shares_user_and_http_strict_ordering() {
        let mut issuer = issuer();
        issuer
            .start_exact(ConnectionEpoch::new(7), clock(10))
            .unwrap();
        let user = issuer
            .issue_completion(ConnectionEpoch::new(7), None, clock(11))
            .unwrap();
        let ticket = issuer.begin_open_orders_query(12).unwrap();
        let http = issuer.complete_http_query(ticket, clock(13), 14).unwrap();
        let control = issuer
            .issue_internal_control(clock(15))
            .unwrap()
            .into_occurrence();
        assert_eq!(
            user.ordering().local_ingress_sequence(),
            IngressSequence::new(2)
        );
        assert_eq!(http.request_sequence(), IngressSequence::new(3));
        assert_eq!(
            http.completion().ordering().local_ingress_sequence(),
            IngressSequence::new(4)
        );
        assert_eq!(
            control.ordering().local_ingress_sequence(),
            IngressSequence::new(5)
        );
    }

    #[test]
    fn delayed_http_edge_keeps_raw_time_behind_a_newer_actor_poll() {
        let mut issuer = issuer();
        issuer
            .start_exact(ConnectionEpoch::new(7), clock(10))
            .unwrap();
        let detail = issuer.begin_order_detail_query(20).unwrap();
        let poll = issuer
            .issue_persistence_poll(clock(30))
            .unwrap()
            .into_occurrence();
        let completed = issuer.complete_http_query(detail, clock(25), 40).unwrap();

        assert_eq!(
            poll.ordering().local_ingress_sequence(),
            IngressSequence::new(3)
        );
        assert_eq!(
            completed.completion().ordering().local_ingress_sequence(),
            IngressSequence::new(4),
            "one shared actor-admission sequence remains collision-free"
        );
        assert_eq!(
            completed
                .completion()
                .received_clock()
                .monotonic_receive_ns(),
            25,
            "the private HTTP receive edge must not be clamped to the newer actor poll"
        );
        assert_eq!(completed.monotonic_service_ns(), 40);

        let before_request = issuer.begin_account_query(41).unwrap();
        assert!(matches!(
            issuer.complete_http_query(before_request, clock(40), 42),
            Err(PmLiveOccurrenceError::CompletionBeforeRequest)
        ));
    }

    #[test]
    fn each_occurrence_producer_rejects_its_own_clock_regression() {
        let mut actor = issuer();
        actor
            .start_exact(ConnectionEpoch::new(7), clock(10))
            .unwrap();
        actor.issue_persistence_poll(clock(30)).unwrap();
        assert!(matches!(
            actor.issue_internal_control(clock(29)),
            Err(PmLiveOccurrenceError::MonotonicClockRegression)
        ));

        let mut user = issuer();
        user.start_exact(ConnectionEpoch::new(7), clock(10))
            .unwrap();
        user.issue_persistence_poll(clock(30)).unwrap();
        user.issue_user_shutdown_control(clock(20)).unwrap();
        assert!(matches!(
            user.issue_user_shutdown_control(clock(19)),
            Err(PmLiveOccurrenceError::MonotonicClockRegression)
        ));

        let mut http = issuer();
        http.start_exact(ConnectionEpoch::new(7), clock(10))
            .unwrap();
        let older = http.begin_order_detail_query(20).unwrap();
        let newer = http.begin_account_query(21).unwrap();
        http.complete_http_query(newer, clock(30), 31).unwrap();
        assert!(matches!(
            http.complete_http_query(older, clock(25), 32),
            Err(PmLiveOccurrenceError::MonotonicClockRegression)
        ));
    }

    #[test]
    fn in_flight_mutation_completion_remains_ordered_after_stream_retirement() {
        let mut issuer = issuer();
        issuer
            .start_exact(ConnectionEpoch::new(7), clock(10))
            .unwrap();
        let retired = issuer
            .retire_exact(
                ConnectionEpoch::new(7),
                clock(11),
                PmUserWsDisconnectReason::SocketClosed,
            )
            .unwrap();
        let retired_occurrence = retired.into_parts().0;
        let completion = issuer
            .issue_mutation_completion(clock(12))
            .unwrap()
            .into_occurrence();
        let persistence = issuer
            .issue_persistence_poll(clock(13))
            .unwrap()
            .into_occurrence();
        assert_eq!(
            retired_occurrence.ordering().local_ingress_sequence(),
            IngressSequence::new(2)
        );
        assert_eq!(
            completion.ordering().connection_epoch(),
            ConnectionEpoch::new(7)
        );
        assert_eq!(
            completion.ordering().local_ingress_sequence(),
            IngressSequence::new(3)
        );
        assert_eq!(
            persistence.ordering().connection_epoch(),
            ConnectionEpoch::new(7)
        );
        assert_eq!(
            persistence.ordering().local_ingress_sequence(),
            IngressSequence::new(4)
        );
    }
}
