use std::collections::{BTreeMap, BTreeSet, HashSet};
use std::path::{Path, PathBuf};

use anyhow::{Context, Result, bail};
use reap_live::{EconomicReconciliationTolerances, FillStatementTolerances};

use super::{
    MAX_BILL_COLLECTION_AGE_MS, MAX_DEADMAN_CERTIFICATION_AGE_MS, MAX_DEMO_SOAK_AGE_MS,
    MAX_EMERGENCY_CANCEL_AGE_MS, MAX_FAULT_RUN_AGE_MS, MAX_FILL_COLLECTION_AGE_MS,
    MAX_FUTURE_TOLERANCE_MS, MAX_LATENCY_SOURCE_AGE_MS, MAX_PRODUCTION_ACCOUNT_BOUNDARY_GAP_MS,
    MAX_PRODUCTION_ACCOUNT_CERTIFICATION_AGE_MS, MAX_PRODUCTION_ECONOMIC_BALANCE_TOLERANCE,
    MAX_PRODUCTION_ECONOMIC_FEE_TOLERANCE, MAX_PRODUCTION_ECONOMIC_FUNDING_ABSOLUTE_TOLERANCE,
    MAX_PRODUCTION_ECONOMIC_FUNDING_MARK_ABSOLUTE_TOLERANCE,
    MAX_PRODUCTION_ECONOMIC_FUNDING_MARK_RELATIVE_TOLERANCE,
    MAX_PRODUCTION_ECONOMIC_FUNDING_RELATIVE_TOLERANCE, MAX_PRODUCTION_ECONOMIC_QUANTITY_TOLERANCE,
    MAX_PRODUCTION_ECONOMIC_TRADE_PNL_ABSOLUTE_TOLERANCE,
    MAX_PRODUCTION_ECONOMIC_TRADE_PNL_RELATIVE_TOLERANCE, MAX_PRODUCTION_EVIDENCE_ACCOUNTS,
    MAX_PRODUCTION_EVIDENCE_CANDIDATE_ID_BYTES, MAX_PRODUCTION_EVIDENCE_LATENCY_REPORTS,
    MAX_PRODUCTION_EVIDENCE_MANIFEST_BYTES, MAX_PRODUCTION_FUNDING_MARK_BRACKET_DISTANCE_MS,
    PRODUCTION_EVIDENCE_MANIFEST_SCHEMA_VERSION, ProductionEvidenceFaultProxyRunInput,
    ProductionEvidenceFileEvidence, ProductionEvidenceFreshnessPolicy, ProductionEvidenceManifest,
    ResolvedDeadmanInput, ResolvedEconomicInput, ResolvedFaultProxyRunInput, ResolvedFillInput,
    sha256_bytes,
};

pub(super) struct LoadedManifest {
    pub(super) evidence: ProductionEvidenceFileEvidence,
    pub(super) value: ProductionEvidenceManifest,
    pub(super) base: PathBuf,
}

pub(super) struct ResolvedManifest {
    pub(super) demo_config: PathBuf,
    pub(super) production_config: PathBuf,
    pub(super) fault_demo_config: PathBuf,
    pub(super) fault_proxy_config: PathBuf,
    pub(super) demo_soak_report: PathBuf,
    pub(super) fault_matrix_manifest: PathBuf,
    pub(super) fault_proxy_runs: Vec<ResolvedFaultProxyRunInput>,
    pub(super) latency_calibration_artifact: PathBuf,
    pub(super) latency_source_reports: Vec<PathBuf>,
    pub(super) research_manifest: PathBuf,
    pub(super) research_report: PathBuf,
    pub(super) account_certifications: Vec<PathBuf>,
    pub(super) deadman_certifications: Vec<ResolvedDeadmanInput>,
    pub(super) emergency_cancel_report: PathBuf,
    pub(super) fill_reconciliations: Vec<ResolvedFillInput>,
    pub(super) economic_reconciliations: Vec<ResolvedEconomicInput>,
}

