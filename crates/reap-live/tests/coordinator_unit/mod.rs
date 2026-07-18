use std::collections::{HashMap, HashSet};
use std::sync::Arc;

use reap_core::{
    AccountUpdate, Balance, FillFee, FillKey, FillLiquidity, Level, MarketEvent, NewOrder,
    OrderBook, OrderEvent, OrderStatus, OrderUpdate, Position, PositionMarginMode, Side,
    SystemEvent, SystemEventKind, TimeInForce, TimeMs, Venue,
};
use reap_feed::FeedOutput;
use reap_order::{
    CancelOrderTransportError, OkxOrderGateway, OrderTransportError, PacingPolicy,
    PreparedRegularCancel, PreparedRegularSubmit, ReconcileIssue, RegularExecution,
    RegularReconciliation, reconcile, reconcile_full_state,
};
use reap_risk::{
    InstrumentOrderLimits, InstrumentRiskModel, RiskDecision, RiskLimits, RiskRejectReason,
    StablecoinGuardConfig,
};
use reap_strategy::{ChaosConfig, ReferenceDataKind};
use reap_venue::okx::{
    OkxAccountLevel, OkxFillPage, OkxInstrumentType, OkxOrderAck, OkxPositionMode,
    OkxRegularOrderPage, RestError,
};
use reap_venue::{PrivateOrderState, PrivateOrderUpdate, RemoteFill, RemoteOrder};

use crate::forbidden_orders::{ForbiddenOrderEvent, ForbiddenOrderState};
use crate::{
    LiveAccountConfig, LiveStorageConfig, OkxTradeModeConfig, OkxVenueConfig, RuntimeConfig,
    VerifiedInstrument,
};

use super::*;

#[derive(Debug)]
struct ScopeOnlyGatewayRoles;

#[async_trait::async_trait]
impl RegularExecution for ScopeOnlyGatewayRoles {
    async fn place_regular_order(
        &self,
        _order: PreparedRegularSubmit,
    ) -> Result<OkxOrderAck, OrderTransportError> {
        unreachable!("scope-only test gateway never executes orders")
    }

    async fn cancel_regular_order(
        &self,
        _cancel: PreparedRegularCancel,
    ) -> Result<OkxOrderAck, CancelOrderTransportError> {
        unreachable!("scope-only test gateway never executes orders")
    }

    async fn cancel_regular_order_via_rest(
        &self,
        _cancel: PreparedRegularCancel,
    ) -> Result<OkxOrderAck, OrderTransportError> {
        unreachable!("scope-only test gateway never executes orders")
    }
}

#[async_trait::async_trait]
impl RegularReconciliation for ScopeOnlyGatewayRoles {
    async fn regular_pending_orders_page(
        &self,
        _instrument_type: Option<&str>,
        _symbol: Option<&str>,
        _after: Option<&str>,
    ) -> Result<OkxRegularOrderPage, RestError> {
        unreachable!("scope-only test gateway never reconciles orders")
    }

    async fn recent_fills_page(
        &self,
        _instrument_type: Option<&str>,
        _symbol: Option<&str>,
        _after: Option<&str>,
    ) -> Result<OkxFillPage, RestError> {
        unreachable!("scope-only test gateway never reconciles fills")
    }

    async fn account_balance(&self) -> Result<AccountUpdate, RestError> {
        unreachable!("scope-only test gateway never fetches balances")
    }

    async fn account_positions(&self) -> Result<AccountUpdate, RestError> {
        unreachable!("scope-only test gateway never fetches positions")
    }

    async fn order_details(
        &self,
        _symbol: &str,
        _client_order_id: &str,
    ) -> Result<RemoteOrder, RestError> {
        unreachable!("scope-only test gateway never fetches order details")
    }

    async fn server_time_ms(&self) -> Result<u64, RestError> {
        unreachable!("scope-only test gateway never fetches server time")
    }
}

