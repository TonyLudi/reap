use std::future::Future;

use reap_pm_core::{ConnectionEpoch, PmConditionId, PmOrderSide, ReceivedEventClock};
use reap_polymarket_auth::{L2CredentialInput, L2Credentials};
use reap_polymarket_live_adapter::{PmRestBookPurpose, PmRestBookSnapshotSink};
use reap_polymarket_wire::parse_live_user_frame;
use serde_json::json;

use crate::PmLaneKind;
use crate::evidence::{
    complete_reached_overload_reconciliation, prepare_reached_overload_product,
    start_reached_overload_product,
};
use crate::private_monitor::{PmLiveHttpQueryFailure, PmLiveIngressReport, PmLiveOccurrenceIssuer};

const OWNER: &str = "9180014b-33c8-9240-a14b-bdca11c0a465";
const CONDITION: &str = "0xaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa";
const MARKET: &str = "0xbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbb";
const PM_FUNDER: &str = "0xabababababababababababababababababababab";
const TOKEN: u64 = 123;
const AUTH_ADDRESS: &str = "0xf39Fd6e51aad88F6F4ce6aB8827279cffFb92266";
const API_SECRET: &str = "AAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAA=";
const PASSPHRASE: &str = "synthetic-passphrase";
const ORDER: &str = "0x1111111111111111111111111111111111111111111111111111111111111111";

fn rest_book() -> String {
    format!(
        r#"{{"market":"{MARKET}","asset_id":"{TOKEN}","timestamp":"123456789","hash":"8cbca234acd8c8a70913b01de917fbf6160b73e0","bids":[{{"price":"0.30","size":"100"}}],"asks":[{{"price":"0.60","size":"75"}}],"min_order_size":"5","tick_size":"0.01","neg_risk":false,"last_trade_price":"0.40"}}"#
    )
}

fn private_clock(monotonic_ns: u64) -> ReceivedEventClock {
    ReceivedEventClock::new(None, 1_700_000_000_000_000_000 + monotonic_ns, monotonic_ns)
        .expect("fixed private receive clock")
}

#[test]
fn authenticated_live_frame_uses_the_existing_product_private_queue() {
    run_product_test(authenticated_live_frame_product_test);
}

#[test]
fn native_rest_book_crosses_async_durability_sink_before_release() {
    run_product_test(native_rest_book_product_test);
}

#[test]
fn recoverable_http_failure_closes_then_complete_cut_restores_reconciliation_gate() {
    run_product_test(normalization_http_failure_product_test);
}

#[test]
fn service_http_failure_also_closes_then_complete_cut_restores_reconciliation_gate() {
    run_product_test(service_http_failure_product_test);
}

#[test]
fn every_http_purpose_fails_closed_and_a_later_complete_cut_restores_only_recoverable_failures() {
    run_product_test(all_recoverable_http_purposes_product_test);
}

#[test]
fn recovered_reconciliation_halt_suppresses_quotes_until_a_fresh_paired_cut() {
    run_product_test(recovered_reconciliation_gate_product_test);
}

#[derive(Clone, Copy)]
enum RecoverableHttpPurpose {
    Account,
    OpenOrders,
    OrderDetail,
    Reconciliation,
}

async fn recovered_reconciliation_gate_product_test() {
    let directory = tempfile::tempdir().expect("temporary recovered-gate directory");
    let mut run = start_reached_overload_product(
        directory.path().join("recovered-gate-capture.jsonl"),
        directory.path().join("recovered-gate-journal.jsonl"),
    )
    .await
    .expect("recovered-gate product starts");
    prepare_reached_overload_product(&mut run)
        .await
        .expect("recovered-gate product starts ready");
    run.project_recovery_reconciliation_gate_for_test();
    assert_eq!(
        run.mutation_halt(),
        Some(crate::coordinator::PmMutationHalt::RecoveryReconciliationRequired)
    );
    assert!(run.telemetry_overload_state().reconciliation_gate());

    let before = run.mutation_counters();
    let quote_clock = 1_900_000_000;
    run.schedule(
        PmOrderSide::Buy,
        crate::PmScheduledActionKind::QuoteEvaluation,
        quote_clock,
        quote_clock,
        1_700_000_000_000,
    )
    .expect("automatic quote is queued behind the recovery gate");
    run.service_turn(quote_clock)
        .expect("recovery gate suppresses rather than surfacing mutation halt");
    while run.pop_effect().is_some() {}
    assert_eq!(
        run.mutation_counters().quote_attempts(),
        before.quote_attempts()
    );
    assert_eq!(
        run.mutation_counters().quote_intents(),
        before.quote_intents()
    );

    complete_reached_overload_reconciliation(&mut run, 1, &[])
        .await
        .expect("fresh paired cut restores recovered mutation authority");
    assert_eq!(run.mutation_halt(), None);
    assert!(!run.telemetry_overload_state().reconciliation_gate());
    run.shutdown().await.expect("recovered-gate shutdown");
}

