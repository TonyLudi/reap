use std::collections::{HashMap, HashSet};

use reap_core::{AccountUpdate, Balance, NewOrder, OrderEvent, OrderStatus, OrderUpdate, Position};
use reap_venue::{PrivateOrderState, PrivateOrderUpdate, RemoteFill};

use crate::OrderReducer;

#[derive(Debug, Default)]
pub struct PrivateStateReducer {
    orders: OrderReducer,
    balances: HashMap<String, Balance>,
    positions: HashMap<String, Position>,
    exchange_to_client: HashMap<String, String>,
    seen_versions: HashSet<PrivateVersion>,
    seen_fill_ids: HashSet<String>,
    cumulative_fills: HashMap<String, f64>,
    last_order_update_ms: HashMap<String, u64>,
}

impl PrivateStateReducer {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn order_reducer(&self) -> &OrderReducer {
        &self.orders
    }

    pub fn order_reducer_mut(&mut self) -> &mut OrderReducer {
        &mut self.orders
    }

    pub fn balances(&self) -> &HashMap<String, Balance> {
        &self.balances
    }

    pub fn positions(&self) -> &HashMap<String, Position> {
        &self.positions
    }

    pub fn seen_fill_ids(&self) -> &HashSet<String> {
        &self.seen_fill_ids
    }

    pub fn canonical_order_id(&self, exchange_order_id: &str) -> Option<&str> {
        self.exchange_to_client
            .get(exchange_order_id)
            .map(String::as_str)
    }

    pub fn register_local_order(&mut self, client_order_id: impl Into<String>, order: NewOrder) {
        let client_order_id = client_order_id.into();
        if !self.orders.contains_order(&client_order_id) {
            self.orders.create_pending(client_order_id, order);
        }
    }

    pub fn remove_local_order(&mut self, client_order_id: &str) {
        self.orders.remove(client_order_id);
    }

    pub fn apply_account(&mut self, update: AccountUpdate) {
        for balance in update.balances {
            self.balances.insert(balance.currency.clone(), balance);
        }
        for position in update.positions {
            self.positions.insert(position.symbol.clone(), position);
        }
    }

    pub fn apply_order(&mut self, update: PrivateOrderUpdate) -> Option<OrderUpdate> {
        let order_id = if update.client_order_id.is_empty() {
            update.exchange_order_id.clone()
        } else {
            update.client_order_id.clone()
        };
        let prior = self.cumulative_fills.get(&order_id).copied().unwrap_or(0.0);
        let existing = self.orders.get(&order_id).cloned();
        let last_update_ms = self
            .last_order_update_ms
            .get(&order_id)
            .copied()
            .unwrap_or(0);
        if update.ts_ms < last_update_ms && update.cumulative_filled_qty <= prior {
            return None;
        }
        let incoming_terminal = matches!(
            update.state,
            PrivateOrderState::Filled | PrivateOrderState::Cancelled | PrivateOrderState::Rejected
        );
        if existing.as_ref().is_some_and(|order| order.is_terminal())
            && !incoming_terminal
            && update.cumulative_filled_qty <= prior
        {
            return None;
        }
        let version = PrivateVersion {
            order_id: order_id.clone(),
            ts_ms: update.ts_ms,
            state: update.state,
            cumulative_filled_qty: update.cumulative_filled_qty.to_bits(),
            last_fill_qty: update.last_fill_qty.to_bits(),
            fill_id: update.fill_id.clone(),
        };
        if !self.seen_versions.insert(version) {
            return None;
        }
        if !update.exchange_order_id.is_empty() {
            self.exchange_to_client
                .insert(update.exchange_order_id.clone(), order_id.clone());
        }
        self.last_order_update_ms
            .insert(order_id.clone(), update.ts_ms.max(last_update_ms));

        let cumulative = update.cumulative_filled_qty.max(prior);
        let fill_id_is_new = update
            .fill_id
            .as_ref()
            .is_some_and(|fill_id| self.seen_fill_ids.insert(fill_id.clone()));
        let inferred_fill = (cumulative - prior).max(0.0);
        let last_fill_qty = if fill_id_is_new || inferred_fill > 0.0 {
            update.last_fill_qty.max(inferred_fill)
        } else {
            0.0
        };
        self.cumulative_fills.insert(order_id.clone(), cumulative);

        let qty = if update.qty > 0.0 {
            update.qty
        } else {
            existing
                .as_ref()
                .map(|order| order.qty)
                .unwrap_or(cumulative)
        };
        let price = if update.price > 0.0 {
            update.price
        } else {
            existing.as_ref().map(|order| order.price).unwrap_or(0.0)
        };
        let mut status = canonical_status(update.state);
        if existing.as_ref().is_some_and(|order| order.is_terminal()) && !incoming_terminal {
            status = existing.as_ref().expect("checked existing order").status;
        }
        let event = if last_fill_qty > 0.0 {
            if status == OrderStatus::Filled {
                OrderEvent::FullyFilled
            } else {
                OrderEvent::PartialFill
            }
        } else {
            canonical_event(update.state)
        };
        let local_reason = existing
            .as_ref()
            .map(|order| order.reason.clone())
            .filter(|reason| !reason.is_empty());
        let canonical = OrderUpdate {
            ts_ms: update.ts_ms,
            order_id,
            symbol: update.symbol,
            side: update.side,
            event,
            status,
            price,
            qty,
            open_qty: (qty - cumulative).max(0.0),
            filled_qty: cumulative,
            avg_fill_price: if update.average_fill_price > 0.0 {
                update.average_fill_price
            } else if inferred_fill > 0.0 && update.last_fill_price > 0.0 && cumulative > 0.0 {
                (existing
                    .as_ref()
                    .map(|order| order.avg_fill_price * prior)
                    .unwrap_or(0.0)
                    + update.last_fill_price * inferred_fill)
                    / cumulative
            } else {
                existing
                    .as_ref()
                    .map(|order| order.avg_fill_price)
                    .unwrap_or(0.0)
            },
            last_fill_qty,
            last_fill_price: if last_fill_qty > 0.0 {
                update.last_fill_price
            } else {
                0.0
            },
            last_fill_liquidity: if last_fill_qty > 0.0 {
                update.liquidity
            } else {
                None
            },
            reason: match (local_reason, update.reject_reason.is_empty()) {
                (Some(reason), true) => reason,
                (Some(reason), false) => format!("{reason}:{}", update.reject_reason),
                (None, true) => "okx_private".to_string(),
                (None, false) => format!("okx_private:{}", update.reject_reason),
            },
        };
        self.orders
            .apply_update(canonical.clone())
            .then_some(canonical)
    }

