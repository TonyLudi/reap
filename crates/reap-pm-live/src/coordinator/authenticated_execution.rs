//! Feature-gated authenticated loopback mutation workers.
//!
//! The coordinator hands each worker an already-Goal-F-durable dispatch by
//! value before any await. Workers own only purpose-specific authentication,
//! transport, and authenticated-journal roles; they never borrow canonical
//! product state across network or durability waits.

mod journal_role;
mod outcome;
mod task;
mod worker;

use std::{path::PathBuf, time::Duration};

use reap_pm_live_contracts::PmConnectivityConfig;
use reap_polymarket_live_adapter::{
    PmAuthenticatedHttpOwner, PmAuthenticatedUserWsRole, PmCredentialAuthoritySupervisor,
    PmExactOwnedCancelLoopbackRole, PmFixedPlaceLoopbackRole, PmLoopbackCancelAuthenticationRole,
    PmLoopbackMutationConnectivityBinding, PmLoopbackMutationConnectivityRoles,
    PmLoopbackPlaceAuthenticationRole,
};
use thiserror::Error;

use self::journal_role::PmAuthenticatedJournalRuntime;
pub(crate) use self::task::{
    PmAuthenticatedCancelTaskFinish, PmAuthenticatedCancelTaskOutcome,
    PmAuthenticatedPlaceTaskFinish, PmAuthenticatedPlaceTaskOutcome,
    PmAuthenticatedTaskPreparationError,
};
pub(crate) use self::worker::{PmAuthenticatedCancelWorker, PmAuthenticatedPlaceWorker};
use crate::authenticated_journal::{
    PmAuthenticatedJournalError, PmAuthenticatedJournalRecovery, PmAuthenticatedJournalSchemaError,
    PmAuthenticatedJournalScopeV1,
};
use crate::coordinator::live_completion::PmAuthenticatedBridgeApplied;
use crate::coordinator::live_completion::PmLiveCompletionError;
use crate::coordinator::mutation::{PmMutationError, PmMutationOwner};
use crate::fake_effect::PmMutationPreparationRole;
use crate::journal::PmJournalRecovery;
use crate::private_monitor::PmPrivateMonitorRuntime;

/// Unactivated authenticated composition. It owns the exact journal lease and
/// recovery cut, but deliberately exposes no mutation worker. Consuming this
/// stage through the paired Goal-F gate is the only way to obtain workers.
pub(crate) struct PmLoopbackAuthenticatedExecutionStage {
    journal_scope: PmAuthenticatedJournalScopeV1,
    journal_recovery: PmAuthenticatedJournalRecovery,
    journal: PmAuthenticatedJournalRuntime,
    authenticated_http: PmAuthenticatedHttpOwner,
    authenticated_user_ws: PmAuthenticatedUserWsRole,
    place_authentication: PmLoopbackPlaceAuthenticationRole,
    cancel_authentication: PmLoopbackCancelAuthenticationRole,
    place_transport: PmFixedPlaceLoopbackRole,
    cancel_transport: PmExactOwnedCancelLoopbackRole,
    credential_supervisor: PmCredentialAuthoritySupervisor,
}

impl PmLoopbackAuthenticatedExecutionStage {
    #[allow(clippy::too_many_arguments)]
    pub(crate) async fn start(
        config: &PmConnectivityConfig,
        journal_path: PathBuf,
        durability_timeout: Duration,
        connectivity: PmLoopbackMutationConnectivityRoles,
        place_transport: PmFixedPlaceLoopbackRole,
        cancel_transport: PmExactOwnedCancelLoopbackRole,
    ) -> Result<Self, PmAuthenticatedExecutionError> {
        let (
            authenticated_http,
            authenticated_user_ws,
            place_authentication,
            cancel_authentication,
            connectivity_binding,
            credential_fingerprint,
            credential_supervisor,
        ) = connectivity.into_roles();
        if let Err(error) = validate_connectivity_binding(config, &connectivity_binding) {
            drop(authenticated_http);
            drop(authenticated_user_ws);
            drop(place_authentication);
            drop(cancel_authentication);
            drop(place_transport);
            drop(cancel_transport);
            return Err(
                preserve_primary_after_credential_shutdown(error, credential_supervisor).await,
            );
        }
        let journal_scope = match PmAuthenticatedJournalScopeV1::from_config(
            config,
            credential_fingerprint.into_authenticated_journal_scope_bytes(),
        ) {
            Ok(scope) => scope,
            Err(error) => {
                drop(authenticated_http);
                drop(authenticated_user_ws);
                drop(place_authentication);
                drop(cancel_authentication);
                drop(place_transport);
                drop(cancel_transport);
                return Err(preserve_primary_after_credential_shutdown(
                    error.into(),
                    credential_supervisor,
                )
                .await);
            }
        };
        let (journal, journal_recovery) = match PmAuthenticatedJournalRuntime::start(
            journal_path,
            journal_scope.clone(),
            durability_timeout,
        )
        .await
        {
            Ok(started) => started,
            Err(primary) => {
                drop(authenticated_http);
                drop(authenticated_user_ws);
                drop(place_authentication);
                drop(cancel_authentication);
                drop(place_transport);
                drop(cancel_transport);
                return Err(preserve_primary_after_credential_shutdown(
                    primary,
                    credential_supervisor,
                )
                .await);
            }
        };
        Ok(Self {
            journal_scope,
            journal_recovery,
            journal,
            authenticated_http,
            authenticated_user_ws,
            place_authentication,
            cancel_authentication,
            place_transport,
            cancel_transport,
            credential_supervisor,
        })
    }

