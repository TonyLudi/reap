mod client_id;
mod gateway;
mod pacing;
mod private;
mod reconcile;

pub use client_id::*;
pub use gateway::*;
pub use pacing::*;
pub use private::*;
pub use reconcile::*;

use std::collections::{HashMap, HashSet};

use serde::{Deserialize, Serialize};

use reap_core::{
    FillLiquidity, NewOrder, OrderEvent, OrderStatus, OrderUpdate, Price, Quantity, Side, Symbol,
    TimeInForce, TimeMs,
};

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct OrderSnapshot {
    pub order_id: String,
    pub symbol: Symbol,
    pub side: Side,
    pub qty: Quantity,
    pub price: Price,
    pub time_in_force: Option<TimeInForce>,
    pub status: OrderStatus,
    pub open_qty: Quantity,
    pub filled_qty: Quantity,
    pub avg_fill_price: Price,
    pub last_fill_qty: Quantity,
    pub last_fill_price: Price,
    pub last_fill_liquidity: Option<FillLiquidity>,
    pub reason: String,
}

impl OrderSnapshot {
    pub fn is_live(&self) -> bool {
        self.open_qty > 0.0
            && matches!(
                self.status,
                OrderStatus::Live | OrderStatus::PartiallyFilled
            )
    }

    pub fn is_terminal(&self) -> bool {
        matches!(
            self.status,
            OrderStatus::Filled | OrderStatus::Cancelled | OrderStatus::Rejected
        )
    }

    fn from_new(order_id: String, order: NewOrder) -> Self {
        Self {
            order_id,
            symbol: order.symbol,
            side: order.side,
            qty: order.qty,
            price: order.price,
            time_in_force: Some(order.time_in_force),
            status: OrderStatus::PendingNew,
            open_qty: order.qty,
            filled_qty: 0.0,
            avg_fill_price: 0.0,
            last_fill_qty: 0.0,
            last_fill_price: 0.0,
            last_fill_liquidity: None,
            reason: order.reason,
        }
    }

    fn from_update(update: &OrderUpdate) -> Self {
        Self {
            order_id: update.order_id.clone(),
            symbol: update.symbol.clone(),
            side: update.side,
            qty: update.qty,
            price: update.price,
            time_in_force: None,
            status: update.status,
            open_qty: update.open_qty,
            filled_qty: update.filled_qty,
            avg_fill_price: update.avg_fill_price,
            last_fill_qty: update.last_fill_qty,
            last_fill_price: update.last_fill_price,
            last_fill_liquidity: update.last_fill_liquidity,
            reason: update.reason.clone(),
        }
    }

    fn apply_update(&mut self, update: &OrderUpdate) {
        self.symbol = update.symbol.clone();
        self.side = update.side;
        self.qty = update.qty;
        self.price = update.price;
        self.status = update.status;
        self.open_qty = update.open_qty;
        self.filled_qty = update.filled_qty;
        self.avg_fill_price = update.avg_fill_price;
        self.last_fill_qty = update.last_fill_qty;
        self.last_fill_price = update.last_fill_price;
        self.last_fill_liquidity = update.last_fill_liquidity;
        self.reason = update.reason.clone();
    }

    fn to_update(&self, ts_ms: TimeMs, event: OrderEvent, reason: &str) -> OrderUpdate {
        OrderUpdate {
            ts_ms,
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
            reason: self.event_reason(reason),
        }
    }

    fn event_reason(&self, reason: &str) -> String {
        if reason == "fill" {
            self.reason.clone()
        } else if self.reason.is_empty() {
            reason.to_string()
        } else {
            format!("{}:{}", self.reason, reason)
        }
    }
}

#[derive(Debug, Clone, Default)]
pub struct OrderReducer {
    orders: HashMap<String, OrderSnapshot>,
    seen_updates: HashSet<UpdateKey>,
}

impl OrderReducer {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn contains_order(&self, order_id: &str) -> bool {
        self.orders.contains_key(order_id)
    }

    pub fn remove(&mut self, order_id: &str) -> Option<OrderSnapshot> {
        self.orders.remove(order_id)
    }

    pub fn get(&self, order_id: &str) -> Option<&OrderSnapshot> {
        self.orders.get(order_id)
    }

    pub fn orders(&self) -> impl Iterator<Item = (&str, &OrderSnapshot)> {
        self.orders
            .iter()
            .map(|(order_id, snapshot)| (order_id.as_str(), snapshot))
    }

    pub fn is_live(&self, order_id: &str) -> bool {
        self.orders
            .get(order_id)
            .is_some_and(OrderSnapshot::is_live)
    }

    pub fn create_pending(
        &mut self,
        order_id: impl Into<String>,
        order: NewOrder,
    ) -> &OrderSnapshot {
        let order_id = order_id.into();
        self.orders.insert(
            order_id.clone(),
            OrderSnapshot::from_new(order_id.clone(), order),
        );
        self.orders
            .get(&order_id)
            .expect("inserted order snapshot must exist")
    }

