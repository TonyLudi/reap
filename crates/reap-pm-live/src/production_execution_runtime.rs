//! Credential-owning composition for the concrete production supervisor.
//!
//! This boundary consumes private connectivity before it is split, retains
//! the resulting credential-authority supervisor beside the trading
//! supervisor, and proves both lifecycles are joined during controlled
//! shutdown.

use std::{fmt, path::PathBuf};

use reap_pm_core::PmTokenId;
use reap_polymarket_live_adapter::{
    PM_CREDENTIAL_AUTHORITY_DEFAULT_SHUTDOWN_BOUNDS, PmCredentialAuthorityShutdownBounds,
    PmCredentialAuthorityShutdownOutcome, PmCredentialAuthoritySupervisor,
    PmPrivateConnectivityOwner, PmProductionPostOnlyPlaceRequest, PmReadServerTimeHttpRole,
};
use reap_polymarket_public_source::PmDataApiCurrentPositionSource;
use thiserror::Error;

use crate::{
    PmProductionExecutionReadInfrastructure, PmProductionSupervisor, PmProductionSupervisorConfig,
    PmProductionSupervisorError, PmProductionSupervisorHandle, PmSupervisorCommandError,
    PmSupervisorFixedHeartbeatRole, PmSupervisorHeartbeatRole, PmSupervisorMutationRole,
    PmSupervisorPlaceCommand, PmSupervisorPollRole, PmSupervisorProductionMutationRole,
    PmSupervisorProductionReadError, PmSupervisorShutdownReport, PmSupervisorWsRole,
};

/// Failure while constructing the credential-owned production runtime.
#[derive(Debug, Error)]
pub enum PmProductionExecutionRuntimeStartError {
    #[error("private connectivity could not be split")]
    PrivateConnectivity(#[source] reap_polymarket_live_adapter::PmLiveAdapterError),
    #[error(
        "production read composition failed; credential-authority cleanup: {credential_authority:?}"
    )]
    ReadComposition {
        #[source]
        source: PmSupervisorProductionReadError,
        credential_authority: PmCredentialAuthorityShutdownOutcome,
    },
    #[error(
        "production supervisor startup failed; credential-authority cleanup: {credential_authority:?}"
    )]
    Supervisor {
        #[source]
        source: PmProductionSupervisorError,
        credential_authority: PmCredentialAuthorityShutdownOutcome,
    },
}

impl PmProductionExecutionRuntimeStartError {
    /// Credential teardown evidence for failures after private connectivity
    /// was successfully split.
    #[must_use]
    pub const fn credential_authority_shutdown(
        &self,
    ) -> Option<PmCredentialAuthorityShutdownOutcome> {
        match self {
            Self::PrivateConnectivity(_) => None,
            Self::ReadComposition {
                credential_authority,
                ..
            }
            | Self::Supervisor {
                credential_authority,
                ..
            } => Some(*credential_authority),
        }
    }
}

/// Successful proof that trading state and private credential custody both
/// completed controlled shutdown.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PmProductionExecutionShutdownReport {
    pub supervisor: PmSupervisorShutdownReport,
    pub credential_authority: PmCredentialAuthorityShutdownOutcome,
}

/// Controlled shutdown failed, with credential destruction evidence retained
/// even when the trading supervisor is the primary failure.
#[derive(Debug, Error)]
pub enum PmProductionExecutionShutdownError {
    #[error(
        "production supervisor shutdown failed; credential-authority cleanup: {credential_authority:?}"
    )]
    Supervisor {
        #[source]
        source: PmProductionSupervisorError,
        credential_authority: PmCredentialAuthorityShutdownOutcome,
    },
    #[error(
        "credential-authority shutdown was not clean after successful production supervisor shutdown: {credential_authority:?}"
    )]
    CredentialAuthority {
        supervisor: PmSupervisorShutdownReport,
        credential_authority: PmCredentialAuthorityShutdownOutcome,
    },
}

impl PmProductionExecutionShutdownError {
    #[must_use]
    pub const fn credential_authority_shutdown(&self) -> PmCredentialAuthorityShutdownOutcome {
        match self {
            Self::Supervisor {
                credential_authority,
                ..
            }
            | Self::CredentialAuthority {
                credential_authority,
                ..
            } => *credential_authority,
        }
    }
}

