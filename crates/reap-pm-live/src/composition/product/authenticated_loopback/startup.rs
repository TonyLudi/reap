use std::{fmt, path::PathBuf, time::Duration};

use reap_pm_core::{PmMarketMetadata, PmProductSource, SnapshotRevision};
use reap_pm_live_contracts::{PmConnectivityConfig, PmConnectivityConfigError};
use reap_pm_strategy::PmQuoteModel;
use reap_polymarket_adapter::{
    PmAuthoritativeMetadata, PmMetadataJoinError, PmMetadataRevisionInput,
};
use reap_polymarket_live_adapter::{
    PmActorProductClock, PmAuthenticatedHttpOwner, PmAuthenticatedUserWsRole,
    PmCancelMutationTimeFinalizer, PmCancelServerTimeHttpRole, PmLiveMetadataPair,
    PmLiveMetadataPairSink, PmLoopbackMutationAuthError, PmOkxProductClock,
    PmPlaceMutationTimeFinalizer, PmPlaceServerTimeHttpRole, PmPrivateReadProductClock,
    PmProductClockError, PmPublicHttpRole, PmPublicMarketWsRole, PmPublicMetadataDeliveryError,
    PmPublicMetadataHttpRole, PmPublicWsTransportPolicy, PmReadServerTimeHttpRole,
};
use thiserror::Error;

use super::root::{PmAuthenticatedLoopbackProduct, PmAuthenticatedLoopbackProductParts};
use crate::{
    authenticated_journal::PmAuthenticatedJournalRecovery,
    capture::{PmCaptureProvenance, PmCaptureSessionPolicy},
    composition::{PmPublicCapture, PmPublicCaptureOutcome, PmPublicCaptureRunError},
    coordinator::{
        PmAuthenticatedExecutionError, PmCoordinator, PmCoordinatorAssemblyError,
        PmCoordinatorError, PmCoordinatorPolicy, PmCoordinatorShutdownError,
        PmLoopbackAuthenticatedExecutionShutdown, PmLoopbackAuthenticatedExecutionStage,
        PmLoopbackAuthenticatedMutationWorkers, PmMutationError,
    },
    journal::PmJournalRecovery,
    private_monitor::PmLiveOccurrenceIssuer,
};

/// Fully started, statically authenticated loopback product.
///
/// No network loop is spawned by this typestate. It owns every purpose role,
/// both durable recovery cuts, the sole coordinator (which owns public capture
/// and Goal-F mutation), and the final authenticated-journal/credential
/// shutdown authority. [`PmAuthenticatedLoopbackRun`](super::run::PmAuthenticatedLoopbackRun)
/// consumes these fields into its statically supervised actor/task topology.
pub(super) struct PmAuthenticatedLoopbackReady<M: PmQuoteModel> {
    pub(super) coordinator: Box<PmCoordinator<M>>,
    pub(super) metadata_http: PmPublicMetadataHttpRole,
    pub(super) book_http: Option<PmPublicHttpRole>,
    pub(super) read_server_time_http: Option<PmReadServerTimeHttpRole>,
    pub(super) private_read_clock: Option<PmPrivateReadProductClock>,
    pub(super) place_server_time_http: Option<PmPlaceServerTimeHttpRole>,
    pub(super) cancel_server_time_http: Option<PmCancelServerTimeHttpRole>,
    pub(super) public_ws: Option<PmPublicMarketWsRole>,
    pub(super) authenticated_http: Option<PmAuthenticatedHttpOwner>,
    pub(super) authenticated_user_ws: Option<PmAuthenticatedUserWsRole>,
    pub(super) mutation_workers: PmLoopbackAuthenticatedMutationWorkers,
    pub(super) actor_clock: PmActorProductClock,
    pub(super) okx_clock: PmOkxProductClock,
    pub(super) place_time_finalizer: PmPlaceMutationTimeFinalizer,
    pub(super) cancel_time_finalizer: PmCancelMutationTimeFinalizer,
    pub(super) occurrence_issuer: PmLiveOccurrenceIssuer,
    pub(super) goal_f_bridge_timeout: Duration,
    pub(super) controlled_shutdown_timeout: Duration,
    pub(super) goal_f_recovery: PmJournalRecovery,
    pub(super) authenticated_recovery: PmAuthenticatedJournalRecovery,
    pub(super) shutdown: PmLoopbackAuthenticatedExecutionShutdown,
}

