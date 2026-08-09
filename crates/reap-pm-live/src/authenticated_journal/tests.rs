use std::io::Cursor;

use reap_pm_core::{
    EvmAddress, PmAccountScope, PmClientOrderId, PmClientOrderKey, PmFunderId, PmVenueOrderId,
    PmVenueOrderKey,
};
use reap_pm_live_contracts::{
    PmAccountConnectivityConfig, PmAccountSignatureProfile, PmConnectivityConfig,
};

use super::*;
use crate::authenticated_journal::recovery::{
    PmAuthenticatedJournalRecoveryError, recover_lines, recover_pm_authenticated_journal,
};
use crate::authenticated_journal::schema::{
    PM_AUTHENTICATED_JOURNAL_FAMILY, PM_AUTHENTICATED_JOURNAL_VERSION,
    PM_T2_PROXY_AUTHENTICATED_JOURNAL_VERSION, PmAuthenticatedCancelPreparedV1,
    PmAuthenticatedCancelResultKindV1, PmAuthenticatedCancelResultV1, PmAuthenticatedCommitmentV1,
    PmAuthenticatedCoordinatorIdentityV1, PmAuthenticatedJournalHeaderV1,
    PmAuthenticatedJournalLineV1, PmAuthenticatedPlacePreparedV1, PmAuthenticatedPlaceResultKindV1,
    PmAuthenticatedPlaceResultV1, test_proxy_scope, test_proxy_scope_with_identities, test_scope,
    test_scope_with_credential_slot,
};

const EXPECTED_ORDER_ID: &str =
    "0x5555555555555555555555555555555555555555555555555555555555555555";
const L2_TIMESTAMP: u64 = 1_760_000_000;
const FROZEN_PRE_PM_T2_V1_HEADER_LINE: &str = concat!(
    r#"["reap-pm-authenticated-mutation-journal",1,"1ffc7aed927722e9192043ba31bb3f3dbd77154790543c26e2c40ac8f7b43ff4",0,{"header":{"scope":{"product":"reap-pm","schema_family":"reap-pm-authenticated-mutation-journal","schema_version":1,"account_scope":{"environment":"authenticated-journal-test","chain":137,"signer":"0x1111111111111111111111111111111111111111","funder":"0x1111111111111111111111111111111111111111","handle":9},"configured_instrument":{"market":"0x2222222222222222222222222222222222222222222222222222222222222222","token":"42"},"configuration_fingerprint":"3333333333333333333333333333333333333333333333333333333333333333","credential_slot_fingerprint":"4444444444444444444444444444444444444444444444444444444444444444","production_order_entry_authorized":false,"scope_fingerprint":"1ffc7aed927722e9192043ba31bb3f3dbd77154790543c26e2c40ac8f7b43ff4"}}}]"#,
    "\n",
);

fn proxy_connectivity_config() -> PmConnectivityConfig {
    let eoa = crate::evidence::connectivity_config();
    let eoa_scope = eoa.account().account_scope();
    let proxy_scope = PmAccountScope::new(
        eoa_scope.environment(),
        eoa_scope.chain(),
        eoa_scope.signer(),
        PmFunderId::new(EvmAddress::from_bytes([0x77; 20]).expect("proxy funder")),
        eoa_scope.handle(),
    );
    let proxy = PmAccountConnectivityConfig::derive_pm_t2_proxy(
        eoa.public(),
        proxy_scope,
        eoa.account().account_route(),
    )
    .expect("PM-T2 proxy account config");
    PmConnectivityConfig::new(eoa.public().clone(), proxy).expect("PM-T2 proxy product config")
}

fn coordinator(scope: &PmAuthenticatedJournalScopeV1) -> PmAuthenticatedCoordinatorIdentityV1 {
    PmAuthenticatedCoordinatorIdentityV1::new(
        PmClientOrderKey::new(
            scope.account(),
            PmClientOrderId::from_bytes([0x44; 16]).expect("client order"),
        ),
        scope.instrument(),
    )
}

fn venue_order(scope: &PmAuthenticatedJournalScopeV1, value: &str) -> PmVenueOrderKey {
    PmVenueOrderKey::new(
        scope.account(),
        PmVenueOrderId::new(value).expect("venue order"),
    )
}

fn place_prepared(scope: &PmAuthenticatedJournalScopeV1) -> PmAuthenticatedPlacePreparedV1 {
    PmAuthenticatedPlacePreparedV1::test_new(
        scope,
        coordinator(scope),
        7,
        [0x33; 32],
        [0x55; 32],
        L2_TIMESTAMP,
    )
}

fn cancel_prepared(scope: &PmAuthenticatedJournalScopeV1) -> PmAuthenticatedCancelPreparedV1 {
    PmAuthenticatedCancelPreparedV1::test_new(
        scope,
        coordinator(scope),
        venue_order(scope, EXPECTED_ORDER_ID),
        8,
        [0x77; 32],
        [0x55; 32],
        L2_TIMESTAMP,
    )
}

fn append(
    bytes: &mut Vec<u8>,
    scope: &PmAuthenticatedJournalScopeV1,
    sequence: u64,
    record: PmAuthenticatedJournalRecordV1,
) {
    serde_json::to_writer(
        &mut *bytes,
        &PmAuthenticatedJournalLineV1::new_for_scope(scope, sequence, record),
    )
    .expect("encode journal line");
    bytes.push(b'\n');
}

fn header(bytes: &mut Vec<u8>, scope: &PmAuthenticatedJournalScopeV1) {
    append(
        bytes,
        scope,
        0,
        PmAuthenticatedJournalRecordV1::Header(PmAuthenticatedJournalHeaderV1::new(scope.clone())),
    );
}

fn recover_one_place_result(
    classification: PmAuthenticatedPlaceResultKindV1,
) -> PmAuthenticatedJournalRecovery {
    let scope = test_scope();
    let prepared = place_prepared(&scope);
    let result = match classification {
        PmAuthenticatedPlaceResultKindV1::Accepted => {
            PmAuthenticatedPlaceResultV1::accepted(coordinator(&scope), 2, [0x55; 32])
        }
        PmAuthenticatedPlaceResultKindV1::Rejected => {
            PmAuthenticatedPlaceResultV1::rejected(coordinator(&scope), 2)
        }
        PmAuthenticatedPlaceResultKindV1::DefinitelyNotDispatched => {
            PmAuthenticatedPlaceResultV1::definitely_not_dispatched(coordinator(&scope), 2)
        }
        PmAuthenticatedPlaceResultKindV1::OutOfProfile => {
            PmAuthenticatedPlaceResultV1::out_of_profile(coordinator(&scope), 2, Some([0x66; 32]))
        }
        PmAuthenticatedPlaceResultKindV1::AcknowledgementUnknown => {
            PmAuthenticatedPlaceResultV1::acknowledgement_unknown(
                coordinator(&scope),
                2,
                Some([0x66; 32]),
            )
        }
    };
    let mut bytes = Vec::new();
    header(&mut bytes, &scope);
    append(
        &mut bytes,
        &scope,
        1,
        PmAuthenticatedJournalRecordV1::PlacePrepared(prepared),
    );
    append(
        &mut bytes,
        &scope,
        2,
        PmAuthenticatedJournalRecordV1::DispatchAuthorized(
            PmAuthenticatedDispatchAuthorizedV1::from_durable_prepared(prepared.operation, 1),
        ),
    );
    append(
        &mut bytes,
        &scope,
        3,
        PmAuthenticatedJournalRecordV1::PlaceResult(result),
    );
    recover_lines(&mut Cursor::new(bytes), &scope).expect("recover classified place")
}

fn recover_one_cancel_result(
    classification: PmAuthenticatedCancelResultKindV1,
) -> PmAuthenticatedJournalRecovery {
    let scope = test_scope();
    let prepared = cancel_prepared(&scope);
    let venue_order = venue_order(&scope, EXPECTED_ORDER_ID);
    let result = match classification {
        PmAuthenticatedCancelResultKindV1::Accepted => {
            PmAuthenticatedCancelResultV1::accepted(coordinator(&scope), venue_order, 2, [0x55; 32])
        }
        PmAuthenticatedCancelResultKindV1::Rejected => {
            PmAuthenticatedCancelResultV1::rejected(coordinator(&scope), venue_order, 2, [0x55; 32])
        }
        PmAuthenticatedCancelResultKindV1::DefinitelyNotDispatched => {
            PmAuthenticatedCancelResultV1::definitely_not_dispatched(
                coordinator(&scope),
                venue_order,
                2,
            )
        }
        PmAuthenticatedCancelResultKindV1::OutOfProfile => {
            PmAuthenticatedCancelResultV1::out_of_profile(
                coordinator(&scope),
                venue_order,
                2,
                Some([0x66; 32]),
            )
        }
        PmAuthenticatedCancelResultKindV1::AcknowledgementUnknown => {
            PmAuthenticatedCancelResultV1::acknowledgement_unknown(
                coordinator(&scope),
                venue_order,
                2,
                Some([0x55; 32]),
            )
        }
    };
    let mut bytes = Vec::new();
    header(&mut bytes, &scope);
    append(
        &mut bytes,
        &scope,
        1,
        PmAuthenticatedJournalRecordV1::CancelPrepared(prepared),
    );
    append(
        &mut bytes,
        &scope,
        2,
        PmAuthenticatedJournalRecordV1::DispatchAuthorized(
            PmAuthenticatedDispatchAuthorizedV1::from_durable_prepared(prepared.operation, 1),
        ),
    );
    append(
        &mut bytes,
        &scope,
        3,
        PmAuthenticatedJournalRecordV1::CancelResult(result),
    );
    recover_lines(&mut Cursor::new(bytes), &scope).expect("recover classified cancel")
}

fn recover_cancel_result(
    result: PmAuthenticatedCancelResultV1,
) -> Result<PmAuthenticatedJournalRecovery, PmAuthenticatedJournalRecoveryError> {
    let scope = test_scope();
    let prepared = cancel_prepared(&scope);
    let mut bytes = Vec::new();
    header(&mut bytes, &scope);
    append(
        &mut bytes,
        &scope,
        1,
        PmAuthenticatedJournalRecordV1::CancelPrepared(prepared),
    );
    append(
        &mut bytes,
        &scope,
        2,
        PmAuthenticatedJournalRecordV1::DispatchAuthorized(
            PmAuthenticatedDispatchAuthorizedV1::from_durable_prepared(prepared.operation, 1),
        ),
    );
    append(
        &mut bytes,
        &scope,
        3,
        PmAuthenticatedJournalRecordV1::CancelResult(result),
    );
    recover_lines(&mut Cursor::new(bytes), &scope)
}

#[test]
fn prepared_only_is_definitely_not_authorized() {
    let scope = test_scope();
    let prepared = place_prepared(&scope);
    let mut bytes = Vec::new();
    header(&mut bytes, &scope);
    append(
        &mut bytes,
        &scope,
        1,
        PmAuthenticatedJournalRecordV1::PlacePrepared(prepared),
    );

    let recovery = recover_lines(&mut Cursor::new(bytes), &scope).expect("recover prepared tail");
    assert_eq!(recovery.prepared_count(), 1);
    assert_eq!(recovery.prepared_without_authorization_count(), 1);
    assert_eq!(recovery.acknowledgement_unknown_count(), 0);
    assert!(!recovery.requires_reconciliation());
    assert!(recovery.unresolved_operations().is_empty());
    let [prepared_only] = recovery.prepared_without_grant() else {
        panic!("expected one exact prepared-only projection")
    };
    assert_eq!(
        prepared_only.client_order(),
        coordinator(&scope).client_order
    );
    assert_eq!(prepared_only.instrument(), scope.instrument());
    assert_eq!(prepared_only.prior_goal_f_sequence(), 7);
    assert_eq!(prepared_only.prepared_journal_sequence(), 1);
    assert_eq!(
        prepared_only.request_commitment(),
        prepared.request_commitment.bytes()
    );
    assert_eq!(prepared_only.expected_place_order_id(), Some([0x55; 32]));
    assert_eq!(prepared_only.exact_cancel_order_id(), None);
    assert_eq!(prepared_only.exact_cancel_venue_order(), None);
    assert!(prepared_only.definitely_not_sent());
    assert!(!prepared_only.allows_automatic_retry());
}

