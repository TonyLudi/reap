use std::time::Duration;

use reap_core::{Channel, ConnId, EventKey, OkxVenue, RawEnvelope};
use reap_okx_public_source::{
    MAX_OKX_PUBLIC_CONNECTION_ID_BYTES, OkxPublicEventEvidence, OkxPublicSession,
    OkxPublicSessionError, OkxPublicSessionEvent, OkxPublicSessionFault,
};
use reap_transport::{ConnectionStatusKind, RawDelivery, ReconnectPolicy};

fn envelope(payload: &str) -> RawEnvelope {
    RawEnvelope {
        venue: OkxVenue,
        conn_id: ConnId::new("okx-public-reference-1"),
        channel: Channel::Custom("index-tickers".to_string()),
        symbol: Some("BTC-USDT".to_string()),
        recv_ts_ns: 1_700_000_000_123_456_789,
        raw_hash: 0xfeed_beef,
        payload: payload.to_string(),
    }
}

fn new_session() -> OkxPublicSession {
    OkxPublicSession::new(
        "BTC-USDT",
        ConnId::new("okx-public-reference-1"),
        7,
        reconnect_policy(),
    )
    .unwrap()
}

fn reconnect_policy() -> ReconnectPolicy {
    ReconnectPolicy {
        initial_delay: Duration::from_millis(10),
        max_delay: Duration::from_millis(40),
        multiplier: 2,
    }
}

fn delivery(payload: &str, monotonic_receive_ns: u64) -> RawDelivery {
    RawDelivery::new(envelope(payload), monotonic_receive_ns).unwrap()
}

fn assert_protocol_evidence(evidence: &OkxPublicEventEvidence, expected_epoch: u64) {
    assert_eq!(evidence.wall_receive_ts_ns(), 1_700_000_000_123_456_789);
    assert_eq!(evidence.connection_epoch(), expected_epoch);
    assert_eq!(evidence.raw_hash(), 0xfeed_beef);
}

fn acknowledge(session: &mut OkxPublicSession) {
    let expected_epoch = session.connection_epoch();
    let classified = session
        .classify(delivery(
            r#"{"event":"subscribe","arg":{"channel":"index-tickers","instId":"BTC-USDT"}}"#,
            90,
        ))
        .unwrap();
    let OkxPublicSessionEvent::SubscriptionAcknowledged(evidence) = classified.payload() else {
        panic!("expected exact subscription acknowledgement");
    };
    assert_protocol_evidence(evidence, expected_epoch);
    assert_eq!(classified.monotonic_receive_ns(), 90);
}

fn consume_unavailable_and_reconnect(session: &mut OkxPublicSession) -> Duration {
    assert!(matches!(
        session.after_failure(),
        Err(OkxPublicSessionError::UnavailableOccurrencePending)
    ));
    let occurrence = session
        .take_unavailable()
        .expect("invalid protocol input emits one unavailable occurrence");
    assert_eq!(occurrence.connection_id(), "okx-public-reference-1");
    assert_eq!(occurrence.connection_epoch(), session.connection_epoch());
    assert_eq!(occurrence.fault(), OkxPublicSessionFault::InvalidTransition);
    assert!(occurrence.wall_receive_ts_ns() > 0);
    assert!(occurrence.monotonic_receive_ns() > 0);
    assert!(occurrence.local_ingress_sequence() > 0);
    assert_eq!(session.take_unavailable(), None);
    session.after_failure().unwrap()
}

