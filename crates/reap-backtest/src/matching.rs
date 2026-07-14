use std::collections::HashMap;

use reap_book::BookReducer;
use reap_core::{
    FillLiquidity, Level, NewOrder, OrderBook, OrderStatus, OrderUpdate, Price, Quantity, Side,
    Symbol, TimeInForce, round_down_to_lot,
};
use reap_order::OrderReducer;
use reap_strategy::InstrumentConfig;

#[derive(Debug, Clone, Copy, PartialEq)]
pub(crate) struct MatchingAssumptions {
    pub(crate) depth_fill_conservative_threshold: f64,
    pub(crate) queue_ahead_multiplier: f64,
    pub(crate) historical_trade_fill_fraction: f64,
    pub(crate) displayed_depth_fill_fraction: f64,
}

impl Default for MatchingAssumptions {
    fn default() -> Self {
        Self {
            depth_fill_conservative_threshold: 0.0,
            queue_ahead_multiplier: 1.0,
            historical_trade_fill_fraction: 1.0,
            displayed_depth_fill_fraction: 1.0,
        }
    }
}

#[derive(Debug, Clone)]
pub struct MatchingEngine {
    symbol: Symbol,
    instrument: InstrumentConfig,
    book: BookReducer,
    // Shared simulated liquidity budget, kept separate from observed market state.
    matching_depth: BookReducer,
    orders: OrderReducer,
    meta: HashMap<String, MatchingMeta>,
    next_order_id: u64,
    next_seq: u64,
    px_threshold: f64,
    assumptions: MatchingAssumptions,
    now_ms: u64,
}

impl MatchingEngine {
    pub fn new(instrument: InstrumentConfig) -> Self {
        Self::with_assumptions(instrument, MatchingAssumptions::default())
    }

    pub fn with_depth_fill_conservative_threshold(
        instrument: InstrumentConfig,
        depth_fill_conservative_threshold: f64,
    ) -> Self {
        Self::with_assumptions(
            instrument,
            MatchingAssumptions {
                depth_fill_conservative_threshold,
                ..MatchingAssumptions::default()
            },
        )
    }

    pub(crate) fn with_assumptions(
        instrument: InstrumentConfig,
        assumptions: MatchingAssumptions,
    ) -> Self {
        Self {
            symbol: instrument.symbol.clone(),
            px_threshold: instrument.tick_size * 0.1,
            book: BookReducer::new(instrument.symbol.clone()),
            matching_depth: BookReducer::new(instrument.symbol.clone()),
            instrument,
            orders: OrderReducer::new(),
            meta: HashMap::new(),
            next_order_id: 1,
            next_seq: 1,
            assumptions,
            now_ms: 0,
        }
    }

    pub fn depth(&self) -> Option<&OrderBook> {
        self.book.book()
    }

    pub fn contains_order(&self, order_id: &str) -> bool {
        self.orders.contains_order(order_id)
    }

    pub fn is_pending(&self, order_id: &str) -> bool {
        self.orders
            .get(order_id)
            .is_some_and(|order| order.status == OrderStatus::PendingNew)
    }

    pub fn is_open_order(&self, order_id: &str) -> bool {
        self.orders
            .get(order_id)
            .is_some_and(|order| order.status == OrderStatus::PendingNew || order.is_live())
    }

    pub fn pending_order_count(&self) -> usize {
        self.orders
            .orders()
            .filter(|(_, order)| order.status == OrderStatus::PendingNew)
            .count()
    }

    pub fn live_order_count(&self) -> usize {
        self.orders
            .orders()
            .filter(|(_, order)| order.is_live())
            .count()
    }

    /// Gross quote/settlement notional of pending and live remainders at limit price.
    pub fn active_order_notional_checked(&self) -> Option<f64> {
        let mut total = 0.0;
        for (_, order) in self
            .orders
            .orders()
            .filter(|(_, order)| order.status == OrderStatus::PendingNew || order.is_live())
        {
            let qty = order.open_qty.abs();
            if !qty.is_finite() {
                return None;
            }
            let notional = if self.instrument.kind.is_inverse() {
                qty * self.instrument.contract_value
            } else {
                if !order.price.is_finite() || order.price <= 0.0 {
                    return None;
                }
                if self.instrument.kind.is_spot() {
                    qty * order.price
                } else {
                    qty * self.instrument.contract_value * order.price
                }
            };
            if !notional.is_finite() {
                return None;
            }
            total += notional;
        }
        total.is_finite().then_some(total)
    }

