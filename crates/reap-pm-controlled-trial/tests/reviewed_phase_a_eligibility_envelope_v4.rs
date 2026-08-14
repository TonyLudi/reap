// Reuse the exact nine-holder integration fixture without adding a second,
// potentially drifting copy of its 1,000+ lines of canonical records.
include!("reviewed_static_online_authorization_v3.rs");

#[test]
fn exact_ten_holder_envelope_is_structural_and_permanently_denied() {
    let fixture = StaticFixture::new();
    let static_v3 = fixture.load_record("eligibility-bound-static-v3.json", &fixture.record());
    let context = phase_a_eligibility_context(&fixture, &static_v3);
    let record = phase_a_eligibility_record(&context);
    let bytes = serde_json::to_vec(&record).unwrap();
    let path = write_json(&fixture.base.root, "phase-a-eligibility-v4.json", &record);
    let canonical =
        reap_pm_controlled_trial::load_canonical_reviewed_phase_a_eligibility_envelope_v4(&path)
            .unwrap();
    let verification = reap_pm_controlled_trial::verify_reviewed_phase_a_eligibility_envelope_v4(
        &context, &canonical,
    )
    .unwrap();

    assert_eq!(verification.schema_version, 4);
    assert_eq!(
        verification.authorization,
        reap_pm_controlled_trial::OfflineAuthorizationState::DENIED
    );
    assert_eq!(canonical.canonical_length(), bytes.len() as u64);
    assert_eq!(canonical.canonical_sha256(), raw_sha256(&bytes));
    assert_eq!(canonical.fingerprint().len(), 64);
    assert_eq!(
        verification.reviewed_phase_a_eligibility_envelope_v4_fingerprint,
        canonical.fingerprint()
    );

    let true_structural_facts = BTreeSet::from([
        "exact_v1_config_pin_structurally_valid",
        "exact_v1_authorization_pin_structurally_valid",
        "exact_online_policy_v2_pin_structurally_valid",
        "exact_online_authorization_v2_pin_structurally_valid",
        "exact_reviewed_production_destination_v1_pin_structurally_valid",
        "exact_reviewed_fresh_credential_slot_locator_v1_pin_structurally_valid",
        "exact_fresh_credential_delivery_binding_v1_pin_structurally_valid",
        "exact_reviewed_signer_proxy_account_identity_v1_pin_structurally_valid",
        "exact_reviewed_remote_credential_proof_policy_v1_pin_structurally_valid",
        "exact_reviewed_static_online_authorization_v3_pin_structurally_valid",
        "exact_phase_a_scope_and_v2_time_envelope_structurally_valid",
        "frozen_static_v3_reverified_as_denied_structural_evidence",
        "unavailable_external_requirements_structurally_valid",
        "unavailable_runtime_requirements_structurally_valid",
    ]);
    let serialized = serde_json::to_value(&verification).unwrap();
    for (name, value) in serialized.as_object().unwrap() {
        if let Some(actual) = value.as_bool() {
            assert_eq!(
                actual,
                true_structural_facts.contains(name.as_str()),
                "unexpected V4 verification boolean for {name}"
            );
        }
    }
    assert_eq!(serialized["place_dispatch_allowance"], 0);
    assert_eq!(serialized["production_order_entry_authorized"], false);
    assert_eq!(serialized["real_order_submission_authorized"], false);

    let text = String::from_utf8(bytes).unwrap();
    for forbidden_field in [
        "reviewer_public_key",
        "reviewer_signature",
        "provider_public_key",
        "provider_signature",
        "acceptance_contract_sha256",
        "private_key",
        "l2_secret",
        "api_key",
        "passphrase",
        "runtime_nonce",
        "request_body",
        "signed_order",
        "prepared_file",
        "consumption_claim",
    ] {
        assert!(
            !text.contains(forbidden_field),
            "V4 envelope gained caller-supplied trust or capability field {forbidden_field}"
        );
    }
    for required_unavailable in [
        "required_external_reviewer_trust_anchor_unavailable_v1",
        "required_authenticated_provider_trust_root_unavailable_v1",
        "required_provider_signed_attempt_audience_lease_unavailable_v1",
        "required_authoritative_remote_acceptance_contract_unavailable_v1",
        "required_same_holder_live_remote_acceptance_proof_unavailable_v1",
        "required_authoritative_signer_proxy_control_contract_unavailable_v1",
        "required_account_specific_current_unrevoked_control_proof_unavailable_v1",
        "required_future_selected_actor_prepared_lineage_unavailable_v1",
        "required_future_retained_evidence_load_token_join_unavailable_v1",
        "required_future_create_new_claim_a3_lineage_unavailable_v1",
        "required_future_selected_egress_single_dispatch_owner_unavailable_v1",
    ] {
        assert!(text.contains(required_unavailable));
    }

    let debug = format!("{canonical:?}");
    assert!(debug.contains("no-value-proof-actor-runtime-or-authority-projection"));
    assert!(!debug.contains(SIGNER));
    assert!(!debug.contains(REVIEWED_CREDENTIAL_DIRECTORY));
}

