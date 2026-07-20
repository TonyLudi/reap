use super::*;

#[test]
fn durable_latch_health_is_seeded_idempotent_and_changes_only_when_applied() {
    let config = config();
    let record = |scope, active| {
        StorageRecord::SafetyLatch(SafetyLatchRecord {
            ts_ms: 1,
            scope,
            active,
            source: SafetyLatchSource::Operator,
            request_id: None,
            reason: "health latch fixture".to_string(),
        })
    };
    let global = record(SafetyLatchScope::Global, true);
    let account = record(
        SafetyLatchScope::Account {
            account_id: "main".to_string(),
        },
        true,
    );
    let symbol = record(
        SafetyLatchScope::Symbol {
            symbol: "BTC-USDT".to_string(),
        },
        true,
    );
    let mut recovered = RecoveredStorage::default();
    let StorageRecord::SafetyLatch(global_record) = global.clone() else {
        unreachable!()
    };
    recovered.global_safety_latch = Some(global_record);
    let StorageRecord::SafetyLatch(account_record) = account.clone() else {
        unreachable!()
    };
    recovered
        .account_safety_latches
        .insert("main".to_string(), account_record);
    let StorageRecord::SafetyLatch(symbol_record) = symbol.clone() else {
        unreachable!()
    };
    recovered
        .symbol_safety_latches
        .insert("BTC-USDT".to_string(), symbol_record);

    let mut tracker = DurableLatchTracker::from_recovered(&config, &recovered);
    assert_eq!(tracker.active_count(), 3);

    let duplicate = tracker.resolve(&global).unwrap();
    assert_eq!(tracker.active_count(), 3, "resolution is observation-only");
    assert_eq!(
        tracker.apply(duplicate),
        3,
        "duplicate activation is idempotent"
    );

    let resume = record(
        SafetyLatchScope::Symbol {
            symbol: "BTC-USDT".to_string(),
        },
        false,
    );
    let resume = tracker.resolve(&resume).unwrap();
    assert_eq!(
        tracker.active_count(),
        3,
        "pre-durability count is unchanged"
    );
    assert_eq!(tracker.apply(resume), 2);
    assert_eq!(tracker.apply(resume), 2, "duplicate resume is idempotent");

    let unknown = record(
        SafetyLatchScope::Account {
            account_id: "not-predeclared".to_string(),
        },
        true,
    );
    assert!(
        tracker.resolve(&unknown).is_none(),
        "unknown scopes are durable-storage concerns, not health gates"
    );
    assert_eq!(
        tracker.active_count(),
        2,
        "unknown scopes do not mutate bounded health state"
    );
}

struct TaskDropSignal(Option<oneshot::Sender<()>>);

impl Drop for TaskDropSignal {
    fn drop(&mut self) {
        if let Some(signal) = self.0.take() {
            let _ = signal.send(());
        }
    }
}

