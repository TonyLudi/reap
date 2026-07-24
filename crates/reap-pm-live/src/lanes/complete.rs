use reap_pm_core::PmInstrumentHandle;
use thiserror::Error;

use super::PmPublicLaneState;
use super::{
    PM_INPUT_SERVICE_PRIORITY, PmCompleteIngress, PmCompleteLane, PmCompleteLaneAgeFault,
    PmCompleteLaneBuildError, PmCompleteLaneCheckError, PmCompleteLaneEnqueueError,
    PmCompleteLaneMetrics, PmCompleteServiced, PmCompleteSourceKind, PmCriticalInput, PmLaneKind,
    PmLaneMetrics, PmLanePolicy, PmPersistenceInput, PmPrivateInput, PmPublicLaneService,
    PmReconciliationInput, PmServiceTurnError, PmTelemetryInput, SaturationAction,
};
use crate::composition::PmPublicCaptureRun;
use crate::schedule::{
    PmDueScheduledAction, PmQuoteScheduleRole, PmScheduleAdmission, PmScheduleError,
    PmScheduleMetrics, PmScheduledActionKey,
};

/// Synchronous transfer boundary for the six non-public complete-scheduler
/// ranks. Public callbacks remain the existing authenticated public service
/// boundary and are inherited rather than duplicated here.
pub(crate) trait PmCompleteLaneService: PmPublicLaneService {
    fn on_critical(&mut self, item: PmCompleteServiced<PmCriticalInput>);
    fn on_persistence(&mut self, item: PmCompleteServiced<PmPersistenceInput>);
    fn on_private(&mut self, item: PmCompleteServiced<PmPrivateInput>);
    fn on_scheduled(&mut self, item: PmDueScheduledAction);
    fn on_reconciliation(&mut self, item: PmCompleteServiced<PmReconciliationInput>);
    fn on_telemetry(&mut self, item: PmCompleteServiced<PmTelemetryInput>);

    fn stop_complete_service_turn(&self) -> bool {
        false
    }
}

/// Fixed-cardinality service counts in the frozen seven-rank order.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub struct PmCompleteServiceCounts {
    by_rank: [usize; 7],
}

impl PmCompleteServiceCounts {
    #[must_use]
    pub const fn for_lane(self, lane: PmLaneKind) -> Option<usize> {
        match lane.service_priority_rank() {
            Some(rank) => Some(self.by_rank[rank as usize]),
            None => None,
        }
    }

    #[must_use]
    pub const fn total(self) -> usize {
        let mut index = 0;
        let mut total = 0;
        while index < self.by_rank.len() {
            total += self.by_rank[index];
            index += 1;
        }
        total
    }

    fn record(&mut self, lane: PmLaneKind, count: usize) {
        let rank = usize::from(
            lane.service_priority_rank()
                .expect("only input lanes are serviced"),
        );
        self.by_rank[rank] = count;
    }
}

/// Fixed-label fail-closed evidence. These are scheduler transition counters,
/// not canonical trading state and do not themselves grant mutation authority.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub struct PmCompleteFailClosedMetrics {
    transitions_by_action: [u64; 10],
    global_stopped: bool,
    account_halted: bool,
    account_unready: bool,
    complete_reconciliation_required: bool,
    retry_pending: bool,
    quote_suppressed: bool,
    cancel_owned_required: bool,
    fake_dispatch_suppressed: bool,
}

impl PmCompleteFailClosedMetrics {
    #[must_use]
    pub const fn transitions(self, action: SaturationAction) -> u64 {
        self.transitions_by_action[action_index(action)]
    }

    #[must_use]
    pub const fn global_stopped(self) -> bool {
        self.global_stopped
    }

    #[must_use]
    pub const fn account_halted(self) -> bool {
        self.account_halted
    }

    #[must_use]
    pub const fn account_unready(self) -> bool {
        self.account_unready
    }

    #[must_use]
    pub const fn complete_reconciliation_required(self) -> bool {
        self.complete_reconciliation_required
    }

    #[must_use]
    pub const fn retry_pending(self) -> bool {
        self.retry_pending
    }

    #[must_use]
    pub const fn quote_suppressed(self) -> bool {
        self.quote_suppressed
    }

