use super::*;

#[tokio::test(flavor = "current_thread")]
async fn accepted_resting_binding_does_not_invent_remote_status_before_or_after_restart() {
    let (config, _directory, journal_path, mut owner) = ready_owner().await;
    let venue = venue_order(&config, "accepted-unobserved-remote");
    let client = place_resting_quote(&mut owner, &config, PmOrderSide::Buy, 1, venue).await;

    let runtime_owned = owner.private_mut().owned_order(client).unwrap();
    assert_eq!(runtime_owned.submit(), PmOwnedSubmitState::Accepted);
    assert_eq!(runtime_owned.status(), Some(PmOrderStatus::Open));
    let runtime_remote = owner
        .private_mut()
        .orders()
        .find(|order| order.identity().venue_order_key() == Some(venue))
        .expect("accepted result binds exact canonical venue identity");
    assert_eq!(runtime_remote.ownership(), PmOrderOwnership::ProvenOwned);
    assert_eq!(runtime_remote.status(), None);

    drain_persistence(&mut owner, 123).await;
    owner.shutdown().await.unwrap();

    let (mut restarted, recovery) = restart_owner(&config, &journal_path).await;
    assert_eq!(
        recovery.recovered_orders().next().unwrap().place(),
        PmJournalRecoveredPlaceV1::Bound
    );
    let recovered_owned = restarted.private_mut().owned_order(client).unwrap();
    assert_eq!(recovered_owned.submit(), PmOwnedSubmitState::Accepted);
    assert_eq!(recovered_owned.status(), Some(PmOrderStatus::Open));
    let recovered_remote = restarted
        .private_mut()
        .orders()
        .find(|order| order.identity().venue_order_key() == Some(venue))
        .expect("replay preserves exact canonical venue identity");
    assert_eq!(recovered_remote.ownership(), PmOrderOwnership::ProvenOwned);
    assert_eq!(recovered_remote.status(), None);
    restarted.shutdown().await.unwrap();
}

#[tokio::test(flavor = "current_thread")]
async fn skipped_settlement_step_reconverges_after_required_restart_cut_without_double_principal() {
    let (config, _directory, journal_path, mut owner) = ready_owner().await;
    let venue = venue_order(&config, "settlement-restart");
    let _client = place_resting_quote(&mut owner, &config, PmOrderSide::Buy, 1, venue).await;
    let matched = remote_fill_with_settlement(
        &config,
        venue,
        "settlement-restart-fill",
        "0.25",
        PmFillSettlementStatus::Matched,
    );
    reduce_ws_fill(&mut owner, &config, matched, 30, 130);
    drain_persistence(&mut owner, 131).await;
    let confirmed = remote_fill_with_settlement(
        &config,
        venue,
        "settlement-restart-fill",
        "0.25",
        PmFillSettlementStatus::Confirmed,
    );
    let (account, fills) = reconciliation_pair_with_fills(
        &config,
        3,
        40,
        41,
        Some(INITIAL_CURSOR_BYTE),
        2,
        vec![confirmed],
        140,
    );
    assert!(matches!(
        owner
            .reduce_serviced_reconciliation(account, fills)
            .unwrap(),
        PmReconciliationApply::Applied { .. }
    ));
    let runtime_projection = owner.private_mut().private_projection_for_test();
    let runtime = runtime_projection.fills().next().unwrap();
    assert_eq!(runtime.key(), matched.fill_key());
    assert_eq!(runtime.settlement(), PmFillSettlementStatus::Confirmed);
    assert!(runtime.covered_by_reconciliation().is_some());
    assert_eq!(
        runtime_projection.fill_counters().principal_applications(),
        1
    );
    drain_persistence(&mut owner, 141).await;
    owner.shutdown().await.unwrap();

    let (mut restarted, recovery) = restart_owner(&config, &journal_path).await;
    assert!(recovery.requires_reconciliation());
    assert_eq!(recovery.recovered_observations().len(), 1);
    let durable_projection = restarted.private_mut().private_projection_for_test();
    let durable = durable_projection.fills().next().unwrap();
    assert_eq!(durable.key(), matched.fill_key());
    assert_eq!(durable.settlement(), PmFillSettlementStatus::Matched);
    assert_eq!(durable.covered_by_reconciliation(), None);
    assert_eq!(
        durable_projection.fill_counters().principal_applications(),
        1
    );

    prime_restarted_private_on_epoch(&mut restarted, &config, 2);
    let (account, fills) = reconciliation_pair_with_fills_on_epoch(
        &config,
        2,
        2,
        220,
        221,
        Some(2),
        3,
        vec![confirmed],
        220,
    );
    assert!(matches!(
        restarted
            .reduce_serviced_reconciliation(account, fills)
            .unwrap(),
        PmReconciliationApply::Applied { .. }
    ));
    let reconverged_projection = restarted.private_mut().private_projection_for_test();
    let reconverged = reconverged_projection.fills().next().unwrap();
    assert_eq!(reconverged.key(), runtime.key());
    assert_eq!(reconverged.settlement(), runtime.settlement());
    assert!(reconverged.covered_by_reconciliation().is_some());
    assert_eq!(
        reconverged_projection
            .fill_counters()
            .principal_applications(),
        1
    );
    restarted.shutdown().await.unwrap();
}