async fn all_recoverable_http_purposes_product_test() {
    let cases = [
        (
            RecoverableHttpPurpose::Account,
            PmLiveHttpQueryFailure::Timeout,
        ),
        (
            RecoverableHttpPurpose::OpenOrders,
            PmLiveHttpQueryFailure::Transport,
        ),
        (
            RecoverableHttpPurpose::OrderDetail,
            PmLiveHttpQueryFailure::MalformedResponse,
        ),
        (
            RecoverableHttpPurpose::Reconciliation,
            PmLiveHttpQueryFailure::IncompleteResponse,
        ),
    ];
    for (index, (purpose, failure_kind)) in cases.into_iter().enumerate() {
        let directory = tempfile::tempdir().expect("temporary purpose-matrix directory");
        let mut run = start_reached_overload_product(
            directory
                .path()
                .join(format!("purpose-{index}-capture.jsonl")),
            directory
                .path()
                .join(format!("purpose-{index}-journal.jsonl")),
        )
        .await
        .expect("purpose-matrix product starts");
        prepare_reached_overload_product(&mut run)
            .await
            .expect("purpose-matrix product starts ready");
        assert!(!run.telemetry_overload_state().reconciliation_gate());

        let mut issuer =
            PmLiveOccurrenceIssuer::new(PmConditionId::parse(CONDITION).expect("fixed condition"));
        issuer
            .start_for_test(ConnectionEpoch::new(1), private_clock(2_000))
            .expect("issuer mirrors product epoch");
        match purpose {
            RecoverableHttpPurpose::Account => {
                let ticket = issuer.begin_account_query(2_001).unwrap();
                let failure = issuer
                    .fail_account_query(ticket, private_clock(2_002), failure_kind)
                    .unwrap();
                run.ingest_account_live_failure(failure).unwrap();
            }
            RecoverableHttpPurpose::OpenOrders => {
                let ticket = issuer.begin_open_orders_query(2_001).unwrap();
                let failure = issuer
                    .fail_open_orders_query(ticket, private_clock(2_002), failure_kind)
                    .unwrap();
                run.ingest_open_orders_live_failure(failure).unwrap();
            }
            RecoverableHttpPurpose::OrderDetail => {
                let ticket = issuer.begin_order_detail_query(2_001).unwrap();
                let failure = issuer
                    .fail_order_detail_query(ticket, private_clock(2_002), failure_kind)
                    .unwrap();
                run.ingest_order_detail_live_failure(failure).unwrap();
            }
            RecoverableHttpPurpose::Reconciliation => {
                let ticket = issuer.begin_reconciliation_query(2_001).unwrap();
                let failure = issuer
                    .fail_reconciliation_query(ticket, private_clock(2_002), failure_kind)
                    .unwrap();
                run.ingest_reconciliation_live_failure(failure).unwrap();
            }
        }
        assert!(
            run.telemetry_overload_state().reconciliation_gate(),
            "purpose {index} must invalidate readiness at admission, before service",
        );
        run.service_turn(2_003)
            .expect("recoverable purpose failure is reduced");
        while run.pop_effect().is_some() {}
        for _ in 0..8 {
            let serviced = run.service_turn(2_003).expect("cancel work drains");
            while run.pop_effect().is_some() {}
            if serviced.total() == 0 {
                break;
            }
        }
        assert_eq!(run.halt(), None);
        assert_eq!(run.mutation_halt(), None);
        complete_reached_overload_reconciliation(&mut run, 1, &[])
            .await
            .expect("later complete compatible cut restores readiness");
        assert!(!run.telemetry_overload_state().reconciliation_gate());
        run.shutdown().await.expect("purpose-matrix shutdown");
    }
}

