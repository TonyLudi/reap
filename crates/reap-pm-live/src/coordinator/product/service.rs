use reap_pm_core::{EventClock, PmOrderSide, PmProductSource};
use reap_pm_state::{PmPrivateExternalIngressFault, PmRiskHaltScope};
use reap_polymarket_adapter::{
    PmFakeCancelScript, PmFakePlaceScript, PmFixtureCompletionOccurrence, PmFixtureFeeEvidence,
};

use super::*;
use crate::lanes::{
    PmCompleteInputSource, PmCompleteLaneBuildError, PmCompleteLaneEnqueueError,
    PmCompleteLaneService, PmCompleteServiceCounts, PmCompleteServiceError, PmPrivateInput,
    PmPublicLaneService, PmReconciliationInput, PmStopControl, PmTelemetryInput,
};
use crate::private_monitor::{
    PmAccountFixtureInput, PmOpenOrdersFixtureInput, PmOrderDetailFixtureInput,
    PmReconciliationFixtureInput,
};
use crate::public_routes::{OkxPublicUnavailable, PmPublicUnavailable};
use crate::schedule::{PmScheduleAdmission, PmScheduleError};

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum PublicUnavailableSource {
    Polymarket,
    Okx,
}

impl<M: PmQuoteModel> PmCoordinator<M> {
    pub(super) fn handle_risk_rejection(
        &mut self,
        timer: PmTimerInput,
        halt: PmRiskHaltScope,
        effects: &mut PmProductEffectBatch,
    ) -> Result<(), PmCoordinatorError> {
        self.record_suppression(PmQuoteSuppression::RiskRejected, effects)?;
        if halt.cancel_owned_required() {
            self.cancel_tracked_owned_orders(
                timer,
                PmJournalCancelReasonV1::SafetyHalt,
                PmControlReason::RiskLimit,
                effects,
            )?;
        }
        Ok(())
    }

    pub(crate) fn scheduler_metrics(
        &mut self,
        monotonic_now_ns: u64,
    ) -> Result<crate::lanes::PmCompleteSchedulerMetrics, PmCoordinatorError> {
        if monotonic_now_ns == 0 {
            return Err(PmCoordinatorError::ZeroServiceClock);
        }
        let result = self.lanes_mut()?.metrics(monotonic_now_ns);
        match result {
            Ok(metrics) => Ok(metrics),
            Err(error) => {
                self.latch_scheduler_failure(PmControlReason::SchedulerOverload);
                Err(error.into())
            }
        }
    }

    pub(crate) fn service_turn(
        &mut self,
        monotonic_now_ns: u64,
    ) -> Result<PmCompleteServiceCounts, PmCoordinatorError> {
        if monotonic_now_ns == 0 {
            return Err(PmCoordinatorError::ZeroServiceClock);
        }
        self.flush_durable_consequence_outputs()?;
        if self.mutation.pending_durable_consequences() != 0 {
            return Ok(PmCompleteServiceCounts::default());
        }
        self.outputs
            .ensure_capacity(MAX_PM_EFFECTS_PER_INPUT)
            .map_err(PmCoordinatorError::from)?;
        let mut lanes = self
            .lanes
            .take()
            .ok_or(PmCoordinatorError::SchedulerReentrant)?;
        let serviced = lanes.service_turn(monotonic_now_ns, self);
        let retry_result = if serviced.is_ok() && self.callback_error.is_none() {
            self.retry_retained_admissions(&mut lanes)
        } else {
            Ok(())
        };
        self.lanes = Some(lanes);

        let schedule_result = self.flush_pending_schedules();
        if let Some(error) = self.callback_error.take() {
            return Err(error);
        }
        let serviced = match serviced {
            Ok(serviced) => serviced,
            Err(error) => {
                self.latch_scheduler_failure(PmControlReason::SchedulerOverload);
                return Err(error.into());
            }
        };
        schedule_result?;
        retry_result?;
        Ok(serviced)
    }

    pub(crate) fn schedule(
        &mut self,
        side: PmOrderSide,
        kind: PmScheduledActionKind,
        deadline_ns: u64,
        scheduled_at_ns: u64,
        decision_wall_timestamp_ms: u64,
    ) -> Result<PmScheduleAdmission, PmCoordinatorError> {
        let key = PmScheduledActionKey::new(self.account_scope, self.instrument, side, kind);
        let result = self.lanes_mut()?.schedule(
            key,
            deadline_ns,
            scheduled_at_ns,
            decision_wall_timestamp_ms,
        );
        match result {
            Ok(admission) => Ok(admission),
            Err(error) => {
                self.latch_scheduler_failure(PmControlReason::SchedulerOverload);
                Err(error.into())
            }
        }
    }

    pub(crate) fn connect_private_fixture(
        &mut self,
        occurrence: PmFixtureCompletionOccurrence,
    ) -> Result<(), PmCoordinatorError> {
        let ingress = self.account_fixture_ingress(&occurrence);
        self.mutation
            .private_mut()
            .prepare_product_private_reconnect(occurrence.ordering().connection_epoch())?;
        self.enqueue_private(ingress, PmPrivateInput::ConnectionAvailable)
    }

    pub(crate) fn mark_private_fixture_unavailable(
        &mut self,
        occurrence: PmFixtureCompletionOccurrence,
        fault: PmPrivateExternalIngressFault,
    ) -> Result<(), PmCoordinatorError> {
        if self.mutation.private_mut().active_epoch()
            != Some(occurrence.ordering().connection_epoch())
        {
            return Err(crate::private_monitor::PmPrivateMonitorError::PrivateEpochMismatch.into());
        }
        let ingress = self.account_fixture_ingress(&occurrence);
        self.enqueue_private(ingress, PmPrivateInput::ConnectionUnavailable(fault))
    }

