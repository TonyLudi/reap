use std::future::Future;
use std::time::Duration;

use reap_pm_core::{
    PmFillQueryCursor, PmOrderSide, PmPositionAvailability, PmSignedUnits, PmVenueOrderId,
    PmVenueOrderKey, U256,
};
use reap_pm_state::{
    PmPrivateExternalIngressFailure, PmPrivateExternalIngressFault, PmPrivateExternalIngressLane,
};
use reap_pm_strategy::PmQuoteModel;
use reap_polymarket_adapter::{
    PmFakeCancelScript, PmFakePlaceScript, PmFixtureBalanceRow, PmFixtureFeeEvidence,
    PmFixtureInstrumentScope, PmFixturePositionRow,
};

use crate::evidence::{
    allowance_row, complete_reached_overload_reconciliation, completion, connectivity_config,
    prepare_reached_overload_product, query_occurrence, start_reached_overload_product,
};
use crate::{
    PmCancelIntentReason, PmControlReason, PmDurableRecordKind, PmEffectDispatchStage, PmLaneKind,
    PmLanePolicy, PmMutationHalt, PmOpenOrdersFixtureInput, PmOrderDetailFixtureInput,
    PmProductEffect, PmProductRun, PmReconciliationFixtureInput, PmRefreshEffectKind,
    PmScheduledActionKind, SaturationAction,
};

const MINIMUM_PRODUCT_STACK_BYTES: usize = 2 * 1024 * 1024;

#[test]
fn recovered_refreshes_wait_for_open_orders_before_missing_detail_admission() {
    let source = include_str!("service.rs");
    let connection = source
        .split_once("PmPrivateInput::ConnectionAvailable => {")
        .expect("connection-available service arm remains explicit")
        .1
        .split_once("PmPrivateInput::ConnectionUnavailable")
        .expect("connection-available arm remains bounded")
        .0;
    let reconnect = connection
        .find("PmRefreshReason::PrivateReconnect")
        .expect("private reconnect ticket is admitted first");
    let ambiguous = connection
        .find("PmRefreshReason::AmbiguousOrder")
        .expect("ambiguous-order ticket is admitted explicitly");
    let next = connection
        .find("let Some(next) = self.mutation.next_pending_refresh()")
        .expect("recovered refresh admission observes the typed FIFO head");
    let missing = connection
        .find("next.key().reason() == PmRefreshReason::MissingOrderDetail")
        .expect("missing detail remains pending until OpenOrders applies");
    let recovered = connection
        .find("self.admit_next_refresh(clock.monotonic_service_ns(), effects)?")
        .expect("pre-existing recovery tickets use typed admission");
    let saturation = connection
        .find("self.mutation.next_pending_refresh().is_some_and(|ticket|")
        .expect("an overfull recovered set fails closed");
    assert!(
        reconnect < ambiguous
            && ambiguous < next
            && next < missing
            && missing < recovered
            && recovered < saturation
    );
    assert!(
        connection.contains("while effects.len() < MAX_PM_EFFECTS_PER_INPUT.saturating_sub(1)")
    );
    assert!(connection.contains("PmCoordinatorError::EffectProjectionSaturated"));

    let reconciliation = include_str!("service/reconciliation.rs");
    let open_orders = reconciliation
        .split_once("PmReconciliationInput::OpenOrders(delivery) => {")
        .expect("OpenOrders reconciliation arm remains explicit")
        .1
        .split_once("PmReconciliationInput::OrderDetail(delivery) => {")
        .expect("OpenOrders reconciliation arm remains bounded")
        .0;
    let complete_ambiguous = open_orders
        .find("PmRefreshReason::AmbiguousOrder")
        .expect("OpenOrders completes the ambiguous-order generation first");
    let canonical_missing = open_orders
        .find("self.mutation.has_missing_order_detail()")
        .expect("OpenOrders consults current canonical detail state");
    let admit_missing = open_orders
        .find("PmRefreshReason::MissingOrderDetail")
        .expect("OpenOrders admits the current missing-detail generation");
    let drain_remaining = open_orders
        .find("self.admit_next_refresh(monotonic_service_ns, effects)?")
        .expect("remaining recovered tickets drain after missing detail");
    let fail_closed = open_orders
        .find("self.mutation.next_pending_refresh().is_some()")
        .expect("unexpected remaining recovery work fails closed");
    assert!(
        complete_ambiguous < canonical_missing
            && canonical_missing < admit_missing
            && admit_missing < drain_remaining
            && drain_remaining < fail_closed
    );
    assert!(
        open_orders.contains("while effects.len() < MAX_PM_EFFECTS_PER_INPUT.saturating_sub(1)")
    );
}

#[test]
fn product_start_service_and_shutdown_fit_two_mebibyte_stack() {
    run_minimum_stack_product_test(minimum_stack_product_lifecycle_test);
}

async fn minimum_stack_product_lifecycle_test() {
    let directory = tempfile::tempdir().expect("temporary minimum-stack directory");
    let mut run = start_reached_overload_product(
        directory.path().join("minimum-stack-capture.jsonl"),
        directory.path().join("minimum-stack-journal.jsonl"),
    )
    .await
    .expect("product starts on the minimum supported stack");
    let serviced = run
        .service_turn(1)
        .expect("an empty service turn fits the minimum supported stack");
    assert_eq!(serviced.total(), 0);
    run.shutdown()
        .await
        .expect("product shutdown fits the minimum supported stack");
}

