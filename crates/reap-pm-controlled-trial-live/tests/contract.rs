mod support;

use std::{
    fs::{self, OpenOptions},
    io::Write as _,
    os::unix::fs::{MetadataExt as _, PermissionsExt as _},
    path::Path,
};

use reap_pm_controlled_trial::{
    FixedOrderId, PreparedAuthorizationConsumption, verify_authorization_consumption,
};
use reap_pm_controlled_trial_live::{
    PM_PHASE_A_LIVE_DISPATCH_BARRIER_FILE_V1, PM_TRIAL_LIVE_DISPATCH_FILE_V1,
    PM_TRIAL_LIVE_INTENT_FILE_V1, PmCancelDispatchClassV1, PmCancelPreparationV1,
    PmCancelResultKindV1, PmControlledTrialLiveJournals, PmControlledTrialLiveRecoveryJournals,
    PmIntentTerminalDispositionV1, PmPlacePreparationV1, PmPlaceResultKindV1,
    PmReconciliationOrderStateV1, PmTrialLiveJournalError, PmTrialLiveRecoveryClassificationV1,
    verify_controlled_trial_live_recovery,
};
use serde::Serialize;
use sha2::{Digest as _, Sha256};

use support::{Fixture, l2_time};

const PLACE_TIME: &str = "2026-08-09T12:05:04Z";
const CANCEL_TIME: &str = "2026-08-09T12:06:00Z";
const INTENT_FINGERPRINT_DOMAIN: &[u8] = b"reap.pm-t2.controlled-trial-live.intent-record.v1\0";
const DISPATCH_FINGERPRINT_DOMAIN: &[u8] = b"reap.pm-t2.controlled-trial-live.dispatch-record.v1\0";

#[derive(Serialize)]
struct TestCounterpartLink {
    sequence: u8,
    record_fingerprint: String,
}

#[derive(Serialize)]
struct TestJournalLine<B> {
    schema_version: u32,
    sequence: u8,
    previous_record_fingerprint: String,
    scope_fingerprint: String,
    body: B,
}

#[derive(Serialize)]
struct TestDispatchTerminal {
    record: &'static str,
    terminal_at_utc: &'static str,
    intent: TestCounterpartLink,
    terminal_is_evidence_not_authority: bool,
}

#[derive(Serialize)]
struct TestIntentTerminal {
    record: &'static str,
    terminal_at_utc: &'static str,
    disposition: &'static str,
    dispatch_terminal: TestCounterpartLink,
    terminal_is_evidence_not_authority: bool,
}

fn bind_preflight(fixture: &Fixture) -> PmControlledTrialLiveJournals {
    let pending = PmControlledTrialLiveJournals::create_pending_preflight(
        &fixture.config,
        &fixture.authorization,
        &fixture.runtime("2026-08-09T12:05:00Z"),
    )
    .expect("pending journals");
    let preflight = fixture.canonical_preflight(pending.lease_evidence().clone());
    pending.bind_preflight(preflight).expect("bind preflight")
}

fn place_preparation(
    fixture: &Fixture,
    journals: &PmControlledTrialLiveJournals,
) -> PmPlacePreparationV1 {
    PmPlacePreparationV1::for_scoped_plan(
        &fixture.config,
        journals.preflight_binding(),
        fixture.config.exact_place_public_request_identity(),
        l2_time(PLACE_TIME),
    )
    .expect("exact place preparation")
}

fn record_place_prepared(
    fixture: &Fixture,
    journals: &mut PmControlledTrialLiveJournals,
) -> reap_pm_controlled_trial_live::PmDurablePlacePreparedAckV1 {
    let intent = journals
        .record_place_intent(PLACE_TIME.into())
        .expect("place intent");
    let preparation = place_preparation(fixture, journals);
    journals
        .record_place_prepared(intent, preparation)
        .expect("place prepared")
}

fn record_place_dispatch(
    fixture: &Fixture,
    prepared_consumption: PreparedAuthorizationConsumption,
    journals: &mut PmControlledTrialLiveJournals,
) -> (
    reap_pm_controlled_trial_live::PmDurablePlaceDispatchAckV1,
    reap_pm_controlled_trial::ConsumedAuthorizationConsumption,
) {
    let prepared = record_place_prepared(fixture, journals);
    let consumed = prepared_consumption
        .consume(
            &fixture.config,
            &fixture.authorization,
            &fixture.runtime(PLACE_TIME),
        )
        .expect("consume authorization");
    let verification = verify_authorization_consumption(&fixture.config, &fixture.authorization)
        .expect("consumption verification");
    let proof = journals
        .bind_consumed_authorization(prepared, consumed, &verification)
        .expect("bind consumed authorization");
    journals
        .record_place_dispatch_authorized(proof)
        .expect("durable dispatch evidence")
}

fn record_phase_a_live_dispatch(
    fixture: &Fixture,
    prepared_consumption: PreparedAuthorizationConsumption,
    journals: &mut PmControlledTrialLiveJournals,
) -> reap_pm_controlled_trial_live::PmPhaseAPlaceLiveDispatchOwnerV1 {
    let prepared = record_place_prepared(fixture, journals);
    let consumed = prepared_consumption
        .consume(
            &fixture.config,
            &fixture.authorization,
            &fixture.runtime(PLACE_TIME),
        )
        .expect("consume authorization");
    let verification = verify_authorization_consumption(&fixture.config, &fixture.authorization)
        .expect("consumption verification");
    let proof = journals
        .bind_consumed_authorization(prepared, consumed, &verification)
        .expect("bind consumed authorization");
    journals
        .record_phase_a_place_live_dispatch_authorized(proof)
        .expect("durable positive live dispatch barrier")
}

fn retain_complete_lines(path: &Path, count: usize) {
    let bytes = fs::read(path).expect("read journal");
    let end = bytes
        .iter()
        .enumerate()
        .filter_map(|(index, byte)| (*byte == b'\n').then_some(index + 1))
        .nth(count.saturating_sub(1))
        .expect("requested complete line exists");
    fs::write(path, &bytes[..end]).expect("truncate to complete records");
}

fn append_and_sync(path: &Path, suffix: &[u8]) {
    let mut file = OpenOptions::new()
        .append(true)
        .open(path)
        .expect("open protected file for append drift");
    file.write_all(suffix).expect("append drift");
    file.sync_all().expect("sync append drift");
}

fn rewrite_same_inode_same_length(path: &Path) {
    let mut bytes = fs::read(path).expect("read protected file for exact-byte drift");
    let index = bytes
        .iter()
        .position(|byte| *byte == b'{')
        .expect("canonical JSON object prefix");
    bytes[index] = b'[';
    fs::write(path, bytes).expect("same-inode same-length drift");
}

fn fingerprint_bytes(domain: &[u8], bytes: &[u8]) -> String {
    let mut hasher = Sha256::new();
    hasher.update(domain);
    hasher.update(bytes);
    hasher
        .finalize()
        .iter()
        .map(|byte| format!("{byte:02x}"))
        .collect()
}

