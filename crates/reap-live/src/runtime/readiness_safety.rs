use std::collections::HashMap;
use std::sync::Arc;
use std::time::Duration;

use async_trait::async_trait;
use reap_okx_live_adapter::{LiveReadiness, LiveSafety};
use reap_telemetry::{AlertEvent, AlertSeverity};
use reap_venue::okx::{
    OKX_MIN_TRADE_FEE_REQUEST_INTERVAL_MS, OkxInstrument, OkxInstrumentType, OkxSystemEnvironment,
    OkxSystemServiceType, OkxSystemStatus, OkxSystemStatusState, OkxTradeFeeRate, RestError,
    okx_capability_registration,
};
use tokio::sync::mpsc;
use tokio::task::JoinHandle;

use crate::forbidden_orders::ForbiddenOrderEvent;
use crate::{
    HostGuardRuntime, HostHealthError, HostHealthSnapshot, LiveConfig, MaintenanceRelevancePlan,
    MaintenanceServicePlan, TradingEnvironment, okx_instrument_type,
};

use super::dispatch::{RuntimeEvent, RuntimeTaskFailure, SafetyTaskCommand};
use super::{LiveRuntime, LiveRuntimeError, unix_time_ms};

pub(super) struct ReadinessSafetyState {
    pub(super) forbidden_rx: mpsc::Receiver<ForbiddenOrderEvent>,
    pub(super) safety_senders: HashMap<String, mpsc::Sender<SafetyTaskCommand>>,
    pub(super) safety_tasks: Vec<JoinHandle<()>>,
    pub(super) forbidden_tasks: Vec<JoinHandle<()>>,
    pub(super) readiness_timeout_ms: u64,
    pub(super) timer_interval_ms: u64,
    pub(super) host_guard: Option<HostGuardRuntime>,
    pub(super) host_failures: Option<mpsc::Receiver<HostHealthError>>,
    pub(super) host_preflight: Option<HostHealthSnapshot>,
    pub(super) host_checks: u64,
    pub(super) host_last_snapshot: Option<HostHealthSnapshot>,
}

impl LiveRuntime {
    pub(super) async fn handle_forbidden_order_event(
        &mut self,
        mut event: ForbiddenOrderEvent,
    ) -> Result<(), LiveRuntimeError> {
        event.expire_delayed_zero_proof(unix_time_ms());
        let alert = event.state.alert_code().map(|code| {
            let reason = event
                .state
                .failure_reason()
                .expect("nonzero forbidden state must have a failure reason");
            let mut alert = AlertEvent::new(
                AlertSeverity::Critical,
                "forbidden_order_sentinel",
                code,
                format!(
                    "account {}: {reason}; run the separate reap-emergency executable",
                    event.account_id
                ),
            )
            .with_attribute("account_id", &event.account_id);
            alert.ts_ms = event.observed_at_ms;
            alert
        });
        let output = self.coordinator.on_forbidden_order_event(event)?;
        // Canonical regular cancellation/reconciliation dispatch stays ahead of
        // telemetry work when the proof becomes invalid.
        self.commit_output(output).await?;
        if let Some(alert) = alert {
            self.emit_alert(alert)?;
        }
        Ok(())
    }
}

pub(super) async fn receive_host_failure(
    receiver: &mut Option<mpsc::Receiver<HostHealthError>>,
) -> Option<HostHealthError> {
    match receiver {
        Some(receiver) => receiver.recv().await,
        None => std::future::pending().await,
    }
}

#[async_trait]
pub(super) trait ReadinessPort: Send + Sync {
    async fn server_time_ms(&self) -> Result<u64, RestError> {
        Err(unimplemented_readiness("server time"))
    }
    async fn system_status(&self) -> Result<Vec<OkxSystemStatus>, RestError> {
        Err(unimplemented_readiness("system status"))
    }
    async fn account_config(&self) -> Result<reap_venue::okx::OkxAccountConfig, RestError> {
        Err(unimplemented_readiness("account config"))
    }
    async fn account_balance_snapshot(
        &self,
    ) -> Result<reap_venue::okx::OkxAccountBalanceSnapshot, RestError> {
        Err(unimplemented_readiness("account balance"))
    }
    async fn account_positions_snapshot(
        &self,
        instrument_type: Option<OkxInstrumentType>,
        symbol: Option<&str>,
    ) -> Result<reap_venue::okx::OkxAccountPositionsSnapshot, RestError> {
        let _ = (instrument_type, symbol);
        Err(unimplemented_readiness("account positions"))
    }
    async fn account_instrument(
        &self,
        instrument_type: OkxInstrumentType,
        symbol: &str,
    ) -> Result<OkxInstrument, RestError> {
        let _ = (instrument_type, symbol);
        Err(unimplemented_readiness("account instrument"))
    }
    async fn account_trade_fee(
        &self,
        instrument_type: OkxInstrumentType,
        instrument_id: Option<&str>,
        instrument_family: Option<&str>,
        group_id: &str,
    ) -> Result<OkxTradeFeeRate, RestError> {
        let _ = (instrument_type, instrument_id, instrument_family, group_id);
        Err(unimplemented_readiness("account trade fee"))
    }
}

