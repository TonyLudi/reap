use serde::Serialize;
use sha1::{Digest, Sha1};

use crate::exact::ExactText;
use crate::limits::{MAX_BOOK_LEVELS, MAX_PUBLIC_WS_FRAME_BYTES};
use crate::raw::{RawBook, RawBookLevel};
use crate::{PmWireError, SnapshotHash};

pub fn compute_snapshot_hash(raw: &[u8]) -> Result<SnapshotHash, PmWireError> {
    if raw.len() > MAX_PUBLIC_WS_FRAME_BYTES {
        return Err(PmWireError::PayloadTooLarge);
    }
    let book = serde_json::from_slice::<RawBook>(raw).map_err(|_| PmWireError::MalformedJson)?;
    compute_raw_snapshot_hash(&book)
}

pub fn verify_snapshot_hash(raw: &[u8]) -> Result<SnapshotHash, PmWireError> {
    if raw.len() > MAX_PUBLIC_WS_FRAME_BYTES {
        return Err(PmWireError::PayloadTooLarge);
    }
    let book = serde_json::from_slice::<RawBook>(raw).map_err(|_| PmWireError::MalformedJson)?;
    verify_raw_snapshot_hash(&book)
}

pub(crate) fn verify_raw_snapshot_hash(book: &RawBook) -> Result<SnapshotHash, PmWireError> {
    let expected = SnapshotHash::parse_hex(required(&book.hash, "hash")?)?;
    let actual = compute_raw_snapshot_hash(book)?;
    if expected != actual {
        return Err(PmWireError::SnapshotHashMismatch { expected, actual });
    }
    Ok(actual)
}

pub(crate) fn compute_raw_snapshot_hash(book: &RawBook) -> Result<SnapshotHash, PmWireError> {
    let market = checked_exact(required(&book.market, "market")?, "market")?;
    let asset_id = checked_exact(required(&book.asset_id, "asset_id")?, "asset_id")?;
    let timestamp = checked_exact(required(&book.timestamp, "timestamp")?, "timestamp")?;
    let minimum = checked_exact(
        required(&book.min_order_size, "min_order_size")?,
        "min_order_size",
    )?;
    let tick = checked_exact(required(&book.tick_size, "tick_size")?, "tick_size")?;
    let last_trade = checked_exact(
        required(&book.last_trade_price, "last_trade_price")?,
        "last_trade_price",
    )?;
    let negative_risk = book.neg_risk.ok_or(PmWireError::MissingField("neg_risk"))?;
    let bids = book
        .bids
        .as_deref()
        .ok_or(PmWireError::MissingField("bids"))?;
    let asks = book
        .asks
        .as_deref()
        .ok_or(PmWireError::MissingField("asks"))?;
    if bids.len().saturating_add(asks.len()) > MAX_BOOK_LEVELS {
        return Err(PmWireError::TooManyBookLevels);
    }

    let bid_levels = hash_levels(bids)?;
    let ask_levels = hash_levels(asks)?;
    let payload = HashPayload {
        market: market.as_str(),
        asset_id: asset_id.as_str(),
        timestamp: timestamp.as_str(),
        hash: "",
        bids: bid_levels,
        asks: ask_levels,
        min_order_size: minimum.as_str(),
        tick_size: tick.as_str(),
        neg_risk: negative_risk,
        last_trade_price: last_trade.as_str(),
    };
    let encoded = serde_json::to_vec(&payload).map_err(|_| PmWireError::Serialization)?;
    let digest: [u8; 20] = Sha1::digest(encoded).into();
    Ok(SnapshotHash::from_bytes(digest))
}

fn hash_levels(levels: &[RawBookLevel]) -> Result<Vec<HashLevel<'_>>, PmWireError> {
    levels
        .iter()
        .map(|level| {
            let price = required(&level.price, "price")?;
            let size = required(&level.size, "size")?;
            checked_exact(price, "price")?;
            checked_exact(size, "size")?;
            Ok(HashLevel { price, size })
        })
        .collect()
}

fn checked_exact(value: &str, field: &'static str) -> Result<ExactText, PmWireError> {
    ExactText::new(field, value)
}

pub(crate) fn required<'a>(
    value: &'a Option<String>,
    field: &'static str,
) -> Result<&'a str, PmWireError> {
    value.as_deref().ok_or(PmWireError::MissingField(field))
}

#[derive(Serialize)]
struct HashPayload<'a> {
    market: &'a str,
    asset_id: &'a str,
    timestamp: &'a str,
    hash: &'static str,
    bids: Vec<HashLevel<'a>>,
    asks: Vec<HashLevel<'a>>,
    min_order_size: &'a str,
    tick_size: &'a str,
    neg_risk: bool,
    last_trade_price: &'a str,
}

#[derive(Serialize)]
struct HashLevel<'a> {
    price: &'a str,
    size: &'a str,
}
