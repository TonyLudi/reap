use std::fs::File;
use std::path::Path;

use anyhow::Result;
use serde::{Deserialize, Serialize};

use crate::types::{Level, MarketEvent, OrderBook, Side, Symbol, TimeMs};

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ReplayRow {
    pub ts_ms: TimeMs,
    pub symbol: Symbol,
    pub bid_px: Option<f64>,
    pub bid_qty: Option<f64>,
    pub ask_px: Option<f64>,
    pub ask_qty: Option<f64>,
    pub trade_px: Option<f64>,
    pub trade_qty: Option<f64>,
    pub taker_side: Option<Side>,
}

pub fn load_events_from_path(path: &Path) -> Result<Vec<MarketEvent>> {
    let file = File::open(path)?;
    load_events(file)
}

pub fn load_events<R: std::io::Read>(reader: R) -> Result<Vec<MarketEvent>> {
    let mut csv = csv::Reader::from_reader(reader);
    let mut events = Vec::new();
    for row in csv.deserialize::<ReplayRow>() {
        let row = row?;
        if let (Some(bid_px), Some(bid_qty), Some(ask_px), Some(ask_qty)) =
            (row.bid_px, row.bid_qty, row.ask_px, row.ask_qty)
        {
            events.push(MarketEvent::Depth(OrderBook::one_level(
                row.symbol.clone(),
                row.ts_ms,
                Level::new(bid_px, bid_qty),
                Level::new(ask_px, ask_qty),
            )));
        }
        if let (Some(price), Some(qty), Some(taker_side)) =
            (row.trade_px, row.trade_qty, row.taker_side)
        {
            events.push(MarketEvent::Trade {
                ts_ms: row.ts_ms,
                symbol: row.symbol,
                price,
                qty,
                taker_side,
            });
        }
    }
    events.sort_by_key(|event| match event {
        MarketEvent::Depth(book) => book.ts_ms,
        MarketEvent::Trade { ts_ms, .. } => *ts_ms,
    });
    Ok(events)
}
