//! Explicit production authority for one Predarb-backed place/cancel trial.
//!
//! This is intentionally separate from the reviewed-artifact Phase-A stack:
//! it never claims that Predarb's local `.env` satisfies those attestations.
//! Authority instead comes from one literal operator phrase, one protected
//! credential file, one create-new durable ledger, a hard five-share cap, and
//! consume-once fixed-purpose Reap auth/transport values. There is no retry,
//! batch order, cancel-all, arbitrary origin, or placement resumption path.

use std::{
    collections::BTreeMap,
    fs::{self, DirBuilder, File, OpenOptions},
    io::{Read as _, Write as _},
    net::IpAddr,
    os::unix::fs::{DirBuilderExt as _, MetadataExt as _, OpenOptionsExt as _},
    path::{Path, PathBuf},
    time::{Duration, SystemTime, UNIX_EPOCH},
};

use reap_pm_core::{
    ConnectionEpoch, EvmAddress, PmAccountHandle, PmAccountScope, PmBookQuantity, PmChainId,
    PmClientOrderId, PmClientOrderKey, PmConditionId, PmEnvironmentId, PmFillId, PmFillKey,
    PmFunderId, PmInstrumentHandle, PmMarketHandle, PmMarketId, PmOrderProgress, PmOrderSalt,
    PmOrderSide, PmOrderStatus, PmPrice, PmQuantity, PmSignerId, PmTokenHandle, PmTokenId,
    PmVenueOrderId, PmVenueOrderKey, U256, exact_order_amounts,
};
use reap_pm_state::{
    PmExactReservation, PmOwnedCancelOutcome, PmOwnedCancelRequestApply, PmOwnedCancelState,
    PmOwnedDetailAbsenceApply, PmOwnedFillObservation, PmOwnedIntentId,
    PmOwnedObservationOccurrence, PmOwnedObservationSource, PmOwnedOrderLifecycle,
    PmOwnedOrderProgressObservation, PmOwnedQuoteAdmission, PmOwnedQuoteIntent,
    PmOwnedQuoteSlotKey, PmOwnedReductionSequence, PmOwnedSubmitResult, PmOwnedSubmitState,
};
use reap_polymarket_auth::{
    EoaPrivateKeyInput, FixedEoaSigner, FixedOrderId, L2CredentialInput, L2Credentials,
    L2Timestamp, LegacyType1ProxyAddress, PmClobDomain,
    derive_gtc_fill_place_public_request_identity, derive_place_public_request_identity,
    legacy_type1_proxy_address_matches,
};
use reap_polymarket_egress_binding::{PmFixedTlsPeerSelection, PmLocalEgressSelection};
use reap_polymarket_live_adapter::{
    PmCompleteTradesCut, PmExactOrderObservation, PmExactOwnedCancelProductionRole,
    PmFixedGtcFillPlaceProductionRole, PmFixedPlaceProductionRole, PmLiveAdapterError,
    PmMutationClassification, PmMutationDiagnosticKind, PmProductionMutationConfig,
    PmPublicHttpRole, PmReadOnlyAccountConnectivityOwner, PmReadOnlyCredentialInput,
    PmReadOnlyPrivateConnectivityOwner, PmReadOnlySignatureType, PmRestBookDeliveryError,
    PmRestBookPurpose, PmRestBookSnapshotSink, PmRestResponseClock, PmRetainedGtcFillPlaceRequest,
    PmRetainedOwnedCancelRequest, PmRetainedPlaceRequest, PmTradesCutProgress, PmUserWsBounds,
};
use reap_polymarket_public_source::{
    PmBtcFiveMinuteMarket, PmBtcFiveMinuteMarketSource, PmConfiguredTokenPosition,
    PmDataApiCurrentPositionSource, PmDataApiPositionScope, PmProductionDataApiPositionObservation,
};
use reap_polymarket_wire::{
    MAX_PM_LIVE_BODY_BYTES, PmBookParserConfig, PmBookSnapshot, PmLiveTrade, PmUnsignedClobV2Order,
    PmWireError, PmWireScope, parse_rest_book_snapshot,
};
use serde::Serialize;
use serde_json::json;
use thiserror::Error;
use zeroize::Zeroizing;

const AUTHORIZATION_PHRASE: &str = "I_ACCEPT_TOTAL_LOSS_AND_ONE_REAL_POLYMARKET_ORDER";
const LEDGER_FILE: &str = "pm-production-place-then-exact-cancel-v1.jsonl";
const MAX_CREDENTIAL_FILE_BYTES: u64 = 64 * 1024;
const MAX_TEST_QUANTITY: &str = "5";
const TEST_PRICE: &str = "0.01";
const MINIMUM_FAR_DISTANCE_UNITS: u32 = 200_000;
const MINIMUM_WINDOW_REMAINING_SECONDS: u64 = 75;
const MAX_BOOK_AGE_MILLIS: u64 = 5_000;
const MAX_BOOK_FUTURE_LEAD_MILLIS: u64 = 2_000;
const CONNECT_TIMEOUT: Duration = Duration::from_secs(10);
const REQUEST_TIMEOUT: Duration = Duration::from_secs(20);
const MAX_FILL_POSITION_RECONCILIATION_ATTEMPTS: u8 = 16;
const FILL_POSITION_RECONCILIATION_INTERVAL: Duration = Duration::from_secs(2);

/// Non-secret CLI inputs for one production place-then-exact-cancel attempt.
/// The fixed credential source is opened only inside this private authority
/// child and none of its values can be supplied through this carrier.
pub(crate) struct PredarbProductionOrderRequestV1 {
    pub(crate) credential_env: PathBuf,
    pub(crate) state_directory: PathBuf,
    pub(crate) fixed_peer_ip: String,
    pub(crate) interface_name: String,
    pub(crate) local_source_ip: IpAddr,
    pub(crate) authorization_phrase: String,
}

#[derive(Clone, Copy, PartialEq, Eq)]
enum ProductionOrderTrialMode {
    FarPostOnlyCancel,
    MinimumFill,
}

impl ProductionOrderTrialMode {
    const fn name(self) -> &'static str {
        match self {
            Self::FarPostOnlyCancel => "gtc_post_only_far_cancel",
            Self::MinimumFill => "gtc_non_post_only_minimum_fill",
        }
    }

    const fn post_only(self) -> bool {
        matches!(self, Self::FarPostOnlyCancel)
    }
}

pub(crate) struct PredarbExactOrderReconciliationRequestV1 {
    pub(crate) credential_env: PathBuf,
    pub(crate) condition_id: String,
    pub(crate) question_id: String,
    pub(crate) token_id: String,
    pub(crate) order_id: String,
}

pub(crate) struct PredarbOwnedFillPositionReconciliationRequestV1 {
    pub(crate) credential_env: PathBuf,
    pub(crate) condition_id: String,
    pub(crate) question_id: String,
    pub(crate) token_id: String,
    pub(crate) order_id: String,
    pub(crate) price: String,
    pub(crate) quantity: String,
    pub(crate) position_before_protocol_units: String,
}

/// Secret-free terminal summary printed by the production command.
#[derive(Debug, Serialize)]
pub(crate) struct PredarbProductionOrderReportV1 {
    ledger_path: PathBuf,
    order_profile: &'static str,
    market_slug: String,
    market_title: String,
    condition_id: String,
    outcome: &'static str,
    token_id: String,
    price: String,
    quantity: String,
    fresh_best_bid: String,
    fresh_best_ask: String,
    collateral_balance_protocol_units: String,
    book_timestamp_millis: u64,
    book_hash: String,
    expected_order_id: String,
    place_classification: &'static str,
    place_diagnostic: &'static str,
    place_http_status: Option<u16>,
    place_response_bytes: usize,
    cancel_attempted: bool,
    cancel_classification: Option<&'static str>,
    cancel_diagnostic: Option<&'static str>,
    cancel_http_status: Option<u16>,
    cancel_response_bytes: Option<usize>,
    position_before: PositionReport,
    position_after: Option<PositionReport>,
    position_unchanged: Option<bool>,
    fill_position_reconciliation: Option<FillPositionReconciliationReport>,
    canonical_order_state: CanonicalOrderStateReport,
    canonical_order_state_consistent: bool,
    exact_order_reconciliation: Option<ExactOrderStateReconciliationReport>,
    manual_reconciliation_required: bool,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
struct PositionReport {
    classification: &'static str,
    size: Option<String>,
    protocol_units: String,
    observed_at_millis: u64,
    commitment: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
struct FillPositionReconciliationReport {
    attempt: u8,
    trade_pages: usize,
    account_trade_rows: usize,
    owned_fill_count: usize,
    fill_quantity: String,
    fill_delta: String,
    position_before: String,
    fill_based_position: String,
    venue_position: String,
    order_cumulative_filled: String,
    known_fill_quantity: String,
    fills_match_order_cumulative: bool,
    fill_ledger_reconciled: bool,
    venue_position_matches_fill_based: bool,
    authoritative_minus_fill_based_position: String,
    converged: bool,
}

/// Venue-neutral lifecycle facts in the same shape consumed by the OKX side,
/// while retaining PM's exact submit/cancel ambiguity and reconciliation
/// states. This is a projection only; [`PmOwnedOrderLifecycle`] remains the
/// sole canonical reducer.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
struct CanonicalOrderStateReport {
    client_order_id: String,
    venue_order_id: Option<String>,
    status: &'static str,
    submit_state: &'static str,
    cancel_state: &'static str,
    quantity: String,
    open_quantity: String,
    filled_quantity: String,
    known_fill_quantity: String,
    is_live: bool,
    is_terminal: bool,
    reconciliation_required: bool,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
struct ExactOrderStateReconciliationReport {
    classification: &'static str,
    status: Option<String>,
    original_size: Option<String>,
    size_matched: Option<String>,
    state_applied: bool,
}

#[derive(Debug, Serialize)]
pub(crate) struct PredarbExactOrderReconciliationReportV1 {
    order_id: String,
    classification: &'static str,
    status: Option<String>,
    original_size: Option<String>,
    size_matched: Option<String>,
    price: Option<String>,
    side: Option<&'static str>,
    cancellation_verified: bool,
}

#[derive(Debug, Serialize)]
pub(crate) struct PredarbOwnedFillPositionReconciliationReportV1 {
    order_id: String,
    exact_order: ExactOrderStateReconciliationReport,
    fill_position_reconciliation: FillPositionReconciliationReport,
    canonical_order_state: CanonicalOrderStateReport,
    position_after: PositionReport,
    collateral_balance_protocol_units: String,
}

#[derive(Debug, Error)]
pub(crate) enum PredarbProductionOrderErrorV1 {
    #[error("production order entry requires the exact total-loss authorization phrase")]
    AuthorizationPhrase,
    #[error("the Predarb credential source must be one owner-held regular 0600 file")]
    CredentialFileProtection,
    #[error("the Predarb credential source could not be read safely")]
    CredentialFileRead,
    #[error(
        "the Predarb credential source is missing, duplicates, or malforms a required variable"
    )]
    CredentialEnvironment,
    #[error("this command accepts only Predarb's legacy type-1 proxy credential profile")]
    CredentialProfile,
    #[error("the Predarb private key, L2 bundle, or proxy binding is invalid")]
    CredentialBinding,
    #[error("the requested order is invalid or exceeds the hard five-share test cap")]
    OrderProfile,
    #[error("the current BTC Up/Down five-minute market could not be discovered safely")]
    MarketDiscovery,
    #[error("the current BTC Up/Down five-minute book failed the fresh far-price preflight")]
    BookPreflight,
    #[error("the current BTC Up/Down five-minute book transport failed")]
    BookTransport,
    #[error("the current BTC Up/Down five-minute book failed strict parsing")]
    BookParse,
    #[error("the current BTC Up/Down five-minute book was stale or future-dated")]
    BookStale,
    #[error("too little time remains in the current BTC five-minute window")]
    MarketWindowTooClose,
    #[error("the pre-order public position observation failed")]
    PositionPreflight,
    #[error("the exact read-only order reconciliation failed")]
    Reconciliation,
    #[error("the authenticated fill cut was inconsistent with the exact owned order")]
    FillReconciliation,
    #[error("the authenticated fill observation failed: {0}")]
    FillObservation(PmLiveAdapterError),
    #[error("fill-derived position arithmetic was not exactly representable")]
    PositionArithmetic,
    #[error("the canonical owned-order lifecycle rejected an inconsistent transition")]
    OrderState,
    #[error("the selected production peer or local egress is invalid")]
    TransportProfile,
    #[error("the one-shot state directory is not one owner-held 0700 directory")]
    StateDirectoryProtection,
    #[error("the create-new one-shot ledger already exists; placement resumption is forbidden")]
    AttemptAlreadyExists,
    #[error("the create-new one-shot ledger could not be made durable")]
    Ledger,
    #[error("fixed-purpose Polymarket request authentication failed")]
    Authentication,
    #[error("the system clock cannot produce a canonical Polymarket timestamp")]
    Clock,
}

struct PredarbCredentialBundle {
    private_key: Zeroizing<String>,
    funder: String,
    signature_type: String,
    api_key: Zeroizing<String>,
    api_secret: Zeroizing<String>,
    api_passphrase: Zeroizing<String>,
}

struct OneShotLedger {
    path: PathBuf,
    file: File,
}

struct ProductionOwnedOrderState {
    lifecycle: PmOwnedOrderLifecycle,
    client_order: PmClientOrderKey,
    venue_order: PmVenueOrderKey,
    next_reduction_sequence: u64,
}

impl ProductionOwnedOrderState {
    #[allow(clippy::too_many_arguments)]
    fn pending(
        signer: EvmAddress,
        funder: EvmAddress,
        expected_order_id: FixedOrderId,
        side: PmOrderSide,
        price: PmPrice,
        quantity: PmQuantity,
    ) -> Result<Self, PredarbProductionOrderErrorV1> {
        let account = PmAccountHandle::from_ordinal(0);
        let account_scope = PmAccountScope::new(
            PmEnvironmentId::new("polymarket-production")
                .map_err(|_| PredarbProductionOrderErrorV1::OrderState)?,
            PmChainId::new(137).map_err(|_| PredarbProductionOrderErrorV1::OrderState)?,
            PmSignerId::new(signer),
            PmFunderId::new(funder),
            account,
        );
        let instrument = PmInstrumentHandle::new(
            PmMarketHandle::from_ordinal(0),
            PmTokenHandle::from_ordinal(0),
        );
        let client_order = PmClientOrderKey::new(
            account,
            PmClientOrderId::from_bytes(
                expected_order_id.bytes()[..16]
                    .try_into()
                    .map_err(|_| PredarbProductionOrderErrorV1::OrderState)?,
            )
            .map_err(|_| PredarbProductionOrderErrorV1::OrderState)?,
        );
        let venue_order_id = expected_order_id.to_string();
        let venue_order = PmVenueOrderKey::new(
            account,
            PmVenueOrderId::new(&venue_order_id)
                .map_err(|_| PredarbProductionOrderErrorV1::OrderState)?,
        );
        let maker = exact_order_amounts(side, price, quantity)
            .map_err(|_| PredarbProductionOrderErrorV1::OrderState)?
            .maker();
        let reservation = match side {
            PmOrderSide::Buy => PmExactReservation::policy_approved(maker, U256::ZERO),
            PmOrderSide::Sell => PmExactReservation::policy_approved(U256::ZERO, maker),
        }
        .map_err(|_| PredarbProductionOrderErrorV1::OrderState)?;
        let quote = PmOwnedQuoteIntent::new(
            PmOwnedIntentId::new(1).map_err(|_| PredarbProductionOrderErrorV1::OrderState)?,
            PmOwnedQuoteSlotKey::new(account_scope, instrument, side),
            client_order,
            price,
            quantity,
            reservation,
        )
        .map_err(|_| PredarbProductionOrderErrorV1::OrderState)?;
        let mut lifecycle = PmOwnedOrderLifecycle::new(account_scope, instrument);
        lifecycle
            .begin_epoch(ConnectionEpoch::new(1))
            .map_err(|_| PredarbProductionOrderErrorV1::OrderState)?;
        if lifecycle
            .admit_quote(quote)
            .map_err(|_| PredarbProductionOrderErrorV1::OrderState)?
            != PmOwnedQuoteAdmission::Admitted(client_order)
        {
            return Err(PredarbProductionOrderErrorV1::OrderState);
        }
        Ok(Self {
            lifecycle,
            client_order,
            venue_order,
            next_reduction_sequence: 1,
        })
    }

