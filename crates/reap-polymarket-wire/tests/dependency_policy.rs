const MANIFEST: &str = include_str!("../Cargo.toml");
const LIB: &str = include_str!("../src/lib.rs");
const PRIVATE_FIXTURE: &str = include_str!("../src/private_fixture.rs");
const REST: &str = include_str!("../src/rest.rs");
const WS: &str = include_str!("../src/ws.rs");

#[test]
fn wire_crate_has_no_network_auth_signer_or_order_entry_dependency() {
    for forbidden_dependency in [
        "reqwest",
        "tokio",
        "tungstenite",
        "hyper",
        "hmac",
        "ring",
        "zeroize",
    ] {
        assert!(
            !MANIFEST.contains(forbidden_dependency),
            "forbidden dependency: {forbidden_dependency}"
        );
    }

    let production = [LIB, PRIVATE_FIXTURE, REST, WS].join("\n");
    for forbidden_symbol in [
        "Credentials",
        "ApiKey",
        "PrivateKey",
        "Signer",
        "passphrase",
        "signed_request",
        "authenticate",
        "place_order",
        "cancel_order",
        "connect_async",
        "TcpStream",
    ] {
        assert!(
            !production.contains(forbidden_symbol),
            "forbidden wire capability: {forbidden_symbol}"
        );
    }
}
