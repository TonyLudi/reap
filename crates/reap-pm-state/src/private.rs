use reap_pm_core::{
    ConnectionEpoch, EventEnvelope, PmAccountHandle, PmCompleteAccountSnapshot,
    PmCompleteFillQuery, PmCompleteOpenOrdersSnapshot, PmExactOrderDetail, PmFillEvent,
    PmFillSettlementStatus, PmInstrumentHandle, PmOrderEvent, PmProductSource,
    PmReconciliationRequestBoundary, PmSignedUnits, PmVenueOrderKey,
};
use thiserror::Error;

use crate::account::{
    PmAccountCounters, PmAccountSnapshotApply, PmAccountSnapshotProjection, PmAccountState,
    PmAccountStateError, PmAllowanceKnowledge, PmObservedAmount, PmPositionKnowledge,
};
use crate::fill_state::{
    PmFillApply, PmFillCounters, PmFillProjection, PmFillState, PmFillStateError,
    PmProvisionalDeltas,
};
use crate::order_state::{
    PmOpenOrderReservation, PmOpenOrdersApply, PmOrderApply, PmOrderCounters, PmOrderOwnership,
    PmOrderProjection, PmOrderState, PmOrderStateError, PmOwnedOrderRegistration,
    PmRemoteOrderKnowledge, PmReservationBasis, PmReservationKnowledge, PmReservationTotalsError,
};
use crate::private_config::{PmPrivateConfigError, PmPrivateStateConfig};
use crate::private_ingress::{PmPrivateExternalIngressCounters, PmPrivateExternalIngressFault};
use crate::private_occurrence::PmPrivateOccurrence;
use crate::private_readiness::{
    PmPrivateConvergence, PmPrivateHaltReason, PmPrivateQuoteRequest, PmPrivateReadiness,
    PmReadinessContext,
};
use crate::refresh::{
    PmRefreshAdmission, PmRefreshCompletion, PmRefreshCounters, PmRefreshError, PmRefreshKey,
    PmRefreshOwnerId, PmRefreshReason, PmRefreshRequired, PmRefreshState, PmRefreshTicket,
};
use crate::risk::{
    PmRiskCandidate, PmRiskCounters, PmRiskDecision, PmRiskDependencies, PmRiskDependency,
    PmRiskExposure, PmRiskLimits, PmRiskReason as StateRiskReason, PmRiskState,
};
use crate::unresolved_fill::{
    PmUnresolvedFillApply, PmUnresolvedFillCounters, PmUnresolvedFillObservation,
    PmUnresolvedFillProjection, PmUnresolvedFillState, PmUnresolvedFillStateError,
};

/// The sole by-value owner of canonical PM private/account state.
///
/// It exposes immutable projections and narrow transitions, never mutable
/// substate, runtime, queue, adapter, transport, or IO capability.
pub struct PmPrivateState {
    owner: PmRefreshOwnerId,
    config: PmPrivateStateConfig,
    account: PmAccountState,
    orders: PmOrderState,
    fills: PmFillState,
    unresolved_fills: PmUnresolvedFillState,
    refresh: PmRefreshState,
    risk: PmRiskState,
    risk_limits: PmRiskLimits,
    current_epoch: Option<ConnectionEpoch>,
    private_available: bool,
    private_observed_ns: Option<u64>,
    convergence: PmPrivateConvergence,
    halt: Option<PmPrivateHaltReason>,
    external_ingress_counters: PmPrivateExternalIngressCounters,
}

impl PmPrivateState {
    pub fn new(
        config: PmPrivateStateConfig,
        risk_limits: PmRiskLimits,
    ) -> Result<Self, PmPrivateStateError> {
        let owner = PmRefreshOwnerId::allocate()?;
        let account = PmAccountState::new(&config);
        let orders = PmOrderState::new();
        let fills = PmFillState::new();
        let unresolved_fills = PmUnresolvedFillState::new();
        let refresh = PmRefreshState::new(owner);
        let risk = PmRiskState::new(config.account(), config.instrument(), risk_limits);
        Ok(Self {
            owner,
            config,
            account,
            orders,
            fills,
            unresolved_fills,
            refresh,
            risk,
            risk_limits,
            current_epoch: None,
            private_available: false,
            private_observed_ns: None,
            convergence: PmPrivateConvergence::Uninitialized,
            halt: None,
            external_ingress_counters: PmPrivateExternalIngressCounters::default(),
        })
    }