#[test]
fn staged_startup_preserves_ordered_construction_and_infallible_task_transfer() {
    let runtime_source = include_str!("../../src/runtime.rs");
    let startup_source = include_str!("../../src/runtime/startup.rs");
    let build = runtime_source
        .split_once("    async fn build(")
        .expect("LiveRuntime::build")
        .1
        .split_once("    async fn run_loop(")
        .expect("run_loop follows build")
        .0;
    let mut previous = 0;
    for marker in [
        "StartupPlan::resolve(",
        "StartupRecovery::open(",
        "AuthenticatedStartup::bootstrap(",
        "CoordinatorStartup::restore(",
        "RuntimeResources::start(",
        ".into_runtime()",
        "finish_startup(",
    ] {
        let position = build
            .find(marker)
            .unwrap_or_else(|| panic!("build is missing ordered stage marker {marker}"));
        assert!(
            position >= previous,
            "startup stage marker {marker} moved out of order"
        );
        previous = position;
    }

    let resource_start = startup_source
        .split_once("    pub(super) async fn start(")
        .expect("RuntimeResources::start")
        .1
        .split_once("    pub(super) fn into_runtime(")
        .expect("into_runtime follows resource start")
        .0;
    let mut previous = 0;
    for marker in [
        "for (seed_index, seed) in seeds.into_iter().enumerate()",
        ".cancel_all_after(timeout_secs)",
        "run_account_safety_task(",
        "run_forbidden_order_sentinel(",
        ".bootstrap_factory(",
        "let (gateway, order_ws_runtime, mut order_ws_status)",
        "order_ws_status_tasks.push(",
        "order_ws_runtimes.push(",
        "run_order_task(",
        "run_reconcile_task(",
    ] {
        let position = resource_start
            .find(marker)
            .unwrap_or_else(|| panic!("resource startup is missing ordered marker {marker}"));
        assert!(
            position >= previous,
            "per-account resource marker {marker} moved out of order"
        );
        previous = position;
    }

    let transfer = startup_source
        .split_once("    pub(super) fn into_runtime(")
        .expect("RuntimeResources::into_runtime")
        .1
        .split_once("pub(super) async fn finish_startup(")
        .expect("finish_startup follows transfer")
        .0;
    assert_eq!(
        transfer.matches(".take()").count(),
        6,
        "exactly six guarded startup task groups must transfer"
    );
    let mut previous = 0;
    for marker in [
        "order_ws_status_tasks.take()",
        "feed_tasks.take()",
        "order_tasks.take()",
        "safety_tasks.take()",
        "forbidden_tasks.take()",
        "reconcile_tasks.take()",
    ] {
        let position = transfer
            .find(marker)
            .unwrap_or_else(|| panic!("task transfer is missing {marker}"));
        assert!(
            position >= previous,
            "guarded task transfer {marker} moved out of order"
        );
        previous = position;
    }
    for forbidden in [
        ".await",
        "?",
        "expect(",
        "unwrap(",
        "tokio::spawn(",
        "mpsc::channel(",
        "return ",
        "Result<",
    ] {
        assert!(
            !transfer.contains(forbidden),
            "infallible task transfer contains forbidden token {forbidden}"
        );
    }
    assert!(
        !startup_source.contains("ProvenRegularSubmitRequest"),
        "startup must not acquire or name recovered submit-proof authority"
    );
}

#[tokio::test]
async fn bounded_run_rejects_zero_duration_before_credentials() {
    let error = run_live(
        config(),
        LiveRunOptions {
            mode: LiveMode::Observe,
            demo_confirmed: false,
            run_duration: Some(Duration::ZERO),
        },
    )
    .await
    .unwrap_err();

    assert!(matches!(error, LiveRuntimeError::InvalidRunDuration));
}

#[tokio::test]
async fn startup_task_group_aborts_owned_tasks_on_early_exit() {
    let (started_tx, started_rx) = oneshot::channel();
    let (dropped_tx, dropped_rx) = oneshot::channel();
    let mut tasks = StartupTaskGroup::default();
    tasks.push(tokio::spawn(async move {
        let _drop_signal = TaskDropSignal(Some(dropped_tx));
        let _ = started_tx.send(());
        std::future::pending::<()>().await;
    }));
    started_rx.await.unwrap();

    drop(tasks);

    tokio::time::timeout(Duration::from_secs(1), dropped_rx)
        .await
        .expect("startup task was not aborted")
        .unwrap();
}

#[tokio::test]
async fn journal_ownership_is_checked_before_credentials_or_network() {
    let path = std::env::temp_dir().join(format!(
        "reap-live-owner-{}-{}.jsonl",
        std::process::id(),
        unix_time_ns()
    ));
    let lease = acquire_storage_lease(&path).unwrap();
    let lock_path = lease.lock_path().to_path_buf();
    let mut config = config();
    config.storage.path = path;

    let error = run_live(
        config,
        LiveRunOptions {
            mode: LiveMode::Observe,
            demo_confirmed: false,
            run_duration: Some(Duration::from_millis(1)),
        },
    )
    .await
    .unwrap_err();

    let (source, _) = unwrap_startup_failure(error);
    assert!(matches!(
        source,
        LiveRuntimeError::Storage(StorageError::AlreadyLocked { .. })
    ));
    drop(lease);
    let _ = std::fs::remove_file(lock_path);
}

