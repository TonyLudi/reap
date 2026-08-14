use std::fs;
use std::path::PathBuf;

use sha2::{Digest as _, Sha256};

const LIB: &str = include_str!("../src/lib.rs");
const ATTEMPT: &str = include_str!("../src/attempt.rs");
const LINEAGE: &str = include_str!("../src/attempt/lineage.rs");
const ATTEMPT_TESTS: &str = include_str!("../src/attempt/tests.rs");
const LINEAGE_TESTS: &str = include_str!("../src/attempt/lineage/tests.rs");
const TRANSPORT: &str = include_str!("../src/transport.rs");
const LOOPBACK_TESTS: &str = include_str!("../src/transport/tests.rs");
const MANIFEST: &str = include_str!("../Cargo.toml");
const WORKSPACE: &str = include_str!("../../../Cargo.toml");
const LIB_SHA256: &str = "afc6201644ac0b185324d1444a9f45cdd36f5268a3557567f8b051909b3ad76e";
const ATTEMPT_SHA256: &str = "5a8a7bcbdc4d5c1d0ba85c5da2599b1ebe29fd239b6db0f902d9586ce92f2e97";
const LINEAGE_SHA256: &str = "dbd06915aa8c2583bb74ad07355db22eaa35c8d67e1a95eab7421ca1c8b5b2da";
const ATTEMPT_TESTS_SHA256: &str =
    "edd830aacfb4ddd2e22f150d050724035645bcdd12d1310dc74019b99b020682";
const LINEAGE_TESTS_SHA256: &str =
    "8ef2220575304a4bf871c371da1d9d538fc6ef09d30c1aef4bdfe737fc2aea5a";
const TRANSPORT_SHA256: &str = "ca5cfc01125eeedb73a792ce7080e79680e43d63db94fb02edcbf9681686fe68";
const LOOPBACK_TESTS_SHA256: &str =
    "b8bc8696b351b8bdce2ab44ae79f4b650315baa203d5ef23790113975d0b005f";
const MANIFEST_SHA256: &str = "db3852c67823bf8436e856e78add273e4d1c4bbec84ec15a1061b4cbf5217d92";
const WORKSPACE_SHA256: &str = "91a8730b2ea0327528842bdbdad06af272997e16132da057f31001d05cbdadfc";

#[test]
fn full_production_sources_and_manifests_are_hash_pinned() {
    assert_frozen_production_inputs();
}

#[test]
fn normal_external_surface_is_an_explicit_denial_only() {
    assert_frozen_production_inputs();
    let all_source = format!("{LIB}\n{TRANSPORT}\n{ATTEMPT}\n{LINEAGE}");
    assert!(LIB.contains("PRODUCTION_CREDENTIAL_PROOF_TRANSPORT_DENIED"));
    assert!(LIB.contains("the private loopback attempt has no externally reachable constructor"));
    assert!(LIB.contains("mod attempt;"));
    assert!(LIB.contains("mod transport;"));
    assert!(!LIB.contains("pub mod transport"));
    assert!(!LIB.contains("pub use"));
    assert_eq!(all_source.matches("\npub const ").count(), 1);
    for forbidden in [
        "\npub fn ",
        "\npub struct ",
        "\npub enum ",
        "\npub trait ",
        "\npub type ",
        "\npub mod ",
        "\npub use ",
        "fn production(",
        "src/main.rs",
        "[[bin]]",
    ] {
        assert!(
            !format!("{all_source}\n{MANIFEST}").contains(forbidden),
            "production surface escaped denial: {forbidden}"
        );
    }
}

