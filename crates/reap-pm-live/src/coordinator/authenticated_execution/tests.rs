use reap_pm_core::{OkxInstrumentId, OkxReferenceInstrument};
use reap_pm_live_contracts::{
    PmAccountConnectivityConfig, PmConnectivityConfig, PmPublicConnectivityConfig,
};

use super::{
    PmAuthenticatedDurabilityStage, PmAuthenticatedExecutionError, combine_shutdown_results,
    validate_configuration_fingerprint,
};

#[test]
fn durability_barriers_remain_operation_ordered_and_distinct() {
    assert_ne!(
        PmAuthenticatedDurabilityStage::Prepared,
        PmAuthenticatedDurabilityStage::DispatchAuthorized
    );
    assert_ne!(
        PmAuthenticatedDurabilityStage::DispatchAuthorized,
        PmAuthenticatedDurabilityStage::Result
    );
}

#[test]
fn same_account_cannot_pair_roles_from_a_different_public_configuration() {
    let expected = crate::evidence::connectivity_config();
    let foreign_public = PmPublicConnectivityConfig::derive_goal_f(
        OkxReferenceInstrument::index(OkxInstrumentId::new("ETH-USDT").unwrap()),
        expected.public().expected_metadata(),
        expected.public().okx_route(),
        expected.public().polymarket_route(),
    )
    .unwrap();
    let foreign_account = PmAccountConnectivityConfig::derive_goal_f(
        &foreign_public,
        expected.account().account_scope(),
        expected.account().account_route(),
    )
    .unwrap();
    let foreign = PmConnectivityConfig::new(foreign_public, foreign_account).unwrap();

    assert_eq!(
        expected.account().account_scope(),
        foreign.account().account_scope()
    );
    assert_ne!(
        expected.public().configuration_fingerprint(),
        foreign.public().configuration_fingerprint()
    );
    assert!(matches!(
        validate_configuration_fingerprint(&expected, foreign.public().configuration_fingerprint()),
        Err(PmAuthenticatedExecutionError::ConfigurationFingerprintMismatch)
    ));
    validate_configuration_fingerprint(&expected, expected.public().configuration_fingerprint())
        .unwrap();
}

#[test]
fn post_send_failures_retain_evidence_then_return_the_primary_and_halt_the_worker() {
    let source = include_str!("worker.rs");
    for snippet in [
        "let (error, unresolved) = failure.into_parts();\n                self.retain_result_durability_failure(unresolved);\n                return Err(error);",
        "let (error, unresolved) = failure.into_parts();\n                self.retain_result_durability_failure(intent, unresolved);\n                return Err(error);",
        "self.retain_unclassified_post_send(grant);\n                return Err(error);",
        "self.retain_unclassified_post_send(intent, grant);\n                return Err(error);",
        "Err(_) => self.halted = true,",
    ] {
        assert!(
            source.contains(snippet),
            "post-send primary-error retention choreography changed: {snippet}",
        );
    }
    assert!(!source.contains("retain_result_durability_failure(unresolved)?"));
    assert!(!source.contains("retain_result_durability_failure(intent, unresolved)?"));
    assert!(!source.contains("retain_result_admission_failure(grant, result)?"));
    assert!(!source.contains("retain_result_admission_failure(intent, grant, result)?"));
    assert!(
        source
            .matches("exclusive place-worker reservation forbids a second post-send quarantine")
            .count()
            >= 3
    );
    assert!(
        source
            .matches("exclusive cancel-worker reservation forbids a second post-send quarantine")
            .count()
            >= 3
    );
    assert!(source.contains("|| self.quarantined_post_send.is_some()"));
}

#[test]
fn exact_body_identity_stays_runtime_only_while_semantics_cross_durability() {
    let worker = include_str!("worker.rs");
    let outcome = include_str!("outcome.rs");

    assert_eq!(
        worker
            .matches(
                "let runtime_exact_body_commitment = retained.runtime_exact_body_commitment();",
            )
            .count(),
        2,
    );
    assert_eq!(
        worker
            .matches(
                "let semantic_request_commitment = retained.semantic_request_commitment().bytes();",
            )
            .count(),
        2,
    );
    assert_eq!(
        outcome
            .matches("outcome.runtime_exact_body_commitment() != runtime_exact_body_commitment",)
            .count(),
        2,
    );
    assert_eq!(
        outcome
            .matches("outcome.semantic_request_commitment().bytes()")
            .count(),
        2,
    );
    assert_eq!(
        outcome.matches("grant.matches_retained_request(").count(),
        2
    );

    for source in [worker, outcome] {
        for forbidden in [
            "runtime_only_bytes",
            "let body_commitment =",
            "retained.commitment()",
            "outcome.commitment()",
        ] {
            assert!(
                !source.contains(forbidden),
                "runtime/durable commitment boundary regressed: {forbidden}",
            );
        }
    }
    let quarantine = worker
        .split_once("struct PmQuarantinedPlaceAuthentication")
        .expect("worker quarantine definitions")
        .1;
    assert!(!quarantine.contains("runtime_exact_body_commitment:"));
}