#[test]
fn private_reconnect_requires_account_trades_and_complete_open_orders() {
    run_product_test(private_reconnect_refresh_lifecycle_test);
}

async fn private_reconnect_refresh_lifecycle_test() {
    let directory = tempfile::tempdir().expect("temporary reconnect-refresh directory");
    let mut run = Box::new(
        Box::pin(start_reached_overload_product(
            directory.path().join("reconnect-refresh-capture.jsonl"),
            directory.path().join("reconnect-refresh-journal.jsonl"),
        ))
        .await
        .expect("reconnect-refresh product starts"),
    );

    run.connect_private_fixture(completion(1, 1, None, 120))
        .expect("private reconnect reaches its sole owner");
    run.service_turn(121).expect("private reconnect is reduced");
    let reconnect_effects = drain_effects(&mut run);
    assert_eq!(
        reconciliation_refresh_count(&reconnect_effects),
        2,
        "the canonical reconnect emits both complete-reconciliation and open-orders requests"
    );
    assert_eq!(
        reconciliation_refresh_kinds(&reconnect_effects),
        vec![
            PmRefreshEffectKind::CompleteReconciliation,
            PmRefreshEffectKind::OpenOrders,
        ]
    );
    let admitted = run.refresh_obligation_metrics();
    assert_eq!(admitted.canonical_insertions(), 2);
    assert_eq!(admitted.total_pending(), 2);
    assert_eq!(admitted.total_in_flight(), 2);
    run.service_turn(122)
        .expect("the reconnect-triggered quote check is serviced on time");
    drain_effects(&mut run);

    let config = connectivity_config();
    let account = config.account();
    let domain = account.trading_domain();
    let balances = [
        PmFixtureBalanceRow::new(domain.collateral(), U256::from_u64(10_000_000_000)),
        PmFixtureBalanceRow::new(domain.outcome(), U256::from_u64(10_000_000_000)),
    ];
    let spenders = account.required_spenders();
    let allowances = [
        allowance_row(spenders[0], domain.collateral()),
        allowance_row(spenders[1], domain.collateral()),
    ];
    let instrument_scope =
        PmFixtureInstrumentScope::from_metadata(account.instrument(), account.expected_metadata())
            .expect("fixed reconnect instrument scope");
    let positions = [PmFixturePositionRow::new(
        instrument_scope,
        U256::from_u64(10_000_000_000),
        PmPositionAvailability::Tradable,
    )];
    let no_fills: [&[u8]; 0] = [];
    run.ingest_reconciliation_fixture(PmReconciliationFixtureInput::new(
        query_occurrence(1, 4, 5, 2, 140).expect("fixed initial reconciliation occurrence"),
        &balances,
        &allowances,
        &positions,
        None,
        PmFillQueryCursor::new(account.account_scope(), [1; 32]),
        &no_fills,
        PmFixtureFeeEvidence::Known {
            asset: domain.collateral(),
            delta: PmSignedUnits::ZERO,
        },
    ))
    .expect("one exact complete account-plus-fill cut reaches reconciliation");
    run.service_turn(142)
        .expect("one exact complete account-plus-fill cut applies");
    drain_effects(&mut run);
    let after_pair = run.refresh_obligation_metrics();
    assert_eq!(after_pair.total_pending(), 1);
    assert_eq!(after_pair.total_in_flight(), 1);

    let empty_orders: [&[u8]; 0] = [];
    run.ingest_open_orders_fixture(PmOpenOrdersFixtureInput::new(
        query_occurrence(1, 6, 7, 3, 143).expect("fixed reconnect open-orders occurrence"),
        &empty_orders,
    ))
    .expect("the exact complete open-orders cut reaches reconciliation");
    run.service_turn(144)
        .expect("the exact complete open-orders cut applies");
    drain_effects(&mut run);
    let completed = run.refresh_obligation_metrics();
    assert_eq!(completed.total_pending(), 0);
    assert_eq!(completed.total_in_flight(), 0);

    let _ = Box::pin((*run).shutdown()).await;
}

#[test]
fn copied_refresh_retries_only_after_inclusive_age_boundary() {
    run_product_test(copied_refresh_age_test);
}

