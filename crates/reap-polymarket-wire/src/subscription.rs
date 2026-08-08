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
        // Current CLOB market-channel authority (revalidated for PM-T1):
        // https://docs.polymarket.com/market-data/websocket/market-channel
        //
        // `initial_dump` and `operation` belonged to an older client shape.
        // The initial subscription is now exactly the configured asset list,
        // market channel type, and feature flag.
        let wire = MarketSubscriptionWire {
            assets_ids: [token.as_str()],
            custom_feature_enabled: true,
            message_type: "market",
        };
        serde_json::to_string(&wire).map_err(|_| PmWireError::Serialization)
    }
}

#[derive(Serialize)]
struct MarketSubscriptionWire<'a> {
    assets_ids: [&'a str; 1],
    custom_feature_enabled: bool,
    #[serde(rename = "type")]
    message_type: &'static str,
}