#[test]
fn durable_authorization_without_result_is_acknowledgement_unknown() {
    let scope = test_scope();
    let prepared = place_prepared(&scope);
    let operation = prepared.operation;
    let mut bytes = Vec::new();
    header(&mut bytes, &scope);
    append(
        &mut bytes,
        &scope,
        1,
        PmAuthenticatedJournalRecordV1::PlacePrepared(prepared),
    );
    append(
        &mut bytes,
        &scope,
        2,
        PmAuthenticatedJournalRecordV1::DispatchAuthorized(
            PmAuthenticatedDispatchAuthorizedV1::from_durable_prepared(operation, 1),
        ),
    );

    let recovery = recover_lines(&mut Cursor::new(bytes), &scope).expect("recover grant tail");
    assert_eq!(recovery.prepared_without_authorization_count(), 0);
    assert_eq!(recovery.acknowledgement_unknown_count(), 1);
    assert!(recovery.requires_reconciliation());
    let [unresolved] = recovery.unresolved_operations() else {
        panic!("expected one exact unresolved place")
    };
    assert_eq!(
        unresolved.kind(),
        PmAuthenticatedUnresolvedOperationKindV1::Place
    );
    assert_eq!(unresolved.client_order(), coordinator(&scope).client_order);
    assert_eq!(unresolved.instrument(), scope.instrument());
    assert_eq!(unresolved.prior_goal_f_sequence(), 7);
    assert_eq!(unresolved.prepared_journal_sequence(), 1);
    assert_eq!(unresolved.grant_journal_sequence(), 2);
    assert_eq!(unresolved.result_journal_sequence(), None);
    assert_eq!(
        unresolved.reason(),
        PmAuthenticatedUnresolvedReasonV1::GrantTail
    );
    assert_eq!(
        unresolved.request_commitment(),
        prepared.request_commitment.bytes()
    );
    assert_eq!(unresolved.expected_place_order_id(), Some([0x55; 32]));
    assert_eq!(unresolved.exact_cancel_order_id(), None);
    assert_eq!(unresolved.exact_cancel_venue_order(), None);
    assert!(unresolved.acknowledgement_unknown());
    assert!(unresolved.may_have_been_sent());
    assert!(unresolved.requires_reconciliation());
    assert!(!unresolved.allows_automatic_resend());
}

#[test]
fn conclusive_place_and_cancel_results_close_their_grants() {
    let scope = test_scope();
    let place = place_prepared(&scope);
    let cancel = cancel_prepared(&scope);
    let mut bytes = Vec::new();
    header(&mut bytes, &scope);
    append(
        &mut bytes,
        &scope,
        1,
        PmAuthenticatedJournalRecordV1::PlacePrepared(place),
    );
    append(
        &mut bytes,
        &scope,
        2,
        PmAuthenticatedJournalRecordV1::DispatchAuthorized(
            PmAuthenticatedDispatchAuthorizedV1::from_durable_prepared(place.operation, 1),
        ),
    );
    append(
        &mut bytes,
        &scope,
        3,
        PmAuthenticatedJournalRecordV1::PlaceResult(PmAuthenticatedPlaceResultV1::accepted(
            coordinator(&scope),
            2,
            [0x55; 32],
        )),
    );
    append(
        &mut bytes,
        &scope,
        4,
        PmAuthenticatedJournalRecordV1::CancelPrepared(cancel),
    );
    append(
        &mut bytes,
        &scope,
        5,
        PmAuthenticatedJournalRecordV1::DispatchAuthorized(
            PmAuthenticatedDispatchAuthorizedV1::from_durable_prepared(cancel.operation, 4),
        ),
    );
    append(
        &mut bytes,
        &scope,
        6,
        PmAuthenticatedJournalRecordV1::CancelResult(PmAuthenticatedCancelResultV1::accepted(
            coordinator(&scope),
            venue_order(&scope, EXPECTED_ORDER_ID),
            5,
            [0x55; 32],
        )),
    );

    let recovery = recover_lines(&mut Cursor::new(bytes), &scope).expect("recover results");
    assert_eq!(recovery.record_count(), 7);
    assert_eq!(recovery.prepared_count(), 2);
    assert_eq!(recovery.conclusive_result_count(), 2);
    assert_eq!(recovery.acknowledgement_unknown_count(), 0);
    assert!(!recovery.requires_reconciliation());
    assert!(recovery.unresolved_operations().is_empty());
}

#[test]
fn every_place_result_classification_has_an_exact_read_only_projection() {
    for classification in [
        PmAuthenticatedPlaceResultKindV1::Accepted,
        PmAuthenticatedPlaceResultKindV1::Rejected,
        PmAuthenticatedPlaceResultKindV1::DefinitelyNotDispatched,
        PmAuthenticatedPlaceResultKindV1::OutOfProfile,
        PmAuthenticatedPlaceResultKindV1::AcknowledgementUnknown,
    ] {
        let recovery = recover_one_place_result(classification);
        let [result] = recovery.classified_results() else {
            panic!("expected one classified place result")
        };
        assert_eq!(
            result.classification(),
            PmAuthenticatedRecoveredResultClassificationV1::Place(classification)
        );
        let scope = test_scope();
        let prepared = place_prepared(&scope);
        assert_eq!(result.client_order(), coordinator(&scope).client_order);
        assert_eq!(result.instrument(), scope.instrument());
        assert_eq!(result.exact_cancel_venue_order(), None);
        assert_eq!(result.prior_goal_f_sequence(), 7);
        assert_eq!(result.prepared_journal_sequence(), 1);
        assert_eq!(result.grant_journal_sequence(), 2);
        assert_eq!(result.result_journal_sequence(), 3);
        assert_eq!(
            result.request_commitment(),
            prepared.request_commitment.bytes()
        );
        assert_eq!(result.expected_place_order_id(), Some([0x55; 32]));
        assert_eq!(result.exact_cancel_order_id(), None);
        let expected_observed = match classification {
            PmAuthenticatedPlaceResultKindV1::Accepted => Some([0x55; 32]),
            PmAuthenticatedPlaceResultKindV1::OutOfProfile => Some([0x66; 32]),
            PmAuthenticatedPlaceResultKindV1::AcknowledgementUnknown => Some([0x66; 32]),
            PmAuthenticatedPlaceResultKindV1::Rejected
            | PmAuthenticatedPlaceResultKindV1::DefinitelyNotDispatched => None,
        };
        assert_eq!(result.observed_place_order_id(), expected_observed);

        let requires_reconciliation = matches!(
            classification,
            PmAuthenticatedPlaceResultKindV1::OutOfProfile
                | PmAuthenticatedPlaceResultKindV1::AcknowledgementUnknown
        );
        assert_eq!(recovery.requires_reconciliation(), requires_reconciliation);
        assert_eq!(
            recovery.conclusive_result_count(),
            usize::from(!requires_reconciliation)
        );
        if requires_reconciliation {
            let [unresolved] = recovery.unresolved_operations() else {
                panic!("ambiguous place result must remain unresolved")
            };
            assert_eq!(unresolved.result_journal_sequence(), Some(3));
            assert!(!unresolved.allows_automatic_resend());
        } else {
            assert!(recovery.unresolved_operations().is_empty());
        }
    }
}

#[test]
fn every_cancel_result_classification_has_an_exact_read_only_projection() {
    for classification in [
        PmAuthenticatedCancelResultKindV1::Accepted,
        PmAuthenticatedCancelResultKindV1::Rejected,
        PmAuthenticatedCancelResultKindV1::DefinitelyNotDispatched,
        PmAuthenticatedCancelResultKindV1::OutOfProfile,
        PmAuthenticatedCancelResultKindV1::AcknowledgementUnknown,
    ] {
        let recovery = recover_one_cancel_result(classification);
        let [result] = recovery.classified_results() else {
            panic!("expected one classified cancel result")
        };
        assert_eq!(
            result.classification(),
            PmAuthenticatedRecoveredResultClassificationV1::Cancel(classification)
        );
        let scope = test_scope();
        let prepared = cancel_prepared(&scope);
        assert_eq!(result.client_order(), coordinator(&scope).client_order);
        assert_eq!(result.instrument(), scope.instrument());
        assert_eq!(
            result.exact_cancel_venue_order(),
            Some(venue_order(&scope, EXPECTED_ORDER_ID))
        );
        assert_eq!(result.prior_goal_f_sequence(), 8);
        assert_eq!(result.prepared_journal_sequence(), 1);
        assert_eq!(result.grant_journal_sequence(), 2);
        assert_eq!(result.result_journal_sequence(), 3);
        assert_eq!(
            result.request_commitment(),
            prepared.request_commitment.bytes()
        );
        assert_eq!(result.expected_place_order_id(), None);
        assert_eq!(result.exact_cancel_order_id(), Some([0x55; 32]));
        assert_eq!(result.observed_place_order_id(), None);
        let expected_observed = match classification {
            PmAuthenticatedCancelResultKindV1::Accepted
            | PmAuthenticatedCancelResultKindV1::Rejected
            | PmAuthenticatedCancelResultKindV1::AcknowledgementUnknown => Some([0x55; 32]),
            PmAuthenticatedCancelResultKindV1::OutOfProfile => Some([0x66; 32]),
            PmAuthenticatedCancelResultKindV1::DefinitelyNotDispatched => None,
        };
        assert_eq!(result.observed_cancel_order_id(), expected_observed);

        let requires_reconciliation = matches!(
            classification,
            PmAuthenticatedCancelResultKindV1::OutOfProfile
                | PmAuthenticatedCancelResultKindV1::AcknowledgementUnknown
        );
        assert_eq!(recovery.requires_reconciliation(), requires_reconciliation);
        if requires_reconciliation {
            let [unresolved] = recovery.unresolved_operations() else {
                panic!("ambiguous cancel result must remain unresolved")
            };
            assert_eq!(unresolved.result_journal_sequence(), Some(3));
            assert!(!unresolved.allows_automatic_resend());
        } else {
            assert!(recovery.unresolved_operations().is_empty());
        }
    }
}

#[test]
fn typed_unknown_result_stays_reconciliation_required() {
    let scope = test_scope();
    let prepared = place_prepared(&scope);
    let mut bytes = Vec::new();
    header(&mut bytes, &scope);
    append(
        &mut bytes,
        &scope,
        1,
        PmAuthenticatedJournalRecordV1::PlacePrepared(prepared),
    );
    append(
        &mut bytes,
        &scope,
        2,
        PmAuthenticatedJournalRecordV1::DispatchAuthorized(
            PmAuthenticatedDispatchAuthorizedV1::from_durable_prepared(prepared.operation, 1),
        ),
    );
    append(
        &mut bytes,
        &scope,
        3,
        PmAuthenticatedJournalRecordV1::PlaceResult(
            PmAuthenticatedPlaceResultV1::acknowledgement_unknown(
                coordinator(&scope),
                2,
                Some([0x66; 32]),
            ),
        ),
    );

    let recovery = recover_lines(&mut Cursor::new(bytes), &scope).expect("recover unknown result");
    assert_eq!(recovery.acknowledgement_unknown_count(), 1);
    assert!(recovery.requires_reconciliation());
    let [unresolved] = recovery.unresolved_operations() else {
        panic!("expected one explicit unknown place")
    };
    assert_eq!(unresolved.result_journal_sequence(), Some(3));
    assert_eq!(
        unresolved.reason(),
        PmAuthenticatedUnresolvedReasonV1::AcknowledgementUnknown
    );
}

