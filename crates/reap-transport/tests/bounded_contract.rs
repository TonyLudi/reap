use reap_core::{Channel, ConnId, RawEnvelope, Venue};
use reap_transport::{DeliveryClockError, ImmutableDelivery, RawDelivery, bounded_channel};

fn raw() -> RawEnvelope {
    RawEnvelope {
        venue: Venue::Okx,
        conn_id: ConnId::new("public"),
        channel: Channel::Books,
        symbol: Some("BTC-USDT".to_string()),
        recv_ts_ns: 7,
        raw_hash: 9,
        payload: "{}".to_string(),
    }
}

#[tokio::test]
async fn legacy_requested_capacity_is_finite_and_zero_clamps_to_one() {
    let (sender, mut receiver) = bounded_channel(0);

    sender.try_send(1_u8).unwrap();
    assert!(sender.try_send(2_u8).is_err());
    assert_eq!(receiver.recv().await, Some(1));
}

#[test]
fn immutable_delivery_keeps_wall_payload_separate_from_monotonic_queue_age() {
    let delivery: RawDelivery = ImmutableDelivery::new(raw(), 100).unwrap();

    assert_eq!(delivery.monotonic_receive_ns(), 100);
    assert_eq!(delivery.queue_age_ns(125).unwrap(), 25);
    assert_eq!(delivery.payload().recv_ts_ns, 7);
    assert_eq!(delivery.into_payload().raw_hash, 9);
}

#[test]
fn fallible_mapping_cannot_detach_receive_evidence() {
    let delivery = ImmutableDelivery::new("17", 100).unwrap();
    let mapped = delivery
        .try_map(|value| value.parse::<u64>())
        .expect("valid mapped payload");

    assert_eq!(mapped.payload(), &17);
    assert_eq!(mapped.monotonic_receive_ns(), 100);
}

#[test]
fn immutable_delivery_rejects_invalid_monotonic_clocks() {
    assert_eq!(
        ImmutableDelivery::new((), 0),
        Err(DeliveryClockError::ZeroMonotonicReceive)
    );
    let delivery = ImmutableDelivery::new((), 10).unwrap();
    assert_eq!(
        delivery.queue_age_ns(9),
        Err(DeliveryClockError::ServiceBeforeReceive)
    );
}
