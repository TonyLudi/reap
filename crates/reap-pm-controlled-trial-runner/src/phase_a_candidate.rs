//! Offline Phase-A release-input freeze, static-V3, and V4 gap report.
//!
//! This module loads only non-secret reviewed artifacts, verifies the existing
//! nine-holder static V3 conjunction and exact ten-holder Phase-A V4 envelope,
//! hashes local release inputs, and observes fixed Linux host facts. It does
//! not read credential files, sample a wall clock or randomness, open a
//! journal, construct authentication material, invoke NSS, use a Polymarket or
//! other network transport, or mint a mutation capability. Its only result is
//! an explicitly non-authorizing, always-DENIED report.

use std::{
    fs::{self, File, Metadata},
    io::{Read, Seek, SeekFrom},
    os::unix::fs::MetadataExt as _,
    path::{Path, PathBuf},
    process::{Child, Command, Stdio},
    sync::mpsc,
    thread,
    time::{Duration, Instant},
};

use reap_pm_controlled_trial::{
    OfflineAuthorizationState, ReviewedPhaseAEligibilityEnvelopeContextV4,
    ReviewedPhaseAEligibilityEnvelopeVerificationV4, ReviewedStaticOnlineAuthorizationContextV3,
    ReviewedStaticOnlineAuthorizationVerificationV3, load_canonical_authorization,
    load_canonical_fresh_credential_delivery_binding_v1, load_canonical_online_authorization_v2,
    load_canonical_online_policy_v2, load_canonical_reviewed_fresh_credential_slot_locator_v1,
    load_canonical_reviewed_phase_a_eligibility_envelope_v4,
    load_canonical_reviewed_production_destination_profile_v1,
    load_canonical_reviewed_remote_credential_proof_policy_v1,
    load_canonical_reviewed_signer_proxy_account_identity_v1,
    load_canonical_reviewed_static_online_authorization_v3, load_canonical_trial_config,
    verify_reviewed_phase_a_eligibility_envelope_v4,
    verify_reviewed_static_online_authorization_v3,
};
use serde::Serialize;
use serde_json::Value;
use sha2::{Digest as _, Sha256};
use thiserror::Error;

const REPORT_SCHEMA_VERSION: u32 = 1;
const REPORT_KIND: &str = "phase_a_non_authorizing_candidate_gap_report_v1";
const GIT_PATH: &str = "/usr/bin/git";
const BOOT_ID_PATH: &str = "/proc/sys/kernel/random/boot_id";
const RUNNING_EXECUTABLE_PATH: &str = "/proc/self/exe";
const NETWORK_NAMESPACE_PATH: &str = "/proc/thread-self/ns/net";
const PROC_ROOT_PATH: &str = "/proc";
const PROC_BOOT_ID_ENTRY: &str = "sys/kernel/random/boot_id";
const PROC_THREAD_NET_NAMESPACE_ENTRY: &str = "thread-self/ns/net";
const PROC_SELF_EXECUTABLE_ENTRY: &str = "self/exe";
#[cfg(target_os = "linux")]
const NSFS_MAGIC: rustix::fs::FsWord = 0x6e73_6673;
const MAX_GIT_OUTPUT_BYTES: usize = 1024 * 1024;
const MAX_CARGO_LOCK_BYTES: u64 = 32 * 1024 * 1024;
const MAX_SOURCE_MANIFEST_BYTES: u64 = 32 * 1024 * 1024;
const MAX_RUNBOOK_BYTES: u64 = 32 * 1024 * 1024;
const MAX_RELEASE_BINARY_BYTES: u64 = 1024 * 1024 * 1024;
const MAX_LOCAL_HELPER_DURATION: Duration = Duration::from_secs(10);

