use std::collections::{BTreeMap, BTreeSet};

use anyhow::{Result, bail};
pub use reap_core::BacktestLatencyClass;
use reap_core::Symbol;
use reap_strategy::ChaosConfig;
use serde::{Deserialize, Serialize};

const MAX_BACKTEST_LATENCY_MS: u64 = 3_600_000;
const MAX_LATENCY_PROFILE_RULES: usize = 4_096;
const MAX_LATENCY_SAMPLES_PER_RULE: usize = 65_536;
const MAX_TOTAL_LATENCY_SAMPLES: usize = 1_000_000;

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct BacktestLatencyRule {
    pub class: BacktestLatencyClass,
    /// Omit for a class-wide rule; a symbol rule takes precedence.
    pub symbol: Option<Symbol>,
    /// Uniform empirical samples. The scheduler sorts before deterministic sampling.
    pub samples_ms: Vec<u64>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, Default)]
#[serde(default, deny_unknown_fields)]
pub struct BacktestLatencyProfile {
    /// Must remain equal between baseline and stress scenarios for quantile coupling.
    pub seed: u64,
    pub rules: Vec<BacktestLatencyRule>,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct BacktestLatencyUsage {
    pub class: BacktestLatencyClass,
    pub symbol: Symbol,
    pub samples: u64,
    pub total_latency_ms: u64,
    pub minimum_latency_ms: u64,
    pub maximum_latency_ms: u64,
    pub mean_latency_ms: f64,
}

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
    /// Optional bounded empirical distributions with class/symbol precedence.
    pub latency_profile: BacktestLatencyProfile,
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
            latency_profile: BacktestLatencyProfile::default(),
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
        self.latency_profile.validate()?;
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

    pub fn latency_is_no_less_conservative_than(&self, baseline: &Self) -> bool {
        if self.latency_profile.seed != baseline.latency_profile.seed {
            return false;
        }
        let symbols = self
            .latency_profile
            .rules
            .iter()
            .chain(&baseline.latency_profile.rules)
            .filter_map(|rule| rule.symbol.as_deref())
            .collect::<BTreeSet<_>>();
        for class in BacktestLatencyClass::ALL {
            if !first_order_stochastically_no_less(
                &self.effective_latency_samples(class, None),
                &baseline.effective_latency_samples(class, None),
            ) {
                return false;
            }
            for symbol in &symbols {
                if !first_order_stochastically_no_less(
                    &self.effective_latency_samples(class, Some(symbol)),
                    &baseline.effective_latency_samples(class, Some(symbol)),
                ) {
                    return false;
                }
            }
        }
        true
    }

    fn effective_latency_samples(
        &self,
        class: BacktestLatencyClass,
        symbol: Option<&str>,
    ) -> Vec<u64> {
        let specific = symbol.and_then(|symbol| {
            self.latency_profile
                .rules
                .iter()
                .find(|rule| rule.class == class && rule.symbol.as_deref() == Some(symbol))
        });
        let generic = self
            .latency_profile
            .rules
            .iter()
            .find(|rule| rule.class == class && rule.symbol.is_none());
        let mut samples = specific
            .or(generic)
            .map(|rule| rule.samples_ms.clone())
            .unwrap_or_else(|| vec![self.legacy_latency_ms(class)]);
        samples.sort_unstable();
        samples
    }

    fn legacy_latency_ms(&self, class: BacktestLatencyClass) -> u64 {
        match class {
            BacktestLatencyClass::MarketDepth
            | BacktestLatencyClass::HistoricalTrade
            | BacktestLatencyClass::ReferenceData => self.market_data_latency_ms,
            BacktestLatencyClass::MatchingNew => self.order_entry_latency_ms,
            BacktestLatencyClass::MatchingCancel => self.cancel_latency_ms,
            BacktestLatencyClass::OrderUpdate => self.order_update_latency_ms,
            BacktestLatencyClass::OrderFill => self.fill_account_latency_ms,
        }
    }
}

