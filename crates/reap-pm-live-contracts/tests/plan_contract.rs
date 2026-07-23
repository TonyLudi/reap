use reap_pm_core::{
    EvmAddress, MAX_REQUIRED_SPENDERS, OkxInstrumentId, OkxReferenceHandle, OkxReferenceInstrument,
    PmAccountHandle, PmAccountScope, PmAssetId, PmChainId, PmConditionId, PmConnectionId,
    PmEnvironmentId, PmFunderId, PmInstrumentHandle, PmInstrumentId, PmMarketHandle, PmMarketId,
    PmMarketLifecycle, PmMarketMetadata, PmOutcomeLabel, PmOutcomeMetadata, PmProductSource,
    PmQuantity, PmReferenceMapping, PmSignerId, PmSourceHandle, PmSpenderDomain, PmSpenderId,
    PmSpenderRequirement, PmTick, PmTokenHandle, PmTokenId, U256,
};
use reap_pm_live_contracts::{
    ConstructedRoleBinding, PmAccountConnectivityConfig, PmCapabilityLane,
    PmCapabilityRequirementId, PmCompositionRoot, PmConnectionRoute, PmConnectivityConfig,
    PmConnectivityConfigError, PmConnectivityPlan, PmFakeExecutionProfile, PmPlanError,
    PmPlanOwner, PmPlanRequirementId, PmPublicConnectivityConfig, PmReadinessDependency,
    PmRequirementConsumer, PmRequirementOrigin, PmRequirementScope, PmRoleKind,
};
use reap_pm_strategy::PmModelInputRequirements;

const GOAL_F_PUSD: &str = "0xC011a7E12a19f7B1f670d46F03B03f3342E82DFB";
const GOAL_F_CTF: &str = "0x4D97DCd97eC945f40cF65F87097ACe5EA0476045";
const GOAL_F_STANDARD_EXCHANGE: &str = "0xE111180000d2663C0091e4f400237545B87B996B";

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
        PmMarketHandle::from_ordinal(0),
        PmTokenHandle::from_ordinal(0),
    )
}

fn reference() -> OkxReferenceHandle {
    OkxReferenceHandle::from_ordinal(0)
}

fn reference_instrument() -> OkxReferenceInstrument {
    OkxReferenceInstrument::index(OkxInstrumentId::new("BTC-USDT").unwrap())
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
    let exchange = EvmAddress::parse(GOAL_F_STANDARD_EXCHANGE).unwrap();
    vec![
        PmSpenderId::new(
            account.handle(),
            PmSpenderRequirement::new(
                PmChainId::new(137).unwrap(),
                exchange,
                PmSpenderDomain::Standard,
                PmAssetId::collateral(EvmAddress::parse(GOAL_F_PUSD).unwrap()),
            ),
        ),
        PmSpenderId::new(
            account.handle(),
            PmSpenderRequirement::new(
                PmChainId::new(137).unwrap(),
                exchange,
                PmSpenderDomain::Standard,
                PmAssetId::outcome(
                    EvmAddress::parse(GOAL_F_CTF).unwrap(),
                    PmTokenId::new(U256::from_u64(11)).unwrap(),
                ),
            ),
        ),
    ]
}

fn expected_metadata() -> PmMarketMetadata {
    expected_metadata_for_market(
        PmMarketId::parse("0xbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbb")
            .unwrap(),
    )
}

fn expected_metadata_for_market(market: PmMarketId) -> PmMarketMetadata {
    expected_metadata_contract(market, "0.01", "1")
}

fn expected_metadata_contract(
    market: PmMarketId,
    tick: &str,
    minimum_order_size: &str,
) -> PmMarketMetadata {
    let spenders = required_spenders(account_scope());
    let mut required = [None; MAX_REQUIRED_SPENDERS];
    for (slot, spender) in required.iter_mut().zip(&spenders) {
        *slot = Some(spender.requirement());
    }
    PmMarketMetadata::new(
        PmConditionId::parse("0xaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa")
            .unwrap(),
        market,
        PmOutcomeMetadata::new(
            PmTokenId::new(U256::from_u64(11)).unwrap(),
            PmOutcomeLabel::new("Yes").unwrap(),
        ),
        PmMarketLifecycle::new(true, false, false, true, true),
        PmTick::parse_decimal(tick).unwrap(),
        PmQuantity::parse_decimal(minimum_order_size).unwrap(),
        false,
        PmChainId::new(137).unwrap(),
        EvmAddress::parse(GOAL_F_STANDARD_EXCHANGE).unwrap(),
        required,
        spenders.len() as u8,
    )
    .unwrap()
}

