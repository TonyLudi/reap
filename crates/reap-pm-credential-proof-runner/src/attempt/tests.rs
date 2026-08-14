use std::{
    collections::BTreeMap,
    io::{Read as _, Write as _},
    net::{IpAddr, Shutdown, SocketAddr, TcpListener},
    os::unix::fs::PermissionsExt as _,
    sync::mpsc::{self, Receiver},
    thread::{self, JoinHandle},
    time::Duration,
};

use reap_polymarket_auth::{EoaPrivateKeyInput, L2CredentialInput};

use super::*;

// Every protocol value below is a published synthetic fixture. These tests
// load no external credential or selection input and never leave loopback.

const SYNTHETIC_KEY: &str = "0xac0974bec39a17e36ba4a6b4d238ff944bacb478cbed5efcae784d7bf4f2ff80";
const SYNTHETIC_ADDRESS: &str = "0xf39Fd6e51aad88F6F4ce6aB8827279cffFb92266";
const SYNTHETIC_API_KEY: &str = "00000000-0000-4000-8000-000000000001";
const SYNTHETIC_SECRET: &str = "AAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAA=";
const SYNTHETIC_PASSPHRASE: &str = "synthetic-passphrase";
const FIRST_TIME: u64 = 1_780_449_126;
const SECOND_TIME: u64 = 1_780_449_127;

struct ObservedRequest {
    peer_ip: IpAddr,
    bytes: Vec<u8>,
}

struct FourRequestFixture {
    peer: SocketAddr,
    observed: Receiver<Result<Vec<ObservedRequest>, String>>,
    server: JoinHandle<()>,
}

impl FourRequestFixture {
    fn spawn(second_time: u64, closed_body: &'static [u8]) -> Self {
        let listener = TcpListener::bind("127.0.0.1:0").unwrap();
        let peer = listener.local_addr().unwrap();
        let (observed_tx, observed_rx) = mpsc::channel();
        let server = thread::spawn(move || {
            let responses = [
                FIRST_TIME.to_string().into_bytes(),
                format!(
                    "{{\"apiKey\":\"{SYNTHETIC_API_KEY}\",\"secret\":\"{SYNTHETIC_SECRET}\",\"passphrase\":\"{SYNTHETIC_PASSPHRASE}\"}}"
                )
                .into_bytes(),
                second_time.to_string().into_bytes(),
                closed_body.to_vec(),
            ];
            let result = serve_requests(listener, &responses);
            let _ = observed_tx.send(result);
        });
        Self {
            peer,
            observed: observed_rx,
            server,
        }
    }

    fn finish(self) -> Vec<ObservedRequest> {
        let observed = self
            .observed
            .recv_timeout(Duration::from_secs(12))
            .expect("four-request loopback observation")
            .unwrap_or_else(|error| panic!("loopback server failed: {error}"));
        self.server.join().unwrap();
        observed
    }
}

fn serve_requests(
    listener: TcpListener,
    responses: &[Vec<u8>; 4],
) -> Result<Vec<ObservedRequest>, String> {
    let mut observed = Vec::with_capacity(4);
    for body in responses {
        let (mut stream, source) = listener.accept().map_err(|error| error.to_string())?;
        stream
            .set_read_timeout(Some(Duration::from_secs(2)))
            .map_err(|error| error.to_string())?;
        let bytes = read_request_head(&mut stream)?;
        let mut response = format!(
            "HTTP/1.1 200 OK\r\nContent-Type: application/json\r\nContent-Length: {}\r\nConnection: close\r\n\r\n",
            body.len()
        )
        .into_bytes();
        response.extend_from_slice(body);
        stream
            .write_all(&response)
            .and_then(|()| stream.flush())
            .map_err(|error| error.to_string())?;
        let _ = stream.shutdown(Shutdown::Both);
        observed.push(ObservedRequest {
            peer_ip: source.ip(),
            bytes,
        });
    }
    listener
        .set_nonblocking(true)
        .map_err(|error| error.to_string())?;
    match listener.accept() {
        Err(error) if error.kind() == std::io::ErrorKind::WouldBlock => Ok(observed),
        Ok(_) => Err("unexpected fifth loopback connection".to_owned()),
        Err(error) => Err(error.to_string()),
    }
}

fn read_request_head(stream: &mut std::net::TcpStream) -> Result<Vec<u8>, String> {
    let mut request = Vec::with_capacity(2_048);
    let mut buffer = [0_u8; 1_024];
    loop {
        let read = stream
            .read(&mut buffer)
            .map_err(|error| error.to_string())?;
        if read == 0 {
            return Err("request ended before header terminator".to_owned());
        }
        request.extend_from_slice(&buffer[..read]);
        if request.windows(4).any(|window| window == b"\r\n\r\n") {
            return Ok(request);
        }
        if request.len() > 8_192 {
            return Err("request header exceeded loopback bound".to_owned());
        }
    }
}

fn protected_directory() -> tempfile::TempDir {
    let directory = tempfile::tempdir().unwrap();
    std::fs::set_permissions(directory.path(), std::fs::Permissions::from_mode(0o700)).unwrap();
    directory
}

fn signer_and_l2() -> (FixedEoaSigner, L2Credentials) {
    let signer = FixedEoaSigner::bind(
        EoaPrivateKeyInput::new(SYNTHETIC_KEY.to_owned()),
        SYNTHETIC_ADDRESS,
    )
    .unwrap();
    let l2 = L2Credentials::bind(
        SYNTHETIC_ADDRESS,
        L2CredentialInput::new(
            SYNTHETIC_API_KEY.to_owned(),
            SYNTHETIC_SECRET.to_owned(),
            SYNTHETIC_PASSPHRASE.to_owned(),
        ),
    )
    .unwrap();
    (signer, l2)
}

