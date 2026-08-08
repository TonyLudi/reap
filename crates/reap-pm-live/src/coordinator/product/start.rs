use std::error::Error;
use std::fmt;
use std::path::PathBuf;

use reap_pm_live_contracts::PmConnectivityConfig;
use reap_pm_strategy::PmQuoteModel;

use super::*;
use crate::composition::{PmPublicCaptureOutcome, PmPublicCaptureRun, PmPublicCaptureRunError};
use crate::fake_effect::{PmFakeEffectRole, PmFixtureEffectExecutor};
use crate::journal::{PmJournalRecovery, PmJournalScopeV1};
use crate::private_monitor::PmPrivateMonitorRuntime;
use crate::schedule::PmQuoteScheduleRole;

impl<M: PmQuoteModel> PmCoordinator<M> {
    /// Performs every fallible coordinator-only check before a mutation or
    /// transport owner is moved into product composition. The resulting
    /// capability is backend-neutral and can be consumed exactly once by
    /// either the fake start path or authenticated static composition.
    pub(crate) fn prepare_assembly(
        config: &PmConnectivityConfig,
        model: M,
        policy: PmCoordinatorPolicy,
    ) -> Result<PmCoordinatorAssembly<M>, PmCoordinatorError> {
        let decision = PmDecisionState::new(config, model, policy)?;
        let account_route = config.account().account_route();
        Ok(PmCoordinatorAssembly {
            decision,
            account_source: account_route.source(),
            account_connection: account_route.connection(),
            account_scope: config.account().account_scope(),
            instrument: config.public().instrument(),
            instrument_id: config.account().instrument_id(),
            configuration_fingerprint: config.public().configuration_fingerprint(),
        })
    }

    /// Assembles the sole coordinator from an already-started backend-neutral
    /// mutation owner after validating the owner's exact journal scope. Fake
    /// and authenticated composition remain separate because neither an
    /// executor nor a backend selector is accepted here. A mismatch returns
    /// every move-only owner for explicit shutdown.
    pub(crate) fn assemble_with_mutation(
        assembly: PmCoordinatorAssembly<M>,
        mutation: PmMutationOwner,
        public: Box<PmPublicCaptureRun>,
        schedule: PmQuoteScheduleRole,
    ) -> Result<Box<Self>, PmCoordinatorAssemblyFailure<M>> {
        if let Err(reason) = assembly.validate_mutation_scope(mutation.journal_scope()) {
            return Err(PmCoordinatorAssemblyFailure {
                reason,
                assembly,
                mutation,
                public,
                schedule,
            });
        }
        let PmCoordinatorAssembly {
            decision,
            account_source,
            account_connection,
            account_scope,
            instrument,
            instrument_id: _,
            configuration_fingerprint: _,
        } = assembly;
        let recovered_halt = matches!(
            mutation.halt(),
            Some(PmMutationHalt::RecoveredSafetyHalt(_))
        )
        .then_some(PmControlReason::RecoveredSafetyHalt);
        let recovery_reconciliation_required = matches!(
            mutation.halt(),
            Some(PmMutationHalt::RecoveryReconciliationRequired)
        );
        let mut counters = PmCoordinatorCounters::default();
        if recovered_halt.is_some() {
            counters.control_halts = 1;
        }
        Ok(Box::new(Self {
            decision,
            account_source,
            account_connection,
            account_scope,
            instrument,
            mutation: Box::new(mutation),
            lanes: Some(Box::new(PmCompleteInputLanes::new(public, schedule))),
            outputs: PmProductEffectOutput::new(),
            private_readiness_revision: 1,
            last_action_sequence: 0,
            pending_correlations: CorrelationRing::new(),
            prepared_correlations: CorrelationRing::new(),
            tracked_quotes: [None; 2],
            halt: recovered_halt,
            counters,
            callback_error: None,
            retained_critical: None,
            retained_persistence: None,
            retained_private_admission: None,
            retained_reconciliation_admission: None,
            pending_schedules: PendingSchedules::new(),
            refresh_obligations: refresh_obligations::PmRefreshObligations::new(),
            reconciliation_gate: recovery_reconciliation_required,
            reconciliation_recovered: false,
        }))
    }

