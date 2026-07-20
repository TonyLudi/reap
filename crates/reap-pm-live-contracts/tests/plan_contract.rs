use reap_pm_core::{
    EvmAddress, OkxReferenceHandle, PmAccountHandle, PmAccountScope, PmAssetId, PmChainId,
    PmConnectionId, PmEnvironmentId, PmFunderId, PmInstrumentHandle, PmMarketHandle,
    PmProductSource, PmReferenceMapping, PmSignerId, PmSourceHandle, PmSpenderDomain, PmSpenderId,
    PmSpenderRequirement, PmTokenHandle, PmTokenId, U256,
};
use reap_pm_live_contracts::{
    ConstructedRoleBinding, PmAccountConnectivityConfig, PmCapabilityLane,
    PmCapabilityRequirementId, PmCompositionRoot, PmConnectionRoute, PmConnectivityConfig,
    PmConnectivityConfigError, PmConnectivityPlan, PmFakeExecutionProfile, PmPlanError,
    PmPlanOwner, PmPlanRequirementId, PmPublicConnectivityConfig, PmReadinessDependency,
    PmRequirementConsumer, PmRequirementOrigin, PmRequirementScope, PmRoleKind,
};
use reap_pm_strategy::PmModelInputRequirements;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
struct ProductPlanRow {
    requirement: PmPlanRequirementId,
    scope: PmRequirementScope,
    origin: PmRequirementOrigin,
    consumer: PmRequirementConsumer,
    owner: PmPlanOwner,
    lane: PmCapabilityLane,
    readiness: PmReadinessDependency,
    route: Option<PmConnectionRoute>,
}

fn instrument() -> PmInstrumentHandle {
    PmInstrumentHandle::new(
        PmMarketHandle::from_ordinal(1),
        PmTokenHandle::from_ordinal(2),
    )
}

fn reference() -> OkxReferenceHandle {
    OkxReferenceHandle::from_ordinal(3)
}

fn account_scope() -> PmAccountScope {
    let eoa = EvmAddress::from_bytes([4; 20]).unwrap();
    PmAccountScope::new(
        PmEnvironmentId::new("fixture").unwrap(),
        PmChainId::new(137).unwrap(),
        PmSignerId::new(eoa),
        PmFunderId::new(eoa),
        PmAccountHandle::from_ordinal(4),
    )
}

fn route(source: PmProductSource, name: &str) -> PmConnectionRoute {
    PmConnectionRoute::new(source, PmConnectionId::new(name).unwrap())
}

fn reference_mapping() -> PmReferenceMapping {
    let mut references = [None; 16];
    references[0] = Some(reference());
    PmReferenceMapping::new(instrument(), references, 1).unwrap()
}

fn required_spenders(account: PmAccountScope) -> Vec<PmSpenderId> {
    vec![
        PmSpenderId::new(
            account.handle(),
            PmSpenderRequirement::new(
                PmChainId::new(137).unwrap(),
                EvmAddress::from_bytes([2; 20]).unwrap(),
                PmSpenderDomain::Standard,
                PmAssetId::collateral(EvmAddress::from_bytes([1; 20]).unwrap()),
            ),
        ),
        PmSpenderId::new(
            account.handle(),
            PmSpenderRequirement::new(
                PmChainId::new(137).unwrap(),
                EvmAddress::from_bytes([2; 20]).unwrap(),
                PmSpenderDomain::Standard,
                PmAssetId::outcome(
                    EvmAddress::from_bytes([5; 20]).unwrap(),
                    PmTokenId::new(U256::from_u64(11)).unwrap(),
                ),
            ),
        ),
    ]
}

