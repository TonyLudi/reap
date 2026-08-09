//! Runner-private source of the current PM-T2 process, host, time, and
//! geoblock binding.
//!
//! Production observation has no caller-selected path, hash, host identity,
//! user, clock, egress, boolean assertion, or network origin. The sole local
//! source reads the running Linux image through `/proc/self/exe`, observes the
//! namespace-visible host and effective user, and joins only an existing typed
//! geoblock observation. This is evidence for a later online permit; it owns no
//! credential, signer, request, journal, mutation, or transport capability.
//!
//! The controlled host must provide a trusted procfs mount and trusted UTS and
//! user namespaces. The fixed `/usr/bin/getent` helper and its NSS providers
//! are correctness, integrity, and availability dependencies: NSS may block,
//! including on a remote provider, but a blocked, compromised, or failed
//! lookup cannot honestly support a witness. The helper is not release-binary
//! attestation and never supplies executable identity.
//! `SystemTime` is only the local kernel wall clock formatted as UTC, not an
//! external time attestation. The geoblock-reported IP is bound to the reviewed
//! egress address but is not evidence that another CLOB connection used it.

use std::{
    fmt,
    fs::File,
    io::{Read, Seek, SeekFrom},
    net::IpAddr,
    process::{Child, Command, Stdio},
    time::{Duration, Instant, SystemTime, UNIX_EPOCH},
};

use chrono::{DateTime, SecondsFormat, Utc};
use reap_pm_controlled_trial::{
    CanonicalAuthorization, CanonicalTrialConfig, TrialPhase, verify_authorization,
};
use reap_polymarket_live_adapter::{
    PmGeoblockObservationCommitment, PmHttpReceiveClock, PmProductionGeoblockObservation,
};
use sha2::{Digest as _, Sha256};
use thiserror::Error;

const PROC_ROOT_PATH: &str = "/proc";
const PROC_SELF_EXE_ENTRY: &str = "self/exe";
const PROC_BOOT_ID_ENTRY: &str = "sys/kernel/random/boot_id";
const GETENT_PATH: &str = "/usr/bin/getent";
const BOOT_ID_TEXT_BYTES: usize = 36;
const BOOT_ID_FILE_BYTES: usize = BOOT_ID_TEXT_BYTES + 1;
const MAX_RUNTIME_USER_BYTES: usize = 128;
const MAX_GETENT_STDOUT_BYTES: usize = 4 * 1024;

/// Sole source of current process-local runtime evidence.
///
/// `system` accepts no facts. Retaining the creating PID makes a witness and
/// its monotonic `Instant` unusable after a fork into another process.
pub(super) struct PmCurrentRuntimeObserver {
    creating_process_id: u32,
}

impl PmCurrentRuntimeObserver {
    #[must_use]
    pub(super) fn system() -> Self {
        Self {
            creating_process_id: std::process::id(),
        }
    }

    /// Observe and bind the exact current runtime for one Phase-A placement.
    ///
    /// The production-origin geoblock wrapper is consumed as typed source
    /// evidence. Its role-private monotonic scalar is retained only as
    /// provenance; age is checked using its wall receive edge and this
    /// observer's `SystemTime`.
    pub(super) fn observe_phase_a_place(
        &mut self,
        config: &CanonicalTrialConfig,
        authorization: &CanonicalAuthorization,
        geoblock: PmProductionGeoblockObservation,
    ) -> Result<PmPhaseAPlaceCurrentRuntimeWitness, PmCurrentRuntimeError> {
        self.validate_observer_process()?;
        validate_place_cancel_phase(config.value().phase, authorization.value().phase)?;
        let preliminary_wall = SystemTime::now();
        let preliminary_utc = system_time_utc(preliminary_wall)?;
        verify_authorization(config, authorization, preliminary_utc)
            .map_err(|_| PmCurrentRuntimeError::AuthorizationBinding)?;

        let maximum_age_ms = config
            .value()
            .time_limits
            .maximum_preflight_observation_age_ms;
        let maximum_age = Duration::from_millis(maximum_age_ms);
        let expected_binary_length = authorization.value().build.release_binary_length;
        let local = capture_local_runtime(expected_binary_length)?;
        validate_capture_window(&local, maximum_age)?;
        if local.process_id != self.creating_process_id {
            return Err(PmCurrentRuntimeError::ProcessIdentityChanged);
        }

        let geoblock = GeoblockEvidence::from_source(geoblock);
        validate_geoblock(&geoblock, local.wall_completed, maximum_age)?;
        let observed_host = ObservedHostBinding {
            host_identity: local.host_identity.clone(),
            boot_identity: local.boot_identity.clone(),
            runtime_user: local.runtime_user.clone(),
            egress_identity: geoblock.ip,
        };
        let cleanup_not_after_utc =
            validate_exact_bindings_and_windows(config, authorization, &local, &observed_host)?;

        Ok(PmPhaseAPlaceCurrentRuntimeWitness {
            evidence: PmCurrentRuntimeEvidence {
                executable: local.executable,
                process_id: local.process_id,
                effective_user_id: local.effective_user_id,
                observed_host,
                geoblock,
                config_sha256: config.canonical_sha256().into(),
                config_length: config.canonical_length(),
                config_fingerprint: config.fingerprint().into(),
                trial_plan_fingerprint: config.plan_fingerprint().into(),
                authorization_id: authorization.value().authorization_id.as_str().into(),
                authorization_fingerprint: authorization.fingerprint().into(),
                first_wall_complete: local.wall_completed,
                first_monotonic_complete: local.monotonic_completed,
                checked_wall_complete: local.wall_completed,
                checked_monotonic_complete: local.monotonic_completed,
                checked_wall_capture_unix_ns: unix_nanoseconds(local.wall_completed)?,
                checked_at_utc: canonical_utc(local.wall_completed)?.into(),
                maximum_age,
                maximum_age_ms,
                cleanup_not_after_utc: cleanup_not_after_utc.into(),
            },
        })
    }

