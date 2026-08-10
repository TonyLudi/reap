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
    rc::Rc,
    time::{Duration, Instant, SystemTime, UNIX_EPOCH},
};

use chrono::{DateTime, SecondsFormat, Utc};
use reap_pm_controlled_trial::{
    AuthorizationHostBinding, AuthorizationRuntimeBinding, CanonicalAuthorization,
    CanonicalOnlineAuthorizationV2, CanonicalOnlinePolicyV2, CanonicalTrialConfig,
    OnlineAuthorizationRuntimeBindingV2, PmAuthorizationConsumptionError,
    PmOnlineAuthorizationConsumptionV2Error, PreparedAuthorizationConsumption,
    PreparedOnlineAuthorizationConsumptionV2, TrialPhase, prepare_authorization_consumption,
    prepare_online_authorization_consumption_v2, verify_authorization,
};
use reap_pm_controlled_trial_live::{
    PmControlledTrialLiveJournals, PmDurablePlacePreparedAckV1,
    PmPendingPhaseAOnlinePreflightBasisV2, PmPhaseAOnlinePreflightDispatchOwnerV2,
    PmPhaseAOnlinePreflightEvidenceManifestV2, PmPhaseAOnlinePreflightV2Error,
    PmPhaseAPlaceDefinitelyNotDispatchedV1, create_phase_a_online_preflight_basis_v2,
};
use reap_polymarket_live_adapter::{
    PmGeoblockObservationCommitment, PmHttpReceiveClock, PmProductionGeoblockObservation,
};
use sha2::{Digest as _, Sha256};
use thiserror::Error;

use super::{
    linux_egress_local_facts::{PmLinuxEgressLocalFactCustody, PmLinuxEgressLocalFactError},
    online_preflight::PmDeniedOnlinePreflightCandidate,
};

mod selected_egress;

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

impl PmCurrentRuntimeObserver {
    /// Begin the sole outer Phase-A online preflight clock window before the
    /// first source operation. Every identity and clock is observed locally.
    pub(super) fn begin_phase_a_online_preflight(
        &mut self,
        config: &CanonicalTrialConfig,
        v1_authorization: &CanonicalAuthorization,
        policy: &CanonicalOnlinePolicyV2,
        online_authorization: &CanonicalOnlineAuthorizationV2,
    ) -> Result<PmPhaseAOnlinePreflightWindow, PmPhaseAOnlineRuntimeError> {
        self.validate_observer_process()?;
        let monotonic_started = Instant::now();
        let wall_started = SystemTime::now();
        let source_thread_id = current_runtime_thread_id()?;
        let effective_user_id = current_effective_user_id()?;
        let maximum_age_ms = validate_exact_online_inputs(
            config,
            v1_authorization,
            policy,
            online_authorization,
            wall_started,
        )?;
        Ok(PmPhaseAOnlinePreflightWindow {
            pins: OnlineRuntimePins::capture(
                config,
                v1_authorization,
                policy,
                online_authorization,
            ),
            creating_process_id: self.creating_process_id,
            source_thread_id,
            effective_user_id,
            wall_started,
            monotonic_started,
            maximum_age: Duration::from_millis(maximum_age_ms),
            maximum_age_ms,
        })
    }

    /// Finish the outer window after the last preflight source operation.
    /// The returned value remains denied and cannot expose runtime bindings.
    pub(super) fn finish_phase_a_online_preflight(
        &mut self,
        window: PmPhaseAOnlinePreflightWindow,
        config: &CanonicalTrialConfig,
        v1_authorization: &CanonicalAuthorization,
        policy: &CanonicalOnlinePolicyV2,
        online_authorization: &CanonicalOnlineAuthorizationV2,
    ) -> Result<PmFinishedPhaseAOnlinePreflightWindow, PmPhaseAOnlineRuntimeError> {
        self.validate_observer_process()?;
        window
            .pins
            .validate(config, v1_authorization, policy, online_authorization)?;
        validate_online_source_identity(
            window.creating_process_id,
            window.source_thread_id,
            window.effective_user_id,
        )?;
        let wall_completed = SystemTime::now();
        let monotonic_completed = Instant::now();
        validate_age_window(
            window.wall_started,
            window.monotonic_started,
            wall_completed,
            monotonic_completed,
            window.maximum_age,
        )?;
        let maximum_age_ms = validate_exact_online_inputs(
            config,
            v1_authorization,
            policy,
            online_authorization,
            wall_completed,
        )?;
        if maximum_age_ms != window.maximum_age_ms {
            return Err(PmCurrentRuntimeError::AuthorizationBinding.into());
        }
        let started_at_utc = canonical_utc(window.wall_started)?.into();
        let completed_at_utc = canonical_utc(wall_completed)?.into();
        Ok(PmFinishedPhaseAOnlinePreflightWindow {
            open: window,
            wall_completed,
            monotonic_completed,
            started_at_utc,
            completed_at_utc,
        })
    }

    /// Consume a closed outer window, same-thread local-egress custody, and a
    /// geoblock response sealed by the selected-egress actor. No generic
    /// production observation can enter this positive V2 typestate.
    #[allow(clippy::too_many_arguments)]
    pub(super) fn observe_phase_a_online_current_runtime(
        &mut self,
        window: PmFinishedPhaseAOnlinePreflightWindow,
        config: &CanonicalTrialConfig,
        v1_authorization: &CanonicalAuthorization,
        policy: &CanonicalOnlinePolicyV2,
        online_authorization: &CanonicalOnlineAuthorizationV2,
        selected_geoblock: PmSelectedEgressGeoblockObservation<PmLinuxEgressLocalFactCustody>,
    ) -> Result<PmPhaseAOnlineCurrentRuntimeWitness, PmPhaseAOnlineRuntimeError> {
        self.validate_observer_process()?;
        let PmFinishedPhaseAOnlinePreflightWindow {
            open,
            wall_completed,
            monotonic_completed,
            started_at_utc,
            completed_at_utc,
        } = window;
        open.pins
            .validate(config, v1_authorization, policy, online_authorization)?;
        validate_online_source_identity(
            open.creating_process_id,
            open.source_thread_id,
            open.effective_user_id,
        )?;
        let (local_egress, geoblock) = consume_selected_geoblock(selected_geoblock)?;
        local_egress.validate_captured_window(
            online_authorization,
            open.wall_started,
            wall_completed,
            open.monotonic_started,
            monotonic_completed,
            open.maximum_age,
        )?;
        validate_production_geoblock_inside_window(
            &geoblock,
            open.wall_started,
            wall_completed,
            open.maximum_age,
        )?;
        let v1 = self.observe_phase_a_place(config, v1_authorization, geoblock)?;
        let PmPhaseAPlaceCurrentRuntimeWitness { mut evidence } = v1;
        evidence.maximum_age = open.maximum_age;
        evidence.maximum_age_ms = open.maximum_age_ms;
        validate_age_window(
            open.wall_started,
            open.monotonic_started,
            evidence.checked_wall_complete,
            evidence.checked_monotonic_complete,
            open.maximum_age,
        )?;
        validate_exact_v2_runtime_binding(
            &evidence,
            config,
            v1_authorization,
            policy,
            online_authorization,
        )?;
        Ok(PmPhaseAOnlineCurrentRuntimeWitness {
            evidence: PmPhaseAOnlineRuntimeEvidence {
                current: evidence,
                window: FinishedOnlineWindowEvidence {
                    pins: open.pins,
                    creating_process_id: open.creating_process_id,
                    source_thread_id: open.source_thread_id,
                    effective_user_id: open.effective_user_id,
                    wall_started: open.wall_started,
                    monotonic_started: open.monotonic_started,
                    wall_completed,
                    monotonic_completed,
                    started_at_utc,
                    completed_at_utc,
                    maximum_age: open.maximum_age,
                    maximum_age_ms: open.maximum_age_ms,
                },
                local_egress,
                prior_selected_geoblocks: Vec::new(),
            },
        })
    }