    pub fn pending_new(
        &mut self,
        order_id: impl Into<String>,
        order: NewOrder,
        ts_ms: TimeMs,
    ) -> OrderUpdate {
        let order_id = order_id.into();
        self.create_pending(order_id.clone(), order);
        let update = self
            .orders
            .get(&order_id)
            .expect("newly-created order must exist")
            .to_update(ts_ms, OrderEvent::PendingNew, "pending_new");
        self.record_update(&update);
        update
    }

    pub fn ack_new(
        &mut self,
        order_id: impl Into<String>,
        order: NewOrder,
        ts_ms: TimeMs,
    ) -> OrderUpdate {
        let order_id = order_id.into();
        self.create_pending(order_id.clone(), order);
        self.mark_live(&order_id, ts_ms, "new")
            .expect("newly-created order should be live-ackable")
    }

    pub fn cancel_new(
        &mut self,
        order_id: impl Into<String>,
        order: NewOrder,
        ts_ms: TimeMs,
        reason: &str,
    ) -> OrderUpdate {
        let order_id = order_id.into();
        self.create_pending(order_id.clone(), order);
        self.cancel(&order_id, ts_ms, reason)
            .expect("newly-created order should be cancellable")
    }

    pub fn mark_live(
        &mut self,
        order_id: &str,
        ts_ms: TimeMs,
        reason: &str,
    ) -> Option<OrderUpdate> {
        let snapshot = self.orders.get_mut(order_id)?;
        if snapshot.open_qty <= 0.0 || snapshot.is_terminal() {
            return None;
        }
        snapshot.status = OrderStatus::Live;
        let update = snapshot.to_update(ts_ms, OrderEvent::New, reason);
        self.record_update(&update);
        Some(update)
    }

    pub fn cancel(&mut self, order_id: &str, ts_ms: TimeMs, reason: &str) -> Option<OrderUpdate> {
        let snapshot = self.orders.get_mut(order_id)?;
        if snapshot.is_terminal() {
            return None;
        }
        snapshot.status = OrderStatus::Cancelled;
        let update = snapshot.to_update(ts_ms, OrderEvent::Cancelled, reason);
        self.record_update(&update);
        Some(update)
    }

    pub fn reject(&mut self, order_id: &str, ts_ms: TimeMs, reason: &str) -> Option<OrderUpdate> {
        let snapshot = self.orders.get_mut(order_id)?;
        if snapshot.is_terminal() {
            return None;
        }
        snapshot.status = OrderStatus::Rejected;
        let update = snapshot.to_update(ts_ms, OrderEvent::Rejected, reason);
        self.record_update(&update);
        Some(update)
    }

    pub fn fill(
        &mut self,
        order_id: &str,
        ts_ms: TimeMs,
        fill_qty: Quantity,
        fill_px: Price,
        liquidity: FillLiquidity,
    ) -> Option<OrderUpdate> {
        if fill_qty <= 0.0 {
            return None;
        }
        let snapshot = self.orders.get_mut(order_id)?;
        if snapshot.open_qty <= 0.0 || snapshot.is_terminal() {
            return None;
        }

        let qty = fill_qty.min(snapshot.open_qty);
        let prior_filled = snapshot.filled_qty;
        snapshot.filled_qty += qty;
        snapshot.open_qty = (snapshot.qty - snapshot.filled_qty).max(0.0);
        snapshot.avg_fill_price = if snapshot.filled_qty > 0.0 {
            (snapshot.avg_fill_price * prior_filled + fill_px * qty) / snapshot.filled_qty
        } else {
            0.0
        };
        snapshot.last_fill_qty = qty;
        snapshot.last_fill_price = fill_px;
        snapshot.last_fill_liquidity = Some(liquidity);
        snapshot.status = if snapshot.open_qty <= 0.0 {
            OrderStatus::Filled
        } else {
            OrderStatus::PartiallyFilled
        };
        let event = if snapshot.open_qty <= 0.0 {
            OrderEvent::FullyFilled
        } else {
            OrderEvent::PartialFill
        };
        let update = snapshot.to_update(ts_ms, event, "fill");
        self.record_update(&update);
        Some(update)
    }

    pub fn apply_update(&mut self, update: OrderUpdate) -> bool {
        let key = UpdateKey::from(&update);
        if !self.seen_updates.insert(key) {
            return false;
        }
        self.orders
            .entry(update.order_id.clone())
            .and_modify(|snapshot| snapshot.apply_update(&update))
            .or_insert_with(|| OrderSnapshot::from_update(&update));
        true
    }

    fn record_update(&mut self, update: &OrderUpdate) {
        self.seen_updates.insert(UpdateKey::from(update));
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Hash)]
struct UpdateKey {
    ts_ms: TimeMs,
    order_id: String,
    side: u8,
    event: u8,
    status: u8,
    price: u64,
    qty: u64,
    open_qty: u64,
    filled_qty: u64,
    avg_fill_price: u64,
    last_fill_qty: u64,
    last_fill_price: u64,
    last_fill_liquidity: u8,
    reason: String,
}