    /// Consume an initial witness and re-observe the host, running image, and
    /// time at this assembly edge. This point-in-time check does not close
    /// final freshness; a future permit/transport boundary must consume this
    /// witness and perform its own immediate source recheck before dispatch.
    pub(super) fn recheck_phase_a_place(
        &mut self,
        witness: PmPhaseAPlaceCurrentRuntimeWitness,
        config: &CanonicalTrialConfig,
        authorization: &CanonicalAuthorization,
    ) -> Result<PmRevalidatedPhaseAPlaceCurrentRuntimeWitness, PmCurrentRuntimeError> {
        self.validate_observer_process()?;
        validate_place_cancel_phase(config.value().phase, authorization.value().phase)?;
        let PmPhaseAPlaceCurrentRuntimeWitness { mut evidence } = witness;
        evidence.executable.revalidate_held_identity()?;

        validate_pinned_config_and_authorization(&evidence, config, authorization)?;
        let local = capture_local_runtime(evidence.executable.length)?;
        validate_capture_window(&local, evidence.maximum_age)?;
        evidence.executable.revalidate_held_identity()?;

        validate_age_window(
            evidence.first_wall_complete,
            evidence.first_monotonic_complete,
            local.wall_completed,
            local.monotonic_completed,
            evidence.maximum_age,
        )?;
        validate_local_runtime_identity(
            LocalRuntimeIdentityView {
                process_id: evidence.process_id,
                effective_user_id: evidence.effective_user_id,
                host_identity: evidence.observed_host.host_identity.as_ref(),
                boot_identity: evidence.observed_host.boot_identity.as_ref(),
                runtime_user: evidence.observed_host.runtime_user.as_ref(),
            },
            LocalRuntimeIdentityView {
                process_id: local.process_id,
                effective_user_id: local.effective_user_id,
                host_identity: local.host_identity.as_ref(),
                boot_identity: local.boot_identity.as_ref(),
                runtime_user: local.runtime_user.as_ref(),
            },
            self.creating_process_id,
        )?;
        validate_executable_match(&evidence.executable, &local.executable)?;

        validate_geoblock(
            &evidence.geoblock,
            local.wall_completed,
            evidence.maximum_age,
        )?;
        let cleanup_not_after_utc = validate_exact_bindings_and_windows(
            config,
            authorization,
            &local,
            &evidence.observed_host,
        )?;
        if cleanup_not_after_utc.as_str() != evidence.cleanup_not_after_utc.as_ref() {
            return Err(PmCurrentRuntimeError::AuthorizationBinding);
        }

        evidence.executable = local.executable;
        evidence.checked_wall_complete = local.wall_completed;
        evidence.checked_monotonic_complete = local.monotonic_completed;
        evidence.checked_wall_capture_unix_ns = unix_nanoseconds(local.wall_completed)?;
        evidence.checked_at_utc = canonical_utc(local.wall_completed)?.into();
        Ok(PmRevalidatedPhaseAPlaceCurrentRuntimeWitness { evidence })
    }

    fn validate_observer_process(&self) -> Result<(), PmCurrentRuntimeError> {
        if std::process::id() != self.creating_process_id {
            return Err(PmCurrentRuntimeError::ProcessIdentityChanged);
        }
        Ok(())
    }
}

impl fmt::Debug for PmCurrentRuntimeObserver {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str("PmCurrentRuntimeObserver(<system-local-source>)")
    }
}

/// Initial move-only runtime witness. It is evidence, not dispatch authority.
pub(super) struct PmPhaseAPlaceCurrentRuntimeWitness {
    evidence: PmCurrentRuntimeEvidence,
}

impl PmPhaseAPlaceCurrentRuntimeWitness {
    /// Borrow nonsecret facts for the canonical V2 evidence assembler. The
    /// returned view cannot outlive or reconstruct this move-only witness.
    #[must_use]
    pub(super) const fn facts(&self) -> PmCurrentRuntimeFactsView<'_> {
        PmCurrentRuntimeFactsView {
            evidence: &self.evidence,
        }
    }
}

impl fmt::Debug for PmPhaseAPlaceCurrentRuntimeWitness {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str(
            "PmPhaseAPlaceCurrentRuntimeWitness(<move-only; runtime-and-geoblock-bound>)",
        )
    }
}

/// Point-in-time, move-only evidence produced only by consuming re-observation.
/// It is explicitly not mutation, dispatch, freshness, or transport authority,
/// cannot be decomposed into custody, and must remain intact for a future
/// permit/transport-side consuming recheck immediately before dispatch.
pub(super) struct PmRevalidatedPhaseAPlaceCurrentRuntimeWitness {
    evidence: PmCurrentRuntimeEvidence,
}

impl PmRevalidatedPhaseAPlaceCurrentRuntimeWitness {
    /// Borrow nonsecret facts for the current canonical V2 evidence join. This
    /// does not expose the `Instant`, held executable descriptor, or custody
    /// fields and does not extend the point-in-time check's freshness.
    #[must_use]
    pub(super) const fn facts(&self) -> PmCurrentRuntimeFactsView<'_> {
        PmCurrentRuntimeFactsView {
            evidence: &self.evidence,
        }
    }
}

impl fmt::Debug for PmRevalidatedPhaseAPlaceCurrentRuntimeWitness {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str(
            "PmRevalidatedPhaseAPlaceCurrentRuntimeWitness(<move-only; non-authoritative; future-consuming-recheck-required>)",
        )
    }
}

/// Lifetime-borrowed, non-constructible view of exact nonsecret persistence
/// facts. It is not freshness or dispatch authority and has no owned form.
pub(super) struct PmCurrentRuntimeFactsView<'a> {
    evidence: &'a PmCurrentRuntimeEvidence,
}

impl PmCurrentRuntimeFactsView<'_> {
    #[must_use]
    pub(super) fn canonical_config_sha256(&self) -> &str {
        &self.evidence.config_sha256
    }

    #[must_use]
    pub(super) const fn canonical_config_length(&self) -> u64 {
        self.evidence.config_length
    }

    #[must_use]
    pub(super) fn canonical_config_fingerprint(&self) -> &str {
        &self.evidence.config_fingerprint
    }

    #[must_use]
    pub(super) fn trial_plan_fingerprint(&self) -> &str {
        &self.evidence.trial_plan_fingerprint
    }

    #[must_use]
    pub(super) fn authorization_id(&self) -> &str {
        &self.evidence.authorization_id
    }

    #[must_use]
    pub(super) fn authorization_fingerprint(&self) -> &str {
        &self.evidence.authorization_fingerprint
    }

    #[must_use]
    pub(super) fn release_binary_sha256(&self) -> &str {
        &self.evidence.executable.sha256_hex
    }

    #[must_use]
    pub(super) const fn release_binary_length(&self) -> u64 {
        self.evidence.executable.length
    }

    /// Exact namespace-visible Linux UTS nodename; no DNS normalization.
    #[must_use]
    pub(super) fn host_identity(&self) -> &str {
        &self.evidence.observed_host.host_identity
    }

    #[must_use]
    pub(super) fn boot_identity(&self) -> &str {
        &self.evidence.observed_host.boot_identity
    }

    /// NSS name resolved from the stable effective UID, never an environment
    /// variable or session login name.
    #[must_use]
    pub(super) fn runtime_user(&self) -> &str {
        &self.evidence.observed_host.runtime_user
    }

    /// Exact numeric Linux effective UID captured by the source. This is a
    /// borrowed V2 evidence fact and is never authority or a V1 schema field.
    #[must_use]
    pub(super) const fn linux_effective_user_id(&self) -> u32 {
        self.evidence.effective_user_id
    }

    #[must_use]
    pub(super) const fn authorized_egress_identity(&self) -> IpAddr {
        self.evidence.observed_host.egress_identity
    }

    #[must_use]
    pub(super) const fn geoblock_reported_ip(&self) -> IpAddr {
        self.evidence.geoblock.ip
    }

    #[must_use]
    pub(super) const fn geoblock_blocked(&self) -> bool {
        self.evidence.geoblock.blocked
    }

    #[must_use]
    pub(super) fn geoblock_country(&self) -> &str {
        &self.evidence.geoblock.country
    }

    #[must_use]
    pub(super) fn geoblock_region(&self) -> &str {
        &self.evidence.geoblock.region
    }

    #[must_use]
    pub(super) const fn geoblock_commitment(&self) -> PmGeoblockObservationCommitment {
        self.evidence.geoblock.commitment
    }

    #[must_use]
    pub(super) const fn geoblock_wall_receive_ns(&self) -> u64 {
        self.evidence.geoblock.receive_clock.local_wall_receive_ns()
    }

    /// Opaque role-private provenance only. This scalar must never be compared
    /// with the runtime observer's `Instant` or used to calculate age.
    #[must_use]
    pub(super) const fn geoblock_source_monotonic_provenance_ns(&self) -> u64 {
        self.evidence.geoblock.receive_clock.monotonic_receive_ns()
    }

    #[must_use]
    pub(super) fn checked_at_utc(&self) -> &str {
        &self.evidence.checked_at_utc
    }

    /// Exact source-captured local wall completion edge as Unix nanoseconds.
    /// This borrowed V2 evidence fact is not external UTC or freshness proof.
    #[must_use]
    pub(super) const fn checked_wall_capture_unix_ns(&self) -> u64 {
        self.evidence.checked_wall_capture_unix_ns
    }

    #[must_use]
    pub(super) const fn maximum_age_ms(&self) -> u64 {
        self.evidence.maximum_age_ms
    }

    /// Absolute authorization cleanup boundary. Admission additionally proves
    /// that the full configured relative cleanup budget remains before it.
    #[must_use]
    pub(super) fn cleanup_not_after_utc(&self) -> &str {
        &self.evidence.cleanup_not_after_utc
    }
}

