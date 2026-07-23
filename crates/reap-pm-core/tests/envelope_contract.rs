use reap_core::Venue;
use reap_pm_core::{
    ConnectionEpoch, EnvelopeError, EventClock, EventEnvelope, EventOrdering, IngressSequence,
    OkxReferenceHandle, PmAccountHandle, PmConnectionId, PmProductSource, PmSourceBound,
    PmSourceHandle, PmTokenHandle, ReceivedEventClock, ReceivedEventEnvelope, SnapshotRevision,
    VenueEventHash, VenueEventHashAlgorithm,
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
        Some(VenueEventHash::sha256([5; 32]).unwrap()),
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
    let event_hash = envelope.ordering().venue_hash().unwrap();
    assert_eq!(event_hash.algorithm(), VenueEventHashAlgorithm::Sha256);
    assert_eq!(event_hash.as_bytes(), &[5; 32]);
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
fn received_envelope_adds_service_time_only_when_the_consumer_services_it() {
    let payload = Payload {
        source: pm_source(),
        value: 12,
    };
    let received = ReceivedEventEnvelope::new(
        Venue::Polymarket,
        pm_source(),
        PmConnectionId::new("pm-public-1").unwrap(),
        ReceivedEventClock::new(Some(10), 20, 30).unwrap(),
        ordering(),
        payload,
    )
    .unwrap();

    assert_eq!(received.received_clock().monotonic_receive_ns(), 30);
    assert_eq!(received.ordering(), ordering());
    assert_eq!(received.payload(), &payload);

    let serviced = received.service_at(45).unwrap();
    assert_eq!(serviced.clock().monotonic_service_ns(), 45);
    assert_eq!(serviced.clock().queue_age_ns(), 15);
    assert_eq!(serviced.ordering(), ordering());
    assert_eq!(serviced.into_payload(), payload);
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
fn ingress_clock_cannot_claim_service_time_before_dequeue() {
    let received = ReceivedEventClock::new(Some(10), 20, 30).unwrap();
    assert_eq!(received.venue_event_timestamp_ns(), Some(10));
    assert_eq!(received.local_wall_receive_ns(), 20);
    assert_eq!(received.monotonic_receive_ns(), 30);
    assert_eq!(
        received.service_at(29),
        Err(EnvelopeError::ServiceBeforeReceive)
    );
    assert_eq!(
        received.service_at(0),
        Err(EnvelopeError::ZeroMonotonicServiceTimestamp)
    );
    let serviced = received.service_at(45).unwrap();
    assert_eq!(serviced.monotonic_service_ns(), 45);
    assert_eq!(serviced.queue_age_ns(), 15);
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
        VenueEventHash::sha256([0; 32]),
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

#[test]
fn venue_hash_retains_exact_algorithm_and_length_without_padding() {
    let sha1 = VenueEventHash::sha1([0x45; 20]).unwrap();
    assert_eq!(sha1.algorithm(), VenueEventHashAlgorithm::Sha1);
    assert_eq!(sha1.len(), 20);
    assert_eq!(sha1.as_bytes(), &[0x45; 20]);

    let opaque = VenueEventHash::opaque(&[1, 2, 3, 4]).unwrap();
    assert_eq!(opaque.algorithm(), VenueEventHashAlgorithm::Opaque);
    assert_eq!(opaque.len(), 4);
    assert_eq!(opaque.as_bytes(), &[1, 2, 3, 4]);

    assert_eq!(
        VenueEventHash::opaque(&[]),
        Err(EnvelopeError::EmptyVenueHash)
    );
    assert_eq!(
        VenueEventHash::opaque(&[1; 65]),
        Err(EnvelopeError::VenueHashTooLong)
    );
}