#[test]
fn loader_rejects_noncanonical_open_contract_and_unprotected_inputs() {
    use std::os::unix::fs::symlink;

    let fixture = StaticFixture::new();
    let static_v3 = fixture.load_record("loader-bound-static-v3.json", &fixture.record());
    let context = phase_a_eligibility_context(&fixture, &static_v3);
    let record = phase_a_eligibility_record(&context);
    let canonical = serde_json::to_vec(&record).unwrap();

    let duplicate = String::from_utf8(canonical.clone()).unwrap().replacen(
        r#"{"schema_version":4,"#,
        r#"{"schema_version":4,"schema_version":4,"#,
        1,
    );
    assert_v4_load_rejected(&fixture, "v4-duplicate.json", duplicate.as_bytes());

    let unknown = String::from_utf8(canonical.clone()).unwrap().replacen(
        r#"{"schema_version":4,"#,
        r#"{"schema_version":4,"unknown":false,"#,
        1,
    );
    assert_v4_load_rejected(&fixture, "v4-unknown.json", unknown.as_bytes());

    let mut trailing = canonical.clone();
    trailing.push(b'\n');
    assert_v4_load_rejected(&fixture, "v4-trailing.json", &trailing);
    assert_v4_load_rejected(
        &fixture,
        "v4-pretty.json",
        &serde_json::to_vec_pretty(&record).unwrap(),
    );

    let open_reviewer_trust = String::from_utf8(canonical.clone()).unwrap().replacen(
        "required_external_reviewer_trust_anchor_unavailable_v1",
        "caller_supplied_reviewer_key_and_signature_v1",
        1,
    );
    assert_v4_load_rejected(
        &fixture,
        "v4-open-reviewer-trust.json",
        open_reviewer_trust.as_bytes(),
    );

    let open_provider_proof = String::from_utf8(canonical.clone()).unwrap().replacen(
        "required_authenticated_provider_trust_root_unavailable_v1",
        "caller_supplied_provider_digest_v1",
        1,
    );
    assert_v4_load_rejected(
        &fixture,
        "v4-open-provider-proof.json",
        open_provider_proof.as_bytes(),
    );

    let world_readable = fixture.base.root.join("v4-world-readable.json");
    write_0600(&world_readable, &canonical);
    fs::set_permissions(&world_readable, fs::Permissions::from_mode(0o644)).unwrap();
    assert!(
        reap_pm_controlled_trial::load_canonical_reviewed_phase_a_eligibility_envelope_v4(
            &world_readable
        )
        .is_err()
    );

    let target = fixture.base.root.join("v4-link-target.json");
    write_0600(&target, &canonical);
    let symbolic = fixture.base.root.join("v4-symbolic.json");
    symlink(&target, &symbolic).unwrap();
    assert!(
        reap_pm_controlled_trial::load_canonical_reviewed_phase_a_eligibility_envelope_v4(
            &symbolic
        )
        .is_err()
    );

    let hard = fixture.base.root.join("v4-hard.json");
    fs::hard_link(&target, &hard).unwrap();
    assert!(
        reap_pm_controlled_trial::load_canonical_reviewed_phase_a_eligibility_envelope_v4(&hard)
            .is_err()
    );
}

