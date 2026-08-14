use std::collections::BTreeMap;
use std::io::{ErrorKind, Read as _, Write as _};
use std::net::{IpAddr, Shutdown, SocketAddr, TcpListener};
use std::sync::mpsc::{self, Receiver, Sender};
use std::thread::{self, JoinHandle};
use std::time::{Duration, Instant};

use reap_polymarket_auth::{
    AuthenticatedL1CredentialDerivationRequest, EoaPrivateKeyInput, FixedEoaSigner,
    L1CredentialDerivationNonce, L1CredentialDerivationResponseInput,
    L1CredentialDerivationTimestamp,
};

use super::*;

// All protocol values are published synthetic fixtures. These tests never
// load credentials or leave literal loopback addresses.
const SYNTHETIC_KEY: &str = "0xac0974bec39a17e36ba4a6b4d238ff944bacb478cbed5efcae784d7bf4f2ff80";
const SYNTHETIC_ADDRESS: &str = "0xf39Fd6e51aad88F6F4ce6aB8827279cffFb92266";
const SYNTHETIC_SIGNATURE: &str = concat!(
    "0x9670627f2da09dc111b4044b1259b7c510188a87655ec2857b135ed5d7c6517c",
    "1030e5d4af93c70eaa24836d185cdd7f8befb2054d875878067921248010593b1b",
);

enum ResponseScript {
    Complete(Vec<u8>),
    StallAfter(Vec<u8>),
}

struct ObservedExchange {
    peer_ip: IpAddr,
    request: Vec<u8>,
    second_socket: bool,
}

struct OneSocketFixture {
    peer: SocketAddr,
    release: Sender<()>,
    observed: Receiver<Result<ObservedExchange, String>>,
    server: JoinHandle<()>,
}

impl OneSocketFixture {
    fn spawn(script: ResponseScript) -> Self {
        let listener = TcpListener::bind("127.0.0.1:0").unwrap();
        let peer = listener.local_addr().unwrap();
        let (release_tx, release_rx) = mpsc::channel();
        let (observed_tx, observed_rx) = mpsc::channel();
        let server = thread::spawn(move || {
            let result = serve_one_socket(listener, script, release_rx);
            let _ = observed_tx.send(result);
        });
        Self {
            peer,
            release: release_tx,
            observed: observed_rx,
            server,
        }
    }

    fn finish(self) -> ObservedExchange {
        self.release.send(()).unwrap();
        let observed = self
            .observed
            .recv_timeout(Duration::from_secs(12))
            .expect("loopback server observation")
            .unwrap_or_else(|error| panic!("loopback server failed: {error}"));
        self.server.join().unwrap();
        observed
    }
}

fn serve_one_socket(
    listener: TcpListener,
    script: ResponseScript,
    release: Receiver<()>,
) -> Result<ObservedExchange, String> {
    let (mut stream, source) = listener.accept().map_err(|error| error.to_string())?;
    stream
        .set_read_timeout(Some(Duration::from_secs(2)))
        .map_err(|error| error.to_string())?;
    let request = read_request_head(&mut stream)?;

    match script {
        ResponseScript::Complete(response) => {
            stream
                .write_all(&response)
                .map_err(|error| error.to_string())?;
            stream.flush().map_err(|error| error.to_string())?;
            let _ = stream.shutdown(Shutdown::Write);
        }
        ResponseScript::StallAfter(prefix) => {
            stream
                .write_all(&prefix)
                .map_err(|error| error.to_string())?;
            stream.flush().map_err(|error| error.to_string())?;
            thread::sleep(PRODUCTION_POLICY.request_timeout + Duration::from_millis(250));
        }
    }
    drop(stream);

    release
        .recv_timeout(Duration::from_secs(10))
        .map_err(|error| error.to_string())?;
    listener
        .set_nonblocking(true)
        .map_err(|error| error.to_string())?;
    let second_socket = match listener.accept() {
        Ok(_) => true,
        Err(error) if error.kind() == ErrorKind::WouldBlock => false,
        Err(error) => return Err(error.to_string()),
    };
    Ok(ObservedExchange {
        peer_ip: source.ip(),
        request,
        second_socket,
    })
}

fn read_request_head(stream: &mut std::net::TcpStream) -> Result<Vec<u8>, String> {
    let mut request = Vec::with_capacity(2_048);
    let mut buffer = [0_u8; 1_024];
    loop {
        let read = stream
            .read(&mut buffer)
            .map_err(|error| error.to_string())?;
        if read == 0 {
            return Err("request closed before its header terminator".to_owned());
        }
        request.extend_from_slice(&buffer[..read]);
        if request.windows(4).any(|window| window == b"\r\n\r\n") {
            return Ok(request);
        }
        if request.len() > 8_192 {
            return Err("request head exceeded synthetic fixture bound".to_owned());
        }
    }
}