#[test]
fn acknowledgement_control_and_heartbeat_are_session_owned() {
    let mut session = new_session();
    assert_eq!(session.health(), ConnectionStatusKind::Disconnected);

    let heartbeat = session.classify(delivery("pong", 89)).unwrap();
    let OkxPublicSessionEvent::Heartbeat(evidence) = heartbeat.payload() else {
        panic!("expected heartbeat");
    };
    assert_protocol_evidence(evidence, 7);
    assert_eq!(heartbeat.monotonic_receive_ns(), 89);
    assert_eq!(
        session.health(),
        ConnectionStatusKind::Disconnected,
        "pre-ack heartbeat is not subscription readiness"
    );
    assert!(!session.subscription_ready());

    acknowledge(&mut session);
    assert_eq!(session.health(), ConnectionStatusKind::Ready);
    assert!(session.subscription_ready());

    let control = session
        .classify(delivery(
            r#"{"event":"channel-conn-count","channel":"index-tickers","connCount":"1","connId":"one"}"#,
            91,
        ))
        .unwrap();
    let OkxPublicSessionEvent::Control(control_evidence) = control.payload() else {
        panic!("expected allowlisted control");
    };
    assert_protocol_evidence(control_evidence.source(), 7);
    assert_eq!(control_evidence.connection_count(), 1);
    assert_eq!(control_evidence.connection_id(), "one");
    assert_eq!(control.monotonic_receive_ns(), 91);

    let heartbeat = session.classify(delivery("pong", 92)).unwrap();
    let OkxPublicSessionEvent::Heartbeat(evidence) = heartbeat.payload() else {
        panic!("expected heartbeat");
    };
    assert_protocol_evidence(evidence, 7);
    assert_eq!(heartbeat.monotonic_receive_ns(), 92);
    assert_eq!(
        session.health(),
        ConnectionStatusKind::Ready,
        "heartbeat must not erase exact subscription readiness"
    );
    assert!(session.subscription_ready());

    let error = session
        .classify(delivery(
            r#"{"event":"error","code":"60012","msg":"bad request"}"#,
            93,
        ))
        .unwrap_err();
    assert!(matches!(error, OkxPublicSessionError::ServerControl { .. }));
    assert!(!session.subscription_ready());
    assert_eq!(session.health(), ConnectionStatusKind::Disconnected);
}

#[test]
fn construction_binds_one_validated_connection_identity() {
    let session = new_session();
    assert_eq!(
        session.connection_id(),
        &ConnId::new("okx-public-reference-1")
    );

    for (connection_id, expected) in [
        (ConnId::new(""), OkxPublicSessionError::EmptyConnectionId),
        (
            ConnId::new("A".repeat(MAX_OKX_PUBLIC_CONNECTION_ID_BYTES + 1)),
            OkxPublicSessionError::ConnectionIdTooLong,
        ),
        (
            ConnId::new("contains space"),
            OkxPublicSessionError::InvalidConnectionId,
        ),
        (
            ConnId::new("contains\"quote"),
            OkxPublicSessionError::InvalidConnectionId,
        ),
    ] {
        let error =
            OkxPublicSession::new("BTC-USDT", connection_id, 7, reconnect_policy()).unwrap_err();
        assert_eq!(
            std::mem::discriminant(&error),
            std::mem::discriminant(&expected)
        );
    }

    assert!(matches!(
        OkxPublicSession::new(
            "BTC-USDT",
            ConnId::new("okx-public-reference-1"),
            0,
            reconnect_policy(),
        ),
        Err(OkxPublicSessionError::ZeroConnectionEpoch)
    ));
    assert!(
        OkxPublicSession::new(
            "BTC-USDT",
            ConnId::new("A".repeat(MAX_OKX_PUBLIC_CONNECTION_ID_BYTES)),
            1,
            reconnect_policy(),
        )
        .is_ok()
    );
}

#[test]
fn envelope_connection_identity_mismatch_invalidates_readiness() {
    let mut session = new_session();
    acknowledge(&mut session);
    let mut wrong_connection = envelope("pong");
    wrong_connection.conn_id = ConnId::new("another-connection");

    assert!(matches!(
        session.classify(RawDelivery::new(wrong_connection, 94).unwrap()),
        Err(OkxPublicSessionError::WrongEnvelopeConnectionId)
    ));
    assert!(!session.subscription_ready());
    assert_eq!(session.health(), ConnectionStatusKind::Disconnected);
}

