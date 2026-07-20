use std::error::Error;
use std::fmt;

use reap_core::Venue;

use crate::identity::{
    ConnectionEpoch, IngressSequence, PmConnectionId, PmProductSource, PmSourceBound,
    SnapshotRevision,
};

/// A venue-supplied integrity hash, retained as exact bytes when one exists.
///
/// Absence is represented by `None` in [`EventOrdering`]. The all-zero value
/// is not a venue hash and is rejected instead of becoming a sentinel.
#[repr(transparent)]
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct VenueEventHash([u8; 32]);

impl VenueEventHash {
    pub fn new(bytes: [u8; 32]) -> Result<Self, EnvelopeError> {
        if bytes == [0; 32] {
            Err(EnvelopeError::ZeroVenueHash)
        } else {
            Ok(Self(bytes))
        }
    }

    #[must_use]
    pub const fn bytes(self) -> [u8; 32] {
        self.0
    }
}

/// Distinct wall, venue, receive, and service clocks for one input.
///
/// Venue time is optional because not every source supplies it. Wall time is
/// evidence only. The monotonic pair is the only pair used to measure queue
/// age, and service time may equal but cannot precede receive time.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct EventClock {
    venue_event_timestamp_ns: Option<u64>,
    local_wall_receive_ns: u64,
    monotonic_receive_ns: u64,
    monotonic_service_ns: u64,
}

impl EventClock {
    pub fn new(
        venue_event_timestamp_ns: Option<u64>,
        local_wall_receive_ns: u64,
        monotonic_receive_ns: u64,
        monotonic_service_ns: u64,
    ) -> Result<Self, EnvelopeError> {
        if venue_event_timestamp_ns == Some(0) {
            return Err(EnvelopeError::ZeroVenueTimestamp);
        }
        if local_wall_receive_ns == 0 {
            return Err(EnvelopeError::ZeroWallReceiveTimestamp);
        }
        if monotonic_receive_ns == 0 {
            return Err(EnvelopeError::ZeroMonotonicReceiveTimestamp);
        }
        if monotonic_service_ns == 0 {
            return Err(EnvelopeError::ZeroMonotonicServiceTimestamp);
        }
        if monotonic_service_ns < monotonic_receive_ns {
            return Err(EnvelopeError::ServiceBeforeReceive);
        }
        Ok(Self {
            venue_event_timestamp_ns,
            local_wall_receive_ns,
            monotonic_receive_ns,
            monotonic_service_ns,
        })
    }

    #[must_use]
    pub const fn venue_event_timestamp_ns(self) -> Option<u64> {
        self.venue_event_timestamp_ns
    }

    #[must_use]
    pub const fn local_wall_receive_ns(self) -> u64 {
        self.local_wall_receive_ns
    }

    #[must_use]
    pub const fn monotonic_receive_ns(self) -> u64 {
        self.monotonic_receive_ns
    }

    #[must_use]
    pub const fn monotonic_service_ns(self) -> u64 {
        self.monotonic_service_ns
    }

    #[must_use]
    pub const fn queue_age_ns(self) -> u64 {
        self.monotonic_service_ns - self.monotonic_receive_ns
    }
}

/// Ordering and integrity facts retained without inventing venue sequencing.
///
/// `venue_sequence` and `venue_hash` are optional because they are populated
/// only when the source actually supplies those facts. In particular, local
/// ingress order is a separate field and is never exposed as a PM predecessor
/// sequence.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct EventOrdering {
    connection_epoch: ConnectionEpoch,
    snapshot_revision: Option<SnapshotRevision>,
    venue_sequence: Option<u64>,
    venue_hash: Option<VenueEventHash>,
    local_ingress_sequence: IngressSequence,
}

impl EventOrdering {
    pub fn new(
        connection_epoch: ConnectionEpoch,
        snapshot_revision: Option<SnapshotRevision>,
        venue_sequence: Option<u64>,
        venue_hash: Option<VenueEventHash>,
        local_ingress_sequence: IngressSequence,
    ) -> Result<Self, EnvelopeError> {
        if connection_epoch.value() == 0 {
            return Err(EnvelopeError::ZeroConnectionEpoch);
        }
        if snapshot_revision.is_some_and(|revision| revision.value() == 0) {
            return Err(EnvelopeError::ZeroSnapshotRevision);
        }
        if venue_sequence == Some(0) {
            return Err(EnvelopeError::ZeroVenueSequence);
        }
        if local_ingress_sequence.value() == 0 {
            return Err(EnvelopeError::ZeroIngressSequence);
        }
        Ok(Self {
            connection_epoch,
            snapshot_revision,
            venue_sequence,
            venue_hash,
            local_ingress_sequence,
        })
    }

    #[must_use]
    pub const fn connection_epoch(self) -> ConnectionEpoch {
        self.connection_epoch
    }