fn approval_scopes(
    account_ids: impl IntoIterator<Item = &'static str>,
) -> HashMap<String, RegularApprovalScope> {
    let roles = Arc::new(ScopeOnlyGatewayRoles);
    account_ids
        .into_iter()
        .map(|account_id| {
            let mut gateway = OkxOrderGateway::new(
                account_id,
                Box::new(ScopeOnlyGatewayRoles),
                roles.clone(),
                HashMap::new(),
                PacingPolicy::default(),
            )
            .expect("scope-only test gateway must be valid");
            let scope = gateway
                .take_approval_scope()
                .expect("test gateway must yield its approval scope once");
            (account_id.to_string(), scope)
        })
        .collect()
}

fn restore_test_owned_order(
    coordinator: &mut LiveCoordinator,
    account_id: &str,
    update: OrderUpdate,
) -> Result<CoordinatorOutput, CoordinatorError> {
    let proof = test_recovered_submit_proof(account_id, &update.symbol, &update.order_id);
    coordinator.restore_owned_order(proof, update)
}

fn coordinator_with_gateway_actions(gateway_actions_enabled: bool) -> LiveCoordinator {
    coordinator_with_risk(
        gateway_actions_enabled,
        RiskLimits {
            require_feed_health: false,
            require_private_health: false,
            ..RiskLimits::default()
        },
    )
}

fn coordinator_with_risk(gateway_actions_enabled: bool, risk: RiskLimits) -> LiveCoordinator {
    let mut strategy: ChaosConfig =
        toml::from_str(include_str!("../../../../examples/iarb2-basic.toml")).unwrap();
    strategy.reference_data_stale_threshold_ms = Some(120_000);
    strategy.risk_groups[0].account_id = Some("main".to_string());
    let config = LiveConfig {
        strategy,
        risk,
        venue: OkxVenueConfig::default(),
        runtime: RuntimeConfig::default(),
        storage: LiveStorageConfig::default(),
        operator: crate::OperatorConfig::default(),
        alerts: crate::AlertConfig::default(),
        host_guard: crate::HostGuardConfig::default(),
        accounts: vec![LiveAccountConfig {
            id: "main".to_string(),
            api_key_env: "KEY".to_string(),
            secret_key_env: "SECRET".to_string(),
            passphrase_env: "PASS".to_string(),
            expected_account_level: OkxAccountLevel::SingleCurrencyMargin,
            expected_position_mode: OkxPositionMode::NetMode,
            api_key_policy: crate::OkxApiKeyPolicyConfig::default(),
            id_prefix: "reap".to_string(),
            node_id: 1,
            trade_modes: HashMap::from([
                ("BTC-USDT".to_string(), OkxTradeModeConfig::Cash),
                ("BTC-PERP".to_string(), OkxTradeModeConfig::Cross),
            ]),
        }],
    };
    let verified = VerifiedBootstrap {
        instruments: HashMap::from([
            (
                "BTC-USDT".to_string(),
                VerifiedInstrument {
                    account_id: "main".to_string(),
                    symbol: "BTC-USDT".to_string(),
                    instrument_type: OkxInstrumentType::Spot,
                    trade_mode: OkxTradeModeConfig::Cash,
                    risk_model: InstrumentRiskModel::Spot,
                    order_limits: InstrumentOrderLimits {
                        max_limit_quantity: 100.0,
                        max_limit_notional_usd: Some(1_000_000.0),
                    },
                    tick_size: 0.1,
                    lot_size: 0.0001,
                    min_size: 0.0001,
                    contract_value: None,
                },
            ),
            (
                "BTC-PERP".to_string(),
                VerifiedInstrument {
                    account_id: "main".to_string(),
                    symbol: "BTC-PERP".to_string(),
                    instrument_type: OkxInstrumentType::Futures,
                    trade_mode: OkxTradeModeConfig::Cross,
                    risk_model: InstrumentRiskModel::LinearDerivative {
                        contract_value: 0.001,
                    },
                    order_limits: InstrumentOrderLimits {
                        max_limit_quantity: 1_000_000.0,
                        max_limit_notional_usd: None,
                    },
                    tick_size: 0.1,
                    lot_size: 1.0,
                    min_size: 1.0,
                    contract_value: Some(0.001),
                },
            ),
        ]),
        account_updates: HashMap::from([(
            "main".to_string(),
            AccountUpdate {
                ts_ms: 1,
                balances: vec![Balance {
                    account_id: Some("main".to_string()),
                    currency: "USDT".to_string(),
                    total: 10_000.0,
                    available: 10_000.0,
                    equity: 10_000.0,
                    liability: 0.0,
                    max_loan: 0.0,
                    forced_repayment_indicator: None,
                }],
                positions: Vec::new(),
                margins: Vec::new(),
            },
        )]),
        baseline_fill_ids: HashMap::from([("main".to_string(), HashSet::new())]),
        quote_stp_verified_accounts: HashSet::from(["main".to_string()]),
    };
    let approval_scopes = if gateway_actions_enabled {
        approval_scopes(["main"])
    } else {
        HashMap::new()
    };
    LiveCoordinator::new(config, verified, approval_scopes, "test-session").unwrap()
}