impl BacktestLatencyProfile {
    pub fn validate(&self) -> Result<()> {
        if self.rules.len() > MAX_LATENCY_PROFILE_RULES {
            bail!(
                "backtest.latency_profile has {} rules, maximum is {MAX_LATENCY_PROFILE_RULES}",
                self.rules.len()
            );
        }
        let mut identities = BTreeSet::new();
        let mut total_samples = 0usize;
        for rule in &self.rules {
            if let Some(symbol) = &rule.symbol
                && (symbol.is_empty()
                    || symbol.len() > 128
                    || symbol.trim() != symbol
                    || !symbol.bytes().all(|byte| byte.is_ascii_graphic()))
            {
                bail!(
                    "backtest.latency_profile symbol {symbol:?} must be 1-128 printable ASCII characters without surrounding whitespace"
                );
            }
            if !identities.insert((rule.class, rule.symbol.as_deref())) {
                bail!(
                    "backtest.latency_profile repeats {:?} rule for {}",
                    rule.class,
                    rule.symbol.as_deref().unwrap_or("all symbols")
                );
            }
            if rule.samples_ms.is_empty() || rule.samples_ms.len() > MAX_LATENCY_SAMPLES_PER_RULE {
                bail!(
                    "backtest.latency_profile {:?}/{} requires 1-{MAX_LATENCY_SAMPLES_PER_RULE} samples",
                    rule.class,
                    rule.symbol.as_deref().unwrap_or("all symbols")
                );
            }
            for sample in &rule.samples_ms {
                if *sample > MAX_BACKTEST_LATENCY_MS {
                    bail!(
                        "backtest.latency_profile {:?}/{} sample {sample} exceeds maximum {MAX_BACKTEST_LATENCY_MS} ms",
                        rule.class,
                        rule.symbol.as_deref().unwrap_or("all symbols")
                    );
                }
            }
            total_samples = total_samples.saturating_add(rule.samples_ms.len());
        }
        if total_samples > MAX_TOTAL_LATENCY_SAMPLES {
            bail!(
                "backtest.latency_profile has {total_samples} total samples, maximum is {MAX_TOTAL_LATENCY_SAMPLES}"
            );
        }
        Ok(())
    }
}

#[derive(Debug, Default)]
struct LatencyUsageAccumulator {
    samples: u64,
    total_latency_ms: u64,
    minimum_latency_ms: Option<u64>,
    maximum_latency_ms: u64,
}

#[derive(Debug)]
pub(crate) struct BacktestLatencySampler {
    seed: u64,
    legacy: BTreeMap<BacktestLatencyClass, u64>,
    rules: BTreeMap<(BacktestLatencyClass, Option<Symbol>), Vec<u64>>,
    ordinals: BTreeMap<(BacktestLatencyClass, Symbol), u64>,
    usage: BTreeMap<(BacktestLatencyClass, Symbol), LatencyUsageAccumulator>,
}

impl BacktestLatencySampler {
    pub(crate) fn new(config: &BacktestExecutionConfig) -> Self {
        let legacy = BacktestLatencyClass::ALL
            .into_iter()
            .map(|class| (class, config.legacy_latency_ms(class)))
            .collect();
        let rules = config
            .latency_profile
            .rules
            .iter()
            .map(|rule| {
                let mut samples = rule.samples_ms.clone();
                samples.sort_unstable();
                ((rule.class, rule.symbol.clone()), samples)
            })
            .collect();
        Self {
            seed: config.latency_profile.seed,
            legacy,
            rules,
            ordinals: BTreeMap::new(),
            usage: BTreeMap::new(),
        }
    }

    pub(crate) fn sample(&mut self, class: BacktestLatencyClass, symbol: &str) -> u64 {
        let key = (class, symbol.to_string());
        let ordinal = self.ordinals.entry(key.clone()).or_default();
        let sample_ordinal = *ordinal;
        *ordinal = ordinal.saturating_add(1);
        let specific = (class, Some(symbol.to_string()));
        let generic = (class, None);
        let latency_ms = self
            .rules
            .get(&specific)
            .or_else(|| self.rules.get(&generic))
            .map(|samples| {
                let index = deterministic_quantile_index(
                    self.seed,
                    class,
                    symbol,
                    sample_ordinal,
                    samples.len(),
                );
                samples[index]
            })
            .unwrap_or_else(|| self.legacy.get(&class).copied().unwrap_or_default());
        let usage = self.usage.entry(key).or_default();
        usage.samples = usage.samples.saturating_add(1);
        usage.total_latency_ms = usage.total_latency_ms.saturating_add(latency_ms);
        usage.minimum_latency_ms = Some(
            usage
                .minimum_latency_ms
                .map_or(latency_ms, |minimum| minimum.min(latency_ms)),
        );
        usage.maximum_latency_ms = usage.maximum_latency_ms.max(latency_ms);
        latency_ms
    }