    /// Perform a fresh selected-egress/runtime recheck, construct both freely
    /// constructible library binding records only as stack locals, and create
    /// the V2 Prepared ledger before the V1 Prepared ledger. The returned pair
    /// retains all source and descriptor custody and exposes no binding getter.
    #[allow(clippy::too_many_arguments)]
    pub(super) fn prepare_phase_a_online_consumptions(
        &mut self,
        selected_geoblock: PmSelectedEgressGeoblockObservation<PmPhaseAOnlineCurrentRuntimeWitness>,
        config: &CanonicalTrialConfig,
        v1_authorization: &CanonicalAuthorization,
        policy: CanonicalOnlinePolicyV2,
        online_authorization: CanonicalOnlineAuthorizationV2,
    ) -> Result<PmPhaseAOnlinePreparedConsumptionPair, PmPhaseAOnlineRuntimeError> {
        let (witness, selected_geoblock) = consume_selected_geoblock(selected_geoblock)?;
        let PmPhaseAOnlineCurrentRuntimeWitness { evidence } = witness;
        let mut runtime = self.recheck_phase_a_online_evidence(
            evidence,
            config,
            v1_authorization,
            &policy,
            &online_authorization,
            GeoblockEvidence::from_source(selected_geoblock),
        )?;
        let (v1_runtime, v2_runtime) =
            exact_consumption_runtime_bindings(&mut runtime.evidence, &online_authorization)?;
        let mut v2_consumption = prepare_online_authorization_consumption_v2(
            config,
            policy,
            online_authorization,
            &v2_runtime,
        )
        .map_err(PmPhaseAOnlineRuntimeError::V2Consumption)?;
        let v1_consumption =
            prepare_authorization_consumption(config, v1_authorization, &v1_runtime)
                .map_err(PmPhaseAOnlineRuntimeError::V1Consumption)?;
        // V1 Prepared creation is a new directory entry after V2 Prepared.
        v2_consumption
            .refresh_after_bound_artifact_create()
            .map_err(PmPhaseAOnlineRuntimeError::V2Consumption)?;
        Ok(PmPhaseAOnlinePreparedConsumptionPair {
            runtime,
            v1_consumption,
            v2_consumption,
        })
    }

    /// Consume the complete denied source candidate and internally assemble
    /// the exact public sidecar manifest. No caller-supplied manifest or
    /// digest enters this path.
    #[allow(clippy::too_many_arguments)]
    pub(super) fn create_phase_a_online_preflight_basis(
        &mut self,
        selected_pair: PmSelectedEgressGeoblockObservation<PmPhaseAOnlinePreparedConsumptionPair>,
        candidate: PmDeniedOnlinePreflightCandidate,
        config: &CanonicalTrialConfig,
        v1_authorization: &CanonicalAuthorization,
        journals: &mut PmControlledTrialLiveJournals,
        prepared: PmDurablePlacePreparedAckV1,
    ) -> Result<PmPendingPhaseAOnlineRuntimeBurnV2, PmPhaseAOnlineRuntimeError> {
        let (pair, selected_geoblock) = consume_selected_geoblock(selected_pair)?;
        let PmPhaseAOnlinePreparedConsumptionPair {
            runtime,
            v1_consumption,
            v2_consumption,
        } = pair;
        let policy = v2_consumption.policy();
        let online_authorization = v2_consumption.authorization();
        let candidate = candidate
            .revalidate_non_authoritative(config, policy)
            .map_err(|_| PmPhaseAOnlineRuntimeError::CandidateManifest)?;
        let mut runtime = self.recheck_phase_a_online_evidence(
            runtime.evidence,
            config,
            v1_authorization,
            policy,
            online_authorization,
            GeoblockEvidence::from_source(selected_geoblock),
        )?;
        validate_candidate_inside_outer_window(
            &candidate,
            &runtime.evidence.window,
            online_authorization,
        )?;
        let (v1_runtime, v2_runtime) =
            exact_consumption_runtime_bindings(&mut runtime.evidence, online_authorization)?;
        let evidence = assemble_online_preflight_manifest(
            &candidate,
            &runtime.evidence,
            policy,
            online_authorization,
            &v1_runtime,
            &v2_runtime,
        )?;
        let pending = create_phase_a_online_preflight_basis_v2(
            config,
            v1_authorization,
            journals,
            prepared,
            v1_consumption,
            v2_consumption,
            evidence,
        )
        .map_err(PmPhaseAOnlineRuntimeError::OnlinePreflight)?;
        Ok(PmPendingPhaseAOnlineRuntimeBurnV2 {
            pending,
            runtime,
            candidate,
        })
    }

    /// Burn V2 then V1 through the live sidecar owner only after another
    /// selected-egress source recheck. Success retains the complete dispatch,
    /// candidate, runtime, window, geoblock history, and egress custody for a
    /// separate final consuming edge.
    pub(super) fn burn_phase_a_online_preflight_a3(
        &mut self,
        selected_pending: PmSelectedEgressGeoblockObservation<PmPendingPhaseAOnlineRuntimeBurnV2>,
        config: &CanonicalTrialConfig,
        v1_authorization: &CanonicalAuthorization,
        journals: &mut PmControlledTrialLiveJournals,
    ) -> Result<PmPhaseAOnlineRuntimeBurnedV2, PmPhaseAOnlineRuntimeError> {
        let (pending_owner, selected_geoblock) = consume_selected_geoblock(selected_pending)?;
        let PmPendingPhaseAOnlineRuntimeBurnV2 {
            pending,
            runtime,
            candidate,
        } = pending_owner;
        let policy = pending.online_policy();
        let online_authorization = pending.online_authorization();
        let candidate = candidate
            .revalidate_non_authoritative(config, policy)
            .map_err(|_| PmPhaseAOnlineRuntimeError::CandidateManifest)?;
        let mut runtime = self.recheck_phase_a_online_evidence(
            runtime.evidence,
            config,
            v1_authorization,
            policy,
            online_authorization,
            GeoblockEvidence::from_source(selected_geoblock),
        )?;
        let (v1_runtime, v2_runtime) =
            exact_consumption_runtime_bindings(&mut runtime.evidence, online_authorization)?;
        let dispatch = pending
            .burn_and_record_a3(journals, config, v1_authorization, &v1_runtime, &v2_runtime)
            .map_err(PmPhaseAOnlineRuntimeError::OnlinePreflight)?;
        Ok(PmPhaseAOnlineRuntimeBurnedV2 {
            dispatch,
            runtime,
            candidate,
        })
    }