impl fmt::Debug for PmCurrentRuntimeFactsView<'_> {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str("PmCurrentRuntimeFactsView(<borrowed; nonsecret; non-authority>)")
    }
}

struct PmCurrentRuntimeEvidence {
    executable: ExecutableCustody,
    process_id: u32,
    effective_user_id: u32,
    observed_host: ObservedHostBinding,
    geoblock: GeoblockEvidence,
    config_sha256: Box<str>,
    config_length: u64,
    config_fingerprint: Box<str>,
    trial_plan_fingerprint: Box<str>,
    authorization_id: Box<str>,
    authorization_fingerprint: Box<str>,
    first_wall_complete: SystemTime,
    first_monotonic_complete: Instant,
    checked_wall_complete: SystemTime,
    checked_monotonic_complete: Instant,
    checked_wall_capture_unix_ns: u64,
    checked_at_utc: Box<str>,
    maximum_age: Duration,
    maximum_age_ms: u64,
    cleanup_not_after_utc: Box<str>,
}

struct ConfigAuthorizationPinsView<'a> {
    config_sha256: &'a str,
    config_length: u64,
    config_fingerprint: &'a str,
    trial_plan_fingerprint: &'a str,
    authorization_id: &'a str,
    authorization_fingerprint: &'a str,
}

struct LocalRuntimeCapture {
    executable: ExecutableCustody,
    process_id: u32,
    effective_user_id: u32,
    host_identity: Box<str>,
    boot_identity: Box<str>,
    runtime_user: Box<str>,
    wall_started: SystemTime,
    wall_completed: SystemTime,
    monotonic_started: Instant,
    monotonic_completed: Instant,
}

struct LocalRuntimeIdentityView<'a> {
    process_id: u32,
    effective_user_id: u32,
    host_identity: &'a str,
    boot_identity: &'a str,
    runtime_user: &'a str,
}

#[derive(PartialEq, Eq)]
struct LocalRuntimeIdentity {
    process_id: u32,
    effective_user_id: u32,
    host_identity: Box<str>,
    boot_identity: Box<str>,
    runtime_user: Box<str>,
}

#[derive(PartialEq, Eq)]
struct ObservedHostBinding {
    host_identity: Box<str>,
    boot_identity: Box<str>,
    runtime_user: Box<str>,
    egress_identity: IpAddr,
}

struct GeoblockEvidence {
    blocked: bool,
    ip: IpAddr,
    country: Box<str>,
    region: Box<str>,
    receive_clock: PmHttpReceiveClock,
    commitment: PmGeoblockObservationCommitment,
}

impl GeoblockEvidence {
    fn from_source(observation: PmProductionGeoblockObservation) -> Self {
        let receive_clock = observation.receive_clock();
        let commitment = observation.commitment();
        let status = observation.status();
        Self {
            blocked: status.blocked(),
            ip: status.ip(),
            country: status.country().into(),
            region: status.region().into(),
            receive_clock,
            commitment,
        }
    }
}

struct ExecutableCustody {
    file: File,
    identity: ExecutableFileIdentity,
    sha256: [u8; 32],
    sha256_hex: Box<str>,
    length: u64,
}

impl ExecutableCustody {
    fn revalidate_held_identity(&self) -> Result<(), PmCurrentRuntimeError> {
        let current = executable_file_identity(&self.file)?;
        if current != self.identity || current.length != self.length {
            return Err(PmCurrentRuntimeError::ExecutableChanged);
        }
        Ok(())
    }
}

#[derive(Clone, Copy, PartialEq, Eq)]
struct ExecutableFileIdentity {
    device: u64,
    inode: u64,
    mode: u32,
    owner_user_id: u32,
    owner_group_id: u32,
    link_count: u64,
    length: u64,
    modified_seconds: i64,
    modified_nanoseconds: i64,
    changed_seconds: i64,
    changed_nanoseconds: i64,
}

fn capture_local_runtime(
    expected_binary_length: u64,
) -> Result<LocalRuntimeCapture, PmCurrentRuntimeError> {
    #[cfg(target_os = "linux")]
    {
        let monotonic_started = Instant::now();
        let wall_started = SystemTime::now();
        let before = observe_local_runtime_identity()?;
        let executable = capture_current_executable(expected_binary_length)?;
        let after = observe_local_runtime_identity()?;
        let wall_completed = SystemTime::now();
        let monotonic_completed = Instant::now();
        if before != after {
            return Err(PmCurrentRuntimeError::LocalRuntimeChanged);
        }
        Ok(LocalRuntimeCapture {
            executable,
            process_id: after.process_id,
            effective_user_id: after.effective_user_id,
            host_identity: after.host_identity,
            boot_identity: after.boot_identity,
            runtime_user: after.runtime_user,
            wall_started,
            wall_completed,
            monotonic_started,
            monotonic_completed,
        })
    }
    #[cfg(not(target_os = "linux"))]
    {
        let _ = expected_binary_length;
        Err(PmCurrentRuntimeError::UnsupportedPlatform)
    }
}

fn validate_capture_window(
    capture: &LocalRuntimeCapture,
    maximum_age: Duration,
) -> Result<(), PmCurrentRuntimeError> {
    if maximum_age.is_zero() {
        return Err(PmCurrentRuntimeError::ObservationExpired);
    }
    validate_age_window(
        capture.wall_started,
        capture.monotonic_started,
        capture.wall_completed,
        capture.monotonic_completed,
        maximum_age,
    )
}