fn unimplemented_readiness(operation: &str) -> RestError {
    RestError::Transport(format!("readiness fake did not implement {operation}"))
}

#[async_trait]
impl ReadinessPort for LiveReadiness {
    async fn server_time_ms(&self) -> Result<u64, RestError> {
        LiveReadiness::server_time_ms(self).await
    }

    async fn system_status(&self) -> Result<Vec<OkxSystemStatus>, RestError> {
        LiveReadiness::system_status(self).await
    }

    async fn account_config(&self) -> Result<reap_venue::okx::OkxAccountConfig, RestError> {
        LiveReadiness::account_config(self).await
    }

    async fn account_balance_snapshot(
        &self,
    ) -> Result<reap_venue::okx::OkxAccountBalanceSnapshot, RestError> {
        LiveReadiness::account_balance_snapshot(self).await
    }

    async fn account_positions_snapshot(
        &self,
        instrument_type: Option<OkxInstrumentType>,
        symbol: Option<&str>,
    ) -> Result<reap_venue::okx::OkxAccountPositionsSnapshot, RestError> {
        LiveReadiness::account_positions_snapshot(self, instrument_type, symbol).await
    }

    async fn account_instrument(
        &self,
        instrument_type: OkxInstrumentType,
        symbol: &str,
    ) -> Result<OkxInstrument, RestError> {
        LiveReadiness::account_instrument(self, instrument_type, symbol).await
    }

    async fn account_trade_fee(
        &self,
        instrument_type: OkxInstrumentType,
        instrument_id: Option<&str>,
        instrument_family: Option<&str>,
        group_id: &str,
    ) -> Result<OkxTradeFeeRate, RestError> {
        LiveReadiness::account_trade_fee(
            self,
            instrument_type,
            instrument_id,
            instrument_family,
            group_id,
        )
        .await
    }
}

#[async_trait]
pub(super) trait SafetyPort: Send + Sync {
    async fn cancel_all_after(&self, timeout_secs: u64) -> Result<(), RestError>;
}

#[async_trait]
impl SafetyPort for LiveSafety {
    async fn cancel_all_after(&self, timeout_secs: u64) -> Result<(), RestError> {
        LiveSafety::cancel_all_after(self, timeout_secs).await
    }
}

#[derive(Debug, Clone)]
pub(super) struct ExchangeStatusGuard {
    pub(super) enabled: bool,
    pub(super) relevance: MaintenanceRelevancePlan,
    pub(super) check_interval_ms: u64,
    pub(super) lead_ms: u64,
}

#[derive(Debug, Clone)]
pub(super) struct ExchangeInstrumentExpectation {
    pub(super) symbol: String,
    pub(super) instrument_type: OkxInstrumentType,
    pub(super) instrument_id: Option<String>,
    pub(super) instrument_family: Option<String>,
    pub(super) group_id: String,
    pub(super) configured_maker_cost: f64,
    pub(super) configured_taker_cost: f64,
    pub(super) expected_instrument: OkxInstrument,
}

#[derive(Debug, Clone)]
pub(super) struct ExchangeInstrumentGuard {
    pub(super) sweep_interval_ms: u64,
    pub(super) change_lead_ms: u64,
    pub(super) expectations: Vec<ExchangeInstrumentExpectation>,
}

pub(super) async fn rest_clock_skew_ms(client: &dyn ReadinessPort) -> Result<u64, RestError> {
    let before_ms = unix_time_ms();
    let exchange_ms = client.server_time_ms().await?;
    let after_ms = unix_time_ms();
    let midpoint_ms = before_ms.saturating_add(after_ms.saturating_sub(before_ms) / 2);
    Ok(midpoint_ms.abs_diff(exchange_ms))
}

