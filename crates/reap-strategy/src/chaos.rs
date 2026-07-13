use std::collections::{HashMap, HashSet};
use std::sync::Arc;

use serde::{Deserialize, Serialize};

use crate::Strategy;
use reap_core::{
    AccountUpdate, FillLiquidity, MarketEvent, NewOrder, OrderBook, OrderEvent, OrderIntent,
    OrderUpdate, Price, Quantity, SelfTradePrevention, Side, StrategyEvent, Symbol, SystemEvent,
    SystemEventKind, TimeInForce, TimeMs, round_down_to_lot, round_to_tick,
};

const HEDGE_VOL_TO_DELTA_RATIO: f64 = 1.5;
const LIVE_ORDER_STOP_QUOTE_THRESHOLD: f64 = 0.6;
const EXTRA_MARGIN_BPS: f64 = 0.0002;
const CAN_TRADE_DEBOUNCE_MS: TimeMs = 500;
const CAN_QUOTE_FULL_SIZE_DEBOUNCE_MS: TimeMs = 10_000;
const ORDER_CHECK_DELTA_THRESHOLD_USD: f64 = 51.0;
const ZOMBIE_HEDGE_THRESHOLD_MS: TimeMs = 30_000;
const EXCHANGE_MARGIN_RATIO_THRESHOLD: f64 = 5.0;
const EPS: f64 = 1e-9;

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(default)]
pub struct ChaosConfig {
    pub strategy_name: String,
    pub underlying: String,
    pub ref_symbol: Symbol,
    pub risk_multiplier: f64,
    pub coin_offset: f64,
    pub balance_sheet_limit_usd: f64,
    pub delta_limit_usd: f64,
    pub pnl_limit_usd: f64,
    pub pnl_breach_debounce_ms: TimeMs,
    pub basis_breach_debounce_ms: TimeMs,
    pub active_hedge_threshold_usd: f64,
    pub min_hedge_interval_ms: TimeMs,
    pub no_hedge_stop_ms: TimeMs,
    pub hedge_not_found_stop_ms: TimeMs,
    pub all_hedges_halted_stop_ms: TimeMs,
    pub insufficient_valid_stop_ms: TimeMs,
    pub index_deviation_limit: f64,
    pub index_deviation_debounce_ms: TimeMs,
    pub margin_breach_debounce_ms: TimeMs,
    pub ignore_best_level: bool,
    pub act_on_burst: bool,
    pub use_funding_rate_manager: bool,
    pub master_strategy: Option<String>,
    pub strategy_group: Option<String>,
    pub quote_only: bool,
    pub instruments: Vec<InstrumentConfig>,
    pub risk_groups: Vec<RiskGroupConfig>,
}

