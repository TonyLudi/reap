use reap_polymarket_auth::{
    L1CredentialDerivationMatchedClosedOnlyDispatch, L1CredentialDerivationMatchedClosedOnlyRequest,
};

fn recover_request(value: L1CredentialDerivationMatchedClosedOnlyRequest) {
    let _ = value.original_l2_credentials();
    let _ = value.request();
}

fn recover_dispatch(value: L1CredentialDerivationMatchedClosedOnlyDispatch<()>) {
    let _ = value.original_l2_credentials();
    let _ = value.output();
    let _ = value.into_output();
}

fn main() {}
