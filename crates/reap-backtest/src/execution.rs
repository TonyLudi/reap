use anyhow::{Result, bail};
use reap_strategy::ChaosConfig;
use serde::{Deserialize, Serialize};

const MAX_BACKTEST_LATENCY_MS: u64 = 3_600_000;

#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(default, deny_unknown_fields)]
pub struct BacktestExecutionConfig {
    /// True only when every latency below came from a representative measured run.
    pub calibrated: bool,
    /// Additional feed processing and strategy visibility delay.
    pub market_data_latency_ms: u64,
    /// Strategy intent to exchange matching eligibility (Java `MatchingNew`).
    pub order_entry_latency_ms: u64,
    /// Cancel intent to exchange cancellation eligibility (Java `MatchingCancel`).
    pub cancel_latency_ms: u64,
    /// Exchange lifecycle transition to strategy visibility (Java `OrderUpdate`).
    pub order_update_latency_ms: u64,
    /// Exchange fill to strategy account/position visibility (Java `OrderFill`).
    pub fill_account_latency_ms: u64,
}

impl BacktestExecutionConfig {
    pub fn validate(&self) -> Result<()> {
        for (name, value) in [
            ("market_data_latency_ms", self.market_data_latency_ms),
            ("order_entry_latency_ms", self.order_entry_latency_ms),
            ("cancel_latency_ms", self.cancel_latency_ms),
            ("order_update_latency_ms", self.order_update_latency_ms),
            ("fill_account_latency_ms", self.fill_account_latency_ms),
        ] {
            if value > MAX_BACKTEST_LATENCY_MS {
                bail!("backtest.{name}={value} exceeds maximum {MAX_BACKTEST_LATENCY_MS} ms");
            }
        }
        Ok(())
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct BacktestConfig {
    #[serde(flatten)]
    pub strategy: ChaosConfig,
    #[serde(default)]
    pub backtest: BacktestExecutionConfig,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum BacktestTimeBasis {
    EventTimestampMs,
    CaptureReceiveTimestampNs,
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn strategy_toml_accepts_optional_backtest_section() {
        let config: BacktestConfig = toml::from_str(
            r#"
                strategy_name = "test"
                underlying = "BTC"
                ref_symbol = "BTC-USDT"

                [backtest]
                calibrated = false
                order_entry_latency_ms = 7
                cancel_latency_ms = 11
            "#,
        )
        .unwrap();

        assert_eq!(config.strategy.ref_symbol, "BTC-USDT");
        assert_eq!(config.backtest.order_entry_latency_ms, 7);
        assert_eq!(config.backtest.cancel_latency_ms, 11);
        assert!(!config.backtest.calibrated);
    }

    #[test]
    fn execution_defaults_preserve_zero_latency_behavior() {
        assert_eq!(
            BacktestExecutionConfig::default(),
            BacktestExecutionConfig {
                calibrated: false,
                market_data_latency_ms: 0,
                order_entry_latency_ms: 0,
                cancel_latency_ms: 0,
                order_update_latency_ms: 0,
                fill_account_latency_ms: 0,
            }
        );
    }

    #[test]
    fn backtest_section_rejects_unknown_latency_fields() {
        let error = toml::from_str::<BacktestConfig>(
            r#"
                ref_symbol = "BTC-USDT"

                [backtest]
                order_entery_latency_ms = 7
            "#,
        )
        .unwrap_err();

        assert!(error.to_string().contains("order_entery_latency_ms"));
    }
}
