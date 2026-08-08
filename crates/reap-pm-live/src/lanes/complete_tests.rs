use reap_pm_core::{
    ConnectionEpoch, EventOrdering, EvmAddress, IngressSequence, PmAccountHandle, PmAccountScope,
    PmChainId, PmClientOrderId, PmClientOrderKey, PmConditionId, PmConnectionId, PmEnvironmentId,
    PmFunderId, PmInstrumentHandle, PmMarketEvent, PmMarketHandle, PmOrderSide, PmPrice,
    PmQuantity, PmSignerId, PmSourceHandle, PmTokenHandle, PmVenueOrderId, PmVenueOrderKey,
    ReceivedEventClock, U256, exact_order_amounts,
};
use reap_pm_state::{
    PmExactReservation, PmOwnedCancelRequestApply, PmOwnedIntentId, PmOwnedOrderLifecycle,
    PmOwnedQuoteAdmission, PmOwnedQuoteIntent, PmOwnedQuoteSlotKey, PmOwnedSubmitResult,
    PmPrivateExternalIngressFailure, PmPrivateExternalIngressFault, PmPrivateExternalIngressLane,
    PmRiskHaltScope,
};

use super::*;
use crate::coordinator::{PmLiveCancelCompletion, PmLivePlaceCompletion};
use crate::journal::{
    PmJournalAuthenticatedCancelResultV1, PmJournalAuthenticatedClassificationV1,
    PmJournalAuthenticatedPlaceResultV1,
};
use crate::private_monitor::{PmLiveHttpQueryFailure, PmLiveOccurrenceIssuer};
use crate::public_routes::{OkxPublicUnavailable, PmPublicUnavailable};
use crate::schedule::{
    PmQuoteScheduleRole, PmScheduleError, PmScheduledActionKey, PmScheduledActionKind,
};

fn instrument() -> PmInstrumentHandle {
    PmInstrumentHandle::new(
        PmMarketHandle::from_ordinal(3),
        PmTokenHandle::from_ordinal(5),
    )
}

fn account_scope(account: u16) -> PmAccountScope {
    PmAccountScope::new(
        PmEnvironmentId::new("complete-lane-test").expect("environment"),
        PmChainId::new(137).expect("chain"),
        PmSignerId::new(EvmAddress::from_bytes([1; 20]).expect("signer")),
        PmFunderId::new(EvmAddress::from_bytes([2; 20]).expect("funder")),
        PmAccountHandle::from_ordinal(account),
    )
}

fn ordering(sequence: u64) -> EventOrdering {
    EventOrdering::new(
        ConnectionEpoch::new(1),
        None,
        None,
        None,
        IngressSequence::new(sequence),
    )
    .expect("ordering")
}

fn clock(receive_ns: u64) -> ReceivedEventClock {
    ReceivedEventClock::new(None, receive_ns + 10_000, receive_ns).expect("clock")
}

fn connection() -> PmConnectionId {
    PmConnectionId::new("complete-lane-test").expect("connection")
}

fn internal_ingress(sequence: u64, receive_ns: u64) -> PmCompleteIngress {
    PmCompleteIngress::internal(
        PmSourceHandle::from_ordinal(9),
        connection(),
        ordering(sequence),
        clock(receive_ns),
    )
}

fn account_ingress(sequence: u64, receive_ns: u64) -> PmCompleteIngress {
    PmCompleteIngress::product(
        reap_pm_core::PmProductSource::polymarket_account(
            PmSourceHandle::from_ordinal(7),
            PmAccountHandle::from_ordinal(1),
        ),
        connection(),
        ordering(sequence),
        clock(receive_ns),
    )
}

fn private_unavailable() -> PmPrivateInput {
    PmPrivateInput::ConnectionUnavailable(PmPrivateExternalIngressFault::new(
        PmPrivateExternalIngressLane::Reconnect,
        PmPrivateExternalIngressFailure::Service,
    ))
}

