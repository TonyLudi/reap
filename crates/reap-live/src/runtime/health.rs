use std::sync::atomic::{AtomicBool, AtomicU8, AtomicU64, Ordering};
use std::time::Instant;

use serde::Serialize;

use crate::LivePhase;

const RUNTIME_HEALTH_SCHEMA_VERSION: u32 = 1;
const NS_PER_MS: u64 = 1_000_000;
pub(super) const HEALTH_EMISSION_INTERVAL_NS: u64 = 5_000_000_000;

const CONNECTIVITY_COMPONENT_COUNT: usize = 3;
const HEARTBEAT_COMPONENT_COUNT: usize = 2;
const QUEUE_COMPONENT_COUNT: usize = 3;
const HEALTH_COUNTER_COUNT: usize = 9;

/// Fixed health identifiers prevent account IDs, symbols, or arbitrary
/// component strings from entering the progress path.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
#[repr(usize)]
pub(super) enum ConnectivityId {
    Feed = 0,
    Private = 1,
    OrderCommand = 2,
}

impl ConnectivityId {
    const ALL: [Self; CONNECTIVITY_COMPONENT_COUNT] =
        [Self::Feed, Self::Private, Self::OrderCommand];

    const fn index(self) -> usize {
        self as usize
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
#[repr(u8)]
pub(super) enum ConnectivityHealthState {
    Unknown = 0,
    Connecting = 1,
    Ready = 2,
    Disconnected = 3,
    Failed = 4,
}

impl ConnectivityHealthState {
    fn from_u8(value: u8) -> Self {
        match value {
            1 => Self::Connecting,
            2 => Self::Ready,
            3 => Self::Disconnected,
            4 => Self::Failed,
            _ => Self::Unknown,
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
#[repr(usize)]
pub(super) enum HeartbeatId {
    RuntimeEventLoop = 0,
    OrderTask = 1,
}

impl HeartbeatId {
    const ALL: [Self; HEARTBEAT_COMPONENT_COUNT] = [Self::RuntimeEventLoop, Self::OrderTask];

    const fn index(self) -> usize {
        self as usize
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
#[repr(usize)]
pub(super) enum QueueId {
    FeedIngress = 0,
    ControlIngress = 1,
    OrderCommand = 2,
}

impl QueueId {
    const ALL: [Self; QUEUE_COMPONENT_COUNT] =
        [Self::FeedIngress, Self::ControlIngress, Self::OrderCommand];

    const fn index(self) -> usize {
        self as usize
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[repr(usize)]
pub(super) enum HealthCounterId {
    SubmitRequests = 0,
    SubmitAcceptedAcks = 1,
    CancelRequests = 2,
    CancelAcceptedAcks = 3,
    PolicyRiskRejected = 4,
    ExchangeRejected = 5,
    Ambiguous = 6,
    ReconciliationTotal = 7,
    ReconciliationClean = 8,
}

impl HealthCounterId {
    #[cfg(test)]
    const ALL: [Self; HEALTH_COUNTER_COUNT] = [
        Self::SubmitRequests,
        Self::SubmitAcceptedAcks,
        Self::CancelRequests,
        Self::CancelAcceptedAcks,
        Self::PolicyRiskRejected,
        Self::ExchangeRejected,
        Self::Ambiguous,
        Self::ReconciliationTotal,
        Self::ReconciliationClean,
    ];

    const fn index(self) -> usize {
        self as usize
    }
}

struct ProgressClock {
    observed: AtomicBool,
    last_progress_ns: AtomicU64,
}

impl ProgressClock {
    fn new() -> Self {
        Self {
            observed: AtomicBool::new(false),
            last_progress_ns: AtomicU64::new(0),
        }
    }

    fn observe_at(&self, now_ns: u64) {
        self.last_progress_ns.fetch_max(now_ns, Ordering::Relaxed);
        self.observed.store(true, Ordering::Release);
    }

    fn age_ms_at(&self, now_ns: u64) -> Option<u64> {
        self.observed.load(Ordering::Acquire).then(|| {
            now_ns.saturating_sub(self.last_progress_ns.load(Ordering::Relaxed)) / NS_PER_MS
        })
    }
}

struct ConnectivitySlot {
    state: AtomicU8,
    progress: ProgressClock,
}

impl ConnectivitySlot {
    fn new() -> Self {
        Self {
            state: AtomicU8::new(ConnectivityHealthState::Unknown as u8),
            progress: ProgressClock::new(),
        }
    }
}

struct HeartbeatSlot {
    progress_count: AtomicU64,
    progress: ProgressClock,
}

impl HeartbeatSlot {
    fn new() -> Self {
        Self {
            progress_count: AtomicU64::new(0),
            progress: ProgressClock::new(),
        }
    }
}

struct QueueSlot {
    capacity: AtomicU64,
    depth: AtomicU64,
    high_water_mark: AtomicU64,
    residence_observed: AtomicBool,
    last_residence_age_ns: AtomicU64,
    max_residence_age_ns: AtomicU64,
    saturated: AtomicBool,
    saturation_events: AtomicU64,
}

impl QueueSlot {
    fn new() -> Self {
        Self {
            capacity: AtomicU64::new(0),
            depth: AtomicU64::new(0),
            high_water_mark: AtomicU64::new(0),
            residence_observed: AtomicBool::new(false),
            last_residence_age_ns: AtomicU64::new(0),
            max_residence_age_ns: AtomicU64::new(0),
            saturated: AtomicBool::new(false),
            saturation_events: AtomicU64::new(0),
        }
    }
}

/// Numeric storage progress sampled from the storage-owned atomics only when a
/// periodic or final health snapshot is assembled.
///
/// `last_writer_progress_age_ns` is already an age in the storage clock
/// domain. Raw monotonic instants from independent origins are intentionally
/// not compared here.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub(super) struct StorageHealthInput {
    pub(super) queue_capacity: u64,
    pub(super) queue_depth: u64,
    pub(super) queue_high_water: u64,
    pub(super) records_enqueued: u64,
    pub(super) records_written: u64,
    pub(super) records_outstanding: u64,
    pub(super) durable_sync_completions: u64,
    pub(super) write_failures: u64,
    pub(super) sync_failures: u64,
    pub(super) dropped_records: u64,
    pub(super) last_writer_progress_age_ns: Option<u64>,
}

/// Fixed-cardinality progress state shared with the event loop and order task.
///
/// Every progress method performs only timer reads and bounded atomic
/// operations. It takes no lock, allocates no memory, and accepts no string,
/// symbol, account, or dynamically registered component identifier.
pub(super) struct RuntimeHealthState {
    origin: Instant,
    connectivity: [ConnectivitySlot; CONNECTIVITY_COMPONENT_COUNT],
    heartbeats: [HeartbeatSlot; HEARTBEAT_COMPONENT_COUNT],
    queues: [QueueSlot; QUEUE_COMPONENT_COUNT],
    readiness: AtomicU8,
    active_durable_safety_latches: AtomicU64,
    counters: [AtomicU64; HEALTH_COUNTER_COUNT],
}

impl RuntimeHealthState {
    pub(super) fn new() -> Self {
        Self {
            origin: Instant::now(),
            connectivity: std::array::from_fn(|_| ConnectivitySlot::new()),
            heartbeats: std::array::from_fn(|_| HeartbeatSlot::new()),
            queues: std::array::from_fn(|_| QueueSlot::new()),
            readiness: AtomicU8::new(live_phase_to_u8(LivePhase::Configured)),
            active_durable_safety_latches: AtomicU64::new(0),
            counters: std::array::from_fn(|_| AtomicU64::new(0)),
        }
    }

    pub(super) fn monotonic_now_ns(&self) -> u64 {
        u64::try_from(self.origin.elapsed().as_nanos()).unwrap_or(u64::MAX)
    }

    pub(super) fn set_connectivity(&self, id: ConnectivityId, state: ConnectivityHealthState) {
        self.set_connectivity_at(id, state, self.monotonic_now_ns());
    }

    fn set_connectivity_at(&self, id: ConnectivityId, state: ConnectivityHealthState, now_ns: u64) {
        let slot = &self.connectivity[id.index()];
        slot.state.store(state as u8, Ordering::Relaxed);
        slot.progress.observe_at(now_ns);
    }

    pub(super) fn mark_connectivity_progress(&self, id: ConnectivityId) {
        self.mark_connectivity_progress_at(id, self.monotonic_now_ns());
    }

    fn mark_connectivity_progress_at(&self, id: ConnectivityId, now_ns: u64) {
        self.connectivity[id.index()].progress.observe_at(now_ns);
    }

    pub(super) fn mark_heartbeat(&self, id: HeartbeatId) {
        self.mark_heartbeat_at(id, self.monotonic_now_ns());
    }

    fn mark_heartbeat_at(&self, id: HeartbeatId, now_ns: u64) {
        let slot = &self.heartbeats[id.index()];
        atomic_saturating_add(&slot.progress_count, 1);
        slot.progress.observe_at(now_ns);
    }

    pub(super) fn configure_queue(&self, id: QueueId, capacity: u64) {
        let slot = &self.queues[id.index()];
        slot.capacity.store(capacity, Ordering::Relaxed);
        let depth = slot.depth.load(Ordering::Relaxed);
        slot.saturated
            .store(capacity > 0 && depth >= capacity, Ordering::Relaxed);
    }

    pub(super) fn observe_queue_depth(&self, id: QueueId, depth: u64) {
        let slot = &self.queues[id.index()];
        slot.depth.store(depth, Ordering::Relaxed);
        slot.high_water_mark.fetch_max(depth, Ordering::Relaxed);
        let capacity = slot.capacity.load(Ordering::Relaxed);
        let saturated = capacity > 0 && depth >= capacity;
        let was_saturated = slot.saturated.swap(saturated, Ordering::Relaxed);
        if saturated && !was_saturated {
            atomic_saturating_add(&slot.saturation_events, 1);
        }
    }

    pub(super) fn observe_queue_residence(&self, id: QueueId, residence_age_ns: u64) {
        let slot = &self.queues[id.index()];
        slot.last_residence_age_ns
            .store(residence_age_ns, Ordering::Relaxed);
        slot.max_residence_age_ns
            .fetch_max(residence_age_ns, Ordering::Relaxed);
        slot.residence_observed.store(true, Ordering::Release);
    }

    pub(super) fn observe_queue_saturation(&self, id: QueueId) {
        let slot = &self.queues[id.index()];
        slot.saturated.store(true, Ordering::Relaxed);
        atomic_saturating_add(&slot.saturation_events, 1);
    }

    pub(super) fn set_readiness(&self, state: LivePhase, active_durable_safety_latches: u64) {
        self.readiness
            .store(live_phase_to_u8(state), Ordering::Relaxed);
        self.active_durable_safety_latches
            .store(active_durable_safety_latches, Ordering::Relaxed);
    }

    pub(super) fn increment_counter(&self, id: HealthCounterId, amount: u64) {
        atomic_saturating_add(&self.counters[id.index()], amount);
    }

    pub(super) fn periodic_snapshot(&self, storage: StorageHealthInput) -> RuntimeHealthSnapshot {
        self.snapshot_at(self.monotonic_now_ns(), storage, false)
    }

    pub(super) fn final_snapshot(&self, storage: StorageHealthInput) -> RuntimeHealthSnapshot {
        self.snapshot_at(self.monotonic_now_ns(), storage, true)
    }

    fn snapshot_at(
        &self,
        now_ns: u64,
        storage: StorageHealthInput,
        final_snapshot: bool,
    ) -> RuntimeHealthSnapshot {
        RuntimeHealthSnapshot {
            schema_version: RUNTIME_HEALTH_SCHEMA_VERSION,
            final_snapshot,
            connectivity: ConnectivityId::ALL.map(|id| {
                let slot = &self.connectivity[id.index()];
                let last_progress_age_ms = slot.progress.age_ms_at(now_ns);
                ConnectivityHealthSnapshot {
                    id,
                    state: ConnectivityHealthState::from_u8(slot.state.load(Ordering::Relaxed)),
                    last_progress_age_ms,
                }
            }),
            heartbeats: HeartbeatId::ALL.map(|id| {
                let slot = &self.heartbeats[id.index()];
                let last_progress_age_ms = slot.progress.age_ms_at(now_ns);
                HeartbeatHealthSnapshot {
                    id,
                    progress_count: slot.progress_count.load(Ordering::Relaxed),
                    last_progress_age_ms,
                }
            }),
            queues: QueueId::ALL.map(|id| {
                let slot = &self.queues[id.index()];
                let residence_observed = slot.residence_observed.load(Ordering::Acquire);
                QueueHealthSnapshot {
                    id,
                    capacity: slot.capacity.load(Ordering::Relaxed),
                    depth: slot.depth.load(Ordering::Relaxed),
                    high_water_mark: slot.high_water_mark.load(Ordering::Relaxed),
                    last_residence_age_ms: residence_observed
                        .then(|| slot.last_residence_age_ns.load(Ordering::Relaxed) / NS_PER_MS),
                    max_residence_age_ms: slot.max_residence_age_ns.load(Ordering::Relaxed)
                        / NS_PER_MS,
                    saturated: slot.saturated.load(Ordering::Relaxed),
                    saturation_events: slot.saturation_events.load(Ordering::Relaxed),
                }
            }),
            readiness: ReadinessHealthSnapshot {
                state: live_phase_from_u8(self.readiness.load(Ordering::Relaxed)),
                active_durable_safety_latches: self
                    .active_durable_safety_latches
                    .load(Ordering::Relaxed),
            },
            orders: OrderHealthCountersSnapshot {
                submit_requests: self.counter(HealthCounterId::SubmitRequests),
                submit_accepted_acks: self.counter(HealthCounterId::SubmitAcceptedAcks),
                cancel_requests: self.counter(HealthCounterId::CancelRequests),
                cancel_accepted_acks: self.counter(HealthCounterId::CancelAcceptedAcks),
                policy_risk_rejected: self.counter(HealthCounterId::PolicyRiskRejected),
                exchange_rejected: self.counter(HealthCounterId::ExchangeRejected),
                ambiguous: self.counter(HealthCounterId::Ambiguous),
                reconciliation_total: self.counter(HealthCounterId::ReconciliationTotal),
                reconciliation_clean: self.counter(HealthCounterId::ReconciliationClean),
            },
            storage: StorageHealthSnapshot::from_input(storage),
        }
    }

    fn counter(&self, id: HealthCounterId) -> u64 {
        self.counters[id.index()].load(Ordering::Relaxed)
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
pub(super) struct ConnectivityHealthSnapshot {
    id: ConnectivityId,
    state: ConnectivityHealthState,
    last_progress_age_ms: Option<u64>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
pub(super) struct HeartbeatHealthSnapshot {
    id: HeartbeatId,
    progress_count: u64,
    last_progress_age_ms: Option<u64>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
pub(super) struct QueueHealthSnapshot {
    id: QueueId,
    capacity: u64,
    depth: u64,
    high_water_mark: u64,
    last_residence_age_ms: Option<u64>,
    max_residence_age_ms: u64,
    saturated: bool,
    saturation_events: u64,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
pub(super) struct ReadinessHealthSnapshot {
    state: LivePhase,
    active_durable_safety_latches: u64,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
pub(super) struct OrderHealthCountersSnapshot {
    submit_requests: u64,
    submit_accepted_acks: u64,
    cancel_requests: u64,
    cancel_accepted_acks: u64,
    policy_risk_rejected: u64,
    exchange_rejected: u64,
    ambiguous: u64,
    reconciliation_total: u64,
    reconciliation_clean: u64,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
pub(super) struct StorageHealthSnapshot {
    queue_capacity: u64,
    queue_depth: u64,
    queue_high_water: u64,
    queue_saturated: bool,
    records_enqueued: u64,
    records_written: u64,
    records_outstanding: u64,
    durable_sync_completions: u64,
    write_failures: u64,
    sync_failures: u64,
    dropped_records: u64,
    last_writer_progress_age_ms: Option<u64>,
}

impl StorageHealthSnapshot {
    fn from_input(input: StorageHealthInput) -> Self {
        Self {
            queue_capacity: input.queue_capacity,
            queue_depth: input.queue_depth,
            queue_high_water: input.queue_high_water,
            queue_saturated: input.queue_capacity > 0 && input.queue_depth >= input.queue_capacity,
            records_enqueued: input.records_enqueued,
            records_written: input.records_written,
            records_outstanding: input.records_outstanding,
            durable_sync_completions: input.durable_sync_completions,
            write_failures: input.write_failures,
            sync_failures: input.sync_failures,
            dropped_records: input.dropped_records,
            last_writer_progress_age_ms: input
                .last_writer_progress_age_ns
                .map(|age_ns| age_ns / NS_PER_MS),
        }
    }
}

/// Private schema-1 heartbeat payload. It is Serialize-only and grants no
/// control, configuration, or order authority.
#[derive(Debug, PartialEq, Eq, Serialize)]
pub(super) struct RuntimeHealthSnapshot {
    schema_version: u32,
    final_snapshot: bool,
    connectivity: [ConnectivityHealthSnapshot; CONNECTIVITY_COMPONENT_COUNT],
    heartbeats: [HeartbeatHealthSnapshot; HEARTBEAT_COMPONENT_COUNT],
    queues: [QueueHealthSnapshot; QUEUE_COMPONENT_COUNT],
    readiness: ReadinessHealthSnapshot,
    orders: OrderHealthCountersSnapshot,
    storage: StorageHealthSnapshot,
}

pub(super) struct HealthEmissionCadence {
    next_due_ns: u64,
    armed: bool,
}

impl HealthEmissionCadence {
    pub(super) fn starting_at(start_ns: u64) -> Self {
        match start_ns.checked_add(HEALTH_EMISSION_INTERVAL_NS) {
            Some(next_due_ns) => Self {
                next_due_ns,
                armed: true,
            },
            None => Self {
                next_due_ns: u64::MAX,
                armed: false,
            },
        }
    }

    /// Returns one due emission and advances to the next fixed interval.
    /// Missed intervals are skipped so a delayed loop never emits a burst.
    pub(super) fn take_due(&mut self, now_ns: u64) -> bool {
        if !self.armed || now_ns < self.next_due_ns {
            return false;
        }
        let skipped_intervals =
            now_ns.saturating_sub(self.next_due_ns) / HEALTH_EMISSION_INTERVAL_NS;
        let intervals_to_advance = skipped_intervals.saturating_add(1);
        let advance = intervals_to_advance.checked_mul(HEALTH_EMISSION_INTERVAL_NS);
        match advance.and_then(|advance| self.next_due_ns.checked_add(advance)) {
            Some(next_due_ns) => self.next_due_ns = next_due_ns,
            None => self.armed = false,
        }
        true
    }
}

fn atomic_saturating_add(value: &AtomicU64, amount: u64) {
    let _ = value.fetch_update(Ordering::Relaxed, Ordering::Relaxed, |current| {
        Some(current.saturating_add(amount))
    });
}

const fn live_phase_to_u8(phase: LivePhase) -> u8 {
    match phase {
        LivePhase::Configured => 0,
        LivePhase::Reconciling => 1,
        LivePhase::AwaitingStreams => 2,
        LivePhase::Ready => 3,
        LivePhase::Degraded => 4,
        LivePhase::Stopping => 5,
    }
}

const fn live_phase_from_u8(value: u8) -> LivePhase {
    match value {
        1 => LivePhase::Reconciling,
        2 => LivePhase::AwaitingStreams,
        3 => LivePhase::Ready,
        4 => LivePhase::Degraded,
        5 => LivePhase::Stopping,
        _ => LivePhase::Configured,
    }
}

#[cfg(test)]
mod tests {
    use serde_json::json;

    use super::*;

    const SECOND_NS: u64 = 1_000_000_000;

    #[test]
    fn connectivity_and_heartbeat_transitions_have_monotonic_progress_ages() {
        let health = RuntimeHealthState::new();
        let initial = health.snapshot_at(SECOND_NS, StorageHealthInput::default(), false);

        assert_eq!(
            initial.connectivity.map(|component| component.state),
            [ConnectivityHealthState::Unknown; CONNECTIVITY_COMPONENT_COUNT]
        );
        assert!(
            initial
                .connectivity
                .iter()
                .all(|component| component.last_progress_age_ms.is_none())
        );
        assert!(
            initial
                .heartbeats
                .iter()
                .all(|heartbeat| heartbeat.last_progress_age_ms.is_none())
        );

        health.set_connectivity_at(
            ConnectivityId::Feed,
            ConnectivityHealthState::Connecting,
            SECOND_NS,
        );
        health.set_connectivity_at(
            ConnectivityId::Feed,
            ConnectivityHealthState::Ready,
            2 * SECOND_NS,
        );
        health.mark_connectivity_progress_at(ConnectivityId::Feed, 3 * SECOND_NS);
        health.mark_heartbeat_at(HeartbeatId::RuntimeEventLoop, 3 * SECOND_NS);
        health.mark_heartbeat_at(HeartbeatId::RuntimeEventLoop, 4 * SECOND_NS);
        health.mark_heartbeat_at(HeartbeatId::OrderTask, 4 * SECOND_NS);

        let snapshot = health.snapshot_at(
            5 * SECOND_NS + 500_000_000,
            StorageHealthInput::default(),
            false,
        );
        assert_eq!(
            snapshot.connectivity[ConnectivityId::Feed.index()],
            ConnectivityHealthSnapshot {
                id: ConnectivityId::Feed,
                state: ConnectivityHealthState::Ready,
                last_progress_age_ms: Some(2_500),
            }
        );
        assert_eq!(
            snapshot.heartbeats[HeartbeatId::RuntimeEventLoop.index()],
            HeartbeatHealthSnapshot {
                id: HeartbeatId::RuntimeEventLoop,
                progress_count: 2,
                last_progress_age_ms: Some(1_500),
            }
        );
        assert_eq!(
            snapshot.heartbeats[HeartbeatId::OrderTask.index()],
            HeartbeatHealthSnapshot {
                id: HeartbeatId::OrderTask,
                progress_count: 1,
                last_progress_age_ms: Some(1_500),
            }
        );
    }

    #[test]
    fn queues_readiness_counters_and_storage_progress_are_distinct_and_bounded() {
        let health = RuntimeHealthState::new();
        health.configure_queue(QueueId::OrderCommand, 4);
        health.observe_queue_depth(QueueId::OrderCommand, 1);
        health.observe_queue_depth(QueueId::OrderCommand, 4);
        health.observe_queue_residence(QueueId::OrderCommand, 2_000_000);
        health.observe_queue_residence(QueueId::OrderCommand, 7_500_000);
        health.observe_queue_depth(QueueId::OrderCommand, 2);
        health.observe_queue_saturation(QueueId::OrderCommand);

        health.set_readiness(LivePhase::Ready, 2);
        health.increment_counter(HealthCounterId::SubmitRequests, 3);
        health.increment_counter(HealthCounterId::SubmitAcceptedAcks, 2);
        health.increment_counter(HealthCounterId::CancelRequests, 5);
        health.increment_counter(HealthCounterId::CancelAcceptedAcks, 4);
        health.increment_counter(HealthCounterId::PolicyRiskRejected, 7);
        health.increment_counter(HealthCounterId::ExchangeRejected, 11);
        health.increment_counter(HealthCounterId::Ambiguous, 13);
        health.increment_counter(HealthCounterId::ReconciliationTotal, 17);
        health.increment_counter(HealthCounterId::ReconciliationClean, 19);

        let storage = StorageHealthInput {
            queue_capacity: 8,
            queue_depth: 3,
            queue_high_water: 6,
            records_enqueued: 23,
            records_written: 21,
            records_outstanding: 1,
            durable_sync_completions: 5,
            write_failures: 1,
            sync_failures: 2,
            dropped_records: 4,
            last_writer_progress_age_ns: Some(9_500_000),
        };
        let snapshot = health.snapshot_at(SECOND_NS, storage, false);

        assert_eq!(snapshot.connectivity.len(), CONNECTIVITY_COMPONENT_COUNT);
        assert_eq!(snapshot.heartbeats.len(), HEARTBEAT_COMPONENT_COUNT);
        assert_eq!(snapshot.queues.len(), QUEUE_COMPONENT_COUNT);
        assert_eq!(
            snapshot.queues.map(|queue| queue.id),
            [
                QueueId::FeedIngress,
                QueueId::ControlIngress,
                QueueId::OrderCommand,
            ]
        );
        assert_eq!(
            snapshot.queues[QueueId::OrderCommand.index()],
            QueueHealthSnapshot {
                id: QueueId::OrderCommand,
                capacity: 4,
                depth: 2,
                high_water_mark: 4,
                last_residence_age_ms: Some(7),
                max_residence_age_ms: 7,
                saturated: true,
                saturation_events: 2,
            }
        );
        assert_eq!(
            snapshot.readiness,
            ReadinessHealthSnapshot {
                state: LivePhase::Ready,
                active_durable_safety_latches: 2,
            }
        );
        assert_eq!(
            snapshot.orders,
            OrderHealthCountersSnapshot {
                submit_requests: 3,
                submit_accepted_acks: 2,
                cancel_requests: 5,
                cancel_accepted_acks: 4,
                policy_risk_rejected: 7,
                exchange_rejected: 11,
                ambiguous: 13,
                reconciliation_total: 17,
                reconciliation_clean: 19,
            }
        );
        assert_eq!(
            snapshot.storage,
            StorageHealthSnapshot {
                queue_capacity: 8,
                queue_depth: 3,
                queue_high_water: 6,
                queue_saturated: false,
                records_enqueued: 23,
                records_written: 21,
                records_outstanding: 1,
                durable_sync_completions: 5,
                write_failures: 1,
                sync_failures: 2,
                dropped_records: 4,
                last_writer_progress_age_ms: Some(9),
            }
        );

        health.increment_counter(HealthCounterId::Ambiguous, u64::MAX);
        assert_eq!(
            health
                .snapshot_at(SECOND_NS, StorageHealthInput::default(), false)
                .orders
                .ambiguous,
            u64::MAX,
            "progress counters must saturate rather than wrap"
        );
    }

    #[test]
    fn schema_one_serialization_is_exact_and_has_no_dynamic_identifiers() {
        let health = RuntimeHealthState::new();
        health.set_connectivity_at(
            ConnectivityId::Private,
            ConnectivityHealthState::Disconnected,
            SECOND_NS,
        );
        health.mark_heartbeat_at(HeartbeatId::RuntimeEventLoop, 2 * SECOND_NS);
        health.configure_queue(QueueId::FeedIngress, 16);
        health.observe_queue_depth(QueueId::FeedIngress, 4);
        health.observe_queue_residence(QueueId::FeedIngress, 3_000_000);
        health.set_readiness(LivePhase::Degraded, 1);
        health.increment_counter(HealthCounterId::ExchangeRejected, 1);

        let serialized = serde_json::to_value(health.snapshot_at(
            3 * SECOND_NS,
            StorageHealthInput::default(),
            false,
        ))
        .unwrap();
        assert_eq!(
            serialized,
            json!({
                "schema_version": 1,
                "final_snapshot": false,
                "connectivity": [
                    {"id": "feed", "state": "unknown", "last_progress_age_ms": null},
                    {"id": "private", "state": "disconnected", "last_progress_age_ms": 2000},
                    {"id": "order_command", "state": "unknown", "last_progress_age_ms": null}
                ],
                "heartbeats": [
                    {"id": "runtime_event_loop", "progress_count": 1, "last_progress_age_ms": 1000},
                    {"id": "order_task", "progress_count": 0, "last_progress_age_ms": null}
                ],
                "queues": [
                    {
                        "id": "feed_ingress",
                        "capacity": 16,
                        "depth": 4,
                        "high_water_mark": 4,
                        "last_residence_age_ms": 3,
                        "max_residence_age_ms": 3,
                        "saturated": false,
                        "saturation_events": 0
                    },
                    {
                        "id": "control_ingress",
                        "capacity": 0,
                        "depth": 0,
                        "high_water_mark": 0,
                        "last_residence_age_ms": null,
                        "max_residence_age_ms": 0,
                        "saturated": false,
                        "saturation_events": 0
                    },
                    {
                        "id": "order_command",
                        "capacity": 0,
                        "depth": 0,
                        "high_water_mark": 0,
                        "last_residence_age_ms": null,
                        "max_residence_age_ms": 0,
                        "saturated": false,
                        "saturation_events": 0
                    }
                ],
                "readiness": {
                    "state": "degraded",
                    "active_durable_safety_latches": 1
                },
                "orders": {
                    "submit_requests": 0,
                    "submit_accepted_acks": 0,
                    "cancel_requests": 0,
                    "cancel_accepted_acks": 0,
                    "policy_risk_rejected": 0,
                    "exchange_rejected": 1,
                    "ambiguous": 0,
                    "reconciliation_total": 0,
                    "reconciliation_clean": 0
                },
                "storage": {
                    "queue_capacity": 0,
                    "queue_depth": 0,
                    "queue_high_water": 0,
                    "queue_saturated": false,
                    "records_enqueued": 0,
                    "records_written": 0,
                    "records_outstanding": 0,
                    "durable_sync_completions": 0,
                    "write_failures": 0,
                    "sync_failures": 0,
                    "dropped_records": 0,
                    "last_writer_progress_age_ms": null
                }
            })
        );

        let final_serialized = serde_json::to_value(health.snapshot_at(
            3 * SECOND_NS,
            StorageHealthInput::default(),
            true,
        ))
        .unwrap();
        assert_eq!(final_serialized["final_snapshot"], true);
        let mut periodic_without_kind = serialized;
        periodic_without_kind
            .as_object_mut()
            .unwrap()
            .remove("final_snapshot");
        let mut final_without_kind = final_serialized;
        final_without_kind
            .as_object_mut()
            .unwrap()
            .remove("final_snapshot");
        assert_eq!(
            final_without_kind, periodic_without_kind,
            "periodic and final schema-1 payloads differ only by the explicit final marker"
        );
    }

    #[test]
    fn five_second_cadence_skips_missed_ticks_without_bursting() {
        let start_ns = 17;
        let mut cadence = HealthEmissionCadence::starting_at(start_ns);

        assert!(!cadence.take_due(start_ns + HEALTH_EMISSION_INTERVAL_NS - 1));
        assert!(cadence.take_due(start_ns + HEALTH_EMISSION_INTERVAL_NS));
        assert!(!cadence.take_due(start_ns + HEALTH_EMISSION_INTERVAL_NS));
        assert!(cadence.take_due(start_ns + 17_000_000_000));
        assert!(!cadence.take_due(start_ns + 19_999_999_999));
        assert!(cadence.take_due(start_ns + 20_000_000_000));
    }

    #[test]
    fn health_state_is_send_sync_and_fixed_cardinality() {
        fn require_send_sync<T: Send + Sync>() {}

        require_send_sync::<RuntimeHealthState>();
        let health = RuntimeHealthState::new();
        health.set_connectivity(ConnectivityId::Feed, ConnectivityHealthState::Ready);
        health.mark_connectivity_progress(ConnectivityId::Feed);
        health.mark_heartbeat(HeartbeatId::RuntimeEventLoop);
        assert!(
            !health
                .periodic_snapshot(StorageHealthInput::default())
                .final_snapshot
        );
        assert!(
            health
                .final_snapshot(StorageHealthInput::default())
                .final_snapshot
        );

        assert_eq!(ConnectivityId::ALL.len(), CONNECTIVITY_COMPONENT_COUNT);
        assert_eq!(HeartbeatId::ALL.len(), HEARTBEAT_COMPONENT_COUNT);
        assert_eq!(QueueId::ALL.len(), QUEUE_COMPONENT_COUNT);
        assert_eq!(HealthCounterId::ALL.len(), HEALTH_COUNTER_COUNT);
    }

    #[test]
    fn production_health_core_has_no_dynamic_registration_or_locking() {
        let source = include_str!("health.rs");
        let production_source = source
            .split_once("\n#[cfg(test)]\nmod tests {")
            .expect("health tests must remain separated from production code")
            .0;

        for forbidden in [
            "Mutex",
            "RwLock",
            "HashMap",
            "BTreeMap",
            "String",
            "Vec<",
            "Box<",
            "format!",
            "to_string",
        ] {
            assert!(
                !production_source.contains(forbidden),
                "production health core must not contain `{forbidden}`"
            );
        }
    }
}
