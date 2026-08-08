//! PM-T1 authenticated-loopback vertical acceptance.
//!
//! The full accepted place/fill/cancel/restart test is assembled here rather
//! than in a generic integration crate so it can inspect the static run owner
//! without widening any production authority surface.

use std::time::Duration;

use reap_pm_core::{
    ConnectionEpoch, EvmAddress, PmAccountScope, PmFillSettlementStatus, PmFunderId, PmOrderStatus,
    PmQuantity, PmSignedUnits, PmSignerId, PmVenueOrderId, PmVenueOrderKey, U256,
};
use reap_pm_live_contracts::{
    PmAccountConnectivityConfig, PmConnectivityConfig, PmFixedExecutionProfile,
};
use reap_pm_state::{
    PmBookFreshness, PmFillFeeState, PmObservedAmount, PmOrderOwnership, PmPositionKnowledge,
    PmPrivateConvergence,
};
use reap_pm_strategy::{PmFixtureQuoteModel, PmModelInputRequirements, PmQuoteSides};
use reap_polymarket_adapter::{PmFixedMutationPreparation, PmPublicHeartbeatConfig};
use reap_polymarket_auth::{
    CredentialSlotId, EoaPrivateKeyInput, FixedEoaSigner, L2CredentialInput, L2Credentials,
};
use reap_polymarket_live_adapter::{
    PmExactOwnedCancelLoopbackRole, PmFixedPlaceLoopbackRole, PmLoopbackMutationConfig,
    PmLoopbackMutationConnectivityOwner, PmPrivateHttpConfig, PmProductClockOwner,
    PmPublicConnectivityOwner, PmPublicHttpConfig, PmPublicWsConfig, PmUserWsConfig,
};
use reap_polymarket_wire::{PmBookParserConfig, PmWireScope};

use super::super::root::PmAuthenticatedLoopbackProduct;
use super::super::startup::PmAuthenticatedLoopbackReady;
use super::loopback_fixture_tests::{
    ADDRESS, API_KEY, API_SECRET, LoopbackVenue, PASSPHRASE, TEST_KEY,
};
use super::{PmAuthenticatedLoopbackRun, PmMutationFinishOutcome, PmMutationStartOutcome};
use crate::coordinator::PmPreparedMutationKind;
use crate::{
    PmCaptureProvenance, PmCaptureReconnectPolicy, PmCaptureSessionPolicy, PmCoordinatorPolicy,
    PmProduct, PmProductPublicIngress, PmScheduledActionKind,
};

fn authenticated_config() -> PmConnectivityConfig {
    let base = crate::evidence::connectivity_config();
    let base_scope = base.account().account_scope();
    let eoa = EvmAddress::parse(ADDRESS).expect("vertical EOA");
    let account_scope = PmAccountScope::new(
        base_scope.environment(),
        base_scope.chain(),
        PmSignerId::new(eoa),
        PmFunderId::new(eoa),
        base_scope.handle(),
    );
    let public = base.public().clone();
    let account = PmAccountConnectivityConfig::derive_goal_f(
        &public,
        account_scope,
        base.account().account_route(),
    )
    .expect("vertical account connectivity");
    PmConnectivityConfig::new(public, account).expect("vertical connectivity")
}

fn model(config: &PmConnectivityConfig) -> PmFixtureQuoteModel {
    PmFixtureQuoteModel::new(
        PmModelInputRequirements::new(
            config.public().okx_reference(),
            config.public().instrument(),
        ),
        0.40,
        PmQuantity::parse_decimal("5").expect("vertical quantity"),
        PmQuoteSides::Buy,
    )
    .expect("vertical model")
}

fn session_policy(initial_epoch: ConnectionEpoch) -> PmCaptureSessionPolicy {
    let reconnect =
        PmCaptureReconnectPolicy::new(Duration::from_millis(1), Duration::from_millis(4), 2)
            .expect("vertical reconnect policy");
    PmCaptureSessionPolicy::new(
        initial_epoch,
        None,
        reconnect,
        PmPublicHeartbeatConfig::new(100_000_000, 50_000_000).expect("vertical heartbeat"),
        PmBookFreshness::new(5_000_000_000, 5_000_000_000).expect("vertical freshness"),
        1,
        reconnect,
    )
    .expect("vertical session policy")
}

