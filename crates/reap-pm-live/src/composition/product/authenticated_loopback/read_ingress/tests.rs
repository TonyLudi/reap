use std::time::Duration;

use reap_pm_core::{ConnectionEpoch, ReceivedEventClock};
use reap_polymarket_auth::{L2CredentialInput, L2Credentials};
use reap_polymarket_live_adapter::{
    PmPrivateConnectivityOwner, PmPrivateHttpConfig, PmProductClockOwner,
    PmPublicConnectivityOwner, PmPublicHttpConfig, PmPublicWsConfig, PmUserWsConfig,
};
use reap_polymarket_wire::{PmBookParserConfig, PmWireScope};
use tokio::{
    io::{AsyncReadExt as _, AsyncWriteExt as _},
    net::TcpListener,
    sync::{mpsc, oneshot},
};

use reap_polymarket_live_adapter::PmRestBookPurpose;

use crate::evidence::{prepare_reached_overload_product, start_reached_overload_product};
use crate::private_monitor::{PmLiveHttpQueryFailure, PmLiveOccurrenceIssuer};
use crate::{PmDurableRecordKind, PmProductEffect, PmScheduledActionKind};

const PARTIAL_ACCOUNT_ADDRESS: &str = "0xf39Fd6e51aad88F6F4ce6aB8827279cffFb92266";
const PARTIAL_ACCOUNT_API_KEY: &str = "00000000-0000-4000-8000-000000000001";
const PARTIAL_ACCOUNT_SECRET: &str = "AAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAA=";
const PARTIAL_ACCOUNT_PASSPHRASE: &str = "partial-account-test";
const PARTIAL_ACCOUNT_SECONDS: u64 = 1_780_449_126;

#[tokio::test]
async fn bounded_request_ack_does_not_expose_an_unbounded_or_shared_owner_channel() {
    let (sender, mut receiver) = mpsc::channel(1);
    let (acknowledgement, response) = oneshot::channel();
    let round_trip = tokio::spawn(async move {
        super::channel::bounded_round_trip_for_test(&sender, 7, response).await
    });
    assert_eq!(receiver.recv().await, Some(7));
    acknowledgement.send(Ok(9)).unwrap();
    assert_eq!(round_trip.await.unwrap().unwrap(), 9);
}

#[test]
fn source_keeps_canonical_owners_out_of_socket_tasks() {
    let root = std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("src/composition/product/authenticated_loopback/read_ingress");
    let mut source = String::new();
    for file in [
        "mod.rs",
        "channel.rs",
        "public.rs",
        "user.rs",
        "http.rs",
        "book.rs",
        "shutdown.rs",
    ] {
        source.push_str(&std::fs::read_to_string(root.join(file)).unwrap());
    }
    assert!(!source.contains("Arc<Mutex"));
    assert!(!source.contains("unbounded_channel"));
    assert!(!source.contains("PmOpenOrdersAssembly) ->"));
    assert!(!source.contains("PmTradesAssembly) ->"));
    assert!(source.contains("public_ws.transport_policy().initial_connection_epoch()"));
    assert!(!source.contains("epoch.value() != 1"));
}

#[test]
fn public_cold_open_uses_the_exact_transport_authorized_epoch() {
    use reap_pm_core::{PmConditionId, PmMarketId, PmTokenId, U256};

    let state = super::public::PmPublicIngressState::new(
        PmConditionId::parse(&format!("0x{}", "11".repeat(32))).unwrap(),
        PmMarketId::parse(&format!("0x{}", "22".repeat(32))).unwrap(),
        PmTokenId::new(U256::from_u64(123)).unwrap(),
        ConnectionEpoch::new(2),
    );
    assert!(
        state
            .validate_authorized_open(ConnectionEpoch::new(2))
            .is_ok(),
        "a restart may cold-open at the exact configured transport epoch"
    );
    assert!(matches!(
        state.validate_authorized_open(ConnectionEpoch::new(1)),
        Err(super::PmReadIngressActorError::PublicProtocol(
            "public connection epoch was not actor-authorized"
        ))
    ));
    assert!(matches!(
        state.validate_authorized_open(ConnectionEpoch::new(3)),
        Err(super::PmReadIngressActorError::PublicProtocol(
            "public connection epoch was not actor-authorized"
        ))
    ));
}

