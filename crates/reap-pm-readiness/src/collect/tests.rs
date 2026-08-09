// End-to-end transport coverage is intentionally compiled only for unit tests.

#[cfg(test)]
mod local_e2e {
    use std::{
        collections::BTreeMap,
        fs,
        os::unix::fs::{MetadataExt as _, PermissionsExt as _},
        sync::{
            Arc, Mutex,
            atomic::{AtomicBool, AtomicUsize, Ordering},
        },
        time::{Duration, Instant},
    };

    use base64::{
        Engine as _,
        engine::general_purpose::{STANDARD as BASE64_STANDARD, URL_SAFE_NO_PAD},
    };
    use futures_util::{SinkExt as _, StreamExt as _};
    use reap_pm_core::ConnectionEpoch;
    use reap_polymarket_live_adapter::{
        PmPrivateHttpConfig, PmPublicHttpConfig, PmPublicMetadataHttpRole,
        PmReadOnlyCredentialInput, PmReadOnlyPrivateConnectivityOwner, PmReadServerTimeHttpRole,
        PmUserWsConfig, pm_user_ws_shutdown_channel,
    };
    use reap_polymarket_wire::MAX_PM_LIVE_BODY_BYTES;
    use sha2::{Digest as _, Sha256};
    use tempfile::TempDir;
    use tokio::{
        io::{AsyncReadExt as _, AsyncWriteExt as _},
        net::{TcpListener, TcpStream},
        sync::Notify,
        task::JoinHandle,
        time::timeout,
    };
    use tokio_tungstenite::{accept_async, tungstenite::Message};

    use super::super::{
        PM_CREDENTIAL_AUTHORITY_READ_ONLY_SHUTDOWN_BOUNDS, PmReadOnlySmokeError, PrivateCollection,
        Provenance, USER_STREAM_GRACEFUL_JOIN_MS, UserEvidenceSink, UserTaskCancellationFailStop,
        collect_authenticated_http, collect_authenticated_with_owner,
        collect_pm_read_only_smoke_path, collect_public_metadata_with_role, finish_private_attempt,
        join_user_task_bounded, reserve_private_output, unix_ms,
    };
    use crate::{
        PmReadOnlySmokeConfig, PmReadOnlyTeardownEvidence,
        credentials::PmReadOnlyArtifactSecretGuard, load_pm_read_only_smoke_config_path,
        require_pm_read_only_smoke_pass, verify_pm_read_only_smoke_path,
    };

    const ADDRESS: &str = "0xf39fd6e51aad88f6f4ce6ab8827279cfffb92266";
    const FOREIGN_ADDRESS: &str = "0x2222222222222222222222222222222222222222";
    const API_KEY: &str = "00000000-0000-4000-8000-000000000001";
    const API_SECRET: &str = "AAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAA=";
    const PASSPHRASE: &str = "readiness-e2e-passphrase-canary";
    const CONDITION: &str = "0x1111111111111111111111111111111111111111111111111111111111111111";
    const FOREIGN_CONDITION: &str =
        "0x3333333333333333333333333333333333333333333333333333333333333333";
    const MARKET: &str = "0x2222222222222222222222222222222222222222222222222222222222222222";
    const EXACT_ORDER: &str = "0xaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa";
    const FOREIGN_ORDER: &str =
        "0xbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbb";
    const SPENDER: &str = "0xe111180000d2663c0091e4f400237545b87b996b";
    const REQUEST_COUNT: usize = 12;

    #[derive(Clone, Copy, PartialEq, Eq)]
    enum HttpFixtureMode {
        Passing,
        RejectTrade,
    }

    #[derive(Clone)]
    struct HttpFixture {
        origin: String,
        requests: Arc<Mutex<Vec<String>>>,
        task: Arc<Mutex<Option<JoinHandle<()>>>>,
    }

    impl HttpFixture {
        async fn join(&self) {
            let task = self.task.lock().unwrap().take().expect("one HTTP join");
            timeout(Duration::from_secs(3), task)
                .await
                .expect("HTTP fixture did not finish")
                .expect("HTTP fixture panicked");
        }

        fn captured(&self) -> Vec<String> {
            self.requests.lock().unwrap().clone()
        }
    }

    struct UserWsFixture {
        endpoint: String,
        subscription: Arc<Mutex<Option<String>>>,
        ping_count: Arc<AtomicUsize>,
        shutdown_observed: Arc<AtomicBool>,
        task: JoinHandle<()>,
    }

    impl UserWsFixture {
        async fn join(self) -> (String, usize, bool) {
            timeout(Duration::from_secs(3), self.task)
                .await
                .expect("user-WS fixture did not finish")
                .expect("user-WS fixture panicked");
            let subscription = self
                .subscription
                .lock()
                .unwrap()
                .clone()
                .expect("user subscription was captured");
            (
                subscription,
                self.ping_count.load(Ordering::SeqCst),
                self.shutdown_observed.load(Ordering::SeqCst),
            )
        }
    }

