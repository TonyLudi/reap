const MANIFEST: &str = include_str!("../Cargo.toml");
const LIB: &str = include_str!("../src/lib.rs");
const CONFIG: &str = include_str!("../src/config.rs");
const CONSUMPTION: &str = include_str!("../src/consumption.rs");
const CUSTODY: &str = include_str!("../src/custody.rs");
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
    assert!(PROTECTED.contains("fn validate_exact_bytes("));
    assert!(!CONSUMPTION.contains("DispatchAuthorized"));
    assert!(!CONSUMPTION.contains("SignedClobV2Order"));
    assert!(!CONSUMPTION.contains("AuthenticatedPlaceRequest"));
}
