use std::error::Error;
use std::fmt;
use std::path::PathBuf;

use reap_pm_core::PmOrderSide;
use reap_pm_live_contracts::{PmConnectivityConfig, PmConnectivityConfigError};
use reap_pm_state::{PmPrivateExternalIngressFault, PmRiskHaltScope};
use reap_pm_strategy::PmQuoteModel;
use reap_polymarket_adapter::{
    PmFakeCancelScript, PmFakePlaceScript, PmFixtureCompletionOccurrence, PmFixtureFeeEvidence,
};

use super::{PmProduct, PmProductPublicIngress};
use crate::capture::{PmCaptureProvenance, PmCaptureSessionPolicy};
use crate::composition::{
    PmPublicAgedLaneEnactError, PmPublicAgedLaneFaultEnactment, PmPublicCapture,
    PmPublicCaptureOutcome, PmPublicCaptureRun, PmPublicCaptureRunError,
};
use crate::coordinator::{
    PmControlReason, PmCoordinator, PmCoordinatorCounters, PmCoordinatorError, PmCoordinatorPolicy,
    PmCoordinatorShutdownError, PmCoordinatorStartError, PmFakeEffectMetrics, PmMutationCounters,
    PmMutationHalt, PmPersistenceMetrics, PmProductEffect, PmProductEffectMetrics,
    PmRefreshObligationMetrics,
};
use crate::journal::PmJournalRecovery;
use crate::lanes::{
    PmCompleteSchedulerMetrics, PmCompleteServiceCounts, PmServiceTurnError, PmTelemetryKind,
    SaturationAction,
};
use crate::private_monitor::{
    PmAccountFixtureInput, PmOpenOrdersFixtureInput, PmOrderDetailFixtureInput,
    PmReconciliationFixtureInput,
};
use crate::schedule::PmScheduledActionKind;

impl<M: PmQuoteModel> PmProduct<M> {
    /// Starts the one integrated PM product owner.
    ///
    /// The active public capture Run, its reducer-backed public lane, the
    /// complete scheduler, private state, journal mutation owner, fake
    /// fixture executor, pure model, and deterministic schedule are moved
    /// under one static coordinator. No sibling public lane is constructed.
    #[allow(
        clippy::too_many_arguments,
        reason = "cold-start paths keep capture, journal, metadata, and explicit timing policy visible"
    )]
    pub async fn start(
        self,
        capture_path: PathBuf,
        journal_path: PathBuf,
        authoritative: reap_polymarket_adapter::PmAuthoritativeMetadata,
        session_policy: PmCaptureSessionPolicy,
        provenance: PmCaptureProvenance,
        coordinator_policy: PmCoordinatorPolicy,
    ) -> Result<(PmProductRun<M>, PmJournalRecovery), PmProductStartError> {
        let public_config = self
            .plan
            .public_config()
            .expect("product plan carries public config")
            .clone();
        let account_config = self
            .plan
            .account_config()
            .expect("product plan carries account config")
            .clone();
        let config = PmConnectivityConfig::new(public_config, account_config)
            .map_err(PmProductStartError::connectivity)?;
        let Self {
            model,
            plan,
            bindings,
            capture,
            private,
            fake_effect,
            schedule,
        } = self;
        let public = PmPublicCapture {
            plan,
            bindings,
            capture,
        }
        .start(capture_path, authoritative, session_policy, provenance)
        .await
        .map_err(PmProductStartError::public)?;
        let (coordinator, recovery) = PmCoordinator::start(
            &config,
            model,
            private,
            fake_effect,
            Box::new(public),
            schedule,
            journal_path,
            coordinator_policy,
        )
        .await
        .map_err(PmProductStartError::coordinator)?;
        Ok((PmProductRun { coordinator }, recovery))
    }
}

/// Active, sole-owner PM product run.
pub struct PmProductRun<M: PmQuoteModel> {
    coordinator: Box<PmCoordinator<M>>,
}

impl<M: PmQuoteModel> PmProductRun<M> {
    #[cfg(test)]
    pub(crate) fn phase6_reach_schedule_full(
        &mut self,
        observed_ns: u64,
        decision_wall_timestamp_ms: u64,
    ) -> Option<(usize, SaturationAction)> {
        self.coordinator
            .phase6_reach_schedule_full(observed_ns, decision_wall_timestamp_ms)
    }