fn fixture() -> (
    PmConnectivityConfig,
    PmModelInputRequirements,
    PmFakeExecutionProfile,
) {
    let account = account_scope();
    let public = PmPublicConnectivityConfig::new(
        reference_mapping(),
        route(
            PmProductSource::okx_reference(PmSourceHandle::from_ordinal(1), reference()),
            "okx-public",
        ),
        route(
            PmProductSource::polymarket_market(
                PmSourceHandle::from_ordinal(2),
                instrument().token(),
            ),
            "pm-public",
        ),
    )
    .unwrap();
    let account = PmAccountConnectivityConfig::new(
        instrument(),
        account,
        route(
            PmProductSource::polymarket_account(PmSourceHandle::from_ordinal(3), account.handle()),
            "pm-account",
        ),
        required_spenders(account),
    )
    .unwrap();
    (
        PmConnectivityConfig::new(public, account).unwrap(),
        PmModelInputRequirements::new(reference(), instrument()),
        PmFakeExecutionProfile::goal_f(),
    )
}

fn self_attested_bindings_for_negative_validation(
    config: &PmConnectivityConfig,
) -> Vec<ConstructedRoleBinding> {
    let public = config.public();
    let account = config.account();
    let mut bindings = vec![ConstructedRoleBinding::okx_public(
        public.okx_reference(),
        public.okx_route(),
    )];
    bindings.extend(ConstructedRoleBinding::pm_public(
        public.instrument(),
        public.polymarket_route(),
    ));
    bindings.extend(ConstructedRoleBinding::private_lifecycle(
        account.account_scope(),
        account.account_route(),
    ));
    bindings.extend(ConstructedRoleBinding::reconciliation(
        account.account_scope(),
        account.account_route(),
    ));
    bindings.extend(
        ConstructedRoleBinding::account_snapshot(
            account.account_scope(),
            account.instrument(),
            account.required_spenders(),
            account.account_route(),
        )
        .unwrap(),
    );
    bindings.extend(ConstructedRoleBinding::owned_execution(
        account.account_scope(),
        account.instrument(),
    ));
    bindings.push(ConstructedRoleBinding::quote_schedule(public.instrument()));
    bindings
}

#[test]
fn stable_endpoint_inventory_is_exact_and_trade_is_absent() {
    let ids = PmCapabilityRequirementId::ALL
        .into_iter()
        .map(PmCapabilityRequirementId::as_str)
        .collect::<Vec<_>>();
    assert_eq!(ids.len(), 16);
    assert_eq!(
        ids,
        vec![
            "PM-OKX-REF",
            "PM-META-LIFECYCLE",
            "PM-META-CLOB",
            "PM-MD-BOOK-SNAPSHOT",
            "PM-MD-BOOK-DELTA",
            "PM-PRIVATE-ORDER",
            "PM-PRIVATE-FILL",
            "PM-RECON-OPEN",
            "PM-RECON-ORDER",
            "PM-RECON-FILLS",
            "PM-ACCOUNT-COLLATERAL",
            "PM-ACCOUNT-TOKEN",
            "PM-ACCOUNT-ALLOWANCE",
            "PM-POSITION-SNAPSHOT",
            "PM-FAKE-PLACE-GTC-PO",
            "PM-FAKE-CANCEL-OWNED",
        ]
    );
    assert!(!ids.iter().any(|id| id.contains("TRADE")));
}

