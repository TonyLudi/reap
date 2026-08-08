#[test]
fn auth_crate_has_no_network_runtime_strategy_or_generic_sdk_dependency() {
    let manifest = include_str!("../Cargo.toml");
    for forbidden in [
        "alloy",
        "anyhow",
        "async-trait",
        "ethers",
        "futures",
        "hyper",
        "reqwest",
        "reap-pm-live",
        "reap-pm-state",
        "reap-pm-strategy",
        "tokio",
        "tungstenite",
        "url =",
        "uuid",
    ] {
        assert!(
            !manifest.contains(forbidden),
            "forbidden auth dependency token: {forbidden}"
        );
    }
}