#[test]
fn controlled_teardown_keeps_book_producers_ahead_of_private_http() {
    use super::shutdown::{book_shutdown_ready, http_shutdown_ready};

    assert!(!book_shutdown_ready(true, true, false));
    assert!(!book_shutdown_ready(true, false, true));
    assert!(!book_shutdown_ready(false, false, false));
    assert!(book_shutdown_ready(true, false, false));

    assert!(!http_shutdown_ready(false, true, false, true, true));
    assert!(!http_shutdown_ready(true, true, true, true, true));
    assert!(!http_shutdown_ready(true, true, false, false, true));
    assert!(!http_shutdown_ready(true, true, false, true, false));
    assert!(http_shutdown_ready(true, true, false, true, true));

    let source = include_str!("mod.rs");
    let teardown = source
        .split("pub(super) async fn shutdown")
        .nth(1)
        .unwrap()
        .split("fn merge_book_demand")
        .next()
        .unwrap();
    let book = teardown.find("book_shutdown_ready(").unwrap();
    let refresh = teardown.find("let refresh_quiescent").unwrap();
    let http = teardown.find("http_shutdown_ready(").unwrap();
    assert!(book < refresh && refresh < http);
    assert!(teardown.contains("peek_reconciliation_refresh_effect"));
    assert!(teardown.contains("read_shutdown_unresolved_counts"));
    assert!(teardown.contains("actor.service_after_ingress()"));
    assert!(teardown.contains("book_obligation_unresolved"));
}

#[test]
fn seed_in_flight_retains_a_higher_priority_resynchronization_demand() {
    assert_eq!(
        super::merge_book_demand(Some(PmRestBookPurpose::Seed), PmRestBookPurpose::Resync).unwrap(),
        Some(PmRestBookPurpose::Resync),
    );
    assert!(
        super::merge_book_demand(Some(PmRestBookPurpose::Resync), PmRestBookPurpose::Seed).is_err(),
        "an initial seed cannot overwrite canonical resynchronization demand",
    );
}

#[test]
fn actor_rotation_and_transport_ordering_are_source_pinned() {
    let root = std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("src/composition/product/authenticated_loopback/read_ingress");
    let actor = std::fs::read_to_string(root.join("mod.rs")).unwrap();
    for source in ["0 =>", "1 =>", "2 =>", "3 =>"] {
        assert!(
            actor.contains(source),
            "fixed rotation omitted source {source}"
        );
    }
    assert!(actor.contains("self.next_source = (source + 1) % 4"));
    let subscription = actor
        .find("PmPublicServiceOutcome::SubscriptionReady")
        .unwrap();
    let seed = actor[subscription..]
        .find("request_rest_book(PmRestBookPurpose::Seed)")
        .unwrap();
    assert!(
        seed > 0,
        "subscription admission did not trigger the REST seed"
    );
    assert!(actor.contains("PmPublicServiceOutcome::ResyncRequired"));
    assert!(actor.contains("request_rest_book(PmRestBookPurpose::Resync)"));

    let user = std::fs::read_to_string(root.join("user.rs")).unwrap();
    let opened = user.find("PmUserWsEvent::ConnectionOpened").unwrap();
    let subscribed = user.find("PmUserWsEvent::SubscriptionSent").unwrap();
    let canonical = user.find("start_user_connection(observation)").unwrap();
    assert!(opened < subscribed && subscribed < canonical);

    let public = std::fs::read_to_string(root.join("public.rs")).unwrap();
    let metadata = public.find("issue_and_enqueue_pm_metadata").unwrap();
    let ready = public
        .find("PmPublicServiceOutcome::SubscriptionReady")
        .unwrap();
    assert!(
        metadata < ready,
        "book seed became ready before durable metadata"
    );
}

