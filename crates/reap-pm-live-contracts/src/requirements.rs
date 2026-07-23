use std::cmp::Ordering;

use reap_pm_core::{
    OkxReferenceHandle, PmAccountScope, PmAssetId, PmInstrumentHandle, PmInstrumentId, PmSpenderId,
};
use reap_pm_strategy::{PmModelInputRequirement, PmModelInputRequirements};
use thiserror::Error;

/// Stable reached endpoint/channel/purpose kinds.
///
/// Public PM trade is intentionally absent because the Goal F fixture model
/// has no consumer for it.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub enum PmCapabilityRequirementId {
    OkxReference,
    MetadataLifecycle,
    MetadataClob,
    BookSnapshot,
    BookDelta,
    PrivateOrder,
    PrivateFill,
    ReconcileOpenOrders,
    ReconcileOrder,
    ReconcileFills,
    AccountCollateral,
    AccountToken,
    AccountAllowance,
    PositionSnapshot,
    FakePlaceGtcPostOnly,
    FakeCancelOwned,
}

impl PmCapabilityRequirementId {
    pub const ALL: [Self; 16] = [
        Self::OkxReference,
        Self::MetadataLifecycle,
        Self::MetadataClob,
        Self::BookSnapshot,
        Self::BookDelta,
        Self::PrivateOrder,
        Self::PrivateFill,
        Self::ReconcileOpenOrders,
        Self::ReconcileOrder,
        Self::ReconcileFills,
        Self::AccountCollateral,
        Self::AccountToken,
        Self::AccountAllowance,
        Self::PositionSnapshot,
        Self::FakePlaceGtcPostOnly,
        Self::FakeCancelOwned,
    ];

