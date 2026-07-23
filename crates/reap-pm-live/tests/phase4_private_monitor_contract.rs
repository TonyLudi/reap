mod support;

use reap_pm_core::{
    ConnectionEpoch, EventOrdering, IngressSequence, PmAccountHandle, PmAccountScope,
    PmAllowanceValue, PmErc1155OperatorApproval, PmFillQueryCursor, PmOrderSide,
    PmPositionAvailability, PmPrice, PmQuantity, PmSnapshotEvidence, PmVenueOrderId,
    PmVenueOrderKey, ReceivedEventClock, SnapshotRevision, U256,
};
use reap_pm_live::{
    PmAccountFixtureInput, PmFixtureQueryOccurrence, PmOpenOrdersFixtureInput,
    PmOrderDetailFixtureInput, PmPrivateMonitorError, PmReadOnlyMonitor,
    PmReconciliationFixtureInput,
};
use reap_pm_state::{
    PmExactReservation, PmObservedAmount, PmOrderOwnership, PmPrivateConvergence,
    PmPrivateExternalIngressFailure, PmPrivateExternalIngressFault, PmPrivateExternalIngressLane,
    PmPrivateHaltReason, PmPrivateOccurrence, PmPrivateQuoteRequest, PmPrivateReadiness,
    PmPrivateReadinessReason, PmPrivateStateError, PmReservationBasis, PmReservationKnowledge,
};
use reap_polymarket_adapter::{
    PmFixtureAllowanceRow, PmFixtureBalanceRow, PmFixtureCompletionOccurrence,
    PmFixtureFeeEvidence, PmFixtureInstrumentScope, PmFixturePositionRow,
};
use serde_json::{Value, json};

fn completion(
    epoch: u64,
    sequence: u64,
    snapshot_revision: Option<u64>,
) -> PmFixtureCompletionOccurrence {
    PmFixtureCompletionOccurrence::new(
        ReceivedEventClock::new(None, 10_000 + sequence, 20_000 + sequence).unwrap(),
        EventOrdering::new(
            ConnectionEpoch::new(epoch),
            snapshot_revision.map(SnapshotRevision::new),
            None,
            None,
            IngressSequence::new(sequence),
        )
        .unwrap(),
    )
}

fn order(id: &str, status: &str) -> Value {
    json!({
        "event_type": "order",
        "id": id,
        "market": support::MARKET,
        "asset_id": support::TOKEN.to_string(),
        "side": "BUY",
        "original_size": "5",
        "size_matched": "0",
        "price": "0.40",
        "status": status,
        "maker_address": support::PM_FUNDER,
        "type": "PLACEMENT"
    })
}

fn trade(id: &str, order_id: &str) -> Value {
    json!({
        "event_type": "trade",
        "id": id,
        "market": support::MARKET,
        "asset_id": support::TOKEN.to_string(),
        "side": "BUY",
        "size": "5",
        "price": "0.40",
        "status": "MATCHED",
        "maker_address": support::PM_FUNDER,
        "transaction_hash": "0xfeed",
        "order_id": order_id,
        "trader_side": "TAKER"
    })
}

fn unresolved_trade(id: &str) -> Value {
    json!({
        "event_type": "trade",
        "id": id,
        "market": support::MARKET,
        "asset_id": support::TOKEN.to_string(),
        "side": "BUY",
        "size": "5",
        "price": "0.40",
        "status": "MATCHED",
        "maker_address": support::PM_FUNDER,
        "transaction_hash": "0xdead"
    })
}

fn raw(value: &Value) -> Vec<u8> {
    serde_json::to_vec(value).unwrap()
}

fn quote(now: u64) -> PmPrivateQuoteRequest {
    PmPrivateQuoteRequest::new(
        now,
        PmOrderSide::Buy,
        PmPrice::parse_decimal("0.40").unwrap(),
        PmQuantity::parse_decimal("5").unwrap(),
        PmExactReservation::policy_approved(U256::from_u64(2_000_000), U256::ZERO).unwrap(),
    )
}

fn other_account_scope() -> PmAccountScope {
    let scope = support::account_scope();
    PmAccountScope::new(
        scope.environment(),
        scope.chain(),
        scope.signer(),
        scope.funder(),
        PmAccountHandle::from_ordinal(scope.handle().ordinal() + 1),
    )
}

