use reap_pm_core::{
    MAX_REQUIRED_SPENDERS, OkxReferenceHandle, PmAccountScope, PmInstrumentHandle, PmSpenderId,
};
use reap_pm_strategy::{PmModelInputRequirement, PmModelInputRequirements};
use thiserror::Error;

use crate::config::{
    PmAccountConnectivityConfig, PmConnectionRoute, PmConnectivityConfig,
    PmPublicConnectivityConfig,
};
use crate::requirements::{
    PmCapabilityLane, PmCapabilityRequirementId as Id, PmModelPlanRequirement,
    PmModelRequirementError, PmPlanOwner as Owner, PmReadinessDependency as Readiness,
    PmRequirementConsumer as Consumer, PmRequirementKey, PmRequirementOrigin as Origin,
    PmRequirementScope as Scope, PmRoleKind as Role, translate_model_requirements,
};

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum PmCompositionRoot {
    PublicCapture,
    ReadOnlyMonitor,
    Product,
}

/// The fixed Goal F mutation profile.
///
/// There is no order-type selector, live endpoint, cancel-all flag, or signer.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct PmFakeExecutionProfile {
    private: (),
}

impl PmFakeExecutionProfile {
    #[must_use]
    pub const fn goal_f() -> Self {
        Self { private: () }
    }