pub(super) fn exchange_instrument_expectations(
    config: &LiveConfig,
    account_id: &str,
    instruments: &HashMap<String, OkxInstrument>,
) -> Result<Vec<ExchangeInstrumentExpectation>, LiveRuntimeError> {
    config
        .instruments_for_account(account_id)
        .map(|configured| {
            let metadata = instruments.get(&configured.symbol).ok_or_else(|| {
                LiveRuntimeError::ExchangeInstrumentCheck(format!(
                    "account {account_id} has no instrument metadata for {}",
                    configured.symbol
                ))
            })?;
            let expected_type = okx_instrument_type(configured.kind);
            if metadata.instrument_type != expected_type {
                return Err(LiveRuntimeError::ExchangeInstrumentDrift(format!(
                    "account {account_id} {} metadata type is {:?}, expected {:?}",
                    configured.symbol, metadata.instrument_type, expected_type
                )));
            }
            let group_id = metadata.trade_fee_group_id.trim();
            if group_id.is_empty() {
                return Err(LiveRuntimeError::ExchangeFeeCheck(format!(
                    "account {account_id} {} has no OKX trade-fee groupId",
                    configured.symbol
                )));
            }
            let (instrument_id, instrument_family) = match expected_type {
                OkxInstrumentType::Spot | OkxInstrumentType::Margin => {
                    (Some(configured.symbol.clone()), None)
                }
                OkxInstrumentType::Swap
                | OkxInstrumentType::Futures
                | OkxInstrumentType::Option => {
                    let family = metadata.instrument_family.trim();
                    if family.is_empty() {
                        return Err(LiveRuntimeError::ExchangeFeeCheck(format!(
                            "account {account_id} {} has no OKX instFamily for fee lookup",
                            configured.symbol
                        )));
                    }
                    (None, Some(family.to_string()))
                }
            };
            Ok(ExchangeInstrumentExpectation {
                symbol: configured.symbol.clone(),
                instrument_type: expected_type,
                instrument_id,
                instrument_family,
                group_id: group_id.to_string(),
                configured_maker_cost: configured.maker_fee,
                configured_taker_cost: configured.taker_fee,
                expected_instrument: metadata.clone(),
            })
        })
        .collect()
}

async fn fetch_exchange_fee(
    client: &dyn ReadinessPort,
    expectation: &ExchangeInstrumentExpectation,
) -> Result<OkxTradeFeeRate, RestError> {
    client
        .account_trade_fee(
            expectation.instrument_type,
            expectation.instrument_id.as_deref(),
            expectation.instrument_family.as_deref(),
            &expectation.group_id,
        )
        .await
}

pub(super) fn exchange_fee_drift_reason(
    expectation: &ExchangeInstrumentExpectation,
    rate: &OkxTradeFeeRate,
) -> Option<String> {
    const RATE_EPSILON: f64 = 1e-12;

    let maker_cost = rate.maker_cost_rate();
    let taker_cost = rate.taker_cost_rate();
    let maker_understated = expectation.configured_maker_cost + RATE_EPSILON < maker_cost;
    let taker_understated = expectation.configured_taker_cost + RATE_EPSILON < taker_cost;
    (maker_understated || taker_understated).then(|| {
        format!(
            "{} group {} level {} configured maker/taker costs {}/{} understate authenticated costs {}/{} at {}",
            expectation.symbol,
            rate.group_id,
            rate.level,
            expectation.configured_maker_cost,
            expectation.configured_taker_cost,
            maker_cost,
            taker_cost,
            rate.timestamp_ms
        )
    })
}