async fn normalization_http_failure_product_test() {
    recoverable_http_failure_product_test(PmLiveHttpQueryFailure::IncompleteResponse).await;
}

async fn service_http_failure_product_test() {
    recoverable_http_failure_product_test(PmLiveHttpQueryFailure::Transport).await;
}

async fn recoverable_http_failure_product_test(failure_kind: PmLiveHttpQueryFailure) {
    let directory = tempfile::tempdir().expect("temporary HTTP-failure ingress directory");
    let mut run = start_reached_overload_product(
        directory.path().join("http-failure-capture.jsonl"),
        directory.path().join("http-failure-journal.jsonl"),
    )
    .await
    .expect("product starts");
    prepare_reached_overload_product(&mut run)
        .await
        .expect("product starts from a ready complete cut");
    assert!(!run.telemetry_overload_state().reconciliation_gate());

    let mut issuer =
        PmLiveOccurrenceIssuer::new(PmConditionId::parse(CONDITION).expect("fixed condition"));
    issuer
        .start_for_test(ConnectionEpoch::new(1), private_clock(1_000))
        .expect("issuer mirrors the active product epoch");
    let ticket = issuer.begin_reconciliation_query(1_001).unwrap();
    let failure = issuer
        .fail_reconciliation_query(ticket, private_clock(1_002), failure_kind)
        .unwrap();
    run.ingest_reconciliation_live_failure(failure)
        .expect("purpose-bound failure reaches the reconciliation lane");
    assert!(
        run.telemetry_overload_state().reconciliation_gate(),
        "admission closes readiness before the failure can wait in the lane"
    );

    run.service_turn(1_003)
        .expect("recoverable dependency failure is reduced");
    while run.pop_effect().is_some() {}
    for _ in 0..8 {
        let serviced = run
            .service_turn(1_003)
            .expect("failure-triggered cancel work drains at the same logical clock");
        while run.pop_effect().is_some() {}
        if serviced.total() == 0 {
            break;
        }
    }
    assert_eq!(run.halt(), None);
    assert_eq!(run.mutation_halt(), None);
    assert!(run.telemetry_overload_state().reconciliation_gate());

    complete_reached_overload_reconciliation(&mut run, 1, &[])
        .await
        .expect("a later complete compatible cut restores readiness");
    assert!(!run.telemetry_overload_state().reconciliation_gate());
    assert_eq!(run.halt(), None);
    assert_eq!(run.mutation_halt(), None);
    run.shutdown().await.expect("clean product shutdown");
}

