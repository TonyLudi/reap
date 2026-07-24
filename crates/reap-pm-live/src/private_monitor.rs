use reap_pm_core::{
    ConnectionEpoch, EnvelopeError, EventClock, EventEnvelope, EventOrdering, IngressSequence,
    PmAccountScope, PmAggregateError, PmClientOrderKey, PmCompleteAccountSnapshot,
    PmCompleteFillQuery, PmCompleteOpenOrdersSnapshot, PmConnectionId, PmExactOrderDetail,
    PmFillEvent, PmFillExecution, PmFillKey, PmFillQueryCursor, PmOrderEvent, PmOrderIdentity,
    PmOrderSide, PmProductSource, PmReconciliationRequestBoundary, PmSnapshotEvidence, PmSpenderId,
    PmVenueOrderKey, U256,
};
use reap_pm_live_contracts::{
    ConstructedRoleBinding, PmAccountConnectivityConfig, PmConnectionRoute, PmConnectivityPlan,
    PmPlanError, PmRoleKind,
};
use reap_pm_state::{
    PmAccountCounters, PmAccountSnapshotApply, PmAccountSnapshotProjection, PmAllowanceKnowledge,
    PmExactReservation, PmFillApply, PmFillCounters, PmFillProjection, PmOpenOrderReservation,
    PmOpenOrdersApply, PmOrderApply, PmOrderCounters, PmOrderProjection, PmOwnedCancelApply,
    PmOwnedCancelIntent, PmOwnedCancelOutcome, PmOwnedCancelRequestApply, PmOwnedFillApply,
    PmOwnedImmediateAckTicket, PmOwnedOrderProgressObservation, PmOwnedOrderProjection,
    PmOwnedProgressApply, PmOwnedQuoteAdmission, PmOwnedQuoteIntent, PmOwnedRecoveryFill,
    PmOwnedReductionSequence, PmOwnedSubmitApply, PmOwnedSubmitResult, PmOwnedTerminalCompaction,
    PmPrivateConvergence, PmPrivateExternalIngressCounters, PmPrivateExternalIngressFailure,
    PmPrivateExternalIngressFault, PmPrivateExternalIngressLane, PmPrivateFillReduction,
    PmPrivateHaltReason, PmPrivateOrderReduction, PmPrivateQuoteRequest, PmPrivateReadiness,
    PmPrivateState, PmPrivateStateConfig, PmPrivateStateError, PmProvisionalDeltas,
    PmReconciliationApply, PmReconciliationReductions, PmRefreshCounters, PmRefreshKey,
    PmRemoteOrderKnowledge, PmReservationKnowledge, PmRiskCounters, PmRiskDecision,
    PmRiskDependency, PmRiskLimits, PmUnresolvedFillApply, PmUnresolvedFillCounters,
    PmUnresolvedFillKey, PmUnresolvedFillObservation, PmUnresolvedFillProjection,
    PmUnresolvedFillReason, PmUnresolvedFillStateError,
};
use reap_polymarket_adapter::{
    PmAccountPositionRoleError, PmFixtureAccountPositionSnapshot, PmFixtureAllowanceRow,
    PmFixtureBalanceRow, PmFixtureCompletionOccurrence, PmFixtureDeliveryScope,
    PmFixtureFeeEvidence, PmFixtureInstrumentScope, PmFixturePositionRow,
    PmFixturePrivateLifecycle, PmFixtureReadOwnerGrant, PmFixtureReconciliation,
    PmPrivateLifecycleObservation, PmPrivateNormalizationError, PmReconciliationContractError,
    PmUnresolvedTradeReason,
};
use thiserror::Error;

use crate::composition::PmCompositionError;

mod product_fixture;

pub(crate) use product_fixture::PmFixturePairedReconciliationDelivery;

/// One validated request/completion cut for a fixture-only read.
///
/// The value does not expose an adapter request builder. It merely proves that
/// a one-shot monitor call has internally consistent epoch, snapshot, causal
/// sequence, and service-time evidence before any role-local request counter is
/// advanced.
#[derive(Debug, PartialEq, Eq)]
pub struct PmFixtureQueryOccurrence {
    connection_epoch: ConnectionEpoch,
    request_sequence: IngressSequence,
    snapshot: PmSnapshotEvidence,
    completion: PmFixtureCompletionOccurrence,
    monotonic_service_ns: u64,
}

impl PmFixtureQueryOccurrence {
    pub fn new(
        connection_epoch: ConnectionEpoch,
        request_sequence: IngressSequence,
        snapshot: PmSnapshotEvidence,
        completion: PmFixtureCompletionOccurrence,
        monotonic_service_ns: u64,
    ) -> Result<Self, PmPrivateMonitorInputError> {
        if completion.ordering().connection_epoch() != connection_epoch {
            return Err(PmPrivateMonitorInputError::CompletionEpochMismatch);
        }
        if completion.ordering().snapshot_revision() != Some(snapshot.revision()) {
            return Err(PmPrivateMonitorInputError::CompletionSnapshotMismatch);
        }
        PmReconciliationRequestBoundary::new(
            request_sequence,
            completion.ordering().local_ingress_sequence(),
        )?;
        completion
            .received_clock()
            .service_at(monotonic_service_ns)?;
        Ok(Self {
            connection_epoch,
            request_sequence,
            snapshot,
            completion,
            monotonic_service_ns,
        })
    }

    fn completion(&self) -> PmFixtureCompletionOccurrence {
        PmFixtureCompletionOccurrence::new(
            self.completion.received_clock(),
            self.completion.ordering(),
        )
    }
}

#[derive(Debug)]
pub struct PmAccountFixtureInput<'a> {
    occurrence: PmFixtureQueryOccurrence,
    balances: &'a [PmFixtureBalanceRow],
    allowances: &'a [PmFixtureAllowanceRow],
    positions: &'a [PmFixturePositionRow],
}

impl<'a> PmAccountFixtureInput<'a> {
    #[must_use]
    pub const fn new(
        occurrence: PmFixtureQueryOccurrence,
        balances: &'a [PmFixtureBalanceRow],
        allowances: &'a [PmFixtureAllowanceRow],
        positions: &'a [PmFixturePositionRow],
    ) -> Self {
        Self {
            occurrence,
            balances,
            allowances,
            positions,
        }
    }
}

#[derive(Debug)]
pub struct PmOpenOrdersFixtureInput<'a> {
    occurrence: PmFixtureQueryOccurrence,
    raw_orders: &'a [&'a [u8]],
}

impl<'a> PmOpenOrdersFixtureInput<'a> {
    #[must_use]
    pub const fn new(occurrence: PmFixtureQueryOccurrence, raw_orders: &'a [&'a [u8]]) -> Self {
        Self {
            occurrence,
            raw_orders,
        }
    }
}