    fn apply_place(
        &mut self,
        classification: PmMutationClassification,
    ) -> Result<(), PredarbProductionOrderErrorV1> {
        let result = match classification {
            PmMutationClassification::Accepted => PmOwnedSubmitResult::Accepted(self.venue_order),
            PmMutationClassification::Rejected
            | PmMutationClassification::DefinitelyNotDispatched => PmOwnedSubmitResult::Rejected,
            PmMutationClassification::OutOfProfile
            | PmMutationClassification::AcknowledgementUnknown => {
                PmOwnedSubmitResult::AmbiguousOwned(self.venue_order)
            }
        };
        self.lifecycle
            .apply_submit_result(self.client_order, result)
            .map_err(|_| PredarbProductionOrderErrorV1::OrderState)?;
        Ok(())
    }

    fn request_cancel(
        &mut self,
    ) -> Result<reap_pm_state::PmOwnedCancelIntent, PredarbProductionOrderErrorV1> {
        match self
            .lifecycle
            .request_cancel(self.client_order)
            .map_err(|_| PredarbProductionOrderErrorV1::OrderState)?
        {
            PmOwnedCancelRequestApply::Issued(intent)
            | PmOwnedCancelRequestApply::Duplicate(intent) => Ok(intent),
            PmOwnedCancelRequestApply::AlreadyTerminal => {
                Err(PredarbProductionOrderErrorV1::OrderState)
            }
        }
    }

    fn apply_cancel(
        &mut self,
        intent: reap_pm_state::PmOwnedCancelIntent,
        classification: PmMutationClassification,
    ) -> Result<(), PredarbProductionOrderErrorV1> {
        let outcome = match classification {
            PmMutationClassification::Accepted => PmOwnedCancelOutcome::Accepted,
            PmMutationClassification::Rejected
            | PmMutationClassification::DefinitelyNotDispatched => PmOwnedCancelOutcome::Rejected,
            PmMutationClassification::OutOfProfile
            | PmMutationClassification::AcknowledgementUnknown => PmOwnedCancelOutcome::Ambiguous,
        };
        self.lifecycle
            .apply_cancel_result(intent, outcome)
            .map_err(|_| PredarbProductionOrderErrorV1::OrderState)?;
        Ok(())
    }

    fn reconcile_exact(
        &mut self,
        observation: &PmExactOrderObservation,
    ) -> Result<ExactOrderStateReconciliationReport, PredarbProductionOrderErrorV1> {
        let occurrence = PmOwnedObservationOccurrence::immediate(
            PmOwnedReductionSequence::new(self.next_reduction_sequence)
                .map_err(|_| PredarbProductionOrderErrorV1::OrderState)?,
        );
        self.next_reduction_sequence = self
            .next_reduction_sequence
            .checked_add(1)
            .ok_or(PredarbProductionOrderErrorV1::OrderState)?;
        match observation {
            PmExactOrderObservation::Absent => {
                let state_applied = self
                    .lifecycle
                    .observe_detail_absence(self.venue_order, occurrence)
                    .map_err(|_| PredarbProductionOrderErrorV1::OrderState)?
                    == PmOwnedDetailAbsenceApply::SettledAcceptedCancel(self.client_order);
                Ok(ExactOrderStateReconciliationReport {
                    classification: "absent",
                    status: None,
                    original_size: None,
                    size_matched: None,
                    state_applied,
                })
            }
            PmExactOrderObservation::Present(order) => {
                let current = self.projection()?;
                if order.id().as_str() != self.venue_order.id().as_str()
                    || order.side() != current.side
                    || order.price() != current.price
                    || order.original_size() != current.quantity
                {
                    return Err(PredarbProductionOrderErrorV1::OrderState);
                }
                let cumulative = book_quantity_units(order.size_matched());
                let progress =
                    exact_order_progress(order.status(), order.original_size(), cumulative)?;
                self.lifecycle
                    .observe_progress(PmOwnedOrderProgressObservation::new(
                        self.client_order,
                        self.venue_order,
                        progress,
                        occurrence,
                        PmOwnedObservationSource::RestReconciliation,
                    ))
                    .map_err(|_| PredarbProductionOrderErrorV1::OrderState)?;
                Ok(ExactOrderStateReconciliationReport {
                    classification: "present",
                    status: Some(order.status().to_owned()),
                    original_size: Some(order.original_size().to_string()),
                    size_matched: Some(book_quantity_string(order.size_matched())),
                    state_applied: true,
                })
            }
        }
    }

    fn apply_polled_fills(
        &mut self,
        fills: &[ObservedOwnedFill],
    ) -> Result<(), PredarbProductionOrderErrorV1> {
        for fill in fills {
            let occurrence = PmOwnedObservationOccurrence::immediate(
                PmOwnedReductionSequence::new(self.next_reduction_sequence)
                    .map_err(|_| PredarbProductionOrderErrorV1::OrderState)?,
            );
            self.next_reduction_sequence = self
                .next_reduction_sequence
                .checked_add(1)
                .ok_or(PredarbProductionOrderErrorV1::OrderState)?;
            let observation = PmOwnedFillObservation::new(
                PmFillKey::new(self.venue_order, fill.id),
                fill.quantity,
                None,
                occurrence,
                PmOwnedObservationSource::RestReconciliation,
            )
            .map_err(|_| PredarbProductionOrderErrorV1::OrderState)?;
            self.lifecycle
                .observe_fill(observation)
                .map_err(|_| PredarbProductionOrderErrorV1::OrderState)?;
        }
        Ok(())
    }

    fn projection(&self) -> Result<ProductionOwnedOrderProjection, PredarbProductionOrderErrorV1> {
        let order = self
            .lifecycle
            .order(self.client_order)
            .ok_or(PredarbProductionOrderErrorV1::OrderState)?;
        Ok(ProductionOwnedOrderProjection {
            side: order.slot().side(),
            price: order.price(),
            quantity: order.quantity(),
            cumulative_filled: order.cumulative_filled(),
            known_fill_total: order.known_fill_total(),
            report: canonical_state_report(order),
        })
    }

