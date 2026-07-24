use std::sync::atomic::{AtomicU64, Ordering};

use reap_durable_writer::JournalCodec;
use thiserror::Error;

use super::schema::{MAX_PM_JOURNAL_BYTES, MAX_PM_JOURNAL_LINE_BYTES, PmJournalLineV1};

#[derive(Debug)]
pub(super) struct PmJournalCodec {
    encoded_bytes: AtomicU64,
}

impl PmJournalCodec {
    pub(super) const fn new(existing_bytes: u64) -> Self {
        Self {
            encoded_bytes: AtomicU64::new(existing_bytes),
        }
    }
}

impl JournalCodec<PmJournalLineV1> for PmJournalCodec {
    type Error = PmJournalCodecError;

    fn encode(&self, record: &PmJournalLineV1, output: &mut Vec<u8>) -> Result<(), Self::Error> {
        let start = output.len();
        if let Err(error) = serde_json::to_writer(&mut *output, record) {
            output.truncate(start);
            return Err(error.into());
        }
        let encoded = output.len() - start;
        if encoded > MAX_PM_JOURNAL_LINE_BYTES {
            output.truncate(start);
            return Err(PmJournalCodecError::LineTooLarge);
        }
        let bytes_with_newline = u64::try_from(encoded.saturating_add(1))
            .map_err(|_| PmJournalCodecError::FileTooLarge)?;
        let update =
            self.encoded_bytes
                .fetch_update(Ordering::Relaxed, Ordering::Relaxed, |current| {
                    current
                        .checked_add(bytes_with_newline)
                        .filter(|next| *next <= MAX_PM_JOURNAL_BYTES)
                });
        if update.is_err() {
            output.truncate(start);
            return Err(PmJournalCodecError::FileTooLarge);
        }
        Ok(())
    }
}

#[derive(Debug, Error)]
pub(super) enum PmJournalCodecError {
    #[error("PM journal JSON encoding failed: {0}")]
    Json(#[from] serde_json::Error),
    #[error("PM journal encoded line exceeds its bounded size")]
    LineTooLarge,
    #[error("PM journal encoded generation exceeds its bounded size")]
    FileTooLarge,
}
