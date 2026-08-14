//! Denied selected-egress observation and credential custody.
//!
//! This child is intentionally private to `current_runtime`. It starts one
//! named OS thread and allocates one private actor generation before capturing
//! any local fact or constructing any role. Exact reviewed peers, one selected
//! local route, credential-free HTTP clients, and dormant observation-only
//! public roles are then constructed and retained on that thread. The same
//! current-thread runtime and `LocalSet` drive only the denied actor lifecycle;
//! no credential or observation task is armed.
//!
//! This remains a permanently denied construction milestone. `Ready` means
//! only that the denied carriers were constructed and final local source
//! revalidation succeeded. It does not claim that a delivery token was
//! consumed, token/evidence pair identity was runtime-verified, a credential
//! directory/EUID/lease/descriptor/credential binding was observed, a socket
//! connected, a source was called, or any live freshness/preflight condition
//! holds. The sole command is `Shutdown`. No
//! role projection, WebSocket run, HTTP source call, deferred-clock promotion,
//! seal, HMAC, request, mutation, or order-entry path is reachable here.
//! The retained `Rc` generation is only an actor-lifecycle topology check; it
//! is not embedded in any selected HTTP/WS source fact and is not positive
//! provenance for a socket, observation, or authorization.
//!
//! Before the actor thread starts, the complete denied local-operator profile
//! context is structurally reverified and one whole delivery token is retained
//! inseparably with its no-path evidence and reviewed profile. Only after all
//! fallible non-secret construction, scope, local-fact, and generation checks
//! does the actor bind that unopened aggregate to its private `Rc` generation.
//! The token exposes no non-consuming fingerprint projection, so exact
//! token/evidence pair identity remains deferred to a future realization
//! boundary. This tranche deliberately does not realize it: the current owner
//! has no reviewed cleanup/terminal API, and Linux supplies no atomic
//! unlink-if-inode primitive. Shutdown only drops the unopened aggregate; it
//! makes no credential-drop, lease-release, basename-removal, secure-erasure,
//! provider, currentness, remote-owner, or mutation-authority claim.

use std::{
    fmt,
    path::{Path, PathBuf},
    rc::Rc,
    str::FromStr as _,
    sync::mpsc,
    thread::{self, JoinHandle, ThreadId},
    time::{Duration, Instant, SystemTime},
};

use async_trait::async_trait;
use reap_pm_controlled_trial::{
    CanonicalFreshCredentialDeliveryBindingV1, CanonicalOnlineAuthorizationV2,
    CanonicalOnlinePolicyV2, CanonicalReviewedFreshCredentialSlotLocatorV1,
    CanonicalReviewedLocalOperatorCooperativeCustodyProfileV1,
    CanonicalReviewedProductionDestinationProfileV1, CanonicalTrialConfig,
    OfflineAuthorizationState, ReviewedLocalOperatorCooperativeCustodyProfileContextV1,
    TrialDomain, TrialPhase, bind_fresh_credential_delivery_binding_v1,
    verify_reviewed_production_destination_profile_v1,
};
use reap_pm_core::{
    ConnectionEpoch, EvmAddress, PmConditionId, PmMarketId, PmQuantity, PmTick, PmTokenId, U256,
};
use reap_polymarket_auth::EoaAddress;
use reap_polymarket_chain_source::PmPolygonAuthorizationSource;
use reap_polymarket_egress_binding::{PmFixedTlsPeerSelection, PmLocalEgressSelection};
use reap_polymarket_live_adapter::{
    PmClobLivenessHealthHttpRole, PmGeoblockHttpRole, PmProductClockOwner,
    PmPublicObservationWithDeferredMutationClockOwner, PmPublicWsBounds, PmPublicWsConfig,
    PmStatusAnnouncementHttpRole,
};
use reap_polymarket_public_source::{PmDataApiCurrentPositionSource, PmDataApiPositionScope};
use reap_polymarket_wire::{MAX_PUBLIC_WS_FRAME_BYTES, PmBookParserConfig, PmWireScope};
use thiserror::Error;
use tokio::{runtime::Builder as TokioRuntimeBuilder, sync::mpsc as tokio_mpsc, task::LocalSet};

use super::super::{
    linux_egress_local_facts::PmLinuxEgressLocalFactCustody,
    public_book::PmDeferredObservationRuntimeRoles,
};
use crate::controlled_trial::authority::{
    GenerationBoundUnopenedLocalOperatorFreshCredentialCustodyV1,
    PreparedUnopenedLocalOperatorFreshCredentialCustodyV1,
    prepare_unopened_local_operator_fresh_credential_custody_v1,
};