#[test]
fn owner_bound_fixture_ingress_reduces_one_private_state_and_reconciles_one_exact_cut() {
    let config = support::account_config();
    let mut monitor =
        PmReadOnlyMonitor::new(config.clone(), support::private_risk_limits()).unwrap();
    monitor
        .reconnect_private(ConnectionEpoch::new(1), 20_000)
        .unwrap();

    let order_raw = raw(&order("order-1", "LIVE"));
    let order_apply = monitor
        .ingest_private_fixture(
            completion(1, 1, None),
            30_001,
            &order_raw,
            PmFixtureFeeEvidence::Unknown,
        )
        .unwrap();
    assert_eq!(order_apply.order_observations(), 1);
    let projection = monitor.private_projection();
    let orders = projection.orders().collect::<Vec<_>>();
    assert_eq!(orders.len(), 1);
    assert_eq!(orders[0].ownership(), PmOrderOwnership::Unmanaged);
    assert_eq!(
        orders[0].reservation(),
        Some(PmReservationKnowledge::Unknown)
    );
    assert!(projection.pending_refresh_count() >= 2);
    assert!(matches!(
        projection.convergence(),
        PmPrivateConvergence::Divergent { .. }
    ));

    let trade_raw = raw(&trade("trade-1", "order-1"));
    let first_fill = monitor
        .ingest_private_fixture(
            completion(1, 2, None),
            30_002,
            &trade_raw,
            PmFixtureFeeEvidence::Unknown,
        )
        .unwrap();
    assert_eq!(first_fill.fill_observations(), 1);
    let before_duplicate = monitor.private_projection().provisional_deltas();
    assert_eq!(before_duplicate.uncovered_fills(), 1);
    assert_eq!(
        before_duplicate.outcome().magnitude(),
        U256::from_u64(5_000_000)
    );

    let duplicate = monitor
        .ingest_private_fixture(
            completion(1, 3, None),
            30_003,
            &trade_raw,
            PmFixtureFeeEvidence::Unknown,
        )
        .unwrap();
    assert_eq!(duplicate.duplicate_or_stale_observations(), 1);
    assert_eq!(
        monitor.private_projection().provisional_deltas(),
        before_duplicate
    );

    monitor
        .reconnect_private(ConnectionEpoch::new(2), 40_000)
        .unwrap();
    let epoch_two_order = raw(&order("order-2", "LIVE"));
    monitor
        .ingest_private_fixture(
            completion(2, 1, None),
            40_001,
            &epoch_two_order,
            PmFixtureFeeEvidence::Unknown,
        )
        .unwrap();
    let old_epoch = monitor
        .ingest_private_fixture(
            completion(1, 100, None),
            40_100,
            &epoch_two_order,
            PmFixtureFeeEvidence::Unknown,
        )
        .unwrap_err();
    assert!(matches!(
        old_epoch,
        PmPrivateMonitorError::PrivateNormalization(
            reap_polymarket_adapter::PmPrivateNormalizationError::ConnectionEpochMismatch
        )
    ));

    let unresolved_raw = raw(&unresolved_trade("trade-unresolved"));
    monitor
        .ingest_private_fixture(
            completion(2, 2, None),
            40_002,
            &unresolved_raw,
            PmFixtureFeeEvidence::Incomplete,
        )
        .unwrap();
    assert_eq!(monitor.private_projection().unresolved_fills().count(), 1);

    let domain = config.trading_domain();
    let snapshot = PmSnapshotEvidence::new(SnapshotRevision::new(9)).unwrap();
    let balances = [
        PmFixtureBalanceRow::new(domain.collateral(), U256::from_u64(1_000_000_000)),
        PmFixtureBalanceRow::new(domain.outcome(), U256::from_u64(100_000_000)),
    ];
    let spenders = config.required_spenders();
    let allowances = [
        PmFixtureAllowanceRow::new(
            spenders[0],
            match spenders[0].requirement().asset() {
                asset if asset == domain.collateral() => {
                    PmAllowanceValue::Erc20(U256::from_u64(1_000_000_000))
                }
                _ => PmAllowanceValue::Erc1155Operator(PmErc1155OperatorApproval::from_bool(true)),
            },
        ),
        PmFixtureAllowanceRow::new(
            spenders[1],
            match spenders[1].requirement().asset() {
                asset if asset == domain.collateral() => {
                    PmAllowanceValue::Erc20(U256::from_u64(1_000_000_000))
                }
                _ => PmAllowanceValue::Erc1155Operator(PmErc1155OperatorApproval::from_bool(true)),
            },
        ),
    ];
    let instrument_scope =
        PmFixtureInstrumentScope::from_metadata(config.instrument(), config.expected_metadata())
            .unwrap();
    let positions = [PmFixturePositionRow::new(
        instrument_scope,
        U256::from_u64(100_000_000),
        PmPositionAvailability::Tradable,
    )];
    let query = PmFixtureQueryOccurrence::new(
        ConnectionEpoch::new(2),
        IngressSequence::new(10),
        snapshot,
        completion(2, 11, Some(9)),
        50_000,
    )
    .unwrap();
    let fill_frames: [&[u8]; 1] = [&trade_raw];
    let reconciliation = PmReconciliationFixtureInput::new(
        query,
        &balances,
        &allowances,
        &positions,
        None,
        PmFillQueryCursor::new(config.account_scope(), [1; 32]),
        &fill_frames,
        PmFixtureFeeEvidence::Unknown,
    );
    monitor
        .ingest_reconciliation_fixture(reconciliation)
        .unwrap();

    let projection = monitor.private_projection();
    assert!(matches!(
        projection.convergence(),
        PmPrivateConvergence::Converged { boundary, .. }
            if boundary.request_sequence() == IngressSequence::new(10)
    ));
    assert_eq!(projection.provisional_deltas().uncovered_fills(), 0);
    assert_eq!(
        projection
            .unresolved_fills()
            .filter(|fill| fill.is_active())
            .count(),
        0
    );
    assert_eq!(
        projection.account_snapshot().completion(),
        Some(PmPrivateOccurrence::new(
            ConnectionEpoch::new(2),
            IngressSequence::new(11)
        ))
    );
}

