use std::time::Duration;

use reap_pm_core::{
    ConnectionEpoch, EvmAddress, MAX_REQUIRED_SPENDERS, OkxInstrumentId, OkxReferenceInstrument,
    PmAssetId, PmBookQuantity, PmBookSide, PmBookUpdate, PmChainId, PmConditionId, PmConnectionId,
    PmInstrumentHandle, PmInstrumentId, PmMarketHandle, PmMarketId, PmMarketLifecycle,
    PmMarketMetadata, PmOutcomeLabel, PmOutcomeMetadata, PmProductSource, PmPublicObservationGrant,
    PmQuantity, PmSourceHandle, PmSpenderDomain, PmSpenderRequirement, PmTick, PmTokenHandle,
    PmTokenId, SnapshotRevision, U256, VenueEventHashAlgorithm,
};
use reap_polymarket_adapter::{
    PM_PUBLIC_PING_BYTES, PM_PUBLIC_PONG_BYTES, PmAuthoritativeMetadata, PmMetadataRevisionInput,
    PmPublicHeartbeatAction, PmPublicHeartbeatConfig, PmPublicRole, PmPublicRoleError,
    PmPublicSession, PmPublicSessionError, PmPublicSessionFault, PmPublicSessionIgnored,
};
use reap_polymarket_wire::{
    PmBookParserConfig, PmWireScope, compute_snapshot_hash, parse_clob_metadata,
    parse_lifecycle_metadata, parse_ws_frame,
};
use reap_transport::{ConnectionStatusKind, ReconnectPolicy};

const MARKET: &str = "0xbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbb";
const CONDITION: &str = "0xaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa";
const PUSD: &str = "0xC011a7E12a19f7B1f670d46F03B03f3342E82DFB";
const CONDITIONAL_TOKENS: &str = "0x4D97DCd97eC945f40cF65F87097ACe5EA0476045";
const STANDARD_EXCHANGE: &str = "0xE111180000d2663C0091e4f400237545B87B996B";

fn instrument() -> PmInstrumentHandle {
    PmInstrumentHandle::new(
        PmMarketHandle::from_ordinal(0),
        PmTokenHandle::from_ordinal(0),
    )
}

fn parser_config() -> PmBookParserConfig {
    PmBookParserConfig::new(
        PmWireScope::new(
            PmConditionId::parse(CONDITION).unwrap(),
            PmMarketId::parse(MARKET).unwrap(),
            PmTokenId::new(U256::from_u64(123)).unwrap(),
        ),
        PmTick::parse_decimal("0.01").unwrap(),
        PmQuantity::parse_decimal("5").unwrap(),
        false,
    )
}

fn observation_grant() -> PmPublicObservationGrant {
    let scope = parser_config().scope();
    PmPublicObservationGrant::derive_goal_f(
        OkxReferenceInstrument::index(OkxInstrumentId::new("BTC-USDT").unwrap()),
        PmInstrumentId::new(scope.market(), scope.token()),
    )
}

fn parse_address(value: &str) -> EvmAddress {
    EvmAddress::parse(value).unwrap()
}

fn expected_metadata() -> PmMarketMetadata {
    let chain = PmChainId::new(137).unwrap();
    let exchange = parse_address(STANDARD_EXCHANGE);
    let token = parser_config().scope().token();
    let mut spenders = [None; MAX_REQUIRED_SPENDERS];
    spenders[0] = Some(PmSpenderRequirement::new(
        chain,
        exchange,
        PmSpenderDomain::Standard,
        PmAssetId::collateral(parse_address(PUSD)),
    ));
    spenders[1] = Some(PmSpenderRequirement::new(
        chain,
        exchange,
        PmSpenderDomain::Standard,
        PmAssetId::outcome(parse_address(CONDITIONAL_TOKENS), token),
    ));
    PmMarketMetadata::new(
        parser_config().scope().condition(),
        parser_config().scope().market(),
        PmOutcomeMetadata::new(token, PmOutcomeLabel::new("Yes").unwrap()),
        PmMarketLifecycle::new(true, false, false, true, true),
        parser_config().tick(),
        parser_config().minimum_order_size(),
        false,
        chain,
        exchange,
        spenders,
        2,
    )
    .unwrap()
}

