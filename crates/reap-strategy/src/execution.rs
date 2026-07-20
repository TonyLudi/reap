use std::sync::Arc;

use reap_core::{
    NewOrder, OrderIntent, Price, Quantity, SelfTradePrevention, Side, Symbol, TimeInForce,
};

/// The three regular-order purposes emitted by the current Chaos strategy.
///
/// This purpose is intentionally not serialized. Serialized [`OrderIntent`] values are legacy
/// evidence/backtest records and do not acquire trusted live authority by resembling one of these
/// profiles.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ChaosExecutionPurpose {
    Quote,
    Hedge,
    CancelOwned,
}

impl ChaosExecutionPurpose {
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::Quote => "quote",
            Self::Hedge => "hedge",
            Self::CancelOwned => "cancel_owned",
        }
    }
}

/// A strategy-created Chaos regular-order intent.
///
/// Payload constructors and fields are private so callers cannot promote a deserialized or
/// free-form [`OrderIntent`] into trusted live output. The live execution policy must still check
/// account, symbol, exchange limits, and canonical ownership before transport.
#[derive(Debug)]
pub enum ChaosExecutionIntent {
    Quote(ChaosQuote),
    Hedge(ChaosHedge),
    CancelOwned(ChaosCancelOwned),
}

impl ChaosExecutionIntent {
    pub const fn purpose(&self) -> ChaosExecutionPurpose {
        match self {
            Self::Quote(_) => ChaosExecutionPurpose::Quote,
            Self::Hedge(_) => ChaosExecutionPurpose::Hedge,
            Self::CancelOwned(_) => ChaosExecutionPurpose::CancelOwned,
        }
    }

    pub const fn as_quote(&self) -> Option<&ChaosQuote> {
        match self {
            Self::Quote(quote) => Some(quote),
            Self::Hedge(_) | Self::CancelOwned(_) => None,
        }
    }

    pub const fn as_hedge(&self) -> Option<&ChaosHedge> {
        match self {
            Self::Hedge(hedge) => Some(hedge),
            Self::Quote(_) | Self::CancelOwned(_) => None,
        }
    }

    pub const fn as_cancel_owned(&self) -> Option<&ChaosCancelOwned> {
        match self {
            Self::CancelOwned(cancel) => Some(cancel),
            Self::Quote(_) | Self::Hedge(_) => None,
        }
    }

    pub(crate) fn take_hedge_commit(&mut self) -> Option<ChaosHedgeCommit> {
        match self {
            Self::Hedge(hedge) => hedge.take_commit(),
            Self::Quote(_) | Self::CancelOwned(_) => None,
        }
    }

    /// Lowers trusted typed output to the unchanged journal/backtest record.
    ///
    /// This conversion is deliberately one-way: there is no `From<OrderIntent>` implementation.
    pub fn to_order_intent(&self) -> OrderIntent {
        match self {
            Self::Quote(quote) => quote.to_order_intent(),
            Self::Hedge(hedge) => hedge.to_order_intent(),
            Self::CancelOwned(cancel) => cancel.to_order_intent(),
        }
    }

    /// Consumes trusted typed output and lowers it to the unchanged journal/backtest record.
    pub fn into_order_intent(self) -> OrderIntent {
        match self {
            Self::Quote(quote) => quote.into_order_intent(),
            Self::Hedge(hedge) => hedge.into_order_intent(),
            Self::CancelOwned(cancel) => cancel.into_order_intent(),
        }
    }

    pub(crate) fn quote(
        symbol: Symbol,
        side: Side,
        qty: Quantity,
        price: Price,
        reason: String,
    ) -> Self {
        Self::Quote(ChaosQuote {
            symbol,
            side,
            qty,
            price,
            reason,
        })
    }