#[test]
fn shutdown_aggregation_never_masks_a_live_journal_role_or_credential_failure() {
    let combined = combine_shutdown_results(
        Err(reap_polymarket_live_adapter::PmLiveAdapterError::CredentialAuthorityTaskFailed),
        Err(PmAuthenticatedExecutionError::JournalRoleStillLive),
    )
    .expect_err("both shutdown failures must remain visible");
    assert!(matches!(
        combined,
        PmAuthenticatedExecutionError::ShutdownBoth {
            credential,
            journal,
        } if *credential
            == reap_polymarket_live_adapter::PmLiveAdapterError::CredentialAuthorityTaskFailed
            && matches!(*journal, PmAuthenticatedExecutionError::JournalRoleStillLive)
    ));

    assert!(matches!(
        combine_shutdown_results(
            Ok(()),
            Err(PmAuthenticatedExecutionError::JournalRoleStillLive),
        ),
        Err(PmAuthenticatedExecutionError::JournalRoleStillLive)
    ));
}

mod durability_before_http_write {
    use std::{
        num::NonZeroU64,
        path::{Path, PathBuf},
        sync::{
            Arc,
            atomic::{AtomicUsize, Ordering},
        },
        time::Duration,
    };

    use reap_pm_core::{
        ConnectionEpoch, EvmAddress, PmAccountScope, PmClientOrderId, PmClientOrderKey, PmFunderId,
        PmOrderSalt, PmOrderSide, PmPrice, PmQuantity, PmSignerId, PmVenueOrderId, PmVenueOrderKey,
        U256, exact_order_amounts,
    };
    use reap_pm_live_contracts::{PmAccountConnectivityConfig, PmConnectivityConfig};
    use reap_pm_state::{
        PmExactReservation, PmOwnedCancelRequestApply, PmOwnedIntentId, PmOwnedOrderLifecycle,
        PmOwnedQuoteAdmission, PmOwnedQuoteIntent, PmOwnedQuoteSlotKey, PmOwnedSubmitResult,
    };
    use reap_polymarket_adapter::{PmFixedMutationPreparation, PmFixtureInstrumentScope};
    use reap_polymarket_auth::{
        CredentialSlotId, EoaPrivateKeyInput, FixedEoaSigner, L2CredentialInput, L2Credentials,
    };
    use reap_polymarket_live_adapter::{
        PmAuthenticatedHttpOwner, PmAuthenticatedUserWsRole, PmCredentialAuthoritySupervisor,
        PmExactOwnedCancelLoopbackRole, PmFixedPlaceLoopbackRole, PmLoopbackMutationAuthError,
        PmLoopbackMutationConfig, PmLoopbackMutationConnectivityOwner, PmLoopbackServerTimeScript,
        PmPrivateHttpConfig, PmUserWsConfig,
    };
    use reap_polymarket_wire::PmWireScope;
    use tokio::{
        io::{AsyncReadExt, AsyncWriteExt},
        net::{TcpListener, TcpStream},
        sync::{mpsc, oneshot},
        task::JoinHandle,
        time::timeout,
    };

    use super::super::journal_role::PmAuthenticatedJournalRuntime;
    use super::super::{
        PmAuthenticatedCancelWorker, PmAuthenticatedExecutionError, PmAuthenticatedPlaceWorker,
    };
    use crate::{
        authenticated_journal::{
            PmAuthenticatedCancelResultKindV1, PmAuthenticatedJournalScopeV1,
            PmAuthenticatedJournalWriteLatch, PmAuthenticatedPlaceResultKindV1,
            PmAuthenticatedRecoveredResultClassificationV1,
            PmAuthenticatedUnresolvedOperationKindV1, PmAuthenticatedUnresolvedReasonV1,
        },
        coordinator::dispatch::{PmPreparedCancelDispatch, PmPreparedPlaceDispatch},
        journal::PmJournalAuthenticatedClassificationV1,
    };

    const TEST_KEY: &str = "0xac0974bec39a17e36ba4a6b4d238ff944bacb478cbed5efcae784d7bf4f2ff80";
    const ADDRESS: &str = "0xf39Fd6e51aad88F6F4ce6aB8827279cffFb92266";
    const API_KEY: &str = "00000000-0000-4000-8000-000000000001";
    const API_SECRET: &str = "AAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAA=";
    const PASSPHRASE: &str = "durability-barrier-test";
    const AUTH_SECONDS: u64 = 1_780_449_126;
    const VENUE_ORDER: &str = "0x5555555555555555555555555555555555555555555555555555555555555555";

    struct ObservedHttpWrite {
        request: Vec<u8>,
        release_response: oneshot::Sender<()>,
    }

    enum HttpProbeResponse {
        Rejection,
        Disconnect,
        Partial {
            body_prefix: &'static [u8],
            declared_length: usize,
        },
    }