    pub(crate) fn ingest_private_fixture(
        &mut self,
        occurrence: PmFixtureCompletionOccurrence,
        raw: &[u8],
        fee: PmFixtureFeeEvidence,
    ) -> Result<(), PmCoordinatorError> {
        let delivery = self
            .mutation
            .private_mut()
            .receive_product_private_fixture(occurrence, raw, fee)?;
        let input = PmPrivateInput::FixtureBatch(delivery);
        let ingress = input
            .fixture_ingress()
            .expect("fixture batches retain exact product ingress");
        self.enqueue_private(ingress, input)
    }

    pub(crate) fn ingest_account_fixture(
        &mut self,
        input: PmAccountFixtureInput<'_>,
    ) -> Result<(), PmCoordinatorError> {
        let delivery = self
            .mutation
            .private_mut()
            .complete_product_account_fixture(input)?;
        let input = PmReconciliationInput::StandaloneAccountFixture(delivery);
        let ingress = input.fixture_ingress();
        self.enqueue_reconciliation(ingress, input)
    }

    pub(crate) fn ingest_open_orders_fixture(
        &mut self,
        input: PmOpenOrdersFixtureInput<'_>,
    ) -> Result<(), PmCoordinatorError> {
        let delivery = self
            .mutation
            .private_mut()
            .complete_product_open_orders_fixture(input)?;
        let input = PmReconciliationInput::OpenOrdersFixture(delivery);
        let ingress = input.fixture_ingress();
        self.enqueue_reconciliation(ingress, input)
    }

    pub(crate) fn ingest_order_detail_fixture(
        &mut self,
        input: PmOrderDetailFixtureInput<'_>,
    ) -> Result<(), PmCoordinatorError> {
        let delivery = self
            .mutation
            .private_mut()
            .complete_product_order_detail_fixture(input)?;
        let input = PmReconciliationInput::OrderDetailFixture(delivery);
        let ingress = input.fixture_ingress();
        self.enqueue_reconciliation(ingress, input)
    }

    pub(crate) fn ingest_reconciliation_fixture(
        &mut self,
        input: PmReconciliationFixtureInput<'_>,
    ) -> Result<(), PmCoordinatorError> {
        let delivery = self
            .mutation
            .private_mut()
            .complete_product_reconciliation_fixture(input)?;
        let input = PmReconciliationInput::PairedFixture(delivery);
        let ingress = input.fixture_ingress();
        self.enqueue_reconciliation(ingress, input)
    }

    pub(crate) fn request_shutdown(
        &mut self,
        occurrence: PmFixtureCompletionOccurrence,
    ) -> Result<(), PmCoordinatorError> {
        let ingress = self.internal_fixture_ingress(&occurrence);
        self.enqueue_critical(ingress, PmCriticalInput::Stop(PmStopControl::Shutdown))
    }

    pub(crate) fn request_global_stop(
        &mut self,
        occurrence: PmFixtureCompletionOccurrence,
    ) -> Result<(), PmCoordinatorError> {
        let ingress = self.internal_fixture_ingress(&occurrence);
        self.enqueue_critical(ingress, PmCriticalInput::Stop(PmStopControl::GlobalStop))
    }

    pub(crate) fn request_scoped_halt(
        &mut self,
        occurrence: PmFixtureCompletionOccurrence,
        scope: PmRiskHaltScope,
    ) -> Result<bool, PmCoordinatorError> {
        let Some(halt) = crate::lanes::PmScopedHalt::new(scope) else {
            return Ok(false);
        };
        let ingress = self.internal_fixture_ingress(&occurrence);
        self.enqueue_critical(ingress, PmCriticalInput::ScopedHalt(halt))?;
        Ok(true)
    }

    pub(crate) fn emit_telemetry(
        &mut self,
        occurrence: PmFixtureCompletionOccurrence,
        kind: crate::lanes::PmTelemetryKind,
        value: u64,
    ) -> Result<(), PmCoordinatorError> {
        let ingress = self.internal_fixture_ingress(&occurrence);
        self.enqueue_telemetry(ingress, PmTelemetryInput::new(kind, value))
    }

    pub(crate) fn poll_persistence_fixture(
        &mut self,
        occurrence: PmFixtureCompletionOccurrence,
        monotonic_poll_ns: u64,
    ) -> Result<bool, PmCoordinatorError> {
        let ingress = self.internal_fixture_ingress(&occurrence);
        self.poll_persistence_into_lane(ingress, monotonic_poll_ns)
    }

    pub(crate) fn execute_prepared_quote_fixture(
        &mut self,
        occurrence: PmFixtureCompletionOccurrence,
        script: PmFakePlaceScript,
        monotonic_effect_ns: u64,
    ) -> Result<(), PmCoordinatorError> {
        let ingress = self.internal_fixture_ingress(&occurrence);
        self.execute_prepared_quote_into_lane(ingress, script, monotonic_effect_ns)
    }

    pub(crate) fn execute_prepared_cancel_fixture(
        &mut self,
        occurrence: PmFixtureCompletionOccurrence,
        script: PmFakeCancelScript,
        monotonic_effect_ns: u64,
    ) -> Result<(), PmCoordinatorError> {
        let ingress = self.internal_fixture_ingress(&occurrence);
        self.execute_prepared_cancel_into_lane(ingress, script, monotonic_effect_ns)
    }

