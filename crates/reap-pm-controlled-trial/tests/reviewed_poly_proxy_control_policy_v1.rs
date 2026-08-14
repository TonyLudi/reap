use std::{
    collections::BTreeSet,
    fs,
    os::unix::fs::{PermissionsExt as _, symlink},
    path::{Path, PathBuf},
};

use reap_pm_controlled_trial::{
    OfflineAuthorizationState, PM_POLY_PROXY_FACTORY_ADDRESS_V1,
    PM_POLY_PROXY_IMPLEMENTATION_ADDRESS_V1, PM_POLY_PROXY_INIT_CODE_BYTE_LENGTH_V1,
    PM_POLY_PROXY_INIT_CODE_HEX_V1, PM_POLY_PROXY_INIT_CODE_KECCAK256_V1,
    PM_POLY_PROXY_NEGATIVE_RISK_V2_EXCHANGE_ADDRESS_V1,
    PM_POLY_PROXY_OWNER_STORAGE_SLOT_LITERAL_V1, PM_POLY_PROXY_OWNER_UTF8_KECCAK256_V1,
    PM_POLY_PROXY_POLYGON_CHAIN_ID_V1, PM_POLY_PROXY_RUNTIME_BYTE_LENGTH_V1,
    PM_POLY_PROXY_RUNTIME_HEX_V1, PM_POLY_PROXY_RUNTIME_KECCAK256_V1,
    PM_POLY_PROXY_STANDARD_V2_EXCHANGE_ADDRESS_V1,
    REVIEWED_POLY_PROXY_CONTROL_POLICY_V1_SCHEMA_VERSION, ReviewedPolyProxyControlPolicyV1,
    ReviewedPolyProxyCreate2AddressRelationV1, ReviewedPolyProxyCreate2PolicyV1,
    ReviewedPolyProxyDeterministicRelationLimitationV1, ReviewedPolyProxyExchangeOraclePolicyV1,
    ReviewedPolyProxyExclusiveControlLimitationV1, ReviewedPolyProxyFutureProofPolicyV1,
    ReviewedPolyProxyFutureProofRequirementV1, ReviewedPolyProxyInitCodeV1,
    ReviewedPolyProxyInitializerEncodingV1, ReviewedPolyProxyLimitationsV1,
    ReviewedPolyProxyOtherSignatureTypesStatusV1, ReviewedPolyProxyOwnerStoragePolicyV1,
    ReviewedPolyProxyOwnerStorageSlotCommentStatusV1, ReviewedPolyProxyOwnerStorageSlotSourceV1,
    ReviewedPolyProxyRecordRoleV1, ReviewedPolyProxyRequiredEvidenceStatusV1,
    ReviewedPolyProxyRuntimeRelationV1, ReviewedPolyProxyRuntimeTemplateV1,
    ReviewedPolyProxySignerSaltRelationV1, ReviewedPolyProxyStoredOwnerRelationV1,
    ReviewedPolyProxyStructuralPolicyV1, ReviewedPolyProxyTypeOneRelationV1,
    ReviewedPolyProxyTypeZeroRelationV1, ReviewedPolyProxyUnattestedSourceLabelsV1,
    ReviewedPolyProxyUnavailableEvidenceV1, load_canonical_reviewed_poly_proxy_control_policy_v1,
    verify_reviewed_poly_proxy_control_policy_v1,
};
use serde_json::Value;
use sha2::Sha256;
use sha3::{Digest as _, Keccak256};

const GOLDEN_CANONICAL_LENGTH: u64 = 5_582;
const GOLDEN_CANONICAL_SHA256: &str =
    "e7067f402de9ba2100088199c7dbf493c82e073cd322e71252645e4fb4c26b99";
const GOLDEN_FINGERPRINT: &str = "6abdcf70eb430fc11e6437f7f52abbe09476950b0e7bb14e9f41af929d7f03c6";
const FINGERPRINT_DOMAIN: &[u8] =
    b"reap.pm-t2.controlled-trial.reviewed-poly-proxy-control-policy.v1\0";

