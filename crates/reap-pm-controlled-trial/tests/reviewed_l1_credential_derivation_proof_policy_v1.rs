// Reuse the exact local-custody/V4/V3 fixture chain so this additive policy
// cannot silently drift into a parallel set of canonical prerequisite inputs.
include!("reviewed_local_operator_cooperative_custody_profile_v1.rs");

const L1_DERIVATION_FINGERPRINT_DOMAIN: &[u8] =
    b"reap.pm-t2.controlled-trial.reviewed-l1-credential-derivation-proof-policy.v1\0";
const L1_DERIVATION_GOLDEN_CANONICAL_LENGTH: u64 = 7_482;
const L1_DERIVATION_GOLDEN_CANONICAL_SHA256: &str =
    "8f0a0929a8b3c93ae0d3c55b11ddffa53787991fc52c040ab7eec380e63f90cf";
const L1_DERIVATION_GOLDEN_FINGERPRINT: &str =
    "0cbf0fc8a0246af8ede6221170f87fa452841549666041774779802a323a3471";

#[test]
fn exact_l1_derivation_policy_is_canonical_structural_and_permanently_denied() {
    let fixture = L1DerivationFixture::new();
    let record = fixture.record();
    let bytes = serde_json::to_vec(&record).unwrap();
    let canonical = fixture.load_record("l1-derivation-policy-v1.json", &record);
    let verification =
        reap_pm_controlled_trial::verify_reviewed_l1_credential_derivation_proof_policy_v1(
            &fixture.context(),
            &canonical,
        )
        .unwrap();

    assert_eq!(verification.schema_version, 1);
    assert_eq!(
        verification.authorization,
        reap_pm_controlled_trial::OfflineAuthorizationState::DENIED
    );
    assert_eq!(verification.credential_derivation_dispatch_allowance, 0);
    assert_eq!(verification.cancel_dispatch_allowance, 0);
    assert_eq!(canonical.canonical_length(), bytes.len() as u64);
    assert_eq!(canonical.canonical_sha256(), raw_sha256(&bytes));
    assert_eq!(
        canonical.fingerprint(),
        domain_fingerprint(L1_DERIVATION_FINGERPRINT_DOMAIN, &bytes)
    );

    let debug = format!("{canonical:?}");
    assert!(debug.contains("redacted; denied"));
    assert!(!debug.contains(SIGNER));
    assert!(!debug.contains(FUNDER));
}

#[test]
fn l1_derivation_policy_has_stable_golden_length_sha_and_domain_fingerprint() {
    let fixture = L1DerivationFixture::new();
    let record = fixture.record();
    let bytes = serde_json::to_vec(&record).unwrap();
    let canonical = fixture.load_record("l1-derivation-golden-v1.json", &record);

    assert_eq!(bytes.len() as u64, L1_DERIVATION_GOLDEN_CANONICAL_LENGTH);
    assert_eq!(raw_sha256(&bytes), L1_DERIVATION_GOLDEN_CANONICAL_SHA256);
    assert_eq!(
        domain_fingerprint(L1_DERIVATION_FINGERPRINT_DOMAIN, &bytes),
        L1_DERIVATION_GOLDEN_FINGERPRINT
    );
    assert_eq!(
        canonical.canonical_sha256(),
        L1_DERIVATION_GOLDEN_CANONICAL_SHA256
    );
    assert_eq!(canonical.fingerprint(), L1_DERIVATION_GOLDEN_FINGERPRINT);
}

#[test]
fn loader_rejects_unknown_duplicate_trailing_noncanonical_and_unprotected_records() {
    let fixture = L1DerivationFixture::new();
    let record = fixture.record();
    let bytes = serde_json::to_vec(&record).unwrap();

    let mut unknown = br#"{"unknown":true,"#.to_vec();
    unknown.extend_from_slice(&bytes[1..]);
    fixture.assert_load_rejected("l1-unknown.json", &unknown);

    let mut duplicate = br#"{"schema_version":1,"#.to_vec();
    duplicate.extend_from_slice(&bytes[1..]);
    fixture.assert_load_rejected("l1-duplicate.json", &duplicate);

    let mut trailing = bytes.clone();
    trailing.push(b'\n');
    fixture.assert_load_rejected("l1-trailing.json", &trailing);

    let pretty = serde_json::to_vec_pretty(&record).unwrap();
    fixture.assert_load_rejected("l1-pretty.json", &pretty);

    let path = fixture.base.base.root.join("l1-world-readable.json");
    write_0600(&path, &bytes);
    fs::set_permissions(&path, fs::Permissions::from_mode(0o644)).unwrap();
    assert!(
        reap_pm_controlled_trial::load_canonical_reviewed_l1_credential_derivation_proof_policy_v1(
            &path
        )
        .is_err()
    );

    let target = fixture.base.base.root.join("l1-hardlink-target.json");
    write_0600(&target, &bytes);
    let hardlink = fixture.base.base.root.join("l1-hardlink.json");
    fs::hard_link(&target, &hardlink).unwrap();
    assert!(
        reap_pm_controlled_trial::load_canonical_reviewed_l1_credential_derivation_proof_policy_v1(
            &hardlink
        )
        .is_err()
    );

    let symlink = fixture.base.base.root.join("l1-symlink.json");
    std::os::unix::fs::symlink(&target, &symlink).unwrap();
    assert!(
        reap_pm_controlled_trial::load_canonical_reviewed_l1_credential_derivation_proof_policy_v1(
            &symlink
        )
        .is_err()
    );
}