    #[must_use]
    pub const fn cancel_owned_required(self) -> bool {
        self.cancel_owned_required
    }

    #[must_use]
    pub const fn fake_dispatch_suppressed(self) -> bool {
        self.fake_dispatch_suppressed
    }

    fn latch(&mut self, action: SaturationAction) {
        let counter = &mut self.transitions_by_action[action_index(action)];
        *counter = counter.saturating_add(1);
        match action {
            SaturationAction::GlobalStop => {
                self.global_stopped = true;
                self.fake_dispatch_suppressed = true;
            }
            SaturationAction::HaltAccountAndRequireReconciliation => {
                self.account_halted = true;
                self.account_unready = true;
                self.complete_reconciliation_required = true;
            }
            SaturationAction::InvalidateStreamAndResync
            | SaturationAction::InvalidateCaptureAndResync => {}
            SaturationAction::KeepUnreadyAndRetry | SaturationAction::RetainPendingRefresh => {
                self.account_unready = true;
                self.retry_pending = true;
            }
            SaturationAction::SuppressDispatchAndHaltQuotes
            | SaturationAction::RejectEffectAndHaltQuotes => {
                self.quote_suppressed = true;
                self.fake_dispatch_suppressed = true;
            }
            SaturationAction::SuppressQuoteAndCancelOwned => {
                self.global_stopped = true;
                self.quote_suppressed = true;
                self.cancel_owned_required = true;
            }
            SaturationAction::CoalesceTelemetry => {}
        }
    }
}

/// One observable fixed-cardinality snapshot of all seven input ranks.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct PmCompleteSchedulerMetrics {
    lanes: [PmCompleteLaneMetrics; 7],
    fail_closed: PmCompleteFailClosedMetrics,
    service_turns: u64,
    consumer_transfer_poisoned: bool,
    reserved_capacity_bytes: usize,
}

impl PmCompleteSchedulerMetrics {
    #[must_use]
    pub const fn lane(self, lane: PmLaneKind) -> Option<PmCompleteLaneMetrics> {
        match lane.service_priority_rank() {
            Some(rank) => Some(self.lanes[rank as usize]),
            None => None,
        }
    }

    #[must_use]
    pub const fn fail_closed(self) -> PmCompleteFailClosedMetrics {
        self.fail_closed
    }

    #[must_use]
    pub const fn service_turns(self) -> u64 {
        self.service_turns
    }

    #[must_use]
    pub const fn consumer_transfer_poisoned(self) -> bool {
        self.consumer_transfer_poisoned
    }

    #[must_use]
    pub const fn reserved_capacity_bytes(self) -> usize {
        self.reserved_capacity_bytes
    }
}

/// The complete by-value input scheduler.
///
/// Construction consumes the existing public lane and quote schedule roles.
/// It never constructs a sibling public authority or a second timer queue.
#[derive(Debug)]
enum PmCompletePublicOwner {
    Capture(Box<PmPublicCaptureRun>),
    Bare(PmPublicLaneState),
}

impl PmCompletePublicOwner {
    fn service_turn<C: PmPublicLaneService>(
        &mut self,
        monotonic_now_ns: u64,
        consumer: &mut C,
    ) -> Result<usize, PmServiceTurnError> {
        match self {
            Self::Capture(run) => run.service_lane_turn(monotonic_now_ns, consumer),
            Self::Bare(lane) => lane.service_turn(monotonic_now_ns, consumer),
        }
    }

    fn metrics(&self) -> PmLaneMetrics {
        match self {
            Self::Capture(run) => run.public_lane_metrics(),
            Self::Bare(lane) => lane.metrics(),
        }
    }

    fn consumer_transfer_poisoned(&self) -> bool {
        match self {
            Self::Capture(run) => run.public_consumer_transfer_poisoned(),
            Self::Bare(lane) => lane.consumer_transfer_poisoned(),
        }
    }

    fn reserved_capacity_bytes(&self) -> usize {
        match self {
            Self::Capture(run) => run
                .reserved_capacity_bytes()
                .saturating_add(std::mem::size_of::<PmPublicCaptureRun>()),
            Self::Bare(lane) => lane.reserved_capacity_bytes(),
        }
    }

