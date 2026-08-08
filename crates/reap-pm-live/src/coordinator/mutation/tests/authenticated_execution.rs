//! Vertical authenticated-execution tests share the canonical mutation-owner
//! fixture helpers so they exercise the same reducers and Goal-F journal.

use super::*;

const AUTHENTICATED_TEST_ADDRESS: &str = "0xf39Fd6e51aad88F6F4ce6aB8827279cffFb92266";

#[test]
fn authenticated_fixture_binds_the_exact_signer_account_scope() {
    let eoa = EvmAddress::parse(AUTHENTICATED_TEST_ADDRESS).unwrap();
    let config = fixture_for_eoa(eoa);

    assert_eq!(config.account().account_scope().signer().address(), eoa);
    assert_eq!(config.account().account_scope().funder().address(), eoa);
}

#[tokio::test(flavor = "current_thread")]
async fn accepted_cancel_detail_targets_are_exact_and_deterministically_ordered() {
    let (config, _directory, _journal_path, mut owner) = ready_owner().await;
    let later = venue_order(&config, "z-detail-target");
    let earlier = venue_order(&config, "a-detail-target");
    let buy = place_resting_quote(&mut owner, &config, PmOrderSide::Buy, 1, later).await;
    drain_persistence(&mut owner, 123).await;
    let sell = place_resting_quote(&mut owner, &config, PmOrderSide::Sell, 2, earlier).await;

    for (index, client) in [buy, sell].into_iter().enumerate() {
        let clock_base = 140 + (index as u64 * 20);
        assert!(matches!(
            owner.begin_cancel(cancel_request(client)).unwrap(),
            PmCancelMutationAdmission::JournalPending { client_order, .. }
                if client_order == client
        ));
        wait_for_prepared_cancel(&mut owner, clock_base).await;
        owner
            .execute_next_cancel(
                &fixture_executor(&config),
                PmFakeCancelScript::accepted(),
                clock_base + 10,
            )
            .unwrap();
    }

    assert!(owner.has_missing_order_detail());
    assert_eq!(owner.next_missing_order_detail(), Some(earlier));
    assert!(
        owner
            .pending_refresh(reap_pm_state::PmRefreshReason::MissingOrderDetail)
            .is_some(),
        "an accepted exact cancel creates its canonical detail obligation without a caller minting it"
    );

    drain_persistence(&mut owner, 180).await;
    owner.shutdown().await.unwrap();
}