fn record() -> ReviewedPolyProxyControlPolicyV1 {
    ReviewedPolyProxyControlPolicyV1 {
        schema_version: REVIEWED_POLY_PROXY_CONTROL_POLICY_V1_SCHEMA_VERSION,
        record_role: ReviewedPolyProxyRecordRoleV1::NonAuthorizingStructuralPolicyOnlyV1,
        unattested_source_labels: ReviewedPolyProxyUnattestedSourceLabelsV1 {
            observed_date_utc: "2026-08-11".to_owned(),
            ctf_exchange_v2_commit: "ccc0596074f4dfd62c944fbca4de252893b82b4b".to_owned(),
            ctf_signatures_path: "src/exchange/mixins/Signatures.sol".to_owned(),
            ctf_poly_factory_helper_path: "src/exchange/mixins/PolyFactoryHelper.sol".to_owned(),
            proxy_factories_commit: "7137c021e6954d671095f77c94afc3d083d10a84".to_owned(),
            ts_sdk_commit: "e19e87da2670a7162ddce1674870fece4521d196".to_owned(),
            magic_example_commit: "a8cd96a7bfe725c085bc7d4f882519b60e4e6f05".to_owned(),
        },
        structural_policy: ReviewedPolyProxyStructuralPolicyV1 {
            polygon_chain_id: PM_POLY_PROXY_POLYGON_CHAIN_ID_V1,
            factory_address: PM_POLY_PROXY_FACTORY_ADDRESS_V1.to_owned(),
            implementation_address: PM_POLY_PROXY_IMPLEMENTATION_ADDRESS_V1.to_owned(),
            create2: ReviewedPolyProxyCreate2PolicyV1 {
                signer_salt_relation:
                    ReviewedPolyProxySignerSaltRelationV1::Keccak256ExactAbiEncodePackedTwentyByteSignerV1,
                abi_padded_thirty_two_byte_signer_input_forbidden: true,
                address_relation:
                    ReviewedPolyProxyCreate2AddressRelationV1::LowTwentyBytesOfKeccak256FfFactorySaltInitCodeHashV1,
                init_code: ReviewedPolyProxyInitCodeV1 {
                    byte_length: PM_POLY_PROXY_INIT_CODE_BYTE_LENGTH_V1,
                    hex: PM_POLY_PROXY_INIT_CODE_HEX_V1.to_owned(),
                    keccak256: PM_POLY_PROXY_INIT_CODE_KECCAK256_V1.to_owned(),
                    initializer_selector_hex: "52e831dd".to_owned(),
                    initializer_encoding:
                        ReviewedPolyProxyInitializerEncodingV1::SelectorThenAbiEncodeSingleEmptyBytesValueV1,
                },
            },
            runtime_template: ReviewedPolyProxyRuntimeTemplateV1 {
                byte_length: PM_POLY_PROXY_RUNTIME_BYTE_LENGTH_V1,
                hex: PM_POLY_PROXY_RUNTIME_HEX_V1.to_owned(),
                keccak256: PM_POLY_PROXY_RUNTIME_KECCAK256_V1.to_owned(),
                relation:
                    ReviewedPolyProxyRuntimeRelationV1::Eip1167DelegateProxyToExactImplementationV1,
            },
            owner_storage: ReviewedPolyProxyOwnerStoragePolicyV1 {
                slot_literal: PM_POLY_PROXY_OWNER_STORAGE_SLOT_LITERAL_V1.to_owned(),
                slot_source:
                    ReviewedPolyProxyOwnerStorageSlotSourceV1::ExactLiteralFromProxyWalletLibV1,
                upstream_inline_comment_status:
                    ReviewedPolyProxyOwnerStorageSlotCommentStatusV1::UnauthenticatedSourceCommentErrorKeccak256OwnerIsFalseV1,
                actual_keccak256_of_utf8_owner: PM_POLY_PROXY_OWNER_UTF8_KECCAK256_V1.to_owned(),
                slot_literal_differs_from_actual_keccak256_of_utf8_owner: true,
                stored_owner_relation:
                    ReviewedPolyProxyStoredOwnerRelationV1::ExactFactoryAddressBecauseFactoryCallsInitializeV1,
                decoded_owner_must_equal_factory_address: PM_POLY_PROXY_FACTORY_ADDRESS_V1
                    .to_owned(),
            },
        },
        exchange_oracle_policy: ReviewedPolyProxyExchangeOraclePolicyV1 {
            standard_v2_exchange_address: PM_POLY_PROXY_STANDARD_V2_EXCHANGE_ADDRESS_V1.to_owned(),
            negative_risk_v2_exchange_address:
                PM_POLY_PROXY_NEGATIVE_RISK_V2_EXCHANGE_ADDRESS_V1.to_owned(),
            selected_signature_type: 1,
            type_zero_relation:
                ReviewedPolyProxyTypeZeroRelationV1::EoaMakerEqualsRecoveredSignerV1,
            type_one_relation:
                ReviewedPolyProxyTypeOneRelationV1::NonemptyEcdsaRecoveredSignerAndExchangeDerivedMakerV1,
            type_two_and_three_status:
                ReviewedPolyProxyOtherSignatureTypesStatusV1::NotAdmittedWithoutSeparateExactReviewedPinsV1,
            type_one_signature_must_be_nonempty: true,
            ecdsa_recovered_address_must_equal_declared_signer: true,
            get_proxy_wallet_address_signature: "getProxyWalletAddress(address)".to_owned(),
            get_proxy_factory_signature: "getProxyFactory()".to_owned(),
            get_proxy_implementation_signature: "getProxyImplementation()".to_owned(),
            get_proxy_wallet_address_of_signer_must_equal_maker: true,
            selected_exchange_factory_getter_must_equal_exact_factory: true,
            selected_exchange_implementation_getter_must_equal_exact_implementation: true,
        },
        unavailable_evidence: ReviewedPolyProxyUnavailableEvidenceV1 {
            exact_source_bytes_and_publisher_authorship:
                ReviewedPolyProxyRequiredEvidenceStatusV1::RequiredButUnavailableV1,
            deployed_code_source_correspondence:
                ReviewedPolyProxyRequiredEvidenceStatusV1::RequiredButUnavailableV1,
            authenticated_finalized_polygon_header_and_state_root:
                ReviewedPolyProxyRequiredEvidenceStatusV1::RequiredButUnavailableV1,
            locally_verified_account_code_and_storage_mpt_bundle:
                ReviewedPolyProxyRequiredEvidenceStatusV1::RequiredButUnavailableV1,
            current_factory_governance_and_module_state:
                ReviewedPolyProxyRequiredEvidenceStatusV1::RequiredButUnavailableV1,
            current_proxy_code_and_owner_state:
                ReviewedPolyProxyRequiredEvidenceStatusV1::RequiredButUnavailableV1,
            current_selected_exchange_code_and_getter_results:
                ReviewedPolyProxyRequiredEvidenceStatusV1::RequiredButUnavailableV1,
            actor_bound_fresh_signer_challenge:
                ReviewedPolyProxyRequiredEvidenceStatusV1::RequiredButUnavailableV1,
            provider_authorship_and_non_equivocation:
                ReviewedPolyProxyRequiredEvidenceStatusV1::RequiredButUnavailableV1,
            proof_freshness_and_reorg_monitoring:
                ReviewedPolyProxyRequiredEvidenceStatusV1::RequiredButUnavailableV1,
        },
        future_proof_policy: ReviewedPolyProxyFutureProofPolicyV1 {
            exact_source_bytes_and_publisher_authorship:
                ReviewedPolyProxyFutureProofRequirementV1::RequiredV1,
            deployed_code_source_correspondence:
                ReviewedPolyProxyFutureProofRequirementV1::RequiredV1,
            trusted_finalized_polygon_header_and_state_root:
                ReviewedPolyProxyFutureProofRequirementV1::RequiredV1,
            same_state_root_for_all_account_and_storage_proofs:
                ReviewedPolyProxyFutureProofRequirementV1::RequiredV1,
            locally_verified_account_mpt_proofs:
                ReviewedPolyProxyFutureProofRequirementV1::RequiredV1,
            locally_verified_code_bytes_and_keccak:
                ReviewedPolyProxyFutureProofRequirementV1::RequiredV1,
            locally_verified_storage_mpt_proofs:
                ReviewedPolyProxyFutureProofRequirementV1::RequiredV1,
            factory_governance_owner_and_module_state:
                ReviewedPolyProxyFutureProofRequirementV1::RequiredV1,
            proxy_runtime_code_and_exact_implementation:
                ReviewedPolyProxyFutureProofRequirementV1::RequiredV1,
            exact_literal_proxy_owner_slot_storage_proof_equals_factory:
                ReviewedPolyProxyFutureProofRequirementV1::RequiredV1,
            selected_exchange_code_factory_and_implementation_getters:
                ReviewedPolyProxyFutureProofRequirementV1::RequiredV1,
            selected_exchange_get_proxy_wallet_address_call:
                ReviewedPolyProxyFutureProofRequirementV1::RequiredV1,
            type_one_nonempty_signature_and_recovered_signer:
                ReviewedPolyProxyFutureProofRequirementV1::RequiredV1,
            actor_bound_fresh_signer_possession_challenge:
                ReviewedPolyProxyFutureProofRequirementV1::RequiredV1,
            authenticated_provider_and_non_equivocation:
                ReviewedPolyProxyFutureProofRequirementV1::RequiredV1,
            bounded_observation_freshness:
                ReviewedPolyProxyFutureProofRequirementV1::RequiredV1,
            finalized_reorg_detection_and_invalidation:
                ReviewedPolyProxyFutureProofRequirementV1::RequiredV1,
        },
        limitations: ReviewedPolyProxyLimitationsV1 {
            deterministic_relation:
                ReviewedPolyProxyDeterministicRelationLimitationV1::StructuralOnlyNoDeploymentStateOrControlClaimV1,
            exclusive_control:
                ReviewedPolyProxyExclusiveControlLimitationV1::LegacyGovernanceAndModulesPreventExclusiveControlSemanticsV1,
            signer_control_if_future_proven_is_nonexclusive: true,
            deterministic_address_is_not_deployment_evidence: true,
            source_labels_are_not_source_authorship_or_deployed_correspondence: true,
            provider_statement_is_not_a_locally_verified_state_proof: true,
        },
    }
}