#[test]
fn unreachable_production_policy_is_closed_and_exact() {
    assert_frozen_production_inputs();
    for required in [
        "scheme: \"https\"",
        "dns_name: \"clob.polymarket.com\"",
        "port: 443",
        "method: MethodPolicy::Get",
        "path: DERIVE_API_KEY_PATH",
        "connect_timeout: Duration::from_secs(3)",
        "request_timeout: Duration::from_secs(5)",
        "https_only: true",
        "proxy: ProxyPolicy::Disabled",
        "redirects: RedirectPolicy::None",
        "retry: RetryPolicy::Never",
        "http_version: HttpVersionPolicy::Http1Only",
        "maximum_idle_connections_per_host: 0",
        "connection: ConnectionPolicy::Close",
        "fixed_peer_resolve: SelectionPolicy::Required",
        "interface_binding: SelectionPolicy::Required",
        "local_address_binding: SelectionPolicy::Required",
        "maximum_response_body_bytes: MAX_L1_CREDENTIAL_DERIVATION_RESPONSE_BYTES",
        "There is intentionally no production",
    ] {
        assert!(
            TRANSPORT.contains(required),
            "closed production policy lost: {required}"
        );
    }
    assert!(!TRANSPORT.contains("fn production("));
}

#[test]
fn loopback_sink_has_no_generic_request_or_retry_surface() {
    assert_frozen_production_inputs();
    for required in [
        "#[cfg(any(test, feature = \"loopback-evidence\"))]",
        "fn loopback_evidence(",
        "L1CredentialDerivationRequestSink for LoopbackCredentialProofSink",
        ".get(url)",
        ".header(CONNECTION, \"close\")",
        ".redirect(Policy::none())",
        ".retry(reqwest::retry::never())",
        ".no_proxy()",
        ".no_gzip()",
        ".no_brotli()",
        ".no_zstd()",
        ".no_deflate()",
        ".http1_only()",
        ".pool_max_idle_per_host(PRODUCTION_POLICY.maximum_idle_connections_per_host)",
        ".interface(selected_local_egress.interface_name())",
        ".local_address(selected_local_egress.local_source_ip())",
        ".resolve(fixed_peer.dns_name(), expected_peer)",
        "url.set_path(DERIVE_API_KEY_PATH)",
        "url.set_query(None)",
        "url.set_fragment(None)",
    ] {
        assert!(
            TRANSPORT.contains(required),
            "transport pin lost: {required}"
        );
    }
    for forbidden in [
        "try_clone",
        "Box<dyn",
        "dyn L1CredentialDerivationRequestSink",
        ".post(",
        ".put(",
        ".patch(",
        ".delete(",
        "fallback",
        "create_api_key",
        "list_api_key",
        "delete_api_key",
        "println!",
        "eprintln!",
        "dbg!",
        "tracing::",
        "log::",
    ] {
        assert!(
            !TRANSPORT.contains(forbidden),
            "transport gained forbidden capability: {forbidden}"
        );
    }
    for forbidden_route in [
        "/auth/api-key",
        "/auth/api-keys",
        "/order",
        "/orders",
        "/book",
        "/data/",
    ] {
        assert!(
            !TRANSPORT.contains(forbidden_route),
            "alternate route entered fixed sink: {forbidden_route}"
        );
    }
    assert_eq!(
        TRANSPORT.matches("/auth/").count(),
        TRANSPORT.matches("/auth/derive-api-key").count()
            + TRANSPORT.matches("/auth/ban-status/closed-only").count()
    );
    for disabled_decoder in ["no_gzip", "no_brotli", "no_zstd", "no_deflate"] {
        assert_eq!(
            TRANSPORT.matches(&format!(".{disabled_decoder}()")).count(),
            1,
            "decoder policy must be explicit and singular: {disabled_decoder}"
        );
    }
}

