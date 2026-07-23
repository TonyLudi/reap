#![allow(dead_code)]

use reap_pm_core::{PmConditionId, PmMarketId, PmQuantity, PmTick, PmTokenId, U256};
use reap_polymarket_wire::{PmBookParserConfig, PmWireScope};

pub const CONDITION: &str = "0xaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa";
pub const MARKET: &str = "0xbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbb";

pub fn scope() -> PmWireScope {
    PmWireScope::new(
        PmConditionId::parse(CONDITION).unwrap(),
        PmMarketId::parse(MARKET).unwrap(),
        PmTokenId::new(U256::from_u64(123)).unwrap(),
    )
}

pub fn book_config() -> PmBookParserConfig {
    PmBookParserConfig::new(
        scope(),
        PmTick::parse_decimal("0.01").unwrap(),
        PmQuantity::parse_decimal("5").unwrap(),
        false,
    )
}

pub fn snapshot_json(hash: &str) -> String {
    snapshot_json_with_last_trade(hash, "0.50")
}

pub fn snapshot_json_with_last_trade(hash: &str, last_trade_price: &str) -> String {
    format!(
        r#"{{
          "event_type":"book",
          "market":"{MARKET}",
          "asset_id":"123",
          "timestamp":"123456789",
          "hash":"{hash}",
          "bids":[{{"price":"0.30","size":"100"}},{{"price":"0.40","size":"50"}}],
          "asks":[{{"price":"0.60","size":"75"}},{{"price":"0.70","size":"100"}}],
          "min_order_size":"5",
          "tick_size":"0.01",
          "neg_risk":false,
          "last_trade_price":"{last_trade_price}"
        }}"#
    )
}
