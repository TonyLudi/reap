use super::*;

async fn serve_one_alert(listener: tokio::net::TcpListener) {
    let (mut socket, _) = listener.accept().await.unwrap();
    let mut request = Vec::new();
    let mut buffer = [0_u8; 4_096];
    loop {
        let read = socket.read(&mut buffer).await.unwrap();
        assert!(read > 0, "alert client closed before sending a request");
        request.extend_from_slice(&buffer[..read]);
        let text = String::from_utf8_lossy(&request);
        let Some(header_end) = text.find("\r\n\r\n") else {
            continue;
        };
        let content_length = text[..header_end]
            .lines()
            .find_map(|line| {
                let (name, value) = line.split_once(':')?;
                name.eq_ignore_ascii_case("content-length")
                    .then(|| value.trim().parse::<usize>().ok())
                    .flatten()
            })
            .unwrap_or(0);
        if request.len() >= header_end + 4 + content_length {
            break;
        }
    }
    socket
        .write_all(b"HTTP/1.1 204 No Content\r\nContent-Length: 0\r\nConnection: close\r\n\r\n")
        .await
        .unwrap();
}

#[tokio::test]
async fn validation_report_is_not_a_soak_pass() {
    let report = run_live(
        config(),
        LiveRunOptions {
            mode: LiveMode::Validate,
            demo_confirmed: false,
            run_duration: None,
        },
    )
    .await
    .unwrap();

    assert_eq!(report.stop_reason, LiveStopReason::Validation);
    assert!(report.failure.is_none());
    assert!(!report.reached_ready);
    assert!(!report.clean_soak);
    assert_eq!(report.schema_version, LIVE_RUN_REPORT_SCHEMA_VERSION);
    assert_eq!(report.java_reference_revision, PINNED_JAVA_REVISION);
    assert_eq!(report.executable_sha256.len(), 64);
    assert!(report.host_identity_sha256.is_none());
    assert!(report.account_identity_sha256s.is_empty());
    assert!(report.latency_evidence.series.is_empty());
}