    async fn read_request(stream: &mut TcpStream, writes: &AtomicUsize) -> Vec<u8> {
        let mut raw = Vec::new();
        let mut chunk = [0_u8; 1_024];
        let mut expected = None;
        loop {
            let read = stream.read(&mut chunk).await.expect("read HTTP request");
            if read == 0 {
                break;
            }
            if raw.is_empty() {
                writes.fetch_add(1, Ordering::SeqCst);
            }
            raw.extend_from_slice(&chunk[..read]);
            if expected.is_none()
                && let Some(header_end) = raw.windows(4).position(|window| window == b"\r\n\r\n")
            {
                let header_end = header_end + 4;
                let head = std::str::from_utf8(&raw[..header_end]).expect("HTTP request headers");
                let content_length = head
                    .lines()
                    .find_map(|line| {
                        let (name, value) = line.split_once(':')?;
                        name.eq_ignore_ascii_case("content-length")
                            .then(|| value.trim().parse::<usize>().expect("content length"))
                    })
                    .unwrap_or(0);
                expected = Some(header_end + content_length);
            }
            if expected.is_some_and(|length| raw.len() >= length) {
                break;
            }
        }
        raw
    }

    async fn start_http_probe() -> (
        String,
        mpsc::UnboundedReceiver<ObservedHttpWrite>,
        Arc<AtomicUsize>,
        JoinHandle<usize>,
    ) {
        start_http_probe_with([HttpProbeResponse::Rejection, HttpProbeResponse::Rejection]).await
    }

    async fn start_http_probe_with(
        responses: [HttpProbeResponse; 2],
    ) -> (
        String,
        mpsc::UnboundedReceiver<ObservedHttpWrite>,
        Arc<AtomicUsize>,
        JoinHandle<usize>,
    ) {
        let listener = TcpListener::bind("127.0.0.1:0")
            .await
            .expect("bind HTTP probe");
        let address = listener.local_addr().expect("HTTP probe address");
        let (observed_tx, observed_rx) = mpsc::unbounded_channel();
        let writes = Arc::new(AtomicUsize::new(0));
        let server_writes = Arc::clone(&writes);
        let server = tokio::spawn(async move {
            for response_plan in responses {
                let (mut stream, _) = timeout(Duration::from_secs(2), listener.accept())
                    .await
                    .expect("expected HTTP write before probe timeout")
                    .expect("accept HTTP probe connection");
                let request = read_request(&mut stream, &server_writes).await;
                let (release_response, wait_for_release) = oneshot::channel();
                observed_tx
                    .send(ObservedHttpWrite {
                        request: request.clone(),
                        release_response,
                    })
                    .expect("deliver observed HTTP write");
                let _ = wait_for_release.await;
                match response_plan {
                    HttpProbeResponse::Rejection => {
                        let request_line = std::str::from_utf8(&request)
                            .expect("UTF-8 HTTP request")
                            .lines()
                            .next()
                            .expect("HTTP request line");
                        let body = if request_line == "POST /order HTTP/1.1" {
                            r#"{"success":false,"errorMsg":"synthetic rejection","orderID":"","status":"","makingAmount":"","takingAmount":"","transactionsHashes":[],"tradeIDs":[]}"#.to_owned()
                        } else {
                            assert_eq!(request_line, "DELETE /order HTTP/1.1");
                            format!(
                                r#"{{"canceled":[],"not_canceled":{{"{VENUE_ORDER}":"synthetic rejection"}}}}"#
                            )
                        };
                        let response = format!(
                            "HTTP/1.1 200 OK\r\nContent-Type: application/json\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{body}",
                            body.len(),
                        );
                        stream
                            .write_all(response.as_bytes())
                            .await
                            .expect("write HTTP probe response");
                    }
                    HttpProbeResponse::Disconnect => {}
                    HttpProbeResponse::Partial {
                        body_prefix,
                        declared_length,
                    } => {
                        let response = format!(
                            "HTTP/1.1 200 OK\r\nContent-Type: application/json\r\nContent-Length: {declared_length}\r\nConnection: close\r\n\r\n",
                        );
                        stream
                            .write_all(response.as_bytes())
                            .await
                            .expect("write partial HTTP probe headers");
                        stream
                            .write_all(body_prefix)
                            .await
                            .expect("write partial HTTP probe body");
                    }
                }
            }
            if let Ok(Ok((mut unexpected, _))) =
                timeout(Duration::from_millis(100), listener.accept()).await
            {
                let mut byte = [0_u8; 1];
                if timeout(Duration::from_millis(100), unexpected.read(&mut byte))
                    .await
                    .is_ok_and(|result| result.is_ok_and(|read| read != 0))
                {
                    server_writes.fetch_add(1, Ordering::SeqCst);
                }
            }
            server_writes.load(Ordering::SeqCst)
        });
        (format!("http://{address}"), observed_rx, writes, server)
    }

    fn authenticated_config() -> PmConnectivityConfig {
        let base = crate::evidence::connectivity_config();
        let base_scope = base.account().account_scope();
        let eoa = EvmAddress::parse(ADDRESS).expect("test EOA");
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
        .expect("test account connectivity");
        PmConnectivityConfig::new(public, account).expect("test connectivity")
    }