fn remote_fill_with_settlement(
    config: &PmConnectivityConfig,
    venue_order: PmVenueOrderKey,
    id: &str,
    quantity: &str,
    settlement: PmFillSettlementStatus,
) -> PmFillEvent {
    PmFillEvent::new(
        config.account().account_route().source(),
        config.account().instrument(),
        PmFillKey::new(venue_order, PmFillId::new(id).unwrap()),
        PmOrderIdentity::new(None, Some(venue_order)).unwrap(),
        PmFillExecution::new(
            PmOrderSide::Buy,
            PmFillRole::Maker,
            settlement,
            PmPrice::parse_decimal("0.40").unwrap(),
            PmQuantity::parse_decimal(quantity).unwrap(),
            PmFillFee::Unknown,
        ),
    )
    .unwrap()
}

#[tokio::test(flavor = "current_thread")]
async fn successful_paired_cut_clears_recovery_reconciliation_halt() {
    let (config, _directory, journal_path, mut owner) = ready_owner().await;
    let _client = admit_quote(&mut owner, &config, PmOrderSide::Buy, 1);
    wait_for_prepared_quotes(&mut owner, 1, 121).await;
    owner.shutdown().await.unwrap();

    let (mut restarted, recovery) = restart_owner(&config, &journal_path).await;
    assert!(recovery.requires_reconciliation());
    assert_eq!(
        restarted.halt(),
        Some(PmMutationHalt::RecoveryReconciliationRequired)
    );
    prime_restarted_private(&mut restarted, &config);
    let (account, fills) =
        reconciliation_pair(&config, 2, 220, 221, Some(INITIAL_CURSOR_BYTE), 2, 220);
    assert!(matches!(
        restarted
            .reduce_serviced_reconciliation(account, fills)
            .unwrap(),
        PmReconciliationApply::Applied { .. }
    ));
    assert_eq!(restarted.halt(), None);
    restarted.shutdown().await.unwrap();
}

#[tokio::test(flavor = "current_thread")]
async fn successful_paired_cut_does_not_clear_recovered_safety_halt() {
    let (config, _directory, journal_path, mut owner) = ready_owner().await;
    owner
        .record_fact(
            PmJournalRecordV1::SafetyHalt(PmJournalSafetyHaltV1 {
                account: config.account().account(),
                reason: PmJournalSafetyReasonV1::ContractViolation,
            }),
            120,
        )
        .unwrap();
    drain_persistence(&mut owner, 121).await;
    owner.shutdown().await.unwrap();

    let (mut restarted, recovery) = restart_owner(&config, &journal_path).await;
    assert!(recovery.safety_halted());
    assert_eq!(
        restarted.halt(),
        Some(PmMutationHalt::RecoveredSafetyHalt(
            PmJournalSafetyReasonV1::ContractViolation
        ))
    );
    prime_restarted_private(&mut restarted, &config);
    let (account, fills) =
        reconciliation_pair(&config, 2, 220, 221, Some(INITIAL_CURSOR_BYTE), 2, 220);
    assert!(matches!(
        restarted
            .reduce_serviced_reconciliation(account, fills)
            .unwrap(),
        PmReconciliationApply::Applied { .. }
    ));
    assert_eq!(
        restarted.halt(),
        Some(PmMutationHalt::RecoveredSafetyHalt(
            PmJournalSafetyReasonV1::ContractViolation
        ))
    );
    restarted.shutdown().await.unwrap();
}