#[test]
fn normalization_and_service_clock_failures_never_bypass_owner_bound_reduction() {
    let mut monitor =
        PmReadOnlyMonitor::new(support::account_config(), support::private_risk_limits()).unwrap();
    monitor
        .reconnect_private(ConnectionEpoch::new(1), 20_000)
        .unwrap();

    let wrong_market = json!({
        "event_type": "order",
        "id": "wrong-market",
        "market": "0xcccccccccccccccccccccccccccccccccccccccccccccccccccccccccccccccc",
        "asset_id": support::TOKEN.to_string(),
        "side": "BUY",
        "original_size": "5",
        "size_matched": "0",
        "price": "0.40",
        "status": "LIVE",
        "maker_address": support::PM_FUNDER,
        "type": "PLACEMENT"
    });
    assert!(matches!(
        monitor.ingest_private_fixture(
            completion(1, 1, None),
            30_000,
            &raw(&wrong_market),
            PmFixtureFeeEvidence::Unknown,
        ),
        Err(PmPrivateMonitorError::PrivateNormalization(_))
    ));
    assert_eq!(monitor.private_projection().orders().count(), 0);

    let duplicate_batch = raw(&json!([
        order("same-order", "LIVE"),
        order("same-order", "LIVE")
    ]));
    assert!(matches!(
        monitor.ingest_private_fixture(
            completion(1, 1, None),
            30_001,
            &duplicate_batch,
            PmFixtureFeeEvidence::Unknown,
        ),
        Err(PmPrivateMonitorError::DuplicateBatchIdentity)
    ));
    assert_eq!(monitor.private_projection().orders().count(), 0);

    let valid = raw(&order("service-before-receive", "LIVE"));
    assert!(matches!(
        monitor.ingest_private_fixture(
            completion(1, 2, None),
            1,
            &valid,
            PmFixtureFeeEvidence::Unknown,
        ),
        Err(PmPrivateMonitorError::Envelope(
            reap_pm_core::EnvelopeError::ServiceBeforeReceive
        ))
    ));
    assert_eq!(monitor.private_projection().orders().count(), 0);

    monitor
        .ingest_private_fixture(
            completion(1, 2, None),
            30_002,
            &valid,
            PmFixtureFeeEvidence::Unknown,
        )
        .unwrap();
    let projection = monitor.private_projection();
    assert_eq!(projection.orders().count(), 1);
    let counters = projection.external_ingress_counters();
    assert_eq!(counters.total(), 3);
    assert_eq!(
        counters.for_lane(PmPrivateExternalIngressLane::PrivateLifecycle),
        3
    );
    assert_eq!(
        counters.for_failure(PmPrivateExternalIngressFailure::Normalization),
        1
    );
    assert_eq!(
        counters.for_failure(PmPrivateExternalIngressFailure::Service),
        1
    );
    assert_eq!(
        counters.for_failure(PmPrivateExternalIngressFailure::Contract),
        1
    );
    let fault = PmPrivateExternalIngressFault::new(
        PmPrivateExternalIngressLane::PrivateLifecycle,
        PmPrivateExternalIngressFailure::Normalization,
    );
    assert_eq!(
        projection.halt(),
        Some(PmPrivateHaltReason::ExternalIngressFault(fault))
    );
    assert_eq!(
        projection.quote_readiness(quote(30_003)),
        PmPrivateReadiness::Blocked(PmPrivateReadinessReason::Halted(
            PmPrivateHaltReason::ExternalIngressFault(fault)
        ))
    );
}

