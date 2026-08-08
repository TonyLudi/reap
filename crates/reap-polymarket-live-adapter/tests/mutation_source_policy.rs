const MANIFEST: &str = include_str!("../Cargo.toml");
const LIB: &str = include_str!("../src/lib.rs");
const MUTATION: &str = include_str!("../src/mutation.rs");
const RETAINED: &str = include_str!("../src/mutation/retained.rs");
const TRANSPORT: &str = include_str!("../src/mutation/transport.rs");
const LOOPBACK_CREDENTIALS: &str = include_str!("../src/loopback_mutation_credentials.rs");

#[test]
fn production_surface_has_no_constructible_mutation_transport() {
    assert!(MANIFEST.contains("default = []"));
    assert!(MANIFEST.contains("loopback-evidence = []"));
    assert!(TRANSPORT.contains("#[cfg(any(test, feature = \"loopback-evidence\"))]"));
    assert!(TRANSPORT.contains("pub fn loopback_evidence("));
    assert!(!TRANSPORT.contains("pub fn production("));
    assert!(!TRANSPORT.contains("pub fn new_origin("));
    assert!(LIB.contains("PRODUCTION_ORDER_ENTRY_AUTHORIZED: bool = false"));
    assert!(!LIB.contains("PRODUCTION_ORDER_ENTRY_AUTHORIZED: bool = true"));
}

#[test]
fn mutation_routes_methods_and_retry_policy_are_closed() {
    assert_eq!(TRANSPORT.matches("url.set_path(\"/order\")").count(), 2);
    assert_eq!(TRANSPORT.matches(".post(url)").count(), 1);
    assert_eq!(TRANSPORT.matches(".delete(url)").count(), 1);
    assert!(TRANSPORT.contains(".redirect(Policy::none())"));
    assert!(TRANSPORT.contains(".retry(reqwest::retry::never())"));
    assert!(TRANSPORT.contains(".no_proxy()"));
    assert!(TRANSPORT.contains(".connect_timeout(config.connect_timeout)"));
    assert!(TRANSPORT.contains(".timeout(config.request_timeout)"));
    assert!(TRANSPORT.contains("MAX_MUTATION_RESPONSE_BYTES"));
    assert_eq!(
        TRANSPORT
            .matches("self.client.execute(request).await")
            .count(),
        1
    );
    for forbidden in [
        "pub fn request(",
        "pub fn client(",
        "pub fn headers(",
        "pub fn body(",
        "pub fn route(",
        "pub fn method(",
        "pub fn socket(",
        "cancel_all",
        "cancel_market",
        "batch_order",
    ] {
        assert!(
            !TRANSPORT.contains(forbidden),
            "capability escape: {forbidden}"
        );
    }
}

#[test]
fn authenticated_retention_is_fixed_purpose_move_only_and_zeroizing() {
    for required in [
        "impl FixedPlaceRequestSink for PlaceRetentionSink",
        "impl FixedOwnedCancelRequestSink for CancelRetentionSink",
        "runtime_exact_body_commitment: RuntimeExactBodyCommitment",
        "semantic_request_commitment: PlaceSemanticRequestCommitment",
        "semantic_request_commitment: OwnedCancelSemanticRequestCommitment",
        "pub const fn runtime_exact_body_commitment(&self) -> RuntimeExactBodyCommitment",
        "pub const fn semantic_request_commitment(&self) -> PlaceSemanticRequestCommitment",
        "pub const fn semantic_request_commitment(&self) -> OwnedCancelSemanticRequestCommitment",
        "body: Zeroizing<Vec<u8>>",
        "address: Zeroizing<String>",
        "signature: Zeroizing<String>",
        "api_key: Zeroizing<String>",
        "passphrase: Zeroizing<String>",
        "(0x21..=0x7e).contains(&byte)",
        "pub const fn l2_timestamp_seconds(&self) -> u64",
        "formatter.write_str(\"RetainedL2Headers([REDACTED])\")",
    ] {
        assert!(
            RETAINED.contains(required),
            "missing retention boundary: {required}"
        );
    }
    for forbidden in [
        "derive(Clone",
        "impl Clone for PmRetained",
        "pub fn exact_body(",
        "pub fn api_key(",
        "pub fn passphrase(",
        "pub fn signature(",
        "pub const fn commitment(&self)",
        "commitment: RequestCommitment",
        "derive(Serialize",
        "serde::",
        "Sha256",
        "L2Credentials",
    ] {
        assert!(
            !RETAINED.contains(forbidden),
            "retention escape: {forbidden}"
        );
    }
}

#[test]
fn mutation_outcomes_keep_exact_body_correlation_runtime_only() {
    for required in [
        "runtime_exact_body_commitment: RuntimeExactBodyCommitment",
        "semantic_request_commitment: PlaceSemanticRequestCommitment",
        "semantic_request_commitment: OwnedCancelSemanticRequestCommitment",
        "pub const fn runtime_exact_body_commitment(&self) -> RuntimeExactBodyCommitment",
        "pub const fn semantic_request_commitment(&self) -> PlaceSemanticRequestCommitment",
        "pub const fn semantic_request_commitment(&self) -> OwnedCancelSemanticRequestCommitment",
        "\"runtime_exact_body_commitment\",",
        "[REDACTED; NON_DURABLE]",
    ] {
        assert!(
            TRANSPORT.contains(required),
            "missing dual transport commitment boundary: {required}"
        );
    }
    for forbidden in [
        "pub const fn commitment(&self)",
        "commitment: RequestCommitment",
        "derive(Serialize",
        "serde::",
        "Sha256",
        "runtime_only_bytes(",
    ] {
        assert!(
            !TRANSPORT.contains(forbidden),
            "transport durability/correlation escape: {forbidden}"
        );
    }
}

#[test]
fn source_authority_and_module_boundary_are_explicit() {
    assert!(MUTATION.contains("mod retained;"));
    assert!(MUTATION.contains("mod transport;"));
    assert!(LIB.contains("mod mutation;"));
    for authority in [
        "https://docs.polymarket.com/trading/place-orders",
        "https://docs.polymarket.com/trading/manage-orders",
        "8222273a9c72033b760e1d2fec813bc77144556d",
    ] {
        assert!(
            TRANSPORT.contains(authority),
            "missing protocol authority: {authority}"
        );
    }
}

#[test]
fn loopback_role_bundle_carries_one_typed_configuration_pairing_proof() {
    for required in [
        "observation_grant: PmPublicObservationGrant",
        "observation_grant.instrument() != instrument",
        "observation_grant.polymarket_instrument() != trading_domain.instrument()",
        "observation_grant.configuration_fingerprint()",
        "pub struct PmLoopbackMutationConnectivityBinding",
        "binding: PmLoopbackMutationConnectivityBinding",
        "account: self.binding.account",
        "instrument: self.binding.instrument",
        "trading_domain: self.binding.trading_domain",
        "wire_scope: self.http_config.exact_order_scope()",
        "configuration_fingerprint: PmConfigurationFingerprint",
        "credential_slot_fingerprint: AuthenticatedJournalCredentialSlotFingerprint",
        "self.binding,",
    ] {
        assert!(
            LOOPBACK_CREDENTIALS.contains(required),
            "missing loopback pairing proof: {required}"
        );
    }
    assert!(!LOOPBACK_CREDENTIALS.contains("pub fn configuration_fingerprint("));
    assert!(!LOOPBACK_CREDENTIALS.contains(
        "configuration_fingerprint: PmConfigurationFingerprint,\n        credential_slot"
    ));
    assert!(!LOOPBACK_CREDENTIALS.contains("[u8; 32],\n        AuthenticatedJournal"));
}
