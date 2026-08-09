//! Runner-private, non-authoritative join of the current PM-T2 Phase-A read
//! evidence.
//!
//! This module deliberately stops one capability level below a live permit.
//! It can join already-collected, production-origin read evidence into one
//! move-only candidate, but it does not own the source calls, the outer
//! `SystemTime` + `Instant` observation window, the current runtime/geoblock
//! witness, or the selected-egress transport. Its wall-clock span is only a
//! coherence check; it is not a shared-monotonic-clock or freshness proof.
//!
//! A successful value therefore remains `DENIED`, cannot cross the credential
//! task channel, and cannot be decomposed. A later sealed assembler must own
//! this candidate, conjunct its private candidate manifest with runtime,
//! repository, outer-window, and selected-egress evidence, and only then ask
//! the durable sidecar owner to create a Basis record. That assembler must
//! also parse the retained exact market end time and prove it is strictly
//! later than the V2 authorization's `cleanup_not_after_utc`; this slice does
//! not own the V2 authorization and cannot close that end-window gate.

use std::{collections::HashSet, fmt, marker::PhantomData, rc::Rc, str::FromStr as _};

use reap_pm_controlled_trial::{
    CanonicalOnlinePolicyV2, CanonicalTrialConfig, OfflineAuthorizationState, TrialDomain,
    TrialPhase, TrialSide, verify_online_policy_v2,
};
use reap_pm_core::{
    EvmAddress, PmAssetId, PmBookPoint, PmBookTop, PmChainId, PmConditionId, PmFillId, PmMarketId,
    PmPrice, PmQuantity, PmSpenderDomain, PmSpenderRequirement, PmTick, PmTokenId, PmVenueOrderId,
    U256,
};
use reap_polymarket_auth::{EoaAddress, FixedOrderId};
use reap_polymarket_chain_source::{
    PmPolygonExchangeSpender, PmProductionPolygonFinalizedAuthorizationCut,
};
use reap_polymarket_live_adapter::{
    PmAccountAsset, PmProductionClobLivenessHealthObservation,
    PmProductionStatusAnnouncementObservation, PmStatusComponentState, PmStatusPageState,
    PmUserWsDisconnectReason, PmUserWsEdgeClock,
};
use reap_polymarket_public_source::{
    PmConfiguredTokenPosition, PmProductionDataApiPositionObservation,
};
use reap_polymarket_wire::{PmBookMarketBinding, PmWireScope};
use sha2::{Digest as _, Sha256};
use thiserror::Error;

use super::{
    private_reads::{PmFreshAuthenticatedRestCut, PmPrivateRestObservationPurpose},
    public_book::{PmPhaseAMarketProjection, PmPublicBookLease},
    user_stream::{FinalCutBusinessBasis, PmUserOnlinePreflightLease},
};

const CANDIDATE_MANIFEST_SCHEMA: u32 = 1;
const CANDIDATE_MANIFEST_DOMAIN: &[u8] =
    b"reap.pm-t2.runner.online-preflight.partial-source-candidate.v1\0";
const CANDIDATE_MANIFEST_FINGERPRINT_DOMAIN: &[u8] =
    b"reap.pm-t2.runner.online-preflight.partial-source-candidate-fingerprint.v1\0";

/// Explicit reason this slice cannot expose a production collector or permit.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Error)]
pub(super) enum PmOnlinePreflightCollectorBlocker {
    #[error(
        "online preflight remains denied until one sealed coordinator owns the outer monotonic window, runtime/geoblock/repository checks, all source calls, and selected-egress final rechecks"
    )]
    OuterWindowRuntimeAndSelectedEgressNotIntegrated,
}

/// There is intentionally no positive collector constructor in this slice.
#[must_use]
pub(super) const fn production_collector_blocker() -> PmOnlinePreflightCollectorBlocker {
    PmOnlinePreflightCollectorBlocker::OuterWindowRuntimeAndSelectedEgressNotIntegrated
}

/// Closed mismatch classes for the conservative first online profile.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Error)]
pub(super) enum PmOnlinePreflightMismatch {
    #[error("the candidate is not the exact Phase-A BUY-at-live-minimum profile")]
    PhaseSideOrMinimum,
    #[error("the live market identity or configured outcome does not match the exact config")]
    MarketIdentity,
    #[error("the live market domain, chain, contracts, or exact spender set drifted")]
    MarketDomainOrContracts,
    #[error("the live market is not actively accepting orders on an enabled book")]
    MarketLifecycle,
    #[error("the live tick or minimum order size drifted")]
    MarketTickOrMinimum,
    #[error("the exact reviewed fee tuple drifted")]
    MarketFees,
    #[error("a delay, order-age, game marker, or unsupported market flag is active")]
    UnsupportedMarketFlags,
    #[error("the exact public book is not a fully-admitted, two-sided, uncrossed top")]
    BookNotTwoSided,
    #[error("the exact configured BUY price is not tick-aligned and passive")]
    BookNotPassive,
    #[error("the authenticated user/REST scope, signer, or proxy funder does not match")]
    SameAccountBinding,
    #[error("the user-stream ticket is not joined to this exact complete REST cut")]
    SameAuthorityRestJoin,
    #[error("the first-profile user stream retained current- or prior-epoch business events")]
    UserBusinessHistory,
    #[error("the first-profile user stream retained a reconnect or epoch transition")]
    UserReconnectHistory,
    #[error("the complete account-wide open-order cut is not empty")]
    OpenOrdersNotEmpty,
    #[error("historical REST trade IDs are not unique")]
    HistoricalTradeDuplicate,
    #[error("a historical REST trade is not in a terminal settlement state")]
    HistoricalTradeNonTerminal,
    #[error("the exact expected order ID already appears in historical trade associations")]
    ExpectedOrderAlreadyObserved,
    #[error("the exact signer-authenticated account is closed-only")]
    ClosedOnly,
    #[error("the two private account observations do not name the exact configured assets")]
    AccountAssetBinding,
    #[error("a private account observation retained an unscoped scalar")]
    UnscopedAccountScalar,
    #[error("the exact exchange allowance is absent from a private account observation")]
    AccountSpenderBinding,
    #[error("checked BUY reservation/loss, collateral balance, or allowance is insufficient")]
    CollateralRisk,
    #[error("the private conditional balance and monitored Data API position disagree")]
    ConditionalPositionMismatch,
    #[error("the production Data API position observation is out of exact scope or domain")]
    PositionScopeOrDomain,
    #[error("the finalized production Polygon cut is out of exact account/spender scope")]
    PolygonScope,
    #[error("the private and finalized Polygon pUSD allowances do not corroborate exactly")]
    PolygonAllowanceMismatch,
    #[error("the private conditional allowance and finalized operator approval do not agree")]
    PolygonOperatorApprovalMismatch,
    #[error("the production status page is not globally up and free of active notices")]
    StatusPage,
    #[error("the exact reviewed CLOB status component is absent, ambiguous, or non-operational")]
    StatusClobComponent,
    #[error(
        "one of the purpose-distinct status or CLOB-health observations is not production-origin"
    )]
    ProductionOperationalSource,
    #[error("source wall clocks are zero, regress arithmetically, or exceed the coherence span")]
    SourceClockCoherence,
}

#[derive(Debug, Error)]
pub(super) enum PmOnlinePreflightJoinError {
    #[error("the canonical config and online-policy V2 binding is invalid")]
    CanonicalPolicyBinding,
    #[error("the canonical config contains an invalid typed scalar")]
    CanonicalScalar,
    #[error("the configured reservation/loss tuple cannot be checked exactly")]
    CanonicalRisk,
    #[error("the monitored Data API position size is not exactly representable in protocol units")]
    PositionNumeric,
    #[error(transparent)]
    Mismatch(#[from] PmOnlinePreflightMismatch),
}

/// The partial source manifest is private to the candidate. It is not the
/// final V2 online-preflight manifest and cannot be persisted independently as
/// authority. Its canonical bytes contain only nonsecret source commitments,
/// source generations/clocks, and reviewed public identities.
struct PmOnlinePreflightCandidateManifest {
    canonical_bytes: Box<[u8]>,
    canonical_sha256: [u8; 32],
    fingerprint: [u8; 32],
    minimum_source_wall_edge_ns: u64,
    maximum_source_wall_edge_ns: u64,
    market_end_time: Box<str>,
    digests: PmOnlinePreflightCandidateSourceDigests,
}

#[derive(Clone, Copy)]
struct PmOnlinePreflightCandidateSourceDigests {
    fresh_status_announcements: [u8; 32],
    clob_ok_liveness: [u8; 32],
    same_account_closed_only: [u8; 32],
    public_book_cut: [u8; 32],
    user_account_cut: [u8; 32],
    same_authority_rest_cut: [u8; 32],
    finalized_chain_cut: [u8; 32],
    data_api_position_cut: [u8; 32],
}

/// Lifetime-bound view used by the later sealed assembler. It has no owned
/// conversion, constructor, mutation authority, or component extraction.
pub(super) struct PmOnlinePreflightCandidateManifestView<'a> {
    manifest: &'a PmOnlinePreflightCandidateManifest,
}

impl PmOnlinePreflightCandidateManifestView<'_> {
    #[must_use]
    pub(super) fn canonical_bytes(&self) -> &[u8] {
        &self.manifest.canonical_bytes
    }

    #[must_use]
    pub(super) const fn fingerprint(&self) -> [u8; 32] {
        self.manifest.fingerprint
    }

    #[must_use]
    pub(super) const fn canonical_sha256(&self) -> [u8; 32] {
        self.manifest.canonical_sha256
    }

    /// Exact extrema derived from the admitted sources. A later runtime
    /// assembler must prove both lie inside its source-owned outer window.
    #[must_use]
    pub(super) const fn minimum_source_wall_edge_ns(&self) -> u64 {
        self.manifest.minimum_source_wall_edge_ns
    }

    #[must_use]
    pub(super) const fn maximum_source_wall_edge_ns(&self) -> u64 {
        self.manifest.maximum_source_wall_edge_ns
    }

    /// Exact source lexeme only. The later assembler must parse and compare
    /// it to the exact V2 cleanup deadline held by the consumption pair.
    #[must_use]
    pub(super) fn market_end_time(&self) -> &str {
        &self.manifest.market_end_time
    }

    #[must_use]
    pub(super) const fn fresh_status_announcements_sha256(&self) -> [u8; 32] {
        self.manifest.digests.fresh_status_announcements
    }

    #[must_use]
    pub(super) const fn clob_ok_liveness_sha256(&self) -> [u8; 32] {
        self.manifest.digests.clob_ok_liveness
    }

    #[must_use]
    pub(super) const fn same_account_closed_only_sha256(&self) -> [u8; 32] {
        self.manifest.digests.same_account_closed_only
    }

    #[must_use]
    pub(super) const fn public_book_cut_sha256(&self) -> [u8; 32] {
        self.manifest.digests.public_book_cut
    }

    #[must_use]
    pub(super) const fn user_account_cut_sha256(&self) -> [u8; 32] {
        self.manifest.digests.user_account_cut
    }

    #[must_use]
    pub(super) const fn same_authority_rest_cut_sha256(&self) -> [u8; 32] {
        self.manifest.digests.same_authority_rest_cut
    }

    #[must_use]
    pub(super) const fn finalized_chain_cut_sha256(&self) -> [u8; 32] {
        self.manifest.digests.finalized_chain_cut
    }

    #[must_use]
    pub(super) const fn data_api_position_cut_sha256(&self) -> [u8; 32] {
        self.manifest.digests.data_api_position_cut
    }
}