    pub fn apply_fill(&mut self, fill: RemoteFill) -> Option<OrderUpdate> {
        let order_id = if fill.client_order_id.is_empty() {
            fill.exchange_order_id.clone()
        } else {
            fill.client_order_id.clone()
        };
        let existing = self.orders.get(&order_id)?.clone();
        if !self.seen_fill_ids.insert(fill.fill_id.clone()) {
            return None;
        }
        if !fill.exchange_order_id.is_empty() {
            self.exchange_to_client
                .insert(fill.exchange_order_id, order_id.clone());
        }
        let applied_qty = fill.qty.min(existing.open_qty);
        if applied_qty <= 0.0 {
            return None;
        }
        let cumulative = existing.filled_qty + applied_qty;
        let open_qty = (existing.qty - cumulative).max(0.0);
        let status = if open_qty == 0.0 {
            OrderStatus::Filled
        } else {
            OrderStatus::PartiallyFilled
        };
        let avg_fill_price =
            (existing.avg_fill_price * existing.filled_qty + fill.price * applied_qty) / cumulative;
        self.cumulative_fills.insert(order_id.clone(), cumulative);
        let canonical = OrderUpdate {
            ts_ms: fill.ts_ms,
            order_id,
            symbol: fill.symbol,
            side: fill.side,
            event: if status == OrderStatus::Filled {
                OrderEvent::FullyFilled
            } else {
                OrderEvent::PartialFill
            },
            status,
            price: existing.price,
            qty: existing.qty,
            open_qty,
            filled_qty: cumulative,
            avg_fill_price,
            last_fill_qty: applied_qty,
            last_fill_price: fill.price,
            last_fill_liquidity: Some(fill.liquidity),
            reason: existing.reason,
        };
        self.orders
            .apply_update(canonical.clone())
            .then_some(canonical)
    }
}

fn canonical_status(state: PrivateOrderState) -> OrderStatus {
    match state {
        PrivateOrderState::Pending => OrderStatus::PendingNew,
        PrivateOrderState::Live => OrderStatus::Live,
        PrivateOrderState::PartiallyFilled => OrderStatus::PartiallyFilled,
        PrivateOrderState::Filled => OrderStatus::Filled,
        PrivateOrderState::Cancelled => OrderStatus::Cancelled,
        PrivateOrderState::Rejected => OrderStatus::Rejected,
    }
}

fn canonical_event(state: PrivateOrderState) -> OrderEvent {
    match state {
        PrivateOrderState::Pending => OrderEvent::PendingNew,
        PrivateOrderState::Live => OrderEvent::New,
        PrivateOrderState::PartiallyFilled => OrderEvent::PartialFill,
        PrivateOrderState::Filled => OrderEvent::FullyFilled,
        PrivateOrderState::Cancelled => OrderEvent::Cancelled,
        PrivateOrderState::Rejected => OrderEvent::Rejected,
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Hash)]
struct PrivateVersion {
    order_id: String,
    ts_ms: u64,
    state: PrivateOrderState,
    cumulative_filled_qty: u64,
    last_fill_qty: u64,
    fill_id: Option<String>,
}

