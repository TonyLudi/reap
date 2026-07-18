use std::collections::HashMap;
use std::hash::{BuildHasherDefault, DefaultHasher};

use crate::ChaosExecutionIntent;
use reap_core::{Side, Symbol, TimeMs};

const LIVE_ORDER_STOP_QUOTE_THRESHOLD: f64 = 0.6;
const EXTRA_MARGIN_BPS: f64 = 0.0002;
const CAN_TRADE_DEBOUNCE_MS: TimeMs = 500;
const CAN_QUOTE_FULL_SIZE_DEBOUNCE_MS: TimeMs = 10_000;
const ORDER_CHECK_DELTA_THRESHOLD_USD: f64 = 51.0;
const ZOMBIE_HEDGE_THRESHOLD_MS: TimeMs = 30_000;
const EXCHANGE_MARGIN_RATIO_THRESHOLD: f64 = 5.0;
const EPS: f64 = 1e-9;

type StableMap<K, V> = HashMap<K, V, BuildHasherDefault<DefaultHasher>>;

mod config;
mod events;
mod execution_state;
mod hedging;
mod instrument;
mod pricing;
mod reference_health;
mod risk;

use execution_state::ExecutionTrackingState;
use hedging::{HedgeCandidate, HedgingState};
use pricing::{JavaRandom, PricingState, round_passive_to_tick};
use reference_health::{
    DebouncedCondition, ReferenceHealthState, TimedPrice, should_accept_timestamp,
    timestamp_is_fresh,
};
use risk::AggregateRiskState;

pub use config::{
    ChaosConfig, CoinConfig, ConfigValidation, HaltIntervalConfig, InstrumentConfig,
    InstrumentKindConfig, ReferenceDataKind, ReferenceDataRequirement, RiskGroupConfig,
    RiskGroupKindConfig, SkewTypeConfig,
};
pub use execution_state::MissedHedge;
pub use hedging::HedgeLevel;
pub use instrument::InstrumentState;
pub use pricing::TheoQuote;
pub use risk::RiskGroupState;

#[derive(Debug, Clone)]
pub struct ChaosStrategy {
    config: ChaosConfig,
    entities: StableMap<Symbol, InstrumentState>,
    risk_groups: StableMap<String, RiskGroupState>,
    symbol_to_group: HashMap<Symbol, String>,
    reference_health: ReferenceHealthState,
    halt_reason: Option<String>,
    pricing: PricingState,
    hedging: HedgingState,
    execution: ExecutionTrackingState,
    now_ms: TimeMs,
    risk: AggregateRiskState,
}

