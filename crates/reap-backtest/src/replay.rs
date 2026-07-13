use std::fs::File;
use std::io::{BufRead, BufReader};
use std::path::Path;

use anyhow::{Context, Result, bail};
use reap_book::BookStatus;
use reap_feed::{FeedOutput, FeedProcessor, RawCapture, SequenceStatus};
use reap_venue::{VenueAdapter, okx::OkxAdapter};
use serde::{Deserialize, Serialize};

use reap_core::{Level, MarketEvent, NormalizedEvent, OrderBook, Side, Symbol, TimeMs};

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

pub fn load_events_from_path(path: &Path) -> Result<Vec<NormalizedEvent>> {
    let file = File::open(path)?;
    load_events(file)
}

pub fn load_normalized_jsonl_from_path(path: &Path) -> Result<Vec<NormalizedEvent>> {
    let file = File::open(path)?;
    load_normalized_jsonl(file)
}

pub fn load_normalized_jsonl<R: std::io::Read>(reader: R) -> Result<Vec<NormalizedEvent>> {
    let mut events = Vec::new();
    for line in BufReader::new(reader).lines() {
        let line = line?;
        let trimmed = line.trim();
        if trimmed.is_empty() || trimmed.starts_with('#') {
            continue;
        }
        events.push(serde_json::from_str(trimmed)?);
    }
    Ok(events)
}

pub fn replay_raw_capture_path(
    path: &Path,
    on_event: impl FnMut(NormalizedEvent) -> Result<()>,
) -> Result<()> {
    let file = File::open(path)?;
    replay_raw_capture(file, on_event)
}

pub fn replay_raw_capture<R: std::io::Read>(
    reader: R,
    mut on_event: impl FnMut(NormalizedEvent) -> Result<()>,
) -> Result<()> {
    let adapter = OkxAdapter::default();
    let mut processor = FeedProcessor::new(100_000, 32_768);
    let mut capture_session: Option<Option<String>> = None;
    for (index, line) in BufReader::new(reader).lines().enumerate() {
        let line_number = index + 1;
        let line = line?;
        let trimmed = line.trim();
        if trimmed.is_empty() || trimmed.starts_with('#') {
            continue;
        }
        let capture: RawCapture = serde_json::from_str(trimmed)
            .with_context(|| format!("invalid raw capture record on line {line_number}"))?;
        match &capture_session {
            Some(expected) if expected != &capture.capture_session_id => {
                bail!(
                    "raw backtest input contains multiple capture sessions at line {line_number}; split the file at capture_session_id boundaries"
                )
            }
            None => capture_session = Some(capture.capture_session_id.clone()),
            Some(_) => {}
        }
        let envelope = capture
            .into_envelope()
            .with_context(|| format!("invalid raw capture payload on line {line_number}"))?;
        let parsed = adapter
            .parse(&envelope)
            .with_context(|| format!("failed to parse raw capture line {line_number}"))?;
        for parsed in parsed {
            for output in processor.process(parsed) {
                match output {
                    FeedOutput::Event(event) => on_event(event)?,
                    FeedOutput::System(event) => on_event(NormalizedEvent::System(event))?,
                    FeedOutput::Duplicate(_) | FeedOutput::RecoveryRequired(_) => {}
                    FeedOutput::PrivateOrder { .. }
                    | FeedOutput::PrivateFill { .. }
                    | FeedOutput::PrivateAccount { .. } => {
                        bail!(
                            "raw backtest input contains private account data on line {line_number}"
                        )
                    }
                }
            }
        }
    }

    let stats = processor.stats();
    if stats.parsed == 0 || stats.normalized_events == 0 {
        bail!("raw backtest input produced no normalized market events");
    }
    if stats.recovery_failures > 0 {
        bail!(
            "raw backtest input contains {} failed book recoveries",
            stats.recovery_failures
        );
    }
    let unhealthy = processor
        .stream_health()
        .into_iter()
        .filter(|health| {
            health.sequence_status != SequenceStatus::Ready
                || health.book_status != BookStatus::Ready
        })
        .map(|health| health.stream.symbol)
        .collect::<Vec<_>>();
    if !unhealthy.is_empty() {
        bail!(
            "raw backtest input ended with unhealthy books: {}",
            unhealthy.join(", ")
        );
    }
    Ok(())
}

pub fn load_events<R: std::io::Read>(reader: R) -> Result<Vec<NormalizedEvent>> {
    let mut csv = csv::Reader::from_reader(reader);
    let mut events = Vec::new();
    for row in csv.deserialize::<ReplayRow>() {
        let row = row?;
        if let (Some(bid_px), Some(bid_qty), Some(ask_px), Some(ask_qty)) =
            (row.bid_px, row.bid_qty, row.ask_px, row.ask_qty)
        {
            events.push(NormalizedEvent::from(MarketEvent::Depth(
                OrderBook::one_level(
                    row.symbol.clone(),
                    row.ts_ms,
                    Level::new(bid_px, bid_qty),
                    Level::new(ask_px, ask_qty),
                ),
            )));
        }
        if let (Some(price), Some(qty), Some(taker_side)) =
            (row.trade_px, row.trade_qty, row.taker_side)
        {
            events.push(NormalizedEvent::from(MarketEvent::Trade {
                ts_ms: row.ts_ms,
                symbol: row.symbol,
                price,
                qty,
                taker_side,
            }));
        }
    }
    events.sort_by_key(NormalizedEvent::ts_ms);
    Ok(events)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn raw_capture_replay_deduplicates_and_recovers_books() {
        let fixture = include_str!("../../../fixtures/raw/okx/depth-gap.jsonl");
        let mut depth_events = 0;
        replay_raw_capture(fixture.as_bytes(), |event| {
            if matches!(event, NormalizedEvent::Market(MarketEvent::Depth(_))) {
                depth_events += 1;
            }
            Ok(())
        })
        .unwrap();

        assert_eq!(depth_events, 3);
    }

    #[test]
    fn raw_capture_replay_rejects_process_session_boundaries() {
        let first_line = include_str!("../../../fixtures/raw/okx/depth-gap.jsonl")
            .lines()
            .next()
            .unwrap();
        let mut first: RawCapture = serde_json::from_str(first_line).unwrap();
        first.capture_session_id = Some("session-a".to_string());
        let mut second = first.clone();
        second.capture_session_id = Some("session-b".to_string());
        let input = format!(
            "{}\n{}\n",
            serde_json::to_string(&first).unwrap(),
            serde_json::to_string(&second).unwrap()
        );

        let error = replay_raw_capture(input.as_bytes(), |_| Ok(())).unwrap_err();

        assert!(error.to_string().contains("multiple capture sessions"));
    }
}