#[test]
fn data_requires_the_exact_acknowledged_session() {
    let payload = r#"{"arg":{"channel":"index-tickers","instId":"BTC-USDT"},"data":[{"instId":"BTC-USDT","idxPx":"50000","ts":"1700000000123"}]}"#;
    let mut session = new_session();

    assert!(matches!(
        session.classify(delivery(payload, 100)),
        Err(OkxPublicSessionError::DataBeforeAcknowledgement)
    ));
    assert!(matches!(
        session.classify(delivery(payload, 101)),
        Err(OkxPublicSessionError::ReconnectRequired)
    ));
    consume_unavailable_and_reconnect(&mut session);

    acknowledge(&mut session);
    assert!(matches!(
        session.classify(delivery(
            r#"{"event":"subscribe","arg":{"channel":"index-tickers","instId":"BTC-USDT"}}"#,
            101,
        )),
        Err(OkxPublicSessionError::UnexpectedAcknowledgement)
    ));

    for invalid_ack in [
        r#"{"event":"subscribe","arg":{"channel":"books","instId":"BTC-USDT"}}"#,
        r#"{"event":"subscribe","arg":{"channel":"index-tickers","instId":"ETH-USDT"}}"#,
        r#"{"event":"subscribe","code":"1","arg":{"channel":"index-tickers","instId":"BTC-USDT"}}"#,
        r#"{"event":"subscribe","code":null,"arg":{"channel":"index-tickers","instId":"BTC-USDT"}}"#,
    ] {
        let mut pending = new_session();
        assert!(matches!(
            pending.classify(delivery(invalid_ack, 102)),
            Err(OkxPublicSessionError::InvalidAcknowledgement)
        ));
    }
}

#[test]
fn reference_delivery_preserves_exact_lexical_and_source_evidence() {
    let mut session = new_session();
    acknowledge(&mut session);
    let payload = r#"{"arg":{"channel":"index-tickers","instId":"BTC-USDT"},"data":[{"instId":"BTC-USDT","idxPx":"00050000.125000","ts":"1700000000123"}]}"#;

    let classified = session.classify(delivery(payload, 123_456_789)).unwrap();
    let OkxPublicSessionEvent::Reference(reference) = classified.payload() else {
        panic!("expected one configured reference");
    };

    assert_eq!(classified.monotonic_receive_ns(), 123_456_789);
    assert_eq!(reference.instrument(), "BTC-USDT");
    assert_eq!(reference.index_price_lexeme(), "00050000.125000");
    assert_eq!(reference.venue_ts_ms(), 1_700_000_000_123);
    assert_eq!(reference.wall_receive_ts_ns(), 1_700_000_000_123_456_789);
    assert_eq!(reference.connection_epoch(), 7);
    assert_eq!(reference.raw_hash(), 0xfeed_beef);
    assert!(matches!(
        reference.event_key(),
        EventKey::TimestampHash {
            ts_ms: 1_700_000_000_123,
            raw_hash: 0xfeed_beef,
        }
    ));
}

#[test]
fn captured_text_api_reconstructs_the_exact_configured_route() {
    let mut session = new_session();
    let acknowledgement = session
        .classify_captured_payload(
            r#"{"event":"subscribe","arg":{"channel":"index-tickers","instId":"BTC-USDT"}}"#,
            1_700_000_000_123_456_789,
            90,
            0xfeed_beef,
        )
        .unwrap();
    let OkxPublicSessionEvent::SubscriptionAcknowledged(evidence) = acknowledgement.payload()
    else {
        panic!("configured acknowledgement");
    };
    assert_protocol_evidence(evidence, 7);
    assert_eq!(acknowledgement.connection_id(), "okx-public-reference-1");
    assert_eq!(acknowledgement.connection_epoch(), 7);

    let reference = session
        .classify_captured_payload(
            r#"{"arg":{"channel":"index-tickers","instId":"BTC-USDT"},"data":[{"instId":"BTC-USDT","idxPx":"00050000.125000","ts":"1700000000123"}]}"#,
            1_700_000_000_123_456_790,
            91,
            0x1234_5678,
        )
        .unwrap();
    let OkxPublicSessionEvent::Reference(reference) = reference.payload() else {
        panic!("configured reference");
    };
    assert_eq!(reference.instrument(), "BTC-USDT");
    assert_eq!(reference.wall_receive_ts_ns(), 1_700_000_000_123_456_790);
    assert_eq!(reference.connection_epoch(), 7);
    assert_eq!(reference.raw_hash(), 0x1234_5678);
}