fn write_protected(directory: &Path, name: &str, bytes: &[u8]) -> PathBuf {
    fs::set_permissions(directory, fs::Permissions::from_mode(0o700)).unwrap();
    let path = directory.join(name);
    fs::write(&path, bytes).unwrap();
    fs::set_permissions(&path, fs::Permissions::from_mode(0o600)).unwrap();
    path
}

fn load_value(
    directory: &Path,
    name: &str,
    value: &Value,
) -> Result<
    reap_pm_controlled_trial::CanonicalReviewedPolyProxyControlPolicyV1,
    reap_pm_controlled_trial::PmReviewedPolyProxyControlPolicyV1Error,
> {
    let bytes = serde_json::to_vec(value).unwrap();
    let path = write_protected(directory, name, &bytes);
    load_canonical_reviewed_poly_proxy_control_policy_v1(&path)
}

fn raw_sha256(domain: &[u8], bytes: &[u8]) -> String {
    let mut digest = Sha256::new();
    digest.update(domain);
    digest.update(bytes);
    format!("{:x}", digest.finalize())
}

fn decode_hex(value: &str) -> Vec<u8> {
    assert_eq!(value.len() % 2, 0, "hex input must contain whole bytes");
    value
        .as_bytes()
        .chunks_exact(2)
        .map(|pair| (hex_nibble(pair[0]) << 4) | hex_nibble(pair[1]))
        .collect()
}