fn authenticated_place_completion() -> PmLivePlaceCompletion {
    let config = crate::evidence::connectivity_config();
    let client_order = PmClientOrderKey::new(
        config.account().account_scope().handle(),
        PmClientOrderId::from_bytes([0x21; 16]).expect("place client order"),
    );
    let result = PmJournalAuthenticatedPlaceResultV1::new(
        1,
        2,
        3,
        1,
        client_order,
        config.account().instrument_id(),
        [0x41; 32],
        [0x31; 32],
        None,
        None,
        PmJournalAuthenticatedClassificationV1::DefinitelyNotDispatched,
    )
    .expect("shape-valid lane-only place bridge");
    PmLivePlaceCompletion::from_journal_for_scheduler_test(result)
}

fn authenticated_cancel_completion() -> PmLiveCancelCompletion {
    let config = crate::evidence::connectivity_config();
    let client_order = PmClientOrderKey::new(
        config.account().account_scope().handle(),
        PmClientOrderId::from_bytes([0x22; 16]).expect("cancel client order"),
    );
    let venue_order = PmVenueOrderKey::new(
        config.account().account_scope().handle(),
        PmVenueOrderId::new("0x3333333333333333333333333333333333333333333333333333333333333333")
            .expect("cancel venue order"),
    );
    let side = PmOrderSide::Buy;
    let price = PmPrice::parse_decimal("0.50").expect("cancel price");
    let quantity = PmQuantity::parse_decimal("5").expect("cancel quantity");
    let maker = exact_order_amounts(side, price, quantity)
        .expect("cancel exact amounts")
        .maker();
    let quote = PmOwnedQuoteIntent::new(
        PmOwnedIntentId::new(1).expect("cancel intent"),
        PmOwnedQuoteSlotKey::new(
            config.account().account_scope(),
            config.public().instrument(),
            side,
        ),
        client_order,
        price,
        quantity,
        PmExactReservation::policy_approved(maker, U256::ZERO).expect("cancel reservation"),
    )
    .expect("cancel quote intent");
    let mut lifecycle = PmOwnedOrderLifecycle::new(
        config.account().account_scope(),
        config.public().instrument(),
    );
    assert!(matches!(
        lifecycle.admit_quote(quote),
        Ok(PmOwnedQuoteAdmission::Admitted(key)) if key == client_order
    ));
    lifecycle
        .apply_submit_result(client_order, PmOwnedSubmitResult::Accepted(venue_order))
        .expect("accept cancel test order");
    let intent = match lifecycle
        .request_cancel(client_order)
        .expect("request cancel test order")
    {
        PmOwnedCancelRequestApply::Issued(intent) => intent,
        other => panic!("expected issued cancel intent, got {other:?}"),
    };
    let result = PmJournalAuthenticatedCancelResultV1::new(
        4,
        5,
        6,
        2,
        client_order,
        config.account().instrument_id(),
        venue_order,
        [0x42; 32],
        [0x33; 32],
        Some([0x33; 32]),
        PmJournalAuthenticatedClassificationV1::Accepted,
    )
    .expect("shape-valid lane-only cancel bridge");
    PmLiveCancelCompletion::from_journal_for_scheduler_test(intent, result)
}

#[derive(Default)]
struct TraceConsumer {
    trace: Vec<&'static str>,
    block_below_persistence: bool,
}

impl PmCompleteLaneService for TraceConsumer {
    fn block_below_persistence(&self) -> bool {
        self.block_below_persistence
    }

    fn on_critical(&mut self, item: PmCompleteServiced<PmCriticalInput>) {
        self.trace.push(match item.into_value() {
            PmCriticalInput::Stop(_) => "critical-stop",
            PmCriticalInput::ScopedHalt(_) => "critical-halt",
            PmCriticalInput::FakeCancelResult(_) => "critical-cancel-result",
            PmCriticalInput::FakePlaceResult(_) => "critical-place-result",
            PmCriticalInput::LiveCancelResult(_) => "critical-live-cancel-result",
            PmCriticalInput::LivePlaceResult(_) => "critical-live-place-result",
        });
    }