    #[allow(clippy::too_many_arguments)]
    fn recheck_phase_a_online_evidence(
        &mut self,
        mut evidence: PmPhaseAOnlineRuntimeEvidence,
        config: &CanonicalTrialConfig,
        v1_authorization: &CanonicalAuthorization,
        policy: &CanonicalOnlinePolicyV2,
        online_authorization: &CanonicalOnlineAuthorizationV2,
        selected: GeoblockEvidence,
    ) -> Result<PmRevalidatedPhaseAOnlineCurrentRuntimeWitness, PmPhaseAOnlineRuntimeError> {
        self.validate_observer_process()?;
        evidence
            .window
            .pins
            .validate(config, v1_authorization, policy, online_authorization)?;
        validate_online_source_identity(
            evidence.window.creating_process_id,
            evidence.window.source_thread_id,
            evidence.window.effective_user_id,
        )?;
        validate_fresh_selected_geoblock(
            &selected,
            evidence.current.checked_wall_complete,
            evidence.window.maximum_age,
        )?;
        let previous = std::mem::replace(&mut evidence.current.geoblock, selected);
        evidence.prior_selected_geoblocks.push(previous);
        let v1 = self.recheck_phase_a_place(
            PmPhaseAPlaceCurrentRuntimeWitness {
                evidence: evidence.current,
            },
            config,
            v1_authorization,
        )?;
        let PmRevalidatedPhaseAPlaceCurrentRuntimeWitness { evidence: current } = v1;
        evidence.current = current;
        validate_age_window(
            evidence.window.wall_started,
            evidence.window.monotonic_started,
            evidence.current.checked_wall_complete,
            evidence.current.checked_monotonic_complete,
            evidence.window.maximum_age,
        )?;
        let maximum_age_ms = validate_exact_online_inputs(
            config,
            v1_authorization,
            policy,
            online_authorization,
            evidence.current.checked_wall_complete,
        )?;
        if maximum_age_ms != evidence.window.maximum_age_ms {
            return Err(PmCurrentRuntimeError::AuthorizationBinding.into());
        }
        validate_exact_v2_runtime_binding(
            &evidence.current,
            config,
            v1_authorization,
            policy,
            online_authorization,
        )?;
        let (egress_wall_completed, egress_monotonic_completed) = {
            let facts = evidence.local_egress.revalidate_for_current_runtime(
                online_authorization,
                evidence.window.wall_started,
                evidence.window.wall_completed,
                evidence.window.monotonic_started,
                evidence.window.monotonic_completed,
                evidence.window.maximum_age,
            )?;
            (facts.wall_completed(), facts.monotonic_completed())
        };
        evidence.current.executable.revalidate_held_identity()?;
        validate_online_source_identity(
            evidence.window.creating_process_id,
            evidence.window.source_thread_id,
            evidence.window.effective_user_id,
        )?;
        validate_geoblock(
            &evidence.current.geoblock,
            egress_wall_completed,
            evidence.window.maximum_age,
        )?;
        validate_age_window(
            evidence.window.wall_started,
            evidence.window.monotonic_started,
            egress_wall_completed,
            egress_monotonic_completed,
            evidence.window.maximum_age,
        )?;
        let _ = validate_exact_online_inputs(
            config,
            v1_authorization,
            policy,
            online_authorization,
            egress_wall_completed,
        )?;
        evidence.current.checked_wall_complete = egress_wall_completed;
        evidence.current.checked_monotonic_complete = egress_monotonic_completed;
        evidence.current.checked_wall_capture_unix_ns = unix_nanoseconds(egress_wall_completed)?;
        evidence.current.checked_at_utc = canonical_utc(egress_wall_completed)?.into();
        Ok(PmRevalidatedPhaseAOnlineCurrentRuntimeWitness { evidence })
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

/// Open, move-only outer window. The observer is the sole clock source; this
/// value is denied evidence and has no fact or runtime-binding projection.
#[must_use = "the source-owned online preflight window must be finished or dropped"]
pub(super) struct PmPhaseAOnlinePreflightWindow {
    pins: OnlineRuntimePins,
    creating_process_id: u32,
    source_thread_id: u32,
    effective_user_id: u32,
    wall_started: SystemTime,
    monotonic_started: Instant,
    maximum_age: Duration,
    maximum_age_ms: u64,
}

impl fmt::Debug for PmPhaseAOnlinePreflightWindow {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str("PmPhaseAOnlinePreflightWindow(<move-only; open; denied>)")
    }
}

/// Closed outer window. It still grants no preparation, HMAC, dispatch, or
/// transport authority. Only the exact V2 runtime join can consume it.
#[must_use = "a finished online preflight window is denied evidence, not authority"]
pub(super) struct PmFinishedPhaseAOnlinePreflightWindow {
    open: PmPhaseAOnlinePreflightWindow,
    wall_completed: SystemTime,
    monotonic_completed: Instant,
    started_at_utc: Box<str>,
    completed_at_utc: Box<str>,
}

impl fmt::Debug for PmFinishedPhaseAOnlinePreflightWindow {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str("PmFinishedPhaseAOnlinePreflightWindow(<move-only; denied>)")
    }
}

/// Runner-private proof that one production geoblock response was received by
/// the future selected-egress actor while it owned the exact input custody.
///
/// There is deliberately no production constructor in this slice. Pairing an
/// arbitrary production-origin observation with separately captured local
/// facts would not prove that the GET used that interface/address selection.
/// Nor does this scaffold yet carry one stable selected actor/client generation
/// across the initial observation, Prepared transition, Basis transition, A3
/// burn, final source refresh, and POST. A future child selected-actor module
/// must retain actor-private generation and client custody through every
/// reseal. Until that lands, this type must gain no constructor and every V2
/// positive path remains unconstructible.
#[must_use = "selected-egress geoblock evidence must remain under runtime custody"]
pub(super) struct PmSelectedEgressGeoblockObservation<T> {
    owned_custody: T,
    observation: PmProductionGeoblockObservation,
    thread_confinement: Rc<()>,
}

impl<T> fmt::Debug for PmSelectedEgressGeoblockObservation<T> {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str(
            "PmSelectedEgressGeoblockObservation(<move-only; inseparable-custody-and-selected-egress-source>)",
        )
    }
}

/// First inseparable V2 runtime typestate. It is still denied and cannot mint
/// a consumption runtime binding on its own.
#[must_use = "online runtime evidence must be rechecked or dropped"]
pub(super) struct PmPhaseAOnlineCurrentRuntimeWitness {
    evidence: PmPhaseAOnlineRuntimeEvidence,
}

