use reap_pm_core::{PmConditionId, PmMarketId, PmQuantity, PmTick, PmTokenId};

/// Exact structural scope for one configured Polymarket outcome.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct PmWireScope {
    condition: PmConditionId,
    market: PmMarketId,
    token: PmTokenId,
}

impl PmWireScope {
    #[must_use]
    pub const fn new(condition: PmConditionId, market: PmMarketId, token: PmTokenId) -> Self {
        Self {
            condition,
            market,
            token,
        }
    }

    #[must_use]
    pub const fn condition(self) -> PmConditionId {
        self.condition
    }

    #[must_use]
    pub const fn market(self) -> PmMarketId {
        self.market
    }

    #[must_use]
    pub const fn token(self) -> PmTokenId {
        self.token
    }
}

/// Metadata-bound parser configuration for one public book connection.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct PmBookParserConfig {
    scope: PmWireScope,
    market_binding: PmBookMarketBinding,
    tick: PmTick,
    minimum_order_size: PmQuantity,
    negative_risk: bool,
}

/// Identity carried by the public book and market-WebSocket `market` field.
///
/// Goal F fixtures used the question/market ID. The current live CLOB routes
/// carry the condition ID instead. Keeping this discriminator in the parser
/// configuration prevents two distinct 32-byte identities from being treated
/// as interchangeable.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum PmBookMarketBinding {
    LegacyMarketId,
    ConditionId,
}

impl PmBookParserConfig {
    #[must_use]
    pub const fn new(
        scope: PmWireScope,
        tick: PmTick,
        minimum_order_size: PmQuantity,
        negative_risk: bool,
    ) -> Self {
        Self {
            scope,
            market_binding: PmBookMarketBinding::LegacyMarketId,
            tick,
            minimum_order_size,
            negative_risk,
        }
    }

    #[must_use]
    pub const fn new_condition_bound(
        scope: PmWireScope,
        tick: PmTick,
        minimum_order_size: PmQuantity,
        negative_risk: bool,
    ) -> Self {
        Self {
            scope,
            market_binding: PmBookMarketBinding::ConditionId,
            tick,
            minimum_order_size,
            negative_risk,
        }
    }

    #[must_use]
    pub const fn scope(self) -> PmWireScope {
        self.scope
    }

    #[must_use]
    pub const fn market_binding(self) -> PmBookMarketBinding {
        self.market_binding
    }

    #[must_use]
    pub const fn tick(self) -> PmTick {
        self.tick
    }

    #[must_use]
    pub const fn minimum_order_size(self) -> PmQuantity {
        self.minimum_order_size
    }

    #[must_use]
    pub const fn negative_risk(self) -> bool {
        self.negative_risk
    }
}
