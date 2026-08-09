mod support;

use reap_pm_core::{
    ConnectionEpoch, IngressSequence, PmFillFee, PmFillQueryCursor, PmOrderStatus, PmSignedUnits,
    PmVenueOrderId, PmVenueOrderKey,
};
use reap_polymarket_adapter::{
    PmLiveNormalizationError, PmReadOwnerGrant, PmReconciliationContractError,
};
use reap_polymarket_wire::{
    PmLiveOpenOrderPage, PmLiveTradePage, parse_live_open_order_page, parse_live_order_detail,
    parse_live_trade_page,
};
use serde_json::{Value, json};

use support::{CONDITION, FUNDER, account_scope, completion, reconciliation_with, snapshot};

const OWNER: &str = "9180014b-33c8-9240-a14b-bdca11c0a465";
const ORDER_A: &str = "0x1111111111111111111111111111111111111111111111111111111111111111";
const ORDER_B: &str = "0x2222222222222222222222222222222222222222222222222222222222222222";
const ORDER_C: &str = "0x3333333333333333333333333333333333333333333333333333333333333333";
const FOREIGN_CONDITION: &str =
    "0xcccccccccccccccccccccccccccccccccccccccccccccccccccccccccccccccc";
const FOREIGN_MAKER: &str = "0xcdcdcdcdcdcdcdcdcdcdcdcdcdcdcdcdcdcdcdcd";

fn order(id: &str, condition: &str, token: &str, maker: &str, price: &str) -> Value {
    json!({
        "id": id,
        "market": condition,
        "asset_id": token,
        "side": "BUY",
        "original_size": "10",
        "size_matched": "0",
        "price": price,
        "status": "LIVE",
        "maker_address": maker,
        "owner": OWNER,
        "expiration": "0",
        "created_at": 1700000000,
        "outcome": "Yes",
        "order_type": "GTC"
    })
}

fn open_page(rows: &[Value], next: &str) -> PmLiveOpenOrderPage {
    parse_live_open_order_page(
        &serde_json::to_vec(&json!({
            "data": rows,
            "next_cursor": next,
            "limit": 128,
            "count": rows.len()
        }))
        .unwrap(),
    )
    .unwrap()
}

fn taker_trade(
    fill: &str,
    condition: &str,
    token: &str,
    order_id: Option<&str>,
    taker_order_id: Option<&str>,
) -> Value {
    json!({
        "id": fill,
        "market": condition,
        "asset_id": token,
        "side": "BUY",
        "size": "5",
        "price": "0.42",
        "status": "MATCHED",
        "trader_side": "TAKER",
        "order_id": order_id,
        "taker_order_id": taker_order_id,
        "maker_orders": [],
        "maker_address": FOREIGN_MAKER,
        "owner": OWNER,
        "match_time": "1700000000001",
        "last_update": "1700000000002"
    })
}

fn maker_trade() -> Value {
    json!({
        "id": "fill-maker",
        "market": CONDITION,
        "asset_id": "999",
        "side": "SELL",
        "size": "5",
        "price": "0.58",
        "status": "CONFIRMED",
        "trader_side": "MAKER",
        "maker_orders": [
            {
                "order_id": ORDER_A,
                "asset_id": "123",
                "side": "BUY",
                "price": "0.42",
                "matched_amount": "5",
                "owner": OWNER,
                "maker_address": FUNDER
            },
            {
                "order_id": ORDER_B,
                "asset_id": "123",
                "side": "BUY",
                "price": "0.42",
                "matched_amount": "5",
                "owner": OWNER,
                "maker_address": FOREIGN_MAKER
            }
        ],
        "maker_address": FUNDER,
        "owner": OWNER,
        "match_time": "1700000000001",
        "last_update": "1700000000002"
    })
}

fn trade_page(rows: &[Value], next: &str) -> PmLiveTradePage {
    parse_live_trade_page(
        &serde_json::to_vec(&json!({
            "data": rows,
            "next_cursor": next,
            "limit": 128,
            "count": rows.len()
        }))
        .unwrap(),
    )
    .unwrap()
}