#[test]
fn verifier_rejects_drift_in_every_distinct_typed_pin() {
    let fixture = L1DerivationFixture::new();
    let baseline = fixture.record();
    let mut cases = Vec::new();

    macro_rules! drift_pin {
        ($field:ident) => {{
            let mut record = baseline.clone();
            record.$field.canonical_sha256 = different_sha(&record.$field.canonical_sha256);
            cases.push(record);
        }};
    }
    drift_pin!(v1_config);
    drift_pin!(online_policy_v2);
    drift_pin!(online_authorization_v2);
    drift_pin!(reviewed_production_destination_v1);
    drift_pin!(fresh_credential_delivery_binding_v1);
    drift_pin!(reviewed_signer_proxy_account_identity_v1);
    drift_pin!(reviewed_remote_credential_proof_policy_v1);
    drift_pin!(reviewed_static_online_authorization_v3);
    drift_pin!(reviewed_phase_a_eligibility_envelope_v4);
    drift_pin!(reviewed_local_operator_cooperative_custody_profile_v1);

    let mut plan = baseline.clone();
    plan.v1_config.plan_fingerprint = different_sha(&plan.v1_config.plan_fingerprint);
    cases.push(plan);

    for (index, record) in cases.iter().enumerate() {
        fixture.assert_verification_rejected(&format!("l1-pin-drift-{index}.json"), record);
    }
}

#[test]
fn every_closed_endpoint_request_eip712_and_time_rule_rejects_drift() {
    let fixture = L1DerivationFixture::new();
    let baseline = fixture.record();
    let mut cases = Vec::new();

    macro_rules! drift_string {
        ($($field:ident).+) => {{
            let mut record = baseline.clone();
            record.$($field).+.push_str("-drift");
            cases.push(record);
        }};
    }
    macro_rules! drift_number {
        ($($field:ident).+) => {{
            let mut record = baseline.clone();
            record.$($field).+ = record.$($field).+.saturating_add(1);
            cases.push(record);
        }};
    }

    drift_string!(protocol.endpoint.scheme);
    drift_string!(protocol.endpoint.dns_name);
    drift_string!(protocol.endpoint.tls_server_name);
    drift_string!(protocol.endpoint.http_host);
    drift_number!(protocol.endpoint.tcp_port);
    drift_string!(protocol.endpoint.selected_peer_ip);
    drift_number!(protocol.endpoint.network_namespace_device);
    drift_number!(protocol.endpoint.network_namespace_inode);
    drift_string!(protocol.endpoint.interface_name);
    drift_number!(protocol.endpoint.interface_index);
    drift_string!(protocol.endpoint.local_egress_ip);
    drift_string!(
        protocol
            .endpoint
            .dedicated_tunnel_or_gateway_profile_reference
    );
    drift_string!(protocol.endpoint.dedicated_tunnel_or_gateway_profile_sha256);

    drift_string!(protocol.request.method);
    drift_string!(protocol.request.path);
    drift_string!(protocol.request.query);
    drift_string!(protocol.request.body);
    drift_string!(protocol.request.content_type);
    drift_string!(protocol.request.accept);
    drift_string!(protocol.request.accept_encoding);
    drift_number!(protocol.request.exact_header_count);
    drift_string!(protocol.request.header_names.poly_address);
    drift_string!(protocol.request.header_names.poly_signature);
    drift_string!(protocol.request.header_names.poly_timestamp);
    drift_string!(protocol.request.header_names.poly_nonce);

    drift_string!(protocol.request.eip712.standard);
    drift_string!(protocol.request.eip712.domain_type);
    drift_string!(protocol.request.eip712.domain_name);
    drift_string!(protocol.request.eip712.domain_version);
    drift_number!(protocol.request.eip712.domain_chain_id);
    drift_string!(protocol.request.eip712.primary_type);
    drift_string!(protocol.request.eip712.struct_type);
    drift_string!(protocol.request.eip712.address_source);
    drift_string!(protocol.request.eip712.timestamp_source);
    drift_number!(protocol.request.eip712.timestamp_decimal_digits);
    drift_number!(protocol.request.eip712.minimum_timestamp_unix_seconds);
    drift_number!(protocol.request.eip712.maximum_timestamp_unix_seconds);
    drift_number!(protocol.request.eip712.nonce_value);
    drift_string!(protocol.request.eip712.nonce_source);
    drift_string!(protocol.request.eip712.message);
    drift_string!(protocol.request.eip712.signature_source);

    drift_string!(protocol.time.clob_time_path);
    drift_number!(protocol.time.maximum_timestamp_creation_to_request_send_ms);
    drift_number!(
        protocol
            .time
            .maximum_timestamp_creation_to_response_receive_ms
    );

    for (index, record) in cases.iter().enumerate() {
        fixture.assert_verification_rejected(&format!("l1-protocol-a-{index}.json"), record);
    }
}