fn validate_age_window(
    first_wall: SystemTime,
    first_monotonic: Instant,
    current_wall: SystemTime,
    current_monotonic: Instant,
    maximum_age: Duration,
) -> Result<(), PmCurrentRuntimeError> {
    let monotonic_age = current_monotonic
        .checked_duration_since(first_monotonic)
        .ok_or(PmCurrentRuntimeError::MonotonicClockRegression)?;
    let wall_age = current_wall
        .duration_since(first_wall)
        .map_err(|_| PmCurrentRuntimeError::WallClockRegression)?;
    if monotonic_age > maximum_age || wall_age > maximum_age {
        return Err(PmCurrentRuntimeError::ObservationExpired);
    }
    Ok(())
}

fn validate_local_runtime_identity(
    expected: LocalRuntimeIdentityView<'_>,
    current: LocalRuntimeIdentityView<'_>,
    observer_process_id: u32,
) -> Result<(), PmCurrentRuntimeError> {
    if current.process_id != expected.process_id
        || current.process_id != observer_process_id
        || current.effective_user_id != expected.effective_user_id
        || current.host_identity != expected.host_identity
        || current.boot_identity != expected.boot_identity
        || current.runtime_user != expected.runtime_user
    {
        return Err(PmCurrentRuntimeError::LocalRuntimeChanged);
    }
    Ok(())
}

fn validate_executable_match(
    expected: &ExecutableCustody,
    current: &ExecutableCustody,
) -> Result<(), PmCurrentRuntimeError> {
    if current.identity != expected.identity
        || current.sha256 != expected.sha256
        || current.length != expected.length
    {
        return Err(PmCurrentRuntimeError::ExecutableChanged);
    }
    Ok(())
}

fn validate_place_cancel_phase(
    config_phase: TrialPhase,
    authorization_phase: TrialPhase,
) -> Result<(), PmCurrentRuntimeError> {
    if config_phase != TrialPhase::APlaceCancel || authorization_phase != TrialPhase::APlaceCancel {
        return Err(PmCurrentRuntimeError::TrialPhaseMismatch);
    }
    Ok(())
}

fn validate_pinned_config_and_authorization(
    evidence: &PmCurrentRuntimeEvidence,
    config: &CanonicalTrialConfig,
    authorization: &CanonicalAuthorization,
) -> Result<(), PmCurrentRuntimeError> {
    validate_matching_pins(
        ConfigAuthorizationPinsView {
            config_sha256: evidence.config_sha256.as_ref(),
            config_length: evidence.config_length,
            config_fingerprint: evidence.config_fingerprint.as_ref(),
            trial_plan_fingerprint: evidence.trial_plan_fingerprint.as_ref(),
            authorization_id: evidence.authorization_id.as_ref(),
            authorization_fingerprint: evidence.authorization_fingerprint.as_ref(),
        },
        ConfigAuthorizationPinsView {
            config_sha256: config.canonical_sha256(),
            config_length: config.canonical_length(),
            config_fingerprint: config.fingerprint(),
            trial_plan_fingerprint: config.plan_fingerprint(),
            authorization_id: &authorization.value().authorization_id,
            authorization_fingerprint: authorization.fingerprint(),
        },
    )
}

fn validate_matching_pins(
    expected: ConfigAuthorizationPinsView<'_>,
    current: ConfigAuthorizationPinsView<'_>,
) -> Result<(), PmCurrentRuntimeError> {
    if current.config_sha256 != expected.config_sha256
        || current.config_length != expected.config_length
        || current.config_fingerprint != expected.config_fingerprint
        || current.trial_plan_fingerprint != expected.trial_plan_fingerprint
        || current.authorization_id != expected.authorization_id
        || current.authorization_fingerprint != expected.authorization_fingerprint
    {
        return Err(PmCurrentRuntimeError::AuthorizationBinding);
    }
    Ok(())
}

fn validate_exact_bindings_and_windows(
    config: &CanonicalTrialConfig,
    authorization: &CanonicalAuthorization,
    local: &LocalRuntimeCapture,
    observed_host: &ObservedHostBinding,
) -> Result<String, PmCurrentRuntimeError> {
    let now = system_time_utc(local.wall_completed)?;
    verify_authorization(config, authorization, now)
        .map_err(|_| PmCurrentRuntimeError::AuthorizationBinding)?;
    let approved = authorization.value();
    if local.executable.sha256_hex.as_ref() != approved.build.release_binary_sha256
        || local.executable.length != approved.build.release_binary_length
        || observed_host.host_identity.as_ref() != approved.host.host_identity
        || observed_host.boot_identity.as_ref() != approved.host.boot_identity
        || observed_host.runtime_user.as_ref() != approved.host.runtime_user
    {
        return Err(PmCurrentRuntimeError::AuthorizationBinding);
    }
    validate_authorized_egress(
        observed_host.egress_identity,
        &approved.host.egress_identity,
    )?;

    let cleanup = parse_canonical_utc(&approved.cleanup_not_after_utc)?;
    let cleanup_wall = SystemTime::from(cleanup);
    let cleanup_budget = Duration::from_millis(config.value().time_limits.cleanup_not_after_ms);
    validate_cleanup_runway(local.wall_completed, cleanup_wall, cleanup_budget)?;
    Ok(approved.cleanup_not_after_utc.clone())
}

fn validate_authorized_egress(
    observed: IpAddr,
    authorized: &str,
) -> Result<(), PmCurrentRuntimeError> {
    let authorized = authorized
        .parse::<IpAddr>()
        .map_err(|_| PmCurrentRuntimeError::AuthorizationBinding)?;
    if observed != authorized {
        return Err(PmCurrentRuntimeError::AuthorizationBinding);
    }
    Ok(())
}

fn validate_cleanup_runway(
    current: SystemTime,
    cleanup_wall: SystemTime,
    cleanup_budget: Duration,
) -> Result<(), PmCurrentRuntimeError> {
    let required_cleanup_boundary = current
        .checked_add(cleanup_budget)
        .ok_or(PmCurrentRuntimeError::CleanupWindow)?;
    // Conservative interpretation: the complete configured relative cleanup
    // budget must remain at the placement edge, even when cleanup may finish
    // earlier in a successful run.
    if current > cleanup_wall || required_cleanup_boundary > cleanup_wall {
        return Err(PmCurrentRuntimeError::CleanupWindow);
    }
    Ok(())
}

fn validate_geoblock(
    geoblock: &GeoblockEvidence,
    now: SystemTime,
    maximum_age: Duration,
) -> Result<(), PmCurrentRuntimeError> {
    validate_geoblock_values(
        geoblock.blocked,
        geoblock.receive_clock.local_wall_receive_ns(),
        now,
        maximum_age,
    )
}

fn validate_geoblock_values(
    blocked: bool,
    received_ns: u64,
    now: SystemTime,
    maximum_age: Duration,
) -> Result<(), PmCurrentRuntimeError> {
    if blocked {
        return Err(PmCurrentRuntimeError::GeoblockBlocked);
    }
    let now_ns = unix_nanoseconds(now)?;
    let age_ns = now_ns
        .checked_sub(received_ns)
        .ok_or(PmCurrentRuntimeError::GeoblockFromFuture)?;
    if u128::from(age_ns) > maximum_age.as_nanos() {
        return Err(PmCurrentRuntimeError::GeoblockExpired);
    }
    Ok(())
}