async fn copied_refresh_age_test() {
    let directory = tempfile::tempdir().expect("temporary refresh-age directory");
    let mut run = Box::new(
        Box::pin(start_reached_overload_product(
            directory.path().join("refresh-age-capture.jsonl"),
            directory.path().join("refresh-age-journal.jsonl"),
        ))
        .await
        .expect("refresh-age product starts"),
    );
    Box::pin(prepare_reached_overload_product(&mut run))
        .await
        .expect("refresh-age product becomes ready");
    drain_effects(&mut run);
    let initial = Box::pin(place_acknowledgement_unknown_quote(&mut run)).await;
    assert_eq!(reconciliation_refresh_count(&initial), 1);
    assert_eq!(
        reconciliation_refresh_kinds(&initial),
        vec![PmRefreshEffectKind::OpenOrders]
    );
    let retained_total = run.refresh_obligation_metrics().total_pending();

    run.service_turn(1_000_001_454)
        .expect("inclusive age boundary services without retry");
    assert_eq!(reconciliation_refresh_count(&drain_effects(&mut run)), 0);
    let at_boundary = run.refresh_obligation_metrics();
    assert_eq!(at_boundary.oldest_in_flight_age_ns(), 1_000_000_000);
    assert_eq!(at_boundary.retry_effects(), 0);

    run.service_turn(1_000_001_455)
        .expect("one nanosecond beyond the age boundary services");
    let retried_effects = drain_effects(&mut run);
    assert_eq!(reconciliation_refresh_count(&retried_effects), 1);
    assert_eq!(
        reconciliation_refresh_kinds(&retried_effects),
        vec![PmRefreshEffectKind::OpenOrders],
        "retry must preserve the originally admitted refresh purpose"
    );
    let retried = run.refresh_obligation_metrics();
    assert_eq!(retried.total_pending(), retained_total);
    assert_eq!(retried.total_in_flight(), 1);
    assert_eq!(retried.oldest_in_flight_age_ns(), 0);
    assert_eq!(retried.maximum_observed_age_ns(), 1_000_000_001);
    assert_eq!(retried.retry_effects(), 1);

    run.service_turn(1_000_001_456)
        .expect("following turn remains inside reset retry age");
    assert_eq!(reconciliation_refresh_count(&drain_effects(&mut run)), 0);
    let stable = run.refresh_obligation_metrics();
    assert_eq!(stable.total_pending(), retained_total);
    assert_eq!(stable.total_in_flight(), 1);
    assert_eq!(stable.oldest_in_flight_age_ns(), 1);
    assert_eq!(stable.retry_effects(), 1);

    let regression = run
        .service_turn(1_000_001_450)
        .expect_err("refresh age clock regression fails closed");
    assert!(regression.to_string().contains("monotonic clock regressed"));
    assert_eq!(run.halt(), Some(PmControlReason::ContractViolation));

    let _ = Box::pin((*run).shutdown()).await;
}

#[test]
fn acknowledgement_unknown_refresh_clears_only_on_authoritative_order_reconciliation() {
    run_product_test(acknowledgement_unknown_refresh_convergence_test);
}

async fn acknowledgement_unknown_refresh_convergence_test() {
    let directory = tempfile::tempdir().expect("temporary ambiguity directory");
    let mut run = Box::new(
        Box::pin(start_reached_overload_product(
            directory.path().join("ambiguity-capture.jsonl"),
            directory.path().join("ambiguity-journal.jsonl"),
        ))
        .await
        .expect("ambiguity product starts"),
    );
    Box::pin(prepare_reached_overload_product(&mut run))
        .await
        .expect("ambiguity product becomes ready");
    drain_effects(&mut run);
    let baseline_total = run.refresh_obligation_metrics().total_pending();

    let ambiguity_effects = Box::pin(place_acknowledgement_unknown_quote(&mut run)).await;
    assert_eq!(
        ambiguity_effects
            .iter()
            .filter(|effect| matches!(effect, PmProductEffect::ReconciliationRefresh(_)))
            .count(),
        1,
        "one copied refresh effect is retained for the ambiguous submit"
    );
    assert_eq!(
        reconciliation_refresh_kinds(&ambiguity_effects),
        vec![PmRefreshEffectKind::OpenOrders],
        "ambiguous ownership must query the authoritative open-orders purpose"
    );
    let admitted = run.refresh_obligation_metrics();
    assert_eq!(admitted.total_pending(), baseline_total + 1);
    assert_eq!(admitted.total_in_flight(), 1);
    assert_eq!(admitted.ambiguous_order_pending(), 1);
    assert_eq!(admitted.ambiguous_order_in_flight(), 1);

    let ambiguous_client = ambiguity_effects
        .iter()
        .find_map(|effect| match effect {
            PmProductEffect::PlaceGtcPostOnly(quote) => Some(quote.client_order()),
            _ => None,
        })
        .expect("ambiguous execution retains its client identity");
    let unrelated_venue = PmVenueOrderKey::new(
        ambiguous_client.account(),
        PmVenueOrderId::new("unrelated-detail").expect("fixed unrelated venue id"),
    );
    let unrelated_detail = br#"{
        "id":"unrelated-detail",
        "market":"0xbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbb",
        "asset_id":"123",
        "side":"BUY",
        "original_size":"5",
        "size_matched":"0",
        "price":"0.40",
        "status":"CANCELLED",
        "maker_address":"0xabababababababababababababababababababab"
    }"#;
    run.ingest_order_detail_fixture(PmOrderDetailFixtureInput::new(
        query_occurrence(1, 100, 101, 50, 1_500).expect("fixed unrelated detail occurrence"),
        unrelated_venue,
        Some(unrelated_detail),
    ))
    .expect("unrelated authoritative detail reaches reconciliation");
    run.service_turn(1_502)
        .expect("unrelated authoritative detail reduces");
    drain_effects(&mut run);
    let after_unrelated_detail = run.refresh_obligation_metrics();
    assert_eq!(after_unrelated_detail.ambiguous_order_pending(), 1);
    assert_eq!(after_unrelated_detail.ambiguous_order_in_flight(), 1);

    Box::pin(complete_reached_overload_reconciliation(&mut run, 1, &[]))
        .await
        .expect("account-plus-fill reconciliation applies");
    drain_effects(&mut run);
    let unrelated = run.refresh_obligation_metrics();
    assert_eq!(unrelated.total_pending(), baseline_total + 1);
    assert_eq!(unrelated.total_in_flight(), 1);
    assert_eq!(unrelated.ambiguous_order_pending(), 1);
    assert_eq!(unrelated.ambiguous_order_in_flight(), 1);

    let empty: [&[u8]; 0] = [];
    run.ingest_open_orders_fixture(PmOpenOrdersFixtureInput::new(
        query_occurrence(1, 20_100, 20_101, 100, 2_100_000_000)
            .expect("fixed authoritative open-orders occurrence"),
        &empty,
    ))
    .expect("authoritative open orders reach the reconciliation lane");
    run.service_turn(2_100_000_002)
        .expect("authoritative open orders reduce");
    let convergence_effects = drain_effects(&mut run);
    assert_eq!(
        reconciliation_refresh_kinds(&convergence_effects),
        vec![PmRefreshEffectKind::CompleteReconciliation],
        "the authoritative open cut must immediately dispatch the newly exposed unbound missing-detail obligation"
    );
    let converged = run.refresh_obligation_metrics();
    assert_eq!(
        converged.total_pending(),
        unrelated.total_pending(),
        "the complete snapshot clears ambiguity but retains its newly exposed missing-detail obligation"
    );
    assert_eq!(converged.total_in_flight(), 1);
    assert_eq!(converged.ambiguous_order_pending(), 0);
    assert_eq!(converged.ambiguous_order_in_flight(), 0);

    let _ = Box::pin((*run).shutdown()).await;
}

