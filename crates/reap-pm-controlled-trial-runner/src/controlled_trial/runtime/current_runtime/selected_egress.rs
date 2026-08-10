//! Denied, credential-free selected-egress HTTP actor custody.
//!
//! This child is intentionally private to `current_runtime`. It starts one
//! named OS thread, captures and immediately revalidates the exact authorized
//! Linux local-egress facts on that thread, and only then derives the six exact
//! reviewed fixed TLS peers and constructs five fixed-purpose HTTP clients.
//! The WebSocket peer remains inert; all peers, clients, and their exact
//! canonical policy, authorization, and destination-profile owners are
//! retained without making any source call. Construction finishes
//! synchronously on that dedicated thread before a runtime exists. A
//! current-thread Tokio runtime and `LocalSet` then confine the move-only actor
//! task, its resources, custody, and one private `Rc` generation to the same
//! thread until an explicit shutdown is joined.
//!
//! This is only a permanently denied bootstrap-topology milestone. Because its
//! custody predates any outer preflight window, it can never be reused by a
//! positive preflight. A later positive flow must begin, capture, finish, and
//! consume its window on this actor thread. The reviewed profile's time
//! envelope remains denied reviewer evidence: startup makes no caller-time or
//! current-time freshness claim. This slice has no credentials, WebSocket
//! transport, observation, selected-observation wrapper, window, candidate,
//! seal, HMAC, request, mutation, or order-entry capability.

use std::{
    fmt,
    path::{Path, PathBuf},
    rc::Rc,
    str::FromStr as _,
    sync::mpsc,
    thread::{self, JoinHandle, ThreadId},
    time::{Duration, Instant, SystemTime},
};

use reap_pm_controlled_trial::{
    CanonicalOnlineAuthorizationV2, CanonicalOnlinePolicyV2,
    CanonicalReviewedProductionDestinationProfileV1, CanonicalTrialConfig,
    OfflineAuthorizationState, TrialPhase, verify_reviewed_production_destination_profile_v1,
};
use reap_pm_core::{EvmAddress, PmConditionId, PmTokenId, U256};
use reap_polymarket_chain_source::PmPolygonAuthorizationSource;
use reap_polymarket_egress_binding::{PmFixedTlsPeerSelection, PmLocalEgressSelection};
use reap_polymarket_live_adapter::{
    PmClobLivenessHealthHttpRole, PmGeoblockHttpRole, PmStatusAnnouncementHttpRole,
};
use reap_polymarket_public_source::{PmDataApiCurrentPositionSource, PmDataApiPositionScope};
use thiserror::Error;
use tokio::{runtime::Builder as TokioRuntimeBuilder, sync::mpsc as tokio_mpsc, task::LocalSet};

use super::super::linux_egress_local_facts::PmLinuxEgressLocalFactCustody;

const SELECTED_EGRESS_ACTOR_THREAD_NAME: &str = "reap-pm-selected-egress";
const FIXED_HTTP_CONNECT_TIMEOUT: Duration = Duration::from_secs(3);
const FIXED_HTTP_REQUEST_TIMEOUT: Duration = Duration::from_secs(5);