impl PmPhaseAOnlineCurrentRuntimeWitness {
    /// Lend only non-authoritative manifest facts while all descriptor and
    /// monotonic custody remains owned by this witness.
    #[must_use]
    pub(super) const fn facts(&self) -> PmPhaseAOnlineCurrentRuntimeFactsView<'_> {
        PmPhaseAOnlineCurrentRuntimeFactsView {
            evidence: &self.evidence,
        }
    }
}

impl fmt::Debug for PmPhaseAOnlineCurrentRuntimeWitness {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str(
            "PmPhaseAOnlineCurrentRuntimeWitness(<move-only; window-runtime-egress-bound; denied>)",
        )
    }
}

/// Revalidated typestate retained inside prepared-consumption custody. It has
/// no public/sibling runtime-binding getter and cannot be detached.
struct PmRevalidatedPhaseAOnlineCurrentRuntimeWitness {
    evidence: PmPhaseAOnlineRuntimeEvidence,
}

/// Exact prepared V1+V2 consumption owners kept inseparable from the current
/// runtime/window/egress custody that created their runtime bindings.
#[must_use = "prepared online consumption custody must create the sealed Basis or be dropped"]
pub(super) struct PmPhaseAOnlinePreparedConsumptionPair {
    runtime: PmRevalidatedPhaseAOnlineCurrentRuntimeWitness,
    v1_consumption: PreparedAuthorizationConsumption,
    v2_consumption: PreparedOnlineAuthorizationConsumptionV2,
}

impl PmPhaseAOnlinePreparedConsumptionPair {
    #[must_use]
    pub(super) const fn facts(&self) -> PmPhaseAOnlineCurrentRuntimeFactsView<'_> {
        PmPhaseAOnlineCurrentRuntimeFactsView {
            evidence: &self.runtime.evidence,
        }
    }
}

impl fmt::Debug for PmPhaseAOnlinePreparedConsumptionPair {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .write_str("PmPhaseAOnlinePreparedConsumptionPair(<move-only; denied; runtime-bound>)")
    }
}

/// Pending Basis plus every source owner that was admitted into its internal
/// manifest. No component getter exists.
pub(super) struct PmPendingPhaseAOnlineRuntimeBurnV2 {
    pending: PmPendingPhaseAOnlinePreflightBasisV2,
    runtime: PmRevalidatedPhaseAOnlineCurrentRuntimeWitness,
    candidate: PmDeniedOnlinePreflightCandidate,
}

/// Burned durable dispatch evidence remains inseparable from source custody.
/// This is not a final freshness bearer; a later purpose-specific consuming
/// network handoff must actively re-observe the live book/user handles and
/// every mutable runtime/selected-egress source again immediately before
/// HMAC/transport. Candidate/Basis/A3 validation only rechecks retained
/// snapshot equality and cannot detect subsequent live source changes.
#[must_use = "burned online runtime custody must be finally consumed or converted to DND"]
pub(super) struct PmPhaseAOnlineRuntimeBurnedV2 {
    dispatch: PmPhaseAOnlinePreflightDispatchOwnerV2,
    runtime: PmRevalidatedPhaseAOnlineCurrentRuntimeWitness,
    candidate: PmDeniedOnlinePreflightCandidate,
}

impl fmt::Debug for PmPhaseAOnlineRuntimeBurnedV2 {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str(
            "PmPhaseAOnlineRuntimeBurnedV2(<move-only; inseparable; final-recheck-required>)",
        )
    }
}

impl PmPhaseAOnlineRuntimeBurnedV2 {
    /// Fail closed without exposing the embedded dispatch owner. The only
    /// current production exit destroys every positive place path.
    #[must_use]
    pub(super) fn abandon_to_definitely_not_dispatched(
        self,
    ) -> PmPhaseAPlaceDefinitelyNotDispatchedV1 {
        let Self {
            dispatch,
            runtime,
            candidate,
        } = self;
        drop(runtime);
        drop(candidate);
        dispatch.into_definitely_not_dispatched()
    }
}

/// Borrowed persistence facts only. It cannot construct either public runtime
/// binding and contains no `Instant`, descriptor, or owned source value.
pub(super) struct PmPhaseAOnlineCurrentRuntimeFactsView<'a> {
    evidence: &'a PmPhaseAOnlineRuntimeEvidence,
}

impl PmPhaseAOnlineCurrentRuntimeFactsView<'_> {
    #[must_use]
    pub(super) fn observation_started_at_utc(&self) -> &str {
        &self.evidence.window.started_at_utc
    }

    #[must_use]
    pub(super) fn observation_completed_at_utc(&self) -> &str {
        &self.evidence.window.completed_at_utc
    }

    #[must_use]
    pub(super) const fn current_runtime(&self) -> PmCurrentRuntimeFactsView<'_> {
        PmCurrentRuntimeFactsView {
            evidence: &self.evidence.current,
        }
    }
}

impl fmt::Debug for PmPhaseAOnlineCurrentRuntimeFactsView<'_> {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str("PmPhaseAOnlineCurrentRuntimeFactsView(<borrowed; denied>)")
    }
}

struct PmPhaseAOnlineRuntimeEvidence {
    current: PmCurrentRuntimeEvidence,
    window: FinishedOnlineWindowEvidence,
    local_egress: PmLinuxEgressLocalFactCustody,
    prior_selected_geoblocks: Vec<GeoblockEvidence>,
}

struct FinishedOnlineWindowEvidence {
    pins: OnlineRuntimePins,
    creating_process_id: u32,
    source_thread_id: u32,
    effective_user_id: u32,
    wall_started: SystemTime,
    monotonic_started: Instant,
    wall_completed: SystemTime,
    monotonic_completed: Instant,
    started_at_utc: Box<str>,
    completed_at_utc: Box<str>,
    maximum_age: Duration,
    maximum_age_ms: u64,
}

struct OnlineRuntimePins {
    config_sha256: Box<str>,
    config_length: u64,
    config_fingerprint: Box<str>,
    plan_fingerprint: Box<str>,
    v1_authorization_id: Box<str>,
    v1_authorization_fingerprint: Box<str>,
    online_policy_sha256: Box<str>,
    online_policy_length: u64,
    online_policy_fingerprint: Box<str>,
    online_authorization_id: Box<str>,
    online_authorization_sha256: Box<str>,
    online_authorization_length: u64,
    online_authorization_fingerprint: Box<str>,
}

impl OnlineRuntimePins {
    fn capture(
        config: &CanonicalTrialConfig,
        v1_authorization: &CanonicalAuthorization,
        policy: &CanonicalOnlinePolicyV2,
        online_authorization: &CanonicalOnlineAuthorizationV2,
    ) -> Self {
        Self {
            config_sha256: config.canonical_sha256().into(),
            config_length: config.canonical_length(),
            config_fingerprint: config.fingerprint().into(),
            plan_fingerprint: config.plan_fingerprint().into(),
            v1_authorization_id: v1_authorization.value().authorization_id.as_str().into(),
            v1_authorization_fingerprint: v1_authorization.fingerprint().into(),
            online_policy_sha256: policy.canonical_sha256().into(),
            online_policy_length: policy.canonical_length(),
            online_policy_fingerprint: policy.fingerprint().into(),
            online_authorization_id: online_authorization
                .value()
                .authorization_id
                .as_str()
                .into(),
            online_authorization_sha256: online_authorization.canonical_sha256().into(),
            online_authorization_length: online_authorization.canonical_length(),
            online_authorization_fingerprint: online_authorization.fingerprint().into(),
        }
    }