    #[must_use]
    pub const fn account(&self) -> PmAccountHandle {
        self.config.account()
    }

    #[must_use]
    pub const fn instrument(&self) -> PmInstrumentHandle {
        self.config.instrument()
    }

    #[must_use]
    pub const fn source(&self) -> PmProductSource {
        self.config.source()
    }

    #[must_use]
    pub const fn convergence(&self) -> PmPrivateConvergence {
        self.convergence
    }

    #[must_use]
    pub const fn halt(&self) -> Option<PmPrivateHaltReason> {
        self.halt
    }

    pub fn observe_reconnect(
        &mut self,
        epoch: ConnectionEpoch,
        monotonic_observed_ns: u64,
    ) -> Result<(), PmPrivateStateError> {
        self.validate_reconnect(epoch, monotonic_observed_ns)?;
        self.enter_epoch(epoch, monotonic_observed_ns)?;
        Ok(())
    }

    pub fn validate_reconnect(
        &self,
        epoch: ConnectionEpoch,
        monotonic_observed_ns: u64,
    ) -> Result<(), PmPrivateStateError> {
        if epoch.value() == 0 || monotonic_observed_ns == 0 {
            return Err(PmPrivateStateError::InvalidReconnectEvidence);
        }
        if self.current_epoch.is_some_and(|current| epoch <= current) {
            return Err(PmPrivateStateError::OldConnectionEpoch);
        }
        Ok(())
    }

    pub fn record_external_ingress_fault(&mut self, fault: PmPrivateExternalIngressFault) {
        self.external_ingress_counters.record(fault);
        if self.halt.is_none() || self.halt == Some(PmPrivateHaltReason::ContractViolation) {
            self.halt = Some(PmPrivateHaltReason::ExternalIngressFault(fault));
        }
        self.convergence = PmPrivateConvergence::Divergent {
            uncovered_fills: self.uncovered_fill_count(),
        };
        let _refresh = self.refresh.require(PmRefreshKey::new(
            self.account(),
            self.instrument(),
            PmRefreshReason::ExternalIngressFault,
        ));
    }

    pub fn register_owned_order(
        &mut self,
        registration: PmOwnedOrderRegistration,
    ) -> Result<(), PmPrivateStateError> {
        self.orders.register_owned(registration, &self.config)?;
        Ok(())
    }

    pub fn observe_order(
        &mut self,
        envelope: EventEnvelope<PmOrderEvent>,
        knowledge: PmRemoteOrderKnowledge,
    ) -> Result<PmOrderApply, PmPrivateStateError> {
        let identity = envelope.payload().order();
        self.prepare_private_event_epoch(
            envelope.ordering().connection_epoch(),
            envelope.clock().monotonic_service_ns(),
        )?;
        let result = self.orders.observe(envelope, knowledge, &self.config);
        match result {
            Ok(outcome) => {
                self.private_observed_ns = self.orders.observed_monotonic_ns();
                match self
                    .orders
                    .ownership(identity)
                    .expect("successful observation retains canonical order")
                {
                    PmOrderOwnership::ProvenOwned => {}
                    PmOrderOwnership::Unmanaged => {
                        self.require_refresh(PmRefreshReason::UnmanagedOrder)?;
                    }
                    PmOrderOwnership::Ambiguous => {
                        self.require_refresh(PmRefreshReason::AmbiguousOrder)?;
                    }
                }
                Ok(outcome)
            }
            Err(error) => {
                self.record_order_error(error)?;
                Err(error.into())
            }
        }
    }

    pub fn observe_fill(
        &mut self,
        envelope: EventEnvelope<PmFillEvent>,
    ) -> Result<PmFillApply, PmPrivateStateError> {
        self.prepare_private_event_epoch(
            envelope.ordering().connection_epoch(),
            envelope.clock().monotonic_service_ns(),
        )?;
        let result = self.fills.observe(envelope, &self.config);
        match result {
            Ok(outcome) => {
                self.private_observed_ns = self.fills.observed_monotonic_ns();
                if matches!(outcome, PmFillApply::PrincipalApplied { .. }) {
                    self.convergence = PmPrivateConvergence::Divergent {
                        uncovered_fills: self.uncovered_fill_count(),
                    };
                    self.require_refresh(PmRefreshReason::FillObserved)?;
                }
                match outcome {
                    PmFillApply::PrincipalApplied { fee, settlement }
                    | PmFillApply::Enriched { fee, settlement } => {
                        self.require_for_fee(fee)?;
                        self.require_for_settlement(settlement)?;
                    }
                    PmFillApply::Duplicate | PmFillApply::IgnoredStale => {}
                }
                Ok(outcome)
            }
            Err(error) => {
                self.record_fill_error(error)?;
                Err(error.into())
            }
        }
    }

