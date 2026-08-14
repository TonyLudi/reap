const MANIFEST: &str = include_str!("../Cargo.toml");
const WORKSPACE_MANIFEST: &str = include_str!("../../../Cargo.toml");
const LIB: &str = include_str!("../src/lib.rs");
const CLOB_HEALTH_HTTP: &str = include_str!("../src/clob_health_http.rs");
const CONFIG: &str = include_str!("../src/config.rs");
const DEFERRED_MUTATION_TIME: &str = include_str!("../src/deferred_mutation_time.rs");
const GEOBLOCK_HTTP: &str = include_str!("../src/geoblock_http.rs");
const HTTP_TRANSPORT: &str = include_str!("../src/http_transport.rs");
const LOOPBACK_MUTATION_CREDENTIALS: &str = include_str!("../src/loopback_mutation_credentials.rs");
const METADATA_HTTP: &str = include_str!("../src/metadata_http.rs");
const MUTATION_TRANSPORT: &str = include_str!("../src/mutation/transport.rs");
const OBSERVATION_CLOCK: &str = include_str!("../src/observation_clock.rs");
const PRIVATE_HTTP: &str = include_str!("../src/private_http.rs");
const PRIVATE_CREDENTIALS: &str = include_str!("../src/private_credentials.rs");
const PRODUCT_CLOCK: &str = include_str!("../src/product_clock.rs");
const PUBLIC_CONNECTIVITY: &str = include_str!("../src/public_connectivity.rs");
const PUBLIC_HTTP: &str = include_str!("../src/public_http.rs");
const PUBLIC_WS: &str = include_str!("../src/public_ws.rs");
const PUBLIC_WS_CONFIG: &str = include_str!("../src/public_ws_config.rs");
const READ_AUTHORITY: &str = include_str!("../src/read_authority.rs");
const READ_ONLY_PRIVATE: &str = include_str!("../src/read_only_private.rs");
const RECONCILIATION: &str = include_str!("../src/reconciliation.rs");
const SELECTED_WS: &str = include_str!("../src/selected_ws.rs");
const STATUS_ANNOUNCEMENT_HTTP: &str = include_str!("../src/status_announcement_http.rs");
const ACCOUNT: &str = include_str!("../src/account.rs");
const USER_WS: &str = include_str!("../src/user_ws.rs");
const USER_WS_CONFIG: &str = include_str!("../src/user_ws_config.rs");
const TASK_GUARD: &str = include_str!("../src/task_guard.rs");
const WS_TRANSPORT: &str = include_str!("../src/ws_transport.rs");

fn production_prefix(source: &str) -> &str {
    source
        .split_once("#[cfg(test)]\nmod tests")
        .map_or(source, |(head, _)| head)
}

#[test]
fn production_mutation_transport_is_fixed_selected_single_order_and_never_retries() {
    for required in [
        "pub struct PmProductionMutationConfig",
        "production_on_fixed_tls_peer_and_selected_local_egress",
        "fixed_tls_peer.dns_name() != PM_CLOB_PRODUCTION_DNS_NAME",
        "selected_local_egress.require_production()",
        ".require_same_address_family(&selected_local_egress)",
        ".resolve(",
        ".interface(config.selected_local_egress.interface_name())",
        ".local_address(config.selected_local_egress.local_source_ip())",
        ".https_only(true)",
        ".retry(reqwest::retry::never())",
        "self.expected_peer != response.remote_addr()",
        "pub struct PmFixedPlaceProductionRole",
        "pub struct PmExactOwnedCancelProductionRole",
        ".post(url)",
        ".delete(url)",
        "url.set_path(\"/order\")",
    ] {
        assert!(
            MUTATION_TRANSPORT.contains(required),
            "production mutation edge lost `{required}`"
        );
    }
    for forbidden in [
        "set_path(\"/cancel-all\")",
        "set_path(\"/cancel-market-orders\")",
        "set_path(\"/orders\")",
        "Client::new()",
        "Policy::limited",
        "reqwest::retry::default",
    ] {
        assert!(
            !MUTATION_TRANSPORT.contains(forbidden),
            "production mutation edge gained `{forbidden}`"
        );
    }
}

fn between<'a>(source: &'a str, start: &str, end: &str) -> &'a str {
    source
        .split_once(start)
        .and_then(|(_, tail)| tail.split_once(end).map(|(slice, _)| slice))
        .expect("source policy markers must remain ordered")
}