    fn report(&self) -> Result<CanonicalOrderStateReport, PredarbProductionOrderErrorV1> {
        self.projection().map(|projection| projection.report)
    }
}

struct ProductionOwnedOrderProjection {
    side: PmOrderSide,
    price: PmPrice,
    quantity: PmQuantity,
    cumulative_filled: U256,
    known_fill_total: U256,
    report: CanonicalOrderStateReport,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
struct ObservedOwnedFill {
    id: PmFillId,
    quantity: PmQuantity,
    price: PmPrice,
    status: String,
    role: &'static str,
}

struct OwnedFillCutObservation {
    pages: usize,
    account_rows: usize,
    fills: Vec<ObservedOwnedFill>,
}

pub(crate) async fn run_authorized_predarb_place_then_cancel_v1(
    request: PredarbProductionOrderRequestV1,
) -> Result<PredarbProductionOrderReportV1, PredarbProductionOrderErrorV1> {
    run_authorized_predarb_production_order_v1(request, ProductionOrderTrialMode::FarPostOnlyCancel)
        .await
}

pub(crate) async fn run_authorized_predarb_minimum_fill_v1(
    request: PredarbProductionOrderRequestV1,
) -> Result<PredarbProductionOrderReportV1, PredarbProductionOrderErrorV1> {
    run_authorized_predarb_production_order_v1(request, ProductionOrderTrialMode::MinimumFill).await
}

async fn run_authorized_predarb_production_order_v1(
    request: PredarbProductionOrderRequestV1,
    mode: ProductionOrderTrialMode,
) -> Result<PredarbProductionOrderReportV1, PredarbProductionOrderErrorV1> {
    if request.authorization_phrase != AUTHORIZATION_PHRASE {
        return Err(PredarbProductionOrderErrorV1::AuthorizationPhrase);
    }

    let (fixed_peer, local_egress) = production_connectivity(
        &request.fixed_peer_ip,
        &request.interface_name,
        request.local_source_ip,
    )?;
    let discovery = PmBtcFiveMinuteMarketSource::production_on_selected_local_egress(
        CONNECT_TIMEOUT,
        REQUEST_TIMEOUT,
        &local_egress,
    )
    .map_err(|_| PredarbProductionOrderErrorV1::MarketDiscovery)?;
    let market = discovery
        .discover_current()
        .await
        .map_err(|_| PredarbProductionOrderErrorV1::MarketDiscovery)?;

    let up_scope = PmWireScope::new(market.condition(), market.market(), market.up_token());
    let down_scope = PmWireScope::new(market.condition(), market.market(), market.down_token());
    let up_book = fetch_fresh_book(&fixed_peer, &local_egress, &market, up_scope).await?;
    let down_book = fetch_fresh_book(&fixed_peer, &local_egress, &market, down_scope).await?;
    let up_ask = best_ask(&up_book.snapshot)?;
    let down_ask = best_ask(&down_book.snapshot)?;
    let (outcome, token_id, exact_scope) = if up_ask >= down_ask {
        ("Up", market.up_token(), up_scope)
    } else {
        ("Down", market.down_token(), down_scope)
    };
    let mut credential = load_predarb_credentials(&request.credential_env)?;
    if credential.signature_type != "1" {
        return Err(PredarbProductionOrderErrorV1::CredentialProfile);
    }

    let signer = FixedEoaSigner::derive(EoaPrivateKeyInput::new(std::mem::take(
        &mut *credential.private_key,
    )))
    .map_err(|_| PredarbProductionOrderErrorV1::CredentialBinding)?;
    let signer_address = signer.address();
    let signer_text = signer_address.to_string();
    let funder = LegacyType1ProxyAddress::parse(&credential.funder)
        .map_err(|_| PredarbProductionOrderErrorV1::CredentialBinding)?;
    if !legacy_type1_proxy_address_matches(signer_address, funder) {
        return Err(PredarbProductionOrderErrorV1::CredentialBinding);
    }
    let proxy_funder = EvmAddress::from_bytes(funder.bytes())
        .map_err(|_| PredarbProductionOrderErrorV1::CredentialBinding)?;
    let l2 = L2Credentials::bind(
        &signer_text,
        L2CredentialInput::new(
            std::mem::take(&mut *credential.api_key),
            std::mem::take(&mut *credential.api_secret),
            std::mem::take(&mut *credential.api_passphrase),
        ),
    )
    .map_err(|_| PredarbProductionOrderErrorV1::CredentialBinding)?;
    drop(credential);

    let position_source = PmDataApiCurrentPositionSource::production_on_selected_local_egress(
        PmDataApiPositionScope::new(proxy_funder, market.condition(), token_id),
        CONNECT_TIMEOUT,
        REQUEST_TIMEOUT,
        &local_egress,
    )
    .map_err(|_| PredarbProductionOrderErrorV1::PositionPreflight)?;
    let position_before = position_source
        .production_observe_configured_token()
        .await
        .map_err(|_| PredarbProductionOrderErrorV1::PositionPreflight)?;
    let position_before_units = configured_position_units(&position_before)
        .map_err(|_| PredarbProductionOrderErrorV1::PositionPreflight)?;
    let position_before = position_report(&position_before, position_before_units);

    // Refresh the chosen outcome after discovery, two-sided selection and the
    // pre-order position read. This is the sole book allowed to authorize the
    // fixed far-away price below.
    let book = fetch_fresh_book(&fixed_peer, &local_egress, &market, exact_scope).await?;
    let best_bid = best_bid(&book.snapshot)?;
    let best_ask = best_ask(&book.snapshot)?;
    let price = match mode {
        ProductionOrderTrialMode::FarPostOnlyCancel => TEST_PRICE
            .parse::<PmPrice>()
            .map_err(|_| PredarbProductionOrderErrorV1::OrderProfile)?,
        ProductionOrderTrialMode::MinimumFill => best_ask,
    };
    price
        .validate_tick(market.tick())
        .map_err(|_| PredarbProductionOrderErrorV1::OrderProfile)?;
    if mode == ProductionOrderTrialMode::FarPostOnlyCancel
        && best_ask.units().saturating_sub(price.units()) < MINIMUM_FAR_DISTANCE_UNITS
    {
        return Err(PredarbProductionOrderErrorV1::BookPreflight);
    }
    let quantity = match mode {
        ProductionOrderTrialMode::FarPostOnlyCancel => MAX_TEST_QUANTITY
            .parse::<PmQuantity>()
            .map_err(|_| PredarbProductionOrderErrorV1::OrderProfile)?,
        ProductionOrderTrialMode::MinimumFill => market.minimum_order_size(),
    };
    let maximum_quantity = MAX_TEST_QUANTITY
        .parse::<PmQuantity>()
        .map_err(|_| PredarbProductionOrderErrorV1::OrderProfile)?;
    if quantity > maximum_quantity || quantity < market.minimum_order_size() {
        return Err(PredarbProductionOrderErrorV1::OrderProfile);
    }
    if mode == ProductionOrderTrialMode::MinimumFill
        && best_ask_quantity(&book.snapshot)? < quantity
    {
        return Err(PredarbProductionOrderErrorV1::BookPreflight);
    }
    let required_collateral = exact_order_amounts(PmOrderSide::Buy, price, quantity)
        .map_err(|_| PredarbProductionOrderErrorV1::OrderProfile)?
        .maker();
    let collateral_balance = observe_predarb_collateral_balance(
        &request.credential_env,
        signer_address.as_core(),
        proxy_funder,
        token_id,
    )
    .await?;
    if collateral_balance < required_collateral {
        return Err(PredarbProductionOrderErrorV1::OrderProfile);
    }
    ensure_window_remaining(&market)?;
    let side = PmOrderSide::Buy;
    let (order_timestamp_ms, order_timestamp_seconds) = current_timestamps()?;
    let salt = PmOrderSalt::from_u64(order_timestamp_ms)
        .map_err(|_| PredarbProductionOrderErrorV1::Clock)?;
    let order = PmUnsignedClobV2Order::new_pm_t2_proxy(
        salt,
        proxy_funder,
        signer_address.as_core(),
        token_id,
        side,
        price,
        quantity,
        market.tick(),
        market.minimum_order_size(),
        order_timestamp_ms,
    )
    .map_err(|_| PredarbProductionOrderErrorV1::OrderProfile)?;
    let domain = if market.negative_risk() {
        PmClobDomain::NegativeRisk
    } else {
        PmClobDomain::Standard
    };
    let public_identity = match mode {
        ProductionOrderTrialMode::FarPostOnlyCancel => {
            derive_place_public_request_identity(domain, order)
        }
        ProductionOrderTrialMode::MinimumFill => {
            derive_gtc_fill_place_public_request_identity(domain, order)
        }
    };
    let expected_order_id = public_identity.expected_order_id();
    let fixed_expected_order_id: FixedOrderId = expected_order_id.into();
    // Match the OKX invariant: canonical PendingNew ownership exists before
    // any request can reach the transport. PM keeps its exact numeric and
    // ambiguity states in the venue-specific owned lifecycle.
    let mut order_state = ProductionOwnedOrderState::pending(
        signer_address.as_core(),
        proxy_funder,
        fixed_expected_order_id,
        side,
        price,
        quantity,
    )?;
    let mut canonical_order_state = order_state.report()?;
    let mut canonical_order_state_consistent = true;

    let transport_config =
        PmProductionMutationConfig::production_on_fixed_tls_peer_and_selected_local_egress(
            CONNECT_TIMEOUT,
            REQUEST_TIMEOUT,
            fixed_peer,
            local_egress,
        )
        .map_err(|_| PredarbProductionOrderErrorV1::TransportProfile)?;
    let mut cancel_transport = PmExactOwnedCancelProductionRole::new(transport_config.clone())
        .map_err(|_| PredarbProductionOrderErrorV1::TransportProfile)?;

    let mut ledger = OneShotLedger::create(&request.state_directory)?;
    ledger.append(&json!({
        "schema_version": 1,
        "event": "place_then_exact_cancel_authorized",
        "order_profile": mode.name(),
        "production_order_entry_authorized": true,
        "real_order_submission_authorized": true,
        "place_dispatch_allowance": 1,
        "placement_resumption_allowed": false,
        "exact_cancel_precommitted": true,
        "post_only": mode.post_only(),
        "expected_order_id": expected_order_id.to_string(),
        "semantic_request_commitment": public_identity.semantic_request_commitment().to_string(),
        "market_slug": market.slug(),
        "market_title": market.title(),
        "condition_id": market.condition().to_string(),
        "question_id": market.market().to_string(),
        "window_start_epoch": market.window_start_epoch(),
        "window_end_epoch": market.window_end_epoch(),
        "outcome": outcome,
        "token_id": token_id.units().to_string(),
        "side": canonical_side(side),
        "price": price.to_string(),
        "quantity": quantity.to_string(),
        "hard_maximum_quantity": MAX_TEST_QUANTITY,
        "tick": market.tick().to_string(),
        "minimum_order_size": market.minimum_order_size().to_string(),
        "negative_risk": market.negative_risk(),
        "fresh_best_bid": best_bid.to_string(),
        "fresh_best_ask": best_ask.to_string(),
        "collateral_balance_protocol_units": collateral_balance.to_string(),
        "required_collateral_protocol_units": required_collateral.to_string(),
        "minimum_far_distance_units": if mode == ProductionOrderTrialMode::FarPostOnlyCancel {
            Some(MINIMUM_FAR_DISTANCE_UNITS)
        } else {
            None
        },
        "book_timestamp_millis": book.snapshot.timestamp_millis(),
        "book_hash": book.snapshot.verified_hash().to_string(),
        "book_receive_wall_ns": book.received.local_wall_receive_ns(),
        "position_before": &position_before,
        "order_timestamp_ms": order_timestamp_ms,
        "canonical_order_state": &canonical_order_state,
    }))?;

    let place_l2_timestamp = L2Timestamp::from_unix_seconds(order_timestamp_seconds)
        .map_err(|_| PredarbProductionOrderErrorV1::Clock)?;
    let place = match mode {
        ProductionOrderTrialMode::FarPostOnlyCancel => {
            let signed = signer
                .sign_clob_v2_order(domain, order)
                .map_err(|_| PredarbProductionOrderErrorV1::Authentication)?;
            let serialized = l2
                .serialize_gtc_post_only(signed)
                .map_err(|_| PredarbProductionOrderErrorV1::Authentication)?;
            let authenticated = l2
                .authenticate_place(place_l2_timestamp, serialized)
                .map_err(|_| PredarbProductionOrderErrorV1::Authentication)?;
            let retained = PmRetainedPlaceRequest::retain(authenticated)
                .map_err(|_| PredarbProductionOrderErrorV1::Authentication)?;
            ledger.append(&json!({
                "schema_version": 1,
                "event": "place_dispatch_started",
                "order_profile": mode.name(),
                "expected_order_id": expected_order_id.to_string(),
                "l2_timestamp_seconds": retained.l2_timestamp_seconds(),
            }))?;
            let mut place_transport = PmFixedPlaceProductionRole::new(transport_config.clone())
                .map_err(|_| PredarbProductionOrderErrorV1::TransportProfile)?;
            place_transport.send(retained).await
        }
        ProductionOrderTrialMode::MinimumFill => {
            let signed = signer
                .sign_gtc_fill_clob_v2_order(domain, order)
                .map_err(|_| PredarbProductionOrderErrorV1::Authentication)?;
            let serialized = l2
                .serialize_gtc_fill(signed)
                .map_err(|_| PredarbProductionOrderErrorV1::Authentication)?;
            let authenticated = l2
                .authenticate_gtc_fill_place(place_l2_timestamp, serialized)
                .map_err(|_| PredarbProductionOrderErrorV1::Authentication)?;
            let retained = PmRetainedGtcFillPlaceRequest::retain(authenticated)
                .map_err(|_| PredarbProductionOrderErrorV1::Authentication)?;
            ledger.append(&json!({
                "schema_version": 1,
                "event": "place_dispatch_started",
                "order_profile": mode.name(),
                "expected_order_id": expected_order_id.to_string(),
                "l2_timestamp_seconds": retained.l2_timestamp_seconds(),
            }))?;
            let mut marketable_transport =
                PmFixedGtcFillPlaceProductionRole::new(transport_config.clone())
                    .map_err(|_| PredarbProductionOrderErrorV1::TransportProfile)?;
            marketable_transport.send(retained).await
        }
    };
    drop(signer);
    let place_classification = classification_name(place.classification());
    let place_diagnostic = diagnostic_name(place.diagnostic().kind());
    let place_status = place.diagnostic().http_status();
    let place_response_bytes = place.diagnostic().response_bytes();
    let place_observed = place.observed_order_id().map(|id| id.as_str().to_owned());
    if order_state.apply_place(place.classification()).is_err() {
        canonical_order_state_consistent = false;
    } else if let Ok(report) = order_state.report() {
        canonical_order_state = report;
    } else {
        canonical_order_state_consistent = false;
    }
    let place_recorded = ledger.append(&json!({
        "schema_version": 1,
        "event": "place_dispatch_result",
        "expected_order_id": expected_order_id.to_string(),
        "classification": place_classification,
        "diagnostic": place_diagnostic,
        "http_status": place_status,
        "response_bytes": place_response_bytes,
        "observed_order_id": place_observed,
        "canonical_order_state": &canonical_order_state,
        "canonical_order_state_consistent": canonical_order_state_consistent,
    }));

    let cancel_needed = matches!(
        place.classification(),
        PmMutationClassification::Accepted
            | PmMutationClassification::OutOfProfile
            | PmMutationClassification::AcknowledgementUnknown
    );
    let mut cancel_classification = None;
    let mut cancel_diagnostic = None;
    let mut cancel_status = None;
    let mut cancel_response_bytes = None;
    let mut exact_order_reconciliation = None;
    if cancel_needed {
        let cancel_intent = order_state.request_cancel().ok();
        if cancel_intent.is_none() {
            canonical_order_state_consistent = false;
        } else if let Ok(report) = order_state.report() {
            canonical_order_state = report;
        } else {
            canonical_order_state_consistent = false;
        }
        let cancel_timestamp = current_l2_timestamp()?;
        let serialized_cancel = l2
            .serialize_owned_cancel(expected_order_id.into())
            .map_err(|_| PredarbProductionOrderErrorV1::Authentication)?;
        let authenticated_cancel = l2
            .authenticate_owned_cancel(cancel_timestamp, serialized_cancel)
            .map_err(|_| PredarbProductionOrderErrorV1::Authentication)?;
        let retained_cancel = PmRetainedOwnedCancelRequest::retain(authenticated_cancel)
            .map_err(|_| PredarbProductionOrderErrorV1::Authentication)?;
        // The initial durable record already precommitted this exact cancel,
        // so a post-place ledger fault cannot strand an accepted order merely
        // by preventing an additional observation line.
        let _ = ledger.append(&json!({
            "schema_version": 1,
            "event": "exact_cancel_dispatch_started",
            "order_id": expected_order_id.to_string(),
            "l2_timestamp_seconds": retained_cancel.l2_timestamp_seconds(),
            "canonical_order_state": &canonical_order_state,
            "canonical_order_state_consistent": canonical_order_state_consistent,
        }));
        let cancel = cancel_transport.send(retained_cancel).await;
        cancel_classification = Some(classification_name(cancel.classification()));
        cancel_diagnostic = Some(diagnostic_name(cancel.diagnostic().kind()));
        cancel_status = cancel.diagnostic().http_status();
        cancel_response_bytes = Some(cancel.diagnostic().response_bytes());
        let cancel_observed = cancel.observed_order_id().map(|id| id.as_str().to_owned());
        if cancel_intent.is_none_or(|intent| {
            order_state
                .apply_cancel(intent, cancel.classification())
                .is_err()
        }) {
            canonical_order_state_consistent = false;
        } else if let Ok(report) = order_state.report() {
            canonical_order_state = report;
        } else {
            canonical_order_state_consistent = false;
        }
        let _ = ledger.append(&json!({
            "schema_version": 1,
            "event": "exact_cancel_dispatch_result",
            "order_id": expected_order_id.to_string(),
            "classification": cancel_classification,
            "diagnostic": cancel_diagnostic,
            "http_status": cancel_status,
            "response_bytes": cancel_response_bytes,
            "observed_order_id": cancel_observed,
            "canonical_order_state": &canonical_order_state,
            "canonical_order_state_consistent": canonical_order_state_consistent,
        }));
    }
    drop(l2);

    if cancel_needed {
        if let Ok(observation) = observe_predarb_exact_order(
            &request.credential_env,
            exact_scope,
            fixed_expected_order_id,
        )
        .await
        {
            match order_state.reconcile_exact(&observation) {
                Ok(reconciliation) => {
                    if let Ok(report) = order_state.report() {
                        canonical_order_state = report;
                        exact_order_reconciliation = Some(reconciliation);
                    } else {
                        canonical_order_state_consistent = false;
                    }
                }
                Err(_) => canonical_order_state_consistent = false,
            }
        }
        let _ = ledger.append(&json!({
            "schema_version": 1,
            "event": "exact_order_state_reconciliation",
            "order_id": expected_order_id.to_string(),
            "observation": &exact_order_reconciliation,
            "canonical_order_state": &canonical_order_state,
            "canonical_order_state_consistent": canonical_order_state_consistent,
        }));
    }

    let mut position_after = None;
    let mut position_unchanged = None;
    let mut fill_position_reconciliation = None;
    if cancel_needed {
        for attempt in 1..=MAX_FILL_POSITION_RECONCILIATION_ATTEMPTS {
            let fill_cut = observe_predarb_owned_fills(
                &request.credential_env,
                exact_scope,
                fixed_expected_order_id,
                side,
                price,
            )
            .await;
            let fill_cut_available = fill_cut.is_ok();
            let venue_position = position_source.production_observe_configured_token().await;
            let venue_position_units = venue_position
                .as_ref()
                .ok()
                .and_then(|observation| configured_position_units(observation).ok());
            if let (Ok(observation), Some(units)) = (&venue_position, venue_position_units) {
                position_after = Some(position_report(observation, units));
                position_unchanged = Some(units == position_before_units);
            }
            if let Ok(fill_cut) = fill_cut {
                if order_state.apply_polled_fills(&fill_cut.fills).is_err() {
                    canonical_order_state_consistent = false;
                } else if let Ok(projection) = order_state.projection() {
                    canonical_order_state = projection.report.clone();
                    let _ = ledger.append(&json!({
                        "schema_version": 1,
                        "event": "authenticated_owned_fill_cut",
                        "order_id": expected_order_id.to_string(),
                        "trade_pages": fill_cut.pages,
                        "account_trade_rows": fill_cut.account_rows,
                        "fills": &fill_cut.fills,
                        "canonical_order_state": &canonical_order_state,
                        "canonical_order_state_consistent": canonical_order_state_consistent,
                    }));
                    if let Some(units) = venue_position_units {
                        let reconciliation = fill_position_reconciliation_report(
                            attempt,
                            &fill_cut,
                            &projection,
                            position_before_units,
                            units,
                        )?;
                        let converged = reconciliation.converged;
                        let _ = ledger.append(&json!({
                            "schema_version": 1,
                            "event": "fill_position_reconciliation",
                            "order_id": expected_order_id.to_string(),
                            "fills": &fill_cut.fills,
                            "reconciliation": &reconciliation,
                            "position_after": &position_after,
                            "canonical_order_state": &canonical_order_state,
                            "canonical_order_state_consistent": canonical_order_state_consistent,
                        }));
                        fill_position_reconciliation = Some(reconciliation);
                        if converged {
                            break;
                        }
                    }
                } else {
                    canonical_order_state_consistent = false;
                }
            }
            let _ = ledger.append(&json!({
                "schema_version": 1,
                "event": "fill_position_reconciliation_pending",
                "order_id": expected_order_id.to_string(),
                "attempt": attempt,
                "fill_cut_available": fill_cut_available,
                "position_after": &position_after,
                "canonical_order_state": &canonical_order_state,
                "canonical_order_state_consistent": canonical_order_state_consistent,
            }));
            if attempt < MAX_FILL_POSITION_RECONCILIATION_ATTEMPTS {
                tokio::time::sleep(FILL_POSITION_RECONCILIATION_INTERVAL).await;
            }
        }
    } else if let Ok(observation) = position_source.production_observe_configured_token().await
        && let Ok(units) = configured_position_units(&observation)
    {
        position_after = Some(position_report(&observation, units));
        position_unchanged = Some(units == position_before_units);
    }
    let _ = ledger.append(&json!({
        "schema_version": 1,
        "event": "post_cancel_position_observation",
        "position_after": &position_after,
        "position_unchanged": position_unchanged,
        "fill_position_reconciliation": &fill_position_reconciliation,
    }));

    let manual_reconciliation_required = place_recorded.is_err()
        || !canonical_order_state_consistent
        || place.classification() == PmMutationClassification::OutOfProfile
        || cancel_classification
            == Some(classification_name(PmMutationClassification::OutOfProfile))
        || !canonical_order_state.is_terminal
        || canonical_order_state.reconciliation_required
        || (cancel_needed
            && !fill_position_reconciliation
                .as_ref()
                .is_some_and(|reconciliation| reconciliation.converged))
        || (!cancel_needed && position_unchanged != Some(true));

    Ok(PredarbProductionOrderReportV1 {
        ledger_path: ledger.path,
        order_profile: mode.name(),
        market_slug: market.slug().to_owned(),
        market_title: market.title().to_owned(),
        condition_id: market.condition().to_string(),
        outcome,
        token_id: token_id.units().to_string(),
        price: price.to_string(),
        quantity: quantity.to_string(),
        fresh_best_bid: best_bid.to_string(),
        fresh_best_ask: best_ask.to_string(),
        collateral_balance_protocol_units: collateral_balance.to_string(),
        book_timestamp_millis: book.snapshot.timestamp_millis(),
        book_hash: book.snapshot.verified_hash().to_string(),
        expected_order_id: expected_order_id.to_string(),
        place_classification,
        place_diagnostic,
        place_http_status: place_status,
        place_response_bytes,
        cancel_attempted: cancel_needed,
        cancel_classification,
        cancel_diagnostic,
        cancel_http_status: cancel_status,
        cancel_response_bytes,
        position_before,
        position_after,
        position_unchanged,
        fill_position_reconciliation,
        canonical_order_state,
        canonical_order_state_consistent,
        exact_order_reconciliation,
        manual_reconciliation_required,
    })
}

pub(crate) async fn reconcile_predarb_exact_order_v1(
    request: PredarbExactOrderReconciliationRequestV1,
) -> Result<PredarbExactOrderReconciliationReportV1, PredarbProductionOrderErrorV1> {
    let condition = PmConditionId::parse(&request.condition_id)
        .map_err(|_| PredarbProductionOrderErrorV1::Reconciliation)?;
    let question = PmMarketId::parse(&request.question_id)
        .map_err(|_| PredarbProductionOrderErrorV1::Reconciliation)?;
    let token = PmTokenId::new(
        request
            .token_id
            .parse::<U256>()
            .map_err(|_| PredarbProductionOrderErrorV1::Reconciliation)?,
    )
    .map_err(|_| PredarbProductionOrderErrorV1::Reconciliation)?;
    let order_id = FixedOrderId::parse(&request.order_id)
        .map_err(|_| PredarbProductionOrderErrorV1::Reconciliation)?;
    let exact_scope = PmWireScope::new(condition, question, token);
    let observation =
        observe_predarb_exact_order(&request.credential_env, exact_scope, order_id).await?;

    Ok(match observation {
        PmExactOrderObservation::Absent => PredarbExactOrderReconciliationReportV1 {
            order_id: order_id.to_string(),
            classification: "absent",
            status: None,
            original_size: None,
            size_matched: None,
            price: None,
            side: None,
            cancellation_verified: false,
        },
        PmExactOrderObservation::Present(order) => {
            let status = order.status().to_owned();
            PredarbExactOrderReconciliationReportV1 {
                order_id: order_id.to_string(),
                classification: "present",
                cancellation_verified: is_cancelled_status(&status),
                status: Some(status),
                original_size: Some(order.original_size().to_string()),
                size_matched: Some(book_quantity_string(order.size_matched())),
                price: Some(order.price().to_string()),
                side: Some(canonical_side(order.side())),
            }
        }
    })
}

pub(crate) async fn reconcile_predarb_owned_fill_position_v1(
    request: PredarbOwnedFillPositionReconciliationRequestV1,
) -> Result<PredarbOwnedFillPositionReconciliationReportV1, PredarbProductionOrderErrorV1> {
    let condition = PmConditionId::parse(&request.condition_id)
        .map_err(|_| PredarbProductionOrderErrorV1::Reconciliation)?;
    let question = PmMarketId::parse(&request.question_id)
        .map_err(|_| PredarbProductionOrderErrorV1::Reconciliation)?;
    let token = PmTokenId::new(
        request
            .token_id
            .parse::<U256>()
            .map_err(|_| PredarbProductionOrderErrorV1::Reconciliation)?,
    )
    .map_err(|_| PredarbProductionOrderErrorV1::Reconciliation)?;
    let order_id = FixedOrderId::parse(&request.order_id)
        .map_err(|_| PredarbProductionOrderErrorV1::Reconciliation)?;
    let price = request
        .price
        .parse::<PmPrice>()
        .map_err(|_| PredarbProductionOrderErrorV1::Reconciliation)?;
    let quantity = request
        .quantity
        .parse::<PmQuantity>()
        .map_err(|_| PredarbProductionOrderErrorV1::Reconciliation)?;
    let position_before = request
        .position_before_protocol_units
        .parse::<U256>()
        .map_err(|_| PredarbProductionOrderErrorV1::Reconciliation)?;
    let exact_scope = PmWireScope::new(condition, question, token);

    let mut credential = load_predarb_credentials(&request.credential_env)?;
    if credential.signature_type != "1" {
        return Err(PredarbProductionOrderErrorV1::CredentialProfile);
    }
    let signer = FixedEoaSigner::derive(EoaPrivateKeyInput::new(std::mem::take(
        &mut *credential.private_key,
    )))
    .map_err(|_| PredarbProductionOrderErrorV1::CredentialBinding)?;
    let signer_address = signer.address().as_core();
    let funder = LegacyType1ProxyAddress::parse(&credential.funder)
        .map_err(|_| PredarbProductionOrderErrorV1::CredentialBinding)?;
    if !legacy_type1_proxy_address_matches(signer.address(), funder) {
        return Err(PredarbProductionOrderErrorV1::CredentialBinding);
    }
    let proxy_funder = EvmAddress::from_bytes(funder.bytes())
        .map_err(|_| PredarbProductionOrderErrorV1::CredentialBinding)?;
    drop(credential);
    drop(signer);

    let exact = observe_predarb_exact_order(&request.credential_env, exact_scope, order_id).await?;
    let fills = observe_predarb_owned_fills(
        &request.credential_env,
        exact_scope,
        order_id,
        PmOrderSide::Buy,
        price,
    )
    .await?;
    let position_source = PmDataApiCurrentPositionSource::production(
        PmDataApiPositionScope::new(proxy_funder, condition, token),
        CONNECT_TIMEOUT,
        REQUEST_TIMEOUT,
    )
    .map_err(|_| PredarbProductionOrderErrorV1::PositionPreflight)?;
    let position = position_source
        .production_observe_configured_token()
        .await
        .map_err(|_| PredarbProductionOrderErrorV1::PositionPreflight)?;
    let position_units = configured_position_units(&position)?;
    let collateral_balance = observe_predarb_collateral_balance(
        &request.credential_env,
        signer_address,
        proxy_funder,
        token,
    )
    .await?;

    let mut state = ProductionOwnedOrderState::pending(
        signer_address,
        proxy_funder,
        order_id,
        PmOrderSide::Buy,
        price,
        quantity,
    )?;
    state.apply_place(PmMutationClassification::Accepted)?;
    let exact_order = state.reconcile_exact(&exact)?;
    state.apply_polled_fills(&fills.fills)?;
    let projection = state.projection()?;
    let reconciliation = fill_position_reconciliation_report(
        1,
        &fills,
        &projection,
        position_before,
        position_units,
    )?;
    Ok(PredarbOwnedFillPositionReconciliationReportV1 {
        order_id: order_id.to_string(),
        exact_order,
        fill_position_reconciliation: reconciliation,
        canonical_order_state: projection.report,
        position_after: position_report(&position, position_units),
        collateral_balance_protocol_units: collateral_balance.to_string(),
    })
}

async fn observe_predarb_collateral_balance(
    credential_env: &Path,
    expected_signer: EvmAddress,
    expected_funder: EvmAddress,
    token_id: PmTokenId,
) -> Result<U256, PredarbProductionOrderErrorV1> {
    let mut credential = load_predarb_credentials(credential_env)?;
    if credential.signature_type != "1" {
        return Err(PredarbProductionOrderErrorV1::CredentialProfile);
    }
    let signer = FixedEoaSigner::derive(EoaPrivateKeyInput::new(std::mem::take(
        &mut *credential.private_key,
    )))
    .map_err(|_| PredarbProductionOrderErrorV1::CredentialBinding)?;
    let funder = LegacyType1ProxyAddress::parse(&credential.funder)
        .map_err(|_| PredarbProductionOrderErrorV1::CredentialBinding)?;
    if signer.address().as_core() != expected_signer
        || EvmAddress::from_bytes(funder.bytes())
            .map_err(|_| PredarbProductionOrderErrorV1::CredentialBinding)?
            != expected_funder
        || !legacy_type1_proxy_address_matches(signer.address(), funder)
    {
        return Err(PredarbProductionOrderErrorV1::CredentialBinding);
    }
    let credentials = PmReadOnlyCredentialInput::new(
        std::mem::take(&mut *credential.api_key),
        std::mem::take(&mut *credential.api_secret),
        std::mem::take(&mut *credential.api_passphrase),
    );
    drop(credential);
    drop(signer);

    let owner = PmReadOnlyAccountConnectivityOwner::production(
        expected_signer,
        expected_funder,
        PmReadOnlySignatureType::Proxy,
        token_id,
        CONNECT_TIMEOUT,
        REQUEST_TIMEOUT,
        credentials,
    )
    .map_err(|_| PredarbProductionOrderErrorV1::PositionPreflight)?;
    let roles = owner
        .split()
        .map_err(|_| PredarbProductionOrderErrorV1::PositionPreflight)?;
    let server_time = roles.server_time;
    let mut account = roles.authenticated_account;
    let supervisor = roles.credential_supervisor;
    let observation = async {
        let timestamp = server_time.fresh_read_server_time().await?;
        account
            .account()
            .collateral_balance_allowance(timestamp)
            .await
    }
    .await;
    drop(account);
    let shutdown = supervisor.shutdown().await;
    let balance = observation
        .map_err(|_| PredarbProductionOrderErrorV1::PositionPreflight)?
        .into_value()
        .balance();
    shutdown.map_err(|_| PredarbProductionOrderErrorV1::PositionPreflight)?;
    Ok(balance)
}

async fn observe_predarb_exact_order(
    credential_env: &Path,
    exact_scope: PmWireScope,
    order_id: FixedOrderId,
) -> Result<PmExactOrderObservation, PredarbProductionOrderErrorV1> {
    let mut credential = load_predarb_credentials(credential_env)?;
    if credential.signature_type != "1" {
        return Err(PredarbProductionOrderErrorV1::CredentialProfile);
    }
    let signer = FixedEoaSigner::derive(EoaPrivateKeyInput::new(std::mem::take(
        &mut *credential.private_key,
    )))
    .map_err(|_| PredarbProductionOrderErrorV1::CredentialBinding)?;
    let signer_address = signer.address();
    let funder = LegacyType1ProxyAddress::parse(&credential.funder)
        .map_err(|_| PredarbProductionOrderErrorV1::CredentialBinding)?;
    if !legacy_type1_proxy_address_matches(signer_address, funder) {
        return Err(PredarbProductionOrderErrorV1::CredentialBinding);
    }
    let proxy_funder = EvmAddress::from_bytes(funder.bytes())
        .map_err(|_| PredarbProductionOrderErrorV1::CredentialBinding)?;
    let read_credentials = PmReadOnlyCredentialInput::new(
        std::mem::take(&mut *credential.api_key),
        std::mem::take(&mut *credential.api_secret),
        std::mem::take(&mut *credential.api_passphrase),
    );
    drop(credential);
    drop(signer);

    let user_ws_bounds = PmUserWsBounds::new(
        CONNECT_TIMEOUT,
        Duration::from_secs(30),
        Duration::from_secs(5),
        MAX_PM_LIVE_BODY_BYTES,
        1,
        Duration::from_millis(100),
        1,
        ConnectionEpoch::new(1),
    )
    .map_err(|_| PredarbProductionOrderErrorV1::Reconciliation)?;
    let owner = PmReadOnlyPrivateConnectivityOwner::production_proxy(
        signer_address.as_core(),
        proxy_funder,
        exact_scope,
        CONNECT_TIMEOUT,
        REQUEST_TIMEOUT,
        user_ws_bounds,
        read_credentials,
    )
    .map_err(|_| PredarbProductionOrderErrorV1::Reconciliation)?;
    let roles = owner
        .split()
        .map_err(|_| PredarbProductionOrderErrorV1::Reconciliation)?;
    let server_time = roles.server_time;
    let mut authenticated_http = roles.authenticated_http;
    let credential_supervisor = roles.credential_supervisor;
    drop(roles.geoblock);
    drop(roles.market_details);
    drop(roles.authenticated_user_ws);

    let observation = async {
        let timestamp = server_time.fresh_read_server_time().await?;
        authenticated_http
            .reconciliation()
            .exact_local_order_detail(timestamp, order_id)
            .await
    }
    .await;
    drop(authenticated_http);
    let shutdown = credential_supervisor.shutdown().await;
    let observation = observation.map_err(|_| PredarbProductionOrderErrorV1::Reconciliation)?;
    shutdown.map_err(|_| PredarbProductionOrderErrorV1::Reconciliation)?;
    Ok(observation)
}

async fn observe_predarb_owned_fills(
    credential_env: &Path,
    exact_scope: PmWireScope,
    order_id: FixedOrderId,
    expected_side: PmOrderSide,
    limit_price: PmPrice,
) -> Result<OwnedFillCutObservation, PredarbProductionOrderErrorV1> {
    let mut credential = load_predarb_credentials(credential_env)?;
    if credential.signature_type != "1" {
        return Err(PredarbProductionOrderErrorV1::CredentialProfile);
    }
    let signer = FixedEoaSigner::derive(EoaPrivateKeyInput::new(std::mem::take(
        &mut *credential.private_key,
    )))
    .map_err(|_| PredarbProductionOrderErrorV1::CredentialBinding)?;
    let signer_address = signer.address();
    let funder = LegacyType1ProxyAddress::parse(&credential.funder)
        .map_err(|_| PredarbProductionOrderErrorV1::CredentialBinding)?;
    if !legacy_type1_proxy_address_matches(signer_address, funder) {
        return Err(PredarbProductionOrderErrorV1::CredentialBinding);
    }
    let proxy_funder = EvmAddress::from_bytes(funder.bytes())
        .map_err(|_| PredarbProductionOrderErrorV1::CredentialBinding)?;
    let read_credentials = PmReadOnlyCredentialInput::new(
        std::mem::take(&mut *credential.api_key),
        std::mem::take(&mut *credential.api_secret),
        std::mem::take(&mut *credential.api_passphrase),
    );
    drop(credential);
    drop(signer);

    let user_ws_bounds = PmUserWsBounds::new(
        CONNECT_TIMEOUT,
        Duration::from_secs(30),
        Duration::from_secs(5),
        MAX_PM_LIVE_BODY_BYTES,
        1,
        Duration::from_millis(100),
        1,
        ConnectionEpoch::new(1),
    )
    .map_err(|_| PredarbProductionOrderErrorV1::Reconciliation)?;
    let owner = PmReadOnlyPrivateConnectivityOwner::production_proxy(
        signer_address.as_core(),
        proxy_funder,
        exact_scope,
        CONNECT_TIMEOUT,
        REQUEST_TIMEOUT,
        user_ws_bounds,
        read_credentials,
    )
    .map_err(|_| PredarbProductionOrderErrorV1::Reconciliation)?;
    let roles = owner
        .split()
        .map_err(|_| PredarbProductionOrderErrorV1::Reconciliation)?;
    let server_time = roles.server_time;
    let mut authenticated_http = roles.authenticated_http;
    let credential_supervisor = roles.credential_supervisor;
    drop(roles.geoblock);
    drop(roles.market_details);
    drop(roles.authenticated_user_ws);

    let observation = async {
        let timestamp = server_time
            .fresh_read_server_time()
            .await
            .map_err(|_| PredarbProductionOrderErrorV1::Reconciliation)?;
        let mut progress = authenticated_http
            .reconciliation()
            .begin_exact_scope_trades(timestamp)
            .await
            .map_err(PredarbProductionOrderErrorV1::FillObservation)?;
        let cut = loop {
            match progress {
                PmTradesCutProgress::Complete(cut) => break cut,
                PmTradesCutProgress::Incomplete(assembly) => {
                    let timestamp = server_time
                        .fresh_read_server_time()
                        .await
                        .map_err(|_| PredarbProductionOrderErrorV1::Reconciliation)?;
                    progress = authenticated_http
                        .reconciliation()
                        .continue_trades(timestamp, assembly)
                        .await
                        .map_err(PredarbProductionOrderErrorV1::FillObservation)?;
                }
            }
        };
        extract_exact_owned_fills(
            &cut,
            exact_scope,
            order_id,
            proxy_funder,
            expected_side,
            limit_price,
        )
    }
    .await;
    drop(authenticated_http);
    let shutdown = credential_supervisor.shutdown().await;
    let observation = observation?;
    shutdown.map_err(|_| PredarbProductionOrderErrorV1::Reconciliation)?;
    Ok(observation)
}

fn extract_exact_owned_fills(
    cut: &PmCompleteTradesCut,
    exact_scope: PmWireScope,
    order_id: FixedOrderId,
    expected_maker: EvmAddress,
    expected_side: PmOrderSide,
    limit_price: PmPrice,
) -> Result<OwnedFillCutObservation, PredarbProductionOrderErrorV1> {
    let mut fills = BTreeMap::<PmFillId, ObservedOwnedFill>::new();
    for trade in cut.pages().iter().flat_map(|page| page.trades()) {
        extract_exact_owned_trade(
            &mut fills,
            trade,
            exact_scope,
            order_id,
            expected_maker,
            expected_side,
            limit_price,
        )?;
    }
    Ok(OwnedFillCutObservation {
        pages: cut.pages().len(),
        account_rows: cut.row_count(),
        fills: fills.into_values().collect(),
    })
}

#[allow(clippy::too_many_arguments)]
fn extract_exact_owned_trade(
    fills: &mut BTreeMap<PmFillId, ObservedOwnedFill>,
    trade: &PmLiveTrade,
    exact_scope: PmWireScope,
    order_id: FixedOrderId,
    expected_maker: EvmAddress,
    expected_side: PmOrderSide,
    limit_price: PmPrice,
) -> Result<(), PredarbProductionOrderErrorV1> {
    let order_id = order_id.to_string();
    let exact_id = |candidate: PmVenueOrderId| candidate.as_str() == order_id;
    match trade.trader_side() {
        Some("TAKER") => {
            let direct = match (trade.order_id(), trade.taker_order_id()) {
                (Some(order), None) | (None, Some(order)) => Some(order),
                (None, None) => None,
                (Some(_), Some(_)) => {
                    return Err(PredarbProductionOrderErrorV1::FillReconciliation);
                }
            };
            if direct.is_some_and(exact_id) {
                validate_owned_fill_leg(
                    trade.condition(),
                    trade.token(),
                    trade.side(),
                    trade.price(),
                    exact_scope,
                    expected_side,
                    limit_price,
                )?;
                retain_owned_fill(
                    fills,
                    ObservedOwnedFill {
                        id: trade.id(),
                        quantity: trade.size(),
                        price: trade.price(),
                        status: canonical_trade_status(trade.status())?.to_owned(),
                        role: "taker",
                    },
                )?;
            }
        }
        Some("MAKER") => {
            for maker in trade
                .maker_orders()
                .iter()
                .filter(|maker| exact_id(maker.order_id()))
            {
                if maker.maker() != expected_maker {
                    return Err(PredarbProductionOrderErrorV1::FillReconciliation);
                }
                validate_owned_fill_leg(
                    trade.condition(),
                    maker.token(),
                    maker.side(),
                    maker.price(),
                    exact_scope,
                    expected_side,
                    limit_price,
                )?;
                retain_owned_fill(
                    fills,
                    ObservedOwnedFill {
                        id: trade.id(),
                        quantity: maker.matched_amount(),
                        price: maker.price(),
                        status: canonical_trade_status(trade.status())?.to_owned(),
                        role: "maker",
                    },
                )?;
            }
        }
        _ => {
            let references_exact = trade.order_id().is_some_and(exact_id)
                || trade.taker_order_id().is_some_and(exact_id)
                || trade
                    .maker_orders()
                    .iter()
                    .any(|maker| exact_id(maker.order_id()));
            if references_exact {
                return Err(PredarbProductionOrderErrorV1::FillReconciliation);
            }
        }
    }
    Ok(())
}

fn validate_owned_fill_leg(
    condition: PmConditionId,
    token: PmTokenId,
    side: PmOrderSide,
    execution_price: PmPrice,
    exact_scope: PmWireScope,
    expected_side: PmOrderSide,
    limit_price: PmPrice,
) -> Result<(), PredarbProductionOrderErrorV1> {
    let price_inside_limit = match side {
        PmOrderSide::Buy => execution_price <= limit_price,
        PmOrderSide::Sell => execution_price >= limit_price,
    };
    if condition != exact_scope.condition()
        || token != exact_scope.token()
        || side != expected_side
        || !price_inside_limit
    {
        return Err(PredarbProductionOrderErrorV1::FillReconciliation);
    }
    Ok(())
}

fn retain_owned_fill(
    fills: &mut BTreeMap<PmFillId, ObservedOwnedFill>,
    fill: ObservedOwnedFill,
) -> Result<(), PredarbProductionOrderErrorV1> {
    if let Some(prior) = fills.get(&fill.id) {
        if prior.quantity != fill.quantity || prior.price != fill.price || prior.role != fill.role {
            return Err(PredarbProductionOrderErrorV1::FillReconciliation);
        }
        return Ok(());
    }
    fills.insert(fill.id, fill);
    Ok(())
}

fn canonical_trade_status(status: &str) -> Result<&str, PredarbProductionOrderErrorV1> {
    let status = status.strip_prefix("TRADE_STATUS_").unwrap_or(status);
    match status {
        "MATCHED_NOT_BROADCASTED" | "MATCHED" | "MINED" | "CONFIRMED" | "RETRYING" | "FAILED" => {
            Ok(status)
        }
        _ => Err(PredarbProductionOrderErrorV1::FillReconciliation),
    }
}

fn exact_order_progress(
    wire_status: &str,
    original: PmQuantity,
    cumulative: U256,
) -> Result<PmOrderProgress, PredarbProductionOrderErrorV1> {
    let original_units = original.protocol_units();
    let status = match wire_status {
        "LIVE" if cumulative.is_zero() => PmOrderStatus::Open,
        "LIVE" if cumulative < original_units => PmOrderStatus::PartiallyFilled,
        "MATCHED" if cumulative == original_units => PmOrderStatus::Filled,
        status if is_cancelled_status(status) && cumulative <= original_units => {
            PmOrderStatus::Cancelled
        }
        _ => return Err(PredarbProductionOrderErrorV1::OrderState),
    };
    PmOrderProgress::new(original, cumulative, status)
        .map_err(|_| PredarbProductionOrderErrorV1::OrderState)
}

fn canonical_state_report(
    order: reap_pm_state::PmOwnedOrderProjection,
) -> CanonicalOrderStateReport {
    let status = order.status();
    CanonicalOrderStateReport {
        client_order_id: order.client_order().id().to_string(),
        venue_order_id: order
            .venue_order()
            .map(|venue_order| venue_order.id().to_string()),
        status: canonical_order_status(status),
        submit_state: canonical_submit_state(order.submit()),
        cancel_state: canonical_cancel_state(order.cancel()),
        quantity: order.quantity().to_string(),
        open_quantity: protocol_quantity_string(order.remaining()),
        filled_quantity: protocol_quantity_string(order.cumulative_filled()),
        known_fill_quantity: protocol_quantity_string(order.known_fill_total()),
        is_live: matches!(
            status,
            Some(PmOrderStatus::Open | PmOrderStatus::PartiallyFilled)
        ),
        is_terminal: order.is_terminal(),
        reconciliation_required: order.reconciliation_required(),
    }
}

const fn canonical_order_status(status: Option<PmOrderStatus>) -> &'static str {
    match status {
        None | Some(PmOrderStatus::Pending) => "pending_new",
        Some(PmOrderStatus::Open) => "live",
        Some(PmOrderStatus::PartiallyFilled) => "partially_filled",
        Some(PmOrderStatus::Filled) => "filled",
        Some(PmOrderStatus::Cancelled) => "cancelled",
        Some(PmOrderStatus::Rejected) => "rejected",
        Some(PmOrderStatus::Expired) => "expired",
    }
}

const fn canonical_submit_state(state: PmOwnedSubmitState) -> &'static str {
    match state {
        PmOwnedSubmitState::Pending => "pending",
        PmOwnedSubmitState::Accepted => "accepted",
        PmOwnedSubmitState::Rejected => "rejected",
        PmOwnedSubmitState::Ambiguous => "ambiguous",
    }
}