fn provenance() -> PmCaptureProvenance {
    PmCaptureProvenance::new(
        "8222273a9c72033b760e1d2fec813bc77144556d",
        "bbb5bc143a914ba8c96d84342321b3dba30ec0fc",
        "8e671f14c4b1e8137b1dc1b0bd7d39c79d9c8f961a8483daa32151df99cbdf81",
        "aca0221387a45e0ab0eec76adfb3dce8e7d3c0cbcb32187167dd5c556c459eeb",
    )
    .expect("vertical provenance")
}

#[derive(Clone, Copy)]
struct ExpectedLiveDependencies {
    book_requests: usize,
    open_order_requests: usize,
    trade_requests: usize,
    balance_requests: usize,
    order_detail_requests: usize,
    position: U256,
}

const INITIAL_LIVE_DEPENDENCIES: ExpectedLiveDependencies = ExpectedLiveDependencies {
    book_requests: 1,
    open_order_requests: 1,
    trade_requests: 1,
    balance_requests: 2,
    order_detail_requests: 0,
    position: U256::from_u64(10_000_000_000),
};

const RESTARTED_LIVE_DEPENDENCIES: ExpectedLiveDependencies = ExpectedLiveDependencies {
    book_requests: 2,
    open_order_requests: 2,
    trade_requests: 4,
    balance_requests: 8,
    order_detail_requests: 2,
    position: U256::from_u64(10_002_500_000),
};

fn authenticated_root(
    venue: &LoopbackVenue,
    config: &PmConnectivityConfig,
    private_epoch: ConnectionEpoch,
) -> PmAuthenticatedLoopbackProduct<PmFixtureQuoteModel> {
    let metadata = config.public().expected_metadata();
    let scope = PmWireScope::new(
        metadata.condition(),
        metadata.market(),
        metadata.outcome().token(),
    );
    let parser = PmBookParserConfig::new_condition_bound(
        scope,
        metadata.tick(),
        metadata.minimum_order_size(),
        metadata.negative_risk(),
    );
    let public_http = PmPublicHttpConfig::loopback_evidence(
        venue.http_origin(),
        Duration::from_millis(100),
        Duration::from_secs(2),
    )
    .expect("vertical public HTTP");
    let public_ws = PmPublicWsConfig::loopback_evidence(
        venue.public_ws_endpoint(),
        scope,
        Duration::from_millis(100),
        Duration::from_secs(2),
        Duration::from_millis(100),
        Duration::from_millis(50),
        4_096,
        1,
        Duration::from_millis(10),
        16,
        private_epoch,
    )
    .expect("vertical public WS");
    let public = PmPublicConnectivityOwner::new(
        public_http,
        parser,
        public_ws,
        PmProductClockOwner::system(),
    )
    .expect("vertical public connectivity");

    let private_http = PmPrivateHttpConfig::loopback_evidence(
        venue.http_origin(),
        Duration::from_millis(100),
        Duration::from_secs(2),
        scope,
    )
    .expect("vertical private HTTP");
    let user_ws = PmUserWsConfig::loopback_evidence(
        venue.user_ws_endpoint(),
        metadata.condition(),
        Duration::from_millis(100),
        Duration::from_secs(2),
        Duration::from_millis(100),
        Duration::from_millis(50),
        8 * 1_024,
        1,
        Duration::from_millis(10),
        16,
        private_epoch,
    )
    .expect("vertical user WS");
    let preparation = PmFixedMutationPreparation::new(
        config.account().account_scope(),
        config.public().instrument(),
    );
    let private = PmLoopbackMutationConnectivityOwner::new(
        private_http,
        user_ws,
        config.account().account_scope(),
        config.public().instrument(),
        config.account().trading_domain(),
        preparation.place_profile(),
        preparation.cancel_purpose(),
        crate::evidence::loopback_public_observation_grant(config),
        CredentialSlotId::new("pm-t1-vertical-slot".into()).expect("vertical credential slot"),
        FixedEoaSigner::bind(EoaPrivateKeyInput::new(TEST_KEY.into()), ADDRESS)
            .expect("vertical signer"),
        L2Credentials::bind(
            ADDRESS,
            L2CredentialInput::new(API_KEY.into(), API_SECRET.into(), PASSPHRASE.into()),
        )
        .expect("vertical credentials"),
    )
    .expect("vertical mutation connectivity");
    let transport = PmLoopbackMutationConfig::loopback_evidence(
        venue.http_origin(),
        Duration::from_millis(100),
        Duration::from_secs(2),
    )
    .expect("vertical mutation transport");
    let place = PmFixedPlaceLoopbackRole::new(transport.clone()).expect("vertical place role");
    let cancel = PmExactOwnedCancelLoopbackRole::new(transport).expect("vertical cancel role");
    let product = PmProduct::new(
        config.clone(),
        model(config),
        PmFixedExecutionProfile::goal_f(),
        crate::evidence::risk_limits(),
    )
    .expect("vertical product");
    PmAuthenticatedLoopbackProduct::new(product, public, private, place, cancel)
        .expect("vertical authenticated root")
}