#[derive(Debug, Clone, Copy, PartialEq, Eq, Error)]
pub(super) enum PmDeniedSelectedEgressActorError {
    #[error("selected-egress actor inputs are not the exact authorized Phase-A config")]
    AuthorizationConfigBinding,
    #[error("selected-egress actor reviewed destination profile binding is invalid")]
    ReviewedDestinationProfileBinding,
    #[error("selected-egress actor could not derive the exact position scope")]
    InvalidPositionScope,
    #[error("selected-egress actor OS thread could not be spawned")]
    ThreadSpawn,
    #[error("selected-egress actor local facts could not be captured")]
    LocalFactCapture,
    #[error("selected-egress actor local facts could not be revalidated")]
    LocalFactRevalidation,
    #[error("selected-egress actor local selection could not be constructed")]
    LocalSelection,
    #[error("selected-egress actor fixed TLS peer could not be constructed")]
    FixedPeerSelection,
    #[error("selected-egress actor fixed TLS peer differs from the local address family")]
    FixedPeerAddressFamily,
    #[error("selected-egress geoblock client could not be constructed")]
    GeoblockClient,
    #[error("selected-egress official-status client could not be constructed")]
    StatusClient,
    #[error("selected-egress CLOB-health client could not be constructed")]
    ClobHealthClient,
    #[error("selected-egress Polygon client could not be constructed")]
    PolygonClient,
    #[error("selected-egress Data API position client could not be constructed")]
    PositionClient,
    #[error("selected-egress current-thread runtime could not be constructed")]
    CurrentThreadRuntime,
    #[error("selected-egress actor startup acknowledgement failed")]
    StartupAcknowledgement,
    #[error("selected-egress actor command channel closed")]
    CommandChannelClosed,
    #[error("selected-egress actor task failed")]
    ActorTaskFailed,
    #[error("selected-egress actor thread changed")]
    ActorThreadChanged,
    #[error("selected-egress actor thread panicked")]
    ActorThreadPanicked,
    #[error("selected-egress actor shutdown could not be delivered")]
    ShutdownDelivery,
}

/// Inputs derived from exact canonical config before entering the actor
/// thread. No local-egress fact is accepted from the caller.
struct PmDeniedSelectedEgressActorStartup {
    online_policy: CanonicalOnlinePolicyV2,
    online_authorization: CanonicalOnlineAuthorizationV2,
    reviewed_destination_profile: CanonicalReviewedProductionDestinationProfileV1,
    reviewed_nonsecret_profile_path: PathBuf,
    position_scope: PmDataApiPositionScope,
    maximum_fact_age: Duration,
}

impl PmDeniedSelectedEgressActorStartup {
    fn from_exact_config(
        config: &CanonicalTrialConfig,
        online_policy: CanonicalOnlinePolicyV2,
        online_authorization: CanonicalOnlineAuthorizationV2,
        reviewed_destination_profile: CanonicalReviewedProductionDestinationProfileV1,
        reviewed_nonsecret_profile_path: &Path,
    ) -> Result<Self, PmDeniedSelectedEgressActorError> {
        // This is exact, clock-free reviewer-record verification only. It does
        // not claim that the profile or authorization is fresh at startup.
        let verification = verify_reviewed_production_destination_profile_v1(
            config,
            &online_policy,
            &online_authorization,
            &reviewed_destination_profile,
        )
        .map_err(|_| PmDeniedSelectedEgressActorError::ReviewedDestinationProfileBinding)?;
        if !verification.exact_v2_bindings_structurally_valid
            || !verification.fixed_six_destination_profile_structurally_valid
            || verification.live_dns_observation_checked
            || verification.destination_nat_equivalence_checked
            || verification.authorization_consumption_checked
            || verification.authorization != OfflineAuthorizationState::DENIED
        {
            return Err(PmDeniedSelectedEgressActorError::ReviewedDestinationProfileBinding);
        }
        let pins = &online_authorization.value().v1_config;
        if config.value().phase != TrialPhase::APlaceCancel
            || pins.canonical_config_sha256 != config.canonical_sha256()
            || pins.canonical_config_length != config.canonical_length()
            || pins.canonical_config_fingerprint != config.fingerprint()
            || pins.trial_plan_fingerprint != config.plan_fingerprint()
        {
            return Err(PmDeniedSelectedEgressActorError::AuthorizationConfigBinding);
        }
        let value = config.value();
        let proxy_funder = EvmAddress::parse(&value.account.funder)
            .map_err(|_| PmDeniedSelectedEgressActorError::InvalidPositionScope)?;
        let condition = PmConditionId::parse(&value.market.condition_id)
            .map_err(|_| PmDeniedSelectedEgressActorError::InvalidPositionScope)?;
        let token = PmTokenId::new(
            U256::from_str(&value.market.token_id)
                .map_err(|_| PmDeniedSelectedEgressActorError::InvalidPositionScope)?,
        )
        .map_err(|_| PmDeniedSelectedEgressActorError::InvalidPositionScope)?;

        Ok(Self {
            online_policy,
            online_authorization,
            reviewed_destination_profile,
            reviewed_nonsecret_profile_path: reviewed_nonsecret_profile_path.to_path_buf(),
            position_scope: PmDataApiPositionScope::new(proxy_funder, condition, token),
            maximum_fact_age: Duration::from_millis(
                value.time_limits.maximum_preflight_observation_age_ms,
            ),
        })
    }
}