    pub fn prepare_submit(&mut self, mut order: NewOrder, ts_ms: u64) -> (String, OrderUpdate) {
        let order_id = format!("{}-{}", self.symbol, self.next_order_id);
        self.next_order_id += 1;
        order.qty = round_down_to_lot(order.qty, self.instrument.lot_size);
        let pending = self.orders.pending_new(order_id.clone(), order, ts_ms);
        (order_id, pending)
    }

    pub fn activate(&mut self, order_id: &str, ts_ms: u64) -> Vec<OrderUpdate> {
        self.now_ms = ts_ms;
        let Some(snapshot) = self.orders.get(order_id) else {
            return Vec::new();
        };
        if snapshot.status != OrderStatus::PendingNew {
            return Vec::new();
        }
        let order = NewOrder {
            symbol: snapshot.symbol.clone(),
            side: snapshot.side,
            qty: snapshot.qty,
            price: snapshot.price,
            time_in_force: snapshot.time_in_force.unwrap_or(TimeInForce::Gtc),
            reduce_only: false,
            self_trade_prevention: None,
            reason: snapshot.reason.clone(),
        };
        let seq = self.next_seq;
        self.next_seq += 1;

        let mut updates = Vec::new();

        if order.qty <= 0.0
            || !order.qty.is_finite()
            || order.price <= 0.0
            || !order.price.is_finite()
        {
            updates.extend(self.orders.reject(order_id, self.now_ms, "invalid_order"));
            self.remove_terminal_orders();
            return updates;
        }

        if self.book.book().is_none() {
            if order.time_in_force == TimeInForce::Ioc {
                updates.extend(self.orders.cancel(order_id, self.now_ms, "no_depth"));
            } else {
                self.add_matching_meta(order_id.to_string(), seq);
                self.add_qty_ahead(order_id);
                updates.extend(self.orders.mark_live(order_id, self.now_ms, "new"));
            }
            self.remove_terminal_orders();
            return updates;
        }

        let crosses = self.crosses_current_depth(&order);
        if order.time_in_force == TimeInForce::PostOnly && crosses {
            updates.extend(self.orders.cancel(order_id, self.now_ms, "post_only_cross"));
            self.remove_terminal_orders();
            return updates;
        }
        if order.time_in_force == TimeInForce::Ioc && !crosses {
            updates.extend(self.orders.cancel(order_id, self.now_ms, "ioc_miss"));
            self.remove_terminal_orders();
            return updates;
        }

        if !crosses {
            self.add_matching_meta(order_id.to_string(), seq);
            self.add_qty_ahead(order_id);
            updates.extend(self.orders.mark_live(order_id, self.now_ms, "new"));
            self.remove_terminal_orders();
            return updates;
        }

        updates.extend(self.match_taker_order(order_id));
        let open_qty = self
            .orders
            .get(order_id)
            .map(|order| order.open_qty)
            .unwrap_or_default();
        if open_qty > 0.0 && order.time_in_force == TimeInForce::Ioc {
            updates.extend(self.orders.cancel(order_id, self.now_ms, "ioc_remainder"));
        } else if open_qty > 0.0 {
            self.add_matching_meta(order_id.to_string(), seq);
            self.add_qty_ahead(order_id);
            updates.extend(
                self.orders
                    .mark_live(order_id, self.now_ms, "resting_remainder"),
            );
        }
        self.remove_terminal_orders();
        updates
    }

    pub fn submit(&mut self, order: NewOrder) -> Vec<OrderUpdate> {
        let (order_id, pending) = self.prepare_submit(order, self.now_ms);
        let mut updates = vec![pending];
        updates.extend(self.activate(&order_id, self.now_ms));
        updates
    }

    pub fn cancel_at(&mut self, order_id: &str, ts_ms: u64, reason: &str) -> Vec<OrderUpdate> {
        self.now_ms = ts_ms;
        if !self.orders.contains_order(order_id) {
            return Vec::new();
        }
        self.meta.remove(order_id);
        self.orders
            .cancel(order_id, self.now_ms, reason)
            .into_iter()
            .collect()
    }

