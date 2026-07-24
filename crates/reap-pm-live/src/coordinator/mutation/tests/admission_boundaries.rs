use super::*;

#[tokio::test(flavor = "current_thread")]
async fn full_persistence_storage_releases_reserved_quote_effect_before_journal() {
    let (config, _directory, _journal_path, mut owner) = ready_owner().await;
    for attempt in 1..=PM_PENDING_PERSISTENCE_CAPACITY {
        owner
            .persistence
            .push(PmPendingPersistence::Fact {
                receipt: PmPendingJournalRecord::phase6_pending_for_age_evidence(),
                compaction: None,
                enqueued_monotonic_ns: u64::try_from(attempt).unwrap(),
            })
            .unwrap();
    }
    let client = owner.scope.client_order_for_intent(1).unwrap();
    let effects_before = owner.fake_effect_metrics();
    let counters_before = owner.counters();

    assert!(matches!(
        owner.begin_quote(quote_request(
            &config,
            PmOrderSide::Buy,
            1,
            QUOTE_EXPIRES_NS
        )),
        Err(PmMutationError::Persistence(PmPersistenceError::Full))
    ));
    assert_eq!(owner.halt(), Some(PmMutationHalt::PersistenceSaturated));
    assert_eq!(owner.persistence_metrics().depth(), 1_024);
    assert_eq!(owner.persistence_metrics().high_water(), 1_024);
    assert_eq!(owner.persistence_metrics().saturations(), 1);
    assert!(owner.private_mut().owned_order(client).is_none());
    assert_eq!(owner.pending_effects(), 0);
    assert_eq!(owner.retained_effect_permits(), 0);
    let effects_after = owner.fake_effect_metrics();
    assert_eq!(
        effects_after.reservations(),
        effects_before.reservations() + 1
    );
    assert_eq!(
        effects_after.released_before_journal(),
        effects_before.released_before_journal() + 1
    );
    assert_eq!(
        owner.counters().quote_attempts(),
        counters_before.quote_attempts() + 1
    );
    assert_eq!(
        owner.counters().quote_intents(),
        counters_before.quote_intents()
    );
    owner.shutdown().await.unwrap();
}

#[tokio::test(flavor = "current_thread")]
async fn saturated_effect_queue_admits_no_order_and_no_journal_record() {
    let (config, _directory, journal_path, mut owner) = ready_owner().await;
    for _ in 0..PM_FAKE_EFFECT_CAPACITY {
        let _unreachable_permit = owner.effects.try_reserve().unwrap();
    }
    let client = owner.scope.client_order_for_intent(1).unwrap();
    assert!(matches!(
        owner.begin_quote(quote_request(
            &config,
            PmOrderSide::Buy,
            1,
            QUOTE_EXPIRES_NS
        )),
        Err(PmMutationError::EffectQueue(PmFakeEffectQueueError::Full))
    ));
    assert_eq!(owner.halt(), Some(PmMutationHalt::FakeEffectSaturated));
    assert_eq!(owner.pending_persistence(), 0);
    assert_eq!(owner.counters().quote_intents(), 0);
    assert_eq!(owner.counters().fact_records(), READY_BASELINE_FACT_RECORDS);
    assert!(owner.private_mut().owned_order(client).is_none());

    owner.shutdown().await.unwrap();
    let scope = PmJournalScopeV1::from_config(&config).unwrap();
    let recovery = recover_pm_mutation_journal(journal_path, &scope).unwrap();
    assert_eq!(
        recovery.record_count(),
        JOURNAL_HEADER_RECORDS + READY_BASELINE_JOURNAL_RECORDS
    );
}

#[derive(Clone, Copy)]
enum FinalDispatchInvalidation {
    RevisionChanged,
    Expired,
}

async fn assert_prepared_quote_is_invalidated_without_dispatch(case: FinalDispatchInvalidation) {
    let (config, _directory, journal_path, mut owner) = ready_owner().await;
    let client = admit_quote(&mut owner, &config, PmOrderSide::Buy, 1);
    wait_for_prepared_quotes(&mut owner, 1, 121).await;
    let service_ns = match case {
        FinalDispatchInvalidation::RevisionChanged => {
            owner.update_revisions(authority_revisions(2));
            122
        }
        FinalDispatchInvalidation::Expired => QUOTE_EXPIRES_NS,
    };
    assert!(owner.invalidate_prepared_quote(client, service_ns).unwrap());
    assert!(
        !owner
            .invalidate_prepared_quote(client, service_ns + 1)
            .unwrap()
    );
    assert_eq!(owner.halt(), None);
    assert_eq!(owner.pending_effects(), 0);
    assert_eq!(owner.retained_effect_permits(), 0);
    assert_eq!(owner.effects.metrics().invalidated_after_durability(), 1);
    assert_eq!(owner.counters().place_results(), 0);
    assert_eq!(
        owner.counters().fact_records(),
        READY_BASELINE_FACT_RECORDS + 1
    );
    let retained = owner.private_mut().owned_order(client).unwrap();
    assert_eq!(retained.submit(), PmOwnedSubmitState::Rejected);
    assert_eq!(retained.status(), Some(PmOrderStatus::Rejected));
    assert_eq!(retained.venue_order(), None);
    let consequence = owner.pop_durable_consequence().unwrap();
    assert_eq!(consequence.kind(), PmDurableRecordKind::PlaceResult);
    assert_eq!(consequence.client_order(), Some(client));
    assert!(matches!(
        owner.execute_next_quote(
            PmFakePlaceScript::acknowledged(
                venue_order(&config, "must-not-dispatch"),
                Box::new([])
            )
            .unwrap(),
            service_ns + 1,
        ),
        Err(PmMutationError::EffectKindMismatch)
    ));

    drain_persistence(&mut owner, service_ns + 2).await;
    owner.shutdown().await.unwrap();
    let scope = PmJournalScopeV1::from_config(&config).unwrap();
    let recovery = recover_pm_mutation_journal(journal_path, &scope).unwrap();
    assert_eq!(
        recovery.record_count(),
        JOURNAL_HEADER_RECORDS + READY_BASELINE_JOURNAL_RECORDS + 2
    );
    assert_eq!(recovery.owned_order_count(), 1);
    assert_eq!(recovery.compacted_intent_id(), 0);
    assert_eq!(
        recovery.recovered_orders().next().unwrap().place(),
        PmJournalRecoveredPlaceV1::Rejected
    );
}

#[tokio::test(flavor = "current_thread")]
async fn prepared_quote_with_changed_revision_is_rejected_locally_without_dispatch() {
    assert_prepared_quote_is_invalidated_without_dispatch(
        FinalDispatchInvalidation::RevisionChanged,
    )
    .await;
}

#[tokio::test(flavor = "current_thread")]
async fn prepared_quote_at_exact_expiry_is_rejected_locally_without_dispatch() {
    assert_prepared_quote_is_invalidated_without_dispatch(FinalDispatchInvalidation::Expired).await;
}