#[test]
fn request_and_response_gates_are_source_ordered_and_secret_safe() {
    assert_frozen_production_inputs();
    for header in [
        "POLY_ADDRESS",
        "POLY_SIGNATURE",
        "POLY_TIMESTAMP",
        "POLY_NONCE",
        "POLY_API_KEY",
        "POLY_PASSPHRASE",
    ] {
        assert!(TRANSPORT.contains(header));
    }
    assert_eq!(TRANSPORT.matches("sensitive_header(").count(), 10);
    assert_eq!(TRANSPORT.matches("value.set_sensitive(true)").count(), 1);
    assert!(TRANSPORT.contains("HeaderValue::from_static(\"application/json\")"));
    assert!(TRANSPORT.contains("HeaderValue::from_static(\"identity\")"));
    assert!(!TRANSPORT.contains("headers.insert(CONTENT_TYPE"));

    let receive = TRANSPORT
        .split_once("async fn receive_one_response(")
        .and_then(|(_, tail)| {
            tail.split_once("fn validate_response_peer(")
                .map(|(body, _)| body)
        })
        .expect("receive function");
    let peer = receive.find("validate_response_peer(").unwrap();
    let status = receive.find("response.status() != StatusCode::OK").unwrap();
    let application = receive.find("validate_application_headers(").unwrap();
    let declared = receive.find("validate_declared_length(").unwrap();
    let streamed = receive.find("while let Some(chunk)").unwrap();
    let wrapped = receive
        .find("L1CredentialDerivationResponseInput::new(owned_body)")
        .unwrap();
    assert!(peer < status && status < application && application < declared);
    assert!(declared < streamed && streamed < wrapped);
    assert!(receive.contains("Zeroizing::new(Vec::with_capacity(capacity))"));
    assert!(receive.contains("std::mem::take(&mut *body)"));

    for forbidden in [
        "response.text(",
        "String::from_utf8",
        "body:?",
        "body = %",
        "error_for_status",
    ] {
        assert!(!TRANSPORT.contains(forbidden));
    }
}

#[test]
fn crate_is_isolated_and_feature_gated_in_the_workspace() {
    assert_frozen_production_inputs();
    assert!(WORKSPACE.contains("\"crates/reap-pm-credential-proof-runner\""));
    assert!(WORKSPACE.contains(
        "reap-pm-credential-proof-runner = { path = \"crates/reap-pm-credential-proof-runner\" }"
    ));
    for required in [
        "default = []",
        "loopback-evidence = [",
        "\"dep:libc\"",
        "\"dep:reap-polymarket-egress-binding\"",
        "\"dep:reqwest\"",
        "\"dep:serde\"",
        "\"dep:serde_json\"",
        "\"dep:sha2\"",
        "\"dep:tokio\"",
        "\"dep:zeroize\"",
        "\"reap-polymarket-egress-binding/loopback-evidence\"",
        "libc = { workspace = true, optional = true }",
        "reap-polymarket-auth.workspace = true",
        "reap-polymarket-egress-binding = { workspace = true, optional = true }",
        "reqwest = { workspace = true, optional = true }",
        "serde = { workspace = true, optional = true }",
        "serde_json = { workspace = true, optional = true }",
        "sha2 = { workspace = true, optional = true }",
        "tokio = { workspace = true, optional = true }",
        "zeroize = { workspace = true, optional = true }",
        "libc.workspace = true",
        "reqwest.workspace = true",
        "serde.workspace = true",
        "serde_json.workspace = true",
        "sha2.workspace = true",
        "tempfile = \"3\"",
        "tokio.workspace = true",
        "zeroize.workspace = true",
    ] {
        assert!(MANIFEST.contains(required), "manifest pin lost: {required}");
    }
}