fn coordinator() -> LiveCoordinator {
    coordinator_with_gateway_actions(true)
}

fn two_account_coordinator() -> LiveCoordinator {
    let mut config = coordinator().config.clone();
    let mut hedge_group = config.strategy.risk_groups[0].clone();
    config.strategy.risk_groups[0].symbols = vec!["BTC-USDT".to_string()];
    hedge_group.name = "hedge".to_string();
    hedge_group.account_id = Some("hedge".to_string());
    hedge_group.symbols = vec!["BTC-PERP".to_string()];
    config.strategy.risk_groups.push(hedge_group);
    config.strategy.instruments[1].risk_group = "hedge".to_string();
    config.accounts[0].trade_modes.remove("BTC-PERP");
    config.accounts.push(LiveAccountConfig {
        id: "hedge".to_string(),
        api_key_env: "HEDGE_KEY".to_string(),
        secret_key_env: "HEDGE_SECRET".to_string(),
        passphrase_env: "HEDGE_PASS".to_string(),
        expected_account_level: OkxAccountLevel::SingleCurrencyMargin,
        expected_position_mode: OkxPositionMode::NetMode,
        api_key_policy: crate::OkxApiKeyPolicyConfig::default(),
        id_prefix: "hedge".to_string(),
        node_id: 2,
        trade_modes: HashMap::from([("BTC-PERP".to_string(), OkxTradeModeConfig::Cross)]),
    });
    let verified = VerifiedBootstrap {
        instruments: HashMap::from([
            (
                "BTC-USDT".to_string(),
                VerifiedInstrument {
                    account_id: "main".to_string(),
                    symbol: "BTC-USDT".to_string(),
                    instrument_type: OkxInstrumentType::Spot,
                    trade_mode: OkxTradeModeConfig::Cash,
                    risk_model: InstrumentRiskModel::Spot,
                    order_limits: InstrumentOrderLimits {
                        max_limit_quantity: 100.0,
                        max_limit_notional_usd: Some(1_000_000.0),
                    },
                    tick_size: 0.1,
                    lot_size: 0.0001,
                    min_size: 0.0001,
                    contract_value: None,
                },
            ),
            (
                "BTC-PERP".to_string(),
                VerifiedInstrument {
                    account_id: "hedge".to_string(),
                    symbol: "BTC-PERP".to_string(),
                    instrument_type: OkxInstrumentType::Futures,
                    trade_mode: OkxTradeModeConfig::Cross,
                    risk_model: InstrumentRiskModel::LinearDerivative {
                        contract_value: 0.001,
                    },
                    order_limits: InstrumentOrderLimits {
                        max_limit_quantity: 1_000_000.0,
                        max_limit_notional_usd: None,
                    },
                    tick_size: 0.1,
                    lot_size: 1.0,
                    min_size: 1.0,
                    contract_value: Some(0.001),
                },
            ),
        ]),
        account_updates: HashMap::from([
            ("main".to_string(), account_update("main", 1)),
            ("hedge".to_string(), account_update("hedge", 1)),
        ]),
        baseline_fill_ids: HashMap::from([
            ("main".to_string(), HashSet::new()),
            ("hedge".to_string(), HashSet::new()),
        ]),
        quote_stp_verified_accounts: HashSet::from(["main".to_string(), "hedge".to_string()]),
    };
    LiveCoordinator::new(
        config,
        verified,
        approval_scopes(["main", "hedge"]),
        "two-account-test",
    )
    .unwrap()
}