struct PmProductionExecutionRuntimeInner<Request> {
    // Field order is intentional: emergency Drop containment aborts trading
    // before aborting the private read credential authority.
    supervisor: PmProductionSupervisorHandle<Request>,
    credential_authority: PmCredentialAuthoritySupervisor,
    credential_shutdown_bounds: PmCredentialAuthorityShutdownBounds,
}

/// One move-only owner for the concrete production trading supervisor and the
/// private-read credential authority created from the same connectivity split.
pub struct PmProductionExecutionRuntime {
    inner: PmProductionExecutionRuntimeInner<PmProductionPostOnlyPlaceRequest>,
}

impl PmProductionExecutionRuntime {
    /// Consume unsplit private connectivity and start the complete concrete
    /// production supervisor. Any failure after the split first drops all
    /// credential clients and then joins the credential-authority task.
    #[allow(clippy::too_many_arguments)]
    pub async fn start(
        config: PmProductionSupervisorConfig,
        journal_path: PathBuf,
        private_connectivity: PmPrivateConnectivityOwner,
        server_time: PmReadServerTimeHttpRole,
        positions: impl IntoIterator<Item = (PmTokenId, PmDataApiCurrentPositionSource)>,
        heartbeat: PmSupervisorFixedHeartbeatRole,
        mutation: PmSupervisorProductionMutationRole,
    ) -> Result<Self, PmProductionExecutionRuntimeStartError> {
        Self::start_with_credential_shutdown_bounds(
            config,
            journal_path,
            private_connectivity,
            server_time,
            positions,
            heartbeat,
            mutation,
            PM_CREDENTIAL_AUTHORITY_DEFAULT_SHUTDOWN_BOUNDS,
        )
        .await
    }

    /// Variant with explicit, validated credential-authority join bounds.
    #[allow(clippy::too_many_arguments)]
    pub async fn start_with_credential_shutdown_bounds(
        config: PmProductionSupervisorConfig,
        journal_path: PathBuf,
        private_connectivity: PmPrivateConnectivityOwner,
        server_time: PmReadServerTimeHttpRole,
        positions: impl IntoIterator<Item = (PmTokenId, PmDataApiCurrentPositionSource)>,
        heartbeat: PmSupervisorFixedHeartbeatRole,
        mutation: PmSupervisorProductionMutationRole,
        credential_shutdown_bounds: PmCredentialAuthorityShutdownBounds,
    ) -> Result<Self, PmProductionExecutionRuntimeStartError> {
        let private_roles = private_connectivity
            .split()
            .map_err(PmProductionExecutionRuntimeStartError::PrivateConnectivity)?;
        let (authenticated_http, authenticated_user_ws, credential_authority) =
            private_roles.into_read_roles();
        let expected_maker = authenticated_http.configured_expected_maker();

        let reads = match PmProductionExecutionReadInfrastructure::new(
            config.scope(),
            expected_maker,
            server_time,
            authenticated_http,
            authenticated_user_ws,
            positions,
        ) {
            Ok(reads) => reads,
            Err(source) => {
                let credential_authority = credential_authority
                    .shutdown_bounded(credential_shutdown_bounds)
                    .await;
                return Err(PmProductionExecutionRuntimeStartError::ReadComposition {
                    source,
                    credential_authority,
                });
            }
        };
        let roles =
            match reads.into_production_supervisor_roles(config.scope(), heartbeat, mutation) {
                Ok(roles) => roles,
                Err(source) => {
                    let credential_authority = credential_authority
                        .shutdown_bounded(credential_shutdown_bounds)
                        .await;
                    return Err(PmProductionExecutionRuntimeStartError::ReadComposition {
                        source,
                        credential_authority,
                    });
                }
            };
        start_supervisor_with_credential_authority(
            config,
            journal_path,
            roles,
            credential_authority,
            credential_shutdown_bounds,
        )
        .await
        .map(|inner| Self { inner })
    }

    pub async fn place(
        &self,
        command: PmSupervisorPlaceCommand<PmProductionPostOnlyPlaceRequest>,
    ) -> Result<crate::PmSupervisorOrderProjection, PmSupervisorCommandError> {
        self.inner.supervisor.place(command).await
    }

