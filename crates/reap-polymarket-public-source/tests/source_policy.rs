const MANIFEST: &str = include_str!("../Cargo.toml");
const LIB: &str = include_str!("../src/lib.rs");
const CONFIG: &str = include_str!("../src/config.rs");
const DECIMAL: &str = include_str!("../src/decimal.rs");
const POSITION: &str = include_str!("../src/position.rs");
const SOURCE: &str = include_str!("../src/source.rs");

#[test]
fn dependency_surface_is_public_read_only_and_credential_free() {
    for forbidden in [
        "reap-polymarket-auth",
        "reap-polymarket-live-adapter",
        "reap-pm-live",
        "hmac",
        "k256",
        "zeroize",
    ] {
        assert!(
            !MANIFEST.contains(forbidden),
            "forbidden dependency: {forbidden}"
        );
    }
    for required in [
        "reap-pm-core.workspace = true",
        "reqwest.workspace = true",
        "serde.workspace = true",
        "serde_json.workspace = true",
    ] {
        assert!(
            MANIFEST.contains(required),
            "missing dependency: {required}"
        );
    }
}

#[test]
fn production_origin_route_query_and_transport_policy_are_closed() {
    for required in [
        "https://data-api.polymarket.com",
        "url.set_path(\"/positions\")",
        ".append_pair(\"user\"",
        ".append_pair(\"market\"",
        ".append_pair(\"sizeThreshold\", \"0\")",
        ".append_pair(\"limit\", \"500\")",
        ".append_pair(\"sortBy\", \"TOKENS\")",
        ".append_pair(\"sortDirection\", \"DESC\")",
        ".redirect(Policy::none())",
        ".retry(reqwest::retry::never())",
        ".no_proxy()",
        ".header(ACCEPT, \"application/json\")",
        ".header(ACCEPT_ENCODING, \"identity\")",
        "MAX_POSITION_PAGE_BODY_BYTES",
        "FullPageAtOffsetCap",
    ] {
        assert!(
            CONFIG.contains(required) || SOURCE.contains(required),
            "missing closed source property: {required}"
        );
    }
    assert_eq!(SOURCE.matches(".get(url)").count(), 1);
    for forbidden in [
        ".post(",
        ".delete(",
        ".put(",
        ".patch(",
        "pub fn request(",
        "pub fn client(",
        "pub fn origin(",
        "pub fn route(",
        "pub fn query(",
        "/order",
        "POLY_",
    ] {
        assert!(
            !SOURCE.contains(forbidden),
            "capability escape: {forbidden}"
        );
    }
}

#[test]
fn position_numbers_never_cross_binary_floating_point() {
    for source in [DECIMAL, POSITION, SOURCE] {
        for forbidden in ["f32", "f64", "as_f64", "from_f64"] {
            assert!(
                !source.contains(forbidden),
                "floating-point escape: {forbidden}"
            );
        }
    }
    for required in [
        "Box<RawValue>",
        "PmExactPositionDecimal",
        "coefficient: U256",
        "decimal_exponent: i16",
        "PmTokenId::new(units)",
        "if asset == opposite_asset",
        "if outcome_index > 1",
        "outcome: Box<str>",
        "opposite_outcome: Box<str>",
    ] {
        assert!(
            DECIMAL.contains(required) || POSITION.contains(required),
            "missing exact numeric boundary: {required}"
        );
    }
}

#[test]
fn observation_language_and_authority_remain_narrow() {
    assert!(LIB.contains("PRODUCTION_ORDER_ENTRY_AUTHORIZED: bool = false"));
    assert!(!LIB.contains("PRODUCTION_ORDER_ENTRY_AUTHORIZED: bool = true"));
    for required in [
        "PmMonitoredPositionObservation",
        "PmConfiguredTokenPosition",
        "Absent",
        "Present(Box<PmDataApiPositionEvidence>)",
        "neither funder-wide inventory",
        "nor authority to sell",
    ] {
        assert!(
            POSITION.contains(required),
            "missing limitation: {required}"
        );
    }
    for forbidden in [
        "SellAuthorized",
        "AtomicComplete",
        "atomic_complete: true",
        "sell_authorized: true",
        "production_order_entry_authorized: true",
    ] {
        assert!(
            !POSITION.contains(forbidden),
            "false authority: {forbidden}"
        );
    }
}

#[test]
fn arbitrary_origin_is_compile_excluded_outside_unit_tests() {
    assert!(CONFIG.contains("#[cfg(test)]\n    pub(crate) fn numeric_loopback_evidence("));
    assert!(SOURCE.contains("#[cfg(test)]\n    fn numeric_loopback_evidence("));
    assert!(CONFIG.contains("host.parse::<IpAddr>()"));
    assert!(CONFIG.contains("address.is_loopback()"));
    assert!(!MANIFEST.contains("loopback-evidence"));
}
