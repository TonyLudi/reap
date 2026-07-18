use std::collections::{HashMap, HashSet};
use std::sync::Arc;
use std::time::Duration;

use reap_okx_live_adapter::{
    BoundRegularOrderGateway, ConnectionSettings, CredentialEnvNames, ForbiddenOrderObserver,
    PrivateStateSessionFactory, demo_from_env, observe_from_env,
};
use reap_order::OkxReconciliationClient;
use reap_venue::okx::OKX_MIN_ACCOUNT_INSTRUMENT_REQUEST_INTERVAL_MS;
use reap_venue::{PrivateOrderState, RemoteOrder};

use crate::{
    AccountBootstrapSnapshot, LiveConfig, LiveMode, MaintenanceRelevancePlan, VerifiedBootstrap,
    okx_instrument_type, verify_bootstrap,
};

use super::readiness_safety::{
    ExchangeInstrumentGuard, ReadinessPort, SafetyPort, exchange_instrument_expectations,
    exchange_status_block_reason, rest_clock_skew_ms, verify_initial_exchange_fees,
    verify_initial_exchange_instruments,
};
use super::recovery::remote_order_id;
use super::{LiveRuntimeError, unix_time_ms};

pub(super) struct AccountSeed {
    pub(super) account_id: String,
    pub(super) readiness: Arc<dyn ReadinessPort>,
    pub(super) reconciliation: OkxReconciliationClient,
    pub(super) forbidden_observer: ForbiddenOrderObserver,
    pub(super) private_state_sessions: PrivateStateSessionFactory,
    pub(super) bound_order_gateway: Option<BoundRegularOrderGateway>,
    pub(super) safety: Option<Arc<dyn SafetyPort>>,
    pub(super) instrument_guard: ExchangeInstrumentGuard,
}

pub(super) async fn bootstrap_accounts(
    config: &LiveConfig,
    restored_orders: &HashMap<String, Vec<reap_core::OrderUpdate>>,
    mode: LiveMode,
    planned_order_accounts: &HashSet<String>,
    maintenance_relevance: &MaintenanceRelevancePlan,
) -> Result<
    (
        VerifiedBootstrap,
        Vec<AccountSeed>,
        HashMap<String, AccountBootstrapSnapshot>,
    ),
    LiveRuntimeError,