#[derive(Debug)]
pub struct PmOrderDetailFixtureInput<'a> {
    occurrence: PmFixtureQueryOccurrence,
    requested_order: PmVenueOrderKey,
    raw_order: Option<&'a [u8]>,
}

impl<'a> PmOrderDetailFixtureInput<'a> {
    #[must_use]
    pub const fn new(
        occurrence: PmFixtureQueryOccurrence,
        requested_order: PmVenueOrderKey,
        raw_order: Option<&'a [u8]>,
    ) -> Self {
        Self {
            occurrence,
            requested_order,
            raw_order,
        }
    }
}

/// One exact account-plus-fill cut used to establish private convergence.
#[derive(Debug)]
pub struct PmReconciliationFixtureInput<'a> {
    occurrence: PmFixtureQueryOccurrence,
    balances: &'a [PmFixtureBalanceRow],
    allowances: &'a [PmFixtureAllowanceRow],
    positions: &'a [PmFixturePositionRow],
    requested_after: Option<PmFillQueryCursor>,
    resulting_watermark: PmFillQueryCursor,
    raw_fill_frames: &'a [&'a [u8]],
    fee: PmFixtureFeeEvidence,
}

impl<'a> PmReconciliationFixtureInput<'a> {
    #[allow(
        clippy::too_many_arguments,
        reason = "the one-shot fixture input keeps its exact account and fill cut explicit"
    )]
    #[must_use]
    pub const fn new(
        occurrence: PmFixtureQueryOccurrence,
        balances: &'a [PmFixtureBalanceRow],
        allowances: &'a [PmFixtureAllowanceRow],
        positions: &'a [PmFixturePositionRow],
        requested_after: Option<PmFillQueryCursor>,
        resulting_watermark: PmFillQueryCursor,
        raw_fill_frames: &'a [&'a [u8]],
        fee: PmFixtureFeeEvidence,
    ) -> Self {
        Self {
            occurrence,
            balances,
            allowances,
            positions,
            requested_after,
            resulting_watermark,
            raw_fill_frames,
            fee,
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub struct PmPrivateBatchApply {
    order_observations: u16,
    fill_observations: u16,
    unresolved_fill_observations: u16,
    duplicate_or_stale_observations: u16,
}

impl PmPrivateBatchApply {
    #[must_use]
    pub const fn order_observations(self) -> u16 {
        self.order_observations
    }

    #[must_use]
    pub const fn fill_observations(self) -> u16 {
        self.fill_observations
    }

    #[must_use]
    pub const fn unresolved_fill_observations(self) -> u16 {
        self.unresolved_fill_observations
    }

    #[must_use]
    pub const fn duplicate_or_stale_observations(self) -> u16 {
        self.duplicate_or_stale_observations
    }

    const fn is_empty(self) -> bool {
        self.order_observations == 0
            && self.fill_observations == 0
            && self.unresolved_fill_observations == 0
            && self.duplicate_or_stale_observations == 0
    }
}

/// Immutable view over the monitor-owned canonical private state.
///
/// There is intentionally no `Deref`, `AsRef`, mutable accessor, or way to
/// recover the underlying state owner.
pub struct PmReadOnlyPrivateProjection<'a> {
    state: &'a PmPrivateState,
}

impl PmReadOnlyPrivateProjection<'_> {
    #[must_use]
    pub const fn account_snapshot(&self) -> PmAccountSnapshotProjection {
        self.state.account_projection()
    }

    pub fn allowance(&self, spender: PmSpenderId) -> PmAllowanceKnowledge {
        self.state.allowance(spender)
    }

    pub fn orders(&self) -> impl Iterator<Item = PmOrderProjection> + '_ {
        self.state.orders()
    }

    pub fn fills(&self) -> impl Iterator<Item = PmFillProjection> + '_ {
        self.state.fills()
    }

    pub fn unresolved_fills(&self) -> impl Iterator<Item = PmUnresolvedFillProjection> + '_ {
        self.state.unresolved_fills()
    }

    #[must_use]
    pub const fn provisional_deltas(&self) -> PmProvisionalDeltas {
        self.state.provisional_deltas()
    }

    #[must_use]
    pub const fn convergence(&self) -> PmPrivateConvergence {
        self.state.convergence()
    }

    #[must_use]
    pub const fn halt(&self) -> Option<PmPrivateHaltReason> {
        self.state.halt()
    }

    #[must_use]
    pub const fn pending_refresh_count(&self) -> usize {
        self.state.pending_refresh_count()
    }

    pub fn pending_refresh_keys(&self) -> impl Iterator<Item = PmRefreshKey> + '_ {
        self.state.pending_refresh_keys()
    }

    #[must_use]
    pub const fn full_reconcile_required(&self) -> bool {
        self.state.full_reconcile_required()
    }

    #[must_use]
    pub const fn account_counters(&self) -> PmAccountCounters {
        self.state.account_counters()
    }

    #[must_use]
    pub const fn order_counters(&self) -> PmOrderCounters {
        self.state.order_counters()
    }

    #[must_use]
    pub const fn fill_counters(&self) -> PmFillCounters {
        self.state.fill_counters()
    }

    #[must_use]
    pub const fn unresolved_fill_counters(&self) -> PmUnresolvedFillCounters {
        self.state.unresolved_fill_counters()
    }

    #[must_use]
    pub const fn refresh_counters(&self) -> PmRefreshCounters {
        self.state.refresh_counters()
    }

    #[must_use]
    pub const fn risk_counters(&self) -> PmRiskCounters {
        self.state.risk_counters()
    }

    #[must_use]
    pub const fn external_ingress_counters(&self) -> PmPrivateExternalIngressCounters {
        self.state.external_ingress_counters()
    }

    #[must_use]
    pub fn quote_readiness(&self, request: PmPrivateQuoteRequest) -> PmPrivateReadiness {
        self.state.quote_readiness(request)
    }
}

/// Least-authority fixture-only account/private monitor.
///
/// The role objects and canonical state are held by value and never exposed.
/// Every mutation enters through an owner-bound serviced delivery produced by
/// one of the three exact read roles.
pub struct PmReadOnlyMonitor {
    plan: PmConnectivityPlan,
    bindings: Vec<ConstructedRoleBinding>,
    runtime: PmPrivateMonitorRuntime,
}

impl PmReadOnlyMonitor {
    pub fn new(
        config: PmAccountConnectivityConfig,
        risk_limits: PmRiskLimits,
    ) -> Result<Self, PmCompositionError> {
        let plan = PmConnectivityPlan::read_only_monitor(config)?;
        let config = plan
            .account_config()
            .expect("monitor plan carries account config");
        let runtime = PmPrivateMonitorRuntime::new(config, risk_limits)?;
        let bindings = runtime.bindings(config)?;
        plan.validate_bindings(&bindings)?;
        Ok(Self {
            plan,
            bindings,
            runtime,
        })
    }

