use sha2::{Digest as _, Sha256};

const MAIN: &str = include_str!("../src/main.rs");
const CANDIDATE: &str = include_str!("../src/phase_a_candidate.rs");
const PHASE_A_V4_DRAFT: &str = include_str!("../src/phase_a_v4_draft.rs");
const MANIFEST: &str = include_str!("../Cargo.toml");

const CANDIDATE_SHA256: &str = "e44aa6fe15ae6df54c4f62096abc3a7ef0d52df4e3001ed1054caa22937ead2d";
const PHASE_A_V4_DRAFT_SHA256: &str =
    "a2f47c81fc7c22efb1fb3a4f1615c98ce6e8409a02967766b733385bfb4f659e";

fn sha256_hex(bytes: &[u8]) -> String {
    let digest = Sha256::digest(bytes);
    let mut output = String::with_capacity(64);
    for byte in digest {
        use std::fmt::Write as _;
        write!(&mut output, "{byte:02x}").expect("writing to String cannot fail");
    }
    output
}

fn production_source() -> &'static str {
    CANDIDATE
        .split_once("\n#[cfg(test)]\nmod tests")
        .map_or(CANDIDATE, |(production, _)| production)
}

fn compact_whitespace(source: &str) -> String {
    source.split_whitespace().collect::<Vec<_>>().join(" ")
}

fn exact_if_condition<'a>(source: &'a str, start: &str) -> &'a str {
    let (_, tail) = source
        .split_once(start)
        .unwrap_or_else(|| panic!("missing condition start `{start}`"));
    let (remainder, _) = tail
        .split_once("\n    {")
        .unwrap_or_else(|| panic!("missing condition terminator after `{start}`"));
    let start_offset = source.len() - tail.len() - start.len();
    &source[start_offset..source.len() - tail.len() + remainder.len()]
}

#[test]
fn runner_cli_preserves_the_three_offline_non_authorizing_commands() {
    assert_eq!(
        sha256_hex(CANDIDATE.as_bytes()),
        CANDIDATE_SHA256,
        "candidate production source changed outside the exact audited freeze"
    );
    assert_eq!(
        sha256_hex(PHASE_A_V4_DRAFT.as_bytes()),
        PHASE_A_V4_DRAFT_SHA256,
        "Phase-A V4 draft source changed outside the exact audited freeze"
    );
    assert_eq!(
        MAIN.matches("enum Command {").count(),
        1,
        "runner Command enum declaration became ambiguous"
    );
    assert_eq!(MAIN.matches("Command::FreezePhaseACandidate {").count(), 1);
    assert_eq!(
        MAIN.matches("Command::GeneratePhaseAAuthorizationRequestNotAuthorization {")
            .count(),
        1
    );
    assert_eq!(
        MAIN.matches("Command::DraftNonAuthorizingPhaseAEligibilityEnvelopeV4 {")
            .count(),
        1
    );

    for required in [
        "FreezePhaseACandidate",
        "GeneratePhaseAAuthorizationRequestNotAuthorization",
        "DraftNonAuthorizingPhaseAEligibilityEnvelopeV4",
        "freeze_phase_a_candidate(",
        "generate_phase_a_authorization_request_not_authorization(",
        "draft_non_authorizing_phase_a_eligibility_envelope_v4(",
        "reviewed_phase_a_eligibility_envelope_v4",
        "serde_json::to_vec(&report)",
        "stdout().lock().write_all(&canonical_bytes)",
        ".write_all(output.canonical_bytes())",
    ] {
        assert!(MAIN.contains(required), "freeze CLI lost `{required}`");
    }
    for forbidden in [
        "println!",
        "TcpStream",
        "UdpSocket",
        "reqwest",
        "POST /order",
        "DELETE /order",
        "output_path",
        "output_file",
        "File::create",
        "OpenOptions",
        "fs::write",
    ] {
        assert!(
            !MAIN.contains(forbidden),
            "freeze CLI gained forbidden `{forbidden}`"
        );
    }
    assert!(MANIFEST.contains("clap.workspace = true"));
    assert!(MANIFEST.contains("serde.workspace = true"));
    assert!(MANIFEST.contains("serde_json.workspace = true"));
    assert!(!MANIFEST.contains("reqwest"));
}