pub(super) fn exchange_instrument_drift_reason(
    expectation: &ExchangeInstrumentExpectation,
    current: &OkxInstrument,
    now_ms: u64,
    change_lead_ms: u64,
) -> Option<String> {
    let expected = &expectation.expected_instrument;
    if current.state != "live" {
        return Some(format!(
            "{} state changed from {:?} to {:?}",
            expectation.symbol, expected.state, current.state
        ));
    }

    macro_rules! check_field {
        ($name:literal, $expected:expr, $current:expr) => {
            if $expected != $current {
                return Some(format!(
                    "{} {} changed from {:?} to {:?}",
                    expectation.symbol, $name, $expected, $current
                ));
            }
        };
    }

    check_field!("symbol", expected.symbol, current.symbol);
    check_field!(
        "instrument type",
        expected.instrument_type,
        current.instrument_type
    );
    check_field!(
        "instrument family",
        expected.instrument_family,
        current.instrument_family
    );
    check_field!(
        "fee group",
        expected.trade_fee_group_id,
        current.trade_fee_group_id
    );
    check_field!("underlying", expected.underlying, current.underlying);
    check_field!(
        "base currency",
        expected.base_currency,
        current.base_currency
    );
    check_field!(
        "quote currency",
        expected.quote_currency,
        current.quote_currency
    );
    check_field!(
        "settle currency",
        expected.settle_currency,
        current.settle_currency
    );
    check_field!(
        "contract type",
        expected.contract_type,
        current.contract_type
    );
    check_field!(
        "contract value",
        expected.contract_value,
        current.contract_value
    );
    check_field!(
        "contract value currency",
        expected.contract_value_currency,
        current.contract_value_currency
    );
    check_field!("tick size", expected.tick_size, current.tick_size);
    check_field!("lot size", expected.lot_size, current.lot_size);
    check_field!("minimum size", expected.min_size, current.min_size);
    check_field!(
        "maximum limit-order size",
        expected.max_limit_size,
        current.max_limit_size
    );
    check_field!(
        "maximum market-order size",
        expected.max_market_size,
        current.max_market_size
    );
    check_field!(
        "maximum limit-order amount",
        expected.max_limit_amount_usd,
        current.max_limit_amount_usd
    );
    check_field!(
        "maximum market-order amount",
        expected.max_market_amount_usd,
        current.max_market_amount_usd
    );

    let cutoff_ms = now_ms.saturating_add(change_lead_ms);
    current
        .upcoming_changes
        .iter()
        .filter(|change| change.effective_time_ms <= cutoff_ms)
        .min_by_key(|change| change.effective_time_ms)
        .map(|change| {
            format!(
                "{} announced {} change to {} effective at {} inside the {}ms guard lead",
                expectation.symbol,
                change.parameter.as_okx_str(),
                change.new_value,
                change.effective_time_ms,
                change_lead_ms
            )
        })
}

pub(super) fn verify_initial_exchange_instruments(
    account_id: &str,
    guard: &ExchangeInstrumentGuard,
    now_ms: u64,
) -> Result<(), LiveRuntimeError> {
    for expectation in &guard.expectations {
        if let Some(reason) = exchange_instrument_drift_reason(
            expectation,
            &expectation.expected_instrument,
            now_ms,
            guard.change_lead_ms,
        ) {
            return Err(LiveRuntimeError::ExchangeInstrumentDrift(format!(
                "account {account_id}: {reason}"
            )));
        }
    }
    Ok(())
}

pub(super) async fn verify_initial_exchange_fees(
    account_id: &str,
    client: &dyn ReadinessPort,
    guard: &ExchangeInstrumentGuard,
) -> Result<(), LiveRuntimeError> {
    for (index, expectation) in guard.expectations.iter().enumerate() {
        if index > 0 {
            tokio::time::sleep(Duration::from_millis(OKX_MIN_TRADE_FEE_REQUEST_INTERVAL_MS)).await;
        }
        let rate = fetch_exchange_fee(client, expectation)
            .await
            .map_err(|error| {
                LiveRuntimeError::ExchangeFeeCheck(format!(
                    "account {account_id} {}: {error}",
                    expectation.symbol
                ))
            })?;
        if let Some(reason) = exchange_fee_drift_reason(expectation, &rate) {
            return Err(LiveRuntimeError::ExchangeFeeDrift(format!(
                "account {account_id}: {reason}"
            )));
        }
    }
    Ok(())
}

