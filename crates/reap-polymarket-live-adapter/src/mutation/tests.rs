use std::time::Duration;

use reap_pm_core::{
    EvmAddress, PmOrderSalt, PmOrderSide, PmPrice, PmQuantity, PmTick, PmTokenId, U256,
};
use reap_polymarket_auth::{
    EoaPrivateKeyInput, FixedEoaSigner, FixedOrderId, L2CredentialInput, L2Credentials,
    L2Timestamp, PmClobDomain,
};
use reap_polymarket_wire::PmUnsignedClobV2Order;
use tokio::{
    io::{AsyncReadExt, AsyncWriteExt},
    net::TcpListener,
    sync::mpsc,
    task::JoinHandle,
    time::{sleep, timeout},
};

use super::*;
use crate::mutation::{
    retained::MAX_RETAINED_PLACE_BODY_BYTES, transport::MAX_MUTATION_RESPONSE_BYTES,
};

const TEST_KEY: &str = "0xac0974bec39a17e36ba4a6b4d238ff944bacb478cbed5efcae784d7bf4f2ff80";
const ADDRESS: &str = "0xf39Fd6e51aad88F6F4ce6aB8827279cffFb92266";
const API_KEY: &str = "00000000-0000-4000-8000-000000000001";
const ROTATED_API_KEY: &str = "00000000-0000-4000-8000-000000000002";
const API_SECRET: &str = "AAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAA=";
const ROTATED_API_SECRET: &str = "AQEBAQEBAQEBAQEBAQEBAQEBAQEBAQEBAQEBAQEBAQE=";
const PASSPHRASE: &str = "synthetic-passphrase";
const ROTATED_PASSPHRASE: &str = "rotated-synthetic-passphrase";
const AUTH_SECONDS: u64 = 1_780_449_126;
const EXPECTED_ID: &str = "0xfaf10599783c69b375a0f0d948d37eb711ec042dbf7d52fc2f8d8832d71af7f1";
const FOREIGN_ID: &str = "0xbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbb";
const EXPECTED_PLACE_SIGNATURE: &str = "rdkpCVcu-66xB2VbkOlUXQ2PLaCeqv3LjgFBfkrQdqo=";
const EXPECTED_CANCEL_SIGNATURE: &str = "beMewzBZba3V05Un8qQtGM0NYW4KFt4wKc8KqZcdo_M=";
const EXPECTED_PLACE_BODY: &str = r#"{"deferExec":false,"order":{"builder":"0x0000000000000000000000000000000000000000000000000000000000000000","expiration":"0","maker":"0xf39Fd6e51aad88F6F4ce6aB8827279cffFb92266","makerAmount":"5200000","metadata":"0x0000000000000000000000000000000000000000000000000000000000000000","salt":479249096354,"side":"BUY","signature":"0xbb81b245ea7ebb9aa480ccbf15364a2cb2cd77d7adebcb56fd5f49b653683110055a3d5ad05adf1aa65b1701bf25c622275f098fd5724c7f782671829e6d4d0b1b","signatureType":0,"signer":"0xf39Fd6e51aad88F6F4ce6aB8827279cffFb92266","takerAmount":"10000000","timestamp":"1780449126930","tokenId":"1234"},"orderType":"GTC","owner":"00000000-0000-4000-8000-000000000001","postOnly":true}"#;
const EXPECTED_CANCEL_BODY: &str =
    r#"{"orderID":"0xfaf10599783c69b375a0f0d948d37eb711ec042dbf7d52fc2f8d8832d71af7f1"}"#;

// Response shapes are frozen against the current official place/manage order
// pages and corroborated only for routes/body keys by pinned Predarb object
// 8222273a9c72033b760e1d2fec813bc77144556d.
fn place_success(id: &str, status: &str, trade_ids: &str) -> String {
    place_success_amounts(id, status, trade_ids, "5200000", "10000000")
}

