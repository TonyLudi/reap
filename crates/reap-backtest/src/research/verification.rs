use std::collections::{BTreeMap, BTreeSet, HashMap, HashSet};
use std::fs::File;
use std::io::{BufReader, Read};
use std::path::{Path, PathBuf};

use anyhow::{Context, Result, bail};
use reap_capture::{
    CaptureConfig, CaptureVerificationReport, analyze_capture_path, verify_capture_paths,
};
use reap_core::PositionMarginMode;
use reap_feed::replay_check_path;
use reap_live_contracts::{
    ACCOUNT_CERTIFICATION_SCHEMA_VERSION, AccountCertificationArtifact, LiveConfig,
    MAX_ACCOUNT_CERTIFICATION_ARTIFACT_BYTES, OkxTradeModeConfig, TradingEnvironment,
    verify_account_certification_artifact_bytes,
};
use reap_venue::okx::{
    OkxAccountBalanceSnapshot, OkxAccountPositionsSnapshot,
    parse_okx_account_balance_response_json, parse_okx_account_positions_response_json,
};
use sha2::{Digest, Sha256};

use crate::{
    BacktestConfig, BacktestInitialBalanceConfig, BacktestInitialMarginConfig,
    BacktestInitialPortfolioConfig, BacktestInitialPositionConfig, LatencyCalibrationArtifact,
    MAX_LATENCY_CALIBRATION_ARTIFACT_BYTES, RawCaptureRecordRange,
};

use super::configuration::resolve;
use super::{
    LatencyCalibrationProvenance, LoadedCandidate, LoadedDataset, LoadedLatencyCalibration,
    OpeningAccountProvenance, PINNED_JAVA_REVISION, ResearchCandidate, ResearchDataFormat,
    ResearchDataset, ResearchMode, ResearchOpeningAccount, ResearchScenario,
};

pub(super) fn load_candidates(
    specs: &[ResearchCandidate],
    base: &Path,
) -> Result<Vec<LoadedCandidate>> {
    let mut loaded = Vec::with_capacity(specs.len());
    let mut canonical_paths = HashSet::new();
    let mut hashes = HashSet::new();
    let mut effective_strategy_hashes = HashSet::new();
    for spec in specs {
        let resolved = resolve(base, &spec.config);
        let canonical = resolved.canonicalize().with_context(|| {
            format!("failed to resolve candidate config {}", resolved.display())
        })?;
        if !canonical_paths.insert(canonical.clone()) {
            bail!(
                "candidate config {} is referenced more than once",
                spec.config.display()
            );
        }
        let bytes = std::fs::read(&canonical)
            .with_context(|| format!("failed to read candidate config {}", canonical.display()))?;
        let sha256 = sha256_bytes(&bytes);
        if !hashes.insert(sha256.clone()) {
            bail!(
                "candidate {} duplicates another candidate's config bytes",
                spec.id
            );
        }
        let config: BacktestConfig = toml::from_str(
            std::str::from_utf8(&bytes).context("candidate config is not UTF-8")?,
        )
        .with_context(|| format!("failed to parse candidate config {}", canonical.display()))?;
        config.backtest.validate()?;
        crate::validate_currency_rate_coverage(&config.backtest, &config.strategy)?;
        config
            .initial_portfolio
            .validate(&config.strategy.effective(), &config.backtest)?;
        let validation = config.strategy.effective().validate();
        if !validation.valid {
            bail!(
                "candidate {} has invalid strategy config: {}",
                spec.id,
                validation.errors.join("; ")
            );
        }
        let effective_strategy_sha256 = effective_strategy_sha256(&config.strategy)?;
        if !effective_strategy_hashes.insert(effective_strategy_sha256.clone()) {
            bail!(
                "candidate {} duplicates another candidate's effective strategy",
                spec.id
            );
        }
        loaded.push(LoadedCandidate {
            spec: spec.clone(),
            resolved_path: canonical,
            config,
            sha256,
            effective_strategy_sha256,
        });
    }
    Ok(loaded)
}

pub(super) fn verify_input_hashes(
    manifest_path: &Path,
    manifest_sha256: &str,
    executable_path: &Path,
    executable_sha256: &str,
    candidates: &[LoadedCandidate],
    datasets: &[LoadedDataset],
    latency_calibration: Option<&LoadedLatencyCalibration>,
) -> Result<()> {
    if sha256_path(manifest_path)? != manifest_sha256 {
        bail!("research manifest changed while research was running");
    }
    if sha256_path(executable_path)? != executable_sha256 {
        bail!("research executable changed while research was running");
    }
    for candidate in candidates {
        if sha256_path(&candidate.resolved_path)? != candidate.sha256 {
            bail!(
                "candidate config {} changed while research was running",
                candidate.spec.id
            );
        }
    }
    for dataset in datasets {
        let final_sha256 = sha256_path(&dataset.resolved_path)?;
        if final_sha256 != dataset.sha256 {
            bail!(
                "dataset {} changed while research was running",
                dataset.spec.id
            );
        }
        if let (Some(config_path), Some(expected_sha256)) = (
            &dataset.resolved_capture_config,
            &dataset.capture_config_sha256,
        ) {
            let final_config_sha256 = sha256_path(config_path)?;
            if &final_config_sha256 != expected_sha256 {
                bail!(
                    "capture config for dataset {} changed while research was running",
                    dataset.spec.id
                );
            }
        }
        if let (Some(report_path), Some(expected_sha256)) = (
            &dataset.resolved_capture_report,
            &dataset.capture_report_sha256,
        ) {
            let final_report_sha256 = sha256_path(report_path)?;
            if &final_report_sha256 != expected_sha256 {
                bail!(
                    "capture report for dataset {} changed while research was running",
                    dataset.spec.id
                );
            }
        }
        if let (Some(normalized_path), Some(expected_sha256)) = (
            &dataset.resolved_normalized_path,
            &dataset.normalized_sha256,
        ) {
            let final_normalized_sha256 = sha256_path(normalized_path)?;
            if &final_normalized_sha256 != expected_sha256 {
                bail!(
                    "normalized capture for dataset {} changed while research was running",
                    dataset.spec.id
                );
            }
        }
        if let (Some(account_path), Some(opening_account)) =
            (&dataset.resolved_opening_account, &dataset.opening_account)
            && sha256_path(account_path)? != opening_account.sha256
        {
            bail!(
                "opening account certification for dataset {} changed while research was running",
                dataset.spec.id
            );
        }
    }
    if let Some(calibration) = latency_calibration
        && sha256_path(&calibration.resolved_path)? != calibration.provenance.sha256
    {
        bail!("latency calibration changed while research was running");
    }
    Ok(())
}

