use reap_core::{
    Channel, ConnId, FeedPriority, RawEnvelope, Subscription, SystemEvent, SystemEventKind, Venue,
};

#[test]
fn existing_okx_venue_and_envelope_bytes_are_unchanged() {
    assert_eq!(serde_json::to_vec(&Venue::Okx).unwrap(), br#""okx""#);

    let subscription = Subscription::public(
        Venue::Okx,
        Channel::Books,
        "BTC-USDT",
        FeedPriority::Critical,
    );
    let subscription_bytes = br#"{"venue":"okx","channel":"books","symbol":"BTC-USDT","priority":"critical","connections":1}"#;
    assert_eq!(
        serde_json::to_vec(&subscription).unwrap(),
        subscription_bytes
    );
    let decoded: Subscription = serde_json::from_slice(subscription_bytes).unwrap();
    assert_eq!(serde_json::to_vec(&decoded).unwrap(), subscription_bytes);

    let envelope = RawEnvelope {
        venue: Venue::Okx,
        conn_id: ConnId::new("public-1"),
        channel: Channel::Books,
        symbol: Some("BTC-USDT".to_string()),
        recv_ts_ns: 123_456_789,
        raw_hash: 42,
        payload: r#"{"arg":{"channel":"books","instId":"BTC-USDT"}}"#.to_string(),
    };
    let envelope_bytes = br#"{"venue":"okx","conn_id":"public-1","channel":"books","symbol":"BTC-USDT","recv_ts_ns":123456789,"raw_hash":42,"payload":"{\"arg\":{\"channel\":\"books\",\"instId\":\"BTC-USDT\"}}"}"#;
    assert_eq!(serde_json::to_vec(&envelope).unwrap(), envelope_bytes);
    let decoded: RawEnvelope = serde_json::from_slice(envelope_bytes).unwrap();
    assert_eq!(serde_json::to_vec(&decoded).unwrap(), envelope_bytes);
}

#[test]
fn existing_okx_system_event_bytes_are_unchanged() {
    let event = SystemEvent {
        ts_ms: 1_700_000_000_000,
        kind: SystemEventKind::FeedStale,
        venue: Some(Venue::Okx),
        account_id: None,
        symbol: Some("BTC-USDT".to_string()),
        reason: "fixture".to_string(),
    };
    let expected = br#"{"ts_ms":1700000000000,"kind":"feed_stale","venue":"okx","account_id":null,"symbol":"BTC-USDT","reason":"fixture"}"#;
    assert_eq!(serde_json::to_vec(&event).unwrap(), expected);
    let decoded: SystemEvent = serde_json::from_slice(expected).unwrap();
    assert_eq!(serde_json::to_vec(&decoded).unwrap(), expected);
}

#[test]
fn polymarket_has_a_distinct_stable_wire_identity() {
    assert_eq!(
        serde_json::to_vec(&Venue::Polymarket).unwrap(),
        br#""polymarket""#
    );
}