    fn config() -> PmReadOnlySmokeConfig {
        PmReadOnlySmokeConfig {
            schema_version: 1,
            credential_slot_id: "readiness-loopback-slot-v1".into(),
            signer_address: ADDRESS.into(),
            funder_address: ADDRESS.into(),
            chain_id: 137,
            signature_type: 0,
            condition_id: CONDITION.into(),
            market_id: MARKET.into(),
            token_id: "123".into(),
            outcome: "Yes".into(),
            tick: "0.01".into(),
            minimum_order_size: "5".into(),
            negative_risk: false,
            connect_timeout_ms: 2_000,
            request_timeout_ms: 3_000,
            user_stream_dwell_ms: 12_000,
            user_stream_idle_timeout_ms: 30_000,
            user_stream_pong_timeout_ms: 5_000,
            user_stream_max_reconnect_attempts: 0,
            user_stream_reconnect_backoff_ms: 10,
            user_stream_event_channel_capacity: 64,
            api_key_file: "api-key".into(),
            secret_file: "secret".into(),
            passphrase_file: "passphrase".into(),
        }
    }

    fn exact_subscription() -> String {
        format!(
            r#"{{"auth":{{"apiKey":"{API_KEY}","secret":"{API_SECRET}","passphrase":"{PASSPHRASE}"}},"markets":["{CONDITION}"],"type":"user"}}"#
        )
    }

    fn public_market_body() -> String {
        format!(
            r#"{{"condition_id":"{CONDITION}","question_id":"{MARKET}","active":true,"closed":false,"archived":false,"accepting_orders":true,"enable_order_book":true,"accepting_order_timestamp":"2026-08-08T00:00:00Z","end_date_iso":"2027-01-01T00:00:00Z","game_start_time":null,"seconds_delay":0,"minimum_order_size":5,"minimum_tick_size":0.01,"tokens":[]}}"#
        )
    }

    fn public_clob_body() -> String {
        format!(
            r#"{{"c":"{CONDITION}","t":[{{"t":"123","o":"Yes"}},{{"t":"456","o":"No"}}],"mts":0.01,"mos":5,"nr":false,"fd":{{"r":0.02,"e":2,"to":true}},"mbf":0,"tbf":0,"ao":true,"sd":0,"gst":null,"cbos":true,"aot":"2026-08-08T00:00:00Z","rfqe":false,"itode":false,"ibce":true,"oas":0}}"#
        )
    }

    fn order(id: &str, condition: &str, token: &str, maker: &str) -> String {
        format!(
            r#"{{"id":"{id}","market":"{condition}","asset_id":"{token}","side":"BUY","original_size":"10.000000","size_matched":"0","price":"0.420000","status":"LIVE","maker_address":"{maker}","owner":"{API_KEY}","expiration":"0","created_at":1700000000}}"#
        )
    }

    fn trade() -> String {
        format!(
            r#"{{"id":"trade-1","market":"{CONDITION}","asset_id":"123","side":"SELL","size":"2.500000","price":"0.420000","status":"CONFIRMED","match_time":"1700000000002","last_update":"1700000000003","order_id":"{EXACT_ORDER}","maker_orders":[],"maker_address":"{ADDRESS}","owner":"{API_KEY}"}}"#
        )
    }

    fn user_order_event() -> String {
        format!(
            r#"{{"event_type":"order","id":"{EXACT_ORDER}","owner":"{API_KEY}","market":"{CONDITION}","asset_id":"123","side":"BUY","original_size":"10.000000","size_matched":"0","price":"0.420000","type":"PLACEMENT","order_owner":"{API_KEY}","timestamp":"1700000000000","outcome":"Yes","created_at":"1700000000000","expiration":"0","order_type":"GTC","status":"LIVE","maker_address":"{ADDRESS}"}}"#
        )
    }

