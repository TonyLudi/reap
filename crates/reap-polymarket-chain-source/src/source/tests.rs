use std::{
    io::{Read, Write},
    net::TcpListener,
    sync::{Arc, Mutex},
    thread,
    time::Duration,
};

use reap_pm_core::{
    EvmAddress, PmAccountHandle, PmAccountScope, PmChainId, PmEnvironmentId, PmFunderId,
    PmSignerId, U256,
};

use super::*;
use crate::{PmPolygonExchangeSpender, PmPolygonSystemClockObservation};

const BLOCK_TIMESTAMP: u64 = 1_700_000_000;
const BLOCK_TIMESTAMP_HEX: &str = "0x6553f100";
const BLOCK_HASH: &str = "0x1111111111111111111111111111111111111111111111111111111111111111";
const ZERO_WORD: &str = "0x0000000000000000000000000000000000000000000000000000000000000000";
const ONE_WORD: &str = "0x0000000000000000000000000000000000000000000000000000000000000001";
const ALLOWANCE_WORD: &str = "0x00000000000000000000000000000000000000000000000000000000000003e8";

#[derive(Clone)]
struct MockReply {
    status: &'static str,
    body: String,
    delay: Duration,
    declared_length: Option<usize>,
}

impl MockReply {
    fn json(body: impl Into<String>) -> Self {
        Self {
            status: "200 OK",
            body: body.into(),
            delay: Duration::ZERO,
            declared_length: None,
        }
    }
}

#[derive(Debug)]
struct RecordedRequest {
    head: String,
    body: String,
}

struct MockServer {
    origin: String,
    requests: Arc<Mutex<Vec<RecordedRequest>>>,
    worker: thread::JoinHandle<()>,
}

impl MockServer {
    fn spawn(replies: Vec<MockReply>) -> Self {
        let listener = TcpListener::bind(("127.0.0.1", 0)).unwrap();
        let address = listener.local_addr().unwrap();
        let requests = Arc::new(Mutex::new(Vec::new()));
        let worker_requests = Arc::clone(&requests);
        let worker = thread::spawn(move || {
            for reply in replies {
                let (mut stream, _) = listener.accept().unwrap();
                stream
                    .set_read_timeout(Some(Duration::from_secs(2)))
                    .unwrap();
                let request = read_request(&mut stream);
                worker_requests.lock().unwrap().push(request);
                if !reply.delay.is_zero() {
                    thread::sleep(reply.delay);
                }
                let length = reply.declared_length.unwrap_or(reply.body.len());
                let response = format!(
                    "HTTP/1.1 {}\r\nContent-Type: application/json\r\nContent-Length: {length}\r\nConnection: close\r\n\r\n{}",
                    reply.status, reply.body
                );
                let _ = stream.write_all(response.as_bytes());
            }
        });
        Self {
            origin: format!("http://{address}/"),
            requests,
            worker,
        }
    }

    fn finish(self) -> Vec<RecordedRequest> {
        self.worker.join().unwrap();
        Arc::try_unwrap(self.requests)
            .unwrap()
            .into_inner()
            .unwrap()
    }
}

fn read_request(stream: &mut std::net::TcpStream) -> RecordedRequest {
    let mut bytes = Vec::new();
    let header_end = loop {
        if let Some(index) = bytes.windows(4).position(|window| window == b"\r\n\r\n") {
            break index + 4;
        }
        let mut chunk = [0_u8; 4096];
        let length = stream.read(&mut chunk).unwrap();
        assert_ne!(length, 0, "request ended before headers");
        bytes.extend_from_slice(&chunk[..length]);
        assert!(bytes.len() <= 64 * 1024, "request exceeds test bound");
    };
    let head = std::str::from_utf8(&bytes[..header_end])
        .unwrap()
        .to_string();
    let content_length = head
        .lines()
        .find_map(|line| {
            let (name, value) = line.split_once(':')?;
            name.eq_ignore_ascii_case("content-length")
                .then(|| value.trim().parse::<usize>().unwrap())
        })
        .unwrap();
    while bytes.len() - header_end < content_length {
        let mut chunk = [0_u8; 4096];
        let length = stream.read(&mut chunk).unwrap();
        assert_ne!(length, 0, "request ended before body");
        bytes.extend_from_slice(&chunk[..length]);
    }
    RecordedRequest {
        head,
        body: std::str::from_utf8(&bytes[header_end..header_end + content_length])
            .unwrap()
            .to_string(),
    }
}

