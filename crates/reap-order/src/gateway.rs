use std::collections::HashMap;
use std::sync::Arc;

use async_trait::async_trait;
use reap_core::{AccountUpdate, NewOrder};
use reap_venue::okx::{
    OkxFillPage, OkxFillPagination, OkxOrderAck, OkxRegularOrderPage, OkxRegularOrderPagination,
    OkxTradeMode, RestError,
};
use serde::{Deserialize, Serialize};
use thiserror::Error;

use crate::authority::{RegularApprovalBinding, RegularApprovalScope};
use crate::{
    ApprovedRegularCancel, CancelOrderTransportError, IdempotencyError, IdempotencyRegistry,
    OrderTransportError, PacingPolicy, PrivateStateReducer, ReconciliationSnapshot, RequestKind,
    RequestPacer, Reservation, ReservedRegularSubmit, reconcile_full_state,
};

#[derive(Debug, Error)]
pub enum GatewayError {
    #[error("OKX gateway request failed: {0}")]
    Rest(#[from] RestError),
    #[error("OKX order transport failed: {0}")]
    OrderTransport(#[from] OrderTransportError),
    #[error("idempotency check failed: {0}")]
    Idempotency(#[from] IdempotencyError),
    #[error("no OKX trade mode configured for {0}")]
    MissingTradeMode(String),
    #[error("regular order gateway account id must not be empty")]
    EmptyAccountId,
    #[error("regular approval scope for account {0} was already taken")]
    ApprovalScopeTaken(String),
    #[error("regular command dispatcher for account {0} was already taken")]
    CommandDispatcherTaken(String),
    #[error("regular approval belongs to account {actual}, expected {expected}")]
    ApprovalAccountMismatch { expected: String, actual: String },
    #[error("regular approval scope does not belong to gateway account {0}")]
    ApprovalScopeMismatch(String),
    #[error(
        "regular order acknowledgement client id {actual:?} does not match expected {expected:?}"
    )]
    AcknowledgementClientIdMismatch { expected: String, actual: String },
    #[error("recoverable pre-send cancel identity {actual:?} does not match original {expected:?}")]
    CancelFallbackIdentityMismatch { expected: String, actual: String },
    #[error("OKX account reconciliation returned no balance rows")]
    EmptyAccountBalance,
}

impl GatewayError {
    pub fn is_ambiguous(&self) -> bool {
        matches!(self, Self::Rest(RestError::Transport(_)))
            || matches!(self, Self::OrderTransport(error) if error.is_ambiguous())
            || matches!(self, Self::AcknowledgementClientIdMismatch { .. })
    }

