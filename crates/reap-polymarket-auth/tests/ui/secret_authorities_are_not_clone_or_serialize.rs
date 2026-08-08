use reap_polymarket_auth::{
    AuthenticatedPlaceRequest, AuthenticatedUserSubscription, CredentialOwnedUserFrame,
    FixedEoaSigner, L2Credentials,
};

fn require_clone<T: Clone>() {}
fn require_serialize<T: serde::Serialize>() {}

fn main() {
    require_clone::<FixedEoaSigner>();
    require_clone::<L2Credentials>();
    require_clone::<AuthenticatedPlaceRequest>();
    require_clone::<AuthenticatedUserSubscription>();
    require_clone::<CredentialOwnedUserFrame>();
    require_serialize::<FixedEoaSigner>();
    require_serialize::<L2Credentials>();
    require_serialize::<CredentialOwnedUserFrame>();
}
