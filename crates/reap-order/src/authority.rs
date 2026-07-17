use std::collections::{BTreeMap, HashMap};
use std::sync::Arc;

use reap_core::{
    NewOrder, OrderStatus, OrderUpdate, SelfTradePrevention, Side, TimeInForce, TimeMs,
};
use reap_risk::{InstrumentOrderLimits, InstrumentRiskModel};
use reap_storage::ProvenRegularSubmitRequest;
use reap_strategy::ChaosExecutionIntent;
use thiserror::Error;

use crate::{ClientIdError, ClientOrderIdGenerator, GeneratedClientOrderId, PrivateStateReducer};

#[derive(Debug, Clone)]
pub(crate) struct RegularApprovalBinding(Arc<RegularApprovalBindingMarker>);

#[derive(Debug)]
struct RegularApprovalBindingMarker;

impl RegularApprovalBinding {
    pub(crate) fn new() -> Self {
        Self(Arc::new(RegularApprovalBindingMarker))
    }

    pub(crate) fn matches(&self, other: &Self) -> bool {
        Arc::ptr_eq(&self.0, &other.0)
    }
}

impl PartialEq for RegularApprovalBinding {
    fn eq(&self, other: &Self) -> bool {
        self.matches(other)
    }
}

impl Eq for RegularApprovalBinding {}

/// Take-once authority for binding one account's verified execution profiles
/// to the gateway that created this scope.
#[derive(Debug)]
pub struct RegularApprovalScope {
    account_id: String,
    binding: RegularApprovalBinding,
}

impl RegularApprovalScope {
    pub(crate) fn new(account_id: String, binding: RegularApprovalBinding) -> Self {
        Self {
            account_id,
            binding,
        }
    }

    pub fn account_id(&self) -> &str {
        &self.account_id
    }

    pub fn bind_profiles(
        self,
        profiles: impl IntoIterator<Item = RegularExecutionProfile>,
    ) -> Result<RegularExecutionProfileSet, RegularExecutionPolicyError> {
        let profiles = profiles.into_iter().collect::<Vec<_>>();
        if let Some(profile) = profiles
            .iter()
            .find(|profile| profile.account_id != self.account_id)
        {
            return Err(RegularExecutionPolicyError::OwnerMismatch {
                symbol: profile.symbol.clone(),
                actual: profile.account_id.clone(),
                expected: self.account_id,
            });
        }
        Ok(RegularExecutionProfileSet {
            account_id: self.account_id,
            binding: self.binding,
            profiles,
        })
    }

    pub fn bind_profiles_and_client_id_generator(
        self,
        profiles: impl IntoIterator<Item = RegularExecutionProfile>,
        id_prefix: impl Into<String>,
        node_id: u16,
    ) -> Result<(RegularExecutionProfileSet, ClientOrderIdGenerator), RegularExecutionPolicyError>
    {
        let account_id = self.account_id.clone();
        let generator = ClientOrderIdGenerator::new(id_prefix, node_id, self.binding.clone())
            .map_err(|source| RegularExecutionPolicyError::ClientIdSetup { account_id, source })?;
        Ok((self.bind_profiles(profiles)?, generator))
    }
}

/// Opaque, gateway-bound profile set. It can only be created by consuming a
/// take-once [`RegularApprovalScope`].
#[derive(Debug)]
pub struct RegularExecutionProfileSet {
    account_id: String,
    binding: RegularApprovalBinding,
    profiles: Vec<RegularExecutionProfile>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum OwnedRegularOrderOrigin {
    Quote,
    Hedge,
    RecoveredSubmitRequest,
}

#[derive(Debug, Clone, PartialEq, Eq)]
struct OwnedRegularOrder {
    account_id: String,
    symbol: String,
    client_order_id: String,
    exchange_order_id: Option<String>,
    origin: OwnedRegularOrderOrigin,
    binding: Option<RegularApprovalBinding>,
}

impl OwnedRegularOrder {
    fn account_id(&self) -> &str {
        &self.account_id
    }

    fn symbol(&self) -> &str {
        &self.symbol
    }

