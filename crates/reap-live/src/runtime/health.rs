use std::sync::Arc;
use std::sync::atomic::{AtomicBool, AtomicU8, AtomicU64, AtomicUsize, Ordering};
use std::time::Instant;

use reap_storage::{OrderAckStatus, OrderOperation, StorageRecord};
use serde::Serialize;
use tokio::sync::mpsc;

use super::LiveRuntime;
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
    NotRequired = 5,
}

impl ConnectivityHealthState {
    fn from_u8(value: u8) -> Self {
        match value {
            1 => Self::Connecting,
            2 => Self::Ready,
            3 => Self::Disconnected,
            4 => Self::Failed,
            5 => Self::NotRequired,
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
    LocalRejected = 4,
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
        Self::LocalRejected,
        Self::ExchangeRejected,
        Self::Ambiguous,
        Self::ReconciliationTotal,
        Self::ReconciliationClean,
    ];

    const fn index(self) -> usize {
        self as usize
    }
}

#[derive(Clone, Copy, Default)]
pub(super) struct HealthCounterUpdates {
    first: Option<HealthCounterId>,
    second: Option<HealthCounterId>,
}

impl HealthCounterUpdates {
    pub(super) fn from_storage_record(record: &StorageRecord) -> Self {
        match record {
            StorageRecord::OrderRequest(request) => Self {
                first: Some(match request.operation {
                    OrderOperation::Submit => HealthCounterId::SubmitRequests,
                    OrderOperation::Cancel => HealthCounterId::CancelRequests,
                }),
                second: None,
            },
            StorageRecord::OrderAck(ack) => match ack.status {
                OrderAckStatus::Accepted => Self {
                    first: Some(match ack.operation {
                        OrderOperation::Submit => HealthCounterId::SubmitAcceptedAcks,
                        OrderOperation::Cancel => HealthCounterId::CancelAcceptedAcks,
                    }),
                    second: None,
                },
                OrderAckStatus::Rejected => Self {
                    first: Some(HealthCounterId::ExchangeRejected),
                    second: None,
                },
                OrderAckStatus::Ambiguous | OrderAckStatus::PendingReconciliation => Self {
                    first: Some(HealthCounterId::Ambiguous),
                    second: None,
                },
                OrderAckStatus::Duplicate => Self::default(),
            },
            StorageRecord::IntentRejected { .. } => Self {
                first: Some(HealthCounterId::LocalRejected),
                second: None,
            },
            StorageRecord::Reconciliation(reconciliation) => Self {
                first: Some(HealthCounterId::ReconciliationTotal),
                second: reconciliation
                    .clean
                    .then_some(HealthCounterId::ReconciliationClean),
            },
            _ => Self::default(),
        }
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
    expected: AtomicBool,
    state: AtomicU8,
    progress: ProgressClock,
}

impl ConnectivitySlot {
    fn new() -> Self {
        Self {
            expected: AtomicBool::new(false),
            state: AtomicU8::new(ConnectivityHealthState::Unknown as u8),
            progress: ProgressClock::new(),
        }
    }
}

struct HeartbeatLane {
    running: AtomicBool,
    progress_count: AtomicU64,
    progress: ProgressClock,
}

impl HeartbeatLane {
    fn new() -> Self {
        Self {
            running: AtomicBool::new(false),
            progress_count: AtomicU64::new(0),
            progress: ProgressClock::new(),
        }
    }
}

struct HeartbeatSlot {
    lanes: Box<[HeartbeatLane]>,
}

impl HeartbeatSlot {
    fn with_lane_count(lane_count: usize) -> Self {
        Self {
            lanes: (0..lane_count)
                .map(|_| HeartbeatLane::new())
                .collect::<Box<[_]>>(),
        }
    }
}

pub(super) struct HeartbeatGuard {
    health: Arc<RuntimeHealthState>,
    id: HeartbeatId,
    lane_index: usize,
}

impl HeartbeatGuard {
    pub(super) fn start(
        health: Arc<RuntimeHealthState>,
        id: HeartbeatId,
        lane_index: usize,
    ) -> Self {
        health.set_heartbeat_running(id, lane_index, true);
        health.mark_heartbeat_lane(id, lane_index);
        Self {
            health,
            id,
            lane_index,
        }
    }
}

impl Drop for HeartbeatGuard {
    fn drop(&mut self) {
        self.health
            .set_heartbeat_running(self.id, self.lane_index, false);
    }
}

pub(super) struct OrderTaskHealth {
    health: Arc<RuntimeHealthState>,
    lane_index: usize,
}

impl OrderTaskHealth {
    pub(super) fn new(health: Arc<RuntimeHealthState>, lane_index: usize) -> Self {
        Self { health, lane_index }
    }

    pub(super) fn start(&self) -> HeartbeatGuard {
        HeartbeatGuard::start(
            Arc::clone(&self.health),
            HeartbeatId::OrderTask,
            self.lane_index,
        )
    }

    pub(super) fn mark(&self) {
        self.health
            .mark_heartbeat_lane(HeartbeatId::OrderTask, self.lane_index);
    }
}

struct QueueLaneState {
    capacity: AtomicU64,
    depth: AtomicU64,
    backlog_token: AtomicU64,
    continuous_backlog_since_ns: AtomicU64,
}

impl QueueLaneState {
    fn new() -> Self {
        Self {
            capacity: AtomicU64::new(0),
            depth: AtomicU64::new(0),
            backlog_token: AtomicU64::new(0),
            continuous_backlog_since_ns: AtomicU64::new(0),
        }
    }
}

struct QueueSlot {
    lanes: Box<[QueueLaneState]>,
    next_lane: AtomicUsize,
    aggregate_depth: AtomicU64,
    high_water_mark: AtomicU64,
    residence_observed: AtomicBool,
    last_residence_age_ns: AtomicU64,
    max_residence_age_ns: AtomicU64,
    saturation_events: AtomicU64,
}

impl QueueSlot {
    fn with_lane_count(lane_count: usize) -> Self {
        Self {
            lanes: (0..lane_count)
                .map(|_| QueueLaneState::new())
                .collect::<Box<[_]>>(),
            next_lane: AtomicUsize::new(0),
            aggregate_depth: AtomicU64::new(0),
            high_water_mark: AtomicU64::new(0),
            residence_observed: AtomicBool::new(false),
            last_residence_age_ns: AtomicU64::new(0),
            max_residence_age_ns: AtomicU64::new(0),
            saturation_events: AtomicU64::new(0),
        }
    }

