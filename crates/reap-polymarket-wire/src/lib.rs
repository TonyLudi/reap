//! Pure, bounded Polymarket wire parsing.
//!
//! This crate owns public REST/WebSocket wire shapes, exact subscription
//! serialization, snapshot integrity verification, fixture-only private
//! parsing, and secret-custody-free live response parsing. It has no network,
//! authentication, signer, private-session, or order-entry capability.

#![forbid(unsafe_code)]

mod error;
mod exact;
mod hash;
mod limits;
mod live_metadata;
mod live_private;
mod preflight;
mod private_fixture;
mod raw;
mod rest;
mod scope;
mod subscription;
mod unsigned_order;
mod ws;

pub use error::PmWireError;
pub use exact::SnapshotHash;
pub use hash::{compute_snapshot_hash, verify_snapshot_hash};
pub use limits::{
    MAX_BOOK_LEVELS, MAX_PRIVATE_FIXTURE_BYTES, MAX_PRIVATE_FIXTURE_EVENTS,
    MAX_PUBLIC_REST_BODY_BYTES, MAX_PUBLIC_WS_FRAME_BYTES, MAX_WS_EVENTS_PER_FRAME,
};
pub use live_metadata::{
    PmClobFeeDecimal, PmClobFeeDetails, PmClobV2Metadata, PmClobV2RequestScope,
    PmLifecycleTimeString, PmLiveClobMarketLifecycle, PmLongMarketLifecycleDetails,
    parse_live_clob_market_lifecycle, parse_live_clob_market_lifecycle_details,
    parse_live_clob_v2_metadata,
};
pub use live_private::{
    MAX_PM_LIVE_BODY_BYTES, MAX_PM_LIVE_CURSOR_BYTES, MAX_PM_LIVE_PAGE_ITEMS, PmCredentialOwner,
    PmLiveAllowanceEntry, PmLiveBalanceAllowance, PmLiveCancelResult, PmLiveCursor,
    PmLiveMakerOrder, PmLiveOpenOrderPage, PmLiveOrder, PmLivePlaceResult, PmLiveTrade,
    PmLiveTradePage, PmLiveUserEvent, PmLiveUserFrame, PmLiveUserOrder, PmLiveWireError,
    parse_live_balance_allowance, parse_live_cancel_result, parse_live_open_order_page,
    parse_live_order_detail, parse_live_place_result, parse_live_trade_page, parse_live_user_frame,
};
pub use preflight::{
    MAX_PM_CLOSED_ONLY_BODY_BYTES, MAX_PM_GEOBLOCK_BODY_BYTES, PmClosedOnlyStatus,
    PmGeoblockStatus, parse_pm_closed_only, parse_pm_geoblock,
};
pub use private_fixture::{
    PmFixtureAllowanceScope, PmFixtureMakerOrder, PmFixtureOpenOrder, PmFixtureTradeLinkage,
    PmFixtureUserEvent, PmFixtureUserFrame, PmFixtureUserOrder, PmFixtureUserTrade,
    PmLegacyBalanceAllowanceFixture, PmPrivateFixtureError, parse_legacy_balance_allowance_fixture,
    parse_open_order_fixture, parse_private_user_fixture,
};
pub use rest::{
    PmClobMetadata, PmClobToken, PmLifecycleMetadata, parse_clob_metadata,
    parse_lifecycle_metadata, parse_rest_book_snapshot, parse_server_time,
};
pub use scope::{PmBookMarketBinding, PmBookParserConfig, PmWireScope};
pub use subscription::PmMarketSubscription;
pub use unsigned_order::{
    PM_CLOB_V2_EMPTY_BYTES32, PM_CLOB_V2_EOA_SIGNATURE_TYPE, PM_CLOB_V2_PROXY_SIGNATURE_TYPE,
    PmClobV2SignatureType, PmUnsignedClobV2Order, PmUnsignedOrderError,
};
pub use ws::{
    PmBestBidAsk, PmBestPrices, PmBookSnapshot, PmExactBookLevel, PmExactPriceChange,
    PmIgnoredEvent, PmPriceChangeBatch, PmTickSizeChange, PmWsEvent, PmWsFrame, parse_ws_frame,
};