impl<M: PmQuoteModel> PmAuthenticatedLoopbackReady<M> {
    /// Ordered shutdown for the no-socket static typestate.
    ///
    /// Purpose roles and mutation workers are destroyed first. The coordinator
    /// then closes Goal-F mutation and the sole public capture writer. The
    /// authenticated journal and credential authority are closed last, even
    /// when coordinator shutdown fails.
    pub(super) async fn shutdown(
        self,
    ) -> Result<PmAuthenticatedLoopbackStopped, PmAuthenticatedLoopbackShutdownError> {
        let Self {
            coordinator,
            metadata_http,
            book_http,
            read_server_time_http,
            private_read_clock,
            place_server_time_http,
            cancel_server_time_http,
            public_ws,
            authenticated_http,
            authenticated_user_ws,
            mutation_workers,
            actor_clock,
            okx_clock,
            place_time_finalizer,
            cancel_time_finalizer,
            occurrence_issuer: _,
            goal_f_bridge_timeout: _,
            controlled_shutdown_timeout: _,
            goal_f_recovery,
            authenticated_recovery,
            shutdown,
        } = self;
        drop(metadata_http);
        drop(book_http);
        drop(read_server_time_http);
        drop(private_read_clock);
        drop(place_server_time_http);
        drop(cancel_server_time_http);
        drop(public_ws);
        drop(authenticated_http);
        drop(authenticated_user_ws);
        drop(mutation_workers);
        drop(actor_clock);
        drop(okx_clock);
        drop(place_time_finalizer);
        drop(cancel_time_finalizer);
        let coordinator_result = coordinator.shutdown().await;
        let authenticated_result = shutdown.shutdown().await;
        match (coordinator_result, authenticated_result) {
            (Ok(public), Ok(())) => Ok(PmAuthenticatedLoopbackStopped {
                public,
                goal_f_recovery,
                authenticated_recovery,
                shutdown_effect_counts: [0; 5],
            }),
            (Err(coordinator), Ok(())) => Err(PmAuthenticatedLoopbackShutdownError::Coordinator(
                Box::new(coordinator),
            )),
            (Ok(_), Err(authenticated)) => Err(
                PmAuthenticatedLoopbackShutdownError::Authenticated(Box::new(authenticated)),
            ),
            (Err(coordinator), Err(authenticated)) => {
                Err(PmAuthenticatedLoopbackShutdownError::Both {
                    coordinator: Box::new(coordinator),
                    authenticated: Box::new(authenticated),
                })
            }
        }
    }
}

/// Clean terminal evidence returned only after both durability owners close.
pub(super) struct PmAuthenticatedLoopbackStopped {
    public: PmPublicCaptureOutcome,
    goal_f_recovery: PmJournalRecovery,
    authenticated_recovery: PmAuthenticatedJournalRecovery,
    shutdown_effect_counts: [u64; 5],
}

impl PmAuthenticatedLoopbackStopped {
    pub(super) const fn public(&self) -> &PmPublicCaptureOutcome {
        &self.public
    }

    pub(super) const fn goal_f_recovery(&self) -> &PmJournalRecovery {
        &self.goal_f_recovery
    }

    pub(super) const fn authenticated_recovery(&self) -> &PmAuthenticatedJournalRecovery {
        &self.authenticated_recovery
    }

    pub(super) const fn shutdown_effect_counts(&self) -> [u64; 5] {
        self.shutdown_effect_counts
    }

    pub(super) fn set_shutdown_effect_counts(&mut self, counts: [u64; 5]) {
        self.shutdown_effect_counts = counts;
    }
}

