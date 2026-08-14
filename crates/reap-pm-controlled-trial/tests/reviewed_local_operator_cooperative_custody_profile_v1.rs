// Reuse the exact V4 ten-holder fixture so this profile cannot drift into a
// parallel set of canonical inputs.
include!("reviewed_phase_a_eligibility_envelope_v4.rs");

const GOLDEN_CLOSED_TRUST_GRAMMAR_JSON: &str = r#"{"same_euid_trust":"exact_authorized_euid_account_and_all_same_euid_actors_trusted_quiescent_for_complete_window_v1","directory_lease":"continuous_advisory_directory_lease_coordination_only_v1","source_descriptor_custody":"one_held_directory_fd_and_four_held_credential_source_fds_v1","recovery_retention":"three_named_l2_entries_persist_for_recovery_until_terminal_v1","cleanup_observation":"conditional_expected_basename_removal_directory_fsync_then_absence_observation_under_continuous_lease_v1","atomic_unlink_limitation":"no_atomic_unlink_if_name_still_identifies_held_inode_claim_v1","secure_erasure_limitation":"name_removal_and_userspace_drop_are_not_secure_erasure_v1","provider_origin_limitation":"no_credential_provider_origin_or_authorship_claim_v1","global_uniqueness_limitation":"no_global_credential_delivery_uniqueness_claim_v1","currentness_limitation":"no_credential_currentness_claim_v1","revocation_limitation":"no_credential_or_delivery_revocation_state_claim_v1","other_copies_limitation":"no_absence_of_other_descriptors_or_credential_copies_claim_v1"}"#;

#[test]
fn exact_local_operator_profile_is_structural_closed_and_permanently_denied() {
    let fixture = StaticFixture::new();
    let static_v3 = fixture.load_record("local-profile-bound-static-v3.json", &fixture.record());
    let v4_context = phase_a_eligibility_context(&fixture, &static_v3);
    let v4_record = phase_a_eligibility_record(&v4_context);
    let v4_path = write_json(
        &fixture.base.root,
        "local-profile-bound-v4.json",
        &v4_record,
    );
    let v4 =
        reap_pm_controlled_trial::load_canonical_reviewed_phase_a_eligibility_envelope_v4(&v4_path)
            .unwrap();
    let context = local_operator_context(&fixture, &static_v3, &v4);

    let first = reap_pm_controlled_trial::draft_non_authorizing_reviewed_local_operator_cooperative_custody_profile_v1(&context).unwrap();
    let second = reap_pm_controlled_trial::draft_non_authorizing_reviewed_local_operator_cooperative_custody_profile_v1(&context).unwrap();
    assert_eq!(first, second);
    assert_eq!(
        serde_json::to_string(&first.cooperative_trust).unwrap(),
        GOLDEN_CLOSED_TRUST_GRAMMAR_JSON
    );
    let bytes = serde_json::to_vec(&first).unwrap();
    assert_eq!(
        serde_json::from_slice::<serde_json::Value>(&bytes).unwrap(),
        serde_json::to_value(&first).unwrap()
    );
    let path = write_json(&fixture.base.root, "local-operator-custody-v1.json", &first);
    let canonical = reap_pm_controlled_trial::load_canonical_reviewed_local_operator_cooperative_custody_profile_v1(&path).unwrap();
    let verification =
        reap_pm_controlled_trial::verify_reviewed_local_operator_cooperative_custody_profile_v1(
            &context, &canonical,
        )
        .unwrap();

    assert_eq!(verification.schema_version, 1);
    assert_eq!(canonical.canonical_length(), bytes.len() as u64);
    assert_eq!(canonical.canonical_sha256(), raw_sha256(&bytes));
    assert_eq!(canonical.fingerprint().len(), 64);
    assert_eq!(
        verification.authorization,
        reap_pm_controlled_trial::OfflineAuthorizationState::DENIED
    );

    let true_structural_facts = BTreeSet::from([
        "exact_canonical_config_pin_structurally_valid",
        "exact_online_policy_v2_pin_structurally_valid",
        "exact_online_authorization_v2_pin_structurally_valid",
        "exact_reviewed_static_online_authorization_v3_pin_structurally_valid",
        "exact_reviewed_phase_a_eligibility_envelope_v4_pin_structurally_valid",
        "exact_reviewed_fresh_credential_slot_locator_v1_pin_structurally_valid",
        "exact_fresh_credential_delivery_binding_v1_pin_structurally_valid",
        "exact_build_host_account_window_labels_match_canonical_holders",
        "closed_local_operator_trust_grammar_structurally_valid",
        "v4_reverified_unchanged_and_denied",
        "future_v5_selection_label_structurally_valid",
    ]);
    let serialized = serde_json::to_value(&verification).unwrap();
    for (name, value) in serialized.as_object().unwrap() {
        if let Some(actual) = value.as_bool() {
            assert_eq!(
                actual,
                true_structural_facts.contains(name.as_str()),
                "unexpected local-operator custody verification boolean for {name}"
            );
        }
    }
    assert_eq!(serialized["place_dispatch_allowance"], 0);
    assert_eq!(serialized["production_order_entry_authorized"], false);
    assert_eq!(serialized["real_order_submission_authorized"], false);

    let text = String::from_utf8(bytes).unwrap();
    for required in [
        "offline_local_trust_alternative_only_no_authorization_v1",
        "exact_authorized_euid_account_and_all_same_euid_actors_trusted_quiescent_for_complete_window_v1",
        "continuous_advisory_directory_lease_coordination_only_v1",
        "one_held_directory_fd_and_four_held_credential_source_fds_v1",
        "three_named_l2_entries_persist_for_recovery_until_terminal_v1",
        "conditional_expected_basename_removal_directory_fsync_then_absence_observation_under_continuous_lease_v1",
        "no_atomic_unlink_if_name_still_identifies_held_inode_claim_v1",
        "name_removal_and_userspace_drop_are_not_secure_erasure_v1",
        "no_credential_provider_origin_or_authorship_claim_v1",
        "no_global_credential_delivery_uniqueness_claim_v1",
        "no_credential_currentness_claim_v1",
        "no_credential_or_delivery_revocation_state_claim_v1",
        "no_absence_of_other_descriptors_or_credential_copies_claim_v1",
        "future_v5_may_select_this_alternative_v4_remains_unchanged_denied_v1",
    ] {
        assert!(
            text.contains(required),
            "missing closed trust term {required}"
        );
    }
    for forbidden in [
        "protected_fresh_credential_directory",
        "private_key",
        "api_key",
        "l2_secret",
        "passphrase",
        "provider_signature",
        "lease_signature",
        "runtime_nonce",
        "actor_generation",
        "request_body",
        "signed_order",
        "dispatch_owner",
        "permit",
    ] {
        assert!(
            !text.contains(forbidden),
            "local-operator profile gained secret/path/proof/capability field {forbidden}"
        );
    }

    let debug = format!("{canonical:?}");
    assert!(debug.contains("no-value-path-fd-proof-runtime-or-authority-projection"));
    assert!(!debug.contains(SIGNER));
    assert!(!debug.contains(REVIEWED_CREDENTIAL_DIRECTORY));
}