const fn canonical_cancel_state(state: PmOwnedCancelState) -> &'static str {
    match state {
        PmOwnedCancelState::None => "none",
        PmOwnedCancelState::Pending => "pending",
        PmOwnedCancelState::Rejected => "rejected",
        PmOwnedCancelState::Accepted => "accepted",
        PmOwnedCancelState::Ambiguous => "ambiguous",
        PmOwnedCancelState::FilledRace => "filled_race",
    }
}

fn protocol_quantity_string(units: U256) -> String {
    book_quantity_string(PmBookQuantity::from_protocol_units(units))
}

const fn book_quantity_units(quantity: PmBookQuantity) -> U256 {
    match quantity {
        PmBookQuantity::Delete => U256::ZERO,
        PmBookQuantity::Quantity(quantity) => quantity.protocol_units(),
    }
}

fn is_cancelled_status(status: &str) -> bool {
    matches!(
        status,
        "CANCELED"
            | "CANCELLED"
            | "ORDER_STATUS_CANCELED"
            | "ORDER_STATUS_CANCELLED"
            | "ORDER_STATUS_CANCELED_MARKET_RESOLVED"
            | "ORDER_STATUS_CANCELLED_MARKET_RESOLVED"
    )
}

fn book_quantity_string(quantity: PmBookQuantity) -> String {
    match quantity {
        PmBookQuantity::Delete => "0".to_owned(),
        PmBookQuantity::Quantity(quantity) => quantity.to_string(),
    }
}

