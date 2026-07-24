const MANIFEST: &str = include_str!("../Cargo.toml");
const LIB: &str = include_str!("../src/lib.rs");
const MODEL: &str = include_str!("../src/model.rs");
const QUOTE_POLICY: &str = include_str!("../src/quote_policy.rs");

#[test]
fn strategy_slice_stays_pure_and_capability_narrow() {
    for forbidden_dependency in [
        "reap-polymarket-wire",
        "reap-polymarket-adapter",
        "reap-pm-live",
        "reap-pm-state",
        "serde",
        "tokio",
        "reqwest",
    ] {
        assert!(
            !MANIFEST.contains(forbidden_dependency),
            "forbidden strategy dependency: {forbidden_dependency}"
        );
    }

    let production = [LIB, MODEL, QUOTE_POLICY].join("\n");
    for forbidden in [
        "Deserialize",
        "Serialize",
        "async fn",
        ".await",
        "TcpStream",
        "Prepared",
        "Reserved",
        "Approved",
        "cancel_all",
    ] {
        assert!(
            !production.contains(forbidden),
            "quote policy contains forbidden capability: {forbidden}"
        );
    }
}

#[test]
fn floating_point_is_confined_to_model_input_conversion() {
    assert_eq!(QUOTE_POLICY.matches("f64").count(), 7);
    assert!(!QUOTE_POLICY.contains("to_string"));
    assert!(!QUOTE_POLICY.contains("parse::<f64>"));
}
