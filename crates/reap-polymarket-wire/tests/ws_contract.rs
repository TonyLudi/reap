mod support;

use reap_pm_core::{PmBookQuantity, PmBookSide};
use reap_polymarket_wire::{
    MAX_BOOK_LEVELS, MAX_WS_EVENTS_PER_FRAME, PmIgnoredEvent, PmWireError, PmWsEvent,
    compute_snapshot_hash, parse_ws_frame,
};

fn valid_snapshot() -> String {
    let placeholder = support::snapshot_json("");
    let hash = compute_snapshot_hash(placeholder.as_bytes()).unwrap();
    support::snapshot_json(&hash.to_string())
}

fn snapshot_with_last_trade(last_trade_price: &str) -> String {
    let placeholder = support::snapshot_json_with_last_trade("", last_trade_price);
    let hash = compute_snapshot_hash(placeholder.as_bytes()).unwrap();
    support::snapshot_json_with_last_trade(&hash.to_string(), last_trade_price)
}

fn bbo_with_size_fields(size_fields: &str) -> String {
    format!(
        r#"{{
          "event_type":"best_bid_ask",
          "market":"{}",
          "asset_id":"123",
          "timestamp":"123456791",
          "best_bid":"0.50",
          "best_ask":"0.60"
          {size_fields}
        }}"#,
        support::MARKET
    )
}

#[test]
fn single_snapshot_is_fully_validated_before_exposure() {
    let frame = parse_ws_frame(valid_snapshot().as_bytes(), support::book_config()).unwrap();
    assert!(!frame.was_array());
    assert_eq!(frame.events().len(), 1);
    let PmWsEvent::BookSnapshot(snapshot) = &frame.events()[0] else {
        panic!("snapshot event");
    };
    assert_eq!(snapshot.bids().len(), 2);
    assert_eq!(snapshot.asks().len(), 2);
    assert_eq!(snapshot.bids()[0].raw_price(), "0.30");
    assert_eq!(snapshot.bids()[1].level().price().to_string(), "0.4");
    assert_eq!(snapshot.verified_hash().to_string().len(), 40);
    assert_eq!(snapshot.timestamp_millis(), 123_456_789);
}

#[test]
fn array_frame_keeps_delta_atomic_and_bbo_price_only() {
    let raw = format!(
        r#"[
          {{
            "event_type":"price_change",
            "market":"{}",
            "timestamp":"123456790",
            "price_changes":[
              {{
                "asset_id":"123",
                "price":"0.40",
                "size":"0",
                "side":"BUY",
                "hash":"tx-delete",
                "best_bid":"0.30",
                "best_ask":"0.60"
              }},
              {{
                "asset_id":"123",
                "price":"0.50",
                "size":"12.5",
                "side":"BUY",
                "hash":"tx-add",
                "best_bid":"0.50",
                "best_ask":"0.60"
              }}
            ]
          }},
          {{
            "event_type":"best_bid_ask",
            "market":"{}",
            "asset_id":"123",
            "timestamp":"123456791",
            "best_bid":"0.50",
            "best_ask":"0.60",
            "bid_size":"12.5",
            "ask_size":"75"
          }}
        ]"#,
        support::MARKET,
        support::MARKET
    );
    let frame = parse_ws_frame(raw.as_bytes(), support::book_config()).unwrap();
    assert!(frame.was_array());
    let PmWsEvent::PriceChanges(batch) = &frame.events()[0] else {
        panic!("delta event");
    };
    assert_eq!(batch.changes().len(), 2);
    assert_eq!(
        batch.changes()[0].level().quantity(),
        PmBookQuantity::Delete
    );
    assert_eq!(batch.changes()[1].level().side(), PmBookSide::Bid);
    assert_eq!(batch.changes()[1].transaction_hash().unwrap(), "tx-add");
    assert_eq!(batch.final_best_prices().bid().to_string(), "0.5");

    let PmWsEvent::BestBidAsk(top) = &frame.events()[1] else {
        panic!("BBO event");
    };
    assert_eq!(top.prices().ask().to_string(), "0.6");
}

#[test]
fn any_invalid_change_rejects_the_whole_batch() {
    let raw = format!(
        r#"{{
          "event_type":"price_change",
          "market":"{}",
          "timestamp":"123456790",
          "price_changes":[
            {{"asset_id":"123","price":"0.40","size":"1","side":"BUY","best_bid":"0.40","best_ask":"0.60"}},
            {{"asset_id":"999","price":"0.50","size":"1","side":"BUY","best_bid":"0.50","best_ask":"0.60"}}
          ]
        }}"#,
        support::MARKET
    );
    assert_eq!(
        parse_ws_frame(raw.as_bytes(), support::book_config()),
        Err(PmWireError::TokenMismatch)
    );
}