    fn validate(
        &self,
        config: &CanonicalTrialConfig,
        v1_authorization: &CanonicalAuthorization,
        policy: &CanonicalOnlinePolicyV2,
        online_authorization: &CanonicalOnlineAuthorizationV2,
    ) -> Result<(), PmCurrentRuntimeError> {
        if self.config_sha256.as_ref() != config.canonical_sha256()
            || self.config_length != config.canonical_length()
            || self.config_fingerprint.as_ref() != config.fingerprint()
            || self.plan_fingerprint.as_ref() != config.plan_fingerprint()
            || self.v1_authorization_id.as_ref()
                != v1_authorization.value().authorization_id.as_str()
            || self.v1_authorization_fingerprint.as_ref() != v1_authorization.fingerprint()
            || self.online_policy_sha256.as_ref() != policy.canonical_sha256()
            || self.online_policy_length != policy.canonical_length()
            || self.online_policy_fingerprint.as_ref() != policy.fingerprint()
            || self.online_authorization_id.as_ref()
                != online_authorization.value().authorization_id.as_str()
            || self.online_authorization_sha256.as_ref() != online_authorization.canonical_sha256()
            || self.online_authorization_length != online_authorization.canonical_length()
            || self.online_authorization_fingerprint.as_ref() != online_authorization.fingerprint()
        {
            return Err(PmCurrentRuntimeError::AuthorizationBinding);
        }
        Ok(())
    }
}

fn consume_selected_geoblock<T>(
    selected: PmSelectedEgressGeoblockObservation<T>,
) -> Result<(T, PmProductionGeoblockObservation), PmPhaseAOnlineRuntimeError> {
    let PmSelectedEgressGeoblockObservation {
        owned_custody,
        observation,
        thread_confinement,
    } = selected;
    if Rc::strong_count(&thread_confinement) != 1 {
        return Err(PmPhaseAOnlineRuntimeError::SelectedEgressProvenance);
    }
    drop(thread_confinement);
    Ok((owned_custody, observation))
}

fn validate_production_geoblock_inside_window(
    observation: &PmProductionGeoblockObservation,
    wall_started: SystemTime,
    wall_completed: SystemTime,
    maximum_age: Duration,
) -> Result<(), PmCurrentRuntimeError> {
    let receive_ns = observation.receive_clock().local_wall_receive_ns();
    let started_ns = unix_nanoseconds(wall_started)?;
    let completed_ns = unix_nanoseconds(wall_completed)?;
    if receive_ns < started_ns || receive_ns > completed_ns {
        return Err(PmCurrentRuntimeError::GeoblockOutsidePreflightWindow);
    }
    validate_geoblock_values(
        observation.status().blocked(),
        receive_ns,
        wall_completed,
        maximum_age,
    )
}

fn validate_fresh_selected_geoblock(
    geoblock: &GeoblockEvidence,
    previous_runtime_edge: SystemTime,
    maximum_age: Duration,
) -> Result<(), PmCurrentRuntimeError> {
    let previous_ns = unix_nanoseconds(previous_runtime_edge)?;
    if geoblock.receive_clock.local_wall_receive_ns() < previous_ns {
        return Err(PmCurrentRuntimeError::GeoblockNotFreshRecheck);
    }
    validate_geoblock(geoblock, SystemTime::now(), maximum_age)
}

fn validate_online_source_identity(
    expected_process_id: u32,
    expected_thread_id: u32,
    expected_effective_user_id: u32,
) -> Result<(), PmCurrentRuntimeError> {
    if std::process::id() != expected_process_id
        || current_runtime_thread_id()? != expected_thread_id
        || current_effective_user_id()? != expected_effective_user_id
    {
        return Err(PmCurrentRuntimeError::OnlineSourceIdentityChanged);
    }
    Ok(())
}

#[cfg(target_os = "linux")]
fn current_runtime_thread_id() -> Result<u32, PmCurrentRuntimeError> {
    u32::try_from(rustix::thread::gettid().as_raw_pid())
        .map_err(|_| PmCurrentRuntimeError::OnlineSourceIdentityChanged)
}

#[cfg(not(target_os = "linux"))]
fn current_runtime_thread_id() -> Result<u32, PmCurrentRuntimeError> {
    Err(PmCurrentRuntimeError::UnsupportedPlatform)
}

#[cfg(target_os = "linux")]
fn current_effective_user_id() -> Result<u32, PmCurrentRuntimeError> {
    Ok(rustix::process::geteuid().as_raw())
}

#[cfg(not(target_os = "linux"))]
fn current_effective_user_id() -> Result<u32, PmCurrentRuntimeError> {
    Err(PmCurrentRuntimeError::UnsupportedPlatform)
}