#[test]
fn recovered_live_order_is_cancelled_before_shutdown_halt_and_can_finish_fake_dispatch() {
    run_product_test(recovered_live_order_control_test);
}

async fn recovered_live_order_control_test() {
    let directory = tempfile::tempdir().expect("temporary control directory");
    let journal_path = directory.path().join("control-journal.jsonl");
    let mut first = Box::new(
        Box::pin(start_reached_overload_product(
            directory.path().join("control-first-capture.jsonl"),
            journal_path.clone(),
        ))
        .await
        .expect("first product starts"),
    );
    Box::pin(prepare_reached_overload_product(&mut first))
        .await
        .expect("first product becomes ready");
    drain_effects(&mut first);

    let client_order = Box::pin(place_live_quote(&mut first)).await;
    let _ = Box::pin((*first).shutdown()).await;

    let mut recovered = Box::new(
        Box::pin(start_reached_overload_product(
            directory.path().join("control-recovered-capture.jsonl"),
            journal_path,
        ))
        .await
        .expect("recovered product starts"),
    );
    assert_eq!(recovered.counters().quote_evaluations(), 0);
    recovered
        .request_shutdown(completion(2, 20, None, 2_000))
        .expect("shutdown reaches the critical lane");
    recovered
        .service_turn(2_001)
        .expect("shutdown admits canonical owned cancellation");

    let effects = drain_effects(&mut recovered);
    assert_eq!(effects.len(), 3);
    let PmProductEffect::DurableRecord(cancel_record) = effects[0] else {
        panic!("cancel intent must precede every stop projection");
    };
    assert_eq!(cancel_record.kind(), PmDurableRecordKind::CancelIntent);
    assert_eq!(cancel_record.client_order(), Some(client_order));
    assert_eq!(cancel_record.correlation(), 20);
    let PmProductEffect::FailClosedHaltOrCancel(cancel) = effects[1] else {
        panic!("owned cancel projection must precede final halt");
    };
    assert_eq!(cancel.reason(), PmControlReason::RequestedShutdown);
    assert_eq!(
        cancel.cancel_intent(),
        Some((client_order, PmCancelIntentReason::SafetyHalt))
    );
    let PmProductEffect::FailClosedHaltOrCancel(halt) = effects[2] else {
        panic!("final stop projection must be last");
    };
    assert_eq!(halt.reason(), PmControlReason::RequestedShutdown);
    assert_eq!(halt.cancel_intent(), None);
    assert_eq!(recovered.halt(), Some(PmControlReason::RequestedShutdown));

    Box::pin(wait_for_persistence(&mut recovered, 21, 2_002)).await;
    recovered
        .service_turn(2_003)
        .expect("durable cancel acknowledgement is serviced after halt");
    let prepared = drain_effects(&mut recovered)
        .into_iter()
        .find_map(|effect| match effect {
            PmProductEffect::CancelOwned(cancel) => Some(cancel),
            _ => None,
        })
        .expect("durability prepares the fake owned cancel after halt");
    assert_eq!(prepared.client_order(), client_order);
    assert_eq!(
        prepared.stage(),
        PmEffectDispatchStage::PreparedAfterDurability
    );

    recovered
        .execute_prepared_cancel_fixture(
            completion(2, 22, None, 2_004),
            PmFakeCancelScript::accepted(),
            2_004,
        )
        .expect("prepared cancel remains executable after halt");
    recovered
        .service_turn(2_005)
        .expect("fake cancel result reaches the halted owner");
    let cancel_effects = drain_effects(&mut recovered);
    let executed = cancel_effects
        .iter()
        .find_map(|effect| match effect {
            PmProductEffect::CancelOwned(cancel) => Some(*cancel),
            _ => None,
        })
        .expect("fake cancel execution is projected");
    assert_eq!(executed.client_order(), client_order);
    assert_eq!(executed.stage(), PmEffectDispatchStage::CompletedByBackend);
    assert_eq!(
        reconciliation_refresh_kinds(&cancel_effects),
        vec![PmRefreshEffectKind::OrderDetail(prepared.venue_order())],
        "an accepted fake cancel and an authenticated cancel share the exact missing-detail obligation"
    );
    let refresh = recovered.refresh_obligation_metrics();
    assert_eq!(refresh.total_pending(), 1);
    assert_eq!(refresh.total_in_flight(), 1);

    Box::pin(wait_for_persistence(&mut recovered, 23, 2_006)).await;
    recovered
        .service_turn(2_007)
        .expect("cancel result durability is serviced");
    drain_effects(&mut recovered);
    let _ = Box::pin((*recovered).shutdown()).await;
}