    fn claim_lane(&self, capacity: u64) -> usize {
        let lane_index = self.next_lane.fetch_add(1, Ordering::Relaxed);
        let lane = self
            .lanes
            .get(lane_index)
            .expect("tracked queue lane count must be predeclared at startup");
        let previous = lane.capacity.swap(capacity, Ordering::Relaxed);
        assert_eq!(
            previous, 0,
            "a predeclared tracked queue lane may be claimed only once"
        );
        lane_index
    }
}

struct QueueLane {
    health: Arc<RuntimeHealthState>,
    id: QueueId,
    lane_index: usize,
    capacity: u64,
}

#[derive(Clone, Copy)]
struct QueueRemoval {
    backlog_token: u64,
    lane_depth: u64,
}

impl QueueLane {
    fn new(health: Arc<RuntimeHealthState>, id: QueueId, capacity: usize) -> Arc<Self> {
        let capacity = u64::try_from(capacity).unwrap_or(u64::MAX);
        let lane_index = health.queues[id.index()].claim_lane(capacity);
        Arc::new(Self {
            health,
            id,
            lane_index,
            capacity,
        })
    }

    fn state(&self) -> &QueueLaneState {
        &self.health.queues[self.id.index()].lanes[self.lane_index]
    }

    fn enqueue(&self) -> u64 {
        let enqueued_at_ns = self.health.monotonic_now_ns();
        self.enqueue_at(enqueued_at_ns);
        enqueued_at_ns
    }

    fn enqueue_at(&self, enqueued_at_ns: u64) {
        let lane_depth = atomic_saturating_increment(&self.state().depth);
        if lane_depth == 1 {
            self.state()
                .continuous_backlog_since_ns
                .store(enqueued_at_ns.saturating_add(1), Ordering::Relaxed);
            // A timestamp is not a safe generation identity: distinct
            // backlogs may begin on the same timer tick. Advance a separate
            // nonzero token so stale dequeue cleanup cannot hit that ABA.
            let _ = self.state().backlog_token.fetch_update(
                Ordering::Release,
                Ordering::Relaxed,
                |current| Some(if current == u64::MAX { 1 } else { current + 1 }),
            );
        }
        let slot = &self.health.queues[self.id.index()];
        let aggregate_depth = atomic_saturating_increment(&slot.aggregate_depth);
        slot.high_water_mark
            .fetch_max(aggregate_depth, Ordering::Relaxed);
        if self.capacity > 0 && lane_depth == self.capacity {
            atomic_saturating_add(&slot.saturation_events, 1);
        }
    }

    fn begin_remove(&self) -> QueueRemoval {
        let state = self.state();
        // Capture identity before releasing accounting depth. A producer that
        // observes zero advances the token before the replacement is visible.
        let backlog_token = state.backlog_token.load(Ordering::Acquire);
        let lane_depth = atomic_saturating_sub(&state.depth, 1);
        let slot = &self.health.queues[self.id.index()];
        atomic_saturating_sub(&slot.aggregate_depth, 1);
        QueueRemoval {
            backlog_token,
            lane_depth,
        }
    }

    fn finish_remove(&self, removal: QueueRemoval) {
        if removal.lane_depth == 0 {
            let _ = self.state().backlog_token.compare_exchange(
                removal.backlog_token,
                0,
                Ordering::AcqRel,
                Ordering::Acquire,
            );
        }
    }

    fn dequeue(&self, enqueued_at_ns: u64) {
        let removal = self.begin_remove();
        self.finish_remove(removal);
        self.health.observe_queue_residence(
            self.id,
            self.health
                .monotonic_now_ns()
                .saturating_sub(enqueued_at_ns),
        );
    }

    fn discard(&self) {
        let removal = self.begin_remove();
        self.finish_remove(removal);
    }

    fn observe_full_attempt(&self) {
        atomic_saturating_add(&self.health.queues[self.id.index()].saturation_events, 1);
    }
}

struct Queued<T> {
    value: Option<T>,
    enqueued_at_ns: u64,
    lane: Arc<QueueLane>,
    armed: bool,
}

impl<T> Queued<T> {
    fn new(value: T, lane: Arc<QueueLane>) -> Self {
        let enqueued_at_ns = lane.enqueue();
        Self {
            value: Some(value),
            enqueued_at_ns,
            lane,
            armed: true,
        }
    }

    fn consume(mut self) -> T {
        self.lane.dequeue(self.enqueued_at_ns);
        self.armed = false;
        self.value
            .take()
            .expect("a queued value must be consumed at most once")
    }
}

impl<T> Drop for Queued<T> {
    fn drop(&mut self) {
        if self.armed {
            self.lane.discard();
        }
    }
}

pub(super) struct TrackedSender<T> {
    sender: mpsc::Sender<Queued<T>>,
    lane: Arc<QueueLane>,
}

impl<T> Clone for TrackedSender<T> {
    fn clone(&self) -> Self {
        Self {
            sender: self.sender.clone(),
            lane: Arc::clone(&self.lane),
        }
    }
}

impl<T> TrackedSender<T> {
    pub(super) async fn send(&self, value: T) -> Result<(), mpsc::error::SendError<T>> {
        let permit = match self.sender.try_reserve() {
            Ok(permit) => permit,
            Err(mpsc::error::TrySendError::Full(_)) => {
                self.lane.observe_full_attempt();
                match self.sender.reserve().await {
                    Ok(permit) => permit,
                    Err(_) => return Err(mpsc::error::SendError(value)),
                }
            }
            Err(mpsc::error::TrySendError::Closed(_)) => {
                return Err(mpsc::error::SendError(value));
            }
        };
        permit.send(Queued::new(value, Arc::clone(&self.lane)));
        Ok(())
    }

    pub(super) fn try_send(&self, value: T) -> Result<(), mpsc::error::TrySendError<T>> {
        let permit = match self.sender.try_reserve() {
            Ok(permit) => permit,
            Err(mpsc::error::TrySendError::Full(_)) => {
                self.lane.observe_full_attempt();
                return Err(mpsc::error::TrySendError::Full(value));
            }
            Err(mpsc::error::TrySendError::Closed(_)) => {
                return Err(mpsc::error::TrySendError::Closed(value));
            }
        };
        permit.send(Queued::new(value, Arc::clone(&self.lane)));
        Ok(())
    }
}

pub(super) struct TrackedReceiver<T> {
    receiver: mpsc::Receiver<Queued<T>>,
}

impl<T> TrackedReceiver<T> {
    pub(super) async fn recv(&mut self) -> Option<T> {
        self.receiver
            .recv()
            .await
            .map(|queued| self.consume(queued))
    }

