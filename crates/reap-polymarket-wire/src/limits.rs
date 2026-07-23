use reap_pm_core::MAX_PM_BOOK_LEVELS;

pub const MAX_BOOK_LEVELS: usize = MAX_PM_BOOK_LEVELS as usize;
pub const MAX_WS_EVENTS_PER_FRAME: usize = 64;
pub const MAX_PUBLIC_WS_FRAME_BYTES: usize = 1_048_576;
pub const MAX_PUBLIC_REST_BODY_BYTES: usize = 1_048_576;
pub const MAX_PRIVATE_FIXTURE_BYTES: usize = 1_048_576;
pub const MAX_PRIVATE_FIXTURE_EVENTS: usize = 64;
pub(crate) const MAX_MARKET_TOKENS: usize = 256;
