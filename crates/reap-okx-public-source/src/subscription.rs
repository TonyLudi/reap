use reap_core::{Channel, FeedPriority, Subscription, Venue};
use thiserror::Error;

pub const OKX_INDEX_TICKERS_CHANNEL: &str = "index-tickers";
pub const MAX_OKX_INDEX_INSTRUMENT_BYTES: usize = 64;

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct OkxIndexTickerSubscription {
    instrument: String,
    wire: String,
}

impl OkxIndexTickerSubscription {
    pub fn new(instrument: impl AsRef<str>) -> Result<Self, OkxIndexTickerSubscriptionError> {
        let instrument = instrument.as_ref();
        validate_instrument(instrument)?;
        let wire = format!(
            r#"{{"op":"subscribe","args":[{{"channel":"{OKX_INDEX_TICKERS_CHANNEL}","instId":"{instrument}"}}]}}"#
        );
        Ok(Self {
            instrument: instrument.to_string(),
            wire,
        })
    }

    #[must_use]
    pub fn instrument(&self) -> &str {
        &self.instrument
    }

    #[must_use]
    pub fn wire_bytes(&self) -> &[u8] {
        self.wire.as_bytes()
    }

    #[must_use]
    pub fn as_core_subscription(&self) -> Subscription {
        Subscription::public(
            Venue::Okx,
            Channel::Custom(OKX_INDEX_TICKERS_CHANNEL.to_string()),
            self.instrument.clone(),
            FeedPriority::Critical,
        )
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Error)]
pub enum OkxIndexTickerSubscriptionError {
    #[error("OKX index instrument is empty")]
    EmptyInstrument,
    #[error("OKX index instrument exceeds its fixed byte bound")]
    InstrumentTooLong,
    #[error("OKX index instrument must be uppercase ASCII segments separated by hyphens")]
    InvalidInstrument,
}

fn validate_instrument(instrument: &str) -> Result<(), OkxIndexTickerSubscriptionError> {
    if instrument.is_empty() {
        return Err(OkxIndexTickerSubscriptionError::EmptyInstrument);
    }
    if instrument.len() > MAX_OKX_INDEX_INSTRUMENT_BYTES {
        return Err(OkxIndexTickerSubscriptionError::InstrumentTooLong);
    }
    if !instrument.is_ascii() {
        return Err(OkxIndexTickerSubscriptionError::InvalidInstrument);
    }
    let mut segments = instrument.split('-');
    let Some(first) = segments.next() else {
        return Err(OkxIndexTickerSubscriptionError::InvalidInstrument);
    };
    if !valid_segment(first) {
        return Err(OkxIndexTickerSubscriptionError::InvalidInstrument);
    }
    let mut segment_count = 1_usize;
    for segment in segments {
        segment_count += 1;
        if !valid_segment(segment) {
            return Err(OkxIndexTickerSubscriptionError::InvalidInstrument);
        }
    }
    if segment_count < 2 {
        return Err(OkxIndexTickerSubscriptionError::InvalidInstrument);
    }
    Ok(())
}

fn valid_segment(segment: &str) -> bool {
    !segment.is_empty()
        && segment
            .bytes()
            .all(|byte| byte.is_ascii_uppercase() || byte.is_ascii_digit())
}