pub(super) fn validate_manifest(manifest: &ProductionEvidenceManifest) -> Result<()> {
    if manifest.schema_version != PRODUCTION_EVIDENCE_MANIFEST_SCHEMA_VERSION {
        bail!(
            "production evidence manifest schema must be {}, got {}",
            PRODUCTION_EVIDENCE_MANIFEST_SCHEMA_VERSION,
            manifest.schema_version
        );
    }
    if manifest.expected_reap_version.is_empty()
        || manifest.expected_reap_version.trim() != manifest.expected_reap_version
    {
        bail!("expected_reap_version must be non-empty without surrounding whitespace");
    }
    for (field, value) in [
        (
            "expected_live_executable_sha256",
            manifest.expected_live_executable_sha256.as_str(),
        ),
        (
            "expected_host_identity_sha256",
            manifest.expected_host_identity_sha256.as_str(),
        ),
        (
            "expected_approval_policy_sha256",
            manifest.expected_approval_policy_sha256.as_str(),
        ),
    ] {
        if !is_lower_sha256(value) {
            bail!("{field} must be a lower-case SHA-256");
        }
    }
    if manifest.expected_deployment_candidate_id.is_empty()
        || manifest.expected_deployment_candidate_id.trim()
            != manifest.expected_deployment_candidate_id
        || manifest.expected_deployment_candidate_id.len()
            > MAX_PRODUCTION_EVIDENCE_CANDIDATE_ID_BYTES
    {
        bail!("expected_deployment_candidate_id is invalid");
    }
    validate_expected_account_map(
        "expected_demo_account_identity_sha256s",
        &manifest.expected_demo_account_identity_sha256s,
    )?;
    validate_expected_account_map(
        "expected_production_account_identity_sha256s",
        &manifest.expected_production_account_identity_sha256s,
    )?;
    validate_freshness_policy(&manifest.freshness)?;
    validate_fault_proxy_run_inputs(&manifest.fault_proxy_runs)?;
    validate_count(
        "latency_source_reports",
        manifest.latency_source_reports.len(),
        MAX_PRODUCTION_EVIDENCE_LATENCY_REPORTS,
    )?;
    validate_count(
        "account_certifications",
        manifest.account_certifications.len(),
        MAX_PRODUCTION_EVIDENCE_ACCOUNTS,
    )?;
    validate_count(
        "deadman_certifications",
        manifest.deadman_certifications.len(),
        MAX_PRODUCTION_EVIDENCE_ACCOUNTS,
    )?;
    validate_count(
        "fill_reconciliations",
        manifest.fill_reconciliations.len(),
        MAX_PRODUCTION_EVIDENCE_ACCOUNTS,
    )?;
    for fill in &manifest.fill_reconciliations {
        if fill.minimum_fills == 0 {
            bail!("every fill reconciliation must require at least one fill");
        }
        for (field, value) in [
            ("price_tolerance", fill.price_tolerance),
            ("quantity_tolerance", fill.quantity_tolerance),
            ("fee_tolerance", fill.fee_tolerance),
        ] {
            if !value.is_finite() || value < 0.0 {
                bail!("fill reconciliation {field} must be finite and non-negative");
            }
        }
        if fill.price_tolerance != 0.0
            || fill.quantity_tolerance != 0.0
            || fill.fee_tolerance != 0.0
        {
            bail!("production fill reconciliation requires exact zero tolerances");
        }
    }
    validate_count(
        "economic_reconciliations",
        manifest.economic_reconciliations.len(),
        MAX_PRODUCTION_EVIDENCE_ACCOUNTS,
    )?;
    for economic in &manifest.economic_reconciliations {
        if economic.minimum_trade_bills == 0
            || economic.minimum_derivative_close_bills == 0
            || economic.minimum_funding_bills == 0
        {
            bail!(
                "every economic reconciliation must require trade, derivative-close, and funding evidence"
            );
        }
        if economic.maximum_trade_bill_delay_ms == 0
            || economic.maximum_trade_bill_delay_ms > reap_live::MAX_TRADE_BILL_DELAY_MS
            || economic.maximum_funding_bill_delay_ms == 0
            || economic.maximum_funding_bill_delay_ms > reap_live::MAX_FUNDING_BILL_DELAY_MS
        {
            bail!("economic reconciliation bill delays are outside supported bounds");
        }
        if economic.maximum_funding_mark_bracket_distance_ms == 0
            || economic.maximum_funding_mark_bracket_distance_ms
                > MAX_PRODUCTION_FUNDING_MARK_BRACKET_DISTANCE_MS
        {
            bail!(
                "economic funding mark bracket distance must be in 1..={MAX_PRODUCTION_FUNDING_MARK_BRACKET_DISTANCE_MS} ms"
            );
        }
        if economic.maximum_account_boundary_gap_ms == 0
            || economic.maximum_account_boundary_gap_ms > MAX_PRODUCTION_ACCOUNT_BOUNDARY_GAP_MS
        {
            bail!(
                "economic account boundary gap must be in 1..={MAX_PRODUCTION_ACCOUNT_BOUNDARY_GAP_MS} ms"
            );
        }
        for (field, value) in [
            ("price_tolerance", economic.price_tolerance),
            ("quantity_tolerance", economic.quantity_tolerance),
            ("fee_tolerance", economic.fee_tolerance),
            ("balance_tolerance", economic.balance_tolerance),
            (
                "trade_pnl_absolute_tolerance",
                economic.trade_pnl_absolute_tolerance,
            ),
            (
                "trade_pnl_relative_tolerance",
                economic.trade_pnl_relative_tolerance,
            ),
            (
                "funding_pnl_absolute_tolerance",
                economic.funding_pnl_absolute_tolerance,
            ),
            (
                "funding_pnl_relative_tolerance",
                economic.funding_pnl_relative_tolerance,
            ),
            (
                "funding_mark_absolute_tolerance",
                economic.funding_mark_absolute_tolerance,
            ),
            (
                "funding_mark_relative_tolerance",
                economic.funding_mark_relative_tolerance,
            ),
        ] {
            if !value.is_finite() || value < 0.0 {
                bail!("economic reconciliation {field} must be finite and non-negative");
            }
        }
        if economic.price_tolerance != 0.0 {
            bail!("production economic trade-price tolerance must be zero");
        }
        for (field, value, maximum) in [
            (
                "quantity_tolerance",
                economic.quantity_tolerance,
                MAX_PRODUCTION_ECONOMIC_QUANTITY_TOLERANCE,
            ),
            (
                "fee_tolerance",
                economic.fee_tolerance,
                MAX_PRODUCTION_ECONOMIC_FEE_TOLERANCE,
            ),
            (
                "balance_tolerance",
                economic.balance_tolerance,
                MAX_PRODUCTION_ECONOMIC_BALANCE_TOLERANCE,
            ),
            (
                "trade_pnl_absolute_tolerance",
                economic.trade_pnl_absolute_tolerance,
                MAX_PRODUCTION_ECONOMIC_TRADE_PNL_ABSOLUTE_TOLERANCE,
            ),
            (
                "trade_pnl_relative_tolerance",
                economic.trade_pnl_relative_tolerance,
                MAX_PRODUCTION_ECONOMIC_TRADE_PNL_RELATIVE_TOLERANCE,
            ),
            (
                "funding_pnl_absolute_tolerance",
                economic.funding_pnl_absolute_tolerance,
                MAX_PRODUCTION_ECONOMIC_FUNDING_ABSOLUTE_TOLERANCE,
            ),
            (
                "funding_pnl_relative_tolerance",
                economic.funding_pnl_relative_tolerance,
                MAX_PRODUCTION_ECONOMIC_FUNDING_RELATIVE_TOLERANCE,
            ),
            (
                "funding_mark_absolute_tolerance",
                economic.funding_mark_absolute_tolerance,
                MAX_PRODUCTION_ECONOMIC_FUNDING_MARK_ABSOLUTE_TOLERANCE,
            ),
            (
                "funding_mark_relative_tolerance",
                economic.funding_mark_relative_tolerance,
                MAX_PRODUCTION_ECONOMIC_FUNDING_MARK_RELATIVE_TOLERANCE,
            ),
        ] {
            if value > maximum {
                bail!("production economic {field} must be at most {maximum}, got {value}");
            }
        }
    }
    Ok(())
}