#[test]
fn private_attempt_is_exactly_ordered_loopback_only_and_always_denied() {
    assert_frozen_production_inputs();
    for required in [
        "execute_private_loopback_credential_proof_attempt",
        "SignerAndL2ActorMismatch",
        "L1CredentialDerivationNonce::from_u64(0)",
        "consume_with_l1_credential_derivation_response",
        "consume_into_authenticated_closed_only",
        "DeniedCredentialProofAttempt",
        "\"DENIED\"",
        "production_permit(&self) -> bool",
        "resume_allowed(&self) -> bool",
        "NO_REMOTE_OR_MUTATION_AUTHORITY",
    ] {
        assert!(ATTEMPT.contains(required), "attempt pin lost: {required}");
    }
    for forbidden in [
        "pub fn execute_private",
        "pub struct DeniedCredentialProofAttempt",
        "pub struct CredentialProofAttemptCommitmentInputs",
        "fn production(",
        "std::env",
        "env::var",
        "read_to_string",
        "OpenOptions",
        "reqwest",
        "TcpStream",
        "SerializedPlaceRequest",
        "AuthenticatedPlaceRequest",
        "FixedPlaceRequestSink",
        "FixedOwnedCancelRequestSink",
    ] {
        assert!(
            !ATTEMPT.contains(forbidden),
            "attempt escaped scope: {forbidden}"
        );
    }

    let run = ATTEMPT
        .split_once("pub(crate) fn execute_private_loopback_credential_proof_attempt(")
        .expect("private attempt")
        .1
        .split_once("fn require_nonzero_inputs(")
        .expect("bounded private attempt")
        .0;
    let prepared = run.find("PreparedAttemptLineage::create_new").unwrap();
    let consumed = run.find("let mut burned = prepared.burn()").unwrap();
    let first_validate = run.find("burned.validate_exact()").unwrap();
    let first_time = run.find("transport.first_server_time()").unwrap();
    let nonce_zero = run
        .find("L1CredentialDerivationNonce::from_u64(0)")
        .unwrap();
    let second_validate = run[first_time..]
        .find("burned.validate_exact()")
        .map(|index| index + first_time)
        .unwrap();
    let derive = run
        .find("derive_request\n        .dispatch(&mut transport)")
        .unwrap();
    let tuple_join = run
        .find("consume_with_l1_credential_derivation_response")
        .unwrap();
    let third_validate = run[derive..]
        .find("burned.validate_exact()")
        .map(|index| index + derive)
        .unwrap();
    let second_time = run.find("transport.second_server_time()").unwrap();
    let same_holder = run.find("consume_into_authenticated_closed_only").unwrap();
    let fourth_validate = run[second_time..]
        .find("burned.validate_exact()")
        .map(|index| index + second_time)
        .unwrap();
    let closed = run
        .find("closed_only_request\n        .dispatch(&mut transport)")
        .unwrap();
    let final_record = run.find("burned.finish()").unwrap();
    assert!(prepared < consumed && consumed < first_validate && first_validate < first_time);
    assert!(first_time < nonce_zero && nonce_zero < second_validate && second_validate < derive);
    assert!(derive < tuple_join && tuple_join < third_validate && third_validate < second_time);
    assert!(second_time < same_holder && same_holder < fourth_validate && fourth_validate < closed);
    assert!(closed < final_record);
    assert_eq!(run.matches("burned.validate_exact()").count(), 4);
}

