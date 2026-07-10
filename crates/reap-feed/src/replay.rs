use std::fs::File;
use std::io::{BufRead, BufReader, Read};
use std::path::Path;

use anyhow::{Context, Result};
use reap_core::{Channel, ConnId, RawEnvelope, Venue};
use reap_venue::{VenueAdapter, okx::OkxAdapter};
use serde::{Deserialize, Serialize};
use serde_json::Value;

use crate::{FeedOutput, FeedProcessor, payload_hash};

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct RawCapture {
    pub venue: Venue,
    pub conn_id: ConnId,
    pub channel: Channel,
    pub symbol: Option<String>,
    pub recv_ts_ns: u64,
    #[serde(default)]
    pub raw_hash: Option<u64>,
    pub payload: Value,
}

impl RawCapture {
    pub fn into_envelope(self) -> Result<RawEnvelope> {
        let payload = serde_json::to_string(&self.payload)?;
        Ok(RawEnvelope {
            venue: self.venue,
            conn_id: self.conn_id,
            channel: self.channel,
            symbol: self.symbol,
            recv_ts_ns: self.recv_ts_ns,
            raw_hash: self
                .raw_hash
                .unwrap_or_else(|| payload_hash(payload.as_bytes())),
            payload,
        })
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ReplayError {
    pub line: usize,
    pub message: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ReplayBookHealth {
    pub symbol: String,
    pub sequence_status: String,
    pub book_status: String,
    pub last_seq_id: Option<i64>,
    pub buffered_updates: usize,
    pub best_bid: Option<f64>,
    pub best_ask: Option<f64>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ReplayCheckReport {
    pub lines: u64,
    pub parsed_events: u64,
    pub accepted_events: u64,
    pub normalized_events: u64,
    pub duplicates: u64,
    pub gaps: u64,
    pub recoveries: u64,
    pub recovery_failures: u64,
    pub unrecovered_streams: usize,
    pub errors: Vec<ReplayError>,
    pub books: Vec<ReplayBookHealth>,
}

impl ReplayCheckReport {
    pub fn is_healthy(&self) -> bool {
        self.errors.is_empty()
            && self.recovery_failures == 0
            && self.unrecovered_streams == 0
            && self.books.iter().all(|book| book.book_status == "ready")
    }
}

pub fn replay_check_path(path: impl AsRef<Path>) -> Result<ReplayCheckReport> {
    let file = File::open(path.as_ref())
        .with_context(|| format!("failed to open raw capture {}", path.as_ref().display()))?;
    replay_check(file)
}

pub fn replay_check<R: Read>(reader: R) -> Result<ReplayCheckReport> {
    let adapters: Vec<Box<dyn VenueAdapter>> = vec![Box::new(OkxAdapter::default())];
    let mut processor = FeedProcessor::new(16_384, 32_768);
    let mut errors = Vec::new();
    let mut lines = 0_u64;

    for (index, line) in BufReader::new(reader).lines().enumerate() {
        let line_number = index + 1;
        let line = line?;
        let trimmed = line.trim();
        if trimmed.is_empty() || trimmed.starts_with('#') {
            continue;
        }
        lines += 1;
        let result = (|| -> Result<()> {
            let capture: RawCapture = serde_json::from_str(trimmed)?;
            let envelope = capture.into_envelope()?;
            let adapter = adapters
                .iter()
                .find(|adapter| adapter.venue() == envelope.venue)
                .context("no adapter registered for capture venue")?;
            for parsed in adapter.parse(&envelope)? {
                for output in processor.process(parsed) {
                    if let FeedOutput::System(event) = output {
                        tracing::debug!(?event, "raw replay system event");
                    }
                }
            }
            Ok(())
        })();
        if let Err(error) = result {
            errors.push(ReplayError {
                line: line_number,
                message: format!("{error:#}"),
            });
        }
    }

    let stats = processor.stats().clone();
    let books = processor
        .stream_health()
        .into_iter()
        .map(|health| ReplayBookHealth {
            symbol: health.stream.symbol,
            sequence_status: format!("{:?}", health.sequence_status).to_lowercase(),
            book_status: format!("{:?}", health.book_status).to_lowercase(),
            last_seq_id: health.last_seq_id,
            buffered_updates: health.buffered_updates,
            best_bid: health.best_bid,
            best_ask: health.best_ask,
        })
        .collect::<Vec<_>>();
    let unrecovered_streams = books
        .iter()
        .filter(|book| book.sequence_status != "ready")
        .count();

    Ok(ReplayCheckReport {
        lines,
        parsed_events: stats.parsed,
        accepted_events: stats.accepted,
        normalized_events: stats.normalized_events,
        duplicates: stats.duplicates,
        gaps: stats.gaps,
        recoveries: stats.recoveries,
        recovery_failures: stats.recovery_failures,
        unrecovered_streams,
        errors,
        books,
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn checker_reports_duplicate_gap_and_recovery() {
        let fixture = include_str!("../../../fixtures/raw/okx/depth-gap.jsonl");
        let report = replay_check(fixture.as_bytes()).unwrap();

        assert!(report.is_healthy(), "{report:#?}");
        assert_eq!(report.duplicates, 3);
        assert_eq!(report.gaps, 1);
        assert_eq!(report.recoveries, 1);
        assert_eq!(report.books[0].last_seq_id, Some(103));
        assert_eq!(report.books[0].best_bid, Some(100.2));
    }
}