/// Six capability-free fixed TLS peer selections derived solely from the
/// exact canonical reviewed destination profile. The WebSocket peer has no
/// transport owner in this milestone and is retained as inert reviewer input.
struct PmReviewedFixedTlsPeerBundle {
    geoblock_https: PmFixedTlsPeerSelection,
    clob_https: PmFixedTlsPeerSelection,
    status_https: PmFixedTlsPeerSelection,
    data_api_https: PmFixedTlsPeerSelection,
    polygon_rpc_https: PmFixedTlsPeerSelection,
    _clob_websocket_wss: PmFixedTlsPeerSelection,
}

impl PmReviewedFixedTlsPeerBundle {
    fn from_canonical_profile(
        profile: &CanonicalReviewedProductionDestinationProfileV1,
        selected_local_egress: &PmLocalEgressSelection,
    ) -> Result<Self, PmDeniedSelectedEgressActorError> {
        let destinations = &profile.value().destinations;
        let geoblock_https = PmFixedTlsPeerSelection::production(
            &destinations.geoblock_https.dns_name,
            &destinations.geoblock_https.peer_ip,
        )
        .map_err(|_| PmDeniedSelectedEgressActorError::FixedPeerSelection)?;
        let clob_https = PmFixedTlsPeerSelection::production(
            &destinations.clob_https.dns_name,
            &destinations.clob_https.peer_ip,
        )
        .map_err(|_| PmDeniedSelectedEgressActorError::FixedPeerSelection)?;
        let status_https = PmFixedTlsPeerSelection::production(
            &destinations.status_https.dns_name,
            &destinations.status_https.peer_ip,
        )
        .map_err(|_| PmDeniedSelectedEgressActorError::FixedPeerSelection)?;
        let data_api_https = PmFixedTlsPeerSelection::production(
            &destinations.data_api_https.dns_name,
            &destinations.data_api_https.peer_ip,
        )
        .map_err(|_| PmDeniedSelectedEgressActorError::FixedPeerSelection)?;
        let polygon_rpc_https = PmFixedTlsPeerSelection::production(
            &destinations.polygon_rpc_https.dns_name,
            &destinations.polygon_rpc_https.peer_ip,
        )
        .map_err(|_| PmDeniedSelectedEgressActorError::FixedPeerSelection)?;
        let clob_websocket_wss = PmFixedTlsPeerSelection::production(
            &destinations.clob_websocket_wss.dns_name,
            &destinations.clob_websocket_wss.peer_ip,
        )
        .map_err(|_| PmDeniedSelectedEgressActorError::FixedPeerSelection)?;

        geoblock_https
            .require_same_address_family(selected_local_egress)
            .map_err(|_| PmDeniedSelectedEgressActorError::FixedPeerAddressFamily)?;
        clob_https
            .require_same_address_family(selected_local_egress)
            .map_err(|_| PmDeniedSelectedEgressActorError::FixedPeerAddressFamily)?;
        status_https
            .require_same_address_family(selected_local_egress)
            .map_err(|_| PmDeniedSelectedEgressActorError::FixedPeerAddressFamily)?;
        data_api_https
            .require_same_address_family(selected_local_egress)
            .map_err(|_| PmDeniedSelectedEgressActorError::FixedPeerAddressFamily)?;
        polygon_rpc_https
            .require_same_address_family(selected_local_egress)
            .map_err(|_| PmDeniedSelectedEgressActorError::FixedPeerAddressFamily)?;
        clob_websocket_wss
            .require_same_address_family(selected_local_egress)
            .map_err(|_| PmDeniedSelectedEgressActorError::FixedPeerAddressFamily)?;

        Ok(Self {
            geoblock_https,
            clob_https,
            status_https,
            data_api_https,
            polygon_rpc_https,
            _clob_websocket_wss: clob_websocket_wss,
        })
    }
}

