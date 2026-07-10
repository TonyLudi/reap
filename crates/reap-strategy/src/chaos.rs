use std::collections::{HashMap, HashSet};

use serde::{Deserialize, Serialize};

use crate::Strategy;
use reap_core::{
    MarketEvent, NewOrder, OrderBook, OrderEvent, OrderIntent, OrderUpdate, Price, Quantity, Side,
    StrategyEvent, Symbol, TimeInForce, TimeMs, round_down_to_lot, round_to_tick,
};

const HEDGE_VOL_TO_DELTA_RATIO: f64 = 1.5;
const LIVE_ORDER_STOP_QUOTE_THRESHOLD: f64 = 0.6;
const EPS: f64 = 1e-9;

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(default)]
pub struct ChaosConfig {
    pub strategy_name: String,
    pub underlying: String,
    pub ref_symbol: Symbol,
    pub delta_limit_usd: f64,
    pub active_hedge_threshold_usd: f64,
    pub min_hedge_interval_ms: TimeMs,
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
            delta_limit_usd: 50_000.0,
            active_hedge_threshold_usd: 2_000.0,
            min_hedge_interval_ms: 1_000,
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

impl ChaosConfig {
    pub fn validate(&self) -> ConfigValidation {
        let mut errors = Vec::new();
        if self.strategy_name.trim().is_empty() {
            errors.push("strategy_name must not be empty".to_string());
        }
        if self.instruments.is_empty() {
            errors.push("at least one instrument is required".to_string());
        }
        check_positive("delta_limit_usd", self.delta_limit_usd, &mut errors);
        check_non_negative(
            "active_hedge_threshold_usd",
            self.active_hedge_threshold_usd,
            &mut errors,
        );

        let mut symbols = HashSet::new();
        for instrument in &self.instruments {
            if instrument.symbol.trim().is_empty() {
                errors.push("instrument symbol must not be empty".to_string());
            } else if !symbols.insert(instrument.symbol.clone()) {
                errors.push(format!("duplicate instrument symbol {}", instrument.symbol));
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
            check_non_negative(
                &format!("{}.min_order_size_usd", instrument.symbol),
                instrument.min_order_size_usd,
                &mut errors,
            );
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
            ] {
                check_finite(
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
            if instrument.kind == InstrumentKindConfig::Future {
                check_positive(
                    &format!("{}.contract_value", instrument.symbol),
                    instrument.contract_value,
                    &mut errors,
                );
            }
        }
        if self.ref_symbol.trim().is_empty() {
            errors.push("ref_symbol must not be empty".to_string());
        } else if !symbols.contains(&self.ref_symbol) {
            errors.push(format!(
                "ref_symbol {} is not configured as an instrument",
                self.ref_symbol
            ));
        }

        let mut group_names = HashSet::new();
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
                }
            }
        }
        if !self.risk_groups.is_empty() {
            for instrument in &self.instruments {
                if !group_names.contains(&instrument.risk_group) {
                    errors.push(format!(
                        "instrument {} references unknown risk group {}",
                        instrument.symbol, instrument.risk_group
                    ));
                }
            }
        }