fn append_unsafe_positive_terminal_tails(fixture: &Fixture) {
    const TERMINAL_AT: &str = "2026-08-09T12:05:08Z";
    let intent_path = fixture.path(PM_TRIAL_LIVE_INTENT_FILE_V1);
    let dispatch_path = fixture.path(PM_TRIAL_LIVE_DISPATCH_FILE_V1);
    let intent_bytes = fs::read(&intent_path).expect("intent journal");
    let dispatch_bytes = fs::read(&dispatch_path).expect("dispatch journal");
    let intent_last = intent_bytes
        .strip_suffix(b"\n")
        .expect("complete intent tail")
        .rsplit(|byte| *byte == b'\n')
        .next()
        .expect("intent tail");
    let dispatch_last = dispatch_bytes
        .strip_suffix(b"\n")
        .expect("complete dispatch tail")
        .rsplit(|byte| *byte == b'\n')
        .next()
        .expect("dispatch tail");
    let intent_value: serde_json::Value =
        serde_json::from_slice(intent_last).expect("intent tail JSON");
    let dispatch_value: serde_json::Value =
        serde_json::from_slice(dispatch_last).expect("dispatch tail JSON");
    let intent_sequence = u8::try_from(intent_value["sequence"].as_u64().expect("intent sequence"))
        .expect("bounded intent sequence");
    let dispatch_sequence = u8::try_from(
        dispatch_value["sequence"]
            .as_u64()
            .expect("dispatch sequence"),
    )
    .expect("bounded dispatch sequence");
    let schema_version = u32::try_from(
        dispatch_value["schema_version"]
            .as_u64()
            .expect("schema version"),
    )
    .expect("bounded schema version");
    let scope_fingerprint = dispatch_value["scope_fingerprint"]
        .as_str()
        .expect("scope fingerprint")
        .to_owned();

    let dispatch_terminal = TestJournalLine {
        schema_version,
        sequence: dispatch_sequence + 1,
        previous_record_fingerprint: fingerprint_bytes(DISPATCH_FINGERPRINT_DOMAIN, dispatch_last),
        scope_fingerprint: scope_fingerprint.clone(),
        body: TestDispatchTerminal {
            record: "terminal",
            terminal_at_utc: TERMINAL_AT,
            intent: TestCounterpartLink {
                sequence: intent_sequence,
                record_fingerprint: fingerprint_bytes(INTENT_FINGERPRINT_DOMAIN, intent_last),
            },
            terminal_is_evidence_not_authority: true,
        },
    };
    let dispatch_terminal_bytes =
        serde_json::to_vec(&dispatch_terminal).expect("canonical dispatch terminal");
    append_and_sync(
        &dispatch_path,
        &[dispatch_terminal_bytes.as_slice(), b"\n"].concat(),
    );

    let intent_terminal = TestJournalLine {
        schema_version,
        sequence: intent_sequence + 1,
        previous_record_fingerprint: fingerprint_bytes(INTENT_FINGERPRINT_DOMAIN, intent_last),
        scope_fingerprint,
        body: TestIntentTerminal {
            record: "terminal",
            terminal_at_utc: TERMINAL_AT,
            disposition: "operator_action_required",
            dispatch_terminal: TestCounterpartLink {
                sequence: dispatch_sequence + 1,
                record_fingerprint: fingerprint_bytes(
                    DISPATCH_FINGERPRINT_DOMAIN,
                    &dispatch_terminal_bytes,
                ),
            },
            terminal_is_evidence_not_authority: true,
        },
    };
    let intent_terminal_bytes =
        serde_json::to_vec(&intent_terminal).expect("canonical intent terminal");
    append_and_sync(
        &intent_path,
        &[intent_terminal_bytes.as_slice(), b"\n"].concat(),
    );
}

fn assert_final_place_revalidation_rejects_drift(drift: impl FnOnce(&Fixture)) {
    let (fixture, prepared_consumption) = Fixture::new();
    let mut journals = bind_preflight(&fixture);
    let owner = record_phase_a_live_dispatch(&fixture, prepared_consumption, &mut journals)
        .revalidate_for_runner()
        .expect("runner-side barrier revalidation");
    drift(&fixture);
    assert!(matches!(
        journals.revalidate_phase_a_place_for_network_dispatch(owner),
        Err(PmTrialLiveJournalError::Protection)
    ));
}

fn assert_transport_conversion_rejects_drift(drift: impl FnOnce(&Fixture)) {
    let (fixture, prepared_consumption) = Fixture::new();
    let mut journals = bind_preflight(&fixture);
    let owner = record_phase_a_live_dispatch(&fixture, prepared_consumption, &mut journals)
        .revalidate_for_runner()
        .expect("runner-side barrier revalidation");
    let final_owner = journals
        .revalidate_phase_a_place_for_network_dispatch(owner)
        .expect("final journal-bound owner");
    drift(&fixture);
    assert!(matches!(
        final_owner.into_may_have_been_dispatched(),
        Err(PmTrialLiveJournalError::Protection)
    ));
}

fn assert_classification(fixture: &Fixture, expected: PmTrialLiveRecoveryClassificationV1) {
    let projection = verify_controlled_trial_live_recovery(&fixture.config, &fixture.authorization)
        .expect("recovery projection");
    assert_eq!(projection.classification(), &expected);
    assert!(!projection.production_order_entry_authorized());
    assert!(!projection.real_order_submission_authorized());
    assert_eq!(projection.place_dispatch_allowance(), 0);
    assert!(!projection.placement_resumption_allowed());
}

#[test]
fn pending_creation_owns_exact_protected_dual_files_and_no_authority() {
    let (fixture, _prepared_consumption) = Fixture::new();
    let pending = PmControlledTrialLiveJournals::create_pending_preflight(
        &fixture.config,
        &fixture.authorization,
        &fixture.runtime("2026-08-09T12:05:00Z"),
    )
    .expect("pending journals");

    for name in [PM_TRIAL_LIVE_INTENT_FILE_V1, PM_TRIAL_LIVE_DISPATCH_FILE_V1] {
        let metadata = fs::metadata(fixture.path(name)).expect("journal metadata");
        assert_eq!(metadata.mode() & 0o7777, 0o600);
        assert_eq!(metadata.nlink(), 1);
    }
    let leases = pending.lease_evidence();
    assert_eq!(
        Path::new(&leases.product_journal_path)
            .file_name()
            .and_then(|name| name.to_str()),
        Some(PM_TRIAL_LIVE_INTENT_FILE_V1)
    );
    assert_eq!(
        Path::new(&leases.authenticated_journal_path)
            .file_name()
            .and_then(|name| name.to_str()),
        Some(PM_TRIAL_LIVE_DISPATCH_FILE_V1)
    );
    assert!(leases.artifact_directory_exclusive);
    assert!(leases.product_journal_exclusive);
    assert!(leases.authenticated_journal_exclusive);
    assert!(leases.leases_held_continuously);
    assert!(!pending.production_order_entry_authorized());
    assert!(!pending.real_order_submission_authorized());
    assert_eq!(pending.place_dispatch_allowance(), 0);
}

#[test]
fn exact_pending_lease_evidence_is_required_before_preflight_binding() {
    let (fixture, _prepared_consumption) = Fixture::new();
    let pending = PmControlledTrialLiveJournals::create_pending_preflight(
        &fixture.config,
        &fixture.authorization,
        &fixture.runtime("2026-08-09T12:05:00Z"),
    )
    .expect("pending journals");
    let mut foreign = pending.lease_evidence().clone();
    foreign.owner_process_identity.push_str("-foreign");
    let preflight = fixture.canonical_preflight(foreign);
    assert!(matches!(
        pending.bind_preflight(preflight),
        Err(PmTrialLiveJournalError::InvalidBinding)
    ));
}