    fn on_persistence(&mut self, _item: PmCompleteServiced<PmPersistenceInput>) {
        self.trace.push("persistence");
    }

    fn on_private(&mut self, _item: PmCompleteServiced<PmPrivateInput>) {
        self.trace.push("private");
    }

    fn on_scheduled(&mut self, _item: crate::schedule::PmDueScheduledAction) {
        self.trace.push("scheduled");
    }

    fn on_reconciliation(&mut self, _item: PmCompleteServiced<PmReconciliationInput>) {
        self.trace.push("reconciliation");
    }

    fn on_telemetry(&mut self, item: PmCompleteServiced<PmTelemetryInput>) {
        let _ = item.into_value().value();
        self.trace.push("telemetry");
    }
}

#[test]
fn authenticated_http_reconciliation_remains_counted_below_persistence_barrier() {
    let mut issuer = PmLiveOccurrenceIssuer::new(
        PmConditionId::parse("0xaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa")
            .expect("fixed condition"),
    );
    issuer
        .start_for_test(ConnectionEpoch::new(1), clock(10))
        .expect("active authenticated read epoch");
    let ticket = issuer
        .begin_account_query(11)
        .expect("account query ticket");
    let failure = issuer
        .fail_account_query(ticket, clock(12), PmLiveHttpQueryFailure::Transport)
        .expect("purpose-bound completed HTTP failure")
        .into_dependency_failure();

    let mut owner = PmCompleteInputLanes::for_instrument(instrument());
    owner
        .enqueue_reconciliation(
            account_ingress(2, 12),
            PmReconciliationInput::DependencyUnavailable(failure),
        )
        .expect("completed authenticated HTTP result enters reconciliation");
    let mut consumer = TraceConsumer {
        block_below_persistence: true,
        ..TraceConsumer::default()
    };
    let blocked = owner
        .service_turn(12, &mut consumer)
        .expect("persistence barrier is a bounded idle turn");
    assert_eq!(blocked.for_lane(PmLaneKind::Reconciliation), Some(0));
    assert_eq!(owner.private_and_reconciliation_depths(), (0, 1));
    assert_eq!(owner.read_input_lane_depths(), (0, 0, 1));
    assert!(consumer.trace.is_empty());

    consumer.block_below_persistence = false;
    let drained = owner
        .service_turn(13, &mut consumer)
        .expect("later actor-only turn drains the accepted HTTP result");
    assert_eq!(drained.for_lane(PmLaneKind::Reconciliation), Some(1));
    assert_eq!(owner.private_and_reconciliation_depths(), (0, 0));
    assert_eq!(owner.read_input_lane_depths(), (0, 0, 0));
    assert_eq!(consumer.trace, ["reconciliation"]);
}

impl PmPublicLaneService for TraceConsumer {
    fn on_pm_public_unavailable(&mut self, _item: ServicedLaneItem<PmPublicUnavailable>) {
        self.trace.push("public");
    }

    fn on_okx_public_unavailable(&mut self, _item: ServicedLaneItem<OkxPublicUnavailable>) {
        self.trace.push("public");
    }

    fn on_market(&mut self, _item: ServicedLaneItem<PmMarketEvent>) {
        self.trace.push("public");
    }

    fn on_book(&mut self, _item: ServicedLaneItem<reap_pm_core::PmBookEvent>) {
        self.trace.push("public");
    }

    fn on_reference(&mut self, _item: ServicedLaneItem<reap_pm_core::OkxReferenceEvent>) {
        self.trace.push("public");
    }
}

