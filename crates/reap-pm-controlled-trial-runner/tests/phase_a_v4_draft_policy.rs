const MAIN: &str = include_str!("../src/main.rs");
const DRAFT: &str = include_str!("../src/phase_a_v4_draft.rs");

fn draft_command_variant() -> &'static str {
    let (_, tail) = MAIN
        .split_once("    DraftNonAuthorizingPhaseAEligibilityEnvelopeV4 {")
        .expect("missing offline V4 draft command");
    let (fields, _) = tail
        .split_once("\n    },\n}\n\n#[tokio::main]")
        .expect("offline V4 draft command must remain the final exact variant");
    fields
}

#[test]
fn command_surface_has_candidate_request_and_non_authorizing_v4_draft() {
    assert_eq!(MAIN.matches("enum Command {").count(), 1);
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
    assert_eq!(
        MAIN.matches("    DraftNonAuthorizingPhaseAEligibilityEnvelopeV4 {")
            .count(),
        1
    );

    let fields = draft_command_variant();
    for required in [
        "config: PathBuf",
        "authorization: PathBuf",
        "online_policy_v2: PathBuf",
        "online_authorization_v2: PathBuf",
        "reviewed_production_destination_v1: PathBuf",
        "reviewed_fresh_credential_slot_locator_v1: PathBuf",
        "fresh_credential_delivery_binding_v1: PathBuf",
        "reviewed_signer_proxy_account_identity_v1: PathBuf",
        "reviewed_remote_credential_proof_policy_v1: PathBuf",
        "reviewed_static_online_authorization_v3: PathBuf",
        "eligibility_record_id: String",
        "reviewer_label: String",
        "reviewed_at_utc: String",
        "not_before_utc: String",
        "expires_at_utc: String",
        "cleanup_not_after_utc: String",
    ] {
        assert!(fields.contains(required), "draft command lost `{required}`");
    }
    for forbidden in [
        "output: PathBuf",
        "output_path",
        "output_file",
        "authorized: bool",
    ] {
        assert!(
            !fields.contains(forbidden),
            "draft command gained forbidden caller field `{forbidden}`"
        );
    }
}

#[test]
fn draft_uses_ten_protected_holders_and_only_context_derived_facts() {
    for required in [
        "load_canonical_trial_config(&paths.config)",
        "load_canonical_authorization(&paths.authorization)",
        "load_canonical_online_policy_v2(&paths.online_policy_v2)",
        "load_canonical_online_authorization_v2(&paths.online_authorization_v2)",
        "&paths.reviewed_production_destination_v1",
        "&paths.reviewed_fresh_credential_slot_locator_v1",
        "&paths.fresh_credential_delivery_binding_v1",
        "&paths.reviewed_signer_proxy_account_identity_v1",
        "&paths.reviewed_remote_credential_proof_policy_v1",
        "&paths.reviewed_static_online_authorization_v3",
        "ReviewedPhaseAEligibilityEnvelopeContextV4 {",
        "draft_non_authorizing_reviewed_phase_a_eligibility_envelope_v4(&context, inputs.clone())",
        "draft_non_authorizing_reviewed_phase_a_eligibility_envelope_v4(&context, inputs)",
    ] {
        assert!(
            DRAFT.contains(required),
            "draft lost exact derivation `{required}`"
        );
    }
    for forbidden in [
        "canonical_sha256: String",
        "fingerprint: String",
        "verified: bool",
        "authorized: bool",
        "current: bool",
        "proof: String",
    ] {
        assert!(
            !DRAFT.contains(forbidden),
            "draft gained caller fact or pin `{forbidden}`"
        );
    }
}

#[test]
fn draft_roundtrip_and_closed_denial_precede_stdout_only_output() {
    for required in [
        "serde_json::to_vec(&drafted)",
        "serde_json::from_slice(&canonical_bytes)",
        "roundtrip != drafted || roundtrip_bytes != canonical_bytes",
        "independently_redrafted != roundtrip",
        "closed_negative_authorization(&roundtrip)",
        "authorization != OfflineAuthorizationState::DENIED",
        "authorization.place_dispatch_allowance != 0",
        "Ok(OfflineAuthorizationState::DENIED)",
        "OfflineEligibilityConjunctionOnlyNoAuthorizationV1",
        "PhaseAExactlyOnePlaceThenExactCancelOnlyV1",
        "RequiredExternalReviewerTrustAnchorUnavailableV1",
        "RequiredAuthenticatedProviderTrustRootUnavailableV1",
        "RequiredProviderSignedAttemptAudienceLeaseUnavailableV1",
        "RequiredAuthoritativeRemoteAcceptanceContractUnavailableV1",
        "RequiredSameHolderLiveRemoteAcceptanceProofUnavailableV1",
        "RequiredAuthoritativeSignerProxyControlContractUnavailableV1",
        "RequiredAccountSpecificCurrentUnrevokedControlProofUnavailableV1",
        "RequiredFutureSelectedActorPreparedLineageUnavailableV1",
        "RequiredFutureRetainedEvidenceLoadTokenJoinUnavailableV1",
        "RequiredFutureCreateNewClaimA3LineageUnavailableV1",
        "RequiredFutureSelectedEgressSingleDispatchOwnerUnavailableV1",
        "write_all(output.canonical_bytes())",
        "protected 0600 file",
    ] {
        assert!(
            DRAFT.contains(required) || MAIN.contains(required),
            "draft lost roundtrip/output boundary `{required}`"
        );
    }
    for forbidden in [
        "println!",
        "std::fs",
        "File::create",
        "OpenOptions",
        "fs::write",
        "std::net",
        "TcpStream",
        "UdpSocket",
        "reqwest",
        "tokio",
        "SystemTime",
        "Utc::now",
        "thread_rng",
        "OsRng",
        "FixedEoaSigner",
        "L2Credentials",
        "journal::",
        "send_once",
        "Permit",
    ] {
        assert!(
            !DRAFT.contains(forbidden),
            "draft command gained forbidden capability `{forbidden}`"
        );
    }
}
