use std::future::Future;
use std::time::Duration;

use reap_pm_core::{
    PmFillQueryCursor, PmOrderSide, PmPositionAvailability, PmSignedUnits, PmVenueOrderId,
    PmVenueOrderKey, U256,
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
    PmCancelIntentReason, PmControlReason, PmDurableRecordKind, PmFakeEffectStage, PmLaneKind,
    PmLanePolicy, PmMutationHalt, PmOpenOrdersFixtureInput, PmOrderDetailFixtureInput,
    PmProductEffect, PmProductRun, PmReconciliationFixtureInput, PmScheduledActionKind,
    SaturationAction,
};

const MINIMUM_PRODUCT_STACK_BYTES: usize = 2 * 1024 * 1024;

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
fn private_reconnect_refresh_requires_one_applied_complete_pair() {
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
        1,
        "the canonical reconnect ticket emits one complete-reconciliation request"
    );
    let admitted = run.refresh_obligation_metrics();
    assert_eq!(admitted.canonical_insertions(), 1);
    assert_eq!(admitted.total_pending(), 1);
    assert_eq!(admitted.total_in_flight(), 1);
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
    let retained_total = run.refresh_obligation_metrics().total_pending();

    run.service_turn(1_000_001_454)
        .expect("inclusive age boundary services without retry");
    assert_eq!(reconciliation_refresh_count(&drain_effects(&mut run)), 0);
    let at_boundary = run.refresh_obligation_metrics();
    assert_eq!(at_boundary.oldest_in_flight_age_ns(), 1_000_000_000);
    assert_eq!(at_boundary.retry_effects(), 0);

    run.service_turn(1_000_001_455)
        .expect("one nanosecond beyond the age boundary services");
    assert_eq!(reconciliation_refresh_count(&drain_effects(&mut run)), 1);
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
    let admitted = run.refresh_obligation_metrics();
    assert_eq!(admitted.total_pending(), baseline_total + 1);
    assert_eq!(admitted.total_in_flight(), 1);
    assert_eq!(admitted.ambiguous_order_pending(), 1);
    assert_eq!(admitted.ambiguous_order_in_flight(), 1);

    let ambiguous_client = ambiguity_effects
        .iter()
        .find_map(|effect| match effect {
            PmProductEffect::FakePassiveQuote(quote) => Some(quote.client_order()),
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
    assert!(
        convergence_effects
            .iter()
            .all(|effect| !matches!(effect, PmProductEffect::ReconciliationRefresh(_))),
        "a converged authoritative snapshot must not emit another refresh"
    );
    let converged = run.refresh_obligation_metrics();
    assert_eq!(
        converged.total_pending(),
        unrelated.total_pending(),
        "the complete snapshot clears ambiguity but retains its newly exposed missing-detail obligation"
    );
    assert_eq!(converged.total_in_flight(), 0);
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
            PmProductEffect::FakeCancelOwned(cancel) => Some(cancel),
            _ => None,
        })
        .expect("durability prepares the fake owned cancel after halt");
    assert_eq!(prepared.client_order(), client_order);
    assert_eq!(prepared.stage(), PmFakeEffectStage::PreparedAfterDurability);

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
    let executed = drain_effects(&mut recovered)
        .into_iter()
        .find_map(|effect| match effect {
            PmProductEffect::FakeCancelOwned(cancel) => Some(cancel),
            _ => None,
        })
        .expect("fake cancel execution is projected");
    assert_eq!(executed.client_order(), client_order);
    assert_eq!(executed.stage(), PmFakeEffectStage::ExecutedByFixture);

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
            PmProductEffect::FakeCancelOwned(cancel) => Some(cancel),
            _ => None,
        })
        .expect("durability prepares the recovered owned cancel after halt");
    assert_eq!(prepared.client_order(), client_order);
    assert_eq!(prepared.stage(), PmFakeEffectStage::PreparedAfterDurability);

    let _ = Box::pin((*recovered).shutdown()).await;
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
            PmProductEffect::FakePassiveQuote(quote) => Some(quote),
            _ => None,
        })
        .expect("durability prepares a fake quote");
    assert_eq!(prepared.client_order(), client_order);
    assert_eq!(prepared.stage(), PmFakeEffectStage::PreparedAfterDurability);

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
            PmProductEffect::FakePassiveQuote(quote) => Some(quote),
            _ => None,
        })
        .expect("durability prepares a fake quote");
    assert_eq!(prepared.stage(), PmFakeEffectStage::PreparedAfterDurability);

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

fn reconciliation_refresh_count(effects: &[PmProductEffect]) -> usize {
    effects
        .iter()
        .filter(|effect| matches!(effect, PmProductEffect::ReconciliationRefresh(_)))
        .count()
}