fn opening_account_evidence_sha256(artifact: &AccountCertificationArtifact) -> Result<String> {
    let index_responses = artifact
        .index_tickers
        .iter()
        .map(|ticker| {
            (
                ticker.currency.as_str(),
                ticker.symbol.as_str(),
                ticker.response.sha256.as_str(),
            )
        })
        .collect::<Vec<_>>();
    let material = serde_json::to_vec(&(
        artifact.schema_version,
        artifact.config.sha256.as_str(),
        artifact.config_fingerprint.as_str(),
        artifact.summary.account_identity_sha256.as_str(),
        artifact.start_clock.local_midpoint_ms,
        artifact.start_clock.server_ms,
        artifact.finish_clock.local_midpoint_ms,
        artifact.finish_clock.server_ms,
        artifact.account_config_before.sha256.as_str(),
        artifact.account_balance.sha256.as_str(),
        index_responses,
        artifact.account_positions.sha256.as_str(),
        artifact.account_config_after.sha256.as_str(),
    ))
    .context("failed to fingerprint opening account evidence")?;
    Ok(sha256_bytes(&material))
}

pub(super) fn sha256_path(path: &Path) -> Result<String> {
    let file = File::open(path)
        .with_context(|| format!("failed to open {} for SHA-256", path.display()))?;
    let mut reader = BufReader::new(file);
    let mut hasher = Sha256::new();
    let mut buffer = [0u8; 64 * 1024];
    loop {
        let read = reader
            .read(&mut buffer)
            .with_context(|| format!("failed to hash {}", path.display()))?;
        if read == 0 {
            break;
        }
        hasher.update(&buffer[..read]);
    }
    Ok(format!("{:x}", hasher.finalize()))
}

pub(super) fn verify_opening_account_certification_path(
    path: &Path,
) -> Result<AccountCertificationArtifact> {
    let metadata = std::fs::symlink_metadata(path).map_err(|error| {
        anyhow::anyhow!(
            "invalid account-certification artifact path {}: {error}",
            path.display()
        )
    })?;
    if metadata.file_type().is_symlink() || !metadata.is_file() {
        bail!(
            "invalid account-certification artifact path {}: must be a regular file and not a symbolic link",
            path.display()
        );
    }
    if metadata.len() > MAX_ACCOUNT_CERTIFICATION_ARTIFACT_BYTES {
        bail!(
            "account-certification artifact is {} bytes; limit is {}",
            metadata.len(),
            MAX_ACCOUNT_CERTIFICATION_ARTIFACT_BYTES
        );
    }
    let bytes = std::fs::read(path).map_err(|source| {
        anyhow::anyhow!(
            "failed to read account-certification artifact {}: {source}",
            path.display()
        )
    })?;
    if bytes.len() as u64 > MAX_ACCOUNT_CERTIFICATION_ARTIFACT_BYTES {
        bail!(
            "account-certification artifact is {} bytes; limit is {}",
            bytes.len(),
            MAX_ACCOUNT_CERTIFICATION_ARTIFACT_BYTES
        );
    }
    verify_account_certification_artifact_bytes(&bytes, path).map_err(Into::into)
}

pub(super) fn sha256_bytes(bytes: &[u8]) -> String {
    format!("{:x}", Sha256::digest(bytes))
}

pub fn effective_strategy_sha256(config: &reap_strategy::ChaosConfig) -> Result<String> {
    let bytes = serde_json::to_vec(&config.effective())
        .context("failed to serialize effective candidate strategy")?;
    Ok(sha256_bytes(&bytes))
}