#[tokio::test(flavor = "current_thread")]
async fn restart_seeds_the_durable_cursor_and_never_reemits_it_for_the_next_cut() {
    let (config, _directory, journal_path, owner) = ready_owner().await;
    owner.shutdown().await.unwrap();

    let (mut restarted, recovery) = restart_owner(&config, &journal_path).await;
    let recovered = PmFillQueryCursor::new(
        config.account().account_scope(),
        recovery.fill_watermark().unwrap().opaque.bytes(),
    );
    assert_eq!(recovered.opaque(), [INITIAL_CURSOR_BYTE; 32]);
    assert_eq!(restarted.private_mut().fill_watermark(), Some(recovered));

    prime_restarted_private(&mut restarted, &config);
    // The normalized account content is still empty, but a distinct complete
    // causal cut advances its chained cursor instead of repeating the prior
    // journal watermark.
    let (account, fills) =
        reconciliation_pair(&config, 2, 220, 221, Some(INITIAL_CURSOR_BYTE), 2, 220);
    assert!(matches!(
        restarted
            .reduce_serviced_reconciliation(account, fills)
            .unwrap(),
        PmReconciliationApply::Applied { .. }
    ));
    drain_persistence(&mut restarted, 221).await;
    restarted.shutdown().await.unwrap();

    let scope = PmJournalScopeV1::from_config(&config).unwrap();
    let recovered_again = recover_pm_mutation_journal(&journal_path, &scope).unwrap();
    assert_eq!(
        recovered_again.fill_watermark().unwrap().opaque.bytes(),
        [2; 32]
    );
}

#[tokio::test(flavor = "current_thread")]
async fn restart_seeds_cursor_before_observations_and_exposes_it_to_the_next_request() {
    let (config, _directory, journal_path, mut owner) = ready_owner().await;
    let venue = venue_order(&config, "recovered-cursor-order");
    let _client = place_resting_quote(&mut owner, &config, PmOrderSide::Buy, 1, venue).await;
    let fill = remote_fill_with_settlement(
        &config,
        venue,
        "recovered-cursor-fill",
        "0.25",
        PmFillSettlementStatus::Matched,
    );
    reduce_ws_fill(&mut owner, &config, fill, 30, 130);
    drain_persistence(&mut owner, 131).await;
    owner.shutdown().await.unwrap();

    let (mut restarted, recovery) = restart_owner(&config, &journal_path).await;
    assert_eq!(recovery.recovered_observations().len(), 1);
    let recovered_cursor = PmFillQueryCursor::new(
        config.account().account_scope(),
        recovery.fill_watermark().unwrap().opaque.bytes(),
    );
    // Recovery succeeds with a retained fill only because cursor seeding runs
    // while fill state is still fresh, before replaying that observation.
    assert_eq!(
        restarted.private_mut().fill_watermark(),
        Some(recovered_cursor)
    );
    assert_eq!(
        restarted.halt(),
        Some(PmMutationHalt::RecoveryReconciliationRequired)
    );
    assert!(matches!(
        restarted
            .private_mut()
            .quote_readiness(PmPrivateQuoteRequest::new(
                200,
                PmOrderSide::Buy,
                PmPrice::parse_decimal("0.40").unwrap(),
                PmQuantity::parse_decimal("1").unwrap(),
                PmExactReservation::policy_approved(U256::from_u64(400_000), U256::ZERO).unwrap(),
            )),
        PmPrivateReadiness::Blocked(_)
    ));

    prime_restarted_private_on_epoch(&mut restarted, &config, 2);
    let (account, fills) = reconciliation_pair_with_fills_on_epoch(
        &config,
        2,
        2,
        220,
        221,
        Some(INITIAL_CURSOR_BYTE),
        2,
        vec![fill],
        220,
    );
    assert_eq!(fills.payload().requested_after(), Some(recovered_cursor));
    assert!(matches!(
        restarted
            .reduce_serviced_reconciliation(account, fills)
            .unwrap(),
        PmReconciliationApply::Applied { .. }
    ));
    drain_persistence(&mut restarted, 221).await;
    restarted.shutdown().await.unwrap();
}