#[test]
fn captured_text_wrong_scope_malformed_and_invalid_clock_fail_closed() {
    for payload in [
        "{",
        r#"{"arg":{"channel":"books","instId":"BTC-USDT"},"data":[{"instId":"BTC-USDT","idxPx":"1","ts":"1"}]}"#,
        r#"{"arg":{"channel":"index-tickers","instId":"ETH-USDT"},"data":[{"instId":"ETH-USDT","idxPx":"1","ts":"1"}]}"#,
    ] {
        let mut session = new_session();
        acknowledge(&mut session);
        assert!(
            session
                .classify_captured_payload(payload, 1_700_000_000_123_456_789, 100, 0xfeed_beef,)
                .is_err()
        );
        assert!(!session.subscription_ready());
        assert!(session.requires_reconnect());
        assert_eq!(session.health(), ConnectionStatusKind::Disconnected);
        consume_unavailable_and_reconnect(&mut session);
        assert!(!session.requires_reconnect());
        assert_eq!(session.last_fault(), None);
    }

    let mut invalid_clock = new_session();
    acknowledge(&mut invalid_clock);
    assert!(
        invalid_clock
            .classify_captured_payload("pong", 1_700_000_000_123_456_789, 0, 0xfeed_beef,)
            .is_err()
    );
    assert!(!invalid_clock.subscription_ready());
    assert!(invalid_clock.requires_reconnect());
    assert_eq!(invalid_clock.health(), ConnectionStatusKind::Disconnected);
}

#[test]
fn malformed_wrong_scope_and_nonpositive_data_fail_closed() {
    let mut session = new_session();
    acknowledge(&mut session);

    for payload in [
        "{",
        r#"{"arg":{"channel":"books","instId":"BTC-USDT"},"data":[{"instId":"BTC-USDT","idxPx":"1","ts":"1"}]}"#,
        r#"{"arg":{"channel":"index-tickers","instId":"ETH-USDT"},"data":[{"instId":"ETH-USDT","idxPx":"1","ts":"1"}]}"#,
        r#"{"arg":{"channel":"index-tickers","instId":"BTC-USDT"},"data":[{"instId":"ETH-USDT","idxPx":"1","ts":"1"}]}"#,
        r#"{"arg":{"channel":"index-tickers","instId":"BTC-USDT"},"data":[]}"#,
        r#"{"arg":{"channel":"index-tickers","instId":"BTC-USDT"},"data":[{"instId":"BTC-USDT","idxPx":"0","ts":"1"}]}"#,
        r#"{"arg":{"channel":"index-tickers","instId":"BTC-USDT"},"data":[{"instId":"BTC-USDT","idxPx":"-1","ts":"1"}]}"#,
        r#"{"arg":{"channel":"index-tickers","instId":"BTC-USDT"},"data":[{"instId":"BTC-USDT","idxPx":"1e2","ts":"1"}]}"#,
        r#"{"arg":{"channel":"index-tickers","instId":"BTC-USDT"},"data":[{"instId":"BTC-USDT","idxPx":".1","ts":"1"}]}"#,
        r#"{"arg":{"channel":"index-tickers","instId":"BTC-USDT"},"data":[{"instId":"BTC-USDT","idxPx":"1.","ts":"1"}]}"#,
        r#"{"arg":{"channel":"index-tickers","instId":"BTC-USDT"},"data":[{"instId":"BTC-USDT","idxPx":"1","ts":"not-a-time"}]}"#,
    ] {
        assert!(
            session.classify(delivery(payload, 200)).is_err(),
            "{payload}"
        );
        consume_unavailable_and_reconnect(&mut session);
        acknowledge(&mut session);
    }

    let oversized_price = "1".repeat(129);
    let oversized_payload = format!(
        r#"{{"arg":{{"channel":"index-tickers","instId":"BTC-USDT"}},"data":[{{"instId":"BTC-USDT","idxPx":"{oversized_price}","ts":"1"}}]}}"#
    );
    assert!(session.classify(delivery(&oversized_payload, 200)).is_err());
    consume_unavailable_and_reconnect(&mut session);
    acknowledge(&mut session);

    let mut wrong_channel = envelope(
        r#"{"arg":{"channel":"index-tickers","instId":"BTC-USDT"},"data":[{"instId":"BTC-USDT","idxPx":"1","ts":"1"}]}"#,
    );
    wrong_channel.channel = Channel::Books;
    assert!(matches!(
        session.classify(RawDelivery::new(wrong_channel, 201).unwrap()),
        Err(OkxPublicSessionError::WrongEnvelopeChannel)
    ));
    consume_unavailable_and_reconnect(&mut session);
    acknowledge(&mut session);

    let mut wrong_instrument = envelope(
        r#"{"arg":{"channel":"index-tickers","instId":"BTC-USDT"},"data":[{"instId":"BTC-USDT","idxPx":"1","ts":"1"}]}"#,
    );
    wrong_instrument.symbol = Some("ETH-USDT".to_string());
    assert!(matches!(
        session.classify(RawDelivery::new(wrong_instrument, 202).unwrap()),
        Err(OkxPublicSessionError::WrongEnvelopeInstrument)
    ));
}