pub(super) fn load_latency_calibration(
    spec: Option<&Path>,
    base: &Path,
    mode: ResearchMode,
    baseline: &ResearchScenario,
    executable_sha256: &str,
) -> Result<Option<LoadedLatencyCalibration>> {
    let Some(spec) = spec else {
        return Ok(None);
    };
    let resolved = resolve(base, spec);
    let canonical = resolved.canonicalize().with_context(|| {
        format!(
            "failed to resolve latency calibration {}",
            resolved.display()
        )
    })?;
    let artifact_size = std::fs::metadata(&canonical)
        .with_context(|| {
            format!(
                "failed to inspect latency calibration {}",
                canonical.display()
            )
        })?
        .len();
    if artifact_size > MAX_LATENCY_CALIBRATION_ARTIFACT_BYTES {
        bail!(
            "latency calibration is {artifact_size} bytes, maximum is {MAX_LATENCY_CALIBRATION_ARTIFACT_BYTES}"
        );
    }
    let bytes = std::fs::read(&canonical)
        .with_context(|| format!("failed to read latency calibration {}", canonical.display()))?;
    let sha256 = sha256_bytes(&bytes);
    let artifact: LatencyCalibrationArtifact =
        serde_json::from_slice(&bytes).with_context(|| {
            format!(
                "failed to parse latency calibration {}",
                canonical.display()
            )
        })?;
    artifact.validate_integrity().with_context(|| {
        format!(
            "latency calibration {} failed integrity validation",
            canonical.display()
        )
    })?;
    if artifact.reap_version != env!("CARGO_PKG_VERSION")
        || artifact.live_executable_sha256 != executable_sha256
    {
        bail!(
            "latency calibration was collected by a different Reap build than this research executable"
        );
    }
    if artifact.profile != baseline.execution.latency_profile {
        bail!("baseline latency profile does not exactly match the bound calibration artifact");
    }
    if mode == ResearchMode::ProductionCandidate && !baseline.execution.calibrated {
        bail!("production latency calibration requires a calibrated baseline execution");
    }
    let mut source_report_sha256s = artifact
        .source_reports
        .iter()
        .map(|source| source.sha256.clone())
        .collect::<Vec<_>>();
    source_report_sha256s.sort();
    source_report_sha256s.dedup();
    Ok(Some(LoadedLatencyCalibration {
        provenance: LatencyCalibrationProvenance {
            path: spec.to_path_buf(),
            sha256,
            schema_version: artifact.schema_version,
            reap_version: artifact.reap_version,
            live_executable_sha256: artifact.live_executable_sha256,
            host_identity_sha256: artifact.host_identity_sha256,
            account_identity_sha256s: artifact.account_identity_sha256s,
            live_config_sha256: artifact.live_config_sha256,
            live_config_fingerprint: artifact.live_config_fingerprint,
            live_config_evidence_fingerprint: artifact.live_config_evidence_fingerprint,
            minimum_samples_per_series: artifact.minimum_samples_per_series,
            matching_latency_is_upper_bound: artifact.matching_latency_is_upper_bound,
            source_report_sha256s,
            calibrated_series: artifact.series.len(),
        },
        resolved_path: canonical,
    }))
}