    #[must_use]
    pub fn reached_roles(&self) -> &[PmRoleKind] {
        self.plan.reached_roles()
    }

    #[must_use]
    pub fn binding_count(&self) -> usize {
        let config = self
            .plan
            .account_config()
            .expect("monitor plan carries account config");
        debug_assert_eq!(self.runtime.private.account_scope(), config.account_scope());
        debug_assert_eq!(
            self.runtime.reconciliation.account_scope(),
            config.account_scope()
        );
        debug_assert_eq!(self.runtime.account.account_scope(), config.account_scope());
        debug_assert_eq!(self.runtime.account.instrument(), config.instrument());
        self.bindings.len()
    }

    pub fn reconnect_private(
        &mut self,
        connection_epoch: ConnectionEpoch,
        monotonic_observed_ns: u64,
    ) -> Result<(), PmPrivateMonitorError> {
        let result = self
            .runtime
            .reconnect_private(connection_epoch, monotonic_observed_ns);
        self.record_result(PmPrivateExternalIngressLane::Reconnect, result)
    }

    pub fn ingest_private_fixture(
        &mut self,
        occurrence: PmFixtureCompletionOccurrence,
        monotonic_service_ns: u64,
        raw: &[u8],
        fee: PmFixtureFeeEvidence,
    ) -> Result<PmPrivateBatchApply, PmPrivateMonitorError> {
        let result =
            self.runtime
                .ingest_private_fixture(occurrence, monotonic_service_ns, raw, fee);
        self.record_result(PmPrivateExternalIngressLane::PrivateLifecycle, result)
    }

    pub fn ingest_account_fixture(
        &mut self,
        input: PmAccountFixtureInput<'_>,
    ) -> Result<PmAccountSnapshotApply, PmPrivateMonitorError> {
        let result = self.runtime.ingest_account_fixture(input);
        self.record_result(PmPrivateExternalIngressLane::AccountSnapshot, result)
    }

    pub fn ingest_open_orders_fixture(
        &mut self,
        input: PmOpenOrdersFixtureInput<'_>,
    ) -> Result<PmOpenOrdersApply, PmPrivateMonitorError> {
        let result = self.runtime.ingest_open_orders_fixture(input);
        self.record_result(PmPrivateExternalIngressLane::OpenOrders, result)
    }

    pub fn ingest_order_detail_fixture(
        &mut self,
        input: PmOrderDetailFixtureInput<'_>,
    ) -> Result<PmOrderApply, PmPrivateMonitorError> {
        let result = self.runtime.ingest_order_detail_fixture(input);
        self.record_result(PmPrivateExternalIngressLane::OrderDetail, result)
    }

    pub fn ingest_reconciliation_fixture(
        &mut self,
        input: PmReconciliationFixtureInput<'_>,
    ) -> Result<PmReconciliationApply, PmPrivateMonitorError> {
        let result = self.runtime.ingest_reconciliation_fixture(input);
        self.record_result(PmPrivateExternalIngressLane::Reconciliation, result)
    }

    #[must_use]
    pub const fn private_projection(&self) -> PmReadOnlyPrivateProjection<'_> {
        PmReadOnlyPrivateProjection {
            state: &self.runtime.state,
        }
    }

    fn record_result<T>(
        &mut self,
        lane: PmPrivateExternalIngressLane,
        result: Result<T, PmPrivateMonitorError>,
    ) -> Result<T, PmPrivateMonitorError> {
        if let Err(error) = &result {
            self.runtime
                .state
                .record_external_ingress_fault(PmPrivateExternalIngressFault::new(
                    lane,
                    classify_monitor_error(error),
                ));
        }
        result
    }
}

pub(crate) struct PmPrivateMonitorRuntime {
    private: PmFixturePrivateLifecycle,
    reconciliation: PmFixtureReconciliation,
    account: PmFixtureAccountPositionSnapshot,
    state: PmPrivateState,
    open_order_reservation_scratch: Vec<PmOpenOrderReservation>,
}

/// Exact canonical result of one complete-scheduler private observation.
#[allow(
    clippy::large_enum_variant,
    reason = "canonical private reductions remain copy-only and allocation-free on the owner loop"
)]
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum PmServicedPrivateReduction {
    Order(PmPrivateOrderReduction),
    Fill(PmPrivateFillReduction),
    Unresolved(PmUnresolvedFillApply),
}

impl std::fmt::Debug for PmPrivateMonitorRuntime {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter
            .debug_struct("PmPrivateMonitorRuntime")
            .field("account_scope", &self.account_scope())
            .field("instrument", &self.instrument())
            .finish_non_exhaustive()
    }
}

impl PmPrivateMonitorRuntime {
    pub(crate) fn new(
        config: &PmAccountConnectivityConfig,
        risk_limits: PmRiskLimits,
    ) -> Result<Self, PmCompositionError> {
        let route = config.account_route();
        let (private_grant, reconciliation_grant, account_grant) =
            PmFixtureReadOwnerGrant::allocate().split();
        let instrument_scope = PmFixtureInstrumentScope::from_metadata(
            config.instrument(),
            config.expected_metadata(),
        )?;
        let private = PmFixturePrivateLifecycle::new(
            private_grant,
            config.account_scope(),
            instrument_scope,
            route.source(),
            route.connection(),
        )?;
        let reconciliation = PmFixtureReconciliation::new(
            reconciliation_grant,
            config.account_scope(),
            instrument_scope,
            route.source(),
            route.connection(),
        )?;
        let account = PmFixtureAccountPositionSnapshot::new(
            account_grant,
            config.account_scope(),
            instrument_scope,
            route.source(),
            route.connection(),
        )?;
        let state_config = PmPrivateStateConfig::new(
            route.source(),
            config.account_scope(),
            config.instrument(),
            config.expected_metadata(),
        )?;
        let state = PmPrivateState::new(state_config, risk_limits)?;
        Ok(Self {
            private,
            reconciliation,
            account,
            state,
            open_order_reservation_scratch: Vec::with_capacity(
                reap_pm_core::MAX_PM_RECONCILIATION_ORDERS,
            ),
        })
    }

    pub(crate) fn bindings(
        &self,
        config: &PmAccountConnectivityConfig,
    ) -> Result<Vec<ConstructedRoleBinding>, PmPlanError> {
        monitor_bindings(config, &self.private, &self.reconciliation, &self.account)
    }

