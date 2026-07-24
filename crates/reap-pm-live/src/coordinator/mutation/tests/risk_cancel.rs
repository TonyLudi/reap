use reap_pm_state::{PmRiskHaltScope, PmRiskReason};

use super::*;

async fn ready_owner_with_risk_limits(
    limits: PmRiskLimits,
) -> (PmConnectivityConfig, TempDir, PmMutationOwner) {
    let config = fixture();
    let directory = tempfile::tempdir().unwrap();
    let journal_path = directory.path().join("pm-mutation.ndjson");
    let account = config.account();
    let private = PmPrivateMonitorRuntime::new(account, limits).unwrap();
    let fake = PmFakeEffectRole::new(
        account.account_scope(),
        account.instrument(),
        account.instrument_id(),
    );
    let (mut owner, recovery) = PmMutationOwner::start(&config, private, fake, journal_path)
        .await
        .unwrap();
    assert_eq!(recovery.record_count(), 0);
    make_private_ready(&mut owner, &config);
    drain_persistence(&mut owner, 111).await;
    assert_eq!(
        owner.pop_durable_consequence().unwrap().kind(),
        PmDurableRecordKind::FillWatermarkAdvanced
    );
    assert!(owner.pop_durable_consequence().is_none());
    (config, directory, owner)
}

#[tokio::test(flavor = "current_thread")]
async fn mutation_preserves_exact_risk_reason_and_required_halt_scope() {
    let limits = PmRiskLimits::new(
        PmOrderRiskLimits::new(
            PmQuantity::parse_decimal("100").unwrap(),
            U256::from_u64(100_000_000),
        )
        .unwrap(),
        PmExposureRiskLimits::new(
            U256::from_u64(5_000_000),
            U256::from_u64(1_000_000_000),
            U256::from_u64(1_000_000_000),
            U256::from_u64(1_000_000_000),
        )
        .unwrap(),
        PmCardinalityRiskLimits::new(8_192, 8_192, 8_192).unwrap(),
        PmFreshnessRiskLimits::new(
            1_000_000_000,
            1_000_000_000,
            1_000_000_000,
            1_000_000_000,
            1_000_000_000,
            1_000_000_000,
        )
        .unwrap(),
    );
    let (config, _directory, mut owner) = ready_owner_with_risk_limits(limits).await;

    assert!(matches!(
        owner.begin_quote(quote_request(
            &config,
            PmOrderSide::Buy,
            1,
            QUOTE_EXPIRES_NS
        )),
        Err(PmMutationError::RiskRejected {
            reason: PmRiskReason::MarketInventory { observed, limit },
            halt: PmRiskHaltScope::Market,
        }) if observed == U256::from_u64(6_000_000)
            && limit == U256::from_u64(5_000_000)
    ));
    assert_eq!(owner.counters().quote_intents(), 0);
    assert_eq!(owner.pending_persistence(), 0);

    owner.shutdown().await.unwrap();
}

async fn assert_operational_halt_keeps_exact_cancel_available(halt: PmMutationHalt) {
    let (config, _directory, _journal_path, mut owner) = ready_owner().await;
    let venue = venue_order(&config, "operational-halt-cancel");
    let client = place_resting_quote(&mut owner, &config, PmOrderSide::Buy, 1, venue).await;
    owner.halt = Some(halt);

    assert!(matches!(
        owner.begin_quote(quote_request(
            &config,
            PmOrderSide::Sell,
            2,
            QUOTE_EXPIRES_NS
        )),
        Err(PmMutationError::Halted(observed)) if observed == halt
    ));
    assert!(matches!(
        owner.begin_cancel(cancel_request(client)).unwrap(),
        PmCancelMutationAdmission::JournalPending {
            client_order,
            venue_order,
        } if client_order == client && venue_order == venue
    ));
    wait_for_prepared_cancel(&mut owner, 131).await;
    owner
        .execute_next_cancel(PmFakeCancelScript::accepted(), 132)
        .unwrap();
    assert_eq!(
        owner.private_mut().owned_order(client).unwrap().status(),
        Some(PmOrderStatus::Cancelled)
    );

    drain_persistence(&mut owner, 133).await;
    owner.shutdown().await.unwrap();
}

#[tokio::test(flavor = "current_thread")]
async fn recovered_operational_capacity_admits_cancel_while_quotes_stay_halted() {
    assert_operational_halt_keeps_exact_cancel_available(PmMutationHalt::PersistenceSaturated)
        .await;
    assert_operational_halt_keeps_exact_cancel_available(PmMutationHalt::FakeEffectSaturated).await;
}