    fn client_order_id(&self) -> &str {
        &self.client_order_id
    }
}

/// Ownership proof is deliberately separate from canonical observation.
///
/// Entries can be added only after policy-approved local submission or from a
/// qualifying durable regular-submit request recovered by storage. Private
/// streams, reconciliation, prefixes, and free-form reasons never add entries.
#[derive(Debug, Default)]
pub struct OwnedRegularOrders {
    by_client_order_id: BTreeMap<String, OwnedRegularOrder>,
}

#[derive(Debug)]
struct RecoveredRegularSubmitProof {
    account_id: String,
    symbol: String,
    client_order_id: String,
    binding: Option<RegularApprovalBinding>,
}

impl OwnedRegularOrders {
    /// Atomically reserves a vacant canonical identity and establishes local
    /// ownership. A proof conflict rolls back the just-created pending order.
    pub fn reserve_local(
        &mut self,
        approved: ApprovedRegularSubmit,
        client_order_id: GeneratedClientOrderId,
        private_state: &mut PrivateStateReducer,
        ts_ms: TimeMs,
    ) -> Result<(OrderUpdate, ReservedRegularSubmit), RegularExecutionPolicyError> {
        if !approved.binding.matches(client_order_id.binding()) {
            return Err(RegularExecutionPolicyError::ClientOrderIdScopeMismatch {
                account_id: approved.account_id,
            });
        }
        let client_order_id = client_order_id.into_string();
        let pending = private_state
            .register_local_order_at(&client_order_id, approved.order.clone(), ts_ms)
            .ok_or_else(|| RegularExecutionPolicyError::CanonicalOrderConflict {
                client_order_id: client_order_id.clone(),
            })?;
        if self.by_client_order_id.contains_key(&client_order_id) {
            private_state.remove_local_order(&client_order_id);
            return Err(RegularExecutionPolicyError::OwnershipConflict { client_order_id });
        }
        if let Err(error) = self.insert(OwnedRegularOrder {
            account_id: approved.account_id.clone(),
            symbol: approved.order.symbol.clone(),
            client_order_id: client_order_id.clone(),
            exchange_order_id: None,
            origin: approved.origin,
            binding: Some(approved.binding.clone()),
        }) {
            private_state.remove_local_order(&client_order_id);
            return Err(error);
        }
        Ok((
            pending,
            ReservedRegularSubmit {
                account_id: approved.account_id,
                client_order_id,
                order: approved.order,
                binding: approved.binding,
            },
        ))
    }

    pub fn register_recovered(
        &mut self,
        policy: &RegularExecutionPolicy,
        recovered: ProvenRegularSubmitRequest,
    ) -> Result<(), RegularExecutionPolicyError> {
        let proof = policy.lower_recovered_submit(&recovered)?;
        self.insert(OwnedRegularOrder {
            account_id: proof.account_id,
            symbol: proof.symbol,
            client_order_id: proof.client_order_id,
            exchange_order_id: None,
            origin: OwnedRegularOrderOrigin::RecoveredSubmitRequest,
            binding: proof.binding,
        })
    }

