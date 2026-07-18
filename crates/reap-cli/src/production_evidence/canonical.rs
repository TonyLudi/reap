use anyhow::{Context, Result};
use serde::Serialize;
use sha2::{Digest, Sha256};

use super::ProductionEvidenceFailure;

pub(super) fn scenario_name(scenario: reap_live::LiveFaultScenario) -> String {
    serde_json::to_value(scenario)
        .ok()
        .and_then(|value| value.as_str().map(str::to_string))
        .unwrap_or_else(|| format!("{scenario:?}"))
}

pub(super) fn failure_sort_key(failure: &ProductionEvidenceFailure) -> String {
    serde_json::to_string(failure).unwrap_or_else(|_| format!("{failure:?}"))
}

pub(super) fn serialized_sha256<T: Serialize>(value: &T) -> Result<String> {
    let bytes = serde_json::to_vec(value).context("failed to serialize reconstructed evidence")?;
    Ok(sha256_bytes(&bytes))
}

pub(super) fn sha256_bytes(bytes: &[u8]) -> String {
    format!("{:x}", Sha256::digest(bytes))
}