struct CapturedBook {
    received: PmRestResponseClock,
    snapshot: PmBookSnapshot,
}

struct StrictBookSink {
    config: PmBookParserConfig,
}

#[async_trait::async_trait]
impl PmRestBookSnapshotSink for StrictBookSink {
    type Output = CapturedBook;
    type Error = PmWireError;

    async fn deliver_native_rest_book(
        &mut self,
        _purpose: PmRestBookPurpose,
        received: PmRestResponseClock,
        raw: &[u8],
    ) -> Result<Self::Output, Self::Error> {
        let snapshot = parse_rest_book_snapshot(raw, self.config)?;
        Ok(CapturedBook { received, snapshot })
    }
}

async fn fetch_fresh_book(
    fixed_peer: &PmFixedTlsPeerSelection,
    local_egress: &PmLocalEgressSelection,
    market: &PmBtcFiveMinuteMarket,
    scope: PmWireScope,
) -> Result<CapturedBook, PredarbProductionOrderErrorV1> {
    let parser_config = PmBookParserConfig::new_condition_bound(
        scope,
        market.tick(),
        market.minimum_order_size(),
        market.negative_risk(),
    );
    let role = PmPublicHttpRole::production_on_fixed_tls_peer_and_selected_local_egress(
        CONNECT_TIMEOUT,
        REQUEST_TIMEOUT,
        fixed_peer.clone(),
        local_egress.clone(),
        parser_config,
    )
    .map_err(|_| PredarbProductionOrderErrorV1::BookPreflight)?;
    let mut sink = StrictBookSink {
        config: parser_config,
    };
    let captured = role
        .seed_book(&mut sink)
        .await
        .map_err(|error| match error {
            PmRestBookDeliveryError::Http(_) => PredarbProductionOrderErrorV1::BookTransport,
            PmRestBookDeliveryError::Sink(_) => PredarbProductionOrderErrorV1::BookParse,
        })?;
    validate_book_freshness(&captured, market)?;
    Ok(captured)
}