    fn insert(&mut self, order: OwnedRegularOrder) -> Result<(), RegularExecutionPolicyError> {
        validate_owned_identity(
            &order.account_id,
            &order.symbol,
            &order.client_order_id,
            order.exchange_order_id.as_deref(),
        )?;
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

    fn get(&self, client_order_id: &str) -> Option<&OwnedRegularOrder> {
        self.by_client_order_id.get(client_order_id)
    }

    pub fn proves_identity(&self, client_order_id: &str, account_id: &str, symbol: &str) -> bool {
        self.get(client_order_id)
            .is_some_and(|owned| owned.account_id() == account_id && owned.symbol() == symbol)
    }

    pub fn proves_account(&self, client_order_id: &str, account_id: &str) -> bool {
        self.get(client_order_id)
            .is_some_and(|owned| owned.account_id() == account_id)
    }

    pub fn bind_exchange_order_id(
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

#[derive(Debug)]
pub struct ApprovedRegularSubmit {
    account_id: String,
    order: NewOrder,
    origin: OwnedRegularOrderOrigin,
    binding: RegularApprovalBinding,
}

impl ApprovedRegularSubmit {
    pub fn account_id(&self) -> &str {
        &self.account_id
    }

    pub fn order(&self) -> &NewOrder {
        &self.order
    }
}

/// A policy-approved submit that has atomically established canonical local
/// state and ownership. Gateways accept this one-shot capability, never the
/// earlier approval token.
#[derive(Debug)]
pub struct ReservedRegularSubmit {
    account_id: String,
    client_order_id: String,
    order: NewOrder,
    binding: RegularApprovalBinding,
}

impl ReservedRegularSubmit {
    pub fn account_id(&self) -> &str {
        &self.account_id
    }

    pub fn order(&self) -> &NewOrder {
        &self.order
    }

    pub fn client_order_id(&self) -> &str {
        &self.client_order_id
    }

    pub(crate) fn binding(&self) -> &RegularApprovalBinding {
        &self.binding
    }

    pub(crate) fn into_parts(self) -> (String, String, NewOrder, RegularApprovalBinding) {
        (
            self.account_id,
            self.client_order_id,
            self.order,
            self.binding,
        )
    }
}

#[derive(Debug, PartialEq, Eq)]
pub struct ApprovedRegularCancel {
    account_id: String,
    symbol: String,
    client_order_id: String,
    reason: String,
    binding: RegularApprovalBinding,
}

impl ApprovedRegularCancel {
    pub fn account_id(&self) -> &str {
        &self.account_id
    }

    pub fn symbol(&self) -> &str {
        &self.symbol
    }

    pub fn client_order_id(&self) -> &str {
        &self.client_order_id
    }

    pub fn reason(&self) -> &str {
        &self.reason
    }

    pub(crate) fn binding(&self) -> &RegularApprovalBinding {
        &self.binding
    }

    pub(crate) fn into_parts(self) -> (String, String, String, String, RegularApprovalBinding) {
        (
            self.account_id,
            self.symbol,
            self.client_order_id,
            self.reason,
            self.binding,
        )
    }
}

#[cfg(test)]
pub(crate) fn reserved_regular_submit_for_test(
    account_id: impl Into<String>,
    client_order_id: impl Into<String>,
    order: NewOrder,
    binding: RegularApprovalBinding,
) -> ReservedRegularSubmit {
    ReservedRegularSubmit {
        account_id: account_id.into(),
        client_order_id: client_order_id.into(),
        order,
        binding,
    }
}

#[cfg(test)]
pub(crate) fn approved_regular_cancel_for_test(
    account_id: impl Into<String>,
    symbol: impl Into<String>,
    client_order_id: impl Into<String>,
    reason: impl Into<String>,
    binding: RegularApprovalBinding,
) -> ApprovedRegularCancel {
    ApprovedRegularCancel {
        account_id: account_id.into(),
        symbol: symbol.into(),
        client_order_id: client_order_id.into(),
        reason: reason.into(),
        binding,
    }
}

#[derive(Debug, Clone)]
pub struct RegularExecutionProfile {
    symbol: String,
    account_id: String,
    risk_model: InstrumentRiskModel,
    order_limits: InstrumentOrderLimits,
    tick_size: f64,
    lot_size: f64,
    min_size: f64,
    quote_allowed: bool,
    hedge_allowed: bool,
    quote_stp_verified: bool,
}

impl RegularExecutionProfile {
    #[allow(clippy::too_many_arguments)]
    pub fn new(
        symbol: impl Into<String>,
        account_id: impl Into<String>,
        risk_model: InstrumentRiskModel,
        order_limits: InstrumentOrderLimits,
        tick_size: f64,
        lot_size: f64,
        min_size: f64,
        quote_allowed: bool,
        hedge_allowed: bool,
        quote_stp_verified: bool,
    ) -> Self {
        Self {
            symbol: symbol.into(),
            account_id: account_id.into(),
            risk_model,
            order_limits,
            tick_size,
            lot_size,
            min_size,
            quote_allowed,
            hedge_allowed,
            quote_stp_verified,
        }
    }
}

#[derive(Debug)]
pub struct RegularExecutionPolicy {
    instruments: BTreeMap<String, BoundRegularExecutionProfile>,
}

#[derive(Debug)]
struct BoundRegularExecutionProfile {
    profile: RegularExecutionProfile,
    binding: Option<RegularApprovalBinding>,
}

#[derive(Debug, Error)]
pub enum RegularExecutionPolicyError {
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
    #[error("verified execution instrument is duplicated for {symbol}")]
    DuplicateInstrument { symbol: String },
    #[error("regular execution approval scope is duplicated for account {account_id}")]
    DuplicateApprovalScope { account_id: String },
    #[error("regular execution approval scope references unconfigured account {account_id}")]
    UnknownApprovalAccount { account_id: String },
    #[error("account {account_id} has no gateway-bound approval scope for {symbol}")]
    MissingApprovalScope { account_id: String, symbol: String },
    #[error("client order id setup failed for account {account_id}: {source}")]
    ClientIdSetup {
        account_id: String,
        #[source]
        source: ClientIdError,
    },
    #[error("generated client order id does not belong to account {account_id}'s gateway scope")]
    ClientOrderIdScopeMismatch { account_id: String },
    #[error("verified execution profile for {symbol:?} has invalid {field}")]
    InvalidProfile { symbol: String, field: &'static str },
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
    #[error("CancelOwned requires canonical ownership authorization")]
    CancelOwnedRequiresOwnership,
}

impl RegularExecutionPolicy {
    pub fn from_profile_sets(
        profile_sets: impl IntoIterator<Item = RegularExecutionProfileSet>,
    ) -> Result<Self, RegularExecutionPolicyError> {
        Self::from_profiles_and_profile_sets([], profile_sets)
    }

    pub fn from_profiles_and_profile_sets(
        unbound_profiles: impl IntoIterator<Item = RegularExecutionProfile>,
        profile_sets: impl IntoIterator<Item = RegularExecutionProfileSet>,
    ) -> Result<Self, RegularExecutionPolicyError> {
        let mut instruments = BTreeMap::new();
        let mut accounts = BTreeMap::new();
        for profile in unbound_profiles {
            validate_profile(&profile)?;
            let symbol = profile.symbol.clone();
            if instruments
                .insert(
                    symbol.clone(),
                    BoundRegularExecutionProfile {
                        profile,
                        binding: None,
                    },
                )
                .is_some()
            {
                return Err(RegularExecutionPolicyError::DuplicateInstrument { symbol });
            }
        }
        for profile_set in profile_sets {
            if accounts
                .insert(profile_set.account_id.clone(), ())
                .is_some()
            {
                return Err(RegularExecutionPolicyError::DuplicateApprovalScope {
                    account_id: profile_set.account_id,
                });
            }
            for profile in profile_set.profiles {
                validate_profile(&profile)?;
                let symbol = profile.symbol.clone();
                let bound = BoundRegularExecutionProfile {
                    profile,
                    binding: Some(profile_set.binding.clone()),
                };
                if instruments.insert(symbol.clone(), bound).is_some() {
                    return Err(RegularExecutionPolicyError::DuplicateInstrument { symbol });
                }
            }
        }
        Ok(Self { instruments })
    }

    pub fn authorize_submit(
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
                Err(RegularExecutionPolicyError::CancelOwnedRequiresOwnership)
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
        let bound = self.instruments.get(&symbol).ok_or_else(|| {
            RegularExecutionPolicyError::MissingInstrument {
                symbol: symbol.clone(),
            }
        })?;
        let profile = &bound.profile;
        let binding = bound.binding.as_ref().ok_or_else(|| {
            RegularExecutionPolicyError::MissingApprovalScope {
                account_id: profile.account_id.clone(),
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
            binding: binding.clone(),
        })
    }

    fn validate_numeric(
        &self,
        symbol: &str,
        qty: f64,
        price: f64,
        profile: &RegularExecutionProfile,
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

    /// Binds storage's opaque durable-submit proof to this gateway-scoped
    /// execution policy. Raw identity fields cannot enter this path.
    fn lower_recovered_submit(
        &self,
        recovered: &ProvenRegularSubmitRequest,
    ) -> Result<RecoveredRegularSubmitProof, RegularExecutionPolicyError> {
        let account_id = recovered.account_id();
        let symbol = recovered.symbol();
        let client_order_id = recovered.client_order_id();
        let bound = self.instruments.get(symbol).ok_or_else(|| {
            RegularExecutionPolicyError::MissingInstrument {
                symbol: symbol.to_string(),
            }
        })?;
        if bound.profile.account_id != account_id {
            return Err(RegularExecutionPolicyError::OwnerMismatch {
                symbol: symbol.to_string(),
                actual: account_id.to_string(),
                expected: bound.profile.account_id.clone(),
            });
        }
        let proof = RecoveredRegularSubmitProof {
            account_id: account_id.to_string(),
            symbol: symbol.to_string(),
            client_order_id: client_order_id.to_string(),
            binding: bound.binding.clone(),
        };
        validate_owned_identity(
            &proof.account_id,
            &proof.symbol,
            &proof.client_order_id,
            None,
        )?;
        Ok(proof)
    }

    pub fn authorize_cancel(
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
        let bound = self.instruments.get(proof.symbol()).ok_or_else(|| {
            RegularExecutionPolicyError::MissingInstrument {
                symbol: proof.symbol().to_string(),
            }
        })?;
        if bound.profile.account_id != proof.account_id() {
            return Err(RegularExecutionPolicyError::OwnerMismatch {
                symbol: proof.symbol().to_string(),
                actual: proof.account_id().to_string(),
                expected: bound.profile.account_id.clone(),
            });
        }
        let Some(binding) = bound.binding.as_ref() else {
            return Err(RegularExecutionPolicyError::MissingApprovalScope {
                account_id: bound.profile.account_id.clone(),
                symbol: proof.symbol().to_string(),
            });
        };
        if !proof
            .binding
            .as_ref()
            .is_some_and(|owned_binding| binding.matches(owned_binding))
        {
            return Err(RegularExecutionPolicyError::OwnershipConflict {
                client_order_id: client_order_id.to_string(),
            });
        }
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
            binding: binding.clone(),
        })
    }
}

fn validate_profile(profile: &RegularExecutionProfile) -> Result<(), RegularExecutionPolicyError> {
    for (field, invalid) in [
        ("symbol", profile.symbol.trim().is_empty()),
        ("account_id", profile.account_id.trim().is_empty()),
        (
            "tick_size",
            !profile.tick_size.is_finite() || profile.tick_size <= 0.0,
        ),
        (
            "lot_size",
            !profile.lot_size.is_finite() || profile.lot_size <= 0.0,
        ),
        (
            "min_size",
            !profile.min_size.is_finite() || profile.min_size <= 0.0,
        ),
        ("order_limits", !profile.order_limits.is_valid()),
        ("risk_model", !profile.risk_model.is_valid()),
        (
            "min_size",
            profile.min_size > profile.order_limits.max_limit_quantity,
        ),
    ] {
        if invalid {
            return Err(RegularExecutionPolicyError::InvalidProfile {
                symbol: profile.symbol.clone(),
                field,
            });
        }
    }
    if profile.quote_allowed && !profile.quote_stp_verified {
        return Err(RegularExecutionPolicyError::QuoteStpUnverified {
            account_id: profile.account_id.clone(),
        });
    }
    Ok(())
}

fn validate_owned_identity(
    account_id: &str,
    symbol: &str,
    client_order_id: &str,
    exchange_order_id: Option<&str>,
) -> Result<(), RegularExecutionPolicyError> {
    if account_id.trim().is_empty()
        || symbol.trim().is_empty()
        || client_order_id.trim().is_empty()
        || client_order_id == "0"
        || exchange_order_id.is_some_and(|exchange_order_id| {
            exchange_order_id.trim().is_empty() || exchange_order_id == "0"
        })
    {
        return Err(RegularExecutionPolicyError::InvalidOwnedIdentity);
    }
    Ok(())
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

    fn valid_profile() -> RegularExecutionProfile {
        RegularExecutionProfile {
            symbol: SYMBOL.to_string(),
            account_id: ACCOUNT_ID.to_string(),
            risk_model: InstrumentRiskModel::Spot,
            order_limits: InstrumentOrderLimits {
                max_limit_quantity: 10.0,
                max_limit_notional_usd: Some(1_000.0),
            },
            tick_size: 0.5,
            lot_size: 0.1,
            min_size: 0.2,
            quote_allowed: true,
            hedge_allowed: true,
            quote_stp_verified: true,
        }
    }

    fn test_policy(quote_allowed: bool, hedge_allowed: bool) -> RegularExecutionPolicy {
        let scope =
            RegularApprovalScope::new(ACCOUNT_ID.to_string(), RegularApprovalBinding::new());
        let mut profile = valid_profile();
        profile.quote_allowed = quote_allowed;
        profile.hedge_allowed = hedge_allowed;
        let profiles = scope
            .bind_profiles([profile])
            .expect("test profile must match its approval scope");
        RegularExecutionPolicy::from_profile_sets([profiles])
            .expect("test execution profile must be valid")
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

    fn reserve_quote(
        policy: &RegularExecutionPolicy,
        owned: &mut OwnedRegularOrders,
        client_order_id: &str,
        private_state: &mut PrivateStateReducer,
        ts_ms: TimeMs,
    ) -> Result<(OrderUpdate, ReservedRegularSubmit), RegularExecutionPolicyError> {
        let approved = approved_quote(policy);
        let generated = GeneratedClientOrderId::for_test(client_order_id, approved.binding.clone());
        owned.reserve_local(approved, generated, private_state, ts_ms)
    }

    fn maybe_recovered_submit_proof(
        account_id: &str,
        symbol: &str,
        client_order_id: &str,
    ) -> Option<ProvenRegularSubmitRequest> {
        let record = reap_storage::StorageRecord::OrderRequest(reap_storage::OrderRequestRecord {
            ts_ms: 1,
            account_id: account_id.to_string(),
            operation: reap_storage::OrderOperation::Submit,
            idempotency_key: Some(format!("proof:{account_id}:{client_order_id}")),
            client_order_id: Some(client_order_id.to_string()),
            exchange_order_id: None,
            symbol: symbol.to_string(),
        });
        let mut journal = serde_json::to_vec(&serde_json::json!({
            "schema_version": 7,
            "record": record,
        }))
        .expect("test recovered submit must serialize");
        journal.push(b'\n');
        let directory = tempfile::tempdir().expect("test journal directory must exist");
        let path = directory.path().join("authority-proof.jsonl");
        std::fs::write(&path, journal).expect("test journal must be written");
        let mut lease =
            reap_storage::acquire_storage_lease(&path).expect("test journal must be leased");
        reap_storage::recover_leased_jsonl(&mut lease)
            .expect("test recovered submit must parse under its lease")
            .proven_regular_submit_requests
            .into_values()
            .next()
    }

    fn recovered_submit_proof(
        account_id: &str,
        symbol: &str,
        client_order_id: &str,
    ) -> ProvenRegularSubmitRequest {
        maybe_recovered_submit_proof(account_id, symbol, client_order_id)
            .expect("valid durable submit request must produce an opaque proof")
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
    fn generated_client_order_id_from_another_gateway_scope_is_rejected() {
        let scope =
            RegularApprovalScope::new(ACCOUNT_ID.to_string(), RegularApprovalBinding::new());
        let (profiles, own_generator) = scope
            .bind_profiles_and_client_id_generator([valid_profile()], "reap", 1)
            .unwrap();
        let policy = RegularExecutionPolicy::from_profile_sets([profiles]).unwrap();

        let foreign_scope =
            RegularApprovalScope::new(ACCOUNT_ID.to_string(), RegularApprovalBinding::new());
        let (_foreign_profiles, foreign_generator) = foreign_scope
            .bind_profiles_and_client_id_generator([valid_profile()], "reap", 1)
            .unwrap();
        let foreign_id = foreign_generator.next(1);
        let foreign_id_value = foreign_id.as_str().to_string();
        let mut owned = OwnedRegularOrders::default();
        let mut private_state = PrivateStateReducer::new();

        assert!(matches!(
            owned.reserve_local(
                approved_quote(&policy),
                foreign_id,
                &mut private_state,
                1,
            ),
            Err(RegularExecutionPolicyError::ClientOrderIdScopeMismatch { account_id })
                if account_id == ACCOUNT_ID
        ));
        assert!(
            !private_state
                .order_reducer()
                .contains_order(&foreign_id_value),
            "cross-gateway generated IDs must fail before canonical state mutation"
        );

        let own_id = own_generator.next(2);
        let own_id_value = own_id.as_str().to_string();
        owned
            .reserve_local(approved_quote(&policy), own_id, &mut private_state, 2)
            .expect("the generator from the policy's gateway scope must reserve");
        assert!(private_state.order_reducer().contains_order(&own_id_value));
    }

    #[test]
    fn invalid_public_execution_profiles_are_rejected_before_authority() {
        let cases = vec![
            {
                let mut profile = valid_profile();
                profile.symbol = "  ".to_string();
                (profile, "symbol")
            },
            {
                let mut profile = valid_profile();
                profile.account_id = String::new();
                (profile, "account_id")
            },
            {
                let mut profile = valid_profile();
                profile.tick_size = f64::NAN;
                (profile, "tick_size")
            },
            {
                let mut profile = valid_profile();
                profile.lot_size = 0.0;
                (profile, "lot_size")
            },
            {
                let mut profile = valid_profile();
                profile.min_size = f64::INFINITY;
                (profile, "min_size")
            },
            {
                let mut profile = valid_profile();
                profile.order_limits.max_limit_quantity = f64::NAN;
                (profile, "order_limits")
            },
            {
                let mut profile = valid_profile();
                profile.order_limits.max_limit_notional_usd = Some(-1.0);
                (profile, "order_limits")
            },
            {
                let mut profile = valid_profile();
                profile.risk_model = InstrumentRiskModel::LinearDerivative {
                    contract_value: f64::NAN,
                };
                (profile, "risk_model")
            },
            {
                let mut profile = valid_profile();
                profile.min_size = profile.order_limits.max_limit_quantity + 0.1;
                (profile, "min_size")
            },
        ];

        for (profile, expected_field) in cases {
            assert!(matches!(
                RegularExecutionPolicy::from_profiles_and_profile_sets([profile], []),
                Err(RegularExecutionPolicyError::InvalidProfile { field, .. })
                    if field == expected_field
            ));
        }
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
        profile.profile.order_limits.max_limit_notional_usd = None;

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
        profile.profile.risk_model = InstrumentRiskModel::LinearDerivative {
            contract_value: 2.0,
        };
        profile.profile.order_limits.max_limit_notional_usd = Some(1_000.0);

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
        profile.profile.risk_model = InstrumentRiskModel::InverseDerivative {
            contract_value: 100.0,
        };
        profile.profile.order_limits.max_limit_quantity = 100.0;
        profile.profile.order_limits.max_limit_notional_usd = Some(1_000.0);

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
        let valid = recovered_submit_proof(ACCOUNT_ID, SYMBOL, "recovered-1");

        assert!(policy.lower_recovered_submit(&valid).is_ok());
        let foreign_account = recovered_submit_proof("foreign-account", SYMBOL, "recovered-1");
        assert!(matches!(
            policy.lower_recovered_submit(&foreign_account),
            Err(RegularExecutionPolicyError::OwnerMismatch {
                symbol,
                actual,
                expected,
            }) if symbol == SYMBOL
                && actual == "foreign-account"
                && expected == ACCOUNT_ID
        ));
        let foreign_symbol = recovered_submit_proof(ACCOUNT_ID, "ETH-USDT", "recovered-1");
        assert!(matches!(
            policy.lower_recovered_submit(&foreign_symbol),
            Err(RegularExecutionPolicyError::MissingInstrument { symbol })
                if symbol == "ETH-USDT"
        ));
        assert!(maybe_recovered_submit_proof(ACCOUNT_ID, SYMBOL, "").is_none());
        assert!(maybe_recovered_submit_proof(ACCOUNT_ID, SYMBOL, "0").is_none());
    }

    #[test]
    fn cancel_accepts_only_the_owned_client_id_with_matching_active_canonical_state() {
        let policy = test_policy(true, true);
        let mut owned = OwnedRegularOrders::default();
        let mut state = PrivateStateReducer::new();
        reserve_quote(&policy, &mut owned, "reap-local-1", &mut state, 0)
            .expect("approved local order must establish ownership");
        owned
            .bind_exchange_order_id(ACCOUNT_ID, "reap-local-1", "exchange-42")
            .expect("exchange acknowledgement must bind to owned order");
        let private_states = HashMap::from([(ACCOUNT_ID.to_string(), state)]);

        let cancel = policy
            .authorize_cancel("reap-local-1", "risk_fail_closed", &owned, &private_states)
            .expect("owned active canonical order must be cancellable");
        assert_eq!(cancel.account_id(), ACCOUNT_ID);
        assert_eq!(cancel.symbol(), SYMBOL);
        assert_eq!(cancel.client_order_id(), "reap-local-1");
        assert_eq!(cancel.reason(), "risk_fail_closed");

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

        let mut missing_owned = OwnedRegularOrders::default();
        let mut removed_state = PrivateStateReducer::new();
        reserve_quote(
            &policy,
            &mut missing_owned,
            "missing-canonical",
            &mut removed_state,
            0,
        )
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
        reserve_quote(
            &policy,
            &mut terminal_owned,
            "terminal-order",
            &mut terminal_state,
            0,
        )
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
        let foreign_account =
            recovered_submit_proof("foreign-account", SYMBOL, "foreign-account-order");
        assert!(matches!(
            foreign_account_owned.register_recovered(&policy, foreign_account),
            Err(RegularExecutionPolicyError::OwnerMismatch { actual, expected, .. })
                if actual == "foreign-account" && expected == ACCOUNT_ID
        ));

        let mut foreign_symbol_owned = OwnedRegularOrders::default();
        let foreign_symbol = recovered_submit_proof(ACCOUNT_ID, "ETH-USDT", "foreign-symbol-order");
        assert!(matches!(
            foreign_symbol_owned.register_recovered(&policy, foreign_symbol),
            Err(RegularExecutionPolicyError::MissingInstrument { symbol })
                if symbol == "ETH-USDT"
        ));

        let mut mismatched_owned = OwnedRegularOrders::default();
        let mut proof_state = PrivateStateReducer::new();
        reserve_quote(
            &policy,
            &mut mismatched_owned,
            "canonical-symbol-mismatch",
            &mut proof_state,
            0,
        )
        .unwrap();
        let mut mismatched_state = PrivateStateReducer::new();
        let mut wrong_symbol_order = approved_quote(&policy).order.clone();
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
        let mut owned = OwnedRegularOrders::default();
        let mut first_state = PrivateStateReducer::new();

        reserve_quote(&policy, &mut owned, "owned-1", &mut first_state, 0).unwrap();
        assert!(matches!(
            reserve_quote(&policy, &mut owned, "owned-1", &mut first_state, 1),
            Err(RegularExecutionPolicyError::CanonicalOrderConflict { client_order_id })
                if client_order_id == "owned-1"
        ));
        let mut duplicate_state = PrivateStateReducer::new();
        assert!(matches!(
            reserve_quote(
                &policy,
                &mut owned,
                "owned-1",
                &mut duplicate_state,
                1,
            ),
            Err(RegularExecutionPolicyError::OwnershipConflict { client_order_id })
                if client_order_id == "owned-1"
        ));
        assert!(
            !duplicate_state.order_reducer().contains_order("owned-1"),
            "ownership conflicts must roll back the new canonical reservation"
        );
        let recovered_conflict = recovered_submit_proof(ACCOUNT_ID, SYMBOL, "owned-1");
        assert!(matches!(
            owned.register_recovered(&policy, recovered_conflict),
            Err(RegularExecutionPolicyError::OwnershipConflict { client_order_id })
                if client_order_id == "owned-1"
        ));
        assert!(maybe_recovered_submit_proof(ACCOUNT_ID, SYMBOL, "").is_none());
        assert!(maybe_recovered_submit_proof(ACCOUNT_ID, SYMBOL, "0").is_none());
        assert!(matches!(
            owned.bind_exchange_order_id(ACCOUNT_ID, "owned-1", ""),
            Err(RegularExecutionPolicyError::InvalidOwnedIdentity)
        ));
        assert!(matches!(
            owned.bind_exchange_order_id(ACCOUNT_ID, "owned-1", "0"),
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