fn validate_fault_proxy_run_inputs(inputs: &[ProductionEvidenceFaultProxyRunInput]) -> Result<()> {
    let expected = reap_live::LiveFaultScenario::REQUIRED
        .into_iter()
        .collect::<BTreeSet<_>>();
    let actual = inputs
        .iter()
        .map(|input| input.scenario)
        .collect::<BTreeSet<_>>();
    if inputs.len() != expected.len() || actual != expected {
        bail!(
            "fault_proxy_runs must cover each required fault scenario exactly once; expected {}, got {} unique across {} entries",
            expected.len(),
            actual.len(),
            inputs.len()
        );
    }
    Ok(())
}

fn validate_freshness_policy(policy: &ProductionEvidenceFreshnessPolicy) -> Result<()> {
    if policy.future_tolerance_ms > MAX_FUTURE_TOLERANCE_MS {
        bail!("freshness.future_tolerance_ms must be at most {MAX_FUTURE_TOLERANCE_MS}");
    }
    for (field, value, maximum) in [
        (
            "demo_soak_max_age_ms",
            policy.demo_soak_max_age_ms,
            MAX_DEMO_SOAK_AGE_MS,
        ),
        (
            "fault_run_max_age_ms",
            policy.fault_run_max_age_ms,
            MAX_FAULT_RUN_AGE_MS,
        ),
        (
            "latency_source_max_age_ms",
            policy.latency_source_max_age_ms,
            MAX_LATENCY_SOURCE_AGE_MS,
        ),
        (
            "production_account_certification_max_age_ms",
            policy.production_account_certification_max_age_ms,
            MAX_PRODUCTION_ACCOUNT_CERTIFICATION_AGE_MS,
        ),
        (
            "deadman_certification_max_age_ms",
            policy.deadman_certification_max_age_ms,
            MAX_DEADMAN_CERTIFICATION_AGE_MS,
        ),
        (
            "emergency_cancel_max_age_ms",
            policy.emergency_cancel_max_age_ms,
            MAX_EMERGENCY_CANCEL_AGE_MS,
        ),
        (
            "fill_collection_max_age_ms",
            policy.fill_collection_max_age_ms,
            MAX_FILL_COLLECTION_AGE_MS,
        ),
        (
            "bill_collection_max_age_ms",
            policy.bill_collection_max_age_ms,
            MAX_BILL_COLLECTION_AGE_MS,
        ),
    ] {
        if value == 0 || value > maximum {
            bail!("freshness.{field} must be within 1..={maximum}, got {value}");
        }
    }
    Ok(())
}

