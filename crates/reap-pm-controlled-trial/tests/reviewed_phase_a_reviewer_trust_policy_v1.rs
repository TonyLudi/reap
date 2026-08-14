use std::{
    collections::BTreeSet,
    fs::{self, OpenOptions},
    io::Write as _,
    os::unix::fs::{OpenOptionsExt as _, PermissionsExt as _},
    path::{Path, PathBuf},
};

use tempfile::TempDir;

const PRIMARY_PUBLIC_KEY: &str = "AQEBAQEBAQEBAQEBAQEBAQEBAQEBAQEBAQEBAQEBAQE";
const SECONDARY_PUBLIC_KEY: &str = "AgICAgICAgICAgICAgICAgICAgICAgICAgICAgICAgI";
const VALID_POLICY: &str = r#"{"schema_version":1,"policy_id":"pm-t2-reviewed-phase-a-reviewer-trust-policy-v1","record_role":"offline_reviewer_trust_prerequisite_only_no_authorization_v1","signature_algorithm":"ed25519","maximum_authorization_ttl_seconds":600,"required_reviewer_roles":["phase_a_reviewer_v1"],"required_distinct_approver_quorum":1,"approvers":[{"approver_id":"phase-a-reviewer-primary","role":"phase_a_reviewer_v1","key_id":"phase-a-reviewer-primary-key-v1","public_key_base64url_no_pad":"AQEBAQEBAQEBAQEBAQEBAQEBAQEBAQEBAQEBAQEBAQE"}]}"#;

#[test]
fn exact_policy_has_a_stable_golden_vector_and_is_structural_denied_only() {
    let directory = protected_dir();
    let path = write_0600(
        directory.path(),
        "reviewer-trust-v1.json",
        VALID_POLICY.as_bytes(),
    );
    let policy =
        reap_pm_controlled_trial::load_canonical_reviewed_phase_a_reviewer_trust_policy_v1(&path)
            .unwrap();
    let verification =
        reap_pm_controlled_trial::verify_reviewed_phase_a_reviewer_trust_policy_v1(&policy)
            .unwrap();

    assert_eq!(policy.canonical_length(), 524);
    assert_eq!(
        policy.canonical_sha256(),
        "c1a586149c188d0cdd4ba0ebb3a3a08e9b865c4faf1d13f9b9fa5b41fc3d3cfc"
    );
    assert_eq!(
        policy.fingerprint(),
        "d7a133e95f8392ebffd27069a6b880a3b3d251aea48e3dec274292577844f4b9"
    );
    assert_ne!(policy.canonical_sha256(), policy.fingerprint());
    assert_eq!(verification.schema_version, 1);
    assert_eq!(
        verification.authorization,
        reap_pm_controlled_trial::OfflineAuthorizationState::DENIED
    );

    let expected_true = BTreeSet::from([
        "bounded_unique_approvers_structurally_valid",
        "canonical_unique_32_byte_public_key_encodings_structurally_valid",
        "closed_phase_a_reviewer_role_structurally_valid",
        "ed25519_only_algorithm_structurally_valid",
        "exact_one_reviewer_quorum_structurally_valid",
        "exact_policy_id_and_record_role_structurally_valid",
        "maximum_authorization_ttl_at_most_900_seconds_structurally_valid",
        "unique_approver_ids_and_key_ids_structurally_valid",
    ]);
    let serialized = serde_json::to_value(&verification).unwrap();
    for (name, value) in serialized.as_object().unwrap() {
        if let Some(actual) = value.as_bool() {
            assert_eq!(
                actual,
                expected_true.contains(name.as_str()),
                "unexpected reviewer trust verification boolean for {name}"
            );
        }
    }
    assert_eq!(serialized["place_dispatch_allowance"], 0);
    assert_eq!(serialized["production_order_entry_authorized"], false);
    assert_eq!(serialized["real_order_submission_authorized"], false);
}