pub(super) fn load_datasets(
    specs: &[ResearchDataset],
    base: &Path,
    mode: ResearchMode,
    candidates: &[LoadedCandidate],
    expected_executable_sha256: &str,
    expected_host_identity_sha256: Option<&str>,
    maximum_opening_account_gap_ms: u64,
) -> Result<Vec<LoadedDataset>> {
    let mut loaded = Vec::with_capacity(specs.len());
    let mut canonical_ranges = HashMap::<PathBuf, Vec<(String, RawCaptureRecordRange)>>::new();
    let mut whole_canonical_paths = HashSet::new();
    let mut hashes = HashMap::<String, PathBuf>::new();
    let mut opening_account_paths = HashSet::new();
    let mut opening_account_hashes = HashSet::new();
    let mut opening_account_evidence_hashes = HashSet::new();
    for spec in specs {
        let resolved = resolve(base, &spec.path);
        let canonical = resolved
            .canonicalize()
            .with_context(|| format!("failed to resolve dataset {}", resolved.display()))?;
        if let Some(existing) = canonical_ranges.get_mut(&canonical) {
            if whole_canonical_paths.contains(&canonical) {
                bail!(
                    "dataset path {} is referenced both as a whole file and a record range",
                    spec.path.display()
                );
            }
            let range = spec.capture_record_range.with_context(|| {
                format!(
                    "dataset path {} is referenced more than once without a capture_record_range",
                    spec.path.display()
                )
            })?;
            if let Some((other_id, other_range)) = existing
                .iter()
                .find(|(_, other)| range.first <= other.last && other.first <= range.last)
            {
                bail!(
                    "dataset {} range {}..={} overlaps dataset {} range {}..={} in one raw capture",
                    spec.id,
                    range.first,
                    range.last,
                    other_id,
                    other_range.first,
                    other_range.last
                );
            }
            existing.push((spec.id.clone(), range));
        } else {
            if spec.capture_record_range.is_none() {
                whole_canonical_paths.insert(canonical.clone());
            }
            let ranges = spec
                .capture_record_range
                .map(|range| vec![(spec.id.clone(), range)])
                .unwrap_or_default();
            canonical_ranges.insert(canonical.clone(), ranges);
        }
        let sha256 = sha256_path(&canonical)?;
        if let Some(existing_path) = hashes.get(&sha256) {
            if existing_path != &canonical {
                bail!("dataset {} duplicates another dataset's bytes", spec.id);
            }
        } else {
            hashes.insert(sha256.clone(), canonical.clone());
        }
        let raw_replay_check = if spec.format == ResearchDataFormat::RawCapture {
            let report = replay_check_path(&canonical)
                .with_context(|| format!("failed to check raw dataset {}", canonical.display()))?;
            if mode == ResearchMode::ProductionCandidate
                && (!report.is_healthy() || report.gaps > 0 || report.recoveries > 0)
            {
                bail!(
                    "production dataset {} failed zero-gap replay integrity: errors={}, gaps={}, recoveries={}, recovery_failures={}, unrecovered_streams={}",
                    spec.id,
                    report.errors.len(),
                    report.gaps,
                    report.recoveries,
                    report.recovery_failures,
                    report.unrecovered_streams
                );
            }
            Some(report)
        } else {
            None
        };
        if spec.capture_report.is_some() && spec.capture_config.is_none() {
            bail!("dataset {} capture_report requires capture_config", spec.id);
        }
        if spec.normalized_path.is_some() && spec.capture_report.is_none() {
            bail!(
                "dataset {} normalized_path requires capture_report",
                spec.id
            );
        }
        if mode == ResearchMode::ProductionCandidate
            && (spec.capture_config.is_none() || spec.capture_report.is_none())
        {
            bail!(
                "production dataset {} requires capture_config and capture_report evidence",
                spec.id
            );
        }
        let resolved_capture_report = spec
            .capture_report
            .as_ref()
            .map(|report_path| -> Result<PathBuf> {
                let resolved = resolve(base, report_path);
                resolved.canonicalize().with_context(|| {
                    format!("failed to resolve capture report {}", resolved.display())
                })
            })
            .transpose()?;
        let resolved_normalized_path = spec
            .normalized_path
            .as_ref()
            .map(|normalized_path| -> Result<PathBuf> {
                let resolved = resolve(base, normalized_path);
                resolved.canonicalize().with_context(|| {
                    format!(
                        "failed to resolve normalized capture {}",
                        resolved.display()
                    )
                })
            })
            .transpose()?;

        let mut resolved_capture_config = None;
        let mut capture_config_sha256 = None;
        let mut capture_report_sha256 = None;
        let mut normalized_sha256 = None;
        let mut capture_analysis = None;
        let mut capture_verification = None;
        if let Some(config_path) = &spec.capture_config {
            let resolved_config = resolve(base, config_path);
            let canonical_config = resolved_config.canonicalize().with_context(|| {
                format!(
                    "failed to resolve capture config {}",
                    resolved_config.display()
                )
            })?;
            let config_bytes = std::fs::read(&canonical_config).with_context(|| {
                format!(
                    "failed to read capture config {}",
                    canonical_config.display()
                )
            })?;
            let config_sha256 = sha256_bytes(&config_bytes);
            let config = CaptureConfig::from_toml(
                std::str::from_utf8(&config_bytes).context("capture config is not UTF-8")?,
            )
            .with_context(|| {
                format!(
                    "failed to parse capture config {}",
                    canonical_config.display()
                )
            })?;
            if mode == ResearchMode::ProductionCandidate {
                validate_production_capture_config(&spec.id, &config, candidates)?;
            }

            let analysis = if let Some(report_path) = &resolved_capture_report {
                let verification = verify_capture_paths(
                    &canonical_config,
                    report_path,
                    &canonical,
                    resolved_normalized_path.as_deref(),
                )
                .with_context(|| {
                    format!("failed to verify capture evidence for dataset {}", spec.id)
                })?;
                if verification.config.sha256 != config_sha256 {
                    bail!(
                        "capture config for dataset {} changed while evidence was being loaded",
                        spec.id
                    );
                }
                if !verification.passed {
                    bail!(
                        "dataset {} failed capture verification: {:?}",
                        spec.id,
                        verification.failures
                    );
                }
                if mode == ResearchMode::ProductionCandidate {
                    if verification.reap_version != env!("CARGO_PKG_VERSION")
                        || verification.java_reference_revision != PINNED_JAVA_REVISION
                        || verification.executable_sha256 != expected_executable_sha256
                    {
                        bail!(
                            "production dataset {} was captured by a different Reap build or Java reference than this research run",
                            spec.id
                        );
                    }
                    let expected_host_identity_sha256 = expected_host_identity_sha256.context(
                        "production capture evidence requires a latency-calibrated target host",
                    )?;
                    if verification.host_identity_sha256.as_deref()
                        != Some(expected_host_identity_sha256)
                    {
                        bail!(
                            "production dataset {} was captured on a different host than the latency calibration",
                            spec.id
                        );
                    }
                    if verification.host_periodic_checks == 0 {
                        bail!(
                            "production dataset {} has no completed periodic host check",
                            spec.id
                        );
                    }
                }
                capture_report_sha256 = Some(verification.run_report.sha256.clone());
                normalized_sha256 = verification
                    .normalized
                    .as_ref()
                    .map(|artifact| artifact.actual_sha256.clone());
                let analysis = verification.analysis.clone();
                capture_verification = Some(verification);
                analysis
            } else {
                analyze_capture_path(&canonical, &config)
                    .with_context(|| format!("failed to analyze research dataset {}", spec.id))?
            };
            if !analysis.integrity_healthy {
                bail!(
                    "dataset {} failed capture-analysis integrity: errors={}, gaps={}, recovery_failures={}, receive_timestamp_regressions={}, unrecovered_books={}",
                    spec.id,
                    analysis.error_count,
                    analysis.gaps,
                    analysis.recovery_failures,
                    analysis.receive_timestamp_regressions,
                    analysis.unrecovered_book_streams
                );
            }
            if analysis.sha256 != sha256 {
                bail!(
                    "dataset {} analysis hash does not match input hash",
                    spec.id
                );
            }
            resolved_capture_config = Some(canonical_config);
            capture_config_sha256 = Some(config_sha256);
            capture_analysis = Some(analysis);
        }
        let (resolved_opening_account, opening_account) = load_dataset_opening_account(
            spec,
            base,
            mode,
            candidates,
            expected_executable_sha256,
            expected_host_identity_sha256,
            maximum_opening_account_gap_ms,
            capture_verification.as_ref(),
        )?;
        if let (Some(path), Some(provenance)) = (&resolved_opening_account, &opening_account) {
            if !opening_account_paths.insert(path.clone()) {
                bail!(
                    "opening account certification {} is referenced by more than one dataset",
                    provenance.source_path.display()
                );
            }
            if !opening_account_hashes.insert(provenance.sha256.clone()) {
                bail!(
                    "dataset {} opening account certification duplicates another dataset's bytes",
                    spec.id
                );
            }
            if !opening_account_evidence_hashes.insert(provenance.evidence_sha256.clone()) {
                bail!(
                    "dataset {} reuses another dataset's opening account evidence",
                    spec.id
                );
            }
        }
        loaded.push(LoadedDataset {
            spec: spec.clone(),
            resolved_path: canonical,
            sha256,
            raw_replay_check,
            resolved_capture_config,
            capture_config_sha256,
            resolved_capture_report,
            capture_report_sha256,
            resolved_normalized_path,
            normalized_sha256,
            capture_analysis,
            capture_verification,
            resolved_opening_account,
            opening_account,
        });
    }
    let loaded_by_id = loaded
        .iter()
        .map(|dataset| (dataset.spec.id.as_str(), dataset))
        .collect::<HashMap<_, _>>();
    for dataset in &loaded {
        let Some(parent_id) = dataset.spec.continuation_of.as_deref() else {
            continue;
        };
        let parent = loaded_by_id.get(parent_id).copied().with_context(|| {
            format!(
                "dataset {} has no loaded parent {parent_id}",
                dataset.spec.id
            )
        })?;
        if dataset.resolved_path != parent.resolved_path
            || dataset.resolved_capture_config != parent.resolved_capture_config
            || dataset.resolved_capture_report != parent.resolved_capture_report
            || dataset.resolved_normalized_path != parent.resolved_normalized_path
        {
            bail!(
                "continuation dataset {} must use the exact raw/config/report/normalized sources of parent {}",
                dataset.spec.id,
                parent.spec.id
            );
        }
    }
    Ok(loaded)
}