#[test]
fn phase_a_v4_draft_is_context_derived_roundtripped_and_permanently_denied() {
    let draft = PHASE_A_V4_DRAFT;
    for required in [
        "load_canonical_trial_config",
        "load_canonical_authorization",
        "load_canonical_online_policy_v2",
        "load_canonical_online_authorization_v2",
        "load_canonical_reviewed_production_destination_profile_v1",
        "load_canonical_reviewed_fresh_credential_slot_locator_v1",
        "load_canonical_fresh_credential_delivery_binding_v1",
        "load_canonical_reviewed_signer_proxy_account_identity_v1",
        "load_canonical_reviewed_remote_credential_proof_policy_v1",
        "load_canonical_reviewed_static_online_authorization_v3",
        "ReviewedPhaseAEligibilityEnvelopeContextV4",
        "v1_config: &config",
        "v1_authorization: &authorization",
        "online_policy_v2: &online_policy",
        "online_authorization_v2: &online_authorization",
        "reviewed_production_destination_v1: &destination",
        "reviewed_fresh_credential_slot_locator_v1: &locator",
        "fresh_credential_delivery_binding_v1: &delivery",
        "reviewed_signer_proxy_account_identity_v1: &account_identity",
        "reviewed_remote_credential_proof_policy_v1: &remote_proof_policy",
        "reviewed_static_online_authorization_v3: &static_authorization",
        "draft_non_authorizing_reviewed_phase_a_eligibility_envelope_v4",
        "serde_json::to_vec(&drafted)",
        "serde_json::from_slice(&canonical_bytes)",
        "roundtrip_bytes != canonical_bytes",
        "independently_redrafted != roundtrip",
        "closed_negative_authorization(&roundtrip)",
        "authorization != OfflineAuthorizationState::DENIED",
        "authorization.place_dispatch_allowance != 0",
        "Ok(OfflineAuthorizationState::DENIED)",
        "VerifiedNonAuthorizingPhaseAEligibilityEnvelopeV4Bytes",
    ] {
        assert!(draft.contains(required), "V4 draft lost `{required}`");
    }
    for forbidden in [
        "SystemTime",
        "Utc::now",
        "std::fs",
        "File::create",
        "OpenOptions",
        "fs::write",
        "std::net",
        "TcpStream",
        "UdpSocket",
        "reqwest",
        "tokio",
        "FixedEoaSigner",
        "L2Credentials",
        "EoaPrivateKeyInput",
        "L2CredentialInput",
        "journal::",
        "Permit",
        "output_path",
        "output_file",
        "verify_reviewed_phase_a_eligibility_envelope_v4",
    ] {
        assert!(
            !draft.contains(forbidden),
            "V4 draft crossed forbidden boundary `{forbidden}`"
        );
    }
}

#[test]
fn candidate_preserves_the_full_static_v3_verifier_and_denial() {
    let production = production_source();
    for required in [
        "load_canonical_trial_config",
        "load_canonical_authorization",
        "load_canonical_online_policy_v2",
        "load_canonical_online_authorization_v2",
        "load_canonical_reviewed_production_destination_profile_v1",
        "load_canonical_reviewed_fresh_credential_slot_locator_v1",
        "load_canonical_fresh_credential_delivery_binding_v1",
        "load_canonical_reviewed_signer_proxy_account_identity_v1",
        "load_canonical_reviewed_remote_credential_proof_policy_v1",
        "load_canonical_reviewed_static_online_authorization_v3",
        "ReviewedStaticOnlineAuthorizationContextV3",
        "verify_reviewed_static_online_authorization_v3(",
        "&static_v3_context",
        "static_verification.authorization != OfflineAuthorizationState::DENIED",
        "record_kind: REPORT_KIND",
        "phase_a_non_authorizing_candidate_gap_report_v1",
        "candidate_only_non_authorizing: true",
        "exact_nine_holder_static_v3_conjunction_verified: true",
        "live_authorization_record_generated: false",
        "static_v3_false_boolean_paths_exhaustive",
        "authorization: OfflineAuthorizationState::DENIED",
    ] {
        assert!(production.contains(required), "candidate lost `{required}`");
    }
    for required_gap in [
        "exact_live_economic_and_time_authorization_complete_and_current_for_this_runner: false",
        "authenticated_credential_provider_trust_root_selected_for_this_runner: false",
        "authenticated_exclusive_delivery_lease_complete_for_this_runner: false",
        "authoritative_remote_credential_acceptance_complete_for_this_runner: false",
        "authoritative_signer_proxy_control_complete_for_this_runner: false",
        "current_egress_and_live_preflight_evidence_complete_for_this_runner: false",
        "selected_place_cancel_actor_complete_for_this_runner: false",
        "durable_v3_attempt_commitment_burn_and_no_resend_complete_for_this_runner: false",
        "recovery_only_exact_cancel_continuation_complete_for_this_runner: false",
        "production_mutation_authority_minted_for_this_runner: false",
    ] {
        assert!(
            production.contains(required_gap),
            "candidate lost gap `{required_gap}`"
        );
    }
}

