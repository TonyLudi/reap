use std::{fs, path::PathBuf};

fn crate_root() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR"))
}

fn source(name: &str) -> String {
    fs::read_to_string(crate_root().join("src").join(name)).expect("source file")
}

fn compact_whitespace(source: &str) -> String {
    source.split_whitespace().collect::<Vec<_>>().join(" ")
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
        source("online_preflight_v2.rs"),
        source("phase_a_attempt_lineage_v4.rs"),
        source("recovery.rs"),
        source("recovery_continuation.rs"),
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
fn phase_a_attempt_lineage_v4_is_denied_burn_evidence_with_no_production_writer() {
    let lineage = source("phase_a_attempt_lineage_v4.rs");
    let online = source("online_preflight_v2.rs");
    let journal = source("journal.rs");
    let production = lineage
        .split_once("\n#[cfg(test)]\nmod tests")
        .map_or(lineage.as_str(), |(production, _)| production);

    for required in [
        "PM_PHASE_A_ATTEMPT_LINEAGE_LEDGER_FILE_V4",
        "PM_PHASE_A_ATTEMPT_BURN_CLAIM_FILE_V4",
        "reap.pm-t2.phase-a.attempt-lineage.prepared.v4\\0",
        "reap.pm-t2.phase-a.attempt-lineage.burn-claim.v4\\0",
        "reap.pm-t2.phase-a.attempt-lineage.consumed.v4\\0",
        "reap.pm-t2.phase-a.attempt-lineage.final-conjunct.v4\\0",
        "PmPhaseAAttemptLineageInspectionV4",
        "PreparedUnclaimed",
        "ClaimOnlyBurned",
        "BurnedAwaitingFinalConjunct",
        "Complete",
        "AmbiguousBeforeClaim",
        "AmbiguousBurned",
        "placement_resumption_allowed(&self) -> bool",
        "OfflineAuthorizationState::DENIED",
        "PmPreparedPhaseAAttemptLineageV4",
        "PmBurnedPhaseAAttemptLineageEvidenceV4",
        "create_phase_a_attempt_lineage_prepared_v4",
        "burn_and_record_a3_with_refresh",
        "PmOnlinePreflightV2BoundCreateRefresh",
        "future,",
        "separately versioned positive successor",
        "Hash commitments attest no proof validity",
        "ProtectedArtifactLease::acquire",
        "lease.validate()",
        "same-EUID process bypassing the",
        "coordinated rollback",
        "TPM-backed monotonic anchor",
        "WORM",
        "authenticated remote registry",
        "([_, _, ..], None) => PmPhaseAAttemptLineageInspectionV4::AmbiguousBurned",
    ] {
        assert!(
            production.contains(required),
            "V4 lineage lost `{required}`"
        );
    }
    for forbidden in [
        "pub struct PmPreparedPhaseAAttemptLineageV4",
        "pub struct PmBurnedPhaseAAttemptLineageEvidenceV4",
        "pub fn create_phase_a_attempt_lineage_prepared_v4",
        "pub fn burn_and_complete",
        "pub fn into_parts",
        "pub fn into_network",
        "pub fn into_dispatch",
        "pub fn permit",
        "AuthenticatedPlaceRequest",
        "L2Credentials",
        "Hmac",
        "reqwest",
        "TcpStream",
        "SerializedPlaceRequest",
        "production_order_entry_authorized: true",
        "real_order_submission_authorized: true",
        "place_dispatch_allowance: 1",
        "synthetic_for_tests",
    ] {
        assert!(
            !production.contains(forbidden),
            "V4 lineage gained forbidden production surface `{forbidden}`"
        );
    }
    for owner in [
        "PmPreparedPhaseAAttemptLineageV4",
        "PmBurnedPhaseAAttemptLineageEvidenceV4",
    ] {
        let declaration = format!("pub(crate) struct {owner} {{");
        let before = production
            .split_once(&declaration)
            .map(|(before, _)| before)
            .unwrap_or_else(|| panic!("missing V4 owner `{owner}`"));
        let declaration_attributes = before
            .rsplit_once("\n}\n\n")
            .map_or(before, |(_, attributes)| attributes);
        assert!(
            !declaration_attributes.contains("#[derive"),
            "V4 owner `{owner}` gained a derived capability"
        );
        for trait_name in ["Clone", "Copy", "Serialize", "Deserialize"] {
            assert!(
                !production.contains(&format!("impl {trait_name} for {owner}")),
                "V4 owner `{owner}` gained `{trait_name}`"
            );
        }
    }

    let burn = production
        .split_once("pub(crate) fn burn_and_complete(")
        .expect("V4 burn writer")
        .1
        .split_once("struct PhaseAAttemptHeldParentRefreshV4")
        .expect("bounded V4 burn writer")
        .0;
    let burn = compact_whitespace(burn);
    let claim_create = burn
        .find("let mut claim_file = ProtectedJournal::create_new(")
        .expect("create-new V4 burn claim");
    let consumed_append = burn
        .find("ledger.append_durable(&expected_ledger_bytes, &consumed_bytes)")
        .expect("V4 Consumed append");
    let existing_lineage = burn
        .find("basis.burn_and_record_a3_with_refresh(")
        .expect("unchanged V2/V1/A3 lineage");
    let final_append = burn
        .find("ledger.append_durable(&expected_ledger_bytes, &final_bytes)")
        .expect("V4 final-conjunct append");
    let final_revalidation = burn
        .find("owner.revalidate_held_evidence(journals)")
        .expect("full final revalidation");
    assert!(
        claim_create < consumed_append
            && consumed_append < existing_lineage
            && existing_lineage < final_append
            && final_append < final_revalidation
    );

    let settled_existing = online
        .split_once("pub(crate) fn burn_and_record_a3_with_refresh(")
        .expect("sealed existing-lineage writer")
        .1
        .split_once("impl fmt::Debug for PmPendingPhaseAOnlinePreflightBasisV2")
        .expect("bounded existing-lineage writer")
        .0;
    let settled_existing = compact_whitespace(settled_existing);
    let v2_burn = settled_existing
        .find("v2_consumption .consume")
        .expect("V2 burn");
    let v1_burn = settled_existing
        .find("v1_consumption .consume")
        .expect("V1 burn");
    let v1_a3 = settled_existing
        .find("record_phase_a_place_live_dispatch_authorized")
        .expect("V1 DispatchAuthorized+A3");
    let v2_conjunct = settled_existing
        .find("append_durable(&expected_sidecar_bytes, &encoded)")
        .expect("V2 final conjunct");
    assert!(v2_burn < v1_burn && v1_burn < v1_a3 && v1_a3 < v2_conjunct);
    assert_eq!(
        settled_existing
            .matches(".refresh_after_bound_create(&mut sidecar, &expected_sidecar_bytes)")
            .count(),
        3,
        "V4 parents and exact sidecar bytes must refresh/revalidate after V2 claim, V1 claim, and A3 barrier"
    );
    assert!(
        settled_existing.contains("record_phase_a_place_live_dispatch_authorized_with_refresh")
    );
    assert!(
        journal.contains(
            "additional_refresh: &mut impl FnMut() -> Result<(), PmTrialLiveJournalError>"
        )
    );

    let assert_aggregated_before = |block: &str, first_check: &str, attempts: &[&str]| {
        let first_check = block
            .find(first_check)
            .unwrap_or_else(|| panic!("missing first aggregate check `{first_check}`"));
        for attempt in attempts {
            let attempt_index = block
                .find(attempt)
                .unwrap_or_else(|| panic!("missing aggregate attempt `{attempt}`"));
            assert!(
                attempt_index < first_check,
                "aggregate attempt `{attempt}` moved after the first error check"
            );
        }
    };

    let prepared_writer = production
        .split_once("pub(crate) fn create_phase_a_attempt_lineage_prepared_v4(")
        .expect("V4 Prepared writer")
        .1
        .split_once("impl PmPreparedPhaseAAttemptLineageV4")
        .expect("bounded V4 Prepared writer")
        .0;
    let prepared_writer = compact_whitespace(prepared_writer);
    assert_aggregated_before(
        &prepared_writer,
        "if let Err(error) = basis_refresh",
        &[
            "let basis_refresh =",
            "let ledger_refresh =",
            "let ledger_validation =",
        ],
    );

    let v4_claim_refresh = burn
        .split_once("claim_file.append_durable(&[], &claim_bytes)?;")
        .expect("V4 claim durable append")
        .1
        .split_once("let consumed =")
        .expect("bounded post-claim refresh")
        .0;
    assert_aggregated_before(
        v4_claim_refresh,
        "ledger_refresh?;",
        &[
            "let ledger_refresh =",
            "let claim_refresh =",
            "let basis_refresh =",
            "let ledger_validation =",
            "let claim_validation =",
        ],
    );

    let v2_claim_refresh = settled_existing
        .split_once("let mut v2_consumption =")
        .expect("V2 consumption")
        .1
        .split_once("let v1_consumption = v1_consumption")
        .expect("bounded post-V2-claim refresh")
        .0;
    assert_aggregated_before(
        v2_claim_refresh,
        "if let Err(error) = sidecar_refresh",
        &[
            "let sidecar_refresh =",
            "let v1_refresh =",
            "let journal_refresh =",
            "let additional_refresh_result =",
        ],
    );

    let v1_claim_refresh = settled_existing
        .split_once("let v1_consumption = v1_consumption")
        .expect("V1 consumption")
        .1
        .split_once("let verification =")
        .expect("bounded post-V1-claim refresh")
        .0;
    assert_aggregated_before(
        v1_claim_refresh,
        "if let Err(error) = sidecar_refresh",
        &[
            "let sidecar_refresh =",
            "let v2_refresh =",
            "let journal_refresh =",
            "let additional_refresh_result =",
        ],
    );

    let a3_outer_refresh = settled_existing
        .split_once("let mut refresh_after_a3_create = || {")
        .expect("A3 outer refresh callback")
        .1
        .split_once("}; journals.record_phase_a_place_live_dispatch_authorized_with_refresh(")
        .expect("bounded A3 outer refresh callback")
        .0;
    assert_aggregated_before(
        a3_outer_refresh,
        "sidecar_refresh?;",
        &[
            "let sidecar_refresh =",
            "let v2_refresh =",
            "let additional_refresh_result =",
        ],
    );

    let a3_journal_refresh = journal
        .split_once("pub(crate) fn record_phase_a_place_live_dispatch_authorized_with_refresh(")
        .expect("A3 journal refresh seam")
        .1
        .split_once("fn record_place_dispatch_authorized_inner(")
        .expect("bounded A3 journal refresh seam")
        .0;
    let a3_journal_refresh = compact_whitespace(a3_journal_refresh);
    assert_aggregated_before(
        &a3_journal_refresh,
        "artifact_refresh?;",
        &[
            "let artifact_refresh =",
            "let intent_refresh =",
            "let dispatch_refresh =",
            "let authorization_refresh =",
            "let additional_refresh_result =",
            "let artifact_validation =",
        ],
    );

    let callback = production
        .split_once("struct PhaseAAttemptHeldParentRefreshV4")
        .expect("V4 held-parent callback")
        .1
        .split_once("fn make_prepared(")
        .expect("bounded V4 held-parent callback")
        .0;
    for required in [
        "expected_ledger_bytes: &'a [u8]",
        "expected_claim_bytes: &'a [u8]",
        "sidecar.validate_exact_bytes(expected_sidecar_bytes)",
        "validate_exact_bytes(self.expected_ledger_bytes)",
        "validate_exact_bytes(self.expected_claim_bytes)",
    ] {
        assert!(
            callback.contains(required),
            "V4 refresh callback lost exact-byte obligation `{required}`"
        );
    }
    let callback = compact_whitespace(callback);
    assert_aggregated_before(
        &callback,
        "sidecar_validation?;",
        &[
            "let sidecar_validation =",
            "let ledger =",
            "let claim =",
            "let ledger_validation =",
            "let claim_validation =",
        ],
    );

    let evidence_revalidation = online
        .split_once("pub(crate) fn revalidate_phase_a_online_preflight_v2_evidence_only(")
        .expect("sealed V2 evidence-only revalidation")
        .1
        .split_once("/// Consume the complete V2 composite")
        .expect("bounded sealed V2 evidence-only revalidation")
        .0;
    assert_eq!(
        evidence_revalidation
            .matches("owner.validate_held_complete_set()")
            .count(),
        2,
        "V2 exact set must bracket the fresh V1 evidence check"
    );
    assert!(
        evidence_revalidation
            .contains("self.revalidate_phase_a_place_evidence_only(&mut owner.v1)")
    );
    assert!(!evidence_revalidation.contains("PmPhaseAOnlinePreflightNetworkDispatchOwnerV2"));
    assert!(!evidence_revalidation.contains("revalidate_phase_a_place_for_network_dispatch"));

    let v1_evidence_revalidation = journal
        .split_once("pub(crate) fn revalidate_phase_a_place_evidence_only(")
        .expect("sealed V1 evidence-only revalidation")
        .1
        .split_once("/// Consume the sole combined DND token")
        .expect("bounded sealed V1 evidence-only revalidation")
        .0;
    for required in [
        "require_phase_a_live_epoch",
        "require_phase_a_live_dispatch_tail",
        "validate_phase_a_live_durable_set",
        "grant_matches_v1_dispatch",
    ] {
        assert!(v1_evidence_revalidation.contains(required));
    }
    for forbidden in [
        "PmPhaseAPlaceNetworkDispatchOwnerV1",
        "PmPhaseAPlaceLiveDispatchProfileV1",
        "into_may_have_been_dispatched",
    ] {
        assert!(!v1_evidence_revalidation.contains(forbidden));
    }
}

#[test]
fn online_preflight_v2_is_an_additive_denied_two_record_conjunct() {
    let online = source("online_preflight_v2.rs");
    let journal = source("journal.rs");
    let recovery = source("recovery.rs");
    let continuation = source("recovery_continuation.rs");
    let schema = source("schema.rs");

    assert!(online.contains("PM_PHASE_A_ONLINE_PREFLIGHT_SIDECAR_FILE_V2"));
    assert!(online.contains("OnlinePreflightSidecarBodyV2::Basis"));
    assert!(online.contains("OnlinePreflightSidecarBodyV2::A3Conjunct"));
    assert!(online.contains("ProtectedJournal::create_new"));
    assert!(online.contains("sidecar.append_durable(&[], &encoded)"));
    assert!(online.contains("append_durable(&expected_sidecar_bytes, &encoded)"));
    assert!(online.contains("OfflineAuthorizationState::DENIED"));
    assert!(online.contains("network_send_authority"));
    assert!(online.contains("v2_consumption\n            .consume"));
    assert!(online.contains("v1_consumption\n            .consume"));
    assert!(online.contains("record_phase_a_place_live_dispatch_authorized"));
    assert!(!online.contains("ExistingV1LifecycleOnlyNoPlacementResume"));
    assert!(!online.contains("pub fn into_parts"));
    assert!(!online.contains("pub fn send"));
    assert!(!online.contains("Hmac"));
    assert!(!online.contains("SerializedPlaceRequest"));
    for owner in [
        "PmPendingPhaseAOnlinePreflightBasisV2",
        "PmPhaseAOnlinePreflightDispatchOwnerV2",
        "PmPhaseAOnlinePreflightNetworkDispatchOwnerV2",
        "PmPhaseAOnlinePreflightPostA3FailureV2",
    ] {
        assert!(online.contains(&format!("pub struct {owner}")));
        assert!(!online.contains(&format!("impl Clone for {owner}")));
        assert!(!online.contains(&format!("impl Serialize for {owner}")));
        assert!(!online.contains(&format!("impl Deserialize for {owner}")));
    }
    let pending_impl = online
        .split_once("impl PmPendingPhaseAOnlinePreflightBasisV2 {")
        .expect("pending V2 owner impl")
        .1
        .split_once("impl fmt::Debug for PmPendingPhaseAOnlinePreflightBasisV2")
        .expect("bounded pending V2 owner impl")
        .0;
    assert!(!pending_impl.contains("public_request_identity"));
    assert!(!pending_impl.contains("l2_timestamp_seconds"));
    assert!(pending_impl.contains("pub const fn online_policy(&self)"));
    assert!(pending_impl.contains("-> &reap_pm_controlled_trial::CanonicalOnlinePolicyV2"));
    assert!(pending_impl.contains("pub const fn online_authorization("));
    assert!(pending_impl.contains("-> &reap_pm_controlled_trial::CanonicalOnlineAuthorizationV2"));
    assert!(!pending_impl.contains("-> reap_pm_controlled_trial::CanonicalOnlinePolicyV2"));
    assert!(!pending_impl.contains("-> reap_pm_controlled_trial::CanonicalOnlineAuthorizationV2"));
    let composite_impl = online
        .split_once("impl PmPhaseAOnlinePreflightDispatchOwnerV2 {")
        .expect("composite V2 owner impl")
        .1
        .split_once("impl fmt::Debug for PmPhaseAOnlinePreflightDispatchOwnerV2")
        .expect("bounded composite V2 owner impl")
        .0;
    assert!(composite_impl.contains("pub const fn public_request_identity(&self)"));
    assert!(composite_impl.contains("pub const fn l2_timestamp_seconds(&self)"));
    assert!(online.contains("revalidate_phase_a_online_preflight_v2_for_network_dispatch"));
    assert!(online.contains("self.revalidate_phase_a_place_for_network_dispatch(v1)"));
    assert!(online.contains("pub fn into_may_have_been_dispatched("));
    assert!(online.contains("pub fn into_definitely_not_dispatched("));
    assert!(!online.contains("Deref for PmPhaseAOnlinePreflight"));
    assert!(!online.contains("pub fn v1("));
    assert!(!online.contains("pub fn profile("));

    assert!(journal.contains("refresh_after_online_preflight_v2_artifact_create"));
    assert!(!schema.contains("OnlinePreflight"));
    assert!(!recovery.contains("OnlinePreflight"));
    assert!(!continuation.contains("OnlinePreflight"));
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
fn production_cancel_authority_is_one_epoch_bound_nonserde_custody_chain() {
    let journal = source("journal.rs");
    let schema = source("schema.rs");
    let lib = source("lib.rs");
    for owner in [
        "PmPhaseAPlaceLiveResultCustodyV1",
        "PmPhaseAPlaceLiveOutcomeCustodyV1",
        "PmPhaseALiveReconciliationCustodyV1",
        "PmPhaseALiveCancelIntentCustodyV1",
        "PmPhaseALiveCancelPreparedCustodyV1",
        "PmPhaseALiveCancelDispatchOwnerV1",
        "PmRevalidatedPhaseALiveCancelDispatchOwnerV1",
        "PmPhaseALiveCancelNetworkDispatchOwnerV1",
        "PmPhaseALiveCancelMayHaveBeenDispatchedV1",
        "PmPhaseALiveCancelDefinitelyNotDispatchedV1",
        "PmPhaseALiveCancelResultCustodyV1",
        "PmPhaseALiveCancelOutcomeCustodyV1",
    ] {
        assert!(journal.contains(&format!("pub struct {owner}")));
        assert!(lib.contains(owner));
        assert!(!journal.contains(&format!("impl Clone for {owner}")));
        assert!(!journal.contains(&format!("impl Serialize for {owner}")));
        assert!(!journal.contains(&format!("impl Deserialize for {owner}")));
    }
    assert!(journal.contains("enum PmPhaseALiveCustodyEpochV1"));
    assert!(journal.contains("phase_a_live_cancel_epoch"));
    assert!(journal.contains("transition_phase_a_live_custody_to_cancel_epoch"));
    assert!(journal.contains("revalidate_phase_a_live_cancel_for_runner"));
    assert!(journal.contains("revalidate_phase_a_live_cancel_for_network_dispatch"));
    assert!(journal.contains("journals: &'journal mut PmControlledTrialLiveJournals"));
    assert!(journal.contains("record_phase_a_live_cancel_definitely_not_dispatched"));
    assert!(schema.contains("DefinitelyNotDispatched"));
    assert!(!journal.contains("fn into_parts"));
    assert!(!journal.contains(
        "revalidate_phase_a_live_cancel_for_runner(\n        &mut self,\n        owner: PmDurablePhaseALiveCancelDispatchAckV1"
    ));
}

#[test]
fn production_recovery_cancel_requires_distinct_current_runtime_bound_wrapper() {
    let journal = source("journal.rs");
    let lib = source("lib.rs");
    assert!(journal.contains("pub struct PmControlledTrialLiveCancelRecoveryJournals"));
    assert!(lib.contains("PmControlledTrialLiveCancelRecoveryJournals"));
    assert!(journal.contains("pub fn open_phase_a_live_cancel("));
    assert!(journal.contains("current_runtime: &AuthorizationRuntimeBinding"));
    assert!(journal.contains("current_utc > cleanup_not_after_utc"));
    assert!(journal.contains("current_runtime.host != authorization_value.host"));
    assert!(journal.contains("reopen_consumed_authorization_consumption"));
    assert!(journal.contains("barrier: Option<PmPhaseAPlaceDispatchBarrierWitnessV1>"));
    let evidence_impl = journal
        .split_once("impl PmControlledTrialLiveRecoveryJournals {")
        .expect("evidence recovery impl")
        .1
        .split_once("impl PmControlledTrialLiveCancelRecoveryJournals {")
        .expect("distinct live recovery impl")
        .0;
    assert!(!evidence_impl.contains("record_phase_a_live_reconciliation_with_custody"));
    assert!(!evidence_impl.contains("revalidate_phase_a_live_cancel_for_runner"));
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

#[test]
fn recovery_continuation_is_closed_cancel_only_false_false_zero_evidence() {
    let continuation = source("recovery_continuation.rs");
    let recovery = source("recovery.rs");
    let journal = source("journal.rs");
    let lib = source("lib.rs");

    let intent_records = continuation
        .split_once("enum ContinuationIntentRecordV1")
        .expect("continuation intent records")
        .1
        .split_once("enum ContinuationDispatchRecordV1")
        .expect("continuation dispatch records")
        .0;
    let dispatch_records = continuation
        .split_once("enum ContinuationDispatchRecordV1")
        .expect("continuation dispatch records")
        .1
        .split_once("struct ContinuationIntentLineV1")
        .expect("continuation lines")
        .0;
    assert!(!intent_records.contains("Place"));
    assert!(!dispatch_records.contains("Place"));
    assert!(!continuation.contains("pub(crate) fn record_place"));
    assert!(continuation.contains("production_order_entry_authorized: false"));
    assert!(continuation.contains("real_order_submission_authorized: false"));
    assert!(continuation.contains("place_dispatch_allowance: 0"));
    assert!(continuation.contains("placement_resumption_allowed: false"));
    assert!(continuation.contains("self.intent.file.validate_exact_bytes"));
    assert!(continuation.contains("validate_exact_bytes(&self.dispatch.bytes)"));
    assert!(journal.contains("revalidate_held_consumption_evidence"));
    assert!(journal.contains("validate_phase_a_live_journal_files"));
    assert!(journal.contains("complete_pending_terminal"));
    assert!(journal.contains("resume_phase_a_live_cancel_outcome_with_custody"));
    assert!(continuation.contains("validate_consumption_registry"));
    assert!(continuation.contains("validate_fully_anchored_against_consumption_registry"));
    assert!(journal.contains("complete_consumption_registry"));
    assert!(continuation.contains("record_cancel_prepared_ledger_first"));
    assert!(continuation.contains("continuation_prepared_record_canonical_json"));
    assert!(continuation.contains("complete_one_anchored_prepared"));
    assert!(continuation.contains("complete_anchored_terminal_plan"));
    assert!(continuation.contains("reconstruct_terminal_plan_lines"));
    assert!(continuation.contains("ContinuationTerminalPlanPhysicalStateV1::MissingBoth"));
    assert!(continuation.contains("ContinuationTerminalPlanPhysicalStateV1::DispatchOnly"));
    assert!(continuation.contains("ContinuationTerminalPlanPhysicalStateV1::Complete"));
    assert!(!continuation.contains("for prepared in self.prepared_anchor_evidence()?"));
    let ledger_first = continuation
        .split_once("pub(crate) fn record_cancel_prepared_ledger_first")
        .expect("ledger-first recovery preparation writer")
        .1
        .split_once("pub(crate) fn record_cancel_dispatch_authorized")
        .expect("bounded recovery preparation writer")
        .0;
    assert!(
        ledger_first
            .find(".anchor_recovery_cancel_prepared(")
            .expect("monotonic ordinal anchor")
            < ledger_first
                .find(".complete_one_anchored_prepared(&registry)")
                .expect("source-owned exact pair completion")
    );
    let terminal_ledger_first = continuation
        .split_once("pub(crate) fn record_terminal(")
        .expect("ledger-first recovery Terminal writer")
        .1
        .split_once("pub(crate) fn has_complete_anchored_terminal_plan")
        .expect("bounded recovery Terminal writer")
        .0;
    assert!(
        terminal_ledger_first
            .find(".anchor_recovery_terminal_plan(")
            .expect("monotonic Terminal plan anchor")
            < terminal_ledger_first
                .find(".complete_anchored_terminal_plan(&registry)")
                .expect("source-owned exact Terminal completion")
    );
    assert!(journal.contains("continuation.complete_consumption_registry"));
    assert!(recovery.contains("is_completed_recovery_continuation_terminal"));
    assert!(journal.contains("allow_completed_recovery_continuation_terminal"));
    assert!(journal.contains("Self::open_inner(config, authorization, projection, false)"));

    for action in [
        "ReconcileCurrentExposure",
        "ResumeCancelOutcome",
        "RecordTerminal",
        "CompletePendingTerminal",
        "TerminalEvidenceOnly",
    ] {
        assert!(recovery.contains(action));
    }
    assert!(lib.contains("PmPhaseALiveCancelRecoveryRequiredActionV1"));
    assert!(recovery.contains("phase_a_live_cancel_recovery_required_action"));
}

#[test]
fn prepared_supersession_requires_the_latest_exact_live_ownership_event() {
    let journal = source("journal.rs");
    let recovery = source("recovery.rs");
    assert!(journal.contains("validate_cancel_prepared_supersession"));
    assert!(journal.contains("ownership_source.sequence.checked_add(1)"));
    assert!(journal.contains("reconciled_target.sequence != preserved_exposure.sequence"));
    assert!(recovery.contains("latest_cancel_ownership_source.as_ref() != Some(ownership_source)"));
    assert!(recovery.contains("ownership_source.sequence.checked_add(1)"));
    assert!(recovery.contains("DispatchStage::CancelPrepared"));
    assert!(recovery.contains("reconciled_target != preserved_exposure"));
}