fn commitment_inputs() -> CredentialProofAttemptCommitmentInputs {
    CredentialProofAttemptCommitmentInputs::synthetic_for_tests(
        [0x11; 32], [0x22; 32], [0x33; 32], [0x44; 32],
    )
}

fn selections(peer: SocketAddr) -> (PmFixedTlsPeerSelection, PmLocalEgressSelection) {
    let fixed = PmFixedTlsPeerSelection::loopback_evidence("credential-proof.test", peer).unwrap();
    let local =
        PmLocalEgressSelection::loopback_evidence("lo", "127.0.0.2".parse().unwrap()).unwrap();
    (fixed, local)
}

fn parsed_request(request: &[u8]) -> (&str, BTreeMap<String, Vec<String>>, &[u8]) {
    let header_end = request
        .windows(4)
        .position(|window| window == b"\r\n\r\n")
        .unwrap();
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
fn durable_attempt_orders_four_exact_requests_and_finishes_denied() {
    let directory = protected_directory();
    let fixture = FourRequestFixture::spawn(SECOND_TIME, br#"{"closed_only":false}"#);
    let (fixed, local) = selections(fixture.peer);
    let (signer, l2) = signer_and_l2();

    let denied = execute_private_loopback_credential_proof_attempt(
        directory.path(),
        fixed,
        local,
        commitment_inputs(),
        signer,
        l2,
    )
    .unwrap();
    let observed = fixture.finish();

    assert_eq!(denied.authorization(), "DENIED");
    assert!(!denied.production_permit());
    assert!(!denied.resume_allowed());
    assert!(format!("{denied:?}").contains("NO_REMOTE_OR_MUTATION_AUTHORITY"));
    assert_eq!(observed.len(), 4);
    assert!(
        observed
            .iter()
            .all(|request| request.peer_ip == "127.0.0.2".parse::<IpAddr>().unwrap())
    );

    let parsed = observed
        .iter()
        .map(|request| parsed_request(&request.bytes))
        .collect::<Vec<_>>();
    assert_eq!(
        parsed.iter().map(|(line, _, _)| *line).collect::<Vec<_>>(),
        [
            "GET /time HTTP/1.1",
            "GET /auth/derive-api-key HTTP/1.1",
            "GET /time HTTP/1.1",
            "GET /auth/ban-status/closed-only HTTP/1.1",
        ]
    );
    assert!(parsed.iter().all(|(_, _, body)| body.is_empty()));

    let derive = &parsed[1].1;
    assert_eq!(one_header(derive, "poly_address"), SYNTHETIC_ADDRESS);
    assert_eq!(one_header(derive, "poly_timestamp"), FIRST_TIME.to_string());
    assert_eq!(one_header(derive, "poly_nonce"), "0");
    assert!(one_header(derive, "poly_signature").starts_with("0x"));

    let closed = &parsed[3].1;
    assert_eq!(one_header(closed, "poly_address"), SYNTHETIC_ADDRESS);
    assert_eq!(
        one_header(closed, "poly_timestamp"),
        SECOND_TIME.to_string()
    );
    assert_eq!(one_header(closed, "poly_api_key"), SYNTHETIC_API_KEY);
    assert_eq!(one_header(closed, "poly_passphrase"), SYNTHETIC_PASSPHRASE);
    assert_eq!(
        one_header(closed, "poly_signature"),
        "p8gs7vdn5rQgINQuMeIRb7B6uYeqDghTBPRGai3RF7A="
    );
    for (_, headers, _) in &parsed {
        assert_eq!(one_header(headers, "accept"), "application/json");
        assert_eq!(one_header(headers, "accept-encoding"), "identity");
        assert_eq!(one_header(headers, "connection"), "close");
        for forbidden in ["content-type", "content-length", "transfer-encoding"] {
            assert!(!headers.contains_key(forbidden));
        }
    }
    drop(denied);
    let inspection = lineage::inspect_attempt_lineage(directory.path()).unwrap();
    assert_eq!(
        inspection,
        lineage::AttemptLineageInspection::CompleteDenied
    );
    assert_eq!(inspection.authorization(), "DENIED");
    assert!(!inspection.resume_allowed());
}

#[test]
fn strict_closed_only_rejection_stays_burned_and_cannot_retry() {
    let directory = protected_directory();
    let fixture = FourRequestFixture::spawn(SECOND_TIME, br#"{"closed_only":true}"#);
    let (fixed, local) = selections(fixture.peer);
    let (signer, l2) = signer_and_l2();
    assert_eq!(
        execute_private_loopback_credential_proof_attempt(
            directory.path(),
            fixed.clone(),
            local.clone(),
            commitment_inputs(),
            signer,
            l2,
        )
        .unwrap_err(),
        CredentialProofAttemptError::Transport
    );
    assert_eq!(fixture.finish().len(), 4);

    let (signer, l2) = signer_and_l2();
    assert_eq!(
        execute_private_loopback_credential_proof_attempt(
            directory.path(),
            fixed,
            local,
            commitment_inputs(),
            signer,
            l2,
        )
        .unwrap_err(),
        CredentialProofAttemptError::AttemptAlreadyBurned
    );
    let ledger =
        std::fs::read_to_string(directory.path().join(lineage::ATTEMPT_LEDGER_FILE)).unwrap();
    assert_eq!(ledger.lines().count(), 2);
    assert!(!ledger.contains("\"record\":\"Final\""));
}
