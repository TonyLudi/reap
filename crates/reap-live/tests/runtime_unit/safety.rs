use super::*;

#[derive(Debug, Clone, PartialEq, Eq)]
struct RecordedRoleRequest {
    path: String,
    body: String,
}

struct SafetyMockPort {
    responses: Arc<Mutex<VecDeque<Result<String, RestError>>>>,
    requests: Arc<Mutex<Vec<RecordedRoleRequest>>>,
}

impl SafetyMockPort {
    fn next(&self, path: impl Into<String>, body: impl Into<String>) -> Result<String, RestError> {
        self.requests.lock().unwrap().push(RecordedRoleRequest {
            path: path.into(),
            body: body.into(),
        });
        self.responses
            .lock()
            .unwrap()
            .pop_front()
            .expect("mock role response")
    }
}

#[async_trait]
impl ReadinessPort for SafetyMockPort {
    async fn server_time_ms(&self) -> Result<u64, RestError> {
        let body = self.next("/api/v5/public/time", "")?;
        parse_okx_server_time_response_json(body.as_bytes())
    }

    async fn system_status(&self) -> Result<Vec<OkxSystemStatus>, RestError> {
        let body = self.next("/api/v5/system/status", "")?;
        parse_okx_system_status_response_json(body.as_bytes())
    }

    async fn account_config(&self) -> Result<OkxAccountConfig, RestError> {
        let body = self.next("/api/v5/account/config", "")?;
        parse_okx_account_config_response_json(body.as_bytes())
    }

    async fn account_balance_snapshot(
        &self,
    ) -> Result<reap_venue::okx::OkxAccountBalanceSnapshot, RestError> {
        let body = self.next("/api/v5/account/balance", "")?;
        parse_okx_account_balance_response_json(body.as_bytes())
    }

    async fn account_positions_snapshot(
        &self,
        _instrument_type: Option<OkxInstrumentType>,
        _symbol: Option<&str>,
    ) -> Result<reap_venue::okx::OkxAccountPositionsSnapshot, RestError> {
        let body = self.next("/api/v5/account/positions", "")?;
        parse_okx_account_positions_response_json(body.as_bytes())
    }

    async fn account_instrument(
        &self,
        instrument_type: OkxInstrumentType,
        symbol: &str,
    ) -> Result<OkxInstrument, RestError> {
        let path = format!(
            "/api/v5/account/instruments?instType={}&instId={symbol}",
            instrument_type.as_str()
        );
        let body = self.next(path, "")?;
        parse_okx_account_instruments_response_json(body.as_bytes())?
            .into_iter()
            .next()
            .ok_or(RestError::EmptyData {
                operation: "account instrument",
            })
    }

    async fn account_trade_fee(
        &self,
        instrument_type: OkxInstrumentType,
        instrument_id: Option<&str>,
        instrument_family: Option<&str>,
        group_id: &str,
    ) -> Result<OkxTradeFeeRate, RestError> {
        let selector = instrument_id
            .map(|value| format!("&instId={value}"))
            .or_else(|| instrument_family.map(|value| format!("&instFamily={value}")))
            .unwrap_or_default();
        let path = format!(
            "/api/v5/account/trade-fee?instType={}{}",
            instrument_type.as_str(),
            selector
        );
        let body = self.next(path, "")?;
        parse_okx_trade_fee_response_json(body.as_bytes())?
            .into_iter()
            .find(|rate| rate.group_id == group_id)
            .ok_or(RestError::EmptyData {
                operation: "account trade fee",
            })
    }
}

#[async_trait]
impl SafetyPort for SafetyMockPort {
    async fn cancel_all_after(&self, timeout_secs: u64) -> Result<(), RestError> {
        let body = format!(r#"{{"timeOut":"{timeout_secs}"}}"#);
        let response = self.next("/api/v5/trade/cancel-all-after", body)?;
        parse_okx_cancel_all_after_response_json(response.as_bytes(), timeout_secs)
    }
}

struct BlockingFeePort {
    fee_started: Arc<Notify>,
}

#[async_trait]
impl ReadinessPort for BlockingFeePort {
    async fn account_instrument(
        &self,
        _instrument_type: OkxInstrumentType,
        _symbol: &str,
    ) -> Result<OkxInstrument, RestError> {
        Ok(
            parse_okx_account_instruments_response_json(spot_instrument_response().as_bytes())?
                .into_iter()
                .next()
                .unwrap(),
        )
    }

