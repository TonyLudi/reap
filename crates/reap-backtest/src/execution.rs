use anyhow::{Result, bail};
use reap_strategy::ChaosConfig;
use serde::{Deserialize, Serialize};

const MAX_BACKTEST_LATENCY_MS: u64 = 3_600_000;

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(default, deny_unknown_fields)]
pub struct BacktestExecutionConfig {
    /// True only when every execution assumption came from representative evidence.
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
    /// Extra relative price cross required for fills from displayed depth.
    pub depth_fill_conservative_threshold: f64,
    /// Multiplier applied to displayed quantity ahead of a newly resting order.
    pub queue_ahead_multiplier: f64,
    /// Fraction of historical trade quantity eligible to consume queue and fill orders.
    pub historical_trade_fill_fraction: f64,
    /// Fraction of each displayed level available to simulated depth matching.
    pub displayed_depth_fill_fraction: f64,
}

impl Default for BacktestExecutionConfig {
    fn default() -> Self {
        Self {
            calibrated: false,
            market_data_latency_ms: 0,
            order_entry_latency_ms: 0,
            cancel_latency_ms: 0,
            order_update_latency_ms: 0,
            fill_account_latency_ms: 0,
            depth_fill_conservative_threshold: 0.0,
            queue_ahead_multiplier: 1.0,
            historical_trade_fill_fraction: 1.0,
            displayed_depth_fill_fraction: 1.0,
        }
    }
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
        if !self.depth_fill_conservative_threshold.is_finite()
            || !(0.0..=0.1).contains(&self.depth_fill_conservative_threshold)
        {
            bail!("backtest.depth_fill_conservative_threshold must be finite and within [0, 0.1]");
        }
        if !self.queue_ahead_multiplier.is_finite()
            || !(1.0..=100.0).contains(&self.queue_ahead_multiplier)
        {
            bail!("backtest.queue_ahead_multiplier must be finite and within [1, 100]");
        }
        for (name, value) in [
            (
                "historical_trade_fill_fraction",
                self.historical_trade_fill_fraction,
            ),
            (
                "displayed_depth_fill_fraction",
                self.displayed_depth_fill_fraction,
            ),
        ] {
            if !value.is_finite() || !(0.0..=1.0).contains(&value) {
                bail!("backtest.{name} must be finite and within [0, 1]");
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
                depth_fill_conservative_threshold = 0.0001
                queue_ahead_multiplier = 2.0
                historical_trade_fill_fraction = 0.25
                displayed_depth_fill_fraction = 0.5
            "#,
        )
        .unwrap();

        assert_eq!(config.strategy.ref_symbol, "BTC-USDT");
        assert_eq!(config.backtest.order_entry_latency_ms, 7);
        assert_eq!(config.backtest.cancel_latency_ms, 11);
        assert_eq!(config.backtest.depth_fill_conservative_threshold, 0.0001);
        assert_eq!(config.backtest.queue_ahead_multiplier, 2.0);
        assert_eq!(config.backtest.historical_trade_fill_fraction, 0.25);
        assert_eq!(config.backtest.displayed_depth_fill_fraction, 0.5);
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
                depth_fill_conservative_threshold: 0.0,
                queue_ahead_multiplier: 1.0,
                historical_trade_fill_fraction: 1.0,
                displayed_depth_fill_fraction: 1.0,
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

    #[test]
    fn conservative_depth_threshold_is_bounded_and_finite() {
        for value in [-0.0001, 0.1001, f64::NAN, f64::INFINITY] {
            let config = BacktestExecutionConfig {
                depth_fill_conservative_threshold: value,
                ..BacktestExecutionConfig::default()
            };

            assert!(config.validate().is_err(), "accepted {value}");
        }
        assert!(
            BacktestExecutionConfig {
                depth_fill_conservative_threshold: 0.0001,
                ..BacktestExecutionConfig::default()
            }
            .validate()
            .is_ok()
        );
    }

    #[test]
    fn capacity_assumptions_are_conservative_and_bounded() {
        for config in [
            BacktestExecutionConfig {
                queue_ahead_multiplier: 0.99,
                ..BacktestExecutionConfig::default()
            },
            BacktestExecutionConfig {
                historical_trade_fill_fraction: 1.01,
                ..BacktestExecutionConfig::default()
            },
            BacktestExecutionConfig {
                displayed_depth_fill_fraction: -0.01,
                ..BacktestExecutionConfig::default()
            },
        ] {
            assert!(config.validate().is_err());
        }

        assert!(
            BacktestExecutionConfig {
                queue_ahead_multiplier: 2.0,
                historical_trade_fill_fraction: 0.25,
                displayed_depth_fill_fraction: 0.5,
                ..BacktestExecutionConfig::default()
            }
            .validate()
            .is_ok()
        );
    }
}