#[test]
fn complete_owner_uses_frozen_rank_and_variant_order_without_a_second_public_owner() {
    let mut owner = PmCompleteInputLanes::for_instrument(instrument());
    let same_ingress = internal_ingress(1, 100);
    owner
        .enqueue_critical(
            same_ingress,
            PmCriticalInput::ScopedHalt(
                PmScopedHalt::new(PmRiskHaltScope::Account).expect("scoped halt"),
            ),
        )
        .expect("halt");
    owner
        .enqueue_critical(
            same_ingress,
            PmCriticalInput::Stop(PmStopControl::GlobalStop),
        )
        .expect("stop");
    owner
        .enqueue_private(account_ingress(1, 100), private_unavailable())
        .expect("private");
    owner
        .enqueue_telemetry(
            internal_ingress(2, 100),
            PmTelemetryInput::new(PmTelemetryKind::Metric, 42),
        )
        .expect("telemetry");

    let mut consumer = TraceConsumer::default();
    let counts = owner
        .service_turn(100, &mut consumer)
        .expect("service turn");
    assert_eq!(
        consumer.trace,
        ["critical-stop", "critical-halt", "private", "telemetry"]
    );
    assert_eq!(counts.for_lane(PmLaneKind::Critical), Some(2));
    assert_eq!(counts.for_lane(PmLaneKind::Private), Some(1));
    assert_eq!(counts.for_lane(PmLaneKind::Telemetry), Some(1));
    assert_eq!(counts.total(), 4);
}

#[test]
fn authenticated_place_and_cancel_use_the_internal_critical_source_and_service_once() {
    let mut owner = PmCompleteInputLanes::for_instrument(instrument());
    let same_ingress = internal_ingress(7, 100);
    owner
        .enqueue_critical(
            same_ingress,
            PmCriticalInput::LivePlaceResult(Box::new(authenticated_place_completion())),
        )
        .expect("authenticated place completion enters critical lane");
    owner
        .enqueue_critical(
            same_ingress,
            PmCriticalInput::LiveCancelResult(authenticated_cancel_completion()),
        )
        .expect("authenticated cancel completion enters critical lane");

    let queued = owner.metrics(100).expect("queued critical metrics");
    let critical = queued.lane(PmLaneKind::Critical).expect("critical lane");
    assert_eq!(critical.policy().capacity(), 512);
    assert_eq!(critical.queue().depth(), 2);
    assert_eq!(critical.queue().rejected_full(), 0);
    assert_eq!(
        queued
            .fail_closed()
            .transitions(SaturationAction::GlobalStop),
        0
    );

    let mut consumer = TraceConsumer::default();
    let counts = owner
        .service_turn(100, &mut consumer)
        .expect("service authenticated completions");
    assert_eq!(
        consumer.trace,
        ["critical-live-cancel-result", "critical-live-place-result"]
    );
    assert_eq!(counts.for_lane(PmLaneKind::Critical), Some(2));
    assert_eq!(counts.total(), 2);
    assert_eq!(
        owner
            .metrics(100)
            .expect("drained critical metrics")
            .lane(PmLaneKind::Critical)
            .expect("critical lane")
            .queue()
            .depth(),
        0
    );
}

#[test]
fn critical_lane_rejects_product_account_source_before_capacity_admission() {
    let mut owner = PmCompleteInputLanes::for_instrument(instrument());
    let rejected = owner.enqueue_critical(
        account_ingress(7, 100),
        PmCriticalInput::LivePlaceResult(Box::new(authenticated_place_completion())),
    );
    assert!(matches!(
        rejected,
        Err(PmCompleteLaneEnqueueError::WrongSource {
            expected: PmCompleteSourceKind::InternalSignal,
            received: PmCompleteSourceKind::PolymarketAccount,
            ..
        })
    ));

    let metrics = owner.metrics(100).expect("wrong-source metrics");
    let critical = metrics.lane(PmLaneKind::Critical).expect("critical lane");
    assert_eq!(critical.queue().depth(), 0);
    assert_eq!(critical.queue().high_water(), 0);
    assert_eq!(critical.queue().rejected_full(), 0);
    assert_eq!(
        metrics
            .fail_closed()
            .transitions(SaturationAction::GlobalStop),
        0
    );
}