async fn start_authenticated(
    venue: &LoopbackVenue,
    directory: &tempfile::TempDir,
    capture_name: &str,
    private_epoch: ConnectionEpoch,
) -> PmAuthenticatedLoopbackReady<PmFixtureQuoteModel> {
    let config = authenticated_config();
    authenticated_root(venue, &config, private_epoch)
        .start(
            directory.path().join(capture_name),
            directory.path().join("goal-f.jsonl"),
            directory.path().join("authenticated.jsonl"),
            session_policy(private_epoch),
            provenance(),
            PmCoordinatorPolicy::new(5_000_000_000, 5_000_000_000, 1_000_000_000)
                .expect("vertical coordinator policy"),
            Duration::from_secs(2),
            Duration::from_secs(2),
            Duration::from_secs(30),
        )
        .await
        .expect("vertical authenticated start")
}

#[test]
fn vertical_socket_transports_share_the_supplied_restart_epoch() {
    let source = include_str!("vertical_tests.rs");
    assert_eq!(
        source.matches("16,\n        private_epoch,\n    )").count(),
        2,
        "public and authenticated user transports must share the supplied epoch"
    );
    assert!(source.contains("session_policy(private_epoch)"));
}

fn run_large_vertical_test<F, Fut>(build: F)
where
    F: FnOnce() -> Fut + Send + 'static,
    Fut: std::future::Future<Output = ()> + Send + 'static,
{
    std::thread::Builder::new()
        .name("pm-t1-vertical".to_owned())
        .stack_size(16 * 1024 * 1024)
        .spawn(move || {
            tokio::runtime::Builder::new_current_thread()
                .enable_all()
                .build()
                .expect("vertical Tokio runtime")
                .block_on(build());
        })
        .expect("spawn bounded large-schema vertical test")
        .join()
        .expect("vertical test thread joins");
}

fn actor_clock<M: reap_pm_strategy::PmQuoteModel>(
    run: &mut PmAuthenticatedLoopbackRun<M>,
) -> reap_pm_core::ReceivedEventClock {
    run.ready
        .actor_clock
        .observe_control_edge()
        .expect("vertical actor clock")
        .received_clock()
}

fn drain_outputs<M: reap_pm_strategy::PmQuoteModel>(run: &mut PmAuthenticatedLoopbackRun<M>) {
    while run
        .ready
        .coordinator
        .peek_reconciliation_refresh_effect()
        .is_none()
        && run.ready.coordinator.pop_effect().is_some()
    {}
}

fn service_until_idle<M: reap_pm_strategy::PmQuoteModel>(run: &mut PmAuthenticatedLoopbackRun<M>) {
    for _ in 0..64 {
        let monotonic = actor_clock(run).monotonic_receive_ns();
        let serviced = run
            .ready
            .coordinator
            .service_turn(monotonic)
            .expect("vertical service turn");
        drain_outputs(run);
        if serviced.total() == 0 {
            return;
        }
    }
    panic!("vertical scheduler did not become idle");
}

