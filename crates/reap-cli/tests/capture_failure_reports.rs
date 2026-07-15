use std::process::Command;

use reap_capture::{CaptureConfig, CaptureRunReport, CaptureStopReason};

const EMPTY_SHA256: &str = "e3b0c44298fc1c149afbf4c8996fb92427ae41e4649b934ca495991b7852b855";

fn example_config() -> CaptureConfig {
    CaptureConfig::from_toml(include_str!("../../../examples/capture-okx-public.toml")).unwrap()
}

#[test]
fn invalid_capture_config_is_rejected_before_report_reservation() {
    let directory = tempfile::tempdir().unwrap();
    let config_path = directory.path().join("capture.toml");
    let report_path = directory.path().join("run-report.json");
    let raw_path = directory.path().join("raw.jsonl");
    let mut config = example_config();
    config.runtime.writer_channel_capacity = 0;
    config.output.raw_path = raw_path.clone();
    std::fs::write(&config_path, toml::to_string(&config).unwrap()).unwrap();

    let output = Command::new(env!("CARGO_BIN_EXE_reap"))
        .args(["capture", "--config"])
        .arg(&config_path)
        .arg("--output")
        .arg(&report_path)
        .args(["--duration-secs", "1"])
        .output()
        .unwrap();

    assert!(!output.status.success());
    assert!(!report_path.exists());
    assert!(!raw_path.exists());
    assert!(
        String::from_utf8_lossy(&output.stderr)
            .contains("writer_channel_capacity must be positive")
    );
}

#[test]
fn zero_capture_duration_is_rejected_before_report_reservation() {
    let directory = tempfile::tempdir().unwrap();
    let config_path = directory.path().join("capture.toml");
    let report_path = directory.path().join("run-report.json");
    let raw_path = directory.path().join("raw.jsonl");
    let mut config = example_config();
    config.output.raw_path = raw_path.clone();
    std::fs::write(&config_path, toml::to_string(&config).unwrap()).unwrap();

    let output = Command::new(env!("CARGO_BIN_EXE_reap"))
        .args(["capture", "--config"])
        .arg(&config_path)
        .arg("--output")
        .arg(&report_path)
        .args(["--duration-secs", "0"])
        .output()
        .unwrap();

    assert!(!output.status.success());
    assert!(!report_path.exists());
    assert!(!raw_path.exists());
    assert!(String::from_utf8_lossy(&output.stderr).contains("duration must be positive"));
}

#[cfg(target_os = "linux")]
#[test]
fn host_preflight_failure_persists_typed_report_without_adopting_old_raw_data() {
    let directory = tempfile::tempdir().unwrap();
    let config_path = directory.path().join("capture.toml");
    let report_path = directory.path().join("run-report.json");
    let raw_path = directory.path().join("raw.jsonl");
    let prior_raw = b"prior process data\n";
    std::fs::write(&raw_path, prior_raw).unwrap();
    let mut config = example_config();
    config.output.raw_path = raw_path.clone();
    config.host_guard.min_disk_available_bytes = u64::MAX;
    config.host_guard.min_memory_available_bytes = u64::MAX;
    config.host_guard.require_clock_synchronized = false;
    std::fs::write(&config_path, toml::to_string(&config).unwrap()).unwrap();

    let output = Command::new(env!("CARGO_BIN_EXE_reap"))
        .args(["capture", "--config"])
        .arg(&config_path)
        .arg("--output")
        .arg(&report_path)
        .args(["--duration-secs", "1"])
        .output()
        .unwrap();

    assert!(!output.status.success());
    let report: CaptureRunReport = serde_json::from_slice(&std::fs::read(&report_path).unwrap())
        .expect("failure report must be valid JSON");
    let stdout_report: CaptureRunReport = serde_json::from_slice(&output.stdout)
        .expect("failure report must also be emitted to stdout");
    assert_eq!(stdout_report, report);
    assert_eq!(report.stop_reason, CaptureStopReason::RuntimeFailure);
    assert!(!report.clean_capture);
    assert_eq!(report.raw_records, 0);
    assert_eq!(report.raw_bytes, 0);
    assert_eq!(report.raw_sha256, EMPTY_SHA256);
    assert_eq!(report.failure.as_ref().unwrap().code, "host_guard");
    assert!(!report.failure.as_ref().unwrap().message.is_empty());
    assert_eq!(std::fs::read(&raw_path).unwrap(), prior_raw);
    assert!(String::from_utf8_lossy(&output.stderr).contains("host health threshold breached"));

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