#[test]
fn rest_trade_zero_fee_survives_leg_and_event_normalization() {
    let (_, reconciliation, _) = PmReadOwnerGrant::allocate().split();
    let mut role = reconciliation_with(reconciliation);
    let mut trade = taker_trade("fill-zero-fee", CONDITION, "123", Some(ORDER_A), None);
    trade["fee_rate_bps"] = json!("0");
    let pages = [trade_page(&[trade], "LTE=")];
    let serviced = role
        .request_fills(ConnectionEpoch::new(1), IngressSequence::new(10), None)
        .unwrap()
        .complete_live_trade_pages(completion(1, 11, Some(1)), snapshot(1), &pages)
        .unwrap()
        .into_delivery()
        .service_at(30_000)
        .unwrap();
    role.reduce_fill_query_delivery(serviced, |_, envelope| {
        let [fill] = envelope.payload().fills() else {
            panic!("one normalized REST fill")
        };
        assert_eq!(
            fill.execution().fee(),
            PmFillFee::Known {
                asset: support::instrument_scope().trading_domain().collateral(),
                delta: PmSignedUnits::ZERO,
            }
        );
    })
    .unwrap();
}

#[test]
fn unknown_incomplete_and_known_fee_facts_are_pairwise_distinct() {
    for (first_rate, second_rate) in [
        (None, Some("25")),
        (None, Some("0")),
        (Some("25"), Some("0")),
    ] {
        let (_, reconciliation, _) = PmReadOwnerGrant::allocate().split();
        let mut role = reconciliation_with(reconciliation);
        let mut first = taker_trade("fill-fee-conflict", CONDITION, "123", Some(ORDER_A), None);
        if let Some(rate) = first_rate {
            first["fee_rate_bps"] = json!(rate);
        }
        let mut second = taker_trade("fill-fee-conflict", CONDITION, "123", Some(ORDER_A), None);
        if let Some(rate) = second_rate {
            second["fee_rate_bps"] = json!(rate);
        }
        let pages = [trade_page(&[first, second], "LTE=")];
        assert!(matches!(
            role.request_fills(ConnectionEpoch::new(1), IngressSequence::new(10), None)
                .unwrap()
                .complete_live_trade_pages(completion(1, 11, Some(1)), snapshot(1), &pages),
            Err(PmReconciliationContractError::Live(
                PmLiveNormalizationError::ConflictingTradeLeg
            ))
        ));
    }
}

#[test]
fn open_order_pages_converge_overlap_retain_foreign_and_reject_conflicts() {
    let (_, reconciliation, _) = PmReadOwnerGrant::allocate().split();
    let mut role = reconciliation_with(reconciliation);
    let configured = order(ORDER_A, CONDITION, "123", FUNDER, "0.42");
    let foreign = order(ORDER_B, FOREIGN_CONDITION, "456", FOREIGN_MAKER, "0.43");
    let pages = [
        open_page(&[configured.clone(), foreign.clone()], "cursor-2"),
        open_page(&[foreign.clone(), configured.clone()], "LTE="),
    ];
    let normalized = role
        .request_open_orders(ConnectionEpoch::new(1), IngressSequence::new(10))
        .unwrap()
        .complete_live_pages(completion(1, 11, Some(1)), snapshot(1), &pages)
        .unwrap();
    assert_eq!(normalized.foreign_diagnostics().count(), 1);
    let first_digest = normalized.foreign_diagnostics().digest();
    let serviced = normalized.into_delivery().service_at(30_000).unwrap();
    role.reduce_open_orders_delivery(serviced, |_, envelope| {
        assert_eq!(envelope.payload().orders().len(), 1);
    })
    .unwrap();

    let reordered = [
        open_page(&[foreign.clone(), configured.clone()], "cursor-3"),
        open_page(&[configured.clone(), foreign.clone()], "LTE="),
    ];
    let normalized = role
        .request_open_orders(ConnectionEpoch::new(1), IngressSequence::new(20))
        .unwrap()
        .complete_live_pages(completion(1, 21, Some(2)), snapshot(2), &reordered)
        .unwrap();
    assert_eq!(normalized.foreign_diagnostics().digest(), first_digest);

    let conflict = [open_page(
        &[configured, order(ORDER_A, CONDITION, "123", FUNDER, "0.44")],
        "LTE=",
    )];
    assert!(matches!(
        role.request_open_orders(ConnectionEpoch::new(1), IngressSequence::new(30))
            .unwrap()
            .complete_live_pages(completion(1, 31, Some(3)), snapshot(3), &conflict),
        Err(PmReconciliationContractError::Live(
            PmLiveNormalizationError::ConflictingOrder
        ))
    ));
}

#[test]
fn configured_order_contradictions_fail_the_whole_cut() {
    let (_, reconciliation, _) = PmReadOwnerGrant::allocate().split();
    let mut role = reconciliation_with(reconciliation);
    let wrong_maker = [open_page(
        &[order(ORDER_A, CONDITION, "123", FOREIGN_MAKER, "0.42")],
        "LTE=",
    )];
    assert!(matches!(
        role.request_open_orders(ConnectionEpoch::new(1), IngressSequence::new(10))
            .unwrap()
            .complete_live_pages(completion(1, 11, Some(1)), snapshot(1), &wrong_maker),
        Err(PmReconciliationContractError::Live(
            PmLiveNormalizationError::AccountProfileMismatch
        ))
    ));
}

