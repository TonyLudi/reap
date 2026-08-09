use std::time::{Instant, SystemTime, UNIX_EPOCH};

use crate::PmLiveAdapterError;

/// Source-captured receive edge for one bounded public HTTP observation.
///
/// The type has no public constructor. Only the live adapter's fixed-purpose
/// HTTP roles can sample and issue it after their response bodies are complete.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct PmHttpReceiveClock {
    local_wall_receive_ns: u64,
    monotonic_receive_ns: u64,
}

impl PmHttpReceiveClock {
    #[must_use]
    pub const fn local_wall_receive_ns(self) -> u64 {
        self.local_wall_receive_ns
    }

    #[must_use]
    pub const fn monotonic_receive_ns(self) -> u64 {
        self.monotonic_receive_ns
    }
}

/// Private clock source retained by the fixed public HTTP role. There is no
/// caller-supplied clock or constructor for a receive observation.
#[derive(Clone)]
pub(crate) struct PmHttpReceiveClockSource {
    origin: Instant,
}

impl PmHttpReceiveClockSource {
    pub(crate) fn system() -> Self {
        Self {
            origin: Instant::now(),
        }
    }

    pub(crate) fn observe(&self) -> Result<PmHttpReceiveClock, PmLiveAdapterError> {
        let local_wall_receive_ns = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .map_err(|_| PmLiveAdapterError::ProductClock)?
            .as_nanos()
            .try_into()
            .map_err(|_| PmLiveAdapterError::ProductClock)?;
        let monotonic_receive_ns = self
            .origin
            .elapsed()
            .as_nanos()
            .saturating_add(1)
            .try_into()
            .map_err(|_| PmLiveAdapterError::ProductClock)?;
        if local_wall_receive_ns == 0 || monotonic_receive_ns == 0 {
            return Err(PmLiveAdapterError::ProductClock);
        }
        Ok(PmHttpReceiveClock {
            local_wall_receive_ns,
            monotonic_receive_ns,
        })
    }
}

impl std::fmt::Debug for PmHttpReceiveClockSource {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter.write_str("PmHttpReceiveClockSource(<opaque-origin>)")
    }
}
