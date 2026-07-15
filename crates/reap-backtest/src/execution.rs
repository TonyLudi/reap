use std::collections::{BTreeMap, BTreeSet, HashMap};

use anyhow::{Result, bail};
pub use reap_core::BacktestLatencyClass;
use reap_core::{AccountUpdate, Balance, MarginSnapshot, Position, PositionMarginMode, Symbol};
use reap_strategy::ChaosConfig;
use serde::{Deserialize, Serialize};

const MAX_BACKTEST_LATENCY_MS: u64 = 3_600_000;
const MAX_LATENCY_PROFILE_RULES: usize = 4_096;
const MAX_LATENCY_SAMPLES_PER_RULE: usize = 65_536;
const MAX_TOTAL_LATENCY_SAMPLES: usize = 1_000_000;
const MAX_CURRENCY_RATE_ROUTES: usize = 256;
const MAX_INITIAL_BALANCES: usize = 256;
const MAX_INITIAL_POSITIONS: usize = 4_096;
const MAX_INITIAL_VALUE: f64 = 1.0e18;
const DEFAULT_CURRENCY_RATE_MAX_AGE_MS: u64 = 75_000;

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, Default)]
#[serde(deny_unknown_fields)]
pub struct BacktestInitialBalanceConfig {
    /// Exchange account currency, for example `BTC` or `USDT`.
    pub currency: String,
    /// Authoritative opening cash balance. Borrowing is not modeled, so this is non-negative.
    pub total: f64,
    /// Exchange available balance. Omission preserves the legacy `total` fallback.
    #[serde(default)]
    pub available: Option<f64>,
    /// Exchange currency equity. Omission preserves the legacy `total` fallback.
    #[serde(default)]
    pub equity: Option<f64>,
    /// Opening liability. The cash-only backtest model requires zero.
    #[serde(default)]
    pub liability: Option<f64>,
    /// Exchange-reported maximum loan, retained for Java strategy parity.
    #[serde(default)]
    pub max_loan: Option<f64>,
    #[serde(default)]
    pub forced_repayment_indicator: Option<u8>,
    /// Spot instrument used to value this currency as inventory instead of generic cash.
    #[serde(default)]
    pub valuation_symbol: Option<Symbol>,
}

impl BacktestInitialBalanceConfig {
    pub fn available(&self) -> f64 {
        self.available.unwrap_or(self.total)
    }

    pub fn equity(&self) -> f64 {
        self.equity.unwrap_or(self.total)
    }

    pub fn liability(&self) -> f64 {
        self.liability.unwrap_or(0.0)
    }

    pub fn max_loan(&self) -> f64 {
        self.max_loan.unwrap_or(0.0)
    }
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct BacktestInitialPositionConfig {
    pub symbol: Symbol,
    pub qty: f64,
    pub avg_price: f64,
    #[serde(default)]
    pub margin_mode: Option<PositionMarginMode>,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, Default)]
#[serde(default, deny_unknown_fields)]
pub struct BacktestInitialMarginConfig {
    pub ratio: Option<f64>,
    pub exchange_ratio: Option<f64>,
    pub adjusted_equity_usd: Option<f64>,
    pub notional_usd: Option<f64>,
}

impl BacktestInitialMarginConfig {
    pub fn is_empty(&self) -> bool {
        self.ratio.is_none()
            && self.exchange_ratio.is_none()
            && self.adjusted_equity_usd.is_none()
            && self.notional_usd.is_none()
    }
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, Default)]
#[serde(default, deny_unknown_fields)]
pub struct BacktestInitialPortfolioConfig {
    /// One account snapshot per runner. Omit only when strategy risk groups omit account IDs.
    pub account_id: Option<String>,
    pub balances: Vec<BacktestInitialBalanceConfig>,
    /// Derivative positions only. Spot inventory is supplied through `balances`.
    pub positions: Vec<BacktestInitialPositionConfig>,
    pub margin: BacktestInitialMarginConfig,
}