#[cfg(target_os = "linux")]
#[tokio::test]
async fn host_preflight_fails_before_credentials_or_network_and_releases_lease() {
    let path = std::env::temp_dir().join(format!(
        "reap-live-host-preflight-{}-{}.jsonl",
        std::process::id(),
        unix_time_ns()
    ));
    let mut config = config();
    config.storage.path = path.clone();
    config.host_guard.enabled = true;
    config.host_guard.min_disk_available_bytes = u64::MAX;
    config.host_guard.min_memory_available_bytes = 1;
    config.host_guard.require_clock_synchronized = false;

    let error = run_live(
        config,
        LiveRunOptions {
            mode: LiveMode::Observe,
            demo_confirmed: false,
            run_duration: Some(Duration::from_millis(1)),
        },
    )
    .await
    .unwrap_err();

    let (source, _) = unwrap_startup_failure(error);
    assert!(matches!(
        source,
        LiveRuntimeError::Host(HostHealthError::Unhealthy { ref code, .. })
            if code == "disk_low"
    ));
    let lease = acquire_storage_lease(&path).unwrap();
    let lock_path = lease.lock_path().to_path_buf();
    drop(lease);
    let _ = std::fs::remove_file(lock_path);
}

#[tokio::test]
async fn connection_pacer_preflight_fails_before_credentials_or_network_and_releases_lease() {
    let directory = tempfile::tempdir().unwrap();
    let path = directory.path().join("live.jsonl");
    let mut config = config();
    config.storage.path = path.clone();
    config.runtime.connection_attempt_pacer_path =
        Some(directory.path().join("missing").join("connect.pacer"));

    let error = run_live(
        config,
        LiveRunOptions {
            mode: LiveMode::Observe,
            demo_confirmed: false,
            run_duration: Some(Duration::from_millis(1)),
        },
    )
    .await
    .unwrap_err();

    let (source, _) = unwrap_startup_failure(error);
    assert!(matches!(source, LiveRuntimeError::ConnectionPacer(_)));
    let lease = acquire_storage_lease(&path).unwrap();
    let lock_path = lease.lock_path().to_path_buf();
    drop(lease);
    let _ = std::fs::remove_file(lock_path);
}

#[tokio::test]
async fn demo_mode_requires_confirmation_and_simulated_environment() {
    let error = run_live(
        config(),
        LiveRunOptions {
            mode: LiveMode::Demo,
            demo_confirmed: false,
            run_duration: None,
        },
    )
    .await
    .unwrap_err();
    assert!(matches!(error, LiveRuntimeError::DemoConfirmationRequired));

    let mut production = config();
    production.venue.environment = TradingEnvironment::Production;
    production.venue.public_ws_url = "wss://ws.okx.com:8443/ws/v5/public".to_string();
    production.venue.private_ws_url = "wss://ws.okx.com:8443/ws/v5/private".to_string();
    production.risk.stablecoin_guards = vec![StablecoinGuardConfig {
        symbol: "USDT-USD".to_string(),
        max_downside_deviation: 0.01,
    }];
    production.accounts[0].api_key_policy.require_ip_binding = true;
    let error = run_live(
        production,
        LiveRunOptions {
            mode: LiveMode::Demo,
            demo_confirmed: true,
            run_duration: None,
        },
    )
    .await
    .unwrap_err();
    assert!(matches!(
        error,
        LiveRuntimeError::DemoRequiresSimulatedTrading
    ));
}
