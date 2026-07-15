use std::collections::BTreeMap;
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

#[derive(Debug, Clone)]
pub struct TimedReplayEvent {
    pub capture_session_id: Option<String>,
    pub capture_record_seq: Option<u64>,
    pub recv_ts_ns: u64,
    pub event: NormalizedEvent,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct RawCaptureRecordRange {
    pub first: u64,
    pub last: u64,
}

impl RawCaptureRecordRange {
    pub fn validate(self) -> Result<()> {
        if self.first == 0 || self.last < self.first {
            bail!(
                "raw capture record range must satisfy 1 <= first <= last, received {}..={}",
                self.first,
                self.last
            );
        }
        Ok(())
    }

    fn len(self) -> u64 {
        self.last - self.first + 1
    }

    fn contains(self, sequence: u64) -> bool {
        (self.first..=self.last).contains(&sequence)
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct RawReplayBoundary {
    pub capture_session_id: String,
    pub first_capture_record_seq: u64,
    pub last_capture_record_seq: u64,
    pub raw_records: u64,
    pub first_recv_ts_ns: u64,
    pub last_recv_ts_ns: u64,
    pub maximum_recv_ts_ns: u64,
}

impl RawReplayBoundary {
    pub fn validate(&self) -> Result<()> {
        let expected_records = self
            .last_capture_record_seq
            .checked_sub(self.first_capture_record_seq)
            .and_then(|distance| distance.checked_add(1));
        if self.capture_session_id.trim().is_empty()
            || self.first_capture_record_seq == 0
            || expected_records != Some(self.raw_records)
            || self.raw_records == 0
            || self.maximum_recv_ts_ns < self.first_recv_ts_ns
            || self.maximum_recv_ts_ns < self.last_recv_ts_ns
        {
            bail!("raw replay boundary is invalid");
        }
        Ok(())
    }
}

#[derive(Default)]
struct ReplayWarmState {
    index_prices: BTreeMap<Symbol, NormalizedEvent>,
    funding_rates: BTreeMap<Symbol, NormalizedEvent>,
    price_limits: BTreeMap<Symbol, WarmPriceLimits>,
}

#[derive(Default)]
struct WarmPriceLimits {
    mark: Option<(TimeMs, f64)>,
    limits: Option<(TimeMs, f64, f64)>,
}

impl ReplayWarmState {
    fn observe(&mut self, event: &NormalizedEvent) {
        let NormalizedEvent::Market(event) = event else {
            return;
        };
        match event {
            MarketEvent::IndexPrice { symbol, .. } => {
                self.index_prices
                    .insert(symbol.clone(), event.clone().into());
            }
            MarketEvent::FundingRate { symbol, .. } => {
                self.funding_rates
                    .insert(symbol.clone(), event.clone().into());
            }
            MarketEvent::PriceLimits {
                ts_ms,
                symbol,
                mark_price,
                limit_down,
                limit_up,
            } => {
                let state = self.price_limits.entry(symbol.clone()).or_default();
                if mark_price.is_finite() && *mark_price > 0.0 {
                    state.mark = Some((*ts_ms, *mark_price));
                }
                if (limit_down.is_finite() && *limit_down > 0.0)
                    || (limit_up.is_finite() && *limit_up > 0.0)
                {
                    state.limits = Some((*ts_ms, *limit_down, *limit_up));
                }
            }
            MarketEvent::Depth(_) | MarketEvent::Trade { .. } | MarketEvent::BurstSignal { .. } => {
            }
        }
    }

    fn events(&self) -> Vec<NormalizedEvent> {
        let mut events = self
            .index_prices
            .values()
            .chain(self.funding_rates.values())
            .cloned()
            .collect::<Vec<_>>();
        events.extend(self.price_limits.iter().filter_map(|(symbol, state)| {
            let ts_ms = state
                .mark
                .map(|(ts_ms, _)| ts_ms)
                .into_iter()
                .chain(state.limits.map(|(ts_ms, _, _)| ts_ms))
                .max()?;
            Some(NormalizedEvent::from(MarketEvent::PriceLimits {
                ts_ms,
                symbol: symbol.clone(),
                mark_price: state.mark.map_or(0.0, |(_, mark)| mark),
                limit_down: state.limits.map_or(0.0, |(_, limit, _)| limit),
                limit_up: state.limits.map_or(0.0, |(_, _, limit)| limit),
            }))
        }));
        events
    }
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

pub fn replay_raw_capture_timed_path(
    path: &Path,
    on_event: impl FnMut(TimedReplayEvent) -> Result<()>,
) -> Result<()> {
    let file = File::open(path)?;
    replay_raw_capture_timed(file, on_event)
}

pub fn replay_raw_capture_timed_path_with_boundary(
    path: &Path,
    on_event: impl FnMut(TimedReplayEvent) -> Result<()>,
) -> Result<Option<RawReplayBoundary>> {
    let file = File::open(path)?;
    replay_raw_capture_timed_selected(file, None, on_event)
}

pub fn replay_raw_capture_timed_range_path(
    path: &Path,
    range: RawCaptureRecordRange,
    on_event: impl FnMut(TimedReplayEvent) -> Result<()>,
) -> Result<RawReplayBoundary> {
    let file = File::open(path)?;
    replay_raw_capture_timed_range(file, range, on_event)
}

pub fn replay_raw_capture_timed_range<R: std::io::Read>(
    reader: R,
    range: RawCaptureRecordRange,
    on_event: impl FnMut(TimedReplayEvent) -> Result<()>,
) -> Result<RawReplayBoundary> {
    replay_raw_capture_timed_selected(reader, Some(range), on_event)?
        .context("sequenced raw capture range did not produce a replay boundary")
}

pub fn replay_raw_capture<R: std::io::Read>(
    reader: R,
    mut on_event: impl FnMut(NormalizedEvent) -> Result<()>,
) -> Result<()> {
    replay_raw_capture_timed(reader, |timed| on_event(timed.event))
}

pub fn replay_raw_capture_timed<R: std::io::Read>(
    reader: R,
    on_event: impl FnMut(TimedReplayEvent) -> Result<()>,
) -> Result<()> {
    replay_raw_capture_timed_selected(reader, None, on_event).map(|_| ())
}

fn replay_raw_capture_timed_selected<R: std::io::Read>(
    reader: R,
    range: Option<RawCaptureRecordRange>,
    mut on_event: impl FnMut(TimedReplayEvent) -> Result<()>,
) -> Result<Option<RawReplayBoundary>> {
    if let Some(range) = range {
        range.validate()?;
    }
    let adapter = OkxAdapter::default();
    let mut processor = FeedProcessor::new(100_000, 32_768);
    let mut capture_session: Option<Option<String>> = None;
    let mut last_global_record_seq = None;
    let mut saw_sequenced_record = false;
    let mut saw_legacy_record = false;
    let mut selected_records = 0_u64;
    let mut first_selected_record_seq = None;
    let mut last_selected_record_seq = None;
    let mut first_selected_recv_ts_ns = None;
    let mut last_selected_recv_ts_ns = None;
    let mut maximum_selected_recv_ts_ns = 0_u64;
    let mut selected_output_events = 0_u64;
    let mut seeded_selected_boundary = false;
    let mut warm_state = ReplayWarmState::default();
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
        match capture.capture_record_seq {
            Some(sequence) => {
                if saw_legacy_record {
                    bail!(
                        "raw backtest input mixes sequenced and legacy records at line {line_number}"
                    );
                }
                let expected = last_global_record_seq
                    .and_then(|previous: u64| previous.checked_add(1))
                    .unwrap_or(1);
                if sequence != expected {
                    bail!(
                        "raw backtest input record sequence expected {expected}, received {sequence} at line {line_number}"
                    );
                }
                saw_sequenced_record = true;
                last_global_record_seq = Some(sequence);
            }
            None => {
                if saw_sequenced_record || range.is_some() {
                    bail!("raw backtest input omits capture_record_seq at line {line_number}");
                }
                saw_legacy_record = true;
            }
        }

        let selected = match (range, capture.capture_record_seq) {
            (Some(range), Some(sequence)) => range.contains(sequence),
            (Some(_), None) => false,
            (None, _) => true,
        };
        if !selected
            && range.is_some_and(|range| {
                capture
                    .capture_record_seq
                    .is_some_and(|sequence| sequence > range.last)
            })
        {
            continue;
        }

        if selected {
            selected_records = selected_records.saturating_add(1);
            if let Some(sequence) = capture.capture_record_seq {
                first_selected_record_seq.get_or_insert(sequence);
                last_selected_record_seq = Some(sequence);
            }
            first_selected_recv_ts_ns.get_or_insert(capture.recv_ts_ns);
            last_selected_recv_ts_ns = Some(capture.recv_ts_ns);
            maximum_selected_recv_ts_ns = maximum_selected_recv_ts_ns.max(capture.recv_ts_ns);
        }
        let event_capture_session_id = capture.capture_session_id.clone();
        let event_capture_record_seq = capture.capture_record_seq;
        let recv_ts_ns = capture.recv_ts_ns;
        if selected && range.is_some() && !seeded_selected_boundary {
            for event in warm_state.events() {
                selected_output_events = selected_output_events.saturating_add(1);
                on_event(TimedReplayEvent {
                    capture_session_id: event_capture_session_id.clone(),
                    capture_record_seq: event_capture_record_seq,
                    recv_ts_ns,
                    event,
                })?;
            }
            for book in processor.ready_books() {
                selected_output_events = selected_output_events.saturating_add(1);
                on_event(TimedReplayEvent {
                    capture_session_id: event_capture_session_id.clone(),
                    capture_record_seq: event_capture_record_seq,
                    recv_ts_ns,
                    event: NormalizedEvent::from(MarketEvent::Depth(book)),
                })?;
            }
            seeded_selected_boundary = true;
        }
        let envelope = capture
            .into_envelope()
            .with_context(|| format!("invalid raw capture payload on line {line_number}"))?;
        let parsed = adapter
            .parse(&envelope)
            .with_context(|| format!("failed to parse raw capture line {line_number}"))?;
        for parsed in parsed {
            for output in processor.process_from(&envelope.conn_id, parsed) {
                match output {
                    FeedOutput::Event(event) => {
                        if selected {
                            selected_output_events = selected_output_events.saturating_add(1);
                            on_event(TimedReplayEvent {
                                capture_session_id: event_capture_session_id.clone(),
                                capture_record_seq: event_capture_record_seq,
                                recv_ts_ns,
                                event,
                            })?;
                        } else if range.is_some() {
                            warm_state.observe(&event);
                        }
                    }
                    FeedOutput::System(event) if selected => {
                        selected_output_events = selected_output_events.saturating_add(1);
                        on_event(TimedReplayEvent {
                            capture_session_id: event_capture_session_id.clone(),
                            capture_record_seq: event_capture_record_seq,
                            recv_ts_ns,
                            event: NormalizedEvent::System(event),
                        })?;
                    }
                    FeedOutput::System(_) => {}
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

    if let Some(range) = range
        && (selected_records != range.len()
            || first_selected_record_seq != Some(range.first)
            || last_selected_record_seq != Some(range.last))
    {
        bail!(
            "raw capture range {}..={} is incomplete: records={}, first={:?}, last={:?}",
            range.first,
            range.last,
            selected_records,
            first_selected_record_seq,
            last_selected_record_seq
        );
    }

    let stats = processor.stats();
    if stats.parsed == 0 || selected_output_events == 0 {
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

    if !saw_sequenced_record {
        return Ok(None);
    }
    let capture_session_id = capture_session
        .flatten()
        .filter(|session| !session.is_empty())
        .context("sequenced raw backtest input requires a non-empty capture_session_id")?;
    let boundary = RawReplayBoundary {
        capture_session_id,
        first_capture_record_seq: first_selected_record_seq
            .context("sequenced raw backtest input selected no first record")?,
        last_capture_record_seq: last_selected_record_seq
            .context("sequenced raw backtest input selected no last record")?,
        raw_records: selected_records,
        first_recv_ts_ns: first_selected_recv_ts_ns
            .context("sequenced raw backtest input selected no first receive timestamp")?,
        last_recv_ts_ns: last_selected_recv_ts_ns
            .context("sequenced raw backtest input selected no last receive timestamp")?,
        maximum_recv_ts_ns: maximum_selected_recv_ts_ns,
    };
    boundary.validate()?;
    Ok(Some(boundary))
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
    use std::collections::HashSet;

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
    fn raw_capture_replay_preserves_sequence_reset_and_heartbeat() {
        let fixture = include_str!("../../../fixtures/raw/okx/depth-reset.jsonl");
        let mut depth_events = 0;
        let mut recovery_events = 0;
        replay_raw_capture(fixture.as_bytes(), |event| {
            match event {
                NormalizedEvent::Market(MarketEvent::Depth(_)) => depth_events += 1,
                NormalizedEvent::System(event)
                    if matches!(
                        event.kind,
                        reap_core::SystemEventKind::FeedGap
                            | reap_core::SystemEventKind::BookRecoveryStarted
                    ) =>
                {
                    recovery_events += 1;
                }
                _ => {}
            }
            Ok(())
        })
        .unwrap();

        assert_eq!(depth_events, 4);
        assert_eq!(recovery_events, 0);
    }

    #[test]
    fn timed_raw_replay_exposes_capture_receive_timestamps() {
        let fixture = include_str!("../../../fixtures/raw/okx/depth-gap.jsonl");
        let expected = fixture
            .lines()
            .map(|line| serde_json::from_str::<RawCapture>(line).unwrap().recv_ts_ns)
            .collect::<HashSet<_>>();
        let mut observed = Vec::new();

        replay_raw_capture_timed(fixture.as_bytes(), |timed| {
            observed.push(timed.recv_ts_ns);
            Ok(())
        })
        .unwrap();

        assert!(!observed.is_empty());
        assert!(
            observed
                .iter()
                .all(|recv_ts_ns| expected.contains(recv_ts_ns))
        );
    }

    #[test]
    fn sequenced_raw_replay_reports_exact_full_and_selected_boundaries() {
        let fixture = include_str!("../../../fixtures/raw/okx/depth-reset.jsonl");

        let full = replay_raw_capture_timed_selected(fixture.as_bytes(), None, |_| Ok(()))
            .unwrap()
            .unwrap();
        let selected = replay_raw_capture_timed_range(
            fixture.as_bytes(),
            RawCaptureRecordRange { first: 1, last: 6 },
            |_| Ok(()),
        )
        .unwrap();

        assert_eq!(full.capture_session_id, "reset-session");
        assert_eq!(full.first_capture_record_seq, 1);
        assert_eq!(full.last_capture_record_seq, 7);
        assert_eq!(full.raw_records, 7);
        assert_eq!(selected.capture_session_id, "reset-session");
        assert_eq!(selected.first_capture_record_seq, 1);
        assert_eq!(selected.last_capture_record_seq, 6);
        assert_eq!(selected.raw_records, 6);
        assert_eq!(selected.first_recv_ts_ns, 1_000_000);
        assert_eq!(selected.last_recv_ts_ns, 1_002_001);
        assert_eq!(selected.maximum_recv_ts_ns, 1_002_001);
    }

    #[test]
    fn raw_range_replay_rejects_invalid_ranges_and_warms_selected_deltas() {
        let fixture = include_str!("../../../fixtures/raw/okx/depth-reset.jsonl");

        let invalid = replay_raw_capture_timed_range(
            fixture.as_bytes(),
            RawCaptureRecordRange { first: 0, last: 1 },
            |_| Ok(()),
        )
        .unwrap_err();
        let incomplete = replay_raw_capture_timed_range(
            fixture.as_bytes(),
            RawCaptureRecordRange { first: 1, last: 8 },
            |_| Ok(()),
        )
        .unwrap_err();
        let mut events = Vec::new();
        let warmed = replay_raw_capture_timed_range(
            fixture.as_bytes(),
            RawCaptureRecordRange { first: 7, last: 7 },
            |event| {
                events.push(event);
                Ok(())
            },
        )
        .unwrap();

        assert!(invalid.to_string().contains("1 <= first <= last"));
        assert!(
            incomplete.to_string().contains("range 1..=8 is incomplete")
                || incomplete
                    .to_string()
                    .contains("produced no normalized market events")
        );
        assert_eq!(warmed.first_capture_record_seq, 7);
        assert_eq!(warmed.last_capture_record_seq, 7);
        assert_eq!(
            events
                .iter()
                .filter(|event| matches!(
                    event.event,
                    NormalizedEvent::Market(MarketEvent::Depth(_))
                ))
                .count(),
            2
        );
        assert!(
            events
                .iter()
                .all(|event| event.capture_record_seq == Some(7))
        );
    }

    #[test]
    fn raw_range_replay_warms_stateful_reference_channels() {
        let records = [
            (
                "index-tickers",
                "BTC-USDT",
                serde_json::json!({
                    "arg": {"channel": "index-tickers", "instId": "BTC-USDT"},
                    "data": [{"instId": "BTC-USDT", "idxPx": "50000", "ts": "1000"}]
                }),
            ),
            (
                "funding-rate",
                "BTC-USDT-SWAP",
                serde_json::json!({
                    "arg": {"channel": "funding-rate", "instId": "BTC-USDT-SWAP"},
                    "data": [{
                        "instId": "BTC-USDT-SWAP",
                        "fundingRate": "0.0001",
                        "fundingTime": "2000",
                        "ts": "1001"
                    }]
                }),
            ),
            (
                "price-limit",
                "BTC-USDT-SWAP",
                serde_json::json!({
                    "arg": {"channel": "price-limit", "instId": "BTC-USDT-SWAP"},
                    "data": [{
                        "instId": "BTC-USDT-SWAP",
                        "buyLmt": "55000",
                        "sellLmt": "45000",
                        "ts": "1002"
                    }]
                }),
            ),
            (
                "mark-price",
                "BTC-USDT-SWAP",
                serde_json::json!({
                    "arg": {"channel": "mark-price", "instId": "BTC-USDT-SWAP"},
                    "data": [{
                        "instId": "BTC-USDT-SWAP",
                        "markPx": "50010",
                        "ts": "1003"
                    }]
                }),
            ),
            (
                "books",
                "BTC-USDT-SWAP",
                serde_json::json!({
                    "arg": {"channel": "books", "instId": "BTC-USDT-SWAP"},
                    "action": "snapshot",
                    "data": [{
                        "asks": [["50001", "2", "0", "1"]],
                        "bids": [["49999", "3", "0", "1"]],
                        "ts": "1004",
                        "prevSeqId": -1,
                        "seqId": 1
                    }]
                }),
            ),
            (
                "books",
                "BTC-USDT-SWAP",
                serde_json::json!({
                    "arg": {"channel": "books", "instId": "BTC-USDT-SWAP"},
                    "action": "update",
                    "data": [{
                        "asks": [],
                        "bids": [["50000", "2", "0", "1"]],
                        "ts": "1005",
                        "prevSeqId": 1,
                        "seqId": 2
                    }]
                }),
            ),
        ];
        let mut fixture = String::new();
        for (index, (channel, symbol, payload)) in records.into_iter().enumerate() {
            let capture_channel = if channel == "books" {
                serde_json::json!("books")
            } else {
                serde_json::json!({"custom": channel})
            };
            let record = serde_json::json!({
                "capture_session_id": "reference-session",
                "capture_record_seq": index + 1,
                "venue": "okx",
                "conn_id": "primary",
                "channel": capture_channel,
                "symbol": symbol,
                "recv_ts_ns": 1_000_000 + index as u64,
                "payload": payload
            });
            fixture.push_str(&serde_json::to_string(&record).unwrap());
            fixture.push('\n');
        }
        let mut events = Vec::new();

        replay_raw_capture_timed_range(
            fixture.as_bytes(),
            RawCaptureRecordRange { first: 6, last: 6 },
            |event| {
                events.push(event);
                Ok(())
            },
        )
        .unwrap();

        assert!(events.iter().any(|event| matches!(
            &event.event,
            NormalizedEvent::Market(MarketEvent::IndexPrice { symbol, price, .. })
                if symbol == "BTC-USDT" && *price == 50_000.0
        )));
        assert!(events.iter().any(|event| matches!(
            &event.event,
            NormalizedEvent::Market(MarketEvent::FundingRate { symbol, rate, .. })
                if symbol == "BTC-USDT-SWAP" && *rate == 0.0001
        )));
        assert!(events.iter().any(|event| matches!(
            &event.event,
            NormalizedEvent::Market(MarketEvent::PriceLimits {
                symbol,
                mark_price,
                limit_down,
                limit_up,
                ..
            }) if symbol == "BTC-USDT-SWAP"
                && *mark_price == 50_010.0
                && *limit_down == 45_000.0
                && *limit_up == 55_000.0
        )));
        assert_eq!(
            events
                .iter()
                .filter(|event| matches!(
                    event.event,
                    NormalizedEvent::Market(MarketEvent::Depth(_))
                ))
                .count(),
            2
        );
        assert!(
            events
                .iter()
                .all(|event| event.capture_record_seq == Some(6))
        );
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
