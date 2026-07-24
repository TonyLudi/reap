use std::fmt;

use reap_pm_core::{
    ConnectionEpoch, EnvelopeError, EventEnvelope, EventOrdering, IngressSequence, PmAccountScope,
    PmConnectionId, PmProductSource, PmSourceBound, ReceivedEventClock, ReceivedEventEnvelope,
    SnapshotRevision,
};
use thiserror::Error;

use crate::PmFixtureInstrumentScope;
use crate::fixture_scope::PmFixtureOwnerId;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct PmFixtureDeliveryScope {
    account_scope: PmAccountScope,
    instrument: PmFixtureInstrumentScope,
}

impl PmFixtureDeliveryScope {
    #[must_use]
    pub const fn account_scope(self) -> PmAccountScope {
        self.account_scope
    }

    #[must_use]
    pub const fn instrument_scope(self) -> PmFixtureInstrumentScope {
        self.instrument
    }
}

#[derive(Debug, PartialEq, Eq)]
pub(crate) struct PmFixtureRequestOccurrence {
    connection_epoch: ConnectionEpoch,
    sequence: IngressSequence,
}

impl PmFixtureRequestOccurrence {
    pub(crate) fn new(
        connection_epoch: ConnectionEpoch,
        sequence: IngressSequence,
    ) -> Result<Self, PmFixtureDeliveryError> {
        if connection_epoch.value() == 0 {
            return Err(PmFixtureDeliveryError::ZeroRequestEpoch);
        }
        if sequence.value() == 0 {
            return Err(PmFixtureDeliveryError::ZeroRequestSequence);
        }
        Ok(Self {
            connection_epoch,
            sequence,
        })
    }

    pub(crate) const fn connection_epoch(&self) -> ConnectionEpoch {
        self.connection_epoch
    }

    pub(crate) const fn sequence(&self) -> IngressSequence {
        self.sequence
    }
}

#[derive(Debug, PartialEq, Eq)]
pub struct PmFixtureCompletionOccurrence {
    received_clock: ReceivedEventClock,
    ordering: EventOrdering,
}

impl PmFixtureCompletionOccurrence {
    #[must_use]
    pub const fn new(received_clock: ReceivedEventClock, ordering: EventOrdering) -> Self {
        Self {
            received_clock,
            ordering,
        }
    }

    #[must_use]
    pub const fn received_clock(&self) -> ReceivedEventClock {
        self.received_clock
    }

    #[must_use]
    pub const fn ordering(&self) -> EventOrdering {
        self.ordering
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Error)]
pub enum PmFixtureDeliveryError {
    #[error("fixture request connection epoch must be nonzero")]
    ZeroRequestEpoch,
    #[error("fixture request sequence must be nonzero")]
    ZeroRequestSequence,
    #[error("fixture request sequence must advance within its exact role instance")]
    RequestSequenceDidNotAdvance,
    #[error("fixture request connection epoch moved backwards within its exact role instance")]
    RequestConnectionEpochWentBackwards,
    #[error("fixture completion belongs to another connection epoch")]
    CompletionEpochMismatch,
    #[error("fixture completion snapshot revision differs from the assembled snapshot")]
    CompletionSnapshotMismatch,
    #[error("fixture envelope construction failed: {0}")]
    Envelope(#[from] EnvelopeError),
}

/// Non-clone, role-instance-bound delivery of one complete fixture aggregate.
pub struct PmFixtureAggregateDelivery<P> {
    owner_id: PmFixtureOwnerId,
    account_scope: PmAccountScope,
    instrument: PmFixtureInstrumentScope,
    envelope: ReceivedEventEnvelope<P>,
}

impl<P> fmt::Debug for PmFixtureAggregateDelivery<P> {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("PmFixtureAggregateDelivery")
            .field("owner_id", &self.owner_id)
            .field("account_scope", &self.account_scope)
            .field("instrument", &self.instrument)
            .field("source", &self.envelope.source())
            .field("connection", &self.envelope.connection_id())
            .field("ordering", &self.envelope.ordering())
            .finish_non_exhaustive()
    }
}

impl<P> PmFixtureAggregateDelivery<P> {
    #[must_use]
    pub const fn account_scope(&self) -> PmAccountScope {
        self.account_scope
    }

    #[must_use]
    pub const fn instrument_scope(&self) -> PmFixtureInstrumentScope {
        self.instrument
    }

    #[must_use]
    pub const fn source(&self) -> PmProductSource {
        self.envelope.source()
    }

    #[must_use]
    pub const fn connection(&self) -> PmConnectionId {
        self.envelope.connection_id()
    }

    #[must_use]
    pub const fn ordering(&self) -> EventOrdering {
        self.envelope.ordering()
    }