#[tokio::test]
async fn bounded_ready_runtime_completes_with_clean_soak_report() {
    let config = config();
    let now_ms = unix_time_ms();
    let coordinator = ready_coordinator(&config, now_ms, false);
    let path = std::env::temp_dir().join(format!(
        "reap-bounded-soak-{}-{}.jsonl",
        std::process::id(),
        unix_time_ns()
    ));
    let storage = start_jsonl_storage(StorageConfig {
        path: path.clone(),
        channel_capacity: 1_024,
        flush_every_records: 1,
    })
    .await
    .unwrap();
    let storage_sink = storage.sink();
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let alert_endpoint = format!("http://{}/alerts", listener.local_addr().unwrap());
    let alert_server = tokio::spawn(serve_one_alert(listener));
    let mut alert_runtime =
        start_webhook_alerts(reap_telemetry::WebhookAlertConfig::new(alert_endpoint)).unwrap();
    let alert_sink = alert_runtime.sink();
    let alert_failures = alert_runtime.take_failures();
    alert_sink
        .try_emit(AlertEvent::new(
            AlertSeverity::Warning,
            "test",
            "lifecycle_test",
            "test alert",
        ))
        .unwrap();
    let (control_tx, control_rx) = test_runtime_event_channel(16);
    let (feed_tx, feed_rx) = test_tracked_channel(QueueId::FeedIngress, 16);
    let (_forbidden_tx, forbidden_rx) = mpsc::channel(16);
    let runtime = TestRuntimeParts {
        session_id: "test-alert-session".to_string(),
        session_started_at_ms: unix_time_ms(),
        config_source: None,
        config_fingerprint: "test-config".to_string(),
        evidence_config_fingerprint: "test-evidence-config".to_string(),
        executable_sha256: "a".repeat(64),
        host_identity_sha256: None,
        account_identity_sha256s: BTreeMap::new(),
        mode: LiveMode::Observe,
        run_duration: Some(Duration::from_millis(25)),
        live_config: config.clone(),
        coordinator,
        processor: FeedProcessor::new(16, 16),
        storage: Some(storage),
        storage_sink,
        control_rx,
        feed_rx,
        forbidden_rx,
        order_senders: HashMap::new(),
        order_tasks: Vec::new(),
        reconcile_senders: HashMap::new(),
        reconcile_tasks: Vec::new(),
        order_ws_runtimes: Vec::new(),
        order_ws_status_tasks: Vec::new(),
        safety_senders: HashMap::new(),
        safety_tasks: Vec::new(),
        forbidden_tasks: Vec::new(),
        feeds: Vec::new(),
        feed_tasks: Vec::new(),
        sources: Vec::new(),
        public_feed_index: 0,
        reconcile_inflight: HashSet::new(),
        cancel_inflight: HashSet::new(),
        last_reconcile_attempt: HashMap::new(),
        fill_convergence: FillConvergenceGuard::default(),
        order_convergence: OrderStateConvergenceGuard::new(5_000),
        readiness_timeout_ms: 1_000,
        timer_interval_ms: 100,
        max_feed_age_ms: 60_000,
        shutdown_timeout_ms: 100,
        teardown_timeout_ms: 1_000,
        safety_latch_sync_timeout_ms: 1_000,
        evidence: RuntimeEvidence::default(),
        latency: LiveLatencyCollector::default(),
        shutdown_in_progress: false,
        shutdown_storage_error: None,
        preserve_deadman_on_shutdown: false,
        shutdown_reconciliation_requested: HashSet::new(),
        shutdown_reconciled_accounts: HashSet::new(),
        operator_service: None,
        operator_rx: None,
        operator_shutdown_reason: None,
        alert_runtime: Some(alert_runtime),
        alert_sink: Some(alert_sink),
        alert_failures: Some(alert_failures),
        alert_shutdown_timeout_ms: 1_000,
        alert_delivery_failure_is_fatal: true,
        observed_alert_delivery_failures: 0,
        alert_stats: AlertStats::default(),
        host_guard: None,
        host_failures: None,
        host_preflight: None,
        host_checks: 0,
        host_last_snapshot: None,
    }
    .into_runtime();

    let report = runtime.run().await.unwrap();
    tokio::time::timeout(Duration::from_secs(1), alert_server)
        .await
        .unwrap()
        .unwrap();
    drop(control_tx);
    drop(feed_tx);
    let _ = std::fs::remove_file(path);

    assert_eq!(report.stop_reason, LiveStopReason::DurationElapsed);
    assert!(report.elapsed_ms >= 20);
    assert!(report.reached_ready);
    assert_eq!(report.time_to_ready_ms, Some(0));
    assert!(report.readiness_at_stop.is_ready());
    assert_eq!(report.alerts_delivered, 1);
    assert_eq!(report.alert_delivery_failures, 0);
    assert_eq!(report.max_alert_queue_depth, 1);
    assert_eq!(report.reconciliation_drift_events, 0);
    assert_eq!(report.dropped_storage_records, 0);
    assert_eq!(report.active_orders_after_shutdown, 0);
    assert!(report.clean_soak);
}

