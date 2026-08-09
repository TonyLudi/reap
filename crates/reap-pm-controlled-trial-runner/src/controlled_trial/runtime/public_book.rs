//! Runner-private live public-book ownership for the controlled trial.
//!
//! This module deliberately keeps the canonical Polymarket session and book
//! reducer behind one short synchronous lock. The long-lived WebSocket sink
//! and the coordinator handle are distinct so freshness/lease checks can run
//! while the socket task remains active. No lock guard crosses an `.await`.

use std::{
    fmt,
    sync::{Arc, Mutex},
};

use async_trait::async_trait;
use reap_pm_core::{
    ConnectionEpoch, IngressSequence, PmBookTop, PmBookUpdate, PmConnectionId, PmMarketMetadata,
    PmProductSource, PmPublicObservationGrant, ReceivedEventClock, SnapshotRevision,
};
use reap_pm_state::{
    PmBookBatchEvidence, PmBookFreshness, PmBookReducer, PmBookReducerAuthorityId,
    PmBookTransition, PmDomainFingerprint, PmExternalBookFault, PmMetadataContract,
    PmMetadataContractError, PmMetadataDrift, PmMetadataFingerprint, PmMetadataObservation,
    PmPublicReadinessReason, PmSnapshotCommitProof,
};
use reap_polymarket_adapter::{
    PM_PUBLIC_PONG_BYTES, PmPublicBookDelivery, PmPublicHeartbeatAction, PmPublicHeartbeatConfig,
    PmPublicHeartbeatEvidence, PmPublicReconnectTransition, PmPublicRole, PmPublicRoleError,
    PmPublicSession, PmPublicSessionBatch, PmPublicSessionError, PmPublicSessionFault,
    PmSnapshotFlowToken,
};
use reap_polymarket_live_adapter::{
    PmActorProductClock, PmCancelMutationTimeOwner, PmHttpReceiveClock,
    PmLiveAuthoritativeMetadataError, PmLiveAuthoritativeMetadataObservation,
    PmLiveMetadataObservationCommitment, PmOkxProductClock, PmPlaceMutationTimeOwner,
    PmPrivateReadProductClock, PmProductClockError, PmPublicConnectivityRoles, PmPublicHttpRole,
    PmPublicMarketWsRole, PmPublicMetadataHttpRole, PmPublicWsActivityView, PmPublicWsConnection,
    PmPublicWsDisconnectReason, PmPublicWsEvent, PmPublicWsEventSink, PmPublicWsReconnect,
    PmPublicWsReconnectDirective, PmPublicWsRetirement, PmPublicWsRunError,
    PmPublicWsShutdownSignal, PmPublicWsTransportPolicy, PmReadServerTimeHttpRole,
    PmRestBookDeliveryError, PmRestBookPurpose, PmRestBookSnapshotSink, PmRestResponseClock,
    PmUserWsProductClock,
};
use reap_polymarket_wire::PmBookMarketBinding;
use reap_transport::ReconnectPolicy;
use thiserror::Error;

const MAX_PUBLIC_BOOK_AGE_NS: u64 = 5_000_000_000;

/// Sole runner-private split of the live adapter's move-only public
/// connectivity roles. Construction consumes the exact source bundle, so the
/// metadata, REST book, public WS, actor clock, and retained companion views
/// cannot be assembled from different clock owners.
pub(super) struct PmControlledTrialPublicConnectivity {
    book: PmPublicBookConnectivityBundle,
    companions: PmPublicBookCompanionRoles,
}

impl PmControlledTrialPublicConnectivity {
    pub(super) fn from_roles(roles: PmPublicConnectivityRoles) -> Self {
        let (
            metadata_http,
            book_http,
            read_server_time_http,
            private_read_clock,
            place_mutation_time,
            cancel_mutation_time,
            public_ws,
            user_ws_clock,
            actor_clock,
            okx_clock,
        ) = roles.into_roles();
        Self {
            book: PmPublicBookConnectivityBundle {
                metadata_http,
                book_http,
                public_ws,
                actor_clock,
            },
            companions: PmPublicBookCompanionRoles {
                read_server_time_http,
                private_read_clock,
                place_mutation_time,
                cancel_mutation_time,
                user_ws_clock,
                okx_clock,
            },
        }
    }

    #[must_use]
    pub(super) fn into_parts(self) -> (PmPublicBookConnectivityBundle, PmPublicBookCompanionRoles) {
        (self.book, self.companions)
    }
}

impl fmt::Debug for PmControlledTrialPublicConnectivity {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str("PmControlledTrialPublicConnectivity(<one-product-owner>)")
    }
}

/// Exact public-book roles split from one `PmPublicConnectivityRoles` owner.
/// There is deliberately no constructor accepting independent roles.
pub(super) struct PmPublicBookConnectivityBundle {
    metadata_http: PmPublicMetadataHttpRole,
    book_http: PmPublicHttpRole,
    public_ws: PmPublicMarketWsRole,
    actor_clock: PmActorProductClock,
}

impl PmPublicBookConnectivityBundle {
    pub(super) async fn start(
        self,
        config: PmPublicBookRuntimeConfig,
    ) -> Result<PmStartedPublicBookRuntime, PmPublicBookRuntimeError> {
        let Self {
            metadata_http,
            book_http,
            public_ws,
            actor_clock,
        } = self;
        let runtime =
            PmPublicBookRuntime::start_bound(&metadata_http, &public_ws, actor_clock, config)
                .await?;
        Ok(PmStartedPublicBookRuntime {
            runtime,
            book_http,
            public_ws,
        })
    }
}

impl fmt::Debug for PmPublicBookConnectivityBundle {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str("PmPublicBookConnectivityBundle(<exact-shared-origin-roles>)")
    }
}

/// Non-book views preserved by the exact connectivity split for private reads
/// and the later mutation owner. This carrier exposes no cloning or alternate
/// construction path.
pub(super) struct PmPublicBookCompanionRoles {
    read_server_time_http: PmReadServerTimeHttpRole,
    private_read_clock: PmPrivateReadProductClock,
    place_mutation_time: PmPlaceMutationTimeOwner,
    cancel_mutation_time: PmCancelMutationTimeOwner,
    user_ws_clock: PmUserWsProductClock,
    okx_clock: PmOkxProductClock,
}

impl PmPublicBookCompanionRoles {
    /// Preserve the one-owner clock provenance when dividing read evidence
    /// from later mutation-time evidence. Neither returned carrier has a
    /// production constructor accepting independent clock roles.
    #[must_use]
    pub(super) fn into_runtime_roles(self) -> (PmPrivateReadClockBundle, PmMutationCompanionRoles) {
        (
            PmPrivateReadClockBundle {
                server_time: self.read_server_time_http,
                private_read: self.private_read_clock,
                user_ws: self.user_ws_clock,
            },
            PmMutationCompanionRoles {
                place_mutation_time: self.place_mutation_time,
                cancel_mutation_time: self.cancel_mutation_time,
                okx_clock: self.okx_clock,
            },
        )
    }
}

impl fmt::Debug for PmPublicBookCompanionRoles {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str("PmPublicBookCompanionRoles(<retained-shared-origin-views>)")
    }
}

/// Exact private-read clock views issued by one public connectivity owner.
/// Production code obtains this value only from `into_runtime_roles`.
pub(super) struct PmPrivateReadClockBundle {
    server_time: PmReadServerTimeHttpRole,
    private_read: PmPrivateReadProductClock,
    user_ws: PmUserWsProductClock,
}

impl PmPrivateReadClockBundle {
    pub(super) fn into_parts(
        self,
    ) -> (
        PmReadServerTimeHttpRole,
        PmPrivateReadProductClock,
        PmUserWsProductClock,
    ) {
        (self.server_time, self.private_read, self.user_ws)
    }

    #[cfg(test)]
    pub(super) const fn test_support_from_roles(
        server_time: PmReadServerTimeHttpRole,
        private_read: PmPrivateReadProductClock,
        user_ws: PmUserWsProductClock,
    ) -> Self {
        Self {
            server_time,
            private_read,
            user_ws,
        }
    }
}

impl fmt::Debug for PmPrivateReadClockBundle {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str("PmPrivateReadClockBundle(<one product-clock owner>)")
    }
}

/// Remaining exact mutation-time views. This carrier still has no request,
/// signer, credential, journal grant, or transport capability.
pub(super) struct PmMutationCompanionRoles {
    place_mutation_time: PmPlaceMutationTimeOwner,
    cancel_mutation_time: PmCancelMutationTimeOwner,
    okx_clock: PmOkxProductClock,
}

impl fmt::Debug for PmMutationCompanionRoles {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str("PmMutationCompanionRoles(<same-product-clock views>)")
    }
}

/// Started exact-role bundle. The REST and WS roles stay paired with the
/// runtime that admitted metadata from their common connectivity owner.
pub(super) struct PmStartedPublicBookRuntime {
    runtime: PmPublicBookRuntime,
    book_http: PmPublicHttpRole,
    public_ws: PmPublicMarketWsRole,
}

impl PmStartedPublicBookRuntime {
    #[must_use]
    pub(super) fn into_parts(self) -> (PmPublicBookWsTask, PmPublicBookCoordinator) {
        let (sink, handle) = self.runtime.into_parts();
        (
            PmPublicBookWsTask {
                role: self.public_ws,
                sink,
            },
            PmPublicBookCoordinator {
                book_http: self.book_http,
                handle,
            },
        )
    }
}

impl fmt::Debug for PmStartedPublicBookRuntime {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str("PmStartedPublicBookRuntime(<exact-role-bound>)")
    }
}

/// Cancellation-safe owner of the exact public WebSocket role and the only
/// sink joined to its book runtime. The pair is never returned separately, so
/// cancellation necessarily drops the sink and terminalizes every old lease.
pub(super) struct PmPublicBookWsTask {
    role: PmPublicMarketWsRole,
    sink: PmPublicBookRuntimeSink,
}

impl PmPublicBookWsTask {
    pub(super) async fn run_to_exit(
        self,
        shutdown: PmPublicWsShutdownSignal,
    ) -> Result<(), PmPublicBookWsTaskError> {
        let Self { role, mut sink } = self;
        let run_result = role.run(shutdown, &mut sink).await;
        sink.invalidate_after_task_exit()
            .map_err(PmPublicBookWsTaskError::ExitInvalidation)?;
        run_result.map_err(PmPublicBookWsTaskError::Run)
    }
}

impl fmt::Debug for PmPublicBookWsTask {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str("PmPublicBookWsTask(<exact-role-and-armed-sink>)")
    }
}

/// Exact public REST role and reducer handle from the same connectivity
/// owner. Orchestration receives only this coordinator, never either raw half.
pub(super) struct PmPublicBookCoordinator {
    book_http: PmPublicHttpRole,
    handle: PmPublicBookRuntimeHandle,
}

impl PmPublicBookCoordinator {
    pub(super) async fn seed_book(
        &mut self,
    ) -> Result<PmPublicBookSnapshotEvidence, PmRestBookDeliveryError<PmPublicBookRuntimeError>>
    {
        self.book_http.seed_book(&mut self.handle).await
    }

    pub(super) async fn resync_book(
        &mut self,
    ) -> Result<PmPublicBookSnapshotEvidence, PmRestBookDeliveryError<PmPublicBookRuntimeError>>
    {
        self.book_http.resync_book(&mut self.handle).await
    }

    pub(super) fn take_snapshot_evidence(
        &mut self,
    ) -> Result<Option<PmPublicBookSnapshotEvidence>, PmPublicBookRuntimeError> {
        self.handle.take_snapshot_evidence()
    }

    pub(super) fn lease(&mut self) -> Result<PmPublicBookLease, PmPublicBookRuntimeError> {
        self.handle.lease()
    }

    pub(super) fn recheck_lease(
        &mut self,
        lease: PmPublicBookLease,
    ) -> Result<PmPublicBookLease, PmPublicBookRuntimeError> {
        self.handle.recheck_lease(lease)
    }
}

impl fmt::Debug for PmPublicBookCoordinator {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str("PmPublicBookCoordinator(<exact-http-and-reducer>)")
    }
}

/// Cold runner-only configuration. It contains no transport or mutation
/// authority and is consumed exactly once by
/// [`PmPublicBookConnectivityBundle::start`].
pub(super) struct PmPublicBookRuntimeConfig {
    observation_grant: PmPublicObservationGrant,
    source: PmProductSource,
    connection: PmConnectionId,
    expected_metadata: PmMarketMetadata,
    metadata_revision: SnapshotRevision,
    last_snapshot_revision: Option<SnapshotRevision>,
    freshness: PmBookFreshness,
    reconnect: ReconnectPolicy,
}

impl PmPublicBookRuntimeConfig {
    #[allow(clippy::too_many_arguments)]
    pub(super) fn new(
        observation_grant: PmPublicObservationGrant,
        source: PmProductSource,
        connection: PmConnectionId,
        expected_metadata: PmMarketMetadata,
        metadata_revision: SnapshotRevision,
        last_snapshot_revision: Option<SnapshotRevision>,
        freshness: PmBookFreshness,
        reconnect: ReconnectPolicy,
    ) -> Self {
        Self {
            observation_grant,
            source,
            connection,
            expected_metadata,
            metadata_revision,
            last_snapshot_revision,
            freshness,
            reconnect,
        }
    }
}

impl fmt::Debug for PmPublicBookRuntimeConfig {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str("PmPublicBookRuntimeConfig(<exact-public-scope>)")
    }
}

/// Move-only owner produced after one authoritative metadata fetch.
///
/// The split is intentional: `sink` remains borrowed by the WebSocket run
/// while `handle` continues to serve REST resync and lease commands.
pub(super) struct PmPublicBookRuntime {
    sink: PmPublicBookRuntimeSink,
    handle: PmPublicBookRuntimeHandle,
}