const SELECTED_EGRESS_ACTOR_THREAD_NAME: &str = "reap-pm-selected-egress";
const FIXED_HTTP_CONNECT_TIMEOUT: Duration = Duration::from_secs(3);
const FIXED_HTTP_REQUEST_TIMEOUT: Duration = Duration::from_secs(5);
const FIXED_WS_IDLE_TIMEOUT: Duration = Duration::from_secs(30);
const FIXED_WS_PONG_TIMEOUT: Duration = Duration::from_secs(5);
const FIXED_WS_RECONNECT_ATTEMPTS: u8 = 0;
const FIXED_WS_RECONNECT_BACKOFF: Duration = Duration::from_secs(1);
const FIXED_WS_EVENT_CHANNEL_CAPACITY: usize = 8;
#[derive(Debug, Clone, Copy, PartialEq, Eq, Error)]
pub(super) enum PmDeniedSelectedEgressActorError {
    #[error("selected-egress actor inputs are not the exact authorized Phase-A config")]
    AuthorizationConfigBinding,
    #[error("selected-egress actor reviewed destination profile binding is invalid")]
    ReviewedDestinationProfileBinding,
    #[error("selected-egress actor reviewed local cooperative custody binding is invalid")]
    ReviewedLocalCooperativeCustodyBinding,
    #[error("selected-egress actor could not derive the exact position scope")]
    InvalidPositionScope,
    #[error("selected-egress actor observation configuration is invalid")]
    ObservationConfiguration,
    #[error("selected-egress actor public observation roles could not be constructed")]
    PublicObservationConstruction,
    #[error("selected-egress actor unopened custody shutdown was abnormal")]
    CustodyShutdownAbnormal,
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

/// Complete non-secret reviewed input conjunction for one denied selected
/// actor startup. The parent runtime can construct this move-only carrier but
/// cannot replace any role with a raw credential path, signer, secret,
/// runtime fact, transport, request, or authority value.
pub(super) struct PmDeniedSelectedEgressReviewedInputConjunction<'input, 'holders> {
    pub(super) config: &'input CanonicalTrialConfig,
    pub(super) online_policy: CanonicalOnlinePolicyV2,
    pub(super) online_authorization: CanonicalOnlineAuthorizationV2,
    pub(super) reviewed_destination_profile: CanonicalReviewedProductionDestinationProfileV1,
    pub(super) reviewed_nonsecret_profile_path: &'input Path,
    pub(super) reviewed_local_operator_context:
        &'input ReviewedLocalOperatorCooperativeCustodyProfileContextV1<'holders>,
    pub(super) reviewed_local_operator_profile:
        CanonicalReviewedLocalOperatorCooperativeCustodyProfileV1,
    pub(super) delivery_locator: CanonicalReviewedFreshCredentialSlotLocatorV1,
    pub(super) delivery_binding: CanonicalFreshCredentialDeliveryBindingV1,
}

/// Inputs derived from exact canonical config before entering the actor
/// thread. No local-egress fact is accepted from the caller.
struct PmDeniedSelectedEgressActorStartup {
    canonical_config: CanonicalTrialConfig,
    online_policy: CanonicalOnlinePolicyV2,
    online_authorization: CanonicalOnlineAuthorizationV2,
    reviewed_destination_profile: CanonicalReviewedProductionDestinationProfileV1,
    reviewed_nonsecret_profile_path: PathBuf,
    prepared_local_operator_custody: PreparedUnopenedLocalOperatorFreshCredentialCustodyV1,
    position_scope: PmDataApiPositionScope,
    scope: PmWireScope,
    parser_config: PmBookParserConfig,
    maximum_fact_age: Duration,
}

impl PmDeniedSelectedEgressActorStartup {
    fn from_exact_config(
        inputs: PmDeniedSelectedEgressReviewedInputConjunction<'_, '_>,
    ) -> Result<Self, PmDeniedSelectedEgressActorError> {
        let PmDeniedSelectedEgressReviewedInputConjunction {
            config,
            online_policy,
            online_authorization,
            reviewed_destination_profile,
            reviewed_nonsecret_profile_path,
            reviewed_local_operator_context,
            reviewed_local_operator_profile,
            delivery_locator,
            delivery_binding,
        } = inputs;
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
        let joined = &reviewed_local_operator_context.phase_a_eligibility_context;
        if joined.v1_config.canonical_sha256() != config.canonical_sha256()
            || joined.v1_config.canonical_length() != config.canonical_length()
            || joined.v1_config.fingerprint() != config.fingerprint()
            || joined.online_policy_v2.canonical_sha256() != online_policy.canonical_sha256()
            || joined.online_policy_v2.canonical_length() != online_policy.canonical_length()
            || joined.online_policy_v2.fingerprint() != online_policy.fingerprint()
            || joined.online_authorization_v2.canonical_sha256()
                != online_authorization.canonical_sha256()
            || joined.online_authorization_v2.canonical_length()
                != online_authorization.canonical_length()
            || joined.online_authorization_v2.fingerprint() != online_authorization.fingerprint()
            || joined.reviewed_production_destination_v1.canonical_sha256()
                != reviewed_destination_profile.canonical_sha256()
            || joined.reviewed_production_destination_v1.canonical_length()
                != reviewed_destination_profile.canonical_length()
            || joined.reviewed_production_destination_v1.fingerprint()
                != reviewed_destination_profile.fingerprint()
            || joined
                .reviewed_fresh_credential_slot_locator_v1
                .canonical_sha256()
                != delivery_locator.canonical_sha256()
            || joined
                .reviewed_fresh_credential_slot_locator_v1
                .canonical_length()
                != delivery_locator.canonical_length()
            || joined
                .reviewed_fresh_credential_slot_locator_v1
                .fingerprint()
                != delivery_locator.fingerprint()
            || joined
                .fresh_credential_delivery_binding_v1
                .canonical_sha256()
                != delivery_binding.canonical_sha256()
            || joined
                .fresh_credential_delivery_binding_v1
                .canonical_length()
                != delivery_binding.canonical_length()
            || joined.fresh_credential_delivery_binding_v1.fingerprint()
                != delivery_binding.fingerprint()
        {
            return Err(PmDeniedSelectedEgressActorError::ReviewedLocalCooperativeCustodyBinding);
        }
        let (retained_delivery_evidence, whole_delivery_load_token) =
            bind_fresh_credential_delivery_binding_v1(
                config,
                &online_policy,
                &online_authorization,
                delivery_locator,
                delivery_binding,
            )
            .map_err(|_| {
                PmDeniedSelectedEgressActorError::ReviewedLocalCooperativeCustodyBinding
            })?;
        let prepared_local_operator_custody =
            prepare_unopened_local_operator_fresh_credential_custody_v1(
                reviewed_local_operator_context,
                reviewed_local_operator_profile,
                whole_delivery_load_token,
                retained_delivery_evidence,
            )
            .map_err(|_| {
                PmDeniedSelectedEgressActorError::ReviewedLocalCooperativeCustodyBinding
            })?;
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
        let signer = EoaAddress::parse(&value.account.signer)
            .map_err(|_| PmDeniedSelectedEgressActorError::InvalidPositionScope)?;
        let condition = PmConditionId::parse(&value.market.condition_id)
            .map_err(|_| PmDeniedSelectedEgressActorError::InvalidPositionScope)?;
        let market = PmMarketId::parse(&value.market.question_id)
            .map_err(|_| PmDeniedSelectedEgressActorError::InvalidPositionScope)?;
        let token = PmTokenId::new(
            U256::from_str(&value.market.token_id)
                .map_err(|_| PmDeniedSelectedEgressActorError::InvalidPositionScope)?,
        )
        .map_err(|_| PmDeniedSelectedEgressActorError::InvalidPositionScope)?;
        let scope = PmWireScope::new(condition, market, token);
        let tick = PmTick::parse_decimal(&value.order.tick)
            .map_err(|_| PmDeniedSelectedEgressActorError::InvalidPositionScope)?;
        let minimum = PmQuantity::parse_decimal(&value.order.minimum_order_size)
            .map_err(|_| PmDeniedSelectedEgressActorError::InvalidPositionScope)?;
        if value.account.chain_id != 137
            || value.account.signature_type != 1
            || signer.as_core() == proxy_funder
        {
            return Err(PmDeniedSelectedEgressActorError::AuthorizationConfigBinding);
        }
        let parser_config = PmBookParserConfig::new_condition_bound(
            scope,
            tick,
            minimum,
            matches!(value.market.domain, TrialDomain::NegativeRisk),
        );
        let maximum_fact_age_ms = value
            .time_limits
            .maximum_preflight_observation_age_ms
            .min(online_policy.value().maximum_observation_age_ms);

        Ok(Self {
            canonical_config: config.clone(),
            online_policy,
            online_authorization,
            reviewed_destination_profile,
            reviewed_nonsecret_profile_path: reviewed_nonsecret_profile_path.to_path_buf(),
            prepared_local_operator_custody,
            position_scope: PmDataApiPositionScope::new(proxy_funder, condition, token),
            scope,
            parser_config,
            maximum_fact_age: Duration::from_millis(maximum_fact_age_ms),
        })
    }
}

/// Six fixed TLS peer selections derived solely from the exact canonical
/// reviewed destination profile.
struct PmReviewedFixedTlsPeerBundle {
    geoblock_https: PmFixedTlsPeerSelection,
    clob_https: PmFixedTlsPeerSelection,
    status_https: PmFixedTlsPeerSelection,
    data_api_https: PmFixedTlsPeerSelection,
    polygon_rpc_https: PmFixedTlsPeerSelection,
    clob_websocket_wss: PmFixedTlsPeerSelection,
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
            clob_websocket_wss,
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

/// Synchronous actor-local construction result. It contains no loaded secret
/// and no armed task. The whole delivery token remains inside one unopened,
/// structurally reviewed aggregate until final assembly binds it to the actor
/// generation.
struct PmDeniedSelectedEgressActorColdResources {
    selected_http_bundle: PmFixedSelectedEgressHttpBundle,
    observation: PmDeferredObservationRuntimeRoles,
    prepared_local_operator_custody: PreparedUnopenedLocalOperatorFreshCredentialCustodyV1,
    reviewed_fixed_tls_peers: PmReviewedFixedTlsPeerBundle,
    selected_local_egress: PmLocalEgressSelection,
    local_egress_custody: PmLinuxEgressLocalFactCustody,
    canonical_config: CanonicalTrialConfig,
    online_authorization: CanonicalOnlineAuthorizationV2,
    online_policy: CanonicalOnlinePolicyV2,
    reviewed_destination_profile: CanonicalReviewedProductionDestinationProfileV1,
    reviewed_nonsecret_profile_path: PathBuf,
    window_wall_started: SystemTime,
    window_wall_completed: SystemTime,
    window_monotonic_started: Instant,
    window_monotonic_completed: Instant,
    maximum_fact_age: Duration,
    generation: Rc<PmSelectedEgressActorGeneration>,
}

/// All denied production resources remain inseparable and die on their source
/// thread through the consuming async shutdown implementation. The custody
/// aggregate is unopened and has no realization or projection API here.
struct PmDeniedSelectedEgressActorResources {
    pending_local_operator_custody: GenerationBoundUnopenedLocalOperatorFreshCredentialCustodyV1<
        PmSelectedEgressActorGeneration,
    >,
    observation: PmDeferredObservationRuntimeRoles,
    selected_http_bundle: PmFixedSelectedEgressHttpBundle,
    reviewed_fixed_tls_peers: PmReviewedFixedTlsPeerBundle,
    selected_local_egress: PmLocalEgressSelection,
    local_egress_custody: PmLinuxEgressLocalFactCustody,
    canonical_config: CanonicalTrialConfig,
    online_authorization: CanonicalOnlineAuthorizationV2,
    online_policy: CanonicalOnlinePolicyV2,
    reviewed_destination_profile: CanonicalReviewedProductionDestinationProfileV1,
    reviewed_nonsecret_profile_path: PathBuf,
    generation: Rc<PmSelectedEgressActorGeneration>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
struct PmSelectedActorTaskShutdownProof {
    shutdown_requested: bool,
    abort_requested: bool,
    selected_actor_unopened_custody_holder_dropped: bool,
    selected_actor_credential_loading_never_started: bool,
    selected_actor_basename_mutation_never_started: bool,
    generation_revalidated: bool,
}

impl PmSelectedActorTaskShutdownProof {
    const fn custody_clean_terminal(self) -> bool {
        self.shutdown_requested
            && !self.abort_requested
            && self.selected_actor_unopened_custody_holder_dropped
            && self.selected_actor_credential_loading_never_started
            && self.selected_actor_basename_mutation_never_started
    }