#[tokio::test]
async fn stalled_teardown_is_aborted_and_reported_within_the_deadline() {
    struct AbortNotice(Option<oneshot::Sender<()>>);

    impl Drop for AbortNotice {
        fn drop(&mut self) {
            if let Some(sender) = self.0.take() {
                let _ = sender.send(());
            }
        }
    }

    let config = config();
    let coordinator = ready_coordinator(&config, unix_time_ms(), false);
    let path = std::env::temp_dir().join(format!(
        "reap-teardown-timeout-{}-{}.jsonl",
        std::process::id(),
        unix_time_ns()
    ));
    let storage = start_jsonl_storage(StorageConfig {
        path: path.clone(),
        channel_capacity: 1_024,
        flush_every_records: 1,
    })
    .await
    .unwrap();
    let storage_sink = storage.sink();
    let (control_tx, control_rx) = test_runtime_event_channel(16);
    let (feed_tx, feed_rx) = test_tracked_channel(QueueId::FeedIngress, 16);
    let (_forbidden_tx, forbidden_rx) = mpsc::channel(16);
    let (aborted_tx, aborted_rx) = oneshot::channel();
    let stalled_task = tokio::spawn(async move {
        let _notice = AbortNotice(Some(aborted_tx));
        std::future::pending::<()>().await;
    });
    let runtime = TestRuntimeParts {
        session_id: "test-teardown-timeout".to_string(),
        session_started_at_ms: unix_time_ms(),
        config_source: None,
        config_fingerprint: "test-config".to_string(),
        evidence_config_fingerprint: "test-evidence-config".to_string(),
        executable_sha256: "a".repeat(64),
        host_identity_sha256: None,
        account_identity_sha256s: BTreeMap::new(),
        mode: LiveMode::Observe,
        run_duration: Some(Duration::from_millis(5)),
        live_config: config.clone(),
        coordinator,
        processor: FeedProcessor::new(16, 16),
        storage: Some(storage),
        storage_sink,
        control_rx,
        feed_rx,
        forbidden_rx,
        order_senders: HashMap::new(),
        order_tasks: Vec::new(),
        reconcile_senders: HashMap::new(),
        reconcile_tasks: Vec::new(),
        order_ws_runtimes: Vec::new(),
        order_ws_status_tasks: Vec::new(),
        safety_senders: HashMap::new(),
        safety_tasks: Vec::new(),
        forbidden_tasks: Vec::new(),
        feeds: Vec::new(),
        feed_tasks: vec![stalled_task],
        sources: Vec::new(),
        public_feed_index: 0,
        reconcile_inflight: HashSet::new(),
        cancel_inflight: HashSet::new(),
        last_reconcile_attempt: HashMap::new(),
        fill_convergence: FillConvergenceGuard::default(),
        order_convergence: OrderStateConvergenceGuard::new(5_000),
        readiness_timeout_ms: 1_000,
        timer_interval_ms: 100,
        max_feed_age_ms: 60_000,
        shutdown_timeout_ms: 100,
        teardown_timeout_ms: 25,
        safety_latch_sync_timeout_ms: 1_000,
        evidence: RuntimeEvidence::default(),
        latency: LiveLatencyCollector::default(),
        shutdown_in_progress: false,
        shutdown_storage_error: None,
        preserve_deadman_on_shutdown: false,
        shutdown_reconciliation_requested: HashSet::new(),
        shutdown_reconciled_accounts: HashSet::new(),
        operator_service: None,
        operator_rx: None,
        operator_shutdown_reason: None,
        alert_runtime: None,
        alert_sink: None,
        alert_failures: None,
        alert_shutdown_timeout_ms: 100,
        alert_delivery_failure_is_fatal: true,
        observed_alert_delivery_failures: 0,
        alert_stats: AlertStats::default(),
        host_guard: None,
        host_failures: None,
        host_preflight: None,
        host_checks: 0,
        host_last_snapshot: None,
    }
    .into_runtime();

    let error = tokio::time::timeout(Duration::from_secs(1), runtime.run())
        .await
        .expect("teardown must honor its application deadline")
        .unwrap_err();
    drop(control_tx);
    drop(feed_tx);
    tokio::time::timeout(Duration::from_secs(1), aborted_rx)
        .await
        .expect("stalled task must be aborted")
        .expect("abort notice sender must remain live until cancellation");

    let LiveRuntimeError::ReportedFailure { source, report } = error else {
        panic!("teardown timeout must retain a typed run report");
    };
    assert!(matches!(*source, LiveRuntimeError::TeardownTimeout(25)));
    assert_eq!(report.stop_reason, LiveStopReason::RuntimeFailure);
    assert_eq!(report.failure.as_ref().unwrap().code, "teardown_timeout");
    assert!(!report.clean_soak);

    let lease = acquire_storage_lease(&path).expect("aborted writer must release journal lease");
    drop(lease);
    let _ = std::fs::remove_file(path.with_extension("jsonl.lock"));
    let _ = std::fs::remove_file(path);
}

