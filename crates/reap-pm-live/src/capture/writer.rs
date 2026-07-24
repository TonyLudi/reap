use std::path::PathBuf;
use std::time::Duration;

use reap_capture_framing::{
    BoundedJsonlFrameError, BoundedJsonlWriter, BoundedWriterConfig, JsonlWriterShutdown,
    measure_jsonl_frame_bounded,
};
use reap_pm_core::{ConnectionEpoch, IngressSequence};

use super::validation::validate_header;
use super::{
    MAX_PM_PUBLIC_CAPTURE_ENCODED_BYTES, MAX_PM_PUBLIC_CAPTURE_FRAME_BYTES,
    MAX_PM_PUBLIC_CAPTURE_PENDING_AGE_NS, MAX_PM_PUBLIC_CAPTURE_RAW_FRAMES,
    MAX_PM_PUBLIC_CAPTURE_RAW_PAYLOAD_BYTES, MAX_PM_PUBLIC_CAPTURE_RECORDS, OkxCaptureLifecycle,
    OkxRawPublicFrame, PmCaptureHeader, PmCaptureLifecycle, PmCaptureVerifyError,
    PmCaptureWriteError, PmPublicCaptureRecord, PmRawPublicFrame,
};

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum CaptureWriterLifecyclePhase {
    AwaitingConnection,
    Connected,
    Subscribed,
    Disconnected,
}

#[derive(Debug, Clone, Copy)]
struct CaptureWriterLifecycle {
    epoch: u64,
    phase: CaptureWriterLifecyclePhase,
}

impl CaptureWriterLifecycle {
    const fn new(epoch: u64) -> Self {
        Self {
            epoch,
            phase: CaptureWriterLifecyclePhase::AwaitingConnection,
        }
    }

    const fn accepts_raw(self, epoch: u64) -> bool {
        self.epoch == epoch && matches!(self.phase, CaptureWriterLifecyclePhase::Subscribed)
    }

    fn preview_pm(
        self,
        epoch: u64,
        event: PmCaptureLifecycle,
    ) -> Result<Self, PmCaptureVerifyError> {
        let (next_epoch, next_phase) = match event {
            PmCaptureLifecycle::ConnectionStarted
                if epoch == self.epoch
                    && self.phase == CaptureWriterLifecyclePhase::AwaitingConnection =>
            {
                (self.epoch, CaptureWriterLifecyclePhase::Connected)
            }
            PmCaptureLifecycle::SubscriptionSent
                if epoch == self.epoch && self.phase == CaptureWriterLifecyclePhase::Connected =>
            {
                (self.epoch, CaptureWriterLifecyclePhase::Subscribed)
            }
            PmCaptureLifecycle::HeartbeatPingSent
                if epoch == self.epoch && self.phase == CaptureWriterLifecyclePhase::Subscribed =>
            {
                (self.epoch, self.phase)
            }
            PmCaptureLifecycle::Disconnected {
                local_wall_receive_ns,
                ..
            } if local_wall_receive_ns != 0
                && epoch == self.epoch
                && matches!(
                    self.phase,
                    CaptureWriterLifecyclePhase::Connected
                        | CaptureWriterLifecyclePhase::Subscribed
                ) =>
            {
                (self.epoch, CaptureWriterLifecyclePhase::Disconnected)
            }
            PmCaptureLifecycle::ReconnectScheduled {
                next_epoch,
                delay_ns,
            } if delay_ns != 0
                && epoch == self.epoch
                && self.phase == CaptureWriterLifecyclePhase::Disconnected
                && self.epoch.checked_add(1) == Some(next_epoch.value()) =>
            {
                (
                    next_epoch.value(),
                    CaptureWriterLifecyclePhase::AwaitingConnection,
                )
            }
            _ => return Err(PmCaptureVerifyError::InvalidLifecycle),
        };
        Ok(Self {
            epoch: next_epoch,
            phase: next_phase,
        })
    }

