use std::{
    collections::BTreeSet,
    convert::Infallible,
    fs::{File, OpenOptions},
    io::Write as _,
    path::{Path, PathBuf},
    time::{Duration, Instant, SystemTime, UNIX_EPOCH},
};

use async_trait::async_trait;
use reap_pm_core::{ConnectionEpoch, EvmAddress, PmAssetId, PmBookQuantity, PmOrderSide};
use reap_polymarket_live_adapter::{
    PM_CLOB_PRODUCTION_ORIGIN, PM_CREDENTIAL_AUTHORITY_READ_ONLY_SHUTDOWN_BOUNDS,
    PmAuthenticatedHttpOwner, PmCompleteOpenOrdersCut, PmCompleteTradesCut, PmLiveAdapterError,
    PmLiveMetadataPair, PmLiveMetadataPairSink, PmOpenOrdersCutProgress, PmPublicHttpConfig,
    PmPublicMetadataDeliveryError, PmPublicMetadataHttpRole, PmReadOnlyPrivateConnectivityOwner,
    PmReadServerTimeHttpRole, PmTradesCutProgress, PmUserWsBounds, PmUserWsDisconnectReason,
    PmUserWsEvent, PmUserWsEventSink, PmUserWsRunError, PmUserWsTransportError,
    pm_user_ws_shutdown_channel,
};
use reap_polymarket_wire::{MAX_PM_LIVE_BODY_BYTES, PmLiveOrder, PmLiveTrade, PmLiveUserEvent};
use reap_telemetry::current_executable_sha256;
use thiserror::Error;

use crate::{
    PmReadOnlyAccountEvidence, PmReadOnlyAllowanceEvidence, PmReadOnlyCollectionFailureEvidence,
    PmReadOnlyConfigEvidence, PmReadOnlyMetadataEvidence, PmReadOnlyOrderEvidence,
    PmReadOnlyReconciliationEvidence, PmReadOnlySmokeArtifact, PmReadOnlySmokeConfig,
    PmReadOnlySmokeConfigError, PmReadOnlySmokeVerificationError, PmReadOnlyTeardownEvidence,
    PmReadOnlyTradeEvidence, PmReadOnlyTradeMakerEvidence, PmReadOnlyUserStreamEvidence,
    load_pm_read_only_credentials, load_pm_read_only_smoke_config_path,
    verify_pm_read_only_smoke_artifact_bytes,
};
use crate::{PmReadOnlyCredentialError, credentials::PmReadOnlyArtifactSecretGuard};

const MAX_HOST_NAME_BYTES: usize = 255;
const MAX_ARTIFACT_ATTEMPT_MS: u64 = 30 * 60 * 1_000;
const MAX_AUTHENTICATED_ACCOUNT_PHASE_MS: u64 = 5 * 60 * 1_000;
const MAX_RECONCILIATION_PHASE_MS: u64 = 10 * 60 * 1_000;
const USER_STREAM_GRACEFUL_JOIN_MS: u64 = 30_000;