#[test]
fn recovered_live_order_is_cancelled_once_when_its_schedule_ages() {
    run_product_test(recovered_live_order_aged_schedule_test);
}

async fn recovered_live_order_aged_schedule_test() {
    let directory = tempfile::tempdir().expect("temporary aged-schedule directory");
    let journal_path = directory.path().join("aged-schedule-journal.jsonl");
    let mut first = Box::new(
        Box::pin(start_reached_overload_product(
            directory.path().join("aged-schedule-first-capture.jsonl"),
            journal_path.clone(),
        ))
        .await
        .expect("first product starts"),
    );
    Box::pin(prepare_reached_overload_product(&mut first))
        .await
        .expect("first product becomes ready");
    drain_effects(&mut first);

    let client_order = Box::pin(place_live_quote(&mut first)).await;
    let _ = Box::pin((*first).shutdown()).await;

    let mut recovered = Box::new(
        Box::pin(start_reached_overload_product(
            directory
                .path()
                .join("aged-schedule-recovered-capture.jsonl"),
            journal_path,
        ))
        .await
        .expect("recovered product starts"),
    );
    let maximum_age_ns = PmLanePolicy::for_lane(PmLaneKind::Scheduled)
        .maximum_age_ns()
        .expect("scheduled age policy");
    let deadline_ns = 10_000;
    let observed_ns = deadline_ns + maximum_age_ns + 1;
    recovered
        .schedule(
            PmOrderSide::Buy,
            PmScheduledActionKind::Freshness,
            deadline_ns,
            2_000,
            1_700_000_000_100,
        )
        .expect("freshness action schedules");

    let error = recovered
        .service_turn(observed_ns)
        .expect_err("one nanosecond beyond the due-age policy fails closed");
    assert_eq!(
        error.saturation_action(),
        Some(SaturationAction::SuppressQuoteAndCancelOwned)
    );
    let effects = drain_effects(&mut recovered);
    assert_eq!(effects.len(), 3);
    let PmProductEffect::DurableRecord(cancel_record) = effects[0] else {
        panic!("aged schedule must journal cancellation before stopping");
    };
    assert_eq!(cancel_record.kind(), PmDurableRecordKind::CancelIntent);
    assert_eq!(cancel_record.client_order(), Some(client_order));
    let PmProductEffect::FailClosedHaltOrCancel(cancel) = effects[1] else {
        panic!("owned cancel projection must precede the final halt");
    };
    assert_eq!(cancel.reason(), PmControlReason::SchedulerOverload);
    assert_eq!(
        cancel.cancel_intent(),
        Some((client_order, PmCancelIntentReason::SafetyHalt))
    );
    let PmProductEffect::FailClosedHaltOrCancel(halt) = effects[2] else {
        panic!("final scheduler halt must be last");
    };
    assert_eq!(halt.reason(), PmControlReason::SchedulerOverload);
    assert_eq!(halt.cancel_intent(), None);
    assert_eq!(recovered.halt(), Some(PmControlReason::SchedulerOverload));

    let metrics = recovered
        .scheduler_metrics(observed_ns)
        .expect("consumed aged schedule remains observable");
    assert_eq!(
        metrics
            .lane(PmLaneKind::Scheduled)
            .expect("scheduled lane")
            .queue()
            .depth(),
        0
    );
    assert_eq!(
        metrics
            .fail_closed()
            .transitions(SaturationAction::SuppressQuoteAndCancelOwned),
        1
    );
    assert!(metrics.fail_closed().cancel_owned_required());

    recovered
        .service_turn(observed_ns + 1)
        .expect("the consumed aged slot cannot fire twice");
    assert!(drain_effects(&mut recovered).is_empty());

    Box::pin(wait_for_persistence(&mut recovered, 31, observed_ns + 2)).await;
    recovered
        .service_turn(observed_ns + 3)
        .expect("the aged cancel approval remains current after durability");
    let prepared = drain_effects(&mut recovered)
        .into_iter()
        .find_map(|effect| match effect {
            PmProductEffect::CancelOwned(cancel) => Some(cancel),
            _ => None,
        })
        .expect("durability prepares the recovered owned cancel after halt");
    assert_eq!(prepared.client_order(), client_order);
    assert_eq!(
        prepared.stage(),
        PmEffectDispatchStage::PreparedAfterDurability
    );

    let _ = Box::pin((*recovered).shutdown()).await;
}

#[derive(Debug, Clone, Copy)]
enum LiveDependencyFaultCase {
    PolymarketBookStale,
    OkxReferenceStale,
    PolymarketUnavailable,
    OkxUnavailable,
    PrivateDisconnected,
}