fn fixture() -> (
    PmConnectivityConfig,
    PmModelInputRequirements,
    PmFakeExecutionProfile,
) {
    let account = account_scope();
    let public = PmPublicConnectivityConfig::derive_goal_f(
        reference_instrument(),
        expected_metadata(),
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
    let account = PmAccountConnectivityConfig::derive_goal_f(
        &public,
        account,
        route(
            PmProductSource::polymarket_account(PmSourceHandle::from_ordinal(3), account.handle()),
            "pm-account",
        ),
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
        account.instrument(),
        account.instrument_id(),
        account.account_route(),
    ));
    bindings.extend(ConstructedRoleBinding::reconciliation(
        account.account_scope(),
        account.instrument(),
        account.instrument_id(),
        account.account_route(),
    ));
    bindings.extend(
        ConstructedRoleBinding::account_snapshot(
            account.account_scope(),
            account.instrument(),
            account.instrument_id(),
            account.collateral_asset(),
            account.required_spenders(),
            account.account_route(),
        )
        .unwrap(),
    );
    bindings.extend(ConstructedRoleBinding::owned_execution(
        account.account_scope(),
        account.instrument(),
        account.instrument_id(),
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
        instrument_id: config.account().instrument_id(),
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
            scope: account_instrument,
            origin: MandatorySafetyAndReadiness,
            consumer: CanonicalOrderState,
            owner: ConnectivityRole(PmPrivateLifecycle),
            lane: Private,
            readiness: PrivateLifecycle,
            route: Some(account_route),
        },
        ProductPlanRow {
            requirement: PmPlanRequirementId::Connectivity(PrivateFill),
            scope: account_instrument,
            origin: MandatorySafetyAndReadiness,
            consumer: FillAndPositionState,
            owner: ConnectivityRole(PmPrivateLifecycle),
            lane: Private,
            readiness: PrivateLifecycle,
            route: Some(account_route),
        },
        ProductPlanRow {
            requirement: PmPlanRequirementId::Connectivity(ReconcileOpenOrders),
            scope: account_instrument,
            origin: MandatorySafetyAndReadiness,
            consumer: OrderReconciliation,
            owner: ConnectivityRole(PmOrderReconciliation),
            lane: Reconciliation,
            readiness: PmReadinessDependency::OrderReconciliation,
            route: Some(account_route),
        },
        ProductPlanRow {
            requirement: PmPlanRequirementId::Connectivity(ReconcileOrder),
            scope: account_instrument,
            origin: MandatorySafetyAndReadiness,
            consumer: OrderReconciliation,
            owner: ConnectivityRole(PmOrderReconciliation),
            lane: Reconciliation,
            readiness: PmReadinessDependency::OrderReconciliation,
            route: Some(account_route),
        },
        ProductPlanRow {
            requirement: PmPlanRequirementId::Connectivity(ReconcileFills),
            scope: account_instrument,
            origin: MandatorySafetyAndReadiness,
            consumer: OrderReconciliation,
            owner: ConnectivityRole(PmOrderReconciliation),
            lane: Reconciliation,
            readiness: PmReadinessDependency::OrderReconciliation,
            route: Some(account_route),
        },
        ProductPlanRow {
            requirement: PmPlanRequirementId::Connectivity(AccountCollateral),
            scope: PmRequirementScope::AccountAsset {
                account,
                asset: config.account().collateral_asset(),
            },
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
            account.instrument_id(),
            account.collateral_asset(),
            &too_many_spenders,
            account.account_route(),
        ),
        Err(PmPlanError::TooManyAccountSnapshotSpenders)
    );
}

#[test]
fn goal_f_public_config_derives_one_canonical_table_and_checked_grant() {
    let (config, _, _) = fixture();
    let public = config.public();
    let grant = public.observation_grant();
    let expected = expected_metadata();

    assert_eq!(grant.okx_reference().ordinal(), 0);
    assert_eq!(grant.instrument().market().ordinal(), 0);
    assert_eq!(grant.instrument().token().ordinal(), 0);
    assert_eq!(grant.okx_instrument(), reference_instrument());
    assert_eq!(
        grant.polymarket_instrument(),
        PmInstrumentId::new(expected.market(), expected.outcome().token())
    );
    assert_eq!(public.mapping().target(), grant.instrument());
    assert_eq!(
        public.mapping().references().collect::<Vec<_>>(),
        vec![grant.okx_reference()]
    );
    assert_eq!(
        public.configuration_fingerprint(),
        grant.configuration_fingerprint()
    );

    let independently_derived = PmPublicConnectivityConfig::derive_goal_f(
        reference_instrument(),
        expected,
        public.okx_route(),
        public.polymarket_route(),
    )
    .unwrap();
    assert_eq!(
        independently_derived.configuration_fingerprint(),
        public.configuration_fingerprint(),
        "the same ordered raw identity tables must produce the same fingerprint"
    );
}