#[test]
fn only_exact_non_mutating_control_is_allowed_and_every_rejection_clears_readiness() {
    let mut session = new_session();

    for payload in [
        r#"{"event":"unsubscribe","arg":{"channel":"index-tickers","instId":"BTC-USDT"}}"#,
        r#"{"event":"login"}"#,
        r#"{"event":"channel-conn-count","channel":"books","connCount":"1","connId":"one"}"#,
        r#"{"event":"channel-conn-count","channel":"index-tickers","connCount":"0","connId":"one"}"#,
        r#"{"event":"channel-conn-count","channel":"index-tickers","connCount":"1","connId":"one","unexpected":true}"#,
        r#"{"event":"subscribe","arg":{"channel":"index-tickers","instId":"BTC-USDT"},"op":"unsubscribe"}"#,
    ] {
        acknowledge(&mut session);
        assert!(session.subscription_ready());
        assert!(
            session.classify(delivery(payload, 300)).is_err(),
            "{payload}"
        );
        assert!(!session.subscription_ready(), "{payload}");
        assert_eq!(
            session.health(),
            ConnectionStatusKind::Disconnected,
            "{payload}"
        );
        consume_unavailable_and_reconnect(&mut session);
    }

    let data = r#"{"arg":{"channel":"index-tickers","instId":"BTC-USDT"},"data":[{"instId":"BTC-USDT","idxPx":"1","ts":"1"}]}"#;
    assert!(matches!(
        session.classify(delivery(data, 301)),
        Err(OkxPublicSessionError::DataBeforeAcknowledgement)
    ));
}