#[test]
fn tick_change_is_an_invalidation() {
    let tick = format!(
        r#"{{
          "event_type":"tick_size_change",
          "market":"{}",
          "asset_id":"123",
          "timestamp":"123456792",
          "old_tick_size":"0.01",
          "new_tick_size":"0.001"
        }}"#,
        support::MARKET
    );
    let frame = parse_ws_frame(tick.as_bytes(), support::book_config()).unwrap();
    let PmWsEvent::TickSizeChange(change) = &frame.events()[0] else {
        panic!("tick invalidation");
    };
    assert_eq!(change.old_tick().to_string(), "0.01");
    assert_eq!(change.new_tick().to_string(), "0.001");
}

#[test]
fn multiplexed_trade_is_discriminator_only_and_never_parses_trade_fields() {
    for raw in [
        br#"{"event_type":"last_trade_price"}"#.as_slice(),
        include_bytes!("../fixtures/ignored_last_trade_unparsed_fields.json").as_slice(),
    ] {
        let frame = parse_ws_frame(raw, support::book_config()).unwrap();
        assert_eq!(
            frame.events(),
            &[PmWsEvent::Ignored(PmIgnoredEvent::PublicTrade)]
        );
    }
}

#[test]
fn snapshot_last_trade_is_closed_interval_integrity_evidence_not_an_executable_price() {
    for last_trade in ["0", "0.0000000", "1", "1.0000000", "0.1234567"] {
        let raw = snapshot_with_last_trade(last_trade);
        let frame = parse_ws_frame(raw.as_bytes(), support::book_config()).unwrap();
        assert!(matches!(frame.events(), [PmWsEvent::BookSnapshot(_)]));
    }

    for last_trade in [
        "-0.1", "+0.1", ".5", "0.", "0..5", "1e0", "NaN", "1.000001", "2",
    ] {
        let raw = snapshot_with_last_trade(last_trade);
        assert_eq!(
            parse_ws_frame(raw.as_bytes(), support::book_config()),
            Err(PmWireError::InvalidNumeric("last_trade_price")),
            "{last_trade}"
        );
    }
}