#[test]
fn durable_attempt_lineage_is_create_new_protected_exact_and_nonresumable() {
    assert_frozen_production_inputs();
    for required in [
        "pm-t2-phase-a-credential-proof-attempt-v1.jsonl",
        "pm-t2-phase-a-credential-proof-attempt-burn-claim-v1.json",
        "\\\"record\\\":\\\"Prepared\\\"",
        "\\\"record\\\":\\\"Consumed\\\"",
        "\\\"record\\\":\\\"Final\\\"",
        "\\\"authorization\\\":\\\"DENIED\\\"",
        "\\\"production_permit\\\":false",
        "\\\"resume_allowed\\\":false",
        ".create_new(true)",
        ".mode(0o600)",
        "libc::O_NOFOLLOW | libc::O_CLOEXEC | libc::O_NONBLOCK",
        "metadata.nlink() != 1",
        "directory.file.try_lock()",
        "file.sync_all()",
        "directory.file.sync_all()",
        "claim presence is a permanent burn signal",
        "there is no reopen or resume",
        "same-EUID process",
        "validate_exact(&mut self)",
        "AttemptLineageInspection",
        "Pristine",
        "PreparedUnclaimed",
        "BurnClaimEmpty",
        "BurnClaimPartial",
        "BurnClaimDurable",
        "ConsumedDenied",
        "CompleteDenied",
        "inspect_attempt_lineage",
        "inspect_while_leased",
        "Only `Pristine` may be followed by the create-new writer",
        "create_new_eligible()",
        "let claim_present = lease.entry_present(ATTEMPT_BURN_CLAIM_FILE)",
        "if claim_present",
        "return Err(LineageError::AlreadyBurned)",
        "let ledger = lease.read_entry(ATTEMPT_LEDGER_FILE",
        "let claim = lease.read_entry(ATTEMPT_BURN_CLAIM_FILE",
        "let final_validation = lease.validate()",
        "remote_acceptance_proven\\\":false",
        "mutation_authority\\\":false",
    ] {
        assert!(LINEAGE.contains(required), "lineage pin lost: {required}");
    }
    for forbidden in [
        "open_existing",
        "resume(",
        "recover(",
        "remove_file",
        "remove_dir",
        "unlink",
        "secure_erase",
        "secure erase",
        "timestamp_seconds",
        "poly_signature",
        "poly_timestamp",
        "apiKey",
        "passphrase",
        "response_digest",
        "response_fingerprint",
        "production_permit\\\":true",
        "resume_allowed\\\":true",
    ] {
        assert!(
            !LINEAGE.contains(forbidden),
            "lineage escaped scope: {forbidden}"
        );
    }

    let burn = LINEAGE
        .split_once("pub(crate) fn burn(self)")
        .expect("burn transition")
        .1
        .split_once("impl BurnedAttemptLineage")
        .expect("bounded burn transition")
        .0;
    let claim_create = burn.find("ProtectedFile::create_new(&claim_path").unwrap();
    let claim_write = burn
        .find("claim.append_durable(&[], &claim_bytes)")
        .unwrap();
    let consumed_write = burn
        .find("ledger.append_durable(&expected_ledger, &consumed_bytes)")
        .unwrap();
    let revalidate = burn[consumed_write..]
        .find("ledger.validate_exact(&expected_ledger)")
        .map(|index| index + consumed_write)
        .unwrap();
    assert!(
        claim_create < claim_write && claim_write < consumed_write && consumed_write < revalidate
    );
    for state in [
        "Pristine",
        "PreparedUnclaimed",
        "BurnClaimEmpty",
        "BurnClaimPartial",
        "BurnClaimDurable",
        "ConsumedDenied",
        "CompleteDenied",
    ] {
        assert!(LINEAGE.contains(state), "inspection state lost: {state}");
    }

    let create = LINEAGE
        .split_once("pub(crate) fn create_new(")
        .expect("create-new transition")
        .1
        .split_once("pub(crate) fn burn(self)")
        .expect("bounded create-new transition")
        .0;
    let claim_presence = create
        .find("lease.entry_present(ATTEMPT_BURN_CLAIM_FILE)")
        .unwrap();
    let inspection = create.find("inspect_while_leased(&lease)").unwrap();
    let claim_dominates = create.find("if claim_present").unwrap();
    let eligibility = create.find("initial.create_new_eligible()").unwrap();
    assert!(
        claim_presence < inspection
            && inspection < claim_dominates
            && claim_dominates < eligibility
    );
}

#[test]
fn attempt_transport_is_one_fixed_four_stage_sequence_without_mutation_routes() {
    assert_frozen_production_inputs();
    for required in [
        "AttemptTransportStage::FirstTime",
        "AttemptTransportStage::Derive",
        "AttemptTransportStage::SecondTime",
        "AttemptTransportStage::ClosedOnly",
        "AttemptTransportStage::Spent",
        "const SERVER_TIME_PATH: &str = \"/time\"",
        "const DERIVE_API_KEY_PATH: &str = \"/auth/derive-api-key\"",
        "const CLOSED_ONLY_PATH: &str = \"/auth/ban-status/closed-only\"",
        "FixedClosedOnlyRequestSink for LoopbackCredentialProofAttemptTransport",
        "body.as_slice() != br#\"{\"closed_only\":false}\"#",
        "CredentialProofTransportError::ClosedOnlyTrueOrMalformed",
        "maximum_body_bytes",
        "const MAX_LOOPBACK_SOURCE_TIME_AGE: Duration = Duration::from_secs(5)",
        "monotonic_receive: Instant",
        "active_server_time: Option<ActiveServerTimeObservation>",
        "take_matching_server_time(poly_timestamp)",
        "require_fresh_source_time(source_time)?",
        "validate_source_time_age(observation, Instant::now())",
        "if age > MAX_LOOPBACK_SOURCE_TIME_AGE",
        "value.set_sensitive(true)",
    ] {
        assert!(
            TRANSPORT.contains(required),
            "attempt transport pin lost: {required}"
        );
    }
    for forbidden in [
        ".post(",
        ".put(",
        ".patch(",
        ".delete(",
        "/order",
        "/orders",
        "/data/",
        "retry(reqwest::retry::default",
    ] {
        assert!(
            !TRANSPORT.contains(forbidden),
            "transport route escaped: {forbidden}"
        );
    }
}