    pub(crate) fn hedge(
        symbol: Symbol,
        side: Side,
        qty: Quantity,
        price: Price,
        reason: String,
        commit: ChaosHedgeCommit,
    ) -> Self {
        debug_assert_eq!(symbol, commit.symbol.as_ref());
        debug_assert_eq!(side, commit.side);
        Self::Hedge(ChaosHedge {
            symbol: commit.symbol,
            side,
            qty,
            price,
            reason: reason.into_boxed_str(),
            transition: ChaosHedgeTransition {
                price: commit.price,
                qty: commit.qty,
            },
            transition_pending: true,
        })
    }

    pub(crate) fn cancel_owned(order_id: String, reason: String) -> Self {
        Self::CancelOwned(ChaosCancelOwned { order_id, reason })
    }
}

/// The venue-neutral fields of a Chaos quote purpose.
#[derive(Debug)]
pub struct ChaosQuote {
    symbol: Symbol,
    side: Side,
    qty: Quantity,
    price: Price,
    reason: String,
}

impl ChaosQuote {
    pub fn symbol(&self) -> &str {
        &self.symbol
    }

    pub const fn side(&self) -> Side {
        self.side
    }

    pub const fn qty(&self) -> Quantity {
        self.qty
    }

    pub const fn price(&self) -> Price {
        self.price
    }

    pub fn reason(&self) -> &str {
        &self.reason
    }

    fn to_order_intent(&self) -> OrderIntent {
        OrderIntent::NewOrder(NewOrder {
            symbol: self.symbol.clone(),
            side: self.side,
            qty: self.qty,
            price: self.price,
            time_in_force: TimeInForce::PostOnly,
            reduce_only: false,
            self_trade_prevention: None,
            reason: self.reason.clone(),
        })
    }

    fn into_order_intent(self) -> OrderIntent {
        OrderIntent::NewOrder(NewOrder {
            symbol: self.symbol,
            side: self.side,
            qty: self.qty,
            price: self.price,
            time_in_force: TimeInForce::PostOnly,
            reduce_only: false,
            self_trade_prevention: None,
            reason: self.reason,
        })
    }
}

/// The venue-neutral fields of a Chaos hedge purpose.
#[derive(Debug)]
pub struct ChaosHedge {
    symbol: Arc<str>,
    side: Side,
    qty: Quantity,
    price: Price,
    reason: Box<str>,
    transition: ChaosHedgeTransition,
    transition_pending: bool,
}

impl ChaosHedge {
    pub fn symbol(&self) -> &str {
        &self.symbol
    }

    pub const fn side(&self) -> Side {
        self.side
    }

    pub const fn qty(&self) -> Quantity {
        self.qty
    }

    pub const fn price(&self) -> Price {
        self.price
    }

    pub fn reason(&self) -> &str {
        &self.reason
    }

    fn take_commit(&mut self) -> Option<ChaosHedgeCommit> {
        if !self.transition_pending {
            return None;
        }
        self.transition_pending = false;
        Some(ChaosHedgeCommit {
            symbol: Arc::clone(&self.symbol),
            side: self.side,
            price: self.transition.price,
            qty: self.transition.qty,
        })
    }

    fn to_order_intent(&self) -> OrderIntent {
        OrderIntent::NewOrder(NewOrder {
            symbol: self.symbol.to_string(),
            side: self.side,
            qty: self.qty,
            price: self.price,
            time_in_force: TimeInForce::Ioc,
            reduce_only: false,
            self_trade_prevention: Some(SelfTradePrevention::CancelMaker),
            reason: self.reason.to_string(),
        })
    }

    fn into_order_intent(self) -> OrderIntent {
        OrderIntent::NewOrder(NewOrder {
            symbol: self.symbol.to_string(),
            side: self.side,
            qty: self.qty,
            price: self.price,
            time_in_force: TimeInForce::Ioc,
            reduce_only: false,
            self_trade_prevention: Some(SelfTradePrevention::CancelMaker),
            reason: self.reason.into_string(),
        })
    }
}

#[derive(Debug)]
struct ChaosHedgeTransition {
    price: Price,
    qty: Quantity,
}

