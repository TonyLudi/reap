const MANIFEST: &str = include_str!("../Cargo.toml");
const LIB: &str = include_str!("../src/lib.rs");
const CONFIG: &str = include_str!("../src/config.rs");
const GEOBLOCK_HTTP: &str = include_str!("../src/geoblock_http.rs");
const HTTP_TRANSPORT: &str = include_str!("../src/http_transport.rs");
const LOOPBACK_MUTATION_CREDENTIALS: &str = include_str!("../src/loopback_mutation_credentials.rs");
const METADATA_HTTP: &str = include_str!("../src/metadata_http.rs");
const OBSERVATION_CLOCK: &str = include_str!("../src/observation_clock.rs");
const PRIVATE_HTTP: &str = include_str!("../src/private_http.rs");
const PRIVATE_CREDENTIALS: &str = include_str!("../src/private_credentials.rs");
const PRODUCT_CLOCK: &str = include_str!("../src/product_clock.rs");
const PUBLIC_CONNECTIVITY: &str = include_str!("../src/public_connectivity.rs");
const PUBLIC_HTTP: &str = include_str!("../src/public_http.rs");
const PUBLIC_WS: &str = include_str!("../src/public_ws.rs");
const PUBLIC_WS_CONFIG: &str = include_str!("../src/public_ws_config.rs");
const READ_ONLY_PRIVATE: &str = include_str!("../src/read_only_private.rs");
const RECONCILIATION: &str = include_str!("../src/reconciliation.rs");
const ACCOUNT: &str = include_str!("../src/account.rs");
const USER_WS: &str = include_str!("../src/user_ws.rs");
const USER_WS_CONFIG: &str = include_str!("../src/user_ws_config.rs");
const TASK_GUARD: &str = include_str!("../src/task_guard.rs");

#[test]
fn phase3_foundation_has_only_role_specific_dependencies() {
    for required in [
        "reap-pm-core.workspace = true",
        "reap-polymarket-adapter.workspace = true",
        "async-trait.workspace = true",
        "reap-polymarket-auth.workspace = true",
        "reap-polymarket-wire.workspace = true",
        "reqwest.workspace = true",
        "futures-util.workspace = true",
        "tokio.workspace = true",
        "tokio-tungstenite.workspace = true",
        "sha2.workspace = true",
        "sha3.workspace = true",
        "zeroize.workspace = true",
    ] {
        assert!(
            MANIFEST.contains(required),
            "missing dependency: {required}"
        );
    }
    for forbidden in ["reap-pm-live", "hmac.workspace", "k256.workspace"] {
        assert!(
            !MANIFEST.contains(forbidden),
            "forbidden dependency: {forbidden}"
        );
    }
}

#[test]
fn modules_are_separate_and_raw_transport_remains_private() {
    for module in [
        "mod config;",
        "mod error;",
        "mod geoblock_http;",
        "mod http_transport;",
        "mod metadata_http;",
        "mod observation_clock;",
        "mod private_http;",
        "mod private_credentials;",
        "mod product_clock;",
        "mod public_connectivity;",
        "mod public_http;",
        "mod public_ws;",
        "mod public_ws_config;",
        "mod read_only_private;",
        "mod reconciliation;",
        "mod task_guard;",
        "mod account;",
        "mod user_ws;",
        "mod user_ws_config;",
    ] {
        assert!(LIB.contains(module), "missing private module: {module}");
    }
    for escape in [
        "PmHttpTransport",
        "PmPrivateHttpTransport",
        "PmPrivateRoute",
        "reqwest::Client",
        "PmPublicRoute",
        "WebSocketStream",
        "tokio_tungstenite::tungstenite::Message",
    ] {
        assert!(!LIB.contains(escape), "raw transport escape: {escape}");
    }
    assert!(CONFIG.contains("#[cfg(test)]\n    pub(crate) fn local_evidence"));
    assert!(!LIB.contains("local_evidence"));
}

