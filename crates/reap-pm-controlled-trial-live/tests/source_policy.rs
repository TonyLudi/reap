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
        source("live_dispatch.rs"),
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
fn positive_phase_a_authority_requires_a_separate_durable_barrier() {
    let journal = source("journal.rs");
    let live_dispatch = source("live_dispatch.rs");
    let recovery = source("recovery.rs");
    let schema = source("schema.rs");
    let lib = source("lib.rs");

    // The legacy A3 record and its public evidence API remain literal
    // false/false/0. The positive profile is a separately versioned file.
    assert!(journal.contains("production_order_entry_authorized: false"));
    assert!(journal.contains("real_order_submission_authorized: false"));
    assert!(journal.contains("place_dispatch_allowance: 0"));
    assert!(live_dispatch.contains("PM_PHASE_A_LIVE_DISPATCH_BARRIER_FILE_V1"));
    assert!(live_dispatch.contains("ProtectedJournal::create_new"));
    assert!(live_dispatch.contains("barrier_file.append_durable(&[], &encoded)"));
    assert!(live_dispatch.contains("production_order_entry_authorized: true"));
    assert!(live_dispatch.contains("real_order_submission_authorized: true"));
    assert!(live_dispatch.contains("place_dispatch_allowance: 1"));
    assert!(journal.contains("record_phase_a_place_live_dispatch_authorized"));
    assert!(journal.contains("prepare_phase_a_place_dispatch_grant"));

    {
        let authority = "PmPhaseAPlaceLiveDispatchProfileV1";
        let declaration = format!("pub struct {authority}");
        let declaration_offset = live_dispatch
            .find(&declaration)
            .expect("authority declaration");
        let declaration_prefix = &live_dispatch[..declaration_offset];
        let attached_attributes_and_docs = declaration_prefix
            .rsplit_once("\n\n")
            .map_or(declaration_prefix, |(_, tail)| tail);
        assert!(!attached_attributes_and_docs.contains("#[derive"));
        assert!(!live_dispatch.contains(&format!("impl Clone for {authority}")));
        assert!(!live_dispatch.contains(&format!("impl Serialize for {authority}")));
        assert!(!live_dispatch.contains(&format!("impl Deserialize for {authority}")));
    }
    for raw in [
        "PmPhaseAPlaceDispatchGrantV1",
        "PmRevalidatedPhaseAPlaceDispatchGrantV1",
        "PmPhaseAPlaceDispatchBarrierWitnessV1",
    ] {
        assert!(live_dispatch.contains(&format!("pub(crate) struct {raw}")));
        assert!(!lib.contains(&format!(", {raw}")));
    }
    for combined in [
        "PmPhaseAPlaceLiveDispatchOwnerV1",
        "PmRevalidatedPhaseAPlaceLiveDispatchOwnerV1",
        "PmPhaseAPlaceNetworkDispatchOwnerV1",
        "PmPhaseAPlaceMayHaveBeenDispatchedV1",
        "PmPhaseAPlaceDefinitelyNotDispatchedV1",
    ] {
        assert!(journal.contains(&format!("pub struct {combined}")));
        assert!(!journal.contains(&format!("impl Clone for {combined}")));
        assert!(!journal.contains(&format!("impl Serialize for {combined}")));
        assert!(!journal.contains(&format!("impl Deserialize for {combined}")));
    }
    assert!(!journal.contains("fn into_parts"));
    assert!(
        journal.contains(") -> Result<PmPhaseAPlaceLiveDispatchOwnerV1, PmTrialLiveJournalError>")
    );
    assert!(journal.contains("journals: &'journal mut PmControlledTrialLiveJournals"));
    assert!(journal.contains("validate_phase_a_live_durable_set"));
    assert!(journal.contains("revalidate_held_consumption_evidence"));
    assert!(journal.contains("validate_exact_bytes(&self.intent.bytes)"));
    assert!(journal.contains("validate_exact_bytes(&self.dispatch.bytes)"));
    assert!(!live_dispatch.contains("pub fn send"));
    assert!(!live_dispatch.contains("request_body:"));
    assert!(!live_dispatch.contains("credential_secret"));
    assert!(schema.contains("DefinitelyNotDispatched"));
    assert!(journal.contains("record_phase_a_place_definitely_not_dispatched"));
    assert!(live_dispatch.contains("revalidate_for_runner"));
    assert!(live_dispatch.contains("validate_exact_bytes"));
    assert!(recovery.contains("an absent\n        // positive barrier cannot distinguish"));
    assert!(
        recovery
            .contains("return PmTrialLiveRecoveryClassificationV1::PlaceMayHaveBeenSentNoResend;")
    );
    assert!(
        recovery.contains("if facts.place_dispatch.is_some()\n        && facts.dispatch_terminal")
    );
}

#[test]
fn positive_result_and_cancel_custody_cannot_collapse_into_legacy_v1_types() {
    let journal = source("journal.rs");
    let lib = source("lib.rs");
    for positive in [
        "PmDurablePhaseAPlaceLiveResultAckV1",
        "PmPhaseAPlaceLiveOutcomeBridgeAckV1",
        "PmPhaseAPlaceLiveOwnedVenueOrderV1",
        "PmDurablePhaseALiveCancelIntentAckV1",
        "PmDurablePhaseALiveCancelPreparedAckV1",
        "PmDurablePhaseALiveCancelDispatchAckV1",
    ] {
        assert!(journal.contains(&format!("pub struct {positive}")));
        assert!(lib.contains(positive));
        assert!(!journal.contains(&format!("impl Clone for {positive}")));
    }
    assert!(
        journal
            .contains(") -> Result<PmDurablePhaseALiveCancelIntentAckV1, PmTrialLiveJournalError>")
    );
    assert!(journal.contains("intent: PmDurablePhaseALiveCancelIntentAckV1"));
    assert!(journal.contains("prepared: PmDurablePhaseALiveCancelPreparedAckV1"));
    assert!(journal.contains("owned: PmPhaseAPlaceLiveOwnedVenueOrderV1"));
    assert!(!journal.contains(
        "record_phase_a_live_primary_cancel_intent(\n        &mut self,\n        place_bridge: PmPhaseAPlaceLiveOutcomeBridgeAckV1,\n        owned: PmJournalOwnedVenueOrderV1"
    ));
    assert!(journal.contains("phase_a_live_dispatch_epoch.is_some()"));
    assert!(journal.contains("phase_a_live_terminal_is_safe"));
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