    const fn clean_terminal(self) -> bool {
        self.custody_clean_terminal() && self.generation_revalidated
    }
}

#[derive(Clone, Copy, PartialEq, Eq)]
pub(super) struct PmDeniedSelectedEgressShutdownEvidence {
    actor_thread_joined: bool,
    shutdown_requested: bool,
    abort_requested: bool,
    selected_actor_unopened_custody_holder_dropped: bool,
    selected_actor_credential_loading_never_started: bool,
    selected_actor_basename_mutation_never_started: bool,
    generation_revalidated: bool,
}

impl PmDeniedSelectedEgressShutdownEvidence {
    #[must_use]
    pub(super) const fn clean_terminal(self) -> bool {
        self.actor_thread_joined
            && self.generation_revalidated
            && self.shutdown_requested
            && !self.abort_requested
            && self.selected_actor_unopened_custody_holder_dropped
            && self.selected_actor_credential_loading_never_started
            && self.selected_actor_basename_mutation_never_started
    }
}

impl fmt::Debug for PmDeniedSelectedEgressShutdownEvidence {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str(
            "PmDeniedSelectedEgressShutdownEvidence(<thread joined; unopened custody dropped; no credential load or basename mutation; denied>)",
        )
    }
}

#[async_trait(?Send)]
trait PmSelectedEgressActorColdResources: 'static {
    type Armed: PmSelectedEgressActorResources;