fn validate_exact_online_inputs(
    config: &CanonicalTrialConfig,
    v1_authorization: &CanonicalAuthorization,
    policy: &CanonicalOnlinePolicyV2,
    online_authorization: &CanonicalOnlineAuthorizationV2,
    current_wall: SystemTime,
) -> Result<u64, PmCurrentRuntimeError> {
    validate_place_cancel_phase(config.value().phase, v1_authorization.value().phase)?;
    let current_utc = system_time_utc(current_wall)?;
    verify_authorization(config, v1_authorization, current_utc)
        .map_err(|_| PmCurrentRuntimeError::AuthorizationBinding)?;
    let policy_value = policy.value();
    let online = online_authorization.value();
    let config_pins = &online.v1_config;
    if policy_value.phase != TrialPhase::APlaceCancel
        || online.phase != TrialPhase::APlaceCancel
        || policy_value.v1_config != online.v1_config
        || config_pins.canonical_config_sha256 != config.canonical_sha256()
        || config_pins.canonical_config_length != config.canonical_length()
        || config_pins.canonical_config_fingerprint != config.fingerprint()
        || config_pins.trial_plan_fingerprint != config.plan_fingerprint()
        || online.online_policy.canonical_sha256 != policy.canonical_sha256()
        || online.online_policy.canonical_length != policy.canonical_length()
        || online.online_policy.fingerprint != policy.fingerprint()
        || online.build.repository_commit != v1_authorization.value().build.repository_commit
        || online.build.cargo_lock_sha256 != v1_authorization.value().build.cargo_lock_sha256
        || online.build.release_binary_sha256
            != v1_authorization.value().build.release_binary_sha256
        || online.build.release_binary_length
            != v1_authorization.value().build.release_binary_length
        || online.host.uts_nodename != v1_authorization.value().host.host_identity
        || online.host.boot_id != v1_authorization.value().host.boot_identity
        || online.host.nss_username != v1_authorization.value().host.runtime_user
        || online.host.egress.authorized_geoblock_reported_public_ip
            != v1_authorization.value().host.egress_identity
        || online.status_notice_history.clob_component
            != policy_value.reviewed_status_clob_component
    {
        return Err(PmCurrentRuntimeError::AuthorizationBinding);
    }

    let policy_reviewed_at = parse_canonical_utc(&policy_value.reviewed_at_utc)?;
    let reviewed_at = parse_canonical_utc(&online.reviewed_at_utc)?;
    let not_before = parse_canonical_utc(&online.not_before_utc)?;
    let expires_at = parse_canonical_utc(&online.expires_at_utc)?;
    let cleanup_not_after = parse_canonical_utc(&online.cleanup_not_after_utc)?;
    let history_started =
        parse_canonical_utc(&online.status_notice_history.history_window_start_utc)?;
    let history_completed =
        parse_canonical_utc(&online.status_notice_history.reviewed_through_utc)?;
    let current_utc = system_time_utc(current_wall)?;
    let history_seconds = history_completed
        .timestamp()
        .checked_sub(history_started.timestamp())
        .ok_or(PmCurrentRuntimeError::AuthorizationBinding)?;
    let required_history_seconds =
        i64::try_from(policy_value.minimum_notice_history_quiet_interval_seconds)
            .map_err(|_| PmCurrentRuntimeError::AuthorizationBinding)?;
    if policy_reviewed_at > reviewed_at
        || reviewed_at > not_before
        || history_completed != reviewed_at
        || history_seconds < required_history_seconds
        || current_utc < not_before
        || current_utc >= expires_at
    {
        return Err(PmCurrentRuntimeError::AuthorizationBinding);
    }
    let cleanup_budget = Duration::from_millis(config.value().time_limits.cleanup_not_after_ms);
    validate_cleanup_runway(
        current_wall,
        SystemTime::from(cleanup_not_after),
        cleanup_budget,
    )?;
    let v1_cleanup = parse_canonical_utc(&v1_authorization.value().cleanup_not_after_utc)?;
    validate_cleanup_runway(current_wall, SystemTime::from(v1_cleanup), cleanup_budget)?;
    let maximum_age_ms = config
        .value()
        .time_limits
        .maximum_preflight_observation_age_ms
        .min(policy_value.maximum_observation_age_ms);
    if maximum_age_ms == 0 {
        return Err(PmCurrentRuntimeError::ObservationExpired);
    }
    Ok(maximum_age_ms)
}

fn validate_exact_v2_runtime_binding(
    evidence: &PmCurrentRuntimeEvidence,
    config: &CanonicalTrialConfig,
    v1_authorization: &CanonicalAuthorization,
    policy: &CanonicalOnlinePolicyV2,
    online_authorization: &CanonicalOnlineAuthorizationV2,
) -> Result<(), PmCurrentRuntimeError> {
    let _ = validate_exact_online_inputs(
        config,
        v1_authorization,
        policy,
        online_authorization,
        evidence.checked_wall_complete,
    )?;
    let online = online_authorization.value();
    let authorized_public_ip = online
        .host
        .egress
        .authorized_geoblock_reported_public_ip
        .parse::<IpAddr>()
        .map_err(|_| PmCurrentRuntimeError::AuthorizationBinding)?;
    if evidence.executable.sha256_hex.as_ref() != online.build.release_binary_sha256
        || evidence.executable.length != online.build.release_binary_length
        || evidence.observed_host.host_identity.as_ref() != online.host.uts_nodename
        || evidence.observed_host.boot_identity.as_ref() != online.host.boot_id
        || evidence.observed_host.runtime_user.as_ref() != online.host.nss_username
        || evidence.effective_user_id != online.host.linux_euid
        || evidence.geoblock.blocked
        || evidence.geoblock.ip != authorized_public_ip
        || evidence.observed_host.egress_identity != authorized_public_ip
    {
        return Err(PmCurrentRuntimeError::AuthorizationBinding);
    }
    Ok(())
}

fn exact_consumption_runtime_bindings(
    evidence: &mut PmPhaseAOnlineRuntimeEvidence,
    online_authorization: &CanonicalOnlineAuthorizationV2,
) -> Result<
    (
        AuthorizationRuntimeBinding,
        OnlineAuthorizationRuntimeBindingV2,
    ),
    PmPhaseAOnlineRuntimeError,
> {
    let (
        final_wall,
        final_monotonic,
        final_effective_user_id,
        network_namespace_device,
        network_namespace_inode,
        interface_name,
        interface_index,
        local_source_ip,
    ) = {
        let local = evidence.local_egress.revalidate_for_current_runtime(
            online_authorization,
            evidence.window.wall_started,
            evidence.window.wall_completed,
            evidence.window.monotonic_started,
            evidence.window.monotonic_completed,
            evidence.window.maximum_age,
        )?;
        (
            local.wall_completed(),
            local.monotonic_completed(),
            local.effective_user_id(),
            local.network_namespace_device(),
            local.network_namespace_inode(),
            local.interface_name().to_owned(),
            local.interface_index(),
            local.local_source_ip().to_string(),
        )
    };
    validate_age_window(
        evidence.window.wall_started,
        evidence.window.monotonic_started,
        final_wall,
        final_monotonic,
        evidence.window.maximum_age,
    )?;
    evidence.current.executable.revalidate_held_identity()?;
    validate_online_source_identity(
        evidence.window.creating_process_id,
        evidence.window.source_thread_id,
        evidence.window.effective_user_id,
    )?;
    validate_geoblock(
        &evidence.current.geoblock,
        final_wall,
        evidence.window.maximum_age,
    )?;
    let observed_at_utc = canonical_utc(final_wall)?;
    let v1_runtime = AuthorizationRuntimeBinding {
        release_binary_sha256: evidence.current.executable.sha256_hex.to_string(),
        release_binary_length: evidence.current.executable.length,
        host: AuthorizationHostBinding {
            host_identity: evidence.current.observed_host.host_identity.to_string(),
            boot_identity: evidence.current.observed_host.boot_identity.to_string(),
            runtime_user: evidence.current.observed_host.runtime_user.to_string(),
            egress_identity: evidence.current.geoblock.ip.to_string(),
        },
        observed_at_utc: observed_at_utc.clone(),
    };
    let v2_runtime = OnlineAuthorizationRuntimeBindingV2 {
        release_binary_sha256: evidence.current.executable.sha256_hex.to_string(),
        release_binary_length: evidence.current.executable.length,
        uts_nodename: evidence.current.observed_host.host_identity.to_string(),
        boot_id: evidence.current.observed_host.boot_identity.to_string(),
        nss_username: evidence.current.observed_host.runtime_user.to_string(),
        linux_euid: final_effective_user_id,
        network_namespace_device,
        network_namespace_inode,
        interface_name,
        interface_index,
        local_source_ip,
        geoblock_reported_public_ip: evidence.current.geoblock.ip.to_string(),
        observed_at_utc,
    };
    evidence.current.checked_wall_complete = final_wall;
    evidence.current.checked_monotonic_complete = final_monotonic;
    evidence.current.checked_wall_capture_unix_ns = unix_nanoseconds(final_wall)?;
    evidence.current.checked_at_utc = canonical_utc(final_wall)?.into();
    Ok((v1_runtime, v2_runtime))
}