    pub fn cancel(&mut self, order_id: &str, reason: &str) -> Vec<OrderUpdate> {
        self.cancel_at(order_id, self.now_ms, reason)
    }

    pub fn on_depth(&mut self, depth: OrderBook) -> Vec<OrderUpdate> {
        let ts_ms = depth.ts_ms;
        self.on_depth_at(depth, ts_ms)
    }

    pub fn on_depth_at(&mut self, depth: OrderBook, ts_ms: u64) -> Vec<OrderUpdate> {
        self.now_ms = ts_ms;
        let matching_depth = self.next_matching_depth(&depth);
        self.book.apply_snapshot(depth);
        self.matching_depth.apply_snapshot(matching_depth);
        self.match_live_orders_on_depth()
    }

    pub fn on_trade(
        &mut self,
        ts_ms: u64,
        price: Price,
        qty: Quantity,
        taker_side: Side,
    ) -> Vec<OrderUpdate> {
        self.on_trade_at(price, qty, taker_side, ts_ms)
    }

    pub fn on_trade_at(
        &mut self,
        price: Price,
        qty: Quantity,
        taker_side: Side,
        simulation_ts_ms: u64,
    ) -> Vec<OrderUpdate> {
        self.now_ms = simulation_ts_ms;
        let maker_side = taker_side.reverse();
        let mut qty_remaining = qty * self.assumptions.historical_trade_fill_fraction;
        let order_ids = self.priority_order_ids(maker_side);
        let mut updates = Vec::new();

        for order_id in order_ids {
            if qty_remaining <= 0.0 {
                break;
            }
            let Some((order_price, open_qty)) = self
                .orders
                .get(&order_id)
                .map(|order| (order.price, order.open_qty))
            else {
                continue;
            };
            if !taker_side.crosses(price, order_price) {
                break;
            }

            if let Some(meta) = self.meta.get_mut(&order_id)
                && meta.qty_ahead > 0.0
            {
                if approx_eq(price, order_price, self.px_threshold) {
                    let consumed = qty_remaining.min(meta.qty_ahead);
                    meta.qty_ahead -= consumed;
                    qty_remaining -= consumed;
                    if qty_remaining <= 0.0 {
                        break;
                    }
                } else {
                    meta.qty_ahead = 0.0;
                }
            }

            let fill_qty = open_qty.min(qty_remaining);
            if fill_qty <= 0.0 {
                continue;
            }
            qty_remaining -= fill_qty;
            updates.extend(self.orders.fill(
                &order_id,
                self.now_ms,
                fill_qty,
                order_price,
                FillLiquidity::Maker,
            ));
        }

        self.remove_terminal_orders();
        updates
    }

    fn match_live_orders_on_depth(&mut self) -> Vec<OrderUpdate> {
        let mut updates = Vec::new();
        updates.extend(self.match_live_side_on_depth(Side::Sell));
        updates.extend(self.match_live_side_on_depth(Side::Buy));
        self.remove_terminal_orders();
        updates
    }

    fn match_live_side_on_depth(&mut self, maker_side: Side) -> Vec<OrderUpdate> {
        let market_side = maker_side.reverse();
        let mut updates = Vec::new();

        for order_id in self.priority_order_ids(maker_side) {
            let Some((order_price, open_qty)) = self
                .orders
                .get(&order_id)
                .map(|order| (order.price, order.open_qty))
            else {
                continue;
            };
            let crosses_observed_depth = self
                .book
                .best(market_side)
                .is_some_and(|level| market_side.crosses(level.px, order_price));
            if !crosses_observed_depth {
                break;
            }
            if let Some(meta) = self.meta.get_mut(&order_id) {
                meta.qty_ahead = 0.0;
            }

            let threshold = self.assumptions.depth_fill_conservative_threshold;
            let matching_limit = match maker_side {
                Side::Buy => order_price * (1.0 - threshold),
                Side::Sell => order_price * (1.0 + threshold),
            };
            for fill in self
                .matching_depth
                .take_liquidity(maker_side, matching_limit, open_qty)
            {
                updates.extend(self.orders.fill(
                    &order_id,
                    self.now_ms,
                    fill.qty,
                    order_price,
                    FillLiquidity::Maker,
                ));
            }
        }
        updates
    }