    pub fn is_order_not_found(&self) -> bool {
        matches!(self, Self::Rest(error) if error.is_order_not_found())
            || matches!(
                self,
                Self::OrderTransport(OrderTransportError::Rejected { code, .. })
                    if code == "51603"
            )
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum SubmitOutcome {
    Submitted {
        client_order_id: String,
        exchange_order_id: String,
    },
    Duplicate {
        client_order_id: String,
        exchange_order_id: String,
    },
    PendingReconciliation {
        client_order_id: String,
    },
}

#[derive(Debug)]
pub struct PreparedRegularSubmit {
    account_id: String,
    binding: RegularApprovalBinding,
    idempotency_key: String,
    client_order_id: String,
    order: NewOrder,
    trade_mode: OkxTradeMode,
}

impl PreparedRegularSubmit {
    pub fn account_id(&self) -> &str {
        &self.account_id
    }

    pub fn client_order_id(&self) -> &str {
        &self.client_order_id
    }

    pub fn order(&self) -> &NewOrder {
        &self.order
    }

    pub fn trade_mode(&self) -> OkxTradeMode {
        self.trade_mode
    }
}

#[derive(Debug)]
pub struct PreparedRegularCancel {
    account_id: String,
    binding: RegularApprovalBinding,
    symbol: String,
    client_order_id: String,
    reason: String,
}

impl PreparedRegularCancel {
    pub fn account_id(&self) -> &str {
        &self.account_id
    }

    pub fn symbol(&self) -> &str {
        &self.symbol
    }

    pub fn client_order_id(&self) -> &str {
        &self.client_order_id
    }

    pub fn reason(&self) -> &str {
        &self.reason
    }
}

#[derive(Debug)]
pub enum SubmitPreparation {
    Ready(PreparedRegularSubmit),
    Complete(SubmitOutcome),
}

#[derive(Debug)]
pub struct RegularSubmitCompletion {
    account_id: String,
    binding: RegularApprovalBinding,
    idempotency_key: String,
    client_order_id: String,
    result: Result<OkxOrderAck, GatewayError>,
}

impl RegularSubmitCompletion {
    pub fn client_order_id(&self) -> &str {
        &self.client_order_id
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct CancelOutcome {
    pub client_order_id: String,
    pub exchange_order_id: String,
}

#[async_trait]
pub trait RegularExecution: Send + Sync {
    async fn place_regular_order(
        &self,
        order: PreparedRegularSubmit,
    ) -> Result<OkxOrderAck, OrderTransportError>;

    async fn cancel_regular_order(
        &self,
        cancel: PreparedRegularCancel,
    ) -> Result<OkxOrderAck, CancelOrderTransportError>;

    async fn cancel_regular_order_via_rest(
        &self,
        cancel: PreparedRegularCancel,
    ) -> Result<OkxOrderAck, OrderTransportError>;
}

#[async_trait]
pub trait RegularReconciliation: Send + Sync {
    async fn regular_pending_orders_page(
        &self,
        instrument_type: Option<&str>,
        symbol: Option<&str>,
        after: Option<&str>,
    ) -> Result<OkxRegularOrderPage, RestError>;

    async fn recent_fills_page(
        &self,
        instrument_type: Option<&str>,
        symbol: Option<&str>,
        after: Option<&str>,
    ) -> Result<OkxFillPage, RestError>;

    async fn account_balance(&self) -> Result<AccountUpdate, RestError>;

    async fn account_positions(&self) -> Result<AccountUpdate, RestError>;

    async fn order_details(
        &self,
        symbol: &str,
        client_order_id: &str,
    ) -> Result<reap_venue::RemoteOrder, RestError>;

    async fn server_time_ms(&self) -> Result<u64, RestError>;
}

pub struct OkxOrderGateway {
    account_id: String,
    binding: RegularApprovalBinding,
    approval_scope: Option<RegularApprovalScope>,
    dispatcher: Option<RegularCommandDispatcher>,
    reconciliation: OkxReconciliationClient,
    idempotency: IdempotencyRegistry,
    trade_modes: HashMap<String, OkxTradeMode>,
    local_orders: HashMap<String, NewOrder>,
}

pub struct RegularCommandDispatcher {
    account_id: String,
    binding: RegularApprovalBinding,
    execution: Arc<dyn RegularExecution>,
    pacer: RequestPacer,
}

#[derive(Clone)]
pub struct OkxReconciliationClient {
    reconciliation: Arc<dyn RegularReconciliation>,
    pacer: RequestPacer,
}

impl OkxOrderGateway {
    pub fn new(
        account_id: impl Into<String>,
        execution: Box<dyn RegularExecution>,
        reconciliation: Arc<dyn RegularReconciliation>,
        trade_modes: HashMap<String, OkxTradeMode>,
        pacing: PacingPolicy,
    ) -> Result<Self, GatewayError> {
        let account_id = account_id.into();
        if account_id.trim().is_empty() {
            return Err(GatewayError::EmptyAccountId);
        }
        let binding = RegularApprovalBinding::new();
        let execution = Arc::from(execution);
        let reconciliation = OkxReconciliationClient {
            reconciliation,
            pacer: RequestPacer::new(pacing.clone()),
        };
        Ok(Self {
            account_id: account_id.clone(),
            binding: binding.clone(),
            approval_scope: Some(RegularApprovalScope::new(
                account_id.clone(),
                binding.clone(),
            )),
            dispatcher: Some(RegularCommandDispatcher {
                account_id,
                binding,
                execution,
                pacer: RequestPacer::new(pacing),
            }),
            reconciliation,
            idempotency: IdempotencyRegistry::default(),
            trade_modes,
            local_orders: HashMap::new(),
        })
    }

    pub fn take_approval_scope(&mut self) -> Result<RegularApprovalScope, GatewayError> {
        self.approval_scope
            .take()
            .ok_or_else(|| GatewayError::ApprovalScopeTaken(self.account_id.clone()))
    }

    pub fn account_id(&self) -> &str {
        &self.account_id
    }

    pub fn reconciliation_client(&self) -> OkxReconciliationClient {
        self.reconciliation.clone()
    }

    pub fn take_command_dispatcher(&mut self) -> Result<RegularCommandDispatcher, GatewayError> {
        self.dispatcher
            .take()
            .ok_or_else(|| GatewayError::CommandDispatcherTaken(self.account_id.clone()))
    }

    pub fn register_local_order(
        &self,
        client_order_id: &str,
        state: &mut PrivateStateReducer,
    ) -> bool {
        let Some(order) = self.local_orders.get(client_order_id) else {
            return false;
        };
        state.register_local_order(client_order_id, order.clone());
        true
    }

    pub fn forget_local_order(&mut self, client_order_id: &str) {
        self.local_orders.remove(client_order_id);
    }

    pub async fn submit(
        &mut self,
        idempotency_key: impl Into<String>,
        approved: ReservedRegularSubmit,
    ) -> Result<SubmitOutcome, GatewayError> {
        match self.prepare_submit(idempotency_key, approved)? {
            SubmitPreparation::Ready(prepared) => self.execute_submit(prepared).await,
            SubmitPreparation::Complete(outcome) => Ok(outcome),
        }
    }

    pub async fn submit_registered(
        &mut self,
        idempotency_key: impl Into<String>,
        approved: ReservedRegularSubmit,
        state: &mut PrivateStateReducer,
    ) -> Result<SubmitOutcome, GatewayError> {
        match self.prepare_submit(idempotency_key, approved)? {
            SubmitPreparation::Ready(prepared) => {
                let client_order_id = prepared.client_order_id.clone();
                state.register_local_order(&client_order_id, prepared.order.clone());
                let result = self.execute_submit(prepared).await;
                if result.is_err() && !self.local_orders.contains_key(&client_order_id) {
                    state.remove_local_order(&client_order_id);
                }
                result
            }
            SubmitPreparation::Complete(outcome) => {
                let client_order_id = match &outcome {
                    SubmitOutcome::Duplicate {
                        client_order_id, ..
                    }
                    | SubmitOutcome::PendingReconciliation { client_order_id } => client_order_id,
                    SubmitOutcome::Submitted { .. } => {
                        unreachable!("submitted outcomes require new order execution")
                    }
                };
                self.register_local_order(client_order_id, state);
                Ok(outcome)
            }
        }
    }

    pub fn prepare_submit(
        &mut self,
        idempotency_key: impl Into<String>,
        approved: ReservedRegularSubmit,
    ) -> Result<SubmitPreparation, GatewayError> {
        self.validate_submit_authority(&approved)?;
        let symbol = approved.order().symbol.clone();
        let trade_mode = self
            .trade_modes
            .get(&symbol)
            .copied()
            .ok_or(GatewayError::MissingTradeMode(symbol))?;
        self.prepare_submit_with_id(idempotency_key, approved, trade_mode)
    }

    fn prepare_submit_with_id(
        &mut self,
        idempotency_key: impl Into<String>,
        approved: ReservedRegularSubmit,
        trade_mode: OkxTradeMode,
    ) -> Result<SubmitPreparation, GatewayError> {
        let idempotency_key = idempotency_key.into();
        let (account_id, generated_id, order, binding) = approved.into_parts();
        let reservation =
            self.idempotency
                .reserve(idempotency_key.clone(), &order, generated_id)?;
        let client_order_id = match reservation {
            Reservation::Accepted {
                client_order_id,
                exchange_order_id,
            } => {
                return Ok(SubmitPreparation::Complete(SubmitOutcome::Duplicate {
                    client_order_id,
                    exchange_order_id,
                }));
            }
            Reservation::Pending { client_order_id } => {
                return Ok(SubmitPreparation::Complete(
                    SubmitOutcome::PendingReconciliation { client_order_id },
                ));
            }
            Reservation::New { client_order_id } => client_order_id,
        };
        self.local_orders
            .insert(client_order_id.clone(), order.clone());
        Ok(SubmitPreparation::Ready(PreparedRegularSubmit {
            account_id,
            binding,
            idempotency_key,
            client_order_id,
            order,
            trade_mode,
        }))
    }

    fn validate_submit_authority(
        &self,
        approved: &ReservedRegularSubmit,
    ) -> Result<(), GatewayError> {
        self.validate_authority(approved.account_id(), approved.binding())
    }

    fn validate_cancel_authority(
        &self,
        approved: &ApprovedRegularCancel,
    ) -> Result<(), GatewayError> {
        self.validate_authority(approved.account_id(), approved.binding())
    }

    fn validate_authority(
        &self,
        account_id: &str,
        binding: &RegularApprovalBinding,
    ) -> Result<(), GatewayError> {
        if account_id != self.account_id {
            return Err(GatewayError::ApprovalAccountMismatch {
                expected: self.account_id.clone(),
                actual: account_id.to_string(),
            });
        }
        if !self.binding.matches(binding) {
            return Err(GatewayError::ApprovalScopeMismatch(self.account_id.clone()));
        }
        Ok(())
    }

    pub async fn execute_submit(
        &mut self,
        prepared: PreparedRegularSubmit,
    ) -> Result<SubmitOutcome, GatewayError> {
        let dispatcher = self
            .dispatcher
            .as_ref()
            .ok_or_else(|| GatewayError::CommandDispatcherTaken(self.account_id.clone()))?;
        let completion = dispatcher.place_prepared(prepared).await;
        self.finish_submit(completion)
    }

    pub fn finish_submit(
        &mut self,
        completion: RegularSubmitCompletion,
    ) -> Result<SubmitOutcome, GatewayError> {
        self.validate_authority(&completion.account_id, &completion.binding)?;
        let RegularSubmitCompletion {
            account_id: _,
            binding: _,
            idempotency_key,
            client_order_id,
            result,
        } = completion;
        let ack = match result {
            Ok(ack) => ack,
            Err(error) => {
                if !error.is_ambiguous() {
                    self.idempotency.release_pending(&idempotency_key)?;
                    self.local_orders.remove(&client_order_id);
                }
                return Err(error);
            }
        };
        self.idempotency
            .mark_accepted(&idempotency_key, ack.exchange_order_id.clone())?;
        Ok(SubmitOutcome::Submitted {
            client_order_id,
            exchange_order_id: ack.exchange_order_id,
        })
    }

    pub fn resolve_pending_submit(
        &mut self,
        idempotency_key: &str,
        exchange_order_id: impl Into<String>,
    ) -> Result<(), GatewayError> {
        self.idempotency
            .mark_accepted(idempotency_key, exchange_order_id)?;
        Ok(())
    }

    pub fn prepare_cancel(
        &self,
        approved: ApprovedRegularCancel,
    ) -> Result<PreparedRegularCancel, GatewayError> {
        self.validate_cancel_authority(&approved)?;
        let (account_id, symbol, client_order_id, reason, binding) = approved.into_parts();
        Ok(PreparedRegularCancel {
            account_id,
            binding,
            symbol,
            client_order_id,
            reason,
        })
    }

    pub async fn cancel(
        &self,
        approved: ApprovedRegularCancel,
    ) -> Result<CancelOutcome, GatewayError> {
        let prepared = self.prepare_cancel(approved)?;
        let dispatcher = self
            .dispatcher
            .as_ref()
            .ok_or_else(|| GatewayError::CommandDispatcherTaken(self.account_id.clone()))?;
        dispatcher.cancel_prepared(prepared).await
    }

    pub async fn reconcile_state(
        &self,
        state: &PrivateStateReducer,
        instrument_type: Option<&str>,
        symbol: Option<&str>,
        max_order_pages: usize,
        max_fill_pages: usize,
    ) -> Result<ReconciliationSnapshot, GatewayError> {
        let (remote_orders, remote_fills) = self
            .fetch_remote_state(instrument_type, symbol, max_order_pages, max_fill_pages)
            .await?;
        let remote_account = self.fetch_remote_account_state().await?;
        let report = reconcile_full_state(state, &remote_orders, &remote_fills, &remote_account);
        Ok(ReconciliationSnapshot {
            remote_orders,
            remote_fills,
            remote_account,
            report,
        })
    }

    pub async fn fetch_remote_state(
        &self,
        instrument_type: Option<&str>,
        symbol: Option<&str>,
        max_order_pages: usize,
        max_fill_pages: usize,
    ) -> Result<(Vec<reap_venue::RemoteOrder>, Vec<reap_venue::RemoteFill>), GatewayError> {
        self.reconciliation
            .fetch_remote_state(instrument_type, symbol, max_order_pages, max_fill_pages)
            .await
    }

    pub async fn fetch_remote_account_state(&self) -> Result<AccountUpdate, GatewayError> {
        self.reconciliation.fetch_remote_account_state().await
    }

    pub async fn fetch_order_details(
        &self,
        symbol: &str,
        client_order_id: &str,
    ) -> Result<reap_venue::RemoteOrder, GatewayError> {
        self.reconciliation
            .fetch_order_details(symbol, client_order_id)
            .await
    }

    pub async fn exchange_time_ms(&self) -> Result<u64, GatewayError> {
        self.reconciliation.exchange_time_ms().await
    }
}

impl RegularCommandDispatcher {
    pub async fn place_prepared(&self, prepared: PreparedRegularSubmit) -> RegularSubmitCompletion {
        let account_id = prepared.account_id.clone();
        let binding = prepared.binding.clone();
        let idempotency_key = prepared.idempotency_key.clone();
        let client_order_id = prepared.client_order_id.clone();
        let result = match self.validate_prepared_submit(&prepared) {
            Err(error) => Err(error),
            Ok(()) => {
                self.pacer.pace(RequestKind::Submit, "account").await;
                self.pacer
                    .pace(RequestKind::Submit, &prepared.order.symbol)
                    .await;
                self.execution
                    .place_regular_order(prepared)
                    .await
                    .and_then(|mut acknowledgement| {
                        validate_acknowledgement_client_id(
                            &client_order_id,
                            &acknowledgement.client_order_id,
                        )
                        .map_err(|error| match error {
                            GatewayError::AcknowledgementClientIdMismatch {
                                expected,
                                actual,
                            } => OrderTransportError::Ambiguous(format!(
                                "acknowledgement client id {actual:?} does not match expected {expected:?}"
                            )),
                            error => OrderTransportError::Ambiguous(error.to_string()),
                        })?;
                        acknowledgement.client_order_id = client_order_id.clone();
                        Ok(acknowledgement)
                    })
                    .map_err(GatewayError::from)
            }
        };
        RegularSubmitCompletion {
            account_id,
            binding,
            idempotency_key,
            client_order_id,
            result,
        }
    }

    pub async fn cancel_prepared(
        &self,
        prepared: PreparedRegularCancel,
    ) -> Result<CancelOutcome, GatewayError> {
        self.validate_prepared_cancel(&prepared)?;
        let expected_account_id = prepared.account_id.clone();
        let expected_symbol = prepared.symbol.clone();
        let expected_client_order_id = prepared.client_order_id.clone();
        let expected_reason = prepared.reason.clone();
        self.pacer
            .pace(RequestKind::Cancel, prepared.symbol())
            .await;
        let ack = match self.execution.cancel_regular_order(prepared).await {
            Ok(ack) => ack,
            Err(error) => {
                let (error, prepared) = error.into_parts();
                match prepared {
                    Some(prepared) => {
                        self.validate_prepared_cancel(&prepared)?;
                        let identity_matches = prepared.account_id == expected_account_id
                            && prepared.symbol == expected_symbol
                            && prepared.client_order_id == expected_client_order_id
                            && prepared.reason == expected_reason;
                        let expected = format!(
                            "{expected_account_id}/{expected_symbol}/{expected_client_order_id}/{expected_reason}"
                        );
                        let actual = format!(
                            "{}/{}/{}/{}",
                            prepared.account_id,
                            prepared.symbol,
                            prepared.client_order_id,
                            prepared.reason
                        );
                        if !identity_matches {
                            return Err(GatewayError::CancelFallbackIdentityMismatch {
                                expected,
                                actual,
                            });
                        }
                        self.execution
                            .cancel_regular_order_via_rest(prepared)
                            .await?
                    }
                    None => return Err(error.into()),
                }
            }
        };
        validate_acknowledgement_client_id(&expected_client_order_id, &ack.client_order_id)?;
        Ok(CancelOutcome {
            client_order_id: expected_client_order_id,
            exchange_order_id: ack.exchange_order_id,
        })
    }

    fn validate_prepared_submit(
        &self,
        prepared: &PreparedRegularSubmit,
    ) -> Result<(), GatewayError> {
        self.validate_prepared(prepared.account_id(), &prepared.binding)
    }

    fn validate_prepared_cancel(
        &self,
        prepared: &PreparedRegularCancel,
    ) -> Result<(), GatewayError> {
        self.validate_prepared(prepared.account_id(), &prepared.binding)
    }

    fn validate_prepared(
        &self,
        account_id: &str,
        binding: &RegularApprovalBinding,
    ) -> Result<(), GatewayError> {
        if account_id != self.account_id {
            return Err(GatewayError::ApprovalAccountMismatch {
                expected: self.account_id.clone(),
                actual: account_id.to_string(),
            });
        }
        if !self.binding.matches(binding) {
            return Err(GatewayError::ApprovalScopeMismatch(self.account_id.clone()));
        }
        Ok(())
    }
}

fn validate_acknowledgement_client_id(expected: &str, actual: &str) -> Result<(), GatewayError> {
    if actual.is_empty() || actual == "0" || actual == expected {
        return Ok(());
    }
    Err(GatewayError::AcknowledgementClientIdMismatch {
        expected: expected.to_string(),
        actual: actual.to_string(),
    })
}

impl OkxReconciliationClient {
    pub fn new(reconciliation: Arc<dyn RegularReconciliation>, pacing: PacingPolicy) -> Self {
        Self {
            reconciliation,
            pacer: RequestPacer::new(pacing),
        }
    }

    pub async fn fetch_remote_state(
        &self,
        instrument_type: Option<&str>,
        symbol: Option<&str>,
        max_order_pages: usize,
        max_fill_pages: usize,
    ) -> Result<(Vec<reap_venue::RemoteOrder>, Vec<reap_venue::RemoteFill>), GatewayError> {
        let mut order_pagination = OkxRegularOrderPagination::new(max_order_pages)?;
        loop {
            self.pacer.pace(RequestKind::Reconcile, "account").await;
            let page = self
                .reconciliation
                .regular_pending_orders_page(instrument_type, symbol, order_pagination.after())
                .await?;
            if order_pagination.accept(page)? {
                break;
            }
        }
        let remote_orders = order_pagination.into_orders();
        let mut pagination = OkxFillPagination::new(max_fill_pages)?;
        loop {
            self.pacer.pace(RequestKind::Reconcile, "account").await;
            let page = self
                .reconciliation
                .recent_fills_page(instrument_type, symbol, pagination.after())
                .await?;
            if pagination.accept(page)? {
                break;
            }
        }
        Ok((remote_orders, pagination.into_fills()))
    }

    pub async fn fetch_remote_account_state(&self) -> Result<AccountUpdate, GatewayError> {
        self.pacer.pace(RequestKind::Reconcile, "account").await;
        let mut account = self.reconciliation.account_balance().await?;
        if account.balances.is_empty() {
            return Err(GatewayError::EmptyAccountBalance);
        }
        self.pacer.pace(RequestKind::Reconcile, "account").await;
        let positions = self.reconciliation.account_positions().await?;
        account.ts_ms = account.ts_ms.max(positions.ts_ms);
        account.positions = positions.positions;
        Ok(account)
    }

    pub async fn fetch_order_details(
        &self,
        symbol: &str,
        client_order_id: &str,
    ) -> Result<reap_venue::RemoteOrder, GatewayError> {
        self.pacer.pace(RequestKind::Reconcile, "account").await;
        Ok(self
            .reconciliation
            .order_details(symbol, client_order_id)
            .await?)
    }

    pub async fn exchange_time_ms(&self) -> Result<u64, GatewayError> {
        Ok(self.reconciliation.server_time_ms().await?)
    }
}

#[cfg(test)]
mod tests {
    use std::collections::VecDeque;
    use std::sync::{Arc, Mutex};

    use async_trait::async_trait;
    use reap_core::{Side, TimeInForce};
    use reap_venue::okx::{
        parse_okx_account_balance_response_json, parse_okx_account_positions_response_json,
        parse_okx_fill_page_response_json, parse_okx_order_details_response_json,
        parse_okx_regular_order_page_response_json,
    };

    use super::*;
    use crate::CancelOrderTransportError;
    use crate::authority::{approved_regular_cancel_for_test, reserved_regular_submit_for_test};

    #[derive(Clone)]
    struct HttpResponse {
        #[allow(dead_code)]
        status: u16,
        body: String,
    }

    #[derive(Clone)]
    struct MockRoles {
        responses: Arc<Mutex<VecDeque<Result<HttpResponse, RestError>>>>,
        calls: Arc<Mutex<usize>>,
        order_responses:
            Arc<Mutex<VecDeque<Result<reap_venue::okx::OkxOrderAck, OrderTransportError>>>>,
        order_calls: Arc<Mutex<usize>>,
        command_behavior: MockCommandBehavior,
    }

    #[derive(Clone, Copy)]
    enum MockCommandBehavior {
        Missing,
        Responses,
        SubstituteCancel,
    }

    impl MockRoles {
        fn next_order(&self) -> Result<reap_venue::okx::OkxOrderAck, OrderTransportError> {
            *self.order_calls.lock().unwrap() += 1;
            self.order_responses
                .lock()
                .unwrap()
                .pop_front()
                .expect("mock order response")
        }

        fn next(&self) -> Result<HttpResponse, RestError> {
            *self.calls.lock().unwrap() += 1;
            self.responses
                .lock()
                .unwrap()
                .pop_front()
                .expect("mock response")
        }
    }

    #[async_trait]
    impl RegularExecution for MockRoles {
        async fn place_regular_order(
            &self,
            _order: PreparedRegularSubmit,
        ) -> Result<OkxOrderAck, OrderTransportError> {
            match self.command_behavior {
                MockCommandBehavior::Missing => Err(OrderTransportError::Unavailable(
                    "regular order transport is not installed".to_string(),
                )),
                MockCommandBehavior::Responses => self.next_order(),
                MockCommandBehavior::SubstituteCancel => {
                    unreachable!("substitution test exercises only cancellation")
                }
            }
        }

        async fn cancel_regular_order(
            &self,
            mut cancel: PreparedRegularCancel,
        ) -> Result<OkxOrderAck, CancelOrderTransportError> {
            match self.command_behavior {
                MockCommandBehavior::Missing => {
                    Err(CancelOrderTransportError::pre_send_unavailable(
                        "regular order transport is not installed",
                        cancel,
                    ))
                }
                MockCommandBehavior::Responses => match self.next_order() {
                    Ok(ack) => Ok(ack),
                    Err(OrderTransportError::Unavailable(message)) => Err(
                        CancelOrderTransportError::pre_send_unavailable(message, cancel),
                    ),
                    Err(error) => Err(CancelOrderTransportError::failed(error)),
                },
                MockCommandBehavior::SubstituteCancel => {
                    cancel.client_order_id = "foreign-order".to_string();
                    Err(CancelOrderTransportError::pre_send_unavailable(
                        "substituted before REST fallback",
                        cancel,
                    ))
                }
            }
        }

        async fn cancel_regular_order_via_rest(
            &self,
            cancel: PreparedRegularCancel,
        ) -> Result<OkxOrderAck, OrderTransportError> {
            let response = self
                .next()
                .map_err(|error| OrderTransportError::Ambiguous(error.to_string()))?;
            parse_ack(response, cancel.client_order_id())
                .map_err(|error| OrderTransportError::Ambiguous(error.to_string()))
        }
    }

    #[async_trait]
    impl RegularReconciliation for MockRoles {
        async fn regular_pending_orders_page(
            &self,
            _instrument_type: Option<&str>,
            _symbol: Option<&str>,
            _after: Option<&str>,
        ) -> Result<OkxRegularOrderPage, RestError> {
            let response = self.next()?;
            parse_okx_regular_order_page_response_json(response.body.as_bytes())
        }

        async fn recent_fills_page(
            &self,
            _instrument_type: Option<&str>,
            _symbol: Option<&str>,
            _after: Option<&str>,
        ) -> Result<OkxFillPage, RestError> {
            let response = self.next()?;
            parse_okx_fill_page_response_json(response.body.as_bytes())
        }

        async fn account_balance(&self) -> Result<AccountUpdate, RestError> {
            let response = self.next()?;
            Ok(parse_okx_account_balance_response_json(response.body.as_bytes())?.account_update())
        }

        async fn account_positions(&self) -> Result<AccountUpdate, RestError> {
            let response = self.next()?;
            Ok(
                parse_okx_account_positions_response_json(response.body.as_bytes())?
                    .account_update(),
            )
        }

        async fn order_details(
            &self,
            _symbol: &str,
            _client_order_id: &str,
        ) -> Result<reap_venue::RemoteOrder, RestError> {
            let response = self.next()?;
            Ok(parse_okx_order_details_response_json(response.body.as_bytes())?.order)
        }

        async fn server_time_ms(&self) -> Result<u64, RestError> {
            let response = self.next()?;
            let value: serde_json::Value = serde_json::from_str(&response.body)?;
            value["data"][0]["ts"]
                .as_str()
                .ok_or_else(|| RestError::InvalidField {
                    field: "ts",
                    value: value["data"][0]["ts"].to_string(),
                    message: "must be a string".to_string(),
                })?
                .parse()
                .map_err(|_| RestError::InvalidField {
                    field: "ts",
                    value: value["data"][0]["ts"].to_string(),
                    message: "must be an unsigned integer".to_string(),
                })
        }
    }

    fn parse_ack(
        response: HttpResponse,
        fallback_client_id: &str,
    ) -> Result<OkxOrderAck, RestError> {
        let value: serde_json::Value = serde_json::from_str(&response.body)?;
        let code = value["code"].as_str().unwrap_or_default();
        if code != "0" {
            return Err(RestError::Api {
                code: code.to_string(),
                message: value["msg"].as_str().unwrap_or_default().to_string(),
            });
        }
        let row = &value["data"][0];
        let sub_code = row["sCode"].as_str().unwrap_or_default();
        if !sub_code.is_empty() && sub_code != "0" {
            return Err(RestError::Api {
                code: sub_code.to_string(),
                message: row["sMsg"].as_str().unwrap_or_default().to_string(),
            });
        }
        Ok(OkxOrderAck {
            exchange_order_id: row["ordId"].as_str().unwrap_or_default().to_string(),
            client_order_id: row["clOrdId"]
                .as_str()
                .filter(|value| !value.is_empty())
                .unwrap_or(fallback_client_id)
                .to_string(),
        })
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

    fn approved_order(gateway: &OkxOrderGateway) -> ReservedRegularSubmit {
        reserved_regular_submit_for_test("main", "reap1", order(), gateway.binding.clone())
    }

    fn approved_cancel(gateway: &OkxOrderGateway, client_order_id: &str) -> ApprovedRegularCancel {
        approved_regular_cancel_for_test(
            "main",
            "BTC-USDT",
            client_order_id,
            "safety_cancel",
            gateway.binding.clone(),
        )
    }

    fn fill_response(first: usize, count: usize) -> HttpResponse {
        let data = (first..first + count)
            .map(|index| {
                serde_json::json!({
                    "billId": format!("bill-{index}"),
                    "tradeId": format!("fill-{index}"),
                    "ordId": format!("order-{index}"),
                    "clOrdId": format!("client-{index}"),
                    "instId": "BTC-USDT",
                    "side": "buy",
                    "fillPx": "100",
                    "fillSz": "0.01",
                    "execType": "M",
                    "fee": "-0.00001",
                    "feeCcy": "BTC",
                    "fillTime": "1000"
                })
            })
            .collect::<Vec<_>>();
        HttpResponse {
            status: 200,
            body: serde_json::json!({"code": "0", "msg": "", "data": data}).to_string(),
        }
    }

    fn regular_order_response(first: usize, count: usize) -> HttpResponse {
        let data = (first..first + count)
            .map(|index| {
                serde_json::json!({
                    "ordId": format!("order-{index}"),
                    "clOrdId": format!("client-{index}"),
                    "instId": "BTC-USDT",
                    "side": "buy",
                    "state": "live",
                    "px": "100",
                    "sz": "0.01",
                    "accFillSz": "0",
                    "avgPx": "",
                    "uTime": "1000"
                })
            })
            .collect::<Vec<_>>();
        HttpResponse {
            status: 200,
            body: serde_json::json!({"code": "0", "msg": "", "data": data}).to_string(),
        }
    }

    fn gateway(
        responses: Vec<Result<HttpResponse, RestError>>,
    ) -> (OkxOrderGateway, Arc<Mutex<usize>>) {
        let (gateway, rest_calls, _) =
            gateway_with_command_behavior(responses, Vec::new(), MockCommandBehavior::Missing);
        (gateway, rest_calls)
    }

    fn gateway_with_order_responses(
        responses: Vec<Result<HttpResponse, RestError>>,
        order_responses: Vec<Result<reap_venue::okx::OkxOrderAck, OrderTransportError>>,
    ) -> (OkxOrderGateway, Arc<Mutex<usize>>, Arc<Mutex<usize>>) {
        gateway_with_command_behavior(responses, order_responses, MockCommandBehavior::Responses)
    }

    fn gateway_with_command_behavior(
        responses: Vec<Result<HttpResponse, RestError>>,
        order_responses: Vec<Result<reap_venue::okx::OkxOrderAck, OrderTransportError>>,
        command_behavior: MockCommandBehavior,
    ) -> (OkxOrderGateway, Arc<Mutex<usize>>, Arc<Mutex<usize>>) {
        let calls = Arc::new(Mutex::new(0));
        let order_calls = Arc::new(Mutex::new(0));
        let roles = Arc::new(MockRoles {
            responses: Arc::new(Mutex::new(responses.into())),
            calls: Arc::clone(&calls),
            order_responses: Arc::new(Mutex::new(order_responses.into())),
            order_calls: Arc::clone(&order_calls),
            command_behavior,
        });
        let gateway = OkxOrderGateway::new(
            "main",
            Box::new((*roles).clone()) as Box<dyn RegularExecution>,
            roles as Arc<dyn RegularReconciliation>,
            HashMap::from([("BTC-USDT".to_string(), OkxTradeMode::Cash)]),
            PacingPolicy::default(),
        )
        .unwrap();
        (gateway, calls, order_calls)
    }

    #[test]
    fn command_dispatcher_is_transferred_once() {
        let (mut gateway, _) = gateway(Vec::new());

        let _dispatcher = gateway
            .take_command_dispatcher()
            .expect("the command role must transfer once");
        assert!(matches!(
            gateway.take_command_dispatcher(),
            Err(GatewayError::CommandDispatcherTaken(account_id)) if account_id == "main"
        ));
    }

    #[test]
    fn gateway_rejects_same_account_authority_from_a_different_scope() {
        let (source, _) = gateway(Vec::new());
        let (mut target, _) = gateway(Vec::new());

        assert!(matches!(
            target.prepare_submit("cross-scope", approved_order(&source)),
            Err(GatewayError::ApprovalScopeMismatch(account_id)) if account_id == "main"
        ));
    }

    #[test]
    fn gateway_rejects_cross_account_authority_before_preparation() {
        let (source, _) = gateway(Vec::new());
        let roles = Arc::new(MockRoles {
            responses: Arc::new(Mutex::new(VecDeque::new())),
            calls: Arc::new(Mutex::new(0)),
            order_responses: Arc::new(Mutex::new(VecDeque::new())),
            order_calls: Arc::new(Mutex::new(0)),
            command_behavior: MockCommandBehavior::Missing,
        });
        let mut target = OkxOrderGateway::new(
            "other",
            Box::new((*roles).clone()) as Box<dyn RegularExecution>,
            roles as Arc<dyn RegularReconciliation>,
            HashMap::from([("BTC-USDT".to_string(), OkxTradeMode::Cash)]),
            PacingPolicy::default(),
        )
        .unwrap();

        assert!(matches!(
            target.prepare_submit("cross-account", approved_order(&source)),
            Err(GatewayError::ApprovalAccountMismatch { expected, actual })
                if expected == "other" && actual == "main"
        ));
    }

    #[tokio::test]
    async fn accepted_idempotent_submit_does_not_send_twice() {
        let (mut gateway, rest_calls, order_calls) = gateway_with_order_responses(
            Vec::new(),
            vec![Ok(OkxOrderAck {
                exchange_order_id: "123".to_string(),
                client_order_id: "reap1".to_string(),
            })],
        );
        let mut state = PrivateStateReducer::new();

        let first = gateway
            .submit_registered("decision-1", approved_order(&gateway), &mut state)
            .await
            .unwrap();
        let second = gateway
            .submit("decision-1", approved_order(&gateway))
            .await
            .unwrap();

        assert!(matches!(&first, SubmitOutcome::Submitted { .. }));
        assert!(matches!(&second, SubmitOutcome::Duplicate { .. }));
        assert_eq!(*order_calls.lock().unwrap(), 1);
        assert_eq!(*rest_calls.lock().unwrap(), 0);
        let SubmitOutcome::Submitted {
            client_order_id, ..
        } = first
        else {
            unreachable!();
        };
        assert_eq!(
            state.order_reducer().get(&client_order_id).unwrap().reason,
            "quote"
        );
    }

    #[tokio::test]
    async fn mismatched_submit_acknowledgement_is_ambiguous_and_not_bound() {
        let (mut gateway, rest_calls, order_calls) = gateway_with_order_responses(
            Vec::new(),
            vec![Ok(OkxOrderAck {
                exchange_order_id: "123".to_string(),
                client_order_id: "foreign-order".to_string(),
            })],
        );

        let error = gateway
            .submit("decision-1", approved_order(&gateway))
            .await
            .unwrap_err();

        assert!(error.is_ambiguous());
        assert!(matches!(
            gateway
                .submit("decision-1", approved_order(&gateway))
                .await
                .unwrap(),
            SubmitOutcome::PendingReconciliation { .. }
        ));
        assert_eq!(*order_calls.lock().unwrap(), 1);
        assert_eq!(*rest_calls.lock().unwrap(), 0);
    }

    #[tokio::test]
    async fn prepared_submit_can_be_registered_before_order_transport_io() {
        let (mut gateway, rest_calls, order_calls) = gateway_with_order_responses(
            Vec::new(),
            vec![Ok(OkxOrderAck {
                exchange_order_id: "123".to_string(),
                client_order_id: "reap1".to_string(),
            })],
        );
        let mut state = PrivateStateReducer::new();

        let SubmitPreparation::Ready(prepared) = gateway
            .prepare_submit("decision-1", approved_order(&gateway))
            .expect("submission should be prepared")
        else {
            panic!("new submission should require execution");
        };

        assert_eq!(*order_calls.lock().unwrap(), 0);
        assert_eq!(*rest_calls.lock().unwrap(), 0);
        state.register_local_order(prepared.client_order_id(), prepared.order().clone());
        let client_order_id = prepared.client_order_id().to_string();

        assert!(matches!(
            gateway.execute_submit(prepared).await.unwrap(),
            SubmitOutcome::Submitted { .. }
        ));
        assert_eq!(*order_calls.lock().unwrap(), 1);
        assert_eq!(*rest_calls.lock().unwrap(), 0);
        assert_eq!(
            state.order_reducer().get(&client_order_id).unwrap().reason,
            "quote"
        );
    }

    #[tokio::test]
    async fn account_reconciliation_fetches_balances_and_positions() {
        let balance = HttpResponse {
            status: 200,
            body: r#"{"code":"0","msg":"","data":[{"uTime":"100","details":[{"ccy":"USDT","cashBal":"100","availBal":"90","eq":"100","liab":"0","maxLoan":"0"}]}]}"#.to_string(),
        };
        let positions = HttpResponse {
            status: 200,
            body: r#"{"code":"0","msg":"","data":[{"instType":"SWAP","instId":"BTC-USDT-SWAP","pos":"2","avgPx":"50000","posSide":"net","mgnMode":"cross","uTime":"101"}]}"#.to_string(),
        };
        let (gateway, calls) = gateway(vec![Ok(balance), Ok(positions)]);

        let account = gateway.fetch_remote_account_state().await.unwrap();

        assert_eq!(*calls.lock().unwrap(), 2);
        assert_eq!(account.ts_ms, 101);
        assert_eq!(account.balances[0].currency, "USDT");
        assert_eq!(account.positions[0].symbol, "BTC-USDT-SWAP");
        assert_eq!(account.positions[0].qty, 2.0);
    }

    #[tokio::test]
    async fn remote_state_reconciliation_fetches_every_fill_page_through_gateway() {
        let open_orders = HttpResponse {
            status: 200,
            body: r#"{"code":"0","msg":"","data":[]}"#.to_string(),
        };
        let (gateway, calls) = gateway(vec![
            Ok(open_orders),
            Ok(fill_response(100, 100)),
            Ok(fill_response(200, 2)),
        ]);

        let (orders, fills) = gateway.fetch_remote_state(None, None, 3, 3).await.unwrap();

        assert!(orders.is_empty());
        assert_eq!(fills.len(), 102);
        assert_eq!(*calls.lock().unwrap(), 3);
    }

    #[tokio::test]
    async fn remote_state_reconciliation_fetches_every_regular_order_page() {
        let (gateway, calls) = gateway(vec![
            Ok(regular_order_response(100, 100)),
            Ok(regular_order_response(200, 1)),
            Ok(fill_response(1, 0)),
        ]);

        let (orders, fills) = gateway.fetch_remote_state(None, None, 3, 3).await.unwrap();

        assert_eq!(orders.len(), 101);
        assert!(fills.is_empty());
        assert_eq!(*calls.lock().unwrap(), 3);
    }

    #[tokio::test]
    async fn empty_balance_snapshot_is_not_authoritative() {
        let balance = HttpResponse {
            status: 200,
            body: r#"{"code":"0","msg":"","data":[{"uTime":"100","details":[] }]}"#.to_string(),
        };
        let (gateway, calls) = gateway(vec![Ok(balance)]);

        let error = gateway.fetch_remote_account_state().await.unwrap_err();

        assert!(matches!(error, GatewayError::EmptyAccountBalance));
        assert_eq!(*calls.lock().unwrap(), 1);
    }

    #[tokio::test]
    async fn ambiguous_failure_is_held_for_reconciliation() {
        let (mut gateway, rest_calls, order_calls) = gateway_with_order_responses(
            Vec::new(),
            vec![Err(OrderTransportError::Ambiguous("timeout".to_string()))],
        );
        assert!(
            gateway
                .submit("decision-1", approved_order(&gateway))
                .await
                .is_err()
        );

        let retry = gateway
            .submit("decision-1", approved_order(&gateway))
            .await
            .unwrap();
        assert!(matches!(retry, SubmitOutcome::PendingReconciliation { .. }));
        assert_eq!(*order_calls.lock().unwrap(), 1);
        assert_eq!(*rest_calls.lock().unwrap(), 0);
    }

    #[tokio::test]
    async fn explicit_api_rejection_releases_idempotency_key_for_retry() {
        let (mut gateway, rest_calls, order_calls) = gateway_with_order_responses(
            Vec::new(),
            vec![
                Err(OrderTransportError::Rejected {
                    code: "51000".to_string(),
                    message: "parameter error".to_string(),
                }),
                Ok(OkxOrderAck {
                    exchange_order_id: "123".to_string(),
                    client_order_id: "reap1".to_string(),
                }),
            ],
        );

        assert!(
            gateway
                .submit("decision-1", approved_order(&gateway))
                .await
                .is_err()
        );
        assert!(matches!(
            gateway
                .submit("decision-1", approved_order(&gateway))
                .await
                .unwrap(),
            SubmitOutcome::Submitted { .. }
        ));
        assert_eq!(*order_calls.lock().unwrap(), 2);
        assert_eq!(*rest_calls.lock().unwrap(), 0);
    }

    #[tokio::test]
    async fn uninstalled_execution_place_is_pre_send_unavailable_and_retryable() {
        let (mut gateway, rest_calls) = gateway(Vec::new());

        for _ in 0..2 {
            let error = gateway
                .submit("decision-1", approved_order(&gateway))
                .await
                .unwrap_err();
            assert!(matches!(
                &error,
                GatewayError::OrderTransport(OrderTransportError::Unavailable(_))
            ));
            assert!(!error.is_ambiguous());
        }
        assert_eq!(*rest_calls.lock().unwrap(), 0);
    }

    #[tokio::test]
    async fn uninstalled_execution_cancel_returns_exact_token_for_rest_fallback() {
        let response = HttpResponse {
            status: 200,
            body: r#"{"code":"0","msg":"","data":[{"ordId":"42","clOrdId":"reap1","sCode":"0","sMsg":""}]}"#.to_string(),
        };
        let (gateway, rest_calls) = gateway(vec![Ok(response)]);

        let outcome = gateway
            .cancel(approved_cancel(&gateway, "reap1"))
            .await
            .unwrap();

        assert_eq!(outcome.client_order_id, "reap1");
        assert_eq!(outcome.exchange_order_id, "42");
        assert_eq!(*rest_calls.lock().unwrap(), 1);
    }

    #[tokio::test]
    async fn websocket_ambiguity_retains_pending_identity_without_rest_fallback() {
        let (mut gateway, rest_calls, order_calls) = gateway_with_order_responses(
            Vec::new(),
            vec![Err(OrderTransportError::Ambiguous(
                "disconnect after write".to_string(),
            ))],
        );

        let error = gateway
            .submit("decision-1", approved_order(&gateway))
            .await
            .unwrap_err();
        assert!(error.is_ambiguous());
        assert!(matches!(
            gateway
                .submit("decision-1", approved_order(&gateway))
                .await
                .unwrap(),
            SubmitOutcome::PendingReconciliation { .. }
        ));
        assert_eq!(*order_calls.lock().unwrap(), 1);
        assert_eq!(*rest_calls.lock().unwrap(), 0);
    }

    #[tokio::test]
    async fn websocket_rejection_releases_identity_for_a_later_decision_retry() {
        let (mut gateway, rest_calls, order_calls) = gateway_with_order_responses(
            Vec::new(),
            vec![
                Err(OrderTransportError::Rejected {
                    code: "51000".to_string(),
                    message: "bad parameter".to_string(),
                }),
                Ok(reap_venue::okx::OkxOrderAck {
                    exchange_order_id: "42".to_string(),
                    client_order_id: "reap1".to_string(),
                }),
            ],
        );

        assert!(
            gateway
                .submit("decision-1", approved_order(&gateway))
                .await
                .is_err()
        );
        assert!(matches!(
            gateway
                .submit("decision-1", approved_order(&gateway))
                .await
                .unwrap(),
            SubmitOutcome::Submitted { ref exchange_order_id, .. } if exchange_order_id == "42"
        ));
        assert_eq!(*order_calls.lock().unwrap(), 2);
        assert_eq!(*rest_calls.lock().unwrap(), 0);
    }

    #[tokio::test]
    async fn cancellation_falls_back_to_rest_only_when_websocket_is_unavailable_before_send() {
        let response = HttpResponse {
            status: 200,
            body: r#"{"code":"0","msg":"","data":[{"ordId":"42","clOrdId":"reap1","sCode":"0","sMsg":""}]}"#.to_string(),
        };
        let (gateway, rest_calls, order_calls) = gateway_with_order_responses(
            vec![Ok(response)],
            vec![Err(OrderTransportError::Unavailable(
                "session disconnected".to_string(),
            ))],
        );

        let outcome = gateway
            .cancel(approved_cancel(&gateway, "reap1"))
            .await
            .unwrap();
        assert_eq!(outcome.exchange_order_id, "42");
        assert_eq!(*order_calls.lock().unwrap(), 1);
        assert_eq!(*rest_calls.lock().unwrap(), 1);
    }

    #[tokio::test]
    async fn cancellation_rejects_a_substituted_pre_send_fallback_token() {
        let response = HttpResponse {
            status: 200,
            body: r#"{"code":"0","msg":"","data":[{"ordId":"42","clOrdId":"reap1","sCode":"0","sMsg":""}]}"#.to_string(),
        };
        let (gateway, rest_calls, _) = gateway_with_command_behavior(
            vec![Ok(response)],
            Vec::new(),
            MockCommandBehavior::SubstituteCancel,
        );

        let error = gateway
            .cancel(approved_cancel(&gateway, "reap1"))
            .await
            .unwrap_err();

        assert!(matches!(
            error,
            GatewayError::CancelFallbackIdentityMismatch { .. }
        ));
        assert_eq!(*rest_calls.lock().unwrap(), 0);
    }

    #[tokio::test]
    async fn mismatched_cancel_acknowledgement_is_ambiguous_without_rest_retry() {
        let (gateway, rest_calls, order_calls) = gateway_with_order_responses(
            Vec::new(),
            vec![Ok(OkxOrderAck {
                exchange_order_id: "42".to_string(),
                client_order_id: "foreign-order".to_string(),
            })],
        );

        let error = gateway
            .cancel(approved_cancel(&gateway, "reap1"))
            .await
            .unwrap_err();

        assert!(matches!(
            &error,
            GatewayError::AcknowledgementClientIdMismatch { .. }
        ));
        assert!(error.is_ambiguous());
        assert_eq!(*order_calls.lock().unwrap(), 1);
        assert_eq!(*rest_calls.lock().unwrap(), 0);
    }

    #[tokio::test]
    async fn ambiguous_websocket_cancel_does_not_retry_over_rest() {
        let response = HttpResponse {
            status: 200,
            body: r#"{"code":"0","msg":"","data":[{"ordId":"42","clOrdId":"reap1","sCode":"0","sMsg":""}]}"#.to_string(),
        };
        let (gateway, rest_calls, order_calls) = gateway_with_order_responses(
            vec![Ok(response)],
            vec![Err(OrderTransportError::Ambiguous(
                "disconnect after write".to_string(),
            ))],
        );

        let error = gateway
            .cancel(approved_cancel(&gateway, "reap1"))
            .await
            .unwrap_err();
        assert!(error.is_ambiguous());
        assert_eq!(*order_calls.lock().unwrap(), 1);
        assert_eq!(*rest_calls.lock().unwrap(), 0);
    }
}