#[test]
fn unresolved_projection_is_in_durable_grant_order_with_exact_place_and_cancel_identity() {
    let scope = test_scope();
    let place = place_prepared(&scope);
    let cancel = cancel_prepared(&scope);
    let mut bytes = Vec::new();
    header(&mut bytes, &scope);
    append(
        &mut bytes,
        &scope,
        1,
        PmAuthenticatedJournalRecordV1::PlacePrepared(place),
    );
    append(
        &mut bytes,
        &scope,
        2,
        PmAuthenticatedJournalRecordV1::CancelPrepared(cancel),
    );
    append(
        &mut bytes,
        &scope,
        3,
        PmAuthenticatedJournalRecordV1::DispatchAuthorized(
            PmAuthenticatedDispatchAuthorizedV1::from_durable_prepared(cancel.operation, 2),
        ),
    );
    append(
        &mut bytes,
        &scope,
        4,
        PmAuthenticatedJournalRecordV1::DispatchAuthorized(
            PmAuthenticatedDispatchAuthorizedV1::from_durable_prepared(place.operation, 1),
        ),
    );
    append(
        &mut bytes,
        &scope,
        5,
        PmAuthenticatedJournalRecordV1::PlaceResult(
            PmAuthenticatedPlaceResultV1::acknowledgement_unknown(coordinator(&scope), 4, None),
        ),
    );

    let recovery = recover_lines(&mut Cursor::new(bytes), &scope).expect("recover two grants");
    let [cancel_unresolved, place_unresolved] = recovery.unresolved_operations() else {
        panic!("expected both unresolved operations")
    };

    assert_eq!(
        cancel_unresolved.kind(),
        PmAuthenticatedUnresolvedOperationKindV1::Cancel
    );
    assert_eq!(cancel_unresolved.grant_journal_sequence(), 3);
    assert_eq!(cancel_unresolved.prepared_journal_sequence(), 2);
    assert_eq!(cancel_unresolved.prior_goal_f_sequence(), 8);
    assert_eq!(
        cancel_unresolved.request_commitment(),
        cancel.request_commitment.bytes()
    );
    assert_eq!(cancel_unresolved.expected_place_order_id(), None);
    assert_eq!(cancel_unresolved.exact_cancel_order_id(), Some([0x55; 32]));
    assert_eq!(
        cancel_unresolved.exact_cancel_venue_order(),
        Some(venue_order(&scope, EXPECTED_ORDER_ID))
    );
    assert_eq!(cancel_unresolved.result_journal_sequence(), None);

    assert_eq!(
        place_unresolved.kind(),
        PmAuthenticatedUnresolvedOperationKindV1::Place
    );
    assert_eq!(place_unresolved.grant_journal_sequence(), 4);
    assert_eq!(place_unresolved.prepared_journal_sequence(), 1);
    assert_eq!(place_unresolved.prior_goal_f_sequence(), 7);
    assert_eq!(
        place_unresolved.request_commitment(),
        place.request_commitment.bytes()
    );
    assert_eq!(place_unresolved.expected_place_order_id(), Some([0x55; 32]));
    assert_eq!(place_unresolved.exact_cancel_order_id(), None);
    assert_eq!(place_unresolved.result_journal_sequence(), Some(5));
}

#[test]
fn corrupted_prepared_commitment_never_becomes_reconciliation_evidence() {
    let scope = test_scope();
    let prepared = place_prepared(&scope);
    let mut bytes = Vec::new();
    header(&mut bytes, &scope);
    append(
        &mut bytes,
        &scope,
        1,
        PmAuthenticatedJournalRecordV1::PlacePrepared(prepared),
    );
    append(
        &mut bytes,
        &scope,
        2,
        PmAuthenticatedJournalRecordV1::DispatchAuthorized(
            PmAuthenticatedDispatchAuthorizedV1::from_durable_prepared(prepared.operation, 1),
        ),
    );
    let encoded_commitment =
        serde_json::to_string(&prepared.request_commitment).expect("serialize request commitment");
    let corrupted_commitment = format!("\"{}\"", "99".repeat(32));
    let encoded = String::from_utf8(bytes).expect("journal UTF-8");
    let corrupted = encoded.replacen(&encoded_commitment, &corrupted_commitment, 1);
    assert_ne!(corrupted, encoded);

    let recovery = recover_lines(&mut Cursor::new(corrupted.into_bytes()), &scope);
    assert!(
        matches!(
            recovery,
            Err(PmAuthenticatedJournalRecoveryError::Schema(
                PmAuthenticatedJournalSchemaError::RequestCommitmentMismatch
            ))
        ),
        "unexpected corruption classification: {recovery:?}"
    );
}

#[test]
fn recovered_evidence_cannot_authorize_a_duplicate_dispatch() {
    let scope = test_scope();
    let prepared = place_prepared(&scope);
    let mut bytes = Vec::new();
    header(&mut bytes, &scope);
    append(
        &mut bytes,
        &scope,
        1,
        PmAuthenticatedJournalRecordV1::PlacePrepared(prepared),
    );
    append(
        &mut bytes,
        &scope,
        2,
        PmAuthenticatedJournalRecordV1::DispatchAuthorized(
            PmAuthenticatedDispatchAuthorizedV1::from_durable_prepared(prepared.operation, 1),
        ),
    );
    append(
        &mut bytes,
        &scope,
        3,
        PmAuthenticatedJournalRecordV1::DispatchAuthorized(
            PmAuthenticatedDispatchAuthorizedV1::from_durable_prepared(prepared.operation, 1),
        ),
    );

    assert!(matches!(
        recover_lines(&mut Cursor::new(bytes), &scope),
        Err(PmAuthenticatedJournalRecoveryError::InvalidAuthorizationTransition)
    ));
}

#[test]
fn accepted_place_must_equal_the_signed_expected_order_id() {
    let scope = test_scope();
    let prepared = place_prepared(&scope);
    let mut bytes = Vec::new();
    header(&mut bytes, &scope);
    append(
        &mut bytes,
        &scope,
        1,
        PmAuthenticatedJournalRecordV1::PlacePrepared(prepared),
    );
    append(
        &mut bytes,
        &scope,
        2,
        PmAuthenticatedJournalRecordV1::DispatchAuthorized(
            PmAuthenticatedDispatchAuthorizedV1::from_durable_prepared(prepared.operation, 1),
        ),
    );
    append(
        &mut bytes,
        &scope,
        3,
        PmAuthenticatedJournalRecordV1::PlaceResult(PmAuthenticatedPlaceResultV1::accepted(
            coordinator(&scope),
            2,
            [0x66; 32],
        )),
    );

    assert!(matches!(
        recover_lines(&mut Cursor::new(bytes), &scope),
        Err(PmAuthenticatedJournalRecoveryError::AcceptedOrderIdentityMismatch)
    ));
}

#[test]
fn conclusive_cancel_results_require_the_exact_fixed_order_identity() {
    let scope = test_scope();
    let order = venue_order(&scope, EXPECTED_ORDER_ID);
    for result in [
        PmAuthenticatedCancelResultV1::accepted(coordinator(&scope), order, 2, [0x66; 32]),
        PmAuthenticatedCancelResultV1::rejected(coordinator(&scope), order, 2, [0x66; 32]),
    ] {
        assert!(matches!(
            recover_cancel_result(result),
            Err(PmAuthenticatedJournalRecoveryError::Schema(
                PmAuthenticatedJournalSchemaError::InvalidResultShape
            ))
        ));
    }

    let invalid_definitely_unsent = PmAuthenticatedCancelResultV1 {
        operation: PmAuthenticatedOperationKeyV1::cancel(coordinator(&scope), order),
        grant_sequence: 2,
        outcome: PmAuthenticatedCancelResultKindV1::DefinitelyNotDispatched,
        observed_order_id: Some(PmAuthenticatedCommitmentV1::from_bytes([0x55; 32])),
    };
    assert!(matches!(
        recover_cancel_result(invalid_definitely_unsent),
        Err(PmAuthenticatedJournalRecoveryError::Schema(
            PmAuthenticatedJournalSchemaError::InvalidResultShape
        ))
    ));
}

#[test]
fn ambiguous_cancel_results_preserve_optional_and_foreign_observed_identities() {
    let scope = test_scope();
    let order = venue_order(&scope, EXPECTED_ORDER_ID);
    let cases = [
        (
            PmAuthenticatedCancelResultV1::out_of_profile(coordinator(&scope), order, 2, None),
            None,
        ),
        (
            PmAuthenticatedCancelResultV1::acknowledgement_unknown(
                coordinator(&scope),
                order,
                2,
                Some([0x66; 32]),
            ),
            Some([0x66; 32]),
        ),
    ];
    for (result, expected_observed) in cases {
        let recovery = recover_cancel_result(result).expect("recover ambiguous cancel identity");
        let [result] = recovery.classified_results() else {
            panic!("one classified cancel result")
        };
        assert_eq!(result.exact_cancel_order_id(), Some([0x55; 32]));
        assert_eq!(result.observed_cancel_order_id(), expected_observed);
        assert!(recovery.requires_reconciliation());
    }
}

#[test]
fn exact_request_commitment_binds_timestamp_and_semantic_request_commitment() {
    let scope = test_scope();
    let first = place_prepared(&scope);
    let changed_timestamp = PmAuthenticatedPlacePreparedV1::test_new(
        &scope,
        coordinator(&scope),
        7,
        [0x33; 32],
        [0x55; 32],
        L2_TIMESTAMP + 1,
    );
    let changed_semantic = PmAuthenticatedPlacePreparedV1::test_new(
        &scope,
        coordinator(&scope),
        7,
        [0x34; 32],
        [0x55; 32],
        L2_TIMESTAMP,
    );

    assert_ne!(
        first.request_commitment,
        changed_timestamp.request_commitment
    );
    assert_ne!(
        first.request_commitment,
        changed_semantic.request_commitment
    );
}

#[test]
fn serialized_family_is_distinct_and_contains_no_secret_or_full_body() {
    let scope = test_scope();
    let mut bytes = Vec::new();
    header(&mut bytes, &scope);
    append(
        &mut bytes,
        &scope,
        1,
        PmAuthenticatedJournalRecordV1::PlacePrepared(place_prepared(&scope)),
    );
    let text = String::from_utf8(bytes).expect("journal UTF-8");

    assert!(text.contains(PM_AUTHENTICATED_JOURNAL_FAMILY));
    assert!(text.contains("semantic_request_commitment"));
    assert!(!text.contains("body_commitment"));
    assert!(!text.contains("reap-pm-mutation-journal\""));
    for forbidden in [
        "synthetic-private-key",
        "synthetic-api-key",
        "synthetic-passphrase",
        "POLY_SIGNATURE",
        "exact_body",
        "auth_headers",
    ] {
        assert!(!text.contains(forbidden), "leaked {forbidden}");
    }
}