#[test]
fn read_dispatch_reserves_before_peek_ticket_pop_and_infallible_send() {
    let source = include_str!("mod.rs");
    let dispatch = source
        .split("fn dispatch_next_refresh")
        .nth(1)
        .unwrap()
        .split("/// Polls exactly one")
        .next()
        .unwrap();
    let reserve = dispatch.find("try_reserve()").unwrap();
    let peek = dispatch.find("peek_reconciliation_refresh_effect").unwrap();
    let ticket = dispatch.find("begin_open_orders_query").unwrap();
    let pop = dispatch.find("pop_reconciliation_refresh_effect").unwrap();
    let send = dispatch.find("permit.send_open_orders()").unwrap();
    assert!(reserve < peek && peek < ticket && ticket < pop && pop < send);
    assert!(dispatch.contains("current_fill_query_cursor"));
    assert!(dispatch.contains("PmRefreshEffectKind::OrderDetail(requested_order)"));
}

#[tokio::test(flavor = "current_thread")]
async fn collateral_success_then_conditional_failure_never_delivers_a_complete_account_cut() {
    let (origin, mut requests, server) = partial_account_server().await;
    let config = crate::evidence::connectivity_config();
    let metadata = config.public().expected_metadata();
    let wire_scope = PmWireScope::new(
        metadata.condition(),
        metadata.market(),
        metadata.outcome().token(),
    );

    let private_http = PmPrivateHttpConfig::loopback_evidence(
        &origin,
        Duration::from_millis(100),
        Duration::from_secs(2),
        wire_scope,
    )
    .expect("partial-account private HTTP config");
    let user_ws = PmUserWsConfig::loopback_evidence(
        "ws://127.0.0.1:9/ws/user",
        metadata.condition(),
        Duration::from_millis(100),
        Duration::from_secs(2),
        Duration::from_millis(100),
        Duration::from_millis(50),
        4_096,
        1,
        Duration::from_millis(10),
        8,
        ConnectionEpoch::new(1),
    )
    .expect("partial-account user WS config");
    let private = PmPrivateConnectivityOwner::new(
        private_http,
        user_ws,
        L2Credentials::bind(
            PARTIAL_ACCOUNT_ADDRESS,
            L2CredentialInput::new(
                PARTIAL_ACCOUNT_API_KEY.into(),
                PARTIAL_ACCOUNT_SECRET.into(),
                PARTIAL_ACCOUNT_PASSPHRASE.into(),
            ),
        )
        .expect("partial-account credentials"),
    )
    .expect("partial-account private connectivity");
    let (authenticated_http, _authenticated_user_ws, credential_supervisor) = private
        .split()
        .expect("split partial-account credentials")
        .into_read_roles();

    let public_http = PmPublicHttpConfig::loopback_evidence(
        &origin,
        Duration::from_millis(100),
        Duration::from_secs(2),
    )
    .expect("partial-account public HTTP config");
    let parser = PmBookParserConfig::new_condition_bound(
        wire_scope,
        metadata.tick(),
        metadata.minimum_order_size(),
        metadata.negative_risk(),
    );
    let public_ws = PmPublicWsConfig::loopback_evidence(
        "ws://127.0.0.1:9/ws/market",
        wire_scope,
        Duration::from_millis(100),
        Duration::from_secs(2),
        Duration::from_millis(100),
        Duration::from_millis(50),
        4_096,
        1,
        Duration::from_millis(10),
        8,
        ConnectionEpoch::new(1),
    )
    .expect("partial-account public WS config");
    let public_roles = PmPublicConnectivityOwner::new(
        public_http,
        parser,
        public_ws,
        PmProductClockOwner::system(),
    )
    .expect("partial-account clocked public connectivity")
    .into_roles();
    let (_, _, read_server_time, private_read_clock, _, _, _, _, _, _) = public_roles.into_roles();

    let (http_dispatch, http_receiver) = super::http::pm_http_read_channel();
    let http_task = tokio::spawn(super::http::run_http_worker(
        authenticated_http,
        read_server_time,
        private_read_clock,
        http_receiver,
    ));
    let response = http_dispatch
        .try_reserve()
        .expect("partial-account worker capacity")
        .send_account();
    let result = tokio::time::timeout(Duration::from_secs(2), response)
        .await
        .expect("partial-account response timeout")
        .expect("partial-account response channel");
    let (completed_at, failure) = match result {
        super::http::PmAuthenticatedHttpReadResult::Recoverable {
            completed_at,
            failure,
        } => (completed_at, failure),
        other => panic!("partial account escaped as a non-recoverable result: {other:?}"),
    };
    assert_eq!(failure, PmLiveHttpQueryFailure::HttpStatus);

    let observed_requests = [
        requests.recv().await.expect("first /time request"),
        requests.recv().await.expect("collateral request"),
        requests.recv().await.expect("second /time request"),
        requests.recv().await.expect("conditional request"),
    ];
    assert_eq!(observed_requests[0], "GET /time HTTP/1.1");
    assert!(
        observed_requests[1]
            .starts_with("GET /balance-allowance?asset_type=COLLATERAL&signature_type=0 HTTP/1.1")
    );
    assert_eq!(observed_requests[2], "GET /time HTTP/1.1");
    assert!(observed_requests[3].starts_with(
        "GET /balance-allowance?asset_type=CONDITIONAL&token_id=123&signature_type=0 HTTP/1.1"
    ));

    let directory = tempfile::tempdir().expect("temporary partial-account product directory");
    let mut run = start_reached_overload_product(
        directory.path().join("partial-account-capture.jsonl"),
        directory.path().join("partial-account-journal.jsonl"),
    )
    .await
    .expect("partial-account product starts");
    prepare_reached_overload_product(&mut run)
        .await
        .expect("partial-account product becomes ready");
    while run.pop_effect().is_some() {}
    let before = run.private_projection_for_test();
    let account_before = before.account_snapshot();
    let account_counters_before = before.account_counters();
    let quote_intents_before = run.mutation_counters().quote_intents();

    let mut issuer = PmLiveOccurrenceIssuer::new(metadata.condition());
    issuer
        .start_for_test(ConnectionEpoch::new(1), private_test_clock(1))
        .expect("partial-account issuer starts");
    for request_ns in 2..6 {
        let _burned_ticket = issuer
            .begin_open_orders_query(request_ns)
            .expect("burned request preserves live sequence");
    }
    let ticket = issuer
        .begin_account_query(6)
        .expect("partial-account query ticket");
    let received =
        super::private_read_received_clock(completed_at).expect("partial-account completion clock");
    let failure_input = issuer
        .fail_account_query(ticket, received, failure)
        .expect("partial-account failure occurrence");
    run.ingest_account_live_failure(failure_input)
        .expect("partial-account failure reaches reconciliation lane");
    let service_ns = received.monotonic_receive_ns() + 1;
    for offset in 0..3 {
        run.service_turn(service_ns + offset)
            .expect("partial-account failure services");
        while run.pop_effect().is_some() {}
    }

    let after = run.private_projection_for_test();
    let account_after = after.account_snapshot();
    assert_eq!(account_after.source(), account_before.source());
    assert_eq!(account_after.snapshot(), account_before.snapshot());
    assert_eq!(account_after.completion(), account_before.completion());
    assert_eq!(
        account_after.local_wall_receive_ns(),
        account_before.local_wall_receive_ns()
    );
    assert_eq!(
        account_after.monotonic_receive_ns(),
        account_before.monotonic_receive_ns()
    );
    assert_eq!(account_after.collateral(), account_before.collateral());
    assert_eq!(
        account_after.outcome_balance(),
        account_before.outcome_balance()
    );
    assert_eq!(account_after.position(), account_before.position());
    assert_eq!(
        account_after.unknown_balance_rows(),
        account_before.unknown_balance_rows()
    );
    assert_eq!(
        account_after.unknown_allowance_rows(),
        account_before.unknown_allowance_rows()
    );
    assert_eq!(
        account_after.unknown_position_rows(),
        account_before.unknown_position_rows()
    );
    assert!(account_before.monotonic_service_ns().is_some());
    assert_eq!(account_after.monotonic_service_ns(), None);
    assert_eq!(after.account_counters(), account_counters_before);

    let quote_ns = service_ns + 10;
    run.schedule(
        reap_pm_core::PmOrderSide::Buy,
        PmScheduledActionKind::QuoteEvaluation,
        quote_ns,
        quote_ns,
        1_700_000_000_000,
    )
    .expect("partial-account readiness check schedules");
    run.service_turn(quote_ns)
        .expect("partial-account readiness check services");
    let effects = std::iter::from_fn(|| run.pop_effect()).collect::<Vec<_>>();
    assert!(effects.iter().all(|effect| {
        !matches!(effect, PmProductEffect::PlaceGtcPostOnly(_))
            && !matches!(
                effect,
                PmProductEffect::DurableRecord(record)
                    if record.kind() == PmDurableRecordKind::QuoteIntent
            )
    }));
    assert_eq!(
        run.mutation_counters().quote_intents(),
        quote_intents_before
    );

    let shutdown_receipt = http_dispatch
        .try_request_shutdown()
        .expect("partial-account HTTP shutdown dispatch");
    shutdown_receipt
        .await
        .expect("partial-account HTTP shutdown receipt");
    http_task
        .await
        .expect("partial-account HTTP task joins")
        .expect("partial-account HTTP task exits cleanly");
    credential_supervisor
        .shutdown()
        .await
        .expect("partial-account credential authority stops");
    server.await.expect("partial-account server joins");
    let _ = run.shutdown().await;
}

