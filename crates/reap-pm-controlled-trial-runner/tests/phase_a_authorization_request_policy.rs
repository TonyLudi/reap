use sha2::{Digest as _, Sha256};

const MAIN: &str = include_str!("../src/main.rs");
const REQUEST: &str = include_str!("../src/phase_a_authorization_request.rs");
const CANDIDATE: &str = include_str!("../src/phase_a_candidate.rs");
const MANIFEST: &str = include_str!("../Cargo.toml");
const ROOT_MANIFEST: &str = include_str!("../../../Cargo.toml");
const LOCKFILE: &[u8] = include_bytes!("../../../Cargo.lock");

const REQUEST_SHA256: &str = "493ca2b35da61898653a9701207d82fc458b17e8030e24eaa3cdf0aea54b7c8b";
const CANDIDATE_SHA256: &str = "e44aa6fe15ae6df54c4f62096abc3a7ef0d52df4e3001ed1054caa22937ead2d";
const MANIFEST_SHA256: &str = "b8c759e6bd534c103d4b6d5aa7d7cd9dfa50b5d4c78ccabd0ce2f558dd456a66";
const ROOT_MANIFEST_SHA256: &str =
    "91a8730b2ea0327528842bdbdad06af272997e16132da057f31001d05cbdadfc";
const LOCKFILE_SHA256: &str = "215a5197be075c5bb3e69bfc7ddacdf3c0906dd8b9a4ac647c958bd562811f8c";

// This is the direct reviewed-record implementation boundary reached by the
// request command. It is intentionally not described as proof that the whole
// linked runner binary lacks dormant network-capable modules. Those modules
// exist, but the exact MAIN/REQUEST/CANDIDATE route above does not call them.
const DIRECT_REVIEWED_SOURCE_BOUNDARY: &[(&str, &str, &str)] = &[
    (
        "controlled-trial lib",
        include_str!("../../reap-pm-controlled-trial/src/lib.rs"),
        "18a1afc6125d2f1706581d4b2654e72a8462ae62f3c40480c5995f371f2ab8fc",
    ),
    (
        "protected reader",
        include_str!("../../reap-pm-controlled-trial/src/protected_file.rs"),
        "c1ca7b4edd4cebe148ad77ae3fc666e7f17e4135a99c3cab389f36f381492bd2",
    ),
    (
        "V1 config and authorization",
        include_str!("../../reap-pm-controlled-trial/src/config.rs"),
        "f3b78a4f18b60c8c4647174b08939e7c239e4b7f58c917777e68e0d43694a533",
    ),
    (
        "V2 policy and authorization",
        include_str!("../../reap-pm-controlled-trial/src/online_policy_v2.rs"),
        "5c3efd9cc1bc6439486628ad65c7e2e85d44dd9746ba6d2bd87ec62bec8647de",
    ),
    (
        "destination profile",
        include_str!("../../reap-pm-controlled-trial/src/reviewed_destination_profile_v1.rs"),
        "907a2cddae011cef5c92bfe1484942e2d5ca882daaa0799d7a62f6cc3711c967",
    ),
    (
        "credential locator",
        include_str!(
            "../../reap-pm-controlled-trial/src/reviewed_fresh_credential_slot_locator_v1.rs"
        ),
        "cfd3a5089ad7afd11f3e4455187968bd3755456efa8246280005118364dff860",
    ),
    (
        "credential delivery",
        include_str!("../../reap-pm-controlled-trial/src/fresh_credential_delivery_binding_v1.rs"),
        "e84ac2f73a1fc599ee421d5c216a9a1d015a876bff2413770d7f29cd79e370d6",
    ),
    (
        "signer proxy identity",
        include_str!(
            "../../reap-pm-controlled-trial/src/reviewed_signer_proxy_account_identity_v1.rs"
        ),
        "6883ea0ff8a33cc4ebf7fbe79d38bf5519224e026d0ed93a11e09d345a472a44",
    ),
    (
        "remote proof policy",
        include_str!(
            "../../reap-pm-controlled-trial/src/reviewed_remote_credential_proof_policy_v1.rs"
        ),
        "d5395a6c68c91d8e52091b2a6b965e0b0a7e1b0ad97a8721de958e81955e7f1e",
    ),
    (
        "static V3",
        include_str!(
            "../../reap-pm-controlled-trial/src/reviewed_static_online_authorization_v3.rs"
        ),
        "e552892d216dba9fc6b596d73ac4d217438091715e6fcf7469e86a68edfd716e",
    ),
    (
        "eligibility V4",
        include_str!(
            "../../reap-pm-controlled-trial/src/reviewed_phase_a_eligibility_envelope_v4.rs"
        ),
        "34ab2f6ace9023c7f050df9f52bb12f1822b6976e03dc5c6121a2b708c18f53f",
    ),
    (
        "proxy policy",
        include_str!("../../reap-pm-controlled-trial/src/reviewed_poly_proxy_control_policy_v1.rs"),
        "ed216aff43a07f6eecb4fb296cb4ed703fe9392e47a9a1f6c2cfd12c5d5902bd",
    ),
    (
        "local custody profile",
        include_str!(
            "../../reap-pm-controlled-trial/src/reviewed_local_operator_cooperative_custody_profile_v1.rs"
        ),
        "994630cfb4014630ed6ef6db4082cc244a18d10c2766aacff4c7dcbac3374d99",
    ),
    (
        "L1 derivation policy",
        include_str!(
            "../../reap-pm-controlled-trial/src/reviewed_l1_credential_derivation_proof_policy_v1.rs"
        ),
        "809b4881ddd0945ef513aed1345c5322f66b5755820ffa4c76b2d239c3a30a7f",
    ),
];

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
    REQUEST
        .split_once("\n#[cfg(test)]\nmod tests")
        .map_or(REQUEST, |(production, _)| production)
}