#[derive(Debug, Error)]
pub enum PmReadOnlySmokeError {
    #[error(transparent)]
    Config(#[from] PmReadOnlySmokeConfigError),
    #[error(transparent)]
    Credential(#[from] PmReadOnlyCredentialError),
    #[error("failed to reserve the create-new read-only artifact")]
    ReserveOutput(#[source] std::io::Error),
    #[error(
        "the read-only artifact parent must be a real mode-0700 directory owned by this process"
    )]
    InvalidOutputParent,
    #[error("failed to persist the read-only artifact")]
    PersistOutput(#[source] std::io::Error),
    #[error("failed to collect fixed public Polymarket metadata")]
    PublicMetadata,
    #[error("failed to construct fixed read-only connectivity")]
    Connectivity,
    #[error("an authenticated read failed")]
    AuthenticatedRead,
    #[error("the authenticated user-stream task failed")]
    UserStream,
    #[error("the credential authority did not shut down cleanly")]
    CredentialShutdown,
    #[error("system provenance is unavailable")]
    Provenance,
    #[error("system time is unavailable")]
    Clock,
    #[error("read-only evidence serialization failed")]
    Serialization(#[source] serde_json::Error),
    #[error(transparent)]
    Verification(#[from] PmReadOnlySmokeVerificationError),
    #[error("the finalized artifact exceeds its fixed byte bound")]
    ArtifactTooLarge,
    #[error("the protected credential directory was not supplied")]
    MissingCredentialsDirectory,
}

/// Resolve the separately supplied, non-secret systemd credential-directory
/// path. Secret values themselves are never accepted from the environment.
pub fn resolve_pm_read_only_credentials_directory(
    explicit: Option<&Path>,
) -> Result<PathBuf, PmReadOnlySmokeError> {
    if let Some(path) = explicit {
        return Ok(path.to_path_buf());
    }
    std::env::var_os("CREDENTIALS_DIRECTORY")
        .filter(|value| !value.is_empty())
        .map(PathBuf::from)
        .ok_or(PmReadOnlySmokeError::MissingCredentialsDirectory)
}

pub async fn collect_pm_read_only_smoke_path(
    config_path: impl AsRef<Path>,
    credentials_directory: impl AsRef<Path>,
    output_path: impl AsRef<Path>,
) -> Result<PmReadOnlySmokeArtifact, PmReadOnlySmokeError> {
    let (config, config_evidence) = load_pm_read_only_smoke_config_path(config_path)?;
    let started_monotonic = Instant::now();
    let started_unix_ms = unix_ms()?;
    let provenance = Provenance::collect(started_monotonic)?;
    let mut output = reserve_private_output(output_path.as_ref())?;

    let credentials = load_pm_read_only_credentials(
        credentials_directory.as_ref(),
        &config.api_key_file,
        &config.secret_file,
        &config.passphrase_file,
    )?;
    // A collision means the otherwise non-secret embedded configuration is
    // itself secret-bearing. Never hand those bytes to artifact assembly; the
    // reserved placeholder is removed by `ReservedOutput::drop` on this error.
    let (credential_input, artifact_secret_guard) =
        credentials.into_adapter_input_and_artifact_guard();
    artifact_secret_guard
        .ensure_config_is_secret_free(config_evidence.canonical_toml.as_bytes())?;
    let metadata = match collect_public_metadata(&config).await {
        Ok(metadata) => metadata,
        Err(failure) => {
            return finish_attempt(
                &config,
                config_evidence,
                provenance,
                started_unix_ms,
                None,
                None,
                None,
                None,
                empty_teardown(true),
                failure,
                Some(&artifact_secret_guard),
                &mut output,
            );
        }
    };
    let private = match collect_authenticated(&config, credential_input).await {
        Ok(private) => private,
        Err(_) => {
            return finish_attempt(
                &config,
                config_evidence,
                provenance,
                started_unix_ms,
                Some(metadata),
                None,
                None,
                None,
                empty_teardown(true),
                FailureClass::new("authenticated_account", "transport"),
                Some(&artifact_secret_guard),
                &mut output,
            );
        }
    };
    finish_private_attempt(
        &config,
        config_evidence,
        provenance,
        started_unix_ms,
        metadata,
        private,
        &artifact_secret_guard,
        &mut output,
    )
}

#[allow(clippy::too_many_arguments)]
fn finish_private_attempt(
    config: &PmReadOnlySmokeConfig,
    config_evidence: PmReadOnlyConfigEvidence,
    provenance: Provenance,
    started_unix_ms: u64,
    metadata: PmReadOnlyMetadataEvidence,
    private: PrivateCollection,
    artifact_secret_guard: &PmReadOnlyArtifactSecretGuard,
    output: &mut ReservedOutput,
) -> Result<PmReadOnlySmokeArtifact, PmReadOnlySmokeError> {
    let finished_unix_ms = bounded_finished_unix_ms(started_unix_ms, &provenance)?;
    let observed_failure = private.failure.map(|failure| {
        PmReadOnlyCollectionFailureEvidence::new(failure.stage, failure.kind, finished_unix_ms)
            .expect("collector failure classification is closed and timestamp-bounded")
    });
    let mut artifact = assemble_artifact(
        config,
        config_evidence,
        provenance,
        started_unix_ms,
        finished_unix_ms,
        Some(metadata),
        private.account,
        private.reconciliation,
        private.user_stream,
        private.teardown,
        observed_failure,
        private.failure.is_none(),
    )?;
    if private.failure.is_none() {
        artifact.collection_failure = if all_observation_gates_pass(&artifact.summary) {
            None
        } else {
            let failure = classify_failed_observation_gate(&artifact)?;
            Some(PmReadOnlyCollectionFailureEvidence::new(
                failure.stage,
                failure.kind,
                finished_unix_ms,
            )?)
        };
        artifact.finalize(config)?;
    }
    output.persist(&artifact, Some(artifact_secret_guard))?;
    Ok(artifact)
}

#[derive(Clone, Copy, Debug)]
struct FailureClass {
    stage: &'static str,
    kind: &'static str,
}

impl FailureClass {
    const fn new(stage: &'static str, kind: &'static str) -> Self {
        Self { stage, kind }
    }
}

#[allow(clippy::too_many_arguments)]
fn finish_attempt(
    config: &PmReadOnlySmokeConfig,
    config_evidence: PmReadOnlyConfigEvidence,
    provenance: Provenance,
    started_unix_ms: u64,
    metadata: Option<PmReadOnlyMetadataEvidence>,
    account: Option<PmReadOnlyAccountEvidence>,
    reconciliation: Option<PmReadOnlyReconciliationEvidence>,
    user_stream: Option<PmReadOnlyUserStreamEvidence>,
    teardown: PmReadOnlyTeardownEvidence,
    failure: FailureClass,
    artifact_secret_guard: Option<&PmReadOnlyArtifactSecretGuard>,
    output: &mut ReservedOutput,
) -> Result<PmReadOnlySmokeArtifact, PmReadOnlySmokeError> {
    let finished_unix_ms = bounded_finished_unix_ms(started_unix_ms, &provenance)?;
    let failure =
        PmReadOnlyCollectionFailureEvidence::new(failure.stage, failure.kind, finished_unix_ms)?;
    let artifact = assemble_artifact(
        config,
        config_evidence,
        provenance,
        started_unix_ms,
        finished_unix_ms,
        metadata,
        account,
        reconciliation,
        user_stream,
        teardown,
        Some(failure),
        false,
    )?;
    output.persist(&artifact, artifact_secret_guard)?;
    Ok(artifact)
}

fn bounded_finished_unix_ms(
    started_unix_ms: u64,
    provenance: &Provenance,
) -> Result<u64, PmReadOnlySmokeError> {
    let elapsed_ms = u64::try_from(provenance.started_monotonic.elapsed().as_millis())
        .map_err(|_| PmReadOnlySmokeError::Clock)?;
    if elapsed_ms > MAX_ARTIFACT_ATTEMPT_MS {
        return Err(PmReadOnlySmokeError::Clock);
    }
    started_unix_ms
        .checked_add(elapsed_ms)
        .ok_or(PmReadOnlySmokeError::Clock)
}

fn all_observation_gates_pass(summary: &crate::PmReadOnlySmokeSummary) -> bool {
    summary.provenance_valid
        && summary.config_valid
        && summary.authorization_closed
        && summary.public_metadata_valid
        && summary.account_balances_observed
        && summary.position_observed
        && summary.required_allowances_complete
        && summary.open_orders_complete
        && summary.trades_complete
        && summary.owner_evidence_non_vacuous
        && summary.user_stream_authenticated
        && summary.teardown_complete
        && summary.limitations_explicit
}

fn classify_failed_observation_gate(
    artifact: &PmReadOnlySmokeArtifact,
) -> Result<FailureClass, PmReadOnlySmokeError> {
    let summary = &artifact.summary;
    if !summary.provenance_valid || !summary.config_valid || !summary.authorization_closed {
        return Err(PmReadOnlySmokeVerificationError::Invalid(
            "collector-created artifact invariants are invalid",
        )
        .into());
    }
    let failure = if !summary.public_metadata_valid {
        FailureClass::new("public_metadata", "insufficient_evidence")
    } else if !summary.account_balances_observed
        || !summary.position_observed
        || !summary.required_allowances_complete
    {
        FailureClass::new("authenticated_account", "insufficient_evidence")
    } else if !summary.open_orders_complete {
        FailureClass::new("open_orders", "insufficient_evidence")
    } else if !summary.trades_complete {
        FailureClass::new("trades", "insufficient_evidence")
    } else if !summary.owner_evidence_non_vacuous {
        FailureClass::new("user_stream", "insufficient_evidence")
    } else if !summary.user_stream_authenticated {
        let kind = artifact
            .user_stream
            .as_ref()
            .map_or("insufficient_evidence", |stream| {
                if stream.owner_mismatch_count > 0 {
                    "owner_mismatch"
                } else if stream.scope_mismatch_count > 0 {
                    "scope_mismatch"
                } else {
                    "insufficient_evidence"
                }
            });
        FailureClass::new("user_stream", kind)
    } else if !summary.teardown_complete {
        FailureClass::new("teardown", "shutdown")
    } else {
        return Err(PmReadOnlySmokeVerificationError::Invalid(
            "collector failure classification has no failed observation gate",
        )
        .into());
    };
    Ok(failure)
}

fn empty_teardown(credentials_loaded: bool) -> PmReadOnlyTeardownEvidence {
    PmReadOnlyTeardownEvidence {
        user_stream_task_started: false,
        user_stream_shutdown_requested: false,
        user_stream_abort_requested: false,
        user_stream_task_joined: false,
        user_stream_task_completed_cleanly: false,
        credential_authority_task_started: false,
        credential_authority_shutdown_requested: false,
        credential_authority_abort_requested: false,
        credential_authority_task_joined: false,
        credential_authority_task_completed_cleanly: false,
        credentials_loaded,
        credentials_dropped_before_return: credentials_loaded,
        all_tasks_joined: true,
        mutation_roles_constructed: false,
        mutation_requests: 0,
    }
}

struct Provenance {
    binary_sha256: String,
    host_name: String,
    started_monotonic: Instant,
}

impl Provenance {
    fn collect(started_monotonic: Instant) -> Result<Self, PmReadOnlySmokeError> {
        let binary_sha256 =
            current_executable_sha256().map_err(|_| PmReadOnlySmokeError::Provenance)?;
        let host_name = std::fs::read_to_string("/proc/sys/kernel/hostname")
            .map_err(|_| PmReadOnlySmokeError::Provenance)?;
        let host_name = host_name.trim_end_matches(['\r', '\n']).to_owned();
        if host_name.is_empty()
            || host_name.len() > MAX_HOST_NAME_BYTES
            || !host_name.is_ascii()
            || host_name.bytes().any(|byte| byte.is_ascii_control())
        {
            return Err(PmReadOnlySmokeError::Provenance);
        }
        Ok(Self {
            binary_sha256,
            host_name,
            started_monotonic,
        })
    }
}

struct MetadataSink<'a> {
    config: &'a PmReadOnlySmokeConfig,
}

impl PmLiveMetadataPairSink for MetadataSink<'_> {
    type Output = PmReadOnlyMetadataEvidence;
    type Error = PmReadOnlySmokeVerificationError;

    fn deliver_native_metadata_pair(
        &mut self,
        pair: PmLiveMetadataPair<'_>,
    ) -> Result<Self::Output, Self::Error> {
        if pair.scope() != self.config.wire_scope()? {
            return Err(PmReadOnlySmokeVerificationError::Invalid(
                "public metadata scope mismatch",
            ));
        }
        PmReadOnlyMetadataEvidence::from_public_bodies(
            self.config,
            pair.market_bytes(),
            pair.clob_v2_bytes(),
        )
    }
}

async fn collect_public_metadata(
    config: &PmReadOnlySmokeConfig,
) -> Result<PmReadOnlyMetadataEvidence, FailureClass> {
    let public_config = PmPublicHttpConfig::production(
        PM_CLOB_PRODUCTION_ORIGIN,
        Duration::from_millis(config.connect_timeout_ms),
        Duration::from_millis(config.request_timeout_ms),
    )
    .map_err(|error| classify_live_failure("public_metadata", error))?;
    let role = PmPublicMetadataHttpRole::new(
        public_config,
        config
            .wire_scope()
            .map_err(|_| FailureClass::new("public_metadata", "invalid_input"))?,
    )
    .map_err(|error| classify_live_failure("public_metadata", error))?;
    collect_public_metadata_with_role(config, role).await
}

async fn collect_public_metadata_with_role(
    config: &PmReadOnlySmokeConfig,
    role: PmPublicMetadataHttpRole,
) -> Result<PmReadOnlyMetadataEvidence, FailureClass> {
    role.refresh(&mut MetadataSink { config })
        .await
        .map_err(|error| match error {
            PmPublicMetadataDeliveryError::Http(error) => {
                classify_live_failure("public_metadata", error)
            }
            PmPublicMetadataDeliveryError::Sink(_) => {
                FailureClass::new("public_metadata", "scope_mismatch")
            }
        })
}

struct PrivateCollection {
    account: Option<PmReadOnlyAccountEvidence>,
    reconciliation: Option<PmReadOnlyReconciliationEvidence>,
    user_stream: Option<PmReadOnlyUserStreamEvidence>,
    teardown: PmReadOnlyTeardownEvidence,
    failure: Option<FailureClass>,
}

type UserTaskOutput = (Result<(), PmUserWsRunError<Infallible>>, UserEvidenceSink);

struct UserTaskCancellationFailStop {
    task: Option<tokio::task::JoinHandle<UserTaskOutput>>,
}

impl UserTaskCancellationFailStop {
    fn new(task: tokio::task::JoinHandle<UserTaskOutput>) -> Self {
        Self { task: Some(task) }
    }

    fn task_mut(&mut self) -> &mut tokio::task::JoinHandle<UserTaskOutput> {
        self.task
            .as_mut()
            .expect("user task is retained until its graceful join completes")
    }

    fn disarm_after_join(&mut self) {
        drop(self.task.take());
    }
}

impl Drop for UserTaskCancellationFailStop {
    fn drop(&mut self) {
        if self.task.is_some() {
            // The outer user task supervises a nested authenticated socket
            // worker. Cancelling or timing out this join cannot prove that
            // nested task's destructors ran, so returning would be unsafe.
            std::process::abort();
        }
    }
}

async fn join_user_task_bounded(
    mut task: UserTaskCancellationFailStop,
    graceful_join: Duration,
) -> UserTaskOutput {
    match tokio::time::timeout(graceful_join, task.task_mut()).await {
        Ok(Ok(output)) => {
            task.disarm_after_join();
            output
        }
        // A panicked/cancelled outer task may only have scheduled an abort of
        // its nested authenticated socket worker. Its JoinError is not proof
        // that the nested worker's destructors completed.
        Ok(Err(_)) => std::process::abort(),
        // The outer task owns a nested socket worker whose abort is
        // asynchronous. A timeout cannot truthfully establish joined secret
        // teardown, so fail-stop rather than detaching either task.
        Err(_) => std::process::abort(),
    }
}

async fn collect_authenticated(
    config: &PmReadOnlySmokeConfig,
    credentials: reap_polymarket_live_adapter::PmReadOnlyCredentialInput,
) -> Result<PrivateCollection, PmReadOnlySmokeError> {
    let scope = config
        .wire_scope()
        .map_err(|_| PmReadOnlySmokeError::Connectivity)?;
    let bounds = PmUserWsBounds::new(
        Duration::from_millis(config.connect_timeout_ms),
        Duration::from_millis(config.user_stream_idle_timeout_ms),
        Duration::from_millis(config.user_stream_pong_timeout_ms),
        MAX_PM_LIVE_BODY_BYTES,
        config.user_stream_max_reconnect_attempts,
        Duration::from_millis(config.user_stream_reconnect_backoff_ms),
        config.user_stream_event_channel_capacity,
        ConnectionEpoch::new(1),
    )
    .map_err(|_| PmReadOnlySmokeError::Connectivity)?;
    // Construct the public `/time` role before starting the credential
    // authority, so every post-credential path reaches explicit teardown.
    let time = PmReadServerTimeHttpRole::new(
        PmPublicHttpConfig::production(
            PM_CLOB_PRODUCTION_ORIGIN,
            Duration::from_millis(config.connect_timeout_ms),
            Duration::from_millis(config.request_timeout_ms),
        )
        .map_err(|_| PmReadOnlySmokeError::Connectivity)?,
    )
    .map_err(|_| PmReadOnlySmokeError::Connectivity)?;
    let owner = PmReadOnlyPrivateConnectivityOwner::production(
        config
            .signer()
            .map_err(|_| PmReadOnlySmokeError::Connectivity)?,
        config
            .funder()
            .map_err(|_| PmReadOnlySmokeError::Connectivity)?,
        scope,
        Duration::from_millis(config.connect_timeout_ms),
        Duration::from_millis(config.request_timeout_ms),
        bounds,
        credentials,
    )
    .map_err(|_| PmReadOnlySmokeError::Connectivity)?;
    collect_authenticated_with_owner(config, time, owner).await
}

async fn collect_authenticated_with_owner(
    config: &PmReadOnlySmokeConfig,
    time: PmReadServerTimeHttpRole,
    owner: PmReadOnlyPrivateConnectivityOwner,
) -> Result<PrivateCollection, PmReadOnlySmokeError> {
    let scope = config
        .wire_scope()
        .map_err(|_| PmReadOnlySmokeError::Connectivity)?;
    let signer = config
        .signer()
        .map_err(|_| PmReadOnlySmokeError::Connectivity)?;
    let roles = owner
        .split()
        .map_err(|_| PmReadOnlySmokeError::Connectivity)?;
    let user_role = roles.authenticated_user_ws;
    let credential_supervisor = roles.credential_supervisor;

    let (shutdown, shutdown_signal) = pm_user_ws_shutdown_channel();
    let user_scope = scope;
    let user_signer = signer;
    let user_started = Instant::now();
    let user_task = UserTaskCancellationFailStop::new(tokio::spawn(async move {
        let mut sink = UserEvidenceSink::new(user_scope, user_signer);
        let result = user_role.run(shutdown_signal, &mut sink).await;
        (result, sink)
    }));

    let (reads, actual_dwell_ms, user_output) = {
        let mut authenticated_http = roles.authenticated_http;
        let dwell = Duration::from_millis(config.user_stream_dwell_ms);
        let (reads, actual_dwell_ms) = {
            let reads = collect_authenticated_http(config, &time, &mut authenticated_http);
            tokio::pin!(reads);
            let dwell_timer = tokio::time::sleep(dwell);
            tokio::pin!(dwell_timer);
            tokio::select! {
                result = &mut reads => {
                    if result.is_ok() {
                        dwell_timer.as_mut().await;
                    }
                    shutdown.request_shutdown();
                    let actual = u64::try_from(user_started.elapsed().as_millis()).unwrap_or(u64::MAX);
                    (result, actual)
                }
                () = &mut dwell_timer => {
                    shutdown.request_shutdown();
                    let actual = u64::try_from(user_started.elapsed().as_millis()).unwrap_or(u64::MAX);
                    (reads.await, actual)
                }
            }
        };
        let user_output = join_user_task_bounded(
            user_task,
            Duration::from_millis(USER_STREAM_GRACEFUL_JOIN_MS),
        )
        .await;
        (reads, actual_dwell_ms, user_output)
    };
    let credential_teardown = credential_supervisor
        .shutdown_bounded(PM_CREDENTIAL_AUTHORITY_READ_ONLY_SHUTDOWN_BOUNDS)
        .await;

    let (user_result, mut user_sink) = user_output;
    user_sink.run_completed_without_transport_error = user_result.is_ok();
    user_sink.dwell_ms = actual_dwell_ms;
    let user_failure = user_result.err().map(classify_user_failure);
    let user_stream = Some(user_sink.into_evidence());
    let teardown = PmReadOnlyTeardownEvidence {
        user_stream_task_started: true,
        user_stream_shutdown_requested: true,
        user_stream_abort_requested: false,
        user_stream_task_joined: true,
        user_stream_task_completed_cleanly: true,
        credential_authority_task_started: true,
        credential_authority_shutdown_requested: credential_teardown.shutdown_requested(),
        credential_authority_abort_requested: credential_teardown.abort_requested(),
        credential_authority_task_joined: credential_teardown.task_joined(),
        credential_authority_task_completed_cleanly: credential_teardown.task_completed_cleanly(),
        credentials_loaded: true,
        credentials_dropped_before_return: credential_teardown.credentials_dropped(),
        all_tasks_joined: credential_teardown.task_joined(),
        mutation_roles_constructed: false,
        mutation_requests: 0,
    };
    let (account, reconciliation, read_failure) = match reads {
        Ok((account, reconciliation)) => (Some(account), Some(reconciliation), None),
        Err(failure) => (None, None, Some(failure)),
    };
    let teardown_failure = (!credential_teardown.shutdown_requested()
        || credential_teardown.abort_requested()
        || !credential_teardown.task_joined()
        || !credential_teardown.task_completed_cleanly()
        || !credential_teardown.credentials_dropped())
    .then_some(FailureClass::new("teardown", "shutdown"));
    let failure = teardown_failure.or(read_failure).or(user_failure);
    Ok(PrivateCollection {
        account,
        reconciliation,
        user_stream,
        teardown,
        failure,
    })
}

fn classify_user_failure(error: PmUserWsRunError<Infallible>) -> FailureClass {
    match error {
        PmUserWsRunError::Transport(PmUserWsTransportError::RetryExhausted {
            final_reason,
            ..
        }) => classify_user_disconnect(final_reason),
        PmUserWsRunError::Transport(_) => FailureClass::new("user_stream", "transport"),
        PmUserWsRunError::Sink(never) => match never {},
    }
}

const fn classify_user_disconnect(reason: PmUserWsDisconnectReason) -> FailureClass {
    match reason {
        PmUserWsDisconnectReason::CredentialOwnerMismatch => {
            FailureClass::new("user_stream", "owner_mismatch")
        }
        PmUserWsDisconnectReason::BinaryFrame
        | PmUserWsDisconnectReason::FrameTooLarge
        | PmUserWsDisconnectReason::MalformedFrame
        | PmUserWsDisconnectReason::UnexpectedProtocolFrame => {
            FailureClass::new("user_stream", "malformed_response")
        }
        PmUserWsDisconnectReason::ConnectTimeout
        | PmUserWsDisconnectReason::SubscriptionWriteTimeout
        | PmUserWsDisconnectReason::SocketWriteTimeout
        | PmUserWsDisconnectReason::IdleTimeout
        | PmUserWsDisconnectReason::PongTimeout => FailureClass::new("user_stream", "timeout"),
        PmUserWsDisconnectReason::ConnectFailed
        | PmUserWsDisconnectReason::SubscriptionWriteFailed
        | PmUserWsDisconnectReason::SocketClosed
        | PmUserWsDisconnectReason::SocketReadFailed
        | PmUserWsDisconnectReason::SocketWriteFailed => {
            FailureClass::new("user_stream", "reconnect_exhausted")
        }
        PmUserWsDisconnectReason::SubscriptionAuthenticationFailed
        | PmUserWsDisconnectReason::CredentialAuthorityUnavailable => {
            FailureClass::new("user_stream", "transport")
        }
    }
}

fn classify_live_failure(stage: &'static str, error: PmLiveAdapterError) -> FailureClass {
    let kind = match error {
        PmLiveAdapterError::RequestTimeout => "timeout",
        PmLiveAdapterError::Redirect { .. } | PmLiveAdapterError::UnexpectedStatus { .. } => {
            "rejected_status"
        }
        PmLiveAdapterError::Wire(_)
        | PmLiveAdapterError::PrivateWire(_)
        | PmLiveAdapterError::ResponseBodyTooLarge { .. }
        | PmLiveAdapterError::ExactOrderIdentityMismatch => "malformed_response",
        PmLiveAdapterError::CredentialOwnerMismatch
        | PmLiveAdapterError::ExactOrderMakerMismatch => "owner_mismatch",
        PmLiveAdapterError::ExactOrderScopeMismatch => "scope_mismatch",
        PmLiveAdapterError::PaginationCursorCycle
        | PmLiveAdapterError::PaginationPageLimit
        | PmLiveAdapterError::PaginationRowLimit => "incomplete_pagination",
        _ => "transport",
    };
    FailureClass::new(stage, kind)
}

async fn collect_authenticated_http(
    config: &PmReadOnlySmokeConfig,
    time: &PmReadServerTimeHttpRole,
    owner: &mut PmAuthenticatedHttpOwner,
) -> Result<(PmReadOnlyAccountEvidence, PmReadOnlyReconciliationEvidence), FailureClass> {
    let account = tokio::time::timeout(
        Duration::from_millis(MAX_AUTHENTICATED_ACCOUNT_PHASE_MS),
        async {
            let collateral = {
                let server_time = time
                    .fresh_read_server_time()
                    .await
                    .map_err(|error| classify_live_failure("authenticated_account", error))?;
                let mut account = owner.account();
                account
                    .collateral_balance_allowance(server_time)
                    .await
                    .map_err(|error| classify_live_failure("authenticated_account", error))?
            };
            let conditional = {
                let server_time = time
                    .fresh_read_server_time()
                    .await
                    .map_err(|error| classify_live_failure("authenticated_account", error))?;
                let mut account = owner.account();
                account
                    .conditional_balance_allowance(server_time)
                    .await
                    .map_err(|error| classify_live_failure("authenticated_account", error))?
            };
            account_evidence(config, collateral.into_value(), conditional.into_value())
                .map_err(|_| FailureClass::new("authenticated_account", "insufficient_evidence"))
        },
    )
    .await
    .map_err(|_| FailureClass::new("authenticated_account", "timeout"))??;
    let open_orders = tokio::time::timeout(
        Duration::from_millis(MAX_RECONCILIATION_PHASE_MS),
        collect_open_orders(time, owner),
    )
    .await
    .map_err(|_| FailureClass::new("open_orders", "timeout"))?
    .map_err(|error| classify_live_failure("open_orders", error))?;
    let trades = tokio::time::timeout(
        Duration::from_millis(MAX_RECONCILIATION_PHASE_MS),
        collect_trades(time, owner),
    )
    .await
    .map_err(|_| FailureClass::new("trades", "timeout"))?
    .map_err(|error| classify_live_failure("trades", error))?;
    let reconciliation = reconciliation_evidence(config, open_orders, trades)
        .map_err(|_| FailureClass::new("trades", "scope_mismatch"))?;
    Ok((account, reconciliation))
}

async fn collect_open_orders(
    time: &PmReadServerTimeHttpRole,
    owner: &mut PmAuthenticatedHttpOwner,
) -> Result<PmCompleteOpenOrdersCut, PmLiveAdapterError> {
    let first_time = time.fresh_read_server_time().await?;
    let mut role = owner.reconciliation();
    let mut progress = role.begin_open_orders(first_time).await?;
    loop {
        match progress {
            PmOpenOrdersCutProgress::Complete(complete) => return Ok(complete),
            PmOpenOrdersCutProgress::Incomplete(assembly) => {
                let server_time = time.fresh_read_server_time().await?;
                progress = role.continue_open_orders(server_time, assembly).await?;
            }
        }
    }
}

async fn collect_trades(
    time: &PmReadServerTimeHttpRole,
    owner: &mut PmAuthenticatedHttpOwner,
) -> Result<PmCompleteTradesCut, PmLiveAdapterError> {
    let first_time = time.fresh_read_server_time().await?;
    let mut role = owner.reconciliation();
    let mut progress = role.begin_trades(first_time).await?;
    loop {
        match progress {
            PmTradesCutProgress::Complete(complete) => return Ok(complete),
            PmTradesCutProgress::Incomplete(assembly) => {
                let server_time = time.fresh_read_server_time().await?;
                progress = role.continue_trades(server_time, assembly).await?;
            }
        }
    }
}

struct UserEvidenceSink {
    scope: reap_polymarket_wire::PmWireScope,
    signer: EvmAddress,
    attempted_epochs: BTreeSet<u64>,
    connection_open_count: u64,
    subscription_count: u64,
    reconnect_attempt_count: u64,
    retirement_count: u64,
    retry_exhausted_count: u64,
    ping_count: u64,
    correlated_pong_count: u64,
    frame_count: u64,
    event_count: u64,
    order_event_count: u64,
    trade_event_count: u64,
    owner_bound_event_count: u64,
    scope_bound_event_count: u64,
    scope_mismatch_count: u64,
    dwell_ms: u64,
    shutdown_event_count: u64,
    run_completed_without_transport_error: bool,
}

impl UserEvidenceSink {
    fn new(scope: reap_polymarket_wire::PmWireScope, signer: EvmAddress) -> Self {
        Self {
            scope,
            signer,
            attempted_epochs: BTreeSet::new(),
            connection_open_count: 0,
            subscription_count: 0,
            reconnect_attempt_count: 0,
            retirement_count: 0,
            retry_exhausted_count: 0,
            ping_count: 0,
            correlated_pong_count: 0,
            frame_count: 0,
            event_count: 0,
            order_event_count: 0,
            trade_event_count: 0,
            owner_bound_event_count: 0,
            scope_bound_event_count: 0,
            scope_mismatch_count: 0,
            dwell_ms: 0,
            shutdown_event_count: 0,
            run_completed_without_transport_error: false,
        }
    }

    fn observe_epoch(&mut self, epoch: ConnectionEpoch) {
        self.attempted_epochs.insert(epoch.value());
    }

    fn observe_business_event(&mut self, event: &PmLiveUserEvent) {
        self.event_count = self.event_count.saturating_add(1);
        self.owner_bound_event_count = self.owner_bound_event_count.saturating_add(1);
        let scoped = match event {
            PmLiveUserEvent::Order(order) => {
                self.order_event_count = self.order_event_count.saturating_add(1);
                order.condition() == self.scope.condition()
                    && order.token() == self.scope.token()
                    && order.maker() == Some(self.signer)
            }
            PmLiveUserEvent::Trade(trade) => {
                self.trade_event_count = self.trade_event_count.saturating_add(1);
                trade.condition() == self.scope.condition()
                    && (trade.token() == self.scope.token()
                        || trade.maker_orders().iter().any(|maker| {
                            maker.token() == self.scope.token() && maker.maker() == self.signer
                        }))
            }
        };
        if scoped {
            self.scope_bound_event_count = self.scope_bound_event_count.saturating_add(1);
        } else {
            self.scope_mismatch_count = self.scope_mismatch_count.saturating_add(1);
        }
    }

    fn into_evidence(self) -> PmReadOnlyUserStreamEvidence {
        PmReadOnlyUserStreamEvidence {
            connection_attempt_count: self.attempted_epochs.len() as u64,
            connection_open_count: self.connection_open_count,
            subscription_count: self.subscription_count,
            reconnect_attempt_count: self.reconnect_attempt_count,
            retirement_count: self.retirement_count,
            retry_exhausted_count: self.retry_exhausted_count,
            ping_count: self.ping_count,
            correlated_pong_count: self.correlated_pong_count,
            frame_count: self.frame_count,
            event_count: self.event_count,
            order_event_count: self.order_event_count,
            trade_event_count: self.trade_event_count,
            owner_bound_event_count: self.owner_bound_event_count,
            scope_bound_event_count: self.scope_bound_event_count,
            owner_mismatch_count: 0,
            scope_mismatch_count: self.scope_mismatch_count,
            bound_observation_count: self
                .correlated_pong_count
                .saturating_add(self.scope_bound_event_count),
            dwell_ms: self.dwell_ms,
            shutdown_event_count: self.shutdown_event_count,
            run_completed_without_transport_error: self.run_completed_without_transport_error,
            lifecycle_fingerprint_sha256: String::new(),
        }
    }
}

#[async_trait]
impl PmUserWsEventSink for UserEvidenceSink {
    type Error = Infallible;

    async fn deliver_user_ws_event(&mut self, event: PmUserWsEvent) -> Result<(), Self::Error> {
        match event {
            PmUserWsEvent::ConnectionOpened(observation) => {
                self.observe_epoch(observation.connection().connection_epoch());
                self.connection_open_count = self.connection_open_count.saturating_add(1);
            }
            PmUserWsEvent::SubscriptionSent(observation) => {
                self.observe_epoch(observation.connection().connection_epoch());
                self.subscription_count = self.subscription_count.saturating_add(1);
            }
            PmUserWsEvent::PingSent(observation) => {
                self.observe_epoch(observation.connection().connection_epoch());
                self.ping_count = self.ping_count.saturating_add(1);
            }
            PmUserWsEvent::Pong(observation) => {
                self.observe_epoch(observation.connection().connection_epoch());
                self.correlated_pong_count = self.correlated_pong_count.saturating_add(1);
            }
            PmUserWsEvent::BoundFrame(frame) => {
                self.observe_epoch(frame.observation().connection().connection_epoch());
                self.frame_count = self.frame_count.saturating_add(1);
                for event in frame.events() {
                    self.observe_business_event(event);
                }
            }
            PmUserWsEvent::ConnectionRetired(retired) => {
                self.observe_epoch(retired.observation().connection().connection_epoch());
                self.retirement_count = self.retirement_count.saturating_add(1);
            }
            PmUserWsEvent::ReconnectScheduled(reconnect) => {
                self.observe_epoch(
                    reconnect
                        .retired()
                        .observation()
                        .connection()
                        .connection_epoch(),
                );
                self.reconnect_attempt_count = self.reconnect_attempt_count.saturating_add(1);
            }
            PmUserWsEvent::RetryExhausted(retired) => {
                self.observe_epoch(retired.observation().connection().connection_epoch());
                self.retry_exhausted_count = self.retry_exhausted_count.saturating_add(1);
            }
            PmUserWsEvent::Shutdown(observation) => {
                self.observe_epoch(observation.connection().connection_epoch());
                self.shutdown_event_count = self.shutdown_event_count.saturating_add(1);
            }
        }
        Ok(())
    }
}

fn account_evidence(
    config: &PmReadOnlySmokeConfig,
    collateral: reap_polymarket_wire::PmLiveBalanceAllowance,
    conditional: reap_polymarket_wire::PmLiveBalanceAllowance,
) -> Result<PmReadOnlyAccountEvidence, PmReadOnlySmokeError> {
    let metadata = config
        .expected_metadata()
        .map_err(|_| PmReadOnlySmokeError::Connectivity)?;
    let domain = reap_pm_core::PmGoalFTradingDomain::from_metadata(metadata)
        .map_err(|_| PmReadOnlySmokeError::Connectivity)?;
    let mut allowances = Vec::with_capacity(2);
    for requirement in domain.required_spenders() {
        let (asset_kind, asset_contract, token_id, observed, unscoped_scalar_present) =
            match requirement.asset() {
                PmAssetId::Collateral { contract } => (
                    "collateral",
                    contract,
                    None,
                    &collateral,
                    collateral.unscoped_scalar_present(),
                ),
                PmAssetId::Outcome { contract, token } => (
                    "outcome",
                    contract,
                    Some(token.units().to_string()),
                    &conditional,
                    conditional.unscoped_scalar_present(),
                ),
            };
        let amount = observed.exact_allowance(requirement.spender());
        allowances.push(PmReadOnlyAllowanceEvidence {
            asset_kind: asset_kind.to_owned(),
            asset_contract: asset_contract.to_string(),
            token_id,
            spender_address: requirement.spender().to_string(),
            amount: amount.map_or_else(String::new, |value| value.to_string()),
            present: amount.is_some(),
            unscoped_scalar_present,
        });
    }
    if allowances
        .iter()
        .any(|allowance| !allowance.present || allowance.unscoped_scalar_present)
    {
        return Err(PmReadOnlySmokeError::AuthenticatedRead);
    }
    let lifecycle = metadata.lifecycle();
    Ok(PmReadOnlyAccountEvidence {
        authenticated_response_count: 2,
        collateral_balance: collateral.balance().to_string(),
        outcome_balance: conditional.balance().to_string(),
        position_token_id: metadata.outcome().token().units().to_string(),
        position_balance: conditional.balance().to_string(),
        position_available: lifecycle.active()
            && !lifecycle.closed()
            && !lifecycle.archived()
            && lifecycle.accepting_orders()
            && lifecycle.order_book_enabled(),
        allowances,
        allowance_count: 0,
        canonical_sha256: String::new(),
    })
}

fn reconciliation_evidence(
    config: &PmReadOnlySmokeConfig,
    open_orders: PmCompleteOpenOrdersCut,
    trades: PmCompleteTradesCut,
) -> Result<PmReadOnlyReconciliationEvidence, PmReadOnlySmokeError> {
    let scope = config
        .wire_scope()
        .map_err(|_| PmReadOnlySmokeError::Connectivity)?;
    let signer = config
        .signer()
        .map_err(|_| PmReadOnlySmokeError::Connectivity)?;
    let open_order_page_count = open_orders.pages().len() as u64;
    let open_order_count = open_orders.row_count() as u64;
    let mut order_rows = Vec::with_capacity(open_orders.row_count());
    for order in open_orders.pages().iter().flat_map(|page| page.orders()) {
        if order.condition() == scope.condition()
            && order.token() == scope.token()
            && order.maker() == signer
        {
            order_rows.push(order_evidence(order));
        }
    }
    let order_scope_bound_count = order_rows.len() as u64;

    let trade_page_count = trades.pages().len() as u64;
    let trade_count = trades.row_count() as u64;
    let mut trade_rows = Vec::with_capacity(trades.row_count());
    for trade in trades.pages().iter().flat_map(|page| page.trades()) {
        if trade.condition() == scope.condition()
            && (trade.token() == scope.token()
                || trade
                    .maker_orders()
                    .iter()
                    .any(|maker| maker.token() == scope.token() && maker.maker() == signer))
        {
            trade_rows.push(trade_evidence(trade, scope, signer));
        }
    }
    let trade_scope_bound_count = trade_rows.len() as u64;

    Ok(PmReadOnlyReconciliationEvidence {
        open_order_page_count,
        open_order_terminal_cursor_seen: true,
        open_order_count,
        open_order_owner_bound_count: open_order_count,
        open_order_scope_bound_count: order_scope_bound_count,
        open_order_owner_mismatch_count: 0,
        open_order_scope_mismatch_count: open_order_count.saturating_sub(order_scope_bound_count),
        open_orders_sha256: String::new(),
        open_orders: order_rows,
        trade_page_count,
        trade_terminal_cursor_seen: true,
        trade_count,
        trade_owner_bound_count: trade_count,
        trade_scope_bound_count,
        trade_owner_mismatch_count: 0,
        trade_scope_mismatch_count: trade_count.saturating_sub(trade_scope_bound_count),
        trades_sha256: String::new(),
        trades: trade_rows,
    })
}

fn order_evidence(order: &PmLiveOrder) -> PmReadOnlyOrderEvidence {
    PmReadOnlyOrderEvidence {
        order_id: order.id().to_string(),
        condition_id: order.condition().to_string(),
        token_id: order.token().units().to_string(),
        side: side(order.side()).to_owned(),
        original_size: order.original_size().to_string(),
        size_matched: book_quantity(order.size_matched()),
        price: order.price().to_string(),
        status: order.status().to_owned(),
        maker_address: order.maker().to_string(),
        created_at: order.created_at(),
        expiration: order.expiration(),
        outcome: order.outcome().map(str::to_owned),
        order_type: order.order_type().map(str::to_owned),
    }
}

fn trade_evidence(
    trade: &PmLiveTrade,
    scope: reap_polymarket_wire::PmWireScope,
    signer: EvmAddress,
) -> PmReadOnlyTradeEvidence {
    PmReadOnlyTradeEvidence {
        trade_id: trade.id().to_string(),
        condition_id: trade.condition().to_string(),
        token_id: trade.token().units().to_string(),
        side: side(trade.side()).to_owned(),
        size: trade.size().to_string(),
        price: trade.price().to_string(),
        status: trade.status().to_owned(),
        order_id: trade.order_id().map(|value| value.to_string()),
        taker_order_id: trade.taker_order_id().map(|value| value.to_string()),
        trader_side: trade.trader_side().map(str::to_owned),
        transaction_hash: trade.transaction_hash().map(str::to_owned),
        fee_rate_bps: trade.fee_rate_bps().map(|value| value.to_string()),
        maker_orders: trade
            .maker_orders()
            .iter()
            .filter(|maker| maker.token() == scope.token() && maker.maker() == signer)
            .map(|maker| PmReadOnlyTradeMakerEvidence {
                order_id: maker.order_id().to_string(),
                token_id: maker.token().units().to_string(),
                side: side(maker.side()).to_owned(),
                price: maker.price().to_string(),
                matched_amount: maker.matched_amount().to_string(),
                fee_rate_bps: maker.fee_rate_bps().map(|value| value.to_string()),
                maker_address: maker.maker().to_string(),
            })
            .collect(),
        maker_address: trade.maker().map(|value| value.to_string()),
        timestamp: trade.timestamp(),
        match_time: trade.match_time(),
        last_update: trade.last_update(),
    }
}

const fn side(side: PmOrderSide) -> &'static str {
    match side {
        PmOrderSide::Buy => "buy",
        PmOrderSide::Sell => "sell",
    }
}

fn book_quantity(quantity: PmBookQuantity) -> String {
    match quantity {
        PmBookQuantity::Delete => "0".to_owned(),
        PmBookQuantity::Quantity(value) => value.to_string(),
    }
}

#[allow(clippy::too_many_arguments)]
fn assemble_artifact(
    config: &PmReadOnlySmokeConfig,
    config_evidence: PmReadOnlyConfigEvidence,
    provenance: Provenance,
    started_unix_ms: u64,
    finished_unix_ms: u64,
    metadata: Option<PmReadOnlyMetadataEvidence>,
    account: Option<PmReadOnlyAccountEvidence>,
    reconciliation: Option<PmReadOnlyReconciliationEvidence>,
    user_stream: Option<PmReadOnlyUserStreamEvidence>,
    teardown: PmReadOnlyTeardownEvidence,
    collection_failure: Option<PmReadOnlyCollectionFailureEvidence>,
    draft: bool,
) -> Result<PmReadOnlySmokeArtifact, PmReadOnlySmokeError> {
    if draft {
        debug_assert!(collection_failure.is_none());
        PmReadOnlySmokeArtifact::from_collected_draft(
            provenance.binary_sha256,
            provenance.host_name,
            std::env::consts::OS.to_owned(),
            std::env::consts::ARCH.to_owned(),
            config,
            config_evidence,
            started_unix_ms,
            finished_unix_ms,
            metadata,
            account,
            reconciliation,
            user_stream,
            teardown,
        )
        .map_err(Into::into)
    } else {
        PmReadOnlySmokeArtifact::from_collected(
            provenance.binary_sha256,
            provenance.host_name,
            std::env::consts::OS.to_owned(),
            std::env::consts::ARCH.to_owned(),
            config,
            config_evidence,
            started_unix_ms,
            finished_unix_ms,
            collection_failure,
            metadata,
            account,
            reconciliation,
            user_stream,
            teardown,
        )
        .map_err(Into::into)
    }
}

struct ReservedOutput {
    placeholder: Option<File>,
    placeholder_metadata: std::fs::Metadata,
    parent: File,
    parent_anchor: PathBuf,
    target_anchor: PathBuf,
    committed: bool,
}

impl ReservedOutput {
    fn persist(
        &mut self,
        artifact: &PmReadOnlySmokeArtifact,
        artifact_secret_guard: Option<&PmReadOnlyArtifactSecretGuard>,
    ) -> Result<(), PmReadOnlySmokeError> {
        let mut bytes =
            serde_json::to_vec_pretty(artifact).map_err(PmReadOnlySmokeError::Serialization)?;
        bytes.push(b'\n');
        if let Some(guard) = artifact_secret_guard {
            if let Some(metadata) = artifact.metadata.as_ref() {
                guard.ensure_base64_artifact_value_is_secret_free(&metadata.market_body_base64)?;
                guard.ensure_base64_artifact_value_is_secret_free(&metadata.clob_body_base64)?;
            }
            guard.ensure_artifact_is_secret_free(&bytes)?;
        }
        if bytes.len() as u64 > crate::MAX_PM_READ_ONLY_ARTIFACT_BYTES {
            return Err(PmReadOnlySmokeError::ArtifactTooLarge);
        }
        verify_pm_read_only_smoke_artifact_bytes(&bytes)?;

        let mut staging = tempfile::Builder::new()
            .prefix(".reap-pm-readiness-")
            .tempfile_in(&self.parent_anchor)
            .map_err(PmReadOnlySmokeError::PersistOutput)?;
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt as _;
            staging
                .as_file()
                .set_permissions(std::fs::Permissions::from_mode(0o600))
                .map_err(PmReadOnlySmokeError::PersistOutput)?;
        }
        staging
            .write_all(&bytes)
            .and_then(|()| staging.as_file().sync_all())
            .map_err(PmReadOnlySmokeError::PersistOutput)?;
        let staging_metadata = staging
            .as_file()
            .metadata()
            .map_err(PmReadOnlySmokeError::PersistOutput)?;
        validate_private_artifact_metadata(&staging_metadata)?;