    pub(crate) fn poll_persistence_into_lane(
        &mut self,
        ingress: PmCompleteIngress,
        monotonic_poll_ns: u64,
    ) -> Result<bool, PmCoordinatorError> {
        if self.retained_persistence.is_some() {
            return Err(PmCoordinatorError::PersistenceAdmissionRetained);
        }
        let poll = self.mutation.poll_persistence(monotonic_poll_ns)?;
        let input = match PmLanePersistenceInput::from_poll(poll) {
            Ok(input) => input,
            Err(crate::lanes::PmPersistenceCarrierError::Empty) => return Ok(false),
            Err(crate::lanes::PmPersistenceCarrierError::Pending) => return Ok(false),
        };
        match self.lanes_mut()?.enqueue_persistence(ingress, input) {
            Ok(()) => Ok(true),
            Err(error) => {
                self.retained_persistence = retain_retryable_admission(ingress, error);
                self.latch_scheduler_failure(PmControlReason::SchedulerOverload);
                Err(PmCoordinatorError::PersistenceLaneRejected)
            }
        }
    }

    pub(crate) fn execute_prepared_quote_into_lane(
        &mut self,
        ingress: PmCompleteIngress,
        script: PmFakePlaceScript,
        monotonic_effect_ns: u64,
    ) -> Result<(), PmCoordinatorError> {
        if self.retained_critical.is_some() {
            return Err(PmCoordinatorError::CriticalAdmissionRetained);
        }
        let mut lanes = self
            .lanes
            .take()
            .ok_or(PmCoordinatorError::SchedulerReentrant)?;
        let result = lanes.enqueue_built_critical(
            ingress,
            PmCriticalInput::fake_place_result_rank(),
            || {
                self.mutation
                    .execute_next_quote_to_result(script, monotonic_effect_ns)
                    .map(PmCriticalInput::FakePlaceResult)
            },
        );
        self.lanes = Some(lanes);
        match result {
            Ok(()) => Ok(()),
            Err(PmCompleteLaneBuildError::Build(error)) => Err(error.into()),
            Err(
                PmCompleteLaneBuildError::WrongSource { .. }
                | PmCompleteLaneBuildError::Duplicate { .. }
                | PmCompleteLaneBuildError::Full { .. },
            ) => {
                self.latch_scheduler_failure(PmControlReason::SchedulerOverload);
                Err(PmCoordinatorError::CriticalLaneRejected)
            }
        }
    }

    pub(crate) fn execute_prepared_cancel_into_lane(
        &mut self,
        ingress: PmCompleteIngress,
        script: PmFakeCancelScript,
        monotonic_effect_ns: u64,
    ) -> Result<(), PmCoordinatorError> {
        if self.retained_critical.is_some() {
            return Err(PmCoordinatorError::CriticalAdmissionRetained);
        }
        let mut lanes = self
            .lanes
            .take()
            .ok_or(PmCoordinatorError::SchedulerReentrant)?;
        let result = lanes.enqueue_built_critical(
            ingress,
            PmCriticalInput::fake_cancel_result_rank(),
            || {
                self.mutation
                    .execute_next_cancel_to_result(script, monotonic_effect_ns)
                    .map(PmCriticalInput::FakeCancelResult)
            },
        );
        self.lanes = Some(lanes);
        match result {
            Ok(()) => Ok(()),
            Err(PmCompleteLaneBuildError::Build(error)) => Err(error.into()),
            Err(
                PmCompleteLaneBuildError::WrongSource { .. }
                | PmCompleteLaneBuildError::Duplicate { .. }
                | PmCompleteLaneBuildError::Full { .. },
            ) => {
                self.latch_scheduler_failure(PmControlReason::SchedulerOverload);
                Err(PmCoordinatorError::CriticalLaneRejected)
            }
        }
    }

    pub(crate) fn enqueue_critical(
        &mut self,
        ingress: PmCompleteIngress,
        input: PmCriticalInput,
    ) -> Result<(), PmCoordinatorError> {
        if self.retained_critical.is_some() {
            return Err(PmCoordinatorError::CriticalAdmissionRetained);
        }
        match self.lanes_mut()?.enqueue_critical(ingress, input) {
            Ok(()) => Ok(()),
            Err(error) => {
                self.retained_critical = retain_retryable_admission(ingress, error);
                self.latch_scheduler_failure(PmControlReason::SchedulerOverload);
                Err(PmCoordinatorError::CriticalLaneRejected)
            }
        }
    }

    pub(crate) fn enqueue_private(
        &mut self,
        ingress: PmCompleteIngress,
        input: PmPrivateInput,
    ) -> Result<(), PmCoordinatorError> {
        if self.retained_private_admission.is_some() {
            return Err(PmCoordinatorError::PrivateAdmissionRetained);
        }
        match self.lanes_mut()?.enqueue_private(ingress, input) {
            Ok(()) => Ok(()),
            Err(error) => {
                self.retained_private_admission = retain_retryable_admission(ingress, error);
                self.latch_scheduler_failure(PmControlReason::SchedulerOverload);
                Err(PmCoordinatorError::PrivateLaneRejected)
            }
        }
    }