fn hex_nibble(byte: u8) -> u8 {
    match byte {
        b'0'..=b'9' => byte - b'0',
        b'a'..=b'f' => byte - b'a' + 10,
        b'A'..=b'F' => byte - b'A' + 10,
        _ => panic!("non-hex byte in frozen vector"),
    }
}

fn lower_hex(bytes: &[u8]) -> String {
    bytes.iter().map(|byte| format!("{byte:02x}")).collect()
}

fn selector_hex(signature: &str) -> String {
    lower_hex(&Keccak256::digest(signature.as_bytes())[..4])
}

#[test]
fn exact_canonical_golden_loads_and_is_permanently_denied() {
    let directory = tempfile::tempdir().unwrap();
    let bytes = serde_json::to_vec(&record()).unwrap();
    assert_eq!(bytes.len() as u64, GOLDEN_CANONICAL_LENGTH);
    assert_eq!(raw_sha256(&[], &bytes), GOLDEN_CANONICAL_SHA256);
    assert_eq!(raw_sha256(FINGERPRINT_DOMAIN, &bytes), GOLDEN_FINGERPRINT);

    let path = write_protected(directory.path(), "poly-proxy-policy.json", &bytes);
    let canonical = load_canonical_reviewed_poly_proxy_control_policy_v1(&path).unwrap();
    assert_eq!(canonical.canonical_length(), GOLDEN_CANONICAL_LENGTH);
    assert_eq!(canonical.canonical_sha256(), GOLDEN_CANONICAL_SHA256);
    assert_eq!(canonical.fingerprint(), GOLDEN_FINGERPRINT);

    let verification = verify_reviewed_poly_proxy_control_policy_v1(&canonical).unwrap();
    assert_eq!(
        verification.authorization,
        OfflineAuthorizationState::DENIED
    );
    assert_eq!(
        verification.reviewed_poly_proxy_control_policy_fingerprint,
        GOLDEN_FINGERPRINT
    );

    let true_structural_facts = BTreeSet::from([
        "exact_unattested_source_labels_structurally_valid",
        "exact_chain_factory_and_implementation_structurally_valid",
        "exact_packed_signer_salt_and_create2_grammar_structurally_valid",
        "exact_init_code_and_keccak_label_structurally_valid",
        "exact_runtime_template_and_keccak_label_structurally_valid",
        "exact_literal_owner_slot_comment_discrepancy_and_factory_relation_structurally_valid",
        "exact_exchange_oracle_and_type_discrimination_structurally_valid",
        "required_unavailable_evidence_statuses_structurally_valid",
        "future_proof_requirements_structurally_valid",
        "structural_and_legacy_governance_limitations_valid",
    ]);
    let serialized = serde_json::to_value(&verification).unwrap();
    for (name, value) in serialized.as_object().unwrap() {
        if let Some(actual) = value.as_bool() {
            assert_eq!(
                actual,
                true_structural_facts.contains(name.as_str()),
                "unexpected proxy-control verification boolean for {name}"
            );
        }
    }
    assert_eq!(serialized["place_dispatch_allowance"], 0);
    assert_eq!(serialized["production_order_entry_authorized"], false);
    assert_eq!(serialized["real_order_submission_authorized"], false);
}