#[tokio::test(flavor = "current_thread")]
async fn fill_watermark_records_every_cursor_advance_and_never_an_unchanged_cursor() {
    let (config, _directory, journal_path, mut owner) = ready_owner().await;
    assert_eq!(owner.counters().fact_records(), READY_BASELINE_FACT_RECORDS);

    let (account, fills) =
        reconciliation_pair(&config, 3, 120, 121, Some(INITIAL_CURSOR_BYTE), 1, 120);
    assert!(matches!(
        owner
            .reduce_serviced_reconciliation(account, fills)
            .unwrap(),
        PmReconciliationApply::Applied { .. }
    ));
    assert_eq!(owner.counters().fact_records(), READY_BASELINE_FACT_RECORDS);

    let venue = venue_order(&config, "watermark-order");
    let client = place_resting_quote(&mut owner, &config, PmOrderSide::Buy, 1, venue).await;
    let fill = owned_fill(&config, client, venue, "watermark-fill", "0.25");
    reduce_ws_fill(&mut owner, &config, fill, 30, 130);
    let after_fill = READY_BASELINE_FACT_RECORDS + 2;
    assert_eq!(owner.counters().fact_records(), after_fill);

    let (account, fills) = reconciliation_pair_with_fills(
        &config,
        4,
        150,
        151,
        Some(INITIAL_CURSOR_BYTE),
        2,
        vec![fill],
        150,
    );
    owner
        .reduce_serviced_reconciliation(account, fills)
        .unwrap();
    assert_eq!(owner.counters().fact_records(), after_fill + 1);
    drain_persistence(&mut owner, 151).await;
    assert!(!owner.private_mut().fill_watermark_compaction_pending());

    let (account, fills) = reconciliation_pair(&config, 5, 160, 161, Some(2), 3, 160);
    owner
        .reduce_serviced_reconciliation(account, fills)
        .unwrap();
    assert_eq!(owner.counters().fact_records(), after_fill + 2);

    let (account, fills) = reconciliation_pair(&config, 6, 170, 171, Some(3), 3, 170);
    owner
        .reduce_serviced_reconciliation(account, fills)
        .unwrap();
    assert_eq!(owner.counters().fact_records(), after_fill + 2);

    let expected = [
        PmDurableRecordKind::PlaceResult,
        PmDurableRecordKind::FillApplied,
        PmDurableRecordKind::FillWatermarkAdvanced,
        PmDurableRecordKind::FillWatermarkAdvanced,
    ];
    for kind in expected {
        assert_eq!(owner.pop_durable_consequence().unwrap().kind(), kind);
    }
    assert!(owner.pop_durable_consequence().is_none());

    drain_persistence(&mut owner, 171).await;
    owner.shutdown().await.unwrap();
    let scope = PmJournalScopeV1::from_config(&config).unwrap();
    let recovery = recover_pm_mutation_journal(journal_path, &scope).unwrap();
    assert_eq!(
        recovery.record_count(),
        JOURNAL_HEADER_RECORDS + READY_BASELINE_JOURNAL_RECORDS + 5
    );
    assert_eq!(
        recovery.fill_watermark().unwrap().opaque.bytes(),
        [3_u8; 32]
    );
}
