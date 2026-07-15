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
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub capture_session_id: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub capture_record_seq: Option<u64>,
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
    pub sequence_resets: u64,
    pub same_sequence_updates: u64,
    pub best_bid: Option<f64>,
    pub best_ask: Option<f64>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ReplayCheckReport {
    pub lines: u64,
    pub capture_sessions: usize,
    pub sequenced_records: u64,
    pub first_capture_record_seq: Option<u64>,
    pub last_capture_record_seq: Option<u64>,
    pub capture_record_sequence_errors: u64,
    pub capture_record_sequence_complete: bool,
    pub parsed_events: u64,
    pub accepted_events: u64,
    pub normalized_events: u64,
    pub duplicates: u64,
    pub gaps: u64,
    pub recoveries: u64,
    pub recovery_failures: u64,
    pub sequence_resets: u64,
    pub same_sequence_updates: u64,
    pub unrecovered_streams: usize,
    pub errors: Vec<ReplayError>,
    pub books: Vec<ReplayBookHealth>,
}

impl ReplayCheckReport {
    pub fn is_healthy(&self) -> bool {
        self.lines > 0
            && self.capture_sessions == 1
            && (self.sequenced_records == 0 || self.capture_record_sequence_complete)
            && !self.books.is_empty()
            && self.errors.is_empty()
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
    let mut capture_sessions = std::collections::HashSet::new();
    let mut sequenced_records = 0_u64;
    let mut first_capture_record_seq = None;
    let mut last_capture_record_seq = None;
    let mut capture_record_sequence_errors = 0_u64;
    let mut saw_unsequenced_record = false;

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
            let session = capture
                .capture_session_id
                .as_deref()
                .map_or_else(|| "legacy:unspecified".to_string(), |id| format!("id:{id}"));
            if capture_sessions.insert(session) && capture_sessions.len() > 1 {
                anyhow::bail!("capture contains more than one process session");
            }
            match capture.capture_record_seq {
                Some(sequence) => {
                    sequenced_records = sequenced_records.saturating_add(1);
                    first_capture_record_seq.get_or_insert(sequence);
                    let expected = last_capture_record_seq
                        .and_then(|previous: u64| previous.checked_add(1))
                        .unwrap_or(1);
                    last_capture_record_seq = Some(sequence);
                    if saw_unsequenced_record || sequence != expected {
                        capture_record_sequence_errors =
                            capture_record_sequence_errors.saturating_add(1);
                        anyhow::bail!(
                            "capture record sequence expected {expected}, received {sequence}"
                        );
                    }
                }
                None => {
                    saw_unsequenced_record = true;
                    if sequenced_records > 0 {
                        capture_record_sequence_errors =
                            capture_record_sequence_errors.saturating_add(1);
                        anyhow::bail!("capture mixes sequenced and legacy unsequenced records");
                    }
                }
            }
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
            sequence_resets: health.sequence_resets,
            same_sequence_updates: health.same_sequence_updates,
            best_bid: health.best_bid,
            best_ask: health.best_ask,
        })
        .collect::<Vec<_>>();
    let unrecovered_streams = books
        .iter()
        .filter(|book| book.sequence_status != "ready")
        .count();
    let capture_record_sequence_complete = lines > 0
        && sequenced_records == lines
        && first_capture_record_seq == Some(1)
        && last_capture_record_seq == Some(lines)
        && capture_record_sequence_errors == 0;

    Ok(ReplayCheckReport {
        lines,
        capture_sessions: capture_sessions.len(),
        sequenced_records,
        first_capture_record_seq,
        last_capture_record_seq,
        capture_record_sequence_errors,
        capture_record_sequence_complete,
        parsed_events: stats.parsed,
        accepted_events: stats.accepted,
        normalized_events: stats.normalized_events,
        duplicates: stats.duplicates,
        gaps: stats.gaps,
        recoveries: stats.recoveries,
        recovery_failures: stats.recovery_failures,
        sequence_resets: stats.sequence_resets,
        same_sequence_updates: stats.same_sequence_updates,
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
        assert_eq!(report.capture_sessions, 1);
        assert_eq!(report.sequenced_records, 0);
        assert!(!report.capture_record_sequence_complete);
        assert_eq!(report.gaps, 1);
        assert_eq!(report.recoveries, 1);
        assert_eq!(report.books[0].last_seq_id, Some(103));
        assert_eq!(report.books[0].best_bid, Some(100.2));
    }

    #[test]
    fn checker_accepts_documented_okx_sequence_reset_and_heartbeat() {
        let fixture = include_str!("../../../fixtures/raw/okx/depth-reset.jsonl");
        let report = replay_check(fixture.as_bytes()).unwrap();

        assert!(report.is_healthy(), "{report:#?}");
        assert_eq!(report.capture_sessions, 1);
        assert_eq!(report.sequenced_records, 7);
        assert_eq!(report.first_capture_record_seq, Some(1));
        assert_eq!(report.last_capture_record_seq, Some(7));
        assert!(report.capture_record_sequence_complete);
        assert_eq!(report.duplicates, 3);
        assert_eq!(report.gaps, 0);
        assert_eq!(report.sequence_resets, 1);
        assert_eq!(report.same_sequence_updates, 1);
        assert_eq!(report.books[0].sequence_resets, 1);
        assert_eq!(report.books[0].same_sequence_updates, 1);
        assert_eq!(report.books[0].last_seq_id, Some(5));
        assert_eq!(report.books[0].best_bid, Some(100.1));
        assert_eq!(report.books[0].best_ask, Some(100.9));
    }

    #[test]
    fn checker_rejects_concatenated_capture_sessions() {
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

        let report = replay_check(input.as_bytes()).unwrap();

        assert!(!report.is_healthy());
        assert_eq!(report.capture_sessions, 2);
        assert_eq!(report.errors.len(), 1);
        assert!(
            report.errors[0]
                .message
                .contains("more than one process session")
        );
    }

    #[test]
    fn checker_rejects_a_broken_capture_record_sequence() {
        let mut records = include_str!("../../../fixtures/raw/okx/depth-reset.jsonl")
            .lines()
            .map(|line| serde_json::from_str::<RawCapture>(line).unwrap())
            .collect::<Vec<_>>();
        records[3].capture_record_seq = Some(9);
        let input = records
            .iter()
            .map(|record| serde_json::to_string(record).unwrap())
            .collect::<Vec<_>>()
            .join("\n")
            + "\n";

        let report = replay_check(input.as_bytes()).unwrap();

        assert!(!report.is_healthy());
        assert!(!report.capture_record_sequence_complete);
        assert!(report.capture_record_sequence_errors > 0);
        assert!(
            report
                .errors
                .iter()
                .any(|error| error.message.contains("record sequence expected"))
        );
    }

    #[test]
    fn checker_rejects_empty_input() {
        let report = replay_check("\n# no records\n".as_bytes()).unwrap();

        assert!(!report.is_healthy());
        assert_eq!(report.lines, 0);
        assert_eq!(report.capture_sessions, 0);
    }
}