    fn response(mode: HttpFixtureMode, target: &str) -> (u16, Vec<u8>) {
        let exact_first = "/data/orders?next_cursor=MA%3D%3D".to_owned();
        let exact_second = "/data/orders?next_cursor=cursor%2B%2F%3D".to_owned();
        let body = match target {
            "/data/trades?next_cursor=MA%3D%3D" if mode == HttpFixtureMode::RejectTrade => {
                return (503, br#"{"error":"test rejection"}"#.to_vec());
            }
            value if value == format!("/markets/{CONDITION}") => public_market_body().into_bytes(),
            value if value == format!("/clob-markets/{CONDITION}") => {
                public_clob_body().into_bytes()
            }
            "/time" => b"1800000000".to_vec(),
            "/balance-allowance?asset_type=COLLATERAL&signature_type=0"
            | "/balance-allowance?asset_type=CONDITIONAL&token_id=123&signature_type=0" => {
                format!(r#"{{"balance":"1000","allowances":{{"{SPENDER}":"900"}}}}"#).into_bytes()
            }
            value if value == exact_first => format!(
                r#"{{"data":[{}],"next_cursor":"cursor+/=","limit":128,"count":1}}"#,
                order(EXACT_ORDER, CONDITION, "123", ADDRESS)
            )
            .into_bytes(),
            value if value == exact_second => format!(
                r#"{{"data":[{}],"next_cursor":"LTE=","limit":128,"count":1}}"#,
                order(FOREIGN_ORDER, FOREIGN_CONDITION, "999", FOREIGN_ADDRESS)
            )
            .into_bytes(),
            "/data/trades?next_cursor=MA%3D%3D" => format!(
                r#"{{"data":[{}],"next_cursor":"LTE=","limit":128,"count":1}}"#,
                trade()
            )
            .into_bytes(),
            unexpected => panic!("unexpected readiness fixture route: {unexpected}"),
        };
        (200, body)
    }

    async fn read_http_request(stream: &mut TcpStream) -> String {
        let mut raw = Vec::new();
        let mut chunk = [0_u8; 2_048];
        loop {
            let read = timeout(Duration::from_secs(2), stream.read(&mut chunk))
                .await
                .expect("HTTP request read timed out")
                .expect("HTTP request read failed");
            assert!(read > 0, "HTTP request ended before its headers");
            raw.extend_from_slice(&chunk[..read]);
            assert!(
                raw.len() <= 64 * 1_024,
                "HTTP request headers exceeded test bound"
            );
            if raw.windows(4).any(|window| window == b"\r\n\r\n") {
                break;
            }
        }
        String::from_utf8(raw).expect("HTTP fixture only accepts UTF-8 request headers")
    }

    async fn spawn_http_fixture(
        mode: HttpFixtureMode,
        trade_release: Option<Arc<Notify>>,
    ) -> HttpFixture {
        let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
        let origin = format!("http://{}", listener.local_addr().unwrap());
        let requests = Arc::new(Mutex::new(Vec::new()));
        let captured = Arc::clone(&requests);
        let task = tokio::spawn(async move {
            for _ in 0..REQUEST_COUNT {
                let (mut stream, _) = timeout(Duration::from_secs(20), listener.accept())
                    .await
                    .expect("HTTP fixture accept timed out")
                    .expect("HTTP fixture accept failed");
                let raw = read_http_request(&mut stream).await;
                let request_line = raw.lines().next().expect("request line");
                let mut parts = request_line.split_ascii_whitespace();
                assert_eq!(parts.next(), Some("GET"));
                let target = parts.next().expect("request target");
                assert_eq!(parts.next(), Some("HTTP/1.1"));
                assert_eq!(parts.next(), None);
                if mode == HttpFixtureMode::RejectTrade
                    && target == "/data/trades?next_cursor=MA%3D%3D"
                {
                    timeout(
                        Duration::from_secs(3),
                        trade_release
                            .as_ref()
                            .expect("rejected trade must await user-WS evidence")
                            .notified(),
                    )
                    .await
                    .expect("user-WS did not reach a correlated PONG before trade rejection");
                }
                let (status, body) = response(mode, target);
                captured.lock().unwrap().push(raw);
                let reason = if status == 200 {
                    "OK"
                } else {
                    "Service Unavailable"
                };
                let response = format!(
                    "HTTP/1.1 {status} {reason}\r\nContent-Type: application/json\r\nContent-Length: {}\r\nConnection: close\r\n\r\n",
                    body.len()
                );
                stream.write_all(response.as_bytes()).await.unwrap();
                stream.write_all(&body).await.unwrap();
            }
        });
        HttpFixture {
            origin,
            requests,
            task: Arc::new(Mutex::new(Some(task))),
        }
    }

    async fn spawn_user_ws_fixture(pong_observed: Option<Arc<Notify>>) -> UserWsFixture {
        let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
        let endpoint = format!("ws://{}/ws/user", listener.local_addr().unwrap());
        let subscription = Arc::new(Mutex::new(None));
        let captured_subscription = Arc::clone(&subscription);
        let ping_count = Arc::new(AtomicUsize::new(0));
        let observed_pings = Arc::clone(&ping_count);
        let shutdown_observed = Arc::new(AtomicBool::new(false));
        let observed_shutdown = Arc::clone(&shutdown_observed);
        let task = tokio::spawn(async move {
            let mut pong_observed = pong_observed;
            let (stream, _) = timeout(Duration::from_secs(3), listener.accept())
                .await
                .expect("user-WS accept timed out")
                .expect("user-WS accept failed");
            let mut socket = accept_async(stream)
                .await
                .expect("user-WS handshake failed");
            let first = timeout(Duration::from_secs(3), socket.next())
                .await
                .expect("user subscription timed out")
                .expect("user-WS closed before subscription")
                .expect("user subscription read failed");
            let raw_subscription = first
                .into_text()
                .expect("user subscription must be text")
                .to_string();
            assert_eq!(raw_subscription, exact_subscription());
            assert!(!raw_subscription.contains("initial_dump"));
            assert!(!raw_subscription.contains("operation"));
            *captured_subscription.lock().unwrap() = Some(raw_subscription);
            socket
                .send(Message::text(user_order_event()))
                .await
                .expect("user event send failed");

            loop {
                let next = timeout(Duration::from_secs(15), socket.next())
                    .await
                    .expect("user-WS went silent before collector shutdown");
                match next {
                    Some(Ok(Message::Text(text))) if text.as_str() == "PING" => {
                        observed_pings.fetch_add(1, Ordering::SeqCst);
                        socket
                            .send(Message::text("PONG"))
                            .await
                            .expect("user PONG send failed");
                        if let Some(observed) = pong_observed.take() {
                            observed.notify_one();
                        }
                    }
                    Some(Ok(Message::Close(_))) | None => {
                        observed_shutdown.store(true, Ordering::SeqCst);
                        break;
                    }
                    Some(Ok(Message::Ping(payload))) => {
                        socket.send(Message::Pong(payload)).await.unwrap();
                    }
                    Some(Ok(Message::Pong(_))) => {}
                    Some(Ok(other)) => panic!("unexpected user-WS client message: {other:?}"),
                    Some(Err(error)) => panic!("user-WS read failed: {error}"),
                }
            }
        });
        UserWsFixture {
            endpoint,
            subscription,
            ping_count,
            shutdown_observed,
            task,
        }
    }

    fn write_config(
        directory: &TempDir,
    ) -> (PmReadOnlySmokeConfig, crate::PmReadOnlyConfigEvidence) {
        let path = directory.path().join("pm-readiness.toml");
        let text = toml::to_string(&config()).expect("config serialization");
        fs::write(&path, text).expect("config write");
        load_pm_read_only_smoke_config_path(path).expect("strict config load")
    }

    fn request_target(request: &str) -> &str {
        request
            .lines()
            .next()
            .expect("request line")
            .split_ascii_whitespace()
            .nth(1)
            .expect("request target")
    }

    fn header<'a>(request: &'a str, expected: &str) -> Option<&'a str> {
        request.lines().skip(1).find_map(|line| {
            let (name, value) = line.split_once(':')?;
            name.eq_ignore_ascii_case(expected).then(|| value.trim())
        })
    }