#[test]
fn every_dispatch_response_tuple_source_and_review_envelope_rule_rejects_drift() {
    let fixture = L1DerivationFixture::new();
    let baseline = fixture.record();
    let mut cases = Vec::new();

    macro_rules! drift_string {
        ($($field:ident).+) => {{
            let mut record = baseline.clone();
            record.$($field).+.push_str("-drift");
            cases.push(record);
        }};
    }
    macro_rules! drift_number {
        ($($field:ident).+) => {{
            let mut record = baseline.clone();
            record.$($field).+ = record.$($field).+.saturating_add(1);
            cases.push(record);
        }};
    }

    drift_number!(protocol.dispatch.maximum_request_count);
    drift_number!(protocol.dispatch.connect_timeout_ms);
    drift_number!(protocol.dispatch.request_timeout_ms);
    let mut redirects = baseline.clone();
    redirects.protocol.dispatch.redirects_allowed = true;
    cases.push(redirects);
    let mut retries = baseline.clone();
    retries.protocol.dispatch.retries_allowed = true;
    cases.push(retries);
    let mut proxy = baseline.clone();
    proxy.protocol.dispatch.forward_proxy_allowed = true;
    cases.push(proxy);
    let mut fallback = baseline.clone();
    fallback.protocol.dispatch.destination_fallback_allowed = true;
    cases.push(fallback);
    let mut peer_order = baseline.clone();
    peer_order
        .protocol
        .dispatch
        .connected_peer_check_before_status_and_body_required = false;
    cases.push(peer_order);

    drift_number!(protocol.response.required_status_code);
    let mut content_type_required = baseline.clone();
    content_type_required
        .protocol
        .response
        .content_type_header_required = false;
    cases.push(content_type_required);
    drift_string!(protocol.response.required_content_type_essence);
    drift_string!(protocol.response.allowed_content_type_charset);
    drift_string!(protocol.response.allowed_content_encoding);
    drift_number!(protocol.response.maximum_body_bytes);
    drift_number!(protocol.response.required_json_object_field_count);
    drift_string!(protocol.response.api_key_field_name);
    drift_string!(protocol.response.api_key_field_type);
    drift_string!(protocol.response.secret_field_name);
    drift_string!(protocol.response.secret_field_type);
    drift_string!(protocol.response.passphrase_field_name);
    drift_string!(protocol.response.passphrase_field_type);
    drift_string!(protocol.response.tuple_association.api_key_association);
    drift_string!(protocol.response.tuple_association.secret_association);
    drift_string!(protocol.response.tuple_association.passphrase_association);

    drift_string!(official_sources.manifest.schema_family);
    drift_number!(official_sources.manifest.schema_version);
    drift_string!(official_sources.manifest.retrieved_at_utc);
    drift_number!(official_sources.manifest.byte_length);
    drift_string!(official_sources.manifest.sha256);
    drift_string!(official_sources.api_authentication.id);
    drift_string!(official_sources.api_authentication.requested_url);
    drift_string!(official_sources.api_authentication.final_url);
    drift_string!(official_sources.api_authentication.retrieved_at_utc);
    drift_string!(official_sources.api_authentication.content_type);
    drift_number!(official_sources.api_authentication.byte_length);
    drift_string!(official_sources.api_authentication.sha256);

    drift_string!(policy_id);
    drift_string!(reviewer_label);
    drift_string!(reviewed_at_utc);
    drift_string!(not_before_utc);
    drift_string!(expires_at_utc);
    drift_string!(cleanup_not_after_utc);

    for (index, record) in cases.iter().enumerate() {
        fixture.assert_verification_rejected(&format!("l1-protocol-b-{index}.json"), record);
    }
}

