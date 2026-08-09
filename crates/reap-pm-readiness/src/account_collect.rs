use std::{
    fs::{File, OpenOptions},
    io::Write as _,
    path::{Path, PathBuf},
    time::{Duration, Instant, SystemTime, UNIX_EPOCH},
};

use reap_pm_core::{PmAssetId, PmGoalFTradingDomain};
use reap_polymarket_live_adapter::{
    PM_CREDENTIAL_AUTHORITY_READ_ONLY_SHUTDOWN_BOUNDS, PmAccountBalanceAllowance,
    PmLiveAdapterError, PmReadOnlyAccountConnectivityOwner, PmReadOnlySignatureType,
};
use reap_telemetry::current_executable_sha256;
use thiserror::Error;

use crate::{
    PmReadOnlyAccountArtifact, PmReadOnlyAccountConfig, PmReadOnlyAccountSnapshotEvidence,
    PmReadOnlyAccountTeardownEvidence, PmReadOnlyAccountVerificationError,
    PmReadOnlyAllowanceEvidence, PmReadOnlyCollectionFailureEvidence, PmReadOnlyCredentialError,
    PmReadOnlySmokeConfigError,
    account_verify::{
        MAX_PM_READ_ONLY_ACCOUNT_ARTIFACT_BYTES, verify_pm_read_only_account_artifact_bytes,
    },
    credentials::PmReadOnlyArtifactSecretGuard,
    load_pm_read_only_account_config_path, load_pm_read_only_credentials,
};

const MAX_ACCOUNT_PHASE_MS: u64 = 5 * 60 * 1_000;
const MAX_ATTEMPT_MS: u64 = 5 * 60 * 1_000;
const MAX_HOST_NAME_BYTES: usize = 255;