async fn drain_goal_f<M: reap_pm_strategy::PmQuoteModel>(run: &mut PmAuthenticatedLoopbackRun<M>) {
    let deadline = tokio::time::Instant::now() + Duration::from_secs(2);
    while run.ready.coordinator.persistence_metrics().depth() != 0 {
        let clock = actor_clock(run);
        let monotonic = clock.monotonic_receive_ns();
        let occurrence = run
            .ready
            .occurrence_issuer
            .issue_persistence_poll(clock)
            .expect("vertical persistence occurrence");
        if run
            .ready
            .coordinator
            .poll_persistence_live(occurrence, monotonic)
            .expect("vertical persistence poll")
        {
            service_until_idle(run);
        } else {
            assert!(
                tokio::time::Instant::now() < deadline,
                "vertical Goal-F writer timed out"
            );
            tokio::task::yield_now().await;
        }
    }
}

async fn prepare_live_dependencies(
    run: &mut PmAuthenticatedLoopbackRun<PmFixtureQuoteModel>,
    venue: &LoopbackVenue,
    expected: ExpectedLiveDependencies,
) {
    run.start_read_ingress()
        .expect("vertical read ingress starts once");
    let okx_started = run
        .ready
        .okx_clock
        .observe_okx_edge()
        .expect("vertical OKX start clock");
    let okx_subscribed = run
        .ready
        .okx_clock
        .observe_okx_edge()
        .expect("vertical OKX subscription clock");
    {
        let mut ingress = PmProductPublicIngress::new(run.ready.coordinator.public_capture_mut());
        ingress
            .record_okx_connection_started(okx_started.monotonic_receive_ns())
            .await
            .expect("vertical OKX connection");
        ingress
            .record_okx_subscription_sent(okx_subscribed.monotonic_receive_ns())
            .await
            .expect("vertical OKX subscription");
    }
    let ack_clock = run
        .ready
        .okx_clock
        .observe_okx_edge()
        .expect("vertical OKX ack clock");
    let reference_clock = run
        .ready
        .okx_clock
        .observe_okx_edge()
        .expect("vertical OKX reference clock");
    {
        let mut ingress = PmProductPublicIngress::new(run.ready.coordinator.public_capture_mut());
        ingress
            .capture_okx_public(
                ack_clock.local_wall_receive_ns(),
                ack_clock.monotonic_receive_ns(),
                br#"{"event":"subscribe","arg":{"channel":"index-tickers","instId":"BTC-USDT"}}"#,
            )
            .await
            .expect("vertical OKX acknowledgement");
        ingress
            .capture_okx_public(
                reference_clock.local_wall_receive_ns(),
                reference_clock.monotonic_receive_ns(),
                br#"{"arg":{"channel":"index-tickers","instId":"BTC-USDT"},"data":[{"instId":"BTC-USDT","idxPx":"00050000.125000","ts":"1780449126123"}]}"#,
            )
            .await
            .expect("vertical OKX reference");
    }
    service_until_idle(run);

    let deadline = tokio::time::Instant::now() + Duration::from_secs(5);
    loop {
        let _ = run
            .dispatch_next_read_refresh()
            .expect("vertical refresh dispatch");
        let _ = run
            .service_read_ingress_once()
            .await
            .expect("vertical read service");
        service_until_idle(run);
        drain_goal_f(run).await;

        let private_ready = {
            let projection = run.ready.coordinator.private_projection_for_test();
            matches!(
                projection.convergence(),
                PmPrivateConvergence::Converged { .. }
            ) && projection.account_snapshot().position()
                == PmPositionKnowledge::Tradable(expected.position)
                && projection.pending_refresh_count() == 0
                && !projection.full_reconcile_required()
        };
        let public = run.ready.coordinator.counters();
        if public.references_applied() >= 1
            && public.markets_applied() >= 1
            && public.books_applied() >= 1
            && private_ready
            && run.ready.coordinator.current_fill_query_cursor().is_some()
            && venue.request_count_prefix("GET /data/orders?") == expected.open_order_requests
        {
            assert_eq!(
                venue.request_count_prefix("GET /book?token_id="),
                expected.book_requests
            );
            assert_eq!(
                venue.request_count_prefix("GET /data/orders?"),
                expected.open_order_requests
            );
            assert_eq!(
                venue.request_count_prefix("GET /data/trades?"),
                expected.trade_requests
            );
            assert_eq!(
                venue.request_count_prefix("GET /balance-allowance?"),
                expected.balance_requests
            );
            assert_eq!(
                venue.request_count_prefix("GET /data/order/"),
                expected.order_detail_requests
            );
            let projection = run.ready.coordinator.private_projection_for_test();
            assert_eq!(projection.pending_refresh_count(), 0);
            assert!(projection.pending_refresh_keys().next().is_none());
            assert!(!projection.full_reconcile_required());
            return;
        }
        assert!(
            tokio::time::Instant::now() < deadline,
            "vertical live dependencies did not converge"
        );
        tokio::time::sleep(Duration::from_millis(1)).await;
    }
}

