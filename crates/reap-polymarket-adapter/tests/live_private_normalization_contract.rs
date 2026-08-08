mod support;

use reap_pm_core::{ConnectionEpoch, PmFillFee, PmFillRole, PmOrderStatus, PmSignedUnits};
use reap_polymarket_adapter::{
    PmPrivateLifecycleObservation, PmPrivateNormalizationError, PmReadOwnerGrant,
};
use reap_polymarket_auth::{CredentialOwnedUserFrame, L2CredentialInput, L2Credentials};
use reap_polymarket_wire::parse_live_user_frame;
use serde_json::{Value, json};

use support::{CONDITION, FUNDER, completion, private_with};

const OWNER: &str = "9180014b-33c8-9240-a14b-bdca11c0a465";
const ORDER_A: &str = "0x1111111111111111111111111111111111111111111111111111111111111111";
const ORDER_B: &str = "0x2222222222222222222222222222222222222222222222222222222222222222";
const FOREIGN_CONDITION: &str =
    "0xcccccccccccccccccccccccccccccccccccccccccccccccccccccccccccccccc";
const FOREIGN_MAKER: &str = "0xcdcdcdcdcdcdcdcdcdcdcdcdcdcdcdcdcdcdcdcd";
const AUTH_ADDRESS: &str = "0xf39Fd6e51aad88F6F4ce6aB8827279cffFb92266";
const API_SECRET: &str = "AAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAA=";
const PASSPHRASE: &str = "synthetic-passphrase";

fn authenticated(value: Value) -> CredentialOwnedUserFrame {
    let raw = serde_json::to_vec(&value).unwrap();
    let frame = parse_live_user_frame(&raw).unwrap();
    L2Credentials::bind(
        AUTH_ADDRESS,
        L2CredentialInput::new(OWNER.into(), API_SECRET.into(), PASSPHRASE.into()),
    )
    .unwrap()
    .bind_user_stream_frame(frame)
    .unwrap()
}

fn user_order(id: &str, condition: &str, matched: &str, kind: &str) -> Value {
    json!({
        "event_type": "order",
        "id": id,
        "market": condition,
        "asset_id": "123",
        "side": "BUY",
        "original_size": "10",
        "size_matched": matched,
        "price": "0.42",
        "type": kind,
        "maker_address": FUNDER,
        "expiration": "0",
        "order_type": "GTC",
        "outcome": "Yes",
        "status": match (kind, matched) {
            ("PLACEMENT", "0") => "LIVE",
            ("UPDATE", "10") => "MATCHED",
            ("UPDATE", _) => "LIVE",
            ("CANCELLATION", _) => "CANCELED",
            _ => "LIVE",
        },
        "created_at": "1700000000",
        "associate_trades": null,
        "owner": OWNER,
        "order_owner": OWNER,
        "timestamp": "1700000000000"
    })
}

fn maker_trade(local: bool, include_foreign: bool) -> Value {
    let mut makers = Vec::new();
    if local {
        makers.push(json!({
            "order_id": ORDER_A,
            "asset_id": "123",
            "side": "BUY",
            "price": "0.42",
            "matched_amount": "5",
            "owner": OWNER,
            "maker_address": FUNDER
        }));
    }
    if include_foreign {
        makers.push(json!({
            "order_id": ORDER_B,
            "asset_id": "123",
            "side": "BUY",
            "price": "0.42",
            "matched_amount": "5",
            "owner": OWNER,
            "maker_address": FOREIGN_MAKER
        }));
    }
    json!({
        "event_type": "trade",
        "id": "fill-maker",
        "market": CONDITION,
        "asset_id": "999",
        "side": "SELL",
        "size": "5",
        "price": "0.58",
        "status": "MATCHED",
        "trader_side": "MAKER",
        "maker_orders": makers,
        "owner": OWNER,
        "trade_owner": OWNER,
        "timestamp": "1700000000001",
        "last_update": "1700000000002"
    })
}

fn taker_trade(order_id: Option<&str>, taker_id: Option<&str>) -> Value {
    json!({
        "event_type": "trade",
        "id": "fill-taker",
        "market": CONDITION,
        "asset_id": "123",
        "side": "BUY",
        "size": "5",
        "price": "0.42",
        "status": "MATCHED",
        "trader_side": "TAKER",
        "order_id": order_id,
        "taker_order_id": taker_id,
        "maker_orders": [],
        "owner": OWNER,
        "trade_owner": OWNER,
        "timestamp": "1700000000001",
        "last_update": "1700000000002"
    })
}

fn observe(
    role: &mut reap_polymarket_adapter::PmPrivateLifecycle,
    sequence: u64,
    value: Value,
) -> (
    Vec<PmPrivateLifecycleObservation>,
    reap_polymarket_adapter::PmForeignRowDiagnostics,
) {
    let frame = authenticated(value);
    let delivery = role
        .receive_live_user_frame(completion(1, sequence, None), frame)
        .unwrap()
        .into_delivery()
        .service_at(30_000 + sequence)
        .unwrap();
    role.reduce_private_delivery(delivery, |_, envelope| {
        (
            envelope.payload().observations().to_vec(),
            envelope.payload().foreign_diagnostics(),
        )
    })
    .unwrap()
}