    fn match_taker_order(&mut self, order_id: &str) -> Vec<OrderUpdate> {
        let Some(order) = self.orders.get(order_id).cloned() else {
            return Vec::new();
        };
        self.matching_depth
            .take_liquidity(order.side, order.price, order.open_qty)
            .into_iter()
            .filter_map(|fill| {
                self.orders.fill(
                    order_id,
                    self.now_ms,
                    fill.qty,
                    fill.px,
                    FillLiquidity::Taker,
                )
            })
            .collect()
    }

    fn add_matching_meta(&mut self, order_id: String, seq: u64) {
        self.meta.insert(
            order_id,
            MatchingMeta {
                seq,
                qty_ahead: 0.0,
            },
        );
    }

    fn add_qty_ahead(&mut self, order_id: &str) {
        let Some((side, price)) = self
            .orders
            .get(order_id)
            .map(|order| (order.side, order.price))
        else {
            return;
        };
        let qty_ahead = self
            .book
            .levels(side)
            .iter()
            .find_map(|level| {
                if side.is_more_passive(level.px, price) {
                    Some(0.0)
                } else if approx_eq(level.px, price, self.px_threshold) {
                    Some(level.qty)
                } else {
                    None
                }
            })
            .unwrap_or(0.0)
            * self.assumptions.queue_ahead_multiplier;
        if let Some(meta) = self.meta.get_mut(order_id) {
            meta.qty_ahead = qty_ahead;
        }
    }

    fn crosses_current_depth(&self, order: &NewOrder) -> bool {
        self.book
            .best(order.side.reverse())
            .is_some_and(|level| order.side.crosses(order.price, level.px))
    }

    fn next_matching_depth(&self, depth: &OrderBook) -> OrderBook {
        let fraction = self.assumptions.displayed_depth_fill_fraction;
        if fraction == 1.0 {
            return depth.clone();
        }
        OrderBook {
            symbol: depth.symbol.clone(),
            ts_ms: depth.ts_ms,
            bids: capacity_levels(
                &depth.bids,
                self.book.levels(Side::Buy),
                self.matching_depth.levels(Side::Buy),
                fraction,
                self.px_threshold,
            ),
            asks: capacity_levels(
                &depth.asks,
                self.book.levels(Side::Sell),
                self.matching_depth.levels(Side::Sell),
                fraction,
                self.px_threshold,
            ),
        }
    }

    fn priority_order_ids(&self, side: Side) -> Vec<String> {
        let mut orders = self
            .meta
            .iter()
            .filter_map(|(order_id, meta)| {
                let order = self.orders.get(order_id)?;
                if order.side == side && order.is_live() {
                    Some((order_id.clone(), order.price, meta.seq))
                } else {
                    None
                }
            })
            .collect::<Vec<_>>();
        orders.sort_by(|a, b| match side {
            Side::Buy => b.1.total_cmp(&a.1).then(a.2.cmp(&b.2)),
            Side::Sell => a.1.total_cmp(&b.1).then(a.2.cmp(&b.2)),
        });
        orders
            .into_iter()
            .map(|(order_id, _, _)| order_id)
            .collect()
    }

    fn remove_terminal_orders(&mut self) {
        let ids = self
            .meta
            .keys()
            .filter(|order_id| !self.orders.is_live(order_id))
            .cloned()
            .collect::<Vec<_>>();
        for order_id in ids {
            self.meta.remove(&order_id);
        }
    }
}

#[derive(Debug, Clone)]
struct MatchingMeta {
    seq: u64,
    qty_ahead: Quantity,
}

fn approx_eq(a: f64, b: f64, threshold: f64) -> bool {
    (a - b).abs() <= threshold
}

fn capacity_levels(
    current: &[Level],
    previous: &[Level],
    remaining: &[Level],
    fraction: f64,
    px_threshold: f64,
) -> Vec<Level> {
    current
        .iter()
        .filter_map(|level| {
            let previous_qty = previous
                .iter()
                .find(|candidate| approx_eq(candidate.px, level.px, px_threshold))
                .map(|candidate| candidate.qty);
            let remaining_qty = remaining
                .iter()
                .find(|candidate| approx_eq(candidate.px, level.px, px_threshold))
                .map(|candidate| candidate.qty)
                .unwrap_or(0.0);
            let capacity = match previous_qty {
                Some(previous_qty) if level.qty > previous_qty => {
                    remaining_qty + (level.qty - previous_qty) * fraction
                }
                Some(_) => remaining_qty,
                None => level.qty * fraction,
            }
            .min(level.qty * fraction);
            (capacity > 0.0).then_some(Level::new(level.px, capacity))
        })
        .collect()
}