    pub async fn cancel_exact(
        &self,
        expected_venue_order_id: impl Into<String>,
    ) -> Result<crate::PmSupervisorOrderProjection, PmSupervisorCommandError> {
        self.inner
            .supervisor
            .cancel_exact(expected_venue_order_id)
            .await
    }

    /// Stop and join the trading supervisor first, then stop and join private
    /// credential custody. Cancelling this future is fail-stop because it
    /// would otherwise discard the teardown proof.
    pub async fn shutdown(
        self,
    ) -> Result<PmProductionExecutionShutdownReport, PmProductionExecutionShutdownError> {
        self.inner.shutdown().await
    }
}

impl fmt::Debug for PmProductionExecutionRuntime {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str("PmProductionExecutionRuntime([REDACTED; RUNNING])")
    }
}

struct PmProductionExecutionShutdownFailStop {
    armed: bool,
}

impl PmProductionExecutionShutdownFailStop {
    const fn armed() -> Self {
        Self { armed: true }
    }

    fn disarm(&mut self) {
        self.armed = false;
    }
}

impl Drop for PmProductionExecutionShutdownFailStop {
    fn drop(&mut self) {
        if self.armed {
            // A cancelled composite shutdown could otherwise abort without
            // joining the task that owns private-read credentials.
            std::process::abort();
        }
    }
}

impl<Request: Send + 'static> PmProductionExecutionRuntimeInner<Request> {
    async fn shutdown(
        self,
    ) -> Result<PmProductionExecutionShutdownReport, PmProductionExecutionShutdownError> {
        let Self {
            supervisor,
            credential_authority,
            credential_shutdown_bounds,
        } = self;
        let mut cancellation_fail_stop = PmProductionExecutionShutdownFailStop::armed();

        // The actor must retain private reads through exact cancellation and
        // its terminal reconciliation cut. Joining it also destroys mutation
        // and heartbeat credential owners before private custody is stopped.
        let supervisor = supervisor.shutdown().await;
        let credential_authority = credential_authority
            .shutdown_bounded(credential_shutdown_bounds)
            .await;
        cancellation_fail_stop.disarm();

        match supervisor {
            Err(source) => Err(PmProductionExecutionShutdownError::Supervisor {
                source,
                credential_authority,
            }),
            Ok(supervisor) if credential_shutdown_is_clean(credential_authority) => {
                Ok(PmProductionExecutionShutdownReport {
                    supervisor,
                    credential_authority,
                })
            }
            Ok(supervisor) => Err(PmProductionExecutionShutdownError::CredentialAuthority {
                supervisor,
                credential_authority,
            }),
        }
    }
}

async fn start_supervisor_with_credential_authority<H, P, W, M>(
    config: PmProductionSupervisorConfig,
    journal_path: PathBuf,
    roles: crate::PmProductionSupervisorRoles<H, P, W, M>,
    credential_authority: PmCredentialAuthoritySupervisor,
    credential_shutdown_bounds: PmCredentialAuthorityShutdownBounds,
) -> Result<
    PmProductionExecutionRuntimeInner<M::PlaceRequest>,
    PmProductionExecutionRuntimeStartError,
>
where
    H: PmSupervisorHeartbeatRole,
    P: PmSupervisorPollRole,
    W: PmSupervisorWsRole,
    M: PmSupervisorMutationRole,
{
    match PmProductionSupervisor::start(config, journal_path, roles).await {
        Ok(supervisor) => Ok(PmProductionExecutionRuntimeInner {
            supervisor,
            credential_authority,
            credential_shutdown_bounds,
        }),
        Err(source) => {
            // `start` has already dropped every consumed role before it
            // returns an error, so no credential client can race this stop.
            let credential_authority = credential_authority
                .shutdown_bounded(credential_shutdown_bounds)
                .await;
            Err(PmProductionExecutionRuntimeStartError::Supervisor {
                source,
                credential_authority,
            })
        }
    }
}

const fn credential_shutdown_is_clean(outcome: PmCredentialAuthorityShutdownOutcome) -> bool {
    outcome.shutdown_requested()
        && !outcome.abort_requested()
        && outcome.task_joined()
        && outcome.task_completed_cleanly()
        && outcome.credentials_dropped()
}
