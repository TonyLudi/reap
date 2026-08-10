const MANIFEST: &str = include_str!("../Cargo.toml");
const LIB: &str = include_str!("../src/lib.rs");
const CONFIG: &str = include_str!("../src/config.rs");
const CONSUMPTION: &str = include_str!("../src/consumption.rs");
const CUSTODY: &str = include_str!("../src/custody.rs");
const ONLINE_CONSUMPTION_V2: &str = include_str!("../src/online_consumption_v2.rs");
const ONLINE_POLICY_V2: &str = include_str!("../src/online_policy_v2.rs");
const PREFLIGHT: &str = include_str!("../src/preflight.rs");
const PROTECTED: &str = include_str!("../src/protected_file.rs");
const REVIEWED_DESTINATION_PROFILE_V1: &str =
    include_str!("../src/reviewed_destination_profile_v1.rs");
const REVIEWED_FRESH_CREDENTIAL_SLOT_LOCATOR_V1: &str =
    include_str!("../src/reviewed_fresh_credential_slot_locator_v1.rs");
const FRESH_CREDENTIAL_DELIVERY_BINDING_V1: &str =
    include_str!("../src/fresh_credential_delivery_binding_v1.rs");
const MAIN: &str = include_str!("../src/main.rs");

#[test]
fn crate_has_no_network_live_adapter_or_mutation_dependency() {
    for forbidden in [
        "reqwest",
        "hyper",
        "tokio",
        "reap-pm-live",
        "reap-polymarket-live-adapter",
        "reap-polymarket-adapter",
    ] {
        assert!(
            !MANIFEST.contains(forbidden),
            "forbidden dependency: {forbidden}"
        );
    }
}

#[test]
fn source_exposes_only_offline_dry_run_commands_and_no_mutation_route_or_body() {
    let source = [
        LIB,
        CONFIG,
        CONSUMPTION,
        CUSTODY,
        ONLINE_CONSUMPTION_V2,
        ONLINE_POLICY_V2,
        PREFLIGHT,
        PROTECTED,
        REVIEWED_DESTINATION_PROFILE_V1,
        REVIEWED_FRESH_CREDENTIAL_SLOT_LOCATOR_V1,
        FRESH_CREDENTIAL_DELIVERY_BINDING_V1,
        MAIN,
    ]
    .join("\n");
    for forbidden in [
        "POST /order",
        "DELETE /order",
        "serialize_gtc_post_only",
        "sign_clob_v2_order",
        "AuthenticatedPlaceRequest",
        "FixedPlaceRequestSink",
        "production_order_entry_authorized: true",
        "real_order_submission_authorized: true",
    ] {
        assert!(
            !source.contains(forbidden),
            "forbidden source surface: {forbidden}"
        );
    }
    assert!(MAIN.contains("VerifyPlan"));
    assert!(MAIN.contains("VerifyAuthorization"));
    assert!(MAIN.contains("InspectCustody"));
    assert!(!MAIN.contains("Place"));
    assert!(!MAIN.contains("Cancel"));
    assert!(!MAIN.contains("ConsumeAuthorization"));
}