#[test]
fn dual_file_crash_boundaries_are_fail_closed_and_never_restore_place() {
    // Intent header durable, dispatch path absent.
    {
        let (fixture, _prepared_consumption) = Fixture::new();
        let pending = PmControlledTrialLiveJournals::create_pending_preflight(
            &fixture.config,
            &fixture.authorization,
            &fixture.runtime("2026-08-09T12:05:00Z"),
        )
        .expect("pending journals");
        drop(pending);
        fs::remove_file(fixture.path(PM_TRIAL_LIVE_DISPATCH_FILE_V1))
            .expect("simulate pre-dispatch-create crash");
        assert_classification(
            &fixture,
            PmTrialLiveRecoveryClassificationV1::PrePreparedDefinitelyUnsent,
        );
    }

    // Intent header durable, dispatch file created but header append absent.
    {
        let (fixture, _prepared_consumption) = Fixture::new();
        let pending = PmControlledTrialLiveJournals::create_pending_preflight(
            &fixture.config,
            &fixture.authorization,
            &fixture.runtime("2026-08-09T12:05:00Z"),
        )
        .expect("pending journals");
        drop(pending);
        fs::write(fixture.path(PM_TRIAL_LIVE_DISPATCH_FILE_V1), [])
            .expect("simulate pre-header crash");
        assert_classification(
            &fixture,
            PmTrialLiveRecoveryClassificationV1::PrePreparedDefinitelyUnsent,
        );
    }

    // Both headers, before canonical preflight binding.
    {
        let (fixture, _prepared_consumption) = Fixture::new();
        let pending = PmControlledTrialLiveJournals::create_pending_preflight(
            &fixture.config,
            &fixture.authorization,
            &fixture.runtime("2026-08-09T12:05:00Z"),
        )
        .expect("pending journals");
        drop(pending);
        assert_classification(
            &fixture,
            PmTrialLiveRecoveryClassificationV1::PrePreparedDefinitelyUnsent,
        );
    }

    // Intent PreflightBound durable, dispatch counterpart not yet appended.
    {
        let (fixture, _prepared_consumption) = Fixture::new();
        let journals = bind_preflight(&fixture);
        drop(journals);
        retain_complete_lines(&fixture.path(PM_TRIAL_LIVE_DISPATCH_FILE_V1), 1);
        assert_classification(
            &fixture,
            PmTrialLiveRecoveryClassificationV1::PrePreparedDefinitelyUnsent,
        );
    }

    // PlaceIntent durable, PlacePrepared absent.
    {
        let (fixture, _prepared_consumption) = Fixture::new();
        let mut journals = bind_preflight(&fixture);
        let _intent = journals
            .record_place_intent(PLACE_TIME.into())
            .expect("place intent");
        drop(journals);
        assert_classification(
            &fixture,
            PmTrialLiveRecoveryClassificationV1::PrePreparedDefinitelyUnsent,
        );
    }

    // PlacePrepared durable, take-once claim absent.
    {
        let (fixture, _prepared_consumption) = Fixture::new();
        let mut journals = bind_preflight(&fixture);
        let _prepared = record_place_prepared(&fixture, &mut journals);
        drop(journals);
        assert_classification(
            &fixture,
            PmTrialLiveRecoveryClassificationV1::PreparedWithoutClaimDefinitelyUnsent,
        );
    }

    // Atomic consume claim durable, Consumed ledger completion absent.
    {
        let (fixture, prepared_consumption) = Fixture::new();
        let mut journals = bind_preflight(&fixture);
        let _prepared = record_place_prepared(&fixture, &mut journals);
        let consumed = prepared_consumption
            .consume(
                &fixture.config,
                &fixture.authorization,
                &fixture.runtime(PLACE_TIME),
            )
            .expect("consume authorization");
        drop(consumed);
        retain_complete_lines(
            &fixture.path(
                &fixture
                    .config
                    .value()
                    .journal
                    .authorization_consumption_ledger_file,
            ),
            1,
        );
        drop(journals);
        assert_classification(
            &fixture,
            PmTrialLiveRecoveryClassificationV1::AuthorizationBurnedNoPlace,
        );
    }

    // Consumed evidence durable, PlaceDispatchAuthorized absent.
    {
        let (fixture, prepared_consumption) = Fixture::new();
        let mut journals = bind_preflight(&fixture);
        let prepared = record_place_prepared(&fixture, &mut journals);
        let consumed = prepared_consumption
            .consume(
                &fixture.config,
                &fixture.authorization,
                &fixture.runtime(PLACE_TIME),
            )
            .expect("consume authorization");
        let verification =
            verify_authorization_consumption(&fixture.config, &fixture.authorization)
                .expect("consumption verification");
        let proof = journals
            .bind_consumed_authorization(prepared, consumed, &verification)
            .expect("bind consumed authorization");
        drop(proof);
        drop(journals);
        assert_classification(
            &fixture,
            PmTrialLiveRecoveryClassificationV1::AuthorizationBurnedNoPlace,
        );
    }

    // PlaceDispatchAuthorized durable, response/result and barrier absent. An
    // absent barrier could also have been unlinked after a possible send, so
    // recovery conservatively forbids resend and requires reconciliation.
    {
        let (fixture, prepared_consumption) = Fixture::new();
        let mut journals = bind_preflight(&fixture);
        let (dispatch, consumed) =
            record_place_dispatch(&fixture, prepared_consumption, &mut journals);
        assert_eq!(
            dispatch.preparation().expected_order_id(),
            fixture
                .config
                .exact_place_public_request_identity()
                .expected_order_id()
        );
        assert!(!dispatch.preparation().mutation_authority());
        drop(dispatch);
        drop(consumed);
        drop(journals);
        assert_classification(
            &fixture,
            PmTrialLiveRecoveryClassificationV1::PlaceMayHaveBeenSentNoResend,
        );
    }
}