    #[cfg(test)]
    pub(crate) fn phase6_enact_next_schedule_failure_with_observer_clock(
        &mut self,
        schedule_observed_ns: u64,
        observer_service_ns: u64,
    ) -> Option<SaturationAction> {
        self.coordinator
            .phase6_enact_next_schedule_failure_with_observer_clock(
                schedule_observed_ns,
                observer_service_ns,
            )
    }

    /// Borrows the sole capture/session/reducer owner used by the complete
    /// scheduler's public rank.
    #[must_use]
    pub fn public_capture(&self) -> &PmPublicCaptureRun {
        self.coordinator.public_capture()
    }

    /// Borrows only the socket/capture ingress capabilities of the sole
    /// public owner.
    ///
    /// The returned handle cannot service or age the public lane. Those
    /// operations remain exclusively owned by the complete product scheduler.
    pub fn public_ingress(&mut self) -> PmProductPublicIngress<'_> {
        PmProductPublicIngress::new(self.coordinator.public_capture_mut())
    }

    /// Admits one fixture-private connection occurrence. Source and
    /// connection identity come from the product plan; callers cannot inject
    /// either value.
    pub fn connect_private_fixture(
        &mut self,
        occurrence: PmFixtureCompletionOccurrence,
    ) -> Result<(), PmProductRunError> {
        self.coordinator
            .connect_private_fixture(occurrence)
            .map_err(PmProductRunError::service)
    }

    pub fn mark_private_fixture_unavailable(
        &mut self,
        occurrence: PmFixtureCompletionOccurrence,
        fault: PmPrivateExternalIngressFault,
    ) -> Result<(), PmProductRunError> {
        self.coordinator
            .mark_private_fixture_unavailable(occurrence, fault)
            .map_err(PmProductRunError::service)
    }

    /// Normalizes raw fixture-only private lifecycle JSON through the exact
    /// configured role and admits its owner-bound typed batch.
    pub fn ingest_private_fixture(
        &mut self,
        occurrence: PmFixtureCompletionOccurrence,
        raw: &[u8],
        fee: PmFixtureFeeEvidence,
    ) -> Result<(), PmProductRunError> {
        self.coordinator
            .ingest_private_fixture(occurrence, raw, fee)
            .map_err(PmProductRunError::service)
    }

    pub fn ingest_account_fixture(
        &mut self,
        input: PmAccountFixtureInput<'_>,
    ) -> Result<(), PmProductRunError> {
        self.coordinator
            .ingest_account_fixture(input)
            .map_err(PmProductRunError::service)
    }

    pub fn ingest_open_orders_fixture(
        &mut self,
        input: PmOpenOrdersFixtureInput<'_>,
    ) -> Result<(), PmProductRunError> {
        self.coordinator
            .ingest_open_orders_fixture(input)
            .map_err(PmProductRunError::service)
    }

    pub fn ingest_order_detail_fixture(
        &mut self,
        input: PmOrderDetailFixtureInput<'_>,
    ) -> Result<(), PmProductRunError> {
        self.coordinator
            .ingest_order_detail_fixture(input)
            .map_err(PmProductRunError::service)
    }

    pub fn ingest_reconciliation_fixture(
        &mut self,
        input: PmReconciliationFixtureInput<'_>,
    ) -> Result<(), PmProductRunError> {
        self.coordinator
            .ingest_reconciliation_fixture(input)
            .map_err(PmProductRunError::service)
    }

    /// Schedules one action for this Run's configured account and instrument.
    ///
    /// Callers select only the side, action kind, and clocks; product scope is
    /// derived by the sole coordinator owner.
    pub fn schedule(
        &mut self,
        side: PmOrderSide,
        kind: PmScheduledActionKind,
        deadline_ns: u64,
        scheduled_at_ns: u64,
        decision_wall_timestamp_ms: u64,
    ) -> Result<(), PmProductRunError> {
        self.coordinator
            .schedule(
                side,
                kind,
                deadline_ns,
                scheduled_at_ns,
                decision_wall_timestamp_ms,
            )
            .map(|_| ())
            .map_err(PmProductRunError::service)
    }

    /// Polls exactly one pending durable receipt and admits a completed
    /// result, if any, into the persistence rank.
    pub fn poll_persistence_fixture(
        &mut self,
        occurrence: PmFixtureCompletionOccurrence,
        monotonic_poll_ns: u64,
    ) -> Result<bool, PmProductRunError> {
        self.coordinator
            .poll_persistence_fixture(occurrence, monotonic_poll_ns)
            .map_err(PmProductRunError::service)
    }

    /// Executes only the next already-durable prepared fake quote.
    pub fn execute_prepared_quote_fixture(
        &mut self,
        occurrence: PmFixtureCompletionOccurrence,
        script: PmFakePlaceScript,
        monotonic_effect_ns: u64,
    ) -> Result<(), PmProductRunError> {
        self.coordinator
            .execute_prepared_quote_fixture(occurrence, script, monotonic_effect_ns)
            .map_err(PmProductRunError::service)
    }

    /// Executes only the next already-durable prepared owned cancel.
    pub fn execute_prepared_cancel_fixture(
        &mut self,
        occurrence: PmFixtureCompletionOccurrence,
        script: PmFakeCancelScript,
        monotonic_effect_ns: u64,
    ) -> Result<(), PmProductRunError> {
        self.coordinator
            .execute_prepared_cancel_fixture(occurrence, script, monotonic_effect_ns)
            .map_err(PmProductRunError::service)
    }

    pub fn request_shutdown(
        &mut self,
        occurrence: PmFixtureCompletionOccurrence,
    ) -> Result<(), PmProductRunError> {
        self.coordinator
            .request_shutdown(occurrence)
            .map_err(PmProductRunError::service)
    }

    pub fn request_global_stop(
        &mut self,
        occurrence: PmFixtureCompletionOccurrence,
    ) -> Result<(), PmProductRunError> {
        self.coordinator
            .request_global_stop(occurrence)
            .map_err(PmProductRunError::service)
    }

    pub fn request_scoped_halt(
        &mut self,
        occurrence: PmFixtureCompletionOccurrence,
        scope: PmRiskHaltScope,
    ) -> Result<bool, PmProductRunError> {
        self.coordinator
            .request_scoped_halt(occurrence, scope)
            .map_err(PmProductRunError::service)
    }

    pub fn emit_telemetry(
        &mut self,
        occurrence: PmFixtureCompletionOccurrence,
        kind: PmTelemetryKind,
        value: u64,
    ) -> Result<(), PmProductRunError> {
        self.coordinator
            .emit_telemetry(occurrence, kind, value)
            .map_err(PmProductRunError::service)
    }

    pub fn service_turn(
        &mut self,
        monotonic_now_ns: u64,
    ) -> Result<PmCompleteServiceCounts, PmProductRunError> {
        self.coordinator
            .service_turn(monotonic_now_ns)
            .map_err(PmProductRunError::service)
    }

    /// Authenticates and enacts the exact public-age failure returned by this
    /// product's complete scheduler.
    ///
    /// The consumed failure carries the private lane and route seals. A
    /// sibling product cannot donate authority: rejection returns the same
    /// move-only run error so the caller can retry it against the owning run.
    pub async fn enact_public_lane_aged(
        &mut self,
        failure: PmProductRunError,
        local_wall_now_ns: u64,
        monotonic_now_ns: u64,
    ) -> Result<PmPublicAgedLaneFaultEnactment, PmProductPublicAgedEnactError> {
        let public_failure = match failure.failure {
            PmProductRunFailure::Service(PmCoordinatorError::CompleteScheduler(
                crate::lanes::PmCompleteServiceError::Public(failure @ PmServiceTurnError::Aged(_)),
            )) => failure,
            failure => {
                return Err(PmProductPublicAgedEnactError::NotPublicAged {
                    failure: PmProductRunError { failure },
                });
            }
        };
        self.coordinator
            .public_capture_mut()
            .enact_public_lane_aged(public_failure, local_wall_now_ns, monotonic_now_ns)
            .await
            .map_err(PmProductPublicAgedEnactError::from_capture)
    }

    pub fn pop_effect(&mut self) -> Option<PmProductEffect> {
        self.coordinator.pop_effect()
    }

    #[must_use]
    pub const fn pending_effect_outputs(&self) -> usize {
        self.coordinator.pending_effect_outputs()
    }

    #[must_use]
    pub const fn counters(&self) -> PmCoordinatorCounters {
        self.coordinator.counters()
    }

    #[must_use]
    pub const fn mutation_counters(&self) -> PmMutationCounters {
        self.coordinator.mutation_counters()
    }

    #[must_use]
    pub const fn mutation_halt(&self) -> Option<PmMutationHalt> {
        self.coordinator.mutation_halt()
    }

    #[must_use]
    pub fn persistence_metrics(&self) -> PmPersistenceMetrics {
        self.coordinator.persistence_metrics()
    }

    #[must_use]
    pub fn fake_effect_metrics(&self) -> PmFakeEffectMetrics {
        self.coordinator.fake_effect_metrics()
    }

    #[must_use]
    pub const fn product_effect_metrics(&self) -> PmProductEffectMetrics {
        self.coordinator.product_effect_metrics()
    }

    #[must_use]
    pub fn refresh_obligation_metrics(&self) -> PmRefreshObligationMetrics {
        self.coordinator.refresh_obligation_metrics()
    }

    #[cfg(test)]
    pub(crate) const fn telemetry_overload_state(
        &self,
    ) -> crate::coordinator::PmTelemetryOverloadState {
        self.coordinator.telemetry_overload_state()
    }

    #[cfg(test)]
    pub(crate) fn tracked_quote_slots_for_test(&self) -> usize {
        self.coordinator.tracked_quote_slots_for_test()
    }

    pub fn scheduler_metrics(
        &mut self,
        monotonic_now_ns: u64,
    ) -> Result<PmCompleteSchedulerMetrics, PmProductRunError> {
        self.coordinator
            .scheduler_metrics(monotonic_now_ns)
            .map_err(PmProductRunError::service)
    }

    #[must_use]
    pub const fn halt(&self) -> Option<PmControlReason> {
        self.coordinator.halt()
    }

    #[must_use]
    pub fn reserved_capacity_bytes(&self) -> usize {
        self.coordinator.boxed_reserved_capacity_bytes()
    }

    /// Shuts down both durable owners even if either one reports a failure.
    pub async fn shutdown(self) -> Result<PmPublicCaptureOutcome, PmProductRunError> {
        self.coordinator
            .shutdown()
            .await
            .map_err(PmProductRunError::shutdown)
    }
}

