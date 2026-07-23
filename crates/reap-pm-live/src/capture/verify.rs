use super::*;

pub fn verify_pm_public_capture(
    path: &Path,
    expected_header: &PmCaptureHeader,
) -> Result<PmCaptureVerification, PmCaptureVerifyError> {
    validate_header(expected_header)?;
    let mut state = VerificationState::new(expected_header);
    let scan = scan_jsonl_file_bounded_total(
        path,
        MAX_PM_PUBLIC_CAPTURE_FRAME_BYTES,
        MAX_PM_PUBLIC_CAPTURE_ENCODED_BYTES,
        |frame| state.validate(frame),
    )
    .map_err(|error| match error {
        JsonlVerifyError::InputTooLarge { .. } => PmCaptureVerifyError::CaptureTooLarge,
        error => PmCaptureVerifyError::Framing(error),
    })?;
    state.finish(&scan)?;
    Ok(PmCaptureVerification {
        schema_version: PM_PUBLIC_CAPTURE_SCHEMA_VERSION,
        product: PM_PUBLIC_CAPTURE_PRODUCT.to_string(),
        artifact_sha256: scan.sha256,
        structural_scope_sha256: expected_header.structural_scope_sha256.clone(),
        records: scan.records,
        bytes: scan.bytes,
        raw_public_frames: state.raw_public_frames,
        okx_raw_public_frames: state.okx_raw_public_frames,
        raw_payload_bytes: state.raw_payload_bytes,
        lifecycle_records: state.lifecycle_records,
        okx_lifecycle_records: state.okx_lifecycle_records,
        freshness_timers: state.freshness_timers,
        production_order_entry_authorized: false,
    })
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum LifecyclePhase {
    AwaitingConnection,
    Connected,
    Subscribed,
    Disconnected,
}

#[derive(Debug, Clone, Copy)]
struct LifecycleTracker {
    epoch: u64,
    phase: LifecyclePhase,
}

impl LifecycleTracker {
    const fn new(epoch: u64) -> Self {
        Self {
            epoch,
            phase: LifecyclePhase::AwaitingConnection,
        }
    }

    fn connection_started(&mut self, epoch: u64) -> bool {
        if epoch != self.epoch || self.phase != LifecyclePhase::AwaitingConnection {
            return false;
        }
        self.phase = LifecyclePhase::Connected;
        true
    }

    fn subscription_sent(&mut self, epoch: u64) -> bool {
        if epoch != self.epoch || self.phase != LifecyclePhase::Connected {
            return false;
        }
        self.phase = LifecyclePhase::Subscribed;
        true
    }

    fn disconnected(&mut self, epoch: u64) -> bool {
        if epoch != self.epoch
            || !matches!(
                self.phase,
                LifecyclePhase::Connected | LifecyclePhase::Subscribed
            )
        {
            return false;
        }
        self.phase = LifecyclePhase::Disconnected;
        true
    }

    fn reconnect_scheduled(&mut self, epoch: u64, next_epoch: u64, delay_ns: u64) -> bool {
        if epoch != self.epoch
            || self.phase != LifecyclePhase::Disconnected
            || self.epoch.checked_add(1) != Some(next_epoch)
            || delay_ns == 0
        {
            return false;
        }
        self.epoch = next_epoch;
        self.phase = LifecyclePhase::AwaitingConnection;
        true
    }

    const fn accepts_raw(self, epoch: u64) -> bool {
        self.epoch == epoch && matches!(self.phase, LifecyclePhase::Subscribed)
    }

    const fn subscribed(self) -> bool {
        matches!(self.phase, LifecyclePhase::Subscribed)
    }
}

struct VerificationState<'a> {
    expected_header: &'a PmCaptureHeader,
    expected_sequence: u64,
    previous_monotonic_ns: u64,
    pm_lifecycle: LifecycleTracker,
    okx_lifecycle: LifecycleTracker,
    previous_raw_epoch: Option<ConnectionEpoch>,
    previous_raw_ingress: Option<IngressSequence>,
    previous_okx_raw_epoch: Option<u64>,
    previous_okx_raw_ingress: Option<u64>,
    raw_public_frames: u64,
    okx_raw_public_frames: u64,
    lifecycle_records: u64,
    okx_lifecycle_records: u64,
    freshness_timers: u64,
    raw_payload_bytes: u64,
}

