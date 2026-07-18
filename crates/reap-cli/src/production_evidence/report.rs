use std::path::PathBuf;

use anyhow::Result;
use serde::Serialize;

use super::canonical::serialized_sha256;
use super::{
    ProductionEvidenceExpectedIdentity, ProductionEvidenceGate, ProductionEvidenceGateReport,
    ProductionEvidenceManifest,
};

pub(super) fn gate_report<T: Serialize>(
    gate: ProductionEvidenceGate,
    subject: Option<String>,
    source_paths: Vec<PathBuf>,
    reconstructed: &T,
    acceptance_passed: bool,
) -> Result<ProductionEvidenceGateReport> {
    Ok(ProductionEvidenceGateReport {
        gate,
        subject,
        source_paths,
        reconstructed_sha256: serialized_sha256(reconstructed)?,
        acceptance_passed,
    })
}

pub(super) fn expected_identity(
    manifest: &ProductionEvidenceManifest,
) -> ProductionEvidenceExpectedIdentity {
    ProductionEvidenceExpectedIdentity {
        reap_version: manifest.expected_reap_version.clone(),
        live_executable_sha256: manifest.expected_live_executable_sha256.clone(),
        host_identity_sha256: manifest.expected_host_identity_sha256.clone(),
        approval_policy_sha256: manifest.expected_approval_policy_sha256.clone(),
        deployment_candidate_id: manifest.expected_deployment_candidate_id.clone(),
        demo_account_identity_sha256s: manifest.expected_demo_account_identity_sha256s.clone(),
        production_account_identity_sha256s: manifest
            .expected_production_account_identity_sha256s
            .clone(),
    }
}