    pub(crate) fn enqueue_reconciliation(
        &mut self,
        ingress: PmCompleteIngress,
        input: PmReconciliationInput,
    ) -> Result<(), PmCoordinatorError> {
        if self.retained_reconciliation_admission.is_some() {
            return Err(PmCoordinatorError::ReconciliationAdmissionRetained);
        }
        match self.lanes_mut()?.enqueue_reconciliation(ingress, input) {
            Ok(()) => Ok(()),
            Err(error) => {
                self.retained_reconciliation_admission = retain_retryable_admission(ingress, error);
                self.latch_scheduler_failure(PmControlReason::SchedulerOverload);
                Err(PmCoordinatorError::ReconciliationLaneRejected)
            }
        }
    }

    pub(crate) fn enqueue_telemetry(
        &mut self,
        ingress: PmCompleteIngress,
        input: PmTelemetryInput,
    ) -> Result<(), PmCoordinatorError> {
        match self.lanes_mut()?.enqueue_telemetry(ingress, input) {
            Ok(()) => Ok(()),
            Err(_) => {
                self.latch_scheduler_failure(PmControlReason::ContractViolation);
                Err(PmCoordinatorError::TelemetryLaneRejected)
            }
        }
    }

    pub(crate) fn pop_effect(&mut self) -> Option<PmProductEffect> {
        self.outputs.pop()
    }

    pub(crate) const fn pending_effect_outputs(&self) -> usize {
        self.outputs.len()
    }

    fn account_fixture_ingress(
        &self,
        occurrence: &PmFixtureCompletionOccurrence,
    ) -> PmCompleteIngress {
        PmCompleteIngress::product(
            self.account_source,
            self.account_connection,
            occurrence.ordering(),
            occurrence.received_clock(),
        )
    }

    fn internal_fixture_ingress(
        &self,
        occurrence: &PmFixtureCompletionOccurrence,
    ) -> PmCompleteIngress {
        PmCompleteIngress::internal(
            self.account_source.source(),
            self.account_connection,
            occurrence.ordering(),
            occurrence.received_clock(),
        )
    }

    fn lanes_mut(&mut self) -> Result<&mut PmCompleteInputLanes, PmCoordinatorError> {
        self.lanes
            .as_mut()
            .ok_or(PmCoordinatorError::SchedulerReentrant)
    }

    fn retry_retained_admissions(
        &mut self,
        lanes: &mut PmCompleteInputLanes,
    ) -> Result<(), PmCoordinatorError> {
        if let Some(retained) = self.retained_critical.take()
            && let Err(error) = lanes.enqueue_critical(retained.ingress, retained.input)
        {
            self.retained_critical = retain_retryable_admission(retained.ingress, error);
            return Err(PmCoordinatorError::CriticalLaneRejected);
        }
        if let Some(retained) = self.retained_persistence.take()
            && let Err(error) = lanes.enqueue_persistence(retained.ingress, retained.input)
        {
            self.retained_persistence = retain_retryable_admission(retained.ingress, error);
            return Err(PmCoordinatorError::PersistenceLaneRejected);
        }
        if let Some(retained) = self.retained_private_admission.take()
            && let Err(error) = lanes.enqueue_private(retained.ingress, retained.input)
        {
            self.retained_private_admission = retain_retryable_admission(retained.ingress, error);
            return Err(PmCoordinatorError::PrivateLaneRejected);
        }
        if let Some(retained) = self.retained_reconciliation_admission.take()
            && let Err(error) = lanes.enqueue_reconciliation(retained.ingress, retained.input)
        {
            self.retained_reconciliation_admission =
                retain_retryable_admission(retained.ingress, error);
            return Err(PmCoordinatorError::ReconciliationLaneRejected);
        }
        Ok(())
    }

    fn flush_pending_schedules(&mut self) -> Result<(), PmCoordinatorError> {
        while let Some(pending) = self.pending_schedules.take_next() {
            let result = self.lanes_mut()?.schedule(
                pending.key,
                pending.deadline_ns,
                pending.scheduled_at_ns,
                pending.decision_wall_timestamp_ms,
            );
            if let Err(error) = result {
                self.pending_schedules.insert(
                    pending.key,
                    pending.deadline_ns,
                    pending.scheduled_at_ns,
                    pending.decision_wall_timestamp_ms,
                )?;
                self.latch_scheduler_failure(PmControlReason::SchedulerOverload);
                return Err(error.into());
            }
        }
        Ok(())
    }

    fn schedule_both_sides(
        &mut self,
        kind: PmScheduledActionKind,
        monotonic_ns: u64,
        decision_wall_timestamp_ms: u64,
    ) -> Result<(), PmCoordinatorError> {
        for side in [PmOrderSide::Buy, PmOrderSide::Sell] {
            self.pending_schedules.insert(
                PmScheduledActionKey::new(self.account_scope, self.instrument, side, kind),
                monotonic_ns,
                monotonic_ns,
                decision_wall_timestamp_ms,
            )?;
        }
        Ok(())
    }

    fn schedule_quote_evaluation(
        &mut self,
        monotonic_ns: u64,
        decision_wall_timestamp_ms: u64,
    ) -> Result<(), PmCoordinatorError> {
        self.pending_schedules.insert(
            PmScheduledActionKey::new(
                self.account_scope,
                self.instrument,
                PmOrderSide::Buy,
                PmScheduledActionKind::QuoteEvaluation,
            ),
            monotonic_ns,
            monotonic_ns,
            decision_wall_timestamp_ms,
        )
    }