fn authoritative_metadata() -> PmAuthoritativeMetadata {
    let scope = parser_config().scope();
    let lifecycle = parse_lifecycle_metadata(
        format!(
            r#"{{"condition_id":"{CONDITION}","market_id":"{MARKET}","active":true,"closed":false,"archived":false,"accepting_orders":true,"enable_order_book":true}}"#
        )
        .as_bytes(),
        scope,
    )
    .unwrap();
    let clob = parse_clob_metadata(
        format!(
            r#"{{"condition_id":"{CONDITION}","market_id":"{MARKET}","minimum_tick_size":"0.01","minimum_order_size":"5","neg_risk":false,"tokens":[{{"token_id":"123","outcome":"Yes"}},{{"token_id":"456","outcome":"No"}}]}}"#
        )
        .as_bytes(),
        scope,
    )
    .unwrap();
    PmAuthoritativeMetadata::join(
        instrument(),
        PmProductSource::polymarket_market(PmSourceHandle::from_ordinal(4), instrument().token()),
        expected_metadata(),
        lifecycle,
        &clob,
        PmMetadataRevisionInput::new(SnapshotRevision::new(7), 50).unwrap(),
    )
    .unwrap()
}

fn role() -> PmPublicRole {
    PmPublicRole::new(
        observation_grant(),
        instrument(),
        parser_config(),
        PmProductSource::polymarket_market(PmSourceHandle::from_ordinal(4), instrument().token()),
        PmConnectionId::new("pm-public-1").unwrap(),
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

fn heartbeat() -> PmPublicHeartbeatConfig {
    PmPublicHeartbeatConfig::new(10, 5).unwrap()
}

fn new_session() -> PmPublicSession {
    PmPublicSession::new(
        role(),
        authoritative_metadata(),
        ConnectionEpoch::new(11),
        None,
        reconnect_policy(),
        heartbeat(),
    )
    .unwrap()
}

#[test]
fn metadata_occurrence_uses_the_session_ingress_counter_once() {
    let mut session = new_session();
    let occurrence = session
        .issue_metadata_occurrence(1_700_000_000_000_000_050)
        .unwrap();

    assert_eq!(occurrence.source(), role().source());
    assert_eq!(occurrence.connection_id(), role().connection());
    assert_eq!(occurrence.received_clock().venue_event_timestamp_ns(), None);
    assert_eq!(occurrence.received_clock().monotonic_receive_ns(), 50);
    assert_eq!(
        occurrence.ordering().connection_epoch(),
        ConnectionEpoch::new(11)
    );
    assert_eq!(occurrence.ordering().local_ingress_sequence().value(), 1);
    assert_eq!(
        session.issue_metadata_occurrence(1_700_000_000_000_000_051),
        Err(PmPublicSessionError::MetadataOccurrenceAlreadyIssued)
    );

    subscribe(&mut session, 100);
    let snapshot = session
        .classify(
            snapshot_with(123_456_789).as_bytes(),
            1_700_000_000_000_000_110,
            110,
        )
        .unwrap();
    assert_eq!(
        snapshot.events()[0]
            .ordering()
            .local_ingress_sequence()
            .value(),
        2,
        "metadata and the first websocket event share one collision-free source counter"
    );
}

fn snapshot_with(timestamp: u64) -> String {
    let placeholder = format!(
        r#"{{
          "event_type":"book",
          "market":"{MARKET}",
          "asset_id":"123",
          "timestamp":"{timestamp}",
          "hash":"",
          "bids":[{{"price":"0.30","size":"100"}},{{"price":"0.40","size":"50"}}],
          "asks":[{{"price":"0.60","size":"75"}},{{"price":"0.70","size":"100"}}],
          "min_order_size":"5",
          "tick_size":"0.01",
          "neg_risk":false,
          "last_trade_price":"0.50"
        }}"#
    );
    let hash = compute_snapshot_hash(placeholder.as_bytes()).unwrap();
    placeholder.replace(r#""hash":"""#, &format!(r#""hash":"{hash}""#))
}

fn delta(timestamp: u64, token: u64, transaction_hash: &str) -> String {
    format!(
        r#"{{
          "event_type":"price_change",
          "market":"{MARKET}",
          "timestamp":"{timestamp}",
          "price_changes":[
            {{
              "asset_id":"{token}",
              "price":"0.40",
              "size":"0",
              "side":"BUY",
              "hash":"{transaction_hash}",
              "best_bid":"0.30",
              "best_ask":"0.60"
            }},
            {{
              "asset_id":"{token}",
              "price":"0.50",
              "size":"12.5",
              "side":"BUY",
              "hash":"tx-add",
              "best_bid":"0.50",
              "best_ask":"0.60"
            }}
          ]
        }}"#
    )
}