    #[must_use]
    pub const fn snapshot_revision(self) -> Option<SnapshotRevision> {
        self.snapshot_revision
    }

    #[must_use]
    pub const fn venue_sequence(self) -> Option<u64> {
        self.venue_sequence
    }

    #[must_use]
    pub const fn venue_hash(self) -> Option<VenueEventHash> {
        self.venue_hash
    }

    #[must_use]
    pub const fn local_ingress_sequence(self) -> IngressSequence {
        self.local_ingress_sequence
    }
}

/// A concrete normalized payload plus immutable source and ordering evidence.
///
/// The payload remains a static type parameter; this layer does not erase it
/// behind a runtime interface. The duplicated venue field is intentional:
/// construction proves that an external venue discriminator agrees with the
/// configured product source.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct EventEnvelope<P> {
    venue: Venue,
    source: PmProductSource,
    connection_id: PmConnectionId,
    clock: EventClock,
    ordering: EventOrdering,
    payload: P,
}

impl<P: PmSourceBound> EventEnvelope<P> {
    pub fn new(
        venue: Venue,
        source: PmProductSource,
        connection_id: PmConnectionId,
        clock: EventClock,
        ordering: EventOrdering,
        payload: P,
    ) -> Result<Self, EnvelopeError> {
        let source_venue = source.venue();
        if venue != source_venue {
            return Err(EnvelopeError::VenueSourceMismatch {
                venue,
                source_venue,
            });
        }
        let payload_source = payload.source();
        if source != payload_source {
            return Err(EnvelopeError::PayloadSourceMismatch {
                envelope_source: source,
                payload_source,
            });
        }
        Ok(Self {
            venue,
            source,
            connection_id,
            clock,
            ordering,
            payload,
        })
    }
}

impl<P> EventEnvelope<P> {
    #[must_use]
    pub const fn venue(&self) -> Venue {
        self.venue
    }

    #[must_use]
    pub const fn source(&self) -> PmProductSource {
        self.source
    }

    #[must_use]
    pub const fn connection_id(&self) -> PmConnectionId {
        self.connection_id
    }

    #[must_use]
    pub const fn clock(&self) -> EventClock {
        self.clock
    }

    #[must_use]
    pub const fn ordering(&self) -> EventOrdering {
        self.ordering
    }

    #[must_use]
    pub const fn payload(&self) -> &P {
        &self.payload
    }

    #[must_use]
    pub fn into_payload(self) -> P {
        self.payload
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum EnvelopeError {
    VenueSourceMismatch {
        venue: Venue,
        source_venue: Venue,
    },
    PayloadSourceMismatch {
        envelope_source: PmProductSource,
        payload_source: PmProductSource,
    },
    ZeroVenueTimestamp,
    ZeroWallReceiveTimestamp,
    ZeroMonotonicReceiveTimestamp,
    ZeroMonotonicServiceTimestamp,
    ServiceBeforeReceive,
    ZeroConnectionEpoch,
    ZeroSnapshotRevision,
    ZeroVenueSequence,
    ZeroVenueHash,
    ZeroIngressSequence,
}

impl fmt::Display for EnvelopeError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::VenueSourceMismatch {
                venue,
                source_venue,
            } => write!(
                formatter,
                "event venue {venue:?} does not match source venue {source_venue:?}"
            ),
            Self::PayloadSourceMismatch {
                envelope_source,
                payload_source,
            } => write!(
                formatter,
                "envelope source {envelope_source:?} does not match payload source {payload_source:?}"
            ),
            Self::ZeroVenueTimestamp => {
                formatter.write_str("venue event timestamp must be nonzero when present")
            }
            Self::ZeroWallReceiveTimestamp => {
                formatter.write_str("local wall receive timestamp must be nonzero")
            }
            Self::ZeroMonotonicReceiveTimestamp => {
                formatter.write_str("monotonic receive timestamp must be nonzero")
            }
            Self::ZeroMonotonicServiceTimestamp => {
                formatter.write_str("monotonic service timestamp must be nonzero")
            }
            Self::ServiceBeforeReceive => {
                formatter.write_str("monotonic service timestamp precedes receive timestamp")
            }
            Self::ZeroConnectionEpoch => formatter.write_str("connection epoch must be nonzero"),
            Self::ZeroSnapshotRevision => {
                formatter.write_str("snapshot revision must be nonzero when present")
            }
            Self::ZeroVenueSequence => {
                formatter.write_str("venue sequence must be nonzero when present")
            }
            Self::ZeroVenueHash => formatter.write_str("venue event hash must not be all zeroes"),
            Self::ZeroIngressSequence => {
                formatter.write_str("local ingress sequence must be nonzero")
            }
        }
    }
}

impl Error for EnvelopeError {}
