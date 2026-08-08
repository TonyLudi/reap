use reap_polymarket_live_adapter::{
    PmRestBookPurpose, PmRestBookSnapshotSink, PmRestResponseClock,
};

use crate::capture_roles::PmPublicCaptureBatch;
use crate::composition::{PmPublicCaptureRun, PmPublicCaptureRunError};

/// One native REST-book response released only after durable raw capture and
/// classification by the configured PM public session.
///
/// This carrier is deliberately move-only and crate-private: product
/// composition may route its batch into the existing reducer, but transport
/// callers cannot recover or replay the native bytes.
#[derive(Debug)]
pub struct PmCapturedRestBook {
    purpose: PmRestBookPurpose,
    batch: PmPublicCaptureBatch,
}

impl PmCapturedRestBook {
    #[must_use]
    pub const fn purpose(&self) -> PmRestBookPurpose {
        self.purpose
    }

    pub fn into_batch(self) -> PmPublicCaptureBatch {
        self.batch
    }
}

/// Purpose-specific bridge from the live public HTTP role into the sole
/// durability-first public capture/session owner.
pub struct PmProductRestBookCaptureSink<'a> {
    capture: &'a mut PmPublicCaptureRun,
}

impl<'a> PmProductRestBookCaptureSink<'a> {
    pub(super) const fn new(capture: &'a mut PmPublicCaptureRun) -> Self {
        Self { capture }
    }
}

#[async_trait::async_trait]
impl PmRestBookSnapshotSink for PmProductRestBookCaptureSink<'_> {
    type Output = PmCapturedRestBook;
    type Error = PmPublicCaptureRunError;

    async fn deliver_native_rest_book(
        &mut self,
        purpose: PmRestBookPurpose,
        received: PmRestResponseClock,
        raw: &[u8],
    ) -> Result<Self::Output, Self::Error> {
        let batch = self
            .capture
            .capture_pm_rest_book(
                received.local_wall_receive_ns(),
                received.monotonic_receive_ns(),
                raw,
            )
            .await?;
        Ok(PmCapturedRestBook { purpose, batch })
    }
}