#[allow(clippy::too_many_arguments)]
fn load_dataset_opening_account(
    dataset: &ResearchDataset,
    base: &Path,
    mode: ResearchMode,
    candidates: &[LoadedCandidate],
    expected_executable_sha256: &str,
    expected_host_identity_sha256: Option<&str>,
    maximum_gap_ms: u64,
    capture_verification: Option<&CaptureVerificationReport>,
) -> Result<(Option<PathBuf>, Option<OpeningAccountProvenance>)> {
    let Some(spec) = &dataset.opening_account else {
        return Ok((None, None));
    };
    let resolved = resolve(base, &spec.certification);
    let canonical = resolved.canonicalize().with_context(|| {
        format!(
            "failed to resolve opening account certification {}",
            resolved.display()
        )
    })?;
    let sha256 = sha256_path(&canonical)?;
    let artifact = verify_opening_account_certification_path(&canonical).with_context(|| {
        format!(
            "failed to reconstruct opening account certification for dataset {}",
            dataset.id
        )
    })?;
    let evidence_sha256 = opening_account_evidence_sha256(&artifact)?;
    if !artifact.summary.passed || !artifact.summary.evidence_complete {
        bail!(
            "dataset {} opening account certification did not pass complete cash-account policy",
            dataset.id
        );
    }
    let capture = capture_verification.context(format!(
        "dataset {} opening account requires verified capture timing",
        dataset.id
    ))?;
    if artifact.reap_version != capture.reap_version
        || artifact.java_reference_revision != capture.java_reference_revision
        || artifact.executable_sha256 != capture.executable_sha256
    {
        bail!(
            "dataset {} opening account and capture were produced by different Reap builds or Java references",
            dataset.id
        );
    }
    if capture.host_identity_sha256.as_deref() != Some(artifact.host_identity_sha256.as_str()) {
        bail!(
            "dataset {} opening account and capture do not identify one host",
            dataset.id
        );
    }
    if artifact.finish_clock.local_midpoint_ms > capture.session_started_at_ms {
        bail!(
            "dataset {} opening account certification finished at {}, after capture started at {}",
            dataset.id,
            artifact.finish_clock.local_midpoint_ms,
            capture.session_started_at_ms
        );
    }
    let capture_gap_ms = capture
        .session_started_at_ms
        .saturating_sub(artifact.finish_clock.local_midpoint_ms);
    if capture_gap_ms > maximum_gap_ms {
        bail!(
            "dataset {} opening account gap {} ms exceeds configured maximum {} ms",
            dataset.id,
            capture_gap_ms,
            maximum_gap_ms
        );
    }

    let live_config = LiveConfig::from_toml(&artifact.config.toml).with_context(|| {
        format!(
            "dataset {} opening account embeds an invalid live config",
            dataset.id
        )
    })?;
    if mode == ResearchMode::ProductionCandidate {
        let expected_host = expected_host_identity_sha256
            .context("production opening account requires a latency-calibrated target host")?;
        if artifact.schema_version != ACCOUNT_CERTIFICATION_SCHEMA_VERSION
            || artifact.java_reference_revision != PINNED_JAVA_REVISION
            || artifact.reap_version != env!("CARGO_PKG_VERSION")
            || artifact.executable_sha256 != expected_executable_sha256
        {
            bail!(
                "production dataset {} opening account was certified by a different Reap build or Java reference",
                dataset.id
            );
        }
        if artifact.host_identity_sha256 != expected_host {
            bail!(
                "production dataset {} opening account, capture, and latency calibration do not identify one host",
                dataset.id
            );
        }
        if artifact.summary.environment != TradingEnvironment::Production {
            bail!(
                "production dataset {} opening account is not from the production environment",
                dataset.id
            );
        }
    }

    let balance = parse_okx_account_balance_response_json(artifact.account_balance.body.as_bytes())
        .with_context(|| {
            format!(
                "failed to parse verified opening balances for dataset {}",
                dataset.id
            )
        })?;
    let positions =
        parse_okx_account_positions_response_json(artifact.account_positions.body.as_bytes())
            .with_context(|| {
                format!(
                    "failed to parse verified opening positions for dataset {}",
                    dataset.id
                )
            })?;
    let mut portfolio = None;
    for candidate in candidates {
        let derived = derive_certified_opening_portfolio(
            dataset,
            spec,
            candidate,
            &live_config,
            &artifact.summary.account_id,
            &balance,
            &positions,
        )?;
        if let Some(expected) = &portfolio {
            if expected != &derived {
                bail!(
                    "dataset {} certified opening portfolio differs for candidate {}; candidates must share one account and instrument universe",
                    dataset.id,
                    candidate.spec.id
                );
            }
        } else {
            portfolio = Some(derived);
        }
    }
    let portfolio = portfolio.context("research requires at least one candidate")?;
    if mode == ResearchMode::ProductionCandidate && !portfolio.has_positive_balance() {
        bail!(
            "production dataset {} certified opening account has no positive modeled balance",
            dataset.id
        );
    }
    Ok((
        Some(canonical),
        Some(OpeningAccountProvenance {
            source_path: spec.certification.clone(),
            sha256,
            evidence_sha256,
            schema_version: artifact.schema_version,
            reap_version: artifact.reap_version,
            executable_sha256: artifact.executable_sha256,
            host_identity_sha256: artifact.host_identity_sha256,
            live_config_sha256: artifact.config.sha256,
            live_config_fingerprint: artifact.config_fingerprint,
            environment: artifact.summary.environment,
            account_id: artifact.summary.account_id,
            account_identity_sha256: artifact.summary.account_identity_sha256,
            certification_finish_local_midpoint_ms: artifact.finish_clock.local_midpoint_ms,
            certification_finish_server_ms: artifact.finish_clock.server_ms,
            capture_started_at_ms: capture.session_started_at_ms,
            capture_gap_ms,
            spot_valuation_symbols: spec.spot_valuation_symbols.clone(),
            portfolio,
        }),
    ))
}