impl<'a> VerificationState<'a> {
    const fn new(expected_header: &'a PmCaptureHeader) -> Self {
        Self {
            expected_header,
            expected_sequence: 1,
            previous_monotonic_ns: expected_header.scope.metadata_monotonic_receive_ns,
            pm_lifecycle: LifecycleTracker::new(
                expected_header.session_policy.pm_initial_epoch.value(),
            ),
            okx_lifecycle: LifecycleTracker::new(expected_header.session_policy.okx_initial_epoch),
            previous_raw_epoch: None,
            previous_raw_ingress: None,
            previous_okx_raw_epoch: None,
            previous_okx_raw_ingress: None,
            raw_public_frames: 0,
            okx_raw_public_frames: 0,
            lifecycle_records: 0,
            okx_lifecycle_records: 0,
            freshness_timers: 0,
            raw_payload_bytes: 0,
        }
    }

    fn validate(&mut self, bytes: &[u8]) -> bool {
        let Ok(record) = serde_json::from_slice::<PmPublicCaptureRecord>(jsonl_payload(bytes))
        else {
            return false;
        };
        if record.sequence() != self.expected_sequence
            || self.expected_sequence > MAX_PM_PUBLIC_CAPTURE_RECORDS
        {
            return false;
        }
        if record
            .monotonic_ns()
            .is_some_and(|value| value == 0 || value < self.previous_monotonic_ns)
        {
            return false;
        }
        let valid = match &record {
            PmPublicCaptureRecord::Header { sequence, header } => {
                *sequence == 1 && header.as_ref() == self.expected_header
            }
            PmPublicCaptureRecord::RawPublicFrame { sequence, frame } => {
                *sequence != 1 && self.validate_raw(frame)
            }
            PmPublicCaptureRecord::OkxRawPublicFrame { sequence, frame } => {
                *sequence != 1 && self.validate_okx_raw(frame)
            }
            PmPublicCaptureRecord::Lifecycle {
                sequence,
                source,
                connection_id,
                connection_epoch,
                event,
                ..
            } => {
                *sequence != 1
                    && *source == self.expected_header.scope.source
                    && *connection_id == self.expected_header.scope.connection_id
                    && self.validate_pm_lifecycle(connection_epoch.value(), *event)
            }
            PmPublicCaptureRecord::OkxLifecycle {
                sequence,
                source,
                connection_id,
                connection_epoch,
                event,
                ..
            } => {
                *sequence != 1
                    && *source == self.expected_header.scope.okx_source
                    && *connection_id == self.expected_header.scope.okx_connection_id
                    && self.validate_okx_lifecycle(*connection_epoch, *event)
            }
            PmPublicCaptureRecord::FreshnessTimer { sequence, .. } => {
                *sequence != 1 && self.pm_lifecycle.subscribed()
            }
        };
        if valid {
            if let Some(monotonic_ns) = record.monotonic_ns() {
                self.previous_monotonic_ns = monotonic_ns;
            }
            match record {
                PmPublicCaptureRecord::Header { .. } => {}
                PmPublicCaptureRecord::RawPublicFrame { .. } => {
                    self.raw_public_frames = self.raw_public_frames.saturating_add(1);
                }
                PmPublicCaptureRecord::OkxRawPublicFrame { .. } => {
                    self.okx_raw_public_frames = self.okx_raw_public_frames.saturating_add(1);
                }
                PmPublicCaptureRecord::Lifecycle { .. } => {
                    self.lifecycle_records = self.lifecycle_records.saturating_add(1);
                }
                PmPublicCaptureRecord::OkxLifecycle { .. } => {
                    self.okx_lifecycle_records = self.okx_lifecycle_records.saturating_add(1);
                }
                PmPublicCaptureRecord::FreshnessTimer { .. } => {
                    self.freshness_timers = self.freshness_timers.saturating_add(1);
                }
            }
            self.expected_sequence = self.expected_sequence.saturating_add(1);
        }
        valid
    }

    fn validate_pm_lifecycle(&mut self, epoch: u64, event: PmCaptureLifecycle) -> bool {
        match event {
            PmCaptureLifecycle::ConnectionStarted => self.pm_lifecycle.connection_started(epoch),
            PmCaptureLifecycle::SubscriptionSent => self.pm_lifecycle.subscription_sent(epoch),
            PmCaptureLifecycle::HeartbeatPingSent => self.pm_lifecycle.accepts_raw(epoch),
            PmCaptureLifecycle::Disconnected {
                local_wall_receive_ns,
                ..
            } => local_wall_receive_ns != 0 && self.pm_lifecycle.disconnected(epoch),
            PmCaptureLifecycle::ReconnectScheduled {
                next_epoch,
                delay_ns,
            } => self
                .pm_lifecycle
                .reconnect_scheduled(epoch, next_epoch.value(), delay_ns),
        }
    }

