use std::collections::HashSet;
use std::fmt;
use std::fs::{self, File};
use std::io::{BufReader, Read};
use std::path::{Path, PathBuf};

use anyhow::{Context, Result, bail};
use serde::de::{self, DeserializeSeed, MapAccess, SeqAccess, Visitor};
use serde::{Deserialize, Serialize};
use serde_json::Value;
use sha2::{Digest, Sha256};

use crate::{PINNED_JAVA_REVISION, RESEARCH_SCHEMA_VERSION, ResearchReport};

pub const RESEARCH_VERIFICATION_FORMAT_VERSION: u16 = 3;
pub const MAX_RESEARCH_MANIFEST_BYTES: u64 = 4 * 1024 * 1024;
pub const MAX_RESEARCH_REPORT_BYTES: u64 = 64 * 1024 * 1024;
const MAX_RESEARCH_VERIFICATION_DIAGNOSTIC_BYTES: usize = 2_048;
const MAX_DIFFERENCE_VALUE_BYTES: usize = 256;

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ResearchFileEvidence {
    pub source_path: PathBuf,
    pub bytes: u64,
    pub sha256: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ResearchOpeningAccountEvidence {
    pub dataset_id: String,
    pub source_path: PathBuf,
    pub source_sha256: String,
    pub evidence_sha256: String,
    pub executable_sha256: String,
    pub host_identity_sha256: String,
    pub live_config_sha256: String,
    pub live_config_fingerprint: String,
    pub account_id: String,
    pub account_identity_sha256: String,
    pub certification_finish_server_ms: u64,
    pub capture_started_at_ms: u64,
    pub capture_gap_ms: u64,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "code", rename_all = "snake_case")]