#[cfg(unix)]
#[tokio::test]
async fn authenticated_operator_commands_run_on_event_loop_and_shutdown_cleanly() {
    use crate::operator::send_operator_command_with_secret;

    const SECRET: &[u8] = b"0123456789abcdef0123456789abcdef";

    let config = config();
    let now_ms = unix_time_ms();
    let coordinator = ready_coordinator(&config, now_ms, false);
    let storage_path = std::env::temp_dir().join(format!(
        "reap-operator-runtime-{}-{}.jsonl",
        std::process::id(),
        unix_time_ns()
    ));
    let socket_path = std::env::temp_dir().join(format!(
        "reap-operator-runtime-{}-{}.sock",
        std::process::id(),
        unix_time_ns()
    ));
    let operator_config = crate::OperatorConfig {
        enabled: true,
        socket_path: socket_path.clone(),
        request_timeout_ms: 1_000,
        ..crate::OperatorConfig::default()
    };
    let storage = start_jsonl_storage(StorageConfig {
        path: storage_path.clone(),
        channel_capacity: 1_024,
        flush_every_records: 1,
    })
    .await
    .unwrap();
    let storage_sink = storage.sink();
    let (control_tx, control_rx) = test_runtime_event_channel(16);
    let (feed_tx, feed_rx) = test_tracked_channel(QueueId::FeedIngress, 16);
    let (_forbidden_tx, forbidden_rx) = mpsc::channel(16);
    let (operator_tx, operator_rx) = mpsc::channel(16);
    let operator_service = start_operator_service(&operator_config, SECRET.to_vec(), operator_tx)
        .await
        .unwrap();
    let runtime = TestRuntimeParts {
        session_id: "test-operator-session".to_string(),
        session_started_at_ms: unix_time_ms(),
        config_source: None,
        config_fingerprint: "test-config".to_string(),
        evidence_config_fingerprint: "test-evidence-config".to_string(),
        executable_sha256: "a".repeat(64),
        host_identity_sha256: None,
        account_identity_sha256s: BTreeMap::new(),
        mode: LiveMode::Observe,
        run_duration: None,
        live_config: config.clone(),
        coordinator,
        processor: FeedProcessor::new(16, 16),
        storage: Some(storage),
        storage_sink,
        control_rx,
        feed_rx,
        forbidden_rx,
        order_senders: HashMap::new(),
        order_tasks: Vec::new(),
        reconcile_senders: HashMap::new(),
        reconcile_tasks: Vec::new(),
        order_ws_runtimes: Vec::new(),
        order_ws_status_tasks: Vec::new(),
        safety_senders: HashMap::new(),
        safety_tasks: Vec::new(),
        forbidden_tasks: Vec::new(),
        feeds: Vec::new(),
        feed_tasks: Vec::new(),
        sources: Vec::new(),
        public_feed_index: 0,
        reconcile_inflight: HashSet::new(),
        cancel_inflight: HashSet::new(),
        last_reconcile_attempt: HashMap::new(),
        fill_convergence: FillConvergenceGuard::default(),
        order_convergence: OrderStateConvergenceGuard::new(5_000),
        readiness_timeout_ms: 1_000,
        timer_interval_ms: 100,
        max_feed_age_ms: 60_000,
        shutdown_timeout_ms: 1_000,
        teardown_timeout_ms: 1_000,
        safety_latch_sync_timeout_ms: 1_000,
        evidence: RuntimeEvidence::default(),
        latency: LiveLatencyCollector::default(),
        shutdown_in_progress: false,
        shutdown_storage_error: None,
        preserve_deadman_on_shutdown: false,
        shutdown_reconciliation_requested: HashSet::new(),
        shutdown_reconciled_accounts: HashSet::new(),
        operator_service: Some(operator_service),
        operator_rx: Some(operator_rx),
        operator_shutdown_reason: None,
        alert_runtime: None,
        alert_sink: None,
        alert_failures: None,
        alert_shutdown_timeout_ms: 100,
        alert_delivery_failure_is_fatal: true,
        observed_alert_delivery_failures: 0,
        alert_stats: AlertStats::default(),
        host_guard: None,
        host_failures: None,
        host_preflight: None,
        host_checks: 0,
        host_last_snapshot: None,
    }
    .into_runtime();
    let runtime_task = tokio::spawn(runtime.run());

    let status =
        send_operator_command_with_secret(&operator_config, SECRET, OperatorCommand::Status)
            .await
            .unwrap();
    assert!(status.ok);
    let status = status.status.unwrap();
    assert!(status.readiness.is_ready());
    assert!(!status.kill_switch_active);
    assert!(status.halted_accounts.is_empty());

    let halt = send_operator_command_with_secret(
        &operator_config,
        SECRET,
        OperatorCommand::HaltSymbol {
            symbol: "BTC-USDT".to_string(),
            reason: "integration test".to_string(),
        },
    )
    .await
    .unwrap();
    assert!(halt.ok);
    let resume = send_operator_command_with_secret(
        &operator_config,
        SECRET,
        OperatorCommand::ResumeSymbol {
            symbol: "BTC-USDT".to_string(),
            reason: "integration test".to_string(),
        },
    )
    .await
    .unwrap();
    assert!(resume.ok);
    let account_kill = send_operator_command_with_secret(
        &operator_config,
        SECRET,
        OperatorCommand::KillAccount {
            account_id: "main".to_string(),
            reason: "integration account isolation".to_string(),
        },
    )
    .await
    .unwrap();
    assert!(account_kill.ok);
    assert!(
        account_kill
            .status
            .unwrap()
            .halted_accounts
            .contains_key("main")
    );
    let blocked_resume = send_operator_command_with_secret(
        &operator_config,
        SECRET,
        OperatorCommand::ResumeSymbol {
            symbol: "BTC-USDT".to_string(),
            reason: "must remain blocked".to_string(),
        },
    )
    .await
    .unwrap();
    assert!(!blocked_resume.ok);
    assert!(
        blocked_resume
            .message
            .contains("account kills cannot be reset live")
    );
    let kill = send_operator_command_with_secret(
        &operator_config,
        SECRET,
        OperatorCommand::KillSwitch {
            reason: "integration global stop".to_string(),
        },
    )
    .await
    .unwrap();
    assert!(kill.ok);
    assert!(kill.status.unwrap().kill_switch_active);
    let shutdown = send_operator_command_with_secret(
        &operator_config,
        SECRET,
        OperatorCommand::Shutdown {
            reason: "integration test complete".to_string(),
        },
    )
    .await
    .unwrap();
    assert!(shutdown.ok);
    assert!(shutdown.status.unwrap().shutdown_in_progress);

    let report = runtime_task.await.unwrap().unwrap();
    drop(control_tx);
    drop(feed_tx);
    let recovered = recover_jsonl(&storage_path).unwrap();
    assert!(recovered.global_safety_latch.is_some());
    assert!(recovered.account_safety_latches.contains_key("main"));
    assert!(recovered.symbol_safety_latches.is_empty());
    let _ = std::fs::remove_file(storage_path);

    assert_eq!(report.stop_reason, LiveStopReason::OperatorCommand);
    assert_eq!(report.operator_commands, 7);
    assert_eq!(report.operator_mutations, 5);
    assert_eq!(report.active_orders_after_shutdown, 0);
    assert!(!report.clean_soak);
    assert!(!socket_path.exists());
}