    pub fn observe_unresolved_fill(
        &mut self,
        envelope: EventEnvelope<PmUnresolvedFillObservation>,
    ) -> Result<PmUnresolvedFillApply, PmPrivateStateError> {
        self.prepare_private_event_epoch(
            envelope.ordering().connection_epoch(),
            envelope.clock().monotonic_service_ns(),
        )?;
        let result = self.unresolved_fills.observe(envelope, &self.config);
        match result {
            Ok(outcome) => {
                self.private_observed_ns = self.unresolved_fills.observed_monotonic_ns();
                match outcome {
                    PmUnresolvedFillApply::Inserted(_)
                    | PmUnresolvedFillApply::SettlementAdvanced { .. } => {
                        self.convergence = PmPrivateConvergence::Divergent {
                            uncovered_fills: self.uncovered_fill_count(),
                        };
                        self.require_refresh(PmRefreshReason::UnresolvedFill)?;
                        if let PmUnresolvedFillApply::SettlementAdvanced { settlement, .. } =
                            outcome
                        {
                            self.require_for_settlement(settlement)?;
                        }
                    }
                    PmUnresolvedFillApply::Duplicate(_)
                    | PmUnresolvedFillApply::IgnoredStale(_) => {}
                }
                Ok(outcome)
            }
            Err(error) => {
                self.record_unresolved_fill_error(error)?;
                Err(error.into())
            }
        }
    }

    pub fn apply_account_snapshot(
        &mut self,
        envelope: EventEnvelope<PmCompleteAccountSnapshot>,
    ) -> Result<PmAccountSnapshotApply, PmPrivateStateError> {
        self.require_private_epoch(envelope.ordering().connection_epoch())?;
        let outcome = match self.account.apply(&envelope, &self.config) {
            Ok(outcome) => outcome,
            Err(error) => {
                self.record_account_error()?;
                return Err(error.into());
            }
        };
        if matches!(outcome, PmAccountSnapshotApply::Applied { .. }) {
            self.convergence = PmPrivateConvergence::Divergent {
                uncovered_fills: self.uncovered_fill_count(),
            };
            self.require_refresh(PmRefreshReason::AccountDivergence)?;
            self.require_for_account_state()?;
        }
        Ok(outcome)
    }

    pub fn apply_open_orders_snapshot(
        &mut self,
        envelope: EventEnvelope<PmCompleteOpenOrdersSnapshot>,
        reservations: &[PmOpenOrderReservation],
    ) -> Result<PmOpenOrdersApply, PmPrivateStateError> {
        self.require_private_epoch(envelope.ordering().connection_epoch())?;
        let result = self
            .orders
            .apply_open_snapshot(envelope, reservations, &self.config);
        match result {
            Ok(outcome) => {
                if matches!(
                    outcome,
                    PmOpenOrdersApply::Applied {
                        retained_missing: 1..,
                        ..
                    }
                ) {
                    self.require_refresh(PmRefreshReason::MissingOrderDetail)?;
                }
                Ok(outcome)
            }
            Err(error) => {
                self.record_order_error(error)?;
                Err(error.into())
            }
        }
    }

    pub fn apply_order_detail(
        &mut self,
        envelope: EventEnvelope<PmExactOrderDetail>,
        reservation: PmReservationKnowledge,
    ) -> Result<PmOrderApply, PmPrivateStateError> {
        self.require_private_epoch(envelope.ordering().connection_epoch())?;
        let result = self
            .orders
            .apply_detail(envelope, reservation, &self.config);
        match result {
            Ok(outcome) => Ok(outcome),
            Err(error) => {
                self.record_order_error(error)?;
                Err(error.into())
            }
        }
    }