fn bbo(timestamp: u64) -> String {
    format!(
        r#"{{
          "event_type":"best_bid_ask",
          "market":"{MARKET}",
          "asset_id":"123",
          "timestamp":"{timestamp}",
          "best_bid":"0.50",
          "best_ask":"0.60",
          "bid_size":"12.5",
          "ask_size":"75"
        }}"#
    )
}

fn trade(timestamp: u64) -> String {
    format!(
        r#"{{
          "event_type":"last_trade_price",
          "market":"{MARKET}",
          "asset_id":"123",
          "price":"0.50",
          "size":"10",
          "side":"BUY",
          "timestamp":"{timestamp}"
        }}"#
    )
}

fn tick_change(timestamp: u64) -> String {
    format!(
        r#"{{
          "event_type":"tick_size_change",
          "market":"{MARKET}",
          "asset_id":"123",
          "timestamp":"{timestamp}",
          "old_tick_size":"0.01",
          "new_tick_size":"0.001"
        }}"#
    )
}

fn subscribe(session: &mut PmPublicSession, now_ns: u64) {
    session.mark_subscription_sent(now_ns).unwrap();
}

fn commit_snapshot(session: &mut PmPublicSession, raw: &str, receive_ns: u64) {
    let batch = session
        .classify(raw.as_bytes(), 1_700_000_000_000_000_000, receive_ns)
        .unwrap();
    let token = batch
        .snapshot_flow_token()
        .expect("snapshot protocol-flow token");
    session.open_protocol_flow_after_snapshot(token).unwrap();
}

#[test]
fn subscription_is_exactly_one_token_and_snapshot_only_opens_protocol_flow() {
    let mut session = new_session();
    assert_eq!(
        session.subscription_bytes(),
        br#"{"assets_ids":["123"],"custom_feature_enabled":true,"initial_dump":true,"operation":"subscribe","type":"market"}"#
    );
    assert_eq!(
        session.configuration_fingerprint(),
        observation_grant().configuration_fingerprint()
    );
    assert_eq!(session.health(), ConnectionStatusKind::Disconnected);
    assert!(!session.protocol_flow_open());

    subscribe(&mut session, 100);
    assert!(
        !session.protocol_flow_open(),
        "sending is not an invented ACK"
    );
    let raw = snapshot_with(123_456_789);
    let expected_hash = match &parse_ws_frame(raw.as_bytes(), parser_config())
        .unwrap()
        .events()[0]
    {
        reap_polymarket_wire::PmWsEvent::BookSnapshot(snapshot) => snapshot.verified_hash(),
        _ => panic!("snapshot"),
    };
    let batch = session
        .classify(raw.as_bytes(), 1_700_000_000_123_456_789, 120)
        .unwrap();

    assert_eq!(batch.events().len(), 1);
    assert!(batch.ignored().is_empty());
    assert!(
        !session.protocol_flow_open(),
        "classification does not prove the snapshot was reduced"
    );
    assert_eq!(session.health(), ConnectionStatusKind::Disconnected);
    let event = &batch.events()[0];
    assert_eq!(event.source(), role().source());
    assert_eq!(event.connection_id(), role().connection());
    assert_eq!(
        event.received_clock().venue_event_timestamp_ns(),
        Some(123_456_789_000_000)
    );
    assert_eq!(
        event.received_clock().local_wall_receive_ns(),
        1_700_000_000_123_456_789
    );
    assert_eq!(event.received_clock().monotonic_receive_ns(), 120);
    assert_eq!(
        event.ordering().connection_epoch(),
        ConnectionEpoch::new(11)
    );
    assert_eq!(
        event.ordering().snapshot_revision(),
        Some(SnapshotRevision::new(1))
    );
    assert_eq!(event.ordering().venue_sequence(), None);
    assert_eq!(event.ordering().local_ingress_sequence().value(), 1);
    let venue_hash = event.ordering().venue_hash().unwrap();
    assert_eq!(venue_hash.algorithm(), VenueEventHashAlgorithm::Sha1);
    assert_eq!(venue_hash.len(), 20);
    assert_eq!(venue_hash.as_bytes(), expected_hash.bytes());
    assert_eq!(
        event.payload().metadata_revision(),
        SnapshotRevision::new(7)
    );
    let PmBookUpdate::Snapshot(snapshot) = event.payload().update() else {
        panic!("atomic snapshot event");
    };
    assert_eq!(snapshot.levels().len(), 4);

    let token = batch.snapshot_flow_token().unwrap();
    assert_eq!(token.connection_epoch(), ConnectionEpoch::new(11));
    assert_eq!(token.snapshot_revision(), SnapshotRevision::new(1));
    assert_eq!(token.local_ingress_sequence().value(), 1);
    assert!(
        !session.protocol_flow_open(),
        "a bare token cannot establish product readiness or even open delta flow"
    );
    session.open_protocol_flow_after_snapshot(token).unwrap();
    assert!(
        session.protocol_flow_open(),
        "the acknowledgement opens venue delta flow only; product readiness remains reducer-owned"
    );
    assert_eq!(session.health(), ConnectionStatusKind::Ready);
}