    fn complete_callback(
        &mut self,
        mut effects: PmProductEffectBatch,
        result: Result<(), PmCoordinatorError>,
    ) {
        self.counters.inputs = self.counters.inputs.saturating_add(1);
        if let Err(error) = self.append_durable_consequences(&mut effects) {
            self.record_callback_error(error);
        }
        if let Err(error) = self.publish_effect_batch(effects) {
            self.record_callback_error(error);
        }
        if let Err(error) = result {
            self.record_callback_error(error);
        }
    }

    fn record_callback_error(&mut self, error: PmCoordinatorError) {
        if error.callback_requires_global_halt() {
            self.latch_halt(PmControlReason::ContractViolation);
        }
        if self.callback_error.is_none() {
            self.callback_error = Some(error);
        }
    }

    pub(super) fn latch_halt(&mut self, reason: PmControlReason) {
        if self.halt.is_some() {
            return;
        }
        self.halt = Some(reason);
        self.counters.control_halts = self.counters.control_halts.saturating_add(1);
    }

    fn latch_scheduler_failure(&mut self, reason: PmControlReason) {
        self.mutation.invalidate_revisions();
        self.mutation.halt_contract();
        self.latch_halt(reason);
    }

    fn append_durable_consequences(
        &mut self,
        effects: &mut PmProductEffectBatch,
    ) -> Result<(), PmCoordinatorError> {
        while effects.len() < MAX_PM_EFFECTS_PER_INPUT {
            let Some(consequence) = self.mutation.pop_durable_consequence() else {
                break;
            };
            effects.push(PmProductEffect::DurableRecord(PmDurableRecordEffect::new(
                consequence.kind(),
                consequence.client_order(),
                consequence.correlation(),
            )))?;
            self.counters.durable_record_effects =
                self.counters.durable_record_effects.saturating_add(1);
        }
        Ok(())
    }

    fn flush_durable_consequence_outputs(&mut self) -> Result<(), PmCoordinatorError> {
        while self.mutation.pending_durable_consequences() != 0
            && self.outputs.can_accept(MAX_PM_EFFECTS_PER_INPUT)
        {
            let mut effects = PmProductEffectBatch::new();
            self.append_durable_consequences(&mut effects)?;
            self.publish_effect_batch(effects)?;
        }
        Ok(())
    }

    fn invalidate_quote_authority_at(
        &mut self,
        monotonic_service_ns: u64,
    ) -> Result<(), PmCoordinatorError> {
        self.mutation.invalidate_revisions();
        for tracked in self.tracked_quotes.into_iter().flatten() {
            if tracked.stage != TrackedQuoteStage::PreparedLocal {
                continue;
            }
            if !self
                .mutation
                .invalidate_prepared_quote(tracked.client_order, monotonic_service_ns)?
            {
                return Err(PmCoordinatorError::PreparedQuoteAuthorityMismatch);
            }
            let _invalidated = self
                .prepared_correlations
                .remove_quote(tracked.client_order)?;
            self.clear_tracked_quote(tracked.client_order);
        }
        Ok(())
    }

    fn service_reference(
        &mut self,
        input: PmOkxReferenceInput,
        effects: &mut PmProductEffectBatch,
    ) -> Result<(), PmCoordinatorError> {
        if self.decision.observe_reference(input)? {
            self.invalidate_quote_authority_at(input.clock().monotonic_service_ns())?;
            self.counters.references_applied = self.counters.references_applied.saturating_add(1);
            push_metric(effects, PmHealthMetricKind::InputObserved, 1)?;
            let clock = input.clock();
            self.schedule_quote_evaluation(
                clock.monotonic_receive_ns(),
                captured_wall_timestamp_ms(clock.local_wall_receive_ns())?,
            )
        } else {
            self.counters.references_ignored = self.counters.references_ignored.saturating_add(1);
            push_metric(effects, PmHealthMetricKind::InputIgnoredStale, 1)
        }
    }

    fn service_market(
        &mut self,
        input: PmMarketInput,
        effects: &mut PmProductEffectBatch,
    ) -> Result<(), PmCoordinatorError> {
        let clock = input.clock();
        let tradable = market_is_tradable(input.event().metadata());
        if self.decision.observe_market(input)? {
            self.invalidate_quote_authority_at(clock.monotonic_service_ns())?;
            self.counters.markets_applied = self.counters.markets_applied.saturating_add(1);
            let decision_wall_timestamp_ms =
                captured_wall_timestamp_ms(clock.local_wall_receive_ns())?;
            if tradable {
                self.schedule_quote_evaluation(
                    clock.monotonic_receive_ns(),
                    decision_wall_timestamp_ms,
                )?;
            } else {
                effects.push(PmProductEffect::FailClosedHaltOrCancel(
                    PmFailClosedEffect::halt(
                        self.account_scope,
                        self.instrument,
                        PmControlReason::PublicUnavailable,
                    ),
                ))?;
                self.schedule_both_sides(
                    PmScheduledActionKind::CancelOwnedQuote,
                    clock.monotonic_receive_ns(),
                    decision_wall_timestamp_ms,
                )?;
            }
        }
        push_metric(effects, PmHealthMetricKind::InputObserved, 1)
    }

    fn service_book(
        &mut self,
        input: PmBookInput,
        effects: &mut PmProductEffectBatch,
    ) -> Result<(), PmCoordinatorError> {
        let clock = input.clock();
        if self.decision.observe_book(input)? {
            self.invalidate_quote_authority_at(clock.monotonic_service_ns())?;
            self.counters.books_applied = self.counters.books_applied.saturating_add(1);
            self.schedule_quote_evaluation(
                clock.monotonic_receive_ns(),
                captured_wall_timestamp_ms(clock.local_wall_receive_ns())?,
            )?;
        }
        push_metric(effects, PmHealthMetricKind::InputObserved, 1)
    }