#[test]
fn legacy_body_commitment_field_fails_closed() {
    let scope = test_scope();

    let mut place = serde_json::to_value(place_prepared(&scope)).expect("place JSON");
    let place = place.as_object_mut().expect("place object");
    let semantic = place
        .remove("semantic_request_commitment")
        .expect("semantic place commitment");
    place.insert("body_commitment".to_owned(), semantic);
    assert!(
        serde_json::from_value::<PmAuthenticatedPlacePreparedV1>(serde_json::Value::Object(
            place.clone()
        ))
        .is_err()
    );

    let mut cancel = serde_json::to_value(cancel_prepared(&scope)).expect("cancel JSON");
    let cancel = cancel.as_object_mut().expect("cancel object");
    let semantic = cancel
        .remove("semantic_request_commitment")
        .expect("semantic cancel commitment");
    cancel.insert("body_commitment".to_owned(), semantic);
    assert!(
        serde_json::from_value::<PmAuthenticatedCancelPreparedV1>(serde_json::Value::Object(
            cancel.clone()
        ))
        .is_err()
    );
}

#[test]
fn production_journal_sources_have_no_runtime_body_identity_escape() {
    let schema = include_str!("schema.rs");
    let journal = include_str!("../authenticated_journal.rs");
    for source in [schema, journal] {
        for forbidden in [
            "body_commitment",
            "RuntimeExactBodyCommitment",
            "runtime_only_bytes",
        ] {
            assert!(
                !source.contains(forbidden),
                "authenticated journal production source contains forbidden {forbidden}"
            );
        }
    }
}

#[test]
fn v1_scope_bytes_roundtrip_and_bind_the_explicit_credential_slot() {
    let scope = test_scope();
    let mut bytes = Vec::new();
    header(&mut bytes, &scope);
    let exact = bytes.clone();
    let line: PmAuthenticatedJournalLineV1 =
        serde_json::from_slice(&exact[..exact.len() - 1]).expect("decode exact header");
    let mut reencoded = serde_json::to_vec(&line).expect("re-encode exact header");
    reencoded.push(b'\n');
    assert_eq!(reencoded, exact);

    let text = String::from_utf8(exact.clone()).expect("header UTF-8");
    assert_eq!(text, FROZEN_PRE_PM_T2_V1_HEADER_LINE);
    assert!(text.starts_with(&format!(
        "[\"{PM_AUTHENTICATED_JOURNAL_FAMILY}\",{PM_AUTHENTICATED_JOURNAL_VERSION},"
    )));
    assert!(!text.contains("account_signature_profile"));
    assert!(text.contains(&format!(
        "\"credential_slot_fingerprint\":\"{}\"",
        "44".repeat(32)
    )));
    assert!(text.contains("\"production_order_entry_authorized\":false"));

    let changed_scope = test_scope_with_credential_slot([0x45; 32]);
    assert!(matches!(
        recover_lines(&mut Cursor::new(exact), &changed_scope),
        Err(PmAuthenticatedJournalRecoveryError::ScopeMismatch)
    ));
}

#[test]
fn authenticated_scope_version_and_profile_are_derived_only_from_checked_config() {
    let eoa_config = crate::evidence::connectivity_config();
    let proxy_config = proxy_connectivity_config();
    let eoa = PmAuthenticatedJournalScopeV1::from_config(&eoa_config, [0x44; 32])
        .expect("EOA authenticated scope");
    let proxy = PmAuthenticatedJournalScopeV1::from_config(&proxy_config, [0x44; 32])
        .expect("proxy authenticated scope");

    assert_eq!(eoa.schema_version(), PM_AUTHENTICATED_JOURNAL_VERSION);
    assert_eq!(
        eoa.account_signature_profile(),
        PmAccountSignatureProfile::EoaType0
    );
    assert_eq!(
        proxy.schema_version(),
        PM_T2_PROXY_AUTHENTICATED_JOURNAL_VERSION
    );
    assert_eq!(
        proxy.account_signature_profile(),
        PmAccountSignatureProfile::ProxyType1
    );
    assert_eq!(
        eoa.configuration_fingerprint(),
        proxy.configuration_fingerprint(),
        "the public market/config identity is intentionally shared"
    );
    assert_ne!(eoa.fingerprint(), proxy.fingerprint());
}

#[test]
fn proxy_scope_uses_truthful_v2_domain_and_roundtrips_exactly() {
    let scope = test_proxy_scope();
    assert_eq!(
        scope.account_signature_profile(),
        PmAccountSignatureProfile::ProxyType1
    );
    assert_ne!(
        scope.account_scope().signer().address(),
        scope.account_scope().funder().address()
    );

    let mut bytes = Vec::new();
    header(&mut bytes, &scope);
    append(
        &mut bytes,
        &scope,
        1,
        PmAuthenticatedJournalRecordV1::PlacePrepared(place_prepared(&scope)),
    );
    append(
        &mut bytes,
        &scope,
        2,
        PmAuthenticatedJournalRecordV1::CancelPrepared(cancel_prepared(&scope)),
    );
    let exact = bytes.clone();
    let text = String::from_utf8(exact.clone()).expect("proxy journal UTF-8");
    assert!(text.starts_with(&format!(
        "[\"{PM_AUTHENTICATED_JOURNAL_FAMILY}\",{PM_T2_PROXY_AUTHENTICATED_JOURNAL_VERSION},"
    )));
    assert!(text.contains("\"account_signature_profile\":\"proxy_type_1\""));
    assert!(text.contains("\"production_order_entry_authorized\":false"));

    let recovery = recover_lines(&mut Cursor::new(exact.clone()), &scope)
        .expect("recover proxy v2 prepared tail");
    assert_eq!(recovery.prepared_count(), 2);
    assert_eq!(recovery.prepared_without_authorization_count(), 2);

    let mut reencoded = Vec::new();
    for line in exact.split_inclusive(|byte| *byte == b'\n') {
        let decoded: PmAuthenticatedJournalLineV1 =
            serde_json::from_slice(&line[..line.len() - 1]).expect("decode proxy v2 line");
        serde_json::to_writer(&mut reencoded, &decoded).expect("re-encode proxy v2 line");
        reencoded.push(b'\n');
    }
    assert_eq!(reencoded, exact);
}

#[test]
fn proxy_v2_line_version_and_profile_mismatches_reject() {
    let scope = test_proxy_scope();
    let mut bytes = Vec::new();
    header(&mut bytes, &scope);
    let value: serde_json::Value =
        serde_json::from_slice(&bytes[..bytes.len() - 1]).expect("proxy header JSON");

    let mut wrong_line_version = value.clone();
    wrong_line_version[1] = serde_json::json!(PM_AUTHENTICATED_JOURNAL_VERSION);
    let mut wrong_line_version =
        serde_json::to_vec(&wrong_line_version).expect("wrong line version JSON");
    wrong_line_version.push(b'\n');
    assert!(matches!(
        recover_lines(&mut Cursor::new(wrong_line_version), &scope),
        Err(PmAuthenticatedJournalRecoveryError::ScopeMismatch)
    ));

    let mut wrong_scope_version = value.clone();
    wrong_scope_version[4]["header"]["scope"]["schema_version"] =
        serde_json::json!(PM_AUTHENTICATED_JOURNAL_VERSION);
    let mut wrong_scope_version =
        serde_json::to_vec(&wrong_scope_version).expect("wrong scope version JSON");
    wrong_scope_version.push(b'\n');
    assert!(matches!(
        recover_lines(&mut Cursor::new(wrong_scope_version), &scope),
        Err(PmAuthenticatedJournalRecoveryError::Schema(
            PmAuthenticatedJournalSchemaError::WrongScopeDomain
        ))
    ));

    let mut wrong_profile = value;
    wrong_profile[4]["header"]["scope"]["account_signature_profile"] =
        serde_json::json!("eoa_type_0");
    let mut wrong_profile = serde_json::to_vec(&wrong_profile).expect("wrong profile JSON");
    wrong_profile.push(b'\n');
    assert!(matches!(
        recover_lines(&mut Cursor::new(wrong_profile), &scope),
        Err(PmAuthenticatedJournalRecoveryError::Json(_))
    ));
}

#[test]
fn recovery_rejects_cross_profile_signer_funder_and_configuration_scopes() {
    let scope = test_proxy_scope();
    let mut bytes = Vec::new();
    header(&mut bytes, &scope);

    for mismatched in [
        test_scope(),
        test_proxy_scope_with_identities([0x13; 20], [0x12; 20], [0x33; 32]),
        test_proxy_scope_with_identities([0x11; 20], [0x13; 20], [0x33; 32]),
        test_proxy_scope_with_identities([0x11; 20], [0x12; 20], [0x34; 32]),
    ] {
        assert!(matches!(
            recover_lines(&mut Cursor::new(bytes.clone()), &mismatched),
            Err(PmAuthenticatedJournalRecoveryError::ScopeMismatch)
        ));
    }
}

#[test]
fn proxy_scope_profile_and_split_identities_are_fingerprint_bound() {
    let scope = test_proxy_scope();
    let mut bytes = Vec::new();
    header(&mut bytes, &scope);
    let value: serde_json::Value =
        serde_json::from_slice(&bytes[..bytes.len() - 1]).expect("proxy header JSON");

    for field in ["account_scope", "configuration_fingerprint"] {
        let mut changed = value.clone();
        let header_scope = changed[4]["header"]["scope"]
            .as_object_mut()
            .expect("proxy scope object");
        match field {
            "account_scope" => {
                header_scope["account_scope"]["funder"] =
                    serde_json::json!("0x1313131313131313131313131313131313131313");
            }
            "configuration_fingerprint" => {
                header_scope["configuration_fingerprint"] =
                    serde_json::Value::String("34".repeat(32));
            }
            _ => unreachable!(),
        }
        let mut changed_bytes = serde_json::to_vec(&changed).expect("changed proxy header");
        changed_bytes.push(b'\n');
        assert!(matches!(
            recover_lines(&mut Cursor::new(changed_bytes), &scope),
            Err(PmAuthenticatedJournalRecoveryError::Schema(
                PmAuthenticatedJournalSchemaError::ScopeFingerprintMismatch
            ))
        ));
    }

    let mut production = serde_json::from_slice::<serde_json::Value>(&bytes[..bytes.len() - 1])
        .expect("proxy header JSON");
    production[4]["header"]["scope"]["production_order_entry_authorized"] = serde_json::json!(true);
    let mut production = serde_json::to_vec(&production).expect("production header JSON");
    production.push(b'\n');
    assert!(matches!(
        recover_lines(&mut Cursor::new(production), &scope),
        Err(PmAuthenticatedJournalRecoveryError::Schema(
            PmAuthenticatedJournalSchemaError::ProductionAuthorityForbidden
        ))
    ));
}

#[test]
fn altered_or_missing_credential_scope_evidence_fails_closed() {
    let scope = test_scope();
    let mut bytes = Vec::new();
    header(&mut bytes, &scope);
    let text = String::from_utf8(bytes).expect("header UTF-8");
    let altered = text.replacen(&"44".repeat(32), &"45".repeat(32), 1);
    assert_ne!(altered, text);
    assert!(matches!(
        recover_lines(&mut Cursor::new(altered.into_bytes()), &scope),
        Err(PmAuthenticatedJournalRecoveryError::Schema(
            PmAuthenticatedJournalSchemaError::ScopeFingerprintMismatch
        ))
    ));

    let mut value: serde_json::Value =
        serde_json::from_str(text.trim_end()).expect("header JSON value");
    let header_scope = value
        .get_mut(4)
        .and_then(|record| record.get_mut("header"))
        .and_then(|header| header.get_mut("scope"))
        .and_then(serde_json::Value::as_object_mut)
        .expect("exact header scope shape");
    assert!(header_scope.remove("credential_slot_fingerprint").is_some());
    let mut missing = serde_json::to_vec(&value).expect("encode missing credential scope");
    missing.push(b'\n');
    assert!(matches!(
        recover_lines(&mut Cursor::new(missing), &scope),
        Err(PmAuthenticatedJournalRecoveryError::Json(_))
    ));
}