impl PmPublicBookRuntime {
    /// Private because independent role arguments do not themselves prove a
    /// shared product-clock origin. Production startup enters only through
    /// `PmPublicBookConnectivityBundle::start`, whose constructor consumes the
    /// live adapter's exact move-only role bundle.
    async fn start_bound(
        metadata_http: &PmPublicMetadataHttpRole,
        public_ws: &PmPublicMarketWsRole,
        mut actor_clock: PmActorProductClock,
        config: PmPublicBookRuntimeConfig,
    ) -> Result<Self, PmPublicBookRuntimeError> {
        let scope = metadata_http.configured_scope();
        if public_ws.scope() != scope
            || config.expected_metadata.condition() != scope.condition()
            || config.expected_metadata.market() != scope.market()
            || config.expected_metadata.outcome().token() != scope.token()
        {
            return Err(PmPublicBookRuntimeError::Configuration(
                "metadata, book WebSocket, and expected market must share one exact scope",
            ));
        }
        let transport_policy = public_ws.transport_policy();
        validate_reconnect_policy(&config.reconnect, transport_policy)?;
        validate_freshness(config.freshness)?;

        let activity = RuntimeActivityView::Live(public_ws.activity_view());
        if activity.generation() != 0 {
            return Err(PmPublicBookRuntimeError::ActivityAlreadyStarted);
        }
        // The pre-fetch edge is the conservative freshness origin. Sampling
        // only after this await would relabel request and parse delay as fresh.
        let metadata_control_begin = actor_clock.observe_control_edge()?.received_clock();

        // This is the sole production metadata admission path. The live
        // observation and authoritative metadata are joined from one pair of
        // response bodies inside the source role.
        let joined = metadata_http
            .refresh_authoritative_observation(
                config.observation_grant.instrument(),
                config.source,
                config.expected_metadata,
                config.metadata_revision,
            )
            .await?;
        let metadata_control_complete = actor_clock.observe_control_edge()?.received_clock();
        if metadata_control_complete.monotonic_receive_ns()
            < metadata_control_begin.monotonic_receive_ns()
        {
            return Err(PmPublicBookRuntimeError::ControlClockRegression);
        }
        if activity.generation() != 0 {
            return Err(PmPublicBookRuntimeError::ActivityAlreadyStarted);
        }
        let state = PmPublicBookState::from_joined(
            joined,
            metadata_control_begin,
            metadata_control_complete,
            config,
            transport_policy,
        )?;
        if activity.generation() != 0 {
            return Err(PmPublicBookRuntimeError::ActivityAlreadyStarted);
        }

        Ok(Self::from_state(
            state,
            activity,
            RuntimeControlClock::Live(actor_clock),
        ))
    }

    fn from_state(
        state: PmPublicBookState,
        activity: RuntimeActivityView,
        clock: RuntimeControlClock,
    ) -> Self {
        let shared = Arc::new(Mutex::new(state));
        Self {
            sink: PmPublicBookRuntimeSink {
                shared: Arc::clone(&shared),
                activity: activity.clone(),
            },
            handle: PmPublicBookRuntimeHandle {
                shared,
                activity,
                clock,
            },
        }
    }

    #[must_use]
    fn into_parts(self) -> (PmPublicBookRuntimeSink, PmPublicBookRuntimeHandle) {
        (self.sink, self.handle)
    }
}

impl fmt::Debug for PmPublicBookRuntime {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str("PmPublicBookRuntime(<move-only>)")
    }
}

/// Long-lived transport sink. It exposes no state or lease surface.
struct PmPublicBookRuntimeSink {
    shared: Arc<Mutex<PmPublicBookState>>,
    activity: RuntimeActivityView,
}

impl fmt::Debug for PmPublicBookRuntimeSink {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str("PmPublicBookRuntimeSink(<serialized>)")
    }
}

/// Coordinator-side runtime handle. It is intentionally non-cloneable.
struct PmPublicBookRuntimeHandle {
    shared: Arc<Mutex<PmPublicBookState>>,
    activity: RuntimeActivityView,
    clock: RuntimeControlClock,
}

enum RuntimeControlClock {
    Live(PmActorProductClock),
    #[cfg(test)]
    Test(std::collections::VecDeque<ReceivedEventClock>),
}

impl RuntimeControlClock {
    fn observe(&mut self) -> Result<ReceivedEventClock, PmPublicBookRuntimeError> {
        match self {
            Self::Live(clock) => Ok(clock.observe_control_edge()?.received_clock()),
            #[cfg(test)]
            Self::Test(clock) => clock
                .pop_front()
                .ok_or(PmPublicBookRuntimeError::TestClockExhausted),
        }
    }
}

impl fmt::Debug for PmPublicBookRuntimeHandle {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str("PmPublicBookRuntimeHandle(<serialized>)")
    }
}

#[derive(Clone)]
enum RuntimeActivityView {
    Live(PmPublicWsActivityView),
    #[cfg(test)]
    Test(Arc<std::sync::atomic::AtomicU64>),
}

impl RuntimeActivityView {
    fn generation(&self) -> u64 {
        match self {
            Self::Live(view) => view.generation(),
            #[cfg(test)]
            Self::Test(value) => value.load(std::sync::atomic::Ordering::Acquire),
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(super) enum PmPublicBookSnapshotSource {
    WebSocket,
    RestSeed,
    RestResync,
}

/// Move-only proof of an exact snapshot commit and correlated flow opening.
pub(super) struct PmPublicBookSnapshotEvidence {
    proof: PmSnapshotCommitProof,
    source: PmPublicBookSnapshotSource,
    top: PmBookTop,
    state_generation: u64,
    admitted_activity_generation: u64,
    source_high_water: u64,
    metadata_source_receive_clock: PmHttpReceiveClock,
    metadata_observation_commitment: PmLiveMetadataObservationCommitment,
    metadata_control_begin_clock: ReceivedEventClock,
    metadata_control_complete_clock: ReceivedEventClock,
    book_receive_clock: ReceivedEventClock,
}

impl PmPublicBookSnapshotEvidence {
    #[must_use]
    pub(super) const fn source(&self) -> PmPublicBookSnapshotSource {
        self.source
    }

    #[must_use]
    pub(super) const fn connection_epoch(&self) -> ConnectionEpoch {
        self.proof.connection_epoch()
    }

    #[must_use]
    pub(super) const fn metadata_revision(&self) -> SnapshotRevision {
        self.proof.metadata_revision()
    }

    #[must_use]
    pub(super) const fn snapshot_revision(&self) -> SnapshotRevision {
        self.proof.snapshot_revision()
    }

    #[must_use]
    pub(super) const fn local_ingress_sequence(&self) -> IngressSequence {
        self.proof.local_ingress_sequence()
    }

    #[must_use]
    pub(super) const fn ready_top(&self) -> PmBookTop {
        self.top
    }

    #[must_use]
    pub(super) const fn state_generation(&self) -> u64 {
        self.state_generation
    }

    #[must_use]
    pub(super) const fn admitted_activity_generation(&self) -> u64 {
        self.admitted_activity_generation
    }

    #[must_use]
    pub(super) const fn source_high_water(&self) -> u64 {
        self.source_high_water
    }

    #[must_use]
    pub(super) const fn metadata_source_receive_clock(&self) -> PmHttpReceiveClock {
        self.metadata_source_receive_clock
    }

    #[must_use]
    pub(super) const fn metadata_observation_commitment(
        &self,
    ) -> PmLiveMetadataObservationCommitment {
        self.metadata_observation_commitment
    }

    #[must_use]
    pub(super) const fn metadata_control_begin_clock(&self) -> ReceivedEventClock {
        self.metadata_control_begin_clock
    }

    #[must_use]
    pub(super) const fn metadata_control_complete_clock(&self) -> ReceivedEventClock {
        self.metadata_control_complete_clock
    }

    #[must_use]
    pub(super) const fn book_receive_clock(&self) -> ReceivedEventClock {
        self.book_receive_clock
    }

    #[must_use]
    pub(super) const fn source_was_fully_admitted(&self) -> bool {
        self.admitted_activity_generation == self.source_high_water
    }
}

impl fmt::Debug for PmPublicBookSnapshotEvidence {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("PmPublicBookSnapshotEvidence")
            .field("source", &self.source)
            .field("epoch", &self.connection_epoch())
            .field("metadata_revision", &self.metadata_revision())
            .field("snapshot_revision", &self.snapshot_revision())
            .field("ingress", &self.local_ingress_sequence())
            .field("state_generation", &self.state_generation)
            .field(
                "admitted_activity_generation",
                &self.admitted_activity_generation,
            )
            .field("source_high_water", &self.source_high_water)
            .finish_non_exhaustive()
    }
}

struct CorrelatedHeartbeat {
    evidence: PmPublicHeartbeatEvidence,
    activity_generation: u64,
}

/// Move-only, process-local permit input for one exact public top.
///
/// Rechecking consumes this value and returns a newly sealed successor. No
/// checked lease can be replayed after a state or activity transition.
pub(super) struct PmPublicBookLease {
    reducer_authority: PmBookReducerAuthorityId,
    state_generation: u64,
    connection_epoch: ConnectionEpoch,
    metadata_revision: SnapshotRevision,
    snapshot_revision: SnapshotRevision,
    local_ingress_sequence: IngressSequence,
    top: PmBookTop,
    heartbeat: PmPublicHeartbeatEvidence,
    heartbeat_activity_generation: u64,
    activity_generation: u64,
    source_high_water: u64,
    metadata_source_receive_clock: PmHttpReceiveClock,
    metadata_observation_commitment: PmLiveMetadataObservationCommitment,
    metadata_control_begin_clock: ReceivedEventClock,
    metadata_control_complete_clock: ReceivedEventClock,
    book_receive_clock: ReceivedEventClock,
    checked_at_control_clock: ReceivedEventClock,
    fresh_until_monotonic_ns: u64,
}

impl PmPublicBookLease {
    #[must_use]
    pub(super) const fn state_generation(&self) -> u64 {
        self.state_generation
    }

    #[must_use]
    pub(super) const fn connection_epoch(&self) -> ConnectionEpoch {
        self.connection_epoch
    }

    #[must_use]
    pub(super) const fn metadata_revision(&self) -> SnapshotRevision {
        self.metadata_revision
    }

    #[must_use]
    pub(super) const fn snapshot_revision(&self) -> SnapshotRevision {
        self.snapshot_revision
    }

    #[must_use]
    pub(super) const fn local_ingress_sequence(&self) -> IngressSequence {
        self.local_ingress_sequence
    }

    #[must_use]
    pub(super) const fn ready_top(&self) -> PmBookTop {
        self.top
    }

    #[must_use]
    pub(super) const fn heartbeat(&self) -> PmPublicHeartbeatEvidence {
        self.heartbeat
    }

    #[must_use]
    pub(super) const fn heartbeat_activity_generation(&self) -> u64 {
        self.heartbeat_activity_generation
    }

    #[must_use]
    pub(super) const fn activity_generation(&self) -> u64 {
        self.activity_generation
    }

    #[must_use]
    pub(super) const fn admitted_activity_generation(&self) -> u64 {
        self.activity_generation
    }

    #[must_use]
    pub(super) const fn source_high_water(&self) -> u64 {
        self.source_high_water
    }

    #[must_use]
    pub(super) const fn source_was_fully_admitted(&self) -> bool {
        self.activity_generation == self.source_high_water
    }

    #[must_use]
    pub(super) const fn metadata_source_receive_clock(&self) -> PmHttpReceiveClock {
        self.metadata_source_receive_clock
    }

    #[must_use]
    pub(super) const fn metadata_observation_commitment(
        &self,
    ) -> PmLiveMetadataObservationCommitment {
        self.metadata_observation_commitment
    }

    #[must_use]
    pub(super) const fn metadata_control_begin_clock(&self) -> ReceivedEventClock {
        self.metadata_control_begin_clock
    }

    #[must_use]
    pub(super) const fn metadata_control_complete_clock(&self) -> ReceivedEventClock {
        self.metadata_control_complete_clock
    }

    #[must_use]
    pub(super) const fn book_receive_clock(&self) -> ReceivedEventClock {
        self.book_receive_clock
    }

    #[must_use]
    pub(super) const fn checked_at_control_clock(&self) -> ReceivedEventClock {
        self.checked_at_control_clock
    }

    #[must_use]
    pub(super) const fn fresh_until_monotonic_ns(&self) -> u64 {
        self.fresh_until_monotonic_ns
    }
}

impl fmt::Debug for PmPublicBookLease {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("PmPublicBookLease")
            .field("epoch", &self.connection_epoch)
            .field("metadata_revision", &self.metadata_revision)
            .field("snapshot_revision", &self.snapshot_revision)
            .field("ingress", &self.local_ingress_sequence)
            .field("state_generation", &self.state_generation)
            .field("activity_generation", &self.activity_generation)
            .field("source_high_water", &self.source_high_water)
            .field(
                "checked_at_monotonic_ns",
                &self.checked_at_control_clock.monotonic_receive_ns(),
            )
            .field("fresh_until_monotonic_ns", &self.fresh_until_monotonic_ns)
            .finish_non_exhaustive()
    }
}

struct PmPublicBookState {
    session: PmPublicSession,
    reducer: PmBookReducer,
    freshness: PmBookFreshness,
    metadata_source_receive_clock: PmHttpReceiveClock,
    metadata_observation_commitment: PmLiveMetadataObservationCommitment,
    metadata_control_begin_clock: ReceivedEventClock,
    metadata_control_complete_clock: ReceivedEventClock,
    metadata_control_receive_ns: u64,
    book_receive_clock: Option<ReceivedEventClock>,
    admitted_activity_generation: u64,
    state_generation: u64,
    last_heartbeat: Option<CorrelatedHeartbeat>,
    pending_snapshot: Option<PmPublicBookSnapshotEvidence>,
    retired: Option<RetiredAttempt>,
    authorized_reconnect: Option<PmPublicReconnectTransition>,
    connection_open: bool,
    unavailable_consumed_for_epoch: bool,
    max_reconnect_attempts: u8,
    max_reconnect_backoff: std::time::Duration,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
struct RetiredAttempt {
    connection: PmPublicWsConnection,
    clock: reap_polymarket_live_adapter::PmPublicWsEdgeClock,
    reason: PmPublicWsDisconnectReason,
}

impl RetiredAttempt {
    const fn from_retirement(retired: PmPublicWsRetirement) -> Self {
        Self {
            connection: retired.connection(),
            clock: retired.clock(),
            reason: retired.reason(),
        }
    }
}

struct SnapshotCore {
    proof: PmSnapshotCommitProof,
    source: PmPublicBookSnapshotSource,
    top: PmBookTop,
    received_clock: ReceivedEventClock,
}

impl PmPublicBookState {
    fn from_joined(
        joined: PmLiveAuthoritativeMetadataObservation,
        metadata_control_begin: ReceivedEventClock,
        metadata_control_complete: ReceivedEventClock,
        config: PmPublicBookRuntimeConfig,
        transport_policy: PmPublicWsTransportPolicy,
    ) -> Result<Self, PmPublicBookRuntimeError> {
        let (live, authoritative) = joined.into_parts();
        let parser = authoritative.parser_config();
        if parser.market_binding() != PmBookMarketBinding::ConditionId {
            return Err(PmPublicBookRuntimeError::Configuration(
                "live public book parser is not condition-bound",
            ));
        }
        if authoritative.monotonic_receive_ns() != live.receive_clock().monotonic_receive_ns() {
            return Err(PmPublicBookRuntimeError::MetadataProvenanceMismatch);
        }

        let role = PmPublicRole::new(
            config.observation_grant,
            config.observation_grant.instrument(),
            parser,
            config.source,
            config.connection,
        )?;
        let metadata_fingerprint =
            PmMetadataFingerprint::new(authoritative.metadata_fingerprint())?;
        let domain_fingerprint = PmDomainFingerprint::new(authoritative.domain_fingerprint())?;
        let metadata_contract =
            PmMetadataContract::goal_f_clob_v2(config.expected_metadata, domain_fingerprint);
        let metadata_event = authoritative.event();
        let metadata_source_receive_clock = live.receive_clock();
        let metadata_observation_commitment = live.commitment();
        if metadata_control_complete.monotonic_receive_ns()
            < metadata_control_begin.monotonic_receive_ns()
        {
            return Err(PmPublicBookRuntimeError::ControlClockRegression);
        }
        let metadata_control_receive_ns = metadata_control_begin.monotonic_receive_ns();
        let metadata_observation = PmMetadataObservation::new(
            metadata_event.instrument(),
            metadata_event.metadata_revision(),
            metadata_fingerprint,
            metadata_contract,
            metadata_control_receive_ns,
        )?;

        let mut reducer = PmBookReducer::new(
            metadata_event.instrument(),
            metadata_fingerprint,
            metadata_contract,
            config.freshness,
        )
        .map_err(PmPublicBookRuntimeError::Reducer)?;
        match reducer
            .apply_metadata(metadata_observation)
            .map_err(PmPublicBookRuntimeError::Reducer)?
        {
            PmBookTransition::MetadataAccepted { revision }
                if revision == metadata_event.metadata_revision() => {}
            _ => return Err(PmPublicBookRuntimeError::ReducerTransitionMismatch),
        }
        let initial_epoch = transport_policy.initial_connection_epoch();
        match reducer
            .begin_epoch(initial_epoch)
            .map_err(PmPublicBookRuntimeError::Reducer)?
        {
            PmBookTransition::EpochStarted { epoch } if epoch == initial_epoch => {}
            _ => return Err(PmPublicBookRuntimeError::ReducerTransitionMismatch),
        }

        let heartbeat = heartbeat_config(transport_policy)?;
        let mut session = PmPublicSession::new(
            role,
            authoritative,
            initial_epoch,
            config.last_snapshot_revision,
            config.reconnect,
            heartbeat,
        )?;
        let metadata_occurrence =
            session.issue_metadata_occurrence(live.receive_clock().local_wall_receive_ns())?;
        if metadata_occurrence.source() != role.source()
            || metadata_occurrence.connection_id() != role.connection()
            || metadata_occurrence.ordering().connection_epoch() != initial_epoch
            || metadata_occurrence.ordering().snapshot_revision().is_some()
            || metadata_occurrence
                .ordering()
                .local_ingress_sequence()
                .value()
                != 1
            || metadata_occurrence.received_clock().monotonic_receive_ns()
                != metadata_source_receive_clock.monotonic_receive_ns()
            || session.local_ingress_sequence()
                != metadata_occurrence.ordering().local_ingress_sequence()
            || reducer.connection_epoch() != Some(initial_epoch)
            || reducer.readiness().reason() != Some(PmPublicReadinessReason::SnapshotMissing)
        {
            return Err(PmPublicBookRuntimeError::MetadataProvenanceMismatch);
        }

        Ok(Self {
            session,
            reducer,
            freshness: config.freshness,
            metadata_source_receive_clock,
            metadata_observation_commitment,
            metadata_control_begin_clock: metadata_control_begin,
            metadata_control_complete_clock: metadata_control_complete,
            metadata_control_receive_ns,
            book_receive_clock: None,
            admitted_activity_generation: 0,
            state_generation: 1,
            last_heartbeat: None,
            pending_snapshot: None,
            retired: None,
            authorized_reconnect: None,
            connection_open: false,
            unavailable_consumed_for_epoch: false,
            max_reconnect_attempts: transport_policy.max_reconnect_attempts(),
            max_reconnect_backoff: transport_policy.max_reconnect_backoff(),
        })
    }