#[test]
fn stale_or_unavailable_live_dependencies_suppress_place_but_preserve_exact_cancel() {
    run_product_test(live_dependency_fault_matrix_test);
}

async fn live_dependency_fault_matrix_test() {
    for (ordinal, case) in [
        LiveDependencyFaultCase::PolymarketBookStale,
        LiveDependencyFaultCase::OkxReferenceStale,
        LiveDependencyFaultCase::PolymarketUnavailable,
        LiveDependencyFaultCase::OkxUnavailable,
        LiveDependencyFaultCase::PrivateDisconnected,
    ]
    .into_iter()
    .enumerate()
    {
        let directory = tempfile::tempdir().expect("temporary dependency-fault directory");
        let mut run = Box::new(
            Box::pin(start_reached_overload_product(
                directory
                    .path()
                    .join(format!("dependency-{ordinal}-capture.jsonl")),
                directory
                    .path()
                    .join(format!("dependency-{ordinal}-journal.jsonl")),
            ))
            .await
            .expect("dependency-fault product starts"),
        );
        Box::pin(prepare_reached_overload_product(&mut run))
            .await
            .expect("dependency-fault product becomes ready");
        drain_effects(&mut run);

        let client_order = Box::pin(place_live_quote(&mut run)).await;
        let venue_order = PmVenueOrderKey::new(
            client_order.account(),
            PmVenueOrderId::new("phase6-recovered-stop").expect("fixed accepted venue order"),
        );
        let quote_intents_before = run.mutation_counters().quote_intents();
        let fault_ns = match case {
            // The fixed PM book was observed at 103 and the OKX reference at
            // 104. At these two exact clocks only the named dependency wins
            // the coordinator's ordered freshness classification.
            LiveDependencyFaultCase::PolymarketBookStale => 1_000_000_104,
            LiveDependencyFaultCase::OkxReferenceStale => 1_000_000_105,
            LiveDependencyFaultCase::PolymarketUnavailable
            | LiveDependencyFaultCase::OkxUnavailable
            | LiveDependencyFaultCase::PrivateDisconnected => 2_000 + ordinal as u64 * 100,
        };

        match case {
            LiveDependencyFaultCase::PolymarketBookStale
            | LiveDependencyFaultCase::OkxReferenceStale => run
                .schedule(
                    PmOrderSide::Buy,
                    PmScheduledActionKind::QuoteEvaluation,
                    fault_ns,
                    fault_ns,
                    1_700_000_000_000,
                )
                .expect("stale-dependency quote check schedules"),
            LiveDependencyFaultCase::PolymarketUnavailable => {
                let fault = run
                    .public_ingress()
                    .record_pm_disconnected(1_700_000_000_000_000_000 + fault_ns, fault_ns)
                    .await
                    .expect("PM disconnect produces authenticated unavailability");
                assert_eq!(
                    fault,
                    reap_polymarket_adapter::PmPublicSessionFault::Disconnect
                );
            }
            LiveDependencyFaultCase::OkxUnavailable => {
                let fault = run
                    .public_ingress()
                    .record_okx_disconnected(1_700_000_000_000_000_000 + fault_ns, fault_ns)
                    .await
                    .expect("OKX disconnect produces authenticated unavailability");
                assert_eq!(
                    fault,
                    reap_okx_public_source::OkxPublicSessionFault::Disconnect
                );
            }
            LiveDependencyFaultCase::PrivateDisconnected => run
                .mark_private_fixture_unavailable(
                    completion(1, 60 + ordinal as u64, None, fault_ns),
                    PmPrivateExternalIngressFault::new(
                        PmPrivateExternalIngressLane::PrivateLifecycle,
                        PmPrivateExternalIngressFailure::Service,
                    ),
                )
                .expect("private disconnect reaches the product lane"),
        }

        let mut fault_effects = service_and_collect(&mut run, fault_ns, 4);
        if !matches!(
            case,
            LiveDependencyFaultCase::PolymarketBookStale
                | LiveDependencyFaultCase::OkxReferenceStale
        ) {
            let quote_check_ns = fault_ns + 10;
            run.schedule(
                PmOrderSide::Buy,
                PmScheduledActionKind::QuoteEvaluation,
                quote_check_ns,
                quote_check_ns,
                1_700_000_000_001,
            )
            .expect("unavailable-dependency quote check schedules");
            fault_effects.extend(service_and_collect(&mut run, quote_check_ns, 2));
        }

        assert!(
            fault_effects
                .iter()
                .all(|effect| !matches!(effect, PmProductEffect::PlaceGtcPostOnly(_))),
            "{case:?} admitted a new place effect"
        );
        assert_eq!(
            run.mutation_counters().quote_intents(),
            quote_intents_before,
            "{case:?} admitted a new durable quote intent"
        );
        let cancel_records = fault_effects
            .iter()
            .filter(|effect| {
                matches!(
                    effect,
                    PmProductEffect::DurableRecord(record)
                        if record.kind() == PmDurableRecordKind::CancelIntent
                            && record.client_order() == Some(client_order)
                )
            })
            .count();
        assert_eq!(
            cancel_records, 1,
            "{case:?} did not preserve exactly one durable cancel intent"
        );

        Box::pin(wait_for_persistence(
            &mut run,
            80 + ordinal as u64,
            fault_ns + 20,
        ))
        .await;
        let prepared = service_and_collect(&mut run, fault_ns + 21, 2)
            .into_iter()
            .find_map(|effect| match effect {
                PmProductEffect::CancelOwned(cancel) => Some(cancel),
                _ => None,
            })
            .unwrap_or_else(|| panic!("{case:?} did not release its durable exact cancel"));
        assert_eq!(prepared.client_order(), client_order);
        assert_eq!(prepared.venue_order(), venue_order);
        assert_eq!(
            prepared.stage(),
            PmEffectDispatchStage::PreparedAfterDurability
        );

        let _ = Box::pin((*run).shutdown()).await;
    }
}