fn account_update(account_id: &str, ts_ms: TimeMs) -> AccountUpdate {
    AccountUpdate {
        ts_ms,
        balances: vec![Balance {
            account_id: Some(account_id.to_string()),
            currency: "USDT".to_string(),
            total: 10_000.0,
            available: 10_000.0,
            equity: 10_000.0,
            liability: 0.0,
            max_loan: 0.0,
            forced_repayment_indicator: None,
        }],
        positions: Vec::new(),
        margins: Vec::new(),
    }
}

fn bootstrap_readiness(coordinator: &mut LiveCoordinator) {
    coordinator.mark_storage_ready(true, "open");
    coordinator.mark_public_connectivity(true, "connected");
    coordinator
        .process_feed(FeedOutput::PrivateAccount {
            account_id: Some("main".to_string()),
            update: AccountUpdate {
                ts_ms: 1,
                balances: vec![Balance {
                    account_id: Some("main".to_string()),
                    currency: "USDT".to_string(),
                    total: 10_000.0,
                    available: 10_000.0,
                    equity: 10_000.0,
                    liability: 0.0,
                    max_loan: 0.0,
                    forced_repayment_indicator: None,
                }],
                positions: Vec::new(),
                margins: Vec::new(),
            },
        })
        .unwrap();
    coordinator
        .on_reconciliation(ReconciliationResult {
            account_id: "main".to_string(),
            ts_ms: 2,
            clean: true,
            local_live_orders: 0,
            remote_live_orders: 0,
            remote_recent_fills: 0,
            reason: "clean".to_string(),
        })
        .unwrap();
    for symbol in ["BTC-USDT", "BTC-PERP"] {
        coordinator.process_event(NormalizedEvent::System(SystemEvent {
            ts_ms: 2,
            kind: SystemEventKind::FeedRecovered,
            venue: Some(Venue::Okx),
            account_id: None,
            symbol: Some(symbol.to_string()),
            reason: "snapshot".to_string(),
        }));
    }
    coordinator.process_event(NormalizedEvent::System(SystemEvent {
        ts_ms: 2,
        kind: SystemEventKind::PrivateStreamRecovered,
        venue: Some(Venue::Okx),
        account_id: Some("main".to_string()),
        symbol: None,
        reason: "connected".to_string(),
    }));
    coordinator.process_event(NormalizedEvent::System(SystemEvent {
        ts_ms: 2,
        kind: SystemEventKind::OrderTransportRecovered,
        venue: Some(Venue::Okx),
        account_id: Some("main".to_string()),
        symbol: None,
        reason: "all sessions authenticated".to_string(),
    }));
    coordinator
        .startup
        .mark_forbidden_order_proof("main", true, "complete zero proof")
        .unwrap();
    seed_strategy_references(coordinator, 2);
}

fn seed_strategy_references(coordinator: &mut LiveCoordinator, ts_ms: TimeMs) {
    let requirements = coordinator.config.strategy.reference_data_requirements();
    for requirement in requirements {
        let event = match requirement.kind {
            ReferenceDataKind::IndexPrice => MarketEvent::IndexPrice {
                ts_ms,
                symbol: requirement.symbol,
                price: 100.0,
            },
            ReferenceDataKind::FundingRate => MarketEvent::FundingRate {
                ts_ms,
                symbol: requirement.symbol,
                rate: 0.0001,
                funding_time_ms: ts_ms + 28_800_000,
                settlement: None,
            },
            ReferenceDataKind::MarkPrice => MarketEvent::PriceLimits {
                ts_ms,
                symbol: requirement.symbol,
                mark_price: 100.0,
                limit_down: 0.0,
                limit_up: 0.0,
            },
            ReferenceDataKind::PriceLimits => MarketEvent::PriceLimits {
                ts_ms,
                symbol: requirement.symbol,
                mark_price: 0.0,
                limit_down: 50.0,
                limit_up: 150.0,
            },
        };
        coordinator.process_event(NormalizedEvent::Market(event));
    }
}

fn ready(coordinator: &mut LiveCoordinator) {
    bootstrap_readiness(coordinator);
    assert!(coordinator.readiness().is_ready());
}