> {
    let mut snapshots = HashMap::new();
    let mut seeds = Vec::new();
    for account in &config.accounts {
        let connection = ConnectionSettings::new(
            config.venue.rest_url.clone(),
            config.venue.environment.is_demo(),
            Duration::from_millis(config.runtime.rest_connect_timeout_ms),
            Duration::from_millis(config.runtime.rest_request_timeout_ms),
        )
        .map_err(|error| LiveRuntimeError::Bootstrap {
            account_id: account.id.clone(),
            message: error.to_string(),
        })?;
        let credential_env = CredentialEnvNames::new(
            account.api_key_env.clone(),
            account.secret_key_env.clone(),
            account.passphrase_env.clone(),
        )
        .map_err(|error| LiveRuntimeError::Bootstrap {
            account_id: account.id.clone(),
            message: error.to_string(),
        })?;
        let trade_modes = account
            .trade_modes
            .iter()
            .map(|(symbol, mode)| (symbol.clone(), (*mode).into()))
            .collect();
        let (
            readiness,
            live_reconciliation,
            forbidden_observer,
            private_state_sessions,
            bound_order_gateway,
            safety,
        ) = match (mode, planned_order_accounts.contains(&account.id)) {
            (LiveMode::Observe, _) | (LiveMode::Demo, false) => {
                let mut roles = observe_from_env(
                    connection,
                    credential_env,
                    config.venue.enable_vip_fills_channel,
                )
                .map_err(|error| LiveRuntimeError::Bootstrap {
                    account_id: account.id.clone(),
                    message: error.to_string(),
                })?;
                let private_state_sessions =
                    roles.take_private_state_sessions().ok_or_else(|| {
                        LiveRuntimeError::Bootstrap {
                            account_id: account.id.clone(),
                            message: "private state session authority was already consumed"
                                .to_string(),
                        }
                    })?;
                (
                    Arc::new(roles.readiness()) as Arc<dyn ReadinessPort>,
                    roles.reconciliation(),
                    roles.forbidden_observer(),
                    private_state_sessions,
                    None,
                    None,
                )
            }
            (LiveMode::Demo, true) => {
                let mut roles = demo_from_env(
                    connection,
                    credential_env,
                    account.id.clone(),
                    config.venue.enable_vip_fills_channel,
                )
                .map_err(|error| LiveRuntimeError::Bootstrap {
                    account_id: account.id.clone(),
                    message: error.to_string(),
                })?;
                let reconciliation = roles.observe().reconciliation();
                let bound_order_gateway = roles
                    .take_bound_order_gateway(trade_modes, config.runtime.pacing_policy())
                    .map_err(|error| LiveRuntimeError::GatewaySetup {
                        account_id: account.id.clone(),
                        message: error.to_string(),
                    })?;
                let safety = roles
                    .take_safety()
                    .ok_or_else(|| LiveRuntimeError::GatewaySetup {
                        account_id: account.id.clone(),
                        message: "demo live-safety authority was already consumed".to_string(),
                    })?;
                let private_state_sessions =
                    roles.take_private_state_sessions().ok_or_else(|| {
                        LiveRuntimeError::Bootstrap {
                            account_id: account.id.clone(),
                            message: "private state session authority was already consumed"
                                .to_string(),
                        }
                    })?;
                (
                    Arc::new(roles.observe().readiness()) as Arc<dyn ReadinessPort>,
                    reconciliation,
                    roles.observe().forbidden_observer(),
                    private_state_sessions,
                    Some(bound_order_gateway),
                    Some(Arc::new(safety) as Arc<dyn SafetyPort>),
                )
            }
            (LiveMode::Validate, _) => {
                return Err(LiveRuntimeError::Bootstrap {
                    account_id: account.id.clone(),
                    message: "validate mode cannot construct network authority".to_string(),
                });
            }
        };
        let reconciliation = OkxReconciliationClient::new(
            Arc::new(live_reconciliation),
            config.runtime.pacing_policy(),
        );
        let clock_skew_ms = rest_clock_skew_ms(readiness.as_ref())
            .await
            .map_err(|error| bootstrap_error(&account.id, "exchange clock", error.to_string()))?;
        if clock_skew_ms > config.runtime.max_exchange_clock_skew_ms {
            return Err(bootstrap_error(
                &account.id,
                "exchange clock",
                format!(
                    "clock skew {clock_skew_ms}ms exceeds configured maximum {}ms",
                    config.runtime.max_exchange_clock_skew_ms
                ),
            ));
        }
        if seeds.is_empty() {
            let statuses = readiness
                .system_status()
                .await
                .map_err(|error| LiveRuntimeError::ExchangeStatusCheck(error.to_string()))?;
            if let Some(reason) = exchange_status_block_reason(
                &statuses,
                maintenance_relevance,
                unix_time_ms(),
                config.runtime.exchange_status_lead_ms,
            ) {
                return Err(LiveRuntimeError::ExchangeStatus(reason));
            }
        }
        let account_config = readiness
            .account_config()
            .await
            .map_err(|error| bootstrap_error(&account.id, "account config", error.to_string()))?;
        let balance_economics = readiness
            .account_balance_snapshot()
            .await
            .map_err(|error| bootstrap_error(&account.id, "account balance", error.to_string()))?;
        let balance = balance_economics.account_update();
        let position_risks = readiness
            .account_positions_snapshot(None, None)
            .await
            .map_err(|error| {
                bootstrap_error(&account.id, "account positions", error.to_string())
            })?;
        let positions = position_risks.account_update();
        let (mut open_orders, recent_fills) = reconciliation
            .fetch_remote_state(
                None,
                None,
                config.runtime.max_order_reconciliation_pages,
                config.runtime.max_fill_reconciliation_pages,
            )
            .await
            .map_err(|error| {
                bootstrap_error(
                    &account.id,
                    "open orders and recent fills",
                    error.to_string(),
                )
            })?;
        let mut remote_ids = open_orders
            .iter()
            .map(remote_order_id)
            .collect::<HashSet<_>>();
        for restored in restored_orders.get(&account.id).into_iter().flatten() {
            if remote_ids.contains(&restored.order_id) {
                continue;
            }
            let details = match reconciliation
                .fetch_order_details(&restored.symbol, &restored.order_id)
                .await
            {
                Ok(details) => details,
                Err(error)
                    if error.is_order_not_found()
                        && unix_time_ms().saturating_sub(restored.ts_ms)
                            < config.runtime.ambiguous_submit_grace_ms =>
                {
                    continue;
                }
                Err(error) if error.is_order_not_found() => RemoteOrder {
                    exchange_order_id: String::new(),
                    client_order_id: restored.order_id.clone(),
                    symbol: restored.symbol.clone(),
                    side: restored.side,
                    state: PrivateOrderState::Rejected,
                    price: restored.price,
                    qty: restored.qty,
                    cumulative_filled_qty: restored.filled_qty,
                    average_fill_price: restored.avg_fill_price,
                    update_time_ms: unix_time_ms(),
                },
                Err(error) => {
                    return Err(bootstrap_error(
                        &account.id,
                        &format!("order details {}", restored.order_id),
                        error.to_string(),
                    ));
                }
            };
            remote_ids.insert(remote_order_id(&details));
            open_orders.push(details);
        }
        let mut instruments = HashMap::new();
        for (index, instrument) in config.instruments_for_account(&account.id).enumerate() {
            if index > 0 {
                tokio::time::sleep(Duration::from_millis(
                    OKX_MIN_ACCOUNT_INSTRUMENT_REQUEST_INTERVAL_MS,
                ))
                .await;
            }
            let metadata = readiness
                .account_instrument(okx_instrument_type(instrument.kind), &instrument.symbol)
                .await
                .map_err(|error| {
                    bootstrap_error(
                        &account.id,
                        &format!("instrument {}", instrument.symbol),
                        error.to_string(),
                    )
                })?;
            instruments.insert(instrument.symbol.clone(), metadata);
        }
        let instrument_guard = ExchangeInstrumentGuard {
            sweep_interval_ms: config.runtime.exchange_fee_check_interval_ms,
            change_lead_ms: config.runtime.exchange_instrument_change_lead_ms,
            expectations: exchange_instrument_expectations(config, &account.id, &instruments)?,
        };
        verify_initial_exchange_instruments(&account.id, &instrument_guard, unix_time_ms())?;
        verify_initial_exchange_fees(&account.id, readiness.as_ref(), &instrument_guard).await?;
        snapshots.insert(
            account.id.clone(),
            AccountBootstrapSnapshot {
                account_config,
                instruments,
                balance_economics,
                position_risks,
                balance,
                positions,
                open_orders,
                recent_fills,
            },
        );
        seeds.push(AccountSeed {
            account_id: account.id.clone(),
            readiness,
            reconciliation,
            forbidden_observer,
            private_state_sessions,
            bound_order_gateway,
            safety,
            instrument_guard,
        });
    }
    let verified = verify_bootstrap(config, &snapshots)
        .map_err(|error| LiveRuntimeError::BootstrapVerification(error.to_string()))?;
    Ok((verified, seeds, snapshots))
}

fn bootstrap_error(account_id: &str, operation: &str, message: String) -> LiveRuntimeError {
    LiveRuntimeError::Bootstrap {
        account_id: account_id.to_string(),
        message: format!("{operation}: {message}"),
    }
}
