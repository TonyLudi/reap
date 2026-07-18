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
mod tests {
    use std::sync::Arc;

    use super::*;
    use crate::Strategy;
    use reap_core::{
        AccountUpdate, Balance, FillLiquidity, Level, MarginSnapshot, MarketEvent, NormalizedEvent,
        OrderBook, OrderEvent, OrderIntent, OrderStatus, OrderUpdate, SelfTradePrevention,
        StrategyEvent, SystemEvent, SystemEventKind, TimeInForce,
    };

    fn legacy_intents(intents: Vec<ChaosExecutionIntent>) -> Vec<OrderIntent> {
        intents
            .into_iter()
            .map(ChaosExecutionIntent::into_order_intent)
            .collect()
    }

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

    fn seed_strict_reference_data(strategy: &mut ChaosStrategy, ts_ms: TimeMs) {
        let events = [
            MarketEvent::IndexPrice {
                ts_ms,
                symbol: "BTC-USDT-INDEX".to_string(),
                price: 50_000.5,
            },
            MarketEvent::PriceLimits {
                ts_ms,
                symbol: "BTC-USDT".to_string(),
                mark_price: 0.0,
                limit_down: 40_000.0,
                limit_up: 60_000.0,
            },
            MarketEvent::PriceLimits {
                ts_ms,
                symbol: "BTC-PERP".to_string(),
                mark_price: 0.0,
                limit_down: 40_000.0,
                limit_up: 60_000.0,
            },
            MarketEvent::PriceLimits {
                ts_ms,
                symbol: "BTC-PERP".to_string(),
                mark_price: 50_003.5,
                limit_down: 0.0,
                limit_up: 0.0,
            },
            MarketEvent::FundingRate {
                ts_ms,
                symbol: "BTC-PERP".to_string(),
                rate: 0.0001,
                funding_time_ms: ts_ms + 28_800_000,
                settlement: None,
            },
        ];
        for event in events {
            strategy.on_event(&StrategyEvent::Market(event));
        }
    }

    #[test]
    fn strict_reference_contract_is_derived_once_for_strategy_and_live() {
        let mut config = config();
        config.reference_data_stale_threshold_ms = Some(1_000);
        config.instruments[0].index_symbol = Some("BTC-USDT-INDEX".to_string());
        config.instruments[1].kind = InstrumentKindConfig::LinearSwap;

        let requirements = config.reference_data_requirements();
        assert_eq!(requirements.len(), 5);
        assert!(requirements.iter().all(|item| item.max_age_ms == 1_000));
        assert!(requirements.iter().any(|item| {
            item.kind == ReferenceDataKind::IndexPrice && item.symbol == "BTC-USDT-INDEX"
        }));
        assert!(requirements.iter().any(|item| {
            item.kind == ReferenceDataKind::FundingRate && item.symbol == "BTC-PERP"
        }));
        assert_eq!(
            requirements
                .iter()
                .filter(|item| item.kind == ReferenceDataKind::PriceLimits)
                .count(),
            2
        );
    }

