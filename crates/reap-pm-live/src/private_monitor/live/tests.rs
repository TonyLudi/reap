use reap_pm_core::{
    ConnectionEpoch, EvmAddress, PmAllowanceValue, PmOrderStatus, PmSign, PmSignedUnits, PmTokenId,
    PmVenueOrderId, PmVenueOrderKey, U256,
};
use reap_pm_state::{
    PmAllowanceKnowledge, PmOpenOrdersApply, PmPositionKnowledge, PmRefreshReason,
};
use reap_pm_state::{
    PmPrivateDependency, PmPrivateExternalIngressFailure, PmPrivateExternalIngressFault,
    PmPrivateExternalIngressLane, PmPrivateHaltReason,
};
use reap_polymarket_adapter::{PmLiveNormalizationError, PmReconciliationContractError};
use reap_polymarket_auth::{CredentialOwnedUserFrame, L2CredentialInput, L2Credentials};
use reap_polymarket_live_adapter::{
    PmAccountAsset, PmAccountBalanceAllowance, PmCompleteOpenOrdersCut, PmCompleteTradesCut,
};
use reap_polymarket_wire::{
    PmLiveOpenOrderPage, PmLiveTradePage, parse_live_balance_allowance, parse_live_open_order_page,
    parse_live_trade_page, parse_live_user_frame,
};
use serde_json::{Value, json};

use super::*;
use crate::evidence::{completion, connectivity_config, query_occurrence, risk_limits};

const CONDITION: &str = "0xaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa";
const PM_FUNDER: &str = "0xabababababababababababababababababababab";
const TOKEN: u64 = 123;
const OWNER: &str = "9180014b-33c8-9240-a14b-bdca11c0a465";
const AUTH_ADDRESS: &str = "0xf39Fd6e51aad88F6F4ce6aB8827279cffFb92266";
const API_SECRET: &str = "AAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAA=";
const PASSPHRASE: &str = "synthetic-passphrase";
const ORDER_A: &str = "0x1111111111111111111111111111111111111111111111111111111111111111";
const ORDER_B: &str = "0x2222222222222222222222222222222222222222222222222222222222222222";
const FOREIGN_CONDITION: &str =
    "0xcccccccccccccccccccccccccccccccccccccccccccccccccccccccccccccccc";
const FOREIGN_MAKER: &str = "0xcdcdcdcdcdcdcdcdcdcdcdcdcdcdcdcdcdcdcdcd";
const EXTRA_SPENDER: &str = "0xdededededededededededededededededededede";

fn connected_monitor() -> PmReadOnlyMonitor {
    let config = connectivity_config();
    let mut monitor =
        PmReadOnlyMonitor::new(config.account().clone(), risk_limits()).expect("fixed monitor");
    monitor
        .reconnect_private(ConnectionEpoch::new(1), 20_000)
        .expect("fixed private reconnect");
    monitor
}

#[test]
fn mismatched_http_dependency_lane_is_rejected_without_state_mutation() {
    let mut monitor = connected_monitor();
    let source = monitor.runtime.account.source();
    let connection = monitor.runtime.account.connection();
    let before_counters = monitor.private_projection().external_ingress_counters();
    let before_convergence = monitor.private_projection().convergence();
    let result = monitor.runtime.reduce_serviced_http_dependency_failure(
        source,
        connection,
        PmPrivateDependency::AccountSnapshot,
        PmPrivateExternalIngressFault::new(
            PmPrivateExternalIngressLane::OpenOrders,
            PmPrivateExternalIngressFailure::Service,
        ),
    );
    assert!(matches!(
        result,
        Err(PmPrivateMonitorError::HttpDependencyLaneMismatch)
    ));
    assert_eq!(
        monitor.private_projection().external_ingress_counters(),
        before_counters
    );
    assert_eq!(
        monitor.private_projection().convergence(),
        before_convergence
    );
}

#[test]
fn scope_corruption_uses_the_terminal_external_fault_path() {
    assert_terminal_http_dependency_failure(PmPrivateExternalIngressFailure::Scope);
}