#[test]
fn attempt_tests_are_synthetic_loopback_and_cover_burn_without_resume() {
    assert_frozen_production_inputs();
    for required in [
        "published synthetic",
        "TcpListener::bind(\"127.0.0.1:0\")",
        "\"127.0.0.2\".parse()",
        "durable_attempt_orders_four_exact_requests_and_finishes_denied",
        "strict_closed_only_rejection_stays_burned_and_cannot_retry",
        "GET /time HTTP/1.1",
        "GET /auth/derive-api-key HTTP/1.1",
        "GET /auth/ban-status/closed-only HTTP/1.1",
        "CredentialProofAttemptError::AttemptAlreadyBurned",
        "lineage::AttemptLineageInspection::CompleteDenied",
    ] {
        assert!(
            ATTEMPT_TESTS.contains(required),
            "attempt test pin lost: {required}"
        );
    }
    for forbidden in ["std::env", "env::var", "Command::", "UdpSocket"] {
        assert!(!ATTEMPT_TESTS.contains(forbidden));
    }
    assert!(LINEAGE_TESTS.contains("prepared_without_claim_is_nonresumable_and_never_reopened"));
    assert!(LINEAGE_TESTS.contains("claim_presence_burns_even_when_the_ledger_is_absent"));
    assert!(LINEAGE_TESTS.contains("exact_byte_revalidation_detects_a_same_inode_append"));
    assert!(
        LINEAGE_TESTS.contains("inspector_classifies_all_seven_exact_nonresumable_crash_shapes")
    );
    assert!(
        LINEAGE_TESTS
            .contains("inspector_rejects_nonprefix_claim_and_ledger_suffix_as_hard_burned_errors")
    );
    assert!(
        LOOPBACK_TESTS
            .contains("source_time_age_accepts_exact_boundary_and_rejects_boundary_plus_one")
    );
    assert!(LOOPBACK_TESTS.contains("expired_source_time_blocks_derive_before_a_second_socket"));
    assert!(LOOPBACK_TESTS.contains("expired time opened an authenticated socket"));
}

#[test]
fn crate_docs_disclaim_every_unowned_proof_and_authority() {
    assert_frozen_production_inputs();
    for required in [
        "no production constructor, command, binary",
        "durable attempt/source-time owner",
        "caller/test evidence only",
        "does not prove a",
        "server or TLS peer",
        "response currentness or uniqueness",
        "credential-tuple",
        "provider delivery",
        "proxy mapping or control",
        "mutation authority",
        "no credential create/list/delete operation",
        "L2 route",
        "order route",
        "reqwest, hyper, rustls",
        "outside that guarantee",
    ] {
        assert!(
            LIB.contains(required),
            "boundary disclaimer lost: {required}"
        );
    }
}

#[test]
fn transport_tests_use_only_published_synthetic_loopback_fixtures() {
    assert_frozen_production_inputs();
    for required in [
        "published synthetic fixtures",
        "never\n// load credentials",
        "TcpListener::bind(\"127.0.0.1:0\")",
        "\"127.0.0.2\".parse()",
        "PmFixedTlsPeerSelection::loopback_evidence",
        "PmLocalEgressSelection::loopback_evidence",
        "redirect_policy_does_not_follow_a_valid_location_to_a_second_loopback_listener",
        "count_redirect_target_accepts(redirect_target, Duration::from_millis(350))",
        "Location: http://{redirect_target_address}/redirect-target",
        "assert_eq!(redirect_accepts, 0, \"redirect target received a follow\")",
    ] {
        assert!(
            LOOPBACK_TESTS.contains(required),
            "synthetic loopback test pin lost: {required}"
        );
    }
    for forbidden in [
        "std::env",
        "env::var",
        "std::fs",
        "File::",
        "TcpStream::connect",
        "UdpSocket",
        "Command::",
    ] {
        assert!(
            !LOOPBACK_TESTS.contains(forbidden),
            "test acquired non-synthetic input or egress: {forbidden}"
        );
    }
}