#[test]
fn roots_have_exact_role_reach_variable_spender_entries_and_product_timer() {
    let (config, model, profile) = fixture();
    let public = PmConnectivityPlan::public_capture(config.public().clone()).unwrap();
    let monitor = PmConnectivityPlan::read_only_monitor(config.account().clone()).unwrap();
    let product = PmConnectivityPlan::product(config, model, profile).unwrap();

    assert_eq!(public.root(), PmCompositionRoot::PublicCapture);
    assert_eq!(monitor.root(), PmCompositionRoot::ReadOnlyMonitor);
    assert_eq!(product.root(), PmCompositionRoot::Product);
    assert_eq!(public.entries().len(), 5);
    let spender_count = monitor.account_config().unwrap().required_spenders().len();
    assert_eq!(spender_count, 2);
    assert_eq!(monitor.entries().len(), 8 + spender_count);
    assert_eq!(product.entries().len(), 16 + spender_count);
    assert_eq!(public.reached_roles().len(), 2);
    assert_eq!(monitor.reached_roles().len(), 3);
    assert_eq!(product.reached_roles().len(), 6);
    assert_eq!(product.model_requirements().len(), 4);

    let timer = product
        .entries()
        .iter()
        .find(|entry| entry.key().id() == PmPlanRequirementId::QuoteEvaluationTimer)
        .unwrap();
    assert_eq!(timer.owner(), PmPlanOwner::QuoteSchedule);
    assert_eq!(timer.role(), None);
    assert_eq!(timer.lane(), PmCapabilityLane::Scheduled);
    assert_eq!(timer.route(), None);

    let endpoint_count = product
        .entries()
        .iter()
        .filter(|entry| entry.key().connectivity_id().is_some())
        .count();
    assert_eq!(endpoint_count, 15 + spender_count);
    assert_eq!(
        product
            .entries()
            .iter()
            .filter(|entry| {
                entry.key().connectivity_id() == Some(PmCapabilityRequirementId::AccountAllowance)
            })
            .count(),
        spender_count
    );
}