fn count_redirect_target_accepts(listener: TcpListener, window: Duration) -> JoinHandle<usize> {
    thread::spawn(move || {
        listener.set_nonblocking(true).unwrap();
        let deadline = Instant::now() + window;
        let mut accepts = 0;
        while Instant::now() < deadline {
            match listener.accept() {
                Ok((mut stream, _)) => {
                    accepts += 1;
                    stream
                        .write_all(
                            b"HTTP/1.1 200 OK\r\nContent-Type: application/json\r\nContent-Length: 2\r\nConnection: close\r\n\r\n{}",
                        )
                        .unwrap();
                }
                Err(error) if error.kind() == ErrorKind::WouldBlock => {
                    thread::sleep(Duration::from_millis(5));
                }
                Err(error) => panic!("redirect target accept failed: {error}"),
            }
        }
        accepts
    })
}

fn synthetic_request() -> AuthenticatedL1CredentialDerivationRequest {
    FixedEoaSigner::bind(
        EoaPrivateKeyInput::new(SYNTHETIC_KEY.to_owned()),
        SYNTHETIC_ADDRESS,
    )
    .unwrap()
    .consume_into_l1_credential_derivation_request(
        L1CredentialDerivationTimestamp::from_unix_seconds(1_780_449_126).unwrap(),
        L1CredentialDerivationNonce::from_u64(7),
    )
    .unwrap()
}

fn loopback_sink(peer: SocketAddr) -> LoopbackCredentialProofSink {
    let fixed_peer =
        PmFixedTlsPeerSelection::loopback_evidence(LOOPBACK_EVIDENCE_DNS_NAME, peer).unwrap();
    let selected_local_egress =
        PmLocalEgressSelection::loopback_evidence("lo", "127.0.0.2".parse().unwrap()).unwrap();
    LoopbackCredentialProofSink::loopback_evidence(fixed_peer, selected_local_egress).unwrap()
}

fn loopback_attempt_transport(peer: SocketAddr) -> LoopbackCredentialProofAttemptTransport {
    let fixed_peer =
        PmFixedTlsPeerSelection::loopback_evidence(LOOPBACK_EVIDENCE_DNS_NAME, peer).unwrap();
    let selected_local_egress =
        PmLocalEgressSelection::loopback_evidence("lo", "127.0.0.2".parse().unwrap()).unwrap();
    LoopbackCredentialProofAttemptTransport::loopback_evidence(fixed_peer, selected_local_egress)
        .unwrap()
}

fn dispatch_script(
    script: ResponseScript,
) -> (
    Result<L1CredentialDerivationResponseInput, CredentialProofTransportError>,
    ObservedExchange,
) {
    let fixture = OneSocketFixture::spawn(script);
    let mut sink = loopback_sink(fixture.peer);
    let result = synthetic_request().dispatch(&mut sink);
    let observed = fixture.finish();
    assert!(!observed.second_socket, "transport opened a second socket");
    (result, observed)
}

fn complete_response(status: &str, headers: &[&str], body: &[u8]) -> Vec<u8> {
    let mut response = format!("HTTP/1.1 {status}\r\n").into_bytes();
    for header in headers {
        response.extend_from_slice(header.as_bytes());
        response.extend_from_slice(b"\r\n");
    }
    response.extend_from_slice(format!("Content-Length: {}\r\n", body.len()).as_bytes());
    response.extend_from_slice(b"Connection: close\r\n\r\n");
    response.extend_from_slice(body);
    response
}

fn raw_response(head: &[u8], body: &[u8]) -> Vec<u8> {
    let mut response = head.to_vec();
    response.extend_from_slice(body);
    response
}

fn require_error(
    result: Result<L1CredentialDerivationResponseInput, CredentialProofTransportError>,
    expected: CredentialProofTransportError,
) {
    match result {
        Ok(_) => panic!("synthetic response was unexpectedly accepted"),
        Err(error) => assert_eq!(error, expected),
    }
}

fn parsed_request(request: &[u8]) -> (&str, BTreeMap<String, Vec<String>>, &[u8]) {
    let header_end = request
        .windows(4)
        .position(|window| window == b"\r\n\r\n")
        .expect("request header terminator");
    let head = std::str::from_utf8(&request[..header_end]).unwrap();
    let mut lines = head.split("\r\n");
    let request_line = lines.next().unwrap();
    let mut headers = BTreeMap::<String, Vec<String>>::new();
    for line in lines {
        let (name, value) = line.split_once(':').unwrap();
        headers
            .entry(name.to_ascii_lowercase())
            .or_default()
            .push(value.trim().to_owned());
    }
    (request_line, headers, &request[header_end + 4..])
}