#[test]
fn package_contains_no_binary_or_unreviewed_extra_source_file() {
    assert_frozen_production_inputs();
    let root = PathBuf::from(env!("CARGO_MANIFEST_DIR"));
    let mut entries = Vec::new();
    collect_entries(&root, &root, &mut entries);
    entries.sort();
    assert_eq!(
        entries,
        [
            PathBuf::from("Cargo.toml"),
            PathBuf::from("src"),
            PathBuf::from("src/attempt"),
            PathBuf::from("src/attempt/lineage"),
            PathBuf::from("src/attempt/lineage/tests.rs"),
            PathBuf::from("src/attempt/lineage.rs"),
            PathBuf::from("src/attempt/tests.rs"),
            PathBuf::from("src/attempt.rs"),
            PathBuf::from("src/lib.rs"),
            PathBuf::from("src/transport"),
            PathBuf::from("src/transport/tests.rs"),
            PathBuf::from("src/transport.rs"),
            PathBuf::from("tests"),
            PathBuf::from("tests/source_policy.rs"),
        ]
    );
    for forbidden_manifest_surface in [
        "\n[lib]",
        "\n[[bin]]",
        "\n[[example]]",
        "\n[[bench]]",
        "\nbuild =",
    ] {
        assert!(
            !MANIFEST.contains(forbidden_manifest_surface),
            "custom target/build path entered frozen manifest: {forbidden_manifest_surface}"
        );
    }
}

fn assert_frozen_production_inputs() {
    for (label, source, expected) in [
        ("src/lib.rs", LIB.as_bytes(), LIB_SHA256),
        ("src/attempt.rs", ATTEMPT.as_bytes(), ATTEMPT_SHA256),
        ("src/attempt/lineage.rs", LINEAGE.as_bytes(), LINEAGE_SHA256),
        (
            "src/attempt/tests.rs",
            ATTEMPT_TESTS.as_bytes(),
            ATTEMPT_TESTS_SHA256,
        ),
        (
            "src/attempt/lineage/tests.rs",
            LINEAGE_TESTS.as_bytes(),
            LINEAGE_TESTS_SHA256,
        ),
        ("src/transport.rs", TRANSPORT.as_bytes(), TRANSPORT_SHA256),
        (
            "src/transport/tests.rs",
            LOOPBACK_TESTS.as_bytes(),
            LOOPBACK_TESTS_SHA256,
        ),
        ("Cargo.toml", MANIFEST.as_bytes(), MANIFEST_SHA256),
        (
            "workspace Cargo.toml",
            WORKSPACE.as_bytes(),
            WORKSPACE_SHA256,
        ),
    ] {
        assert_eq!(
            sha256_hex(source),
            expected,
            "frozen production input changed: {label}"
        );
    }
}

fn sha256_hex(source: &[u8]) -> String {
    format!("{:x}", Sha256::digest(source))
}

fn collect_entries(
    root: &std::path::Path,
    directory: &std::path::Path,
    entries: &mut Vec<PathBuf>,
) {
    for entry in fs::read_dir(directory).unwrap() {
        let path = entry.unwrap().path();
        let metadata = fs::symlink_metadata(&path).unwrap();
        assert!(
            !metadata.file_type().is_symlink(),
            "symlink not allowed: {path:?}"
        );
        entries.push(path.strip_prefix(root).unwrap().to_path_buf());
        if metadata.is_dir() {
            collect_entries(root, &path, entries);
        } else {
            assert!(metadata.is_file(), "non-file crate entry: {path:?}");
        }
    }
}