#[test]
fn construction_requires_the_exact_authoritative_metadata_join_for_the_role() {
    let authority = authoritative_metadata();
    let derived_role = PmPublicRole::from_expected_metadata(
        observation_grant(),
        expected_metadata(),
        role().source(),
        role().connection(),
    )
    .unwrap();
    assert_eq!(derived_role.parser_config(), parser_config());
    assert_eq!(
        new_session().authoritative_metadata(),
        authority,
        "session retains the complete checked metadata observation"
    );

    let wrong_instrument = PmInstrumentHandle::new(
        PmMarketHandle::from_ordinal(9),
        PmTokenHandle::from_ordinal(9),
    );
    assert_eq!(
        PmPublicRole::new(
            observation_grant(),
            wrong_instrument,
            parser_config(),
            PmProductSource::polymarket_market(
                PmSourceHandle::from_ordinal(4),
                wrong_instrument.token(),
            ),
            PmConnectionId::new("pm-public-wrong-instrument").unwrap(),
        ),
        Err(PmPublicRoleError::GrantInstrumentMismatch)
    );

    let scope = parser_config().scope();
    let wrong_market_grant = PmPublicObservationGrant::derive_goal_f(
        observation_grant().okx_instrument(),
        PmInstrumentId::new(PmMarketId::parse(CONDITION).unwrap(), scope.token()),
    );
    assert_eq!(
        PmPublicRole::new(
            wrong_market_grant,
            instrument(),
            parser_config(),
            role().source(),
            PmConnectionId::new("pm-public-wrong-raw-market").unwrap(),
        ),
        Err(PmPublicRoleError::GrantMarketMismatch)
    );
    let wrong_token_grant = PmPublicObservationGrant::derive_goal_f(
        observation_grant().okx_instrument(),
        PmInstrumentId::new(scope.market(), PmTokenId::new(U256::from_u64(999)).unwrap()),
    );
    assert_eq!(
        PmPublicRole::new(
            wrong_token_grant,
            instrument(),
            parser_config(),
            role().source(),
            PmConnectionId::new("pm-public-wrong-raw-token").unwrap(),
        ),
        Err(PmPublicRoleError::GrantTokenMismatch)
    );

    let wrong_source_role = PmPublicRole::new(
        observation_grant(),
        instrument(),
        parser_config(),
        PmProductSource::polymarket_market(PmSourceHandle::from_ordinal(99), instrument().token()),
        PmConnectionId::new("pm-public-wrong-source").unwrap(),
    )
    .unwrap();
    assert!(matches!(
        PmPublicSession::new(
            wrong_source_role,
            authority,
            ConnectionEpoch::new(1),
            None,
            reconnect_policy(),
            heartbeat(),
        ),
        Err(PmPublicSessionError::MetadataSourceMismatch)
    ));

    let wrong_parser_role = PmPublicRole::new(
        observation_grant(),
        instrument(),
        PmBookParserConfig::new(
            parser_config().scope(),
            PmTick::parse_decimal("0.001").unwrap(),
            parser_config().minimum_order_size(),
            false,
        ),
        role().source(),
        PmConnectionId::new("pm-public-wrong-parser").unwrap(),
    )
    .unwrap();
    assert!(matches!(
        PmPublicSession::new(
            wrong_parser_role,
            authority,
            ConnectionEpoch::new(1),
            None,
            reconnect_policy(),
            heartbeat(),
        ),
        Err(PmPublicSessionError::MetadataParserConfigMismatch)
    ));

    let mut session = new_session();
    assert_eq!(
        session.mark_subscription_sent(49),
        Err(PmPublicSessionError::MonotonicClockRegression)
    );
    assert!(session.requires_reconnect());
}