    fn validate_okx_lifecycle(&mut self, epoch: u64, event: OkxCaptureLifecycle) -> bool {
        match event {
            OkxCaptureLifecycle::ConnectionStarted => self.okx_lifecycle.connection_started(epoch),
            OkxCaptureLifecycle::SubscriptionSent => self.okx_lifecycle.subscription_sent(epoch),
            OkxCaptureLifecycle::Disconnected {
                local_wall_receive_ns,
                ..
            } => local_wall_receive_ns != 0 && self.okx_lifecycle.disconnected(epoch),
            OkxCaptureLifecycle::ReconnectScheduled {
                next_epoch,
                delay_ns,
            } => self
                .okx_lifecycle
                .reconnect_scheduled(epoch, next_epoch, delay_ns),
        }
    }

    fn validate_raw(&mut self, frame: &PmRawPublicFrame) -> bool {
        if frame.source != self.expected_header.scope.source
            || frame.connection_id != self.expected_header.scope.connection_id
            || frame.outcome_token != self.expected_header.scope.outcome_token
            || !self
                .pm_lifecycle
                .accepts_raw(frame.connection_epoch.value())
            || frame.local_ingress_sequence.value() == 0
            || frame.local_wall_receive_ns == 0
            || frame.monotonic_receive_ns == 0
        {
            return false;
        }
        let Ok(raw) = frame.decode_raw() else {
            return false;
        };
        let Some(next_raw_bytes) = self
            .raw_payload_bytes
            .checked_add(u64::try_from(raw.len()).expect("bounded raw length"))
        else {
            return false;
        };
        if next_raw_bytes > MAX_PM_PUBLIC_CAPTURE_RAW_PAYLOAD_BYTES {
            return false;
        }
        let ingress_valid = match self.previous_raw_epoch {
            Some(epoch) if epoch == frame.connection_epoch => {
                self.previous_raw_ingress
                    .and_then(|value| value.value().checked_add(1))
                    == Some(frame.local_ingress_sequence.value())
            }
            Some(_) | None => frame.local_ingress_sequence.value() == 1,
        };
        if ingress_valid {
            self.previous_raw_epoch = Some(frame.connection_epoch);
            self.previous_raw_ingress = Some(frame.local_ingress_sequence);
            self.raw_payload_bytes = next_raw_bytes;
        }
        ingress_valid
    }

    fn validate_okx_raw(&mut self, frame: &OkxRawPublicFrame) -> bool {
        if frame.source != self.expected_header.scope.okx_source
            || frame.connection_id != self.expected_header.scope.okx_connection_id
            || frame.reference_instrument != self.expected_header.scope.okx_reference_instrument
            || !self.okx_lifecycle.accepts_raw(frame.connection_epoch)
            || frame.local_ingress_sequence == 0
            || frame.local_wall_receive_ns == 0
            || frame.monotonic_receive_ns == 0
        {
            return false;
        }
        let Ok(raw) = frame.decode_raw() else {
            return false;
        };
        let Some(next_raw_bytes) = self
            .raw_payload_bytes
            .checked_add(u64::try_from(raw.len()).expect("bounded raw length"))
        else {
            return false;
        };
        if next_raw_bytes > MAX_PM_PUBLIC_CAPTURE_RAW_PAYLOAD_BYTES {
            return false;
        }
        let ingress_valid = match self.previous_okx_raw_epoch {
            Some(epoch) if epoch == frame.connection_epoch => {
                self.previous_okx_raw_ingress
                    .and_then(|previous| previous.checked_add(1))
                    == Some(frame.local_ingress_sequence)
            }
            Some(_) | None => frame.local_ingress_sequence == 1,
        };
        if ingress_valid {
            self.previous_okx_raw_epoch = Some(frame.connection_epoch);
            self.previous_okx_raw_ingress = Some(frame.local_ingress_sequence);
            self.raw_payload_bytes = next_raw_bytes;
        }
        ingress_valid
    }

    fn finish(&self, scan: &JsonlFileScan) -> Result<(), PmCaptureVerifyError> {
        if scan.bytes > MAX_PM_PUBLIC_CAPTURE_ENCODED_BYTES {
            return Err(PmCaptureVerifyError::CaptureTooLarge);
        }
        if scan.records == 0 || scan.records > MAX_PM_PUBLIC_CAPTURE_RECORDS {
            return Err(PmCaptureVerifyError::TooManyRecords);
        }
        if self
            .raw_public_frames
            .saturating_add(self.okx_raw_public_frames)
            > MAX_PM_PUBLIC_CAPTURE_RAW_FRAMES
        {
            return Err(PmCaptureVerifyError::TooManyRawFrames);
        }
        if scan.invalid_records != 0 {
            return Err(PmCaptureVerifyError::InvalidRecords);
        }
        if scan.has_trailing_partial_record {
            return Err(PmCaptureVerifyError::TrailingPartialRecord);
        }
        if !scan.stable_while_reading {
            return Err(PmCaptureVerifyError::ChangedWhileReading);
        }
        Ok(())
    }
}