    fn capture(&self) -> Option<&PmPublicCaptureRun> {
        match self {
            Self::Capture(run) => Some(run.as_ref()),
            Self::Bare(_) => None,
        }
    }

    fn capture_mut(&mut self) -> Option<&mut PmPublicCaptureRun> {
        match self {
            Self::Capture(run) => Some(run.as_mut()),
            Self::Bare(_) => None,
        }
    }

    fn into_capture(self) -> Option<PmPublicCaptureRun> {
        match self {
            Self::Capture(run) => Some(*run),
            Self::Bare(_) => None,
        }
    }
}

pub(crate) struct PmCompleteInputLanes {
    critical: PmCompleteLane<PmCriticalInput>,
    persistence: PmCompleteLane<PmPersistenceInput>,
    private: PmCompleteLane<PmPrivateInput>,
    public: PmCompletePublicOwner,
    reconciliation: PmCompleteLane<PmReconciliationInput>,
    telemetry: PmCompleteLane<PmTelemetryInput>,
    schedule: PmQuoteScheduleRole,
    fail_closed: PmCompleteFailClosedMetrics,
    failure_latched: [bool; 7],
    recoverable_aged_drain_remaining: [usize; 7],
    service_turns: u64,
    public_serviced: u64,
    consumer_transfer_in_flight: bool,
}

impl PmCompleteInputLanes {
    pub(crate) fn new(public: Box<PmPublicCaptureRun>, schedule: PmQuoteScheduleRole) -> Self {
        Self::with_public_owner(PmCompletePublicOwner::Capture(public), schedule)
    }

    fn with_public_owner(public: PmCompletePublicOwner, schedule: PmQuoteScheduleRole) -> Self {
        Self {
            critical: PmCompleteLane::new(PmLaneKind::Critical),
            persistence: PmCompleteLane::new(PmLaneKind::Persistence),
            private: PmCompleteLane::new(PmLaneKind::Private),
            public,
            reconciliation: PmCompleteLane::new(PmLaneKind::Reconciliation),
            telemetry: PmCompleteLane::new(PmLaneKind::Telemetry),
            schedule,
            fail_closed: PmCompleteFailClosedMetrics::default(),
            failure_latched: [false; 7],
            recoverable_aged_drain_remaining: [0; 7],
            service_turns: 0,
            public_serviced: 0,
            consumer_transfer_in_flight: false,
        }
    }

    pub(crate) fn for_instrument(instrument: PmInstrumentHandle) -> Self {
        Self::with_public_owner(
            PmCompletePublicOwner::Bare(PmPublicLaneState::new()),
            PmQuoteScheduleRole::new(instrument),
        )
    }

    pub(crate) fn enqueue_critical(
        &mut self,
        ingress: PmCompleteIngress,
        input: PmCriticalInput,
    ) -> Result<(), PmCompleteLaneEnqueueError<PmCriticalInput>> {
        let rank = input.variant_rank();
        let result =
            self.critical
                .enqueue(ingress, input, rank, PmCompleteSourceKind::InternalSignal);
        self.observe_enqueue_result(PmLaneKind::Critical, &result);
        result
    }

    pub(crate) fn enqueue_built_critical<E>(
        &mut self,
        ingress: PmCompleteIngress,
        variant_rank: u8,
        build: impl FnOnce() -> Result<PmCriticalInput, E>,
    ) -> Result<(), PmCompleteLaneBuildError<E>> {
        let result = self.critical.enqueue_built(
            ingress,
            variant_rank,
            PmCompleteSourceKind::InternalSignal,
            build,
        );
        if let Err(error) = &result
            && let Some(action) = error.action()
        {
            self.latch_failure(PmLaneKind::Critical, action);
        }
        result
    }

    pub(crate) fn enqueue_persistence(
        &mut self,
        ingress: PmCompleteIngress,
        input: PmPersistenceInput,
    ) -> Result<(), PmCompleteLaneEnqueueError<PmPersistenceInput>> {
        let rank = input.variant_rank();
        let result =
            self.persistence
                .enqueue(ingress, input, rank, PmCompleteSourceKind::InternalSignal);
        self.observe_enqueue_result(PmLaneKind::Persistence, &result);
        result
    }

