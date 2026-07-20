use reap_pm_core::{
    ConnectionEpoch, EvmAddress, IngressSequence, PmAccountHandle, PmAccountScope, PmChainId,
    PmConnectionId, PmEnvironmentId, PmFillEvent, PmFillExecution, PmFillFee, PmFillId, PmFillKey,
    PmFillRole, PmFunderId, PmInstrumentHandle, PmMarketHandle, PmOrderEvent, PmOrderIdentity,
    PmOrderProgress, PmOrderSide, PmOrderStatus, PmPrice, PmProductSource, PmQuantity, PmSignerId,
    PmSnapshotCompleteness, PmSnapshotEvidence, PmSourceHandle, PmTokenHandle, PmVenueOrderId,
    PmVenueOrderKey, ReceivedEventClock, SnapshotRevision, U256,
};
use reap_pm_live::{
    LaneEnqueueError, PmIngressOrder, PmLaneKind, PmLanePolicy, PmLaneService, PmLaneSet,
    PmLaneSignal, PmLaneSignalKind, PmObservedEvent, PmScheduledAction, PmScheduledActionKind,
    PmScheduledSide, PmServiceTurnError, SaturationAction, ServicedLaneItem,
    ServicedScheduledAction,
};
use reap_pm_live_contracts::PmCapabilityLane;
use reap_polymarket_adapter::PmCompleteOpenOrdersSnapshot;

fn clock(receive: u64) -> ReceivedEventClock {
    ReceivedEventClock::new(None, receive + 1_000, receive).unwrap()
}

fn ingress(sequence: u64) -> PmIngressOrder {
    PmIngressOrder::new(
        PmConnectionId::new("fixture-connection").unwrap(),
        ConnectionEpoch::new(1),
        IngressSequence::new(sequence),
    )
    .unwrap()
}

fn signal(kind: PmLaneSignalKind) -> PmLaneSignal {
    PmLaneSignal::new(kind, PmSourceHandle::from_ordinal(1))
}

fn instrument() -> PmInstrumentHandle {
    PmInstrumentHandle::new(
        PmMarketHandle::from_ordinal(1),
        PmTokenHandle::from_ordinal(2),
    )
}

fn account_scope() -> PmAccountScope {
    let eoa = EvmAddress::from_bytes([7; 20]).unwrap();
    PmAccountScope::new(
        PmEnvironmentId::new("fixture").unwrap(),
        PmChainId::new(137).unwrap(),
        PmSignerId::new(eoa),
        PmFunderId::new(eoa),
        PmAccountHandle::from_ordinal(4),
    )
}

fn account_source() -> PmProductSource {
    PmProductSource::polymarket_account(PmSourceHandle::from_ordinal(2), account_scope().handle())
}

fn order_identity() -> PmOrderIdentity {
    PmOrderIdentity::new(
        None,
        Some(PmVenueOrderKey::new(
            account_scope().handle(),
            PmVenueOrderId::new("order-1").unwrap(),
        )),
    )
    .unwrap()
}

fn order() -> PmOrderEvent {
    PmOrderEvent::new(
        account_source(),
        instrument(),
        order_identity(),
        PmOrderSide::Buy,
        PmPrice::parse_decimal("0.4").unwrap(),
        PmOrderProgress::new(
            PmQuantity::parse_decimal("1").unwrap(),
            U256::ZERO,
            PmOrderStatus::Open,
        )
        .unwrap(),
    )
    .unwrap()
}

fn fill() -> PmFillEvent {
    PmFillEvent::new(
        account_source(),
        instrument(),
        PmFillKey::new(account_scope().handle(), PmFillId::new("fill-1").unwrap()),
        order_identity(),
        PmFillExecution::new(
            PmOrderSide::Buy,
            PmFillRole::Maker,
            PmPrice::parse_decimal("0.4").unwrap(),
            PmQuantity::parse_decimal("0.1").unwrap(),
            PmFillFee::Unknown,
        ),
    )
    .unwrap()
}