#[allow(
    clippy::large_enum_variant,
    reason = "exact cold-start failures remain inline so error construction preserves the allocation boundary"
)]
#[derive(Debug)]
enum PmProductStartFailure {
    Connectivity(PmConnectivityConfigError),
    Public(PmPublicCaptureRunError),
    Coordinator(PmCoordinatorStartError),
}

#[derive(Debug)]
pub struct PmProductStartError {
    failure: PmProductStartFailure,
}

impl PmProductStartError {
    fn connectivity(source: PmConnectivityConfigError) -> Self {
        Self {
            failure: PmProductStartFailure::Connectivity(source),
        }
    }

    fn public(source: PmPublicCaptureRunError) -> Self {
        Self {
            failure: PmProductStartFailure::Public(source),
        }
    }

    fn coordinator(source: PmCoordinatorStartError) -> Self {
        Self {
            failure: PmProductStartFailure::Coordinator(source),
        }
    }
}

impl fmt::Display for PmProductStartError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match &self.failure {
            PmProductStartFailure::Connectivity(source) => {
                write!(
                    formatter,
                    "PM product connectivity reconstruction failed: {source}"
                )
            }
            PmProductStartFailure::Public(source) => {
                write!(
                    formatter,
                    "PM product public capture start failed: {source}"
                )
            }
            PmProductStartFailure::Coordinator(source) => source.fmt(formatter),
        }
    }
}