fn private_test_clock(monotonic_ns: u64) -> ReceivedEventClock {
    ReceivedEventClock::new(None, 1_700_000_000_000_000_000 + monotonic_ns, monotonic_ns)
        .expect("fixed private test clock")
}

async fn partial_account_server() -> (String, mpsc::Receiver<String>, tokio::task::JoinHandle<()>) {
    let listener = TcpListener::bind("127.0.0.1:0")
        .await
        .expect("bind partial-account server");
    let address = listener.local_addr().expect("partial-account address");
    let (requests, request_rx) = mpsc::channel(4);
    let task = tokio::spawn(async move {
        for ordinal in 0..4 {
            let (mut stream, _) = listener.accept().await.expect("partial-account accept");
            let mut raw = Vec::new();
            let mut chunk = [0_u8; 1_024];
            loop {
                let read = stream.read(&mut chunk).await.expect("partial-account read");
                if read == 0 {
                    break;
                }
                raw.extend_from_slice(&chunk[..read]);
                if raw.windows(4).any(|window| window == b"\r\n\r\n") {
                    break;
                }
            }
            let head = std::str::from_utf8(&raw).expect("partial-account request UTF-8");
            requests
                .send(
                    head.lines()
                        .next()
                        .expect("partial-account request line")
                        .to_owned(),
                )
                .await
                .expect("record partial-account request");
            if ordinal == 1 || ordinal == 3 {
                assert!(head.to_ascii_lowercase().contains("poly_address:"));
                assert!(head.to_ascii_lowercase().contains("poly_signature:"));
            }
            let (status, body) = match ordinal {
                0 | 2 => (200, PARTIAL_ACCOUNT_SECONDS.to_string()),
                1 => (
                    200,
                    r#"{"balance":"10000000000","allowances":{"0xE111180000d2663C0091e4f400237545B87B996B":"10000000000"}}"#
                        .to_owned(),
                ),
                3 => (503, r#"{"error":"synthetic conditional failure"}"#.to_owned()),
                _ => unreachable!(),
            };
            let reason = if status == 200 {
                "OK"
            } else {
                "Service Unavailable"
            };
            let response = format!(
                "HTTP/1.1 {status} {reason}\r\nContent-Type: application/json\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{body}",
                body.len(),
            );
            stream
                .write_all(response.as_bytes())
                .await
                .expect("partial-account response");
        }
    });
    (format!("http://{address}"), request_rx, task)
}
