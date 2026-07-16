use std::collections::{BTreeMap, HashMap};

use reap_core::{
    NewOrder, OrderStatus, OrderUpdate, SelfTradePrevention, Side, TimeInForce, TimeMs,
};
use reap_order::PrivateStateReducer;
use reap_risk::{InstrumentOrderLimits, InstrumentRiskModel};
use reap_strategy::{ChaosExecutionIntent, RiskGroupKindConfig};
use thiserror::Error;

use crate::{LiveConfig, VerifiedBootstrap};

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum OwnedRegularOrderOrigin {
    Quote,
    Hedge,
    RecoveredSubmitRequest,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct OwnedRegularOrder {
    account_id: String,
    symbol: String,
    client_order_id: String,
    exchange_order_id: Option<String>,
    origin: OwnedRegularOrderOrigin,
}

impl OwnedRegularOrder {
    pub(crate) fn account_id(&self) -> &str {
        &self.account_id
    }

    pub(crate) fn symbol(&self) -> &str {
        &self.symbol
    }

    pub(crate) fn client_order_id(&self) -> &str {
        &self.client_order_id
    }
}

/// Ownership proof is deliberately separate from canonical observation.
///
/// Entries can be added only after policy-approved local submission or from a
/// qualifying durable regular-submit request recovered by storage. Private
/// streams, reconciliation, prefixes, and free-form reasons never add entries.
#[derive(Debug, Default)]
pub(crate) struct OwnedRegularOrders {
    by_client_order_id: BTreeMap<String, OwnedRegularOrder>,
}

impl OwnedRegularOrders {
    /// Atomically reserves a vacant canonical identity and establishes local
    /// ownership. A proof conflict rolls back the just-created pending order.
    pub(crate) fn reserve_local(
        &mut self,
        approved: &ApprovedRegularSubmit,
        client_order_id: &str,
        private_state: &mut PrivateStateReducer,
        ts_ms: TimeMs,
    ) -> Result<OrderUpdate, RegularExecutionPolicyError> {
        let pending = private_state
            .register_local_order_at(client_order_id, approved.order.clone(), ts_ms)
            .ok_or_else(|| RegularExecutionPolicyError::CanonicalOrderConflict {
                client_order_id: client_order_id.to_string(),
            })?;
        if self.by_client_order_id.contains_key(client_order_id) {
            private_state.remove_local_order(client_order_id);
            return Err(RegularExecutionPolicyError::OwnershipConflict {
                client_order_id: client_order_id.to_string(),
            });
        }
        if let Err(error) = self.insert(OwnedRegularOrder {
            account_id: approved.account_id.clone(),
            symbol: approved.order.symbol.clone(),
            client_order_id: client_order_id.to_string(),
            exchange_order_id: None,
            origin: approved.origin,
        }) {
            private_state.remove_local_order(client_order_id);
            return Err(error);
        }
        Ok(pending)
    }

    pub(crate) fn register_recovered(
        &mut self,
        account_id: &str,
        symbol: &str,
        client_order_id: &str,
        exchange_order_id: Option<&str>,
    ) -> Result<(), RegularExecutionPolicyError> {
        self.insert(OwnedRegularOrder {
            account_id: account_id.to_string(),
            symbol: symbol.to_string(),
            client_order_id: client_order_id.to_string(),
            exchange_order_id: exchange_order_id.map(str::to_string),
            origin: OwnedRegularOrderOrigin::RecoveredSubmitRequest,
        })
    }

    fn insert(&mut self, order: OwnedRegularOrder) -> Result<(), RegularExecutionPolicyError> {
        if order.account_id.trim().is_empty()
            || order.symbol.trim().is_empty()
            || order.client_order_id.trim().is_empty()
            || order.client_order_id == "0"
            || order
                .exchange_order_id
                .as_deref()
                .is_some_and(|exchange_order_id| {
                    exchange_order_id.trim().is_empty() || exchange_order_id == "0"
                })
        {
            return Err(RegularExecutionPolicyError::InvalidOwnedIdentity);
        }
        if let Some(existing) = self.by_client_order_id.get(&order.client_order_id) {
            if existing == &order {
                return Ok(());
            }
            return Err(RegularExecutionPolicyError::OwnershipConflict {
                client_order_id: order.client_order_id,
            });
        }
        self.by_client_order_id
            .insert(order.client_order_id.clone(), order);
        Ok(())
    }

    pub(crate) fn get(&self, client_order_id: &str) -> Option<&OwnedRegularOrder> {
        self.by_client_order_id.get(client_order_id)
    }

    pub(crate) fn bind_exchange_order_id(
        &mut self,
        account_id: &str,
        client_order_id: &str,
        exchange_order_id: &str,
    ) -> Result<(), RegularExecutionPolicyError> {
        if exchange_order_id.trim().is_empty() || exchange_order_id == "0" {
            return Err(RegularExecutionPolicyError::InvalidOwnedIdentity);
        }
        let owned = self
            .by_client_order_id
            .get_mut(client_order_id)
            .ok_or_else(|| RegularExecutionPolicyError::UnknownOwnedOrder {
                client_order_id: client_order_id.to_string(),
            })?;
        if owned.account_id != account_id {
            return Err(RegularExecutionPolicyError::OwnershipConflict {
                client_order_id: client_order_id.to_string(),
            });
        }
        match owned.exchange_order_id.as_deref() {
            Some(existing) if existing != exchange_order_id => {
                Err(RegularExecutionPolicyError::OwnershipConflict {
                    client_order_id: client_order_id.to_string(),
                })
            }
            Some(_) => Ok(()),
            None => {
                owned.exchange_order_id = Some(exchange_order_id.to_string());
                Ok(())
            }
        }
    }
}

#[derive(Debug, Clone)]
pub(crate) struct ApprovedRegularSubmit {
    account_id: String,
    order: NewOrder,
    origin: OwnedRegularOrderOrigin,
}

impl ApprovedRegularSubmit {
    pub(crate) fn account_id(&self) -> &str {
        &self.account_id
    }

    pub(crate) fn order(&self) -> &NewOrder {
        &self.order
    }

    pub(crate) fn into_parts(self) -> (String, NewOrder) {
        (self.account_id, self.order)
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct ApprovedRegularCancel {
    account_id: String,
    symbol: String,
    client_order_id: String,
    reason: String,
}

impl ApprovedRegularCancel {
    pub(crate) fn into_parts(self) -> (String, String, String, String) {
        (
            self.account_id,
            self.symbol,
            self.client_order_id,
            self.reason,
        )
    }
}

#[derive(Debug, Clone)]
struct InstrumentExecutionProfile {
    account_id: String,
    risk_model: InstrumentRiskModel,
    order_limits: InstrumentOrderLimits,
    tick_size: f64,
    lot_size: f64,
    min_size: f64,
    quote_allowed: bool,
    hedge_allowed: bool,
}

#[derive(Debug)]
pub(crate) struct RegularExecutionPolicy {
    instruments: BTreeMap<String, InstrumentExecutionProfile>,
}

#[derive(Debug, Error)]
pub(crate) enum RegularExecutionPolicyError {
    #[error("verified execution instrument is missing for {symbol}")]
    MissingInstrument { symbol: String },
    #[error("verified execution owner for {symbol} is {actual}, expected {expected}")]
    OwnerMismatch {
        symbol: String,
        actual: String,
        expected: String,
    },
    #[error("verified execution trade mode for {symbol} differs from configuration")]
    TradeModeMismatch { symbol: String },
    #[error("quote-capable account {account_id} has no cancel_maker bootstrap proof")]
    QuoteStpUnverified { account_id: String },
    #[error("{purpose} is not enabled for configured symbol {symbol}")]
    PurposeUnavailable {
        purpose: &'static str,
        symbol: String,
    },
    #[error("regular order for {symbol} has a non-finite or non-positive {field}")]
    InvalidNumber { symbol: String, field: &'static str },
    #[error("regular order quantity {value} for {symbol} is below minimum {minimum}")]
    BelowMinimum {
        symbol: String,
        value: f64,
        minimum: f64,
    },
    #[error("regular order {field} {value} for {symbol} is not aligned to {increment}")]
    Misaligned {
        symbol: String,
        field: &'static str,
        value: f64,
        increment: f64,
    },
    #[error("regular order quantity {value} for {symbol} exceeds limit {limit}")]
    QuantityLimit {
        symbol: String,
        value: f64,
        limit: f64,
    },
    #[error("regular order notional {value} for {symbol} exceeds limit {limit}")]
    NotionalLimit {
        symbol: String,
        value: f64,
        limit: f64,
    },
    #[error("cancel target {client_order_id} is not a proven owned regular order")]
    UnknownOwnedOrder { client_order_id: String },
    #[error("owned regular order {client_order_id} is absent from canonical private state")]
    MissingCanonicalOrder { client_order_id: String },
    #[error("owned regular order {client_order_id} canonical symbol differs from ownership proof")]
    CanonicalSymbolMismatch { client_order_id: String },
    #[error("owned regular order {client_order_id} is no longer active")]
    TerminalOrder { client_order_id: String },
    #[error("owned regular order identity is empty or invalid")]
    InvalidOwnedIdentity,
    #[error("owned regular order {client_order_id} conflicts with an existing proof")]
    OwnershipConflict { client_order_id: String },
    #[error("client order id {client_order_id} already exists in canonical private state")]
    CanonicalOrderConflict { client_order_id: String },
}

impl RegularExecutionPolicy {
    pub(crate) fn from_verified(
        config: &LiveConfig,
        verified: &VerifiedBootstrap,
    ) -> Result<Self, RegularExecutionPolicyError> {
        let mut instruments = BTreeMap::new();
        for configured in &config.strategy.instruments {
            let verified_instrument =
                verified
                    .instruments
                    .get(&configured.symbol)
                    .ok_or_else(|| RegularExecutionPolicyError::MissingInstrument {
                        symbol: configured.symbol.clone(),
                    })?;
            let expected_account = config
                .account_for_symbol(&configured.symbol)
                .expect("validated live symbol must have an account owner");
            if verified_instrument.account_id != expected_account.id {
                return Err(RegularExecutionPolicyError::OwnerMismatch {
                    symbol: configured.symbol.clone(),
                    actual: verified_instrument.account_id.clone(),
                    expected: expected_account.id.clone(),
                });
            }
            if expected_account.trade_modes.get(&configured.symbol)
                != Some(&verified_instrument.trade_mode)
            {
                return Err(RegularExecutionPolicyError::TradeModeMismatch {
                    symbol: configured.symbol.clone(),
                });
            }
            let reference_only = config
                .strategy
                .risk_groups
                .iter()
                .find(|group| group.name == configured.risk_group)
                .is_some_and(|group| group.kind == RiskGroupKindConfig::RefOnly);
            let quote_allowed =
                !configured.halted && !reference_only && configured.quote_profit_margin < 1.0;
            let hedge_allowed =
                !configured.halted && !reference_only && configured.hedge_profit_margin < 1.0;
            if quote_allowed
                && !verified
                    .quote_stp_verified_accounts
                    .contains(&expected_account.id)
            {
                return Err(RegularExecutionPolicyError::QuoteStpUnverified {
                    account_id: expected_account.id.clone(),
                });
            }
            instruments.insert(
                configured.symbol.clone(),
                InstrumentExecutionProfile {
                    account_id: expected_account.id.clone(),
                    risk_model: verified_instrument.risk_model,
                    order_limits: verified_instrument.order_limits,
                    tick_size: verified_instrument.tick_size,
                    lot_size: verified_instrument.lot_size,
                    min_size: verified_instrument.min_size,
                    quote_allowed,
                    hedge_allowed,
                },
            );
        }
        Ok(Self { instruments })
    }

    pub(crate) fn authorize_submit(
        &self,
        intent: ChaosExecutionIntent,
    ) -> Result<ApprovedRegularSubmit, RegularExecutionPolicyError> {
        match intent {
            ChaosExecutionIntent::Quote(quote) => self.authorize_fields(
                quote.symbol(),
                quote.side(),
                quote.qty(),
                quote.price(),
                quote.reason(),
                OwnedRegularOrderOrigin::Quote,
            ),
            ChaosExecutionIntent::Hedge(hedge) => self.authorize_fields(
                hedge.symbol(),
                hedge.side(),
                hedge.qty(),
                hedge.price(),
                hedge.reason(),
                OwnedRegularOrderOrigin::Hedge,
            ),
            ChaosExecutionIntent::CancelOwned(_) => {
                unreachable!("CancelOwned must be authorized with ownership and canonical state")
            }
        }
    }

    fn authorize_fields(
        &self,
        symbol: &str,
        side: Side,
        qty: f64,
        price: f64,
        reason: &str,
        origin: OwnedRegularOrderOrigin,
    ) -> Result<ApprovedRegularSubmit, RegularExecutionPolicyError> {
        let symbol = symbol.to_string();
        let purpose = match origin {
            OwnedRegularOrderOrigin::Quote => "quote",
            OwnedRegularOrderOrigin::Hedge => "hedge",
            OwnedRegularOrderOrigin::RecoveredSubmitRequest => "recovered_submit_request",
        };
        let profile = self.instruments.get(&symbol).ok_or_else(|| {
            RegularExecutionPolicyError::MissingInstrument {
                symbol: symbol.clone(),
            }
        })?;
        let enabled = match origin {
            OwnedRegularOrderOrigin::Quote => profile.quote_allowed,
            OwnedRegularOrderOrigin::Hedge => profile.hedge_allowed,
            OwnedRegularOrderOrigin::RecoveredSubmitRequest => false,
        };
        if !enabled {
            return Err(RegularExecutionPolicyError::PurposeUnavailable { purpose, symbol });
        }
        self.validate_numeric(&symbol, qty, price, profile)?;
        let (time_in_force, self_trade_prevention) = match origin {
            OwnedRegularOrderOrigin::Quote => (TimeInForce::PostOnly, None),
            OwnedRegularOrderOrigin::Hedge => {
                (TimeInForce::Ioc, Some(SelfTradePrevention::CancelMaker))
            }
            OwnedRegularOrderOrigin::RecoveredSubmitRequest => unreachable!(),
        };
        Ok(ApprovedRegularSubmit {
            account_id: profile.account_id.clone(),
            order: NewOrder {
                symbol,
                side,
                qty,
                price,
                time_in_force,
                reduce_only: false,
                self_trade_prevention,
                reason: reason.to_string(),
            },
            origin,
        })
    }

    fn validate_numeric(
        &self,
        symbol: &str,
        qty: f64,
        price: f64,
        profile: &InstrumentExecutionProfile,
    ) -> Result<(), RegularExecutionPolicyError> {
        for (field, value) in [("quantity", qty), ("price", price)] {
            if !value.is_finite() || value <= 0.0 {
                return Err(RegularExecutionPolicyError::InvalidNumber {
                    symbol: symbol.to_string(),
                    field,
                });
            }
        }
        if qty < profile.min_size {
            return Err(RegularExecutionPolicyError::BelowMinimum {
                symbol: symbol.to_string(),
                value: qty,
                minimum: profile.min_size,
            });
        }
        for (field, value, increment) in [
            ("quantity", qty, profile.lot_size),
            ("price", price, profile.tick_size),
        ] {
            if !aligned(value, increment) {
                return Err(RegularExecutionPolicyError::Misaligned {
                    symbol: symbol.to_string(),
                    field,
                    value,
                    increment,
                });
            }
        }
        if qty > profile.order_limits.max_limit_quantity {
            return Err(RegularExecutionPolicyError::QuantityLimit {
                symbol: symbol.to_string(),
                value: qty,
                limit: profile.order_limits.max_limit_quantity,
            });
        }
        if let Some(limit) = profile.order_limits.max_limit_notional_usd {
            let value = profile.risk_model.notional_usd(qty, price);
            if value > limit {
                return Err(RegularExecutionPolicyError::NotionalLimit {
                    symbol: symbol.to_string(),
                    value,
                    limit,
                });
            }
        }
        Ok(())
    }

    pub(crate) fn validate_recovered_identity(
        &self,
        account_id: &str,
        symbol: &str,
    ) -> Result<(), RegularExecutionPolicyError> {
        let profile = self.instruments.get(symbol).ok_or_else(|| {
            RegularExecutionPolicyError::MissingInstrument {
                symbol: symbol.to_string(),
            }
        })?;
        if profile.account_id != account_id {
            return Err(RegularExecutionPolicyError::OwnerMismatch {
                symbol: symbol.to_string(),
                actual: account_id.to_string(),
                expected: profile.account_id.clone(),
            });
        }
        Ok(())
    }

    pub(crate) fn authorize_cancel(
        &self,
        client_order_id: &str,
        reason: &str,
        owned: &OwnedRegularOrders,
        private_states: &HashMap<String, PrivateStateReducer>,
    ) -> Result<ApprovedRegularCancel, RegularExecutionPolicyError> {
        let proof = owned.get(client_order_id).ok_or_else(|| {
            RegularExecutionPolicyError::UnknownOwnedOrder {
                client_order_id: client_order_id.to_string(),
            }
        })?;
        self.validate_recovered_identity(proof.account_id(), proof.symbol())?;
        let canonical = private_states
            .get(proof.account_id())
            .and_then(|state| state.order_reducer().get(proof.client_order_id()))
            .ok_or_else(|| RegularExecutionPolicyError::MissingCanonicalOrder {
                client_order_id: client_order_id.to_string(),
            })?;
        if canonical.symbol != proof.symbol() {
            return Err(RegularExecutionPolicyError::CanonicalSymbolMismatch {
                client_order_id: client_order_id.to_string(),
            });
        }
        if !matches!(
            canonical.status,
            OrderStatus::PendingNew | OrderStatus::Live | OrderStatus::PartiallyFilled
        ) {
            return Err(RegularExecutionPolicyError::TerminalOrder {
                client_order_id: client_order_id.to_string(),
            });
        }
        Ok(ApprovedRegularCancel {
            account_id: proof.account_id.clone(),
            symbol: proof.symbol.clone(),
            client_order_id: proof.client_order_id.clone(),
            reason: reason.to_string(),
        })
    }
}

fn aligned(value: f64, increment: f64) -> bool {
    if !increment.is_finite() || increment <= 0.0 {
        return false;
    }
    let units = value / increment;
    (units - units.round()).abs() <= 8.0 * f64::EPSILON * units.abs().max(1.0)
}

#[cfg(test)]
mod tests {
    use super::*;

    const ACCOUNT_ID: &str = "regular-account";
    const SYMBOL: &str = "BTC-USDT";

    fn test_policy(quote_allowed: bool, hedge_allowed: bool) -> RegularExecutionPolicy {
        RegularExecutionPolicy {
            instruments: BTreeMap::from([(
                SYMBOL.to_string(),
                InstrumentExecutionProfile {
                    account_id: ACCOUNT_ID.to_string(),
                    risk_model: InstrumentRiskModel::Spot,
                    order_limits: InstrumentOrderLimits {
                        max_limit_quantity: 10.0,
                        max_limit_notional_usd: Some(1_000.0),
                    },
                    tick_size: 0.5,
                    lot_size: 0.1,
                    min_size: 0.2,
                    quote_allowed,
                    hedge_allowed,
                },
            )]),
        }
    }

    fn authorize(
        policy: &RegularExecutionPolicy,
        symbol: &str,
        side: Side,
        qty: f64,
        price: f64,
        reason: &str,
        origin: OwnedRegularOrderOrigin,
    ) -> Result<ApprovedRegularSubmit, RegularExecutionPolicyError> {
        policy.authorize_fields(symbol, side, qty, price, reason, origin)
    }

    fn approved_quote(policy: &RegularExecutionPolicy) -> ApprovedRegularSubmit {
        authorize(
            policy,
            SYMBOL,
            Side::Buy,
            2.0,
            100.0,
            "quote_reason",
            OwnedRegularOrderOrigin::Quote,
        )
        .expect("valid quote must be approved")
    }

    #[test]
    fn quote_and_hedge_have_exact_final_execution_profiles() {
        let policy = test_policy(true, true);

        let quote = approved_quote(&policy);
        assert_eq!(quote.account_id, ACCOUNT_ID);
        assert_eq!(quote.origin, OwnedRegularOrderOrigin::Quote);
        assert_eq!(quote.order.symbol, SYMBOL);
        assert_eq!(quote.order.side, Side::Buy);
        assert_eq!(quote.order.qty, 2.0);
        assert_eq!(quote.order.price, 100.0);
        assert_eq!(quote.order.time_in_force, TimeInForce::PostOnly);
        assert!(!quote.order.reduce_only);
        assert_eq!(quote.order.self_trade_prevention, None);
        assert_eq!(quote.order.reason, "quote_reason");

        let hedge = authorize(
            &policy,
            SYMBOL,
            Side::Sell,
            1.5,
            200.0,
            "hedge_reason",
            OwnedRegularOrderOrigin::Hedge,
        )
        .expect("valid hedge must be approved");
        assert_eq!(hedge.account_id, ACCOUNT_ID);
        assert_eq!(hedge.origin, OwnedRegularOrderOrigin::Hedge);
        assert_eq!(hedge.order.symbol, SYMBOL);
        assert_eq!(hedge.order.side, Side::Sell);
        assert_eq!(hedge.order.qty, 1.5);
        assert_eq!(hedge.order.price, 200.0);
        assert_eq!(hedge.order.time_in_force, TimeInForce::Ioc);
        assert!(!hedge.order.reduce_only);
        assert_eq!(
            hedge.order.self_trade_prevention,
            Some(SelfTradePrevention::CancelMaker)
        );
        assert_eq!(hedge.order.reason, "hedge_reason");
    }

    #[test]
    fn submit_policy_rejects_unverified_symbols_purposes_and_numeric_limits() {
        let policy = test_policy(true, true);

        assert!(matches!(
            authorize(
                &policy,
                "ETH-USDT",
                Side::Buy,
                1.0,
                100.0,
                "unknown",
                OwnedRegularOrderOrigin::Quote,
            ),
            Err(RegularExecutionPolicyError::MissingInstrument { symbol })
                if symbol == "ETH-USDT"
        ));

        let quote_disabled = test_policy(false, true);
        assert!(matches!(
            authorize(
                &quote_disabled,
                SYMBOL,
                Side::Buy,
                1.0,
                100.0,
                "disabled",
                OwnedRegularOrderOrigin::Quote,
            ),
            Err(RegularExecutionPolicyError::PurposeUnavailable {
                purpose: "quote",
                symbol,
            }) if symbol == SYMBOL
        ));
        let hedge_disabled = test_policy(true, false);
        assert!(matches!(
            authorize(
                &hedge_disabled,
                SYMBOL,
                Side::Sell,
                1.0,
                100.0,
                "disabled",
                OwnedRegularOrderOrigin::Hedge,
            ),
            Err(RegularExecutionPolicyError::PurposeUnavailable {
                purpose: "hedge",
                symbol,
            }) if symbol == SYMBOL
        ));

        for (qty, price, expected_field) in [
            (0.0, 100.0, "quantity"),
            (f64::NAN, 100.0, "quantity"),
            (1.0, 0.0, "price"),
            (1.0, f64::INFINITY, "price"),
        ] {
            assert!(matches!(
                authorize(
                    &policy,
                    SYMBOL,
                    Side::Buy,
                    qty,
                    price,
                    "invalid",
                    OwnedRegularOrderOrigin::Quote,
                ),
                Err(RegularExecutionPolicyError::InvalidNumber { symbol, field })
                    if symbol == SYMBOL && field == expected_field
            ));
        }

        assert!(matches!(
            authorize(
                &policy,
                SYMBOL,
                Side::Buy,
                0.1,
                100.0,
                "below-minimum",
                OwnedRegularOrderOrigin::Quote,
            ),
            Err(RegularExecutionPolicyError::BelowMinimum {
                symbol,
                value: 0.1,
                minimum: 0.2,
            }) if symbol == SYMBOL
        ));
        assert!(matches!(
            authorize(
                &policy,
                SYMBOL,
                Side::Buy,
                0.25,
                100.0,
                "bad-lot",
                OwnedRegularOrderOrigin::Quote,
            ),
            Err(RegularExecutionPolicyError::Misaligned {
                symbol,
                field: "quantity",
                value: 0.25,
                increment: 0.1,
            }) if symbol == SYMBOL
        ));
        assert!(matches!(
            authorize(
                &policy,
                SYMBOL,
                Side::Buy,
                1.0,
                100.25,
                "bad-tick",
                OwnedRegularOrderOrigin::Quote,
            ),
            Err(RegularExecutionPolicyError::Misaligned {
                symbol,
                field: "price",
                value: 100.25,
                increment: 0.5,
            }) if symbol == SYMBOL
        ));
        assert!(matches!(
            authorize(
                &policy,
                SYMBOL,
                Side::Buy,
                10.1,
                50.0,
                "quantity-limit",
                OwnedRegularOrderOrigin::Quote,
            ),
            Err(RegularExecutionPolicyError::QuantityLimit {
                symbol,
                value: 10.1,
                limit: 10.0,
            }) if symbol == SYMBOL
        ));
        assert!(matches!(
            authorize(
                &policy,
                SYMBOL,
                Side::Buy,
                2.0,
                600.0,
                "notional-limit",
                OwnedRegularOrderOrigin::Quote,
            ),
            Err(RegularExecutionPolicyError::NotionalLimit {
                symbol,
                value: 1_200.0,
                limit: 1_000.0,
            }) if symbol == SYMBOL
        ));
    }

    #[test]
    fn alignment_tolerance_stays_bounded_at_large_magnitudes() {
        assert!(aligned(0.3, 0.1));
        assert!(aligned(500_000_000.0, 0.5));
        assert!(!aligned(500_000_000.25, 0.5));

        let mut policy = test_policy(true, true);
        let profile = policy
            .instruments
            .get_mut(SYMBOL)
            .expect("test policy must contain its configured symbol");
        profile.order_limits.max_limit_notional_usd = None;

        assert!(
            authorize(
                &policy,
                SYMBOL,
                Side::Buy,
                1.0,
                500_000_000.0,
                "large-aligned-price",
                OwnedRegularOrderOrigin::Quote,
            )
            .is_ok()
        );
        assert!(matches!(
            authorize(
                &policy,
                SYMBOL,
                Side::Buy,
                1.0,
                500_000_000.25,
                "large-half-tick",
                OwnedRegularOrderOrigin::Quote,
            ),
            Err(RegularExecutionPolicyError::Misaligned {
                symbol,
                field: "price",
                value: 500_000_000.25,
                increment: 0.5,
            }) if symbol == SYMBOL
        ));
    }

    #[test]
    fn notional_limit_uses_the_authenticated_instrument_risk_model() {
        let mut linear = test_policy(true, true);
        let profile = linear.instruments.get_mut(SYMBOL).unwrap();
        profile.risk_model = InstrumentRiskModel::LinearDerivative {
            contract_value: 2.0,
        };
        profile.order_limits.max_limit_notional_usd = Some(1_000.0);

        assert!(
            authorize(
                &linear,
                SYMBOL,
                Side::Buy,
                5.0,
                100.0,
                "linear-at-limit",
                OwnedRegularOrderOrigin::Quote,
            )
            .is_ok()
        );
        assert!(matches!(
            authorize(
                &linear,
                SYMBOL,
                Side::Buy,
                5.0,
                100.5,
                "linear-over-limit",
                OwnedRegularOrderOrigin::Quote,
            ),
            Err(RegularExecutionPolicyError::NotionalLimit {
                value: 1_005.0,
                limit: 1_000.0,
                ..
            })
        ));

        let mut inverse = test_policy(true, true);
        let profile = inverse.instruments.get_mut(SYMBOL).unwrap();
        profile.risk_model = InstrumentRiskModel::InverseDerivative {
            contract_value: 100.0,
        };
        profile.order_limits.max_limit_quantity = 100.0;
        profile.order_limits.max_limit_notional_usd = Some(1_000.0);

        assert!(
            authorize(
                &inverse,
                SYMBOL,
                Side::Sell,
                10.0,
                50_000.0,
                "inverse-at-limit",
                OwnedRegularOrderOrigin::Hedge,
            )
            .is_ok()
        );
        assert!(matches!(
            authorize(
                &inverse,
                SYMBOL,
                Side::Sell,
                10.1,
                1.0,
                "inverse-over-limit",
                OwnedRegularOrderOrigin::Hedge,
            ),
            Err(RegularExecutionPolicyError::NotionalLimit {
                value: 1_010.0,
                limit: 1_000.0,
                ..
            })
        ));
    }

    #[test]
    fn recovered_identity_requires_the_configured_account_and_symbol() {
        let policy = test_policy(true, true);

        assert!(
            policy
                .validate_recovered_identity(ACCOUNT_ID, SYMBOL)
                .is_ok()
        );
        assert!(matches!(
            policy.validate_recovered_identity("foreign-account", SYMBOL),
            Err(RegularExecutionPolicyError::OwnerMismatch {
                symbol,
                actual,
                expected,
            }) if symbol == SYMBOL
                && actual == "foreign-account"
                && expected == ACCOUNT_ID
        ));
        assert!(matches!(
            policy.validate_recovered_identity(ACCOUNT_ID, "ETH-USDT"),
            Err(RegularExecutionPolicyError::MissingInstrument { symbol })
                if symbol == "ETH-USDT"
        ));
    }

    #[test]
    fn cancel_accepts_only_the_owned_client_id_with_matching_active_canonical_state() {
        let policy = test_policy(true, true);
        let approved = approved_quote(&policy);
        let mut owned = OwnedRegularOrders::default();
        let mut state = PrivateStateReducer::new();
        owned
            .reserve_local(&approved, "reap-local-1", &mut state, 0)
            .expect("approved local order must establish ownership");
        owned
            .bind_exchange_order_id(ACCOUNT_ID, "reap-local-1", "exchange-42")
            .expect("exchange acknowledgement must bind to owned order");
        let private_states = HashMap::from([(ACCOUNT_ID.to_string(), state)]);

        let cancel = policy
            .authorize_cancel("reap-local-1", "risk_fail_closed", &owned, &private_states)
            .expect("owned active canonical order must be cancellable");
        assert_eq!(
            cancel.into_parts(),
            (
                ACCOUNT_ID.to_string(),
                SYMBOL.to_string(),
                "reap-local-1".to_string(),
                "risk_fail_closed".to_string(),
            )
        );

        for unproven_id in [
            "unknown-order",
            "reap-local-1-suffix",
            "algo-order-7",
            "spread-order-9",
            "exchange-42",
        ] {
            assert!(matches!(
                policy.authorize_cancel(unproven_id, "deny", &owned, &private_states),
                Err(RegularExecutionPolicyError::UnknownOwnedOrder { client_order_id })
                    if client_order_id == unproven_id
            ));
        }
    }

    #[test]
    fn cancel_rejects_missing_terminal_and_account_symbol_mismatched_state() {
        let policy = test_policy(true, true);
        let approved = approved_quote(&policy);

        let mut missing_owned = OwnedRegularOrders::default();
        let mut removed_state = PrivateStateReducer::new();
        missing_owned
            .reserve_local(&approved, "missing-canonical", &mut removed_state, 0)
            .unwrap();
        removed_state.remove_local_order("missing-canonical");
        assert!(matches!(
            policy.authorize_cancel(
                "missing-canonical",
                "deny",
                &missing_owned,
                &HashMap::new(),
            ),
            Err(RegularExecutionPolicyError::MissingCanonicalOrder { client_order_id })
                if client_order_id == "missing-canonical"
        ));

        let mut terminal_owned = OwnedRegularOrders::default();
        let mut terminal_state = PrivateStateReducer::new();
        terminal_owned
            .reserve_local(&approved, "terminal-order", &mut terminal_state, 0)
            .unwrap();
        terminal_state
            .reject_local_order("terminal-order", 1, "rejected")
            .expect("pending order can become terminal");
        let terminal_states = HashMap::from([(ACCOUNT_ID.to_string(), terminal_state)]);
        assert!(matches!(
            policy.authorize_cancel(
                "terminal-order",
                "deny",
                &terminal_owned,
                &terminal_states,
            ),
            Err(RegularExecutionPolicyError::TerminalOrder { client_order_id })
                if client_order_id == "terminal-order"
        ));

        let mut foreign_account_owned = OwnedRegularOrders::default();
        foreign_account_owned
            .register_recovered("foreign-account", SYMBOL, "foreign-account-order", None)
            .unwrap();
        assert!(matches!(
            policy.authorize_cancel(
                "foreign-account-order",
                "deny",
                &foreign_account_owned,
                &HashMap::new(),
            ),
            Err(RegularExecutionPolicyError::OwnerMismatch { actual, expected, .. })
                if actual == "foreign-account" && expected == ACCOUNT_ID
        ));

        let mut foreign_symbol_owned = OwnedRegularOrders::default();
        foreign_symbol_owned
            .register_recovered(ACCOUNT_ID, "ETH-USDT", "foreign-symbol-order", None)
            .unwrap();
        assert!(matches!(
            policy.authorize_cancel(
                "foreign-symbol-order",
                "deny",
                &foreign_symbol_owned,
                &HashMap::new(),
            ),
            Err(RegularExecutionPolicyError::MissingInstrument { symbol })
                if symbol == "ETH-USDT"
        ));

        let mut mismatched_owned = OwnedRegularOrders::default();
        let mut proof_state = PrivateStateReducer::new();
        mismatched_owned
            .reserve_local(&approved, "canonical-symbol-mismatch", &mut proof_state, 0)
            .unwrap();
        let mut mismatched_state = PrivateStateReducer::new();
        let mut wrong_symbol_order = approved.order.clone();
        wrong_symbol_order.symbol = "ETH-USDT".to_string();
        mismatched_state.register_local_order("canonical-symbol-mismatch", wrong_symbol_order);
        let mismatched_states = HashMap::from([(ACCOUNT_ID.to_string(), mismatched_state)]);
        assert!(matches!(
            policy.authorize_cancel(
                "canonical-symbol-mismatch",
                "deny",
                &mismatched_owned,
                &mismatched_states,
            ),
            Err(RegularExecutionPolicyError::CanonicalSymbolMismatch { client_order_id })
                if client_order_id == "canonical-symbol-mismatch"
        ));
    }

    #[test]
    fn ownership_registry_rejects_duplicate_local_invalid_and_conflicting_proof() {
        let policy = test_policy(true, true);
        let approved = approved_quote(&policy);
        let mut owned = OwnedRegularOrders::default();
        let mut first_state = PrivateStateReducer::new();

        owned
            .reserve_local(&approved, "owned-1", &mut first_state, 0)
            .unwrap();
        assert!(matches!(
            owned.reserve_local(&approved, "owned-1", &mut first_state, 1),
            Err(RegularExecutionPolicyError::CanonicalOrderConflict { client_order_id })
                if client_order_id == "owned-1"
        ));
        let mut duplicate_state = PrivateStateReducer::new();
        assert!(matches!(
            owned.reserve_local(&approved, "owned-1", &mut duplicate_state, 1),
            Err(RegularExecutionPolicyError::OwnershipConflict { client_order_id })
                if client_order_id == "owned-1"
        ));
        assert!(
            !duplicate_state.order_reducer().contains_order("owned-1"),
            "ownership conflicts must roll back the new canonical reservation"
        );
        assert!(matches!(
            owned.register_recovered(ACCOUNT_ID, SYMBOL, "owned-1", None),
            Err(RegularExecutionPolicyError::OwnershipConflict { client_order_id })
                if client_order_id == "owned-1"
        ));
        assert!(matches!(
            owned.register_recovered(ACCOUNT_ID, SYMBOL, "", None),
            Err(RegularExecutionPolicyError::InvalidOwnedIdentity)
        ));
        assert!(matches!(
            owned.register_recovered(ACCOUNT_ID, SYMBOL, "0", None),
            Err(RegularExecutionPolicyError::InvalidOwnedIdentity)
        ));
        assert!(matches!(
            owned.register_recovered(ACCOUNT_ID, SYMBOL, "owned-2", Some("")),
            Err(RegularExecutionPolicyError::InvalidOwnedIdentity)
        ));
        assert!(matches!(
            owned.register_recovered(ACCOUNT_ID, SYMBOL, "owned-2", Some("0")),
            Err(RegularExecutionPolicyError::InvalidOwnedIdentity)
        ));

        owned
            .bind_exchange_order_id(ACCOUNT_ID, "owned-1", "exchange-1")
            .unwrap();
        owned
            .bind_exchange_order_id(ACCOUNT_ID, "owned-1", "exchange-1")
            .unwrap();
        assert_eq!(
            owned.get("owned-1").unwrap().exchange_order_id.as_deref(),
            Some("exchange-1")
        );
        assert!(matches!(
            owned.bind_exchange_order_id(ACCOUNT_ID, "owned-1", "exchange-2"),
            Err(RegularExecutionPolicyError::OwnershipConflict { client_order_id })
                if client_order_id == "owned-1"
        ));
        assert!(matches!(
            owned.bind_exchange_order_id("foreign-account", "owned-1", "exchange-1"),
            Err(RegularExecutionPolicyError::OwnershipConflict { client_order_id })
                if client_order_id == "owned-1"
        ));
        assert!(matches!(
            owned.bind_exchange_order_id(ACCOUNT_ID, "unknown", "exchange-3"),
            Err(RegularExecutionPolicyError::UnknownOwnedOrder { client_order_id })
                if client_order_id == "unknown"
        ));
    }
}