impl fmt::Debug for PmOnlinePreflightCandidateManifestView<'_> {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("PmOnlinePreflightCandidateManifestView")
            .field("canonical_length", &self.manifest.canonical_bytes.len())
            .field("fingerprint", &"<partial-source-candidate>")
            .field("authorization", &OfflineAuthorizationState::DENIED)
            .finish()
    }
}

/// Move-only, process-local custody of every source input admitted by the
/// pure join. There is no `into_parts`, `Clone`, serialization, transport, or
/// conversion into any runtime/HMAC/dispatch binding.
#[must_use = "a denied online-preflight candidate must remain under sealed coordinator custody"]
pub(super) struct PmDeniedOnlinePreflightCandidate {
    book: PmPublicBookLease,
    rest: PmFreshAuthenticatedRestCut,
    user: PmUserOnlinePreflightLease,
    status: PmProductionStatusAnnouncementObservation,
    health: PmProductionClobLivenessHealthObservation,
    polygon: PmProductionPolygonFinalizedAuthorizationCut,
    position: PmProductionDataApiPositionObservation,
    candidate_manifest: PmOnlinePreflightCandidateManifest,
    // Keep the complete candidate on the runner's local coordinator thread.
    _not_send_or_sync: PhantomData<Rc<()>>,
}

impl PmDeniedOnlinePreflightCandidate {
    #[must_use]
    pub(super) const fn authorization(&self) -> OfflineAuthorizationState {
        OfflineAuthorizationState::DENIED
    }

    #[must_use]
    pub(super) const fn candidate_manifest(&self) -> PmOnlinePreflightCandidateManifestView<'_> {
        PmOnlinePreflightCandidateManifestView {
            manifest: &self.candidate_manifest,
        }
    }

    /// Re-run only the same pure equality/coherence join. This consumes the
    /// candidate on every outcome but still performs no live source recheck
    /// and therefore remains denied.
    pub(super) fn revalidate_non_authoritative(
        self,
        config: &CanonicalTrialConfig,
        policy: &CanonicalOnlinePolicyV2,
    ) -> Result<Self, PmOnlinePreflightJoinError> {
        let expected = ExpectedProfile::from_canonical(config, policy)?;
        let facts = JoinedFacts::from_sources(
            &expected,
            &self.book,
            &self.rest,
            &self.user,
            &self.status,
            &self.health,
            &self.polygon,
            &self.position,
        )?;
        validate_joined_facts(&expected, &facts)?;
        let candidate_manifest = build_candidate_manifest(
            config,
            policy,
            &self.book,
            &self.rest,
            &self.user,
            &self.status,
            &self.health,
            &self.polygon,
            &self.position,
            &facts,
        );
        if candidate_manifest.fingerprint != self.candidate_manifest.fingerprint
            || candidate_manifest.canonical_bytes != self.candidate_manifest.canonical_bytes
        {
            return Err(PmOnlinePreflightMismatch::SourceClockCoherence.into());
        }
        Ok(self)
    }
}

impl fmt::Debug for PmDeniedOnlinePreflightCandidate {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("PmDeniedOnlinePreflightCandidate")
            .field("sources", &"<inseparable; redacted>")
            .field("candidate_manifest", &"<private; partial>")
            .field("authorization", &OfflineAuthorizationState::DENIED)
            .field(
                "collector_blocker",
                &PmOnlinePreflightCollectorBlocker::OuterWindowRuntimeAndSelectedEgressNotIntegrated,
            )
            .finish()
    }
}

/// Pure, sealed join of already-observed evidence. The parameter types make
/// production origin explicit for status, health, Polygon, and Data API
/// observations; the user ticket separately proves the exact same credential
/// authority as the complete authenticated REST cut.
#[allow(clippy::too_many_arguments)]
pub(super) fn join_denied_online_preflight(
    config: &CanonicalTrialConfig,
    policy: &CanonicalOnlinePolicyV2,
    book: PmPublicBookLease,
    rest: PmFreshAuthenticatedRestCut,
    user: PmUserOnlinePreflightLease,
    status: PmProductionStatusAnnouncementObservation,
    health: PmProductionClobLivenessHealthObservation,
    polygon: PmProductionPolygonFinalizedAuthorizationCut,
    position: PmProductionDataApiPositionObservation,
) -> Result<PmDeniedOnlinePreflightCandidate, PmOnlinePreflightJoinError> {
    let expected = ExpectedProfile::from_canonical(config, policy)?;
    let facts = JoinedFacts::from_sources(
        &expected, &book, &rest, &user, &status, &health, &polygon, &position,
    )?;
    validate_joined_facts(&expected, &facts)?;
    let candidate_manifest = build_candidate_manifest(
        config, policy, &book, &rest, &user, &status, &health, &polygon, &position, &facts,
    );
    Ok(PmDeniedOnlinePreflightCandidate {
        book,
        rest,
        user,
        status,
        health,
        polygon,
        position,
        candidate_manifest,
        _not_send_or_sync: PhantomData,
    })
}

#[cfg_attr(test, derive(Clone))]
struct ExpectedProfile {
    scope: PmWireScope,
    signer: EoaAddress,
    funder: EvmAddress,
    outcome_label: Box<str>,
    domain: PmSpenderDomain,
    exchange: EvmAddress,
    collateral_contract: EvmAddress,
    conditional_tokens_contract: EvmAddress,
    polygon_spender: PmPolygonExchangeSpender,
    price: PmPrice,
    quantity: PmQuantity,
    tick: PmTick,
    minimum: PmQuantity,
    maker_base_fee_bps: u64,
    taker_base_fee_bps: u64,
    fee_rate: Box<str>,
    fee_exponent: Box<str>,
    reservation: U256,
    maximum_loss: U256,
    expected_order_id: FixedOrderId,
    status_component_id: Box<str>,
    status_component_name: Box<str>,
    maximum_coherence_span_ns: u64,
}

impl ExpectedProfile {
    fn from_canonical(
        config: &CanonicalTrialConfig,
        policy: &CanonicalOnlinePolicyV2,
    ) -> Result<Self, PmOnlinePreflightJoinError> {
        verify_online_policy_v2(config, policy)
            .map_err(|_| PmOnlinePreflightJoinError::CanonicalPolicyBinding)?;
        let value = config.value();
        if value.phase != TrialPhase::APlaceCancel || value.order.side != TrialSide::Buy {
            return Err(PmOnlinePreflightMismatch::PhaseSideOrMinimum.into());
        }

        let condition = PmConditionId::parse(&value.market.condition_id)
            .map_err(|_| PmOnlinePreflightJoinError::CanonicalScalar)?;
        let market = PmMarketId::parse(&value.market.question_id)
            .map_err(|_| PmOnlinePreflightJoinError::CanonicalScalar)?;
        let token = PmTokenId::new(
            U256::from_str(&value.market.token_id)
                .map_err(|_| PmOnlinePreflightJoinError::CanonicalScalar)?,
        )
        .map_err(|_| PmOnlinePreflightJoinError::CanonicalScalar)?;
        let signer = EoaAddress::parse(&value.account.signer)
            .map_err(|_| PmOnlinePreflightJoinError::CanonicalScalar)?;
        let funder = EvmAddress::parse(&value.account.funder)
            .map_err(|_| PmOnlinePreflightJoinError::CanonicalScalar)?;
        let exchange = EvmAddress::parse(&value.market.exchange)
            .map_err(|_| PmOnlinePreflightJoinError::CanonicalScalar)?;
        let collateral_contract = EvmAddress::parse(&value.market.pusd_contract)
            .map_err(|_| PmOnlinePreflightJoinError::CanonicalScalar)?;
        let conditional_tokens_contract =
            EvmAddress::parse(&value.market.conditional_tokens_contract)
                .map_err(|_| PmOnlinePreflightJoinError::CanonicalScalar)?;
        let price = PmPrice::parse_decimal(&value.order.price)
            .map_err(|_| PmOnlinePreflightJoinError::CanonicalScalar)?;
        let quantity = PmQuantity::parse_decimal(&value.order.quantity)
            .map_err(|_| PmOnlinePreflightJoinError::CanonicalScalar)?;
        let tick = PmTick::parse_decimal(&value.order.tick)
            .map_err(|_| PmOnlinePreflightJoinError::CanonicalScalar)?;
        let minimum = PmQuantity::parse_decimal(&value.order.minimum_order_size)
            .map_err(|_| PmOnlinePreflightJoinError::CanonicalScalar)?;
        if quantity != minimum {
            return Err(PmOnlinePreflightMismatch::PhaseSideOrMinimum.into());
        }

        let reservation = U256::from_str(&value.order.reservation_pusd_base_units)
            .map_err(|_| PmOnlinePreflightJoinError::CanonicalRisk)?;
        let maximum_loss = U256::from_str(&value.order.maximum_loss_pusd_base_units)
            .map_err(|_| PmOnlinePreflightJoinError::CanonicalRisk)?;
        let maker_amount = U256::from_str(&value.order.maker_amount)
            .map_err(|_| PmOnlinePreflightJoinError::CanonicalRisk)?;
        if reservation.is_zero()
            || reservation != maker_amount
            || maximum_loss.checked_sub(reservation).is_err()
        {
            return Err(PmOnlinePreflightJoinError::CanonicalRisk);
        }
        let maximum_coherence_span_ns = policy
            .value()
            .maximum_observation_age_ms
            .checked_mul(1_000_000)
            .ok_or(PmOnlinePreflightJoinError::CanonicalRisk)?;
        let (domain, polygon_spender) = match value.market.domain {
            TrialDomain::Standard => (
                PmSpenderDomain::Standard,
                PmPolygonExchangeSpender::StandardV2,
            ),
            TrialDomain::NegativeRisk => (
                PmSpenderDomain::NegativeRisk,
                PmPolygonExchangeSpender::NegativeRiskV2,
            ),
        };

        Ok(Self {
            scope: PmWireScope::new(condition, market, token),
            signer,
            funder,
            outcome_label: value.market.outcome_label.as_str().into(),
            domain,
            exchange,
            collateral_contract,
            conditional_tokens_contract,
            polygon_spender,
            price,
            quantity,
            tick,
            minimum,
            maker_base_fee_bps: value.market.maker_base_fee_bps,
            taker_base_fee_bps: value.market.taker_base_fee_bps,
            fee_rate: value.market.fee_rate.as_str().into(),
            fee_exponent: value.market.fee_exponent.as_str().into(),
            reservation,
            maximum_loss,
            expected_order_id: FixedOrderId::from(
                config
                    .exact_place_public_request_identity()
                    .expected_order_id(),
            ),
            status_component_id: policy
                .value()
                .reviewed_status_clob_component
                .component_id
                .as_str()
                .into(),
            status_component_name: policy
                .value()
                .reviewed_status_clob_component
                .component_name
                .as_str()
                .into(),
            maximum_coherence_span_ns,
        })
    }
}

