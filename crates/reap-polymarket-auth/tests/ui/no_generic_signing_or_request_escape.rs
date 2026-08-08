use reap_polymarket_auth::{
    FixedEoaSigner, L2Credentials, L2Timestamp, SerializedPlaceRequest, SignedClobV2Order,
};

fn attempt_escape(
    signer: &FixedEoaSigner,
    credentials: &L2Credentials,
    signed: &SignedClobV2Order,
    body: &SerializedPlaceRequest,
    timestamp: L2Timestamp,
) {
    let _ = signer.sign_bytes(b"arbitrary");
    let _ = signer.sign_digest([0_u8; 32]);
    let _ = credentials.hmac(b"arbitrary");
    let _ = credentials.authenticate_request(timestamp, "PATCH", "/anything", b"{}");
    let _ = credentials.secret();
    let _ = signed.signature();
    let _ = body.as_bytes();
}

fn main() {}