#[test]
fn phase3_foundation_has_only_role_specific_dependencies() {
    for required in [
        "reap-pm-core.workspace = true",
        "reap-polymarket-adapter.workspace = true",
        "async-trait.workspace = true",
        "reap-polymarket-auth.workspace = true",
        "reap-polymarket-egress-binding.workspace = true",
        "reap-polymarket-wire.workspace = true",
        "reqwest.workspace = true",
        "serde.workspace = true",
        "serde_json.workspace = true",
        "futures-util.workspace = true",
        "tokio.workspace = true",
        "tokio-tungstenite.workspace = true",
        "sha2.workspace = true",
        "sha3.workspace = true",
        "socket2.workspace = true",
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
    assert!(
        WORKSPACE_MANIFEST.contains("socket2 = { version = \"=0.6.4\", features = [\"all\"] }")
    );
}

#[test]
fn selected_local_http_egress_is_configuration_only_and_never_a_combined_owner_claim() {
    for required in [
        "loopback-evidence = [\"reap-polymarket-egress-binding/loopback-evidence\"]",
        "read-only-evidence = [\"reap-polymarket-egress-binding/loopback-evidence\"]",
    ] {
        assert!(
            MANIFEST.contains(required),
            "missing selected feature pin: {required}"
        );
    }
    assert_eq!(
        CONFIG
            .matches("selected_local_egress: Option<PmLocalEgressSelection>")
            .count(),
        4,
        "every HTTP configuration family must retain the same optional local selection"
    );
    assert_eq!(
        CONFIG
            .matches("fixed_tls_peer: Option<PmFixedTlsPeerSelection>")
            .count(),
        4,
        "every HTTP configuration family must retain the paired fixed peer privately"
    );
    assert_eq!(
        CONFIG
            .matches("pub fn production_on_selected_local_egress(")
            .count(),
        2,
        "only private-read and geoblock configs expose selected production setup externally"
    );
    assert_eq!(
        CONFIG
            .matches("pub(crate) fn production_on_selected_local_egress(")
            .count(),
        2,
        "status and mixed-purpose public configuration remain crate-private"
    );
    assert_eq!(
        CONFIG
            .matches("pub(crate) const fn selected_local_egress(&self)")
            .count(),
        4,
        "raw config selection is borrowed only by private transports"
    );
    assert_eq!(
        CONFIG
            .matches("pub(crate) const fn fixed_tls_peer(&self)")
            .count(),
        4,
        "fixed peers are borrowed only by private transports"
    );
    assert_eq!(
        CONFIG
            .matches("pub fn production_on_fixed_tls_peer_and_selected_local_egress(")
            .count(),
        2,
        "only private-read and geoblock configs expose paired production setup"
    );
    assert_eq!(
        CONFIG
            .matches("pub(crate) fn production_on_fixed_tls_peer_and_selected_local_egress(")
            .count(),
        2,
        "status and public CLOB paired configs remain crate-private"
    );
    assert_eq!(
        CONFIG
            .matches("require_same_address_family(&fixed_tls_peer, &selected_local_egress)?;")
            .count(),
        8,
        "every production and loopback pair must reject mixed address families"
    );
    assert_eq!(
        CONFIG
            .matches("require_production_local_egress(&selected_local_egress)?;")
            .count(),
        8,
        "every selected-only and paired production configuration must reject evidence mode"
    );
    assert_eq!(
        CONFIG
            .matches("require_loopback_local_egress(&selected_local_egress)?;")
            .count(),
        8,
        "every selected-only and paired loopback configuration must reject production mode"
    );
    for required in [
        ".require_production()",
        ".require_loopback_evidence()",
        "production HTTP requires a production local-egress selection",
        "loopback HTTP evidence requires a loopback local-egress selection",
    ] {
        assert!(
            CONFIG.contains(required),
            "missing mode boundary: {required}"
        );
    }

    for transport in [HTTP_TRANSPORT, PRIVATE_HTTP] {
        let production = production_prefix(transport);
        assert_eq!(production.matches(".interface(").count(), 1);
        assert_eq!(production.matches(".local_address(").count(), 1);
        assert_eq!(production.matches("builder.resolve(").count(), 1);
        assert_eq!(production.matches("response.remote_addr()").count(), 1);
        assert_eq!(production.matches(".redirect(Policy::none())").count(), 1);
        assert_eq!(
            production
                .matches(".retry(reqwest::retry::never())")
                .count(),
            1
        );
        assert_eq!(production.matches(".no_proxy()").count(), 1);
        assert!(production.contains("validate_response_peer(self.expected_peer"));
        assert!(
            production.contains("fixed TLS peer requires an inseparable selected local egress")
        );
        let peer_check = production
            .find("validate_response_peer(self.expected_peer, response.remote_addr())?")
            .expect("fixed peer response validation");
        let status_read = production[peer_check..]
            .find("let status = response.status();")
            .map(|offset| peer_check + offset)
            .expect("status read after peer validation");
        assert!(peer_check < status_read);
        assert!(production.contains("#[cfg(target_os = \"linux\")]"));
        assert!(production.contains("selected local egress requires Linux"));
        for escape in [
            "pub fn client(",
            "pub fn request(",
            "pub fn execute(",
            "pub fn post(",
            "pub fn delete(",
        ] {
            assert!(
                !production.contains(escape),
                "selected transport escape: {escape}"
            );
        }
    }
    for role in [CLOB_HEALTH_HTTP, STATUS_ANNOUNCEMENT_HTTP] {
        assert!(role.contains("pub fn production_on_selected_local_egress("));
        assert!(role.contains("pub fn production_on_fixed_tls_peer_and_selected_local_egress("));
        assert!(
            role.contains("selection remains non-authoritative")
                || role.contains("selection is configuration only")
        );
    }
    assert!(
        GEOBLOCK_HTTP.contains("pub fn production_on_fixed_tls_peer_and_selected_local_egress(")
    );
    let full_public_connectivity = between(
        PUBLIC_CONNECTIVITY,
        "pub struct PmPublicConnectivityOwner",
        "// BEGIN OBSERVATION_ONLY_PUBLIC_CONNECTIVITY",
    );
    for combined_owner in [
        full_public_connectivity,
        production_prefix(READ_ONLY_PRIVATE),
    ] {
        assert!(
            !combined_owner.contains("production_on_selected_local_egress"),
            "an owner containing a generic WebSocket must not claim selected egress"
        );
        assert!(
            !combined_owner.contains("PmFixedTlsPeerSelection"),
            "an owner containing a generic WebSocket must not claim a fixed HTTP peer"
        );
    }
    let external_fixed_http = between(
        READ_AUTHORITY,
        "pub fn production_on_fixed_tls_peer_and_selected_local_egress<H, U>(",
        "/// Literal-loopback construction for synthetic protocol evidence only.",
    );
    assert!(
        external_fixed_http.contains(
            "PmPrivateHttpConfig::production_on_fixed_tls_peer_and_selected_local_egress("
        )
    );
    assert!(
        external_fixed_http.contains(
            "PmUserWsConfig::production(exact_order_scope.condition(), user_ws_bounds)?;"
        )
    );
    for forbidden in [
        "PmUserWsConfig::production_on_",
        "PmGeoblockHttpConfig",
        "PmPublicHttpConfig",
        "PmFixedPlace",
        "PmExactOwnedCancel",
    ] {
        assert!(
            !external_fixed_http.contains(forbidden),
            "purpose-closed external read constructor gained another selected role: {forbidden}"
        );
    }
    assert!(READ_AUTHORITY.contains(
        "fixed_peer_selected_http_preserves_external_read_authorities_without_selecting_ws"
    ));
    for (role, constructor_end) in [
        (GEOBLOCK_HTTP, "\n    pub async fn status("),
        (
            CLOB_HEALTH_HTTP,
            "\n    #[cfg(any(test, feature = \"read-only-evidence\"))]",
        ),
        (
            STATUS_ANNOUNCEMENT_HTTP,
            "\n    #[cfg(any(test, feature = \"read-only-evidence\"))]",
        ),
    ] {
        let constructor = between(
            production_prefix(role),
            "pub fn production_on_fixed_tls_peer_and_selected_local_egress(",
            constructor_end,
        );
        assert_eq!(
            constructor.matches("PmFixedTlsPeerSelection").count(),
            production_prefix(role)
                .matches("PmFixedTlsPeerSelection")
                .count(),
            "fixed peer type must remain confined to role construction, never an observation seal",
        );
        assert_eq!(
            constructor.matches("fixed_tls_peer").count() + 1,
            production_prefix(role).matches("fixed_tls_peer").count(),
            "fixed peer value must remain confined to role construction, never an observation seal",
        );
    }
    assert!(
        CONFIG
            .contains("pub(crate) fn production_on_selected_local_egress(\n        origin: &str,")
    );
    assert!(
        CONFIG.contains("pub(crate) fn production_on_fixed_tls_peer_and_selected_local_egress(")
    );
    assert!(!full_public_connectivity.contains("selected_local_egress"));
    for (consumer, purpose) in [
        (READ_AUTHORITY, "loopback external read owner"),
        (LOOPBACK_MUTATION_CREDENTIALS, "loopback mutation authority"),
    ] {
        assert!(consumer.contains(
            "http_config.mode() != OriginMode::LocalEvidence || user_ws_config.is_production()"
        ));
        assert!(consumer.contains(purpose));
    }
    assert!(!LIB.contains("pub use reap_polymarket_egress_binding"));
    for source in [
        PUBLIC_WS,
        USER_WS,
        PRIVATE_CREDENTIALS,
        LOOPBACK_MUTATION_CREDENTIALS,
    ] {
        assert!(!source.contains("PmFixedTlsPeerSelection"));
        assert!(!source.contains("expected_peer"));
        assert!(!source.contains("remote_addr()"));
    }
    for transport_test in [HTTP_TRANSPORT, PRIVATE_HTTP] {
        for required in [
            "TcpListener::bind(\"0.0.0.0:0\")",
            "\"127.0.0.2\"",
            "peer_ip",
            "\"missing0\"",
            "clob.polymarket.test",
            "127.0.0.3",
            "decoy.accept()",
            "validate_response_peer(Some(expected), None)",
        ] {
            assert!(
                transport_test.contains(required),
                "selected socket test does not prove {required}"
            );
        }
    }
}

#[test]
fn modules_are_separate_and_raw_transport_remains_private() {
    for module in [
        "mod config;",
        "mod clob_health_http;",
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
        "mod selected_ws;",
        "mod status_announcement_http;",
        "mod task_guard;",
        "mod account;",
        "mod user_ws;",
        "mod user_ws_config;",
        "mod ws_transport;",
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
fn production_geoblock_observation_is_move_only_origin_proved_and_non_authoritative() {
    let production = production_prefix(GEOBLOCK_HTTP);
    for required in [
        "pub struct PmProductionGeoblockObservation",
        "observation: PmGeoblockObservation",
        "_production_origin: ProductionGeoblockOrigin",
        "struct ProductionGeoblockOrigin;",
        "fn verify(mode: OriginMode) -> Result<Self, PmLiveAdapterError>",
        "OriginMode::Production => Ok(Self)",
        "OriginMode::LocalEvidence => Err(PmLiveAdapterError::InvalidConfiguration(",
        "pub async fn production_status_observation(",
        "let production_origin = ProductionGeoblockOrigin::verify(self.mode)?;",
        "PmProductionGeoblockObservation::from_source(",
        "impl std::fmt::Debug for PmProductionGeoblockObservation",
        "<production-origin; sealed>",
    ] {
        assert!(
            production.contains(required),
            "production geoblock proof lost `{required}`",
        );
    }
    assert!(LIB.contains("PmProductionGeoblockObservation,"));

    let declaration = between(
        production,
        "pub struct PmProductionGeoblockObservation {",
        "\n}\n\nimpl PmProductionGeoblockObservation",
    );
    assert_eq!(
        declaration.trim(),
        "observation: PmGeoblockObservation,",
        "production geoblock proof must retain its exact sole private field",
    );

    let method = between(
        production,
        "pub async fn production_status_observation(",
        "\n    #[must_use]\n    pub const fn production_order_entry_authorized",
    );
    let verify = method
        .find("ProductionGeoblockOrigin::verify(self.mode)?")
        .expect("private origin verification");
    let fetch = method
        .find("self.status_observation().await?")
        .expect("fixed geoblock fetch");
    let seal = method
        .find("PmProductionGeoblockObservation::from_source(")
        .expect("production observation seal");
    assert!(verify < fetch && fetch < seal);
    assert_eq!(
        production
            .matches("PmProductionGeoblockObservation::from_source(")
            .count(),
        1,
        "production observation must have one mode-proved construction path",
    );

    let declaration_attributes = production
        .split_once("pub struct PmProductionGeoblockObservation")
        .expect("production observation declaration")
        .0
        .rsplit("\n\n")
        .next()
        .expect("production observation attributes");
    for forbidden in ["Clone", "Copy", "Serialize", "Deserialize"] {
        assert!(!declaration_attributes.contains(forbidden));
        assert!(!production.contains(&format!("{forbidden} for PmProductionGeoblockObservation")));
    }
    for forbidden in [
        "impl From<PmGeoblockObservation> for PmProductionGeoblockObservation",
        "impl TryFrom<PmGeoblockObservation> for PmProductionGeoblockObservation",
        "pub fn from_source(",
        "pub fn into_observation(",
        "pub fn into_parts(",
        "Deref for PmProductionGeoblockObservation",
        "AsRef<PmGeoblockObservation> for PmProductionGeoblockObservation",
        "pub fn observation(&self)",
        "pub const fn observation(&self)",
        "production_order_entry_authorized: true",
    ] {
        assert!(
            !production.contains(forbidden),
            "production geoblock proof gained escape `{forbidden}`",
        );
    }
    for test_pin in [
        "local_evidence_cannot_issue_a_production_observation",
        "production_origin_proof_accepts_only_production_mode",
    ] {
        assert!(GEOBLOCK_HTTP.contains(test_pin));
    }
}

#[test]
fn clob_health_is_exact_bounded_origin_proved_and_liveness_only() {
    let production = production_prefix(CLOB_HEALTH_HTTP);
    for required in [
        "const EXACT_CLOB_LIVENESS_HEALTH_BODY: &[u8; 4] = b\"\\\"OK\\\"\";",
        r#"reap.pm.live-adapter.clob-liveness-health-observation.v1\0"#,
        "pub struct PmClobLivenessHealthHttpRole",
        "PmPublicHttpConfig::production(",
        "PM_CLOB_PRODUCTION_ORIGIN,",
        "PmPublicRoute::ClobHealth",
        "EXACT_CLOB_LIVENESS_HEALTH_BODY.len()",
        "if body.as_slice() != EXACT_CLOB_LIVENESS_HEALTH_BODY",
        "let receive_clock = self.clock.observe()?;",
        "pub struct PmProductionClobLivenessHealthObservation",
        "observation: PmClobLivenessHealthObservation",
        "struct ProductionClobHealthOrigin;",
        "let production_origin = ProductionClobHealthOrigin::verify(self.mode)?;",
        "self.liveness_health_observation().await?",
        "liveness/health evidence only",
        "not evidence about matching",
        "does not prove",
    ] {
        assert!(
            production.contains(required),
            "health role lost `{required}`"
        );
    }
    for required in [
        "PmClobLivenessHealthHttpRole",
        "PmClobLivenessHealthObservationCommitment",
        "PmProductionClobLivenessHealthObservation",
    ] {
        assert!(LIB.contains(required));
    }

    let wrapper = between(
        production,
        "pub struct PmProductionClobLivenessHealthObservation {",
        "\n}\n\nimpl PmProductionClobLivenessHealthObservation",
    );
    assert_eq!(
        wrapper.trim(),
        "observation: PmClobLivenessHealthObservation,",
    );
    let method = between(
        production,
        "pub async fn production_liveness_health_observation(",
        "\n    #[must_use]\n    pub const fn production_order_entry_authorized",
    );
    let verify = method
        .find("ProductionClobHealthOrigin::verify(self.mode)?")
        .unwrap();
    let fetch = method
        .find("self.liveness_health_observation().await?")
        .unwrap();
    let seal = method
        .find("PmProductionClobLivenessHealthObservation::from_source(")
        .unwrap();
    assert!(verify < fetch && fetch < seal);

    let wrapper_attributes = production
        .split_once("pub struct PmProductionClobLivenessHealthObservation")
        .unwrap()
        .0
        .rsplit("\n\n")
        .next()
        .unwrap();
    for forbidden in ["Clone", "Copy", "Serialize", "Deserialize"] {
        assert!(!wrapper_attributes.contains(forbidden));
        assert!(!production.contains(&format!(
            "{forbidden} for PmProductionClobLivenessHealthObservation"
        )));
    }
    for forbidden in [
        "impl From<PmClobLivenessHealthObservation>",
        "impl TryFrom<PmClobLivenessHealthObservation>",
        "Deref for PmProductionClobLivenessHealthObservation",
        "AsRef<PmClobLivenessHealthObservation>",
        "pub fn into_observation(",
        "pub fn raw_body(",
        "pub fn client(",
        "pub fn origin(",
        "pub fn path(",
        "pub fn retry(",
        "production_order_entry_authorized: true",
    ] {
        assert!(
            !production.contains(forbidden),
            "health escape: {forbidden}"
        );
    }
    for test_pin in [
        "unquoted_case_newline_whitespace_and_trailing_bytes_fail_closed",
        "declared_and_streamed_oversize_bodies_are_bounded",
        "redirects_non_success_and_timeout_fail_closed",
        "local_evidence_cannot_issue_production_proof_and_checks_before_io",
    ] {
        assert!(CLOB_HEALTH_HTTP.contains(test_pin));
    }
}

#[test]
fn status_cut_is_ordered_strict_complete_and_announcement_only() {
    let production = production_prefix(STATUS_ANNOUNCEMENT_HTTP);
    for required in [
        r#"reap.pm.live-adapter.status-announcement-observation.v1\0"#,
        "summary-object-v3+components-wrapper-object-v3/current-announcements-only",
        "pub const MAX_PM_STATUS_SUMMARY_BODY_BYTES: usize = 256 * 1024;",
        "pub const MAX_PM_STATUS_COMPONENTS_BODY_BYTES: usize = 512 * 1024;",
        "pub const MAX_PM_STATUS_ACTIVE_INCIDENTS: usize = 64;",
        "pub const MAX_PM_STATUS_ACTIVE_MAINTENANCES: usize = 64;",
        "pub const MAX_PM_STATUS_COMPONENTS: usize = 256;",
        "pub struct PmStatusAnnouncementHttpRole",
        "PmStatusHttpConfig::production(",
        "PmPublicRoute::StatusSummary",
        "let summary_receive_clock = self.clock.observe()?;",
        "let summary = parse_status_summary(&summary_body)?;",
        "PmPublicRoute::StatusComponents",
        "let components_receive_clock = self.clock.observe()?;",
        "let components = parse_status_components(&components_body)?;",
        "status_announcement_observation_commitment(",
        "encode_status_bytes(&mut digest, summary_body);",
        "encode_status_bytes(&mut digest, components_body);",
        "#[serde(default)]\n    active_incidents",
        "#[serde(default)]\n    active_maintenances",
        "struct RawStatusComponentsEnvelope",
        "components: Vec<RawStatusComponent>",
        "#[serde(deny_unknown_fields)]",
        "announcement evidence only",
        "excludes components",
        "historical notices",
        "does not prove the egress",
    ] {
        assert!(
            production.contains(required),
            "status role lost `{required}`"
        );
    }

    let summary_fetch = production.find("PmPublicRoute::StatusSummary").unwrap();
    let summary_clock = production
        .find("let summary_receive_clock = self.clock.observe()?;")
        .unwrap();
    let component_fetch = production.find("PmPublicRoute::StatusComponents").unwrap();
    let component_clock = production
        .find("let components_receive_clock = self.clock.observe()?;")
        .unwrap();
    assert!(summary_fetch < summary_clock && summary_clock < component_fetch);
    assert!(component_fetch < component_clock);

    let component_fields = between(
        production,
        "struct RawStatusComponent {",
        "\n}\n\n#[derive(Deserialize)]\n#[serde(deny_unknown_fields)]\nstruct RawStatusComponentGroup",
    );
    for exact in [
        "id: String,",
        "name: String,",
        "description: String,",
        "status: RawStatusComponentState,",
        "group: RawStatusComponentGroupField,",
    ] {
        assert!(component_fields.contains(exact));
    }
    for invented in [
        "active_incidents",
        "active_maintenances",
        "is_parent",
        "children",
    ] {
        assert!(!component_fields.contains(invented));
    }
    assert!(production.contains("currently shows a stale bare-array shape"));
    assert!(production.contains("parser intentionally rejects that alternate shape"));

    for forbidden in [
        "pub fn raw_body(",
        "pub fn summary_body(",
        "pub fn components_body(",
        "pub fn client(",
        "pub fn origin(",
        "pub fn path(",
        "pub fn retry(",
        "L2Credentials",
        "PrivateKey",
        "authenticate_",
        "place_order",
        "cancel_order",
        "production_order_entry_authorized: true",
    ] {
        assert!(
            !production.contains(forbidden),
            "status escape: {forbidden}"
        );
    }
    for test_pin in [
        "fixed_ordered_cut_retains_all_typed_current_announcement_rows",
        "omitted_empty_summary_issue_arrays_match_current_production_shape",
        "stale_documentation_bare_array_shape_and_unreviewed_issue_fields_are_rejected",
        "summary_unknown_duplicate_missing_type_and_enum_drift_fail_closed",
        "active_issue_rows_reject_missing_unknown_duplicate_ids_and_bad_scalars",
        "component_rows_reject_unknown_duplicate_missing_group_and_inconsistent_parent",
        "summary_and_components_declared_body_bounds_fail_before_parse",
        "streamed_body_bound_is_enforced_without_content_length",
        "local_evidence_cannot_issue_production_proof_and_checks_before_io",
    ] {
        assert!(STATUS_ANNOUNCEMENT_HTTP.contains(test_pin));
    }
}

#[test]
fn production_status_wrapper_is_move_only_origin_proved_and_redacted() {
    let production = production_prefix(STATUS_ANNOUNCEMENT_HTTP);
    let wrapper = between(
        production,
        "pub struct PmProductionStatusAnnouncementObservation {",
        "\n}\n\nimpl PmProductionStatusAnnouncementObservation",
    );
    assert_eq!(
        wrapper.trim(),
        "observation: PmStatusAnnouncementObservation,",
    );
    for required in [
        "struct ProductionStatusOrigin;",
        "OriginMode::Production => Ok(Self)",
        "OriginMode::LocalEvidence => Err(PmLiveAdapterError::InvalidConfiguration(",
        "pub async fn production_ordered_announcement_observation(",
        "let production_origin = ProductionStatusOrigin::verify(self.mode)?;",
        "self.ordered_announcement_observation().await?",
        "PmProductionStatusAnnouncementObservation::from_source(",
        "PmProductionStatusAnnouncementObservation(<production-origin; announcement-only; sealed>)",
    ] {
        assert!(production.contains(required));
    }
    let method = between(
        production,
        "pub async fn production_ordered_announcement_observation(",
        "\n    #[must_use]\n    pub const fn production_order_entry_authorized",
    );
    let verify = method
        .find("ProductionStatusOrigin::verify(self.mode)?")
        .unwrap();
    let fetch = method
        .find("self.ordered_announcement_observation().await?")
        .unwrap();
    let seal = method
        .find("PmProductionStatusAnnouncementObservation::from_source(")
        .unwrap();
    assert!(verify < fetch && fetch < seal);

    let wrapper_attributes = production
        .split_once("pub struct PmProductionStatusAnnouncementObservation")
        .unwrap()
        .0
        .rsplit("\n\n")
        .next()
        .unwrap();
    for forbidden in ["Clone", "Copy", "Serialize", "Deserialize"] {
        assert!(!wrapper_attributes.contains(forbidden));
        assert!(!production.contains(&format!(
            "{forbidden} for PmProductionStatusAnnouncementObservation"
        )));
    }
    for forbidden in [
        "impl From<PmStatusAnnouncementObservation>",
        "impl TryFrom<PmStatusAnnouncementObservation>",
        "Deref for PmProductionStatusAnnouncementObservation",
        "AsRef<PmStatusAnnouncementObservation>",
        "pub fn into_observation(",
        "pub fn into_parts(",
        "pub fn observation(&self)",
        "pub const fn observation(&self)",
    ] {
        assert!(!production.contains(forbidden));
    }
    assert!(LIB.contains("PmProductionStatusAnnouncementObservation"));
}

#[test]
fn public_metadata_authority_bridge_is_one_fetch_same_bytes_and_move_only() {
    let bridge_start = METADATA_HTTP
        .find("pub async fn refresh_authoritative_observation(")
        .unwrap();
    let bridge_end = METADATA_HTTP[bridge_start..]
        .find("\n    fn seal_pair(")
        .map(|offset| bridge_start + offset)
        .unwrap();
    let bridge = &METADATA_HTTP[bridge_start..bridge_end];

    assert_eq!(bridge.matches("fetch_pair(").count(), 1);
    assert_eq!(bridge.matches("self.fetch_pair().await?").count(), 1);
    assert_eq!(bridge.matches("self.clock.observe()?").count(), 1);
    assert_eq!(bridge.matches("&market_bytes").count(), 2);
    assert_eq!(bridge.matches("&clob_v2_bytes").count(), 2);
    for required in [
        "if metadata_scope(expected) != self.scope",
        "let (market_bytes, clob_v2_bytes) = self.fetch_pair().await?;",
        "let receive_clock = self.clock.observe()?;",
        "self.seal_pair(&market_bytes, &clob_v2_bytes, receive_clock)?",
        "live_observation.receive_clock().monotonic_receive_ns()",
        "PmAuthoritativeMetadata::join_live_clob_v2_raw(",
        "&market_bytes,",
        "&clob_v2_bytes,",
        "PmLiveAuthoritativeMetadataObservation::from_source(",
    ] {
        assert!(
            bridge.contains(required),
            "missing one-fetch metadata authority bridge invariant: {required}"
        );
    }
    assert!(
        bridge.find("metadata_scope(expected)").unwrap()
            < bridge.find("self.fetch_pair().await?").unwrap()
    );
    for forbidden in [
        "origin:",
        "PmPublicHttpConfig",
        "market_bytes.to_vec()",
        "clob_v2_bytes.to_vec()",
        "market_bytes.clone()",
        "clob_v2_bytes.clone()",
    ] {
        assert!(
            !bridge.contains(forbidden),
            "metadata authority bridge widened its source seam: {forbidden}"
        );
    }

    for required in [
        "Result<(Zeroizing<Vec<u8>>, Zeroizing<Vec<u8>>), PmLiveAdapterError>",
        "let market_bytes = Zeroizing::new(",
        "let clob_v2_bytes = Zeroizing::new(",
        "pub struct PmLiveAuthoritativeMetadataObservation",
        "live_observation: PmLiveMetadataObservation",
        "authoritative_metadata: PmAuthoritativeMetadata",
        "fn from_source(",
        "pub fn into_parts(self)",
    ] {
        assert!(
            METADATA_HTTP.contains(required),
            "missing sealed/zeroizing metadata bridge invariant: {required}"
        );
    }

    let carrier_start = METADATA_HTTP
        .find("pub struct PmLiveAuthoritativeMetadataObservation")
        .unwrap();
    let carrier_end = METADATA_HTTP[carrier_start..]
        .find("pub enum PmLiveAuthoritativeMetadataError")
        .map(|offset| carrier_start + offset)
        .unwrap();
    let carrier = &METADATA_HTTP[carrier_start..carrier_end];
    for forbidden in [
        "#[derive(Debug, Clone",
        "market_bytes:",
        "clob_v2_bytes:",
        "raw_body:",
        "pub fn from_source(",
        "pub fn market_bytes(",
        "pub fn clob_v2_bytes(",
    ] {
        assert!(
            !carrier.contains(forbidden),
            "metadata authority carrier widened provenance: {forbidden}"
        );
    }

    for preserved in [
        "pub async fn refresh<S>(",
        "pub async fn refresh_typed(&self)",
        "pub async fn refresh_typed_observation(",
    ] {
        assert!(
            METADATA_HTTP.contains(preserved),
            "existing metadata API was not preserved: {preserved}"
        );
    }
}

#[test]
fn server_time_and_type_one_account_observations_are_sealed_and_source_clocked() {
    let source = [PUBLIC_HTTP, PRIVATE_HTTP, ACCOUNT].join("\n");
    for required in [
        "pub struct PmReadServerTimeObservation",
        "pub struct PmPlaceServerTimeObservation",
        "pub struct PmCancelServerTimeObservation",
        "pub async fn fresh_read_server_time_observation(",
        "pub async fn fresh_place_time_observation(",
        "pub async fn fresh_cancel_time_observation(",
        "pub fn into_read_server_time(self) -> PmReadServerTime",
        "pub fn into_proof(self) -> PmPlaceMutationTimeProof",
        "pub fn into_proof(self) -> PmCancelMutationTimeProof",
        "READ_SERVER_TIME_OBSERVATION_COMMITMENT_DOMAIN",
        "PLACE_SERVER_TIME_OBSERVATION_COMMITMENT_DOMAIN",
        "CANCEL_SERVER_TIME_OBSERVATION_COMMITMENT_DOMAIN",
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
    for forbidden in [
        "PmFixedTlsPeerSelection",
        "fixed_tls_peer",
        "expected_peer",
        "PmLocalEgressSelection",
    ] {
        assert!(
            !closed_observe.contains(forbidden),
            "transport selection became closed-only sealing authority: {forbidden}"
        );
        assert!(
            !RECONCILIATION.contains(forbidden),
            "transport selection became reconciliation sealing authority: {forbidden}"
        );
    }
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
    assert!(
        MANIFEST.contains(
            "read-only-evidence = [\"reap-polymarket-egress-binding/loopback-evidence\"]"
        )
    );
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
        assert!(source.contains("worker\n        .join()\n        .await"));
        assert!(!source.contains("let worker = tokio::spawn"));
    }
    assert!(TASK_GUARD.contains("impl<T> Drop for AbortOnDropTask<T>"));
    assert!(TASK_GUARD.contains("task.abort()"));
    assert!(TASK_GUARD.contains("pub(crate) async fn join(mut self)"));
}

#[test]
fn websocket_dial_strategy_is_private_default_preserving_and_production_selected() {
    assert!(LIB.contains("mod ws_transport;"));
    assert!(!LIB.contains("pub mod ws_transport;"));
    assert!(!LIB.contains("pub use ws_transport"));
    assert!(!LIB.contains("PmWsDialStrategy"));
    assert!(!LIB.contains("PmWsSocket"));
    for required in [
        "pub(crate) trait PmWsDialStrategy",
        "pub(crate) struct PmDefaultWsDialer",
        "connect_async_with_config(request.endpoint, Some(request.websocket_config), true)",
        "pub(crate) struct PmProductionSelectedWsDialer",
        "PmWsDialFailure::RetryableConnect",
        "PmWsDialFailure::TerminalInvariant",
        "pub(crate) struct PmTestSelectedLoopbackWsDialer",
        "thread_confinement: Rc<()>",
    ] {
        assert!(
            WS_TRANSPORT.contains(required),
            "missing private WebSocket dial invariant: {required}"
        );
    }
    assert_eq!(
        WS_TRANSPORT.matches("connect_async_with_config(").count(),
        1
    );

    let production_dial = between(
        WS_TRANSPORT,
        "async fn dial_production_selected(",
        "\nfn exact_endpoint(",
    );
    for required in [
        "request.route != binding.route()",
        "request.endpoint != exact_endpoint(binding.route())",
        "!binding.revalidate_process_and_thread()",
        "fixed_peer.require_production().is_err()",
        "local_egress.require_production().is_err()",
        ".require_same_address_family(local_egress)",
        "TcpSocket::new_v4()",
        "TcpSocket::new_v6()",
        ".bind_device(Some(interface_name))",
        ".device()",
        ".bind(SocketAddr::new(local_source_ip, 0))",
        "socket.connect(peer_addr)",
        "client_async_tls_with_config(",
        "MaybeTlsStream::Rustls(stream)",
        "PmSelectedWsSocketFacts::from_verified_socket(",
    ] {
        assert!(
            production_dial.contains(required),
            "production selected dial lost `{required}`"
        );
    }
    assert_eq!(
        production_dial.matches("socket.connect(peer_addr)").count(),
        1
    );
    assert_eq!(
        production_dial
            .matches("validate_connected_stream(")
            .count(),
        2
    );
    let pre_handshake_check = production_dial.find("validate_connected_stream(").unwrap();
    let tls_upgrade = production_dial
        .find("client_async_tls_with_config(")
        .unwrap();
    let post_handshake_check = production_dial.rfind("validate_connected_stream(").unwrap();
    let facts = production_dial
        .find("PmSelectedWsSocketFacts::from_verified_socket(")
        .unwrap();
    assert!(pre_handshake_check < tls_upgrade);
    assert!(tls_upgrade < post_handshake_check && post_handshake_check < facts);
    for forbidden in [
        "lookup_host",
        "ToSocketAddrs",
        "connect_async(",
        "connect_async_with_config(",
        "loop {",
        "while let",
        "for peer",
        "fallback",
    ] {
        assert!(
            !production_dial.contains(forbidden),
            "production selected dial gained hidden resolution/retry path: {forbidden}"
        );
    }

    let production_stream_validation = between(
        WS_TRANSPORT,
        "fn validate_connected_stream(",
        "\nfn retryable_exact_peer_connect_error(",
    );
    for required in [
        "!binding.revalidate_process_and_thread()",
        ".local_addr()",
        ".peer_addr()",
        ".nodelay()",
        "SockRef::from(stream)",
        ".device()",
        "local_addr.ip() != expected_local_ip",
        "peer_addr != expected_peer_addr",
        "readback_device.as_slice() != expected_interface_name",
    ] {
        assert!(
            production_stream_validation.contains(required),
            "production selected socket recheck lost `{required}`"
        );
    }

    for (source, selected_classifier, classifier_end) in [
        (
            PUBLIC_WS,
            "const fn selected_public_retirement_is_terminal(",
            "\n}\n\nasync fn emit(",
        ),
        (
            USER_WS,
            "const fn selected_user_retirement_is_terminal(",
            "\n}\n\n#[cfg(test)]",
        ),
    ] {
        let attempt = between(
            source,
            "async fn run_attempt<D>(",
            "\nasync fn run_connected(",
        );
        let timeout_branch = between(
            attempt,
            "Err(_) =>",
            "Ok(Err(PmWsDialFailure::RetryableConnect))",
        );
        let retryable_branch = between(
            attempt,
            "Ok(Err(PmWsDialFailure::RetryableConnect))",
            "Ok(Err(PmWsDialFailure::TerminalInvariant))",
        );
        let terminal_branch = between(
            attempt,
            "Ok(Err(PmWsDialFailure::TerminalInvariant))",
            "Ok(Ok(outcome))",
        );
        assert!(timeout_branch.contains("ConnectTimeout"));
        assert!(timeout_branch.contains("retired("));
        assert!(!timeout_branch.contains("terminal_retired("));
        assert!(retryable_branch.contains("ConnectFailed"));
        assert!(retryable_branch.contains("retired("));
        assert!(!retryable_branch.contains("terminal_retired("));
        assert!(terminal_branch.contains("ConnectFailed"));
        assert!(terminal_branch.contains("terminal_retired("));
        assert!(
            attempt.contains("let (socket, selected_socket_facts) = dial_outcome.into_parts()")
        );

        let terminal_protocol_reasons = between(source, selected_classifier, classifier_end);
        for retryable in ["ConnectTimeout", "ConnectFailed", "SocketClosed"] {
            assert!(!terminal_protocol_reasons.contains(retryable));
        }
    }

    assert!(!PUBLIC_WS.contains("connect_async_with_config("));
    assert!(!USER_WS.contains("connect_async_with_config("));
    for (source, run_start, selected_start, test_impl) in [
        (
            PUBLIC_WS,
            "    pub async fn run<S>(\n        self,\n        shutdown: PmPublicWsShutdownSignal,",
            "\n}\n\n/// Public market WebSocket role fixed",
            "#[cfg(test)]\nimpl PmPublicMarketWsRole",
        ),
        (
            USER_WS,
            "    pub async fn run<S>(\n        self,\n        shutdown: PmUserWsShutdownSignal,",
            "\n}\n\n/// Authenticated user WebSocket role fixed",
            "#[cfg(test)]\nimpl PmAuthenticatedUserWsRole",
        ),
    ] {
        let default_run = between(source, run_start, selected_start);
        assert!(default_run.contains("AbortOnDropTask::new(tokio::spawn"));
        assert!(default_run.contains("PmDefaultWsDialer"));
        assert!(!default_run.contains("spawn_local"));
        assert!(!default_run.contains("PmProductionSelectedWsDialer"));
        assert!(!default_run.contains("serve_inline_worker_events"));
        assert!(source.contains("AbortOnDropTask::new(tokio::spawn"));
        assert!(source.contains("PmDefaultWsDialer"));
        assert!(source.contains(test_impl));
        assert!(source.contains("async fn run_with_test_selected_loopback"));
        assert!(source.contains("serve_inline_worker_events(worker"));
        assert!(!source.contains("spawn_local(run_worker("));
    }
    for (source, selected_impl_start, selected_impl_end, test_impl_start, test_impl_end) in [
        (
            PUBLIC_WS,
            "impl PmProductionSelectedPublicWsRole {",
            "\n}\n\nimpl fmt::Debug for PmProductionSelectedPublicWsRole",
            "#[cfg(test)]\nimpl PmPublicMarketWsRole {",
            "\n}\n\nfn observe(",
        ),
        (
            USER_WS,
            "impl PmProductionSelectedUserWsRole {",
            "\n}\n\nimpl fmt::Debug for PmProductionSelectedUserWsRole",
            "#[cfg(test)]\nimpl PmAuthenticatedUserWsRole {",
            "\n}\n\nasync fn emit(",
        ),
    ] {
        let selected_impl = between(source, selected_impl_start, selected_impl_end);
        let selected_test_impl = between(source, test_impl_start, test_impl_end);
        for selected_method in [selected_impl, selected_test_impl] {
            assert!(selected_method.contains("serve_inline_worker_events(worker"));
            assert!(!selected_method.contains("spawn_local"));
            assert!(!selected_method.contains("tokio::spawn"));
        }
    }
    let public_inline_pump = between(
        PUBLIC_WS,
        "async fn serve_inline_worker_events<F, S>(",
        "\nasync fn deliver_inline_worker_event<S>(",
    );
    let user_inline_pump = between(
        USER_WS,
        "async fn serve_inline_worker_events<F, S>(",
        "\nasync fn serve_worker_events<S>(",
    );
    for (pump, helper) in [
        (
            public_inline_pump,
            "deliver_inline_worker_event_while_polling(",
        ),
        (user_inline_pump, "deliver_inline_user_event_while_polling("),
    ] {
        for required in [
            helper,
            "worker.as_mut()",
            "delivery.as_mut()",
            "worker_result = worker.as_mut()",
            "InlineWorkerCompletion::Completed(worker_result)",
            "while let Some(event)",
            "return result.map_err(",
        ] {
            assert!(pump.contains(required), "inline pump lost `{required}`");
        }
        assert!(!pump.contains("tokio::spawn"));
        assert!(!pump.contains("spawn_local"));
    }
    assert!(public_inline_pump.contains("delivery.await?"));
    assert!(user_inline_pump.contains("delivery.await.map_err(PmUserWsRunError::Sink)?"));
    let public_inline_admission = between(
        PUBLIC_WS,
        "async fn deliver_inline_worker_event<S>(",
        "\nasync fn serve_worker_events<S>(",
    );
    let public_reconnect_admission = between(
        public_inline_admission,
        "WorkerEvent::ReconnectAuthority { retired, response } => {",
        "\n        }\n    }",
    );
    assert!(public_reconnect_admission.contains("authorize_public_ws_reconnect(retired)"));
    assert!(public_reconnect_admission.contains("response"));
    assert!(public_reconnect_admission.contains(".send(directive)"));
    assert!(
        WS_TRANSPORT.contains("#[cfg(test)]\npub(crate) struct PmTestSelectedLoopbackWsDialer")
    );
    assert!(!WS_TRANSPORT.contains("pub struct PmTestSelectedLoopbackWsDialer"));
    for forbidden in [
        "pub fn socket(",
        "pub fn client(",
        "pub fn endpoint(",
        "pub fn send(",
        "AuthenticatedPlaceRequest",
        "PmRetainedPlaceRequest",
        "authenticate_place",
        "authenticate_owned_cancel",
        "POST /order",
        "DELETE /order",
    ] {
        assert!(
            !WS_TRANSPORT.contains(forbidden),
            "private WebSocket dial seam gained forbidden production/capability surface: {forbidden}"
        );
    }
    assert!(
        PUBLIC_WS
            .contains("selected_loopback_dialer_preserves_public_worker_protocol_across_reconnect")
    );
    assert!(
        USER_WS.contains("selected_loopback_dialer_preserves_user_worker_protocol_on_local_set")
    );

    let public_selected_socket_test = between(
        PUBLIC_WS,
        "async fn selected_loopback_dialer_preserves_public_worker_protocol_across_reconnect()",
        "async fn selected_inline_public_sink_keeps_worker_live_and_drains_after_completion()",
    );
    let user_selected_socket_test = between(
        USER_WS,
        "async fn selected_loopback_dialer_preserves_user_worker_protocol_on_local_set()",
        "async fn selected_inline_user_sink_keeps_worker_live_and_drains_after_completion()",
    );
    for test in [public_selected_socket_test, user_selected_socket_test] {
        for required in [
            "TcpListener::bind(\"127.0.0.1:0\")",
            "\"127.0.0.2\".parse()",
            "let decoy_ip: std::net::IpAddr = \"127.0.0.3\".parse()",
            "TcpListener::bind(format!(\"127.0.0.3:{}\"",
            "assert_eq!(address.ip(), exact_peer_ip)",
            "assert_eq!(decoy.local_addr().unwrap().ip(), decoy_ip)",
            "assert_eq!(accepted_peer.ip(), selected_source)",
            "decoy.accept()",
            ".is_err()",
            "facts.peer_addr(), address",
            "facts.local_addr().ip(), selected_source",
        ] {
            assert!(
                test.contains(required),
                "selected loopback socket test lost `{required}`"
            );
        }
    }

    let public_inline_regression = between(
        PUBLIC_WS,
        "async fn selected_inline_public_sink_keeps_worker_live_and_drains_after_completion()",
        "async fn selected_public_backoff_shutdown_retains_retired_epoch_facts()",
    );
    for required in [
        "BlockingSelectedInlineSink",
        "Duration::from_secs(30)",
        "Duration::from_secs(10)",
        "allow_worker_progress.notify_one()",
        "worker_completed.notified()",
        "activity.generation() >= 4",
        "timeout(Duration::from_millis(50), &mut task)",
        "PmPublicWsTransportError::EventChannelSaturated",
        "[\"opened\", \"subscription\", \"raw\"]",
    ] {
        assert!(public_inline_regression.contains(required));
    }
    assert!(
        public_inline_regression
            .find("worker_completed.notified()")
            .unwrap()
            < public_inline_regression
                .find("release.notify_one()")
                .unwrap()
    );
    let user_inline_regression = between(
        USER_WS,
        "async fn selected_inline_user_sink_keeps_worker_live_and_drains_after_completion()",
        "async fn selected_user_backoff_shutdown_retains_retired_epoch_facts()",
    );
    for required in [
        "BlockingSelectedInlineSink",
        "Duration::from_secs(30)",
        "Duration::from_secs(10)",
        "allow_worker_progress.notify_one()",
        "worker_completed.notified()",
        "activity.high_water() >= 4",
        "timeout(Duration::from_millis(50), &mut task)",
        "PmUserWsTransportError::EventChannelSaturated",
        "[\"opened\", \"subscription\", \"bound\"]",
    ] {
        assert!(user_inline_regression.contains(required));
    }
    assert!(
        user_inline_regression
            .find("worker_completed.notified()")
            .unwrap()
            < user_inline_regression.find("release.notify_one()").unwrap()
    );

    let public_backoff = between(
        PUBLIC_WS,
        "async fn selected_public_backoff_shutdown_retains_retired_epoch_facts()",
        "async fn selected_binding_failure_is_terminal_before_public_reconnect_authority()",
    );
    for required in [
        "Seen::Shutdown(121)",
        "assert_eq!(facts.len(), 5)",
        "*epoch == 121",
        "*observed == Some(exact)",
    ] {
        assert!(public_backoff.contains(required));
    }
    let user_backoff = between(
        USER_WS,
        "async fn selected_user_backoff_shutdown_retains_retired_epoch_facts()",
        "async fn selected_binding_failure_is_terminal_before_user_auto_retry()",
    );
    for required in [
        "Seen::Shutdown(5)",
        "assert_eq!(facts.len(), 5)",
        "*epoch == 5",
        "*observed == Some(exact)",
    ] {
        assert!(user_backoff.contains(required));
    }
    let public_terminal = between(
        PUBLIC_WS,
        "async fn selected_binding_failure_is_terminal_before_public_reconnect_authority()",
        "async fn selected_exact_peer_refusal_remains_publicly_authorized_retryable()",
    );
    for required in [
        "\"missing0\"",
        "PmPublicWsTransportError::WorkerFailed",
        "assert_eq!(sink.reconnect_authority_calls, 0)",
        "TryRecvError::Empty",
        "[(101, None), (101, None)]",
        "listener.accept()",
    ] {
        assert!(public_terminal.contains(required));
    }
    let user_terminal = between(
        USER_WS,
        "async fn selected_binding_failure_is_terminal_before_user_auto_retry()",
        "async fn selected_exact_peer_refusal_reaches_bounded_user_retry_exhaustion()",
    );
    for required in [
        "\"missing0\"",
        "PmUserWsTransportError::WorkerFailed",
        "TryRecvError::Empty",
        "[(5, None)]",
        "listener.accept()",
    ] {
        assert!(user_terminal.contains(required));
    }
    assert!(PUBLIC_WS.contains("retired.connection(),\n                        clock.as_mut(),"));
    assert!(
        USER_WS
            .contains("retired.observation().connection(),\n                    clock.as_mut(),")
    );
    assert!(
        PUBLIC_WS.contains("selected_exact_peer_refusal_remains_publicly_authorized_retryable")
    );
    assert!(USER_WS.contains("selected_exact_peer_refusal_reaches_bounded_user_retry_exhaustion"));

    for required in [
        "pub struct PmProductionSelectedWsOwner",
        "pub fn into_roles(",
        "Rc<PmProductionSelectedWsBundleIdentity>",
        "public.scope().condition() != user.condition()",
        "fixed_tls_peer.dns_name() != PRODUCTION_WS_DNS_NAME",
        "pub struct PmSelectedWsSocketFacts",
        "interface_name: [u8; LINUX_INTERFACE_NAME_MAX_BYTES]",
        "interface_name_len: u8",
        "pub(crate) fn from_verified_socket(",
        "pub fn interface_name(&self) -> &str",
        "pub const fn local_addr(self) -> SocketAddr",
        "pub const fn peer_addr(self) -> SocketAddr",
        "device/local/peer redacted",
        "validated caller-provided fixed peer",
    ] {
        assert!(
            SELECTED_WS.contains(required),
            "missing selected-WebSocket owner/fact invariant: {required}"
        );
    }
    for source in [SELECTED_WS, PUBLIC_WS, USER_WS] {
        assert!(source.contains("production_order_entry_authorized"));
        assert!(source.contains("false"));
    }
    for source in [PUBLIC_WS, USER_WS] {
        assert!(source.contains("selected_socket_facts: Option<PmSelectedWsSocketFacts>"));
        assert!(source.contains("pub const fn selected_socket_facts("));
        assert!(source.contains("PmProductionSelectedWsDialer::new(binding)"));
        assert!(source.contains("serve_inline_worker_events(worker"));
    }
    let facts_fields = between(
        SELECTED_WS,
        "pub struct PmSelectedWsSocketFacts {",
        "\n}\n\nimpl PmSelectedWsSocketFacts",
    );
    let owner_fields = between(
        SELECTED_WS,
        "pub struct PmProductionSelectedWsOwner {",
        "\n}\n\nimpl PmProductionSelectedWsOwner",
    );
    assert!(!facts_fields.contains("pub "));
    assert!(!owner_fields.contains("pub "));
    assert!(!SELECTED_WS.contains("reviewed fixed peer"));
    for source in [SELECTED_WS, PUBLIC_WS, USER_WS] {
        assert!(!source.contains("unsafe impl Send"));
        assert!(!source.contains("unsafe impl Sync"));
    }
    for forbidden in [
        "pub fn socket(",
        "pub fn endpoint(",
        "pub fn dial(",
        "CanonicalOnline",
        "controlled_trial",
        "Hmac",
        "seal",
    ] {
        assert!(
            !SELECTED_WS.contains(forbidden),
            "selected-WebSocket public owner gained forbidden surface: {forbidden}"
        );
    }
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
        "request_reconnect_authority(&activity, &events, retired).await?",
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
fn public_websocket_activity_watermark_is_checked_source_issued_and_read_only() {
    for required in [
        "pub struct PmPublicWsActivityView",
        "pub fn activity_view(&self) -> PmPublicWsActivityView",
        ".fetch_update(",
        "|current| current.checked_add(1)",
        "ActivityGenerationOverflow",
        "source_observation(",
        "event.activity_generation()",
    ] {
        assert!(
            PUBLIC_WS.contains(required),
            "missing public-WS activity watermark invariant: {required}"
        );
    }
    assert!(LIB.contains("PmPublicWsActivityView"));

    let issuer_start = PUBLIC_WS.find("struct PmPublicWsActivitySource").unwrap();
    let issuer_end = PUBLIC_WS[issuer_start..]
        .find("struct SystemPublicWsClock")
        .map(|offset| issuer_start + offset)
        .unwrap();
    let issuer = &PUBLIC_WS[issuer_start..issuer_end];
    assert!(!issuer.contains(".fetch_add("));
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
        "Arc::ptr_eq(expected, &proof.authority)",
        "Arc::ptr_eq(&expected.domain, &proof.authority.domain)",
        "PM_MUTATION_SERVER_TIME_MAX_AGE",
        "pub struct PmPlaceMutationTimeProof",
        "pub struct PmCancelMutationTimeProof",
        "pub struct PmPlaceMutationTimeFinalizer",
        "pub struct PmCancelMutationTimeFinalizer",
        "pub enum PmPlaceMutationAuthenticationError",
        "pub enum PmCancelMutationAuthenticationError",
        "pub struct PmFinalPlaceMutationTime<'proof>",
        "pub struct PmFinalCancelMutationTime<'proof>",
        "pub trait PmPlaceMutationTimeProvider",
        "pub trait PmCancelMutationTimeProvider",
        "pub struct PmReadServerTime",
        "validate_age(&domain, received)?",
        "validate_age(&expected.domain, proof.received)",
        "pub struct PmPublicConnectivityOwner",
        "place_mutation_time: PmPlaceMutationTimeOwner",
        "cancel_mutation_time: PmCancelMutationTimeOwner",
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
        "pub fn l2_timestamp",
        "pub const fn l2_timestamp",
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
fn observation_only_public_connectivity_has_no_write_authority_shape() {
    let clock = between(
        PRODUCT_CLOCK,
        "// BEGIN OBSERVATION_ONLY_CLOCK_SPLIT",
        "// END OBSERVATION_ONLY_CLOCK_SPLIT",
    );
    let connectivity = between(
        PUBLIC_CONNECTIVITY,
        "// BEGIN OBSERVATION_ONLY_PUBLIC_CONNECTIVITY",
        "// END OBSERVATION_ONLY_PUBLIC_CONNECTIVITY",
    );

    for required in [
        "pub(crate) fn split_observation_only(self) -> PmObservationClockViews",
        "pub(crate) struct PmObservationClockViews",
        "public_ws: PmPublicWsProductClock",
        "user_ws: PmUserWsProductClock",
        "public_http: PmPublicHttpProductClock",
        "read_server_time_http: PmReadServerTimeProductClock",
        "private_read: PmPrivateReadProductClock",
        "actor: PmActorProductClock",
        "okx: PmOkxProductClock",
    ] {
        assert!(
            clock.contains(required),
            "missing observation-only clock invariant: {required}"
        );
    }
    for required in [
        "pub struct PmPublicObservationConnectivityOwner",
        "pub struct PmPublicObservationConnectivityRoles",
        "PmPublicMetadataHttpRole",
        "PmPublicHttpRole",
        "PmReadServerTimeHttpRole",
        "PmPrivateReadProductClock",
        "PmPublicMarketWsRole",
        "PmUserWsProductClock",
        "PmActorProductClock",
        "PmOkxProductClock",
        "if parser_config.scope() != public_ws_config.scope()",
        "clock_owner.split_observation_only().into_views()",
        "pub fn production_on_fixed_tls_peer_and_selected_local_egress(",
        "if !public_ws_config.is_production()",
        "selected production observation connectivity requires a production public WebSocket configuration",
        "caller-provided fixed TLS peer",
        "Both values remain non-authoritative configuration.",
        "PmPublicHttpConfig::production_on_fixed_tls_peer_and_selected_local_egress(",
        "Self::new(",
        "PmPublicObservationConnectivityOwner(<observation-only; scope-and-clock-bound>)",
        "PmPublicObservationConnectivityRoles(<observation-only; move-only>)",
    ] {
        assert!(
            connectivity.contains(required),
            "missing observation-only connectivity invariant: {required}"
        );
    }
    let selected_constructor = between(
        connectivity,
        "pub fn production_on_fixed_tls_peer_and_selected_local_egress(",
        "    #[must_use]\n    pub const fn configured_scope",
    );
    let ws_mode_check = selected_constructor
        .find("if !public_ws_config.is_production()")
        .expect("selected observation owner must check the WebSocket mode");
    let fixed_http_construction = selected_constructor
        .find("PmPublicHttpConfig::production_on_fixed_tls_peer_and_selected_local_egress(")
        .expect("selected observation owner must construct fixed HTTP configuration");
    assert!(
        ws_mode_check < fixed_http_construction,
        "selected observation owner must reject WebSocket mode crossing before HTTP construction"
    );
    for required in [
        "PmPublicObservationConnectivityOwner",
        "PmPublicObservationConnectivityRoles",
    ] {
        assert!(
            LIB.contains(required),
            "observation-only connectivity is not exported: {required}"
        );
    }
    assert_eq!(
        clock
            .matches("pub(crate) fn split_observation_only(self) -> PmObservationClockViews")
            .count(),
        1,
        "observation clock owner must expose exactly one direct narrow split"
    );
    assert_eq!(
        connectivity
            .matches("clock_owner.split_observation_only().into_views()")
            .count(),
        1,
        "observation connectivity must consume exactly one direct narrow split"
    );

    for forbidden in ["place", "cancel", "mutation"] {
        assert!(
            !clock.to_ascii_lowercase().contains(forbidden),
            "observation-only clock split gained forbidden authority term: {forbidden}"
        );
        assert!(
            !connectivity.to_ascii_lowercase().contains(forbidden),
            "observation-only connectivity gained forbidden authority term: {forbidden}"
        );
    }
    for forbidden in ["PmProductClockViews", ".split()", "drop("] {
        assert!(
            !clock.contains(forbidden),
            "observation-only clock split delegated through the full path: {forbidden}"
        );
    }
    for forbidden in [
        "#[derive(Clone",
        "impl Clone for PmPublicObservation",
        "Serialize",
        "Deserialize",
        "impl From<",
        "impl Into<",
        "PmProductionSelectedWsOwner",
        "PmProductionSelectedPublicWsRole",
        "PmProductionSelectedUserWsRole",
        "PmPublicConnectivityOwner",
        "PmPublicConnectivityRoles",
        "PmProductClockViews",
        ".split()",
        ".into_roles().into_roles()",
        "drop(",
    ] {
        assert!(
            !connectivity.contains(forbidden),
            "observation-only connectivity gained an escape: {forbidden}"
        );
    }

    let full_connectivity = between(
        PUBLIC_CONNECTIVITY,
        "pub struct PmPublicConnectivityOwner",
        "// BEGIN OBSERVATION_ONLY_PUBLIC_CONNECTIVITY",
    );
    let full_clock = PRODUCT_CLOCK
        .split_once("// BEGIN OBSERVATION_ONLY_CLOCK_SPLIT")
        .map(|(source, _)| source)
        .expect("observation-only clock marker must follow the full split");
    for required in [
        "place_mutation_time: PmPlaceMutationTimeOwner",
        "cancel_mutation_time: PmCancelMutationTimeOwner",
        "PmPlaceMutationTimeOwner::with_product_clock(",
        "PmCancelMutationTimeOwner::with_product_clock(",
    ] {
        assert!(
            full_connectivity.contains(required),
            "existing full connectivity lost authority: {required}"
        );
    }
    for required in [
        "let place_time_authority = Arc::new(MutationTimeAuthority",
        "let cancel_time_authority = Arc::new(MutationTimeAuthority",
        "place_mutation_time_finalizer: PmPlaceMutationTimeFinalizer",
        "cancel_mutation_time_finalizer: PmCancelMutationTimeFinalizer",
    ] {
        assert!(
            full_clock.contains(required),
            "existing full clock split lost authority: {required}"
        );
    }
}

#[test]
fn deferred_mutation_clock_is_same_domain_opaque_and_allocated_only_on_selected_promotion() {
    let custody = between(
        PRODUCT_CLOCK,
        "// BEGIN DEFERRED_MUTATION_CLOCK_CUSTODY",
        "// END DEFERRED_MUTATION_CLOCK_CUSTODY",
    );
    let expansion = between(
        PRODUCT_CLOCK,
        "// BEGIN DEFERRED_MUTATION_CLOCK_EXPANSION",
        "// END DEFERRED_MUTATION_CLOCK_EXPANSION",
    );
    let capsule = between(
        DEFERRED_MUTATION_TIME,
        "// BEGIN SELECTED_DEFERRED_MUTATION_CLOCK_CAPSULE",
        "// END SELECTED_DEFERRED_MUTATION_CLOCK_CAPSULE",
    );
    let staging = between(
        DEFERRED_MUTATION_TIME,
        "// BEGIN DEFERRED_MUTATION_OBSERVATION_STAGING",
        "// END DEFERRED_MUTATION_OBSERVATION_STAGING",
    );
    let promotion = between(
        DEFERRED_MUTATION_TIME,
        "// BEGIN DEFERRED_MUTATION_SELECTED_PROMOTION",
        "// END DEFERRED_MUTATION_SELECTED_PROMOTION",
    );

    for required in [
        "pub(crate) struct PmDeferredMutationClockDomain",
        "domain: Arc<ProductClockDomain>",
        "pub(crate) fn split_observation_with_deferred_mutation(",
        "public_ws: PmPublicWsProductClock",
        "user_ws: PmUserWsProductClock",
        "public_http: PmPublicHttpProductClock",
        "read_server_time_http: PmReadServerTimeProductClock",
        "private_read: PmPrivateReadProductClock",
        "actor: PmActorProductClock",
        "okx: PmOkxProductClock",
        "deferred_mutation: PmDeferredMutationClockDomain",
        "domain: self.domain",
    ] {
        assert!(
            custody.contains(required),
            "missing deferred clock-custody invariant: {required}"
        );
    }
    for forbidden in [
        "MutationTimeAuthority",
        "PmPlaceServerTimeProductClock",
        "PmCancelServerTimeProductClock",
        "PmPlaceMutationTimeProof",
        "PmCancelMutationTimeProof",
        "PmPlaceMutationTimeFinalizer",
        "PmCancelMutationTimeFinalizer",
        "#[derive(Clone",
        "#[derive(Copy",
        "Serialize",
        "Deserialize",
        "pub struct PmDeferredMutationClockCapsule",
        "impl Clone for PmDeferredMutationClockDomain",
        "impl Copy for PmDeferredMutationClockDomain",
        "impl From<",
        "impl Into<",
        "Deref",
        "AsRef",
        "observe_rest_edge",
        "sample(",
    ] {
        assert!(
            !custody.contains(forbidden),
            "deferred clock custody gained an early capability: {forbidden}"
        );
    }

    for required in [
        "pub(crate) fn into_purpose_closed_views(self)",
        "let place_time_authority = Arc::new(MutationTimeAuthority",
        "purpose: MutationTimePurpose::Place",
        "let cancel_time_authority = Arc::new(MutationTimeAuthority",
        "purpose: MutationTimePurpose::Cancel",
        "place_server_time_http: PmPlaceServerTimeProductClock",
        "place_mutation_time_finalizer: PmPlaceMutationTimeFinalizer",
        "cancel_server_time_http: PmCancelServerTimeProductClock",
        "cancel_mutation_time_finalizer: PmCancelMutationTimeFinalizer",
    ] {
        assert!(
            expansion.contains(required),
            "missing deferred clock-expansion invariant: {required}"
        );
    }
    assert_eq!(
        expansion.matches("Arc::new(MutationTimeAuthority").count(),
        2,
        "deferred expansion must allocate exactly one authority per purpose"
    );
    assert_eq!(
        production_prefix(PRODUCT_CLOCK)
            .matches("Arc::new(MutationTimeAuthority")
            .count(),
        4,
        "only the compatible full split and deferred consuming expansion allocate purpose authorities"
    );

    for required in [
        "pub struct PmDeferredMutationClockCapsule",
        "clock_domain: PmDeferredMutationClockDomain",
        "http_config: PmPublicHttpConfig",
        "scope: PmWireScope",
        "_actor_local: PhantomData<Rc<()>>",
        "PmDeferredMutationClockCapsule(<non-authoritative; selected-route-scope-and-domain redacted>)",
    ] {
        assert!(
            capsule.contains(required),
            "missing selected deferred-capsule invariant: {required}"
        );
    }
    for forbidden in [
        "#[derive(Clone",
        "#[derive(Copy",
        "Serialize",
        "Deserialize",
        "pub fn",
        "pub const fn",
        "impl Clone",
        "impl Copy",
        "impl From<",
        "impl Into<",
        "Deref",
        "AsRef",
        "observe_",
        "sample(",
        "into_parts",
        "into_purpose_closed_views",
    ] {
        assert!(
            !capsule.contains(forbidden),
            "selected deferred capsule gained an escape: {forbidden}"
        );
    }
    assert!(
        !production_prefix(PRODUCT_CLOCK).contains("pub struct PmDeferredMutationClockCapsule"),
        "the bare domain capsule must never be public"
    );

    for required in [
        "pub struct PmPublicObservationWithDeferredMutationClockOwner",
        "observation: PmPublicObservationConnectivityOwner",
        "deferred_mutation_clock: PmDeferredMutationClockCapsule",
        "if !public_ws_config.is_production()",
        "if parser_config.scope() != public_ws_config.scope()",
        "PmPublicHttpConfig::production_on_fixed_tls_peer_and_selected_local_egress(",
        "let deferred_http_config = http_config.clone()",
        "let configured_scope = parser_config.scope()",
        "clock_owner.split_observation_with_deferred_mutation()",
        "PmPublicObservationConnectivityOwner::from_observation_clock_views(",
        "let deferred_mutation_clock = PmDeferredMutationClockCapsule",
        "clock_domain",
        "http_config: deferred_http_config",
        "scope: configured_scope",
        "_actor_local: PhantomData",
        "PmPublicObservationConnectivityRoles",
        "PmDeferredMutationClockCapsule",
        "pub const fn production_order_entry_authorized(&self) -> bool",
        "false",
    ] {
        assert!(
            staging.contains(required),
            "missing deferred observation-staging invariant: {required}"
        );
    }
    let ws_check = staging
        .find("if !public_ws_config.is_production()")
        .unwrap();
    let scope_check = staging
        .find("if parser_config.scope() != public_ws_config.scope()")
        .unwrap();
    let http_config = staging
        .find("PmPublicHttpConfig::production_on_fixed_tls_peer_and_selected_local_egress(")
        .unwrap();
    let deferred_split = staging
        .find("clock_owner.split_observation_with_deferred_mutation()")
        .unwrap();
    let observation_assembly = staging
        .find("PmPublicObservationConnectivityOwner::from_observation_clock_views(")
        .unwrap();
    let bound_capsule = staging
        .find("let deferred_mutation_clock = PmDeferredMutationClockCapsule")
        .unwrap();
    assert!(
        ws_check < scope_check
            && scope_check < http_config
            && http_config < deferred_split
            && deferred_split < observation_assembly
            && observation_assembly < bound_capsule
    );
    for forbidden in [
        "PmPlaceMutationTimeOwner",
        "PmCancelMutationTimeOwner",
        "PmPlaceServerTimeHttpRole",
        "PmCancelServerTimeHttpRole",
        "PmPlaceMutationTimeFinalizer",
        "PmCancelMutationTimeFinalizer",
        "into_purpose_closed_views",
        ".await",
    ] {
        assert!(
            !staging.contains(forbidden),
            "deferred observation staging gained early mutation-time construction: {forbidden}"
        );
    }

    for required in [
        "pub struct PmProductionSelectedPlaceCancelTimeOwner",
        "reason = \"sealed until a future purpose-specific runner-gated bridge consumes these owners\"",
        "place: PmPlaceMutationTimeOwner",
        "cancel: PmCancelMutationTimeOwner",
        "scope: PmWireScope",
        "_actor_local: PhantomData<Rc<()>>",
        "pub fn from_deferred_clock(",
        "deferred_clock: PmDeferredMutationClockCapsule",
        "let PmDeferredMutationClockCapsule",
        "clock_domain",
        "http_config",
        "scope",
        "_actor_local: _",
        ".into_purpose_closed_views()",
        "PmPlaceMutationTimeOwner::with_product_clock(",
        "PmCancelMutationTimeOwner::with_product_clock(",
        "pub const fn configured_scope(&self) -> PmWireScope",
        "#[cfg(test)]",
        "pub(crate) fn into_purpose_owners(",
        "Ok(Self {",
        "pub const fn production_order_entry_authorized(&self) -> bool",
        "false",
    ] {
        assert!(
            promotion.contains(required),
            "missing selected deferred-promotion invariant: {required}"
        );
    }
    let bound_config = promotion
        .find("let PmDeferredMutationClockCapsule")
        .unwrap();
    let authority_expansion = promotion.find(".into_purpose_closed_views()").unwrap();
    let place_owner = promotion
        .find("PmPlaceMutationTimeOwner::with_product_clock(")
        .unwrap();
    let cancel_owner = promotion
        .find("PmCancelMutationTimeOwner::with_product_clock(")
        .unwrap();
    let atomic_return = promotion.find("Ok(Self {").unwrap();
    assert!(
        bound_config < authority_expansion
            && authority_expansion < place_owner
            && place_owner < cancel_owner
            && cancel_owner < atomic_return
    );
    assert_eq!(
        promotion.matches("    pub fn ").count(),
        1,
        "the selected promoter publicly exposes only its consuming capsule constructor"
    );
    assert_eq!(
        promotion.matches("    pub const fn ").count(),
        2,
        "the selected promoter publicly exposes only scope and constant-false authority observations"
    );
    for forbidden in [
        ".await",
        "fresh_place_time",
        "fresh_cancel_time",
        "L2Credentials",
        "authenticate_exact_place",
        "authenticate_exact_owned_cancel",
        "SerializedPlaceRequest",
        "SerializedOwnedCancelRequest",
        "reqwest",
        "PmFixedTlsPeerSelection",
        "PmLocalEgressSelection",
        "connect_timeout",
        "request_timeout",
        "production_on_fixed_tls_peer_and_selected_local_egress",
        "pub fn into_purpose_owners(",
    ] {
        assert!(
            !promotion.contains(forbidden),
            "selected deferred promotion gained request or credential authority: {forbidden}"
        );
    }

    assert_eq!(
        production_prefix(DEFERRED_MUTATION_TIME)
            .matches("clock_owner.split_observation_with_deferred_mutation()")
            .count(),
        1,
        "one selected staging path must mint deferred clock custody"
    );
    assert_eq!(
        production_prefix(DEFERRED_MUTATION_TIME)
            .matches(".into_purpose_closed_views()")
            .count(),
        1,
        "one selected promotion path must consume deferred clock custody"
    );
    for required in [
        "PmDeferredMutationClockCapsule",
        "PmPublicObservationWithDeferredMutationClockOwner",
        "PmProductionSelectedPlaceCancelTimeOwner",
        "PmPublicConnectivityOwner",
    ] {
        assert!(
            LIB.contains(required),
            "missing compatible export: {required}"
        );
    }
}

#[test]
fn production_mutation_time_is_purpose_closed_and_legacy_is_feature_gated() {
    for required in [
        "pub struct PmPlaceMutationTimeOwner",
        "pub struct PmCancelMutationTimeOwner",
        "pub struct PmPlaceServerTimeHttpRole",
        "pub struct PmCancelServerTimeHttpRole",
        "pub const fn observed_l2_timestamp_seconds(&self) -> u64",
        "MutationTimePurpose::Place",
        "MutationTimePurpose::Cancel",
        "provider.consume_final_place_time(PmFinalPlaceMutationTime { core: &proof.core })",
        "provider.consume_final_cancel_time(PmFinalCancelMutationTime { core: &proof.core })",
        "#[cfg(any(test, feature = \"loopback-evidence\"))]",
        "Purpose-erased proof retained only for literal-loopback compatibility.",
    ] {
        let source = [PRODUCT_CLOCK, PUBLIC_HTTP, PUBLIC_CONNECTIVITY, LIB].join("\n");
        assert!(
            source.contains(required),
            "missing purpose-closed mutation-time invariant: {required}"
        );
    }

    let place_observation = PUBLIC_HTTP
        .split("pub struct PmPlaceServerTimeObservation {")
        .nth(1)
        .unwrap()
        .split("pub struct PmCancelServerTimeObservation {")
        .next()
        .unwrap();
    let cancel_observation = PUBLIC_HTTP
        .split("pub struct PmCancelServerTimeObservation {")
        .nth(1)
        .unwrap()
        .split("struct FetchedServerTime")
        .next()
        .unwrap();
    for carrier in [place_observation, cancel_observation] {
        for forbidden in [
            "parsed_l2_timestamp",
            "pub fn timestamp",
            "pub const fn timestamp",
            "pub fn l2_timestamp",
            "pub const fn l2_timestamp",
        ] {
            assert!(
                !carrier.contains(forbidden),
                "production mutation-time observation exposes raw time: {forbidden}"
            );
        }
    }

    assert!(!PUBLIC_CONNECTIVITY.contains("PmMutationServerTimeHttpRole"));
    assert!(!PUBLIC_CONNECTIVITY.contains("PmMutationServerTimeValidator"));
}

#[test]
fn place_hmac_bridge_uses_hidden_source_time_and_rechecks_at_the_auth_boundary() {
    let bridge = PRODUCT_CLOCK
        .split("pub fn authenticate_exact_place(")
        .nth(1)
        .unwrap()
        .split("pub fn consume_with(")
        .next()
        .unwrap();
    for required in [
        "proof: PmPlaceMutationTimeProof",
        "expected_seconds: u64",
        "credentials: &L2Credentials",
        "request: SerializedPlaceRequest",
        "Result<AuthenticatedPlaceRequest, PmPlaceMutationAuthenticationError>",
        "validate_final_mutation_time(&self.authority, &proof.core, MutationTimePurpose::Place)?",
        "proof.core.timestamp.unix_seconds() != expected_seconds",
        "PmPlaceMutationAuthenticationError::ObservedTimestampMismatch",
        "PmFinalPlaceMutationTime { core: &proof.core }.consume_l2_timestamp()?",
        "credentials.authenticate_place(timestamp, request)",
    ] {
        assert!(
            bridge.contains(required),
            "missing exact place-HMAC bridge invariant: {required}"
        );
    }
    for forbidden in [
        "L2Timestamp::from_unix_seconds(expected_seconds)",
        "pub fn timestamp",
        "pub fn l2_timestamp",
        "pub fn credentials",
        "pub fn request",
    ] {
        assert!(
            !bridge.contains(forbidden),
            "place-HMAC bridge exposes or reconstructs authority: {forbidden}"
        );
    }

    let observation = PUBLIC_HTTP
        .split("impl PmPlaceServerTimeObservation {")
        .nth(1)
        .unwrap()
        .split("impl std::fmt::Debug for PmPlaceServerTimeObservation")
        .next()
        .unwrap();
    for required in [
        "pub const fn observed_l2_timestamp_seconds(&self) -> u64",
        "self.proof.observed_l2_timestamp_seconds()",
        "This value grants no authentication authority.",
    ] {
        assert!(
            observation.contains(required),
            "missing evidence-only source timestamp invariant: {required}"
        );
    }
}

#[test]
fn cancel_hmac_bridge_uses_hidden_source_time_and_rechecks_at_the_auth_boundary() {
    let bridge = PRODUCT_CLOCK
        .split("pub fn authenticate_exact_owned_cancel(")
        .nth(1)
        .unwrap()
        .split("pub fn consume_with(")
        .next()
        .unwrap();
    for required in [
        "proof: PmCancelMutationTimeProof",
        "expected_seconds: u64",
        "credentials: &L2Credentials",
        "request: SerializedOwnedCancelRequest",
        "Result<AuthenticatedOwnedCancelRequest, PmCancelMutationAuthenticationError>",
        "validate_final_mutation_time(&self.authority, &proof.core, MutationTimePurpose::Cancel)?",
        "proof.core.timestamp.unix_seconds() != expected_seconds",
        "PmCancelMutationAuthenticationError::ObservedTimestampMismatch",
        "PmFinalCancelMutationTime { core: &proof.core }.consume_l2_timestamp()?",
        "credentials.authenticate_owned_cancel(timestamp, request)",
    ] {
        assert!(
            bridge.contains(required),
            "missing exact-owned cancel-HMAC bridge invariant: {required}"
        );
    }
    for forbidden in [
        "L2Timestamp::from_unix_seconds(expected_seconds)",
        "pub fn timestamp",
        "pub fn l2_timestamp",
        "pub fn credentials",
        "pub fn request",
    ] {
        assert!(
            !bridge.contains(forbidden),
            "cancel-HMAC bridge exposes or reconstructs authority: {forbidden}"
        );
    }

    let observation = PUBLIC_HTTP
        .split("impl PmCancelServerTimeObservation {")
        .nth(1)
        .unwrap()
        .split("impl std::fmt::Debug for PmCancelServerTimeObservation")
        .next()
        .unwrap();
    for required in [
        "pub const fn observed_l2_timestamp_seconds(&self) -> u64",
        "self.proof.observed_l2_timestamp_seconds()",
        "This value grants no authentication authority.",
    ] {
        assert!(
            observation.contains(required),
            "missing evidence-only cancel timestamp invariant: {required}"
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
    assert!(RECONCILIATION.contains("authority: &'a mut dyn PmHttpReadAuthorityProvider"));
    assert!(ACCOUNT.contains("authority: &'a mut dyn PmHttpReadAuthorityProvider"));
    assert!(PRIVATE_HTTP.contains("authority: Box<dyn PmHttpReadAuthorityProvider>"));
    assert!(USER_WS.contains("credentials: Box<dyn PmUserWsReadAuthorityProvider>"));
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
        "authenticate_user_subscription(config.condition())",
        "subscription.dispatch(&mut RetainSubscriptionSink)",
        "parse_live_user_frame(raw.as_slice())",
        "credentials.bind_user_frame(frame).await",
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
    assert!(HTTP_TRANSPORT.contains("url.set_path(\"/ok\")"));
    assert!(HTTP_TRANSPORT.contains("url.set_path(\"/time\")"));
    assert!(HTTP_TRANSPORT.contains("url.set_path(\"/book\")"));
    assert!(HTTP_TRANSPORT.contains(".append_pair(\"token_id\""));
    assert!(HTTP_TRANSPORT.contains("format!(\"/markets/{condition}\")"));
    assert!(HTTP_TRANSPORT.contains("format!(\"/clob-markets/{condition}\")"));
    assert!(HTTP_TRANSPORT.contains("url.set_path(\"/api/geoblock\")"));
    assert!(HTTP_TRANSPORT.contains("url.set_path(\"/v3/summary.json\")"));
    assert!(HTTP_TRANSPORT.contains("url.set_path(\"/v3/components.json\")"));
    assert!(CLOB_HEALTH_HTTP.contains("PmPublicRoute::ClobHealth"));
    assert!(STATUS_ANNOUNCEMENT_HTTP.contains("PmPublicRoute::StatusSummary"));
    assert!(STATUS_ANNOUNCEMENT_HTTP.contains("PmPublicRoute::StatusComponents"));
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
        CLOB_HEALTH_HTTP,
        GEOBLOCK_HTTP,
        HTTP_TRANSPORT,
        PUBLIC_HTTP,
        METADATA_HTTP,
        STATUS_ANNOUNCEMENT_HTTP,
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