#[cfg_attr(test, derive(Clone))]
struct MarketFacts {
    condition: PmConditionId,
    question: PmMarketId,
    reported_condition: Option<PmConditionId>,
    book_market_binding: PmBookMarketBinding,
    configured_token: PmTokenId,
    configured_outcome_label: Box<str>,
    configured_token_membership_count: usize,
    chain: PmChainId,
    domain: PmSpenderDomain,
    exchange: EvmAddress,
    collateral_asset: PmAssetId,
    outcome_asset: PmAssetId,
    required_spenders: [PmSpenderRequirement; 2],
    lifecycle_active: bool,
    lifecycle_closed: bool,
    lifecycle_archived: bool,
    lifecycle_accepting_orders: bool,
    lifecycle_order_book_enabled: bool,
    tick: PmTick,
    minimum: PmQuantity,
    maker_base_fee_bps: u64,
    taker_base_fee_bps: u64,
    fee_rate: Option<Box<str>>,
    fee_exponent: Option<Box<str>>,
    fee_taker_only: Option<bool>,
    seconds_delay: u64,
    reported_seconds_delay: Option<u64>,
    take_only_delay_enabled: Option<bool>,
    cancel_book_on_start: Option<bool>,
    minimum_order_age_seconds: u64,
    accepting_orders_reported: Option<bool>,
    rfq_enabled: Option<bool>,
    bonding_curve_enabled: Option<bool>,
    game_start_time_present: bool,
    end_time: Box<str>,
}

#[cfg_attr(test, derive(Clone))]
struct BookFacts {
    source_fully_admitted: bool,
    top: PmBookTop,
}

#[cfg_attr(test, derive(Clone))]
struct UserFacts {
    scope: PmWireScope,
    signer: EoaAddress,
    proxy_maker: EvmAddress,
    same_rest_allocation: bool,
    user_activity_generation: u64,
    rest_activity_generation: u64,
    initial_connection_epoch: u64,
    current_connection_epoch: u64,
    reconnect_count: usize,
    reconnect_history_count: usize,
    all_business_event_count: usize,
    current_business_event_count: usize,
    rest_backed_quiet_basis: bool,
    ticket_open_order_rows: usize,
    ticket_trade_rows: usize,
}

#[cfg_attr(test, derive(Clone))]
struct TradeFacts {
    id: PmFillId,
    status: Box<str>,
    order_id: Option<PmVenueOrderId>,
    taker_order_id: Option<PmVenueOrderId>,
    maker_order_ids: Box<[PmVenueOrderId]>,
}

#[cfg_attr(test, derive(Clone))]
struct AccountFacts {
    closed_only: bool,
    collateral_asset: PmAccountAsset,
    conditional_asset: PmAccountAsset,
    collateral_unscoped_scalar: bool,
    conditional_unscoped_scalar: bool,
    collateral_balance: U256,
    collateral_exchange_allowance: Option<U256>,
    conditional_balance: U256,
    conditional_exchange_allowance: Option<U256>,
    open_order_page_count: usize,
    open_order_row_count: usize,
    open_order_projection_count: usize,
    trade_page_count: usize,
    trade_row_count: usize,
    trades: Vec<TradeFacts>,
}

#[cfg_attr(test, derive(Clone))]
struct PolygonFacts {
    chain_id: u64,
    signer: EvmAddress,
    owner: EvmAddress,
    spender: PmPolygonExchangeSpender,
    spender_address: EvmAddress,
    pusd_allowance: U256,
    conditional_operator_approved: bool,
    finalized_block_number: u64,
    finalized_block_timestamp: u64,
    observed_unix_seconds: u64,
}

#[cfg_attr(test, derive(Clone))]
struct PositionFacts {
    proxy_funder: EvmAddress,
    condition: PmConditionId,
    token: PmTokenId,
    pages_observed: u8,
    configured_row_present: bool,
    configured_size: U256,
    configured_row_asset: Option<PmTokenId>,
    configured_outcome: Option<Box<str>>,
    configured_negative_risk: Option<bool>,
}

#[cfg_attr(test, derive(Clone))]
struct OperationalFacts {
    production_status: bool,
    production_health: bool,
    page_state: PmStatusPageState,
    active_incident_count: usize,
    active_maintenance_count: usize,
    reviewed_component_match_count: usize,
    reviewed_component_state: Option<PmStatusComponentState>,
}

#[cfg_attr(test, derive(Clone))]
struct JoinedFacts {
    market: MarketFacts,
    book: BookFacts,
    user: UserFacts,
    account: AccountFacts,
    polygon: PolygonFacts,
    position: PositionFacts,
    operational: OperationalFacts,
    source_wall_clocks_ns: Vec<u64>,
}

impl JoinedFacts {
    #[allow(clippy::too_many_arguments)]
    fn from_sources(
        expected: &ExpectedProfile,
        book: &PmPublicBookLease,
        rest: &PmFreshAuthenticatedRestCut,
        user: &PmUserOnlinePreflightLease,
        status: &PmProductionStatusAnnouncementObservation,
        health: &PmProductionClobLivenessHealthObservation,
        polygon: &PmProductionPolygonFinalizedAuthorizationCut,
        position: &PmProductionDataApiPositionObservation,
    ) -> Result<Self, PmOnlinePreflightJoinError> {
        let market = market_facts(book.phase_a_market(), expected.scope.token());
        let rest_backed_quiet_basis = matches!(
            user.business_basis(),
            FinalCutBusinessBasis::RestBackedNoStreamEvents { .. }
        );
        let user_facts = UserFacts {
            scope: user.scope(),
            signer: user.signer(),
            proxy_maker: user.proxy_maker(),
            same_rest_allocation: user.matches_fresh_rest_cut(rest),
            user_activity_generation: user.admitted_activity_generation(),
            rest_activity_generation: rest.activity_generation(),
            initial_connection_epoch: user.initial_connection_epoch().value(),
            current_connection_epoch: user.current_connection_epoch().value(),
            reconnect_count: usize::from(user.reconnect_count()),
            reconnect_history_count: user.reconnect_history().len(),
            all_business_event_count: user.business_events().len(),
            current_business_event_count: user.current_epoch_business_events().len(),
            rest_backed_quiet_basis,
            ticket_open_order_rows: user.open_order_rows(),
            ticket_trade_rows: user.trade_rows(),
        };

        let trades = rest
            .trades()
            .rows()
            .iter()
            .map(|trade| TradeFacts {
                id: trade.id(),
                status: trade.status().into(),
                order_id: trade.order_id(),
                taker_order_id: trade.taker_order_id(),
                maker_order_ids: trade
                    .maker_orders()
                    .iter()
                    .map(|maker| maker.order_id())
                    .collect::<Vec<_>>()
                    .into_boxed_slice(),
            })
            .collect();
        let account = AccountFacts {
            closed_only: rest.closed_only().status().closed_only(),
            collateral_asset: rest.collateral().asset(),
            conditional_asset: rest.conditional().asset(),
            collateral_unscoped_scalar: rest.collateral().unscoped_scalar_present(),
            conditional_unscoped_scalar: rest.conditional().unscoped_scalar_present(),
            collateral_balance: rest.collateral().balance(),
            collateral_exchange_allowance: rest.collateral().exact_allowance(expected.exchange),
            conditional_balance: rest.conditional().balance(),
            conditional_exchange_allowance: rest.conditional().exact_allowance(expected.exchange),
            open_order_page_count: rest.open_orders().page_count(),
            open_order_row_count: rest.open_orders().row_count(),
            open_order_projection_count: rest.open_orders().rows().len(),
            trade_page_count: rest.trades().page_count(),
            trade_row_count: rest.trades().row_count(),
            trades,
        };

        let polygon_scope = polygon.scope();
        let polygon_account = polygon_scope.account_scope();
        let polygon_block = polygon.block();
        let polygon_observed_wall_ns = polygon
            .observed_clock()
            .unix_seconds()
            .checked_mul(1_000_000_000)
            .ok_or(PmOnlinePreflightMismatch::SourceClockCoherence)?;
        let polygon_facts = PolygonFacts {
            chain_id: polygon_account.chain().value(),
            signer: polygon_account.signer().address(),
            owner: polygon.scope().owner(),
            spender: polygon_scope.spender(),
            spender_address: polygon_scope.spender().address(),
            pusd_allowance: polygon.pusd_allowance(),
            conditional_operator_approved: polygon.conditional_tokens_approval().is_approved(),
            finalized_block_number: polygon_block.number(),
            finalized_block_timestamp: polygon_block.timestamp(),
            observed_unix_seconds: polygon.observed_clock().unix_seconds(),
        };

        let position_scope = position.scope();
        let position_completed_wall_ns = position
            .completed_clock()
            .unix_milliseconds()
            .checked_mul(1_000_000)
            .ok_or(PmOnlinePreflightMismatch::SourceClockCoherence)?;
        let (
            configured_row_present,
            configured_size,
            configured_row_asset,
            configured_outcome,
            configured_negative_risk,
        ) = match position.configured_token() {
            PmConfiguredTokenPosition::Absent => (false, U256::ZERO, None, None, None),
            PmConfiguredTokenPosition::Present(row) => (
                true,
                row.size_protocol_units()
                    .map_err(|_| PmOnlinePreflightJoinError::PositionNumeric)?,
                Some(row.asset()),
                Some(row.outcome().into()),
                Some(row.negative_risk()),
            ),
        };
        let position_facts = PositionFacts {
            proxy_funder: position_scope.proxy_funder(),
            condition: position_scope.condition(),
            token: position_scope.configured_token(),
            pages_observed: position.pages_observed(),
            configured_row_present,
            configured_size,
            configured_row_asset,
            configured_outcome,
            configured_negative_risk,
        };

        let matching_components = status
            .components()
            .iter()
            .filter(|component| {
                component.id() == expected.status_component_id.as_ref()
                    && component.name() == expected.status_component_name.as_ref()
            })
            .collect::<Vec<_>>();
        let operational = OperationalFacts {
            production_status: true,
            production_health: true,
            page_state: status.page().state(),
            active_incident_count: status.active_incidents().len(),
            active_maintenance_count: status.active_maintenances().len(),
            reviewed_component_match_count: matching_components.len(),
            reviewed_component_state: matching_components
                .first()
                .map(|component| component.state()),
        };

        let mut source_wall_clocks_ns = vec![
            book.metadata_source_receive_clock().local_wall_receive_ns(),
            book.metadata_control_begin_clock().local_wall_receive_ns(),
            book.metadata_control_complete_clock()
                .local_wall_receive_ns(),
            book.book_receive_clock().local_wall_receive_ns(),
            book.checked_at_control_clock().local_wall_receive_ns(),
            book.heartbeat().local_wall_receive_ns(),
            rest.closed_only().receive_clock().local_wall_receive_ns(),
            rest.collateral().receive_clock().local_wall_receive_ns(),
            rest.conditional().receive_clock().local_wall_receive_ns(),
            rest.open_orders().receive_clock().local_wall_receive_ns(),
            rest.trades().receive_clock().local_wall_receive_ns(),
            status.summary_receive_clock().local_wall_receive_ns(),
            status.components_receive_clock().local_wall_receive_ns(),
            health.receive_clock().local_wall_receive_ns(),
            polygon_observed_wall_ns,
            position_completed_wall_ns,
        ];
        source_wall_clocks_ns.extend(
            rest.server_times()
                .iter()
                .map(|time| time.receive_clock().local_wall_receive_ns()),
        );
        append_current_user_source_wall_clocks(
            &mut source_wall_clocks_ns,
            user.connection_open_clock(),
            user.subscription_clock(),
            user.ping_clock(),
            user.correlated_pong_clock(),
        );
        for reconnect in user.reconnect_history() {
            append_reconnect_user_source_wall_clocks(
                &mut source_wall_clocks_ns,
                reconnect.connection_open_generation(),
                reconnect.connection_open_clock(),
                reconnect.subscription_generation(),
                reconnect.subscription_clock(),
                reconnect.latest_ping_generation(),
                reconnect.latest_ping_clock(),
                reconnect.correlated_pong_generation(),
                reconnect.correlated_pong_clock(),
                reconnect.retirement_clock(),
                reconnect.reconnect_clock(),
            )?;
        }

        Ok(Self {
            market,
            book: BookFacts {
                source_fully_admitted: book.source_was_fully_admitted(),
                top: book.ready_top(),
            },
            user: user_facts,
            account,
            polygon: polygon_facts,
            position: position_facts,
            operational,
            source_wall_clocks_ns,
        })
    }
}