impl Default for ChaosConfig {
    fn default() -> Self {
        Self {
            strategy_name: "reap-chaos".to_string(),
            underlying: "BTC".to_string(),
            ref_symbol: String::new(),
            risk_multiplier: 1.0,
            coin_offset: 0.0,
            balance_sheet_limit_usd: f64::MAX,
            delta_limit_usd: 50_000.0,
            pnl_limit_usd: f64::MAX,
            pnl_breach_debounce_ms: 10_000,
            basis_breach_debounce_ms: 5_000,
            active_hedge_threshold_usd: 2_000.0,
            min_hedge_interval_ms: 1_000,
            no_hedge_stop_ms: 30_000,
            hedge_not_found_stop_ms: 3_000,
            all_hedges_halted_stop_ms: 90_000,
            insufficient_valid_stop_ms: 60_000,
            index_deviation_limit: 0.05,
            index_deviation_debounce_ms: 120_000,
            margin_breach_debounce_ms: 5_000,
            ignore_best_level: false,
            act_on_burst: false,
            use_funding_rate_manager: false,
            master_strategy: None,
            strategy_group: None,
            quote_only: false,
            instruments: Vec::new(),
            risk_groups: Vec::new(),
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ConfigValidation {
    pub valid: bool,
    pub errors: Vec<String>,
}

impl std::fmt::Display for ConfigValidation {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter.write_str(&self.errors.join("; "))
    }
}

impl std::error::Error for ConfigValidation {}

impl ChaosConfig {
    pub fn effective(&self) -> Self {
        let mut config = self.clone();
        let multiplier = if config.risk_multiplier > 0.0 {
            config.risk_multiplier
        } else {
            1.0
        };
        config.balance_sheet_limit_usd =
            scale_risk_limit(config.balance_sheet_limit_usd, multiplier);
        config.delta_limit_usd = scale_risk_limit(config.delta_limit_usd, multiplier);
        config.pnl_limit_usd = scale_risk_limit(config.pnl_limit_usd, multiplier);
        config.index_deviation_limit = scale_risk_limit(config.index_deviation_limit, multiplier);
        for group in &mut config.risk_groups {
            group.delta_stop_limit_usd = scale_risk_limit(group.delta_stop_limit_usd, multiplier);
            group.live_order_limit_usd = scale_risk_limit(group.live_order_limit_usd, multiplier);
            group.turnover_limit_usd = scale_risk_limit(group.turnover_limit_usd, multiplier);
            group.basis_limit = scale_risk_limit(group.basis_limit, multiplier);
            for coin in &mut group.coins {
                if coin.safety_multiplier == 0.0 {
                    coin.safety_multiplier = if coin.currency == "USDT" { 4.0 } else { 2.5 };
                }
            }
        }
        for instrument in &mut config.instruments {
            if instrument.safety_multiplier == 0.0 {
                instrument.safety_multiplier = if instrument.kind.is_derivative() {
                    2.0
                } else {
                    1.0
                };
            }
        }
        config.risk_multiplier = 1.0;
        config
    }

    pub fn validate(&self) -> ConfigValidation {
        let mut errors = Vec::new();
        if self.strategy_name.trim().is_empty() {
            errors.push("strategy_name must not be empty".to_string());
        }
        if self.underlying.trim().is_empty() {
            errors.push("underlying must not be empty".to_string());
        }
        if self
            .master_strategy
            .as_ref()
            .is_some_and(|name| name.trim().is_empty())
        {
            errors.push("master_strategy must be omitted or non-empty".to_string());
        }
        if self
            .strategy_group
            .as_ref()
            .is_some_and(|name| name.trim().is_empty())
        {
            errors.push("strategy_group must be omitted or non-empty".to_string());
        }
        if self.instruments.len() < 2 {
            errors.push(
                "iarb2 requires at least two instruments so quotes have a distinct hedge"
                    .to_string(),
            );
        }
        check_finite("coin_offset", self.coin_offset, &mut errors);
        check_positive("risk_multiplier", self.risk_multiplier, &mut errors);
        check_positive(
            "balance_sheet_limit_usd",
            self.balance_sheet_limit_usd,
            &mut errors,
        );
        check_positive("delta_limit_usd", self.delta_limit_usd, &mut errors);
        check_positive("pnl_limit_usd", self.pnl_limit_usd, &mut errors);
        check_non_negative(
            "active_hedge_threshold_usd",
            self.active_hedge_threshold_usd,
            &mut errors,
        );
        check_non_negative(
            "index_deviation_limit",
            self.index_deviation_limit,
            &mut errors,
        );

        let mut symbols = HashSet::new();
        for instrument in &self.instruments {
            if instrument.symbol.trim().is_empty() {
                errors.push("instrument symbol must not be empty".to_string());
            } else if !symbols.insert(instrument.symbol.clone()) {
                errors.push(format!("duplicate instrument symbol {}", instrument.symbol));
            }
            if !instrument.base_currency.is_empty() && instrument.base_currency != self.underlying {
                errors.push(format!(
                    "{}.base_currency {} does not match strategy underlying {}",
                    instrument.symbol, instrument.base_currency, self.underlying
                ));
            }
            check_positive(
                &format!("{}.tick_size", instrument.symbol),
                instrument.tick_size,
                &mut errors,
            );
            check_positive(
                &format!("{}.lot_size", instrument.symbol),
                instrument.lot_size,
                &mut errors,
            );
            check_positive(
                &format!("{}.max_order_size", instrument.symbol),
                instrument.max_order_size,
                &mut errors,
            );
            check_positive(
                &format!("{}.max_order_size_usd", instrument.symbol),
                instrument.max_order_size_usd,
                &mut errors,
            );
            check_positive(
                &format!("{}.safety_multiplier", instrument.symbol),
                instrument.safety_multiplier,
                &mut errors,
            );
            check_non_negative(
                &format!("{}.min_order_size_usd", instrument.symbol),
                instrument.min_order_size_usd,
                &mut errors,
            );
            if instrument.num_quote_levels == 0 {
                errors.push(format!(
                    "{}.num_quote_levels must be at least one",
                    instrument.symbol
                ));
            }
            check_non_negative(
                &format!("{}.min_level_spread", instrument.symbol),
                instrument.min_level_spread,
                &mut errors,
            );
            check_non_negative(
                &format!("{}.max_level_spread", instrument.symbol),
                instrument.max_level_spread,
                &mut errors,
            );
            check_non_negative(
                &format!("{}.debounce_width", instrument.symbol),
                instrument.debounce_width,
                &mut errors,
            );
            check_non_negative(
                &format!("{}.debounce_size_usd", instrument.symbol),
                instrument.debounce_size_usd,
                &mut errors,
            );
            if instrument.min_level_spread > instrument.max_level_spread {
                errors.push(format!(
                    "{}.min_level_spread exceeds max_level_spread",
                    instrument.symbol
                ));
            }
            if instrument.force_quote_update_ms == 0 {
                errors.push(format!(
                    "{}.force_quote_update_ms must be positive",
                    instrument.symbol
                ));
            }
            if instrument.depth_stale_threshold_ms == 0 {
                errors.push(format!(
                    "{}.depth_stale_threshold_ms must be positive",
                    instrument.symbol
                ));
            }
            check_positive(
                &format!("{}.min_trade_size", instrument.symbol),
                instrument.min_trade_size,
                &mut errors,
            );
            for (field, value) in [
                ("maker_fee", instrument.maker_fee),
                ("taker_fee", instrument.taker_fee),
                ("hedge_profit_margin", instrument.hedge_profit_margin),
                ("quote_profit_margin", instrument.quote_profit_margin),
                ("hedge_aggression", instrument.hedge_aggression),
                ("fv_offset", instrument.fv_offset),
                ("position_offset", instrument.position_offset),
                ("pos_skew", instrument.pos_skew),
                ("neg_skew", instrument.neg_skew),
                ("pos_extra_skew", instrument.pos_extra_skew),
                ("neg_extra_skew", instrument.neg_extra_skew),
                ("pos_activation", instrument.pos_activation),
                ("neg_activation", instrument.neg_activation),
                ("funding_rate", instrument.funding_rate),
                ("safety_multiplier", instrument.safety_multiplier),
                (
                    "index_deviation_adjustment",
                    instrument.index_deviation_adjustment,
                ),
                ("price_limit_buffer", instrument.price_limit_buffer),
            ] {
                check_finite(
                    &format!("{}.{field}", instrument.symbol),
                    value,
                    &mut errors,
                );
            }
            if instrument
                .funding_override
                .is_some_and(|funding| !funding.is_finite())
            {
                errors.push(format!(
                    "{}.funding_override must be finite when configured",
                    instrument.symbol
                ));
            }
            if instrument.index_deviation_adjustment.abs() > 0.09 {
                errors.push(format!(
                    "{}.index_deviation_adjustment must be within +/-0.09",
                    instrument.symbol
                ));
            }
            if !(0.0..1.0).contains(&instrument.price_limit_buffer) {
                errors.push(format!(
                    "{}.price_limit_buffer must be in [0, 1)",
                    instrument.symbol
                ));
            }
            for (field, value) in [
                ("hedge_profit_margin", instrument.hedge_profit_margin),
                ("quote_profit_margin", instrument.quote_profit_margin),
            ] {
                check_positive(
                    &format!("{}.{field}", instrument.symbol),
                    value,
                    &mut errors,
                );
            }
            for (field, value) in [
                ("hedge_aggression", instrument.hedge_aggression),
                ("pos_skew", instrument.pos_skew),
                ("neg_skew", instrument.neg_skew),
                ("pos_extra_skew", instrument.pos_extra_skew),
                ("neg_extra_skew", instrument.neg_extra_skew),
            ] {
                check_non_negative(
                    &format!("{}.{field}", instrument.symbol),
                    value,
                    &mut errors,
                );
            }
            if instrument.min_order_size_usd > instrument.max_order_size_usd {
                errors.push(format!(
                    "{}.min_order_size_usd exceeds max_order_size_usd",
                    instrument.symbol
                ));
            }
            if instrument.min_trade_size > instrument.max_order_size {
                errors.push(format!(
                    "{}.min_trade_size exceeds max_order_size",
                    instrument.symbol
                ));
            }
            if instrument.min_position > instrument.max_position {
                errors.push(format!(
                    "{}.min_position exceeds max_position",
                    instrument.symbol
                ));
            }
            if instrument.kind.is_derivative() {
                check_positive(
                    &format!("{}.contract_value", instrument.symbol),
                    instrument.contract_value,
                    &mut errors,
                );
            }
            for interval in &instrument.halt_intervals {
                if interval.start_sec_utc > interval.end_sec_utc || interval.end_sec_utc > 86_400 {
                    errors.push(format!(
                        "{} has invalid UTC halt interval {}..{}",
                        instrument.symbol, interval.start_sec_utc, interval.end_sec_utc
                    ));
                }
            }
        }
        if self.ref_symbol.trim().is_empty() {
            errors.push("ref_symbol must not be empty".to_string());
        } else if !symbols.contains(&self.ref_symbol) {
            errors.push(format!(
                "ref_symbol {} is not configured as an instrument",
                self.ref_symbol
            ));
        } else if self
            .instruments
            .iter()
            .find(|instrument| instrument.symbol == self.ref_symbol)
            .is_some_and(|instrument| !instrument.kind.is_spot())
        {
            errors.push(format!(
                "ref_symbol {} must be a spot instrument",
                self.ref_symbol
            ));
        }

        let mut group_names = HashSet::new();
        let mut grouped_symbols = HashSet::new();
        let mut account_coin_owner: HashMap<(Option<String>, String), String> = HashMap::new();
        if self.risk_groups.is_empty() {
            errors.push("at least one risk group is required".to_string());
        }
        for group in &self.risk_groups {
            if group.name.trim().is_empty() {
                errors.push("risk group name must not be empty".to_string());
            } else if !group_names.insert(group.name.clone()) {
                errors.push(format!("duplicate risk group {}", group.name));
            }
            check_positive(
                &format!("{}.soft_delta_limit_usd", group.name),
                group.soft_delta_limit_usd,
                &mut errors,
            );
            check_positive(
                &format!("{}.hard_delta_limit_usd", group.name),
                group.hard_delta_limit_usd,
                &mut errors,
            );
            check_positive(
                &format!("{}.delta_stop_limit_usd", group.name),
                group.delta_stop_limit_usd,
                &mut errors,
            );
            check_positive(
                &format!("{}.live_order_limit_usd", group.name),
                group.live_order_limit_usd,
                &mut errors,
            );
            check_positive(
                &format!("{}.turnover_limit_usd", group.name),
                group.turnover_limit_usd,
                &mut errors,
            );
            check_non_negative(
                &format!("{}.min_margin_level", group.name),
                group.min_margin_level,
                &mut errors,
            );
            if group.min_margin_level < 0.2 {
                errors.push(format!(
                    "{}.min_margin_level must be at least 0.2",
                    group.name
                ));
            }
            check_non_negative(
                &format!("{}.basis_limit", group.name),
                group.basis_limit,
                &mut errors,
            );
            check_finite(
                &format!("{}.coin_offset", group.name),
                group.coin_offset,
                &mut errors,
            );
            if !(group.soft_delta_limit_usd <= group.hard_delta_limit_usd
                && group.hard_delta_limit_usd <= group.delta_stop_limit_usd)
            {
                errors.push(format!(
                    "{} delta limits must satisfy soft <= hard <= stop",
                    group.name
                ));
            }
            let mut group_symbols = HashSet::new();
            if group.symbols.is_empty() {
                errors.push(format!(
                    "risk group {} must contain at least one instrument",
                    group.name
                ));
            }
            for symbol in &group.symbols {
                if !group_symbols.insert(symbol) {
                    errors.push(format!(
                        "risk group {} contains duplicate symbol {}",
                        group.name, symbol
                    ));
                }
                if !symbols.contains(symbol) {
                    errors.push(format!(
                        "risk group {} references unknown symbol {}",
                        group.name, symbol
                    ));
                } else if self
                    .instruments
                    .iter()
                    .find(|instrument| instrument.symbol == *symbol)
                    .is_some_and(|instrument| instrument.risk_group != group.name)
                {
                    errors.push(format!(
                        "instrument {} is listed in risk group {} but declares {}",
                        symbol,
                        group.name,
                        self.instruments
                            .iter()
                            .find(|instrument| instrument.symbol == *symbol)
                            .map(|instrument| instrument.risk_group.as_str())
                            .unwrap_or_default()
                    ));
                }
                if !grouped_symbols.insert(symbol.clone()) {
                    errors.push(format!(
                        "instrument {} belongs to more than one risk group",
                        symbol
                    ));
                }
                if let Some(instrument) = self
                    .instruments
                    .iter()
                    .find(|instrument| instrument.symbol == *symbol)
                {
                    let incompatible = match group.kind {
                        RiskGroupKindConfig::RefOnly | RiskGroupKindConfig::SpotOnly => {
                            !instrument.kind.is_spot()
                        }
                        RiskGroupKindConfig::FutureOnly => instrument.kind.is_spot(),
                        RiskGroupKindConfig::PortfolioAccount => false,
                    };
                    if incompatible {
                        errors.push(format!(
                            "instrument {} is incompatible with {:?} risk group {}",
                            symbol, group.kind, group.name
                        ));
                    }
                }
            }

            let max_expected_live_size = group
                .symbols
                .iter()
                .filter_map(|symbol| {
                    self.instruments
                        .iter()
                        .find(|instrument| instrument.symbol == *symbol)
                })
                .map(|instrument| {
                    instrument.max_order_size_usd * instrument.num_quote_levels as f64 * 2.0
                })
                .sum::<f64>();
            if group.live_order_limit_usd * LIVE_ORDER_STOP_QUOTE_THRESHOLD
                < max_expected_live_size * 1.4
            {
                errors.push(format!(
                    "{}.live_order_limit_usd is too small for configured quote levels",
                    group.name
                ));
            }

            let mut coin_currencies = HashSet::new();
            for coin in &group.coins {
                if coin.currency.trim().is_empty() {
                    errors.push(format!(
                        "risk group {} contains a coin with an empty currency",
                        group.name
                    ));
                } else if !coin_currencies.insert(coin.currency.clone()) {
                    errors.push(format!(
                        "risk group {} contains duplicate coin {}",
                        group.name, coin.currency
                    ));
                }
                let account_coin_key = (group.account_id.clone(), coin.currency.clone());
                if let Some(owner) = account_coin_owner.get(&account_coin_key) {
                    if owner != &group.name {
                        errors.push(format!(
                            "account {:?} currency {} is mapped to both risk groups {} and {}",
                            group.account_id, coin.currency, owner, group.name
                        ));
                    }
                } else {
                    account_coin_owner.insert(account_coin_key, group.name.clone());
                }
                if coin.min_balance > coin.max_balance {
                    errors.push(format!(
                        "{}.{} min_balance exceeds max_balance",
                        group.name, coin.currency
                    ));
                }
                check_non_negative(
                    &format!("{}.{}.borrow_limit_usd", group.name, coin.currency),
                    coin.borrow_limit_usd,
                    &mut errors,
                );
                check_non_negative(
                    &format!("{}.{}.borrow_limit_coin", group.name, coin.currency),
                    coin.borrow_limit_coin,
                    &mut errors,
                );
                check_positive(
                    &format!("{}.{}.safety_multiplier", group.name, coin.currency),
                    coin.safety_multiplier,
                    &mut errors,
                );
                for (field, value) in [
                    ("buy_skew", coin.buy_skew),
                    ("buy_extra_skew", coin.buy_extra_skew),
                    ("sell_skew", coin.sell_skew),
                    ("sell_extra_skew", coin.sell_extra_skew),
                ] {
                    check_non_negative(
                        &format!("{}.{}.{field}", group.name, coin.currency),
                        value,
                        &mut errors,
                    );
                }
                if coin.skew_type.is_none()
                    && (coin.buy_skew != 0.0
                        || coin.sell_skew != 0.0
                        || coin.buy_extra_skew != 0.0
                        || coin.sell_extra_skew != 0.0
                        || coin.skew_offset != 0.0)
                {
                    errors.push(format!(
                        "{}.{} has skew values but no skew_type",
                        group.name, coin.currency
                    ));
                }
            }
        }
        for instrument in &self.instruments {
            if !group_names.contains(&instrument.risk_group) {
                errors.push(format!(
                    "instrument {} references unknown risk group {}",
                    instrument.symbol, instrument.risk_group
                ));
            }
            if !grouped_symbols.contains(&instrument.symbol) {
                errors.push(format!(
                    "instrument {} is not listed in its risk group {}",
                    instrument.symbol, instrument.risk_group
                ));
            }
            if instrument.quote_profit_margin < 1.0
                && !self.instruments.iter().any(|hedge| {
                    let hedge_group_is_ref_only = self
                        .risk_groups
                        .iter()
                        .find(|group| group.name == hedge.risk_group)
                        .is_some_and(|group| group.kind == RiskGroupKindConfig::RefOnly);
                    hedge.symbol != instrument.symbol
                        && hedge.hedge_profit_margin < 1.0
                        && !hedge.halted
                        && !hedge_group_is_ref_only
                })
            {
                errors.push(format!(
                    "quote-enabled instrument {} has no distinct hedge-enabled instrument",
                    instrument.symbol
                ));
            }
        }

        let risk_group_coin_offset = self
            .risk_groups
            .iter()
            .map(|group| group.coin_offset)
            .sum::<f64>();
        if (self.coin_offset - risk_group_coin_offset).abs() > EPS {
            errors.push(format!(
                "coin_offset {} must equal the sum of risk-group offsets {}",
                self.coin_offset, risk_group_coin_offset
            ));
        }

        ConfigValidation {
            valid: errors.is_empty(),
            errors,
        }
    }
}

fn scale_risk_limit(value: f64, multiplier: f64) -> f64 {
    if value >= f64::MAX / multiplier {
        f64::MAX
    } else {
        value * multiplier
    }
}

fn check_positive(name: &str, value: f64, errors: &mut Vec<String>) {
    if !value.is_finite() || value <= 0.0 {
        errors.push(format!("{name} must be finite and positive"));
    }
}

fn check_non_negative(name: &str, value: f64, errors: &mut Vec<String>) {
    if !value.is_finite() || value < 0.0 {
        errors.push(format!("{name} must be finite and non-negative"));
    }
}

fn check_finite(name: &str, value: f64, errors: &mut Vec<String>) {
    if !value.is_finite() {
        errors.push(format!("{name} must be finite"));
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(default)]
pub struct RiskGroupConfig {
    pub name: String,
    pub kind: RiskGroupKindConfig,
    pub account_id: Option<String>,
    pub symbols: Vec<Symbol>,
    pub coins: Vec<CoinConfig>,
    pub coin_offset: f64,
    pub soft_delta_limit_usd: f64,
    pub hard_delta_limit_usd: f64,
    pub delta_stop_limit_usd: f64,
    pub live_order_limit_usd: f64,
    pub turnover_limit_usd: f64,
    pub min_margin_level: f64,
    pub basis_limit: f64,
}

impl Default for RiskGroupConfig {
    fn default() -> Self {
        Self {
            name: "default".to_string(),
            kind: RiskGroupKindConfig::PortfolioAccount,
            account_id: None,
            symbols: Vec::new(),
            coins: Vec::new(),
            coin_offset: 0.0,
            soft_delta_limit_usd: 25_000.0,
            hard_delta_limit_usd: 40_000.0,
            delta_stop_limit_usd: 60_000.0,
            live_order_limit_usd: 250_000.0,
            turnover_limit_usd: f64::MAX,
            min_margin_level: 0.2,
            basis_limit: 0.1,
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum RiskGroupKindConfig {
    RefOnly,
    SpotOnly,
    FutureOnly,
    PortfolioAccount,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(default)]
pub struct CoinConfig {
    pub currency: String,
    pub min_balance: f64,
    pub max_balance: f64,
    pub borrow_limit_usd: f64,
    pub borrow_limit_coin: f64,
    pub safety_multiplier: f64,
    pub skew_offset: f64,
    pub skew_type: Option<SkewTypeConfig>,
    pub buy_skew: f64,
    pub buy_extra_skew: f64,
    pub buy_activation: f64,
    pub sell_skew: f64,
    pub sell_extra_skew: f64,
    pub sell_activation: f64,
}

impl Default for CoinConfig {
    fn default() -> Self {
        Self {
            currency: String::new(),
            min_balance: -f64::MAX,
            max_balance: f64::MAX,
            borrow_limit_usd: 0.0,
            borrow_limit_coin: 0.0,
            safety_multiplier: 0.0,
            skew_offset: 0.0,
            skew_type: None,
            buy_skew: 0.0,
            buy_extra_skew: 0.0,
            buy_activation: f64::MAX,
            sell_skew: 0.0,
            sell_extra_skew: 0.0,
            sell_activation: -f64::MAX,
        }
    }
}

impl CoinConfig {
    fn borrow_limit(&self, usd_rate: f64) -> f64 {
        (self.borrow_limit_usd / usd_rate.max(EPS)).min(self.borrow_limit_coin)
    }

    fn skew_rate_at(&self, balance: f64) -> f64 {
        let Some(skew_type) = self.skew_type else {
            return 0.0;
        };
        if balance > self.skew_offset {
            if skew_type == SkewTypeConfig::Fix || balance <= self.buy_activation {
                self.buy_skew
            } else {
                self.buy_skew + self.buy_extra_skew
            }
        } else if balance < self.skew_offset {
            if skew_type == SkewTypeConfig::Fix || balance >= self.sell_activation {
                self.sell_skew
            } else {
                self.sell_skew + self.sell_extra_skew
            }
        } else {
            self.buy_skew.max(self.sell_skew)
        }
    }

    fn skew_zone(&self, balance: f64) -> i8 {
        if self.skew_type.is_none() {
            return 0;
        }
        if balance > self.skew_offset {
            if self.skew_type == Some(SkewTypeConfig::Fix) || balance <= self.buy_activation {
                1
            } else {
                2
            }
        } else if balance < self.skew_offset {
            if self.skew_type == Some(SkewTypeConfig::Fix) || balance >= self.sell_activation {
                -1
            } else {
                -2
            }
        } else {
            0
        }
    }

    fn integrated_skew(&self, balance: f64) -> f64 {
        let Some(skew_type) = self.skew_type else {
            return 0.0;
        };
        if balance >= self.skew_offset {
            let activation = self.buy_activation.max(self.skew_offset);
            let basic_end = balance.min(activation);
            let basic = (basic_end - self.skew_offset).max(0.0) * self.buy_skew;
            let extra = (balance - activation).max(0.0)
                * (self.buy_skew
                    + if skew_type == SkewTypeConfig::Step {
                        self.buy_extra_skew
                    } else {
                        0.0
                    });
            basic + extra
        } else {
            let activation = self.sell_activation.min(self.skew_offset);
            let basic_end = balance.max(activation);
            let basic = (basic_end - self.skew_offset).min(0.0) * self.sell_skew;
            let extra = (balance - activation).min(0.0)
                * (self.sell_skew
                    + if skew_type == SkewTypeConfig::Step {
                        self.sell_extra_skew
                    } else {
                        0.0
                    });
            basic + extra
        }
    }

    fn average_skew_rate(&self, current: f64, target: f64) -> f64 {
        let delta = target - current;
        if delta.abs() <= EPS {
            return self.skew_rate_at(target);
        }
        (self.integrated_skew(target) - self.integrated_skew(current)) / delta
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(default)]
pub struct InstrumentConfig {
    pub symbol: Symbol,
    pub kind: InstrumentKindConfig,
    pub risk_group: String,
    pub base_currency: String,
    pub quote_currency: String,
    pub settle_currency: String,
    pub maker_fee: f64,
    pub taker_fee: f64,
    pub hedge_profit_margin: f64,
    pub quote_profit_margin: f64,
    pub hedge_aggression: f64,
    pub hedge_priority: i32,
    pub fv_offset: f64,
    pub max_order_size_usd: f64,
    pub min_order_size_usd: f64,
    pub num_quote_levels: usize,
    pub min_level_spread: f64,
    pub max_level_spread: f64,
    pub debounce_width: f64,
    pub debounce_size_usd: f64,
    pub debounce_ms: TimeMs,
    pub force_quote_update_ms: TimeMs,
    pub min_refill_interval_ms: TimeMs,
    pub depth_stale_threshold_ms: TimeMs,
    pub max_order_size: f64,
    pub min_trade_size: f64,
    pub tick_size: f64,
    pub lot_size: f64,
    pub contract_value: f64,
    pub funding_rate: f64,
    pub funding_override: Option<f64>,
    pub index_symbol: Option<Symbol>,
    pub index_deviation_adjustment: f64,
    pub price_limit_buffer: f64,
    pub min_position: f64,
    pub max_position: f64,
    pub safety_multiplier: f64,
    pub position_offset: f64,
    pub skew_type: SkewTypeConfig,
    pub pos_skew: f64,
    pub neg_skew: f64,
    pub pos_extra_skew: f64,
    pub neg_extra_skew: f64,
    pub pos_activation: f64,
    pub neg_activation: f64,
    pub halt_intervals: Vec<HaltIntervalConfig>,
    pub halted: bool,
}

impl Default for InstrumentConfig {
    fn default() -> Self {
        Self {
            symbol: String::new(),
            kind: InstrumentKindConfig::Spot,
            risk_group: "default".to_string(),
            base_currency: String::new(),
            quote_currency: String::new(),
            settle_currency: String::new(),
            maker_fee: 0.0,
            taker_fee: 0.0004,
            hedge_profit_margin: 0.0003,
            quote_profit_margin: 0.0003,
            hedge_aggression: 0.0002,
            hedge_priority: 0,
            fv_offset: 0.0,
            max_order_size_usd: 5_000.0,
            min_order_size_usd: 100.0,
            num_quote_levels: 1,
            min_level_spread: 0.0001,
            max_level_spread: 0.0003,
            debounce_width: 0.0002,
            debounce_size_usd: 100.0,
            debounce_ms: 30,
            force_quote_update_ms: 30_000,
            min_refill_interval_ms: 300,
            depth_stale_threshold_ms: 60_000,
            max_order_size: 1.0,
            min_trade_size: 0.0001,
            tick_size: 0.1,
            lot_size: 0.0001,
            contract_value: 1.0,
            funding_rate: 0.0,
            funding_override: None,
            index_symbol: None,
            index_deviation_adjustment: 0.0,
            price_limit_buffer: 0.0,
            min_position: -f64::MAX,
            max_position: f64::MAX,
            safety_multiplier: 0.0,
            position_offset: 0.0,
            skew_type: SkewTypeConfig::Fix,
            pos_skew: 0.0,
            neg_skew: 0.0,
            pos_extra_skew: 0.0,
            neg_extra_skew: 0.0,
            pos_activation: f64::MAX,
            neg_activation: -f64::MAX,
            halt_intervals: Vec::new(),
            halted: false,
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum InstrumentKindConfig {
    Spot,
    /// Legacy linear derivative variant retained for existing reap configs.
    Future,
    LinearFuture,
    InverseFuture,
    LinearSwap,
    InverseSwap,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum SkewTypeConfig {
    Fix,
    Step,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub struct HaltIntervalConfig {
    pub start_sec_utc: u32,
    pub end_sec_utc: u32,
}

impl InstrumentKindConfig {
    pub fn is_spot(self) -> bool {
        self == Self::Spot
    }

    pub fn is_derivative(self) -> bool {
        !self.is_spot()
    }

    pub fn is_inverse(self) -> bool {
        matches!(self, Self::InverseFuture | Self::InverseSwap)
    }

    pub fn is_swap(self) -> bool {
        matches!(self, Self::LinearSwap | Self::InverseSwap)
    }
}

#[derive(Debug, Clone)]
pub struct ChaosStrategy {
    config: ChaosConfig,
    entities: HashMap<Symbol, InstrumentState>,
    risk_groups: HashMap<String, RiskGroupState>,
    symbol_to_group: HashMap<Symbol, String>,
    index_symbols: HashSet<Symbol>,
    index_prices: HashMap<Symbol, Price>,
    index_debouncers: HashMap<String, DebouncedCondition>,
    basis_debouncers: HashMap<String, DebouncedCondition>,
    basis_breaches: HashMap<String, (Symbol, f64)>,
    startup_basis_checked: bool,
    halt_reason: Option<String>,
    burst: f64,
    burst_symbol: Option<Symbol>,
    best_hedges: HashMap<Side, Vec<HedgeLevel>>,
    hedge_candidate_scratch: Vec<HedgeCandidate>,
    active_quotes: HashMap<(Symbol, Side, usize), ActiveQuote>,
    active_hedges: HashMap<String, ActiveHedge>,
    quote_targets: HashMap<(Symbol, Side), QuoteTargetState>,
    last_quote_fill_ms: HashMap<(Symbol, Side), TimeMs>,
    random: JavaRandom,
    now_ms: TimeMs,
    last_hedge_ms: TimeMs,
    delta_usd: f64,
    pending_delta_usd: f64,
    net_filled_delta_usd: f64,
    turnover_by_group: HashMap<String, f64>,
    pnl_debouncer: DebouncedCondition,
    margin_debouncers: HashMap<String, DebouncedCondition>,
    exchange_margin_debouncers: HashMap<String, DebouncedCondition>,
    no_hedge_found_since: Option<TimeMs>,
    hedge_not_found_since: Option<TimeMs>,
    all_hedges_halted_since: Option<TimeMs>,
    insufficient_valid_since: Option<TimeMs>,
    missed_hedges: Vec<MissedHedge>,
}

impl ChaosStrategy {
    pub fn new(config: ChaosConfig) -> Result<Self, ConfigValidation> {
        let config = config.effective();
        let validation = config.validate();
        if !validation.valid {
            return Err(validation);
        }

        let mut risk_groups = HashMap::new();
        for rg in &config.risk_groups {
            risk_groups.insert(rg.name.clone(), RiskGroupState::new(rg.clone()));
        }
        if risk_groups.is_empty() {
            risk_groups.insert(
                "default".to_string(),
                RiskGroupState::new(RiskGroupConfig::default()),
            );
        }

        let mut entities = HashMap::new();
        let mut symbol_to_group = HashMap::new();
        let index_symbols = config
            .instruments
            .iter()
            .filter_map(|instrument| instrument.index_symbol.clone())
            .collect();
        for inst in &config.instruments {
            let mut state = InstrumentState::new(inst.clone());
            state.ignore_best_level = config.ignore_best_level;
            if let Some(group) = risk_groups.get(&inst.risk_group) {
                if group.config.kind == RiskGroupKindConfig::RefOnly {
                    state.config.halted = true;
                }
                state.base_coin_config = group
                    .config
                    .coins
                    .iter()
                    .find(|coin| coin.currency == inst.base_currency)
                    .cloned();
                state.quote_coin_config = group
                    .config
                    .coins
                    .iter()
                    .find(|coin| coin.currency == inst.quote_currency)
                    .cloned();
                state.margin_coin_config = group
                    .config
                    .coins
                    .iter()
                    .find(|coin| coin.currency == inst.settle_currency)
                    .cloned();
            }
            symbol_to_group.insert(inst.symbol.clone(), inst.risk_group.clone());
            risk_groups
                .entry(inst.risk_group.clone())
                .or_insert_with(|| {
                    let rg = RiskGroupConfig {
                        name: inst.risk_group.clone(),
                        ..RiskGroupConfig::default()
                    };
                    RiskGroupState::new(rg)
                })
                .symbols
                .insert(inst.symbol.clone());
            entities.insert(inst.symbol.clone(), state);
        }

        for rg in risk_groups.values_mut() {
            if rg.symbols.is_empty() {
                rg.symbols.extend(rg.config.symbols.iter().cloned());
            }
            rg.max_quote_size_usd = rg
                .symbols
                .iter()
                .filter_map(|symbol| entities.get(symbol))
                .map(|entity| entity.config.max_order_size_usd)
                .fold(0.0, f64::max);
        }

        let mut best_hedges = HashMap::new();
        best_hedges.insert(Side::Buy, Vec::new());
        best_hedges.insert(Side::Sell, Vec::new());
        let index_debouncers = risk_groups
            .keys()
            .cloned()
            .map(|name| (name, DebouncedCondition::default()))
            .collect();
        let basis_debouncers = risk_groups
            .keys()
            .cloned()
            .map(|name| (name, DebouncedCondition::default()))
            .collect();
        let margin_debouncers = risk_groups
            .keys()
            .cloned()
            .map(|name| (name, DebouncedCondition::default()))
            .collect();
        let exchange_margin_debouncers = risk_groups
            .keys()
            .cloned()
            .map(|name| (name, DebouncedCondition::default()))
            .collect();

        Ok(Self {
            config,
            entities,
            risk_groups,
            symbol_to_group,
            index_symbols,
            index_prices: HashMap::new(),
            index_debouncers,
            basis_debouncers,
            basis_breaches: HashMap::new(),
            startup_basis_checked: false,
            halt_reason: None,
            burst: 0.0,
            burst_symbol: None,
            best_hedges,
            hedge_candidate_scratch: Vec::new(),
            active_quotes: HashMap::new(),
            active_hedges: HashMap::new(),
            quote_targets: HashMap::new(),
            last_quote_fill_ms: HashMap::new(),
            random: JavaRandom::new(1),
            now_ms: 0,
            last_hedge_ms: 0,
            delta_usd: 0.0,
            pending_delta_usd: 0.0,
            net_filled_delta_usd: 0.0,
            turnover_by_group: HashMap::new(),
            pnl_debouncer: DebouncedCondition::default(),
            margin_debouncers,
            exchange_margin_debouncers,
            no_hedge_found_since: None,
            hedge_not_found_since: None,
            all_hedges_halted_since: None,
            insufficient_valid_since: None,
            missed_hedges: Vec::new(),
        })
    }

    pub fn delta_usd(&self) -> f64 {
        self.delta_usd
    }

    pub fn pending_delta_usd(&self) -> f64 {
        self.pending_delta_usd
    }

    pub fn entity(&self, symbol: &str) -> Option<&InstrumentState> {
        self.entities.get(symbol)
    }

    pub fn risk_group(&self, name: &str) -> Option<&RiskGroupState> {
        self.risk_groups.get(name)
    }

    pub fn halt_reason(&self) -> Option<&str> {
        self.halt_reason.as_deref()
    }

    pub fn trading_pnl_usd(&self) -> f64 {
        let Some(ref_mid) = self.ref_mid() else {
            return 0.0;
        };
        self.entities
            .values()
            .map(|entity| entity.trading_pnl_usd(ref_mid))
            .sum()
    }

    pub fn missed_hedges(&self) -> &[MissedHedge] {
        &self.missed_hedges
    }

    pub fn basis_breaches(&self) -> &HashMap<String, (Symbol, f64)> {
        &self.basis_breaches
    }

    fn on_depth(&mut self, book: &OrderBook) -> Vec<OrderIntent> {
        self.now_ms = book.ts_ms;
        if let Some(entity) = self.entities.get_mut(&book.symbol) {
            entity.book = Some(book.clone());
        }
        self.refresh_quotes()
    }

    fn on_owned_depth(&mut self, book: OrderBook) -> Vec<OrderIntent> {
        self.now_ms = book.ts_ms;
        if let Some(entity) = self.entities.get_mut(book.symbol.as_str()) {
            entity.book = Some(book);
        }
        self.refresh_quotes()
    }

    fn refresh_quotes(&mut self) -> Vec<OrderIntent> {
        self.update_interval_halts();
        self.update_funding_window();
        self.update_risk();
        let validity_healthy = self.check_validity();
        let risk_healthy = validity_healthy && self.check_risk_limits();
        self.update_best_hedges();
        let hedge_healthy = self.check_hedge_availability();
        let startup_basis_healthy = if !self.startup_basis_checked && self.pricing_ready() {
            self.startup_basis_checked = true;
            if self.check_basis(true) {
                true
            } else {
                self.halt_reason = Some("startup basis limit breached".to_string());
                false
            }
        } else {
            if self.startup_basis_checked {
                let _ = self.check_basis(false);
            }
            true
        };
        self.update_theo_quotes();
        let pricing_healthy =
            risk_healthy && hedge_healthy && startup_basis_healthy && self.check_index_deviation();

        let strat_can_buy = self.pending_delta_usd <= 0.5 * self.config.delta_limit_usd
            && self.delta_usd <= 0.5 * self.config.delta_limit_usd;
        let strat_can_sell = self.pending_delta_usd >= -0.5 * self.config.delta_limit_usd
            && self.delta_usd >= -0.5 * self.config.delta_limit_usd;

        let mut commands = Vec::new();
        let symbols: Vec<_> = self.entities.keys().cloned().collect();
        for symbol in symbols {
            for side in [Side::Buy, Side::Sell] {
                let can_side = match side {
                    Side::Buy => strat_can_buy,
                    Side::Sell => strat_can_sell,
                };
                let Some(entity) = self.entities.get(&symbol) else {
                    continue;
                };
                let group_can_quote = self
                    .symbol_to_group
                    .get(&symbol)
                    .and_then(|rg| self.risk_groups.get(rg))
                    .is_none_or(|rg| rg.can_quote_side(side));
                let desired =
                    if pricing_healthy && can_side && group_can_quote && entity.can_quote(side) {
                        entity
                            .theo(side)
                            .map(|mut quote| {
                                if self.config.quote_only {
                                    quote.price = entity.quote_only_price(side, quote.price);
                                }
                                quote
                            })
                            .filter(|quote| {
                                quote.price.is_finite() && quote.qty >= entity.config.min_trade_size
                            })
                    } else {
                        None
                    };
                let desired_levels = self.desired_quote_levels(&symbol, side, desired);
                self.sync_quotes(&symbol, side, &desired_levels, &mut commands);
            }
        }
        commands
    }

    fn check_validity(&mut self) -> bool {
        if self.halt_reason.is_some() {
            return false;
        }
        let valid_count = self
            .entities
            .values()
            .filter(|entity| entity.book_is_valid_at(self.now_ms) && !entity.feed_stale)
            .count();
        if valid_count < 2 {
            let since = self.insufficient_valid_since.get_or_insert(self.now_ms);
            if self.now_ms.saturating_sub(*since) > self.config.insufficient_valid_stop_ms {
                self.halt_reason = Some(format!(
                    "fewer than two instruments valid for more than {}ms",
                    self.config.insufficient_valid_stop_ms
                ));
                return false;
            }
        } else {
            self.insufficient_valid_since = None;
        }
        true
    }

    fn check_risk_limits(&mut self) -> bool {
        if self.halt_reason.is_some() {
            return false;
        }
        if let Some((order_id, hedge)) = self.active_hedges.iter().find(|(_, hedge)| {
            self.now_ms.saturating_sub(hedge.updated_ms) > ZOMBIE_HEDGE_THRESHOLD_MS
        }) {
            self.halt_reason = Some(format!(
                "hedge order {} on {} has remained live for more than {}ms",
                order_id, hedge.symbol, ZOMBIE_HEDGE_THRESHOLD_MS
            ));
            return false;
        }
        if !self.delta_usd.is_finite() || !self.pending_delta_usd.is_finite() {
            self.halt_reason = Some(format!(
                "non-finite strategy delta/pending delta {}/{}",
                self.delta_usd, self.pending_delta_usd
            ));
            return false;
        }
        if self.delta_usd.abs() > self.config.delta_limit_usd {
            self.halt_reason = Some(format!(
                "strategy delta {} exceeds {}",
                self.delta_usd, self.config.delta_limit_usd
            ));
            return false;
        }
        if self.config.strategy_group.is_none()
            && self.net_filled_delta_usd > self.config.delta_limit_usd * 2.0
        {
            self.halt_reason = Some(format!(
                "net filled delta {} exceeds {}",
                self.net_filled_delta_usd,
                self.config.delta_limit_usd * 2.0
            ));
            return false;
        }
        for group in self.risk_groups.values() {
            if !group.delta_usd.is_finite() || !group.pending_delta_usd.is_finite() {
                self.halt_reason = Some(format!(
                    "risk group {} has non-finite delta/pending delta {}/{}",
                    group.config.name, group.delta_usd, group.pending_delta_usd
                ));
                return false;
            }
            if group.delta_usd.abs() > group.config.delta_stop_limit_usd {
                self.halt_reason = Some(format!(
                    "risk group {} delta {} exceeds {}",
                    group.config.name, group.delta_usd, group.config.delta_stop_limit_usd
                ));
                return false;
            }
            if group.pending_delta_usd.abs() > group.config.delta_stop_limit_usd {
                self.halt_reason = Some(format!(
                    "risk group {} pending delta {} exceeds {}",
                    group.config.name, group.pending_delta_usd, group.config.delta_stop_limit_usd
                ));
                return false;
            }
            if !group.live_order_size_usd.is_finite()
                || group.live_order_size_usd > group.config.live_order_limit_usd
            {
                self.halt_reason = Some(format!(
                    "risk group {} live order size {} exceeds {}",
                    group.config.name, group.live_order_size_usd, group.config.live_order_limit_usd
                ));
                return false;
            }
            let turnover = self
                .turnover_by_group
                .get(&group.config.name)
                .copied()
                .unwrap_or(0.0);
            if !turnover.is_finite() || turnover > group.config.turnover_limit_usd {
                self.halt_reason = Some(format!(
                    "risk group {} turnover {} exceeds {}",
                    group.config.name, turnover, group.config.turnover_limit_usd
                ));
                return false;
            }
            if let Some(exchange_margin_ratio) = group.exchange_margin_ratio {
                let breaching = !exchange_margin_ratio.is_finite()
                    || exchange_margin_ratio < EXCHANGE_MARGIN_RATIO_THRESHOLD;
                if self
                    .exchange_margin_debouncers
                    .entry(group.config.name.clone())
                    .or_default()
                    .check(
                        breaching,
                        self.now_ms,
                        self.config.margin_breach_debounce_ms,
                    )
                {
                    self.halt_reason = Some(format!(
                        "risk group {} exchange margin ratio {} is below {}",
                        group.config.name, exchange_margin_ratio, EXCHANGE_MARGIN_RATIO_THRESHOLD
                    ));
                    return false;
                }
            }
            if let Some(margin_ratio) = group.margin_ratio {
                let breaching =
                    !margin_ratio.is_finite() || margin_ratio < group.config.min_margin_level;
                if self
                    .margin_debouncers
                    .entry(group.config.name.clone())
                    .or_default()
                    .check(
                        breaching,
                        self.now_ms,
                        self.config.margin_breach_debounce_ms,
                    )
                {
                    self.halt_reason = Some(format!(
                        "risk group {} margin ratio {} is below {}",
                        group.config.name, margin_ratio, group.config.min_margin_level
                    ));
                    return false;
                }
            }
        }

        let Some(ref_mid) = self.ref_mid() else {
            return true;
        };
        let trading_pnl = self
            .entities
            .values()
            .map(|entity| entity.trading_pnl_usd(ref_mid))
            .sum::<f64>();
        if !trading_pnl.is_finite() {
            self.halt_reason = Some("trading pnl is non-finite".to_string());
            return false;
        }
        if self.pnl_debouncer.check(
            trading_pnl < -self.config.pnl_limit_usd,
            self.now_ms,
            self.config.pnl_breach_debounce_ms,
        ) {
            self.halt_reason = Some(format!(
                "trading pnl {} is below -{}",
                trading_pnl, self.config.pnl_limit_usd
            ));
            return false;
        }
        let mut total_balance_usd = 0.0;
        let mut seen_balances = HashSet::new();
        for group in self.risk_groups.values() {
            for coin in &group.config.coins {
                let Some(balance) = group.account_balances.get(&coin.currency) else {
                    continue;
                };
                let usd_rate = if is_usd_equivalent(&coin.currency) {
                    1.0
                } else {
                    ref_mid
                };
                let borrow_limit = coin.borrow_limit(usd_rate);
                if !balance.liability.is_finite()
                    || !balance.cash.is_finite()
                    || balance.liability > borrow_limit
                {
                    self.halt_reason = Some(format!(
                        "risk group {} {} liability {} exceeds {}",
                        group.config.name, coin.currency, balance.liability, borrow_limit
                    ));
                    return false;
                }
                if !is_usd_equivalent(&coin.currency)
                    && seen_balances
                        .insert((group.config.account_id.clone(), coin.currency.clone()))
                {
                    total_balance_usd += balance.cash * usd_rate;
                }
            }
        }
        for entity in self.entities.values() {
            if !entity.config.kind.is_spot() || !entity.balances_initialized {
                continue;
            }
            let group = self.risk_groups.get(&entity.config.risk_group);
            if group.is_some_and(|group| !group.account_balances.is_empty()) {
                continue;
            }
            let account_id = group.and_then(|group| group.config.account_id.as_deref());
            if !is_usd_equivalent(&entity.config.base_currency)
                && seen_balances.insert((
                    account_id.map(str::to_string),
                    entity.config.base_currency.clone(),
                ))
            {
                total_balance_usd += entity.base_balance * ref_mid;
            }
            if let Some(base) = &entity.base_coin_config {
                let borrow_limit = base.borrow_limit(ref_mid);
                if !entity.base_liability.is_finite() || entity.base_liability > borrow_limit {
                    self.halt_reason = Some(format!(
                        "{} {} liability {} exceeds {}",
                        entity.config.symbol,
                        entity.config.base_currency,
                        entity.base_liability,
                        borrow_limit
                    ));
                    return false;
                }
            }
            if let Some(quote) = &entity.quote_coin_config {
                let quote_rate = if is_usd_equivalent(&entity.config.quote_currency) {
                    1.0
                } else {
                    ref_mid
                };
                let borrow_limit = quote.borrow_limit(quote_rate);
                if !entity.quote_liability.is_finite() || entity.quote_liability > borrow_limit {
                    self.halt_reason = Some(format!(
                        "{} {} liability {} exceeds {}",
                        entity.config.symbol,
                        entity.config.quote_currency,
                        entity.quote_liability,
                        borrow_limit
                    ));
                    return false;
                }
            }
        }
        if !total_balance_usd.is_finite() || total_balance_usd > self.config.balance_sheet_limit_usd
        {
            self.halt_reason = Some(format!(
                "balance sheet {} exceeds {}",
                total_balance_usd, self.config.balance_sheet_limit_usd
            ));
            return false;
        }
        true
    }

    fn update_interval_halts(&mut self) {
        let second_in_day = ((self.now_ms / 1_000) % 86_400) as u32;
        for entity in self.entities.values_mut() {
            entity.interval_halted = entity.config.kind.is_derivative()
                && entity.config.halt_intervals.iter().any(|interval| {
                    second_in_day >= interval.start_sec_utc && second_in_day <= interval.end_sec_utc
                });
        }
    }

    fn update_funding_window(&mut self) {
        if !self.config.use_funding_rate_manager {
            for entity in self.entities.values_mut() {
                entity.funding_rate_active = true;
            }
            return;
        }
        let current_window = self
            .entities
            .values()
            .filter(|entity| entity.config.kind.is_swap())
            .map(|entity| entity.funding_time_ms)
            .filter(|funding_time| *funding_time > self.now_ms)
            .min();
        for entity in self.entities.values_mut() {
            entity.funding_rate_active = entity.config.kind.is_swap()
                && current_window.is_some_and(|window| entity.funding_time_ms == window);
        }
    }

    fn check_index_deviation(&mut self) -> bool {
        if self.halt_reason.is_some() {
            return false;
        }
        let mut group_names = self.risk_groups.keys().cloned().collect::<Vec<_>>();
        group_names.sort();
        for group_name in group_names {
            let mut worst: Option<(Symbol, Symbol, f64)> = None;
            if let Some(group) = self.risk_groups.get(&group_name) {
                for symbol in &group.symbols {
                    let Some(entity) = self.entities.get(symbol) else {
                        continue;
                    };
                    if !entity.config.kind.is_spot() {
                        continue;
                    }
                    let Some(index_symbol) = entity.config.index_symbol.as_ref() else {
                        continue;
                    };
                    let (Some(spot_mid), Some(index_price)) =
                        (entity.mid(), self.index_prices.get(index_symbol).copied())
                    else {
                        continue;
                    };
                    let deviation =
                        spot_mid / index_price - 1.0 - entity.config.index_deviation_adjustment;
                    if deviation.abs() > self.config.index_deviation_limit
                        && worst
                            .as_ref()
                            .is_none_or(|(_, _, current)| deviation.abs() > current.abs())
                    {
                        worst = Some((symbol.clone(), index_symbol.clone(), deviation));
                    }
                }
            }
            let should_stop = self.index_debouncers.entry(group_name).or_default().check(
                worst.is_some(),
                self.now_ms,
                self.config.index_deviation_debounce_ms,
            );
            if should_stop && let Some((symbol, index_symbol, deviation)) = worst {
                self.halt_reason = Some(format!(
                    "{} index deviation {} versus {} exceeds {}",
                    symbol, deviation, index_symbol, self.config.index_deviation_limit
                ));
                return false;
            }
        }
        true
    }

    fn desired_quote_levels(
        &mut self,
        symbol: &str,
        side: Side,
        desired: Option<TheoQuote>,
    ) -> Vec<TheoQuote> {
        let key = (symbol.to_string(), side);
        let Some(mut top) = desired else {
            self.quote_targets.remove(&key);
            return Vec::new();
        };
        let Some(entity) = self.entities.get(symbol).cloned() else {
            return Vec::new();
        };
        let Some(ref_mid) = self.ref_mid() else {
            return Vec::new();
        };
        let quote_level_count = entity.quote_level_count(side);
        top.price = round_passive_to_tick(top.price, entity.config.tick_size, side);

        if let Some(current) = self.quote_targets.get(&key) {
            let current_top = current.levels.first();
            let price_changed = current_top.is_none_or(|quote| {
                let passive_width = if self.config.quote_only {
                    0.0
                } else {
                    entity.config.debounce_width
                };
                let more_aggressive = side.factor() * (top.price - quote.price) / top.price
                    > entity.config.debounce_width;
                let less_aggressive =
                    side.factor() * (quote.price - top.price) / top.price > passive_width;
                more_aggressive || less_aggressive
            });
            let debounce_qty = entity.size_from_usd(entity.config.debounce_size_usd, ref_mid);
            let size_reduced = current_top.is_some_and(|quote| top.qty < quote.qty - debounce_qty);
            let force_update = self.now_ms.saturating_sub(current.updated_ms)
                >= entity.config.force_quote_update_ms;
            let level_count_changed = current.levels.len() != quote_level_count;
            if !price_changed && !size_reduced && !force_update && !level_count_changed {
                return current.levels.clone();
            }
            let force_debounce = current_top.is_some_and(|quote| match side {
                Side::Buy => top.price < quote.price * 0.999,
                Side::Sell => top.price > quote.price * 1.001,
            });
            if !force_debounce
                && !force_update
                && !level_count_changed
                && self.now_ms.saturating_sub(current.updated_ms) < entity.config.debounce_ms
            {
                return current.levels.clone();
            }
        }

        let mut levels = Vec::with_capacity(quote_level_count);
        levels.push(top.clone());
        let min_qty = entity.size_from_usd(entity.config.min_order_size_usd, ref_mid);
        let max_qty = entity.size_from_usd(entity.config.max_order_size_usd, ref_mid);
        let qty_span = (max_qty - min_qty).max(0.0);
        let spread_span = entity.config.max_level_spread - entity.config.min_level_spread;
        let mut last_px = top.price;
        for _ in 1..quote_level_count {
            let random = self.random.next_f64();
            let spread = entity.config.min_level_spread + random * spread_span;
            let raw_px = match side {
                Side::Buy => last_px * (1.0 - spread),
                Side::Sell => last_px * (1.0 + spread),
            };
            let rounded = round_passive_to_tick(raw_px, entity.config.tick_size, side);
            let price = match side {
                Side::Buy => rounded.min(last_px - entity.config.tick_size),
                Side::Sell => rounded.max(last_px + entity.config.tick_size),
            };
            let qty = min_qty + random * qty_span;
            levels.push(TheoQuote {
                price,
                qty,
                hedge_px: top.hedge_px,
                hedge_symbol: top.hedge_symbol.clone(),
            });
            last_px = price;
        }
        self.quote_targets.insert(
            key,
            QuoteTargetState {
                levels: levels.clone(),
                updated_ms: self.now_ms,
            },
        );
        levels
    }

    fn sync_quotes(
        &mut self,
        symbol: &str,
        side: Side,
        desired: &[TheoQuote],
        commands: &mut Vec<OrderIntent>,
    ) {
        let Some(entity) = self.entities.get(symbol) else {
            return;
        };
        let min_refill_interval_ms = entity.config.min_refill_interval_ms;
        let refill_blocked = self
            .last_quote_fill_ms
            .get(&(symbol.to_string(), side))
            .is_some_and(|last_fill_ms| {
                self.now_ms.saturating_sub(*last_fill_ms) < min_refill_interval_ms
            });
        let desired = desired
            .iter()
            .filter_map(|quote| {
                let price = round_passive_to_tick(quote.price, entity.config.tick_size, side);
                let qty = round_down_to_lot(quote.qty, entity.config.lot_size);
                (price.is_finite() && price > 0.0 && qty >= entity.config.min_trade_size)
                    .then_some((price, qty))
            })
            .collect::<Vec<_>>();

        let active_levels = self
            .active_quotes
            .iter()
            .filter(|((active_symbol, active_side, _), _)| {
                active_symbol == symbol && *active_side == side
            })
            .map(|((_, _, level), quote)| (*level, quote.clone()))
            .collect::<HashMap<_, _>>();

        for (level, active) in &active_levels {
            let matches = desired.get(*level).is_some_and(|(price, qty)| {
                approx_eq(active.price, *price)
                    && (approx_eq(active.qty, *qty)
                        || (*level == 0 && refill_blocked && active.qty < *qty))
            });
            if !matches {
                commands.push(OrderIntent::CancelOrder {
                    order_id: active.order_id.clone(),
                    reason: if desired.get(*level).is_some() {
                        "replace_quote".to_string()
                    } else {
                        "quote_disabled".to_string()
                    },
                });
            }
        }

        for (level, (price, qty)) in desired.into_iter().enumerate() {
            let current_matches = active_levels.get(&level).is_some_and(|active| {
                approx_eq(active.price, price)
                    && (approx_eq(active.qty, qty)
                        || (level == 0 && refill_blocked && active.qty < qty))
            });
            if current_matches {
                continue;
            }
            if level == 0 && refill_blocked && !active_levels.contains_key(&level) {
                continue;
            }
            commands.push(OrderIntent::NewOrder(NewOrder {
                symbol: symbol.to_string(),
                side,
                qty,
                price,
                time_in_force: TimeInForce::PostOnly,
                reduce_only: false,
                self_trade_prevention: None,
                reason: if level == 0 {
                    "quote".to_string()
                } else {
                    format!("quote:{level}")
                },
            }));
        }
    }

    fn update_risk(&mut self) {
        let ref_mid = self.ref_mid().unwrap_or(1.0);
        let live_sizes: HashMap<String, f64> = self
            .risk_groups
            .iter()
            .map(|(name, rg)| {
                let quote_size = self
                    .active_quotes
                    .iter()
                    .filter(|((symbol, _, _), _)| rg.symbols.contains(symbol))
                    .map(|((symbol, _, _), quote)| {
                        self.entities
                            .get(symbol)
                            .map(|entity| entity.notional_usd(quote.qty, quote.price, ref_mid))
                            .unwrap_or(0.0)
                    })
                    .sum::<f64>();
                let hedge_size = self
                    .active_hedges
                    .values()
                    .filter(|hedge| rg.symbols.contains(&hedge.symbol))
                    .map(|hedge| {
                        self.entities
                            .get(&hedge.symbol)
                            .map(|entity| {
                                entity.notional_usd(
                                    hedge.signed_open_qty.abs(),
                                    hedge.price,
                                    ref_mid,
                                )
                            })
                            .unwrap_or(0.0)
                    })
                    .sum::<f64>();
                (name.clone(), quote_size + hedge_size)
            })
            .collect();

        for (name, rg) in self.risk_groups.iter_mut() {
            if let (Some(equity), Some(notional)) =
                (rg.margin_adjusted_equity_usd, rg.margin_notional_usd)
            {
                let liability_usd = rg
                    .config
                    .coins
                    .iter()
                    .filter_map(|coin| {
                        rg.account_balances.get(&coin.currency).map(|balance| {
                            let rate = if is_usd_equivalent(&coin.currency) {
                                1.0
                            } else {
                                ref_mid
                            };
                            balance.liability.abs() * rate
                        })
                    })
                    .sum::<f64>();
                rg.margin_ratio = Some(equity / (notional + liability_usd));
            }
            let mut spot_delta_coin = 0.0;
            let mut derivative_delta_coin = 0.0;
            let mut seen_spot_balances = HashSet::new();
            for symbol in &rg.symbols {
                if let Some(entity) = self.entities.get(symbol) {
                    if entity.config.kind.is_spot() {
                        let balance_key = if entity.config.base_currency.is_empty() {
                            entity.config.symbol.clone()
                        } else {
                            entity.config.base_currency.clone()
                        };
                        if seen_spot_balances.insert(balance_key) {
                            spot_delta_coin += entity.delta_coin();
                        }
                    } else {
                        derivative_delta_coin += entity.delta_coin();
                    }
                }
            }
            let account_delta_coin = rg
                .account_balances
                .iter()
                .filter(|(currency, _)| !is_usd_equivalent(currency))
                .map(|(_, balance)| balance.cash)
                .sum::<f64>();
            let account_delta_coin = if rg.account_balances.is_empty() {
                spot_delta_coin
            } else {
                account_delta_coin
            };
            let delta_coin = match rg.config.kind {
                RiskGroupKindConfig::RefOnly => 0.0,
                RiskGroupKindConfig::SpotOnly => account_delta_coin,
                RiskGroupKindConfig::FutureOnly => derivative_delta_coin,
                RiskGroupKindConfig::PortfolioAccount => account_delta_coin + derivative_delta_coin,
            } - rg.config.coin_offset;
            rg.delta_usd = delta_coin * ref_mid;
            let live_hedge_delta_coin = self
                .active_hedges
                .values()
                .filter(|hedge| rg.symbols.contains(&hedge.symbol))
                .filter_map(|hedge| {
                    self.entities
                        .get(&hedge.symbol)
                        .map(|entity| entity.delta_coin_for_qty(hedge.signed_open_qty, hedge.price))
                })
                .sum::<f64>();
            rg.pending_delta_usd = (delta_coin + live_hedge_delta_coin) * ref_mid;
            rg.live_order_size_usd = live_sizes.get(name).copied().unwrap_or(0.0);
        }

        self.delta_usd = self
            .risk_groups
            .values()
            .map(|rg| rg.delta_usd)
            .sum::<f64>();
        self.pending_delta_usd = self
            .risk_groups
            .values()
            .map(|rg| rg.pending_delta_usd)
            .sum::<f64>();
    }

    fn update_best_hedges(&mut self) {
        for entity in self.entities.values_mut() {
            entity.refresh_trade_permissions(self.now_ms);
        }
        let Some(ref_mid) = self.ref_mid() else {
            return;
        };
        for entity in self.entities.values_mut() {
            entity.update_take_rate(Side::Buy, ref_mid);
            entity.update_take_rate(Side::Sell, ref_mid);
        }

        self.best_hedges.entry(Side::Buy).or_default().clear();
        self.best_hedges.entry(Side::Sell).or_default().clear();

        let mut levels = std::mem::take(&mut self.hedge_candidate_scratch);
        let group_names: Vec<_> = self.risk_groups.keys().cloned().collect();
        for group_name in group_names {
            let candidate_notional_limit = self
                .hedge_selection_target(&group_name)
                .unwrap_or(f64::INFINITY);
            for side in [Side::Buy, Side::Sell] {
                levels.clear();
                if let Some(rg) = self.risk_groups.get(&group_name) {
                    for symbol in &rg.symbols {
                        let Some(entity) = self.entities.get(symbol) else {
                            continue;
                        };
                        if !entity.can_take(side) {
                            continue;
                        }
                        let own_quotes = self
                            .active_quotes
                            .iter()
                            .filter(|((quote_symbol, _, _), _)| quote_symbol == symbol)
                            .map(|((_, quote_side, _), quote)| {
                                (*quote_side, quote.price, quote.qty)
                            })
                            .collect::<Vec<_>>();
                        entity.append_hedge_candidates(
                            side,
                            ref_mid,
                            &own_quotes,
                            candidate_notional_limit,
                            &mut levels,
                        );
                    }
                }

                sort_hedge_candidates(side, &mut levels);
                let selected = self.select_required_hedges(&group_name, side, &levels);
                if let Some(rg) = self.risk_groups.get_mut(&group_name) {
                    rg.best_hedges.insert(side, selected);
                }
            }
        }
        levels.clear();
        self.hedge_candidate_scratch = levels;

        if self.risk_groups.len() == 1 {
            if let Some(rg) = self.risk_groups.values().next() {
                self.best_hedges
                    .insert(Side::Buy, rg.best_hedges_for(Side::Buy).to_vec());
                self.best_hedges
                    .insert(Side::Sell, rg.best_hedges_for(Side::Sell).to_vec());
            }
            return;
        }

        for side in [Side::Buy, Side::Sell] {
            let mut levels = Vec::new();
            for rg in self.risk_groups.values() {
                if rg.can_increase_delta_with_quote_buffer(side) {
                    levels.extend_from_slice(rg.best_hedges_for(side));
                }
            }
            sort_hedge_levels(side, &mut levels);
            self.best_hedges.insert(side, levels);
        }
    }

    fn pricing_ready(&self) -> bool {
        self.entities
            .values()
            .all(|entity| entity.book_is_valid_at(self.now_ms) && !entity.feed_stale)
    }

    fn check_basis(&mut self, first_run: bool) -> bool {
        let mut group_names = self.risk_groups.keys().cloned().collect::<Vec<_>>();
        group_names.sort();
        self.basis_breaches.clear();
        for group_name in group_names {
            let Some(group) = self.risk_groups.get(&group_name) else {
                continue;
            };
            let mut max_basis = 0.0;
            let mut max_symbol = String::new();
            for symbol in &group.symbols {
                let Some(entity) = self.entities.get(symbol) else {
                    continue;
                };
                if !entity.config.kind.is_derivative() {
                    continue;
                }
                let basis =
                    (entity.take_rate(Side::Buy) + entity.take_rate(Side::Sell)) * 0.5 - 1.0;
                if basis.is_finite() && basis.abs() > max_basis {
                    max_basis = basis.abs();
                    max_symbol = symbol.clone();
                }
            }
            let limit = if first_run {
                group.config.basis_limit / 3.0
            } else {
                group.config.basis_limit
            };
            let breached = self
                .basis_debouncers
                .entry(group_name.clone())
                .or_default()
                .check(
                    max_basis > limit,
                    self.now_ms,
                    self.config.basis_breach_debounce_ms,
                );
            if breached {
                self.basis_breaches
                    .insert(group_name, (max_symbol, max_basis));
                return false;
            }
        }
        true
    }

    fn check_hedge_availability(&mut self) -> bool {
        let all_halted = self.all_hedges_halted();
        if all_halted {
            let since = self.all_hedges_halted_since.get_or_insert(self.now_ms);
            if self.now_ms.saturating_sub(*since) > self.config.all_hedges_halted_stop_ms {
                self.halt_reason = Some(format!(
                    "all hedge-enabled instruments halted for more than {}ms",
                    self.config.all_hedges_halted_stop_ms
                ));
                return false;
            }
        } else {
            self.all_hedges_halted_since = None;
        }

        let no_hedges = self.best_hedges.get(&Side::Buy).is_none_or(Vec::is_empty)
            && self.best_hedges.get(&Side::Sell).is_none_or(Vec::is_empty);
        if no_hedges && !all_halted {
            let since = self.no_hedge_found_since.get_or_insert(self.now_ms);
            if self.now_ms.saturating_sub(*since) > self.config.no_hedge_stop_ms {
                self.halt_reason = Some(format!(
                    "neither buy nor sell hedge found for more than {}ms",
                    self.config.no_hedge_stop_ms
                ));
                return false;
            }
        } else {
            self.no_hedge_found_since = None;
        }
        true
    }

    fn all_hedges_halted(&self) -> bool {
        self.entities
            .values()
            .filter(|entity| {
                entity.hedge_profit_margin() < 1.0
                    && self
                        .risk_groups
                        .get(&entity.config.risk_group)
                        .is_none_or(|group| group.config.kind != RiskGroupKindConfig::RefOnly)
            })
            .all(|entity| entity.config.halted || entity.interval_halted || entity.system_halted)
    }

    fn select_required_hedges(
        &self,
        group_name: &str,
        _side: Side,
        levels: &[HedgeCandidate],
    ) -> Vec<HedgeLevel> {
        let Some(rg) = self.risk_groups.get(group_name) else {
            return levels.iter().map(HedgeCandidate::to_owned_level).collect();
        };
        let target = self
            .hedge_selection_target(group_name)
            .unwrap_or(f64::INFINITY);

        let mut selected = Vec::new();
        let mut total = 0.0;
        let mut per_symbol: HashMap<&str, f64> = HashMap::new();
        for level in levels {
            total += level.notional_usd;
            *per_symbol.entry(level.symbol.as_ref()).or_default() += level.notional_usd;
            selected.push(level.to_owned_level());
            if total >= target
                && all_symbols_have_hedge(
                    &per_symbol,
                    total,
                    rg.max_quote_size_usd,
                    rg.symbols.len(),
                )
            {
                break;
            }
        }
        selected
    }

    fn hedge_selection_target(&self, group_name: &str) -> Option<f64> {
        let rg = self.risk_groups.get(group_name)?;
        let hedge_required_for_quote = rg
            .symbols
            .iter()
            .filter_map(|symbol| self.entities.get(symbol))
            .map(|entity| entity.config.max_order_size_usd)
            .sum::<f64>()
            * 2.0;
        let min_hedge_usd = self.config.delta_limit_usd.min(hedge_required_for_quote);
        let delta_need =
            self.delta_usd.abs().max(self.pending_delta_usd.abs()) * HEDGE_VOL_TO_DELTA_RATIO;
        Some(min_hedge_usd.max(delta_need))
    }

    fn update_theo_quotes(&mut self) {
        let group_names: Vec<_> = self.risk_groups.keys().cloned().collect();
        for group_name in group_names {
            for side in [Side::Buy, Side::Sell] {
                let Some(rg) = self.risk_groups.get(&group_name) else {
                    continue;
                };
                let symbols: Vec<_> = rg.symbols.iter().cloned().collect();
                if !rg.can_quote_side(side)
                    || rg.live_order_size_usd
                        > LIVE_ORDER_STOP_QUOTE_THRESHOLD * rg.config.live_order_limit_usd
                {
                    for symbol in symbols {
                        if let Some(entity) = self.entities.get_mut(&symbol) {
                            entity.clear_theo(side);
                        }
                    }
                    continue;
                }

                let hedge_side = side.reverse();
                let hedges = if rg.can_increase_delta_with_quote_buffer(side) {
                    self.best_hedges
                        .get(&hedge_side)
                        .cloned()
                        .unwrap_or_default()
                } else {
                    rg.best_hedges_for(hedge_side).to_vec()
                };

                for symbol in symbols {
                    self.update_theo_for_symbol(&symbol, side, &hedges);
                }
            }
        }

        for entity in self.entities.values_mut() {
            entity.prevent_self_cross();
        }
    }

    fn update_theo_for_symbol(&mut self, symbol: &str, side: Side, hedges: &[HedgeLevel]) {
        let quote = self.calculate_theo_for_symbol(symbol, side, hedges);
        if let Some(entity) = self.entities.get_mut(symbol) {
            match quote {
                Some(quote) => entity.set_theo(side, quote),
                None => entity.clear_theo(side),
            }
        }
    }

    fn calculate_theo_for_symbol(
        &self,
        symbol: &str,
        side: Side,
        hedges: &[HedgeLevel],
    ) -> Option<TheoQuote> {
        let ref_mid = self.ref_mid()?;
        let pricing_entity = self.entities.get(symbol)?;
        if pricing_entity.quote_profit_margin() >= 1.0 {
            return None;
        }

        let mut px_by_hedge = 0.0;
        let mut current_size_usd = 0.0;
        let mut hedge_notional_by_symbol: HashMap<Symbol, f64> = HashMap::new();
        let mut weighted_hedge_px_by_symbol: HashMap<Symbol, (f64, f64)> = HashMap::new();
        let mut best_hedge_symbol = String::new();

        for hedge_level in hedges {
            if hedge_level.symbol == symbol {
                continue;
            }
            let Some(hedge_entity) = self.entities.get(&hedge_level.symbol) else {
                continue;
            };
            if best_hedge_symbol.is_empty() {
                best_hedge_symbol = hedge_level.symbol.clone();
            }

            let current_px =
                price_by_hedge(side, pricing_entity, hedge_entity, hedge_level, ref_mid);
            if !current_px.is_finite() || current_px <= 0.0 {
                continue;
            }

            let last_size_usd = current_size_usd;
            current_size_usd += hedge_level.notional_usd;

            if current_size_usd < pricing_entity.config.min_order_size_usd {
                px_by_hedge = weighted_avg(
                    px_by_hedge,
                    last_size_usd,
                    current_px,
                    hedge_level.notional_usd,
                );
                *hedge_notional_by_symbol
                    .entry(hedge_level.symbol.clone())
                    .or_default() += hedge_level.notional_usd;
                update_weighted_px(
                    &mut weighted_hedge_px_by_symbol,
                    &hedge_level.symbol,
                    hedge_level.px,
                    hedge_level.notional_usd,
                );
                continue;
            }

            let (this_size_usd, quote_size_usd) =
                if current_size_usd > pricing_entity.config.max_order_size_usd {
                    (
                        pricing_entity.config.max_order_size_usd - last_size_usd,
                        pricing_entity.config.max_order_size_usd,
                    )
                } else {
                    (hedge_level.notional_usd, current_size_usd)
                };
            if this_size_usd <= 0.0 {
                break;
            }

            let px = weighted_avg(px_by_hedge, last_size_usd, current_px, this_size_usd);
            *hedge_notional_by_symbol
                .entry(hedge_level.symbol.clone())
                .or_default() += this_size_usd;
            *hedge_notional_by_symbol
                .entry(symbol.to_string())
                .or_default() += quote_size_usd;
            update_weighted_px(
                &mut weighted_hedge_px_by_symbol,
                &hedge_level.symbol,
                hedge_level.px,
                this_size_usd,
            );

            let pos_skew_adj =
                self.calculate_total_pos_skew_adj(side, &hedge_notional_by_symbol, symbol);
            let burst_adj = if self.config.act_on_burst && side.factor() * self.burst < 0.0 {
                self.burst * ref_mid
            } else {
                0.0
            };
            let raw_px = px + pos_skew_adj + burst_adj;
            let passive_px = side.passive_price(
                raw_px,
                pricing_entity.opposite_touch(side),
                pricing_entity.config.tick_size,
            );
            let quote_px = pricing_entity.px_within_limit(side, passive_px);
            let quote_qty = pricing_entity.quote_qty_from_usd(quote_size_usd, ref_mid);

            return Some(TheoQuote {
                price: quote_px,
                qty: quote_qty,
                hedge_symbol: best_hedge_symbol,
                hedge_px: single_weighted_px(weighted_hedge_px_by_symbol),
            });
        }

        None
    }

    fn calculate_total_pos_skew_adj(
        &self,
        side: Side,
        notionals: &HashMap<Symbol, f64>,
        pricing_symbol: &str,
    ) -> f64 {
        let Some(ref_mid) = self.ref_mid() else {
            return 0.0;
        };
        let mut total = 0.0;
        for (symbol, usd) in notionals {
            let Some(entity) = self.entities.get(symbol) else {
                continue;
            };
            let size = entity.size_from_usd(*usd, ref_mid);
            let is_hedge = symbol != pricing_symbol;
            let end_pos = if is_hedge {
                entity.inventory_position() - side.factor() * size
            } else {
                entity.inventory_position() + side.factor() * size
            };
            let adj_size = if entity.config.kind.is_derivative() && size >= 2.0 {
                (size + 1.0) * 0.5
            } else {
                size * 0.5
            };
            total += -side.factor() * entity.average_skew_rate_to(end_pos) * adj_size * ref_mid;
        }
        total
    }

    fn should_hedge_strategy_delta(&self) -> bool {
        if self.config.master_strategy.is_some() {
            return false;
        }
        if self.now_ms < self.last_hedge_ms + self.config.min_hedge_interval_ms {
            return false;
        }
        self.delta_to_hedge().abs() >= self.config.active_hedge_threshold_usd
    }

    fn hedge_delta(
        &mut self,
        delta_to_hedge: f64,
        source_symbol: Option<&str>,
        strategy_delta_hedge: bool,
    ) -> Vec<OrderIntent> {
        if self.config.master_strategy.is_some() {
            return Vec::new();
        }
        self.update_risk();
        self.update_best_hedges();

        let hedge_side = if delta_to_hedge > 0.0 {
            Side::Sell
        } else {
            Side::Buy
        };
        let group_name = source_symbol
            .and_then(|symbol| self.symbol_to_group.get(symbol))
            .cloned();
        let mut targets = Vec::new();
        if let Some(group_name) = group_name
            && let Some(rg) = self.risk_groups.get(&group_name)
            && rg.must_hedge_within_group(delta_to_hedge)
        {
            targets = self.summarize_hedges(
                rg.best_hedges_for(hedge_side),
                hedge_side,
                delta_to_hedge.abs(),
                source_symbol,
            );
        }
        if targets.is_empty() {
            let hedges = self
                .best_hedges
                .get(&hedge_side)
                .map(Vec::as_slice)
                .unwrap_or(&[]);
            targets =
                self.summarize_hedges(hedges, hedge_side, delta_to_hedge.abs(), source_symbol);
        }

        if targets.is_empty() {
            if !self.all_hedges_halted() {
                let since = self.hedge_not_found_since.get_or_insert(self.now_ms);
                if self.now_ms.saturating_sub(*since) > self.config.hedge_not_found_stop_ms {
                    self.halt_reason = Some(format!(
                        "delta hedge unavailable for more than {}ms",
                        self.config.hedge_not_found_stop_ms
                    ));
                }
            }
            return Vec::new();
        }
        self.hedge_not_found_since = None;

        if strategy_delta_hedge {
            self.last_hedge_ms = self.now_ms;
        }
        let source_label = source_symbol.unwrap_or("timer");
        targets
            .into_iter()
            .filter_map(|target| {
                let entity = self.entities.get(&target.symbol)?;
                if target.qty < entity.config.min_trade_size {
                    return None;
                }
                Some(OrderIntent::NewOrder(NewOrder {
                    symbol: target.symbol,
                    side: hedge_side,
                    qty: target.qty,
                    price: target.hedge_px,
                    time_in_force: TimeInForce::Ioc,
                    reduce_only: false,
                    self_trade_prevention: Some(SelfTradePrevention::CancelMaker),
                    reason: format!("hedge:{}:{}", source_label, target.orig_px),
                }))
            })
            .collect()
    }

    fn delta_to_hedge(&self) -> f64 {
        if self.pending_delta_usd * self.delta_usd < 0.0 {
            return 0.0;
        }
        if self.delta_usd > 0.0 && self.pending_delta_usd > 0.0 {
            self.delta_usd.min(self.pending_delta_usd)
        } else {
            self.delta_usd.max(self.pending_delta_usd)
        }
    }

    fn summarize_hedges(
        &self,
        hedges: &[HedgeLevel],
        hedge_side: Side,
        usd_amt: f64,
        exclude_symbol: Option<&str>,
    ) -> Vec<HedgeTarget> {
        let mut out: HashMap<Symbol, HedgeTarget> = HashMap::new();
        let mut total = 0.0;

        for level in hedges {
            if exclude_symbol.is_some_and(|symbol| symbol == level.symbol) {
                continue;
            }
            if total >= usd_amt {
                break;
            }
            let Some(entity) = self.entities.get(&level.symbol) else {
                continue;
            };
            let pending = self
                .active_hedges
                .values()
                .filter(|hedge| {
                    hedge.symbol == level.symbol
                        && hedge.signed_open_qty.signum() == hedge_side.factor()
                })
                .collect::<Vec<_>>();
            if pending
                .iter()
                .any(|hedge| hedge_side.is_more_passive(level.px, hedge.reference_price))
            {
                continue;
            }
            let pending_qty_at_level = pending
                .iter()
                .filter(|hedge| approx_eq(level.px, hedge.reference_price))
                .map(|hedge| hedge.signed_open_qty.abs())
                .sum::<f64>();
            let available_level_qty = (level.qty - pending_qty_at_level).max(0.0);
            if available_level_qty <= 0.0 {
                continue;
            }
            let available_level_notional =
                level.notional_usd * available_level_qty / level.qty.max(EPS);
            let gap = usd_amt - total;
            let use_notional = available_level_notional.min(gap);
            let qty = if available_level_notional > gap {
                round_down_to_lot(
                    available_level_qty * gap / available_level_notional,
                    entity.config.lot_size,
                )
            } else {
                round_down_to_lot(available_level_qty, entity.config.lot_size)
            };
            if qty <= 0.0 {
                continue;
            }
            let notional = if available_level_qty > 0.0 {
                qty * available_level_notional / available_level_qty
            } else {
                use_notional
            }
            .min(use_notional);
            let hedge_aggression =
                if self.config.act_on_burst && hedge_side.factor() * self.burst > 0.0 {
                    entity.hedge_aggression().max(self.burst.abs())
                } else {
                    entity.hedge_aggression()
                };
            let hedge_px = entity.hedge_px(hedge_side, level.px, hedge_aggression);
            out.entry(level.symbol.clone())
                .and_modify(|target| {
                    target.qty += qty;
                    target.notional_usd += notional;
                    target.orig_px = level.px;
                    target.hedge_px = hedge_px;
                    target.cur_level_acc_qty = qty;
                })
                .or_insert_with(|| HedgeTarget {
                    symbol: level.symbol.clone(),
                    orig_px: level.px,
                    hedge_px,
                    qty,
                    cur_level_acc_qty: qty,
                    notional_usd: notional,
                });
            total += use_notional;
        }

        out.into_values().collect()
    }

    fn ref_mid(&self) -> Option<f64> {
        self.entities
            .get(&self.config.ref_symbol)
            .and_then(InstrumentState::mid)
    }
}

impl ChaosStrategy {
    fn on_market_event(&mut self, event: &MarketEvent) -> Vec<OrderIntent> {
        match event {
            MarketEvent::Depth(book) => self.on_depth(book),
            MarketEvent::Trade { ts_ms, .. } => {
                self.now_ms = *ts_ms;
                Vec::new()
            }
            MarketEvent::IndexPrice {
                ts_ms,
                symbol,
                price,
            } => {
                self.now_ms = *ts_ms;
                if !self.index_symbols.contains(symbol) {
                    return Vec::new();
                }
                if price.is_finite() && *price > 0.0 {
                    self.index_prices.insert(symbol.clone(), *price);
                }
                self.refresh_quotes()
            }
            MarketEvent::FundingRate {
                ts_ms,
                symbol,
                rate,
                funding_time_ms,
            } => {
                self.now_ms = *ts_ms;
                if let Some(entity) = self.entities.get_mut(symbol)
                    && rate.is_finite()
                {
                    entity.funding_rate = *rate;
                    entity.funding_time_ms = *funding_time_ms;
                }
                self.refresh_quotes()
            }
            MarketEvent::BurstSignal {
                ts_ms,
                symbol,
                value,
            } => {
                self.now_ms = *ts_ms;
                let mut should_reprice = false;
                if *value == 0.0 {
                    if self.burst_symbol.as_deref() == Some(symbol) {
                        self.burst = 0.0;
                        self.burst_symbol = None;
                    }
                } else if self.burst == 0.0 {
                    self.burst = *value;
                    self.burst_symbol = Some(symbol.clone());
                    should_reprice = true;
                } else if self.burst * value < 0.0 {
                    self.burst = 0.0;
                    self.burst_symbol = None;
                } else if value.abs() > self.burst.abs() {
                    self.burst = *value;
                    self.burst_symbol = Some(symbol.clone());
                    should_reprice = true;
                }

                if should_reprice && self.config.act_on_burst {
                    self.refresh_quotes()
                } else {
                    Vec::new()
                }
            }
            MarketEvent::PriceLimits {
                ts_ms,
                symbol,
                mark_price,
                limit_down,
                limit_up,
            } => {
                self.now_ms = *ts_ms;
                if let Some(entity) = self.entities.get_mut(symbol) {
                    if mark_price.is_finite() && *mark_price > 0.0 {
                        entity.mark_price = Some(*mark_price);
                    }
                    if limit_down.is_finite() && *limit_down > 0.0 {
                        entity.limit_down = Some(*limit_down);
                    }
                    if limit_up.is_finite() && *limit_up > 0.0 {
                        entity.limit_up = Some(*limit_up);
                    }
                }
                self.refresh_quotes()
            }
        }
    }

    fn on_order_update(&mut self, update: &OrderUpdate) -> Vec<OrderIntent> {
        self.now_ms = update.ts_ms;
        if update.event == OrderEvent::Cancelled && update.reason.starts_with("hedge") {
            let missed_qty = (update.qty - update.filled_qty).max(0.0);
            if missed_qty > 0.0
                && let (Some(entity), Some(ref_mid)) =
                    (self.entities.get(&update.symbol), self.ref_mid())
            {
                self.missed_hedges.push(MissedHedge {
                    ts_ms: update.ts_ms,
                    order_id: update.order_id.clone(),
                    symbol: update.symbol.clone(),
                    side: update.side,
                    price: update.price,
                    missed_qty,
                    missed_delta_usd: entity
                        .delta_coin_for_qty(update.side.factor() * missed_qty, update.price)
                        * ref_mid,
                    reference_symbol: update.reason.split(':').nth(1).map(str::to_string),
                });
                if self.missed_hedges.len() > 4_096 {
                    self.missed_hedges.remove(0);
                }
            }
        }
        if update.reason.starts_with("hedge") {
            if matches!(update.event, OrderEvent::New | OrderEvent::PartialFill)
                && update.open_qty > 0.0
            {
                self.active_hedges.insert(
                    update.order_id.clone(),
                    ActiveHedge {
                        symbol: update.symbol.clone(),
                        signed_open_qty: update.side.factor() * update.open_qty,
                        price: update.price,
                        reference_price: hedge_reference_price(&update.reason)
                            .unwrap_or(update.price),
                        updated_ms: update.ts_ms,
                    },
                );
            } else if matches!(
                update.event,
                OrderEvent::Cancelled | OrderEvent::FullyFilled | OrderEvent::Rejected
            ) {
                self.active_hedges.remove(&update.order_id);
            }
        }
        match update.event {
            OrderEvent::New if update.reason.starts_with("quote") => {
                let level = quote_level_from_reason(&update.reason);
                self.active_quotes.insert(
                    (update.symbol.clone(), update.side, level),
                    ActiveQuote {
                        order_id: update.order_id.clone(),
                        price: update.price,
                        qty: update.open_qty,
                    },
                );
            }
            OrderEvent::PartialFill if update.reason.starts_with("quote") => {
                if let Some(active) = self
                    .active_quotes
                    .values_mut()
                    .find(|quote| quote.order_id == update.order_id)
                {
                    active.qty = update.open_qty;
                }
            }
            OrderEvent::Cancelled | OrderEvent::FullyFilled | OrderEvent::Rejected => {
                self.active_quotes
                    .retain(|_, quote| quote.order_id != update.order_id);
            }
            _ => {}
        }

        if update.has_fill() && update.reason.starts_with("quote") {
            self.last_quote_fill_ms
                .insert((update.symbol.clone(), update.side), self.now_ms);
        }

        if update.has_fill() {
            if let Some(ref_mid) = self.ref_mid()
                && let Some(entity) = self.entities.get(&update.symbol)
            {
                let turnover =
                    entity.notional_usd(update.last_fill_qty, update.last_fill_price, ref_mid);
                self.net_filled_delta_usd += entity.delta_coin_for_qty(
                    update.side.factor() * update.last_fill_qty,
                    update.last_fill_price,
                ) * ref_mid;
                *self
                    .turnover_by_group
                    .entry(entity.config.risk_group.clone())
                    .or_default() += turnover;
            }
            if let Some(entity) = self.entities.get_mut(&update.symbol) {
                entity.record_fill(
                    update.side,
                    update.last_fill_qty,
                    update.last_fill_price,
                    update.last_fill_liquidity,
                );
                if !update.reason.starts_with("hedge")
                    && entity.anomalous_fill_should_stop(
                        update.ts_ms,
                        update.side,
                        update.price,
                        update.last_fill_price,
                    )
                {
                    self.halt_reason = Some(format!(
                        "{} received consecutive anomalous fills",
                        update.symbol
                    ));
                }
            }
            self.update_risk();
            return Vec::new();
        }

        Vec::new()
    }

    fn on_account_update(&mut self, update: &AccountUpdate) -> Vec<OrderIntent> {
        self.now_ms = update.ts_ms;
        self.update_risk();
        let old_delta = self.delta_usd;
        let mut source_symbol = None;

        for position in &update.positions {
            if let Some(entity) = self.entities.get_mut(&position.symbol) {
                entity.position_qty = position.qty;
                entity.position_avg_price = position.avg_price;
                source_symbol = Some(position.symbol.clone());
            }
        }

        for balance in &update.balances {
            let equity = if balance.equity == 0.0 && balance.total != 0.0 {
                balance.total
            } else {
                balance.equity
            };
            for entity in self.entities.values_mut() {
                let Some(group) = self.risk_groups.get(&entity.config.risk_group) else {
                    continue;
                };
                if group.config.account_id.is_some()
                    && group.config.account_id != balance.account_id
                {
                    continue;
                }
                if entity.config.kind.is_spot() {
                    if balance.currency == entity.config.base_currency {
                        entity.base_balance = balance.total;
                        entity.base_available = balance.available;
                        entity.base_equity = equity;
                        entity.base_liability = balance.liability;
                        entity.base_max_loan = balance.max_loan;
                        entity.balances_initialized = true;
                        source_symbol.get_or_insert_with(|| entity.config.symbol.clone());
                    }
                    if balance.currency == entity.config.quote_currency {
                        entity.quote_balance = balance.total;
                        entity.quote_available = balance.available;
                        entity.quote_equity = equity;
                        entity.quote_liability = balance.liability;
                        entity.quote_max_loan = balance.max_loan;
                        entity.balances_initialized = true;
                    }
                } else if balance.currency == entity.config.settle_currency {
                    entity.margin_balance = balance.total;
                    entity.margin_available = balance.available;
                    entity.margin_equity = equity;
                    entity.margin_liability = balance.liability;
                    entity.margin_max_loan = balance.max_loan;
                    entity.margin_initialized = true;
                }
            }

            for group in self.risk_groups.values_mut() {
                if group.config.account_id.is_some()
                    && group.config.account_id != balance.account_id
                {
                    continue;
                }
                if group
                    .config
                    .coins
                    .iter()
                    .any(|coin| coin.currency == balance.currency)
                {
                    group.account_balances.insert(
                        balance.currency.clone(),
                        AccountBalanceState {
                            cash: balance.total,
                            liability: balance.liability,
                        },
                    );
                }
            }
        }

        for margin in &update.margins {
            for group in self.risk_groups.values_mut() {
                if group.config.account_id.is_none() || group.config.account_id == margin.account_id
                {
                    if let Some(ratio) = margin.ratio
                        && ratio.is_finite()
                    {
                        group.margin_ratio = Some(ratio);
                    }
                    if let Some(ratio) = margin.exchange_ratio {
                        group.exchange_margin_ratio = Some(ratio);
                    }
                    if let Some(adjusted_equity_usd) = margin.adjusted_equity_usd {
                        group.margin_adjusted_equity_usd = Some(adjusted_equity_usd);
                    }
                    if let Some(notional_usd) = margin.notional_usd {
                        group.margin_notional_usd = Some(notional_usd);
                    }
                }
            }
        }

        let mut intents = self.refresh_quotes();
        let delta_change = self.delta_usd - old_delta;
        if self.halt_reason.is_none()
            && delta_change.abs() > ORDER_CHECK_DELTA_THRESHOLD_USD
            && delta_change.signum() == self.delta_usd.signum()
            && delta_change.signum() == self.pending_delta_usd.signum()
            && let Some(source_symbol) = source_symbol
        {
            let (delta_to_hedge, strategy_delta_hedge) = if self.should_hedge_strategy_delta() {
                (self.delta_to_hedge(), true)
            } else if delta_change > 0.0 {
                (
                    delta_change.min(self.pending_delta_usd.min(self.delta_usd)),
                    false,
                )
            } else {
                (
                    delta_change.max(self.pending_delta_usd.max(self.delta_usd)),
                    false,
                )
            };
            if delta_to_hedge.abs() > self.config.active_hedge_threshold_usd {
                intents.extend(self.hedge_delta(
                    delta_to_hedge,
                    Some(&source_symbol),
                    strategy_delta_hedge,
                ));
            }
        }
        intents
    }

    fn on_system_event(&mut self, event: &SystemEvent) -> Vec<OrderIntent> {
        self.now_ms = event.ts_ms;
        if event.kind == SystemEventKind::AccountHalted {
            let Some(account_id) = event.account_id.as_deref() else {
                return Vec::new();
            };
            let affected_groups = self
                .risk_groups
                .iter()
                .filter(|(_, group)| group.config.account_id.as_deref() == Some(account_id))
                .map(|(name, _)| name.clone())
                .collect::<HashSet<_>>();
            let mut changed = false;
            for entity in self.entities.values_mut() {
                if affected_groups.contains(&entity.config.risk_group) {
                    changed |= update_flag(&mut entity.system_halted, true);
                }
            }
            return if changed {
                self.refresh_quotes()
            } else {
                Vec::new()
            };
        }
        let Some(symbol) = event.symbol.as_deref() else {
            return Vec::new();
        };
        let Some(entity) = self.entities.get_mut(symbol) else {
            return Vec::new();
        };
        let changed = match event.kind {
            SystemEventKind::SymbolHalted => update_flag(&mut entity.system_halted, true),
            SystemEventKind::SymbolResumed => update_flag(&mut entity.system_halted, false),
            SystemEventKind::FeedStale
            | SystemEventKind::FeedGap
            | SystemEventKind::BookRecoveryStarted
            | SystemEventKind::BookRecoveryFailed => update_flag(&mut entity.feed_stale, true),
            SystemEventKind::FeedHeartbeat | SystemEventKind::FeedRecovered => {
                entity.feed_stale = false;
                return Vec::new();
            }
            _ => return Vec::new(),
        };
        if !changed {
            return Vec::new();
        }
        self.refresh_quotes()
    }
}

impl Strategy for ChaosStrategy {
    fn on_event(&mut self, event: &StrategyEvent) -> Vec<OrderIntent> {
        match event {
            StrategyEvent::Market(market) => self.on_market_event(market),
            StrategyEvent::Order(update) => self.on_order_update(update),
            StrategyEvent::Timer(timer) => {
                self.now_ms = timer.ts_ms;
                let mut intents = self.refresh_quotes();
                if self.halt_reason.is_none() && self.should_hedge_strategy_delta() {
                    intents.extend(self.hedge_delta(self.delta_to_hedge(), None, true));
                }
                intents
            }
            StrategyEvent::Account(update) => self.on_account_update(update),
            StrategyEvent::System(event) => self.on_system_event(event),
            StrategyEvent::Control(_) => Vec::new(),
        }
    }

    fn on_owned_event(&mut self, event: StrategyEvent) -> Vec<OrderIntent> {
        match event {
            StrategyEvent::Market(MarketEvent::Depth(book)) => self.on_owned_depth(book),
            event => self.on_event(&event),
        }
    }
}

#[derive(Debug, Clone)]
pub struct InstrumentState {
    pub config: InstrumentConfig,
    symbol_key: Arc<str>,
    pub book: Option<OrderBook>,
    pub position_qty: Quantity,
    pub position_avg_price: Price,
    buy_fill_qty: Quantity,
    buy_fill_notional: f64,
    sell_fill_qty: Quantity,
    sell_fill_notional: f64,
    pub funding_rate: f64,
    pub funding_time_ms: TimeMs,
    funding_rate_active: bool,
    pub mark_price: Option<Price>,
    pub limit_down: Option<Price>,
    pub limit_up: Option<Price>,
    pub base_balance: Quantity,
    pub base_available: Quantity,
    pub base_equity: Quantity,
    pub base_liability: Quantity,
    pub base_max_loan: Quantity,
    pub quote_balance: Quantity,
    pub quote_available: Quantity,
    pub quote_equity: Quantity,
    pub quote_liability: Quantity,
    pub quote_max_loan: Quantity,
    pub balances_initialized: bool,
    pub margin_balance: Quantity,
    pub margin_available: Quantity,
    pub margin_equity: Quantity,
    pub margin_liability: Quantity,
    pub margin_max_loan: Quantity,
    pub margin_initialized: bool,
    ignore_best_level: bool,
    interval_halted: bool,
    system_halted: bool,
    feed_stale: bool,
    base_coin_config: Option<CoinConfig>,
    quote_coin_config: Option<CoinConfig>,
    margin_coin_config: Option<CoinConfig>,
    can_trade: HashMap<Side, bool>,
    can_trade_debouncers: HashMap<Side, DebouncedCondition>,
    reduced_quote_level_side: Option<Side>,
    full_quote_balance_debouncer: DebouncedCondition,
    take_buy_rate: f64,
    take_sell_rate: f64,
    last_aggressive_fill_ms: TimeMs,
    last_normal_fill_ms: TimeMs,
    aggressive_fill_count: u32,
    pub buy_theo: Option<TheoQuote>,
    pub sell_theo: Option<TheoQuote>,
}

impl InstrumentState {
    fn new(config: InstrumentConfig) -> Self {
        let funding_rate = config.funding_rate;
        let symbol_key = Arc::<str>::from(config.symbol.as_str());
        Self {
            config,
            symbol_key,
            book: None,
            position_qty: 0.0,
            position_avg_price: 0.0,
            buy_fill_qty: 0.0,
            buy_fill_notional: 0.0,
            sell_fill_qty: 0.0,
            sell_fill_notional: 0.0,
            funding_rate,
            funding_time_ms: 0,
            funding_rate_active: true,
            mark_price: None,
            limit_down: None,
            limit_up: None,
            base_balance: 0.0,
            base_available: 0.0,
            base_equity: 0.0,
            base_liability: 0.0,
            base_max_loan: 0.0,
            quote_balance: 0.0,
            quote_available: 0.0,
            quote_equity: 0.0,
            quote_liability: 0.0,
            quote_max_loan: 0.0,
            balances_initialized: false,
            margin_balance: 0.0,
            margin_available: 0.0,
            margin_equity: 0.0,
            margin_liability: 0.0,
            margin_max_loan: 0.0,
            margin_initialized: false,
            ignore_best_level: false,
            interval_halted: false,
            system_halted: false,
            feed_stale: false,
            base_coin_config: None,
            quote_coin_config: None,
            margin_coin_config: None,
            can_trade: HashMap::from([(Side::Buy, false), (Side::Sell, false)]),
            can_trade_debouncers: HashMap::from([
                (Side::Buy, DebouncedCondition::default()),
                (Side::Sell, DebouncedCondition::default()),
            ]),
            reduced_quote_level_side: None,
            full_quote_balance_debouncer: DebouncedCondition::default(),
            take_buy_rate: f64::NAN,
            take_sell_rate: f64::NAN,
            last_aggressive_fill_ms: 0,
            last_normal_fill_ms: 0,
            aggressive_fill_count: 0,
            buy_theo: None,
            sell_theo: None,
        }
    }

    pub fn theo(&self, side: Side) -> Option<TheoQuote> {
        match side {
            Side::Buy => self.buy_theo.clone(),
            Side::Sell => self.sell_theo.clone(),
        }
    }

    fn set_theo(&mut self, side: Side, quote: TheoQuote) {
        match side {
            Side::Buy => self.buy_theo = Some(quote),
            Side::Sell => self.sell_theo = Some(quote),
        }
    }

    fn clear_theo(&mut self, side: Side) {
        match side {
            Side::Buy => self.buy_theo = None,
            Side::Sell => self.sell_theo = None,
        }
    }

    fn prevent_self_cross(&mut self) {
        let (Some(mut buy), Some(mut sell)) = (self.buy_theo.clone(), self.sell_theo.clone())
        else {
            return;
        };
        if buy.price < sell.price {
            return;
        }
        let Some(mid) = self.mid() else {
            self.buy_theo = None;
            self.sell_theo = None;
            return;
        };
        let margin = self.quote_profit_margin();
        buy.price = buy.price.min(mid * (1.0 - margin));
        sell.price = sell.price.max(mid * (1.0 + margin));
        self.buy_theo = Some(buy);
        self.sell_theo = Some(sell);
    }

    fn refresh_trade_permissions(&mut self, now_ms: TimeMs) {
        for side in [Side::Buy, Side::Sell] {
            if !self.is_valid_and_not_halted(now_ms) {
                self.can_trade.insert(side, false);
                continue;
            }
            let raw = if self.config.kind.is_spot() {
                self.spot_can_trade_raw(side, now_ms)
            } else {
                self.max_trade_size(side, false) >= self.config.min_trade_size
            };
            let allowed = self.can_trade_debouncers.entry(side).or_default().check(
                raw,
                now_ms,
                CAN_TRADE_DEBOUNCE_MS,
            );
            self.can_trade.insert(side, allowed);
        }
    }

    fn is_valid_and_not_halted(&self, now_ms: TimeMs) -> bool {
        !self.config.halted
            && !self.interval_halted
            && !self.system_halted
            && !self.feed_stale
            && self.book_is_valid_at(now_ms)
    }

    fn book_is_valid(&self) -> bool {
        let Some(book) = &self.book else {
            return false;
        };
        let (Some(bid), Some(ask)) = (book.best_bid(), book.best_ask()) else {
            return false;
        };
        bid.px.is_finite()
            && ask.px.is_finite()
            && bid.qty.is_finite()
            && ask.qty.is_finite()
            && bid.px > 0.0
            && ask.px > bid.px
            && bid.qty > 0.0
            && ask.qty > 0.0
    }

    fn book_is_valid_at(&self, now_ms: TimeMs) -> bool {
        self.book_is_valid()
            && self.book.as_ref().is_some_and(|book| {
                now_ms.saturating_sub(book.ts_ms) <= self.config.depth_stale_threshold_ms
            })
    }

    fn spot_can_trade_raw(&mut self, side: Side, now_ms: TimeMs) -> bool {
        if self.max_trade_size(side, false) < self.config.min_trade_size {
            return false;
        }
        if !self.balances_initialized {
            return true;
        }
        let (Some(base), Some(quote), Some(mid)) = (
            self.base_coin_config.clone(),
            self.quote_coin_config.clone(),
            self.mid(),
        ) else {
            return true;
        };
        let full_levels = self.config.num_quote_levels;
        match side {
            Side::Buy => {
                let full_sufficient = self.quote_balance_sufficient(
                    self.config.max_order_size,
                    full_levels,
                    base.borrow_limit(mid),
                    1.0,
                    quote.min_balance,
                );
                if !full_sufficient {
                    self.reduced_quote_level_side = Some(side);
                    if !self.quote_balance_sufficient(
                        self.config.max_order_size,
                        1,
                        quote.borrow_limit(1.0),
                        quote.safety_multiplier,
                        quote.min_balance,
                    ) {
                        return false;
                    }
                } else if self.reduced_quote_level_side == Some(side) {
                    let can_restore = self.quote_balance_sufficient(
                        self.config.max_order_size,
                        full_levels,
                        base.borrow_limit(mid),
                        2.0,
                        quote.min_balance,
                    );
                    if self.full_quote_balance_debouncer.check(
                        can_restore,
                        now_ms,
                        CAN_QUOTE_FULL_SIZE_DEBOUNCE_MS,
                    ) {
                        self.reduced_quote_level_side = None;
                    }
                }
                self.quote_balance >= quote.min_balance
            }
            Side::Sell => {
                let safety = if self.config.quote_currency == "USDC" {
                    base.safety_multiplier.max(10.0)
                } else {
                    base.safety_multiplier
                };
                let full_sufficient = self.trade_balance_sufficient(
                    self.config.max_order_size,
                    full_levels,
                    base.borrow_limit(mid),
                    safety,
                    base.min_balance,
                );
                if !full_sufficient {
                    self.reduced_quote_level_side = Some(side);
                    if !self.trade_balance_sufficient(
                        self.config.max_order_size,
                        1,
                        base.borrow_limit(mid),
                        safety,
                        base.min_balance,
                    ) {
                        return false;
                    }
                } else if self.reduced_quote_level_side == Some(side) {
                    let can_restore = self.trade_balance_sufficient(
                        self.config.max_order_size,
                        full_levels,
                        base.borrow_limit(mid),
                        2.0,
                        base.min_balance,
                    );
                    if self.full_quote_balance_debouncer.check(
                        can_restore,
                        now_ms,
                        CAN_QUOTE_FULL_SIZE_DEBOUNCE_MS,
                    ) {
                        self.reduced_quote_level_side = None;
                    }
                }
                quote.max_balance <= 0.0 || self.quote_balance < quote.max_balance
            }
        }
    }

    fn trade_balance_sufficient(
        &self,
        max_order_size: f64,
        levels: usize,
        max_borrow: f64,
        safety_multiplier: f64,
        minimum: f64,
    ) -> bool {
        if self.base_balance < -max_borrow || self.base_liability.abs() > max_borrow {
            return false;
        }
        available_coin_qty(self.base_balance, self.base_max_loan, max_borrow, minimum)
            > max_order_size * levels as f64 * safety_multiplier
    }

    fn quote_balance_sufficient(
        &self,
        max_order_size: f64,
        levels: usize,
        max_borrow: f64,
        safety_multiplier: f64,
        minimum: f64,
    ) -> bool {
        if self.quote_balance < -max_borrow || self.quote_liability.abs() > max_borrow {
            return false;
        }
        let Some(ask) = self.effective_levels(Side::Sell).first() else {
            return false;
        };
        available_coin_qty(self.quote_balance, self.quote_max_loan, max_borrow, minimum) / ask.px
            > max_order_size * levels as f64 * safety_multiplier
    }

    fn quote_level_count(&self, side: Side) -> usize {
        if self.reduced_quote_level_side == Some(side) {
            1
        } else {
            self.config.num_quote_levels
        }
    }

    fn can_quote(&self, side: Side) -> bool {
        self.quote_profit_margin() < 1.0 && self.can_trade.get(&side).copied().unwrap_or(false)
    }

    fn can_take(&self, side: Side) -> bool {
        self.hedge_profit_margin() < 1.0
            && self.can_trade.get(&side).copied().unwrap_or(false)
            && self.can_take_within_price_limit(side)
    }

    fn update_take_rate(&mut self, side: Side, ref_mid: f64) {
        let rate = if self.can_trade.get(&side).copied().unwrap_or(false) {
            self.effective_levels(side.reverse())
                .first()
                .map(|level| {
                    let profit_margin = if self.hedge_profit_margin() < 1.0 {
                        self.hedge_profit_margin()
                    } else if self.quote_profit_margin() < 1.0 {
                        self.quote_profit_margin()
                    } else {
                        0.0
                    };
                    level.px / ref_mid + side.factor() * (self.config.taker_fee + profit_margin)
                        - self.fv_adjust()
                })
                .unwrap_or(f64::NAN)
        } else {
            f64::NAN
        };
        match side {
            Side::Buy => self.take_buy_rate = rate,
            Side::Sell => self.take_sell_rate = rate,
        }
    }

    fn take_rate(&self, side: Side) -> f64 {
        match side {
            Side::Buy => self.take_buy_rate,
            Side::Sell => self.take_sell_rate,
        }
    }

    fn max_trade_size(&self, side: Side, is_hedge: bool) -> f64 {
        let size_limit = self.config.max_order_size * if is_hedge { 10.0 } else { 1.0 };
        if self.config.kind.is_spot()
            && self.balances_initialized
            && let (Some(base), Some(quote)) = (&self.base_coin_config, &self.quote_coin_config)
        {
            let mid = self.mid().unwrap_or(0.0);
            if mid <= 0.0 {
                return 0.0;
            }
            let base_equity = account_equity(self.base_equity, self.base_balance);
            let quote_available = available_coin_qty(
                self.quote_balance,
                self.quote_max_loan,
                quote.borrow_limit(1.0),
                0.0,
            );
            let base_available = available_coin_qty(
                self.base_balance,
                self.base_max_loan,
                base.borrow_limit(mid),
                0.0,
            );
            let available = match side {
                Side::Buy => {
                    let base_capacity = base.max_balance - base_equity;
                    let quote_capacity = quote_available / mid;
                    base_capacity.min(quote_capacity)
                }
                Side::Sell => {
                    if quote.max_balance > 0.0 && self.quote_balance >= quote.max_balance {
                        return 0.0;
                    }
                    let base_capacity = base_equity - base.min_balance;
                    base_capacity.min(base_available - quote.min_balance)
                }
            };
            return available.min(size_limit);
        }

        let buffer = self.config.max_order_size * self.config.safety_multiplier;
        let available_by_position = match side {
            Side::Buy => self.config.max_position - self.position_qty - buffer,
            Side::Sell => self.position_qty - self.config.min_position - buffer,
        };
        let available_by_margin = if side.factor() * self.position_qty < 0.0 {
            f64::MAX
        } else if self.margin_initialized
            && let Some(margin_coin) = &self.margin_coin_config
        {
            let mid = self.mid().unwrap_or(0.0);
            if mid <= 0.0 {
                return 0.0;
            }
            let borrow_limit = margin_coin.borrow_limit(if self.config.kind.is_inverse() {
                mid
            } else {
                1.0
            });
            if self.margin_balance < -borrow_limit || self.margin_liability.abs() > borrow_limit {
                0.0
            } else {
                let available_coin = available_coin_qty(
                    self.margin_balance,
                    self.margin_max_loan,
                    borrow_limit,
                    0.0,
                );
                let margin_coin_per_contract = if self.config.kind.is_inverse() {
                    self.config.contract_value / mid
                } else {
                    self.config.contract_value * mid
                };
                let margin_multiplier = if self.config.kind.is_inverse() {
                    10.0
                } else {
                    2.0
                };
                available_coin / margin_coin_per_contract.max(EPS) * margin_multiplier - buffer
            }
        } else {
            f64::MAX
        };
        available_by_position
            .min(available_by_margin)
            .min(size_limit)
    }

    fn append_hedge_candidates(
        &self,
        hedge_side: Side,
        ref_mid: f64,
        own_quotes: &[(Side, Price, Quantity)],
        notional_limit: f64,
        levels: &mut Vec<HedgeCandidate>,
    ) {
        let book_side = hedge_side.reverse();
        if self.book.is_none() {
            return;
        }
        levels.reserve(self.effective_levels(book_side).len());
        let skew_bps = self.fv_adjust();
        let adjust_bps = hedge_side.factor() * (self.config.taker_fee + self.hedge_profit_margin());
        let max_chunk = self.max_hedge_chunk_qty(ref_mid);
        let mut acc_qty = 0.0;
        let mut candidate_notional = 0.0;

        for (level_idx, level) in self.effective_levels(book_side).iter().enumerate() {
            if level.qty <= 0.0 || !level.px.is_finite() {
                continue;
            }
            let own_qty = own_quotes
                .iter()
                .filter(|(side, price, _)| *side == book_side && approx_eq(*price, level.px))
                .map(|(_, _, qty)| *qty)
                .sum::<f64>();
            let mut remaining = (level.qty - own_qty).max(0.0);
            while remaining > 0.0 {
                let qty = round_down_to_lot(remaining.min(max_chunk), self.config.lot_size)
                    .max(self.config.min_trade_size)
                    .min(remaining);
                if qty <= 0.0 {
                    break;
                }
                acc_qty += qty;
                let end_pos = self.inventory_position() + hedge_side.factor() * acc_qty;
                let pos_skew_rate = self.average_skew_rate_to(end_pos);
                let hedge_rate = level.px / ref_mid - skew_bps
                    + adjust_bps
                    + hedge_side.factor() * acc_qty * pos_skew_rate * 0.5;
                let notional_usd = self.notional_usd(qty, level.px, ref_mid);
                levels.push(HedgeCandidate {
                    symbol: Arc::clone(&self.symbol_key),
                    priority: self.config.hedge_priority,
                    level: level_idx,
                    px: level.px,
                    qty,
                    hedge_rate,
                    notional_usd,
                    acc_qty,
                });
                candidate_notional += notional_usd;
                if candidate_notional >= notional_limit {
                    return;
                }
                remaining -= qty;
            }
        }
    }

    #[cfg(test)]
    fn hedge_levels(
        &self,
        hedge_side: Side,
        ref_mid: f64,
        own_quotes: &[(Side, Price, Quantity)],
    ) -> Vec<HedgeCandidate> {
        let mut levels = Vec::new();
        self.append_hedge_candidates(hedge_side, ref_mid, own_quotes, f64::INFINITY, &mut levels);
        levels
    }

    fn max_hedge_chunk_qty(&self, ref_mid: f64) -> f64 {
        let contract_usd = if self.config.kind.is_inverse() {
            self.config.contract_value
        } else if self.config.kind.is_derivative() {
            self.config.contract_value * ref_mid
        } else {
            ref_mid
        }
        .max(EPS);
        let minimum = (200.0 / contract_usd).max(self.config.min_trade_size);
        let chunk_for_skew = |skew: f64| {
            if skew <= EPS {
                f64::INFINITY
            } else {
                (self.config.hedge_profit_margin / skew).max(minimum)
            }
        };

        if self.config.kind.is_spot()
            && let (Some(base), Some(quote)) = (&self.base_coin_config, &self.quote_coin_config)
        {
            let mid = self.mid().unwrap_or(ref_mid);
            let quote_skew = mid * quote.buy_skew;
            let zone = base.skew_zone(self.inventory_position());
            return match zone {
                -2 => chunk_for_skew(base.skew_rate_at(base.min_balance) + quote_skew),
                -1 => chunk_for_skew(base.sell_skew + quote_skew),
                0 => chunk_for_skew(base.sell_skew + quote_skew)
                    .min(chunk_for_skew(base.buy_skew + quote_skew)),
                1 => chunk_for_skew(base.buy_skew + quote_skew),
                2 => chunk_for_skew(base.skew_rate_at(base.max_balance) + quote_skew),
                _ => f64::INFINITY,
            };
        }

        let zone = self.position_skew_zone(self.position_qty);
        match zone {
            -2 => chunk_for_skew(
                self.skew_rate_at((self.config.min_position + self.config.neg_activation) * 0.5),
            ),
            -1 => chunk_for_skew(self.config.neg_skew),
            0 => chunk_for_skew(self.config.neg_skew).min(chunk_for_skew(self.config.pos_skew)),
            1 => chunk_for_skew(self.config.pos_skew),
            2 => chunk_for_skew(
                self.skew_rate_at((self.config.max_position + self.config.pos_activation) * 0.5),
            ),
            _ => f64::INFINITY,
        }
    }

    fn mid(&self) -> Option<f64> {
        let bid = self.effective_levels(Side::Buy).first()?;
        let ask = self.effective_levels(Side::Sell).first()?;
        Some((bid.px + ask.px) * 0.5)
    }

    fn opposite_touch(&self, quote_side: Side) -> Option<f64> {
        self.book.as_ref()?;
        match quote_side {
            Side::Buy => self
                .effective_levels(Side::Sell)
                .first()
                .map(|level| level.px),
            Side::Sell => self
                .effective_levels(Side::Buy)
                .first()
                .map(|level| level.px),
        }
    }

    fn quote_only_price(&self, side: Side, target: f64) -> f64 {
        let Some(book) = &self.book else {
            return target;
        };
        let (Some(raw_bid), Some(raw_ask)) = (book.best_bid(), book.best_ask()) else {
            return target;
        };
        match side {
            Side::Buy if target > raw_bid.px => {
                (raw_bid.px + self.config.tick_size).min(raw_ask.px - self.config.tick_size)
            }
            Side::Sell if target < raw_ask.px => {
                (raw_ask.px - self.config.tick_size).max(raw_bid.px + self.config.tick_size)
            }
            _ => target,
        }
    }

    fn px_within_limit(&self, side: Side, px: f64) -> f64 {
        let Some(_book) = &self.book else {
            return px;
        };
        let passive_px = match side {
            Side::Buy => self
                .effective_levels(Side::Sell)
                .get(1)
                .or_else(|| self.effective_levels(Side::Sell).first())
                .map(|ask| px.min(ask.px))
                .unwrap_or(px),
            Side::Sell => self
                .effective_levels(Side::Buy)
                .get(1)
                .or_else(|| self.effective_levels(Side::Buy).first())
                .map(|bid| px.max(bid.px))
                .unwrap_or(px),
        };
        match side {
            Side::Buy => self
                .limit_up
                .map(|limit| passive_px.min(limit * (1.0 - self.config.price_limit_buffer)))
                .unwrap_or(passive_px),
            Side::Sell => self
                .limit_down
                .map(|limit| passive_px.max(limit * (1.0 + self.config.price_limit_buffer)))
                .unwrap_or(passive_px),
        }
    }

    fn can_take_within_price_limit(&self, side: Side) -> bool {
        let level = self
            .effective_levels(side.reverse())
            .get(1)
            .or_else(|| self.effective_levels(side.reverse()).first())
            .map(|level| level.px);
        match (side, level) {
            (Side::Buy, Some(px)) => self
                .limit_up
                .is_none_or(|limit| px <= limit * (1.0 - self.config.price_limit_buffer)),
            (Side::Sell, Some(px)) => self
                .limit_down
                .is_none_or(|limit| px >= limit * (1.0 + self.config.price_limit_buffer)),
            (_, None) => false,
        }
    }

    fn hedge_px(&self, hedge_side: Side, px: f64, agg_factor: f64) -> f64 {
        let side_to_take = hedge_side.reverse();
        let ag_mult = 1.0 - side_to_take.factor() * agg_factor;
        round_to_tick(px * ag_mult, self.config.tick_size)
    }

    fn effective_levels(&self, side: Side) -> &[reap_core::Level] {
        let Some(book) = &self.book else {
            return &[];
        };
        let levels = book.levels(side);
        if self.ignore_best_level && levels.len() > 1 {
            &levels[1..]
        } else {
            levels
        }
    }

    fn quote_qty_from_usd(&self, usd: f64, ref_mid: f64) -> f64 {
        self.size_from_usd(usd, ref_mid)
    }

    fn size_from_usd(&self, usd: f64, ref_mid: f64) -> f64 {
        if self.config.kind.is_spot() {
            return self.mid().map(|mid| usd / mid).unwrap_or(0.0);
        }
        if self.config.kind.is_inverse() {
            return usd / self.config.contract_value.max(EPS);
        }
        usd / (self.mid().unwrap_or(ref_mid) * self.config.contract_value.max(EPS))
    }

    fn notional_usd(&self, qty: f64, px: f64, ref_mid: f64) -> f64 {
        self.notional_coin(qty, px) * ref_mid
    }

    fn notional_coin(&self, qty: f64, px: f64) -> f64 {
        if self.config.kind.is_spot() {
            return qty;
        }
        if self.config.kind.is_inverse() {
            return qty * self.config.contract_value / px.max(EPS);
        }
        qty * self.config.contract_value
    }

    fn delta_coin(&self) -> f64 {
        if self.config.kind.is_spot() {
            return self.inventory_position();
        }
        if self.config.kind.is_inverse() {
            if self.position_qty == 0.0 || self.position_avg_price <= 0.0 {
                return 0.0;
            }
            return self.position_qty * self.config.contract_value / self.position_avg_price;
        }
        self.position_qty * self.config.contract_value
    }

    fn delta_coin_for_qty(&self, signed_qty: f64, px: f64) -> f64 {
        if self.config.kind.is_spot() {
            signed_qty
        } else if self.config.kind.is_inverse() {
            signed_qty * self.config.contract_value / px.max(EPS)
        } else {
            signed_qty * self.config.contract_value
        }
    }

    fn record_fill(&mut self, side: Side, qty: f64, px: f64, _liquidity: Option<FillLiquidity>) {
        match side {
            Side::Buy => {
                self.buy_fill_qty += qty;
                self.buy_fill_notional += qty * px;
            }
            Side::Sell => {
                self.sell_fill_qty += qty;
                self.sell_fill_notional += qty * px;
            }
        }
    }

    fn anomalous_fill_should_stop(
        &mut self,
        now_ms: TimeMs,
        side: Side,
        order_price: Price,
        fill_price: Price,
    ) -> bool {
        let market_price = self
            .effective_levels(side.reverse())
            .first()
            .map(|level| level.px)
            .unwrap_or(f64::NAN);
        let market_ratio = 1.0 - side.factor() * 0.005;
        let anomalous_to_market = side.is_more_passive(fill_price, market_ratio * order_price)
            && market_price.is_finite()
            && side.is_more_passive(fill_price, market_ratio * market_price);
        let target_ratio = 1.0 - side.factor() * 0.01;
        let anomalous_to_target = side.is_more_passive(fill_price, order_price * target_ratio);
        if !(anomalous_to_market || anomalous_to_target) {
            self.last_normal_fill_ms = now_ms;
            return false;
        }

        if now_ms.saturating_sub(self.last_aggressive_fill_ms) < 1_000
            && now_ms.saturating_sub(self.last_normal_fill_ms) > 2_000
            && self.aggressive_fill_count >= 5
        {
            return true;
        }
        if now_ms.saturating_sub(self.last_aggressive_fill_ms) > 600_000 {
            self.aggressive_fill_count = 0;
        }
        self.last_aggressive_fill_ms = now_ms;
        self.aggressive_fill_count += 1;
        false
    }

    fn fv_adjust(&self) -> f64 {
        self.posn_skew() + self.config.fv_offset - self.effective_funding_rate()
    }

    fn effective_funding_rate(&self) -> f64 {
        if !self.config.kind.is_swap() {
            return 0.0;
        }
        if let Some(funding_override) = self.config.funding_override {
            return funding_override;
        }
        if self.funding_rate_active {
            self.funding_rate
        } else {
            0.0
        }
    }

    fn trading_pnl_usd(&self, ref_mid: f64) -> f64 {
        let mark = self.mid().unwrap_or(ref_mid);
        let buy_avg_px = if self.buy_fill_qty > 0.0 {
            self.buy_fill_notional / self.buy_fill_qty
        } else {
            0.0
        };
        let sell_avg_px = if self.sell_fill_qty > 0.0 {
            self.sell_fill_notional / self.sell_fill_qty
        } else {
            0.0
        };
        let average_fee_rate = (self.config.maker_fee + self.config.taker_fee) * 0.5;
        if self.config.kind.is_spot() {
            let gross_quote =
                (mark - buy_avg_px) * self.buy_fill_qty + (sell_avg_px - mark) * self.sell_fill_qty;
            let fees_quote = (self.buy_fill_notional + self.sell_fill_notional) * average_fee_rate;
            (gross_quote - fees_quote) / mark.max(EPS) * ref_mid
        } else if self.config.kind.is_inverse() {
            let mut pnl_coin = 0.0;
            if self.buy_fill_qty > 0.0 {
                pnl_coin -= self.config.contract_value
                    * (self.buy_fill_qty / mark.max(EPS) - self.buy_fill_qty / buy_avg_px.max(EPS));
                pnl_coin -= average_fee_rate * self.config.contract_value * self.buy_fill_qty
                    / buy_avg_px.max(EPS);
            }
            if self.sell_fill_qty > 0.0 {
                pnl_coin += self.config.contract_value
                    * (self.sell_fill_qty / mark.max(EPS)
                        - self.sell_fill_qty / sell_avg_px.max(EPS));
                pnl_coin -= average_fee_rate * self.config.contract_value * self.sell_fill_qty
                    / sell_avg_px.max(EPS);
            }
            pnl_coin * ref_mid
        } else {
            let mut pnl_coin = 0.0;
            if self.buy_fill_qty > 0.0 {
                pnl_coin += self.config.contract_value * self.buy_fill_qty * (mark - buy_avg_px)
                    / mark.max(EPS);
                pnl_coin -= average_fee_rate * self.config.contract_value * self.buy_fill_qty;
            }
            if self.sell_fill_qty > 0.0 {
                pnl_coin -= self.config.contract_value * self.sell_fill_qty * (mark - sell_avg_px)
                    / mark.max(EPS);
                pnl_coin -= average_fee_rate * self.config.contract_value * self.sell_fill_qty;
            }
            pnl_coin * ref_mid
        }
    }

    fn posn_skew(&self) -> f64 {
        if self.config.kind.is_spot()
            && let (Some(base), Some(quote)) = (&self.base_coin_config, &self.quote_coin_config)
        {
            let base_skew = -base.integrated_skew(self.inventory_position());
            let quote_skew = -(self.quote_balance - quote.skew_offset) * quote.buy_skew;
            return base_skew - quote_skew;
        }
        -self.integrated_skew(self.position_qty)
    }

    fn skew_rate_at(&self, target_pos: f64) -> f64 {
        let shifted = target_pos - self.config.position_offset;
        if shifted > 0.0 {
            if self.config.skew_type == SkewTypeConfig::Fix
                || target_pos <= self.config.pos_activation
            {
                self.config.pos_skew
            } else {
                self.config.pos_skew + self.config.pos_extra_skew
            }
        } else if shifted < 0.0 {
            if self.config.skew_type == SkewTypeConfig::Fix
                || target_pos >= self.config.neg_activation
            {
                self.config.neg_skew
            } else {
                self.config.neg_skew + self.config.neg_extra_skew
            }
        } else {
            self.config.pos_skew.max(self.config.neg_skew)
        }
    }

    fn position_skew_zone(&self, position: f64) -> i8 {
        let shifted = position - self.config.position_offset;
        if shifted > 0.0 {
            if self.config.skew_type == SkewTypeConfig::Fix
                || position <= self.config.pos_activation
            {
                1
            } else {
                2
            }
        } else if shifted < 0.0 {
            if self.config.skew_type == SkewTypeConfig::Fix
                || position >= self.config.neg_activation
            {
                -1
            } else {
                -2
            }
        } else {
            0
        }
    }

    fn integrated_skew(&self, target_pos: f64) -> f64 {
        let offset = self.config.position_offset;
        if target_pos >= offset {
            let activation = self.config.pos_activation.max(offset);
            let basic_end = target_pos.min(activation);
            let basic = (basic_end - offset).max(0.0) * self.config.pos_skew;
            let extra = (target_pos - activation).max(0.0)
                * (self.config.pos_skew
                    + if self.config.skew_type == SkewTypeConfig::Step {
                        self.config.pos_extra_skew
                    } else {
                        0.0
                    });
            basic + extra
        } else {
            let activation = self.config.neg_activation.min(offset);
            let basic_end = target_pos.max(activation);
            let basic = (basic_end - offset).min(0.0) * self.config.neg_skew;
            let extra = (target_pos - activation).min(0.0)
                * (self.config.neg_skew
                    + if self.config.skew_type == SkewTypeConfig::Step {
                        self.config.neg_extra_skew
                    } else {
                        0.0
                    });
            basic + extra
        }
    }

    fn average_skew_rate_to(&self, target_pos: f64) -> f64 {
        if self.config.kind.is_spot()
            && let (Some(base), Some(quote)) = (&self.base_coin_config, &self.quote_coin_config)
        {
            return base.average_skew_rate(self.inventory_position(), target_pos)
                + self.mid().unwrap_or(0.0) * quote.buy_skew;
        }
        let delta = target_pos - self.position_qty;
        if delta.abs() <= EPS {
            return self.skew_rate_at(target_pos);
        }
        (self.integrated_skew(target_pos) - self.integrated_skew(self.position_qty)) / delta
    }

    fn in_extra_skew_zone(&self) -> bool {
        if !self.config.kind.is_derivative() || self.config.skew_type != SkewTypeConfig::Step {
            return false;
        }
        self.position_qty > self.config.pos_activation
            || self.position_qty < self.config.neg_activation
    }

    fn hedge_profit_margin(&self) -> f64 {
        self.config.hedge_profit_margin
            + if self.in_extra_skew_zone() {
                EXTRA_MARGIN_BPS
            } else {
                0.0
            }
    }

    fn quote_profit_margin(&self) -> f64 {
        self.config.quote_profit_margin
            + if self.in_extra_skew_zone() {
                EXTRA_MARGIN_BPS
            } else {
                0.0
            }
    }

    fn hedge_aggression(&self) -> f64 {
        self.config.hedge_aggression
            + if self.in_extra_skew_zone() {
                EXTRA_MARGIN_BPS
            } else {
                0.0
            }
    }

    fn inventory_position(&self) -> f64 {
        if self.config.kind.is_spot() && self.balances_initialized {
            self.base_balance
        } else {
            self.position_qty
        }
    }
}

#[derive(Debug, Clone, Default)]
struct DebouncedCondition {
    last_disqualify_ms: Option<TimeMs>,
}

impl DebouncedCondition {
    fn check(&mut self, qualifies: bool, now_ms: TimeMs, interval_ms: TimeMs) -> bool {
        if !qualifies {
            self.last_disqualify_ms = Some(now_ms);
            return false;
        }
        self.last_disqualify_ms
            .is_none_or(|last| now_ms.saturating_sub(last) > interval_ms)
    }
}

#[derive(Debug, Clone)]
pub struct RiskGroupState {
    pub config: RiskGroupConfig,
    pub symbols: HashSet<Symbol>,
    pub max_quote_size_usd: f64,
    pub delta_usd: f64,
    pub pending_delta_usd: f64,
    pub live_order_size_usd: f64,
    pub margin_ratio: Option<f64>,
    pub exchange_margin_ratio: Option<f64>,
    margin_adjusted_equity_usd: Option<f64>,
    margin_notional_usd: Option<f64>,
    pub best_hedges: HashMap<Side, Vec<HedgeLevel>>,
    account_balances: HashMap<String, AccountBalanceState>,
}

impl RiskGroupState {
    fn new(config: RiskGroupConfig) -> Self {
        let symbols = config.symbols.iter().cloned().collect();
        let mut best_hedges = HashMap::new();
        best_hedges.insert(Side::Buy, Vec::new());
        best_hedges.insert(Side::Sell, Vec::new());
        Self {
            config,
            symbols,
            max_quote_size_usd: 0.0,
            delta_usd: 0.0,
            pending_delta_usd: 0.0,
            live_order_size_usd: 0.0,
            margin_ratio: None,
            exchange_margin_ratio: None,
            margin_adjusted_equity_usd: None,
            margin_notional_usd: None,
            best_hedges,
            account_balances: HashMap::new(),
        }
    }

    fn best_hedges_for(&self, side: Side) -> &[HedgeLevel] {
        self.best_hedges
            .get(&side)
            .map(Vec::as_slice)
            .unwrap_or(&[])
    }

    fn can_quote_side(&self, side: Side) -> bool {
        match side {
            Side::Buy => {
                self.delta_usd + self.max_quote_size_usd < self.config.hard_delta_limit_usd
            }
            Side::Sell => {
                self.delta_usd - self.max_quote_size_usd > -self.config.hard_delta_limit_usd
            }
        }
    }

    fn can_increase_delta_with_quote_buffer(&self, side: Side) -> bool {
        match side {
            Side::Buy => {
                self.delta_usd + self.max_quote_size_usd < self.config.soft_delta_limit_usd
            }
            Side::Sell => {
                self.delta_usd - self.max_quote_size_usd > -self.config.soft_delta_limit_usd
            }
        }
    }

    fn must_hedge_within_group(&self, delta_to_hedge: f64) -> bool {
        if delta_to_hedge < 0.0 && self.delta_usd < self.config.soft_delta_limit_usd {
            return false;
        }
        !((delta_to_hedge > 0.0) && (self.delta_usd > -self.config.soft_delta_limit_usd))
    }
}

#[derive(Debug, Clone, Copy, Default)]
struct AccountBalanceState {
    cash: Quantity,
    liability: Quantity,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct TheoQuote {
    pub price: Price,
    pub qty: Quantity,
    pub hedge_px: Price,
    #[serde(skip)]
    pub hedge_symbol: Symbol,
}

#[derive(Debug, Clone)]
pub struct HedgeLevel {
    pub symbol: Symbol,
    pub priority: i32,
    pub level: usize,
    pub px: Price,
    pub qty: Quantity,
    pub hedge_rate: f64,
    pub notional_usd: f64,
    pub acc_qty: Quantity,
}

#[derive(Debug, Clone)]
struct HedgeCandidate {
    symbol: Arc<str>,
    priority: i32,
    level: usize,
    px: Price,
    qty: Quantity,
    hedge_rate: f64,
    notional_usd: f64,
    acc_qty: Quantity,
}

impl HedgeCandidate {
    fn to_owned_level(&self) -> HedgeLevel {
        HedgeLevel {
            symbol: self.symbol.to_string(),
            priority: self.priority,
            level: self.level,
            px: self.px,
            qty: self.qty,
            hedge_rate: self.hedge_rate,
            notional_usd: self.notional_usd,
            acc_qty: self.acc_qty,
        }
    }
}

#[derive(Debug, Clone)]
struct HedgeTarget {
    symbol: Symbol,
    orig_px: Price,
    hedge_px: Price,
    qty: Quantity,
    cur_level_acc_qty: Quantity,
    notional_usd: f64,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct MissedHedge {
    pub ts_ms: TimeMs,
    pub order_id: String,
    pub symbol: Symbol,
    pub side: Side,
    pub price: Price,
    pub missed_qty: Quantity,
    pub missed_delta_usd: f64,
    pub reference_symbol: Option<Symbol>,
}

#[derive(Debug, Clone)]
struct ActiveQuote {
    order_id: String,
    price: Price,
    qty: Quantity,
}

#[derive(Debug, Clone)]
struct ActiveHedge {
    symbol: Symbol,
    signed_open_qty: Quantity,
    price: Price,
    reference_price: Price,
    updated_ms: TimeMs,
}

#[derive(Debug, Clone)]
struct QuoteTargetState {
    levels: Vec<TheoQuote>,
    updated_ms: TimeMs,
}

#[derive(Debug, Clone)]
struct JavaRandom {
    seed: u64,
}

impl JavaRandom {
    const MULTIPLIER: u64 = 0x5DEECE66D;
    const ADDEND: u64 = 0xB;
    const MASK: u64 = (1_u64 << 48) - 1;

    fn new(seed: u64) -> Self {
        Self {
            seed: (seed ^ Self::MULTIPLIER) & Self::MASK,
        }
    }

    fn next_bits(&mut self, bits: u32) -> u64 {
        self.seed = (self
            .seed
            .wrapping_mul(Self::MULTIPLIER)
            .wrapping_add(Self::ADDEND))
            & Self::MASK;
        self.seed >> (48 - bits)
    }

    fn next_f64(&mut self) -> f64 {
        let high = self.next_bits(26);
        let low = self.next_bits(27);
        ((high << 27) + low) as f64 / (1_u64 << 53) as f64
    }
}

fn price_by_hedge(
    quote_side: Side,
    pricing: &InstrumentState,
    hedge: &InstrumentState,
    hedge_level: &HedgeLevel,
    ref_mid: f64,
) -> f64 {
    let hedge_px_at_spot = hedge_level.px - hedge.fv_adjust() * ref_mid;
    let adjust_bps = quote_side.factor()
        * (hedge.config.taker_fee
            + hedge.hedge_profit_margin()
            + pricing.quote_profit_margin()
            + pricing.config.maker_fee);
    hedge_px_at_spot + (pricing.fv_adjust() - adjust_bps) * ref_mid
}

fn sort_hedge_levels(side: Side, levels: &mut [HedgeLevel]) {
    match side {
        Side::Buy => levels.sort_by(|a, b| {
            a.hedge_rate
                .total_cmp(&b.hedge_rate)
                .then_with(|| b.priority.cmp(&a.priority))
                .then_with(|| b.symbol.cmp(&a.symbol))
                .then_with(|| a.level.cmp(&b.level))
        }),
        Side::Sell => levels.sort_by(|a, b| {
            b.hedge_rate
                .total_cmp(&a.hedge_rate)
                .then_with(|| b.priority.cmp(&a.priority))
                .then_with(|| b.symbol.cmp(&a.symbol))
                .then_with(|| a.level.cmp(&b.level))
        }),
    }
}

fn sort_hedge_candidates(side: Side, levels: &mut [HedgeCandidate]) {
    match side {
        Side::Buy => levels.sort_by(|a, b| {
            a.hedge_rate
                .total_cmp(&b.hedge_rate)
                .then_with(|| b.priority.cmp(&a.priority))
                .then_with(|| b.symbol.cmp(&a.symbol))
                .then_with(|| a.level.cmp(&b.level))
        }),
        Side::Sell => levels.sort_by(|a, b| {
            b.hedge_rate
                .total_cmp(&a.hedge_rate)
                .then_with(|| b.priority.cmp(&a.priority))
                .then_with(|| b.symbol.cmp(&a.symbol))
                .then_with(|| a.level.cmp(&b.level))
        }),
    }
}

fn all_symbols_have_hedge(
    per_symbol: &HashMap<&str, f64>,
    total_hedge_size: f64,
    max_quote_size: f64,
    symbol_count: usize,
) -> bool {
    if symbol_count <= 1 {
        return true;
    }
    if per_symbol.len() < 2 {
        return false;
    }
    per_symbol
        .values()
        .all(|hedge_size| total_hedge_size - hedge_size >= max_quote_size)
}

fn weighted_avg(old_px: f64, old_qty: f64, new_px: f64, new_qty: f64) -> f64 {
    if old_qty + new_qty <= 0.0 {
        return new_px;
    }
    (old_px * old_qty + new_px * new_qty) / (old_qty + new_qty)
}

fn update_flag(value: &mut bool, next: bool) -> bool {
    let changed = *value != next;
    *value = next;
    changed
}

fn update_weighted_px(map: &mut HashMap<Symbol, (f64, f64)>, symbol: &str, px: f64, qty: f64) {
    map.entry(symbol.to_string())
        .and_modify(|(cur_px, cur_qty)| {
            *cur_px = weighted_avg(*cur_px, *cur_qty, px, qty);
            *cur_qty += qty;
        })
        .or_insert((px, qty));
}

fn single_weighted_px(map: HashMap<Symbol, (f64, f64)>) -> f64 {
    if map.len() == 1 {
        map.into_values()
            .next()
            .map(|(px, _)| px)
            .unwrap_or(f64::NAN)
    } else {
        f64::NAN
    }
}

fn approx_eq(a: f64, b: f64) -> bool {
    (a - b).abs() <= 1e-7_f64.max(a.abs().max(b.abs()) * 1e-9)
}

fn round_passive_to_tick(px: f64, tick_size: f64, side: Side) -> f64 {
    if tick_size <= 0.0 || !px.is_finite() {
        return px;
    }
    match side {
        Side::Buy => (px / tick_size).floor() * tick_size,
        Side::Sell => (px / tick_size).ceil() * tick_size,
    }
}

fn quote_level_from_reason(reason: &str) -> usize {
    reason
        .split(':')
        .nth(1)
        .and_then(|level| level.parse().ok())
        .unwrap_or(0)
}

fn hedge_reference_price(reason: &str) -> Option<Price> {
    reason.split(':').nth(2)?.parse().ok()
}

fn is_usd_equivalent(currency: &str) -> bool {
    matches!(currency, "USD" | "USDT" | "USDC" | "FDUSD" | "BUSD")
}

fn account_equity(equity: f64, cash: f64) -> f64 {
    if equity == 0.0 && cash != 0.0 {
        cash
    } else {
        equity
    }
}

fn available_coin_qty(cash: f64, max_loan: f64, borrow_limit: f64, minimum: f64) -> f64 {
    cash + borrow_limit.min(max_loan.max(0.0)) - minimum.max(0.0)
}

#[cfg(test)]
mod tests {
    use super::*;
    use reap_core::{
        AccountUpdate, Balance, Level, MarginSnapshot, MarketEvent, NormalizedEvent, OrderBook,
        OrderStatus, StrategyEvent,
    };

    fn config() -> ChaosConfig {
        ChaosConfig {
            ref_symbol: "BTC-USDT".to_string(),
            delta_limit_usd: 50_000.0,
            active_hedge_threshold_usd: 1_000.0,
            min_hedge_interval_ms: 0,
            risk_groups: vec![RiskGroupConfig {
                name: "main".to_string(),
                symbols: vec!["BTC-USDT".to_string(), "BTC-PERP".to_string()],
                soft_delta_limit_usd: 25_000.0,
                hard_delta_limit_usd: 40_000.0,
                live_order_limit_usd: 100_000.0,
                ..RiskGroupConfig::default()
            }],
            instruments: vec![
                InstrumentConfig {
                    symbol: "BTC-USDT".to_string(),
                    risk_group: "main".to_string(),
                    kind: InstrumentKindConfig::Spot,
                    tick_size: 0.1,
                    lot_size: 0.0001,
                    min_trade_size: 0.0001,
                    max_order_size_usd: 5_000.0,
                    min_order_size_usd: 100.0,
                    max_order_size: 1.0,
                    ..InstrumentConfig::default()
                },
                InstrumentConfig {
                    symbol: "BTC-PERP".to_string(),
                    risk_group: "main".to_string(),
                    kind: InstrumentKindConfig::Future,
                    tick_size: 0.1,
                    lot_size: 1.0,
                    min_trade_size: 1.0,
                    contract_value: 0.001,
                    max_order_size_usd: 5_000.0,
                    min_order_size_usd: 100.0,
                    max_order_size: 200.0,
                    min_position: -10_000.0,
                    max_position: 10_000.0,
                    ..InstrumentConfig::default()
                },
            ],
            ..ChaosConfig::default()
        }
    }

    fn java_calculator_config() -> ChaosConfig {
        let instrument = |symbol: &str,
                          kind: InstrumentKindConfig,
                          risk_group: &str,
                          contract_value: f64,
                          max_order_size: f64,
                          min_position: f64,
                          max_position: f64,
                          skew: f64| InstrumentConfig {
            symbol: symbol.to_string(),
            kind,
            risk_group: risk_group.to_string(),
            maker_fee: -0.001,
            taker_fee: 0.001,
            hedge_profit_margin: 0.0005,
            quote_profit_margin: 0.0005,
            hedge_aggression: 0.0003,
            min_order_size_usd: 1_000.0,
            max_order_size_usd: 4_000.0,
            max_order_size,
            min_trade_size: if kind.is_spot() { 0.01 } else { 1.0 },
            tick_size: 0.01,
            lot_size: if kind.is_spot() { 0.01 } else { 1.0 },
            contract_value,
            min_position,
            max_position,
            pos_skew: skew,
            neg_skew: skew,
            ..InstrumentConfig::default()
        };

        let mut instruments = vec![
            instrument(
                "BTC-USDT.OK",
                InstrumentKindConfig::Spot,
                "OKEX-Spot",
                1.0,
                0.08,
                -1_000.0,
                1_000.0,
                0.0,
            ),
            instrument(
                "BTC-USD-SWAP.OK",
                InstrumentKindConfig::InverseSwap,
                "OKEX-Invert",
                100.0,
                40.0,
                -1_000.0,
                1_000.0,
                0.000005,
            ),
            instrument(
                "BTC-USD-211231.OK",
                InstrumentKindConfig::InverseFuture,
                "OKEX-Invert",
                100.0,
                40.0,
                -1_000.0,
                1_000.0,
                0.000005,
            ),
            instrument(
                "BTC-USDT-SWAP.OK",
                InstrumentKindConfig::LinearSwap,
                "OKEX-Linear",
                0.01,
                8.0,
                -161.0,
                161.0,
                0.000031055900621118014,
            ),
            instrument(
                "BTC-USDT-211231.OK",
                InstrumentKindConfig::LinearFuture,
                "OKEX-Linear",
                0.01,
                8.0,
                -161.0,
                161.0,
                0.000031055900621118014,
            ),
        ];
        instruments
            .iter_mut()
            .find(|instrument| instrument.symbol == "BTC-USD-SWAP.OK")
            .unwrap()
            .hedge_priority = 1;
        instruments
            .iter_mut()
            .find(|instrument| instrument.symbol == "BTC-USDT-211231.OK")
            .unwrap()
            .hedge_priority = 1;

        ChaosConfig {
            strategy_name: "CalcTest".to_string(),
            underlying: "BTC".to_string(),
            ref_symbol: "BTC-USDT.OK".to_string(),
            delta_limit_usd: 50_000.0,
            active_hedge_threshold_usd: 800.0,
            min_hedge_interval_ms: 200,
            risk_groups: vec![
                RiskGroupConfig {
                    name: "OKEX-Spot".to_string(),
                    symbols: vec!["BTC-USDT.OK".to_string()],
                    soft_delta_limit_usd: 20_000.0,
                    hard_delta_limit_usd: 30_000.0,
                    delta_stop_limit_usd: 50_000.0,
                    live_order_limit_usd: 100_000.0,
                    ..RiskGroupConfig::default()
                },
                RiskGroupConfig {
                    name: "OKEX-Invert".to_string(),
                    symbols: vec![
                        "BTC-USD-SWAP.OK".to_string(),
                        "BTC-USD-211231.OK".to_string(),
                    ],
                    soft_delta_limit_usd: 20_000.0,
                    hard_delta_limit_usd: 30_000.0,
                    delta_stop_limit_usd: 50_000.0,
                    live_order_limit_usd: 100_000.0,
                    ..RiskGroupConfig::default()
                },
                RiskGroupConfig {
                    name: "OKEX-Linear".to_string(),
                    symbols: vec![
                        "BTC-USDT-SWAP.OK".to_string(),
                        "BTC-USDT-211231.OK".to_string(),
                    ],
                    soft_delta_limit_usd: 20_000.0,
                    hard_delta_limit_usd: 30_000.0,
                    delta_stop_limit_usd: 50_000.0,
                    live_order_limit_usd: 100_000.0,
                    ..RiskGroupConfig::default()
                },
            ],
            instruments,
            ..ChaosConfig::default()
        }
    }

    fn seed_java_calculator_books(strategy: &mut ChaosStrategy) {
        for entity in strategy.entities.values_mut() {
            let qty = if entity.config.kind.is_spot() {
                0.2
            } else {
                20.0
            };
            entity.book = Some(OrderBook {
                symbol: entity.config.symbol.clone(),
                ts_ms: 1,
                bids: [45_000.0, 40_000.0, 35_000.0, 30_000.0, 25_000.0]
                    .into_iter()
                    .map(|px| Level::new(px, qty))
                    .collect(),
                asks: [55_000.0, 60_000.0, 65_000.0, 70_000.0, 75_000.0]
                    .into_iter()
                    .map(|px| Level::new(px, qty))
                    .collect(),
            });
        }
    }

    fn spot_skew_state(
        base_coin_config: CoinConfig,
        quote_coin_config: CoinConfig,
        base_balance: f64,
        quote_balance: f64,
    ) -> InstrumentState {
        let mut state = InstrumentState::new(InstrumentConfig {
            symbol: "BTC-USD".to_string(),
            kind: InstrumentKindConfig::Spot,
            base_currency: "BTC".to_string(),
            quote_currency: "USD".to_string(),
            hedge_profit_margin: 0.0005,
            min_trade_size: 0.0001,
            ..InstrumentConfig::default()
        });
        state.book = Some(OrderBook::one_level(
            "BTC-USD",
            1,
            Level::new(49_999.0, 10.0),
            Level::new(50_001.0, 10.0),
        ));
        state.base_coin_config = Some(base_coin_config);
        state.quote_coin_config = Some(quote_coin_config);
        state.base_balance = base_balance;
        state.base_available = base_balance.max(0.0);
        state.quote_balance = quote_balance;
        state.quote_available = quote_balance.max(0.0);
        state.balances_initialized = true;
        state
    }

    fn fixed_base_skew() -> CoinConfig {
        CoinConfig {
            currency: "BTC".to_string(),
            min_balance: 28.0,
            max_balance: 32.0,
            skew_offset: 30.0,
            skew_type: Some(SkewTypeConfig::Fix),
            buy_skew: 0.005,
            sell_skew: 0.005,
            buy_activation: 32.0,
            sell_activation: 28.0,
            ..CoinConfig::default()
        }
    }

    fn fixed_quote_skew() -> CoinConfig {
        CoinConfig {
            currency: "USD".to_string(),
            min_balance: 0.0,
            max_balance: 50_000.0,
            skew_offset: 25_000.0,
            skew_type: Some(SkewTypeConfig::Fix),
            buy_skew: 0.0000001,
            sell_skew: 0.0000001,
            ..CoinConfig::default()
        }
    }

    #[test]
    fn computes_quotes_from_opposite_hedge_ladder() {
        let mut strategy = ChaosStrategy::new(config()).unwrap();
        let spot = OrderBook::one_level(
            "BTC-USDT",
            1,
            Level::new(50_000.0, 1.0),
            Level::new(50_001.0, 1.0),
        );
        let perp = OrderBook::one_level(
            "BTC-PERP",
            1,
            Level::new(50_003.0, 200.0),
            Level::new(50_004.0, 200.0),
        );

        strategy.on_event(&StrategyEvent::Market(MarketEvent::Depth(spot)));
        let commands = strategy.on_event(&StrategyEvent::Market(MarketEvent::Depth(perp)));

        assert!(
            commands
                .iter()
                .any(|cmd| matches!(cmd, OrderIntent::NewOrder(o) if o.reason == "quote"))
        );
        let spot_state = strategy.entity("BTC-USDT").unwrap();
        assert!(spot_state.theo(Side::Buy).unwrap().price < 50_001.0);
        assert!(spot_state.theo(Side::Sell).unwrap().price > 50_000.0);
    }

    #[test]
    fn quote_fill_triggers_ioc_hedge_excluding_fill_symbol() {
        let mut strategy = ChaosStrategy::new(config()).unwrap();
        strategy.on_event(&StrategyEvent::Market(MarketEvent::Depth(
            OrderBook::one_level(
                "BTC-USDT",
                1,
                Level::new(50_000.0, 1.0),
                Level::new(50_001.0, 1.0),
            ),
        )));
        strategy.on_event(&StrategyEvent::Market(MarketEvent::Depth(
            OrderBook::one_level(
                "BTC-PERP",
                1,
                Level::new(50_003.0, 2000.0),
                Level::new(50_004.0, 2000.0),
            ),
        )));

        let fill_intents = strategy.on_event(&StrategyEvent::Order(OrderUpdate {
            ts_ms: 2,
            order_id: "q1".to_string(),
            symbol: "BTC-USDT".to_string(),
            side: Side::Buy,
            event: OrderEvent::FullyFilled,
            status: OrderStatus::Filled,
            price: 50_000.0,
            qty: 0.1,
            open_qty: 0.0,
            filled_qty: 0.1,
            avg_fill_price: 50_000.0,
            last_fill_qty: 0.1,
            last_fill_price: 50_000.0,
            last_fill_liquidity: None,
            reason: "quote".to_string(),
        }));
        assert!(fill_intents.is_empty());
        let hedge = strategy.on_event(&StrategyEvent::Account(AccountUpdate {
            ts_ms: 2,
            balances: Vec::new(),
            positions: vec![reap_core::Position {
                symbol: "BTC-USDT".to_string(),
                qty: 0.1,
                avg_price: 50_000.0,
            }],
            margins: Vec::new(),
        }));

        assert!(
            hedge
                .iter()
                .any(|cmd| matches!(cmd, OrderIntent::NewOrder(o)
            if o.symbol == "BTC-PERP"
                && o.side == Side::Sell
                && o.time_in_force == TimeInForce::Ioc
                && o.self_trade_prevention == Some(SelfTradePrevention::CancelMaker)))
        );
    }

    #[test]
    fn normalized_fixture_drives_quote_then_hedge_decisions() {
        let events = include_str!("../../../fixtures/normalized/chaos_quote_hedge.jsonl")
            .lines()
            .filter(|line| !line.trim().is_empty())
            .map(|line| serde_json::from_str::<NormalizedEvent>(line).unwrap())
            .collect::<Vec<_>>();
        let mut strategy = ChaosStrategy::new(config()).unwrap();

        let mut all_intents = Vec::new();
        for event in events {
            let intents = strategy.on_event(&event.into_strategy_event());
            all_intents.push(intents);
        }

        assert!(all_intents[1].iter().any(
            |intent| matches!(intent, OrderIntent::NewOrder(order) if order.reason == "quote")
        ));
        assert!(all_intents[2].is_empty());
        assert!(all_intents[3].iter().any(|intent| matches!(intent, OrderIntent::NewOrder(order)
            if order.symbol == "BTC-PERP" && order.side == Side::Sell && order.time_in_force == TimeInForce::Ioc)));
    }

    #[test]
    fn config_validation_catches_duplicate_symbols_and_invalid_ticks() {
        let valid = config().effective();
        assert!(valid.validate().valid);

        let mut invalid = valid;
        invalid.instruments[1].symbol = invalid.instruments[0].symbol.clone();
        invalid.instruments[0].tick_size = 0.0;
        let report = invalid.validate();
        assert!(!report.valid);
        assert!(
            report
                .errors
                .iter()
                .any(|error| error.contains("duplicate instrument symbol"))
        );
        assert!(
            report
                .errors
                .iter()
                .any(|error| error.contains("tick_size"))
        );
    }

    #[test]
    fn config_validation_rejects_single_instrument_iarb() {
        let mut single = config();
        single.instruments.truncate(1);
        single.risk_groups[0].symbols.truncate(1);

        let report = single.validate();

        assert!(!report.valid);
        assert!(
            report
                .errors
                .iter()
                .any(|error| error.contains("requires at least two instruments"))
        );
        assert!(
            report
                .errors
                .iter()
                .any(|error| error.contains("no distinct hedge-enabled instrument"))
        );
    }

    #[test]
    fn config_validation_requires_spot_reference() {
        let mut invalid = config();
        invalid.ref_symbol = "BTC-PERP".to_string();

        let report = invalid.validate();

        assert!(!report.valid);
        assert!(
            report
                .errors
                .iter()
                .any(|error| error.contains("must be a spot instrument"))
        );
    }

    #[test]
    fn java_parity_applies_risk_multiplier_only_to_java_limits() {
        let mut cfg = config();
        cfg.risk_multiplier = 2.0;
        cfg.coin_offset = 30.0;
        cfg.balance_sheet_limit_usd = 10_000_000.0;
        cfg.delta_limit_usd = 30_000.0;
        cfg.pnl_limit_usd = 5_000.0;
        cfg.index_deviation_limit = 0.05;
        cfg.active_hedge_threshold_usd = 800.0;
        cfg.risk_groups[0].coin_offset = 30.0;
        cfg.risk_groups[0].soft_delta_limit_usd = 10_000.0;
        cfg.risk_groups[0].hard_delta_limit_usd = 20_000.0;
        cfg.risk_groups[0].delta_stop_limit_usd = 40_000.0;
        cfg.risk_groups[0].live_order_limit_usd = 100_000.0;
        cfg.risk_groups[0].turnover_limit_usd = 10_000_000.0;
        cfg.risk_groups[0].basis_limit = 0.05;
        cfg.risk_groups[0].min_margin_level = 0.3;

        let effective = cfg.effective();

        assert!(approx_eq(effective.balance_sheet_limit_usd, 20_000_000.0));
        assert!(approx_eq(effective.delta_limit_usd, 60_000.0));
        assert!(approx_eq(effective.pnl_limit_usd, 10_000.0));
        assert!(approx_eq(effective.index_deviation_limit, 0.1));
        assert!(approx_eq(effective.coin_offset, 30.0));
        assert!(approx_eq(effective.active_hedge_threshold_usd, 800.0));
        let group = &effective.risk_groups[0];
        assert!(approx_eq(group.delta_stop_limit_usd, 80_000.0));
        assert!(approx_eq(group.live_order_limit_usd, 200_000.0));
        assert!(approx_eq(group.turnover_limit_usd, 20_000_000.0));
        assert!(approx_eq(group.basis_limit, 0.1));
        assert!(approx_eq(group.coin_offset, 30.0));
        assert!(approx_eq(group.soft_delta_limit_usd, 10_000.0));
        assert!(approx_eq(group.hard_delta_limit_usd, 20_000.0));
        assert!(approx_eq(group.min_margin_level, 0.3));

        let unlimited = ChaosConfig {
            risk_multiplier: 2.0,
            ..ChaosConfig::default()
        }
        .effective();
        assert_eq!(unlimited.balance_sheet_limit_usd, f64::MAX);
        assert_eq!(unlimited.pnl_limit_usd, f64::MAX);
    }

    #[test]
    fn java_parity_applies_default_safety_multipliers() {
        let mut cfg = config();
        cfg.risk_groups[0].coins = vec![
            CoinConfig {
                currency: "BTC".to_string(),
                ..CoinConfig::default()
            },
            CoinConfig {
                currency: "USDT".to_string(),
                ..CoinConfig::default()
            },
        ];

        let effective = cfg.effective();

        assert!(approx_eq(
            effective.risk_groups[0].coins[0].safety_multiplier,
            2.5
        ));
        assert!(approx_eq(
            effective.risk_groups[0].coins[1].safety_multiplier,
            4.0
        ));
        assert!(approx_eq(effective.instruments[0].safety_multiplier, 1.0));
        assert!(approx_eq(effective.instruments[1].safety_multiplier, 2.0));
    }

    #[test]
    fn inverse_contract_uses_java_iarb_coin_and_usd_conversions() {
        let mut state = InstrumentState::new(InstrumentConfig {
            symbol: "BTC-USD-SWAP".to_string(),
            kind: InstrumentKindConfig::InverseSwap,
            contract_value: 100.0,
            funding_rate: 0.001,
            ..InstrumentConfig::default()
        });
        state.book = Some(OrderBook::one_level(
            "BTC-USD-SWAP",
            1,
            Level::new(44_999.0, 100.0),
            Level::new(45_001.0, 100.0),
        ));
        state.position_qty = 20.0;
        state.position_avg_price = 50_000.0;

        assert!(approx_eq(state.size_from_usd(2_000.0, 50_000.0), 20.0));
        assert!(approx_eq(
            state.notional_coin(20.0, 45_000.0),
            20.0 * 100.0 / 45_000.0
        ));
        assert!(approx_eq(
            state.notional_usd(20.0, 45_000.0, 50_000.0),
            2_222.222222222222
        ));
        assert!(approx_eq(state.delta_coin(), 0.04));
        assert!(approx_eq(state.effective_funding_rate(), 0.001));
    }

    #[test]
    fn funding_override_matches_java_swap_precedence() {
        let state = InstrumentState::new(InstrumentConfig {
            kind: InstrumentKindConfig::LinearSwap,
            funding_rate: 0.001,
            funding_override: Some(-0.002),
            ..InstrumentConfig::default()
        });
        assert!(approx_eq(state.effective_funding_rate(), -0.002));

        let dated = InstrumentState::new(InstrumentConfig {
            kind: InstrumentKindConfig::LinearFuture,
            funding_rate: 0.001,
            funding_override: Some(-0.002),
            ..InstrumentConfig::default()
        });
        assert!(approx_eq(dated.effective_funding_rate(), 0.0));
    }

    #[test]
    fn java_parity_funding_manager_uses_earliest_swap_window() {
        let mut cfg = java_calculator_config();
        cfg.use_funding_rate_manager = true;
        let mut strategy = ChaosStrategy::new(cfg).unwrap();
        {
            let early = strategy.entities.get_mut("BTC-USD-SWAP.OK").unwrap();
            early.funding_rate = 0.001;
            early.funding_time_ms = 1_000;
        }
        {
            let later = strategy.entities.get_mut("BTC-USDT-SWAP.OK").unwrap();
            later.funding_rate = 0.002;
            later.funding_time_ms = 2_000;
        }

        strategy.now_ms = 100;
        strategy.update_funding_window();
        assert!(approx_eq(
            strategy
                .entity("BTC-USD-SWAP.OK")
                .unwrap()
                .effective_funding_rate(),
            0.001
        ));
        assert!(approx_eq(
            strategy
                .entity("BTC-USDT-SWAP.OK")
                .unwrap()
                .effective_funding_rate(),
            0.0
        ));

        strategy.now_ms = 1_500;
        strategy.update_funding_window();
        assert!(approx_eq(
            strategy
                .entity("BTC-USD-SWAP.OK")
                .unwrap()
                .effective_funding_rate(),
            0.0
        ));
        assert!(approx_eq(
            strategy
                .entity("BTC-USDT-SWAP.OK")
                .unwrap()
                .effective_funding_rate(),
            0.002
        ));
    }

    #[test]
    fn java_parity_prices_spot_buy_from_inverse_hedge() {
        let mut strategy = ChaosStrategy::new(java_calculator_config()).unwrap();
        seed_java_calculator_books(&mut strategy);

        strategy.update_best_hedges();
        strategy.update_theo_quotes();

        let quote = strategy
            .entity("BTC-USDT.OK")
            .unwrap()
            .theo(Side::Buy)
            .unwrap();
        assert!(
            approx_eq(quote.price, 44_947.09722222222),
            "{}",
            quote.price
        );
        assert!(approx_eq(quote.qty, 0.044444444444444446), "{}", quote.qty);
        assert!(quote.hedge_symbol.starts_with("BTC-USD-"));
    }

    #[test]
    fn java_parity_disables_quote_when_group_can_only_self_hedge() {
        let mut strategy = ChaosStrategy::new(java_calculator_config()).unwrap();
        seed_java_calculator_books(&mut strategy);
        strategy.risk_groups.get_mut("OKEX-Spot").unwrap().delta_usd = 25_000.0;

        strategy.update_best_hedges();
        strategy.update_theo_quotes();

        assert!(
            strategy
                .entity("BTC-USDT.OK")
                .unwrap()
                .theo(Side::Buy)
                .is_none()
        );
    }

    #[test]
    fn java_parity_uses_linear_hedge_when_inverse_group_cannot_sell() {
        let mut strategy = ChaosStrategy::new(java_calculator_config()).unwrap();
        seed_java_calculator_books(&mut strategy);
        strategy
            .risk_groups
            .get_mut("OKEX-Invert")
            .unwrap()
            .delta_usd = -40_000.0;

        strategy.update_best_hedges();
        strategy.update_theo_quotes();

        let quote = strategy
            .entity("BTC-USDT.OK")
            .unwrap()
            .theo(Side::Buy)
            .unwrap();
        assert!(
            approx_eq(quote.price, 44_943.01242236025),
            "{}",
            quote.price
        );
        assert!(approx_eq(quote.qty, 0.08), "{}", quote.qty);
        assert!(quote.hedge_symbol.starts_with("BTC-USDT-"));
    }

    #[test]
    fn java_parity_applies_swap_funding_to_spot_quote() {
        let mut strategy = ChaosStrategy::new(java_calculator_config()).unwrap();
        seed_java_calculator_books(&mut strategy);
        strategy
            .risk_groups
            .get_mut("OKEX-Invert")
            .unwrap()
            .delta_usd = -40_000.0;
        strategy
            .entities
            .get_mut("BTC-USDT-SWAP.OK")
            .unwrap()
            .funding_rate = 0.001;

        strategy.update_best_hedges();
        strategy.update_theo_quotes();

        let quote = strategy
            .entity("BTC-USDT.OK")
            .unwrap()
            .theo(Side::Buy)
            .unwrap();
        assert!(
            approx_eq(quote.price, 44_993.01242236025),
            "{}",
            quote.price
        );
        assert_eq!(quote.hedge_symbol, "BTC-USDT-SWAP.OK");
    }

    #[test]
    fn java_parity_prices_inverse_sell_from_spot_hedge() {
        let mut strategy = ChaosStrategy::new(java_calculator_config()).unwrap();
        seed_java_calculator_books(&mut strategy);

        strategy.update_best_hedges();
        strategy.update_theo_quotes();

        for symbol in ["BTC-USD-SWAP.OK", "BTC-USD-211231.OK"] {
            let quote = strategy.entity(symbol).unwrap().theo(Side::Sell).unwrap();
            assert!(approx_eq(quote.price, 55_055.125), "{}", quote.price);
            assert!(approx_eq(quote.qty, 40.0), "{}", quote.qty);
            assert_eq!(quote.hedge_symbol, "BTC-USDT.OK");
        }
    }

    #[test]
    fn java_parity_summarizes_global_sell_hedge() {
        let mut strategy = ChaosStrategy::new(java_calculator_config()).unwrap();
        seed_java_calculator_books(&mut strategy);
        strategy.update_best_hedges();

        let targets = strategy.summarize_hedges(
            strategy.best_hedges.get(&Side::Sell).unwrap(),
            Side::Sell,
            3_000.0,
            None,
        );

        assert_eq!(targets.len(), 1);
        let target = targets
            .iter()
            .find(|target| target.symbol == "BTC-USDT.OK")
            .unwrap();
        assert!(approx_eq(target.orig_px, 45_000.0));
        assert!(approx_eq(target.hedge_px, 44_986.5));
        assert!(approx_eq(target.qty, 0.06));
    }

    #[test]
    fn java_parity_summarizes_inverse_hedges_when_spot_is_blocked() {
        let mut strategy = ChaosStrategy::new(java_calculator_config()).unwrap();
        seed_java_calculator_books(&mut strategy);
        strategy.risk_groups.get_mut("OKEX-Spot").unwrap().delta_usd = -35_000.0;
        strategy.update_best_hedges();

        let targets = strategy.summarize_hedges(
            strategy.best_hedges.get(&Side::Sell).unwrap(),
            Side::Sell,
            3_000.0,
            None,
        );

        assert_eq!(targets.len(), 2);
        let swap = targets
            .iter()
            .find(|target| target.symbol == "BTC-USD-SWAP.OK")
            .unwrap();
        let future = targets
            .iter()
            .find(|target| target.symbol == "BTC-USD-211231.OK")
            .unwrap();
        assert!(approx_eq(swap.orig_px, 45_000.0));
        assert!(approx_eq(swap.hedge_px, 44_986.5));
        assert!(approx_eq(swap.qty, 20.0));
        assert!(approx_eq(future.orig_px, 45_000.0));
        assert!(approx_eq(future.hedge_px, 44_986.5));
        assert!(approx_eq(future.qty, 7.0));
    }

    #[test]
    fn java_parity_summarizes_linear_hedge_when_other_groups_are_blocked() {
        let mut strategy = ChaosStrategy::new(java_calculator_config()).unwrap();
        seed_java_calculator_books(&mut strategy);
        strategy.risk_groups.get_mut("OKEX-Spot").unwrap().delta_usd = -35_000.0;
        strategy
            .risk_groups
            .get_mut("OKEX-Invert")
            .unwrap()
            .delta_usd = -35_000.0;
        strategy.update_best_hedges();

        let targets = strategy.summarize_hedges(
            strategy.best_hedges.get(&Side::Sell).unwrap(),
            Side::Sell,
            3_000.0,
            None,
        );

        assert_eq!(targets.len(), 1);
        let target = &targets[0];
        assert_eq!(target.symbol, "BTC-USDT-211231.OK");
        assert!(approx_eq(target.orig_px, 45_000.0));
        assert!(approx_eq(target.hedge_px, 44_986.5));
        assert!(approx_eq(target.qty, 6.0));
    }

    #[test]
    fn java_parity_summarizes_multi_level_inverse_buy_hedge() {
        let mut strategy = ChaosStrategy::new(java_calculator_config()).unwrap();
        seed_java_calculator_books(&mut strategy);
        strategy.update_best_hedges();
        let hedges = strategy
            .risk_groups
            .get("OKEX-Invert")
            .unwrap()
            .best_hedges_for(Side::Buy);

        let targets = strategy.summarize_hedges(hedges, Side::Buy, 9_000.0, None);

        assert_eq!(targets.len(), 2);
        let swap = targets
            .iter()
            .find(|target| target.symbol == "BTC-USD-SWAP.OK")
            .unwrap();
        let future = targets
            .iter()
            .find(|target| target.symbol == "BTC-USD-211231.OK")
            .unwrap();
        assert!(approx_eq(swap.orig_px, 65_000.0));
        assert!(approx_eq(swap.hedge_px, 65_019.5));
        assert!(approx_eq(swap.qty, 60.0));
        assert!(approx_eq(future.orig_px, 65_000.0));
        assert!(approx_eq(future.hedge_px, 65_019.5));
        assert!(approx_eq(future.qty, 46.0));
    }

    #[test]
    fn java_parity_spot_skew_uses_base_and_quote_balances() {
        let no_skew = spot_skew_state(
            CoinConfig::default(),
            CoinConfig::default(),
            30.0,
            1_000_000.0,
        );
        assert!(approx_eq(no_skew.average_skew_rate_to(0.0), 0.0));
        assert!(approx_eq(no_skew.posn_skew(), 0.0));
        assert!(no_skew.max_hedge_chunk_qty(50_000.0).is_infinite());

        let quote_skew = spot_skew_state(CoinConfig::default(), fixed_quote_skew(), 30.0, 10_000.0);
        assert!(approx_eq(quote_skew.average_skew_rate_to(0.0), 0.005));
        assert!(approx_eq(quote_skew.posn_skew(), -0.0015));
        assert!(approx_eq(quote_skew.max_hedge_chunk_qty(50_000.0), 0.1));

        let base_skew =
            spot_skew_state(fixed_base_skew(), CoinConfig::default(), 29.8, 1_000_000.0);
        assert!(approx_eq(base_skew.average_skew_rate_to(0.0), 0.005));
        assert!(approx_eq(base_skew.posn_skew(), 0.001));
        assert!(approx_eq(base_skew.max_hedge_chunk_qty(50_000.0), 0.1));

        let both_skew = spot_skew_state(fixed_base_skew(), fixed_quote_skew(), 29.8, 10_000.0);
        assert!(approx_eq(both_skew.average_skew_rate_to(0.0), 0.01));
        assert!(approx_eq(both_skew.posn_skew(), -0.0005));
        assert!(approx_eq(both_skew.max_hedge_chunk_qty(50_000.0), 0.05));
    }

    #[test]
    fn java_parity_spot_skew_changes_sign_with_inventory() {
        let base_positive_quote_negative =
            spot_skew_state(fixed_base_skew(), fixed_quote_skew(), 30.2, 10_000.0);
        assert!(approx_eq(base_positive_quote_negative.posn_skew(), -0.0025));

        let base_negative_quote_positive =
            spot_skew_state(fixed_base_skew(), fixed_quote_skew(), 29.8, 40_000.0);
        assert!(approx_eq(base_negative_quote_positive.posn_skew(), 0.0025));

        let both_positive = spot_skew_state(fixed_base_skew(), fixed_quote_skew(), 30.2, 40_000.0);
        assert!(approx_eq(both_positive.posn_skew(), 0.0005));
    }

    #[test]
    fn java_random_matches_seeded_backtest_provider() {
        let mut random = JavaRandom::new(1);
        assert!(approx_eq(random.next_f64(), 0.7308781907032909));
        assert!(approx_eq(random.next_f64(), 0.41008081149220166));
    }

    #[test]
    fn java_parity_builds_configured_mass_quote_levels() {
        let mut cfg = config();
        let spot = cfg
            .instruments
            .iter_mut()
            .find(|instrument| instrument.symbol == "BTC-USDT")
            .unwrap();
        spot.num_quote_levels = 3;
        spot.min_level_spread = 0.001;
        spot.max_level_spread = 0.002;
        spot.min_order_size_usd = 100.0;
        spot.max_order_size_usd = 200.0;
        spot.tick_size = 0.1;
        spot.lot_size = 0.01;

        let mut strategy = ChaosStrategy::new(cfg).unwrap();
        strategy.entities.get_mut("BTC-USDT").unwrap().book = Some(OrderBook::one_level(
            "BTC-USDT",
            1,
            Level::new(99.0, 10.0),
            Level::new(101.0, 10.0),
        ));
        strategy.now_ms = 10_000;
        let levels = strategy.desired_quote_levels(
            "BTC-USDT",
            Side::Buy,
            Some(TheoQuote {
                price: 100.0,
                qty: 1.5,
                hedge_px: 101.0,
                hedge_symbol: "BTC-PERP".to_string(),
            }),
        );

        assert_eq!(levels.len(), 3);
        assert!(approx_eq(levels[0].price, 100.0));
        assert!(levels[1].price < levels[0].price);
        assert!(levels[2].price < levels[1].price);
        assert!(levels[1].qty >= 100.0 / 100.0);
        assert!(levels[1].qty <= 200.0 / 100.0);

        let mut intents = Vec::new();
        strategy.sync_quotes("BTC-USDT", Side::Buy, &levels, &mut intents);
        assert_eq!(intents.len(), 3);
        assert!(matches!(&intents[0], OrderIntent::NewOrder(order) if order.reason == "quote"));
        assert!(matches!(&intents[1], OrderIntent::NewOrder(order) if order.reason == "quote:1"));
        assert!(matches!(&intents[2], OrderIntent::NewOrder(order) if order.reason == "quote:2"));
    }

    #[test]
    fn java_parity_pending_delta_includes_live_hedges_only() {
        let mut strategy = ChaosStrategy::new(config()).unwrap();
        strategy.entities.get_mut("BTC-USDT").unwrap().book = Some(OrderBook::one_level(
            "BTC-USDT",
            1,
            Level::new(50_000.0, 1.0),
            Level::new(50_002.0, 1.0),
        ));
        strategy.on_order_update(&OrderUpdate {
            ts_ms: 2,
            order_id: "h1".to_string(),
            symbol: "BTC-PERP".to_string(),
            side: Side::Sell,
            event: OrderEvent::New,
            status: OrderStatus::Live,
            price: 49_990.0,
            qty: 100.0,
            open_qty: 100.0,
            filled_qty: 0.0,
            avg_fill_price: 0.0,
            last_fill_qty: 0.0,
            last_fill_price: 0.0,
            last_fill_liquidity: None,
            reason: "hedge:BTC-USDT:50000".to_string(),
        });
        strategy.update_risk();

        assert!(approx_eq(strategy.delta_usd(), 0.0));
        assert!(approx_eq(strategy.pending_delta_usd(), -5_000.1));
    }

    #[test]
    fn java_parity_does_not_reuse_pending_hedge_liquidity() {
        let mut strategy = ChaosStrategy::new(java_calculator_config()).unwrap();
        seed_java_calculator_books(&mut strategy);
        let entity = strategy.entity("BTC-USD-SWAP.OK").unwrap();
        let level = entity.hedge_levels(Side::Sell, 50_000.0, &[])[0].to_owned_level();
        strategy.active_hedges.insert(
            "h1".to_string(),
            ActiveHedge {
                symbol: level.symbol.clone(),
                signed_open_qty: -level.qty,
                price: 44_986.5,
                reference_price: level.px,
                updated_ms: 1,
            },
        );

        let targets = strategy.summarize_hedges(
            std::slice::from_ref(&level),
            Side::Sell,
            level.notional_usd,
            None,
        );

        assert!(targets.is_empty());
    }

    #[test]
    fn java_parity_latches_spot_index_deviation_after_debounce() {
        let mut cfg = config();
        cfg.index_deviation_limit = 0.05;
        cfg.index_deviation_debounce_ms = 100;
        cfg.instruments
            .iter_mut()
            .find(|instrument| instrument.symbol == "BTC-USDT")
            .unwrap()
            .index_symbol = Some("BTC-INDEX".to_string());
        let mut strategy = ChaosStrategy::new(cfg).unwrap();
        strategy.on_depth(&OrderBook::one_level(
            "BTC-USDT",
            1,
            Level::new(99.0, 10.0),
            Level::new(101.0, 10.0),
        ));
        strategy.on_depth(&OrderBook::one_level(
            "BTC-PERP",
            1,
            Level::new(99.0, 100_000.0),
            Level::new(101.0, 100_000.0),
        ));

        strategy.on_market_event(&MarketEvent::IndexPrice {
            ts_ms: 10,
            symbol: "BTC-INDEX".to_string(),
            price: 100.0,
        });
        strategy.on_market_event(&MarketEvent::IndexPrice {
            ts_ms: 11,
            symbol: "BTC-INDEX".to_string(),
            price: 80.0,
        });
        assert!(strategy.halt_reason().is_none());
        strategy.on_market_event(&MarketEvent::IndexPrice {
            ts_ms: 110,
            symbol: "BTC-INDEX".to_string(),
            price: 80.0,
        });
        assert!(strategy.halt_reason().is_none());
        let intents = strategy.on_market_event(&MarketEvent::IndexPrice {
            ts_ms: 111,
            symbol: "BTC-INDEX".to_string(),
            price: 80.0,
        });

        assert!(strategy.halt_reason().is_some());
        assert!(
            intents
                .iter()
                .all(|intent| !matches!(intent, OrderIntent::NewOrder(_)))
        );
    }

    #[test]
    fn unrelated_index_prices_do_not_enter_strategy_pricing_state() {
        let mut strategy = ChaosStrategy::new(config()).unwrap();
        let intents = strategy.on_market_event(&MarketEvent::IndexPrice {
            ts_ms: 10,
            symbol: "USDT-USD".to_string(),
            price: 1.0,
        });

        assert!(intents.is_empty());
        assert!(!strategy.index_prices.contains_key("USDT-USD"));
        assert_eq!(strategy.now_ms, 10);
    }

    #[test]
    fn java_parity_burst_adjusts_one_quote_side_and_hedge_aggression() {
        let mut cfg = java_calculator_config();
        cfg.act_on_burst = true;
        let mut strategy = ChaosStrategy::new(cfg).unwrap();
        seed_java_calculator_books(&mut strategy);
        strategy.update_best_hedges();
        strategy.update_theo_quotes();
        let baseline_buy = strategy
            .entity("BTC-USDT.OK")
            .unwrap()
            .theo(Side::Buy)
            .unwrap()
            .price;
        let baseline_sell = strategy
            .entity("BTC-USDT.OK")
            .unwrap()
            .theo(Side::Sell)
            .unwrap()
            .price;

        strategy.on_market_event(&MarketEvent::BurstSignal {
            ts_ms: 2,
            symbol: "BTC-USDT.OK".to_string(),
            value: 0.001,
        });

        let burst_buy = strategy
            .entity("BTC-USDT.OK")
            .unwrap()
            .theo(Side::Buy)
            .unwrap()
            .price;
        let burst_sell = strategy
            .entity("BTC-USDT.OK")
            .unwrap()
            .theo(Side::Sell)
            .unwrap()
            .price;
        assert!(approx_eq(burst_buy, baseline_buy));
        assert!(approx_eq(burst_sell, baseline_sell + 50.0));

        let targets = strategy.summarize_hedges(
            strategy.best_hedges.get(&Side::Buy).unwrap(),
            Side::Buy,
            9_000.0,
            None,
        );
        let spot = targets
            .iter()
            .find(|target| target.symbol == "BTC-USDT.OK")
            .unwrap();
        assert!(approx_eq(spot.hedge_px, 55_055.0));

        strategy.on_market_event(&MarketEvent::BurstSignal {
            ts_ms: 3,
            symbol: "BTC-USDT-SWAP.OK".to_string(),
            value: -0.002,
        });
        assert!(approx_eq(strategy.burst, 0.0));
        assert!(strategy.burst_symbol.is_none());
    }

    #[test]
    fn java_parity_ignore_best_level_uses_second_raw_level() {
        let mut cfg = config();
        cfg.ignore_best_level = true;
        let mut strategy = ChaosStrategy::new(cfg).unwrap();
        let entity = strategy.entities.get_mut("BTC-USDT").unwrap();
        entity.book = Some(OrderBook {
            symbol: "BTC-USDT".to_string(),
            ts_ms: 1,
            bids: vec![Level::new(46_000.0, 1.0), Level::new(45_000.0, 1.0)],
            asks: vec![Level::new(54_000.0, 1.0), Level::new(55_000.0, 1.0)],
        });

        assert!(approx_eq(entity.mid().unwrap(), 50_000.0));
        assert!(approx_eq(
            entity.effective_levels(Side::Buy)[0].px,
            45_000.0
        ));
        assert!(approx_eq(
            entity.effective_levels(Side::Sell)[0].px,
            55_000.0
        ));
    }

    #[test]
    fn java_parity_halts_derivative_during_utc_interval() {
        let mut cfg = config();
        cfg.instruments
            .iter_mut()
            .find(|instrument| instrument.symbol == "BTC-PERP")
            .unwrap()
            .halt_intervals = vec![HaltIntervalConfig {
            start_sec_utc: 10,
            end_sec_utc: 20,
        }];
        let mut strategy = ChaosStrategy::new(cfg).unwrap();
        strategy.entities.get_mut("BTC-USDT").unwrap().book = Some(OrderBook::one_level(
            "BTC-USDT",
            1,
            Level::new(50_000.0, 10.0),
            Level::new(50_001.0, 10.0),
        ));
        strategy.entities.get_mut("BTC-PERP").unwrap().book = Some(OrderBook::one_level(
            "BTC-PERP",
            1,
            Level::new(50_000.0, 10_000.0),
            Level::new(50_001.0, 10_000.0),
        ));

        strategy.now_ms = 15_000;
        let halted = strategy.refresh_quotes();
        assert!(strategy.entity("BTC-PERP").unwrap().interval_halted);
        assert!(
            halted
                .iter()
                .all(|intent| !matches!(intent, OrderIntent::NewOrder(_)))
        );

        strategy.now_ms = 21_000;
        let resumed = strategy.refresh_quotes();
        assert!(!strategy.entity("BTC-PERP").unwrap().interval_halted);
        assert!(
            resumed
                .iter()
                .any(|intent| matches!(intent, OrderIntent::NewOrder(_)))
        );
    }

    #[test]
    fn java_parity_quote_only_stays_at_top_of_book() {
        let mut entity = InstrumentState::new(InstrumentConfig {
            tick_size: 1.0,
            ..InstrumentConfig::default()
        });
        entity.book = Some(OrderBook::one_level(
            "BTC-USDT",
            1,
            Level::new(100.0, 1.0),
            Level::new(105.0, 1.0),
        ));

        assert!(approx_eq(entity.quote_only_price(Side::Buy, 103.0), 101.0));
        assert!(approx_eq(entity.quote_only_price(Side::Sell, 102.0), 104.0));
        assert!(approx_eq(entity.quote_only_price(Side::Buy, 99.0), 99.0));
        assert!(approx_eq(entity.quote_only_price(Side::Sell, 106.0), 106.0));
    }

    #[test]
    fn java_parity_separates_self_crossing_theoretical_quotes() {
        let mut entity = InstrumentState::new(InstrumentConfig {
            quote_profit_margin: 0.001,
            ..InstrumentConfig::default()
        });
        entity.book = Some(OrderBook::one_level(
            "BTC-USDT",
            1,
            Level::new(99.0, 1.0),
            Level::new(101.0, 1.0),
        ));
        entity.buy_theo = Some(TheoQuote {
            price: 101.0,
            qty: 1.0,
            hedge_px: 100.0,
            hedge_symbol: "hedge".to_string(),
        });
        entity.sell_theo = Some(TheoQuote {
            price: 99.0,
            qty: 1.0,
            hedge_px: 100.0,
            hedge_symbol: "hedge".to_string(),
        });

        entity.prevent_self_cross();

        assert!(approx_eq(entity.buy_theo.unwrap().price, 99.9));
        assert!(approx_eq(entity.sell_theo.unwrap().price, 100.1));
    }

    #[test]
    fn java_parity_account_balances_drive_spot_group_delta() {
        let mut cfg = config();
        cfg.coin_offset = 30.0;
        cfg.risk_groups[0].coin_offset = 30.0;
        cfg.risk_groups[0].kind = RiskGroupKindConfig::PortfolioAccount;
        cfg.risk_groups[0].coins = vec![
            CoinConfig {
                currency: "BTC".to_string(),
                min_balance: 20.0,
                max_balance: 40.0,
                borrow_limit_usd: 50_000.0,
                borrow_limit_coin: 1.0,
                ..CoinConfig::default()
            },
            CoinConfig {
                currency: "USDT".to_string(),
                min_balance: 0.0,
                max_balance: 2_000_000.0,
                ..CoinConfig::default()
            },
        ];
        let spot = cfg
            .instruments
            .iter_mut()
            .find(|instrument| instrument.symbol == "BTC-USDT")
            .unwrap();
        spot.base_currency = "BTC".to_string();
        spot.quote_currency = "USDT".to_string();
        let mut strategy = ChaosStrategy::new(cfg).unwrap();
        strategy.entities.get_mut("BTC-USDT").unwrap().book = Some(OrderBook::one_level(
            "BTC-USDT",
            1,
            Level::new(49_999.0, 1.0),
            Level::new(50_001.0, 1.0),
        ));

        strategy.on_account_update(&AccountUpdate {
            ts_ms: 2,
            balances: vec![
                Balance {
                    account_id: None,
                    currency: "BTC".to_string(),
                    total: 31.0,
                    available: 30.5,
                    equity: 31.0,
                    liability: 0.5,
                    max_loan: 1.0,
                },
                Balance {
                    account_id: None,
                    currency: "USDT".to_string(),
                    total: 1_000_000.0,
                    available: 1_000_000.0,
                    equity: 1_000_000.0,
                    liability: 0.0,
                    max_loan: 0.0,
                },
            ],
            positions: Vec::new(),
            margins: Vec::new(),
        });

        let spot = strategy.entity("BTC-USDT").unwrap();
        assert!(approx_eq(spot.base_balance, 31.0));
        assert!(approx_eq(spot.base_liability, 0.5));
        assert!(approx_eq(strategy.delta_usd(), 50_000.0));
    }

    #[test]
    fn java_parity_latches_delta_limit_breach_and_stops_new_quotes() {
        let mut cfg = config();
        cfg.delta_limit_usd = 10_000.0;
        let mut strategy = ChaosStrategy::new(cfg).unwrap();
        let spot = strategy.entities.get_mut("BTC-USDT").unwrap();
        spot.book = Some(OrderBook::one_level(
            "BTC-USDT",
            1,
            Level::new(49_999.0, 10.0),
            Level::new(50_001.0, 10.0),
        ));
        spot.position_qty = 1.0;
        strategy.entities.get_mut("BTC-PERP").unwrap().book = Some(OrderBook::one_level(
            "BTC-PERP",
            1,
            Level::new(49_999.0, 10_000.0),
            Level::new(50_001.0, 10_000.0),
        ));

        let intents = strategy.refresh_quotes();

        assert!(strategy.halt_reason().unwrap().contains("strategy delta"));
        assert!(
            intents
                .iter()
                .all(|intent| !matches!(intent, OrderIntent::NewOrder(_)))
        );
    }

    #[test]
    fn java_parity_latches_trading_pnl_breach() {
        let mut cfg = config();
        cfg.pnl_limit_usd = 10.0;
        cfg.pnl_breach_debounce_ms = 0;
        let mut strategy = ChaosStrategy::new(cfg).unwrap();
        strategy.entities.get_mut("BTC-USDT").unwrap().book = Some(OrderBook::one_level(
            "BTC-USDT",
            1,
            Level::new(79.0, 10.0),
            Level::new(81.0, 10.0),
        ));
        strategy.entities.get_mut("BTC-PERP").unwrap().book = Some(OrderBook::one_level(
            "BTC-PERP",
            1,
            Level::new(79.0, 10_000.0),
            Level::new(81.0, 10_000.0),
        ));
        strategy.on_order_update(&OrderUpdate {
            ts_ms: 2,
            order_id: "q1".to_string(),
            symbol: "BTC-USDT".to_string(),
            side: Side::Buy,
            event: OrderEvent::FullyFilled,
            status: OrderStatus::Filled,
            price: 100.0,
            qty: 1.0,
            open_qty: 0.0,
            filled_qty: 1.0,
            avg_fill_price: 100.0,
            last_fill_qty: 1.0,
            last_fill_price: 100.0,
            last_fill_liquidity: None,
            reason: "quote".to_string(),
        });

        let intents = strategy.refresh_quotes();

        assert!(approx_eq(strategy.trading_pnl_usd(), -20.02));
        assert!(strategy.halt_reason().unwrap().contains("trading pnl"));
        assert!(
            intents
                .iter()
                .all(|intent| !matches!(intent, OrderIntent::NewOrder(_)))
        );
    }

    #[test]
    fn java_parity_debounces_margin_ratio_breach() {
        let mut cfg = config();
        cfg.margin_breach_debounce_ms = 100;
        cfg.risk_groups[0].min_margin_level = 0.3;
        let mut strategy = ChaosStrategy::new(cfg).unwrap();
        strategy.on_account_update(&AccountUpdate {
            ts_ms: 10,
            balances: Vec::new(),
            positions: Vec::new(),
            margins: vec![MarginSnapshot {
                account_id: None,
                ratio: Some(0.4),
                exchange_ratio: None,
                adjusted_equity_usd: Some(40_000.0),
                notional_usd: Some(100_000.0),
            }],
        });
        strategy.on_account_update(&AccountUpdate {
            ts_ms: 11,
            balances: Vec::new(),
            positions: Vec::new(),
            margins: vec![MarginSnapshot {
                account_id: None,
                ratio: Some(0.2),
                exchange_ratio: None,
                adjusted_equity_usd: Some(20_000.0),
                notional_usd: Some(100_000.0),
            }],
        });
        assert!(strategy.halt_reason().is_none());

        strategy.on_event(&StrategyEvent::Timer(reap_core::TimerEvent {
            ts_ms: 110,
            name: "risk".to_string(),
        }));
        assert!(strategy.halt_reason().is_none());
        strategy.on_event(&StrategyEvent::Timer(reap_core::TimerEvent {
            ts_ms: 111,
            name: "risk".to_string(),
        }));

        assert!(strategy.halt_reason().unwrap().contains("margin ratio"));
    }

    #[test]
    fn java_parity_respects_top_quote_refill_interval() {
        let mut strategy = ChaosStrategy::new(config()).unwrap();
        strategy.entities.get_mut("BTC-USDT").unwrap().book = Some(OrderBook::one_level(
            "BTC-USDT",
            1,
            Level::new(99.0, 10.0),
            Level::new(101.0, 10.0),
        ));
        let levels = vec![TheoQuote {
            price: 100.0,
            qty: 1.0,
            hedge_px: 101.0,
            hedge_symbol: "BTC-PERP".to_string(),
        }];
        strategy.on_order_update(&OrderUpdate {
            ts_ms: 10,
            order_id: "q1".to_string(),
            symbol: "BTC-USDT".to_string(),
            side: Side::Buy,
            event: OrderEvent::New,
            status: OrderStatus::Live,
            price: 100.0,
            qty: 1.0,
            open_qty: 1.0,
            filled_qty: 0.0,
            avg_fill_price: 0.0,
            last_fill_qty: 0.0,
            last_fill_price: 0.0,
            last_fill_liquidity: None,
            reason: "quote".to_string(),
        });
        strategy.on_order_update(&OrderUpdate {
            ts_ms: 100,
            order_id: "q1".to_string(),
            symbol: "BTC-USDT".to_string(),
            side: Side::Buy,
            event: OrderEvent::FullyFilled,
            status: OrderStatus::Filled,
            price: 100.0,
            qty: 1.0,
            open_qty: 0.0,
            filled_qty: 1.0,
            avg_fill_price: 100.0,
            last_fill_qty: 1.0,
            last_fill_price: 100.0,
            last_fill_liquidity: Some(FillLiquidity::Maker),
            reason: "quote".to_string(),
        });

        strategy.now_ms = 399;
        let mut blocked = Vec::new();
        strategy.sync_quotes("BTC-USDT", Side::Buy, &levels, &mut blocked);
        assert!(blocked.is_empty());

        strategy.now_ms = 400;
        let mut refill = Vec::new();
        strategy.sync_quotes("BTC-USDT", Side::Buy, &levels, &mut refill);
        assert!(matches!(refill.as_slice(), [OrderIntent::NewOrder(_)]));
    }

    #[test]
    fn java_parity_conflates_quote_changes_within_debounce_interval() {
        let mut cfg = config();
        cfg.instruments
            .iter_mut()
            .find(|instrument| instrument.symbol == "BTC-USDT")
            .unwrap()
            .tick_size = 0.01;
        let mut strategy = ChaosStrategy::new(cfg).unwrap();
        strategy.entities.get_mut("BTC-USDT").unwrap().book = Some(OrderBook::one_level(
            "BTC-USDT",
            1,
            Level::new(99.0, 10.0),
            Level::new(101.0, 10.0),
        ));
        let quote = |price| TheoQuote {
            price,
            qty: 1.0,
            hedge_px: 101.0,
            hedge_symbol: "BTC-PERP".to_string(),
        };

        strategy.now_ms = 100;
        let initial = strategy.desired_quote_levels("BTC-USDT", Side::Buy, Some(quote(100.0)));
        strategy.now_ms = 110;
        let conflated = strategy.desired_quote_levels("BTC-USDT", Side::Buy, Some(quote(100.05)));
        strategy.now_ms = 130;
        let updated = strategy.desired_quote_levels("BTC-USDT", Side::Buy, Some(quote(100.05)));

        assert!(approx_eq(initial[0].price, 100.0));
        assert!(approx_eq(conflated[0].price, 100.0));
        assert!(approx_eq(updated[0].price, 100.05));
    }

    #[test]
    fn java_parity_timer_hedges_strategy_delta_without_symbol_exclusion() {
        let mut strategy = ChaosStrategy::new(config()).unwrap();
        strategy.entities.get_mut("BTC-USDT").unwrap().book = Some(OrderBook::one_level(
            "BTC-USDT",
            1,
            Level::new(50_000.0, 1.0),
            Level::new(50_001.0, 1.0),
        ));
        strategy.entities.get_mut("BTC-PERP").unwrap().book = Some(OrderBook::one_level(
            "BTC-PERP",
            1,
            Level::new(50_000.0, 10_000.0),
            Level::new(50_001.0, 10_000.0),
        ));
        strategy.entities.get_mut("BTC-USDT").unwrap().position_qty = 0.1;

        let intents = strategy.on_event(&StrategyEvent::Timer(reap_core::TimerEvent {
            ts_ms: 2_000,
            name: "risk".to_string(),
        }));

        assert!(intents.iter().any(|intent| matches!(intent, OrderIntent::NewOrder(order)
            if order.time_in_force == TimeInForce::Ioc && order.reason.starts_with("hedge:timer:"))));
    }

    #[test]
    fn java_parity_reduces_spot_quote_levels_until_balance_recovers() {
        let mut entity = InstrumentState::new(InstrumentConfig {
            symbol: "BTC-USDT".to_string(),
            kind: InstrumentKindConfig::Spot,
            base_currency: "BTC".to_string(),
            quote_currency: "USDT".to_string(),
            num_quote_levels: 3,
            max_order_size: 1.0,
            min_trade_size: 0.01,
            ..InstrumentConfig::default()
        });
        entity.book = Some(OrderBook::one_level(
            "BTC-USDT",
            1,
            Level::new(99.0, 10.0),
            Level::new(101.0, 10.0),
        ));
        entity.base_coin_config = Some(CoinConfig {
            currency: "BTC".to_string(),
            min_balance: 0.0,
            max_balance: 100.0,
            safety_multiplier: 1.0,
            ..CoinConfig::default()
        });
        entity.quote_coin_config = Some(CoinConfig {
            currency: "USDT".to_string(),
            min_balance: 0.0,
            max_balance: 10_000.0,
            safety_multiplier: 1.0,
            ..CoinConfig::default()
        });
        entity.base_balance = 10.0;
        entity.base_equity = 10.0;
        entity.quote_balance = 150.0;
        entity.quote_equity = 150.0;
        entity.balances_initialized = true;

        entity.refresh_trade_permissions(100);
        assert!(entity.can_quote(Side::Buy));
        assert_eq!(entity.quote_level_count(Side::Buy), 1);

        entity.quote_balance = 400.0;
        entity.quote_equity = 400.0;
        entity.refresh_trade_permissions(200);
        assert_eq!(entity.quote_level_count(Side::Buy), 1);
        entity.quote_balance = 1_000.0;
        entity.quote_equity = 1_000.0;
        entity.refresh_trade_permissions(10_200);
        assert_eq!(entity.quote_level_count(Side::Buy), 1);
        entity.refresh_trade_permissions(10_201);
        assert_eq!(entity.quote_level_count(Side::Buy), 3);
    }

    #[test]
    fn java_parity_debounces_trade_permission_recovery() {
        let mut entity = InstrumentState::new(InstrumentConfig {
            symbol: "BTC-USDT-SWAP".to_string(),
            kind: InstrumentKindConfig::LinearSwap,
            max_order_size: 1.0,
            min_trade_size: 1.0,
            min_position: -100.0,
            max_position: 100.0,
            safety_multiplier: 1.0,
            ..InstrumentConfig::default()
        });
        entity.book = Some(OrderBook::one_level(
            "BTC-USDT-SWAP",
            1,
            Level::new(99.0, 10.0),
            Level::new(101.0, 10.0),
        ));
        entity.position_qty = 99.0;
        entity.refresh_trade_permissions(100);
        assert!(!entity.can_trade[&Side::Buy]);

        entity.position_qty = 0.0;
        entity.refresh_trade_permissions(600);
        assert!(!entity.can_trade[&Side::Buy]);
        entity.refresh_trade_permissions(601);
        assert!(entity.can_trade[&Side::Buy]);
    }

    #[test]
    fn java_parity_startup_basis_uses_one_third_limit() {
        let mut strategy = ChaosStrategy::new(config()).unwrap();
        strategy.on_depth(&OrderBook::one_level(
            "BTC-USDT",
            1,
            Level::new(99.0, 10.0),
            Level::new(101.0, 10.0),
        ));
        strategy.on_depth(&OrderBook::one_level(
            "BTC-PERP",
            1,
            Level::new(110.0, 10_000.0),
            Level::new(112.0, 10_000.0),
        ));

        assert_eq!(strategy.halt_reason(), Some("startup basis limit breached"));
    }

    #[test]
    fn java_parity_runtime_basis_breach_is_diagnostic_only() {
        let mut strategy = ChaosStrategy::new(config()).unwrap();
        strategy.on_depth(&OrderBook::one_level(
            "BTC-USDT",
            1,
            Level::new(99.0, 10.0),
            Level::new(101.0, 10.0),
        ));
        strategy.on_depth(&OrderBook::one_level(
            "BTC-PERP",
            1,
            Level::new(100.0, 10_000.0),
            Level::new(102.0, 10_000.0),
        ));
        assert!(strategy.halt_reason().is_none());

        strategy.on_depth(&OrderBook::one_level(
            "BTC-PERP",
            6_002,
            Level::new(110.0, 10_000.0),
            Level::new(112.0, 10_000.0),
        ));

        assert!(strategy.halt_reason().is_none());
        assert!(strategy.basis_breaches().contains_key("main"));
    }

    #[test]
    fn java_parity_local_account_hedge_ignores_strategy_interval() {
        let mut cfg = config();
        cfg.min_hedge_interval_ms = 100_000;
        let mut strategy = ChaosStrategy::new(cfg).unwrap();
        strategy.on_depth(&OrderBook::one_level(
            "BTC-USDT",
            1,
            Level::new(50_000.0, 10.0),
            Level::new(50_001.0, 10.0),
        ));
        strategy.on_depth(&OrderBook::one_level(
            "BTC-PERP",
            1,
            Level::new(50_000.0, 10_000.0),
            Level::new(50_001.0, 10_000.0),
        ));
        strategy.last_hedge_ms = 10;

        let intents = strategy.on_account_update(&AccountUpdate {
            ts_ms: 20,
            balances: Vec::new(),
            positions: vec![reap_core::Position {
                symbol: "BTC-USDT".to_string(),
                qty: 0.1,
                avg_price: 50_000.0,
            }],
            margins: Vec::new(),
        });

        assert!(intents.iter().any(|intent| matches!(intent, OrderIntent::NewOrder(order)
            if order.symbol == "BTC-PERP" && order.side == Side::Sell && order.time_in_force == TimeInForce::Ioc)));
        assert_eq!(strategy.last_hedge_ms, 10);
    }

    #[test]
    fn java_parity_excludes_own_quotes_from_hedge_depth() {
        let mut entity = InstrumentState::new(InstrumentConfig {
            symbol: "BTC-USDT".to_string(),
            kind: InstrumentKindConfig::Spot,
            min_trade_size: 0.01,
            ..InstrumentConfig::default()
        });
        entity.book = Some(OrderBook::one_level(
            "BTC-USDT",
            1,
            Level::new(99.0, 2.0),
            Level::new(101.0, 2.0),
        ));

        let levels = entity.hedge_levels(Side::Buy, 100.0, &[(Side::Sell, 101.0, 1.5)]);

        assert!(approx_eq(levels[0].qty, 0.5));
    }

    #[test]
    fn hedge_candidates_stop_after_covering_notional_target() {
        let mut entity = InstrumentState::new(InstrumentConfig {
            symbol: "BTC-USDT".to_string(),
            kind: InstrumentKindConfig::Spot,
            lot_size: 0.01,
            min_trade_size: 0.01,
            ..InstrumentConfig::default()
        });
        entity.book = Some(OrderBook {
            symbol: "BTC-USDT".to_string(),
            ts_ms: 1,
            bids: (0..100)
                .map(|level| Level::new(99.0 - level as f64 * 0.01, 1.0))
                .collect(),
            asks: (0..100)
                .map(|level| Level::new(101.0 + level as f64 * 0.01, 1.0))
                .collect(),
        });
        let mut candidates = Vec::new();

        entity.append_hedge_candidates(Side::Buy, 100.0, &[], 150.0, &mut candidates);

        assert_eq!(candidates.len(), 2);
        assert!(
            candidates
                .iter()
                .map(|level| level.notional_usd)
                .sum::<f64>()
                >= 150.0
        );
    }

    #[test]
    fn hedge_selection_preserves_rate_order_until_alternative_coverage() {
        let strategy = ChaosStrategy::new(config()).unwrap();
        let candidate = |symbol, level| HedgeCandidate {
            symbol: Arc::from(symbol),
            priority: 0,
            level,
            px: 100.0,
            qty: 1_000.0,
            hedge_rate: 1.0,
            notional_usd: 100_000.0,
            acc_qty: 1_000.0,
        };
        let levels = vec![
            candidate("BTC-USDT", 0),
            candidate("BTC-USDT", 1),
            candidate("BTC-PERP", 0),
        ];

        let selected = strategy.select_required_hedges("main", Side::Buy, &levels);

        assert_eq!(selected.len(), 3);
        assert_eq!(selected[0].symbol, "BTC-USDT");
        assert_eq!(selected[1].symbol, "BTC-USDT");
        assert_eq!(selected[2].symbol, "BTC-PERP");
    }

    #[test]
    fn java_parity_stops_after_six_consecutive_anomalous_fills() {
        let mut strategy = ChaosStrategy::new(config()).unwrap();
        strategy.entities.get_mut("BTC-USDT").unwrap().book = Some(OrderBook::one_level(
            "BTC-USDT",
            1,
            Level::new(99.0, 10.0),
            Level::new(101.0, 10.0),
        ));
        for index in 0..6 {
            strategy.on_order_update(&OrderUpdate {
                ts_ms: 3_000 + index * 100,
                order_id: format!("q{index}"),
                symbol: "BTC-USDT".to_string(),
                side: Side::Buy,
                event: OrderEvent::FullyFilled,
                status: OrderStatus::Filled,
                price: 100.0,
                qty: 0.01,
                open_qty: 0.0,
                filled_qty: 0.01,
                avg_fill_price: 98.0,
                last_fill_qty: 0.01,
                last_fill_price: 98.0,
                last_fill_liquidity: Some(FillLiquidity::Maker),
                reason: "quote".to_string(),
            });
            if index < 5 {
                assert!(strategy.halt_reason().is_none());
            }
        }
        assert!(
            strategy
                .halt_reason()
                .unwrap()
                .contains("consecutive anomalous fills")
        );
    }

    #[test]
    fn java_parity_stops_when_fewer_than_two_books_remain_valid() {
        let mut cfg = config();
        cfg.insufficient_valid_stop_ms = 100;
        for instrument in &mut cfg.instruments {
            instrument.depth_stale_threshold_ms = 10;
        }
        let mut strategy = ChaosStrategy::new(cfg).unwrap();
        strategy.on_depth(&OrderBook::one_level(
            "BTC-USDT",
            1,
            Level::new(99.0, 10.0),
            Level::new(101.0, 10.0),
        ));
        strategy.on_depth(&OrderBook::one_level(
            "BTC-PERP",
            1,
            Level::new(99.0, 10_000.0),
            Level::new(101.0, 10_000.0),
        ));
        strategy.on_event(&StrategyEvent::Timer(reap_core::TimerEvent {
            ts_ms: 12,
            name: "risk".to_string(),
        }));
        assert!(strategy.halt_reason().is_none());
        strategy.on_event(&StrategyEvent::Timer(reap_core::TimerEvent {
            ts_ms: 113,
            name: "risk".to_string(),
        }));
        assert!(
            strategy
                .halt_reason()
                .unwrap()
                .contains("fewer than two instruments")
        );
    }

    #[test]
    fn java_parity_enforces_exchange_price_limits() {
        let mut entity = InstrumentState::new(InstrumentConfig {
            price_limit_buffer: 0.01,
            ..InstrumentConfig::default()
        });
        entity.book = Some(OrderBook {
            symbol: "BTC-USDT".to_string(),
            ts_ms: 1,
            bids: vec![Level::new(95.0, 1.0), Level::new(94.0, 1.0)],
            asks: vec![Level::new(105.0, 1.0), Level::new(106.0, 1.0)],
        });
        entity.limit_up = Some(100.0);
        entity.limit_down = Some(90.0);

        assert!(approx_eq(entity.px_within_limit(Side::Buy, 102.0), 99.0));
        assert!(approx_eq(entity.px_within_limit(Side::Sell, 88.0), 94.0));
        assert!(!entity.can_take_within_price_limit(Side::Buy));
        assert!(entity.can_take_within_price_limit(Side::Sell));
    }

    #[test]
    fn java_parity_master_strategy_suppresses_automatic_hedges() {
        let mut cfg = config();
        cfg.master_strategy = Some("leader".to_string());
        let mut strategy = ChaosStrategy::new(cfg).unwrap();
        strategy.entities.get_mut("BTC-USDT").unwrap().book = Some(OrderBook::one_level(
            "BTC-USDT",
            1,
            Level::new(49_900.0, 10.0),
            Level::new(50_100.0, 10.0),
        ));
        strategy.entities.get_mut("BTC-PERP").unwrap().book = Some(OrderBook::one_level(
            "BTC-PERP",
            1,
            Level::new(49_900.0, 10_000.0),
            Level::new(50_100.0, 10_000.0),
        ));

        let account_intents = strategy.on_account_update(&AccountUpdate {
            ts_ms: 10,
            balances: Vec::new(),
            positions: vec![reap_core::Position {
                symbol: "BTC-USDT".to_string(),
                qty: 0.1,
                avg_price: 50_000.0,
            }],
            margins: Vec::new(),
        });
        let timer_intents = strategy.on_event(&StrategyEvent::Timer(reap_core::TimerEvent {
            ts_ms: 11,
            name: "risk".to_string(),
        }));

        assert!(account_intents.iter().chain(&timer_intents).all(|intent| {
            !matches!(intent, OrderIntent::NewOrder(order) if order.time_in_force == TimeInForce::Ioc)
        }));
    }

    #[test]
    fn java_parity_records_unfilled_ioc_hedge_delta() {
        let mut strategy = ChaosStrategy::new(config()).unwrap();
        strategy.entities.get_mut("BTC-USDT").unwrap().book = Some(OrderBook::one_level(
            "BTC-USDT",
            1,
            Level::new(99.0, 10.0),
            Level::new(101.0, 10.0),
        ));

        strategy.on_order_update(&OrderUpdate {
            ts_ms: 20,
            order_id: "hedge-1".to_string(),
            symbol: "BTC-PERP".to_string(),
            side: Side::Sell,
            event: OrderEvent::Cancelled,
            status: OrderStatus::Cancelled,
            price: 100.0,
            qty: 1_000.0,
            open_qty: 0.0,
            filled_qty: 200.0,
            avg_fill_price: 100.0,
            last_fill_qty: 0.0,
            last_fill_price: 0.0,
            last_fill_liquidity: None,
            reason: "hedge:BTC-USDT:100".to_string(),
        });

        let missed = strategy.missed_hedges().last().unwrap();
        assert_eq!(missed.order_id, "hedge-1");
        assert_eq!(missed.missed_qty, 800.0);
        assert_eq!(missed.reference_symbol.as_deref(), Some("BTC-USDT"));
        assert!(missed.missed_delta_usd.is_finite() && missed.missed_delta_usd < 0.0);
    }

    #[test]
    fn java_parity_removes_halted_and_stale_symbols_from_hedges() {
        let mut strategy = ChaosStrategy::new(java_calculator_config()).unwrap();
        seed_java_calculator_books(&mut strategy);
        strategy.update_best_hedges();
        assert!(
            strategy
                .best_hedges
                .values()
                .flatten()
                .any(|level| { level.symbol == "BTC-USD-SWAP.OK" })
        );

        strategy.on_system_event(&SystemEvent {
            ts_ms: 2,
            kind: SystemEventKind::SymbolHalted,
            venue: None,
            account_id: None,
            symbol: Some("BTC-USD-SWAP.OK".to_string()),
            reason: "test".to_string(),
        });
        assert!(
            strategy
                .best_hedges
                .values()
                .flatten()
                .all(|level| { level.symbol != "BTC-USD-SWAP.OK" })
        );

        strategy.on_system_event(&SystemEvent {
            ts_ms: 3,
            kind: SystemEventKind::FeedStale,
            venue: None,
            account_id: None,
            symbol: Some("BTC-USDT-SWAP.OK".to_string()),
            reason: "test".to_string(),
        });
        assert!(
            strategy
                .best_hedges
                .values()
                .flatten()
                .all(|level| { level.symbol != "BTC-USDT-SWAP.OK" })
        );
    }

    #[test]
    fn account_halt_disables_every_instrument_owned_by_the_account() {
        let mut config = config();
        config.risk_groups[0].account_id = Some("main".to_string());
        let mut strategy = ChaosStrategy::new(config).unwrap();

        let intents = strategy.on_system_event(&SystemEvent {
            ts_ms: 2,
            kind: SystemEventKind::AccountHalted,
            venue: None,
            account_id: Some("main".to_string()),
            symbol: None,
            reason: "operator".to_string(),
        });

        assert!(intents.is_empty());
        assert!(
            strategy
                .entities
                .values()
                .all(|entity| entity.system_halted)
        );
    }

    #[test]
    fn healthy_feed_heartbeat_does_not_recalculate_quotes() {
        let mut strategy = ChaosStrategy::new(java_calculator_config()).unwrap();
        seed_java_calculator_books(&mut strategy);
        assert!(!strategy.startup_basis_checked);

        let intents = strategy.on_system_event(&SystemEvent {
            ts_ms: 2,
            kind: SystemEventKind::FeedHeartbeat,
            venue: None,
            account_id: None,
            symbol: Some("BTC-USDT.OK".to_string()),
            reason: "accepted sequence".to_string(),
        });

        assert!(intents.is_empty());
        assert!(!strategy.startup_basis_checked);
        assert_eq!(strategy.now_ms, 2);
    }

    #[test]
    fn feed_recovery_waits_for_the_following_book_before_repricing() {
        let mut strategy = ChaosStrategy::new(java_calculator_config()).unwrap();
        seed_java_calculator_books(&mut strategy);
        let symbol = "BTC-USDT.OK";

        strategy.on_system_event(&SystemEvent {
            ts_ms: 2,
            kind: SystemEventKind::FeedStale,
            venue: None,
            account_id: None,
            symbol: Some(symbol.to_string()),
            reason: "gap".to_string(),
        });
        assert!(strategy.entities[symbol].feed_stale);

        let intents = strategy.on_system_event(&SystemEvent {
            ts_ms: 3,
            kind: SystemEventKind::FeedRecovered,
            venue: None,
            account_id: None,
            symbol: Some(symbol.to_string()),
            reason: "snapshot accepted".to_string(),
        });

        assert!(intents.is_empty());
        assert!(!strategy.entities[symbol].feed_stale);
        assert!(!strategy.startup_basis_checked);
    }

    #[test]
    fn java_parity_derivative_size_is_limited_by_margin_capacity() {
        let mut entity = InstrumentState::new(InstrumentConfig {
            symbol: "BTC-USDT-SWAP".to_string(),
            kind: InstrumentKindConfig::LinearSwap,
            contract_value: 0.01,
            max_order_size: 100.0,
            safety_multiplier: 1.0,
            min_position: -1_000.0,
            max_position: 1_000.0,
            ..InstrumentConfig::default()
        });
        entity.book = Some(OrderBook::one_level(
            "BTC-USDT-SWAP",
            1,
            Level::new(49_999.0, 1_000.0),
            Level::new(50_001.0, 1_000.0),
        ));
        entity.margin_initialized = true;
        entity.margin_balance = 30_000.0;
        entity.margin_coin_config = Some(CoinConfig {
            currency: "USDT".to_string(),
            ..CoinConfig::default()
        });

        assert!(approx_eq(entity.max_trade_size(Side::Buy, false), 20.0));
        entity.position_qty = -50.0;
        assert!(approx_eq(entity.max_trade_size(Side::Buy, false), 100.0));
    }

    #[test]
    fn java_parity_checks_exchange_margin_ratio_separately() {
        let mut cfg = config();
        cfg.margin_breach_debounce_ms = 100;
        let mut strategy = ChaosStrategy::new(cfg).unwrap();
        strategy.on_account_update(&AccountUpdate {
            ts_ms: 10,
            balances: Vec::new(),
            positions: Vec::new(),
            margins: vec![MarginSnapshot {
                account_id: None,
                ratio: None,
                exchange_ratio: Some(10.0),
                adjusted_equity_usd: None,
                notional_usd: None,
            }],
        });
        strategy.on_account_update(&AccountUpdate {
            ts_ms: 11,
            balances: Vec::new(),
            positions: Vec::new(),
            margins: vec![MarginSnapshot {
                account_id: None,
                ratio: None,
                exchange_ratio: Some(4.0),
                adjusted_equity_usd: None,
                notional_usd: None,
            }],
        });
        assert!(strategy.halt_reason().is_none());

        strategy.on_event(&StrategyEvent::Timer(reap_core::TimerEvent {
            ts_ms: 112,
            name: "risk".to_string(),
        }));

        assert!(
            strategy
                .halt_reason()
                .unwrap()
                .contains("exchange margin ratio")
        );
    }

    #[test]
    fn java_parity_stops_on_zombie_hedge_order() {
        let mut strategy = ChaosStrategy::new(config()).unwrap();
        strategy.entities.get_mut("BTC-USDT").unwrap().book = Some(OrderBook::one_level(
            "BTC-USDT",
            1,
            Level::new(99.0, 10.0),
            Level::new(101.0, 10.0),
        ));
        strategy.entities.get_mut("BTC-PERP").unwrap().book = Some(OrderBook::one_level(
            "BTC-PERP",
            1,
            Level::new(99.0, 10_000.0),
            Level::new(101.0, 10_000.0),
        ));
        strategy.on_order_update(&OrderUpdate {
            ts_ms: 1,
            order_id: "stuck-hedge".to_string(),
            symbol: "BTC-PERP".to_string(),
            side: Side::Sell,
            event: OrderEvent::New,
            status: OrderStatus::Live,
            price: 100.0,
            qty: 1.0,
            open_qty: 1.0,
            filled_qty: 0.0,
            avg_fill_price: 0.0,
            last_fill_qty: 0.0,
            last_fill_price: 0.0,
            last_fill_liquidity: None,
            reason: "hedge:BTC-USDT:100".to_string(),
        });

        strategy.on_event(&StrategyEvent::Timer(reap_core::TimerEvent {
            ts_ms: 30_001,
            name: "risk".to_string(),
        }));
        assert!(strategy.halt_reason().is_none());
        strategy.on_event(&StrategyEvent::Timer(reap_core::TimerEvent {
            ts_ms: 30_002,
            name: "risk".to_string(),
        }));

        assert!(strategy.halt_reason().unwrap().contains("stuck-hedge"));
    }

    #[test]
    fn risk_checks_fail_closed_on_non_finite_state() {
        let mut strategy = ChaosStrategy::new(config()).unwrap();
        strategy.risk_groups.get_mut("main").unwrap().delta_usd = f64::NAN;

        assert!(!strategy.check_risk_limits());
        assert!(strategy.halt_reason().unwrap().contains("non-finite"));
    }
}
