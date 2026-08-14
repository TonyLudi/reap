use reap_polymarket_auth::{
    L1CredentialDerivationMatchedL2Credentials, L1CredentialDerivationResponseInput,
};

fn require_clone<T: Clone>() {}
fn require_serialize<T: serde::Serialize>() {}

fn main() {
    require_clone::<L1CredentialDerivationResponseInput>();
    require_clone::<L1CredentialDerivationMatchedL2Credentials>();
    require_serialize::<L1CredentialDerivationResponseInput>();
    require_serialize::<L1CredentialDerivationMatchedL2Credentials>();
}