        let current = std::fs::symlink_metadata(&self.target_anchor)
            .map_err(PmReadOnlySmokeError::PersistOutput)?;
        if current.file_type().is_symlink()
            || !same_file_identity(&current, &self.placeholder_metadata)
        {
            return Err(PmReadOnlySmokeError::PersistOutput(std::io::Error::new(
                std::io::ErrorKind::AlreadyExists,
                "the reserved artifact path changed before commit",
            )));
        }

        let persisted = staging
            .persist(&self.target_anchor)
            .map_err(|error| PmReadOnlySmokeError::PersistOutput(error.error))?;
        let persisted_metadata = persisted
            .metadata()
            .map_err(PmReadOnlySmokeError::PersistOutput)?;
        validate_private_artifact_metadata(&persisted_metadata)?;
        if !same_file_identity(&persisted_metadata, &staging_metadata)
            || persisted_metadata.len() != bytes.len() as u64
        {
            return Err(PmReadOnlySmokeError::PersistOutput(std::io::Error::other(
                "the committed artifact does not match its staged file",
            )));
        }
        persisted
            .sync_all()
            .map_err(PmReadOnlySmokeError::PersistOutput)?;
        self.committed = true;
        drop(self.placeholder.take());
        self.parent
            .sync_all()
            .map_err(PmReadOnlySmokeError::PersistOutput)
    }
}