pub(super) fn exchange_status_block_reason(
    statuses: &[OkxSystemStatus],
    relevance: &MaintenanceRelevancePlan,
    now_ms: u64,
    lead_ms: u64,
) -> Option<String> {
    let _maintenance_capability = okx_capability_registration("OKX-MAINTENANCE-FILTER")
        .expect("maintenance filter must remain in the OKX capability registry");
    let expected_environment = match relevance.environment() {
        TradingEnvironment::Demo => OkxSystemEnvironment::Demo,
        TradingEnvironment::Production => OkxSystemEnvironment::Production,
    };
    statuses.iter().find_map(|status| {
        let planned_service = match status.service_type {
            OkxSystemServiceType::WebSocket => Some(MaintenanceServicePlan::Websocket),
            OkxSystemServiceType::Trading => Some(MaintenanceServicePlan::Trading),
            OkxSystemServiceType::TradingAccounts => Some(MaintenanceServicePlan::TradingAccounts),
            OkxSystemServiceType::TradingProducts => Some(MaintenanceServicePlan::TradingProducts),
            OkxSystemServiceType::Other => Some(MaintenanceServicePlan::OtherAmbiguous),
            OkxSystemServiceType::BlockTrading
            | OkxSystemServiceType::TradingBot
            | OkxSystemServiceType::SpreadTrading
            | OkxSystemServiceType::CopyTrading => None,
        };
        let plan_relevant_service =
            planned_service.is_some_and(|service| relevance.services().contains(&service));
        let inside_guard_window = match status.state {
            OkxSystemStatusState::Scheduled => {
                status.begin_time_ms <= now_ms.saturating_add(lead_ms)
            }
            OkxSystemStatusState::Ongoing | OkxSystemStatusState::PreOpen => true,
            OkxSystemStatusState::Completed | OkxSystemStatusState::Canceled => false,
        };
        (relevance.unified_system()
            && status.system.eq_ignore_ascii_case("unified")
            && status.environment == expected_environment
            && plan_relevant_service
            && inside_guard_window)
            .then(|| {
                format!(
                    "{:?} {:?} maintenance {:?} from {} to {} ({}ms lead): {}",
                    status.environment,
                    status.service_type,
                    status.state,
                    status.begin_time_ms,
                    status.end_time_ms,
                    lead_ms,
                    status.title.trim()
                )
            })
    })
}

async fn run_exchange_instrument_guard(
    account_id: String,
    client: Arc<dyn ReadinessPort>,
    guard: ExchangeInstrumentGuard,
) -> RuntimeTaskFailure {
    if guard.expectations.is_empty() {
        return std::future::pending::<RuntimeTaskFailure>().await;
    }
    let request_interval_ms =
        exchange_fee_request_interval_ms(guard.sweep_interval_ms, guard.expectations.len());
    let mut interval = tokio::time::interval(Duration::from_millis(request_interval_ms));
    interval.set_missed_tick_behavior(tokio::time::MissedTickBehavior::Delay);
    interval.tick().await;
    let mut next = 0;

    loop {
        interval.tick().await;
        let expectation = &guard.expectations[next];
        let instrument = match client
            .account_instrument(expectation.instrument_type, &expectation.symbol)
            .await
        {
            Ok(instrument) => instrument,
            Err(error) => {
                return RuntimeTaskFailure::ExchangeInstrumentCheck(format!(
                    "account {account_id} {}: {error}",
                    expectation.symbol
                ));
            }
        };
        if let Some(reason) = exchange_instrument_drift_reason(
            expectation,
            &instrument,
            unix_time_ms(),
            guard.change_lead_ms,
        ) {
            return RuntimeTaskFailure::ExchangeInstrumentDrift(format!(
                "account {account_id}: {reason}"
            ));
        }
        let rate = match fetch_exchange_fee(client.as_ref(), expectation).await {
            Ok(rate) => rate,
            Err(error) => {
                return RuntimeTaskFailure::ExchangeFeeCheck(format!(
                    "account {account_id} {}: {error}",
                    expectation.symbol
                ));
            }
        };
        if let Some(reason) = exchange_fee_drift_reason(expectation, &rate) {
            return RuntimeTaskFailure::ExchangeFeeDrift(format!("account {account_id}: {reason}"));
        }
        next = (next + 1) % guard.expectations.len();
    }
}

pub(super) fn exchange_fee_request_interval_ms(
    sweep_interval_ms: u64,
    instrument_count: usize,
) -> u64 {
    debug_assert!(instrument_count > 0);
    (sweep_interval_ms / instrument_count as u64).max(OKX_MIN_TRADE_FEE_REQUEST_INTERVAL_MS)
}