/// Five credential-free, fixed-purpose clients. Fields have no projection and
/// are retained only to establish the future actor's private ownership shape.
struct PmFixedSelectedEgressHttpBundle {
    _geoblock: PmGeoblockHttpRole,
    _official_status: PmStatusAnnouncementHttpRole,
    _clob_health: PmClobLivenessHealthHttpRole,
    _polygon_authorization: PmPolygonAuthorizationSource,
    _data_api_position: PmDataApiCurrentPositionSource,
}

impl PmFixedSelectedEgressHttpBundle {
    fn build(
        selection: &PmLocalEgressSelection,
        fixed_peers: &PmReviewedFixedTlsPeerBundle,
        position_scope: PmDataApiPositionScope,
    ) -> Result<Self, PmDeniedSelectedEgressActorError> {
        let geoblock = PmGeoblockHttpRole::production_on_fixed_tls_peer_and_selected_local_egress(
            FIXED_HTTP_CONNECT_TIMEOUT,
            FIXED_HTTP_REQUEST_TIMEOUT,
            fixed_peers.geoblock_https.clone(),
            selection.clone(),
        )
        .map_err(|_| PmDeniedSelectedEgressActorError::GeoblockClient)?;
        let official_status =
            PmStatusAnnouncementHttpRole::production_on_fixed_tls_peer_and_selected_local_egress(
                FIXED_HTTP_CONNECT_TIMEOUT,
                FIXED_HTTP_REQUEST_TIMEOUT,
                fixed_peers.status_https.clone(),
                selection.clone(),
            )
            .map_err(|_| PmDeniedSelectedEgressActorError::StatusClient)?;
        let clob_health =
            PmClobLivenessHealthHttpRole::production_on_fixed_tls_peer_and_selected_local_egress(
                FIXED_HTTP_CONNECT_TIMEOUT,
                FIXED_HTTP_REQUEST_TIMEOUT,
                fixed_peers.clob_https.clone(),
                selection.clone(),
            )
            .map_err(|_| PmDeniedSelectedEgressActorError::ClobHealthClient)?;
        let polygon_authorization =
            PmPolygonAuthorizationSource::production_on_fixed_tls_peer_and_selected_local_egress(
                &fixed_peers.polygon_rpc_https,
                selection,
            )
            .map_err(|_| PmDeniedSelectedEgressActorError::PolygonClient)?;
        let data_api_position =
            PmDataApiCurrentPositionSource::production_on_fixed_tls_peer_and_selected_local_egress(
                position_scope,
                FIXED_HTTP_CONNECT_TIMEOUT,
                FIXED_HTTP_REQUEST_TIMEOUT,
                &fixed_peers.data_api_https,
                selection,
            )
            .map_err(|_| PmDeniedSelectedEgressActorError::PositionClient)?;
        Ok(Self {
            _geoblock: geoblock,
            _official_status: official_status,
            _clob_health: clob_health,
            _polygon_authorization: polygon_authorization,
            _data_api_position: data_api_position,
        })
    }
}

