use std::error::Error;
use std::fmt;
use std::path::PathBuf;

use reap_pm_live_contracts::PmConnectivityConfig;
use reap_pm_strategy::PmQuoteModel;

use super::*;
use crate::composition::{PmPublicCaptureOutcome, PmPublicCaptureRun, PmPublicCaptureRunError};
use crate::fake_effect::PmFakeEffectRole;
use crate::journal::PmJournalRecovery;
use crate::private_monitor::PmPrivateMonitorRuntime;
use crate::schedule::PmQuoteScheduleRole;

impl<M: PmQuoteModel> PmCoordinator<M> {
    #[allow(
        clippy::too_many_arguments,
        reason = "cold start keeps each preconstructed sole owner and explicit policy visible"
    )]
    pub(crate) async fn start(
        config: &PmConnectivityConfig,
        model: M,
        private: PmPrivateMonitorRuntime,
        fake: PmFakeEffectRole,
        public: PmPublicCaptureRun,
        schedule: PmQuoteScheduleRole,
        journal_path: PathBuf,
        policy: PmCoordinatorPolicy,
    ) -> Result<(Self, PmJournalRecovery), PmCoordinatorStartError> {
        let decision = match PmDecisionState::new(config, model, policy) {
            Ok(decision) => decision,
            Err(source) => {
                let public_cleanup = public.finish().await.err();
                return Err(PmCoordinatorStartError::coordinator(source, public_cleanup));
            }
        };
        let (mutation, recovery) =
            match PmMutationOwner::start(config, private, fake, journal_path).await {
                Ok(started) => started,
                Err(source) => {
                    let public_cleanup = public.finish().await.err();
                    return Err(PmCoordinatorStartError::mutation(source, public_cleanup));
                }
            };
        let lanes = PmCompleteInputLanes::new(public, schedule);
        let account_route = config.account().account_route();
        let recovered_halt = matches!(
            mutation.halt(),
            Some(PmMutationHalt::RecoveredSafetyHalt(_))
        )
        .then_some(PmControlReason::RecoveredSafetyHalt);
        let mut counters = PmCoordinatorCounters::default();
        if recovered_halt.is_some() {
            counters.control_halts = 1;
        }
        Ok((
            Self {
                decision,
                account_source: account_route.source(),
                account_connection: account_route.connection(),
                account_scope: config.account().account_scope(),
                instrument: config.public().instrument(),
                mutation,
                lanes: Some(lanes),
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
            },
            recovery,
        ))
    }

    pub(crate) fn public_capture(&self) -> &PmPublicCaptureRun {
        self.lanes
            .as_ref()
            .and_then(PmCompleteInputLanes::public_capture)
            .expect("a started product coordinator owns its sole public capture run")
    }

    pub(crate) fn public_capture_mut(&mut self) -> &mut PmPublicCaptureRun {
        self.lanes
            .as_mut()
            .and_then(PmCompleteInputLanes::public_capture_mut)
            .expect("a started product coordinator owns its sole public capture run")
    }

    pub(crate) async fn shutdown(
        self,
    ) -> Result<PmPublicCaptureOutcome, PmCoordinatorShutdownError> {
        let Self {
            mutation, lanes, ..
        } = self;
        let mutation_result = mutation.shutdown().await;
        let Some(public) = lanes.and_then(PmCompleteInputLanes::into_public_capture) else {
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

#[derive(Debug)]
enum PmCoordinatorStartFailure {
    Coordinator(PmCoordinatorError),
    Mutation(PmMutationError),
}

#[derive(Debug)]
pub(crate) struct PmCoordinatorStartError {
    failure: PmCoordinatorStartFailure,
    public_cleanup: Option<PmPublicCaptureRunError>,
}

impl PmCoordinatorStartError {
    fn coordinator(
        source: PmCoordinatorError,
        public_cleanup: Option<PmPublicCaptureRunError>,
    ) -> Self {
        Self {
            failure: PmCoordinatorStartFailure::Coordinator(source),
            public_cleanup,
        }
    }

    fn mutation(source: PmMutationError, public_cleanup: Option<PmPublicCaptureRunError>) -> Self {
        Self {
            failure: PmCoordinatorStartFailure::Mutation(source),
            public_cleanup,
        }
    }
}

impl fmt::Display for PmCoordinatorStartError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match &self.failure {
            PmCoordinatorStartFailure::Coordinator(source) => {
                write!(formatter, "PM coordinator preflight failed: {source}")?;
                if let Some(cleanup) = &self.public_cleanup {
                    write!(formatter, "; public capture cleanup also failed: {cleanup}")?;
                }
                Ok(())
            }
            PmCoordinatorStartFailure::Mutation(source) => {
                write!(formatter, "PM mutation owner start failed: {source}")?;
                if let Some(cleanup) = &self.public_cleanup {
                    write!(formatter, "; public capture cleanup also failed: {cleanup}")?;
                }
                Ok(())
            }
        }
    }
}

impl Error for PmCoordinatorStartError {
    fn source(&self) -> Option<&(dyn Error + 'static)> {
        match &self.failure {
            PmCoordinatorStartFailure::Coordinator(source) => Some(source),
            PmCoordinatorStartFailure::Mutation(source) => Some(source),
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