#[test]
fn verification_boolean_allowlist_is_exhaustive_and_every_other_fact_is_false() {
    let fixture = L1DerivationFixture::new();
    let canonical = fixture.load_record("l1-bool-policy.json", &fixture.record());
    let verification =
        reap_pm_controlled_trial::verify_reviewed_l1_credential_derivation_proof_policy_v1(
            &fixture.context(),
            &canonical,
        )
        .unwrap();
    let json = serde_json::to_value(&verification).unwrap();
    let object = json.as_object().unwrap();
    let expected_true = BTreeSet::from([
        "exact_v1_config_pin_structurally_valid",
        "exact_online_policy_v2_pin_structurally_valid",
        "exact_online_authorization_v2_pin_structurally_valid",
        "exact_reviewed_production_destination_v1_pin_structurally_valid",
        "exact_fresh_credential_delivery_binding_v1_pin_structurally_valid",
        "exact_reviewed_signer_proxy_account_identity_v1_pin_structurally_valid",
        "exact_reviewed_remote_credential_proof_policy_v1_pin_structurally_valid",
        "exact_reviewed_static_online_authorization_v3_pin_structurally_valid",
        "exact_reviewed_phase_a_eligibility_envelope_v4_pin_structurally_valid",
        "exact_reviewed_local_operator_cooperative_custody_profile_v1_pin_structurally_valid",
        "exact_official_manifest_pin_structurally_valid",
        "exact_api_authentication_source_entry_pin_structurally_valid",
        "closed_endpoint_and_route_correlation_grammar_structurally_valid",
        "closed_request_grammar_structurally_valid",
        "closed_eip712_timestamp_and_nonce_grammar_structurally_valid",
        "source_owned_time_requirement_structurally_valid",
        "monotonic_freshness_envelope_structurally_valid",
        "one_request_transport_grammar_structurally_valid",
        "strict_response_grammar_structurally_valid",
        "same_loaded_l2_holder_tuple_association_requirement_structurally_valid",
        "review_time_envelope_nested_within_online_authorization_v2",
    ]);
    let actual_true = object
        .iter()
        .filter_map(|(name, value)| (value.as_bool() == Some(true)).then_some(name.as_str()))
        .collect::<BTreeSet<_>>();
    assert_eq!(actual_true, expected_true);
    for (name, value) in object {
        if let Some(value) = value.as_bool() {
            assert_eq!(
                value,
                expected_true.contains(name.as_str()),
                "unexpected truth value for {name}"
            );
        }
    }
    for allowance in [
        "credential_derivation_dispatch_allowance",
        "cancel_dispatch_allowance",
        "place_dispatch_allowance",
    ] {
        assert_eq!(object[allowance], 0, "nonzero allowance {allowance}");
    }
    assert_eq!(object["production_order_entry_authorized"], false);
    assert_eq!(object["real_order_submission_authorized"], false);
}

#[test]
fn canonical_record_has_only_public_policy_grammar_and_no_evidence_values() {
    let fixture = L1DerivationFixture::new();
    let record = fixture.record();
    let text = serde_json::to_string(&record).unwrap();

    for required in [
        "GET",
        "/auth/derive-api-key",
        "POLY_ADDRESS",
        "POLY_SIGNATURE",
        "POLY_TIMESTAMP",
        "POLY_NONCE",
        "ClobAuthDomain",
        "ClobAuth(address address,string timestamp,uint256 nonce,string message)",
        "This message attests that I control the given wallet",
        "apiKey",
        "secret",
        "passphrase",
        "/time",
        "not_established_by_frozen_official_source_pins_v1",
    ] {
        assert!(text.contains(required), "missing closed grammar {required}");
    }
    for forbidden_value_marker in [
        "api-key-value",
        "l2-secret-value",
        "passphrase-value",
        "private-key-value",
        "signature-value",
        "response-hash-value",
    ] {
        assert!(!text.contains(forbidden_value_marker));
    }
    let value = serde_json::to_value(&record).unwrap();
    let object = value.as_object().unwrap();
    for forbidden_field in [
        "now",
        "observed_timestamp",
        "signature",
        "header_bytes",
        "body_bytes",
        "api_uuid",
        "api_key",
        "l2_secret",
        "passphrase_value",
        "secret_digest",
        "signature_digest",
        "response_hash",
        "response_length",
        "response_value",
        "actor_generation",
        "runtime_attempt",
    ] {
        assert!(!object.contains_key(forbidden_field));
    }
}