fn validate_candidate_inside_outer_window(
    candidate: &PmDeniedOnlinePreflightCandidate,
    window: &FinishedOnlineWindowEvidence,
    online_authorization: &CanonicalOnlineAuthorizationV2,
) -> Result<(), PmPhaseAOnlineRuntimeError> {
    let manifest = candidate.candidate_manifest();
    let outer_started_ns = unix_nanoseconds(window.wall_started)?;
    let outer_completed_ns = unix_nanoseconds(window.wall_completed)?;
    let source_minimum = manifest.minimum_source_wall_edge_ns();
    let source_maximum = manifest.maximum_source_wall_edge_ns();
    if source_minimum == 0
        || source_minimum > source_maximum
        || source_minimum < outer_started_ns
        || source_maximum > outer_completed_ns
    {
        return Err(PmPhaseAOnlineRuntimeError::CandidateManifest);
    }
    let market_end = parse_canonical_utc(manifest.market_end_time())?;
    let cleanup = parse_canonical_utc(&online_authorization.value().cleanup_not_after_utc)?;
    if market_end <= cleanup {
        return Err(PmPhaseAOnlineRuntimeError::CandidateManifest);
    }
    let canonical_sha256: [u8; 32] = Sha256::digest(manifest.canonical_bytes()).into();
    if canonical_sha256 != manifest.canonical_sha256()
        || manifest.canonical_bytes().is_empty()
        || manifest.fingerprint() == [0_u8; 32]
    {
        return Err(PmPhaseAOnlineRuntimeError::CandidateManifest);
    }
    Ok(())
}

fn assemble_online_preflight_manifest(
    candidate: &PmDeniedOnlinePreflightCandidate,
    runtime: &PmPhaseAOnlineRuntimeEvidence,
    policy: &CanonicalOnlinePolicyV2,
    online_authorization: &CanonicalOnlineAuthorizationV2,
    v1_runtime: &AuthorizationRuntimeBinding,
    v2_runtime: &OnlineAuthorizationRuntimeBindingV2,
) -> Result<PmPhaseAOnlinePreflightEvidenceManifestV2, PmPhaseAOnlineRuntimeError> {
    let candidate_manifest = candidate.candidate_manifest();
    let reviewed_market_evidence_sha256 = policy.value().reviewed_market.review_sha256.clone();
    let reviewed_status_history_sha256 = online_authorization
        .value()
        .status_notice_history
        .review_sha256
        .clone();
    let fresh_status_announcements_sha256 =
        lower_hex(&candidate_manifest.fresh_status_announcements_sha256());
    let clob_ok_liveness_sha256 = lower_hex(&candidate_manifest.clob_ok_liveness_sha256());
    let same_account_closed_only_sha256 =
        lower_hex(&candidate_manifest.same_account_closed_only_sha256());
    let public_book_cut_sha256 = lower_hex(&candidate_manifest.public_book_cut_sha256());
    let user_account_cut_sha256 = lower_hex(&candidate_manifest.user_account_cut_sha256());
    let same_authority_rest_cut_sha256 =
        lower_hex(&candidate_manifest.same_authority_rest_cut_sha256());
    let finalized_chain_cut_sha256 = lower_hex(&candidate_manifest.finalized_chain_cut_sha256());
    let data_api_position_cut_sha256 =
        lower_hex(&candidate_manifest.data_api_position_cut_sha256());
    let current_runtime_and_egress_sha256 =
        runtime_and_egress_digest(runtime, v1_runtime, v2_runtime);
    let reviewed_repository_state_sha256 = reviewed_repository_state_digest(online_authorization);
    let (canonical_manifest_sha256, canonical_manifest_length) =
        full_online_preflight_manifest_identity(
            &candidate_manifest,
            runtime,
            policy,
            online_authorization,
            &[
                &reviewed_market_evidence_sha256,
                &reviewed_status_history_sha256,
                &fresh_status_announcements_sha256,
                &clob_ok_liveness_sha256,
                &same_account_closed_only_sha256,
                &public_book_cut_sha256,
                &user_account_cut_sha256,
                &same_authority_rest_cut_sha256,
                &finalized_chain_cut_sha256,
                &data_api_position_cut_sha256,
                &current_runtime_and_egress_sha256,
                &reviewed_repository_state_sha256,
            ],
        )?;
    Ok(PmPhaseAOnlinePreflightEvidenceManifestV2 {
        observation_started_at_utc: runtime.window.started_at_utc.to_string(),
        observation_completed_at_utc: runtime.window.completed_at_utc.to_string(),
        canonical_manifest_sha256,
        canonical_manifest_length,
        reviewed_market_evidence_sha256,
        reviewed_status_history_sha256,
        fresh_status_announcements_sha256,
        clob_ok_liveness_sha256,
        same_account_closed_only_sha256,
        public_book_cut_sha256,
        user_account_cut_sha256,
        same_authority_rest_cut_sha256,
        finalized_chain_cut_sha256,
        data_api_position_cut_sha256,
        current_runtime_and_egress_sha256,
        reviewed_repository_state_sha256,
    })
}

