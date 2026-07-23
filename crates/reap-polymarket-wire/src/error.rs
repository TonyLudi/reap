use thiserror::Error;

use crate::SnapshotHash;

/// A fail-closed public Polymarket wire failure.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Error)]
pub enum PmWireError {
    #[error("public Polymarket JSON is malformed or has a wrong wire type")]
    MalformedJson,
    #[error("public Polymarket payload exceeds its byte bound")]
    PayloadTooLarge,
    #[error("public Polymarket WebSocket frame exceeds its byte bound")]
    WsFrameTooLarge,
    #[error("public Polymarket REST body exceeds its byte bound")]
    RestBodyTooLarge,
    #[error("public Polymarket WebSocket envelope is empty")]
    EmptyEnvelope,
    #[error("public Polymarket WebSocket envelope exceeds its event bound")]
    TooManyEvents,
    #[error("public Polymarket book exceeds its level bound")]
    TooManyBookLevels,
    #[error("public Polymarket CLOB metadata exceeds its token bound")]
    TooManyMarketTokens,
    #[error("required public Polymarket field `{0}` is missing")]
    MissingField(&'static str),
    #[error("public Polymarket field `{0}` exceeds its byte bound")]
    FieldTooLong(&'static str),
    #[error("public Polymarket field `{0}` is not ASCII")]
    NonAsciiField(&'static str),
    #[error("public Polymarket field `{0}` has an invalid structural identity")]
    InvalidIdentity(&'static str),
    #[error("public Polymarket field `{0}` has an invalid exact numeric value")]
    InvalidNumeric(&'static str),
    #[error("public Polymarket condition does not match configured scope")]
    ConditionMismatch,
    #[error("public Polymarket market does not match configured scope")]
    MarketMismatch,
    #[error("public Polymarket outcome token does not match configured scope")]
    TokenMismatch,
    #[error("configured outcome token is absent from CLOB metadata")]
    ConfiguredTokenMissing,
    #[error("CLOB metadata contains a duplicate outcome token")]
    DuplicateToken,
    #[error("book or price-change batch contains a duplicate side/price")]
    DuplicateLevel,
    #[error("book snapshot has an empty bid or ask side")]
    EmptyBook,
    #[error("book snapshot is locked or crossed")]
    CrossedBook,
    #[error("public Polymarket price is off the configured tick")]
    PriceOffConfiguredTick,
    #[error("snapshot tick differs from the configured metadata")]
    MetadataTickMismatch,
    #[error("snapshot minimum order size differs from the configured metadata")]
    MetadataMinimumMismatch,
    #[error("snapshot negative-risk flag differs from the configured metadata")]
    MetadataNegativeRiskMismatch,
    #[error("snapshot hash is not canonical lowercase 20-byte SHA-1 hex")]
    NonCanonicalSnapshotHash,
    #[error("snapshot SHA-1 mismatch")]
    SnapshotHashMismatch {
        expected: SnapshotHash,
        actual: SnapshotHash,
    },
    #[error("public Polymarket event type is outside the reached public protocol")]
    UnsupportedEventType,
    #[error("public Polymarket price-change side is invalid")]
    InvalidSide,
    #[error("public Polymarket price-change batch is empty")]
    EmptyPriceChanges,
    #[error("price-change best bid/ask fields must appear as a complete pair")]
    PartialBestPrices,
    #[error("best-bid/ask size fields must appear as a complete pair")]
    PartialBestSizes,
    #[error("price-change batch lacks final best-bid/ask integrity values")]
    MissingBestPrices,
    #[error("best bid is not strictly below best ask")]
    CrossedBestPrices,
    #[error("tick-size-change old tick differs from configured metadata")]
    TickChangeOldMismatch,
    #[error("tick-size-change does not actually change the tick")]
    TickSizeUnchanged,
    #[error("public Polymarket server time must be a positive integer")]
    InvalidServerTime,
    #[error("public Polymarket subscription serialization failed")]
    Serialization,
}