    fn route_multiset(requests: &[String]) -> BTreeMap<String, usize> {
        let mut routes = BTreeMap::new();
        for request in requests {
            *routes
                .entry(request_target(request).to_owned())
                .or_default() += 1;
        }
        routes
    }

    fn expected_route_multiset() -> BTreeMap<String, usize> {
        BTreeMap::from([
            (format!("/markets/{CONDITION}"), 1),
            (format!("/clob-markets/{CONDITION}"), 1),
            ("/time".into(), 5),
            (
                "/balance-allowance?asset_type=COLLATERAL&signature_type=0".into(),
                1,
            ),
            (
                "/balance-allowance?asset_type=CONDITIONAL&token_id=123&signature_type=0".into(),
                1,
            ),
            ("/data/orders?next_cursor=MA%3D%3D".into(), 1),
            ("/data/orders?next_cursor=cursor%2B%2F%3D".into(), 1),
            ("/data/trades?next_cursor=MA%3D%3D".into(), 1),
        ])
    }

    fn assert_get_only_and_auth_partition(requests: &[String]) -> Vec<String> {
        assert_eq!(requests.len(), REQUEST_COUNT);
        assert_eq!(route_multiset(requests), expected_route_multiset());
        let mut signatures = Vec::new();
        for request in requests {
            assert_eq!(
                request.lines().next().unwrap().split(' ').next(),
                Some("GET")
            );
            let target = request_target(request);
            let authenticated = target.starts_with("/balance-allowance?")
                || target.starts_with("/data/orders?")
                || target.starts_with("/data/trades?");
            for name in [
                "poly_address",
                "poly_signature",
                "poly_timestamp",
                "poly_api_key",
                "poly_passphrase",
            ] {
                assert_eq!(
                    header(request, name).is_some(),
                    authenticated,
                    "authentication header partition mismatch for {target}: {name}"
                );
            }
            if authenticated {
                assert_eq!(header(request, "poly_api_key"), Some(API_KEY));
                assert_eq!(header(request, "poly_passphrase"), Some(PASSPHRASE));
                signatures.push(
                    header(request, "poly_signature")
                        .expect("private signature header")
                        .to_owned(),
                );
            } else {
                assert!(!request.to_ascii_lowercase().contains("\r\npoly_"));
            }
        }
        signatures
    }

