use std::{fs, path::PathBuf};

fn crate_root() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR"))
}

fn source(name: &str) -> String {
    fs::read_to_string(crate_root().join("src").join(name)).expect("source file")
}

#[test]
fn crate_has_no_transport_auth_secret_or_mutation_dependency() {
    let manifest = fs::read_to_string(crate_root().join("Cargo.toml")).expect("manifest");
    for forbidden in [
        "reqwest",
        "tokio",
        "reap-pm-live",
        "reap-polymarket-live-adapter",
        "reap-polymarket-auth",
        "reap-polymarket-wire",
    ] {
        assert!(
            !manifest.contains(forbidden),
            "forbidden dependency: {forbidden}"
        );
    }
    assert!(!manifest.contains("[[bin]]"));

    let all_source = [
        source("lib.rs"),
        source("journal.rs"),
        source("recovery.rs"),
        source("schema.rs"),
        source("protected.rs"),
    ]
    .join("\n");
    for forbidden in [
        "RuntimeExactBodyCommitment",
        "SerializedPlaceRequest",
        "SerializedOwnedCancelRequest",
        "L2Credentials",
        "private_key",
        "credential_secret",
        "send_grant",
        "network_send_grant",
    ] {
        assert!(
            !all_source.contains(forbidden),
            "forbidden source surface: {forbidden}"
        );
    }
}

#[test]
fn recovery_owner_has_no_place_retry_or_place_ack_reconstruction_surface() {
    let journal = source("journal.rs");
    let recovery = journal
        .split_once("// Recovery methods are implemented")
        .expect("recovery section marker")
        .1;
    assert!(!recovery.contains("record_place_intent"));
    assert!(!recovery.contains("record_place_prepared"));
    assert!(!recovery.contains("record_place_dispatch_authorized"));
    assert!(!recovery.contains("PmDurablePlaceDispatchAckV1"));
    assert!(recovery.contains("record_recovery_cancel_intent"));
}

#[test]
fn durable_evidence_is_move_only_and_literal_false_false_zero() {
    let journal = source("journal.rs");
    let schema = source("schema.rs");
    let lib = source("lib.rs");
    for holder in [
        "PmDurablePlacePreparedAckV1",
        "PmDurablePlaceDispatchAckV1",
        "PmPreparedConsumedAuthorizationProofV1",
        "PmDurableCancelPreparedAckV1",
        "PmDurableCancelDispatchAckV1",
        "PmTrialLiveRecoveryProjectionV1",
    ] {
        assert!(journal.contains(holder) || source("recovery.rs").contains(holder));
        assert!(!journal.contains(&format!("impl Clone for {holder}")));
    }
    assert!(schema.contains("production_order_entry_authorized: bool"));
    assert!(schema.contains("real_order_submission_authorized: bool"));
    assert!(schema.contains("place_dispatch_allowance: u8"));
    assert!(journal.contains("production_order_entry_authorized: false"));
    assert!(journal.contains("real_order_submission_authorized: false"));
    assert!(journal.contains("place_dispatch_allowance: 0"));
    assert!(lib.contains("PRODUCTION_ORDER_ENTRY_AUTHORIZED: bool = false"));
    assert!(lib.contains("REAL_ORDER_SUBMISSION_AUTHORIZED: bool = false"));
    assert!(lib.contains("PLACE_DISPATCH_ALLOWANCE: u8 = 0"));
}

#[test]
fn only_journal_safe_semantic_commitments_can_be_persisted() {
    let schema = source("schema.rs");
    assert!(schema.contains("PlaceSemanticRequestCommitment"));
    assert!(schema.contains("OwnedCancelSemanticRequestCommitment"));
    assert!(schema.contains("identity.expected_order_id().bytes()"));
    assert!(schema.contains("identity.semantic_request_commitment().bytes()"));
    assert!(schema.contains("PREPARED_REQUEST_FINGERPRINT_DOMAIN"));
    assert!(!schema.contains("runtime_exact_body"));
    assert!(!schema.contains("body_commitment"));
}