fn assert_frozen_direct_request_boundary() {
    assert_eq!(sha256_hex(REQUEST.as_bytes()), REQUEST_SHA256);
    assert_eq!(sha256_hex(CANDIDATE.as_bytes()), CANDIDATE_SHA256);
    assert_eq!(sha256_hex(MANIFEST.as_bytes()), MANIFEST_SHA256);
    assert_eq!(sha256_hex(ROOT_MANIFEST.as_bytes()), ROOT_MANIFEST_SHA256);
    assert_eq!(sha256_hex(LOCKFILE), LOCKFILE_SHA256);
    for (role, source, expected) in DIRECT_REVIEWED_SOURCE_BOUNDARY {
        assert_eq!(
            sha256_hex(source.as_bytes()),
            *expected,
            "direct reviewed request source changed: {role}"
        );
    }
}

#[test]
fn request_sources_are_exactly_frozen() {
    assert_frozen_direct_request_boundary();
}

#[test]
fn request_is_explicitly_not_authorization_and_closed_denied() {
    assert_frozen_direct_request_boundary();
    let source = production_source();
    for required in [
        "phase_a_authorization_request_not_authorization_v1",
        "request_only_not_authorization: true",
        "live_authorization_record_generated: false",
        "caller_supplied_authorization_or_proof_dto_accepted: false",
        "MissingRequirementStatusV1",
        "ReviewerInputIsMissingV1",
        "ExternalProofIsUnavailableV1",
        "RuntimeGateIsNotObservedV1",
        "assert_closed_denial(&proxy_verification.authorization)",
        "assert_closed_denial(&local_verification.authorization)",
        "assert_closed_denial(&l1_verification.authorization)",
        "authorization: OfflineAuthorizationState::DENIED",
        "authorization.place_dispatch_allowance != 0",
    ] {
        assert!(source.contains(required), "request lost `{required}`");
    }
    for forbidden in [
        "pub(crate) status:",
        "status: bool",
        "proof: String",
        "authorized: bool",
        "place_dispatch_allowance: 1",
        "OfflineAuthorizationState {",
    ] {
        assert!(
            !source.contains(forbidden),
            "request gained caller-selected or positive authority `{forbidden}`"
        );
    }
}

