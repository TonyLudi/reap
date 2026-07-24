use reap_core::{Channel, EventKey, RawEnvelope};
use serde::Serialize;
use thiserror::Error;

use crate::public_wire::{WireArg, WireIndexTicker};

pub const MAX_OKX_INDEX_PRICE_BYTES: usize = 128;

/// Compatibility-only lexical fields used by the legacy broad OKX adapter.
///
/// This type carries no configured-source proof, connection epoch, raw
/// identity, or receive evidence and must not be treated as a trusted
/// reference observation.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct LegacyOkxIndexTickerFields {
    instrument: String,
    index_price_lexeme: String,
    venue_ts_ms: u64,
}

impl LegacyOkxIndexTickerFields {
    #[must_use]
    pub fn instrument(&self) -> &str {
        &self.instrument
    }

    #[must_use]
    pub fn index_price_lexeme(&self) -> &str {
        &self.index_price_lexeme
    }

    #[must_use]
    pub const fn venue_ts_ms(&self) -> u64 {
        self.venue_ts_ms
    }
}

/// Preserves the legacy adapter's `instId` fallback order and timestamp
/// parsing without granting configured-reference authority.
pub fn extract_legacy_index_ticker_fields(
    data_instrument: Option<String>,
    argument_instrument: Option<&str>,
    envelope_instrument: Option<&str>,
    index_price_lexeme: String,
    venue_timestamp_lexeme: &str,
) -> Result<LegacyOkxIndexTickerFields, LegacyOkxIndexTickerFieldError> {
    let instrument = data_instrument
        .or_else(|| argument_instrument.map(str::to_string))
        .or_else(|| envelope_instrument.map(str::to_string))
        .ok_or(LegacyOkxIndexTickerFieldError::MissingInstrument)?;
    let venue_ts_ms = venue_timestamp_lexeme.parse::<u64>().map_err(|error| {
        LegacyOkxIndexTickerFieldError::InvalidTimestamp {
            value: venue_timestamp_lexeme.to_string(),
            reason: error.to_string(),
        }
    })?;
    Ok(LegacyOkxIndexTickerFields {
        instrument,
        index_price_lexeme,
        venue_ts_ms,
    })
}

#[derive(Debug, Error)]
pub enum LegacyOkxIndexTickerFieldError {
    #[error("index-tickers message has no instId")]
    MissingInstrument,
    #[error("invalid ts {value:?}: {reason}")]
    InvalidTimestamp { value: String, reason: String },
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct OkxIndexTickerReference {
    instrument: String,
    index_price_lexeme: String,
    venue_ts_ms: u64,
    wall_receive_ts_ns: u64,
    connection_epoch: u64,
    raw_hash: u64,
}

impl OkxIndexTickerReference {
    #[must_use]
    pub fn instrument(&self) -> &str {
        &self.instrument
    }

    #[must_use]
    pub fn index_price_lexeme(&self) -> &str {
        &self.index_price_lexeme
    }

    #[must_use]
    pub const fn venue_ts_ms(&self) -> u64 {
        self.venue_ts_ms
    }

    #[must_use]
    pub const fn wall_receive_ts_ns(&self) -> u64 {
        self.wall_receive_ts_ns
    }

    #[must_use]
    pub const fn connection_epoch(&self) -> u64 {
        self.connection_epoch
    }

    #[must_use]
    pub const fn raw_hash(&self) -> u64 {
        self.raw_hash
    }