#[test]
fn aged_schedule_representation_failure_is_consumed_after_one_attempt() {
    run_product_test(aged_schedule_single_attempt_failure_test);
}

async fn aged_schedule_single_attempt_failure_test() {
    let directory = tempfile::tempdir().expect("temporary single-attempt directory");
    let mut run = Box::new(
        Box::pin(start_reached_overload_product(
            directory.path().join("single-attempt-capture.jsonl"),
            directory.path().join("single-attempt-journal.jsonl"),
        ))
        .await
        .expect("single-attempt product starts"),
    );
    Box::pin(prepare_reached_overload_product(&mut run))
        .await
        .expect("single-attempt product becomes ready");
    drain_effects(&mut run);

    let maximum_age_ns = PmLanePolicy::for_lane(PmLaneKind::Scheduled)
        .maximum_age_ns()
        .expect("scheduled age policy");
    let deadline_ns = 20_000;
    let schedule_observed_ns = deadline_ns + maximum_age_ns + 1;
    run.schedule(
        PmOrderSide::Buy,
        PmScheduledActionKind::Freshness,
        deadline_ns,
        2_000,
        1_700_000_000_200,
    )
    .expect("freshness action schedules");
    let reserved_before = run.reserved_capacity_bytes();

    assert_eq!(
        run.phase6_enact_next_schedule_failure_with_observer_clock(
            schedule_observed_ns,
            schedule_observed_ns + 1,
        ),
        Some(SaturationAction::SuppressQuoteAndCancelOwned)
    );
    assert_eq!(run.halt(), Some(PmControlReason::SchedulerOverload));
    assert_eq!(run.mutation_halt(), Some(PmMutationHalt::InternalInvariant));
    let first_effects = drain_effects(&mut run);
    assert_eq!(first_effects.len(), 1);
    assert!(matches!(
        first_effects[0],
        PmProductEffect::FailClosedHaltOrCancel(effect)
            if effect.reason() == PmControlReason::SchedulerOverload
                && effect.cancel_intent().is_none()
    ));

    let before_second = run
        .scheduler_metrics(schedule_observed_ns + 1)
        .expect("failed representation still consumes the aged slot");
    let scheduled_before = before_second
        .lane(PmLaneKind::Scheduled)
        .expect("scheduled lane");
    assert_eq!(scheduled_before.queue().depth(), 0);
    assert_eq!(scheduled_before.queue().high_water(), 1);
    assert_eq!(
        before_second
            .fail_closed()
            .transitions(SaturationAction::SuppressQuoteAndCancelOwned),
        1
    );
    let mutation_before = run.mutation_counters();

    run.service_turn(schedule_observed_ns + 2)
        .expect("global halt leaves no aged schedule to emit again");
    assert!(drain_effects(&mut run).is_empty());
    let after_second = run
        .scheduler_metrics(schedule_observed_ns + 2)
        .expect("second projection remains stable");
    let scheduled_after = after_second
        .lane(PmLaneKind::Scheduled)
        .expect("scheduled lane");
    assert_eq!(scheduled_after.queue(), scheduled_before.queue());
    assert_eq!(
        after_second
            .fail_closed()
            .transitions(SaturationAction::SuppressQuoteAndCancelOwned),
        1
    );
    assert_eq!(run.mutation_counters(), mutation_before);
    assert_eq!(run.reserved_capacity_bytes(), reserved_before);
    assert_eq!(run.mutation_halt(), Some(PmMutationHalt::InternalInvariant));
    assert_eq!(run.halt(), Some(PmControlReason::SchedulerOverload));

    let _ = Box::pin((*run).shutdown()).await;
}

fn run_product_test<F, Fut>(test: F)
where
    F: FnOnce() -> Fut,
    Fut: Future<Output = ()>,
{
    tokio::runtime::Builder::new_current_thread()
        .enable_all()
        .build()
        .expect("product control runtime")
        .block_on(test());
}

fn run_minimum_stack_product_test<F, Fut>(test: F)
where
    F: FnOnce() -> Fut + Send + 'static,
    Fut: Future<Output = ()> + 'static,
{
    let handle = std::thread::Builder::new()
        .name("minimum-product-stack".to_string())
        .stack_size(MINIMUM_PRODUCT_STACK_BYTES)
        .spawn(move || {
            tokio::runtime::Builder::new_current_thread()
                .enable_all()
                .build()
                .expect("minimum-stack product runtime")
                .block_on(test());
        })
        .expect("minimum-stack product test thread");
    if let Err(panic) = handle.join() {
        std::panic::resume_unwind(panic);
    }
}