async fn schedule_quote(run: &mut PmAuthenticatedLoopbackRun<PmFixtureQuoteModel>) {
    let clock = actor_clock(run);
    let scheduled = clock.monotonic_receive_ns();
    run.ready
        .coordinator
        .schedule(
            reap_pm_core::PmOrderSide::Buy,
            PmScheduledActionKind::QuoteEvaluation,
            scheduled,
            scheduled,
            clock.local_wall_receive_ns() / 1_000_000,
        )
        .expect("vertical quote schedule");
    service_until_idle(run);
    drain_goal_f(run).await;
    service_until_idle(run);
}

async fn execute_prepared_place(run: &mut PmAuthenticatedLoopbackRun<PmFixtureQuoteModel>) {
    let deadline = tokio::time::Instant::now() + Duration::from_secs(5);
    let initial_metrics = run.ready.coordinator.effect_dispatch_metrics();
    let initial_kind = run.ready.coordinator.next_prepared_mutation_kind();
    if initial_kind != Some(PmPreparedMutationKind::Quote) {
        panic!(
            "vertical expected one prepared quote: kind={initial_kind:?}, coordinator={:?}, mutation={:?}, dispatch={initial_metrics:?}, persistence={:?}, halt={:?}/{:?}, convergence={:?}",
            run.ready.coordinator.counters(),
            run.ready.coordinator.mutation_counters(),
            run.ready.coordinator.persistence_metrics(),
            run.ready.coordinator.halt(),
            run.ready.coordinator.mutation_halt(),
            run.ready
                .coordinator
                .private_projection_for_test()
                .convergence(),
        );
    }
    assert_eq!(initial_metrics.depth(), 1);
    assert_eq!(initial_metrics.queued(), 1);
    assert_eq!(initial_metrics.blocked(), 0);
    assert_eq!(initial_metrics.retained(), 0);
    assert!(!initial_metrics.quote_suppressed());
    let mut requested_time = false;
    loop {
        assert_eq!(
            run.ready.coordinator.next_prepared_mutation_kind(),
            Some(PmPreparedMutationKind::Quote)
        );
        let metrics_before = run.ready.coordinator.effect_dispatch_metrics();
        assert_eq!(metrics_before.depth(), initial_metrics.depth());
        assert_eq!(metrics_before.queued(), initial_metrics.queued());
        assert_eq!(metrics_before.blocked(), initial_metrics.blocked());
        assert_eq!(metrics_before.retained(), initial_metrics.retained());
        assert_eq!(metrics_before.serviced(), initial_metrics.serviced());
        let result = run.start_place();
        let metrics_after = run.ready.coordinator.effect_dispatch_metrics();
        match result.expect("vertical place start/poll") {
            PmMutationStartOutcome::TimeRequested => {
                assert!(!requested_time, "one place uses one mutation-time request");
                requested_time = true;
                assert!(run.place_time_pending());
                assert_eq!(metrics_after, metrics_before);
            }
            PmMutationStartOutcome::PendingTime => {
                assert!(requested_time);
                assert!(run.place_time_pending());
                assert_eq!(metrics_after, metrics_before);
            }
            PmMutationStartOutcome::Started => {
                assert!(requested_time);
                assert!(!run.place_time_pending());
                assert_eq!(run.ready.coordinator.next_prepared_mutation_kind(), None);
                assert_eq!(metrics_after.depth(), 0);
                assert_eq!(metrics_after.queued(), 0);
                assert_eq!(metrics_after.blocked(), 0);
                assert_eq!(metrics_after.retained(), 0);
                assert_eq!(
                    metrics_after.serviced(),
                    initial_metrics.serviced().saturating_add(1)
                );
                break;
            }
        }
        assert!(
            tokio::time::Instant::now() < deadline,
            "vertical place time proof did not arrive"
        );
        tokio::time::sleep(Duration::from_millis(1)).await;
    }
    loop {
        match run.finish_place().await.expect("vertical place completion") {
            PmMutationFinishOutcome::Applied => return,
            PmMutationFinishOutcome::PendingTask | PmMutationFinishOutcome::PendingBridge => {}
        }
        assert!(
            tokio::time::Instant::now() < deadline,
            "vertical place did not cross both durability barriers"
        );
        tokio::task::yield_now().await;
    }
}

