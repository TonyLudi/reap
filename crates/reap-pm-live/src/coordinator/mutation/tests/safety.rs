use super::*;
use crate::coordinator::mutation::{
    PmDurableConsequence, PmTerminalSafetyAdmissionFailure, PmTerminalSafetyTransition,
};
use crate::coordinator::persistence::PM_PENDING_PERSISTENCE_CAPACITY;
use crate::coordinator::reduction::PmReductionError;

#[tokio::test(flavor = "current_thread")]
async fn terminal_safety_admits_once_before_live_latch_and_recovers_exact_cause() {
    let (config, _directory, journal_path, mut owner) = ready_owner().await;
    let baseline_facts = owner.counters().fact_records();
    owner.update_revisions(authority_revisions(7));

    assert_eq!(
        owner.enter_terminal_safety(PmJournalSafetyReasonV1::ContractViolation, 120),
        PmTerminalSafetyTransition::Admitted {
            reason: PmJournalSafetyReasonV1::ContractViolation,
        }
    );
    assert_eq!(owner.current_revisions, None);
    assert_eq!(
        owner.halt(),
        Some(PmMutationHalt::LiveSafetyHalt(
            PmJournalSafetyReasonV1::ContractViolation
        ))
    );
    assert_eq!(owner.counters().fact_records(), baseline_facts + 1);
    assert_eq!(owner.pending_persistence(), 1);
    let consequence = owner.pop_durable_consequence().unwrap();
    assert_eq!(consequence.kind(), PmDurableRecordKind::SafetyHalt);
    assert_eq!(consequence.client_order(), None);

    assert_eq!(
        owner.enter_terminal_safety(PmJournalSafetyReasonV1::QueueSaturation, 121),
        PmTerminalSafetyTransition::AlreadyEntered {
            halt: PmMutationHalt::LiveSafetyHalt(PmJournalSafetyReasonV1::ContractViolation),
        }
    );
    assert_eq!(owner.counters().fact_records(), baseline_facts + 1);
    assert_eq!(owner.pending_persistence(), 1);
    assert!(owner.pop_durable_consequence().is_none());

    drain_persistence(&mut owner, 122).await;
    owner.shutdown().await.unwrap();
    let scope = PmJournalScopeV1::from_config(&config).unwrap();
    let recovery = recover_pm_mutation_journal(&journal_path, &scope).unwrap();
    assert_eq!(
        recovery.record_count(),
        JOURNAL_HEADER_RECORDS + READY_BASELINE_JOURNAL_RECORDS + 1
    );
    assert_eq!(
        recovery.safety_reason(),
        Some(PmJournalSafetyReasonV1::ContractViolation)
    );

    let (restarted, recovered) = restart_owner(&config, &journal_path).await;
    assert_eq!(
        restarted.halt(),
        Some(PmMutationHalt::RecoveredSafetyHalt(
            PmJournalSafetyReasonV1::ContractViolation
        ))
    );
    assert_eq!(
        recovered.safety_reason(),
        Some(PmJournalSafetyReasonV1::ContractViolation)
    );
    restarted.shutdown().await.unwrap();
}

#[tokio::test(flavor = "current_thread")]
async fn full_consequence_capacity_is_explicitly_unjournalable_without_false_durability() {
    let (config, _directory, journal_path, mut owner) = ready_owner().await;
    let filler = PmDurableConsequence {
        kind: PmDurableRecordKind::FillApplied,
        client_order: None,
        correlation: 1,
    };
    owner
        .durable_consequences
        .resize(PM_PENDING_PERSISTENCE_CAPACITY, filler);
    let baseline_facts = owner.counters().fact_records();
    owner.update_revisions(authority_revisions(9));

    assert_eq!(
        owner.enter_terminal_safety(PmJournalSafetyReasonV1::QueueSaturation, 120),
        PmTerminalSafetyTransition::Unjournalable {
            reason: PmJournalSafetyReasonV1::QueueSaturation,
            failure: PmTerminalSafetyAdmissionFailure::DurableConsequenceFull,
        }
    );
    assert_eq!(owner.current_revisions, None);
    assert_eq!(
        owner.halt(),
        Some(PmMutationHalt::UnjournalableSafetyHalt(
            PmJournalSafetyReasonV1::QueueSaturation
        ))
    );
    assert_eq!(owner.counters().fact_records(), baseline_facts);
    assert_eq!(owner.pending_persistence(), 0);

    owner.shutdown().await.unwrap();
    let scope = PmJournalScopeV1::from_config(&config).unwrap();
    let recovery = recover_pm_mutation_journal(journal_path, &scope).unwrap();
    assert!(!recovery.safety_halted());
}

#[tokio::test(flavor = "current_thread")]
async fn exact_owned_safety_cancel_remains_available_after_live_terminal_halt() {
    let (config, _directory, _journal_path, mut owner) = ready_owner().await;
    let venue = venue_order(&config, "terminal-safety-cancel");
    let client = place_resting_quote(&mut owner, &config, PmOrderSide::Buy, 1, venue).await;

    assert!(matches!(
        owner.enter_terminal_safety(PmJournalSafetyReasonV1::ContractViolation, 123),
        PmTerminalSafetyTransition::Admitted { .. }
    ));
    assert!(matches!(
        owner.begin_cancel(cancel_request(client)).unwrap(),
        PmCancelMutationAdmission::JournalPending {
            client_order,
            venue_order,
        } if client_order == client && venue_order == venue
    ));

    owner.shutdown().await.unwrap();
}

#[tokio::test(flavor = "current_thread")]
async fn fake_result_unknown_ownership_routes_through_exact_terminal_safety() {
    let (config, _directory, _journal_path, mut owner) = ready_owner().await;
    let unknown = owner.scope.client_order_for_intent(99).unwrap();
    let result = late_ack_result(
        &config,
        unknown,
        venue_order(&config, "unknown-result-owner"),
    )
    .unwrap();

    assert!(matches!(
        owner.reduce_serviced_fake_place(result, 120),
        Err(PmMutationError::Reduction(
            PmReductionError::UnknownOwnedOrder
        ))
    ));
    assert_eq!(
        owner.halt(),
        Some(PmMutationHalt::LiveSafetyHalt(
            PmJournalSafetyReasonV1::UnresolvedOwnership
        ))
    );
    assert_eq!(
        owner.pop_durable_consequence().unwrap().kind(),
        PmDurableRecordKind::SafetyHalt
    );

    drain_persistence(&mut owner, 121).await;
    owner.shutdown().await.unwrap();
}