#[test]
fn public_preflight_observations_are_source_clocked_committed_and_sealed() {
    assert!(METADATA_HTTP.contains(r#"reap.pm.live-adapter.public-metadata-observation.v2\0"#));
    assert!(!METADATA_HTTP.contains(r#"reap.pm.live-adapter.public-metadata-observation.v1\0"#));
    for required in [
        "pub struct PmHttpReceiveClock",
        "pub(crate) struct PmHttpReceiveClockSource",
        "SystemTime::now()",
        "pub(crate) fn observe(&self) -> Result<PmHttpReceiveClock",
        "pub async fn refresh_typed_observation(",
        "let receive_clock = self.clock.observe()?;",
        "LIVE_METADATA_OBSERVATION_COMMITMENT_DOMAIN",
        "live_metadata_observation_commitment(",
        "encode_metadata_bytes(&mut digest, market_bytes);",
        "encode_metadata_bytes(&mut digest, clob_v2_bytes);",
        "details.lifecycle()",
        "details.clob()",
        "pub async fn status_observation(&self)",
        "GEOBLOCK_OBSERVATION_COMMITMENT_DOMAIN",
        "geoblock_observation_commitment(",
        "encode_geoblock_bytes(&mut digest, raw_response);",
        "status.blocked()",
        "status.ip()",
        "origin_mode_name(mode)",
    ] {
        let source = [OBSERVATION_CLOCK, METADATA_HTTP, GEOBLOCK_HTTP].join("\n");
        assert!(
            source.contains(required),
            "missing sealed public observation invariant: {required}"
        );
    }

    for forbidden in [
        "pub fn from_source(",
        "pub const fn from_source_bytes(",
        "pub fn raw_body(",
        "pub const fn raw_body(",
        "pub fn test_support_new(",
        "PRODUCTION_ORDER_ENTRY_AUTHORIZED: bool = true",
    ] {
        let source = [OBSERVATION_CLOCK, METADATA_HTTP, GEOBLOCK_HTTP].join("\n");
        assert!(
            !source.contains(forbidden),
            "public observation construction/raw/authority escape: {forbidden}"
        );
    }

    let metadata_start = METADATA_HTTP
        .find("pub struct PmLiveMetadataObservationCommitment")
        .unwrap();
    let metadata_end = METADATA_HTTP[metadata_start..]
        .find("pub struct PmPublicMetadataHttpRole")
        .map(|offset| metadata_start + offset)
        .unwrap();
    let metadata_carriers = &METADATA_HTTP[metadata_start..metadata_end];
    for forbidden_field in ["market_bytes:", "clob_v2_bytes:", "raw_response:"] {
        assert!(
            !metadata_carriers.contains(forbidden_field),
            "metadata carrier leaks native response bytes: {forbidden_field}"
        );
    }

    let geoblock_start = GEOBLOCK_HTTP
        .find("pub struct PmGeoblockObservationCommitment")
        .unwrap();
    let geoblock_end = GEOBLOCK_HTTP[geoblock_start..]
        .find("pub struct PmGeoblockHttpRole")
        .map(|offset| geoblock_start + offset)
        .unwrap();
    let geoblock_carriers = &GEOBLOCK_HTTP[geoblock_start..geoblock_end];
    assert!(!geoblock_carriers.contains("raw_response:"));
    assert!(!geoblock_carriers.contains("body:"));
}

#[test]
fn server_time_and_type_one_account_observations_are_sealed_and_source_clocked() {
    let source = [PUBLIC_HTTP, PRIVATE_HTTP, ACCOUNT].join("\n");
    for required in [
        "pub struct PmReadServerTimeObservation",
        "pub struct PmMutationServerTimeObservation",
        "pub async fn fresh_read_server_time_observation(",
        "pub async fn fresh_mutation_server_time_observation(",
        "pub fn into_read_server_time(self) -> PmReadServerTime",
        "pub fn into_pending_mutation_server_time(self) -> PmPendingMutationServerTime",
        "READ_SERVER_TIME_OBSERVATION_COMMITMENT_DOMAIN",
        "MUTATION_SERVER_TIME_OBSERVATION_COMMITMENT_DOMAIN",
        "encode_server_time_bytes(&mut digest, &fetched.raw_response);",
        "fetched.parsed_l2_timestamp.unix_seconds()",
        "fetched.receive_clock.local_wall_receive_ns()",
        "pub struct PmClosedOnlyObservation",
        "pub async fn closed_only_observation(",
        "let fetched = self.closed_only_source(server_time).await?;",
        "observe_authenticated_read_complete()",
        "CLOSED_ONLY_OBSERVATION_COMMITMENT_DOMAIN",
        "pub struct PmAccountBalanceAllowanceObservation",
        "pub async fn collateral_balance_allowance_observation(",
        "pub async fn conditional_balance_allowance_observation(",
        "BALANCE_ALLOWANCE_OBSERVATION_COMMITMENT_DOMAIN",
        "self.transport.configured_signer().bytes()",
        "signature_type.value()",
        "parsed.asset()",
        "encode_balance_bytes(&mut digest, raw_response);",
        "parsed.value().balance()",
    ] {
        assert!(
            source.contains(required),
            "missing server/private provenance invariant: {required}"
        );
    }

    for forbidden in [
        "pub fn from_source(",
        "pub const fn from_source_bytes(",
        "pub fn raw_response(",
        "pub const fn raw_response(",
        "pub fn commitment_from_bytes(",
        "PRODUCTION_ORDER_ENTRY_AUTHORIZED: bool = true",
    ] {
        assert!(
            !source.contains(forbidden),
            "server/private observation construction or authority escape: {forbidden}"
        );
    }

    let fetch_start = PUBLIC_HTTP.find("async fn fetch_server_time").unwrap();
    let fetch_end = PUBLIC_HTTP[fetch_start..]
        .find("fn server_time_observation_commitment")
        .map(|offset| fetch_start + offset)
        .unwrap();
    let fetch = &PUBLIC_HTTP[fetch_start..fetch_end];
    assert!(
        fetch.find("let received = observe_rest_edge()").unwrap()
            < fetch
                .find("let seconds = parse_server_time(&body)")
                .unwrap()
    );

    let closed_observe_start = PRIVATE_HTTP
        .find("pub async fn closed_only_observation(")
        .unwrap();
    let closed_observe_end = PRIVATE_HTTP[closed_observe_start..]
        .find("async fn closed_only_source(")
        .map(|offset| closed_observe_start + offset)
        .unwrap();
    let closed_observe = &PRIVATE_HTTP[closed_observe_start..closed_observe_end];
    assert!(
        closed_observe
            .find("self.closed_only_source(server_time)")
            .unwrap()
            < closed_observe
                .find("observe_authenticated_read_complete()")
                .unwrap()
    );

    let balance_observe_start = ACCOUNT.find("async fn read_observation(").unwrap();
    let balance_observe_end = ACCOUNT[balance_observe_start..]
        .find("async fn read_source(")
        .map(|offset| balance_observe_start + offset)
        .unwrap();
    let balance_observe = &ACCOUNT[balance_observe_start..balance_observe_end];
    assert!(
        balance_observe
            .find("self.read_source(server_time, route, asset)")
            .unwrap()
            < balance_observe
                .find("observe_authenticated_read_complete()")
                .unwrap()
    );

    let closed_commitment_start = PRIVATE_HTTP
        .find("fn closed_only_observation_commitment(")
        .unwrap();
    let closed_commitment_end = PRIVATE_HTTP[closed_commitment_start..]
        .find("fn encode_closed_only_bytes")
        .map(|offset| closed_commitment_start + offset)
        .unwrap();
    let closed_commitment = &PRIVATE_HTTP[closed_commitment_start..closed_commitment_end];
    let balance_commitment_start = ACCOUNT
        .find("pub(crate) fn balance_allowance_observation_commitment(")
        .unwrap();
    let balance_commitment_end = ACCOUNT[balance_commitment_start..]
        .find("fn encode_balance_bytes")
        .map(|offset| balance_commitment_start + offset)
        .unwrap();
    let balance_commitment = &ACCOUNT[balance_commitment_start..balance_commitment_end];
    for commitment in [closed_commitment, balance_commitment] {
        assert!(!commitment.contains("funder"));
        assert!(!commitment.contains("expected_order_maker"));
    }
    assert!(PRIVATE_HTTP.contains("response does not remotely attest"));
    assert!(ACCOUNT.contains("was echoed or remotely attested"));
}

#[test]
fn authenticated_reconciliation_observations_are_source_owned_terminal_clocked_and_sealed() {
    let source = [PRIVATE_HTTP, RECONCILIATION].join("\n");
    for required in [
        r#"reap.pm.live-adapter.open-orders-page-source.v1\0"#,
        r#"reap.pm.live-adapter.trades-page-source.v1\0"#,
        r#"reap.pm.live-adapter.complete-open-orders-observation.v1\0"#,
        r#"reap.pm.live-adapter.complete-trades-observation.v1\0"#,
        r#"reap.pm.live-adapter.exact-order-source.v1\0"#,
        r#"reap.pm.live-adapter.exact-order-observation.v1\0"#,
        "struct AuthenticatedPageSource",
        "struct LiveCutSource",
        "terminal_receive_clock: Option<PmPrivateReadEdgeClock>",
        "pub struct PmCompleteOpenOrdersObservation",
        "pub struct PmCompleteTradesObservation",
        "pub struct PmExactOrderDetailObservation",
        "pub async fn begin_open_orders_observation(",
        "pub async fn continue_open_orders_observation(",
        "pub async fn begin_trades_observation(",
        "pub async fn continue_trades_observation(",
        "fn observe_terminal_page<P: ReconciliationPage>(",
        "pub fn seal_complete_open_orders(",
        "pub fn seal_complete_trades(",
        "pub async fn exact_local_order_detail_observation(",
        "PmPrivateHttpObservation::NotFound =>",
        "exact_order_source_commitment(",
        "encode_reconciliation_bytes(&mut digest, b\"next_cursor\");",
        "encode_page_projection(&mut digest, parsed);",
        "encode_exact_order_classification(&mut digest, classification);",
        "fn encode_live_order(",
        "for trade in self.trades()",
        "configured_expected_maker",
        "signature_type.value()",
        "PM_CLOB_PRODUCTION_ORIGIN.as_bytes()",
        "live_source: None",
        "does not claim a separately echoed funder identity",
        "reconciliation_commitments_exclude_api_key_but_owner_binding_stays_strict",
    ] {
        assert!(
            source.contains(required),
            "missing reconciliation provenance invariant: {required}"
        );
    }

    for retired in [
        r#"reap.pm.live-adapter.open-orders-page-source.v0\0"#,
        r#"reap.pm.live-adapter.trades-page-source.v0\0"#,
        r#"reap.pm.live-adapter.complete-open-orders-observation.v0\0"#,
        r#"reap.pm.live-adapter.complete-trades-observation.v0\0"#,
        r#"reap.pm.live-adapter.exact-order-source.v0\0"#,
        r#"reap.pm.live-adapter.exact-order-observation.v0\0"#,
    ] {
        assert!(!RECONCILIATION.contains(retired));
    }

    for forbidden in [
        "pub fn from_source(",
        "pub const fn from_source_bytes(",
        "pub fn raw_response(",
        "pub fn raw_body(",
        "pub fn source_commitment(",
        "PRODUCTION_ORDER_ENTRY_AUTHORIZED: bool = true",
    ] {
        assert!(
            !source.contains(forbidden),
            "reconciliation construction/raw/authority escape: {forbidden}"
        );
    }

    let begin = RECONCILIATION
        .find("async fn begin_open_orders_source(")
        .unwrap();
    let begin_end = RECONCILIATION[begin..]
        .find("/// Consume the only authority to continue")
        .map(|offset| begin + offset)
        .unwrap();
    let begin = &RECONCILIATION[begin..begin_end];
    assert!(
        begin.find(".open_orders_page(").unwrap()
            < begin.find("observe_terminal_page(&fetched.parsed").unwrap()
    );

    let continue_cut = RECONCILIATION
        .find("async fn continue_open_orders_source(")
        .unwrap();
    let continue_cut_end = RECONCILIATION[continue_cut..]
        .find("/// Start an unfiltered full-account trade cut")
        .map(|offset| continue_cut + offset)
        .unwrap();
    let continue_cut = &RECONCILIATION[continue_cut..continue_cut_end];
    assert!(
        continue_cut.find(".open_orders_page(").unwrap()
            < continue_cut
                .find("observe_terminal_page(&fetched.parsed")
                .unwrap()
    );

    for seal_name in [
        "pub fn seal_complete_open_orders(",
        "pub fn seal_complete_trades(",
    ] {
        let seal = RECONCILIATION.find(seal_name).unwrap();
        let seal_end = RECONCILIATION[seal..]
            .find("\n    ///")
            .map(|offset| seal + offset)
            .unwrap();
        let seal = &RECONCILIATION[seal..seal_end];
        assert!(!seal.contains("PmPrivateReadProductClock"));
        assert!(!seal.contains("observe_authenticated_read_complete"));
        assert!(seal.contains("terminal_receive_clock"));
    }

    let open_page = RECONCILIATION.find("async fn open_orders_page(").unwrap();
    let open_page_end = RECONCILIATION[open_page..]
        .find("async fn trades_page(")
        .map(|offset| open_page + offset)
        .unwrap();
    let open_page = &RECONCILIATION[open_page..open_page_end];
    assert!(open_page.contains("let body = found("));
    assert!(
        open_page.find("bind_open_orders(").unwrap()
            < open_page.find("authenticated_page_source(").unwrap()
    );
    assert!(PRIVATE_HTTP.contains("Found(Zeroizing<Vec<u8>>)"));
    assert!(PRIVATE_HTTP.contains("PmPrivateHttpObservation::NotFound"));

    let page_source = RECONCILIATION
        .find("fn authenticated_page_source<P: ReconciliationPage>(")
        .unwrap();
    let page_source_end = RECONCILIATION[page_source..]
        .find("fn exact_order_source_commitment(")
        .map(|offset| page_source + offset)
        .unwrap();
    let page_source = &RECONCILIATION[page_source..page_source_end];
    let exact_source = RECONCILIATION
        .find("fn exact_order_source_commitment(")
        .unwrap();
    let exact_source_end = RECONCILIATION[exact_source..]
        .find("fn complete_open_orders_observation_commitment(")
        .map(|offset| exact_source + offset)
        .unwrap();
    let exact_source = &RECONCILIATION[exact_source..exact_source_end];
    for secret_escape in [
        "raw_response",
        "raw_body",
        "&body",
        ".owner()",
        ".trade_owner()",
        "api_key",
        "API_KEY",
    ] {
        assert!(
            !page_source.contains(secret_escape) && !exact_source.contains(secret_escape),
            "durable reconciliation source commitment is secret-derived: {secret_escape}"
        );
    }
    assert!(page_source.contains("encode_page_projection(&mut digest, parsed)"));
    assert!(exact_source.contains("encode_exact_order_classification"));
    for secret_projection in ["raw_response", "raw_body", ".owner()", ".trade_owner()"] {
        assert!(
            !RECONCILIATION.contains(secret_projection),
            "reconciliation provenance retains secret-bearing projection: {secret_projection}"
        );
    }

    let exact_fetch = RECONCILIATION.find("async fn exact_order_source(").unwrap();
    let exact_fetch_end = RECONCILIATION[exact_fetch..]
        .find("async fn open_orders_page(")
        .map(|offset| exact_fetch + offset)
        .unwrap();
    let exact_found = RECONCILIATION[exact_fetch..exact_fetch_end]
        .find("PmPrivateHttpObservation::Found(body)")
        .map(|offset| exact_fetch + offset)
        .unwrap();
    let exact_found = &RECONCILIATION[exact_found..exact_fetch_end];
    assert!(
        exact_found.find(".bind_exact_order(").unwrap()
            < exact_found.find("exact_order_source_commitment(").unwrap()
    );

    for carrier in [
        "PmCompleteOpenOrdersObservation",
        "PmCompleteTradesObservation",
        "PmExactOrderDetailObservation",
    ] {
        let declaration = format!("pub struct {carrier} {{");
        let start = RECONCILIATION.find(&declaration).unwrap();
        let end = RECONCILIATION[start..]
            .find("\n}")
            .map(|offset| start + offset)
            .unwrap();
        let body = &RECONCILIATION[start..end];
        assert!(!body.contains("raw_response:"));
        assert!(!body.contains("raw_body:"));
    }

    let move_only_start = RECONCILIATION
        .find("/// Move-only terminal observation of a real authenticated open-order cut.")
        .unwrap();
    let move_only_end = RECONCILIATION[move_only_start..]
        .find("/// Borrowed authenticated capability")
        .map(|offset| move_only_start + offset)
        .unwrap();
    let move_only_carriers = &RECONCILIATION[move_only_start..move_only_end];
    for forbidden_trait in [
        "#[derive(",
        "Serialize",
        "Deserialize",
        "impl Clone",
        "impl Copy",
    ] {
        assert!(
            !move_only_carriers.contains(forbidden_trait),
            "sealed reconciliation carrier gains forgeable trait: {forbidden_trait}"
        );
    }
    assert!(RECONCILIATION.matches("live_source: None").count() >= 2);
}

#[test]
fn production_read_only_facade_is_opaque_exact_bound_and_mutation_free() {
    for required in [
        "pub struct PmReadOnlyCredentialInput",
        "api_key: Zeroizing<String>",
        "secret: Zeroizing<String>",
        "passphrase: Zeroizing<String>",
        "pub struct PmReadOnlyPrivateConnectivityOwner",
        "pub fn production(",
        "expected_signer: EvmAddress",
        "expected_funder: EvmAddress",
        "exact_scope: PmWireScope",
        "validate_one_eoa(expected_signer, expected_funder)?",
        "credentials.address().as_core() != expected_eoa",
        "pub struct PmReadOnlyPrivateConnectivityRoles",
        "pub server_time: PmReadServerTimeHttpRole",
        "pub geoblock: PmGeoblockHttpRole",
        "pub market_details: PmPublicMetadataHttpRole",
        "pub authenticated_http: PmAuthenticatedHttpOwner",
        "pub authenticated_user_ws: PmAuthenticatedUserWsRole",
        "pub credential_supervisor: PmCredentialAuthoritySupervisor",
        "pub const fn production_order_entry_authorized(&self) -> bool",
    ] {
        assert!(
            READ_ONLY_PRIVATE.contains(required),
            "missing read-only facade invariant: {required}"
        );
    }
    for forbidden in [
        "pub fn authenticate_place",
        "pub fn authenticate_cancel",
        "pub fn credentials(",
        "pub fn signer(",
        "PmMutationServerTime",
        "PmMutationServerTimeValidator",
        "PmPendingMutationServerTime",
        "FixedEoaSigner",
    ] {
        assert!(
            !READ_ONLY_PRIVATE.contains(forbidden),
            "read-only facade mutation/auth escape: {forbidden}"
        );
    }
    assert!(LIB.contains("PmReadOnlyCredentialInput"));
    assert!(LIB.contains("PmReadOnlyPrivateConnectivityOwner"));
    assert!(!LIB.contains("L2CredentialInput"));
    assert!(!LIB.contains("L2Credentials"));
    assert!(LIB.contains("PRODUCTION_ORDER_ENTRY_AUTHORIZED: bool = false"));
}

#[test]
fn account_only_facade_releases_no_websocket_reconciliation_or_mutation_role() {
    for required in [
        "pub struct PmReadOnlyAccountConnectivityOwner",
        "pub fn production(",
        "signature_type: PmReadOnlySignatureType",
        "conditional_token: PmTokenId",
        "expected_signer.bytes() == [0; 20] || expected_funder.bytes() == [0; 20]",
        "funder is therefore an operator-reviewed structural input",
        "PmPrivateHttpTransport::for_account(&config, credentials.address())?",
        "account_http_credential_role(self.credentials)?",
        "pub struct PmReadOnlyAccountConnectivityRoles",
        "pub server_time: PmReadServerTimeHttpRole",
        "pub authenticated_account: PmReadOnlyAccountHttpOwner",
        "pub credential_supervisor: PmCredentialAuthoritySupervisor",
    ] {
        assert!(
            READ_ONLY_PRIVATE.contains(required),
            "missing account-only facade invariant: {required}"
        );
    }

    let roles_start = READ_ONLY_PRIVATE
        .find("pub struct PmReadOnlyAccountConnectivityRoles")
        .unwrap();
    let roles_end = READ_ONLY_PRIVATE[roles_start..]
        .find("/// Sole production-safe constructor")
        .map(|offset| roles_start + offset)
        .unwrap();
    let roles = &READ_ONLY_PRIVATE[roles_start..roles_end];
    for forbidden in [
        "PmAuthenticatedUserWsRole",
        "PmAuthenticatedHttpOwner",
        "reconciliation",
        "exact_order",
        "mutation",
    ] {
        assert!(
            !roles.contains(forbidden),
            "account-only role escape: {forbidden}"
        );
    }
    assert!(LIB.contains("PmReadOnlyAccountConnectivityOwner"));
    assert!(LIB.contains("PmReadOnlyAccountHttpOwner"));
}

#[test]
fn read_only_evidence_feature_cannot_enable_mutation_constructors() {
    assert!(MANIFEST.contains("read-only-evidence = []"));
    assert!(!MANIFEST.contains("read-only-evidence = [\"loopback-evidence\"]"));
    for source in [CONFIG, USER_WS_CONFIG, READ_ONLY_PRIVATE] {
        assert!(source.contains("feature = \"read-only-evidence\""));
        assert!(source.contains("pub fn read_only_evidence("));
    }
    assert!(!LOOPBACK_MUTATION_CREDENTIALS.contains("read-only-evidence"));
}

#[test]
fn standalone_read_time_role_constructs_no_mutation_clock_capability() {
    assert!(PUBLIC_HTTP.contains("pub fn new(config: PmPublicHttpConfig)"));
    assert!(PUBLIC_HTTP.contains("PmReadServerTimeProductClock::standalone_system()"));
    assert!(PRODUCT_CLOCK.contains("impl PmReadServerTimeProductClock"));
    assert!(PRODUCT_CLOCK.contains("fn standalone_system() -> Self"));
    assert!(LIB.contains("PRODUCTION_ORDER_ENTRY_AUTHORIZED: bool = false"));
}

#[test]
fn websocket_socket_workers_are_abort_on_outer_drop_and_explicitly_joined() {
    for source in [PUBLIC_WS, USER_WS] {
        assert!(source.contains("AbortOnDropTask::new(tokio::spawn"));
        assert!(source.contains("worker.abort_and_join().await"));
        assert!(source.contains("worker\n            .join()\n            .await"));
        assert!(!source.contains("let worker = tokio::spawn"));
    }
    assert!(TASK_GUARD.contains("impl<T> Drop for AbortOnDropTask<T>"));
    assert!(TASK_GUARD.contains("task.abort()"));
    assert!(TASK_GUARD.contains("pub(crate) async fn join(mut self)"));
}

#[test]
fn public_market_websocket_is_exact_scoped_bounded_and_transport_private() {
    for required in [
        "wss://ws-subscriptions-clob.polymarket.com/ws/market",
        "PM_PUBLIC_WS_HEARTBEAT_INTERVAL: Duration = Duration::from_secs(10)",
        "PmMarketSubscription::new(config.scope().token()).to_json()?",
        "socket.send(Message::text(APPLICATION_PING))",
        "mpsc::channel(self.config.event_channel_capacity())",
        "sink.deliver_public_ws_event(event).await",
        "max_message_size(Some(config.max_frame_bytes()))",
        "max_frame_size(Some(config.max_frame_bytes()))",
        "PmPublicWsEvent::RawData",
        "PmPublicWsEvent::PingSent",
        "PmPublicWsEvent::Pong",
        "PmPublicWsEvent::ReconnectScheduled",
        "PmPublicWsReconnectDirective",
        "request_reconnect_authority(&events, retired).await?",
        "PmPublicWsClockSource",
        "observe(clock)?",
    ] {
        let source = [PUBLIC_WS_CONFIG, PUBLIC_WS].join("\n");
        assert!(
            source.contains(required),
            "missing public-WS invariant: {required}"
        );
    }
    for forbidden in [
        "L2Credentials",
        "AuthenticatedUserSubscription",
        "apiKey",
        "passphrase",
        "authenticate_",
        "place_order",
        "cancel_order",
        "pub fn socket(",
        "pub fn client(",
        "pub fn endpoint(",
        "pub fn send(",
    ] {
        let source = [PUBLIC_WS_CONFIG, PUBLIC_WS].join("\n");
        assert!(
            !source.contains(forbidden),
            "forbidden public-WS capability: {forbidden}"
        );
    }
    assert!(PUBLIC_WS_CONFIG.contains("feature = \"loopback-evidence\""));
    assert!(PUBLIC_WS_CONFIG.contains("pub fn loopback_evidence"));
    assert!(!LIB.contains("local_evidence"));
    assert!(LIB.contains("PRODUCTION_ORDER_ENTRY_AUTHORIZED: bool = false"));
}

#[test]
fn rest_book_delivery_can_await_durability_before_releasing_parsed_evidence() {
    for required in [
        "#[async_trait::async_trait]",
        "pub trait PmRestBookSnapshotSink: Send",
        "async fn deliver_native_rest_book(",
        "sink.deliver_native_rest_book(purpose, received, &raw)",
        ".await",
        ".observe_rest_edge()",
    ] {
        assert!(
            PUBLIC_HTTP.contains(required),
            "missing durability-first REST-book delivery invariant: {required}"
        );
    }
}

#[test]
fn authenticated_public_bundle_owns_one_clock_domain_and_no_default_clock_escape() {
    for required in [
        "pub struct PmProductClockOwner",
        "Arc<ProductClockDomain>",
        "Arc::ptr_eq(&self.domain, &pending.domain)",
        "PM_MUTATION_SERVER_TIME_MAX_AGE",
        "pub struct PmPendingMutationServerTime",
        "pub struct PmAuthorizedMutationServerTime",
        "pub struct PmReadServerTime",
        "validate_age(&domain, received)?",
        "pub struct PmPublicConnectivityOwner",
        "PmPublicHttpRole::with_product_clock(",
        "PmPublicMarketWsRole::with_clock_source(public_ws_config, public_ws_clock)",
    ] {
        let source = [PRODUCT_CLOCK, PUBLIC_CONNECTIVITY].join("\n");
        assert!(
            source.contains(required),
            "missing shared product-clock invariant: {required}"
        );
    }
    for forbidden in [
        "Arc<Mutex",
        "pub fn into_l2_timestamp",
        "pub const fn into_l2_timestamp",
        "pub fn timestamp(",
        "pub fn server_time_seconds",
    ] {
        let source = [PRODUCT_CLOCK, PUBLIC_HTTP].join("\n");
        assert!(
            !source.contains(forbidden),
            "raw clock/time capability escape: {forbidden}"
        );
    }
}

#[test]
fn loopback_mutation_custody_is_scope_bound_feature_gated_and_separate_from_read_owner() {
    assert!(LIB.contains("#[cfg(any(test, feature = \"loopback-evidence\"))]"));
    assert!(LIB.contains("mod loopback_mutation_credentials;"));
    for required in [
        "PmLoopbackMutationConnectivityOwner",
        "PmGtcPostOnlyPlaceRequest",
        "PmExactOwnedCancelRequest",
        "authenticate_place(",
        "authenticate_cancel(",
        "request.account_scope() != binding.account",
        "request.instrument_id() != binding.trading_domain.instrument()",
        "request.trading_domain() != binding.trading_domain",
        "unsigned.token_id() != binding.trading_domain.instrument().token()",
        "request.purpose() != binding.cancel_purpose",
        "CredentialSlotId",
        "credentials.authenticated_journal_credential_slot(credential_slot)",
        "AuthenticatedJournalCredentialSlotFingerprint",
        "PmLoopbackPlaceAuthenticationFailure",
        "PmLoopbackCancelAuthenticationFailure",
        "Arc::clone(&request)",
        "pub fn into_request(self)",
        "mpsc::channel(MUTATION_AUTHORITY_CAPACITY)",
        "PmHttpCredentialRole::from_sender(senders.read.clone())",
        "PmUserWsCredentialRole::from_sender(senders.read)",
        "PmCredentialAuthoritySupervisor::from_task(shutdown, task)",
    ] {
        assert!(
            LOOPBACK_MUTATION_CREDENTIALS.contains(required),
            "missing scope-bound loopback custody invariant: {required}"
        );
    }
    for forbidden in [
        "SignedClobV2Order",
        "pub fn signer(",
        "pub fn credentials(",
        "pub fn token(",
        "pub fn domain(",
        "Arc<L2Credentials>",
        "Option<FixedEoaSigner>",
    ] {
        assert!(
            !LOOPBACK_MUTATION_CREDENTIALS.contains(forbidden),
            "loopback mutation custody escape: {forbidden}"
        );
    }
    assert!(!PRIVATE_CREDENTIALS.contains("PmLoopbackMutation"));
    assert!(!PRIVATE_CREDENTIALS.contains("FixedEoaSigner"));
    assert_eq!(
        LOOPBACK_MUTATION_CREDENTIALS
            .matches("The supervised authenticated worker MUST await this future")
            .count(),
        2,
        "place/cancel cancellation contracts must stay explicit"
    );
    assert_eq!(
        LOOPBACK_MUTATION_CREDENTIALS
            .matches("Process/task stop recovers durable Goal-F intent; it must never resend.")
            .count(),
        2
    );
}

#[test]
fn authenticated_routes_are_fixed_reads_without_filter_or_mutation_escape() {
    for required in [
        "url.set_path(\"/data/orders\")",
        "url.set_path(\"/data/trades\")",
        "format!(\"/data/order/{order_id}\")",
        "url.set_path(\"/balance-allowance\")",
        ".append_pair(\"next_cursor\", cursor)",
        ".append_pair(\"asset_type\", \"COLLATERAL\")",
        ".append_pair(\"asset_type\", \"CONDITIONAL\")",
        ".append_pair(\"signature_type\", signature_type.query_value())",
    ] {
        assert!(
            PRIVATE_HTTP.contains(required),
            "missing fixed route: {required}"
        );
    }
    for forbidden_filter in [
        "append_pair(\"market\"",
        "append_pair(\"asset_id\"",
        "append_pair(\"after\"",
        "append_pair(\"maker_address\"",
    ] {
        assert!(
            !PRIVATE_HTTP.contains(forbidden_filter),
            "account-cut filter escape: {forbidden_filter}"
        );
    }
    for forbidden_method in [".post(", ".put(", ".patch(", ".delete("] {
        assert!(
            !PRIVATE_HTTP.contains(forbidden_method),
            "private mutation method: {forbidden_method}"
        );
    }
    assert!(!PRIVATE_HTTP.contains("/balance-allowance/update"));
    for required in [
        "pub enum PmReadOnlySignatureType",
        "Eoa = 0",
        "Proxy = 1",
        "impl TryFrom<u8> for PmReadOnlySignatureType",
        "_ => Err(PmLiveAdapterError::InvalidConfiguration(",
    ] {
        assert!(
            ACCOUNT.contains(required),
            "missing account profile guard: {required}"
        );
    }
    for forbidden_capability in [
        "authenticate_place",
        "authenticate_owned_cancel",
        "serialize_place",
        "serialize_owned_cancel",
        "update_balance",
        "WebSocket",
        "connect_async",
    ] {
        let authenticated = [PRIVATE_HTTP, RECONCILIATION, ACCOUNT].join("\n");
        assert!(
            !authenticated.contains(forbidden_capability),
            "forbidden private capability: {forbidden_capability}"
        );
    }
}

#[test]
fn borrowing_roles_are_distinct_and_credentials_have_one_owner() {
    assert!(PRIVATE_CREDENTIALS.contains("credentials: L2Credentials"));
    assert!(!PRIVATE_CREDENTIALS.contains("Arc<L2Credentials>"));
    assert!(PRIVATE_CREDENTIALS.contains("mpsc::channel(CREDENTIAL_AUTHORITY_CAPACITY)"));
    assert!(PRIVATE_CREDENTIALS.contains("PmCredentialAuthoritySupervisor"));
    assert!(
        PRIVATE_CREDENTIALS.contains("timeout(bounds.graceful_join_timeout(), &mut *task).await")
    );
    assert!(PRIVATE_CREDENTIALS.contains("timeout(bounds.abort_join_timeout(), &mut *task).await"));
    assert!(!PRIVATE_CREDENTIALS.contains("task.await"));
    assert!(PRIVATE_CREDENTIALS.contains("task.abort()"));
    assert!(PRIVATE_CREDENTIALS.contains("PmCredentialAuthorityShutdownFailStop"));
    assert!(PRIVATE_CREDENTIALS.contains("std::process::abort()"));
    assert!(PRIVATE_CREDENTIALS.contains("let Some(task) = self.task.as_mut()"));
    assert!(PRIVATE_CREDENTIALS.contains("PmCredentialAuthoritySupervisor,"));
    assert!(PRIVATE_CREDENTIALS.contains("PmHttpCredentialRole"));
    assert!(PRIVATE_CREDENTIALS.contains("PmUserWsCredentialRole"));
    assert!(!PRIVATE_CREDENTIALS.contains("SignedClobV2Order"));
    assert!(!PRIVATE_CREDENTIALS.contains("PmMutationAuthenticationRole"));
    assert!(PRIVATE_HTTP.contains("Found(Zeroizing<Vec<u8>>)"));
    assert!(!PRIVATE_HTTP.contains("Found(Vec<u8>)"));
    assert!(!PRIVATE_HTTP.contains("Clone for PmAuthenticatedHttpOwner"));
    assert!(RECONCILIATION.contains("authority: &'a mut PmHttpCredentialRole"));
    assert!(ACCOUNT.contains("authority: &'a mut PmHttpCredentialRole"));
    assert!(RECONCILIATION.contains("assembly: PmOpenOrdersAssembly"));
    assert!(!RECONCILIATION.contains("assembly: &PmOpenOrdersAssembly"));
    assert!(RECONCILIATION.contains("PmOpenOrdersCutProgress::Complete"));
    assert!(RECONCILIATION.contains("PaginationCursorCycle"));
    assert!(RECONCILIATION.contains("MAX_PM_AUTHENTICATED_CUT_PAGES"));
    assert!(RECONCILIATION.contains("MAX_PM_RECONCILIATION_ORDERS"));
    assert!(RECONCILIATION.contains("MAX_PM_RECONCILIATION_FILLS"));
    assert!(!RECONCILIATION.contains("balance_allowance("));
    assert!(!ACCOUNT.contains("open_orders("));
    assert!(!ACCOUNT.contains("trades("));
    for source in [PRIVATE_HTTP, RECONCILIATION, ACCOUNT] {
        assert!(!source.contains("pub fn request("));
        assert!(!source.contains("pub fn headers("));
        assert!(!source.contains("pub fn url("));
        assert!(!source.contains("pub fn path("));
    }
}

#[test]
fn authenticated_user_websocket_is_fixed_bound_and_has_no_raw_or_mutation_escape() {
    let source = [PRIVATE_CREDENTIALS, USER_WS_CONFIG, USER_WS].join("\n");
    for required in [
        "wss://ws-subscriptions-clob.polymarket.com/ws/user",
        "PM_USER_WS_HEARTBEAT_INTERVAL: Duration = Duration::from_secs(10)",
        "fresh_subscription(config.condition()).await",
        "parse_live_user_frame(raw.as_slice())",
        "credentials.bind_frame(frame).await",
        "PmUserWsEvent::BoundFrame",
        "into_credential_owned_frame(self) -> CredentialOwnedUserFrame",
        "Zeroizing::new(text.as_str().as_bytes().to_vec())",
        "mpsc::channel(self.config.event_channel_capacity())",
        "sink.deliver_user_ws_event(event).await",
        "feature = \"loopback-evidence\"",
    ] {
        assert!(
            source.contains(required),
            "missing user-WS invariant: {required}"
        );
    }
    for forbidden in [
        "pub fn socket(",
        "pub fn client(",
        "pub fn endpoint(",
        "pub fn raw",
        "SignedClobV2Order",
        "authenticate_place",
        "authenticate_owned_cancel",
        "operation: String",
        "initial_dump: bool",
    ] {
        assert!(
            !source.contains(forbidden),
            "user-WS escape/stale shape: {forbidden}"
        );
    }
    assert!(LIB.contains("PmPrivateConnectivityOwner"));
    assert!(!LIB.contains("PmMutationAuthenticationRole"));
    assert!(!USER_WS.contains("impl Clone for PmUserWsBoundFrame"));
    assert!(USER_WS.contains(".try_send(event)"));
    assert!(PUBLIC_WS.contains(".try_send(WorkerEvent::Evidence(event))"));
    assert!(USER_WS.contains("EventChannelSaturated"));
    assert!(PUBLIC_WS.contains("EventChannelSaturated"));
    assert!(!USER_WS.contains("SinkDeliveryCancelledByShutdown"));
    assert!(!PUBLIC_WS.contains("SinkDeliveryCancelledByShutdown"));
    assert!(!USER_WS.contains("wait_for_shutdown(&mut sink_shutdown)"));
    assert!(!PUBLIC_WS.contains("wait_for_shutdown(&mut sink_shutdown)"));
    assert!(USER_WS.contains("sink.deliver_user_ws_event(event).await"));
    assert!(PUBLIC_WS.contains("sink.deliver_public_ws_event(event).await"));
    assert!(PUBLIC_WS.contains("sink.authorize_public_ws_reconnect(retired).await"));
}

#[test]
fn public_routes_are_exactly_the_reviewed_get_only_surface() {
    assert!(HTTP_TRANSPORT.contains("url.set_path(\"/time\")"));
    assert!(HTTP_TRANSPORT.contains("url.set_path(\"/book\")"));
    assert!(HTTP_TRANSPORT.contains(".append_pair(\"token_id\""));
    assert!(HTTP_TRANSPORT.contains("format!(\"/markets/{condition}\")"));
    assert!(HTTP_TRANSPORT.contains("format!(\"/clob-markets/{condition}\")"));
    assert!(HTTP_TRANSPORT.contains("url.set_path(\"/api/geoblock\")"));
    assert!(GEOBLOCK_HTTP.contains("PmPublicRoute::Geoblock"));
    assert!(GEOBLOCK_HTTP.contains("MAX_PM_GEOBLOCK_BODY_BYTES"));
    assert!(METADATA_HTTP.contains("PmLiveMetadataPair"));
    assert!(METADATA_HTTP.contains("deliver_native_metadata_pair"));
    assert!(METADATA_HTTP.contains("self.scope.condition()"));
    for forbidden_route in ["/orders", "/trades", "/balance-allowance", "/ws/"] {
        assert!(
            !HTTP_TRANSPORT.contains(forbidden_route),
            "unsupported route: {forbidden_route}"
        );
    }
    for forbidden_method in [".post(", ".put(", ".patch(", ".delete("] {
        assert!(
            !HTTP_TRANSPORT.contains(forbidden_method),
            "unsupported HTTP method: {forbidden_method}"
        );
    }
}

#[test]
fn public_role_has_no_auth_private_ws_or_mutation_capability() {
    let production = [
        CONFIG,
        GEOBLOCK_HTTP,
        HTTP_TRANSPORT,
        PUBLIC_HTTP,
        METADATA_HTTP,
    ]
    .join("\n");
    for forbidden in [
        "L2Credentials",
        "PrivateKey",
        "POLY_API_KEY",
        "POLY_SIGNATURE",
        "authenticate_",
        "place_order",
        "cancel_order",
        "WebSocket",
        "connect_async",
    ] {
        assert!(
            !production.contains(forbidden),
            "forbidden live capability: {forbidden}"
        );
    }
    assert!(LIB.contains("PRODUCTION_ORDER_ENTRY_AUTHORIZED: bool = false"));
    assert!(!LIB.contains("PRODUCTION_ORDER_ENTRY_AUTHORIZED: bool = true"));
    assert!(PUBLIC_HTTP.contains("PmBookMarketBinding::ConditionId"));
    assert!(!PUBLIC_HTTP.contains("PmBookMarketBinding::LegacyMarketId"));
}