fn rpc_result(id: u64, result: &str) -> String {
    format!(r#"{{"jsonrpc":"2.0","id":{id},"result":{result}}}"#)
}

fn block_result(id: u64, number: &str, hash: &str, timestamp: &str) -> String {
    rpc_result(
        id,
        &format!(
            r#"{{"number":"{number}","hash":"{hash}","timestamp":"{timestamp}","transactions":[]}}"#
        ),
    )
}

fn successful_replies() -> Vec<MockReply> {
    vec![
        MockReply::json(rpc_result(1, r#""0x89""#)),
        MockReply::json(block_result(2, "0x1234", BLOCK_HASH, BLOCK_TIMESTAMP_HEX)),
        MockReply::json(rpc_result(3, &format!(r#""{ALLOWANCE_WORD}""#))),
        MockReply::json(rpc_result(4, &format!(r#""{ONE_WORD}""#))),
        MockReply::json(block_result(5, "0x1234", BLOCK_HASH, BLOCK_TIMESTAMP_HEX)),
    ]
}

fn proxy_scope(spender: PmPolygonExchangeSpender) -> PmPolygonAuthorizationScope {
    PmPolygonAuthorizationScope::new_pm_t2_proxy(account_scope(137, false), spender).unwrap()
}

fn account_scope(chain: u64, identities_match: bool) -> PmAccountScope {
    let signer = EvmAddress::parse("0x1111111111111111111111111111111111111111").unwrap();
    let funder = if identities_match {
        signer
    } else {
        EvmAddress::parse("0x2222222222222222222222222222222222222222").unwrap()
    };
    PmAccountScope::new(
        PmEnvironmentId::new("pm-chain-evidence").unwrap(),
        PmChainId::new(chain).unwrap(),
        PmSignerId::new(signer),
        PmFunderId::new(funder),
        PmAccountHandle::from_ordinal(7),
    )
}

fn loopback_source(
    origin: &str,
    timestamp: u64,
    timeout: Duration,
) -> PmPolygonAuthorizationSource {
    PmPolygonAuthorizationSource::loopback_evidence(
        origin,
        PmPolygonSystemClockObservation::for_loopback_evidence(timestamp),
        timeout,
    )
    .unwrap()
}

#[tokio::test]
async fn exact_five_request_bodies_produce_one_bound_cut() {
    let server = MockServer::spawn(successful_replies());
    let source = loopback_source(&server.origin, BLOCK_TIMESTAMP + 10, Duration::from_secs(1));
    let scope = proxy_scope(PmPolygonExchangeSpender::StandardV2);

    let cut = source.finalized_authorization_cut(scope).await.unwrap();

    assert_eq!(cut.scope(), scope);
    assert_eq!(
        cut.scope().owner().to_string(),
        "0x2222222222222222222222222222222222222222"
    );
    assert_eq!(cut.block().number(), 0x1234);
    assert_eq!(cut.block().hash(), [0x11; 32]);
    assert_eq!(cut.block().timestamp(), BLOCK_TIMESTAMP);
    assert_eq!(cut.pusd_allowance(), U256::from_u64(1_000));
    assert!(cut.conditional_tokens_approval().is_approved());
    assert_eq!(cut.observed_clock().unix_seconds(), BLOCK_TIMESTAMP + 10);
    assert!(!cut.production_order_entry_authorized());

    let requests = server.finish();
    let bodies = requests
        .iter()
        .map(|request| request.body.as_str())
        .collect::<Vec<_>>();
    assert_eq!(
        bodies,
        [
            r#"{"jsonrpc":"2.0","id":1,"method":"eth_chainId","params":[]}"#,
            r#"{"jsonrpc":"2.0","id":2,"method":"eth_getBlockByNumber","params":["finalized",false]}"#,
            concat!(
                r#"{"jsonrpc":"2.0","id":3,"method":"eth_call","params":[{"to":"0xc011a7e12a19f7b1f670d46f03b03f3342e82dfb","data":"0xdd62ed3e"#,
                "0000000000000000000000002222222222222222222222222222222222222222",
                "000000000000000000000000e111180000d2663c0091e4f400237545b87b996b",
                r#""},"0x1234"]}"#,
            ),
            concat!(
                r#"{"jsonrpc":"2.0","id":4,"method":"eth_call","params":[{"to":"0x4d97dcd97ec945f40cf65f87097ace5ea0476045","data":"0xe985e9c5"#,
                "0000000000000000000000002222222222222222222222222222222222222222",
                "000000000000000000000000e111180000d2663c0091e4f400237545b87b996b",
                r#""},"0x1234"]}"#,
            ),
            r#"{"jsonrpc":"2.0","id":5,"method":"eth_getBlockByNumber","params":["0x1234",false]}"#,
        ]
    );
    for request in requests {
        let lower = request.head.to_ascii_lowercase();
        assert!(lower.starts_with("post / http/1.1\r\n"));
        assert!(lower.contains("accept: application/json\r\n"));
        assert!(lower.contains("content-type: application/json\r\n"));
        assert!(!lower.contains("authorization:"));
        assert!(!lower.contains("proxy-authorization:"));
    }
}

#[tokio::test]
async fn the_other_closed_spender_changes_only_the_frozen_exchange_words() {
    let server = MockServer::spawn(successful_replies());
    let source = loopback_source(&server.origin, BLOCK_TIMESTAMP, Duration::from_secs(1));
    let scope = proxy_scope(PmPolygonExchangeSpender::NegativeRiskV2);
    let cut = source.finalized_authorization_cut(scope).await.unwrap();
    assert_eq!(
        cut.scope().spender().address().to_string(),
        "0xe2222d279d744050d28e00520010520000310f59"
    );
    let requests = server.finish();
    for request in &requests[2..4] {
        assert!(
            request
                .body
                .contains("000000000000000000000000e2222d279d744050d28e00520010520000310f59")
        );
    }
}

#[tokio::test]
async fn wrong_chain_version_id_and_outcome_shapes_are_rejected() {
    let cases = [
        (
            rpc_result(1, r#""0x1""#),
            PmPolygonChainSourceError::WrongRpcChain,
        ),
        (
            rpc_result(2, r#""0x89""#),
            PmPolygonChainSourceError::WrongResponseId {
                expected: 1,
                actual: 2,
            },
        ),
        (
            r#"{"jsonrpc":"1.0","id":1,"result":"0x89"}"#.to_string(),
            PmPolygonChainSourceError::WrongJsonRpcVersion,
        ),
        (
            r#"{"jsonrpc":"2.0","id":1,"result":"0x89","extra":true}"#.to_string(),
            PmPolygonChainSourceError::MalformedJsonRpc,
        ),
        (
            r#"{"jsonrpc":"2.0","id":1,"result":"0x89","error":{"code":3,"message":"reverted"}}"#
                .to_string(),
            PmPolygonChainSourceError::InvalidJsonRpcOutcome,
        ),
    ];
    for (response, expected) in cases {
        let server = MockServer::spawn(vec![MockReply::json(response)]);
        let source = loopback_source(&server.origin, BLOCK_TIMESTAMP, Duration::from_secs(1));
        assert_eq!(
            source
                .finalized_authorization_cut(proxy_scope(PmPolygonExchangeSpender::StandardV2))
                .await
                .unwrap_err(),
            expected
        );
        assert_eq!(server.finish().len(), 1);
    }
}

#[tokio::test]
async fn canonical_quantities_and_block_hash_are_mandatory() {
    for (block, expected) in [
        (
            block_result(2, "0x01234", BLOCK_HASH, BLOCK_TIMESTAMP_HEX),
            PmPolygonChainSourceError::NonCanonicalQuantity,
        ),
        (
            block_result(2, "0x1234", BLOCK_HASH, "0x06553f100"),
            PmPolygonChainSourceError::NonCanonicalQuantity,
        ),
        (
            block_result(
                2,
                "0x1234",
                &BLOCK_HASH.to_ascii_uppercase(),
                BLOCK_TIMESTAMP_HEX,
            ),
            PmPolygonChainSourceError::InvalidBlockHash,
        ),
        (
            block_result(2, "0x1234", ZERO_WORD, BLOCK_TIMESTAMP_HEX),
            PmPolygonChainSourceError::ZeroBlockHash,
        ),
    ] {
        let server = MockServer::spawn(vec![
            MockReply::json(rpc_result(1, r#""0x89""#)),
            MockReply::json(block),
        ]);
        let source = loopback_source(&server.origin, BLOCK_TIMESTAMP, Duration::from_secs(1));
        assert_eq!(
            source
                .finalized_authorization_cut(proxy_scope(PmPolygonExchangeSpender::StandardV2))
                .await
                .unwrap_err(),
            expected
        );
        assert_eq!(server.finish().len(), 2);
    }
}

#[tokio::test]
async fn rpc_revert_returns_no_partial_cut() {
    let mut replies = successful_replies();
    replies.truncate(3);
    replies[2] = MockReply::json(
        r#"{"jsonrpc":"2.0","id":3,"error":{"code":3,"message":"execution reverted","data":"0x08c379a0"}}"#,
    );
    let server = MockServer::spawn(replies);
    let source = loopback_source(&server.origin, BLOCK_TIMESTAMP, Duration::from_secs(1));
    assert_eq!(
        source
            .finalized_authorization_cut(proxy_scope(PmPolygonExchangeSpender::StandardV2))
            .await
            .unwrap_err(),
        PmPolygonChainSourceError::RemoteRpcError { code: 3 }
    );
    assert_eq!(server.finish().len(), 3);
}

#[tokio::test]
async fn allowance_and_approval_require_exact_abi_words() {
    for invalid in [
        r#""0x01""#,
        r#""0X0000000000000000000000000000000000000000000000000000000000000001""#,
    ] {
        let mut replies = successful_replies();
        replies.truncate(3);
        replies[2] = MockReply::json(rpc_result(3, invalid));
        let server = MockServer::spawn(replies);
        let source = loopback_source(&server.origin, BLOCK_TIMESTAMP, Duration::from_secs(1));
        assert_eq!(
            source
                .finalized_authorization_cut(proxy_scope(PmPolygonExchangeSpender::StandardV2))
                .await
                .unwrap_err(),
            PmPolygonChainSourceError::InvalidAllowanceWord
        );
        assert_eq!(server.finish().len(), 3);
    }

    for invalid in [
        r#""0x02""#.to_string(),
        format!(r#""{}2""#, &ZERO_WORD[..ZERO_WORD.len() - 1]),
        format!(r#""{}A""#, &ZERO_WORD[..ZERO_WORD.len() - 1]),
    ] {
        let mut replies = successful_replies();
        replies.truncate(4);
        replies[3] = MockReply::json(rpc_result(4, &invalid));
        let server = MockServer::spawn(replies);
        let source = loopback_source(&server.origin, BLOCK_TIMESTAMP, Duration::from_secs(1));
        let error = source
            .finalized_authorization_cut(proxy_scope(PmPolygonExchangeSpender::StandardV2))
            .await
            .unwrap_err();
        assert!(matches!(
            error,
            PmPolygonChainSourceError::InvalidApprovalWord
                | PmPolygonChainSourceError::NonCanonicalApprovalBoolean
        ));
        assert_eq!(server.finish().len(), 4);
    }
}

#[tokio::test]
async fn false_approval_is_preserved_as_an_exact_observation() {
    let mut replies = successful_replies();
    replies[3] = MockReply::json(rpc_result(4, &format!(r#""{ZERO_WORD}""#)));
    let server = MockServer::spawn(replies);
    let source = loopback_source(&server.origin, BLOCK_TIMESTAMP, Duration::from_secs(1));
    let cut = source
        .finalized_authorization_cut(proxy_scope(PmPolygonExchangeSpender::StandardV2))
        .await
        .unwrap();
    assert!(!cut.conditional_tokens_approval().is_approved());
    assert_eq!(server.finish().len(), 5);
}

#[tokio::test]
async fn exact_block_number_hash_and_timestamp_must_survive_the_reread() {
    let mismatches = [
        block_result(5, "0x1235", BLOCK_HASH, BLOCK_TIMESTAMP_HEX),
        block_result(
            5,
            "0x1234",
            "0x2222222222222222222222222222222222222222222222222222222222222222",
            BLOCK_TIMESTAMP_HEX,
        ),
        block_result(5, "0x1234", BLOCK_HASH, "0x6553f101"),
    ];
    for mismatch in mismatches {
        let mut replies = successful_replies();
        replies[4] = MockReply::json(mismatch);
        let server = MockServer::spawn(replies);
        let source = loopback_source(&server.origin, BLOCK_TIMESTAMP, Duration::from_secs(1));
        assert_eq!(
            source
                .finalized_authorization_cut(proxy_scope(PmPolygonExchangeSpender::StandardV2))
                .await
                .unwrap_err(),
            PmPolygonChainSourceError::FinalizedBlockChanged
        );
        assert_eq!(server.finish().len(), 5);
    }
}

#[tokio::test]
async fn finalized_block_freshness_has_exact_past_and_future_bounds() {
    for (clock, expected) in [
        (BLOCK_TIMESTAMP + 30, None),
        (
            BLOCK_TIMESTAMP + 31,
            Some(PmPolygonChainSourceError::StaleFinalizedBlock),
        ),
        (BLOCK_TIMESTAMP - 5, None),
        (
            BLOCK_TIMESTAMP - 6,
            Some(PmPolygonChainSourceError::FutureFinalizedBlock),
        ),
    ] {
        let server = MockServer::spawn(successful_replies());
        let source = loopback_source(&server.origin, clock, Duration::from_secs(1));
        let result = source
            .finalized_authorization_cut(proxy_scope(PmPolygonExchangeSpender::StandardV2))
            .await;
        match expected {
            Some(error) => assert_eq!(result.unwrap_err(), error),
            None => assert!(result.is_ok()),
        }
        assert_eq!(server.finish().len(), 5);
    }
}

#[tokio::test]
async fn timeout_redirect_and_oversized_response_are_closed_failures() {
    let delayed = MockReply {
        status: "200 OK",
        body: rpc_result(1, r#""0x89""#),
        delay: Duration::from_millis(80),
        declared_length: None,
    };
    let server = MockServer::spawn(vec![delayed]);
    let source = loopback_source(&server.origin, BLOCK_TIMESTAMP, Duration::from_millis(10));
    assert_eq!(
        source
            .finalized_authorization_cut(proxy_scope(PmPolygonExchangeSpender::StandardV2))
            .await
            .unwrap_err(),
        PmPolygonChainSourceError::RequestTimeout
    );
    assert_eq!(server.finish().len(), 1);

    let server = MockServer::spawn(vec![MockReply {
        status: "302 Found",
        body: String::new(),
        delay: Duration::ZERO,
        declared_length: None,
    }]);
    let source = loopback_source(&server.origin, BLOCK_TIMESTAMP, Duration::from_secs(1));
    assert_eq!(
        source
            .finalized_authorization_cut(proxy_scope(PmPolygonExchangeSpender::StandardV2))
            .await
            .unwrap_err(),
        PmPolygonChainSourceError::Redirect { status: 302 }
    );
    assert_eq!(server.finish().len(), 1);

    let server = MockServer::spawn(vec![MockReply {
        status: "200 OK",
        body: String::new(),
        delay: Duration::ZERO,
        declared_length: Some(MAX_JSON_RPC_RESPONSE_BYTES + 1),
    }]);
    let source = loopback_source(&server.origin, BLOCK_TIMESTAMP, Duration::from_secs(1));
    assert_eq!(
        source
            .finalized_authorization_cut(proxy_scope(PmPolygonExchangeSpender::StandardV2))
            .await
            .unwrap_err(),
        PmPolygonChainSourceError::ResponseBodyTooLarge {
            limit: MAX_JSON_RPC_RESPONSE_BYTES
        }
    );
    assert_eq!(server.finish().len(), 1);
}

#[test]
fn account_scope_cannot_drift_from_polygon_proxy_funder_semantics() {
    assert_eq!(
        PmPolygonAuthorizationScope::new_pm_t2_proxy(
            account_scope(1, false),
            PmPolygonExchangeSpender::StandardV2,
        )
        .unwrap_err(),
        PmPolygonChainSourceError::WrongAccountChain
    );
    assert_eq!(
        PmPolygonAuthorizationScope::new_pm_t2_proxy(
            account_scope(137, true),
            PmPolygonExchangeSpender::StandardV2,
        )
        .unwrap_err(),
        PmPolygonChainSourceError::SignerFunderNotDistinct
    );
    let scope = proxy_scope(PmPolygonExchangeSpender::StandardV2);
    assert_eq!(scope.owner(), scope.account_scope().funder().address());
    assert_ne!(scope.owner(), scope.account_scope().signer().address());
}

#[test]
fn only_canonical_numeric_loopback_origins_and_bounded_timeouts_are_accepted() {
    let clock = PmPolygonSystemClockObservation::for_loopback_evidence(BLOCK_TIMESTAMP);
    for invalid in [
        "http://localhost:1234/",
        "http://192.0.2.1:1234/",
        "https://127.0.0.1:1234/",
        "http://127.0.0.1/",
        "http://127.0.0.1:1234/path",
        "http://127.0.0.1:1234/?query=1",
        "http://user@127.0.0.1:1234/",
        "http://127.1:1234/",
    ] {
        assert!(matches!(
            PmPolygonAuthorizationSource::loopback_evidence(invalid, clock, Duration::from_secs(1)),
            Err(PmPolygonChainSourceError::InvalidLoopbackOrigin)
        ));
    }
    for timeout in [Duration::ZERO, Duration::from_millis(5_001)] {
        assert!(matches!(
            PmPolygonAuthorizationSource::loopback_evidence(
                "http://127.0.0.1:1234/",
                clock,
                timeout
            ),
            Err(PmPolygonChainSourceError::InvalidLoopbackTimeout)
        ));
    }
    assert!(
        PmPolygonAuthorizationSource::loopback_evidence(
            "http://127.0.0.1:1234/",
            clock,
            Duration::from_millis(1)
        )
        .is_ok()
    );
    assert!(
        PmPolygonAuthorizationSource::loopback_evidence(
            "http://[::1]:1234/",
            clock,
            Duration::from_secs(5)
        )
        .is_ok()
    );
}

#[test]
fn production_constructor_has_one_fixed_https_origin_and_system_clock() {
    let source = PmPolygonAuthorizationSource::production().unwrap();
    assert_eq!(
        source.transport.origin.as_str(),
        "https://polygon.drpc.org/"
    );
    assert!(matches!(source.clock, ClockSource::System));
}