    pub(crate) fn enqueue_private(
        &mut self,
        ingress: PmCompleteIngress,
        input: PmPrivateInput,
    ) -> Result<(), PmCompleteLaneEnqueueError<PmPrivateInput>> {
        let ingress = input.fixture_ingress().unwrap_or(ingress);
        let rank = input.variant_rank();
        let result = self.private.enqueue(
            ingress,
            input,
            rank,
            PmCompleteSourceKind::PolymarketAccount,
        );
        self.observe_enqueue_result(PmLaneKind::Private, &result);
        result
    }

    pub(crate) fn enqueue_reconciliation(
        &mut self,
        _ingress: PmCompleteIngress,
        input: PmReconciliationInput,
    ) -> Result<(), PmCompleteLaneEnqueueError<PmReconciliationInput>> {
        let ingress = input.fixture_ingress();
        let rank = input.variant_rank();
        let result = self.reconciliation.enqueue(
            ingress,
            input,
            rank,
            PmCompleteSourceKind::PolymarketAccount,
        );
        self.observe_enqueue_result(PmLaneKind::Reconciliation, &result);
        result
    }

    pub(crate) fn enqueue_telemetry(
        &mut self,
        ingress: PmCompleteIngress,
        input: PmTelemetryInput,
    ) -> Result<(), PmCompleteLaneEnqueueError<PmTelemetryInput>> {
        let rank = input.variant_rank();
        self.telemetry
            .enqueue(ingress, input, rank, PmCompleteSourceKind::InternalSignal)
    }

    pub(crate) fn schedule(
        &mut self,
        key: PmScheduledActionKey,
        deadline_ns: u64,
        scheduled_at_ns: u64,
        decision_wall_timestamp_ms: u64,
    ) -> Result<PmScheduleAdmission, PmScheduleError> {
        let result = self.schedule.schedule(
            key,
            deadline_ns,
            scheduled_at_ns,
            decision_wall_timestamp_ms,
        );
        if matches!(result, Err(PmScheduleError::Full { .. })) {
            self.latch_failure(
                PmLaneKind::Scheduled,
                SaturationAction::SuppressQuoteAndCancelOwned,
            );
        }
        result
    }

    pub(crate) fn resolve_aged_schedule(&mut self, key: PmScheduledActionKey) -> bool {
        self.schedule.resolve_aged(key)
    }

    pub(crate) fn public_capture(&self) -> Option<&PmPublicCaptureRun> {
        self.public.capture()
    }

    pub(crate) fn public_capture_mut(&mut self) -> Option<&mut PmPublicCaptureRun> {
        self.public.capture_mut()
    }

    pub(crate) fn into_public_capture(self) -> Option<PmPublicCaptureRun> {
        self.public.into_capture()
    }