fn exact_venue_order(order_id: &str) -> PmVenueOrderKey {
    PmVenueOrderKey::new(
        authenticated_config().account().account(),
        PmVenueOrderId::new(order_id).expect("vertical venue order id"),
    )
}

async fn wait_for_partial_fill_and_account_cut(
    run: &mut PmAuthenticatedLoopbackRun<PmFixtureQuoteModel>,
    venue: &LoopbackVenue,
    expected_order: PmVenueOrderKey,
    prior_cursor: reap_pm_core::PmFillQueryCursor,
) {
    venue.publish_partial_fill().await;
    let deadline = tokio::time::Instant::now() + Duration::from_secs(5);
    let collateral_asset = authenticated_config().account().collateral_asset();
    let compacted_before = run
        .ready
        .coordinator
        .mutation_counters()
        .canonical_fill_rows_compacted();
    loop {
        let _ = run
            .dispatch_next_read_refresh()
            .expect("vertical post-fill refresh dispatch");
        let _ = run
            .service_read_ingress_once()
            .await
            .expect("vertical post-fill read service");
        service_until_idle(run);

        let complete_cut_applied = {
            let projection = run.ready.coordinator.private_projection_for_test();
            let exact_order = projection
                .orders()
                .find(|order| order.identity().venue_order_key() == Some(expected_order));
            let exact_fill = projection
                .fills()
                .find(|fill| fill.key().venue_order() == expected_order);
            let account = projection.account_snapshot();
            let advanced_cursor = run
                .ready
                .coordinator
                .current_fill_query_cursor()
                .is_some_and(|cursor| cursor != prior_cursor);
            exact_order.is_some_and(|order| {
                order.status() == Some(PmOrderStatus::PartiallyFilled)
                    && order.ownership() == PmOrderOwnership::ProvenOwned
            }) && exact_fill.is_some_and(|fill| {
                fill.settlement() == PmFillSettlementStatus::Confirmed
                    && fill.covered_by_reconciliation().is_some()
                    && matches!(
                        fill.fee(),
                        PmFillFeeState::Known { asset, delta }
                            if asset == collateral_asset && delta == PmSignedUnits::ZERO
                    )
            }) && account.collateral() == PmObservedAmount::Present(U256::from_u64(9_999_000_000))
                && account.outcome_balance()
                    == PmObservedAmount::Present(U256::from_u64(10_002_500_000))
                && account.position()
                    == PmPositionKnowledge::Tradable(U256::from_u64(10_002_500_000))
                && matches!(
                    projection.convergence(),
                    PmPrivateConvergence::Converged { .. }
                )
                && advanced_cursor
        };
        if complete_cut_applied {
            assert_eq!(
                run.ready
                    .coordinator
                    .private_projection_for_test()
                    .fill_counters()
                    .principal_applications(),
                1
            );
            drain_goal_f(run).await;
            let projection = run.ready.coordinator.private_projection_for_test();
            assert!(
                projection
                    .fills()
                    .all(|fill| fill.key().venue_order() != expected_order),
                "durable watermark acknowledgement compacts the covered exact fill"
            );
            assert_eq!(
                run.ready
                    .coordinator
                    .mutation_counters()
                    .canonical_fill_rows_compacted(),
                compacted_before + 1
            );
            assert!(
                run.ready
                    .coordinator
                    .current_fill_query_cursor()
                    .is_some_and(|cursor| cursor != prior_cursor)
            );
            return;
        }
        drain_goal_f(run).await;
        assert!(
            tokio::time::Instant::now() < deadline,
            "vertical partial fill/account/trades did not converge"
        );
        tokio::time::sleep(Duration::from_millis(1)).await;
    }
}

