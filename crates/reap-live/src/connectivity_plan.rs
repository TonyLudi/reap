use std::collections::{BTreeMap, BTreeSet};

use reap_core::PINNED_JAVA_REVISION;
use reap_order::okx_order_dispatch_key;
use reap_strategy::{
    ChaosDecisionInput, ChaosDecisionRequirementId, ChaosDecisionRequirements, ReferenceDataKind,
};
use reap_venue::okx::okx_capability_registration;
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use thiserror::Error;

use crate::{LiveConfig, LiveConfigError, LiveMode, TradingEnvironment};

pub const CHAOS_CONNECTIVITY_PLAN_SCHEMA_VERSION: u32 = 1;
pub const FORBIDDEN_PROOF_DEFAULT_MAX_AGE_MS: u64 = 30_000;
pub const FORBIDDEN_PROOF_HARD_MAX_AGE_MS: u64 = 60_000;
pub const FORBIDDEN_PROOF_DEFAULT_SCAN_INTERVAL_MS: u64 = 15_000;

#[derive(Debug, Error)]
pub enum ChaosConnectivityPlanError {
    #[error(transparent)]
    Config(#[from] LiveConfigError),
    #[error("configured symbol {symbol} has no live account owner")]
    MissingAccountOwner { symbol: String },
    #[error("production order entry is unavailable")]
    ProductionOrderEntryUnavailable,
}

/// Stable requirement identifiers admitted to the in-process Chaos live plan.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize)]
pub enum ConnectivityRequirementId {
    #[serde(rename = "CHAOS-EXEC-CANCEL-OWNED")]
    ChaosExecCancelOwned,
    #[serde(rename = "CHAOS-EXEC-HEDGE")]
    ChaosExecHedge,
    #[serde(rename = "CHAOS-EXEC-QUOTE")]
    ChaosExecQuote,
    #[serde(rename = "CHAOS-MD-BOOK")]
    ChaosMdBook,
    #[serde(rename = "CHAOS-MD-TRADE")]
    ChaosMdTrade,
    #[serde(rename = "CHAOS-REF-FUNDING")]
    ChaosRefFunding,
    #[serde(rename = "CHAOS-REF-INDEX")]
    ChaosRefIndex,
    #[serde(rename = "CHAOS-REF-LIMITS")]
    ChaosRefLimits,
    #[serde(rename = "CHAOS-REF-MARK")]
    ChaosRefMark,
    #[serde(rename = "CHAOS-STATE-ACCOUNT")]
    ChaosStateAccount,
    #[serde(rename = "CHAOS-STATE-ORDERS")]
    ChaosStateOrders,
    #[serde(rename = "CHAOS-STATE-POSITIONS")]
    ChaosStatePositions,
    #[serde(rename = "CHAOS-TIMER")]
    ChaosTimer,
    #[serde(rename = "SAFE-ACCOUNT-POSITIONS")]
    SafeAccountPositions,
    #[serde(rename = "SAFE-CLOCK-STATUS")]
    SafeClockStatus,
    #[serde(rename = "SAFE-FORBIDDEN-ZERO")]
    SafeForbiddenZero,
    #[serde(rename = "SAFE-METADATA")]
    SafeMetadata,
    #[serde(rename = "SAFE-RECONCILE")]
    SafeReconcile,
    #[serde(rename = "SAFE-REGULAR-CAA")]
    SafeRegularCaa,
    #[serde(rename = "SAFE-REGULAR-CANCEL")]
    SafeRegularCancel,
    #[serde(rename = "SAFE-STABLECOIN")]
    SafeStablecoin,
}

impl ConnectivityRequirementId {
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::ChaosExecCancelOwned => "CHAOS-EXEC-CANCEL-OWNED",
            Self::ChaosExecHedge => "CHAOS-EXEC-HEDGE",
            Self::ChaosExecQuote => "CHAOS-EXEC-QUOTE",
            Self::ChaosMdBook => "CHAOS-MD-BOOK",
            Self::ChaosMdTrade => "CHAOS-MD-TRADE",
            Self::ChaosRefFunding => "CHAOS-REF-FUNDING",
            Self::ChaosRefIndex => "CHAOS-REF-INDEX",
            Self::ChaosRefLimits => "CHAOS-REF-LIMITS",
            Self::ChaosRefMark => "CHAOS-REF-MARK",
            Self::ChaosStateAccount => "CHAOS-STATE-ACCOUNT",
            Self::ChaosStateOrders => "CHAOS-STATE-ORDERS",
            Self::ChaosStatePositions => "CHAOS-STATE-POSITIONS",
            Self::ChaosTimer => "CHAOS-TIMER",
            Self::SafeAccountPositions => "SAFE-ACCOUNT-POSITIONS",
            Self::SafeClockStatus => "SAFE-CLOCK-STATUS",
            Self::SafeForbiddenZero => "SAFE-FORBIDDEN-ZERO",
            Self::SafeMetadata => "SAFE-METADATA",
            Self::SafeReconcile => "SAFE-RECONCILE",
            Self::SafeRegularCaa => "SAFE-REGULAR-CAA",
            Self::SafeRegularCancel => "SAFE-REGULAR-CANCEL",
            Self::SafeStablecoin => "SAFE-STABLECOIN",
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ConnectivityConsumer {
    AccountRiskState,
    AccountWideExposureGuard,
    CanonicalOrderState,
    ClockAndMaintenanceGuard,
    DerivativeValuationAndSafety,
    ForbiddenExposureProof,
    FundingAwarePricingAndRisk,
    HedgeExecution,
    ImpliedDepthAndRepricing,
    IndexDeviationValuationAndPricing,
    MetadataAndAccountModeGuard,
    OwnedCancellation,
    PositionRiskState,
    QuoteAndHedgeCalculations,
    QuoteAndHedgePriceBounds,
    QuoteExecution,
    RegularDeadman,
    SafetyOwnedCancellation,
    StablecoinEntryGuard,
    StateConvergence,
    TimeBasedStrategyLogic,
}

#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct RequirementUse {
    requirement_id: ConnectivityRequirementId,
    consumer: ConnectivityConsumer,
}

impl RequirementUse {
    pub const fn requirement_id(&self) -> ConnectivityRequirementId {
        self.requirement_id
    }

    pub const fn consumer(&self) -> ConnectivityConsumer {
        self.consumer
    }

    fn new(requirement_id: ConnectivityRequirementId) -> Self {
        let consumer = match requirement_id {
            ConnectivityRequirementId::ChaosExecCancelOwned => {
                ConnectivityConsumer::OwnedCancellation
            }
            ConnectivityRequirementId::ChaosExecHedge => ConnectivityConsumer::HedgeExecution,
            ConnectivityRequirementId::ChaosExecQuote => ConnectivityConsumer::QuoteExecution,
            ConnectivityRequirementId::ChaosMdBook => {
                ConnectivityConsumer::QuoteAndHedgeCalculations
            }
            ConnectivityRequirementId::ChaosMdTrade => {
                ConnectivityConsumer::ImpliedDepthAndRepricing
            }
            ConnectivityRequirementId::ChaosRefFunding => {
                ConnectivityConsumer::FundingAwarePricingAndRisk
            }
            ConnectivityRequirementId::ChaosRefIndex => {
                ConnectivityConsumer::IndexDeviationValuationAndPricing
            }
            ConnectivityRequirementId::ChaosRefLimits => {
                ConnectivityConsumer::QuoteAndHedgePriceBounds
            }
            ConnectivityRequirementId::ChaosRefMark => {
                ConnectivityConsumer::DerivativeValuationAndSafety
            }
            ConnectivityRequirementId::ChaosStateAccount => ConnectivityConsumer::AccountRiskState,
            ConnectivityRequirementId::ChaosStateOrders => {
                ConnectivityConsumer::CanonicalOrderState
            }
            ConnectivityRequirementId::ChaosStatePositions => {
                ConnectivityConsumer::PositionRiskState
            }
            ConnectivityRequirementId::ChaosTimer => ConnectivityConsumer::TimeBasedStrategyLogic,
            ConnectivityRequirementId::SafeAccountPositions => {
                ConnectivityConsumer::AccountWideExposureGuard
            }
            ConnectivityRequirementId::SafeClockStatus => {
                ConnectivityConsumer::ClockAndMaintenanceGuard
            }
            ConnectivityRequirementId::SafeForbiddenZero => {
                ConnectivityConsumer::ForbiddenExposureProof
            }
            ConnectivityRequirementId::SafeMetadata => {
                ConnectivityConsumer::MetadataAndAccountModeGuard
            }
            ConnectivityRequirementId::SafeReconcile => ConnectivityConsumer::StateConvergence,
            ConnectivityRequirementId::SafeRegularCaa => ConnectivityConsumer::RegularDeadman,
            ConnectivityRequirementId::SafeRegularCancel => {
                ConnectivityConsumer::SafetyOwnedCancellation
            }
            ConnectivityRequirementId::SafeStablecoin => ConnectivityConsumer::StablecoinEntryGuard,
        };
        Self {
            requirement_id,
            consumer,
        }
    }
}

fn requirement_uses(
    ids: impl IntoIterator<Item = ConnectivityRequirementId>,
) -> Vec<RequirementUse> {
    ids.into_iter()
        .map(RequirementUse::new)
        .collect::<BTreeSet<_>>()
        .into_iter()
        .collect()
}

/// Exact registry-backed wire surface admitted by a plan item.
#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct CapabilitySurface {
    capability_id: String,
    endpoint_or_channel: String,
    operation: String,
}