#[derive(Debug)]
pub(crate) struct FreezePhaseACandidatePaths {
    pub(crate) repository_root: PathBuf,
    pub(crate) config: PathBuf,
    pub(crate) authorization: PathBuf,
    pub(crate) online_policy_v2: PathBuf,
    pub(crate) online_authorization_v2: PathBuf,
    pub(crate) reviewed_production_destination_v1: PathBuf,
    pub(crate) reviewed_fresh_credential_slot_locator_v1: PathBuf,
    pub(crate) fresh_credential_delivery_binding_v1: PathBuf,
    pub(crate) reviewed_signer_proxy_account_identity_v1: PathBuf,
    pub(crate) reviewed_remote_credential_proof_policy_v1: PathBuf,
    pub(crate) reviewed_static_online_authorization_v3: PathBuf,
    pub(crate) reviewed_phase_a_eligibility_envelope_v4: PathBuf,
    pub(crate) source_manifest: PathBuf,
    pub(crate) runbook: PathBuf,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub(crate) struct PhaseANonAuthorizingCandidateGapReportV1 {
    schema_version: u32,
    record_kind: &'static str,
    candidate_only_non_authorizing: bool,
    exact_nine_holder_static_v3_conjunction_verified: bool,
    exact_ten_holder_phase_a_v4_envelope_verified: bool,
    live_authorization_record_generated: bool,
    frozen_local_inputs: FrozenLocalInputs,
    local_host_facts: LocalHostFacts,
    current_binding_checks: CurrentBindingChecks,
    gap_summary: GapSummary,
    static_v3_false_boolean_paths_exhaustive: Vec<String>,
    phase_a_v4_false_boolean_paths_exhaustive: Vec<String>,
    static_v3_verification: ReviewedStaticOnlineAuthorizationVerificationV3,
    phase_a_v4_verification: ReviewedPhaseAEligibilityEnvelopeVerificationV4,
    #[serde(flatten)]
    authorization: OfflineAuthorizationState,
}

impl PhaseANonAuthorizingCandidateGapReportV1 {
    /// Bind a later request wrapper to the exact config and V4 holder that
    /// this independently frozen candidate verified. V4 in turn pins the
    /// complete ten-holder chain, so no caller-supplied equality DTO exists.
    pub(crate) fn exact_request_chain_matches(
        &self,
        canonical_config_sha256: &str,
        canonical_config_length: u64,
        phase_a_v4_sha256: &str,
        phase_a_v4_length: u64,
        phase_a_v4_fingerprint: &str,
    ) -> bool {
        self.frozen_local_inputs.canonical_config.sha256 == canonical_config_sha256
            && self.frozen_local_inputs.canonical_config.length == canonical_config_length
            && self
                .frozen_local_inputs
                .reviewed_phase_a_eligibility_envelope_v4
                .canonical_sha256
                == phase_a_v4_sha256
            && self
                .frozen_local_inputs
                .reviewed_phase_a_eligibility_envelope_v4
                .canonical_length
                == phase_a_v4_length
            && self
                .frozen_local_inputs
                .reviewed_phase_a_eligibility_envelope_v4
                .fingerprint
                == phase_a_v4_fingerprint
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
struct FrozenLocalInputs {
    repository_commit: String,
    git_worktree_clean_observed_before_and_after: bool,
    cargo_lock: ArtifactDigest,
    running_release_binary: ArtifactDigest,
    running_release_binary_source: &'static str,
    canonical_config: ArtifactDigest,
    source_manifest: ArtifactDigest,
    runbook_revision: String,
    runbook: ArtifactDigest,
    reviewed_static_online_authorization_v3: ArtifactDigest,
    reviewed_phase_a_eligibility_envelope_v4: CanonicalReviewedArtifactDigest,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
struct ArtifactDigest {
    sha256: String,
    length: u64,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
struct CanonicalReviewedArtifactDigest {
    canonical_sha256: String,
    canonical_length: u64,
    fingerprint: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
struct LocalHostFacts {
    uts_nodename: String,
    boot_id: String,
    linux_euid: u32,
    network_namespace_device: u64,
    network_namespace_inode: u64,
    fixed_sources: LocalHostFactSources,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
struct LocalHostFactSources {
    uts_nodename: &'static str,
    boot_id: &'static str,
    linux_euid: &'static str,
    network_namespace: &'static str,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
struct CurrentBindingChecks {
    repository_commit_matches_v1_and_v2: bool,
    cargo_lock_matches_v1_and_v2: bool,
    running_release_binary_matches_v1_and_v2: bool,
    source_manifest_sha256_matches_v1_config: bool,
    runbook_sha256_matches_v1_config: bool,
    local_uts_nodename_matches_v1_and_v2: bool,
    local_boot_id_matches_v1_and_v2: bool,
    local_linux_euid_matches_v2: bool,
    local_network_namespace_identity_matches_v2: bool,
    runtime_nss_username_checked: bool,
    complete_v1_egress_identity_checked: bool,
    complete_v2_egress_identity_checked: bool,
    current_public_egress_identity_checked: bool,
    all_observed_offline_subset_bindings_match: bool,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
struct GapSummary {
    exact_live_economic_and_time_authorization_complete_and_current_for_this_runner: bool,
    authenticated_credential_provider_trust_root_selected_for_this_runner: bool,
    authenticated_exclusive_delivery_lease_complete_for_this_runner: bool,
    authoritative_remote_credential_acceptance_complete_for_this_runner: bool,
    authoritative_signer_proxy_control_complete_for_this_runner: bool,
    current_egress_and_live_preflight_evidence_complete_for_this_runner: bool,
    selected_place_cancel_actor_complete_for_this_runner: bool,
    durable_v3_attempt_commitment_burn_and_no_resend_complete_for_this_runner: bool,
    recovery_only_exact_cancel_continuation_complete_for_this_runner: bool,
    production_mutation_authority_minted_for_this_runner: bool,
}

#[derive(Debug, Clone, PartialEq, Eq)]
struct GitSnapshot {
    repository_commit: String,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
struct StableFileIdentity {
    device: u64,
    inode: u64,
    mode: u32,
    length: u64,
    modified_seconds: i64,
    modified_nanoseconds: i64,
    changed_seconds: i64,
    changed_nanoseconds: i64,
}

impl StableFileIdentity {
    fn from_metadata(metadata: &Metadata) -> Self {
        Self {
            device: metadata.dev(),
            inode: metadata.ino(),
            mode: metadata.mode(),
            length: metadata.len(),
            modified_seconds: metadata.mtime(),
            modified_nanoseconds: metadata.mtime_nsec(),
            changed_seconds: metadata.ctime(),
            changed_nanoseconds: metadata.ctime_nsec(),
        }
    }
}

pub(crate) fn freeze_phase_a_candidate(
    paths: FreezePhaseACandidatePaths,
) -> Result<PhaseANonAuthorizingCandidateGapReportV1, FreezePhaseACandidateError> {
    let repository_root = fs::canonicalize(&paths.repository_root)
        .map_err(|_| FreezePhaseACandidateError::RepositoryUnavailable)?;
    if !repository_root
        .metadata()
        .is_ok_and(|metadata| metadata.is_dir())
    {
        return Err(FreezePhaseACandidateError::RepositoryUnavailable);
    }
    let git_before = observe_clean_git(&repository_root)?;

    let config = load_canonical_trial_config(&paths.config)
        .map_err(|_| FreezePhaseACandidateError::InvalidReviewedInput("V1 config"))?;
    let authorization = load_canonical_authorization(&paths.authorization)
        .map_err(|_| FreezePhaseACandidateError::InvalidReviewedInput("V1 authorization"))?;
    let online_policy = load_canonical_online_policy_v2(&paths.online_policy_v2)
        .map_err(|_| FreezePhaseACandidateError::InvalidReviewedInput("online policy V2"))?;
    let online_authorization =
        load_canonical_online_authorization_v2(&paths.online_authorization_v2).map_err(|_| {
            FreezePhaseACandidateError::InvalidReviewedInput("online authorization V2")
        })?;
    let destination = load_canonical_reviewed_production_destination_profile_v1(
        &paths.reviewed_production_destination_v1,
    )
    .map_err(|_| {
        FreezePhaseACandidateError::InvalidReviewedInput("reviewed production destination V1")
    })?;
    let locator = load_canonical_reviewed_fresh_credential_slot_locator_v1(
        &paths.reviewed_fresh_credential_slot_locator_v1,
    )
    .map_err(|_| {
        FreezePhaseACandidateError::InvalidReviewedInput("reviewed credential locator V1")
    })?;
    let delivery = load_canonical_fresh_credential_delivery_binding_v1(
        &paths.fresh_credential_delivery_binding_v1,
    )
    .map_err(|_| {
        FreezePhaseACandidateError::InvalidReviewedInput("credential delivery binding V1")
    })?;
    let account_identity = load_canonical_reviewed_signer_proxy_account_identity_v1(
        &paths.reviewed_signer_proxy_account_identity_v1,
    )
    .map_err(|_| {
        FreezePhaseACandidateError::InvalidReviewedInput("reviewed signer/proxy identity V1")
    })?;
    let remote_proof_policy = load_canonical_reviewed_remote_credential_proof_policy_v1(
        &paths.reviewed_remote_credential_proof_policy_v1,
    )
    .map_err(|_| {
        FreezePhaseACandidateError::InvalidReviewedInput("reviewed remote proof policy V1")
    })?;
    let static_authorization = load_canonical_reviewed_static_online_authorization_v3(
        &paths.reviewed_static_online_authorization_v3,
    )
    .map_err(|_| {
        FreezePhaseACandidateError::InvalidReviewedInput("reviewed static authorization V3")
    })?;
    let phase_a_v4 = load_canonical_reviewed_phase_a_eligibility_envelope_v4(
        &paths.reviewed_phase_a_eligibility_envelope_v4,
    )
    .map_err(|_| {
        FreezePhaseACandidateError::InvalidReviewedInput("reviewed Phase-A eligibility envelope V4")
    })?;

    let static_v3_context = ReviewedStaticOnlineAuthorizationContextV3 {
        v1_config: &config,
        v1_authorization: &authorization,
        online_policy_v2: &online_policy,
        online_authorization_v2: &online_authorization,
        reviewed_production_destination_v1: &destination,
        reviewed_fresh_credential_slot_locator_v1: &locator,
        fresh_credential_delivery_binding_v1: &delivery,
        reviewed_signer_proxy_account_identity_v1: &account_identity,
        reviewed_remote_credential_proof_policy_v1: &remote_proof_policy,
    };
    let static_verification =
        verify_reviewed_static_online_authorization_v3(&static_v3_context, &static_authorization)
            .map_err(|_| FreezePhaseACandidateError::StaticV3ConjunctionInvalid)?;
    if static_verification.authorization != OfflineAuthorizationState::DENIED
        || static_verification.authorization.place_dispatch_allowance != 0
    {
        return Err(FreezePhaseACandidateError::StaticV3ConjunctionInvalid);
    }
    let phase_a_v4_context = ReviewedPhaseAEligibilityEnvelopeContextV4 {
        v1_config: &config,
        v1_authorization: &authorization,
        online_policy_v2: &online_policy,
        online_authorization_v2: &online_authorization,
        reviewed_production_destination_v1: &destination,
        reviewed_fresh_credential_slot_locator_v1: &locator,
        fresh_credential_delivery_binding_v1: &delivery,
        reviewed_signer_proxy_account_identity_v1: &account_identity,
        reviewed_remote_credential_proof_policy_v1: &remote_proof_policy,
        reviewed_static_online_authorization_v3: &static_authorization,
    };
    let phase_a_v4_verification =
        verify_reviewed_phase_a_eligibility_envelope_v4(&phase_a_v4_context, &phase_a_v4)
            .map_err(|_| FreezePhaseACandidateError::PhaseAV4EnvelopeInvalid)?;
    if phase_a_v4_verification.authorization != OfflineAuthorizationState::DENIED
        || phase_a_v4_verification
            .authorization
            .place_dispatch_allowance
            != 0
    {
        return Err(FreezePhaseACandidateError::PhaseAV4EnvelopeInvalid);
    }

    let cargo_lock = hash_stable_regular_file(
        &repository_root.join("Cargo.lock"),
        MAX_CARGO_LOCK_BYTES,
        "Cargo.lock",
    )?;
    let running_release_binary = hash_running_executable()?;
    if running_release_binary.length == 0 {
        return Err(FreezePhaseACandidateError::InvalidArtifact(
            "running release binary",
        ));
    }
    let source_manifest = hash_stable_regular_file(
        &paths.source_manifest,
        MAX_SOURCE_MANIFEST_BYTES,
        "source manifest",
    )?;
    let runbook = hash_stable_regular_file(&paths.runbook, MAX_RUNBOOK_BYTES, "trial runbook")?;
    let local_host = observe_local_host_facts()?;

    let v1 = authorization.value();
    let v2 = online_authorization.value();
    let config_value = config.value();
    let repository_commit_matches_v1_and_v2 = git_before.repository_commit
        == v1.build.repository_commit
        && git_before.repository_commit == v2.build.repository_commit;
    let cargo_lock_matches_v1_and_v2 = cargo_lock.sha256 == v1.build.cargo_lock_sha256
        && cargo_lock.sha256 == v2.build.cargo_lock_sha256;
    let running_release_binary_matches_v1_and_v2 = running_release_binary.sha256
        == v1.build.release_binary_sha256
        && running_release_binary.length == v1.build.release_binary_length
        && running_release_binary.sha256 == v2.build.release_binary_sha256
        && running_release_binary.length == v2.build.release_binary_length;
    let source_manifest_sha256_matches_v1_config =
        source_manifest.sha256 == config_value.source_pin_manifest_sha256;
    let runbook_sha256_matches_v1_config = runbook.sha256 == config_value.runbook_sha256;
    let local_uts_nodename_matches_v1_and_v2 = local_host.uts_nodename == v1.host.host_identity
        && local_host.uts_nodename == v2.host.uts_nodename;
    let local_boot_id_matches_v1_and_v2 =
        local_host.boot_id == v1.host.boot_identity && local_host.boot_id == v2.host.boot_id;
    let local_linux_euid_matches_v2 = local_host.linux_euid == v2.host.linux_euid;
    let local_network_namespace_identity_matches_v2 = local_host.network_namespace_device
        == v2.host.egress.network_namespace_device
        && local_host.network_namespace_inode == v2.host.egress.network_namespace_inode;
    // Runtime NSS identity and the full interface/source/gateway/public-egress
    // tuple are deliberately not observed by this no-network command. They
    // remain later runtime/preflight gates even when this local subset matches.
    let runtime_nss_username_checked = false;
    let complete_v1_egress_identity_checked = false;
    let complete_v2_egress_identity_checked = false;
    let current_public_egress_identity_checked = false;
    let all_observed_offline_subset_bindings_match = repository_commit_matches_v1_and_v2
        && cargo_lock_matches_v1_and_v2
        && running_release_binary_matches_v1_and_v2
        && source_manifest_sha256_matches_v1_config
        && runbook_sha256_matches_v1_config
        && local_uts_nodename_matches_v1_and_v2
        && local_boot_id_matches_v1_and_v2
        && local_linux_euid_matches_v2
        && local_network_namespace_identity_matches_v2;

    let static_v3_false_boolean_paths_exhaustive = false_boolean_paths(&static_verification)?;
    let phase_a_v4_false_boolean_paths_exhaustive = false_boolean_paths(&phase_a_v4_verification)?;
    let git_after = observe_clean_git(&repository_root)?;
    if git_before != git_after {
        return Err(FreezePhaseACandidateError::RepositoryChanged);
    }

    Ok(PhaseANonAuthorizingCandidateGapReportV1 {
        schema_version: REPORT_SCHEMA_VERSION,
        record_kind: REPORT_KIND,
        candidate_only_non_authorizing: true,
        exact_nine_holder_static_v3_conjunction_verified: true,
        exact_ten_holder_phase_a_v4_envelope_verified: true,
        live_authorization_record_generated: false,
        frozen_local_inputs: FrozenLocalInputs {
            repository_commit: git_before.repository_commit,
            git_worktree_clean_observed_before_and_after: true,
            cargo_lock,
            running_release_binary,
            running_release_binary_source: RUNNING_EXECUTABLE_PATH,
            canonical_config: ArtifactDigest {
                sha256: config.canonical_sha256().to_owned(),
                length: config.canonical_length(),
            },
            source_manifest,
            runbook_revision: config_value.runbook_revision.clone(),
            runbook,
            reviewed_static_online_authorization_v3: ArtifactDigest {
                sha256: static_authorization.canonical_sha256().to_owned(),
                length: static_authorization.canonical_length(),
            },
            reviewed_phase_a_eligibility_envelope_v4: CanonicalReviewedArtifactDigest {
                canonical_sha256: phase_a_v4.canonical_sha256().to_owned(),
                canonical_length: phase_a_v4.canonical_length(),
                fingerprint: phase_a_v4.fingerprint().to_owned(),
            },
        },
        local_host_facts: local_host,
        current_binding_checks: CurrentBindingChecks {
            repository_commit_matches_v1_and_v2,
            cargo_lock_matches_v1_and_v2,
            running_release_binary_matches_v1_and_v2,
            source_manifest_sha256_matches_v1_config,
            runbook_sha256_matches_v1_config,
            local_uts_nodename_matches_v1_and_v2,
            local_boot_id_matches_v1_and_v2,
            local_linux_euid_matches_v2,
            local_network_namespace_identity_matches_v2,
            runtime_nss_username_checked,
            complete_v1_egress_identity_checked,
            complete_v2_egress_identity_checked,
            current_public_egress_identity_checked,
            all_observed_offline_subset_bindings_match,
        },
        gap_summary: GapSummary {
            exact_live_economic_and_time_authorization_complete_and_current_for_this_runner: false,
            authenticated_credential_provider_trust_root_selected_for_this_runner: false,
            authenticated_exclusive_delivery_lease_complete_for_this_runner: false,
            authoritative_remote_credential_acceptance_complete_for_this_runner: false,
            authoritative_signer_proxy_control_complete_for_this_runner: false,
            current_egress_and_live_preflight_evidence_complete_for_this_runner: false,
            selected_place_cancel_actor_complete_for_this_runner: false,
            durable_v3_attempt_commitment_burn_and_no_resend_complete_for_this_runner: false,
            recovery_only_exact_cancel_continuation_complete_for_this_runner: false,
            production_mutation_authority_minted_for_this_runner: false,
        },
        static_v3_false_boolean_paths_exhaustive,
        phase_a_v4_false_boolean_paths_exhaustive,
        static_v3_verification: static_verification,
        phase_a_v4_verification,
        authorization: OfflineAuthorizationState::DENIED,
    })
}

fn observe_clean_git(repository_root: &Path) -> Result<GitSnapshot, FreezePhaseACandidateError> {
    reject_git_external_process_configuration(repository_root)?;
    if fs::symlink_metadata(repository_root.join(".gitmodules")).is_ok() {
        return Err(FreezePhaseACandidateError::UnsupportedRepositoryFeature);
    }
    let reported_root = run_git(repository_root, &["rev-parse", "--show-toplevel"])?;
    let reported_root = one_line(&reported_root)
        .ok_or(FreezePhaseACandidateError::RepositoryUnavailable)
        .and_then(|value| {
            fs::canonicalize(value).map_err(|_| FreezePhaseACandidateError::RepositoryUnavailable)
        })?;
    if reported_root != repository_root {
        return Err(FreezePhaseACandidateError::RepositoryUnavailable);
    }
    let commit = run_git(repository_root, &["rev-parse", "--verify", "HEAD"])?;
    let repository_commit = one_line(&commit)
        .filter(|value| {
            value.len() == 40
                && value
                    .bytes()
                    .all(|byte| byte.is_ascii_digit() || (b'a'..=b'f').contains(&byte))
        })
        .ok_or(FreezePhaseACandidateError::RepositoryUnavailable)?
        .to_owned();
    let status = run_git(
        repository_root,
        &[
            "status",
            "--porcelain=v1",
            "--untracked-files=all",
            "--ignore-submodules=none",
        ],
    )?;
    if !status.is_empty() {
        return Err(FreezePhaseACandidateError::RepositoryDirty);
    }
    Ok(GitSnapshot { repository_commit })
}

fn reject_git_external_process_configuration(
    repository_root: &Path,
) -> Result<(), FreezePhaseACandidateError> {
    let keys = run_git(
        repository_root,
        &[
            "config",
            "--local",
            "--no-includes",
            "--name-only",
            "--null",
            "--list",
        ],
    )?;
    for key in keys.split(|byte| *byte == 0).filter(|key| !key.is_empty()) {
        let key = std::str::from_utf8(key)
            .map_err(|_| FreezePhaseACandidateError::UnsupportedRepositoryFeature)?
            .to_ascii_lowercase();
        if key.starts_with("filter.")
            || key.starts_with("include.")
            || key.starts_with("includeif.")
            || key == "extensions.worktreeconfig"
            || key == "extensions.partialclone"
            || key == "core.fsmonitor"
            || key == "core.hookspath"
            || (key.starts_with("remote.") && key.ends_with(".promisor"))
            || (key.starts_with("submodule.") && key.ends_with(".update"))
        {
            return Err(FreezePhaseACandidateError::UnsupportedRepositoryFeature);
        }
    }
    Ok(())
}

fn run_git(
    repository_root: &Path,
    arguments: &[&str],
) -> Result<Vec<u8>, FreezePhaseACandidateError> {
    let mut command = Command::new(GIT_PATH);
    command
        .arg("-c")
        .arg("core.fsmonitor=false")
        .arg("-c")
        .arg("core.untrackedCache=false")
        .arg("-c")
        .arg("core.hooksPath=/dev/null")
        .args(arguments)
        .current_dir(repository_root)
        .env_clear()
        .env("LC_ALL", "C")
        .env("GIT_CONFIG_GLOBAL", "/dev/null")
        .env("GIT_CONFIG_NOSYSTEM", "1")
        .env("GIT_OPTIONAL_LOCKS", "0")
        .env("GIT_NO_LAZY_FETCH", "1")
        .env("GIT_NO_REPLACE_OBJECTS", "1")
        .env("GIT_TERMINAL_PROMPT", "0")
        .env("GIT_ASKPASS", "/bin/false")
        .env("SSH_ASKPASS", "/bin/false")
        .env("GIT_SSH_COMMAND", "/bin/false")
        .stdin(Stdio::null())
        .stdout(Stdio::piped())
        .stderr(Stdio::null());
    run_bounded(command, MAX_GIT_OUTPUT_BYTES)
        .map_err(|_| FreezePhaseACandidateError::RepositoryUnavailable)
}

fn one_line(bytes: &[u8]) -> Option<&str> {
    if bytes.is_empty() || bytes.last() != Some(&b'\n') || bytes[..bytes.len() - 1].contains(&b'\n')
    {
        return None;
    }
    std::str::from_utf8(&bytes[..bytes.len() - 1]).ok()
}

fn hash_stable_regular_file(
    path: &Path,
    maximum_length: u64,
    role: &'static str,
) -> Result<ArtifactDigest, FreezePhaseACandidateError> {
    let path_before = fs::symlink_metadata(path)
        .map_err(|_| FreezePhaseACandidateError::InvalidArtifact(role))?;
    if !path_before.file_type().is_file() || path_before.len() > maximum_length {
        return Err(FreezePhaseACandidateError::InvalidArtifact(role));
    }
    let descriptor = rustix::fs::open(
        path,
        rustix::fs::OFlags::RDONLY
            | rustix::fs::OFlags::CLOEXEC
            | rustix::fs::OFlags::NOFOLLOW
            | rustix::fs::OFlags::NONBLOCK,
        rustix::fs::Mode::empty(),
    )
    .map_err(|_| FreezePhaseACandidateError::InvalidArtifact(role))?;
    let mut file = File::from(descriptor);
    let descriptor_before = file
        .metadata()
        .map_err(|_| FreezePhaseACandidateError::InvalidArtifact(role))?;
    let before = StableFileIdentity::from_metadata(&descriptor_before);
    if before != StableFileIdentity::from_metadata(&path_before) {
        return Err(FreezePhaseACandidateError::ArtifactChanged(role));
    }
    let first = hash_open_file(&mut file, maximum_length, role)?;
    let second = hash_open_file(&mut file, maximum_length, role)?;
    let descriptor_after = file
        .metadata()
        .map_err(|_| FreezePhaseACandidateError::InvalidArtifact(role))?;
    let path_after = fs::symlink_metadata(path)
        .map_err(|_| FreezePhaseACandidateError::ArtifactChanged(role))?;
    if first != second
        || before != StableFileIdentity::from_metadata(&descriptor_after)
        || before != StableFileIdentity::from_metadata(&path_after)
    {
        return Err(FreezePhaseACandidateError::ArtifactChanged(role));
    }
    Ok(first)
}

#[cfg(target_os = "linux")]
fn hash_running_executable() -> Result<ArtifactDigest, FreezePhaseACandidateError> {
    let proc = open_proc_directory()?;
    // `/proc/self/exe` is a fixed kernel magic link, not a caller-selected
    // pathname. Following it is intentional and yields the currently running
    // executable object; a second open below must resolve to the same object.
    let descriptor = rustix::fs::openat(
        &proc,
        PROC_SELF_EXECUTABLE_ENTRY,
        rustix::fs::OFlags::RDONLY | rustix::fs::OFlags::CLOEXEC,
        rustix::fs::Mode::empty(),
    )
    .map_err(|_| FreezePhaseACandidateError::InvalidArtifact("running release binary"))?;
    let mut file = File::from(descriptor);
    let before = file
        .metadata()
        .map_err(|_| FreezePhaseACandidateError::InvalidArtifact("running release binary"))?;
    if !before.file_type().is_file() || before.len() > MAX_RELEASE_BINARY_BYTES {
        return Err(FreezePhaseACandidateError::InvalidArtifact(
            "running release binary",
        ));
    }
    let before = StableFileIdentity::from_metadata(&before);
    let first = hash_open_file(
        &mut file,
        MAX_RELEASE_BINARY_BYTES,
        "running release binary",
    )?;
    let second = hash_open_file(
        &mut file,
        MAX_RELEASE_BINARY_BYTES,
        "running release binary",
    )?;
    let after = file
        .metadata()
        .map_err(|_| FreezePhaseACandidateError::InvalidArtifact("running release binary"))?;
    let reopened = rustix::fs::openat(
        &proc,
        PROC_SELF_EXECUTABLE_ENTRY,
        rustix::fs::OFlags::RDONLY | rustix::fs::OFlags::CLOEXEC,
        rustix::fs::Mode::empty(),
    )
    .map_err(|_| FreezePhaseACandidateError::ArtifactChanged("running release binary"))?;
    let reopened = File::from(reopened)
        .metadata()
        .map_err(|_| FreezePhaseACandidateError::ArtifactChanged("running release binary"))?;
    if first != second
        || before != StableFileIdentity::from_metadata(&after)
        || before != StableFileIdentity::from_metadata(&reopened)
    {
        return Err(FreezePhaseACandidateError::ArtifactChanged(
            "running release binary",
        ));
    }
    Ok(first)
}

#[cfg(not(target_os = "linux"))]
fn hash_running_executable() -> Result<ArtifactDigest, FreezePhaseACandidateError> {
    Err(FreezePhaseACandidateError::UnsupportedHost)
}

fn hash_open_file(
    file: &mut File,
    maximum_length: u64,
    role: &'static str,
) -> Result<ArtifactDigest, FreezePhaseACandidateError> {
    file.seek(SeekFrom::Start(0))
        .map_err(|_| FreezePhaseACandidateError::InvalidArtifact(role))?;
    let mut reader = file.take(maximum_length.saturating_add(1));
    let mut digest = Sha256::new();
    let mut buffer = [0_u8; 64 * 1024];
    let mut length = 0_u64;
    loop {
        let count = reader
            .read(&mut buffer)
            .map_err(|_| FreezePhaseACandidateError::InvalidArtifact(role))?;
        if count == 0 {
            break;
        }
        length = length
            .checked_add(count as u64)
            .ok_or(FreezePhaseACandidateError::InvalidArtifact(role))?;
        if length > maximum_length {
            return Err(FreezePhaseACandidateError::InvalidArtifact(role));
        }
        digest.update(&buffer[..count]);
    }
    Ok(ArtifactDigest {
        sha256: format!("{:x}", digest.finalize()),
        length,
    })
}

#[cfg(target_os = "linux")]
fn observe_local_host_facts() -> Result<LocalHostFacts, FreezePhaseACandidateError> {
    let uname = rustix::system::uname();
    let uts_nodename = std::str::from_utf8(uname.nodename().to_bytes())
        .ok()
        .filter(|value| {
            !value.is_empty()
                && value.len() <= 64
                && value.trim() == *value
                && !value.chars().any(char::is_control)
        })
        .ok_or(FreezePhaseACandidateError::HostFactUnavailable(
            "UTS nodename",
        ))?
        .to_owned();
    let proc = open_proc_directory()?;
    let boot_descriptor = rustix::fs::openat(
        &proc,
        PROC_BOOT_ID_ENTRY,
        rustix::fs::OFlags::RDONLY | rustix::fs::OFlags::CLOEXEC | rustix::fs::OFlags::NOFOLLOW,
        rustix::fs::Mode::empty(),
    )
    .map_err(|_| FreezePhaseACandidateError::HostFactUnavailable("boot ID"))?;
    let mut boot_file = File::from(boot_descriptor).take(38);
    let mut boot_bytes = Vec::with_capacity(38);
    boot_file
        .read_to_end(&mut boot_bytes)
        .map_err(|_| FreezePhaseACandidateError::HostFactUnavailable("boot ID"))?;
    let boot_id = parse_boot_id(&boot_bytes)?;
    let before_euid = rustix::process::geteuid().as_raw();
    let after_euid = rustix::process::geteuid().as_raw();
    if before_euid != after_euid {
        return Err(FreezePhaseACandidateError::HostFactUnavailable(
            "effective user",
        ));
    }
    // The terminal component is intentionally a procfs magic link. Following
    // it is how Linux supplies an FD for the calling thread's network
    // namespace; all caller-selected paths remain absent.
    let namespace_descriptor = rustix::fs::openat(
        &proc,
        PROC_THREAD_NET_NAMESPACE_ENTRY,
        rustix::fs::OFlags::RDONLY | rustix::fs::OFlags::CLOEXEC,
        rustix::fs::Mode::empty(),
    )
    .map_err(|_| FreezePhaseACandidateError::HostFactUnavailable("network namespace"))?;
    let namespace_filesystem = rustix::fs::fstatfs(&namespace_descriptor)
        .map_err(|_| FreezePhaseACandidateError::HostFactUnavailable("network namespace"))?;
    if namespace_filesystem.f_type != NSFS_MAGIC {
        return Err(FreezePhaseACandidateError::HostFactUnavailable(
            "network namespace",
        ));
    }
    let network_namespace = File::from(namespace_descriptor)
        .metadata()
        .map_err(|_| FreezePhaseACandidateError::HostFactUnavailable("network namespace"))?;
    if network_namespace.dev() == 0 || network_namespace.ino() == 0 {
        return Err(FreezePhaseACandidateError::HostFactUnavailable(
            "network namespace",
        ));
    }
    Ok(LocalHostFacts {
        uts_nodename,
        boot_id,
        linux_euid: before_euid,
        network_namespace_device: network_namespace.dev(),
        network_namespace_inode: network_namespace.ino(),
        fixed_sources: LocalHostFactSources {
            uts_nodename: "linux_uname_uts_nodename_v1",
            boot_id: BOOT_ID_PATH,
            linux_euid: "linux_geteuid_v1",
            network_namespace: NETWORK_NAMESPACE_PATH,
        },
    })
}

#[cfg(target_os = "linux")]
fn open_proc_directory() -> Result<rustix::fd::OwnedFd, FreezePhaseACandidateError> {
    let proc = rustix::fs::open(
        PROC_ROOT_PATH,
        rustix::fs::OFlags::RDONLY | rustix::fs::OFlags::DIRECTORY | rustix::fs::OFlags::CLOEXEC,
        rustix::fs::Mode::empty(),
    )
    .map_err(|_| FreezePhaseACandidateError::HostFactUnavailable("procfs"))?;
    let filesystem = rustix::fs::fstatfs(&proc)
        .map_err(|_| FreezePhaseACandidateError::HostFactUnavailable("procfs"))?;
    if filesystem.f_type != rustix::fs::PROC_SUPER_MAGIC {
        return Err(FreezePhaseACandidateError::HostFactUnavailable("procfs"));
    }
    Ok(proc)
}

#[cfg(not(target_os = "linux"))]
fn observe_local_host_facts() -> Result<LocalHostFacts, FreezePhaseACandidateError> {
    Err(FreezePhaseACandidateError::UnsupportedHost)
}

fn parse_boot_id(bytes: &[u8]) -> Result<String, FreezePhaseACandidateError> {
    if bytes.len() != 37 || bytes.last() != Some(&b'\n') {
        return Err(FreezePhaseACandidateError::HostFactUnavailable("boot ID"));
    }
    let text = std::str::from_utf8(&bytes[..36])
        .map_err(|_| FreezePhaseACandidateError::HostFactUnavailable("boot ID"))?;
    let valid = text.bytes().enumerate().all(|(index, byte)| {
        if matches!(index, 8 | 13 | 18 | 23) {
            byte == b'-'
        } else {
            byte.is_ascii_digit() || (b'a'..=b'f').contains(&byte)
        }
    });
    if !valid {
        return Err(FreezePhaseACandidateError::HostFactUnavailable("boot ID"));
    }
    Ok(text.to_owned())
}

fn run_bounded(
    mut command: Command,
    maximum_output_bytes: usize,
) -> Result<Vec<u8>, FreezePhaseACandidateError> {
    let mut child = command
        .spawn()
        .map_err(|_| FreezePhaseACandidateError::LocalCommandFailed)?;
    let Some(stdout) = child.stdout.take() else {
        terminate_and_reap(&mut child);
        return Err(FreezePhaseACandidateError::LocalCommandFailed);
    };
    let (sender, receiver) = mpsc::sync_channel(1);
    let reader = thread::spawn(move || {
        let mut reader = stdout.take((maximum_output_bytes + 1) as u64);
        let mut bytes = Vec::with_capacity(maximum_output_bytes.min(8 * 1024));
        let result = reader.read_to_end(&mut bytes).map(|_| bytes);
        let _sent = sender.send(result);
    });
    let deadline = Instant::now() + MAX_LOCAL_HELPER_DURATION;
    let status = loop {
        match child.try_wait() {
            Ok(Some(status)) => break status,
            Ok(None) if Instant::now() < deadline => {
                thread::sleep(Duration::from_millis(5));
            }
            Ok(None) | Err(_) => {
                terminate_and_reap(&mut child);
                let _joined = reader.join();
                return Err(FreezePhaseACandidateError::LocalCommandFailed);
            }
        }
    };
    let bytes = receiver
        .recv_timeout(Duration::from_secs(1))
        .map_err(|_| FreezePhaseACandidateError::LocalCommandFailed)?
        .map_err(|_| FreezePhaseACandidateError::LocalCommandFailed)?;
    reader
        .join()
        .map_err(|_| FreezePhaseACandidateError::LocalCommandFailed)?;
    if bytes.len() > maximum_output_bytes {
        return Err(FreezePhaseACandidateError::LocalCommandFailed);
    }
    if !status.success() {
        return Err(FreezePhaseACandidateError::LocalCommandFailed);
    }
    Ok(bytes)
}

fn terminate_and_reap(child: &mut Child) {
    let _kill = child.kill();
    let _wait = child.wait();
}

fn false_boolean_paths<T: Serialize>(
    verification: &T,
) -> Result<Vec<String>, FreezePhaseACandidateError> {
    let value = serde_json::to_value(verification)
        .map_err(|_| FreezePhaseACandidateError::ReportSerialization)?;
    let mut paths = Vec::new();
    collect_false_boolean_paths("", &value, &mut paths);
    paths.sort();
    Ok(paths)
}

fn collect_false_boolean_paths(prefix: &str, value: &Value, paths: &mut Vec<String>) {
    match value {
        Value::Bool(false) if !prefix.is_empty() => paths.push(prefix.to_owned()),
        Value::Object(fields) => {
            for (name, value) in fields {
                let path = if prefix.is_empty() {
                    name.clone()
                } else {
                    format!("{prefix}.{name}")
                };
                collect_false_boolean_paths(&path, value, paths);
            }
        }
        Value::Array(values) => {
            for (index, value) in values.iter().enumerate() {
                collect_false_boolean_paths(&format!("{prefix}[{index}]"), value, paths);
            }
        }
        _ => {}
    }
}

#[derive(Debug, Error)]
pub(crate) enum FreezePhaseACandidateError {
    #[error("Phase-A candidate repository is unavailable or is not the exact supplied Git root")]
    RepositoryUnavailable,
    #[error("Phase-A candidate requires a clean Git worktree including untracked files")]
    RepositoryDirty,
    #[error("Phase-A candidate Git commit or cleanliness changed during the freeze")]
    RepositoryChanged,
    #[error("Phase-A candidate repository uses unsupported external-process Git configuration")]
    UnsupportedRepositoryFeature,
    #[error("Phase-A candidate reviewed input is invalid: {0}")]
    InvalidReviewedInput(&'static str),
    #[error("Phase-A candidate static V3 nine-holder conjunction is invalid")]
    StaticV3ConjunctionInvalid,
    #[error("Phase-A candidate ten-holder eligibility envelope V4 is invalid")]
    PhaseAV4EnvelopeInvalid,
    #[error("Phase-A candidate artifact is not one bounded stable regular file: {0}")]
    InvalidArtifact(&'static str),
    #[error("Phase-A candidate artifact changed while it was hashed: {0}")]
    ArtifactChanged(&'static str),
    #[error("Phase-A candidate fixed local host fact is unavailable: {0}")]
    HostFactUnavailable(&'static str),
    #[error("Phase-A candidate is supported only on Linux")]
    UnsupportedHost,
    #[error("Phase-A candidate fixed local helper failed")]
    LocalCommandFailed,
    #[error("Phase-A candidate report serialization failed")]
    ReportSerialization,
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn stable_hash_is_exact_and_rejects_symlinks() {
        let directory = tempfile::tempdir().unwrap();
        let path = directory.path().join("artifact");
        fs::write(&path, b"phase-a\n").unwrap();
        let digest = hash_stable_regular_file(&path, 64, "test artifact").unwrap();
        assert_eq!(digest.length, 8);
        assert_eq!(
            digest.sha256,
            "1a650de65ad998ad402f6c59922f4e91b1ef6308f9e43c4beb744d44ff125394"
        );

        let link = directory.path().join("link");
        std::os::unix::fs::symlink(&path, &link).unwrap();
        assert!(matches!(
            hash_stable_regular_file(&link, 64, "test artifact"),
            Err(FreezePhaseACandidateError::InvalidArtifact("test artifact"))
        ));
    }

    #[test]
    fn fixed_boot_id_parser_is_canonical_and_bounded() {
        assert_eq!(
            parse_boot_id(b"01234567-89ab-cdef-0123-456789abcdef\n").unwrap(),
            "01234567-89ab-cdef-0123-456789abcdef"
        );
        assert!(parse_boot_id(b"01234567-89ab-cdef-0123-456789abcdeF\n").is_err());
    }

    #[test]
    fn false_fact_collection_is_complete_and_sorted() {
        let value = serde_json::json!({
            "z": false,
            "a": {"true": true, "false": false},
            "array": [false, true]
        });
        let mut paths = Vec::new();
        collect_false_boolean_paths("", &value, &mut paths);
        paths.sort();
        assert_eq!(paths, ["a.false", "array[0]", "z"]);
    }
}