#[derive(Debug, Error)]
pub enum PmReadOnlyAccountError {
    #[error(transparent)]
    Config(#[from] PmReadOnlySmokeConfigError),
    #[error(transparent)]
    Credential(#[from] PmReadOnlyCredentialError),
    #[error("failed to reserve the create-new account-only artifact")]
    ReserveOutput(#[source] std::io::Error),
    #[error(
        "the account-only artifact parent must be a real mode-0700 directory owned by this process"
    )]
    InvalidOutputParent,
    #[error("failed to persist the account-only artifact")]
    PersistOutput(#[source] std::io::Error),
    #[error("failed to construct fixed account-only connectivity")]
    Connectivity,
    #[error("system provenance is unavailable")]
    Provenance,
    #[error("system time is unavailable")]
    Clock,
    #[error("account-only evidence serialization failed")]
    Serialization(#[source] serde_json::Error),
    #[error(transparent)]
    Verification(#[from] PmReadOnlyAccountVerificationError),
    #[error("the finalized account-only artifact exceeds its fixed byte bound")]
    ArtifactTooLarge,
}

#[derive(Clone, Copy)]
struct FailureClass {
    stage: &'static str,
    kind: &'static str,
}

impl FailureClass {
    const fn account(kind: &'static str) -> Self {
        Self {
            stage: "authenticated_account",
            kind,
        }
    }

    const fn teardown() -> Self {
        Self {
            stage: "teardown",
            kind: "shutdown",
        }
    }
}

struct Provenance {
    binary_sha256: String,
    host_name: String,
    started_monotonic: Instant,
}

pub async fn collect_pm_read_only_account_path(
    config_path: impl AsRef<Path>,
    credentials_directory: impl AsRef<Path>,
    output_path: impl AsRef<Path>,
) -> Result<PmReadOnlyAccountArtifact, PmReadOnlyAccountError> {
    let (config, config_evidence) = load_pm_read_only_account_config_path(config_path)?;
    let started_monotonic = Instant::now();
    let started_unix_ms = unix_ms()?;
    let provenance = collect_provenance(started_monotonic)?;
    let mut output = reserve_private_output(output_path.as_ref())?;
    let smoke = config.smoke();
    let credentials = load_pm_read_only_credentials(
        credentials_directory.as_ref(),
        &smoke.api_key_file,
        &smoke.secret_file,
        &smoke.passphrase_file,
    )?;
    let (credential_input, secret_guard) = credentials.into_adapter_input_and_artifact_guard();
    secret_guard.ensure_config_is_secret_free(config_evidence.canonical_toml.as_bytes())?;

    let signature_type = PmReadOnlySignatureType::try_from(config.signature_type())
        .map_err(|_| PmReadOnlyAccountError::Connectivity)?;
    let owner = PmReadOnlyAccountConnectivityOwner::production(
        config.signer()?,
        config.funder()?,
        signature_type,
        config.wire_scope()?.token(),
        Duration::from_millis(smoke.connect_timeout_ms),
        Duration::from_millis(smoke.request_timeout_ms),
        credential_input,
    )
    .map_err(|_| PmReadOnlyAccountError::Connectivity)?;
    let roles = owner
        .split()
        .map_err(|_| PmReadOnlyAccountError::Connectivity)?;
    // From the instant a secret-owning authority task exists, cancellation
    // must fail-stop the process unless bounded shutdown returns and proves
    // that the task joined. Dropping the adapter supervisor alone requests an
    // abort but cannot synchronously prove secret destruction.
    let mut credential_fail_stop = CredentialShutdownCancellationFailStop::armed();
    let mut counters = RequestCounters::default();
    let reads = tokio::time::timeout(
        Duration::from_millis(MAX_ACCOUNT_PHASE_MS),
        collect_two_balances(
            &mut counters,
            roles.server_time,
            roles.authenticated_account,
        ),
    )
    .await;
    let teardown_outcome = roles
        .credential_supervisor
        .shutdown_bounded(PM_CREDENTIAL_AUTHORITY_READ_ONLY_SHUTDOWN_BOUNDS)
        .await;
    credential_fail_stop.disarm();
    let teardown = PmReadOnlyAccountTeardownEvidence {
        public_time_attempt_count: counters.public_time,
        authenticated_balance_attempt_count: counters.authenticated_balance,
        private_reconciliation_request_count: 0,
        user_stream_connection_count: 0,
        credential_authority_task_started: true,
        credential_authority_shutdown_requested: teardown_outcome.shutdown_requested(),
        credential_authority_abort_requested: teardown_outcome.abort_requested(),
        credential_authority_task_joined: teardown_outcome.task_joined(),
        credential_authority_task_completed_cleanly: teardown_outcome.task_completed_cleanly(),
        credentials_loaded: true,
        credentials_dropped_before_return: teardown_outcome.credentials_dropped(),
        all_tasks_joined: teardown_outcome.task_joined(),
        mutation_roles_constructed: false,
        mutation_requests: 0,
    };
    let teardown_failure = (!teardown.credential_authority_shutdown_requested
        || teardown.credential_authority_abort_requested
        || !teardown.credential_authority_task_joined
        || !teardown.credential_authority_task_completed_cleanly
        || !teardown.credentials_dropped_before_return)
        .then_some(FailureClass::teardown());
    let (account, read_failure) = match reads {
        Err(_) => (None, Some(FailureClass::account("timeout"))),
        Ok(Err(error)) => (None, Some(classify_live_failure(error))),
        Ok(Ok((collateral, conditional))) => {
            let evidence = account_evidence(&config, collateral, conditional)?;
            let incomplete = evidence
                .allowances
                .iter()
                .any(|allowance| !allowance.present || allowance.unscoped_scalar_present)
                .then_some(FailureClass::account("insufficient_evidence"));
            (Some(evidence), incomplete)
        }
    };
    let failure = teardown_failure.or(read_failure);
    let finished_unix_ms = bounded_finished(started_unix_ms, &provenance)?;
    let collection_failure = failure
        .map(|failure| {
            PmReadOnlyCollectionFailureEvidence::new(failure.stage, failure.kind, finished_unix_ms)
        })
        .transpose()
        .map_err(|_| PmReadOnlyAccountError::Connectivity)?;
    let artifact = PmReadOnlyAccountArtifact::from_collected(
        provenance.binary_sha256,
        provenance.host_name,
        std::env::consts::OS.to_owned(),
        std::env::consts::ARCH.to_owned(),
        &config,
        config_evidence,
        started_unix_ms,
        finished_unix_ms,
        collection_failure,
        account,
        teardown,
    )?;
    output.persist(&artifact, &secret_guard)?;
    Ok(artifact)
}

struct CredentialShutdownCancellationFailStop {
    armed: bool,
}

impl CredentialShutdownCancellationFailStop {
    const fn armed() -> Self {
        Self { armed: true }
    }

    fn disarm(&mut self) {
        self.armed = false;
    }
}

impl Drop for CredentialShutdownCancellationFailStop {
    fn drop(&mut self) {
        if self.armed {
            std::process::abort();
        }
    }
}

#[derive(Default)]
struct RequestCounters {
    public_time: u64,
    authenticated_balance: u64,
}

async fn collect_two_balances(
    counters: &mut RequestCounters,
    server_time: reap_polymarket_live_adapter::PmReadServerTimeHttpRole,
    mut account_owner: reap_polymarket_live_adapter::PmReadOnlyAccountHttpOwner,
) -> Result<(PmAccountBalanceAllowance, PmAccountBalanceAllowance), PmLiveAdapterError> {
    counters.public_time += 1;
    let collateral_time = server_time.fresh_read_server_time().await?;
    counters.authenticated_balance += 1;
    let collateral = account_owner
        .account()
        .collateral_balance_allowance(collateral_time)
        .await?;
    counters.public_time += 1;
    let conditional_time = server_time.fresh_read_server_time().await?;
    counters.authenticated_balance += 1;
    let conditional = account_owner
        .account()
        .conditional_balance_allowance(conditional_time)
        .await?;
    Ok((collateral, conditional))
}

fn account_evidence(
    config: &PmReadOnlyAccountConfig,
    collateral: PmAccountBalanceAllowance,
    conditional: PmAccountBalanceAllowance,
) -> Result<PmReadOnlyAccountSnapshotEvidence, PmReadOnlyAccountError> {
    let collateral = collateral.into_value();
    let conditional = conditional.into_value();
    let domain = PmGoalFTradingDomain::from_metadata(config.expected_metadata()?)
        .map_err(|_| PmReadOnlyAccountError::Connectivity)?;
    let allowances = domain
        .required_spenders()
        .iter()
        .map(|requirement| {
            let (asset_kind, asset_contract, token_id, observed) = match requirement.asset() {
                PmAssetId::Collateral { contract } => ("collateral", contract, None, &collateral),
                PmAssetId::Outcome { contract, token } => (
                    "outcome",
                    contract,
                    Some(token.units().to_string()),
                    &conditional,
                ),
            };
            let amount = observed.exact_allowance(requirement.spender());
            PmReadOnlyAllowanceEvidence {
                asset_kind: asset_kind.to_owned(),
                asset_contract: asset_contract.to_string(),
                token_id,
                spender_address: requirement.spender().to_string(),
                amount: amount.map_or_else(String::new, |value| value.to_string()),
                present: amount.is_some(),
                unscoped_scalar_present: observed.unscoped_scalar_present(),
            }
        })
        .collect();
    Ok(PmReadOnlyAccountSnapshotEvidence {
        authenticated_response_count: 2,
        collateral_balance: collateral.balance().to_string(),
        conditional_balance: conditional.balance().to_string(),
        token_id: config.smoke().token_id.clone(),
        allowances,
        allowance_count: 0,
        canonical_sha256: String::new(),
    })
}

fn classify_live_failure(error: PmLiveAdapterError) -> FailureClass {
    let kind = match error {
        PmLiveAdapterError::RequestTimeout => "timeout",
        PmLiveAdapterError::Redirect { .. } | PmLiveAdapterError::UnexpectedStatus { .. } => {
            "rejected_status"
        }
        PmLiveAdapterError::Wire(_)
        | PmLiveAdapterError::PrivateWire(_)
        | PmLiveAdapterError::ResponseBodyTooLarge { .. } => "malformed_response",
        PmLiveAdapterError::CredentialOwnerMismatch => "owner_mismatch",
        _ => "transport",
    };
    FailureClass::account(kind)
}

fn collect_provenance(started_monotonic: Instant) -> Result<Provenance, PmReadOnlyAccountError> {
    let binary_sha256 =
        current_executable_sha256().map_err(|_| PmReadOnlyAccountError::Provenance)?;
    let host_name = std::fs::read_to_string("/proc/sys/kernel/hostname")
        .map_err(|_| PmReadOnlyAccountError::Provenance)?;
    let host_name = host_name.trim_end_matches(['\r', '\n']).to_owned();
    if host_name.is_empty()
        || host_name.len() > MAX_HOST_NAME_BYTES
        || !host_name.is_ascii()
        || host_name.bytes().any(|byte| byte.is_ascii_control())
    {
        return Err(PmReadOnlyAccountError::Provenance);
    }
    Ok(Provenance {
        binary_sha256,
        host_name,
        started_monotonic,
    })
}

fn bounded_finished(
    started_unix_ms: u64,
    provenance: &Provenance,
) -> Result<u64, PmReadOnlyAccountError> {
    let elapsed = u64::try_from(provenance.started_monotonic.elapsed().as_millis())
        .map_err(|_| PmReadOnlyAccountError::Clock)?;
    if elapsed > MAX_ATTEMPT_MS {
        return Err(PmReadOnlyAccountError::Clock);
    }
    started_unix_ms
        .checked_add(elapsed)
        .ok_or(PmReadOnlyAccountError::Clock)
}

fn unix_ms() -> Result<u64, PmReadOnlyAccountError> {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map_err(|_| PmReadOnlyAccountError::Clock)?
        .as_millis()
        .try_into()
        .map_err(|_| PmReadOnlyAccountError::Clock)
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
        artifact: &PmReadOnlyAccountArtifact,
        guard: &PmReadOnlyArtifactSecretGuard,
    ) -> Result<(), PmReadOnlyAccountError> {
        let mut bytes =
            serde_json::to_vec_pretty(artifact).map_err(PmReadOnlyAccountError::Serialization)?;
        bytes.push(b'\n');
        guard.ensure_artifact_is_secret_free(&bytes)?;
        if bytes.len() as u64 > MAX_PM_READ_ONLY_ACCOUNT_ARTIFACT_BYTES {
            return Err(PmReadOnlyAccountError::ArtifactTooLarge);
        }
        verify_pm_read_only_account_artifact_bytes(&bytes)?;
        let mut staging = tempfile::Builder::new()
            .prefix(".reap-pm-account-")
            .tempfile_in(&self.parent_anchor)
            .map_err(PmReadOnlyAccountError::PersistOutput)?;
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt as _;
            staging
                .as_file()
                .set_permissions(std::fs::Permissions::from_mode(0o600))
                .map_err(PmReadOnlyAccountError::PersistOutput)?;
        }
        staging
            .write_all(&bytes)
            .and_then(|()| staging.as_file().sync_all())
            .map_err(PmReadOnlyAccountError::PersistOutput)?;
        let staging_metadata = staging
            .as_file()
            .metadata()
            .map_err(PmReadOnlyAccountError::PersistOutput)?;
        validate_private_artifact_metadata(&staging_metadata)?;
        let current = std::fs::symlink_metadata(&self.target_anchor)
            .map_err(PmReadOnlyAccountError::PersistOutput)?;
        if current.file_type().is_symlink()
            || !same_file_identity(&current, &self.placeholder_metadata)
        {
            return Err(PmReadOnlyAccountError::PersistOutput(std::io::Error::new(
                std::io::ErrorKind::AlreadyExists,
                "reserved account artifact changed before commit",
            )));
        }
        let persisted = staging
            .persist(&self.target_anchor)
            .map_err(|error| PmReadOnlyAccountError::PersistOutput(error.error))?;
        let persisted_metadata = persisted
            .metadata()
            .map_err(PmReadOnlyAccountError::PersistOutput)?;
        validate_private_artifact_metadata(&persisted_metadata)?;
        if !same_file_identity(&persisted_metadata, &staging_metadata)
            || persisted_metadata.len() != bytes.len() as u64
        {
            return Err(PmReadOnlyAccountError::PersistOutput(
                std::io::Error::other("committed account artifact differs from staging"),
            ));
        }
        persisted
            .sync_all()
            .map_err(PmReadOnlyAccountError::PersistOutput)?;
        self.committed = true;
        drop(self.placeholder.take());
        self.parent
            .sync_all()
            .map_err(PmReadOnlyAccountError::PersistOutput)
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

fn reserve_private_output(path: &Path) -> Result<ReservedOutput, PmReadOnlyAccountError> {
    let parent = path
        .parent()
        .filter(|value| !value.as_os_str().is_empty())
        .unwrap_or_else(|| Path::new("."));
    let path_metadata =
        std::fs::symlink_metadata(parent).map_err(PmReadOnlyAccountError::ReserveOutput)?;
    if path_metadata.file_type().is_symlink() || !path_metadata.is_dir() {
        return Err(PmReadOnlyAccountError::InvalidOutputParent);
    }
    let mut options = OpenOptions::new();
    options.read(true);
    #[cfg(unix)]
    {
        use std::os::unix::fs::OpenOptionsExt as _;
        options.custom_flags(libc::O_DIRECTORY | libc::O_CLOEXEC | libc::O_NOFOLLOW);
    }
    let parent_file = options
        .open(parent)
        .map_err(PmReadOnlyAccountError::ReserveOutput)?;
    let parent_metadata = parent_file
        .metadata()
        .map_err(PmReadOnlyAccountError::ReserveOutput)?;
    if !same_private_parent(&path_metadata, &parent_metadata)? {
        return Err(PmReadOnlyAccountError::InvalidOutputParent);
    }
    let file_name = path
        .file_name()
        .filter(|value| !value.is_empty())
        .ok_or(PmReadOnlyAccountError::InvalidOutputParent)?;
    #[cfg(target_os = "linux")]
    let parent_anchor = {
        use std::os::fd::AsRawFd as _;
        PathBuf::from("/proc/self/fd").join(parent_file.as_raw_fd().to_string())
    };
    #[cfg(not(target_os = "linux"))]
    let parent_anchor = parent.to_path_buf();
    let target_anchor = parent_anchor.join(file_name);
    let mut file_options = OpenOptions::new();
    file_options.write(true).create_new(true);
    #[cfg(unix)]
    {
        use std::os::unix::fs::OpenOptionsExt as _;
        file_options
            .mode(0o600)
            .custom_flags(libc::O_CLOEXEC | libc::O_NOFOLLOW);
    }
    let file = file_options
        .open(&target_anchor)
        .map_err(PmReadOnlyAccountError::ReserveOutput)?;
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt as _;
        file.set_permissions(std::fs::Permissions::from_mode(0o600))
            .map_err(PmReadOnlyAccountError::ReserveOutput)?;
    }
    let after_parent = parent_file
        .metadata()
        .map_err(PmReadOnlyAccountError::ReserveOutput)?;
    if !same_private_parent(&parent_metadata, &after_parent)? {
        return Err(PmReadOnlyAccountError::InvalidOutputParent);
    }
    let placeholder_metadata = file
        .metadata()
        .map_err(PmReadOnlyAccountError::ReserveOutput)?;
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
) -> Result<(), PmReadOnlyAccountError> {
    if !metadata.is_file() {
        return Err(PmReadOnlyAccountError::InvalidOutputParent);
    }
    #[cfg(unix)]
    {
        use std::os::unix::fs::MetadataExt as _;
        if metadata.mode() & 0o7777 != 0o600
            || metadata.uid() != effective_uid()?
            || metadata.nlink() != 1
        {
            return Err(PmReadOnlyAccountError::InvalidOutputParent);
        }
    }
    Ok(())
}

fn same_private_parent(
    expected: &std::fs::Metadata,
    actual: &std::fs::Metadata,
) -> Result<bool, PmReadOnlyAccountError> {
    if !expected.is_dir() || !actual.is_dir() {
        return Ok(false);
    }
    #[cfg(unix)]
    {
        use std::os::unix::fs::MetadataExt as _;
        let uid = effective_uid()?;
        Ok(expected.dev() == actual.dev()
            && expected.ino() == actual.ino()
            && expected.uid() == uid
            && actual.uid() == uid
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
fn effective_uid() -> Result<u32, PmReadOnlyAccountError> {
    std::fs::read_to_string("/proc/self/status")
        .map_err(PmReadOnlyAccountError::ReserveOutput)?
        .lines()
        .find(|line| line.starts_with("Uid:"))
        .and_then(|line| line.split_ascii_whitespace().nth(2))
        .and_then(|value| value.parse().ok())
        .ok_or(PmReadOnlyAccountError::InvalidOutputParent)
}

#[cfg(test)]
mod tests {
    use std::{
        collections::BTreeMap,
        sync::{Arc, Mutex},
        time::Duration,
    };

    use reap_pm_core::{EvmAddress, PmTokenId, U256};
    use reap_polymarket_live_adapter::{
        PM_CREDENTIAL_AUTHORITY_READ_ONLY_SHUTDOWN_BOUNDS, PmReadOnlyAccountConnectivityOwner,
        PmReadOnlyCredentialInput, PmReadOnlySignatureType,
    };
    use tokio::{
        io::{AsyncReadExt as _, AsyncWriteExt as _},
        net::{TcpListener, TcpStream},
        time::timeout,
    };

    use super::{RequestCounters, collect_two_balances};

    const SIGNER: &str = "0xf39fd6e51aad88f6f4ce6ab8827279cfffb92266";
    const FUNDER: &str = "0x1000000000000000000000000000000000000001";
    const API_KEY: &str = "00000000-0000-4000-8000-000000000001";
    const API_SECRET: &str = "AAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAA=";
    const PASSPHRASE: &str = "account-e2e-passphrase";
    const SPENDER: &str = "0xe111180000d2663c0091e4f400237545b87b996b";

    async fn read_request(stream: &mut TcpStream) -> String {
        let mut bytes = Vec::new();
        let mut chunk = [0_u8; 2048];
        loop {
            let read = timeout(Duration::from_secs(2), stream.read(&mut chunk))
                .await
                .unwrap()
                .unwrap();
            assert!(read > 0);
            bytes.extend_from_slice(&chunk[..read]);
            assert!(bytes.len() <= 64 * 1024);
            if bytes.windows(4).any(|window| window == b"\r\n\r\n") {
                return String::from_utf8(bytes).unwrap();
            }
        }
    }

    fn target(request: &str) -> String {
        request
            .lines()
            .next()
            .unwrap()
            .split_ascii_whitespace()
            .nth(1)
            .unwrap()
            .to_owned()
    }

    #[tokio::test]
    async fn proxy_account_slice_uses_only_two_times_and_two_typed_balance_gets() {
        let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
        let origin = format!("http://{}", listener.local_addr().unwrap());
        let requests = Arc::new(Mutex::new(Vec::new()));
        let captured = Arc::clone(&requests);
        let server = tokio::spawn(async move {
            for _ in 0..4 {
                let (mut stream, _) = listener.accept().await.unwrap();
                let request = read_request(&mut stream).await;
                let request_target = target(&request);
                captured.lock().unwrap().push(request);
                let body = if request_target == "/time" {
                    "1800000000".to_owned()
                } else {
                    format!(r#"{{"balance":"100","allowances":{{"{SPENDER}":"42"}}}}"#)
                };
                let response = format!(
                    "HTTP/1.1 200 OK\r\ncontent-type: application/json\r\ncontent-length: {}\r\nconnection: close\r\n\r\n{body}",
                    body.len()
                );
                stream.write_all(response.as_bytes()).await.unwrap();
            }
        });
        let owner = PmReadOnlyAccountConnectivityOwner::read_only_evidence(
            EvmAddress::parse(SIGNER).unwrap(),
            EvmAddress::parse(FUNDER).unwrap(),
            PmReadOnlySignatureType::Proxy,
            PmTokenId::new(U256::from_u64(123)).unwrap(),
            &origin,
            Duration::from_secs(2),
            Duration::from_secs(3),
            PmReadOnlyCredentialInput::new(API_KEY.into(), API_SECRET.into(), PASSPHRASE.into()),
        )
        .unwrap();
        let roles = owner.split().unwrap();
        let mut counters = RequestCounters::default();
        let (collateral, conditional) = collect_two_balances(
            &mut counters,
            roles.server_time,
            roles.authenticated_account,
        )
        .await
        .unwrap();
        assert_eq!(collateral.value().balance(), U256::from_u64(100));
        assert_eq!(conditional.value().balance(), U256::from_u64(100));
        assert_eq!(counters.public_time, 2);
        assert_eq!(counters.authenticated_balance, 2);
        let shutdown = roles
            .credential_supervisor
            .shutdown_bounded(PM_CREDENTIAL_AUTHORITY_READ_ONLY_SHUTDOWN_BOUNDS)
            .await;
        assert!(shutdown.task_joined() && shutdown.credentials_dropped());
        server.await.unwrap();

        let routes = requests
            .lock()
            .unwrap()
            .iter()
            .map(|request| target(request))
            .fold(BTreeMap::new(), |mut counts, route| {
                *counts.entry(route).or_insert(0_usize) += 1;
                counts
            });
        assert_eq!(
            routes,
            BTreeMap::from([
                ("/time".to_owned(), 2),
                (
                    "/balance-allowance?asset_type=COLLATERAL&signature_type=1".to_owned(),
                    1,
                ),
                (
                    "/balance-allowance?asset_type=CONDITIONAL&token_id=123&signature_type=1"
                        .to_owned(),
                    1,
                ),
            ])
        );
        for request in requests.lock().unwrap().iter() {
            assert!(request.starts_with("GET "));
            let authenticated = target(request).starts_with("/balance-allowance?");
            assert_eq!(
                request.to_ascii_lowercase().contains("\r\npoly_address:"),
                authenticated
            );
        }
    }
}