    pub(crate) fn usage(&self) -> Vec<BacktestLatencyUsage> {
        self.usage
            .iter()
            .map(|((class, symbol), usage)| BacktestLatencyUsage {
                class: *class,
                symbol: symbol.clone(),
                samples: usage.samples,
                total_latency_ms: usage.total_latency_ms,
                minimum_latency_ms: usage.minimum_latency_ms.unwrap_or_default(),
                maximum_latency_ms: usage.maximum_latency_ms,
                mean_latency_ms: if usage.samples == 0 {
                    0.0
                } else {
                    usage.total_latency_ms as f64 / usage.samples as f64
                },
            })
            .collect()
    }
}

fn deterministic_quantile_index(
    seed: u64,
    class: BacktestLatencyClass,
    symbol: &str,
    ordinal: u64,
    sample_count: usize,
) -> usize {
    if sample_count <= 1 {
        return 0;
    }
    let mut hash = 0xcbf2_9ce4_8422_2325_u64;
    for byte in seed
        .to_le_bytes()
        .into_iter()
        .chain([class.stable_tag()])
        .chain(symbol.bytes())
        .chain(ordinal.to_le_bytes())
    {
        hash ^= u64::from(byte);
        hash = hash.wrapping_mul(0x0000_0100_0000_01b3);
    }
    let random = splitmix64(hash);
    ((u128::from(random) * sample_count as u128) >> 64) as usize
}

fn splitmix64(mut value: u64) -> u64 {
    value = value.wrapping_add(0x9e37_79b9_7f4a_7c15);
    value = (value ^ (value >> 30)).wrapping_mul(0xbf58_476d_1ce4_e5b9);
    value = (value ^ (value >> 27)).wrapping_mul(0x94d0_49bb_1331_11eb);
    value ^ (value >> 31)
}