impl CapabilitySurface {
    pub fn capability_id(&self) -> &str {
        &self.capability_id
    }

    pub fn endpoint_or_channel(&self) -> &str {
        &self.endpoint_or_channel
    }

    pub fn operation(&self) -> &str {
        &self.operation
    }
}

fn capability_surface(capability_id: &'static str) -> CapabilitySurface {
    let registration = okx_capability_registration(capability_id)
        .expect("resolved Chaos capability must remain registered");
    assert!(
        registration.allowed_in_live_plan,
        "resolved Chaos capability must be admitted to the live plan"
    );
    CapabilitySurface {
        capability_id: registration.capability_id.to_string(),
        endpoint_or_channel: registration.endpoint_or_channel.to_string(),
        operation: registration.operation.to_string(),
    }
}

fn capability_surfaces(ids: impl IntoIterator<Item = &'static str>) -> Vec<CapabilitySurface> {
    ids.into_iter()
        .map(capability_surface)
        .collect::<BTreeSet<_>>()
        .into_iter()
        .collect()
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ChaosAccountRequirements {
    account_id: String,
    symbols: Vec<String>,
    has_derivatives: bool,
    quote_enabled: bool,
    hedge_enabled: bool,
    requirements: Vec<RequirementUse>,
}

impl ChaosAccountRequirements {
    pub fn account_id(&self) -> &str {
        &self.account_id
    }

    pub fn symbols(&self) -> &[String] {
        &self.symbols
    }

    pub const fn has_derivatives(&self) -> bool {
        self.has_derivatives
    }

    pub const fn quote_enabled(&self) -> bool {
        self.quote_enabled
    }

    pub const fn hedge_enabled(&self) -> bool {
        self.hedge_enabled
    }

    pub fn requirements(&self) -> &[RequirementUse] {
        &self.requirements
    }

    fn requirement_uses(
        &self,
        ids: impl IntoIterator<Item = ConnectivityRequirementId>,
    ) -> Vec<RequirementUse> {
        let required = self
            .requirements
            .iter()
            .map(RequirementUse::requirement_id)
            .collect::<BTreeSet<_>>();
        requirement_uses(ids)
            .into_iter()
            .filter(|requirement| required.contains(&requirement.requirement_id()))
            .collect()
    }
}

/// Venue-neutral Chaos inputs plus explicit live risk/account requirements.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ChaosConnectivityRequirements {
    decision: ChaosDecisionRequirements,
    accounts: Vec<ChaosAccountRequirements>,
    stablecoin_symbols: Vec<String>,
    global_requirements: Vec<RequirementUse>,
}

impl ChaosConnectivityRequirements {
    pub fn from_config(config: &LiveConfig) -> Result<Self, ChaosConnectivityPlanError> {
        config.ensure_valid()?;
        let decision = config.strategy.decision_requirements();
        let mut accounts = config
            .accounts
            .iter()
            .map(|account| ChaosAccountRequirements {
                account_id: account.id.clone(),
                symbols: Vec::new(),
                has_derivatives: false,
                quote_enabled: false,
                hedge_enabled: false,
                requirements: Vec::new(),
            })
            .collect::<Vec<_>>();
        accounts.sort_by(|left, right| left.account_id.cmp(&right.account_id));
        for instrument in &config.strategy.instruments {
            let account = config
                .account_for_symbol(&instrument.symbol)
                .ok_or_else(|| ChaosConnectivityPlanError::MissingAccountOwner {
                    symbol: instrument.symbol.clone(),
                })?;
            let target = accounts
                .iter_mut()
                .find(|entry| entry.account_id == account.id)
                .expect("validated live account must be present");
            target.symbols.push(instrument.symbol.clone());
            target.has_derivatives |= instrument.kind.is_derivative();
            let group_is_reference_only = config
                .strategy
                .risk_groups
                .iter()
                .find(|group| group.name == instrument.risk_group)
                .is_some_and(|group| group.kind == reap_strategy::RiskGroupKindConfig::RefOnly);
            if !instrument.halted && !group_is_reference_only {
                target.quote_enabled |= instrument.quote_profit_margin < 1.0;
                target.hedge_enabled |= instrument.hedge_profit_margin < 1.0;
            }
        }
        for account in &mut accounts {
            account.symbols.sort();
            account.symbols.dedup();
            let mut ids = vec![
                ConnectivityRequirementId::SafeAccountPositions,
                ConnectivityRequirementId::SafeForbiddenZero,
                ConnectivityRequirementId::SafeMetadata,
                ConnectivityRequirementId::SafeReconcile,
            ];
            if account.has_derivatives {
                ids.push(ConnectivityRequirementId::ChaosStatePositions);
            }
            if account.quote_enabled {
                ids.push(ConnectivityRequirementId::ChaosExecQuote);
            }
            if account.hedge_enabled {
                ids.push(ConnectivityRequirementId::ChaosExecHedge);
            }
            if account.quote_enabled || account.hedge_enabled {
                ids.extend([
                    ConnectivityRequirementId::ChaosStateAccount,
                    ConnectivityRequirementId::ChaosStateOrders,
                    ConnectivityRequirementId::ChaosExecCancelOwned,
                    ConnectivityRequirementId::SafeRegularCaa,
                    ConnectivityRequirementId::SafeRegularCancel,
                ]);
            }
            account.requirements = requirement_uses(ids);
        }
        let stablecoin_symbols = config
            .risk
            .stablecoin_guards
            .iter()
            .map(|guard| guard.symbol.clone())
            .collect::<BTreeSet<_>>()
            .into_iter()
            .collect::<Vec<_>>();
        Ok(Self {
            decision,
            accounts,
            stablecoin_symbols,
            global_requirements: requirement_uses([ConnectivityRequirementId::SafeClockStatus]),
        })
    }

    pub fn decision(&self) -> &ChaosDecisionRequirements {
        &self.decision
    }

    pub fn accounts(&self) -> &[ChaosAccountRequirements] {
        &self.accounts
    }

    pub fn stablecoin_symbols(&self) -> &[String] {
        &self.stablecoin_symbols
    }

    pub fn global_requirements(&self) -> &[RequirementUse] {
        &self.global_requirements
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize)]
pub enum PublicChannelPlan {
    #[serde(rename = "books")]
    Books,
    #[serde(rename = "funding-rate")]
    FundingRate,
    #[serde(rename = "index-tickers")]
    IndexTickers,
    #[serde(rename = "mark-price")]
    MarkPrice,
    #[serde(rename = "price-limit")]
    PriceLimit,
    #[serde(rename = "trades")]
    Trades,
}