impl<M: PmQuoteModel> PmAuthenticatedLoopbackProduct<M> {
    /// Perform the complete static authenticated-loopback start sequence.
    ///
    /// This function deliberately starts no socket/read/mutation task. It
    /// opens only the already-owned capture and journal runtimes needed to
    /// prove that all scopes and both recovery cuts agree before a ready owner
    /// exists.
    #[allow(
        clippy::too_many_arguments,
        reason = "cold start keeps the three durable paths and their explicit policies visible"
    )]
    pub(super) async fn start(
        self,
        capture_path: PathBuf,
        goal_f_journal_path: PathBuf,
        authenticated_journal_path: PathBuf,
        session_policy: PmCaptureSessionPolicy,
        provenance: PmCaptureProvenance,
        coordinator_policy: PmCoordinatorPolicy,
        goal_f_durability_timeout: Duration,
        authenticated_durability_timeout: Duration,
        controlled_shutdown_timeout: Duration,
    ) -> Result<PmAuthenticatedLoopbackReady<M>, PmAuthenticatedLoopbackStartError> {
        if controlled_shutdown_timeout.is_zero() {
            return Err(PmAuthenticatedLoopbackStartError::InvalidControlledShutdownTimeout);
        }
        let (common, public_connectivity, private_connectivity, place_transport, cancel_transport) =
            self.into_parts();
        let public_config = common
            .plan
            .public_config()
            .expect("authenticated product plan carries public config")
            .clone();
        let account_config = common
            .plan
            .account_config()
            .expect("authenticated product plan carries account config")
            .clone();
        let config = PmConnectivityConfig::new(public_config.clone(), account_config)
            .map_err(PmAuthenticatedLoopbackStartError::Connectivity)?;

        let (
            metadata_http,
            book_http,
            read_server_time_http,
            private_read_clock,
            place_mutation_time,
            cancel_mutation_time,
            public_ws,
            user_ws_clock,
            mut actor_clock,
            okx_clock,
        ) = public_connectivity.into_roles().into_roles();
        let (place_server_time_http, place_time_finalizer) = place_mutation_time.into_roles();
        let (cancel_server_time_http, cancel_time_finalizer) = cancel_mutation_time.into_roles();
        validate_public_ws_policy(public_ws.transport_policy(), session_policy)?;
        let mut metadata_sink = AuthoritativeMetadataSink {
            instrument: public_config.instrument(),
            source: public_config.polymarket_route().source(),
            expected: public_config.expected_metadata(),
            actor_clock: &mut actor_clock,
        };
        let authoritative = metadata_http
            .refresh(&mut metadata_sink)
            .await
            .map_err(PmAuthenticatedLoopbackStartError::Metadata)?;

        let PmAuthenticatedLoopbackProductParts {
            model,
            plan,
            bindings,
            capture,
            private,
            preparation,
            schedule,
        } = common;
        let public = PmPublicCapture {
            plan,
            bindings,
            capture,
        }
        .start(capture_path, authoritative, session_policy, provenance)
        .await
        .map_err(PmAuthenticatedLoopbackStartError::PublicCapture)?;

        let assembly = match PmCoordinator::prepare_assembly(&config, model, coordinator_policy) {
            Ok(assembly) => assembly,
            Err(primary) => {
                let public_cleanup = public.finish().await.err().map(Box::new);
                return Err(PmAuthenticatedLoopbackStartError::CoordinatorPreflight {
                    primary,
                    public_cleanup,
                });
            }
        };

        let private_connectivity = match private_connectivity.split() {
            Ok(connectivity) => connectivity,
            Err(primary) => {
                let public_cleanup = public.finish().await.err().map(Box::new);
                return Err(PmAuthenticatedLoopbackStartError::PrivateConnectivity {
                    primary,
                    public_cleanup,
                });
            }
        };
        let stage = match PmLoopbackAuthenticatedExecutionStage::start(
            &config,
            authenticated_journal_path,
            authenticated_durability_timeout,
            private_connectivity,
            place_transport,
            cancel_transport,
        )
        .await
        {
            Ok(stage) => stage,
            Err(primary) => {
                let public_cleanup = public.finish().await.err().map(Box::new);
                return Err(PmAuthenticatedLoopbackStartError::AuthenticatedStage {
                    primary,
                    public_cleanup,
                });
            }
        };
        let (execution, mutation, goal_f_recovery, authenticated_recovery) = match stage
            .activate_after_goal_f_validation(
                &config,
                private,
                preparation,
                goal_f_journal_path,
                goal_f_durability_timeout,
            )
            .await
        {
            Ok(started) => started,
            Err(primary) => {
                let public_cleanup = public.finish().await.err().map(Box::new);
                return Err(PmAuthenticatedLoopbackStartError::AuthenticatedActivation {
                    primary,
                    public_cleanup,
                });
            }
        };

        let coordinator = match PmCoordinator::assemble_with_mutation(
            assembly,
            mutation,
            Box::new(public),
            schedule,
        ) {
            Ok(coordinator) => coordinator,
            Err(failure) => {
                let (primary, assembly, mutation, public, schedule) = failure.into_parts();
                drop(assembly);
                drop(schedule);
                let mutation = mutation.shutdown().await.err().map(Box::new);
                let public = public.finish().await.err().map(Box::new);
                let authenticated = execution.shutdown().await.err().map(Box::new);
                return Err(PmAuthenticatedLoopbackStartError::CoordinatorAssembly {
                    primary,
                    cleanup: PmAuthenticatedLoopbackCleanupErrors {
                        mutation,
                        public,
                        authenticated,
                    },
                });
            }
        };
        let (authenticated_http, authenticated_user_ws, mutation_workers, shutdown) =
            execution.into_roles();
        let authenticated_user_ws = authenticated_user_ws.with_clock_source(user_ws_clock);

        Ok(PmAuthenticatedLoopbackReady {
            coordinator,
            metadata_http,
            book_http: Some(book_http),
            read_server_time_http: Some(read_server_time_http),
            private_read_clock: Some(private_read_clock),
            place_server_time_http: Some(place_server_time_http),
            cancel_server_time_http: Some(cancel_server_time_http),
            public_ws: Some(public_ws),
            authenticated_http: Some(authenticated_http),
            authenticated_user_ws: Some(authenticated_user_ws),
            mutation_workers,
            actor_clock,
            okx_clock,
            place_time_finalizer,
            cancel_time_finalizer,
            occurrence_issuer: PmLiveOccurrenceIssuer::new(
                public_config.expected_metadata().condition(),
            ),
            goal_f_bridge_timeout: goal_f_durability_timeout,
            controlled_shutdown_timeout,
            goal_f_recovery,
            authenticated_recovery,
            shutdown,
        })
    }
}

