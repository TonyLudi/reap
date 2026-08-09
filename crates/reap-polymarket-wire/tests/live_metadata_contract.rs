mod support;

use reap_pm_core::{PmConditionId, PmTick};
use reap_polymarket_wire::{
    PmBookMarketBinding, PmBookParserConfig, PmClobV2RequestScope, PmWireError,
    compute_snapshot_hash, parse_live_clob_market_lifecycle,
    parse_live_clob_market_lifecycle_details, parse_live_clob_v2_metadata,
    parse_rest_book_snapshot,
};

// Contract sources:
// - Polymarket/clob-client-v2 f3e1a05f868a1fd0c34ef85dfc45c6ce78f5bb69,
//   src/endpoints.ts, src/client.ts, and src/types/clob.ts.
// - Predarb 8222273a9c72033b760e1d2fec813bc77144556d,
//   crates/venue-polymarket/src/rest/public.rs.
const OTHER_CONDITION: &str = "0xcccccccccccccccccccccccccccccccccccccccccccccccccccccccccccccccc";
const OTHER_MARKET: &str = "0xdddddddddddddddddddddddddddddddddddddddddddddddddddddddddddddddd";

fn request_scope() -> PmClobV2RequestScope {
    PmClobV2RequestScope::new(
        PmConditionId::parse(support::CONDITION).unwrap(),
        support::scope().token(),
    )
}

fn long_market() -> String {
    format!(
        r#"{{
          "enable_order_book":true,
          "active":true,
          "closed":false,
          "archived":false,
          "accepting_orders":true,
          "accepting_order_timestamp":"2026-08-08T00:00:00Z",
          "minimum_order_size":5,
          "minimum_tick_size":0.01,
          "condition_id":"{}",
          "question_id":"{}",
          "question":"Will the exact parser remain deterministic?",
          "description":"synthetic protocol vector",
          "market_slug":"exact-parser-vector",
          "end_date_iso":"2027-01-01T00:00:00Z",
          "game_start_time":null,
          "seconds_delay":0,
          "fpmm":"0x0000000000000000000000000000000000000000",
          "maker_base_fee":0,
          "taker_base_fee":0,
          "notifications_enabled":true,
          "neg_risk":false,
          "neg_risk_market_id":"",
          "neg_risk_request_id":"",
          "icon":"https://example.invalid/icon.png",
          "image":"https://example.invalid/image.png",
          "rewards":{{"rates":null,"min_size":0,"max_spread":0}},
          "is_50_50_outcome":false,
          "tokens":[{{"token_id":"123","outcome":"Yes","price":0.5,"winner":false}}],
          "tags":["synthetic"]
        }}"#,
        support::CONDITION,
        support::MARKET,
    )
}