impl From<&OrderUpdate> for UpdateKey {
    fn from(update: &OrderUpdate) -> Self {
        Self {
            ts_ms: update.ts_ms,
            order_id: update.order_id.clone(),
            side: side_code(update.side),
            event: event_code(update.event),
            status: status_code(update.status),
            price: update.price.to_bits(),
            qty: update.qty.to_bits(),
            open_qty: update.open_qty.to_bits(),
            filled_qty: update.filled_qty.to_bits(),
            avg_fill_price: update.avg_fill_price.to_bits(),
            last_fill_qty: update.last_fill_qty.to_bits(),
            last_fill_price: update.last_fill_price.to_bits(),
            last_fill_liquidity: liquidity_code(update.last_fill_liquidity),
            reason: update.reason.clone(),
        }
    }
}

fn side_code(side: Side) -> u8 {
    match side {
        Side::Buy => 1,
        Side::Sell => 2,
    }
}

fn event_code(event: OrderEvent) -> u8 {
    match event {
        OrderEvent::PendingNew => 1,
        OrderEvent::New => 2,
        OrderEvent::PartialFill => 3,
        OrderEvent::FullyFilled => 4,
        OrderEvent::Cancelled => 5,
        OrderEvent::Rejected => 6,
    }
}

fn status_code(status: OrderStatus) -> u8 {
    match status {
        OrderStatus::PendingNew => 1,
        OrderStatus::Live => 2,
        OrderStatus::PartiallyFilled => 3,
        OrderStatus::Filled => 4,
        OrderStatus::Cancelled => 5,
        OrderStatus::Rejected => 6,
    }
}

fn liquidity_code(liquidity: Option<FillLiquidity>) -> u8 {
    match liquidity {
        None => 0,
        Some(FillLiquidity::Maker) => 1,
        Some(FillLiquidity::Taker) => 2,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn new_order() -> NewOrder {
        NewOrder {
            symbol: "BTC-USDT".to_string(),
            side: Side::Buy,
            qty: 1.0,
            price: 100.0,
            time_in_force: TimeInForce::Gtc,
            reduce_only: false,
            self_trade_prevention: None,
            reason: "quote".to_string(),
        }
    }

    #[test]
    fn duplicate_fill_update_is_idempotent() {
        let mut reducer = OrderReducer::new();
        let update = OrderUpdate {
            ts_ms: 10,
            order_id: "order-1".to_string(),
            symbol: "BTC-USDT".to_string(),
            side: Side::Buy,
            event: OrderEvent::PartialFill,
            status: OrderStatus::PartiallyFilled,
            price: 100.0,
            qty: 1.0,
            open_qty: 0.6,
            filled_qty: 0.4,
            avg_fill_price: 99.5,
            last_fill_qty: 0.4,
            last_fill_price: 99.5,
            last_fill_liquidity: Some(FillLiquidity::Maker),
            reason: "quote".to_string(),
        };

        assert!(reducer.apply_update(update.clone()));
        assert!(!reducer.apply_update(update));

        let snapshot = reducer.get("order-1").unwrap();
        assert_eq!(snapshot.filled_qty, 0.4);
        assert_eq!(snapshot.open_qty, 0.6);
    }

    #[test]
    fn generated_fills_are_canonical() {
        let mut reducer = OrderReducer::new();
        let update = reducer.ack_new("order-1", new_order(), 1);
        assert_eq!(update.event, OrderEvent::New);
        assert_eq!(update.reason, "quote:new");

        let first = reducer
            .fill("order-1", 2, 0.25, 99.0, FillLiquidity::Maker)
            .unwrap();
        let second = reducer
            .fill("order-1", 3, 0.75, 101.0, FillLiquidity::Maker)
            .unwrap();

        assert_eq!(first.event, OrderEvent::PartialFill);
        assert_eq!(second.event, OrderEvent::FullyFilled);
        assert_eq!(second.status, OrderStatus::Filled);
        assert_eq!(second.filled_qty, 1.0);
        assert_eq!(second.open_qty, 0.0);
        assert_eq!(second.avg_fill_price, 100.5);
    }

    #[test]
    fn cancel_is_terminal_and_idempotent() {
        let mut reducer = OrderReducer::new();
        reducer.ack_new("order-1", new_order(), 1);

        let cancel = reducer.cancel("order-1", 2, "replace_quote").unwrap();
        assert_eq!(cancel.event, OrderEvent::Cancelled);
        assert_eq!(cancel.status, OrderStatus::Cancelled);
        assert_eq!(cancel.reason, "quote:replace_quote");
        assert!(reducer.cancel("order-1", 3, "replace_quote").is_none());
        assert!(!reducer.is_live("order-1"));
    }
}