#[test]
fn positive_phase_a_barrier_binds_the_exact_attempt_and_is_the_send_linearization_point() {
    let (fixture, prepared_consumption) = Fixture::new();
    let mut journals = bind_preflight(&fixture);
    let owner = record_phase_a_live_dispatch(&fixture, prepared_consumption, &mut journals);
    let owner = owner
        .revalidate_for_runner()
        .expect("exact barrier revalidation before runner join");
    let profile = owner.profile();
    let expected_identity = fixture.config.exact_place_public_request_identity();

    assert!(profile.production_order_entry_authorized());
    assert!(profile.real_order_submission_authorized());
    assert_eq!(profile.place_dispatch_allowance(), 1);
    assert!(!profile.placement_resumption_allowed());
    assert_eq!(
        profile.phase(),
        reap_pm_controlled_trial::TrialPhase::APlaceCancel
    );
    assert_eq!(
        profile.canonical_config_sha256(),
        fixture.config.canonical_sha256()
    );
    assert_eq!(
        profile.canonical_config_length(),
        fixture.config.canonical_length()
    );
    assert_eq!(
        profile.canonical_config_fingerprint(),
        fixture.config.fingerprint()
    );
    assert_eq!(
        profile.trial_plan_fingerprint(),
        fixture.config.plan_fingerprint()
    );
    assert_eq!(
        profile.authorization_id(),
        fixture.authorization.value().authorization_id
    );
    assert_eq!(
        profile.authorization_fingerprint(),
        fixture.authorization.fingerprint()
    );
    assert_eq!(profile.public_request_identity(), expected_identity);
    assert_eq!(
        profile.expected_order_id(),
        expected_identity.expected_order_id()
    );
    assert_eq!(
        profile.semantic_request_commitment(),
        expected_identity.semantic_request_commitment()
    );
    assert_eq!(profile.l2_timestamp_seconds(), l2_time(PLACE_TIME));
    assert_eq!(profile.build(), &fixture.authorization.value().build);
    assert_eq!(profile.host(), &fixture.authorization.value().host);
    assert_eq!(
        profile.credential_slot(),
        &fixture.config.value().credential_slot
    );
    for fingerprint in [
        profile.prepared_request_commitment(),
        profile.authorization_consumption_binding_fingerprint(),
        profile.authorization_consumption_prepared_record_fingerprint(),
        profile.atomic_claim_fingerprint(),
        profile.consumed_record_fingerprint(),
        profile.place_prepared_record_fingerprint(),
        profile.durable_barrier_record_fingerprint(),
    ] {
        assert_eq!(fingerprint.len(), 64);
        assert_ne!(fingerprint, "0".repeat(64));
    }
    let v1_lines = fs::read_to_string(fixture.path(PM_TRIAL_LIVE_DISPATCH_FILE_V1))
        .expect("V1 dispatch journal");
    let v1_dispatch_line = v1_lines
        .lines()
        .last()
        .expect("V1 dispatch acknowledgement");
    let v1_dispatch: serde_json::Value =
        serde_json::from_str(v1_dispatch_line).expect("V1 dispatch JSON");
    assert_eq!(v1_dispatch["body"]["record"], "place_dispatch_authorized");
    assert_eq!(
        v1_dispatch["body"]["production_order_entry_authorized"],
        false
    );
    assert_eq!(
        v1_dispatch["body"]["real_order_submission_authorized"],
        false
    );
    assert_eq!(v1_dispatch["body"]["place_dispatch_allowance"], 0);
    assert_eq!(
        profile.v1_dispatch_authorized_record_fingerprint(),
        fingerprint_bytes(DISPATCH_FINGERPRINT_DOMAIN, v1_dispatch_line.as_bytes())
    );

    let barrier_path = fixture.path(PM_PHASE_A_LIVE_DISPATCH_BARRIER_FILE_V1);
    let barrier_metadata = fs::metadata(&barrier_path).expect("barrier metadata");
    assert_eq!(barrier_metadata.mode() & 0o7777, 0o600);
    assert_eq!(barrier_metadata.nlink(), 1);
    let barrier_bytes = fs::read(&barrier_path).expect("barrier bytes");
    assert!(barrier_bytes.ends_with(b"\n"));
    let barrier: serde_json::Value =
        serde_json::from_slice(&barrier_bytes[..barrier_bytes.len() - 1]).expect("barrier JSON");
    assert_eq!(barrier["record"], "place_dispatch_authorized");
    assert_eq!(barrier["production_order_entry_authorized"], true);
    assert_eq!(barrier["real_order_submission_authorized"], true);
    assert_eq!(barrier["place_dispatch_allowance"], 1);
    assert_eq!(barrier["placement_resumption_allowed"], false);

    drop(owner);
    drop(journals);
    let projection = verify_controlled_trial_live_recovery(&fixture.config, &fixture.authorization)
        .expect("recovery projection");
    assert_eq!(
        projection.classification(),
        &PmTrialLiveRecoveryClassificationV1::PlaceMayHaveBeenSentNoResend
    );
    assert!(projection.phase_a_live_dispatch_barrier_durable());
    assert_eq!(
        projection.phase_a_live_dispatch_barrier_fingerprint(),
        Some(
            barrier["record_fingerprint"]
                .as_str()
                .expect("barrier fingerprint")
        )
    );
    assert!(!projection.placement_resumption_allowed());
}

#[test]
fn incomplete_positive_barriers_fail_closed_without_a_place_resume() {
    // The legacy V1 false/false/0 record remains unchanged, but barrier absence
    // is indistinguishable from post-send unlink and therefore fails closed as
    // may-have-sent/no-resend.
    {
        let (fixture, prepared_consumption) = Fixture::new();
        let mut journals = bind_preflight(&fixture);
        let (dispatch, consumed) =
            record_place_dispatch(&fixture, prepared_consumption, &mut journals);
        drop(dispatch);
        drop(consumed);
        drop(journals);
        assert_classification(
            &fixture,
            PmTrialLiveRecoveryClassificationV1::PlaceMayHaveBeenSentNoResend,
        );
    }

    // Create-new succeeded, but no canonical positive record was appended.
    {
        let (fixture, prepared_consumption) = Fixture::new();
        let mut journals = bind_preflight(&fixture);
        let (dispatch, consumed) =
            record_place_dispatch(&fixture, prepared_consumption, &mut journals);
        drop(dispatch);
        drop(consumed);
        drop(journals);
        let path = fixture.path(PM_PHASE_A_LIVE_DISPATCH_BARRIER_FILE_V1);
        fs::write(&path, []).expect("zero-byte barrier");
        fs::set_permissions(&path, fs::Permissions::from_mode(0o600))
            .expect("protect zero-byte barrier");
        assert!(matches!(
            verify_controlled_trial_live_recovery(&fixture.config, &fixture.authorization),
            Err(PmTrialLiveJournalError::AmbiguousTail)
        ));
    }

    // A torn record is never accepted as the full fsynced linearization point.
    {
        let (fixture, prepared_consumption) = Fixture::new();
        let mut journals = bind_preflight(&fixture);
        let owner = record_phase_a_live_dispatch(&fixture, prepared_consumption, &mut journals);
        drop(owner);
        drop(journals);
        let path = fixture.path(PM_PHASE_A_LIVE_DISPATCH_BARRIER_FILE_V1);
        let bytes = fs::read(&path).expect("complete barrier");
        fs::write(&path, &bytes[..bytes.len() / 2]).expect("torn barrier");
        assert!(matches!(
            verify_controlled_trial_live_recovery(&fixture.config, &fixture.authorization),
            Err(PmTrialLiveJournalError::AmbiguousTail)
        ));
    }
}