fn append_current_user_source_wall_clocks(
    clocks: &mut Vec<u64>,
    connection_open: PmUserWsEdgeClock,
    subscription: PmUserWsEdgeClock,
    ping: PmUserWsEdgeClock,
    correlated_pong: PmUserWsEdgeClock,
) {
    clocks.extend(
        [connection_open, subscription, ping, correlated_pong]
            .into_iter()
            .map(PmUserWsEdgeClock::local_wall_receive_ns),
    );
}

#[allow(clippy::too_many_arguments)]
fn append_reconnect_user_source_wall_clocks(
    clocks: &mut Vec<u64>,
    connection_open_generation: Option<u64>,
    connection_open: Option<PmUserWsEdgeClock>,
    subscription_generation: Option<u64>,
    subscription: Option<PmUserWsEdgeClock>,
    ping_generation: Option<u64>,
    ping: Option<PmUserWsEdgeClock>,
    correlated_pong_generation: Option<u64>,
    correlated_pong: Option<PmUserWsEdgeClock>,
    retirement: PmUserWsEdgeClock,
    reconnect_scheduled: PmUserWsEdgeClock,
) -> Result<(), PmOnlinePreflightMismatch> {
    append_optional_user_source_wall_clock(clocks, connection_open_generation, connection_open)?;
    append_optional_user_source_wall_clock(clocks, subscription_generation, subscription)?;
    append_optional_user_source_wall_clock(clocks, ping_generation, ping)?;
    append_optional_user_source_wall_clock(clocks, correlated_pong_generation, correlated_pong)?;
    clocks.push(retirement.local_wall_receive_ns());
    clocks.push(reconnect_scheduled.local_wall_receive_ns());
    Ok(())
}

fn append_optional_user_source_wall_clock(
    clocks: &mut Vec<u64>,
    generation: Option<u64>,
    clock: Option<PmUserWsEdgeClock>,
) -> Result<(), PmOnlinePreflightMismatch> {
    match (generation, clock) {
        (Some(_), Some(clock)) => {
            clocks.push(clock.local_wall_receive_ns());
            Ok(())
        }
        (None, None) => Ok(()),
        _ => Err(PmOnlinePreflightMismatch::SourceClockCoherence),
    }
}

fn market_facts(market: &PmPhaseAMarketProjection, configured_token: PmTokenId) -> MarketFacts {
    let lifecycle = market.lifecycle();
    MarketFacts {
        condition: market.condition(),
        question: market.question(),
        reported_condition: market.reported_condition(),
        book_market_binding: market.book_market_binding(),
        configured_token: market.configured_outcome().token(),
        configured_outcome_label: market.configured_outcome().label().as_str().into(),
        configured_token_membership_count: market
            .token_membership()
            .iter()
            .filter(|outcome| outcome.token() == configured_token)
            .count(),
        chain: market.chain(),
        domain: market.spender_domain(),
        exchange: market.exchange(),
        collateral_asset: market.collateral_asset(),
        outcome_asset: market.outcome_asset(),
        required_spenders: market.required_spenders(),
        lifecycle_active: lifecycle.active(),
        lifecycle_closed: lifecycle.closed(),
        lifecycle_archived: lifecycle.archived(),
        lifecycle_accepting_orders: lifecycle.accepting_orders(),
        lifecycle_order_book_enabled: lifecycle.order_book_enabled(),
        tick: market.tick(),
        minimum: market.minimum_order_size(),
        maker_base_fee_bps: market.maker_base_fee_bps(),
        taker_base_fee_bps: market.taker_base_fee_bps(),
        fee_rate: market.fee_rate().map(|value| value.as_str().into()),
        fee_exponent: market.fee_exponent().map(|value| value.as_str().into()),
        fee_taker_only: market.fee_taker_only(),
        seconds_delay: market.seconds_delay(),
        reported_seconds_delay: market.reported_seconds_delay(),
        take_only_delay_enabled: market.take_only_delay_enabled_reported(),
        cancel_book_on_start: market.cancel_book_on_start(),
        minimum_order_age_seconds: market.minimum_order_age_seconds(),
        accepting_orders_reported: market.accepting_orders_reported(),
        rfq_enabled: market.rfq_enabled(),
        bonding_curve_enabled: market.bonding_curve_enabled(),
        game_start_time_present: market.game_start_time_present(),
        end_time: market.end_time().as_str().into(),
    }
}

fn validate_joined_facts(
    expected: &ExpectedProfile,
    facts: &JoinedFacts,
) -> Result<(), PmOnlinePreflightMismatch> {
    if expected.quantity != expected.minimum {
        return Err(PmOnlinePreflightMismatch::PhaseSideOrMinimum);
    }
    validate_market(expected, &facts.market)?;
    validate_book(expected, &facts.book)?;
    validate_user_and_rest(expected, &facts.user, &facts.account)?;
    validate_account_sources(expected, &facts.account, &facts.polygon, &facts.position)?;
    validate_operational(&facts.operational)?;
    validate_source_clock_coherence(expected, &facts.source_wall_clocks_ns)
}

fn validate_market(
    expected: &ExpectedProfile,
    market: &MarketFacts,
) -> Result<(), PmOnlinePreflightMismatch> {
    if market.condition != expected.scope.condition()
        || market.question != expected.scope.market()
        || market
            .reported_condition
            .is_some_and(|reported| reported != expected.scope.condition())
        || market.book_market_binding != PmBookMarketBinding::ConditionId
        || market.configured_token != expected.scope.token()
        || market.configured_outcome_label.as_ref() != expected.outcome_label.as_ref()
        || market.configured_token_membership_count != 1
    {
        return Err(PmOnlinePreflightMismatch::MarketIdentity);
    }

    let collateral_asset = PmAssetId::collateral(expected.collateral_contract);
    let outcome_asset =
        PmAssetId::outcome(expected.conditional_tokens_contract, expected.scope.token());
    let expected_spenders = [
        PmSpenderRequirement::new(
            PmChainId::new(137).expect("frozen nonzero Polygon chain"),
            expected.exchange,
            expected.domain,
            collateral_asset,
        ),
        PmSpenderRequirement::new(
            PmChainId::new(137).expect("frozen nonzero Polygon chain"),
            expected.exchange,
            expected.domain,
            outcome_asset,
        ),
    ];
    if market.chain.value() != 137
        || market.domain != expected.domain
        || market.exchange != expected.exchange
        || market.collateral_asset != collateral_asset
        || market.outcome_asset != outcome_asset
        || expected_spenders
            .iter()
            .any(|required| !market.required_spenders.contains(required))
        || market
            .required_spenders
            .iter()
            .any(|required| !expected_spenders.contains(required))
    {
        return Err(PmOnlinePreflightMismatch::MarketDomainOrContracts);
    }

    if !market.lifecycle_active
        || market.lifecycle_closed
        || market.lifecycle_archived
        || !market.lifecycle_accepting_orders
        || !market.lifecycle_order_book_enabled
        || market.accepting_orders_reported != Some(true)
    {
        return Err(PmOnlinePreflightMismatch::MarketLifecycle);
    }
    if market.tick != expected.tick || market.minimum != expected.minimum {
        return Err(PmOnlinePreflightMismatch::MarketTickOrMinimum);
    }
    if market.maker_base_fee_bps != expected.maker_base_fee_bps
        || market.taker_base_fee_bps != expected.taker_base_fee_bps
        || market.fee_rate.as_deref() != Some(expected.fee_rate.as_ref())
        || market.fee_exponent.as_deref() != Some(expected.fee_exponent.as_ref())
        || market.fee_taker_only != Some(true)
    {
        return Err(PmOnlinePreflightMismatch::MarketFees);
    }
    if market.seconds_delay != 0
        || market
            .reported_seconds_delay
            .is_some_and(|delay| delay != 0)
        || market.take_only_delay_enabled != Some(false)
        || market.cancel_book_on_start != Some(false)
        || market.minimum_order_age_seconds != 0
        || market.rfq_enabled != Some(false)
        || market.bonding_curve_enabled != Some(false)
        || market.game_start_time_present
    {
        return Err(PmOnlinePreflightMismatch::UnsupportedMarketFlags);
    }
    Ok(())
}

fn validate_book(
    expected: &ExpectedProfile,
    book: &BookFacts,
) -> Result<(), PmOnlinePreflightMismatch> {
    let (Some(bid), Some(ask)) = (book.top.bid(), book.top.ask()) else {
        return Err(PmOnlinePreflightMismatch::BookNotTwoSided);
    };
    if !book.source_fully_admitted || bid.price() >= ask.price() {
        return Err(PmOnlinePreflightMismatch::BookNotTwoSided);
    }
    if expected.price.validate_tick(expected.tick).is_err()
        || bid.price().validate_tick(expected.tick).is_err()
        || ask.price().validate_tick(expected.tick).is_err()
        || expected.price >= ask.price()
    {
        return Err(PmOnlinePreflightMismatch::BookNotPassive);
    }
    Ok(())
}

fn validate_user_and_rest(
    expected: &ExpectedProfile,
    user: &UserFacts,
    account: &AccountFacts,
) -> Result<(), PmOnlinePreflightMismatch> {
    if user.scope != expected.scope
        || user.signer != expected.signer
        || user.proxy_maker != expected.funder
        || user.user_activity_generation != user.rest_activity_generation
    {
        return Err(PmOnlinePreflightMismatch::SameAccountBinding);
    }
    if user.reconnect_count != user.reconnect_history_count
        || user.reconnect_count != 0
        || user.initial_connection_epoch != user.current_connection_epoch
    {
        return Err(PmOnlinePreflightMismatch::UserReconnectHistory);
    }
    if !user.same_rest_allocation
        || !user.rest_backed_quiet_basis
        || user.ticket_open_order_rows != account.open_order_row_count
        || user.ticket_trade_rows != account.trade_row_count
    {
        return Err(PmOnlinePreflightMismatch::SameAuthorityRestJoin);
    }
    if user.all_business_event_count != 0 || user.current_business_event_count != 0 {
        return Err(PmOnlinePreflightMismatch::UserBusinessHistory);
    }
    if account.open_order_page_count == 0
        || account.open_order_row_count != 0
        || account.open_order_projection_count != 0
    {
        return Err(PmOnlinePreflightMismatch::OpenOrdersNotEmpty);
    }
    if account.trade_page_count == 0 || account.trade_row_count != account.trades.len() {
        return Err(PmOnlinePreflightMismatch::SameAuthorityRestJoin);
    }

    let mut ids = HashSet::with_capacity(account.trades.len());
    let expected_order_id = expected.expected_order_id.to_string();
    for trade in &account.trades {
        if !ids.insert(trade.id) {
            return Err(PmOnlinePreflightMismatch::HistoricalTradeDuplicate);
        }
        if !matches!(trade.status.as_ref(), "CONFIRMED" | "FAILED") {
            return Err(PmOnlinePreflightMismatch::HistoricalTradeNonTerminal);
        }
        let expected_was_seen = trade
            .order_id
            .is_some_and(|order| order.as_str() == expected_order_id.as_str())
            || trade
                .taker_order_id
                .is_some_and(|order| order.as_str() == expected_order_id.as_str())
            || trade
                .maker_order_ids
                .iter()
                .any(|order| order.as_str() == expected_order_id.as_str());
        if expected_was_seen {
            return Err(PmOnlinePreflightMismatch::ExpectedOrderAlreadyObserved);
        }
    }
    Ok(())
}