    fn service_public_unavailable(
        &mut self,
        clock: EventClock,
        source: PublicUnavailableSource,
        effects: &mut PmProductEffectBatch,
    ) -> Result<(), PmCoordinatorError> {
        match source {
            PublicUnavailableSource::Polymarket => self.decision.invalidate_pm_public(),
            PublicUnavailableSource::Okx => self.decision.invalidate_okx_public(),
        }
        self.invalidate_quote_authority_at(clock.monotonic_service_ns())?;
        effects.push(PmProductEffect::FailClosedHaltOrCancel(
            PmFailClosedEffect::halt(
                self.account_scope,
                self.instrument,
                PmControlReason::PublicUnavailable,
            ),
        ))?;
        self.schedule_both_sides(
            PmScheduledActionKind::CancelOwnedQuote,
            clock.monotonic_receive_ns(),
            captured_wall_timestamp_ms(clock.local_wall_receive_ns())?,
        )
    }

    fn service_critical(
        &mut self,
        input: PmCriticalInput,
        monotonic_service_ns: u64,
        effects: &mut PmProductEffectBatch,
    ) -> Result<(), PmCoordinatorError> {
        match input {
            PmCriticalInput::Stop(PmStopControl::Shutdown) => {
                self.invalidate_quote_authority_at(monotonic_service_ns)?;
                self.latch_halt(PmControlReason::RequestedShutdown);
                Ok(())
            }
            PmCriticalInput::Stop(PmStopControl::GlobalStop) => {
                self.invalidate_quote_authority_at(monotonic_service_ns)?;
                self.latch_halt(PmControlReason::ContractViolation);
                Ok(())
            }
            PmCriticalInput::ScopedHalt(halt) => {
                let _scope = halt.scope();
                self.invalidate_quote_authority_at(monotonic_service_ns)?;
                self.latch_halt(PmControlReason::ContractViolation);
                Ok(())
            }
            PmCriticalInput::FakePlaceResult(result) => {
                let prepared = self
                    .prepared_correlations
                    .remove_quote(result.client_order())?;
                self.mutation
                    .reduce_serviced_fake_place(result, monotonic_service_ns)?;
                self.advance_private_readiness_revision()?;
                let executed = PmFakeQuoteEffect::new(
                    prepared.account_scope(),
                    prepared.instrument(),
                    prepared.client_order(),
                    prepared.side(),
                    prepared.price(),
                    prepared.quantity(),
                    PmFakeEffectStage::ExecutedByFixture,
                );
                effects.push(PmProductEffect::FakePassiveQuote(executed))?;
                self.refresh_tracked_quote(prepared.client_order());
                self.counters.fake_quote_effects =
                    self.counters.fake_quote_effects.saturating_add(1);
                push_metric(effects, PmHealthMetricKind::FakeEffectExecuted, 1)
            }
            PmCriticalInput::FakeCancelResult(result) => {
                let prepared = self
                    .prepared_correlations
                    .remove_cancel(result.client_order(), result.venue_order())?;
                self.mutation
                    .reduce_serviced_fake_cancel(result, monotonic_service_ns)?;
                self.advance_private_readiness_revision()?;
                let executed = PmFakeCancelEffect::new(
                    prepared.account_scope(),
                    prepared.instrument(),
                    prepared.client_order(),
                    prepared.venue_order(),
                    PmFakeEffectStage::ExecutedByFixture,
                );
                effects.push(PmProductEffect::FakeCancelOwned(executed))?;
                self.refresh_tracked_quote(prepared.client_order());
                self.counters.fake_cancel_effects =
                    self.counters.fake_cancel_effects.saturating_add(1);
                push_metric(effects, PmHealthMetricKind::FakeEffectExecuted, 1)
            }
        }
    }

    fn service_private(
        &mut self,
        item: PmCompleteServiced<PmPrivateInput>,
        effects: &mut PmProductEffectBatch,
    ) -> Result<(), PmCoordinatorError> {
        let source = product_source(item.source())?;
        let connection = item.connection();
        let ordering = item.ordering();
        let clock = item.clock();
        let input = item.into_value();
        self.invalidate_quote_authority_at(clock.monotonic_service_ns())?;
        match input {
            PmPrivateInput::ConnectionAvailable => {
                self.mutation
                    .private_mut()
                    .reduce_serviced_connection_available(
                        source,
                        connection,
                        ordering.connection_epoch(),
                        clock.monotonic_receive_ns(),
                    )?;
                self.advance_private_readiness_revision()?;
                self.schedule_quote_evaluation(
                    clock.monotonic_receive_ns(),
                    captured_wall_timestamp_ms(clock.local_wall_receive_ns())?,
                )?;
                push_metric(effects, PmHealthMetricKind::InputObserved, 1)
            }
            PmPrivateInput::ConnectionUnavailable(fault) => {
                self.mutation
                    .private_mut()
                    .reduce_serviced_connection_unavailable(source, connection, fault)?;
                self.advance_private_readiness_revision()?;
                effects.push(PmProductEffect::FailClosedHaltOrCancel(
                    PmFailClosedEffect::halt(
                        self.account_scope,
                        self.instrument,
                        PmControlReason::PrivateUnavailable,
                    ),
                ))?;
                self.schedule_both_sides(
                    PmScheduledActionKind::CancelOwnedQuote,
                    clock.monotonic_receive_ns(),
                    captured_wall_timestamp_ms(clock.local_wall_receive_ns())?,
                )?;
                push_metric(effects, PmHealthMetricKind::InputObserved, 1)
            }
            PmPrivateInput::FixtureBatch(delivery) => {
                let _observations = self
                    .mutation
                    .reduce_serviced_private_fixture(delivery, clock.monotonic_service_ns())?;
                self.advance_private_readiness_revision()?;
                push_metric(effects, PmHealthMetricKind::InputObserved, 1)
            }
        }
    }