#[test]
fn failed_reconnect_is_transactional_and_same_epoch_retry_accepts_sequence_one() {
    let mut monitor =
        PmReadOnlyMonitor::new(support::account_config(), support::private_risk_limits()).unwrap();
    assert!(matches!(
        monitor.reconnect_private(ConnectionEpoch::new(1), 0),
        Err(PmPrivateMonitorError::State(
            PmPrivateStateError::InvalidReconnectEvidence
        ))
    ));
    let counters = monitor.private_projection().external_ingress_counters();
    assert_eq!(
        counters.for_lane(PmPrivateExternalIngressLane::Reconnect),
        1
    );
    assert_eq!(
        counters.for_failure(PmPrivateExternalIngressFailure::Contract),
        1
    );

    monitor
        .reconnect_private(ConnectionEpoch::new(1), 20_000)
        .unwrap();
    let first = raw(&order("retry-sequence-one", "LIVE"));
    let applied = monitor
        .ingest_private_fixture(
            completion(1, 1, None),
            30_001,
            &first,
            PmFixtureFeeEvidence::Unknown,
        )
        .unwrap();
    assert_eq!(applied.order_observations(), 1);
}

#[test]
fn private_batch_partial_progress_is_explicit_and_fail_closed() {
    let mut monitor =
        PmReadOnlyMonitor::new(support::account_config(), support::private_risk_limits()).unwrap();
    monitor
        .reconnect_private(ConnectionEpoch::new(1), 20_000)
        .unwrap();
    let seeded = raw(&trade("partial-fill", "seeded-order"));
    monitor
        .ingest_private_fixture(
            completion(1, 1, None),
            30_001,
            &seeded,
            PmFixtureFeeEvidence::Unknown,
        )
        .unwrap();

    let mut conflicting = trade("partial-fill", "seeded-order");
    conflicting["size"] = json!("10");
    let batch = raw(&json!([order("visible-prefix-order", "LIVE"), conflicting]));
    let error = monitor
        .ingest_private_fixture(
            completion(1, 2, None),
            30_002,
            &batch,
            PmFixtureFeeEvidence::Unknown,
        )
        .unwrap_err();
    let PmPrivateMonitorError::PrivateBatchPartial { applied, source } = error else {
        panic!("later failure must expose the exact applied prefix");
    };
    assert_eq!(applied.order_observations(), 1);
    assert_eq!(applied.fill_observations(), 0);
    assert!(matches!(
        *source,
        PmPrivateMonitorError::State(PmPrivateStateError::Fill(_))
    ));
    let projection = monitor.private_projection();
    assert_eq!(projection.orders().count(), 1);
    assert_eq!(projection.fills().count(), 1);
    assert_eq!(
        projection
            .external_ingress_counters()
            .for_failure(PmPrivateExternalIngressFailure::Contract),
        1
    );
    assert_eq!(
        projection.halt(),
        Some(PmPrivateHaltReason::ExternalIngressFault(
            PmPrivateExternalIngressFault::new(
                PmPrivateExternalIngressLane::PrivateLifecycle,
                PmPrivateExternalIngressFailure::Contract,
            )
        ))
    );
}