pub enum ResearchVerificationFailure {
    ArtifactShapeInvalid,
    ArtifactSchemaUnsupported { actual: u32, supported: u32 },
    ArtifactManifestMismatch,
    ArtifactJavaRevisionMismatch,
    ArtifactExecutableMismatch,
    ArtifactVersionMismatch,
    ArtifactDeploymentBindingInvalid,
    ArtifactDidNotPass { failure_count: usize },
    RebuildFailed,
    RebuiltResearchDidNotPass { failure_count: usize },
    ArtifactDoesNotMatchRebuild,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ResearchVerificationReport {
    pub format_version: u16,
    pub java_reference_revision: String,
    pub verifier_reap_version: String,
    pub verifier_executable_sha256: String,
    pub manifest: ResearchFileEvidence,
    pub artifact: ResearchFileEvidence,
    pub artifact_schema_version: u32,
    pub artifact_mode: crate::ResearchMode,
    pub artifact_reap_version: String,
    pub artifact_executable_sha256: String,
    pub artifact_deployment_candidate_id: Option<String>,
    pub artifact_deployment_effective_strategy_sha256: Option<String>,
    pub artifact_deployment_binding_valid: bool,
    pub artifact_opening_accounts: Vec<ResearchOpeningAccountEvidence>,
    pub artifact_manifest_matches: bool,
    pub artifact_shape_valid: bool,
    pub artifact_reported_pass: bool,
    pub rebuild_succeeded: bool,
    pub rebuilt_report_passed: bool,
    pub artifact_matches_rebuild: bool,
    pub normalized_artifact_sha256: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub normalized_rebuild_sha256: Option<String>,
    pub failures: Vec<ResearchVerificationFailure>,
    pub diagnostics: Vec<String>,
    pub limitations: Vec<String>,
    pub acceptance_passed: bool,
}

pub fn verify_research_paths(
    manifest_path: impl AsRef<Path>,
    artifact_path: impl AsRef<Path>,
) -> Result<ResearchVerificationReport> {
    let (manifest, _) = read_bounded_regular_file(
        manifest_path.as_ref(),
        "research manifest",
        MAX_RESEARCH_MANIFEST_BYTES,
    )?;
    let (artifact, artifact_bytes) = read_bounded_regular_file(
        artifact_path.as_ref(),
        "research report",
        MAX_RESEARCH_REPORT_BYTES,
    )?;
    if manifest.source_path == artifact.source_path {
        bail!(
            "research manifest and report resolve to the same file {}",
            manifest.source_path.display()
        );
    }

    validate_unique_json_keys(&artifact_bytes).with_context(|| {
        format!(
            "research report {} contains ambiguous JSON",
            artifact.source_path.display()
        )
    })?;
    let artifact_value: Value = serde_json::from_slice(&artifact_bytes).with_context(|| {
        format!(
            "failed to parse research report {} as JSON",
            artifact.source_path.display()
        )
    })?;
    let artifact_report: ResearchReport =
        serde_json::from_slice(&artifact_bytes).with_context(|| {
            format!(
                "failed to parse research report {}",
                artifact.source_path.display()
            )
        })?;
    let typed_artifact_value = serde_json::to_value(&artifact_report)
        .context("failed to serialize parsed research report")?;
    let artifact_shape_valid = artifact_value == typed_artifact_value;
    drop(artifact_value);
    drop(typed_artifact_value);
    let artifact_manifest_matches = artifact_report.manifest_sha256 == manifest.sha256;
    let artifact_schema_version = artifact_report.schema_version;
    let artifact_mode = artifact_report.mode;
    let artifact_reap_version = artifact_report.reap_version.clone();
    let artifact_executable_sha256 = artifact_report.executable_sha256.clone();
    let (
        artifact_deployment_candidate_id,
        artifact_deployment_effective_strategy_sha256,
        artifact_deployment_binding_valid,
    ) = deployment_binding(&artifact_report);
    let artifact_reported_pass = artifact_report.passed;
    let artifact_failure_count = artifact_report.failures.len();
    let artifact_opening_accounts = artifact_report
        .datasets
        .iter()
        .filter_map(|dataset| {
            dataset
                .opening_account
                .as_ref()
                .map(|account| ResearchOpeningAccountEvidence {
                    dataset_id: dataset.id.clone(),
                    source_path: account.source_path.clone(),
                    source_sha256: account.sha256.clone(),
                    evidence_sha256: account.evidence_sha256.clone(),
                    executable_sha256: account.executable_sha256.clone(),
                    host_identity_sha256: account.host_identity_sha256.clone(),
                    live_config_sha256: account.live_config_sha256.clone(),
                    live_config_fingerprint: account.live_config_fingerprint.clone(),
                    account_id: account.account_id.clone(),
                    account_identity_sha256: account.account_identity_sha256.clone(),
                    certification_finish_server_ms: account.certification_finish_server_ms,
                    capture_started_at_ms: account.capture_started_at_ms,
                    capture_gap_ms: account.capture_gap_ms,
                })
        })
        .collect::<Vec<_>>();
    let verifier_executable_sha256 =
        sha256_path(&std::env::current_exe().context("failed to resolve verifier executable")?)?;

    let mut failures = Vec::new();
    if !artifact_shape_valid {
        failures.push(ResearchVerificationFailure::ArtifactShapeInvalid);
    }
    if artifact_schema_version != RESEARCH_SCHEMA_VERSION {
        failures.push(ResearchVerificationFailure::ArtifactSchemaUnsupported {
            actual: artifact_schema_version,
            supported: RESEARCH_SCHEMA_VERSION,
        });
    }
    if !artifact_manifest_matches {
        failures.push(ResearchVerificationFailure::ArtifactManifestMismatch);
    }
    if artifact_report.java_reference_revision != PINNED_JAVA_REVISION {
        failures.push(ResearchVerificationFailure::ArtifactJavaRevisionMismatch);
    }
    if artifact_executable_sha256 != verifier_executable_sha256 {
        failures.push(ResearchVerificationFailure::ArtifactExecutableMismatch);
    }
    if artifact_reap_version != env!("CARGO_PKG_VERSION") {
        failures.push(ResearchVerificationFailure::ArtifactVersionMismatch);
    }
    if !artifact_deployment_binding_valid {
        failures.push(ResearchVerificationFailure::ArtifactDeploymentBindingInvalid);
    }
    if !artifact_reported_pass {
        failures.push(ResearchVerificationFailure::ArtifactDidNotPass {
            failure_count: artifact_failure_count,
        });
    }

    let normalized_artifact = normalized_report_value(artifact_report)?;
    let normalized_artifact_sha256 = normalized_value_sha256(&normalized_artifact)?;
    let mut diagnostics = Vec::new();
    let mut rebuild_succeeded = false;
    let mut rebuilt_report_passed = false;
    let mut artifact_matches_rebuild = false;
    let mut normalized_rebuild_sha256 = None;
    match crate::run_research_manifest_path(&manifest.source_path) {
        Ok(rebuilt) => {
            rebuild_succeeded = true;
            rebuilt_report_passed = rebuilt.passed;
            if !rebuilt.passed {
                failures.push(ResearchVerificationFailure::RebuiltResearchDidNotPass {
                    failure_count: rebuilt.failures.len(),
                });
                diagnostics.extend(
                    rebuilt
                        .failures
                        .iter()
                        .take(16)
                        .map(|failure| bounded_diagnostic(format!("rebuild: {failure}"))),
                );
            }
            let normalized_rebuild = normalized_report_value(rebuilt)?;
            normalized_rebuild_sha256 = Some(normalized_value_sha256(&normalized_rebuild)?);
            artifact_matches_rebuild = normalized_artifact == normalized_rebuild;
            if !artifact_matches_rebuild {
                failures.push(ResearchVerificationFailure::ArtifactDoesNotMatchRebuild);
                let (path, artifact_value, rebuild_value) =
                    first_difference(&normalized_artifact, &normalized_rebuild).unwrap_or_else(
                        || {
                            (
                                "/".to_string(),
                                "<unknown>".to_string(),
                                "<unknown>".to_string(),
                            )
                        },
                    );
                diagnostics.push(bounded_diagnostic(format!(
                    "artifact differs from rebuild at {path}: artifact={artifact_value}, rebuild={rebuild_value}"
                )));
            }
        }
        Err(error) => {
            failures.push(ResearchVerificationFailure::RebuildFailed);
            diagnostics.push(bounded_diagnostic(format!("research rebuild: {error:#}")));
        }
    }

    let acceptance_passed = failures.is_empty()
        && artifact_shape_valid
        && artifact_manifest_matches
        && artifact_deployment_binding_valid
        && artifact_reported_pass
        && rebuild_succeeded
        && rebuilt_report_passed
        && artifact_matches_rebuild;
    Ok(ResearchVerificationReport {
        format_version: RESEARCH_VERIFICATION_FORMAT_VERSION,
        java_reference_revision: PINNED_JAVA_REVISION.to_string(),
        verifier_reap_version: env!("CARGO_PKG_VERSION").to_string(),
        verifier_executable_sha256,
        manifest,
        artifact,
        artifact_schema_version,
        artifact_mode,
        artifact_reap_version,
        artifact_executable_sha256,
        artifact_deployment_candidate_id,
        artifact_deployment_effective_strategy_sha256,
        artifact_deployment_binding_valid,
        artifact_opening_accounts,
        artifact_manifest_matches,
        artifact_shape_valid,
        artifact_reported_pass,
        rebuild_succeeded,
        rebuilt_report_passed,
        artifact_matches_rebuild,
        normalized_artifact_sha256,
        normalized_rebuild_sha256,
        failures,
        diagnostics,
        limitations: vec![
            "a passing reconstruction proves deterministic derivation from the supplied manifest and current archived inputs; embedded source verifiers bind declared venue, host, and account identities but do not provide external host/process attestation"
                .to_string(),
            "research acceptance remains conditional on independently verified capture, latency, fee, funding, account, statement, fault, and deployment evidence"
                .to_string(),
            "a passing research report is economic simulation evidence and does not authorize production order entry"
                .to_string(),
        ],
        acceptance_passed,
    })
}

fn deployment_binding(report: &ResearchReport) -> (Option<String>, Option<String>, bool) {
    let candidate_id = report.deployment_candidate_id.clone();
    match report.mode {
        crate::ResearchMode::Smoke => {
            let valid = candidate_id.is_none();
            (candidate_id, None, valid)
        }
        crate::ResearchMode::ProductionCandidate => {
            let Some(candidate_id_ref) = candidate_id.as_deref() else {
                return (None, None, false);
            };
            let mut matching = report
                .candidates
                .iter()
                .filter(|candidate| candidate.id == candidate_id_ref);
            let effective_strategy_sha256 = matching
                .next()
                .map(|candidate| candidate.effective_strategy_sha256.clone());
            let exactly_one_candidate =
                effective_strategy_sha256.is_some() && matching.next().is_none();
            let valid = exactly_one_candidate
                && effective_strategy_sha256
                    .as_deref()
                    .is_some_and(is_lower_sha256)
                && !report.folds.is_empty()
                && report
                    .folds
                    .iter()
                    .all(|fold| fold.selected_candidate_id.as_deref() == Some(candidate_id_ref));
            (candidate_id, effective_strategy_sha256, valid)
        }
    }
}

fn is_lower_sha256(value: &str) -> bool {
    value.len() == 64
        && value
            .bytes()
            .all(|byte| byte.is_ascii_digit() || (b'a'..=b'f').contains(&byte))
}

fn validate_unique_json_keys(bytes: &[u8]) -> Result<()> {
    struct UniqueKeys;

    impl<'de> DeserializeSeed<'de> for UniqueKeys {
        type Value = ();

        fn deserialize<D>(self, deserializer: D) -> std::result::Result<Self::Value, D::Error>
        where
            D: serde::Deserializer<'de>,
        {
            deserializer.deserialize_any(self)
        }
    }

    impl<'de> Visitor<'de> for UniqueKeys {
        type Value = ();

        fn expecting(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
            formatter.write_str("a JSON value without duplicate object keys")
        }

        fn visit_bool<E>(self, _value: bool) -> std::result::Result<Self::Value, E> {
            Ok(())
        }

        fn visit_i64<E>(self, _value: i64) -> std::result::Result<Self::Value, E> {
            Ok(())
        }

        fn visit_u64<E>(self, _value: u64) -> std::result::Result<Self::Value, E> {
            Ok(())
        }

        fn visit_f64<E>(self, _value: f64) -> std::result::Result<Self::Value, E> {
            Ok(())
        }

        fn visit_str<E>(self, _value: &str) -> std::result::Result<Self::Value, E>
        where
            E: de::Error,
        {
            Ok(())
        }

        fn visit_string<E>(self, _value: String) -> std::result::Result<Self::Value, E>
        where
            E: de::Error,
        {
            Ok(())
        }

        fn visit_unit<E>(self) -> std::result::Result<Self::Value, E> {
            Ok(())
        }

        fn visit_none<E>(self) -> std::result::Result<Self::Value, E> {
            Ok(())
        }

        fn visit_some<D>(self, deserializer: D) -> std::result::Result<Self::Value, D::Error>
        where
            D: serde::Deserializer<'de>,
        {
            UniqueKeys.deserialize(deserializer)
        }

        fn visit_seq<A>(self, mut sequence: A) -> std::result::Result<Self::Value, A::Error>
        where
            A: SeqAccess<'de>,
        {
            while sequence.next_element_seed(UniqueKeys)?.is_some() {}
            Ok(())
        }

        fn visit_map<A>(self, mut map: A) -> std::result::Result<Self::Value, A::Error>
        where
            A: MapAccess<'de>,
        {
            let mut keys = HashSet::new();
            while let Some(key) = map.next_key::<String>()? {
                if !keys.insert(key.clone()) {
                    return Err(de::Error::custom(format!(
                        "duplicate JSON object key {key:?}"
                    )));
                }
                map.next_value_seed(UniqueKeys)?;
            }
            Ok(())
        }
    }

    let mut deserializer = serde_json::Deserializer::from_slice(bytes);
    UniqueKeys
        .deserialize(&mut deserializer)
        .context("failed to validate JSON object keys")?;
    deserializer
        .end()
        .context("failed to validate complete JSON document")?;
    Ok(())
}

fn normalized_value_sha256(report: &Value) -> Result<String> {
    let bytes =
        serde_json::to_vec(report).context("failed to serialize normalized research report")?;
    Ok(sha256_bytes(&bytes))
}

fn normalized_report_value(mut report: ResearchReport) -> Result<Value> {
    for dataset in &mut report.datasets {
        if let Some(opening_account) = &mut dataset.opening_account {
            opening_account.source_path = content_path(&opening_account.sha256);
        }
        if let Some(analysis) = &mut dataset.capture_analysis {
            analysis.source_path = Some(content_path(&analysis.sha256));
        }
        if let Some(verification) = &mut dataset.capture_verification {
            verification.run_report.source_path = content_path(&verification.run_report.sha256);
            verification.config.source_path = content_path(&verification.config.sha256);
            verification.raw.source_path = content_path(&verification.raw.actual_sha256);
            verification.analysis.source_path = Some(content_path(&verification.analysis.sha256));
            if let Some(normalized) = &mut verification.normalized {
                normalized.source_path = content_path(&normalized.actual_sha256);
            }
        }
    }
    serde_json::to_value(report).context("failed to serialize path-normalized research report")
}

fn content_path(sha256: &str) -> PathBuf {
    PathBuf::from(format!("sha256:{sha256}"))
}

fn first_difference(left: &Value, right: &Value) -> Option<(String, String, String)> {
    fn visit(left: &Value, right: &Value, path: &mut String) -> Option<(String, String, String)> {
        match (left, right) {
            (Value::Array(left), Value::Array(right)) => {
                if left.len() != right.len() {
                    path.push_str("/length");
                    return Some((
                        path.clone(),
                        left.len().to_string(),
                        right.len().to_string(),
                    ));
                }
                for (index, (left, right)) in left.iter().zip(right).enumerate() {
                    let original_len = path.len();
                    path.push('/');
                    path.push_str(&index.to_string());
                    if let Some(difference) = visit(left, right, path) {
                        return Some(difference);
                    }
                    path.truncate(original_len);
                }
                None
            }
            (Value::Object(left), Value::Object(right)) => {
                let mut keys = left.keys().chain(right.keys()).collect::<Vec<_>>();
                keys.sort();
                keys.dedup();
                for key in keys {
                    let original_len = path.len();
                    path.push('/');
                    path.push_str(&key.replace('~', "~0").replace('/', "~1"));
                    match (left.get(key), right.get(key)) {
                        (Some(left), Some(right)) => {
                            if let Some(difference) = visit(left, right, path) {
                                return Some(difference);
                            }
                            path.truncate(original_len);
                        }
                        (Some(left), None) => {
                            return Some((
                                path.clone(),
                                display_difference_value(left),
                                "<missing>".to_string(),
                            ));
                        }
                        (None, Some(right)) => {
                            return Some((
                                path.clone(),
                                "<missing>".to_string(),
                                display_difference_value(right),
                            ));
                        }
                        (None, None) => unreachable!("key came from at least one object"),
                    }
                }
                None
            }
            _ if left != right => Some((
                path.clone(),
                display_difference_value(left),
                display_difference_value(right),
            )),
            _ => None,
        }
    }

    let mut path = String::new();
    visit(left, right, &mut path)
}

fn display_difference_value(value: &Value) -> String {
    let value = match value {
        Value::Array(values) => format!("<array:{}>", values.len()),
        Value::Object(values) => format!("<object:{}>", values.len()),
        _ => value.to_string(),
    };
    if value.len() <= MAX_DIFFERENCE_VALUE_BYTES {
        return value;
    }
    let mut end = MAX_DIFFERENCE_VALUE_BYTES;
    while !value.is_char_boundary(end) {
        end -= 1;
    }
    format!("{}...", &value[..end])
}

fn read_bounded_regular_file(
    path: &Path,
    label: &'static str,
    limit: u64,
) -> Result<(ResearchFileEvidence, Vec<u8>)> {
    let metadata = fs::symlink_metadata(path)
        .with_context(|| format!("failed to inspect {label} {}", path.display()))?;
    if metadata.file_type().is_symlink() || !metadata.is_file() {
        bail!(
            "{label} {} must be a regular file and not a symbolic link",
            path.display()
        );
    }
    if metadata.len() > limit {
        bail!(
            "{label} {} is {} bytes; maximum is {limit}",
            path.display(),
            metadata.len()
        );
    }
    let canonical = fs::canonicalize(path)
        .with_context(|| format!("failed to resolve {label} {}", path.display()))?;
    let bytes = fs::read(&canonical)
        .with_context(|| format!("failed to read {label} {}", canonical.display()))?;
    if bytes.len() as u64 > limit {
        bail!(
            "{label} {} is {} bytes; maximum is {limit}",
            canonical.display(),
            bytes.len()
        );
    }
    Ok((
        ResearchFileEvidence {
            source_path: canonical,
            bytes: bytes.len() as u64,
            sha256: sha256_bytes(&bytes),
        },
        bytes,
    ))
}

fn sha256_path(path: &Path) -> Result<String> {
    let file = File::open(path)
        .with_context(|| format!("failed to open executable {}", path.display()))?;
    let mut reader = BufReader::new(file);
    let mut hasher = Sha256::new();
    let mut buffer = [0_u8; 64 * 1024];
    loop {
        let read = reader
            .read(&mut buffer)
            .with_context(|| format!("failed to read executable {}", path.display()))?;
        if read == 0 {
            break;
        }
        hasher.update(&buffer[..read]);
    }
    Ok(hasher
        .finalize()
        .iter()
        .map(|byte| format!("{byte:02x}"))
        .collect())
}

fn sha256_bytes(bytes: &[u8]) -> String {
    let digest = Sha256::digest(bytes);
    digest.iter().map(|byte| format!("{byte:02x}")).collect()
}

fn bounded_diagnostic(value: String) -> String {
    let sanitized = value
        .chars()
        .map(|character| {
            if character.is_control() {
                ' '
            } else {
                character
            }
        })
        .collect::<String>();
    if sanitized.len() <= MAX_RESEARCH_VERIFICATION_DIAGNOSTIC_BYTES {
        return sanitized;
    }
    let mut end = MAX_RESEARCH_VERIFICATION_DIAGNOSTIC_BYTES;
    while !sanitized.is_char_boundary(end) {
        end -= 1;
    }
    sanitized[..end].to_string()
}

#[cfg(test)]
mod tests {
    use std::fs;