#[test]
fn malformed_wrong_token_and_ack_fiction_fail_closed_until_reconnect_and_resubscribe() {
    for raw in [
        "{".to_string(),
        delta(123_456_790, 999, "raw-transaction-evidence"),
        r#"{"event_type":"subscribed","asset_id":"123"}"#.to_string(),
    ] {
        let mut session = new_session();
        subscribe(&mut session, 100);
        assert!(
            session
                .classify(raw.as_bytes(), 1_700_000_000_000_000_000, 101)
                .is_err()
        );
        assert!(session.requires_reconnect());
        assert!(!session.subscription_sent());
        assert!(!session.protocol_flow_open());
        assert!(matches!(
            session.classify(
                snapshot_with(123_456_789).as_bytes(),
                1_700_000_000_000_000_000,
                102
            ),
            Err(PmPublicSessionError::ReconnectRequired)
        ));
        assert_eq!(
            session.after_failure(),
            Err(PmPublicSessionError::UnavailableOccurrencePending)
        );
        let unavailable = session
            .take_unavailable()
            .expect("failed classification emits one unavailable occurrence");
        assert_eq!(unavailable.source(), role().source());
        assert_eq!(unavailable.connection_id(), role().connection());
        assert_eq!(
            unavailable.ordering().connection_epoch(),
            ConnectionEpoch::new(11)
        );
        assert_eq!(unavailable.ordering().local_ingress_sequence().value(), 1);
        assert_eq!(
            unavailable.received_clock().venue_event_timestamp_ns(),
            None
        );
        assert_eq!(session.take_unavailable(), None);
        assert_eq!(session.after_failure().unwrap(), Duration::from_millis(10));
        assert_eq!(session.connection_epoch(), ConnectionEpoch::new(12));
        assert!(matches!(
            session.classify(
                snapshot_with(123_456_789).as_bytes(),
                1_700_000_000_000_000_000,
                103
            ),
            Err(PmPublicSessionError::SubscriptionNotSent)
        ));
    }
}