fn system_time_utc(value: SystemTime) -> Result<DateTime<Utc>, PmCurrentRuntimeError> {
    unix_nanoseconds(value)?;
    Ok(DateTime::<Utc>::from(value))
}

fn canonical_utc(value: SystemTime) -> Result<String, PmCurrentRuntimeError> {
    Ok(system_time_utc(value)?.to_rfc3339_opts(SecondsFormat::Secs, true))
}

fn parse_canonical_utc(value: &str) -> Result<DateTime<Utc>, PmCurrentRuntimeError> {
    let parsed = DateTime::parse_from_rfc3339(value)
        .map_err(|_| PmCurrentRuntimeError::AuthorizationBinding)?
        .with_timezone(&Utc);
    if parsed.to_rfc3339_opts(SecondsFormat::Secs, true) != value {
        return Err(PmCurrentRuntimeError::AuthorizationBinding);
    }
    Ok(parsed)
}

fn unix_nanoseconds(value: SystemTime) -> Result<u64, PmCurrentRuntimeError> {
    value
        .duration_since(UNIX_EPOCH)
        .map_err(|_| PmCurrentRuntimeError::WallClockRegression)?
        .as_nanos()
        .try_into()
        .map_err(|_| PmCurrentRuntimeError::ClockOverflow)
}

#[cfg(target_os = "linux")]
fn observe_local_runtime_identity() -> Result<LocalRuntimeIdentity, PmCurrentRuntimeError> {
    let process_id = std::process::id();
    let host_identity = linux_uts_nodename()?;
    let boot_identity = linux_boot_identity()?;
    let (effective_user_id, runtime_user) = effective_runtime_user()?;
    if std::process::id() != process_id {
        return Err(PmCurrentRuntimeError::ProcessIdentityChanged);
    }
    Ok(LocalRuntimeIdentity {
        process_id,
        effective_user_id,
        host_identity,
        boot_identity,
        runtime_user,
    })
}

#[cfg(target_os = "linux")]
fn linux_uts_nodename() -> Result<Box<str>, PmCurrentRuntimeError> {
    let value = rustix::system::uname();
    let text = std::str::from_utf8(value.nodename().to_bytes())
        .map_err(|_| PmCurrentRuntimeError::HostIdentityUnavailable)?;
    if text.is_empty()
        || text.len() > 64
        || text.trim() != text
        || text.chars().any(char::is_control)
    {
        return Err(PmCurrentRuntimeError::HostIdentityUnavailable);
    }
    Ok(text.into())
}

#[cfg(target_os = "linux")]
fn linux_boot_identity() -> Result<Box<str>, PmCurrentRuntimeError> {
    let proc = open_proc_directory()?;
    let descriptor = rustix::fs::openat(
        &proc,
        PROC_BOOT_ID_ENTRY,
        rustix::fs::OFlags::RDONLY | rustix::fs::OFlags::CLOEXEC | rustix::fs::OFlags::NOFOLLOW,
        rustix::fs::Mode::empty(),
    )
    .map_err(|_| PmCurrentRuntimeError::BootIdentityUnavailable)?;
    let file = File::from(descriptor);
    let mut bytes = Vec::with_capacity(BOOT_ID_FILE_BYTES + 1);
    file.take((BOOT_ID_FILE_BYTES + 1) as u64)
        .read_to_end(&mut bytes)
        .map_err(|_| PmCurrentRuntimeError::BootIdentityUnavailable)?;
    if bytes.len() != BOOT_ID_FILE_BYTES || bytes[BOOT_ID_TEXT_BYTES] != b'\n' {
        return Err(PmCurrentRuntimeError::BootIdentityUnavailable);
    }
    let text = std::str::from_utf8(&bytes[..BOOT_ID_TEXT_BYTES])
        .map_err(|_| PmCurrentRuntimeError::BootIdentityUnavailable)?;
    if !is_canonical_boot_id(text) {
        return Err(PmCurrentRuntimeError::BootIdentityUnavailable);
    }
    Ok(text.into())
}

fn is_canonical_boot_id(value: &str) -> bool {
    value.len() == BOOT_ID_TEXT_BYTES
        && value.bytes().enumerate().all(|(index, byte)| {
            if matches!(index, 8 | 13 | 18 | 23) {
                byte == b'-'
            } else {
                byte.is_ascii_digit() || (b'a'..=b'f').contains(&byte)
            }
        })
}

#[cfg(target_os = "linux")]
fn effective_runtime_user() -> Result<(u32, Box<str>), PmCurrentRuntimeError> {
    let before = rustix::process::geteuid().as_raw();
    let name = lookup_runtime_user_with_fixed_getent(before)?;
    let after = rustix::process::geteuid().as_raw();
    if before != after {
        return Err(PmCurrentRuntimeError::RuntimeUserUnavailable);
    }
    Ok((before, name))
}

#[cfg(target_os = "linux")]
fn lookup_runtime_user_with_fixed_getent(
    effective_user_id: u32,
) -> Result<Box<str>, PmCurrentRuntimeError> {
    // The absolute helper path and cleared environment prevent caller PATH,
    // locale, or process-environment selection. NSS itself is a trusted,
    // potentially blocking host dependency; elapsed time is rejected by the
    // surrounding source-owned capture window.
    let mut child = Command::new(GETENT_PATH)
        .arg("passwd")
        .arg(effective_user_id.to_string())
        .env_clear()
        .env("LC_ALL", "C")
        .stdin(Stdio::null())
        .stdout(Stdio::piped())
        .stderr(Stdio::null())
        .spawn()
        .map_err(|_| PmCurrentRuntimeError::RuntimeUserUnavailable)?;
    let Some(stdout) = child.stdout.take() else {
        terminate_and_reap(&mut child);
        return Err(PmCurrentRuntimeError::RuntimeUserUnavailable);
    };
    let mut bounded = stdout.take((MAX_GETENT_STDOUT_BYTES + 1) as u64);
    let mut bytes = Vec::with_capacity(MAX_GETENT_STDOUT_BYTES + 1);
    if bounded.read_to_end(&mut bytes).is_err() {
        terminate_and_reap(&mut child);
        return Err(PmCurrentRuntimeError::RuntimeUserUnavailable);
    }
    drop(bounded);
    if bytes.len() > MAX_GETENT_STDOUT_BYTES {
        terminate_and_reap(&mut child);
        return Err(PmCurrentRuntimeError::RuntimeUserUnavailable);
    }
    let status = child
        .wait()
        .map_err(|_| PmCurrentRuntimeError::RuntimeUserUnavailable)?;
    if !status.success() {
        return Err(PmCurrentRuntimeError::RuntimeUserUnavailable);
    }
    parse_getent_passwd_record(&bytes, effective_user_id)
}

#[cfg(target_os = "linux")]
fn terminate_and_reap(child: &mut Child) {
    let _kill_result = child.kill();
    let _wait_result = child.wait();
}