/// Opaque proof that a strategy-created hedge carries a deferred
/// implied-depth state transition.
///
/// It remains crate-private and can be consumed only by the strategy after the
/// genuine [`ChaosExecutionIntent::Hedge`] reaches the local-send boundary.
#[derive(Debug)]
pub(crate) struct ChaosHedgeCommit {
    symbol: Arc<str>,
    side: Side,
    price: Price,
    qty: Quantity,
}

impl ChaosHedgeCommit {
    pub(crate) fn new(symbol: Arc<str>, side: Side, price: Price, qty: Quantity) -> Self {
        Self {
            symbol,
            side,
            price,
            qty,
        }
    }

    pub(crate) fn into_parts(self) -> (Arc<str>, Side, Price, Quantity) {
        (self.symbol, self.side, self.price, self.qty)
    }
}

/// A request to cancel one regular order after canonical ownership is proven by live policy.
#[derive(Debug)]
pub struct ChaosCancelOwned {
    order_id: String,
    reason: String,
}

impl ChaosCancelOwned {
    pub fn order_id(&self) -> &str {
        &self.order_id
    }

    pub fn reason(&self) -> &str {
        &self.reason
    }

    fn to_order_intent(&self) -> OrderIntent {
        OrderIntent::CancelOrder {
            order_id: self.order_id.clone(),
            reason: self.reason.clone(),
        }
    }

    fn into_order_intent(self) -> OrderIntent {
        OrderIntent::CancelOrder {
            order_id: self.order_id,
            reason: self.reason,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn private_hedge_transition_does_not_enlarge_the_typed_intent() {
        assert_eq!(std::mem::size_of::<ChaosExecutionIntent>(), 80);
    }

    #[test]
    fn typed_purposes_lower_to_exact_legacy_profiles() {
        let quote = ChaosExecutionIntent::quote(
            "BTC-USDT".to_string(),
            Side::Buy,
            1.25,
            50_000.5,
            "quote:2".to_string(),
        );
        let hedge = ChaosExecutionIntent::hedge(
            "BTC-USDT-SWAP".to_string(),
            Side::Sell,
            125.0,
            49_999.5,
            "hedge:BTC-USDT:50000".to_string(),
            ChaosHedgeCommit::new(
                Arc::<str>::from("BTC-USDT-SWAP"),
                Side::Sell,
                50_000.0,
                125.0,
            ),
        );
        let cancel = ChaosExecutionIntent::cancel_owned(
            "canonical-order-id".to_string(),
            "replace_quote".to_string(),
        );

        assert_eq!(quote.purpose(), ChaosExecutionPurpose::Quote);
        assert_eq!(hedge.purpose(), ChaosExecutionPurpose::Hedge);
        assert_eq!(cancel.purpose(), ChaosExecutionPurpose::CancelOwned);
        assert_eq!(
            serde_json::to_value(quote.into_order_intent()).unwrap(),
            serde_json::json!({
                "NewOrder": {
                    "symbol": "BTC-USDT",
                    "side": "buy",
                    "qty": 1.25,
                    "price": 50_000.5,
                    "time_in_force": "post_only",
                    "reduce_only": false,
                    "self_trade_prevention": null,
                    "reason": "quote:2"
                }
            })
        );
        assert_eq!(
            serde_json::to_value(hedge.into_order_intent()).unwrap(),
            serde_json::json!({
                "NewOrder": {
                    "symbol": "BTC-USDT-SWAP",
                    "side": "sell",
                    "qty": 125.0,
                    "price": 49_999.5,
                    "time_in_force": "ioc",
                    "reduce_only": false,
                    "self_trade_prevention": "cancel_maker",
                    "reason": "hedge:BTC-USDT:50000"
                }
            })
        );
        assert_eq!(
            serde_json::to_value(cancel.into_order_intent()).unwrap(),
            serde_json::json!({
                "CancelOrder": {
                    "order_id": "canonical-order-id",
                    "reason": "replace_quote"
                }
            })
        );
    }
}
