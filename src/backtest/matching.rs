use std::collections::HashMap;

use crate::strategy::InstrumentConfig;
use crate::types::{
    FillLiquidity, Level, NewOrder, OrderBook, OrderEvent, OrderStatus, OrderUpdate, Price,
    Quantity, Side, Symbol, TimeInForce, round_down_to_lot,
};

#[derive(Debug, Clone)]
pub struct MatchingEngine {
    symbol: Symbol,
    instrument: InstrumentConfig,
    depth: Option<OrderBook>,
    orders: HashMap<String, SimOrder>,
    next_order_id: u64,
    next_seq: u64,
    px_threshold: f64,
}

impl MatchingEngine {
    pub fn new(instrument: InstrumentConfig) -> Self {
        Self {
            symbol: instrument.symbol.clone(),
            px_threshold: instrument.tick_size * 0.1,
            instrument,
            depth: None,
            orders: HashMap::new(),
            next_order_id: 1,
            next_seq: 1,
        }
    }

    pub fn depth(&self) -> Option<&OrderBook> {
        self.depth.as_ref()
    }

    pub fn contains_order(&self, order_id: &str) -> bool {
        self.orders.contains_key(order_id)
    }

    pub fn submit(&mut self, order: NewOrder) -> Vec<OrderUpdate> {
        let order_id = format!("{}-{}", self.symbol, self.next_order_id);
        self.next_order_id += 1;
        let seq = self.next_seq;
        self.next_seq += 1;

        let mut order = SimOrder::new(order_id, order, seq, self.instrument.lot_size);
        let mut updates = Vec::new();

        if self.depth.is_none() {
            if order.time_in_force == TimeInForce::Ioc {
                order.status = OrderStatus::Cancelled;
                updates.push(order.update(OrderEvent::Cancelled, "no_depth"));
            } else {
                self.add_qty_ahead(&mut order);
                order.status = OrderStatus::Live;
                updates.push(order.update(OrderEvent::New, "new"));
                self.orders.insert(order.order_id.clone(), order);
            }
            return updates;
        }

        let crosses = self.crosses_current_depth(&order);
        if order.time_in_force == TimeInForce::PostOnly && crosses {
            order.status = OrderStatus::Cancelled;
            updates.push(order.update(OrderEvent::Cancelled, "post_only_cross"));
            return updates;
        }
        if order.time_in_force == TimeInForce::Ioc && !crosses {
            order.status = OrderStatus::Cancelled;
            updates.push(order.update(OrderEvent::Cancelled, "ioc_miss"));
            return updates;
        }

        if !crosses {
            self.add_qty_ahead(&mut order);
            order.status = OrderStatus::Live;
            updates.push(order.update(OrderEvent::New, "new"));
            self.orders.insert(order.order_id.clone(), order);
            return updates;
        }

        updates.extend(self.match_taker_order(&mut order));
        if order.open_qty > 0.0 && order.time_in_force == TimeInForce::Ioc {
            order.status = OrderStatus::Cancelled;
            updates.push(order.update(OrderEvent::Cancelled, "ioc_remainder"));
        } else if order.open_qty > 0.0 {
            self.add_qty_ahead(&mut order);
            order.status = OrderStatus::Live;
            updates.push(order.update(OrderEvent::New, "resting_remainder"));
            self.orders.insert(order.order_id.clone(), order);
        }
        updates
    }

    pub fn cancel(&mut self, order_id: &str, reason: &str) -> Vec<OrderUpdate> {
        let Some(mut order) = self.orders.remove(order_id) else {
            return Vec::new();
        };
        order.status = OrderStatus::Cancelled;
        vec![order.update(OrderEvent::Cancelled, reason)]
    }

    pub fn on_depth(&mut self, depth: OrderBook) -> Vec<OrderUpdate> {
        self.depth = Some(depth);
        self.match_live_orders_on_depth()
    }

