use std::collections::BTreeMap;

use reap_core::{BacktestLatencyClass, Symbol};
use serde::{Deserialize, Serialize};

pub const LIVE_LATENCY_EVIDENCE_SCHEMA_VERSION: u32 = 1;
pub const LIVE_LATENCY_RESERVOIR_CAPACITY: usize = 8_192;
pub const MAX_LIVE_LATENCY_SERIES: usize = 4_096;
pub const MAX_LIVE_LATENCY_US: u64 = 3_600_000_000;

#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum LiveLatencySemantics {
    /// Local websocket receive to entry into the single-owner strategy coordinator.
    HostReceiveToStrategyVisibility,
    /// Exchange event timestamp to entry into the strategy coordinator.
    ExchangeTimestampToStrategyVisibility,
    /// Strategy dispatch through local queue, pacing, REST, and successful acknowledgement.
    StrategyDispatchToRestAckUpperBound,
    /// Canonical fill visibility to the covering account/position update visibility.
    FillToAccountStateVisibility,
}

impl LiveLatencySemantics {
    const fn stable_tag(self) -> u8 {
        match self {
            Self::HostReceiveToStrategyVisibility => 1,
            Self::ExchangeTimestampToStrategyVisibility => 2,
            Self::StrategyDispatchToRestAckUpperBound => 3,
            Self::FillToAccountStateVisibility => 4,
        }
    }

    pub const fn is_matching_upper_bound(self) -> bool {
        matches!(self, Self::StrategyDispatchToRestAckUpperBound)
    }

