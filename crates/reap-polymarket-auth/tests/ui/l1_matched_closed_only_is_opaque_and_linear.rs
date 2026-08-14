use reap_polymarket_auth::{
    L1CredentialDerivationMatchedClosedOnlyDispatch, L1CredentialDerivationMatchedClosedOnlyRequest,
};

fn require_clone<T: Clone>() {}
fn require_serialize<T: serde::Serialize>() {}

fn main() {
    require_clone::<L1CredentialDerivationMatchedClosedOnlyRequest>();
    require_clone::<L1CredentialDerivationMatchedClosedOnlyDispatch<()>>();
    require_serialize::<L1CredentialDerivationMatchedClosedOnlyRequest>();
    require_serialize::<L1CredentialDerivationMatchedClosedOnlyDispatch<()>>();
}