    #[must_use]
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::OkxReference => "PM-OKX-REF",
            Self::MetadataLifecycle => "PM-META-LIFECYCLE",
            Self::MetadataClob => "PM-META-CLOB",
            Self::BookSnapshot => "PM-MD-BOOK-SNAPSHOT",
            Self::BookDelta => "PM-MD-BOOK-DELTA",
            Self::PrivateOrder => "PM-PRIVATE-ORDER",
            Self::PrivateFill => "PM-PRIVATE-FILL",
            Self::ReconcileOpenOrders => "PM-RECON-OPEN",
            Self::ReconcileOrder => "PM-RECON-ORDER",
            Self::ReconcileFills => "PM-RECON-FILLS",
            Self::AccountCollateral => "PM-ACCOUNT-COLLATERAL",
            Self::AccountToken => "PM-ACCOUNT-TOKEN",
            Self::AccountAllowance => "PM-ACCOUNT-ALLOWANCE",
            Self::PositionSnapshot => "PM-POSITION-SNAPSHOT",
            Self::FakePlaceGtcPostOnly => "PM-FAKE-PLACE-GTC-PO",
            Self::FakeCancelOwned => "PM-FAKE-CANCEL-OWNED",
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum PmRequirementScope {
    OkxReference(OkxReferenceHandle),
    Instrument(PmInstrumentHandle),
    AccountAsset {
        account: PmAccountScope,
        asset: PmAssetId,
    },
    AccountInstrument {
        account: PmAccountScope,
        instrument: PmInstrumentHandle,
        instrument_id: PmInstrumentId,
    },
    Spender {
        account: PmAccountScope,
        spender: PmSpenderId,
    },
}

impl PmRequirementScope {
    const fn rank(self) -> u8 {
        match self {
            Self::OkxReference(_) => 0,
            Self::Instrument(_) => 1,
            Self::AccountAsset { .. } => 2,
            Self::AccountInstrument { .. } => 3,
            Self::Spender { .. } => 4,
        }
    }
}

impl Ord for PmRequirementScope {
    fn cmp(&self, other: &Self) -> Ordering {
        let rank = self.rank().cmp(&other.rank());
        if rank != Ordering::Equal {
            return rank;
        }
        match (*self, *other) {
            (Self::OkxReference(left), Self::OkxReference(right)) => left.cmp(&right),
            (Self::Instrument(left), Self::Instrument(right)) => left.cmp(&right),
            (
                Self::AccountAsset {
                    account: left_account,
                    asset: left_asset,
                },
                Self::AccountAsset {
                    account: right_account,
                    asset: right_asset,
                },
            ) => (account_scope_key(left_account), left_asset)
                .cmp(&(account_scope_key(right_account), right_asset)),
            (
                Self::AccountInstrument {
                    account: left_account,
                    instrument: left_instrument,
                    instrument_id: left_instrument_id,
                },
                Self::AccountInstrument {
                    account: right_account,
                    instrument: right_instrument,
                    instrument_id: right_instrument_id,
                },
            ) => (
                account_scope_key(left_account),
                left_instrument,
                left_instrument_id,
            )
                .cmp(&(
                    account_scope_key(right_account),
                    right_instrument,
                    right_instrument_id,
                )),
            (
                Self::Spender {
                    account: left_account,
                    spender: left_spender,
                },
                Self::Spender {
                    account: right_account,
                    spender: right_spender,
                },
            ) => (account_scope_key(left_account), left_spender.requirement()).cmp(&(
                account_scope_key(right_account),
                right_spender.requirement(),
            )),
            _ => Ordering::Equal,
        }
    }
}

fn account_scope_key(
    scope: PmAccountScope,
) -> (
    reap_pm_core::PmEnvironmentId,
    reap_pm_core::PmChainId,
    reap_pm_core::PmSignerId,
    reap_pm_core::PmFunderId,
    reap_pm_core::PmAccountHandle,
) {
    (
        scope.environment(),
        scope.chain(),
        scope.signer(),
        scope.funder(),
        scope.handle(),
    )
}

impl PartialOrd for PmRequirementScope {
    fn partial_cmp(&self, other: &Self) -> Option<Ordering> {
        Some(self.cmp(other))
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub enum PmPlanRequirementId {
    Connectivity(PmCapabilityRequirementId),
    QuoteEvaluationTimer,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct PmRequirementKey {
    id: PmPlanRequirementId,
    scope: PmRequirementScope,
}

impl PmRequirementKey {
    #[must_use]
    pub const fn id(self) -> PmPlanRequirementId {
        self.id
    }

    #[must_use]
    pub const fn connectivity_id(self) -> Option<PmCapabilityRequirementId> {
        match self.id {
            PmPlanRequirementId::Connectivity(id) => Some(id),
            PmPlanRequirementId::QuoteEvaluationTimer => None,
        }
    }

    #[must_use]
    pub const fn scope(self) -> PmRequirementScope {
        self.scope
    }

    pub(crate) const fn new(id: PmCapabilityRequirementId, scope: PmRequirementScope) -> Self {
        Self {
            id: PmPlanRequirementId::Connectivity(id),
            scope,
        }
    }

    pub(crate) const fn quote_evaluation_timer(scope: PmRequirementScope) -> Self {
        Self {
            id: PmPlanRequirementId::QuoteEvaluationTimer,
            scope,
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub enum PmRequirementOrigin {
    ModelPublicInput,
    ConfiguredPublicCapture,
    MandatorySafetyAndReadiness,
    FixedFakeExecutionProfile,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub enum PmRequirementConsumer {
    QuoteModelReference,
    MetadataReadiness,
    BookIntegrity,
    CanonicalOrderState,
    FillAndPositionState,
    OrderReconciliation,
    PositionReadiness,
    AllowanceReadiness,
    FakeEffectWorker,
    OwnedCancellation,
    QuoteEvaluationSchedule,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub enum PmRoleKind {
    OkxPublicObservation,
    PmPublicObservation,
    PmPrivateLifecycle,
    PmOrderReconciliation,
    PmAccountPositionSnapshot,
    PmOwnedExecution,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub enum PmPlanOwner {
    ConnectivityRole(PmRoleKind),
    QuoteSchedule,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub enum PmCapabilityLane {
    Critical,
    Persistence,
    Private,
    Scheduled,
    Public,
    Reconciliation,
    Telemetry,
    ReconciliationRequest,
    Capture,
    Journal,
    FakeEffect,
}

impl PmCapabilityLane {
    pub const ALL: [Self; 11] = [
        Self::Critical,
        Self::Persistence,
        Self::Private,
        Self::Scheduled,
        Self::Public,
        Self::Reconciliation,
        Self::Telemetry,
        Self::ReconciliationRequest,
        Self::Capture,
        Self::Journal,
        Self::FakeEffect,
    ];
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub enum PmReadinessDependency {
    OkxReference,
    Metadata,
    Book,
    PrivateLifecycle,
    OrderReconciliation,
    Collateral,
    TokenInventory,
    Allowance,
    Position,
    OwnedExecution,
    QuoteEvaluationClock,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct PmModelPlanRequirement {
    input: PmModelInputRequirement,
    scope: PmRequirementScope,
    lane: PmCapabilityLane,
    consumer: PmRequirementConsumer,
    readiness: PmReadinessDependency,
}

impl PmModelPlanRequirement {
    #[must_use]
    pub const fn input(self) -> PmModelInputRequirement {
        self.input
    }

    #[must_use]
    pub const fn scope(self) -> PmRequirementScope {
        self.scope
    }

    #[must_use]
    pub const fn lane(self) -> PmCapabilityLane {
        self.lane
    }

    #[must_use]
    pub const fn consumer(self) -> PmRequirementConsumer {
        self.consumer
    }

    #[must_use]
    pub const fn readiness(self) -> PmReadinessDependency {
        self.readiness
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Error)]
pub enum PmModelRequirementError {
    #[error("model OKX reference does not match the checked reference mapping")]
    ReferenceMismatch,
    #[error("model PM instrument does not match the checked reference mapping")]
    InstrumentMismatch,
    #[error("model input requirements contain a duplicate category")]
    DuplicateCategory,
    #[error("model input requirements omit a required category")]
    MissingCategory,
}

pub(crate) fn translate_model_requirements(
    expected_reference: OkxReferenceHandle,
    expected_instrument: PmInstrumentHandle,
    model: PmModelInputRequirements,
) -> Result<Vec<PmModelPlanRequirement>, PmModelRequirementError> {
    let mut translated = Vec::with_capacity(4);
    let mut seen = [false; 4];
    for requirement in model.requirements() {
        let (category, scope, lane, consumer, readiness) = match requirement {
            PmModelInputRequirement::OkxReference(reference) => {
                if reference != expected_reference {
                    return Err(PmModelRequirementError::ReferenceMismatch);
                }
                (
                    0,
                    PmRequirementScope::OkxReference(reference),
                    PmCapabilityLane::Public,
                    PmRequirementConsumer::QuoteModelReference,
                    PmReadinessDependency::OkxReference,
                )
            }
            PmModelInputRequirement::MarketMetadata(instrument) => {
                validate_instrument(instrument, expected_instrument)?;
                (
                    1,
                    PmRequirementScope::Instrument(instrument),
                    PmCapabilityLane::Public,
                    PmRequirementConsumer::MetadataReadiness,
                    PmReadinessDependency::Metadata,
                )
            }
            PmModelInputRequirement::MarketBook(instrument) => {
                validate_instrument(instrument, expected_instrument)?;
                (
                    2,
                    PmRequirementScope::Instrument(instrument),
                    PmCapabilityLane::Public,
                    PmRequirementConsumer::BookIntegrity,
                    PmReadinessDependency::Book,
                )
            }
            PmModelInputRequirement::QuoteEvaluationTimer => (
                3,
                PmRequirementScope::Instrument(expected_instrument),
                PmCapabilityLane::Scheduled,
                PmRequirementConsumer::QuoteEvaluationSchedule,
                PmReadinessDependency::QuoteEvaluationClock,
            ),
        };
        if std::mem::replace(&mut seen[category], true) {
            return Err(PmModelRequirementError::DuplicateCategory);
        }
        translated.push(PmModelPlanRequirement {
            input: requirement,
            scope,
            lane,
            consumer,
            readiness,
        });
    }
    if seen.into_iter().all(|present| present) {
        Ok(translated)
    } else {
        Err(PmModelRequirementError::MissingCategory)
    }
}

fn validate_instrument(
    actual: PmInstrumentHandle,
    expected: PmInstrumentHandle,
) -> Result<(), PmModelRequirementError> {
    if actual == expected {
        Ok(())
    } else {
        Err(PmModelRequirementError::InstrumentMismatch)
    }
}