fn assert_terminal_semantics(
    run: &PmAuthenticatedLoopbackRun<PmFixtureQuoteModel>,
    expected_order: PmVenueOrderKey,
) -> Option<(Option<PmOrderStatus>, PmOrderOwnership)> {
    let projection = run.ready.coordinator.private_projection_for_test();
    let order = projection
        .orders()
        .find(|order| order.identity().venue_order_key() == Some(expected_order));
    let order_projection = order.map(|order| (order.status(), order.ownership()));
    if let Some((status, ownership)) = order_projection {
        assert_eq!(status, Some(PmOrderStatus::Cancelled));
        assert_eq!(ownership, PmOrderOwnership::ProvenOwned);
    }
    assert!(
        projection
            .fills()
            .all(|fill| fill.key().venue_order() != expected_order),
        "covered fill remains compacted after its durable watermark"
    );
    let account = projection.account_snapshot();
    assert_eq!(
        account.collateral(),
        PmObservedAmount::Present(U256::from_u64(9_999_000_000))
    );
    assert_eq!(
        account.outcome_balance(),
        PmObservedAmount::Present(U256::from_u64(10_002_500_000))
    );
    assert_eq!(
        account.position(),
        PmPositionKnowledge::Tradable(U256::from_u64(10_002_500_000))
    );
    assert!(matches!(
        projection.convergence(),
        PmPrivateConvergence::Converged { .. }
    ));
    order_projection
}

fn assert_journals_are_secret_free(directory: &tempfile::TempDir) {
    for name in ["goal-f.jsonl", "authenticated.jsonl"] {
        let journal = std::fs::read(directory.path().join(name)).expect("vertical journal bytes");
        for forbidden in [TEST_KEY, API_SECRET, PASSPHRASE, API_KEY] {
            assert!(
                !journal
                    .windows(forbidden.len())
                    .any(|window| window == forbidden.as_bytes()),
                "{name} must not retain credential material"
            );
        }
    }
}

#[tokio::test]
async fn combined_loopback_fixture_serves_all_fixed_route_families() {
    let venue = LoopbackVenue::start().await;
    assert!(venue.http_origin().starts_with("http://127.0.0.1:"));
    assert!(venue.user_ws_endpoint().starts_with("ws://127.0.0.1:"));
    assert!(venue.public_ws_endpoint().starts_with("ws://127.0.0.1:"));
    assert_eq!(venue.request_count("POST /order"), 0);
    venue.shutdown().await;
}

#[test]
fn authenticated_root_fetches_exact_metadata_and_closes_both_durability_owners() {
    run_large_vertical_test(|| async {
        let venue = LoopbackVenue::start().await;
        let directory = tempfile::tempdir().expect("vertical temporary directory");
        let ready = start_authenticated(
            &venue,
            &directory,
            "capture-static.jsonl",
            ConnectionEpoch::new(1),
        )
        .await;
        assert_eq!(
            venue.request_count(&format!(
                "GET /markets/{}",
                super::loopback_fixture_tests::CONDITION
            )),
            1
        );
        assert_eq!(
            venue.request_count(&format!(
                "GET /clob-markets/{}",
                super::loopback_fixture_tests::CONDITION
            )),
            1
        );
        ready.shutdown().await.expect("vertical clean shutdown");
        venue.shutdown().await;
    });
}

#[test]
fn authenticated_live_cuts_reach_one_goal_f_durable_place_dispatch() {
    run_large_vertical_test(|| async {
        let venue = LoopbackVenue::start().await;
        let directory = tempfile::tempdir().expect("vertical temporary directory");
        let ready = start_authenticated(
            &venue,
            &directory,
            "capture-unsent.jsonl",
            ConnectionEpoch::new(1),
        )
        .await;
        let mut run = ready.into_run();
        prepare_live_dependencies(&mut run, &venue, INITIAL_LIVE_DEPENDENCIES).await;
        schedule_quote(&mut run).await;
        assert_eq!(run.ready.coordinator.mutation_counters().quote_intents(), 1);
        assert_eq!(
            run.ready.coordinator.mutation_counters().prepared_quotes(),
            1
        );
        assert_eq!(venue.request_count("POST /order"), 0);
        let Err(shutdown) = run.shutdown().await else {
            panic!("a durable unsent quote must be retained and reported");
        };
        assert!(matches!(
            shutdown.safety.as_deref(),
            Some(super::PmAuthenticatedLoopbackRunError::UnsentPreparedQuoteRetained)
        ));
        assert!(shutdown.read.is_none());
        assert!(shutdown.mutation_time.is_none());
        assert!(shutdown.owner.is_none());
        venue.shutdown().await;
    });
}