#[test]
fn fresh_credential_delivery_binding_v1_is_unsigned_exact_move_only_and_denied_only() {
    for required in [
        "FRESH_CREDENTIAL_DELIVERY_BINDING_V1_FINGERPRINT_DOMAIN",
        "reap.pm-t2.controlled-trial.fresh-credential-delivery-binding.v1\\0",
        "pm-t2-fresh-credential-delivery-binding-v1.json",
        "pub struct FreshCredentialSlotLocatorPinsV1",
        "pub locator_id: String,",
        "pub canonical_sha256: String,",
        "pub canonical_length: u64,",
        "pub fingerprint: String,",
        "self.locator_id == pins.locator_id",
        "&& self.canonical_sha256 == pins.canonical_sha256",
        "&& self.canonical_length == pins.canonical_length",
        "&& self.fingerprint == pins.fingerprint",
        "struct FreshCredentialSlotLocatorPinsViewV1<'a>",
        "struct FreshCredentialLinuxTimesViewV1",
        "pub struct UnattestedFreshCredentialProviderGenerationV1",
        "pub provider_id: String,",
        "pub provider_key_id: String,",
        "pub rotation_namespace_id: String,",
        "pub delivery_id: String,",
        "pub rotation_generation: u64,",
        "pub struct FreshCredentialLinuxDirectoryIdentityV1",
        "pub struct FreshCredentialLinuxFileIdentityV1",
        "pub struct FreshCredentialLinuxFileIdentitiesV1",
        "pub private_key: FreshCredentialLinuxFileIdentityV1,",
        "pub api_key: FreshCredentialLinuxFileIdentityV1,",
        "pub l2_secret: FreshCredentialLinuxFileIdentityV1,",
        "pub passphrase: FreshCredentialLinuxFileIdentityV1,",
        "pub filesystem_device: u64,",
        "pub inode: u64,",
        "pub owner_uid: u32,",
        "pub permission_mode: u32,",
        "pub hard_link_count: u64,",
        "pub modified_seconds: i64,",
        "pub modified_nanoseconds: i64,",
        "pub status_changed_seconds: i64,",
        "pub status_changed_nanoseconds: i64,",
        "permission_mode != required_permission_mode",
        "self.hard_link_count != 1",
        "file.filesystem_device != self.directory.filesystem_device",
        "file.owner_uid != self.directory.owner_uid",
        "files[left].inode_key() == files[right].inode_key()",
        "pub unattested_delivery_recorded_at_utc: String,",
        "pub unattested_valid_not_before_utc: String,",
        "pub unattested_valid_not_after_utc: String,",
        "binding_times.recorded_at < authorization_times.reviewed_at",
        "binding_times.valid_not_before < authorization_times.not_before",
        "binding_times.valid_not_after > authorization_times.cleanup_not_after",
        "directory.owner_uid != authorization.value().host.linux_euid",
        "not an active placement-window",
        "current-time, provider-freshness, or unrevoked-lease check",
        "future V3",
        "both locator and delivery records by schema/ID/exact",
        "schema defines no designated secret/value, credential-file-length",
        "credential-content-digest field",
        "must never place secret material or",
        "secret-derived values in any label field",
        "there is no provider signature",
        "including its private path",
        "offers no public path projection",
        "Initial combined construction needs no process-local `Arc` correlation",
        "do not prove same-invocation or same-load",
        "provider-owned descriptor delivery",
        "authenticated descriptor-delivery receipt must commit",
        "this exact binding fingerprint and ultimately the exact V3 Prepared",
        "actor/generation audience",
        "generic provider proof is not this",
        "pub struct CanonicalFreshCredentialDeliveryBindingV1",
        "pub struct CanonicalFreshCredentialDeliveryBindingEvidenceV1",
        "pub struct FreshCredentialDeliveryLoadTokenV1",
        "pub fn bind_fresh_credential_delivery_binding_v1(",
        "locator: CanonicalReviewedFreshCredentialSlotLocatorV1,",
        "binding: CanonicalFreshCredentialDeliveryBindingV1,",
        "bind_reviewed_fresh_credential_slot_locator_v1(config, policy, authorization, locator)",
        "pub fn verify_fresh_credential_delivery_binding_evidence_v1(",
        "pub fn into_parts(",
        "ReviewedFreshCredentialLoadTokenV1,",
        "FreshCredentialLinuxObjectSetV1,",
        "source_owned_current_time_checked: false",
        "protected_credential_directory_and_four_files_checked: false",
        "loaded_linux_objects_match_unattested_binding: false",
        "same_loaded_holder_attested: false",
        "globally_unique_delivery_attested: false",
        "provider_authorship_attested: false",
        "provider_signature_verified: false",
        "provider_lease_fresh_and_unrevoked: false",
        "rotation_generation_attested: false",
        "delivery_freshness_attested: false",
        "loaded_bundle_matches_credential_slot_generation: false",
        "remote_api_key_owner_attested: false",
        "locator_fingerprint_pinned_by_v2: false",
        "delivery_binding_fingerprint_pinned_by_v2: false",
        "delivery_consumption_durably_recorded: false",
        "authorization_consumption_checked: false",
        "credential_mutation_authority_attested: false",
        "OfflineAuthorizationState::DENIED",
        "ProtectedFileKind::FreshCredentialDeliveryBindingV1",
        "CanonicalFreshCredentialDeliveryBindingV1(<unsigned-exact-canonical-bytes; redacted; denied>)",
        "CanonicalFreshCredentialDeliveryBindingEvidenceV1(<retained-exact-evidence; no-path-or-provider-projection; redacted; denied>)",
        "FreshCredentialDeliveryLoadTokenV1(<one-shot-local-load; path-signer-and-metadata-redacted; denied>)",
    ] {
        assert!(
            FRESH_CREDENTIAL_DELIVERY_BINDING_V1.contains(required),
            "missing fresh credential delivery binding V1 source pin `{required}`"
        );
    }

    for forbidden in [
        "reqwest",
        "tokio",
        "TcpStream",
        "connect_async",
        "L2Credentials",
        "EoaPrivateKeyInput",
        "CredentialSlotId",
        "authenticated_journal_credential_slot",
        "read_four(",
        "Arc<",
        "production_order_entry_authorized: true",
        "real_order_submission_authorized: true",
        "source_owned_current_time_checked: true",
        "protected_credential_directory_and_four_files_checked: true",
        "loaded_linux_objects_match_unattested_binding: true",
        "same_loaded_holder_attested: true",
        "globally_unique_delivery_attested: true",
        "provider_authorship_attested: true",
        "provider_signature_verified: true",
        "provider_lease_fresh_and_unrevoked: true",
        "rotation_generation_attested: true",
        "delivery_freshness_attested: true",
        "loaded_bundle_matches_credential_slot_generation: true",
        "remote_api_key_owner_attested: true",
        "locator_fingerprint_pinned_by_v2: true",
        "delivery_binding_fingerprint_pinned_by_v2: true",
        "delivery_consumption_durably_recorded: true",
        "authorization_consumption_checked: true",
        "credential_mutation_authority_attested: true",
        "impl Clone for CanonicalFreshCredentialDeliveryBindingV1",
        "impl Serialize for CanonicalFreshCredentialDeliveryBindingV1",
        "Deserialize<'de> for CanonicalFreshCredentialDeliveryBindingV1",
        "impl Clone for CanonicalFreshCredentialDeliveryBindingEvidenceV1",
        "impl Serialize for CanonicalFreshCredentialDeliveryBindingEvidenceV1",
        "Deserialize<'de> for CanonicalFreshCredentialDeliveryBindingEvidenceV1",
        "impl Clone for FreshCredentialDeliveryLoadTokenV1",
        "impl Serialize for FreshCredentialDeliveryLoadTokenV1",
        "Deserialize<'de> for FreshCredentialDeliveryLoadTokenV1",
        "pub const fn value(&self)",
        "pub fn value(&self)",
        "pub fn path(&self)",
        "pub fn provider_id(&self)",
        "pub fn rotation_generation(&self)",
    ] {
        assert!(
            !FRESH_CREDENTIAL_DELIVERY_BINDING_V1.contains(forbidden),
            "forbidden secret, transport, clone, projection, or authority surface: {forbidden}"
        );
    }

    let file_identity = FRESH_CREDENTIAL_DELIVERY_BINDING_V1
        .split_once("pub struct FreshCredentialLinuxFileIdentityV1 {")
        .and_then(|(_, tail)| tail.split_once("\n}").map(|(body, _)| body))
        .expect("fresh credential file identity declaration must remain recognizable");
    for forbidden_field in ["length", "sha256", "hash", "digest", "content", "value"] {
        assert!(
            !file_identity.contains(forbidden_field),
            "credential file identity gained forbidden field `{forbidden_field}`"
        );
    }

    let binder_signature = FRESH_CREDENTIAL_DELIVERY_BINDING_V1
        .split_once("pub fn bind_fresh_credential_delivery_binding_v1(")
        .and_then(|(_, tail)| {
            tail.split_once(") -> Result<")
                .map(|(signature, _)| signature)
        })
        .expect("fresh credential delivery binder signature must remain recognizable");
    for forbidden_input in [
        "EvidenceV1",
        "LoadTokenV1",
        "signer",
        "provider_signature",
        "Arc",
    ] {
        assert!(
            !binder_signature.contains(forbidden_input),
            "fresh credential delivery binder gained forbidden input `{forbidden_input}`"
        );
    }

    for declaration in [
        "pub struct CanonicalFreshCredentialDeliveryBindingV1 {",
        "pub struct CanonicalFreshCredentialDeliveryBindingEvidenceV1 {",
        "pub struct FreshCredentialDeliveryLoadTokenV1 {",
    ] {
        let before = FRESH_CREDENTIAL_DELIVERY_BINDING_V1
            .split_once(declaration)
            .map(|(before, _)| before)
            .expect("delivery binding move-only declaration must remain recognizable");
        let declaration_attributes = before
            .rsplit_once("\n}\n\n")
            .map_or(before, |(_, attributes)| attributes);
        assert!(
            !declaration_attributes.contains("#[derive"),
            "move-only delivery binding type gained a derive: {declaration}"
        );

        let type_name = declaration
            .strip_prefix("pub struct ")
            .and_then(|value| value.strip_suffix(" {"))
            .expect("delivery binding declaration name must remain recognizable");
        for implementation in FRESH_CREDENTIAL_DELIVERY_BINDING_V1.split("\nimpl") {
            let header = implementation
                .split_once('{')
                .map_or(implementation, |(header, _)| header);
            assert!(
                !(header.contains(type_name) && header.contains("Deserialize")),
                "move-only delivery binding type gained manual Deserialize: {type_name}"
            );
        }
    }

    assert!(PROTECTED.contains("FreshCredentialDeliveryBindingV1"));
    assert!(LIB.contains("mod fresh_credential_delivery_binding_v1;"));
    assert!(!ONLINE_POLICY_V2.contains("FreshCredentialDeliveryBindingV1"));
    assert!(!ONLINE_CONSUMPTION_V2.contains("FreshCredentialDeliveryBindingV1"));
    assert!(
        !REVIEWED_FRESH_CREDENTIAL_SLOT_LOCATOR_V1.contains("FreshCredentialDeliveryBindingV1")
    );
}