#[test]
fn delta_bbo_tick_and_trade_have_narrow_typed_semantics() {
    let mut session = new_session();
    subscribe(&mut session, 100);
    commit_snapshot(&mut session, &snapshot_with(123_456_789), 110);

    let delta = session
        .classify(
            delta(123_456_790, 123, "raw-transaction-evidence").as_bytes(),
            1_700_000_000_000_000_000,
            120,
        )
        .unwrap();
    assert_eq!(delta.events().len(), 1);
    let delta_event = &delta.events()[0];
    assert_eq!(
        delta_event.ordering().snapshot_revision(),
        Some(SnapshotRevision::new(1))
    );
    assert_eq!(delta_event.ordering().venue_sequence(), None);
    assert_eq!(
        delta_event.ordering().venue_hash(),
        None,
        "transaction hashes are raw capture evidence, not ordering hashes"
    );
    assert_eq!(delta_event.ordering().local_ingress_sequence().value(), 2);
    assert!(!delta_event.is_terminal_tick_size_change());
    let PmBookUpdate::DeltaBatch(changes) = delta_event.payload().update() else {
        panic!("one atomic delta batch");
    };
    assert_eq!(changes.changes().len(), 2);
    assert_eq!(
        changes.venue_change_hashes()[0]
            .expect("delete hash")
            .as_str(),
        "raw-transaction-evidence"
    );
    assert_eq!(
        changes.venue_change_hashes()[1].expect("add hash").as_str(),
        "tx-add"
    );
    assert_eq!(changes.expected_top().bid().unwrap().to_string(), "0.5");
    assert_eq!(changes.expected_top().ask().unwrap().to_string(), "0.6");
    assert_eq!(changes.changes()[0].quantity(), PmBookQuantity::Delete);
    assert_eq!(changes.changes()[1].side(), PmBookSide::Bid);

    let bbo = session
        .classify(bbo(123_456_791).as_bytes(), 1_700_000_000_000_000_000, 121)
        .unwrap();
    let PmBookUpdate::TopCheck(top) = bbo.events()[0].payload().update() else {
        panic!("one atomic BBO check");
    };
    assert_eq!(top.bid().unwrap().to_string(), "0.5");
    assert_eq!(top.ask().unwrap().to_string(), "0.6");
    assert_eq!(
        bbo.events()[0].ordering().local_ingress_sequence().value(),
        3
    );
    assert_eq!(
        bbo.events()[0].ordering().snapshot_revision(),
        Some(SnapshotRevision::new(1))
    );

    let ignored = session
        .classify(
            trade(123_456_792).as_bytes(),
            1_700_000_000_000_000_000,
            122,
        )
        .unwrap();
    assert!(ignored.events().is_empty());
    assert_eq!(ignored.ignored(), &[PmPublicSessionIgnored::PublicTrade]);
    assert!(
        session.protocol_flow_open(),
        "ignored trade cannot mutate protocol flow"
    );

    let tick = session
        .classify(
            tick_change(123_456_793).as_bytes(),
            1_700_000_000_000_000_000,
            123,
        )
        .unwrap();
    assert_eq!(tick.events().len(), 1);
    let PmBookUpdate::TickSizeChanged { old, new } = tick.events()[0].payload().update() else {
        panic!("typed tick-size invalidation");
    };
    assert_eq!(old.to_string(), "0.01");
    assert_eq!(new.to_string(), "0.001");
    assert_eq!(
        tick.events()[0].ordering().snapshot_revision(),
        Some(SnapshotRevision::new(1))
    );
    assert_eq!(tick.events()[0].ordering().venue_sequence(), None);
    assert_eq!(
        tick.events()[0].received_clock().venue_event_timestamp_ns(),
        Some(123_456_793_000_000)
    );
    assert!(tick.events()[0].is_terminal_tick_size_change());
    assert!(!session.protocol_flow_open());
    assert!(session.requires_reconnect());
    assert_eq!(
        session.last_fault(),
        Some(PmPublicSessionFault::TickSizeChanged)
    );
    assert_eq!(
        session.after_failure(),
        Err(PmPublicSessionError::UnavailableOccurrencePending)
    );
    let unavailable = session
        .take_unavailable()
        .expect("terminal tick change emits one unavailable occurrence");
    assert_eq!(unavailable.fault(), PmPublicSessionFault::TickSizeChanged);
    assert_eq!(
        unavailable.received_clock().venue_event_timestamp_ns(),
        None
    );
    assert_eq!(
        unavailable.ordering().local_ingress_sequence().value(),
        tick.events()[0].ordering().local_ingress_sequence().value() + 1
    );
    assert_eq!(session.take_unavailable(), None);
}