#[test]
fn standalone_bbo_sizes_are_an_optional_exact_positive_pair_and_are_not_normalized() {
    for size_fields in [
        "",
        r#","bid_size":"0.000001","ask_size":"999999999.123456""#,
    ] {
        let frame = parse_ws_frame(
            bbo_with_size_fields(size_fields).as_bytes(),
            support::book_config(),
        )
        .unwrap();
        let PmWsEvent::BestBidAsk(top) = &frame.events()[0] else {
            panic!("price-only BBO");
        };
        assert_eq!(top.prices().bid().to_string(), "0.5");
        assert_eq!(top.prices().ask().to_string(), "0.6");
    }

    for size_fields in [r#","bid_size":"1""#, r#","ask_size":"1""#] {
        assert_eq!(
            parse_ws_frame(
                bbo_with_size_fields(size_fields).as_bytes(),
                support::book_config()
            ),
            Err(PmWireError::PartialBestSizes)
        );
    }

    for (size_fields, field) in [
        (r#","bid_size":"0","ask_size":"1""#, "bid_size"),
        (r#","bid_size":"1","ask_size":"0""#, "ask_size"),
        (r#","bid_size":"-1","ask_size":"1""#, "bid_size"),
        (r#","bid_size":"1","ask_size":".""#, "ask_size"),
    ] {
        assert_eq!(
            parse_ws_frame(
                bbo_with_size_fields(size_fields).as_bytes(),
                support::book_config()
            ),
            Err(PmWireError::InvalidNumeric(field))
        );
    }
}

#[test]
fn invalid_prices_quantities_books_and_scope_fail_closed() {
    let snapshot = valid_snapshot();
    for (needle, replacement, expected) in [
        ("\"0.30\"", "\"0\"", PmWireError::InvalidNumeric("price")),
        ("\"0.60\"", "\"1\"", PmWireError::InvalidNumeric("price")),
        ("\"0.30\"", "\"0.305\"", PmWireError::PriceOffConfiguredTick),
        (
            "\"size\":\"100\"",
            "\"size\":\"-1\"",
            PmWireError::InvalidNumeric("size"),
        ),
    ] {
        let raw = snapshot.replacen(needle, replacement, 1);
        assert_eq!(
            parse_ws_frame(raw.as_bytes(), support::book_config()),
            Err(expected)
        );
    }

    let crossed = snapshot.replace("\"0.60\"", "\"0.40\"");
    assert_eq!(
        parse_ws_frame(crossed.as_bytes(), support::book_config()),
        Err(PmWireError::CrossedBook)
    );
    let empty = snapshot.replace(
        r#""bids":[{"price":"0.30","size":"100"},{"price":"0.40","size":"50"}]"#,
        r#""bids":[] "#,
    );
    assert_eq!(
        parse_ws_frame(empty.as_bytes(), support::book_config()),
        Err(PmWireError::EmptyBook)
    );
    let wrong_market = snapshot.replace(support::MARKET, support::CONDITION);
    assert_eq!(
        parse_ws_frame(wrong_market.as_bytes(), support::book_config()),
        Err(PmWireError::MarketMismatch)
    );

    let duplicate = snapshot.replace(
        r#""bids":[{"price":"0.30","size":"100"},{"price":"0.40","size":"50"}]"#,
        r#""bids":[{"price":"0.30","size":"100"},{"price":"0.30","size":"50"}]"#,
    );
    assert_eq!(
        parse_ws_frame(duplicate.as_bytes(), support::book_config()),
        Err(PmWireError::DuplicateLevel)
    );

    let zero_size = snapshot.replacen(r#""size":"100""#, r#""size":"0""#, 1);
    assert_eq!(
        parse_ws_frame(zero_size.as_bytes(), support::book_config()),
        Err(PmWireError::InvalidNumeric("size"))
    );
}

#[test]
fn frame_and_level_bounds_are_enforced() {
    let event = format!(
        r#"{{"event_type":"last_trade_price","market":"{}","asset_id":"123"}}"#,
        support::MARKET
    );
    let too_many_events = format!(
        "[{}]",
        std::iter::repeat_n(event, MAX_WS_EVENTS_PER_FRAME + 1)
            .collect::<Vec<_>>()
            .join(",")
    );
    assert_eq!(
        parse_ws_frame(too_many_events.as_bytes(), support::book_config()),
        Err(PmWireError::TooManyEvents)
    );

    let levels = std::iter::repeat_n(r#"{"price":"0.30","size":"1"}"#, MAX_BOOK_LEVELS + 1)
        .collect::<Vec<_>>()
        .join(",");
    let oversized = format!(
        r#"{{
          "event_type":"book","market":"{}","asset_id":"123","timestamp":"1","hash":"0000000000000000000000000000000000000001",
          "bids":[{}],"asks":[{{"price":"0.60","size":"1"}}],
          "min_order_size":"5","tick_size":"0.01","neg_risk":false,"last_trade_price":"0.50"
        }}"#,
        support::MARKET,
        levels
    );
    assert_eq!(
        parse_ws_frame(oversized.as_bytes(), support::book_config()),
        Err(PmWireError::TooManyBookLevels)
    );
}

#[test]
fn malformed_unknown_and_integrity_incomplete_events_are_typed_failures() {
    assert_eq!(
        parse_ws_frame(b"{", support::book_config()),
        Err(PmWireError::MalformedJson)
    );
    let unknown = format!(
        r#"{{"event_type":"mystery","market":"{}","asset_id":"123"}}"#,
        support::MARKET
    );
    assert_eq!(
        parse_ws_frame(unknown.as_bytes(), support::book_config()),
        Err(PmWireError::UnsupportedEventType)
    );
    let no_top = format!(
        r#"{{
          "event_type":"price_change","market":"{}","timestamp":"2",
          "price_changes":[{{"asset_id":"123","price":"0.40","size":"1","side":"BUY"}}]
        }}"#,
        support::MARKET
    );
    assert_eq!(
        parse_ws_frame(no_top.as_bytes(), support::book_config()),
        Err(PmWireError::MissingBestPrices)
    );
    let negative_delta = no_top.replace(r#""size":"1""#, r#""size":"-1""#).replace(
        r#""side":"BUY""#,
        r#""side":"BUY","best_bid":"0.40","best_ask":"0.60""#,
    );
    assert_eq!(
        parse_ws_frame(negative_delta.as_bytes(), support::book_config()),
        Err(PmWireError::InvalidNumeric("size"))
    );
}