    pub(crate) const fn account_scope(&self) -> PmAccountScope {
        self.private.account_scope()
    }

    pub(crate) const fn instrument(&self) -> reap_pm_core::PmInstrumentHandle {
        self.account.instrument()
    }

    pub(crate) const fn active_epoch(&self) -> Option<ConnectionEpoch> {
        self.private.active_epoch()
    }

    pub(crate) fn owned_fill_event(
        &self,
        key: PmFillKey,
        client_order: PmClientOrderKey,
        execution: PmFillExecution,
    ) -> Result<PmFillEvent, reap_pm_core::PmEventError> {
        let order = PmOrderIdentity::new(Some(client_order), Some(key.venue_order()))?;
        PmFillEvent::new(
            self.state.source(),
            self.state.instrument(),
            key,
            order,
            execution,
        )
    }

    pub(crate) fn issue_owned_immediate_ack_ticket(
        &mut self,
    ) -> Result<PmOwnedImmediateAckTicket, PmPrivateStateError> {
        self.state.issue_owned_immediate_ack_ticket()
    }

    pub(crate) fn observe_owned_immediate_fill(
        &mut self,
        ticket: PmOwnedImmediateAckTicket,
        event: PmFillEvent,
        reported_cumulative: Option<U256>,
    ) -> Result<PmOwnedFillApply, PmPrivateStateError> {
        self.state
            .observe_owned_immediate_fill(ticket, event, reported_cumulative)
    }

    pub(crate) fn quote_readiness(&self, request: PmPrivateQuoteRequest) -> PmPrivateReadiness {
        self.state.quote_readiness(request)
    }

    pub(crate) fn evaluate_risk_candidate(
        &mut self,
        request: PmPrivateQuoteRequest,
        reference: PmRiskDependency,
        book: PmRiskDependency,
    ) -> Result<PmRiskDecision, PmPrivateStateError> {
        self.state.evaluate_risk_candidate(request, reference, book)
    }

    pub(crate) fn admit_owned_quote(
        &mut self,
        intent: PmOwnedQuoteIntent,
    ) -> Result<PmOwnedQuoteAdmission, PmPrivateStateError> {
        self.state.admit_owned_quote(intent)
    }

    pub(crate) fn apply_owned_submit_result(
        &mut self,
        client_order: PmClientOrderKey,
        result: PmOwnedSubmitResult,
    ) -> Result<PmOwnedSubmitApply, PmPrivateStateError> {
        self.state.apply_owned_submit_result(client_order, result)
    }

    pub(crate) fn recover_owned_fill(
        &mut self,
        recovery: PmOwnedRecoveryFill,
    ) -> Result<PmOwnedFillApply, PmPrivateStateError> {
        self.state.recover_owned_fill(recovery)
    }

    pub(crate) fn recover_owned_progress(
        &mut self,
        observation: PmOwnedOrderProgressObservation,
    ) -> Result<PmOwnedProgressApply, PmPrivateStateError> {
        self.state.recover_owned_progress(observation)
    }

    pub(crate) fn finish_owned_recovery(
        &mut self,
        high_watermark: PmOwnedReductionSequence,
    ) -> Result<(), PmPrivateStateError> {
        self.state.finish_owned_recovery(high_watermark)
    }

    pub(crate) fn request_owned_cancel(
        &mut self,
        client_order: PmClientOrderKey,
    ) -> Result<PmOwnedCancelRequestApply, PmPrivateStateError> {
        self.state.request_owned_cancel(client_order)
    }

    pub(crate) fn apply_owned_cancel_result(
        &mut self,
        intent: PmOwnedCancelIntent,
        outcome: PmOwnedCancelOutcome,
    ) -> Result<PmOwnedCancelApply, PmPrivateStateError> {
        self.state.apply_owned_cancel_result(intent, outcome)
    }

    pub(crate) fn compact_proven_owned_terminal(
        &mut self,
        client_order: PmClientOrderKey,
    ) -> Result<PmOwnedTerminalCompaction, PmPrivateStateError> {
        self.state.compact_proven_owned_terminal(client_order)
    }

    pub(crate) fn owned_order(
        &self,
        client_order: PmClientOrderKey,
    ) -> Option<PmOwnedOrderProjection> {
        self.state
            .owned_orders()
            .find(|order| order.client_order() == client_order)
    }

    pub(crate) fn reduce_serviced_connection_available(
        &mut self,
        source: PmProductSource,
        connection: PmConnectionId,
        connection_epoch: ConnectionEpoch,
        monotonic_observed_ns: u64,
    ) -> Result<(), PmPrivateMonitorError> {
        if source != self.private.source() || connection != self.private.connection() {
            return Err(PmPrivateMonitorError::DeliveryScopeMismatch);
        }
        if self.private.active_epoch() != Some(connection_epoch) {
            return Err(PmPrivateMonitorError::PrivateEpochMismatch);
        }
        self.state
            .validate_reconnect(connection_epoch, monotonic_observed_ns)?;
        self.state
            .observe_reconnect(connection_epoch, monotonic_observed_ns)?;
        Ok(())
    }

    pub(crate) fn reduce_serviced_connection_unavailable(
        &mut self,
        source: PmProductSource,
        connection: PmConnectionId,
        fault: PmPrivateExternalIngressFault,
    ) -> Result<(), PmPrivateMonitorError> {
        if source != self.private.source() || connection != self.private.connection() {
            return Err(PmPrivateMonitorError::DeliveryScopeMismatch);
        }
        self.state.record_external_ingress_fault(fault);
        Ok(())
    }

    #[allow(clippy::too_many_arguments)]
    pub(crate) fn reduce_serviced_private_observation(
        &mut self,
        source: PmProductSource,
        connection: PmConnectionId,
        clock: EventClock,
        ordering: EventOrdering,
        observation: PmPrivateLifecycleObservation,
    ) -> Result<PmServicedPrivateReduction, PmPrivateMonitorError> {
        self.validate_serviced_account_scope(source, connection, ordering)?;
        match observation {
            PmPrivateLifecycleObservation::Order(order) => {
                let knowledge = PmRemoteOrderKnowledge::Unmanaged(remote_reservation(order)?);
                let envelope =
                    EventEnvelope::new(source.venue(), source, connection, clock, ordering, order)?;
                Ok(PmServicedPrivateReduction::Order(
                    self.state.observe_order_reduction(envelope, knowledge)?,
                ))
            }
            PmPrivateLifecycleObservation::Fill(fill) => {
                let envelope =
                    EventEnvelope::new(source.venue(), source, connection, clock, ordering, fill)?;
                Ok(PmServicedPrivateReduction::Fill(
                    self.state.observe_fill_reduction(envelope)?,
                ))
            }
            PmPrivateLifecycleObservation::UnresolvedTrade(unresolved) => {
                let observation = PmUnresolvedFillObservation::new(
                    source,
                    unresolved.account(),
                    unresolved.instrument(),
                    unresolved.fill_id(),
                    unresolved.order(),
                    unresolved.candidate_order(),
                    unresolved_reason(unresolved.reason()),
                    unresolved.settlement(),
                )?;
                let envelope = EventEnvelope::new(
                    source.venue(),
                    source,
                    connection,
                    clock,
                    ordering,
                    observation,
                )?;
                Ok(PmServicedPrivateReduction::Unresolved(
                    self.state.observe_unresolved_fill(envelope)?,
                ))
            }
        }
    }