fn parse_getent_passwd_record(
    bytes: &[u8],
    effective_user_id: u32,
) -> Result<Box<str>, PmCurrentRuntimeError> {
    if bytes.len() > MAX_GETENT_STDOUT_BYTES
        || bytes.last() != Some(&b'\n')
        || bytes[..bytes.len().saturating_sub(1)].contains(&b'\n')
        || bytes.contains(&b'\r')
        || bytes.contains(&0)
    {
        return Err(PmCurrentRuntimeError::RuntimeUserUnavailable);
    }
    let record = std::str::from_utf8(&bytes[..bytes.len() - 1])
        .map_err(|_| PmCurrentRuntimeError::RuntimeUserUnavailable)?;
    let mut fields = record.split(':');
    let name = fields
        .next()
        .ok_or(PmCurrentRuntimeError::RuntimeUserUnavailable)?;
    let _password = fields
        .next()
        .ok_or(PmCurrentRuntimeError::RuntimeUserUnavailable)?;
    let user_id = fields
        .next()
        .ok_or(PmCurrentRuntimeError::RuntimeUserUnavailable)?;
    let group_id = fields
        .next()
        .ok_or(PmCurrentRuntimeError::RuntimeUserUnavailable)?;
    let _gecos = fields
        .next()
        .ok_or(PmCurrentRuntimeError::RuntimeUserUnavailable)?;
    let _home = fields
        .next()
        .ok_or(PmCurrentRuntimeError::RuntimeUserUnavailable)?;
    let _shell = fields
        .next()
        .ok_or(PmCurrentRuntimeError::RuntimeUserUnavailable)?;
    let canonical_group_id = group_id
        .parse::<u32>()
        .is_ok_and(|parsed| group_id == parsed.to_string());
    if fields.next().is_some()
        || name.is_empty()
        || name.len() > MAX_RUNTIME_USER_BYTES
        || name.bytes().any(|byte| {
            !(byte.is_ascii_alphanumeric() || matches!(byte, b'-' | b'_' | b'.' | b'/'))
        })
        || user_id != effective_user_id.to_string()
        || !canonical_group_id
    {
        return Err(PmCurrentRuntimeError::RuntimeUserUnavailable);
    }
    Ok(name.into())
}

#[cfg(target_os = "linux")]
fn capture_current_executable(
    expected_binary_length: u64,
) -> Result<ExecutableCustody, PmCurrentRuntimeError> {
    if expected_binary_length == 0 {
        return Err(PmCurrentRuntimeError::ExecutableLengthMismatch);
    }
    let mut file = open_proc_self_executable()?;
    let before = executable_file_identity(&file)?;
    if before.length != expected_binary_length {
        return Err(PmCurrentRuntimeError::ExecutableLengthMismatch);
    }
    let (sha256, length) = hash_executable(&mut file, expected_binary_length)?;
    let after = executable_file_identity(&file)?;
    if before != after || length != before.length {
        return Err(PmCurrentRuntimeError::ExecutableChanged);
    }
    Ok(ExecutableCustody {
        file,
        identity: after,
        sha256,
        sha256_hex: lower_hex(&sha256).into(),
        length,
    })
}

#[cfg(target_os = "linux")]
fn open_proc_self_executable() -> Result<File, PmCurrentRuntimeError> {
    let proc = open_proc_directory()?;
    // `/proc/self/exe` is intentionally a procfs magic link. `O_NOFOLLOW`
    // would reject the only kernel-provided running-image handle.
    let executable = rustix::fs::openat(
        &proc,
        PROC_SELF_EXE_ENTRY,
        rustix::fs::OFlags::RDONLY | rustix::fs::OFlags::CLOEXEC,
        rustix::fs::Mode::empty(),
    )
    .map_err(|_| PmCurrentRuntimeError::ExecutableUnavailable)?;
    Ok(File::from(executable))
}

#[cfg(target_os = "linux")]
fn open_proc_directory() -> Result<std::os::fd::OwnedFd, PmCurrentRuntimeError> {
    let proc = rustix::fs::open(
        PROC_ROOT_PATH,
        rustix::fs::OFlags::RDONLY | rustix::fs::OFlags::DIRECTORY | rustix::fs::OFlags::CLOEXEC,
        rustix::fs::Mode::empty(),
    )
    .map_err(|_| PmCurrentRuntimeError::ProcfsUnavailable)?;
    let filesystem =
        rustix::fs::fstatfs(&proc).map_err(|_| PmCurrentRuntimeError::ProcfsUnavailable)?;
    if filesystem.f_type != rustix::fs::PROC_SUPER_MAGIC {
        return Err(PmCurrentRuntimeError::ProcfsUnavailable);
    }
    Ok(proc)
}

#[cfg(target_os = "linux")]
fn executable_file_identity(file: &File) -> Result<ExecutableFileIdentity, PmCurrentRuntimeError> {
    use std::os::unix::fs::MetadataExt as _;

    let metadata = file
        .metadata()
        .map_err(|_| PmCurrentRuntimeError::ExecutableUnavailable)?;
    if !metadata.file_type().is_file() || metadata.len() == 0 || metadata.nlink() == 0 {
        return Err(PmCurrentRuntimeError::ExecutableIdentityInvalid);
    }
    Ok(ExecutableFileIdentity {
        device: metadata.dev(),
        inode: metadata.ino(),
        mode: metadata.mode(),
        owner_user_id: metadata.uid(),
        owner_group_id: metadata.gid(),
        link_count: metadata.nlink(),
        length: metadata.len(),
        modified_seconds: metadata.mtime(),
        modified_nanoseconds: metadata.mtime_nsec(),
        changed_seconds: metadata.ctime(),
        changed_nanoseconds: metadata.ctime_nsec(),
    })
}

#[cfg(not(target_os = "linux"))]
fn executable_file_identity(_file: &File) -> Result<ExecutableFileIdentity, PmCurrentRuntimeError> {
    Err(PmCurrentRuntimeError::UnsupportedPlatform)
}

#[cfg(target_os = "linux")]
fn hash_executable(
    file: &mut File,
    expected_length: u64,
) -> Result<([u8; 32], u64), PmCurrentRuntimeError> {
    file.seek(SeekFrom::Start(0))
        .map_err(|_| PmCurrentRuntimeError::ExecutableUnavailable)?;
    let mut digest = Sha256::new();
    let mut buffer = [0_u8; 64 * 1024];
    let mut total = 0_u64;
    loop {
        let count = file
            .read(&mut buffer)
            .map_err(|_| PmCurrentRuntimeError::ExecutableUnavailable)?;
        if count == 0 {
            break;
        }
        total = total
            .checked_add(u64::try_from(count).map_err(|_| PmCurrentRuntimeError::ClockOverflow)?)
            .ok_or(PmCurrentRuntimeError::ExecutableLengthMismatch)?;
        if total > expected_length {
            return Err(PmCurrentRuntimeError::ExecutableLengthMismatch);
        }
        digest.update(&buffer[..count]);
    }
    if total != expected_length {
        return Err(PmCurrentRuntimeError::ExecutableLengthMismatch);
    }
    Ok((digest.finalize().into(), total))
}

