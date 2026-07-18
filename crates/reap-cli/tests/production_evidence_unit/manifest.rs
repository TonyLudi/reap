use super::super::*;
use super::support::manifest_toml;

#[test]
fn strict_manifest_accepts_complete_shape_and_rejects_unknown_fields() {
    let parsed: ProductionEvidenceManifest = toml::from_str(&manifest_toml("")).unwrap();
    validate_manifest(&parsed).unwrap();

    let template: ProductionEvidenceManifest = toml::from_str(include_str!(
        "../../../../examples/production-evidence.toml"
    ))
    .unwrap();
    validate_manifest(&template).unwrap();

    let error = toml::from_str::<ProductionEvidenceManifest>(&manifest_toml(
        "unknown_release_switch = true",
    ))
    .unwrap_err();
    assert!(error.to_string().contains("unknown field"));
}

#[test]
fn manifest_rejects_weak_reconciliation_and_invalid_identity() {
    let mut parsed: ProductionEvidenceManifest = toml::from_str(&manifest_toml("")).unwrap();
    parsed.fill_reconciliations[0].minimum_fills = 0;
    assert!(validate_manifest(&parsed).is_err());

    parsed.fill_reconciliations[0].minimum_fills = 1;
    parsed.economic_reconciliations[0].minimum_funding_bills = 0;
    assert!(validate_manifest(&parsed).is_err());

    parsed.economic_reconciliations[0].minimum_funding_bills = 1;
    parsed.economic_reconciliations[0].minimum_derivative_close_bills = 0;
    assert!(validate_manifest(&parsed).is_err());

    parsed.economic_reconciliations[0].minimum_derivative_close_bills = 1;
    parsed.economic_reconciliations[0].maximum_funding_bill_delay_ms =
        reap_live::MAX_FUNDING_BILL_DELAY_MS + 1;
    assert!(validate_manifest(&parsed).is_err());

    parsed.economic_reconciliations[0].maximum_funding_bill_delay_ms = 60_000;
    parsed.fill_reconciliations[0].fee_tolerance = f64::EPSILON;
    assert!(validate_manifest(&parsed).is_err());

    parsed.fill_reconciliations[0].fee_tolerance = 0.0;
    parsed.economic_reconciliations[0].price_tolerance = f64::EPSILON;
    assert!(validate_manifest(&parsed).is_err());

    parsed.economic_reconciliations[0].price_tolerance = 0.0;
    parsed.economic_reconciliations[0].balance_tolerance =
        MAX_PRODUCTION_ECONOMIC_BALANCE_TOLERANCE * 2.0;
    assert!(validate_manifest(&parsed).is_err());

    parsed.economic_reconciliations[0].balance_tolerance = 0.0;
    parsed.economic_reconciliations[0].trade_pnl_relative_tolerance =
        MAX_PRODUCTION_ECONOMIC_TRADE_PNL_RELATIVE_TOLERANCE * 2.0;
    assert!(validate_manifest(&parsed).is_err());

    parsed.economic_reconciliations[0].trade_pnl_relative_tolerance = 0.0;
    parsed.economic_reconciliations[0].maximum_funding_mark_bracket_distance_ms =
        MAX_PRODUCTION_FUNDING_MARK_BRACKET_DISTANCE_MS + 1;
    assert!(validate_manifest(&parsed).is_err());

    parsed.economic_reconciliations[0].maximum_funding_mark_bracket_distance_ms = 1_000;
    parsed.economic_reconciliations[0].maximum_account_boundary_gap_ms =
        MAX_PRODUCTION_ACCOUNT_BOUNDARY_GAP_MS + 1;
    assert!(validate_manifest(&parsed).is_err());

    parsed.economic_reconciliations[0].maximum_account_boundary_gap_ms = 60_000;
    parsed.economic_reconciliations[0].funding_mark_relative_tolerance =
        MAX_PRODUCTION_ECONOMIC_FUNDING_MARK_RELATIVE_TOLERANCE * 2.0;
    assert!(validate_manifest(&parsed).is_err());

    parsed.economic_reconciliations[0].funding_mark_relative_tolerance = 0.0;
    parsed.expected_host_identity_sha256 = "ABC".to_string();
    assert!(validate_manifest(&parsed).is_err());

    parsed.expected_host_identity_sha256 = "2".repeat(64);
    parsed.expected_approval_policy_sha256 = "invalid".to_string();
    assert!(validate_manifest(&parsed).is_err());

    parsed.expected_approval_policy_sha256 = "3".repeat(64);
    parsed.freshness.production_account_certification_max_age_ms =
        MAX_PRODUCTION_ACCOUNT_CERTIFICATION_AGE_MS + 1;
    assert!(validate_manifest(&parsed).is_err());

    let mut missing_proxy: ProductionEvidenceManifest = toml::from_str(&manifest_toml("")).unwrap();
    missing_proxy.fault_proxy_runs.pop();
    assert!(validate_manifest(&missing_proxy).is_err());

    let mut legacy: ProductionEvidenceManifest = toml::from_str(&manifest_toml("")).unwrap();
    legacy.schema_version = 3;
    assert!(validate_manifest(&legacy).is_err());
}

#[test]
fn manifest_validation_preserves_fail_first_error_order_and_text() {
    let mut manifest: ProductionEvidenceManifest = toml::from_str(&manifest_toml("")).unwrap();
    manifest.schema_version = 7;
    manifest.freshness.future_tolerance_ms = MAX_FUTURE_TOLERANCE_MS + 1;
    manifest.latency_source_reports.clear();
    manifest.fill_reconciliations[0].fee_tolerance = f64::EPSILON;

    assert_eq!(
        validate_manifest(&manifest).unwrap_err().to_string(),
        "production evidence manifest schema must be 8, got 7"
    );

    manifest.schema_version = PRODUCTION_EVIDENCE_MANIFEST_SCHEMA_VERSION;
    assert_eq!(
        validate_manifest(&manifest).unwrap_err().to_string(),
        "freshness.future_tolerance_ms must be at most 300000"
    );

    manifest.freshness.future_tolerance_ms = 60_000;
    assert_eq!(
        validate_manifest(&manifest).unwrap_err().to_string(),
        "latency_source_reports must contain 1..=128 entries, got 0"
    );

    manifest.latency_source_reports = vec![PathBuf::from("latency-source.json")];
    assert_eq!(
        validate_manifest(&manifest).unwrap_err().to_string(),
        "production fill reconciliation requires exact zero tolerances"
    );
}