    fn client_order(config: &PmConnectivityConfig, byte: u8) -> PmClientOrderKey {
        PmClientOrderKey::new(
            config.account().account_scope().handle(),
            PmClientOrderId::from_bytes([byte; 16]).expect("test client order"),
        )
    }

    fn cancel_intent(
        config: &PmConnectivityConfig,
        client_order: PmClientOrderKey,
        venue_order: PmVenueOrderKey,
    ) -> reap_pm_state::PmOwnedCancelIntent {
        let side = PmOrderSide::Buy;
        let price = PmPrice::parse_decimal("0.50").expect("test price");
        let quantity = PmQuantity::parse_decimal("5").expect("test quantity");
        let maker = exact_order_amounts(side, price, quantity)
            .expect("test exact amounts")
            .maker();
        let quote = PmOwnedQuoteIntent::new(
            PmOwnedIntentId::new(2).expect("test intent"),
            PmOwnedQuoteSlotKey::new(
                config.account().account_scope(),
                config.public().instrument(),
                side,
            ),
            client_order,
            price,
            quantity,
            PmExactReservation::policy_approved(maker, U256::ZERO).expect("test reservation"),
        )
        .expect("test owned quote");
        let mut lifecycle = PmOwnedOrderLifecycle::new(
            config.account().account_scope(),
            config.public().instrument(),
        );
        assert!(matches!(
            lifecycle.admit_quote(quote),
            Ok(PmOwnedQuoteAdmission::Admitted(key)) if key == client_order
        ));
        lifecycle
            .apply_submit_result(client_order, PmOwnedSubmitResult::Accepted(venue_order))
            .expect("accept test owned order");
        match lifecycle
            .request_cancel(client_order)
            .expect("request test cancel")
        {
            PmOwnedCancelRequestApply::Issued(intent) => intent,
            other => panic!("expected issued cancel intent, got {other:?}"),
        }
    }

    async fn wait_until_entered(latch: &PmAuthenticatedJournalWriteLatch) {
        timeout(Duration::from_secs(2), async {
            while !latch.entered() {
                tokio::task::yield_now().await;
            }
        })
        .await
        .expect("journal write did not reach test latch");
    }

    fn durable_line_count(path: &Path) -> usize {
        std::fs::read(path)
            .expect("read authenticated journal")
            .split(|byte| *byte == b'\n')
            .filter(|line| !line.is_empty())
            .count()
    }

    fn durable_record_count(path: &Path, record_name: &str) -> usize {
        std::fs::read_to_string(path)
            .expect("read authenticated journal text")
            .matches(record_name)
            .count()
    }

    struct AuthenticatedWorkerHarness {
        config: PmConnectivityConfig,
        preparation: PmFixedMutationPreparation,
        instrument_scope: PmFixtureInstrumentScope,
        scope: PmAuthenticatedJournalScopeV1,
        runtime: PmAuthenticatedJournalRuntime,
        place_worker: PmAuthenticatedPlaceWorker,
        cancel_worker: PmAuthenticatedCancelWorker,
        authenticated_http: PmAuthenticatedHttpOwner,
        authenticated_user_ws: PmAuthenticatedUserWsRole,
        credential_supervisor: PmCredentialAuthoritySupervisor,
    }

