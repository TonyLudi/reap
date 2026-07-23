#![forbid(unsafe_code)]

mod envelope;
mod event;
mod identity;
mod mapping;
mod metadata;
mod numeric;
mod reconciliation;

pub use envelope::{
    EnvelopeError, EventClock, EventEnvelope, EventOrdering, MAX_VENUE_EVENT_HASH_BYTES,
    ReceivedEventClock, ReceivedEventEnvelope, VenueEventHash, VenueEventHashAlgorithm,
};
pub use event::{
    MAX_PM_BOOK_LEVELS, MAX_PM_VENUE_CHANGE_HASH_BYTES, OkxReferenceEvent, OkxReferenceEventError,
    PmAllowanceEvent, PmAllowanceValue, PmBalanceEvent, PmBookDeltaBatch, PmBookDeltaParts,
    PmBookEvent, PmBookLevel, PmBookPoint, PmBookSide, PmBookSnapshot, PmBookTop, PmBookTopCheck,
    PmBookUpdate, PmEventError, PmFillEvent, PmFillExecution, PmFillFee, PmFillRole,
    PmFillSettlementStatus, PmMarketEvent, PmOrderEvent, PmOrderIdentity, PmOrderProgress,
    PmOrderStatus, PmPositionAvailability, PmPositionEvent, PmSnapshotEvidence, PmVenueChangeHash,
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
pub use mapping::{
    MAX_OKX_REFERENCES_PER_MAPPING, PmConfigurationFingerprint, PmMappingError,
    PmPublicObservationGrant, PmReferenceMapping,
};
pub use metadata::{
    MAX_REQUIRED_SPENDERS, PmGoalFTradingDomain, PmMarketLifecycle, PmMarketMetadata,
    PmMetadataError, PmOutcomeLabel, PmOutcomeMetadata,
};
pub use numeric::{
    CLOB_V2_LOT_UNITS, MAX_OKX_REFERENCE_DECIMAL_SCALE, OkxReferencePrice, OkxReferencePriceError,
    PM_ORDER_SALT_MAX, PM_PROTOCOL_SCALE, PmBookQuantity, PmErc1155OperatorApproval,
    PmNumericError, PmOrderAmounts, PmOrderSalt, PmOrderSide, PmPrice, PmQuantity, PmSign,
    PmSignedUnits, PmTick, U256, exact_order_amounts,
};
pub use reconciliation::{
    MAX_PM_ACCOUNT_DIAGNOSTIC_EXTRA_ROWS, MAX_PM_ACCOUNT_EXPECTED_ASSETS,
    MAX_PM_ACCOUNT_EXPECTED_INSTRUMENTS, MAX_PM_ACCOUNT_EXPECTED_SPENDERS,
    MAX_PM_ACCOUNT_SNAPSHOT_ROWS, MAX_PM_RECONCILIATION_FILLS, MAX_PM_RECONCILIATION_ORDERS,
    PmAggregateError, PmCompleteAccountSnapshot, PmCompleteFillQuery, PmCompleteOpenOrdersSnapshot,
    PmExactOrderDetail, PmFillQueryCursor, PmReconciliationRequestBoundary,
};