/// All production resources remain inseparable and die on their source
/// thread. This bootstrap custody is permanently denied and is retained only
/// to prove the intended lifetime and teardown topology.
struct PmDeniedSelectedEgressActorResources {
    _selected_http_bundle: PmFixedSelectedEgressHttpBundle,
    _reviewed_fixed_tls_peers: PmReviewedFixedTlsPeerBundle,
    _selected_local_egress: PmLocalEgressSelection,
    _local_egress_custody: PmLinuxEgressLocalFactCustody,
    _online_authorization: CanonicalOnlineAuthorizationV2,
    _online_policy: CanonicalOnlinePolicyV2,
    _reviewed_destination_profile: CanonicalReviewedProductionDestinationProfileV1,
    _reviewed_nonsecret_profile_path: PathBuf,
}

trait PmSelectedEgressActorResources: 'static {
    fn on_actor_task_enter(&self) -> Result<(), PmDeniedSelectedEgressActorError> {
        Ok(())
    }
}

impl PmSelectedEgressActorResources for PmDeniedSelectedEgressActorResources {}

struct PmSelectedEgressActorGeneration {
    creating_process_id: u32,
    source_thread_id: ThreadId,
}

impl PmSelectedEgressActorGeneration {
    fn current() -> Self {
        Self {
            creating_process_id: std::process::id(),
            source_thread_id: thread::current().id(),
        }
    }

    fn revalidate(&self) -> Result<(), PmDeniedSelectedEgressActorError> {
        if self.creating_process_id != std::process::id()
            || self.source_thread_id != thread::current().id()
        {
            return Err(PmDeniedSelectedEgressActorError::ActorThreadChanged);
        }
        Ok(())
    }
}

struct PmSelectedEgressActorState<R> {
    _resources: R,
    generation: Rc<PmSelectedEgressActorGeneration>,
}

enum PmSelectedEgressActorCommand {
    Shutdown,
}

#[must_use = "selected-egress actor supervisor must be shut down and joined"]
pub(super) struct PmDeniedSelectedEgressActorSupervisor {
    commands: Option<tokio_mpsc::UnboundedSender<PmSelectedEgressActorCommand>>,
    thread: Option<JoinHandle<Result<(), PmDeniedSelectedEgressActorError>>>,
    armed: bool,
}

impl PmDeniedSelectedEgressActorSupervisor {
    pub(super) fn spawn(
        config: &CanonicalTrialConfig,
        online_policy: CanonicalOnlinePolicyV2,
        online_authorization: CanonicalOnlineAuthorizationV2,
        reviewed_destination_profile: CanonicalReviewedProductionDestinationProfileV1,
        reviewed_nonsecret_profile_path: &Path,
    ) -> Result<Self, PmDeniedSelectedEgressActorError> {
        let startup = PmDeniedSelectedEgressActorStartup::from_exact_config(
            config,
            online_policy,
            online_authorization,
            reviewed_destination_profile,
            reviewed_nonsecret_profile_path,
        )?;
        spawn_selected_egress_actor_thread(move || build_production_resources(startup))
    }

    pub(super) fn shutdown_and_join(mut self) -> Result<(), PmDeniedSelectedEgressActorError> {
        let shutdown_delivered = self.commands.take().is_some_and(|commands| {
            commands
                .send(PmSelectedEgressActorCommand::Shutdown)
                .is_ok()
        });
        let joined = self
            .thread
            .take()
            .ok_or(PmDeniedSelectedEgressActorError::ActorThreadPanicked)?
            .join();
        self.armed = false;
        if !shutdown_delivered {
            return Err(PmDeniedSelectedEgressActorError::ShutdownDelivery);
        }
        joined.map_err(|_| PmDeniedSelectedEgressActorError::ActorThreadPanicked)??;
        Ok(())
    }
}

impl Drop for PmDeniedSelectedEgressActorSupervisor {
    fn drop(&mut self) {
        if self.armed {
            // An early drop must not detach the thread-confined descriptor and
            // client custody. Synchronous shutdown-and-join is mandatory.
            std::process::abort();
        }
    }
}