#[allow(dead_code)]
fn level(px: Price, qty: Quantity) -> Level {
    Level { px, qty }
}

#[cfg(test)]
mod tests {
    use reap_core::OrderEvent;
    use reap_strategy::InstrumentKindConfig;

    use super::*;

    fn inst() -> InstrumentConfig {
        InstrumentConfig {
            symbol: "BTC-USDT".to_string(),
            kind: InstrumentKindConfig::Spot,
            tick_size: 0.1,
            lot_size: 0.0001,
            ..InstrumentConfig::default()
        }
    }

    #[test]
    fn post_only_cross_cancels() {
        let mut engine = MatchingEngine::new(inst());
        engine.on_depth(OrderBook::one_level(
            "BTC-USDT",
            1,
            level(100.0, 1.0),
            level(101.0, 1.0),
        ));
        let updates = engine.submit(NewOrder {
            symbol: "BTC-USDT".to_string(),
            side: Side::Buy,
            qty: 1.0,
            price: 101.0,
            time_in_force: TimeInForce::PostOnly,
            reduce_only: false,
            self_trade_prevention: None,
            reason: "quote".to_string(),
        });
        assert_eq!(updates[0].event, OrderEvent::PendingNew);
        assert_eq!(updates[1].event, OrderEvent::Cancelled);
    }

    #[test]
    fn active_order_notional_includes_pending_orders() {
        let mut engine = MatchingEngine::new(inst());
        engine.prepare_submit(
            NewOrder {
                symbol: "BTC-USDT".to_string(),
                side: Side::Buy,
                qty: 2.0,
                price: 100.0,
                time_in_force: TimeInForce::PostOnly,
                reduce_only: false,
                self_trade_prevention: None,
                reason: "quote".to_string(),
            },
            1,
        );

        assert_eq!(engine.pending_order_count(), 1);
        assert_eq!(engine.active_order_notional_checked(), Some(200.0));
    }

    #[test]
    fn ioc_cross_fills_as_taker() {
        let mut engine = MatchingEngine::new(inst());
        engine.on_depth(OrderBook::one_level(
            "BTC-USDT",
            1,
            level(100.0, 1.0),
            level(101.0, 1.0),
        ));
        let updates = engine.submit(NewOrder {
            symbol: "BTC-USDT".to_string(),
            side: Side::Buy,
            qty: 0.5,
            price: 101.0,
            time_in_force: TimeInForce::Ioc,
            reduce_only: false,
            self_trade_prevention: None,
            reason: "hedge".to_string(),
        });
        assert_eq!(updates[0].event, OrderEvent::PendingNew);
        assert_eq!(updates[1].event, OrderEvent::FullyFilled);
        assert_eq!(updates[1].last_fill_liquidity, Some(FillLiquidity::Taker));
    }

    #[test]
    fn ioc_miss_is_terminal_without_a_fill() {
        let mut engine = MatchingEngine::new(inst());
        engine.on_depth(OrderBook::one_level(
            "BTC-USDT",
            1,
            level(100.0, 1.0),
            level(101.0, 1.0),
        ));
        let updates = engine.submit(NewOrder {
            symbol: "BTC-USDT".to_string(),
            side: Side::Buy,
            qty: 0.5,
            price: 100.0,
            time_in_force: TimeInForce::Ioc,
            reduce_only: false,
            self_trade_prevention: None,
            reason: "hedge".to_string(),
        });

        assert_eq!(updates.len(), 2);
        assert_eq!(updates[0].event, OrderEvent::PendingNew);
        assert_eq!(updates[1].event, OrderEvent::Cancelled);
        assert_eq!(updates[1].filled_qty, 0.0);
    }