    pub(crate) fn service_turn<C: PmCompleteLaneService>(
        &mut self,
        monotonic_now_ns: u64,
        consumer: &mut C,
    ) -> Result<PmCompleteServiceCounts, PmCompleteServiceError> {
        if self.consumer_transfer_in_flight || self.public.consumer_transfer_poisoned() {
            return Err(PmCompleteServiceError::ConsumerTransferPoisoned);
        }
        self.service_turns = self.service_turns.saturating_add(1);
        let mut counts = PmCompleteServiceCounts::default();

        let count = service_lane(
            &mut self.critical,
            monotonic_now_ns,
            &mut self.consumer_transfer_in_flight,
            None,
            |item| {
                consumer.on_critical(item);
                consumer.stop_complete_service_turn()
            },
        )
        .map_err(|error| self.observe_service_error(error))?;
        counts.record(PmLaneKind::Critical, count);
        if consumer.stop_complete_service_turn() {
            return Ok(counts);
        }
        if self.critical.len() != 0 {
            self.latch_failure(PmLaneKind::Critical, SaturationAction::GlobalStop);
            return Err(PmCompleteServiceError::CriticalBurstExhausted {
                remaining: self.critical.len(),
            });
        }

        let count = service_lane(
            &mut self.persistence,
            monotonic_now_ns,
            &mut self.consumer_transfer_in_flight,
            None,
            |item| {
                consumer.on_persistence(item);
                consumer.stop_complete_service_turn()
            },
        )
        .map_err(|error| self.observe_service_error(error))?;
        counts.record(PmLaneKind::Persistence, count);
        if consumer.stop_complete_service_turn() {
            return Ok(counts);
        }

        let private_rank = usize::from(
            PmLaneKind::Private
                .service_priority_rank()
                .expect("private is a complete scheduler rank"),
        );
        let count = service_lane(
            &mut self.private,
            monotonic_now_ns,
            &mut self.consumer_transfer_in_flight,
            Some(&mut self.recoverable_aged_drain_remaining[private_rank]),
            |item| {
                consumer.on_private(item);
                consumer.stop_complete_service_turn()
            },
        )
        .map_err(|error| self.observe_service_error(error))?;
        counts.record(PmLaneKind::Private, count);
        if consumer.stop_complete_service_turn() {
            return Ok(counts);
        }

        let count = self
            .service_schedule(monotonic_now_ns, consumer)
            .map_err(|error| self.observe_schedule_error(error))?;
        counts.record(PmLaneKind::Scheduled, count);
        if consumer.stop_complete_service_turn() {
            return Ok(counts);
        }

        let count = self
            .public
            .service_turn(monotonic_now_ns, consumer)
            .map_err(PmCompleteServiceError::Public)?;
        self.public_serviced = self
            .public_serviced
            .saturating_add(u64::try_from(count).expect("public burst fits u64"));
        counts.record(PmLaneKind::Public, count);
        if consumer.stop_complete_service_turn() {
            return Ok(counts);
        }

        let reconciliation_rank = usize::from(
            PmLaneKind::Reconciliation
                .service_priority_rank()
                .expect("reconciliation is a complete scheduler rank"),
        );
        let count = service_lane(
            &mut self.reconciliation,
            monotonic_now_ns,
            &mut self.consumer_transfer_in_flight,
            Some(&mut self.recoverable_aged_drain_remaining[reconciliation_rank]),
            |item| {
                consumer.on_reconciliation(item);
                consumer.stop_complete_service_turn()
            },
        )
        .map_err(|error| self.observe_service_error(error))?;
        counts.record(PmLaneKind::Reconciliation, count);
        if consumer.stop_complete_service_turn() {
            return Ok(counts);
        }

        let count = service_lane(
            &mut self.telemetry,
            monotonic_now_ns,
            &mut self.consumer_transfer_in_flight,
            None,
            |item| {
                consumer.on_telemetry(item);
                consumer.stop_complete_service_turn()
            },
        )
        .map_err(|error| self.observe_service_error(error))?;
        counts.record(PmLaneKind::Telemetry, count);
        Ok(counts)
    }

    pub(crate) fn metrics(
        &mut self,
        monotonic_now_ns: u64,
    ) -> Result<PmCompleteSchedulerMetrics, PmScheduleError> {
        let schedule = self.schedule.projection(monotonic_now_ns)?.metrics();
        let scheduled_queue = PmLaneMetrics::new(
            schedule.depth(),
            schedule.high_water(),
            schedule.rejected_full(),
            0,
            0,
        );
        let lanes = [
            self.critical.metrics(),
            self.persistence.metrics(),
            self.private.metrics(),
            PmCompleteLaneMetrics::new(
                PmLaneKind::Scheduled,
                scheduled_queue,
                schedule.serviced(),
                u64::from(schedule.fail_closed()),
                schedule.maximum_due_age_ns(),
            ),
            PmCompleteLaneMetrics::new(
                PmLaneKind::Public,
                self.public.metrics(),
                self.public_serviced,
                0,
                0,
            ),
            self.reconciliation.metrics(),
            self.telemetry.metrics(),
        ];
        debug_assert_eq!(
            lanes.map(PmCompleteLaneMetrics::lane),
            PM_INPUT_SERVICE_PRIORITY
        );
        Ok(PmCompleteSchedulerMetrics {
            lanes,
            fail_closed: self.fail_closed,
            service_turns: self.service_turns,
            consumer_transfer_poisoned: self.consumer_transfer_in_flight
                || self.public.consumer_transfer_poisoned(),
            reserved_capacity_bytes: self.reserved_capacity_bytes(),
        })
    }