impl fmt::Debug for PmDeniedSelectedEgressActorSupervisor {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str("PmDeniedSelectedEgressActorSupervisor(<ARMED; denied>)")
    }
}

fn build_production_resources(
    startup: PmDeniedSelectedEgressActorStartup,
) -> Result<PmDeniedSelectedEgressActorResources, PmDeniedSelectedEgressActorError> {
    let PmDeniedSelectedEgressActorStartup {
        online_policy,
        online_authorization,
        reviewed_destination_profile,
        reviewed_nonsecret_profile_path,
        position_scope,
        maximum_fact_age,
    } = startup;
    let window_wall_started = SystemTime::now();
    let window_monotonic_started = Instant::now();
    let mut local_egress_custody = PmLinuxEgressLocalFactCustody::capture(
        &online_authorization,
        &reviewed_nonsecret_profile_path,
    )
    .map_err(|_| PmDeniedSelectedEgressActorError::LocalFactCapture)?;
    let window_wall_completed = SystemTime::now();
    let window_monotonic_completed = Instant::now();
    let (selected_local_egress, reviewed_fixed_tls_peers, selected_http_bundle) = {
        let revalidated_local_egress = local_egress_custody
            .revalidate_for_current_runtime(
                &online_authorization,
                window_wall_started,
                window_wall_completed,
                window_monotonic_started,
                window_monotonic_completed,
                maximum_fact_age,
            )
            .map_err(|_| PmDeniedSelectedEgressActorError::LocalFactRevalidation)?;
        let selection = PmLocalEgressSelection::production(
            revalidated_local_egress.interface_name(),
            revalidated_local_egress.local_source_ip(),
        )
        .map_err(|_| PmDeniedSelectedEgressActorError::LocalSelection)?;
        let fixed_peers = PmReviewedFixedTlsPeerBundle::from_canonical_profile(
            &reviewed_destination_profile,
            &selection,
        )?;
        // Keep the borrowed revalidation view live across all six peer and all
        // five HTTP constructors.
        let bundle =
            PmFixedSelectedEgressHttpBundle::build(&selection, &fixed_peers, position_scope)?;
        if selection.interface_name() != revalidated_local_egress.interface_name()
            || selection.local_source_ip() != revalidated_local_egress.local_source_ip()
        {
            return Err(PmDeniedSelectedEgressActorError::LocalSelection);
        }
        (selection, fixed_peers, bundle)
    };
    // Close client-construction drift before Ready. This remains denied:
    // bootstrap capture is outside any later positive outer window.
    let post_constructor_local_egress = local_egress_custody
        .revalidate_for_current_runtime(
            &online_authorization,
            window_wall_started,
            window_wall_completed,
            window_monotonic_started,
            window_monotonic_completed,
            maximum_fact_age,
        )
        .map_err(|_| PmDeniedSelectedEgressActorError::LocalFactRevalidation)?;
    if selected_local_egress.interface_name() != post_constructor_local_egress.interface_name()
        || selected_local_egress.local_source_ip()
            != post_constructor_local_egress.local_source_ip()
    {
        return Err(PmDeniedSelectedEgressActorError::LocalSelection);
    }
    drop(post_constructor_local_egress);
    Ok(PmDeniedSelectedEgressActorResources {
        _selected_http_bundle: selected_http_bundle,
        _reviewed_fixed_tls_peers: reviewed_fixed_tls_peers,
        _selected_local_egress: selected_local_egress,
        _local_egress_custody: local_egress_custody,
        _online_authorization: online_authorization,
        _online_policy: online_policy,
        _reviewed_destination_profile: reviewed_destination_profile,
        _reviewed_nonsecret_profile_path: reviewed_nonsecret_profile_path,
    })
}

