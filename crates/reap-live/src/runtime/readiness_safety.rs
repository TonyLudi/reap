use std::collections::HashMap;

use async_trait::async_trait;
use reap_okx_live_adapter::{LiveReadiness, LiveSafety};
use reap_venue::okx::{
    OkxInstrument, OkxInstrumentType, OkxSystemStatus, OkxTradeFeeRate, RestError,
};
use tokio::sync::mpsc;
use tokio::task::JoinHandle;

use super::{
    ForbiddenOrderEvent, HostGuardRuntime, HostHealthError, HostHealthSnapshot,
    MaintenanceRelevancePlan, SafetyTaskCommand,
};

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