fn open_orders() -> PmCompleteOpenOrdersSnapshot {
    PmCompleteOpenOrdersSnapshot::new(
        account_source(),
        account_scope(),
        PmSnapshotEvidence::new(SnapshotRevision::new(1), PmSnapshotCompleteness::Complete)
            .unwrap(),
        Vec::new(),
    )
    .unwrap()
}

#[derive(Default)]
struct Recorder {
    signals: Vec<(PmLaneKind, PmLaneSignalKind, u8, u64)>,
    scheduled: Vec<(PmScheduledActionKind, u8)>,
    orders: Vec<(PmLaneKind, u8)>,
    fills: Vec<(PmLaneKind, u8)>,
}

impl PmLaneService for Recorder {
    fn on_signal(&mut self, item: ServicedLaneItem<PmLaneSignal>) {
        let lane = item.lane();
        let rank = item.key().variant_rank();
        let age = item.clock().queue_age_ns();
        let kind = item.into_value().kind();
        self.signals.push((lane, kind, rank, age));
    }

    fn on_scheduled(&mut self, item: ServicedScheduledAction) {
        self.scheduled
            .push((item.action().kind(), item.key().action_variant_rank()));
    }

    fn on_order(&mut self, item: ServicedLaneItem<PmOrderEvent>) {
        self.orders.push((item.lane(), item.key().variant_rank()));
    }

    fn on_fill(&mut self, item: ServicedLaneItem<PmFillEvent>) {
        self.fills.push((item.lane(), item.key().variant_rank()));
    }
}

#[test]
fn every_plan_lane_maps_to_exact_bounded_runtime_storage() {
    let lanes = PmLaneSet::new();
    for plan_lane in PmCapabilityLane::ALL {
        let lane = PmLaneKind::from(plan_lane);
        let policy = PmLanePolicy::for_lane(lane);
        assert!(policy.capacity() > 0);
        assert_eq!(lanes.metrics(lane).depth(), 0);
    }
}

#[test]
fn all_eleven_lane_policies_match_the_frozen_oracle() {
    let expected = [
        (
            PmLaneKind::Critical,
            512,
            32,
            Some(250_000_000),
            SaturationAction::GlobalStop,
            Some(512),
        ),
        (
            PmLaneKind::Persistence,
            512,
            32,
            Some(250_000_000),
            SaturationAction::GlobalStop,
            Some(512),
        ),
        (
            PmLaneKind::Private,
            4_096,
            64,
            Some(250_000_000),
            SaturationAction::HaltAccountAndRequireReconciliation,
            Some(64),
        ),
        (
            PmLaneKind::Scheduled,
            4_096,
            64,
            Some(100_000_000),
            SaturationAction::SuppressQuoteAndCancelOwned,
            Some(16),
        ),
        (
            PmLaneKind::Public,
            8_192,
            256,
            Some(500_000_000),
            SaturationAction::InvalidateStreamAndResync,
            Some(256),
        ),
        (
            PmLaneKind::Reconciliation,
            128,
            16,
            Some(5_000_000_000),
            SaturationAction::KeepUnreadyAndRetry,
            Some(8),
        ),
        (
            PmLaneKind::Telemetry,
            128,
            32,
            None,
            SaturationAction::CoalesceTelemetry,
            Some(1),
        ),
        (
            PmLaneKind::ReconciliationRequest,
            128,
            16,
            Some(1_000_000_000),
            SaturationAction::RetainPendingRefresh,
            None,
        ),
        (
            PmLaneKind::Capture,
            8_192,
            256,
            Some(500_000_000),
            SaturationAction::InvalidateCaptureAndResync,
            None,
        ),
        (
            PmLaneKind::Journal,
            1_024,
            128,
            Some(1_000_000_000),
            SaturationAction::SuppressDispatchAndHaltQuotes,
            None,
        ),
        (
            PmLaneKind::FakeEffect,
            256,
            32,
            Some(250_000_000),
            SaturationAction::RejectEffectAndHaltQuotes,
            None,
        ),
    ];

    assert_eq!(expected.len(), PmCapabilityLane::ALL.len());
    for (plan_lane, expected_policy) in PmCapabilityLane::ALL.into_iter().zip(expected) {
        let (lane, capacity, high_water, age, action, burst) = expected_policy;
        assert_eq!(PmLaneKind::from(plan_lane), lane);
        let actual = PmLanePolicy::for_lane(lane);
        assert_eq!(actual.capacity(), capacity);
        assert_eq!(actual.nominal_high_water(), high_water);
        assert_eq!(actual.maximum_age_ns(), age);
        assert_eq!(actual.saturation_action(), action);
        assert_eq!(actual.service_burst(), burst);
    }
}