    /// Holds the authenticated lease continuously while Goal-F is recovered,
    /// repaired append-once, and revalidated against this exact recovery cut.
    /// Workers do not exist until the whole gate succeeds.
    #[allow(clippy::too_many_arguments)]
    pub(crate) async fn activate_after_goal_f_validation(
        self,
        config: &PmConnectivityConfig,
        private: Box<PmPrivateMonitorRuntime>,
        preparation: PmMutationPreparationRole,
        goal_f_journal_path: PathBuf,
        goal_f_durability_timeout: Duration,
    ) -> Result<
        (
            PmLoopbackAuthenticatedExecution,
            PmMutationOwner,
            PmJournalRecovery,
            PmAuthenticatedJournalRecovery,
        ),
        PmAuthenticatedExecutionError,
    > {
        let Self {
            journal_scope,
            journal_recovery,
            journal,
            authenticated_http,
            authenticated_user_ws,
            place_authentication,
            cancel_authentication,
            place_transport,
            cancel_transport,
            credential_supervisor,
        } = self;
        let (mutation, goal_f_recovery) = match PmMutationOwner::start_authenticated(
            config,
            private,
            preparation,
            goal_f_journal_path,
            &journal_recovery,
            goal_f_durability_timeout,
        )
        .await
        {
            Ok(started) => started,
            Err(primary) => {
                drop(place_authentication);
                drop(cancel_authentication);
                drop(authenticated_http);
                drop(authenticated_user_ws);
                drop(place_transport);
                drop(cancel_transport);
                let credential_cleanup = credential_supervisor.shutdown().await.err();
                let journal_cleanup = journal.shutdown().await.err();
                let primary = PmAuthenticatedExecutionError::Mutation(primary);
                return match (credential_cleanup, journal_cleanup) {
                    (None, None) => Err(primary),
                    (credential, journal) => {
                        Err(PmAuthenticatedExecutionError::ActivationCleanup {
                            primary: Box::new(primary),
                            credential: credential.map(Box::new),
                            journal: journal.map(Box::new),
                        })
                    }
                };
            }
        };
        let place = PmAuthenticatedPlaceWorker::new(
            journal_scope.clone(),
            journal.role(),
            place_authentication,
            place_transport,
        );
        let cancel = PmAuthenticatedCancelWorker::new(
            journal_scope,
            journal.role(),
            cancel_authentication,
            cancel_transport,
        );
        Ok((
            PmLoopbackAuthenticatedExecution {
                authenticated_http,
                authenticated_user_ws,
                mutation: PmLoopbackAuthenticatedMutationWorkers {
                    place: Some(place),
                    cancel: Some(cancel),
                },
                shutdown: PmLoopbackAuthenticatedExecutionShutdown {
                    journal,
                    credential_supervisor,
                },
            },
            mutation,
            goal_f_recovery,
            journal_recovery,
        ))
    }
}

async fn preserve_primary_after_credential_shutdown(
    primary: PmAuthenticatedExecutionError,
    credential_supervisor: PmCredentialAuthoritySupervisor,
) -> PmAuthenticatedExecutionError {
    match credential_supervisor.shutdown().await {
        Ok(()) => primary,
        Err(cleanup) => PmAuthenticatedExecutionError::CredentialCleanup {
            primary: Box::new(primary),
            cleanup: Box::new(cleanup),
        },
    }
}

