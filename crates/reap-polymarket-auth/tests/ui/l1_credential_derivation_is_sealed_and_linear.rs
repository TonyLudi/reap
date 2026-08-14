use reap_polymarket_auth::{
    AuthenticatedL1CredentialDerivationRequest, FixedEoaSigner, L1CredentialDerivationNonce,
    L1CredentialDerivationRequestSink, L1CredentialDerivationTimestamp,
};

fn require_clone<T: Clone>() {}
fn require_serialize<T: serde::Serialize>() {}

fn reuse_request<S: L1CredentialDerivationRequestSink>(
    request: AuthenticatedL1CredentialDerivationRequest,
    sink: &mut S,
) {
    let _ = request.dispatch(sink);
    let _ = request.dispatch(sink);
}

fn reuse_signer(
    signer: FixedEoaSigner,
    timestamp: L1CredentialDerivationTimestamp,
    nonce: L1CredentialDerivationNonce,
) {
    let _ = signer.consume_into_l1_credential_derivation_request(timestamp, nonce);
    let _ = signer.consume_into_l1_credential_derivation_request(timestamp, nonce);
}

fn reroute_request<S: L1CredentialDerivationRequestSink>(
    request: AuthenticatedL1CredentialDerivationRequest,
    sink: &mut S,
) {
    let _ = request.dispatch_to("GET", "/auth/api-key", sink);
}

fn main() {
    require_clone::<AuthenticatedL1CredentialDerivationRequest>();
    require_serialize::<AuthenticatedL1CredentialDerivationRequest>();
}