#[test]
fn contract_corruption_uses_the_terminal_external_fault_path() {
    assert_terminal_http_dependency_failure(PmPrivateExternalIngressFailure::Contract);
}

fn assert_terminal_http_dependency_failure(failure: PmPrivateExternalIngressFailure) {
    let mut monitor = connected_monitor();
    let source = monitor.runtime.account.source();
    let connection = monitor.runtime.account.connection();
    let fault =
        PmPrivateExternalIngressFault::new(PmPrivateExternalIngressLane::AccountSnapshot, failure);

    monitor
        .runtime
        .reduce_serviced_http_dependency_failure(
            source,
            connection,
            PmPrivateDependency::AccountSnapshot,
            fault,
        )
        .unwrap();

    assert_eq!(
        monitor.private_projection().halt(),
        Some(PmPrivateHaltReason::ExternalIngressFault(fault))
    );
    assert_eq!(
        monitor
            .private_projection()
            .external_ingress_counters()
            .for_failure(failure),
        1
    );
}

fn authenticated(value: Value) -> CredentialOwnedUserFrame {
    let frame = parse_live_user_frame(&serde_json::to_vec(&value).expect("user JSON"))
        .expect("typed user frame");
    L2Credentials::bind(
        AUTH_ADDRESS,
        L2CredentialInput::new(OWNER.into(), API_SECRET.into(), PASSPHRASE.into()),
    )
    .expect("fixed credentials")
    .bind_user_stream_frame(frame)
    .expect("credential-owned user frame")
}

fn user_order(id: &str) -> Value {
    json!({
        "event_type": "order",
        "id": id,
        "market": CONDITION,
        "asset_id": TOKEN.to_string(),
        "side": "BUY",
        "original_size": "10",
        "size_matched": "0",
        "price": "0.42",
        "type": "PLACEMENT",
        "maker_address": PM_FUNDER,
        "expiration": "0",
        "order_type": "GTC",
        "outcome": "Yes",
        "status": "LIVE",
        "created_at": "1700000000",
        "associate_trades": null,
        "owner": OWNER,
        "order_owner": OWNER,
        "timestamp": "1700000000000"
    })
}

fn user_partial_fill(id: &str) -> Value {
    json!([
        {
            "event_type": "order",
            "id": id,
            "market": CONDITION,
            "asset_id": TOKEN.to_string(),
            "side": "BUY",
            "original_size": "10",
            "size_matched": "5",
            "price": "0.42",
            "type": "UPDATE",
            "maker_address": PM_FUNDER,
            "expiration": "0",
            "order_type": "GTC",
            "outcome": "Yes",
            "status": "LIVE",
            "created_at": "1700000000",
            "associate_trades": ["fill-maker"],
            "owner": OWNER,
            "order_owner": OWNER,
            "timestamp": "1700000000001"
        },
        {
            "event_type": "trade",
            "id": "fill-maker",
            "market": CONDITION,
            "asset_id": "999",
            "side": "SELL",
            "size": "5",
            "price": "0.58",
            "status": "MATCHED",
            "trader_side": "MAKER",
            "maker_orders": [{
                "order_id": id,
                "asset_id": TOKEN.to_string(),
                "side": "BUY",
                "price": "0.42",
                "matched_amount": "5",
                "owner": OWNER,
                "maker_address": PM_FUNDER
            }],
            "owner": OWNER,
            "trade_owner": OWNER,
            "timestamp": "1700000000001",
            "last_update": "1700000000002"
        }
    ])
}

fn duplicate_user_fill(id: &str) -> Value {
    user_partial_fill(id)
        .as_array()
        .and_then(|events| events.get(1))
        .cloned()
        .expect("partial-fill fixture contains a trade")
}