#[test]
fn exact_candidate_and_additive_protected_holder_chain_are_preserved() {
    assert_frozen_direct_request_boundary();
    let source = production_source();
    for required in [
        "freeze_phase_a_candidate(FreezePhaseACandidatePaths",
        "release_candidate_snapshot: candidate",
        "candidate.exact_request_chain_matches(",
        "config.canonical_sha256()",
        "config.canonical_length()",
        "phase_a_v4.canonical_sha256()",
        "phase_a_v4.canonical_length()",
        "phase_a_v4.fingerprint()",
        "load_canonical_reviewed_poly_proxy_control_policy_v1(",
        "verify_reviewed_poly_proxy_control_policy_v1(&proxy_policy)",
        "load_canonical_reviewed_local_operator_cooperative_custody_profile_v1(",
        "verify_reviewed_local_operator_cooperative_custody_profile_v1(",
        "load_canonical_reviewed_l1_credential_derivation_proof_policy_v1(",
        "verify_reviewed_l1_credential_derivation_proof_policy_v1(",
        "phase_a_context(",
        "reviewed_phase_a_eligibility_envelope_v4: &phase_a_v4",
        "reviewed_local_operator_cooperative_custody_profile_v1: &local_custody",
        "config.exact_place_public_request_identity()",
        "trial_plan_fingerprint: config.plan_fingerprint().to_owned()",
        "expected_order_id: place_identity.expected_order_id().to_string()",
        "semantic_request_commitment: place_identity",
        "exact_nonsecret_trial_config = config.value().clone()",
        "ExactNonsecretTrialConfigAndPublicPlanFingerprintApproval",
        "non-secret config does not mean previously or generally publicly disclosed.",
        "false_boolean_paths(&proxy_verification)",
        "false_boolean_paths(&local_verification)",
        "false_boolean_paths(&l1_verification)",
    ] {
        assert!(
            source.contains(required),
            "request lost exact chain `{required}`"
        );
    }
    for forbidden in [
        "exact_public_trial_config",
        "ExactPublicTrialConfig",
        "exact already-public trial config",
    ] {
        assert!(
            !source.contains(forbidden),
            "request overstated protected config disclosure as `{forbidden}`"
        );
    }
    assert!(REQUEST.contains("exact_nonsecret_trial_config_and_public_plan_fingerprint_approval"));
    for artifact in [
        "canonical_config_v1",
        "authorization_v1",
        "online_policy_v2",
        "online_authorization_v2",
        "reviewed_production_destination_v1",
        "reviewed_fresh_credential_slot_locator_v1",
        "fresh_credential_delivery_binding_v1",
        "reviewed_signer_proxy_account_identity_v1",
        "reviewed_remote_credential_proof_policy_v1",
        "reviewed_static_online_authorization_v3",
        "reviewed_phase_a_eligibility_envelope_v4",
        "reviewed_poly_proxy_control_policy_v1",
        "reviewed_local_operator_cooperative_custody_profile_v1",
        "reviewed_l1_credential_derivation_proof_policy_v1",
    ] {
        assert!(
            source.contains(artifact),
            "request lost artifact role `{artifact}`"
        );
    }
}

#[test]
fn request_route_adds_no_direct_secret_network_journal_clock_or_file_output_surface() {
    assert_frozen_direct_request_boundary();
    let source = production_source();
    for forbidden in [
        "EoaPrivateKeyInput",
        "L2CredentialInput",
        "L2Credentials",
        "FixedEoaSigner",
        "private_key: PathBuf",
        "api_key: PathBuf",
        "l2_secret: PathBuf",
        "passphrase: PathBuf",
        "std::net",
        "TcpStream",
        "UdpSocket",
        "reqwest",
        "tokio",
        "SystemTime",
        "Utc::now",
        "thread_rng",
        "OsRng",
        "journal::",
        "Permit",
        "File::create",
        "OpenOptions",
        "fs::write",
        "output_path",
        "output_file",
        "POST /order",
        "DELETE /order",
        "send_once",
    ] {
        assert!(
            !source.contains(forbidden),
            "request crossed forbidden boundary `{forbidden}`"
        );
    }
    // The runner manifest deliberately links dormant network-capable crates;
    // this test makes no whole-binary capability claim. Exact source and
    // dependency-input hashes above scope the assertion to this command route.
    assert!(MANIFEST.contains("reap-polymarket-live-adapter.workspace = true"));
}

#[test]
fn main_exposes_one_stdout_only_request_command() {
    assert_frozen_direct_request_boundary();
    assert_eq!(
        MAIN.matches("    GeneratePhaseAAuthorizationRequestNotAuthorization {")
            .count(),
        1
    );
    assert_eq!(
        MAIN.matches("Command::GeneratePhaseAAuthorizationRequestNotAuthorization {")
            .count(),
        1
    );
    for required in [
        "reviewed_poly_proxy_control_policy_v1: PathBuf",
        "reviewed_local_operator_cooperative_custody_profile_v1: PathBuf",
        "reviewed_l1_credential_derivation_proof_policy_v1: PathBuf",
        "generate_phase_a_authorization_request_not_authorization(",
        "serde_json::to_vec(&request)",
        "stdout().lock().write_all(&canonical_bytes)",
    ] {
        assert!(MAIN.contains(required), "request CLI lost `{required}`");
    }
    for forbidden in ["request_status", "proof_status", "authorized: bool"] {
        assert!(
            !MAIN.contains(forbidden),
            "request CLI gained forbidden caller field `{forbidden}`"
        );
    }
}