#[test]
fn authenticated_completion_admissions_derive_internal_ingress_from_the_sealed_occurrence() {
    let source = include_str!("../coordinator/product/service.rs");
    for method in ["admit_place", "admit_cancel"] {
        let body = source
            .split_once(&format!("fn {method}"))
            .expect("authenticated completion admission method")
            .1
            .split_once("\n    }")
            .expect("bounded authenticated completion method")
            .0;
        assert!(body.contains("internal_ingress(&occurrence)"));
        assert!(!body.contains("account_ingress(&occurrence)"));
    }
}

#[test]
fn critical_capacity_rejects_513th_and_latches_one_global_stop() {
    let mut owner = PmCompleteInputLanes::for_instrument(instrument());
    let capacity = PmLanePolicy::for_lane(PmLaneKind::Critical).capacity();
    for sequence in 1..=capacity {
        let sequence = u64::try_from(sequence).expect("bounded");
        owner
            .enqueue_critical(
                internal_ingress(sequence, sequence),
                PmCriticalInput::Stop(PmStopControl::Shutdown),
            )
            .expect("within capacity");
    }
    let rejected = owner.enqueue_critical(
        internal_ingress(513, 513),
        PmCriticalInput::Stop(PmStopControl::Shutdown),
    );
    assert!(matches!(
        rejected,
        Err(PmCompleteLaneEnqueueError::Full {
            action: SaturationAction::GlobalStop,
            ..
        })
    ));

    let metrics = owner.metrics(513).expect("metrics");
    let critical = metrics.lane(PmLaneKind::Critical).expect("critical");
    assert_eq!(critical.queue().depth(), 512);
    assert_eq!(critical.queue().high_water(), 512);
    assert_eq!(critical.queue().rejected_full(), 1);
    assert_eq!(
        metrics
            .fail_closed()
            .transitions(SaturationAction::GlobalStop),
        1
    );
    assert!(metrics.fail_closed().global_stopped());
    assert!(metrics.fail_closed().fake_dispatch_suppressed());
}

#[test]
fn telemetry_is_the_only_complete_input_lane_that_coalesces() {
    let mut owner = PmCompleteInputLanes::for_instrument(instrument());
    for sequence in 1..=129_u64 {
        owner
            .enqueue_telemetry(
                internal_ingress(sequence, sequence),
                PmTelemetryInput::new(PmTelemetryKind::Health, sequence),
            )
            .expect("telemetry coalesces at capacity");
    }
    let metrics = owner.metrics(129).expect("metrics");
    let telemetry = metrics.lane(PmLaneKind::Telemetry).expect("telemetry");
    assert_eq!(telemetry.queue().depth(), 128);
    assert_eq!(telemetry.queue().high_water(), 128);
    assert_eq!(telemetry.queue().coalesced(), 1);
    assert_eq!(telemetry.queue().rejected_full(), 0);
    assert_eq!(
        metrics.fail_closed(),
        PmCompleteFailClosedMetrics::default()
    );
}

#[test]
fn every_state_bearing_nonpublic_queue_faults_one_nanosecond_past_its_age_limit() {
    for lane_kind in [
        PmLaneKind::Critical,
        PmLaneKind::Persistence,
        PmLaneKind::Private,
        PmLaneKind::Reconciliation,
    ] {
        let mut lane = PmCompleteLane::<u64>::new(lane_kind);
        let ingress = if matches!(lane_kind, PmLaneKind::Private | PmLaneKind::Reconciliation) {
            account_ingress(1, 1)
        } else {
            internal_ingress(1, 1)
        };
        let expected_source =
            if matches!(lane_kind, PmLaneKind::Private | PmLaneKind::Reconciliation) {
                PmCompleteSourceKind::PolymarketAccount
            } else {
                PmCompleteSourceKind::InternalSignal
            };
        lane.enqueue(ingress, 7, 0, expected_source).expect("admit");
        let maximum = PmLanePolicy::for_lane(lane_kind)
            .maximum_age_ns()
            .expect("state lane age");
        lane.check_age(1 + maximum).expect("boundary is inclusive");
        let fault = match lane.check_age(1 + maximum + 1) {
            Err(PmCompleteLaneCheckError::Aged(fault)) => fault,
            _ => panic!("one nanosecond over the limit must fault"),
        };
        assert_eq!(fault.lane(), lane_kind);
        assert_eq!(fault.observed_age_ns(), maximum + 1);
        assert_eq!(fault.maximum_age_ns(), maximum);
        assert_eq!(lane.metrics().age_faults(), 1);
        assert_eq!(lane.metrics().queue().depth(), 1);
        let _ = fault.key();
    }
}