fn validate_account_sources(
    expected: &ExpectedProfile,
    account: &AccountFacts,
    polygon: &PolygonFacts,
    position: &PositionFacts,
) -> Result<(), PmOnlinePreflightMismatch> {
    if account.closed_only {
        return Err(PmOnlinePreflightMismatch::ClosedOnly);
    }
    if account.collateral_asset != PmAccountAsset::Collateral
        || account.conditional_asset != PmAccountAsset::Conditional(expected.scope.token())
    {
        return Err(PmOnlinePreflightMismatch::AccountAssetBinding);
    }
    if account.collateral_unscoped_scalar || account.conditional_unscoped_scalar {
        return Err(PmOnlinePreflightMismatch::UnscopedAccountScalar);
    }
    let (Some(collateral_allowance), Some(conditional_allowance)) = (
        account.collateral_exchange_allowance,
        account.conditional_exchange_allowance,
    ) else {
        return Err(PmOnlinePreflightMismatch::AccountSpenderBinding);
    };
    if expected
        .maximum_loss
        .checked_sub(expected.reservation)
        .is_err()
        || account
            .collateral_balance
            .checked_sub(expected.reservation)
            .is_err()
        || collateral_allowance
            .checked_sub(expected.reservation)
            .is_err()
    {
        return Err(PmOnlinePreflightMismatch::CollateralRisk);
    }

    if position.proxy_funder != expected.funder
        || position.condition != expected.scope.condition()
        || position.token != expected.scope.token()
        || position.pages_observed == 0
    {
        return Err(PmOnlinePreflightMismatch::PositionScopeOrDomain);
    }
    let expected_negative_risk = expected.domain == PmSpenderDomain::NegativeRisk;
    if position.configured_row_present {
        if position.configured_row_asset != Some(expected.scope.token())
            || position.configured_outcome.as_deref() != Some(expected.outcome_label.as_ref())
            || position.configured_negative_risk != Some(expected_negative_risk)
        {
            return Err(PmOnlinePreflightMismatch::PositionScopeOrDomain);
        }
    } else if position.configured_row_asset.is_some()
        || position.configured_outcome.is_some()
        || position.configured_negative_risk.is_some()
        || !position.configured_size.is_zero()
    {
        return Err(PmOnlinePreflightMismatch::PositionScopeOrDomain);
    }
    if account.conditional_balance != position.configured_size {
        return Err(PmOnlinePreflightMismatch::ConditionalPositionMismatch);
    }

    if polygon.chain_id != 137
        || polygon.signer != expected.signer.as_core()
        || polygon.owner != expected.funder
        || polygon.spender != expected.polygon_spender
        || polygon.spender_address != expected.exchange
        || polygon.finalized_block_number == 0
        || polygon.finalized_block_timestamp == 0
        || polygon.observed_unix_seconds == 0
    {
        return Err(PmOnlinePreflightMismatch::PolygonScope);
    }
    if polygon.pusd_allowance != collateral_allowance {
        return Err(PmOnlinePreflightMismatch::PolygonAllowanceMismatch);
    }
    if !polygon.conditional_operator_approved || conditional_allowance.is_zero() {
        return Err(PmOnlinePreflightMismatch::PolygonOperatorApprovalMismatch);
    }
    Ok(())
}

fn validate_operational(operational: &OperationalFacts) -> Result<(), PmOnlinePreflightMismatch> {
    if !operational.production_status || !operational.production_health {
        return Err(PmOnlinePreflightMismatch::ProductionOperationalSource);
    }
    if operational.page_state != PmStatusPageState::Up
        || operational.active_incident_count != 0
        || operational.active_maintenance_count != 0
    {
        return Err(PmOnlinePreflightMismatch::StatusPage);
    }
    if operational.reviewed_component_match_count != 1
        || operational.reviewed_component_state != Some(PmStatusComponentState::Operational)
    {
        return Err(PmOnlinePreflightMismatch::StatusClobComponent);
    }
    Ok(())
}

fn validate_source_clock_coherence(
    expected: &ExpectedProfile,
    clocks: &[u64],
) -> Result<(), PmOnlinePreflightMismatch> {
    let Some((minimum, maximum)) = source_wall_extrema(clocks) else {
        return Err(PmOnlinePreflightMismatch::SourceClockCoherence);
    };
    if minimum == 0
        || maximum
            .checked_sub(minimum)
            .is_none_or(|span| span > expected.maximum_coherence_span_ns)
    {
        return Err(PmOnlinePreflightMismatch::SourceClockCoherence);
    }
    Ok(())
}

fn source_wall_extrema(clocks: &[u64]) -> Option<(u64, u64)> {
    Some((clocks.iter().copied().min()?, clocks.iter().copied().max()?))
}

#[allow(clippy::too_many_arguments)]
fn build_candidate_manifest(
    config: &CanonicalTrialConfig,
    policy: &CanonicalOnlinePolicyV2,
    book: &PmPublicBookLease,
    rest: &PmFreshAuthenticatedRestCut,
    user: &PmUserOnlinePreflightLease,
    status: &PmProductionStatusAnnouncementObservation,
    health: &PmProductionClobLivenessHealthObservation,
    polygon: &PmProductionPolygonFinalizedAuthorizationCut,
    position: &PmProductionDataApiPositionObservation,
    facts: &JoinedFacts,
) -> PmOnlinePreflightCandidateManifest {
    let digests = PmOnlinePreflightCandidateSourceDigests {
        fresh_status_announcements: status.commitment().bytes(),
        clob_ok_liveness: health.commitment().bytes(),
        same_account_closed_only: rest.closed_only().commitment().bytes(),
        public_book_cut: public_book_cut_digest(book),
        user_account_cut: user_account_cut_digest(user),
        same_authority_rest_cut: same_authority_rest_cut_digest(rest),
        finalized_chain_cut: polygon.commitment().bytes(),
        data_api_position_cut: position.commitment().bytes(),
    };
    let (minimum_source_wall_edge_ns, maximum_source_wall_edge_ns) =
        source_wall_extrema(&facts.source_wall_clocks_ns)
            .expect("validated candidate contains source clocks");

    let mut manifest = ManifestEncoder::new(CANDIDATE_MANIFEST_DOMAIN);
    manifest.u32(b"schema", CANDIDATE_MANIFEST_SCHEMA);
    manifest.text(b"config_sha256", config.canonical_sha256());
    manifest.u64(b"config_length", config.canonical_length());
    manifest.text(b"config_fingerprint", config.fingerprint());
    manifest.text(b"plan_fingerprint", config.plan_fingerprint());
    manifest.text(b"policy_sha256", policy.canonical_sha256());
    manifest.u64(b"policy_length", policy.canonical_length());
    manifest.text(b"policy_fingerprint", policy.fingerprint());
    manifest.text(b"market_end_time", &facts.market.end_time);
    encode_optional_bool(
        &mut manifest,
        b"take_only_delay_enabled_reported",
        facts.market.take_only_delay_enabled,
    );
    encode_optional_bool(
        &mut manifest,
        b"cancel_book_on_start",
        facts.market.cancel_book_on_start,
    );
    encode_optional_bool(&mut manifest, b"rfq_enabled", facts.market.rfq_enabled);
    encode_optional_bool(
        &mut manifest,
        b"bonding_curve_enabled",
        facts.market.bonding_curve_enabled,
    );
    manifest.bytes(
        b"expected_order_id",
        &config
            .exact_place_public_request_identity()
            .expected_order_id()
            .bytes(),
    );
    manifest.u64(b"source_wall_min_ns", minimum_source_wall_edge_ns);
    manifest.u64(b"source_wall_max_ns", maximum_source_wall_edge_ns);
    manifest.u64(
        b"source_wall_edge_count",
        facts.source_wall_clocks_ns.len() as u64,
    );
    manifest.bytes(b"status_announcements", &digests.fresh_status_announcements);
    manifest.bytes(b"clob_ok_liveness", &digests.clob_ok_liveness);
    manifest.bytes(
        b"same_account_closed_only",
        &digests.same_account_closed_only,
    );
    manifest.bytes(b"public_book_cut", &digests.public_book_cut);
    manifest.bytes(b"user_account_cut", &digests.user_account_cut);
    manifest.bytes(b"same_authority_rest_cut", &digests.same_authority_rest_cut);
    manifest.bytes(b"finalized_chain_cut", &digests.finalized_chain_cut);
    manifest.bytes(b"data_api_position_cut", &digests.data_api_position_cut);
    let canonical_bytes = manifest.finish().into_boxed_slice();
    let canonical_sha256 = sha256(&canonical_bytes);
    let mut fingerprint_basis =
        Vec::with_capacity(CANDIDATE_MANIFEST_FINGERPRINT_DOMAIN.len() + canonical_bytes.len());
    fingerprint_basis.extend_from_slice(CANDIDATE_MANIFEST_FINGERPRINT_DOMAIN);
    fingerprint_basis.extend_from_slice(&canonical_bytes);
    let fingerprint = sha256(&fingerprint_basis);
    PmOnlinePreflightCandidateManifest {
        canonical_bytes,
        canonical_sha256,
        fingerprint,
        minimum_source_wall_edge_ns,
        maximum_source_wall_edge_ns,
        market_end_time: facts.market.end_time.clone(),
        digests,
    }
}

fn public_book_cut_digest(book: &PmPublicBookLease) -> [u8; 32] {
    let mut encoder = ManifestEncoder::new(b"reap.pm-t2.runner.public-book-cut.v1\0");
    encoder.bytes(
        b"metadata_commitment",
        &book.metadata_observation_commitment().bytes(),
    );
    encoder.u64(
        b"metadata_source_wall_ns",
        book.metadata_source_receive_clock().local_wall_receive_ns(),
    );
    encoder.u64(
        b"metadata_source_monotonic_ns",
        book.metadata_source_receive_clock().monotonic_receive_ns(),
    );
    encoder.u64(b"state_generation", book.state_generation());
    encoder.u64(b"connection_epoch", book.connection_epoch().value());
    encoder.u64(b"metadata_revision", book.metadata_revision().value());
    encoder.u64(b"snapshot_revision", book.snapshot_revision().value());
    encoder.u64(b"ingress_sequence", book.local_ingress_sequence().value());
    encode_book_top(&mut encoder, book.ready_top());
    let heartbeat = book.heartbeat();
    encoder.u64(b"heartbeat_epoch", heartbeat.connection_epoch().value());
    encoder.u64(b"heartbeat_wall_ns", heartbeat.local_wall_receive_ns());
    encoder.u64(b"heartbeat_monotonic_ns", heartbeat.monotonic_receive_ns());
    encoder.u64(
        b"heartbeat_activity_generation",
        book.heartbeat_activity_generation(),
    );
    encoder.u64(b"activity_generation", book.activity_generation());
    encoder.u64(b"source_high_water", book.source_high_water());
    encoder.u64(b"fresh_until_monotonic_ns", book.fresh_until_monotonic_ns());
    encode_received_clock(
        &mut encoder,
        b"metadata_control_begin",
        book.metadata_control_begin_clock(),
    );
    encode_received_clock(
        &mut encoder,
        b"metadata_control_complete",
        book.metadata_control_complete_clock(),
    );
    encode_received_clock(&mut encoder, b"book_receive", book.book_receive_clock());
    encode_received_clock(
        &mut encoder,
        b"book_checked",
        book.checked_at_control_clock(),
    );
    sha256(&encoder.finish())
}