fn ready_two_accounts(coordinator: &mut LiveCoordinator) {
    coordinator.mark_storage_ready(true, "open");
    coordinator.mark_public_connectivity(true, "connected");
    for account_id in ["main", "hedge"] {
        coordinator
            .process_feed(FeedOutput::PrivateAccount {
                account_id: Some(account_id.to_string()),
                update: account_update(account_id, 1),
            })
            .unwrap();
        coordinator
            .on_reconciliation(ReconciliationResult {
                account_id: account_id.to_string(),
                ts_ms: 2,
                clean: true,
                local_live_orders: 0,
                remote_live_orders: 0,
                remote_recent_fills: 0,
                reason: "clean".to_string(),
            })
            .unwrap();
        coordinator
            .startup
            .mark_forbidden_order_proof(account_id, true, "complete zero proof")
            .unwrap();
    }
    for symbol in ["BTC-USDT", "BTC-PERP"] {
        coordinator.process_event(NormalizedEvent::System(SystemEvent {
            ts_ms: 2,
            kind: SystemEventKind::FeedRecovered,
            venue: Some(Venue::Okx),
            account_id: None,
            symbol: Some(symbol.to_string()),
            reason: "snapshot".to_string(),
        }));
    }
    for account_id in ["main", "hedge"] {
        coordinator.process_event(NormalizedEvent::System(SystemEvent {
            ts_ms: 2,
            kind: SystemEventKind::PrivateStreamRecovered,
            venue: Some(Venue::Okx),
            account_id: Some(account_id.to_string()),
            symbol: None,
            reason: "connected".to_string(),
        }));
        coordinator.process_event(NormalizedEvent::System(SystemEvent {
            ts_ms: 2,
            kind: SystemEventKind::OrderTransportRecovered,
            venue: Some(Venue::Okx),
            account_id: Some(account_id.to_string()),
            symbol: None,
            reason: "all sessions authenticated".to_string(),
        }));
    }
    seed_strategy_references(coordinator, 2);
    assert!(coordinator.readiness().is_ready());
}

fn order() -> NewOrder {
    NewOrder {
        symbol: "BTC-USDT".to_string(),
        side: Side::Buy,
        qty: 0.1,
        price: 100.0,
        time_in_force: TimeInForce::PostOnly,
        reduce_only: false,
        self_trade_prevention: None,
        reason: "quote".to_string(),
    }
}

fn cancelled_private_order(
    client_order_id: &str,
    exchange_order_id: &str,
    ts_ms: TimeMs,
) -> PrivateOrderUpdate {
    PrivateOrderUpdate {
        ts_ms,
        exchange_order_id: exchange_order_id.to_string(),
        client_order_id: client_order_id.to_string(),
        symbol: "BTC-USDT".to_string(),
        side: Side::Buy,
        state: PrivateOrderState::Cancelled,
        price: 100.0,
        qty: 0.1,
        cumulative_filled_qty: 0.0,
        average_fill_price: 0.0,
        last_fill_qty: 0.0,
        last_fill_price: 0.0,
        liquidity: None,
        last_fill_fee: None,
        fill_id: None,
        reject_reason: String::new(),
    }
}

mod execution_recovery;
mod private_feed;
mod readiness_reconciliation;
mod routing_safety;

fn collect_coordinator_module_sources(
    directory: &std::path::Path,
    sources: &mut Vec<(String, String)>,
) {
    if !directory.exists() {
        return;
    }
    let mut entries = std::fs::read_dir(directory)
        .unwrap_or_else(|error| panic!("failed to read {}: {error}", directory.display()))
        .collect::<Result<Vec<_>, _>>()
        .unwrap_or_else(|error| panic!("failed to enumerate {}: {error}", directory.display()));
    entries.sort_by_key(|entry| entry.path());
    for entry in entries {
        let path = entry.path();
        if path.is_dir() {
            collect_coordinator_module_sources(&path, sources);
        } else if path.extension().is_some_and(|extension| extension == "rs") {
            let source = std::fs::read_to_string(&path)
                .unwrap_or_else(|error| panic!("failed to read {}: {error}", path.display()));
            sources.push((path.display().to_string(), source));
        }
    }
}