    fn hex(bytes: &[u8]) -> String {
        use std::fmt::Write as _;

        let mut encoded = String::with_capacity(bytes.len() * 2);
        for byte in bytes {
            write!(&mut encoded, "{byte:02x}").unwrap();
        }
        encoded
    }

    fn sha256(value: &str) -> String {
        hex(&Sha256::digest(value.as_bytes()))
    }

    fn assert_no_secret_derived_material(artifact: &str, canaries: &[String]) {
        let mut forbidden = Vec::new();
        for canary in canaries {
            forbidden.push(canary.clone());
            forbidden.push(BASE64_STANDARD.encode(canary.as_bytes()));
            forbidden.push(URL_SAFE_NO_PAD.encode(canary.as_bytes()));
            forbidden.push(hex(canary.as_bytes()));
            forbidden.push(sha256(canary));
            forbidden.push(serde_json::to_string(canary).unwrap());
        }
        forbidden.sort();
        forbidden.dedup();
        for value in forbidden {
            assert!(
                !artifact.contains(&value),
                "artifact retained secret-derived canary of {} bytes",
                value.len()
            );
        }
    }

    #[test]
    fn uncommitted_output_reservation_is_removed_and_can_be_retried() {
        let directory = TempDir::new().expect("temporary readiness directory");
        fs::set_permissions(directory.path(), fs::Permissions::from_mode(0o700)).unwrap();
        let output_path = directory.path().join("uncommitted.json");

        let reservation = reserve_private_output(&output_path).expect("private reservation");
        assert!(output_path.exists());
        drop(reservation);
        assert!(!output_path.exists());

        let retry = reserve_private_output(&output_path).expect("retry reservation");
        drop(retry);
        assert!(!output_path.exists());
    }

    #[tokio::test]
    async fn partial_credential_load_never_persists_config_that_aliases_a_loaded_secret() {
        let directory = TempDir::new().expect("temporary readiness directory");
        fs::set_permissions(directory.path(), fs::Permissions::from_mode(0o700)).unwrap();

        let mut config = config();
        config.credential_slot_id = API_KEY.into();
        let config_path = directory.path().join("pm-readiness.toml");
        fs::write(&config_path, toml::to_string(&config).unwrap()).unwrap();
        for (entry, value) in [
            ("api-key", API_KEY.as_bytes()),
            ("secret", b"not-canonical-base64url".as_slice()),
            ("passphrase", PASSPHRASE.as_bytes()),
        ] {
            let path = directory.path().join(entry);
            fs::write(&path, value).unwrap();
            fs::set_permissions(path, fs::Permissions::from_mode(0o600)).unwrap();
        }

        let output_path = directory.path().join("must-not-exist.json");
        let error = collect_pm_read_only_smoke_path(&config_path, directory.path(), &output_path)
            .await
            .unwrap_err();
        assert!(matches!(error, PmReadOnlySmokeError::Credential(_)));
        assert!(!output_path.exists());
    }

