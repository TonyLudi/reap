const MANIFEST: &str = include_str!("../Cargo.toml");
const LIB: &str = include_str!("../src/lib.rs");
const PRIVATE_FIXTURE: &str = include_str!("../src/private_fixture.rs");
const REST: &str = include_str!("../src/rest.rs");
const UNSIGNED_ORDER: &str = include_str!("../src/unsigned_order.rs");
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

    let production = [LIB, PRIVATE_FIXTURE, REST, UNSIGNED_ORDER, WS].join("\n");
    for forbidden_symbol in [
        "Credentials",
        "ApiKey",
        "PrivateKey",
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

    for forbidden_unsigned_capability in [
        "Deserialize for PmUnsignedClobV2Order",
        "signature:",
        "expiration:",
        "owner:",
        "Secret",
        "Eip712",
        "HttpClient",
    ] {
        assert!(
            !UNSIGNED_ORDER.contains(forbidden_unsigned_capability),
            "unsigned DTO contains forbidden capability: {forbidden_unsigned_capability}"
        );
    }
}