    #[allow(clippy::too_many_arguments)]
    pub(crate) fn reduce_serviced_open_orders(
        &mut self,
        source: PmProductSource,
        connection: PmConnectionId,
        clock: EventClock,
        ordering: EventOrdering,
        snapshot: PmCompleteOpenOrdersSnapshot,
    ) -> Result<PmOpenOrdersApply, PmPrivateMonitorError> {
        self.validate_serviced_account_scope(source, connection, ordering)?;
        let envelope = EventEnvelope::new(
            source.venue(),
            source,
            connection,
            clock,
            ordering,
            snapshot,
        )?;
        open_order_reservations_into(
            envelope.payload().orders(),
            &mut self.open_order_reservation_scratch,
        )?;
        Ok(self
            .state
            .apply_open_orders_snapshot(envelope, &self.open_order_reservation_scratch)?)
    }

    #[allow(clippy::too_many_arguments)]
    pub(crate) fn reduce_serviced_order_detail(
        &mut self,
        source: PmProductSource,
        connection: PmConnectionId,
        clock: EventClock,
        ordering: EventOrdering,
        detail: PmExactOrderDetail,
    ) -> Result<PmOrderApply, PmPrivateMonitorError> {
        self.validate_serviced_account_scope(source, connection, ordering)?;
        let reservation = detail
            .order()
            .map(remote_reservation)
            .transpose()?
            .unwrap_or(PmReservationKnowledge::Unknown);
        let envelope =
            EventEnvelope::new(source.venue(), source, connection, clock, ordering, detail)?;
        Ok(self.state.apply_order_detail(envelope, reservation)?)
    }

    #[allow(clippy::too_many_arguments)]
    pub(crate) fn reduce_serviced_account_snapshot(
        &mut self,
        source: PmProductSource,
        connection: PmConnectionId,
        clock: EventClock,
        ordering: EventOrdering,
        snapshot: PmCompleteAccountSnapshot,
    ) -> Result<PmAccountSnapshotApply, PmPrivateMonitorError> {
        self.validate_serviced_account_scope(source, connection, ordering)?;
        let envelope = EventEnvelope::new(
            source.venue(),
            source,
            connection,
            clock,
            ordering,
            snapshot,
        )?;
        Ok(self.state.apply_account_snapshot(envelope)?)
    }

    /// Reduces a paired, complete account-plus-fill cut through the sole
    /// canonical private owner and exposes exact REST owned-fill consequences
    /// in caller-owned bounded scratch.
    pub(crate) fn reduce_serviced_reconciliation(
        &mut self,
        account: EventEnvelope<PmCompleteAccountSnapshot>,
        fills: EventEnvelope<PmCompleteFillQuery>,
        reductions: &mut PmReconciliationReductions,
    ) -> Result<PmReconciliationApply, PmPrivateMonitorError> {
        self.validate_serviced_account_scope(
            account.source(),
            account.connection_id(),
            account.ordering(),
        )?;
        self.validate_serviced_account_scope(
            fills.source(),
            fills.connection_id(),
            fills.ordering(),
        )?;
        Ok(self
            .state
            .apply_reconciliation_with_reductions(account, fills, reductions)?)
    }

    fn validate_serviced_account_scope(
        &self,
        source: PmProductSource,
        connection: PmConnectionId,
        ordering: EventOrdering,
    ) -> Result<(), PmPrivateMonitorError> {
        if source != self.private.source()
            || connection != self.private.connection()
            || ordering.connection_epoch()
                != self
                    .private
                    .active_epoch()
                    .ok_or(PmPrivateMonitorError::PrivateEpochMismatch)?
        {
            return Err(PmPrivateMonitorError::DeliveryScopeMismatch);
        }
        Ok(())
    }

    fn reconnect_private(
        &mut self,
        connection_epoch: ConnectionEpoch,
        monotonic_observed_ns: u64,
    ) -> Result<(), PmPrivateMonitorError> {
        validate_private_role_reconnect(self.private.active_epoch(), connection_epoch)?;
        self.state
            .validate_reconnect(connection_epoch, monotonic_observed_ns)?;
        self.state
            .observe_reconnect(connection_epoch, monotonic_observed_ns)?;
        self.private
            .reconnect(connection_epoch)
            .expect("the single-owner private role reconnect was prevalidated");
        Ok(())
    }

    fn ingest_private_fixture(
        &mut self,
        occurrence: PmFixtureCompletionOccurrence,
        monotonic_service_ns: u64,
        raw: &[u8],
        fee: PmFixtureFeeEvidence,
    ) -> Result<PmPrivateBatchApply, PmPrivateMonitorError> {
        occurrence
            .received_clock()
            .service_at(monotonic_service_ns)?;
        let delivery = self.private.receive_user_fixture(occurrence, raw, fee)?;
        let serviced = delivery.service_at(monotonic_service_ns)?;
        let expected_account = self.private.account_scope();
        let expected_instrument = self.private.instrument_scope();
        let state = &mut self.state;
        match self
            .private
            .reduce_private_delivery(serviced, |scope, envelope| {
                validate_scope(scope, expected_account, expected_instrument)?;
                reduce_private_batch(state, envelope)
            }) {
            Ok(result) => result,
            Err(_) => Err(PmPrivateMonitorError::PrivateDeliveryOwnerMismatch),
        }
    }

    fn ingest_account_fixture(
        &mut self,
        input: PmAccountFixtureInput<'_>,
    ) -> Result<PmAccountSnapshotApply, PmPrivateMonitorError> {
        let query = input.occurrence;
        self.validate_private_epoch(query.connection_epoch)?;
        let request = self
            .account
            .request_snapshot(query.connection_epoch, query.request_sequence)?;
        let delivery = request.complete(
            query.completion(),
            query.snapshot,
            self.account.account_scope(),
            input.balances,
            input.allowances,
            input.positions,
        )?;
        let serviced = delivery.service_at(query.monotonic_service_ns)?;
        let expected_account = self.account.account_scope();
        let expected_instrument = self.account.instrument_scope();
        let state = &mut self.state;
        match self
            .account
            .reduce_snapshot_delivery(serviced, |scope, envelope| {
                validate_scope(scope, expected_account, expected_instrument)?;
                Ok(state.apply_account_snapshot(envelope)?)
            }) {
            Ok(result) => result,
            Err(_) => Err(PmPrivateMonitorError::AccountDeliveryOwnerMismatch),
        }
    }

