use std::collections::HashSet;
use std::fs::File;
use std::io::{BufReader, Read};
use std::path::Path;

use anyhow::{Context, Result, bail};
use reap_live_contracts::{
    AccountCertificationArtifact, MAX_ACCOUNT_CERTIFICATION_ARTIFACT_BYTES,
    verify_account_certification_artifact_bytes,
};
use sha2::{Digest, Sha256};

use crate::BacktestConfig;

use super::configuration::resolve;
use super::{LoadedCandidate, LoadedDataset, LoadedLatencyCalibration, ResearchCandidate};

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

pub(super) fn opening_account_evidence_sha256(
    artifact: &AccountCertificationArtifact,
) -> Result<String> {
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