fn spawn_selected_egress_actor_thread<R, F>(
    setup: F,
) -> Result<PmDeniedSelectedEgressActorSupervisor, PmDeniedSelectedEgressActorError>
where
    R: PmSelectedEgressActorResources,
    F: FnOnce() -> Result<R, PmDeniedSelectedEgressActorError> + Send + 'static,
{
    let (commands, command_receiver) = tokio_mpsc::unbounded_channel();
    let (startup_sender, startup_receiver) = mpsc::sync_channel(1);
    let actor_thread = thread::Builder::new()
        .name(SELECTED_EGRESS_ACTOR_THREAD_NAME.to_owned())
        .spawn(move || {
            let outcome =
                run_selected_egress_actor_thread(setup, command_receiver, &startup_sender);
            if let Err(error) = outcome {
                let _ = startup_sender.send(Err(error));
            }
            outcome
        })
        .map_err(|_| PmDeniedSelectedEgressActorError::ThreadSpawn)?;

    match startup_receiver.recv() {
        Ok(Ok(())) => Ok(PmDeniedSelectedEgressActorSupervisor {
            commands: Some(commands),
            thread: Some(actor_thread),
            armed: true,
        }),
        Ok(Err(error)) => {
            let _ = actor_thread.join();
            Err(error)
        }
        Err(_) => {
            let joined = actor_thread.join();
            if joined.is_err() {
                Err(PmDeniedSelectedEgressActorError::ActorThreadPanicked)
            } else {
                Err(PmDeniedSelectedEgressActorError::StartupAcknowledgement)
            }
        }
    }
}

fn run_selected_egress_actor_thread<R, F>(
    setup: F,
    command_receiver: tokio_mpsc::UnboundedReceiver<PmSelectedEgressActorCommand>,
    startup_sender: &mpsc::SyncSender<Result<(), PmDeniedSelectedEgressActorError>>,
) -> Result<(), PmDeniedSelectedEgressActorError>
where
    R: PmSelectedEgressActorResources,
    F: FnOnce() -> Result<R, PmDeniedSelectedEgressActorError>,
{
    // Production setup performs capture, consuming revalidation, selection,
    // and all client construction before this runtime boundary.
    let resources = setup()?;
    let runtime = TokioRuntimeBuilder::new_current_thread()
        .enable_all()
        .build()
        .map_err(|_| PmDeniedSelectedEgressActorError::CurrentThreadRuntime)?;
    let local_set = LocalSet::new();
    let generation = Rc::new(PmSelectedEgressActorGeneration::current());
    let state = PmSelectedEgressActorState {
        _resources: resources,
        generation,
    };
    let actor_task = local_set.spawn_local(run_selected_egress_actor(
        state,
        command_receiver,
        startup_sender.clone(),
    ));
    runtime.block_on(local_set.run_until(async move {
        actor_task
            .await
            .map_err(|_| PmDeniedSelectedEgressActorError::ActorTaskFailed)?
    }))
}