impl PublicChannelPlan {
    pub const fn capability_id(self) -> &'static str {
        match self {
            Self::Books => "OKX-WS-BOOKS",
            Self::FundingRate => "OKX-WS-FUNDING-RATE",
            Self::IndexTickers => "OKX-WS-INDEX-TICKERS",
            Self::MarkPrice => "OKX-WS-MARK-PRICE",
            Self::PriceLimit => "OKX-WS-PRICE-LIMIT",
            Self::Trades => "OKX-WS-TRADES",
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum PublicRedundancyConsumer {
    IndependentBookSequenceArbitrationAndRecovery,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct PublicSubscriptionPlan {
    channel: PublicChannelPlan,
    symbol: String,
    channel_surface: CapabilitySurface,
    session_surfaces: Vec<CapabilitySurface>,
    replica_count: u16,
    redundancy_consumer: Option<PublicRedundancyConsumer>,
    data_max_age_ms: Option<u64>,
    connectivity_health_max_age_ms: u64,
    requirements: Vec<RequirementUse>,
}

impl PublicSubscriptionPlan {
    pub const fn channel(&self) -> PublicChannelPlan {
        self.channel
    }

    pub fn symbol(&self) -> &str {
        &self.symbol
    }

    pub fn channel_surface(&self) -> &CapabilitySurface {
        &self.channel_surface
    }

    pub fn session_surfaces(&self) -> &[CapabilitySurface] {
        &self.session_surfaces
    }

    pub const fn replica_count(&self) -> u16 {
        self.replica_count
    }

    pub const fn redundancy_consumer(&self) -> Option<PublicRedundancyConsumer> {
        self.redundancy_consumer
    }

    pub const fn data_max_age_ms(&self) -> Option<u64> {
        self.data_max_age_ms
    }

    pub const fn connectivity_health_max_age_ms(&self) -> u64 {
        self.connectivity_health_max_age_ms
    }

    pub fn requirements(&self) -> &[RequirementUse] {
        &self.requirements
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct LocalTimerPlan {
    interval_ms: u64,
    requirements: Vec<RequirementUse>,
}

impl LocalTimerPlan {
    pub const fn interval_ms(&self) -> u64 {
        self.interval_ms
    }

    pub fn requirements(&self) -> &[RequirementUse] {
        &self.requirements
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum PrivateChannelPlan {
    Account,
    Fills,
    Orders,
    Positions,
}

impl PrivateChannelPlan {
    pub const fn capability_id(self) -> &'static str {
        match self {
            Self::Account => "OKX-WS-ACCOUNT",
            Self::Fills => "OKX-WS-FILLS",
            Self::Orders => "OKX-WS-ORDERS",
            Self::Positions => "OKX-WS-POSITIONS",
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct PrivateChannelBinding {
    channel: PrivateChannelPlan,
    surface: CapabilitySurface,
    requirements: Vec<RequirementUse>,
}

impl PrivateChannelBinding {
    pub const fn channel(&self) -> PrivateChannelPlan {
        self.channel
    }

    pub fn surface(&self) -> &CapabilitySurface {
        &self.surface
    }

    pub fn requirements(&self) -> &[RequirementUse] {
        &self.requirements
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct PrivateStateSessionPlan {
    account_id: String,
    socket_count: u16,
    session_surfaces: Vec<CapabilitySurface>,
    connectivity_health_max_age_ms: u64,
    channels: Vec<PrivateChannelBinding>,
    requirements: Vec<RequirementUse>,
}

impl PrivateStateSessionPlan {
    pub fn account_id(&self) -> &str {
        &self.account_id
    }

    pub const fn socket_count(&self) -> u16 {
        self.socket_count
    }

    pub fn session_surfaces(&self) -> &[CapabilitySurface] {
        &self.session_surfaces
    }

    pub const fn connectivity_health_max_age_ms(&self) -> u64 {
        self.connectivity_health_max_age_ms
    }

    pub fn channels(&self) -> &[PrivateChannelBinding] {
        &self.channels
    }

    pub fn requirements(&self) -> &[RequirementUse] {
        &self.requirements
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum AuthenticatedReadOperation {
    AccountBalance,
    AccountConfig,
    AccountInstruments,
    AccountPositions,
    Fills,
    OrderDetails,
    RegularPending,
    TradeFee,
}

impl AuthenticatedReadOperation {
    pub const fn capability_id(self) -> &'static str {
        match self {
            Self::AccountBalance => "OKX-REST-ACCOUNT-BALANCE",
            Self::AccountConfig => "OKX-REST-ACCOUNT-CONFIG",
            Self::AccountInstruments => "OKX-REST-ACCOUNT-INSTRUMENTS",
            Self::AccountPositions => "OKX-REST-ACCOUNT-POSITIONS",
            Self::Fills => "OKX-REST-FILLS",
            Self::OrderDetails => "OKX-REST-ORDER-DETAILS",
            Self::RegularPending => "OKX-REST-REGULAR-PENDING",
            Self::TradeFee => "OKX-REST-ACCOUNT-TRADE-FEE",
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum PublicSafetyReadOperation {
    SystemStatus,
    Time,
}

impl PublicSafetyReadOperation {
    pub const fn capability_id(self) -> &'static str {
        match self {
            Self::SystemStatus => "OKX-REST-SYSTEM-STATUS",
            Self::Time => "OKX-REST-PUBLIC-TIME",
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct PublicSafetyReadPlan {
    operation: PublicSafetyReadOperation,
    surface: CapabilitySurface,
    requirements: Vec<RequirementUse>,
}

impl PublicSafetyReadPlan {
    pub const fn operation(&self) -> PublicSafetyReadOperation {
        self.operation
    }

    pub fn surface(&self) -> &CapabilitySurface {
        &self.surface
    }

    pub fn requirements(&self) -> &[RequirementUse] {
        &self.requirements
    }
}

#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct AuthenticatedReadPlan {
    operation: AuthenticatedReadOperation,
    account_id: String,
    symbol: Option<String>,
    surface: CapabilitySurface,
    requirements: Vec<RequirementUse>,
}

impl AuthenticatedReadPlan {
    pub const fn operation(&self) -> AuthenticatedReadOperation {
        self.operation
    }

    pub fn account_id(&self) -> &str {
        &self.account_id
    }

    pub fn symbol(&self) -> Option<&str> {
        self.symbol.as_deref()
    }

    pub fn surface(&self) -> &CapabilitySurface {
        &self.surface
    }

    pub fn requirements(&self) -> &[RequirementUse] {
        &self.requirements
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ForbiddenOrderQuery {
    Chase,
    ConditionalAndOco,
    Iceberg,
    MoveOrderStop,
    SmartIceberg,
    Spread,
    Trigger,
    Twap,
}

impl ForbiddenOrderQuery {
    pub const fn capability_id(self) -> &'static str {
        match self {
            Self::Spread => "OKX-REST-SPREAD-PENDING",
            Self::Chase
            | Self::ConditionalAndOco
            | Self::Iceberg
            | Self::MoveOrderStop
            | Self::SmartIceberg
            | Self::Trigger
            | Self::Twap => "OKX-REST-ALGO-PENDING",
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ForbiddenOrderCheckPlan {
    account_id: String,
    query: ForbiddenOrderQuery,
    surface: CapabilitySurface,
    requirements: Vec<RequirementUse>,
}

impl ForbiddenOrderCheckPlan {
    pub fn account_id(&self) -> &str {
        &self.account_id
    }

    pub const fn query(&self) -> ForbiddenOrderQuery {
        self.query
    }

    pub fn surface(&self) -> &CapabilitySurface {
        &self.surface
    }

    pub fn requirements(&self) -> &[RequirementUse] {
        &self.requirements
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ForbiddenProofPolicy {
    max_age_ms: u64,
    scan_interval_ms: u64,
    hard_max_age_ms: u64,
    requirements: Vec<RequirementUse>,
}

impl ForbiddenProofPolicy {
    pub const fn max_age_ms(&self) -> u64 {
        self.max_age_ms
    }

    pub const fn scan_interval_ms(&self) -> u64 {
        self.scan_interval_ms
    }

    pub const fn hard_max_age_ms(&self) -> u64 {
        self.hard_max_age_ms
    }

    pub fn requirements(&self) -> &[RequirementUse] {
        &self.requirements
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum RegularMutationOperation {
    RestCancelRegular,
    RestRegularCancelAllAfter,
    WebsocketCancelRegular,
    WebsocketPlaceRegular,
}

impl RegularMutationOperation {
    pub const fn capability_id(self) -> &'static str {
        match self {
            Self::RestCancelRegular => "OKX-REST-CANCEL-REGULAR",
            Self::RestRegularCancelAllAfter => "OKX-REST-REGULAR-CAA",
            Self::WebsocketCancelRegular => "OKX-WS-CANCEL-REGULAR",
            Self::WebsocketPlaceRegular => "OKX-WS-PLACE-REGULAR",
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct RegularMutationPlan {
    operation: RegularMutationOperation,
    account_id: String,
    surface: CapabilitySurface,
    requirements: Vec<RequirementUse>,
}

impl RegularMutationPlan {
    pub const fn operation(&self) -> RegularMutationOperation {
        self.operation
    }

    pub fn account_id(&self) -> &str {
        &self.account_id
    }

    pub fn surface(&self) -> &CapabilitySurface {
        &self.surface
    }

    pub fn requirements(&self) -> &[RequirementUse] {
        &self.requirements
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct OrderCommandLanePlan {
    account_id: String,
    lane_index: u16,
    dispatch_families: Vec<String>,
    session_surfaces: Vec<CapabilitySurface>,
    requirements: Vec<RequirementUse>,
}

impl OrderCommandLanePlan {
    pub fn account_id(&self) -> &str {
        &self.account_id
    }

    pub const fn lane_index(&self) -> u16 {
        self.lane_index
    }

    pub fn dispatch_families(&self) -> &[String] {
        &self.dispatch_families
    }

    pub fn session_surfaces(&self) -> &[CapabilitySurface] {
        &self.session_surfaces
    }

    pub fn requirements(&self) -> &[RequirementUse] {
        &self.requirements
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum LiveConnectivityRole {
    ClockAndStatusRead,
    ForbiddenOrderObservation,
    MetadataRead,
    PrivateStateObservation,
    PublicMarketObservation,
    ReconciliationRead,
    RegularExecution,
    RegularLiveSafety,
}

#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct LiveConnectivityRolePlan {
    role: LiveConnectivityRole,
    requirements: Vec<RequirementUse>,
}

impl LiveConnectivityRolePlan {
    pub const fn role(&self) -> LiveConnectivityRole {
        self.role
    }

    pub fn requirements(&self) -> &[RequirementUse] {
        &self.requirements
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum MaintenanceServicePlan {
    OtherAmbiguous,
    Trading,
    TradingAccounts,
    TradingProducts,
    Websocket,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum MaintenanceProductPlan {
    Futures,
    Spot,
    Swap,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct MaintenanceRelevancePlan {
    unified_system: bool,
    environment: TradingEnvironment,
    services: Vec<MaintenanceServicePlan>,
    products: Vec<MaintenanceProductPlan>,
    surface: CapabilitySurface,
    requirements: Vec<RequirementUse>,
}

impl MaintenanceRelevancePlan {
    pub const fn unified_system(&self) -> bool {
        self.unified_system
    }

    pub const fn environment(&self) -> TradingEnvironment {
        self.environment
    }

    pub fn services(&self) -> &[MaintenanceServicePlan] {
        &self.services
    }

    pub fn products(&self) -> &[MaintenanceProductPlan] {
        &self.products
    }

    pub fn surface(&self) -> &CapabilitySurface {
        &self.surface
    }

    pub fn requirements(&self) -> &[RequirementUse] {
        &self.requirements
    }
}

/// Canonical, schema-versioned and secret-free OKX plan for one Chaos live mode.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(deny_unknown_fields)]
pub struct ChaosConnectivityPlan {
    schema_version: u32,
    java_reference_revision: String,
    mode: LiveMode,
    environment: TradingEnvironment,
    symbols: Vec<String>,
    account_ids: Vec<String>,
    public_subscriptions: Vec<PublicSubscriptionPlan>,
    local_timers: Vec<LocalTimerPlan>,
    private_state_sessions: Vec<PrivateStateSessionPlan>,
    public_safety_reads: Vec<PublicSafetyReadPlan>,
    authenticated_reads: Vec<AuthenticatedReadPlan>,
    forbidden_order_checks: Vec<ForbiddenOrderCheckPlan>,
    forbidden_proof_policy: ForbiddenProofPolicy,
    regular_mutations: Vec<RegularMutationPlan>,
    command_lanes: Vec<OrderCommandLanePlan>,
    roles: Vec<LiveConnectivityRolePlan>,
    maintenance_relevance: MaintenanceRelevancePlan,
}

impl ChaosConnectivityPlan {
    pub fn resolve(
        config: &LiveConfig,
        mode: LiveMode,
    ) -> Result<Self, ChaosConnectivityPlanError> {
        if mode == LiveMode::Demo && config.venue.environment != TradingEnvironment::Demo {
            return Err(ChaosConnectivityPlanError::ProductionOrderEntryUnavailable);
        }
        let requirements = ChaosConnectivityRequirements::from_config(config)?;
        let public_subscriptions = resolve_public_subscriptions(config, &requirements);
        let private_state_sessions = resolve_private_sessions(config, &requirements);
        let public_safety_reads = resolve_public_safety_reads(&requirements);
        let authenticated_reads = resolve_authenticated_reads(&requirements);
        let forbidden_order_checks = resolve_forbidden_checks(&requirements);
        let regular_mutations = resolve_regular_mutations(mode, &requirements);
        let command_lanes = resolve_command_lanes(mode, &requirements);
        let roles = resolve_roles(
            mode,
            &public_subscriptions,
            &private_state_sessions,
            &authenticated_reads,
            &forbidden_order_checks,
            &regular_mutations,
        );
        let symbols = config
            .strategy
            .instruments
            .iter()
            .map(|instrument| instrument.symbol.clone())
            .collect::<BTreeSet<_>>()
            .into_iter()
            .collect();
        let account_ids = requirements
            .accounts()
            .iter()
            .map(|account| account.account_id.clone())
            .collect();
        let products = config
            .strategy
            .instruments
            .iter()
            .map(|instrument| {
                if instrument.kind.is_spot() {
                    MaintenanceProductPlan::Spot
                } else if instrument.kind.is_swap() {
                    MaintenanceProductPlan::Swap
                } else {
                    MaintenanceProductPlan::Futures
                }
            })
            .collect::<BTreeSet<_>>()
            .into_iter()
            .collect();
        Ok(Self {
            schema_version: CHAOS_CONNECTIVITY_PLAN_SCHEMA_VERSION,
            java_reference_revision: PINNED_JAVA_REVISION.to_string(),
            mode,
            environment: config.venue.environment,
            symbols,
            account_ids,
            public_subscriptions,
            local_timers: vec![LocalTimerPlan {
                interval_ms: config.runtime.timer_interval_ms,
                requirements: requirement_uses([ConnectivityRequirementId::ChaosTimer]),
            }],
            private_state_sessions,
            public_safety_reads,
            authenticated_reads,
            forbidden_order_checks,
            forbidden_proof_policy: ForbiddenProofPolicy {
                max_age_ms: FORBIDDEN_PROOF_DEFAULT_MAX_AGE_MS,
                scan_interval_ms: FORBIDDEN_PROOF_DEFAULT_SCAN_INTERVAL_MS,
                hard_max_age_ms: FORBIDDEN_PROOF_HARD_MAX_AGE_MS,
                requirements: requirement_uses([ConnectivityRequirementId::SafeForbiddenZero]),
            },
            regular_mutations,
            command_lanes,
            roles,
            maintenance_relevance: MaintenanceRelevancePlan {
                unified_system: true,
                environment: config.venue.environment,
                services: vec![
                    MaintenanceServicePlan::OtherAmbiguous,
                    MaintenanceServicePlan::Trading,
                    MaintenanceServicePlan::TradingAccounts,
                    MaintenanceServicePlan::TradingProducts,
                    MaintenanceServicePlan::Websocket,
                ],
                products,
                surface: capability_surface("OKX-MAINTENANCE-FILTER"),
                requirements: requirement_uses([ConnectivityRequirementId::SafeClockStatus]),
            },
        })
    }

    pub const fn schema_version(&self) -> u32 {
        self.schema_version
    }

    pub fn java_reference_revision(&self) -> &str {
        &self.java_reference_revision
    }

    pub const fn mode(&self) -> LiveMode {
        self.mode
    }

    pub const fn environment(&self) -> TradingEnvironment {
        self.environment
    }

    pub fn symbols(&self) -> &[String] {
        &self.symbols
    }

    pub fn account_ids(&self) -> &[String] {
        &self.account_ids
    }

    pub fn public_subscriptions(&self) -> &[PublicSubscriptionPlan] {
        &self.public_subscriptions
    }

    pub fn local_timers(&self) -> &[LocalTimerPlan] {
        &self.local_timers
    }

    pub fn private_state_sessions(&self) -> &[PrivateStateSessionPlan] {
        &self.private_state_sessions
    }

    pub fn public_safety_reads(&self) -> &[PublicSafetyReadPlan] {
        &self.public_safety_reads
    }

    pub fn authenticated_reads(&self) -> &[AuthenticatedReadPlan] {
        &self.authenticated_reads
    }

    pub fn forbidden_order_checks(&self) -> &[ForbiddenOrderCheckPlan] {
        &self.forbidden_order_checks
    }

    pub const fn forbidden_proof_policy(&self) -> &ForbiddenProofPolicy {
        &self.forbidden_proof_policy
    }

    pub fn regular_mutations(&self) -> &[RegularMutationPlan] {
        &self.regular_mutations
    }

    pub fn command_lanes(&self) -> &[OrderCommandLanePlan] {
        &self.command_lanes
    }

    pub fn roles(&self) -> &[LiveConnectivityRolePlan] {
        &self.roles
    }

    pub fn maintenance_relevance(&self) -> &MaintenanceRelevancePlan {
        &self.maintenance_relevance
    }

    pub fn canonical_bytes(&self) -> Vec<u8> {
        serde_json::to_vec(self).expect("Chaos connectivity plan has no fallible JSON values")
    }

    pub fn canonical_json(&self) -> String {
        String::from_utf8(self.canonical_bytes())
            .expect("JSON serialization always produces valid UTF-8")
    }

    pub fn sha256(&self) -> String {
        format!("{:x}", Sha256::digest(self.canonical_bytes()))
    }
}

#[derive(Debug)]
struct PublicSubscriptionBuilder {
    replica_count: u16,
    redundancy_consumer: Option<PublicRedundancyConsumer>,
    max_age_ms: Option<u64>,
    requirements: BTreeSet<RequirementUse>,
}

fn resolve_public_subscriptions(
    config: &LiveConfig,
    requirements: &ChaosConnectivityRequirements,
) -> Vec<PublicSubscriptionPlan> {
    let mut subscriptions =
        BTreeMap::<(PublicChannelPlan, String), PublicSubscriptionBuilder>::new();
    for decision in requirements.decision().inputs() {
        let (channel, symbol, replica_count, redundancy_consumer, max_age_ms, requirement_id) =
            match decision.input() {
                ChaosDecisionInput::Book { symbol, max_age_ms } => (
                    PublicChannelPlan::Books,
                    symbol.clone(),
                    2,
                    Some(PublicRedundancyConsumer::IndependentBookSequenceArbitrationAndRecovery),
                    Some(*max_age_ms),
                    ConnectivityRequirementId::ChaosMdBook,
                ),
                ChaosDecisionInput::Trade { symbol } => (
                    PublicChannelPlan::Trades,
                    symbol.clone(),
                    1,
                    None,
                    None,
                    ConnectivityRequirementId::ChaosMdTrade,
                ),
                ChaosDecisionInput::Reference {
                    kind,
                    symbol,
                    max_age_ms,
                } => (
                    match kind {
                        ReferenceDataKind::IndexPrice => PublicChannelPlan::IndexTickers,
                        ReferenceDataKind::FundingRate => PublicChannelPlan::FundingRate,
                        ReferenceDataKind::MarkPrice => PublicChannelPlan::MarkPrice,
                        ReferenceDataKind::PriceLimits => PublicChannelPlan::PriceLimit,
                    },
                    symbol.clone(),
                    1,
                    None,
                    *max_age_ms,
                    map_decision_requirement(decision.requirement_id()),
                ),
                ChaosDecisionInput::Timer => continue,
            };
        merge_public_subscription(
            &mut subscriptions,
            channel,
            symbol,
            replica_count,
            redundancy_consumer,
            max_age_ms,
            requirement_id,
        );
    }
    for symbol in requirements.stablecoin_symbols() {
        merge_public_subscription(
            &mut subscriptions,
            PublicChannelPlan::IndexTickers,
            symbol.clone(),
            1,
            None,
            Some(config.risk.stablecoin_max_age_ms),
            ConnectivityRequirementId::SafeStablecoin,
        );
    }
    subscriptions
        .into_iter()
        .map(|((channel, symbol), builder)| PublicSubscriptionPlan {
            channel,
            symbol,
            channel_surface: capability_surface(channel.capability_id()),
            session_surfaces: capability_surfaces([
                "OKX-CONNECTION-PUBLIC",
                "OKX-WS-LIVENESS",
                "OKX-WS-SUBSCRIBE",
            ]),
            replica_count: builder.replica_count,
            redundancy_consumer: builder.redundancy_consumer,
            data_max_age_ms: builder.max_age_ms,
            connectivity_health_max_age_ms: config.risk.max_feed_age_ms,
            requirements: builder.requirements.into_iter().collect(),
        })
        .collect()
}

#[allow(clippy::too_many_arguments)]
fn merge_public_subscription(
    subscriptions: &mut BTreeMap<(PublicChannelPlan, String), PublicSubscriptionBuilder>,
    channel: PublicChannelPlan,
    symbol: String,
    replica_count: u16,
    redundancy_consumer: Option<PublicRedundancyConsumer>,
    max_age_ms: Option<u64>,
    requirement_id: ConnectivityRequirementId,
) {
    let entry =
        subscriptions
            .entry((channel, symbol))
            .or_insert_with(|| PublicSubscriptionBuilder {
                replica_count,
                redundancy_consumer,
                max_age_ms,
                requirements: BTreeSet::new(),
            });
    entry.replica_count = entry.replica_count.max(replica_count);
    if redundancy_consumer.is_some() {
        entry.redundancy_consumer = redundancy_consumer;
    }
    entry.max_age_ms = match (entry.max_age_ms, max_age_ms) {
        (Some(left), Some(right)) => Some(left.min(right)),
        (Some(value), None) | (None, Some(value)) => Some(value),
        (None, None) => None,
    };
    entry
        .requirements
        .insert(RequirementUse::new(requirement_id));
}

fn map_decision_requirement(id: ChaosDecisionRequirementId) -> ConnectivityRequirementId {
    match id {
        ChaosDecisionRequirementId::MarketBook => ConnectivityRequirementId::ChaosMdBook,
        ChaosDecisionRequirementId::MarketTrade => ConnectivityRequirementId::ChaosMdTrade,
        ChaosDecisionRequirementId::ReferenceIndex => ConnectivityRequirementId::ChaosRefIndex,
        ChaosDecisionRequirementId::ReferenceFunding => ConnectivityRequirementId::ChaosRefFunding,
        ChaosDecisionRequirementId::ReferenceMark => ConnectivityRequirementId::ChaosRefMark,
        ChaosDecisionRequirementId::ReferencePriceLimits => {
            ConnectivityRequirementId::ChaosRefLimits
        }
        ChaosDecisionRequirementId::Timer => ConnectivityRequirementId::ChaosTimer,
    }
}

fn resolve_private_sessions(
    config: &LiveConfig,
    requirements: &ChaosConnectivityRequirements,
) -> Vec<PrivateStateSessionPlan> {
    requirements
        .accounts()
        .iter()
        .map(|account| {
            let mut channels = [
                PrivateChannelBinding {
                    channel: PrivateChannelPlan::Account,
                    surface: capability_surface(PrivateChannelPlan::Account.capability_id()),
                    requirements: account
                        .requirement_uses([ConnectivityRequirementId::ChaosStateAccount]),
                },
                PrivateChannelBinding {
                    channel: PrivateChannelPlan::Orders,
                    surface: capability_surface(PrivateChannelPlan::Orders.capability_id()),
                    requirements: account
                        .requirement_uses([ConnectivityRequirementId::ChaosStateOrders]),
                },
                PrivateChannelBinding {
                    channel: PrivateChannelPlan::Positions,
                    surface: capability_surface(PrivateChannelPlan::Positions.capability_id()),
                    requirements: account.requirement_uses([
                        ConnectivityRequirementId::ChaosStatePositions,
                        ConnectivityRequirementId::SafeAccountPositions,
                    ]),
                },
            ]
            .into_iter()
            .filter(|binding| !binding.requirements.is_empty())
            .collect::<Vec<_>>();
            if config.venue.enable_vip_fills_channel
                && account.requirements().iter().any(|requirement| {
                    requirement.requirement_id() == ConnectivityRequirementId::ChaosStateOrders
                })
            {
                let fills = PrivateChannelBinding {
                    channel: PrivateChannelPlan::Fills,
                    surface: capability_surface(PrivateChannelPlan::Fills.capability_id()),
                    requirements: account.requirement_uses([
                        ConnectivityRequirementId::ChaosStateOrders,
                        ConnectivityRequirementId::SafeReconcile,
                    ]),
                };
                if !fills.requirements.is_empty() {
                    channels.push(fills);
                }
            }
            channels.sort();
            let requirements = channels
                .iter()
                .flat_map(|channel| channel.requirements.iter().cloned())
                .collect::<BTreeSet<_>>()
                .into_iter()
                .collect();
            PrivateStateSessionPlan {
                account_id: account.account_id.clone(),
                socket_count: 1,
                session_surfaces: capability_surfaces([
                    "OKX-CONNECTION-PRIVATE-STATE",
                    "OKX-WS-LIVENESS",
                    "OKX-WS-LOGIN",
                    "OKX-WS-SUBSCRIBE",
                ]),
                connectivity_health_max_age_ms: config.risk.max_private_age_ms,
                channels,
                requirements,
            }
        })
        .collect()
}

fn resolve_authenticated_reads(
    requirements: &ChaosConnectivityRequirements,
) -> Vec<AuthenticatedReadPlan> {
    let mut reads = BTreeSet::new();
    for account in requirements.accounts() {
        for (operation, ids) in [
            (
                AuthenticatedReadOperation::AccountBalance,
                vec![ConnectivityRequirementId::SafeReconcile],
            ),
            (
                AuthenticatedReadOperation::AccountConfig,
                vec![ConnectivityRequirementId::SafeMetadata],
            ),
            (
                AuthenticatedReadOperation::AccountPositions,
                vec![
                    ConnectivityRequirementId::SafeAccountPositions,
                    ConnectivityRequirementId::SafeReconcile,
                ],
            ),
            (
                AuthenticatedReadOperation::Fills,
                vec![ConnectivityRequirementId::SafeReconcile],
            ),
            (
                AuthenticatedReadOperation::OrderDetails,
                vec![ConnectivityRequirementId::SafeReconcile],
            ),
            (
                AuthenticatedReadOperation::RegularPending,
                vec![ConnectivityRequirementId::SafeReconcile],
            ),
        ] {
            reads.insert(AuthenticatedReadPlan {
                operation,
                account_id: account.account_id.clone(),
                symbol: None,
                surface: capability_surface(operation.capability_id()),
                requirements: account.requirement_uses(ids),
            });
        }
        for symbol in account.symbols() {
            for operation in [
                AuthenticatedReadOperation::AccountInstruments,
                AuthenticatedReadOperation::TradeFee,
            ] {
                reads.insert(AuthenticatedReadPlan {
                    operation,
                    account_id: account.account_id.clone(),
                    symbol: Some(symbol.clone()),
                    surface: capability_surface(operation.capability_id()),
                    requirements: account
                        .requirement_uses([ConnectivityRequirementId::SafeMetadata]),
                });
            }
        }
    }
    reads.into_iter().collect()
}

fn resolve_public_safety_reads(
    requirements: &ChaosConnectivityRequirements,
) -> Vec<PublicSafetyReadPlan> {
    [
        PublicSafetyReadOperation::SystemStatus,
        PublicSafetyReadOperation::Time,
    ]
    .into_iter()
    .map(|operation| PublicSafetyReadPlan {
        operation,
        surface: capability_surface(operation.capability_id()),
        requirements: requirements.global_requirements().to_vec(),
    })
    .collect()
}

fn resolve_forbidden_checks(
    requirements: &ChaosConnectivityRequirements,
) -> Vec<ForbiddenOrderCheckPlan> {
    const QUERIES: [ForbiddenOrderQuery; 8] = [
        ForbiddenOrderQuery::Chase,
        ForbiddenOrderQuery::ConditionalAndOco,
        ForbiddenOrderQuery::Iceberg,
        ForbiddenOrderQuery::MoveOrderStop,
        ForbiddenOrderQuery::SmartIceberg,
        ForbiddenOrderQuery::Spread,
        ForbiddenOrderQuery::Trigger,
        ForbiddenOrderQuery::Twap,
    ];
    requirements
        .accounts()
        .iter()
        .flat_map(|account| {
            QUERIES.map(|query| ForbiddenOrderCheckPlan {
                account_id: account.account_id.clone(),
                query,
                surface: capability_surface(query.capability_id()),
                requirements: account
                    .requirement_uses([ConnectivityRequirementId::SafeForbiddenZero]),
            })
        })
        .collect()
}

fn resolve_regular_mutations(
    mode: LiveMode,
    requirements: &ChaosConnectivityRequirements,
) -> Vec<RegularMutationPlan> {
    if mode != LiveMode::Demo {
        return Vec::new();
    }
    requirements
        .accounts()
        .iter()
        .flat_map(|account| {
            if !account.quote_enabled() && !account.hedge_enabled() {
                return Vec::new();
            }
            vec![
                RegularMutationPlan {
                    operation: RegularMutationOperation::RestCancelRegular,
                    account_id: account.account_id.clone(),
                    surface: capability_surface(
                        RegularMutationOperation::RestCancelRegular.capability_id(),
                    ),
                    requirements: account.requirement_uses([
                        ConnectivityRequirementId::ChaosExecCancelOwned,
                        ConnectivityRequirementId::SafeRegularCancel,
                    ]),
                },
                RegularMutationPlan {
                    operation: RegularMutationOperation::RestRegularCancelAllAfter,
                    account_id: account.account_id.clone(),
                    surface: capability_surface(
                        RegularMutationOperation::RestRegularCancelAllAfter.capability_id(),
                    ),
                    requirements: account
                        .requirement_uses([ConnectivityRequirementId::SafeRegularCaa]),
                },
                RegularMutationPlan {
                    operation: RegularMutationOperation::WebsocketCancelRegular,
                    account_id: account.account_id.clone(),
                    surface: capability_surface(
                        RegularMutationOperation::WebsocketCancelRegular.capability_id(),
                    ),
                    requirements: account.requirement_uses([
                        ConnectivityRequirementId::ChaosExecCancelOwned,
                        ConnectivityRequirementId::SafeRegularCancel,
                    ]),
                },
                RegularMutationPlan {
                    operation: RegularMutationOperation::WebsocketPlaceRegular,
                    account_id: account.account_id.clone(),
                    surface: capability_surface(
                        RegularMutationOperation::WebsocketPlaceRegular.capability_id(),
                    ),
                    requirements: account.requirement_uses([
                        ConnectivityRequirementId::ChaosExecHedge,
                        ConnectivityRequirementId::ChaosExecQuote,
                    ]),
                },
            ]
        })
        .collect::<BTreeSet<_>>()
        .into_iter()
        .collect()
}

fn resolve_command_lanes(
    mode: LiveMode,
    requirements: &ChaosConnectivityRequirements,
) -> Vec<OrderCommandLanePlan> {
    if mode != LiveMode::Demo {
        return Vec::new();
    }
    requirements
        .accounts()
        .iter()
        .filter_map(|account| {
            let dispatch_families = account
                .symbols()
                .iter()
                .map(|symbol| okx_order_dispatch_key(symbol))
                .collect::<BTreeSet<_>>()
                .into_iter()
                .collect::<Vec<_>>();
            (!dispatch_families.is_empty() && (account.quote_enabled() || account.hedge_enabled()))
                .then(|| OrderCommandLanePlan {
                    account_id: account.account_id.clone(),
                    lane_index: 0,
                    dispatch_families,
                    session_surfaces: capability_surfaces([
                        "OKX-CONNECTION-ORDER-COMMAND",
                        "OKX-WS-LIVENESS",
                        "OKX-WS-LOGIN",
                    ]),
                    requirements: account.requirement_uses([
                        ConnectivityRequirementId::ChaosExecCancelOwned,
                        ConnectivityRequirementId::ChaosExecHedge,
                        ConnectivityRequirementId::ChaosExecQuote,
                        ConnectivityRequirementId::SafeRegularCancel,
                    ]),
                })
        })
        .collect()
}

fn role(
    role: LiveConnectivityRole,
    ids: impl IntoIterator<Item = ConnectivityRequirementId>,
) -> LiveConnectivityRolePlan {
    LiveConnectivityRolePlan {
        role,
        requirements: requirement_uses(ids),
    }
}

fn collect_requirement_ids<'a>(
    requirements: impl IntoIterator<Item = &'a RequirementUse>,
) -> Vec<ConnectivityRequirementId> {
    requirements
        .into_iter()
        .map(RequirementUse::requirement_id)
        .collect::<BTreeSet<_>>()
        .into_iter()
        .collect()
}

fn resolve_roles(
    mode: LiveMode,
    public_subscriptions: &[PublicSubscriptionPlan],
    private_state_sessions: &[PrivateStateSessionPlan],
    authenticated_reads: &[AuthenticatedReadPlan],
    forbidden_order_checks: &[ForbiddenOrderCheckPlan],
    regular_mutations: &[RegularMutationPlan],
) -> Vec<LiveConnectivityRolePlan> {
    if mode == LiveMode::Validate {
        return Vec::new();
    }
    let public_ids = collect_requirement_ids(
        public_subscriptions
            .iter()
            .flat_map(|subscription| subscription.requirements()),
    );
    let private_ids = collect_requirement_ids(
        private_state_sessions
            .iter()
            .flat_map(|session| session.requirements()),
    );
    let metadata_ids = collect_requirement_ids(
        authenticated_reads
            .iter()
            .flat_map(|read| read.requirements())
            .filter(|requirement| {
                requirement.requirement_id() == ConnectivityRequirementId::SafeMetadata
            }),
    );
    let reconciliation_ids = collect_requirement_ids(
        authenticated_reads
            .iter()
            .flat_map(|read| read.requirements())
            .filter(|requirement| {
                matches!(
                    requirement.requirement_id(),
                    ConnectivityRequirementId::SafeAccountPositions
                        | ConnectivityRequirementId::SafeReconcile
                )
            }),
    );
    let forbidden_ids = collect_requirement_ids(
        forbidden_order_checks
            .iter()
            .flat_map(|check| check.requirements()),
    );
    let mut roles = vec![
        role(
            LiveConnectivityRole::ClockAndStatusRead,
            [ConnectivityRequirementId::SafeClockStatus],
        ),
        role(
            LiveConnectivityRole::ForbiddenOrderObservation,
            forbidden_ids,
        ),
        role(LiveConnectivityRole::MetadataRead, metadata_ids),
        role(LiveConnectivityRole::PrivateStateObservation, private_ids),
        role(LiveConnectivityRole::PublicMarketObservation, public_ids),
        role(LiveConnectivityRole::ReconciliationRead, reconciliation_ids),
    ];
    if mode == LiveMode::Demo {
        let execution_ids = collect_requirement_ids(
            regular_mutations
                .iter()
                .flat_map(|mutation| mutation.requirements())
                .filter(|requirement| {
                    matches!(
                        requirement.requirement_id(),
                        ConnectivityRequirementId::ChaosExecCancelOwned
                            | ConnectivityRequirementId::ChaosExecHedge
                            | ConnectivityRequirementId::ChaosExecQuote
                    )
                }),
        );
        let safety_ids = collect_requirement_ids(
            regular_mutations
                .iter()
                .flat_map(|mutation| mutation.requirements())
                .filter(|requirement| {
                    matches!(
                        requirement.requirement_id(),
                        ConnectivityRequirementId::SafeRegularCaa
                            | ConnectivityRequirementId::SafeRegularCancel
                    )
                }),
        );
        if !execution_ids.is_empty() {
            roles.push(role(LiveConnectivityRole::RegularExecution, execution_ids));
        }
        if !safety_ids.is_empty() {
            roles.push(role(LiveConnectivityRole::RegularLiveSafety, safety_ids));
        }
    }
    roles.sort();
    roles
}

#[cfg(test)]
mod tests {
    use super::*;

    fn sample_config() -> LiveConfig {
        LiveConfig::from_toml(include_str!("../../../examples/live-okx-demo.toml")).unwrap()
    }

    fn add_unused_account(config: &mut LiveConfig) {
        let mut account = config.accounts[0].clone();
        account.id = "unused".to_string();
        account.api_key_env = "UNUSED_API_KEY".to_string();
        account.secret_key_env = "UNUSED_SECRET_KEY".to_string();
        account.passphrase_env = "UNUSED_PASSPHRASE".to_string();
        account.id_prefix = "unused".to_string();
        account.node_id = 2;
        account.trade_modes.clear();
        config.accounts.push(account);
    }

    #[test]
    fn equal_effective_configs_have_identical_canonical_bytes_and_hash() {
        let config = sample_config();
        let first = ChaosConnectivityPlan::resolve(&config, LiveMode::Demo).unwrap();
        let second = ChaosConnectivityPlan::resolve(&config.clone(), LiveMode::Demo).unwrap();

        assert_eq!(first.canonical_bytes(), second.canonical_bytes());
        assert_eq!(first.sha256(), second.sha256());
        assert_eq!(
            first.sha256(),
            "6771c97a373f12f77093624ea4b2914d867aae6a710eddadde925fc288fc6477"
        );
        assert_eq!(first.sha256().len(), 64);
        assert_eq!(
            first.schema_version(),
            CHAOS_CONNECTIVITY_PLAN_SCHEMA_VERSION
        );
        assert_eq!(first.java_reference_revision(), PINNED_JAVA_REVISION);
    }

    #[test]
    fn canonicalization_is_independent_of_equivalent_input_ordering() {
        let mut config = sample_config();
        add_unused_account(&mut config);
        let first = ChaosConnectivityPlan::resolve(&config, LiveMode::Demo).unwrap();

        let mut reordered = config.clone();
        reordered.accounts.reverse();
        reordered.strategy.instruments.reverse();
        reordered.strategy.risk_groups.reverse();
        for group in &mut reordered.strategy.risk_groups {
            group.symbols.reverse();
            group.coins.reverse();
        }
        reordered.risk.stablecoin_guards.reverse();
        for account in &mut reordered.accounts {
            let mut modes = account.trade_modes.drain().collect::<Vec<_>>();
            modes.reverse();
            account.trade_modes.extend(modes);
        }
        let second = ChaosConnectivityPlan::resolve(&reordered, LiveMode::Demo).unwrap();

        assert_eq!(first.canonical_bytes(), second.canonical_bytes());
        assert_eq!(first.sha256(), second.sha256());
    }

    #[test]
    fn sample_plan_is_secret_free_and_has_one_nonidle_family_lane() {
        let mut config = sample_config();
        config.accounts[0].api_key_env = "PHASE1_POISON_API_KEY".to_string();
        config.accounts[0].secret_key_env = "PHASE1_POISON_SECRET_KEY".to_string();
        config.accounts[0].passphrase_env = "PHASE1_POISON_PASSPHRASE".to_string();
        config.accounts[0].id_prefix = "Z9POISON".to_string();
        config.storage.path = "var/PHASE1_POISON_STORAGE".into();
        config.operator.socket_path = "var/PHASE1_POISON_OPERATOR".into();
        config.runtime.connection_attempt_pacer_path = Some("var/PHASE1_POISON_PACER".into());
        config.venue.rest_url = "https://us.okx.com".to_string();
        config.venue.public_ws_url = "wss://wsuspap.okx.com:8443/ws/v5/public".to_string();
        config.venue.private_ws_url = "wss://wsuspap.okx.com:8443/ws/v5/private".to_string();
        let plan = ChaosConnectivityPlan::resolve(&config, LiveMode::Demo).unwrap();
        let json = plan.canonical_json();

        assert_eq!(plan.command_lanes().len(), 1);
        assert_eq!(plan.command_lanes()[0].dispatch_families(), &["BTC-USDT"]);
        for forbidden in [
            "PHASE1_POISON_API_KEY",
            "PHASE1_POISON_SECRET_KEY",
            "PHASE1_POISON_PASSPHRASE",
            "Z9POISON",
            "PHASE1_POISON_STORAGE",
            "PHASE1_POISON_OPERATOR",
            "PHASE1_POISON_PACER",
            "us.okx.com",
            "wsuspap.okx.com",
            "emergency",
            "offline_evidence",
            "capture",
            "fault",
            "cancel_algo",
            "spread_mass_cancel",
        ] {
            assert!(!json.contains(forbidden), "plan leaked {forbidden}");
        }
        assert!(json.contains("OKX-CONNECTION-PUBLIC"));
        assert!(json.contains("/api/v5/account/config"));
        assert!(json.contains("\"funding-rate\""));
    }

    #[test]
    fn derivative_matrix_resolves_only_kind_relevant_references() {
        for (kind, symbol, funding_required) in [
            (
                reap_strategy::InstrumentKindConfig::LinearSwap,
                "BTC-USDT-SWAP",
                true,
            ),
            (
                reap_strategy::InstrumentKindConfig::InverseSwap,
                "BTC-USD-SWAP",
                true,
            ),
            (
                reap_strategy::InstrumentKindConfig::InverseFuture,
                "BTC-USD-261225",
                false,
            ),
        ] {
            let mut config = sample_config();
            let old_symbol = config.strategy.instruments[1].symbol.clone();
            config.strategy.instruments[1].symbol = symbol.to_string();
            config.strategy.instruments[1].kind = kind;
            if kind.is_inverse() {
                config.strategy.instruments[1].quote_currency = "USD".to_string();
                config.strategy.instruments[1].settle_currency = "BTC".to_string();
            }
            for configured in &mut config.strategy.risk_groups[0].symbols {
                if *configured == old_symbol {
                    *configured = symbol.to_string();
                }
            }
            let mode = config.accounts[0].trade_modes.remove(&old_symbol).unwrap();
            config.accounts[0]
                .trade_modes
                .insert(symbol.to_string(), mode);

            let plan = ChaosConnectivityPlan::resolve(&config, LiveMode::Demo).unwrap();
            let has = |channel| {
                plan.public_subscriptions().iter().any(|subscription| {
                    subscription.symbol() == symbol && subscription.channel() == channel
                })
            };

            assert!(
                has(PublicChannelPlan::MarkPrice),
                "missing mark for {kind:?}"
            );
            assert!(
                has(PublicChannelPlan::PriceLimit),
                "missing limits for {kind:?}"
            );
            assert_eq!(
                has(PublicChannelPlan::FundingRate),
                funding_required,
                "wrong funding requirement for {kind:?}"
            );
        }
    }

    #[test]
    fn books_are_redundant_and_trades_are_exactly_once_per_instrument() {
        let config = sample_config();
        let plan = ChaosConnectivityPlan::resolve(&config, LiveMode::Demo).unwrap();
        let configured_symbols = config
            .strategy
            .instruments
            .iter()
            .map(|instrument| instrument.symbol.as_str())
            .collect::<BTreeSet<_>>();
        let books = plan
            .public_subscriptions()
            .iter()
            .filter(|subscription| subscription.channel() == PublicChannelPlan::Books)
            .collect::<Vec<_>>();
        let trades = plan
            .public_subscriptions()
            .iter()
            .filter(|subscription| subscription.channel() == PublicChannelPlan::Trades)
            .collect::<Vec<_>>();

        assert_eq!(books.len(), configured_symbols.len());
        assert_eq!(trades.len(), configured_symbols.len());
        assert!(books.iter().all(|subscription| {
            subscription.replica_count() == 2 && subscription.redundancy_consumer().is_some()
        }));
        assert!(
            trades
                .iter()
                .all(|subscription| subscription.replica_count() == 1)
        );
        assert_eq!(
            trades
                .iter()
                .map(|subscription| subscription.symbol())
                .collect::<BTreeSet<_>>(),
            configured_symbols
        );
    }

    #[test]
    fn stablecoin_and_strategy_index_consumers_merge_without_duplication() {
        let mut config = sample_config();
        config.risk.stablecoin_guards[0].symbol = "BTC-USDT".to_string();
        let plan = ChaosConnectivityPlan::resolve(&config, LiveMode::Demo).unwrap();
        let merged = plan
            .public_subscriptions()
            .iter()
            .find(|subscription| {
                subscription.channel() == PublicChannelPlan::IndexTickers
                    && subscription.symbol() == "BTC-USDT"
            })
            .unwrap();

        assert_eq!(
            merged
                .requirements()
                .iter()
                .map(RequirementUse::requirement_id)
                .collect::<BTreeSet<_>>(),
            BTreeSet::from([
                ConnectivityRequirementId::ChaosRefIndex,
                ConnectivityRequirementId::SafeStablecoin,
            ])
        );
        assert_eq!(
            merged.data_max_age_ms(),
            Some(config.risk.stablecoin_max_age_ms)
        );
    }

    #[test]
    fn every_account_including_spot_only_gets_account_wide_position_observation() {
        let mut config = sample_config();
        config.strategy.instruments[1].kind = reap_strategy::InstrumentKindConfig::Spot;
        config.accounts[0]
            .trade_modes
            .insert("BTC-USDT-SWAP".to_string(), crate::OkxTradeModeConfig::Cash);
        let plan = ChaosConnectivityPlan::resolve(&config, LiveMode::Observe).unwrap();
        let positions = plan.private_state_sessions()[0]
            .channels()
            .iter()
            .find(|binding| binding.channel() == PrivateChannelPlan::Positions)
            .unwrap();

        assert!(positions.requirements().iter().any(|requirement| {
            requirement.requirement_id() == ConnectivityRequirementId::SafeAccountPositions
        }));
        assert!(!positions.requirements().iter().any(|requirement| {
            requirement.requirement_id() == ConnectivityRequirementId::ChaosStatePositions
        }));
    }

    #[test]
    fn unused_accounts_are_observed_but_receive_no_execution_authority() {
        let mut config = sample_config();
        add_unused_account(&mut config);
        config.venue.enable_vip_fills_channel = true;
        let plan = ChaosConnectivityPlan::resolve(&config, LiveMode::Demo).unwrap();

        assert_eq!(
            plan.forbidden_order_checks()
                .iter()
                .filter(|check| check.account_id() == "unused")
                .count(),
            8
        );
        let unused_positions = plan
            .private_state_sessions()
            .iter()
            .find(|session| session.account_id() == "unused")
            .unwrap()
            .channels();
        assert_eq!(
            unused_positions
                .iter()
                .map(PrivateChannelBinding::channel)
                .collect::<Vec<_>>(),
            vec![PrivateChannelPlan::Positions]
        );
        assert_eq!(
            unused_positions[0]
                .requirements()
                .iter()
                .map(RequirementUse::requirement_id)
                .collect::<Vec<_>>(),
            vec![ConnectivityRequirementId::SafeAccountPositions]
        );
        assert!(
            plan.regular_mutations()
                .iter()
                .all(|mutation| mutation.account_id() != "unused")
        );
        assert!(
            plan.command_lanes()
                .iter()
                .all(|lane| lane.account_id() != "unused")
        );
    }

    #[test]
    fn reference_only_account_has_observation_but_no_regular_execution() {
        let mut config = sample_config();
        config.strategy.risk_groups[0].kind = reap_strategy::RiskGroupKindConfig::RefOnly;
        config.strategy.instruments[1].kind = reap_strategy::InstrumentKindConfig::Spot;
        config.accounts[0]
            .trade_modes
            .insert("BTC-USDT-SWAP".to_string(), crate::OkxTradeModeConfig::Cash);
        for instrument in &mut config.strategy.instruments {
            instrument.quote_profit_margin = 1.0;
            instrument.hedge_profit_margin = 1.0;
        }

        let plan = ChaosConnectivityPlan::resolve(&config, LiveMode::Demo).unwrap();

        assert_eq!(plan.forbidden_order_checks().len(), 8);
        assert!(plan.regular_mutations().is_empty());
        assert!(plan.command_lanes().is_empty());
        assert!(!plan.roles().iter().any(|role| matches!(
            role.role(),
            LiveConnectivityRole::RegularExecution | LiveConnectivityRole::RegularLiveSafety
        ) && !role.requirements().is_empty()));
    }

    #[test]
    fn modes_admit_only_their_closed_role_sets() {
        let config = sample_config();
        let validate = ChaosConnectivityPlan::resolve(&config, LiveMode::Validate).unwrap();
        let observe = ChaosConnectivityPlan::resolve(&config, LiveMode::Observe).unwrap();
        let demo = ChaosConnectivityPlan::resolve(&config, LiveMode::Demo).unwrap();

        assert!(validate.roles().is_empty());
        assert!(validate.regular_mutations().is_empty());
        assert!(validate.command_lanes().is_empty());
        assert!(observe.regular_mutations().is_empty());
        assert!(observe.command_lanes().is_empty());
        assert!(!observe.roles().iter().any(|role| matches!(
            role.role(),
            LiveConnectivityRole::RegularExecution | LiveConnectivityRole::RegularLiveSafety
        )));
        assert!(
            demo.roles()
                .iter()
                .any(|role| { role.role() == LiveConnectivityRole::RegularExecution })
        );
        assert!(demo.regular_mutations().iter().all(|mutation| matches!(
            mutation.operation(),
            RegularMutationOperation::RestCancelRegular
                | RegularMutationOperation::RestRegularCancelAllAfter
                | RegularMutationOperation::WebsocketCancelRegular
                | RegularMutationOperation::WebsocketPlaceRegular
        )));
        assert_eq!(
            demo.regular_mutations()
                .iter()
                .map(RegularMutationPlan::operation)
                .collect::<BTreeSet<_>>(),
            BTreeSet::from([
                RegularMutationOperation::RestCancelRegular,
                RegularMutationOperation::RestRegularCancelAllAfter,
                RegularMutationOperation::WebsocketCancelRegular,
                RegularMutationOperation::WebsocketPlaceRegular,
            ])
        );
    }

    #[test]
    fn production_order_entry_has_no_plan_composition() {
        let mut config = sample_config();
        config.venue.environment = TradingEnvironment::Production;

        assert!(matches!(
            ChaosConnectivityPlan::resolve(&config, LiveMode::Demo),
            Err(ChaosConnectivityPlanError::ProductionOrderEntryUnavailable)
        ));
    }

    #[test]
    fn maintenance_environment_is_part_of_the_canonical_plan() {
        let demo = ChaosConnectivityPlan::resolve(&sample_config(), LiveMode::Observe).unwrap();
        let mut production_config = sample_config();
        production_config.venue.environment = TradingEnvironment::Production;
        production_config.venue.public_ws_url = "wss://ws.okx.com:8443/ws/v5/public".to_string();
        production_config.venue.private_ws_url = "wss://ws.okx.com:8443/ws/v5/private".to_string();
        let production =
            ChaosConnectivityPlan::resolve(&production_config, LiveMode::Observe).unwrap();

        assert_eq!(demo.environment(), TradingEnvironment::Demo);
        assert_eq!(production.environment(), TradingEnvironment::Production);
        assert_eq!(
            production.maintenance_relevance().environment(),
            TradingEnvironment::Production
        );
        assert_ne!(demo.sha256(), production.sha256());
    }

    #[test]
    fn plan_items_map_to_allowed_registry_requirements() {
        let plan = ChaosConnectivityPlan::resolve(&sample_config(), LiveMode::Demo).unwrap();
        let mut rows = Vec::<(&CapabilitySurface, &[RequirementUse])>::new();
        for subscription in plan.public_subscriptions() {
            rows.push((subscription.channel_surface(), subscription.requirements()));
            for surface in subscription.session_surfaces() {
                rows.push((surface, subscription.requirements()));
            }
        }
        for session in plan.private_state_sessions() {
            for surface in session.session_surfaces() {
                rows.push((surface, session.requirements()));
            }
            for channel in session.channels() {
                rows.push((channel.surface(), channel.requirements()));
            }
        }
        for read in plan.authenticated_reads() {
            rows.push((read.surface(), read.requirements()));
        }
        for read in plan.public_safety_reads() {
            rows.push((read.surface(), read.requirements()));
        }
        for check in plan.forbidden_order_checks() {
            rows.push((check.surface(), check.requirements()));
        }
        for mutation in plan.regular_mutations() {
            rows.push((mutation.surface(), mutation.requirements()));
        }
        for lane in plan.command_lanes() {
            for surface in lane.session_surfaces() {
                rows.push((surface, lane.requirements()));
            }
        }
        rows.push((
            plan.maintenance_relevance().surface(),
            plan.maintenance_relevance().requirements(),
        ));

        for (surface, requirements) in rows {
            let capability_id = surface.capability_id();
            assert!(
                !requirements.is_empty(),
                "{capability_id} has no requirement"
            );
            let registration = okx_capability_registration(capability_id).unwrap();
            assert!(
                registration.allowed_in_live_plan,
                "{capability_id} is not live-allowed"
            );
            assert_eq!(
                surface.endpoint_or_channel(),
                registration.endpoint_or_channel
            );
            assert_eq!(surface.operation(), registration.operation);
            for requirement in requirements {
                assert!(
                    registration
                        .requirement_ids
                        .contains(&requirement.requirement_id().as_str()),
                    "{capability_id} lacks {}",
                    requirement.requirement_id().as_str()
                );
            }
        }
    }

    #[test]
    fn every_resolved_item_has_a_typed_requirement_and_consumer() {
        let plan = ChaosConnectivityPlan::resolve(&sample_config(), LiveMode::Demo).unwrap();
        let mut requirement_sets = Vec::<&[RequirementUse]>::new();
        requirement_sets.extend(
            plan.public_subscriptions()
                .iter()
                .map(PublicSubscriptionPlan::requirements),
        );
        requirement_sets.extend(plan.local_timers().iter().map(LocalTimerPlan::requirements));
        requirement_sets.extend(
            plan.private_state_sessions()
                .iter()
                .map(PrivateStateSessionPlan::requirements),
        );
        for session in plan.private_state_sessions() {
            requirement_sets.extend(
                session
                    .channels()
                    .iter()
                    .map(PrivateChannelBinding::requirements),
            );
        }
        requirement_sets.extend(
            plan.public_safety_reads()
                .iter()
                .map(PublicSafetyReadPlan::requirements),
        );
        requirement_sets.extend(
            plan.authenticated_reads()
                .iter()
                .map(AuthenticatedReadPlan::requirements),
        );
        requirement_sets.extend(
            plan.forbidden_order_checks()
                .iter()
                .map(ForbiddenOrderCheckPlan::requirements),
        );
        requirement_sets.push(plan.forbidden_proof_policy().requirements());
        requirement_sets.extend(
            plan.regular_mutations()
                .iter()
                .map(RegularMutationPlan::requirements),
        );
        requirement_sets.extend(
            plan.command_lanes()
                .iter()
                .map(OrderCommandLanePlan::requirements),
        );
        requirement_sets.extend(
            plan.roles()
                .iter()
                .map(LiveConnectivityRolePlan::requirements),
        );
        requirement_sets.push(plan.maintenance_relevance().requirements());

        assert!(!requirement_sets.is_empty());
        for requirements in requirement_sets {
            assert!(!requirements.is_empty());
            for requirement in requirements {
                assert!(!requirement.requirement_id().as_str().is_empty());
                let _closed_consumer = requirement.consumer();
            }
        }
    }

    #[test]
    fn forbidden_policy_is_bounded_and_covers_seven_algo_queries_plus_spread() {
        let plan = ChaosConnectivityPlan::resolve(&sample_config(), LiveMode::Observe).unwrap();
        let policy = plan.forbidden_proof_policy();

        assert_eq!(policy.max_age_ms(), 30_000);
        assert_eq!(policy.hard_max_age_ms(), 60_000);
        assert!(policy.scan_interval_ms() <= policy.max_age_ms() / 2);
        assert_eq!(
            plan.forbidden_order_checks().len(),
            plan.account_ids().len() * 8
        );
    }
}
