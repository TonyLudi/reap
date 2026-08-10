#![forbid(unsafe_code)]

mod config;
mod decimal;
mod error;
mod position;
mod source;

pub use config::{PM_DATA_API_PRODUCTION_ORIGIN, PmDataApiPositionConfig, PmDataApiPositionScope};
pub use decimal::{MAX_POSITION_DECIMAL_BYTES, PmExactPositionDecimal};
pub use error::{PmDataApiFixedPeerSourceError, PmPublicPositionError};
pub use position::{
    MAX_POSITION_PAGE_ROWS, PM_POSITION_API_SOURCE_AUTHORITY, PM_POSITION_API_SOURCE_SHA256,
    PmConfiguredTokenPosition, PmDataApiPositionEvidence, PmDataApiPositionObservationCommitment,
    PmDataApiReceiveClockObservation, PmMonitoredPositionObservation,
};
pub use source::{
    MAX_POSITION_PAGE_BODY_BYTES, PmDataApiCurrentPositionSource, PmProductionDataApiPositionError,
    PmProductionDataApiPositionObservation,
};

/// Public position reads can never authorize order entry.
pub const PRODUCTION_ORDER_ENTRY_AUTHORIZED: bool = false;