#[test]
fn reviewed_fresh_credential_slot_locator_v1_is_exact_additive_and_denied_only() {
    for required in [
        "REVIEWED_FRESH_CREDENTIAL_SLOT_LOCATOR_V1_FINGERPRINT_DOMAIN",
        "reap.pm-t2.controlled-trial.reviewed-fresh-credential-slot-locator.v1\\0",
        "pm-t2-reviewed-fresh-credential-slot-locator-v1.json",
        "pub const PM_T2_FRESH_PRIVATE_KEY_ENTRY_V1: &str = \"private-key\";",
        "pub const PM_T2_FRESH_API_KEY_ENTRY_V1: &str = \"api-key\";",
        "pub const PM_T2_FRESH_L2_SECRET_ENTRY_V1: &str = \"l2-secret\";",
        "pub const PM_T2_FRESH_PASSPHRASE_ENTRY_V1: &str = \"passphrase\";",
        "pub protected_fresh_credential_directory: String,",
        "pub credential_slot_id: String,",
        "pub credential_slot_nonsecret_fingerprint_sha256: String,",
        "validate_online_authorization_contract_v2",
        "locator_times.valid_not_before != authorization_times.not_before",
        "locator_times.valid_not_after != authorization_times.cleanup_not_after",
        "locator.credential_slot_id != config.value().credential_slot.slot_id",
        "nonsecret_fingerprint_sha256",
        "validate_absolute_lexical_directory",
        "normalized.as_os_str() != OsStr::new(value)",
        "without consulting a caller clock",
        "source_owned_current_time_checked: false",
        "protected_credential_directory_and_four_files_checked: false",
        "loaded_bundle_matches_credential_slot_generation: false",
        "remote_api_key_owner_attested: false",
        "locator_fingerprint_pinned_by_v2: false",
        "reviewer_authorship_attested: false",
        "load_token_consumption_durably_recorded: false",
        "authorization_consumption_checked: false",
        "OfflineAuthorizationState::DENIED",
        "does not derive a fingerprint from an API key",
        "protected sidecar and absolute directory remain caller-supplied",
        "unattested by the frozen V2 lineage",
        "they do not attest reviewer",
        "authorship, and `issuing_reviewer` is only a reviewer label",
        "caller cannot select a",
        "non-fixed basename",
        "Reviewer-labeled, non-authenticated",
        "filesystem object currently exists at the locator",
        "future positive gate needs",
        "frozen online-authorization V2 consumption",
        "pub struct CanonicalReviewedFreshCredentialSlotLocatorEvidenceV1",
        "pub struct ReviewedFreshCredentialLoadTokenV1",
        "pub fn bind_reviewed_fresh_credential_slot_locator_v1(",
        "pub fn verify_reviewed_fresh_credential_slot_locator_evidence_v1(",
        "let configured_signer = config.value().account.signer.clone();",
        "pub fn into_parts(self) -> (PathBuf, String)",
        "One-shot local projection capability for one loaded canonical holder",
        "returns ordinary cloneable `PathBuf`",
        "does not prevent their later copying or arbitrary",
        "Actual single-load composition is enforced only when",
        "runner's private staged loader immediately consumes this token",
        "Loading the protected sidecar again can",
        "issue another independently denied",
        "ProtectedFileKind::ReviewedFreshCredentialSlotLocatorV1",
        "CanonicalReviewedFreshCredentialSlotLocatorV1(<reviewed-locator-evidence; exact-canonical-bytes; redacted>)",
    ] {
        assert!(
            REVIEWED_FRESH_CREDENTIAL_SLOT_LOCATOR_V1.contains(required),
            "missing reviewed fresh credential-slot locator V1 source pin `{required}`"
        );
    }
    for forbidden in [
        "reqwest",
        "tokio",
        "TcpStream",
        "connect_async",
        "L2Credentials",
        "EoaPrivateKeyInput",
        "CredentialSlotId",
        "authenticated_journal_credential_slot",
        ".canonicalize(",
        "symlink_metadata",
        "production_order_entry_authorized: true",
        "real_order_submission_authorized: true",
        "remote_api_key_owner_attested: true",
        "loaded_bundle_matches_credential_slot_generation: true",
        "impl Clone for CanonicalReviewedFreshCredentialSlotLocatorV1",
        "impl Serialize for CanonicalReviewedFreshCredentialSlotLocatorV1",
        "impl Clone for CanonicalReviewedFreshCredentialSlotLocatorEvidenceV1",
        "impl Serialize for CanonicalReviewedFreshCredentialSlotLocatorEvidenceV1",
        "Deserialize<'de> for CanonicalReviewedFreshCredentialSlotLocatorEvidenceV1",
        "impl Clone for ReviewedFreshCredentialLoadTokenV1",
        "impl Serialize for ReviewedFreshCredentialLoadTokenV1",
        "Deserialize<'de> for ReviewedFreshCredentialLoadTokenV1",
        "Deserialize<'de> for CanonicalReviewedFreshCredentialSlotLocatorV1",
        "pub const fn value(&self)",
        "pub fn protected_fresh_credential_directory(&self)",
    ] {
        assert!(
            !REVIEWED_FRESH_CREDENTIAL_SLOT_LOCATOR_V1.contains(forbidden),
            "forbidden secret, filesystem, transport, or authority surface: {forbidden}"
        );
    }
    assert!(PROTECTED.contains("ReviewedFreshCredentialSlotLocatorV1"));
    assert!(LIB.contains("mod reviewed_fresh_credential_slot_locator_v1;"));
    assert!(!ONLINE_POLICY_V2.contains("ReviewedFreshCredentialSlotLocatorV1"));
    assert!(!ONLINE_CONSUMPTION_V2.contains("ReviewedFreshCredentialSlotLocatorV1"));

    let binder_signature = REVIEWED_FRESH_CREDENTIAL_SLOT_LOCATOR_V1
        .split_once("pub fn bind_reviewed_fresh_credential_slot_locator_v1(")
        .and_then(|(_, tail)| {
            tail.split_once(") -> Result<")
                .map(|(signature, _)| signature)
        })
        .expect("reviewed locator binder signature must remain recognizable");
    assert!(!binder_signature.contains("signer"));

    let canonical_surface = REVIEWED_FRESH_CREDENTIAL_SLOT_LOCATOR_V1
        .split_once("impl CanonicalReviewedFreshCredentialSlotLocatorV1 {")
        .and_then(|(_, tail)| {
            tail.split_once("impl fmt::Debug for CanonicalReviewedFreshCredentialSlotLocatorV1")
                .map(|(surface, _)| surface)
        })
        .expect("canonical reviewed locator surface must remain recognizable");
    assert!(!canonical_surface.contains("value("));
    assert!(!canonical_surface.contains("directory("));

    let evidence_surface = REVIEWED_FRESH_CREDENTIAL_SLOT_LOCATOR_V1
        .split_once("impl CanonicalReviewedFreshCredentialSlotLocatorEvidenceV1 {")
        .and_then(|(_, tail)| {
            tail.split_once(
                "impl fmt::Debug for CanonicalReviewedFreshCredentialSlotLocatorEvidenceV1",
            )
            .map(|(surface, _)| surface)
        })
        .expect("retained reviewed locator evidence surface must remain recognizable");
    assert!(!evidence_surface.contains("value("));
    assert!(!evidence_surface.contains("directory("));

    for declaration in [
        "pub struct CanonicalReviewedFreshCredentialSlotLocatorV1 {",
        "pub struct CanonicalReviewedFreshCredentialSlotLocatorEvidenceV1 {",
        "pub struct ReviewedFreshCredentialLoadTokenV1 {",
    ] {
        let before = REVIEWED_FRESH_CREDENTIAL_SLOT_LOCATOR_V1
            .split_once(declaration)
            .map(|(before, _)| before)
            .expect("reviewed locator declaration must remain recognizable");
        let declaration_attributes = before
            .rsplit_once("\n}\n\n")
            .map_or(before, |(_, attributes)| attributes);
        assert!(
            !declaration_attributes.contains("#[derive"),
            "move-only reviewed locator type gained a derive: {declaration}"
        );

        let type_name = declaration
            .strip_prefix("pub struct ")
            .and_then(|value| value.strip_suffix(" {"))
            .expect("reviewed locator declaration name must remain recognizable");
        for implementation in REVIEWED_FRESH_CREDENTIAL_SLOT_LOCATOR_V1.split("\nimpl") {
            let header = implementation
                .split_once('{')
                .map_or(implementation, |(header, _)| header);
            assert!(
                !(header.contains(type_name) && header.contains("Deserialize")),
                "move-only reviewed locator type gained manual Deserialize: {type_name}"
            );
        }
    }
}