async fn run_selected_egress_actor<R: PmSelectedEgressActorResources>(
    state: PmSelectedEgressActorState<R>,
    mut commands: tokio_mpsc::UnboundedReceiver<PmSelectedEgressActorCommand>,
    startup_sender: mpsc::SyncSender<Result<(), PmDeniedSelectedEgressActorError>>,
) -> Result<(), PmDeniedSelectedEgressActorError> {
    state.generation.revalidate()?;
    state._resources.on_actor_task_enter()?;
    startup_sender
        .send(Ok(()))
        .map_err(|_| PmDeniedSelectedEgressActorError::StartupAcknowledgement)?;
    match commands.recv().await {
        Some(PmSelectedEgressActorCommand::Shutdown) => {
            state.generation.revalidate()?;
            // Field order drops clients and selection before descriptor
            // custody; the whole state drops before `block_on` returns and
            // the current-thread runtime is torn down.
            drop(state);
            Ok(())
        }
        None => Err(PmDeniedSelectedEgressActorError::CommandChannelClosed),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[derive(Debug, Clone, PartialEq, Eq)]
    enum LifecycleEvent {
        Setup(ThreadId, Option<String>),
        Task(ThreadId, Option<String>),
        Dropped(ThreadId, Option<String>),
    }

    struct TestResources {
        events: mpsc::Sender<LifecycleEvent>,
        fail_on_enter: bool,
    }

    impl PmSelectedEgressActorResources for TestResources {
        fn on_actor_task_enter(&self) -> Result<(), PmDeniedSelectedEgressActorError> {
            self.events
                .send(LifecycleEvent::Task(
                    thread::current().id(),
                    thread::current().name().map(str::to_owned),
                ))
                .map_err(|_| PmDeniedSelectedEgressActorError::ActorTaskFailed)?;
            if self.fail_on_enter {
                return Err(PmDeniedSelectedEgressActorError::ActorTaskFailed);
            }
            Ok(())
        }
    }

    impl Drop for TestResources {
        fn drop(&mut self) {
            let _ = self.events.send(LifecycleEvent::Dropped(
                thread::current().id(),
                thread::current().name().map(str::to_owned),
            ));
        }
    }

    #[test]
    fn current_thread_local_set_actor_is_tid_confined_and_shutdown_is_joined() {
        let parent_thread = thread::current().id();
        let (events, received) = mpsc::channel();
        let setup_events = events.clone();
        let supervisor = spawn_selected_egress_actor_thread(move || {
            setup_events
                .send(LifecycleEvent::Setup(
                    thread::current().id(),
                    thread::current().name().map(str::to_owned),
                ))
                .map_err(|_| PmDeniedSelectedEgressActorError::ActorTaskFailed)?;
            Ok(TestResources {
                events: setup_events,
                fail_on_enter: false,
            })
        })
        .expect("test actor must start");
        supervisor
            .shutdown_and_join()
            .expect("shutdown must be delivered and the OS thread joined");
        drop(events);

        let lifecycle: Vec<_> = received.into_iter().collect();
        assert_eq!(lifecycle.len(), 3);
        let expected_name = Some(SELECTED_EGRESS_ACTOR_THREAD_NAME.to_owned());
        let LifecycleEvent::Setup(setup_tid, setup_name) = &lifecycle[0] else {
            panic!("setup must be first");
        };
        let LifecycleEvent::Task(task_tid, task_name) = &lifecycle[1] else {
            panic!("local task must be second");
        };
        let LifecycleEvent::Dropped(drop_tid, drop_name) = &lifecycle[2] else {
            panic!("owned resources must drop last");
        };
        assert_ne!(*setup_tid, parent_thread);
        assert_eq!(setup_tid, task_tid);
        assert_eq!(task_tid, drop_tid);
        assert_eq!(setup_name, &expected_name);
        assert_eq!(task_name, &expected_name);
        assert_eq!(drop_name, &expected_name);
    }

    #[test]
    fn failing_task_entry_returns_startup_error_without_a_supervisor() {
        let (events, received) = mpsc::channel();
        let setup_events = events.clone();
        let result = spawn_selected_egress_actor_thread(move || {
            setup_events
                .send(LifecycleEvent::Setup(
                    thread::current().id(),
                    thread::current().name().map(str::to_owned),
                ))
                .map_err(|_| PmDeniedSelectedEgressActorError::ActorTaskFailed)?;
            Ok(TestResources {
                events: setup_events,
                fail_on_enter: true,
            })
        });
        assert_eq!(
            result.expect_err("fallible task entry must not return an armed supervisor"),
            PmDeniedSelectedEgressActorError::ActorTaskFailed,
        );
        drop(events);

        let lifecycle: Vec<_> = received.into_iter().collect();
        assert_eq!(lifecycle.len(), 3);
        assert!(matches!(lifecycle[0], LifecycleEvent::Setup(_, _)));
        assert!(matches!(lifecycle[1], LifecycleEvent::Task(_, _)));
        assert!(matches!(lifecycle[2], LifecycleEvent::Dropped(_, _)));
    }
}
