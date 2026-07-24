use std::time::Duration;

use reap_pm_state::{PmBookTransition, PmPublicReadinessReason};

use crate::capture_roles::{
    OkxPublicCaptureEvent, PmPublicCaptureBatch, PmPublicFreshnessTimerOutcome,
    PmPublicSnapshotFlow,
};
use crate::composition::{
    PmPublicBookPipelineError, PmPublicCaptureRun, PmPublicCaptureRunError,
    PmPublicDataPipelineError, PmPublicLaneEnactError, PmPublicLaneFaultEnactment,
    PmPublicTerminalTickApplyError,
};
use crate::public_routes::{
    OkxPublicReferenceDelivery, PmPublicBookDelivery, PmPublicMetadataDelivery,
};

/// Result of one integrated public ingress operation.
#[derive(Debug)]
pub enum PmProductPublicIngressOutcome<T, U> {
    Enqueued(T),
    ResyncRequired(PmPublicLaneFaultEnactment<U>),
}

/// Failure while normalizing/reducing public ingress or enacting the exact
/// authenticated lane failure returned by that same operation.
#[derive(Debug)]
pub enum PmProductPublicIngressError<D> {
    Run(PmPublicCaptureRunError),
    Enact(PmPublicLaneEnactError<D>),
}

/// Narrow socket/capture ingress for an integrated PM product.
///
/// Unlike a standalone [`PmPublicCaptureRun`], this handle cannot service or
/// age the public lane and cannot enact arbitrary lane failures. The complete
/// product scheduler remains the sole consumer of queued public observations.
pub struct PmProductPublicIngress<'a> {
    capture: &'a mut PmPublicCaptureRun,
}

impl<'a> PmProductPublicIngress<'a> {
    pub(super) const fn new(capture: &'a mut PmPublicCaptureRun) -> Self {
        Self { capture }
    }

    pub async fn record_pm_connection_started(
        &mut self,
        monotonic_ns: u64,
    ) -> Result<(), PmPublicCaptureRunError> {
        self.capture
            .record_pm_connection_started(monotonic_ns)
            .await
    }

    pub async fn record_okx_connection_started(
        &mut self,
        monotonic_ns: u64,
    ) -> Result<(), PmPublicCaptureRunError> {
        self.capture
            .record_okx_connection_started(monotonic_ns)
            .await
    }

    pub async fn record_pm_subscription_sent(
        &mut self,
        monotonic_ns: u64,
    ) -> Result<(), PmPublicCaptureRunError> {
        self.capture.record_pm_subscription_sent(monotonic_ns).await
    }

    pub async fn record_okx_subscription_sent(
        &mut self,
        monotonic_ns: u64,
    ) -> Result<(), PmPublicCaptureRunError> {
        self.capture
            .record_okx_subscription_sent(monotonic_ns)
            .await
    }

    pub async fn issue_and_enqueue_pm_metadata(
        &mut self,
        local_wall_receive_ns: u64,
    ) -> Result<
        PmProductPublicIngressOutcome<(), reap_polymarket_adapter::PmPublicSessionFault>,
        PmProductPublicIngressError<PmPublicMetadataDelivery>,
    > {
        match self
            .capture
            .issue_and_enqueue_pm_metadata(local_wall_receive_ns)
        {
            Ok(()) => Ok(PmProductPublicIngressOutcome::Enqueued(())),
            Err(PmPublicDataPipelineError::Run(source)) => {
                Err(PmProductPublicIngressError::Run(source))
            }
            Err(PmPublicDataPipelineError::Lane(failure)) => {
                let clock = failure.delivery().envelope().received_clock();
                self.capture
                    .enact_pm_metadata_lane_failure(
                        failure,
                        clock.local_wall_receive_ns(),
                        clock.monotonic_receive_ns(),
                    )
                    .await
                    .map(PmProductPublicIngressOutcome::ResyncRequired)
                    .map_err(PmProductPublicIngressError::Enact)
            }
        }
    }

    pub async fn capture_pm_public(
        &mut self,
        local_wall_receive_ns: u64,
        monotonic_receive_ns: u64,
        raw: &[u8],
    ) -> Result<PmPublicCaptureBatch, PmPublicCaptureRunError> {
        self.capture
            .capture_pm_public(local_wall_receive_ns, monotonic_receive_ns, raw)
            .await
    }

    #[cfg(test)]
    pub(crate) fn phase6_reject_pm_capture_write_for_evidence(
        &mut self,
        source: crate::PmCaptureWriteError,
        local_wall_receive_ns: u64,
        monotonic_receive_ns: u64,
    ) -> PmPublicCaptureRunError {
        self.capture.phase6_reject_pm_capture_write_for_evidence(
            source,
            local_wall_receive_ns,
            monotonic_receive_ns,
        )
    }