#[test]
fn reviewed_destination_v1_is_exact_additive_denied_peer_evidence_only() {
    for required in [
        "REVIEWED_PRODUCTION_DESTINATION_PROFILE_V1_FINGERPRINT_DOMAIN",
        "reap.pm-t2.controlled-trial.reviewed-production-destinations.v1\\0",
        "pm-t2-reviewed-production-destinations-v1.json",
        "MAX_REVIEWED_DNS_ANSWER_AGE_SECONDS_V1: i64 = 300",
        "ReviewerCapturedFixedAnswers",
        "pub peer_ip: String,",
        "pub geoblock_https: ReviewedFixedTlsDestinationV1,",
        "pub clob_https: ReviewedFixedTlsDestinationV1,",
        "pub status_https: ReviewedFixedTlsDestinationV1,",
        "pub data_api_https: ReviewedFixedTlsDestinationV1,",
        "pub polygon_rpc_https: ReviewedFixedTlsDestinationV1,",
        "pub clob_websocket_wss: ReviewedFixedWebSocketDestinationV1,",
        "polymarket.com",
        "clob.polymarket.com",
        "status.polymarket.com",
        "data-api.polymarket.com",
        "polygon.drpc.org",
        "ws-subscriptions-clob.polymarket.com",
        "/ws/market",
        "/ws/user",
        "dns_name != expected_host",
        "tls_server_name != expected_host",
        "http_host != expected_host",
        "tcp_port != REVIEWED_TLS_PORT_V1",
        "validate_online_authorization_contract_v2",
        "profile_times.valid_not_before != authorization_times.not_before",
        "profile_times.valid_not_after != authorization_times.cleanup_not_after",
        ".checked_sub(profile_times.dns_resolved_at.timestamp())",
        "reviewed DNS answers are older than 300 seconds at authorization start",
        "same_address_family(local_source_ip, peer_ip)",
        "is_public_global_unicast_v4",
        "is_public_global_unicast_v6",
        "live_dns_observation_checked: false",
        "destination_nat_equivalence_checked: false",
        "authorization_consumption_checked: false",
        "OfflineAuthorizationState::DENIED",
        "Loading or verifying it performs no DNS lookup and proves no",
        "live DNS answer, DNSSEC result, TTL",
        "must not fall back to DNS or another",
        "profile makes no destination-independent NAT or common-public-IP",
        "validity window neither resets nor extends the V2 status-history quiet",
        "reusable sidecar is not consumed by the frozen V2 ledger",
        "without consulting a caller clock",
        "ProtectedFileKind::ReviewedProductionDestinationProfileV1",
        "CanonicalReviewedProductionDestinationProfileV1(<reviewed-evidence; exact-canonical-bytes>)",
    ] {
        assert!(
            REVIEWED_DESTINATION_PROFILE_V1.contains(required),
            "missing reviewed destination V1 source pin `{required}`"
        );
    }
    for forbidden in [
        "reqwest",
        "tokio",
        "lookup_host",
        "ToSocketAddrs",
        "TcpStream",
        "connect_async",
        "pub peer_ips:",
        "pub destination_independent_nat_assumption:",
        "production_order_entry_authorized: true",
        "real_order_submission_authorized: true",
        "impl Clone for CanonicalReviewedProductionDestinationProfileV1",
        "impl Serialize for CanonicalReviewedProductionDestinationProfileV1",
    ] {
        assert!(
            !REVIEWED_DESTINATION_PROFILE_V1.contains(forbidden),
            "forbidden resolver, transport, NAT, or authority surface: {forbidden}"
        );
    }
    assert_eq!(
        REVIEWED_DESTINATION_PROFILE_V1
            .matches("pub peer_ip: String,")
            .count(),
        2,
        "each destination record shape must expose one scalar peer, never a list",
    );
    assert!(PROTECTED.contains("ReviewedProductionDestinationProfileV1"));
    assert!(LIB.contains("mod reviewed_destination_profile_v1;"));
    assert!(!ONLINE_POLICY_V2.contains("ReviewedProductionDestinationProfileV1"));
    assert!(!ONLINE_CONSUMPTION_V2.contains("ReviewedProductionDestinationProfileV1"));
}

