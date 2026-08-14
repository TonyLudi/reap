use std::convert::Infallible;

use reap_polymarket_auth::{
    AuthenticatedL1CredentialDerivationRequest, EoaPrivateKeyInput, FixedEoaSigner,
    L1CredentialDerivationNonce, L1CredentialDerivationRequestSink,
    L1CredentialDerivationTimestamp, PmAuthError,
};

const KEY: &str = "0xac0974bec39a17e36ba4a6b4d238ff944bacb478cbed5efcae784d7bf4f2ff80";
const ADDRESS: &str = "0xf39Fd6e51aad88F6F4ce6aB8827279cffFb92266";
const EXPECTED_SIGNATURE: &str = concat!(
    "0x9670627f2da09dc111b4044b1259b7c510188a87655ec2857b135ed5d7c6517c",
    "1030e5d4af93c70eaa24836d185cdd7f8befb2054d875878067921248010593b1b",
);

#[derive(Default)]
struct Capture {
    calls: usize,
    operation: &'static str,
    address: String,
    signature: String,
    timestamp: String,
    nonce: String,
}

impl L1CredentialDerivationRequestSink for Capture {
    type Output = ();
    type Error = Infallible;

    fn send_exact_get_auth_derive_api_key(
        &mut self,
        poly_address: &str,
        poly_signature: &str,
        poly_timestamp: &str,
        poly_nonce: &str,
    ) -> Result<Self::Output, Self::Error> {
        self.calls += 1;
        self.operation = "GET /auth/derive-api-key";
        self.address = poly_address.to_owned();
        self.signature = poly_signature.to_owned();
        self.timestamp = poly_timestamp.to_owned();
        self.nonce = poly_nonce.to_owned();
        Ok(())
    }
}

fn request() -> AuthenticatedL1CredentialDerivationRequest {
    FixedEoaSigner::bind(EoaPrivateKeyInput::new(KEY.into()), ADDRESS)
        .unwrap()
        .consume_into_l1_credential_derivation_request(
            L1CredentialDerivationTimestamp::from_unix_seconds(1_780_449_126).unwrap(),
            L1CredentialDerivationNonce::from_u64(7),
        )
        .unwrap()
}

#[test]
fn one_local_request_holder_presents_the_exact_semantic_invocation_to_a_trusted_sink() {
    let request = request();
    assert_eq!(
        format!("{request:?}"),
        "AuthenticatedL1CredentialDerivationRequest([REDACTED; SIGNATURE_CONSTRUCTION_ONLY; NO_SERVER_PROOF_OR_AUTHORITY])"
    );
    let mut capture = Capture::default();
    request.dispatch(&mut capture).unwrap();
    assert_eq!(capture.calls, 1);
    assert_eq!(capture.operation, "GET /auth/derive-api-key");
    assert_eq!(capture.address, ADDRESS);
    assert_eq!(capture.signature, EXPECTED_SIGNATURE);
    assert_eq!(capture.timestamp, "1780449126");
    assert_eq!(capture.nonce, "7");
    assert_eq!(capture.signature.len(), 132);
    assert!(capture.signature.starts_with("0x"));
    assert!(
        capture.signature[2..]
            .bytes()
            .all(|byte| byte.is_ascii_digit() || (b'a'..=b'f').contains(&byte))
    );
}

#[test]
fn timestamp_and_nonce_have_narrow_canonical_grammars_without_freshness_claims() {
    assert_eq!(
        L1CredentialDerivationTimestamp::from_unix_seconds(999_999_999),
        Err(PmAuthError::InvalidL1CredentialDerivationTimestamp)
    );
    assert_eq!(
        L1CredentialDerivationTimestamp::from_unix_seconds(10_000_000_000),
        Err(PmAuthError::InvalidL1CredentialDerivationTimestamp)
    );
    assert_eq!(
        L1CredentialDerivationTimestamp::from_unix_seconds(1_000_000_000)
            .unwrap()
            .to_string(),
        "1000000000"
    );
    assert_eq!(L1CredentialDerivationNonce::from_u64(0).to_string(), "0");
    assert_eq!(
        L1CredentialDerivationNonce::from_u64(u64::MAX).to_string(),
        u64::MAX.to_string()
    );
}

#[test]
fn source_is_separate_plane_sealed_non_generic_and_non_authorizing() {
    let source = include_str!("../src/l1_credential_derivation.rs");
    for required in [
        "GET /auth/derive-api-key",
        "POLY_ADDRESS",
        "POLY_SIGNATURE",
        "POLY_TIMESTAMP",
        "POLY_NONCE",
        "ClobAuthDomain",
        "ClobAuth(address address,string timestamp,uint256 nonce,string message)",
        "This message attests that I control the given wallet",
        "Consuming the signer enforces one construction through this particular",
        "does not establish that",
        "timestamp came from the CLOB server",
        "no generic",
        "reusable",
        "header projection",
        "no mutation authority",
        "trusted external transport",
        "can retain or replay",
        "route elsewhere",
        "Consumption",
        "local only to this signer/request holder",
        "does not enforce sink",
        "host or destination",
        "TLS",
        "confidentiality",
        "non-retention",
        "global no-replay",
        "no caller-supplied query, body",
        "does not attest transport",
    ] {
        assert!(
            source.contains(required),
            "missing L1 boundary pin: {required}"
        );
    }

    for forbidden in [
        "reqwest::",
        "tokio::",
        "TcpStream",
        "UdpSocket",
        "L2Credentials",
        "L2CredentialInput",
        "AuthenticatedL2Headers",
        "create_api_key",
        "list_api_key",
        "delete_api_key",
        "POLY_API_KEY",
        "POLY_PASSPHRASE",
        "CONTENT_TYPE",
        "std::fs",
        "File::",
        "std::time",
        "std::env",
        "println!",
        "eprintln!",
        "dbg!",
        "tracing::",
        "log::",
        "pub fn signature",
        "pub fn headers",
        "pub fn as_bytes",
        "pub fn method",
        "pub fn path",
        "impl Clone for AuthenticatedL1CredentialDerivationRequest",
        "impl Serialize for AuthenticatedL1CredentialDerivationRequest",
    ] {
        assert!(
            !source.contains(forbidden),
            "L1 provisioning helper crossed capability boundary: {forbidden}"
        );
    }
    assert_eq!(
        source.matches("/auth/").count(),
        source.matches("/auth/derive-api-key").count(),
        "an alternate L1 credential endpoint entered the sealed helper"
    );

    let declaration = "pub struct AuthenticatedL1CredentialDerivationRequest {";
    let before = source.split_once(declaration).unwrap().0;
    let attributes = before
        .rsplit_once("\n}\n\n")
        .map_or(before, |(_, tail)| tail);
    assert!(!attributes.contains("#[derive"));
    let dispatch_surface = source
        .split_once("impl AuthenticatedL1CredentialDerivationRequest {")
        .and_then(|(_, tail)| tail.split_once("impl fmt::Debug").map(|(body, _)| body))
        .unwrap();
    assert_eq!(dispatch_surface.matches("pub fn ").count(), 1);
    assert!(dispatch_surface.contains("pub fn dispatch<"));
}
