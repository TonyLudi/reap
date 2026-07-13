use std::collections::HashMap;

use reap_book::BookReducer;
use reap_core::{
    FillLiquidity, Level, NewOrder, OrderBook, OrderStatus, OrderUpdate, Price, Quantity, Side,
    Symbol, TimeInForce, round_down_to_lot,
};
use reap_order::OrderReducer;
use reap_strategy::InstrumentConfig;

#[derive(Debug, Clone)]
pub struct MatchingEngine {
    symbol: Symbol,
    instrument: InstrumentConfig,
    book: BookReducer,
    orders: OrderReducer,
    meta: HashMap<String, MatchingMeta>,
    next_order_id: u64,
    next_seq: u64,
    px_threshold: f64,
    depth_fill_conservative_threshold: f64,
    now_ms: u64,
}

impl MatchingEngine {
    pub fn new(instrument: InstrumentConfig) -> Self {
        Self::with_depth_fill_conservative_threshold(instrument, 0.0)
    }

    pub fn with_depth_fill_conservative_threshold(
        instrument: InstrumentConfig,
        depth_fill_conservative_threshold: f64,
    ) -> Self {
        Self {
            symbol: instrument.symbol.clone(),
            px_threshold: instrument.tick_size * 0.1,
            book: BookReducer::new(instrument.symbol.clone()),
            instrument,
            orders: OrderReducer::new(),
            meta: HashMap::new(),
            next_order_id: 1,
            next_seq: 1,
            depth_fill_conservative_threshold,
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
        self.book.apply_snapshot(depth);
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
        let mut qty_remaining = qty;
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
        let mut market_levels = self.book.levels(market_side).to_vec();
        let mut updates = Vec::new();

        for order_id in self.priority_order_ids(maker_side) {
            for level in &mut market_levels {
                if level.qty <= 0.0 {
                    continue;
                }
                let Some((order_price, open_qty)) = self
                    .orders
                    .get(&order_id)
                    .map(|order| (order.price, order.open_qty))
                else {
                    break;
                };
                if !market_side.crosses(level.px, order_price) {
                    break;
                }
                if let Some(meta) = self.meta.get_mut(&order_id) {
                    meta.qty_ahead = 0.0;
                }
                if !crosses_with_threshold(
                    market_side,
                    order_price,
                    level.px,
                    self.depth_fill_conservative_threshold,
                ) {
                    break;
                }

                let fill_qty = open_qty.min(level.qty);
                if fill_qty <= 0.0 {
                    break;
                }
                level.qty -= fill_qty;
                updates.extend(self.orders.fill(
                    &order_id,
                    self.now_ms,
                    fill_qty,
                    order_price,
                    FillLiquidity::Maker,
                ));
                if self
                    .orders
                    .get(&order_id)
                    .is_none_or(|order| order.open_qty <= 0.0)
                {
                    break;
                }
            }
        }
        updates
    }

    fn match_taker_order(&mut self, order_id: &str) -> Vec<OrderUpdate> {
        let Some(order) = self.orders.get(order_id).cloned() else {
            return Vec::new();
        };
        self.book
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
            .unwrap_or(0.0);
        if let Some(meta) = self.meta.get_mut(order_id) {
            meta.qty_ahead = qty_ahead;
        }
    }

    fn crosses_current_depth(&self, order: &NewOrder) -> bool {
        self.book
            .best(order.side.reverse())
            .is_some_and(|level| order.side.crosses(order.price, level.px))
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

fn crosses_with_threshold(
    taker_side: Side,
    maker_px: Price,
    taker_px: Price,
    threshold: f64,
) -> bool {
    match taker_side {
        Side::Buy => taker_px >= maker_px * (1.0 + threshold),
        Side::Sell => taker_px <= maker_px * (1.0 - threshold),
    }
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
}