    fn service_reconciliation(
        &mut self,
        item: PmCompleteServiced<PmReconciliationInput>,
        effects: &mut PmProductEffectBatch,
    ) -> Result<(), PmCoordinatorError> {
        let monotonic_service_ns = item.clock().monotonic_service_ns();
        product_source(item.source())?;
        let clock = item.clock();
        self.invalidate_quote_authority_at(clock.monotonic_service_ns())?;
        match item.into_value() {
            PmReconciliationInput::OpenOrdersFixture(delivery) => {
                let envelope = self
                    .mutation
                    .private_mut()
                    .open_product_open_orders_fixture(delivery, monotonic_service_ns)?;
                let source = envelope.source();
                let connection = envelope.connection_id();
                let clock = envelope.clock();
                let ordering = envelope.ordering();
                self.mutation.private_mut().reduce_serviced_open_orders(
                    source,
                    connection,
                    clock,
                    ordering,
                    envelope.into_payload(),
                )?;
            }
            PmReconciliationInput::OrderDetailFixture(delivery) => {
                let envelope = self
                    .mutation
                    .private_mut()
                    .open_product_order_detail_fixture(delivery, monotonic_service_ns)?;
                let source = envelope.source();
                let connection = envelope.connection_id();
                let clock = envelope.clock();
                let ordering = envelope.ordering();
                self.mutation.private_mut().reduce_serviced_order_detail(
                    source,
                    connection,
                    clock,
                    ordering,
                    envelope.into_payload(),
                )?;
            }
            PmReconciliationInput::StandaloneAccountFixture(delivery) => {
                let envelope = self
                    .mutation
                    .private_mut()
                    .open_product_account_fixture(delivery, monotonic_service_ns)?;
                let source = envelope.source();
                let connection = envelope.connection_id();
                let clock = envelope.clock();
                let ordering = envelope.ordering();
                self.mutation
                    .private_mut()
                    .reduce_serviced_account_snapshot(
                        source,
                        connection,
                        clock,
                        ordering,
                        envelope.into_payload(),
                    )?;
            }
            PmReconciliationInput::PairedFixture(delivery) => {
                let (account, fills) = self
                    .mutation
                    .private_mut()
                    .open_product_reconciliation_fixture(delivery, monotonic_service_ns)?;
                let unique_before = self.mutation.counters().unique_fills();
                let _apply = self
                    .mutation
                    .reduce_serviced_reconciliation(account, fills)?;
                if self.mutation.counters().unique_fills() != unique_before {
                    for tracked in self.tracked_quotes.into_iter().flatten() {
                        self.refresh_tracked_quote(tracked.client_order);
                    }
                }
            }
        }
        self.advance_private_readiness_revision()?;
        push_metric(effects, PmHealthMetricKind::InputObserved, 1)
    }

    fn advance_private_readiness_revision(&mut self) -> Result<(), PmCoordinatorError> {
        self.private_readiness_revision = self
            .private_readiness_revision
            .checked_add(1)
            .ok_or(PmCoordinatorError::RevisionExhausted)?;
        Ok(())
    }
}

impl<M: PmQuoteModel> PmCompleteLaneService for PmCoordinator<M> {
    fn stop_complete_service_turn(&self) -> bool {
        self.callback_error.is_some()
            || self.mutation.pending_durable_consequences() != 0
            || !self.outputs.can_accept(MAX_PM_EFFECTS_PER_INPUT)
    }

    fn on_critical(&mut self, item: PmCompleteServiced<PmCriticalInput>) {
        debug_assert_eq!(item.lane(), crate::lanes::PmLaneKind::Critical);
        let _key = item.key();
        let monotonic_service_ns = item.clock().monotonic_service_ns();
        let input = item.into_value();
        let mut effects = PmProductEffectBatch::new();
        let result = self.service_critical(input, monotonic_service_ns, &mut effects);
        self.complete_callback(effects, result);
    }

    fn on_persistence(&mut self, item: PmCompleteServiced<PmLanePersistenceInput>) {
        debug_assert_eq!(item.lane(), crate::lanes::PmLaneKind::Persistence);
        let _key = item.key();
        let monotonic_service_ns = item.clock().monotonic_service_ns();
        let poll = item.into_value().into_poll();
        let mut effects = PmProductEffectBatch::new();
        let result = self
            .mutation
            .reduce_persistence_poll(poll, monotonic_service_ns)
            .map_err(PmCoordinatorError::from)
            .and_then(|service| self.record_persistence_service(service, &mut effects));
        self.complete_callback(effects, result);
    }

    fn on_private(&mut self, item: PmCompleteServiced<PmPrivateInput>) {
        debug_assert_eq!(item.lane(), crate::lanes::PmLaneKind::Private);
        let _key = item.key();
        let mut effects = PmProductEffectBatch::new();
        let result = self.service_private(item, &mut effects);
        self.complete_callback(effects, result);
    }

