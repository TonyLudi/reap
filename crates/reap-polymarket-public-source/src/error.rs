use thiserror::Error;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Error)]
pub enum PmPublicPositionError {
    #[error("PM Data API position configuration is invalid")]
    InvalidConfiguration,
    #[error("PM Data API HTTP transport could not be built")]
    TransportBuild,
    #[error("PM Data API position request timed out")]
    RequestTimeout,
    #[error("PM Data API position request failed")]
    RequestFailed,
    #[error("PM Data API position response body could not be read")]
    ResponseBodyRead,
    #[error("PM Data API position response redirected with status {0}")]
    Redirect(u16),
    #[error("PM Data API position response had unexpected status {0}")]
    UnexpectedStatus(u16),
    #[error("PM Data API position response has invalid application headers")]
    InvalidApplicationHeaders,
    #[error("PM Data API position response exceeds its body bound")]
    ResponseBodyTooLarge,
    #[error("PM Data API position response is not exact bounded JSON")]
    InvalidJson,
    #[error("PM Data API position page exceeds the exact row limit")]
    PageRowLimit,
    #[error("PM Data API position row contains an invalid {0} field")]
    InvalidField(&'static str),
    #[error("PM Data API position row contains an oversized {0} field")]
    FieldTooLong(&'static str),
    #[error("PM Data API position row conflicts with the configured {0} scope")]
    ScopeMismatch(&'static str),
    #[error("PM Data API position observation repeats an asset")]
    DuplicateAsset,
    #[error("PM Data API position pagination reached a full page at the offset cap")]
    FullPageAtOffsetCap,
}