    #[must_use]
    pub const fn is_gtc_post_only(self) -> bool {
        true
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct PmPlanEntry {
    key: PmRequirementKey,
    origin: Origin,
    consumer: Consumer,
    owner: Owner,
    lane: PmCapabilityLane,
    readiness: Readiness,
    route: Option<PmConnectionRoute>,
}

impl PmPlanEntry {
    #[must_use]
    pub const fn key(self) -> PmRequirementKey {
        self.key
    }

    #[must_use]
    pub const fn origin(self) -> Origin {
        self.origin
    }

    #[must_use]
    pub const fn consumer(self) -> Consumer {
        self.consumer
    }

    #[must_use]
    pub const fn owner(self) -> Owner {
        self.owner
    }

    #[must_use]
    pub const fn role(self) -> Option<Role> {
        match self.owner {
            Owner::ConnectivityRole(role) => Some(role),
            Owner::QuoteSchedule => None,
        }
    }

    #[must_use]
    pub const fn lane(self) -> PmCapabilityLane {
        self.lane
    }

    #[must_use]
    pub const fn readiness(self) -> Readiness {
        self.readiness
    }

    #[must_use]
    pub const fn route(self) -> Option<PmConnectionRoute> {
        self.route
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct ConstructedRoleBinding {
    key: PmRequirementKey,
    owner: Owner,
    route: Option<PmConnectionRoute>,
}

impl ConstructedRoleBinding {
    #[must_use]
    pub const fn okx_public(reference: OkxReferenceHandle, route: PmConnectionRoute) -> Self {
        binding(
            Id::OkxReference,
            Scope::OkxReference(reference),
            Role::OkxPublicObservation,
            Some(route),
        )
    }

    #[must_use]
    pub const fn pm_public(instrument: PmInstrumentHandle, route: PmConnectionRoute) -> [Self; 4] {
        [
            binding(
                Id::MetadataLifecycle,
                Scope::Instrument(instrument),
                Role::PmPublicObservation,
                Some(route),
            ),
            binding(
                Id::MetadataClob,
                Scope::Instrument(instrument),
                Role::PmPublicObservation,
                Some(route),
            ),
            binding(
                Id::BookSnapshot,
                Scope::Instrument(instrument),
                Role::PmPublicObservation,
                Some(route),
            ),
            binding(
                Id::BookDelta,
                Scope::Instrument(instrument),
                Role::PmPublicObservation,
                Some(route),
            ),
        ]
    }

    #[must_use]
    pub const fn private_lifecycle(account: PmAccountScope, route: PmConnectionRoute) -> [Self; 2] {
        [
            binding(
                Id::PrivateOrder,
                Scope::Account(account),
                Role::PmPrivateLifecycle,
                Some(route),
            ),
            binding(
                Id::PrivateFill,
                Scope::Account(account),
                Role::PmPrivateLifecycle,
                Some(route),
            ),
        ]
    }

    #[must_use]
    pub const fn reconciliation(account: PmAccountScope, route: PmConnectionRoute) -> [Self; 3] {
        [
            binding(
                Id::ReconcileOpenOrders,
                Scope::Account(account),
                Role::PmOrderReconciliation,
                Some(route),
            ),
            binding(
                Id::ReconcileOrder,
                Scope::Account(account),
                Role::PmOrderReconciliation,
                Some(route),
            ),
            binding(
                Id::ReconcileFills,
                Scope::Account(account),
                Role::PmOrderReconciliation,
                Some(route),
            ),
        ]
    }

    pub fn account_snapshot(
        account: PmAccountScope,
        instrument: PmInstrumentHandle,
        spenders: &[PmSpenderId],
        route: PmConnectionRoute,
    ) -> Result<Vec<Self>, PmPlanError> {
        if spenders.len() > MAX_REQUIRED_SPENDERS {
            return Err(PmPlanError::TooManyAccountSnapshotSpenders);
        }
        let scope = Scope::AccountInstrument {
            account,
            instrument,
        };
        let mut bindings = Vec::with_capacity(3 + spenders.len());
        bindings.push(binding(
            Id::AccountCollateral,
            Scope::Account(account),
            Role::PmAccountPositionSnapshot,
            Some(route),
        ));
        bindings.push(binding(
            Id::AccountToken,
            scope,
            Role::PmAccountPositionSnapshot,
            Some(route),
        ));
        bindings.push(binding(
            Id::PositionSnapshot,
            scope,
            Role::PmAccountPositionSnapshot,
            Some(route),
        ));
        bindings.extend(spenders.iter().map(|spender| {
            binding(
                Id::AccountAllowance,
                Scope::Spender {
                    account,
                    spender: *spender,
                },
                Role::PmAccountPositionSnapshot,
                Some(route),
            )
        }));
        Ok(bindings)
    }

    #[must_use]
    pub const fn owned_execution(
        account: PmAccountScope,
        instrument: PmInstrumentHandle,
    ) -> [Self; 2] {
        let scope = Scope::AccountInstrument {
            account,
            instrument,
        };
        [
            binding(
                Id::FakePlaceGtcPostOnly,
                scope,
                Role::PmOwnedExecution,
                None,
            ),
            binding(Id::FakeCancelOwned, scope, Role::PmOwnedExecution, None),
        ]
    }

    #[must_use]
    pub const fn quote_schedule(instrument: PmInstrumentHandle) -> Self {
        Self {
            key: PmRequirementKey::quote_evaluation_timer(Scope::Instrument(instrument)),
            owner: Owner::QuoteSchedule,
            route: None,
        }
    }

    #[must_use]
    pub const fn key(self) -> PmRequirementKey {
        self.key
    }

    #[must_use]
    pub const fn owner(self) -> Owner {
        self.owner
    }

    #[must_use]
    pub const fn role(self) -> Option<Role> {
        match self.owner {
            Owner::ConnectivityRole(role) => Some(role),
            Owner::QuoteSchedule => None,
        }
    }

    #[must_use]
    pub const fn route(self) -> Option<PmConnectionRoute> {
        self.route
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PmConnectivityPlan {
    root: PmCompositionRoot,
    public_config: Option<PmPublicConnectivityConfig>,
    account_config: Option<PmAccountConnectivityConfig>,
    entries: Vec<PmPlanEntry>,
    model_requirements: Vec<PmModelPlanRequirement>,
    reached_roles: Vec<Role>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Error)]
pub enum PmPlanError {
    #[error(transparent)]
    ModelRequirement(#[from] PmModelRequirementError),
    #[error("capability plan contains a duplicate requirement key")]
    DuplicatePlanEntry,
    #[error("constructed role bindings contain a duplicate requirement key")]
    DuplicateRoleBinding,
    #[error("a required constructed role binding is missing")]
    MissingRoleBinding,
    #[error("a constructed role binding is not present in the plan")]
    UnexpectedRoleBinding,
    #[error("a constructed binding uses the wrong owner for its requirement")]
    OwnerBindingMismatch,
    #[error("a constructed binding uses the wrong source/connection route")]
    RouteBindingMismatch,
    #[error("account snapshot role exceeds the fixed required-spender bound")]
    TooManyAccountSnapshotSpenders,
}

impl PmConnectivityPlan {
    pub fn public_capture(config: PmPublicConnectivityConfig) -> Result<Self, PmPlanError> {
        Self::build(
            PmCompositionRoot::PublicCapture,
            Some(config),
            None,
            Vec::new(),
        )
    }

    pub fn read_only_monitor(config: PmAccountConnectivityConfig) -> Result<Self, PmPlanError> {
        Self::build(
            PmCompositionRoot::ReadOnlyMonitor,
            None,
            Some(config),
            Vec::new(),
        )
    }

    pub fn product(
        config: PmConnectivityConfig,
        model: PmModelInputRequirements,
        _profile: PmFakeExecutionProfile,
    ) -> Result<Self, PmPlanError> {
        let model_requirements = translate_model_requirements(
            config.public().okx_reference(),
            config.public().instrument(),
            model,
        )?;
        Self::build(
            PmCompositionRoot::Product,
            Some(config.public().clone()),
            Some(config.account().clone()),
            model_requirements,
        )
    }

    fn build(
        root: PmCompositionRoot,
        public_config: Option<PmPublicConnectivityConfig>,
        account_config: Option<PmAccountConnectivityConfig>,
        model_requirements: Vec<PmModelPlanRequirement>,
    ) -> Result<Self, PmPlanError> {
        let mut entries = Vec::new();
        if let Some(config) = &public_config {
            if root == PmCompositionRoot::PublicCapture {
                entries.extend(capture_public_entries(config));
            } else {
                entries.extend(product_public_entries(config, &model_requirements));
            }
        }
        if let Some(config) = &account_config {
            entries.extend(read_entries(config));
        }
        if root == PmCompositionRoot::Product {
            entries.extend(execution_entries(
                account_config
                    .as_ref()
                    .expect("product account config is present"),
            ));
        }
        entries.sort_by_key(|entry| entry.key);
        if entries.windows(2).any(|pair| pair[0].key == pair[1].key) {
            return Err(PmPlanError::DuplicatePlanEntry);
        }
        let reached_roles = exact_reached_roles(root).to_vec();
        Ok(Self {
            root,
            public_config,
            account_config,
            entries,
            model_requirements,
            reached_roles,
        })
    }

    #[must_use]
    pub const fn root(&self) -> PmCompositionRoot {
        self.root
    }

    #[must_use]
    pub fn public_config(&self) -> Option<&PmPublicConnectivityConfig> {
        self.public_config.as_ref()
    }

    #[must_use]
    pub fn account_config(&self) -> Option<&PmAccountConnectivityConfig> {
        self.account_config.as_ref()
    }

    #[must_use]
    pub fn entries(&self) -> &[PmPlanEntry] {
        &self.entries
    }

    #[must_use]
    pub fn model_requirements(&self) -> &[PmModelPlanRequirement] {
        &self.model_requirements
    }

    #[must_use]
    pub fn reached_roles(&self) -> &[Role] {
        &self.reached_roles
    }

    pub fn validate_bindings(
        &self,
        bindings: &[ConstructedRoleBinding],
    ) -> Result<(), PmPlanError> {
        if bindings.len() < self.entries.len() {
            return Err(PmPlanError::MissingRoleBinding);
        }
        if bindings.len() > self.entries.len() {
            return Err(PmPlanError::UnexpectedRoleBinding);
        }
        let mut actual = bindings.to_vec();
        actual.sort_by_key(|binding| binding.key);
        if actual.windows(2).any(|pair| pair[0].key == pair[1].key) {
            return Err(PmPlanError::DuplicateRoleBinding);
        }
        for (entry, binding) in self.entries.iter().zip(actual) {
            if entry.key != binding.key {
                return Err(if entry.key < binding.key {
                    PmPlanError::MissingRoleBinding
                } else {
                    PmPlanError::UnexpectedRoleBinding
                });
            }
            if entry.owner != binding.owner {
                return Err(PmPlanError::OwnerBindingMismatch);
            }
            if entry.route != binding.route {
                return Err(PmPlanError::RouteBindingMismatch);
            }
        }
        Ok(())
    }
}

const fn exact_reached_roles(root: PmCompositionRoot) -> &'static [Role] {
    match root {
        PmCompositionRoot::PublicCapture => {
            &[Role::OkxPublicObservation, Role::PmPublicObservation]
        }
        PmCompositionRoot::ReadOnlyMonitor => &[
            Role::PmPrivateLifecycle,
            Role::PmOrderReconciliation,
            Role::PmAccountPositionSnapshot,
        ],
        PmCompositionRoot::Product => &[
            Role::OkxPublicObservation,
            Role::PmPublicObservation,
            Role::PmPrivateLifecycle,
            Role::PmOrderReconciliation,
            Role::PmAccountPositionSnapshot,
            Role::PmOwnedExecution,
        ],
    }
}

fn entry(
    key: PmRequirementKey,
    origin: Origin,
    consumer: Consumer,
    role: Role,
    lane: PmCapabilityLane,
    readiness: Readiness,
    route: Option<PmConnectionRoute>,
) -> PmPlanEntry {
    PmPlanEntry {
        key,
        origin,
        consumer,
        owner: Owner::ConnectivityRole(role),
        lane,
        readiness,
        route,
    }
}

const fn binding(
    id: Id,
    scope: Scope,
    role: Role,
    route: Option<PmConnectionRoute>,
) -> ConstructedRoleBinding {
    ConstructedRoleBinding {
        key: PmRequirementKey::new(id, scope),
        owner: Owner::ConnectivityRole(role),
        route,
    }
}

fn capture_public_entries(config: &PmPublicConnectivityConfig) -> [PmPlanEntry; 5] {
    let instrument = Scope::Instrument(config.instrument());
    [
        entry(
            PmRequirementKey::new(
                Id::OkxReference,
                Scope::OkxReference(config.okx_reference()),
            ),
            Origin::ConfiguredPublicCapture,
            Consumer::QuoteModelReference,
            Role::OkxPublicObservation,
            PmCapabilityLane::Public,
            Readiness::OkxReference,
            Some(config.okx_route()),
        ),
        entry(
            PmRequirementKey::new(Id::MetadataLifecycle, instrument),
            Origin::ConfiguredPublicCapture,
            Consumer::MetadataReadiness,
            Role::PmPublicObservation,
            PmCapabilityLane::Public,
            Readiness::Metadata,
            Some(config.polymarket_route()),
        ),
        entry(
            PmRequirementKey::new(Id::MetadataClob, instrument),
            Origin::ConfiguredPublicCapture,
            Consumer::MetadataReadiness,
            Role::PmPublicObservation,
            PmCapabilityLane::Public,
            Readiness::Metadata,
            Some(config.polymarket_route()),
        ),
        entry(
            PmRequirementKey::new(Id::BookSnapshot, instrument),
            Origin::ConfiguredPublicCapture,
            Consumer::BookIntegrity,
            Role::PmPublicObservation,
            PmCapabilityLane::Public,
            Readiness::Book,
            Some(config.polymarket_route()),
        ),
        entry(
            PmRequirementKey::new(Id::BookDelta, instrument),
            Origin::ConfiguredPublicCapture,
            Consumer::BookIntegrity,
            Role::PmPublicObservation,
            PmCapabilityLane::Public,
            Readiness::Book,
            Some(config.polymarket_route()),
        ),
    ]
}

fn product_public_entries(
    config: &PmPublicConnectivityConfig,
    requirements: &[PmModelPlanRequirement],
) -> Vec<PmPlanEntry> {
    let mut entries = Vec::with_capacity(6);
    for requirement in requirements {
        if requirement.input() == PmModelInputRequirement::QuoteEvaluationTimer {
            entries.push(PmPlanEntry {
                key: PmRequirementKey::quote_evaluation_timer(requirement.scope()),
                origin: Origin::ModelPublicInput,
                consumer: requirement.consumer(),
                owner: Owner::QuoteSchedule,
                lane: requirement.lane(),
                readiness: requirement.readiness(),
                route: None,
            });
            continue;
        }
        let (ids, role): (&[Id], Role) = match requirement.input() {
            PmModelInputRequirement::OkxReference(_) => {
                (&[Id::OkxReference], Role::OkxPublicObservation)
            }
            PmModelInputRequirement::MarketMetadata(_) => (
                &[Id::MetadataLifecycle, Id::MetadataClob],
                Role::PmPublicObservation,
            ),
            PmModelInputRequirement::MarketBook(_) => (
                &[Id::BookSnapshot, Id::BookDelta],
                Role::PmPublicObservation,
            ),
            PmModelInputRequirement::QuoteEvaluationTimer => unreachable!("handled above"),
        };
        entries.extend(ids.iter().map(|id| {
            let route = match role {
                Role::OkxPublicObservation => config.okx_route(),
                Role::PmPublicObservation => config.polymarket_route(),
                Role::PmPrivateLifecycle
                | Role::PmOrderReconciliation
                | Role::PmAccountPositionSnapshot
                | Role::PmOwnedExecution => unreachable!("public input role"),
            };
            entry(
                PmRequirementKey::new(*id, requirement.scope()),
                Origin::ModelPublicInput,
                requirement.consumer(),
                role,
                requirement.lane(),
                requirement.readiness(),
                Some(route),
            )
        }));
    }
    entries
}

fn read_entries(config: &PmAccountConnectivityConfig) -> Vec<PmPlanEntry> {
    let account_scope = config.account_scope();
    let account = Scope::Account(account_scope);
    let account_instrument = Scope::AccountInstrument {
        account: account_scope,
        instrument: config.instrument(),
    };
    let mut entries = vec![
        entry(
            PmRequirementKey::new(Id::PrivateOrder, account),
            Origin::MandatorySafetyAndReadiness,
            Consumer::CanonicalOrderState,
            Role::PmPrivateLifecycle,
            PmCapabilityLane::Private,
            Readiness::PrivateLifecycle,
            Some(config.account_route()),
        ),
        entry(
            PmRequirementKey::new(Id::PrivateFill, account),
            Origin::MandatorySafetyAndReadiness,
            Consumer::FillAndPositionState,
            Role::PmPrivateLifecycle,
            PmCapabilityLane::Private,
            Readiness::PrivateLifecycle,
            Some(config.account_route()),
        ),
        entry(
            PmRequirementKey::new(Id::ReconcileOpenOrders, account),
            Origin::MandatorySafetyAndReadiness,
            Consumer::OrderReconciliation,
            Role::PmOrderReconciliation,
            PmCapabilityLane::Reconciliation,
            Readiness::OrderReconciliation,
            Some(config.account_route()),
        ),
        entry(
            PmRequirementKey::new(Id::ReconcileOrder, account),
            Origin::MandatorySafetyAndReadiness,
            Consumer::OrderReconciliation,
            Role::PmOrderReconciliation,
            PmCapabilityLane::Reconciliation,
            Readiness::OrderReconciliation,
            Some(config.account_route()),
        ),
        entry(
            PmRequirementKey::new(Id::ReconcileFills, account),
            Origin::MandatorySafetyAndReadiness,
            Consumer::OrderReconciliation,
            Role::PmOrderReconciliation,
            PmCapabilityLane::Reconciliation,
            Readiness::OrderReconciliation,
            Some(config.account_route()),
        ),
        entry(
            PmRequirementKey::new(Id::AccountCollateral, account),
            Origin::MandatorySafetyAndReadiness,
            Consumer::PositionReadiness,
            Role::PmAccountPositionSnapshot,
            PmCapabilityLane::Reconciliation,
            Readiness::Collateral,
            Some(config.account_route()),
        ),
        entry(
            PmRequirementKey::new(Id::AccountToken, account_instrument),
            Origin::MandatorySafetyAndReadiness,
            Consumer::PositionReadiness,
            Role::PmAccountPositionSnapshot,
            PmCapabilityLane::Reconciliation,
            Readiness::TokenInventory,
            Some(config.account_route()),
        ),
        entry(
            PmRequirementKey::new(Id::PositionSnapshot, account_instrument),
            Origin::MandatorySafetyAndReadiness,
            Consumer::PositionReadiness,
            Role::PmAccountPositionSnapshot,
            PmCapabilityLane::Reconciliation,
            Readiness::Position,
            Some(config.account_route()),
        ),
    ];
    entries.extend(config.required_spenders().iter().map(|spender| {
        entry(
            PmRequirementKey::new(
                Id::AccountAllowance,
                Scope::Spender {
                    account: account_scope,
                    spender: *spender,
                },
            ),
            Origin::MandatorySafetyAndReadiness,
            Consumer::AllowanceReadiness,
            Role::PmAccountPositionSnapshot,
            PmCapabilityLane::Reconciliation,
            Readiness::Allowance,
            Some(config.account_route()),
        )
    }));
    entries
}

fn execution_entries(config: &PmAccountConnectivityConfig) -> [PmPlanEntry; 2] {
    let scope = Scope::AccountInstrument {
        account: config.account_scope(),
        instrument: config.instrument(),
    };
    [
        entry(
            PmRequirementKey::new(Id::FakePlaceGtcPostOnly, scope),
            Origin::FixedFakeExecutionProfile,
            Consumer::FakeEffectWorker,
            Role::PmOwnedExecution,
            PmCapabilityLane::FakeEffect,
            Readiness::OwnedExecution,
            None,
        ),
        entry(
            PmRequirementKey::new(Id::FakeCancelOwned, scope),
            Origin::FixedFakeExecutionProfile,
            Consumer::OwnedCancellation,
            Role::PmOwnedExecution,
            PmCapabilityLane::FakeEffect,
            Readiness::OwnedExecution,
            None,
        ),
    ]
}