#[cfg(test)]
mod tests {
    use reap_core::{FillLiquidity, Side};

    use super::*;

    fn private_fill() -> PrivateOrderUpdate {
        PrivateOrderUpdate {
            ts_ms: 10,
            exchange_order_id: "exchange-1".to_string(),
            client_order_id: "client-1".to_string(),
            symbol: "BTC-USDT".to_string(),
            side: Side::Buy,
            state: PrivateOrderState::PartiallyFilled,
            price: 100.0,
            qty: 1.0,
            cumulative_filled_qty: 0.4,
            average_fill_price: 99.5,
            last_fill_qty: 0.4,
            last_fill_price: 99.5,
            liquidity: Some(FillLiquidity::Maker),
            fill_id: Some("fill-1".to_string()),
            reject_reason: String::new(),
        }
    }

    #[test]
    fn duplicate_private_fill_is_idempotent() {
        let mut reducer = PrivateStateReducer::new();
        let first = reducer.apply_order(private_fill()).unwrap();
        let second = reducer.apply_order(private_fill());

        assert_eq!(first.last_fill_qty, 0.4);
        assert!(second.is_none());
        assert_eq!(
            reducer.order_reducer().get("client-1").unwrap().filled_qty,
            0.4
        );
        assert_eq!(reducer.canonical_order_id("exchange-1"), Some("client-1"));
    }

    #[test]
    fn private_updates_preserve_registered_strategy_reason() {
        let mut reducer = PrivateStateReducer::new();
        reducer.register_local_order(
            "client-1",
            NewOrder {
                symbol: "BTC-USDT".to_string(),
                side: Side::Buy,
                qty: 1.0,
                price: 100.0,
                time_in_force: reap_core::TimeInForce::PostOnly,
                reduce_only: false,
                self_trade_prevention: None,
                reason: "quote:1".to_string(),
            },
        );

        let update = reducer.apply_order(private_fill()).unwrap();

        assert_eq!(update.reason, "quote:1");
        assert_eq!(
            reducer.order_reducer().get("client-1").unwrap().reason,
            "quote:1"
        );
    }

    #[test]
    fn account_updates_replace_currency_and_position_state() {
        let mut reducer = PrivateStateReducer::new();
        reducer.apply_account(AccountUpdate {
            ts_ms: 1,
            balances: vec![Balance {
                account_id: None,
                currency: "USDT".to_string(),
                total: 100.0,
                available: 90.0,
                equity: 100.0,
                liability: 0.0,
                max_loan: 0.0,
            }],
            positions: vec![Position {
                symbol: "BTC-USDT-SWAP".to_string(),
                qty: -2.0,
                avg_price: 100.0,
            }],
            margins: Vec::new(),
        });

        assert_eq!(reducer.balances()["USDT"].available, 90.0);
        assert_eq!(reducer.positions()["BTC-USDT-SWAP"].qty, -2.0);
    }

    #[test]
    fn explicit_fill_channel_updates_known_order_once() {
        let mut reducer = PrivateStateReducer::new();
        reducer.order_reducer_mut().ack_new(
            "client-1",
            reap_core::NewOrder {
                symbol: "BTC-USDT".to_string(),
                side: Side::Buy,
                qty: 1.0,
                price: 100.0,
                time_in_force: reap_core::TimeInForce::Gtc,
                reduce_only: false,
                self_trade_prevention: None,
                reason: "test".to_string(),
            },
            1,
        );
        let fill = RemoteFill {
            fill_id: "fill-1".to_string(),
            exchange_order_id: "exchange-1".to_string(),
            client_order_id: "client-1".to_string(),
            symbol: "BTC-USDT".to_string(),
            side: Side::Buy,
            price: 99.5,
            qty: 0.4,
            liquidity: FillLiquidity::Maker,
            ts_ms: 2,
        };

        assert_eq!(reducer.apply_fill(fill.clone()).unwrap().filled_qty, 0.4);
        assert!(reducer.apply_fill(fill).is_none());
    }

    #[test]
    fn late_live_update_does_not_reopen_terminal_order() {
        let mut reducer = PrivateStateReducer::new();
        let mut terminal = private_fill();
        terminal.state = PrivateOrderState::Filled;
        terminal.cumulative_filled_qty = 1.0;
        terminal.last_fill_qty = 1.0;
        terminal.ts_ms = 20;
        reducer.apply_order(terminal).unwrap();

        let mut late = private_fill();
        late.state = PrivateOrderState::Live;
        late.cumulative_filled_qty = 0.0;
        late.last_fill_qty = 0.0;
        late.fill_id = None;
        late.ts_ms = 10;
        assert!(reducer.apply_order(late).is_none());
        assert_eq!(
            reducer.order_reducer().get("client-1").unwrap().status,
            OrderStatus::Filled
        );
    }
}