    pub(super) fn try_recv(&mut self) -> Result<T, mpsc::error::TryRecvError> {
        self.receiver.try_recv().map(|queued| self.consume(queued))
    }

    pub(super) fn len(&self) -> usize {
        self.receiver.len()
    }

    pub(super) fn is_closed(&self) -> bool {
        self.receiver.is_closed()
    }

    pub(super) fn close(&mut self) {
        self.receiver.close();
    }

    fn consume(&self, queued: Queued<T>) -> T {
        queued.consume()
    }
}

pub(super) fn tracked_channel<T>(
    health: Arc<RuntimeHealthState>,
    id: QueueId,
    capacity: usize,
) -> (TrackedSender<T>, TrackedReceiver<T>) {
    let (sender, receiver) = mpsc::channel(capacity);
    let lane = QueueLane::new(health, id, capacity);
    (
        TrackedSender {
            sender,
            lane: Arc::clone(&lane),
        },
        TrackedReceiver { receiver },
    )
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
    #[cfg(test)]
    pub(super) fn new() -> Self {
        Self::with_order_task_count(0)
    }

    pub(super) fn with_order_task_count(order_task_count: usize) -> Self {
        Self {
            origin: Instant::now(),
            connectivity: std::array::from_fn(|_| ConnectivitySlot::new()),
            heartbeats: [
                HeartbeatSlot::with_lane_count(1),
                HeartbeatSlot::with_lane_count(order_task_count),
            ],
            queues: [
                QueueSlot::with_lane_count(1),
                QueueSlot::with_lane_count(1),
                QueueSlot::with_lane_count(order_task_count),
            ],
            readiness: AtomicU8::new(live_phase_to_u8(LivePhase::Configured)),
            active_durable_safety_latches: AtomicU64::new(0),
            counters: std::array::from_fn(|_| AtomicU64::new(0)),
        }
    }

    pub(super) fn monotonic_now_ns(&self) -> u64 {
        u64::try_from(self.origin.elapsed().as_nanos()).unwrap_or(u64::MAX)
    }

    #[cfg(test)]
    pub(super) fn set_connectivity(&self, id: ConnectivityId, state: ConnectivityHealthState) {
        self.set_connectivity_at(id, state, self.monotonic_now_ns());
    }

    pub(super) fn set_connectivity_expected(&self, id: ConnectivityId, expected: bool) {
        let slot = &self.connectivity[id.index()];
        slot.expected.store(expected, Ordering::Relaxed);
        slot.state.store(
            if expected {
                ConnectivityHealthState::Connecting
            } else {
                ConnectivityHealthState::NotRequired
            } as u8,
            Ordering::Relaxed,
        );
    }

    pub(super) fn set_connectivity_state(
        &self,
        id: ConnectivityId,
        state: ConnectivityHealthState,
    ) {
        self.connectivity[id.index()]
            .state
            .store(state as u8, Ordering::Relaxed);
    }

    fn projected_connectivity_state(
        &self,
        id: ConnectivityId,
        ready: bool,
    ) -> ConnectivityHealthState {
        if !self.connectivity_expected(id) {
            return ConnectivityHealthState::NotRequired;
        }
        let current = ConnectivityHealthState::from_u8(
            self.connectivity[id.index()].state.load(Ordering::Relaxed),
        );
        if current == ConnectivityHealthState::Failed {
            return current;
        }
        if ready {
            ConnectivityHealthState::Ready
        } else if current == ConnectivityHealthState::Connecting {
            ConnectivityHealthState::Connecting
        } else {
            ConnectivityHealthState::Disconnected
        }
    }

    fn finalized_connectivity_state(&self, id: ConnectivityId) -> ConnectivityHealthState {
        if !self.connectivity_expected(id) {
            return ConnectivityHealthState::NotRequired;
        }
        let current = ConnectivityHealthState::from_u8(
            self.connectivity[id.index()].state.load(Ordering::Relaxed),
        );
        if current == ConnectivityHealthState::Failed {
            current
        } else {
            ConnectivityHealthState::Disconnected
        }
    }

    #[cfg(test)]
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
        self.mark_heartbeat_lane(id, 0);
    }

    pub(super) fn mark_heartbeat_lane(&self, id: HeartbeatId, lane_index: usize) {
        self.mark_heartbeat_lane_at(id, lane_index, self.monotonic_now_ns());
    }

    #[cfg(test)]
    fn mark_heartbeat_at(&self, id: HeartbeatId, now_ns: u64) {
        self.mark_heartbeat_lane_at(id, 0, now_ns);
    }

    fn mark_heartbeat_lane_at(&self, id: HeartbeatId, lane_index: usize, now_ns: u64) {
        let lane = &self.heartbeats[id.index()].lanes[lane_index];
        atomic_saturating_add(&lane.progress_count, 1);
        lane.progress.observe_at(now_ns);
    }

    fn set_heartbeat_running(&self, id: HeartbeatId, lane_index: usize, running: bool) {
        self.heartbeats[id.index()].lanes[lane_index]
            .running
            .store(running, Ordering::Relaxed);
    }

    #[cfg(test)]
    pub(super) fn configure_queue(&self, id: QueueId, capacity: u64) {
        let slot = &self.queues[id.index()];
        slot.lanes[0].capacity.store(capacity, Ordering::Relaxed);
    }

    #[cfg(test)]
    pub(super) fn observe_queue_depth(&self, id: QueueId, depth: u64) {
        self.observe_queue_depth_at(id, depth, self.monotonic_now_ns());
    }

