use reap_polymarket_wire::{
    MAX_PRIVATE_FIXTURE_BYTES, MAX_PRIVATE_FIXTURE_EVENTS, PmFixtureAllowanceScope,
    PmFixtureTradeLinkage, PmFixtureUserEvent, PmPrivateFixtureError,
    parse_legacy_balance_allowance_fixture, parse_open_order_fixture, parse_private_user_fixture,
};

const BALANCE: &[u8] = include_bytes!("../fixtures/predarb_balance_allowance_seed.json");
const OPEN_ORDER: &[u8] = include_bytes!("../fixtures/predarb_open_order_seed.json");
const USER_ORDER: &[u8] = include_bytes!("../fixtures/predarb_user_order_seed.json");
const USER_TRADE: &[u8] = include_bytes!("../fixtures/predarb_user_trade_seed.json");

#[test]
fn pinned_private_seeds_parse_without_normalizing_their_wire_strings() {
    let balance = parse_legacy_balance_allowance_fixture(BALANCE).unwrap();
    assert_eq!(balance.balance(), "1000.00");
    assert_eq!(balance.unscoped_allowance(), "1000.00");
    assert_eq!(
        balance.allowance_scope(),
        PmFixtureAllowanceScope::UnscopedLegacyScalar
    );

    let open_order = parse_open_order_fixture(OPEN_ORDER).unwrap();
    assert_eq!(open_order.id(), "0xorder");
    assert_eq!(open_order.market(), "0xbd31");
    assert_eq!(open_order.asset_id(), "5211431950123");
    assert_eq!(open_order.side(), "BUY");
    assert_eq!(open_order.original_size(), "100.0");
    assert_eq!(open_order.size_matched(), "0");
    assert_eq!(open_order.price(), "0.40");
    assert_eq!(open_order.status(), "LIVE");
    assert_eq!(
        open_order.maker_address(),
        "0x1111111111111111111111111111111111111111"
    );

    let order_frame = parse_private_user_fixture(USER_ORDER).unwrap();
    let PmFixtureUserEvent::Order(order) = &order_frame.events()[0] else {
        panic!("tracked order seed must remain an order");
    };
    assert_eq!(order.id(), "0xorder");
    assert_eq!(order.market(), "0xbd31");
    assert_eq!(order.asset_id(), "5211431950123");
    assert_eq!(order.side(), "BUY");
    assert_eq!(order.original_size(), "100.0");
    assert_eq!(order.size_matched(), "0");
    assert_eq!(order.price(), "0.40");
    assert_eq!(order.status(), "LIVE");
    assert_eq!(
        order.maker_address(),
        "0x1111111111111111111111111111111111111111"
    );
    assert_eq!(order.event_kind(), "PLACEMENT");
}

#[test]
fn pinned_trade_is_explicitly_unlinked_and_scalar_allowance_is_unscoped() {
    let frame = parse_private_user_fixture(USER_TRADE).unwrap();
    let PmFixtureUserEvent::Trade(trade) = &frame.events()[0] else {
        panic!("tracked trade seed must remain a trade");
    };
    assert_eq!(trade.id(), "0xtrade");
    assert_eq!(trade.market(), "0xbd31");
    assert_eq!(trade.asset_id(), "5211431950123");
    assert_eq!(trade.side(), "BUY");
    assert_eq!(trade.size(), "10.0");
    assert_eq!(trade.price(), "0.40");
    assert_eq!(trade.status(), "MATCHED");
    assert_eq!(
        trade.maker_address(),
        "0x1111111111111111111111111111111111111111"
    );
    assert_eq!(trade.transaction_hash(), "0xdeadbeef");
    assert_eq!(trade.order_id(), None);
    assert_eq!(trade.taker_order_id(), None);
    assert_eq!(trade.trader_side(), None);
    assert_eq!(trade.maker_orders(), None);
    assert_eq!(trade.linkage(), PmFixtureTradeLinkage::Unlinked);

    let balance = parse_legacy_balance_allowance_fixture(BALANCE).unwrap();
    assert_eq!(
        balance.allowance_scope(),
        PmFixtureAllowanceScope::UnscopedLegacyScalar
    );
}

#[test]
fn user_fixture_accepts_one_object_or_a_bounded_array() {
    assert_eq!(
        parse_private_user_fixture(USER_ORDER)
            .unwrap()
            .events()
            .len(),
        1
    );

    let array = format!(
        "[{},{}]",
        String::from_utf8_lossy(USER_ORDER),
        String::from_utf8_lossy(USER_TRADE)
    );
    let frame = parse_private_user_fixture(array.as_bytes()).unwrap();
    assert_eq!(frame.events().len(), 2);
    assert!(matches!(frame.events()[0], PmFixtureUserEvent::Order(_)));
    assert!(matches!(frame.events()[1], PmFixtureUserEvent::Trade(_)));
}

