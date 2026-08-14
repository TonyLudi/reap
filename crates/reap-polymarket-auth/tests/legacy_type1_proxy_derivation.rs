use reap_polymarket_auth::{
    EoaAddress, LegacyType1ProxyAddress, POLYMARKET_LEGACY_TYPE1_PROXY_CHAIN_ID, PmAuthError,
    derive_legacy_type1_proxy_address, legacy_type1_proxy_address_matches,
};

const DUMMY_SIGNER: &str = "0x0000000000000000000000000000000000000001";
const DUMMY_PROXY: &str = "0x7754536ecd85c00b2E0CF9c1aA679340D8550756";

#[test]
fn public_helper_is_fixed_to_the_polygon_vector_and_canonical_proxy_type() {
    assert_eq!(POLYMARKET_LEGACY_TYPE1_PROXY_CHAIN_ID, 137);
    let signer = EoaAddress::parse(DUMMY_SIGNER).unwrap();
    let expected = LegacyType1ProxyAddress::parse(DUMMY_PROXY).unwrap();
    let derived = derive_legacy_type1_proxy_address(signer);
    assert_eq!(derived, expected);
    assert_eq!(derived.to_string(), DUMMY_PROXY);
    assert_eq!(
        format!("{derived:?}"),
        format!("LegacyType1ProxyAddress(\"{DUMMY_PROXY}\")")
    );
    assert!(legacy_type1_proxy_address_matches(signer, expected));
}

#[test]
fn proxy_parser_rejects_noncanonical_spelling_without_inventing_deployment_facts() {
    assert_eq!(
        LegacyType1ProxyAddress::parse(&DUMMY_PROXY.to_ascii_lowercase()),
        Err(PmAuthError::InvalidLegacyType1ProxyAddress)
    );
    assert_eq!(
        LegacyType1ProxyAddress::parse("0x0000000000000000000000000000000000000000")
            .unwrap()
            .bytes(),
        [0_u8; 20]
    );
}

#[test]
fn source_is_pure_offline_and_names_every_non_claim() {
    let source = include_str!("../src/legacy_type1_proxy.rs");
    for required in [
        "structural address relation",
        "deployed code",
        "current chain state",
        "signer-key possession or exclusivity",
        "proxy control",
        "provider acceptance",
        "authentication",
        "authorization",
        "hasher.update([0xff])",
        "Keccak256::digest(signer.bytes())",
    ] {
        assert!(
            source.contains(required),
            "missing boundary pin: {required}"
        );
    }
    for forbidden in [
        "std::fs",
        "std::net",
        "std::process",
        "reqwest",
        "tokio",
        "hyper",
        "FixedEoaSigner",
        "L2Credentials",
    ] {
        assert!(
            !source.contains(forbidden),
            "offline relation helper crossed capability boundary: {forbidden}"
        );
    }
}