    fn validate_next_activity(
        &self,
        generation: u64,
        source_high_water: u64,
    ) -> Result<(), PmPublicBookRuntimeError> {
        let expected = self
            .admitted_activity_generation
            .checked_add(1)
            .ok_or(PmPublicBookRuntimeError::ActivityGenerationOverflow)?;
        if generation != expected || source_high_water < generation {
            return Err(PmPublicBookRuntimeError::ActivityDiscontinuity {
                expected,
                observed: generation,
                source_high_water,
            });
        }
        Ok(())
    }

    fn finish_activity(&mut self, generation: u64) -> Result<(), PmPublicBookRuntimeError> {
        self.admitted_activity_generation = generation;
        self.bump_state_generation()
    }

    fn bump_state_generation(&mut self) -> Result<(), PmPublicBookRuntimeError> {
        self.state_generation = self
            .state_generation
            .checked_add(1)
            .ok_or(PmPublicBookRuntimeError::StateGenerationOverflow)?;
        Ok(())
    }

    fn validate_connection(
        &self,
        connection: PmPublicWsConnection,
    ) -> Result<(), PmPublicBookRuntimeError> {
        if connection.scope() != self.session.role().wire_scope()
            || connection.connection_epoch() != self.session.connection_epoch()
        {
            return Err(PmPublicBookRuntimeError::ConnectionMismatch);
        }
        Ok(())
    }

    fn admit_ws_event(
        &mut self,
        event: PmPublicWsEvent,
        source_high_water: u64,
    ) -> Result<(), PmPublicBookRuntimeError> {
        if self.pending_snapshot.is_some() {
            return Err(PmPublicBookRuntimeError::SnapshotEvidencePending);
        }
        let generation = event.activity_generation();
        self.validate_next_activity(generation, source_high_water)?;
        let edge = event_clock(&event);
        let result = match event {
            PmPublicWsEvent::ConnectionOpened(observation) => {
                self.admit_connection_opened(observation.connection())
            }
            PmPublicWsEvent::SubscriptionSent(observation) => {
                self.admit_subscription_sent(observation.connection(), observation.clock())
            }
            PmPublicWsEvent::PingSent(observation) => {
                self.admit_ping_sent(observation.connection(), observation.clock())
            }
            PmPublicWsEvent::Pong(observation) => {
                self.admit_pong(observation.connection(), observation.clock(), generation)
            }
            PmPublicWsEvent::RawData(raw) => {
                self.admit_raw(raw.connection(), raw.clock(), raw.bytes(), generation)
            }
            PmPublicWsEvent::ConnectionRetired(retired) => self.admit_retirement(retired),
            PmPublicWsEvent::ReconnectScheduled(reconnect) => {
                self.admit_reconnect_scheduled(reconnect)
            }
            PmPublicWsEvent::ReconnectStopped(retired) => self.admit_reconnect_stopped(retired),
            PmPublicWsEvent::Shutdown(observation) => {
                self.admit_shutdown(observation.connection(), observation.clock())
            }
        };

        match result {
            Ok(snapshot) => {
                self.finish_activity(generation)?;
                if let Some(snapshot) = snapshot {
                    if self.pending_snapshot.is_some() {
                        self.fail_closed(
                            edge,
                            PmPublicSessionFault::InvalidTransition,
                            PmExternalBookFault::InvalidTransition,
                        );
                        return Err(PmPublicBookRuntimeError::SnapshotEvidencePending);
                    }
                    self.pending_snapshot = Some(self.seal_snapshot(snapshot, source_high_water));
                }
                Ok(())
            }
            Err(error) => {
                let _ = self.finish_activity(generation);
                self.fail_closed(
                    edge,
                    PmPublicSessionFault::InvalidTransition,
                    PmExternalBookFault::InvalidTransition,
                );
                Err(error)
            }
        }
    }

    fn admit_connection_opened(
        &mut self,
        connection: PmPublicWsConnection,
    ) -> Result<Option<SnapshotCore>, PmPublicBookRuntimeError> {
        self.validate_connection(connection)?;
        if self.session.requires_reconnect()
            || self.session.subscription_sent()
            || self.connection_open
        {
            return Err(PmPublicBookRuntimeError::LifecycleMismatch);
        }
        self.connection_open = true;
        Ok(None)
    }

    fn admit_subscription_sent(
        &mut self,
        connection: PmPublicWsConnection,
        clock: reap_polymarket_live_adapter::PmPublicWsEdgeClock,
    ) -> Result<Option<SnapshotCore>, PmPublicBookRuntimeError> {
        self.validate_connection(connection)?;
        if !self.connection_open || self.session.subscription_sent() {
            return Err(PmPublicBookRuntimeError::LifecycleMismatch);
        }
        self.session
            .mark_subscription_sent(clock.monotonic_receive_ns())?;
        Ok(None)
    }

    fn admit_ping_sent(
        &mut self,
        connection: PmPublicWsConnection,
        clock: reap_polymarket_live_adapter::PmPublicWsEdgeClock,
    ) -> Result<Option<SnapshotCore>, PmPublicBookRuntimeError> {
        self.validate_connection(connection)?;
        if !self.connection_open || !self.session.subscription_sent() {
            return Err(PmPublicBookRuntimeError::LifecycleMismatch);
        }
        if self.session.poll_heartbeat(clock.monotonic_receive_ns())?
            != PmPublicHeartbeatAction::SendPing
        {
            return Err(PmPublicBookRuntimeError::HeartbeatMismatch);
        }
        // An outstanding PING retires the prior PONG. Only the subsequent
        // session-correlated PONG may restore leaseable heartbeat evidence.
        self.last_heartbeat = None;
        Ok(None)
    }

    fn admit_pong(
        &mut self,
        connection: PmPublicWsConnection,
        clock: reap_polymarket_live_adapter::PmPublicWsEdgeClock,
        activity_generation: u64,
    ) -> Result<Option<SnapshotCore>, PmPublicBookRuntimeError> {
        self.validate_connection(connection)?;
        if !self.connection_open || !self.session.subscription_sent() {
            return Err(PmPublicBookRuntimeError::LifecycleMismatch);
        }
        let batch = self.session.classify(
            PM_PUBLIC_PONG_BYTES,
            clock.local_wall_receive_ns(),
            clock.monotonic_receive_ns(),
        )?;
        let heartbeat = batch
            .heartbeat()
            .ok_or(PmPublicBookRuntimeError::HeartbeatMismatch)?;
        if !batch.events().is_empty()
            || !batch.ignored().is_empty()
            || batch.snapshot_flow_token().is_some()
            || heartbeat.connection_epoch() != self.session.connection_epoch()
            || heartbeat.local_wall_receive_ns() != clock.local_wall_receive_ns()
            || heartbeat.monotonic_receive_ns() != clock.monotonic_receive_ns()
        {
            return Err(PmPublicBookRuntimeError::HeartbeatMismatch);
        }
        self.last_heartbeat = Some(CorrelatedHeartbeat {
            evidence: heartbeat,
            activity_generation,
        });
        Ok(None)
    }

    fn admit_raw(
        &mut self,
        connection: PmPublicWsConnection,
        clock: reap_polymarket_live_adapter::PmPublicWsEdgeClock,
        raw: &[u8],
        activity_generation: u64,
    ) -> Result<Option<SnapshotCore>, PmPublicBookRuntimeError> {
        self.validate_connection(connection)?;
        if !self.connection_open || !self.session.subscription_sent() {
            return Err(PmPublicBookRuntimeError::LifecycleMismatch);
        }
        let batch = self.session.classify(
            raw,
            clock.local_wall_receive_ns(),
            clock.monotonic_receive_ns(),
        )?;
        self.reduce_batch(
            batch,
            PmPublicBookSnapshotSource::WebSocket,
            activity_generation,
        )
    }

    fn admit_retirement(
        &mut self,
        retired: PmPublicWsRetirement,
    ) -> Result<Option<SnapshotCore>, PmPublicBookRuntimeError> {
        self.validate_connection(retired.connection())?;
        if self.retired.is_some() {
            return Err(PmPublicBookRuntimeError::LifecycleMismatch);
        }
        if !self.session.requires_reconnect() {
            let (session_fault, reducer_fault) = retirement_fault(retired.reason());
            self.invalidate_pair(retired.clock(), session_fault, reducer_fault);
        }
        if !self.session.requires_reconnect() {
            return Err(PmPublicBookRuntimeError::LifecycleMismatch);
        }
        self.consume_unavailable(self.session.last_fault())?;
        self.retired = Some(RetiredAttempt::from_retirement(retired));
        self.connection_open = false;
        self.last_heartbeat = None;
        self.book_receive_clock = None;
        self.pending_snapshot = None;
        Ok(None)
    }

