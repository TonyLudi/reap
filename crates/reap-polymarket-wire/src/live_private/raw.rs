//! Serde-only wire shapes for authenticated Polymarket private responses.

use std::{collections::BTreeMap, fmt};

use serde::{Deserialize, Deserializer, de::Visitor};
use zeroize::Zeroizing;

use super::associate_trades_wire::RawAssociateTrades;

pub(super) struct RawCredentialOwner(pub(super) Zeroizing<Vec<u8>>);

struct RawCredentialOwnerVisitor;

impl<'de> Visitor<'de> for RawCredentialOwnerVisitor {
    type Value = RawCredentialOwner;

    fn expecting(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str("a credential-owner UUID string")
    }

    fn visit_str<E>(self, value: &str) -> Result<Self::Value, E>
    where
        E: serde::de::Error,
    {
        Ok(RawCredentialOwner(Zeroizing::new(
            value.as_bytes().to_vec(),
        )))
    }

    fn visit_string<E>(self, value: String) -> Result<Self::Value, E>
    where
        E: serde::de::Error,
    {
        Ok(RawCredentialOwner(Zeroizing::new(value.into_bytes())))
    }
}

impl<'de> Deserialize<'de> for RawCredentialOwner {
    fn deserialize<D>(deserializer: D) -> Result<Self, D::Error>
    where
        D: Deserializer<'de>,
    {
        deserializer.deserialize_string(RawCredentialOwnerVisitor)
    }
}

#[derive(Deserialize)]
#[serde(untagged)]
pub(super) enum RawExactU64 {
    Text(String),
    Integer(u64),
}

#[derive(Deserialize)]
#[serde(untagged)]
pub(super) enum RawUserFrame {
    One(Box<RawUserEvent>),
    Many(Vec<RawUserEvent>),
}

#[derive(Deserialize)]
#[serde(tag = "event_type")]
pub(super) enum RawUserEvent {
    #[serde(rename = "order")]
    Order(RawUserOrder),
    #[serde(rename = "trade")]
    Trade(RawUserTrade),
}

#[derive(Deserialize)]
pub(super) struct RawUserOrder {
    pub(super) id: String,
    pub(super) market: String,
    pub(super) asset_id: String,
    pub(super) side: String,
    pub(super) original_size: String,
    pub(super) size_matched: String,
    pub(super) price: String,
    #[serde(rename = "type")]
    pub(super) event_kind: String,
    #[serde(default)]
    pub(super) maker_address: Option<String>,
    #[serde(default)]
    pub(super) expiration: Option<String>,
    #[serde(default)]
    pub(super) order_type: Option<String>,
    #[serde(default)]
    pub(super) outcome: Option<String>,
    #[serde(default)]
    pub(super) status: Option<String>,
    #[serde(default)]
    pub(super) created_at: Option<String>,
    #[serde(default)]
    pub(super) associate_trades: Option<RawAssociateTrades>,
    pub(super) owner: RawCredentialOwner,
    #[serde(default)]
    pub(super) order_owner: Option<RawCredentialOwner>,
    #[serde(default)]
    pub(super) timestamp: Option<String>,
}

#[derive(Deserialize)]
pub(super) struct RawUserTrade {
    #[serde(flatten)]
    pub(super) trade: RawTradeCore,
    pub(super) owner: RawCredentialOwner,
    #[serde(default)]
    pub(super) trade_owner: Option<RawCredentialOwner>,
    #[serde(default)]
    pub(super) timestamp: Option<String>,
    #[serde(default)]
    pub(super) last_update: Option<String>,
    #[serde(default, rename = "matchtime", alias = "match_time")]
    pub(super) match_time: Option<String>,
    #[serde(default)]
    pub(super) maker_address: Option<String>,
}

#[derive(Deserialize)]
pub(super) struct RawOrder {
    pub(super) id: String,
    pub(super) market: String,
    pub(super) asset_id: String,
    pub(super) side: String,
    pub(super) original_size: String,
    pub(super) size_matched: String,
    pub(super) price: String,
    pub(super) status: String,
    pub(super) maker_address: String,
    pub(super) owner: RawCredentialOwner,
    pub(super) expiration: String,
    pub(super) created_at: RawExactU64,
    #[serde(default)]
    pub(super) outcome: Option<String>,
    #[serde(default)]
    pub(super) order_type: Option<String>,
}

#[derive(Deserialize)]
pub(super) struct RawTradeCore {
    pub(super) id: String,
    pub(super) market: String,
    pub(super) asset_id: String,
    pub(super) side: String,
    pub(super) size: String,
    pub(super) price: String,
    pub(super) status: String,
    #[serde(default)]
    pub(super) order_id: Option<String>,
    #[serde(default)]
    pub(super) taker_order_id: Option<String>,
    #[serde(default)]
    pub(super) trader_side: Option<String>,
    #[serde(default)]
    pub(super) transaction_hash: Option<String>,
    #[serde(default)]
    pub(super) fee_rate_bps: Option<String>,
    #[serde(default)]
    pub(super) maker_orders: Vec<RawMakerOrder>,
}

#[derive(Deserialize)]
pub(super) struct RawRestTrade {
    #[serde(flatten)]
    pub(super) trade: RawTradeCore,
    pub(super) maker_address: String,
    pub(super) owner: RawCredentialOwner,
    pub(super) match_time: String,
    pub(super) last_update: String,
}

#[derive(Deserialize)]
pub(super) struct RawMakerOrder {
    pub(super) order_id: String,
    pub(super) asset_id: String,
    pub(super) side: String,
    pub(super) price: String,
    pub(super) matched_amount: String,
    #[serde(default)]
    pub(super) fee_rate_bps: Option<String>,
    pub(super) owner: RawCredentialOwner,
    pub(super) maker_address: String,
}

#[derive(Deserialize)]
#[serde(bound(deserialize = "T: Deserialize<'de>"))]
pub(super) struct RawPage<T> {
    pub(super) data: Vec<T>,
    pub(super) next_cursor: String,
    pub(super) limit: usize,
    pub(super) count: usize,
}

#[derive(Deserialize)]
pub(super) struct RawBalanceAllowance {
    pub(super) balance: String,
    #[serde(default)]
    pub(super) allowance: Option<serde_json::Value>,
    #[serde(default)]
    pub(super) allowances: Option<BTreeMap<String, String>>,
}