fn one_header<'a>(headers: &'a BTreeMap<String, Vec<String>>, name: &str) -> &'a str {
    let values = headers
        .get(name)
        .unwrap_or_else(|| panic!("missing {name}"));
    assert_eq!(values.len(), 1, "duplicate {name}");
    &values[0]
}

#[test]
fn opaque_request_emits_one_exact_get_and_the_sink_is_one_shot() {
    let fixture = OneSocketFixture::spawn(ResponseScript::Complete(complete_response(
        "200 OK",
        &["Content-Type: application/json"],
        b"{}",
    )));
    let port = fixture.peer.port();
    let mut sink = loopback_sink(fixture.peer);

    assert!(synthetic_request().dispatch(&mut sink).is_ok());
    require_error(
        synthetic_request().dispatch(&mut sink),
        CredentialProofTransportError::SinkAlreadySpent,
    );

    let observed = fixture.finish();
    assert_eq!(observed.peer_ip, "127.0.0.2".parse::<IpAddr>().unwrap());
    assert!(!observed.second_socket);
    let (request_line, headers, body) = parsed_request(&observed.request);
    assert_eq!(request_line, "GET /auth/derive-api-key HTTP/1.1");
    assert!(!request_line.contains('?'));
    assert!(body.is_empty());
    assert_eq!(headers.len(), 8);
    assert_eq!(
        one_header(&headers, "host"),
        format!("{LOOPBACK_EVIDENCE_DNS_NAME}:{port}")
    );
    assert_eq!(one_header(&headers, "accept"), "application/json");
    assert_eq!(one_header(&headers, "accept-encoding"), "identity");
    assert_eq!(one_header(&headers, "connection"), "close");
    assert_eq!(one_header(&headers, "poly_address"), SYNTHETIC_ADDRESS);
    assert_eq!(one_header(&headers, "poly_signature"), SYNTHETIC_SIGNATURE);
    assert_eq!(one_header(&headers, "poly_timestamp"), "1780449126");
    assert_eq!(one_header(&headers, "poly_nonce"), "7");
    assert_eq!(
        headers
            .keys()
            .filter(|name| name.starts_with("poly_"))
            .count(),
        4
    );
    for forbidden in ["content-type", "content-length", "transfer-encoding"] {
        assert!(!headers.contains_key(forbidden));
    }
}

#[test]
fn source_time_age_accepts_exact_boundary_and_rejects_boundary_plus_one() {
    let now = Instant::now();
    let at_boundary = ActiveServerTimeObservation {
        unix_seconds: 1_780_449_126,
        monotonic_receive: now.checked_sub(MAX_LOOPBACK_SOURCE_TIME_AGE).unwrap(),
    };
    assert_eq!(validate_source_time_age(&at_boundary, now), Ok(()));

    let beyond_boundary = ActiveServerTimeObservation {
        unix_seconds: 1_780_449_126,
        monotonic_receive: now
            .checked_sub(MAX_LOOPBACK_SOURCE_TIME_AGE + Duration::from_nanos(1))
            .unwrap(),
    };
    assert_eq!(
        validate_source_time_age(&beyond_boundary, now),
        Err(CredentialProofTransportError::SourceTimeExpired)
    );
}

#[test]
fn expired_source_time_blocks_derive_before_a_second_socket() {
    let fixture = OneSocketFixture::spawn(ResponseScript::Complete(complete_response(
        "200 OK",
        &["Content-Type: application/json"],
        b"1780449126",
    )));
    let mut transport = loopback_attempt_transport(fixture.peer);
    let source_time = transport.first_server_time().unwrap();
    transport.expire_active_server_time_for_test();
    let request = FixedEoaSigner::bind(
        EoaPrivateKeyInput::new(SYNTHETIC_KEY.to_owned()),
        SYNTHETIC_ADDRESS,
    )
    .unwrap()
    .consume_into_l1_credential_derivation_request(
        L1CredentialDerivationTimestamp::from_unix_seconds(source_time.unix_seconds()).unwrap(),
        L1CredentialDerivationNonce::from_u64(0),
    )
    .unwrap();
    require_error(
        request.dispatch(&mut transport),
        CredentialProofTransportError::SourceTimeExpired,
    );

    let observed = fixture.finish();
    assert!(
        !observed.second_socket,
        "expired time opened an authenticated socket"
    );
    assert!(
        parsed_request(&observed.request)
            .0
            .starts_with("GET /time ")
    );
}