impl ChaosStrategy {
    pub fn new(config: ChaosConfig) -> Result<Self, ConfigValidation> {
        let config = config.effective();
        let validation = config.validate();
        if !validation.valid {
            return Err(validation);
        }

        let mut risk_groups = StableMap::default();
        for rg in &config.risk_groups {
            risk_groups.insert(rg.name.clone(), RiskGroupState::new(rg.clone()));
        }
        if risk_groups.is_empty() {
            risk_groups.insert(
                "default".to_string(),
                RiskGroupState::new(RiskGroupConfig::default()),
            );
        }

        let mut entities = StableMap::default();
        let mut symbol_to_group = HashMap::new();
        let index_symbols = config
            .instruments
            .iter()
            .filter_map(|instrument| instrument.index_symbol.clone())
            .collect();
        for inst in &config.instruments {
            let mut state = InstrumentState::new(inst.clone());
            state.ignore_best_level = config.ignore_best_level;
            state.reference_data_stale_threshold_ms = config.reference_data_stale_threshold_ms;
            if let Some(group) = risk_groups.get(&inst.risk_group) {
                if group.config.kind == RiskGroupKindConfig::RefOnly {
                    state.config.halted = true;
                }
                state.trade.base_coin_config = group
                    .config
                    .coins
                    .iter()
                    .find(|coin| coin.currency == inst.base_currency)
                    .cloned();
                state.trade.quote_coin_config = group
                    .config
                    .coins
                    .iter()
                    .find(|coin| coin.currency == inst.quote_currency)
                    .cloned();
                state.trade.margin_coin_config = group
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
            rg.ordered_symbols.clear();
            rg.ordered_symbols.extend(rg.symbols.iter().cloned());
            rg.ordered_symbols.sort();
            rg.max_quote_size_usd = rg
                .ordered_symbols
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
            reference_health: ReferenceHealthState {
                index_symbols,
                index_prices: HashMap::new(),
                index_debouncers,
                basis_debouncers,
                basis_breaches: HashMap::new(),
                startup_basis_checked: false,
                insufficient_valid_since: None,
            },
            halt_reason: None,
            pricing: PricingState {
                burst: 0.0,
                burst_symbol: None,
                quote_targets: HashMap::new(),
                random: JavaRandom::new(1),
            },
            hedging: HedgingState {
                best_hedges,
                hedge_candidate_scratch: Vec::new(),
                last_hedge_ms: 0,
                no_hedge_found_since: None,
                hedge_not_found_since: None,
                all_hedges_halted_since: None,
            },
            execution: ExecutionTrackingState {
                active_quotes: StableMap::default(),
                active_hedges: StableMap::default(),
                last_quote_fill_ms: HashMap::new(),
                missed_hedges: Vec::new(),
            },
            now_ms: 0,
            risk: AggregateRiskState {
                delta_usd: 0.0,
                pending_delta_usd: 0.0,
                net_filled_delta_usd: 0.0,
                turnover_by_group: HashMap::new(),
                pnl_debouncer: DebouncedCondition::default(),
                margin_debouncers,
                exchange_margin_debouncers,
            },
        })
    }

    pub fn delta_usd(&self) -> f64 {
        self.risk.delta_usd
    }

    pub fn pending_delta_usd(&self) -> f64 {
        self.risk.pending_delta_usd
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
        &self.execution.missed_hedges
    }

    pub fn basis_breaches(&self) -> &HashMap<String, (Symbol, f64)> {
        &self.reference_health.basis_breaches
    }

    fn refresh_quotes(&mut self) -> Vec<ChaosExecutionIntent> {
        self.update_interval_halts();
        self.update_funding_window();
        self.update_risk();
        let validity_healthy = self.check_validity();
        let risk_healthy = validity_healthy && self.check_risk_limits();
        self.update_best_hedges();
        let hedge_healthy = self.check_hedge_availability();
        let startup_basis_healthy =
            if !self.reference_health.startup_basis_checked && self.pricing_ready() {
                self.reference_health.startup_basis_checked = true;
                if self.check_basis(true) {
                    true
                } else {
                    self.halt_reason = Some("startup basis limit breached".to_string());
                    false
                }
            } else {
                if self.reference_health.startup_basis_checked {
                    let _ = self.check_basis(false);
                }
                true
            };
        self.update_theo_quotes();
        let reference_healthy = self.configured_indexes_ready_at(self.now_ms);
        let pricing_healthy = risk_healthy
            && hedge_healthy
            && startup_basis_healthy
            && reference_healthy
            && self.check_index_deviation();

        let strat_can_buy = self.risk.pending_delta_usd <= 0.5 * self.config.delta_limit_usd
            && self.risk.delta_usd <= 0.5 * self.config.delta_limit_usd;
        let strat_can_sell = self.risk.pending_delta_usd >= -0.5 * self.config.delta_limit_usd
            && self.risk.delta_usd >= -0.5 * self.config.delta_limit_usd;

        let mut commands = Vec::new();
        // Java ChaosContext uses a TreeSet here so backtest command order is stable.
        let mut symbols = self.entities.keys().cloned().collect::<Vec<_>>();
        symbols.sort();
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

    fn ref_mid(&self) -> Option<f64> {
        self.entities
            .get(&self.config.ref_symbol)
            .and_then(InstrumentState::mid)
    }
}

fn approx_eq(a: f64, b: f64) -> bool {
    (a - b).abs() <= 1e-7_f64.max(a.abs().max(b.abs()) * 1e-9)
}

#[cfg(test)]
#[path = "../../tests/chaos_unit/mod.rs"]
mod tests;