pub(super) fn validate_public_ws_policy(
    transport: PmPublicWsTransportPolicy,
    session: PmCaptureSessionPolicy,
) -> Result<(), PmAuthenticatedLoopbackStartError> {
    let heartbeat = session
        .pm_heartbeat()
        .map_err(|_| PmAuthenticatedLoopbackStartError::PublicWsPolicyMismatch)?;
    let reconnect = session.pm_reconnect().as_transport();
    if transport.heartbeat_interval() != Duration::from_nanos(heartbeat.ping_interval_ns())
        || transport.pong_timeout() != Duration::from_nanos(heartbeat.pong_timeout_ns())
        || transport.initial_connection_epoch() != session.pm_initial_epoch()
        || transport.max_reconnect_attempts() == 0
        || reconnect.initial_delay > transport.max_reconnect_backoff()
        || reconnect.max_delay > transport.max_reconnect_backoff()
    {
        return Err(PmAuthenticatedLoopbackStartError::PublicWsPolicyMismatch);
    }
    Ok(())
}

struct AuthoritativeMetadataSink<'a> {
    instrument: reap_pm_core::PmInstrumentHandle,
    source: PmProductSource,
    expected: PmMarketMetadata,
    actor_clock: &'a mut PmActorProductClock,
}

impl PmLiveMetadataPairSink for AuthoritativeMetadataSink<'_> {
    type Output = PmAuthoritativeMetadata;
    type Error = PmAuthenticatedMetadataJoinError;

    fn deliver_native_metadata_pair(
        &mut self,
        pair: PmLiveMetadataPair<'_>,
    ) -> Result<Self::Output, Self::Error> {
        if pair.scope().condition() != self.expected.condition()
            || pair.scope().market() != self.expected.market()
            || pair.scope().token() != self.expected.outcome().token()
        {
            return Err(PmAuthenticatedMetadataJoinError::ScopeMismatch);
        }
        let receive = self.actor_clock.observe_control_edge()?;
        let revision = PmMetadataRevisionInput::new(
            SnapshotRevision::new(1),
            receive.received_clock().monotonic_receive_ns(),
        )?;
        PmAuthoritativeMetadata::join_live_clob_v2_raw(
            self.instrument,
            self.source,
            self.expected,
            pair.market_bytes(),
            pair.clob_v2_bytes(),
            revision,
        )
        .map_err(Into::into)
    }
}