    pub(crate) fn schedule_metrics(
        &mut self,
        monotonic_now_ns: u64,
    ) -> Result<PmScheduleMetrics, PmScheduleError> {
        Ok(self.schedule.projection(monotonic_now_ns)?.metrics())
    }

    pub(crate) fn reserved_capacity_bytes(&self) -> usize {
        self.critical
            .reserved_capacity_bytes()
            .saturating_add(self.persistence.reserved_capacity_bytes())
            .saturating_add(self.private.reserved_capacity_bytes())
            .saturating_add(self.reconciliation.reserved_capacity_bytes())
            .saturating_add(self.telemetry.reserved_capacity_bytes())
            .saturating_add(self.schedule.reserved_capacity_bytes())
            .saturating_add(self.public.reserved_capacity_bytes())
    }

    pub(crate) fn private_and_reconciliation_empty(&self) -> bool {
        self.private.len() == 0 && self.reconciliation.len() == 0
    }

    fn service_schedule<C: PmCompleteLaneService>(
        &mut self,
        monotonic_now_ns: u64,
        consumer: &mut C,
    ) -> Result<usize, PmScheduleError> {
        let limit = PmLanePolicy::for_lane(PmLaneKind::Scheduled)
            .service_burst()
            .expect("scheduled lane has a fixed burst");
        let mut serviced = 0;
        while serviced < limit {
            let Some(item) = self.schedule.pop_due(monotonic_now_ns)? else {
                break;
            };
            self.consumer_transfer_in_flight = true;
            consumer.on_scheduled(item);
            self.consumer_transfer_in_flight = false;
            serviced += 1;
            if consumer.stop_complete_service_turn() {
                break;
            }
        }
        Ok(serviced)
    }

    fn observe_enqueue_result<T>(
        &mut self,
        lane: PmLaneKind,
        result: &Result<(), PmCompleteLaneEnqueueError<T>>,
    ) {
        if let Err(error) = result
            && let Some(action) = error.action()
        {
            self.latch_failure(lane, action);
        }
    }

    fn observe_service_error(&mut self, error: PmCompleteLaneCheckError) -> PmCompleteServiceError {
        match error {
            PmCompleteLaneCheckError::Clock(source) => {
                self.latch_failure(PmLaneKind::Critical, SaturationAction::GlobalStop);
                PmCompleteServiceError::DeliveryClock(source)
            }
            PmCompleteLaneCheckError::EventClock(source) => {
                self.latch_failure(PmLaneKind::Critical, SaturationAction::GlobalStop);
                PmCompleteServiceError::EventClock(source)
            }
            PmCompleteLaneCheckError::Aged(fault) => {
                let _ = (fault.key(), fault.observed_age_ns(), fault.maximum_age_ns());
                self.latch_failure(fault.lane(), fault.action());
                if matches!(
                    fault.action(),
                    SaturationAction::HaltAccountAndRequireReconciliation
                        | SaturationAction::KeepUnreadyAndRetry
                ) {
                    let rank = usize::from(
                        fault
                            .lane()
                            .service_priority_rank()
                            .expect("aged complete lane has a scheduler rank"),
                    );
                    self.recoverable_aged_drain_remaining[rank] = match fault.lane() {
                        PmLaneKind::Private => self.private.len(),
                        PmLaneKind::Reconciliation => self.reconciliation.len(),
                        _ => 0,
                    };
                }
                PmCompleteServiceError::Aged(fault)
            }
        }
    }

    fn observe_schedule_error(&mut self, error: PmScheduleError) -> PmCompleteServiceError {
        if let PmScheduleError::Aged { action, .. } | PmScheduleError::Full { action, .. } = error {
            self.latch_failure(PmLaneKind::Scheduled, action);
        }
        PmCompleteServiceError::Schedule(error)
    }

    fn latch_failure(&mut self, lane: PmLaneKind, action: SaturationAction) {
        let rank = usize::from(
            lane.service_priority_rank()
                .expect("only complete input ranks latch failures"),
        );
        if self.failure_latched[rank] {
            return;
        }
        self.failure_latched[rank] = true;
        self.fail_closed.latch(action);
    }
}