async fn place_live_quote<M: PmQuoteModel>(
    run: &mut PmProductRun<M>,
) -> reap_pm_core::PmClientOrderKey {
    run.schedule(
        PmOrderSide::Buy,
        PmScheduledActionKind::QuoteEvaluation,
        1_450,
        1_400,
        1_700_000_000_000,
    )
    .expect("quote evaluation schedules");
    run.service_turn(1_450)
        .expect("quote evaluation reaches the owner");
    let client_order = drain_effects(run)
        .into_iter()
        .find_map(|effect| match effect {
            PmProductEffect::DurableRecord(record)
                if record.kind() == PmDurableRecordKind::QuoteIntent =>
            {
                record.client_order()
            }
            _ => None,
        })
        .expect("quote intent projection");

    Box::pin(wait_for_persistence(run, 10, 1_451)).await;
    run.service_turn(1_452)
        .expect("quote durability reaches the owner");
    let prepared = drain_effects(run)
        .into_iter()
        .find_map(|effect| match effect {
            PmProductEffect::PlaceGtcPostOnly(quote) => Some(quote),
            _ => None,
        })
        .expect("durability prepares a fake quote");
    assert_eq!(prepared.client_order(), client_order);
    assert_eq!(
        prepared.stage(),
        PmEffectDispatchStage::PreparedAfterDurability
    );

    let venue_order = PmVenueOrderKey::new(
        client_order.account(),
        PmVenueOrderId::new("phase6-recovered-stop").expect("fixed venue order"),
    );
    run.execute_prepared_quote_fixture(
        completion(1, 11, None, 1_453),
        PmFakePlaceScript::acknowledged(venue_order, Box::new([]))
            .expect("valid fake acknowledgement"),
        1_453,
    )
    .expect("prepared quote executes");
    run.service_turn(1_454)
        .expect("fake place result reaches the owner");
    drain_effects(run);

    Box::pin(wait_for_persistence(run, 12, 1_455)).await;
    run.service_turn(1_456)
        .expect("place result durability reaches the owner");
    drain_effects(run);
    client_order
}

async fn place_acknowledgement_unknown_quote<M: PmQuoteModel>(
    run: &mut PmProductRun<M>,
) -> Vec<PmProductEffect> {
    run.schedule(
        PmOrderSide::Buy,
        PmScheduledActionKind::QuoteEvaluation,
        1_450,
        1_400,
        1_700_000_000_000,
    )
    .expect("quote evaluation schedules");
    run.service_turn(1_450)
        .expect("quote evaluation reaches the owner");
    drain_effects(run);

    Box::pin(wait_for_persistence(run, 40, 1_451)).await;
    run.service_turn(1_452)
        .expect("quote durability reaches the owner");
    let prepared = drain_effects(run)
        .into_iter()
        .find_map(|effect| match effect {
            PmProductEffect::PlaceGtcPostOnly(quote) => Some(quote),
            _ => None,
        })
        .expect("durability prepares a fake quote");
    assert_eq!(
        prepared.stage(),
        PmEffectDispatchStage::PreparedAfterDurability
    );

    run.execute_prepared_quote_fixture(
        completion(1, 41, None, 1_453),
        PmFakePlaceScript::acknowledgement_unknown(),
        1_453,
    )
    .expect("acknowledgement-unknown fixture executes");
    run.service_turn(1_454)
        .expect("ambiguous fake place result reaches the owner");
    drain_effects(run)
}

async fn wait_for_persistence(
    run: &mut PmProductRun<impl PmQuoteModel>,
    sequence: u64,
    monotonic_ns: u64,
) {
    let deadline = tokio::time::Instant::now() + Duration::from_secs(3);
    loop {
        match run
            .poll_persistence_fixture(completion(1, sequence, None, monotonic_ns), monotonic_ns)
        {
            Ok(true) => return,
            Ok(false) if tokio::time::Instant::now() < deadline => tokio::task::yield_now().await,
            Ok(false) => panic!("timed out waiting for durable acknowledgement"),
            Err(error) => panic!("durable acknowledgement admission failed: {error}"),
        }
    }
}

fn drain_effects(run: &mut PmProductRun<impl PmQuoteModel>) -> Vec<PmProductEffect> {
    let mut effects = Vec::new();
    while let Some(effect) = run.pop_effect() {
        effects.push(effect);
    }
    effects
}

fn service_and_collect(
    run: &mut PmProductRun<impl PmQuoteModel>,
    monotonic_ns: u64,
    turns: usize,
) -> Vec<PmProductEffect> {
    let mut effects = Vec::new();
    for offset in 0..turns {
        run.service_turn(monotonic_ns + offset as u64)
            .expect("dependency-fault service turn succeeds");
        effects.extend(drain_effects(run));
    }
    effects
}

fn reconciliation_refresh_count(effects: &[PmProductEffect]) -> usize {
    effects
        .iter()
        .filter(|effect| matches!(effect, PmProductEffect::ReconciliationRefresh(_)))
        .count()
}

fn reconciliation_refresh_kinds(effects: &[PmProductEffect]) -> Vec<PmRefreshEffectKind> {
    effects
        .iter()
        .filter_map(|effect| match effect {
            PmProductEffect::ReconciliationRefresh(refresh) => Some(refresh.kind()),
            _ => None,
        })
        .collect()
}
