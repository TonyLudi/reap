const MANIFEST: &str = include_str!("../Cargo.toml");
const LIB: &str = include_str!("../src/lib.rs");
const CONFIG: &str = include_str!("../src/config.rs");
const CONSUMPTION: &str = include_str!("../src/consumption.rs");
const CUSTODY: &str = include_str!("../src/custody.rs");
const ONLINE_CONSUMPTION_V2: &str = include_str!("../src/online_consumption_v2.rs");
const ONLINE_POLICY_V2: &str = include_str!("../src/online_policy_v2.rs");
const PREFLIGHT: &str = include_str!("../src/preflight.rs");
const PROTECTED: &str = include_str!("../src/protected_file.rs");
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