#[test]
fn online_v2_is_a_separate_canonical_denied_evidence_lineage() {
    for required in [
        "ONLINE_POLICY_V2_FINGERPRINT_DOMAIN",
        "ONLINE_AUTHORIZATION_V2_FINGERPRINT_DOMAIN",
        "reap.pm-t2.controlled-trial.online-policy.v2\\0",
        "reap.pm-t2.controlled-trial.online-authorization.v2\\0",
        "CanonicalOnlinePolicyV2(<reviewed-evidence; exact-canonical-bytes>)",
        "CanonicalOnlineAuthorizationV2(<reviewed-evidence; exact-canonical-bytes>)",
        "OfflineAuthorizationState::DENIED",
        "The separate V2 consumption ledger consumes and",
        "fingerprints these exact V2 authorization bytes directly",
        "A later V2 A3",
        "falling back to an arbitrary V1",
        "authorization is forbidden",
        "Perform an offline reviewer/CLI structural display check",
        "`now` is caller supplied",
        "This check cannot establish live freshness or",
        "source-owned current-runtime witness",
        "None of these records asserts that a global matching-engine",
        "restart, restricted mode, or order-admission mode is absent",
    ] {
        assert!(
            ONLINE_POLICY_V2.contains(required),
            "missing online V2 source pin `{required}`"
        );
    }
    for forbidden in [
        "CanonicalTrialPreflight",
        "TrialPreflightEvidence",
        "TrialPreflightBinding",
        "response_sha256",
        "preflight_fingerprint",
        "CanonicalAuthorization",
        "TrialAuthorization",
        "load_canonical_authorization",
        "verify_authorization",
        "impl Clone for CanonicalOnlinePolicyV2",
        "impl Clone for CanonicalOnlineAuthorizationV2",
        "impl Serialize for CanonicalOnlinePolicyV2",
        "impl Serialize for CanonicalOnlineAuthorizationV2",
        "production_order_entry_authorized: true",
        "real_order_submission_authorized: true",
    ] {
        assert!(
            !ONLINE_POLICY_V2.contains(forbidden),
            "forbidden online V2 fallback or authority surface: {forbidden}"
        );
    }
}