    #[allow(
        clippy::too_many_arguments,
        reason = "cold start keeps each preconstructed sole owner and explicit policy visible"
    )]
    pub(crate) async fn start(
        config: &PmConnectivityConfig,
        model: M,
        private: Box<PmPrivateMonitorRuntime>,
        fake: PmFakeEffectRole,
        public: Box<PmPublicCaptureRun>,
        schedule: PmQuoteScheduleRole,
        journal_path: PathBuf,
        policy: PmCoordinatorPolicy,
    ) -> Result<(Box<Self>, PmJournalRecovery, PmFixtureEffectExecutor), PmCoordinatorStartError>
    {
        let assembly = match Self::prepare_assembly(config, model, policy) {
            Ok(assembly) => assembly,
            Err(source) => {
                let public_cleanup = public.finish().await.err();
                return Err(PmCoordinatorStartError::coordinator(source, public_cleanup));
            }
        };
        let (mutation, recovery, executor) =
            match PmMutationOwner::start(config, private, fake, journal_path).await {
                Ok(started) => started,
                Err(source) => {
                    let public_cleanup = public.finish().await.err();
                    return Err(PmCoordinatorStartError::mutation(source, public_cleanup));
                }
            };
        let coordinator = match Self::assemble_with_mutation(assembly, mutation, public, schedule) {
            Ok(coordinator) => coordinator,
            Err(failure) => {
                let (reason, _assembly, mutation, public, schedule) = failure.into_parts();
                drop(schedule);
                let mutation_cleanup = mutation.shutdown().await.err();
                let public_cleanup = public.finish().await.err();
                return Err(PmCoordinatorStartError::assembly(
                    reason,
                    mutation_cleanup,
                    public_cleanup,
                ));
            }
        };
        Ok((coordinator, recovery, executor))
    }

    pub(crate) fn public_capture(&self) -> &PmPublicCaptureRun {
        self.lanes
            .as_deref()
            .and_then(PmCompleteInputLanes::public_capture)
            .expect("a started product coordinator owns its sole public capture run")
    }

    pub(crate) fn public_capture_mut(&mut self) -> &mut PmPublicCaptureRun {
        self.lanes
            .as_deref_mut()
            .and_then(PmCompleteInputLanes::public_capture_mut)
            .expect("a started product coordinator owns its sole public capture run")
    }

    pub(crate) async fn shutdown(
        self: Box<Self>,
    ) -> Result<PmPublicCaptureOutcome, PmCoordinatorShutdownError> {
        let Self {
            mutation, lanes, ..
        } = *self;
        let mutation_result = (*mutation).shutdown().await;
        let Some(public) = lanes.and_then(|lanes| (*lanes).into_public_capture()) else {
            return match mutation_result {
                Ok(()) => Err(PmCoordinatorShutdownError::MissingPublicOwner),
                Err(source) => Err(PmCoordinatorShutdownError::Mutation(source)),
            };
        };
        let public_result = public.finish().await;
        match (mutation_result, public_result) {
            (Ok(()), Ok(outcome)) => Ok(outcome),
            (Err(mutation), Ok(_)) => Err(PmCoordinatorShutdownError::Mutation(mutation)),
            (Ok(()), Err(public)) => Err(PmCoordinatorShutdownError::Public(public)),
            (Err(mutation), Err(public)) => {
                Err(PmCoordinatorShutdownError::Both { mutation, public })
            }
        }
    }
}

/// Consume-once coordinator preflight. It contains decision ownership and
/// exact copied scope only; no mutation, journal, transport, or executor.
pub(crate) struct PmCoordinatorAssembly<M: PmQuoteModel> {
    decision: PmDecisionState<M>,
    account_source: reap_pm_core::PmProductSource,
    account_connection: reap_pm_core::PmConnectionId,
    account_scope: reap_pm_core::PmAccountScope,
    instrument: reap_pm_core::PmInstrumentHandle,
    instrument_id: reap_pm_core::PmInstrumentId,
    configuration_fingerprint: reap_pm_core::PmConfigurationFingerprint,
}

impl<M: PmQuoteModel> PmCoordinatorAssembly<M> {
    pub(super) fn validate_mutation_scope(
        &self,
        mutation_scope: &PmJournalScopeV1,
    ) -> Result<(), PmCoordinatorAssemblyError> {
        if mutation_scope.account_scope() != self.account_scope
            || mutation_scope.instrument() != self.instrument_id
            || mutation_scope.configuration_fingerprint().bytes()
                != self.configuration_fingerprint.bytes()
        {
            return Err(PmCoordinatorAssemblyError::MutationScopeMismatch);
        }
        Ok(())
    }
}