    pub fn on_trade(&mut self, price: Price, qty: Quantity, taker_side: Side) -> Vec<OrderUpdate> {
        let maker_side = taker_side.reverse();
        let mut qty_remaining = qty;
        let order_ids = self.priority_order_ids(maker_side);
        let mut updates = Vec::new();

        for order_id in order_ids {
            if qty_remaining <= 0.0 {
                break;
            }
            let Some(order) = self.orders.get_mut(&order_id) else {
                continue;
            };
            if !taker_side.crosses(price, order.price) {
                break;
            }

            if order.qty_ahead > 0.0 {
                if approx_eq(price, order.price, self.px_threshold) {
                    let consumed = qty_remaining.min(order.qty_ahead);
                    order.qty_ahead -= consumed;
                    qty_remaining -= consumed;
                    if qty_remaining <= 0.0 {
                        break;
                    }
                } else {
                    order.qty_ahead = 0.0;
                }
            }

            let fill_qty = order.open_qty.min(qty_remaining);
            qty_remaining -= fill_qty;
            let update = order.fill(fill_qty, order.price, FillLiquidity::Maker);
            updates.push(update);
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
        let Some(depth) = self.depth.clone() else {
            return Vec::new();
        };
        let market_side = maker_side.reverse();
        let mut market_levels = depth.levels(market_side).to_vec();
        let mut updates = Vec::new();

        for order_id in self.priority_order_ids(maker_side) {
            let Some(order) = self.orders.get_mut(&order_id) else {
                continue;
            };
            for level in &mut market_levels {
                if level.qty <= 0.0 {
                    continue;
                }
                if !market_side.crosses(level.px, order.price) {
                    break;
                }
                order.qty_ahead = 0.0;
                let fill_qty = order.open_qty.min(level.qty);
                level.qty -= fill_qty;
                updates.push(order.fill(fill_qty, order.price, FillLiquidity::Maker));
                if order.open_qty <= 0.0 {
                    break;
                }
            }
        }
        updates
    }

    fn match_taker_order(&mut self, order: &mut SimOrder) -> Vec<OrderUpdate> {
        let Some(depth) = self.depth.as_mut() else {
            return Vec::new();
        };
        let levels = depth.levels_mut(order.side.reverse());
        let mut updates = Vec::new();

        for level in levels.iter_mut() {
            if order.open_qty <= 0.0 {
                break;
            }
            if level.qty <= 0.0 || !order.side.crosses(order.price, level.px) {
                break;
            }
            let fill_qty = order.open_qty.min(level.qty);
            level.qty -= fill_qty;
            updates.push(order.fill(fill_qty, level.px, FillLiquidity::Taker));
        }
        levels.retain(|level| level.qty > 0.0);
        updates
    }

    fn add_qty_ahead(&self, order: &mut SimOrder) {
        let Some(depth) = &self.depth else {
            return;
        };
        for level in depth.levels(order.side) {
            if order.side.is_more_passive(level.px, order.price) {
                return;
            }
            if approx_eq(level.px, order.price, self.px_threshold) {
                order.qty_ahead = level.qty;
                return;
            }
        }
    }

    fn crosses_current_depth(&self, order: &SimOrder) -> bool {
        let Some(depth) = &self.depth else {
            return false;
        };
        let best = match order.side {
            Side::Buy => depth.best_ask(),
            Side::Sell => depth.best_bid(),
        };
        best.is_some_and(|level| order.side.crosses(order.price, level.px))
    }

    fn priority_order_ids(&self, side: Side) -> Vec<String> {
        let mut orders = self
            .orders
            .values()
            .filter(|order| order.side == side && order.status == OrderStatus::Live)
            .collect::<Vec<_>>();
        orders.sort_by(|a, b| match side {
            Side::Buy => b.price.total_cmp(&a.price).then(a.seq.cmp(&b.seq)),
            Side::Sell => a.price.total_cmp(&b.price).then(a.seq.cmp(&b.seq)),
        });
        orders
            .into_iter()
            .map(|order| order.order_id.clone())
            .collect()
    }

    fn remove_terminal_orders(&mut self) {
        self.orders
            .retain(|_, order| order.status == OrderStatus::Live && order.open_qty > 0.0);
    }
}

#[derive(Debug, Clone)]
struct SimOrder {
    order_id: String,
    symbol: Symbol,
    side: Side,
    qty: Quantity,
    price: Price,
    time_in_force: TimeInForce,
    reason: String,
    open_qty: Quantity,
    filled_qty: Quantity,
    avg_fill_price: Price,
    last_fill_qty: Quantity,
    last_fill_price: Price,
    last_fill_liquidity: Option<FillLiquidity>,
    status: OrderStatus,
    seq: u64,
    qty_ahead: Quantity,
}

impl SimOrder {
    fn new(order_id: String, order: NewOrder, seq: u64, lot_size: f64) -> Self {
        let qty = round_down_to_lot(order.qty, lot_size);
        Self {
            order_id,
            symbol: order.symbol,
            side: order.side,
            qty,
            price: order.price,
            time_in_force: order.time_in_force,
            reason: order.reason,
            open_qty: qty,
            filled_qty: 0.0,
            avg_fill_price: 0.0,
            last_fill_qty: 0.0,
            last_fill_price: 0.0,
            last_fill_liquidity: None,
            status: OrderStatus::PendingNew,
            seq,
            qty_ahead: 0.0,
        }
    }