#[test]
fn all_eleven_lanes_have_typed_admission() {
    let mut lanes = PmLaneSet::new();
    let signals = [
        PmLaneSignalKind::Shutdown,
        PmLaneSignalKind::DurableFailure,
        PmLaneSignalKind::PrivateConnectionUnavailable,
        PmLaneSignalKind::PublicConnectionUnavailable,
        PmLaneSignalKind::Telemetry,
        PmLaneSignalKind::ReconciliationRequest,
        PmLaneSignalKind::CaptureFrame,
        PmLaneSignalKind::JournalRecord,
        PmLaneSignalKind::FakePlaceGtcPostOnly,
    ];
    for (index, kind) in signals.into_iter().enumerate() {
        let sequence = u64::try_from(index).unwrap() + 1;
        lanes
            .enqueue_signal(clock(sequence), ingress(sequence), signal(kind))
            .unwrap();
        assert_eq!(lanes.metrics(kind.lane()).depth(), 1);
    }

    lanes
        .enqueue_scheduled(
            10,
            1,
            PmScheduledAction::new(
                PmScheduledActionKind::QuoteEvaluation,
                account_scope().handle(),
                instrument().token(),
                PmScheduledSide::NotApplicable,
            ),
        )
        .unwrap();
    lanes
        .enqueue_observation(clock(20), ingress(20), open_orders())
        .unwrap();
    assert_eq!(lanes.metrics(PmLaneKind::Scheduled).depth(), 1);
    assert_eq!(lanes.metrics(PmLaneKind::Reconciliation).depth(), 1);
}

#[test]
fn full_private_lane_returns_unconsumed_signal_and_reports_metrics() {
    let mut lanes = PmLaneSet::new();
    for sequence in 1..=4_096 {
        lanes
            .enqueue_signal(
                clock(sequence),
                ingress(sequence),
                signal(PmLaneSignalKind::PrivateConnectionUnavailable),
            )
            .unwrap();
    }
    let rejected = signal(PmLaneSignalKind::PrivateConnectionUnavailable);
    assert_eq!(
        lanes.enqueue_signal(clock(4_097), ingress(4_097), rejected),
        Err(LaneEnqueueError::Full {
            value: rejected,
            action: SaturationAction::HaltAccountAndRequireReconciliation,
        })
    );
    let metrics = lanes.metrics(PmLaneKind::Private);
    assert_eq!(metrics.depth(), 4_096);
    assert_eq!(metrics.high_water(), 4_096);
    assert_eq!(metrics.rejected_full(), 1);
    assert_eq!(metrics.coalesced(), 0);
}

#[test]
fn telemetry_alone_coalesces_at_its_exact_bound() {
    let mut lanes = PmLaneSet::new();
    for sequence in 1..=129 {
        lanes
            .enqueue_signal(
                clock(sequence),
                ingress(sequence),
                signal(PmLaneSignalKind::Telemetry),
            )
            .unwrap();
    }
    let metrics = lanes.metrics(PmLaneKind::Telemetry);
    assert_eq!(metrics.depth(), 128);
    assert_eq!(metrics.high_water(), 128);
    assert_eq!(metrics.rejected_full(), 0);
    assert_eq!(metrics.coalesced(), 1);
}