    fn admit_reconnect_scheduled(
        &mut self,
        reconnect: PmPublicWsReconnect,
    ) -> Result<Option<SnapshotCore>, PmPublicBookRuntimeError> {
        let expected = self
            .authorized_reconnect
            .ok_or(PmPublicBookRuntimeError::LifecycleMismatch)?;
        if RetiredAttempt::from_retirement(reconnect.retired())
            != self
                .retired
                .ok_or(PmPublicBookRuntimeError::LifecycleMismatch)?
            || reconnect.replacement_epoch() != expected.replacement_epoch()
            || reconnect.reconnect_attempt() != expected.reconnect_attempt()
            || reconnect.backoff() != expected.delay()
            || self.session.connection_epoch() != expected.replacement_epoch()
            || reconnect.scheduled_clock().monotonic_receive_ns()
                < reconnect.retired().clock().monotonic_receive_ns()
        {
            return Err(PmPublicBookRuntimeError::ReconnectMismatch);
        }
        self.authorized_reconnect = None;
        self.retired = None;
        Ok(None)
    }

    fn admit_reconnect_stopped(
        &mut self,
        retired: PmPublicWsRetirement,
    ) -> Result<Option<SnapshotCore>, PmPublicBookRuntimeError> {
        if !self.session.requires_reconnect()
            || RetiredAttempt::from_retirement(retired)
                != self
                    .retired
                    .ok_or(PmPublicBookRuntimeError::LifecycleMismatch)?
        {
            return Err(PmPublicBookRuntimeError::ReconnectMismatch);
        }
        self.authorized_reconnect = None;
        Ok(None)
    }

    fn admit_shutdown(
        &mut self,
        connection: PmPublicWsConnection,
        clock: reap_polymarket_live_adapter::PmPublicWsEdgeClock,
    ) -> Result<Option<SnapshotCore>, PmPublicBookRuntimeError> {
        // During authorized reconnect backoff the transport still owns the
        // retired socket-attempt identity while the canonical session has
        // already advanced to its replacement epoch. A future epoch or a
        // different scope is never accepted.
        if connection.scope() != self.session.role().wire_scope()
            || connection.connection_epoch().value() > self.session.connection_epoch().value()
        {
            return Err(PmPublicBookRuntimeError::ConnectionMismatch);
        }
        if !self.session.requires_reconnect() {
            self.invalidate_pair(
                clock,
                PmPublicSessionFault::Disconnect,
                PmExternalBookFault::Disconnect,
            );
        }
        self.consume_unavailable(self.session.last_fault())?;
        self.connection_open = false;
        self.last_heartbeat = None;
        self.book_receive_clock = None;
        self.pending_snapshot = None;
        Ok(None)
    }

    fn reduce_batch(
        &mut self,
        batch: PmPublicSessionBatch,
        snapshot_source: PmPublicBookSnapshotSource,
        activity_generation: u64,
    ) -> Result<Option<SnapshotCore>, PmPublicBookRuntimeError> {
        if let Some(heartbeat) = batch.heartbeat() {
            if !batch.events().is_empty()
                || !batch.ignored().is_empty()
                || batch.snapshot_flow_token().is_some()
                || heartbeat.connection_epoch() != self.session.connection_epoch()
            {
                return Err(PmPublicBookRuntimeError::HeartbeatMismatch);
            }
            self.last_heartbeat = Some(CorrelatedHeartbeat {
                evidence: heartbeat,
                activity_generation,
            });
            return Ok(None);
        }

        let token = batch.snapshot_flow_token();
        let events = batch.into_events();
        if let Some(token) = token {
            if events.len() != 1 || self.pending_snapshot.is_some() {
                return Err(PmPublicBookRuntimeError::SnapshotEvidencePending);
            }
            return self
                .commit_snapshot(&events[0], token, snapshot_source)
                .map(Some);
        }
        let mut terminal_tick = false;
        for delivery in &events {
            terminal_tick |= self.reduce_flow_delivery(delivery)?;
        }
        if terminal_tick && events.len() != 1 {
            return Err(PmPublicBookRuntimeError::DeliveryShapeMismatch);
        }
        Ok(None)
    }

    fn commit_snapshot(
        &mut self,
        delivery: &PmPublicBookDelivery,
        token: PmSnapshotFlowToken,
        source: PmPublicBookSnapshotSource,
    ) -> Result<SnapshotCore, PmPublicBookRuntimeError> {
        self.validate_delivery(delivery, false)?;
        let ordering = delivery.ordering();
        let payload = delivery.payload();
        let PmBookUpdate::Snapshot(snapshot) = payload.update() else {
            return Err(PmPublicBookRuntimeError::DeliveryShapeMismatch);
        };
        let revision = ordering
            .snapshot_revision()
            .ok_or(PmPublicBookRuntimeError::DeliveryShapeMismatch)?;
        if token.connection_epoch() != ordering.connection_epoch()
            || token.snapshot_revision() != revision
            || token.local_ingress_sequence() != ordering.local_ingress_sequence()
            || Some(token.venue_hash()) != ordering.venue_hash()
            || self.session.current_snapshot_revision() != Some(revision)
            || self.session.protocol_flow_open()
        {
            return Err(PmPublicBookRuntimeError::SnapshotProofMismatch);
        }
        let evidence = self.batch_evidence(delivery)?;
        let transition = self
            .reducer
            .apply_snapshot(evidence, snapshot)
            .map_err(PmPublicBookRuntimeError::Reducer)?;
        let PmBookTransition::SnapshotCommitted {
            revision: committed_revision,
            levels,
            proof,
        } = transition
        else {
            return Err(PmPublicBookRuntimeError::ReducerTransitionMismatch);
        };
        if committed_revision != revision
            || usize::from(levels) != snapshot.levels().len()
            || proof.instrument() != self.session.role().instrument()
            || proof.metadata_fingerprint() != self.reducer.expected_metadata_fingerprint()
            || proof.connection_epoch() != token.connection_epoch()
            || proof.metadata_revision() != self.session.metadata_revision()
            || proof.snapshot_revision() != token.snapshot_revision()
            || proof.local_ingress_sequence() != token.local_ingress_sequence()
            || proof.venue_hash() != token.venue_hash()
            || self.reducer.connection_epoch() != Some(self.session.connection_epoch())
            || self.reducer.last_ingress_sequence() != Some(token.local_ingress_sequence())
            || self.reducer.last_verified_snapshot_hash() != Some(token.venue_hash())
        {
            return Err(PmPublicBookRuntimeError::SnapshotProofMismatch);
        }
        let top = self
            .reducer
            .ready_top()
            .ok_or(PmPublicBookRuntimeError::ReadyTopUnavailable)?;
        if let Err(error) = self.session.open_protocol_flow_after_snapshot(token) {
            let _ = self.reducer.apply_external_fault(
                self.session.connection_epoch(),
                PmExternalBookFault::InvalidTransition,
            );
            return Err(PmPublicBookRuntimeError::Session(error));
        }
        if !self.session.protocol_flow_open()
            || self.reducer.readiness().snapshot_revision() != Some(revision)
        {
            return Err(PmPublicBookRuntimeError::SnapshotProofMismatch);
        }
        self.book_receive_clock = Some(delivery.received_clock());
        Ok(SnapshotCore {
            proof,
            source,
            top,
            received_clock: delivery.received_clock(),
        })
    }

    /// Returns `true` only for the terminal tick-drift delivery.
    fn reduce_flow_delivery(
        &mut self,
        delivery: &PmPublicBookDelivery,
    ) -> Result<bool, PmPublicBookRuntimeError> {
        let is_tick = delivery.is_terminal_tick_size_change();
        self.validate_delivery(delivery, is_tick)?;
        let update = delivery.payload().update();
        if matches!(update, PmBookUpdate::Snapshot(_)) {
            return Err(PmPublicBookRuntimeError::DeliveryShapeMismatch);
        }
        let evidence = self.batch_evidence(delivery)?;
        if let PmBookUpdate::TickSizeChanged { old, new } = update {
            if !is_tick
                || !self.session.requires_reconnect()
                || self.session.last_fault() != Some(PmPublicSessionFault::TickSizeChanged)
            {
                return Err(PmPublicBookRuntimeError::DeliveryShapeMismatch);
            }
            let reason = match self.reducer.tick_size_changed(evidence, *old, *new) {
                Err(reason) => reason,
                Ok(_) => return Err(PmPublicBookRuntimeError::ReducerTransitionMismatch),
            };
            if reason != PmPublicReadinessReason::MetadataDrift(PmMetadataDrift::Grid) {
                return Err(PmPublicBookRuntimeError::Reducer(reason));
            }
            self.consume_unavailable(Some(PmPublicSessionFault::TickSizeChanged))?;
            self.last_heartbeat = None;
            self.book_receive_clock = None;
            self.pending_snapshot = None;
            return Ok(true);
        }
        if is_tick || !self.session.protocol_flow_open() || self.session.requires_reconnect() {
            return Err(PmPublicBookRuntimeError::LifecycleMismatch);
        }
        let prior_snapshot_hash = self.reducer.last_verified_snapshot_hash();
        let transition = self
            .reducer
            .apply_update(evidence, update)
            .map_err(PmPublicBookRuntimeError::Reducer)?;
        match (update, transition) {
            (
                PmBookUpdate::DeltaBatch(batch),
                PmBookTransition::DeltaBatchCommitted { revision, changes },
            ) if Some(revision) == delivery.ordering().snapshot_revision()
                && usize::from(changes) == batch.changes().len() =>
            {
                self.book_receive_clock = Some(delivery.received_clock());
            }
            (PmBookUpdate::TopCheck(_), PmBookTransition::TopConfirmed) => {}
            _ => return Err(PmPublicBookRuntimeError::ReducerTransitionMismatch),
        }
        if self.reducer.last_verified_snapshot_hash() != prior_snapshot_hash
            || self.reducer.last_ingress_sequence()
                != Some(delivery.ordering().local_ingress_sequence())
            || self.reducer.ready_top().is_none()
        {
            return Err(PmPublicBookRuntimeError::ReducerTransitionMismatch);
        }
        Ok(false)
    }

    fn validate_delivery(
        &self,
        delivery: &PmPublicBookDelivery,
        terminal_tick: bool,
    ) -> Result<(), PmPublicBookRuntimeError> {
        let ordering = delivery.ordering();
        let payload = delivery.payload();
        // The canonical session emits the terminal tick delivery, then
        // invalidates its attempt and clears `current_snapshot_revision` in
        // the same classify transition. Bind that delivery to the reducer's
        // still-current pre-invalidation snapshot instead; all nonterminal
        // deliveries continue to bind directly to the session snapshot.
        let snapshot_identity_matches = if terminal_tick {
            self.session.requires_reconnect()
                && self.session.last_fault() == Some(PmPublicSessionFault::TickSizeChanged)
                && self.session.current_snapshot_revision().is_none()
                && self.reducer.readiness().snapshot_revision() == ordering.snapshot_revision()
        } else {
            self.session.current_snapshot_revision() == ordering.snapshot_revision()
        };
        if delivery.source() != self.session.role().source()
            || delivery.connection_id() != self.session.role().connection()
            || delivery.attempt_connection_epoch() != self.session.connection_epoch()
            || ordering.connection_epoch() != self.session.connection_epoch()
            || payload.instrument() != self.session.role().instrument()
            || payload.metadata_revision() != self.session.metadata_revision()
            || ordering.snapshot_revision().is_none()
            || !snapshot_identity_matches
            || self.reducer.instrument() != self.session.role().instrument()
            || self.reducer.expected_metadata_fingerprint()
                != PmMetadataFingerprint::new(self.session.metadata_fingerprint())?
            || self.reducer.connection_epoch() != Some(self.session.connection_epoch())
        {
            return Err(PmPublicBookRuntimeError::DeliveryIdentityMismatch);
        }
        if terminal_tick {
            if !delivery.is_terminal_tick_size_change() {
                return Err(PmPublicBookRuntimeError::DeliveryShapeMismatch);
            }
        } else if delivery.is_terminal_tick_size_change() {
            return Err(PmPublicBookRuntimeError::DeliveryShapeMismatch);
        }
        Ok(())
    }

    fn batch_evidence(
        &self,
        delivery: &PmPublicBookDelivery,
    ) -> Result<PmBookBatchEvidence, PmPublicBookRuntimeError> {
        let ordering = delivery.ordering();
        PmBookBatchEvidence::new(
            self.session.role().instrument(),
            ordering.connection_epoch(),
            self.session.metadata_revision(),
            ordering
                .snapshot_revision()
                .ok_or(PmPublicBookRuntimeError::DeliveryShapeMismatch)?,
            ordering.local_ingress_sequence(),
            delivery.received_clock().monotonic_receive_ns(),
            ordering.venue_hash(),
        )
        .map_err(PmPublicBookRuntimeError::Reducer)
    }

    fn seal_snapshot(
        &self,
        core: SnapshotCore,
        source_high_water: u64,
    ) -> PmPublicBookSnapshotEvidence {
        PmPublicBookSnapshotEvidence {
            proof: core.proof,
            source: core.source,
            top: core.top,
            state_generation: self.state_generation,
            admitted_activity_generation: self.admitted_activity_generation,
            source_high_water,
            metadata_source_receive_clock: self.metadata_source_receive_clock,
            metadata_observation_commitment: self.metadata_observation_commitment,
            metadata_control_begin_clock: self.metadata_control_begin_clock,
            metadata_control_complete_clock: self.metadata_control_complete_clock,
            book_receive_clock: core.received_clock,
        }
    }

    fn ingest_rest_snapshot(
        &mut self,
        purpose: PmRestBookPurpose,
        received: PmRestResponseClock,
        raw: &[u8],
        source_high_water: u64,
    ) -> Result<PmPublicBookSnapshotEvidence, PmPublicBookRuntimeError> {
        if self.pending_snapshot.is_some() {
            return Err(PmPublicBookRuntimeError::SnapshotEvidencePending);
        }
        let source = match purpose {
            PmRestBookPurpose::Seed => PmPublicBookSnapshotSource::RestSeed,
            PmRestBookPurpose::Resync => PmPublicBookSnapshotSource::RestResync,
        };
        let result = self
            .session
            .classify_rest_book_snapshot(
                raw,
                received.local_wall_receive_ns(),
                received.monotonic_receive_ns(),
            )
            .map_err(PmPublicBookRuntimeError::Session)
            .and_then(|batch| self.reduce_batch(batch, source, self.admitted_activity_generation))
            .and_then(|snapshot| snapshot.ok_or(PmPublicBookRuntimeError::DeliveryShapeMismatch));
        match result {
            Ok(snapshot) => {
                self.bump_state_generation()?;
                Ok(self.seal_snapshot(snapshot, source_high_water))
            }
            Err(error) => {
                self.fail_closed_rest(
                    received,
                    PmPublicSessionFault::InvalidTransition,
                    PmExternalBookFault::InvalidTransition,
                );
                Err(error)
            }
        }
    }