impl BacktestInitialPortfolioConfig {
    pub fn is_empty(&self) -> bool {
        self.balances.is_empty() && self.positions.is_empty() && self.margin.is_empty()
    }

    pub fn has_positive_balance(&self) -> bool {
        self.balances.iter().any(|balance| balance.total > 0.0)
    }

    pub fn validate(
        &self,
        strategy: &ChaosConfig,
        execution: &BacktestExecutionConfig,
    ) -> Result<()> {
        if self.is_empty() {
            if self.account_id.is_some() {
                bail!("initial_portfolio.account_id requires balances or positions");
            }
            return Ok(());
        }
        validate_optional_account_id(self.account_id.as_deref())?;
        if self.balances.len() > MAX_INITIAL_BALANCES {
            bail!(
                "initial_portfolio.balances has {} rows, maximum is {MAX_INITIAL_BALANCES}",
                self.balances.len()
            );
        }
        if self.positions.len() > MAX_INITIAL_POSITIONS {
            bail!(
                "initial_portfolio.positions has {} rows, maximum is {MAX_INITIAL_POSITIONS}",
                self.positions.len()
            );
        }

        let configured_account_ids = strategy
            .risk_groups
            .iter()
            .filter_map(|group| group.account_id.as_deref())
            .collect::<BTreeSet<_>>();
        if configured_account_ids.len() > 1 {
            bail!(
                "initial_portfolio supports one account, but strategy risk groups reference multiple account IDs"
            );
        }
        if let Some(expected) = configured_account_ids.first()
            && self.account_id.as_deref() != Some(*expected)
        {
            bail!(
                "initial_portfolio.account_id must match strategy risk-group account ID {expected:?}"
            );
        }

        let instruments = strategy
            .instruments
            .iter()
            .map(|instrument| (instrument.symbol.as_str(), instrument))
            .collect::<HashMap<_, _>>();
        let configured_rates = execution
            .currency_rates
            .iter()
            .map(|route| route.currency.as_str())
            .collect::<BTreeSet<_>>();
        let mut required_balances = BTreeSet::new();
        for instrument in &strategy.instruments {
            if instrument.kind.is_spot() {
                if instrument.base_currency.is_empty() || instrument.quote_currency.is_empty() {
                    bail!(
                        "initial_portfolio requires non-empty base_currency and quote_currency for spot instrument {}",
                        instrument.symbol
                    );
                }
                required_balances.insert(instrument.base_currency.as_str());
                required_balances.insert(instrument.quote_currency.as_str());
            } else {
                if instrument.settle_currency.is_empty() {
                    bail!(
                        "initial_portfolio requires non-empty settle_currency for derivative instrument {}",
                        instrument.symbol
                    );
                }
                required_balances.insert(instrument.settle_currency.as_str());
            }
        }
        let mut balances = BTreeSet::new();
        for balance in &self.balances {
            validate_currency_name("initial_portfolio.balances currency", &balance.currency)?;
            if !required_balances.contains(balance.currency.as_str()) {
                bail!(
                    "initial_portfolio balance {} is outside the configured instrument account universe",
                    balance.currency
                );
            }
            if !balance.total.is_finite()
                || balance.total < 0.0
                || balance.total > MAX_INITIAL_VALUE
            {
                bail!(
                    "initial_portfolio balance {} total must be finite and within [0, {MAX_INITIAL_VALUE}]",
                    balance.currency
                );
            }
            for (field, value) in [
                ("available", balance.available()),
                ("equity", balance.equity()),
                ("liability", balance.liability()),
                ("max_loan", balance.max_loan()),
            ] {
                if !value.is_finite() || value.abs() > MAX_INITIAL_VALUE {
                    bail!(
                        "initial_portfolio balance {} {field} must be finite and within [-{MAX_INITIAL_VALUE}, {MAX_INITIAL_VALUE}]",
                        balance.currency
                    );
                }
            }
            if balance.liability() != 0.0 {
                bail!(
                    "initial_portfolio balance {} liability must be zero because borrowing is not modeled",
                    balance.currency
                );
            }
            if balance.max_loan() < 0.0 {
                bail!(
                    "initial_portfolio balance {} max_loan must be non-negative",
                    balance.currency
                );
            }
            if balance.forced_repayment_indicator.unwrap_or(0) != 0 {
                bail!(
                    "initial_portfolio balance {} has an active forced repayment indicator",
                    balance.currency
                );
            }
            if !balances.insert(balance.currency.as_str()) {
                bail!(
                    "initial_portfolio has duplicate balance currency {}",
                    balance.currency
                );
            }
            if let Some(symbol) = &balance.valuation_symbol {
                let instrument = instruments.get(symbol.as_str()).ok_or_else(|| {
                    anyhow::anyhow!(
                        "initial_portfolio balance {} valuation_symbol {} is not a configured instrument",
                        balance.currency,
                        symbol
                    )
                })?;
                if !instrument.kind.is_spot() || instrument.base_currency != balance.currency {
                    bail!(
                        "initial_portfolio balance {} valuation_symbol {} must be a spot instrument with that base currency",
                        balance.currency,
                        symbol
                    );
                }
                if balance.total < instrument.min_position
                    || balance.total > instrument.max_position
                {
                    bail!(
                        "initial_portfolio balance {} total {} is outside {} position bounds [{}, {}]",
                        balance.currency,
                        balance.total,
                        symbol,
                        instrument.min_position,
                        instrument.max_position
                    );
                }
            } else if balance.total > 0.0 {
                let is_spot_base = strategy.instruments.iter().any(|instrument| {
                    instrument.kind.is_spot() && instrument.base_currency == balance.currency
                });
                if is_spot_base {
                    bail!(
                        "positive initial spot-base balance {} requires valuation_symbol to prevent inventory double counting",
                        balance.currency
                    );
                }
                if balance.currency != "USD"
                    && !configured_rates.contains(balance.currency.as_str())
                {
                    bail!(
                        "positive initial cash balance {} requires a backtest.currency_rates route",
                        balance.currency
                    );
                }
            }
        }

        let missing_balances = required_balances
            .difference(&balances)
            .copied()
            .collect::<Vec<_>>();
        if !missing_balances.is_empty() {
            bail!(
                "initial_portfolio requires a complete account balance snapshot; missing currencies: {}",
                missing_balances.join(", ")
            );
        }

        let mut position_symbols = BTreeSet::new();
        for position in &self.positions {
            let instrument = instruments.get(position.symbol.as_str()).ok_or_else(|| {
                anyhow::anyhow!(
                    "initial_portfolio position {} is not a configured instrument",
                    position.symbol
                )
            })?;
            if instrument.kind.is_spot() {
                bail!(
                    "initial_portfolio position {} is spot; provide spot inventory through balances",
                    position.symbol
                );
            }
            if !position_symbols.insert(position.symbol.as_str()) {
                bail!(
                    "initial_portfolio has duplicate position symbol {}",
                    position.symbol
                );
            }
            if !position.qty.is_finite()
                || position.qty < instrument.min_position
                || position.qty > instrument.max_position
            {
                bail!(
                    "initial_portfolio position {} qty {} must be finite and within [{}, {}]",
                    position.symbol,
                    position.qty,
                    instrument.min_position,
                    instrument.max_position
                );
            }
            if !position.avg_price.is_finite()
                || position.avg_price < 0.0
                || position.avg_price > MAX_INITIAL_VALUE
                || (position.qty != 0.0 && position.avg_price == 0.0)
            {
                bail!(
                    "initial_portfolio position {} avg_price must be finite, non-negative, and positive for nonzero qty",
                    position.symbol
                );
            }
            if position.qty != 0.0 && position.margin_mode.is_none() {
                bail!(
                    "initial_portfolio nonzero derivative position {} requires margin_mode",
                    position.symbol
                );
            }
        }
        for (field, value) in [
            ("ratio", self.margin.ratio),
            ("exchange_ratio", self.margin.exchange_ratio),
            ("adjusted_equity_usd", self.margin.adjusted_equity_usd),
            ("notional_usd", self.margin.notional_usd),
        ] {
            if let Some(value) = value
                && (!value.is_finite() || !(0.0..=MAX_INITIAL_VALUE).contains(&value))
            {
                bail!(
                    "initial_portfolio margin {field} must be finite and within [0, {MAX_INITIAL_VALUE}]"
                );
            }
        }
        Ok(())
    }