fn validate_book_freshness(
    captured: &CapturedBook,
    market: &PmBtcFiveMinuteMarket,
) -> Result<(), PredarbProductionOrderErrorV1> {
    let received_millis = captured.received.local_wall_receive_ns() / 1_000_000;
    let book_millis = captured.snapshot.timestamp_millis();
    if book_millis > received_millis.saturating_add(MAX_BOOK_FUTURE_LEAD_MILLIS)
        || received_millis.saturating_sub(book_millis) > MAX_BOOK_AGE_MILLIS
    {
        return Err(PredarbProductionOrderErrorV1::BookStale);
    }
    ensure_window_remaining(market)
}

fn ensure_window_remaining(
    market: &PmBtcFiveMinuteMarket,
) -> Result<(), PredarbProductionOrderErrorV1> {
    let (_, now_seconds) = current_timestamps()?;
    if market.window_end_epoch().saturating_sub(now_seconds) < MINIMUM_WINDOW_REMAINING_SECONDS {
        return Err(PredarbProductionOrderErrorV1::MarketWindowTooClose);
    }
    Ok(())
}

fn best_bid(snapshot: &PmBookSnapshot) -> Result<PmPrice, PredarbProductionOrderErrorV1> {
    snapshot
        .bids()
        .last()
        .map(|level| level.level().price())
        .ok_or(PredarbProductionOrderErrorV1::BookPreflight)
}

fn best_ask(snapshot: &PmBookSnapshot) -> Result<PmPrice, PredarbProductionOrderErrorV1> {
    snapshot
        .asks()
        .first()
        .map(|level| level.level().price())
        .ok_or(PredarbProductionOrderErrorV1::BookPreflight)
}

fn best_ask_quantity(
    snapshot: &PmBookSnapshot,
) -> Result<PmQuantity, PredarbProductionOrderErrorV1> {
    match snapshot
        .asks()
        .first()
        .map(|level| level.level().quantity())
    {
        Some(PmBookQuantity::Quantity(quantity)) => Ok(quantity),
        Some(PmBookQuantity::Delete) | None => Err(PredarbProductionOrderErrorV1::BookPreflight),
    }
}

fn configured_position_units(
    observation: &PmProductionDataApiPositionObservation,
) -> Result<U256, PredarbProductionOrderErrorV1> {
    match observation.configured_token() {
        PmConfiguredTokenPosition::Absent => Ok(U256::ZERO),
        PmConfiguredTokenPosition::Present(position) => position
            .size_protocol_units()
            .map_err(|_| PredarbProductionOrderErrorV1::PositionArithmetic),
    }
}

fn position_report(
    observation: &PmProductionDataApiPositionObservation,
    protocol_units: U256,
) -> PositionReport {
    let (classification, size) = match observation.configured_token() {
        PmConfiguredTokenPosition::Absent => ("absent", None),
        PmConfiguredTokenPosition::Present(position) => {
            ("present", Some(position.size().lexeme().to_owned()))
        }
    };
    PositionReport {
        classification,
        size,
        protocol_units: protocol_quantity_string(protocol_units),
        observed_at_millis: observation.completed_clock().unix_milliseconds(),
        commitment: observation.commitment().to_string(),
    }
}