#[test]
fn exact_structural_constants_and_closed_requirements_do_not_drift() {
    let bytes = serde_json::to_vec(&record()).unwrap();
    let text = std::str::from_utf8(&bytes).unwrap();
    for required in [
        "0xaB45c5A4B0c941a2F231C04C3f49182e1A254052",
        "0x44e999d5c2F66Ef0861317f9A4805AC2e90aEB4f",
        "0xE111180000d2663C0091e4f400237545B87B996B",
        "0xe2222d279d744050d28e00520010520000310F59",
        PM_POLY_PROXY_INIT_CODE_HEX_V1,
        PM_POLY_PROXY_INIT_CODE_KECCAK256_V1,
        PM_POLY_PROXY_RUNTIME_HEX_V1,
        PM_POLY_PROXY_RUNTIME_KECCAK256_V1,
        PM_POLY_PROXY_OWNER_STORAGE_SLOT_LITERAL_V1,
        PM_POLY_PROXY_OWNER_UTF8_KECCAK256_V1,
        "keccak256_exact_abi_encode_packed_twenty_byte_signer_v1",
        "low_twenty_bytes_of_keccak256_ff_factory_salt_init_code_hash_v1",
        "exact_literal_from_proxy_wallet_lib_v1",
        "unauthenticated_source_comment_error_keccak256_owner_is_false_v1",
        "exact_factory_address_because_factory_calls_initialize_v1",
        "getProxyWalletAddress(address)",
        "getProxyFactory()",
        "getProxyImplementation()",
        "not_admitted_without_separate_exact_reviewed_pins_v1",
        "legacy_governance_and_modules_prevent_exclusive_control_semantics_v1",
    ] {
        assert!(
            text.contains(required),
            "missing exact proxy policy value {required}"
        );
    }
    assert_eq!(text.matches("required_but_unavailable_v1").count(), 10);
    assert_eq!(text.matches("required_v1").count(), 17);
    assert_eq!(PM_POLY_PROXY_INIT_CODE_HEX_V1.len(), 167 * 2);
    assert_eq!(PM_POLY_PROXY_RUNTIME_HEX_V1.len(), 45 * 2);
}