#[test]
fn mixed_tick_frame_and_reducer_rejection_emit_one_fail_closed_occurrence() {
    let mut mixed = new_session();
    subscribe(&mut mixed, 100);
    commit_snapshot(&mut mixed, &snapshot_with(123_456_789), 110);
    let frame = format!("[{},{}]", bbo(123_456_790), tick_change(123_456_791));
    assert_eq!(
        mixed.classify(frame.as_bytes(), 1_700_000_000_000_000_120, 120,),
        Err(PmPublicSessionError::TickSizeChangeMustBeSoleEvent)
    );
    assert_eq!(
        mixed
            .take_unavailable()
            .expect("mixed terminal frame emits unavailable evidence")
            .fault(),
        PmPublicSessionFault::InvalidTransition
    );

    let mut rejected = new_session();
    subscribe(&mut rejected, 100);
    let batch = rejected
        .classify(
            snapshot_with(123_456_789).as_bytes(),
            1_700_000_000_000_000_110,
            110,
        )
        .unwrap();
    let token = batch.snapshot_flow_token().expect("snapshot flow token");
    rejected
        .reject_snapshot_flow_with_receive_evidence(token, 1_700_000_000_000_000_111, 111)
        .unwrap();
    assert_eq!(
        rejected
            .take_unavailable()
            .expect("reducer rejection emits unavailable evidence")
            .fault(),
        PmPublicSessionFault::ReducerRejected
    );
}

#[test]
fn venue_timestamps_are_evidence_and_never_invent_a_predecessor_sequence() {
    let mut session = new_session();
    subscribe(&mut session, 100);
    commit_snapshot(&mut session, &snapshot_with(123_456_789), 110);
    let same_timestamp = session
        .classify(
            delta(123_456_789, 123, "one").as_bytes(),
            1_700_000_000_000_000_000,
            120,
        )
        .unwrap();
    let decreasing_timestamp = session
        .classify(bbo(123_456_788).as_bytes(), 1_700_000_000_000_000_000, 121)
        .unwrap();

    assert_eq!(
        same_timestamp.events()[0]
            .received_clock()
            .venue_event_timestamp_ns(),
        Some(123_456_789_000_000)
    );
    assert_eq!(
        decreasing_timestamp.events()[0]
            .received_clock()
            .venue_event_timestamp_ns(),
        Some(123_456_788_000_000)
    );
    assert_eq!(same_timestamp.events()[0].ordering().venue_sequence(), None);
    assert_eq!(
        decreasing_timestamp.events()[0].ordering().venue_sequence(),
        None
    );
    assert_eq!(
        decreasing_timestamp.events()[0]
            .ordering()
            .local_ingress_sequence()
            .value(),
        3
    );
    assert!(session.protocol_flow_open());
}

#[test]
fn reconnect_advances_epoch_once_resets_connection_state_and_uses_commit_backoff_history() {
    let mut session = new_session();
    assert_eq!(session.after_failure().unwrap(), Duration::from_millis(10));
    assert_eq!(session.connection_epoch(), ConnectionEpoch::new(12));
    assert_eq!(session.after_failure().unwrap(), Duration::from_millis(20));
    assert_eq!(session.connection_epoch(), ConnectionEpoch::new(13));

    subscribe(&mut session, 100);
    commit_snapshot(&mut session, &snapshot_with(123_456_789), 110);
    assert_eq!(
        session.after_failure().unwrap(),
        Duration::from_millis(10),
        "only an acknowledged, synchronously reduced snapshot resets startup history"
    );
    assert_eq!(session.connection_epoch(), ConnectionEpoch::new(14));
    assert!(!session.subscription_sent());
    assert!(!session.protocol_flow_open());
    assert_eq!(session.current_snapshot_revision(), None);
    assert_eq!(session.local_ingress_sequence().value(), 0);

    subscribe(&mut session, 200);
    let next = session
        .classify(
            snapshot_with(123_456_789).as_bytes(),
            1_700_000_000_000_000_000,
            210,
        )
        .unwrap();
    assert_eq!(
        next.events()[0].ordering().snapshot_revision(),
        Some(SnapshotRevision::new(2)),
        "fresh snapshots remain globally increasing across connection epochs"
    );
    assert_eq!(
        next.events()[0].ordering().local_ingress_sequence().value(),
        1,
        "ingress is connection-local"
    );
}