#[test]
fn loader_rejects_noncanonical_open_contract_and_unprotected_local_profile() {
    use std::os::unix::fs::symlink;

    let fixture = StaticFixture::new();
    let static_v3 = fixture.load_record("local-loader-static-v3.json", &fixture.record());
    let v4_context = phase_a_eligibility_context(&fixture, &static_v3);
    let v4_record = phase_a_eligibility_record(&v4_context);
    let v4_path = write_json(&fixture.base.root, "local-loader-v4.json", &v4_record);
    let v4 =
        reap_pm_controlled_trial::load_canonical_reviewed_phase_a_eligibility_envelope_v4(&v4_path)
            .unwrap();
    let context = local_operator_context(&fixture, &static_v3, &v4);
    let record = reap_pm_controlled_trial::draft_non_authorizing_reviewed_local_operator_cooperative_custody_profile_v1(&context).unwrap();
    let canonical = serde_json::to_vec(&record).unwrap();

    let duplicate = String::from_utf8(canonical.clone()).unwrap().replacen(
        r#"{"schema_version":1,"#,
        r#"{"schema_version":1,"schema_version":1,"#,
        1,
    );
    assert_local_profile_load_rejected(&fixture, "local-duplicate.json", duplicate.as_bytes());

    let unknown = String::from_utf8(canonical.clone()).unwrap().replacen(
        r#"{"schema_version":1,"#,
        r#"{"schema_version":1,"unknown":false,"#,
        1,
    );
    assert_local_profile_load_rejected(&fixture, "local-unknown.json", unknown.as_bytes());

    let mut trailing = canonical.clone();
    trailing.push(b'\n');
    assert_local_profile_load_rejected(&fixture, "local-trailing.json", &trailing);
    assert_local_profile_load_rejected(
        &fixture,
        "local-pretty.json",
        &serde_json::to_vec_pretty(&record).unwrap(),
    );

    let widened_lease = String::from_utf8(canonical.clone()).unwrap().replacen(
        "continuous_advisory_directory_lease_coordination_only_v1",
        "mandatory_hostile_same_euid_exclusion_v1",
        1,
    );
    assert_local_profile_load_rejected(
        &fixture,
        "local-widened-lease.json",
        widened_lease.as_bytes(),
    );

    let world_readable = fixture.base.root.join("local-world-readable.json");
    write_0600(&world_readable, &canonical);
    fs::set_permissions(&world_readable, fs::Permissions::from_mode(0o644)).unwrap();
    assert!(
        reap_pm_controlled_trial::load_canonical_reviewed_local_operator_cooperative_custody_profile_v1(&world_readable).is_err()
    );

    let target = fixture.base.root.join("local-link-target.json");
    write_0600(&target, &canonical);
    let symbolic = fixture.base.root.join("local-symbolic.json");
    symlink(&target, &symbolic).unwrap();
    assert!(
        reap_pm_controlled_trial::load_canonical_reviewed_local_operator_cooperative_custody_profile_v1(&symbolic).is_err()
    );
    let hard = fixture.base.root.join("local-hard.json");
    fs::hard_link(&target, &hard).unwrap();
    assert!(
        reap_pm_controlled_trial::load_canonical_reviewed_local_operator_cooperative_custody_profile_v1(&hard).is_err()
    );
}