#[test]
fn result_shape_is_exact_and_unknown_fields_fail_closed() {
    let scope = test_scope();
    let prepared = place_prepared(&scope);
    let result = PmAuthenticatedJournalLineV1::new(
        scope.fingerprint(),
        3,
        PmAuthenticatedJournalRecordV1::PlaceResult(PmAuthenticatedPlaceResultV1::accepted(
            coordinator(&scope),
            2,
            [0x55; 32],
        )),
    );
    let mut invalid_shape = serde_json::to_value(&result).expect("result JSON value");
    let result_fields = invalid_shape
        .get_mut(4)
        .and_then(|record| record.get_mut("place_result"))
        .and_then(serde_json::Value::as_object_mut)
        .expect("exact result shape");
    result_fields.insert("observed_order_id".to_owned(), serde_json::Value::Null);

    let mut bytes = Vec::new();
    header(&mut bytes, &scope);
    append(
        &mut bytes,
        &scope,
        1,
        PmAuthenticatedJournalRecordV1::PlacePrepared(prepared),
    );
    append(
        &mut bytes,
        &scope,
        2,
        PmAuthenticatedJournalRecordV1::DispatchAuthorized(
            PmAuthenticatedDispatchAuthorizedV1::from_durable_prepared(prepared.operation, 1),
        ),
    );
    serde_json::to_writer(&mut bytes, &invalid_shape).expect("encode invalid shape");
    bytes.push(b'\n');
    assert!(matches!(
        recover_lines(&mut Cursor::new(bytes), &scope),
        Err(PmAuthenticatedJournalRecoveryError::Schema(
            PmAuthenticatedJournalSchemaError::InvalidResultShape
        ))
    ));

    let mut unknown = serde_json::to_value(result).expect("result JSON value");
    unknown[4]["place_result"]["unexpected"] = serde_json::json!(true);
    let mut bytes = Vec::new();
    header(&mut bytes, &scope);
    append(
        &mut bytes,
        &scope,
        1,
        PmAuthenticatedJournalRecordV1::PlacePrepared(prepared),
    );
    append(
        &mut bytes,
        &scope,
        2,
        PmAuthenticatedJournalRecordV1::DispatchAuthorized(
            PmAuthenticatedDispatchAuthorizedV1::from_durable_prepared(prepared.operation, 1),
        ),
    );
    serde_json::to_writer(&mut bytes, &unknown).expect("encode unknown result field");
    bytes.push(b'\n');
    assert!(matches!(
        recover_lines(&mut Cursor::new(bytes), &scope),
        Err(PmAuthenticatedJournalRecoveryError::Json(_))
    ));
}

#[test]
fn out_of_profile_place_preserves_a_different_observed_identity_for_reconciliation() {
    let scope = test_scope();
    let prepared = place_prepared(&scope);
    let mut bytes = Vec::new();
    header(&mut bytes, &scope);
    append(
        &mut bytes,
        &scope,
        1,
        PmAuthenticatedJournalRecordV1::PlacePrepared(prepared),
    );
    append(
        &mut bytes,
        &scope,
        2,
        PmAuthenticatedJournalRecordV1::DispatchAuthorized(
            PmAuthenticatedDispatchAuthorizedV1::from_durable_prepared(prepared.operation, 1),
        ),
    );
    append(
        &mut bytes,
        &scope,
        3,
        PmAuthenticatedJournalRecordV1::PlaceResult(PmAuthenticatedPlaceResultV1::out_of_profile(
            coordinator(&scope),
            2,
            Some([0x66; 32]),
        )),
    );
    let recovery = recover_lines(&mut Cursor::new(bytes), &scope).expect("recover contradiction");
    let [result] = recovery.classified_results() else {
        panic!("one out-of-profile result")
    };
    assert_eq!(result.expected_place_order_id(), Some([0x55; 32]));
    assert_eq!(result.observed_place_order_id(), Some([0x66; 32]));
    assert!(recovery.requires_reconciliation());
}

#[test]
fn out_of_profile_place_without_observed_identity_remains_reconcilable() {
    let scope = test_scope();
    let prepared = place_prepared(&scope);
    let mut bytes = Vec::new();
    header(&mut bytes, &scope);
    append(
        &mut bytes,
        &scope,
        1,
        PmAuthenticatedJournalRecordV1::PlacePrepared(prepared),
    );
    append(
        &mut bytes,
        &scope,
        2,
        PmAuthenticatedJournalRecordV1::DispatchAuthorized(
            PmAuthenticatedDispatchAuthorizedV1::from_durable_prepared(prepared.operation, 1),
        ),
    );
    append(
        &mut bytes,
        &scope,
        3,
        PmAuthenticatedJournalRecordV1::PlaceResult(PmAuthenticatedPlaceResultV1::out_of_profile(
            coordinator(&scope),
            2,
            None,
        )),
    );
    let recovery = recover_lines(&mut Cursor::new(bytes), &scope).expect("recover absent identity");
    let [result] = recovery.classified_results() else {
        panic!("one out-of-profile result")
    };
    assert_eq!(result.expected_place_order_id(), Some([0x55; 32]));
    assert_eq!(result.observed_place_order_id(), None);
    assert!(recovery.requires_reconciliation());
}

#[test]
fn send_grants_have_no_generic_acknowledgement_escape_or_public_constructor() {
    let source = include_str!("../authenticated_journal.rs");
    assert!(!source.contains("PmAuthenticatedDispatchAcknowledged"));
    assert!(!source.contains("struct PmAuthenticatedAcknowledged"));
    assert!(source.contains("fn from_durable_dispatch("));
    assert!(!source.contains("pub(crate) fn from_durable_dispatch("));
    assert!(source.contains("pub(crate) fn try_authorize_dispatch("));
    assert!(source.contains("pub(crate) fn try_record_place_result("));
    assert!(source.contains("pub(crate) fn try_record_cancel_result("));
    assert!(source.contains("DispatchAuthorizationRequiresDurablePrepared"));
    assert!(source.contains("ResultRequiresExactDurableGrant"));
    assert!(source.contains("kind: PmAuthenticatedPendingKind::PlaceResult { grant, result }"));
    assert!(source.contains("kind: PmAuthenticatedPendingKind::CancelResult { grant, result }"));
    assert!(!source.contains("impl Clone for PmAuthenticatedPlaceSendGrant"));
    assert!(!source.contains("impl Clone for PmAuthenticatedCancelSendGrant"));
    assert!(!source.contains("impl Clone for PmAuthenticatedPlaceResultAcknowledged"));
    assert!(!source.contains("impl Clone for PmAuthenticatedCancelResultAcknowledged"));
    assert!(!source.contains("impl Clone for PmAuthenticatedPlaceCompletionGrant"));
    assert!(!source.contains("impl Clone for PmAuthenticatedCancelCompletionGrant"));
    assert!(source.contains("PmAuthenticatedPlaceCompletionGrant { grant }"));
    assert!(source.contains("PmAuthenticatedCancelCompletionGrant { grant }"));
    assert!(source.contains("struct PmAuthenticatedJournalRuntimeIdentity;"));
    assert_eq!(
        source.matches("Arc::ptr_eq(&self.runtime_identity").count(),
        3,
        "prepared authorization and both typed result paths must reject cross-runtime proofs",
    );
    assert!(!source.contains("impl Clone for PmAuthenticatedPreparedAcknowledged"));
    assert!(!source.contains("impl Clone for PmAuthenticatedJournalRuntimeIdentity"));
}

async fn acknowledge_cancel_result(
    mut pending: PmAuthenticatedPendingRecord,
) -> PmAuthenticatedCancelResultAcknowledged {
    for _ in 0..10_000 {
        match pending.poll() {
            PmAuthenticatedReceiptPoll::Pending(next) => {
                pending = next;
                tokio::task::yield_now().await;
            }
            PmAuthenticatedReceiptPoll::CancelResultAcknowledged(acknowledged) => {
                return acknowledged;
            }
            PmAuthenticatedReceiptPoll::PlaceResultAcknowledged(_) => {
                panic!("cancel result acknowledgement changed operation kind")
            }
            PmAuthenticatedReceiptPoll::SendGranted(_) => {
                panic!("expected a result acknowledgement")
            }
            PmAuthenticatedReceiptPoll::PreparedAcknowledged(_) => {
                panic!("expected a non-preparation acknowledgement")
            }
            PmAuthenticatedReceiptPoll::Failed(message) => {
                panic!("journal durability failed: {message}")
            }
            PmAuthenticatedReceiptPoll::Closed => panic!("journal writer closed"),
            PmAuthenticatedReceiptPoll::CancelResultFailed { message, .. } => {
                panic!("cancel-result durability failed: {message}")
            }
            PmAuthenticatedReceiptPoll::CancelResultClosed(_) => {
                panic!("cancel-result writer closed")
            }
            PmAuthenticatedReceiptPoll::PlaceResultFailed { .. }
            | PmAuthenticatedReceiptPoll::PlaceResultClosed(_) => {
                panic!("cancel result acknowledgement changed operation kind")
            }
        }
    }
    panic!("journal acknowledgement did not arrive")
}

async fn acknowledge_place_result(
    mut pending: PmAuthenticatedPendingRecord,
) -> PmAuthenticatedPlaceResultAcknowledged {
    for _ in 0..10_000 {
        match pending.poll() {
            PmAuthenticatedReceiptPoll::Pending(next) => {
                pending = next;
                tokio::task::yield_now().await;
            }
            PmAuthenticatedReceiptPoll::PlaceResultAcknowledged(acknowledged) => {
                return acknowledged;
            }
            PmAuthenticatedReceiptPoll::CancelResultAcknowledged(_) => {
                panic!("place result acknowledgement changed operation kind")
            }
            PmAuthenticatedReceiptPoll::SendGranted(_) => {
                panic!("expected a result acknowledgement")
            }
            PmAuthenticatedReceiptPoll::PreparedAcknowledged(_) => {
                panic!("expected a non-preparation acknowledgement")
            }
            PmAuthenticatedReceiptPoll::Failed(message) => {
                panic!("journal durability failed: {message}")
            }
            PmAuthenticatedReceiptPoll::Closed => panic!("journal writer closed"),
            PmAuthenticatedReceiptPoll::PlaceResultFailed { message, .. } => {
                panic!("place-result durability failed: {message}")
            }
            PmAuthenticatedReceiptPoll::PlaceResultClosed(_) => {
                panic!("place-result writer closed")
            }
            PmAuthenticatedReceiptPoll::CancelResultFailed { .. }
            | PmAuthenticatedReceiptPoll::CancelResultClosed(_) => {
                panic!("place result acknowledgement changed operation kind")
            }
        }
    }
    panic!("journal acknowledgement did not arrive")
}