pub(super) fn derive_certified_opening_portfolio(
    dataset: &ResearchDataset,
    opening: &ResearchOpeningAccount,
    candidate: &LoadedCandidate,
    live_config: &LiveConfig,
    account_id: &str,
    balance: &OkxAccountBalanceSnapshot,
    positions: &OkxAccountPositionsSnapshot,
) -> Result<BacktestInitialPortfolioConfig> {
    let candidate_account_ids = candidate
        .config
        .strategy
        .risk_groups
        .iter()
        .map(|group| group.account_id.as_deref())
        .collect::<BTreeSet<_>>();
    if candidate_account_ids != BTreeSet::from([Some(account_id)]) {
        bail!(
            "dataset {} candidate {} must bind every risk group to certified account {:?}",
            dataset.id,
            candidate.spec.id,
            account_id
        );
    }
    if let Some(group) = candidate.config.strategy.risk_groups.iter().find(|group| {
        group
            .coins
            .iter()
            .any(|coin| coin.borrow_limit_usd != 0.0 || coin.borrow_limit_coin != 0.0)
    }) {
        bail!(
            "dataset {} candidate {} risk group {} enables borrowing, which certified opening accounting does not model",
            dataset.id,
            candidate.spec.id,
            group.name
        );
    }
    validate_certified_instrument_scope(dataset, candidate, live_config, account_id)?;

    let mut required_currencies = BTreeSet::new();
    let mut spot_base_currencies = BTreeSet::new();
    for instrument in &candidate.config.strategy.instruments {
        if instrument.kind.is_spot() {
            required_currencies.insert(instrument.base_currency.clone());
            required_currencies.insert(instrument.quote_currency.clone());
            spot_base_currencies.insert(instrument.base_currency.clone());
        } else {
            required_currencies.insert(instrument.settle_currency.clone());
        }
    }
    let mapped_currencies = opening
        .spot_valuation_symbols
        .keys()
        .cloned()
        .collect::<BTreeSet<_>>();
    if mapped_currencies != spot_base_currencies {
        bail!(
            "dataset {} opening spot valuation currencies {:?} do not exactly match candidate {} spot bases {:?}",
            dataset.id,
            mapped_currencies,
            candidate.spec.id,
            spot_base_currencies
        );
    }

    let mut details = BTreeMap::new();
    for detail in &balance.details {
        if details.insert(detail.currency.as_str(), detail).is_some() {
            bail!(
                "dataset {} opening account repeats balance currency {}",
                dataset.id,
                detail.currency
            );
        }
        if detail.forced_repayment_indicator.unwrap_or(0) != 0 {
            bail!(
                "dataset {} opening account currency {} has an active forced repayment indicator",
                dataset.id,
                detail.currency
            );
        }
        let has_unmodeled_value = [
            detail.cash_balance,
            detail.available_balance,
            detail.equity,
            detail.equity_usd,
            detail.discounted_equity_usd,
            detail.unrealized_pnl,
            detail.liability,
            detail.cross_liability,
            detail.isolated_liability,
            detail.unrealized_loss_liability,
            detail.accrued_interest,
            detail.borrow_frozen_usd,
        ]
        .into_iter()
        .flatten()
        .any(|value| value != 0.0);
        if !required_currencies.contains(&detail.currency) && has_unmodeled_value {
            bail!(
                "dataset {} opening account has nonzero unmodeled balance or equity in currency {}",
                dataset.id,
                detail.currency
            );
        }
    }

    let mut initial_balances = Vec::with_capacity(required_currencies.len());
    for currency in required_currencies {
        let detail = details.get(currency.as_str()).copied();
        let total = detail.and_then(|item| item.cash_balance).unwrap_or(0.0);
        let available = detail
            .map(|item| {
                item.available_balance.with_context(|| {
                    format!(
                        "dataset {} opening balance {} omits availBal",
                        dataset.id, currency
                    )
                })
            })
            .transpose()?
            .unwrap_or(0.0);
        let equity = detail
            .map(|item| {
                item.equity.with_context(|| {
                    format!(
                        "dataset {} opening balance {} omits eq",
                        dataset.id, currency
                    )
                })
            })
            .transpose()?
            .unwrap_or(0.0);
        initial_balances.push(BacktestInitialBalanceConfig {
            currency: currency.clone(),
            total,
            available: Some(available),
            equity: Some(equity),
            liability: Some(detail.and_then(|item| item.liability).unwrap_or(0.0)),
            max_loan: Some(detail.and_then(|item| item.max_loan).unwrap_or(0.0)),
            forced_repayment_indicator: detail.and_then(|item| item.forced_repayment_indicator),
            valuation_symbol: opening.spot_valuation_symbols.get(&currency).cloned(),
        });
    }

    let instruments = candidate
        .config
        .strategy
        .instruments
        .iter()
        .map(|instrument| (instrument.symbol.as_str(), instrument))
        .collect::<HashMap<_, _>>();
    let mut certified_positions = HashMap::new();
    for risk in &positions.positions {
        let position = &risk.position;
        if certified_positions
            .insert(position.symbol.as_str(), position)
            .is_some()
        {
            bail!(
                "dataset {} opening account repeats position {}",
                dataset.id,
                position.symbol
            );
        }
        if position.qty == 0.0 {
            continue;
        }
        let instrument = instruments.get(position.symbol.as_str()).with_context(|| {
            format!(
                "dataset {} opening account has nonzero unmodeled position {}",
                dataset.id, position.symbol
            )
        })?;
        if instrument.kind.is_spot() {
            bail!(
                "dataset {} opening account reported spot position {} instead of cash balance",
                dataset.id,
                position.symbol
            );
        }
    }
    let live_account = live_config
        .accounts
        .iter()
        .find(|account| account.id == account_id)
        .with_context(|| {
            format!(
                "dataset {} certified live config omits account {}",
                dataset.id, account_id
            )
        })?;
    let mut initial_positions = Vec::new();
    for instrument in candidate
        .config
        .strategy
        .instruments
        .iter()
        .filter(|instrument| instrument.kind.is_derivative())
    {
        let expected_margin_mode = match live_account.trade_modes.get(&instrument.symbol) {
            Some(OkxTradeModeConfig::Cross) => PositionMarginMode::Cross,
            Some(OkxTradeModeConfig::Isolated) => PositionMarginMode::Isolated,
            Some(OkxTradeModeConfig::Cash) | None => {
                bail!(
                    "dataset {} derivative {} has no supported configured margin mode",
                    dataset.id,
                    instrument.symbol
                )
            }
        };
        let certified = certified_positions.get(instrument.symbol.as_str()).copied();
        if certified.is_some_and(|position| {
            position.qty != 0.0 && position.margin_mode != Some(expected_margin_mode)
        }) {
            bail!(
                "dataset {} opening position {} margin mode differs from certified live config",
                dataset.id,
                instrument.symbol
            );
        }
        initial_positions.push(BacktestInitialPositionConfig {
            symbol: instrument.symbol.clone(),
            qty: certified.map_or(0.0, |position| position.qty),
            avg_price: certified
                .filter(|position| position.qty != 0.0)
                .map_or(0.0, |position| position.avg_price),
            margin_mode: Some(expected_margin_mode),
        });
    }
    initial_positions.sort_by(|left, right| left.symbol.cmp(&right.symbol));
    let initial = BacktestInitialPortfolioConfig {
        account_id: Some(account_id.to_string()),
        balances: initial_balances,
        positions: initial_positions,
        margin: BacktestInitialMarginConfig {
            ratio: None,
            exchange_ratio: balance.margin_ratio,
            adjusted_equity_usd: balance.adjusted_equity_usd,
            notional_usd: balance.notional_usd,
        },
    };
    initial
        .validate(
            &candidate.config.strategy.effective(),
            &candidate.config.backtest,
        )
        .with_context(|| {
            format!(
                "dataset {} certified opening state is incompatible with candidate {}",
                dataset.id, candidate.spec.id
            )
        })?;
    Ok(initial)
}