#[test]
fn loader_rejects_malformed_duplicate_unknown_trailing_and_noncanonical_bytes() {
    let directory = protected_dir();
    assert_load_rejected(&directory, "malformed.json", b"{");

    let duplicate = VALID_POLICY.replacen(
        r#"{"schema_version":1,"#,
        r#"{"schema_version":1,"schema_version":1,"#,
        1,
    );
    assert_load_rejected(&directory, "duplicate.json", duplicate.as_bytes());

    let unknown = VALID_POLICY.replacen(
        r#"{"schema_version":1,"#,
        r#"{"schema_version":1,"unknown":false,"#,
        1,
    );
    assert_load_rejected(&directory, "unknown.json", unknown.as_bytes());

    let mut trailing = VALID_POLICY.as_bytes().to_vec();
    trailing.push(b'\n');
    assert_load_rejected(&directory, "trailing.json", &trailing);

    let value: serde_json::Value = serde_json::from_str(VALID_POLICY).unwrap();
    let pretty = serde_json::to_vec_pretty(&value).unwrap();
    assert_load_rejected(&directory, "pretty.json", &pretty);

    let reordered = VALID_POLICY.replacen(
        r#"{"schema_version":1,"policy_id":"pm-t2-reviewed-phase-a-reviewer-trust-policy-v1","#,
        r#"{"policy_id":"pm-t2-reviewed-phase-a-reviewer-trust-policy-v1","schema_version":1,"#,
        1,
    );
    assert_load_rejected(&directory, "reordered.json", reordered.as_bytes());
}

#[test]
fn identity_algorithm_role_ttl_and_quorum_are_closed() {
    let directory = protected_dir();
    for (name, changed) in [
        (
            "policy-id.json",
            VALID_POLICY.replace(
                "pm-t2-reviewed-phase-a-reviewer-trust-policy-v1",
                "pm-t2-reviewed-phase-a-reviewer-trust-policy-v2",
            ),
        ),
        (
            "record-role.json",
            VALID_POLICY.replace(
                "offline_reviewer_trust_prerequisite_only_no_authorization_v1",
                "online_authorization_v1",
            ),
        ),
        (
            "algorithm.json",
            VALID_POLICY.replace(
                r#""signature_algorithm":"ed25519""#,
                r#""signature_algorithm":"ecdsa""#,
            ),
        ),
        (
            "unknown-role.json",
            VALID_POLICY.replace("phase_a_reviewer_v1", "operations"),
        ),
        (
            "zero-ttl.json",
            VALID_POLICY.replace(
                r#""maximum_authorization_ttl_seconds":600"#,
                r#""maximum_authorization_ttl_seconds":0"#,
            ),
        ),
        (
            "overlong-ttl.json",
            VALID_POLICY.replace(
                r#""maximum_authorization_ttl_seconds":600"#,
                r#""maximum_authorization_ttl_seconds":901"#,
            ),
        ),
        (
            "zero-quorum.json",
            VALID_POLICY.replace(
                r#""required_distinct_approver_quorum":1"#,
                r#""required_distinct_approver_quorum":0"#,
            ),
        ),
        (
            "widened-quorum.json",
            VALID_POLICY.replace(
                r#""required_distinct_approver_quorum":1"#,
                r#""required_distinct_approver_quorum":2"#,
            ),
        ),
        (
            "missing-role.json",
            VALID_POLICY.replace(
                r#""required_reviewer_roles":["phase_a_reviewer_v1"]"#,
                r#""required_reviewer_roles":[]"#,
            ),
        ),
    ] {
        assert_load_rejected(&directory, name, changed.as_bytes());
    }

    let maximum = VALID_POLICY.replace(
        r#""maximum_authorization_ttl_seconds":600"#,
        r#""maximum_authorization_ttl_seconds":900"#,
    );
    let path = write_0600(directory.path(), "maximum-ttl.json", maximum.as_bytes());
    assert!(
        reap_pm_controlled_trial::load_canonical_reviewed_phase_a_reviewer_trust_policy_v1(&path)
            .is_ok()
    );
}