#[test]
fn proxy_open_order_uses_funder_as_maker_and_rejects_a_foreign_maker() {
    let (_, reconciliation, _) = PmReadOwnerGrant::allocate().split();
    let mut role =
        support::reconciliation_with_account(reconciliation, support::proxy_account_scope());
    let configured = [open_page(
        &[order(ORDER_A, CONDITION, "123", FUNDER, "0.42")],
        "LTE=",
    )];
    let normalized = role
        .request_open_orders(ConnectionEpoch::new(1), IngressSequence::new(10))
        .unwrap()
        .complete_live_pages(completion(1, 11, Some(1)), snapshot(1), &configured)
        .unwrap();
    assert!(normalized.foreign_diagnostics().is_empty());
    let serviced = normalized.into_delivery().service_at(30_000).unwrap();
    role.reduce_open_orders_delivery(serviced, |_, envelope| {
        assert_eq!(envelope.payload().orders().len(), 1);
    })
    .unwrap();

    let foreign = [open_page(
        &[order(ORDER_B, CONDITION, "123", FOREIGN_MAKER, "0.42")],
        "LTE=",
    )];
    assert!(matches!(
        role.request_open_orders(ConnectionEpoch::new(1), IngressSequence::new(20))
            .unwrap()
            .complete_live_pages(completion(1, 21, Some(2)), snapshot(2), &foreign),
        Err(PmReconciliationContractError::Live(
            PmLiveNormalizationError::AccountProfileMismatch
        ))
    ));
}

#[test]
fn empty_live_cuts_are_explicit_and_exact_detail_uses_rest_status() {
    let (_, reconciliation, _) = PmReadOwnerGrant::allocate().split();
    let mut role = reconciliation_with(reconciliation);
    let empty_orders = [open_page(&[], "LTE=")];
    let empty = role
        .request_open_orders(ConnectionEpoch::new(1), IngressSequence::new(10))
        .unwrap()
        .complete_live_pages(completion(1, 11, Some(1)), snapshot(1), &empty_orders)
        .unwrap();
    assert!(empty.foreign_diagnostics().is_empty());
    let serviced = empty.into_delivery().service_at(30_000).unwrap();
    role.reduce_open_orders_delivery(serviced, |_, envelope| {
        assert!(envelope.payload().orders().is_empty());
    })
    .unwrap();

    let empty_trades = [trade_page(&[], "LTE=")];
    let empty = role
        .request_fills(ConnectionEpoch::new(1), IngressSequence::new(20), None)
        .unwrap()
        .complete_live_trade_pages(completion(1, 21, Some(2)), snapshot(2), &empty_trades)
        .unwrap();
    assert!(empty.foreign_diagnostics().is_empty());
    let serviced = empty.into_delivery().service_at(30_001).unwrap();
    role.reduce_fill_query_delivery(serviced, |_, envelope| {
        assert!(envelope.payload().fills().is_empty());
    })
    .unwrap();

    let requested = PmVenueOrderKey::new(
        account_scope().handle(),
        PmVenueOrderId::new(ORDER_A).unwrap(),
    );
    let row = order(ORDER_A, CONDITION, "123", FUNDER, "0.42");
    let row = parse_live_order_detail(&serde_json::to_vec(&row).unwrap()).unwrap();
    let serviced = role
        .request_order_detail(ConnectionEpoch::new(1), IngressSequence::new(30), requested)
        .unwrap()
        .complete_live(completion(1, 31, Some(3)), snapshot(3), Some(&row))
        .unwrap()
        .service_at(30_002)
        .unwrap();
    role.reduce_order_detail_delivery(serviced, |_, envelope| {
        assert_eq!(
            envelope.payload().order().unwrap().progress().status(),
            PmOrderStatus::Open
        );
    })
    .unwrap();
}