fn validate_certified_instrument_scope(
    dataset: &ResearchDataset,
    candidate: &LoadedCandidate,
    live_config: &LiveConfig,
    account_id: &str,
) -> Result<()> {
    let source_instruments = live_config
        .strategy
        .instruments
        .iter()
        .map(|instrument| (instrument.symbol.as_str(), instrument))
        .collect::<HashMap<_, _>>();
    let source_groups = live_config
        .strategy
        .risk_groups
        .iter()
        .map(|group| (group.name.as_str(), group))
        .collect::<HashMap<_, _>>();
    for instrument in &candidate.config.strategy.instruments {
        let source = source_instruments
            .get(instrument.symbol.as_str())
            .with_context(|| {
                format!(
                    "dataset {} certified live config does not contain candidate {} instrument {}",
                    dataset.id, candidate.spec.id, instrument.symbol
                )
            })?;
        if source.kind != instrument.kind
            || source.base_currency != instrument.base_currency
            || source.quote_currency != instrument.quote_currency
            || source.settle_currency != instrument.settle_currency
            || source.contract_value.to_bits() != instrument.contract_value.to_bits()
        {
            bail!(
                "dataset {} certified live instrument {} accounting contract differs from candidate {}",
                dataset.id,
                instrument.symbol,
                candidate.spec.id
            );
        }
        let source_group = source_groups
            .get(source.risk_group.as_str())
            .with_context(|| {
                format!(
                    "dataset {} certified live instrument {} references unknown risk group {}",
                    dataset.id, source.symbol, source.risk_group
                )
            })?;
        if source_group.account_id.as_deref() != Some(account_id) {
            bail!(
                "dataset {} certified live instrument {} is not routed to account {:?}",
                dataset.id,
                source.symbol,
                account_id
            );
        }
    }
    Ok(())
}

