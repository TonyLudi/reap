use std::{cell::Cell, convert::Infallible, fmt::Write as _, rc::Rc};

use reap_polymarket_auth::{
    FixedClosedOnlyRequestSink, L1CredentialDerivationMatchedL2Credentials,
    L1CredentialDerivationResponseInput, L2CredentialInput, L2Credentials, L2Timestamp,
    MAX_L1_CREDENTIAL_DERIVATION_RESPONSE_BYTES, PmAuthError,
};
use sha2::{Digest, Sha256};

const ADDRESS: &str = "0xf39Fd6e51aad88F6F4ce6aB8827279cffFb92266";
const API_KEY: &str = "00000000-0000-4000-8000-000000000001";
const OTHER_API_KEY: &str = "00000000-0000-4000-8000-000000000002";
const SECRET: &str = "AA==";
const OTHER_SECRET: &str = "AQ==";
const PASSPHRASE: &str = "synthetic-passphrase";
const OTHER_PASSPHRASE: &str = "synthetic-passphrasf";

fn credentials() -> L2Credentials {
    credentials_with(API_KEY, SECRET, PASSPHRASE)
}

fn credentials_with(api_key: &str, secret: &str, passphrase: &str) -> L2Credentials {
    L2Credentials::bind(
        ADDRESS,
        L2CredentialInput::new(api_key.into(), secret.into(), passphrase.into()),
    )
    .unwrap()
}