#[test]
fn candidate_joins_the_exact_ten_holder_v4_envelope_and_preserves_hard_denial() {
    let production = production_source();
    assert_eq!(
        compact_whitespace(exact_if_condition(
            production,
            "if static_verification.authorization",
        )),
        "if static_verification.authorization != OfflineAuthorizationState::DENIED || static_verification.authorization.place_dispatch_allowance != 0",
        "candidate changed the actual static-V3 DENIED/zero condition"
    );
    assert_eq!(
        compact_whitespace(exact_if_condition(
            production,
            "if phase_a_v4_verification.authorization",
        )),
        "if phase_a_v4_verification.authorization != OfflineAuthorizationState::DENIED || phase_a_v4_verification .authorization .place_dispatch_allowance != 0",
        "candidate changed the actual Phase-A V4 DENIED/zero condition"
    );
    for required in [
        "load_canonical_reviewed_phase_a_eligibility_envelope_v4",
        "ReviewedPhaseAEligibilityEnvelopeContextV4",
        "v1_config: &config",
        "v1_authorization: &authorization",
        "online_policy_v2: &online_policy",
        "online_authorization_v2: &online_authorization",
        "reviewed_production_destination_v1: &destination",
        "reviewed_fresh_credential_slot_locator_v1: &locator",
        "fresh_credential_delivery_binding_v1: &delivery",
        "reviewed_signer_proxy_account_identity_v1: &account_identity",
        "reviewed_remote_credential_proof_policy_v1: &remote_proof_policy",
        "reviewed_static_online_authorization_v3: &static_authorization",
        "verify_reviewed_phase_a_eligibility_envelope_v4(",
        "phase_a_v4_verification.authorization != OfflineAuthorizationState::DENIED",
        "exact_ten_holder_phase_a_v4_envelope_verified: true",
        "canonical_sha256: phase_a_v4.canonical_sha256().to_owned()",
        "canonical_length: phase_a_v4.canonical_length()",
        "fingerprint: phase_a_v4.fingerprint().to_owned()",
        "phase_a_v4_false_boolean_paths_exhaustive",
        "phase_a_v4_verification",
        "authorization: OfflineAuthorizationState::DENIED",
    ] {
        assert!(
            production.contains(required),
            "candidate lost V4 join `{required}`"
        );
    }
    for forbidden in [
        "caller_supplied_v4",
        "v4_authorized: bool",
        "v4_verified: bool",
        "PhaseAV4VerificationInput",
        "PhaseAV4VerificationDto",
    ] {
        assert!(
            !production.contains(forbidden),
            "candidate gained caller-controlled V4 fact `{forbidden}`"
        );
    }
}

#[test]
fn candidate_has_fixed_local_sources_and_no_live_or_secret_authority() {
    let production = production_source();
    for required in [
        "const GIT_PATH: &str = \"/usr/bin/git\";",
        "const PROC_ROOT_PATH: &str = \"/proc\";",
        "const PROC_BOOT_ID_ENTRY: &str = \"sys/kernel/random/boot_id\";",
        "const PROC_THREAD_NET_NAMESPACE_ENTRY: &str = \"thread-self/ns/net\";",
        "const PROC_SELF_EXECUTABLE_ENTRY: &str = \"self/exe\";",
        "GIT_OPTIONAL_LOCKS",
        "GIT_NO_LAZY_FETCH",
        "GIT_TERMINAL_PROMPT",
        "GIT_SSH_COMMAND",
        "core.fsmonitor=false",
        "core.hooksPath=/dev/null",
        "reject_git_external_process_configuration",
        "git_worktree_clean_observed_before_and_after: true",
        "running_release_binary_source: RUNNING_EXECUTABLE_PATH",
        "runtime_nss_username_checked = false",
        "complete_v1_egress_identity_checked = false",
        "complete_v2_egress_identity_checked = false",
        "current_public_egress_identity_checked = false",
        "all_observed_offline_subset_bindings_match",
        "hash_stable_regular_file(",
        "rustix::fs::OFlags::NOFOLLOW",
        "let first = hash_open_file",
        "let second = hash_open_file",
    ] {
        assert!(production.contains(required), "candidate lost `{required}`");
    }
    for forbidden in [
        "FixedEoaSigner",
        "L2Credentials",
        "EoaPrivateKeyInput",
        "L2CredentialInput",
        "AuthenticatedPlaceRequest",
        "authenticate_place_once",
        "send_once",
        "TcpStream",
        "UdpSocket",
        "reqwest",
        "hyper::",
        "SystemTime::now",
        "thread_rng",
        "OsRng",
        "File::create",
        "OpenOptions",
        "fs::write",
        "journal::",
        "getent",
        "lookup_nss_username",
        "all_current_offline_build_and_host_bindings_match",
        "release_binary: PathBuf",
    ] {
        assert!(
            !production.contains(forbidden),
            "candidate gained forbidden `{forbidden}`"
        );
    }
}
