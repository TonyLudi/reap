use reap_core::Venue;
use reap_pm_core::{
    ConnectionEpoch, EnvelopeError, EventClock, EventEnvelope, EventOrdering, IngressSequence,
    OkxReferenceHandle, PmAccountHandle, PmConnectionId, PmProductSource, PmSourceBound,
    PmSourceHandle, PmTokenHandle, SnapshotRevision, VenueEventHash,
};

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
struct Payload {
    source: PmProductSource,
    value: u32,
}

impl PmSourceBound for Payload {
    fn source(&self) -> PmProductSource {
        self.source
    }
}

fn clock() -> EventClock {
    EventClock::new(Some(10), 20, 30, 35).unwrap()
}

fn ordering() -> EventOrdering {
    EventOrdering::new(
        ConnectionEpoch::new(2),
        Some(SnapshotRevision::new(3)),
        Some(4),
        Some(VenueEventHash::new([5; 32]).unwrap()),
        IngressSequence::new(6),
    )
    .unwrap()
}

fn pm_source() -> PmProductSource {
    PmProductSource::polymarket_market(
        PmSourceHandle::from_ordinal(7),
        PmTokenHandle::from_ordinal(8),
    )
}

#[test]
fn envelope_retains_concrete_payload_and_all_distinct_ordering_facts() {
    let envelope = EventEnvelope::new(
        Venue::Polymarket,
        pm_source(),
        PmConnectionId::new("pm-public-1").unwrap(),
        clock(),
        ordering(),
        Payload {
            source: pm_source(),
            value: 9,
        },
    )
    .unwrap();

    assert_eq!(envelope.venue(), Venue::Polymarket);
    assert_eq!(envelope.source(), pm_source());
    assert_eq!(envelope.connection_id().as_str(), "pm-public-1");
    assert_eq!(envelope.clock().venue_event_timestamp_ns(), Some(10));
    assert_eq!(envelope.clock().local_wall_receive_ns(), 20);
    assert_eq!(envelope.clock().monotonic_receive_ns(), 30);
    assert_eq!(envelope.clock().monotonic_service_ns(), 35);
    assert_eq!(envelope.clock().queue_age_ns(), 5);
    assert_eq!(envelope.ordering().connection_epoch().value(), 2);
    assert_eq!(envelope.ordering().snapshot_revision().unwrap().value(), 3);
    assert_eq!(envelope.ordering().venue_sequence(), Some(4));
    assert_eq!(envelope.ordering().venue_hash().unwrap().bytes(), [5; 32]);
    assert_eq!(envelope.ordering().local_ingress_sequence().value(), 6);
    assert_eq!(
        *envelope.payload(),
        Payload {
            source: pm_source(),
            value: 9,
        }
    );

    assert_eq!(
        envelope.into_payload(),
        Payload {
            source: pm_source(),
            value: 9,
        }
    );
}

#[test]
fn envelope_rejects_polymarket_and_okx_source_mismatches() {
    let pm_as_okx = EventEnvelope::new(
        Venue::Okx,
        pm_source(),
        PmConnectionId::new("pm-public-1").unwrap(),
        clock(),
        ordering(),
        Payload {
            source: pm_source(),
            value: 0,
        },
    );
    assert_eq!(
        pm_as_okx,
        Err(EnvelopeError::VenueSourceMismatch {
            venue: Venue::Okx,
            source_venue: Venue::Polymarket,
        })
    );

    let okx_source = PmProductSource::okx_reference(
        PmSourceHandle::from_ordinal(1),
        OkxReferenceHandle::from_ordinal(2),
    );
    let okx_as_pm = EventEnvelope::new(
        Venue::Polymarket,
        okx_source,
        PmConnectionId::new("okx-public-1").unwrap(),
        clock(),
        ordering(),
        Payload {
            source: okx_source,
            value: 0,
        },
    );
    assert_eq!(
        okx_as_pm,
        Err(EnvelopeError::VenueSourceMismatch {
            venue: Venue::Polymarket,
            source_venue: Venue::Okx,
        })
    );
}