    fn ingest_open_orders_fixture(
        &mut self,
        input: PmOpenOrdersFixtureInput<'_>,
    ) -> Result<PmOpenOrdersApply, PmPrivateMonitorError> {
        let query = input.occurrence;
        self.validate_private_epoch(query.connection_epoch)?;
        let request = self
            .reconciliation
            .request_open_orders(query.connection_epoch, query.request_sequence)?;
        let delivery =
            request.complete_json_objects(query.completion(), query.snapshot, input.raw_orders)?;
        let serviced = delivery.service_at(query.monotonic_service_ns)?;
        let expected_account = self.reconciliation.account_scope();
        let expected_instrument = self.reconciliation.instrument_scope();
        let state = &mut self.state;
        let reservations = &mut self.open_order_reservation_scratch;
        match self
            .reconciliation
            .reduce_open_orders_delivery(serviced, |scope, envelope| {
                validate_scope(scope, expected_account, expected_instrument)?;
                open_order_reservations_into(envelope.payload().orders(), reservations)?;
                Ok(state.apply_open_orders_snapshot(envelope, reservations)?)
            }) {
            Ok(result) => result,
            Err(_) => Err(PmPrivateMonitorError::ReconciliationDeliveryOwnerMismatch),
        }
    }

    fn ingest_order_detail_fixture(
        &mut self,
        input: PmOrderDetailFixtureInput<'_>,
    ) -> Result<PmOrderApply, PmPrivateMonitorError> {
        let query = input.occurrence;
        self.validate_private_epoch(query.connection_epoch)?;
        let request = self.reconciliation.request_order_detail(
            query.connection_epoch,
            query.request_sequence,
            input.requested_order,
        )?;
        let delivery =
            request.complete_json_object(query.completion(), query.snapshot, input.raw_order)?;
        let serviced = delivery.service_at(query.monotonic_service_ns)?;
        let expected_account = self.reconciliation.account_scope();
        let expected_instrument = self.reconciliation.instrument_scope();
        let state = &mut self.state;
        match self
            .reconciliation
            .reduce_order_detail_delivery(serviced, |scope, envelope| {
                validate_scope(scope, expected_account, expected_instrument)?;
                let reservation = envelope
                    .payload()
                    .order()
                    .map(remote_reservation)
                    .transpose()?
                    .unwrap_or(PmReservationKnowledge::Unknown);
                Ok(state.apply_order_detail(envelope, reservation)?)
            }) {
            Ok(result) => result,
            Err(_) => Err(PmPrivateMonitorError::ReconciliationDeliveryOwnerMismatch),
        }
    }

    fn ingest_reconciliation_fixture(
        &mut self,
        input: PmReconciliationFixtureInput<'_>,
    ) -> Result<PmReconciliationApply, PmPrivateMonitorError> {
        let query = input.occurrence;
        self.validate_private_epoch(query.connection_epoch)?;
        let account_request = self
            .account
            .request_snapshot(query.connection_epoch, query.request_sequence)?;
        let fill_request = self.reconciliation.request_fills(
            query.connection_epoch,
            query.request_sequence,
            input.requested_after,
        )?;
        let account_delivery = account_request.complete(
            query.completion(),
            query.snapshot,
            self.account.account_scope(),
            input.balances,
            input.allowances,
            input.positions,
        )?;
        let fill_delivery = fill_request.complete_user_frames(
            query.completion(),
            query.snapshot,
            input.resulting_watermark,
            input.raw_fill_frames,
            input.fee,
        )?;
        let account_serviced = account_delivery.service_at(query.monotonic_service_ns)?;
        let fill_serviced = fill_delivery.service_at(query.monotonic_service_ns)?;
        let expected_account = self.account.account_scope();
        let expected_instrument = self.account.instrument_scope();
        let mut account_envelope = None;
        match self
            .account
            .reduce_snapshot_delivery(account_serviced, |scope, envelope| {
                validate_scope(scope, expected_account, expected_instrument)?;
                account_envelope = Some(envelope);
                Ok::<(), PmPrivateMonitorError>(())
            }) {
            Ok(result) => result?,
            Err(_) => return Err(PmPrivateMonitorError::AccountDeliveryOwnerMismatch),
        }
        let expected_account = self.reconciliation.account_scope();
        let expected_instrument = self.reconciliation.instrument_scope();
        let mut fill_envelope = None;
        match self
            .reconciliation
            .reduce_fill_query_delivery(fill_serviced, |scope, envelope| {
                validate_scope(scope, expected_account, expected_instrument)?;
                fill_envelope = Some(envelope);
                Ok::<(), PmPrivateMonitorError>(())
            }) {
            Ok(result) => result?,
            Err(_) => {
                return Err(PmPrivateMonitorError::ReconciliationDeliveryOwnerMismatch);
            }
        }
        self.state
            .apply_reconciliation(
                account_envelope.expect("owner-opened account delivery"),
                fill_envelope.expect("owner-opened fill delivery"),
            )
            .map_err(PmPrivateMonitorError::from)
    }

    fn validate_private_epoch(
        &self,
        connection_epoch: ConnectionEpoch,
    ) -> Result<(), PmPrivateMonitorError> {
        if self.private.active_epoch() == Some(connection_epoch) {
            Ok(())
        } else {
            Err(PmPrivateMonitorError::PrivateEpochMismatch)
        }
    }
}

pub(crate) fn monitor_bindings(
    config: &PmAccountConnectivityConfig,
    private: &PmFixturePrivateLifecycle,
    reconciliation: &PmFixtureReconciliation,
    account: &PmFixtureAccountPositionSnapshot,
) -> Result<Vec<ConstructedRoleBinding>, PmPlanError> {
    let mut bindings = Vec::with_capacity(16);
    bindings.extend(ConstructedRoleBinding::private_lifecycle(
        private.account_scope(),
        config.instrument(),
        config.instrument_id(),
        PmConnectionRoute::new(private.source(), private.connection()),
    ));
    bindings.extend(ConstructedRoleBinding::reconciliation(
        reconciliation.account_scope(),
        config.instrument(),
        config.instrument_id(),
        PmConnectionRoute::new(reconciliation.source(), reconciliation.connection()),
    ));
    bindings.extend(ConstructedRoleBinding::account_snapshot(
        account.account_scope(),
        account.instrument(),
        config.instrument_id(),
        config.collateral_asset(),
        account.required_spenders(),
        PmConnectionRoute::new(account.source(), account.connection()),
    )?);
    Ok(bindings)
}