#[test]
fn product_plan_matches_the_independent_full_entry_table() {
    use PmCapabilityLane::{FakeEffect, Private, Public, Reconciliation, Scheduled};
    use PmCapabilityRequirementId::{
        AccountAllowance, AccountCollateral, AccountToken, BookDelta, BookSnapshot,
        FakeCancelOwned, FakePlaceGtcPostOnly, MetadataClob, MetadataLifecycle, OkxReference,
        PositionSnapshot, PrivateFill, PrivateOrder, ReconcileFills, ReconcileOpenOrders,
        ReconcileOrder,
    };
    use PmPlanOwner::{ConnectivityRole, QuoteSchedule};
    use PmReadinessDependency::{
        Allowance, Book, Collateral, Metadata, OkxReference as OkxReferenceReady, OwnedExecution,
        Position, PrivateLifecycle, QuoteEvaluationClock, TokenInventory,
    };
    use PmRequirementConsumer::{
        AllowanceReadiness, BookIntegrity, CanonicalOrderState, FakeEffectWorker,
        FillAndPositionState, MetadataReadiness, OrderReconciliation, OwnedCancellation,
        PositionReadiness, QuoteEvaluationSchedule, QuoteModelReference,
    };
    use PmRequirementOrigin::{
        FixedFakeExecutionProfile, MandatorySafetyAndReadiness, ModelPublicInput,
    };
    use PmRoleKind::{
        OkxPublicObservation, PmAccountPositionSnapshot, PmOrderReconciliation, PmOwnedExecution,
        PmPrivateLifecycle, PmPublicObservation,
    };

    let (config, model, profile) = fixture();
    let account = config.account().account_scope();
    let instrument = config.public().instrument();
    let spenders = config.account().required_spenders();
    assert_eq!(spenders.len(), 2);
    let okx_route = config.public().okx_route();
    let public_route = config.public().polymarket_route();
    let account_route = config.account().account_route();
    let account_instrument = PmRequirementScope::AccountInstrument {
        account,
        instrument,
    };

    let expected = [
        ProductPlanRow {
            requirement: PmPlanRequirementId::Connectivity(OkxReference),
            scope: PmRequirementScope::OkxReference(reference()),
            origin: ModelPublicInput,
            consumer: QuoteModelReference,
            owner: ConnectivityRole(OkxPublicObservation),
            lane: Public,
            readiness: OkxReferenceReady,
            route: Some(okx_route),
        },
        ProductPlanRow {
            requirement: PmPlanRequirementId::Connectivity(MetadataLifecycle),
            scope: PmRequirementScope::Instrument(instrument),
            origin: ModelPublicInput,
            consumer: MetadataReadiness,
            owner: ConnectivityRole(PmPublicObservation),
            lane: Public,
            readiness: Metadata,
            route: Some(public_route),
        },
        ProductPlanRow {
            requirement: PmPlanRequirementId::Connectivity(MetadataClob),
            scope: PmRequirementScope::Instrument(instrument),
            origin: ModelPublicInput,
            consumer: MetadataReadiness,
            owner: ConnectivityRole(PmPublicObservation),
            lane: Public,
            readiness: Metadata,
            route: Some(public_route),
        },
        ProductPlanRow {
            requirement: PmPlanRequirementId::Connectivity(BookSnapshot),
            scope: PmRequirementScope::Instrument(instrument),
            origin: ModelPublicInput,
            consumer: BookIntegrity,
            owner: ConnectivityRole(PmPublicObservation),
            lane: Public,
            readiness: Book,
            route: Some(public_route),
        },
        ProductPlanRow {
            requirement: PmPlanRequirementId::Connectivity(BookDelta),
            scope: PmRequirementScope::Instrument(instrument),
            origin: ModelPublicInput,
            consumer: BookIntegrity,
            owner: ConnectivityRole(PmPublicObservation),
            lane: Public,
            readiness: Book,
            route: Some(public_route),
        },
        ProductPlanRow {
            requirement: PmPlanRequirementId::Connectivity(PrivateOrder),
            scope: PmRequirementScope::Account(account),
            origin: MandatorySafetyAndReadiness,
            consumer: CanonicalOrderState,
            owner: ConnectivityRole(PmPrivateLifecycle),
            lane: Private,
            readiness: PrivateLifecycle,
            route: Some(account_route),
        },
        ProductPlanRow {
            requirement: PmPlanRequirementId::Connectivity(PrivateFill),
            scope: PmRequirementScope::Account(account),
            origin: MandatorySafetyAndReadiness,
            consumer: FillAndPositionState,
            owner: ConnectivityRole(PmPrivateLifecycle),
            lane: Private,
            readiness: PrivateLifecycle,
            route: Some(account_route),
        },
        ProductPlanRow {
            requirement: PmPlanRequirementId::Connectivity(ReconcileOpenOrders),
            scope: PmRequirementScope::Account(account),
            origin: MandatorySafetyAndReadiness,
            consumer: OrderReconciliation,
            owner: ConnectivityRole(PmOrderReconciliation),
            lane: Reconciliation,
            readiness: PmReadinessDependency::OrderReconciliation,
            route: Some(account_route),
        },
        ProductPlanRow {
            requirement: PmPlanRequirementId::Connectivity(ReconcileOrder),
            scope: PmRequirementScope::Account(account),
            origin: MandatorySafetyAndReadiness,
            consumer: OrderReconciliation,
            owner: ConnectivityRole(PmOrderReconciliation),
            lane: Reconciliation,
            readiness: PmReadinessDependency::OrderReconciliation,
            route: Some(account_route),
        },
        ProductPlanRow {
            requirement: PmPlanRequirementId::Connectivity(ReconcileFills),
            scope: PmRequirementScope::Account(account),
            origin: MandatorySafetyAndReadiness,
            consumer: OrderReconciliation,
            owner: ConnectivityRole(PmOrderReconciliation),
            lane: Reconciliation,
            readiness: PmReadinessDependency::OrderReconciliation,
            route: Some(account_route),
        },
        ProductPlanRow {
            requirement: PmPlanRequirementId::Connectivity(AccountCollateral),
            scope: PmRequirementScope::Account(account),
            origin: MandatorySafetyAndReadiness,
            consumer: PositionReadiness,
            owner: ConnectivityRole(PmAccountPositionSnapshot),
            lane: Reconciliation,
            readiness: Collateral,
            route: Some(account_route),
        },
        ProductPlanRow {
            requirement: PmPlanRequirementId::Connectivity(AccountToken),
            scope: account_instrument,
            origin: MandatorySafetyAndReadiness,
            consumer: PositionReadiness,
            owner: ConnectivityRole(PmAccountPositionSnapshot),
            lane: Reconciliation,
            readiness: TokenInventory,
            route: Some(account_route),
        },
        ProductPlanRow {
            requirement: PmPlanRequirementId::Connectivity(AccountAllowance),
            scope: PmRequirementScope::Spender {
                account,
                spender: spenders[0],
            },
            origin: MandatorySafetyAndReadiness,
            consumer: AllowanceReadiness,
            owner: ConnectivityRole(PmAccountPositionSnapshot),
            lane: Reconciliation,
            readiness: Allowance,
            route: Some(account_route),
        },
        ProductPlanRow {
            requirement: PmPlanRequirementId::Connectivity(AccountAllowance),
            scope: PmRequirementScope::Spender {
                account,
                spender: spenders[1],
            },
            origin: MandatorySafetyAndReadiness,
            consumer: AllowanceReadiness,
            owner: ConnectivityRole(PmAccountPositionSnapshot),
            lane: Reconciliation,
            readiness: Allowance,
            route: Some(account_route),
        },
        ProductPlanRow {
            requirement: PmPlanRequirementId::Connectivity(PositionSnapshot),
            scope: account_instrument,
            origin: MandatorySafetyAndReadiness,
            consumer: PositionReadiness,
            owner: ConnectivityRole(PmAccountPositionSnapshot),
            lane: Reconciliation,
            readiness: Position,
            route: Some(account_route),
        },
        ProductPlanRow {
            requirement: PmPlanRequirementId::Connectivity(FakePlaceGtcPostOnly),
            scope: account_instrument,
            origin: FixedFakeExecutionProfile,
            consumer: FakeEffectWorker,
            owner: ConnectivityRole(PmOwnedExecution),
            lane: FakeEffect,
            readiness: OwnedExecution,
            route: None,
        },
        ProductPlanRow {
            requirement: PmPlanRequirementId::Connectivity(FakeCancelOwned),
            scope: account_instrument,
            origin: FixedFakeExecutionProfile,
            consumer: OwnedCancellation,
            owner: ConnectivityRole(PmOwnedExecution),
            lane: FakeEffect,
            readiness: OwnedExecution,
            route: None,
        },
        ProductPlanRow {
            requirement: PmPlanRequirementId::QuoteEvaluationTimer,
            scope: PmRequirementScope::Instrument(instrument),
            origin: ModelPublicInput,
            consumer: QuoteEvaluationSchedule,
            owner: QuoteSchedule,
            lane: Scheduled,
            readiness: QuoteEvaluationClock,
            route: None,
        },
    ];

    let plan = PmConnectivityPlan::product(config, model, profile).unwrap();
    let actual = plan
        .entries()
        .iter()
        .map(|entry| ProductPlanRow {
            requirement: entry.key().id(),
            scope: entry.key().scope(),
            origin: entry.origin(),
            consumer: entry.consumer(),
            owner: entry.owner(),
            lane: entry.lane(),
            readiness: entry.readiness(),
            route: entry.route(),
        })
        .collect::<Vec<_>>();

    assert_eq!(actual, expected);
}