    async fn assemble(
        self,
        local_set: &LocalSet,
    ) -> Result<Self::Armed, PmDeniedSelectedEgressActorError>;
}

#[async_trait(?Send)]
trait PmSelectedEgressActorResources: 'static {
    fn belongs_to_generation(&self, generation: &Rc<PmSelectedEgressActorGeneration>) -> bool;

    fn on_actor_task_enter(&self) -> Result<(), PmDeniedSelectedEgressActorError> {
        Ok(())
    }

    async fn shutdown(
        self,
    ) -> Result<PmSelectedActorTaskShutdownProof, PmDeniedSelectedEgressActorError>;
}

struct PmSelectedEgressActorState<R> {
    resources: R,
    generation: Rc<PmSelectedEgressActorGeneration>,
}

enum PmSelectedEgressActorCommand {
    Shutdown,
}

#[must_use = "selected-egress actor supervisor must be shut down and joined"]
pub(super) struct PmDeniedSelectedEgressActorSupervisor {
    commands: Option<tokio_mpsc::UnboundedSender<PmSelectedEgressActorCommand>>,
    thread: Option<
        JoinHandle<Result<PmSelectedActorTaskShutdownProof, PmDeniedSelectedEgressActorError>>,
    >,
    armed: bool,
}

impl PmDeniedSelectedEgressActorSupervisor {
    pub(super) fn spawn(
        inputs: PmDeniedSelectedEgressReviewedInputConjunction<'_, '_>,
    ) -> Result<Self, PmDeniedSelectedEgressActorError> {
        let startup = PmDeniedSelectedEgressActorStartup::from_exact_config(inputs)?;
        spawn_selected_egress_actor_thread(move |generation| {
            build_production_cold_resources(startup, generation)
        })
    }