    use serde_json::json;
    use tempfile::tempdir;

    use super::*;

    fn smoke_manifest() -> PathBuf {
        PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../../examples/research-smoke.toml")
    }

    #[test]
    fn verifier_rebuilds_exact_report_and_rejects_forged_or_unknown_fields() {
        let directory = tempdir().unwrap();
        let manifest = smoke_manifest();
        let report = crate::run_research_manifest_path(&manifest).unwrap();
        assert!(report.passed, "{:#?}", report.failures);
        let mut production_binding = report.clone();
        production_binding.mode = crate::ResearchMode::ProductionCandidate;
        production_binding.deployment_candidate_id = Some("base".to_string());
        let (candidate_id, strategy_sha256, binding_valid) =
            deployment_binding(&production_binding);
        assert_eq!(candidate_id.as_deref(), Some("base"));
        assert_eq!(
            strategy_sha256.as_deref(),
            Some(report.candidates[0].effective_strategy_sha256.as_str())
        );
        assert!(binding_valid);
        production_binding
            .candidates
            .push(production_binding.candidates[0].clone());
        assert!(!deployment_binding(&production_binding).2);

        let encoded = serde_json::to_vec_pretty(&report).unwrap();
        let decoded: ResearchReport = serde_json::from_slice(&encoded).unwrap();
        assert_eq!(
            report.aggregate.total_baseline_test_pnl_usd.to_bits(),
            decoded.aggregate.total_baseline_test_pnl_usd.to_bits(),
            "direct JSON round trip changed {} into {}",
            report.aggregate.total_baseline_test_pnl_usd,
            decoded.aggregate.total_baseline_test_pnl_usd
        );
        let value: Value = serde_json::from_slice(&encoded).unwrap();
        let decoded_from_value: ResearchReport = serde_json::from_value(value).unwrap();
        assert_eq!(
            report.aggregate.total_baseline_test_pnl_usd.to_bits(),
            decoded_from_value
                .aggregate
                .total_baseline_test_pnl_usd
                .to_bits(),
            "Value round trip changed {} into {}",
            report.aggregate.total_baseline_test_pnl_usd,
            decoded_from_value.aggregate.total_baseline_test_pnl_usd
        );
        let artifact_path = directory.path().join("research.json");
        fs::write(&artifact_path, &encoded).unwrap();

        let verification = verify_research_paths(&manifest, &artifact_path).unwrap();
        assert!(verification.acceptance_passed, "{verification:#?}");
        assert_eq!(
            verification.format_version,
            RESEARCH_VERIFICATION_FORMAT_VERSION
        );
        assert!(verification.artifact_shape_valid);
        assert!(verification.artifact_matches_rebuild);
        assert!(verification.artifact_deployment_binding_valid);
        assert_eq!(verification.artifact_deployment_candidate_id, None);
        assert!(verification.artifact_opening_accounts.is_empty());
        assert_eq!(
            verification.artifact_deployment_effective_strategy_sha256,
            None
        );
        assert_eq!(
            verification.normalized_rebuild_sha256.as_deref(),
            Some(verification.normalized_artifact_sha256.as_str())
        );

        let mut forged = serde_json::to_value(&report).unwrap();
        forged["aggregate"]["total_baseline_test_pnl_usd"] = json!(123456.0);
        let forged_path = directory.path().join("forged-research.json");
        fs::write(&forged_path, serde_json::to_vec_pretty(&forged).unwrap()).unwrap();
        let forged_verification = verify_research_paths(&manifest, &forged_path).unwrap();
        assert!(!forged_verification.acceptance_passed);
        assert!(forged_verification.artifact_shape_valid);
        assert!(!forged_verification.artifact_matches_rebuild);
        assert!(
            forged_verification
                .failures
                .contains(&ResearchVerificationFailure::ArtifactDoesNotMatchRebuild)
        );

        let mut extended = serde_json::to_value(&report).unwrap();
        extended["unreviewed_approval"] = json!(true);
        let extended_path = directory.path().join("extended-research.json");
        fs::write(
            &extended_path,
            serde_json::to_vec_pretty(&extended).unwrap(),
        )
        .unwrap();
        let extended_verification = verify_research_paths(&manifest, &extended_path).unwrap();
        assert!(!extended_verification.acceptance_passed);
        assert!(!extended_verification.artifact_shape_valid);
        assert!(
            extended_verification
                .failures
                .contains(&ResearchVerificationFailure::ArtifactShapeInvalid)
        );

        let mut invalid_binding = serde_json::to_value(&report).unwrap();
        invalid_binding["deployment_candidate_id"] = json!("base");
        let invalid_binding_path = directory.path().join("invalid-deployment-binding.json");
        fs::write(
            &invalid_binding_path,
            serde_json::to_vec_pretty(&invalid_binding).unwrap(),
        )
        .unwrap();
        let invalid_binding_verification =
            verify_research_paths(&manifest, &invalid_binding_path).unwrap();
        assert!(!invalid_binding_verification.acceptance_passed);
        assert!(!invalid_binding_verification.artifact_deployment_binding_valid);
        assert!(
            invalid_binding_verification
                .failures
                .contains(&ResearchVerificationFailure::ArtifactDeploymentBindingInvalid)
        );

        let mut duplicate = String::from_utf8(encoded).unwrap();
        let root_end = duplicate.rfind('}').unwrap();
        duplicate.insert_str(root_end, ",\n  \"passed\": true\n");
        let duplicate_path = directory.path().join("duplicate-key-research.json");
        fs::write(&duplicate_path, duplicate).unwrap();
        let duplicate_error = verify_research_paths(&manifest, &duplicate_path).unwrap_err();
        assert!(
            format!("{duplicate_error:#}").contains("duplicate JSON object key \"passed\""),
            "{duplicate_error:#}"
        );
    }
}