#[test]
fn exact_peer_is_required_before_any_response_semantics() {
    let expected = "127.0.0.1:43210".parse().unwrap();
    assert_eq!(validate_response_peer(expected, Some(expected)), Ok(()));
    assert_eq!(
        validate_response_peer(expected, None),
        Err(CredentialProofTransportError::ResponsePeerMismatch)
    );
    assert_eq!(
        validate_response_peer(expected, Some("127.0.0.1:43211".parse().unwrap())),
        Err(CredentialProofTransportError::ResponsePeerMismatch)
    );
}

#[test]
fn only_exact_status_200_is_accepted_without_retry() {
    for status in [
        "301 Moved Permanently",
        "201 Created",
        "204 No Content",
        "401 Unauthorized",
        "429 Too Many Requests",
        "500 Internal Server Error",
        "503 Service Unavailable",
    ] {
        let (result, _) = dispatch_script(ResponseScript::Complete(complete_response(
            status,
            &["Content-Type: application/json"],
            b"",
        )));
        require_error(result, CredentialProofTransportError::UnexpectedStatus);
    }
}

#[test]
fn redirect_policy_does_not_follow_a_valid_location_to_a_second_loopback_listener() {
    let redirect_target = TcpListener::bind("127.0.0.1:0").unwrap();
    let redirect_target_address = redirect_target.local_addr().unwrap();
    let target_accepts = count_redirect_target_accepts(redirect_target, Duration::from_millis(350));
    let location = format!("Location: http://{redirect_target_address}/redirect-target");

    let (result, first_exchange) = dispatch_script(ResponseScript::Complete(complete_response(
        "301 Moved Permanently",
        &[&location],
        b"",
    )));
    let redirect_accepts = target_accepts.join().unwrap();

    assert!(!first_exchange.second_socket);
    assert_eq!(redirect_accepts, 0, "redirect target received a follow");
    require_error(result, CredentialProofTransportError::UnexpectedStatus);
}

#[test]
fn content_type_allows_only_json_with_no_parameter_or_one_utf8_charset() {
    for content_type in [
        "application/json",
        "Application/JSON; Charset=UTF-8",
        "application/json; charset = utf-8",
    ] {
        let header = format!("Content-Type: {content_type}");
        let (result, _) = dispatch_script(ResponseScript::Complete(complete_response(
            "200 OK",
            &[&header],
            b"{}",
        )));
        assert!(
            result.is_ok(),
            "allowed content type rejected: {content_type}"
        );
    }

    for content_type in [
        "text/json",
        "application/json; charset=us-ascii",
        "application/json; charset=\"utf-8\"",
        "application/json; charset=utf-8; version=1",
        "application/json; boundary=x",
    ] {
        let header = format!("Content-Type: {content_type}");
        let (result, _) = dispatch_script(ResponseScript::Complete(complete_response(
            "200 OK",
            &[&header],
            b"{}",
        )));
        require_error(
            result,
            CredentialProofTransportError::InvalidApplicationHeaders,
        );
    }

    let (missing, _) = dispatch_script(ResponseScript::Complete(complete_response(
        "200 OK",
        &[],
        b"{}",
    )));
    require_error(
        missing,
        CredentialProofTransportError::InvalidApplicationHeaders,
    );

    let (duplicate, _) = dispatch_script(ResponseScript::Complete(complete_response(
        "200 OK",
        &[
            "Content-Type: application/json",
            "Content-Type: application/json",
        ],
        b"{}",
    )));
    require_error(
        duplicate,
        CredentialProofTransportError::InvalidApplicationHeaders,
    );
}

#[test]
fn content_encoding_allows_only_absence_or_one_identity_value() {
    for headers in [
        vec!["Content-Type: application/json"],
        vec![
            "Content-Type: application/json",
            "Content-Encoding: identity",
        ],
        vec![
            "Content-Type: application/json",
            "Content-Encoding: IDENTITY",
        ],
    ] {
        let (result, _) = dispatch_script(ResponseScript::Complete(complete_response(
            "200 OK", &headers, b"{}",
        )));
        assert!(result.is_ok());
    }

    for encoding in ["gzip", "identity, identity"] {
        let header = format!("Content-Encoding: {encoding}");
        let (result, _) = dispatch_script(ResponseScript::Complete(complete_response(
            "200 OK",
            &["Content-Type: application/json", &header],
            b"{}",
        )));
        require_error(
            result,
            CredentialProofTransportError::InvalidApplicationHeaders,
        );
    }

    let (duplicate, _) = dispatch_script(ResponseScript::Complete(complete_response(
        "200 OK",
        &[
            "Content-Type: application/json",
            "Content-Encoding: identity",
            "Content-Encoding: identity",
        ],
        b"{}",
    )));
    require_error(
        duplicate,
        CredentialProofTransportError::InvalidApplicationHeaders,
    );
}

