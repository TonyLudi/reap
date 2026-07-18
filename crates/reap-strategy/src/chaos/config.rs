use std::collections::{BTreeSet, HashMap, HashSet};

use reap_core::{Symbol, TimeMs};
use serde::{Deserialize, Serialize};

use super::{EPS, LIVE_ORDER_STOP_QUOTE_THRESHOLD};

#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub enum ReferenceDataKind {
    IndexPrice,
    FundingRate,
    MarkPrice,
    PriceLimits,
}

impl ReferenceDataKind {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::IndexPrice => "index_price",
            Self::FundingRate => "funding_rate",
            Self::MarkPrice => "mark_price",
            Self::PriceLimits => "price_limits",
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct ReferenceDataRequirement {
    pub kind: ReferenceDataKind,
    pub symbol: Symbol,
    pub max_age_ms: TimeMs,
}

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
    /// Enables fail-closed freshness checks for every derived pricing input.
    /// Live configurations must set this explicitly; legacy backtests may omit it.
    pub reference_data_stale_threshold_ms: Option<TimeMs>,
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
            reference_data_stale_threshold_ms: None,
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
    pub fn reference_data_requirements(&self) -> Vec<ReferenceDataRequirement> {
        let Some(max_age_ms) = self.reference_data_stale_threshold_ms else {
            return Vec::new();
        };
        let mut requirements = BTreeSet::new();
        for instrument in &self.instruments {
            requirements.insert((ReferenceDataKind::PriceLimits, instrument.symbol.clone()));
            if instrument.kind.is_derivative() {
                requirements.insert((ReferenceDataKind::MarkPrice, instrument.symbol.clone()));
            }
            if instrument.kind.is_swap() {
                requirements.insert((ReferenceDataKind::FundingRate, instrument.symbol.clone()));
            }
            if let Some(index_symbol) = &instrument.index_symbol {
                requirements.insert((ReferenceDataKind::IndexPrice, index_symbol.clone()));
            }
        }
        requirements
            .into_iter()
            .map(|(kind, symbol)| ReferenceDataRequirement {
                kind,
                symbol,
                max_age_ms,
            })
            .collect()
    }

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
        self.validate_top_level_fields(&mut errors);
        let symbols = self.validate_instruments(&mut errors);
        self.validate_reference_instrument(&symbols, &mut errors);
        let (group_names, grouped_symbols) = self.validate_risk_groups(&symbols, &mut errors);
        self.validate_instrument_membership_and_hedges(&group_names, &grouped_symbols, &mut errors);
        self.validate_coin_offset(&mut errors);