#[test]
fn authenticated_place_fill_exact_cancel_and_restart_converge_without_resend() {
    run_large_vertical_test(|| async {
        let venue = LoopbackVenue::start().await;
        let directory = tempfile::tempdir().expect("vertical temporary directory");
        let ready = start_authenticated(
            &venue,
            &directory,
            "capture-first.jsonl",
            ConnectionEpoch::new(1),
        )
        .await;
        let mut run = ready.into_run();
        prepare_live_dependencies(&mut run, &venue, INITIAL_LIVE_DEPENDENCIES).await;
        let initial_cursor = run
            .ready
            .coordinator
            .current_fill_query_cursor()
            .expect("initial complete trade cut establishes a cursor");

        schedule_quote(&mut run).await;
        assert_eq!(venue.request_count("POST /order"), 0);
        execute_prepared_place(&mut run).await;
        assert_eq!(venue.request_count("POST /order"), 1);
        let placed_order = venue.placed_order().expect("accepted place identity");
        let exact_order = exact_venue_order(&placed_order);
        {
            let projection = run.ready.coordinator.private_projection_for_test();
            let order = projection
                .orders()
                .find(|order| order.identity().venue_order_key() == Some(exact_order))
                .expect("accepted place enters the sole canonical order store");
            // The authenticated bridge proves local resting ownership and
            // exact venue identity, but it is not a private order event. The
            // remote-status projection remains unobserved until WS/REST.
            assert_eq!(order.status(), None);
            assert_eq!(order.ownership(), PmOrderOwnership::ProvenOwned);
        }

        wait_for_partial_fill_and_account_cut(&mut run, &venue, exact_order, initial_cursor).await;
        let filled_cursor = run
            .ready
            .coordinator
            .current_fill_query_cursor()
            .expect("post-fill complete cut advances the causal cursor");
        assert_ne!(filled_cursor, initial_cursor);

        run.drive_controlled_shutdown_safety()
            .await
            .expect("controlled shutdown proves exact owned-order safety");
        assert_eq!(venue.request_count("DELETE /order"), 1);
        assert_eq!(
            venue.cancelled_order().as_deref(),
            Some(placed_order.as_str())
        );
        assert_eq!(
            venue.request_count(&format!("GET /data/order/{placed_order}")),
            1,
            "a partially filled accepted cancel is settled only by its exact terminal detail"
        );
        let runtime_terminal_order = assert_terminal_semantics(&run, exact_order);

        let stopped = run.shutdown().await.expect("vertical clean shutdown");
        assert_eq!(stopped.shutdown_effect_counts()[0], 0);
        assert_journals_are_secret_free(&directory);
        let mutation_counts = (
            venue.request_count("POST /order"),
            venue.request_count("DELETE /order"),
        );

        let restarted = start_authenticated(
            &venue,
            &directory,
            "capture-restart.jsonl",
            ConnectionEpoch::new(2),
        )
        .await;
        assert_eq!(
            restarted.goal_f_recovery.authenticated_results().count(),
            2,
            "Goal-F restart cross-validates one place and one cancel bridge"
        );
        assert_eq!(
            restarted.authenticated_recovery.classified_results().len(),
            2,
            "authenticated restart retains the exact two classified results"
        );
        assert_eq!(
            restarted.coordinator.current_fill_query_cursor(),
            Some(filled_cursor),
            "restart seeds the exact causal fill watermark before new reads"
        );
        let mut restarted = restarted.into_run();
        prepare_live_dependencies(&mut restarted, &venue, RESTARTED_LIVE_DEPENDENCIES).await;
        assert_eq!(
            assert_terminal_semantics(&restarted, exact_order),
            runtime_terminal_order,
            "runtime and restart retain the same optional compacted order projection"
        );
        assert_eq!(
            (
                venue.request_count("POST /order"),
                venue.request_count("DELETE /order"),
            ),
            mutation_counts,
            "restart and fresh private cuts never resend either mutation"
        );
        restarted
            .shutdown()
            .await
            .expect("restarted vertical closes without mutation resend");
        assert_journals_are_secret_free(&directory);
        venue.shutdown().await;
    });
}