#[allow(
    clippy::result_large_err,
    reason = "startup validation preserves the bounded typed execution failure inline for exact cleanup reporting"
)]
fn validate_connectivity_binding(
    config: &PmConnectivityConfig,
    binding: &PmLoopbackMutationConnectivityBinding,
) -> Result<(), PmAuthenticatedExecutionError> {
    validate_configuration_fingerprint(config, binding.configuration_fingerprint())?;
    let expected = config.public().expected_metadata();
    let wire_scope = binding.wire_scope();
    if binding.account() != config.account().account_scope()
        || binding.instrument() != config.account().instrument()
        || binding.trading_domain() != config.account().trading_domain()
        || wire_scope.condition() != expected.condition()
        || wire_scope.market() != expected.market()
        || wire_scope.token() != expected.outcome().token()
    {
        return Err(PmAuthenticatedExecutionError::ConnectivityBindingMismatch);
    }
    Ok(())
}

#[allow(
    clippy::result_large_err,
    reason = "startup validation preserves the bounded typed execution failure inline for exact cleanup reporting"
)]
fn validate_configuration_fingerprint(
    config: &PmConnectivityConfig,
    connectivity_fingerprint: reap_pm_core::PmConfigurationFingerprint,
) -> Result<(), PmAuthenticatedExecutionError> {
    if connectivity_fingerprint != config.public().configuration_fingerprint() {
        return Err(PmAuthenticatedExecutionError::ConfigurationFingerprintMismatch);
    }
    Ok(())
}

/// Static loopback execution composition. Construction is absent unless the
/// crate is under test or explicitly built for loopback evidence.
pub(crate) struct PmLoopbackAuthenticatedExecution {
    authenticated_http: PmAuthenticatedHttpOwner,
    authenticated_user_ws: PmAuthenticatedUserWsRole,
    mutation: PmLoopbackAuthenticatedMutationWorkers,
    shutdown: PmLoopbackAuthenticatedExecutionShutdown,
}

impl PmLoopbackAuthenticatedExecution {
    /// Consumes the ready composition exactly once into owned read roles,
    /// independent mutation workers, and the ordered shutdown owner. This is
    /// compatible with the consuming user-WebSocket run API and creates no
    /// detached task or runtime backend selector.
    pub(crate) fn into_roles(
        self,
    ) -> (
        PmAuthenticatedHttpOwner,
        PmAuthenticatedUserWsRole,
        PmLoopbackAuthenticatedMutationWorkers,
        PmLoopbackAuthenticatedExecutionShutdown,
    ) {
        (
            self.authenticated_http,
            self.authenticated_user_ws,
            self.mutation,
            self.shutdown,
        )
    }

    pub(crate) async fn shutdown(self) -> Result<(), PmAuthenticatedExecutionError> {
        let (authenticated_http, authenticated_user_ws, mutation, shutdown) = self.into_roles();
        drop(authenticated_http);
        drop(authenticated_user_ws);
        drop(mutation);
        shutdown.shutdown().await
    }
}

/// Two independent purpose workers. A slow placement cannot hold the cancel
/// role, and the only release path consumes the product owner's exact Goal-F
/// bridge-applied proof.
pub(crate) struct PmLoopbackAuthenticatedMutationWorkers {
    place: Option<PmAuthenticatedPlaceWorker>,
    cancel: Option<PmAuthenticatedCancelWorker>,
}

impl PmLoopbackAuthenticatedMutationWorkers {
    #[allow(
        clippy::result_large_err,
        reason = "bridge confirmation returns the bounded typed worker failure inline without changing the move-only handshake"
    )]
    pub(crate) fn confirm_goal_f_bridge(
        &mut self,
        applied: PmAuthenticatedBridgeApplied,
    ) -> Result<(), PmAuthenticatedExecutionError> {
        if applied.is_place() {
            self.place
                .as_mut()
                .ok_or(PmAuthenticatedExecutionError::WorkerUnavailable)?
                .confirm_goal_f_bridge(applied)
        } else {
            self.cancel
                .as_mut()
                .ok_or(PmAuthenticatedExecutionError::WorkerUnavailable)?
                .confirm_goal_f_bridge(applied)
        }
    }
}

/// Ordered lifecycle owner. Callers invoke this only after joining/stopping
/// HTTP, user-WS, and mutation tasks; live worker journal handles make an
/// early shutdown fail rather than detaching a task or releasing the lease.
pub(crate) struct PmLoopbackAuthenticatedExecutionShutdown {
    journal: PmAuthenticatedJournalRuntime,
    credential_supervisor: PmCredentialAuthoritySupervisor,
}

impl PmLoopbackAuthenticatedExecutionShutdown {
    pub(crate) async fn shutdown(self) -> Result<(), PmAuthenticatedExecutionError> {
        let credential_result = self.credential_supervisor.shutdown().await;
        let journal_result = self.journal.shutdown().await;
        combine_shutdown_results(credential_result, journal_result)
    }
}