    #[test]
    fn ioc_partial_fill_cancels_the_remainder() {
        let mut engine = MatchingEngine::new(inst());
        engine.on_depth(OrderBook::one_level(
            "BTC-USDT",
            1,
            level(100.0, 1.0),
            level(101.0, 0.4),
        ));
        let updates = engine.submit(NewOrder {
            symbol: "BTC-USDT".to_string(),
            side: Side::Buy,
            qty: 1.0,
            price: 101.0,
            time_in_force: TimeInForce::Ioc,
            reduce_only: false,
            self_trade_prevention: None,
            reason: "hedge".to_string(),
        });

        assert_eq!(updates.len(), 3);
        assert_eq!(updates[0].event, OrderEvent::PendingNew);
        assert_eq!(updates[1].event, OrderEvent::PartialFill);
        assert_eq!(updates[1].filled_qty, 0.4);
        assert_eq!(updates[2].event, OrderEvent::Cancelled);
        assert_eq!(updates[2].open_qty, 0.6);
    }

    #[test]
    fn delayed_activation_matches_against_the_latest_book() {
        let mut engine = MatchingEngine::new(inst());
        engine.on_depth(OrderBook::one_level(
            "BTC-USDT",
            1,
            level(99.0, 1.0),
            level(101.0, 1.0),
        ));
        let (order_id, pending) = engine.prepare_submit(
            NewOrder {
                symbol: "BTC-USDT".to_string(),
                side: Side::Buy,
                qty: 0.5,
                price: 100.0,
                time_in_force: TimeInForce::PostOnly,
                reduce_only: false,
                self_trade_prevention: None,
                reason: "quote".to_string(),
            },
            2,
        );
        assert_eq!(pending.event, OrderEvent::PendingNew);

        engine.on_depth_at(
            OrderBook::one_level("BTC-USDT", 3, level(99.0, 1.0), level(100.0, 1.0)),
            5,
        );
        let updates = engine.activate(&order_id, 12);

        assert_eq!(updates.len(), 1);
        assert_eq!(updates[0].event, OrderEvent::Cancelled);
        assert_eq!(updates[0].ts_ms, 12);
        assert!(updates[0].reason.ends_with("post_only_cross"));
    }

    #[test]
    fn cancel_before_activation_prevents_late_order_entry() {
        let mut engine = MatchingEngine::new(inst());
        let (order_id, _) = engine.prepare_submit(
            NewOrder {
                symbol: "BTC-USDT".to_string(),
                side: Side::Buy,
                qty: 0.5,
                price: 100.0,
                time_in_force: TimeInForce::PostOnly,
                reduce_only: false,
                self_trade_prevention: None,
                reason: "quote".to_string(),
            },
            2,
        );

        let cancelled = engine.cancel_at(&order_id, 4, "replace_quote");
        let activated = engine.activate(&order_id, 10);

        assert_eq!(cancelled.len(), 1);
        assert_eq!(cancelled[0].event, OrderEvent::Cancelled);
        assert!(activated.is_empty());
        assert!(!engine.is_open_order(&order_id));
    }

    #[test]
    fn conservative_depth_threshold_blocks_shallow_cross_but_clears_queue_ahead() {
        let mut engine = MatchingEngine::with_depth_fill_conservative_threshold(inst(), 0.01);
        engine.on_depth(OrderBook::one_level(
            "BTC-USDT",
            1,
            level(100.0, 1.0),
            level(101.0, 1.0),
        ));
        engine.submit(NewOrder {
            symbol: "BTC-USDT".to_string(),
            side: Side::Buy,
            qty: 0.5,
            price: 100.0,
            time_in_force: TimeInForce::PostOnly,
            reduce_only: false,
            self_trade_prevention: None,
            reason: "quote".to_string(),
        });

        let depth_updates = engine.on_depth(OrderBook::one_level(
            "BTC-USDT",
            2,
            level(99.0, 1.0),
            level(100.0, 1.0),
        ));
        let trade_updates = engine.on_trade(3, 100.0, 0.5, Side::Sell);

        assert!(depth_updates.is_empty());
        assert_eq!(trade_updates.len(), 1);
        assert_eq!(trade_updates[0].event, OrderEvent::FullyFilled);
        assert_eq!(
            trade_updates[0].last_fill_liquidity,
            Some(FillLiquidity::Maker)
        );
    }