#[test]
fn runner_join_revalidation_rejects_barrier_path_or_content_drift() {
    // Rename removes the fixed recoverable pathname.
    {
        let (fixture, prepared_consumption) = Fixture::new();
        let mut journals = bind_preflight(&fixture);
        let owner = record_phase_a_live_dispatch(&fixture, prepared_consumption, &mut journals);
        let path = fixture.path(PM_PHASE_A_LIVE_DISPATCH_BARRIER_FILE_V1);
        fs::rename(&path, fixture.path("renamed-live-dispatch-barrier")).expect("rename barrier");
        assert!(matches!(
            owner.revalidate_for_runner(),
            Err(PmTrialLiveJournalError::Protection)
        ));
        drop(journals);
    }

    // Unlink likewise destroys recovery-path identity.
    {
        let (fixture, prepared_consumption) = Fixture::new();
        let mut journals = bind_preflight(&fixture);
        let owner = record_phase_a_live_dispatch(&fixture, prepared_consumption, &mut journals);
        fs::remove_file(fixture.path(PM_PHASE_A_LIVE_DISPATCH_BARRIER_FILE_V1))
            .expect("unlink barrier");
        assert!(matches!(
            owner.revalidate_for_runner(),
            Err(PmTrialLiveJournalError::Protection)
        ));
        drop(journals);
    }

    // Same bytes at a replacement inode are not the fsynced held barrier.
    {
        let (fixture, prepared_consumption) = Fixture::new();
        let mut journals = bind_preflight(&fixture);
        let owner = record_phase_a_live_dispatch(&fixture, prepared_consumption, &mut journals);
        let path = fixture.path(PM_PHASE_A_LIVE_DISPATCH_BARRIER_FILE_V1);
        let bytes = fs::read(&path).expect("barrier bytes");
        fs::remove_file(&path).expect("remove held barrier path");
        fs::write(&path, bytes).expect("replacement barrier");
        fs::set_permissions(&path, fs::Permissions::from_mode(0o600))
            .expect("protect replacement barrier");
        assert!(matches!(
            owner.revalidate_for_runner(),
            Err(PmTrialLiveJournalError::Protection)
        ));
        drop(journals);
    }

    // Appended bytes invalidate both the fixed length and canonical content.
    {
        let (fixture, prepared_consumption) = Fixture::new();
        let mut journals = bind_preflight(&fixture);
        let owner = record_phase_a_live_dispatch(&fixture, prepared_consumption, &mut journals);
        let path = fixture.path(PM_PHASE_A_LIVE_DISPATCH_BARRIER_FILE_V1);
        let mut file = OpenOptions::new()
            .append(true)
            .open(path)
            .expect("open barrier for append drift");
        file.write_all(b"x").expect("append drift");
        file.sync_all().expect("sync append drift");
        drop(file);
        assert!(matches!(
            owner.revalidate_for_runner(),
            Err(PmTrialLiveJournalError::Protection)
        ));
        drop(journals);
    }

    // Same-inode, same-length content drift is caught by the exact byte check.
    {
        let (fixture, prepared_consumption) = Fixture::new();
        let mut journals = bind_preflight(&fixture);
        let owner = record_phase_a_live_dispatch(&fixture, prepared_consumption, &mut journals);
        let path = fixture.path(PM_PHASE_A_LIVE_DISPATCH_BARRIER_FILE_V1);
        let mut bytes = fs::read(&path).expect("barrier bytes");
        let index = bytes
            .iter()
            .position(|byte| *byte == b'{')
            .expect("JSON object prefix");
        bytes[index] = b'[';
        fs::write(&path, bytes).expect("same-length barrier drift");
        assert!(matches!(
            owner.revalidate_for_runner(),
            Err(PmTrialLiveJournalError::Protection)
        ));
        drop(journals);
    }
}

#[test]
fn final_network_boundary_rechecks_every_held_durable_artifact() {
    // Runner-side validation is deliberately not timeless: later barrier
    // unlink/replacement is caught at the journal-bound transport handoff.
    assert_final_place_revalidation_rejects_drift(|fixture| {
        fs::remove_file(fixture.path(PM_PHASE_A_LIVE_DISPATCH_BARRIER_FILE_V1))
            .expect("unlink barrier after runner revalidation");
    });
    assert_final_place_revalidation_rejects_drift(|fixture| {
        let path = fixture.path(PM_PHASE_A_LIVE_DISPATCH_BARRIER_FILE_V1);
        let bytes = fs::read(&path).expect("barrier bytes");
        fs::remove_file(&path).expect("remove barrier");
        fs::write(&path, bytes).expect("replace barrier inode");
        fs::set_permissions(&path, fs::Permissions::from_mode(0o600))
            .expect("protect replacement barrier");
    });

    // The in-memory V1 tail is joined to exact held journal bytes, not merely
    // to parsed records retained before a same-inode rewrite or append.
    assert_final_place_revalidation_rejects_drift(|fixture| {
        rewrite_same_inode_same_length(&fixture.path(PM_TRIAL_LIVE_DISPATCH_FILE_V1));
    });
    assert_final_place_revalidation_rejects_drift(|fixture| {
        append_and_sync(&fixture.path(PM_TRIAL_LIVE_INTENT_FILE_V1), b"x");
    });

    let ledger_name = |fixture: &Fixture| {
        fixture
            .config
            .value()
            .journal
            .authorization_consumption_ledger_file
            .clone()
    };
    let claim_name = |fixture: &Fixture| {
        fixture
            .config
            .value()
            .journal
            .authorization_consumption_claim_file
            .clone()
    };

    // The move-only consumed-authorization owner retains and rechecks both
    // fixed evidence descriptors and exact bytes at the same boundary.
    assert_final_place_revalidation_rejects_drift(|fixture| {
        fs::remove_file(fixture.path(&claim_name(fixture))).expect("unlink atomic claim");
    });
    assert_final_place_revalidation_rejects_drift(|fixture| {
        rewrite_same_inode_same_length(&fixture.path(&claim_name(fixture)));
    });
    assert_final_place_revalidation_rejects_drift(|fixture| {
        append_and_sync(&fixture.path(&claim_name(fixture)), b"x");
    });
    assert_final_place_revalidation_rejects_drift(|fixture| {
        fs::remove_file(fixture.path(&ledger_name(fixture))).expect("unlink consumption ledger");
    });
    assert_final_place_revalidation_rejects_drift(|fixture| {
        rewrite_same_inode_same_length(&fixture.path(&ledger_name(fixture)));
    });
    assert_final_place_revalidation_rejects_drift(|fixture| {
        append_and_sync(&fixture.path(&ledger_name(fixture)), b"x");
    });

    // The transport conversion consumes the borrowed owner and repeats the
    // same checks immediately before producing the may-have-sent observation.
    assert_transport_conversion_rejects_drift(|fixture| {
        append_and_sync(
            &fixture.path(PM_PHASE_A_LIVE_DISPATCH_BARRIER_FILE_V1),
            b"x",
        );
    });
    assert_transport_conversion_rejects_drift(|fixture| {
        rewrite_same_inode_same_length(&fixture.path(PM_TRIAL_LIVE_DISPATCH_FILE_V1));
    });
    assert_transport_conversion_rejects_drift(|fixture| {
        fs::remove_file(fixture.path(&claim_name(fixture))).expect("unlink atomic claim");
    });
}

#[test]
fn lost_positive_barrier_after_possible_send_never_downgrades_to_no_place() {
    let (fixture, prepared_consumption) = Fixture::new();
    let mut journals = bind_preflight(&fixture);
    let owner = record_phase_a_live_dispatch(&fixture, prepared_consumption, &mut journals)
        .revalidate_for_runner()
        .expect("runner owner");
    let observation = journals
        .revalidate_phase_a_place_for_network_dispatch(owner)
        .expect("final network-boundary owner")
        .into_may_have_been_dispatched()
        .expect("possible-send observation");
    drop(observation);
    drop(journals);
    fs::remove_file(fixture.path(PM_PHASE_A_LIVE_DISPATCH_BARRIER_FILE_V1))
        .expect("model barrier loss after possible send");

    let projection = verify_controlled_trial_live_recovery(&fixture.config, &fixture.authorization)
        .expect("conservative recovery projection");
    assert_eq!(
        projection.classification(),
        &PmTrialLiveRecoveryClassificationV1::PlaceMayHaveBeenSentNoResend
    );
    assert!(!projection.phase_a_live_dispatch_barrier_durable());
    assert!(!projection.placement_resumption_allowed());
}