async fn native_rest_book_product_test() {
    let directory = tempfile::tempdir().expect("temporary REST-book ingress directory");
    let capture_path = directory.path().join("rest-book-live-ingress.jsonl");
    let mut run = start_reached_overload_product(
        capture_path.clone(),
        directory.path().join("rest-book-live-journal.jsonl"),
    )
    .await
    .expect("product starts");
    let raw = rest_book();

    let captured = {
        let mut ingress = run.public_ingress();
        ingress
            .record_pm_connection_started(60)
            .await
            .expect("connection start");
        ingress
            .record_pm_subscription_sent(80)
            .await
            .expect("subscription");
        let mut sink = ingress.rest_book_capture_sink();
        sink.deliver_native_rest_book(
            PmRestBookPurpose::Seed,
            reap_polymarket_live_adapter::PmRestResponseClock::test_support_new(
                1_700_000_000_000_000_100,
                100,
            )
            .unwrap(),
            raw.as_bytes(),
        )
        .await
        .expect("durable REST-book capture")
    };
    assert_eq!(captured.purpose(), PmRestBookPurpose::Seed);
    let mut batch = captured.into_batch();
    assert_eq!(batch.books().len(), 1);
    let flow = batch
        .take_snapshot_flow()
        .expect("REST book carries the session-owned snapshot flow");
    let delivery = batch
        .into_books()
        .into_iter()
        .next()
        .expect("REST book carries one classified delivery");
    {
        let mut ingress = run.public_ingress();
        let _ = ingress
            .commit_then_enqueue_pm_snapshot(delivery, flow)
            .await
            .expect("durably captured REST book reaches the existing reducer queue");
    }
    assert_eq!(
        run.service_turn(101)
            .expect("REST-book reduction is serviced")
            .for_lane(PmLaneKind::Public),
        Some(1)
    );

    run.shutdown().await.expect("clean product shutdown");
    let capture = std::fs::read_to_string(capture_path).expect("durable capture artifact");
    assert!(capture.contains(r#""transport":"book_rest""#));
}

async fn authenticated_live_frame_product_test() {
    let directory = tempfile::tempdir().expect("temporary live-ingress directory");
    let mut run = start_reached_overload_product(
        directory.path().join("live-ingress-capture.jsonl"),
        directory.path().join("live-ingress-journal.jsonl"),
    )
    .await
    .expect("product starts");

    let mut issuer =
        PmLiveOccurrenceIssuer::new(PmConditionId::parse(CONDITION).expect("fixed condition"));
    let connection = issuer
        .start_for_test(ConnectionEpoch::new(1), private_clock(120))
        .expect("sealed live connection occurrence");
    run.connect_private_live(connection)
        .expect("live reconnect reaches the sole owner");
    run.service_turn(121).expect("reconnect is reduced");
    while run.pop_effect().is_some() {}
    run.service_turn(122)
        .expect("reconnect quote check is reduced");
    while run.pop_effect().is_some() {}

    let frame = parse_live_user_frame(
        &serde_json::to_vec(&json!({
            "event_type": "order",
            "id": ORDER,
            "market": CONDITION,
            "asset_id": TOKEN.to_string(),
            "side": "BUY",
            "original_size": "10",
            "size_matched": "0",
            "price": "0.42",
            "type": "PLACEMENT",
            "maker_address": PM_FUNDER,
            "expiration": "0",
            "order_type": "GTC",
            "outcome": "Yes",
            "status": "LIVE",
            "created_at": "1700000000",
            "associate_trades": null,
            "owner": OWNER,
            "order_owner": OWNER,
            "timestamp": "1700000000000"
        }))
        .expect("user JSON"),
    )
    .expect("typed user frame");
    let frame = L2Credentials::bind(
        AUTH_ADDRESS,
        L2CredentialInput::new(OWNER.into(), API_SECRET.into(), PASSPHRASE.into()),
    )
    .expect("fixed credentials")
    .bind_user_stream_frame(frame)
    .expect("credential-owned user frame");
    let input = issuer
        .issue_user_frame_for_test(ConnectionEpoch::new(1), private_clock(130), frame)
        .expect("sealed live frame occurrence");
    let report = run
        .ingest_private_live(input)
        .expect("live frame is admitted");
    assert!(matches!(
        report,
        PmLiveIngressReport::Private { foreign } if foreign.is_empty()
    ));

    let queued = run.scheduler_metrics(131).expect("queue metrics");
    assert_eq!(queued.lane(PmLaneKind::Private).unwrap().queue().depth(), 1);
    let serviced = run
        .service_turn(132)
        .expect("live private lane is serviced");
    assert_eq!(serviced.for_lane(PmLaneKind::Private), Some(1));
    assert_eq!(
        run.scheduler_metrics(133)
            .expect("post-service metrics")
            .lane(PmLaneKind::Private)
            .unwrap()
            .queue()
            .depth(),
        0
    );

    run.shutdown().await.expect("clean product shutdown");
}

fn run_product_test<F, Fut>(test: F)
where
    F: FnOnce() -> Fut + Send + 'static,
    Fut: Future<Output = ()> + 'static,
{
    std::thread::Builder::new()
        .name("live-ingress-product".to_owned())
        .stack_size(16 * 1024 * 1024)
        .spawn(move || {
            tokio::runtime::Builder::new_current_thread()
                .enable_all()
                .build()
                .expect("live-ingress product runtime")
                .block_on(test());
        })
        .expect("spawn bounded large-schema live-ingress test")
        .join()
        .expect("live-ingress product test thread joins");
}