    pub fn apply_reconciliation(
        &mut self,
        account: EventEnvelope<PmCompleteAccountSnapshot>,
        fills: EventEnvelope<PmCompleteFillQuery>,
    ) -> Result<PmReconciliationApply, PmPrivateStateError> {
        if let Err(error) = validate_reconciliation_pair(&account, &fills) {
            self.record_account_error()?;
            return Err(error);
        }
        let epoch = account.ordering().connection_epoch();
        let observed_ns = account
            .clock()
            .monotonic_service_ns()
            .max(fills.clock().monotonic_service_ns());
        self.require_private_epoch(epoch)?;
        let preview = match self.account.preview(&account, &self.config) {
            Ok(preview) => preview,
            Err(error) => {
                self.record_account_error()?;
                return Err(error.into());
            }
        };
        if !matches!(preview, PmAccountSnapshotApply::Applied { .. }) {
            return Ok(PmReconciliationApply::NotApplied(preview));
        }
        if let Err(error) = self.fills.preflight_query(&fills, &self.config) {
            self.record_fill_error(error)?;
            return Err(error.into());
        }
        if let Err(error) = self.fills.apply_preflighted_query(fills, &self.config) {
            self.record_fill_error(error)?;
            return Err(error.into());
        }
        let account_outcome = match self.account.apply(&account, &self.config) {
            Ok(outcome) => outcome,
            Err(error) => {
                self.record_account_error()?;
                return Err(error.into());
            }
        };
        let boundary = account.payload().boundary();
        let request_occurrence = PmPrivateOccurrence::new(epoch, boundary.request_sequence());
        let completion_occurrence = PmPrivateOccurrence::new(epoch, boundary.completion_sequence());
        self.unresolved_fills
            .cover_through(request_occurrence, completion_occurrence);
        self.private_available = true;
        let uncovered = self.uncovered_fill_count();
        self.convergence = if uncovered == 0 {
            PmPrivateConvergence::Converged {
                boundary,
                observed_monotonic_ns: observed_ns,
            }
        } else {
            PmPrivateConvergence::Divergent {
                uncovered_fills: uncovered,
            }
        };
        if uncovered > 0 {
            self.require_refresh(PmRefreshReason::AccountDivergence)?;
        }
        self.require_for_account_state()?;
        self.risk.recover_after_complete_reconciliation();
        Ok(PmReconciliationApply::Applied {
            boundary,
            account: account_outcome,
            uncovered_fills: uncovered,
        })
    }

    #[must_use]
    pub fn quote_readiness(&self, request: PmPrivateQuoteRequest) -> PmPrivateReadiness {
        if request
            .reservation()
            .validate_for(request.side(), request.price(), request.quantity())
            .is_err()
            || request.reservation().basis() != PmReservationBasis::PolicyApprovedWorstCase
        {
            return PmPrivateReadiness::Blocked(
                crate::private_readiness::PmPrivateReadinessReason::ArithmeticInvalid,
            );
        }
        crate::private_readiness::check(
            PmReadinessContext {
                config: &self.config,
                account: &self.account,
                orders: &self.orders,
                fills: &self.fills,
                unresolved_fills: &self.unresolved_fills,
                convergence: self.convergence,
                private_observed_ns: self.private_observed_ns,
                private_available: self.private_available,
                halt: self.halt,
                risk_halt: self.active_risk_halt(),
                full_reconcile_required: self.refresh.full_reconcile_required(),
                freshness: self.risk_limits.freshness(),
            },
            request,
        )
    }

    pub fn evaluate_risk_candidate(
        &mut self,
        request: PmPrivateQuoteRequest,
        reference: PmRiskDependency,
        book: PmRiskDependency,
    ) -> Result<PmRiskDecision, PmPrivateStateError> {
        request
            .reservation()
            .validate_for(request.side(), request.price(), request.quantity())?;
        if request.reservation().basis() != PmReservationBasis::PolicyApprovedWorstCase {
            return Err(PmOrderStateError::OwnedReservationRequiresPolicy.into());
        }
        let (reserved_collateral, reserved_outcome) =
            self.orders
                .reservation_totals()
                .map_err(|error| match error {
                    PmReservationTotalsError::Unknown(_) => {
                        PmPrivateStateError::CanonicalExposureUnavailable
                    }
                    PmReservationTotalsError::Overflow => PmPrivateStateError::ArithmeticOverflow,
                })?;
        let inventory = self
            .effective_inventory()
            .ok_or(PmPrivateStateError::CanonicalInventoryUnavailable)?;
        let dependencies = PmRiskDependencies::new(
            reference,
            book,
            dependency(self.private_available, self.private_observed_ns),
            dependency(true, self.account.observed_monotonic_ns()),
            dependency(true, self.orders.observed_monotonic_ns()),
            dependency(
                matches!(self.convergence, PmPrivateConvergence::Converged { .. }),
                convergence_observed(self.convergence),
            ),
        );
        let candidate = PmRiskCandidate::new(
            self.account(),
            self.instrument(),
            request.side(),
            request.quantity(),
            request.reservation().collateral(),
            request.reservation().outcome(),
        );
        let exposure = PmRiskExposure::new(
            inventory,
            reserved_collateral,
            reserved_collateral,
            reserved_outcome,
            self.orders.live_count(),
            self.orders.unresolved_count(),
            self.unresolved_risk_count(),
        );
        let decision = self.risk.evaluate(crate::risk::PmRiskInput::new(
            request.monotonic_now_ns(),
            candidate,
            exposure,
            dependencies,
        ));
        if decision.halt_scope().cancel_owned_required() {
            self.require_refresh(PmRefreshReason::RiskBreach)?;
        }
        Ok(decision)
    }