fn observed_taker_fee(fee_rate_bps: Option<&str>) -> PmFillFee {
    let (private, _, _) = PmReadOwnerGrant::allocate().split();
    let mut role = private_with(private);
    role.reconnect(ConnectionEpoch::new(1)).unwrap();
    let mut trade = taker_trade(Some(ORDER_A), None);
    if let Some(fee_rate_bps) = fee_rate_bps {
        trade["fee_rate_bps"] = json!(fee_rate_bps);
    }
    let (observations, diagnostics) = observe(&mut role, 1, trade);
    assert!(diagnostics.is_empty());
    let [PmPrivateLifecycleObservation::Fill(fill)] = observations.as_slice() else {
        panic!("one fill")
    };
    fill.execution().fee()
}

fn observed_maker_fee(top_level_rate: &str, maker_rate: &str) -> PmFillFee {
    let (private, _, _) = PmReadOwnerGrant::allocate().split();
    let mut role = private_with(private);
    role.reconnect(ConnectionEpoch::new(1)).unwrap();
    let mut trade = maker_trade(true, false);
    trade["fee_rate_bps"] = json!(top_level_rate);
    trade["maker_orders"][0]["fee_rate_bps"] = json!(maker_rate);
    let (observations, diagnostics) = observe(&mut role, 1, trade);
    assert!(diagnostics.is_empty());
    let [PmPrivateLifecycleObservation::Fill(fill)] = observations.as_slice() else {
        panic!("one maker fill")
    };
    fill.execution().fee()
}

#[test]
fn live_zero_fee_rate_proves_exact_goal_f_collateral_zero_only() {
    let known_zero = PmFillFee::Known {
        asset: support::instrument_scope().trading_domain().collateral(),
        delta: PmSignedUnits::ZERO,
    };
    assert_eq!(observed_taker_fee(Some("0")), known_zero);
    assert_eq!(observed_taker_fee(None), PmFillFee::Unknown);
    assert_eq!(observed_taker_fee(Some("25")), PmFillFee::Incomplete);
}

#[test]
fn maker_fee_authority_comes_from_the_exact_nested_leg() {
    let known_zero = PmFillFee::Known {
        asset: support::instrument_scope().trading_domain().collateral(),
        delta: PmSignedUnits::ZERO,
    };
    assert_eq!(observed_maker_fee("0", "25"), PmFillFee::Incomplete);
    assert_eq!(observed_maker_fee("25", "0"), known_zero);
}

#[test]
fn user_order_status_is_proven_by_kind_cumulative_and_wire_status() {
    let (private, _, _) = PmReadOwnerGrant::allocate().split();
    let mut role = private_with(private);
    role.reconnect(ConnectionEpoch::new(1)).unwrap();

    let cases = [
        (1, "0", "PLACEMENT", PmOrderStatus::Open),
        (2, "5", "UPDATE", PmOrderStatus::PartiallyFilled),
        (3, "10", "UPDATE", PmOrderStatus::Filled),
        (4, "5", "CANCELLATION", PmOrderStatus::Cancelled),
    ];
    for (sequence, matched, kind, expected) in cases {
        let (observations, diagnostics) = observe(
            &mut role,
            sequence,
            user_order(ORDER_A, CONDITION, matched, kind),
        );
        let [PmPrivateLifecycleObservation::Order(order)] = observations.as_slice() else {
            panic!("one order")
        };
        assert_eq!(order.progress().status(), expected);
        assert!(diagnostics.is_empty());
    }

    let invalid = authenticated(user_order(ORDER_A, CONDITION, "1", "PLACEMENT"));
    assert!(matches!(
        role.receive_live_user_frame(completion(1, 5, None), invalid),
        Err(PmPrivateNormalizationError::Live(
            reap_polymarket_adapter::PmLiveNormalizationError::UserOrderKindProgressMismatch
        ))
    ));
}