    #[tokio::test]
    async fn local_read_only_certification_persists_a_secret_free_offline_verified_pass() {
        let directory = TempDir::new().expect("temporary readiness directory");
        fs::set_permissions(directory.path(), fs::Permissions::from_mode(0o700)).unwrap();
        let (config, config_evidence) = write_config(&directory);
        let scope = config.wire_scope().unwrap();
        let http = spawn_http_fixture(HttpFixtureMode::Passing, None).await;
        let user_ws = spawn_user_ws_fixture(None).await;

        let public = PmPublicHttpConfig::read_only_evidence(
            &http.origin,
            Duration::from_millis(config.connect_timeout_ms),
            Duration::from_millis(config.request_timeout_ms),
        )
        .unwrap();
        let metadata_role = PmPublicMetadataHttpRole::new(public.clone(), scope).unwrap();
        let time_role = PmReadServerTimeHttpRole::new(public).unwrap();
        let private_http = PmPrivateHttpConfig::read_only_evidence(
            &http.origin,
            Duration::from_millis(config.connect_timeout_ms),
            Duration::from_millis(config.request_timeout_ms),
            scope,
        )
        .unwrap();
        let user_config = PmUserWsConfig::read_only_evidence(
            &user_ws.endpoint,
            scope.condition(),
            Duration::from_secs(2),
            Duration::from_secs(2),
            Duration::from_millis(200),
            Duration::from_millis(100),
            MAX_PM_LIVE_BODY_BYTES,
            0,
            Duration::from_millis(10),
            config.user_stream_event_channel_capacity,
            ConnectionEpoch::new(1),
        )
        .unwrap();
        let owner = PmReadOnlyPrivateConnectivityOwner::read_only_evidence(
            config.signer().unwrap(),
            config.funder().unwrap(),
            scope,
            private_http,
            user_config,
            PmReadOnlyCredentialInput::new(API_KEY.into(), API_SECRET.into(), PASSPHRASE.into()),
        )
        .unwrap();
        assert!(!owner.production_order_entry_authorized());

        let output_path = directory.path().join("readiness-artifact.json");
        let mut output = reserve_private_output(&output_path).expect("private output reservation");
        let started_unix_ms = unix_ms().unwrap();
        let provenance = Provenance::collect(std::time::Instant::now()).unwrap();
        let metadata = collect_public_metadata_with_role(&config, metadata_role)
            .await
            .expect("public metadata collection");
        let private = collect_authenticated_with_owner(&config, time_role, owner)
            .await
            .expect("authenticated collection");
        assert!(
            private.failure.is_none(),
            "loopback collection must be clean"
        );

        http.join().await;
        let (raw_subscription, ping_count, shutdown_observed) = user_ws.join().await;
        assert_eq!(raw_subscription, exact_subscription());
        assert!(ping_count >= 2, "fixture must exercise repeated PING/PONG");
        assert!(
            shutdown_observed,
            "fixture must observe the socket shutdown"
        );

        let artifact_secret_guard =
            PmReadOnlyArtifactSecretGuard::for_test(API_KEY, API_SECRET, PASSPHRASE);
        let artifact = finish_private_attempt(
            &config,
            config_evidence,
            provenance,
            started_unix_ms,
            metadata,
            private,
            &artifact_secret_guard,
            &mut output,
        )
        .expect("pass artifact persistence");
        drop(output);

        let offline = verify_pm_read_only_smoke_path(&output_path)
            .expect("persisted artifact must verify offline");
        require_pm_read_only_smoke_pass(&offline).expect("artifact must satisfy every pass gate");
        assert_eq!(offline, artifact);
        assert!(offline.summary.passed);
        assert!(offline.collection_failure.is_none());
        assert!(!offline.production_order_entry_authorized);
        assert!(!offline.mutation_roles_constructed);
        assert_eq!(offline.mutation_requests, 0);

        let reconciliation = offline.reconciliation.as_ref().expect("reconciliation");
        assert_eq!(reconciliation.open_order_page_count, 2);
        assert!(reconciliation.open_order_terminal_cursor_seen);
        assert_eq!(reconciliation.open_order_count, 2);
        assert_eq!(reconciliation.open_order_owner_bound_count, 2);
        assert_eq!(reconciliation.open_order_scope_bound_count, 1);
        assert_eq!(reconciliation.open_order_owner_mismatch_count, 0);
        assert_eq!(reconciliation.open_order_scope_mismatch_count, 1);
        assert_eq!(reconciliation.open_orders.len(), 1);
        assert_eq!(reconciliation.open_orders[0].order_id, EXACT_ORDER);
        assert_eq!(reconciliation.trade_page_count, 1);
        assert!(reconciliation.trade_terminal_cursor_seen);
        assert_eq!(reconciliation.trade_count, 1);
        assert_eq!(reconciliation.trade_owner_bound_count, 1);
        assert_eq!(reconciliation.trade_scope_bound_count, 1);
        assert_eq!(reconciliation.trade_scope_mismatch_count, 0);
        assert_eq!(reconciliation.trades.len(), 1);
        assert_eq!(reconciliation.trades[0].trade_id, "trade-1");

        let stream = offline.user_stream.as_ref().expect("user-stream evidence");
        assert_eq!(stream.connection_attempt_count, 1);
        assert_eq!(stream.connection_open_count, 1);
        assert_eq!(stream.subscription_count, 1);
        assert_eq!(stream.reconnect_attempt_count, 0);
        assert!(stream.ping_count >= 2);
        assert_eq!(stream.correlated_pong_count, stream.ping_count);
        assert_eq!(stream.order_event_count, 1);
        assert_eq!(stream.trade_event_count, 0);
        assert_eq!(stream.owner_bound_event_count, 1);
        assert_eq!(stream.scope_bound_event_count, 1);
        assert_eq!(stream.shutdown_event_count, 1);
        assert!(stream.run_completed_without_transport_error);

        let teardown = &offline.teardown;
        assert!(teardown.user_stream_task_started);
        assert!(teardown.user_stream_shutdown_requested);
        assert!(teardown.user_stream_task_joined);
        assert!(teardown.credential_authority_task_started);
        assert!(teardown.credential_authority_shutdown_requested);
        assert!(teardown.credential_authority_task_joined);
        assert!(teardown.credentials_loaded);
        assert!(teardown.credentials_dropped_before_return);
        assert!(teardown.all_tasks_joined);
        assert!(!teardown.mutation_roles_constructed);
        assert_eq!(teardown.mutation_requests, 0);

        let output_metadata = fs::metadata(&output_path).unwrap();
        assert_eq!(output_metadata.mode() & 0o7777, 0o600);
        assert_eq!(output_metadata.nlink(), 1);

        let requests = http.captured();
        let signatures = assert_get_only_and_auth_partition(&requests);

        let artifact_text = fs::read_to_string(&output_path).unwrap();
        let mut canaries = vec![
            API_KEY.to_owned(),
            API_SECRET.to_owned(),
            PASSPHRASE.to_owned(),
            raw_subscription,
        ];
        canaries.extend(signatures);
        assert_no_secret_derived_material(&artifact_text, &canaries);
    }

