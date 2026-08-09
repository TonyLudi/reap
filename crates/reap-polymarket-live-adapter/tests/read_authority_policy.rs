const AUTHORITY: &str = include_str!("../src/read_authority.rs");
const PRIVATE_CREDENTIALS: &str = include_str!("../src/private_credentials.rs");
const PRIVATE_HTTP: &str = include_str!("../src/private_http.rs");
const ACCOUNT: &str = include_str!("../src/account.rs");
const RECONCILIATION: &str = include_str!("../src/reconciliation.rs");
const USER_WS: &str = include_str!("../src/user_ws.rs");
const LIB: &str = include_str!("../src/lib.rs");

#[test]
fn external_authority_is_purpose_closed_and_all_parsed_bindings_are_complete() {
    let production = AUTHORITY
        .split_once("#[cfg(test)]")
        .map(|(production, _)| production)
        .expect("external-authority unit tests must follow production source");
    for required in [
        "pub trait PmHttpReadAuthorityProvider: Send",
        "async fn authenticate_open_orders(",
        "async fn authenticate_trades(",
        "async fn authenticate_balance_allowance(",
        "async fn authenticate_closed_only(",
        "async fn authenticate_exact_order(",
        "async fn bind_open_orders(",
        "async fn bind_trades(",
        "async fn bind_exact_order(",
        "pub trait PmUserWsReadAuthorityProvider: Send",
        "async fn authenticate_user_subscription(",
        "async fn bind_user_frame(",
    ] {
        assert!(
            AUTHORITY.contains(required),
            "external authority seam is missing `{required}`",
        );
    }
    for forbidden in [
        "authenticate_place",
        "authenticate_owned_cancel",
        "sign_clob",
        "generic_request",
        "fn headers(",
        "fn credentials(",
        "fn api_key(",
        "fn secret(",
        "fn body(",
    ] {
        assert!(
            !production.contains(forbidden),
            "external read authority gained forbidden `{forbidden}` surface",
        );
    }
    assert!(
        PRIVATE_CREDENTIALS.contains("impl PmHttpReadAuthorityProvider for PmHttpCredentialRole")
    );
    assert!(
        PRIVATE_CREDENTIALS
            .contains("impl PmUserWsReadAuthorityProvider for PmUserWsCredentialRole")
    );
    assert!(PRIVATE_HTTP.contains("authority: Box<dyn PmHttpReadAuthorityProvider>"));
    assert!(ACCOUNT.contains("authority: &'a mut dyn PmHttpReadAuthorityProvider"));
    assert!(RECONCILIATION.contains("authority: &'a mut dyn PmHttpReadAuthorityProvider"));
    assert!(USER_WS.contains("credentials: Box<dyn PmUserWsReadAuthorityProvider>"));
}

#[test]
fn external_proxy_owner_fixes_production_endpoints_identity_and_read_only_profile() {
    for required in [
        "PmPrivateHttpConfig::production(",
        "PmUserWsConfig::production(exact_order_scope.condition(), user_ws_bounds)",
        "l2_signer_address.as_core() == proxy_funder",
        "PmReadOnlySignatureType::Proxy",
        "pub const fn production_order_entry_authorized(&self) -> bool",
        "false",
    ] {
        assert!(
            AUTHORITY.contains(required),
            "external proxy owner is missing `{required}`",
        );
    }
    assert!(LIB.contains("PmExternalProxyReadConnectivityOwner"));
    assert!(LIB.contains("PmHttpReadAuthorityProvider"));
    assert!(LIB.contains("PmUserWsReadAuthorityProvider"));
}

#[test]
fn user_activity_high_water_is_checked_and_reserved_before_untrusted_handoffs() {
    let production = USER_WS
        .split_once("#[cfg(test)]")
        .map(|(production, _)| production)
        .expect("user-WS unit tests must follow production source");
    for required in [
        "pub struct PmUserWsActivityView",
        "pub fn generation(&self) -> u64",
        ".fetch_update(Ordering::AcqRel, Ordering::Acquire",
        "generation.checked_add(1)",
        "ActivityGenerationOverflow",
        "let received_generation = activity.advance()?;",
        "let received_clock = observe(clock)?;",
        "parse_live_user_frame(raw.as_slice())",
        "credentials.bind_user_frame(frame).await",
        "pub const fn activity_generation(&self) -> u64",
        "Raw protocol",
        "intentionally consume an un-emitted",
    ] {
        assert!(
            production.contains(required),
            "private user-WS watermark is missing `{required}`",
        );
    }
    assert!(!production.contains(".fetch_add("));
    assert!(LIB.contains("PmUserWsActivityView"));

    let reserve = production
        .find("let received_generation = activity.advance()?;")
        .unwrap();
    let parse = production
        .find("parse_live_user_frame(raw.as_slice())")
        .unwrap();
    let bind = production
        .find("credentials.bind_user_frame(frame).await")
        .unwrap();
    assert!(reserve < parse && parse < bind);
}