#[test]
fn binding_validator_rejects_missing_duplicate_and_wrong_route() {
    let (config, model, profile) = fixture();
    let plan = PmConnectivityPlan::product(config.clone(), model, profile).unwrap();
    let bindings = self_attested_bindings_for_negative_validation(&config);

    let mut missing = bindings.clone();
    missing.pop();
    assert_eq!(
        plan.validate_bindings(&missing),
        Err(PmPlanError::MissingRoleBinding)
    );

    let mut duplicate = bindings.clone();
    let last = duplicate.len() - 1;
    duplicate[last] = bindings[0];
    assert_eq!(
        plan.validate_bindings(&duplicate),
        Err(PmPlanError::DuplicateRoleBinding)
    );

    let mut wrong_route = bindings;
    wrong_route[0] = ConstructedRoleBinding::okx_public(
        config.public().okx_reference(),
        route(
            PmProductSource::okx_reference(
                PmSourceHandle::from_ordinal(9),
                config.public().okx_reference(),
            ),
            "wrong-okx-route",
        ),
    );
    assert_eq!(
        plan.validate_bindings(&wrong_route),
        Err(PmPlanError::RouteBindingMismatch)
    );

    let account = config.account();
    let too_many_spenders = vec![account.required_spenders()[0]; 9];
    assert_eq!(
        ConstructedRoleBinding::account_snapshot(
            account.account_scope(),
            account.instrument(),
            &too_many_spenders,
            account.account_route(),
        ),
        Err(PmPlanError::TooManyAccountSnapshotSpenders)
    );
}