#[test]
fn online_v2_pins_exact_runtime_and_status_source_domains() {
    for required in [
        "value.len() > 64",
        "value.trim() != value",
        "value.chars().any(char::is_control)",
        "byte.is_ascii_lowercase() || byte.is_ascii_digit()",
        "self.host.linux_euid == 0",
        "self.host.linux_euid == u32::MAX",
        "MIN_STATUS_NOTICE_HISTORY_QUIET_INTERVAL_SECONDS_V2",
        "MAX_STATUS_NOTICE_HISTORY_QUIET_INTERVAL_SECONDS_V2",
        ".checked_sub(times.history_window_start.timestamp())",
        "history_reviewed_through != reviewed_at",
        "policy_reviewed_at > times.reviewed_at || times.reviewed_at > times.not_before",
        "!= policy.value.reviewed_status_clob_component",
        "config.value().phase != TrialPhase::APlaceCancel",
        "!policy.value.v1_config.matches(config)",
        "pub egress: ReviewedLinuxEgressProfileV2",
        "pub network_namespace_device: u64",
        "pub network_namespace_inode: u64",
        "pub interface_name: String",
        "pub interface_index: u32",
        "pub local_source_ip: String",
        "pub dedicated_tunnel_or_gateway_profile_reference: String",
        "pub dedicated_tunnel_or_gateway_profile_sha256: String",
        "pub destination_independent_nat_assumption: ReviewedDestinationIndependentNatV2",
        "pub authorized_geoblock_reported_public_ip: String",
        "egress.interface_name.len() > 15",
        "egress.interface_index > 2_147_483_647",
        "parsed.is_unspecified()",
        "parsed.is_loopback()",
        "parsed.is_multicast()",
        "validate_public_egress_ip(",
        "is_public_global_unicast_v4",
        "is_public_global_unicast_v6",
        "(a == 100 && (64..=127).contains(&b))",
        "(a == 198 && b == 51 && c == 100)",
        "(a == 203 && b == 0 && c == 113)",
        "segments[0] & 0xe000 == 0x2000",
        "segments[0] == 0x2001 && segments[1] == 0x0db8",
        "segments[0] == 0x3fff",
        "Tunnel-local `local_source_ip` uses the distinct",
        "restrictive validator above and may remain private",
        "reviewed destination-independent NAT assumption",
        "SameLocalEgressSelection",
    ] {
        assert!(
            ONLINE_POLICY_V2.contains(required),
            "missing exact online V2 domain pin `{required}`"
        );
    }
    assert!(!ONLINE_POLICY_V2.contains("pub authorized_egress_ip: String"));
    assert!(!ONLINE_POLICY_V2.contains(".is_global()"));
}

