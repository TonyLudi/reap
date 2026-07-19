use super::*;

pub(super) struct SeededOrder {
    pub(super) client_order_id: String,
    pub(super) order: NewOrder,
}

pub(super) struct PreparationRig {
    policy: RegularExecutionPolicy,
    client_ids: reap_order::ClientOrderIdGenerator,
    owned: OwnedRegularOrders,
    private_states: HashMap<String, PrivateStateReducer>,
    gateway: OkxOrderGateway,
}

impl PreparationRig {
    pub(super) fn new() -> Self {
        let roles = Arc::new(BenchmarkGatewayRoles);
        let mut gateway = OkxOrderGateway::new(
            ACCOUNT_ID,
            Box::new(BenchmarkGatewayRoles),
            roles,
            HashMap::from([
                ("BTC-USDT".to_string(), OkxTradeMode::Cash),
                ("BTC-PERP".to_string(), OkxTradeMode::Cross),
            ]),
            PacingPolicy::default(),
        )
        .expect("benchmark gateway");
        let scope = gateway
            .take_approval_scope()
            .expect("benchmark gateway yields one approval scope");
        let profiles = vec![
            RegularExecutionProfile::new(
                "BTC-USDT",
                ACCOUNT_ID,
                InstrumentRiskModel::Spot,
                InstrumentOrderLimits {
                    max_limit_quantity: 100.0,
                    max_limit_notional_usd: Some(1_000_000.0),
                },
                0.1,
                0.0001,
                0.0001,
                true,
                true,
                true,
            ),
            RegularExecutionProfile::new(
                "BTC-PERP",
                ACCOUNT_ID,
                InstrumentRiskModel::LinearDerivative {
                    contract_value: 0.001,
                },
                InstrumentOrderLimits {
                    max_limit_quantity: 1_000_000.0,
                    max_limit_notional_usd: None,
                },
                0.1,
                1.0,
                1.0,
                true,
                true,
                true,
            ),
        ];
        let (profile_set, client_ids) = scope
            .bind_profiles_and_client_id_generator(profiles, "bench", 1)
            .expect("benchmark profiles and client ID generator");
        let policy = RegularExecutionPolicy::from_profile_sets([profile_set])
            .expect("benchmark execution policy");
        Self {
            policy,
            client_ids,
            owned: OwnedRegularOrders::default(),
            private_states: HashMap::from([(ACCOUNT_ID.to_string(), PrivateStateReducer::new())]),
            gateway,
        }
    }

    pub(super) fn seed_submit(
        &mut self,
        intent: ChaosExecutionIntent,
        ordinal: usize,
    ) -> SeededOrder {
        let approved = self
            .policy
            .authorize_submit(intent)
            .expect("production-emitted benchmark submit must pass policy");
        let generated = self.client_ids.next(BASE_TS_MS + ordinal as u64);
        let client_order_id = generated.as_str().to_string();
        let (_, reserved) = self
            .owned
            .reserve_local(
                approved,
                generated,
                self.private_states
                    .get_mut(ACCOUNT_ID)
                    .expect("benchmark private state"),
                BASE_TS_MS + ordinal as u64,
            )
            .expect("benchmark local reservation");
        let order = reserved.order().clone();
        let preparation = self
            .gateway
            .prepare_submit(format!("seed:{ordinal}"), reserved)
            .expect("benchmark gateway preparation");
        let SubmitPreparation::Ready(prepared) = preparation else {
            panic!("unique seed idempotency must prepare a submit");
        };
        black_box(prepared);
        SeededOrder {
            client_order_id,
            order,
        }
    }

    pub(super) fn prepare_submit(&mut self, intent: ChaosExecutionIntent, ordinal: usize) {
        let approved = self
            .policy
            .authorize_submit(intent)
            .expect("production-emitted benchmark submit must pass policy");
        let generated = self.client_ids.next(BASE_TS_MS + ordinal as u64);
        let (_, reserved) = self
            .owned
            .reserve_local(
                approved,
                generated,
                self.private_states
                    .get_mut(ACCOUNT_ID)
                    .expect("benchmark private state"),
                BASE_TS_MS + ordinal as u64,
            )
            .expect("benchmark local reservation");
        let preparation = self
            .gateway
            .prepare_submit(format!("action:{ordinal}"), reserved)
            .expect("benchmark gateway preparation");
        let SubmitPreparation::Ready(prepared) = preparation else {
            panic!("unique action idempotency must prepare a submit");
        };
        black_box((
            prepared.account_id(),
            prepared.client_order_id(),
            prepared.order(),
            prepared.trade_mode(),
        ));
    }

    pub(super) fn prepare_cancel(&mut self, client_order_id: &str, reason: &str) {
        let approved = self
            .policy
            .authorize_cancel(client_order_id, reason, &self.owned, &self.private_states)
            .expect("owned benchmark cancellation must pass policy");
        let prepared = self
            .gateway
            .prepare_cancel(approved)
            .expect("benchmark cancel preparation");
        black_box((
            prepared.account_id(),
            prepared.symbol(),
            prepared.client_order_id(),
            prepared.reason(),
        ));
    }
}

#[derive(Debug)]
struct BenchmarkGatewayRoles;

#[async_trait]
impl RegularExecution for BenchmarkGatewayRoles {
    async fn place_regular_order(
        &self,
        _order: PreparedRegularSubmit,
    ) -> Result<OkxOrderAck, OrderTransportError> {
        unreachable!("the action benchmark stops at prepared authority")
    }

    async fn cancel_regular_order(
        &self,
        _cancel: PreparedRegularCancel,
    ) -> Result<OkxOrderAck, CancelOrderTransportError> {
        unreachable!("the action benchmark stops at prepared authority")
    }

    async fn cancel_regular_order_via_rest(
        &self,
        _cancel: PreparedRegularCancel,
    ) -> Result<OkxOrderAck, OrderTransportError> {
        unreachable!("the action benchmark stops at prepared authority")
    }
}

#[async_trait]
impl RegularReconciliation for BenchmarkGatewayRoles {
    async fn regular_pending_orders_page(
        &self,
        _instrument_type: Option<&str>,
        _symbol: Option<&str>,
        _after: Option<&str>,
    ) -> Result<OkxRegularOrderPage, RestError> {
        unreachable!("the action benchmark does not reconcile")
    }

    async fn recent_fills_page(
        &self,
        _instrument_type: Option<&str>,
        _symbol: Option<&str>,
        _after: Option<&str>,
    ) -> Result<OkxFillPage, RestError> {
        unreachable!("the action benchmark does not reconcile")
    }

    async fn account_balance(&self) -> Result<AccountUpdate, RestError> {
        unreachable!("the action benchmark does not reconcile")
    }

    async fn account_positions(&self) -> Result<AccountUpdate, RestError> {
        unreachable!("the action benchmark does not reconcile")
    }

    async fn order_details(
        &self,
        _symbol: &str,
        _client_order_id: &str,
    ) -> Result<RemoteOrder, RestError> {
        unreachable!("the action benchmark does not reconcile")
    }

    async fn server_time_ms(&self) -> Result<u64, RestError> {
        unreachable!("the action benchmark does not reconcile")
    }
}