    pub async fn capture_okx_public(
        &mut self,
        local_wall_receive_ns: u64,
        monotonic_receive_ns: u64,
        raw: &[u8],
    ) -> Result<
        PmProductPublicIngressOutcome<
            OkxPublicCaptureEvent,
            reap_okx_public_source::OkxPublicSessionFault,
        >,
        PmProductPublicIngressError<OkxPublicReferenceDelivery>,
    > {
        match self
            .capture
            .capture_okx_public(local_wall_receive_ns, monotonic_receive_ns, raw)
            .await
        {
            Ok(event) => Ok(PmProductPublicIngressOutcome::Enqueued(event)),
            Err(PmPublicDataPipelineError::Run(source)) => {
                Err(PmProductPublicIngressError::Run(source))
            }
            Err(PmPublicDataPipelineError::Lane(failure)) => self
                .capture
                .enact_okx_reference_lane_failure(
                    failure,
                    local_wall_receive_ns,
                    monotonic_receive_ns,
                )
                .await
                .map(PmProductPublicIngressOutcome::ResyncRequired)
                .map_err(PmProductPublicIngressError::Enact),
        }
    }

    pub async fn commit_then_enqueue_pm_snapshot(
        &mut self,
        delivery: PmPublicBookDelivery,
        flow: PmPublicSnapshotFlow,
    ) -> Result<
        PmProductPublicIngressOutcome<(), reap_polymarket_adapter::PmPublicSessionFault>,
        PmProductPublicIngressError<PmPublicBookDelivery>,
    > {
        match self.capture.commit_then_enqueue_pm_snapshot(delivery, flow) {
            Ok(()) => Ok(PmProductPublicIngressOutcome::Enqueued(())),
            Err(PmPublicBookPipelineError::Reduce(source)) => {
                Err(PmProductPublicIngressError::Run(source))
            }
            Err(PmPublicBookPipelineError::Lane(failure)) => {
                self.enact_pm_book_lane_failure(failure).await
            }
        }
    }

    pub async fn reduce_then_enqueue_pm_book(
        &mut self,
        delivery: PmPublicBookDelivery,
    ) -> Result<
        PmProductPublicIngressOutcome<
            PmBookTransition,
            reap_polymarket_adapter::PmPublicSessionFault,
        >,
        PmProductPublicIngressError<PmPublicBookDelivery>,
    > {
        match self.capture.reduce_then_enqueue_pm_book(delivery) {
            Ok(transition) => Ok(PmProductPublicIngressOutcome::Enqueued(transition)),
            Err(PmPublicBookPipelineError::Reduce(source)) => {
                Err(PmProductPublicIngressError::Run(source))
            }
            Err(PmPublicBookPipelineError::Lane(failure)) => {
                self.enact_pm_book_lane_failure(failure).await
            }
        }
    }

    pub async fn record_pm_disconnected(
        &mut self,
        local_wall_receive_ns: u64,
        monotonic_ns: u64,
    ) -> Result<reap_polymarket_adapter::PmPublicSessionFault, PmPublicCaptureRunError> {
        self.capture
            .record_pm_disconnected(local_wall_receive_ns, monotonic_ns)
            .await
    }

    pub async fn record_okx_disconnected(
        &mut self,
        local_wall_receive_ns: u64,
        monotonic_ns: u64,
    ) -> Result<reap_okx_public_source::OkxPublicSessionFault, PmPublicCaptureRunError> {
        self.capture
            .record_okx_disconnected(local_wall_receive_ns, monotonic_ns)
            .await
    }

    pub async fn record_pm_reconnect_scheduled(
        &mut self,
        monotonic_ns: u64,
    ) -> Result<Duration, PmPublicCaptureRunError> {
        self.capture
            .record_pm_reconnect_scheduled(monotonic_ns)
            .await
    }

    pub async fn record_okx_reconnect_scheduled(
        &mut self,
        monotonic_ns: u64,
    ) -> Result<Duration, PmPublicCaptureRunError> {
        self.capture
            .record_okx_reconnect_scheduled(monotonic_ns)
            .await
    }

    pub async fn record_pm_heartbeat_ping_sent(
        &mut self,
        local_wall_now_ns: u64,
        monotonic_ns: u64,
    ) -> Result<(), PmPublicCaptureRunError> {
        self.capture
            .record_pm_heartbeat_ping_sent(local_wall_now_ns, monotonic_ns)
            .await
    }

    pub async fn record_freshness_timer(
        &mut self,
        monotonic_ns: u64,
    ) -> Result<PmPublicFreshnessTimerOutcome, PmPublicCaptureRunError> {
        self.capture.record_freshness_timer(monotonic_ns).await
    }

    pub fn apply_terminal_tick_invalidation(
        &mut self,
        delivery: PmPublicBookDelivery,
    ) -> Result<PmPublicReadinessReason, PmPublicTerminalTickApplyError> {
        self.capture.apply_terminal_tick_invalidation(delivery)
    }

    async fn enact_pm_book_lane_failure<T>(
        &mut self,
        failure: crate::composition::PmPublicLaneAdmissionError<PmPublicBookDelivery>,
    ) -> Result<
        PmProductPublicIngressOutcome<T, reap_polymarket_adapter::PmPublicSessionFault>,
        PmProductPublicIngressError<PmPublicBookDelivery>,
    > {
        let clock = failure.delivery().envelope().received_clock();
        self.capture
            .enact_pm_book_lane_failure(
                failure,
                clock.local_wall_receive_ns(),
                clock.monotonic_receive_ns(),
            )
            .await
            .map(PmProductPublicIngressOutcome::ResyncRequired)
            .map_err(PmProductPublicIngressError::Enact)
    }
}