fn user_account_cut_digest(user: &PmUserOnlinePreflightLease) -> [u8; 32] {
    let mut encoder = ManifestEncoder::new(b"reap.pm-t2.runner.user-account-cut.v1\0");
    encode_wire_scope(&mut encoder, user.scope());
    encoder.bytes(b"signer", &user.signer().bytes());
    encoder.bytes(b"proxy_maker", &user.proxy_maker().bytes());
    encoder.u64(b"stream_revision", user.stream_revision());
    encoder.u64(b"initial_epoch", user.initial_connection_epoch().value());
    encoder.u64(b"current_epoch", user.current_connection_epoch().value());
    encoder.u64(
        b"connection_open_generation",
        user.connection_open_generation(),
    );
    encode_user_clock(
        &mut encoder,
        b"connection_open",
        user.connection_open_clock(),
    );
    encoder.u64(b"subscription_generation", user.subscription_generation());
    encode_user_clock(&mut encoder, b"subscription", user.subscription_clock());
    encoder.u64(b"ping_generation", user.ping_generation());
    encode_user_clock(&mut encoder, b"ping", user.ping_clock());
    encoder.u64(
        b"correlated_pong_generation",
        user.correlated_pong_generation(),
    );
    encode_user_clock(
        &mut encoder,
        b"correlated_pong",
        user.correlated_pong_clock(),
    );
    encoder.u64(
        b"admitted_activity_generation",
        user.admitted_activity_generation(),
    );
    encoder.u64(b"reconnect_count", u64::from(user.reconnect_count()));
    for (ordinal, reconnect) in user.reconnect_history().iter().enumerate() {
        encoder.u64(b"reconnect_ordinal", ordinal as u64);
        encoder.u64(b"retired_epoch", reconnect.retired_epoch().value());
        encoder.u64(b"replacement_epoch", reconnect.replacement_epoch().value());
        encoder.u64(
            b"reconnect_attempt",
            u64::from(reconnect.reconnect_attempt()),
        );
        encoder.u8(
            b"reconnect_reason",
            disconnect_reason_code(reconnect.reason()),
        );
        encoder.u128(b"reconnect_backoff_ns", reconnect.backoff().as_nanos());
        encode_optional_user_edge(
            &mut encoder,
            b"reconnect_open",
            reconnect.connection_open_generation(),
            reconnect.connection_open_clock(),
        );
        encode_optional_user_edge(
            &mut encoder,
            b"reconnect_subscription",
            reconnect.subscription_generation(),
            reconnect.subscription_clock(),
        );
        encode_optional_user_edge(
            &mut encoder,
            b"reconnect_ping",
            reconnect.latest_ping_generation(),
            reconnect.latest_ping_clock(),
        );
        encode_optional_user_edge(
            &mut encoder,
            b"reconnect_pong",
            reconnect.correlated_pong_generation(),
            reconnect.correlated_pong_clock(),
        );
        encoder.u64(
            b"retirement_activity_generation",
            reconnect.retirement_activity_generation(),
        );
        encode_user_clock(
            &mut encoder,
            b"retirement_clock",
            reconnect.retirement_clock(),
        );
        encoder.u64(
            b"reconnect_activity_generation",
            reconnect.reconnect_activity_generation(),
        );
        encode_user_clock(
            &mut encoder,
            b"reconnect_clock",
            reconnect.reconnect_clock(),
        );
    }
    encoder.u64(b"business_event_count", user.business_events().len() as u64);
    encoder.u64(b"open_order_rows", user.open_order_rows() as u64);
    encoder.u64(b"trade_rows", user.trade_rows() as u64);
    sha256(&encoder.finish())
}

fn same_authority_rest_cut_digest(rest: &PmFreshAuthenticatedRestCut) -> [u8; 32] {
    let mut encoder = ManifestEncoder::new(b"reap.pm-t2.runner.same-authority-rest-cut.v1\0");
    encoder.u64(b"activity_generation", rest.activity_generation());
    for (ordinal, time) in rest.server_times().iter().enumerate() {
        encoder.u64(b"server_time_ordinal", ordinal as u64);
        let (purpose, page_ordinal) = private_read_purpose_identity(time.purpose());
        encoder.u8(b"server_time_purpose", purpose);
        if let Some(page_ordinal) = page_ordinal {
            encoder.u8(b"server_time_page_present", 1);
            encoder.u64(b"server_time_page_ordinal", page_ordinal as u64);
        } else {
            encoder.u8(b"server_time_page_present", 0);
        }
        encoder.u64(b"server_time_seconds", time.timestamp().unix_seconds());
        encoder.u64(
            b"server_time_wall_ns",
            time.receive_clock().local_wall_receive_ns(),
        );
        encoder.u64(
            b"server_time_monotonic_ns",
            time.receive_clock().monotonic_receive_ns(),
        );
        encoder.bytes(b"server_time_commitment", &time.commitment().bytes());
    }
    encoder.bytes(b"closed_only", &rest.closed_only().commitment().bytes());
    encoder.bytes(b"collateral", &rest.collateral().commitment().bytes());
    encoder.bytes(b"conditional", &rest.conditional().commitment().bytes());
    encoder.bytes(b"open_orders", &rest.open_orders().commitment().bytes());
    encoder.u64(b"open_order_pages", rest.open_orders().page_count() as u64);
    encoder.u64(b"open_order_rows", rest.open_orders().row_count() as u64);
    encoder.bytes(b"trades", &rest.trades().commitment().bytes());
    encoder.u64(b"trade_pages", rest.trades().page_count() as u64);
    encoder.u64(b"trade_rows", rest.trades().row_count() as u64);
    sha256(&encoder.finish())
}

fn encode_wire_scope(encoder: &mut ManifestEncoder, scope: PmWireScope) {
    encoder.bytes(b"condition", &scope.condition().bytes());
    encoder.bytes(b"market", &scope.market().bytes());
    encoder.bytes(b"token", &scope.token().units().to_be_bytes());
}

fn encode_optional_bool(encoder: &mut ManifestEncoder, label: &[u8], value: Option<bool>) {
    match value {
        Some(value) => encoder.bytes(label, &[1, u8::from(value)]),
        None => encoder.bytes(label, &[0]),
    }
}

fn encode_book_top(encoder: &mut ManifestEncoder, top: PmBookTop) {
    encode_book_point(encoder, b"bid", top.bid());
    encode_book_point(encoder, b"ask", top.ask());
}

fn encode_book_point(encoder: &mut ManifestEncoder, label: &[u8], point: Option<PmBookPoint>) {
    match point {
        Some(point) => {
            encoder.u8(label, 1);
            encoder.u32(b"price_units", point.price().units());
            encoder.bytes(
                b"quantity_units",
                &point.quantity().protocol_units().to_be_bytes(),
            );
        }
        None => encoder.u8(label, 0),
    }
}

fn encode_received_clock(
    encoder: &mut ManifestEncoder,
    label: &[u8],
    clock: reap_pm_core::ReceivedEventClock,
) {
    encoder.bytes(label, b"received-event-clock");
    encoder.u64(b"wall_ns", clock.local_wall_receive_ns());
    encoder.u64(b"monotonic_ns", clock.monotonic_receive_ns());
}

fn encode_user_clock(
    encoder: &mut ManifestEncoder,
    label: &[u8],
    clock: reap_polymarket_live_adapter::PmUserWsEdgeClock,
) {
    encoder.bytes(label, b"user-ws-edge-clock");
    encoder.u64(b"wall_ns", clock.local_wall_receive_ns());
    encoder.u64(b"monotonic_ns", clock.monotonic_receive_ns());
}

fn encode_optional_user_edge(
    encoder: &mut ManifestEncoder,
    label: &[u8],
    generation: Option<u64>,
    clock: Option<reap_polymarket_live_adapter::PmUserWsEdgeClock>,
) {
    encoder.bytes(label, b"optional-user-ws-edge");
    match (generation, clock) {
        (Some(generation), Some(clock)) => {
            encoder.u8(b"present", 1);
            encoder.u64(b"generation", generation);
            encode_user_clock(encoder, b"clock", clock);
        }
        (None, None) => encoder.u8(b"present", 0),
        _ => {
            // The source state machine never constructs a torn pair. Encode a
            // distinct impossible marker so any future drift changes identity.
            encoder.u8(b"present", u8::MAX);
        }
    }
}

const fn private_read_purpose_identity(
    purpose: PmPrivateRestObservationPurpose,
) -> (u8, Option<usize>) {
    match purpose {
        PmPrivateRestObservationPurpose::ClosedOnly => (0, None),
        PmPrivateRestObservationPurpose::CollateralBalanceAllowance => (1, None),
        PmPrivateRestObservationPurpose::ConditionalBalanceAllowance => (2, None),
        PmPrivateRestObservationPurpose::OpenOrdersPage { ordinal } => (3, Some(ordinal)),
        PmPrivateRestObservationPurpose::TradesPage { ordinal } => (4, Some(ordinal)),
        PmPrivateRestObservationPurpose::RecoveryExactOrder => (5, None),
    }
}

const fn disconnect_reason_code(reason: PmUserWsDisconnectReason) -> u8 {
    match reason {
        PmUserWsDisconnectReason::ConnectTimeout => 0,
        PmUserWsDisconnectReason::ConnectFailed => 1,
        PmUserWsDisconnectReason::SubscriptionAuthenticationFailed => 2,
        PmUserWsDisconnectReason::SubscriptionWriteTimeout => 3,
        PmUserWsDisconnectReason::SubscriptionWriteFailed => 4,
        PmUserWsDisconnectReason::SocketReadFailed => 5,
        PmUserWsDisconnectReason::SocketClosed => 6,
        PmUserWsDisconnectReason::SocketWriteTimeout => 7,
        PmUserWsDisconnectReason::SocketWriteFailed => 8,
        PmUserWsDisconnectReason::BinaryFrame => 9,
        PmUserWsDisconnectReason::FrameTooLarge => 10,
        PmUserWsDisconnectReason::MalformedFrame => 11,
        PmUserWsDisconnectReason::CredentialOwnerMismatch => 12,
        PmUserWsDisconnectReason::CredentialAuthorityUnavailable => 13,
        PmUserWsDisconnectReason::IdleTimeout => 14,
        PmUserWsDisconnectReason::PongTimeout => 15,
        PmUserWsDisconnectReason::UnexpectedProtocolFrame => 16,
    }
}

struct ManifestEncoder {
    bytes: Vec<u8>,
}

impl ManifestEncoder {
    fn new(domain: &[u8]) -> Self {
        let mut bytes = Vec::with_capacity(1024);
        bytes.extend_from_slice(domain);
        Self { bytes }
    }