#[test]
fn full_account_trades_project_only_configured_local_legs_and_fail_ambiguous_linkage() {
    let (_, reconciliation, _) = PmReadOwnerGrant::allocate().split();
    let mut role = reconciliation_with(reconciliation);
    let foreign_local = taker_trade(
        "fill-foreign",
        FOREIGN_CONDITION,
        "456",
        Some(ORDER_C),
        None,
    );
    let pages = [trade_page(&[maker_trade(), foreign_local], "LTE=")];
    let normalized = role
        .request_fills(ConnectionEpoch::new(1), IngressSequence::new(10), None)
        .unwrap()
        .complete_live_trade_pages(completion(1, 11, Some(1)), snapshot(1), &pages)
        .unwrap();
    assert_eq!(normalized.foreign_diagnostics().count(), 2);
    let serviced = normalized.into_delivery().service_at(30_000).unwrap();
    role.reduce_fill_query_delivery(serviced, |_, envelope| {
        assert_eq!(envelope.payload().fills().len(), 1);
        assert_eq!(
            envelope.payload().fills()[0]
                .fill_key()
                .venue_order()
                .id()
                .as_str(),
            ORDER_A
        );
    })
    .unwrap();

    let ambiguous = [trade_page(
        &[taker_trade(
            "fill-ambiguous",
            CONDITION,
            "123",
            Some(ORDER_A),
            Some(ORDER_B),
        )],
        "LTE=",
    )];
    assert!(matches!(
        role.request_fills(ConnectionEpoch::new(1), IngressSequence::new(20), None)
            .unwrap()
            .complete_live_trade_pages(completion(1, 21, Some(2)), snapshot(2), &ambiguous),
        Err(PmReconciliationContractError::Live(
            PmLiveNormalizationError::UnresolvedCompleteTrade
        ))
    ));
}

#[test]
fn trade_page_overlap_converges_and_conflicting_exact_leg_fails_closed() {
    let (_, reconciliation, _) = PmReadOwnerGrant::allocate().split();
    let mut role = reconciliation_with(reconciliation);
    let trade = taker_trade("fill-overlap", CONDITION, "123", Some(ORDER_A), None);
    let pages = [
        trade_page(std::slice::from_ref(&trade), "cursor-2"),
        trade_page(std::slice::from_ref(&trade), "LTE="),
    ];
    let serviced = role
        .request_fills(ConnectionEpoch::new(1), IngressSequence::new(10), None)
        .unwrap()
        .complete_live_trade_pages(completion(1, 11, Some(1)), snapshot(1), &pages)
        .unwrap()
        .into_delivery()
        .service_at(30_000)
        .unwrap();
    role.reduce_fill_query_delivery(serviced, |_, envelope| {
        assert_eq!(envelope.payload().fills().len(), 1);
    })
    .unwrap();

    let mut conflict = trade.clone();
    conflict["price"] = json!("0.44");
    let pages = [trade_page(&[trade, conflict], "LTE=")];
    assert!(matches!(
        role.request_fills(ConnectionEpoch::new(1), IngressSequence::new(20), None)
            .unwrap()
            .complete_live_trade_pages(completion(1, 21, Some(2)), snapshot(2), &pages),
        Err(PmReconciliationContractError::Live(
            PmLiveNormalizationError::ConflictingTradeLeg
        ))
    ));
}

fn complete_cursor(
    role: &mut reap_polymarket_adapter::PmReconciliation,
    request: u64,
    revision: u64,
    prior: Option<PmFillQueryCursor>,
    rows: &[Value],
) -> PmFillQueryCursor {
    let pages = [trade_page(rows, "LTE=")];
    let completion = role
        .request_fills(
            ConnectionEpoch::new(1),
            IngressSequence::new(request),
            prior,
        )
        .unwrap()
        .complete_live_trade_pages(
            support::completion(1, request + 1, Some(revision)),
            snapshot(revision),
            &pages,
        )
        .unwrap()
        .into_delivery()
        .service_at(40_000 + request)
        .unwrap();
    role.reduce_fill_query_delivery(completion, |_, envelope| {
        envelope.payload().resulting_watermark()
    })
    .unwrap()
}

#[test]
fn unchanged_and_a_b_a_full_account_cuts_advance_causally() {
    let (_, reconciliation, _) = PmReadOwnerGrant::allocate().split();
    let mut role = reconciliation_with(reconciliation);
    let a = taker_trade("fill-a", CONDITION, "123", Some(ORDER_A), None);
    let b = taker_trade("fill-b", CONDITION, "123", Some(ORDER_B), None);

    let cursor_a1 = complete_cursor(&mut role, 10, 1, None, std::slice::from_ref(&a));
    let cursor_a2 = complete_cursor(&mut role, 20, 2, Some(cursor_a1), std::slice::from_ref(&a));
    let cursor_b = complete_cursor(&mut role, 30, 3, Some(cursor_a2), std::slice::from_ref(&b));
    let cursor_a3 = complete_cursor(&mut role, 40, 4, Some(cursor_b), &[a]);

    assert_ne!(cursor_a1, cursor_a2);
    assert_ne!(cursor_a2, cursor_b);
    assert_ne!(cursor_b, cursor_a3);
    assert_ne!(cursor_a1, cursor_a3);
}
