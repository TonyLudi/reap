use std::collections::HashMap;
use std::sync::Arc;

use reap_core::{
    FillLiquidity, OrderBook, Price, Quantity, Side, TimeMs, round_down_to_lot, round_to_tick,
};

use super::{
    CAN_QUOTE_FULL_SIZE_DEBOUNCE_MS, CAN_TRADE_DEBOUNCE_MS, CoinConfig, DebouncedCondition, EPS,
    EXTRA_MARGIN_BPS, HedgeCandidate, ImpliedDepthState, InstrumentConfig, SkewTypeConfig,
    TheoQuote, approx_eq, timestamp_is_fresh,
};

#[derive(Debug, Clone)]
struct FillHistoryState {
    buy_fill_qty: Quantity,
    buy_fill_notional: f64,
    sell_fill_qty: Quantity,
    sell_fill_notional: f64,
    last_aggressive_fill_ms: TimeMs,
    last_normal_fill_ms: TimeMs,
    aggressive_fill_count: u32,
}

#[derive(Debug, Clone)]
pub(super) struct TradeControlState {
    pub(super) base_coin_config: Option<CoinConfig>,
    pub(super) quote_coin_config: Option<CoinConfig>,
    pub(super) margin_coin_config: Option<CoinConfig>,
    pub(super) can_trade: HashMap<Side, bool>,
    pub(super) can_trade_debouncers: HashMap<Side, DebouncedCondition>,
    pub(super) reduced_quote_level_side: Option<Side>,
    pub(super) full_quote_balance_debouncer: DebouncedCondition,
    pub(super) take_buy_rate: f64,
    pub(super) take_sell_rate: f64,
}

#[derive(Debug, Clone)]
pub struct InstrumentState {
    pub config: InstrumentConfig,
    pub(super) symbol_key: Arc<str>,
    pub book: Option<OrderBook>,
    pub position_qty: Quantity,
    pub position_avg_price: Price,
    fills: FillHistoryState,
    pub funding_rate: f64,
    pub funding_time_ms: TimeMs,
    pub funding_rate_updated_ms: Option<TimeMs>,
    pub(super) funding_rate_active: bool,
    pub mark_price: Option<Price>,
    pub mark_price_updated_ms: Option<TimeMs>,
    pub limit_down: Option<Price>,
    pub limit_up: Option<Price>,
    pub price_limits_updated_ms: Option<TimeMs>,
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
    pub(super) ignore_best_level: bool,
    pub(super) reference_data_stale_threshold_ms: Option<TimeMs>,
    pub(super) interval_halted: bool,
    pub(super) system_halted: bool,
    pub(super) feed_stale: bool,
    pub(super) implied_depth: ImpliedDepthState,
    pub(super) trade: TradeControlState,
    pub buy_theo: Option<TheoQuote>,
    pub sell_theo: Option<TheoQuote>,
}