#[test]
fn online_v2_take_once_consumption_is_separate_denied_evidence_only() {
    for required in [
        "ONLINE_AUTHORIZATION_CONSUMPTION_V2_SCHEMA_VERSION",
        "pm-t2-online-authorization-consumption-v2.jsonl",
        "pm-t2-online-authorization-consumed-v2.claim",
        "pm-t2-phase-a-online-preflight-v2.jsonl",
        "reap.pm-t2.controlled-trial.online-authorization-consumption.binding.v2\\0",
        "reap.pm-t2.controlled-trial.online-authorization-consumption.record.v2\\0",
        "reap.pm-t2.controlled-trial.online-authorization-consumption.claim.v2\\0",
        "PreparedOnlineAuthorizationConsumptionV2",
        "ConsumedOnlineAuthorizationConsumptionV2",
        "create_new(",
        "The create-new, fsynced claim is the take-once linearization point",
        "OnlineAuthorizationPlacementReuseV2::PermanentlyBurned",
        "OnlineAuthorizationCrashRecoveryV2::ExistingV1LifecycleOnlyNoPlacementResume",
        "OfflineAuthorizationState::DENIED",
        "SameLocalEgressSelection",
        "validate_online_authorization_contract_v2",
        ".timestamp_millis()",
        ".checked_add(cleanup_runway_ms)",
        "times.cleanup_not_after.timestamp_millis()",
        "pub fn prepare_online_authorization_consumption_v2(",
        "pub fn verify_online_authorization_consumption_v2(",
        "pub fn consume(",
        "revalidate_held_consumption_evidence",
        "refresh_after_bound_artifact_create",
        "ProtectedFileKind::OnlineAuthorizationConsumptionV2",
        "monotonic crash-durability",
        "trusted local storage",
        "post-crash same-EUID actor",
        "TPM counter, WORM storage, or a",
        "trusted remote registry",
        "atomic V2 claim creation may have created its marker; placement is burned",
        "post_create_claim_errors_are_always_reported_as_burned",
    ] {
        assert!(
            ONLINE_CONSUMPTION_V2.contains(required),
            "missing V2 consumption source pin `{required}`"
        );
    }
    for forbidden in [
        "CanonicalAuthorization,",
        "TrialAuthorization",
        "CanonicalTrialPreflight",
        "TrialPreflightEvidence",
        "response_sha256",
        "preflight_fingerprint",
        "claim_prepared_authorization_consumption",
        "reopen_consumed_authorization_consumption",
        "PmRevalidatedPhaseAPlaceLiveDispatchOwnerV1",
        "SignedClobV2Order",
        "AuthenticatedPlaceRequest",
        "production_order_entry_authorized: true",
        "real_order_submission_authorized: true",
        "impl Clone for PreparedOnlineAuthorizationConsumptionV2",
        "impl Clone for ConsumedOnlineAuthorizationConsumptionV2",
        "impl Serialize for PreparedOnlineAuthorizationConsumptionV2",
        "impl Serialize for ConsumedOnlineAuthorizationConsumptionV2",
    ] {
        assert!(
            !ONLINE_CONSUMPTION_V2.contains(forbidden),
            "forbidden V1 fallback or authority in V2 consumption: {forbidden}"
        );
    }
    assert!(!ONLINE_CONSUMPTION_V2.contains("verify_online_authorization_v2(config"));
    assert!(
        !ONLINE_CONSUMPTION_V2
            .contains("_ => invalid(\"atomic V2 consume claim cannot be created safely\")")
    );
    assert!(PROTECTED.contains("OnlineAuthorizationConsumptionV2"));
    assert!(LIB.contains("mod online_consumption_v2;"));
}

#[test]
fn custody_is_move_only_redacted_and_descriptor_pinned() {
    assert!(CUSTODY.contains(
        "pub struct CustodyInspection {\n    _signer: FixedEoaSigner,\n    _l2: L2Credentials,"
    ));
    assert!(!CUSTODY.contains("impl Clone for CustodyInspection"));
    assert!(!CUSTODY.contains("impl Serialize for CustodyInspection"));
    assert!(CUSTODY.contains("FixedEoaSigner"));
    assert!(CUSTODY.contains("L2Credentials"));
    assert!(PROTECTED.contains("libc::O_NOFOLLOW"));
    assert!(PROTECTED.contains("libc::O_CLOEXEC"));
    assert!(PROTECTED.contains("metadata.nlink() != 1"));
    assert!(PROTECTED.contains("metadata.mode() & 0o7777 != 0o600"));
    assert!(PROTECTED.contains("metadata.mode() & 0o7777 != 0o700"));
    assert!(CUSTODY.contains("Zeroizing"));
}

#[test]
fn schemas_are_closed_canonical_and_domain_separated() {
    assert!(CONFIG.matches("#[serde(deny_unknown_fields)]").count() >= 10);
    assert!(CONFIG.contains("canonical != bytes"));
    assert!(CONFIG.contains("CONFIG_FINGERPRINT_DOMAIN"));
    assert!(CONFIG.contains("PLAN_FINGERPRINT_DOMAIN"));
    assert!(CONFIG.contains("AUTHORIZATION_FINGERPRINT_DOMAIN"));
    assert!(LIB.contains("production_order_entry_authorized: false"));
    assert!(LIB.contains("real_order_submission_authorized: false"));
    assert!(LIB.contains("place_dispatch_allowance: 0"));
    assert!(CONFIG.contains("authorization_consumption_checked: false"));
    assert!(!CONFIG.contains("authorization_consumed: false"));
    assert!(PREFLIGHT.contains("#[serde(deny_unknown_fields)]"));
    assert!(PREFLIGHT.contains("canonical_bytes.is_empty()"));
    assert!(PREFLIGHT.contains("reencoded != canonical_bytes"));
    assert!(PREFLIGHT.contains("PREFLIGHT_FINGERPRINT_DOMAIN"));
}

