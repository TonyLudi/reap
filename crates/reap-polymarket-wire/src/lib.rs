//! Pure, bounded Polymarket wire parsing.
//!
//! This crate owns public REST/WebSocket wire shapes, exact subscription
//! serialization, snapshot integrity verification, and fixture-only private
//! response parsing. It has no network, authentication, signer,
//! private-session, or order-entry capability.

#![forbid(unsafe_code)]

mod error;
mod exact;
mod hash;
mod limits;
mod private_fixture;
mod raw;
mod rest;
mod scope;
mod subscription;
mod ws;

pub use error::PmWireError;
pub use exact::SnapshotHash;
pub use hash::{compute_snapshot_hash, verify_snapshot_hash};
pub use limits::{
    MAX_BOOK_LEVELS, MAX_PRIVATE_FIXTURE_BYTES, MAX_PRIVATE_FIXTURE_EVENTS,
    MAX_PUBLIC_REST_BODY_BYTES, MAX_PUBLIC_WS_FRAME_BYTES, MAX_WS_EVENTS_PER_FRAME,
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
pub use scope::{PmBookParserConfig, PmWireScope};
pub use subscription::PmMarketSubscription;
pub use ws::{
    PmBestBidAsk, PmBestPrices, PmBookSnapshot, PmExactBookLevel, PmExactPriceChange,
    PmIgnoredEvent, PmPriceChangeBatch, PmTickSizeChange, PmWsEvent, PmWsFrame, parse_ws_frame,
};