#[test]
fn reconnect_backoff_uses_only_ack_history_owned_by_the_session() {
    let mut session = new_session();

    assert_eq!(session.after_failure().unwrap(), Duration::from_millis(10));
    assert_eq!(session.connection_epoch(), 8);
    assert_eq!(session.after_failure().unwrap(), Duration::from_millis(20));
    assert_eq!(session.connection_epoch(), 9);

    acknowledge(&mut session);
    assert_eq!(
        session.after_failure().unwrap(),
        Duration::from_millis(10),
        "an exact ACK resets startup-failure history without caller input"
    );
    assert_eq!(session.connection_epoch(), 10);

    acknowledge(&mut session);
    assert!(
        session
            .classify(delivery(r#"{"event":"unsubscribe"}"#, 400))
            .is_err()
    );
    let occurrence = session
        .take_unavailable()
        .expect("state-changing control emits unavailable evidence");
    assert_eq!(occurrence.connection_epoch(), 10);
    assert_eq!(
        session.after_failure().unwrap(),
        Duration::from_millis(10),
        "readiness invalidation retains the session-owned fact that this attempt reached ACK"
    );
    assert_eq!(session.connection_epoch(), 11);
}

#[test]
fn reconnect_advances_epoch_for_subsequent_ack_and_reference_evidence() {
    let mut session = new_session();
    assert_eq!(session.after_failure().unwrap(), Duration::from_millis(10));
    assert_eq!(session.connection_epoch(), 8);

    acknowledge(&mut session);
    let payload = r#"{"arg":{"channel":"index-tickers","instId":"BTC-USDT"},"data":[{"instId":"BTC-USDT","idxPx":"50000","ts":"1700000000123"}]}"#;
    let classified = session.classify(delivery(payload, 500)).unwrap();
    let OkxPublicSessionEvent::Reference(reference) = classified.payload() else {
        panic!("expected reference after reconnect");
    };
    assert_eq!(reference.connection_epoch(), 8);
}

#[test]
fn disconnect_cannot_reconnect_without_one_consumed_receive_occurrence() {
    let mut session = new_session();
    session.invalidate(OkxPublicSessionFault::Disconnect);
    assert!(matches!(
        session.after_failure(),
        Err(OkxPublicSessionError::UnavailableOccurrenceMissing)
    ));

    session
        .invalidate_with_receive_evidence(
            OkxPublicSessionFault::Disconnect,
            1_700_000_000_123_456_789,
            100,
        )
        .unwrap();
    assert!(matches!(
        session.after_failure(),
        Err(OkxPublicSessionError::UnavailableOccurrencePending)
    ));
    let occurrence = session.take_unavailable().expect("disconnect occurrence");
    assert_eq!(occurrence.fault(), OkxPublicSessionFault::Disconnect);
    assert_eq!(session.take_unavailable(), None);
    session.after_failure().unwrap();
    assert_eq!(session.connection_epoch(), 8);
}

#[test]
fn connection_epoch_overflow_is_typed_and_fatal() {
    let mut session = OkxPublicSession::new(
        "BTC-USDT",
        ConnId::new("okx-public-reference-1"),
        u64::MAX,
        reconnect_policy(),
    )
    .unwrap();
    acknowledge(&mut session);

    assert!(matches!(
        session.after_failure(),
        Err(OkxPublicSessionError::ConnectionEpochOverflow)
    ));
    assert_eq!(session.connection_epoch(), u64::MAX);
    assert_eq!(session.health(), ConnectionStatusKind::Fatal);
    assert!(!session.subscription_ready());

    let acknowledgement =
        r#"{"event":"subscribe","arg":{"channel":"index-tickers","instId":"BTC-USDT"}}"#;
    let data = r#"{"arg":{"channel":"index-tickers","instId":"BTC-USDT"},"data":[{"instId":"BTC-USDT","idxPx":"50000","ts":"1700000000123"}]}"#;
    for payload in [acknowledgement, data, "{"] {
        assert!(matches!(
            session.classify(delivery(payload, 500)),
            Err(OkxPublicSessionError::SessionFatal)
        ));
        assert_eq!(session.health(), ConnectionStatusKind::Fatal);
        assert!(!session.subscription_ready());
    }
    assert!(matches!(
        session.after_failure(),
        Err(OkxPublicSessionError::SessionFatal)
    ));
    assert_eq!(session.connection_epoch(), u64::MAX);
    assert_eq!(session.health(), ConnectionStatusKind::Fatal);
}

#[test]
fn output_and_health_queues_are_finite() {
    let channels = OkxPublicSession::bounded_channels(1);
    channels
        .health_sender
        .try_send(ConnectionStatusKind::Ready)
        .unwrap();
    assert!(
        channels
            .health_sender
            .try_send(ConnectionStatusKind::Heartbeat)
            .is_err()
    );
}
