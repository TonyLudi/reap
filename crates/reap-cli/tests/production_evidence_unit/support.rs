pub(super) fn manifest_toml(extra: &str) -> String {
    format!(
        r#"
schema_version = 8
expected_reap_version = "0.1.0"
expected_live_executable_sha256 = "{}"
expected_host_identity_sha256 = "{}"
expected_approval_policy_sha256 = "{}"
expected_deployment_candidate_id = "candidate-a"
demo_config = "demo.toml"
production_config = "production.toml"
fault_demo_config = "fault-demo.toml"
fault_proxy_config = "fault-proxy.toml"
demo_soak_report = "soak.json"
fault_matrix_manifest = "faults.toml"
latency_calibration_artifact = "latency.json"
latency_source_reports = ["latency-source.json"]
research_manifest = "research.toml"
research_report = "research.json"
account_certifications = ["account.json"]
emergency_cancel_report = "emergency.json"

[[fault_proxy_runs]]
scenario = "clean_observe"
report = "proxy-clean-observe.json"
[[fault_proxy_runs]]
scenario = "clean_demo"
report = "proxy-clean-demo.json"
[[fault_proxy_runs]]
scenario = "public_reconnect"
report = "proxy-public-reconnect.json"
[[fault_proxy_runs]]
scenario = "private_reconnect"
report = "proxy-private-reconnect.json"
[[fault_proxy_runs]]
scenario = "order_transport_reconnect"
report = "proxy-order-transport-reconnect.json"
[[fault_proxy_runs]]
scenario = "ambiguous_submit"
report = "proxy-ambiguous-submit.json"
[[fault_proxy_runs]]
scenario = "ambiguous_cancel"
report = "proxy-ambiguous-cancel.json"
[[fault_proxy_runs]]
scenario = "partial_fill"
report = "proxy-partial-fill.json"
[[fault_proxy_runs]]
scenario = "fill_convergence_timeout"
report = "proxy-fill-convergence-timeout.json"
[[fault_proxy_runs]]
scenario = "order_convergence_timeout"
report = "proxy-order-convergence-timeout.json"
[[fault_proxy_runs]]
scenario = "restored_safety_latch"
report = "proxy-restored-safety-latch.json"
[[fault_proxy_runs]]
scenario = "deadman_heartbeat_failure"
report = "proxy-deadman-heartbeat-failure.json"
[[fault_proxy_runs]]
scenario = "exchange_clock_failure"
report = "proxy-exchange-clock-failure.json"
[[fault_proxy_runs]]
scenario = "exchange_status_failure"
report = "proxy-exchange-status-failure.json"
[[fault_proxy_runs]]
scenario = "exchange_instrument_failure"
report = "proxy-exchange-instrument-failure.json"
[[fault_proxy_runs]]
scenario = "exchange_fee_failure"
report = "proxy-exchange-fee-failure.json"
[[fault_proxy_runs]]
scenario = "account_config_failure"
report = "proxy-account-config-failure.json"

[freshness]
future_tolerance_ms = 60000
demo_soak_max_age_ms = 3600000
fault_run_max_age_ms = 3600000
latency_source_max_age_ms = 3600000
production_account_certification_max_age_ms = 600000
deadman_certification_max_age_ms = 3600000
emergency_cancel_max_age_ms = 3600000
fill_collection_max_age_ms = 3600000
bill_collection_max_age_ms = 3600000

[expected_demo_account_identity_sha256s]
main = "{}"

[expected_production_account_identity_sha256s]
main = "{}"

[[deadman_certifications]]
artifact = "deadman.json"
journal = "journal.jsonl"

[[fill_reconciliations]]
collection_manifest = "fills/manifest.json"
journal = "journal.jsonl"
minimum_fills = 1

[[economic_reconciliations]]
fill_collection_manifest = "fills/manifest.json"
bill_collection_manifest = "bills/manifest.json"
opening_account_certification = "opening-account.json"
closing_account_certification = "closing-account.json"
journal = "journal.jsonl"
minimum_trade_bills = 1
minimum_derivative_close_bills = 1
minimum_funding_bills = 1
maximum_trade_bill_delay_ms = 60000
maximum_funding_bill_delay_ms = 60000
maximum_funding_mark_bracket_distance_ms = 1000
maximum_account_boundary_gap_ms = 60000
{extra}
"#,
        "1".repeat(64),
        "2".repeat(64),
        "3".repeat(64),
        "4".repeat(64),
        "5".repeat(64),
    )
}
