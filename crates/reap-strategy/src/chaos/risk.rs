use std::collections::{HashMap, HashSet};

use reap_core::{AccountUpdate, Quantity, Side, Symbol};

use crate::ChaosExecutionIntent;

use super::{
    ChaosStrategy, EXCHANGE_MARGIN_RATIO_THRESHOLD, HedgeLevel, ORDER_CHECK_DELTA_THRESHOLD_USD,
    RiskGroupConfig, RiskGroupKindConfig, StableMap, ZOMBIE_HEDGE_THRESHOLD_MS,
};

#[derive(Debug, Clone)]
pub struct RiskGroupState {
    pub config: RiskGroupConfig,
    pub symbols: HashSet<Symbol>,
    pub(super) ordered_symbols: Vec<Symbol>,
    pub max_quote_size_usd: f64,
    pub delta_usd: f64,
    pub pending_delta_usd: f64,
    pub live_order_size_usd: f64,
    pub margin_ratio: Option<f64>,
    pub exchange_margin_ratio: Option<f64>,
    margin_adjusted_equity_usd: Option<f64>,
    margin_notional_usd: Option<f64>,
    pub best_hedges: HashMap<Side, Vec<HedgeLevel>>,
    account_balances: StableMap<String, AccountBalanceState>,
}

impl RiskGroupState {
    pub(super) fn new(config: RiskGroupConfig) -> Self {
        let mut ordered_symbols = config.symbols.clone();
        ordered_symbols.sort();
        ordered_symbols.dedup();
        let symbols = ordered_symbols.iter().cloned().collect();
        let mut best_hedges = HashMap::new();
        best_hedges.insert(Side::Buy, Vec::new());
        best_hedges.insert(Side::Sell, Vec::new());
        Self {
            config,
            symbols,
            ordered_symbols,
            max_quote_size_usd: 0.0,
            delta_usd: 0.0,
            pending_delta_usd: 0.0,
            live_order_size_usd: 0.0,
            margin_ratio: None,
            exchange_margin_ratio: None,
            margin_adjusted_equity_usd: None,
            margin_notional_usd: None,
            best_hedges,
            account_balances: StableMap::default(),
        }
    }

    pub(super) fn best_hedges_for(&self, side: Side) -> &[HedgeLevel] {
        self.best_hedges
            .get(&side)
            .map(Vec::as_slice)
            .unwrap_or(&[])
    }

    pub(super) fn can_quote_side(&self, side: Side) -> bool {
        match side {
            Side::Buy => {
                self.delta_usd + self.max_quote_size_usd < self.config.hard_delta_limit_usd
            }
            Side::Sell => {
                self.delta_usd - self.max_quote_size_usd > -self.config.hard_delta_limit_usd
            }
        }
    }

    pub(super) fn can_increase_delta_with_quote_buffer(&self, side: Side) -> bool {
        match side {
            Side::Buy => {
                self.delta_usd + self.max_quote_size_usd < self.config.soft_delta_limit_usd
            }
            Side::Sell => {
                self.delta_usd - self.max_quote_size_usd > -self.config.soft_delta_limit_usd
            }
        }
    }

    pub(super) fn must_hedge_within_group(&self, delta_to_hedge: f64) -> bool {
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

impl ChaosStrategy {
    pub(super) fn check_risk_limits(&mut self) -> bool {
        if !self.check_strategy_risk_limits() {
            return false;
        }
        if !self.check_risk_group_limits() {
            return false;
        }
        let Some(ref_mid) = self.ref_mid() else {
            return true;
        };
        if !self.check_trading_pnl_limit(ref_mid) {
            return false;
        }
        self.check_liabilities_and_balance_sheet(ref_mid)
    }

    fn check_strategy_risk_limits(&mut self) -> bool {
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
        true
    }

    fn check_risk_group_limits(&mut self) -> bool {
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
        true
    }

    fn check_trading_pnl_limit(&mut self, ref_mid: f64) -> bool {
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
        true
    }

    fn check_liabilities_and_balance_sheet(&mut self, ref_mid: f64) -> bool {
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

    pub(super) fn update_risk(&mut self) {
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
                let denominator = notional + liability_usd;
                rg.margin_ratio = (denominator > 0.0).then_some(equity / denominator);
            }
            let mut spot_delta_coin = 0.0;
            let mut derivative_delta_coin = 0.0;
            let mut seen_spot_balances = HashSet::new();
            for symbol in &rg.ordered_symbols {
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

    pub(super) fn on_account_update(
        &mut self,
        update: &AccountUpdate,
    ) -> Vec<ChaosExecutionIntent> {
        self.advance_time(update.ts_ms);
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
}

fn is_usd_equivalent(currency: &str) -> bool {
    matches!(currency, "USD" | "USDT" | "USDC" | "FDUSD" | "BUSD")
}