#[test]
fn configured_user_order_requires_exact_eoa_gtc_outcome_and_status_consistency() {
    let cases = [
        (
            "maker_address",
            Value::String(FOREIGN_MAKER.into()),
            reap_polymarket_adapter::PmLiveNormalizationError::AccountProfileMismatch,
        ),
        (
            "order_type",
            Value::String("GTD".into()),
            reap_polymarket_adapter::PmLiveNormalizationError::UnsupportedOrderType,
        ),
        (
            "expiration",
            Value::String("1700000010".into()),
            reap_polymarket_adapter::PmLiveNormalizationError::UnexpectedExpiration,
        ),
        (
            "outcome",
            Value::String("No".into()),
            reap_polymarket_adapter::PmLiveNormalizationError::OutcomeMismatch,
        ),
        (
            "status",
            Value::String("MATCHED".into()),
            reap_polymarket_adapter::PmLiveNormalizationError::UserOrderStatusProgressMismatch,
        ),
    ];

    for (sequence, (field, value, expected)) in cases.into_iter().enumerate() {
        let (private, _, _) = PmReadOwnerGrant::allocate().split();
        let mut role = private_with(private);
        role.reconnect(ConnectionEpoch::new(1)).unwrap();
        let mut order = user_order(ORDER_A, CONDITION, "0", "PLACEMENT");
        order[field] = value;
        let error = match role.receive_live_user_frame(
            completion(1, u64::try_from(sequence + 1).unwrap(), None),
            authenticated(order),
        ) {
            Err(error) => error,
            Ok(_) => panic!("contradictory configured order must fail"),
        };
        assert_eq!(error, PmPrivateNormalizationError::Live(expected));
    }

    for (sequence, field) in [
        "maker_address",
        "order_type",
        "expiration",
        "outcome",
        "status",
    ]
    .into_iter()
    .enumerate()
    {
        let (private, _, _) = PmReadOwnerGrant::allocate().split();
        let mut role = private_with(private);
        role.reconnect(ConnectionEpoch::new(1)).unwrap();
        let mut order = user_order(ORDER_A, CONDITION, "0", "PLACEMENT");
        order.as_object_mut().unwrap().remove(field);
        let error = match role.receive_live_user_frame(
            completion(1, u64::try_from(sequence + 20).unwrap(), None),
            authenticated(order),
        ) {
            Err(error) => error,
            Ok(_) => panic!("configured normalization must require {field}"),
        };
        assert_eq!(
            error,
            PmPrivateNormalizationError::Live(
                reap_polymarket_adapter::PmLiveNormalizationError::MissingUserOrderProfileFact(
                    field
                )
            )
        );
    }
}

#[test]
fn matched_not_broadcasted_remains_a_distinct_settlement_fact() {
    let (private, _, _) = PmReadOwnerGrant::allocate().split();
    let mut role = private_with(private);
    role.reconnect(ConnectionEpoch::new(1)).unwrap();
    let mut trade = taker_trade(Some(ORDER_A), None);
    trade["status"] = json!("MATCHED_NOT_BROADCASTED");
    let (observations, diagnostics) = observe(&mut role, 1, trade);
    let [PmPrivateLifecycleObservation::Fill(fill)] = observations.as_slice() else {
        panic!("one fill")
    };
    assert_eq!(
        fill.execution().settlement(),
        reap_pm_core::PmFillSettlementStatus::MatchedNotBroadcasted
    );
    assert!(diagnostics.is_empty());
}

#[test]
fn foreign_orders_and_nested_makers_are_retained_while_only_local_rows_reduce() {
    let (private, _, _) = PmReadOwnerGrant::allocate().split();
    let mut role = private_with(private);
    role.reconnect(ConnectionEpoch::new(1)).unwrap();

    let frame = json!([
        user_order(ORDER_B, FOREIGN_CONDITION, "0", "PLACEMENT"),
        maker_trade(true, true)
    ]);
    let (observations, diagnostics) = observe(&mut role, 1, frame);
    assert_eq!(observations.len(), 1);
    let PmPrivateLifecycleObservation::Fill(fill) = observations[0] else {
        panic!("local maker fill")
    };
    assert_eq!(fill.execution().role(), PmFillRole::Maker);
    assert_eq!(fill.fill_key().venue_order().id().as_str(), ORDER_A);
    assert_eq!(diagnostics.count(), 2);
    assert_ne!(diagnostics.digest(), [0; 32]);
}

#[test]
fn ambiguous_linkage_is_unresolved_on_stream_and_delivery_remains_owner_bound() {
    let (private_a, _, _) = PmReadOwnerGrant::allocate().split();
    let (private_b, _, _) = PmReadOwnerGrant::allocate().split();
    let mut owner_a = private_with(private_a);
    let owner_b = private_with(private_b);
    owner_a.reconnect(ConnectionEpoch::new(1)).unwrap();

    let frame = authenticated(taker_trade(Some(ORDER_A), Some(ORDER_B)));
    let serviced = owner_a
        .receive_live_user_frame(completion(1, 1, None), frame)
        .unwrap()
        .into_delivery()
        .service_at(30_001)
        .unwrap();
    let serviced = *owner_b
        .reduce_private_delivery(serviced, |_, _| ())
        .expect_err("another role cannot open delivery");
    owner_a
        .reduce_private_delivery(serviced, |_, envelope| {
            let [PmPrivateLifecycleObservation::UnresolvedTrade(unresolved)] =
                envelope.payload().observations()
            else {
                panic!("one unresolved trade")
            };
            assert_eq!(
                unresolved.reason(),
                reap_polymarket_adapter::PmUnresolvedTradeReason::MultipleOrderReferenceKinds
            );
        })
        .unwrap();
}