    pub(crate) fn account_update(&self, ts_ms: u64) -> AccountUpdate {
        AccountUpdate {
            ts_ms,
            balances: self
                .balances
                .iter()
                .map(|balance| Balance {
                    account_id: self.account_id.clone(),
                    currency: balance.currency.clone(),
                    total: balance.total,
                    available: balance.available(),
                    equity: balance.equity(),
                    liability: balance.liability(),
                    max_loan: balance.max_loan(),
                    forced_repayment_indicator: balance.forced_repayment_indicator,
                })
                .collect(),
            positions: self
                .positions
                .iter()
                .map(|position| Position {
                    symbol: position.symbol.clone(),
                    qty: position.qty,
                    avg_price: position.avg_price,
                    margin_mode: position.margin_mode,
                })
                .collect(),
            margins: if self.margin.is_empty() {
                Vec::new()
            } else {
                vec![MarginSnapshot {
                    account_id: self.account_id.clone(),
                    ratio: self.margin.ratio,
                    exchange_ratio: self.margin.exchange_ratio,
                    adjusted_equity_usd: self.margin.adjusted_equity_usd,
                    notional_usd: self.margin.notional_usd,
                }]
            },
        }
    }
}

fn validate_optional_account_id(account_id: Option<&str>) -> Result<()> {
    let Some(account_id) = account_id else {
        return Ok(());
    };
    if account_id.is_empty()
        || account_id.len() > 128
        || account_id.trim() != account_id
        || !account_id.bytes().all(|byte| byte.is_ascii_graphic())
    {
        bail!(
            "initial_portfolio.account_id must be 1-128 printable ASCII characters without surrounding whitespace"
        );
    }
    Ok(())
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct BacktestCurrencyRateConfig {
    /// Currency units valued by this route, for example `USDT`.
    pub currency: String,
    /// Direct index whose price is USD per one currency unit, for example `USDT-USD`.
    pub index_symbol: Symbol,
    #[serde(default = "default_currency_rate_max_age_ms")]
    pub max_age_ms: u64,
}

fn default_currency_rate_max_age_ms() -> u64 {
    DEFAULT_CURRENCY_RATE_MAX_AGE_MS
}

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
    /// Explicit direct currency/USD indexes used only for portfolio accounting.
    pub currency_rates: Vec<BacktestCurrencyRateConfig>,
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
            currency_rates: Vec::new(),
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
        validate_currency_rates(&self.currency_rates)?;
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

fn validate_currency_rates(routes: &[BacktestCurrencyRateConfig]) -> Result<()> {
    if routes.len() > MAX_CURRENCY_RATE_ROUTES {
        bail!(
            "backtest.currency_rates has {} routes, maximum is {MAX_CURRENCY_RATE_ROUTES}",
            routes.len()
        );
    }
    let mut currencies = BTreeSet::new();
    let mut symbols = BTreeSet::new();
    for route in routes {
        validate_currency_name("backtest.currency_rates currency", &route.currency)?;
        if route.currency == "USD" {
            bail!("backtest.currency_rates must not configure USD, which is fixed at 1");
        }
        if route.index_symbol.is_empty()
            || route.index_symbol.len() > 128
            || route.index_symbol.trim() != route.index_symbol
            || !route
                .index_symbol
                .bytes()
                .all(|byte| byte.is_ascii_graphic())
        {
            bail!(
                "backtest.currency_rates index_symbol {:?} must be 1-128 printable ASCII characters without surrounding whitespace",
                route.index_symbol
            );
        }
        if route.max_age_ms == 0 || route.max_age_ms > MAX_BACKTEST_LATENCY_MS {
            bail!(
                "backtest.currency_rates {}/{} max_age_ms={} must be within [1, {MAX_BACKTEST_LATENCY_MS}]",
                route.currency,
                route.index_symbol,
                route.max_age_ms
            );
        }
        if !currencies.insert(route.currency.as_str()) {
            bail!(
                "backtest.currency_rates repeats currency {}",
                route.currency
            );
        }
        if !symbols.insert(route.index_symbol.as_str()) {
            bail!(
                "backtest.currency_rates repeats index_symbol {}",
                route.index_symbol
            );
        }
    }
    Ok(())
}

fn validate_currency_name(field: &str, currency: &str) -> Result<()> {
    if currency.is_empty()
        || currency.len() > 16
        || currency.trim() != currency
        || !currency
            .bytes()
            .all(|byte| byte.is_ascii_uppercase() || byte.is_ascii_digit())
    {
        bail!("{field} {currency:?} must be 1-16 uppercase ASCII letters or digits");
    }
    Ok(())
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
    #[serde(default)]
    pub initial_portfolio: BacktestInitialPortfolioConfig,
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
        assert!(config.initial_portfolio.is_empty());
    }

    #[test]
    fn strategy_toml_accepts_and_validates_complete_initial_portfolio() {
        let config: BacktestConfig = toml::from_str(
            r#"
                ref_symbol = "BTC-USDT"

                [backtest]
                [[backtest.currency_rates]]
                currency = "USDT"
                index_symbol = "USDT-USD"

                [[risk_groups]]
                name = "main"
                account_id = "acct-1"
                symbols = ["BTC-USDT", "BTC-USDT-SWAP"]

                [[instruments]]
                symbol = "BTC-USDT"
                kind = "spot"
                risk_group = "main"
                base_currency = "BTC"
                quote_currency = "USDT"
                min_position = 0.0
                max_position = 10.0

                [[instruments]]
                symbol = "BTC-USDT-SWAP"
                kind = "linear_swap"
                risk_group = "main"
                base_currency = "BTC"
                quote_currency = "USDT"
                settle_currency = "USDT"
                contract_value = 0.01
                min_position = -100.0
                max_position = 100.0

                [initial_portfolio]
                account_id = "acct-1"

                [[initial_portfolio.balances]]
                currency = "BTC"
                total = 0.25
                valuation_symbol = "BTC-USDT"

                [[initial_portfolio.balances]]
                currency = "USDT"
                total = 10000.0

                [[initial_portfolio.positions]]
                symbol = "BTC-USDT-SWAP"
                qty = -2.0
                avg_price = 50000.0
                margin_mode = "cross"
            "#,
        )
        .unwrap();

        assert_eq!(
            config.initial_portfolio.account_id.as_deref(),
            Some("acct-1")
        );
        assert_eq!(config.initial_portfolio.balances.len(), 2);
        assert_eq!(config.initial_portfolio.positions.len(), 1);
        config
            .initial_portfolio
            .validate(&config.strategy, &config.backtest)
            .unwrap();
    }

    #[test]
    fn initial_portfolio_rejects_ambiguous_or_incomplete_account_state() {
        let mut strategy = ChaosConfig {
            instruments: vec![reap_strategy::InstrumentConfig {
                symbol: "BTC-USDT".to_string(),
                kind: reap_strategy::InstrumentKindConfig::Spot,
                base_currency: "BTC".to_string(),
                quote_currency: "USDT".to_string(),
                min_position: 0.0,
                max_position: 1.0,
                ..Default::default()
            }],
            ..Default::default()
        };
        strategy.ref_symbol = "BTC-USDT".to_string();
        let execution = BacktestExecutionConfig {
            currency_rates: vec![BacktestCurrencyRateConfig {
                currency: "USDT".to_string(),
                index_symbol: "USDT-USD".to_string(),
                max_age_ms: 1_000,
            }],
            ..Default::default()
        };
        let initial = BacktestInitialPortfolioConfig {
            balances: vec![BacktestInitialBalanceConfig {
                currency: "BTC".to_string(),
                total: 0.1,
                valuation_symbol: None,
                ..Default::default()
            }],
            ..Default::default()
        };

        let error = initial
            .validate(&strategy, &execution)
            .unwrap_err()
            .to_string();
        assert!(error.contains("requires valuation_symbol"));

        let initial = BacktestInitialPortfolioConfig {
            balances: vec![BacktestInitialBalanceConfig {
                currency: "BTC".to_string(),
                total: 0.0,
                valuation_symbol: Some("BTC-USDT".to_string()),
                ..Default::default()
            }],
            ..Default::default()
        };
        let error = initial
            .validate(&strategy, &execution)
            .unwrap_err()
            .to_string();
        assert!(error.contains("missing currencies: USDT"));
    }

    #[test]
    fn initial_account_event_preserves_certified_balance_and_margin_fields() {
        let initial = BacktestInitialPortfolioConfig {
            account_id: Some("main".to_string()),
            balances: vec![BacktestInitialBalanceConfig {
                currency: "USDT".to_string(),
                total: 9_000.0,
                available: Some(8_000.0),
                equity: Some(10_000.0),
                liability: Some(0.0),
                max_loan: Some(500.0),
                forced_repayment_indicator: Some(0),
                valuation_symbol: None,
            }],
            margin: BacktestInitialMarginConfig {
                ratio: None,
                exchange_ratio: Some(12.5),
                adjusted_equity_usd: Some(10_000.0),
                notional_usd: Some(2_000.0),
            },
            ..Default::default()
        };

        let update = initial.account_update(1_000);

        assert_eq!(update.ts_ms, 1_000);
        assert_eq!(update.balances[0].available, 8_000.0);
        assert_eq!(update.balances[0].equity, 10_000.0);
        assert_eq!(update.balances[0].max_loan, 500.0);
        assert_eq!(update.balances[0].forced_repayment_indicator, Some(0));
        assert_eq!(update.margins[0].exchange_ratio, Some(12.5));
        assert_eq!(update.margins[0].adjusted_equity_usd, Some(10_000.0));
        assert_eq!(update.margins[0].notional_usd, Some(2_000.0));
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
                currency_rates: Vec::new(),
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
    fn strategy_toml_accepts_explicit_currency_rate_routes() {
        let config: BacktestConfig = toml::from_str(
            r#"
                ref_symbol = "BTC-USDT"

                [[backtest.currency_rates]]
                currency = "USDT"
                index_symbol = "USDT-USD"

                [[backtest.currency_rates]]
                currency = "USDC"
                index_symbol = "USDC-USD"
                max_age_ms = 10000
            "#,
        )
        .unwrap();

        assert_eq!(config.backtest.currency_rates.len(), 2);
        assert_eq!(
            config.backtest.currency_rates[0].max_age_ms,
            DEFAULT_CURRENCY_RATE_MAX_AGE_MS
        );
        assert_eq!(config.backtest.currency_rates[1].max_age_ms, 10_000);
        config.backtest.validate().unwrap();
    }

    #[test]
    fn currency_rate_routes_are_direct_unique_and_bounded() {
        let route = BacktestCurrencyRateConfig {
            currency: "USDT".to_string(),
            index_symbol: "USDT-USD".to_string(),
            max_age_ms: 75_000,
        };
        for routes in [
            vec![route.clone(), route.clone()],
            vec![BacktestCurrencyRateConfig {
                currency: "USD".to_string(),
                ..route.clone()
            }],
            vec![BacktestCurrencyRateConfig {
                currency: "usdt".to_string(),
                ..route.clone()
            }],
            vec![BacktestCurrencyRateConfig {
                max_age_ms: 0,
                ..route.clone()
            }],
        ] {
            let config = BacktestExecutionConfig {
                currency_rates: routes,
                ..BacktestExecutionConfig::default()
            };
            assert!(config.validate().is_err());
        }
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
