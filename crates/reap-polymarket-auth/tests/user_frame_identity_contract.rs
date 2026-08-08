use reap_polymarket_auth::{
    CredentialOwnedUserFrame, L2CredentialInput, L2Credentials, PmAuthError,
};
use reap_polymarket_wire::{PmLiveUserEvent, parse_live_user_frame};

const ADDRESS: &str = "0xf39Fd6e51aad88F6F4ce6aB8827279cffFb92266";
const API_KEY: &str = "00000000-0000-4000-8000-000000000001";
const FOREIGN_API_KEY: &str = "00000000-0000-4000-8000-000000000002";
const API_SECRET: &str = "AAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAA=";
const PASSPHRASE: &str = "synthetic-passphrase";
const CONDITION: &str = "0x1111111111111111111111111111111111111111111111111111111111111111";
const ORDER_1: &str = "0xaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa";
const ORDER_2: &str = "0xbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbb";
const MAKER: &str = "0x2222222222222222222222222222222222222222";

fn credentials() -> L2Credentials {
    L2Credentials::bind(
        ADDRESS,
        L2CredentialInput::new(API_KEY.into(), API_SECRET.into(), PASSPHRASE.into()),
    )
    .unwrap()
}

fn order_frame(owner: &str, order_owner: &str) -> reap_polymarket_wire::PmLiveUserFrame {
    let raw = format!(
        r#"{{"event_type":"order","id":"{ORDER_1}","market":"{CONDITION}","asset_id":"1234","side":"BUY","original_size":"10.000000","size_matched":"0","price":"0.520000","type":"PLACEMENT","owner":"{owner}","order_owner":"{order_owner}","maker_address":"{MAKER}","expiration":"0","order_type":"GTC","outcome":"Yes","status":"LIVE","created_at":"1780449126","associate_trades":null,"timestamp":"1780449126930"}}"#
    );
    parse_live_user_frame(raw.as_bytes()).unwrap()
}

fn trade_frame(
    owner: &str,
    trade_owner: &str,
    maker_owner: &str,
) -> reap_polymarket_wire::PmLiveUserFrame {
    let raw = format!(
        r#"{{"event_type":"trade","type":"TRADE","id":"28c4d2eb-bbea-40e7-a9f0-b2fdb56b2c2e","market":"{CONDITION}","asset_id":"1234","side":"BUY","size":"10.000000","price":"0.520000","status":"MATCHED","taker_order_id":"{ORDER_1}","maker_orders":[{{"order_id":"{ORDER_2}","owner":"{maker_owner}","maker_address":"{MAKER}","matched_amount":"10.000000","price":"0.520000","asset_id":"1234","side":"SELL"}}],"owner":"{owner}","trade_owner":"{trade_owner}","timestamp":"1780449126930","last_update":"1780449126930","trader_side":"TAKER"}}"#
    );
    parse_live_user_frame(raw.as_bytes()).unwrap()
}

#[test]
fn exact_top_level_owners_bind_and_foreign_nested_maker_owner_is_preserved() {
    let credentials = credentials();
    let order = credentials
        .bind_user_stream_frame(order_frame(API_KEY, API_KEY))
        .unwrap();
    assert!(matches!(order.events(), [PmLiveUserEvent::Order(_)]));

    let trade = credentials
        .bind_user_stream_frame(trade_frame(API_KEY, API_KEY, FOREIGN_API_KEY))
        .unwrap();
    let [PmLiveUserEvent::Trade(trade_event)] = trade.events() else {
        panic!("fixture must contain one trade")
    };
    assert_eq!(trade_event.maker_orders().len(), 1);
    assert!(
        !credentials.matches_credential_owner(trade_event.maker_orders()[0].owner()),
        "a taker's maker row is counterparty evidence, not subscribed-account scope"
    );
}