#[test]
fn every_read_lane_failure_is_latched_and_metered_before_reduction() {
    let config = support::account_config();
    let mut monitor =
        PmReadOnlyMonitor::new(config.clone(), support::private_risk_limits()).unwrap();
    monitor
        .reconnect_private(ConnectionEpoch::new(1), 20_000)
        .unwrap();

    let snapshot = PmSnapshotEvidence::new(SnapshotRevision::new(1)).unwrap();
    let future_account = PmFixtureQueryOccurrence::new(
        ConnectionEpoch::new(2),
        IngressSequence::new(1),
        snapshot,
        completion(2, 2, Some(1)),
        30_000,
    )
    .unwrap();
    assert!(matches!(
        monitor.ingest_account_fixture(PmAccountFixtureInput::new(future_account, &[], &[], &[],)),
        Err(PmPrivateMonitorError::PrivateEpochMismatch)
    ));

    let open = PmFixtureQueryOccurrence::new(
        ConnectionEpoch::new(1),
        IngressSequence::new(1),
        snapshot,
        completion(1, 2, Some(1)),
        30_001,
    )
    .unwrap();
    let invalid_json: [&[u8]; 1] = [b"{"];
    assert!(matches!(
        monitor.ingest_open_orders_fixture(PmOpenOrdersFixtureInput::new(open, &invalid_json,)),
        Err(PmPrivateMonitorError::Reconciliation(_))
    ));

    let detail = PmFixtureQueryOccurrence::new(
        ConnectionEpoch::new(1),
        IngressSequence::new(3),
        PmSnapshotEvidence::new(SnapshotRevision::new(2)).unwrap(),
        completion(1, 4, Some(2)),
        30_002,
    )
    .unwrap();
    let foreign_order = PmVenueOrderKey::new(
        other_account_scope().handle(),
        PmVenueOrderId::new("foreign-order").unwrap(),
    );
    assert!(matches!(
        monitor.ingest_order_detail_fixture(PmOrderDetailFixtureInput::new(
            detail,
            foreign_order,
            None,
        )),
        Err(PmPrivateMonitorError::Reconciliation(_))
    ));

    let reconciliation = PmFixtureQueryOccurrence::new(
        ConnectionEpoch::new(1),
        IngressSequence::new(5),
        PmSnapshotEvidence::new(SnapshotRevision::new(3)).unwrap(),
        completion(1, 6, Some(3)),
        30_003,
    )
    .unwrap();
    let input = PmReconciliationFixtureInput::new(
        reconciliation,
        &[],
        &[],
        &[],
        Some(PmFillQueryCursor::new(other_account_scope(), [9; 32])),
        PmFillQueryCursor::new(config.account_scope(), [10; 32]),
        &[],
        PmFixtureFeeEvidence::Unknown,
    );
    assert!(matches!(
        monitor.ingest_reconciliation_fixture(input),
        Err(PmPrivateMonitorError::Reconciliation(_))
    ));

    let counters = monitor.private_projection().external_ingress_counters();
    assert_eq!(counters.total(), 4);
    assert_eq!(
        counters.for_lane(PmPrivateExternalIngressLane::AccountSnapshot),
        1
    );
    assert_eq!(
        counters.for_lane(PmPrivateExternalIngressLane::OpenOrders),
        1
    );
    assert_eq!(
        counters.for_lane(PmPrivateExternalIngressLane::OrderDetail),
        1
    );
    assert_eq!(
        counters.for_lane(PmPrivateExternalIngressLane::Reconciliation),
        1
    );
    assert_eq!(
        counters.for_failure(PmPrivateExternalIngressFailure::Normalization),
        1
    );
    assert_eq!(
        counters.for_failure(PmPrivateExternalIngressFailure::Scope),
        3
    );
}

#[test]
fn account_and_reconciliation_reads_cannot_create_or_advance_a_private_epoch() {
    let config = support::account_config();
    let mut monitor = PmReadOnlyMonitor::new(config, support::private_risk_limits()).unwrap();
    let snapshot = PmSnapshotEvidence::new(SnapshotRevision::new(1)).unwrap();
    let query_without_private = PmFixtureQueryOccurrence::new(
        ConnectionEpoch::new(1),
        IngressSequence::new(1),
        snapshot,
        completion(1, 2, Some(1)),
        30_000,
    )
    .unwrap();
    assert!(matches!(
        monitor.ingest_account_fixture(PmAccountFixtureInput::new(
            query_without_private,
            &[],
            &[],
            &[],
        )),
        Err(PmPrivateMonitorError::PrivateEpochMismatch)
    ));

    monitor
        .reconnect_private(ConnectionEpoch::new(1), 30_001)
        .unwrap();
    let future_query = PmFixtureQueryOccurrence::new(
        ConnectionEpoch::new(2),
        IngressSequence::new(1),
        snapshot,
        completion(2, 2, Some(1)),
        31_000,
    )
    .unwrap();
    assert!(matches!(
        monitor.ingest_account_fixture(PmAccountFixtureInput::new(future_query, &[], &[], &[],)),
        Err(PmPrivateMonitorError::PrivateEpochMismatch)
    ));
    monitor
        .reconnect_private(ConnectionEpoch::new(2), 31_001)
        .unwrap();
}

