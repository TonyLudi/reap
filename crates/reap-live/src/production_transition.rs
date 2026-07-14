use std::collections::BTreeSet;
use std::path::{Path, PathBuf};

use reap_core::PINNED_JAVA_REVISION;
use serde::{Deserialize, Serialize};
use serde_json::Value;
use thiserror::Error;

use crate::provenance::sha256_bytes;
use crate::{
    LiveConfig, LiveConfigError, LiveConfigFileEvidence, OkxEndpointRegion, TradingEnvironment,
};

pub const PRODUCTION_TRANSITION_FORMAT_VERSION: u16 = 1;
pub const MAX_REPORTED_TRANSITION_CHANGES: usize = 256;
const MAX_DISPLAY_STRING_BYTES: usize = 256;

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ProductionTransitionConfigEvidence {
    pub file: LiveConfigFileEvidence,
    pub config_fingerprint: String,
    pub evidence_config_fingerprint: String,
    pub environment: TradingEnvironment,
    pub endpoint_region: OkxEndpointRegion,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum TransitionValueKind {
    Missing,
    Null,
    Boolean,
    Number,
    String,
    Array,
    Object,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct TransitionValueEvidence {
    pub kind: TransitionValueKind,
    pub sha256: Option<String>,
    pub display: Option<Value>,
    pub entries: Option<u64>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ProductionTransitionChange {
    /// RFC 6901 JSON Pointer into the serialized effective `LiveConfig`.
    pub path: String,
    pub demo: TransitionValueEvidence,
    pub production: TransitionValueEvidence,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "code", rename_all = "snake_case")]
pub enum ProductionTransitionFailure {
    DemoEnvironmentRequired {
        actual: TradingEnvironment,
    },
    ProductionEnvironmentRequired {
        actual: TradingEnvironment,
    },
    DemoLoopbackNotEligible,
    EndpointRegionMismatch {
        demo: OkxEndpointRegion,
        production: OkxEndpointRegion,
    },
    DisallowedConfigurationDrift {
        count: u64,
    },
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ProductionTransitionReport {
    pub format_version: u16,
    pub java_reference_revision: String,
    pub reap_version: String,
    pub demo: ProductionTransitionConfigEvidence,
    pub production: ProductionTransitionConfigEvidence,
    pub environment_pair_valid: bool,
    pub endpoint_region_consistent: bool,
    pub allowed_change_count: u64,
    pub allowed_changes: Vec<ProductionTransitionChange>,
    pub allowed_changes_truncated: bool,
    pub disallowed_change_count: u64,
    pub disallowed_changes: Vec<ProductionTransitionChange>,
    pub disallowed_changes_truncated: bool,
    pub failures: Vec<ProductionTransitionFailure>,
    pub limitations: Vec<String>,
    pub acceptance_passed: bool,
}

#[derive(Debug, Error)]
pub enum ProductionTransitionError {
    #[error("failed to process {label} config: {source}")]
    Config {
        label: &'static str,
        #[source]
        source: LiveConfigError,
    },
    #[error("demo and production config resolve to the same file {0}")]
    PathCollision(PathBuf),
    #[error("failed to serialize effective configs: {0}")]
    Serialize(#[from] serde_json::Error),
    #[error("validated endpoint tuple had no region: {0}")]
    EndpointInvariant(String),
}

pub fn verify_production_transition_paths(
    demo_config_path: impl AsRef<Path>,
    production_config_path: impl AsRef<Path>,
) -> Result<ProductionTransitionReport, ProductionTransitionError> {
    let (demo_config, demo_file) =
        LiveConfig::load_with_evidence(demo_config_path).map_err(|source| {
            ProductionTransitionError::Config {
                label: "demo",
                source,
            }
        })?;
    let (production_config, production_file) =
        LiveConfig::load_with_evidence(production_config_path).map_err(|source| {
            ProductionTransitionError::Config {
                label: "production",
                source,
            }
        })?;
    if demo_file.source_path == production_file.source_path {
        return Err(ProductionTransitionError::PathCollision(
            demo_file.source_path,
        ));
    }

    let demo = config_evidence("demo", &demo_config, demo_file)?;
    let production = config_evidence("production", &production_config, production_file)?;
    let demo_value = serde_json::to_value(&demo_config)?;
    let production_value = serde_json::to_value(&production_config)?;
    let mut differences = DifferenceCollector::default();
    collect_differences(
        Some(&demo_value),
        Some(&production_value),
        &mut Vec::new(),
        &mut differences,
    );

    let environment_pair_valid = demo.environment == TradingEnvironment::Demo
        && production.environment == TradingEnvironment::Production;
    let endpoint_region_consistent = demo.endpoint_region != OkxEndpointRegion::DemoLoopback
        && demo.endpoint_region == production.endpoint_region;
    let mut failures = Vec::new();
    if demo.environment != TradingEnvironment::Demo {
        failures.push(ProductionTransitionFailure::DemoEnvironmentRequired {
            actual: demo.environment,
        });
    }
    if production.environment != TradingEnvironment::Production {
        failures.push(ProductionTransitionFailure::ProductionEnvironmentRequired {
            actual: production.environment,
        });
    }
    if demo.endpoint_region == OkxEndpointRegion::DemoLoopback {
        failures.push(ProductionTransitionFailure::DemoLoopbackNotEligible);
    } else if demo.endpoint_region != production.endpoint_region {
        failures.push(ProductionTransitionFailure::EndpointRegionMismatch {
            demo: demo.endpoint_region,
            production: production.endpoint_region,
        });
    }
    if differences.disallowed_count > 0 {
        failures.push(ProductionTransitionFailure::DisallowedConfigurationDrift {
            count: differences.disallowed_count,
        });
    }
    let acceptance_passed = failures.is_empty();
    Ok(ProductionTransitionReport {
        format_version: PRODUCTION_TRANSITION_FORMAT_VERSION,
        java_reference_revision: PINNED_JAVA_REVISION.to_string(),
        reap_version: env!("CARGO_PKG_VERSION").to_string(),
        demo,
        production,
        environment_pair_valid,
        endpoint_region_consistent,
        allowed_change_count: differences.allowed_count,
        allowed_changes: differences.allowed,
        allowed_changes_truncated: differences.allowed_truncated,
        disallowed_change_count: differences.disallowed_count,
        disallowed_changes: differences.disallowed,
        disallowed_changes_truncated: differences.disallowed_truncated,
        failures,
        limitations: vec![
            "This verifier compares configuration only; it does not authorize or enable production order entry."
                .to_string(),
            "Credential environment-variable values, API-key permissions, IP restrictions, and exchange account identities are not read or verified."
                .to_string(),
            "Passing demo, fault, reconciliation, emergency, target-host, and operator rollout evidence remain separate production gates."
                .to_string(),
        ],
        acceptance_passed,
    })
}

fn config_evidence(
    label: &'static str,
    config: &LiveConfig,
    file: LiveConfigFileEvidence,
) -> Result<ProductionTransitionConfigEvidence, ProductionTransitionError> {
    let endpoint_region = config.venue.endpoint_region().map_err(|errors| {
        ProductionTransitionError::EndpointInvariant(format!("{label}: {}", errors.join("; ")))
    })?;
    Ok(ProductionTransitionConfigEvidence {
        file,
        config_fingerprint: config
            .fingerprint()
            .map_err(|source| ProductionTransitionError::Config { label, source })?,
        evidence_config_fingerprint: config
            .evidence_fingerprint()
            .map_err(|source| ProductionTransitionError::Config { label, source })?,
        environment: config.venue.environment,
        endpoint_region,
    })
}

#[derive(Debug, Clone, PartialEq, Eq)]
enum ConfigPathSegment {
    Key(String),
    Index(usize),
}

#[derive(Default)]
struct DifferenceCollector {
    allowed_count: u64,
    allowed: Vec<ProductionTransitionChange>,
    allowed_truncated: bool,
    disallowed_count: u64,
    disallowed: Vec<ProductionTransitionChange>,
    disallowed_truncated: bool,
}

impl DifferenceCollector {
    fn record(
        &mut self,
        path: &[ConfigPathSegment],
        demo: Option<&Value>,
        production: Option<&Value>,
    ) {
        let change = ProductionTransitionChange {
            path: json_pointer(path),
            demo: value_evidence(demo),
            production: value_evidence(production),
        };
        if is_allowed_transition_path(path) {
            self.allowed_count = self.allowed_count.saturating_add(1);
            if self.allowed.len() < MAX_REPORTED_TRANSITION_CHANGES {
                self.allowed.push(change);
            } else {
                self.allowed_truncated = true;
            }
        } else {
            self.disallowed_count = self.disallowed_count.saturating_add(1);
            if self.disallowed.len() < MAX_REPORTED_TRANSITION_CHANGES {
                self.disallowed.push(change);
            } else {
                self.disallowed_truncated = true;
            }
        }
    }
}

fn collect_differences(
    demo: Option<&Value>,
    production: Option<&Value>,
    path: &mut Vec<ConfigPathSegment>,
    differences: &mut DifferenceCollector,
) {
    if demo == production {
        return;
    }
    match (demo, production) {
        (Some(Value::Object(demo)), Some(Value::Object(production))) => {
            let keys = demo
                .keys()
                .chain(production.keys())
                .cloned()
                .collect::<BTreeSet<_>>();
            for key in keys {
                path.push(ConfigPathSegment::Key(key.clone()));
                collect_differences(demo.get(&key), production.get(&key), path, differences);
                path.pop();
            }
        }
        (Some(Value::Array(demo)), Some(Value::Array(production))) => {
            for index in 0..demo.len().max(production.len()) {
                path.push(ConfigPathSegment::Index(index));
                collect_differences(demo.get(index), production.get(index), path, differences);
                path.pop();
            }
        }
        _ => differences.record(path, demo, production),
    }
}

fn is_allowed_transition_path(path: &[ConfigPathSegment]) -> bool {
    match path {
        [
            ConfigPathSegment::Key(section),
            ConfigPathSegment::Key(field),
        ] if section == "venue" => {
            matches!(
                field.as_str(),
                "environment" | "rest_url" | "public_ws_url" | "private_ws_url"
            )
        }
        [
            ConfigPathSegment::Key(section),
            ConfigPathSegment::Key(field),
        ] if section == "storage" => field == "path",
        [
            ConfigPathSegment::Key(section),
            ConfigPathSegment::Key(field),
        ] if section == "operator" => {
            matches!(field.as_str(), "socket_path" | "token_env")
        }
        [
            ConfigPathSegment::Key(section),
            ConfigPathSegment::Key(field),
        ] if section == "alerts" => {
            matches!(field.as_str(), "endpoint_env" | "bearer_token_env")
        }
        [
            ConfigPathSegment::Key(section),
            ConfigPathSegment::Index(_),
            ConfigPathSegment::Key(field),
        ] if section == "accounts" => matches!(
            field.as_str(),
            "api_key_env" | "secret_key_env" | "passphrase_env"
        ),
        _ => false,
    }
}

fn json_pointer(path: &[ConfigPathSegment]) -> String {
    let mut pointer = String::new();
    for segment in path {
        pointer.push('/');
        match segment {
            ConfigPathSegment::Key(key) => {
                pointer.push_str(&key.replace('~', "~0").replace('/', "~1"));
            }
            ConfigPathSegment::Index(index) => pointer.push_str(&index.to_string()),
        }
    }
    pointer
}

fn value_evidence(value: Option<&Value>) -> TransitionValueEvidence {
    let Some(value) = value else {
        return TransitionValueEvidence {
            kind: TransitionValueKind::Missing,
            sha256: None,
            display: None,
            entries: None,
        };
    };
    let (kind, display, entries) = match value {
        Value::Null => (TransitionValueKind::Null, Some(Value::Null), None),
        Value::Bool(_) => (TransitionValueKind::Boolean, Some(value.clone()), None),
        Value::Number(_) => (TransitionValueKind::Number, Some(value.clone()), None),
        Value::String(text) if text.len() <= MAX_DISPLAY_STRING_BYTES => {
            (TransitionValueKind::String, Some(value.clone()), None)
        }
        Value::String(_) => (TransitionValueKind::String, None, None),
        Value::Array(values) => (
            TransitionValueKind::Array,
            None,
            Some(values.len().min(u64::MAX as usize) as u64),
        ),
        Value::Object(values) => (
            TransitionValueKind::Object,
            None,
            Some(values.len().min(u64::MAX as usize) as u64),
        ),
    };
    let encoded = serde_json::to_vec(value).expect("serde_json::Value always serializes");
    TransitionValueEvidence {
        kind,
        sha256: Some(sha256_bytes(&encoded)),
        display,
        entries,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn demo_config() -> LiveConfig {
        LiveConfig::from_toml(include_str!("../../../examples/live-okx-demo.toml")).unwrap()
    }

    fn production_config() -> LiveConfig {
        let mut config = demo_config();
        config.venue.environment = TradingEnvironment::Production;
        config.venue.public_ws_url = "wss://ws.okx.com:8443/ws/v5/public".to_string();
        config.venue.private_ws_url = "wss://ws.okx.com:8443/ws/v5/private".to_string();
        config.accounts[0].api_key_env = "REAP_OKX_PROD_API_KEY".to_string();
        config.accounts[0].secret_key_env = "REAP_OKX_PROD_SECRET_KEY".to_string();
        config.accounts[0].passphrase_env = "REAP_OKX_PROD_PASSPHRASE".to_string();
        config.storage.path = PathBuf::from("var/reap/production-events.jsonl");
        config.operator.socket_path = PathBuf::from("var/reap/production-operator.sock");
        config.operator.token_env = "REAP_PRODUCTION_OPERATOR_TOKEN".to_string();
        config.alerts.endpoint_env = "REAP_PRODUCTION_ALERT_WEBHOOK_URL".to_string();
        config
    }

    fn write_config(path: &Path, config: &LiveConfig) {
        std::fs::write(path, toml::to_string_pretty(config).unwrap()).unwrap();
    }

    fn verify(demo: &LiveConfig, production: &LiveConfig) -> ProductionTransitionReport {
        let directory = tempfile::tempdir().unwrap();
        let demo_path = directory.path().join("demo.toml");
        let production_path = directory.path().join("production.toml");
        write_config(&demo_path, demo);
        write_config(&production_path, production);
        verify_production_transition_paths(demo_path, production_path).unwrap()
    }

    #[test]
    fn deployment_only_changes_pass_the_transition_policy() {
        let report = verify(&demo_config(), &production_config());

        assert!(report.acceptance_passed, "{:?}", report.failures);
        assert!(report.environment_pair_valid);
        assert!(report.endpoint_region_consistent);
        assert_eq!(report.demo.endpoint_region, OkxEndpointRegion::Global);
        assert_eq!(report.production.endpoint_region, OkxEndpointRegion::Global);
        assert_eq!(report.disallowed_change_count, 0);
        assert!(report.allowed_change_count >= 10);
        assert!(
            report
                .allowed_changes
                .iter()
                .any(|change| change.path == "/venue/environment")
        );
        assert!(
            report
                .allowed_changes
                .iter()
                .any(|change| change.path == "/accounts/0/api_key_env")
        );
    }

    #[test]
    fn economic_account_and_safety_drift_fail_closed() {
        let demo = demo_config();
        let mut production = production_config();
        production.strategy.instruments[0].hedge_profit_margin += 0.0001;
        production.risk.max_order_notional_usd += 1.0;
        production.runtime.event_channel_capacity += 1;
        production.accounts[0].node_id += 1;
        production.venue.enable_vip_fills_channel = true;
        production.storage.channel_capacity += 1;
        production.host_guard.min_memory_available_bytes += 1;
        production.ensure_valid().unwrap();

        let report = verify(&demo, &production);

        assert!(!report.acceptance_passed);
        assert_eq!(report.disallowed_change_count, 7);
        let paths = report
            .disallowed_changes
            .iter()
            .map(|change| change.path.as_str())
            .collect::<BTreeSet<_>>();
        for expected in [
            "/strategy/instruments/0/hedge_profit_margin",
            "/risk/max_order_notional_usd",
            "/runtime/event_channel_capacity",
            "/accounts/0/node_id",
            "/venue/enable_vip_fills_channel",
            "/storage/channel_capacity",
            "/host_guard/min_memory_available_bytes",
        ] {
            assert!(paths.contains(expected), "missing {expected} in {paths:?}");
        }
    }

    #[test]
    fn region_mismatch_and_loopback_demo_are_not_eligible() {
        let demo = demo_config();
        let mut eea_production = production_config();
        eea_production.venue.rest_url = "https://eea.okx.com".to_string();
        eea_production.venue.public_ws_url = "wss://wseea.okx.com:8443/ws/v5/public".to_string();
        eea_production.venue.private_ws_url = "wss://wseea.okx.com:8443/ws/v5/private".to_string();
        let report = verify(&demo, &eea_production);
        assert!(!report.acceptance_passed);
        assert!(!report.endpoint_region_consistent);
        assert!(matches!(
            report.failures.as_slice(),
            [ProductionTransitionFailure::EndpointRegionMismatch { .. }]
        ));

        let mut loopback_demo = demo;
        loopback_demo.venue.rest_url = "http://127.0.0.1:18080".to_string();
        loopback_demo.venue.public_ws_url = "ws://127.0.0.1:18081/ws/v5/public".to_string();
        loopback_demo.venue.private_ws_url = "ws://127.0.0.1:18082/ws/v5/private".to_string();
        let report = verify(&loopback_demo, &production_config());
        assert!(
            report
                .failures
                .contains(&ProductionTransitionFailure::DemoLoopbackNotEligible)
        );
    }

    #[test]
    fn wrong_environment_and_same_file_fail_explicitly() {
        let demo = demo_config();
        let report = verify(&demo, &demo);
        assert!(!report.acceptance_passed);
        assert!(report.failures.contains(
            &ProductionTransitionFailure::ProductionEnvironmentRequired {
                actual: TradingEnvironment::Demo,
            }
        ));

        let directory = tempfile::tempdir().unwrap();
        let path = directory.path().join("config.toml");
        write_config(&path, &demo);
        assert!(matches!(
            verify_production_transition_paths(&path, &path),
            Err(ProductionTransitionError::PathCollision(_))
        ));
    }

    #[test]
    fn json_pointer_escapes_object_keys_and_large_values_are_digest_only() {
        let path = vec![
            ConfigPathSegment::Key("trade_modes".to_string()),
            ConfigPathSegment::Key("BTC/USDT~test".to_string()),
        ];
        assert_eq!(json_pointer(&path), "/trade_modes/BTC~1USDT~0test");

        let value = Value::String("x".repeat(MAX_DISPLAY_STRING_BYTES + 1));
        let evidence = value_evidence(Some(&value));
        assert_eq!(evidence.kind, TransitionValueKind::String);
        assert!(evidence.display.is_none());
        assert!(evidence.sha256.is_some());
    }
}