fn validate_expected_account_map(label: &str, values: &BTreeMap<String, String>) -> Result<()> {
    validate_count(label, values.len(), MAX_PRODUCTION_EVIDENCE_ACCOUNTS)?;
    for (account_id, sha256) in values {
        if account_id.is_empty() || account_id.trim() != account_id || account_id.len() > 128 {
            bail!("{label} contains an invalid account id");
        }
        if !is_lower_sha256(sha256) {
            bail!("{label}.{account_id} must be a lower-case SHA-256");
        }
    }
    Ok(())
}

fn validate_count(label: &str, actual: usize, maximum: usize) -> Result<()> {
    if actual == 0 || actual > maximum {
        bail!("{label} must contain 1..={maximum} entries, got {actual}");
    }
    Ok(())
}

pub(super) fn resolve_manifest(loaded: &LoadedManifest) -> Result<ResolvedManifest> {
    let value = &loaded.value;
    let base = &loaded.base;
    let demo_config = resolve_regular_file(base, &value.demo_config, "demo config")?;
    let production_config =
        resolve_regular_file(base, &value.production_config, "production config")?;
    if demo_config == production_config {
        bail!("demo and production configs resolve to the same file");
    }
    let fault_demo_config =
        resolve_regular_file(base, &value.fault_demo_config, "routed fault demo config")?;
    let fault_proxy_config =
        resolve_regular_file(base, &value.fault_proxy_config, "fault-proxy config")?;
    if fault_demo_config == demo_config
        || fault_demo_config == production_config
        || fault_proxy_config == demo_config
        || fault_proxy_config == production_config
        || fault_proxy_config == fault_demo_config
    {
        bail!("demo, production, routed-fault, and fault-proxy configs must be distinct files");
    }
    let demo_soak_report = resolve_regular_file(base, &value.demo_soak_report, "demo soak report")?;
    let fault_matrix_manifest =
        resolve_regular_file(base, &value.fault_matrix_manifest, "fault matrix manifest")?;
    let mut fault_proxy_runs = Vec::with_capacity(value.fault_proxy_runs.len());
    let mut fault_proxy_run_paths = HashSet::new();
    for input in &value.fault_proxy_runs {
        let report = resolve_regular_file(base, &input.report, "fault-proxy run report")?;
        if !fault_proxy_run_paths.insert(report.clone()) {
            bail!("duplicate fault-proxy run report {}", report.display());
        }
        fault_proxy_runs.push(ResolvedFaultProxyRunInput {
            scenario: input.scenario,
            report,
        });
    }
    let latency_calibration_artifact = resolve_regular_file(
        base,
        &value.latency_calibration_artifact,
        "latency calibration artifact",
    )?;
    let latency_source_reports =
        resolve_unique_paths(base, &value.latency_source_reports, "latency source report")?;
    let research_manifest =
        resolve_regular_file(base, &value.research_manifest, "research manifest")?;
    let research_report = resolve_regular_file(base, &value.research_report, "research report")?;
    let account_certifications =
        resolve_unique_paths(base, &value.account_certifications, "account certification")?;
    let mut deadman_certifications = Vec::with_capacity(value.deadman_certifications.len());
    let mut deadman_artifacts = HashSet::new();
    for input in &value.deadman_certifications {
        let artifact = resolve_regular_file(base, &input.artifact, "deadman artifact")?;
        let journal = resolve_regular_file(base, &input.journal, "deadman journal")?;
        if artifact == journal {
            bail!("deadman artifact and journal resolve to the same file");
        }
        if !deadman_artifacts.insert(artifact.clone()) {
            bail!("duplicate deadman artifact {}", artifact.display());
        }
        deadman_certifications.push(ResolvedDeadmanInput { artifact, journal });
    }
    let emergency_cancel_report = resolve_regular_file(
        base,
        &value.emergency_cancel_report,
        "emergency cancel report",
    )?;
    let mut fill_reconciliations = Vec::with_capacity(value.fill_reconciliations.len());
    let mut fill_manifests = HashSet::new();
    for input in &value.fill_reconciliations {
        let collection_manifest =
            resolve_regular_file(base, &input.collection_manifest, "fill collection manifest")?;
        let journal = resolve_regular_file(base, &input.journal, "fill journal")?;
        if collection_manifest == journal {
            bail!("fill collection manifest and journal resolve to the same file");
        }
        if !fill_manifests.insert(collection_manifest.clone()) {
            bail!(
                "duplicate fill collection manifest {}",
                collection_manifest.display()
            );
        }
        fill_reconciliations.push(ResolvedFillInput {
            collection_manifest,
            journal,
            minimum_fills: input.minimum_fills,
            tolerances: FillStatementTolerances {
                price_abs: input.price_tolerance,
                quantity_abs: input.quantity_tolerance,
                fee_abs: input.fee_tolerance,
            },
        });
    }
    let mut economic_reconciliations = Vec::with_capacity(value.economic_reconciliations.len());
    let mut economic_bill_manifests = HashSet::new();
    let mut economic_fill_manifests = HashSet::new();
    let mut economic_account_boundaries = HashSet::new();
    for input in &value.economic_reconciliations {
        let fill_collection_manifest = resolve_regular_file(
            base,
            &input.fill_collection_manifest,
            "economic fill collection manifest",
        )?;
        let bill_collection_manifest = resolve_regular_file(
            base,
            &input.bill_collection_manifest,
            "economic bill collection manifest",
        )?;
        let opening_account_certification = resolve_regular_file(
            base,
            &input.opening_account_certification,
            "opening economic account certification",
        )?;
        let closing_account_certification = resolve_regular_file(
            base,
            &input.closing_account_certification,
            "closing economic account certification",
        )?;
        let journal = resolve_regular_file(base, &input.journal, "economic journal")?;
        let distinct_paths = [
            &fill_collection_manifest,
            &bill_collection_manifest,
            &opening_account_certification,
            &closing_account_certification,
            &journal,
        ]
        .into_iter()
        .collect::<HashSet<_>>();
        if distinct_paths.len() != 5 {
            bail!(
                "economic fill manifest, bill manifest, account boundaries, and journal must be distinct files"
            );
        }
        if !economic_fill_manifests.insert(fill_collection_manifest.clone()) {
            bail!(
                "duplicate economic fill collection manifest {}",
                fill_collection_manifest.display()
            );
        }
        if !economic_bill_manifests.insert(bill_collection_manifest.clone()) {
            bail!(
                "duplicate economic bill collection manifest {}",
                bill_collection_manifest.display()
            );
        }
        for boundary in [
            opening_account_certification.clone(),
            closing_account_certification.clone(),
        ] {
            if !economic_account_boundaries.insert(boundary.clone()) {
                bail!("duplicate economic account boundary {}", boundary.display());
            }
        }
        economic_reconciliations.push(ResolvedEconomicInput {
            fill_collection_manifest,
            bill_collection_manifest,
            opening_account_certification,
            closing_account_certification,
            journal,
            minimum_trade_bills: input.minimum_trade_bills,
            minimum_derivative_close_bills: input.minimum_derivative_close_bills,
            minimum_funding_bills: input.minimum_funding_bills,
            maximum_trade_bill_delay_ms: input.maximum_trade_bill_delay_ms,
            maximum_funding_bill_delay_ms: input.maximum_funding_bill_delay_ms,
            maximum_funding_mark_bracket_distance_ms: input
                .maximum_funding_mark_bracket_distance_ms,
            maximum_account_boundary_gap_ms: input.maximum_account_boundary_gap_ms,
            tolerances: EconomicReconciliationTolerances {
                price_abs: input.price_tolerance,
                quantity_abs: input.quantity_tolerance,
                fee_abs: input.fee_tolerance,
                balance_abs: input.balance_tolerance,
                trade_pnl_abs: input.trade_pnl_absolute_tolerance,
                trade_pnl_relative: input.trade_pnl_relative_tolerance,
                funding_pnl_abs: input.funding_pnl_absolute_tolerance,
                funding_pnl_relative: input.funding_pnl_relative_tolerance,
                funding_mark_abs: input.funding_mark_absolute_tolerance,
                funding_mark_relative: input.funding_mark_relative_tolerance,
            },
        });
    }
    Ok(ResolvedManifest {
        demo_config,
        production_config,
        fault_demo_config,
        fault_proxy_config,
        demo_soak_report,
        fault_matrix_manifest,
        fault_proxy_runs,
        latency_calibration_artifact,
        latency_source_reports,
        research_manifest,
        research_report,
        account_certifications,
        deadman_certifications,
        emergency_cancel_report,
        fill_reconciliations,
        economic_reconciliations,
    })
}

