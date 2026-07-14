use std::collections::{BTreeMap, HashMap, HashSet};
use std::path::PathBuf;

use anyhow::{Result, bail};
use reap_core::PINNED_JAVA_REVISION;
use serde::{Deserialize, Serialize};

use crate::{BacktestLatencyClass, BacktestLatencyProfile};

pub const LATENCY_CALIBRATION_SCHEMA_VERSION: u32 = 4;
pub const MAX_LATENCY_CALIBRATION_SOURCE_REPORTS: usize = 32;
pub const MAX_LATENCY_CALIBRATION_RETAINED_INPUT_SAMPLES: usize = 4_000_000;
pub const MAX_LATENCY_CALIBRATION_ARTIFACT_BYTES: u64 = 64 * 1024 * 1024;

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct LatencyCalibrationArtifact {
    pub schema_version: u32,
    pub java_reference_revision: String,
    pub reap_version: String,
    pub live_executable_sha256: String,
    pub host_identity_sha256: String,
    pub account_identity_sha256s: BTreeMap<String, String>,
    pub live_config_sha256: String,
    pub live_config_fingerprint: String,
    pub live_config_evidence_fingerprint: String,
    pub profile_seed: u64,
    pub minimum_samples_per_series: u64,
    pub matching_latency_is_upper_bound: bool,
    pub matching_upper_bounds_accepted: bool,
    pub source_reports: Vec<LatencySourceReport>,
    pub series: Vec<LatencyCalibrationSeries>,
    pub profile: BacktestLatencyProfile,
    pub passed: bool,
    pub failures: Vec<String>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct LatencySourceReport {
    pub path: PathBuf,
    pub sha256: String,
    pub session_id: String,
    pub mode: String,
    pub reap_version: String,
    pub executable_sha256: String,
    pub host_identity_sha256: String,
    pub account_identity_sha256s: BTreeMap<String, String>,
    pub config_fingerprint: String,
    pub evidence_config_fingerprint: String,
    pub clean_soak: bool,
    pub reached_ready: bool,
    pub clock_guarded: bool,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct LatencyCalibrationSeries {
    pub class: BacktestLatencyClass,
    pub symbol: String,
    pub semantics: String,
    pub source_report_sha256s: Vec<String>,
    pub total_valid_observations: u64,
    pub total_operation_failures: u64,
    pub retained_input_samples: usize,
    pub profile_samples_ms: Vec<u64>,
    pub passed: bool,
    pub failures: Vec<String>,
}

impl LatencyCalibrationArtifact {
    /// Treat calibration JSON as untrusted evidence before binding it to research.
    pub fn validate_integrity(&self) -> Result<()> {
        if self.schema_version != LATENCY_CALIBRATION_SCHEMA_VERSION {
            bail!(
                "latency calibration schema must be {LATENCY_CALIBRATION_SCHEMA_VERSION}, got {}",
                self.schema_version
            );
        }
        if self.java_reference_revision != PINNED_JAVA_REVISION {
            bail!("latency calibration Java revision is not the pinned revision");
        }
        if self.reap_version.is_empty()
            || !is_lower_sha256(&self.live_executable_sha256)
            || !is_lower_sha256(&self.host_identity_sha256)
            || self.account_identity_sha256s.is_empty()
            || self
                .account_identity_sha256s
                .iter()
                .any(|(account_id, hash)| account_id.is_empty() || !is_lower_sha256(hash))
        {
            bail!("latency calibration has invalid build, host, or account provenance");
        }
        if !self.passed || !self.failures.is_empty() {
            bail!("latency calibration artifact did not pass its evidence gates");
        }
        if self.minimum_samples_per_series == 0
            || self.series.is_empty()
            || self.source_reports.is_empty()
            || self.source_reports.len() > MAX_LATENCY_CALIBRATION_SOURCE_REPORTS
        {
            bail!("latency calibration artifact has incomplete source evidence");
        }
        if !self.matching_latency_is_upper_bound || !self.matching_upper_bounds_accepted {
            bail!(
                "latency calibration must retain and explicitly accept matching-delay upper-bound semantics"
            );
        }
        for (label, value) in [
            ("live config SHA-256", self.live_config_sha256.as_str()),
            (
                "live config checkpoint fingerprint",
                self.live_config_fingerprint.as_str(),
            ),
            (
                "live config evidence fingerprint",
                self.live_config_evidence_fingerprint.as_str(),
            ),
        ] {
            if !is_lower_sha256(value) {
                bail!("latency calibration has invalid {label}");
            }
        }
        if self.profile_seed != self.profile.seed {
            bail!("latency calibration profile seed does not match its provenance");
        }
        self.profile.validate()?;

        let mut sources = HashMap::new();
        let mut sessions = HashSet::new();
        let mut paths = HashSet::new();
        for source in &self.source_reports {
            if !is_lower_sha256(&source.sha256) {
                bail!("latency calibration contains an invalid source report SHA-256");
            }
            if sources.insert(source.sha256.as_str(), source).is_some() {
                bail!("latency calibration repeats a source report SHA-256");
            }
            if source.session_id.is_empty() || !sessions.insert(source.session_id.as_str()) {
                bail!("latency calibration has an empty or repeated source session id");
            }
            if !paths.insert(source.path.as_path()) {
                bail!("latency calibration repeats a source report path");
            }
            if !matches!(source.mode.as_str(), "observe" | "demo") {
                bail!("latency calibration source mode must be observe or demo");
            }
            if !source.clean_soak || !source.reached_ready || !source.clock_guarded {
                bail!("latency calibration contains an invalid source live report");
            }
            if source.reap_version != self.reap_version
                || source.executable_sha256 != self.live_executable_sha256
                || source.host_identity_sha256 != self.host_identity_sha256
                || source.account_identity_sha256s != self.account_identity_sha256s
                || source.config_fingerprint != self.live_config_fingerprint
                || source.evidence_config_fingerprint != self.live_config_evidence_fingerprint
            {
                bail!("latency calibration source provenance is inconsistent");
            }
        }

        let mut calibrated = BTreeMap::new();
        let mut total_retained_input_samples = 0_usize;
        for series in &self.series {
            total_retained_input_samples = total_retained_input_samples
                .checked_add(series.retained_input_samples)
                .ok_or_else(|| anyhow::anyhow!("latency calibration sample count overflow"))?;
            if total_retained_input_samples > MAX_LATENCY_CALIBRATION_RETAINED_INPUT_SAMPLES {
                bail!("latency calibration retains too many input samples");
            }
            if !series.passed
                || !series.failures.is_empty()
                || series.total_valid_observations < self.minimum_samples_per_series
                || series.total_operation_failures > 0
                || series.retained_input_samples == 0
                || series.profile_samples_ms.is_empty()
                || series.profile_samples_ms.len() > series.retained_input_samples
            {
                bail!("latency calibration contains an incomplete or failed series");
            }
            if series.semantics != expected_semantics(series.class) {
                bail!("latency calibration series has incorrect delay semantics");
            }
            if !series
                .profile_samples_ms
                .windows(2)
                .all(|pair| pair[0] <= pair[1])
            {
                bail!("latency calibration profile samples must be sorted");
            }
            let key = (series.class, series.symbol.clone());
            if calibrated.insert(key, series).is_some() {
                bail!("latency calibration repeats a class/symbol series");
            }
            if series.source_report_sha256s.is_empty() {
                bail!("latency calibration series has no source reports");
            }
            let mut series_sources = HashSet::new();
            for source_hash in &series.source_report_sha256s {
                if !series_sources.insert(source_hash.as_str()) {
                    bail!("latency calibration series repeats a source report");
                }
                let Some(source) = sources.get(source_hash.as_str()) else {
                    bail!("latency calibration series references an unknown source report");
                };
                if requires_demo(series.class) && source.mode != "demo" {
                    bail!("private-path latency calibration requires demo source reports");
                }
            }
        }

        if self.profile.rules.len() != calibrated.len() {
            bail!("latency calibration profile and evidence series counts differ");
        }
        for rule in &self.profile.rules {
            let Some(symbol) = &rule.symbol else {
                bail!("calibrated latency profile rules must be symbol-specific");
            };
            let Some(series) = calibrated.get(&(rule.class, symbol.clone())) else {
                bail!("latency calibration profile rule has no evidence series");
            };
            if rule.samples_ms != series.profile_samples_ms {
                bail!("latency calibration profile samples do not match evidence series");
            }
        }
        Ok(())
    }
}

fn expected_semantics(class: BacktestLatencyClass) -> &'static str {
    match class {
        BacktestLatencyClass::MarketDepth
        | BacktestLatencyClass::HistoricalTrade
        | BacktestLatencyClass::ReferenceData => "host_receive_to_strategy_visibility",
        BacktestLatencyClass::MatchingNew | BacktestLatencyClass::MatchingCancel => {
            "strategy_dispatch_to_order_ack_upper_bound"
        }
        BacktestLatencyClass::OrderUpdate => "exchange_timestamp_to_strategy_visibility",
        BacktestLatencyClass::OrderFill => "fill_to_account_state_visibility",
    }
}

fn requires_demo(class: BacktestLatencyClass) -> bool {
    matches!(
        class,
        BacktestLatencyClass::MatchingNew
            | BacktestLatencyClass::MatchingCancel
            | BacktestLatencyClass::OrderUpdate
            | BacktestLatencyClass::OrderFill
    )
}

fn is_lower_sha256(value: &str) -> bool {
    value.len() == 64
        && value
            .bytes()
            .all(|byte| byte.is_ascii_digit() || (b'a'..=b'f').contains(&byte))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::BacktestLatencyRule;

    fn artifact() -> LatencyCalibrationArtifact {
        let source_hash = "d".repeat(64);
        let samples = vec![1, 2, 3];
        LatencyCalibrationArtifact {
            schema_version: LATENCY_CALIBRATION_SCHEMA_VERSION,
            java_reference_revision: PINNED_JAVA_REVISION.to_string(),
            reap_version: "0.1.0".to_string(),
            live_executable_sha256: "9".repeat(64),
            host_identity_sha256: "8".repeat(64),
            account_identity_sha256s: BTreeMap::from([("main".to_string(), "7".repeat(64))]),
            live_config_sha256: "a".repeat(64),
            live_config_fingerprint: "b".repeat(64),
            live_config_evidence_fingerprint: "c".repeat(64),
            profile_seed: 42,
            minimum_samples_per_series: 3,
            matching_latency_is_upper_bound: true,
            matching_upper_bounds_accepted: true,
            source_reports: vec![LatencySourceReport {
                path: "/tmp/live-report.json".into(),
                sha256: source_hash.clone(),
                session_id: "session-1".to_string(),
                mode: "observe".to_string(),
                reap_version: "0.1.0".to_string(),
                executable_sha256: "9".repeat(64),
                host_identity_sha256: "8".repeat(64),
                account_identity_sha256s: BTreeMap::from([("main".to_string(), "7".repeat(64))]),
                config_fingerprint: "b".repeat(64),
                evidence_config_fingerprint: "c".repeat(64),
                clean_soak: true,
                reached_ready: true,
                clock_guarded: true,
            }],
            series: vec![LatencyCalibrationSeries {
                class: BacktestLatencyClass::MarketDepth,
                symbol: "BTC-USDT".to_string(),
                semantics: "host_receive_to_strategy_visibility".to_string(),
                source_report_sha256s: vec![source_hash],
                total_valid_observations: 3,
                total_operation_failures: 0,
                retained_input_samples: 3,
                profile_samples_ms: samples.clone(),
                passed: true,
                failures: Vec::new(),
            }],
            profile: BacktestLatencyProfile {
                seed: 42,
                rules: vec![BacktestLatencyRule {
                    class: BacktestLatencyClass::MarketDepth,
                    symbol: Some("BTC-USDT".to_string()),
                    samples_ms: samples,
                }],
            },
            passed: true,
            failures: Vec::new(),
        }
    }

    #[test]
    fn artifact_integrity_binds_series_sources_and_profile() {
        artifact().validate_integrity().unwrap();

        let mut wrong_semantics = artifact();
        wrong_semantics.series[0].semantics = "wrong_latency_semantics".to_string();
        assert!(
            wrong_semantics
                .validate_integrity()
                .unwrap_err()
                .to_string()
                .contains("semantics")
        );

        let mut wrong_profile = artifact();
        wrong_profile.profile.rules[0].samples_ms[0] = 99;
        assert!(
            wrong_profile
                .validate_integrity()
                .unwrap_err()
                .to_string()
                .contains("do not match")
        );

        let mut wrong_config = artifact();
        wrong_config.source_reports[0].evidence_config_fingerprint = "e".repeat(64);
        assert!(
            wrong_config
                .validate_integrity()
                .unwrap_err()
                .to_string()
                .contains("provenance")
        );
    }
}