fn fill_position_reconciliation_report(
    attempt: u8,
    fill_cut: &OwnedFillCutObservation,
    projection: &ProductionOwnedOrderProjection,
    position_before: U256,
    venue_position: U256,
) -> Result<FillPositionReconciliationReport, PredarbProductionOrderErrorV1> {
    let fill_cut_total = fill_cut.fills.iter().try_fold(U256::ZERO, |total, fill| {
        total
            .checked_add(fill.quantity.protocol_units())
            .map_err(|_| PredarbProductionOrderErrorV1::PositionArithmetic)
    })?;
    let fill_based_position = match projection.side {
        PmOrderSide::Buy => position_before
            .checked_add(projection.known_fill_total)
            .map_err(|_| PredarbProductionOrderErrorV1::PositionArithmetic)?,
        PmOrderSide::Sell => position_before
            .checked_sub(projection.known_fill_total)
            .map_err(|_| PredarbProductionOrderErrorV1::PositionArithmetic)?,
    };
    let fills_match_order_cumulative = fill_cut_total == projection.known_fill_total
        && projection.known_fill_total == projection.cumulative_filled;
    let fill_ledger_reconciled =
        fills_match_order_cumulative && !projection.report.reconciliation_required;
    let venue_position_matches_fill_based = venue_position == fill_based_position;
    Ok(FillPositionReconciliationReport {
        attempt,
        trade_pages: fill_cut.pages,
        account_trade_rows: fill_cut.account_rows,
        owned_fill_count: fill_cut.fills.len(),
        fill_quantity: protocol_quantity_string(fill_cut_total),
        fill_delta: signed_fill_delta(projection.side, projection.known_fill_total),
        position_before: protocol_quantity_string(position_before),
        fill_based_position: protocol_quantity_string(fill_based_position),
        venue_position: protocol_quantity_string(venue_position),
        order_cumulative_filled: protocol_quantity_string(projection.cumulative_filled),
        known_fill_quantity: protocol_quantity_string(projection.known_fill_total),
        fills_match_order_cumulative,
        fill_ledger_reconciled,
        venue_position_matches_fill_based,
        authoritative_minus_fill_based_position: signed_position_difference(
            venue_position,
            fill_based_position,
        )?,
        converged: fill_ledger_reconciled && venue_position_matches_fill_based,
    })
}

fn signed_position_difference(
    authoritative: U256,
    fill_based: U256,
) -> Result<String, PredarbProductionOrderErrorV1> {
    if authoritative == fill_based {
        return Ok("0".to_owned());
    }
    if authoritative > fill_based {
        let difference = authoritative
            .checked_sub(fill_based)
            .map_err(|_| PredarbProductionOrderErrorV1::PositionArithmetic)?;
        return Ok(format!("+{}", protocol_quantity_string(difference)));
    }
    let difference = fill_based
        .checked_sub(authoritative)
        .map_err(|_| PredarbProductionOrderErrorV1::PositionArithmetic)?;
    Ok(format!("-{}", protocol_quantity_string(difference)))
}

fn signed_fill_delta(side: PmOrderSide, units: U256) -> String {
    if units.is_zero() {
        return "0".to_owned();
    }
    let quantity = protocol_quantity_string(units);
    match side {
        PmOrderSide::Buy => format!("+{quantity}"),
        PmOrderSide::Sell => format!("-{quantity}"),
    }
}

fn production_connectivity(
    fixed_peer_ip: &str,
    interface_name: &str,
    local_source_ip: IpAddr,
) -> Result<(PmFixedTlsPeerSelection, PmLocalEgressSelection), PredarbProductionOrderErrorV1> {
    let peer = PmFixedTlsPeerSelection::production("clob.polymarket.com", fixed_peer_ip)
        .map_err(|_| PredarbProductionOrderErrorV1::TransportProfile)?;
    let local = PmLocalEgressSelection::production(interface_name, local_source_ip)
        .map_err(|_| PredarbProductionOrderErrorV1::TransportProfile)?;
    Ok((peer, local))
}

const fn canonical_side(side: PmOrderSide) -> &'static str {
    match side {
        PmOrderSide::Buy => "BUY",
        PmOrderSide::Sell => "SELL",
    }
}

const fn classification_name(classification: PmMutationClassification) -> &'static str {
    match classification {
        PmMutationClassification::DefinitelyNotDispatched => "definitely_not_dispatched",
        PmMutationClassification::Accepted => "accepted",
        PmMutationClassification::Rejected => "rejected",
        PmMutationClassification::OutOfProfile => "out_of_profile",
        PmMutationClassification::AcknowledgementUnknown => "acknowledgement_unknown",
    }
}

const fn diagnostic_name(diagnostic: PmMutationDiagnosticKind) -> &'static str {
    match diagnostic {
        PmMutationDiagnosticKind::PreSendValidation => "pre_send_validation",
        PmMutationDiagnosticKind::AcceptedProfile => "accepted_profile",
        PmMutationDiagnosticKind::VenueRejected => "venue_rejected",
        PmMutationDiagnosticKind::ResponseIdentityMismatch => "response_identity_mismatch",
        PmMutationDiagnosticKind::ResponseProfileMismatch => "response_profile_mismatch",
        PmMutationDiagnosticKind::Redirect => "redirect",
        PmMutationDiagnosticKind::AuthenticationInvalid => "authentication_invalid",
        PmMutationDiagnosticKind::ReconciliationRequiredStatus => "reconciliation_required_status",
        PmMutationDiagnosticKind::UnexpectedHttpStatus => "unexpected_http_status",
        PmMutationDiagnosticKind::MalformedResponse => "malformed_response",
        PmMutationDiagnosticKind::ResponseTooLarge => "response_too_large",
        PmMutationDiagnosticKind::RequestTimeout => "request_timeout",
        PmMutationDiagnosticKind::TransportFailure => "transport_failure",
        PmMutationDiagnosticKind::ResponseBodyTimeout => "response_body_timeout",
        PmMutationDiagnosticKind::ResponseBodyFailure => "response_body_failure",
        PmMutationDiagnosticKind::ConnectedPeerMismatch => "connected_peer_mismatch",
    }
}

fn current_timestamps() -> Result<(u64, u64), PredarbProductionOrderErrorV1> {
    let duration = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map_err(|_| PredarbProductionOrderErrorV1::Clock)?;
    let milliseconds =
        u64::try_from(duration.as_millis()).map_err(|_| PredarbProductionOrderErrorV1::Clock)?;
    Ok((milliseconds, duration.as_secs()))
}

fn current_l2_timestamp() -> Result<L2Timestamp, PredarbProductionOrderErrorV1> {
    let (_, seconds) = current_timestamps()?;
    L2Timestamp::from_unix_seconds(seconds).map_err(|_| PredarbProductionOrderErrorV1::Clock)
}

fn load_predarb_credentials(
    path: &Path,
) -> Result<PredarbCredentialBundle, PredarbProductionOrderErrorV1> {
    let before = fs::symlink_metadata(path)
        .map_err(|_| PredarbProductionOrderErrorV1::CredentialFileProtection)?;
    validate_secret_metadata(&before)?;
    let file = OpenOptions::new()
        .read(true)
        .custom_flags(libc::O_CLOEXEC | libc::O_NOFOLLOW)
        .open(path)
        .map_err(|_| PredarbProductionOrderErrorV1::CredentialFileRead)?;
    let opened = file
        .metadata()
        .map_err(|_| PredarbProductionOrderErrorV1::CredentialFileRead)?;
    validate_secret_metadata(&opened)?;
    if before.dev() != opened.dev() || before.ino() != opened.ino() {
        return Err(PredarbProductionOrderErrorV1::CredentialFileProtection);
    }
    let mut contents = Zeroizing::new(String::new());
    file.take(MAX_CREDENTIAL_FILE_BYTES + 1)
        .read_to_string(&mut contents)
        .map_err(|_| PredarbProductionOrderErrorV1::CredentialFileRead)?;
    if contents.len() as u64 > MAX_CREDENTIAL_FILE_BYTES {
        return Err(PredarbProductionOrderErrorV1::CredentialFileRead);
    }
    parse_predarb_environment(contents.as_str())
}

fn validate_secret_metadata(metadata: &fs::Metadata) -> Result<(), PredarbProductionOrderErrorV1> {
    if !metadata.file_type().is_file()
        || metadata.uid() != rustix::process::geteuid().as_raw()
        || metadata.mode() & 0o777 != 0o600
        || metadata.nlink() != 1
        || metadata.len() > MAX_CREDENTIAL_FILE_BYTES
    {
        return Err(PredarbProductionOrderErrorV1::CredentialFileProtection);
    }
    Ok(())
}

fn parse_predarb_environment(
    contents: &str,
) -> Result<PredarbCredentialBundle, PredarbProductionOrderErrorV1> {
    let mut private_key = None;
    let mut funder = None;
    let mut signature_type = None;
    let mut api_key = None;
    let mut api_secret = None;
    let mut api_passphrase = None;
    for raw_line in contents.lines() {
        let line = raw_line.trim();
        if line.is_empty() || line.starts_with('#') {
            continue;
        }
        let line = line.strip_prefix("export ").unwrap_or(line);
        let Some((name, value)) = line.split_once('=') else {
            continue;
        };
        let target = match name.trim() {
            "POLYMARKET_PRIVATE_KEY" => &mut private_key,
            "POLYMARKET_FUNDER" => &mut funder,
            "POLYMARKET_SIGNATURE_TYPE" => &mut signature_type,
            "POLYMARKET_API_KEY" => &mut api_key,
            "POLYMARKET_API_SECRET" => &mut api_secret,
            "POLYMARKET_API_PASSPHRASE" => &mut api_passphrase,
            _ => continue,
        };
        if target.is_some() {
            return Err(PredarbProductionOrderErrorV1::CredentialEnvironment);
        }
        *target = Some(Zeroizing::new(parse_env_value(value)?));
    }
    let mut funder = funder.ok_or(PredarbProductionOrderErrorV1::CredentialEnvironment)?;
    let mut signature_type =
        signature_type.ok_or(PredarbProductionOrderErrorV1::CredentialEnvironment)?;
    Ok(PredarbCredentialBundle {
        private_key: private_key.ok_or(PredarbProductionOrderErrorV1::CredentialEnvironment)?,
        funder: std::mem::take(&mut *funder),
        signature_type: std::mem::take(&mut *signature_type),
        api_key: api_key.ok_or(PredarbProductionOrderErrorV1::CredentialEnvironment)?,
        api_secret: api_secret.ok_or(PredarbProductionOrderErrorV1::CredentialEnvironment)?,
        api_passphrase: api_passphrase
            .ok_or(PredarbProductionOrderErrorV1::CredentialEnvironment)?,
    })
}

fn parse_env_value(value: &str) -> Result<String, PredarbProductionOrderErrorV1> {
    let value = value.trim();
    if value.is_empty() || value.as_bytes().contains(&0) {
        return Err(PredarbProductionOrderErrorV1::CredentialEnvironment);
    }
    if let Some(inner) = value.strip_prefix('"').and_then(|v| v.strip_suffix('"')) {
        if inner.contains(['"', '\\', '\n', '\r']) || inner.is_empty() {
            return Err(PredarbProductionOrderErrorV1::CredentialEnvironment);
        }
        return Ok(inner.to_owned());
    }
    if let Some(inner) = value.strip_prefix('\'').and_then(|v| v.strip_suffix('\'')) {
        if inner.contains('\'') || inner.is_empty() {
            return Err(PredarbProductionOrderErrorV1::CredentialEnvironment);
        }
        return Ok(inner.to_owned());
    }
    if value.bytes().any(|byte| byte.is_ascii_whitespace()) {
        return Err(PredarbProductionOrderErrorV1::CredentialEnvironment);
    }
    Ok(value.to_owned())
}

impl OneShotLedger {
    fn create(directory: &Path) -> Result<Self, PredarbProductionOrderErrorV1> {
        if !directory.exists() {
            DirBuilder::new()
                .mode(0o700)
                .create(directory)
                .map_err(|_| PredarbProductionOrderErrorV1::Ledger)?;
            sync_parent(directory)?;
        }
        let metadata = fs::symlink_metadata(directory)
            .map_err(|_| PredarbProductionOrderErrorV1::StateDirectoryProtection)?;
        if !metadata.file_type().is_dir()
            || metadata.uid() != rustix::process::geteuid().as_raw()
            || metadata.mode() & 0o777 != 0o700
        {
            return Err(PredarbProductionOrderErrorV1::StateDirectoryProtection);
        }
        let path = directory.join(LEDGER_FILE);
        let file = OpenOptions::new()
            .create_new(true)
            .write(true)
            .custom_flags(libc::O_CLOEXEC | libc::O_NOFOLLOW)
            .mode(0o600)
            .open(&path)
            .map_err(|error| {
                if error.kind() == std::io::ErrorKind::AlreadyExists {
                    PredarbProductionOrderErrorV1::AttemptAlreadyExists
                } else {
                    PredarbProductionOrderErrorV1::Ledger
                }
            })?;
        file.sync_all()
            .map_err(|_| PredarbProductionOrderErrorV1::Ledger)?;
        File::open(directory)
            .and_then(|parent| parent.sync_all())
            .map_err(|_| PredarbProductionOrderErrorV1::Ledger)?;
        Ok(Self { path, file })
    }