#[test]
fn order_and_fill_derive_private_lane_and_frozen_ranks() {
    assert_eq!(PmObservedEvent::lane(&order()), PmLaneKind::Private);
    assert_eq!(PmObservedEvent::variant_rank(&order()), 2);
    assert_eq!(PmObservedEvent::lane(&fill()), PmLaneKind::Private);
    assert_eq!(PmObservedEvent::variant_rank(&fill()), 1);

    let mut lanes = PmLaneSet::new();
    lanes
        .enqueue_observation(clock(2), ingress(2), order())
        .unwrap();
    lanes
        .enqueue_observation(clock(1), ingress(1), fill())
        .unwrap();
    let mut recorder = Recorder::default();
    lanes.service_turn(3, &mut recorder).unwrap();
    assert_eq!(recorder.fills, vec![(PmLaneKind::Private, 1)]);
    assert_eq!(recorder.orders, vec![(PmLaneKind::Private, 2)]);
}

#[test]
fn scheduled_key_is_distinct_and_only_due_actions_are_serviced() {
    let mut lanes = PmLaneSet::new();
    let cancel = PmScheduledAction::new(
        PmScheduledActionKind::CancelOwned,
        account_scope().handle(),
        instrument().token(),
        PmScheduledSide::Bid,
    );
    let quote = PmScheduledAction::new(
        PmScheduledActionKind::QuoteEvaluation,
        account_scope().handle(),
        instrument().token(),
        PmScheduledSide::NotApplicable,
    );
    lanes.enqueue_scheduled(10, 2, quote).unwrap();
    lanes.enqueue_scheduled(10, 1, cancel).unwrap();
    lanes.enqueue_scheduled(30, 3, quote).unwrap();

    let mut recorder = Recorder::default();
    assert_eq!(lanes.service_turn(20, &mut recorder).unwrap(), 2);
    assert_eq!(
        recorder.scheduled,
        vec![
            (PmScheduledActionKind::CancelOwned, 0),
            (PmScheduledActionKind::QuoteEvaluation, 3),
        ]
    );
    assert_eq!(lanes.metrics(PmLaneKind::Scheduled).depth(), 1);
}

#[test]
fn lower_rank_age_failure_does_not_prevent_critical_service() {
    let mut lanes = PmLaneSet::new();
    lanes
        .enqueue_signal(
            clock(1),
            ingress(1),
            signal(PmLaneSignalKind::PublicConnectionUnavailable),
        )
        .unwrap();
    lanes
        .enqueue_signal(
            clock(500_000_001),
            ingress(2),
            signal(PmLaneSignalKind::Shutdown),
        )
        .unwrap();

    let mut recorder = Recorder::default();
    assert_eq!(
        lanes.service_turn(500_000_002, &mut recorder),
        Err(PmServiceTurnError::Aged {
            lane: PmLaneKind::Public,
            action: SaturationAction::InvalidateStreamAndResync,
        })
    );
    assert_eq!(recorder.signals.len(), 1);
    assert_eq!(recorder.signals[0].0, PmLaneKind::Critical);
}

#[test]
fn consumer_stamps_service_time_after_priority_selection() {
    let mut lanes = PmLaneSet::new();
    lanes
        .enqueue_signal(
            clock(7),
            ingress(1),
            signal(PmLaneSignalKind::PublicConnectionUnavailable),
        )
        .unwrap();
    let mut recorder = Recorder::default();
    lanes.service_turn(19, &mut recorder).unwrap();
    assert_eq!(
        recorder.signals,
        vec![(
            PmLaneKind::Public,
            PmLaneSignalKind::PublicConnectionUnavailable,
            0,
            12,
        )]
    );
}