#[test]
fn independent_ethereum_keccak_selectors_and_create2_vectors_match_exact_literals() {
    let init_code = decode_hex(PM_POLY_PROXY_INIT_CODE_HEX_V1);
    let runtime = decode_hex(PM_POLY_PROXY_RUNTIME_HEX_V1);
    let factory = decode_hex(&PM_POLY_PROXY_FACTORY_ADDRESS_V1[2..]);
    let implementation = decode_hex(&PM_POLY_PROXY_IMPLEMENTATION_ADDRESS_V1[2..]);
    assert_eq!(init_code.len(), 167);
    assert_eq!(runtime.len(), 45);
    assert_eq!(
        lower_hex(&Keccak256::digest(&init_code)),
        PM_POLY_PROXY_INIT_CODE_KECCAK256_V1
    );
    assert_eq!(
        lower_hex(&Keccak256::digest(&runtime)),
        PM_POLY_PROXY_RUNTIME_KECCAK256_V1
    );
    assert_eq!(&init_code[13..33], factory.as_slice());
    assert_eq!(&init_code[64..84], implementation.as_slice());
    assert_eq!(&runtime[10..30], implementation.as_slice());
    assert_eq!(&init_code[99..103], &[0x52, 0xe8, 0x31, 0xdd]);
    assert_eq!(selector_hex("cloneConstructor(bytes)"), "52e831dd");

    let policy = record();
    assert_eq!(
        policy
            .structural_policy
            .create2
            .init_code
            .initializer_selector_hex,
        selector_hex("cloneConstructor(bytes)")
    );
    assert_eq!(
        selector_hex(
            &policy
                .exchange_oracle_policy
                .get_proxy_wallet_address_signature
        ),
        "58d8b6bb"
    );
    assert_eq!(
        selector_hex(&policy.exchange_oracle_policy.get_proxy_factory_signature),
        "b28c51c0"
    );
    assert_eq!(
        selector_hex(
            &policy
                .exchange_oracle_policy
                .get_proxy_implementation_signature
        ),
        "90e4b720"
    );

    let actual_owner_hash = format!("0x{:x}", Keccak256::digest(b"owner"));
    assert_eq!(actual_owner_hash, PM_POLY_PROXY_OWNER_UTF8_KECCAK256_V1);
    assert_ne!(
        actual_owner_hash,
        PM_POLY_PROXY_OWNER_STORAGE_SLOT_LITERAL_V1
    );
    let owner = &policy.structural_policy.owner_storage;
    assert_eq!(
        owner.slot_source,
        ReviewedPolyProxyOwnerStorageSlotSourceV1::ExactLiteralFromProxyWalletLibV1
    );
    assert_eq!(
        owner.slot_literal,
        PM_POLY_PROXY_OWNER_STORAGE_SLOT_LITERAL_V1
    );
    assert_eq!(owner.actual_keccak256_of_utf8_owner, actual_owner_hash);
    assert!(owner.slot_literal_differs_from_actual_keccak256_of_utf8_owner);
    assert_eq!(
        owner.upstream_inline_comment_status,
        ReviewedPolyProxyOwnerStorageSlotCommentStatusV1::UnauthenticatedSourceCommentErrorKeccak256OwnerIsFalseV1
    );

    let mut signer = [0_u8; 20];
    signer[19] = 1;
    let packed_salt = Keccak256::digest(signer);
    assert_eq!(
        lower_hex(&packed_salt),
        "1468288056310c82aa4c01a7e12a10f8111a0560e72b700555479031b86c357d"
    );
    let mut padded_signer = [0_u8; 32];
    padded_signer[12..].copy_from_slice(&signer);
    let padded_salt = Keccak256::digest(padded_signer);
    assert_ne!(packed_salt, padded_salt);

    let init_code_hash = decode_hex(PM_POLY_PROXY_INIT_CODE_KECCAK256_V1);
    let derive = |salt: &[u8]| {
        let mut preimage = Vec::with_capacity(85);
        preimage.push(0xff);
        preimage.extend_from_slice(&factory);
        preimage.extend_from_slice(salt);
        preimage.extend_from_slice(&init_code_hash);
        let digest = Keccak256::digest(preimage);
        lower_hex(&digest[12..])
    };
    assert_eq!(
        derive(&packed_salt),
        "7754536ecd85c00b2e0cf9c1aa679340d8550756"
    );
    assert_ne!(
        derive(&padded_salt),
        "7754536ecd85c00b2e0cf9c1aa679340d8550756"
    );
}

#[test]
fn source_unavailability_has_distinct_one_to_one_future_requirements() {
    let policy = record();
    let mappings = [
        (
            "exact_source_bytes_and_publisher_authorship",
            policy
                .unavailable_evidence
                .exact_source_bytes_and_publisher_authorship,
            policy
                .future_proof_policy
                .exact_source_bytes_and_publisher_authorship,
        ),
        (
            "deployed_code_source_correspondence",
            policy
                .unavailable_evidence
                .deployed_code_source_correspondence,
            policy
                .future_proof_policy
                .deployed_code_source_correspondence,
        ),
    ];
    assert_eq!(mappings.len(), 2);
    let mut names = BTreeSet::new();
    for (name, unavailable, required) in mappings {
        assert!(
            names.insert(name),
            "duplicate unavailable-to-future mapping"
        );
        assert_eq!(
            unavailable,
            ReviewedPolyProxyRequiredEvidenceStatusV1::RequiredButUnavailableV1
        );
        assert_eq!(
            required,
            ReviewedPolyProxyFutureProofRequirementV1::RequiredV1
        );
    }
}