        ConfigValidation {
            valid: errors.is_empty(),
            errors,
        }
    }

    fn validate_top_level_fields(&self, errors: &mut Vec<String>) {
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
        check_finite("coin_offset", self.coin_offset, errors);
        check_positive("risk_multiplier", self.risk_multiplier, errors);
        check_positive(
            "balance_sheet_limit_usd",
            self.balance_sheet_limit_usd,
            errors,
        );
        check_positive("delta_limit_usd", self.delta_limit_usd, errors);
        check_positive("pnl_limit_usd", self.pnl_limit_usd, errors);
        check_non_negative(
            "active_hedge_threshold_usd",
            self.active_hedge_threshold_usd,
            errors,
        );
        check_non_negative("index_deviation_limit", self.index_deviation_limit, errors);
        if self.reference_data_stale_threshold_ms == Some(0) {
            errors.push(
                "reference_data_stale_threshold_ms must be positive when configured".to_string(),
            );
        }
    }

    fn validate_instruments(&self, errors: &mut Vec<String>) -> HashSet<Symbol> {
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
                errors,
            );
            check_positive(
                &format!("{}.lot_size", instrument.symbol),
                instrument.lot_size,
                errors,
            );
            check_positive(
                &format!("{}.max_order_size", instrument.symbol),
                instrument.max_order_size,
                errors,
            );
            check_positive(
                &format!("{}.max_order_size_usd", instrument.symbol),
                instrument.max_order_size_usd,
                errors,
            );
            check_positive(
                &format!("{}.safety_multiplier", instrument.symbol),
                instrument.safety_multiplier,
                errors,
            );
            check_non_negative(
                &format!("{}.min_order_size_usd", instrument.symbol),
                instrument.min_order_size_usd,
                errors,
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
                errors,
            );
            check_non_negative(
                &format!("{}.max_level_spread", instrument.symbol),
                instrument.max_level_spread,
                errors,
            );
            check_non_negative(
                &format!("{}.debounce_width", instrument.symbol),
                instrument.debounce_width,
                errors,
            );
            check_non_negative(
                &format!("{}.debounce_size_usd", instrument.symbol),
                instrument.debounce_size_usd,
                errors,
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
                errors,
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
                check_finite(&format!("{}.{field}", instrument.symbol), value, errors);
            }
            if instrument.taker_fee < instrument.maker_fee {
                errors.push(format!(
                    "{}.taker_fee must not be lower than maker_fee",
                    instrument.symbol
                ));
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
                check_positive(&format!("{}.{field}", instrument.symbol), value, errors);
            }
            for (field, value) in [
                ("hedge_aggression", instrument.hedge_aggression),
                ("pos_skew", instrument.pos_skew),
                ("neg_skew", instrument.neg_skew),
                ("pos_extra_skew", instrument.pos_extra_skew),
                ("neg_extra_skew", instrument.neg_extra_skew),
            ] {
                check_non_negative(&format!("{}.{field}", instrument.symbol), value, errors);
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
                    errors,
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
        symbols
    }

    fn validate_reference_instrument(&self, symbols: &HashSet<Symbol>, errors: &mut Vec<String>) {
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
    }

    fn validate_risk_groups(
        &self,
        symbols: &HashSet<Symbol>,
        errors: &mut Vec<String>,
    ) -> (HashSet<String>, HashSet<Symbol>) {
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
                errors,
            );
            check_positive(
                &format!("{}.hard_delta_limit_usd", group.name),
                group.hard_delta_limit_usd,
                errors,
            );
            check_positive(
                &format!("{}.delta_stop_limit_usd", group.name),
                group.delta_stop_limit_usd,
                errors,
            );
            check_positive(
                &format!("{}.live_order_limit_usd", group.name),
                group.live_order_limit_usd,
                errors,
            );
            check_positive(
                &format!("{}.turnover_limit_usd", group.name),
                group.turnover_limit_usd,
                errors,
            );
            check_non_negative(
                &format!("{}.min_margin_level", group.name),
                group.min_margin_level,
                errors,
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
                errors,
            );
            check_finite(
                &format!("{}.coin_offset", group.name),
                group.coin_offset,
                errors,
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
                    errors,
                );
                check_non_negative(
                    &format!("{}.{}.borrow_limit_coin", group.name, coin.currency),
                    coin.borrow_limit_coin,
                    errors,
                );
                check_positive(
                    &format!("{}.{}.safety_multiplier", group.name, coin.currency),
                    coin.safety_multiplier,
                    errors,
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
                        errors,
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
        (group_names, grouped_symbols)
    }

    fn validate_instrument_membership_and_hedges(
        &self,
        group_names: &HashSet<String>,
        grouped_symbols: &HashSet<Symbol>,
        errors: &mut Vec<String>,
    ) {
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
    }

    fn validate_coin_offset(&self, errors: &mut Vec<String>) {
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
    pub(super) fn borrow_limit(&self, usd_rate: f64) -> f64 {
        (self.borrow_limit_usd / usd_rate.max(EPS)).min(self.borrow_limit_coin)
    }

    pub(super) fn skew_rate_at(&self, balance: f64) -> f64 {
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

    pub(super) fn skew_zone(&self, balance: f64) -> i8 {
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

    pub(super) fn integrated_skew(&self, balance: f64) -> f64 {
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

    pub(super) fn average_skew_rate(&self, current: f64, target: f64) -> f64 {
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
