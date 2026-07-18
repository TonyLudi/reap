use super::*;

struct TaskDropSignal(Option<oneshot::Sender<()>>);

impl Drop for TaskDropSignal {
    fn drop(&mut self) {
        if let Some(signal) = self.0.take() {
            let _ = signal.send(());
        }
    }
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