    #[test]
    fn conservative_depth_threshold_fills_a_sufficiently_deep_cross() {
        let mut engine = MatchingEngine::with_depth_fill_conservative_threshold(inst(), 0.001);
        engine.on_depth(OrderBook::one_level(
            "BTC-USDT",
            1,
            level(100.0, 1.0),
            level(101.0, 1.0),
        ));
        engine.submit(NewOrder {
            symbol: "BTC-USDT".to_string(),
            side: Side::Buy,
            qty: 0.5,
            price: 100.0,
            time_in_force: TimeInForce::PostOnly,
            reduce_only: false,
            self_trade_prevention: None,
            reason: "quote".to_string(),
        });

        let updates = engine.on_depth(OrderBook::one_level(
            "BTC-USDT",
            2,
            level(99.0, 1.0),
            level(99.8, 1.0),
        ));

        assert_eq!(updates.len(), 1);
        assert_eq!(updates[0].event, OrderEvent::FullyFilled);
        assert_eq!(updates[0].last_fill_liquidity, Some(FillLiquidity::Maker));
    }

    #[test]
    fn conservative_depth_threshold_is_symmetric_for_resting_sells() {
        let mut engine = MatchingEngine::with_depth_fill_conservative_threshold(inst(), 0.001);
        engine.on_depth(OrderBook::one_level(
            "BTC-USDT",
            1,
            level(99.0, 1.0),
            level(100.0, 1.0),
        ));
        engine.submit(NewOrder {
            symbol: "BTC-USDT".to_string(),
            side: Side::Sell,
            qty: 0.5,
            price: 100.0,
            time_in_force: TimeInForce::PostOnly,
            reduce_only: false,
            self_trade_prevention: None,
            reason: "quote".to_string(),
        });

        let updates = engine.on_depth(OrderBook::one_level(
            "BTC-USDT",
            2,
            level(100.2, 1.0),
            level(101.0, 1.0),
        ));

        assert_eq!(updates.len(), 1);
        assert_eq!(updates[0].event, OrderEvent::FullyFilled);
        assert_eq!(updates[0].side, Side::Sell);
        assert_eq!(updates[0].last_fill_liquidity, Some(FillLiquidity::Maker));
    }

    #[test]
    fn queue_multiplier_requires_more_trade_volume_before_a_fill() {
        let mut engine = MatchingEngine::with_assumptions(
            inst(),
            MatchingAssumptions {
                queue_ahead_multiplier: 2.0,
                ..MatchingAssumptions::default()
            },
        );
        engine.on_depth(OrderBook::one_level(
            "BTC-USDT",
            1,
            level(100.0, 1.0),
            level(101.0, 1.0),
        ));
        engine.submit(NewOrder {
            symbol: "BTC-USDT".to_string(),
            side: Side::Buy,
            qty: 0.5,
            price: 100.0,
            time_in_force: TimeInForce::PostOnly,
            reduce_only: false,
            self_trade_prevention: None,
            reason: "quote".to_string(),
        });

        let updates = engine.on_trade(2, 100.0, 2.25, Side::Sell);

        assert_eq!(updates.len(), 1);
        assert_eq!(updates[0].event, OrderEvent::PartialFill);
        assert_eq!(updates[0].last_fill_qty, 0.25);
        assert_eq!(updates[0].open_qty, 0.25);
    }

    #[test]
    fn historical_trade_fraction_reduces_queue_consumption_and_fill() {
        let mut engine = MatchingEngine::with_assumptions(
            inst(),
            MatchingAssumptions {
                historical_trade_fill_fraction: 0.25,
                ..MatchingAssumptions::default()
            },
        );
        engine.on_depth(OrderBook::one_level(
            "BTC-USDT",
            1,
            level(100.0, 1.0),
            level(101.0, 1.0),
        ));
        engine.submit(NewOrder {
            symbol: "BTC-USDT".to_string(),
            side: Side::Buy,
            qty: 0.5,
            price: 99.5,
            time_in_force: TimeInForce::PostOnly,
            reduce_only: false,
            self_trade_prevention: None,
            reason: "quote".to_string(),
        });

        let updates = engine.on_trade(2, 99.5, 1.0, Side::Sell);

        assert_eq!(updates.len(), 1);
        assert_eq!(updates[0].event, OrderEvent::PartialFill);
        assert_eq!(updates[0].last_fill_qty, 0.25);
        assert_eq!(updates[0].open_qty, 0.25);
    }