#[test]
fn a_later_private_age_episode_faults_again_after_the_first_backlog_drains() {
    let mut owner = PmCompleteInputLanes::for_instrument(instrument());
    let maximum = PmLanePolicy::for_lane(PmLaneKind::Private)
        .maximum_age_ns()
        .expect("private lane age");
    owner
        .enqueue_private(account_ingress(1, 1), private_unavailable())
        .expect("first private occurrence");

    let first_fault_ns = 1 + maximum + 1;
    let mut consumer = TraceConsumer::default();
    let first = owner
        .service_turn(first_fault_ns, &mut consumer)
        .expect_err("the first aged head faults before transfer");
    assert!(matches!(
        first,
        PmCompleteServiceError::Aged(fault)
            if fault.lane() == PmLaneKind::Private
                && fault.action()
                    == SaturationAction::HaltAccountAndRequireReconciliation
    ));
    assert!(consumer.trace.is_empty());

    let drained = owner
        .service_turn(first_fault_ns, &mut consumer)
        .expect("the exact faulted backlog drains on the recovery turn");
    assert_eq!(drained.for_lane(PmLaneKind::Private), Some(1));
    assert_eq!(consumer.trace, ["private"]);

    let second_receive_ns = first_fault_ns + 1;
    owner
        .enqueue_private(account_ingress(2, second_receive_ns), private_unavailable())
        .expect("later private occurrence");
    let second = owner
        .service_turn(second_receive_ns + maximum + 1, &mut consumer)
        .expect_err("a new aged episode must not inherit the old drain permit");
    assert!(matches!(
        second,
        PmCompleteServiceError::Aged(fault)
            if fault.lane() == PmLaneKind::Private
                && fault.action()
                    == SaturationAction::HaltAccountAndRequireReconciliation
    ));
    let metrics = owner
        .metrics(second_receive_ns + maximum + 1)
        .expect("scheduler metrics");
    assert_eq!(
        metrics
            .lane(PmLaneKind::Private)
            .expect("private metrics")
            .age_faults(),
        2
    );
}

#[test]
fn private_inputs_arriving_after_an_age_fault_do_not_inherit_its_drain_authority() {
    let mut owner = PmCompleteInputLanes::for_instrument(instrument());
    let maximum = PmLanePolicy::for_lane(PmLaneKind::Private)
        .maximum_age_ns()
        .expect("private lane age");
    owner
        .enqueue_private(account_ingress(1, 1), private_unavailable())
        .expect("first private occurrence");

    let first_fault_ns = 1 + maximum + 1;
    let mut consumer = TraceConsumer::default();
    assert!(matches!(
        owner.service_turn(first_fault_ns, &mut consumer),
        Err(PmCompleteServiceError::Aged(fault))
            if fault.lane() == PmLaneKind::Private
    ));

    let later_receive_ns = first_fault_ns + 1;
    owner
        .enqueue_private(account_ingress(2, later_receive_ns), private_unavailable())
        .expect("later private occurrence");
    let later_fault_ns = later_receive_ns + maximum + 1;
    let second = owner
        .service_turn(later_fault_ns, &mut consumer)
        .expect_err("only the backlog present at the first fault may drain");
    assert!(matches!(
        second,
        PmCompleteServiceError::Aged(fault)
            if fault.lane() == PmLaneKind::Private
                && fault.action()
                    == SaturationAction::HaltAccountAndRequireReconciliation
    ));
    assert_eq!(consumer.trace, ["private"]);
    let metrics = owner.metrics(later_fault_ns).expect("scheduler metrics");
    assert_eq!(
        metrics
            .lane(PmLaneKind::Private)
            .expect("private metrics")
            .queue()
            .depth(),
        1
    );

    let drained = owner
        .service_turn(later_fault_ns, &mut consumer)
        .expect("the second exact backlog drains after its own fault");
    assert_eq!(drained.for_lane(PmLaneKind::Private), Some(1));
    assert_eq!(consumer.trace, ["private", "private"]);
}