fn contains_rust_identifier(source: &str, identifier: &str) -> bool {
    source.match_indices(identifier).any(|(start, _)| {
        let before = source[..start].chars().next_back();
        let after = source[start + identifier.len()..].chars().next();
        let is_identifier_character =
            |character: char| character.is_ascii_alphanumeric() || character == '_';
        !before.is_some_and(is_identifier_character) && !after.is_some_and(is_identifier_character)
    })
}

#[test]
fn production_coordinator_keeps_single_owner_responsibility_state() {
    const TEST_MODULE_MARKER: &str =
        "#[cfg(test)]\n#[path = \"../tests/coordinator_unit/mod.rs\"]\nmod tests";

    let manifest = std::path::Path::new(env!("CARGO_MANIFEST_DIR"));
    let root_path = manifest.join("src/coordinator.rs");
    let root_source = std::fs::read_to_string(&root_path)
        .unwrap_or_else(|error| panic!("failed to read {}: {error}", root_path.display()));
    let root_path_string = root_path.display().to_string();
    let (production_root, _) = root_source
        .split_once(TEST_MODULE_MARKER)
        .expect("coordinator test module marker");
    let mut sources = vec![(root_path.display().to_string(), production_root.to_string())];
    collect_coordinator_module_sources(&manifest.join("src/coordinator"), &mut sources);

    assert_eq!(
        sources
            .iter()
            .map(|(_, source)| source.matches("pub struct LiveCoordinator").count())
            .sum::<usize>(),
        1,
        "production coordinator sources must define exactly one LiveCoordinator owner",
    );
    for field in [
        "config: LiveConfig",
        "engine: TradingEngine<ChaosStrategy>",
        "startup: StartupGate",
        "private_states: HashMap<String, PrivateStateReducer>",
        "regular_execution: RegularExecutionPolicy",
        "owned_regular_orders: OwnedRegularOrders",
        "client_ids: HashMap<String, ClientOrderIdGenerator>",
        "gateway_action_accounts: HashSet<String>",
        "order_entry_enabled: bool",
        "halted_accounts: BTreeMap<String, String>",
        "journal_fill_keys_by_account: HashMap<String, HashSet<FillKey>>",
        "session_id: String",
        "decision_sequence: u64",
    ] {
        assert!(
            production_root.contains(field),
            "sole-owner state `{field}` must remain on the root LiveCoordinator",
        );
        for (path, source) in sources.iter().filter(|(path, _)| path != &root_path_string) {
            assert!(
                !source.contains(field),
                "{path} must not redeclare sole-owner state `{field}`",
            );
        }
    }
    assert_eq!(
        production_root.matches(".register_recovered(").count(),
        2,
        "production and test recovery seams must remain one-for-one on the root coordinator",
    );
    assert_eq!(
        sources
            .iter()
            .filter(|(path, _)| path != &root_path_string)
            .map(|(_, source)| source.matches(".register_recovered(").count())
            .sum::<usize>(),
        0,
        "child coordinator reducers must not register recovered ownership authority",
    );
    assert!(
        production_root.contains("ProvenRegularSubmitRequest"),
        "durable recovered-submit proof handling must remain on the root coordinator",
    );

    for (path, source) in sources {
        let compact = source
            .chars()
            .filter(|character| !character.is_whitespace())
            .collect::<String>();
        assert!(
            !compact.contains("Arc<Mutex"),
            "{path} must not put coordinator ownership behind Arc<Mutex<_>>",
        );
        assert!(
            !source.contains("use super::*;"),
            "{path} must declare explicit production dependencies",
        );
        if path != root_path_string {
            for authority_type in [
                "TradingEngine",
                "StartupGate",
                "PrivateStateReducer",
                "RegularExecutionPolicy",
                "OwnedRegularOrders",
                "ClientOrderIdGenerator",
                "ProvenRegularSubmitRequest",
                "ReservedRegularSubmit",
                "ApprovedRegularCancel",
            ] {
                assert!(
                    !contains_rust_identifier(&source, authority_type),
                    "{path} must not declare, accept, or mint authority-bearing `{authority_type}` state",
                );
            }
        }
    }
}