#[test]
fn every_documented_top_level_credential_owner_fails_closed_independently() {
    let credentials = credentials();
    let cases = [
        (
            credentials.bind_user_stream_frame(order_frame(FOREIGN_API_KEY, API_KEY)),
            PmAuthError::UserOrderOwnerMismatch,
        ),
        (
            credentials.bind_user_stream_frame(order_frame(API_KEY, FOREIGN_API_KEY)),
            PmAuthError::UserOrderOrderOwnerMismatch,
        ),
        (
            credentials.bind_user_stream_frame(trade_frame(FOREIGN_API_KEY, API_KEY, API_KEY)),
            PmAuthError::UserTradeOwnerMismatch,
        ),
        (
            credentials.bind_user_stream_frame(trade_frame(API_KEY, FOREIGN_API_KEY, API_KEY)),
            PmAuthError::UserTradeTradeOwnerMismatch,
        ),
    ];

    for (actual, expected) in cases {
        assert_eq!(actual.unwrap_err(), expected);
    }
}

#[test]
fn optional_owner_aliases_may_be_absent_but_never_foreign() {
    let credentials = credentials();
    let raw_order = format!(
        r#"{{"event_type":"order","id":"{ORDER_1}","market":"{CONDITION}","asset_id":"1234","side":"BUY","original_size":"10.000000","size_matched":"0","price":"0.520000","type":"PLACEMENT","owner":"{API_KEY}","timestamp":"1780449126930"}}"#
    );
    let raw_trade = format!(
        r#"{{"event_type":"trade","type":"TRADE","id":"28c4d2eb-bbea-40e7-a9f0-b2fdb56b2c2e","market":"{CONDITION}","asset_id":"1234","side":"BUY","size":"10.000000","price":"0.520000","status":"MATCHED","taker_order_id":"{ORDER_1}","maker_orders":[],"owner":"{API_KEY}","timestamp":"1780449126930"}}"#
    );
    for raw in [raw_order, raw_trade] {
        let frame = parse_live_user_frame(raw.as_bytes()).unwrap();
        credentials.bind_user_stream_frame(frame).unwrap();
    }
}

#[test]
fn wrapper_and_failures_are_redacted_and_retain_no_credential_copy() {
    let credentials = credentials();
    let bound: CredentialOwnedUserFrame = credentials
        .bind_user_stream_frame(trade_frame(API_KEY, API_KEY, FOREIGN_API_KEY))
        .unwrap();
    let failure = credentials
        .bind_user_stream_frame(order_frame(FOREIGN_API_KEY, API_KEY))
        .unwrap_err();

    for rendered in [
        format!("{credentials:?}"),
        format!("{bound:?}"),
        format!("{failure:?}"),
        failure.to_string(),
    ] {
        assert!(!rendered.contains(API_KEY));
        assert!(!rendered.contains(FOREIGN_API_KEY));
        assert!(!rendered.contains(API_SECRET));
        assert!(!rendered.contains(PASSPHRASE));
    }
    assert_eq!(format!("{bound:?}"), "CredentialOwnedUserFrame([REDACTED])");
}

#[test]
fn identity_boundary_is_pinned_and_has_no_secret_or_raw_frame_escape() {
    let source = include_str!("../src/user_frame.rs");
    for authority in [
        "https://docs.polymarket.com/market-data/websocket/user-channel",
        "f3e1a05f868a1fd0c34ef85dfc45c6ce78f5bb69",
        "8222273a9c72033b760e1d2fec813bc77144556d",
    ] {
        assert!(
            source.contains(authority),
            "missing source pin: {authority}"
        );
    }
    for forbidden in [
        "self.api_key()",
        "self.hmac_key()",
        "self.passphrase()",
        "maker_orders()",
        "pub fn into_frame",
        "impl From<PmLiveUserFrame>",
    ] {
        assert!(
            !source.contains(forbidden),
            "identity wrapper contains forbidden escape or owner check: {forbidden}"
        );
    }
}