#[test]
fn one_shot_account_open_order_and_detail_reads_keep_ownership_and_absence_exact() {
    let config = support::account_config();
    let mut monitor =
        PmReadOnlyMonitor::new(config.clone(), support::private_risk_limits()).unwrap();
    monitor
        .reconnect_private(ConnectionEpoch::new(1), 20_000)
        .unwrap();

    let buy = json!({
        "id": "remote-buy",
        "market": support::MARKET,
        "asset_id": support::TOKEN.to_string(),
        "side": "BUY",
        "original_size": "5",
        "size_matched": "0",
        "price": "0.40",
        "status": "LIVE",
        "maker_address": support::PM_FUNDER
    });
    let sell = json!({
        "id": "remote-sell",
        "market": support::MARKET,
        "asset_id": support::TOKEN.to_string(),
        "side": "SELL",
        "original_size": "5",
        "size_matched": "0",
        "price": "0.60",
        "status": "LIVE",
        "maker_address": support::PM_FUNDER
    });
    let buy_raw = raw(&buy);
    let sell_raw = raw(&sell);
    let snapshot = PmSnapshotEvidence::new(SnapshotRevision::new(1)).unwrap();
    let open_occurrence = PmFixtureQueryOccurrence::new(
        ConnectionEpoch::new(1),
        IngressSequence::new(1),
        snapshot,
        completion(1, 2, Some(1)),
        30_000,
    )
    .unwrap();
    let rows: [&[u8]; 2] = [&buy_raw, &sell_raw];
    monitor
        .ingest_open_orders_fixture(PmOpenOrdersFixtureInput::new(open_occurrence, &rows))
        .unwrap();

    let orders = monitor.private_projection().orders().collect::<Vec<_>>();
    assert_eq!(orders.len(), 2);
    assert!(
        orders
            .iter()
            .all(|order| order.ownership() == PmOrderOwnership::Unmanaged)
    );
    let buy_projection = orders
        .iter()
        .copied()
        .find(|order| order.identity().venue_order_key().unwrap().id().as_str() == "remote-buy")
        .unwrap();
    assert_eq!(
        buy_projection.reservation(),
        Some(PmReservationKnowledge::Unknown)
    );
    let sell_projection = orders
        .iter()
        .copied()
        .find(|order| order.identity().venue_order_key().unwrap().id().as_str() == "remote-sell")
        .unwrap();
    let Some(PmReservationKnowledge::Known(sell_reservation)) = sell_projection.reservation()
    else {
        panic!("complete live sell must retain exact authoritative remaining inventory");
    };
    assert_eq!(
        sell_reservation.basis(),
        PmReservationBasis::AuthoritativeSellRemaining
    );
    assert_eq!(sell_reservation.outcome(), U256::from_u64(5_000_000));

    let detail_occurrence = PmFixtureQueryOccurrence::new(
        ConnectionEpoch::new(1),
        IngressSequence::new(3),
        PmSnapshotEvidence::new(SnapshotRevision::new(2)).unwrap(),
        completion(1, 4, Some(2)),
        31_000,
    )
    .unwrap();
    monitor
        .ingest_order_detail_fixture(PmOrderDetailFixtureInput::new(
            detail_occurrence,
            sell_projection.identity().venue_order_key().unwrap(),
            None,
        ))
        .unwrap();
    let sell_after_absence = monitor
        .private_projection()
        .orders()
        .find(|order| order.identity() == sell_projection.identity())
        .unwrap();
    assert!(sell_after_absence.terminal_by_detail_absence());
    assert_eq!(sell_after_absence.reservation(), None);

    let account_occurrence = PmFixtureQueryOccurrence::new(
        ConnectionEpoch::new(1),
        IngressSequence::new(1),
        snapshot,
        completion(1, 2, Some(1)),
        32_000,
    )
    .unwrap();
    monitor
        .ingest_account_fixture(PmAccountFixtureInput::new(
            account_occurrence,
            &[],
            &[],
            &[],
        ))
        .unwrap();
    let account = monitor.private_projection().account_snapshot();
    assert_eq!(account.collateral(), PmObservedAmount::ExplicitAbsent);
    assert_eq!(account.outcome_balance(), PmObservedAmount::ExplicitAbsent);
}
