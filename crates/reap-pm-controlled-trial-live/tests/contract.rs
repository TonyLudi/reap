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
    PM_PHASE_A_LIVE_CANCEL_RECOVERY_CONTINUATION_DISPATCH_FILE_V1,
    PM_PHASE_A_LIVE_CANCEL_RECOVERY_CONTINUATION_INTENT_FILE_V1,
    PM_PHASE_A_LIVE_DISPATCH_BARRIER_FILE_V1, PM_TRIAL_LIVE_DISPATCH_FILE_V1,
    PM_TRIAL_LIVE_INTENT_FILE_V1, PmCancelDispatchClassV1, PmCancelPreparationV1,
    PmCancelResultKindV1, PmControlledTrialLiveCancelRecoveryJournals,
    PmControlledTrialLiveJournals, PmControlledTrialLiveRecoveryJournals,
    PmIntentTerminalDispositionV1, PmPhaseALiveCancelRecoveryRequiredActionV1,
    PmPlacePreparationV1, PmPlaceResultKindV1, PmReconciliationOrderStateV1,
    PmTrialLiveJournalError, PmTrialLiveRecoveryClassificationV1,
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

fn record_accepted_place_custody(
    fixture: &Fixture,
    prepared_consumption: PreparedAuthorizationConsumption,
    journals: &mut PmControlledTrialLiveJournals,
) -> reap_pm_controlled_trial_live::PmPhaseAPlaceLiveOutcomeCustodyV1 {
    let expected = fixture
        .config
        .exact_place_public_request_identity()
        .expected_order_id()
        .to_string();
    let owner = record_phase_a_live_dispatch(fixture, prepared_consumption, journals)
        .revalidate_for_runner()
        .expect("runner place owner");
    let observation = journals
        .revalidate_phase_a_place_for_network_dispatch(owner)
        .expect("final place owner")
        .into_may_have_been_dispatched()
        .expect("place may-have-sent observation");
    let result = journals
        .record_phase_a_place_live_result_with_custody(
            observation,
            PmPlaceResultKindV1::Accepted,
            Some(expected),
        )
        .expect("accepted positive place result custody");
    journals
        .record_phase_a_place_live_outcome_bridge_with_custody(result)
        .expect("positive place bridge custody")
}

