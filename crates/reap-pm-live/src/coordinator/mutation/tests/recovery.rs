use super::*;

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
    let (account, fills) = reconciliation_pair(&config, 2, 220, 221, None, 1, 220);
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
    let (account, fills) = reconciliation_pair(&config, 2, 220, 221, None, 1, 220);
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