#[test]
fn approvers_are_bounded_sorted_and_unique_by_identity_key_id_and_public_key() {
    let directory = protected_dir();
    let second = approver(
        "phase-a-reviewer-secondary",
        "phase-a-reviewer-secondary-key-v1",
        SECONDARY_PUBLIC_KEY,
    );
    let two = with_extra_approvers(&[second]);
    let path = write_0600(
        directory.path(),
        "two-eligible-reviewers.json",
        two.as_bytes(),
    );
    assert!(
        reap_pm_controlled_trial::load_canonical_reviewed_phase_a_reviewer_trust_policy_v1(&path)
            .is_ok()
    );

    let empty = VALID_POLICY.replace(
        r#""approvers":[{"approver_id":"phase-a-reviewer-primary","role":"phase_a_reviewer_v1","key_id":"phase-a-reviewer-primary-key-v1","public_key_base64url_no_pad":"AQEBAQEBAQEBAQEBAQEBAQEBAQEBAQEBAQEBAQEBAQE"}]"#,
        r#""approvers":[]"#,
    );
    assert_load_rejected(&directory, "no-approver.json", empty.as_bytes());

    let duplicate_id = with_extra_approvers(&[approver(
        "phase-a-reviewer-primary",
        "phase-a-reviewer-secondary-key-v1",
        SECONDARY_PUBLIC_KEY,
    )]);
    assert_load_rejected(&directory, "duplicate-id.json", duplicate_id.as_bytes());

    let duplicate_key_id = with_extra_approvers(&[approver(
        "phase-a-reviewer-secondary",
        "phase-a-reviewer-primary-key-v1",
        SECONDARY_PUBLIC_KEY,
    )]);
    assert_load_rejected(
        &directory,
        "duplicate-key-id.json",
        duplicate_key_id.as_bytes(),
    );

    let duplicate_public_key = with_extra_approvers(&[approver(
        "phase-a-reviewer-secondary",
        "phase-a-reviewer-secondary-key-v1",
        PRIMARY_PUBLIC_KEY,
    )]);
    assert_load_rejected(
        &directory,
        "duplicate-public-key.json",
        duplicate_public_key.as_bytes(),
    );

    let unsorted = with_extra_approvers(&[approver(
        "a-phase-a-reviewer",
        "a-phase-a-reviewer-key-v1",
        SECONDARY_PUBLIC_KEY,
    )]);
    assert_load_rejected(&directory, "unsorted.json", unsorted.as_bytes());

    let invalid_identifier =
        VALID_POLICY.replace("phase-a-reviewer-primary-key-v1", "Phase-A-Reviewer-Key");
    assert_load_rejected(
        &directory,
        "invalid-identifier.json",
        invalid_identifier.as_bytes(),
    );

    let too_many = (0..8)
        .map(|index| {
            approver(
                &format!("phase-a-reviewer-secondary-{index}"),
                &format!("phase-a-reviewer-secondary-key-{index}"),
                SECONDARY_PUBLIC_KEY,
            )
        })
        .collect::<Vec<_>>();
    let too_many = with_extra_approvers(&too_many);
    assert_load_rejected(&directory, "too-many.json", too_many.as_bytes());
}

#[test]
fn ed25519_public_key_encoding_is_exact_nonzero_32_byte_base64url_without_padding() {
    let directory = protected_dir();
    let all_zero = "AAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAA";
    for (name, key) in [
        ("padded.json", format!("{PRIMARY_PUBLIC_KEY}=")),
        (
            "short.json",
            PRIMARY_PUBLIC_KEY[..PRIMARY_PUBLIC_KEY.len() - 1].to_owned(),
        ),
        ("all-zero.json", all_zero.to_owned()),
        (
            "standard-alphabet.json",
            "//////////////////////////////////////////8".to_owned(),
        ),
        (
            "noncanonical-tail.json",
            "AQEBAQEBAQEBAQEBAQEBAQEBAQEBAQEBAQEBAQEBAQF".to_owned(),
        ),
    ] {
        let changed = VALID_POLICY.replace(PRIMARY_PUBLIC_KEY, &key);
        assert_load_rejected(&directory, name, changed.as_bytes());
    }
}