    pub const fn depends_on_exchange_clock(self) -> bool {
        matches!(self, Self::ExchangeTimestampToStrategyVisibility)
    }
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct LiveLatencySeries {
    pub class: BacktestLatencyClass,
    pub symbol: Symbol,
    pub semantics: LiveLatencySemantics,
    /// Every duration presented to the collector, including rejected clock samples.
    pub observations: u64,
    pub valid_observations: u64,
    /// Exchange operations that failed and therefore did not produce a latency sample.
    pub operation_failures: u64,
    pub negative_clock_observations: u64,
    pub above_limit_observations: u64,
    pub total_latency_us: u64,
    pub minimum_latency_us: Option<u64>,
    pub maximum_latency_us: Option<u64>,
    pub mean_latency_us: Option<f64>,
    /// Deterministic uniform reservoir, sorted only when the report is built.
    pub retained_samples_us: Vec<u64>,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct LiveLatencyEvidence {
    pub schema_version: u32,
    pub reservoir_capacity_per_series: usize,
    pub maximum_latency_us: u64,
    /// Samples lost to bounded collection or censored by authoritative recovery.
    pub dropped_observations: u64,
    pub series: Vec<LiveLatencySeries>,
}

impl Default for LiveLatencyEvidence {
    fn default() -> Self {
        Self {
            schema_version: LIVE_LATENCY_EVIDENCE_SCHEMA_VERSION,
            reservoir_capacity_per_series: LIVE_LATENCY_RESERVOIR_CAPACITY,
            maximum_latency_us: MAX_LIVE_LATENCY_US,
            dropped_observations: 0,
            series: Vec::new(),
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord)]
struct SeriesKey {
    class: BacktestLatencyClass,
    symbol: Symbol,
    semantics: LiveLatencySemantics,
}

#[derive(Debug, Default)]
struct SeriesAccumulator {
    observations: u64,
    valid_observations: u64,
    operation_failures: u64,
    negative_clock_observations: u64,
    above_limit_observations: u64,
    total_latency_us: u64,
    minimum_latency_us: Option<u64>,
    maximum_latency_us: Option<u64>,
    retained_samples_us: Vec<u64>,
}

#[derive(Debug, Default)]
pub(crate) struct LiveLatencyCollector {
    series: BTreeMap<SeriesKey, SeriesAccumulator>,
    dropped_observations: u64,
}

impl LiveLatencyCollector {
    pub fn observe_ns(
        &mut self,
        class: BacktestLatencyClass,
        symbol: &str,
        semantics: LiveLatencySemantics,
        started_ns: u64,
        visible_ns: u64,
    ) {
        if visible_ns < started_ns {
            self.invalid_clock(class, symbol, semantics);
            return;
        }
        self.observe_us(
            class,
            symbol,
            semantics,
            visible_ns.saturating_sub(started_ns).saturating_add(999) / 1_000,
        );
    }

    pub fn observe_exchange_ms(
        &mut self,
        class: BacktestLatencyClass,
        symbol: &str,
        exchange_ms: u64,
        visible_ns: u64,
    ) {
        let semantics = LiveLatencySemantics::ExchangeTimestampToStrategyVisibility;
        let exchange_ns = exchange_ms.saturating_mul(1_000_000);
        if exchange_ms == 0 || visible_ns < exchange_ns {
            self.invalid_clock(class, symbol, semantics);
            return;
        }
        self.observe_us(
            class,
            symbol,
            semantics,
            visible_ns.saturating_sub(exchange_ns).saturating_add(999) / 1_000,
        );
    }

    pub fn observe_us(
        &mut self,
        class: BacktestLatencyClass,
        symbol: &str,
        semantics: LiveLatencySemantics,
        latency_us: u64,
    ) {
        let Some((key, accumulator)) = self.series_mut(class, symbol, semantics) else {
            self.observe_dropped_observation();
            return;
        };
        accumulator.observations = accumulator.observations.saturating_add(1);
        if latency_us > MAX_LIVE_LATENCY_US {
            accumulator.above_limit_observations =
                accumulator.above_limit_observations.saturating_add(1);
            return;
        }

        accumulator.valid_observations = accumulator.valid_observations.saturating_add(1);
        accumulator.total_latency_us = accumulator.total_latency_us.saturating_add(latency_us);
        accumulator.minimum_latency_us = Some(
            accumulator
                .minimum_latency_us
                .map_or(latency_us, |minimum| minimum.min(latency_us)),
        );
        accumulator.maximum_latency_us = Some(
            accumulator
                .maximum_latency_us
                .map_or(latency_us, |maximum| maximum.max(latency_us)),
        );
        retain_uniform_sample(key, accumulator, latency_us);
    }

    pub fn observe_operation_failure(
        &mut self,
        class: BacktestLatencyClass,
        symbol: &str,
        semantics: LiveLatencySemantics,
    ) {
        let Some((_, accumulator)) = self.series_mut(class, symbol, semantics) else {
            self.observe_dropped_observation();
            return;
        };
        accumulator.operation_failures = accumulator.operation_failures.saturating_add(1);
    }

    pub fn observe_dropped_observation(&mut self) {
        self.observe_dropped_observations(1);
    }

    pub fn observe_dropped_observations(&mut self, count: u64) {
        self.dropped_observations = self.dropped_observations.saturating_add(count);
    }

    pub fn report(&self) -> LiveLatencyEvidence {
        let series = self
            .series
            .iter()
            .map(|(key, accumulator)| {
                let mut retained_samples_us = accumulator.retained_samples_us.clone();
                retained_samples_us.sort_unstable();
                LiveLatencySeries {
                    class: key.class,
                    symbol: key.symbol.clone(),
                    semantics: key.semantics,
                    observations: accumulator.observations,
                    valid_observations: accumulator.valid_observations,
                    operation_failures: accumulator.operation_failures,
                    negative_clock_observations: accumulator.negative_clock_observations,
                    above_limit_observations: accumulator.above_limit_observations,
                    total_latency_us: accumulator.total_latency_us,
                    minimum_latency_us: accumulator.minimum_latency_us,
                    maximum_latency_us: accumulator.maximum_latency_us,
                    mean_latency_us: (accumulator.valid_observations > 0).then(|| {
                        accumulator.total_latency_us as f64 / accumulator.valid_observations as f64
                    }),
                    retained_samples_us,
                }
            })
            .collect();
        LiveLatencyEvidence {
            dropped_observations: self.dropped_observations,
            series,
            ..LiveLatencyEvidence::default()
        }
    }

    fn invalid_clock(
        &mut self,
        class: BacktestLatencyClass,
        symbol: &str,
        semantics: LiveLatencySemantics,
    ) {
        let Some((_, accumulator)) = self.series_mut(class, symbol, semantics) else {
            self.observe_dropped_observation();
            return;
        };
        accumulator.observations = accumulator.observations.saturating_add(1);
        accumulator.negative_clock_observations =
            accumulator.negative_clock_observations.saturating_add(1);
    }

    fn series_mut(
        &mut self,
        class: BacktestLatencyClass,
        symbol: &str,
        semantics: LiveLatencySemantics,
    ) -> Option<(SeriesKey, &mut SeriesAccumulator)> {
        let key = SeriesKey {
            class,
            symbol: symbol.to_string(),
            semantics,
        };
        if !self.series.contains_key(&key) && self.series.len() >= MAX_LIVE_LATENCY_SERIES {
            return None;
        }
        let accumulator = self.series.entry(key.clone()).or_default();
        Some((key, accumulator))
    }
}

fn retain_uniform_sample(key: SeriesKey, accumulator: &mut SeriesAccumulator, latency_us: u64) {
    if accumulator.retained_samples_us.len() < LIVE_LATENCY_RESERVOIR_CAPACITY {
        accumulator.retained_samples_us.push(latency_us);
        return;
    }
    let ordinal = accumulator.valid_observations;
    let replacement = stable_sample_hash(&key, ordinal) % ordinal;
    if replacement < LIVE_LATENCY_RESERVOIR_CAPACITY as u64 {
        accumulator.retained_samples_us[replacement as usize] = latency_us;
    }
}

fn stable_sample_hash(key: &SeriesKey, ordinal: u64) -> u64 {
    let mut hash = 0xcbf2_9ce4_8422_2325_u64;
    for byte in [key.class.stable_tag(), key.semantics.stable_tag()]
        .into_iter()
        .chain(key.symbol.bytes())
        .chain(ordinal.to_le_bytes())
    {
        hash ^= u64::from(byte);
        hash = hash.wrapping_mul(0x0000_0100_0000_01b3);
    }
    splitmix64(hash)
}

fn splitmix64(mut value: u64) -> u64 {
    value = value.wrapping_add(0x9e37_79b9_7f4a_7c15);
    value = (value ^ (value >> 30)).wrapping_mul(0xbf58_476d_1ce4_e5b9);
    value = (value ^ (value >> 27)).wrapping_mul(0x94d0_49bb_1331_11eb);
    value ^ (value >> 31)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn bounds_and_sorts_deterministic_reservoir_without_losing_population_stats() {
        let collect = || {
            let mut collector = LiveLatencyCollector::default();
            for latency in 0..10_000_u64 {
                collector.observe_us(
                    BacktestLatencyClass::MarketDepth,
                    "BTC-USDT",
                    LiveLatencySemantics::HostReceiveToStrategyVisibility,
                    latency,
                );
            }
            collector.report()
        };

        let first = collect();
        let second = collect();
        assert_eq!(first, second);
        let series = &first.series[0];
        assert_eq!(series.observations, 10_000);
        assert_eq!(series.valid_observations, 10_000);
        assert_eq!(series.minimum_latency_us, Some(0));
        assert_eq!(series.maximum_latency_us, Some(9_999));
        assert_eq!(
            series.retained_samples_us.len(),
            LIVE_LATENCY_RESERVOIR_CAPACITY
        );
        assert!(
            series
                .retained_samples_us
                .windows(2)
                .all(|pair| pair[0] <= pair[1])
        );
    }

    #[test]
    fn rejects_negative_cross_clock_and_above_limit_samples_explicitly() {
        let mut collector = LiveLatencyCollector::default();
        collector.observe_exchange_ms(
            BacktestLatencyClass::OrderUpdate,
            "BTC-USDT",
            200,
            199_000_000,
        );
        collector.observe_us(
            BacktestLatencyClass::OrderUpdate,
            "BTC-USDT",
            LiveLatencySemantics::ExchangeTimestampToStrategyVisibility,
            MAX_LIVE_LATENCY_US + 1,
        );
        collector.observe_operation_failure(
            BacktestLatencyClass::MatchingNew,
            "BTC-USDT",
            LiveLatencySemantics::StrategyDispatchToRestAckUpperBound,
        );
        collector.observe_dropped_observation();

        let report = collector.report();
        let order = &report.series[1];
        assert_eq!(order.observations, 2);
        assert_eq!(order.valid_observations, 0);
        assert_eq!(order.negative_clock_observations, 1);
        assert_eq!(order.above_limit_observations, 1);
        assert!(order.retained_samples_us.is_empty());
        assert_eq!(report.series[0].operation_failures, 1);
        assert_eq!(report.dropped_observations, 1);
    }
}