async fn acknowledge_prepared(
    mut pending: PmAuthenticatedPendingRecord,
) -> PmAuthenticatedPreparedAcknowledged {
    for _ in 0..10_000 {
        match pending.poll() {
            PmAuthenticatedReceiptPoll::Pending(next) => {
                pending = next;
                tokio::task::yield_now().await;
            }
            PmAuthenticatedReceiptPoll::PreparedAcknowledged(acknowledged) => {
                return acknowledged;
            }
            PmAuthenticatedReceiptPoll::SendGranted(_)
            | PmAuthenticatedReceiptPoll::PlaceResultAcknowledged(_)
            | PmAuthenticatedReceiptPoll::CancelResultAcknowledged(_)
            | PmAuthenticatedReceiptPoll::PlaceResultFailed { .. }
            | PmAuthenticatedReceiptPoll::CancelResultFailed { .. }
            | PmAuthenticatedReceiptPoll::PlaceResultClosed(_)
            | PmAuthenticatedReceiptPoll::CancelResultClosed(_) => {
                panic!("expected a prepared acknowledgement")
            }
            PmAuthenticatedReceiptPoll::Failed(message) => {
                panic!("journal durability failed: {message}")
            }
            PmAuthenticatedReceiptPoll::Closed => panic!("journal writer closed"),
        }
    }
    panic!("journal acknowledgement did not arrive")
}

async fn acknowledge_send_grant(
    mut pending: PmAuthenticatedPendingRecord,
) -> PmAuthenticatedSendGrant {
    for _ in 0..10_000 {
        match pending.poll() {
            PmAuthenticatedReceiptPoll::Pending(next) => {
                pending = next;
                tokio::task::yield_now().await;
            }
            PmAuthenticatedReceiptPoll::SendGranted(grant) => return grant,
            PmAuthenticatedReceiptPoll::PreparedAcknowledged(_)
            | PmAuthenticatedReceiptPoll::PlaceResultAcknowledged(_)
            | PmAuthenticatedReceiptPoll::CancelResultAcknowledged(_)
            | PmAuthenticatedReceiptPoll::PlaceResultFailed { .. }
            | PmAuthenticatedReceiptPoll::CancelResultFailed { .. }
            | PmAuthenticatedReceiptPoll::PlaceResultClosed(_)
            | PmAuthenticatedReceiptPoll::CancelResultClosed(_) => {
                panic!("expected a typed send grant")
            }
            PmAuthenticatedReceiptPoll::Failed(message) => {
                panic!("journal durability failed: {message}")
            }
            PmAuthenticatedReceiptPoll::Closed => panic!("journal writer closed"),
        }
    }
    panic!("journal acknowledgement did not arrive")
}

#[tokio::test]
async fn leased_runtime_persists_a_may_have_sent_barrier() {
    let directory = tempfile::tempdir().expect("temporary directory");
    let path = directory.path().join("authenticated.jsonl");
    let scope = test_scope();
    let (mut journal, initial) = PmAuthenticatedMutationJournal::start(path.clone(), scope.clone())
        .await
        .expect("start journal");
    assert_eq!(initial.record_count(), 0);

    let prepared = place_prepared(&scope);
    let prepared_acknowledged = acknowledge_prepared(
        journal
            .try_record(PmAuthenticatedJournalRecordV1::PlacePrepared(prepared))
            .expect("record prepared"),
    )
    .await;
    let prepared_sequence = prepared_acknowledged.sequence();
    assert_eq!(prepared_sequence, 1);
    let grant = acknowledge_send_grant(
        journal
            .try_authorize_dispatch(prepared_acknowledged)
            .expect("record grant"),
    )
    .await;
    let grant_sequence = match grant {
        PmAuthenticatedSendGrant::Place(grant) => {
            assert_eq!(grant.prepared_sequence(), prepared_sequence);
            assert_eq!(grant.grant_sequence(), 2);
            assert_eq!(grant.client_order(), coordinator(&scope).client_order);
            assert_eq!(grant.instrument(), scope.instrument());
            assert_eq!(
                grant.prior_goal_f_sequence(),
                prepared.prior_intent_sequence
            );
            assert_eq!(
                grant.request_commitment(),
                prepared.request_commitment.bytes()
            );
            assert_eq!(grant.expected_order_id(), [0x55; 32]);
            assert_eq!(grant.l2_timestamp_seconds(), prepared.l2_timestamp_seconds);
            assert!(grant.matches_retained_request(
                prepared.semantic_request_commitment.bytes(),
                [0x55; 32],
                prepared.l2_timestamp_seconds,
            ));
            assert!(!grant.matches_retained_request(
                [0x99; 32],
                [0x55; 32],
                prepared.l2_timestamp_seconds,
            ));
            assert!(!grant.matches_retained_request(
                prepared.semantic_request_commitment.bytes(),
                [0x99; 32],
                prepared.l2_timestamp_seconds,
            ));
            assert!(!grant.matches_retained_request(
                prepared.semantic_request_commitment.bytes(),
                [0x55; 32],
                prepared.l2_timestamp_seconds + 1,
            ));
            assert!(format!("{grant:?}").contains("[REDACTED]"));
            grant.grant_sequence()
        }
        PmAuthenticatedSendGrant::Cancel(_) => panic!("place grant changed operation kind"),
    };
    journal.shutdown().await.expect("clean journal shutdown");

    let recovery = recover_pm_authenticated_journal(path, &scope).expect("recover journal");
    assert_eq!(recovery.record_count(), 3);
    assert_eq!(recovery.last_sequence(), 2);
    assert_eq!(recovery.acknowledgement_unknown_count(), 1);
    assert!(recovery.requires_reconciliation());
    let [unresolved] = recovery.unresolved_operations() else {
        panic!("expected the restarted exact place obligation")
    };
    assert_eq!(unresolved.prepared_journal_sequence(), prepared_sequence);
    assert_eq!(unresolved.grant_journal_sequence(), grant_sequence);
    assert_eq!(
        unresolved.request_commitment(),
        prepared.request_commitment.bytes()
    );
    assert_eq!(unresolved.expected_place_order_id(), Some([0x55; 32]));
}

#[tokio::test]
async fn durable_cancel_grant_retains_exact_correlation_and_result_barrier() {
    let directory = tempfile::tempdir().expect("temporary directory");
    let path = directory.path().join("authenticated-cancel.jsonl");
    let scope = test_scope();
    let (mut journal, _) = PmAuthenticatedMutationJournal::start(path.clone(), scope.clone())
        .await
        .expect("start journal");
    let prepared = cancel_prepared(&scope);
    let prepared_acknowledged = acknowledge_prepared(
        journal
            .try_record(PmAuthenticatedJournalRecordV1::CancelPrepared(prepared))
            .expect("record cancel prepared"),
    )
    .await;
    let prepared_sequence = prepared_acknowledged.sequence();
    let grant = acknowledge_send_grant(
        journal
            .try_authorize_dispatch(prepared_acknowledged)
            .expect("record cancel grant"),
    )
    .await;
    let grant = match grant {
        PmAuthenticatedSendGrant::Cancel(grant) => {
            assert_eq!(grant.prepared_sequence(), prepared_sequence);
            assert_eq!(grant.grant_sequence(), 2);
            assert_eq!(grant.client_order(), coordinator(&scope).client_order);
            assert_eq!(grant.instrument(), scope.instrument());
            assert_eq!(grant.venue_order(), venue_order(&scope, EXPECTED_ORDER_ID));
            assert_eq!(
                grant.prior_goal_f_sequence(),
                prepared.prior_cancel_sequence
            );
            assert_eq!(
                grant.request_commitment(),
                prepared.request_commitment.bytes()
            );
            assert_eq!(grant.fixed_order_id(), [0x55; 32]);
            assert_eq!(grant.l2_timestamp_seconds(), prepared.l2_timestamp_seconds);
            assert!(grant.matches_retained_request(
                prepared.semantic_request_commitment.bytes(),
                [0x55; 32],
                prepared.l2_timestamp_seconds,
            ));
            assert!(!grant.matches_retained_request(
                [0x99; 32],
                [0x55; 32],
                prepared.l2_timestamp_seconds,
            ));
            assert!(!grant.matches_retained_request(
                prepared.semantic_request_commitment.bytes(),
                [0x99; 32],
                prepared.l2_timestamp_seconds,
            ));
            assert!(!grant.matches_retained_request(
                prepared.semantic_request_commitment.bytes(),
                [0x55; 32],
                prepared.l2_timestamp_seconds + 1,
            ));
            assert!(format!("{grant:?}").contains("[REDACTED]"));
            grant
        }
        PmAuthenticatedSendGrant::Place(_) => panic!("cancel grant changed operation kind"),
    };
    let grant_sequence = grant.grant_sequence();
    let exact_result = PmAuthenticatedCancelResultV1::definitely_not_dispatched(
        coordinator(&scope),
        venue_order(&scope, EXPECTED_ORDER_ID),
        grant_sequence,
    );
    let acknowledged = acknowledge_cancel_result(
        journal
            .try_record_cancel_result(grant, exact_result)
            .expect("record cancel result"),
    )
    .await;
    assert_eq!(acknowledged.sequence(), 3);
    assert!(format!("{acknowledged:?}").contains("[REDACTED]"));
    let (durable_grant, result_sequence, durable_result) = acknowledged.into_parts();
    assert_eq!(durable_grant.grant_sequence(), grant_sequence);
    assert_eq!(result_sequence, 3);
    assert_eq!(durable_result, exact_result);
    assert_eq!(
        durable_result.client_order(),
        coordinator(&scope).client_order
    );
    assert_eq!(durable_result.instrument(), scope.instrument());
    assert_eq!(
        durable_result.venue_order(),
        venue_order(&scope, EXPECTED_ORDER_ID)
    );
    assert_eq!(durable_result.grant_sequence(), grant_sequence);
    assert_eq!(
        durable_result.outcome(),
        PmAuthenticatedCancelResultKindV1::DefinitelyNotDispatched
    );
    assert_eq!(durable_result.observed_order_id(), None);
    journal.shutdown().await.expect("clean journal shutdown");

    let recovery = recover_pm_authenticated_journal(path, &scope).expect("recover cancel journal");
    let [result] = recovery.classified_results() else {
        panic!("expected classified cancel result")
    };
    assert_eq!(result.prepared_journal_sequence(), prepared_sequence);
    assert_eq!(result.grant_journal_sequence(), grant_sequence);
    assert_eq!(result.result_journal_sequence(), result_sequence);
    assert_eq!(
        result.classification(),
        PmAuthenticatedRecoveredResultClassificationV1::Cancel(
            PmAuthenticatedCancelResultKindV1::DefinitelyNotDispatched
        )
    );
    assert!(!recovery.requires_reconciliation());
}

#[tokio::test]
async fn durable_place_result_acknowledgement_retains_the_exact_classification() {
    let directory = tempfile::tempdir().expect("temporary directory");
    let path = directory.path().join("authenticated-place-result.jsonl");
    let scope = test_scope();
    let (mut journal, _) = PmAuthenticatedMutationJournal::start(path, scope.clone())
        .await
        .expect("start journal");
    let prepared = place_prepared(&scope);
    let prepared_acknowledged = acknowledge_prepared(
        journal
            .try_record(PmAuthenticatedJournalRecordV1::PlacePrepared(prepared))
            .expect("record place prepared"),
    )
    .await;
    let grant = acknowledge_send_grant(
        journal
            .try_authorize_dispatch(prepared_acknowledged)
            .expect("record place grant"),
    )
    .await;
    let grant = match grant {
        PmAuthenticatedSendGrant::Place(grant) => grant,
        PmAuthenticatedSendGrant::Cancel(_) => panic!("place grant changed operation kind"),
    };
    let grant_sequence = grant.grant_sequence();
    let exact_result =
        PmAuthenticatedPlaceResultV1::accepted(coordinator(&scope), grant_sequence, [0x55; 32]);
    let acknowledged = acknowledge_place_result(
        journal
            .try_record_place_result(grant, exact_result)
            .expect("record place result"),
    )
    .await;
    assert_eq!(acknowledged.sequence(), 3);
    assert!(format!("{acknowledged:?}").contains("[REDACTED]"));
    let (durable_grant, sequence, durable_result) = acknowledged.into_parts();
    assert_eq!(durable_grant.grant_sequence(), grant_sequence);
    assert_eq!(sequence, 3);
    assert_eq!(durable_result, exact_result);
    assert_eq!(
        durable_result.client_order(),
        coordinator(&scope).client_order
    );
    assert_eq!(durable_result.instrument(), scope.instrument());
    assert_eq!(durable_result.grant_sequence(), grant_sequence);
    assert_eq!(
        durable_result.outcome(),
        PmAuthenticatedPlaceResultKindV1::Accepted
    );
    assert_eq!(durable_result.observed_order_id(), Some([0x55; 32]));
    journal.shutdown().await.expect("clean journal shutdown");
}