    fn preview_okx(
        self,
        epoch: u64,
        event: OkxCaptureLifecycle,
    ) -> Result<Self, PmCaptureVerifyError> {
        let (next_epoch, next_phase) = match event {
            OkxCaptureLifecycle::ConnectionStarted
                if epoch == self.epoch
                    && self.phase == CaptureWriterLifecyclePhase::AwaitingConnection =>
            {
                (self.epoch, CaptureWriterLifecyclePhase::Connected)
            }
            OkxCaptureLifecycle::SubscriptionSent
                if epoch == self.epoch && self.phase == CaptureWriterLifecyclePhase::Connected =>
            {
                (self.epoch, CaptureWriterLifecyclePhase::Subscribed)
            }
            OkxCaptureLifecycle::Disconnected {
                local_wall_receive_ns,
                ..
            } if local_wall_receive_ns != 0
                && epoch == self.epoch
                && matches!(
                    self.phase,
                    CaptureWriterLifecyclePhase::Connected
                        | CaptureWriterLifecyclePhase::Subscribed
                ) =>
            {
                (self.epoch, CaptureWriterLifecyclePhase::Disconnected)
            }
            OkxCaptureLifecycle::ReconnectScheduled {
                next_epoch,
                delay_ns,
            } if delay_ns != 0
                && epoch == self.epoch
                && self.phase == CaptureWriterLifecyclePhase::Disconnected
                && self.epoch.checked_add(1) == Some(next_epoch) =>
            {
                (next_epoch, CaptureWriterLifecyclePhase::AwaitingConnection)
            }
            _ => return Err(PmCaptureVerifyError::InvalidLifecycle),
        };
        Ok(Self {
            epoch: next_epoch,
            phase: next_phase,
        })
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
struct RawCaptureCapacity {
    frames: u64,
    payload_bytes: u64,
}

impl RawCaptureCapacity {
    fn preflight(self, raw_length: usize) -> Result<u64, PmCaptureVerifyError> {
        if raw_length > super::MAX_PM_RAW_PUBLIC_FRAME_BYTES {
            return Err(PmCaptureVerifyError::RawFrameTooLarge);
        }
        if self.frames >= MAX_PM_PUBLIC_CAPTURE_RAW_FRAMES {
            return Err(PmCaptureVerifyError::TooManyRawFrames);
        }
        let raw_length =
            u64::try_from(raw_length).map_err(|_| PmCaptureVerifyError::RawPayloadTooLarge)?;
        let next_payload_bytes = self
            .payload_bytes
            .checked_add(raw_length)
            .ok_or(PmCaptureVerifyError::RawPayloadTooLarge)?;
        if next_payload_bytes > MAX_PM_PUBLIC_CAPTURE_RAW_PAYLOAD_BYTES {
            return Err(PmCaptureVerifyError::RawPayloadTooLarge);
        }
        Ok(next_payload_bytes)
    }

    fn commit(&mut self, next_payload_bytes: u64) {
        debug_assert!(next_payload_bytes >= self.payload_bytes);
        self.payload_bytes = next_payload_bytes;
        self.frames = self.frames.saturating_add(1);
    }
}

#[derive(Debug)]
struct PendingCaptureRecordTimes {
    monotonic_ns: Box<[u64]>,
    head: usize,
    len: usize,
}

impl PendingCaptureRecordTimes {
    fn new() -> Self {
        Self {
            monotonic_ns: vec![0; MAX_PM_PUBLIC_CAPTURE_RAW_FRAMES as usize].into_boxed_slice(),
            head: 0,
            len: 0,
        }
    }

    fn initialize_after_header(
        &mut self,
        writer: &BoundedJsonlWriter<PmPublicCaptureRecord>,
        monotonic_ns: u64,
    ) -> Result<(), PmCaptureWriteError> {
        let writer_depth = writer.queue_entry_evidence().current_depth;
        if writer_depth > 1 {
            return Err(PmCaptureWriteError::CaptureQueueEvidenceMismatch {
                writer_depth,
                tracked_depth: 1,
            });
        }
        if writer_depth == 1 {
            self.push_back(monotonic_ns)?;
        }
        Ok(())
    }

    fn preflight(
        &mut self,
        writer: &BoundedJsonlWriter<PmPublicCaptureRecord>,
        monotonic_ns: u64,
    ) -> Result<usize, PmCaptureWriteError> {
        let writer_depth = writer.queue_entry_evidence().current_depth;
        self.preflight_depth(writer_depth, monotonic_ns)
    }

    fn preflight_depth(
        &mut self,
        writer_depth: usize,
        monotonic_ns: u64,
    ) -> Result<usize, PmCaptureWriteError> {
        self.synchronize(writer_depth)?;
        let Some(oldest_monotonic_ns) = self.oldest() else {
            return Ok(writer_depth);
        };
        let Some(observed_age_ns) = monotonic_ns.checked_sub(oldest_monotonic_ns) else {
            return Err(PmCaptureWriteError::CaptureQueueClockRegression {
                observed_monotonic_ns: monotonic_ns,
                oldest_monotonic_ns,
            });
        };
        if observed_age_ns > MAX_PM_PUBLIC_CAPTURE_PENDING_AGE_NS {
            return Err(PmCaptureWriteError::CaptureAged {
                observed_age_ns,
                maximum_age_ns: MAX_PM_PUBLIC_CAPTURE_PENDING_AGE_NS,
            });
        }
        Ok(writer_depth)
    }

    fn record_successful_send(
        &mut self,
        writer: &BoundedJsonlWriter<PmPublicCaptureRecord>,
        prior_writer_depth: usize,
        monotonic_ns: u64,
    ) -> Result<(), PmCaptureWriteError> {
        debug_assert_eq!(self.len, prior_writer_depth);
        let writer_depth = writer.queue_entry_evidence().current_depth;
        let tracked_with_new = prior_writer_depth.checked_add(1).ok_or(
            PmCaptureWriteError::CaptureQueueEvidenceMismatch {
                writer_depth,
                tracked_depth: prior_writer_depth,
            },
        )?;
        if writer_depth > tracked_with_new {
            return Err(PmCaptureWriteError::CaptureQueueEvidenceMismatch {
                writer_depth,
                tracked_depth: tracked_with_new,
            });
        }
        let completed = tracked_with_new - writer_depth;
        let completed_old = completed.min(prior_writer_depth);
        for _ in 0..completed_old {
            self.pop_front();
        }
        if completed <= prior_writer_depth {
            self.push_back(monotonic_ns)?;
        }
        debug_assert_eq!(self.len, writer_depth);
        Ok(())
    }

    fn synchronize(&mut self, writer_depth: usize) -> Result<(), PmCaptureWriteError> {
        if writer_depth > self.len {
            return Err(PmCaptureWriteError::CaptureQueueEvidenceMismatch {
                writer_depth,
                tracked_depth: self.len,
            });
        }
        while self.len > writer_depth {
            self.pop_front();
        }
        Ok(())
    }

    fn push_back(&mut self, monotonic_ns: u64) -> Result<(), PmCaptureWriteError> {
        if self.len == self.monotonic_ns.len() {
            return Err(PmCaptureWriteError::CaptureQueueEvidenceMismatch {
                writer_depth: self.len.saturating_add(1),
                tracked_depth: self.len,
            });
        }
        let tail = (self.head + self.len) % self.monotonic_ns.len();
        self.monotonic_ns[tail] = monotonic_ns;
        self.len += 1;
        Ok(())
    }

    fn pop_front(&mut self) {
        debug_assert!(self.len != 0);
        self.monotonic_ns[self.head] = 0;
        self.head = (self.head + 1) % self.monotonic_ns.len();
        self.len -= 1;
    }

    fn oldest(&self) -> Option<u64> {
        (self.len != 0).then(|| self.monotonic_ns[self.head])
    }

    fn reserved_capacity_bytes(&self) -> usize {
        std::mem::size_of_val(self.monotonic_ns.as_ref())
    }
}

#[cfg(test)]
pub(crate) struct Phase6RawCapacityProbe {
    capacity: RawCaptureCapacity,
}

#[cfg(test)]
impl Phase6RawCapacityProbe {
    pub(crate) const fn new() -> Self {
        Self {
            capacity: RawCaptureCapacity {
                frames: 0,
                payload_bytes: 0,
            },
        }
    }

    pub(crate) fn attempt(&mut self, raw_length: usize) -> Result<(), PmCaptureVerifyError> {
        let next = self.capacity.preflight(raw_length)?;
        self.capacity.commit(next);
        Ok(())
    }

    pub(crate) const fn frames(&self) -> u64 {
        self.capacity.frames
    }

    pub(crate) const fn payload_bytes(&self) -> u64 {
        self.capacity.payload_bytes
    }
}

#[derive(Debug)]
pub(crate) struct PmPublicCaptureWriter {
    header: PmCaptureHeader,
    sequence: u64,
    encoded_bytes: u64,
    raw_capacity: RawCaptureCapacity,
    previous_monotonic_ns: u64,
    pm_lifecycle: CaptureWriterLifecycle,
    okx_lifecycle: CaptureWriterLifecycle,
    previous_pm_raw_epoch: Option<u64>,
    previous_pm_raw_ingress: Option<u64>,
    previous_okx_raw_epoch: Option<u64>,
    previous_okx_raw_ingress: Option<u64>,
    pending_record_times: PendingCaptureRecordTimes,
    writer: BoundedJsonlWriter<PmPublicCaptureRecord>,
}

impl PmPublicCaptureWriter {
    #[cfg(test)]
    pub(crate) fn pending_record_depth_for_evidence(&self) -> usize {
        self.writer.queue_entry_evidence().current_depth
    }

    pub(crate) fn reserved_capacity_bytes(&self) -> usize {
        // The raw writer's entry and byte ceilings are admission limits, not
        // allocated payload storage. This projection therefore counts only
        // deterministic owner-side preallocation.
        self.pending_record_times.reserved_capacity_bytes()
    }

    pub(crate) async fn start(
        path: PathBuf,
        header: PmCaptureHeader,
    ) -> Result<Self, PmCaptureWriteError> {
        validate_header(&header)?;
        let writer =
            BoundedJsonlWriter::start("pm-public-capture", path, bounded_writer_config()).await?;
        let header_record = PmPublicCaptureRecord::Header {
            sequence: 1,
            header: Box::new(header.clone()),
        };
        let encoded_bytes = measured_record_bytes(&header_record)?;
        writer.send(header_record).await?;
        let previous_monotonic_ns = header.scope.metadata_monotonic_receive_ns;
        let mut pending_record_times = PendingCaptureRecordTimes::new();
        pending_record_times.initialize_after_header(&writer, previous_monotonic_ns)?;
        let policy = header.session_policy;
        Ok(Self {
            header,
            sequence: 1,
            encoded_bytes,
            raw_capacity: RawCaptureCapacity::default(),
            previous_monotonic_ns,
            pm_lifecycle: CaptureWriterLifecycle::new(policy.pm_initial_epoch.value()),
            okx_lifecycle: CaptureWriterLifecycle::new(policy.okx_initial_epoch),
            previous_pm_raw_epoch: None,
            previous_pm_raw_ingress: None,
            previous_okx_raw_epoch: None,
            previous_okx_raw_ingress: None,
            pending_record_times,
            writer,
        })
    }

    #[allow(clippy::too_many_arguments)]
    pub(crate) async fn capture_raw_before_parse(
        &mut self,
        connection_epoch: ConnectionEpoch,
        local_ingress_sequence: IngressSequence,
        local_wall_receive_ns: u64,
        monotonic_receive_ns: u64,
        raw_bytes: &[u8],
    ) -> Result<(), PmCaptureWriteError> {
        self.preflight_raw_admission(raw_bytes.len())?;
        self.preflight_pm_raw(
            connection_epoch.value(),
            local_ingress_sequence.value(),
            local_wall_receive_ns,
            monotonic_receive_ns,
        )?;
        let frame = PmRawPublicFrame::new(
            self.header.scope(),
            connection_epoch,
            local_ingress_sequence,
            local_wall_receive_ns,
            monotonic_receive_ns,
            raw_bytes,
        )?;
        self.send_raw_record(
            u64::from(frame.raw_length),
            monotonic_receive_ns,
            |sequence| PmPublicCaptureRecord::RawPublicFrame { sequence, frame },
        )
        .await?;
        self.previous_monotonic_ns = monotonic_receive_ns;
        self.previous_pm_raw_epoch = Some(connection_epoch.value());
        self.previous_pm_raw_ingress = Some(local_ingress_sequence.value());
        Ok(())
    }

    #[allow(clippy::too_many_arguments)]
    pub(crate) async fn capture_okx_raw_before_parse(
        &mut self,
        connection_epoch: u64,
        local_ingress_sequence: u64,
        local_wall_receive_ns: u64,
        monotonic_receive_ns: u64,
        raw_bytes: &[u8],
    ) -> Result<u64, PmCaptureWriteError> {
        self.preflight_raw_admission(raw_bytes.len())?;
        self.preflight_okx_raw(
            connection_epoch,
            local_ingress_sequence,
            local_wall_receive_ns,
            monotonic_receive_ns,
        )?;
        let frame = OkxRawPublicFrame::new(
            self.header.scope(),
            connection_epoch,
            local_ingress_sequence,
            local_wall_receive_ns,
            monotonic_receive_ns,
            raw_bytes,
        )?;
        let raw_hash = frame.raw_hash;
        self.send_raw_record(
            u64::from(frame.raw_length),
            monotonic_receive_ns,
            |sequence| PmPublicCaptureRecord::OkxRawPublicFrame { sequence, frame },
        )
        .await?;
        self.previous_monotonic_ns = monotonic_receive_ns;
        self.previous_okx_raw_epoch = Some(connection_epoch);
        self.previous_okx_raw_ingress = Some(local_ingress_sequence);
        Ok(raw_hash)
    }

    pub(crate) fn preflight_pm_lifecycle(
        &self,
        connection_epoch: ConnectionEpoch,
        monotonic_ns: u64,
        event: PmCaptureLifecycle,
    ) -> Result<(), PmCaptureWriteError> {
        self.preview_pm_lifecycle(connection_epoch, monotonic_ns, event)
            .map(|_| ())
    }

    pub(crate) fn preflight_okx_lifecycle(
        &self,
        connection_epoch: u64,
        monotonic_ns: u64,
        event: OkxCaptureLifecycle,
    ) -> Result<(), PmCaptureWriteError> {
        self.preview_okx_lifecycle(connection_epoch, monotonic_ns, event)
            .map(|_| ())
    }

    pub(crate) async fn record_lifecycle(
        &mut self,
        connection_epoch: ConnectionEpoch,
        monotonic_ns: u64,
        event: PmCaptureLifecycle,
    ) -> Result<(), PmCaptureWriteError> {
        let next_lifecycle = self.preview_pm_lifecycle(connection_epoch, monotonic_ns, event)?;
        let sequence = self.next_sequence()?;
        let record = PmPublicCaptureRecord::Lifecycle {
            sequence,
            source: self.header.scope.source,
            connection_id: self.header.scope.connection_id,
            connection_epoch,
            monotonic_ns,
            event,
        };
        self.send_record(record, monotonic_ns).await?;
        self.sequence = sequence;
        self.previous_monotonic_ns = monotonic_ns;
        self.pm_lifecycle = next_lifecycle;
        Ok(())
    }

    pub(crate) async fn record_okx_lifecycle(
        &mut self,
        connection_epoch: u64,
        monotonic_ns: u64,
        event: OkxCaptureLifecycle,
    ) -> Result<(), PmCaptureWriteError> {
        let next_lifecycle = self.preview_okx_lifecycle(connection_epoch, monotonic_ns, event)?;
        let sequence = self.next_sequence()?;
        let record = PmPublicCaptureRecord::OkxLifecycle {
            sequence,
            source: self.header.scope.okx_source,
            connection_id: self.header.scope.okx_connection_id,
            connection_epoch,
            monotonic_ns,
            event,
        };
        self.send_record(record, monotonic_ns).await?;
        self.sequence = sequence;
        self.previous_monotonic_ns = monotonic_ns;
        self.okx_lifecycle = next_lifecycle;
        Ok(())
    }

    pub(crate) async fn record_freshness_timer(
        &mut self,
        monotonic_ns: u64,
    ) -> Result<(), PmCaptureWriteError> {
        self.preflight_freshness_timer(monotonic_ns)?;
        let sequence = self.next_sequence()?;
        let record = PmPublicCaptureRecord::FreshnessTimer {
            sequence,
            monotonic_ns,
        };
        self.send_record(record, monotonic_ns).await?;
        self.sequence = sequence;
        self.previous_monotonic_ns = monotonic_ns;
        Ok(())
    }

    pub(crate) fn preflight_freshness_timer(
        &self,
        monotonic_ns: u64,
    ) -> Result<(), PmCaptureWriteError> {
        if monotonic_ns == 0
            || monotonic_ns < self.previous_monotonic_ns
            || !matches!(
                self.pm_lifecycle.phase,
                CaptureWriterLifecyclePhase::Subscribed
            )
        {
            return Err(PmCaptureVerifyError::InvalidFreshnessTimer.into());
        }
        self.next_sequence().map(|_| ())
    }

    pub(crate) async fn finish(self) -> Result<JsonlWriterShutdown, PmCaptureWriteError> {
        Ok(self.writer.shutdown_with_evidence().await?)
    }

    fn next_sequence(&self) -> Result<u64, PmCaptureWriteError> {
        let next = self
            .sequence
            .checked_add(1)
            .ok_or(PmCaptureVerifyError::TooManyRecords)?;
        if next > MAX_PM_PUBLIC_CAPTURE_RECORDS {
            return Err(PmCaptureVerifyError::TooManyRecords.into());
        }
        Ok(next)
    }

    fn preview_pm_lifecycle(
        &self,
        connection_epoch: ConnectionEpoch,
        monotonic_ns: u64,
        event: PmCaptureLifecycle,
    ) -> Result<CaptureWriterLifecycle, PmCaptureWriteError> {
        if connection_epoch.value() == 0
            || monotonic_ns == 0
            || monotonic_ns < self.previous_monotonic_ns
        {
            return Err(PmCaptureVerifyError::InvalidLifecycle.into());
        }
        Ok(self
            .pm_lifecycle
            .preview_pm(connection_epoch.value(), event)?)
    }

    fn preview_okx_lifecycle(
        &self,
        connection_epoch: u64,
        monotonic_ns: u64,
        event: OkxCaptureLifecycle,
    ) -> Result<CaptureWriterLifecycle, PmCaptureWriteError> {
        if connection_epoch == 0 || monotonic_ns == 0 || monotonic_ns < self.previous_monotonic_ns {
            return Err(PmCaptureVerifyError::InvalidLifecycle.into());
        }
        Ok(self.okx_lifecycle.preview_okx(connection_epoch, event)?)
    }

    fn preflight_pm_raw(
        &self,
        connection_epoch: u64,
        ingress: u64,
        local_wall_receive_ns: u64,
        monotonic_receive_ns: u64,
    ) -> Result<(), PmCaptureWriteError> {
        if local_wall_receive_ns == 0
            || monotonic_receive_ns == 0
            || monotonic_receive_ns < self.previous_monotonic_ns
            || !self.pm_lifecycle.accepts_raw(connection_epoch)
            || !next_raw_ingress_is_valid(
                self.previous_pm_raw_epoch,
                self.previous_pm_raw_ingress,
                connection_epoch,
                ingress,
            )
        {
            return Err(PmCaptureVerifyError::InvalidRawFrame(
                "PM raw frame does not match the active writer lifecycle",
            )
            .into());
        }
        Ok(())
    }

    fn preflight_okx_raw(
        &self,
        connection_epoch: u64,
        ingress: u64,
        local_wall_receive_ns: u64,
        monotonic_receive_ns: u64,
    ) -> Result<(), PmCaptureWriteError> {
        if local_wall_receive_ns == 0
            || monotonic_receive_ns == 0
            || monotonic_receive_ns < self.previous_monotonic_ns
            || !self.okx_lifecycle.accepts_raw(connection_epoch)
            || !next_raw_ingress_is_valid(
                self.previous_okx_raw_epoch,
                self.previous_okx_raw_ingress,
                connection_epoch,
                ingress,
            )
        {
            return Err(PmCaptureVerifyError::InvalidRawFrame(
                "OKX raw frame does not match the active writer lifecycle",
            )
            .into());
        }
        Ok(())
    }

    fn preflight_raw_admission(&self, raw_length: usize) -> Result<(), PmCaptureWriteError> {
        self.next_sequence()?;
        self.raw_capacity.preflight(raw_length)?;
        Ok(())
    }

    async fn send_record(
        &mut self,
        record: PmPublicCaptureRecord,
        monotonic_ns: u64,
    ) -> Result<(), PmCaptureWriteError> {
        let prior_writer_depth = self
            .pending_record_times
            .preflight(&self.writer, monotonic_ns)?;
        let frame_bytes = measured_record_bytes(&record)?;
        let total = self
            .encoded_bytes
            .checked_add(frame_bytes)
            .ok_or(PmCaptureVerifyError::CaptureTooLarge)?;
        if total > MAX_PM_PUBLIC_CAPTURE_ENCODED_BYTES {
            return Err(PmCaptureVerifyError::CaptureTooLarge.into());
        }
        self.writer.send(record).await?;
        self.pending_record_times.record_successful_send(
            &self.writer,
            prior_writer_depth,
            monotonic_ns,
        )?;
        self.encoded_bytes = total;
        Ok(())
    }

    async fn send_raw_record(
        &mut self,
        raw_length: u64,
        monotonic_ns: u64,
        make_record: impl FnOnce(u64) -> PmPublicCaptureRecord,
    ) -> Result<(), PmCaptureWriteError> {
        let raw_length =
            usize::try_from(raw_length).map_err(|_| PmCaptureVerifyError::RawPayloadTooLarge)?;
        let next_raw_bytes = self.raw_capacity.preflight(raw_length)?;
        let sequence = self.next_sequence()?;
        self.send_record(make_record(sequence), monotonic_ns)
            .await?;
        self.sequence = sequence;
        self.raw_capacity.commit(next_raw_bytes);
        Ok(())
    }
}

fn next_raw_ingress_is_valid(
    previous_epoch: Option<u64>,
    previous_ingress: Option<u64>,
    epoch: u64,
    ingress: u64,
) -> bool {
    if ingress == 0 {
        return false;
    }
    match previous_epoch {
        Some(previous_epoch) if previous_epoch == epoch => {
            previous_ingress.and_then(|value| value.checked_add(1)) == Some(ingress)
        }
        Some(_) | None => ingress == 1,
    }
}

fn bounded_writer_config() -> BoundedWriterConfig {
    BoundedWriterConfig {
        capacity: MAX_PM_PUBLIC_CAPTURE_RAW_FRAMES as usize,
        max_frame_bytes: MAX_PM_PUBLIC_CAPTURE_FRAME_BYTES,
        max_queued_bytes: MAX_PM_PUBLIC_CAPTURE_ENCODED_BYTES as usize,
        flush_every_records: 1,
        fsync_every_records: 0,
        enqueue_timeout: Duration::from_secs(1),
        shutdown_timeout: Duration::from_secs(30),
        abort_timeout: Duration::from_secs(1),
        evidence_scan_timeout: Duration::from_secs(5),
    }
}

fn measured_record_bytes(record: &PmPublicCaptureRecord) -> Result<u64, BoundedJsonlFrameError> {
    let bytes = measure_jsonl_frame_bounded(record, MAX_PM_PUBLIC_CAPTURE_FRAME_BYTES)?;
    Ok(u64::try_from(bytes).expect("bounded JSONL frame length"))
}

#[cfg(test)]
mod phase6_overload_tests;