/// Canonical identity of the complete runner-assembled manifest body. The
/// self-identifying SHA/length fields are intentionally excluded; every
/// candidate, window, config/policy/authorization, runtime/egress, reviewed
/// repository, and per-source digest input is included exactly once.
fn full_online_preflight_manifest_identity(
    candidate: &super::online_preflight::PmOnlinePreflightCandidateManifestView<'_>,
    runtime: &PmPhaseAOnlineRuntimeEvidence,
    policy: &CanonicalOnlinePolicyV2,
    online_authorization: &CanonicalOnlineAuthorizationV2,
    evidence_sha256: &[&str; 12],
) -> Result<(String, u64), PmPhaseAOnlineRuntimeError> {
    let mut bytes = Vec::new();
    append_manifest_bytes(
        &mut bytes,
        b"domain",
        b"reap.pm-t2.runner.complete-online-preflight-manifest.v2\0",
    );
    append_manifest_bytes(
        &mut bytes,
        b"partial_candidate",
        candidate.canonical_bytes(),
    );
    append_manifest_bytes(
        &mut bytes,
        b"partial_candidate_sha256",
        &candidate.canonical_sha256(),
    );
    append_manifest_bytes(
        &mut bytes,
        b"partial_candidate_fingerprint",
        &candidate.fingerprint(),
    );
    append_manifest_u64(
        &mut bytes,
        b"source_wall_min_ns",
        candidate.minimum_source_wall_edge_ns(),
    );
    append_manifest_u64(
        &mut bytes,
        b"source_wall_max_ns",
        candidate.maximum_source_wall_edge_ns(),
    );
    append_manifest_text(&mut bytes, b"market_end_time", candidate.market_end_time());
    append_manifest_text(
        &mut bytes,
        b"window_started_at_utc",
        &runtime.window.started_at_utc,
    );
    append_manifest_text(
        &mut bytes,
        b"window_completed_at_utc",
        &runtime.window.completed_at_utc,
    );
    append_manifest_u64(
        &mut bytes,
        b"window_maximum_age_ms",
        runtime.window.maximum_age_ms,
    );
    append_manifest_text(
        &mut bytes,
        b"config_fingerprint",
        &runtime.window.pins.config_fingerprint,
    );
    append_manifest_text(
        &mut bytes,
        b"v1_authorization_fingerprint",
        &runtime.window.pins.v1_authorization_fingerprint,
    );
    append_manifest_text(&mut bytes, b"policy_sha256", policy.canonical_sha256());
    append_manifest_u64(&mut bytes, b"policy_length", policy.canonical_length());
    append_manifest_text(&mut bytes, b"policy_fingerprint", policy.fingerprint());
    append_manifest_text(
        &mut bytes,
        b"online_authorization_sha256",
        online_authorization.canonical_sha256(),
    );
    append_manifest_u64(
        &mut bytes,
        b"online_authorization_length",
        online_authorization.canonical_length(),
    );
    append_manifest_text(
        &mut bytes,
        b"online_authorization_fingerprint",
        online_authorization.fingerprint(),
    );
    for (index, digest) in evidence_sha256.iter().enumerate() {
        append_manifest_u64(&mut bytes, b"evidence_index", index as u64);
        append_manifest_text(&mut bytes, b"evidence_sha256", digest);
    }
    let length =
        u64::try_from(bytes.len()).map_err(|_| PmPhaseAOnlineRuntimeError::CandidateManifest)?;
    if length == 0 {
        return Err(PmPhaseAOnlineRuntimeError::CandidateManifest);
    }
    let sha256: [u8; 32] = Sha256::digest(&bytes).into();
    Ok((lower_hex(&sha256), length))
}

fn append_manifest_bytes(target: &mut Vec<u8>, key: &[u8], value: &[u8]) {
    target.extend_from_slice(&(key.len() as u64).to_be_bytes());
    target.extend_from_slice(key);
    target.extend_from_slice(&(value.len() as u64).to_be_bytes());
    target.extend_from_slice(value);
}

fn append_manifest_text(target: &mut Vec<u8>, key: &[u8], value: &str) {
    append_manifest_bytes(target, key, value.as_bytes());
}

fn append_manifest_u64(target: &mut Vec<u8>, key: &[u8], value: u64) {
    append_manifest_bytes(target, key, &value.to_be_bytes());
}

fn runtime_and_egress_digest(
    runtime: &PmPhaseAOnlineRuntimeEvidence,
    v1: &AuthorizationRuntimeBinding,
    v2: &OnlineAuthorizationRuntimeBindingV2,
) -> String {
    let mut digest = Sha256::new();
    digest.update(b"reap.pm-t2.runner.current-runtime-and-selected-egress.v2\0");
    digest_text(&mut digest, &v1.release_binary_sha256);
    digest_u64(&mut digest, v1.release_binary_length);
    digest_text(&mut digest, &v1.host.host_identity);
    digest_text(&mut digest, &v1.host.boot_identity);
    digest_text(&mut digest, &v1.host.runtime_user);
    digest_text(&mut digest, &v1.host.egress_identity);
    digest_text(&mut digest, &v1.observed_at_utc);
    digest_text(&mut digest, &v2.uts_nodename);
    digest_text(&mut digest, &v2.boot_id);
    digest_text(&mut digest, &v2.nss_username);
    digest_u64(&mut digest, u64::from(v2.linux_euid));
    digest_u64(&mut digest, v2.network_namespace_device);
    digest_u64(&mut digest, v2.network_namespace_inode);
    digest_text(&mut digest, &v2.interface_name);
    digest_u64(&mut digest, u64::from(v2.interface_index));
    digest_text(&mut digest, &v2.local_source_ip);
    digest_text(&mut digest, &v2.geoblock_reported_public_ip);
    digest_text(&mut digest, &v2.observed_at_utc);
    digest_text(&mut digest, &runtime.window.started_at_utc);
    digest_text(&mut digest, &runtime.window.completed_at_utc);
    digest_u64(&mut digest, runtime.window.maximum_age_ms);
    digest.update(runtime.current.geoblock.commitment.bytes());
    digest_u64(&mut digest, runtime.prior_selected_geoblocks.len() as u64);
    for geoblock in &runtime.prior_selected_geoblocks {
        digest.update(geoblock.commitment.bytes());
    }
    let value: [u8; 32] = digest.finalize().into();
    lower_hex(&value)
}

fn reviewed_repository_state_digest(
    online_authorization: &CanonicalOnlineAuthorizationV2,
) -> String {
    let build = &online_authorization.value().build;
    let mut digest = Sha256::new();
    digest.update(b"reap.pm-t2.runner.reviewed-repository-state.v2\0");
    digest_text(&mut digest, &build.repository_commit);
    digest_text(&mut digest, "exact_clean_commit");
    digest_text(&mut digest, &build.cargo_lock_sha256);
    digest_text(&mut digest, &build.release_binary_sha256);
    digest_u64(&mut digest, build.release_binary_length);
    let value: [u8; 32] = digest.finalize().into();
    lower_hex(&value)
}

fn digest_text(digest: &mut Sha256, value: &str) {
    digest_u64(digest, value.len() as u64);
    digest.update(value.as_bytes());
}

fn digest_u64(digest: &mut Sha256, value: u64) {
    digest.update(value.to_be_bytes());
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
    #[error("the selected-egress geoblock response falls outside the outer preflight window")]
    GeoblockOutsidePreflightWindow,
    #[error("the selected-egress geoblock response does not follow the prior runtime edge")]
    GeoblockNotFreshRecheck,
    #[error("the online preflight process, thread, or effective-user source changed")]
    OnlineSourceIdentityChanged,
    #[error("canonical config, authorization, release, host, or egress binding differs")]
    AuthorizationBinding,
    #[error("the full conservative cleanup budget no longer fits its authorization window")]
    CleanupWindow,
}

#[derive(Debug, Error)]
pub(super) enum PmPhaseAOnlineRuntimeError {
    #[error(transparent)]
    CurrentRuntime(#[from] PmCurrentRuntimeError),
    #[error(transparent)]
    LocalEgress(#[from] PmLinuxEgressLocalFactError),
    #[error("selected-egress provenance custody is not unique")]
    SelectedEgressProvenance,
    #[error("V1 authorization-consumption preparation failed closed")]
    V1Consumption(PmAuthorizationConsumptionError),
    #[error("V2 authorization-consumption preparation failed closed")]
    V2Consumption(PmOnlineAuthorizationConsumptionV2Error),
    #[error("durable online-preflight sidecar transition failed closed")]
    OnlinePreflight(PmPhaseAOnlinePreflightV2Error),
    #[error("the sealed online-preflight candidate manifest binding is invalid")]
    CandidateManifest,
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
