use std::collections::{BTreeMap, BTreeSet, HashMap, HashSet};
use std::path::Path;
use std::sync::Arc;
use std::time::{Duration, Instant};

use crate::analysis;
use crate::cleanliness::{CaptureCleanRunInputs, capture_run_is_clean};
use crate::configuration::{CaptureConfig, CaptureRuntimeConfig};
use crate::error::{
    CaptureError, MAX_CAPTURE_FAILURE_MESSAGE_BYTES, combine_capture_failures,
    combine_capture_lifecycle_errors, truncate_utf8,
};
use crate::report::{
    CAPTURE_RUN_REPORT_FORMAT_VERSION, CaptureBookHealth, CaptureConfigFileEvidence,
    CaptureFailureEvidence, CaptureRunReport, CaptureStopReason,
};
use crate::writer::{JsonlWriter, JsonlWriterShutdown, JsonlWriterStats};
use reap_book::BookStatus;
use reap_core::{
    Channel, NormalizedEvent, PINNED_JAVA_REVISION, RawEnvelope, SystemEventKind, Venue,
};
use reap_feed::{
    ConnectionAttemptPacer, ConnectionStatus, ConnectionStatusKind, FeedOutput, FeedProcessor,
    RawCapture, ReconnectPolicy, RecoveryRequest, SequenceStatus, SocketPlan, no_bootstrap,
    spawn_supervised_feed,
};
pub use reap_telemetry::{HostGuardConfig, HostHealthSnapshot};
use reap_telemetry::{
    HostGuardRuntime, HostGuardStats, HostHealthError, check_host_health,
    current_executable_sha256, host_identity_sha256, start_host_guard,
};
use reap_venue::{VenueAdapter, okx::OkxAdapter};
use serde_json::Value;
use tokio::sync::mpsc;

const FEED_SHUTDOWN_TIMEOUT: Duration = Duration::from_secs(5);
const HOST_GUARD_SHUTDOWN_TIMEOUT: Duration = Duration::from_secs(5);

#[derive(Debug, Clone)]
pub struct CaptureRunOptions {
    pub run_duration: Option<Duration>,
    pub config_source: Option<CaptureConfigFileEvidence>,
}

pub async fn run_capture_path(
    path: impl AsRef<Path>,
    mut options: CaptureRunOptions,
) -> Result<CaptureRunReport, CaptureError> {
    let (config, config_source) = CaptureConfig::load_with_evidence(path)?;
    options.config_source = Some(config_source);
    run_capture(config, options).await
}

pub fn capture_startup_failure_report(
    config: &CaptureConfig,
    config_source: Option<CaptureConfigFileEvidence>,
    error: &CaptureError,
) -> CaptureRunReport {
    let completed_at_ms = unix_time_ms();
    // Setup failures happen before feed startup. Empty evidence avoids binding a
    // pre-existing output that this process never successfully created.
    let raw = JsonlWriterStats::empty();
    let normalized = JsonlWriterStats::empty();
    let mut books = config
        .expected_book_symbols()
        .into_iter()
        .map(|symbol| CaptureBookHealth {
            symbol,
            sequence_status: "awaiting_snapshot".to_string(),
            book_status: "empty".to_string(),
            last_seq_id: None,
            buffered_updates: 0,
            sequence_resets: 0,
            same_sequence_updates: 0,
            best_bid: None,
            best_ask: None,
        })
        .collect::<Vec<_>>();
    books.sort_by(|left, right| left.symbol.cmp(&right.symbol));
    let expected_connections = config.socket_plans().map_or(0, |plans| plans.len());
    CaptureRunReport {
        format_version: CAPTURE_RUN_REPORT_FORMAT_VERSION,
        reap_version: env!("CARGO_PKG_VERSION").to_string(),
        java_reference_revision: PINNED_JAVA_REVISION.to_string(),
        executable_sha256: current_executable_sha256().unwrap_or_default(),
        host_identity_sha256: None,
        host_preflight: None,
        host_periodic_checks: 0,
        host_last_snapshot: None,
        session_started_at_ms: completed_at_ms,
        session_completed_at_ms: completed_at_ms,
        capture_session_id: format!(
            "failed-startup-{:x}-{:x}",
            reap_feed::unix_time_ns(),
            std::process::id()
        ),
        config_fingerprint: config.fingerprint().unwrap_or_default(),
        config_source,
        stop_reason: CaptureStopReason::RuntimeFailure,
        elapsed_ms: 0,
        raw_path: config.output.raw_path.clone(),
        normalized_path: config.output.normalized_path.clone(),
        raw_records: raw.records,
        normalized_records: normalized.records,
        raw_bytes: raw.bytes,
        normalized_bytes: normalized.bytes,
        raw_sha256: raw.sha256,
        normalized_sha256: config
            .output
            .normalized_path
            .as_ref()
            .map(|_| normalized.sha256),
        max_raw_queue_depth: raw.max_queue_depth,
        max_normalized_queue_depth: normalized.max_queue_depth,
        parsed_events: 0,
        accepted_events: 0,
        duplicates: 0,
        gaps: 0,
        recoveries: 0,
        recovery_failures: 0,
        sequence_resets: 0,
        same_sequence_updates: 0,
        recovery_requests: 0,
        missing_recovery_routes: 0,
        parse_errors: 0,
        stale_book_events: 0,
        connection_disconnects: 0,
        expected_connections,
        ready_connections_at_stop: 0,
        reached_all_connections_ready: false,
        books,
        failure: Some(capture_failure_evidence(error)),
        clean_capture: false,
    }
}