#[test]
fn goal_f_public_config_rejects_arbitrary_compact_handle_associations() {
    let mut noncanonical_references = [None; 16];
    let arbitrary_reference = OkxReferenceHandle::from_ordinal(8);
    noncanonical_references[0] = Some(arbitrary_reference);
    let mapping = PmReferenceMapping::new(instrument(), noncanonical_references, 1).unwrap();
    assert_eq!(
        PmPublicConnectivityConfig::new(
            mapping,
            reference_instrument(),
            expected_metadata(),
            route(
                PmProductSource::okx_reference(
                    PmSourceHandle::from_ordinal(1),
                    arbitrary_reference,
                ),
                "arbitrary-okx-handle",
            ),
            route(
                PmProductSource::polymarket_market(
                    PmSourceHandle::from_ordinal(2),
                    instrument().token(),
                ),
                "pm-public",
            ),
        ),
        Err(PmConnectivityConfigError::NonCanonicalReferenceHandle)
    );

    let arbitrary_instrument = PmInstrumentHandle::new(
        PmMarketHandle::from_ordinal(7),
        PmTokenHandle::from_ordinal(9),
    );
    let mut references = [None; 16];
    references[0] = Some(reference());
    let mapping = PmReferenceMapping::new(arbitrary_instrument, references, 1).unwrap();
    assert_eq!(
        PmPublicConnectivityConfig::new(
            mapping,
            reference_instrument(),
            expected_metadata(),
            route(
                PmProductSource::okx_reference(PmSourceHandle::from_ordinal(1), reference(),),
                "okx-public",
            ),
            route(
                PmProductSource::polymarket_market(
                    PmSourceHandle::from_ordinal(2),
                    arbitrary_instrument.token(),
                ),
                "arbitrary-pm-handle",
            ),
        ),
        Err(PmConnectivityConfigError::NonCanonicalInstrumentHandle)
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
            reference_instrument(),
            expected_metadata(),
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
            reference_instrument(),
            expected_metadata(),
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
            reference_instrument(),
            expected_metadata(),
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
        PmAccountConnectivityConfig::derive_goal_f(
            config.public(),
            account.account_scope(),
            route(
                PmProductSource::polymarket_account(
                    PmSourceHandle::from_ordinal(3),
                    PmAccountHandle::from_ordinal(9),
                ),
                "wrong-pm-account",
            ),
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
        PmAccountConnectivityConfig::derive_goal_f(
            config.public(),
            wrong_chain,
            account.account_route(),
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
        PmAccountConnectivityConfig::derive_goal_f(config.public(), split, account.account_route(),),
        Err(PmConnectivityConfigError::SignerFunderMismatch)
    );
}

#[test]
fn combined_scope_rejects_a_structurally_different_market_with_the_same_compact_handle() {
    let (config, _, _) = fixture();
    let public = config.public().clone();
    let account = config.account();
    let different_market = PmMarketId::from_bytes([91; 32]).unwrap();
    let other_public = PmPublicConnectivityConfig::derive_goal_f(
        reference_instrument(),
        expected_metadata_for_market(different_market),
        public.okx_route(),
        public.polymarket_route(),
    )
    .unwrap();
    let other_account = PmAccountConnectivityConfig::derive_goal_f(
        &other_public,
        account.account_scope(),
        account.account_route(),
    )
    .unwrap();
    assert_eq!(
        PmConnectivityConfig::new(public.clone(), other_account),
        Err(PmConnectivityConfigError::InstrumentScopeMismatch)
    );
    assert_eq!(public.instrument(), account.instrument());
}

#[test]
fn combined_scope_rejects_private_grid_drift_for_the_same_structural_instrument() {
    let (config, _, _) = fixture();
    let public = config.public().clone();
    let account = config.account();
    let other_public = PmPublicConnectivityConfig::derive_goal_f(
        reference_instrument(),
        expected_metadata_contract(public.expected_metadata().market(), "0.005", "1"),
        public.okx_route(),
        public.polymarket_route(),
    )
    .unwrap();
    let other_account = PmAccountConnectivityConfig::derive_goal_f(
        &other_public,
        account.account_scope(),
        account.account_route(),
    )
    .unwrap();

    assert_eq!(
        PmConnectivityConfig::new(public, other_account),
        Err(PmConnectivityConfigError::AccountInstrumentScopeMismatch)
    );
}