#[test]
fn outstanding_positive_owner_blocks_terminal_and_drops_fail_closed() {
    // Before final transport handoff, terminal is rejected and the combined
    // owner remains usable.
    {
        let (fixture, prepared_consumption) = Fixture::new();
        let mut journals = bind_preflight(&fixture);
        let owner = record_phase_a_live_dispatch(&fixture, prepared_consumption, &mut journals)
            .revalidate_for_runner()
            .expect("runner owner");
        assert!(matches!(
            journals.record_terminal(
                "2026-08-09T12:05:05Z".into(),
                PmIntentTerminalDispositionV1::OperatorActionRequired,
            ),
            Err(PmTrialLiveJournalError::InvalidTransition)
        ));
        let final_owner = journals
            .revalidate_phase_a_place_for_network_dispatch(owner)
            .expect("final network-boundary owner");
        drop(final_owner);
        assert!(matches!(
            journals.record_terminal(
                "2026-08-09T12:05:06Z".into(),
                PmIntentTerminalDispositionV1::OperatorActionRequired,
            ),
            Err(PmTrialLiveJournalError::InvalidTransition)
        ));
        drop(journals);
        assert_classification(
            &fixture,
            PmTrialLiveRecoveryClassificationV1::PlaceMayHaveBeenSentNoResend,
        );
    }

    // Dropping a may-have-sent observation likewise never creates a terminal
    // or resumable state.
    {
        let (fixture, prepared_consumption) = Fixture::new();
        let mut journals = bind_preflight(&fixture);
        let owner = record_phase_a_live_dispatch(&fixture, prepared_consumption, &mut journals)
            .revalidate_for_runner()
            .expect("runner owner");
        let observation = journals
            .revalidate_phase_a_place_for_network_dispatch(owner)
            .expect("final network-boundary owner")
            .into_may_have_been_dispatched()
            .expect("typed transport observation");
        drop(observation);
        assert!(matches!(
            journals.record_terminal(
                "2026-08-09T12:05:06Z".into(),
                PmIntentTerminalDispositionV1::OperatorActionRequired,
            ),
            Err(PmTrialLiveJournalError::InvalidTransition)
        ));
        drop(journals);
        assert_classification(
            &fixture,
            PmTrialLiveRecoveryClassificationV1::PlaceMayHaveBeenSentNoResend,
        );
    }
}

#[test]
fn positive_place_exchanges_reject_foreign_journal_epochs() {
    // Final transport handoff cannot join a revalidated owner to another live
    // journal, even when both have otherwise valid positive barriers.
    {
        let (fixture_a, consumption_a) = Fixture::new();
        let (fixture_b, consumption_b) = Fixture::new();
        let mut journals_a = bind_preflight(&fixture_a);
        let mut journals_b = bind_preflight(&fixture_b);
        let owner_a = record_phase_a_live_dispatch(&fixture_a, consumption_a, &mut journals_a)
            .revalidate_for_runner()
            .expect("runner owner A");
        let owner_b = record_phase_a_live_dispatch(&fixture_b, consumption_b, &mut journals_b);
        assert!(matches!(
            journals_b.revalidate_phase_a_place_for_network_dispatch(owner_a),
            Err(PmTrialLiveJournalError::ForeignAcknowledgement)
        ));
        drop(owner_b);
    }

    // Both no-send and may-have-sent result exchanges require the exact Arc
    // epoch owned by the originating journals.
    {
        let (fixture_a, consumption_a) = Fixture::new();
        let (fixture_b, consumption_b) = Fixture::new();
        let mut journals_a = bind_preflight(&fixture_a);
        let mut journals_b = bind_preflight(&fixture_b);
        let no_send = record_phase_a_live_dispatch(&fixture_a, consumption_a, &mut journals_a)
            .revalidate_for_runner()
            .expect("runner owner A")
            .into_definitely_not_dispatched();
        let owner_b = record_phase_a_live_dispatch(&fixture_b, consumption_b, &mut journals_b);
        assert!(matches!(
            journals_b.record_phase_a_place_definitely_not_dispatched(no_send),
            Err(PmTrialLiveJournalError::ForeignAcknowledgement)
        ));
        drop(owner_b);
    }

    {
        let (fixture_a, consumption_a) = Fixture::new();
        let (fixture_b, consumption_b) = Fixture::new();
        let mut journals_a = bind_preflight(&fixture_a);
        let mut journals_b = bind_preflight(&fixture_b);
        let owner_a = record_phase_a_live_dispatch(&fixture_a, consumption_a, &mut journals_a)
            .revalidate_for_runner()
            .expect("runner owner A");
        let observation = journals_a
            .revalidate_phase_a_place_for_network_dispatch(owner_a)
            .expect("final owner A")
            .into_may_have_been_dispatched()
            .expect("observation A");
        let owner_b = record_phase_a_live_dispatch(&fixture_b, consumption_b, &mut journals_b);
        assert!(matches!(
            journals_b.record_phase_a_place_live_result(
                observation,
                PmPlaceResultKindV1::AcknowledgementUnknown,
                None,
            ),
            Err(PmTrialLiveJournalError::ForeignAcknowledgement)
        ));
        drop(owner_b);
    }
}

#[test]
fn recovery_rejects_unsafe_terminal_even_when_positive_barrier_is_lost() {
    // Control: the hand-authored paired tails are canonical and accepted after
    // a sufficiently recent durable no-exposure reconciliation.
    {
        let (fixture, prepared_consumption) = Fixture::new();
        let mut journals = bind_preflight(&fixture);
        let (dispatch, consumed) =
            record_place_dispatch(&fixture, prepared_consumption, &mut journals);
        let result = journals
            .record_place_result(dispatch, PmPlaceResultKindV1::AcknowledgementUnknown, None)
            .expect("legacy unknown result");
        let (bridge, owned) = journals
            .record_place_outcome_bridge(result)
            .expect("legacy place bridge");
        assert!(owned.is_none());
        let (_reconciliation, owned) = journals
            .record_place_reconciliation(
                bridge,
                "2026-08-09T12:05:07Z".into(),
                PmReconciliationOrderStateV1::Absent,
                None,
            )
            .expect("durable no-exposure reconciliation");
        assert!(owned.is_none());
        drop(consumed);
        drop(journals);
        append_unsafe_positive_terminal_tails(&fixture);
        assert_classification(
            &fixture,
            PmTrialLiveRecoveryClassificationV1::TerminalEvidenceOnly,
        );
    }

    let (fixture, prepared_consumption) = Fixture::new();
    let mut journals = bind_preflight(&fixture);
    let owner = record_phase_a_live_dispatch(&fixture, prepared_consumption, &mut journals)
        .revalidate_for_runner()
        .expect("runner owner");
    let observation = journals
        .revalidate_phase_a_place_for_network_dispatch(owner)
        .expect("final network-boundary owner")
        .into_may_have_been_dispatched()
        .expect("possible-send observation");
    drop(observation);
    drop(journals);
    fs::remove_file(fixture.path(PM_PHASE_A_LIVE_DISPATCH_BARRIER_FILE_V1))
        .expect("lose positive barrier after possible send");

    // Model an older writer or hand-authored but otherwise canonical paired
    // Terminal tail before any no-exposure reconciliation.
    append_unsafe_positive_terminal_tails(&fixture);
    assert!(matches!(
        verify_controlled_trial_live_recovery(&fixture.config, &fixture.authorization),
        Err(PmTrialLiveJournalError::InvalidRecord)
    ));
}

#[test]
fn legacy_false_dispatch_cannot_terminalize_an_ambiguous_place() {
    let (fixture, prepared_consumption) = Fixture::new();
    let mut journals = bind_preflight(&fixture);
    let (dispatch, consumed) = record_place_dispatch(&fixture, prepared_consumption, &mut journals);
    drop(dispatch);
    assert!(matches!(
        journals.record_terminal(
            "2026-08-09T12:05:08Z".into(),
            PmIntentTerminalDispositionV1::OperatorActionRequired,
        ),
        Err(PmTrialLiveJournalError::InvalidTransition)
    ));
    drop(consumed);
    drop(journals);
    assert_classification(
        &fixture,
        PmTrialLiveRecoveryClassificationV1::PlaceMayHaveBeenSentNoResend,
    );
}