        ConfigValidation {
            valid: errors.is_empty(),
            errors,
        }
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
    pub symbols: Vec<Symbol>,
    pub coin_offset: f64,
    pub soft_delta_limit_usd: f64,
    pub hard_delta_limit_usd: f64,
    pub delta_stop_limit_usd: f64,
    pub live_order_limit_usd: f64,
    pub turnover_limit_usd: f64,
    pub basis_limit: f64,
}

impl Default for RiskGroupConfig {
    fn default() -> Self {
        Self {
            name: "default".to_string(),
            symbols: Vec::new(),
            coin_offset: 0.0,
            soft_delta_limit_usd: 25_000.0,
            hard_delta_limit_usd: 40_000.0,
            delta_stop_limit_usd: 60_000.0,
            live_order_limit_usd: 250_000.0,
            turnover_limit_usd: f64::MAX,
            basis_limit: 0.1,
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(default)]
pub struct InstrumentConfig {
    pub symbol: Symbol,
    pub kind: InstrumentKindConfig,
    pub risk_group: String,
    pub maker_fee: f64,
    pub taker_fee: f64,
    pub hedge_profit_margin: f64,
    pub quote_profit_margin: f64,
    pub hedge_aggression: f64,
    pub fv_offset: f64,
    pub max_order_size_usd: f64,
    pub min_order_size_usd: f64,
    pub max_order_size: f64,
    pub min_trade_size: f64,
    pub tick_size: f64,
    pub lot_size: f64,
    pub contract_value: f64,
    pub min_position: f64,
    pub max_position: f64,
    pub position_offset: f64,
    pub pos_skew: f64,
    pub neg_skew: f64,
    pub pos_extra_skew: f64,
    pub neg_extra_skew: f64,
    pub pos_activation: f64,
    pub neg_activation: f64,
    pub halted: bool,
}

impl Default for InstrumentConfig {
    fn default() -> Self {
        Self {
            symbol: String::new(),
            kind: InstrumentKindConfig::Spot,
            risk_group: "default".to_string(),
            maker_fee: 0.0,
            taker_fee: 0.0004,
            hedge_profit_margin: 0.0003,
            quote_profit_margin: 0.0003,
            hedge_aggression: 0.0002,
            fv_offset: 0.0,
            max_order_size_usd: 5_000.0,
            min_order_size_usd: 100.0,
            max_order_size: 1.0,
            min_trade_size: 0.0001,
            tick_size: 0.1,
            lot_size: 0.0001,
            contract_value: 1.0,
            min_position: -f64::MAX,
            max_position: f64::MAX,
            position_offset: 0.0,
            pos_skew: 0.0,
            neg_skew: 0.0,
            pos_extra_skew: 0.0,
            neg_extra_skew: 0.0,
            pos_activation: f64::MAX,
            neg_activation: -f64::MAX,
            halted: false,
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum InstrumentKindConfig {
    Spot,
    Future,
}

#[derive(Debug, Clone)]
pub struct ChaosStrategy {
    config: ChaosConfig,
    entities: HashMap<Symbol, InstrumentState>,
    risk_groups: HashMap<String, RiskGroupState>,
    symbol_to_group: HashMap<Symbol, String>,
    best_hedges: HashMap<Side, Vec<HedgeLevel>>,
    active_quotes: HashMap<(Symbol, Side), ActiveQuote>,
    now_ms: TimeMs,
    last_hedge_ms: TimeMs,
    delta_usd: f64,
    pending_delta_usd: f64,
}

impl ChaosStrategy {
    pub fn new(mut config: ChaosConfig) -> Self {
        if config.ref_symbol.is_empty() {
            config.ref_symbol = config
                .instruments
                .first()
                .map(|inst| inst.symbol.clone())
                .unwrap_or_default();
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
        for inst in &config.instruments {
            let state = InstrumentState::new(inst.clone());
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

        Self {
            config,
            entities,
            risk_groups,
            symbol_to_group,
            best_hedges,
            active_quotes: HashMap::new(),
            now_ms: 0,
            last_hedge_ms: 0,
            delta_usd: 0.0,
            pending_delta_usd: 0.0,
        }
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

    fn on_depth(&mut self, book: &OrderBook) -> Vec<OrderIntent> {
        self.now_ms = book.ts_ms;
        if let Some(entity) = self.entities.get_mut(&book.symbol) {
            entity.book = Some(book.clone());
        }
        self.refresh_quotes()
    }

    fn refresh_quotes(&mut self) -> Vec<OrderIntent> {
        self.update_risk();
        self.update_best_hedges();
        self.update_theo_quotes();

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
                let desired = if can_side && group_can_quote && entity.can_quote(side) {
                    entity.theo(side).filter(|quote| {
                        quote.price.is_finite() && quote.qty >= entity.config.min_trade_size
                    })
                } else {
                    None
                };
                self.sync_quote(&symbol, side, desired, &mut commands);
            }
        }
        commands
    }

    fn sync_quote(
        &mut self,
        symbol: &str,
        side: Side,
        desired: Option<TheoQuote>,
        commands: &mut Vec<OrderIntent>,
    ) {
        let key = (symbol.to_string(), side);
        let current = self.active_quotes.get(&key).cloned();
        let Some(desired) = desired else {
            if let Some(active) = current {
                commands.push(OrderIntent::CancelOrder {
                    order_id: active.order_id,
                    reason: "quote_disabled".to_string(),
                });
            }
            return;
        };

        if let Some(active) = current {
            if approx_eq(active.price, desired.price) && approx_eq(active.qty, desired.qty) {
                return;
            }
            commands.push(OrderIntent::CancelOrder {
                order_id: active.order_id,
                reason: "replace_quote".to_string(),
            });
        }

        commands.push(OrderIntent::NewOrder(NewOrder {
            symbol: symbol.to_string(),
            side,
            qty: desired.qty,
            price: desired.price,
            time_in_force: TimeInForce::PostOnly,
            reason: "quote".to_string(),
        }));
    }

    fn update_risk(&mut self) {
        let ref_mid = self.ref_mid().unwrap_or(1.0);
        let live_sizes: HashMap<String, f64> = self
            .risk_groups
            .iter()
            .map(|(name, rg)| {
                let size = self
                    .active_quotes
                    .iter()
                    .filter(|((symbol, _), _)| rg.symbols.contains(symbol))
                    .map(|((symbol, _), quote)| {
                        self.entities
                            .get(symbol)
                            .map(|entity| entity.notional_usd(quote.qty, ref_mid))
                            .unwrap_or(0.0)
                    })
                    .sum();
                (name.clone(), size)
            })
            .collect();

        for (name, rg) in self.risk_groups.iter_mut() {
            let mut delta_coin = rg.config.coin_offset;
            for symbol in &rg.symbols {
                if let Some(entity) = self.entities.get(symbol) {
                    delta_coin += entity.delta_coin();
                }
            }
            rg.delta_usd = delta_coin * ref_mid;
            rg.pending_delta_usd = rg.delta_usd;
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
        let Some(ref_mid) = self.ref_mid() else {
            return;
        };

        self.best_hedges.entry(Side::Buy).or_default().clear();
        self.best_hedges.entry(Side::Sell).or_default().clear();

        let group_names: Vec<_> = self.risk_groups.keys().cloned().collect();
        for group_name in group_names {
            for side in [Side::Buy, Side::Sell] {
                let mut levels = Vec::new();
                let symbols: Vec<_> = self
                    .risk_groups
                    .get(&group_name)
                    .map(|rg| rg.symbols.iter().cloned().collect())
                    .unwrap_or_default();
                for symbol in symbols {
                    let Some(entity) = self.entities.get(&symbol) else {
                        continue;
                    };
                    if !entity.can_take(side) {
                        continue;
                    }
                    levels.extend(entity.hedge_levels(side, ref_mid));
                }

                sort_hedge_levels(side, &mut levels);
                let selected = self.select_required_hedges(&group_name, side, levels);
                if let Some(rg) = self.risk_groups.get_mut(&group_name) {
                    rg.best_hedges.insert(side, selected);
                }
            }
        }

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

    fn select_required_hedges(
        &self,
        group_name: &str,
        _side: Side,
        levels: Vec<HedgeLevel>,
    ) -> Vec<HedgeLevel> {
        let Some(rg) = self.risk_groups.get(group_name) else {
            return levels;
        };
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
        let target = min_hedge_usd.max(delta_need);

        let mut selected = Vec::new();
        let mut total = 0.0;
        let mut per_symbol: HashMap<Symbol, f64> = HashMap::new();
        for level in levels {
            total += level.notional_usd;
            *per_symbol.entry(level.symbol.clone()).or_default() += level.notional_usd;
            selected.push(level);
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
    }

    fn update_theo_for_symbol(&mut self, symbol: &str, side: Side, hedges: &[HedgeLevel]) {
        let Some(ref_mid) = self.ref_mid() else {
            return;
        };
        let Some(pricing_entity) = self.entities.get(symbol).cloned() else {
            return;
        };
        if pricing_entity.config.quote_profit_margin >= 1.0 {
            if let Some(entity) = self.entities.get_mut(symbol) {
                entity.clear_theo(side);
            }
            return;
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
                price_by_hedge(side, &pricing_entity, hedge_entity, hedge_level, ref_mid);
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
            let raw_px = px + pos_skew_adj;
            let passive_px = side.passive_price(
                raw_px,
                pricing_entity.opposite_touch(side),
                pricing_entity.config.tick_size,
            );
            let quote_px = pricing_entity.px_within_limit(side, passive_px);
            let quote_qty = pricing_entity.quote_qty_from_usd(quote_size_usd, ref_mid);

            if let Some(entity) = self.entities.get_mut(symbol) {
                entity.set_theo(
                    side,
                    TheoQuote {
                        price: quote_px,
                        qty: quote_qty,
                        hedge_symbol: best_hedge_symbol,
                        hedge_px: single_weighted_px(weighted_hedge_px_by_symbol),
                    },
                );
            }
            return;
        }

        if let Some(entity) = self.entities.get_mut(symbol) {
            entity.clear_theo(side);
        }
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
                entity.position_qty - side.factor() * size
            } else {
                entity.position_qty + side.factor() * size
            };
            let adj_size = if entity.config.kind == InstrumentKindConfig::Future && size >= 2.0 {
                (size + 1.0) * 0.5
            } else {
                size * 0.5
            };
            total += -side.factor() * entity.skew_rate_at(end_pos) * adj_size * ref_mid;
        }
        total
    }

    fn maybe_hedge(&mut self, fill_symbol: &str, source_reason: &str) -> Vec<OrderIntent> {
        if source_reason.starts_with("hedge") {
            return Vec::new();
        }
        self.update_risk();
        if self.now_ms < self.last_hedge_ms + self.config.min_hedge_interval_ms {
            return Vec::new();
        }
        let delta_to_hedge = self.delta_to_hedge();
        if delta_to_hedge.abs() < self.config.active_hedge_threshold_usd {
            return Vec::new();
        }
        self.update_best_hedges();

        let hedge_side = if delta_to_hedge > 0.0 {
            Side::Sell
        } else {
            Side::Buy
        };
        let group_name = self.symbol_to_group.get(fill_symbol).cloned();
        let mut targets = Vec::new();
        if let Some(group_name) = group_name
            && let Some(rg) = self.risk_groups.get(&group_name)
            && rg.must_hedge_within_group(delta_to_hedge)
        {
            targets = self.summarize_hedges(
                rg.best_hedges_for(hedge_side),
                hedge_side,
                delta_to_hedge.abs(),
                Some(fill_symbol),
            );
        }
        if targets.is_empty() {
            let hedges = self
                .best_hedges
                .get(&hedge_side)
                .map(Vec::as_slice)
                .unwrap_or(&[]);
            targets =
                self.summarize_hedges(hedges, hedge_side, delta_to_hedge.abs(), Some(fill_symbol));
        }

        if targets.is_empty() {
            return Vec::new();
        }

        self.last_hedge_ms = self.now_ms;
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
                    reason: format!("hedge:{}", fill_symbol),
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
            let gap = usd_amt - total;
            let use_notional = level.notional_usd.min(gap);
            let qty = if level.notional_usd > gap {
                round_down_to_lot(level.qty * gap / level.notional_usd, entity.config.lot_size)
            } else {
                round_down_to_lot(level.qty, entity.config.lot_size)
            };
            if qty <= 0.0 {
                continue;
            }
            let notional = if level.qty > 0.0 {
                use_notional * qty / (level.qty * use_notional / level.notional_usd)
            } else {
                use_notional
            }
            .min(use_notional);
            let hedge_px = entity.hedge_px(hedge_side, level.px, entity.config.hedge_aggression);
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
            .or_else(|| self.entities.values().find_map(InstrumentState::mid))
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
        }
    }

    fn on_order_update(&mut self, update: &OrderUpdate) -> Vec<OrderIntent> {
        let key = (update.symbol.clone(), update.side);
        match update.event {
            OrderEvent::New if update.reason.starts_with("quote") => {
                self.active_quotes.insert(
                    key,
                    ActiveQuote {
                        order_id: update.order_id.clone(),
                        price: update.price,
                        qty: update.qty,
                    },
                );
            }
            OrderEvent::Cancelled | OrderEvent::FullyFilled | OrderEvent::Rejected => {
                self.active_quotes.remove(&key);
            }
            _ => {}
        }

        if update.has_fill() {
            if let Some(entity) = self.entities.get_mut(&update.symbol) {
                entity.apply_fill(update.side, update.last_fill_qty);
            }
            self.update_risk();
            return self.maybe_hedge(&update.symbol, &update.reason);
        }

        Vec::new()
    }
}

impl Strategy for ChaosStrategy {
    fn on_event(&mut self, event: &StrategyEvent) -> Vec<OrderIntent> {
        match event {
            StrategyEvent::Market(market) => self.on_market_event(market),
            StrategyEvent::Order(update) => self.on_order_update(update),
            StrategyEvent::Timer(timer) => {
                self.now_ms = timer.ts_ms;
                self.refresh_quotes()
            }
            StrategyEvent::Account(_) | StrategyEvent::Control(_) | StrategyEvent::System(_) => {
                Vec::new()
            }
        }
    }
}

#[derive(Debug, Clone)]
pub struct InstrumentState {
    pub config: InstrumentConfig,
    pub book: Option<OrderBook>,
    pub position_qty: Quantity,
    pub buy_theo: Option<TheoQuote>,
    pub sell_theo: Option<TheoQuote>,
}

impl InstrumentState {
    fn new(config: InstrumentConfig) -> Self {
        Self {
            config,
            book: None,
            position_qty: 0.0,
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

    fn can_quote(&self, side: Side) -> bool {
        !self.config.halted
            && self.config.quote_profit_margin < 1.0
            && self.book.is_some()
            && self.max_trade_size(side) >= self.config.min_trade_size
    }

    fn can_take(&self, side: Side) -> bool {
        !self.config.halted
            && self.config.hedge_profit_margin < 1.0
            && self.book.is_some()
            && self.max_trade_size(side) >= self.config.min_trade_size
    }

    fn max_trade_size(&self, side: Side) -> f64 {
        let buffer = self.config.max_order_size;
        match side {
            Side::Buy => (self.config.max_position - self.position_qty - buffer)
                .min(self.config.max_order_size),
            Side::Sell => (self.position_qty - self.config.min_position - buffer)
                .min(self.config.max_order_size),
        }
    }

    fn hedge_levels(&self, hedge_side: Side, ref_mid: f64) -> Vec<HedgeLevel> {
        let mut levels = Vec::new();
        let book_side = hedge_side.reverse();
        let Some(book) = &self.book else {
            return levels;
        };
        let skew_bps = self.fv_adjust();
        let adjust_bps =
            hedge_side.factor() * (self.config.taker_fee + self.config.hedge_profit_margin);
        let max_chunk = self.max_hedge_chunk_qty();
        let mut acc_qty = 0.0;

        for (level_idx, level) in book.levels(book_side).iter().enumerate() {
            if level.qty <= 0.0 || !level.px.is_finite() {
                continue;
            }
            let mut remaining = level.qty.min(self.max_trade_size(hedge_side).max(0.0));
            while remaining > 0.0 {
                let qty = round_down_to_lot(remaining.min(max_chunk), self.config.lot_size)
                    .max(self.config.min_trade_size)
                    .min(remaining);
                if qty <= 0.0 {
                    break;
                }
                acc_qty += qty;
                let end_pos = self.position_qty + hedge_side.factor() * acc_qty;
                let pos_skew_rate = self.skew_rate_at(end_pos);
                let hedge_rate = level.px / ref_mid - skew_bps
                    + adjust_bps
                    + hedge_side.factor() * acc_qty * pos_skew_rate * 0.5;
                let notional_usd = self.notional_usd(qty, ref_mid);
                levels.push(HedgeLevel {
                    symbol: self.config.symbol.clone(),
                    level: level_idx,
                    px: level.px,
                    qty,
                    hedge_rate,
                    notional_usd,
                    acc_qty,
                });
                remaining -= qty;
            }
        }
        levels
    }

    fn max_hedge_chunk_qty(&self) -> f64 {
        let skew = self.config.pos_skew.max(self.config.neg_skew).max(EPS);
        (self.config.hedge_profit_margin / skew)
            .max(self.config.min_trade_size)
            .min(self.config.max_order_size)
    }

    fn mid(&self) -> Option<f64> {
        self.book.as_ref()?.mid()
    }

    fn opposite_touch(&self, quote_side: Side) -> Option<f64> {
        let book = self.book.as_ref()?;
        match quote_side {
            Side::Buy => book.best_ask().map(|level| level.px),
            Side::Sell => book.best_bid().map(|level| level.px),
        }
    }

    fn px_within_limit(&self, side: Side, px: f64) -> f64 {
        let Some(book) = &self.book else {
            return round_to_tick(px, self.config.tick_size);
        };
        let rounded = round_to_tick(px, self.config.tick_size);
        match side {
            Side::Buy => book
                .best_ask()
                .map(|ask| rounded.min(ask.px - self.config.tick_size))
                .unwrap_or(rounded),
            Side::Sell => book
                .best_bid()
                .map(|bid| rounded.max(bid.px + self.config.tick_size))
                .unwrap_or(rounded),
        }
    }

    fn hedge_px(&self, hedge_side: Side, px: f64, agg_factor: f64) -> f64 {
        let side_to_take = hedge_side.reverse();
        let ag_mult = 1.0 - side_to_take.factor() * agg_factor;
        round_to_tick(px * ag_mult, self.config.tick_size)
    }

    fn quote_qty_from_usd(&self, usd: f64, ref_mid: f64) -> f64 {
        round_down_to_lot(self.size_from_usd(usd, ref_mid), self.config.lot_size)
    }

    fn size_from_usd(&self, usd: f64, ref_mid: f64) -> f64 {
        match self.config.kind {
            InstrumentKindConfig::Spot => self.mid().map(|mid| usd / mid).unwrap_or(0.0),
            InstrumentKindConfig::Future => usd / (ref_mid * self.config.contract_value.max(EPS)),
        }
    }

    fn notional_usd(&self, qty: f64, ref_mid: f64) -> f64 {
        self.notional_coin(qty) * ref_mid
    }

    fn notional_coin(&self, qty: f64) -> f64 {
        match self.config.kind {
            InstrumentKindConfig::Spot => qty,
            InstrumentKindConfig::Future => qty * self.config.contract_value,
        }
    }

    fn delta_coin(&self) -> f64 {
        match self.config.kind {
            InstrumentKindConfig::Spot => self.position_qty,
            InstrumentKindConfig::Future => self.position_qty * self.config.contract_value,
        }
    }

    fn apply_fill(&mut self, side: Side, qty: f64) {
        self.position_qty += side.factor() * qty;
    }

    fn fv_adjust(&self) -> f64 {
        self.posn_skew() + self.config.fv_offset
    }

    fn posn_skew(&self) -> f64 {
        let pos = self.position_qty;
        let offset = self.config.position_offset;
        let rate = self.skew_rate_at(pos);
        -(pos - offset) * rate
    }

    fn skew_rate_at(&self, target_pos: f64) -> f64 {
        let shifted = target_pos - self.config.position_offset;
        if shifted > 0.0 {
            if target_pos <= self.config.pos_activation {
                self.config.pos_skew
            } else {
                self.config.pos_skew + self.config.pos_extra_skew
            }
        } else if shifted < 0.0 {
            if target_pos >= self.config.neg_activation {
                self.config.neg_skew
            } else {
                self.config.neg_skew + self.config.neg_extra_skew
            }
        } else {
            self.config.pos_skew.max(self.config.neg_skew)
        }
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
    pub best_hedges: HashMap<Side, Vec<HedgeLevel>>,
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
            best_hedges,
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
    pub level: usize,
    pub px: Price,
    pub qty: Quantity,
    pub hedge_rate: f64,
    pub notional_usd: f64,
    pub acc_qty: Quantity,
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

#[derive(Debug, Clone)]
struct ActiveQuote {
    order_id: String,
    price: Price,
    qty: Quantity,
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
            + hedge.config.hedge_profit_margin
            + pricing.config.quote_profit_margin
            + pricing.config.maker_fee);
    hedge_px_at_spot + (pricing.fv_adjust() - adjust_bps) * ref_mid
}

fn sort_hedge_levels(side: Side, levels: &mut [HedgeLevel]) {
    match side {
        Side::Buy => levels.sort_by(|a, b| a.hedge_rate.total_cmp(&b.hedge_rate)),
        Side::Sell => levels.sort_by(|a, b| b.hedge_rate.total_cmp(&a.hedge_rate)),
    }
}

fn all_symbols_have_hedge(
    per_symbol: &HashMap<Symbol, f64>,
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

#[cfg(test)]
mod tests {
    use super::*;
    use reap_core::{Level, MarketEvent, NormalizedEvent, OrderBook, OrderStatus, StrategyEvent};

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

    #[test]
    fn computes_quotes_from_opposite_hedge_ladder() {
        let mut strategy = ChaosStrategy::new(config());
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
        let mut strategy = ChaosStrategy::new(config());
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

        let hedge = strategy.on_event(&StrategyEvent::Order(OrderUpdate {
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

        assert!(hedge.iter().any(|cmd| matches!(cmd, OrderIntent::NewOrder(o)
            if o.symbol == "BTC-PERP" && o.side == Side::Sell && o.time_in_force == TimeInForce::Ioc)));
    }

    #[test]
    fn normalized_fixture_drives_quote_then_hedge_decisions() {
        let events = include_str!("../../../fixtures/normalized/chaos_quote_hedge.jsonl")
            .lines()
            .filter(|line| !line.trim().is_empty())
            .map(|line| serde_json::from_str::<NormalizedEvent>(line).unwrap())
            .collect::<Vec<_>>();
        let mut strategy = ChaosStrategy::new(config());

        let mut all_intents = Vec::new();
        for event in events {
            let intents = strategy.on_event(&event.into_strategy_event());
            all_intents.push(intents);
        }

        assert!(all_intents[1].iter().any(
            |intent| matches!(intent, OrderIntent::NewOrder(order) if order.reason == "quote")
        ));
        assert!(all_intents[2].iter().any(|intent| matches!(intent, OrderIntent::NewOrder(order)
            if order.symbol == "BTC-PERP" && order.side == Side::Sell && order.time_in_force == TimeInForce::Ioc)));
    }

    #[test]
    fn config_validation_catches_duplicate_symbols_and_invalid_ticks() {
        let valid = config();
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
}