    #[must_use]
    pub const fn event_key(&self) -> EventKey {
        EventKey::TimestampHash {
            ts_ms: self.venue_ts_ms,
            raw_hash: self.raw_hash,
        }
    }
}

pub(crate) fn configured_reference_from_wire(
    envelope: &RawEnvelope,
    expected_instrument: &str,
    connection_epoch: u64,
    argument: WireArg,
    mut values: Vec<WireIndexTicker>,
) -> Result<OkxIndexTickerReference, OkxIndexTickerReferenceError> {
    if !matches!(
        &envelope.channel,
        Channel::Custom(channel) if channel == "index-tickers"
    ) {
        return Err(OkxIndexTickerReferenceError::WrongChannel);
    }
    if envelope.symbol.as_deref() != Some(expected_instrument)
        || argument.instrument.as_deref() != Some(expected_instrument)
    {
        return Err(OkxIndexTickerReferenceError::WrongInstrument);
    }
    if argument.channel != "index-tickers" {
        return Err(OkxIndexTickerReferenceError::WrongChannel);
    }
    if values.len() != 1 {
        return Err(OkxIndexTickerReferenceError::WrongValueCount);
    }
    let value = values.pop().expect("one value checked");
    if value.instrument.as_deref() != Some(expected_instrument) {
        return Err(OkxIndexTickerReferenceError::WrongInstrument);
    }
    if envelope.recv_ts_ns == 0 {
        return Err(OkxIndexTickerReferenceError::ZeroWallReceiveTimestamp);
    }
    if connection_epoch == 0 {
        return Err(OkxIndexTickerReferenceError::ZeroConnectionEpoch);
    }

    validate_positive_decimal(&value.index_price)?;
    let fields = extract_legacy_index_ticker_fields(
        value.instrument,
        argument.instrument.as_deref(),
        envelope.symbol.as_deref(),
        value.index_price,
        &value.ts,
    )?;
    if fields.venue_ts_ms == 0 {
        return Err(OkxIndexTickerReferenceError::ZeroVenueTimestamp);
    }
    Ok(OkxIndexTickerReference {
        instrument: fields.instrument,
        index_price_lexeme: fields.index_price_lexeme,
        venue_ts_ms: fields.venue_ts_ms,
        wall_receive_ts_ns: envelope.recv_ts_ns,
        connection_epoch,
        raw_hash: envelope.raw_hash,
    })
}

fn validate_positive_decimal(value: &str) -> Result<(), OkxIndexTickerReferenceError> {
    if value.is_empty() {
        return Err(OkxIndexTickerReferenceError::InvalidIndexPrice);
    }
    if value.len() > MAX_OKX_INDEX_PRICE_BYTES {
        return Err(OkxIndexTickerReferenceError::IndexPriceTooLong);
    }
    let bytes = value.as_bytes();
    let mut dot = None;
    let mut nonzero = false;
    for (index, byte) in bytes.iter().copied().enumerate() {
        if byte == b'.' {
            if dot.replace(index).is_some() {
                return Err(OkxIndexTickerReferenceError::InvalidIndexPrice);
            }
        } else if byte.is_ascii_digit() {
            nonzero |= byte != b'0';
        } else {
            return Err(OkxIndexTickerReferenceError::InvalidIndexPrice);
        }
    }
    if dot.is_some_and(|index| index == 0 || index + 1 == bytes.len()) {
        return Err(OkxIndexTickerReferenceError::InvalidIndexPrice);
    }
    if !nonzero {
        return Err(OkxIndexTickerReferenceError::NonpositiveIndexPrice);
    }
    Ok(())
}

#[derive(Debug, Error)]
pub enum OkxIndexTickerReferenceError {
    #[error("reference envelope venue is not OKX")]
    WrongVenue,
    #[error("reference frame is not the index-tickers channel")]
    WrongChannel,
    #[error("reference frame does not match the configured index instrument")]
    WrongInstrument,
    #[error("reference frame must contain exactly one configured value")]
    WrongValueCount,
    #[error("reference index price exceeds its fixed byte bound")]
    IndexPriceTooLong,
    #[error("reference index price is not an exact decimal lexeme")]
    InvalidIndexPrice,
    #[error("reference index price must be positive")]
    NonpositiveIndexPrice,
    #[error("reference venue timestamp must be positive")]
    ZeroVenueTimestamp,
    #[error("reference wall receive timestamp must be positive")]
    ZeroWallReceiveTimestamp,
    #[error("reference connection epoch must be positive")]
    ZeroConnectionEpoch,
    #[error(transparent)]
    Fields(#[from] LegacyOkxIndexTickerFieldError),
}