#[test]
fn scheduled_role_materializes_all_four_exact_variant_ranks_and_age_is_fail_closed() {
    assert_eq!(PmScheduledActionKind::CancelOwnedQuote.variant_rank(), 0);
    assert_eq!(
        PmScheduledActionKind::ReconciliationRefresh.variant_rank(),
        1
    );
    assert_eq!(PmScheduledActionKind::Freshness.variant_rank(), 2);
    assert_eq!(PmScheduledActionKind::QuoteEvaluation.variant_rank(), 3);

    let mut schedule = PmQuoteScheduleRole::new(instrument());
    let action = PmScheduledActionKey::new(
        account_scope(1),
        instrument(),
        PmOrderSide::Buy,
        PmScheduledActionKind::Freshness,
    );
    schedule.schedule(action, 10, 1, 1_000).expect("schedule");
    let maximum = PmLanePolicy::for_lane(PmLaneKind::Scheduled)
        .maximum_age_ns()
        .expect("scheduled age");
    let error = schedule
        .pop_due(10 + maximum + 1)
        .expect_err("aged schedule fails closed");
    assert!(matches!(
        error,
        PmScheduleError::Aged {
            pending,
            due_age_ns,
            maximum_due_age_ns,
            action: SaturationAction::SuppressQuoteAndCancelOwned,
            ..
        } if pending == action
            && due_age_ns == maximum + 1
            && maximum_due_age_ns == maximum
    ));
    let projection = schedule.projection(10 + maximum + 1).expect("projection");
    assert_eq!(projection.metrics().depth(), 1);
    assert!(projection.metrics().fail_closed());
}

#[test]
fn scheduled_equal_deadline_order_is_variant_account_token_side() {
    let mut schedule = PmQuoteScheduleRole::new(instrument());
    let kinds = [
        PmScheduledActionKind::QuoteEvaluation,
        PmScheduledActionKind::Freshness,
        PmScheduledActionKind::ReconciliationRefresh,
        PmScheduledActionKind::CancelOwnedQuote,
    ];
    for kind in kinds {
        schedule
            .schedule(
                PmScheduledActionKey::new(account_scope(1), instrument(), PmOrderSide::Buy, kind),
                100,
                1,
                1_000,
            )
            .expect("schedule");
    }
    let mut observed = Vec::with_capacity(4);
    while let Some(due) = schedule.pop_due(100).expect("pop") {
        observed.push(due.key().kind());
    }
    assert_eq!(
        observed,
        [
            PmScheduledActionKind::CancelOwnedQuote,
            PmScheduledActionKind::ReconciliationRefresh,
            PmScheduledActionKind::Freshness,
            PmScheduledActionKind::QuoteEvaluation,
        ]
    );
}

#[test]
fn preallocated_queue_capacity_does_not_grow_across_reuse() {
    let mut lane = PmCompleteLane::<u64>::new(PmLaneKind::Critical);
    let reserved = lane.reserved_capacity_bytes();
    for sequence in 1..=512_u64 {
        lane.enqueue(
            internal_ingress(sequence, sequence),
            sequence,
            0,
            PmCompleteSourceKind::InternalSignal,
        )
        .expect("first fill");
    }
    while lane.pop().is_some() {}
    for sequence in 1..=512_u64 {
        lane.enqueue(
            internal_ingress(sequence + 1_000, sequence + 1_000),
            sequence,
            0,
            PmCompleteSourceKind::InternalSignal,
        )
        .expect("second fill");
    }
    assert_eq!(lane.reserved_capacity_bytes(), reserved);
}