pub(super) fn validate_production_capture_config(
    dataset_id: &str,
    config: &CaptureConfig,
    candidates: &[LoadedCandidate],
) -> Result<()> {
    if !config.host_guard.enabled {
        bail!("production dataset {dataset_id} requires an enabled capture host guard");
    }
    let host_guard_policy_errors = config.host_guard.production_policy_errors("host_guard");
    if !host_guard_policy_errors.is_empty() {
        bail!(
            "production dataset {dataset_id} capture host guard policy failed: {}",
            host_guard_policy_errors.join("; ")
        );
    }
    let connection_pacer_path = config
        .runtime
        .connection_attempt_pacer_path
        .as_ref()
        .context("production capture requires a process-shared connection pacer")?;
    if !connection_pacer_path.is_absolute() {
        bail!(
            "production dataset {dataset_id} requires an absolute process-shared connection pacer path"
        );
    }
    let streams = config
        .subscriptions
        .iter()
        .map(|subscription| {
            (
                subscription.channel.trim().to_string(),
                subscription.symbol.trim().to_string(),
            )
        })
        .collect::<HashSet<_>>();
    for subscription in &config.subscriptions {
        if subscription.connections < 2 {
            bail!(
                "production dataset {dataset_id} capture stream {}/{} requires at least two connections",
                subscription.channel,
                subscription.symbol
            );
        }
    }

    let has_stream =
        |channel: &str, symbol: &str| streams.contains(&(channel.to_string(), symbol.to_string()));
    let has_book = |symbol: &str| {
        ["books", "books-l2-tbt", "books50-l2-tbt"]
            .iter()
            .any(|channel| has_stream(channel, symbol))
    };
    for candidate in candidates {
        for route in &candidate.config.backtest.currency_rates {
            if !has_stream("index-tickers", &route.index_symbol) {
                bail!(
                    "production dataset {dataset_id} lacks index-tickers for candidate {} accounting currency {} via {}",
                    candidate.spec.id,
                    route.currency,
                    route.index_symbol
                );
            }
        }
        for instrument in &candidate.config.strategy.instruments {
            if !has_book(&instrument.symbol) {
                bail!(
                    "production dataset {dataset_id} lacks a book stream for candidate {} symbol {}",
                    candidate.spec.id,
                    instrument.symbol
                );
            }
            if !has_stream("trades", &instrument.symbol) {
                bail!(
                    "production dataset {dataset_id} lacks trades for candidate {} symbol {}",
                    candidate.spec.id,
                    instrument.symbol
                );
            }
            if instrument.kind.is_derivative() {
                for channel in ["mark-price", "price-limit"] {
                    if !has_stream(channel, &instrument.symbol) {
                        bail!(
                            "production dataset {dataset_id} lacks {channel} for candidate {} symbol {}",
                            candidate.spec.id,
                            instrument.symbol
                        );
                    }
                }
            }
            if instrument.kind.is_swap() && !has_stream("funding-rate", &instrument.symbol) {
                bail!(
                    "production dataset {dataset_id} lacks funding-rate for candidate {} symbol {}",
                    candidate.spec.id,
                    instrument.symbol
                );
            }
            if let Some(index_symbol) = &instrument.index_symbol
                && !has_stream("index-tickers", index_symbol)
            {
                bail!(
                    "production dataset {dataset_id} lacks index-tickers for candidate {} index symbol {}",
                    candidate.spec.id,
                    index_symbol
                );
            }
        }
    }
    Ok(())
}
