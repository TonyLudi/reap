use serde::Deserialize;

#[derive(Debug, Clone, Deserialize)]
pub(crate) struct RawBookLevel {
    #[serde(default)]
    pub(crate) price: Option<String>,
    #[serde(default)]
    pub(crate) size: Option<String>,
}

#[derive(Debug, Clone, Deserialize)]
pub(crate) struct RawBook {
    #[serde(default)]
    pub(crate) event_type: Option<String>,
    #[serde(default)]
    pub(crate) market: Option<String>,
    #[serde(default)]
    pub(crate) asset_id: Option<String>,
    #[serde(default)]
    pub(crate) timestamp: Option<String>,
    #[serde(default)]
    pub(crate) hash: Option<String>,
    #[serde(default)]
    pub(crate) bids: Option<Vec<RawBookLevel>>,
    #[serde(default)]
    pub(crate) asks: Option<Vec<RawBookLevel>>,
    #[serde(default)]
    pub(crate) min_order_size: Option<String>,
    #[serde(default)]
    pub(crate) tick_size: Option<String>,
    #[serde(default)]
    pub(crate) neg_risk: Option<bool>,
    #[serde(default)]
    pub(crate) last_trade_price: Option<String>,
}

#[derive(Debug, Clone, Deserialize)]
pub(crate) struct RawPriceChange {
    #[serde(default)]
    pub(crate) asset_id: Option<String>,
    #[serde(default)]
    pub(crate) price: Option<String>,
    #[serde(default)]
    pub(crate) size: Option<String>,
    #[serde(default)]
    pub(crate) side: Option<String>,
    #[serde(default)]
    pub(crate) hash: Option<String>,
    #[serde(default)]
    pub(crate) best_bid: Option<String>,
    #[serde(default)]
    pub(crate) best_ask: Option<String>,
}

#[derive(Debug, Clone, Deserialize)]
pub(crate) struct RawPriceChangeEvent {
    #[serde(default)]
    pub(crate) market: Option<String>,
    #[serde(default)]
    pub(crate) timestamp: Option<String>,
    #[serde(default)]
    pub(crate) price_changes: Option<Vec<RawPriceChange>>,
}

#[derive(Debug, Clone, Deserialize)]
pub(crate) struct RawBestBidAskEvent {
    #[serde(default)]
    pub(crate) market: Option<String>,
    #[serde(default)]
    pub(crate) asset_id: Option<String>,
    #[serde(default)]
    pub(crate) timestamp: Option<String>,
    #[serde(default)]
    pub(crate) best_bid: Option<String>,
    #[serde(default)]
    pub(crate) best_ask: Option<String>,
    #[serde(default)]
    pub(crate) bid_size: Option<String>,
    #[serde(default)]
    pub(crate) ask_size: Option<String>,
}

#[derive(Debug, Clone, Deserialize)]
pub(crate) struct RawTickSizeChangeEvent {
    #[serde(default)]
    pub(crate) market: Option<String>,
    #[serde(default)]
    pub(crate) asset_id: Option<String>,
    #[serde(default)]
    pub(crate) timestamp: Option<String>,
    #[serde(default)]
    pub(crate) old_tick_size: Option<String>,
    #[serde(default)]
    pub(crate) new_tick_size: Option<String>,
}