#[test]
fn held_grant_can_terminally_prove_definitely_not_dispatched() {
    let (fixture, prepared_consumption) = Fixture::new();
    let expected = fixture
        .config
        .exact_place_public_request_identity()
        .expected_order_id()
        .to_string();
    let mut journals = bind_preflight(&fixture);
    let owner = record_phase_a_live_dispatch(&fixture, prepared_consumption, &mut journals);
    let owner = owner
        .revalidate_for_runner()
        .expect("exact barrier revalidation before pre-send result");
    let no_send = owner.into_definitely_not_dispatched();
    let (result, consumed) = journals
        .record_phase_a_place_definitely_not_dispatched(no_send)
        .expect("terminal pre-send result");
    assert_eq!(
        result.outcome(),
        PmPlaceResultKindV1::DefinitelyNotDispatched
    );
    let (bridge, owned) = journals
        .record_phase_a_place_live_outcome_bridge(result)
        .expect("definitely-unsent bridge");
    assert!(owned.is_none());
    assert!(matches!(
        journals.record_phase_a_place_live_reconciliation(
            bridge,
            "2026-08-09T12:05:10Z".into(),
            PmReconciliationOrderStateV1::ExactLive,
            Some(expected),
        ),
        Err(PmTrialLiveJournalError::InvalidTransition)
    ));
    drop(consumed);
    drop(journals);

    let projection = verify_controlled_trial_live_recovery(&fixture.config, &fixture.authorization)
        .expect("definitely-unsent recovery projection");
    assert_eq!(
        projection.classification(),
        &PmTrialLiveRecoveryClassificationV1::AuthorizationBurnedNoPlace
    );
    assert!(projection.phase_a_live_dispatch_barrier_durable());
    assert!(!projection.placement_resumption_allowed());
}

#[test]
fn positive_transport_result_preserves_live_cancel_lineage_and_blocks_early_terminal() {
    let (fixture, prepared_consumption) = Fixture::new();
    let expected = fixture
        .config
        .exact_place_public_request_identity()
        .expected_order_id();
    let expected_text = expected.to_string();
    let mut journals = bind_preflight(&fixture);
    let owner = record_phase_a_live_dispatch(&fixture, prepared_consumption, &mut journals)
        .revalidate_for_runner()
        .expect("runner owner");
    let observation = journals
        .revalidate_phase_a_place_for_network_dispatch(owner)
        .expect("final network-boundary owner")
        .into_may_have_been_dispatched()
        .expect("typed transport observation");
    let (result, consumed) = journals
        .record_phase_a_place_live_result(
            observation,
            PmPlaceResultKindV1::Accepted,
            Some(expected_text.clone()),
        )
        .expect("positive accepted result");
    assert!(matches!(
        journals.record_terminal(
            "2026-08-09T12:05:05Z".into(),
            PmIntentTerminalDispositionV1::OperatorActionRequired,
        ),
        Err(PmTrialLiveJournalError::InvalidTransition)
    ));

    let (bridge, owned) = journals
        .record_phase_a_place_live_outcome_bridge(result)
        .expect("positive place bridge");
    let owned = owned.expect("positive live-owned venue order");
    assert_eq!(
        owned.exact_venue_order_id().expect("typed order ID"),
        FixedOrderId::from(expected)
    );
    assert!(matches!(
        journals.record_terminal(
            "2026-08-09T12:05:06Z".into(),
            PmIntentTerminalDispositionV1::OperatorActionRequired,
        ),
        Err(PmTrialLiveJournalError::InvalidTransition)
    ));

    let cancel_identity = owned.exact_venue_order_id().expect("typed order ID");
    let cancel_intent = journals
        .record_phase_a_live_primary_cancel_intent(bridge, owned, "2026-08-09T12:05:10Z".into())
        .expect("positive primary-cancel intent");
    let cancellation = PmCancelPreparationV1::for_scoped_plan(
        &fixture.config,
        journals.preflight_binding(),
        cancel_identity,
        l2_time("2026-08-09T12:05:11Z"),
    )
    .expect("exact positive cancellation");
    let prepared = journals
        .record_phase_a_live_cancel_prepared(cancel_intent, cancellation)
        .expect("positive cancel preparation");
    let cancel_dispatch = journals
        .record_phase_a_live_cancel_dispatch_authorized(prepared)
        .expect("positive cancel dispatch evidence");
    assert_eq!(
        cancel_dispatch.dispatch_class(),
        PmCancelDispatchClassV1::Primary
    );
    assert_eq!(
        cancel_dispatch.preparation().exact_venue_order_id(),
        FixedOrderId::from(expected)
    );
    assert!(!cancel_dispatch.preparation().mutation_authority());
    assert!(matches!(
        journals.record_terminal(
            "2026-08-09T12:05:12Z".into(),
            PmIntentTerminalDispositionV1::OperatorActionRequired,
        ),
        Err(PmTrialLiveJournalError::InvalidTransition)
    ));
    drop(cancel_dispatch);
    drop(consumed);
    drop(journals);

    assert_classification(
        &fixture,
        PmTrialLiveRecoveryClassificationV1::ReconcileBeforeRecoveryCancel {
            exact_venue_order_id: Some(expected_text),
        },
    );
}

#[test]
fn exact_accepted_order_reopens_only_as_recovery_cancel_and_terminal_is_evidence() {
    let (fixture, prepared_consumption) = Fixture::new();
    let expected = fixture
        .config
        .exact_place_public_request_identity()
        .expected_order_id();
    let expected_text = expected.to_string();
    let mut journals = bind_preflight(&fixture);
    let (dispatch, consumed) = record_place_dispatch(&fixture, prepared_consumption, &mut journals);
    let result = journals
        .record_place_result(
            dispatch,
            PmPlaceResultKindV1::Accepted,
            Some(expected_text.clone()),
        )
        .expect("accepted result");
    let (_bridge, _owned) = journals
        .record_place_outcome_bridge(result)
        .expect("place outcome bridge");
    drop(consumed);
    drop(journals);

    let projection = verify_controlled_trial_live_recovery(&fixture.config, &fixture.authorization)
        .expect("recovery projection");
    assert_eq!(
        projection.classification(),
        &PmTrialLiveRecoveryClassificationV1::RecoveryCancelOnly {
            exact_venue_order_id: expected_text.clone(),
        }
    );
    let mut recovery = PmControlledTrialLiveRecoveryJournals::open(
        &fixture.config,
        &fixture.authorization,
        projection,
    )
    .expect("cancel-only recovery owner");
    assert!(!recovery.production_order_entry_authorized());
    assert!(!recovery.real_order_submission_authorized());
    assert_eq!(recovery.place_dispatch_allowance(), 0);

    let (reconciliation, owned) = recovery
        .record_reconciliation(
            "2026-08-09T12:05:30Z".into(),
            PmReconciliationOrderStateV1::ExactLive,
            Some(expected_text.clone()),
        )
        .expect("exact-live reconciliation");
    let owned = owned.expect("journal-owned exact order");
    assert_eq!(
        owned.exact_venue_order_id().expect("typed order id"),
        FixedOrderId::from(expected)
    );
    let cancel_identity = owned.exact_venue_order_id().expect("typed order id");
    let cancel_intent = recovery
        .record_recovery_cancel_intent(reconciliation, owned, "2026-08-09T12:05:31Z".into())
        .expect("recovery cancel intent");
    let cancellation = PmCancelPreparationV1::for_scoped_plan(
        &fixture.config,
        recovery.preflight_binding(),
        cancel_identity,
        l2_time(CANCEL_TIME),
    )
    .expect("exact cancel preparation");
    let prepared = recovery
        .record_cancel_prepared(cancel_intent, cancellation)
        .expect("cancel prepared");
    let cancel_dispatch = recovery
        .record_cancel_dispatch_authorized(prepared)
        .expect("cancel dispatch evidence");
    assert_eq!(
        cancel_dispatch.preparation().dispatch_class(),
        PmCancelDispatchClassV1::Recovery { ordinal: 1 }
    );
    assert_eq!(
        cancel_dispatch.preparation().exact_venue_order_id(),
        FixedOrderId::from(expected)
    );
    assert!(!cancel_dispatch.preparation().mutation_authority());
    let canceled = recovery
        .record_cancel_result(cancel_dispatch, PmCancelResultKindV1::Canceled)
        .expect("cancel result");
    let bridge = recovery
        .record_cancel_outcome_bridge(canceled)
        .expect("cancel bridge");
    let (_reconciled, no_owned) = recovery
        .record_cancel_reconciliation(
            bridge,
            "2026-08-09T12:06:01Z".into(),
            PmReconciliationOrderStateV1::ExactCanceled,
            Some(expected_text),
        )
        .expect("cancel reconciliation");
    assert!(no_owned.is_none());
    let _terminal = recovery
        .record_terminal(
            "2026-08-09T12:06:02Z".into(),
            PmIntentTerminalDispositionV1::Completed,
        )
        .expect("terminal evidence");
    drop(recovery);

    assert_classification(
        &fixture,
        PmTrialLiveRecoveryClassificationV1::TerminalEvidenceOnly,
    );
}