    async fn account_trade_fee(
        &self,
        _instrument_type: OkxInstrumentType,
        _instrument_id: Option<&str>,
        _instrument_family: Option<&str>,
        _group_id: &str,
    ) -> Result<OkxTradeFeeRate, RestError> {
        self.fee_started.notify_one();
        std::future::pending().await
    }
}

struct BlockingInstrumentPort {
    instrument_started: Arc<Notify>,
}

#[async_trait]
impl ReadinessPort for BlockingInstrumentPort {
    async fn account_instrument(
        &self,
        _instrument_type: OkxInstrumentType,
        _symbol: &str,
    ) -> Result<OkxInstrument, RestError> {
        self.instrument_started.notify_one();
        std::future::pending().await
    }

    async fn account_trade_fee(
        &self,
        _instrument_type: OkxInstrumentType,
        _instrument_id: Option<&str>,
        _instrument_family: Option<&str>,
        _group_id: &str,
    ) -> Result<OkxTradeFeeRate, RestError> {
        panic!("fee should not run while the instrument request is blocked")
    }
}

struct NotifyingSafety {
    deadman_seen: Arc<Notify>,
}

#[async_trait]
impl SafetyPort for NotifyingSafety {
    async fn cancel_all_after(&self, _timeout_secs: u64) -> Result<(), RestError> {
        self.deadman_seen.notify_one();
        Ok(())
    }
}

fn safety_client(
    responses: Vec<Result<&str, RestError>>,
) -> (Arc<SafetyMockPort>, Arc<Mutex<Vec<RecordedRoleRequest>>>) {
    let requests = Arc::new(Mutex::new(Vec::new()));
    let port = Arc::new(SafetyMockPort {
        responses: Arc::new(Mutex::new(
            responses
                .into_iter()
                .map(|response| response.map(str::to_string))
                .collect(),
        )),
        requests: Arc::clone(&requests),
    });
    (port, requests)
}

fn safety_account_config() -> OkxAccountConfig {
    OkxAccountConfig {
        account_level: OkxAccountLevel::SingleCurrencyMargin,
        position_mode: OkxPositionMode::NetMode,
        account_stp_mode: "cancel_maker".to_string(),
        user_id: "7".to_string(),
        main_user_id: "6".to_string(),
        api_key_label: "reap-demo".to_string(),
        api_key_permissions: BTreeSet::from([
            OkxApiKeyPermission::ReadOnly,
            OkxApiKeyPermission::Trade,
        ]),
        api_key_ip_bindings: BTreeSet::from(["203.0.113.5".to_string()]),
        enable_spot_borrow: Some(false),
        auto_loan: Some(false),
        spot_borrow_auto_repay: Some(false),
    }
}

fn exchange_status_guard(enabled: bool, check_interval_ms: u64) -> ExchangeStatusGuard {
    ExchangeStatusGuard {
        enabled,
        relevance: ChaosConnectivityPlan::resolve(&config(), LiveMode::Demo)
            .unwrap()
            .maintenance_relevance()
            .clone(),
        check_interval_ms,
        lead_ms: 60_000,
    }
}

fn exchange_instrument_guard(
    check_interval_ms: u64,
    expectations: Vec<ExchangeInstrumentExpectation>,
) -> ExchangeInstrumentGuard {
    ExchangeInstrumentGuard {
        sweep_interval_ms: check_interval_ms,
        change_lead_ms: 3_600_000,
        expectations,
    }
}

fn exchange_instrument_expectation(
    configured_maker_cost: f64,
    configured_taker_cost: f64,
) -> ExchangeInstrumentExpectation {
    ExchangeInstrumentExpectation {
        symbol: "BTC-USDT".to_string(),
        instrument_type: OkxInstrumentType::Spot,
        instrument_id: Some("BTC-USDT".to_string()),
        instrument_family: None,
        group_id: "1".to_string(),
        configured_maker_cost,
        configured_taker_cost,
        expected_instrument: spot_instrument(Vec::new()),
    }
}

fn spot_instrument(upcoming_changes: Vec<OkxInstrumentChange>) -> OkxInstrument {
    let mut instrument =
        parse_okx_account_instruments_response_json(spot_instrument_response().as_bytes())
            .expect("spot instrument fixture must parse")
            .pop()
            .expect("spot instrument fixture must contain one row");
    instrument.upcoming_changes = upcoming_changes;
    instrument
}

fn spot_instrument_response() -> &'static str {
    r#"{"code":"0","msg":"","data":[{"instId":"BTC-USDT","instType":"SPOT","instFamily":"","groupId":"1","baseCcy":"BTC","quoteCcy":"USDT","settleCcy":"","ctType":"","ctVal":"","ctValCcy":"","tickSz":"0.1","lotSz":"0.001","minSz":"0.001","maxLmtSz":"100","maxMktSz":"1000000","maxLmtAmt":"1000000","maxMktAmt":"1000000","state":"live","upcChg":[]}]}"#
}

