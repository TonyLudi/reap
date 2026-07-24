use std::fs;
use std::path::PathBuf;

fn manifest_dir() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR"))
}

fn read(relative: &str) -> String {
    fs::read_to_string(manifest_dir().join(relative)).unwrap()
}

#[test]
fn private_fixture_slice_has_no_network_auth_scheduler_or_mutation_dependency() {
    let manifest = read("Cargo.toml");
    for forbidden in [
        "reqwest",
        "tokio",
        "tungstenite",
        "hyper",
        "hmac",
        "ring",
        "zeroize",
    ] {
        assert!(
            !manifest.contains(forbidden),
            "fixture-only adapter must not depend on {forbidden}"
        );
    }

    let sources = [
        read("src/private_fixture.rs"),
        read("src/reconcile_fixture.rs"),
        read("src/account_fixture.rs"),
        read("src/fixture_delivery.rs"),
        read("src/fixture_scope.rs"),
    ]
    .join("\n");
    for forbidden in [
        "connect_async",
        "TcpStream",
        "place_order",
        "cancel_order",
        "spawn(",
        "ApiSecret",
        "PrivateKey",
        "effective_allowance",
        "PmSnapshotCompleteness",
    ] {
        assert!(
            !sources.contains(forbidden),
            "private fixture seam must not contain {forbidden}"
        );
    }
}

#[test]
fn ownership_metadata_and_linkage_authority_do_not_escape_as_scalar_fallbacks() {
    let delivery = read("src/fixture_delivery.rs");
    assert!(!delivery.contains("pub const fn owner_id"));
    assert!(!delivery.contains("pub fn reduce_with"));
    assert!(!delivery.contains("pub const fn payload"));

    let scope = read("src/fixture_scope.rs");
    assert!(scope.contains("PmFixtureReadOwnerGrant"));
    assert!(scope.contains("PmGoalFTradingDomain::from_metadata"));
    assert!(!scope.contains("pub fn new(value: u64)"));

    let private = read("src/private_fixture.rs");
    assert!(!private.contains("trade.linkage()"));
    assert!(!private.contains("parse_fill_role"));
    assert!(private.contains("PmFillSettlementStatus::Failed"));

    let metadata = read("src/public_metadata.rs");
    assert!(!metadata.contains("PmAuthoritativeMetadata::verify_recorded"));
    assert!(metadata.contains("PmRecordedMetadataEvidence"));
    assert!(metadata.contains("PmGoalFTradingDomain::from_metadata"));
}

#[test]
fn fake_execution_is_fixed_bounded_and_in_process_only() {
    let fake = [
        read("src/fake_execution.rs"),
        read("src/fake_execution/command.rs"),
        read("src/fake_execution/outcome.rs"),
    ]
    .join("\n");

    for forbidden in [
        "async fn",
        ".await",
        "TcpStream",
        "connect_async",
        "reqwest",
        "Credentials",
        "PrivateKey",
        "ApiKey",
        "cancel_all",
        "PreparedPm",
        "ApprovedPm",
        "ReservedPm",
        "Deserialize",
    ] {
        assert!(
            !fake.contains(forbidden),
            "fake execution contains forbidden capability: {forbidden}"
        );
    }
    assert!(fake.contains("MAX_PM_FAKE_ACK_FILL_LEGS"));
    assert!(fake.contains("PmFakeOrderType::Gtc"));
    assert!(fake.contains("post_only"));
    assert!(fake.contains("defer_exec"));
    assert!(fake.contains("expiration"));
}