    fn authorize_reconnect(
        &mut self,
        retired: PmPublicWsRetirement,
        source_high_water: u64,
    ) -> Result<PmPublicWsReconnectDirective, PmPublicBookRuntimeError> {
        let generation = retired.activity_generation();
        self.validate_next_activity(generation, source_high_water)?;
        if RetiredAttempt::from_retirement(retired)
            != self
                .retired
                .ok_or(PmPublicBookRuntimeError::LifecycleMismatch)?
            || !self.session.requires_reconnect()
            || self.authorized_reconnect.is_some()
        {
            return Err(PmPublicBookRuntimeError::ReconnectMismatch);
        }
        self.consume_unavailable(self.session.last_fault())?;
        let preview = self.session.preview_after_failure_transition()?;
        if preview.reconnect_attempt() > self.max_reconnect_attempts
            || preview.delay().is_zero()
            || preview.delay() > self.max_reconnect_backoff
        {
            self.finish_activity(generation)?;
            return Ok(PmPublicWsReconnectDirective::stop());
        }
        let transition = self.session.after_failure_transition()?;
        if transition != preview {
            return Err(PmPublicBookRuntimeError::ReconnectMismatch);
        }
        match self
            .reducer
            .begin_epoch(transition.replacement_epoch())
            .map_err(PmPublicBookRuntimeError::Reducer)?
        {
            PmBookTransition::EpochStarted { epoch } if epoch == transition.replacement_epoch() => {
            }
            _ => return Err(PmPublicBookRuntimeError::ReducerTransitionMismatch),
        }
        self.authorized_reconnect = Some(transition);
        self.connection_open = false;
        self.unavailable_consumed_for_epoch = false;
        self.last_heartbeat = None;
        self.book_receive_clock = None;
        self.pending_snapshot = None;
        self.finish_activity(generation)?;
        Ok(PmPublicWsReconnectDirective::reconnect(
            transition.retired_epoch(),
            transition.replacement_epoch(),
            transition.reconnect_attempt(),
            transition.delay(),
        ))
    }

    fn take_snapshot_evidence(&mut self) -> Option<PmPublicBookSnapshotEvidence> {
        self.pending_snapshot.take()
    }

    fn issue_lease(
        &mut self,
        now: ReceivedEventClock,
        source_high_water: u64,
    ) -> Result<PmPublicBookLease, PmPublicBookRuntimeError> {
        let now_monotonic_ns = now.monotonic_receive_ns();
        if self.pending_snapshot.is_some() {
            return Err(PmPublicBookRuntimeError::SnapshotEvidencePending);
        }
        if source_high_water != self.admitted_activity_generation {
            return Err(PmPublicBookRuntimeError::ActivityQueued {
                admitted: self.admitted_activity_generation,
                source_high_water,
            });
        }
        if self.session.requires_reconnect() || !self.session.protocol_flow_open() {
            return Err(PmPublicBookRuntimeError::BookUnavailable);
        }
        match self.reducer.check_freshness(now_monotonic_ns) {
            Ok(PmBookTransition::FreshnessConfirmed) => {}
            Ok(_) => return Err(PmPublicBookRuntimeError::ReducerTransitionMismatch),
            Err(reason) => return Err(PmPublicBookRuntimeError::Reducer(reason)),
        }
        let epoch = self.session.connection_epoch();
        let metadata_revision = self.session.metadata_revision();
        let snapshot_revision = self
            .session
            .current_snapshot_revision()
            .ok_or(PmPublicBookRuntimeError::BookUnavailable)?;
        let ingress = self.session.local_ingress_sequence();
        let top = self
            .reducer
            .ready_top()
            .ok_or(PmPublicBookRuntimeError::ReadyTopUnavailable)?;
        let heartbeat = self
            .last_heartbeat
            .as_ref()
            .ok_or(PmPublicBookRuntimeError::HeartbeatMissing)?;
        if heartbeat.evidence.connection_epoch() != epoch
            || heartbeat.activity_generation > self.admitted_activity_generation
            || heartbeat.evidence.monotonic_receive_ns() > now_monotonic_ns
            || self.reducer.connection_epoch() != Some(epoch)
            || self.reducer.readiness().metadata_revision() != Some(metadata_revision)
            || self.reducer.readiness().snapshot_revision() != Some(snapshot_revision)
            || self.reducer.last_ingress_sequence() != Some(ingress)
        {
            return Err(PmPublicBookRuntimeError::LeaseStateMismatch);
        }
        let fresh_until = self.freshness_deadline()?;
        if now_monotonic_ns > fresh_until {
            return Err(PmPublicBookRuntimeError::FreshnessDeadlineExpired);
        }
        Ok(PmPublicBookLease {
            reducer_authority: self.reducer.authority_id(),
            state_generation: self.state_generation,
            connection_epoch: epoch,
            metadata_revision,
            snapshot_revision,
            local_ingress_sequence: ingress,
            top,
            heartbeat: heartbeat.evidence,
            heartbeat_activity_generation: heartbeat.activity_generation,
            activity_generation: self.admitted_activity_generation,
            source_high_water,
            metadata_source_receive_clock: self.metadata_source_receive_clock,
            metadata_observation_commitment: self.metadata_observation_commitment,
            metadata_control_begin_clock: self.metadata_control_begin_clock,
            metadata_control_complete_clock: self.metadata_control_complete_clock,
            book_receive_clock: self
                .book_receive_clock
                .ok_or(PmPublicBookRuntimeError::BookUnavailable)?,
            checked_at_control_clock: now,
            fresh_until_monotonic_ns: fresh_until,
        })
    }

    fn recheck_lease(
        &mut self,
        lease: PmPublicBookLease,
        now: ReceivedEventClock,
        source_high_water: u64,
    ) -> Result<PmPublicBookLease, PmPublicBookRuntimeError> {
        let now_monotonic_ns = now.monotonic_receive_ns();
        if lease.reducer_authority != self.reducer.authority_id()
            || lease.state_generation != self.state_generation
            || lease.connection_epoch != self.session.connection_epoch()
            || Some(lease.snapshot_revision) != self.session.current_snapshot_revision()
            || lease.metadata_revision != self.session.metadata_revision()
            || lease.local_ingress_sequence != self.session.local_ingress_sequence()
            || self.reducer.ready_top() != Some(lease.top)
            || lease.activity_generation != self.admitted_activity_generation
            || lease.source_high_water != lease.activity_generation
            || source_high_water != lease.source_high_water
            || now_monotonic_ns < lease.checked_at_control_clock.monotonic_receive_ns()
            || now_monotonic_ns > lease.fresh_until_monotonic_ns
        {
            return Err(PmPublicBookRuntimeError::LeaseStateMismatch);
        }
        let successor = self.issue_lease(now, source_high_water)?;
        if successor.fresh_until_monotonic_ns != lease.fresh_until_monotonic_ns {
            return Err(PmPublicBookRuntimeError::LeaseStateMismatch);
        }
        Ok(successor)
    }

    fn freshness_deadline(&self) -> Result<u64, PmPublicBookRuntimeError> {
        let book_receive_ns = self
            .book_receive_clock
            .ok_or(PmPublicBookRuntimeError::BookUnavailable)?;
        let metadata_deadline = self
            .metadata_control_receive_ns
            .checked_add(self.freshness.metadata_max_age_ns())
            .ok_or(PmPublicBookRuntimeError::FreshnessDeadlineOverflow)?;
        let book_deadline = book_receive_ns
            .monotonic_receive_ns()
            .checked_add(self.freshness.book_max_age_ns())
            .ok_or(PmPublicBookRuntimeError::FreshnessDeadlineOverflow)?;
        Ok(metadata_deadline.min(book_deadline))
    }

    fn consume_unavailable(
        &mut self,
        expected_fault: Option<PmPublicSessionFault>,
    ) -> Result<(), PmPublicBookRuntimeError> {
        if self.unavailable_consumed_for_epoch {
            return Ok(());
        }
        let unavailable = self
            .session
            .take_unavailable()
            .ok_or(PmPublicBookRuntimeError::UnavailableEvidenceMissing)?;
        if unavailable.source() != self.session.role().source()
            || unavailable.connection_id() != self.session.role().connection()
            || unavailable.ordering().connection_epoch() != self.session.connection_epoch()
            || unavailable.ordering().local_ingress_sequence()
                != self.session.local_ingress_sequence()
            || expected_fault.is_some_and(|fault| unavailable.fault() != fault)
        {
            return Err(PmPublicBookRuntimeError::UnavailableEvidenceMismatch);
        }
        self.unavailable_consumed_for_epoch = true;
        Ok(())
    }

    fn invalidate_pair(
        &mut self,
        clock: reap_polymarket_live_adapter::PmPublicWsEdgeClock,
        session_fault: PmPublicSessionFault,
        reducer_fault: PmExternalBookFault,
    ) {
        let epoch = self.session.connection_epoch();
        let _ = self.session.invalidate_with_receive_evidence(
            session_fault,
            clock.local_wall_receive_ns(),
            clock.monotonic_receive_ns(),
        );
        let _ = self.reducer.apply_external_fault(epoch, reducer_fault);
        self.connection_open = false;
        self.last_heartbeat = None;
        self.book_receive_clock = None;
        self.pending_snapshot = None;
        let _ = self.bump_state_generation();
    }

    fn fail_closed(
        &mut self,
        clock: reap_polymarket_live_adapter::PmPublicWsEdgeClock,
        session_fault: PmPublicSessionFault,
        reducer_fault: PmExternalBookFault,
    ) {
        self.invalidate_pair(clock, session_fault, reducer_fault);
    }

    fn fail_closed_rest(
        &mut self,
        clock: PmRestResponseClock,
        session_fault: PmPublicSessionFault,
        reducer_fault: PmExternalBookFault,
    ) {
        let epoch = self.session.connection_epoch();
        let _ = self.session.invalidate_with_receive_evidence(
            session_fault,
            clock.local_wall_receive_ns(),
            clock.monotonic_receive_ns(),
        );
        let _ = self.reducer.apply_external_fault(epoch, reducer_fault);
        self.connection_open = false;
        self.last_heartbeat = None;
        self.book_receive_clock = None;
        self.pending_snapshot = None;
        let _ = self.bump_state_generation();
    }

    fn fail_closed_without_receive(
        &mut self,
        session_fault: PmPublicSessionFault,
        reducer_fault: PmExternalBookFault,
    ) {
        let epoch = self.session.connection_epoch();
        self.session.invalidate(session_fault);
        let _ = self.reducer.apply_external_fault(epoch, reducer_fault);
        self.connection_open = false;
        self.last_heartbeat = None;
        self.book_receive_clock = None;
        self.pending_snapshot = None;
        let _ = self.bump_state_generation();
    }
}

impl PmPublicBookRuntimeSink {
    /// Explicit synchronous exit hook for orchestration that drives the role
    /// elsewhere. Calling it repeatedly remains fail-closed and never restores
    /// a retired attempt.
    pub(super) fn invalidate_after_task_exit(&mut self) -> Result<(), PmPublicBookRuntimeError> {
        let mut state = self
            .shared
            .lock()
            .map_err(|_| PmPublicBookRuntimeError::LockPoisoned)?;
        state.fail_closed_without_receive(
            PmPublicSessionFault::Disconnect,
            PmExternalBookFault::Disconnect,
        );
        Ok(())
    }

    fn deliver_sync(&mut self, event: PmPublicWsEvent) -> Result<(), PmPublicBookRuntimeError> {
        let edge = event_clock(&event);
        let high_water = self.activity.generation();
        let mut state = self
            .shared
            .lock()
            .map_err(|_| PmPublicBookRuntimeError::LockPoisoned)?;
        let result = state.admit_ws_event(event, high_water);
        if result.is_err() {
            state.fail_closed(
                edge,
                PmPublicSessionFault::InvalidTransition,
                PmExternalBookFault::InvalidTransition,
            );
        }
        result
    }

    fn authorize_sync(
        &mut self,
        retired: PmPublicWsRetirement,
    ) -> Result<PmPublicWsReconnectDirective, PmPublicBookRuntimeError> {
        let high_water = self.activity.generation();
        let mut state = self
            .shared
            .lock()
            .map_err(|_| PmPublicBookRuntimeError::LockPoisoned)?;
        let result = state.authorize_reconnect(retired, high_water);
        if result.is_err() {
            state.fail_closed(
                retired.clock(),
                PmPublicSessionFault::InvalidTransition,
                PmExternalBookFault::InvalidTransition,
            );
        }
        result
    }
}

impl Drop for PmPublicBookRuntimeSink {
    fn drop(&mut self) {
        // Losing the long-lived sink means the socket task can no longer
        // prove continued admission. Never recover a poisoned guard.
        let _ = self.invalidate_after_task_exit();
    }
}

#[async_trait]
impl PmPublicWsEventSink for PmPublicBookRuntimeSink {
    type Error = PmPublicBookRuntimeError;

    async fn deliver_public_ws_event(&mut self, event: PmPublicWsEvent) -> Result<(), Self::Error> {
        self.deliver_sync(event)
    }

    async fn authorize_public_ws_reconnect(
        &mut self,
        retired: PmPublicWsRetirement,
    ) -> Result<PmPublicWsReconnectDirective, Self::Error> {
        self.authorize_sync(retired)
    }
}

impl PmPublicBookRuntimeHandle {
    fn observe_control_or_fault(&mut self) -> Result<ReceivedEventClock, PmPublicBookRuntimeError> {
        match self.clock.observe() {
            Ok(clock) => Ok(clock),
            Err(error) => {
                self.fault_without_receive()?;
                Err(error)
            }
        }
    }