    pub(super) fn shutdown_and_join(
        mut self,
    ) -> Result<PmDeniedSelectedEgressShutdownEvidence, PmDeniedSelectedEgressActorError> {
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
        let actor = joined.map_err(|_| PmDeniedSelectedEgressActorError::ActorThreadPanicked)??;
        if !actor.clean_terminal() {
            return if !actor.custody_clean_terminal() {
                Err(PmDeniedSelectedEgressActorError::CustodyShutdownAbnormal)
            } else {
                Err(PmDeniedSelectedEgressActorError::ActorThreadChanged)
            };
        }
        if !shutdown_delivered {
            return Err(PmDeniedSelectedEgressActorError::ShutdownDelivery);
        }
        let evidence = PmDeniedSelectedEgressShutdownEvidence {
            actor_thread_joined: true,
            shutdown_requested: actor.shutdown_requested,
            abort_requested: actor.abort_requested,
            selected_actor_unopened_custody_holder_dropped: actor
                .selected_actor_unopened_custody_holder_dropped,
            selected_actor_credential_loading_never_started: actor
                .selected_actor_credential_loading_never_started,
            selected_actor_basename_mutation_never_started: actor
                .selected_actor_basename_mutation_never_started,
            generation_revalidated: actor.generation_revalidated,
        };
        debug_assert!(evidence.clean_terminal());
        Ok(evidence)
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

fn build_production_cold_resources(
    startup: PmDeniedSelectedEgressActorStartup,
    generation: Rc<PmSelectedEgressActorGeneration>,
) -> Result<PmDeniedSelectedEgressActorColdResources, PmDeniedSelectedEgressActorError> {
    generation.revalidate()?;
    let PmDeniedSelectedEgressActorStartup {
        canonical_config,
        online_policy,
        online_authorization,
        reviewed_destination_profile,
        reviewed_nonsecret_profile_path,
        prepared_local_operator_custody,
        position_scope,
        scope,
        parser_config,
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
    let (selected_local_egress, reviewed_fixed_tls_peers, selected_http_bundle, observation) = {
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
        // five credential-free HTTP constructors plus the selected public
        // observation constructors.
        let bundle =
            PmFixedSelectedEgressHttpBundle::build(&selection, &fixed_peers, position_scope)?;
        let public_ws_bounds = PmPublicWsBounds::new(
            FIXED_HTTP_CONNECT_TIMEOUT,
            FIXED_WS_IDLE_TIMEOUT,
            FIXED_WS_PONG_TIMEOUT,
            MAX_PUBLIC_WS_FRAME_BYTES,
            FIXED_WS_RECONNECT_ATTEMPTS,
            FIXED_WS_RECONNECT_BACKOFF,
            FIXED_WS_EVENT_CHANNEL_CAPACITY,
            ConnectionEpoch::new(1),
        )
        .map_err(|_| PmDeniedSelectedEgressActorError::ObservationConfiguration)?;
        let public_ws_config = PmPublicWsConfig::production(scope, public_ws_bounds)
            .map_err(|_| PmDeniedSelectedEgressActorError::ObservationConfiguration)?;
        let observation_owner = PmPublicObservationWithDeferredMutationClockOwner::
            production_on_fixed_tls_peer_and_selected_local_egress(
                FIXED_HTTP_CONNECT_TIMEOUT,
                FIXED_HTTP_REQUEST_TIMEOUT,
                parser_config,
                public_ws_config,
                fixed_peers.clob_https.clone(),
                selection.clone(),
                PmProductClockOwner::system(),
            )
            .map_err(|_| PmDeniedSelectedEgressActorError::PublicObservationConstruction)?;
        if observation_owner.configured_scope() != scope {
            return Err(PmDeniedSelectedEgressActorError::ObservationConfiguration);
        }
        let observation = PmDeferredObservationRuntimeRoles::from_owner(observation_owner);
        if selection.interface_name() != revalidated_local_egress.interface_name()
            || selection.local_source_ip() != revalidated_local_egress.local_source_ip()
        {
            return Err(PmDeniedSelectedEgressActorError::LocalSelection);
        }
        (selection, fixed_peers, bundle, observation)
    };
    // Close all non-secret construction drift before retaining the still-whole
    // token in cold resources. No token consumption, credential load, lease,
    // descriptor open, or remote-owner claim occurs here.
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
    generation.revalidate()?;
    Ok(PmDeniedSelectedEgressActorColdResources {
        selected_http_bundle,
        observation,
        prepared_local_operator_custody,
        reviewed_fixed_tls_peers,
        selected_local_egress,
        local_egress_custody,
        canonical_config,
        online_authorization,
        online_policy,
        reviewed_destination_profile,
        reviewed_nonsecret_profile_path,
        window_wall_started,
        window_wall_completed,
        window_monotonic_started,
        window_monotonic_completed,
        maximum_fact_age,
        generation,
    })
}

#[async_trait(?Send)]
impl PmSelectedEgressActorColdResources for PmDeniedSelectedEgressActorColdResources {
    type Armed = PmDeniedSelectedEgressActorResources;

    async fn assemble(
        self,
        _local_set: &LocalSet,
    ) -> Result<Self::Armed, PmDeniedSelectedEgressActorError> {
        let Self {
            selected_http_bundle,
            observation,
            prepared_local_operator_custody,
            reviewed_fixed_tls_peers,
            selected_local_egress,
            mut local_egress_custody,
            canonical_config,
            online_authorization,
            online_policy,
            reviewed_destination_profile,
            reviewed_nonsecret_profile_path,
            window_wall_started,
            window_wall_completed,
            window_monotonic_started,
            window_monotonic_completed,
            maximum_fact_age,
            generation,
        } = self;
        generation.revalidate()?;
        let final_local_egress = local_egress_custody
            .revalidate_for_current_runtime(
                &online_authorization,
                window_wall_started,
                window_wall_completed,
                window_monotonic_started,
                window_monotonic_completed,
                maximum_fact_age,
            )
            .map_err(|_| PmDeniedSelectedEgressActorError::LocalFactRevalidation)?;
        if selected_local_egress.interface_name() != final_local_egress.interface_name()
            || selected_local_egress.local_source_ip() != final_local_egress.local_source_ip()
        {
            return Err(PmDeniedSelectedEgressActorError::LocalSelection);
        }
        drop(final_local_egress);
        generation.revalidate()?;
        let custody_generation = Rc::clone(&generation);
        generation.revalidate()?;
        // This is deliberately the last transition: it is infallible, retains
        // the whole token unopened, and grants no way to realize or project it.
        let pending_local_operator_custody =
            prepared_local_operator_custody.bind_to_actor_generation(custody_generation);

        Ok(PmDeniedSelectedEgressActorResources {
            pending_local_operator_custody,
            observation,
            selected_http_bundle,
            reviewed_fixed_tls_peers,
            selected_local_egress,
            local_egress_custody,
            canonical_config,
            online_authorization,
            online_policy,
            reviewed_destination_profile,
            reviewed_nonsecret_profile_path,
            generation,
        })
    }
}

#[async_trait(?Send)]
impl PmSelectedEgressActorResources for PmDeniedSelectedEgressActorResources {
    fn belongs_to_generation(&self, generation: &Rc<PmSelectedEgressActorGeneration>) -> bool {
        Rc::ptr_eq(&self.generation, generation)
    }

    async fn shutdown(
        self,
    ) -> Result<PmSelectedActorTaskShutdownProof, PmDeniedSelectedEgressActorError> {
        let Self {
            pending_local_operator_custody,
            observation,
            selected_http_bundle,
            reviewed_fixed_tls_peers,
            selected_local_egress,
            local_egress_custody,
            canonical_config,
            online_authorization,
            online_policy,
            reviewed_destination_profile,
            reviewed_nonsecret_profile_path,
            generation,
        } = self;
        let generation_revalidated = generation.revalidate().is_ok();
        // Only the unopened holder is dropped. No credential owner/task,
        // credential directory/source descriptor, cooperative credential
        // lease, or credential-basename mutation exists in this actor.
        drop(pending_local_operator_custody);
        drop(observation);
        drop(selected_http_bundle);
        drop(reviewed_fixed_tls_peers);
        drop(selected_local_egress);
        drop(local_egress_custody);
        drop(canonical_config);
        drop(online_authorization);
        drop(online_policy);
        drop(reviewed_destination_profile);
        drop(reviewed_nonsecret_profile_path);
        drop(generation);
        Ok(PmSelectedActorTaskShutdownProof {
            shutdown_requested: true,
            abort_requested: false,
            selected_actor_unopened_custody_holder_dropped: true,
            selected_actor_credential_loading_never_started: true,
            selected_actor_basename_mutation_never_started: true,
            generation_revalidated,
        })
    }
}

fn spawn_selected_egress_actor_thread<C, F>(
    setup: F,
) -> Result<PmDeniedSelectedEgressActorSupervisor, PmDeniedSelectedEgressActorError>
where
    C: PmSelectedEgressActorColdResources,
    F: FnOnce(Rc<PmSelectedEgressActorGeneration>) -> Result<C, PmDeniedSelectedEgressActorError>
        + Send
        + 'static,
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
        Ok(Err(error)) => match actor_thread.join() {
            Err(_) => Err(PmDeniedSelectedEgressActorError::ActorThreadPanicked),
            Ok(Err(joined_error)) => Err(joined_error),
            Ok(Ok(_)) => Err(error),
        },
        Err(_) => match actor_thread.join() {
            Err(_) => Err(PmDeniedSelectedEgressActorError::ActorThreadPanicked),
            Ok(Err(error)) => Err(error),
            Ok(Ok(_)) => Err(PmDeniedSelectedEgressActorError::StartupAcknowledgement),
        },
    }
}

fn run_selected_egress_actor_thread<C, F>(
    setup: F,
    command_receiver: tokio_mpsc::UnboundedReceiver<PmSelectedEgressActorCommand>,
    startup_sender: &mpsc::SyncSender<Result<(), PmDeniedSelectedEgressActorError>>,
) -> Result<PmSelectedActorTaskShutdownProof, PmDeniedSelectedEgressActorError>
where
    C: PmSelectedEgressActorColdResources,
    F: FnOnce(Rc<PmSelectedEgressActorGeneration>) -> Result<C, PmDeniedSelectedEgressActorError>,
{
    let generation = Rc::new(PmSelectedEgressActorGeneration::current());
    generation.revalidate()?;
    // Build the runtime/LocalSet before setup retains the unopened reviewed
    // custody aggregate. This actor never follows the locator into a
    // credential directory, so runtime-construction failure cannot load a
    // credential or mutate a credential basename.
    let runtime = TokioRuntimeBuilder::new_current_thread()
        .enable_all()
        .build()
        .map_err(|_| PmDeniedSelectedEgressActorError::CurrentThreadRuntime)?;
    let local_set = LocalSet::new();
    let cold = setup(Rc::clone(&generation))?;
    let resources = runtime.block_on(local_set.run_until(cold.assemble(&local_set)))?;
    if generation.revalidate().is_err() || !resources.belongs_to_generation(&generation) {
        let primary = PmDeniedSelectedEgressActorError::ActorThreadChanged;
        return runtime
            .block_on(local_set.run_until(shutdown_resources_after_error(resources, primary)));
    }
    let state = PmSelectedEgressActorState {
        resources,
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
) -> Result<PmSelectedActorTaskShutdownProof, PmDeniedSelectedEgressActorError> {
    let PmSelectedEgressActorState {
        resources,
        generation,
    } = state;
    if generation.revalidate().is_err() || !resources.belongs_to_generation(&generation) {
        return shutdown_resources_after_error(
            resources,
            PmDeniedSelectedEgressActorError::ActorThreadChanged,
        )
        .await;
    }
    if let Err(primary) = resources.on_actor_task_enter() {
        return shutdown_resources_after_error(resources, primary).await;
    }
    if generation.revalidate().is_err() || !resources.belongs_to_generation(&generation) {
        return shutdown_resources_after_error(
            resources,
            PmDeniedSelectedEgressActorError::ActorThreadChanged,
        )
        .await;
    }
    if startup_sender.send(Ok(())).is_err() {
        return shutdown_resources_after_error(
            resources,
            PmDeniedSelectedEgressActorError::StartupAcknowledgement,
        )
        .await;
    }
    match commands.recv().await {
        Some(PmSelectedEgressActorCommand::Shutdown) => {
            if generation.revalidate().is_err() || !resources.belongs_to_generation(&generation) {
                shutdown_resources_after_error(
                    resources,
                    PmDeniedSelectedEgressActorError::ActorThreadChanged,
                )
                .await
            } else {
                resources.shutdown().await
            }
        }
        None => {
            shutdown_resources_after_error(
                resources,
                PmDeniedSelectedEgressActorError::CommandChannelClosed,
            )
            .await
        }
    }
}

async fn shutdown_resources_after_error<R: PmSelectedEgressActorResources>(
    resources: R,
    primary: PmDeniedSelectedEgressActorError,
) -> Result<PmSelectedActorTaskShutdownProof, PmDeniedSelectedEgressActorError> {
    match resources.shutdown().await {
        Ok(proof) if !proof.custody_clean_terminal() => {
            Err(PmDeniedSelectedEgressActorError::CustodyShutdownAbnormal)
        }
        Ok(proof) if !proof.generation_revalidated => {
            Err(PmDeniedSelectedEgressActorError::ActorThreadChanged)
        }
        Ok(_) => Err(primary),
        Err(cleanup) => Err(cleanup),
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
        clean_shutdown: bool,
        generation: Rc<PmSelectedEgressActorGeneration>,
    }

    #[async_trait(?Send)]
    impl PmSelectedEgressActorColdResources for TestResources {
        type Armed = Self;

        async fn assemble(
            self,
            _local_set: &LocalSet,
        ) -> Result<Self::Armed, PmDeniedSelectedEgressActorError> {
            Ok(self)
        }
    }

    #[async_trait(?Send)]
    impl PmSelectedEgressActorResources for TestResources {
        fn belongs_to_generation(&self, generation: &Rc<PmSelectedEgressActorGeneration>) -> bool {
            Rc::ptr_eq(&self.generation, generation)
        }

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

        async fn shutdown(
            self,
        ) -> Result<PmSelectedActorTaskShutdownProof, PmDeniedSelectedEgressActorError> {
            let clean_shutdown = self.clean_shutdown;
            drop(self);
            Ok(PmSelectedActorTaskShutdownProof {
                shutdown_requested: true,
                abort_requested: !clean_shutdown,
                selected_actor_unopened_custody_holder_dropped: clean_shutdown,
                selected_actor_credential_loading_never_started: true,
                selected_actor_basename_mutation_never_started: true,
                generation_revalidated: true,
            })
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
        let supervisor = spawn_selected_egress_actor_thread(move |generation| {
            setup_events
                .send(LifecycleEvent::Setup(
                    thread::current().id(),
                    thread::current().name().map(str::to_owned),
                ))
                .map_err(|_| PmDeniedSelectedEgressActorError::ActorTaskFailed)?;
            Ok(TestResources {
                events: setup_events,
                fail_on_enter: false,
                clean_shutdown: true,
                generation,
            })
        })
        .expect("test actor must start");
        let evidence = supervisor
            .shutdown_and_join()
            .expect("shutdown must be delivered and the OS thread joined");
        assert!(evidence.clean_terminal());
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
        let result = spawn_selected_egress_actor_thread(move |generation| {
            setup_events
                .send(LifecycleEvent::Setup(
                    thread::current().id(),
                    thread::current().name().map(str::to_owned),
                ))
                .map_err(|_| PmDeniedSelectedEgressActorError::ActorTaskFailed)?;
            Ok(TestResources {
                events: setup_events,
                fail_on_enter: true,
                clean_shutdown: true,
                generation,
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

    #[test]
    fn post_assembly_generation_mismatch_cleans_before_startup_error() {
        let (events, received) = mpsc::channel();
        let setup_events = events.clone();
        let result = spawn_selected_egress_actor_thread(move |_generation| {
            setup_events
                .send(LifecycleEvent::Setup(
                    thread::current().id(),
                    thread::current().name().map(str::to_owned),
                ))
                .map_err(|_| PmDeniedSelectedEgressActorError::ActorTaskFailed)?;
            Ok(TestResources {
                events: setup_events,
                fail_on_enter: false,
                clean_shutdown: true,
                generation: Rc::new(PmSelectedEgressActorGeneration::current()),
            })
        });
        assert_eq!(
            result.expect_err("foreign post-assembly generation must fail closed"),
            PmDeniedSelectedEgressActorError::ActorThreadChanged,
        );
        drop(events);

        let lifecycle: Vec<_> = received.into_iter().collect();
        assert_eq!(lifecycle.len(), 2);
        assert!(matches!(lifecycle[0], LifecycleEvent::Setup(_, _)));
        assert!(matches!(lifecycle[1], LifecycleEvent::Dropped(_, _)));
    }

    #[test]
    fn ready_delivery_failure_cleans_assembled_resources() {
        let runtime = TokioRuntimeBuilder::new_current_thread()
            .enable_all()
            .build()
            .unwrap();
        let local_set = LocalSet::new();
        let (events, received) = mpsc::channel();
        let generation = Rc::new(PmSelectedEgressActorGeneration::current());
        let state = PmSelectedEgressActorState {
            resources: TestResources {
                events: events.clone(),
                fail_on_enter: false,
                clean_shutdown: true,
                generation: Rc::clone(&generation),
            },
            generation,
        };
        let (_commands, command_receiver) = tokio_mpsc::unbounded_channel();
        let (startup_sender, startup_receiver) = mpsc::sync_channel(1);
        drop(startup_receiver);

        let result = runtime.block_on(local_set.run_until(run_selected_egress_actor(
            state,
            command_receiver,
            startup_sender,
        )));
        assert_eq!(
            result.expect_err("disconnected Ready receiver must fail closed"),
            PmDeniedSelectedEgressActorError::StartupAcknowledgement,
        );
        drop(events);
        let lifecycle: Vec<_> = received.into_iter().collect();
        assert_eq!(lifecycle.len(), 2);
        assert!(matches!(lifecycle[0], LifecycleEvent::Task(_, _)));
        assert!(matches!(lifecycle[1], LifecycleEvent::Dropped(_, _)));
    }

    #[test]
    fn actor_custody_error_precedes_shutdown_delivery_error() {
        let (events, _received) = mpsc::channel();
        let mut supervisor = spawn_selected_egress_actor_thread(move |generation| {
            Ok(TestResources {
                events,
                fail_on_enter: false,
                clean_shutdown: false,
                generation,
            })
        })
        .expect("test actor must start");
        drop(supervisor.commands.take());

        assert_eq!(
            supervisor
                .shutdown_and_join()
                .expect_err("abnormal custody shutdown must outrank failed Shutdown delivery"),
            PmDeniedSelectedEgressActorError::CustodyShutdownAbnormal,
        );
    }

    #[test]
    fn shutdown_proof_is_send_and_payload_free() {
        fn assert_send<T: Send>() {}
        assert_send::<PmSelectedActorTaskShutdownProof>();
        assert!(!std::mem::needs_drop::<PmSelectedActorTaskShutdownProof>());
        let evidence = PmDeniedSelectedEgressShutdownEvidence {
            actor_thread_joined: true,
            shutdown_requested: true,
            abort_requested: false,
            selected_actor_unopened_custody_holder_dropped: true,
            selected_actor_credential_loading_never_started: true,
            selected_actor_basename_mutation_never_started: true,
            generation_revalidated: true,
        };
        assert!(evidence.clean_terminal());
        assert_eq!(
            format!("{evidence:?}"),
            "PmDeniedSelectedEgressShutdownEvidence(<thread joined; unopened custody dropped; no credential load or basename mutation; denied>)",
        );
    }
}