#[test]
fn envelope_rejects_same_venue_payload_source_mismatches() {
    let other_market_source = PmProductSource::polymarket_market(
        PmSourceHandle::from_ordinal(7),
        PmTokenHandle::from_ordinal(99),
    );
    let token_mismatch = EventEnvelope::new(
        Venue::Polymarket,
        pm_source(),
        PmConnectionId::new("pm-public-1").unwrap(),
        clock(),
        ordering(),
        Payload {
            source: other_market_source,
            value: 1,
        },
    );
    assert_eq!(
        token_mismatch,
        Err(EnvelopeError::PayloadSourceMismatch {
            envelope_source: pm_source(),
            payload_source: other_market_source,
        })
    );

    let account_source = PmProductSource::polymarket_account(
        PmSourceHandle::from_ordinal(8),
        PmAccountHandle::from_ordinal(3),
    );
    let other_account_source = PmProductSource::polymarket_account(
        PmSourceHandle::from_ordinal(8),
        PmAccountHandle::from_ordinal(4),
    );
    let account_mismatch = EventEnvelope::new(
        Venue::Polymarket,
        account_source,
        PmConnectionId::new("pm-account-1").unwrap(),
        clock(),
        ordering(),
        Payload {
            source: other_account_source,
            value: 2,
        },
    );
    assert_eq!(
        account_mismatch,
        Err(EnvelopeError::PayloadSourceMismatch {
            envelope_source: account_source,
            payload_source: other_account_source,
        })
    );
}

#[test]
fn clocks_reject_zero_and_backwards_ordering() {
    assert_eq!(
        EventClock::new(Some(0), 2, 3, 4),
        Err(EnvelopeError::ZeroVenueTimestamp)
    );
    assert_eq!(
        EventClock::new(None, 0, 3, 4),
        Err(EnvelopeError::ZeroWallReceiveTimestamp)
    );
    assert_eq!(
        EventClock::new(None, 2, 0, 4),
        Err(EnvelopeError::ZeroMonotonicReceiveTimestamp)
    );
    assert_eq!(
        EventClock::new(None, 2, 3, 0),
        Err(EnvelopeError::ZeroMonotonicServiceTimestamp)
    );
    assert_eq!(
        EventClock::new(None, 2, 4, 3),
        Err(EnvelopeError::ServiceBeforeReceive)
    );
    assert!(EventClock::new(None, 2, 3, 3).is_ok());
}

#[test]
fn ordering_rejects_zero_identifiers_without_synthesizing_a_predecessor() {
    assert_eq!(
        EventOrdering::new(
            ConnectionEpoch::new(0),
            None,
            None,
            None,
            IngressSequence::new(1),
        ),
        Err(EnvelopeError::ZeroConnectionEpoch)
    );
    assert_eq!(
        EventOrdering::new(
            ConnectionEpoch::new(1),
            Some(SnapshotRevision::new(0)),
            None,
            None,
            IngressSequence::new(1),
        ),
        Err(EnvelopeError::ZeroSnapshotRevision)
    );
    assert_eq!(
        EventOrdering::new(
            ConnectionEpoch::new(1),
            None,
            Some(0),
            None,
            IngressSequence::new(1),
        ),
        Err(EnvelopeError::ZeroVenueSequence)
    );
    assert_eq!(
        EventOrdering::new(
            ConnectionEpoch::new(1),
            None,
            None,
            None,
            IngressSequence::new(0),
        ),
        Err(EnvelopeError::ZeroIngressSequence)
    );
    assert_eq!(
        VenueEventHash::new([0; 32]),
        Err(EnvelopeError::ZeroVenueHash)
    );

    let pm_without_venue_sequence = EventOrdering::new(
        ConnectionEpoch::new(1),
        Some(SnapshotRevision::new(2)),
        None,
        None,
        IngressSequence::new(3),
    )
    .unwrap();
    assert_eq!(pm_without_venue_sequence.venue_sequence(), None);
    assert_eq!(
        pm_without_venue_sequence.local_ingress_sequence().value(),
        3
    );
}