    fn fill(&mut self, qty: Quantity, price: Price, liquidity: FillLiquidity) -> OrderUpdate {
        if qty <= 0.0 {
            return self.update(OrderEvent::New, "zero_fill");
        }
        let prior_filled = self.filled_qty;
        self.filled_qty += qty;
        self.open_qty = (self.qty - self.filled_qty).max(0.0);
        self.avg_fill_price = if self.filled_qty > 0.0 {
            (self.avg_fill_price * prior_filled + price * qty) / self.filled_qty
        } else {
            0.0
        };
        self.last_fill_qty = qty;
        self.last_fill_price = price;
        self.last_fill_liquidity = Some(liquidity);
        self.status = if self.open_qty <= 0.0 {
            OrderStatus::Filled
        } else {
            OrderStatus::PartiallyFilled
        };
        let event = if self.open_qty <= 0.0 {
            OrderEvent::FullyFilled
        } else {
            OrderEvent::PartialFill
        };
        self.update(event, "fill")
    }

    fn update(&self, event: OrderEvent, reason: &str) -> OrderUpdate {
        OrderUpdate {
            order_id: self.order_id.clone(),
            symbol: self.symbol.clone(),
            side: self.side,
            event,
            status: self.status,
            price: self.price,
            qty: self.qty,
            open_qty: self.open_qty,
            filled_qty: self.filled_qty,
            avg_fill_price: self.avg_fill_price,
            last_fill_qty: self.last_fill_qty,
            last_fill_price: self.last_fill_price,
            last_fill_liquidity: self.last_fill_liquidity,
            reason: if reason == "fill" {
                self.reason.clone()
            } else {
                format!("{}:{}", self.reason, reason)
            },
        }
    }
}

fn approx_eq(a: f64, b: f64, threshold: f64) -> bool {
    (a - b).abs() <= threshold
}

#[allow(dead_code)]
fn level(px: Price, qty: Quantity) -> Level {
    Level { px, qty }
}

#[cfg(test)]
mod tests {
    use crate::strategy::InstrumentKindConfig;

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
            reason: "quote".to_string(),
        });
        assert_eq!(updates[0].event, OrderEvent::Cancelled);
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
            reason: "hedge".to_string(),
        });
        assert_eq!(updates[0].event, OrderEvent::FullyFilled);
        assert_eq!(updates[0].last_fill_liquidity, Some(FillLiquidity::Taker));
    }
}
