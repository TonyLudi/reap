use std::path::Path;

use anyhow::{Context, Result};
use reap_backtest::{
    ResearchMode, ResearchOpeningAccountEvidence, ResearchVerificationReport,
    effective_strategy_sha256, verify_research_paths,
};
use reap_core::PINNED_JAVA_REVISION;
use reap_live::{LiveConfig, LiveConfigFileEvidence, TradingEnvironment};
use serde::{Deserialize, Serialize};

pub(crate) const RESEARCH_DEPLOYMENT_VERIFICATION_FORMAT_VERSION: u16 = 2;

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct ResearchDeploymentConfigEvidence {
    pub file: LiveConfigFileEvidence,
    pub config_fingerprint: String,
    pub evidence_config_fingerprint: String,
    pub environment: TradingEnvironment,
    pub effective_strategy_sha256: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "code", rename_all = "snake_case")]
pub(crate) enum ResearchDeploymentVerificationFailure {
    InputPathCollision,
    ProductionEnvironmentRequired {
        actual: TradingEnvironment,
    },
    ResearchVerificationFailed,
    ProductionResearchRequired {
        actual: ResearchMode,
    },
    DeploymentBindingInvalid,
    DeploymentStrategyHashMissing,
    EffectiveStrategyMismatch {
        research_sha256: String,
        live_sha256: String,
    },
    OpeningAccountEvidenceMissing,
    OpeningAccountConfigMismatch {
        dataset_id: String,
        expected_live_config_sha256: String,
        actual_opening_config_sha256: String,
    },
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct ResearchDeploymentVerificationReport {
    pub format_version: u16,
    pub java_reference_revision: String,
    pub reap_version: String,
    pub production_config: ResearchDeploymentConfigEvidence,
    pub research: ResearchVerificationReport,
    pub deployment_candidate_id: Option<String>,
    pub research_effective_strategy_sha256: Option<String>,
    pub effective_strategy_matches: bool,
    pub failures: Vec<ResearchDeploymentVerificationFailure>,
    pub limitations: Vec<String>,
    pub acceptance_passed: bool,
}

pub(crate) fn verify_research_deployment_paths(
    production_config_path: &Path,
    research_manifest_path: &Path,
    research_report_path: &Path,
) -> Result<ResearchDeploymentVerificationReport> {
    let research = verify_research_paths(research_manifest_path, research_report_path)
        .with_context(|| {
            format!(
                "failed to reconstruct research report {} from {}",
                research_report_path.display(),
                research_manifest_path.display()
            )
        })?;
    // Research reconstruction can be expensive; bind the production config bytes
    // only after it completes so a long-running verification cannot retain stale input.
    let (config, config_file) = LiveConfig::load_with_evidence(production_config_path)
        .with_context(|| {
            format!(
                "failed to load production live config {}",
                production_config_path.display()
            )
        })?;
    let live_effective_strategy_sha256 = effective_strategy_sha256(&config.strategy)?;
    let input_path_collision = config_file.source_path == research.manifest.source_path
        || config_file.source_path == research.artifact.source_path;
    let mut failures = binding_failures(
        config.venue.environment,
        research.artifact_mode,
        research.acceptance_passed,
        research.artifact_deployment_binding_valid,
        research
            .artifact_deployment_effective_strategy_sha256
            .as_deref(),
        &live_effective_strategy_sha256,
        input_path_collision,
    );
    failures.extend(opening_account_config_failures(
        &research.artifact_opening_accounts,
        &config_file.sha256,
    ));
    let effective_strategy_matches = research
        .artifact_deployment_effective_strategy_sha256
        .as_deref()
        == Some(live_effective_strategy_sha256.as_str());
    let acceptance_passed = failures.is_empty();
    Ok(ResearchDeploymentVerificationReport {
        format_version: RESEARCH_DEPLOYMENT_VERIFICATION_FORMAT_VERSION,
        java_reference_revision: PINNED_JAVA_REVISION.to_string(),
        reap_version: env!("CARGO_PKG_VERSION").to_string(),
        production_config: ResearchDeploymentConfigEvidence {
            file: config_file,
            config_fingerprint: config.fingerprint()?,
            evidence_config_fingerprint: config.evidence_fingerprint()?,
            environment: config.venue.environment,
            effective_strategy_sha256: live_effective_strategy_sha256,
        },
        deployment_candidate_id: research.artifact_deployment_candidate_id.clone(),
        research_effective_strategy_sha256: research
            .artifact_deployment_effective_strategy_sha256
            .clone(),
        effective_strategy_matches,
        research,
        failures,
        limitations: vec![
            "a passing binding proves that one reconstructed production-research candidate has the same effective strategy as the exact proposed production config and that every dataset opening was certified from those exact config bytes; it does not prove profitability or evidence freshness"
                .to_string(),
            "credential permissions, target-account state, target-host operation, fault campaigns, statement reconciliation, and emergency procedures remain separate production gates"
                .to_string(),
            "this verifier does not authorize or enable production order entry".to_string(),
        ],
        acceptance_passed,
    })
}

fn opening_account_config_failures(
    openings: &[ResearchOpeningAccountEvidence],
    production_config_sha256: &str,
) -> Vec<ResearchDeploymentVerificationFailure> {
    if openings.is_empty() {
        return vec![ResearchDeploymentVerificationFailure::OpeningAccountEvidenceMissing];
    }
    openings
        .iter()
        .filter(|opening| opening.live_config_sha256 != production_config_sha256)
        .map(
            |opening| ResearchDeploymentVerificationFailure::OpeningAccountConfigMismatch {
                dataset_id: opening.dataset_id.clone(),
                expected_live_config_sha256: production_config_sha256.to_string(),
                actual_opening_config_sha256: opening.live_config_sha256.clone(),
            },
        )
        .collect()
}

fn binding_failures(
    environment: TradingEnvironment,
    research_mode: ResearchMode,
    research_acceptance_passed: bool,
    deployment_binding_valid: bool,
    research_sha256: Option<&str>,
    live_sha256: &str,
    input_path_collision: bool,
) -> Vec<ResearchDeploymentVerificationFailure> {
    let mut failures = Vec::new();
    if input_path_collision {
        failures.push(ResearchDeploymentVerificationFailure::InputPathCollision);
    }
    if environment != TradingEnvironment::Production {
        failures.push(
            ResearchDeploymentVerificationFailure::ProductionEnvironmentRequired {
                actual: environment,
            },
        );
    }
    if !research_acceptance_passed {
        failures.push(ResearchDeploymentVerificationFailure::ResearchVerificationFailed);
    }
    if research_mode != ResearchMode::ProductionCandidate {
        failures.push(
            ResearchDeploymentVerificationFailure::ProductionResearchRequired {
                actual: research_mode,
            },
        );
    }
    if !deployment_binding_valid {
        failures.push(ResearchDeploymentVerificationFailure::DeploymentBindingInvalid);
    }
    match research_sha256 {
        None => failures.push(ResearchDeploymentVerificationFailure::DeploymentStrategyHashMissing),
        Some(research_sha256) if research_sha256 != live_sha256 => failures.push(
            ResearchDeploymentVerificationFailure::EffectiveStrategyMismatch {
                research_sha256: research_sha256.to_string(),
                live_sha256: live_sha256.to_string(),
            },
        ),
        Some(_) => {}
    }
    failures
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::path::PathBuf;

    #[test]
    fn production_binding_requires_every_independent_gate_and_exact_strategy() {
        let hash = "a".repeat(64);
        assert!(
            binding_failures(
                TradingEnvironment::Production,
                ResearchMode::ProductionCandidate,
                true,
                true,
                Some(&hash),
                &hash,
                false,
            )
            .is_empty()
        );

        let failures = binding_failures(
            TradingEnvironment::Demo,
            ResearchMode::Smoke,
            false,
            false,
            Some(&"b".repeat(64)),
            &hash,
            true,
        );
        assert_eq!(failures.len(), 6);
        assert!(failures.contains(&ResearchDeploymentVerificationFailure::InputPathCollision));
        assert!(failures.contains(
            &ResearchDeploymentVerificationFailure::ProductionEnvironmentRequired {
                actual: TradingEnvironment::Demo,
            }
        ));
        assert!(
            failures.contains(&ResearchDeploymentVerificationFailure::ResearchVerificationFailed)
        );
        assert!(failures.contains(
            &ResearchDeploymentVerificationFailure::ProductionResearchRequired {
                actual: ResearchMode::Smoke,
            }
        ));
        assert!(
            failures.contains(&ResearchDeploymentVerificationFailure::DeploymentBindingInvalid)
        );
        assert!(failures.iter().any(|failure| matches!(
            failure,
            ResearchDeploymentVerificationFailure::EffectiveStrategyMismatch { .. }
        )));

        let failures = binding_failures(
            TradingEnvironment::Production,
            ResearchMode::ProductionCandidate,
            true,
            true,
            None,
            &hash,
            false,
        );
        assert_eq!(
            failures,
            [ResearchDeploymentVerificationFailure::DeploymentStrategyHashMissing]
        );
    }

    #[test]
    fn production_binding_requires_exact_opening_account_config_bytes() {
        let config_sha256 = "a".repeat(64);
        let opening = ResearchOpeningAccountEvidence {
            dataset_id: "train".to_string(),
            source_path: PathBuf::from("opening.json"),
            source_sha256: "b".repeat(64),
            evidence_sha256: "c".repeat(64),
            executable_sha256: "d".repeat(64),
            host_identity_sha256: "e".repeat(64),
            live_config_sha256: config_sha256.clone(),
            live_config_fingerprint: "f".repeat(64),
            account_id: "main".to_string(),
            account_identity_sha256: "1".repeat(64),
            certification_finish_server_ms: 100,
            capture_started_at_ms: 101,
            capture_gap_ms: 1,
        };

        assert!(
            opening_account_config_failures(std::slice::from_ref(&opening), &config_sha256)
                .is_empty()
        );
        assert_eq!(
            opening_account_config_failures(&[], &config_sha256),
            [ResearchDeploymentVerificationFailure::OpeningAccountEvidenceMissing]
        );
        assert!(matches!(
            opening_account_config_failures(&[opening], &"2".repeat(64)).as_slice(),
            [ResearchDeploymentVerificationFailure::OpeningAccountConfigMismatch { .. }]
        ));
    }
}