#[test]
fn verifier_rejects_every_distinct_pin_audience_and_trust_drift() {
    let fixture = StaticFixture::new();
    let static_v3 = fixture.load_record("local-drift-static-v3.json", &fixture.record());
    let v4_context = phase_a_eligibility_context(&fixture, &static_v3);
    let v4_record = phase_a_eligibility_record(&v4_context);
    let v4_path = write_json(&fixture.base.root, "local-drift-v4.json", &v4_record);
    let v4 =
        reap_pm_controlled_trial::load_canonical_reviewed_phase_a_eligibility_envelope_v4(&v4_path)
            .unwrap();
    let context = local_operator_context(&fixture, &static_v3, &v4);
    let base = reap_pm_controlled_trial::draft_non_authorizing_reviewed_local_operator_cooperative_custody_profile_v1(&context).unwrap();

    let mut drift = base.clone();
    drift.canonical_config.trial_plan_fingerprint = "a1".repeat(32);
    assert_local_profile_verification_rejected(
        &fixture,
        &context,
        "local-drift-config.json",
        &drift,
    );

    let mut drift = base.clone();
    drift.online_policy_v2.canonical_length += 1;
    assert_local_profile_verification_rejected(
        &fixture,
        &context,
        "local-drift-policy.json",
        &drift,
    );

    let mut drift = base.clone();
    drift
        .online_authorization_v2
        .authorization_id
        .push_str("-drift");
    assert_local_profile_verification_rejected(&fixture, &context, "local-drift-auth.json", &drift);

    let mut drift = base.clone();
    drift.reviewed_static_online_authorization_v3.fingerprint = "a2".repeat(32);
    assert_local_profile_verification_rejected(&fixture, &context, "local-drift-v3.json", &drift);

    let mut drift = base.clone();
    drift
        .reviewed_phase_a_eligibility_envelope_v4
        .canonical_sha256 = "a3".repeat(32);
    assert_local_profile_verification_rejected(
        &fixture,
        &context,
        "local-drift-v4-pin.json",
        &drift,
    );

    let mut drift = base.clone();
    drift
        .reviewed_fresh_credential_slot_locator_v1
        .canonical_length += 1;
    assert_local_profile_verification_rejected(
        &fixture,
        &context,
        "local-drift-locator.json",
        &drift,
    );

    let mut drift = base.clone();
    drift.fresh_credential_delivery_binding_v1.fingerprint = "a4".repeat(32);
    assert_local_profile_verification_rejected(
        &fixture,
        &context,
        "local-drift-delivery.json",
        &drift,
    );

    let mut drift = base.clone();
    drift.audience.linux_euid += 1;
    assert_local_profile_verification_rejected(
        &fixture,
        &context,
        "local-drift-audience.json",
        &drift,
    );

    let mut drift = base;
    drift.audience.not_before_utc = "2026-08-09T12:00:01Z".into();
    assert_local_profile_verification_rejected(
        &fixture,
        &context,
        "local-drift-window.json",
        &drift,
    );
}

fn local_operator_context<'a>(
    fixture: &'a StaticFixture,
    static_v3: &'a reap_pm_controlled_trial::CanonicalReviewedStaticOnlineAuthorizationV3,
    v4: &'a reap_pm_controlled_trial::CanonicalReviewedPhaseAEligibilityEnvelopeV4,
) -> reap_pm_controlled_trial::ReviewedLocalOperatorCooperativeCustodyProfileContextV1<'a> {
    reap_pm_controlled_trial::ReviewedLocalOperatorCooperativeCustodyProfileContextV1 {
        phase_a_eligibility_context: phase_a_eligibility_context(fixture, static_v3),
        reviewed_phase_a_eligibility_envelope_v4: v4,
    }
}

fn assert_local_profile_load_rejected(fixture: &StaticFixture, name: &str, bytes: &[u8]) {
    let path = fixture.base.root.join(name);
    write_0600(&path, bytes);
    assert!(
        reap_pm_controlled_trial::load_canonical_reviewed_local_operator_cooperative_custody_profile_v1(&path).is_err()
    );
}

fn assert_local_profile_verification_rejected(
    fixture: &StaticFixture,
    context: &reap_pm_controlled_trial::ReviewedLocalOperatorCooperativeCustodyProfileContextV1<'_>,
    name: &str,
    record: &reap_pm_controlled_trial::ReviewedLocalOperatorCooperativeCustodyProfileV1,
) {
    let path = write_json(&fixture.base.root, name, record);
    let canonical = reap_pm_controlled_trial::load_canonical_reviewed_local_operator_cooperative_custody_profile_v1(&path).unwrap();
    assert!(
        reap_pm_controlled_trial::verify_reviewed_local_operator_cooperative_custody_profile_v1(
            context, &canonical
        )
        .is_err()
    );
}
