mod support;

use std::{fs, os::unix::fs::MetadataExt as _, path::Path};

use reap_pm_controlled_trial::{
    FixedOrderId, PreparedAuthorizationConsumption, verify_authorization_consumption,
};
use reap_pm_controlled_trial_live::{
    PM_TRIAL_LIVE_DISPATCH_FILE_V1, PM_TRIAL_LIVE_INTENT_FILE_V1, PmCancelDispatchClassV1,
    PmCancelPreparationV1, PmCancelResultKindV1, PmControlledTrialLiveJournals,
    PmControlledTrialLiveRecoveryJournals, PmIntentTerminalDispositionV1, PmPlacePreparationV1,
    PmPlaceResultKindV1, PmReconciliationOrderStateV1, PmTrialLiveJournalError,
    PmTrialLiveRecoveryClassificationV1, verify_controlled_trial_live_recovery,
};

use support::{Fixture, l2_time};

const PLACE_TIME: &str = "2026-08-09T12:05:04Z";
const CANCEL_TIME: &str = "2026-08-09T12:06:00Z";

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

    // PlaceDispatchAuthorized durable, response/result absent.
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
    let (dispatch, consumed) = record_place_dispatch(&fixture, prepared_consumption, &mut journals);
    drop(dispatch);
    drop(consumed);
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