#[test]
fn every_constant_family_and_closed_enum_drift_is_rejected() {
    let directory = tempfile::tempdir().unwrap();
    let base = serde_json::to_value(record()).unwrap();
    let mutations: &[(&str, &[&str], Value)] = &[
        (
            "chain",
            &["structural_policy", "polygon_chain_id"],
            Value::from(1),
        ),
        (
            "factory",
            &["structural_policy", "factory_address"],
            Value::from("0x0000000000000000000000000000000000000000"),
        ),
        (
            "implementation",
            &["structural_policy", "implementation_address"],
            Value::from("0x0000000000000000000000000000000000000000"),
        ),
        (
            "init-code",
            &["structural_policy", "create2", "init_code", "hex"],
            Value::from("00"),
        ),
        (
            "init-hash",
            &["structural_policy", "create2", "init_code", "keccak256"],
            Value::from("00"),
        ),
        (
            "runtime",
            &["structural_policy", "runtime_template", "hex"],
            Value::from("00"),
        ),
        (
            "owner-slot",
            &["structural_policy", "owner_storage", "slot_literal"],
            Value::from("0x00"),
        ),
        (
            "owner-slot-source",
            &["structural_policy", "owner_storage", "slot_source"],
            Value::from("derived_keccak_v1"),
        ),
        (
            "owner-comment-status",
            &[
                "structural_policy",
                "owner_storage",
                "upstream_inline_comment_status",
            ],
            Value::from("authenticated_correct_comment_v1"),
        ),
        (
            "actual-owner-keccak",
            &[
                "structural_policy",
                "owner_storage",
                "actual_keccak256_of_utf8_owner",
            ],
            Value::from(PM_POLY_PROXY_OWNER_STORAGE_SLOT_LITERAL_V1),
        ),
        (
            "owner-slot-discrepancy",
            &[
                "structural_policy",
                "owner_storage",
                "slot_literal_differs_from_actual_keccak256_of_utf8_owner",
            ],
            Value::from(false),
        ),
        (
            "decoded-owner",
            &[
                "structural_policy",
                "owner_storage",
                "decoded_owner_must_equal_factory_address",
            ],
            Value::from("0x0000000000000000000000000000000000000000"),
        ),
        (
            "exchange",
            &["exchange_oracle_policy", "standard_v2_exchange_address"],
            Value::from("0x0000000000000000000000000000000000000000"),
        ),
        (
            "signature-type",
            &["exchange_oracle_policy", "selected_signature_type"],
            Value::from(0),
        ),
        (
            "source-commit",
            &["unattested_source_labels", "ctf_exchange_v2_commit"],
            Value::from("0000000000000000000000000000000000000000"),
        ),
        (
            "unavailable-status",
            &[
                "unavailable_evidence",
                "exact_source_bytes_and_publisher_authorship",
            ],
            Value::from("available_v1"),
        ),
        (
            "future-requirement",
            &[
                "future_proof_policy",
                "locally_verified_code_bytes_and_keccak",
            ],
            Value::from("optional_v1"),
        ),
        (
            "future-source-authorship-requirement",
            &[
                "future_proof_policy",
                "exact_source_bytes_and_publisher_authorship",
            ],
            Value::from("optional_v1"),
        ),
        (
            "future-source-correspondence-requirement",
            &["future_proof_policy", "deployed_code_source_correspondence"],
            Value::from("optional_v1"),
        ),
        (
            "legacy-limit",
            &["limitations", "exclusive_control"],
            Value::from("exclusive_v1"),
        ),
    ];

    for (index, (name, path, replacement)) in mutations.iter().enumerate() {
        let mut changed = base.clone();
        let mut cursor = &mut changed;
        for component in &path[..path.len() - 1] {
            cursor = cursor.get_mut(*component).unwrap();
        }
        cursor[path[path.len() - 1]] = replacement.clone();
        assert!(
            load_value(directory.path(), &format!("drift-{index}.json"), &changed).is_err(),
            "proxy policy accepted drift in {name}"
        );
    }
}

