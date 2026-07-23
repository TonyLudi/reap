mod support;

use reap_polymarket_wire::{
    PmWireError, compute_snapshot_hash, parse_clob_metadata, parse_lifecycle_metadata,
    parse_rest_book_snapshot, parse_server_time,
};

#[test]
fn lifecycle_and_clob_metadata_are_exact_and_scope_checked() {
    let lifecycle = parse_lifecycle_metadata(
        include_bytes!("../fixtures/valid_lifecycle.json"),
        support::scope(),
    )
    .unwrap();
    assert!(lifecycle.lifecycle().active());
    assert!(!lifecycle.lifecycle().closed());
    assert!(!lifecycle.lifecycle().archived());
    assert!(lifecycle.lifecycle().accepting_orders());
    assert!(lifecycle.lifecycle().order_book_enabled());

    let clob = parse_clob_metadata(
        include_bytes!("../fixtures/valid_clob_market.json"),
        support::scope(),
    )
    .unwrap();
    assert_eq!(clob.tick().to_string(), "0.01");
    assert_eq!(clob.minimum_order_size().to_string(), "5");
    assert!(!clob.negative_risk());
    assert_eq!(clob.configured_outcome().label().as_str(), "Yes");
    assert_eq!(clob.tokens().len(), 2);
}

#[test]
fn server_time_accepts_only_positive_exact_integer_forms() {
    assert_eq!(parse_server_time(br#"123456"#).unwrap(), 123_456);
    assert_eq!(
        parse_server_time(br#"{"timestamp":123456}"#).unwrap(),
        123_456
    );
    assert_eq!(
        parse_server_time(br#"{"timestamp":0}"#),
        Err(PmWireError::InvalidServerTime)
    );
    assert_eq!(
        parse_server_time(br#"{"timestamp":1.5}"#),
        Err(PmWireError::MalformedJson)
    );
}

#[test]
fn wrong_or_ambiguous_token_membership_fails_closed() {
    let raw = include_str!("../fixtures/valid_clob_market.json");
    let missing = raw.replace("\"123\"", "\"789\"");
    assert_eq!(
        parse_clob_metadata(missing.as_bytes(), support::scope()),
        Err(PmWireError::ConfiguredTokenMissing)
    );

    let duplicate = raw.replace(
        r#"{"token_id": "456", "outcome": "No"}"#,
        r#"{"token_id": "123", "outcome": "No"}"#,
    );
    assert_eq!(
        parse_clob_metadata(duplicate.as_bytes(), support::scope()),
        Err(PmWireError::DuplicateToken)
    );
}

#[test]
fn tracked_predarb_seed_is_pinned_but_not_accepted_as_complete_snapshot() {
    let raw = include_bytes!("../fixtures/market_book_predarb_seed.json");
    assert_eq!(
        parse_rest_book_snapshot(raw, support::book_config()),
        Err(PmWireError::InvalidIdentity("market"))
    );
}

#[test]
fn complete_rest_snapshot_uses_the_same_exact_integrity_path_without_event_type() {
    let placeholder = support::snapshot_json("").replace(r#""event_type":"book","#, "");
    let hash = compute_snapshot_hash(placeholder.as_bytes()).unwrap();
    let raw = support::snapshot_json(&hash.to_string()).replace(r#""event_type":"book","#, "");

    let snapshot = parse_rest_book_snapshot(raw.as_bytes(), support::book_config()).unwrap();
    assert_eq!(snapshot.token(), support::scope().token());
    assert_eq!(snapshot.verified_hash(), hash);
}