    fn on_scheduled(&mut self, item: crate::schedule::PmDueScheduledAction) {
        let mut effects = PmProductEffectBatch::new();
        let result = PmTimerInput::from_due(item)
            .map_err(PmCoordinatorError::from)
            .and_then(|timer| self.service_timer(timer, &mut effects));
        self.complete_callback(effects, result);
    }

    fn on_reconciliation(&mut self, item: PmCompleteServiced<PmReconciliationInput>) {
        debug_assert_eq!(item.lane(), crate::lanes::PmLaneKind::Reconciliation);
        let _key = item.key();
        let mut effects = PmProductEffectBatch::new();
        let result = self.service_reconciliation(item, &mut effects);
        self.complete_callback(effects, result);
    }

    fn on_telemetry(&mut self, item: PmCompleteServiced<PmTelemetryInput>) {
        debug_assert_eq!(item.lane(), crate::lanes::PmLaneKind::Telemetry);
        let _key = item.key();
        let input = item.into_value();
        let _kind = input.kind();
        let value = input.value();
        let mut effects = PmProductEffectBatch::new();
        let result = push_metric(&mut effects, PmHealthMetricKind::InputObserved, value);
        self.complete_callback(effects, result);
    }
}

impl<M: PmQuoteModel> PmPublicLaneService for PmCoordinator<M> {
    fn stop_public_service_turn(&self) -> bool {
        self.callback_error.is_some()
            || self.mutation.pending_durable_consequences() != 0
            || !self.outputs.can_accept(MAX_PM_EFFECTS_PER_INPUT)
    }

    fn on_pm_public_unavailable(
        &mut self,
        item: crate::lanes::ServicedLaneItem<PmPublicUnavailable>,
    ) {
        let clock = item.clock();
        let mut effects = PmProductEffectBatch::new();
        let result = self.service_public_unavailable(
            clock,
            PublicUnavailableSource::Polymarket,
            &mut effects,
        );
        self.complete_callback(effects, result);
    }

    fn on_okx_public_unavailable(
        &mut self,
        item: crate::lanes::ServicedLaneItem<OkxPublicUnavailable>,
    ) {
        let clock = item.clock();
        let mut effects = PmProductEffectBatch::new();
        let result =
            self.service_public_unavailable(clock, PublicUnavailableSource::Okx, &mut effects);
        self.complete_callback(effects, result);
    }

    fn on_market(&mut self, item: crate::lanes::ServicedLaneItem<reap_pm_core::PmMarketEvent>) {
        let mut effects = PmProductEffectBatch::new();
        let result = PmMarketInput::from_serviced(item)
            .map_err(PmCoordinatorError::from)
            .and_then(|input| self.service_market(input, &mut effects));
        self.complete_callback(effects, result);
    }

    fn on_book(&mut self, _item: crate::lanes::ServicedLaneItem<reap_pm_core::PmBookEvent>) {
        let effects = PmProductEffectBatch::new();
        self.complete_callback(
            effects,
            Err(PmCoordinatorError::MissingCanonicalBookProjection),
        );
    }

    fn on_reduced_book(
        &mut self,
        item: crate::lanes::ServicedLaneItem<reap_pm_core::PmBookEvent>,
        projection: super::super::input::PmBookDecisionProjection,
    ) {
        let mut effects = PmProductEffectBatch::new();
        let result = PmBookInput::from_serviced(item, projection)
            .map_err(PmCoordinatorError::from)
            .and_then(|input| self.service_book(input, &mut effects));
        self.complete_callback(effects, result);
    }

    fn on_reference(
        &mut self,
        item: crate::lanes::ServicedLaneItem<reap_pm_core::OkxReferenceEvent>,
    ) {
        let mut effects = PmProductEffectBatch::new();
        let result = PmOkxReferenceInput::from_serviced(item)
            .map_err(PmCoordinatorError::from)
            .and_then(|input| self.service_reference(input, &mut effects));
        self.complete_callback(effects, result);
    }
}

fn product_source(source: PmCompleteInputSource) -> Result<PmProductSource, PmCoordinatorError> {
    match source {
        PmCompleteInputSource::Product(source) => Ok(source),
        PmCompleteInputSource::Internal(_) => Err(PmCoordinatorError::AccountSourceRequired),
    }
}

fn captured_wall_timestamp_ms(local_wall_receive_ns: u64) -> Result<u64, PmCoordinatorError> {
    let timestamp_ms = local_wall_receive_ns / 1_000_000;
    if timestamp_ms == 0 {
        Err(PmCoordinatorError::InvalidCapturedWallTimestamp)
    } else {
        Ok(timestamp_ms)
    }
}

fn retain_retryable_admission<T>(
    ingress: PmCompleteIngress,
    error: PmCompleteLaneEnqueueError<T>,
) -> Option<RetainedLaneInput<T>> {
    if matches!(&error, PmCompleteLaneEnqueueError::Full { .. }) {
        Some(RetainedLaneInput {
            ingress,
            input: error.into_input(),
        })
    } else {
        // Wrong-source and duplicate values are terminally consumed after the
        // coordinator latches fail-closed evidence. Retaining them would make
        // every later service turn fail without a possible state transition.
        drop(error.into_input());
        None
    }
}

impl From<PmScheduleError> for PmCoordinatorError {
    fn from(error: PmScheduleError) -> Self {
        Self::Schedule(error)
    }
}

impl From<PmCompleteServiceError> for PmCoordinatorError {
    fn from(error: PmCompleteServiceError) -> Self {
        Self::CompleteScheduler(error)
    }
}