fn response(api_key: &str, secret: &str, passphrase: &str) -> L1CredentialDerivationResponseInput {
    L1CredentialDerivationResponseInput::new(
        format!(r#"{{"apiKey":"{api_key}","secret":"{secret}","passphrase":"{passphrase}"}}"#)
            .into_bytes(),
    )
    .unwrap()
}

fn join_raw(raw: &str) -> Result<L1CredentialDerivationMatchedL2Credentials, PmAuthError> {
    credentials().consume_with_l1_credential_derivation_response(
        L1CredentialDerivationResponseInput::new(raw.as_bytes().to_vec()).unwrap(),
    )
}

fn assert_invalid(raw: &str) {
    assert_eq!(
        join_raw(raw).unwrap_err(),
        PmAuthError::InvalidL1CredentialDerivationResponse,
        "accepted or misclassified response: {raw}"
    );
}

struct OutputDropProbe(Rc<Cell<bool>>);

impl Drop for OutputDropProbe {
    fn drop(&mut self) {
        self.0.set(true);
    }
}

struct ClosedOnlyCapture {
    calls: usize,
    headers: Option<[String; 5]>,
    output: Option<OutputDropProbe>,
}

#[derive(Debug, PartialEq, Eq)]
struct SyntheticClosedOnlySinkError;

struct ErroringClosedOnlyCapture {
    calls: usize,
    retained_header_copies: Option<[String; 5]>,
}

impl FixedClosedOnlyRequestSink for ErroringClosedOnlyCapture {
    type Output = ();
    type Error = SyntheticClosedOnlySinkError;

    fn send_exact_get_auth_ban_status_closed_only(
        &mut self,
        poly_address: &str,
        poly_signature: &str,
        poly_timestamp: &str,
        poly_api_key: &str,
        poly_passphrase: &str,
    ) -> Result<Self::Output, Self::Error> {
        self.calls += 1;
        self.retained_header_copies = Some([
            poly_address.to_owned(),
            poly_signature.to_owned(),
            poly_timestamp.to_owned(),
            poly_api_key.to_owned(),
            poly_passphrase.to_owned(),
        ]);
        Err(SyntheticClosedOnlySinkError)
    }
}

fn expected_closed_only_headers() -> [String; 5] {
    [
        ADDRESS.to_owned(),
        "n1obdNq7AuHb1M63PMyCfo6tkGeGkwjGRzkV86-ZtfE=".to_owned(),
        "1780449126".to_owned(),
        API_KEY.to_owned(),
        PASSPHRASE.to_owned(),
    ]
}

fn source_sha256(source: &str) -> String {
    let mut encoded = String::with_capacity(64);
    for byte in Sha256::digest(source.as_bytes()) {
        write!(&mut encoded, "{byte:02x}").expect("writing to String cannot fail");
    }
    encoded
}

impl FixedClosedOnlyRequestSink for ClosedOnlyCapture {
    type Output = OutputDropProbe;
    type Error = Infallible;

    fn send_exact_get_auth_ban_status_closed_only(
        &mut self,
        poly_address: &str,
        poly_signature: &str,
        poly_timestamp: &str,
        poly_api_key: &str,
        poly_passphrase: &str,
    ) -> Result<Self::Output, Self::Error> {
        self.calls += 1;
        self.headers = Some([
            poly_address.to_owned(),
            poly_signature.to_owned(),
            poly_timestamp.to_owned(),
            poly_api_key.to_owned(),
            poly_passphrase.to_owned(),
        ]);
        Ok(self.output.take().expect("one synthetic sink output"))
    }
}

#[test]
fn exact_canonical_tuple_consumes_into_one_redacted_local_equality_holder() {
    let input = response(API_KEY, SECRET, PASSPHRASE);
    assert_eq!(
        format!("{input:?}"),
        "L1CredentialDerivationResponseInput([REDACTED; CALLER_SUPPLIED_BYTES; NO_SOURCE_PROOF])"
    );

    let matched = credentials()
        .consume_with_l1_credential_derivation_response(input)
        .unwrap();
    assert_eq!(
        format!("{matched:?}"),
        "L1CredentialDerivationMatchedL2Credentials([REDACTED; LOCAL_EQUALITY_ONLY; NO_REMOTE_OR_MUTATION_AUTHORITY])"
    );
    for secret_value in [API_KEY, SECRET, PASSPHRASE] {
        assert!(!format!("{matched:?}").contains(secret_value));
    }

    let reordered_with_json_whitespace = format!(
        "  {{ \"passphrase\" : \"{PASSPHRASE}\", \"secret\" : \"{SECRET}\", \"apiKey\" : \"{API_KEY}\" }}\n"
    );
    assert!(join_raw(&reordered_with_json_whitespace).is_ok());

    let escaped_string = L1CredentialDerivationResponseInput::new(
        r#"{"apiKey":"00000000-0000-4000-8000-000000000001","secret":"AA==","passphrase":"opaque\"passphrase"}"#
            .as_bytes()
            .to_vec(),
    )
    .unwrap();
    assert!(
        credentials_with(API_KEY, SECRET, "opaque\"passphrase")
            .consume_with_l1_credential_derivation_response(escaped_string)
            .is_ok()
    );
}

#[test]
fn matched_holder_consumes_into_one_exact_closed_only_hmac_and_retains_sink_output() {
    let timestamp = L2Timestamp::from_unix_seconds(1_780_449_126).unwrap();
    let matched = credentials()
        .consume_with_l1_credential_derivation_response(response(API_KEY, SECRET, PASSPHRASE))
        .unwrap();
    let request = matched
        .consume_into_authenticated_closed_only(timestamp)
        .unwrap();
    let rendered_request = format!("{request:?}");
    assert_eq!(
        rendered_request,
        "L1CredentialDerivationMatchedClosedOnlyRequest([REDACTED; LOCAL_EQUALITY_AND_HMAC_CONSTRUCTION_ONLY; NO_REMOTE_OR_MUTATION_AUTHORITY])"
    );
    for secret_value in [API_KEY, SECRET, PASSPHRASE] {
        assert!(!rendered_request.contains(secret_value));
    }

    let output_dropped = Rc::new(Cell::new(false));
    let mut capture = ClosedOnlyCapture {
        calls: 0,
        headers: None,
        output: Some(OutputDropProbe(Rc::clone(&output_dropped))),
    };
    let dispatched = request.dispatch(&mut capture).unwrap();
    assert_eq!(capture.calls, 1);
    assert_eq!(capture.headers.unwrap(), expected_closed_only_headers());
    assert!(!output_dropped.get());
    let rendered_dispatch = format!("{dispatched:?}");
    assert_eq!(
        rendered_dispatch,
        "L1CredentialDerivationMatchedClosedOnlyDispatch([REDACTED; TRUSTED_SINK_OUTPUT_RETAINED; NO_REMOTE_OR_MUTATION_AUTHORITY])"
    );
    for secret_value in [API_KEY, SECRET, PASSPHRASE] {
        assert!(!rendered_dispatch.contains(secret_value));
    }
    drop(dispatched);
    assert!(output_dropped.get());
}

#[test]
fn sink_error_is_final_for_the_consumed_request_even_when_sink_retains_header_copies() {
    let matched = credentials()
        .consume_with_l1_credential_derivation_response(response(API_KEY, SECRET, PASSPHRASE))
        .unwrap();
    let request = matched
        .consume_into_authenticated_closed_only(
            L2Timestamp::from_unix_seconds(1_780_449_126).unwrap(),
        )
        .unwrap();
    let mut capture = ErroringClosedOnlyCapture {
        calls: 0,
        retained_header_copies: None,
    };

    let error = request.dispatch(&mut capture).unwrap_err();

    assert_eq!(error, SyntheticClosedOnlySinkError);
    assert_eq!(capture.calls, 1);
    assert_eq!(
        capture.retained_header_copies,
        Some(expected_closed_only_headers())
    );
    // `request` was consumed regardless of `Err`; the compile-fail contract
    // separately pins that it cannot be dispatched again or recovered.
}

#[test]
fn response_input_enforces_the_hard_bound_before_parsing() {
    assert_eq!(MAX_L1_CREDENTIAL_DERIVATION_RESPONSE_BYTES, 1024);
    assert!(L1CredentialDerivationResponseInput::new(vec![b' '; 1024]).is_ok());
    assert_eq!(
        L1CredentialDerivationResponseInput::new(vec![b' '; 1025]).unwrap_err(),
        PmAuthError::L1CredentialDerivationResponseTooLong
    );
}

#[test]
fn schema_is_one_exact_object_with_three_required_string_fields() {
    for invalid in [
        "",
        "null",
        "[]",
        r#"{"apiKey":"00000000-0000-4000-8000-000000000001","secret":"AA=="}"#,
        r#"{"apiKey":"00000000-0000-4000-8000-000000000001","passphrase":"synthetic-passphrase"}"#,
        r#"{"secret":"AA==","passphrase":"synthetic-passphrase"}"#,
        r#"{"apiKey":"00000000-0000-4000-8000-000000000001","apiKey":"00000000-0000-4000-8000-000000000001","secret":"AA==","passphrase":"synthetic-passphrase"}"#,
        r#"{"apiKey":"00000000-0000-4000-8000-000000000001","secret":"AA==","secret":"AA==","passphrase":"synthetic-passphrase"}"#,
        r#"{"apiKey":"00000000-0000-4000-8000-000000000001","secret":"AA==","passphrase":"synthetic-passphrase","passphrase":"synthetic-passphrase"}"#,
        r#"{"apiKey":"00000000-0000-4000-8000-000000000001","secret":"AA==","passphrase":"synthetic-passphrase","extra":"value"}"#,
        r#"{"apiKey":null,"secret":"AA==","passphrase":"synthetic-passphrase"}"#,
        r#"{"apiKey":7,"secret":"AA==","passphrase":"synthetic-passphrase"}"#,
        r#"{"apiKey":"00000000-0000-4000-8000-000000000001","secret":null,"passphrase":"synthetic-passphrase"}"#,
        r#"{"apiKey":"00000000-0000-4000-8000-000000000001","secret":[],"passphrase":"synthetic-passphrase"}"#,
        r#"{"apiKey":"00000000-0000-4000-8000-000000000001","secret":"AA==","passphrase":null}"#,
        r#"{"apiKey":"00000000-0000-4000-8000-000000000001","secret":"AA==","passphrase":false}"#,
        r#"{"apiKey":"00000000-0000-4000-8000-000000000001","secret":"AA==","passphrase":"synthetic-passphrase"} true"#,
        r#"{"apiKey":"00000000-0000-4000-8000-000000000001","secret":"AA==","passphrase":"synthetic-passphrase"}garbage"#,
    ] {
        assert_invalid(invalid);
    }
}

#[test]
fn response_values_reuse_the_closed_l2_credential_grammars() {
    for invalid in [
        r#"{"apiKey":"00000000-0000-4000-8000-00000000000A","secret":"AA==","passphrase":"synthetic-passphrase"}"#,
        r#"{"apiKey":"000000000000-4000-8000-000000000001","secret":"AA==","passphrase":"synthetic-passphrase"}"#,
        r#"{"apiKey":"00000000-0000-4000-8000-000000000001","secret":"AA","passphrase":"synthetic-passphrase"}"#,
        r#"{"apiKey":"00000000-0000-4000-8000-000000000001","secret":"++==","passphrase":"synthetic-passphrase"}"#,
        r#"{"apiKey":"00000000-0000-4000-8000-000000000001","secret":"AA==","passphrase":"contains space"}"#,
        r#"{"apiKey":"00000000-0000-4000-8000-000000000001","secret":"AA==","passphrase":""}"#,
    ] {
        assert_invalid(invalid);
    }

    let overlong_passphrase = "x".repeat(129);
    assert_invalid(&format!(
        r#"{{"apiKey":"{API_KEY}","secret":"{SECRET}","passphrase":"{overlong_passphrase}"}}"#
    ));
}

#[test]
fn every_canonical_component_mismatch_has_one_content_free_error() {
    for input in [
        response(OTHER_API_KEY, SECRET, PASSPHRASE),
        response(API_KEY, OTHER_SECRET, PASSPHRASE),
        response(API_KEY, SECRET, OTHER_PASSPHRASE),
    ] {
        let error = credentials()
            .consume_with_l1_credential_derivation_response(input)
            .unwrap_err();
        assert_eq!(error, PmAuthError::L1CredentialDerivationCredentialMismatch);
        for secret_value in [API_KEY, OTHER_API_KEY, SECRET, OTHER_SECRET, PASSPHRASE] {
            assert!(!format!("{error:?} {error}").contains(secret_value));
        }
    }
}

#[test]
fn source_keeps_parsing_joining_and_comparison_narrow_and_non_authorizing() {
    let response_source = include_str!("../src/l1_credential_derivation_response.rs");
    let l2_source = include_str!("../src/l2.rs");
    let lib_source = include_str!("../src/lib.rs");
    let secret_source = include_str!("../src/secret.rs");

    // Exact source pins make the critical type-state and export surfaces fail
    // closed on any code or comment change. Update only after fresh review.
    assert_eq!(
        source_sha256(response_source),
        "f1e58f4056cacc6a1fde5c655cb40eea41d492a4e2f7541b998c694d82d23005"
    );
    assert_eq!(
        source_sha256(l2_source),
        "712c445b03e21a7d82d1bf6a1a7f339561b3e50a93ea3b37c48895af893a2e12"
    );
    assert_eq!(
        source_sha256(lib_source),
        "03d64af8ed27b253e6197f686dd8976732bad4370049a9f63137a7dffaa979a7"
    );

    for required in [
        "caller-supplied bytes",
        "retained, replayed, fabricated, or misrouted bytes",
        "local byte document",
        "does not prove a server call",
        "TLS peer or selected local egress",
        "response currentness or uniqueness",
        "response MIME type",
        "later L2 acceptance",
        "provider delivery",
        "proxy mapping or control",
        "mutation authorization",
        "future private sink",
        "fixed `https://clob.polymarket.com` host",
        "parser can use implementation-owned scratch",
        "cannot zeroize",
        "MAX_L1_CREDENTIAL_DERIVATION_RESPONSE_BYTES: usize = 1024",
        "bytes: Zeroizing<Vec<u8>>",
        "#[serde(deny_unknown_fields)]",
        "#[serde(rename = \"apiKey\")]",
        "struct ZeroizingJsonString(Zeroizing<String>)",
        "serde_json::from_slice",
        "L2CredentialInput::from_zeroizing",
        "Self::bind_to_address",
        "self.matches_exact_l2_bundle(&candidate)",
        "original_l2_credentials: self",
        "consume_into_authenticated_closed_only",
        "FixedClosedOnlyRequestSink",
        "original_l2_credentials.authenticate_exact_closed_only_request(timestamp)",
        "let output = match request.dispatch(sink) {",
        "_original_l2_credentials: original_l2_credentials",
        "_output: output",
        "TRUSTED_SINK_OUTPUT_RETAINED",
        "sink can retain or replay headers, reroute the operation",
        "On `Err`, the exact header",
        "there is no retry or recovery authority",
        "drop(original_l2_credentials)",
        "LOCAL_EQUALITY_ONLY",
        "LOCAL_EQUALITY_AND_HMAC_CONSTRUCTION_ONLY",
        "NO_REMOTE_OR_MUTATION_AUTHORITY",
    ] {
        assert!(
            response_source.contains(required),
            "missing response/join boundary pin: {required}"
        );
    }

    for forbidden in [
        "reqwest::",
        "hyper::",
        "tokio::",
        "TcpStream",
        "std::net",
        "std::fs",
        "std::env",
        "println!",
        "eprintln!",
        "dbg!",
        "tracing::",
        "log::",
        "serde::Serialize",
        "derive(Clone",
        "impl Clone for L1CredentialDerivationResponseInput",
        "impl Clone for L1CredentialDerivationMatchedL2Credentials",
        "pub fn parse",
        "pub fn recover",
        "pub fn split",
        "pub fn credentials",
        "pub fn api_key",
        "pub fn secret",
        "pub fn passphrase",
        "pub fn hash",
        "pub fn response_length",
        "pub fn as_bytes",
        "pub fn into_bytes",
        "pub fn output",
        "pub fn into_output",
        "pub fn original_l2_credentials",
    ] {
        assert!(
            !response_source.contains(forbidden),
            "response/join slice crossed its boundary: {forbidden}"
        );
    }

    for required in [
        "pub trait FixedClosedOnlyRequestSink",
        "fn send_exact_get_auth_ban_status_closed_only(",
        "pub(crate) struct AuthenticatedClosedOnlyRequest(L2Headers)",
        "pub(crate) fn authenticate_exact_closed_only_request(",
        "b\"GET\"",
        "b\"/auth/ban-status/closed-only\"",
        "None",
        "can retain or replay them, route them elsewhere",
        "`Error` has no rollback",
        "Drop the",
        "drop(self)",
        "does not attest transport",
        "remote authentication acceptance",
        "credential currentness",
        "provider origin",
        "proxy control",
        "mutation authorization",
    ] {
        assert!(
            l2_source.contains(required),
            "missing fixed closed-only HMAC/sink boundary pin: {required}"
        );
    }

    for forbidden in [
        "pub fn authenticate_exact_closed_only_request",
        "pub struct AuthenticatedClosedOnlyRequest",
        "fn send_exact_get_auth_ban_status_closed_only(\n        &mut self,\n        method",
        "fn send_exact_get_auth_ban_status_closed_only(\n        &mut self,\n        path",
    ] {
        assert!(
            !l2_source.contains(forbidden),
            "fixed closed-only boundary gained a broader surface: {forbidden}"
        );
    }

    for required in [
        "pub(crate) fn matches_exact_l2_bundle",
        "let api_key_matches = fixed_bound_eq(",
        "let secret_matches = fixed_bound_eq(",
        "let passphrase_matches = fixed_bound_eq(",
        "api_key_matches & secret_matches & passphrase_matches",
        "for index in 0..bound",
        "ordinary Rust indexing",
        "not a formally verified constant-time implementation",
    ] {
        assert!(
            secret_source.contains(required),
            "missing fixed-bound comparison pin: {required}"
        );
    }
}