#[tokio::test]
async fn safety_task_disables_deadman_only_on_explicit_command() {
    let (client, requests) = safety_client(vec![Ok(
        r#"{"code":"0","msg":"","data":[{"triggerTime":"0","tag":"","ts":"1"}]}"#,
    )]);
    let (command_tx, command_rx) = mpsc::channel(2);
    let (event_tx, _event_rx) = test_runtime_event_channel(2);
    let task = tokio::spawn(run_account_safety_task(
        "main".to_string(),
        client.clone(),
        Some(client),
        safety_account_config(),
        command_rx,
        event_tx,
        Some(30),
        60_000,
        60_000,
        1_000,
        exchange_status_guard(false, 60_000),
        exchange_instrument_guard(60_000, Vec::new()),
    ));
    let (result_tx, result_rx) = oneshot::channel();
    command_tx
        .send(SafetyTaskCommand::DisableDeadMan { result: result_tx })
        .await
        .unwrap();
    result_rx.await.unwrap().unwrap();
    command_tx.send(SafetyTaskCommand::Shutdown).await.unwrap();
    task.await.unwrap();

    let requests = requests.lock().unwrap();
    assert_eq!(requests.len(), 1);
    assert_eq!(requests[0].path, "/api/v5/trade/cancel-all-after");
    assert_eq!(requests[0].body, r#"{"timeOut":"0"}"#);
}

#[tokio::test]
async fn deadman_heartbeat_failure_is_fatal() {
    let (client, _) = safety_client(vec![Err(RestError::Transport(
        "injected heartbeat failure".to_string(),
    ))]);
    let (_command_tx, command_rx) = mpsc::channel(2);
    let (event_tx, mut event_rx) = test_runtime_event_channel(2);
    let task = tokio::spawn(run_account_safety_task(
        "main".to_string(),
        client.clone(),
        Some(client),
        safety_account_config(),
        command_rx,
        event_tx,
        Some(30),
        1,
        60_000,
        1_000,
        exchange_status_guard(false, 60_000),
        exchange_instrument_guard(60_000, Vec::new()),
    ));

    let event = tokio::time::timeout(Duration::from_millis(100), event_rx.recv())
        .await
        .unwrap()
        .unwrap();
    assert!(matches!(
        event,
        RuntimeEvent::Fatal(RuntimeTaskFailure::DeadmanHeartbeat(message))
            if message.contains("injected heartbeat failure")
    ));
    task.await.unwrap();
}

#[tokio::test]
async fn blocked_exchange_fee_check_does_not_delay_deadman_heartbeat() {
    let fee_started = Arc::new(Notify::new());
    let deadman_seen = Arc::new(Notify::new());
    let readiness: Arc<dyn ReadinessPort> = Arc::new(BlockingFeePort {
        fee_started: Arc::clone(&fee_started),
    });
    let safety: Arc<dyn SafetyPort> = Arc::new(NotifyingSafety {
        deadman_seen: Arc::clone(&deadman_seen),
    });
    let (command_tx, command_rx) = mpsc::channel(2);
    let (event_tx, mut event_rx) = test_runtime_event_channel(2);
    let task = tokio::spawn(run_account_safety_task(
        "main".to_string(),
        readiness,
        Some(safety),
        safety_account_config(),
        command_rx,
        event_tx,
        Some(30),
        500,
        60_000,
        1_000,
        exchange_status_guard(false, 60_000),
        exchange_instrument_guard(400, vec![exchange_instrument_expectation(0.001, 0.001)]),
    ));

    tokio::time::timeout(Duration::from_secs(1), fee_started.notified())
        .await
        .expect("fee request did not start");
    tokio::time::timeout(Duration::from_secs(1), deadman_seen.notified())
        .await
        .expect("deadman heartbeat was blocked by fee request");
    assert!(event_rx.try_recv().is_err());

    command_tx.send(SafetyTaskCommand::Shutdown).await.unwrap();
    task.await.unwrap();
}

#[tokio::test]
async fn blocked_exchange_instrument_check_does_not_delay_deadman_heartbeat() {
    let instrument_started = Arc::new(Notify::new());
    let deadman_seen = Arc::new(Notify::new());
    let readiness: Arc<dyn ReadinessPort> = Arc::new(BlockingInstrumentPort {
        instrument_started: Arc::clone(&instrument_started),
    });
    let safety: Arc<dyn SafetyPort> = Arc::new(NotifyingSafety {
        deadman_seen: Arc::clone(&deadman_seen),
    });
    let (command_tx, command_rx) = mpsc::channel(2);
    let (event_tx, mut event_rx) = test_runtime_event_channel(2);
    let task = tokio::spawn(run_account_safety_task(
        "main".to_string(),
        readiness,
        Some(safety),
        safety_account_config(),
        command_rx,
        event_tx,
        Some(30),
        500,
        60_000,
        1_000,
        exchange_status_guard(false, 60_000),
        exchange_instrument_guard(400, vec![exchange_instrument_expectation(0.001, 0.001)]),
    ));

    tokio::time::timeout(Duration::from_secs(1), instrument_started.notified())
        .await
        .expect("instrument request did not start");
    tokio::time::timeout(Duration::from_secs(1), deadman_seen.notified())
        .await
        .expect("deadman heartbeat was blocked by instrument request");
    assert!(event_rx.try_recv().is_err());

    command_tx.send(SafetyTaskCommand::Shutdown).await.unwrap();
    task.await.unwrap();
}

#[tokio::test]
async fn periodic_exchange_clock_skew_has_a_typed_failure() {
    let (client, requests) =
        safety_client(vec![Ok(r#"{"code":"0","msg":"","data":[{"ts":"0"}]}"#)]);
    let (_command_tx, command_rx) = mpsc::channel(2);
    let (event_tx, mut event_rx) = test_runtime_event_channel(2);
    let task = tokio::spawn(run_account_safety_task(
        "main".to_string(),
        client,
        None,
        safety_account_config(),
        command_rx,
        event_tx,
        None,
        60_000,
        1,
        1,
        exchange_status_guard(false, 60_000),
        exchange_instrument_guard(60_000, Vec::new()),
    ));

    let event = tokio::time::timeout(Duration::from_millis(100), event_rx.recv())
        .await
        .unwrap()
        .unwrap();
    assert!(matches!(
        event,
        RuntimeEvent::Fatal(RuntimeTaskFailure::ExchangeClockSkew(message))
            if message.contains("maximum is 1ms")
    ));
    task.await.unwrap();
    assert_eq!(requests.lock().unwrap().len(), 1);
}

#[tokio::test]
async fn account_configuration_drift_is_fatal() {
    let (client, requests) = safety_client(vec![
        Ok(r#"{"code":"0","msg":"","data":[{"ts":"0"}]}"#),
        Ok(
            r#"{"code":"0","msg":"","data":[{"acctLv":"2","posMode":"net_mode","acctStpMode":"cancel_maker","uid":"7","mainUid":"6","label":"reap-demo","perm":"read_only,trade","ip":"203.0.113.5","enableSpotBorrow":true,"autoLoan":false,"spotBorrowAutoRepay":false}]}"#,
        ),
    ]);
    let (_command_tx, command_rx) = mpsc::channel(2);
    let (event_tx, mut event_rx) = test_runtime_event_channel(2);
    let task = tokio::spawn(run_account_safety_task(
        "main".to_string(),
        client,
        None,
        safety_account_config(),
        command_rx,
        event_tx,
        None,
        60_000,
        1,
        u64::MAX,
        exchange_status_guard(false, 60_000),
        exchange_instrument_guard(60_000, Vec::new()),
    ));

    let event = tokio::time::timeout(Duration::from_millis(100), event_rx.recv())
        .await
        .unwrap()
        .unwrap();
    assert!(matches!(
        event,
        RuntimeEvent::Fatal(RuntimeTaskFailure::AccountConfigDrift(message))
            if message.contains("configuration or authenticated identity differs")
    ));
    task.await.unwrap();
    assert_eq!(requests.lock().unwrap().len(), 2);
}

#[tokio::test]
async fn account_stp_mode_drift_is_fatal() {
    let (client, requests) = safety_client(vec![
        Ok(r#"{"code":"0","msg":"","data":[{"ts":"0"}]}"#),
        Ok(
            r#"{"code":"0","msg":"","data":[{"acctLv":"2","posMode":"net_mode","acctStpMode":"cancel_taker","uid":"7","mainUid":"6","label":"reap-demo","perm":"read_only,trade","ip":"203.0.113.5","enableSpotBorrow":false,"autoLoan":false,"spotBorrowAutoRepay":false}]}"#,
        ),
    ]);
    let (_command_tx, command_rx) = mpsc::channel(2);
    let (event_tx, mut event_rx) = test_runtime_event_channel(2);
    let task = tokio::spawn(run_account_safety_task(
        "main".to_string(),
        client,
        None,
        safety_account_config(),
        command_rx,
        event_tx,
        None,
        60_000,
        1,
        u64::MAX,
        exchange_status_guard(false, 60_000),
        exchange_instrument_guard(60_000, Vec::new()),
    ));

    let event = tokio::time::timeout(Duration::from_millis(100), event_rx.recv())
        .await
        .unwrap()
        .unwrap();
    assert!(matches!(
        event,
        RuntimeEvent::Fatal(RuntimeTaskFailure::AccountConfigDrift(message))
            if message.contains("configuration or authenticated identity differs")
    ));
    task.await.unwrap();
    assert_eq!(requests.lock().unwrap().len(), 2);
}

#[test]
fn exchange_status_guard_matches_planned_service_scope_and_environment() {
    let relevance = ChaosConnectivityPlan::resolve(&config(), LiveMode::Demo)
        .unwrap()
        .maintenance_relevance()
        .clone();
    let status = |service_type, environment, state, begin_time_ms| OkxSystemStatus {
        title: "maintenance".to_string(),
        description: String::new(),
        state,
        begin_time_ms,
        end_time_ms: begin_time_ms.saturating_add(60_000),
        pre_open_begin_time_ms: None,
        service_type,
        maintenance_type: reap_venue::okx::OkxSystemMaintenanceType::Scheduled,
        environment,
        system: "unified".to_string(),
    };
    let now_ms = 1_000_000;
    let lead_ms = 60_000;

    let trading = status(
        OkxSystemServiceType::Trading,
        OkxSystemEnvironment::Demo,
        OkxSystemStatusState::Scheduled,
        now_ms + lead_ms,
    );
    assert!(exchange_status_block_reason(&[trading], &relevance, now_ms, lead_ms).is_some());

    let too_early = status(
        OkxSystemServiceType::TradingAccounts,
        OkxSystemEnvironment::Demo,
        OkxSystemStatusState::Scheduled,
        now_ms + lead_ms + 1,
    );
    assert!(exchange_status_block_reason(&[too_early], &relevance, now_ms, lead_ms).is_none());

    let copy_trading = status(
        OkxSystemServiceType::CopyTrading,
        OkxSystemEnvironment::Demo,
        OkxSystemStatusState::Ongoing,
        1,
    );
    let production = status(
        OkxSystemServiceType::Trading,
        OkxSystemEnvironment::Production,
        OkxSystemStatusState::Ongoing,
        1,
    );
    let completed = status(
        OkxSystemServiceType::TradingProducts,
        OkxSystemEnvironment::Demo,
        OkxSystemStatusState::Completed,
        1,
    );
    assert!(
        exchange_status_block_reason(
            &[copy_trading, production, completed],
            &relevance,
            now_ms,
            lead_ms
        )
        .is_none()
    );
}

#[test]
fn spread_only_ongoing_maintenance_is_irrelevant_to_the_planned_scope() {
    let relevance = ChaosConnectivityPlan::resolve(&config(), LiveMode::Demo)
        .unwrap()
        .maintenance_relevance()
        .clone();
    let spread = OkxSystemStatus {
        title: "spread maintenance".to_string(),
        description: String::new(),
        state: OkxSystemStatusState::Ongoing,
        begin_time_ms: 1,
        end_time_ms: 60_001,
        pre_open_begin_time_ms: None,
        service_type: OkxSystemServiceType::SpreadTrading,
        maintenance_type: reap_venue::okx::OkxSystemMaintenanceType::Scheduled,
        environment: OkxSystemEnvironment::Demo,
        system: "unified".to_string(),
    };

    assert!(exchange_status_block_reason(&[spread], &relevance, 1_000_000, 60_000).is_none());
}

#[test]
fn ambiguous_ongoing_maintenance_blocks_the_planned_scope() {
    let relevance = ChaosConnectivityPlan::resolve(&config(), LiveMode::Demo)
        .unwrap()
        .maintenance_relevance()
        .clone();
    let ambiguous = OkxSystemStatus {
        title: "ambiguous maintenance".to_string(),
        description: String::new(),
        state: OkxSystemStatusState::Ongoing,
        begin_time_ms: 1,
        end_time_ms: 60_001,
        pre_open_begin_time_ms: None,
        service_type: OkxSystemServiceType::Other,
        maintenance_type: reap_venue::okx::OkxSystemMaintenanceType::Scheduled,
        environment: OkxSystemEnvironment::Demo,
        system: "unified".to_string(),
    };

    assert!(exchange_status_block_reason(&[ambiguous], &relevance, 1_000_000, 60_000).is_some());
}

#[test]
fn exchange_fee_guard_converts_signed_rates_and_allows_conservative_config() {
    let mut expectation = exchange_instrument_expectation(0.0002, 0.0005);
    let rate = OkxTradeFeeRate {
        instrument_type: OkxInstrumentType::Spot,
        group_id: "1".to_string(),
        level: "Lv1".to_string(),
        maker_rate: -0.0002,
        taker_rate: -0.0005,
        timestamp_ms: 1,
    };
    assert!(exchange_fee_drift_reason(&expectation, &rate).is_none());

    expectation.configured_maker_cost = 0.0003;
    expectation.configured_taker_cost = 0.0006;
    assert!(exchange_fee_drift_reason(&expectation, &rate).is_none());

    expectation.configured_maker_cost = 0.0001;
    let reason = exchange_fee_drift_reason(&expectation, &rate).unwrap();
    assert!(reason.contains("understate authenticated costs"));

    expectation.configured_maker_cost = -0.0002;
    let rebate = OkxTradeFeeRate {
        maker_rate: 0.0001,
        ..rate
    };
    assert!(exchange_fee_drift_reason(&expectation, &rebate).is_some());
    expectation.configured_maker_cost = 0.0;
    assert!(exchange_fee_drift_reason(&expectation, &rebate).is_none());
}

#[test]
fn exchange_instrument_guard_detects_rule_drift_and_announced_changes() {
    let expectation = exchange_instrument_expectation(0.001, 0.001);
    let mut current = expectation.expected_instrument.clone();
    assert!(exchange_instrument_drift_reason(&expectation, &current, 1_000, 100).is_none());

    current.tick_size = 0.01;
    let reason = exchange_instrument_drift_reason(&expectation, &current, 1_000, 100).unwrap();
    assert!(reason.contains("tick size changed"));

    let exact_drift_response = spot_instrument_response()
        .replace(r#""tickSz":"0.1""#, r#""tickSz":"0.10000000000000001""#);
    current = parse_okx_account_instruments_response_json(exact_drift_response.as_bytes())
        .unwrap()
        .remove(0);
    assert_eq!(
        current.tick_size.to_bits(),
        expectation.expected_instrument.tick_size.to_bits(),
        "the binary float companion deliberately cannot distinguish this drift"
    );
    let reason = exchange_instrument_drift_reason(&expectation, &current, 1_000, 100).unwrap();
    assert!(reason.contains("exact regular-order rules changed"));

    current =
        serde_json::from_value(serde_json::to_value(&expectation.expected_instrument).unwrap())
            .unwrap();
    let reason = exchange_instrument_drift_reason(&expectation, &current, 1_000, 100).unwrap();
    assert!(reason.contains("exact regular-order rules changed"));

    current = expectation.expected_instrument.clone();
    current.max_limit_size = 0.5;
    let reason = exchange_instrument_drift_reason(&expectation, &current, 1_000, 100).unwrap();
    assert!(reason.contains("maximum limit-order size changed"));

    current = expectation.expected_instrument.clone();
    current.upcoming_changes.push(OkxInstrumentChange {
        parameter: OkxInstrumentChangeParameter::MinimumSize,
        new_value: 0.01,
        effective_time_ms: 1_101,
    });
    assert!(exchange_instrument_drift_reason(&expectation, &current, 1_000, 100).is_none());
    current.upcoming_changes[0].effective_time_ms = 1_100;
    let reason = exchange_instrument_drift_reason(&expectation, &current, 1_000, 100).unwrap();
    assert!(reason.contains("announced minSz change"));

    current = expectation.expected_instrument.clone();
    current.state = "post_only".to_string();
    let reason = exchange_instrument_drift_reason(&expectation, &current, 1_000, 100).unwrap();
    assert!(reason.contains("state changed"));
}

#[test]
fn initial_announced_instrument_change_is_typed() {
    let mut expectation = exchange_instrument_expectation(0.001, 0.001);
    expectation
        .expected_instrument
        .upcoming_changes
        .push(OkxInstrumentChange {
            parameter: OkxInstrumentChangeParameter::TickSize,
            new_value: 0.01,
            effective_time_ms: 1_100,
        });
    let mut guard = exchange_instrument_guard(400, vec![expectation]);
    guard.change_lead_ms = 100;

    let error = verify_initial_exchange_instruments("main", &guard, 1_000).unwrap_err();

    assert!(matches!(
        error,
        LiveRuntimeError::ExchangeInstrumentDrift(message)
            if message.contains("account main") && message.contains("tickSz")
    ));
}

#[test]
fn exchange_fee_request_spacing_finishes_within_the_sweep_deadline() {
    assert_eq!(exchange_fee_request_interval_ms(1_001, 2), 500);
    assert_eq!(exchange_fee_request_interval_ms(800, 2), 400);
}

#[tokio::test]
async fn initial_exchange_fee_understatement_is_typed() {
    let (client, requests) = safety_client(vec![Ok(
        r#"{"code":"0","msg":"","data":[{"feeGroup":[{"groupId":"1","maker":"-0.0008","taker":"-0.001"}],"instType":"SPOT","level":"Lv1","ts":"1763979985847"}]}"#,
    )]);
    let guard = exchange_instrument_guard(
        60_000,
        vec![exchange_instrument_expectation(0.0002, 0.0005)],
    );

    let error = verify_initial_exchange_fees("main", client.as_ref(), &guard)
        .await
        .unwrap_err();

    assert!(matches!(
        error,
        LiveRuntimeError::ExchangeFeeDrift(message)
            if message.contains("account main") && message.contains("BTC-USDT")
    ));
    assert_eq!(requests.lock().unwrap().len(), 1);
}

#[tokio::test]
async fn periodic_exchange_fee_understatement_is_fatal() {
    let (client, requests) = safety_client(vec![
        Ok(spot_instrument_response()),
        Ok(
            r#"{"code":"0","msg":"","data":[{"feeGroup":[{"groupId":"1","maker":"-0.0008","taker":"-0.001"}],"instType":"SPOT","level":"Lv1","ts":"1763979985847"}]}"#,
        ),
    ]);
    let (_command_tx, command_rx) = mpsc::channel(2);
    let (event_tx, mut event_rx) = test_runtime_event_channel(2);
    let task = tokio::spawn(run_account_safety_task(
        "main".to_string(),
        client,
        None,
        safety_account_config(),
        command_rx,
        event_tx,
        None,
        60_000,
        60_000,
        1_000,
        exchange_status_guard(false, 60_000),
        exchange_instrument_guard(400, vec![exchange_instrument_expectation(0.0002, 0.0005)]),
    ));

    let event = tokio::time::timeout(Duration::from_secs(1), event_rx.recv())
        .await
        .unwrap()
        .unwrap();
    assert!(matches!(
        event,
        RuntimeEvent::Fatal(RuntimeTaskFailure::ExchangeFeeDrift(message))
            if message.contains("BTC-USDT") && message.contains("0.001")
    ));
    task.await.unwrap();
    let requests = requests.lock().unwrap();
    assert_eq!(requests.len(), 2);
    assert_eq!(
        requests[0].path,
        "/api/v5/account/instruments?instType=SPOT&instId=BTC-USDT"
    );
    assert_eq!(
        requests[1].path,
        "/api/v5/account/trade-fee?instType=SPOT&instId=BTC-USDT"
    );
}

#[tokio::test]
async fn periodic_exchange_fee_check_failure_is_typed() {
    let (client, _) = safety_client(vec![
        Ok(spot_instrument_response()),
        Err(RestError::Transport("injected fee failure".to_string())),
    ]);
    let (_command_tx, command_rx) = mpsc::channel(2);
    let (event_tx, mut event_rx) = test_runtime_event_channel(2);
    let task = tokio::spawn(run_account_safety_task(
        "main".to_string(),
        client,
        None,
        safety_account_config(),
        command_rx,
        event_tx,
        None,
        60_000,
        60_000,
        1_000,
        exchange_status_guard(false, 60_000),
        exchange_instrument_guard(400, vec![exchange_instrument_expectation(0.001, 0.001)]),
    ));

    let event = tokio::time::timeout(Duration::from_secs(1), event_rx.recv())
        .await
        .unwrap()
        .unwrap();
    assert!(matches!(
        event,
        RuntimeEvent::Fatal(RuntimeTaskFailure::ExchangeFeeCheck(message))
            if message.contains("injected fee failure")
    ));
    task.await.unwrap();
}

#[tokio::test]
async fn periodic_exchange_instrument_maximum_drift_is_fatal() {
    let changed = r#"{"code":"0","msg":"","data":[{"instId":"BTC-USDT","instType":"SPOT","instFamily":"","groupId":"1","baseCcy":"BTC","quoteCcy":"USDT","settleCcy":"","ctType":"","ctVal":"","ctValCcy":"","tickSz":"0.1","lotSz":"0.001","minSz":"0.001","maxLmtSz":"99","maxMktSz":"1000000","maxLmtAmt":"1000000","maxMktAmt":"1000000","state":"live","upcChg":[]}]}"#;
    let (client, requests) = safety_client(vec![Ok(changed)]);
    let (_command_tx, command_rx) = mpsc::channel(2);
    let (event_tx, mut event_rx) = test_runtime_event_channel(2);
    let task = tokio::spawn(run_account_safety_task(
        "main".to_string(),
        client,
        None,
        safety_account_config(),
        command_rx,
        event_tx,
        None,
        60_000,
        60_000,
        1_000,
        exchange_status_guard(false, 60_000),
        exchange_instrument_guard(400, vec![exchange_instrument_expectation(0.001, 0.001)]),
    ));

    let event = tokio::time::timeout(Duration::from_secs(1), event_rx.recv())
        .await
        .unwrap()
        .unwrap();
    assert!(matches!(
        event,
        RuntimeEvent::Fatal(RuntimeTaskFailure::ExchangeInstrumentDrift(message))
            if message.contains("maximum limit-order size changed")
    ));
    task.await.unwrap();
    assert_eq!(requests.lock().unwrap().len(), 1);
}

#[tokio::test]
async fn periodic_exchange_instrument_check_failure_is_typed() {
    let (client, _) = safety_client(vec![Err(RestError::Transport(
        "injected instrument failure".to_string(),
    ))]);
    let (_command_tx, command_rx) = mpsc::channel(2);
    let (event_tx, mut event_rx) = test_runtime_event_channel(2);
    let task = tokio::spawn(run_account_safety_task(
        "main".to_string(),
        client,
        None,
        safety_account_config(),
        command_rx,
        event_tx,
        None,
        60_000,
        60_000,
        1_000,
        exchange_status_guard(false, 60_000),
        exchange_instrument_guard(400, vec![exchange_instrument_expectation(0.001, 0.001)]),
    ));

    let event = tokio::time::timeout(Duration::from_secs(1), event_rx.recv())
        .await
        .unwrap()
        .unwrap();
    assert!(matches!(
        event,
        RuntimeEvent::Fatal(RuntimeTaskFailure::ExchangeInstrumentCheck(message))
            if message.contains("injected instrument failure")
    ));
    task.await.unwrap();
}

#[tokio::test]
async fn periodic_relevant_exchange_status_is_fatal() {
    let (client, requests) = safety_client(vec![Ok(
        r#"{"code":"0","msg":"","data":[{"begin":"1","end":"60001","env":"2","maintType":"2","serviceType":"5","state":"ongoing","system":"unified","title":"Trading maintenance"}]}"#,
    )]);
    let (_command_tx, command_rx) = mpsc::channel(2);
    let (event_tx, mut event_rx) = test_runtime_event_channel(2);
    let task = tokio::spawn(run_account_safety_task(
        "main".to_string(),
        client,
        None,
        safety_account_config(),
        command_rx,
        event_tx,
        None,
        60_000,
        60_000,
        1_000,
        exchange_status_guard(true, 1),
        exchange_instrument_guard(60_000, Vec::new()),
    ));

    let event = tokio::time::timeout(Duration::from_millis(100), event_rx.recv())
        .await
        .unwrap()
        .unwrap();
    assert!(matches!(
        event,
        RuntimeEvent::Fatal(RuntimeTaskFailure::ExchangeStatus(message))
            if message.contains("Trading maintenance")
    ));
    task.await.unwrap();
    let requests = requests.lock().unwrap();
    assert_eq!(requests.len(), 1);
    assert_eq!(requests[0].path, "/api/v5/system/status");
}

#[tokio::test]
async fn periodic_exchange_status_check_failure_is_typed() {
    let (client, _) = safety_client(vec![Err(RestError::Transport(
        "injected status failure".to_string(),
    ))]);
    let (_command_tx, command_rx) = mpsc::channel(2);
    let (event_tx, mut event_rx) = test_runtime_event_channel(2);
    let task = tokio::spawn(run_account_safety_task(
        "main".to_string(),
        client,
        None,
        safety_account_config(),
        command_rx,
        event_tx,
        None,
        60_000,
        60_000,
        1_000,
        exchange_status_guard(true, 1),
        exchange_instrument_guard(60_000, Vec::new()),
    ));

    let event = tokio::time::timeout(Duration::from_millis(100), event_rx.recv())
        .await
        .unwrap()
        .unwrap();
    assert!(matches!(
        event,
        RuntimeEvent::Fatal(RuntimeTaskFailure::ExchangeStatusCheck(message))
            if message.contains("injected status failure")
    ));
    task.await.unwrap();
}