    struct TwoBarrierWriteExpectation<'a> {
        journal_path: &'a Path,
        base_lines: usize,
        prior_writes: usize,
        prepared_record_name: &'a str,
        expected_request_line: &'a str,
    }

    async fn start_worker_harness(
        origin: &str,
        journal_path: PathBuf,
        credential_slot: &str,
    ) -> AuthenticatedWorkerHarness {
        let config = authenticated_config();
        let metadata = config.public().expected_metadata();
        let wire_scope = PmWireScope::new(
            metadata.condition(),
            metadata.market(),
            metadata.outcome().token(),
        );
        let private_http = PmPrivateHttpConfig::loopback_evidence(
            origin,
            Duration::from_millis(100),
            Duration::from_secs(2),
            wire_scope,
        )
        .expect("test private HTTP config");
        let user_ws_endpoint = format!("{}/ws/user", origin.replacen("http://", "ws://", 1));
        let user_ws = PmUserWsConfig::loopback_evidence(
            &user_ws_endpoint,
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
        .expect("test user WebSocket config");
        let preparation = PmFixedMutationPreparation::new(
            config.account().account_scope(),
            config.public().instrument(),
        );
        let owner = PmLoopbackMutationConnectivityOwner::new(
            private_http,
            user_ws,
            config.account().account_scope(),
            config.public().instrument(),
            config.account().trading_domain(),
            preparation.place_profile(),
            preparation.cancel_purpose(),
            config.public().observation_grant(),
            CredentialSlotId::new(credential_slot.into()).expect("test credential slot"),
            FixedEoaSigner::bind(EoaPrivateKeyInput::new(TEST_KEY.into()), ADDRESS)
                .expect("test signer"),
            L2Credentials::bind(
                ADDRESS,
                L2CredentialInput::new(API_KEY.into(), API_SECRET.into(), PASSPHRASE.into()),
            )
            .expect("test credentials"),
        )
        .expect("test mutation connectivity");
        let roles = owner.split().expect("split test mutation connectivity");
        let (
            authenticated_http,
            authenticated_user_ws,
            place_authentication,
            cancel_authentication,
            _binding,
            credential_fingerprint,
            credential_supervisor,
        ) = roles.into_roles();
        let scope = PmAuthenticatedJournalScopeV1::from_config(
            &config,
            credential_fingerprint.into_authenticated_journal_scope_bytes(),
        )
        .expect("test authenticated journal scope");
        let (runtime, recovery) = PmAuthenticatedJournalRuntime::start(
            journal_path.clone(),
            scope.clone(),
            Duration::from_secs(2),
        )
        .await
        .expect("start test authenticated journal");
        assert_eq!(recovery.record_count(), 0);
        assert_eq!(durable_line_count(&journal_path), 1);
        let transport_config = PmLoopbackMutationConfig::loopback_evidence(
            origin,
            Duration::from_millis(100),
            Duration::from_secs(2),
        )
        .expect("test mutation transport config");
        let place_transport =
            PmFixedPlaceLoopbackRole::new(transport_config.clone()).expect("place transport");
        let cancel_transport =
            PmExactOwnedCancelLoopbackRole::new(transport_config).expect("cancel transport");
        let place_worker = PmAuthenticatedPlaceWorker::new(
            scope.clone(),
            runtime.role(),
            place_authentication,
            place_transport,
        );
        let cancel_worker = PmAuthenticatedCancelWorker::new(
            scope.clone(),
            runtime.role(),
            cancel_authentication,
            cancel_transport,
        );
        let instrument_scope =
            PmFixtureInstrumentScope::from_metadata(config.public().instrument(), metadata)
                .expect("test instrument scope");
        AuthenticatedWorkerHarness {
            config,
            preparation,
            instrument_scope,
            scope,
            runtime,
            place_worker,
            cancel_worker,
            authenticated_http,
            authenticated_user_ws,
            credential_supervisor,
        }
    }

    async fn prove_two_barriers_before_write(
        prepared: PmAuthenticatedJournalWriteLatch,
        dispatch: PmAuthenticatedJournalWriteLatch,
        expectation: TwoBarrierWriteExpectation<'_>,
        writes: &AtomicUsize,
        observed: &mut mpsc::UnboundedReceiver<ObservedHttpWrite>,
    ) -> oneshot::Sender<()> {
        let TwoBarrierWriteExpectation {
            journal_path,
            base_lines,
            prior_writes,
            prepared_record_name,
            expected_request_line,
        } = expectation;
        wait_until_entered(&prepared).await;
        assert_eq!(durable_line_count(journal_path), base_lines);
        assert_eq!(durable_record_count(journal_path, prepared_record_name), 0);
        assert_eq!(
            durable_record_count(journal_path, "dispatch_authorized"),
            prior_writes,
        );
        assert_eq!(writes.load(Ordering::SeqCst), prior_writes);
        prepared.release();

        wait_until_entered(&dispatch).await;
        assert_eq!(durable_line_count(journal_path), base_lines + 1);
        assert_eq!(durable_record_count(journal_path, prepared_record_name), 1);
        assert_eq!(
            durable_record_count(journal_path, "dispatch_authorized"),
            prior_writes,
        );
        assert_eq!(writes.load(Ordering::SeqCst), prior_writes);
        dispatch.release();

        let observed = timeout(Duration::from_secs(2), observed.recv())
            .await
            .expect("HTTP write did not follow durable grant")
            .expect("HTTP probe closed");
        assert_eq!(writes.load(Ordering::SeqCst), prior_writes + 1);
        assert_eq!(durable_line_count(journal_path), base_lines + 2);
        assert_eq!(durable_record_count(journal_path, prepared_record_name), 1);
        assert_eq!(
            durable_record_count(journal_path, "dispatch_authorized"),
            prior_writes + 1,
        );
        assert_eq!(
            std::str::from_utf8(&observed.request)
                .expect("UTF-8 HTTP request")
                .lines()
                .next(),
            Some(expected_request_line),
        );
        observed.release_response
    }

    #[tokio::test(flavor = "multi_thread", worker_threads = 2)]
    async fn place_and_cancel_http_writes_follow_both_durable_barriers_exactly_once() {
        let directory = tempfile::tempdir().expect("temporary directory");
        let journal_path = directory.path().join("barrier-ordering.jsonl");
        let (origin, mut observed, writes, server) = start_http_probe().await;
        let AuthenticatedWorkerHarness {
            config,
            preparation,
            instrument_scope,
            scope: _,
            runtime,
            mut place_worker,
            mut cancel_worker,
            authenticated_http,
            authenticated_user_ws,
            credential_supervisor,
        } = start_worker_harness(&origin, journal_path.clone(), "barrier-ordering-v1").await;
        let place_client = client_order(&config, 1);
        let place_request = preparation
            .prepare_place(
                instrument_scope,
                place_client,
                PmOrderSalt::from_u64(1).expect("test salt"),
                PmOrderSide::Buy,
                PmPrice::parse_decimal("0.50").expect("test price"),
                PmQuantity::parse_decimal("5").expect("test quantity"),
                AUTH_SECONDS * 1_000,
            )
            .expect("test place request");
        let place_dispatch = PmPreparedPlaceDispatch::new(
            NonZeroU64::new(1).expect("test Goal-F sequence"),
            place_request,
        );
        let mut times =
            PmLoopbackServerTimeScript::new(&[AUTH_SECONDS, AUTH_SECONDS + 1]).expect("test times");

        let place_prepared = runtime.latch_next_write_for_test().await;
        let place_dispatch_authorized = runtime.latch_next_write_for_test().await;
        let place_time = times
            .issue_authorized_mutation_server_time()
            .expect("place server time");
        let place_task = tokio::spawn(async move {
            let result = place_worker.run_task(place_dispatch, place_time).await;
            (place_worker, result)
        });
        let release_place_response = prove_two_barriers_before_write(
            place_prepared,
            place_dispatch_authorized,
            TwoBarrierWriteExpectation {
                journal_path: &journal_path,
                base_lines: 1,
                prior_writes: 0,
                prepared_record_name: "place_prepared",
                expected_request_line: "POST /order HTTP/1.1",
            },
            &writes,
            &mut observed,
        )
        .await;
        release_place_response
            .send(())
            .expect("release place response");
        let (place_worker, place_result) = place_task.await.expect("join place worker");
        place_result.expect("place worker completed after durable result");
        assert_eq!(writes.load(Ordering::SeqCst), 1);
        assert_eq!(durable_line_count(&journal_path), 4);
        drop(place_worker);

        let cancel_client = client_order(&config, 2);
        let venue_order = PmVenueOrderKey::new(
            config.account().account_scope().handle(),
            PmVenueOrderId::new(VENUE_ORDER).expect("test venue order"),
        );
        let intent = cancel_intent(&config, cancel_client, venue_order);
        let cancel_request = preparation
            .prepare_cancel(instrument_scope, cancel_client, venue_order)
            .expect("test cancel request");
        let cancel_dispatch = PmPreparedCancelDispatch::new(
            NonZeroU64::new(2).expect("test Goal-F sequence"),
            cancel_request,
        );
        let cancel_prepared = runtime.latch_next_write_for_test().await;
        let cancel_dispatch_authorized = runtime.latch_next_write_for_test().await;
        let cancel_time = times
            .issue_authorized_mutation_server_time()
            .expect("cancel server time");
        let cancel_task = tokio::spawn(async move {
            let result = cancel_worker
                .run_task(cancel_dispatch, intent, cancel_time)
                .await;
            (cancel_worker, result)
        });
        let release_cancel_response = prove_two_barriers_before_write(
            cancel_prepared,
            cancel_dispatch_authorized,
            TwoBarrierWriteExpectation {
                journal_path: &journal_path,
                base_lines: 4,
                prior_writes: 1,
                prepared_record_name: "cancel_prepared",
                expected_request_line: "DELETE /order HTTP/1.1",
            },
            &writes,
            &mut observed,
        )
        .await;
        release_cancel_response
            .send(())
            .expect("release cancel response");
        let (cancel_worker, cancel_result) = cancel_task.await.expect("join cancel worker");
        cancel_result.expect("cancel worker completed after durable result");
        assert_eq!(writes.load(Ordering::SeqCst), 2);
        assert_eq!(durable_line_count(&journal_path), 7);
        drop(cancel_worker);

        drop(authenticated_http);
        drop(authenticated_user_ws);
        credential_supervisor
            .shutdown()
            .await
            .expect("shutdown credential authority");
        runtime.shutdown().await.expect("shutdown journal runtime");
        assert_eq!(server.await.expect("join HTTP probe"), 2);
    }

    #[tokio::test(flavor = "multi_thread", worker_threads = 2)]
    async fn authentication_failure_is_primary_retains_exact_neutral_place_and_writes_nothing() {
        let listener = TcpListener::bind("127.0.0.1:0")
            .await
            .expect("bind zero-write HTTP probe");
        let origin = format!(
            "http://{}",
            listener.local_addr().expect("zero-write HTTP address")
        );
        let directory = tempfile::tempdir().expect("temporary authentication-failure directory");
        let journal_path = directory.path().join("authentication-failure.jsonl");
        let AuthenticatedWorkerHarness {
            config,
            preparation,
            instrument_scope,
            scope: _,
            runtime,
            mut place_worker,
            cancel_worker,
            authenticated_http,
            authenticated_user_ws,
            credential_supervisor,
        } = start_worker_harness(&origin, journal_path.clone(), "authentication-failure-v1").await;
        credential_supervisor
            .shutdown()
            .await
            .expect("close credential authority before authentication");

        let client = client_order(&config, 9);
        let request = preparation
            .prepare_place(
                instrument_scope,
                client,
                PmOrderSalt::from_u64(9).expect("test salt"),
                PmOrderSide::Buy,
                PmPrice::parse_decimal("0.50").expect("test price"),
                PmQuantity::parse_decimal("5").expect("test quantity"),
                AUTH_SECONDS * 1_000,
            )
            .expect("test place request");
        let expected_retained = preparation
            .prepare_place(
                instrument_scope,
                client,
                PmOrderSalt::from_u64(9).expect("test salt"),
                PmOrderSide::Buy,
                PmPrice::parse_decimal("0.50").expect("test price"),
                PmQuantity::parse_decimal("5").expect("test quantity"),
                AUTH_SECONDS * 1_000,
            )
            .expect("exact comparison request");
        let dispatch = PmPreparedPlaceDispatch::new(
            NonZeroU64::new(9).expect("test Goal-F sequence"),
            request,
        );
        let mut times = PmLoopbackServerTimeScript::new(&[AUTH_SECONDS]).expect("test server time");
        let error = place_worker
            .run_task(
                dispatch,
                times
                    .issue_authorized_mutation_server_time()
                    .expect("authorized test time"),
            )
            .await
            .expect_err("closed authority must fail before transport");
        assert!(matches!(
            error,
            PmAuthenticatedExecutionError::Authentication(
                PmLoopbackMutationAuthError::AuthorityClosed
            )
        ));
        let quarantine = place_worker
            .quarantined_pre_send()
            .expect("authentication failure retains pre-send identity");
        assert_eq!(quarantine.prior_goal_f_sequence(), 9);
        assert_eq!(quarantine.client_order(), client);
        assert_eq!(
            place_worker.quarantined_place_request_for_test(),
            Some(&expected_retained),
            "the move-only neutral request must be retained byte-for-byte in memory"
        );
        assert!(!place_worker.is_available());
        assert_eq!(durable_line_count(&journal_path), 1);
        assert_eq!(durable_record_count(&journal_path, "place_prepared"), 0);
        assert!(
            timeout(Duration::from_millis(100), listener.accept())
                .await
                .is_err(),
            "authentication failure reached the network transport"
        );

        drop(place_worker);
        drop(cancel_worker);
        drop(authenticated_http);
        drop(authenticated_user_ws);
        runtime.shutdown().await.expect("shutdown journal runtime");
    }

    #[tokio::test(flavor = "multi_thread", worker_threads = 2)]
    async fn post_send_faults_are_durable_acknowledgement_unknown_and_never_resent() {
        let directory = tempfile::tempdir().expect("temporary directory");
        let journal_path = directory.path().join("post-send-faults.jsonl");
        let (origin, mut observed, writes, server) = start_http_probe_with([
            HttpProbeResponse::Disconnect,
            HttpProbeResponse::Partial {
                body_prefix: b"{\"canceled\":[",
                declared_length: 100,
            },
        ])
        .await;
        let AuthenticatedWorkerHarness {
            config,
            preparation,
            instrument_scope,
            scope,
            runtime,
            mut place_worker,
            mut cancel_worker,
            authenticated_http,
            authenticated_user_ws,
            credential_supervisor,
        } = start_worker_harness(&origin, journal_path.clone(), "post-send-faults-v1").await;
        let mut times =
            PmLoopbackServerTimeScript::new(&[AUTH_SECONDS, AUTH_SECONDS + 1]).expect("test times");

        let place_client = client_order(&config, 3);
        let place_request = preparation
            .prepare_place(
                instrument_scope,
                place_client,
                PmOrderSalt::from_u64(3).expect("test salt"),
                PmOrderSide::Buy,
                PmPrice::parse_decimal("0.50").expect("test price"),
                PmQuantity::parse_decimal("5").expect("test quantity"),
                AUTH_SECONDS * 1_000,
            )
            .expect("test place request");
        let place_dispatch = PmPreparedPlaceDispatch::new(
            NonZeroU64::new(3).expect("test Goal-F sequence"),
            place_request,
        );
        let place_time = times
            .issue_authorized_mutation_server_time()
            .expect("place server time");
        let place_task = tokio::spawn(async move {
            let result = place_worker.run_task(place_dispatch, place_time).await;
            (place_worker, result)
        });
        let place_write = timeout(Duration::from_secs(2), observed.recv())
            .await
            .expect("place write did not reach disconnect probe")
            .expect("HTTP probe closed before place write");
        assert_eq!(
            std::str::from_utf8(&place_write.request)
                .expect("UTF-8 place request")
                .lines()
                .next(),
            Some("POST /order HTTP/1.1"),
        );
        assert_eq!(writes.load(Ordering::SeqCst), 1);
        assert_eq!(durable_line_count(&journal_path), 3);
        place_write
            .release_response
            .send(())
            .expect("release disconnect response");
        let (place_worker, place_result) = place_task.await.expect("join place worker");
        {
            let place_completion = place_result.expect("disconnect must become a durable result");
            assert_eq!(
                place_completion.result().classification(),
                PmJournalAuthenticatedClassificationV1::AcknowledgementUnknown,
            );
        }
        assert!(
            !place_worker.is_available(),
            "ambiguous place worker became available before reconciliation bridge"
        );
        assert_eq!(durable_line_count(&journal_path), 4);
        assert_eq!(
            durable_record_count(&journal_path, "acknowledgement_unknown"),
            1,
        );
        tokio::time::sleep(Duration::from_millis(50)).await;
        assert_eq!(
            writes.load(Ordering::SeqCst),
            1,
            "place disconnect caused a blind retry"
        );
        drop(place_worker);

        let cancel_client = client_order(&config, 4);
        let venue_order = PmVenueOrderKey::new(
            config.account().account_scope().handle(),
            PmVenueOrderId::new(VENUE_ORDER).expect("test venue order"),
        );
        let intent = cancel_intent(&config, cancel_client, venue_order);
        let cancel_request = preparation
            .prepare_cancel(instrument_scope, cancel_client, venue_order)
            .expect("test cancel request");
        let cancel_dispatch = PmPreparedCancelDispatch::new(
            NonZeroU64::new(4).expect("test Goal-F sequence"),
            cancel_request,
        );
        let cancel_time = times
            .issue_authorized_mutation_server_time()
            .expect("cancel server time");
        let cancel_task = tokio::spawn(async move {
            let result = cancel_worker
                .run_task(cancel_dispatch, intent, cancel_time)
                .await;
            (cancel_worker, result)
        });
        let cancel_write = timeout(Duration::from_secs(2), observed.recv())
            .await
            .expect("cancel write did not reach partial-response probe")
            .expect("HTTP probe closed before cancel write");
        assert_eq!(
            std::str::from_utf8(&cancel_write.request)
                .expect("UTF-8 cancel request")
                .lines()
                .next(),
            Some("DELETE /order HTTP/1.1"),
        );
        assert_eq!(writes.load(Ordering::SeqCst), 2);
        assert_eq!(durable_line_count(&journal_path), 6);
        cancel_write
            .release_response
            .send(())
            .expect("release partial cancel response");
        let (cancel_worker, cancel_result) = cancel_task.await.expect("join cancel worker");
        {
            let cancel_completion =
                cancel_result.expect("partial response must become a durable result");
            assert_eq!(
                cancel_completion.result().classification(),
                PmJournalAuthenticatedClassificationV1::AcknowledgementUnknown,
            );
        }
        assert!(
            !cancel_worker.is_available(),
            "ambiguous cancel worker became available before reconciliation bridge"
        );
        assert_eq!(durable_line_count(&journal_path), 7);
        assert_eq!(
            durable_record_count(&journal_path, "acknowledgement_unknown"),
            2,
        );
        tokio::time::sleep(Duration::from_millis(50)).await;
        assert_eq!(
            writes.load(Ordering::SeqCst),
            2,
            "cancel partial response caused a blind retry"
        );
        drop(cancel_worker);

        drop(authenticated_http);
        drop(authenticated_user_ws);
        credential_supervisor
            .shutdown()
            .await
            .expect("shutdown credential authority");
        runtime.shutdown().await.expect("shutdown journal runtime");
        assert_eq!(server.await.expect("join HTTP probe"), 2);

        let (restart, recovery) =
            PmAuthenticatedJournalRuntime::start(journal_path, scope, Duration::from_secs(2))
                .await
                .expect("restart authenticated journal");
        assert_eq!(recovery.record_count(), 7);
        assert_eq!(recovery.acknowledgement_unknown_count(), 2);
        assert!(recovery.requires_reconciliation());
        assert!(recovery.prepared_without_grant().is_empty());
        assert_eq!(
            recovery
                .classified_results()
                .iter()
                .map(|result| result.classification())
                .collect::<Vec<_>>(),
            vec![
                PmAuthenticatedRecoveredResultClassificationV1::Place(
                    PmAuthenticatedPlaceResultKindV1::AcknowledgementUnknown,
                ),
                PmAuthenticatedRecoveredResultClassificationV1::Cancel(
                    PmAuthenticatedCancelResultKindV1::AcknowledgementUnknown,
                ),
            ],
        );
        let unresolved = recovery.unresolved_operations();
        assert_eq!(unresolved.len(), 2);
        assert_eq!(
            unresolved
                .iter()
                .map(|operation| operation.kind())
                .collect::<Vec<_>>(),
            vec![
                PmAuthenticatedUnresolvedOperationKindV1::Place,
                PmAuthenticatedUnresolvedOperationKindV1::Cancel,
            ],
        );
        for operation in unresolved {
            assert_eq!(
                operation.reason(),
                PmAuthenticatedUnresolvedReasonV1::AcknowledgementUnknown,
            );
            assert!(operation.may_have_been_sent());
            assert!(operation.requires_reconciliation());
            assert!(!operation.allows_automatic_resend());
            assert!(operation.result_journal_sequence().is_some());
        }
        tokio::time::sleep(Duration::from_millis(50)).await;
        assert_eq!(
            writes.load(Ordering::SeqCst),
            2,
            "restart recovery reissued a may-have-sent operation"
        );
        restart
            .shutdown()
            .await
            .expect("shutdown restarted journal runtime");
    }
}