#[test]
fn declared_and_streamed_response_bounds_are_both_exactly_1024() {
    let exact_body = vec![b' '; PRODUCTION_POLICY.maximum_response_body_bytes];
    let (exact, _) = dispatch_script(ResponseScript::Complete(complete_response(
        "200 OK",
        &["Content-Type: application/json"],
        &exact_body,
    )));
    assert!(exact.is_ok());

    let declared_oversize = raw_response(
        b"HTTP/1.1 200 OK\r\nContent-Type: application/json\r\nContent-Length: 1025\r\nConnection: close\r\n\r\n",
        b"",
    );
    let (declared, _) = dispatch_script(ResponseScript::Complete(declared_oversize));
    require_error(
        declared,
        CredentialProofTransportError::ResponseBodyTooLarge,
    );

    let streamed_body = vec![b'x'; PRODUCTION_POLICY.maximum_response_body_bytes + 1];
    let mut chunked = b"HTTP/1.1 200 OK\r\nContent-Type: application/json\r\nTransfer-Encoding: chunked\r\nConnection: close\r\n\r\n401\r\n".to_vec();
    chunked.extend_from_slice(&streamed_body);
    chunked.extend_from_slice(b"\r\n0\r\n\r\n");
    let (streamed, _) = dispatch_script(ResponseScript::Complete(chunked));
    require_error(
        streamed,
        CredentialProofTransportError::ResponseBodyTooLarge,
    );
}

#[test]
fn body_fault_has_no_retry_or_partial_auth_input() {
    let incomplete = raw_response(
        b"HTTP/1.1 200 OK\r\nContent-Type: application/json\r\nContent-Length: 4\r\nConnection: close\r\n\r\n",
        b"{",
    );
    let (result, _) = dispatch_script(ResponseScript::Complete(incomplete));
    require_error(result, CredentialProofTransportError::ResponseBodyFault);
}

#[test]
fn body_timeout_has_no_retry_or_partial_auth_input() {
    let prefix = b"HTTP/1.1 200 OK\r\nContent-Type: application/json\r\nTransfer-Encoding: chunked\r\nConnection: close\r\n\r\n5\r\nab".to_vec();
    let (result, _) = dispatch_script(ResponseScript::StallAfter(prefix));
    require_error(result, CredentialProofTransportError::RequestTimeout);
}

#[test]
fn production_and_wrong_host_selections_cannot_enter_the_loopback_seam() {
    let local =
        PmLocalEgressSelection::loopback_evidence("lo", "127.0.0.2".parse().unwrap()).unwrap();
    let wrong_host = PmFixedTlsPeerSelection::loopback_evidence(
        "wrong-host.test",
        "127.0.0.1:43210".parse().unwrap(),
    )
    .unwrap();
    match LoopbackCredentialProofSink::loopback_evidence(wrong_host, local) {
        Ok(_) => panic!("wrong evidence host accepted"),
        Err(error) => assert_eq!(
            error,
            CredentialProofTransportError::InvalidLoopbackSelection
        ),
    }

    let production_peer =
        PmFixedTlsPeerSelection::production("clob.polymarket.com", "8.8.8.8").unwrap();
    let production_local =
        PmLocalEgressSelection::production("pm0", "192.0.2.10".parse().unwrap()).unwrap();
    match LoopbackCredentialProofSink::loopback_evidence(production_peer, production_local) {
        Ok(_) => panic!("production selection entered loopback seam"),
        Err(error) => assert_eq!(
            error,
            CredentialProofTransportError::InvalidLoopbackSelection
        ),
    }
}

#[test]
fn errors_and_auth_input_debug_views_never_echo_the_synthetic_body() {
    let marker = "SYNTHETIC_RESPONSE_MARKER_MUST_NOT_APPEAR";
    let (result, _) = dispatch_script(ResponseScript::Complete(complete_response(
        "200 OK",
        &["Content-Type: application/json"],
        marker.as_bytes(),
    )));
    let response = match result {
        Ok(response) => response,
        Err(error) => panic!("synthetic response failed: {error}"),
    };
    assert!(!format!("{response:?}").contains(marker));
    for error in [
        CredentialProofTransportError::RequestTimeout,
        CredentialProofTransportError::ResponseBodyFault,
        CredentialProofTransportError::ResponseBodyTooLarge,
    ] {
        assert!(!format!("{error:?} {error}").contains(marker));
    }
}
