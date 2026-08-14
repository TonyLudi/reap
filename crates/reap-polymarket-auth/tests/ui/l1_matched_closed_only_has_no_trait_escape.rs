use std::{borrow::Borrow, ops::Deref};

use reap_polymarket_auth::{
    L1CredentialDerivationMatchedClosedOnlyDispatch,
    L1CredentialDerivationMatchedClosedOnlyRequest, L2Credentials,
};

fn require_deserialize<T: serde::de::DeserializeOwned>() {}
fn require_as_ref<T: AsRef<L2Credentials>>() {}
fn require_borrow<T: Borrow<L2Credentials>>() {}
fn require_deref<T: Deref<Target = L2Credentials>>() {}
fn require_into<T: Into<L2Credentials>>() {}

fn main() {
    require_deserialize::<L1CredentialDerivationMatchedClosedOnlyRequest>();
    require_deserialize::<L1CredentialDerivationMatchedClosedOnlyDispatch<()>>();
    require_as_ref::<L1CredentialDerivationMatchedClosedOnlyRequest>();
    require_deref::<L1CredentialDerivationMatchedClosedOnlyRequest>();
    require_borrow::<L1CredentialDerivationMatchedClosedOnlyDispatch<()>>();
    require_into::<L1CredentialDerivationMatchedClosedOnlyDispatch<()>>();
}