fn validate_scope(
    scope: PmFixtureDeliveryScope,
    expected_account: PmAccountScope,
    expected_instrument: PmFixtureInstrumentScope,
) -> Result<(), PmPrivateMonitorError> {
    if scope.account_scope() != expected_account || scope.instrument_scope() != expected_instrument
    {
        Err(PmPrivateMonitorError::DeliveryScopeMismatch)
    } else {
        Ok(())
    }
}

fn validate_private_role_reconnect(
    active_epoch: Option<ConnectionEpoch>,
    connection_epoch: ConnectionEpoch,
) -> Result<(), PmPrivateMonitorError> {
    if connection_epoch.value() == 0 {
        return Err(PmPrivateNormalizationError::ZeroConnectionEpoch.into());
    }
    if active_epoch.is_some_and(|active| connection_epoch <= active) {
        return Err(PmPrivateNormalizationError::ConnectionEpochDidNotAdvance.into());
    }
    Ok(())
}

fn reduce_private_batch(
    state: &mut PmPrivateState,
    envelope: EventEnvelope<reap_polymarket_adapter::PmFixturePrivateBatch>,
) -> Result<PmPrivateBatchApply, PmPrivateMonitorError> {
    validate_private_batch(envelope.payload().observations())?;
    let mut applied = PmPrivateBatchApply::default();
    for observation in envelope.payload().observations().iter().copied() {
        if let Err(source_error) =
            reduce_private_observation(state, &envelope, observation, &mut applied)
        {
            return Err(if applied.is_empty() {
                source_error
            } else {
                PmPrivateMonitorError::PrivateBatchPartial {
                    applied,
                    source: Box::new(source_error),
                }
            });
        }
    }
    Ok(applied)
}

fn reduce_private_observation(
    state: &mut PmPrivateState,
    batch: &EventEnvelope<reap_polymarket_adapter::PmFixturePrivateBatch>,
    observation: PmPrivateLifecycleObservation,
    applied: &mut PmPrivateBatchApply,
) -> Result<(), PmPrivateMonitorError> {
    let venue = batch.venue();
    let source = batch.source();
    let connection = batch.connection_id();
    let clock = batch.clock();
    let ordering = batch.ordering();
    match observation {
        PmPrivateLifecycleObservation::Order(order) => {
            let knowledge = PmRemoteOrderKnowledge::Unmanaged(remote_reservation(order)?);
            let envelope = EventEnvelope::new(venue, source, connection, clock, ordering, order)?;
            let outcome = state.observe_order(envelope, knowledge)?;
            applied.order_observations = checked_increment(applied.order_observations)?;
            if matches!(
                outcome,
                PmOrderApply::Duplicate | PmOrderApply::IgnoredStale
            ) {
                applied.duplicate_or_stale_observations =
                    checked_increment(applied.duplicate_or_stale_observations)?;
            }
        }
        PmPrivateLifecycleObservation::Fill(fill) => {
            let envelope = EventEnvelope::new(venue, source, connection, clock, ordering, fill)?;
            let outcome = state.observe_fill(envelope)?;
            applied.fill_observations = checked_increment(applied.fill_observations)?;
            if matches!(outcome, PmFillApply::Duplicate | PmFillApply::IgnoredStale) {
                applied.duplicate_or_stale_observations =
                    checked_increment(applied.duplicate_or_stale_observations)?;
            }
        }
        PmPrivateLifecycleObservation::UnresolvedTrade(unresolved) => {
            let observation = PmUnresolvedFillObservation::new(
                source,
                unresolved.account(),
                unresolved.instrument(),
                unresolved.fill_id(),
                unresolved.order(),
                unresolved.candidate_order(),
                unresolved_reason(unresolved.reason()),
                unresolved.settlement(),
            )?;
            let envelope =
                EventEnvelope::new(venue, source, connection, clock, ordering, observation)?;
            let outcome = state.observe_unresolved_fill(envelope)?;
            applied.unresolved_fill_observations =
                checked_increment(applied.unresolved_fill_observations)?;
            if matches!(
                outcome,
                PmUnresolvedFillApply::Duplicate(_) | PmUnresolvedFillApply::IgnoredStale(_)
            ) {
                applied.duplicate_or_stale_observations =
                    checked_increment(applied.duplicate_or_stale_observations)?;
            }
        }
    }
    Ok(())
}

pub(crate) fn validate_private_batch(
    observations: &[PmPrivateLifecycleObservation],
) -> Result<(), PmPrivateMonitorError> {
    let mut client_orders = Vec::<PmClientOrderKey>::with_capacity(observations.len());
    let mut venue_orders = Vec::<PmVenueOrderKey>::with_capacity(observations.len());
    let mut fills = Vec::<PmFillKey>::with_capacity(observations.len());
    let mut unresolved = Vec::<PmUnresolvedFillKey>::with_capacity(observations.len());
    for observation in observations {
        match *observation {
            PmPrivateLifecycleObservation::Order(order) => {
                client_orders.extend(order.order().client_order_key());
                venue_orders.extend(order.order().venue_order_key());
            }
            PmPrivateLifecycleObservation::Fill(fill) => fills.push(fill.fill_key()),
            PmPrivateLifecycleObservation::UnresolvedTrade(trade) => {
                unresolved.push(PmUnresolvedFillKey::new(
                    trade.fill_id(),
                    trade.order(),
                    trade.candidate_order(),
                ));
            }
        }
    }
    client_orders.sort_unstable();
    venue_orders.sort_unstable();
    fills.sort_unstable();
    unresolved.sort_unstable();
    if has_adjacent_duplicate(&client_orders)
        || has_adjacent_duplicate(&venue_orders)
        || has_adjacent_duplicate(&fills)
        || has_adjacent_duplicate(&unresolved)
        || unresolved.iter().any(|unresolved| {
            unresolved.exact_order().is_some_and(|order| {
                fills
                    .binary_search(&PmFillKey::new(order, unresolved.fill_id()))
                    .is_ok()
            })
        })
    {
        Err(PmPrivateMonitorError::DuplicateBatchIdentity)
    } else {
        Ok(())
    }
}

fn has_adjacent_duplicate<T: Eq>(values: &[T]) -> bool {
    values.windows(2).any(|pair| pair[0] == pair[1])
}