    fn bytes(&mut self, label: &[u8], value: &[u8]) {
        self.bytes
            .extend_from_slice(&(label.len() as u32).to_be_bytes());
        self.bytes.extend_from_slice(label);
        self.bytes
            .extend_from_slice(&(value.len() as u64).to_be_bytes());
        self.bytes.extend_from_slice(value);
    }

    fn text(&mut self, label: &[u8], value: &str) {
        self.bytes(label, value.as_bytes());
    }

    fn u8(&mut self, label: &[u8], value: u8) {
        self.bytes(label, &[value]);
    }

    fn u32(&mut self, label: &[u8], value: u32) {
        self.bytes(label, &value.to_be_bytes());
    }

    fn u64(&mut self, label: &[u8], value: u64) {
        self.bytes(label, &value.to_be_bytes());
    }

    fn u128(&mut self, label: &[u8], value: u128) {
        self.bytes(label, &value.to_be_bytes());
    }

    fn finish(self) -> Vec<u8> {
        self.bytes
    }
}

fn sha256(bytes: &[u8]) -> [u8; 32] {
    Sha256::digest(bytes).into()
}

#[cfg(test)]
mod tests {
    use super::*;

    const CONDITION: &str = "0x1111111111111111111111111111111111111111111111111111111111111111";
    const MARKET: &str = "0x2222222222222222222222222222222222222222222222222222222222222222";
    const SIGNER: &str = "0xf39Fd6e51aad88F6F4ce6aB8827279cffFb92266";
    const OTHER_SIGNER: &str = "0x3C44CdDdB6a900fa2b585dd299e03d12FA4293BC";
    const FUNDER: &str = "0x70997970C51812dc3A010C7d01b50e0d17dc79C8";
    const EXCHANGE: &str = "0xE111180000d2663C0091e4f400237545B87B996B";
    const PUSD: &str = "0xC011a7E12a19f7B1f670d46F03B03f3342E82DFB";
    const CONDITIONAL: &str = "0x4D97DCd97eC945f40cF65F87097ACe5EA0476045";
    const EXPECTED_ORDER: &str =
        "0xaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa";
    const FOREIGN_ORDER: &str =
        "0xbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbb";

    fn address(value: &str) -> EvmAddress {
        EvmAddress::parse(value).unwrap()
    }

    fn condition() -> PmConditionId {
        PmConditionId::parse(CONDITION).unwrap()
    }

    fn market() -> PmMarketId {
        PmMarketId::parse(MARKET).unwrap()
    }

    fn token() -> PmTokenId {
        PmTokenId::new(U256::from_u64(1234)).unwrap()
    }

    fn quantity(value: &str) -> PmQuantity {
        PmQuantity::parse_decimal(value).unwrap()
    }

    fn price(value: &str) -> PmPrice {
        PmPrice::parse_decimal(value).unwrap()
    }

    fn tick(value: &str) -> PmTick {
        PmTick::parse_decimal(value).unwrap()
    }

    fn user_clock(wall_ns: u64) -> PmUserWsEdgeClock {
        PmUserWsEdgeClock::new(wall_ns, wall_ns).unwrap()
    }

    fn expected() -> ExpectedProfile {
        ExpectedProfile {
            scope: PmWireScope::new(condition(), market(), token()),
            signer: EoaAddress::parse(SIGNER).unwrap(),
            funder: address(FUNDER),
            outcome_label: "YES".into(),
            domain: PmSpenderDomain::Standard,
            exchange: address(EXCHANGE),
            collateral_contract: address(PUSD),
            conditional_tokens_contract: address(CONDITIONAL),
            polygon_spender: PmPolygonExchangeSpender::StandardV2,
            price: price("0.5"),
            quantity: quantity("5"),
            tick: tick("0.01"),
            minimum: quantity("5"),
            maker_base_fee_bps: 0,
            taker_base_fee_bps: 0,
            fee_rate: "0.02".into(),
            fee_exponent: "2".into(),
            reservation: U256::from_u64(5_000_000),
            maximum_loss: U256::from_u64(5_000_000),
            expected_order_id: FixedOrderId::parse(EXPECTED_ORDER).unwrap(),
            status_component_id: "clob".into(),
            status_component_name: "CLOB".into(),
            maximum_coherence_span_ns: 5_000_000_000,
        }
    }

    fn required_spenders() -> [PmSpenderRequirement; 2] {
        let chain = PmChainId::new(137).unwrap();
        let exchange = address(EXCHANGE);
        [
            PmSpenderRequirement::new(
                chain,
                exchange,
                PmSpenderDomain::Standard,
                PmAssetId::collateral(address(PUSD)),
            ),
            PmSpenderRequirement::new(
                chain,
                exchange,
                PmSpenderDomain::Standard,
                PmAssetId::outcome(address(CONDITIONAL), token()),
            ),
        ]
    }

    fn historical_trade() -> TradeFacts {
        TradeFacts {
            id: PmFillId::new("historical-fill-1").unwrap(),
            status: "CONFIRMED".into(),
            order_id: Some(PmVenueOrderId::new(FOREIGN_ORDER).unwrap()),
            taker_order_id: None,
            maker_order_ids: Box::new([]),
        }
    }

    fn facts() -> JoinedFacts {
        let top = PmBookTop::new(
            Some(PmBookPoint::new(price("0.49"), quantity("5"))),
            Some(PmBookPoint::new(price("0.51"), quantity("5"))),
        )
        .unwrap();
        JoinedFacts {
            market: MarketFacts {
                condition: condition(),
                question: market(),
                reported_condition: Some(condition()),
                book_market_binding: PmBookMarketBinding::ConditionId,
                configured_token: token(),
                configured_outcome_label: "YES".into(),
                configured_token_membership_count: 1,
                chain: PmChainId::new(137).unwrap(),
                domain: PmSpenderDomain::Standard,
                exchange: address(EXCHANGE),
                collateral_asset: PmAssetId::collateral(address(PUSD)),
                outcome_asset: PmAssetId::outcome(address(CONDITIONAL), token()),
                required_spenders: required_spenders(),
                lifecycle_active: true,
                lifecycle_closed: false,
                lifecycle_archived: false,
                lifecycle_accepting_orders: true,
                lifecycle_order_book_enabled: true,
                tick: tick("0.01"),
                minimum: quantity("5"),
                maker_base_fee_bps: 0,
                taker_base_fee_bps: 0,
                fee_rate: Some("0.02".into()),
                fee_exponent: Some("2".into()),
                fee_taker_only: Some(true),
                seconds_delay: 0,
                reported_seconds_delay: Some(0),
                take_only_delay_enabled: Some(false),
                cancel_book_on_start: Some(false),
                minimum_order_age_seconds: 0,
                accepting_orders_reported: Some(true),
                rfq_enabled: Some(false),
                bonding_curve_enabled: Some(false),
                game_start_time_present: false,
                end_time: "2026-08-10T00:00:00Z".into(),
            },
            book: BookFacts {
                source_fully_admitted: true,
                top,
            },
            user: UserFacts {
                scope: PmWireScope::new(condition(), market(), token()),
                signer: EoaAddress::parse(SIGNER).unwrap(),
                proxy_maker: address(FUNDER),
                same_rest_allocation: true,
                user_activity_generation: 4,
                rest_activity_generation: 4,
                initial_connection_epoch: 11,
                current_connection_epoch: 11,
                reconnect_count: 0,
                reconnect_history_count: 0,
                all_business_event_count: 0,
                current_business_event_count: 0,
                rest_backed_quiet_basis: true,
                ticket_open_order_rows: 0,
                ticket_trade_rows: 1,
            },
            account: AccountFacts {
                closed_only: false,
                collateral_asset: PmAccountAsset::Collateral,
                conditional_asset: PmAccountAsset::Conditional(token()),
                collateral_unscoped_scalar: false,
                conditional_unscoped_scalar: false,
                collateral_balance: U256::from_u64(10_000_000),
                collateral_exchange_allowance: Some(U256::from_u64(10_000_000)),
                conditional_balance: U256::ZERO,
                conditional_exchange_allowance: Some(U256::ONE),
                open_order_page_count: 1,
                open_order_row_count: 0,
                open_order_projection_count: 0,
                trade_page_count: 1,
                trade_row_count: 1,
                trades: vec![historical_trade()],
            },
            polygon: PolygonFacts {
                chain_id: 137,
                signer: EoaAddress::parse(SIGNER).unwrap().as_core(),
                owner: address(FUNDER),
                spender: PmPolygonExchangeSpender::StandardV2,
                spender_address: address(EXCHANGE),
                pusd_allowance: U256::from_u64(10_000_000),
                conditional_operator_approved: true,
                finalized_block_number: 1,
                finalized_block_timestamp: 1_786_000_000,
                observed_unix_seconds: 1_786_000_001,
            },
            position: PositionFacts {
                proxy_funder: address(FUNDER),
                condition: condition(),
                token: token(),
                pages_observed: 1,
                configured_row_present: false,
                configured_size: U256::ZERO,
                configured_row_asset: None,
                configured_outcome: None,
                configured_negative_risk: None,
            },
            operational: OperationalFacts {
                production_status: true,
                production_health: true,
                page_state: PmStatusPageState::Up,
                active_incident_count: 0,
                active_maintenance_count: 0,
                reviewed_component_match_count: 1,
                reviewed_component_state: Some(PmStatusComponentState::Operational),
            },
            source_wall_clocks_ns: vec![
                1_786_000_000_000_000_000,
                1_786_000_000_100_000_000,
                1_786_000_000_200_000_000,
            ],
        }
    }

    fn assert_mismatch(
        expected_mismatch: PmOnlinePreflightMismatch,
        mutate: impl FnOnce(&mut ExpectedProfile, &mut JoinedFacts),
    ) {
        let mut expected = expected();
        let mut facts = facts();
        mutate(&mut expected, &mut facts);
        assert_eq!(
            validate_joined_facts(&expected, &facts),
            Err(expected_mismatch)
        );
    }

    #[test]
    fn exact_quiet_buy_profile_and_corroborated_present_position_are_admitted_as_evidence() {
        let expected = expected();
        let mut facts = facts();
        assert_eq!(validate_joined_facts(&expected, &facts), Ok(()));

        facts.account.conditional_balance = U256::from_u64(2_000_000);
        facts.position.configured_row_present = true;
        facts.position.configured_size = U256::from_u64(2_000_000);
        facts.position.configured_row_asset = Some(token());
        facts.position.configured_outcome = Some("YES".into());
        facts.position.configured_negative_risk = Some(false);
        assert_eq!(validate_joined_facts(&expected, &facts), Ok(()));
    }

    #[test]
    fn every_optional_market_safety_flag_requires_explicit_false() {
        for absent in 0..4 {
            assert_mismatch(
                PmOnlinePreflightMismatch::UnsupportedMarketFlags,
                |_, facts| match absent {
                    0 => facts.market.take_only_delay_enabled = None,
                    1 => facts.market.cancel_book_on_start = None,
                    2 => facts.market.rfq_enabled = None,
                    _ => facts.market.bonding_curve_enabled = None,
                },
            );
        }
        for enabled in 0..4 {
            assert_mismatch(
                PmOnlinePreflightMismatch::UnsupportedMarketFlags,
                |_, facts| match enabled {
                    0 => facts.market.take_only_delay_enabled = Some(true),
                    1 => facts.market.cancel_book_on_start = Some(true),
                    2 => facts.market.rfq_enabled = Some(true),
                    _ => facts.market.bonding_curve_enabled = Some(true),
                },
            );
        }
    }