#[test]
fn authorization_v1_host_schema_is_frozen_and_documents_current_linux_uts_binding() {
    for required in [
        "Exact UTF-8 Linux UTS nodename exposed by the current UTS namespace",
        "Runtime binding is byte-for-byte",
        "performs no DNS lookup",
        "FQDN",
        "expansion, case folding",
        "case folding",
        "trailing-dot",
        "`/etc/hostname`",
        "machine-id",
        "cloud instance alias mapping",
        "`/usr/bin/getent passwd <euid>` lookup",
        "NSS",
        "correctness, integrity, and availability dependencies",
        "not executable-release attestation",
        "validate_reference(&self.host.host_identity, \"host identity is invalid\")?;",
    ] {
        assert!(
            CONFIG.contains(required),
            "missing UTS host pin `{required}`"
        );
    }
    let host_binding = CONFIG
        .split_once("pub struct AuthorizationHostBinding {")
        .and_then(|(_, tail)| tail.split_once("\n}").map(|(body, _)| body))
        .expect("authorization host binding");
    let declared_fields = host_binding
        .lines()
        .map(str::trim)
        .filter(|line| !line.is_empty() && !line.starts_with("///"))
        .collect::<Vec<_>>();
    assert_eq!(
        declared_fields.as_slice(),
        &[
            "pub host_identity: String,",
            "pub boot_identity: String,",
            "pub runtime_user: String,",
            "pub egress_identity: String,",
        ],
        "V1 authorization host binding changed its exact serialized fields",
    );
    for forbidden_v2_field in ["effective_user_id:", "wall_capture", "wall_receive_ns:"] {
        assert!(
            !host_binding.contains(forbidden_v2_field),
            "V2 runtime evidence leaked into V1 authorization bytes: {forbidden_v2_field}",
        );
    }
    assert!(CONFIG.contains("pub const TRIAL_CONFIG_SCHEMA_VERSION: u32 = 1;"));
    assert!(CONFIG.contains("pub const TRIAL_AUTHORIZATION_SCHEMA_VERSION: u32 = 1;"));
}

#[test]
fn preflight_is_move_only_redacted_structural_evidence_and_never_a_permit() {
    assert!(PREFLIGHT.contains("pub struct CanonicalTrialPreflight"));
    assert!(!PREFLIGHT.contains("impl Clone for CanonicalTrialPreflight"));
    assert!(!PREFLIGHT.contains("impl Serialize for CanonicalTrialPreflight"));
    assert!(PREFLIGHT.contains("pub const fn production_order_entry_authorized(&self) -> bool"));
    assert!(PREFLIGHT.contains("pub const fn real_order_submission_authorized(&self) -> bool"));
    assert!(PREFLIGHT.contains("pub const fn place_dispatch_allowance(&self) -> u8"));
    assert!(PREFLIGHT.contains("TrialConfiguredPositionState::Absent"));
    for forbidden_private_numeric in [
        "collateral_balance_base_units",
        "configured_token_balance_base_units",
        "pusd_allowance_base_units",
        "position_size_decimal",
    ] {
        assert!(
            !PREFLIGHT.contains(forbidden_private_numeric),
            "durable preflight exposes private numeric state: {forbidden_private_numeric}"
        );
    }
}

#[test]
fn take_once_consumption_is_fixed_path_atomic_durable_and_never_a_permit() {
    assert!(CONFIG.contains("authorization_consumption_ledger_file"));
    assert!(CONFIG.contains("authorization_consumption_claim_file"));
    assert!(CONSUMPTION.contains("fn bound_paths("));
    assert!(!CONSUMPTION.contains("prepare_authorization_consumption(\n    path:"));
    assert!(CONSUMPTION.contains("create_new("));
    assert!(CONSUMPTION.contains("burned_before_dispatch_authority: true"));
    assert!(CONSUMPTION.contains("placement_can_never_resume: true"));
    assert!(CONSUMPTION.contains("crash_allows_recovery_cancel_only: true"));
    assert!(PROTECTED.contains(".create_new(true)"));
    assert!(PROTECTED.contains(".sync_all()"));
    assert!(CONSUMPTION.contains("claim: DurableCreateNewFile"));
    assert!(CONSUMPTION.contains("revalidate_held_consumption_evidence"));
    assert!(CONSUMPTION.contains("reopen_consumed_authorization_consumption"));
    assert!(CONSUMPTION.contains("records.len() < 2"));
    assert!(CONSUMPTION.contains("AuthorizationConsumptionState::Terminal { .. }"));
    assert!(CONSUMPTION.contains("Consumed recovery custody requires its atomic claim"));
    assert!(CONSUMPTION.contains("owner.revalidate_held_consumption_evidence()?"));
    assert!(
        CONSUMPTION.contains("The fsynced consumption ledger is the non-rollback trust boundary")
    );
    assert!(CONSUMPTION.contains("anchor_recovery_continuation_root"));
    assert!(CONSUMPTION.contains("anchor_recovery_cancel_prepared"));
    assert!(CONSUMPTION.contains("continuation_prepared_record_canonical_json"));
    assert!(CONSUMPTION.contains("anchor_recovery_terminal_plan"));
    assert!(CONSUMPTION.contains("RECOVERY_TERMINAL_PLAN_FINGERPRINT_DOMAIN"));
    assert!(CONSUMPTION.contains("continuation_dispatch_terminal_record_canonical_json"));
    assert!(CONSUMPTION.contains("continuation_intent_terminal_record_canonical_json"));
    assert!(CONSUMPTION.contains("recovery preparation cannot follow its Terminal plan"));
    assert!(CONSUMPTION.contains("recovery_cancel_dispatch_budget: u8"));
    assert!(
        CONSUMPTION.contains("base Terminal is forbidden after recovery-continuation anchoring")
    );
    assert!(PROTECTED.contains("fn validate_exact_bytes("));
    assert!(!CONSUMPTION.contains("DispatchAuthorized"));
    assert!(!CONSUMPTION.contains("SignedClobV2Order"));
    assert!(!CONSUMPTION.contains("AuthenticatedPlaceRequest"));
}