#[test]
fn external_overflow_requires_a_new_epoch_subscription_and_snapshot() {
    let mut session = new_session();
    subscribe(&mut session, 100);
    commit_snapshot(&mut session, &snapshot_with(123_456_789), 110);

    session.invalidate(PmPublicSessionFault::Overflow);
    assert!(session.requires_reconnect());
    assert!(!session.protocol_flow_open());
    assert_eq!(session.last_fault(), Some(PmPublicSessionFault::Overflow));
    assert_eq!(
        session.after_failure(),
        Err(PmPublicSessionError::UnavailableOccurrenceMissing)
    );
    session
        .invalidate_with_receive_evidence(
            PmPublicSessionFault::Overflow,
            1_700_000_000_000_000_111,
            111,
        )
        .unwrap();
    assert_eq!(
        session
            .take_unavailable()
            .expect("external overflow must attach receive evidence")
            .fault(),
        PmPublicSessionFault::Overflow
    );
    session.after_failure().unwrap();
    subscribe(&mut session, 200);
    assert!(matches!(
        session.classify(bbo(123_456_790).as_bytes(), 1_700_000_000_000_000_000, 210,),
        Err(PmPublicSessionError::DataBeforeSnapshotFlowOpen)
    ));
}

#[test]
fn heartbeat_ping_pong_and_deadline_are_venue_owned_and_never_readiness() {
    assert_eq!(PM_PUBLIC_PING_BYTES, b"PING");
    assert_eq!(PM_PUBLIC_PONG_BYTES, b"PONG");
    let mut session = new_session();
    subscribe(&mut session, 100);

    assert_eq!(
        session.poll_heartbeat(109).unwrap(),
        PmPublicHeartbeatAction::Idle
    );
    assert_eq!(
        session.poll_heartbeat(110).unwrap(),
        PmPublicHeartbeatAction::SendPing
    );
    assert_eq!(
        session.poll_heartbeat(114).unwrap(),
        PmPublicHeartbeatAction::Idle
    );
    let pong = session
        .classify(PM_PUBLIC_PONG_BYTES, 1_700_000_000_000_000_000, 114)
        .unwrap();
    let evidence = pong.heartbeat().expect("typed heartbeat evidence");
    assert_eq!(evidence.connection_epoch(), ConnectionEpoch::new(11));
    assert_eq!(evidence.monotonic_receive_ns(), 114);
    assert!(
        !session.protocol_flow_open(),
        "PONG is connection liveness, never snapshot protocol flow or product readiness"
    );

    assert_eq!(
        session.poll_heartbeat(124).unwrap(),
        PmPublicHeartbeatAction::SendPing
    );
    assert!(matches!(
        session.poll_heartbeat_with_receive_evidence(1_700_000_000_000_000_129, 129),
        Err(PmPublicSessionError::HeartbeatTimeout { deadline_ns: 129 })
    ));
    assert!(session.requires_reconnect());
    let unavailable = session
        .take_unavailable()
        .expect("heartbeat timeout emits unavailable evidence");
    assert_eq!(unavailable.fault(), PmPublicSessionFault::HeartbeatTimeout);
    assert_eq!(
        unavailable.received_clock().venue_event_timestamp_ns(),
        None
    );
}

#[test]
fn checked_timestamp_revision_and_epoch_overflow_fail_closed() {
    let mut timestamp = new_session();
    subscribe(&mut timestamp, 100);
    assert!(matches!(
        timestamp.classify(
            snapshot_with(u64::MAX).as_bytes(),
            1_700_000_000_000_000_000,
            110,
        ),
        Err(PmPublicSessionError::VenueTimestampOverflow)
    ));
    assert!(timestamp.requires_reconnect());

    let mut revision = PmPublicSession::new(
        role(),
        authoritative_metadata(),
        ConnectionEpoch::new(11),
        Some(SnapshotRevision::new(u64::MAX)),
        reconnect_policy(),
        heartbeat(),
    )
    .unwrap();
    subscribe(&mut revision, 100);
    assert!(matches!(
        revision.classify(
            snapshot_with(123_456_789).as_bytes(),
            1_700_000_000_000_000_000,
            110,
        ),
        Err(PmPublicSessionError::SnapshotRevisionOverflow)
    ));
    assert!(revision.requires_reconnect());

    let mut epoch = PmPublicSession::new(
        role(),
        authoritative_metadata(),
        ConnectionEpoch::new(u64::MAX),
        None,
        reconnect_policy(),
        heartbeat(),
    )
    .unwrap();
    assert!(matches!(
        epoch.after_failure(),
        Err(PmPublicSessionError::ConnectionEpochOverflow)
    ));
    assert_eq!(epoch.health(), ConnectionStatusKind::Fatal);
}