fn service_lane<T>(
    lane: &mut PmCompleteLane<T>,
    monotonic_now_ns: u64,
    transfer_in_flight: &mut bool,
    mut recoverable_aged_drain_remaining: Option<&mut usize>,
    mut consume: impl FnMut(PmCompleteServiced<T>) -> bool,
) -> Result<usize, PmCompleteLaneCheckError> {
    let limit = PmLanePolicy::for_lane(lane.lane())
        .service_burst()
        .expect("complete input lane has a fixed burst");
    let count = limit.min(lane.len());
    let mut serviced = 0;
    for _ in 0..count {
        match lane.check_age(monotonic_now_ns) {
            Err(PmCompleteLaneCheckError::Aged(_))
                if recoverable_aged_drain_remaining
                    .as_deref()
                    .is_some_and(|remaining| *remaining != 0) => {}
            result => result?,
        }
        let item = lane.pop().expect("bounded count proves a queued item");
        if let Some(remaining) = recoverable_aged_drain_remaining.as_deref_mut() {
            *remaining = remaining.saturating_sub(1);
        }
        let serviced_item = item
            .into_serviced(lane.lane(), monotonic_now_ns)
            .map_err(PmCompleteLaneCheckError::EventClock)?;
        *transfer_in_flight = true;
        let stop = consume(serviced_item);
        *transfer_in_flight = false;
        serviced += 1;
        if stop {
            break;
        }
    }
    Ok(serviced)
}

#[allow(
    clippy::large_enum_variant,
    reason = "scheduler failures preserve exact bounded evidence without heap allocation"
)]
#[derive(Debug, Error)]
pub(crate) enum PmCompleteServiceError {
    #[error("complete scheduler consumer transfer is poisoned")]
    ConsumerTransferPoisoned,
    #[error("critical scheduler burst exhausted with {remaining} items pending")]
    CriticalBurstExhausted { remaining: usize },
    #[error("complete scheduler delivery clock failed: {0}")]
    DeliveryClock(#[from] reap_transport::DeliveryClockError),
    #[error("complete scheduler event clock failed: {0}")]
    EventClock(#[from] reap_pm_core::EnvelopeError),
    #[error("complete scheduler scheduled action failed: {0:?}")]
    Schedule(PmScheduleError),
    #[error("complete scheduler public lane failed: {0}")]
    Public(PmServiceTurnError),
    #[error("complete scheduler internal aged evidence")]
    Aged(PmCompleteLaneAgeFault),
}

impl PmCompleteServiceError {
    pub(crate) const fn action(&self) -> Option<SaturationAction> {
        match self {
            Self::Aged(fault) => Some(fault.action()),
            Self::Public(PmServiceTurnError::Aged(failure)) => Some(failure.action()),
            Self::Schedule(
                PmScheduleError::Full { action, .. } | PmScheduleError::Aged { action, .. },
            ) => Some(*action),
            Self::ConsumerTransferPoisoned
            | Self::CriticalBurstExhausted { .. }
            | Self::DeliveryClock(_)
            | Self::EventClock(_)
            | Self::Schedule(
                PmScheduleError::ZeroMonotonicTimestamp
                | PmScheduleError::ZeroWallTimestamp
                | PmScheduleError::LocalActionSequenceExhausted
                | PmScheduleError::InstrumentMismatch { .. }
                | PmScheduleError::ClockRegression { .. },
            )
            | Self::Public(_) => Some(SaturationAction::GlobalStop),
        }
    }
}

const fn action_index(action: SaturationAction) -> usize {
    match action {
        SaturationAction::GlobalStop => 0,
        SaturationAction::HaltAccountAndRequireReconciliation => 1,
        SaturationAction::InvalidateStreamAndResync => 2,
        SaturationAction::KeepUnreadyAndRetry => 3,
        SaturationAction::RetainPendingRefresh => 4,
        SaturationAction::InvalidateCaptureAndResync => 5,
        SaturationAction::SuppressDispatchAndHaltQuotes => 6,
        SaturationAction::RejectEffectAndHaltQuotes => 7,
        SaturationAction::SuppressQuoteAndCancelOwned => 8,
        SaturationAction::CoalesceTelemetry => 9,
    }
}