#[derive(Debug, Error)]
pub(super) enum PmAuthenticatedMetadataJoinError {
    #[error("atomic metadata pair does not match the exact configured wire scope")]
    ScopeMismatch,
    #[error(transparent)]
    Clock(#[from] PmProductClockError),
    #[error(transparent)]
    Metadata(#[from] PmMetadataJoinError),
}

#[derive(Debug, Error)]
pub(super) enum PmAuthenticatedLoopbackStartError {
    #[error("controlled shutdown safety timeout must be nonzero")]
    InvalidControlledShutdownTimeout,
    #[error(
        "public WebSocket heartbeat, epoch, or reconnect bounds contradict the canonical capture session"
    )]
    PublicWsPolicyMismatch,
    #[error(transparent)]
    Connectivity(#[from] PmConnectivityConfigError),
    #[error(transparent)]
    Metadata(#[from] PmPublicMetadataDeliveryError<PmAuthenticatedMetadataJoinError>),
    #[error(transparent)]
    PublicCapture(#[from] PmPublicCaptureRunError),
    #[error("coordinator preflight failed: {primary}; public cleanup={public_cleanup:?}")]
    CoordinatorPreflight {
        #[source]
        primary: PmCoordinatorError,
        public_cleanup: Option<Box<PmPublicCaptureRunError>>,
    },
    #[error("private connectivity split failed: {primary}; public cleanup={public_cleanup:?}")]
    PrivateConnectivity {
        #[source]
        primary: PmLoopbackMutationAuthError,
        public_cleanup: Option<Box<PmPublicCaptureRunError>>,
    },
    #[error("authenticated stage start failed: {primary}; public cleanup={public_cleanup:?}")]
    AuthenticatedStage {
        #[source]
        primary: PmAuthenticatedExecutionError,
        public_cleanup: Option<Box<PmPublicCaptureRunError>>,
    },
    #[error("authenticated activation failed: {primary}; public cleanup={public_cleanup:?}")]
    AuthenticatedActivation {
        #[source]
        primary: PmAuthenticatedExecutionError,
        public_cleanup: Option<Box<PmPublicCaptureRunError>>,
    },
    #[error("coordinator assembly failed: {primary}; cleanup={cleanup}")]
    CoordinatorAssembly {
        #[source]
        primary: PmCoordinatorAssemblyError,
        cleanup: PmAuthenticatedLoopbackCleanupErrors,
    },
}

#[derive(Debug, Default)]
pub(super) struct PmAuthenticatedLoopbackCleanupErrors {
    mutation: Option<Box<PmMutationError>>,
    public: Option<Box<PmPublicCaptureRunError>>,
    authenticated: Option<Box<PmAuthenticatedExecutionError>>,
}

impl fmt::Display for PmAuthenticatedLoopbackCleanupErrors {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(
            formatter,
            "mutation={:?}, public={:?}, authenticated={:?}",
            self.mutation, self.public, self.authenticated,
        )
    }
}

#[derive(Debug, Error)]
pub(super) enum PmAuthenticatedLoopbackShutdownError {
    #[error(transparent)]
    Coordinator(Box<PmCoordinatorShutdownError>),
    #[error(transparent)]
    Authenticated(Box<PmAuthenticatedExecutionError>),
    #[error(
        "coordinator and authenticated shutdown both failed: coordinator={coordinator}; authenticated={authenticated}"
    )]
    Both {
        coordinator: Box<PmCoordinatorShutdownError>,
        authenticated: Box<PmAuthenticatedExecutionError>,
    },
}