fn short_market(condition: Option<&str>, tick: &str, minimum: &str, nr: Option<&str>) -> String {
    let condition = condition
        .map(|value| format!(r#", "c":"{value}""#))
        .unwrap_or_default();
    let nr = nr
        .map(|value| format!(r#", "nr":{value}"#))
        .unwrap_or_default();
    format!(
        r#"{{
          "t":[{{"t":"123","o":"Yes"}},{{"t":"456","o":"No"}}],
          "mts":{tick},
          "mos":{minimum},
          "r":{{"rates":null}},
          "fd":{{"r":0.02,"e":2,"to":true}},
          "mbf":0,
          "tbf":0,
          "ao":true,
          "sd":0,
          "gst":null,
          "cbos":true,
          "aot":"2026-08-08T00:00:00Z",
          "rfqe":false,
          "itode":false,
          "oas":0,
          "ibce":true
          {condition}{nr}
        }}"#
    )
}

#[test]
fn current_long_route_owns_exact_lifecycle_and_question_identity() {
    let parsed =
        parse_live_clob_market_lifecycle_details(long_market().as_bytes(), support::scope())
            .expect("reviewed long response");
    let metadata = parsed.metadata();
    assert_eq!(metadata.condition(), support::scope().condition());
    assert_eq!(metadata.market(), support::scope().market());
    assert!(metadata.lifecycle().active());
    assert!(!metadata.lifecycle().closed());
    assert!(!metadata.lifecycle().archived());
    assert!(metadata.lifecycle().accepting_orders());
    assert!(metadata.lifecycle().order_book_enabled());
    assert_eq!(
        parsed
            .details()
            .accepting_order_timestamp()
            .unwrap()
            .as_str(),
        "2026-08-08T00:00:00Z"
    );
    assert_eq!(
        parsed.details().end_date_iso().as_str(),
        "2027-01-01T00:00:00Z"
    );
    assert_eq!(parsed.details().game_start_time(), None);
    assert_eq!(parsed.details().seconds_delay(), 0);
    assert_eq!(
        parse_live_clob_market_lifecycle(long_market().as_bytes(), support::scope()).unwrap(),
        *metadata
    );

    let wrong_condition = long_market().replace(support::CONDITION, OTHER_CONDITION);
    assert_eq!(
        parse_live_clob_market_lifecycle(wrong_condition.as_bytes(), support::scope()),
        Err(PmWireError::ConditionMismatch)
    );
    let wrong_market = long_market().replace(support::MARKET, OTHER_MARKET);
    assert_eq!(
        parse_live_clob_market_lifecycle(wrong_market.as_bytes(), support::scope()),
        Err(PmWireError::MarketMismatch)
    );
}

#[test]
fn long_lifecycle_detail_missing_null_type_and_bounds_match_the_frozen_shape() {
    let valid = long_market();
    let without_accepting = valid.replace(
        "          \"accepting_order_timestamp\":\"2026-08-08T00:00:00Z\",\n",
        "",
    );
    assert_eq!(
        parse_live_clob_market_lifecycle_details(without_accepting.as_bytes(), support::scope())
            .unwrap()
            .details()
            .accepting_order_timestamp(),
        None
    );
    let without_game = valid.replace("          \"game_start_time\":null,\n", "");
    assert_eq!(
        parse_live_clob_market_lifecycle_details(without_game.as_bytes(), support::scope())
            .unwrap()
            .details()
            .game_start_time(),
        None
    );
    let scheduled = valid
        .replace(
            r#""game_start_time":null"#,
            r#""game_start_time":"2026-12-31T00:00:00Z""#,
        )
        .replace(r#""seconds_delay":0"#, r#""seconds_delay":17"#);
    let scheduled =
        parse_live_clob_market_lifecycle_details(scheduled.as_bytes(), support::scope()).unwrap();
    assert_eq!(
        scheduled.details().game_start_time().unwrap().as_str(),
        "2026-12-31T00:00:00Z"
    );
    assert_eq!(scheduled.details().seconds_delay(), 17);

    for (raw, error) in [
        (
            valid.replace(
                r#""accepting_order_timestamp":"2026-08-08T00:00:00Z""#,
                r#""accepting_order_timestamp":null"#,
            ),
            PmWireError::NullField("accepting_order_timestamp"),
        ),
        (
            valid.replace("          \"end_date_iso\":\"2027-01-01T00:00:00Z\",\n", ""),
            PmWireError::MissingField("end_date_iso"),
        ),
        (
            valid.replace(
                r#""end_date_iso":"2027-01-01T00:00:00Z""#,
                r#""end_date_iso":null"#,
            ),
            PmWireError::NullField("end_date_iso"),
        ),
        (
            valid.replace("          \"seconds_delay\":0,\n", ""),
            PmWireError::MissingField("seconds_delay"),
        ),
        (
            valid.replace(r#""seconds_delay":0"#, r#""seconds_delay":null"#),
            PmWireError::NullField("seconds_delay"),
        ),
        (
            valid.replace(
                r#""end_date_iso":"2027-01-01T00:00:00Z""#,
                &format!(r#""end_date_iso":"{}""#, "x".repeat(129)),
            ),
            PmWireError::FieldTooLong("end_date_iso"),
        ),
        (
            valid.replace(
                r#""end_date_iso":"2027-01-01T00:00:00Z""#,
                r#""end_date_iso":"""#,
            ),
            PmWireError::InvalidIdentity("end_date_iso"),
        ),
        (
            valid.replace(
                r#""end_date_iso":"2027-01-01T00:00:00Z""#,
                r#""end_date_iso":"2027-été""#,
            ),
            PmWireError::NonAsciiField("end_date_iso"),
        ),
    ] {
        assert_eq!(
            parse_live_clob_market_lifecycle_details(raw.as_bytes(), support::scope()),
            Err(error)
        );
    }

    for raw in [
        valid.replace(
            r#""accepting_order_timestamp":"2026-08-08T00:00:00Z""#,
            r#""accepting_order_timestamp":true"#,
        ),
        valid.replace(r#""game_start_time":null"#, r#""game_start_time":7"#),
        valid.replace(r#""seconds_delay":0"#, r#""seconds_delay":"0""#),
        valid.replace(
            r#""end_date_iso":"2027-01-01T00:00:00Z""#,
            r#""end_date_iso":"2027-01-01T00:00:00Z","end_date_iso":"2027-01-02T00:00:00Z""#,
        ),
    ] {
        assert_eq!(
            parse_live_clob_market_lifecycle_details(raw.as_bytes(), support::scope()),
            Err(PmWireError::MalformedJson)
        );
    }
}

#[test]
fn long_route_requires_authority_fields_and_rejects_unreviewed_extensions() {
    let missing_question = long_market().replace(
        &format!(
            r#"          "question_id":"{}",
"#,
            support::MARKET
        ),
        "",
    );
    assert_eq!(
        parse_live_clob_market_lifecycle(missing_question.as_bytes(), support::scope()),
        Err(PmWireError::MissingField("question_id"))
    );

    let wrong_lifecycle_type = long_market().replace(r#""active":true"#, r#""active":"true""#);
    assert_eq!(
        parse_live_clob_market_lifecycle(wrong_lifecycle_type.as_bytes(), support::scope()),
        Err(PmWireError::MalformedJson)
    );

    let unknown = long_market().replacen('{', r#"{"unreviewed":1,"#, 1);
    assert_eq!(
        parse_live_clob_market_lifecycle(unknown.as_bytes(), support::scope()),
        Err(PmWireError::MalformedJson)
    );
}

#[test]
fn abbreviated_route_retains_request_provenance_without_fabricating_market_identity() {
    let omitted = parse_live_clob_v2_metadata(
        short_market(None, "0.01", "5", None).as_bytes(),
        request_scope(),
    )
    .expect("condition and nr may be omitted by the reviewed response");
    assert_eq!(omitted.requested_condition(), request_scope().condition());
    assert_eq!(omitted.reported_condition(), None);
    assert!(!omitted.negative_risk());
    assert_eq!(omitted.configured_outcome().label().as_str(), "Yes");
    assert_eq!(omitted.tokens().len(), 2);
    assert_eq!(omitted.maker_base_fee_bps(), 0);
    assert_eq!(omitted.taker_base_fee_bps(), 0);
    assert_eq!(omitted.fee_details().rate().unwrap().as_str(), "0.02");
    assert_eq!(omitted.fee_details().exponent().unwrap().as_str(), "2");
    assert_eq!(omitted.fee_details().taker_only(), Some(true));
    assert!(!omitted.take_only_delay_enabled());
    assert_eq!(omitted.minimum_order_age_seconds(), 0);

    let reported = parse_live_clob_v2_metadata(
        short_market(Some(support::CONDITION), "0.01", "5", Some("true")).as_bytes(),
        request_scope(),
    )
    .expect("matching reported condition");
    assert_eq!(
        reported.reported_condition(),
        Some(request_scope().condition())
    );
    assert!(reported.negative_risk());

    assert_eq!(
        parse_live_clob_v2_metadata(
            short_market(Some(OTHER_CONDITION), "0.01", "5", None).as_bytes(),
            request_scope(),
        ),
        Err(PmWireError::ConditionMismatch)
    );
}

#[test]
fn abbreviated_optional_fields_default_only_when_omitted() {
    for (field, expected) in [
        (r#""nr":null"#, PmWireError::MalformedJson),
        (r#""nr":"false""#, PmWireError::MalformedJson),
        (r#""c":null"#, PmWireError::MalformedJson),
        (r#""c":1"#, PmWireError::MalformedJson),
    ] {
        let raw = if field.starts_with(r#""nr"#) {
            short_market(None, "0.01", "5", Some(field.split_once(':').unwrap().1))
        } else {
            short_market(None, "0.01", "5", None).replacen('{', &format!("{{{field},"), 1)
        };
        assert_eq!(
            parse_live_clob_v2_metadata(raw.as_bytes(), request_scope()),
            Err(expected),
            "{field}"
        );
    }
}

#[test]
fn abbreviated_numerics_are_parsed_from_their_exact_json_lexemes() {
    let exact_large = parse_live_clob_v2_metadata(
        short_market(None, "0.01", "9007199254740993", None).as_bytes(),
        request_scope(),
    )
    .expect("integer beyond the f64 exact range remains exact");
    assert_eq!(
        exact_large.minimum_order_size().to_string(),
        "9007199254740993"
    );

    for tick in ["0.1", "0.01", "0.005", "0.0025", "0.001", "0.0001"] {
        let parsed = parse_live_clob_v2_metadata(
            short_market(None, tick, "5", None).as_bytes(),
            request_scope(),
        )
        .expect("one of the six exact protocol ticks");
        assert_eq!(parsed.tick(), PmTick::parse_decimal(tick).unwrap());
    }

    for (tick, minimum, field) in [
        ("1e-2", "5", "minimum_tick_size"),
        ("0.0100001", "5", "minimum_tick_size"),
        (r#""0.01""#, "5", "minimum_tick_size"),
        ("0.01", "5e0", "minimum_order_size"),
        ("0.01", "5.0000001", "minimum_order_size"),
        ("0.01", "5.000001", "minimum_order_size"),
        ("0.01", r#""5""#, "minimum_order_size"),
    ] {
        assert_eq!(
            parse_live_clob_v2_metadata(
                short_market(None, tick, minimum, None).as_bytes(),
                request_scope(),
            ),
            Err(PmWireError::InvalidNumeric(field)),
            "tick={tick}, minimum={minimum}"
        );
    }
}

#[test]
fn abbreviated_membership_required_fields_and_unknowns_fail_closed() {
    let valid = short_market(None, "0.01", "5", None);
    let missing_token = valid.replace(r#""t":"123""#, r#""t":"789""#);
    assert_eq!(
        parse_live_clob_v2_metadata(missing_token.as_bytes(), request_scope()),
        Err(PmWireError::ConfiguredTokenMissing)
    );

    let duplicate = valid.replace(r#""t":"456""#, r#""t":"123""#);
    assert_eq!(
        parse_live_clob_v2_metadata(duplicate.as_bytes(), request_scope()),
        Err(PmWireError::DuplicateToken)
    );

    let extra_token = valid.replace(
        r#"{"t":"456","o":"No"}"#,
        r#"{"t":"456","o":"No"},{"t":"789","o":"Other"}"#,
    );
    assert_eq!(
        parse_live_clob_v2_metadata(extra_token.as_bytes(), request_scope()),
        Err(PmWireError::UnexpectedMarketTokenCount)
    );

    for (needle, field) in [
        (
            r#"          "mts":0.01,
"#,
            "minimum_tick_size",
        ),
        (
            r#"          "mos":5,
"#,
            "minimum_order_size",
        ),
        (
            r#"          "t":[{"t":"123","o":"Yes"},{"t":"456","o":"No"}],
"#,
            "tokens",
        ),
        (
            r#"          "mbf":0,
"#,
            "maker_base_fee",
        ),
        (
            r#"          "tbf":0,
"#,
            "taker_base_fee",
        ),
        (
            r#"          "fd":{"r":0.02,"e":2,"to":true},
"#,
            "fee_details",
        ),
        (
            r#"          "oas":0,
"#,
            "minimum_order_age_seconds",
        ),
    ] {
        let missing = valid.replace(needle, "");
        assert_eq!(
            parse_live_clob_v2_metadata(missing.as_bytes(), request_scope()),
            Err(PmWireError::MissingField(field)),
            "{field}"
        );
    }

    let unknown_top = valid.replacen('{', r#"{"unreviewed":1,"#, 1);
    assert_eq!(
        parse_live_clob_v2_metadata(unknown_top.as_bytes(), request_scope()),
        Err(PmWireError::MalformedJson)
    );
    let unknown_token = valid.replace(r#"{"t":"123","o":"Yes"}"#, r#"{"t":"123","o":"Yes","x":1}"#);
    assert_eq!(
        parse_live_clob_v2_metadata(unknown_token.as_bytes(), request_scope()),
        Err(PmWireError::MalformedJson)
    );

    let null_fee_details = valid.replace(r#""fd":{"r":0.02,"e":2,"to":true}"#, r#""fd":null"#);
    assert_eq!(
        parse_live_clob_v2_metadata(null_fee_details.as_bytes(), request_scope()),
        Err(PmWireError::NullField("fee_details"))
    );
}

#[test]
fn abbreviated_fee_delay_and_order_age_fields_are_exact_and_typed() {
    let valid = short_market(None, "0.01", "5", None);
    let delayed = valid.replace(r#""itode":false"#, r#""itode":true"#);
    assert!(
        parse_live_clob_v2_metadata(delayed.as_bytes(), request_scope())
            .unwrap()
            .take_only_delay_enabled()
    );

    for (raw, field) in [
        (valid.replace(r#""mbf":0"#, r#""mbf":-1"#), "maker_base_fee"),
        (
            valid.replace(r#""tbf":0"#, r#""tbf":0.5"#),
            "taker_base_fee",
        ),
        (
            valid.replace(r#""oas":0"#, r#""oas":"0""#),
            "minimum_order_age_seconds",
        ),
    ] {
        assert_eq!(
            parse_live_clob_v2_metadata(raw.as_bytes(), request_scope()),
            Err(PmWireError::MalformedJson),
            "{field}"
        );
    }

    for exact in ["0.0", "0.020", "1e-2"] {
        let raw = valid.replace("0.02", exact);
        assert_eq!(
            parse_live_clob_v2_metadata(raw.as_bytes(), request_scope())
                .unwrap()
                .fee_details()
                .rate()
                .unwrap()
                .as_str(),
            exact
        );
    }

    for malformed in ["-0.02", "\"0.02\""] {
        let raw = valid.replace("0.02", malformed);
        assert_eq!(
            parse_live_clob_v2_metadata(raw.as_bytes(), request_scope()),
            Err(PmWireError::InvalidNumeric("fee_details.rate")),
            "{malformed}"
        );
    }

    let duplicate_fee_member = valid.replace(
        r#""fd":{"r":0.02,"e":2,"to":true}"#,
        r#""fd":{"r":0.02,"r":0.03,"e":2,"to":true}"#,
    );
    assert_eq!(
        parse_live_clob_v2_metadata(duplicate_fee_member.as_bytes(), request_scope()),
        Err(PmWireError::MalformedJson)
    );
}

#[test]
fn condition_bound_book_mode_validates_wire_condition_and_normalizes_known_market() {
    let legacy = support::book_config();
    let live = PmBookParserConfig::new_condition_bound(
        legacy.scope(),
        legacy.tick(),
        legacy.minimum_order_size(),
        legacy.negative_risk(),
    );
    assert_eq!(live.market_binding(), PmBookMarketBinding::ConditionId);

    let placeholder = support::snapshot_json("").replace(support::MARKET, support::CONDITION);
    let hash = compute_snapshot_hash(placeholder.as_bytes()).unwrap();
    let raw =
        support::snapshot_json(&hash.to_string()).replace(support::MARKET, support::CONDITION);

    assert_eq!(
        parse_rest_book_snapshot(raw.as_bytes(), legacy),
        Err(PmWireError::MarketMismatch)
    );
    let snapshot = parse_rest_book_snapshot(raw.as_bytes(), live)
        .expect("current live market field is the configured condition");
    assert_eq!(snapshot.market(), support::scope().market());

    let wrong_condition = raw.replace(support::CONDITION, OTHER_CONDITION);
    assert_eq!(
        parse_rest_book_snapshot(wrong_condition.as_bytes(), live),
        Err(PmWireError::ConditionMismatch)
    );
}