fn rest_order(id: &str, condition: &str, maker: &str, price: &str) -> Value {
    json!({
        "id": id,
        "market": condition,
        "asset_id": TOKEN.to_string(),
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

fn open_page(rows: &[Value], next_cursor: &str) -> PmLiveOpenOrderPage {
    parse_live_open_order_page(
        &serde_json::to_vec(&json!({
            "data": rows,
            "next_cursor": next_cursor,
            "limit": 128,
            "count": rows.len()
        }))
        .expect("open-order page JSON"),
    )
    .expect("typed open-order page")
}

fn open_cut(pages: Vec<PmLiveOpenOrderPage>) -> PmCompleteOpenOrdersCut {
    PmCompleteOpenOrdersCut::test_support_from_pages(pages.into_boxed_slice())
        .expect("terminal complete open-order cut")
}

fn trade_page(rows: &[Value]) -> PmLiveTradePage {
    parse_live_trade_page(
        &serde_json::to_vec(&json!({
            "data": rows,
            "next_cursor": "LTE=",
            "limit": 128,
            "count": rows.len()
        }))
        .expect("trade page JSON"),
    )
    .expect("typed trade page")
}

fn trade_cut(rows: &[Value]) -> PmCompleteTradesCut {
    PmCompleteTradesCut::test_support_from_pages(vec![trade_page(rows)].into_boxed_slice())
        .expect("terminal complete trade cut")
}

fn balance(
    asset: PmAccountAsset,
    amount: u64,
    exact_spender: EvmAddress,
    allowance: u64,
    retain_extra: bool,
) -> PmAccountBalanceAllowance {
    let mut allowances = serde_json::Map::new();
    allowances.insert(exact_spender.to_string(), json!(allowance.to_string()));
    if retain_extra {
        allowances.insert(EXTRA_SPENDER.to_owned(), json!("777"));
    }
    let value = parse_live_balance_allowance(
        &serde_json::to_vec(&json!({
            "balance": amount.to_string(),
            "allowances": allowances
        }))
        .expect("balance JSON"),
    )
    .expect("typed balance/allowance");
    PmAccountBalanceAllowance::test_support_new(asset, value)
}

fn account_pair(retain_extra: bool) -> (PmAccountBalanceAllowance, PmAccountBalanceAllowance) {
    let config = connectivity_config();
    let account = config.account();
    let spenders = account.required_spenders();
    (
        balance(
            PmAccountAsset::Collateral,
            1_000,
            spenders[0].requirement().spender(),
            100,
            retain_extra,
        ),
        balance(
            PmAccountAsset::Conditional(configured_token()),
            25,
            spenders[1].requirement().spender(),
            1,
            false,
        ),
    )
}

fn configured_token() -> PmTokenId {
    connectivity_config()
        .account()
        .expected_metadata()
        .outcome()
        .token()
}

#[test]
fn credential_owned_private_frame_reaches_the_existing_private_reducer() {
    let mut monitor = connected_monitor();
    let input = PmLivePrivateInput::new(
        completion(1, 1, None, 30_000),
        authenticated(user_order(ORDER_A)),
    );
    let result = monitor
        .ingest_private_live(input, 30_001)
        .expect("live private occurrence applies");

    assert_eq!(result.apply().order_observations(), 1);
    assert!(result.report().private_foreign().unwrap().is_empty());
    let orders = monitor.private_projection().orders().collect::<Vec<_>>();
    assert_eq!(orders.len(), 1);
    assert_eq!(
        orders[0]
            .identity()
            .venue_order_key()
            .unwrap()
            .id()
            .as_str(),
        ORDER_A
    );
}

#[test]
fn live_fill_advances_order_and_provisional_position_once_until_reconciliation() {
    let mut monitor = connected_monitor();
    let (collateral, conditional) = account_pair(false);
    monitor
        .ingest_account_live(
            PmLiveAccountInput::new(
                query_occurrence(1, 1, 2, 1, 29_000).expect("initial account occurrence"),
                collateral,
                conditional,
            )
            .expect("initial account input"),
        )
        .expect("initial account cut applies");
    monitor
        .ingest_private_live(
            PmLivePrivateInput::new(
                completion(1, 3, None, 30_000),
                authenticated(user_order(ORDER_A)),
            ),
            30_001,
        )
        .expect("live placement applies");

    let applied = monitor
        .ingest_private_live(
            PmLivePrivateInput::new(
                completion(1, 4, None, 31_000),
                authenticated(user_partial_fill(ORDER_A)),
            ),
            31_001,
        )
        .expect("live partial fill applies");
    assert_eq!(applied.apply().order_observations(), 1);
    assert_eq!(applied.apply().fill_observations(), 1);

    let provisional = {
        let projection = monitor.private_projection();
        let order = projection.orders().next().expect("tracked order");
        assert_eq!(order.status(), Some(PmOrderStatus::PartiallyFilled));
        let provisional = projection.provisional_deltas();
        assert_eq!(
            provisional.collateral(),
            PmSignedUnits::from_parts(PmSign::Negative, U256::from_u64(2_100_000)).unwrap()
        );
        assert_eq!(
            provisional.outcome(),
            PmSignedUnits::from_parts(PmSign::Positive, U256::from_u64(5_000_000)).unwrap()
        );
        assert_eq!(provisional.uncovered_fills(), 1);
        assert_eq!(
            projection.effective_position(),
            Some(PmSignedUnits::from_parts(PmSign::Positive, U256::from_u64(5_000_025)).unwrap())
        );
        assert!(
            projection
                .pending_refresh_keys()
                .any(|key| key.reason() == PmRefreshReason::FillObserved)
        );
        assert_eq!(projection.fill_counters().principal_applications(), 1);
        provisional
    };

    let duplicate = monitor
        .ingest_private_live(
            PmLivePrivateInput::new(
                completion(1, 5, None, 32_000),
                authenticated(duplicate_user_fill(ORDER_A)),
            ),
            32_001,
        )
        .expect("duplicate fill is classified without a second position move");
    assert_eq!(duplicate.apply().fill_observations(), 1);
    assert_eq!(duplicate.apply().duplicate_or_stale_observations(), 1);

    let projection = monitor.private_projection();
    assert_eq!(projection.provisional_deltas(), provisional);
    assert_eq!(projection.fill_counters().principal_applications(), 1);
    assert_eq!(projection.fill_counters().duplicates(), 1);
}

#[test]
fn complete_open_order_cuts_preserve_empty_overlap_foreign_and_conflict_semantics() {
    let mut monitor = connected_monitor();
    let empty = PmLiveOpenOrdersInput::new(
        query_occurrence(1, 10, 11, 1, 40_000).expect("empty occurrence"),
        open_cut(vec![open_page(&[], "LTE=")]),
    );
    let empty = monitor
        .ingest_open_orders_live(empty)
        .expect("empty complete cut applies");
    assert!(matches!(empty.apply(), PmOpenOrdersApply::Applied { .. }));
    assert!(empty.report().open_orders_foreign().unwrap().is_empty());

    let configured = rest_order(ORDER_A, CONDITION, PM_FUNDER, "0.42");
    let foreign = rest_order(ORDER_B, FOREIGN_CONDITION, FOREIGN_MAKER, "0.43");
    let overlap = PmLiveOpenOrdersInput::new(
        query_occurrence(1, 20, 21, 2, 41_000).expect("overlap occurrence"),
        open_cut(vec![
            open_page(&[configured.clone(), foreign.clone()], "cursor-2"),
            open_page(&[foreign.clone(), configured.clone()], "LTE="),
        ]),
    );
    let overlap = monitor
        .ingest_open_orders_live(overlap)
        .expect("overlapping pages converge");
    assert_eq!(overlap.report().open_orders_foreign().unwrap().count(), 1);
    assert_eq!(monitor.private_projection().orders().count(), 1);

    let conflict = PmLiveOpenOrdersInput::new(
        query_occurrence(1, 30, 31, 3, 42_000).expect("conflict occurrence"),
        open_cut(vec![open_page(
            &[
                configured,
                rest_order(ORDER_A, CONDITION, PM_FUNDER, "0.44"),
            ],
            "LTE=",
        )]),
    );
    assert!(matches!(
        monitor.ingest_open_orders_live(conflict),
        Err(PmPrivateMonitorError::Reconciliation(
            PmReconciliationContractError::Live(PmLiveNormalizationError::ConflictingOrder)
        ))
    ));
}

#[test]
fn account_input_requires_the_exact_asset_pair_and_configured_token() {
    let mut monitor = connected_monitor();
    let (collateral, conditional) = account_pair(true);
    let input = PmLiveAccountInput::new(
        query_occurrence(1, 10, 11, 1, 40_000).expect("account occurrence"),
        collateral,
        conditional,
    )
    .expect("exact nominal asset pair");
    let result = monitor
        .ingest_account_live(input)
        .expect("exact live account cut applies");
    assert_eq!(result.report().account_foreign().unwrap().count(), 1);

    let config = connectivity_config();
    let account = config.account();
    let projection = monitor.private_projection();
    assert_eq!(
        projection.account_snapshot().collateral().value(),
        Some(U256::from_u64(1_000))
    );
    assert_eq!(
        projection.account_snapshot().outcome_balance().value(),
        Some(U256::from_u64(25))
    );
    assert_eq!(
        projection.account_snapshot().position(),
        PmPositionKnowledge::Tradable(U256::from_u64(25))
    );
    assert_eq!(
        projection.allowance(account.required_spenders()[0]),
        PmAllowanceKnowledge::Present(PmAllowanceValue::Erc20(U256::from_u64(100)))
    );

    let (collateral, _) = account_pair(false);
    let wrong_token = PmTokenId::new(U256::from_u64(TOKEN + 1)).expect("other token");
    let conditional = balance(
        PmAccountAsset::Conditional(wrong_token),
        25,
        account.required_spenders()[1].requirement().spender(),
        1,
        false,
    );
    let input = PmLiveAccountInput::new(
        query_occurrence(1, 20, 21, 2, 41_000).expect("wrong-token occurrence"),
        collateral,
        conditional,
    )
    .expect("nominal pair alone is not configured-token proof");
    assert!(matches!(
        monitor.ingest_account_live(input),
        Err(PmPrivateMonitorError::ConditionalTokenMismatch)
    ));
}

#[test]
fn unchanged_full_account_fill_content_still_advances_the_causal_cursor() {
    let mut monitor = connected_monitor();
    let (collateral, conditional) = account_pair(false);
    let first = PmLiveReconciliationInput::new(
        query_occurrence(1, 10, 11, 1, 40_000).expect("first reconciliation occurrence"),
        collateral,
        conditional,
        None,
        trade_cut(&[]),
    )
    .expect("first complete input");
    let first = monitor
        .ingest_reconciliation_live(first)
        .expect("first complete cut applies");
    let first_cursor = monitor.runtime.fill_watermark().expect("first cursor");
    let first_digest = first
        .report()
        .full_account_fill_digest()
        .expect("full-account digest");
    assert!(first.report().fill_foreign().unwrap().is_empty());

    let (collateral, conditional) = account_pair(false);
    let second = PmLiveReconciliationInput::new(
        query_occurrence(1, 20, 21, 2, 41_000).expect("second reconciliation occurrence"),
        collateral,
        conditional,
        Some(first_cursor),
        trade_cut(&[]),
    )
    .expect("second complete input");
    let second = monitor
        .ingest_reconciliation_live(second)
        .expect("second unchanged-content cut applies");
    let second_cursor = monitor.runtime.fill_watermark().expect("second cursor");

    assert_ne!(first_cursor, second_cursor);
    assert_eq!(
        first_digest,
        second
            .report()
            .full_account_fill_digest()
            .expect("same full-account digest")
    );
}

#[test]
fn exact_order_detail_input_retains_the_requested_identity() {
    let mut monitor = connected_monitor();
    let requested = PmVenueOrderKey::new(
        connectivity_config().account().account_scope().handle(),
        PmVenueOrderId::new(ORDER_A).expect("fixed venue order"),
    );
    let input = PmLiveOrderDetailInput::new(
        query_occurrence(1, 10, 11, 1, 40_000).expect("detail occurrence"),
        requested,
        reap_polymarket_live_adapter::PmExactOrderObservation::Absent,
    );
    let result = monitor
        .ingest_order_detail_live(input)
        .expect("exact absence remains typed");
    assert_eq!(result.report(), PmLiveIngressReport::OrderDetail);
}
