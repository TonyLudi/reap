use reap_polymarket_auth::{
    L1CredentialDerivationMatchedClosedOnlyDispatch,
    L1CredentialDerivationMatchedClosedOnlyRequest, L2Credentials,
};

fn impossible<T>() -> T {
    panic!("unreachable private request value")
}

fn destructure_request(value: L1CredentialDerivationMatchedClosedOnlyRequest) {
    let L1CredentialDerivationMatchedClosedOnlyRequest {
        original_l2_credentials: _,
        request: _,
    } = value;
}

fn forge_request(credentials: L2Credentials) -> L1CredentialDerivationMatchedClosedOnlyRequest {
    L1CredentialDerivationMatchedClosedOnlyRequest {
        original_l2_credentials: credentials,
        request: impossible(),
    }
}

fn destructure_dispatch(value: L1CredentialDerivationMatchedClosedOnlyDispatch<()>) {
    let L1CredentialDerivationMatchedClosedOnlyDispatch {
        _original_l2_credentials: _,
        _output: _,
    } = value;
}

fn forge_dispatch(
    credentials: L2Credentials,
) -> L1CredentialDerivationMatchedClosedOnlyDispatch<()> {
    L1CredentialDerivationMatchedClosedOnlyDispatch {
        _original_l2_credentials: credentials,
        _output: (),
    }
}

fn main() {}