pub(super) fn resolve_unique_paths(
    base: &Path,
    values: &[PathBuf],
    label: &'static str,
) -> Result<Vec<PathBuf>> {
    let mut resolved = Vec::with_capacity(values.len());
    let mut unique = HashSet::new();
    for value in values {
        let path = resolve_regular_file(base, value, label)?;
        if !unique.insert(path.clone()) {
            bail!("duplicate {label} path {}", path.display());
        }
        resolved.push(path);
    }
    Ok(resolved)
}

pub(super) fn resolve_regular_file(
    base: &Path,
    value: &Path,
    label: &'static str,
) -> Result<PathBuf> {
    let path = if value.is_absolute() {
        value.to_path_buf()
    } else {
        base.join(value)
    };
    let metadata = std::fs::symlink_metadata(&path)
        .with_context(|| format!("invalid {label} path {}", path.display()))?;
    if metadata.file_type().is_symlink() || !metadata.is_file() {
        bail!(
            "{label} {} must be a regular file and not a symbolic link",
            path.display()
        );
    }
    std::fs::canonicalize(&path)
        .with_context(|| format!("failed to canonicalize {label} {}", path.display()))
}

pub(super) fn load_manifest(path: &Path) -> Result<LoadedManifest> {
    let metadata = std::fs::symlink_metadata(path)
        .with_context(|| format!("invalid production evidence manifest {}", path.display()))?;
    if metadata.file_type().is_symlink() || !metadata.is_file() {
        bail!(
            "production evidence manifest {} must be a regular file and not a symbolic link",
            path.display()
        );
    }
    if metadata.len() > MAX_PRODUCTION_EVIDENCE_MANIFEST_BYTES {
        bail!(
            "production evidence manifest is {} bytes; limit is {}",
            metadata.len(),
            MAX_PRODUCTION_EVIDENCE_MANIFEST_BYTES
        );
    }
    let source_path = std::fs::canonicalize(path).with_context(|| {
        format!(
            "failed to canonicalize production evidence manifest {}",
            path.display()
        )
    })?;
    let bytes = std::fs::read(&source_path).with_context(|| {
        format!(
            "failed to read production evidence manifest {}",
            source_path.display()
        )
    })?;
    if bytes.len() as u64 > MAX_PRODUCTION_EVIDENCE_MANIFEST_BYTES {
        bail!(
            "production evidence manifest is {} bytes after reading; limit is {}",
            bytes.len(),
            MAX_PRODUCTION_EVIDENCE_MANIFEST_BYTES
        );
    }
    let text = std::str::from_utf8(&bytes).context("production evidence manifest is not UTF-8")?;
    let value: ProductionEvidenceManifest =
        toml::from_str(text).context("failed to parse strict production evidence manifest")?;
    let base = source_path
        .parent()
        .unwrap_or_else(|| Path::new("."))
        .to_path_buf();
    Ok(LoadedManifest {
        evidence: ProductionEvidenceFileEvidence {
            source_path,
            bytes: bytes.len() as u64,
            sha256: sha256_bytes(&bytes),
        },
        value,
        base,
    })
}

fn is_lower_sha256(value: &str) -> bool {
    value.len() == 64
        && value
            .bytes()
            .all(|byte| byte.is_ascii_digit() || (b'a'..=b'f').contains(&byte))
}