impl Drop for ReservedOutput {
    fn drop(&mut self) {
        if self.committed {
            return;
        }
        drop(self.placeholder.take());
        if std::fs::symlink_metadata(&self.target_anchor)
            .ok()
            .filter(|metadata| {
                !metadata.file_type().is_symlink()
                    && same_file_identity(metadata, &self.placeholder_metadata)
            })
            .is_some()
        {
            let _ = std::fs::remove_file(&self.target_anchor);
            let _ = self.parent.sync_all();
        }
    }
}

fn reserve_private_output(path: &Path) -> Result<ReservedOutput, PmReadOnlySmokeError> {
    let parent = path
        .parent()
        .filter(|value| !value.as_os_str().is_empty())
        .unwrap_or_else(|| Path::new("."));
    let path_metadata =
        std::fs::symlink_metadata(parent).map_err(PmReadOnlySmokeError::ReserveOutput)?;
    if path_metadata.file_type().is_symlink() || !path_metadata.is_dir() {
        return Err(PmReadOnlySmokeError::InvalidOutputParent);
    }
    let mut parent_options = OpenOptions::new();
    parent_options.read(true);
    #[cfg(unix)]
    {
        use std::os::unix::fs::OpenOptionsExt as _;
        parent_options.custom_flags(libc::O_DIRECTORY | libc::O_CLOEXEC | libc::O_NOFOLLOW);
    }
    let parent_file = parent_options
        .open(parent)
        .map_err(PmReadOnlySmokeError::ReserveOutput)?;
    let parent_metadata = parent_file
        .metadata()
        .map_err(PmReadOnlySmokeError::ReserveOutput)?;
    if !same_private_parent(&path_metadata, &parent_metadata)? {
        return Err(PmReadOnlySmokeError::InvalidOutputParent);
    }

    let file_name = path
        .file_name()
        .filter(|value| !value.is_empty())
        .ok_or(PmReadOnlySmokeError::InvalidOutputParent)?;
    #[cfg(target_os = "linux")]
    let parent_anchor = {
        use std::os::fd::AsRawFd as _;
        PathBuf::from("/proc/self/fd").join(parent_file.as_raw_fd().to_string())
    };
    #[cfg(not(target_os = "linux"))]
    let parent_anchor = parent.to_path_buf();
    let target_anchor = parent_anchor.join(file_name);

    let mut options = OpenOptions::new();
    options.write(true).create_new(true);
    #[cfg(unix)]
    {
        use std::os::unix::fs::OpenOptionsExt as _;
        options
            .mode(0o600)
            .custom_flags(libc::O_CLOEXEC | libc::O_NOFOLLOW);
    }
    let file = options
        .open(&target_anchor)
        .map_err(PmReadOnlySmokeError::ReserveOutput)?;
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt as _;
        file.set_permissions(std::fs::Permissions::from_mode(0o600))
            .map_err(PmReadOnlySmokeError::ReserveOutput)?;
        let after_parent = parent_file
            .metadata()
            .map_err(PmReadOnlySmokeError::ReserveOutput)?;
        if !same_private_parent(&parent_metadata, &after_parent)? {
            return Err(PmReadOnlySmokeError::InvalidOutputParent);
        }
    }
    let placeholder_metadata = file
        .metadata()
        .map_err(PmReadOnlySmokeError::ReserveOutput)?;
    validate_private_artifact_metadata(&placeholder_metadata)?;
    Ok(ReservedOutput {
        placeholder: Some(file),
        placeholder_metadata,
        parent: parent_file,
        parent_anchor,
        target_anchor,
        committed: false,
    })
}