fn record_primary_cancel_owner(
    fixture: &Fixture,
    prepared_consumption: PreparedAuthorizationConsumption,
    journals: &mut PmControlledTrialLiveJournals,
) -> reap_pm_controlled_trial_live::PmPhaseALiveCancelDispatchOwnerV1 {
    let place = record_accepted_place_custody(fixture, prepared_consumption, journals);
    let cancel_identity = place
        .exact_live_order_id()
        .expect("valid exact order ID")
        .expect("Accepted order custody");
    let intent = journals
        .record_phase_a_live_primary_cancel_intent_with_custody(
            place,
            "2026-08-09T12:05:10Z".into(),
        )
        .expect("primary cancel intent custody");
    let cancellation = PmCancelPreparationV1::for_scoped_plan(
        &fixture.config,
        journals.preflight_binding(),
        cancel_identity,
        l2_time("2026-08-09T12:05:11Z"),
    )
    .expect("exact primary cancellation");
    let prepared = journals
        .record_phase_a_live_cancel_prepared_with_custody(intent, cancellation)
        .expect("primary cancel prepared custody");
    journals
        .record_phase_a_live_cancel_dispatch_authorized_with_custody(prepared)
        .expect("primary positive cancel owner")
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

fn assert_final_cancel_revalidation_rejects_drift(drift: impl FnOnce(&Fixture)) {
    let (fixture, prepared_consumption) = Fixture::new();
    let mut journals = bind_preflight(&fixture);
    let owner = record_primary_cancel_owner(&fixture, prepared_consumption, &mut journals);
    let owner = journals
        .revalidate_phase_a_live_cancel_for_runner(owner)
        .expect("runner-side cancel revalidation");
    drift(&fixture);
    assert!(matches!(
        journals.revalidate_phase_a_live_cancel_for_network_dispatch(owner),
        Err(PmTrialLiveJournalError::Protection)
    ));
}

fn assert_cancel_transport_conversion_rejects_drift(drift: impl FnOnce(&Fixture)) {
    let (fixture, prepared_consumption) = Fixture::new();
    let mut journals = bind_preflight(&fixture);
    let owner = record_primary_cancel_owner(&fixture, prepared_consumption, &mut journals);
    let owner = journals
        .revalidate_phase_a_live_cancel_for_runner(owner)
        .expect("runner-side cancel revalidation");
    let final_owner = journals
        .revalidate_phase_a_live_cancel_for_network_dispatch(owner)
        .expect("final journal-bound cancel owner");
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

fn terminal_dnd_fixture() -> (Fixture, String) {
    let (fixture, prepared_consumption) = Fixture::new();
    let expected = fixture
        .config
        .exact_place_public_request_identity()
        .expected_order_id()
        .to_string();
    let mut journals = bind_preflight(&fixture);
    let no_send = record_phase_a_live_dispatch(&fixture, prepared_consumption, &mut journals)
        .revalidate_for_runner()
        .expect("runner owner")
        .into_definitely_not_dispatched();
    let (_result, consumed) = journals
        .record_phase_a_place_definitely_not_dispatched(no_send)
        .expect("durable DND result");
    journals
        .record_terminal(
            "2026-08-09T12:05:10Z".into(),
            PmIntentTerminalDispositionV1::Stopped,
        )
        .expect("terminal while positive barrier is intact");
    drop(consumed);
    drop(journals);
    (fixture, expected)
}

fn continuation_terminal_plan_fixture() -> (Fixture, usize, usize) {
    let (fixture, _) = terminal_dnd_fixture();
    fs::remove_file(fixture.path(PM_PHASE_A_LIVE_DISPATCH_BARRIER_FILE_V1))
        .expect("remove barrier");
    let projection = verify_controlled_trial_live_recovery(&fixture.config, &fixture.authorization)
        .expect("continuation basis");
    let mut recovery = PmControlledTrialLiveCancelRecoveryJournals::open_phase_a_live_cancel(
        &fixture.config,
        &fixture.authorization,
        &fixture.runtime("2026-08-09T12:05:30Z"),
        projection,
    )
    .expect("continuation owner");
    recovery
        .record_phase_a_live_reconciliation_with_custody(
            "2026-08-09T12:05:31Z".into(),
            PmReconciliationOrderStateV1::Absent,
            None,
        )
        .expect("safe reconciliation");
    let intent = fixture.path(PM_PHASE_A_LIVE_CANCEL_RECOVERY_CONTINUATION_INTENT_FILE_V1);
    let dispatch = fixture.path(PM_PHASE_A_LIVE_CANCEL_RECOVERY_CONTINUATION_DISPATCH_FILE_V1);
    let intent_predecessor_count = fs::read(&intent)
        .expect("intent predecessor")
        .iter()
        .filter(|byte| **byte == b'\n')
        .count();
    let dispatch_predecessor_count = fs::read(&dispatch)
        .expect("dispatch predecessor")
        .iter()
        .filter(|byte| **byte == b'\n')
        .count();
    recovery
        .record_terminal(
            "2026-08-09T12:05:32Z".into(),
            PmIntentTerminalDispositionV1::Completed,
        )
        .expect("ledger-first Terminal plan and pair");
    drop(recovery);
    (
        fixture,
        intent_predecessor_count,
        dispatch_predecessor_count,
    )
}

fn recovery_continuation_prepared_fixture() -> (Fixture, String) {
    let (fixture, expected) = terminal_dnd_fixture();
    fs::remove_file(fixture.path(PM_PHASE_A_LIVE_DISPATCH_BARRIER_FILE_V1))
        .expect("remove barrier");
    let projection = verify_controlled_trial_live_recovery(&fixture.config, &fixture.authorization)
        .expect("continuation basis");
    let mut recovery = PmControlledTrialLiveCancelRecoveryJournals::open_phase_a_live_cancel(
        &fixture.config,
        &fixture.authorization,
        &fixture.runtime("2026-08-09T12:05:30Z"),
        projection,
    )
    .expect("continuation owner");
    let reconciliation = recovery
        .record_phase_a_live_reconciliation_with_custody(
            "2026-08-09T12:05:31Z".into(),
            PmReconciliationOrderStateV1::ExactLive,
            Some(expected.clone()),
        )
        .expect("exact live");
    let intent = recovery
        .record_phase_a_live_recovery_cancel_intent_with_custody(
            reconciliation,
            "2026-08-09T12:05:32Z".into(),
        )
        .expect("recovery intent");
    let preparation = PmCancelPreparationV1::for_scoped_plan(
        &fixture.config,
        recovery.preflight_binding(),
        FixedOrderId::parse(&expected).expect("fixed order ID"),
        l2_time("2026-08-09T12:05:33Z"),
    )
    .expect("recovery preparation");
    let prepared = recovery
        .record_phase_a_live_cancel_prepared_with_custody(intent, preparation)
        .expect("prepared and independently anchored");
    drop(prepared);
    drop(recovery);
    (fixture, expected)
}

fn recovery_continuation_two_prepared_fixture() -> (Fixture, String) {
    let (fixture, expected) = recovery_continuation_prepared_fixture();
    let projection = verify_controlled_trial_live_recovery(&fixture.config, &fixture.authorization)
        .expect("first prepared projection");
    let mut recovery = PmControlledTrialLiveCancelRecoveryJournals::open_phase_a_live_cancel(
        &fixture.config,
        &fixture.authorization,
        &fixture.runtime("2026-08-09T12:05:40Z"),
        projection,
    )
    .expect("second cycle owner");
    let reconciliation = recovery
        .record_phase_a_live_reconciliation_with_custody(
            "2026-08-09T12:05:41Z".into(),
            PmReconciliationOrderStateV1::ExactLive,
            Some(expected.clone()),
        )
        .expect("second exact live");
    let intent = recovery
        .record_phase_a_live_recovery_cancel_intent_with_custody(
            reconciliation,
            "2026-08-09T12:05:42Z".into(),
        )
        .expect("second recovery intent");
    let preparation = PmCancelPreparationV1::for_scoped_plan(
        &fixture.config,
        recovery.preflight_binding(),
        FixedOrderId::parse(&expected).expect("fixed order ID"),
        l2_time("2026-08-09T12:05:43Z"),
    )
    .expect("second recovery preparation");
    let prepared = recovery
        .record_phase_a_live_cancel_prepared_with_custody(intent, preparation)
        .expect("second prepared and independently anchored");
    drop(prepared);
    drop(recovery);
    (fixture, expected)
}

fn barrier_corruptions() -> [fn(&Fixture); 5] {
    [
        |fixture| {
            fs::remove_file(fixture.path(PM_PHASE_A_LIVE_DISPATCH_BARRIER_FILE_V1))
                .expect("unlink barrier");
        },
        |fixture| {
            fs::write(fixture.path(PM_PHASE_A_LIVE_DISPATCH_BARRIER_FILE_V1), [])
                .expect("zero barrier");
        },
        |fixture| {
            let path = fixture.path(PM_PHASE_A_LIVE_DISPATCH_BARRIER_FILE_V1);
            let bytes = fs::read(&path).expect("barrier bytes");
            fs::write(path, &bytes[..bytes.len() / 2]).expect("torn barrier");
        },
        |fixture| {
            rewrite_same_inode_same_length(&fixture.path(PM_PHASE_A_LIVE_DISPATCH_BARRIER_FILE_V1));
        },
        |fixture| {
            let path = fixture.path(PM_PHASE_A_LIVE_DISPATCH_BARRIER_FILE_V1);
            let mut bytes = fs::read(&path).expect("barrier bytes");
            bytes[0] = b'[';
            fs::remove_file(&path).expect("remove barrier inode");
            fs::write(&path, bytes).expect("replacement barrier inode");
            fs::set_permissions(&path, fs::Permissions::from_mode(0o600))
                .expect("protect replacement barrier");
        },
    ]
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
        let projection =
            verify_controlled_trial_live_recovery(&fixture.config, &fixture.authorization)
                .expect("unusable barrier is revoked, not trusted");
        assert_eq!(
            projection.classification(),
            &PmTrialLiveRecoveryClassificationV1::PlaceMayHaveBeenSentNoResend
        );
        assert!(!projection.phase_a_live_dispatch_barrier_durable());
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
        let projection =
            verify_controlled_trial_live_recovery(&fixture.config, &fixture.authorization)
                .expect("torn barrier cannot suppress cleanup");
        assert_eq!(
            projection.classification(),
            &PmTrialLiveRecoveryClassificationV1::PlaceMayHaveBeenSentNoResend
        );
        assert!(!projection.phase_a_live_dispatch_barrier_durable());
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
fn cancel_network_boundaries_recheck_journals_barrier_ledger_and_claim() {
    assert_final_cancel_revalidation_rejects_drift(|fixture| {
        rewrite_same_inode_same_length(&fixture.path(PM_TRIAL_LIVE_INTENT_FILE_V1));
    });
    assert_final_cancel_revalidation_rejects_drift(|fixture| {
        append_and_sync(&fixture.path(PM_TRIAL_LIVE_DISPATCH_FILE_V1), b"x");
    });
    assert_final_cancel_revalidation_rejects_drift(|fixture| {
        fs::remove_file(fixture.path(PM_PHASE_A_LIVE_DISPATCH_BARRIER_FILE_V1))
            .expect("unlink held positive barrier");
    });
    assert_final_cancel_revalidation_rejects_drift(|fixture| {
        let ledger = fixture.path(
            &fixture
                .config
                .value()
                .journal
                .authorization_consumption_ledger_file,
        );
        rewrite_same_inode_same_length(&ledger);
    });
    assert_final_cancel_revalidation_rejects_drift(|fixture| {
        let claim = fixture.path(
            &fixture
                .config
                .value()
                .journal
                .authorization_consumption_claim_file,
        );
        fs::remove_file(claim).expect("unlink held claim");
    });
    assert_cancel_transport_conversion_rejects_drift(|fixture| {
        append_and_sync(&fixture.path(PM_TRIAL_LIVE_INTENT_FILE_V1), b"x");
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
fn positive_cancel_custody_is_linear_through_dnd_reconciliation_and_recovery_cancel() {
    let (fixture, prepared_consumption) = Fixture::new();
    let expected = fixture
        .config
        .exact_place_public_request_identity()
        .expected_order_id();
    let expected_text = expected.to_string();
    let mut journals = bind_preflight(&fixture);
    let owner = record_primary_cancel_owner(&fixture, prepared_consumption, &mut journals);
    assert_eq!(owner.dispatch_class(), PmCancelDispatchClassV1::Primary);
    assert_eq!(owner.exact_venue_order_id(), FixedOrderId::from(expected));
    assert_eq!(
        owner.semantic_request_commitment(),
        owner.preparation().semantic_request_commitment()
    );
    assert_eq!(
        owner.l2_timestamp_seconds(),
        l2_time("2026-08-09T12:05:11Z")
    );
    assert!(matches!(
        journals.record_terminal(
            "2026-08-09T12:05:12Z".into(),
            PmIntentTerminalDispositionV1::OperatorActionRequired,
        ),
        Err(PmTrialLiveJournalError::InvalidTransition)
    ));

    let revalidated = journals
        .revalidate_phase_a_live_cancel_for_runner(owner)
        .expect("runner cancel owner");
    let no_send = journals
        .revalidate_phase_a_live_cancel_for_network_dispatch(revalidated)
        .expect("final cancel owner")
        .into_definitely_not_dispatched()
        .expect("proven no-send cancel token");
    let result = journals
        .record_phase_a_live_cancel_definitely_not_dispatched(no_send)
        .expect("durable cancel DND result");
    assert_eq!(
        result.outcome(),
        PmCancelResultKindV1::DefinitelyNotDispatched
    );
    assert!(matches!(
        journals.record_terminal(
            "2026-08-09T12:05:13Z".into(),
            PmIntentTerminalDispositionV1::OperatorActionRequired,
        ),
        Err(PmTrialLiveJournalError::InvalidTransition)
    ));
    let bridge = journals
        .record_phase_a_live_cancel_outcome_bridge_with_custody(result)
        .expect("positive cancel bridge");
    let reconciliation = journals
        .record_phase_a_live_cancel_reconciliation_with_custody(
            bridge,
            "2026-08-09T12:05:14Z".into(),
            PmReconciliationOrderStateV1::ExactLive,
            Some(expected_text.clone()),
        )
        .expect("exact live after proven no-send cancel");
    let intent = journals
        .record_phase_a_live_recovery_cancel_intent_with_custody(
            reconciliation,
            "2026-08-09T12:05:15Z".into(),
        )
        .expect("next exact recovery cancel intent");
    let cancellation = PmCancelPreparationV1::for_scoped_plan(
        &fixture.config,
        journals.preflight_binding(),
        FixedOrderId::from(expected),
        l2_time("2026-08-09T12:05:16Z"),
    )
    .expect("recovery cancellation");
    let prepared = journals
        .record_phase_a_live_cancel_prepared_with_custody(intent, cancellation)
        .expect("recovery cancel prepared");
    let owner = journals
        .record_phase_a_live_cancel_dispatch_authorized_with_custody(prepared)
        .expect("recovery cancel owner");
    assert_eq!(
        owner.dispatch_class(),
        PmCancelDispatchClassV1::Recovery { ordinal: 1 }
    );
    let revalidated = journals
        .revalidate_phase_a_live_cancel_for_runner(owner)
        .expect("runner recovery cancel owner");
    let observation = journals
        .revalidate_phase_a_live_cancel_for_network_dispatch(revalidated)
        .expect("final recovery cancel owner")
        .into_may_have_been_dispatched()
        .expect("cancel may-have-sent observation");
    let result = journals
        .record_phase_a_live_cancel_result(observation, PmCancelResultKindV1::Canceled)
        .expect("positive canceled result");
    let bridge = journals
        .record_phase_a_live_cancel_outcome_bridge_with_custody(result)
        .expect("canceled bridge");
    let resolved = journals
        .record_phase_a_live_cancel_reconciliation_with_custody(
            bridge,
            "2026-08-09T12:05:17Z".into(),
            PmReconciliationOrderStateV1::ExactCanceled,
            Some(expected_text),
        )
        .expect("exact canceled reconciliation");
    assert_eq!(
        resolved.state(),
        PmReconciliationOrderStateV1::ExactCanceled
    );
    let _terminal = journals
        .record_terminal(
            "2026-08-09T12:05:18Z".into(),
            PmIntentTerminalDispositionV1::Completed,
        )
        .expect("terminal after exact canceled reconciliation");
}

#[test]
fn dropping_positive_cancel_owner_keeps_terminal_blocked_and_recovery_never_resends() {
    let (fixture, prepared_consumption) = Fixture::new();
    let expected = fixture
        .config
        .exact_place_public_request_identity()
        .expected_order_id()
        .to_string();
    let mut journals = bind_preflight(&fixture);
    let owner = record_primary_cancel_owner(&fixture, prepared_consumption, &mut journals);
    drop(owner);
    assert!(matches!(
        journals.record_terminal(
            "2026-08-09T12:05:12Z".into(),
            PmIntentTerminalDispositionV1::OperatorActionRequired,
        ),
        Err(PmTrialLiveJournalError::InvalidTransition)
    ));
    drop(journals);
    assert_classification(
        &fixture,
        PmTrialLiveRecoveryClassificationV1::ReconcileBeforeRecoveryCancel {
            exact_venue_order_id: Some(expected),
        },
    );
}

#[test]
fn live_recovery_cancel_custody_survives_every_unusable_barrier_state() {
    let corruptions: [fn(&Fixture); 5] = [
        |fixture| {
            fs::remove_file(fixture.path(PM_PHASE_A_LIVE_DISPATCH_BARRIER_FILE_V1))
                .expect("unlink barrier");
        },
        |fixture| {
            fs::write(fixture.path(PM_PHASE_A_LIVE_DISPATCH_BARRIER_FILE_V1), [])
                .expect("zero barrier");
        },
        |fixture| {
            let path = fixture.path(PM_PHASE_A_LIVE_DISPATCH_BARRIER_FILE_V1);
            let mut bytes = fs::read(&path).expect("barrier bytes");
            let index = bytes
                .iter()
                .position(|byte| *byte == b'{')
                .expect("JSON prefix");
            bytes[index] = b'[';
            fs::write(path, &bytes[..bytes.len() / 2]).expect("torn barrier");
        },
        |fixture| {
            rewrite_same_inode_same_length(&fixture.path(PM_PHASE_A_LIVE_DISPATCH_BARRIER_FILE_V1));
        },
        |fixture| {
            let path = fixture.path(PM_PHASE_A_LIVE_DISPATCH_BARRIER_FILE_V1);
            let mut bytes = fs::read(&path).expect("barrier bytes");
            let index = bytes
                .iter()
                .position(|byte| *byte == b'{')
                .expect("JSON prefix");
            bytes[index] = b'[';
            fs::remove_file(&path).expect("remove barrier inode");
            fs::write(&path, bytes).expect("replacement barrier inode");
            fs::set_permissions(&path, fs::Permissions::from_mode(0o600))
                .expect("protect replacement barrier");
        },
    ];
    for corrupt in corruptions {
        let (fixture, prepared_consumption) = Fixture::new();
        let expected = fixture
            .config
            .exact_place_public_request_identity()
            .expected_order_id();
        let expected_text = expected.to_string();
        let mut journals = bind_preflight(&fixture);
        let owner = record_phase_a_live_dispatch(&fixture, prepared_consumption, &mut journals);
        drop(owner);
        drop(journals);
        corrupt(&fixture);

        let projection =
            verify_controlled_trial_live_recovery(&fixture.config, &fixture.authorization)
                .expect("unusable barrier remains conservative recovery evidence");
        assert_eq!(
            projection.classification(),
            &PmTrialLiveRecoveryClassificationV1::PlaceMayHaveBeenSentNoResend
        );
        assert!(!projection.phase_a_live_dispatch_barrier_durable());
        let mut recovery = PmControlledTrialLiveCancelRecoveryJournals::open_phase_a_live_cancel(
            &fixture.config,
            &fixture.authorization,
            &fixture.runtime("2026-08-09T12:05:30Z"),
            projection,
        )
        .expect("live recovery-only cancel custody");
        assert!(!recovery.production_order_entry_authorized());
        assert!(!recovery.real_order_submission_authorized());
        assert_eq!(recovery.place_dispatch_allowance(), 0);
        let reconciliation = recovery
            .record_phase_a_live_reconciliation_with_custody(
                "2026-08-09T12:05:31Z".into(),
                PmReconciliationOrderStateV1::ExactLive,
                Some(expected_text),
            )
            .expect("exact expected order is live");
        let intent = recovery
            .record_phase_a_live_recovery_cancel_intent_with_custody(
                reconciliation,
                "2026-08-09T12:05:32Z".into(),
            )
            .expect("recovery cancel intent from burned custody");
        let cancellation = PmCancelPreparationV1::for_scoped_plan(
            &fixture.config,
            recovery.preflight_binding(),
            FixedOrderId::from(expected),
            l2_time(CANCEL_TIME),
        )
        .expect("recovery cancel preparation");
        let prepared = recovery
            .record_phase_a_live_cancel_prepared_with_custody(intent, cancellation)
            .expect("recovery cancel prepared custody");
        let owner = recovery
            .record_phase_a_live_cancel_dispatch_authorized_with_custody(prepared)
            .expect("recovery cancel owner despite unusable barrier");
        assert_eq!(
            owner.dispatch_class(),
            PmCancelDispatchClassV1::Recovery { ordinal: 1 }
        );
        drop(owner);
    }
}

#[test]
fn live_recovery_cancel_constructor_rejects_foreign_runtime_and_late_cleanup_time() {
    for mutate in [
        |runtime: &mut reap_pm_controlled_trial::AuthorizationRuntimeBinding| {
            runtime.host.boot_identity = "foreign-boot".into();
        },
        |runtime: &mut reap_pm_controlled_trial::AuthorizationRuntimeBinding| {
            runtime.observed_at_utc = "2026-08-09T12:20:01Z".into();
        },
    ] {
        let (fixture, prepared_consumption) = Fixture::new();
        let mut journals = bind_preflight(&fixture);
        let owner = record_phase_a_live_dispatch(&fixture, prepared_consumption, &mut journals);
        drop(owner);
        drop(journals);
        let projection =
            verify_controlled_trial_live_recovery(&fixture.config, &fixture.authorization)
                .expect("recovery projection");
        let mut runtime = fixture.runtime("2026-08-09T12:05:30Z");
        mutate(&mut runtime);
        assert!(matches!(
            PmControlledTrialLiveCancelRecoveryJournals::open_phase_a_live_cancel(
                &fixture.config,
                &fixture.authorization,
                &runtime,
                projection,
            ),
            Err(PmTrialLiveJournalError::InvalidBinding)
        ));
    }
}

#[test]
fn unusable_barrier_revokes_place_dnd_and_terminal_trust() {
    // DND without its positive barrier is conservatively treated as a
    // possible send that requires reconciliation.
    {
        let (fixture, prepared_consumption) = Fixture::new();
        let mut journals = bind_preflight(&fixture);
        let no_send = record_phase_a_live_dispatch(&fixture, prepared_consumption, &mut journals)
            .revalidate_for_runner()
            .expect("runner owner")
            .into_definitely_not_dispatched();
        let (_result, consumed) = journals
            .record_phase_a_place_definitely_not_dispatched(no_send)
            .expect("durable DND result");
        drop(consumed);
        drop(journals);
        fs::remove_file(fixture.path(PM_PHASE_A_LIVE_DISPATCH_BARRIER_FILE_V1))
            .expect("unlink DND barrier");
        assert_classification(
            &fixture,
            PmTrialLiveRecoveryClassificationV1::PlaceMayHaveBeenSentNoResend,
        );
    }

    // A terminal tail written while the barrier was intact becomes a distinct
    // recovery-only continuation root if that prerequisite later disappears;
    // it cannot suppress exact cleanup or revive placement.
    {
        let (fixture, prepared_consumption) = Fixture::new();
        let mut journals = bind_preflight(&fixture);
        let no_send = record_phase_a_live_dispatch(&fixture, prepared_consumption, &mut journals)
            .revalidate_for_runner()
            .expect("runner owner")
            .into_definitely_not_dispatched();
        let (_result, consumed) = journals
            .record_phase_a_place_definitely_not_dispatched(no_send)
            .expect("durable DND result");
        let _terminal = journals
            .record_terminal(
                "2026-08-09T12:05:10Z".into(),
                PmIntentTerminalDispositionV1::Stopped,
            )
            .expect("terminal while DND barrier is intact");
        drop(consumed);
        drop(journals);
        fs::remove_file(fixture.path(PM_PHASE_A_LIVE_DISPATCH_BARRIER_FILE_V1))
            .expect("unlink terminal prerequisite barrier");
        let projection =
            verify_controlled_trial_live_recovery(&fixture.config, &fixture.authorization)
                .expect("recovery-only continuation projection");
        assert_eq!(
            projection.classification(),
            &PmTrialLiveRecoveryClassificationV1::ReconcileBeforeRecoveryCancel {
                exact_venue_order_id: Some(
                    fixture
                        .config
                        .exact_place_public_request_identity()
                        .expected_order_id()
                        .to_string(),
                ),
            }
        );
        assert_eq!(
            projection.phase_a_live_cancel_recovery_required_action(),
            Some(&PmPhaseALiveCancelRecoveryRequiredActionV1::ReconcileCurrentExposure)
        );
    }
}

#[test]
fn dnd_terminal_barrier_loss_enters_closed_recovery_continuation() {
    let (intact, _) = terminal_dnd_fixture();
    let intact_projection =
        verify_controlled_trial_live_recovery(&intact.config, &intact.authorization)
            .expect("intact terminal projection");
    assert_eq!(
        intact_projection.classification(),
        &PmTrialLiveRecoveryClassificationV1::TerminalEvidenceOnly
    );
    assert_eq!(
        intact_projection.phase_a_live_cancel_recovery_required_action(),
        None
    );
    assert!(
        !intact
            .path(PM_PHASE_A_LIVE_CANCEL_RECOVERY_CONTINUATION_INTENT_FILE_V1)
            .exists()
    );

    for (index, corrupt) in barrier_corruptions().into_iter().enumerate() {
        let (fixture, expected) = terminal_dnd_fixture();
        corrupt(&fixture);
        let projection =
            verify_controlled_trial_live_recovery(&fixture.config, &fixture.authorization)
                .expect("unusable barrier continuation projection");
        assert_eq!(
            projection.classification(),
            &PmTrialLiveRecoveryClassificationV1::ReconcileBeforeRecoveryCancel {
                exact_venue_order_id: Some(expected.clone()),
            }
        );
        assert_eq!(
            projection.phase_a_live_cancel_recovery_required_action(),
            Some(&PmPhaseALiveCancelRecoveryRequiredActionV1::ReconcileCurrentExposure)
        );
        assert!(!projection.placement_resumption_allowed());
        let mut recovery = PmControlledTrialLiveCancelRecoveryJournals::open_phase_a_live_cancel(
            &fixture.config,
            &fixture.authorization,
            &fixture.runtime("2026-08-09T12:05:30Z"),
            projection,
        )
        .expect("recovery continuation owner");
        assert_eq!(
            recovery.required_action(),
            Some(&PmPhaseALiveCancelRecoveryRequiredActionV1::ReconcileCurrentExposure)
        );
        if index == 0 {
            let _absent = recovery
                .record_phase_a_live_reconciliation_with_custody(
                    "2026-08-09T12:05:31Z".into(),
                    PmReconciliationOrderStateV1::Absent,
                    None,
                )
                .expect("exact absence");
            recovery
                .record_terminal(
                    "2026-08-09T12:05:32Z".into(),
                    PmIntentTerminalDispositionV1::Completed,
                )
                .expect("continuation terminal");
            drop(recovery);
            let terminal =
                verify_controlled_trial_live_recovery(&fixture.config, &fixture.authorization)
                    .expect("completed continuation");
            assert_eq!(
                terminal.phase_a_live_cancel_recovery_required_action(),
                Some(&PmPhaseALiveCancelRecoveryRequiredActionV1::TerminalEvidenceOnly)
            );
        } else {
            let _live = recovery
                .record_phase_a_live_reconciliation_with_custody(
                    "2026-08-09T12:05:31Z".into(),
                    PmReconciliationOrderStateV1::ExactLive,
                    Some(expected),
                )
                .expect("exact live cleanup custody");
        }
    }
}

#[test]
fn surviving_root_anchor_rejects_continuation_pair_rollback() {
    let prefixes: [fn(&Path, &Path); 4] = [
        |intent, dispatch| {
            fs::remove_file(intent).expect("remove intent");
            fs::remove_file(dispatch).expect("remove dispatch");
        },
        |intent, dispatch| {
            fs::write(intent, []).expect("zero intent");
            fs::remove_file(dispatch).expect("remove dispatch");
        },
        |_intent, dispatch| {
            fs::remove_file(dispatch).expect("remove dispatch");
        },
        |_intent, dispatch| {
            fs::write(dispatch, []).expect("zero dispatch");
        },
    ];
    for mutate in prefixes {
        let (fixture, _) = terminal_dnd_fixture();
        fs::remove_file(fixture.path(PM_PHASE_A_LIVE_DISPATCH_BARRIER_FILE_V1))
            .expect("remove barrier");
        let projection =
            verify_controlled_trial_live_recovery(&fixture.config, &fixture.authorization)
                .expect("continuation basis");
        let recovery = PmControlledTrialLiveCancelRecoveryJournals::open_phase_a_live_cancel(
            &fixture.config,
            &fixture.authorization,
            &fixture.runtime("2026-08-09T12:05:30Z"),
            projection,
        )
        .expect("create complete continuation headers");
        drop(recovery);
        let intent = fixture.path(PM_PHASE_A_LIVE_CANCEL_RECOVERY_CONTINUATION_INTENT_FILE_V1);
        let dispatch = fixture.path(PM_PHASE_A_LIVE_CANCEL_RECOVERY_CONTINUATION_DISPATCH_FILE_V1);
        mutate(&intent, &dispatch);

        assert!(
            verify_controlled_trial_live_recovery(&fixture.config, &fixture.authorization).is_err(),
            "a surviving monotonic root anchor makes pair loss/reset fail closed"
        );
    }
}

#[test]
fn surviving_prepared_anchor_rejects_pair_loss_and_header_reset() {
    for reset in [0_u8, 1] {
        let (fixture, _) = recovery_continuation_prepared_fixture();
        let intent = fixture.path(PM_PHASE_A_LIVE_CANCEL_RECOVERY_CONTINUATION_INTENT_FILE_V1);
        let dispatch = fixture.path(PM_PHASE_A_LIVE_CANCEL_RECOVERY_CONTINUATION_DISPATCH_FILE_V1);
        match reset {
            0 => {
                fs::remove_file(intent).expect("remove anchored intent journal");
                fs::remove_file(dispatch).expect("remove anchored dispatch journal");
            }
            1 => {
                retain_complete_lines(&intent, 1);
                retain_complete_lines(&dispatch, 1);
            }
            _ => unreachable!(),
        }
        assert!(
            verify_controlled_trial_live_recovery(&fixture.config, &fixture.authorization).is_err(),
            "surviving prepared registry evidence forbids pair rollback"
        );
    }
}

#[test]
fn anchor_ahead_by_one_reconstructs_the_exact_prepared_record() {
    let (fixture, _) = recovery_continuation_prepared_fixture();
    let dispatch = fixture.path(PM_PHASE_A_LIVE_CANCEL_RECOVERY_CONTINUATION_DISPATCH_FILE_V1);
    retain_complete_lines(&dispatch, 1);
    let projection = verify_controlled_trial_live_recovery(&fixture.config, &fixture.authorization)
        .expect("ledger-first Prepared prefix is source-completable");
    let recovery = PmControlledTrialLiveCancelRecoveryJournals::open_phase_a_live_cancel(
        &fixture.config,
        &fixture.authorization,
        &fixture.runtime("2026-08-09T12:05:40Z"),
        projection,
    )
    .expect("append exact canonical Prepared from its surviving anchor");
    drop(recovery);
    assert_eq!(
        fs::read(&dispatch)
            .expect("reconstructed dispatch journal")
            .iter()
            .filter(|byte| **byte == b'\n')
            .count(),
        2
    );
    let projection = verify_controlled_trial_live_recovery(&fixture.config, &fixture.authorization)
        .expect("reconstructed pair verifies idempotently");
    let reopened = PmControlledTrialLiveCancelRecoveryJournals::open_phase_a_live_cancel(
        &fixture.config,
        &fixture.authorization,
        &fixture.runtime("2026-08-09T12:05:41Z"),
        projection,
    )
    .expect("reconstructed pair reopens without another append");
    drop(reopened);
    assert_eq!(
        fs::read(dispatch)
            .expect("idempotently reopened dispatch journal")
            .iter()
            .filter(|byte| **byte == b'\n')
            .count(),
        2
    );
}

#[test]
fn anchor_ahead_requires_its_exact_latest_cancel_intent() {
    let (fixture, _) = recovery_continuation_prepared_fixture();
    let intent = fixture.path(PM_PHASE_A_LIVE_CANCEL_RECOVERY_CONTINUATION_INTENT_FILE_V1);
    let dispatch = fixture.path(PM_PHASE_A_LIVE_CANCEL_RECOVERY_CONTINUATION_DISPATCH_FILE_V1);
    retain_complete_lines(&intent, 2);
    retain_complete_lines(&dispatch, 1);
    assert!(
        verify_controlled_trial_live_recovery(&fixture.config, &fixture.authorization).is_err(),
        "the registry cannot reconstruct Prepared onto a missing or foreign intent predecessor"
    );
}

#[test]
fn prepared_pair_ahead_of_the_monotonic_registry_is_rejected() {
    let (fixture, _) = recovery_continuation_prepared_fixture();
    let ledger = fixture.path(
        &fixture
            .config
            .value()
            .journal
            .authorization_consumption_ledger_file,
    );
    retain_complete_lines(&ledger, 3);
    assert!(
        verify_controlled_trial_live_recovery(&fixture.config, &fixture.authorization).is_err(),
        "a durable Prepared without its ledger-first ordinal anchor is never resumable"
    );
}

#[test]
fn registry_rejects_pair_ahead_by_two_and_anchor_tamper_or_reorder() {
    for mutation in [0_u8, 1, 2] {
        let (fixture, _) = recovery_continuation_two_prepared_fixture();
        let ledger = fixture.path(
            &fixture
                .config
                .value()
                .journal
                .authorization_consumption_ledger_file,
        );
        match mutation {
            0 => retain_complete_lines(&ledger, 3),
            1 => rewrite_same_inode_same_length(&ledger),
            2 => {
                let bytes = fs::read(&ledger).expect("anchored consumption ledger");
                let mut lines: Vec<Vec<u8>> = bytes
                    .split_inclusive(|byte| *byte == b'\n')
                    .map(<[u8]>::to_vec)
                    .collect();
                assert_eq!(lines.len(), 5);
                lines.swap(3, 4);
                fs::write(&ledger, lines.concat()).expect("reorder prepared anchors");
            }
            _ => unreachable!(),
        }
        assert!(
            verify_controlled_trial_live_recovery(&fixture.config, &fixture.authorization).is_err(),
            "registry/pair mismatch or registry chain drift fails closed"
        );
    }
}

#[test]
fn sent_attempt_pair_loss_cannot_reset_recovery_ordinal() {
    let (fixture, expected) = recovery_continuation_prepared_fixture();
    let projection = verify_controlled_trial_live_recovery(&fixture.config, &fixture.authorization)
        .expect("prepared projection");
    let mut recovery = PmControlledTrialLiveCancelRecoveryJournals::open_phase_a_live_cancel(
        &fixture.config,
        &fixture.authorization,
        &fixture.runtime("2026-08-09T12:05:40Z"),
        projection,
    )
    .expect("prepared recovery owner");
    let reconciliation = recovery
        .record_phase_a_live_reconciliation_with_custody(
            "2026-08-09T12:05:41Z".into(),
            PmReconciliationOrderStateV1::ExactLive,
            Some(expected.clone()),
        )
        .expect("fresh exact live");
    let intent = recovery
        .record_phase_a_live_recovery_cancel_intent_with_custody(
            reconciliation,
            "2026-08-09T12:05:42Z".into(),
        )
        .expect("second recovery intent");
    let preparation = PmCancelPreparationV1::for_scoped_plan(
        &fixture.config,
        recovery.preflight_binding(),
        FixedOrderId::parse(&expected).expect("fixed order ID"),
        l2_time("2026-08-09T12:05:43Z"),
    )
    .expect("second recovery preparation");
    let prepared = recovery
        .record_phase_a_live_cancel_prepared_with_custody(intent, preparation)
        .expect("second prepared anchor");
    let owner = recovery
        .record_phase_a_live_cancel_dispatch_authorized_with_custody(prepared)
        .expect("second dispatch owner");
    let owner = recovery
        .revalidate_phase_a_live_cancel_for_runner(owner)
        .expect("runner revalidation");
    let may_have_sent = recovery
        .revalidate_phase_a_live_cancel_for_network_dispatch(owner)
        .expect("network owner")
        .into_may_have_been_dispatched()
        .expect("may-have-sent transition");
    drop(may_have_sent);
    drop(recovery);
    fs::remove_file(fixture.path(PM_PHASE_A_LIVE_CANCEL_RECOVERY_CONTINUATION_INTENT_FILE_V1))
        .expect("remove continuation intent");
    fs::remove_file(fixture.path(PM_PHASE_A_LIVE_CANCEL_RECOVERY_CONTINUATION_DISPATCH_FILE_V1))
        .expect("remove continuation dispatch");
    assert!(
        verify_controlled_trial_live_recovery(&fixture.config, &fixture.authorization).is_err(),
        "the surviving two-ordinal ledger registry forbids a fresh ordinal-one pair"
    );
}

#[test]
fn anchored_prepared_prefix_survives_loss_of_later_dispatch_records_conservatively() {
    let (fixture, expected) = recovery_continuation_prepared_fixture();
    let projection = verify_controlled_trial_live_recovery(&fixture.config, &fixture.authorization)
        .expect("prepared projection");
    let mut recovery = PmControlledTrialLiveCancelRecoveryJournals::open_phase_a_live_cancel(
        &fixture.config,
        &fixture.authorization,
        &fixture.runtime("2026-08-09T12:05:40Z"),
        projection,
    )
    .expect("prepared recovery owner");
    let reconciliation = recovery
        .record_phase_a_live_reconciliation_with_custody(
            "2026-08-09T12:05:41Z".into(),
            PmReconciliationOrderStateV1::ExactLive,
            Some(expected.clone()),
        )
        .expect("fresh exact live");
    let intent = recovery
        .record_phase_a_live_recovery_cancel_intent_with_custody(
            reconciliation,
            "2026-08-09T12:05:42Z".into(),
        )
        .expect("second intent");
    let preparation = PmCancelPreparationV1::for_scoped_plan(
        &fixture.config,
        recovery.preflight_binding(),
        FixedOrderId::parse(&expected).expect("fixed order ID"),
        l2_time("2026-08-09T12:05:43Z"),
    )
    .expect("second preparation");
    let prepared = recovery
        .record_phase_a_live_cancel_prepared_with_custody(intent, preparation)
        .expect("second prepared anchor");
    let owner = recovery
        .record_phase_a_live_cancel_dispatch_authorized_with_custody(prepared)
        .expect("later dispatch record");
    drop(owner);
    drop(recovery);

    let intent = fixture.path(PM_PHASE_A_LIVE_CANCEL_RECOVERY_CONTINUATION_INTENT_FILE_V1);
    let dispatch = fixture.path(PM_PHASE_A_LIVE_CANCEL_RECOVERY_CONTINUATION_DISPATCH_FILE_V1);
    retain_complete_lines(&intent, 5);
    retain_complete_lines(&dispatch, 3);
    let projection = verify_controlled_trial_live_recovery(&fixture.config, &fixture.authorization)
        .expect("anchored Prepared remains a conservative burned prefix");
    assert!(matches!(
        projection.classification(),
        PmTrialLiveRecoveryClassificationV1::RecoveryCancelOnly { .. }
    ));
}

#[test]
fn restoring_a_barrier_after_continuation_creation_cannot_restore_place_trust() {
    let (fixture, _) = terminal_dnd_fixture();
    let barrier_path = fixture.path(PM_PHASE_A_LIVE_DISPATCH_BARRIER_FILE_V1);
    let barrier_bytes = fs::read(&barrier_path).expect("intact barrier bytes");
    fs::remove_file(&barrier_path).expect("remove barrier");
    let projection = verify_controlled_trial_live_recovery(&fixture.config, &fixture.authorization)
        .expect("continuation basis");
    let recovery = PmControlledTrialLiveCancelRecoveryJournals::open_phase_a_live_cancel(
        &fixture.config,
        &fixture.authorization,
        &fixture.runtime("2026-08-09T12:05:30Z"),
        projection,
    )
    .expect("create continuation pair");
    drop(recovery);
    fs::write(&barrier_path, barrier_bytes).expect("restore exact barrier bytes");
    fs::set_permissions(&barrier_path, fs::Permissions::from_mode(0o600))
        .expect("protect restored barrier");

    let projection = verify_controlled_trial_live_recovery(&fixture.config, &fixture.authorization)
        .expect("continuation remains authoritative recovery evidence");
    assert!(!projection.phase_a_live_dispatch_barrier_durable());
    assert!(!projection.placement_resumption_allowed());
    assert_eq!(
        projection.phase_a_live_cancel_recovery_required_action(),
        Some(&PmPhaseALiveCancelRecoveryRequiredActionV1::ReconcileCurrentExposure)
    );
}

#[test]
fn original_dispatch_terminal_only_root_is_preserved_read_only() {
    let (fixture, _) = terminal_dnd_fixture();
    let intent_path = fixture.path(PM_TRIAL_LIVE_INTENT_FILE_V1);
    let intent_count = fs::read(&intent_path)
        .expect("main intent journal")
        .iter()
        .filter(|byte| **byte == b'\n')
        .count();
    retain_complete_lines(&intent_path, intent_count - 1);
    let original_prefix = fs::read(&intent_path).expect("dispatch-terminal predecessor");
    fs::remove_file(fixture.path(PM_PHASE_A_LIVE_DISPATCH_BARRIER_FILE_V1))
        .expect("remove barrier");
    let projection = verify_controlled_trial_live_recovery(&fixture.config, &fixture.authorization)
        .expect("distinct dispatch-terminal-only continuation basis");
    assert_eq!(
        projection.phase_a_live_cancel_recovery_required_action(),
        Some(&PmPhaseALiveCancelRecoveryRequiredActionV1::ReconcileCurrentExposure)
    );
    let mut recovery = PmControlledTrialLiveCancelRecoveryJournals::open_phase_a_live_cancel(
        &fixture.config,
        &fixture.authorization,
        &fixture.runtime("2026-08-09T12:05:30Z"),
        projection,
    )
    .expect("recovery-only continuation owner");
    recovery
        .record_phase_a_live_reconciliation_with_custody(
            "2026-08-09T12:05:31Z".into(),
            PmReconciliationOrderStateV1::Absent,
            None,
        )
        .expect("exact absence");
    recovery
        .record_terminal(
            "2026-08-09T12:05:32Z".into(),
            PmIntentTerminalDispositionV1::Completed,
        )
        .expect("continuation terminal");
    drop(recovery);
    assert_eq!(
        fs::read(intent_path).expect("original intent remains exact"),
        original_prefix
    );
}

#[test]
fn continuation_terminal_revalidates_every_held_root_after_safe_reconciliation() {
    let drifts: [fn(&Fixture); 5] = [
        |fixture| rewrite_same_inode_same_length(&fixture.path(PM_TRIAL_LIVE_INTENT_FILE_V1)),
        |fixture| rewrite_same_inode_same_length(&fixture.path(PM_TRIAL_LIVE_DISPATCH_FILE_V1)),
        |fixture| {
            rewrite_same_inode_same_length(
                &fixture.path(
                    &fixture
                        .config
                        .value()
                        .journal
                        .authorization_consumption_ledger_file,
                ),
            );
        },
        |fixture| {
            fs::remove_file(
                fixture.path(
                    &fixture
                        .config
                        .value()
                        .journal
                        .authorization_consumption_claim_file,
                ),
            )
            .expect("remove atomic claim");
        },
        |fixture| {
            fs::write(
                fixture.path("unexpected-after-safe-reconciliation"),
                b"drift",
            )
            .expect("change artifact directory");
        },
    ];
    for drift in drifts {
        let (fixture, _) = terminal_dnd_fixture();
        fs::remove_file(fixture.path(PM_PHASE_A_LIVE_DISPATCH_BARRIER_FILE_V1))
            .expect("remove barrier");
        let projection =
            verify_controlled_trial_live_recovery(&fixture.config, &fixture.authorization)
                .expect("continuation basis");
        let mut recovery = PmControlledTrialLiveCancelRecoveryJournals::open_phase_a_live_cancel(
            &fixture.config,
            &fixture.authorization,
            &fixture.runtime("2026-08-09T12:05:30Z"),
            projection,
        )
        .expect("continuation owner");
        recovery
            .record_phase_a_live_reconciliation_with_custody(
                "2026-08-09T12:05:31Z".into(),
                PmReconciliationOrderStateV1::Absent,
                None,
            )
            .expect("safe reconciliation");
        drift(&fixture);
        assert!(matches!(
            recovery.record_terminal(
                "2026-08-09T12:05:32Z".into(),
                PmIntentTerminalDispositionV1::Completed,
            ),
            Err(PmTrialLiveJournalError::Protection) | Err(PmTrialLiveJournalError::AmbiguousTail)
        ));
    }
}

#[test]
fn impossible_continuation_creation_pairs_are_rejected() {
    for mutate in [
        0_u8, // dispatch without intent
        1_u8, // both zero (not reachable in create order)
        2_u8, // torn intent header
    ] {
        let (fixture, _) = terminal_dnd_fixture();
        fs::remove_file(fixture.path(PM_PHASE_A_LIVE_DISPATCH_BARRIER_FILE_V1))
            .expect("remove barrier");
        let projection =
            verify_controlled_trial_live_recovery(&fixture.config, &fixture.authorization)
                .expect("continuation basis");
        let recovery = PmControlledTrialLiveCancelRecoveryJournals::open_phase_a_live_cancel(
            &fixture.config,
            &fixture.authorization,
            &fixture.runtime("2026-08-09T12:05:30Z"),
            projection,
        )
        .expect("create headers");
        drop(recovery);
        let intent = fixture.path(PM_PHASE_A_LIVE_CANCEL_RECOVERY_CONTINUATION_INTENT_FILE_V1);
        let dispatch = fixture.path(PM_PHASE_A_LIVE_CANCEL_RECOVERY_CONTINUATION_DISPATCH_FILE_V1);
        match mutate {
            0 => fs::remove_file(intent).expect("remove intent"),
            1 => {
                fs::write(intent, []).expect("zero intent");
                fs::write(dispatch, []).expect("zero dispatch");
            }
            2 => {
                let bytes = fs::read(&intent).expect("intent header");
                fs::write(&intent, &bytes[..bytes.len() / 2]).expect("torn intent");
                fs::remove_file(dispatch).expect("remove dispatch");
            }
            _ => unreachable!(),
        }
        assert!(
            verify_controlled_trial_live_recovery(&fixture.config, &fixture.authorization).is_err()
        );
    }
}

#[test]
fn terminal_plan_anchor_source_completes_zero_one_or_two_halves() {
    for retained_halves in [0_u8, 1, 2] {
        let (fixture, _) = terminal_dnd_fixture();
        fs::remove_file(fixture.path(PM_PHASE_A_LIVE_DISPATCH_BARRIER_FILE_V1))
            .expect("remove barrier");
        let projection =
            verify_controlled_trial_live_recovery(&fixture.config, &fixture.authorization)
                .expect("continuation basis");
        let mut recovery = PmControlledTrialLiveCancelRecoveryJournals::open_phase_a_live_cancel(
            &fixture.config,
            &fixture.authorization,
            &fixture.runtime("2026-08-09T12:05:30Z"),
            projection,
        )
        .expect("continuation owner");
        recovery
            .record_phase_a_live_reconciliation_with_custody(
                "2026-08-09T12:05:31Z".into(),
                PmReconciliationOrderStateV1::Absent,
                None,
            )
            .expect("safe reconciliation");
        let intent = fixture.path(PM_PHASE_A_LIVE_CANCEL_RECOVERY_CONTINUATION_INTENT_FILE_V1);
        let dispatch = fixture.path(PM_PHASE_A_LIVE_CANCEL_RECOVERY_CONTINUATION_DISPATCH_FILE_V1);
        let intent_predecessor_count = fs::read(&intent)
            .expect("intent predecessor")
            .iter()
            .filter(|byte| **byte == b'\n')
            .count();
        let dispatch_predecessor_count = fs::read(&dispatch)
            .expect("dispatch predecessor")
            .iter()
            .filter(|byte| **byte == b'\n')
            .count();
        recovery
            .record_terminal(
                "2026-08-09T12:05:32Z".into(),
                PmIntentTerminalDispositionV1::Completed,
            )
            .expect("ledger-first paired continuation Terminal");
        drop(recovery);
        let registry = verify_authorization_consumption(&fixture.config, &fixture.authorization)
            .expect("monotonic Terminal plan")
            .recovery_continuation_registry
            .expect("continuation registry");
        assert!(registry.terminal_plan.is_some());

        if retained_halves < 2 {
            retain_complete_lines(&intent, intent_predecessor_count);
        }
        if retained_halves == 0 {
            retain_complete_lines(&dispatch, dispatch_predecessor_count);
        }
        let prefix = verify_controlled_trial_live_recovery(&fixture.config, &fixture.authorization)
            .expect("Terminal plan prefix is closed");
        let expected_action = if retained_halves == 2 {
            PmPhaseALiveCancelRecoveryRequiredActionV1::TerminalEvidenceOnly
        } else {
            PmPhaseALiveCancelRecoveryRequiredActionV1::CompletePendingTerminal
        };
        assert_eq!(
            prefix.phase_a_live_cancel_recovery_required_action(),
            Some(&expected_action)
        );
        let mut reopened = PmControlledTrialLiveCancelRecoveryJournals::open_phase_a_live_cancel(
            &fixture.config,
            &fixture.authorization,
            &fixture.runtime("2026-08-09T12:05:40Z"),
            prefix,
        )
        .expect("open source-completes the anchored Terminal plan");
        assert_eq!(
            reopened.required_action(),
            Some(&PmPhaseALiveCancelRecoveryRequiredActionV1::TerminalEvidenceOnly)
        );
        assert!(matches!(
            reopened.record_phase_a_live_reconciliation_with_custody(
                "2026-08-09T12:05:41Z".into(),
                PmReconciliationOrderStateV1::Absent,
                None,
            ),
            Err(PmTrialLiveJournalError::RecoveryOperationForbidden)
        ));
        assert!(matches!(
            reopened.record_terminal(
                "2026-08-09T12:05:42Z".into(),
                PmIntentTerminalDispositionV1::Stopped,
            ),
            Err(PmTrialLiveJournalError::RecoveryOperationForbidden)
        ));
        assert!(matches!(
            reopened.complete_pending_terminal(),
            Err(PmTrialLiveJournalError::RecoveryOperationForbidden)
        ));
        drop(reopened);
        let complete =
            verify_controlled_trial_live_recovery(&fixture.config, &fixture.authorization)
                .expect("completed Terminal pair");
        assert_eq!(
            complete.phase_a_live_cancel_recovery_required_action(),
            Some(&PmPhaseALiveCancelRecoveryRequiredActionV1::TerminalEvidenceOnly)
        );

        if retained_halves == 0 {
            // Repeating the exact pair rollback cannot change caller facts or
            // advance the continuation; the same ledger plan completes again.
            retain_complete_lines(&intent, intent_predecessor_count);
            retain_complete_lines(&dispatch, dispatch_predecessor_count);
            let replay =
                verify_controlled_trial_live_recovery(&fixture.config, &fixture.authorization)
                    .expect("same anchor-only prefix");
            let replayed = PmControlledTrialLiveCancelRecoveryJournals::open_phase_a_live_cancel(
                &fixture.config,
                &fixture.authorization,
                &fixture.runtime("2026-08-09T12:05:50Z"),
                replay,
            )
            .expect("idempotent source completion");
            assert_eq!(
                replayed.required_action(),
                Some(&PmPhaseALiveCancelRecoveryRequiredActionV1::TerminalEvidenceOnly)
            );
        }
    }
}

#[test]
fn base_v1_terminal_cannot_use_the_completed_continuation_evidence_open() {
    let (fixture, _) = terminal_dnd_fixture();
    let projection = verify_controlled_trial_live_recovery(&fixture.config, &fixture.authorization)
        .expect("intact-barrier V1 Terminal evidence");
    assert_eq!(
        projection.classification(),
        &PmTrialLiveRecoveryClassificationV1::TerminalEvidenceOnly
    );
    assert!(
        PmControlledTrialLiveCancelRecoveryJournals::open_phase_a_live_cancel(
            &fixture.config,
            &fixture.authorization,
            &fixture.runtime("2026-08-09T12:05:30Z"),
            projection,
        )
        .is_err(),
        "only a verifier-bound completed continuation Terminal may use the narrow evidence reopen"
    );
}

#[test]
fn terminal_plan_rejects_pair_reset_tamper_and_pair_ahead() {
    let (fixture, _, _) = continuation_terminal_plan_fixture();
    retain_complete_lines(
        &fixture.path(PM_PHASE_A_LIVE_CANCEL_RECOVERY_CONTINUATION_INTENT_FILE_V1),
        1,
    );
    retain_complete_lines(
        &fixture.path(PM_PHASE_A_LIVE_CANCEL_RECOVERY_CONTINUATION_DISPATCH_FILE_V1),
        1,
    );
    assert!(
        verify_controlled_trial_live_recovery(&fixture.config, &fixture.authorization).is_err(),
        "the surviving plan rejects rollback before its exact causal predecessors"
    );

    let (fixture, _, _) = continuation_terminal_plan_fixture();
    fs::remove_file(fixture.path(PM_PHASE_A_LIVE_CANCEL_RECOVERY_CONTINUATION_INTENT_FILE_V1))
        .expect("remove continuation intent");
    fs::remove_file(fixture.path(PM_PHASE_A_LIVE_CANCEL_RECOVERY_CONTINUATION_DISPATCH_FILE_V1))
        .expect("remove continuation dispatch");
    assert!(
        verify_controlled_trial_live_recovery(&fixture.config, &fixture.authorization).is_err(),
        "pair loss cannot erase a surviving Terminal plan"
    );

    let (fixture, _, _) = continuation_terminal_plan_fixture();
    rewrite_same_inode_same_length(
        &fixture.path(PM_PHASE_A_LIVE_CANCEL_RECOVERY_CONTINUATION_DISPATCH_FILE_V1),
    );
    assert!(
        verify_controlled_trial_live_recovery(&fixture.config, &fixture.authorization).is_err(),
        "Terminal bytes must match the exact canonical plan"
    );

    let (fixture, _, _) = continuation_terminal_plan_fixture();
    let ledger = fixture.path(
        &fixture
            .config
            .value()
            .journal
            .authorization_consumption_ledger_file,
    );
    let ledger_count = fs::read(&ledger)
        .expect("consumption ledger")
        .iter()
        .filter(|byte| **byte == b'\n')
        .count();
    retain_complete_lines(&ledger, ledger_count - 1);
    assert!(
        verify_controlled_trial_live_recovery(&fixture.config, &fixture.authorization).is_err(),
        "a physical Terminal pair may never lead the monotonic ledger"
    );
}

#[test]
fn continuation_result_and_bridge_crash_prefixes_resume_exact_cancel_custody() {
    for bridge_before_crash in [false, true] {
        let (fixture, expected) = terminal_dnd_fixture();
        fs::remove_file(fixture.path(PM_PHASE_A_LIVE_DISPATCH_BARRIER_FILE_V1))
            .expect("remove barrier");
        let projection =
            verify_controlled_trial_live_recovery(&fixture.config, &fixture.authorization)
                .expect("continuation basis");
        let mut recovery = PmControlledTrialLiveCancelRecoveryJournals::open_phase_a_live_cancel(
            &fixture.config,
            &fixture.authorization,
            &fixture.runtime("2026-08-09T12:05:30Z"),
            projection,
        )
        .expect("continuation owner");
        let reconciliation = recovery
            .record_phase_a_live_reconciliation_with_custody(
                "2026-08-09T12:05:31Z".into(),
                PmReconciliationOrderStateV1::ExactLive,
                Some(expected.clone()),
            )
            .expect("exact live");
        let intent = recovery
            .record_phase_a_live_recovery_cancel_intent_with_custody(
                reconciliation,
                "2026-08-09T12:05:32Z".into(),
            )
            .expect("recovery intent");
        let preparation = PmCancelPreparationV1::for_scoped_plan(
            &fixture.config,
            recovery.preflight_binding(),
            FixedOrderId::parse(&expected).expect("fixed order ID"),
            l2_time("2026-08-09T12:05:33Z"),
        )
        .expect("cancel preparation");
        let prepared = recovery
            .record_phase_a_live_cancel_prepared_with_custody(intent, preparation)
            .expect("prepared");
        let owner = recovery
            .record_phase_a_live_cancel_dispatch_authorized_with_custody(prepared)
            .expect("dispatch owner");
        let owner = recovery
            .revalidate_phase_a_live_cancel_for_runner(owner)
            .expect("runner revalidation");
        let observation = recovery
            .revalidate_phase_a_live_cancel_for_network_dispatch(owner)
            .expect("network owner")
            .into_may_have_been_dispatched()
            .expect("may-have-sent observation");
        let result = recovery
            .record_phase_a_live_cancel_result(observation, PmCancelResultKindV1::Canceled)
            .expect("durable canceled result");
        if bridge_before_crash {
            let bridge = recovery
                .record_phase_a_live_cancel_outcome_bridge_with_custody(result)
                .expect("durable bridge");
            drop(bridge);
        } else {
            drop(result);
        }
        drop(recovery);

        let projection =
            verify_controlled_trial_live_recovery(&fixture.config, &fixture.authorization)
                .expect("result prefix projection");
        assert_eq!(
            projection.phase_a_live_cancel_recovery_required_action(),
            Some(&PmPhaseALiveCancelRecoveryRequiredActionV1::ResumeCancelOutcome)
        );
        let mut reopened = PmControlledTrialLiveCancelRecoveryJournals::open_phase_a_live_cancel(
            &fixture.config,
            &fixture.authorization,
            &fixture.runtime("2026-08-09T12:05:40Z"),
            projection,
        )
        .expect("result recovery owner");
        let bridge = reopened
            .resume_phase_a_live_cancel_outcome_with_custody()
            .expect("source-owned result/bridge recovery");
        let resolved = reopened
            .record_phase_a_live_cancel_reconciliation_with_custody(
                bridge,
                "2026-08-09T12:05:41Z".into(),
                PmReconciliationOrderStateV1::ExactCanceled,
                Some(expected),
            )
            .expect("resumed cancel-epoch reconciliation");
        assert_eq!(
            resolved.state(),
            PmReconciliationOrderStateV1::ExactCanceled
        );
    }
}

#[test]
fn continuation_prepared_cycles_burn_ordinals_and_replay_across_restarts() {
    let (fixture, expected) = terminal_dnd_fixture();
    let fixed = FixedOrderId::parse(&expected).expect("fixed order ID");
    fs::remove_file(fixture.path(PM_PHASE_A_LIVE_DISPATCH_BARRIER_FILE_V1))
        .expect("remove barrier");
    let projection = verify_controlled_trial_live_recovery(&fixture.config, &fixture.authorization)
        .expect("continuation basis");
    let mut recovery = PmControlledTrialLiveCancelRecoveryJournals::open_phase_a_live_cancel(
        &fixture.config,
        &fixture.authorization,
        &fixture.runtime("2026-08-09T12:05:30Z"),
        projection,
    )
    .expect("continuation owner");
    let reconciliation = recovery
        .record_phase_a_live_reconciliation_with_custody(
            "2026-08-09T12:05:31Z".into(),
            PmReconciliationOrderStateV1::ExactLive,
            Some(expected.clone()),
        )
        .expect("cycle one reconciliation");
    let intent = recovery
        .record_phase_a_live_recovery_cancel_intent_with_custody(
            reconciliation,
            "2026-08-09T12:05:32Z".into(),
        )
        .expect("cycle one intent");
    let preparation = PmCancelPreparationV1::for_scoped_plan(
        &fixture.config,
        recovery.preflight_binding(),
        fixed,
        l2_time("2026-08-09T12:05:33Z"),
    )
    .expect("cycle one preparation");
    let prepared = recovery
        .record_phase_a_live_cancel_prepared_with_custody(intent, preparation)
        .expect("cycle one prepared");
    assert_eq!(
        prepared.preparation().dispatch_class(),
        PmCancelDispatchClassV1::Recovery { ordinal: 1 }
    );
    drop(prepared);
    drop(recovery);

    let projection = verify_controlled_trial_live_recovery(&fixture.config, &fixture.authorization)
        .expect("prepared-prefix projection");
    assert_eq!(
        projection.classification(),
        &PmTrialLiveRecoveryClassificationV1::RecoveryCancelOnly {
            exact_venue_order_id: expected.clone(),
        }
    );
    let mut recovery = PmControlledTrialLiveCancelRecoveryJournals::open_phase_a_live_cancel(
        &fixture.config,
        &fixture.authorization,
        &fixture.runtime("2026-08-09T12:05:40Z"),
        projection,
    )
    .expect("cycle two owner");
    let reconciliation = recovery
        .record_phase_a_live_reconciliation_with_custody(
            "2026-08-09T12:05:41Z".into(),
            PmReconciliationOrderStateV1::ExactLive,
            Some(expected.clone()),
        )
        .expect("fresh exact-live observation");
    let intent = recovery
        .record_phase_a_live_recovery_cancel_intent_with_custody(
            reconciliation,
            "2026-08-09T12:05:42Z".into(),
        )
        .expect("cycle two intent");
    let preparation = PmCancelPreparationV1::for_scoped_plan(
        &fixture.config,
        recovery.preflight_binding(),
        fixed,
        l2_time("2026-08-09T12:05:43Z"),
    )
    .expect("cycle two preparation");
    let prepared = recovery
        .record_phase_a_live_cancel_prepared_with_custody(intent, preparation)
        .expect("cycle two prepared");
    let owner = recovery
        .record_phase_a_live_cancel_dispatch_authorized_with_custody(prepared)
        .expect("cycle two dispatch");
    assert_eq!(
        owner.dispatch_class(),
        PmCancelDispatchClassV1::Recovery { ordinal: 2 }
    );
    drop(owner);
    drop(recovery);

    let projection = verify_controlled_trial_live_recovery(&fixture.config, &fixture.authorization)
        .expect("two-cycle canonical replay");
    let mut recovery = PmControlledTrialLiveCancelRecoveryJournals::open_phase_a_live_cancel(
        &fixture.config,
        &fixture.authorization,
        &fixture.runtime("2026-08-09T12:05:50Z"),
        projection,
    )
    .expect("post-cycle recovery owner");
    let reconciliation = recovery
        .record_phase_a_live_reconciliation_with_custody(
            "2026-08-09T12:05:51Z".into(),
            PmReconciliationOrderStateV1::ExactLive,
            Some(expected),
        )
        .expect("latest exact-live exposure");
    assert!(matches!(
        recovery.record_phase_a_live_recovery_cancel_intent_with_custody(
            reconciliation,
            "2026-08-09T12:05:52Z".into(),
        ),
        Err(PmTrialLiveJournalError::BoundExceeded)
    ));
}

#[test]
fn continuation_recovery_cancel_only_accepts_a_fresh_safe_observation() {
    let (fixture, expected) = terminal_dnd_fixture();
    fs::remove_file(fixture.path(PM_PHASE_A_LIVE_DISPATCH_BARRIER_FILE_V1))
        .expect("remove barrier");
    let projection = verify_controlled_trial_live_recovery(&fixture.config, &fixture.authorization)
        .expect("continuation basis");
    let mut recovery = PmControlledTrialLiveCancelRecoveryJournals::open_phase_a_live_cancel(
        &fixture.config,
        &fixture.authorization,
        &fixture.runtime("2026-08-09T12:05:30Z"),
        projection,
    )
    .expect("continuation owner");
    let reconciliation = recovery
        .record_phase_a_live_reconciliation_with_custody(
            "2026-08-09T12:05:31Z".into(),
            PmReconciliationOrderStateV1::ExactLive,
            Some(expected),
        )
        .expect("initial exact live");
    let intent = recovery
        .record_phase_a_live_recovery_cancel_intent_with_custody(
            reconciliation,
            "2026-08-09T12:05:32Z".into(),
        )
        .expect("intent-only prefix");
    drop(intent);
    drop(recovery);

    let projection = verify_controlled_trial_live_recovery(&fixture.config, &fixture.authorization)
        .expect("intent-prefix projection");
    assert!(matches!(
        projection.classification(),
        PmTrialLiveRecoveryClassificationV1::RecoveryCancelOnly { .. }
    ));
    let mut recovery = PmControlledTrialLiveCancelRecoveryJournals::open_phase_a_live_cancel(
        &fixture.config,
        &fixture.authorization,
        &fixture.runtime("2026-08-09T12:05:40Z"),
        projection,
    )
    .expect("intent-prefix owner");
    recovery
        .record_phase_a_live_reconciliation_with_custody(
            "2026-08-09T12:05:41Z".into(),
            PmReconciliationOrderStateV1::Absent,
            None,
        )
        .expect("fresh absence supersedes prior exact-live state");
    recovery
        .record_terminal(
            "2026-08-09T12:05:42Z".into(),
            PmIntentTerminalDispositionV1::Completed,
        )
        .expect("safe terminal");
}

#[test]
fn main_v1_prepared_supersession_burns_primary_and_recovery_ordinals() {
    let (fixture, prepared_consumption) = Fixture::new();
    let expected = fixture
        .config
        .exact_place_public_request_identity()
        .expected_order_id();
    let expected_text = expected.to_string();
    let fixed = FixedOrderId::from(expected);
    let mut journals = bind_preflight(&fixture);
    let place = record_accepted_place_custody(&fixture, prepared_consumption, &mut journals);
    let intent = journals
        .record_phase_a_live_primary_cancel_intent_with_custody(
            place,
            "2026-08-09T12:05:10Z".into(),
        )
        .expect("primary intent");
    let preparation = PmCancelPreparationV1::for_scoped_plan(
        &fixture.config,
        journals.preflight_binding(),
        fixed,
        l2_time("2026-08-09T12:05:11Z"),
    )
    .expect("primary preparation");
    let prepared = journals
        .record_phase_a_live_cancel_prepared_with_custody(intent, preparation)
        .expect("primary prepared tail");
    assert_eq!(
        prepared.preparation().dispatch_class(),
        PmCancelDispatchClassV1::Primary
    );
    drop(prepared);
    drop(journals);

    let projection = verify_controlled_trial_live_recovery(&fixture.config, &fixture.authorization)
        .expect("primary prepared recovery");
    let mut recovery = PmControlledTrialLiveCancelRecoveryJournals::open_phase_a_live_cancel(
        &fixture.config,
        &fixture.authorization,
        &fixture.runtime("2026-08-09T12:05:40Z"),
        projection,
    )
    .expect("recovery-one owner");
    let reconciliation = recovery
        .record_phase_a_live_reconciliation_with_custody(
            "2026-08-09T12:05:41Z".into(),
            PmReconciliationOrderStateV1::ExactLive,
            Some(expected_text.clone()),
        )
        .expect("fresh exact-live after primary prepared crash");
    let intent = recovery
        .record_phase_a_live_recovery_cancel_intent_with_custody(
            reconciliation,
            "2026-08-09T12:05:42Z".into(),
        )
        .expect("recovery-one intent");
    let preparation = PmCancelPreparationV1::for_scoped_plan(
        &fixture.config,
        recovery.preflight_binding(),
        fixed,
        l2_time("2026-08-09T12:05:43Z"),
    )
    .expect("recovery-one preparation");
    let prepared = recovery
        .record_phase_a_live_cancel_prepared_with_custody(intent, preparation)
        .expect("superseding recovery-one prepared");
    assert_eq!(
        prepared.preparation().dispatch_class(),
        PmCancelDispatchClassV1::Recovery { ordinal: 1 }
    );
    drop(prepared);
    drop(recovery);

    let projection = verify_controlled_trial_live_recovery(&fixture.config, &fixture.authorization)
        .expect("recovery-one prepared replay");
    let mut recovery = PmControlledTrialLiveCancelRecoveryJournals::open_phase_a_live_cancel(
        &fixture.config,
        &fixture.authorization,
        &fixture.runtime("2026-08-09T12:05:50Z"),
        projection,
    )
    .expect("recovery-two owner");
    let reconciliation = recovery
        .record_phase_a_live_reconciliation_with_custody(
            "2026-08-09T12:05:51Z".into(),
            PmReconciliationOrderStateV1::ExactLive,
            Some(expected_text.clone()),
        )
        .expect("fresh exact-live after recovery-one prepared crash");
    let intent = recovery
        .record_phase_a_live_recovery_cancel_intent_with_custody(
            reconciliation,
            "2026-08-09T12:05:52Z".into(),
        )
        .expect("recovery-two intent");
    let preparation = PmCancelPreparationV1::for_scoped_plan(
        &fixture.config,
        recovery.preflight_binding(),
        fixed,
        l2_time("2026-08-09T12:05:53Z"),
    )
    .expect("recovery-two preparation");
    let prepared = recovery
        .record_phase_a_live_cancel_prepared_with_custody(intent, preparation)
        .expect("superseding recovery-two prepared");
    let owner = recovery
        .record_phase_a_live_cancel_dispatch_authorized_with_custody(prepared)
        .expect("dispatch only newest prepared attempt");
    assert_eq!(
        owner.dispatch_class(),
        PmCancelDispatchClassV1::Recovery { ordinal: 2 }
    );
    drop(owner);
    drop(recovery);

    let projection = verify_controlled_trial_live_recovery(&fixture.config, &fixture.authorization)
        .expect("reopen after superseding prepared and dispatch");
    let mut recovery = PmControlledTrialLiveCancelRecoveryJournals::open_phase_a_live_cancel(
        &fixture.config,
        &fixture.authorization,
        &fixture.runtime("2026-08-09T12:06:00Z"),
        projection,
    )
    .expect("budget-exhaustion owner");
    let reconciliation = recovery
        .record_phase_a_live_reconciliation_with_custody(
            "2026-08-09T12:06:01Z".into(),
            PmReconciliationOrderStateV1::ExactLive,
            Some(expected_text),
        )
        .expect("latest exact-live observation");
    assert!(matches!(
        recovery.record_phase_a_live_recovery_cancel_intent_with_custody(
            reconciliation,
            "2026-08-09T12:06:02Z".into(),
        ),
        Err(PmTrialLiveJournalError::BoundExceeded)
    ));
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