    pub fn owned_cancel_intents(&self) -> impl Iterator<Item = PmCancelOwnedIntent> + '_ {
        let reason = self.cancel_reason();
        self.orders
            .owned_live_venue_orders()
            .filter_map(move |venue_order| {
                reason.map(|reason| PmCancelOwnedIntent {
                    venue_order,
                    reason,
                })
            })
    }

    pub fn orders(&self) -> impl Iterator<Item = PmOrderProjection> + '_ {
        self.orders.projections()
    }

    pub fn fills(&self) -> impl Iterator<Item = PmFillProjection> + '_ {
        self.fills.projections()
    }

    pub fn unresolved_fills(&self) -> impl Iterator<Item = PmUnresolvedFillProjection> + '_ {
        self.unresolved_fills.projections()
    }

    #[must_use]
    pub const fn account_projection(&self) -> PmAccountSnapshotProjection {
        self.account.projection()
    }

    pub fn allowance(
        &self,
        spender: reap_pm_core::PmSpenderId,
    ) -> crate::account::PmAllowanceKnowledge {
        self.account.allowance(spender)
    }

    pub fn diagnostic_balance_rows(
        &self,
    ) -> impl Iterator<Item = reap_pm_core::PmBalanceEvent> + '_ {
        self.account.diagnostic_balances()
    }

    pub fn diagnostic_allowance_rows(
        &self,
    ) -> impl Iterator<Item = reap_pm_core::PmAllowanceEvent> + '_ {
        self.account.diagnostic_allowances()
    }

    pub fn diagnostic_position_rows(
        &self,
    ) -> impl Iterator<Item = reap_pm_core::PmPositionEvent> + '_ {
        self.account.diagnostic_positions()
    }

    #[must_use]
    pub const fn provisional_deltas(&self) -> PmProvisionalDeltas {
        self.fills.provisional()
    }

    #[must_use]
    pub const fn fill_watermark(&self) -> Option<reap_pm_core::PmFillQueryCursor> {
        self.fills.watermark()
    }

    pub fn require_refresh(
        &mut self,
        reason: PmRefreshReason,
    ) -> Result<PmRefreshRequired, PmPrivateStateError> {
        Ok(self
            .refresh
            .require(PmRefreshKey::new(self.account(), self.instrument(), reason))?)
    }

    pub fn mark_refresh_admitted(
        &mut self,
        ticket: PmRefreshTicket,
    ) -> Result<PmRefreshAdmission, PmPrivateStateError> {
        self.validate_refresh_scope(ticket)?;
        Ok(self.refresh.mark_admitted(ticket))
    }

    pub fn complete_refresh(
        &mut self,
        ticket: PmRefreshTicket,
    ) -> Result<PmRefreshCompletion, PmPrivateStateError> {
        self.validate_refresh_scope(ticket)?;
        Ok(self.refresh.complete(ticket))
    }

    pub fn pending_refreshes(&self) -> impl Iterator<Item = PmRefreshTicket> + '_ {
        self.refresh.pending()
    }

    pub fn pending_refresh_keys(&self) -> impl Iterator<Item = PmRefreshKey> + '_ {
        self.refresh.pending_keys()
    }

    #[must_use]
    pub const fn pending_refresh_count(&self) -> usize {
        self.refresh.len()
    }

    #[must_use]
    pub const fn full_reconcile_required(&self) -> bool {
        self.refresh.full_reconcile_required()
    }

    #[must_use]
    pub const fn refresh_counters(&self) -> PmRefreshCounters {
        self.refresh.counters()
    }

    #[must_use]
    pub const fn account_counters(&self) -> PmAccountCounters {
        self.account.counters()
    }

    #[must_use]
    pub const fn order_counters(&self) -> PmOrderCounters {
        self.orders.counters()
    }

    #[must_use]
    pub const fn fill_counters(&self) -> PmFillCounters {
        self.fills.counters()
    }

    #[must_use]
    pub const fn unresolved_fill_counters(&self) -> PmUnresolvedFillCounters {
        self.unresolved_fills.counters()
    }

    #[must_use]
    pub const fn risk_counters(&self) -> PmRiskCounters {
        self.risk.counters()
    }

    #[must_use]
    pub const fn external_ingress_counters(&self) -> PmPrivateExternalIngressCounters {
        self.external_ingress_counters
    }

    #[must_use]
    pub const fn market_halt(&self) -> Option<StateRiskReason> {
        self.risk.market_halt()
    }

    #[must_use]
    pub const fn account_halt(&self) -> Option<StateRiskReason> {
        self.risk.account_halt()
    }

    #[must_use]
    pub const fn global_halt(&self) -> Option<StateRiskReason> {
        self.risk.global_halt()
    }

    fn prepare_private_event_epoch(
        &mut self,
        epoch: ConnectionEpoch,
        monotonic_ns: u64,
    ) -> Result<(), PmPrivateStateError> {
        match self.current_epoch {
            Some(current) if epoch < current => Err(PmPrivateStateError::OldConnectionEpoch),
            Some(current) if epoch == current => Ok(()),
            Some(_) => self.enter_epoch(epoch, monotonic_ns),
            None => self.enter_epoch(epoch, monotonic_ns),
        }
    }

    fn require_private_epoch(&self, epoch: ConnectionEpoch) -> Result<(), PmPrivateStateError> {
        match self.current_epoch {
            None => Err(PmPrivateStateError::MissingPrivateEpochEvidence),
            Some(current) if epoch < current => Err(PmPrivateStateError::OldConnectionEpoch),
            Some(current) if epoch > current => Err(PmPrivateStateError::PrivateEpochMismatch),
            Some(_) => Ok(()),
        }
    }

    fn enter_epoch(
        &mut self,
        epoch: ConnectionEpoch,
        monotonic_ns: u64,
    ) -> Result<(), PmPrivateStateError> {
        self.require_refresh(PmRefreshReason::PrivateReconnect)?;
        self.current_epoch = Some(epoch);
        self.private_available = false;
        self.private_observed_ns = Some(monotonic_ns);
        self.orders.invalidate_freshness();
        self.convergence = PmPrivateConvergence::Divergent {
            uncovered_fills: self.uncovered_fill_count(),
        };
        Ok(())
    }

    fn require_for_fee(
        &mut self,
        fee: crate::fill_state::PmFillFeeState,
    ) -> Result<(), PmPrivateStateError> {
        match fee {
            crate::fill_state::PmFillFeeState::Unknown
            | crate::fill_state::PmFillFeeState::Incomplete => {
                self.require_refresh(PmRefreshReason::FillFeeUnknown)?;
            }
            crate::fill_state::PmFillFeeState::UnmappedAsset { .. } => {
                self.require_refresh(PmRefreshReason::FillConflict)?;
            }
            crate::fill_state::PmFillFeeState::Known { .. } => {}
        }
        Ok(())
    }

    fn require_for_settlement(
        &mut self,
        settlement: PmFillSettlementStatus,
    ) -> Result<(), PmPrivateStateError> {
        match settlement {
            PmFillSettlementStatus::Retrying => {
                self.require_refresh(PmRefreshReason::FillSettlementRetrying)?;
            }
            PmFillSettlementStatus::Failed => {
                self.require_refresh(PmRefreshReason::FillSettlementFailed)?;
            }
            PmFillSettlementStatus::Matched
            | PmFillSettlementStatus::Mined
            | PmFillSettlementStatus::Confirmed => {}
        }
        Ok(())
    }

    fn require_for_account_state(&mut self) -> Result<(), PmPrivateStateError> {
        if matches!(
            self.account.position(),
            PmPositionKnowledge::Unavailable
                | PmPositionKnowledge::ResolvedUnredeemed(_)
                | PmPositionKnowledge::VenueUnavailable(_)
        ) {
            self.require_refresh(PmRefreshReason::PositionUnavailable)?;
        }
        if self
            .config
            .required_spenders()
            .iter()
            .copied()
            .any(|spender| {
                matches!(
                    self.account.allowance(spender),
                    PmAllowanceKnowledge::Unconfigured
                        | PmAllowanceKnowledge::Unavailable
                        | PmAllowanceKnowledge::ExplicitAbsent
                )
            })
        {
            self.require_refresh(PmRefreshReason::AllowanceUnavailable)?;
        }
        let balance = self.account.outcome_balance();
        let position = self.account.position();
        if matches!(
            (balance, position),
            (
                PmObservedAmount::ExplicitAbsent | PmObservedAmount::Present(_),
                PmPositionKnowledge::ExplicitAbsent | PmPositionKnowledge::Tradable(_)
            )
        ) && balance.value() != position.tradable_units()
        {
            self.require_refresh(PmRefreshReason::AccountDivergence)?;
        }
        Ok(())
    }

    fn record_order_error(&mut self, error: PmOrderStateError) -> Result<(), PmPrivateStateError> {
        self.halt.get_or_insert(match error {
            PmOrderStateError::Capacity => PmPrivateHaltReason::OrderCapacity,
            _ => PmPrivateHaltReason::ContractViolation,
        });
        self.require_refresh(if error == PmOrderStateError::Capacity {
            PmRefreshReason::StateCapacity
        } else {
            PmRefreshReason::AmbiguousOrder
        })?;
        Ok(())
    }

    fn record_account_error(&mut self) -> Result<(), PmPrivateStateError> {
        self.halt
            .get_or_insert(PmPrivateHaltReason::ContractViolation);
        self.require_refresh(PmRefreshReason::AccountSnapshotStale)?;
        Ok(())
    }

    fn record_fill_error(&mut self, error: PmFillStateError) -> Result<(), PmPrivateStateError> {
        self.halt.get_or_insert(match error {
            PmFillStateError::Capacity => PmPrivateHaltReason::FillCapacity,
            PmFillStateError::ArithmeticOverflow => PmPrivateHaltReason::ArithmeticOverflow,
            _ => PmPrivateHaltReason::ContractViolation,
        });
        self.require_refresh(if error == PmFillStateError::Capacity {
            PmRefreshReason::StateCapacity
        } else {
            PmRefreshReason::FillConflict
        })?;
        Ok(())
    }

    fn record_unresolved_fill_error(
        &mut self,
        error: PmUnresolvedFillStateError,
    ) -> Result<(), PmPrivateStateError> {
        self.halt.get_or_insert(match error {
            PmUnresolvedFillStateError::Capacity => PmPrivateHaltReason::UnresolvedFillCapacity,
            _ => PmPrivateHaltReason::ContractViolation,
        });
        self.require_refresh(if error == PmUnresolvedFillStateError::Capacity {
            PmRefreshReason::StateCapacity
        } else {
            PmRefreshReason::UnresolvedFill
        })?;
        Ok(())
    }

    fn validate_refresh_scope(&self, ticket: PmRefreshTicket) -> Result<(), PmPrivateStateError> {
        if ticket.owner() != self.owner
            || ticket.key().account() != self.account()
            || ticket.key().instrument() != self.instrument()
        {
            Err(PmPrivateStateError::RefreshScopeMismatch)
        } else {
            Ok(())
        }
    }

    fn active_risk_halt(&self) -> Option<StateRiskReason> {
        self.risk
            .global_halt()
            .or(self.risk.account_halt())
            .or(self.risk.market_halt())
    }

    fn effective_inventory(&self) -> Option<PmSignedUnits> {
        let published = self.account.position().tradable_units()?;
        if self.account.outcome_balance().value()? != published {
            return None;
        }
        let signed_published = if published.is_zero() {
            PmSignedUnits::ZERO
        } else {
            PmSignedUnits::from_parts(reap_pm_core::PmSign::Positive, published).ok()?
        };
        crate::fill_state::add_signed(signed_published, self.fills.provisional().outcome()).ok()
    }

    fn uncovered_fill_count(&self) -> u16 {
        self.fills
            .provisional()
            .uncovered_fills()
            .checked_add(self.unresolved_fills.active_count())
            .expect("configured fill bounds fit u16")
    }

    fn unresolved_risk_count(&self) -> u16 {
        self.fills
            .unresolved_count()
            .checked_add(self.unresolved_fills.active_count())
            .expect("configured fill bounds fit u16")
    }

    fn cancel_reason(&self) -> Option<PmCancelOwnedReason> {
        if let Some(reason) = self.risk.global_halt() {
            return Some(PmCancelOwnedReason::Risk(reason));
        }
        if let Some(reason) = self.risk.account_halt() {
            return Some(PmCancelOwnedReason::Risk(reason));
        }
        if let Some(reason) = self.risk.market_halt() {
            return Some(PmCancelOwnedReason::Risk(reason));
        }
        self.halt.map(PmCancelOwnedReason::Private)
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum PmReconciliationApply {
    Applied {
        boundary: PmReconciliationRequestBoundary,
        account: PmAccountSnapshotApply,
        uncovered_fills: u16,
    },
    NotApplied(PmAccountSnapshotApply),
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum PmCancelOwnedReason {
    Risk(StateRiskReason),
    Private(PmPrivateHaltReason),
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct PmCancelOwnedIntent {
    venue_order: PmVenueOrderKey,
    reason: PmCancelOwnedReason,
}

impl PmCancelOwnedIntent {
    #[must_use]
    pub const fn venue_order(self) -> PmVenueOrderKey {
        self.venue_order
    }

    #[must_use]
    pub const fn reason(self) -> PmCancelOwnedReason {
        self.reason
    }
}

#[derive(Debug, Error)]
pub enum PmPrivateStateError {
    #[error("refresh ticket belongs to another PM owner, account, or instrument")]
    RefreshScopeMismatch,
    #[error("private event belongs to an older connection epoch")]
    OldConnectionEpoch,
    #[error("read-only PM input arrived before any private epoch evidence")]
    MissingPrivateEpochEvidence,
    #[error("read-only PM input cannot create or advance the private connection epoch")]
    PrivateEpochMismatch,
    #[error("private reconnect evidence must have nonzero epoch and monotonic time")]
    InvalidReconnectEvidence,
    #[error("account and fill reconciliation results do not share one exact cut")]
    ReconciliationPairMismatch,
    #[error("canonical exposure is unavailable because an exact reservation is unknown")]
    CanonicalExposureUnavailable,
    #[error("canonical inventory is unavailable or published PM inventory facts disagree")]
    CanonicalInventoryUnavailable,
    #[error("canonical exact exposure arithmetic overflowed")]
    ArithmeticOverflow,
    #[error(transparent)]
    Config(#[from] PmPrivateConfigError),
    #[error(transparent)]
    Refresh(#[from] PmRefreshError),
    #[error(transparent)]
    Account(#[from] PmAccountStateError),
    #[error(transparent)]
    Order(#[from] PmOrderStateError),
    #[error(transparent)]
    Fill(#[from] PmFillStateError),
    #[error(transparent)]
    UnresolvedFill(#[from] PmUnresolvedFillStateError),
}

fn validate_reconciliation_pair(
    account: &EventEnvelope<PmCompleteAccountSnapshot>,
    fills: &EventEnvelope<PmCompleteFillQuery>,
) -> Result<(), PmPrivateStateError> {
    if account.source() != fills.source()
        || account.connection_id() != fills.connection_id()
        || account.ordering().connection_epoch() != fills.ordering().connection_epoch()
        || account.payload().account_scope() != fills.payload().account_scope()
        || account.payload().boundary() != fills.payload().boundary()
        || account.payload().snapshot() != fills.payload().snapshot()
    {
        Err(PmPrivateStateError::ReconciliationPairMismatch)
    } else {
        Ok(())
    }
}

fn dependency(available: bool, observed_ns: Option<u64>) -> PmRiskDependency {
    match (available, observed_ns) {
        (true, Some(observed)) => PmRiskDependency::available(observed),
        (false, _) | (true, None) => PmRiskDependency::unavailable(),
    }
}

const fn convergence_observed(convergence: PmPrivateConvergence) -> Option<u64> {
    match convergence {
        PmPrivateConvergence::Converged {
            observed_monotonic_ns,
            ..
        } => Some(observed_monotonic_ns),
        PmPrivateConvergence::Uninitialized | PmPrivateConvergence::Divergent { .. } => None,
    }
}