#[allow(clippy::too_many_arguments)]
pub(super) async fn run_account_safety_task(
    account_id: String,
    readiness: Arc<dyn ReadinessPort>,
    safety: Option<Arc<dyn SafetyPort>>,
    expected_account_config: reap_venue::okx::OkxAccountConfig,
    mut commands: mpsc::Receiver<SafetyTaskCommand>,
    events: mpsc::Sender<RuntimeEvent>,
    mut deadman_timeout_secs: Option<u64>,
    deadman_heartbeat_ms: u64,
    clock_check_interval_ms: u64,
    max_clock_skew_ms: u64,
    exchange_status_guard: ExchangeStatusGuard,
    exchange_instrument_guard: ExchangeInstrumentGuard,
) {
    let mut instrument_task = tokio::spawn(run_exchange_instrument_guard(
        account_id.clone(),
        readiness.clone(),
        exchange_instrument_guard,
    ));
    let mut deadman = tokio::time::interval(Duration::from_millis(deadman_heartbeat_ms));
    deadman.set_missed_tick_behavior(tokio::time::MissedTickBehavior::Delay);
    deadman.tick().await;
    let mut clock = tokio::time::interval(Duration::from_millis(clock_check_interval_ms));
    clock.set_missed_tick_behavior(tokio::time::MissedTickBehavior::Delay);
    clock.tick().await;
    let mut exchange_status = tokio::time::interval(Duration::from_millis(
        exchange_status_guard.check_interval_ms,
    ));
    exchange_status.set_missed_tick_behavior(tokio::time::MissedTickBehavior::Delay);
    exchange_status.tick().await;

    let terminal_failure = loop {
        tokio::select! {
            instrument_result = &mut instrument_task => {
                break Some(match instrument_result {
                    Ok(failure) => failure,
                    Err(error) => RuntimeTaskFailure::ExchangeInstrumentCheck(format!(
                        "account {account_id} instrument/fee guard task failed: {error}"
                    )),
                });
            }
            command = commands.recv() => {
                let Some(command) = command else { break None; };
                match command {
                    SafetyTaskCommand::DisableDeadMan { result } => {
                        let disabled = match (deadman_timeout_secs, safety.as_ref()) {
                            (Some(_), Some(safety)) => safety.cancel_all_after(0).await.map_err(|error| error.to_string()),
                            (None, _) => Ok(()),
                            (Some(_), None) => Err("dead-man authority is absent".to_string()),
                        };
                        if disabled.is_ok() {
                            deadman_timeout_secs = None;
                        }
                        let _ = result.send(disabled);
                    }
                    SafetyTaskCommand::Shutdown => break None,
                }
            }
            _ = deadman.tick(), if deadman_timeout_secs.is_some() => {
                let timeout_secs = deadman_timeout_secs.expect("guarded dead-man timeout");
                let Some(safety) = safety.as_ref() else {
                    break Some(RuntimeTaskFailure::DeadmanHeartbeat(format!(
                        "account {account_id}: dead-man authority is absent"
                    )));
                };
                if let Err(error) = safety.cancel_all_after(timeout_secs).await {
                    break Some(RuntimeTaskFailure::DeadmanHeartbeat(format!(
                        "account {account_id}: {error}"
                    )));
                }
            }
            _ = clock.tick() => {
                match rest_clock_skew_ms(readiness.as_ref()).await {
                    Ok(skew_ms) if skew_ms <= max_clock_skew_ms => {}
                    Ok(skew_ms) => {
                        break Some(RuntimeTaskFailure::ExchangeClockSkew(format!(
                            "account {account_id} observed {skew_ms}ms; maximum is {max_clock_skew_ms}ms"
                        )));
                    }
                    Err(error) => {
                        break Some(RuntimeTaskFailure::ExchangeClockCheck(format!(
                            "account {account_id}: {error}"
                        )));
                    }
                }
                match readiness.account_config().await {
                    Ok(current) if current == expected_account_config => {}
                    Ok(_) => {
                        break Some(RuntimeTaskFailure::AccountConfigDrift(format!(
                            "account {account_id} configuration or authenticated identity differs from bootstrap"
                        )));
                    }
                    Err(error) => {
                        break Some(RuntimeTaskFailure::AccountConfigCheck(format!(
                            "account {account_id}: {error}"
                        )));
                    }
                }
            }
            _ = exchange_status.tick(), if exchange_status_guard.enabled => {
                match readiness.system_status().await {
                    Ok(statuses) => {
                        if let Some(reason) = exchange_status_block_reason(
                            &statuses,
                            &exchange_status_guard.relevance,
                            unix_time_ms(),
                            exchange_status_guard.lead_ms,
                        ) {
                            break Some(RuntimeTaskFailure::ExchangeStatus(reason));
                        }
                    }
                    Err(error) => {
                        break Some(RuntimeTaskFailure::ExchangeStatusCheck(error.to_string()));
                    }
                }
            }
        }
    };

    if !instrument_task.is_finished() {
        instrument_task.abort();
        let _ = instrument_task.await;
    }
    if let Some(failure) = terminal_failure {
        let _ = events.send(RuntimeEvent::Fatal(failure)).await;
    }
}