    #[test]
    fn every_digested_user_clock_participates_in_candidate_coherence() {
        let expected = expected();
        let fresh = 10_000_000_000;
        let stale = 1;

        let mut current_open_stale = Vec::new();
        append_current_user_source_wall_clocks(
            &mut current_open_stale,
            user_clock(stale),
            user_clock(fresh),
            user_clock(fresh + 1),
            user_clock(fresh + 2),
        );
        assert_eq!(
            source_wall_extrema(&current_open_stale),
            Some((stale, fresh + 2)),
            "candidate minimum must retain the pre-window connection-open edge",
        );
        assert_eq!(
            validate_source_clock_coherence(&expected, &current_open_stale),
            Err(PmOnlinePreflightMismatch::SourceClockCoherence),
            "a fresh ping/PONG must not hide a pre-window connection-open edge",
        );

        for stale_reconnect_edge in 0..2 {
            let mut reconnect_stale = Vec::new();
            append_current_user_source_wall_clocks(
                &mut reconnect_stale,
                user_clock(fresh),
                user_clock(fresh + 1),
                user_clock(fresh + 2),
                user_clock(fresh + 3),
            );
            append_reconnect_user_source_wall_clocks(
                &mut reconnect_stale,
                Some(1),
                Some(user_clock(if stale_reconnect_edge == 0 {
                    stale
                } else {
                    fresh
                })),
                None,
                None,
                None,
                None,
                None,
                None,
                user_clock(fresh + 4),
                user_clock(if stale_reconnect_edge == 1 {
                    stale
                } else {
                    fresh + 5
                }),
            )
            .unwrap();
            assert_eq!(
                source_wall_extrema(&reconnect_stale).map(|edges| edges.0),
                Some(stale),
                "candidate minimum must retain the pre-window reconnect edge",
            );
            assert_eq!(
                validate_source_clock_coherence(&expected, &reconnect_stale),
                Err(PmOnlinePreflightMismatch::SourceClockCoherence),
                "fresh current ping/PONG must not hide a pre-window reconnect edge",
            );
        }
    }

    #[test]
    fn user_clock_collection_retains_current_and_every_reconnect_edge() {
        let mut clocks = Vec::new();
        append_current_user_source_wall_clocks(
            &mut clocks,
            user_clock(1),
            user_clock(2),
            user_clock(3),
            user_clock(4),
        );
        append_reconnect_user_source_wall_clocks(
            &mut clocks,
            Some(1),
            Some(user_clock(5)),
            Some(2),
            Some(user_clock(6)),
            Some(3),
            Some(user_clock(7)),
            Some(4),
            Some(user_clock(8)),
            user_clock(9),
            user_clock(10),
        )
        .unwrap();
        assert_eq!(clocks, (1..=10).collect::<Vec<_>>());
    }

    #[test]
    fn optional_reconnect_clock_and_generation_pairs_cannot_tear() {
        for (generation, clock) in [(Some(1), None), (None, Some(user_clock(10)))] {
            assert_eq!(
                append_reconnect_user_source_wall_clocks(
                    &mut Vec::new(),
                    generation,
                    clock,
                    None,
                    None,
                    None,
                    None,
                    None,
                    None,
                    user_clock(11),
                    user_clock(12),
                ),
                Err(PmOnlinePreflightMismatch::SourceClockCoherence),
            );
        }
    }

    #[test]
    fn quiet_profile_rejects_every_reconnect_or_epoch_history_shape() {
        for drift in 0..4 {
            assert_mismatch(
                PmOnlinePreflightMismatch::UserReconnectHistory,
                |_, facts| match drift {
                    0 => facts.user.current_connection_epoch += 1,
                    1 => {
                        facts.user.reconnect_count = 1;
                        facts.user.reconnect_history_count = 1;
                    }
                    2 => facts.user.reconnect_count = 1,
                    _ => facts.user.reconnect_history_count = 1,
                },
            );
        }
    }

    #[test]
    fn every_closed_mismatch_class_fails_independently() {
        assert_mismatch(
            PmOnlinePreflightMismatch::PhaseSideOrMinimum,
            |expected, _| {
                expected.quantity = quantity("6");
            },
        );
        assert_mismatch(PmOnlinePreflightMismatch::MarketIdentity, |_, facts| {
            facts.market.configured_token_membership_count = 0;
        });
        assert_mismatch(
            PmOnlinePreflightMismatch::MarketDomainOrContracts,
            |_, facts| facts.market.exchange = address(FUNDER),
        );
        assert_mismatch(PmOnlinePreflightMismatch::MarketLifecycle, |_, facts| {
            facts.market.lifecycle_accepting_orders = false;
        });
        assert_mismatch(
            PmOnlinePreflightMismatch::MarketTickOrMinimum,
            |_, facts| facts.market.minimum = quantity("6"),
        );
        assert_mismatch(PmOnlinePreflightMismatch::MarketFees, |_, facts| {
            facts.market.fee_taker_only = Some(false);
        });
        assert_mismatch(
            PmOnlinePreflightMismatch::UnsupportedMarketFlags,
            |_, facts| facts.market.minimum_order_age_seconds = 1,
        );
        assert_mismatch(PmOnlinePreflightMismatch::BookNotTwoSided, |_, facts| {
            facts.book.top =
                PmBookTop::new(Some(PmBookPoint::new(price("0.49"), quantity("5"))), None).unwrap();
        });
        assert_mismatch(PmOnlinePreflightMismatch::BookNotPassive, |expected, _| {
            expected.price = price("0.51");
        });
        assert_mismatch(PmOnlinePreflightMismatch::SameAccountBinding, |_, facts| {
            facts.user.signer = EoaAddress::parse(OTHER_SIGNER).unwrap();
        });
        assert_mismatch(
            PmOnlinePreflightMismatch::SameAuthorityRestJoin,
            |_, facts| facts.user.same_rest_allocation = false,
        );
        assert_mismatch(
            PmOnlinePreflightMismatch::UserBusinessHistory,
            |_, facts| {
                facts.user.all_business_event_count = 1;
            },
        );
        assert_mismatch(
            PmOnlinePreflightMismatch::UserReconnectHistory,
            |_, facts| facts.user.current_connection_epoch += 1,
        );
        assert_mismatch(PmOnlinePreflightMismatch::OpenOrdersNotEmpty, |_, facts| {
            facts.account.open_order_row_count = 1;
            facts.account.open_order_projection_count = 1;
            facts.user.ticket_open_order_rows = 1;
        });
        assert_mismatch(
            PmOnlinePreflightMismatch::HistoricalTradeDuplicate,
            |_, facts| {
                facts.account.trades.push(historical_trade());
                facts.account.trade_row_count = 2;
                facts.user.ticket_trade_rows = 2;
            },
        );
        assert_mismatch(
            PmOnlinePreflightMismatch::HistoricalTradeNonTerminal,
            |_, facts| facts.account.trades[0].status = "MATCHED".into(),
        );
        assert_mismatch(PmOnlinePreflightMismatch::ClosedOnly, |_, facts| {
            facts.account.closed_only = true;
        });
        assert_mismatch(
            PmOnlinePreflightMismatch::AccountAssetBinding,
            |_, facts| {
                facts.account.conditional_asset = PmAccountAsset::Collateral;
            },
        );
        assert_mismatch(
            PmOnlinePreflightMismatch::UnscopedAccountScalar,
            |_, facts| facts.account.collateral_unscoped_scalar = true,
        );
        assert_mismatch(
            PmOnlinePreflightMismatch::AccountSpenderBinding,
            |_, facts| {
                facts.account.collateral_exchange_allowance = None;
            },
        );
        assert_mismatch(PmOnlinePreflightMismatch::CollateralRisk, |_, facts| {
            facts.account.collateral_balance = U256::from_u64(4_999_999);
        });
        assert_mismatch(
            PmOnlinePreflightMismatch::ConditionalPositionMismatch,
            |_, facts| facts.account.conditional_balance = U256::ONE,
        );
        assert_mismatch(
            PmOnlinePreflightMismatch::PositionScopeOrDomain,
            |_, facts| facts.position.proxy_funder = address(EXCHANGE),
        );
        assert_mismatch(PmOnlinePreflightMismatch::PolygonScope, |_, facts| {
            facts.polygon.chain_id = 1;
        });
        assert_mismatch(
            PmOnlinePreflightMismatch::PolygonAllowanceMismatch,
            |_, facts| facts.polygon.pusd_allowance = U256::from_u64(9_000_000),
        );
        assert_mismatch(
            PmOnlinePreflightMismatch::PolygonOperatorApprovalMismatch,
            |_, facts| facts.polygon.conditional_operator_approved = false,
        );
        assert_mismatch(PmOnlinePreflightMismatch::StatusPage, |_, facts| {
            facts.operational.active_incident_count = 1;
        });
        assert_mismatch(
            PmOnlinePreflightMismatch::StatusClobComponent,
            |_, facts| {
                facts.operational.reviewed_component_match_count = 0;
            },
        );
        assert_mismatch(
            PmOnlinePreflightMismatch::ProductionOperationalSource,
            |_, facts| facts.operational.production_health = false,
        );
        assert_mismatch(
            PmOnlinePreflightMismatch::SourceClockCoherence,
            |_, facts| facts.source_wall_clocks_ns.push(1_786_000_010_000_000_000),
        );
    }

    #[test]
    fn expected_order_is_excluded_from_top_taker_and_every_maker_leg() {
        for association in 0..3 {
            assert_mismatch(
                PmOnlinePreflightMismatch::ExpectedOrderAlreadyObserved,
                |_, facts| {
                    let expected_id = PmVenueOrderId::new(EXPECTED_ORDER).unwrap();
                    match association {
                        0 => facts.account.trades[0].order_id = Some(expected_id),
                        1 => facts.account.trades[0].taker_order_id = Some(expected_id),
                        _ => facts.account.trades[0].maker_order_ids = Box::new([expected_id]),
                    }
                },
            );
        }
    }

    #[test]
    fn candidate_manifest_encoding_is_domain_separated_and_unambiguous() {
        let mut first = ManifestEncoder::new(CANDIDATE_MANIFEST_DOMAIN);
        first.bytes(b"ab", b"c");
        let mut second = ManifestEncoder::new(CANDIDATE_MANIFEST_DOMAIN);
        second.bytes(b"a", b"bc");
        assert_ne!(first.finish(), second.finish());
        let mut absent = ManifestEncoder::new(CANDIDATE_MANIFEST_DOMAIN);
        encode_optional_bool(&mut absent, b"flag", None);
        let mut explicit_false = ManifestEncoder::new(CANDIDATE_MANIFEST_DOMAIN);
        encode_optional_bool(&mut explicit_false, b"flag", Some(false));
        assert_ne!(absent.finish(), explicit_false.finish());
        assert_ne!(
            sha256(CANDIDATE_MANIFEST_DOMAIN),
            sha256(CANDIDATE_MANIFEST_FINGERPRINT_DOMAIN)
        );
    }

    #[test]
    fn production_collector_is_explicitly_closed() {
        assert_eq!(
            production_collector_blocker(),
            PmOnlinePreflightCollectorBlocker::OuterWindowRuntimeAndSelectedEgressNotIntegrated
        );
    }
}