    #[test]
    fn strict_reference_staleness_withdraws_quotes_without_cross_channel_masking() {
        let mut config = config();
        config.reference_data_stale_threshold_ms = Some(1_000);
        config.instruments[0].index_symbol = Some("BTC-USDT-INDEX".to_string());
        config.instruments[1].kind = InstrumentKindConfig::LinearSwap;
        let mut strategy = ChaosStrategy::new(config).unwrap();
        seed_strict_reference_data(&mut strategy, 1_000);

        strategy.on_event(&StrategyEvent::Market(MarketEvent::Depth(
            OrderBook::one_level(
                "BTC-USDT",
                1_000,
                Level::new(50_000.0, 1.0),
                Level::new(50_001.0, 1.0),
            ),
        )));
        let quotes = strategy.on_event(&StrategyEvent::Market(MarketEvent::Depth(
            OrderBook::one_level(
                "BTC-PERP",
                1_000,
                Level::new(50_003.0, 200.0),
                Level::new(50_004.0, 200.0),
            ),
        )));
        let quote = quotes
            .iter()
            .find_map(|intent| match intent {
                OrderIntent::NewOrder(order) if order.reason == "quote" => Some(order.clone()),
                _ => None,
            })
            .expect("fresh strict references should permit quoting");
        strategy.on_event(&StrategyEvent::Order(OrderUpdate {
            ts_ms: 1_001,
            order_id: "strict-q1".to_string(),
            symbol: quote.symbol,
            side: quote.side,
            event: OrderEvent::PendingNew,
            status: OrderStatus::PendingNew,
            price: quote.price,
            time_in_force: Some(quote.time_in_force),
            qty: quote.qty,
            open_qty: quote.qty,
            filled_qty: 0.0,
            avg_fill_price: 0.0,
            last_fill_qty: 0.0,
            last_fill_price: 0.0,
            last_fill_liquidity: None,
            last_fill_fee: None,
            reason: quote.reason,
        }));

        let intents = strategy.on_event(&StrategyEvent::Market(MarketEvent::PriceLimits {
            ts_ms: 2_001,
            symbol: "BTC-PERP".to_string(),
            mark_price: 50_003.5,
            limit_down: 0.0,
            limit_up: 0.0,
        }));

        let swap = strategy.entity("BTC-PERP").unwrap();
        assert_eq!(swap.mark_price_updated_ms, Some(2_001));
        assert_eq!(swap.price_limits_updated_ms, Some(1_000));
        assert_eq!(swap.funding_rate_updated_ms, Some(1_000));
        assert!(intents.iter().any(|intent| {
            matches!(intent, OrderIntent::CancelOrder { order_id, .. } if order_id == "strict-q1")
        }));

        strategy.on_event(&StrategyEvent::Market(MarketEvent::PriceLimits {
            ts_ms: 1_500,
            symbol: "BTC-PERP".to_string(),
            mark_price: 49_000.0,
            limit_down: 0.0,
            limit_up: 0.0,
        }));
        let swap = strategy.entity("BTC-PERP").unwrap();
        assert_eq!(swap.mark_price, Some(50_003.5));
        assert_eq!(swap.mark_price_updated_ms, Some(2_001));
        assert_eq!(strategy.now_ms, 2_001);
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
            time_in_force: Some(TimeInForce::PostOnly),
            qty: 0.1,
            open_qty: 0.0,
            filled_qty: 0.1,
            avg_fill_price: 50_000.0,
            last_fill_qty: 0.1,
            last_fill_price: 50_000.0,
            last_fill_liquidity: None,
            last_fill_fee: None,
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
                margin_mode: None,
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
        let events = include_str!("../../../../fixtures/normalized/chaos_quote_hedge.jsonl")
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

        let expected: serde_json::Value = serde_json::from_str(include_str!(
            "../../../../fixtures/normalized/chaos_quote_hedge_intents.json"
        ))
        .unwrap();
        assert_eq!(serde_json::to_value(&all_intents).unwrap(), expected);

        assert!(all_intents[1].iter().any(
            |intent| matches!(intent, OrderIntent::NewOrder(order) if order.reason == "quote")
        ));
        assert!(all_intents[2].is_empty());
        assert!(all_intents[3].iter().any(|intent| matches!(intent, OrderIntent::NewOrder(order)
            if order.symbol == "BTC-PERP" && order.side == Side::Sell && order.time_in_force == TimeInForce::Ioc)));
    }