pub async fn run_capture(
    config: CaptureConfig,
    options: CaptureRunOptions,
) -> Result<CaptureRunReport, CaptureError> {
    config.ensure_valid()?;
    let config_fingerprint = config.fingerprint()?;
    let CaptureRunOptions {
        run_duration,
        config_source,
    } = options;
    if run_duration.is_some_and(|duration| duration.is_zero()) {
        return Err(CaptureError::InvalidConfig(
            "capture duration must be positive".to_string(),
        ));
    }
    let session_started_at_ms = unix_time_ms();
    let executable_sha256 = current_executable_sha256().map_err(CaptureError::Provenance)?;
    let (host_identity_sha256, host_preflight) = if config.host_guard.enabled {
        if let Some(parent) = config
            .output
            .raw_path
            .parent()
            .filter(|parent| !parent.as_os_str().is_empty())
        {
            tokio::fs::create_dir_all(parent).await?;
        }
        (
            Some(host_identity_sha256().map_err(CaptureError::Provenance)?),
            Some(check_host_health(
                &config.host_guard,
                &config.output.raw_path,
            )?),
        )
    } else {
        (None, None)
    };
    let connection_attempt_interval =
        Duration::from_millis(config.runtime.connection_attempt_interval_ms);
    let connection_attempt_pacer = match (
        &config.runtime.connection_attempt_pacer_path,
        connection_attempt_interval.is_zero(),
    ) {
        (Some(path), false) => {
            ConnectionAttemptPacer::process_shared(connection_attempt_interval, path)?
        }
        _ => ConnectionAttemptPacer::new(connection_attempt_interval),
    };

    let plans = config.socket_plans()?;
    let expected_connections = plans
        .iter()
        .map(|plan| plan.conn_id.clone())
        .collect::<HashSet<_>>();
    let stream_coverage = CaptureStreamCoverageState::from_plans(&plans)?;
    let raw_writer = JsonlWriter::start(
        "raw",
        config.output.raw_path.clone(),
        config.runtime.writer_channel_capacity,
        config.output.flush_every_records,
        config.output.fsync_every_records,
    )
    .await?;
    let normalized_writer = match &config.output.normalized_path {
        Some(path) => match JsonlWriter::start(
            "normalized",
            path.clone(),
            config.runtime.writer_channel_capacity,
            config.output.flush_every_records,
            config.output.fsync_every_records,
        )
        .await
        {
            Ok(writer) => Some(writer),
            Err(error) => {
                let raw_shutdown = raw_writer.shutdown_with_evidence().await;
                let error = match raw_shutdown {
                    Ok(JsonlWriterShutdown {
                        failure: Some(shutdown_error),
                        ..
                    }) => combine_capture_lifecycle_errors(
                        error,
                        vec![("raw writer shutdown", shutdown_error)],
                    ),
                    Ok(JsonlWriterShutdown { failure: None, .. }) => error,
                    Err(shutdown_error) => combine_capture_lifecycle_errors(
                        error,
                        vec![("raw writer evidence", shutdown_error)],
                    ),
                };
                return Err(error);
            }
        },
        None => None,
    };
    let mut host_guard = config
        .host_guard
        .enabled
        .then(|| start_host_guard(config.host_guard.clone(), config.output.raw_path.clone()));
    let mut host_failures = host_guard.as_mut().map(HostGuardRuntime::take_failures);
    let adapter: Arc<dyn VenueAdapter> = Arc::new(OkxAdapter::new(
        &config.venue.public_ws_url,
        &config.venue.public_ws_url,
    ));
    let mut feed = spawn_supervised_feed(
        Arc::clone(&adapter),
        plans,
        no_bootstrap(),
        config.runtime.feed_channel_capacity,
        connection_attempt_pacer,
        ReconnectPolicy::default(),
    );
    let mut raw_rx = feed.take_raw();
    let mut status_rx = feed.take_status();

    let started = Instant::now();
    let capture_session_id = format!("{:x}-{:x}", reap_feed::unix_time_ns(), std::process::id());
    let mut state = CaptureState::new(
        config.runtime.dedup_capacity_per_stream,
        config.runtime.max_sequence_buffer,
        expected_connections,
        config.expected_book_symbols(),
        stream_coverage,
        capture_session_id,
    );
    let loop_result = run_capture_loop(
        &mut state,
        &adapter,
        &feed,
        &mut raw_rx,
        &mut status_rx,
        &raw_writer,
        normalized_writer.as_ref(),
        &config.runtime,
        run_duration,
        &mut host_failures,
    )
    .await;

    let drain_result = match tokio::time::timeout(FEED_SHUTDOWN_TIMEOUT, async {
        let (_, drain_result) = tokio::join!(
            feed.shutdown(),
            drain_capture_channels(
                &mut state,
                adapter.as_ref(),
                &mut raw_rx,
                &mut status_rx,
                &raw_writer,
                normalized_writer.as_ref(),
            )
        );
        drain_result
    })
    .await
    {
        Ok(result) => result,
        Err(_) => Err(CaptureError::FeedShutdownTimeout {
            timeout_ms: FEED_SHUTDOWN_TIMEOUT.as_millis(),
        }),
    };
    let (raw_shutdown_result, normalized_shutdown_result, host_stats_result) = tokio::join!(
        raw_writer.shutdown_with_evidence(),
        async move {
            match normalized_writer {
                Some(writer) => writer.shutdown_with_evidence().await,
                None => Ok(JsonlWriterShutdown {
                    stats: JsonlWriterStats::default(),
                    failure: None,
                }),
            }
        },
        async move {
            match host_guard {
                Some(host_guard) => {
                    match tokio::time::timeout(HOST_GUARD_SHUTDOWN_TIMEOUT, host_guard.shutdown())
                        .await
                    {
                        Ok(result) => result.map_err(CaptureError::HostGuardJoin),
                        Err(_) => Err(CaptureError::HostGuardShutdownTimeout {
                            timeout_ms: HOST_GUARD_SHUTDOWN_TIMEOUT.as_millis(),
                        }),
                    }
                }
                None => Ok(HostGuardStats::default()),
            }
        }
    );
    let pending_host_failure = host_failures
        .as_mut()
        .and_then(|failures| failures.try_recv().ok());
    host_failures.take();
    let mut lifecycle_failures = Vec::<(&'static str, CaptureError)>::new();
    let mut stop_reason = match loop_result {
        Ok(reason) => reason,
        Err(error) => {
            lifecycle_failures.push(("capture loop", error));
            CaptureStopReason::RuntimeFailure
        }
    };
    if let Err(error) = drain_result {
        lifecycle_failures.push(("feed drain", error));
    }
    let raw_shutdown = match raw_shutdown_result {
        Ok(outcome) => outcome,
        Err(error) => {
            lifecycle_failures.push(("raw writer evidence", error));
            JsonlWriterShutdown {
                stats: JsonlWriterStats::empty(),
                failure: None,
            }
        }
    };
    if let Some(error) = raw_shutdown.failure {
        lifecycle_failures.push(("raw writer", error));
    }
    let normalized_shutdown = match normalized_shutdown_result {
        Ok(outcome) => outcome,
        Err(error) => {
            lifecycle_failures.push(("normalized writer evidence", error));
            JsonlWriterShutdown {
                stats: JsonlWriterStats::empty(),
                failure: None,
            }
        }
    };
    if let Some(error) = normalized_shutdown.failure {
        lifecycle_failures.push(("normalized writer", error));
    }
    let host_stats = match host_stats_result {
        Ok(stats) => stats,
        Err(error) => {
            lifecycle_failures.push(("host guard shutdown", error));
            HostGuardStats::default()
        }
    };
    if let Some(error) = pending_host_failure {
        lifecycle_failures.push(("host guard pending failure", error.into()));
    }
    let failure = (!lifecycle_failures.is_empty()).then(|| {
        stop_reason = CaptureStopReason::RuntimeFailure;
        combine_capture_failures(lifecycle_failures)
    });
    let run_elapsed_ms = elapsed_ms(&started);
    let session_completed_at_ms = unix_time_ms();
    let failure_evidence = failure.as_ref().map(capture_failure_evidence);
    let report = state.report(
        CaptureRunTiming {
            stop_reason,
            elapsed_ms: run_elapsed_ms,
            completed_at_ms: session_completed_at_ms,
        },
        &config,
        CaptureRunProvenance {
            config_fingerprint,
            config_source,
            executable_sha256,
            host_identity_sha256,
            host_preflight,
            session_started_at_ms,
        },
        CaptureHostEvidence {
            periodic_checks: host_stats.checks,
            last_snapshot: host_stats.last_snapshot,
        },
        CaptureWriterEvidence {
            raw: raw_shutdown.stats,
            normalized: normalized_shutdown.stats,
        },
        failure_evidence,
    );
    match failure {
        Some(source) => Err(CaptureError::ReportedFailure {
            source: Box::new(source),
            report: Box::new(report),
        }),
        None => Ok(report),
    }
}

async fn drain_capture_channels(
    state: &mut CaptureState,
    adapter: &dyn VenueAdapter,
    raw_rx: &mut mpsc::Receiver<RawEnvelope>,
    status_rx: &mut mpsc::Receiver<ConnectionStatus>,
    raw_writer: &JsonlWriter<RawCapture>,
    normalized_writer: Option<&JsonlWriter<NormalizedEvent>>,
) -> Result<(), CaptureError> {
    let mut raw_closed = false;
    let mut status_closed = false;
    while !raw_closed || !status_closed {
        tokio::select! {
            envelope = raw_rx.recv(), if !raw_closed => match envelope {
                Some(envelope) => {
                    let recoveries = state
                        .on_envelope(adapter, envelope, raw_writer, normalized_writer)
                        .await?;
                    state.recovery_requests = state
                        .recovery_requests
                        .saturating_add(recoveries.len() as u64);
                }
                None => raw_closed = true,
            },
            status = status_rx.recv(), if !status_closed => match status {
                Some(status) => state.on_status(status),
                None => status_closed = true,
            },
        }
    }
    Ok(())
}

#[allow(clippy::too_many_arguments)]
async fn run_capture_loop(
    state: &mut CaptureState,
    adapter: &Arc<dyn VenueAdapter>,
    feed: &reap_feed::SupervisedFeed,
    raw_rx: &mut mpsc::Receiver<RawEnvelope>,
    status_rx: &mut mpsc::Receiver<ConnectionStatus>,
    raw_writer: &JsonlWriter<RawCapture>,
    normalized_writer: Option<&JsonlWriter<NormalizedEvent>>,
    runtime: &CaptureRuntimeConfig,
    run_duration: Option<Duration>,
    host_failures: &mut Option<mpsc::Receiver<HostHealthError>>,
) -> Result<CaptureStopReason, CaptureError> {
    let mut health = tokio::time::interval(Duration::from_millis(runtime.health_interval_ms));
    health.set_missed_tick_behavior(tokio::time::MissedTickBehavior::Skip);
    let shutdown = shutdown_signal();
    tokio::pin!(shutdown);
    let duration_elapsed = async move {
        match run_duration {
            Some(duration) => tokio::time::sleep(duration).await,
            None => std::future::pending().await,
        }
    };
    tokio::pin!(duration_elapsed);

    loop {
        tokio::select! {
            signal = &mut shutdown => {
                signal?;
                return Ok(CaptureStopReason::OperatorSignal);
            }
            _ = &mut duration_elapsed => {
                return Ok(CaptureStopReason::DurationElapsed);
            }
            failure = receive_host_failure(host_failures) => {
                let failure = failure.ok_or(CaptureError::HostGuardClosed)?;
                return Err(failure.into());
            }
            status = status_rx.recv() => {
                let status = status.ok_or(CaptureError::FeedClosed)?;
                if status.kind == ConnectionStatusKind::Fatal {
                    return Err(CaptureError::ConnectionPacerRuntime(format!(
                        "{}: {}",
                        status.conn_id, status.reason
                    )));
                }
                state.on_status(status);
            }
            envelope = raw_rx.recv() => {
                let envelope = envelope.ok_or(CaptureError::FeedClosed)?;
                let recoveries = state
                    .on_envelope(adapter.as_ref(), envelope, raw_writer, normalized_writer)
                    .await?;
                for request in recoveries {
                    state.recovery_requests += 1;
                    if feed.request_recovery(&request) == 0 {
                        state.missing_recovery_routes += 1;
                    }
                }
            }
            _ = health.tick() => {
                let now_ms = unix_time_ms();
                for event in state.processor.mark_stale(now_ms, runtime.max_book_age_ms) {
                    state.stale_book_events += 1;
                    if let Some(symbol) = event.symbol {
                        let request = RecoveryRequest {
                            stream: reap_feed::FeedStreamId {
                                venue: event.venue.unwrap_or(Venue::Okx),
                                channel: Channel::Books,
                                symbol,
                            },
                            source_conn_id: None,
                            expected_prev: None,
                            received_prev: 0,
                            received_seq: 0,
                        };
                        state.recovery_requests += 1;
                        if feed.request_recovery(&request) == 0 {
                            state.missing_recovery_routes += 1;
                        }
                    }
                }
            }
        }
    }
}

async fn receive_host_failure(
    failures: &mut Option<mpsc::Receiver<HostHealthError>>,
) -> Option<HostHealthError> {
    match failures {
        Some(failures) => failures.recv().await,
        None => std::future::pending().await,
    }
}

struct CaptureState {
    processor: FeedProcessor,
    capture_session_id: String,
    next_raw_record_seq: u64,
    expected_connections: HashSet<reap_core::ConnId>,
    expected_book_symbols: HashSet<String>,
    stream_coverage: CaptureStreamCoverageState,
    ready_connections: HashSet<reap_core::ConnId>,
    reached_all_connections_ready: bool,
    parse_errors: u64,
    stale_book_events: u64,
    connection_disconnects: u64,
    recovery_requests: u64,
    missing_recovery_routes: u64,
}

#[derive(Default)]
struct CaptureStreamObservation {
    data_sources: BTreeSet<String>,
    data_frames: u64,
    accepted_events: u64,
}

#[derive(Default)]
struct CaptureStreamCoverageState {
    expected_sources: BTreeMap<analysis::StreamKey, BTreeSet<String>>,
    observed: BTreeMap<analysis::StreamKey, CaptureStreamObservation>,
    unclassified_data_frames: u64,
}

impl CaptureStreamCoverageState {
    fn from_plans(plans: &[SocketPlan]) -> Result<Self, CaptureError> {
        Ok(Self {
            expected_sources: analysis::expected_stream_sources(plans)?,
            ..Self::default()
        })
    }

    fn observe_frame(&mut self, capture: &RawCapture) {
        if !analysis::is_data_frame(capture) {
            return;
        }
        let Some(key) = analysis::capture_stream_key(capture) else {
            self.unclassified_data_frames = self.unclassified_data_frames.saturating_add(1);
            return;
        };
        let observation = self.observed.entry(key).or_default();
        observation.data_frames = observation.data_frames.saturating_add(1);
        observation.data_sources.insert(capture.conn_id.0.clone());
    }

    fn observe_accepted(&mut self, key: Option<&analysis::StreamKey>, accepted: u64) {
        if accepted == 0 {
            return;
        }
        let Some(key) = key else {
            return;
        };
        let observation = self.observed.entry(key.clone()).or_default();
        observation.accepted_events = observation.accepted_events.saturating_add(accepted);
    }

    fn complete(&self) -> bool {
        self.unclassified_data_frames == 0
            && self.observed.iter().all(|(key, observation)| {
                observation.data_frames == 0 || self.expected_sources.contains_key(key)
            })
            && self.expected_sources.iter().all(|(key, expected)| {
                self.observed.get(key).is_some_and(|observation| {
                    observation.data_sources == *expected
                        && observation.data_frames > 0
                        && observation.accepted_events > 0
                })
            })
    }
}

struct CaptureRunProvenance {
    config_fingerprint: String,
    config_source: Option<CaptureConfigFileEvidence>,
    executable_sha256: String,
    host_identity_sha256: Option<String>,
    host_preflight: Option<HostHealthSnapshot>,
    session_started_at_ms: u64,
}

struct CaptureHostEvidence {
    periodic_checks: u64,
    last_snapshot: Option<HostHealthSnapshot>,
}

struct CaptureRunTiming {
    stop_reason: CaptureStopReason,
    elapsed_ms: u64,
    completed_at_ms: u64,
}

struct CaptureWriterEvidence {
    raw: JsonlWriterStats,
    normalized: JsonlWriterStats,
}

impl CaptureState {
    fn new(
        dedup_capacity_per_stream: usize,
        max_sequence_buffer: usize,
        expected_connections: HashSet<reap_core::ConnId>,
        expected_book_symbols: HashSet<String>,
        stream_coverage: CaptureStreamCoverageState,
        capture_session_id: String,
    ) -> Self {
        Self {
            processor: FeedProcessor::new(dedup_capacity_per_stream, max_sequence_buffer),
            capture_session_id,
            next_raw_record_seq: 1,
            expected_connections,
            expected_book_symbols,
            stream_coverage,
            ready_connections: HashSet::new(),
            reached_all_connections_ready: false,
            parse_errors: 0,
            stale_book_events: 0,
            connection_disconnects: 0,
            recovery_requests: 0,
            missing_recovery_routes: 0,
        }
    }

    fn on_status(&mut self, status: ConnectionStatus) {
        match status.kind {
            ConnectionStatusKind::Ready | ConnectionStatusKind::Heartbeat => {
                self.ready_connections.insert(status.conn_id);
                self.reached_all_connections_ready |=
                    self.expected_connections.is_subset(&self.ready_connections);
            }
            ConnectionStatusKind::Disconnected | ConnectionStatusKind::Fatal => {
                self.connection_disconnects += 1;
                self.ready_connections.remove(&status.conn_id);
            }
        }
    }

    async fn on_envelope(
        &mut self,
        adapter: &dyn VenueAdapter,
        envelope: RawEnvelope,
        raw_writer: &JsonlWriter<RawCapture>,
        normalized_writer: Option<&JsonlWriter<NormalizedEvent>>,
    ) -> Result<Vec<RecoveryRequest>, CaptureError> {
        let capture_record_seq = self.next_raw_record_seq;
        let next_raw_record_seq = capture_record_seq
            .checked_add(1)
            .ok_or(CaptureError::RawRecordSequenceExhausted)?;
        let capture = raw_capture(&envelope, &self.capture_session_id, capture_record_seq);
        let frame_stream = analysis::capture_stream_key(&capture);
        self.stream_coverage.observe_frame(&capture);
        raw_writer.send(capture).await?;
        self.next_raw_record_seq = next_raw_record_seq;
        let parsed = match adapter.parse(&envelope) {
            Ok(parsed) => parsed,
            Err(error) => {
                self.parse_errors += 1;
                tracing::warn!(?error, conn_id = %envelope.conn_id, "capture payload parse failed");
                return Ok(Vec::new());
            }
        };

        let mut recoveries = Vec::new();
        for parsed in parsed {
            let stream = frame_stream
                .clone()
                .or_else(|| analysis::parsed_stream_key(&parsed));
            let accepted_before = self.processor.stats().accepted;
            let outputs = self.processor.process_from(&envelope.conn_id, parsed);
            let accepted = self
                .processor
                .stats()
                .accepted
                .saturating_sub(accepted_before);
            self.stream_coverage
                .observe_accepted(stream.as_ref(), accepted);
            for output in outputs {
                match output {
                    FeedOutput::Event(event) => {
                        if let Some(writer) = normalized_writer {
                            writer.send(event).await?;
                        }
                    }
                    FeedOutput::RecoveryRequired(request) => recoveries.push(request),
                    FeedOutput::System(event) => {
                        if event.kind == SystemEventKind::BookRecoveryFailed {
                            tracing::warn!(?event, "capture book recovery failed");
                        }
                        if let Some(writer) = normalized_writer {
                            writer.send(NormalizedEvent::System(event)).await?;
                        }
                    }
                    FeedOutput::Duplicate(_)
                    | FeedOutput::PrivateOrder { .. }
                    | FeedOutput::PrivateFill { .. }
                    | FeedOutput::PrivateAccount { .. } => {}
                }
            }
        }
        Ok(recoveries)
    }

    fn report(
        &self,
        timing: CaptureRunTiming,
        config: &CaptureConfig,
        provenance: CaptureRunProvenance,
        host: CaptureHostEvidence,
        writers: CaptureWriterEvidence,
        failure: Option<CaptureFailureEvidence>,
    ) -> CaptureRunReport {
        let raw = writers.raw;
        let normalized = writers.normalized;
        let processor = self.processor.stats();
        let stream_health = self.processor.stream_health();
        let stream_health = stream_health
            .into_iter()
            .map(|health| (health.stream.symbol.clone(), health))
            .collect::<HashMap<_, _>>();
        let all_books_ready = !self.expected_book_symbols.is_empty()
            && self.expected_book_symbols.iter().all(|symbol| {
                stream_health.get(symbol).is_some_and(|health| {
                    health.sequence_status == SequenceStatus::Ready
                        && health.book_status == BookStatus::Ready
                })
            });
        let mut expected_book_symbols = self
            .expected_book_symbols
            .iter()
            .cloned()
            .collect::<Vec<_>>();
        expected_book_symbols.sort();
        let books = expected_book_symbols
            .into_iter()
            .map(|symbol| match stream_health.get(&symbol) {
                Some(health) => CaptureBookHealth {
                    symbol,
                    sequence_status: format!("{:?}", health.sequence_status).to_lowercase(),
                    book_status: format!("{:?}", health.book_status).to_lowercase(),
                    last_seq_id: health.last_seq_id,
                    buffered_updates: health.buffered_updates,
                    sequence_resets: health.sequence_resets,
                    same_sequence_updates: health.same_sequence_updates,
                    best_bid: health.best_bid,
                    best_ask: health.best_ask,
                },
                None => CaptureBookHealth {
                    symbol,
                    sequence_status: "awaiting_snapshot".to_string(),
                    book_status: "empty".to_string(),
                    last_seq_id: None,
                    buffered_updates: 0,
                    sequence_resets: 0,
                    same_sequence_updates: 0,
                    best_bid: None,
                    best_ask: None,
                },
            })
            .collect::<Vec<_>>();
        let all_connections_ready = self.expected_connections.is_subset(&self.ready_connections);
        let clean_capture = capture_run_is_clean(&CaptureCleanRunInputs {
            duration_elapsed: timing.stop_reason == CaptureStopReason::DurationElapsed,
            failure_free: failure.is_none(),
            reached_all_connections_ready: self.reached_all_connections_ready,
            connections_ready_at_stop: all_connections_ready,
            books_ready: all_books_ready,
            stream_coverage_complete: self.stream_coverage.complete(),
            raw_records_present: raw.records > 0,
            raw_record_sequence_complete: raw.records == self.next_raw_record_seq.saturating_sub(1),
            normalized_records_present_or_disabled: config.output.normalized_path.is_none()
                || normalized.records > 0,
            parse_clean: self.parse_errors == 0,
            no_stale_book_events: self.stale_book_events == 0,
            no_recovery_requests: self.recovery_requests == 0,
            no_missing_recovery_routes: self.missing_recovery_routes == 0,
            no_gaps: processor.gaps == 0,
            no_recovery_failures: processor.recovery_failures == 0,
            session_bounds_valid: provenance.session_started_at_ms > 0
                && provenance.session_started_at_ms <= timing.completed_at_ms,
            executable_sha256_valid: is_lower_sha256(&provenance.executable_sha256),
            host_evidence_healthy: capture_host_evidence_is_healthy(
                config,
                &provenance,
                &host,
                timing.completed_at_ms,
            ),
        });

        CaptureRunReport {
            format_version: CAPTURE_RUN_REPORT_FORMAT_VERSION,
            reap_version: env!("CARGO_PKG_VERSION").to_string(),
            java_reference_revision: PINNED_JAVA_REVISION.to_string(),
            executable_sha256: provenance.executable_sha256,
            host_identity_sha256: provenance.host_identity_sha256,
            host_preflight: provenance.host_preflight,
            host_periodic_checks: host.periodic_checks,
            host_last_snapshot: host.last_snapshot,
            session_started_at_ms: provenance.session_started_at_ms,
            session_completed_at_ms: timing.completed_at_ms,
            capture_session_id: self.capture_session_id.clone(),
            config_fingerprint: provenance.config_fingerprint,
            config_source: provenance.config_source,
            stop_reason: timing.stop_reason,
            elapsed_ms: timing.elapsed_ms,
            raw_path: config.output.raw_path.clone(),
            normalized_path: config.output.normalized_path.clone(),
            raw_records: raw.records,
            normalized_records: normalized.records,
            raw_bytes: raw.bytes,
            normalized_bytes: normalized.bytes,
            raw_sha256: raw.sha256,
            normalized_sha256: config
                .output
                .normalized_path
                .as_ref()
                .map(|_| normalized.sha256),
            max_raw_queue_depth: raw.max_queue_depth,
            max_normalized_queue_depth: normalized.max_queue_depth,
            parsed_events: processor.parsed,
            accepted_events: processor.accepted,
            duplicates: processor.duplicates,
            gaps: processor.gaps,
            recoveries: processor.recoveries,
            recovery_failures: processor.recovery_failures,
            sequence_resets: processor.sequence_resets,
            same_sequence_updates: processor.same_sequence_updates,
            recovery_requests: self.recovery_requests,
            missing_recovery_routes: self.missing_recovery_routes,
            parse_errors: self.parse_errors,
            stale_book_events: self.stale_book_events,
            connection_disconnects: self.connection_disconnects,
            expected_connections: self.expected_connections.len(),
            ready_connections_at_stop: self.ready_connections.len(),
            reached_all_connections_ready: self.reached_all_connections_ready,
            books,
            failure,
            clean_capture,
        }
    }
}

fn capture_host_evidence_is_healthy(
    config: &CaptureConfig,
    provenance: &CaptureRunProvenance,
    host: &CaptureHostEvidence,
    session_completed_at_ms: u64,
) -> bool {
    if !config.host_guard.enabled {
        return provenance.host_identity_sha256.is_none()
            && provenance.host_preflight.is_none()
            && host.periodic_checks == 0
            && host.last_snapshot.is_none();
    }

    let Some(identity) = provenance.host_identity_sha256.as_deref() else {
        return false;
    };
    let Some(preflight) = provenance.host_preflight.as_ref() else {
        return false;
    };
    if !is_lower_sha256(identity)
        || preflight.checked_at_ms < provenance.session_started_at_ms
        || preflight.checked_at_ms > session_completed_at_ms
        || !preflight.is_healthy_evidence(&config.host_guard)
    {
        return false;
    }
    match (host.periodic_checks, host.last_snapshot.as_ref()) {
        (0, None) => true,
        (0, Some(_)) | (1.., None) => false,
        (_, Some(last)) => {
            last.checked_at_ms >= preflight.checked_at_ms
                && last.checked_at_ms <= session_completed_at_ms
                && last.is_healthy_evidence(&config.host_guard)
        }
    }
}

fn is_lower_sha256(value: &str) -> bool {
    value.len() == 64
        && value
            .bytes()
            .all(|byte| byte.is_ascii_digit() || (b'a'..=b'f').contains(&byte))
}

fn raw_capture(
    envelope: &RawEnvelope,
    capture_session_id: &str,
    capture_record_seq: u64,
) -> RawCapture {
    RawCapture {
        capture_session_id: Some(capture_session_id.to_string()),
        capture_record_seq: Some(capture_record_seq),
        venue: envelope.venue,
        conn_id: envelope.conn_id.clone(),
        channel: envelope.channel.clone(),
        symbol: envelope.symbol.clone(),
        recv_ts_ns: envelope.recv_ts_ns,
        raw_hash: Some(envelope.raw_hash),
        payload: serde_json::from_str(&envelope.payload)
            .unwrap_or_else(|_| Value::String(envelope.payload.clone())),
    }
}

fn capture_failure_evidence(error: &CaptureError) -> CaptureFailureEvidence {
    CaptureFailureEvidence {
        code: error.stable_code().to_string(),
        message: truncate_utf8(&error.to_string(), MAX_CAPTURE_FAILURE_MESSAGE_BYTES),
    }
}

fn unix_time_ms() -> u64 {
    reap_feed::unix_time_ns() / 1_000_000
}

fn elapsed_ms(started: &Instant) -> u64 {
    started.elapsed().as_millis().min(u64::MAX as u128) as u64
}

#[cfg(unix)]
async fn shutdown_signal() -> Result<(), std::io::Error> {
    let mut terminate = tokio::signal::unix::signal(tokio::signal::unix::SignalKind::terminate())?;
    tokio::select! {
        result = tokio::signal::ctrl_c() => result,
        _ = terminate.recv() => Ok(()),
    }
}

#[cfg(not(unix))]
async fn shutdown_signal() -> Result<(), std::io::Error> {
    tokio::signal::ctrl_c().await
}

#[cfg(test)]
mod tests {
    use reap_core::{Channel, ConnId, Venue};

    use crate::configuration::{
        CaptureOutputConfig, CapturePriority, CaptureSubscriptionConfig, CaptureVenueConfig,
    };
    use crate::hashing::sha256_hex;

    use super::*;

    #[test]
    fn guarded_capture_evidence_is_bound_to_session_time_and_thresholds() {
        let mut config =
            CaptureConfig::from_toml(include_str!("../../../examples/capture-okx-public.toml"))
                .unwrap();
        config.host_guard.min_disk_available_bytes = 10;
        config.host_guard.min_memory_available_bytes = 20;
        let provenance = CaptureRunProvenance {
            config_fingerprint: "f".repeat(64),
            config_source: None,
            executable_sha256: "e".repeat(64),
            host_identity_sha256: Some("9".repeat(64)),
            host_preflight: Some(HostHealthSnapshot {
                checked_at_ms: 10,
                disk_available_bytes: 10,
                memory_available_bytes: 20,
                clock_synchronized: true,
            }),
            session_started_at_ms: 9,
        };
        let healthy = CaptureHostEvidence {
            periodic_checks: 1,
            last_snapshot: Some(HostHealthSnapshot {
                checked_at_ms: 11,
                disk_available_bytes: 10,
                memory_available_bytes: 20,
                clock_synchronized: true,
            }),
        };

        assert!(capture_host_evidence_is_healthy(
            &config,
            &provenance,
            &healthy,
            12,
        ));

        let late = CaptureHostEvidence {
            periodic_checks: 1,
            last_snapshot: Some(HostHealthSnapshot {
                checked_at_ms: 13,
                disk_available_bytes: 10,
                memory_available_bytes: 20,
                clock_synchronized: true,
            }),
        };
        assert!(!capture_host_evidence_is_healthy(
            &config,
            &provenance,
            &late,
            12,
        ));
    }

    #[test]
    fn clean_report_requires_every_configured_book_snapshot() {
        let config = CaptureConfig {
            venue: CaptureVenueConfig::default(),
            runtime: CaptureRuntimeConfig::default(),
            output: CaptureOutputConfig::default(),
            host_guard: HostGuardConfig::default(),
            subscriptions: vec![
                CaptureSubscriptionConfig {
                    channel: "books".to_string(),
                    symbol: "BTC-USDT".to_string(),
                    connections: 1,
                    priority: CapturePriority::Critical,
                },
                CaptureSubscriptionConfig {
                    channel: "books".to_string(),
                    symbol: "ETH-USDT".to_string(),
                    connections: 1,
                    priority: CapturePriority::Critical,
                },
            ],
        };
        let mut state = CaptureState::new(
            16,
            16,
            HashSet::new(),
            config.expected_book_symbols(),
            CaptureStreamCoverageState::from_plans(&config.socket_plans().unwrap()).unwrap(),
            "test-session".to_string(),
        );
        state.reached_all_connections_ready = true;
        let first_line = include_str!("../../../fixtures/raw/okx/depth-gap.jsonl")
            .lines()
            .next()
            .unwrap();
        let capture: RawCapture = serde_json::from_str(first_line).unwrap();
        let adapter = OkxAdapter::default();
        let envelope = capture.into_envelope().unwrap();
        for parsed in adapter.parse(&envelope).unwrap() {
            let _ = state.processor.process_from(&envelope.conn_id, parsed);
        }

        let report = state.report(
            CaptureRunTiming {
                stop_reason: CaptureStopReason::DurationElapsed,
                elapsed_ms: 1,
                completed_at_ms: 2,
            },
            &config,
            CaptureRunProvenance {
                config_fingerprint: config.fingerprint().unwrap(),
                config_source: None,
                executable_sha256: "0".repeat(64),
                host_identity_sha256: None,
                host_preflight: None,
                session_started_at_ms: 1,
            },
            CaptureHostEvidence {
                periodic_checks: 0,
                last_snapshot: None,
            },
            CaptureWriterEvidence {
                raw: JsonlWriterStats {
                    records: 1,
                    bytes: 1,
                    max_queue_depth: 1,
                    sha256: "0".repeat(64),
                },
                normalized: JsonlWriterStats::default(),
            },
            None,
        );

        assert!(!report.clean_capture);
        assert_eq!(report.books.len(), 2);
        assert_eq!(report.books[1].symbol, "ETH-USDT");
        assert_eq!(report.books[1].sequence_status, "awaiting_snapshot");
    }

    #[test]
    fn stream_coverage_requires_exact_planned_sources_and_an_accepted_event() {
        let config = CaptureConfig {
            venue: CaptureVenueConfig::default(),
            runtime: CaptureRuntimeConfig::default(),
            output: CaptureOutputConfig::default(),
            host_guard: HostGuardConfig::default(),
            subscriptions: vec![
                CaptureSubscriptionConfig {
                    channel: "books".to_string(),
                    symbol: "BTC-USDT".to_string(),
                    connections: 2,
                    priority: CapturePriority::Critical,
                },
                CaptureSubscriptionConfig {
                    channel: "trades".to_string(),
                    symbol: "BTC-USDT".to_string(),
                    connections: 2,
                    priority: CapturePriority::High,
                },
            ],
        };
        let book = analysis::StreamKey {
            channel: "books".to_string(),
            symbol: "BTC-USDT".to_string(),
        };
        let trade = analysis::StreamKey {
            channel: "trades".to_string(),
            symbol: "BTC-USDT".to_string(),
        };
        let mut coverage =
            CaptureStreamCoverageState::from_plans(&config.socket_plans().unwrap()).unwrap();

        coverage.observe_frame(&stream_capture(
            "books",
            "BTC-USDT",
            "okx-books-critical-r0-0",
        ));
        coverage.observe_frame(&stream_capture(
            "books",
            "BTC-USDT",
            "okx-books-critical-r1-0",
        ));
        coverage.observe_accepted(Some(&book), 1);
        assert!(!coverage.complete());

        coverage.observe_frame(&stream_capture(
            "trades",
            "BTC-USDT",
            "okx-trades-high-r0-0",
        ));
        coverage.observe_accepted(Some(&trade), 1);
        assert!(!coverage.complete());

        coverage.observe_frame(&stream_capture(
            "trades",
            "BTC-USDT",
            "okx-trades-high-r1-0",
        ));
        assert!(coverage.complete());

        coverage.observe_frame(&stream_capture(
            "books",
            "BTC-USDT",
            "okx-books-critical-r9-0",
        ));
        assert!(!coverage.complete());
    }

    #[test]
    fn expected_stream_sources_preserve_socket_partition_chunks() {
        let mut config = CaptureConfig {
            venue: CaptureVenueConfig::default(),
            runtime: CaptureRuntimeConfig::default(),
            output: CaptureOutputConfig::default(),
            host_guard: HostGuardConfig::default(),
            subscriptions: vec![
                CaptureSubscriptionConfig {
                    channel: "books".to_string(),
                    symbol: "BTC-USDT".to_string(),
                    connections: 2,
                    priority: CapturePriority::Critical,
                },
                CaptureSubscriptionConfig {
                    channel: "books".to_string(),
                    symbol: "ETH-USDT".to_string(),
                    connections: 2,
                    priority: CapturePriority::Critical,
                },
            ],
        };
        config.runtime.max_subscriptions_per_socket = 1;

        let sources = analysis::expected_stream_sources(&config.socket_plans().unwrap()).unwrap();

        assert_eq!(
            sources[&analysis::StreamKey {
                channel: "books".to_string(),
                symbol: "BTC-USDT".to_string(),
            }],
            BTreeSet::from([
                "okx-books-critical-r0-0".to_string(),
                "okx-books-critical-r1-0".to_string(),
            ])
        );
        assert_eq!(
            sources[&analysis::StreamKey {
                channel: "books".to_string(),
                symbol: "ETH-USDT".to_string(),
            }],
            BTreeSet::from([
                "okx-books-critical-r0-1".to_string(),
                "okx-books-critical-r1-1".to_string(),
            ])
        );
    }

    #[cfg(target_os = "linux")]
    #[tokio::test]
    async fn host_preflight_fails_before_capture_output_or_network_startup() {
        let directory = tempfile::tempdir().unwrap();
        let raw_path = directory.path().join("new").join("raw.jsonl");
        let mut config =
            CaptureConfig::from_toml(include_str!("../../../examples/capture-okx-public.toml"))
                .unwrap();
        config.output.raw_path = raw_path.clone();
        config.host_guard.min_disk_available_bytes = u64::MAX;
        config.host_guard.min_memory_available_bytes = u64::MAX;
        config.host_guard.require_clock_synchronized = false;

        let error = run_capture(
            config,
            CaptureRunOptions {
                run_duration: Some(Duration::from_millis(1)),
                config_source: None,
            },
        )
        .await
        .unwrap_err();

        assert!(matches!(
            error,
            CaptureError::Host(HostHealthError::Unhealthy { code, .. })
                if code.contains("disk_low") && code.contains("memory_low")
        ));
        assert!(!raw_path.exists());
    }

    #[tokio::test]
    async fn capture_refuses_to_append_a_second_process_session() {
        let directory = tempfile::tempdir().unwrap();
        let path = directory.path().join("existing.jsonl");
        std::fs::write(&path, "existing-session\n").unwrap();
        let config = CaptureConfig {
            venue: CaptureVenueConfig::default(),
            runtime: CaptureRuntimeConfig {
                connection_attempt_pacer_path: Some(directory.path().join("connect.pacer")),
                ..CaptureRuntimeConfig::default()
            },
            output: CaptureOutputConfig {
                raw_path: path.clone(),
                ..CaptureOutputConfig::default()
            },
            host_guard: HostGuardConfig::default(),
            subscriptions: vec![CaptureSubscriptionConfig {
                channel: "books".to_string(),
                symbol: "BTC-USDT".to_string(),
                connections: 1,
                priority: CapturePriority::Critical,
            }],
        };

        let error = run_capture(
            config,
            CaptureRunOptions {
                run_duration: Some(Duration::from_millis(1)),
                config_source: None,
            },
        )
        .await
        .unwrap_err();

        assert!(matches!(
            error,
            CaptureError::OpenOutput {
                name: "raw",
                source,
                ..
            } if source.kind() == std::io::ErrorKind::AlreadyExists
        ));
        assert_eq!(std::fs::read_to_string(path).unwrap(), "existing-session\n");
    }

    #[tokio::test]
    async fn normalized_output_failure_shuts_down_the_initialized_raw_writer() {
        let directory = tempfile::tempdir().unwrap();
        let raw_path = directory.path().join("raw.jsonl");
        let normalized_path = directory.path().join("existing-normalized.jsonl");
        std::fs::write(&normalized_path, "existing-session\n").unwrap();
        let config = CaptureConfig {
            venue: CaptureVenueConfig::default(),
            runtime: CaptureRuntimeConfig {
                connection_attempt_pacer_path: Some(directory.path().join("connect.pacer")),
                ..CaptureRuntimeConfig::default()
            },
            output: CaptureOutputConfig {
                raw_path: raw_path.clone(),
                normalized_path: Some(normalized_path.clone()),
                ..CaptureOutputConfig::default()
            },
            host_guard: HostGuardConfig::default(),
            subscriptions: vec![CaptureSubscriptionConfig {
                channel: "books".to_string(),
                symbol: "BTC-USDT".to_string(),
                connections: 1,
                priority: CapturePriority::Critical,
            }],
        };

        let error = run_capture(
            config,
            CaptureRunOptions {
                run_duration: Some(Duration::from_millis(1)),
                config_source: None,
            },
        )
        .await
        .unwrap_err();

        assert!(matches!(
            error,
            CaptureError::OpenOutput {
                name: "normalized",
                source,
                ..
            } if source.kind() == std::io::ErrorKind::AlreadyExists
        ));
        assert_eq!(std::fs::read_to_string(raw_path).unwrap(), "");
        assert_eq!(
            std::fs::read_to_string(normalized_path).unwrap(),
            "existing-session\n"
        );
    }

    #[tokio::test]
    async fn connection_pacer_preflight_fails_before_capture_output_or_network_startup() {
        let directory = tempfile::tempdir().unwrap();
        let raw_path = directory.path().join("raw.jsonl");
        let mut config =
            CaptureConfig::from_toml(include_str!("../../../examples/capture-okx-public.toml"))
                .unwrap();
        config.venue.public_ws_url = "ws://127.0.0.1:18081/ws/v5/public".to_string();
        config.output.raw_path = raw_path.clone();
        config.host_guard.enabled = false;
        config.runtime.connection_attempt_pacer_path =
            Some(directory.path().join("missing").join("connect.pacer"));

        let error = run_capture(
            config,
            CaptureRunOptions {
                run_duration: Some(Duration::from_millis(1)),
                config_source: None,
            },
        )
        .await
        .unwrap_err();

        assert!(matches!(error, CaptureError::ConnectionPacer(_)));
        assert!(!raw_path.exists());
    }

    #[test]
    fn startup_failure_report_does_not_adopt_preexisting_capture_outputs() {
        let directory = tempfile::tempdir().unwrap();
        let raw_path = directory.path().join("raw.jsonl");
        let normalized_path = directory.path().join("normalized.jsonl");
        std::fs::write(&raw_path, b"prior raw session\n").unwrap();
        std::fs::write(&normalized_path, b"prior normalized session\n").unwrap();
        let mut config =
            CaptureConfig::from_toml(include_str!("../../../examples/capture-okx-public.toml"))
                .unwrap();
        config.output.raw_path = raw_path.clone();
        config.output.normalized_path = Some(normalized_path.clone());
        let error = CaptureError::Host(HostHealthError::Probe("probe unavailable".to_string()));

        let report = capture_startup_failure_report(&config, None, &error);

        assert_eq!(report.stop_reason, CaptureStopReason::RuntimeFailure);
        assert!(!report.clean_capture);
        assert_eq!(report.raw_records, 0);
        assert_eq!(report.raw_bytes, 0);
        assert_eq!(report.raw_sha256, sha256_hex(&[]));
        assert_eq!(report.normalized_records, 0);
        assert_eq!(report.normalized_bytes, 0);
        assert_eq!(report.normalized_sha256, Some(sha256_hex(&[])));
        assert_eq!(
            report.expected_connections,
            config.socket_plans().unwrap().len()
        );
        assert_eq!(
            report.failure,
            Some(CaptureFailureEvidence {
                code: "host_guard".to_string(),
                message: error.to_string(),
            })
        );
        assert_eq!(std::fs::read(&raw_path).unwrap(), b"prior raw session\n");
        assert_eq!(
            std::fs::read(&normalized_path).unwrap(),
            b"prior normalized session\n"
        );
    }

    #[test]
    fn failure_evidence_has_a_stable_code_and_utf8_byte_bound() {
        let error = CaptureError::InvalidConfig("e\u{301}".repeat(4_096));

        let evidence = capture_failure_evidence(&error);

        assert_eq!(evidence.code, "config");
        assert!(!evidence.message.is_empty());
        assert!(evidence.message.len() <= MAX_CAPTURE_FAILURE_MESSAGE_BYTES);
        assert!(evidence.message.is_char_boundary(evidence.message.len()));
    }

    #[tokio::test]
    async fn shutdown_drain_persists_queued_feed_frames() {
        let directory = tempfile::tempdir().unwrap();
        let path = directory.path().join("raw.jsonl");
        let writer = JsonlWriter::start("raw", path.clone(), 4, 1_000, 1_000)
            .await
            .unwrap();
        let first_line = include_str!("../../../fixtures/raw/okx/depth-gap.jsonl")
            .lines()
            .next()
            .unwrap();
        let capture: RawCapture = serde_json::from_str(first_line).unwrap();
        let (raw_tx, mut raw_rx) = mpsc::channel(1);
        raw_tx.send(capture.into_envelope().unwrap()).await.unwrap();
        drop(raw_tx);
        let (status_tx, mut status_rx) = mpsc::channel(1);
        drop(status_tx);
        let mut state = CaptureState::new(
            16,
            16,
            HashSet::new(),
            HashSet::new(),
            CaptureStreamCoverageState::default(),
            "test-session".to_string(),
        );

        drain_capture_channels(
            &mut state,
            &OkxAdapter::default(),
            &mut raw_rx,
            &mut status_rx,
            &writer,
            None,
        )
        .await
        .unwrap();
        let outcome = writer.shutdown_with_evidence().await.unwrap();
        assert!(outcome.failure.is_none());
        let stats = outcome.stats;

        assert_eq!(stats.records, 1);
        assert_eq!(state.processor.stats().accepted, 1);
        let persisted: RawCapture =
            serde_json::from_str(std::fs::read_to_string(path).unwrap().trim()).unwrap();
        assert_eq!(
            persisted.capture_session_id.as_deref(),
            Some("test-session")
        );
        assert_eq!(persisted.capture_record_seq, Some(1));
    }

    fn stream_capture(channel: &str, symbol: &str, conn_id: &str) -> RawCapture {
        RawCapture {
            capture_session_id: Some("test-session".to_string()),
            capture_record_seq: Some(1),
            venue: Venue::Okx,
            conn_id: ConnId::new(conn_id),
            channel: match channel {
                "books" => Channel::Books,
                "trades" => Channel::Trades,
                channel => Channel::Custom(channel.to_string()),
            },
            symbol: Some(symbol.to_string()),
            recv_ts_ns: 1,
            raw_hash: None,
            payload: serde_json::json!({
                "arg": {"channel": channel, "instId": symbol},
                "data": [{"instId": symbol}],
            }),
        }
    }
}