fn validate_private_artifact_metadata(
    metadata: &std::fs::Metadata,
) -> Result<(), PmReadOnlySmokeError> {
    if !metadata.is_file() {
        return Err(PmReadOnlySmokeError::InvalidOutputParent);
    }
    #[cfg(unix)]
    {
        use std::os::unix::fs::MetadataExt as _;
        if metadata.mode() & 0o7777 != 0o600
            || metadata.uid() != effective_uid()?
            || metadata.nlink() != 1
        {
            return Err(PmReadOnlySmokeError::InvalidOutputParent);
        }
    }
    Ok(())
}

fn same_private_parent(
    expected: &std::fs::Metadata,
    actual: &std::fs::Metadata,
) -> Result<bool, PmReadOnlySmokeError> {
    if !expected.is_dir() || !actual.is_dir() {
        return Ok(false);
    }
    #[cfg(unix)]
    {
        use std::os::unix::fs::MetadataExt as _;
        let effective_uid = effective_uid()?;
        Ok(expected.dev() == actual.dev()
            && expected.ino() == actual.ino()
            && expected.uid() == effective_uid
            && actual.uid() == effective_uid
            && expected.mode() & 0o7777 == 0o700
            && actual.mode() & 0o7777 == 0o700)
    }
    #[cfg(not(unix))]
    Ok(true)
}

fn same_file_identity(expected: &std::fs::Metadata, actual: &std::fs::Metadata) -> bool {
    #[cfg(unix)]
    {
        use std::os::unix::fs::MetadataExt as _;
        expected.dev() == actual.dev() && expected.ino() == actual.ino()
    }
    #[cfg(not(unix))]
    {
        expected.len() == actual.len()
            && expected.modified().ok() == actual.modified().ok()
            && expected.created().ok() == actual.created().ok()
    }
}

#[cfg(unix)]
fn effective_uid() -> Result<u32, PmReadOnlySmokeError> {
    let status = std::fs::read_to_string("/proc/self/status")
        .map_err(PmReadOnlySmokeError::ReserveOutput)?;
    status
        .lines()
        .find(|line| line.starts_with("Uid:"))
        .and_then(|line| line.split_ascii_whitespace().nth(2))
        .and_then(|value| value.parse().ok())
        .ok_or(PmReadOnlySmokeError::InvalidOutputParent)
}

fn unix_ms() -> Result<u64, PmReadOnlySmokeError> {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map_err(|_| PmReadOnlySmokeError::Clock)?
        .as_millis()
        .try_into()
        .map_err(|_| PmReadOnlySmokeError::Clock)
}

#[cfg(test)]
mod tests;