#[test]
fn public_config_rejects_extra_references_and_unbound_routes() {
    let mut references = [None; 16];
    references[0] = Some(reference());
    references[1] = Some(OkxReferenceHandle::from_ordinal(8));
    let multiple = PmReferenceMapping::new(instrument(), references, 2).unwrap();
    assert_eq!(
        PmPublicConnectivityConfig::new(
            multiple,
            route(
                PmProductSource::okx_reference(PmSourceHandle::from_ordinal(1), reference()),
                "okx-public",
            ),
            route(
                PmProductSource::polymarket_market(
                    PmSourceHandle::from_ordinal(2),
                    instrument().token(),
                ),
                "pm-public",
            ),
        ),
        Err(PmConnectivityConfigError::ExpectedSingleReference)
    );

    assert_eq!(
        PmPublicConnectivityConfig::new(
            reference_mapping(),
            route(
                PmProductSource::okx_reference(
                    PmSourceHandle::from_ordinal(1),
                    OkxReferenceHandle::from_ordinal(8),
                ),
                "wrong-okx-reference",
            ),
            route(
                PmProductSource::polymarket_market(
                    PmSourceHandle::from_ordinal(2),
                    instrument().token(),
                ),
                "pm-public",
            ),
        ),
        Err(PmConnectivityConfigError::OkxRouteMismatch)
    );

    assert_eq!(
        PmPublicConnectivityConfig::new(
            reference_mapping(),
            route(
                PmProductSource::okx_reference(PmSourceHandle::from_ordinal(1), reference()),
                "okx-public",
            ),
            route(
                PmProductSource::polymarket_market(
                    PmSourceHandle::from_ordinal(2),
                    PmTokenHandle::from_ordinal(8),
                ),
                "wrong-pm-token",
            ),
        ),
        Err(PmConnectivityConfigError::PublicRouteMismatch)
    );
}

#[test]
fn fixed_account_profile_rejects_wrong_chain_and_split_signer_funder() {
    let (config, _, _) = fixture();
    let account = config.account();
    let eoa = EvmAddress::from_bytes([4; 20]).unwrap();
    let other = EvmAddress::from_bytes([5; 20]).unwrap();

    assert_eq!(
        PmAccountConnectivityConfig::new(
            account.instrument(),
            account.account_scope(),
            route(
                PmProductSource::polymarket_account(
                    PmSourceHandle::from_ordinal(3),
                    PmAccountHandle::from_ordinal(9),
                ),
                "wrong-pm-account",
            ),
            account.required_spenders().to_vec(),
        ),
        Err(PmConnectivityConfigError::AccountRouteMismatch)
    );

    let wrong_chain = PmAccountScope::new(
        account.account_scope().environment(),
        PmChainId::new(1).unwrap(),
        PmSignerId::new(eoa),
        PmFunderId::new(eoa),
        account.account(),
    );
    assert_eq!(
        PmAccountConnectivityConfig::new(
            account.instrument(),
            wrong_chain,
            account.account_route(),
            account.required_spenders().to_vec(),
        ),
        Err(PmConnectivityConfigError::WrongGoalFChain)
    );

    let split = PmAccountScope::new(
        account.account_scope().environment(),
        PmChainId::new(137).unwrap(),
        PmSignerId::new(eoa),
        PmFunderId::new(other),
        account.account(),
    );
    assert_eq!(
        PmAccountConnectivityConfig::new(
            account.instrument(),
            split,
            account.account_route(),
            account.required_spenders().to_vec(),
        ),
        Err(PmConnectivityConfigError::SignerFunderMismatch)
    );
}