    fn fault_without_receive(&self) -> Result<(), PmPublicBookRuntimeError> {
        let mut state = self
            .shared
            .lock()
            .map_err(|_| PmPublicBookRuntimeError::LockPoisoned)?;
        state.fail_closed_without_receive(
            PmPublicSessionFault::InvalidTransition,
            PmExternalBookFault::InvalidTransition,
        );
        Ok(())
    }

    /// Moves out the exact snapshot proof only while it is still the current
    /// serialized state and the source queue is empty at both observation
    /// edges.
    pub(super) fn take_snapshot_evidence(
        &mut self,
    ) -> Result<Option<PmPublicBookSnapshotEvidence>, PmPublicBookRuntimeError> {
        let before = self.activity.generation();
        let evidence = {
            let mut state = self
                .shared
                .lock()
                .map_err(|_| PmPublicBookRuntimeError::LockPoisoned)?;
            if before != state.admitted_activity_generation {
                return Err(PmPublicBookRuntimeError::ActivityQueued {
                    admitted: state.admitted_activity_generation,
                    source_high_water: before,
                });
            }
            let Some(evidence) = state.take_snapshot_evidence() else {
                return Ok(None);
            };
            if evidence.state_generation != state.state_generation
                || evidence.admitted_activity_generation != before
                || evidence.source_high_water != before
                || evidence.connection_epoch() != state.session.connection_epoch()
                || evidence.local_ingress_sequence() != state.session.local_ingress_sequence()
                || state.reducer.ready_top() != Some(evidence.top)
            {
                state.pending_snapshot = Some(evidence);
                return Err(PmPublicBookRuntimeError::SnapshotProofMismatch);
            }
            evidence
        };
        let after = self.activity.generation();
        if after != before {
            self.fault_without_receive()?;
            return Err(PmPublicBookRuntimeError::ActivityQueued {
                admitted: before,
                source_high_water: after,
            });
        }
        Ok(Some(evidence))
    }

    /// Mints one move-only lease using a source-owned product-clock sample.
    pub(super) fn lease(&mut self) -> Result<PmPublicBookLease, PmPublicBookRuntimeError> {
        let before = self.activity.generation();
        let now = self.observe_control_or_fault()?;
        let result = {
            let mut state = self
                .shared
                .lock()
                .map_err(|_| PmPublicBookRuntimeError::LockPoisoned)?;
            state.issue_lease(now, before)
        };
        let lease = match result {
            Ok(lease) => lease,
            Err(error) => {
                if matches!(
                    &error,
                    PmPublicBookRuntimeError::LeaseStateMismatch
                        | PmPublicBookRuntimeError::FreshnessDeadlineExpired
                        | PmPublicBookRuntimeError::Reducer(_)
                ) {
                    self.fault_without_receive()?;
                }
                return Err(error);
            }
        };
        let after = self.activity.generation();
        if after != before {
            return Err(PmPublicBookRuntimeError::ActivityQueued {
                admitted: lease.activity_generation,
                source_high_water: after,
            });
        }
        Ok(lease)
    }

    /// Consumes an old lease and returns a freshly sampled successor only if
    /// every reducer, session, heartbeat, activity, and freshness edge still
    /// agrees exactly.
    pub(super) fn recheck_lease(
        &mut self,
        lease: PmPublicBookLease,
    ) -> Result<PmPublicBookLease, PmPublicBookRuntimeError> {
        let before = self.activity.generation();
        let now = self.observe_control_or_fault()?;
        let result = {
            let mut state = self
                .shared
                .lock()
                .map_err(|_| PmPublicBookRuntimeError::LockPoisoned)?;
            state.recheck_lease(lease, now, before)
        };
        let successor = match result {
            Ok(successor) => successor,
            Err(error) => {
                self.fault_without_receive()?;
                return Err(error);
            }
        };
        let after = self.activity.generation();
        if after != before {
            return Err(PmPublicBookRuntimeError::ActivityQueued {
                admitted: successor.activity_generation,
                source_high_water: after,
            });
        }
        Ok(successor)
    }

    fn deliver_rest_sync(
        &mut self,
        purpose: PmRestBookPurpose,
        received: PmRestResponseClock,
        raw: &[u8],
    ) -> Result<PmPublicBookSnapshotEvidence, PmPublicBookRuntimeError> {
        let before = self.activity.generation();
        let evidence = {
            let mut state = self
                .shared
                .lock()
                .map_err(|_| PmPublicBookRuntimeError::LockPoisoned)?;
            if before != state.admitted_activity_generation {
                return Err(PmPublicBookRuntimeError::ActivityQueued {
                    admitted: state.admitted_activity_generation,
                    source_high_water: before,
                });
            }
            state.ingest_rest_snapshot(purpose, received, raw, before)?
        };
        let after = self.activity.generation();
        if after != before {
            self.fault_after_rest_race(received)?;
            return Err(PmPublicBookRuntimeError::ActivityQueued {
                admitted: before,
                source_high_water: after,
            });
        }
        Ok(evidence)
    }

    fn fault_after_rest_race(
        &self,
        received: PmRestResponseClock,
    ) -> Result<(), PmPublicBookRuntimeError> {
        let mut state = self
            .shared
            .lock()
            .map_err(|_| PmPublicBookRuntimeError::LockPoisoned)?;
        state.fail_closed_rest(
            received,
            PmPublicSessionFault::InvalidTransition,
            PmExternalBookFault::InvalidTransition,
        );
        Ok(())
    }
}

#[async_trait]
impl PmRestBookSnapshotSink for PmPublicBookRuntimeHandle {
    type Output = PmPublicBookSnapshotEvidence;
    type Error = PmPublicBookRuntimeError;