#[test]
fn policy_requires_protected_file_custody_and_projects_no_policy_values() {
    use std::os::unix::fs::symlink;

    let directory = protected_dir();
    let valid = write_0600(directory.path(), "valid.json", VALID_POLICY.as_bytes());
    let policy =
        reap_pm_controlled_trial::load_canonical_reviewed_phase_a_reviewer_trust_policy_v1(&valid)
            .unwrap();
    assert_eq!(
        format!("{policy:?}"),
        "CanonicalReviewedPhaseAReviewerTrustPolicyV1(<exact-protected-canonical-bytes; no-value-id-role-key-path-signature-runtime-or-authority-projection; redacted; denied>)"
    );
    for hidden in [
        "phase-a-reviewer-primary",
        "phase-a-reviewer-primary-key-v1",
        PRIMARY_PUBLIC_KEY,
    ] {
        assert!(!format!("{policy:?}").contains(hidden));
    }

    let loose = write_0600(directory.path(), "loose.json", VALID_POLICY.as_bytes());
    fs::set_permissions(&loose, fs::Permissions::from_mode(0o644)).unwrap();
    assert!(
        reap_pm_controlled_trial::load_canonical_reviewed_phase_a_reviewer_trust_policy_v1(&loose)
            .is_err()
    );

    let target = write_0600(directory.path(), "target.json", VALID_POLICY.as_bytes());
    let symbolic = directory.path().join("symbolic.json");
    symlink(&target, &symbolic).unwrap();
    assert!(
        reap_pm_controlled_trial::load_canonical_reviewed_phase_a_reviewer_trust_policy_v1(
            &symbolic
        )
        .is_err()
    );

    let hard = directory.path().join("hard.json");
    fs::hard_link(&target, &hard).unwrap();
    assert!(
        reap_pm_controlled_trial::load_canonical_reviewed_phase_a_reviewer_trust_policy_v1(&hard)
            .is_err()
    );

    let loose_directory = tempfile::tempdir().unwrap();
    fs::set_permissions(loose_directory.path(), fs::Permissions::from_mode(0o755)).unwrap();
    let protected_file = write_0600(
        loose_directory.path(),
        "protected-file.json",
        VALID_POLICY.as_bytes(),
    );
    assert!(
        reap_pm_controlled_trial::load_canonical_reviewed_phase_a_reviewer_trust_policy_v1(
            &protected_file
        )
        .is_err()
    );
}

fn approver(id: &str, key_id: &str, public_key: &str) -> String {
    format!(
        r#"{{"approver_id":"{id}","role":"phase_a_reviewer_v1","key_id":"{key_id}","public_key_base64url_no_pad":"{public_key}"}}"#
    )
}

fn with_extra_approvers(extra: &[String]) -> String {
    let prefix = VALID_POLICY
        .strip_suffix("]}")
        .expect("valid policy approver array suffix");
    format!("{prefix},{}]}}", extra.join(","))
}

fn protected_dir() -> TempDir {
    let directory = tempfile::tempdir().unwrap();
    fs::set_permissions(directory.path(), fs::Permissions::from_mode(0o700)).unwrap();
    directory
}

fn write_0600(directory: &Path, name: &str, bytes: &[u8]) -> PathBuf {
    let path = directory.join(name);
    let mut file = OpenOptions::new()
        .write(true)
        .create_new(true)
        .mode(0o600)
        .open(&path)
        .unwrap();
    file.write_all(bytes).unwrap();
    file.sync_all().unwrap();
    path
}

fn assert_load_rejected(directory: &TempDir, name: &str, bytes: &[u8]) {
    let path = write_0600(directory.path(), name, bytes);
    assert!(
        reap_pm_controlled_trial::load_canonical_reviewed_phase_a_reviewer_trust_policy_v1(&path)
            .is_err(),
        "reviewer trust policy loader accepted {name}"
    );
}