#[test]
fn verifier_rejects_drift_in_every_distinct_role_pin_and_v2_envelope() {
    let fixture = StaticFixture::new();
    let static_v3 = fixture.load_record("drift-bound-static-v3.json", &fixture.record());
    let context = phase_a_eligibility_context(&fixture, &static_v3);

    let mut record = phase_a_eligibility_record(&context);
    record.v1_config.canonical_sha256 = "a1".repeat(32);
    assert_v4_verification_rejected(&fixture, &context, "v4-drift-config.json", &record);

    let mut record = phase_a_eligibility_record(&context);
    record.v1_authorization.canonical_length += 1;
    assert_v4_verification_rejected(&fixture, &context, "v4-drift-v1-auth.json", &record);

    let mut record = phase_a_eligibility_record(&context);
    record.online_policy_v2.fingerprint = "a2".repeat(32);
    assert_v4_verification_rejected(&fixture, &context, "v4-drift-policy.json", &record);

    let mut record = phase_a_eligibility_record(&context);
    record
        .online_authorization_v2
        .authorization_id
        .push_str("-drift");
    assert_v4_verification_rejected(&fixture, &context, "v4-drift-v2-auth.json", &record);

    let mut record = phase_a_eligibility_record(&context);
    record.reviewed_production_destination_v1.canonical_sha256 = "a3".repeat(32);
    assert_v4_verification_rejected(&fixture, &context, "v4-drift-destination.json", &record);

    let mut record = phase_a_eligibility_record(&context);
    record
        .reviewed_fresh_credential_slot_locator_v1
        .locator_id
        .push_str("-drift");
    assert_v4_verification_rejected(&fixture, &context, "v4-drift-locator.json", &record);

    let mut record = phase_a_eligibility_record(&context);
    record.fresh_credential_delivery_binding_v1.canonical_length += 1;
    assert_v4_verification_rejected(&fixture, &context, "v4-drift-delivery.json", &record);

    let mut record = phase_a_eligibility_record(&context);
    record.reviewed_signer_proxy_account_identity_v1.fingerprint = "a4".repeat(32);
    assert_v4_verification_rejected(&fixture, &context, "v4-drift-identity.json", &record);

    let mut record = phase_a_eligibility_record(&context);
    record
        .reviewed_remote_credential_proof_policy_v1
        .canonical_sha256 = "a5".repeat(32);
    assert_v4_verification_rejected(&fixture, &context, "v4-drift-remote.json", &record);

    let mut record = phase_a_eligibility_record(&context);
    record.reviewed_static_online_authorization_v3.fingerprint = "a6".repeat(32);
    assert_v4_verification_rejected(&fixture, &context, "v4-drift-static-v3.json", &record);

    let mut record = phase_a_eligibility_record(&context);
    record.not_before_utc = "2026-08-09T12:00:01Z".into();
    assert_v4_verification_rejected(&fixture, &context, "v4-drift-time.json", &record);
}

fn phase_a_eligibility_context<'a>(
    fixture: &'a StaticFixture,
    static_v3: &'a reap_pm_controlled_trial::CanonicalReviewedStaticOnlineAuthorizationV3,
) -> reap_pm_controlled_trial::ReviewedPhaseAEligibilityEnvelopeContextV4<'a> {
    reap_pm_controlled_trial::ReviewedPhaseAEligibilityEnvelopeContextV4 {
        v1_config: &fixture.base.config,
        v1_authorization: &fixture.v1_authorization,
        online_policy_v2: &fixture.base.online_policy,
        online_authorization_v2: &fixture.base.online_authorization,
        reviewed_production_destination_v1: &fixture.base.reviewed_destination,
        reviewed_fresh_credential_slot_locator_v1: &fixture.base.reviewed_fresh_credential_locator,
        fresh_credential_delivery_binding_v1: &fixture.base.fresh_credential_delivery,
        reviewed_signer_proxy_account_identity_v1: &fixture.base.reviewed_signer_proxy_identity,
        reviewed_remote_credential_proof_policy_v1: &fixture.remote_policy,
        reviewed_static_online_authorization_v3: static_v3,
    }
}

fn phase_a_eligibility_record(
    context: &reap_pm_controlled_trial::ReviewedPhaseAEligibilityEnvelopeContextV4<'_>,
) -> reap_pm_controlled_trial::ReviewedPhaseAEligibilityEnvelopeV4 {
    reap_pm_controlled_trial::draft_non_authorizing_reviewed_phase_a_eligibility_envelope_v4(
        context,
        reap_pm_controlled_trial::ReviewedPhaseAEligibilityEnvelopeDraftInputsV4 {
            eligibility_record_id: "pm-t2-phase-a-eligibility-envelope-v4".into(),
            reviewer_label: "operator-reviewer-unattested-label".into(),
            reviewed_at_utc: "2026-08-09T12:00:00Z".into(),
            not_before_utc: "2026-08-09T12:00:00Z".into(),
            expires_at_utc: "2026-08-09T12:15:00Z".into(),
            cleanup_not_after_utc: "2026-08-09T12:20:00Z".into(),
        },
    )
    .unwrap()
}

fn assert_v4_load_rejected(fixture: &StaticFixture, name: &str, bytes: &[u8]) {
    let path = fixture.base.root.join(name);
    write_0600(&path, bytes);
    assert!(
        reap_pm_controlled_trial::load_canonical_reviewed_phase_a_eligibility_envelope_v4(&path)
            .is_err()
    );
}

fn assert_v4_verification_rejected(
    fixture: &StaticFixture,
    context: &reap_pm_controlled_trial::ReviewedPhaseAEligibilityEnvelopeContextV4<'_>,
    name: &str,
    record: &reap_pm_controlled_trial::ReviewedPhaseAEligibilityEnvelopeV4,
) {
    let path = write_json(&fixture.base.root, name, record);
    let canonical =
        reap_pm_controlled_trial::load_canonical_reviewed_phase_a_eligibility_envelope_v4(&path)
            .unwrap();
    assert!(
        reap_pm_controlled_trial::verify_reviewed_phase_a_eligibility_envelope_v4(
            context, &canonical,
        )
        .is_err()
    );
}