fn first_order_stochastically_no_less(stress: &[u64], baseline: &[u64]) -> bool {
    if stress.is_empty() || baseline.is_empty() {
        return false;
    }
    let mut values = stress
        .iter()
        .chain(baseline)
        .copied()
        .collect::<BTreeSet<_>>();
    values.insert(u64::MAX);
    values.into_iter().all(|value| {
        let stress_count = stress.partition_point(|sample| *sample <= value) as u64;
        let baseline_count = baseline.partition_point(|sample| *sample <= value) as u64;
        stress_count.saturating_mul(baseline.len() as u64)
            <= baseline_count.saturating_mul(stress.len() as u64)
    })
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
    fn strategy_toml_accepts_bounded_class_and_symbol_latency_samples() {
        let config: BacktestConfig = toml::from_str(
            r#"
                strategy_name = "test"
                underlying = "BTC"
                ref_symbol = "BTC-USDT"

                [backtest.latency_profile]
                seed = 42

                [[backtest.latency_profile.rules]]
                class = "market_depth"
                samples_ms = [3, 1, 2]

                [[backtest.latency_profile.rules]]
                class = "market_depth"
                symbol = "BTC-USDT"
                samples_ms = [1, 0]
            "#,
        )
        .unwrap();

        assert_eq!(config.backtest.latency_profile.seed, 42);
        assert_eq!(config.backtest.latency_profile.rules.len(), 2);
        assert_eq!(
            config.backtest.latency_profile.rules[1].symbol.as_deref(),
            Some("BTC-USDT")
        );
        config.backtest.validate().unwrap();
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
                latency_profile: BacktestLatencyProfile::default(),
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
    fn latency_profile_rejects_duplicate_empty_and_unbounded_rules() {
        let duplicate = BacktestLatencyRule {
            class: BacktestLatencyClass::MarketDepth,
            symbol: Some("BTC-USDT".to_string()),
            samples_ms: vec![1],
        };
        let configs = [
            BacktestExecutionConfig {
                latency_profile: BacktestLatencyProfile {
                    seed: 0,
                    rules: vec![duplicate.clone(), duplicate],
                },
                ..BacktestExecutionConfig::default()
            },
            BacktestExecutionConfig {
                latency_profile: BacktestLatencyProfile {
                    seed: 0,
                    rules: vec![BacktestLatencyRule {
                        class: BacktestLatencyClass::OrderUpdate,
                        symbol: None,
                        samples_ms: Vec::new(),
                    }],
                },
                ..BacktestExecutionConfig::default()
            },
            BacktestExecutionConfig {
                latency_profile: BacktestLatencyProfile {
                    seed: 0,
                    rules: vec![BacktestLatencyRule {
                        class: BacktestLatencyClass::MatchingNew,
                        symbol: None,
                        samples_ms: vec![MAX_BACKTEST_LATENCY_MS + 1],
                    }],
                },
                ..BacktestExecutionConfig::default()
            },
        ];

        for config in configs {
            assert!(config.validate().is_err());
        }

        let malformed_stress = BacktestExecutionConfig {
            latency_profile: BacktestLatencyProfile {
                seed: 0,
                rules: vec![BacktestLatencyRule {
                    class: BacktestLatencyClass::MarketDepth,
                    symbol: None,
                    samples_ms: Vec::new(),
                }],
            },
            ..BacktestExecutionConfig::default()
        };
        assert!(
            !malformed_stress
                .latency_is_no_less_conservative_than(&BacktestExecutionConfig::default())
        );
    }

    #[test]
    fn latency_sampler_is_deterministic_and_prefers_symbol_rules() {
        let config = BacktestExecutionConfig {
            market_data_latency_ms: 9,
            latency_profile: BacktestLatencyProfile {
                seed: 7,
                rules: vec![
                    BacktestLatencyRule {
                        class: BacktestLatencyClass::MarketDepth,
                        symbol: None,
                        samples_ms: vec![3],
                    },
                    BacktestLatencyRule {
                        class: BacktestLatencyClass::MarketDepth,
                        symbol: Some("BTC-USDT".to_string()),
                        samples_ms: vec![2, 1],
                    },
                ],
            },
            ..BacktestExecutionConfig::default()
        };
        let mut first = BacktestLatencySampler::new(&config);
        let mut second = BacktestLatencySampler::new(&config);

        let first_sequence = (0..32)
            .map(|_| first.sample(BacktestLatencyClass::MarketDepth, "BTC-USDT"))
            .collect::<Vec<_>>();
        let second_sequence = (0..32)
            .map(|_| second.sample(BacktestLatencyClass::MarketDepth, "BTC-USDT"))
            .collect::<Vec<_>>();

        assert_eq!(first_sequence, second_sequence);
        assert!(first_sequence.iter().all(|sample| matches!(sample, 1 | 2)));
        assert_eq!(
            first.sample(BacktestLatencyClass::MarketDepth, "ETH-USDT"),
            3
        );
        assert_eq!(
            first.sample(BacktestLatencyClass::HistoricalTrade, "BTC-USDT"),
            9
        );
        let usage = first.usage();
        assert_eq!(
            usage
                .iter()
                .find(|usage| {
                    usage.class == BacktestLatencyClass::MarketDepth && usage.symbol == "BTC-USDT"
                })
                .unwrap()
                .samples,
            32
        );
    }

    #[test]
    fn stress_latency_distribution_requires_same_seed_and_stochastic_dominance() {
        let profile = |seed, samples_ms| BacktestLatencyProfile {
            seed,
            rules: vec![BacktestLatencyRule {
                class: BacktestLatencyClass::MarketDepth,
                symbol: Some("BTC-USDT".to_string()),
                samples_ms,
            }],
        };
        let baseline = BacktestExecutionConfig {
            latency_profile: profile(11, vec![1, 3, 5]),
            ..BacktestExecutionConfig::default()
        };
        let conservative = BacktestExecutionConfig {
            latency_profile: profile(11, vec![2, 4, 6]),
            ..BacktestExecutionConfig::default()
        };
        let optimistic_tail = BacktestExecutionConfig {
            latency_profile: profile(11, vec![0, 4, 6]),
            ..BacktestExecutionConfig::default()
        };
        let different_seed = BacktestExecutionConfig {
            latency_profile: profile(12, vec![2, 4, 6]),
            ..BacktestExecutionConfig::default()
        };

        assert!(conservative.latency_is_no_less_conservative_than(&baseline));
        assert!(!optimistic_tail.latency_is_no_less_conservative_than(&baseline));
        assert!(!different_seed.latency_is_no_less_conservative_than(&baseline));

        let mut baseline_sampler = BacktestLatencySampler::new(&baseline);
        let mut stress_sampler = BacktestLatencySampler::new(&conservative);
        for _ in 0..1_000 {
            let baseline_sample =
                baseline_sampler.sample(BacktestLatencyClass::MarketDepth, "BTC-USDT");
            let stress_sample =
                stress_sampler.sample(BacktestLatencyClass::MarketDepth, "BTC-USDT");
            assert!(stress_sample >= baseline_sample);
        }
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