/// Retaining assembly failure. No already-started owner is dropped merely
/// because a crate-internal caller paired two same-shaped but different
/// configurations.
pub(crate) struct PmCoordinatorAssemblyFailure<M: PmQuoteModel> {
    reason: PmCoordinatorAssemblyError,
    assembly: PmCoordinatorAssembly<M>,
    mutation: PmMutationOwner,
    public: Box<PmPublicCaptureRun>,
    schedule: PmQuoteScheduleRole,
}

impl<M: PmQuoteModel> PmCoordinatorAssemblyFailure<M> {
    pub(crate) fn into_parts(
        self,
    ) -> (
        PmCoordinatorAssemblyError,
        PmCoordinatorAssembly<M>,
        PmMutationOwner,
        Box<PmPublicCaptureRun>,
        PmQuoteScheduleRole,
    ) {
        (
            self.reason,
            self.assembly,
            self.mutation,
            self.public,
            self.schedule,
        )
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, thiserror::Error)]
pub(crate) enum PmCoordinatorAssemblyError {
    #[error("coordinator assembly and mutation journal scopes differ")]
    MutationScopeMismatch,
}

#[derive(Debug)]
enum PmCoordinatorStartFailure {
    Coordinator(PmCoordinatorError),
    Mutation(PmMutationError),
    Assembly(PmCoordinatorAssemblyError),
}

#[derive(Debug)]
pub(crate) struct PmCoordinatorStartError {
    failure: PmCoordinatorStartFailure,
    mutation_cleanup: Option<PmMutationError>,
    public_cleanup: Option<PmPublicCaptureRunError>,
}

impl PmCoordinatorStartError {
    fn coordinator(
        source: PmCoordinatorError,
        public_cleanup: Option<PmPublicCaptureRunError>,
    ) -> Self {
        Self {
            failure: PmCoordinatorStartFailure::Coordinator(source),
            mutation_cleanup: None,
            public_cleanup,
        }
    }

    fn mutation(source: PmMutationError, public_cleanup: Option<PmPublicCaptureRunError>) -> Self {
        Self {
            failure: PmCoordinatorStartFailure::Mutation(source),
            mutation_cleanup: None,
            public_cleanup,
        }
    }

    fn assembly(
        source: PmCoordinatorAssemblyError,
        mutation_cleanup: Option<PmMutationError>,
        public_cleanup: Option<PmPublicCaptureRunError>,
    ) -> Self {
        Self {
            failure: PmCoordinatorStartFailure::Assembly(source),
            mutation_cleanup,
            public_cleanup,
        }
    }

    fn write_cleanup(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        if let Some(cleanup) = &self.mutation_cleanup {
            write!(formatter, "; mutation cleanup also failed: {cleanup}")?;
        }
        if let Some(cleanup) = &self.public_cleanup {
            write!(formatter, "; public capture cleanup also failed: {cleanup}")?;
        }
        Ok(())
    }
}

impl fmt::Display for PmCoordinatorStartError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match &self.failure {
            PmCoordinatorStartFailure::Coordinator(source) => {
                write!(formatter, "PM coordinator preflight failed: {source}")?;
                self.write_cleanup(formatter)
            }
            PmCoordinatorStartFailure::Mutation(source) => {
                write!(formatter, "PM mutation owner start failed: {source}")?;
                self.write_cleanup(formatter)
            }
            PmCoordinatorStartFailure::Assembly(source) => {
                write!(formatter, "PM coordinator assembly failed: {source}")?;
                self.write_cleanup(formatter)
            }
        }
    }
}

impl Error for PmCoordinatorStartError {
    fn source(&self) -> Option<&(dyn Error + 'static)> {
        match &self.failure {
            PmCoordinatorStartFailure::Coordinator(source) => Some(source),
            PmCoordinatorStartFailure::Mutation(source) => Some(source),
            PmCoordinatorStartFailure::Assembly(source) => Some(source),
        }
    }
}

#[allow(
    clippy::large_enum_variant,
    reason = "shutdown retains both exact owner failures inline and must not allocate while failing closed"
)]
#[derive(Debug, thiserror::Error)]
pub(crate) enum PmCoordinatorShutdownError {
    #[error("started PM coordinator lost its public owner")]
    MissingPublicOwner,
    #[error("PM mutation owner shutdown failed: {0}")]
    Mutation(PmMutationError),
    #[error("PM public capture shutdown failed: {0}")]
    Public(PmPublicCaptureRunError),
    #[error(
        "PM mutation and public capture shutdown both failed: mutation={mutation}; public={public}"
    )]
    Both {
        mutation: PmMutationError,
        public: PmPublicCaptureRunError,
    },
}