    #[test]
    fn normalized_fixture_typed_output_preserves_exact_ordered_intents() {
        let events = include_str!("../../../../fixtures/normalized/chaos_quote_hedge.jsonl")
            .lines()
            .filter(|line| !line.trim().is_empty())
            .map(|line| serde_json::from_str::<NormalizedEvent>(line).unwrap())
            .collect::<Vec<_>>();
        let mut strategy = ChaosStrategy::new(config()).unwrap();

        let mut typed_by_event = Vec::new();
        for event in events {
            typed_by_event.push(strategy.on_execution_event(&event.into_strategy_event()));
        }
        let purposes = typed_by_event
            .iter()
            .flatten()
            .map(ChaosExecutionIntent::purpose)
            .collect::<Vec<_>>();
        assert!(purposes.contains(&crate::ChaosExecutionPurpose::Quote));
        assert!(purposes.contains(&crate::ChaosExecutionPurpose::Hedge));

        let lowered = typed_by_event
            .into_iter()
            .map(legacy_intents)
            .collect::<Vec<_>>();
        let expected: serde_json::Value = serde_json::from_str(include_str!(
            "../../../../fixtures/normalized/chaos_quote_hedge_intents.json"
        ))
        .unwrap();
        assert_eq!(serde_json::to_value(lowered).unwrap(), expected);
    }