    /// Receive-time evidence retained by this exact owner-bound delivery.
    ///
    /// This is an observation-only projection used by the product scheduler
    /// to derive its ordering key before the role instance opens the delivery
    /// at service time.
    #[must_use]
    pub const fn received_clock(&self) -> ReceivedEventClock {
        self.envelope.received_clock()
    }

    pub fn service_at(
        self,
        monotonic_service_ns: u64,
    ) -> Result<PmFixtureServicedAggregate<P>, EnvelopeError> {
        Ok(PmFixtureServicedAggregate {
            owner_id: self.owner_id,
            account_scope: self.account_scope,
            instrument: self.instrument,
            envelope: self.envelope.service_at(monotonic_service_ns)?,
        })
    }
}

/// Serviced non-clone authority retaining the exact fixture role instance.
pub struct PmFixtureServicedAggregate<P> {
    owner_id: PmFixtureOwnerId,
    account_scope: PmAccountScope,
    instrument: PmFixtureInstrumentScope,
    envelope: EventEnvelope<P>,
}

impl<P> fmt::Debug for PmFixtureServicedAggregate<P> {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("PmFixtureServicedAggregate")
            .field("owner_id", &self.owner_id)
            .field("account_scope", &self.account_scope)
            .field("instrument", &self.instrument)
            .field("source", &self.envelope.source())
            .field("connection", &self.envelope.connection_id())
            .field("ordering", &self.envelope.ordering())
            .finish_non_exhaustive()
    }
}

impl<P> PmFixtureServicedAggregate<P> {
    #[must_use]
    pub const fn account_scope(&self) -> PmAccountScope {
        self.account_scope
    }

    #[must_use]
    pub const fn instrument_scope(&self) -> PmFixtureInstrumentScope {
        self.instrument
    }

    #[must_use]
    pub const fn source(&self) -> PmProductSource {
        self.envelope.source()
    }

    #[must_use]
    pub const fn connection(&self) -> PmConnectionId {
        self.envelope.connection_id()
    }

    #[must_use]
    pub const fn ordering(&self) -> EventOrdering {
        self.envelope.ordering()
    }

    pub(crate) fn reduce_with_owner<R>(
        self,
        expected_owner: PmFixtureOwnerId,
        reduce: impl FnOnce(PmFixtureDeliveryScope, EventEnvelope<P>) -> R,
    ) -> Result<R, Box<Self>> {
        if self.owner_id != expected_owner {
            return Err(Box::new(self));
        }
        let scope = PmFixtureDeliveryScope {
            account_scope: self.account_scope,
            instrument: self.instrument,
        };
        Ok(reduce(scope, self.envelope))
    }
}

pub(crate) fn validate_next_request(
    previous: Option<(ConnectionEpoch, IngressSequence)>,
    connection_epoch: ConnectionEpoch,
    sequence: IngressSequence,
) -> Result<PmFixtureRequestOccurrence, PmFixtureDeliveryError> {
    let request = PmFixtureRequestOccurrence::new(connection_epoch, sequence)?;
    if let Some((previous_epoch, previous_sequence)) = previous {
        if connection_epoch < previous_epoch {
            return Err(PmFixtureDeliveryError::RequestConnectionEpochWentBackwards);
        }
        if connection_epoch == previous_epoch && sequence <= previous_sequence {
            return Err(PmFixtureDeliveryError::RequestSequenceDidNotAdvance);
        }
    }
    Ok(request)
}

pub(crate) fn validate_completion(
    request: &PmFixtureRequestOccurrence,
    completion: &PmFixtureCompletionOccurrence,
    snapshot_revision: SnapshotRevision,
) -> Result<IngressSequence, PmFixtureDeliveryError> {
    let ordering = completion.ordering();
    if ordering.connection_epoch() != request.connection_epoch() {
        return Err(PmFixtureDeliveryError::CompletionEpochMismatch);
    }
    if ordering.snapshot_revision() != Some(snapshot_revision) {
        return Err(PmFixtureDeliveryError::CompletionSnapshotMismatch);
    }
    Ok(ordering.local_ingress_sequence())
}

#[allow(clippy::too_many_arguments)]
pub(crate) fn checked_delivery<P: PmSourceBound>(
    owner_id: PmFixtureOwnerId,
    account_scope: PmAccountScope,
    instrument: PmFixtureInstrumentScope,
    source: PmProductSource,
    connection: PmConnectionId,
    completion: PmFixtureCompletionOccurrence,
    payload: P,
) -> Result<PmFixtureAggregateDelivery<P>, PmFixtureDeliveryError> {
    let envelope = ReceivedEventEnvelope::new(
        source.venue(),
        source,
        connection,
        completion.received_clock,
        completion.ordering,
        payload,
    )?;
    Ok(PmFixtureAggregateDelivery {
        owner_id,
        account_scope,
        instrument,
        envelope,
    })
}