fn place_success_amounts(
    id: &str,
    status: &str,
    trade_ids: &str,
    making_amount: &str,
    taking_amount: &str,
) -> String {
    format!(
        r#"{{"success":true,"errorMsg":"","orderID":"{id}","status":"{status}","makingAmount":"{making_amount}","takingAmount":"{taking_amount}","transactionsHashes":[],"tradeIDs":{trade_ids}}}"#
    )
}

fn place_rejection(reason: &str) -> String {
    format!(
        r#"{{"success":false,"errorMsg":"{reason}","orderID":"","status":"","makingAmount":"","takingAmount":"","transactionsHashes":[],"tradeIDs":[]}}"#
    )
}

fn cancel_success(id: &str) -> String {
    format!(r#"{{"canceled":["{id}"],"not_canceled":{{}}}}"#)
}

fn cancel_rejection(id: &str, reason: &str) -> String {
    format!(r#"{{"canceled":[],"not_canceled":{{"{id}":"{reason}"}}}}"#)
}

fn signer() -> FixedEoaSigner {
    FixedEoaSigner::bind(EoaPrivateKeyInput::new(TEST_KEY.into()), ADDRESS).unwrap()
}

fn credentials() -> L2Credentials {
    credentials_with_passphrase(PASSPHRASE)
}

fn credentials_with_passphrase(passphrase: &str) -> L2Credentials {
    credentials_with_values(API_KEY, API_SECRET, passphrase)
}

fn credentials_with_values(api_key: &str, api_secret: &str, passphrase: &str) -> L2Credentials {
    L2Credentials::bind(
        ADDRESS,
        L2CredentialInput::new(api_key.into(), api_secret.into(), passphrase.into()),
    )
    .unwrap()
}

fn unsigned_order() -> PmUnsignedClobV2Order {
    let maker = EvmAddress::parse(ADDRESS).unwrap();
    PmUnsignedClobV2Order::new_goal_f(
        PmOrderSalt::from_u64(479_249_096_354).unwrap(),
        maker,
        maker,
        PmTokenId::new(U256::from_u64(1_234)).unwrap(),
        PmOrderSide::Buy,
        PmPrice::parse_decimal("0.52").unwrap(),
        PmQuantity::parse_decimal("10").unwrap(),
        PmTick::parse_decimal("0.01").unwrap(),
        PmQuantity::parse_decimal("5").unwrap(),
        1_780_449_126_930,
    )
    .unwrap()
}

fn retained_place() -> PmRetainedPlaceRequest {
    retained_place_with_passphrase(PASSPHRASE)
}

fn retained_place_with_passphrase(passphrase: &str) -> PmRetainedPlaceRequest {
    retained_place_with_credentials(API_KEY, API_SECRET, passphrase)
}

fn retained_place_with_credentials(
    api_key: &str,
    api_secret: &str,
    passphrase: &str,
) -> PmRetainedPlaceRequest {
    let signer = signer();
    let credentials = credentials_with_values(api_key, api_secret, passphrase);
    let signed = signer
        .sign_clob_v2_order(PmClobDomain::Standard, unsigned_order())
        .unwrap();
    let body = credentials.serialize_gtc_post_only(signed).unwrap();
    let authenticated = credentials
        .authenticate_place(L2Timestamp::from_unix_seconds(AUTH_SECONDS).unwrap(), body)
        .unwrap();
    PmRetainedPlaceRequest::retain(authenticated).unwrap()
}

fn retained_cancel() -> PmRetainedOwnedCancelRequest {
    let credentials = credentials();
    let order_id = FixedOrderId::parse(EXPECTED_ID).unwrap();
    let body = credentials.serialize_owned_cancel(order_id).unwrap();
    let authenticated = credentials
        .authenticate_owned_cancel(L2Timestamp::from_unix_seconds(AUTH_SECONDS).unwrap(), body)
        .unwrap();
    PmRetainedOwnedCancelRequest::retain(authenticated).unwrap()
}

enum MockPlan {
    Complete {
        status: u16,
        body: Vec<u8>,
        delay: Duration,
        declared_length: Option<usize>,
    },
    Partial {
        status: u16,
        body_prefix: Vec<u8>,
        declared_length: usize,
    },
    Disconnect,
}

impl MockPlan {
    fn ok(body: impl Into<Vec<u8>>) -> Self {
        let body = body.into();
        Self::Complete {
            status: 200,
            declared_length: Some(body.len()),
            body,
            delay: Duration::ZERO,
        }
    }

    fn status(status: u16, body: impl Into<Vec<u8>>) -> Self {
        let body = body.into();
        Self::Complete {
            status,
            declared_length: Some(body.len()),
            body,
            delay: Duration::ZERO,
        }
    }
}

async fn mock_server(
    plan: MockPlan,
) -> (String, mpsc::UnboundedReceiver<Vec<u8>>, JoinHandle<usize>) {
    let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let address = listener.local_addr().unwrap();
    let (requests_tx, requests_rx) = mpsc::unbounded_channel();
    let task = tokio::spawn(async move {
        let Ok(Ok((mut stream, _))) = timeout(Duration::from_secs(2), listener.accept()).await
        else {
            return 0;
        };
        let raw = read_request(&mut stream).await;
        let _ = requests_tx.send(raw);
        match plan {
            MockPlan::Complete {
                status,
                body,
                delay,
                declared_length,
            } => {
                sleep(delay).await;
                let length = declared_length.unwrap_or(body.len());
                let response = format!(
                    "HTTP/1.1 {status} {}\r\nContent-Type: application/json\r\nContent-Length: {length}\r\n{}\r\n",
                    reason(status),
                    if (300..400).contains(&status) {
                        "Location: /must-not-follow\r\n"
                    } else {
                        ""
                    }
                );
                if stream.write_all(response.as_bytes()).await.is_ok() {
                    let _ = stream.write_all(&body).await;
                }
            }
            MockPlan::Partial {
                status,
                body_prefix,
                declared_length,
            } => {
                let response = format!(
                    "HTTP/1.1 {status} {}\r\nContent-Type: application/json\r\nContent-Length: {declared_length}\r\n\r\n",
                    reason(status)
                );
                if stream.write_all(response.as_bytes()).await.is_ok() {
                    let _ = stream.write_all(&body_prefix).await;
                }
            }
            MockPlan::Disconnect => {}
        }
        drop(stream);
        if let Ok(Ok((_unexpected, _))) =
            timeout(Duration::from_millis(100), listener.accept()).await
        {
            2
        } else {
            1
        }
    });
    (format!("http://{address}"), requests_rx, task)
}

async fn read_request(stream: &mut tokio::net::TcpStream) -> Vec<u8> {
    let mut raw = Vec::new();
    let mut chunk = [0_u8; 1_024];
    let mut expected = None;
    loop {
        let read = stream.read(&mut chunk).await.unwrap();
        if read == 0 {
            break;
        }
        raw.extend_from_slice(&chunk[..read]);
        if expected.is_none()
            && let Some(header_end) = raw.windows(4).position(|window| window == b"\r\n\r\n")
        {
            let header_end = header_end + 4;
            let head = std::str::from_utf8(&raw[..header_end]).unwrap();
            let content_length = head
                .lines()
                .find_map(|line| {
                    let (name, value) = line.split_once(':')?;
                    name.eq_ignore_ascii_case("content-length")
                        .then(|| value.trim().parse::<usize>().unwrap())
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

fn reason(status: u16) -> &'static str {
    match status {
        200 => "OK",
        307 => "Temporary Redirect",
        400 => "Bad Request",
        401 => "Unauthorized",
        408 => "Request Timeout",
        409 => "Conflict",
        422 => "Unprocessable Entity",
        425 => "Too Early",
        429 => "Too Many Requests",
        503 => "Service Unavailable",
        _ => "Mock",
    }
}

fn loopback_config(origin: &str, request_timeout: Duration) -> PmLoopbackMutationConfig {
    PmLoopbackMutationConfig::loopback_evidence(origin, Duration::from_millis(100), request_timeout)
        .unwrap()
}

fn request_body(raw: &[u8]) -> &[u8] {
    let offset = raw
        .windows(4)
        .position(|window| window == b"\r\n\r\n")
        .unwrap()
        + 4;
    &raw[offset..]
}

fn request_header<'a>(raw: &'a [u8], expected_name: &str) -> &'a str {
    let head_end = raw
        .windows(4)
        .position(|window| window == b"\r\n\r\n")
        .unwrap();
    std::str::from_utf8(&raw[..head_end])
        .unwrap()
        .lines()
        .find_map(|line| {
            let (name, value) = line.split_once(':')?;
            name.eq_ignore_ascii_case(expected_name)
                .then(|| value.trim())
        })
        .unwrap()
}

fn lower_hex(bytes: [u8; 32]) -> String {
    const DIGITS: &[u8; 16] = b"0123456789abcdef";
    let mut encoded = String::with_capacity(64);
    for byte in bytes {
        encoded.push(char::from(DIGITS[usize::from(byte >> 4)]));
        encoded.push(char::from(DIGITS[usize::from(byte & 0x0f)]));
    }
    encoded
}

#[test]
fn mutation_config_is_literal_loopback_only_and_retention_is_redacted() {
    let connect = Duration::from_millis(10);
    let request = Duration::from_millis(20);
    for forbidden in [
        "https://127.0.0.1:1",
        "http://localhost:1",
        "http://127.0.0.1",
        "http://127.0.0.1:1/path",
        "http://127.0.0.1:1?query=x",
        "http://user@127.0.0.1:1",
        "http://192.0.2.1:1",
    ] {
        assert!(
            PmLoopbackMutationConfig::loopback_evidence(forbidden, connect, request).is_err(),
            "accepted forbidden mutation origin {forbidden}"
        );
    }
    assert!(
        PmLoopbackMutationConfig::loopback_evidence("http://127.0.0.1:1", Duration::ZERO, request)
            .is_err()
    );

    let place = retained_place();
    let cancel = retained_cancel();
    assert_eq!(place.expected_order_id().to_string(), EXPECTED_ID);
    assert_eq!(place.expected_making_amount(), U256::from_u64(5_200_000));
    assert_eq!(place.expected_taking_amount(), U256::from_u64(10_000_000));
    assert_eq!(cancel.order_id().to_string(), EXPECTED_ID);
    assert_eq!(place.l2_timestamp_seconds(), AUTH_SECONDS);
    assert_eq!(cancel.l2_timestamp_seconds(), AUTH_SECONDS);
    assert_eq!(
        lower_hex(place.runtime_exact_body_commitment().runtime_only_bytes()),
        "6a016c150aa3605270289489beff1d54d735952db64948d2bb3c1e01beb0ae9f"
    );
    assert_eq!(
        lower_hex(cancel.runtime_exact_body_commitment().runtime_only_bytes()),
        "90c066349138ab5e31d12e10006859cb239f8cbab5b46e5b0bec00c974536022"
    );
    assert_eq!(place.semantic_request_commitment().to_string().len(), 66);
    assert_eq!(cancel.semantic_request_commitment().to_string().len(), 66);
    let debug = format!("{place:?} {cancel:?}");
    for secret in [API_KEY, PASSPHRASE, EXPECTED_PLACE_SIGNATURE, TEST_KEY] {
        assert!(!debug.contains(secret));
    }
    assert!(debug.contains("[REDACTED]"));
    assert!(debug.contains("[REDACTED; NON_DURABLE]"));
    assert!(!debug.contains("6a016c150aa3605270289489beff1d54"));
    assert!(!debug.contains("90c066349138ab5e31d12e10006859cb"));
}

#[tokio::test]
async fn place_transports_exact_once_serialized_bytes_only_after_send_and_only_once() {
    let (origin, mut requests, task) =
        mock_server(MockPlan::ok(place_success(EXPECTED_ID, "live", "[]"))).await;
    let retained = retained_place();
    let runtime_exact_body_commitment = retained.runtime_exact_body_commitment();
    let semantic_request_commitment = retained.semantic_request_commitment();
    let mut role =
        PmFixedPlaceLoopbackRole::new(loopback_config(&origin, Duration::from_secs(1))).unwrap();

    assert!(
        timeout(Duration::from_millis(30), requests.recv())
            .await
            .is_err(),
        "retention or role construction wrote to the socket"
    );
    let outcome = role.send(retained).await;
    assert_eq!(outcome.classification(), PmMutationClassification::Accepted);
    assert_eq!(outcome.expected_order_id().to_string(), EXPECTED_ID);
    assert_eq!(outcome.observed_order_id().unwrap().as_str(), EXPECTED_ID);
    assert_eq!(
        outcome.runtime_exact_body_commitment(),
        runtime_exact_body_commitment
    );
    assert_eq!(
        outcome.semantic_request_commitment(),
        semantic_request_commitment
    );
    assert_eq!(
        outcome.diagnostic().kind(),
        PmMutationDiagnosticKind::AcceptedProfile
    );

    let raw = requests.recv().await.unwrap();
    assert_eq!(
        std::str::from_utf8(&raw).unwrap().lines().next().unwrap(),
        "POST /order HTTP/1.1"
    );
    assert_eq!(request_header(&raw, "poly_address"), ADDRESS);
    assert_eq!(
        request_header(&raw, "poly_signature"),
        EXPECTED_PLACE_SIGNATURE
    );
    assert_eq!(
        request_header(&raw, "poly_timestamp"),
        AUTH_SECONDS.to_string()
    );
    assert_eq!(request_header(&raw, "poly_api_key"), API_KEY);
    assert_eq!(request_header(&raw, "poly_passphrase"), PASSPHRASE);
    assert_eq!(request_header(&raw, "content-type"), "application/json");
    assert_eq!(request_body(&raw), EXPECTED_PLACE_BODY.as_bytes());
    assert_eq!(task.await.unwrap(), 1, "transport attempted more than once");
}

#[tokio::test]
async fn visible_ascii_passphrase_reaches_the_fixed_header_sink_exactly() {
    let passphrase = "opaque+/=passphrase";
    let (origin, mut requests, task) =
        mock_server(MockPlan::ok(place_success(EXPECTED_ID, "live", "[]"))).await;
    let mut role =
        PmFixedPlaceLoopbackRole::new(loopback_config(&origin, Duration::from_secs(1))).unwrap();
    let outcome = role.send(retained_place_with_passphrase(passphrase)).await;
    assert_eq!(outcome.classification(), PmMutationClassification::Accepted);
    let raw = requests.recv().await.unwrap();
    assert_eq!(request_header(&raw, "poly_passphrase"), passphrase);
    assert_eq!(task.await.unwrap(), 1);
}

#[tokio::test]
async fn cancel_transports_exact_body_and_classifies_exact_identity() {
    let (origin, mut requests, task) = mock_server(MockPlan::ok(cancel_success(EXPECTED_ID))).await;
    let retained = retained_cancel();
    let runtime_exact_body_commitment = retained.runtime_exact_body_commitment();
    let semantic_request_commitment = retained.semantic_request_commitment();
    let mut role =
        PmExactOwnedCancelLoopbackRole::new(loopback_config(&origin, Duration::from_secs(1)))
            .unwrap();
    let outcome = role.send(retained).await;
    assert_eq!(outcome.classification(), PmMutationClassification::Accepted);
    assert_eq!(outcome.order_id().to_string(), EXPECTED_ID);
    assert_eq!(outcome.observed_order_id().unwrap().as_str(), EXPECTED_ID);
    assert_eq!(
        outcome.runtime_exact_body_commitment(),
        runtime_exact_body_commitment
    );
    assert_eq!(
        outcome.semantic_request_commitment(),
        semantic_request_commitment
    );

    let raw = requests.recv().await.unwrap();
    assert_eq!(
        std::str::from_utf8(&raw).unwrap().lines().next().unwrap(),
        "DELETE /order HTTP/1.1"
    );
    assert_eq!(
        request_header(&raw, "poly_signature"),
        EXPECTED_CANCEL_SIGNATURE
    );
    assert_eq!(request_body(&raw), EXPECTED_CANCEL_BODY.as_bytes());
    assert_eq!(task.await.unwrap(), 1);
}

#[tokio::test]
async fn place_outcome_correlates_exact_consumed_body_but_not_rotated_credentials() {
    let first = retained_place();
    let rotated =
        retained_place_with_credentials(ROTATED_API_KEY, ROTATED_API_SECRET, ROTATED_PASSPHRASE);
    let first_runtime = first.runtime_exact_body_commitment();
    let rotated_runtime = rotated.runtime_exact_body_commitment();
    let semantic = first.semantic_request_commitment();
    assert_ne!(first_runtime, rotated_runtime);
    assert_eq!(semantic, rotated.semantic_request_commitment());
    assert_ne!(first.body.as_slice(), rotated.body.as_slice());

    for (retained, expected_runtime) in [(first, first_runtime), (rotated, rotated_runtime)] {
        let exact_consumed_body = retained.body.as_slice().to_vec();
        let (origin, mut requests, task) =
            mock_server(MockPlan::ok(place_success(EXPECTED_ID, "live", "[]"))).await;
        let mut role =
            PmFixedPlaceLoopbackRole::new(loopback_config(&origin, Duration::from_secs(1)))
                .unwrap();

        let outcome = role.send(retained).await;
        let raw = requests.recv().await.unwrap();
        assert_eq!(request_body(&raw), exact_consumed_body);
        assert_eq!(outcome.runtime_exact_body_commitment(), expected_runtime);
        assert_eq!(outcome.semantic_request_commitment(), semantic);

        let runtime_hex = lower_hex(expected_runtime.runtime_only_bytes());
        let debug = format!("{outcome:?}");
        assert!(!debug.contains(&runtime_hex));
        assert!(debug.contains("[REDACTED; NON_DURABLE]"));
        assert_eq!(task.await.unwrap(), 1);
    }
}

#[tokio::test]
async fn place_response_matrix_is_conservative_and_retains_primary_evidence() {
    let cases = [
        (
            MockPlan::ok(place_rejection("not enough balance / allowance")),
            PmMutationClassification::Rejected,
            PmMutationDiagnosticKind::VenueRejected,
            Some("not enough balance / allowance"),
        ),
        (
            MockPlan::status(422, place_rejection("invalid order")),
            PmMutationClassification::Rejected,
            PmMutationDiagnosticKind::VenueRejected,
            Some("invalid order"),
        ),
        (
            MockPlan::ok(place_success(FOREIGN_ID, "live", "[]")),
            PmMutationClassification::OutOfProfile,
            PmMutationDiagnosticKind::ResponseIdentityMismatch,
            None,
        ),
        (
            MockPlan::ok(place_success(EXPECTED_ID, "matched", r#"["trade-1"]"#)),
            PmMutationClassification::OutOfProfile,
            PmMutationDiagnosticKind::ResponseProfileMismatch,
            None,
        ),
        (
            MockPlan::ok(place_success_amounts(
                EXPECTED_ID,
                "live",
                "[]",
                "10000000",
                "5200000",
            )),
            PmMutationClassification::OutOfProfile,
            PmMutationDiagnosticKind::ResponseProfileMismatch,
            None,
        ),
        (
            MockPlan::ok(place_success_amounts(
                EXPECTED_ID,
                "live",
                "[]",
                "5200001",
                "10000000",
            )),
            PmMutationClassification::OutOfProfile,
            PmMutationDiagnosticKind::ResponseProfileMismatch,
            None,
        ),
        (
            MockPlan::ok(b"{}".to_vec()),
            PmMutationClassification::AcknowledgementUnknown,
            PmMutationDiagnosticKind::MalformedResponse,
            None,
        ),
        (
            MockPlan::status(307, Vec::new()),
            PmMutationClassification::OutOfProfile,
            PmMutationDiagnosticKind::Redirect,
            None,
        ),
        (
            MockPlan::status(401, b"{}".to_vec()),
            PmMutationClassification::OutOfProfile,
            PmMutationDiagnosticKind::AuthenticationInvalid,
            None,
        ),
        (
            MockPlan::status(409, b"{}".to_vec()),
            PmMutationClassification::OutOfProfile,
            PmMutationDiagnosticKind::ReconciliationRequiredStatus,
            None,
        ),
        (
            MockPlan::status(429, b"{}".to_vec()),
            PmMutationClassification::OutOfProfile,
            PmMutationDiagnosticKind::ReconciliationRequiredStatus,
            None,
        ),
        (
            MockPlan::status(503, b"{}".to_vec()),
            PmMutationClassification::AcknowledgementUnknown,
            PmMutationDiagnosticKind::UnexpectedHttpStatus,
            None,
        ),
    ];

    for (plan, classification, diagnostic, reason) in cases {
        let (origin, _requests, task) = mock_server(plan).await;
        let mut role =
            PmFixedPlaceLoopbackRole::new(loopback_config(&origin, Duration::from_secs(1)))
                .unwrap();
        let outcome = role.send(retained_place()).await;
        assert_eq!(outcome.classification(), classification);
        assert_eq!(outcome.diagnostic().kind(), diagnostic);
        assert_eq!(outcome.rejection_reason(), reason);
        assert_eq!(outcome.expected_order_id().to_string(), EXPECTED_ID);
        if let Some(reason) = reason {
            let debug = format!("{outcome:?}");
            assert!(!debug.contains(reason));
            assert!(debug.contains("[REDACTED]"));
        }
        assert_eq!(task.await.unwrap(), 1);
    }
}

#[tokio::test]
async fn cancel_rejection_and_contradictory_or_foreign_shapes_fail_closed() {
    let foreign_mix =
        format!(r#"{{"canceled":["{EXPECTED_ID}"],"not_canceled":{{"{FOREIGN_ID}":"foreign"}}}}"#);
    let cases = [
        (
            MockPlan::ok(cancel_rejection(EXPECTED_ID, "already matched")),
            PmMutationClassification::Rejected,
            PmMutationDiagnosticKind::VenueRejected,
            Some("already matched"),
        ),
        (
            MockPlan::ok(cancel_success(FOREIGN_ID)),
            PmMutationClassification::OutOfProfile,
            PmMutationDiagnosticKind::ResponseIdentityMismatch,
            None,
        ),
        (
            MockPlan::ok(foreign_mix),
            PmMutationClassification::OutOfProfile,
            PmMutationDiagnosticKind::ResponseProfileMismatch,
            None,
        ),
        (
            MockPlan::ok(b"{}".to_vec()),
            PmMutationClassification::AcknowledgementUnknown,
            PmMutationDiagnosticKind::MalformedResponse,
            None,
        ),
    ];
    for (plan, classification, diagnostic, reason) in cases {
        let (origin, _requests, task) = mock_server(plan).await;
        let mut role =
            PmExactOwnedCancelLoopbackRole::new(loopback_config(&origin, Duration::from_secs(1)))
                .unwrap();
        let outcome = role.send(retained_cancel()).await;
        assert_eq!(outcome.classification(), classification);
        assert_eq!(outcome.diagnostic().kind(), diagnostic);
        assert_eq!(outcome.rejection_reason(), reason);
        assert_eq!(task.await.unwrap(), 1);
    }
}

#[tokio::test]
async fn timeout_and_oversize_are_never_reusable_rejections() {
    let oversized = MockPlan::Complete {
        status: 200,
        body: Vec::new(),
        delay: Duration::ZERO,
        declared_length: Some(MAX_MUTATION_RESPONSE_BYTES + 1),
    };
    let delayed = MockPlan::Complete {
        status: 200,
        body: place_success(EXPECTED_ID, "live", "[]").into_bytes(),
        delay: Duration::from_millis(150),
        declared_length: None,
    };
    let cases = [
        (
            oversized,
            PmMutationDiagnosticKind::ResponseTooLarge,
            Duration::from_secs(1),
        ),
        (
            delayed,
            PmMutationDiagnosticKind::RequestTimeout,
            Duration::from_millis(40),
        ),
    ];
    for (plan, diagnostic, request_timeout) in cases {
        let (origin, _requests, task) = mock_server(plan).await;
        let mut role =
            PmFixedPlaceLoopbackRole::new(loopback_config(&origin, request_timeout)).unwrap();
        let outcome = role.send(retained_place()).await;
        assert_eq!(
            outcome.classification(),
            PmMutationClassification::AcknowledgementUnknown
        );
        assert_eq!(outcome.diagnostic().kind(), diagnostic);
        assert_eq!(task.await.unwrap(), 1);
    }
}

#[tokio::test]
async fn disconnect_and_partial_response_are_exactly_one_transport_attempt() {
    let cases = [
        (
            MockPlan::Partial {
                status: 200,
                body_prefix: b"{\"success\":".to_vec(),
                declared_length: 100,
            },
            PmMutationDiagnosticKind::ResponseBodyFailure,
        ),
        (
            MockPlan::Disconnect,
            PmMutationDiagnosticKind::TransportFailure,
        ),
    ];
    for (plan, diagnostic) in cases {
        let (origin, mut requests, task) = mock_server(plan).await;
        let mut role =
            PmFixedPlaceLoopbackRole::new(loopback_config(&origin, Duration::from_secs(1)))
                .unwrap();
        let outcome = role.send(retained_place()).await;
        assert_eq!(
            outcome.classification(),
            PmMutationClassification::AcknowledgementUnknown
        );
        assert_eq!(outcome.diagnostic().kind(), diagnostic);
        assert!(
            requests.recv().await.is_some(),
            "request never reached server"
        );
        assert_eq!(
            task.await.unwrap(),
            1,
            "mutation transport retried after the may-have-sent boundary"
        );
    }
}

#[tokio::test]
async fn retained_integrity_failure_is_definitely_unsent() {
    let (origin, mut requests, task) =
        mock_server(MockPlan::ok(place_success(EXPECTED_ID, "live", "[]"))).await;
    let mut retained = retained_place();
    retained
        .body
        .resize(MAX_RETAINED_PLACE_BODY_BYTES + 1, b'x');
    let mut role =
        PmFixedPlaceLoopbackRole::new(loopback_config(&origin, Duration::from_secs(1))).unwrap();
    let outcome = role.send(retained).await;
    assert_eq!(
        outcome.classification(),
        PmMutationClassification::DefinitelyNotDispatched
    );
    assert_eq!(
        outcome.diagnostic().kind(),
        PmMutationDiagnosticKind::PreSendValidation
    );
    assert!(
        timeout(Duration::from_millis(150), requests.recv())
            .await
            .is_err()
    );
    assert_eq!(task.await.unwrap(), 0);
}
