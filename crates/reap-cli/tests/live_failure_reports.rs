use std::process::Command;

use reap_live::{
    LIVE_RUN_REPORT_SCHEMA_VERSION, LiveConfig, LiveMode, LiveRunReport, LiveStopReason,
    verify_live_run_paths,
};

fn example_config() -> LiveConfig {
    LiveConfig::from_toml(include_str!("../../../examples/live-okx-demo.toml")).unwrap()
}

fn write_config(path: &std::path::Path, config: &LiveConfig) {
    std::fs::write(path, toml::to_string(config).unwrap()).unwrap();
}

#[test]
fn invalid_live_config_is_rejected_before_report_reservation() {
    let directory = tempfile::tempdir().unwrap();
    let config_path = directory.path().join("live.toml");
    let report_path = directory.path().join("run-report.json");
    let mut config = example_config();
    config.runtime.event_channel_capacity = 0;
    write_config(&config_path, &config);

    let output = Command::new(env!("CARGO_BIN_EXE_reap"))
        .args(["live", "--config"])
        .arg(&config_path)
        .arg("--output")
        .arg(&report_path)
        .args(["--mode", "observe", "--duration-secs", "1"])
        .output()
        .unwrap();

    assert!(!output.status.success());
    assert!(!report_path.exists());
    assert!(
        String::from_utf8_lossy(&output.stderr).contains("event_channel_capacity must be positive")
    );
}

#[test]
fn zero_live_duration_is_rejected_before_report_reservation() {
    let directory = tempfile::tempdir().unwrap();
    let config_path = directory.path().join("live.toml");
    let report_path = directory.path().join("run-report.json");
    write_config(&config_path, &example_config());

    let output = Command::new(env!("CARGO_BIN_EXE_reap"))
        .args(["live", "--config"])
        .arg(&config_path)
        .arg("--output")
        .arg(&report_path)
        .args(["--mode", "observe", "--duration-secs", "0"])
        .output()
        .unwrap();

    assert!(!output.status.success());
    assert!(!report_path.exists());
    assert!(
        String::from_utf8_lossy(&output.stderr)
            .contains("live run duration must be greater than zero")
    );
}

#[cfg(target_os = "linux")]
#[test]
fn host_preflight_failure_persists_verifiable_startup_report() {
    let directory = tempfile::tempdir().unwrap();
    let config_path = directory.path().join("live.toml");
    let report_path = directory.path().join("run-report.json");
    let mut config = example_config();
    config.storage.path = directory.path().join("live-events.jsonl");
    config.runtime.connection_attempt_pacer_path =
        Some(directory.path().join("connection-attempt.pacer"));
    config.operator.socket_path = directory.path().join("operator.sock");
    config.host_guard.enabled = true;
    config.host_guard.min_disk_available_bytes = u64::MAX;
    config.host_guard.min_memory_available_bytes = u64::MAX;
    config.host_guard.require_clock_synchronized = false;
    write_config(&config_path, &config);

    let output = Command::new(env!("CARGO_BIN_EXE_reap"))
        .args(["live", "--config"])
        .arg(&config_path)
        .arg("--output")
        .arg(&report_path)
        .args(["--mode", "observe", "--duration-secs", "1"])
        .output()
        .unwrap();

    assert!(!output.status.success());
    let report_bytes = std::fs::read(&report_path).unwrap();
    let report: LiveRunReport =
        serde_json::from_slice(&report_bytes).expect("failure report must be valid JSON");
    let stdout: serde_json::Value = serde_json::from_slice(&output.stdout)
        .expect("failure report must also be emitted to stdout");
    assert_eq!(
        stdout,
        serde_json::from_slice::<serde_json::Value>(&report_bytes).unwrap()
    );
    assert_eq!(report.schema_version, LIVE_RUN_REPORT_SCHEMA_VERSION);
    assert_eq!(report.mode, LiveMode::Observe);
    assert_eq!(report.stop_reason, LiveStopReason::RuntimeFailure);
    assert!(report.session_id.is_none());
    assert!(report.account_identity_sha256s.is_empty());
    assert!(report.host_identity_sha256.is_some());
    assert!(report.host_preflight.is_none());
    assert_eq!(report.host_checks, 0);
    assert!(report.host_last_snapshot.is_none());
    assert_eq!(report.active_orders_after_shutdown, 0);
    assert!(!report.clean_soak);
    let failure = report.failure.as_ref().unwrap();
    assert_eq!(failure.code, "host_guard");
    assert!(failure.message.contains("not exchange-zero proof"));
    assert!(String::from_utf8_lossy(&output.stderr).contains("host health threshold breached"));

    let verification =
        verify_live_run_paths(&config_path, &report_path, Some(LiveMode::Observe)).unwrap();
    assert!(verification.evidence_valid, "{verification:#?}");
    assert!(!verification.acceptance_passed);

    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;

        assert_eq!(
            std::fs::metadata(&report_path)
                .unwrap()
                .permissions()
                .mode()
                & 0o777,
            0o600
        );
    }
}