#[test]
fn loader_rejects_noncanonical_unknown_duplicate_and_unprotected_files() {
    let directory = tempfile::tempdir().unwrap();
    let bytes = serde_json::to_vec(&record()).unwrap();

    let pretty = serde_json::to_vec_pretty(&record()).unwrap();
    let pretty_path = write_protected(directory.path(), "pretty.json", &pretty);
    assert!(load_canonical_reviewed_poly_proxy_control_policy_v1(&pretty_path).is_err());

    let mut trailing = bytes.clone();
    trailing.push(b'\n');
    let trailing_path = write_protected(directory.path(), "trailing.json", &trailing);
    assert!(load_canonical_reviewed_poly_proxy_control_policy_v1(&trailing_path).is_err());

    let text = String::from_utf8(bytes.clone()).unwrap();
    let duplicate = text.replacen(
        r#"{"schema_version":1,"#,
        r#"{"schema_version":1,"schema_version":1,"#,
        1,
    );
    let duplicate_path = write_protected(directory.path(), "duplicate.json", duplicate.as_bytes());
    assert!(load_canonical_reviewed_poly_proxy_control_policy_v1(&duplicate_path).is_err());

    let mut unknown = serde_json::to_value(record()).unwrap();
    unknown["caller_proof"] = Value::from("untrusted");
    let unknown_path = write_protected(
        directory.path(),
        "unknown.json",
        &serde_json::to_vec(&unknown).unwrap(),
    );
    assert!(load_canonical_reviewed_poly_proxy_control_policy_v1(&unknown_path).is_err());

    let loose_path = write_protected(directory.path(), "loose.json", &bytes);
    fs::set_permissions(&loose_path, fs::Permissions::from_mode(0o644)).unwrap();
    assert!(load_canonical_reviewed_poly_proxy_control_policy_v1(&loose_path).is_err());

    let target = write_protected(directory.path(), "target.json", &bytes);
    let link = directory.path().join("link.json");
    symlink(&target, &link).unwrap();
    assert!(load_canonical_reviewed_poly_proxy_control_policy_v1(&link).is_err());
}

#[test]
fn canonical_holder_and_verification_do_not_project_account_or_proof_values() {
    let directory = tempfile::tempdir().unwrap();
    let bytes = serde_json::to_vec(&record()).unwrap();
    let path = write_protected(directory.path(), "privacy.json", &bytes);
    let canonical = load_canonical_reviewed_poly_proxy_control_policy_v1(&path).unwrap();
    let debug = format!("{canonical:?}");
    assert_eq!(
        debug,
        "CanonicalReviewedPolyProxyControlPolicyV1(<exact-protected-canonical-bytes; no-value-signer-maker-proxy-proof-provider-actor-or-authority-projection; redacted; denied>)"
    );
    for public_value_not_projected in [
        PM_POLY_PROXY_FACTORY_ADDRESS_V1,
        PM_POLY_PROXY_IMPLEMENTATION_ADDRESS_V1,
        PM_POLY_PROXY_INIT_CODE_HEX_V1,
        PM_POLY_PROXY_OWNER_STORAGE_SLOT_LITERAL_V1,
        "ccc0596074f4dfd62c944fbca4de252893b82b4b",
    ] {
        assert!(!debug.contains(public_value_not_projected));
    }

    let verification = verify_reviewed_poly_proxy_control_policy_v1(&canonical).unwrap();
    let verification_text = serde_json::to_string(&verification).unwrap();
    for public_value_not_projected in [
        PM_POLY_PROXY_FACTORY_ADDRESS_V1,
        PM_POLY_PROXY_IMPLEMENTATION_ADDRESS_V1,
        PM_POLY_PROXY_INIT_CODE_HEX_V1,
        PM_POLY_PROXY_OWNER_STORAGE_SLOT_LITERAL_V1,
    ] {
        assert!(!verification_text.contains(public_value_not_projected));
    }

    let record_text = String::from_utf8(bytes).unwrap();
    for forbidden_field in [
        "private_key",
        "api_key",
        "l2_secret",
        "passphrase",
        "credential",
        "rpc_url",
        "block_number",
        "block_hash",
        "state_root_value",
        "proof_nodes",
        "signer_address",
        "maker_address",
        "proxy_address",
        "challenge_bytes",
        "signature_bytes",
        "provider_public_key",
        "provider_signature",
        "runtime_nonce",
        "actor_generation",
        "request_body",
        "dispatch_owner",
        "slot_preimage_utf8",
    ] {
        assert!(
            !record_text.contains(forbidden_field),
            "proxy policy gained caller/account/proof/capability field {forbidden_field}"
        );
    }
}