#[allow(
    clippy::result_large_err,
    reason = "shutdown aggregation preserves the bounded primary and cleanup failures inline instead of masking either one"
)]
fn combine_shutdown_results(
    credential_result: Result<(), reap_polymarket_live_adapter::PmLiveAdapterError>,
    journal_result: Result<(), PmAuthenticatedExecutionError>,
) -> Result<(), PmAuthenticatedExecutionError> {
    match (credential_result, journal_result) {
        (Ok(()), Ok(())) => Ok(()),
        (Err(credential), Ok(())) => Err(PmAuthenticatedExecutionError::LiveAdapter(credential)),
        (Ok(()), Err(journal)) => Err(journal),
        (Err(credential), Err(journal)) => Err(PmAuthenticatedExecutionError::ShutdownBoth {
            credential: Box::new(credential),
            journal: Box::new(journal),
        }),
    }
}

#[allow(
    clippy::large_enum_variant,
    reason = "the bounded execution error carries exact mutation and cleanup failures inline across the supervised owner boundary"
)]
#[derive(Debug, Error)]
pub(crate) enum PmAuthenticatedExecutionError {
    #[error(transparent)]
    Journal(#[from] PmAuthenticatedJournalError),
    #[error(transparent)]
    Schema(#[from] PmAuthenticatedJournalSchemaError),
    #[error("authenticated journal returned an acknowledgement for the wrong operation kind")]
    JournalOperationMismatch,
    #[error("authenticated journal runtime still has a live purpose role during shutdown")]
    JournalRoleStillLive,
    #[error("authenticated journal durability timeout is zero or above the fixed cap")]
    InvalidDurabilityTimeout,
    #[error("loopback connectivity roles belong to a different exact public configuration")]
    ConfigurationFingerprintMismatch,
    #[error("loopback connectivity roles contradict the exact account/instrument/domain scope")]
    ConnectivityBindingMismatch,
    #[error("{primary}; credential-authority cleanup also failed: {cleanup}")]
    CredentialCleanup {
        #[source]
        primary: Box<PmAuthenticatedExecutionError>,
        cleanup: Box<reap_polymarket_live_adapter::PmLiveAdapterError>,
    },
    #[error(
        "{primary}; authenticated activation cleanup also failed: credential={credential:?}; journal={journal:?}"
    )]
    ActivationCleanup {
        #[source]
        primary: Box<PmAuthenticatedExecutionError>,
        credential: Option<Box<reap_polymarket_live_adapter::PmLiveAdapterError>>,
        journal: Option<Box<PmAuthenticatedExecutionError>>,
    },
    #[error(
        "credential-authority and authenticated-journal shutdown both failed: credential={credential}; journal={journal}"
    )]
    ShutdownBoth {
        credential: Box<reap_polymarket_live_adapter::PmLiveAdapterError>,
        journal: Box<PmAuthenticatedExecutionError>,
    },
    #[error("authenticated journal timed out at the {0:?} durability barrier")]
    JournalAcknowledgementTimeout(PmAuthenticatedDurabilityStage),
    #[error("authenticated journal shutdown exceeded the fixed durability timeout")]
    JournalShutdownTimeout,
    #[error("scope-bound mutation authentication failed before authenticated preparation: {0}")]
    Authentication(reap_polymarket_live_adapter::PmLoopbackMutationAuthError),
    #[error("authenticated worker already retains its sole bounded pre-send failure")]
    PreSendQuarantineOccupied,
    #[error("authenticated worker is halted, quarantined, or already reserved")]
    WorkerUnavailable,
    #[error("durable authenticated send grant does not match the retained request bytes")]
    RetainedRequestMismatch,
    #[error("loopback mutation outcome contradicts its durable request identity")]
    OutcomeIdentityMismatch,
    #[error("loopback mutation outcome has an invalid fixed-profile shape")]
    OutcomeShapeMismatch,
    #[error("authenticated cancel dispatch lost its move-only local ownership proof")]
    CancelOwnershipMismatch,
    #[error("Goal-F bridge acknowledgement does not match the worker's exact pending result")]
    GoalFBridgeAcknowledgementMismatch,
    #[error(transparent)]
    Completion(#[from] PmLiveCompletionError),
    #[error(transparent)]
    Mutation(#[from] PmMutationError),
    #[error(transparent)]
    LiveAdapter(#[from] reap_polymarket_live_adapter::PmLiveAdapterError),
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum PmAuthenticatedDurabilityStage {
    Prepared,
    DispatchAuthorized,
    Result,
}

#[cfg(test)]
#[path = "authenticated_execution/tests.rs"]
mod tests;