#[tokio::test]
async fn fatal_runtime_error_with_closed_storage_still_resolves_live_orders() {
    let config = config();
    let now_ms = unix_time_ms();
    let mut coordinator = ready_coordinator(&config, now_ms, true);
    coordinator
        .restore_owned_order(
            recovered_submit_proof("main", "BTC-USDT", "client-live"),
            OrderUpdate {
                ts_ms: now_ms,
                order_id: "client-live".to_string(),
                symbol: "BTC-USDT".to_string(),
                side: Side::Buy,
                event: OrderEvent::New,
                status: OrderStatus::Live,
                price: 100.0,
                time_in_force: Some(reap_core::TimeInForce::PostOnly),
                qty: 1.0,
                open_qty: 1.0,
                filled_qty: 0.0,
                avg_fill_price: 0.0,
                last_fill_qty: 0.0,
                last_fill_price: 0.0,
                last_fill_liquidity: None,
                last_fill_fee: None,
                reason: "test live order".to_string(),
            },
        )
        .unwrap();
    coordinator.set_order_entry_enabled(false);
    assert_eq!(coordinator.active_order_count(), 1);

    let path = std::env::temp_dir().join(format!(
        "reap-fail-closed-{}-{}.jsonl",
        std::process::id(),
        unix_time_ns()
    ));
    let storage = start_jsonl_storage(StorageConfig {
        path: path.clone(),
        channel_capacity: 1_024,
        flush_every_records: 1,
    })
    .await
    .unwrap();
    let storage_sink = storage.sink();
    storage.shutdown().await.unwrap();
    let (control_tx, control_rx) = test_runtime_event_channel(16);
    let (feed_tx, feed_rx) = test_tracked_channel(QueueId::FeedIngress, 16);
    let (_forbidden_tx, forbidden_rx) = mpsc::channel(16);
    let (order_tx, mut order_rx) = test_tracked_channel(QueueId::OrderCommand, 16);
    let (reconcile_tx, mut reconcile_rx) = mpsc::channel(16);
    let cancel_observed = Arc::new(AtomicBool::new(false));
    let reconcile_received = Arc::new(Notify::new());
    let task_cancel_observed = Arc::clone(&cancel_observed);
    let task_reconcile_received = Arc::clone(&reconcile_received);
    let order_task = tokio::spawn(async move {
        while let Some(command) = order_rx.recv().await {
            match command {
                OrderTaskCommand::Cancel { action, .. } => {
                    assert_eq!(action.client_order_id(), "client-live");
                    task_cancel_observed.store(true, Ordering::SeqCst);
                }
                OrderTaskCommand::Flush(waiter) => {
                    assert!(task_cancel_observed.load(Ordering::SeqCst));
                    task_reconcile_received.notified().await;
                    waiter.send(()).unwrap();
                }
                OrderTaskCommand::Submit { .. } => panic!("shutdown dispatched a submit"),
                OrderTaskCommand::Shutdown => return,
            }
        }
    });
    let reconcile_cancel_observed = Arc::clone(&cancel_observed);
    let task_reconcile_received = Arc::clone(&reconcile_received);
    let task_events = control_tx.clone();
    let reconcile_task = tokio::spawn(async move {
        while let Some(command) = reconcile_rx.recv().await {
            match command {
                ReconcileTaskCommand::Reconcile {
                    restored_orders: orders,
                    command_flush,
                } => {
                    task_reconcile_received.notify_one();
                    if let Some(command_flush) = command_flush {
                        command_flush.await.unwrap();
                    }
                    assert!(reconcile_cancel_observed.load(Ordering::SeqCst));
                    assert_eq!(orders.len(), 1);
                    task_events
                        .send(RuntimeEvent::RemoteState {
                            account_id: "main".to_string(),
                            remote_orders: vec![RemoteOrder {
                                exchange_order_id: "exchange-live".to_string(),
                                client_order_id: "client-live".to_string(),
                                symbol: "BTC-USDT".to_string(),
                                side: Side::Buy,
                                state: PrivateOrderState::Cancelled,
                                price: 100.0,
                                qty: 1.0,
                                cumulative_filled_qty: 0.0,
                                average_fill_price: 0.0,
                                update_time_ms: unix_time_ms(),
                            }],
                            remote_fills: Vec::new(),
                            remote_account: account_update(unix_time_ms()),
                            ts_ms: unix_time_ms(),
                        })
                        .await
                        .unwrap();
                }
                ReconcileTaskCommand::Shutdown => return,
            }
        }
    });
    control_tx
        .send(RuntimeEvent::Fatal(RuntimeTaskFailure::Gateway(
            "injected runtime failure".to_string(),
        )))
        .await
        .unwrap();
    let mut runtime = TestRuntimeParts {
        session_id: "test-shutdown-session".to_string(),
        session_started_at_ms: unix_time_ms(),
        config_source: None,
        config_fingerprint: "test-config".to_string(),
        evidence_config_fingerprint: "test-evidence-config".to_string(),
        executable_sha256: "a".repeat(64),
        host_identity_sha256: None,
        account_identity_sha256s: BTreeMap::new(),
        mode: LiveMode::Demo,
        run_duration: None,
        live_config: config.clone(),
        coordinator,
        processor: FeedProcessor::new(16, 16),
        storage: None,
        storage_sink,
        control_rx,
        feed_rx,
        forbidden_rx,
        order_senders: HashMap::from([("main".to_string(), order_tx)]),
        order_tasks: vec![order_task],
        reconcile_senders: HashMap::from([("main".to_string(), reconcile_tx)]),
        reconcile_tasks: vec![reconcile_task],
        order_ws_runtimes: Vec::new(),
        order_ws_status_tasks: Vec::new(),
        safety_senders: HashMap::new(),
        safety_tasks: Vec::new(),
        forbidden_tasks: Vec::new(),
        feeds: Vec::new(),
        feed_tasks: Vec::new(),
        sources: Vec::new(),
        public_feed_index: 0,
        reconcile_inflight: HashSet::new(),
        cancel_inflight: HashSet::new(),
        last_reconcile_attempt: HashMap::new(),
        fill_convergence: FillConvergenceGuard::default(),
        order_convergence: OrderStateConvergenceGuard::new(5_000),
        readiness_timeout_ms: 1_000,
        timer_interval_ms: 100,
        max_feed_age_ms: 60_000,
        shutdown_timeout_ms: 1_000,
        teardown_timeout_ms: 1_000,
        safety_latch_sync_timeout_ms: 1_000,
        evidence: RuntimeEvidence::default(),
        latency: LiveLatencyCollector::default(),
        shutdown_in_progress: false,
        shutdown_storage_error: None,
        preserve_deadman_on_shutdown: false,
        shutdown_reconciliation_requested: HashSet::new(),
        shutdown_reconciled_accounts: HashSet::new(),
        operator_service: None,
        operator_rx: None,
        operator_shutdown_reason: None,
        alert_runtime: None,
        alert_sink: None,
        alert_failures: None,
        alert_shutdown_timeout_ms: 100,
        alert_delivery_failure_is_fatal: true,
        observed_alert_delivery_failures: 0,
        alert_stats: AlertStats::default(),
        host_guard: None,
        host_failures: None,
        host_preflight: None,
        host_checks: 0,
        host_last_snapshot: None,
    }
    .into_runtime();

    let (_authority_gateway, cancel_policy, client_order_ids) =
        runtime_order_gateway(&["BTC-USDT"], Vec::new());
    assert_eq!(runtime.durable_latches.active_count(), 0);
    assert!(
        matches!(
            runtime
                .record_durable_storage(StorageRecord::SafetyLatch(latch(
                    SafetyLatchScope::Account {
                        account_id: "not-predeclared".to_string(),
                    },
                    SafetyLatchSource::Risk,
                )))
                .await,
            Err(LiveRuntimeError::Storage(StorageError::Closed))
        ),
        "unknown health scopes must still reach authoritative storage"
    );
    assert!(matches!(
        runtime
            .record_durable_storage(StorageRecord::SafetyLatch(latch(
                SafetyLatchScope::Global,
                SafetyLatchSource::Risk,
            )))
            .await,
        Err(LiveRuntimeError::Storage(StorageError::Closed))
    ));
    assert_eq!(
        runtime.durable_latches.active_count(),
        0,
        "a failed durable write must not advance the latch tracker"
    );
    let health =
        serde_json::to_value(runtime.health.periodic_snapshot(Default::default())).unwrap();
    assert_eq!(
        health["readiness"]["active_durable_safety_latches"], 0,
        "a failed durable write must not advance latch health"
    );
    // Keep the remainder of this lifecycle test focused on its original
    // fail-closed cleanup behavior after proving the durable failure boundary.
    runtime.shutdown.preserve_deadman = false;

    assert!(matches!(
        runtime.dispatch_action(LiveAction::Cancel(cancel_action(
            &cancel_policy,
            &client_order_ids,
            "BTC-USDT",
            "client-live",
            "injected pre-shutdown storage failure",
        ))),
        Err(LiveRuntimeError::Storage(StorageError::Closed))
    ));
    assert!(runtime.reconciliation.cancel_inflight.is_empty());
    let health =
        serde_json::to_value(runtime.health.periodic_snapshot(Default::default())).unwrap();
    assert_eq!(
        health["orders"]["cancel_requests"], 0,
        "a closed non-durable OrderRequest enqueue must not advance health"
    );

    runtime
        .health
        .set_connectivity_expected(ConnectivityId::Feed, true);
    runtime
        .health
        .set_connectivity_state(ConnectivityId::Feed, ConnectivityHealthState::Ready);
    runtime
        .health
        .set_connectivity_expected(ConnectivityId::Private, true);
    runtime
        .health
        .set_connectivity_state(ConnectivityId::Private, ConnectivityHealthState::Failed);
    runtime
        .health
        .set_connectivity_expected(ConnectivityId::OrderCommand, false);
    assert!(runtime.emit_final_health_snapshot());
    assert!(
        !runtime.emit_final_health_snapshot(),
        "one runtime may emit its final health snapshot only once"
    );
    let final_health =
        serde_json::to_value(runtime.health.final_snapshot(Default::default())).unwrap();
    assert_eq!(final_health["readiness"]["state"], "stopping");
    assert_eq!(
        final_health["connectivity"][0]["state"], "disconnected",
        "ready required connectivity finalizes as disconnected"
    );
    assert_eq!(
        final_health["connectivity"][1]["state"], "failed",
        "a terminal connectivity failure is preserved"
    );
    assert_eq!(
        final_health["connectivity"][2]["state"], "not_required",
        "optional connectivity remains explicitly not required"
    );
    // Restore the guard so this fixture can continue exercising the original
    // production close path, which must perform its own post-teardown emission.
    runtime.health_final_emitted = false;

    let error = runtime.run().await.unwrap_err();
    drop(control_tx);
    drop(feed_tx);
    let _ = std::fs::remove_file(path);

    let LiveRuntimeError::ReportedFailure { source, report } = error else {
        panic!("runtime failure must retain its post-cleanup evidence report");
    };
    assert_eq!(report.stop_reason, LiveStopReason::RuntimeFailure);
    assert!(!report.clean_soak);
    assert_eq!(report.active_orders_after_shutdown, 0);
    let failure = report.failure.as_ref().expect("failure evidence");
    assert_eq!(failure.code, "gateway_task");
    assert!(failure.message.contains("injected runtime failure"));

    let error = *source;
    let LiveRuntimeError::LifecycleFailure { primary, secondary } = error else {
        panic!("expected combined runtime and shutdown-storage failure");
    };
    assert!(matches!(
        *primary,
        LiveRuntimeError::GatewayTask(message) if message == "injected runtime failure"
    ));
    assert!(secondary.contains("fail-closed cleanup"));
    assert!(secondary.contains("storage remained unavailable"));
    assert!(cancel_observed.load(Ordering::SeqCst));
}