    async fn deliver_native_rest_book(
        &mut self,
        purpose: PmRestBookPurpose,
        received: PmRestResponseClock,
        raw: &[u8],
    ) -> Result<Self::Output, Self::Error> {
        self.deliver_rest_sync(purpose, received, raw)
    }
}

fn validate_freshness(freshness: PmBookFreshness) -> Result<(), PmPublicBookRuntimeError> {
    if freshness.metadata_max_age_ns() > MAX_PUBLIC_BOOK_AGE_NS
        || freshness.book_max_age_ns() > MAX_PUBLIC_BOOK_AGE_NS
    {
        return Err(PmPublicBookRuntimeError::Configuration(
            "public metadata and book age bounds must not exceed five seconds",
        ));
    }
    Ok(())
}

fn validate_reconnect_policy(
    reconnect: &ReconnectPolicy,
    transport: PmPublicWsTransportPolicy,
) -> Result<(), PmPublicBookRuntimeError> {
    if reconnect.initial_delay.is_zero()
        || reconnect.max_delay < reconnect.initial_delay
        || reconnect.max_delay > transport.max_reconnect_backoff()
        || reconnect.multiplier == 0
    {
        return Err(PmPublicBookRuntimeError::Configuration(
            "session reconnect policy exceeds the fixed WebSocket transport bounds",
        ));
    }
    Ok(())
}

fn heartbeat_config(
    transport: PmPublicWsTransportPolicy,
) -> Result<PmPublicHeartbeatConfig, PmPublicBookRuntimeError> {
    let ping = transport
        .heartbeat_interval()
        .as_nanos()
        .try_into()
        .map_err(|_| PmPublicBookRuntimeError::Configuration("heartbeat interval overflow"))?;
    let pong = transport
        .pong_timeout()
        .as_nanos()
        .try_into()
        .map_err(|_| PmPublicBookRuntimeError::Configuration("PONG timeout overflow"))?;
    PmPublicHeartbeatConfig::new(ping, pong).map_err(PmPublicBookRuntimeError::Session)
}

fn event_clock(event: &PmPublicWsEvent) -> reap_polymarket_live_adapter::PmPublicWsEdgeClock {
    match event {
        PmPublicWsEvent::ConnectionOpened(observation)
        | PmPublicWsEvent::SubscriptionSent(observation)
        | PmPublicWsEvent::PingSent(observation)
        | PmPublicWsEvent::Pong(observation)
        | PmPublicWsEvent::Shutdown(observation) => observation.clock(),
        PmPublicWsEvent::RawData(raw) => raw.clock(),
        PmPublicWsEvent::ConnectionRetired(retired)
        | PmPublicWsEvent::ReconnectStopped(retired) => retired.clock(),
        PmPublicWsEvent::ReconnectScheduled(reconnect) => reconnect.scheduled_clock(),
    }
}

fn retirement_fault(
    reason: PmPublicWsDisconnectReason,
) -> (PmPublicSessionFault, PmExternalBookFault) {
    if reason == PmPublicWsDisconnectReason::PongTimeout {
        (
            PmPublicSessionFault::HeartbeatTimeout,
            PmExternalBookFault::HeartbeatTimeout,
        )
    } else {
        (
            PmPublicSessionFault::Disconnect,
            PmExternalBookFault::Disconnect,
        )
    }
}

#[derive(Debug, Error)]
pub(super) enum PmPublicBookWsTaskError {
    #[error("public market WebSocket task failed: {0}")]
    Run(PmPublicWsRunError<PmPublicBookRuntimeError>),
    #[error("public-book task-exit invalidation failed: {0}")]
    ExitInvalidation(PmPublicBookRuntimeError),
}

#[derive(Debug, Error)]
pub(super) enum PmPublicBookRuntimeError {
    #[error("invalid public-book runtime configuration: {0}")]
    Configuration(&'static str),
    #[error("public WebSocket activity began before runtime ownership was sealed")]
    ActivityAlreadyStarted,
    #[error("public WebSocket source activity generation overflowed")]
    ActivityGenerationOverflow,
    #[error(
        "public WebSocket activity was discontinuous (expected {expected}, observed {observed}, source high-water {source_high_water})"
    )]
    ActivityDiscontinuity {
        expected: u64,
        observed: u64,
        source_high_water: u64,
    },
    #[error(
        "public WebSocket source has queued activity (admitted {admitted}, source high-water {source_high_water})"
    )]
    ActivityQueued {
        admitted: u64,
        source_high_water: u64,
    },
    #[error("public-book state generation overflowed")]
    StateGenerationOverflow,
    #[error("public-book shared state mutex was poisoned")]
    LockPoisoned,
    #[error("public-book metadata source and authoritative projection disagree")]
    MetadataProvenanceMismatch,
    #[error("public WebSocket connection identity or epoch differs")]
    ConnectionMismatch,
    #[error("public WebSocket lifecycle transition differs")]
    LifecycleMismatch,
    #[error("public heartbeat evidence differs from the canonical session transition")]
    HeartbeatMismatch,
    #[error("public-book delivery shape differs from the canonical session batch")]
    DeliveryShapeMismatch,
    #[error("public-book delivery identity differs from the canonical session/reducer")]
    DeliveryIdentityMismatch,
    #[error("public-book reducer returned an unexpected transition")]
    ReducerTransitionMismatch,
    #[error("public snapshot commit proof differs from the exact opened flow")]
    SnapshotProofMismatch,
    #[error("a previous snapshot proof must be consumed before continuing")]
    SnapshotEvidencePending,
    #[error("canonical ready top is unavailable")]
    ReadyTopUnavailable,
    #[error("public book is unavailable")]
    BookUnavailable,
    #[error("correlated post-subscription PONG evidence is missing")]
    HeartbeatMissing,
    #[error("public-book lease no longer matches current state")]
    LeaseStateMismatch,
    #[error("public-book freshness deadline expired")]
    FreshnessDeadlineExpired,
    #[error("public-book freshness deadline overflowed")]
    FreshnessDeadlineOverflow,
    #[error("shared actor control clock regressed")]
    ControlClockRegression,
    #[error("canonical public-session unavailable evidence was missing")]
    UnavailableEvidenceMissing,
    #[error("canonical public-session unavailable evidence differed")]
    UnavailableEvidenceMismatch,
    #[error("public reconnect authorization differs from transport evidence")]
    ReconnectMismatch,
    #[cfg(test)]
    #[error("test-only control clock was exhausted")]
    TestClockExhausted,
    #[error(transparent)]
    Metadata(#[from] PmLiveAuthoritativeMetadataError),
    #[error(transparent)]
    Role(#[from] PmPublicRoleError),
    #[error(transparent)]
    Session(#[from] PmPublicSessionError),
    #[error(transparent)]
    MetadataContract(#[from] PmMetadataContractError),
    #[error("public-book reducer rejected evidence: {0}")]
    Reducer(PmPublicReadinessReason),
    #[error(transparent)]
    Clock(#[from] PmProductClockError),
}

#[cfg(test)]
mod tests {
    use std::{
        collections::VecDeque,
        sync::{
            Arc,
            atomic::{AtomicU64, Ordering},
        },
        time::Duration,
    };

    use reap_pm_core::{
        EvmAddress, MAX_REQUIRED_SPENDERS, OkxInstrumentId, OkxReferenceInstrument, PmAssetId,
        PmChainId, PmConditionId, PmInstrumentId, PmMarketHandle, PmMarketId, PmMarketLifecycle,
        PmOutcomeLabel, PmOutcomeMetadata, PmQuantity, PmSourceHandle, PmSpenderDomain,
        PmSpenderRequirement, PmTick, PmTokenHandle, PmTokenId, U256,
    };
    use reap_polymarket_live_adapter::{PmPublicHttpConfig, PmPublicWsConfig};
    use reap_polymarket_wire::{MAX_PUBLIC_WS_FRAME_BYTES, PmWireScope, compute_snapshot_hash};
    use tokio::{
        io::{AsyncReadExt, AsyncWriteExt},
        net::TcpListener,
    };

    use super::*;

    const CONDITION: &str = "0xaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa";
    const MARKET: &str = "0xbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbb";
    const PUSD: &str = "0xC011a7E12a19f7B1f670d46F03B03f3342E82DFB";
    const CONDITIONAL_TOKENS: &str = "0x4D97DCd97eC945f40cF65F87097ACe5EA0476045";
    const STANDARD_EXCHANGE: &str = "0xE111180000d2663C0091e4f400237545B87B996B";
    const WALL_NS: u64 = 1_800_000_000_000_000_000;
    const CONTROL_BEGIN_NS: u64 = 10_000_000_000;
    const CONTROL_COMPLETE_NS: u64 = 10_050_000_000;
    const SUBSCRIBE_NS: u64 = 10_100_000_000;
    const SNAPSHOT_NS: u64 = 10_120_000_000;
    const PING_NS: u64 = 10_130_000_000;
    const PONG_NS: u64 = 10_140_000_000;

    fn scope() -> PmWireScope {
        PmWireScope::new(
            PmConditionId::parse(CONDITION).expect("condition"),
            PmMarketId::parse(MARKET).expect("market"),
            PmTokenId::new(U256::from_u64(123)).expect("token"),
        )
    }

    fn instrument() -> reap_pm_core::PmInstrumentHandle {
        reap_pm_core::PmInstrumentHandle::new(
            PmMarketHandle::from_ordinal(0),
            PmTokenHandle::from_ordinal(0),
        )
    }

    fn source() -> PmProductSource {
        PmProductSource::polymarket_market(PmSourceHandle::from_ordinal(4), instrument().token())
    }

    fn observation_grant() -> PmPublicObservationGrant {
        PmPublicObservationGrant::derive_goal_f(
            OkxReferenceInstrument::index(
                OkxInstrumentId::new("BTC-USDT").expect("OKX instrument"),
            ),
            PmInstrumentId::new(scope().market(), scope().token()),
        )
    }

    fn expected_metadata() -> PmMarketMetadata {
        let chain = PmChainId::new(137).expect("chain");
        let exchange = EvmAddress::parse(STANDARD_EXCHANGE).expect("exchange");
        let mut spenders = [None; MAX_REQUIRED_SPENDERS];
        spenders[0] = Some(PmSpenderRequirement::new(
            chain,
            exchange,
            PmSpenderDomain::Standard,
            PmAssetId::collateral(EvmAddress::parse(PUSD).expect("collateral")),
        ));
        spenders[1] = Some(PmSpenderRequirement::new(
            chain,
            exchange,
            PmSpenderDomain::Standard,
            PmAssetId::outcome(
                EvmAddress::parse(CONDITIONAL_TOKENS).expect("conditional tokens"),
                scope().token(),
            ),
        ));
        PmMarketMetadata::new(
            scope().condition(),
            scope().market(),
            PmOutcomeMetadata::new(
                scope().token(),
                PmOutcomeLabel::new("Yes").expect("outcome"),
            ),
            PmMarketLifecycle::new(true, false, false, true, true),
            PmTick::parse_decimal("0.01").expect("tick"),
            PmQuantity::parse_decimal("5").expect("minimum"),
            false,
            chain,
            exchange,
            spenders,
            2,
        )
        .expect("metadata")
    }

    fn long_market() -> String {
        format!(
            r#"{{"condition_id":"{CONDITION}","question_id":"{MARKET}","active":true,"closed":false,"archived":false,"accepting_orders":true,"enable_order_book":true,"accepting_order_timestamp":"2026-08-08T00:00:00Z","end_date_iso":"2027-01-01T00:00:00Z","game_start_time":null,"seconds_delay":0}}"#
        )
    }

    fn short_market() -> String {
        format!(
            r#"{{"c":"{CONDITION}","t":[{{"t":"123","o":"Yes"}},{{"t":"456","o":"No"}}],"mts":0.01,"mos":5,"nr":false,"fd":{{"r":0.02,"e":2,"to":true}},"mbf":0,"tbf":0,"ao":true,"sd":0,"gst":null,"cbos":true,"aot":"2026-08-08T00:00:00Z","rfqe":false,"itode":false,"ibce":true,"oas":0}}"#
        )
    }

    async fn metadata_role() -> (PmPublicMetadataHttpRole, tokio::task::JoinHandle<()>) {
        let listener = TcpListener::bind("127.0.0.1:0")
            .await
            .expect("bind metadata loopback");
        let address = listener.local_addr().expect("metadata address");
        let task = tokio::spawn(async move {
            for _ in 0..2 {
                let (mut stream, _) = listener.accept().await.expect("accept metadata request");
                let mut request = Vec::new();
                let mut buffer = [0_u8; 1024];
                loop {
                    let read = stream.read(&mut buffer).await.expect("read request");
                    if read == 0 {
                        break;
                    }
                    request.extend_from_slice(&buffer[..read]);
                    if request.windows(4).any(|window| window == b"\r\n\r\n") {
                        break;
                    }
                }
                let request = String::from_utf8(request).expect("HTTP request");
                let body = if request.starts_with("GET /markets/") {
                    long_market()
                } else if request.starts_with("GET /clob-markets/") {
                    short_market()
                } else {
                    panic!("unexpected metadata route: {request}");
                };
                let response = format!(
                    "HTTP/1.1 200 OK\r\nContent-Type: application/json\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{body}",
                    body.len()
                );
                stream
                    .write_all(response.as_bytes())
                    .await
                    .expect("write metadata response");
            }
        });
        let config = PmPublicHttpConfig::loopback_evidence(
            &format!("http://{address}"),
            Duration::from_secs(1),
            Duration::from_secs(1),
        )
        .expect("loopback public HTTP config");
        (
            PmPublicMetadataHttpRole::new(config, scope()).expect("metadata role"),
            task,
        )
    }

    fn control_clock(monotonic_ns: u64) -> ReceivedEventClock {
        ReceivedEventClock::new(None, WALL_NS + monotonic_ns, monotonic_ns)
            .expect("test control clock")
    }

    fn ws_edge(monotonic_ns: u64) -> reap_polymarket_live_adapter::PmPublicWsEdgeClock {
        reap_polymarket_live_adapter::PmPublicWsEdgeClock::new(WALL_NS + monotonic_ns, monotonic_ns)
            .expect("test WS edge")
    }

    fn transport_policy() -> PmPublicWsTransportPolicy {
        PmPublicWsConfig::loopback_evidence(
            "ws://127.0.0.1:9/ws/market",
            scope(),
            Duration::from_secs(1),
            Duration::from_secs(20),
            Duration::from_millis(20),
            Duration::from_millis(15),
            MAX_PUBLIC_WS_FRAME_BYTES,
            2,
            Duration::from_millis(10),
            8,
            ConnectionEpoch::new(11),
        )
        .expect("loopback WS config")
        .transport_policy()
    }

    async fn state_with_bracket(begin_ns: u64, complete_ns: u64) -> PmPublicBookState {
        let (metadata, server) = metadata_role().await;
        let joined = metadata
            .refresh_authoritative_observation(
                instrument(),
                source(),
                expected_metadata(),
                SnapshotRevision::new(7),
            )
            .await
            .expect("joined live metadata");
        server.await.expect("metadata server");
        let config = PmPublicBookRuntimeConfig::new(
            observation_grant(),
            source(),
            PmConnectionId::new("pm-controlled-public").expect("connection"),
            expected_metadata(),
            SnapshotRevision::new(7),
            None,
            PmBookFreshness::new(MAX_PUBLIC_BOOK_AGE_NS, MAX_PUBLIC_BOOK_AGE_NS)
                .expect("freshness"),
            ReconnectPolicy {
                initial_delay: Duration::from_millis(1),
                max_delay: Duration::from_millis(10),
                multiplier: 2,
            },
        );
        let mut state = PmPublicBookState::from_joined(
            joined,
            control_clock(begin_ns),
            control_clock(complete_ns),
            config,
            transport_policy(),
        )
        .expect("public-book state");
        state.connection_open = true;
        state
            .session
            .mark_subscription_sent(SUBSCRIBE_NS.max(complete_ns + 1))
            .expect("canonical subscription transition");
        state
    }

    async fn state() -> PmPublicBookState {
        state_with_bracket(CONTROL_BEGIN_NS, CONTROL_COMPLETE_NS).await
    }

    fn snapshot(timestamp: u64) -> String {
        let placeholder = format!(
            r#"{{
              "event_type":"book",
              "market":"{CONDITION}",
              "asset_id":"123",
              "timestamp":"{timestamp}",
              "hash":"",
              "bids":[{{"price":"0.30","size":"100"}},{{"price":"0.40","size":"50"}}],
              "asks":[{{"price":"0.60","size":"75"}},{{"price":"0.70","size":"100"}}],
              "min_order_size":"5",
              "tick_size":"0.01",
              "neg_risk":false,
              "last_trade_price":"0.50"
            }}"#
        );
        let hash = compute_snapshot_hash(placeholder.as_bytes()).expect("snapshot hash");
        placeholder.replace(r#""hash":"""#, &format!(r#""hash":"{hash}""#))
    }

    fn rest_snapshot(timestamp: u64) -> String {
        snapshot(timestamp).replace(r#""event_type":"book","#, "")
    }

    fn delta(timestamp: u64, price: &str, best_bid: &str, best_ask: &str) -> String {
        format!(
            r#"{{
              "event_type":"price_change",
              "market":"{CONDITION}",
              "timestamp":"{timestamp}",
              "price_changes":[
                {{
                  "asset_id":"123",
                  "price":"0.40",
                  "size":"0",
                  "side":"BUY",
                  "hash":"tx-delete",
                  "best_bid":"{best_bid}",
                  "best_ask":"{best_ask}"
                }},
                {{
                  "asset_id":"123",
                  "price":"{price}",
                  "size":"12.5",
                  "side":"BUY",
                  "hash":"tx-add",
                  "best_bid":"{best_bid}",
                  "best_ask":"{best_ask}"
                }}
              ]
            }}"#
        )
    }

    fn tick_change(timestamp: u64) -> String {
        format!(
            r#"{{
              "event_type":"tick_size_change",
              "market":"{CONDITION}",
              "asset_id":"123",
              "timestamp":"{timestamp}",
              "old_tick_size":"0.01",
              "new_tick_size":"0.001"
            }}"#
        )
    }

    fn admit_frame(
        state: &mut PmPublicBookState,
        raw: &[u8],
        monotonic_ns: u64,
        generation: u64,
        source: PmPublicBookSnapshotSource,
    ) -> Result<(), PmPublicBookRuntimeError> {
        state.validate_next_activity(generation, generation)?;
        let classified = state
            .session
            .classify(raw, WALL_NS + monotonic_ns, monotonic_ns)
            .map_err(PmPublicBookRuntimeError::Session)
            .and_then(|batch| state.reduce_batch(batch, source, generation));
        match classified {
            Ok(snapshot) => {
                state.finish_activity(generation)?;
                if let Some(snapshot) = snapshot {
                    state.pending_snapshot = Some(state.seal_snapshot(snapshot, generation));
                }
                Ok(())
            }
            Err(error) => {
                let _ = state.finish_activity(generation);
                state.fail_closed(
                    ws_edge(monotonic_ns),
                    PmPublicSessionFault::InvalidTransition,
                    PmExternalBookFault::InvalidTransition,
                );
                Err(error)
            }
        }
    }

    fn admit_rest_snapshot(
        state: &mut PmPublicBookState,
        raw: &[u8],
        monotonic_ns: u64,
    ) -> Result<PmPublicBookSnapshotEvidence, PmPublicBookRuntimeError> {
        let batch =
            state
                .session
                .classify_rest_book_snapshot(raw, WALL_NS + monotonic_ns, monotonic_ns)?;
        let snapshot = state
            .reduce_batch(batch, PmPublicBookSnapshotSource::RestSeed, 0)?
            .ok_or(PmPublicBookRuntimeError::DeliveryShapeMismatch)?;
        state.bump_state_generation()?;
        Ok(state.seal_snapshot(snapshot, 0))
    }

    fn admit_ping(
        state: &mut PmPublicBookState,
        monotonic_ns: u64,
        generation: u64,
    ) -> Result<(), PmPublicBookRuntimeError> {
        state.validate_next_activity(generation, generation)?;
        if state.session.poll_heartbeat(monotonic_ns)? != PmPublicHeartbeatAction::SendPing {
            return Err(PmPublicBookRuntimeError::HeartbeatMismatch);
        }
        state.last_heartbeat = None;
        state.finish_activity(generation)
    }

    fn admit_pong(
        state: &mut PmPublicBookState,
        monotonic_ns: u64,
        generation: u64,
    ) -> Result<(), PmPublicBookRuntimeError> {
        state.validate_next_activity(generation, generation)?;
        let batch =
            state
                .session
                .classify(PM_PUBLIC_PONG_BYTES, WALL_NS + monotonic_ns, monotonic_ns)?;
        let heartbeat = batch
            .heartbeat()
            .ok_or(PmPublicBookRuntimeError::HeartbeatMismatch)?;
        if !batch.events().is_empty()
            || !batch.ignored().is_empty()
            || batch.snapshot_flow_token().is_some()
        {
            return Err(PmPublicBookRuntimeError::HeartbeatMismatch);
        }
        state.last_heartbeat = Some(CorrelatedHeartbeat {
            evidence: heartbeat,
            activity_generation: generation,
        });
        state.finish_activity(generation)
    }

    struct Harness {
        sink: Option<PmPublicBookRuntimeSink>,
        handle: PmPublicBookRuntimeHandle,
        activity: Arc<AtomicU64>,
    }

    impl Harness {
        fn new(state: PmPublicBookState, clock_ns: &[u64]) -> Self {
            let activity = Arc::new(AtomicU64::new(state.admitted_activity_generation));
            let runtime = PmPublicBookRuntime::from_state(
                state,
                RuntimeActivityView::Test(Arc::clone(&activity)),
                RuntimeControlClock::Test(
                    clock_ns
                        .iter()
                        .copied()
                        .map(control_clock)
                        .collect::<VecDeque<_>>(),
                ),
            );
            let (sink, handle) = runtime.into_parts();
            Self {
                sink: Some(sink),
                handle,
                activity,
            }
        }

        fn publish(&self, generation: u64) {
            self.activity.store(generation, Ordering::Release);
        }

        fn with_state<T>(&self, f: impl FnOnce(&mut PmPublicBookState) -> T) -> T {
            let mut state = self.handle.shared.lock().expect("runtime state");
            f(&mut state)
        }
    }

    async fn ready_harness(clock_ns: &[u64]) -> (Harness, PmPublicBookLease) {
        let mut state = state().await;
        admit_frame(
            &mut state,
            snapshot(1_000).as_bytes(),
            SNAPSHOT_NS,
            1,
            PmPublicBookSnapshotSource::WebSocket,
        )
        .expect("snapshot");
        let mut harness = Harness::new(state, clock_ns);
        let proof = harness
            .handle
            .take_snapshot_evidence()
            .expect("snapshot proof command")
            .expect("snapshot proof");
        assert!(proof.source_was_fully_admitted());
        harness.publish(2);
        harness
            .with_state(|state| admit_ping(state, PING_NS, 2))
            .expect("PING");
        harness.publish(3);
        harness
            .with_state(|state| admit_pong(state, PONG_NS, 3))
            .expect("PONG");
        let lease = harness.handle.lease().expect("ready lease");
        (harness, lease)
    }

    #[tokio::test]
    async fn lease_requires_snapshot_proof_and_correlated_pong() {
        let state = state().await;
        let mut harness = Harness::new(
            state,
            &[SNAPSHOT_NS, SNAPSHOT_NS + 1, SNAPSHOT_NS + 2, PONG_NS + 1],
        );
        assert!(matches!(
            harness.handle.lease(),
            Err(PmPublicBookRuntimeError::BookUnavailable)
        ));
        harness.publish(1);
        harness
            .with_state(|state| {
                admit_frame(
                    state,
                    snapshot(1_000).as_bytes(),
                    SNAPSHOT_NS,
                    1,
                    PmPublicBookSnapshotSource::WebSocket,
                )
            })
            .expect("snapshot");
        assert!(matches!(
            harness.handle.lease(),
            Err(PmPublicBookRuntimeError::SnapshotEvidencePending)
        ));
        harness
            .handle
            .take_snapshot_evidence()
            .expect("proof command")
            .expect("proof");
        assert!(matches!(
            harness.handle.lease(),
            Err(PmPublicBookRuntimeError::HeartbeatMissing)
        ));
        harness.publish(2);
        harness
            .with_state(|state| admit_ping(state, PING_NS, 2))
            .expect("PING");
        harness.publish(3);
        harness
            .with_state(|state| admit_pong(state, PONG_NS, 3))
            .expect("PONG");
        assert!(harness.handle.lease().is_ok());
    }

    #[tokio::test]
    async fn unsolicited_pong_is_rejected_by_the_canonical_session() {
        let mut state = state().await;
        let error = state
            .session
            .classify(PM_PUBLIC_PONG_BYTES, WALL_NS + PONG_NS, PONG_NS)
            .expect_err("unsolicited PONG");
        assert_eq!(error, PmPublicSessionError::UnexpectedPong);
        assert!(state.session.requires_reconnect());
        assert!(state.last_heartbeat.is_none());
    }

    #[tokio::test]
    async fn a_new_ping_retires_the_old_pong_until_a_new_pong_arrives() {
        let (mut harness, _old) =
            ready_harness(&[PONG_NS + 1, PONG_NS + 21_000_000, PONG_NS + 31_000_000]).await;
        harness.publish(4);
        harness
            .with_state(|state| admit_ping(state, PONG_NS + 20_000_000, 4))
            .expect("next PING");
        assert!(matches!(
            harness.handle.lease(),
            Err(PmPublicBookRuntimeError::HeartbeatMissing)
        ));
        harness.publish(5);
        harness
            .with_state(|state| admit_pong(state, PONG_NS + 30_000_000, 5))
            .expect("next PONG");
        assert!(harness.handle.lease().is_ok());
    }

    #[tokio::test]
    async fn delta_before_snapshot_fails_closed() {
        let mut state = state().await;
        assert!(
            admit_frame(
                &mut state,
                delta(1_001, "0.50", "0.50", "0.60").as_bytes(),
                SNAPSHOT_NS,
                1,
                PmPublicBookSnapshotSource::WebSocket,
            )
            .is_err()
        );
        assert!(state.session.requires_reconnect());
        assert!(state.reducer.ready_top().is_none());
    }

    #[tokio::test]
    async fn websocket_and_rest_snapshots_use_the_same_reducer_and_flow_proof() {
        let mut websocket = state().await;
        let mut rest = state().await;
        admit_frame(
            &mut websocket,
            snapshot(1_000).as_bytes(),
            SNAPSHOT_NS,
            1,
            PmPublicBookSnapshotSource::WebSocket,
        )
        .expect("WS snapshot");
        let ws = websocket.take_snapshot_evidence().expect("WS proof");
        let rest = admit_rest_snapshot(&mut rest, rest_snapshot(1_000).as_bytes(), SNAPSHOT_NS)
            .expect("REST snapshot");
        assert_eq!(ws.ready_top(), rest.ready_top());
        assert_eq!(ws.connection_epoch(), rest.connection_epoch());
        assert_eq!(ws.metadata_revision(), rest.metadata_revision());
        assert_eq!(ws.snapshot_revision(), rest.snapshot_revision());
        assert_eq!(ws.source(), PmPublicBookSnapshotSource::WebSocket);
        assert_eq!(rest.source(), PmPublicBookSnapshotSource::RestSeed);
    }

    #[tokio::test]
    async fn queued_source_generation_blocks_lease_mint() {
        let (mut harness, _lease) = ready_harness(&[PONG_NS + 1, PONG_NS + 2]).await;
        harness.publish(4);
        assert!(matches!(
            harness.handle.lease(),
            Err(PmPublicBookRuntimeError::ActivityQueued {
                admitted: 3,
                source_high_water: 4
            })
        ));
    }

    #[tokio::test]
    async fn top_change_invalidates_the_consumed_old_lease() {
        let (mut harness, lease) = ready_harness(&[PONG_NS + 1, PONG_NS + 30_000_000]).await;
        harness.publish(4);
        harness
            .with_state(|state| {
                admit_frame(
                    state,
                    delta(1_001, "0.50", "0.50", "0.60").as_bytes(),
                    PONG_NS + 10_000_000,
                    4,
                    PmPublicBookSnapshotSource::WebSocket,
                )
            })
            .expect("delta");
        assert!(matches!(
            harness.handle.recheck_lease(lease),
            Err(PmPublicBookRuntimeError::LeaseStateMismatch)
        ));
    }

    #[tokio::test]
    async fn crossed_and_off_tick_updates_fail_closed() {
        for raw in [
            delta(1_001, "0.70", "0.70", "0.60"),
            delta(1_001, "0.555", "0.555", "0.60"),
        ] {
            let mut state = state().await;
            admit_frame(
                &mut state,
                snapshot(1_000).as_bytes(),
                SNAPSHOT_NS,
                1,
                PmPublicBookSnapshotSource::WebSocket,
            )
            .expect("snapshot");
            state.take_snapshot_evidence().expect("snapshot proof");
            assert!(
                admit_frame(
                    &mut state,
                    raw.as_bytes(),
                    SNAPSHOT_NS + 1,
                    2,
                    PmPublicBookSnapshotSource::WebSocket,
                )
                .is_err()
            );
            assert!(state.session.requires_reconnect());
            assert!(state.reducer.ready_top().is_none());
        }
    }

    #[tokio::test]
    async fn stale_actor_edge_and_prefetch_age_fail_closed() {
        let mut state =
            state_with_bracket(CONTROL_BEGIN_NS, CONTROL_BEGIN_NS + 4_000_000_000).await;
        let snapshot_ns = CONTROL_BEGIN_NS + 4_100_000_000;
        admit_frame(
            &mut state,
            snapshot(1_000).as_bytes(),
            snapshot_ns,
            1,
            PmPublicBookSnapshotSource::WebSocket,
        )
        .expect("snapshot");
        let mut harness = Harness::new(state, &[CONTROL_BEGIN_NS + MAX_PUBLIC_BOOK_AGE_NS + 1]);
        let proof = harness
            .handle
            .take_snapshot_evidence()
            .expect("proof command")
            .expect("proof");
        assert_eq!(
            proof.metadata_control_begin_clock().monotonic_receive_ns(),
            CONTROL_BEGIN_NS
        );
        assert_eq!(
            proof
                .metadata_control_complete_clock()
                .monotonic_receive_ns(),
            CONTROL_BEGIN_NS + 4_000_000_000
        );
        harness.publish(2);
        harness
            .with_state(|state| admit_ping(state, snapshot_ns + 10_000_000, 2))
            .expect("PING");
        harness.publish(3);
        harness
            .with_state(|state| admit_pong(state, snapshot_ns + 20_000_000, 3))
            .expect("PONG");
        assert!(matches!(
            harness.handle.lease(),
            Err(PmPublicBookRuntimeError::Reducer(
                PmPublicReadinessReason::MetadataStale
            ))
        ));
    }

    #[tokio::test]
    async fn retirement_reconnect_and_tick_drift_invalidate_old_leases() {
        let (mut retired, retired_lease) =
            ready_harness(&[PONG_NS + 1, PONG_NS + 30_000_000]).await;
        retired.with_state(|state| {
            state.invalidate_pair(
                ws_edge(PONG_NS + 10_000_000),
                PmPublicSessionFault::Disconnect,
                PmExternalBookFault::Disconnect,
            );
            state
                .consume_unavailable(Some(PmPublicSessionFault::Disconnect))
                .expect("unavailable");
            let transition = state
                .session
                .after_failure_transition()
                .expect("reconnect transition");
            state
                .reducer
                .begin_epoch(transition.replacement_epoch())
                .expect("replacement reducer epoch");
            state.bump_state_generation().expect("state generation");
        });
        assert!(retired.handle.recheck_lease(retired_lease).is_err());

        let (mut ticked, tick_lease) = ready_harness(&[PONG_NS + 1, PONG_NS + 30_000_000]).await;
        ticked.publish(4);
        assert!(
            ticked
                .with_state(|state| {
                    admit_frame(
                        state,
                        tick_change(1_001).as_bytes(),
                        PONG_NS + 10_000_000,
                        4,
                        PmPublicBookSnapshotSource::WebSocket,
                    )
                })
                .is_ok()
        );
        assert!(ticked.handle.recheck_lease(tick_lease).is_err());
    }

    #[tokio::test]
    async fn canonical_terminal_tick_binds_reducer_pre_invalidation_snapshot() {
        let mut state = state().await;
        admit_frame(
            &mut state,
            snapshot(1_000).as_bytes(),
            SNAPSHOT_NS,
            1,
            PmPublicBookSnapshotSource::WebSocket,
        )
        .expect("snapshot");
        state.take_snapshot_evidence().expect("snapshot proof");
        let batch = state
            .session
            .classify(
                tick_change(1_001).as_bytes(),
                WALL_NS + SNAPSHOT_NS + 1,
                SNAPSHOT_NS + 1,
            )
            .expect("canonical terminal tick batch");
        assert_eq!(batch.events().len(), 1);
        assert!(batch.events()[0].is_terminal_tick_size_change());
        assert!(state.session.current_snapshot_revision().is_none());
        assert_eq!(
            state.session.last_fault(),
            Some(PmPublicSessionFault::TickSizeChanged)
        );
        assert_eq!(
            state.reducer.readiness().snapshot_revision(),
            batch.events()[0].ordering().snapshot_revision()
        );
        assert!(
            state
                .reduce_batch(batch, PmPublicBookSnapshotSource::WebSocket, 2)
                .expect("terminal tick reduction")
                .is_none()
        );
        assert_eq!(
            state.reducer.readiness().reason(),
            Some(PmPublicReadinessReason::MetadataDrift(
                PmMetadataDrift::Grid
            ))
        );
        assert!(state.reducer.ready_top().is_none());
    }

    #[tokio::test]
    async fn backwards_or_missing_actor_clock_terminalizes_runtime() {
        let (mut backwards, lease) = ready_harness(&[PONG_NS + 10, PONG_NS + 9]).await;
        assert!(matches!(
            backwards.handle.recheck_lease(lease),
            Err(PmPublicBookRuntimeError::LeaseStateMismatch)
        ));
        assert!(backwards.with_state(|state| state.session.requires_reconnect()));

        let mut state = state().await;
        admit_frame(
            &mut state,
            snapshot(1_000).as_bytes(),
            SNAPSHOT_NS,
            1,
            PmPublicBookSnapshotSource::WebSocket,
        )
        .expect("snapshot");
        let mut missing = Harness::new(state, &[]);
        missing
            .handle
            .take_snapshot_evidence()
            .expect("proof command")
            .expect("proof");
        assert!(matches!(
            missing.handle.lease(),
            Err(PmPublicBookRuntimeError::TestClockExhausted)
        ));
        assert!(missing.with_state(|state| state.session.requires_reconnect()));
    }

    #[tokio::test]
    async fn task_exit_sink_drop_and_mutex_poison_never_leave_a_leaseable_handle() {
        let (mut exited, lease) = ready_harness(&[PONG_NS + 1, PONG_NS + 2]).await;
        exited
            .sink
            .as_mut()
            .expect("retained sink")
            .invalidate_after_task_exit()
            .expect("task-exit invalidation");
        assert!(exited.handle.recheck_lease(lease).is_err());

        let (mut dropped, lease) = ready_harness(&[PONG_NS + 1, PONG_NS + 2]).await;
        drop(dropped.sink.take());
        assert!(dropped.handle.recheck_lease(lease).is_err());

        let (mut poisoned, lease) = ready_harness(&[PONG_NS + 1, PONG_NS + 2]).await;
        let shared = Arc::clone(&poisoned.handle.shared);
        let worker = std::thread::spawn(move || {
            let _guard = shared.lock().expect("state lock before poison");
            panic!("intentional mutex poison");
        });
        assert!(worker.join().is_err());
        assert!(matches!(
            poisoned.handle.recheck_lease(lease),
            Err(PmPublicBookRuntimeError::LockPoisoned)
        ));
    }

    #[tokio::test]
    async fn cancelling_a_task_owned_sink_invalidates_every_old_lease() {
        let (mut cancelled, lease) = ready_harness(&[PONG_NS + 1, PONG_NS + 2]).await;
        let sink = cancelled.sink.take().expect("task-owned sink");
        let task = tokio::spawn(async move {
            // Keep the sink alive across the suspension point exactly as
            // `PmPublicBookWsTask::run_to_exit` does while the role runs.
            std::future::pending::<()>().await;
            drop(sink);
        });
        tokio::task::yield_now().await;
        task.abort();
        assert!(task.await.is_err());
        assert!(cancelled.handle.recheck_lease(lease).is_err());
    }
}