fn lower_hex(value: &[u8; 32]) -> String {
    let mut encoded = String::with_capacity(64);
    for byte in value {
        use std::fmt::Write as _;
        write!(&mut encoded, "{byte:02x}").expect("writing to a String cannot fail");
    }
    encoded
}

#[derive(Debug, Error)]
pub(super) enum PmCurrentRuntimeError {
    #[error("current-runtime evidence is restricted to TrialPhase::APlaceCancel")]
    TrialPhaseMismatch,
    #[error("current-runtime observation is supported only on Linux")]
    UnsupportedPlatform,
    #[error("trusted Linux procfs is unavailable")]
    ProcfsUnavailable,
    #[error("the running executable descriptor is unavailable")]
    ExecutableUnavailable,
    #[error("the running executable descriptor identity is invalid")]
    ExecutableIdentityInvalid,
    #[error("the running executable length differs from authorization")]
    ExecutableLengthMismatch,
    #[error("the running executable changed during observation")]
    ExecutableChanged,
    #[error("the Linux UTS host identity is unavailable")]
    HostIdentityUnavailable,
    #[error("the Linux boot identity is unavailable")]
    BootIdentityUnavailable,
    #[error("the effective runtime NSS user is unavailable or unstable")]
    RuntimeUserUnavailable,
    #[error("the observing process identity changed")]
    ProcessIdentityChanged,
    #[error("the local runtime identity changed during observation")]
    LocalRuntimeChanged,
    #[error("the local monotonic clock regressed")]
    MonotonicClockRegression,
    #[error("the local wall clock regressed")]
    WallClockRegression,
    #[error("the local clock value overflowed")]
    ClockOverflow,
    #[error("the current-runtime observation exceeded its maximum age")]
    ObservationExpired,
    #[error("the typed geoblock source reports placement blocked")]
    GeoblockBlocked,
    #[error("the typed geoblock wall observation is from the future")]
    GeoblockFromFuture,
    #[error("the typed geoblock wall observation is stale")]
    GeoblockExpired,
    #[error("canonical config, authorization, release, host, or egress binding differs")]
    AuthorizationBinding,
    #[error("the full conservative cleanup budget no longer fits its authorization window")]
    CleanupWindow,
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn boot_identity_parser_is_exact() {
        assert!(is_canonical_boot_id("01234567-89ab-cdef-0123-456789abcdef"));
        assert!(!is_canonical_boot_id(
            "01234567-89AB-CDEF-0123-456789ABCDEF"
        ));
        assert!(!is_canonical_boot_id("0123456789ab-cdef-0123-456789abcdef"));
    }

    #[test]
    fn getent_passwd_parser_requires_one_exact_uid_record() {
        assert_eq!(
            parse_getent_passwd_record(b"trial-user:x:1000:1000::/home/trial:/bin/sh\n", 1000)
                .unwrap()
                .as_ref(),
            "trial-user",
        );
        for invalid in [
            b"trial-user:x:1001:1000::/home/trial:/bin/sh\n".as_slice(),
            b"trial-user:x:01000:1000::/home/trial:/bin/sh\n".as_slice(),
            b"trial user:x:1000:1000::/home/trial:/bin/sh\n".as_slice(),
            b"trial\tuser:x:1000:1000::/home/trial:/bin/sh\n".as_slice(),
            b"trial\0user:x:1000:1000::/home/trial:/bin/sh\n".as_slice(),
            b"trial-user:x:1000:1000::/home/trial:/bin/sh\r\n".as_slice(),
            b"trial-user:x:1000:01000::/home/trial:/bin/sh\n".as_slice(),
            b"trial-user:x:1000:1000::/home/trial:/bin/sh".as_slice(),
            b"trial-user:x:1000:1000::/home/trial:/bin/sh\nsecond:x:1000:1000::/:/bin/sh\n"
                .as_slice(),
        ] {
            assert!(parse_getent_passwd_record(invalid, 1000).is_err());
        }
        let mut oversized = vec![b'a'; MAX_GETENT_STDOUT_BYTES];
        oversized.push(b'\n');
        assert!(parse_getent_passwd_record(&oversized, 1000).is_err());
    }

    #[cfg(target_os = "linux")]
    #[test]
    fn local_sources_capture_only_the_current_linux_runtime() {
        let identity = observe_local_runtime_identity().unwrap();
        assert_eq!(identity.process_id, std::process::id());
        assert_eq!(
            identity.effective_user_id,
            rustix::process::geteuid().as_raw(),
        );
        assert!(!identity.host_identity.is_empty());
        assert!(is_canonical_boot_id(identity.boot_identity.as_ref()));
        assert!(!identity.runtime_user.is_empty());

        let file = open_proc_self_executable().unwrap();
        let length = executable_file_identity(&file).unwrap().length;
        let capture = capture_local_runtime(length).unwrap();
        assert_eq!(capture.process_id, std::process::id());
        assert_eq!(
            capture.effective_user_id,
            rustix::process::geteuid().as_raw(),
        );
        assert!(unix_nanoseconds(capture.wall_completed).unwrap() > 0);
        assert!(capture.monotonic_completed >= capture.monotonic_started);
        capture.executable.revalidate_held_identity().unwrap();
    }

    #[cfg(target_os = "linux")]
    #[test]
    fn executable_recheck_rejects_identity_hash_and_length_drift() {
        fn capture() -> ExecutableCustody {
            let file = open_proc_self_executable().unwrap();
            let length = executable_file_identity(&file).unwrap().length;
            capture_current_executable(length).unwrap()
        }

        let expected = capture();
        let current = capture();
        validate_executable_match(&expected, &current).unwrap();

        let mut identity_drift = capture();
        identity_drift.identity.inode ^= 1;
        assert!(matches!(
            validate_executable_match(&expected, &identity_drift),
            Err(PmCurrentRuntimeError::ExecutableChanged),
        ));

        let mut hash_drift = capture();
        hash_drift.sha256[0] ^= 1;
        assert!(matches!(
            validate_executable_match(&expected, &hash_drift),
            Err(PmCurrentRuntimeError::ExecutableChanged),
        ));

        let mut length_drift = capture();
        length_drift.length = length_drift.length.checked_add(1).unwrap();
        assert!(matches!(
            validate_executable_match(&expected, &length_drift),
            Err(PmCurrentRuntimeError::ExecutableChanged),
        ));
    }