    fn append(&mut self, value: &serde_json::Value) -> Result<(), PredarbProductionOrderErrorV1> {
        serde_json::to_writer(&mut self.file, value)
            .map_err(|_| PredarbProductionOrderErrorV1::Ledger)?;
        self.file
            .write_all(b"\n")
            .and_then(|()| self.file.flush())
            .and_then(|()| self.file.sync_all())
            .map_err(|_| PredarbProductionOrderErrorV1::Ledger)
    }
}

fn sync_parent(path: &Path) -> Result<(), PredarbProductionOrderErrorV1> {
    let parent = path.parent().ok_or(PredarbProductionOrderErrorV1::Ledger)?;
    File::open(parent)
        .and_then(|directory| directory.sync_all())
        .map_err(|_| PredarbProductionOrderErrorV1::Ledger)
}

#[cfg(test)]
mod tests {
    use super::*;

    const KEY: &str = "0x59c6995e998f97a5a0044966f09453885a7f2f2e8f47b57e6f77f1bff7b6f6a3";
    const ORDER_ID: &str = "0x1111111111111111111111111111111111111111111111111111111111111111";
    const CONDITION_ID: &str = "0x2222222222222222222222222222222222222222222222222222222222222222";

    fn production_order_state() -> ProductionOwnedOrderState {
        ProductionOwnedOrderState::pending(
            EvmAddress::from_bytes([1; 20]).unwrap(),
            EvmAddress::from_bytes([2; 20]).unwrap(),
            FixedOrderId::parse(ORDER_ID).unwrap(),
            PmOrderSide::Buy,
            PmPrice::parse_decimal("0.01").unwrap(),
            PmQuantity::parse_decimal("5").unwrap(),
        )
        .unwrap()
    }

    fn exact_order(status: &str, matched: &str) -> PmExactOrderObservation {
        let body = format!(
            r#"{{"id":"{ORDER_ID}","market":"{CONDITION_ID}","asset_id":"1","side":"BUY","original_size":"5","size_matched":"{matched}","price":"0.01","status":"{status}","maker_address":"0x0202020202020202020202020202020202020202","owner":"00000000-0000-0000-0000-000000000000","created_at":1700000000,"expiration":"0","outcome":"Up","order_type":"GTC"}}"#,
        );
        PmExactOrderObservation::Present(Box::new(
            reap_polymarket_wire::parse_live_order_detail(body.as_bytes()).unwrap(),
        ))
    }

    fn owned_fill(id: &str, quantity: &str) -> ObservedOwnedFill {
        ObservedOwnedFill {
            id: PmFillId::new(id).unwrap(),
            quantity: PmQuantity::parse_decimal(quantity).unwrap(),
            price: PmPrice::parse_decimal("0.01").unwrap(),
            status: "CONFIRMED".to_owned(),
            role: "maker",
        }
    }

    #[test]
    fn parser_extracts_only_the_six_required_predarb_values() {
        let source = format!(
            "IGNORED=value\nPOLYMARKET_PRIVATE_KEY={KEY}\nPOLYMARKET_FUNDER=0x7754536ecd85c00b2E0CF9c1aA679340D8550756\nPOLYMARKET_SIGNATURE_TYPE=1\nPOLYMARKET_API_KEY=00000000-0000-0000-0000-000000000000\nPOLYMARKET_API_SECRET=YWJj\nPOLYMARKET_API_PASSPHRASE=pass\n"
        );
        let bundle = parse_predarb_environment(&source).unwrap();
        assert_eq!(bundle.signature_type, "1");
        assert_eq!(bundle.funder, "0x7754536ecd85c00b2E0CF9c1aA679340D8550756");
        assert_eq!(bundle.private_key.len(), KEY.len());
    }

    #[test]
    fn duplicate_required_variable_is_rejected() {
        let source = format!(
            "POLYMARKET_PRIVATE_KEY={KEY}\nPOLYMARKET_PRIVATE_KEY={KEY}\nPOLYMARKET_FUNDER=x\nPOLYMARKET_SIGNATURE_TYPE=1\nPOLYMARKET_API_KEY=x\nPOLYMARKET_API_SECRET=x\nPOLYMARKET_API_PASSPHRASE=x\n"
        );
        assert!(matches!(
            parse_predarb_environment(&source),
            Err(PredarbProductionOrderErrorV1::CredentialEnvironment)
        ));
    }

    #[test]
    fn one_shot_ledger_refuses_a_second_attempt() {
        let temporary = tempfile::tempdir().unwrap();
        let state = temporary.path().join("state");
        let mut first = OneShotLedger::create(&state).unwrap();
        first.append(&json!({"event": "authorized"})).unwrap();
        drop(first);
        assert!(matches!(
            OneShotLedger::create(&state),
            Err(PredarbProductionOrderErrorV1::AttemptAlreadyExists)
        ));
    }

    #[test]
    fn hard_cap_and_authorization_phrase_are_closed_constants() {
        assert_eq!(MAX_TEST_QUANTITY, "5");
        assert_eq!(
            AUTHORIZATION_PHRASE,
            "I_ACCEPT_TOTAL_LOSS_AND_ONE_REAL_POLYMARKET_ORDER"
        );
    }

    #[test]
    fn exact_reconciliation_accepts_only_reviewed_cancelled_statuses() {
        assert!(is_cancelled_status("CANCELED"));
        assert!(is_cancelled_status("ORDER_STATUS_CANCELLED"));
        assert!(!is_cancelled_status("LIVE"));
        assert!(!is_cancelled_status("MATCHED"));
    }

    #[test]
    fn production_state_matches_pending_ambiguous_cancel_and_reconcile_lifecycle() {
        let mut state = production_order_state();
        assert_eq!(state.report().unwrap().status, "pending_new");
        assert_eq!(state.report().unwrap().submit_state, "pending");
        assert!(state.report().unwrap().venue_order_id.is_none());

        state
            .apply_place(PmMutationClassification::AcknowledgementUnknown)
            .unwrap();
        let ambiguous = state.report().unwrap();
        assert_eq!(ambiguous.status, "pending_new");
        assert_eq!(ambiguous.submit_state, "ambiguous");
        assert!(ambiguous.venue_order_id.is_some());
        assert!(ambiguous.reconciliation_required);

        let cancel = state.request_cancel().unwrap();
        assert_eq!(state.report().unwrap().cancel_state, "pending");
        state
            .apply_cancel(cancel, PmMutationClassification::Accepted)
            .unwrap();
        let cancelled = state.report().unwrap();
        assert_eq!(cancelled.status, "cancelled");
        assert_eq!(cancelled.submit_state, "accepted");
        assert_eq!(cancelled.cancel_state, "accepted");
        assert!(cancelled.is_terminal);
        assert!(cancelled.reconciliation_required);

        let reconciled = state
            .reconcile_exact(&exact_order("CANCELED", "0"))
            .unwrap();
        assert!(reconciled.state_applied);
        let terminal = state.report().unwrap();
        assert_eq!(terminal.status, "cancelled");
        assert_eq!(terminal.open_quantity, "5");
        assert_eq!(terminal.filled_quantity, "0");
        assert!(!terminal.is_live);
        assert!(terminal.is_terminal);
        assert!(!terminal.reconciliation_required);
    }

    #[test]
    fn exact_partial_cancel_keeps_reconciliation_until_fill_legs_arrive() {
        let mut state = production_order_state();
        state
            .apply_place(PmMutationClassification::Accepted)
            .unwrap();
        let cancel = state.request_cancel().unwrap();
        state
            .apply_cancel(cancel, PmMutationClassification::Accepted)
            .unwrap();
        state
            .reconcile_exact(&exact_order("CANCELED", "1"))
            .unwrap();

        let report = state.report().unwrap();
        assert_eq!(report.status, "cancelled");
        assert_eq!(report.open_quantity, "4");
        assert_eq!(report.filled_quantity, "1");
        assert!(report.reconciliation_required);
    }

    #[test]
    fn exact_fill_ids_drive_position_and_clear_cumulative_reconciliation_once() {
        let mut state = production_order_state();
        state
            .apply_place(PmMutationClassification::Accepted)
            .unwrap();
        let cancel = state.request_cancel().unwrap();
        state
            .apply_cancel(cancel, PmMutationClassification::Accepted)
            .unwrap();
        state
            .reconcile_exact(&exact_order("CANCELED", "1"))
            .unwrap();
        let fill = owned_fill("fill-1", "1");
        state
            .apply_polled_fills(std::slice::from_ref(&fill))
            .unwrap();
        state
            .apply_polled_fills(std::slice::from_ref(&fill))
            .unwrap();

        let projection = state.projection().unwrap();
        assert_eq!(projection.known_fill_total, fill.quantity.protocol_units());
        assert_eq!(projection.cumulative_filled, fill.quantity.protocol_units());
        assert!(!projection.report.reconciliation_required);

        let cut = OwnedFillCutObservation {
            pages: 1,
            account_rows: 4,
            fills: vec![fill],
        };
        let before = PmQuantity::parse_decimal("2").unwrap().protocol_units();
        let venue = PmQuantity::parse_decimal("3").unwrap().protocol_units();
        let converged =
            fill_position_reconciliation_report(1, &cut, &projection, before, venue).unwrap();
        assert_eq!(converged.fill_delta, "+1");
        assert_eq!(converged.fill_based_position, "3");
        assert!(converged.fills_match_order_cumulative);
        assert!(converged.fill_ledger_reconciled);
        assert!(converged.venue_position_matches_fill_based);
        assert_eq!(converged.authoritative_minus_fill_based_position, "0");
        assert!(converged.converged);

        let lagging =
            fill_position_reconciliation_report(2, &cut, &projection, before, before).unwrap();
        assert!(lagging.fill_ledger_reconciled);
        assert!(!lagging.venue_position_matches_fill_based);
        assert_eq!(lagging.authoritative_minus_fill_based_position, "-1");
        assert!(!lagging.converged);
    }

    #[test]
    fn authenticated_trade_shape_extracts_only_the_exact_owned_order() {
        let body = format!(
            r#"{{"data":[{{"id":"fill-1","market":"{CONDITION_ID}","asset_id":"1","side":"BUY","size":"1","price":"0.01","status":"TRADE_STATUS_CONFIRMED","match_time":"1700000000","last_update":"1700000001","taker_order_id":"{ORDER_ID}","trader_side":"TAKER","maker_orders":[],"maker_address":"0x0303030303030303030303030303030303030303","owner":"00000000-0000-0000-0000-000000000000"}},{{"id":"foreign-fill","market":"{CONDITION_ID}","asset_id":"1","side":"BUY","size":"4","price":"0.01","status":"CONFIRMED","match_time":"1700000000","last_update":"1700000001","taker_order_id":"0xaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa","trader_side":"TAKER","maker_orders":[],"maker_address":"0x0303030303030303030303030303030303030303","owner":"00000000-0000-0000-0000-000000000000"}}],"next_cursor":"LTE=","limit":128,"count":2}}"#,
        );
        let page = reap_polymarket_wire::parse_live_trade_page(body.as_bytes()).unwrap();
        let exact_scope = PmWireScope::new(
            PmConditionId::parse(CONDITION_ID).unwrap(),
            PmMarketId::parse("0x3333333333333333333333333333333333333333333333333333333333333333")
                .unwrap(),
            PmTokenId::new(U256::ONE).unwrap(),
        );
        let mut fills = BTreeMap::new();
        for trade in page.trades() {
            extract_exact_owned_trade(
                &mut fills,
                trade,
                exact_scope,
                FixedOrderId::parse(ORDER_ID).unwrap(),
                EvmAddress::from_bytes([2; 20]).unwrap(),
                PmOrderSide::Buy,
                PmPrice::parse_decimal("0.01").unwrap(),
            )
            .unwrap();
        }
        assert_eq!(fills.len(), 1);
        let fill = fills.values().next().unwrap();
        assert_eq!(fill.id.as_str(), "fill-1");
        assert_eq!(fill.quantity.to_string(), "1");
        assert_eq!(fill.price.to_string(), "0.01");
        assert_eq!(fill.status, "CONFIRMED");
        assert_eq!(fill.role, "taker");
    }

    #[test]
    fn rejected_submit_is_terminal_without_cancel() {
        let mut state = production_order_state();
        state
            .apply_place(PmMutationClassification::Rejected)
            .unwrap();
        let report = state.report().unwrap();
        assert_eq!(report.status, "rejected");
        assert_eq!(report.submit_state, "rejected");
        assert!(report.is_terminal);
        assert!(!report.reconciliation_required);
    }
}