    #[cfg(test)]
    fn observe_queue_depth_at(&self, id: QueueId, depth: u64, now_ns: u64) {
        let slot = &self.queues[id.index()];
        let lane = &slot.lanes[0];
        let previous = lane.depth.swap(depth, Ordering::AcqRel);
        slot.aggregate_depth.store(depth, Ordering::Relaxed);
        slot.high_water_mark.fetch_max(depth, Ordering::Relaxed);
        if previous == 0 && depth > 0 {
            lane.continuous_backlog_since_ns
                .store(now_ns.saturating_add(1), Ordering::Relaxed);
            lane.backlog_token.store(1, Ordering::Release);
        } else if depth == 0 {
            lane.backlog_token.store(0, Ordering::Release);
            lane.continuous_backlog_since_ns.store(0, Ordering::Relaxed);
        }
        let capacity = lane.capacity.load(Ordering::Relaxed);
        if capacity > 0 && previous < capacity && depth >= capacity {
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

    #[cfg(test)]
    pub(super) fn observe_queue_saturation(&self, id: QueueId) {
        let slot = &self.queues[id.index()];
        atomic_saturating_add(&slot.saturation_events, 1);
    }

    pub(super) fn set_readiness(&self, state: LivePhase, active_durable_safety_latches: u64) {
        self.readiness
            .store(live_phase_to_u8(state), Ordering::Relaxed);
        self.active_durable_safety_latches
            .store(active_durable_safety_latches, Ordering::Relaxed);
    }

    pub(super) fn set_active_durable_safety_latches(&self, count: u64) {
        self.active_durable_safety_latches
            .store(count, Ordering::Relaxed);
    }

    pub(super) fn increment_counter(&self, id: HealthCounterId, amount: u64) {
        atomic_saturating_add(&self.counters[id.index()], amount);
    }

    pub(super) fn apply_counter_updates(&self, updates: HealthCounterUpdates) {
        if let Some(id) = updates.first {
            self.increment_counter(id, 1);
        }
        if let Some(id) = updates.second {
            self.increment_counter(id, 1);
        }
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
                let expected_instances = usize_to_u64(slot.lanes.len());
                let running_instances = usize_to_u64(
                    slot.lanes
                        .iter()
                        .filter(|lane| lane.running.load(Ordering::Relaxed))
                        .count(),
                );
                let observed_instances = usize_to_u64(
                    slot.lanes
                        .iter()
                        .filter(|lane| lane.progress.observed.load(Ordering::Acquire))
                        .count(),
                );
                let progress_count = slot.lanes.iter().fold(0_u64, |total, lane| {
                    total.saturating_add(lane.progress_count.load(Ordering::Relaxed))
                });
                let oldest_progress_age_ms = slot
                    .lanes
                    .iter()
                    .filter_map(|lane| lane.progress.age_ms_at(now_ns))
                    .max();
                HeartbeatHealthSnapshot {
                    id,
                    expected_instances,
                    running_instances,
                    unobserved_instances: expected_instances.saturating_sub(observed_instances),
                    progress_count,
                    oldest_progress_age_ms,
                }
            }),
            queues: QueueId::ALL.map(|id| {
                let slot = &self.queues[id.index()];
                let mut capacity = 0_u64;
                let mut depth = 0_u64;
                let mut saturated = false;
                let mut continuous_backlog_age_ms: Option<u64> = None;
                for lane in &slot.lanes {
                    let lane_capacity = lane.capacity.load(Ordering::Relaxed);
                    let raw_lane_depth = lane.depth.load(Ordering::Acquire);
                    let lane_depth = raw_lane_depth.min(lane_capacity);
                    capacity = capacity.saturating_add(lane_capacity);
                    depth = depth.saturating_add(lane_depth);
                    saturated |= lane_capacity > 0 && raw_lane_depth >= lane_capacity;
                    if lane_depth > 0 {
                        let backlog_token = lane.backlog_token.load(Ordering::Acquire);
                        // The depth RMW necessarily precedes token publication.
                        // A concurrent snapshot may land between those atomics;
                        // report a conservative zero age instead of falsely
                        // claiming that a non-empty lane has no backlog.
                        let lane_age_ms = if backlog_token == 0 {
                            0
                        } else {
                            lane.continuous_backlog_since_ns
                                .load(Ordering::Relaxed)
                                .checked_sub(1)
                                .map_or(0, |origin_ns| now_ns.saturating_sub(origin_ns) / NS_PER_MS)
                        };
                        continuous_backlog_age_ms =
                            Some(continuous_backlog_age_ms.unwrap_or(0).max(lane_age_ms));
                    }
                }
                depth = depth.min(capacity);
                let residence_observed = slot.residence_observed.load(Ordering::Acquire);
                QueueHealthSnapshot {
                    id,
                    capacity,
                    // Tokio releases its internal channel permit immediately before
                    // `recv` returns. Clamp the sub-instruction accounting handoff
                    // without adding a second capacity gate that could perturb
                    // producer ordering.
                    depth,
                    high_water_mark: slot
                        .high_water_mark
                        .load(Ordering::Relaxed)
                        .max(depth)
                        .min(capacity),
                    continuous_backlog_age_ms,
                    last_residence_age_ms: residence_observed
                        .then(|| slot.last_residence_age_ns.load(Ordering::Relaxed) / NS_PER_MS),
                    max_residence_age_ms: slot.max_residence_age_ns.load(Ordering::Relaxed)
                        / NS_PER_MS,
                    saturated,
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
                local_rejected: self.counter(HealthCounterId::LocalRejected),
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

impl LiveRuntime {
    pub(super) fn emit_periodic_health_snapshot(&mut self) {
        self.emit_health_snapshot(false);
    }

    pub(super) fn emit_final_health_snapshot(&mut self) -> bool {
        if self.health_final_emitted {
            return false;
        }
        self.health_final_emitted = true;
        self.emit_health_snapshot(true);
        true
    }

    fn emit_health_snapshot(&mut self, final_snapshot: bool) {
        let readiness = self.coordinator.readiness();
        self.health.set_connectivity_state(
            ConnectivityId::Feed,
            if final_snapshot {
                self.health
                    .finalized_connectivity_state(ConnectivityId::Feed)
            } else {
                self.health.projected_connectivity_state(
                    ConnectivityId::Feed,
                    readiness.public_connectivity_ready,
                )
            },
        );
        self.health.set_connectivity_state(
            ConnectivityId::Private,
            if final_snapshot {
                self.health
                    .finalized_connectivity_state(ConnectivityId::Private)
            } else {
                self.health.projected_connectivity_state(
                    ConnectivityId::Private,
                    readiness.missing_private_streams.is_empty(),
                )
            },
        );
        self.health.set_connectivity_state(
            ConnectivityId::OrderCommand,
            if final_snapshot {
                self.health
                    .finalized_connectivity_state(ConnectivityId::OrderCommand)
            } else {
                self.health.projected_connectivity_state(
                    ConnectivityId::OrderCommand,
                    readiness.missing_order_transports.is_empty(),
                )
            },
        );
        let phase = if final_snapshot {
            LivePhase::Stopping
        } else {
            readiness.phase
        };
        self.health
            .set_readiness(phase, self.durable_latches.active_count());
        let storage = storage_health_input(&self.composition.storage_sink.progress_snapshot());
        let snapshot = if final_snapshot {
            self.health.final_snapshot(storage)
        } else {
            self.health.periodic_snapshot(storage)
        };
        let payload = serde_json::to_string(&snapshot)
            .expect("the fixed runtime-health schema must serialize");
        tracing::info!(
            schema_version = RUNTIME_HEALTH_SCHEMA_VERSION,
            final_snapshot,
            runtime_health = %payload,
            "runtime health snapshot"
        );
    }
}

impl RuntimeHealthState {
    fn connectivity_expected(&self, id: ConnectivityId) -> bool {
        self.connectivity[id.index()]
            .expected
            .load(Ordering::Relaxed)
    }
}

fn storage_health_input(snapshot: &reap_storage::StorageProgressSnapshot) -> StorageHealthInput {
    StorageHealthInput {
        queue_capacity: usize_to_u64(snapshot.queue_capacity),
        queue_depth: usize_to_u64(snapshot.queue_depth),
        queue_high_water: usize_to_u64(snapshot.queue_high_water),
        records_enqueued: snapshot.records_enqueued,
        records_written: snapshot.records_written,
        records_outstanding: usize_to_u64(snapshot.records_outstanding),
        durable_sync_completions: snapshot.durable_sync_completions,
        write_failures: snapshot.write_failures,
        sync_failures: snapshot.sync_failures,
        dropped_records: snapshot.dropped_records,
        last_writer_progress_age_ns: (snapshot.last_writer_progress_ns != 0)
            .then_some(snapshot.last_writer_progress_age_ns),
    }
}

fn usize_to_u64(value: usize) -> u64 {
    u64::try_from(value).unwrap_or(u64::MAX)
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
    expected_instances: u64,
    running_instances: u64,
    unobserved_instances: u64,
    progress_count: u64,
    oldest_progress_age_ms: Option<u64>,
}

/// Bounded observational queue fold.
///
/// Each lane is sampled from lock-free atomics, so a snapshot taken during a
/// concurrent handoff may conservatively combine adjacent lane states. Depth
/// and high-water remain capacity-bounded, saturation cannot be masked by
/// spare capacity in another lane, and a live backlog always has an age.
/// This telemetry is diagnostic, not a synchronization or control boundary.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
pub(super) struct QueueHealthSnapshot {
    id: QueueId,
    capacity: u64,
    depth: u64,
    high_water_mark: u64,
    continuous_backlog_age_ms: Option<u64>,
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
    local_rejected: u64,
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

#[cfg(test)]
pub(super) struct HealthEmissionCadence {
    next_due_ns: u64,
    armed: bool,
}

#[cfg(test)]
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

fn atomic_saturating_increment(value: &AtomicU64) -> u64 {
    let previous = value
        .fetch_update(Ordering::AcqRel, Ordering::Acquire, |current| {
            Some(current.saturating_add(1))
        })
        .unwrap_or_else(|current| current);
    previous.saturating_add(1)
}

fn atomic_saturating_sub(value: &AtomicU64, amount: u64) -> u64 {
    let previous = value
        .fetch_update(Ordering::AcqRel, Ordering::Acquire, |current| {
            Some(current.saturating_sub(amount))
        })
        .unwrap_or_else(|current| current);
    previous.saturating_sub(amount)
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
    use std::alloc::{GlobalAlloc, Layout, System};
    use std::cell::Cell;

    use reap_core::OrderIntent;
    use reap_storage::{OrderAckRecord, OrderRequestRecord, ReconciliationRecord};
    use serde_json::json;

    use super::*;

    const SECOND_NS: u64 = 1_000_000_000;

    struct ThreadCountingAllocator;

    thread_local! {
        static TRACK_ALLOCATIONS: Cell<bool> = const { Cell::new(false) };
        static ALLOCATION_CALLS: Cell<u64> = const { Cell::new(0) };
    }

    #[global_allocator]
    static TEST_ALLOCATOR: ThreadCountingAllocator = ThreadCountingAllocator;

    unsafe impl GlobalAlloc for ThreadCountingAllocator {
        unsafe fn alloc(&self, layout: Layout) -> *mut u8 {
            count_test_allocation();
            // SAFETY: allocation is delegated unchanged to the system allocator.
            unsafe { System.alloc(layout) }
        }

        unsafe fn alloc_zeroed(&self, layout: Layout) -> *mut u8 {
            count_test_allocation();
            // SAFETY: allocation is delegated unchanged to the system allocator.
            unsafe { System.alloc_zeroed(layout) }
        }

        unsafe fn dealloc(&self, ptr: *mut u8, layout: Layout) {
            // SAFETY: deallocation is delegated unchanged to the allocator that
            // produced the pointer.
            unsafe { System.dealloc(ptr, layout) }
        }

        unsafe fn realloc(&self, ptr: *mut u8, layout: Layout, new_size: usize) -> *mut u8 {
            count_test_allocation();
            // SAFETY: reallocation is delegated unchanged to the system allocator.
            unsafe { System.realloc(ptr, layout, new_size) }
        }
    }

    fn count_test_allocation() {
        let _ = TRACK_ALLOCATIONS.try_with(|tracking| {
            if tracking.get() {
                let _ = ALLOCATION_CALLS.try_with(|calls| {
                    calls.set(calls.get().saturating_add(1));
                });
            }
        });
    }

    fn start_allocation_count() {
        ALLOCATION_CALLS.with(|calls| calls.set(0));
        TRACK_ALLOCATIONS.with(|tracking| tracking.set(true));
    }

    fn stop_allocation_count() -> u64 {
        TRACK_ALLOCATIONS.with(|tracking| tracking.set(false));
        ALLOCATION_CALLS.with(Cell::get)
    }

    #[test]
    fn connectivity_and_heartbeat_transitions_have_monotonic_progress_ages() {
        let health = RuntimeHealthState::with_order_task_count(2);
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
                .all(|heartbeat| heartbeat.oldest_progress_age_ms.is_none())
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
        health.set_heartbeat_running(HeartbeatId::RuntimeEventLoop, 0, true);
        health.mark_heartbeat_at(HeartbeatId::RuntimeEventLoop, 3 * SECOND_NS);
        health.mark_heartbeat_at(HeartbeatId::RuntimeEventLoop, 4 * SECOND_NS);
        health.set_heartbeat_running(HeartbeatId::OrderTask, 0, true);
        health.set_heartbeat_running(HeartbeatId::OrderTask, 1, true);
        health.mark_heartbeat_lane_at(HeartbeatId::OrderTask, 0, 4 * SECOND_NS);
        health.mark_heartbeat_lane_at(HeartbeatId::OrderTask, 1, 3 * SECOND_NS);

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
                expected_instances: 1,
                running_instances: 1,
                unobserved_instances: 0,
                progress_count: 2,
                oldest_progress_age_ms: Some(1_500),
            }
        );
        assert_eq!(
            snapshot.heartbeats[HeartbeatId::OrderTask.index()],
            HeartbeatHealthSnapshot {
                id: HeartbeatId::OrderTask,
                expected_instances: 2,
                running_instances: 2,
                unobserved_instances: 0,
                progress_count: 2,
                oldest_progress_age_ms: Some(2_500),
            }
        );
    }

    #[test]
    fn connectivity_projection_preserves_expectation_and_terminal_failure() {
        let health = RuntimeHealthState::new();

        health.set_connectivity_expected(ConnectivityId::Private, false);
        assert_eq!(
            health.projected_connectivity_state(ConnectivityId::Private, true),
            ConnectivityHealthState::NotRequired
        );

        health.set_connectivity_expected(ConnectivityId::Private, true);
        assert_eq!(
            health.projected_connectivity_state(ConnectivityId::Private, false),
            ConnectivityHealthState::Connecting
        );
        assert_eq!(
            health.projected_connectivity_state(ConnectivityId::Private, true),
            ConnectivityHealthState::Ready
        );

        health.set_connectivity_state(
            ConnectivityId::Private,
            ConnectivityHealthState::Disconnected,
        );
        assert_eq!(
            health.projected_connectivity_state(ConnectivityId::Private, false),
            ConnectivityHealthState::Disconnected
        );

        health.set_connectivity_state(ConnectivityId::Private, ConnectivityHealthState::Failed);
        assert_eq!(
            health.projected_connectivity_state(ConnectivityId::Private, true),
            ConnectivityHealthState::Failed
        );
    }

    #[test]
    fn heartbeat_guard_reports_running_and_stopped_instances() {
        let health = Arc::new(RuntimeHealthState::with_order_task_count(1));
        {
            let _guard = HeartbeatGuard::start(Arc::clone(&health), HeartbeatId::OrderTask, 0);
            let running = health.snapshot_at(
                health.monotonic_now_ns(),
                StorageHealthInput::default(),
                false,
            );
            let order = running.heartbeats[HeartbeatId::OrderTask.index()];
            assert_eq!(order.expected_instances, 1);
            assert_eq!(order.running_instances, 1);
            assert_eq!(order.unobserved_instances, 0);
            assert_eq!(order.progress_count, 1);
            assert!(order.oldest_progress_age_ms.is_some());
        }
        let stopped = health.snapshot_at(
            health.monotonic_now_ns(),
            StorageHealthInput::default(),
            true,
        );
        assert_eq!(
            stopped.heartbeats[HeartbeatId::OrderTask.index()].running_instances,
            0
        );
    }

    #[test]
    fn queues_readiness_counters_and_storage_progress_are_distinct_and_bounded() {
        let health = RuntimeHealthState::with_order_task_count(1);
        health.configure_queue(QueueId::OrderCommand, 4);
        health.observe_queue_depth_at(QueueId::OrderCommand, 1, 100_000_000);
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
        health.increment_counter(HealthCounterId::LocalRejected, 7);
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
                continuous_backlog_age_ms: Some(900),
                last_residence_age_ms: Some(7),
                max_residence_age_ms: 7,
                saturated: false,
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
                local_rejected: 7,
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
    fn journaled_order_and_reconciliation_records_have_distinct_counter_semantics() {
        let health = RuntimeHealthState::new();
        let request = |operation| {
            StorageRecord::OrderRequest(OrderRequestRecord {
                ts_ms: 1,
                account_id: "main".to_string(),
                operation,
                idempotency_key: None,
                client_order_id: Some("client-1".to_string()),
                exchange_order_id: None,
                symbol: "BTC-USDT".to_string(),
            })
        };
        let ack = |operation, status| {
            StorageRecord::OrderAck(OrderAckRecord {
                ts_ms: 2,
                account_id: "main".to_string(),
                operation,
                client_order_id: "client-1".to_string(),
                exchange_order_id: None,
                status,
                message: String::new(),
            })
        };
        let reconciliation = |clean| {
            StorageRecord::Reconciliation(ReconciliationRecord {
                ts_ms: 3,
                account_id: "main".to_string(),
                clean,
                local_live_orders: 0,
                remote_live_orders: 0,
                remote_recent_fills: 0,
                reason: String::new(),
            })
        };
        for record in [
            request(OrderOperation::Submit),
            request(OrderOperation::Cancel),
            ack(OrderOperation::Submit, OrderAckStatus::Accepted),
            ack(OrderOperation::Cancel, OrderAckStatus::Duplicate),
            ack(OrderOperation::Submit, OrderAckStatus::Rejected),
            ack(OrderOperation::Submit, OrderAckStatus::Ambiguous),
            ack(
                OrderOperation::Cancel,
                OrderAckStatus::PendingReconciliation,
            ),
            StorageRecord::IntentRejected {
                ts_ms: 3,
                intent: OrderIntent::CancelOrder {
                    order_id: "client-1".to_string(),
                    reason: "fixture".to_string(),
                },
                reason: "risk rejected".to_string(),
            },
            reconciliation(true),
            reconciliation(false),
        ] {
            health.apply_counter_updates(HealthCounterUpdates::from_storage_record(&record));
        }
        let counters = health
            .snapshot_at(SECOND_NS, StorageHealthInput::default(), false)
            .orders;
        assert_eq!(counters.submit_requests, 1);
        assert_eq!(counters.cancel_requests, 1);
        assert_eq!(counters.submit_accepted_acks, 1);
        assert_eq!(counters.cancel_accepted_acks, 0);
        assert_eq!(counters.local_rejected, 1);
        assert_eq!(counters.exchange_rejected, 1);
        assert_eq!(counters.ambiguous, 2);
        assert_eq!(counters.reconciliation_total, 2);
        assert_eq!(counters.reconciliation_clean, 1);
    }

    #[test]
    fn storage_projection_distinguishes_no_writer_progress_from_a_zero_age_sample() {
        let base = reap_storage::StorageProgressSnapshot {
            records_enqueued: 1,
            records_written: 1,
            durable_sync_completions: 0,
            write_failures: 0,
            sync_failures: 0,
            dropped_records: 0,
            records_outstanding: 0,
            queue_capacity: 8,
            queue_depth: 0,
            queue_high_water: 1,
            last_writer_progress_ns: 0,
            last_writer_progress_age_ns: 0,
        };
        assert_eq!(
            storage_health_input(&base).last_writer_progress_age_ns,
            None
        );
        assert_eq!(
            storage_health_input(&reap_storage::StorageProgressSnapshot {
                last_writer_progress_ns: 1,
                ..base
            })
            .last_writer_progress_age_ns,
            Some(0)
        );
    }

    #[test]
    fn schema_one_serialization_is_exact_and_has_no_dynamic_identifiers() {
        let health = RuntimeHealthState::with_order_task_count(1);
        health.set_connectivity_at(
            ConnectivityId::Private,
            ConnectivityHealthState::Disconnected,
            SECOND_NS,
        );
        health.set_heartbeat_running(HeartbeatId::RuntimeEventLoop, 0, true);
        health.mark_heartbeat_at(HeartbeatId::RuntimeEventLoop, 2 * SECOND_NS);
        health.configure_queue(QueueId::FeedIngress, 16);
        health.observe_queue_depth_at(QueueId::FeedIngress, 4, SECOND_NS);
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
                    {
                        "id": "runtime_event_loop",
                        "expected_instances": 1,
                        "running_instances": 1,
                        "unobserved_instances": 0,
                        "progress_count": 1,
                        "oldest_progress_age_ms": 1000
                    },
                    {
                        "id": "order_task",
                        "expected_instances": 1,
                        "running_instances": 0,
                        "unobserved_instances": 1,
                        "progress_count": 0,
                        "oldest_progress_age_ms": null
                    }
                ],
                "queues": [
                    {
                        "id": "feed_ingress",
                        "capacity": 16,
                        "depth": 4,
                        "high_water_mark": 4,
                        "continuous_backlog_age_ms": 2000,
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
                        "continuous_backlog_age_ms": null,
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
                        "continuous_backlog_age_ms": null,
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
                    "local_rejected": 0,
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

    #[tokio::test]
    async fn tracked_queue_lanes_report_exact_aggregate_depth_and_any_lane_saturation() {
        let health = Arc::new(RuntimeHealthState::with_order_task_count(2));
        let (first_tx, mut first_rx) =
            tracked_channel(Arc::clone(&health), QueueId::OrderCommand, 2);
        let (second_tx, mut second_rx) =
            tracked_channel(Arc::clone(&health), QueueId::OrderCommand, 1);

        first_tx.try_send(1_u8).unwrap();
        first_tx.try_send(2_u8).unwrap();
        second_tx.try_send(3_u8).unwrap();
        assert!(matches!(
            second_tx.try_send(4_u8),
            Err(mpsc::error::TrySendError::Full(4))
        ));

        let full = health.snapshot_at(SECOND_NS, StorageHealthInput::default(), false);
        let queue = full.queues[QueueId::OrderCommand.index()];
        assert_eq!(queue.capacity, 3);
        assert_eq!(queue.depth, 3);
        assert_eq!(queue.high_water_mark, 3);
        assert!(queue.saturated);
        assert_eq!(queue.saturation_events, 3);

        assert_eq!(first_rx.recv().await, Some(1));
        let one_lane_still_full =
            health.snapshot_at(SECOND_NS, StorageHealthInput::default(), false);
        let queue = one_lane_still_full.queues[QueueId::OrderCommand.index()];
        assert_eq!(queue.depth, 2);
        assert!(
            queue.saturated,
            "one full account lane must not be masked by spare aggregate capacity"
        );

        assert_eq!(second_rx.recv().await, Some(3));
        let no_lane_full = health.snapshot_at(SECOND_NS, StorageHealthInput::default(), false);
        let queue = no_lane_full.queues[QueueId::OrderCommand.index()];
        assert_eq!(queue.depth, 1);
        assert!(!queue.saturated);
        assert!(queue.last_residence_age_ms.is_some());

        drop(first_rx);
        let drained_on_drop = health.snapshot_at(SECOND_NS, StorageHealthInput::default(), false);
        assert_eq!(
            drained_on_drop.queues[QueueId::OrderCommand.index()].depth,
            0
        );
    }

    #[tokio::test]
    async fn tracked_async_send_waits_for_capacity_without_publishing_phantom_depth() {
        let health = Arc::new(RuntimeHealthState::new());
        let (sender, mut receiver) =
            tracked_channel(Arc::clone(&health), QueueId::ControlIngress, 1);
        sender.send(1_u8).await.unwrap();

        let waiting = tokio::spawn({
            let sender = sender.clone();
            async move { sender.send(2_u8).await }
        });
        tokio::task::yield_now().await;
        let while_waiting = health.snapshot_at(SECOND_NS, StorageHealthInput::default(), false);
        assert_eq!(
            while_waiting.queues[QueueId::ControlIngress.index()].depth,
            1,
            "a producer waiting for capacity is not resident in the bounded queue"
        );

        assert_eq!(receiver.recv().await, Some(1));
        waiting.await.unwrap().unwrap();
        assert_eq!(receiver.recv().await, Some(2));
        let drained = health.snapshot_at(SECOND_NS, StorageHealthInput::default(), false);
        assert_eq!(drained.queues[QueueId::ControlIngress.index()].depth, 0);
        assert!(
            drained.queues[QueueId::ControlIngress.index()].saturation_events >= 2,
            "filling the lane and a blocked producer are both saturation evidence"
        );
    }

    #[tokio::test]
    async fn tracked_receiver_drop_discards_queued_values_and_wakes_blocked_sender() {
        let health = Arc::new(RuntimeHealthState::new());
        let (sender, receiver) = tracked_channel(Arc::clone(&health), QueueId::ControlIngress, 1);
        sender.send(1_u8).await.unwrap();
        let waiting = tokio::spawn({
            let sender = sender.clone();
            async move { sender.send(2_u8).await }
        });
        tokio::task::yield_now().await;

        drop(receiver);

        let error = waiting.await.unwrap().unwrap_err();
        assert_eq!(error.0, 2);
        let discarded = health.snapshot_at(SECOND_NS, StorageHealthInput::default(), false);
        let queue = discarded.queues[QueueId::ControlIngress.index()];
        assert_eq!(queue.depth, 0);
        assert_eq!(queue.high_water_mark, 1);
        assert!(!queue.saturated);
    }

    #[tokio::test]
    async fn tracked_try_recv_updates_depth_saturation_and_residence() {
        let health = Arc::new(RuntimeHealthState::new());
        let (sender, mut receiver) =
            tracked_channel(Arc::clone(&health), QueueId::ControlIngress, 1);
        sender.try_send(7_u8).unwrap();

        let full = health.snapshot_at(SECOND_NS, StorageHealthInput::default(), false);
        let queue = full.queues[QueueId::ControlIngress.index()];
        assert_eq!(queue.depth, 1);
        assert!(queue.saturated);

        assert_eq!(receiver.try_recv().unwrap(), 7);
        let drained = health.snapshot_at(SECOND_NS, StorageHealthInput::default(), false);
        let queue = drained.queues[QueueId::ControlIngress.index()];
        assert_eq!(queue.depth, 0);
        assert!(!queue.saturated);
        assert!(queue.last_residence_age_ms.is_some());
    }

    #[test]
    fn stale_dequeue_cleanup_cannot_erase_a_new_full_lane_backlog() {
        let health = Arc::new(RuntimeHealthState::new());
        let lane = QueueLane::new(Arc::clone(&health), QueueId::FeedIngress, 1);
        let identical_origin_ns = 37;
        lane.enqueue_at(identical_origin_ns);
        let first_token = lane.state().backlog_token.load(Ordering::Acquire);

        // Deterministically model Tokio releasing the old item before the
        // producer fills the capacity-1 lane, while the old receiver has not
        // yet completed its telemetry cleanup. Force both generations to use
        // the same timestamp to cover the timestamp-ABA case.
        let stale_removal = lane.begin_remove();
        assert_eq!(stale_removal.lane_depth, 0);
        lane.enqueue_at(identical_origin_ns);
        let replacement_token = lane.state().backlog_token.load(Ordering::Acquire);
        assert_ne!(replacement_token, first_token);
        lane.finish_remove(stale_removal);
        assert_eq!(
            lane.state().backlog_token.load(Ordering::Acquire),
            replacement_token
        );

        let handoff = health.snapshot_at(SECOND_NS, StorageHealthInput::default(), false);
        let queue = handoff.queues[QueueId::FeedIngress.index()];
        assert_eq!(queue.capacity, 1);
        assert_eq!(queue.depth, 1);
        assert!(queue.saturated);
        assert!(
            queue.continuous_backlog_age_ms.is_some(),
            "a stale dequeue must not clear the newer backlog generation"
        );

        lane.discard();
    }

    #[tokio::test(flavor = "multi_thread", worker_threads = 2)]
    async fn tracked_queue_reporting_stays_bounded_during_concurrent_handoffs() {
        const PRODUCERS: usize = 4;
        const PER_PRODUCER: usize = 250;

        let health = Arc::new(RuntimeHealthState::new());
        let (sender, mut receiver) = tracked_channel(Arc::clone(&health), QueueId::FeedIngress, 4);
        let producers = (0..PRODUCERS)
            .map(|producer| {
                let sender = sender.clone();
                tokio::spawn(async move {
                    for sequence in 0..PER_PRODUCER {
                        sender.send((producer, sequence)).await.unwrap();
                    }
                })
            })
            .collect::<Vec<_>>();
        drop(sender);

        for _ in 0..PRODUCERS * PER_PRODUCER {
            receiver.recv().await.unwrap();
            let snapshot = health.snapshot_at(SECOND_NS, StorageHealthInput::default(), false);
            let queue = snapshot.queues[QueueId::FeedIngress.index()];
            assert!(queue.depth <= queue.capacity);
            assert!(queue.high_water_mark <= queue.capacity);
        }
        for producer in producers {
            producer.await.unwrap();
        }
        assert_eq!(
            health
                .snapshot_at(SECOND_NS, StorageHealthInput::default(), false)
                .queues[QueueId::FeedIngress.index()]
            .depth,
            0
        );
    }

    #[test]
    fn progress_and_queue_instrumentation_updates_allocate_zero_times() {
        let health = Arc::new(RuntimeHealthState::new());
        let lane = QueueLane::new(Arc::clone(&health), QueueId::FeedIngress, 1);
        start_allocation_count();
        for _ in 0..10_000 {
            health.mark_connectivity_progress(ConnectivityId::Feed);
            health.set_connectivity_state(ConnectivityId::Feed, ConnectivityHealthState::Ready);
            health.mark_heartbeat(HeartbeatId::RuntimeEventLoop);
            health.set_readiness(LivePhase::Ready, 1);
            health.increment_counter(HealthCounterId::SubmitRequests, 1);
            let queued = Queued::new(1_u8, Arc::clone(&lane));
            assert_eq!(queued.consume(), 1);
        }
        let allocations = stop_allocation_count();
        assert_eq!(
            allocations, 0,
            "fixed-ID progress and queue instrumentation must allocate zero times"
        );
    }

    #[test]
    fn progress_clock_ignores_out_of_order_observations_and_cadence_overflow_disarms() {
        let clock = ProgressClock::new();
        clock.observe_at(10);
        clock.observe_at(5);
        assert_eq!(clock.age_ms_at(10 + NS_PER_MS), Some(1));

        let mut cadence =
            HealthEmissionCadence::starting_at(u64::MAX - HEALTH_EMISSION_INTERVAL_NS + 1);
        assert!(!cadence.take_due(u64::MAX));
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
            "Mutex", "RwLock", "HashMap", "BTreeMap", "String", "Vec<", "format!",
        ] {
            assert!(
                !production_source.contains(forbidden),
                "production health core must not contain `{forbidden}`"
            );
        }
        assert_eq!(
            production_source.matches("Box<").count(),
            4,
            "boxed health state is limited to startup-predeclared heartbeat and queue lane slices"
        );
        assert!(production_source.contains("lanes: Box<[HeartbeatLane]>"));
        assert!(production_source.contains("lanes: Box<[QueueLaneState]>"));
        assert_eq!(
            production_source.matches(".collect::<Box<[_]>>()").count(),
            2
        );
    }
}