#[test]
fn maker_order_legs_preserve_all_order_reference_evidence() {
    let raw = br#"{
        "event_type":"trade",
        "id":"trade-1",
        "market":"market-1",
        "asset_id":"123",
        "side":"BUY",
        "size":"2.5",
        "price":"0.40",
        "status":"MATCHED",
        "maker_address":"maker-1",
        "transaction_hash":"tx-1",
        "trader_side":"MAKER",
        "maker_orders":[{
            "order_id":"order-1",
            "asset_id":"123",
            "side":"SELL",
            "price":"0.40",
            "matched_amount":"2.5",
            "maker_address":"0x1111111111111111111111111111111111111111"
        },{
            "order_id":"order-2",
            "asset_id":"123",
            "side":"SELL",
            "price":"0.40",
            "matched_amount":"1"
        }]
    }"#;
    let frame = parse_private_user_fixture(raw).unwrap();
    let PmFixtureUserEvent::Trade(trade) = &frame.events()[0] else {
        panic!("maker-leg fixture must remain a trade");
    };
    assert_eq!(trade.linkage(), PmFixtureTradeLinkage::MakerOrders);
    assert_eq!(trade.trader_side(), Some("MAKER"));
    let maker_orders = trade.maker_orders().unwrap();
    assert_eq!(maker_orders.len(), 2);
    assert_eq!(maker_orders[0].order_id(), "order-1");
    assert_eq!(maker_orders[0].asset_id(), "123");
    assert_eq!(maker_orders[0].side(), "SELL");
    assert_eq!(maker_orders[0].price(), "0.40");
    assert_eq!(maker_orders[0].matched_amount(), "2.5");
    assert_eq!(
        maker_orders[0].maker_address(),
        Some("0x1111111111111111111111111111111111111111")
    );
    assert_eq!(maker_orders[1].maker_address(), None);
}

#[test]
fn required_unknown_discriminator_and_malformed_user_shapes_fail_closed() {
    for raw in [
        br#"{"event_type":"order","id":"only-id"}"#.as_slice(),
        br#"{"event_type":"ORDER","id":"only-id"}"#.as_slice(),
        br#"{"event_type":"balance","asset":"pUSD"}"#.as_slice(),
        br#"{"event_type":"position","asset_id":"123"}"#.as_slice(),
        br#"{"event_type":"order","id":"id","market":"m","asset_id":"1","side":"BUY","original_size":"1","size_matched":"0","price":"0.4","status":"LIVE","maker_address":"maker","type":"PLACEMENT","extra":true}"#.as_slice(),
        br#"{"#.as_slice(),
        br#"null"#.as_slice(),
    ] {
        assert_eq!(
            parse_private_user_fixture(raw),
            Err(PmPrivateFixtureError::MalformedJson)
        );
    }
}

#[test]
fn standalone_and_legacy_seed_shapes_require_exact_fields() {
    assert_eq!(
        parse_open_order_fixture(USER_ORDER),
        Err(PmPrivateFixtureError::MalformedJson)
    );
    assert_eq!(
        parse_open_order_fixture(br#"{"id":"missing-the-rest"}"#),
        Err(PmPrivateFixtureError::MalformedJson)
    );
    assert_eq!(
        parse_legacy_balance_allowance_fixture(
            br#"{"balance":"1","allowance":"2","allowances":{}}"#
        ),
        Err(PmPrivateFixtureError::MalformedJson)
    );
    assert_eq!(
        parse_legacy_balance_allowance_fixture(br#"{"balance":"1"}"#),
        Err(PmPrivateFixtureError::MalformedJson)
    );
    assert_eq!(
        parse_legacy_balance_allowance_fixture(br#"{"balance":"","allowance":"2"}"#),
        Err(PmPrivateFixtureError::EmptyField("balance"))
    );
}

#[test]
fn nested_maker_legs_are_strict_and_bounded() {
    let missing_field = br#"{
        "event_type":"trade","id":"t","market":"m","asset_id":"1","side":"BUY",
        "size":"1","price":"0.4","status":"MATCHED","maker_address":"maker",
        "transaction_hash":"tx","maker_orders":[{"order_id":"o"}]
    }"#;
    assert_eq!(
        parse_private_user_fixture(missing_field),
        Err(PmPrivateFixtureError::MalformedJson)
    );

    let unknown_field = br#"{
        "event_type":"trade","id":"t","market":"m","asset_id":"1","side":"BUY",
        "size":"1","price":"0.4","status":"MATCHED","maker_address":"maker",
        "transaction_hash":"tx","maker_orders":[{
            "order_id":"o","asset_id":"1","side":"SELL","price":"0.4",
            "matched_amount":"1","owner":"unknown"
        }]
    }"#;
    assert_eq!(
        parse_private_user_fixture(unknown_field),
        Err(PmPrivateFixtureError::MalformedJson)
    );

    let leg = r#"{"order_id":"o","asset_id":"1","side":"SELL","price":"0.4","matched_amount":"1"}"#;
    let too_many_legs = format!(
        r#"{{"event_type":"trade","id":"t","market":"m","asset_id":"1","side":"BUY","size":"1","price":"0.4","status":"MATCHED","maker_address":"maker","transaction_hash":"tx","maker_orders":[{}]}}"#,
        vec![leg; MAX_PRIVATE_FIXTURE_EVENTS + 1].join(",")
    );
    assert_eq!(
        parse_private_user_fixture(too_many_legs.as_bytes()),
        Err(PmPrivateFixtureError::TooManyMakerOrders)
    );
}

#[test]
fn private_fixture_byte_and_event_limits_fail_closed() {
    let oversized = vec![b' '; MAX_PRIVATE_FIXTURE_BYTES + 1];
    assert_eq!(
        parse_private_user_fixture(&oversized),
        Err(PmPrivateFixtureError::PayloadTooLarge)
    );
    assert_eq!(
        parse_open_order_fixture(&oversized),
        Err(PmPrivateFixtureError::PayloadTooLarge)
    );
    assert_eq!(
        parse_legacy_balance_allowance_fixture(&oversized),
        Err(PmPrivateFixtureError::PayloadTooLarge)
    );
    assert_eq!(
        parse_private_user_fixture(b"[]"),
        Err(PmPrivateFixtureError::EmptyUserFrame)
    );

    let event = String::from_utf8_lossy(USER_ORDER);
    let too_many = format!(
        "[{}]",
        vec![event.as_ref(); MAX_PRIVATE_FIXTURE_EVENTS + 1].join(",")
    );
    assert_eq!(
        parse_private_user_fixture(too_many.as_bytes()),
        Err(PmPrivateFixtureError::TooManyEvents)
    );
}