    #[tokio::test]
    async fn rejected_trade_after_split_persists_a_verified_nonpass_with_clean_teardown() {
        let directory = TempDir::new().expect("temporary readiness directory");
        fs::set_permissions(directory.path(), fs::Permissions::from_mode(0o700)).unwrap();
        let (config, config_evidence) = write_config(&directory);
        let scope = config.wire_scope().unwrap();
        let ws_pong = Arc::new(Notify::new());
        let http =
            spawn_http_fixture(HttpFixtureMode::RejectTrade, Some(Arc::clone(&ws_pong))).await;
        let user_ws = spawn_user_ws_fixture(Some(ws_pong)).await;

        let public = PmPublicHttpConfig::read_only_evidence(
            &http.origin,
            Duration::from_millis(config.connect_timeout_ms),
            Duration::from_millis(config.request_timeout_ms),
        )
        .unwrap();
        let metadata_role = PmPublicMetadataHttpRole::new(public.clone(), scope).unwrap();
        let time_role = PmReadServerTimeHttpRole::new(public).unwrap();
        let private_http = PmPrivateHttpConfig::read_only_evidence(
            &http.origin,
            Duration::from_millis(config.connect_timeout_ms),
            Duration::from_millis(config.request_timeout_ms),
            scope,
        )
        .unwrap();
        let user_config = PmUserWsConfig::read_only_evidence(
            &user_ws.endpoint,
            scope.condition(),
            Duration::from_secs(2),
            Duration::from_secs(2),
            Duration::from_millis(200),
            Duration::from_millis(100),
            MAX_PM_LIVE_BODY_BYTES,
            0,
            Duration::from_millis(10),
            config.user_stream_event_channel_capacity,
            ConnectionEpoch::new(1),
        )
        .unwrap();
        let owner = PmReadOnlyPrivateConnectivityOwner::read_only_evidence(
            config.signer().unwrap(),
            config.funder().unwrap(),
            scope,
            private_http,
            user_config,
            PmReadOnlyCredentialInput::new(API_KEY.into(), API_SECRET.into(), PASSPHRASE.into()),
        )
        .unwrap();

        let output_path = directory.path().join("rejected-trade-artifact.json");
        let mut output = reserve_private_output(&output_path).expect("private output reservation");
        let started_monotonic = Instant::now();
        let started_unix_ms = unix_ms().unwrap();
        let provenance = Provenance::collect(started_monotonic).unwrap();
        let metadata = collect_public_metadata_with_role(&config, metadata_role)
            .await
            .expect("public metadata collection");

        // Exercise the real post-split capabilities but request shutdown as
        // soon as the deliberately rejected trade read returns. A fixture
        // barrier guarantees the authenticated WS has completed PING/PONG
        // first, without spending a second full production dwell interval.
        let roles = owner.split().expect("read-only facade split");
        assert!(!roles.production_order_entry_authorized());
        let user_role = roles.authenticated_user_ws;
        let credential_supervisor = roles.credential_supervisor;
        let (shutdown, shutdown_signal) = pm_user_ws_shutdown_channel();
        let user_started = Instant::now();
        let signer = config.signer().unwrap();
        let user_task = tokio::spawn(async move {
            let mut sink = UserEvidenceSink::new(scope, signer);
            let result = user_role.run(shutdown_signal, &mut sink).await;
            (result, sink)
        });

        let reads = {
            let mut authenticated_http = roles.authenticated_http;
            collect_authenticated_http(&config, &time_role, &mut authenticated_http).await
        };
        let failure = match reads {
            Err(failure) => failure,
            Ok(_) => panic!("trade rejection unexpectedly produced a complete private cut"),
        };
        assert_eq!(failure.stage, "trades");
        assert_eq!(failure.kind, "rejected_status");
        shutdown.request_shutdown();
        let actual_dwell_ms =
            u64::try_from(user_started.elapsed().as_millis()).expect("bounded test dwell");
        let user_output = join_user_task_bounded(
            UserTaskCancellationFailStop::new(user_task),
            Duration::from_millis(USER_STREAM_GRACEFUL_JOIN_MS),
        )
        .await;
        let (user_result, mut user_sink) = user_output;
        user_sink.run_completed_without_transport_error = user_result.is_ok();
        user_sink.dwell_ms = actual_dwell_ms;
        let user_stream = user_sink.into_evidence();
        let credential_teardown = credential_supervisor
            .shutdown_bounded(PM_CREDENTIAL_AUTHORITY_READ_ONLY_SHUTDOWN_BOUNDS)
            .await;
        assert!(
            user_result.is_ok(),
            "user stream must stop through its explicit shutdown path"
        );
        assert!(
            !credential_teardown.abort_requested()
                && credential_teardown.task_joined()
                && credential_teardown.task_completed_cleanly()
                && credential_teardown.credentials_dropped(),
            "credential authority must shut down cleanly"
        );

        let teardown = PmReadOnlyTeardownEvidence {
            user_stream_task_started: true,
            user_stream_shutdown_requested: true,
            user_stream_abort_requested: false,
            user_stream_task_joined: true,
            user_stream_task_completed_cleanly: true,
            credential_authority_task_started: true,
            credential_authority_shutdown_requested: credential_teardown.shutdown_requested(),
            credential_authority_abort_requested: credential_teardown.abort_requested(),
            credential_authority_task_joined: credential_teardown.task_joined(),
            credential_authority_task_completed_cleanly: credential_teardown
                .task_completed_cleanly(),
            credentials_loaded: true,
            credentials_dropped_before_return: credential_teardown.credentials_dropped(),
            all_tasks_joined: credential_teardown.task_joined(),
            mutation_roles_constructed: false,
            mutation_requests: 0,
        };
        let private = PrivateCollection {
            account: None,
            reconciliation: None,
            user_stream: Some(user_stream),
            teardown,
            failure: Some(failure),
        };

        http.join().await;
        let (raw_subscription, ping_count, shutdown_observed) = user_ws.join().await;
        assert_eq!(raw_subscription, exact_subscription());
        assert!(ping_count >= 1, "failure fixture must authenticate a PONG");
        assert!(
            shutdown_observed,
            "fixture must observe the socket shutdown"
        );

        let artifact_secret_guard =
            PmReadOnlyArtifactSecretGuard::for_test(API_KEY, API_SECRET, PASSPHRASE);
        let artifact = finish_private_attempt(
            &config,
            config_evidence,
            provenance,
            started_unix_ms,
            metadata,
            private,
            &artifact_secret_guard,
            &mut output,
        )
        .expect("nonpass artifact persistence");
        drop(output);

        let offline = verify_pm_read_only_smoke_path(&output_path)
            .expect("persisted nonpass must remain structurally verifiable");
        assert_eq!(offline, artifact);
        assert!(!offline.summary.passed);
        assert!(offline.summary.authorization_closed);
        assert!(offline.summary.teardown_complete);
        assert!(require_pm_read_only_smoke_pass(&offline).is_err());
        let observed_failure = offline.collection_failure.as_ref().expect("typed failure");
        assert_eq!(observed_failure.stage, "trades");
        assert_eq!(observed_failure.kind, "rejected_status");
        assert!(offline.account.is_none());
        assert!(offline.reconciliation.is_none());
        assert!(offline.user_stream.is_some());
        assert!(!offline.production_order_entry_authorized);
        assert!(!offline.mutation_roles_constructed);
        assert_eq!(offline.mutation_requests, 0);

        let teardown = &offline.teardown;
        assert!(teardown.user_stream_task_started);
        assert!(teardown.user_stream_shutdown_requested);
        assert!(teardown.user_stream_task_joined);
        assert!(teardown.credential_authority_task_started);
        assert!(teardown.credential_authority_shutdown_requested);
        assert!(teardown.credential_authority_task_joined);
        assert!(teardown.credentials_loaded);
        assert!(teardown.credentials_dropped_before_return);
        assert!(teardown.all_tasks_joined);
        assert!(!teardown.mutation_roles_constructed);
        assert_eq!(teardown.mutation_requests, 0);
        assert_eq!(fs::metadata(&output_path).unwrap().mode() & 0o7777, 0o600);

        let requests = http.captured();
        let signatures = assert_get_only_and_auth_partition(&requests);
        let artifact_text = fs::read_to_string(&output_path).unwrap();
        let mut canaries = vec![
            API_KEY.to_owned(),
            API_SECRET.to_owned(),
            PASSPHRASE.to_owned(),
            raw_subscription,
        ];
        canaries.extend(signatures);
        assert_no_secret_derived_material(&artifact_text, &canaries);
    }
}