fn open_order_reservations_into(
    orders: &[PmOrderEvent],
    reservations: &mut Vec<PmOpenOrderReservation>,
) -> Result<(), PmPrivateMonitorError> {
    reservations.clear();
    if orders.len() > reservations.capacity() {
        return Err(PmPrivateMonitorError::BatchCounterOverflow);
    }
    for order in orders {
        let venue_order = order
            .order()
            .venue_order_key()
            .ok_or(PmPrivateMonitorError::OpenOrderMissingVenueIdentity)?;
        reservations.push(PmOpenOrderReservation::new(
            venue_order,
            remote_reservation(*order)?,
        ));
    }
    Ok(())
}

fn remote_reservation(
    order: PmOrderEvent,
) -> Result<PmReservationKnowledge, PmPrivateMonitorError> {
    if order.progress().status().is_terminal() || order.side() == PmOrderSide::Buy {
        return Ok(PmReservationKnowledge::Unknown);
    }
    Ok(PmReservationKnowledge::Known(
        PmExactReservation::authoritative_sell_remaining(
            order.progress().remaining_quantity_units(),
        )
        .map_err(PmPrivateStateError::from)?,
    ))
}

const fn unresolved_reason(reason: PmUnresolvedTradeReason) -> PmUnresolvedFillReason {
    match reason {
        PmUnresolvedTradeReason::MissingExactOrderLinkage => {
            PmUnresolvedFillReason::MissingExactOrderLinkage
        }
        PmUnresolvedTradeReason::MultipleOrderReferenceKinds => {
            PmUnresolvedFillReason::MultipleOrderReferenceKinds
        }
        PmUnresolvedTradeReason::MissingDirectOrderRole => {
            PmUnresolvedFillReason::MissingDirectOrderRole
        }
        PmUnresolvedTradeReason::MissingLocalMakerOrderProof => {
            PmUnresolvedFillReason::MissingLocalMakerOrderProof
        }
        PmUnresolvedTradeReason::ExternalMakerOrder => PmUnresolvedFillReason::ExternalMakerOrder,
    }
}

fn checked_increment(value: u16) -> Result<u16, PmPrivateMonitorError> {
    value
        .checked_add(1)
        .ok_or(PmPrivateMonitorError::BatchCounterOverflow)
}

fn classify_monitor_error(error: &PmPrivateMonitorError) -> PmPrivateExternalIngressFailure {
    match error {
        PmPrivateMonitorError::PrivateNormalization(_) => {
            PmPrivateExternalIngressFailure::Normalization
        }
        PmPrivateMonitorError::Envelope(_) => PmPrivateExternalIngressFailure::Service,
        PmPrivateMonitorError::Reconciliation(PmReconciliationContractError::Normalization(_)) => {
            PmPrivateExternalIngressFailure::Normalization
        }
        PmPrivateMonitorError::Reconciliation(
            PmReconciliationContractError::WrongSource
            | PmReconciliationContractError::SourceAccountMismatch
            | PmReconciliationContractError::RequestedOrderAccountMismatch
            | PmReconciliationContractError::CursorAccountScopeMismatch
            | PmReconciliationContractError::InstrumentMismatch,
        )
        | PmPrivateMonitorError::Account(
            PmAccountPositionRoleError::WrongSource
            | PmAccountPositionRoleError::SourceAccountMismatch
            | PmAccountPositionRoleError::DomainChainMismatch
            | PmAccountPositionRoleError::SignerFunderMismatch
            | PmAccountPositionRoleError::AccountScopeMismatch
            | PmAccountPositionRoleError::SpenderAccountMismatch
            | PmAccountPositionRoleError::SpenderChainMismatch
            | PmAccountPositionRoleError::InstrumentMismatch,
        )
        | PmPrivateMonitorError::PrivateDeliveryOwnerMismatch
        | PmPrivateMonitorError::ReconciliationDeliveryOwnerMismatch
        | PmPrivateMonitorError::AccountDeliveryOwnerMismatch
        | PmPrivateMonitorError::DeliveryScopeMismatch
        | PmPrivateMonitorError::PrivateEpochMismatch => PmPrivateExternalIngressFailure::Scope,
        PmPrivateMonitorError::PrivateBatchPartial { source, .. } => classify_monitor_error(source),
        PmPrivateMonitorError::Reconciliation(_)
        | PmPrivateMonitorError::Account(_)
        | PmPrivateMonitorError::State(_)
        | PmPrivateMonitorError::UnresolvedFill(_)
        | PmPrivateMonitorError::OpenOrderMissingVenueIdentity
        | PmPrivateMonitorError::BatchCounterOverflow
        | PmPrivateMonitorError::DuplicateBatchIdentity => {
            PmPrivateExternalIngressFailure::Contract
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Error)]
pub enum PmPrivateMonitorInputError {
    #[error("fixture completion belongs to another requested connection epoch")]
    CompletionEpochMismatch,
    #[error("fixture completion snapshot differs from the requested snapshot")]
    CompletionSnapshotMismatch,
    #[error("fixture request/completion causal boundary is invalid: {0}")]
    Boundary(#[from] PmAggregateError),
    #[error("fixture completion service time is invalid: {0}")]
    ServiceClock(#[from] EnvelopeError),
}

#[derive(Debug, Error)]
pub enum PmPrivateMonitorError {
    #[error("private fixture batch failed after visible partial progress {applied:?}: {source}")]
    PrivateBatchPartial {
        applied: PmPrivateBatchApply,
        #[source]
        source: Box<PmPrivateMonitorError>,
    },
    #[error(transparent)]
    PrivateNormalization(#[from] PmPrivateNormalizationError),
    #[error(transparent)]
    Reconciliation(#[from] PmReconciliationContractError),
    #[error(transparent)]
    Account(#[from] PmAccountPositionRoleError),
    #[error(transparent)]
    Envelope(#[from] EnvelopeError),
    #[error(transparent)]
    State(#[from] PmPrivateStateError),
    #[error(transparent)]
    UnresolvedFill(#[from] PmUnresolvedFillStateError),
    #[error("serviced private delivery belongs to another role owner")]
    PrivateDeliveryOwnerMismatch,
    #[error("serviced reconciliation delivery belongs to another role owner")]
    ReconciliationDeliveryOwnerMismatch,
    #[error("serviced account delivery belongs to another role owner")]
    AccountDeliveryOwnerMismatch,
    #[error("serviced delivery differs from the monitor's exact account/instrument scope")]
    DeliveryScopeMismatch,
    #[error("fixture read belongs to an epoch that is not active on the private role")]
    PrivateEpochMismatch,
    #[error("complete open-order fixture row lacks an exact venue identity")]
    OpenOrderMissingVenueIdentity,
    #[error("private fixture batch counter overflowed")]
    BatchCounterOverflow,
    #[error("one private fixture batch repeats an order or exact fill identity")]
    DuplicateBatchIdentity,
}