    #[test]
    fn local_runtime_recheck_rejects_each_identity_drift() {
        fn identity<'a>(
            process_id: u32,
            effective_user_id: u32,
            host_identity: &'a str,
            boot_identity: &'a str,
            runtime_user: &'a str,
        ) -> LocalRuntimeIdentityView<'a> {
            LocalRuntimeIdentityView {
                process_id,
                effective_user_id,
                host_identity,
                boot_identity,
                runtime_user,
            }
        }

        validate_local_runtime_identity(
            identity(7, 1000, "host-a", "boot-a", "user-a"),
            identity(7, 1000, "host-a", "boot-a", "user-a"),
            7,
        )
        .unwrap();
        for current in [
            identity(8, 1000, "host-a", "boot-a", "user-a"),
            identity(7, 1001, "host-a", "boot-a", "user-a"),
            identity(7, 1000, "host-b", "boot-a", "user-a"),
            identity(7, 1000, "host-a", "boot-b", "user-a"),
            identity(7, 1000, "host-a", "boot-a", "user-b"),
        ] {
            assert!(matches!(
                validate_local_runtime_identity(
                    identity(7, 1000, "host-a", "boot-a", "user-a"),
                    current,
                    7,
                ),
                Err(PmCurrentRuntimeError::LocalRuntimeChanged),
            ));
        }
        assert!(matches!(
            validate_local_runtime_identity(
                identity(7, 1000, "host-a", "boot-a", "user-a"),
                identity(7, 1000, "host-a", "boot-a", "user-a"),
                8,
            ),
            Err(PmCurrentRuntimeError::LocalRuntimeChanged),
        ));
    }

    #[test]
    fn clock_window_rejects_regression_and_expires_after_exact_boundary() {
        let wall = UNIX_EPOCH + Duration::from_secs(1_000);
        let monotonic = Instant::now();
        let maximum_age = Duration::from_millis(10);
        validate_age_window(
            wall,
            monotonic,
            wall + maximum_age,
            monotonic + maximum_age,
            maximum_age,
        )
        .unwrap();
        assert!(matches!(
            validate_age_window(
                wall,
                monotonic + Duration::from_nanos(1),
                wall,
                monotonic,
                maximum_age,
            ),
            Err(PmCurrentRuntimeError::MonotonicClockRegression),
        ));
        assert!(matches!(
            validate_age_window(
                wall + Duration::from_nanos(1),
                monotonic,
                wall,
                monotonic,
                maximum_age,
            ),
            Err(PmCurrentRuntimeError::WallClockRegression),
        ));
        assert!(matches!(
            validate_age_window(
                wall,
                monotonic,
                wall + maximum_age + Duration::from_nanos(1),
                monotonic + maximum_age,
                maximum_age,
            ),
            Err(PmCurrentRuntimeError::ObservationExpired),
        ));
        assert!(matches!(
            validate_age_window(
                wall,
                monotonic,
                wall + maximum_age,
                monotonic + maximum_age + Duration::from_nanos(1),
                maximum_age,
            ),
            Err(PmCurrentRuntimeError::ObservationExpired),
        ));
    }

    #[test]
    fn geoblock_values_reject_blocked_future_stale_and_wrong_egress() {
        let now = UNIX_EPOCH + Duration::from_secs(1_000);
        let now_ns = unix_nanoseconds(now).unwrap();
        let maximum_age = Duration::from_millis(10);
        let maximum_age_ns = u64::try_from(maximum_age.as_nanos()).unwrap();
        validate_geoblock_values(false, now_ns - maximum_age_ns, now, maximum_age).unwrap();
        assert!(matches!(
            validate_geoblock_values(true, now_ns, now, maximum_age),
            Err(PmCurrentRuntimeError::GeoblockBlocked),
        ));
        assert!(matches!(
            validate_geoblock_values(false, now_ns + 1, now, maximum_age),
            Err(PmCurrentRuntimeError::GeoblockFromFuture),
        ));
        assert!(matches!(
            validate_geoblock_values(false, now_ns - maximum_age_ns - 1, now, maximum_age),
            Err(PmCurrentRuntimeError::GeoblockExpired),
        ));
        validate_authorized_egress("192.0.2.1".parse().unwrap(), "192.0.2.1").unwrap();
        assert!(matches!(
            validate_authorized_egress("192.0.2.2".parse().unwrap(), "192.0.2.1"),
            Err(PmCurrentRuntimeError::AuthorizationBinding),
        ));
    }

    #[test]
    fn config_authorization_pins_reject_each_swap() {
        fn pins<'a>(
            config_sha256: &'a str,
            authorization_id: &'a str,
        ) -> ConfigAuthorizationPinsView<'a> {
            ConfigAuthorizationPinsView {
                config_sha256,
                config_length: 42,
                config_fingerprint: "config-fingerprint",
                trial_plan_fingerprint: "plan-fingerprint",
                authorization_id,
                authorization_fingerprint: "authorization-fingerprint",
            }
        }

        validate_matching_pins(pins("config-a", "auth-a"), pins("config-a", "auth-a")).unwrap();
        assert!(matches!(
            validate_matching_pins(pins("config-a", "auth-a"), pins("config-b", "auth-a")),
            Err(PmCurrentRuntimeError::AuthorizationBinding),
        ));
        assert!(matches!(
            validate_matching_pins(pins("config-a", "auth-a"), pins("config-a", "auth-b")),
            Err(PmCurrentRuntimeError::AuthorizationBinding),
        ));
        let mut config_swap = pins("config-a", "auth-a");
        config_swap.config_fingerprint = "different-config-fingerprint";
        assert!(matches!(
            validate_matching_pins(pins("config-a", "auth-a"), config_swap),
            Err(PmCurrentRuntimeError::AuthorizationBinding),
        ));
        let mut authorization_swap = pins("config-a", "auth-a");
        authorization_swap.authorization_fingerprint = "different-authorization-fingerprint";
        assert!(matches!(
            validate_matching_pins(pins("config-a", "auth-a"), authorization_swap),
            Err(PmCurrentRuntimeError::AuthorizationBinding),
        ));
    }

    #[test]
    fn cleanup_runway_accepts_exact_boundary_and_rejects_shortfall() {
        let current = UNIX_EPOCH + Duration::from_secs(1_000);
        let budget = Duration::from_secs(120);
        let cleanup = current + budget;
        validate_cleanup_runway(current, cleanup, budget).unwrap();
        assert!(matches!(
            validate_cleanup_runway(current, cleanup - Duration::from_nanos(1), budget,),
            Err(PmCurrentRuntimeError::CleanupWindow),
        ));
        assert!(matches!(
            validate_cleanup_runway(current, current - Duration::from_nanos(1), Duration::ZERO),
            Err(PmCurrentRuntimeError::CleanupWindow),
        ));
    }

    #[test]
    fn current_runtime_rejects_every_non_place_cancel_phase_pair() {
        validate_place_cancel_phase(TrialPhase::APlaceCancel, TrialPhase::APlaceCancel).unwrap();
        for (config_phase, authorization_phase) in [
            (TrialPhase::BFillPosition, TrialPhase::APlaceCancel),
            (TrialPhase::APlaceCancel, TrialPhase::BFillPosition),
            (TrialPhase::BFillPosition, TrialPhase::BFillPosition),
        ] {
            assert!(matches!(
                validate_place_cancel_phase(config_phase, authorization_phase),
                Err(PmCurrentRuntimeError::TrialPhaseMismatch),
            ));
        }
    }

    #[test]
    fn debug_is_redacted() {
        let debug = format!("{:?}", PmCurrentRuntimeObserver::system());
        assert!(!debug.contains(&std::process::id().to_string()));
        assert!(debug.contains("system-local-source"));
    }
}