    #[test]
    fn quote_replacement_emits_typed_cancel_owned_without_changing_legacy_record() {
        let mut strategy = ChaosStrategy::new(config()).unwrap();
        strategy.execution.active_quotes.insert(
            ("BTC-USDT".to_string(), Side::Buy, 0),
            execution_state::ActiveQuote {
                order_id: "canonical-q1".to_string(),
                price: 49_900.0,
                qty: 0.1,
            },
        );
        let mut intents = Vec::new();
        strategy.sync_quotes("BTC-USDT", Side::Buy, &[], &mut intents);

        let [ChaosExecutionIntent::CancelOwned(cancel)] = intents.as_slice() else {
            panic!("expected one typed owned-order cancellation");
        };
        assert_eq!(cancel.order_id(), "canonical-q1");
        assert_eq!(cancel.reason(), "quote_disabled");
        assert_eq!(
            serde_json::to_value(intents.remove(0).into_order_intent()).unwrap(),
            serde_json::json!({
                "CancelOrder": {
                    "order_id": "canonical-q1",
                    "reason": "quote_disabled"
                }
            })
        );
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
    fn java_parity_rejects_taker_fee_below_maker_fee() {
        let mut invalid = config();
        invalid.instruments[0].maker_fee = 0.001;
        invalid.instruments[0].taker_fee = 0.0005;

        let report = invalid.validate();

        assert!(!report.valid);
        assert!(
            report
                .errors
                .iter()
                .any(|error| error.contains("taker_fee must not be lower than maker_fee"))
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
            strategy.hedging.best_hedges.get(&Side::Sell).unwrap(),
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
            strategy.hedging.best_hedges.get(&Side::Sell).unwrap(),
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
            strategy.hedging.best_hedges.get(&Side::Sell).unwrap(),
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
        let intents = legacy_intents(intents);
        assert_eq!(intents.len(), 3);
        assert!(matches!(&intents[0], OrderIntent::NewOrder(order) if order.reason == "quote"));
        assert!(matches!(&intents[1], OrderIntent::NewOrder(order) if order.reason == "quote:1"));
        assert!(matches!(&intents[2], OrderIntent::NewOrder(order) if order.reason == "quote:2"));
    }

    #[test]
    fn pending_new_quote_blocks_duplicate_intent_before_exchange_ack() {
        let mut strategy = ChaosStrategy::new(config()).unwrap();
        let levels = vec![TheoQuote {
            price: 49_900.0,
            qty: 0.1,
            hedge_px: 50_001.0,
            hedge_symbol: "BTC-PERP".to_string(),
        }];
        let mut first = Vec::new();
        strategy.sync_quotes("BTC-USDT", Side::Buy, &levels, &mut first);
        let first = legacy_intents(first);
        let OrderIntent::NewOrder(order) = &first[0] else {
            panic!("expected quote order");
        };

        strategy.on_order_update(&OrderUpdate {
            ts_ms: 2,
            order_id: "pending-q1".to_string(),
            symbol: order.symbol.clone(),
            side: order.side,
            event: OrderEvent::PendingNew,
            status: OrderStatus::PendingNew,
            price: order.price,
            time_in_force: Some(order.time_in_force),
            qty: order.qty,
            open_qty: order.qty,
            filled_qty: 0.0,
            avg_fill_price: 0.0,
            last_fill_qty: 0.0,
            last_fill_price: 0.0,
            last_fill_liquidity: None,
            last_fill_fee: None,
            reason: "quote:pending_new".to_string(),
        });

        let mut repeated = Vec::new();
        strategy.sync_quotes("BTC-USDT", Side::Buy, &levels, &mut repeated);

        assert!(repeated.is_empty());
    }

    #[test]
    fn java_parity_pending_delta_includes_pending_and_live_hedges() {
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
            event: OrderEvent::PendingNew,
            status: OrderStatus::PendingNew,
            price: 49_990.0,
            time_in_force: Some(TimeInForce::Ioc),
            qty: 100.0,
            open_qty: 100.0,
            filled_qty: 0.0,
            avg_fill_price: 0.0,
            last_fill_qty: 0.0,
            last_fill_price: 0.0,
            last_fill_liquidity: None,
            last_fill_fee: None,
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
        strategy.execution.active_hedges.insert(
            "h1".to_string(),
            execution_state::ActiveHedge {
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
        let intents = legacy_intents(strategy.on_market_event(&MarketEvent::IndexPrice {
            ts_ms: 111,
            symbol: "BTC-INDEX".to_string(),
            price: 80.0,
        }));

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
        assert!(
            !strategy
                .reference_health
                .index_prices
                .contains_key("USDT-USD")
        );
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
            strategy.hedging.best_hedges.get(&Side::Buy).unwrap(),
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
        assert!(approx_eq(strategy.pricing.burst, 0.0));
        assert!(strategy.pricing.burst_symbol.is_none());
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
        let halted = legacy_intents(strategy.refresh_quotes());
        assert!(strategy.entity("BTC-PERP").unwrap().interval_halted);
        assert!(
            halted
                .iter()
                .all(|intent| !matches!(intent, OrderIntent::NewOrder(_)))
        );

        strategy.now_ms = 21_000;
        let resumed = legacy_intents(strategy.refresh_quotes());
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
                    forced_repayment_indicator: None,
                },
                Balance {
                    account_id: None,
                    currency: "USDT".to_string(),
                    total: 1_000_000.0,
                    available: 1_000_000.0,
                    equity: 1_000_000.0,
                    liability: 0.0,
                    max_loan: 0.0,
                    forced_repayment_indicator: None,
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

        let intents = legacy_intents(strategy.refresh_quotes());

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
            time_in_force: Some(TimeInForce::PostOnly),
            qty: 1.0,
            open_qty: 0.0,
            filled_qty: 1.0,
            avg_fill_price: 100.0,
            last_fill_qty: 1.0,
            last_fill_price: 100.0,
            last_fill_liquidity: None,
            last_fill_fee: None,
            reason: "quote".to_string(),
        });

        let intents = legacy_intents(strategy.refresh_quotes());

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
    fn zero_notional_account_does_not_create_infinite_margin_breach() {
        let mut strategy = ChaosStrategy::new(config()).unwrap();
        strategy.on_account_update(&AccountUpdate {
            ts_ms: 10,
            balances: Vec::new(),
            positions: Vec::new(),
            margins: vec![MarginSnapshot {
                account_id: None,
                ratio: None,
                exchange_ratio: None,
                adjusted_equity_usd: Some(10_000.0),
                notional_usd: Some(0.0),
            }],
        });
        strategy.entities.get_mut("BTC-USDT").unwrap().book = Some(OrderBook::one_level(
            "BTC-USDT",
            10,
            Level::new(99.0, 10.0),
            Level::new(101.0, 10.0),
        ));
        strategy.entities.get_mut("BTC-PERP").unwrap().book = Some(OrderBook::one_level(
            "BTC-PERP",
            10,
            Level::new(99.0, 10_000.0),
            Level::new(101.0, 10_000.0),
        ));

        strategy.refresh_quotes();

        assert!(strategy.halt_reason().is_none());
        assert!(strategy.risk_groups["main"].margin_ratio.is_none());
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
            time_in_force: Some(TimeInForce::PostOnly),
            qty: 1.0,
            open_qty: 1.0,
            filled_qty: 0.0,
            avg_fill_price: 0.0,
            last_fill_qty: 0.0,
            last_fill_price: 0.0,
            last_fill_liquidity: None,
            last_fill_fee: None,
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
            time_in_force: Some(TimeInForce::PostOnly),
            qty: 1.0,
            open_qty: 0.0,
            filled_qty: 1.0,
            avg_fill_price: 100.0,
            last_fill_qty: 1.0,
            last_fill_price: 100.0,
            last_fill_liquidity: Some(FillLiquidity::Maker),
            last_fill_fee: None,
            reason: "quote".to_string(),
        });

        strategy.now_ms = 399;
        let mut blocked = Vec::new();
        strategy.sync_quotes("BTC-USDT", Side::Buy, &levels, &mut blocked);
        assert!(blocked.is_empty());

        strategy.now_ms = 400;
        let mut refill = Vec::new();
        strategy.sync_quotes("BTC-USDT", Side::Buy, &levels, &mut refill);
        let refill = legacy_intents(refill);
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
        strategy.hedging.last_hedge_ms = 10;

        let intents = legacy_intents(strategy.on_account_update(&AccountUpdate {
            ts_ms: 20,
            balances: Vec::new(),
            positions: vec![reap_core::Position {
                symbol: "BTC-USDT".to_string(),
                qty: 0.1,
                avg_price: 50_000.0,
                margin_mode: None,
            }],
            margins: Vec::new(),
        }));

        assert!(intents.iter().any(|intent| matches!(intent, OrderIntent::NewOrder(order)
            if order.symbol == "BTC-PERP" && order.side == Side::Sell && order.time_in_force == TimeInForce::Ioc)));
        assert_eq!(strategy.hedging.last_hedge_ms, 10);
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
                time_in_force: Some(TimeInForce::PostOnly),
                qty: 0.01,
                open_qty: 0.0,
                filled_qty: 0.01,
                avg_fill_price: 98.0,
                last_fill_qty: 0.01,
                last_fill_price: 98.0,
                last_fill_liquidity: Some(FillLiquidity::Maker),
                last_fill_fee: None,
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

        let account_intents = legacy_intents(strategy.on_account_update(&AccountUpdate {
            ts_ms: 10,
            balances: Vec::new(),
            positions: vec![reap_core::Position {
                symbol: "BTC-USDT".to_string(),
                qty: 0.1,
                avg_price: 50_000.0,
                margin_mode: None,
            }],
            margins: Vec::new(),
        }));
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
            time_in_force: Some(TimeInForce::Ioc),
            qty: 1_000.0,
            open_qty: 0.0,
            filled_qty: 200.0,
            avg_fill_price: 100.0,
            last_fill_qty: 0.0,
            last_fill_price: 0.0,
            last_fill_liquidity: None,
            last_fill_fee: None,
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
                .hedging
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
                .hedging
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
                .hedging
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
        assert!(!strategy.reference_health.startup_basis_checked);

        let intents = strategy.on_system_event(&SystemEvent {
            ts_ms: 2,
            kind: SystemEventKind::FeedHeartbeat,
            venue: None,
            account_id: None,
            symbol: Some("BTC-USDT.OK".to_string()),
            reason: "accepted sequence".to_string(),
        });

        assert!(intents.is_empty());
        assert!(!strategy.reference_health.startup_basis_checked);
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
        assert!(!strategy.reference_health.startup_basis_checked);
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
            time_in_force: Some(TimeInForce::Ioc),
            qty: 1.0,
            open_qty: 1.0,
            filled_qty: 0.0,
            avg_fill_price: 0.0,
            last_fill_qty: 0.0,
            last_fill_price: 0.0,
            last_fill_liquidity: None,
            last_fill_fee: None,
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