#[tokio::test]
async fn generic_record_path_cannot_mint_a_send_grant_or_append_a_result() {
    let directory = tempfile::tempdir().expect("temporary directory");
    let path = directory
        .path()
        .join("authenticated-no-generic-grant.jsonl");
    let scope = test_scope();
    let (mut journal, _) = PmAuthenticatedMutationJournal::start(path, scope.clone())
        .await
        .expect("start journal");
    let prepared = place_prepared(&scope);
    let acknowledged = acknowledge_prepared(
        journal
            .try_record(PmAuthenticatedJournalRecordV1::PlacePrepared(prepared))
            .expect("record place prepared"),
    )
    .await;
    let forged_record = PmAuthenticatedJournalRecordV1::DispatchAuthorized(
        PmAuthenticatedDispatchAuthorizedV1::from_durable_prepared(
            prepared.operation,
            acknowledged.sequence(),
        ),
    );
    assert!(matches!(
        journal.try_record(forged_record),
        Err(PmAuthenticatedJournalError::DispatchAuthorizationRequiresDurablePrepared)
    ));
    assert!(matches!(
        journal.try_record(PmAuthenticatedJournalRecordV1::PlaceResult(
            PmAuthenticatedPlaceResultV1::accepted(
                coordinator(&scope),
                acknowledged.sequence() + 1,
                [0x55; 32],
            ),
        )),
        Err(PmAuthenticatedJournalError::ResultRequiresExactDurableGrant)
    ));
    journal.shutdown().await.expect("clean journal shutdown");
}

#[tokio::test]
async fn place_result_mismatch_returns_the_sole_grant_without_consuming_a_sequence() {
    let directory = tempfile::tempdir().expect("temporary directory");
    let path = directory.path().join("authenticated-retaining-error.jsonl");
    let scope = test_scope();
    let (mut journal, _) = PmAuthenticatedMutationJournal::start(path.clone(), scope.clone())
        .await
        .expect("start journal");
    let prepared = place_prepared(&scope);
    let prepared = acknowledge_prepared(
        journal
            .try_record(PmAuthenticatedJournalRecordV1::PlacePrepared(prepared))
            .expect("record place prepared"),
    )
    .await;
    let grant = match acknowledge_send_grant(
        journal
            .try_authorize_dispatch(prepared)
            .expect("record place grant"),
    )
    .await
    {
        PmAuthenticatedSendGrant::Place(grant) => grant,
        PmAuthenticatedSendGrant::Cancel(_) => panic!("place grant changed operation kind"),
    };
    let grant_sequence = grant.grant_sequence();
    let mismatched =
        PmAuthenticatedPlaceResultV1::accepted(coordinator(&scope), grant_sequence, [0x66; 32]);
    let failure = match journal.try_record_place_result(grant, mismatched) {
        Ok(_) => panic!("mismatched accepted identity reached the journal writer"),
        Err(failure) => failure,
    };
    assert!(matches!(
        failure.error(),
        PmAuthenticatedJournalError::ResultDoesNotMatchDurableGrant
    ));
    let diagnostic = format!("{failure:?}");
    assert!(diagnostic.contains("ResultDoesNotMatchDurableGrant"));
    assert!(diagnostic.contains("[REDACTED]"));
    let (error, grant, returned_result) = failure.into_parts();
    assert!(matches!(
        error,
        PmAuthenticatedJournalError::ResultDoesNotMatchDurableGrant
    ));
    assert_eq!(grant.grant_sequence(), grant_sequence);
    assert_eq!(returned_result, mismatched);

    let exact =
        PmAuthenticatedPlaceResultV1::accepted(coordinator(&scope), grant_sequence, [0x55; 32]);
    let acknowledged = acknowledge_place_result(
        journal
            .try_record_place_result(grant, exact)
            .expect("retained grant admits the exact result"),
    )
    .await;
    assert_eq!(acknowledged.sequence(), 3);
    let (grant, sequence, durable_result) = acknowledged.into_parts();
    assert_eq!(grant.grant_sequence(), grant_sequence);
    assert_eq!(sequence, 3);
    assert_eq!(durable_result, exact);
    journal.shutdown().await.expect("clean journal shutdown");

    let recovery = recover_pm_authenticated_journal(path, &scope).expect("recover exact journal");
    assert_eq!(recovery.last_sequence(), 3);
    assert_eq!(recovery.classified_results().len(), 1);
}

#[tokio::test]
async fn closed_result_queue_returns_the_exact_grant_and_classification() {
    let directory = tempfile::tempdir().expect("temporary directory");
    let path = directory
        .path()
        .join("authenticated-closed-result-queue.jsonl");
    let scope = test_scope();
    let (mut journal, _) = PmAuthenticatedMutationJournal::start(path, scope.clone())
        .await
        .expect("start journal");
    let prepared = acknowledge_prepared(
        journal
            .try_record(PmAuthenticatedJournalRecordV1::PlacePrepared(
                place_prepared(&scope),
            ))
            .expect("record place prepared"),
    )
    .await;
    let grant = match acknowledge_send_grant(
        journal
            .try_authorize_dispatch(prepared)
            .expect("record place grant"),
    )
    .await
    {
        PmAuthenticatedSendGrant::Place(grant) => grant,
        PmAuthenticatedSendGrant::Cancel(_) => panic!("place grant changed operation kind"),
    };
    let result = PmAuthenticatedPlaceResultV1::acknowledgement_unknown(
        coordinator(&scope),
        grant.grant_sequence(),
        None,
    );
    journal
        .runtime
        .stop_writer()
        .await
        .expect("close writer before admission");
    let failure = match journal.try_record_place_result(grant, result) {
        Ok(_) => panic!("closed journal accepted a classified result"),
        Err(failure) => failure,
    };
    assert!(matches!(
        failure.error(),
        PmAuthenticatedJournalError::WriterClosed
    ));
    let (error, grant, returned_result) = failure.into_parts();
    assert!(matches!(error, PmAuthenticatedJournalError::WriterClosed));
    assert_eq!(grant.grant_sequence(), 2);
    assert_eq!(returned_result, result);
    journal.shutdown().await.expect("idempotent clean shutdown");
}

#[tokio::test]
async fn failed_result_receipt_seals_the_exact_place_grant_and_classification() {
    let directory = tempfile::tempdir().expect("temporary directory");
    let path = directory
        .path()
        .join("authenticated-result-write-failure.jsonl");
    let scope = test_scope();
    let (mut journal, _) = PmAuthenticatedMutationJournal::start(path, scope.clone())
        .await
        .expect("start journal");
    let prepared = acknowledge_prepared(
        journal
            .try_record(PmAuthenticatedJournalRecordV1::PlacePrepared(
                place_prepared(&scope),
            ))
            .expect("record place prepared"),
    )
    .await;
    let grant = match acknowledge_send_grant(
        journal
            .try_authorize_dispatch(prepared)
            .expect("record place grant"),
    )
    .await
    {
        PmAuthenticatedSendGrant::Place(grant) => grant,
        PmAuthenticatedSendGrant::Cancel(_) => panic!("place grant changed operation kind"),
    };
    let client_order = grant.client_order();
    let grant_sequence = grant.grant_sequence();
    let observed = [0x66; 32];
    let result = PmAuthenticatedPlaceResultV1::acknowledgement_unknown(
        coordinator(&scope),
        grant_sequence,
        Some(observed),
    );
    journal.fail_next_write_for_test();
    let mut pending = journal
        .try_record_place_result(grant, result)
        .expect("admit place result before injected writer failure");
    let (message, unresolved) = loop {
        match pending.poll() {
            PmAuthenticatedReceiptPoll::Pending(next) => {
                pending = next;
                tokio::task::yield_now().await;
            }
            PmAuthenticatedReceiptPoll::PlaceResultFailed {
                message,
                unresolved,
            } => break (message, unresolved),
            _ => panic!("injected place-result failure changed terminal kind"),
        }
    };
    assert!(message.contains("injected authenticated journal codec failure"));
    assert_eq!(unresolved.sequence(), 3);
    assert_eq!(unresolved.grant_sequence(), grant_sequence);
    assert_eq!(unresolved.client_order(), client_order);
    assert_eq!(
        unresolved.outcome(),
        PmAuthenticatedPlaceResultKindV1::AcknowledgementUnknown
    );
    assert_eq!(unresolved.observed_order_id(), Some(observed));
    assert!(format!("{unresolved:?}").contains("[REDACTED]"));
    assert!(matches!(
        journal.shutdown().await,
        Err(PmAuthenticatedJournalError::Writer(_))
    ));
}

#[tokio::test]
async fn closed_result_receipt_seals_the_exact_cancel_grant_and_classification() {
    let directory = tempfile::tempdir().expect("temporary directory");
    let path = directory
        .path()
        .join("authenticated-result-writer-close.jsonl");
    let scope = test_scope();
    let (mut journal, _) = PmAuthenticatedMutationJournal::start(path, scope.clone())
        .await
        .expect("start journal");
    let prepared = acknowledge_prepared(
        journal
            .try_record(PmAuthenticatedJournalRecordV1::CancelPrepared(
                cancel_prepared(&scope),
            ))
            .expect("record cancel prepared"),
    )
    .await;
    let grant = match acknowledge_send_grant(
        journal
            .try_authorize_dispatch(prepared)
            .expect("record cancel grant"),
    )
    .await
    {
        PmAuthenticatedSendGrant::Cancel(grant) => grant,
        PmAuthenticatedSendGrant::Place(_) => panic!("cancel grant changed operation kind"),
    };
    let client_order = grant.client_order();
    let venue_order = grant.venue_order();
    let grant_sequence = grant.grant_sequence();
    let observed = [0x66; 32];
    let result = PmAuthenticatedCancelResultV1::out_of_profile(
        coordinator(&scope),
        venue_order,
        grant_sequence,
        Some(observed),
    );
    journal.close_next_write_for_test();
    let mut pending = journal
        .try_record_cancel_result(grant, result)
        .expect("admit cancel result before injected writer close");
    let unresolved = loop {
        match pending.poll() {
            PmAuthenticatedReceiptPoll::Pending(next) => {
                pending = next;
                tokio::task::yield_now().await;
            }
            PmAuthenticatedReceiptPoll::CancelResultClosed(unresolved) => break unresolved,
            _ => panic!("injected cancel-result close changed terminal kind"),
        }
    };
    assert_eq!(unresolved.sequence(), 3);
    assert_eq!(unresolved.grant_sequence(), grant_sequence);
    assert_eq!(unresolved.client_order(), client_order);
    assert_eq!(
        unresolved.outcome(),
        PmAuthenticatedCancelResultKindV1::OutOfProfile
    );
    assert_eq!(unresolved.observed_order_id(), Some(observed));
    assert!(format!("{unresolved:?}").contains("[REDACTED]"));
    drop(journal);
}