impl Error for PmProductStartError {
    fn source(&self) -> Option<&(dyn Error + 'static)> {
        match &self.failure {
            PmProductStartFailure::Connectivity(source) => Some(source),
            PmProductStartFailure::Public(source) => Some(source),
            PmProductStartFailure::Coordinator(source) => Some(source),
        }
    }
}

#[allow(
    clippy::large_enum_variant,
    reason = "owner-loop and shutdown failures remain inline so fail-closed error paths do not allocate"
)]
#[derive(Debug)]
enum PmProductRunFailure {
    Service(PmCoordinatorError),
    Shutdown(PmCoordinatorShutdownError),
}

#[derive(Debug)]
pub struct PmProductRunError {
    failure: PmProductRunFailure,
}

impl PmProductRunError {
    fn service(source: PmCoordinatorError) -> Self {
        Self {
            failure: PmProductRunFailure::Service(source),
        }
    }

    fn shutdown(source: PmCoordinatorShutdownError) -> Self {
        Self {
            failure: PmProductRunFailure::Shutdown(source),
        }
    }

    #[must_use]
    pub const fn saturation_action(&self) -> Option<SaturationAction> {
        match &self.failure {
            PmProductRunFailure::Service(PmCoordinatorError::CompleteScheduler(error)) => {
                error.action()
            }
            PmProductRunFailure::Service(_) | PmProductRunFailure::Shutdown(_) => None,
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum PmProductPublicAgedRetryReason {
    TargetRunTerminal,
    PendingBookFault,
    PendingBookReduction,
    FailureNotOwnedOrStale,
}

/// Product-level public-age enactment failure.
///
/// `NotPublicAged` and `Retryable` retain the exact move-only scheduler error.
/// Call [`PmProductPublicAgedEnactError::into_run_failure`] to recover it.
#[derive(Debug)]
pub enum PmProductPublicAgedEnactError {
    NotPublicAged {
        failure: PmProductRunError,
    },
    Retryable {
        reason: PmProductPublicAgedRetryReason,
        failure: PmProductRunError,
    },
    Enact(PmPublicAgedLaneEnactError),
}

impl PmProductPublicAgedEnactError {
    fn from_capture(error: PmPublicAgedLaneEnactError) -> Self {
        let (reason, failure) = match error {
            PmPublicAgedLaneEnactError::RunTerminal { failure } => {
                (PmProductPublicAgedRetryReason::TargetRunTerminal, failure)
            }
            PmPublicAgedLaneEnactError::PendingBookFault { failure } => {
                (PmProductPublicAgedRetryReason::PendingBookFault, failure)
            }
            PmPublicAgedLaneEnactError::PendingBookReduction { failure } => (
                PmProductPublicAgedRetryReason::PendingBookReduction,
                failure,
            ),
            PmPublicAgedLaneEnactError::InvalidFailure { failure } => (
                PmProductPublicAgedRetryReason::FailureNotOwnedOrStale,
                failure,
            ),
            error => return Self::Enact(error),
        };
        Self::Retryable {
            reason,
            failure: PmProductRunError::service(PmCoordinatorError::CompleteScheduler(
                crate::lanes::PmCompleteServiceError::Public(failure),
            )),
        }
    }

    #[must_use]
    pub const fn retry_reason(&self) -> Option<PmProductPublicAgedRetryReason> {
        match self {
            Self::Retryable { reason, .. } => Some(*reason),
            Self::NotPublicAged { .. } | Self::Enact(_) => None,
        }
    }

    pub fn into_run_failure(self) -> Option<PmProductRunError> {
        match self {
            Self::NotPublicAged { failure } | Self::Retryable { failure, .. } => Some(failure),
            Self::Enact(_) => None,
        }
    }
}

impl fmt::Display for PmProductPublicAgedEnactError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::NotPublicAged { .. } => {
                formatter.write_str("product failure is not an evidenced public-lane age fault")
            }
            Self::Retryable { reason, .. } => {
                write!(
                    formatter,
                    "public-lane age fault was not consumed and may be retried: {reason:?}"
                )
            }
            Self::Enact(source) => source.fmt(formatter),
        }
    }
}

impl Error for PmProductPublicAgedEnactError {
    fn source(&self) -> Option<&(dyn Error + 'static)> {
        match self {
            Self::NotPublicAged { failure } | Self::Retryable { failure, .. } => Some(failure),
            Self::Enact(source) => Some(source),
        }
    }
}

impl fmt::Display for PmProductRunError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match &self.failure {
            PmProductRunFailure::Service(source) => {
                write!(formatter, "PM product service turn failed: {source}")
            }
            PmProductRunFailure::Shutdown(source) => source.fmt(formatter),
        }
    }
}

impl Error for PmProductRunError {
    fn source(&self) -> Option<&(dyn Error + 'static)> {
        match &self.failure {
            PmProductRunFailure::Service(source) => Some(source),
            PmProductRunFailure::Shutdown(source) => Some(source),
        }
    }
}
