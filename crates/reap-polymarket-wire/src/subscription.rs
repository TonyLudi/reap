use reap_pm_core::PmTokenId;
use serde::Serialize;

use crate::PmWireError;

/// Exact public market subscription for one configured outcome token.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct PmMarketSubscription {
    token: PmTokenId,
}

impl PmMarketSubscription {
    #[must_use]
    pub const fn new(token: PmTokenId) -> Self {
        Self { token }
    }

    #[must_use]
    pub const fn token(self) -> PmTokenId {
        self.token
    }

    pub fn to_json(self) -> Result<String, PmWireError> {
        let token = self.token.units().to_string();
        let wire = MarketSubscriptionWire {
            assets_ids: [token.as_str()],
            custom_feature_enabled: true,
            initial_dump: true,
            operation: "subscribe",
            message_type: "market",
        };
        serde_json::to_string(&wire).map_err(|_| PmWireError::Serialization)
    }
}

#[derive(Serialize)]
struct MarketSubscriptionWire<'a> {
    assets_ids: [&'a str; 1],
    custom_feature_enabled: bool,
    initial_dump: bool,
    operation: &'static str,
    #[serde(rename = "type")]
    message_type: &'static str,
}