#[test]
fn unresolved_result_evidence_has_no_reappend_or_resend_escape() {
    let source = include_str!("../authenticated_journal.rs");
    for type_name in [
        "PmAuthenticatedPlaceResultUnresolved",
        "PmAuthenticatedCancelResultUnresolved",
    ] {
        assert!(!source.contains(&format!("impl Clone for {type_name}")));
        let start = source
            .find(&format!("impl {type_name} {{"))
            .expect("unresolved evidence implementation exists");
        let tail = &source[start..];
        let end = tail
            .find("impl std::fmt::Debug")
            .expect("unresolved evidence has redacted Debug boundary");
        let implementation = &tail[..end];
        assert!(!implementation.contains("into_parts"));
        assert!(!implementation.contains("send("));
        assert!(!implementation.contains("try_record"));
    }
}

#[tokio::test]
async fn cancel_result_requires_the_grant_operation_but_preserves_foreign_observation() {
    let directory = tempfile::tempdir().expect("temporary directory");
    let path = directory
        .path()
        .join("authenticated-cancel-correlation.jsonl");
    let scope = test_scope();
    let (mut journal, _) = PmAuthenticatedMutationJournal::start(path.clone(), scope.clone())
        .await
        .expect("start journal");
    let prepared = cancel_prepared(&scope);
    let prepared = acknowledge_prepared(
        journal
            .try_record(PmAuthenticatedJournalRecordV1::CancelPrepared(prepared))
            .expect("record cancel prepared"),
    )
    .await;
    let grant = match acknowledge_send_grant(
        journal
            .try_authorize_dispatch(prepared)
            .expect("record cancel grant"),
    )
    .await
    {
        PmAuthenticatedSendGrant::Cancel(grant) => grant,
        PmAuthenticatedSendGrant::Place(_) => panic!("cancel grant changed operation kind"),
    };
    let grant_sequence = grant.grant_sequence();
    let wrong_venue = venue_order(
        &scope,
        "0x7777777777777777777777777777777777777777777777777777777777777777",
    );
    let mismatched = PmAuthenticatedCancelResultV1::out_of_profile(
        coordinator(&scope),
        wrong_venue,
        grant_sequence,
        Some([0x66; 32]),
    );
    let failure = match journal.try_record_cancel_result(grant, mismatched) {
        Ok(_) => panic!("foreign cancel operation reached the journal writer"),
        Err(failure) => failure,
    };
    assert!(matches!(
        failure.error(),
        PmAuthenticatedJournalError::ResultDoesNotMatchDurableGrant
    ));
    assert!(format!("{failure:?}").contains("[REDACTED]"));
    let (error, grant, returned_result) = failure.into_parts();
    assert!(matches!(
        error,
        PmAuthenticatedJournalError::ResultDoesNotMatchDurableGrant
    ));
    assert_eq!(returned_result, mismatched);

    let exact_operation_with_foreign_observation = PmAuthenticatedCancelResultV1::out_of_profile(
        coordinator(&scope),
        venue_order(&scope, EXPECTED_ORDER_ID),
        grant_sequence,
        Some([0x66; 32]),
    );
    let acknowledged = acknowledge_cancel_result(
        journal
            .try_record_cancel_result(grant, exact_operation_with_foreign_observation)
            .expect("out-of-profile foreign observation remains durable evidence"),
    )
    .await;
    assert_eq!(acknowledged.sequence(), 3);
    let (grant, sequence, durable_result) = acknowledged.into_parts();
    assert_eq!(grant.grant_sequence(), grant_sequence);
    assert_eq!(sequence, 3);
    assert_eq!(durable_result, exact_operation_with_foreign_observation);
    journal.shutdown().await.expect("clean journal shutdown");

    let recovery = recover_pm_authenticated_journal(path, &scope).expect("recover exact journal");
    let [result] = recovery.classified_results() else {
        panic!("one classified cancel result")
    };
    assert_eq!(result.observed_cancel_order_id(), Some([0x66; 32]));
    assert!(recovery.requires_reconciliation());
}

#[tokio::test]
async fn prepared_acknowledgement_cannot_cross_an_equal_scope_runtime() {
    let directory = tempfile::tempdir().expect("temporary directory");
    let scope = test_scope();
    let (mut first, _) = PmAuthenticatedMutationJournal::start(
        directory.path().join("first-authenticated.jsonl"),
        scope.clone(),
    )
    .await
    .expect("start first journal");
    let (mut second, _) = PmAuthenticatedMutationJournal::start(
        directory.path().join("second-authenticated.jsonl"),
        scope.clone(),
    )
    .await
    .expect("start second journal");
    let foreign = acknowledge_prepared(
        first
            .try_record(PmAuthenticatedJournalRecordV1::PlacePrepared(
                place_prepared(&scope),
            ))
            .expect("record first prepared"),
    )
    .await;
    let local = acknowledge_prepared(
        second
            .try_record(PmAuthenticatedJournalRecordV1::PlacePrepared(
                place_prepared(&scope),
            ))
            .expect("record second prepared"),
    )
    .await;
    assert_eq!(second.next_sequence, 2);
    assert!(matches!(
        second.try_authorize_dispatch(foreign),
        Err(PmAuthenticatedJournalError::DurableProofFromDifferentRuntime)
    ));
    assert_eq!(second.next_sequence, 2);
    assert_eq!(local.sequence(), 1);

    first.shutdown().await.expect("shutdown first journal");
    second.shutdown().await.expect("shutdown second journal");
}

#[tokio::test]
async fn send_grant_cannot_cross_an_equal_scope_runtime_and_is_returned_intact() {
    let directory = tempfile::tempdir().expect("temporary directory");
    let scope = test_scope();
    let (mut first, _) = PmAuthenticatedMutationJournal::start(
        directory.path().join("first-grant.jsonl"),
        scope.clone(),
    )
    .await
    .expect("start first journal");
    let (mut second, _) = PmAuthenticatedMutationJournal::start(
        directory.path().join("second-grant.jsonl"),
        scope.clone(),
    )
    .await
    .expect("start second journal");

    let first_prepared = acknowledge_prepared(
        first
            .try_record(PmAuthenticatedJournalRecordV1::PlacePrepared(
                place_prepared(&scope),
            ))
            .expect("record first prepared"),
    )
    .await;
    let second_prepared = acknowledge_prepared(
        second
            .try_record(PmAuthenticatedJournalRecordV1::PlacePrepared(
                place_prepared(&scope),
            ))
            .expect("record second prepared"),
    )
    .await;
    let foreign_grant = match acknowledge_send_grant(
        first
            .try_authorize_dispatch(first_prepared)
            .expect("record first grant"),
    )
    .await
    {
        PmAuthenticatedSendGrant::Place(grant) => grant,
        PmAuthenticatedSendGrant::Cancel(_) => panic!("place grant changed operation kind"),
    };
    let local_grant = acknowledge_send_grant(
        second
            .try_authorize_dispatch(second_prepared)
            .expect("record second grant"),
    )
    .await;
    assert!(matches!(local_grant, PmAuthenticatedSendGrant::Place(_)));
    assert_eq!(second.next_sequence, 3);

    let result = PmAuthenticatedPlaceResultV1::accepted(
        coordinator(&scope),
        foreign_grant.grant_sequence(),
        foreign_grant.expected_order_id(),
    );
    let failure = match second.try_record_place_result(foreign_grant, result) {
        Ok(_) => panic!("foreign-runtime grant reached the second writer"),
        Err(failure) => failure,
    };
    assert!(matches!(
        failure.error(),
        PmAuthenticatedJournalError::DurableProofFromDifferentRuntime
    ));
    assert_eq!(second.next_sequence, 3);
    let (error, returned_grant, returned_result) = failure.into_parts();
    assert!(matches!(
        error,
        PmAuthenticatedJournalError::DurableProofFromDifferentRuntime
    ));
    assert_eq!(returned_grant.grant_sequence(), 2);
    assert_eq!(returned_result, result);

    let acknowledged = acknowledge_place_result(
        first
            .try_record_place_result(returned_grant, returned_result)
            .expect("original runtime accepts its exact returned grant"),
    )
    .await;
    assert_eq!(acknowledged.sequence(), 3);
    first.shutdown().await.expect("shutdown first journal");
    second.shutdown().await.expect("shutdown second journal");
}

#[tokio::test]
async fn cancel_send_grant_cannot_cross_an_equal_scope_runtime() {
    let directory = tempfile::tempdir().expect("temporary directory");
    let scope = test_scope();
    let (mut first, _) = PmAuthenticatedMutationJournal::start(
        directory.path().join("first-cancel-grant.jsonl"),
        scope.clone(),
    )
    .await
    .expect("start first journal");
    let (mut second, _) = PmAuthenticatedMutationJournal::start(
        directory.path().join("second-cancel-grant.jsonl"),
        scope.clone(),
    )
    .await
    .expect("start second journal");

    let first_prepared = acknowledge_prepared(
        first
            .try_record(PmAuthenticatedJournalRecordV1::CancelPrepared(
                cancel_prepared(&scope),
            ))
            .expect("record first prepared"),
    )
    .await;
    let second_prepared = acknowledge_prepared(
        second
            .try_record(PmAuthenticatedJournalRecordV1::CancelPrepared(
                cancel_prepared(&scope),
            ))
            .expect("record second prepared"),
    )
    .await;
    let foreign_grant = match acknowledge_send_grant(
        first
            .try_authorize_dispatch(first_prepared)
            .expect("record first grant"),
    )
    .await
    {
        PmAuthenticatedSendGrant::Cancel(grant) => grant,
        PmAuthenticatedSendGrant::Place(_) => panic!("cancel grant changed operation kind"),
    };
    let local_grant = acknowledge_send_grant(
        second
            .try_authorize_dispatch(second_prepared)
            .expect("record second grant"),
    )
    .await;
    assert!(matches!(local_grant, PmAuthenticatedSendGrant::Cancel(_)));
    assert_eq!(second.next_sequence, 3);

    let result = PmAuthenticatedCancelResultV1::acknowledgement_unknown(
        coordinator(&scope),
        foreign_grant.venue_order(),
        foreign_grant.grant_sequence(),
        Some([0x66; 32]),
    );
    let failure = match second.try_record_cancel_result(foreign_grant, result) {
        Ok(_) => panic!("foreign-runtime cancel grant reached the second writer"),
        Err(failure) => failure,
    };
    assert!(matches!(
        failure.error(),
        PmAuthenticatedJournalError::DurableProofFromDifferentRuntime
    ));
    assert_eq!(second.next_sequence, 3);
    let (error, returned_grant, returned_result) = failure.into_parts();
    assert!(matches!(
        error,
        PmAuthenticatedJournalError::DurableProofFromDifferentRuntime
    ));
    assert_eq!(returned_grant.grant_sequence(), 2);
    assert_eq!(returned_result, result);

    let acknowledged = acknowledge_cancel_result(
        first
            .try_record_cancel_result(returned_grant, returned_result)
            .expect("original runtime accepts its exact returned grant"),
    )
    .await;
    assert_eq!(acknowledged.sequence(), 3);
    first.shutdown().await.expect("shutdown first journal");
    second.shutdown().await.expect("shutdown second journal");
}