    #[test]
    fn displayed_depth_fraction_caps_taker_fill_per_level() {
        let mut engine = MatchingEngine::with_assumptions(
            inst(),
            MatchingAssumptions {
                displayed_depth_fill_fraction: 0.5,
                ..MatchingAssumptions::default()
            },
        );
        engine.on_depth(OrderBook {
            symbol: "BTC-USDT".to_string(),
            ts_ms: 1,
            bids: vec![level(100.0, 1.0)],
            asks: vec![level(101.0, 1.0), level(102.0, 1.0)],
        });

        let updates = engine.submit(NewOrder {
            symbol: "BTC-USDT".to_string(),
            side: Side::Buy,
            qty: 1.0,
            price: 102.0,
            time_in_force: TimeInForce::Ioc,
            reduce_only: false,
            self_trade_prevention: None,
            reason: "hedge".to_string(),
        });

        assert_eq!(updates.len(), 3);
        assert_eq!(updates[1].event, OrderEvent::PartialFill);
        assert_eq!(updates[1].last_fill_qty, 0.5);
        assert_eq!(updates[1].last_fill_price, 101.0);
        assert_eq!(updates[2].event, OrderEvent::FullyFilled);
        assert_eq!(updates[2].last_fill_qty, 0.5);
        assert_eq!(updates[2].last_fill_price, 102.0);
        assert_eq!(updates[2].avg_fill_price, 101.5);
    }

    #[test]
    fn unchanged_depth_does_not_replenish_fractional_capacity() {
        let mut engine = MatchingEngine::with_assumptions(
            inst(),
            MatchingAssumptions {
                displayed_depth_fill_fraction: 0.5,
                ..MatchingAssumptions::default()
            },
        );
        let depth = OrderBook::one_level("BTC-USDT", 1, level(100.0, 1.0), level(101.0, 1.0));
        engine.on_depth(depth.clone());
        let ioc = NewOrder {
            symbol: "BTC-USDT".to_string(),
            side: Side::Buy,
            qty: 1.0,
            price: 101.0,
            time_in_force: TimeInForce::Ioc,
            reduce_only: false,
            self_trade_prevention: None,
            reason: "hedge".to_string(),
        };

        let first = engine.submit(ioc.clone());
        let second = engine.submit(ioc.clone());
        engine.on_depth_at(depth.clone(), 2);
        let after_unchanged_snapshot = engine.submit(ioc.clone());
        engine.on_depth_at(
            OrderBook::one_level("BTC-USDT", 3, level(100.0, 1.0), level(101.0, 2.0)),
            3,
        );
        let after_size_increase = engine.submit(ioc);

        assert_eq!(first[1].event, OrderEvent::PartialFill);
        assert_eq!(first[1].last_fill_qty, 0.5);
        assert_eq!(second.len(), 2);
        assert_eq!(second[1].event, OrderEvent::Cancelled);
        assert_eq!(second[1].filled_qty, 0.0);
        assert_eq!(after_unchanged_snapshot.len(), 2);
        assert_eq!(after_unchanged_snapshot[1].event, OrderEvent::Cancelled);
        assert_eq!(after_unchanged_snapshot[1].filled_qty, 0.0);
        assert_eq!(after_size_increase[1].event, OrderEvent::PartialFill);
        assert_eq!(after_size_increase[1].last_fill_qty, 0.5);
    }

    #[test]
    fn displayed_depth_fraction_caps_resting_depth_fill() {
        let mut engine = MatchingEngine::with_assumptions(
            inst(),
            MatchingAssumptions {
                displayed_depth_fill_fraction: 0.25,
                ..MatchingAssumptions::default()
            },
        );
        engine.on_depth(OrderBook::one_level(
            "BTC-USDT",
            1,
            level(98.0, 1.0),
            level(101.0, 1.0),
        ));
        engine.submit(NewOrder {
            symbol: "BTC-USDT".to_string(),
            side: Side::Buy,
            qty: 1.0,
            price: 100.0,
            time_in_force: TimeInForce::PostOnly,
            reduce_only: false,
            self_trade_prevention: None,
            reason: "quote".to_string(),
        });

        let updates = engine.on_depth(OrderBook::one_level(
            "BTC-USDT",
            2,
            level(98.0, 1.0),
            level(99.0, 1.0),
        ));

        assert_eq!(updates.len(), 1);
        assert_eq!(updates[0].event, OrderEvent::PartialFill);
        assert_eq!(updates[0].last_fill_qty, 0.25);
        assert_eq!(updates[0].open_qty, 0.75);
    }
}