#[test]
fn ambiguous_cancel_tail_requires_reconciliation_before_another_bounded_cancel() {
    let (fixture, prepared_consumption) = Fixture::new();
    let expected = fixture
        .config
        .exact_place_public_request_identity()
        .expected_order_id();
    let expected_text = expected.to_string();
    let mut journals = bind_preflight(&fixture);
    let (dispatch, consumed) = record_place_dispatch(&fixture, prepared_consumption, &mut journals);
    let result = journals
        .record_place_result(
            dispatch,
            PmPlaceResultKindV1::Accepted,
            Some(expected_text.clone()),
        )
        .expect("accepted result");
    let (bridge, owned) = journals
        .record_place_outcome_bridge(result)
        .expect("place bridge");
    let owned = owned.expect("owned order");
    let cancel_id = owned.exact_venue_order_id().expect("typed order id");
    let cancel_intent = journals
        .record_primary_cancel_intent(bridge, owned, "2026-08-09T12:05:10Z".into())
        .expect("primary cancel intent");
    let cancellation = PmCancelPreparationV1::for_scoped_plan(
        &fixture.config,
        journals.preflight_binding(),
        cancel_id,
        l2_time("2026-08-09T12:05:11Z"),
    )
    .expect("primary cancellation");
    let prepared = journals
        .record_cancel_prepared(cancel_intent, cancellation)
        .expect("cancel prepared");
    let _dispatch = journals
        .record_cancel_dispatch_authorized(prepared)
        .expect("cancel dispatch evidence");
    drop(consumed);
    drop(journals);

    assert_classification(
        &fixture,
        PmTrialLiveRecoveryClassificationV1::ReconcileBeforeRecoveryCancel {
            exact_venue_order_id: Some(expected_text),
        },
    );
}

#[test]
fn reconciliation_cannot_mint_cancel_custody_for_a_foreign_order_id() {
    let (fixture, prepared_consumption) = Fixture::new();
    let mut journals = bind_preflight(&fixture);
    let owner = record_phase_a_live_dispatch(&fixture, prepared_consumption, &mut journals);
    drop(owner);
    drop(journals);
    let projection = verify_controlled_trial_live_recovery(&fixture.config, &fixture.authorization)
        .expect("recovery projection");
    let mut recovery = PmControlledTrialLiveRecoveryJournals::open(
        &fixture.config,
        &fixture.authorization,
        projection,
    )
    .expect("recovery owner");
    assert!(matches!(
        recovery.record_reconciliation(
            "2026-08-09T12:05:30Z".into(),
            PmReconciliationOrderStateV1::ExactLive,
            Some(format!("0x{}", "aa".repeat(32))),
        ),
        Err(PmTrialLiveJournalError::InvalidTransition)
    ));
}

#[test]
fn place_preparation_rejects_prefixed_or_foreign_public_identity_fields() {
    let (fixture, _prepared_consumption) = Fixture::new();
    let mut journals = bind_preflight(&fixture);
    let intent = journals
        .record_place_intent(PLACE_TIME.into())
        .expect("place intent");
    let valid = place_preparation(&fixture, &journals);
    let mut foreign = serde_json::to_value(valid).expect("preparation value");
    foreign["expected_order_id"] = serde_json::Value::String(
        fixture
            .config
            .exact_place_public_request_identity()
            .expected_order_id()
            .to_string(),
    );
    let foreign = serde_json::from_value(foreign).expect("structural preparation");
    assert!(matches!(
        journals.record_place_prepared(intent, foreign),
        Err(PmTrialLiveJournalError::InvalidBinding)
    ));
}

#[test]
fn torn_unknown_and_extra_tails_are_rejected_without_place_authority() {
    let (fixture, _prepared_consumption) = Fixture::new();
    let journals = bind_preflight(&fixture);
    drop(journals);
    let intent = fixture.path(PM_TRIAL_LIVE_INTENT_FILE_V1);
    let mut bytes = fs::read(&intent).expect("intent bytes");
    bytes.extend_from_slice(b"{");
    fs::write(&intent, bytes).expect("torn tail");
    assert!(matches!(
        verify_controlled_trial_live_recovery(&fixture.config, &fixture.authorization),
        Err(PmTrialLiveJournalError::AmbiguousTail)
    ));

    let (fixture, _prepared_consumption) = Fixture::new();
    let journals = bind_preflight(&fixture);
    drop(journals);
    let dispatch = fixture.path(PM_TRIAL_LIVE_DISPATCH_FILE_V1);
    let mut bytes = fs::read(&dispatch).expect("dispatch bytes");
    bytes.extend_from_slice(b"{}\n");
    fs::write(&dispatch, bytes).expect("unknown tail");
    assert!(matches!(
        verify_controlled_trial_live_recovery(&fixture.config, &fixture.authorization),
        Err(PmTrialLiveJournalError::InvalidRecord)
    ));

    let (fixture, _prepared_consumption) = Fixture::new();
    let journals = bind_preflight(&fixture);
    drop(journals);
    let intent = fixture.path(PM_TRIAL_LIVE_INTENT_FILE_V1);
    let dispatch = fixture.path(PM_TRIAL_LIVE_DISPATCH_FILE_V1);
    let intent_bytes = fs::read(&intent).expect("intent bytes");
    let dispatch_bytes = fs::read(&dispatch).expect("dispatch bytes");
    fs::write(&intent, dispatch_bytes).expect("cross journal intent");
    fs::write(&dispatch, intent_bytes).expect("cross journal dispatch");
    assert!(matches!(
        verify_controlled_trial_live_recovery(&fixture.config, &fixture.authorization),
        Err(PmTrialLiveJournalError::InvalidRecord)
    ));
}