struct L1DerivationFixture {
    base: StaticFixture,
    static_v3: reap_pm_controlled_trial::CanonicalReviewedStaticOnlineAuthorizationV3,
    v4: reap_pm_controlled_trial::CanonicalReviewedPhaseAEligibilityEnvelopeV4,
    local_custody:
        reap_pm_controlled_trial::CanonicalReviewedLocalOperatorCooperativeCustodyProfileV1,
}

impl L1DerivationFixture {
    fn new() -> Self {
        let base = StaticFixture::new();
        let static_v3 = base.load_record("l1-bound-static-v3.json", &base.record());
        let v4_context = phase_a_eligibility_context(&base, &static_v3);
        let v4_record = phase_a_eligibility_record(&v4_context);
        let v4_path = write_json(&base.base.root, "l1-bound-v4.json", &v4_record);
        let v4 = reap_pm_controlled_trial::load_canonical_reviewed_phase_a_eligibility_envelope_v4(
            &v4_path,
        )
        .unwrap();
        let local_context = local_operator_context(&base, &static_v3, &v4);
        let local_record = reap_pm_controlled_trial::draft_non_authorizing_reviewed_local_operator_cooperative_custody_profile_v1(&local_context).unwrap();
        let local_path = write_json(
            &base.base.root,
            "l1-bound-local-custody-v1.json",
            &local_record,
        );
        let local_custody = reap_pm_controlled_trial::load_canonical_reviewed_local_operator_cooperative_custody_profile_v1(&local_path).unwrap();
        Self {
            base,
            static_v3,
            v4,
            local_custody,
        }
    }

    fn context(
        &self,
    ) -> reap_pm_controlled_trial::ReviewedL1CredentialDerivationProofPolicyContextV1<'_> {
        reap_pm_controlled_trial::ReviewedL1CredentialDerivationProofPolicyContextV1 {
            phase_a_eligibility_context: phase_a_eligibility_context(&self.base, &self.static_v3),
            reviewed_phase_a_eligibility_envelope_v4: &self.v4,
            reviewed_local_operator_cooperative_custody_profile_v1: &self.local_custody,
        }
    }

    fn record(&self) -> reap_pm_controlled_trial::ReviewedL1CredentialDerivationProofPolicyV1 {
        reap_pm_controlled_trial::draft_non_authorizing_reviewed_l1_credential_derivation_proof_policy_v1(
            &self.context(),
        )
        .unwrap()
    }

    fn load_record(
        &self,
        name: &str,
        record: &reap_pm_controlled_trial::ReviewedL1CredentialDerivationProofPolicyV1,
    ) -> reap_pm_controlled_trial::CanonicalReviewedL1CredentialDerivationProofPolicyV1 {
        let path = write_json(&self.base.base.root, name, record);
        reap_pm_controlled_trial::load_canonical_reviewed_l1_credential_derivation_proof_policy_v1(
            &path,
        )
        .unwrap()
    }

    fn assert_load_rejected(&self, name: &str, bytes: &[u8]) {
        let path = self.base.base.root.join(name);
        write_0600(&path, bytes);
        assert!(
            reap_pm_controlled_trial::load_canonical_reviewed_l1_credential_derivation_proof_policy_v1(
                &path
            )
            .is_err(),
            "loader accepted {name}"
        );
    }

    fn assert_verification_rejected(
        &self,
        name: &str,
        record: &reap_pm_controlled_trial::ReviewedL1CredentialDerivationProofPolicyV1,
    ) {
        let path = write_json(&self.base.base.root, name, record);
        if let Ok(canonical) = reap_pm_controlled_trial::load_canonical_reviewed_l1_credential_derivation_proof_policy_v1(
            &path,
        ) {
            assert!(
                reap_pm_controlled_trial::verify_reviewed_l1_credential_derivation_proof_policy_v1(
                    &self.context(),
                    &canonical,
                )
                .is_err(),
                "verifier accepted {name}"
            );
        }
    }
}

fn different_sha(value: &str) -> String {
    let replacement = if value.starts_with('0') { '1' } else { '0' };
    format!("{replacement}{}", &value[1..])
}

fn domain_fingerprint(domain: &[u8], bytes: &[u8]) -> String {
    use sha2::Digest as _;

    let mut hasher = sha2::Sha256::new();
    hasher.update(domain);
    hasher.update(bytes);
    format!("{:x}", hasher.finalize())
}
