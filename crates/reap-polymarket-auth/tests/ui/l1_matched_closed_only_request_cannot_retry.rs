use reap_polymarket_auth::{
    FixedClosedOnlyRequestSink, L1CredentialDerivationMatchedClosedOnlyRequest,
};

struct AlwaysErrors;

impl FixedClosedOnlyRequestSink for AlwaysErrors {
    type Output = ();
    type Error = ();

    fn send_exact_get_auth_ban_status_closed_only(
        &mut self,
        _poly_address: &str,
        _poly_signature: &str,
        _poly_timestamp: &str,
        _poly_api_key: &str,
        _poly_passphrase: &str,
    ) -> Result<Self::Output, Self::Error> {
        Err(())
    }
}

fn retry_after_error(
    request: L1CredentialDerivationMatchedClosedOnlyRequest,
    sink: &mut AlwaysErrors,
) {
    let _first_error = request.dispatch(sink).unwrap_err();
    let _retry = request.dispatch(sink);
}

fn main() {}