impl InstrumentState {
    pub(super) fn new(config: InstrumentConfig) -> Self {
        let funding_rate = config.funding_rate;
        let symbol_key = Arc::<str>::from(config.symbol.as_str());
        Self {
            config,
            symbol_key,
            book: None,
            position_qty: 0.0,
            position_avg_price: 0.0,
            fills: FillHistoryState {
                buy_fill_qty: 0.0,
                buy_fill_notional: 0.0,
                sell_fill_qty: 0.0,
                sell_fill_notional: 0.0,
                last_aggressive_fill_ms: 0,
                last_normal_fill_ms: 0,
                aggressive_fill_count: 0,
            },
            funding_rate,
            funding_time_ms: 0,
            funding_rate_updated_ms: None,
            funding_rate_active: true,
            mark_price: None,
            mark_price_updated_ms: None,
            limit_down: None,
            limit_up: None,
            price_limits_updated_ms: None,
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
            reference_data_stale_threshold_ms: None,
            interval_halted: false,
            system_halted: false,
            feed_stale: false,
            implied_depth: ImpliedDepthState::default(),
            trade: TradeControlState {
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
            },
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

    pub(super) fn set_theo(&mut self, side: Side, quote: TheoQuote) {
        match side {
            Side::Buy => self.buy_theo = Some(quote),
            Side::Sell => self.sell_theo = Some(quote),
        }
    }

    pub(super) fn clear_theo(&mut self, side: Side) {
        match side {
            Side::Buy => self.buy_theo = None,
            Side::Sell => self.sell_theo = None,
        }
    }

    pub(super) fn prevent_self_cross(&mut self) {
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

    pub(super) fn refresh_trade_permissions(&mut self, now_ms: TimeMs) {
        for side in [Side::Buy, Side::Sell] {
            if !self.is_valid_and_not_halted(now_ms) {
                self.trade.can_trade.insert(side, false);
                continue;
            }
            let raw = if self.config.kind.is_spot() {
                self.spot_can_trade_raw(side, now_ms)
            } else {
                self.max_trade_size(side, false) >= self.config.min_trade_size
            };
            let allowed = self
                .trade
                .can_trade_debouncers
                .entry(side)
                .or_default()
                .check(raw, now_ms, CAN_TRADE_DEBOUNCE_MS);
            self.trade.can_trade.insert(side, allowed);
        }
    }

    pub(super) fn is_valid_and_not_halted(&self, now_ms: TimeMs) -> bool {
        !self.config.halted
            && !self.interval_halted
            && !self.system_halted
            && !self.feed_stale
            && self.market_data_is_valid_at(now_ms)
    }

    pub(super) fn book_is_valid(&self) -> bool {
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

    pub(super) fn book_is_valid_at(&self, now_ms: TimeMs) -> bool {
        self.book_is_valid()
            && self.book.as_ref().is_some_and(|book| {
                now_ms.saturating_sub(book.ts_ms) <= self.config.depth_stale_threshold_ms
            })
    }

    pub(super) fn market_data_is_valid_at(&self, now_ms: TimeMs) -> bool {
        self.book_is_valid_at(now_ms) && self.reference_data_is_valid_at(now_ms)
    }

    pub(super) fn reference_data_is_valid_at(&self, now_ms: TimeMs) -> bool {
        let Some(max_age_ms) = self.reference_data_stale_threshold_ms else {
            return true;
        };
        let limits_fresh = self.limit_down.is_some()
            && self.limit_up.is_some()
            && self
                .price_limits_updated_ms
                .is_some_and(|updated_ms| timestamp_is_fresh(updated_ms, now_ms, max_age_ms));
        if !limits_fresh {
            return false;
        }
        if self.config.kind.is_derivative()
            && (self.mark_price.is_none()
                || !self
                    .mark_price_updated_ms
                    .is_some_and(|updated_ms| timestamp_is_fresh(updated_ms, now_ms, max_age_ms)))
        {
            return false;
        }
        !self.config.kind.is_swap()
            || self
                .funding_rate_updated_ms
                .is_some_and(|updated_ms| timestamp_is_fresh(updated_ms, now_ms, max_age_ms))
    }

    pub(super) fn spot_can_trade_raw(&mut self, side: Side, now_ms: TimeMs) -> bool {
        if self.max_trade_size(side, false) < self.config.min_trade_size {
            return false;
        }
        if !self.balances_initialized {
            return true;
        }
        let (Some(base), Some(quote), Some(mid)) = (
            self.trade.base_coin_config.clone(),
            self.trade.quote_coin_config.clone(),
            self.mid(),
        ) else {
            return true;
        };
        let full_levels = self.config.num_quote_levels;
        match side {
            Side::Buy => self.spot_buy_can_trade_raw(side, now_ms, &base, &quote, mid, full_levels),
            Side::Sell => {
                self.spot_sell_can_trade_raw(side, now_ms, &base, &quote, mid, full_levels)
            }
        }
    }

    fn spot_buy_can_trade_raw(
        &mut self,
        side: Side,
        now_ms: TimeMs,
        base: &CoinConfig,
        quote: &CoinConfig,
        mid: f64,
        full_levels: usize,
    ) -> bool {
        let full_sufficient = self.quote_balance_sufficient(
            self.config.max_order_size,
            full_levels,
            base.borrow_limit(mid),
            1.0,
            quote.min_balance,
        );
        if !full_sufficient {
            self.trade.reduced_quote_level_side = Some(side);
            if !self.quote_balance_sufficient(
                self.config.max_order_size,
                1,
                quote.borrow_limit(1.0),
                quote.safety_multiplier,
                quote.min_balance,
            ) {
                return false;
            }
        } else if self.trade.reduced_quote_level_side == Some(side) {
            let can_restore = self.quote_balance_sufficient(
                self.config.max_order_size,
                full_levels,
                base.borrow_limit(mid),
                2.0,
                quote.min_balance,
            );
            if self.trade.full_quote_balance_debouncer.check(
                can_restore,
                now_ms,
                CAN_QUOTE_FULL_SIZE_DEBOUNCE_MS,
            ) {
                self.trade.reduced_quote_level_side = None;
            }
        }
        self.quote_balance >= quote.min_balance
    }

    fn spot_sell_can_trade_raw(
        &mut self,
        side: Side,
        now_ms: TimeMs,
        base: &CoinConfig,
        quote: &CoinConfig,
        mid: f64,
        full_levels: usize,
    ) -> bool {
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
            self.trade.reduced_quote_level_side = Some(side);
            if !self.trade_balance_sufficient(
                self.config.max_order_size,
                1,
                base.borrow_limit(mid),
                safety,
                base.min_balance,
            ) {
                return false;
            }
        } else if self.trade.reduced_quote_level_side == Some(side) {
            let can_restore = self.trade_balance_sufficient(
                self.config.max_order_size,
                full_levels,
                base.borrow_limit(mid),
                2.0,
                base.min_balance,
            );
            if self.trade.full_quote_balance_debouncer.check(
                can_restore,
                now_ms,
                CAN_QUOTE_FULL_SIZE_DEBOUNCE_MS,
            ) {
                self.trade.reduced_quote_level_side = None;
            }
        }
        quote.max_balance <= 0.0 || self.quote_balance < quote.max_balance
    }

    pub(super) fn trade_balance_sufficient(
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

    pub(super) fn quote_balance_sufficient(
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

    pub(super) fn quote_level_count(&self, side: Side) -> usize {
        if self.trade.reduced_quote_level_side == Some(side) {
            1
        } else {
            self.config.num_quote_levels
        }
    }

    pub(super) fn can_quote(&self, side: Side) -> bool {
        self.quote_profit_margin() < 1.0
            && self.trade.can_trade.get(&side).copied().unwrap_or(false)
    }

    pub(super) fn can_take(&self, side: Side) -> bool {
        self.hedge_profit_margin() < 1.0
            && self.trade.can_trade.get(&side).copied().unwrap_or(false)
            && self.can_take_within_price_limit(side)
    }

    pub(super) fn update_take_rate(&mut self, side: Side, ref_mid: f64) {
        let rate = if self.trade.can_trade.get(&side).copied().unwrap_or(false) {
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
            Side::Buy => self.trade.take_buy_rate = rate,
            Side::Sell => self.trade.take_sell_rate = rate,
        }
    }

    pub(super) fn take_rate(&self, side: Side) -> f64 {
        match side {
            Side::Buy => self.trade.take_buy_rate,
            Side::Sell => self.trade.take_sell_rate,
        }
    }

    pub(super) fn max_trade_size(&self, side: Side, is_hedge: bool) -> f64 {
        let size_limit = self.config.max_order_size * if is_hedge { 10.0 } else { 1.0 };
        if let Some(available) = self.spot_balance_capacity(side, size_limit) {
            return available;
        }

        let buffer = self.config.max_order_size * self.config.safety_multiplier;
        let available_by_position = self.position_capacity(side, buffer);
        let Some(available_by_margin) = self.margin_capacity(side, buffer) else {
            return 0.0;
        };
        available_by_position
            .min(available_by_margin)
            .min(size_limit)
    }

    fn spot_balance_capacity(&self, side: Side, size_limit: f64) -> Option<f64> {
        if !self.config.kind.is_spot() || !self.balances_initialized {
            return None;
        }
        let (Some(base), Some(quote)) =
            (&self.trade.base_coin_config, &self.trade.quote_coin_config)
        else {
            return None;
        };
        let mid = self.mid().unwrap_or(0.0);
        if mid <= 0.0 {
            return Some(0.0);
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
                    return Some(0.0);
                }
                let base_capacity = base_equity - base.min_balance;
                base_capacity.min(base_available - quote.min_balance)
            }
        };
        Some(available.min(size_limit))
    }

    fn position_capacity(&self, side: Side, buffer: f64) -> f64 {
        match side {
            Side::Buy => self.config.max_position - self.position_qty - buffer,
            Side::Sell => self.position_qty - self.config.min_position - buffer,
        }
    }

    fn margin_capacity(&self, side: Side, buffer: f64) -> Option<f64> {
        if side.factor() * self.position_qty < 0.0 {
            Some(f64::MAX)
        } else if self.margin_initialized
            && let Some(margin_coin) = &self.trade.margin_coin_config
        {
            let mid = self.mid().unwrap_or(0.0);
            if mid <= 0.0 {
                return None;
            }
            let borrow_limit = margin_coin.borrow_limit(if self.config.kind.is_inverse() {
                mid
            } else {
                1.0
            });
            if self.margin_balance < -borrow_limit || self.margin_liability.abs() > borrow_limit {
                Some(0.0)
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
                Some(
                    available_coin / margin_coin_per_contract.max(EPS) * margin_multiplier - buffer,
                )
            }
        } else {
            Some(f64::MAX)
        }
    }

    pub(super) fn append_hedge_candidates(
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
    pub(super) fn hedge_levels(
        &self,
        hedge_side: Side,
        ref_mid: f64,
        own_quotes: &[(Side, Price, Quantity)],
    ) -> Vec<HedgeCandidate> {
        let mut levels = Vec::new();
        self.append_hedge_candidates(hedge_side, ref_mid, own_quotes, f64::INFINITY, &mut levels);
        levels
    }

    pub(super) fn max_hedge_chunk_qty(&self, ref_mid: f64) -> f64 {
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
            && let (Some(base), Some(quote)) =
                (&self.trade.base_coin_config, &self.trade.quote_coin_config)
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

    pub(super) fn mid(&self) -> Option<f64> {
        let bid = self.effective_levels(Side::Buy).first()?;
        let ask = self.effective_levels(Side::Sell).first()?;
        Some((bid.px + ask.px) * 0.5)
    }

    pub(super) fn opposite_touch(&self, quote_side: Side) -> Option<f64> {
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

    pub(super) fn quote_only_price(&self, side: Side, target: f64) -> f64 {
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

    pub(super) fn px_within_limit(&self, side: Side, px: f64) -> f64 {
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

    pub(super) fn can_take_within_price_limit(&self, side: Side) -> bool {
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

    pub(super) fn hedge_px(&self, hedge_side: Side, px: f64, agg_factor: f64) -> f64 {
        let side_to_take = hedge_side.reverse();
        let ag_mult = 1.0 - side_to_take.factor() * agg_factor;
        round_to_tick(px * ag_mult, self.config.tick_size)
    }

    pub(super) fn effective_levels(&self, side: Side) -> &[reap_core::Level] {
        let Some(book) = &self.book else {
            return &[];
        };
        let levels = book.levels(side);
        let first = self
            .implied_depth
            .first_valid_level(book, side, self.ignore_best_level);
        levels.get(first..).unwrap_or(&[])
    }

    pub(super) fn quote_qty_from_usd(&self, usd: f64, ref_mid: f64) -> f64 {
        self.size_from_usd(usd, ref_mid)
    }

    pub(super) fn size_from_usd(&self, usd: f64, ref_mid: f64) -> f64 {
        if self.config.kind.is_spot() {
            return self.mid().map(|mid| usd / mid).unwrap_or(0.0);
        }
        if self.config.kind.is_inverse() {
            return usd / self.config.contract_value.max(EPS);
        }
        usd / (self.mid().unwrap_or(ref_mid) * self.config.contract_value.max(EPS))
    }

    pub(super) fn notional_usd(&self, qty: f64, px: f64, ref_mid: f64) -> f64 {
        self.notional_coin(qty, px) * ref_mid
    }

    pub(super) fn notional_coin(&self, qty: f64, px: f64) -> f64 {
        if self.config.kind.is_spot() {
            return qty;
        }
        if self.config.kind.is_inverse() {
            return qty * self.config.contract_value / px.max(EPS);
        }
        qty * self.config.contract_value
    }

    pub(super) fn delta_coin(&self) -> f64 {
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

    pub(super) fn delta_coin_for_qty(&self, signed_qty: f64, px: f64) -> f64 {
        if self.config.kind.is_spot() {
            signed_qty
        } else if self.config.kind.is_inverse() {
            signed_qty * self.config.contract_value / px.max(EPS)
        } else {
            signed_qty * self.config.contract_value
        }
    }

    pub(super) fn record_fill(
        &mut self,
        side: Side,
        qty: f64,
        px: f64,
        _liquidity: Option<FillLiquidity>,
    ) {
        match side {
            Side::Buy => {
                self.fills.buy_fill_qty += qty;
                self.fills.buy_fill_notional += qty * px;
            }
            Side::Sell => {
                self.fills.sell_fill_qty += qty;
                self.fills.sell_fill_notional += qty * px;
            }
        }
    }

    pub(super) fn anomalous_fill_should_stop(
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
            self.fills.last_normal_fill_ms = now_ms;
            return false;
        }

        if now_ms.saturating_sub(self.fills.last_aggressive_fill_ms) < 1_000
            && now_ms.saturating_sub(self.fills.last_normal_fill_ms) > 2_000
            && self.fills.aggressive_fill_count >= 5
        {
            return true;
        }
        if now_ms.saturating_sub(self.fills.last_aggressive_fill_ms) > 600_000 {
            self.fills.aggressive_fill_count = 0;
        }
        self.fills.last_aggressive_fill_ms = now_ms;
        self.fills.aggressive_fill_count += 1;
        false
    }

    pub(super) fn fv_adjust(&self) -> f64 {
        self.posn_skew() + self.config.fv_offset - self.effective_funding_rate()
    }

    pub(super) fn effective_funding_rate(&self) -> f64 {
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

    pub(super) fn trading_pnl_usd(&self, ref_mid: f64) -> f64 {
        let mark = self.mid().unwrap_or(ref_mid);
        let buy_avg_px = if self.fills.buy_fill_qty > 0.0 {
            self.fills.buy_fill_notional / self.fills.buy_fill_qty
        } else {
            0.0
        };
        let sell_avg_px = if self.fills.sell_fill_qty > 0.0 {
            self.fills.sell_fill_notional / self.fills.sell_fill_qty
        } else {
            0.0
        };
        let average_fee_rate = (self.config.maker_fee + self.config.taker_fee) * 0.5;
        if self.config.kind.is_spot() {
            let gross_quote = (mark - buy_avg_px) * self.fills.buy_fill_qty
                + (sell_avg_px - mark) * self.fills.sell_fill_qty;
            let fees_quote =
                (self.fills.buy_fill_notional + self.fills.sell_fill_notional) * average_fee_rate;
            (gross_quote - fees_quote) / mark.max(EPS) * ref_mid
        } else if self.config.kind.is_inverse() {
            let mut pnl_coin = 0.0;
            if self.fills.buy_fill_qty > 0.0 {
                pnl_coin -= self.config.contract_value
                    * (self.fills.buy_fill_qty / mark.max(EPS)
                        - self.fills.buy_fill_qty / buy_avg_px.max(EPS));
                pnl_coin -= average_fee_rate * self.config.contract_value * self.fills.buy_fill_qty
                    / buy_avg_px.max(EPS);
            }
            if self.fills.sell_fill_qty > 0.0 {
                pnl_coin += self.config.contract_value
                    * (self.fills.sell_fill_qty / mark.max(EPS)
                        - self.fills.sell_fill_qty / sell_avg_px.max(EPS));
                pnl_coin -=
                    average_fee_rate * self.config.contract_value * self.fills.sell_fill_qty
                        / sell_avg_px.max(EPS);
            }
            pnl_coin * ref_mid
        } else {
            let mut pnl_coin = 0.0;
            if self.fills.buy_fill_qty > 0.0 {
                pnl_coin +=
                    self.config.contract_value * self.fills.buy_fill_qty * (mark - buy_avg_px)
                        / mark.max(EPS);
                pnl_coin -= average_fee_rate * self.config.contract_value * self.fills.buy_fill_qty;
            }
            if self.fills.sell_fill_qty > 0.0 {
                pnl_coin -=
                    self.config.contract_value * self.fills.sell_fill_qty * (mark - sell_avg_px)
                        / mark.max(EPS);
                pnl_coin -=
                    average_fee_rate * self.config.contract_value * self.fills.sell_fill_qty;
            }
            pnl_coin * ref_mid
        }
    }

    pub(super) fn posn_skew(&self) -> f64 {
        if self.config.kind.is_spot()
            && let (Some(base), Some(quote)) =
                (&self.trade.base_coin_config, &self.trade.quote_coin_config)
        {
            let base_skew = -base.integrated_skew(self.inventory_position());
            let quote_skew = -(self.quote_balance - quote.skew_offset) * quote.buy_skew;
            return base_skew - quote_skew;
        }
        -self.integrated_skew(self.position_qty)
    }

    pub(super) fn skew_rate_at(&self, target_pos: f64) -> f64 {
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

    pub(super) fn position_skew_zone(&self, position: f64) -> i8 {
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

    pub(super) fn integrated_skew(&self, target_pos: f64) -> f64 {
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

    pub(super) fn average_skew_rate_to(&self, target_pos: f64) -> f64 {
        if self.config.kind.is_spot()
            && let (Some(base), Some(quote)) =
                (&self.trade.base_coin_config, &self.trade.quote_coin_config)
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

    pub(super) fn in_extra_skew_zone(&self) -> bool {
        if !self.config.kind.is_derivative() || self.config.skew_type != SkewTypeConfig::Step {
            return false;
        }
        self.position_qty > self.config.pos_activation
            || self.position_qty < self.config.neg_activation
    }

    pub(super) fn hedge_profit_margin(&self) -> f64 {
        self.config.hedge_profit_margin
            + if self.in_extra_skew_zone() {
                EXTRA_MARGIN_BPS
            } else {
                0.0
            }
    }

    pub(super) fn quote_profit_margin(&self) -> f64 {
        self.config.quote_profit_margin
            + if self.in_extra_skew_zone() {
                EXTRA_MARGIN_BPS
            } else {
                0.0
            }
    }

    pub(super) fn hedge_aggression(&self) -> f64 {
        self.config.hedge_aggression
            + if self.in_extra_skew_zone() {
                EXTRA_MARGIN_BPS
            } else {
                0.0
            }
    }

    pub(super) fn inventory_position(&self) -> f64 {
        if self.config.kind.is_spot() && self.balances_initialized {
            self.base_balance
        } else {
            self.position_qty
        }
    }
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
