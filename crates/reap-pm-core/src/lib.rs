#![forbid(unsafe_code)]

mod envelope;
mod event;
mod identity;
mod mapping;
mod metadata;
mod numeric;

pub use envelope::{EnvelopeError, EventClock, EventEnvelope, EventOrdering, VenueEventHash};
pub use event::{
    MAX_PM_BOOK_LEVELS, OkxReferenceEvent, OkxReferenceEventError, PmAllowanceEvent,
    PmAllowanceValue, PmBalanceEvent, PmBookEvent, PmBookLevel, PmBookPoint, PmBookSide, PmBookTop,
    PmBookUpdate, PmEventError, PmFillEvent, PmFillExecution, PmFillFee, PmFillRole, PmMarketEvent,
    PmOrderEvent, PmOrderIdentity, PmOrderProgress, PmOrderStatus, PmPositionAvailability,
    PmPositionEvent, PmSnapshotCompleteness, PmSnapshotEvidence,
};
pub use identity::{
    ConnectionEpoch, EvmAddress, IngressSequence, OkxInstrumentId, OkxReferenceHandle,
    OkxReferenceInstrument, OkxReferenceKind, PmAccountHandle, PmAccountScope, PmAssetId,
    PmChainId, PmClientOrderId, PmClientOrderKey, PmConditionId, PmConnectionId, PmEnvironmentId,
    PmFillId, PmFillKey, PmFunderId, PmIdentityError, PmInstrumentHandle, PmInstrumentId,
    PmMarketHandle, PmMarketId, PmProductSource, PmSignerId, PmSourceBound, PmSourceHandle,
    PmSpenderDomain, PmSpenderHandle, PmSpenderId, PmSpenderRequirement, PmTokenHandle, PmTokenId,
    PmVenueOrderId, PmVenueOrderKey, SnapshotRevision,
};
pub use mapping::{MAX_OKX_REFERENCES_PER_MAPPING, PmMappingError, PmReferenceMapping};
pub use metadata::{
    MAX_REQUIRED_SPENDERS, PmMarketLifecycle, PmMarketMetadata, PmMetadataError, PmOutcomeLabel,
    PmOutcomeMetadata,
};
pub use numeric::{
    CLOB_V2_LOT_UNITS, MAX_OKX_REFERENCE_DECIMAL_SCALE, OkxReferencePrice, OkxReferencePriceError,
    PM_ORDER_SALT_MAX, PM_PROTOCOL_SCALE, PmBookQuantity, PmErc1155OperatorApproval,
    PmNumericError, PmOrderAmounts, PmOrderSalt, PmOrderSide, PmPrice, PmQuantity, PmSign,
    PmSignedUnits, PmTick, U256, exact_order_amounts,
};
