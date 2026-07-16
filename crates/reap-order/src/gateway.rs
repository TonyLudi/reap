use std::collections::HashMap;
use std::sync::Arc;
use std::time::{SystemTime, UNIX_EPOCH};

use reap_core::{AccountUpdate, NewOrder};
use reap_venue::okx::{
    HttpTransport, OkxCancelOrder, OkxFillPagination, OkxOrderAck, OkxPlaceOrder,
    OkxRegularOrderPagination, OkxRestClient, OkxTradeMode, RestError,
};
use serde::{Deserialize, Serialize};
use thiserror::Error;

use crate::{
    ClientIdError, ClientOrderIdGenerator, IdempotencyError, IdempotencyRegistry,
    OkxOrderTransport, OrderTransportError, PacingPolicy, PrivateStateReducer,
    ReconciliationSnapshot, RequestKind, RequestPacer, Reservation, reconcile_full_state,
};

#[derive(Debug, Error)]
pub enum GatewayError {
    #[error("OKX gateway request failed: {0}")]
    Rest(#[from] RestError),
    #[error("OKX order transport failed: {0}")]
    OrderTransport(#[from] OrderTransportError),
    #[error("client order id configuration is invalid: {0}")]
    ClientId(#[from] ClientIdError),
    #[error("idempotency check failed: {0}")]
    Idempotency(#[from] IdempotencyError),
    #[error("no OKX trade mode configured for {0}")]
    MissingTradeMode(String),
    #[error("OKX account reconciliation returned no balance rows")]
    EmptyAccountBalance,
}

impl GatewayError {
    pub fn is_ambiguous(&self) -> bool {
        matches!(self, Self::Rest(RestError::Transport(_)))
            || matches!(self, Self::OrderTransport(error) if error.is_ambiguous())
    }

    pub fn is_order_not_found(&self) -> bool {
        matches!(self, Self::Rest(error) if error.is_order_not_found())
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

#[derive(Debug, Clone)]
pub struct PreparedOrder {
    idempotency_key: String,
    client_order_id: String,
    order: NewOrder,
    trade_mode: OkxTradeMode,
}

impl PreparedOrder {
    pub fn client_order_id(&self) -> &str {
        &self.client_order_id
    }

    pub fn order(&self) -> &NewOrder {
        &self.order
    }
}

#[derive(Debug, Clone)]
pub enum SubmitPreparation {
    Ready(PreparedOrder),
    Complete(SubmitOutcome),
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct CancelOutcome {
    pub client_order_id: String,
    pub exchange_order_id: String,
}

pub struct OkxOrderGateway<T> {
    io: OkxGatewayIo<T>,
    ids: ClientOrderIdGenerator,
    idempotency: IdempotencyRegistry,
    trade_modes: HashMap<String, OkxTradeMode>,
    local_orders: HashMap<String, NewOrder>,
}

#[derive(Clone)]
pub struct OkxGatewayIo<T> {
    client: OkxRestClient<T>,
    order_transport: Option<Arc<dyn OkxOrderTransport>>,
    pacer: RequestPacer,
}

impl<T> OkxOrderGateway<T>
where
    T: HttpTransport,
{
    pub fn new(
        client: OkxRestClient<T>,
        id_prefix: impl Into<String>,
        node_id: u16,
        trade_modes: HashMap<String, OkxTradeMode>,
        pacing: PacingPolicy,
    ) -> Result<Self, GatewayError> {
        Ok(Self {
            io: OkxGatewayIo {
                client,
                order_transport: None,
                pacer: RequestPacer::new(pacing),
            },
            ids: ClientOrderIdGenerator::new(id_prefix, node_id)?,
            idempotency: IdempotencyRegistry::default(),
            trade_modes,
            local_orders: HashMap::new(),
        })
    }

    pub fn set_order_transport(&mut self, transport: Box<dyn OkxOrderTransport>) {
        self.io.order_transport = Some(Arc::from(transport));
    }

    pub fn io_client(&self) -> OkxGatewayIo<T>
    where
        T: Clone,
    {
        self.io.clone()
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
        order: NewOrder,
    ) -> Result<SubmitOutcome, GatewayError> {
        match self.prepare_submit(idempotency_key, order)? {
            SubmitPreparation::Ready(prepared) => self.execute_submit(prepared).await,
            SubmitPreparation::Complete(outcome) => Ok(outcome),
        }
    }

    pub async fn submit_registered(
        &mut self,
        idempotency_key: impl Into<String>,
        order: NewOrder,
        state: &mut PrivateStateReducer,
    ) -> Result<SubmitOutcome, GatewayError> {
        match self.prepare_submit(idempotency_key, order)? {
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
                        unreachable!("submitted outcomes require REST execution")
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
        order: NewOrder,
    ) -> Result<SubmitPreparation, GatewayError> {
        let trade_mode = self
            .trade_modes
            .get(&order.symbol)
            .copied()
            .ok_or_else(|| GatewayError::MissingTradeMode(order.symbol.clone()))?;
        let generated_id = self.ids.next(unix_time_ms());
        self.prepare_submit_with_id(idempotency_key, order, generated_id, trade_mode)
    }

    pub fn prepare_registered_submit(
        &mut self,
        idempotency_key: impl Into<String>,
        order: NewOrder,
        client_order_id: impl Into<String>,
    ) -> Result<SubmitPreparation, GatewayError> {
        let trade_mode = self
            .trade_modes
            .get(&order.symbol)
            .copied()
            .ok_or_else(|| GatewayError::MissingTradeMode(order.symbol.clone()))?;
        self.prepare_submit_with_id(idempotency_key, order, client_order_id.into(), trade_mode)
    }

    fn prepare_submit_with_id(
        &mut self,
        idempotency_key: impl Into<String>,
        order: NewOrder,
        generated_id: String,
        trade_mode: OkxTradeMode,
    ) -> Result<SubmitPreparation, GatewayError> {
        let idempotency_key = idempotency_key.into();
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
        Ok(SubmitPreparation::Ready(PreparedOrder {
            idempotency_key,
            client_order_id,
            order,
            trade_mode,
        }))
    }

    pub async fn execute_submit(
        &mut self,
        prepared: PreparedOrder,
    ) -> Result<SubmitOutcome, GatewayError> {
        let result = self.io.place_prepared(&prepared).await;
        self.finish_submit(prepared, result)
    }

    pub fn finish_submit(
        &mut self,
        prepared: PreparedOrder,
        result: Result<OkxOrderAck, GatewayError>,
    ) -> Result<SubmitOutcome, GatewayError> {
        let PreparedOrder {
            idempotency_key,
            client_order_id,
            order: _,
            trade_mode: _,
        } = prepared;
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

    pub async fn cancel(
        &self,
        symbol: &str,
        exchange_order_id: Option<String>,
        client_order_id: Option<String>,
    ) -> Result<CancelOutcome, GatewayError> {
        self.io
            .cancel(symbol, exchange_order_id, client_order_id)
            .await
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
        self.io
            .fetch_remote_state(instrument_type, symbol, max_order_pages, max_fill_pages)
            .await
    }

    pub async fn fetch_remote_account_state(&self) -> Result<AccountUpdate, GatewayError> {
        self.io.fetch_remote_account_state().await
    }

    pub async fn fetch_order_details(
        &self,
        symbol: &str,
        client_order_id: &str,
    ) -> Result<reap_venue::RemoteOrder, GatewayError> {
        self.io.fetch_order_details(symbol, client_order_id).await
    }

    pub async fn exchange_time_ms(&self) -> Result<u64, GatewayError> {
        self.io.exchange_time_ms().await
    }

    pub async fn cancel_all_after(&self, timeout_secs: u64) -> Result<(), GatewayError> {
        self.io.cancel_all_after(timeout_secs).await
    }
}

impl<T> OkxGatewayIo<T>
where
    T: HttpTransport,
{
    pub async fn place_prepared(
        &self,
        prepared: &PreparedOrder,
    ) -> Result<OkxOrderAck, GatewayError> {
        self.pacer.pace(RequestKind::Submit, "account").await;
        self.pacer
            .pace(RequestKind::Submit, &prepared.order.symbol)
            .await;
        let request = OkxPlaceOrder {
            symbol: prepared.order.symbol.clone(),
            trade_mode: prepared.trade_mode,
            side: prepared.order.side,
            time_in_force: prepared.order.time_in_force,
            price: prepared.order.price,
            qty: prepared.order.qty,
            client_order_id: prepared.client_order_id.clone(),
            reduce_only: prepared.order.reduce_only,
            self_trade_prevention: prepared.order.self_trade_prevention,
        };
        match self.order_transport.as_ref() {
            Some(transport) => transport
                .place_order(&request)
                .await
                .map_err(GatewayError::from),
            None => self
                .client
                .place_order(&request)
                .await
                .map_err(GatewayError::from),
        }
    }

    pub async fn cancel(
        &self,
        symbol: &str,
        exchange_order_id: Option<String>,
        client_order_id: Option<String>,
    ) -> Result<CancelOutcome, GatewayError> {
        self.pacer.pace(RequestKind::Cancel, symbol).await;
        let request = OkxCancelOrder {
            symbol: symbol.to_string(),
            exchange_order_id,
            client_order_id,
        };
        let ack = match self.order_transport.as_ref() {
            Some(transport) => match transport.cancel_order(&request).await {
                Ok(ack) => ack,
                Err(error) if error.is_unavailable() => self.client.cancel_order(&request).await?,
                Err(error) => return Err(error.into()),
            },
            None => self.client.cancel_order(&request).await?,
        };
        Ok(CancelOutcome {
            client_order_id: ack.client_order_id,
            exchange_order_id: ack.exchange_order_id,
        })
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
                .client
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
                .client
                .fills_page(instrument_type, symbol, pagination.after())
                .await?;
            if pagination.accept(page)? {
                break;
            }
        }
        Ok((remote_orders, pagination.into_fills()))
    }

    pub async fn fetch_remote_account_state(&self) -> Result<AccountUpdate, GatewayError> {
        self.pacer.pace(RequestKind::Reconcile, "account").await;
        let mut account = self.client.account_balance().await?;
        if account.balances.is_empty() {
            return Err(GatewayError::EmptyAccountBalance);
        }
        self.pacer.pace(RequestKind::Reconcile, "account").await;
        let positions = self.client.account_positions(None, None).await?;
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
            .client
            .order_details(symbol, None, Some(client_order_id))
            .await?)
    }

    pub async fn exchange_time_ms(&self) -> Result<u64, GatewayError> {
        Ok(self.client.server_time_ms().await?)
    }

    pub async fn cancel_all_after(&self, timeout_secs: u64) -> Result<(), GatewayError> {
        Ok(self.client.cancel_all_after(timeout_secs).await?)
    }
}

fn unix_time_ms() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap_or_default()
        .as_millis()
        .min(u64::MAX as u128) as u64
}

#[cfg(test)]
mod tests {
    use std::collections::VecDeque;
    use std::sync::{Arc, Mutex};

    use async_trait::async_trait;
    use reap_core::{Side, TimeInForce};
    use reap_venue::okx::{HttpResponse, OkxCredentials, OkxSigner, SignedRequest};

    use super::*;

    #[derive(Clone)]
    struct MockTransport {
        responses: Arc<Mutex<VecDeque<Result<HttpResponse, RestError>>>>,
        calls: Arc<Mutex<usize>>,
    }

    struct MockOrderTransport {
        responses: Arc<Mutex<VecDeque<Result<reap_venue::okx::OkxOrderAck, OrderTransportError>>>>,
        calls: Arc<Mutex<usize>>,
    }

    #[async_trait]
    impl OkxOrderTransport for MockOrderTransport {
        async fn place_order(
            &self,
            _order: &OkxPlaceOrder,
        ) -> Result<reap_venue::okx::OkxOrderAck, OrderTransportError> {
            self.next()
        }

        async fn cancel_order(
            &self,
            _order: &OkxCancelOrder,
        ) -> Result<reap_venue::okx::OkxOrderAck, OrderTransportError> {
            self.next()
        }
    }

    impl MockOrderTransport {
        fn next(&self) -> Result<reap_venue::okx::OkxOrderAck, OrderTransportError> {
            *self.calls.lock().unwrap() += 1;
            self.responses
                .lock()
                .unwrap()
                .pop_front()
                .expect("mock order response")
        }
    }

    #[async_trait]
    impl HttpTransport for MockTransport {
        async fn execute(&self, _request: SignedRequest) -> Result<HttpResponse, RestError> {
            *self.calls.lock().unwrap() += 1;
            self.responses
                .lock()
                .unwrap()
                .pop_front()
                .expect("mock response")
        }
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
    ) -> (OkxOrderGateway<MockTransport>, Arc<Mutex<usize>>) {
        let calls = Arc::new(Mutex::new(0));
        let transport = MockTransport {
            responses: Arc::new(Mutex::new(responses.into())),
            calls: Arc::clone(&calls),
        };
        let client = OkxRestClient::new(
            transport,
            OkxSigner::new(OkxCredentials::new("key", "secret", "pass"), true),
        );
        let gateway = OkxOrderGateway::new(
            client,
            "reap",
            1,
            HashMap::from([("BTC-USDT".to_string(), OkxTradeMode::Cash)]),
            PacingPolicy::default(),
        )
        .unwrap();
        (gateway, calls)
    }

    fn install_order_transport(
        gateway: &mut OkxOrderGateway<MockTransport>,
        responses: Vec<Result<reap_venue::okx::OkxOrderAck, OrderTransportError>>,
    ) -> Arc<Mutex<usize>> {
        let calls = Arc::new(Mutex::new(0));
        gateway.set_order_transport(Box::new(MockOrderTransport {
            responses: Arc::new(Mutex::new(responses.into())),
            calls: Arc::clone(&calls),
        }));
        calls
    }

    #[tokio::test]
    async fn accepted_idempotent_submit_does_not_send_twice() {
        let response = HttpResponse {
            status: 200,
            body: r#"{"code":"0","msg":"","data":[{"ordId":"123","clOrdId":"ignored","sCode":"0","sMsg":""}]}"#.to_string(),
        };
        let (mut gateway, calls) = gateway(vec![Ok(response)]);
        let mut state = PrivateStateReducer::new();

        let first = gateway
            .submit_registered("decision-1", order(), &mut state)
            .await
            .unwrap();
        let second = gateway.submit("decision-1", order()).await.unwrap();

        assert!(matches!(&first, SubmitOutcome::Submitted { .. }));
        assert!(matches!(&second, SubmitOutcome::Duplicate { .. }));
        assert_eq!(*calls.lock().unwrap(), 1);
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
    async fn prepared_submit_can_be_registered_before_rest_io() {
        let response = HttpResponse {
            status: 200,
            body: r#"{"code":"0","msg":"","data":[{"ordId":"123","clOrdId":"ignored","sCode":"0","sMsg":""}]}"#.to_string(),
        };
        let (mut gateway, calls) = gateway(vec![Ok(response)]);
        let mut state = PrivateStateReducer::new();

        let SubmitPreparation::Ready(prepared) = gateway
            .prepare_submit("decision-1", order())
            .expect("submission should be prepared")
        else {
            panic!("new submission should require execution");
        };

        assert_eq!(*calls.lock().unwrap(), 0);
        state.register_local_order(prepared.client_order_id(), prepared.order().clone());
        let client_order_id = prepared.client_order_id().to_string();

        assert!(matches!(
            gateway.execute_submit(prepared).await.unwrap(),
            SubmitOutcome::Submitted { .. }
        ));
        assert_eq!(*calls.lock().unwrap(), 1);
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
        let (mut gateway, calls) = gateway(vec![Err(RestError::Transport("timeout".to_string()))]);
        assert!(gateway.submit("decision-1", order()).await.is_err());

        let retry = gateway.submit("decision-1", order()).await.unwrap();
        assert!(matches!(retry, SubmitOutcome::PendingReconciliation { .. }));
        assert_eq!(*calls.lock().unwrap(), 1);
    }

    #[tokio::test]
    async fn explicit_api_rejection_releases_idempotency_key_for_retry() {
        let response = HttpResponse {
            status: 200,
            body: r#"{"code":"0","msg":"","data":[{"ordId":"123","clOrdId":"ignored","sCode":"0","sMsg":""}]}"#.to_string(),
        };
        let (mut gateway, calls) = gateway(vec![
            Err(RestError::Api {
                code: "51000".to_string(),
                message: "parameter error".to_string(),
            }),
            Ok(response),
        ]);

        assert!(gateway.submit("decision-1", order()).await.is_err());
        assert!(matches!(
            gateway.submit("decision-1", order()).await.unwrap(),
            SubmitOutcome::Submitted { .. }
        ));
        assert_eq!(*calls.lock().unwrap(), 2);
    }

    #[tokio::test]
    async fn websocket_ambiguity_retains_pending_identity_without_rest_fallback() {
        let (mut gateway, rest_calls) = gateway(Vec::new());
        let order_calls = install_order_transport(
            &mut gateway,
            vec![Err(OrderTransportError::Ambiguous(
                "disconnect after write".to_string(),
            ))],
        );

        let error = gateway.submit("decision-1", order()).await.unwrap_err();
        assert!(error.is_ambiguous());
        assert!(matches!(
            gateway.submit("decision-1", order()).await.unwrap(),
            SubmitOutcome::PendingReconciliation { .. }
        ));
        assert_eq!(*order_calls.lock().unwrap(), 1);
        assert_eq!(*rest_calls.lock().unwrap(), 0);
    }

    #[tokio::test]
    async fn websocket_rejection_releases_identity_for_a_later_decision_retry() {
        let (mut gateway, rest_calls) = gateway(Vec::new());
        let order_calls = install_order_transport(
            &mut gateway,
            vec![
                Err(OrderTransportError::Rejected {
                    code: "51000".to_string(),
                    message: "bad parameter".to_string(),
                }),
                Ok(reap_venue::okx::OkxOrderAck {
                    exchange_order_id: "42".to_string(),
                    client_order_id: "ignored".to_string(),
                }),
            ],
        );

        assert!(gateway.submit("decision-1", order()).await.is_err());
        assert!(matches!(
            gateway.submit("decision-1", order()).await.unwrap(),
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
        let (mut gateway, rest_calls) = gateway(vec![Ok(response)]);
        let order_calls = install_order_transport(
            &mut gateway,
            vec![Err(OrderTransportError::Unavailable(
                "session disconnected".to_string(),
            ))],
        );

        let outcome = gateway
            .cancel("BTC-USDT", None, Some("reap1".to_string()))
            .await
            .unwrap();
        assert_eq!(outcome.exchange_order_id, "42");
        assert_eq!(*order_calls.lock().unwrap(), 1);
        assert_eq!(*rest_calls.lock().unwrap(), 1);
    }

    #[tokio::test]
    async fn ambiguous_websocket_cancel_does_not_retry_over_rest() {
        let response = HttpResponse {
            status: 200,
            body: r#"{"code":"0","msg":"","data":[{"ordId":"42","clOrdId":"reap1","sCode":"0","sMsg":""}]}"#.to_string(),
        };
        let (mut gateway, rest_calls) = gateway(vec![Ok(response)]);
        let order_calls = install_order_transport(
            &mut gateway,
            vec![Err(OrderTransportError::Ambiguous(
                "disconnect after write".to_string(),
            ))],
        );

        let error = gateway
            .cancel("BTC-USDT", None, Some("reap1".to_string()))
            .await
            .unwrap_err();
        assert!(error.is_ambiguous());
        assert_eq!(*order_calls.lock().unwrap(), 1);
        assert_eq!(*rest_calls.lock().unwrap(), 0);
    }
}
