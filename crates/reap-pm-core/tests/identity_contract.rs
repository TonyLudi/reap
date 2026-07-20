use reap_core::Venue;
use reap_pm_core::{
    ConnectionEpoch, EvmAddress, IngressSequence, OkxInstrumentId, OkxReferenceHandle,
    OkxReferenceInstrument, PmAccountHandle, PmAccountScope, PmAssetId, PmChainId, PmClientOrderId,
    PmClientOrderKey, PmConditionId, PmConnectionId, PmEnvironmentId, PmFillId, PmFillKey,
    PmFunderId, PmInstrumentHandle, PmInstrumentId, PmMarketHandle, PmMarketId, PmProductSource,
    PmSignerId, PmSourceHandle, PmSpenderDomain, PmSpenderId, PmSpenderRequirement, PmTokenHandle,
    PmTokenId, PmVenueOrderId, PmVenueOrderKey, SnapshotRevision, U256,
};

const ADDRESS_A: &str = "0x1111111111111111111111111111111111111111";
const ADDRESS_B: &str = "0x2222222222222222222222222222222222222222";
const CONDITION: &str = "0xaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa";
const MARKET: &str = "0xbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbb";

#[test]
fn fixed_hex_identities_round_trip_canonically_without_cross_type_aliases() {
    let address = EvmAddress::parse(ADDRESS_A).unwrap();
    let condition = PmConditionId::parse(CONDITION).unwrap();
    let market = PmMarketId::parse(MARKET).unwrap();
    let token = PmTokenId::new(U256::from_u64(123)).unwrap();
    let instrument = PmInstrumentId::new(market, token);

    assert_eq!(address.to_string(), ADDRESS_A);
    assert_eq!(condition.to_string(), CONDITION);
    assert_eq!(market.to_string(), MARKET);
    assert_eq!(token.units(), U256::from_u64(123));
    assert_eq!(instrument.market(), market);
    assert_eq!(instrument.token(), token);

    assert_eq!(
        serde_json::to_string(&address).unwrap(),
        format!("\"{ADDRESS_A}\"")
    );
    assert_eq!(
        serde_json::from_str::<PmConditionId>(&serde_json::to_string(&condition).unwrap()).unwrap(),
        condition
    );
    assert_eq!(
        serde_json::from_str::<PmMarketId>(&serde_json::to_string(&market).unwrap()).unwrap(),
        market
    );
    assert_ne!(condition.bytes(), market.bytes());
}

#[test]
fn mixed_case_evm_input_has_one_lowercase_canonical_identity() {
    let address = EvmAddress::parse("0xAbCdAbCdAbCdAbCdAbCdAbCdAbCdAbCdAbCdAbCd").unwrap();
    assert_eq!(
        address.to_string(),
        "0xabcdabcdabcdabcdabcdabcdabcdabcdabcdabcd"
    );
    assert_eq!(
        serde_json::to_string(&address).unwrap(),
        "\"0xabcdabcdabcdabcdabcdabcdabcdabcdabcdabcd\""
    );
}

#[test]
fn fixed_hex_identities_reject_wrong_width_invalid_text_and_zero() {
    assert!(EvmAddress::parse("0x11").is_err());
    assert!(EvmAddress::parse("1111111111111111111111111111111111111111").is_err());
    assert!(EvmAddress::parse("0xzz11111111111111111111111111111111111111").is_err());
    assert!(EvmAddress::parse("0x0000000000000000000000000000000000000000").is_err());
    assert!(
        PmConditionId::parse("0x0000000000000000000000000000000000000000000000000000000000000000")
            .is_err()
    );
    assert!(PmTokenId::new(U256::ZERO).is_err());
}

#[test]
fn client_order_identity_is_exactly_sixteen_bytes_and_lowercase_hex() {
    let id = PmClientOrderId::parse("00112233445566778899aabbccddeeff").unwrap();
    assert_eq!(
        id.bytes(),
        [
            0x00, 0x11, 0x22, 0x33, 0x44, 0x55, 0x66, 0x77, 0x88, 0x99, 0xaa, 0xbb, 0xcc, 0xdd,
            0xee, 0xff
        ]
    );
    assert_eq!(id.to_string(), "00112233445566778899aabbccddeeff");
    assert_eq!(
        serde_json::from_str::<PmClientOrderId>(&serde_json::to_string(&id).unwrap()).unwrap(),
        id
    );

    assert!(PmClientOrderId::parse("00112233445566778899AABBCCDDEEFF").is_err());
    assert!(PmClientOrderId::parse("0x00112233445566778899aabbccddeeff").is_err());
    assert!(PmClientOrderId::parse("0011").is_err());
}

#[test]
fn opaque_remote_and_connection_ids_are_bounded_exact_ascii() {
    let venue = PmVenueOrderId::new("order-123/abc").unwrap();
    let fill = PmFillId::new("fill:456").unwrap();
    let connection = PmConnectionId::new("pm-market-1").unwrap();
    assert_eq!(venue.as_str(), "order-123/abc");
    assert_eq!(fill.as_str(), "fill:456");
    assert_eq!(connection.as_str(), "pm-market-1");

    assert!(PmVenueOrderId::new("").is_err());
    assert!(PmFillId::new("contains space").is_err());
    assert!(PmConnectionId::new("line\nbreak").is_err());
    assert!(PmVenueOrderId::new(&"x".repeat(97)).is_err());
}

#[test]
fn configured_environment_account_funder_and_spender_are_structural() {
    let environment = PmEnvironmentId::new("mainnet").unwrap();
    let chain = PmChainId::new(137).unwrap();
    let signer = PmSignerId::new(EvmAddress::parse(ADDRESS_A).unwrap());
    let funder = PmFunderId::new(EvmAddress::parse(ADDRESS_B).unwrap());
    let account_handle = PmAccountHandle::from_ordinal(2);
    let account = PmAccountScope::new(environment, chain, signer, funder, account_handle);
    assert_eq!(account.handle(), account_handle);
    assert_eq!(account.chain(), chain);
    assert_ne!(account.signer().address(), account.funder().address());

    let collateral = PmAssetId::collateral(EvmAddress::parse(ADDRESS_A).unwrap());
    let requirement = PmSpenderRequirement::new(
        chain,
        EvmAddress::parse(ADDRESS_B).unwrap(),
        PmSpenderDomain::Standard,
        collateral,
    );
    let spender = PmSpenderId::new(account_handle, requirement);
    let other = PmSpenderId::new(PmAccountHandle::from_ordinal(3), requirement);
    assert_ne!(spender, other);
    assert_eq!(spender.account(), account_handle);
}

#[test]
fn zero_chain_deserialization_is_rejected_at_every_nested_identity_boundary() {
    assert!(serde_json::from_str::<PmChainId>("0").is_err());
    assert_eq!(
        serde_json::from_str::<PmChainId>("137").unwrap(),
        PmChainId::new(137).unwrap()
    );

    let chain = PmChainId::new(137).unwrap();
    let account = PmAccountScope::new(
        PmEnvironmentId::new("mainnet").unwrap(),
        chain,
        PmSignerId::new(EvmAddress::parse(ADDRESS_A).unwrap()),
        PmFunderId::new(EvmAddress::parse(ADDRESS_B).unwrap()),
        PmAccountHandle::from_ordinal(2),
    );
    let mut account_json = serde_json::to_value(account).unwrap();
    account_json["chain"] = serde_json::json!(0);
    assert!(serde_json::from_value::<PmAccountScope>(account_json).is_err());

    let requirement = PmSpenderRequirement::new(
        chain,
        EvmAddress::parse(ADDRESS_B).unwrap(),
        PmSpenderDomain::Standard,
        PmAssetId::collateral(EvmAddress::parse(ADDRESS_A).unwrap()),
    );
    let mut requirement_json = serde_json::to_value(requirement).unwrap();
    requirement_json["chain"] = serde_json::json!(0);
    assert!(serde_json::from_value::<PmSpenderRequirement>(requirement_json).is_err());

    let spender = PmSpenderId::new(PmAccountHandle::from_ordinal(2), requirement);
    let mut spender_json = serde_json::to_value(spender).unwrap();
    spender_json["requirement"]["chain"] = serde_json::json!(0);
    assert!(serde_json::from_value::<PmSpenderId>(spender_json).is_err());
}

#[test]
fn order_and_fill_keys_scope_identical_raw_components_by_account() {
    let first_account = PmAccountHandle::from_ordinal(1);
    let second_account = PmAccountHandle::from_ordinal(2);

    let client_id = PmClientOrderId::parse("00112233445566778899aabbccddeeff").unwrap();
    let first_client = PmClientOrderKey::new(first_account, client_id);
    let second_client = PmClientOrderKey::new(second_account, client_id);
    assert_ne!(first_client, second_client);
    assert_eq!(first_client.account(), first_account);
    assert_eq!(first_client.id().bytes(), second_client.id().bytes());
    assert_eq!(
        serde_json::from_str::<PmClientOrderKey>(&serde_json::to_string(&second_client).unwrap())
            .unwrap(),
        second_client
    );
    let mut invalid_client = serde_json::to_value(first_client).unwrap();
    invalid_client["id"] = serde_json::json!("00112233445566778899AABBCCDDEEFF");
    assert!(serde_json::from_value::<PmClientOrderKey>(invalid_client).is_err());

    let venue_id = PmVenueOrderId::new("shared-order").unwrap();
    let first_venue = PmVenueOrderKey::new(first_account, venue_id);
    let second_venue = PmVenueOrderKey::new(second_account, venue_id);
    assert_ne!(first_venue, second_venue);
    assert_eq!(first_venue.id(), second_venue.id());
    assert_eq!(
        serde_json::from_str::<PmVenueOrderKey>(&serde_json::to_string(&second_venue).unwrap())
            .unwrap()
            .account(),
        second_account
    );

    let fill_id = PmFillId::new("shared-fill").unwrap();
    let first_fill = PmFillKey::new(first_account, fill_id);
    let second_fill = PmFillKey::new(second_account, fill_id);
    assert_ne!(first_fill, second_fill);
    assert_eq!(first_fill.id(), second_fill.id());
    assert_eq!(
        serde_json::from_str::<PmFillKey>(&serde_json::to_string(&second_fill).unwrap()).unwrap(),
        second_fill
    );
    let mut ambiguous_fill = serde_json::to_value(second_fill).unwrap();
    ambiguous_fill["venue"] = serde_json::json!("polymarket");
    assert!(serde_json::from_value::<PmFillKey>(ambiguous_fill).is_err());
}

#[test]
fn identical_underlying_values_remain_distinct_across_structural_scopes() {
    let condition = PmConditionId::parse(CONDITION).unwrap();
    let same_bytes_as_market = PmMarketId::parse(CONDITION).unwrap();
    assert_eq!(
        serde_json::to_string(&condition).unwrap(),
        serde_json::to_string(&same_bytes_as_market).unwrap()
    );

    let token = PmTokenId::new(U256::from_u64(123)).unwrap();
    let first_instrument = PmInstrumentId::new(same_bytes_as_market, token);
    let second_instrument = PmInstrumentId::new(PmMarketId::parse(MARKET).unwrap(), token);
    assert_ne!(first_instrument, second_instrument);
    assert_eq!(first_instrument.token(), second_instrument.token());

    let environment = PmEnvironmentId::new("mainnet").unwrap();
    let chain = PmChainId::new(137).unwrap();
    let signer_address = EvmAddress::parse(ADDRESS_A).unwrap();
    let handle = PmAccountHandle::from_ordinal(1);
    let first_account = PmAccountScope::new(
        environment,
        chain,
        PmSignerId::new(signer_address),
        PmFunderId::new(signer_address),
        handle,
    );
    let second_account = PmAccountScope::new(
        environment,
        chain,
        PmSignerId::new(signer_address),
        PmFunderId::new(EvmAddress::parse(ADDRESS_B).unwrap()),
        handle,
    );
    assert_ne!(first_account, second_account);

    let requirement = PmSpenderRequirement::new(
        chain,
        EvmAddress::parse(ADDRESS_B).unwrap(),
        PmSpenderDomain::Standard,
        PmAssetId::collateral(signer_address),
    );
    assert_ne!(
        PmSpenderId::new(PmAccountHandle::from_ordinal(1), requirement),
        PmSpenderId::new(PmAccountHandle::from_ordinal(2), requirement)
    );
}

#[test]
fn okx_reference_is_explicit_index_identity_without_suffix_parsing() {
    let instrument_id = OkxInstrumentId::new("BTC-USDT").unwrap();
    let reference = OkxReferenceInstrument::index(instrument_id);
    assert_eq!(reference.venue(), Venue::Okx);
    assert_eq!(reference.instrument_id().as_str(), "BTC-USDT");
    assert!(OkxInstrumentId::new("BTC-USDT.PM").is_err());
    assert!(OkxInstrumentId::new("BTC-USDT.PF").is_err());
    assert!(OkxInstrumentId::new("btc-usdt").is_err());
}

#[test]
fn okx_reference_wire_form_has_no_mutable_venue_discriminator() {
    let reference = OkxReferenceInstrument::index(OkxInstrumentId::new("BTC-USDT").unwrap());
    let encoded = serde_json::to_string(&reference).unwrap();
    assert!(!encoded.contains("venue"));
    assert_eq!(
        serde_json::from_str::<OkxReferenceInstrument>(&encoded).unwrap(),
        reference
    );
    assert_eq!(reference.venue(), Venue::Okx);
}

#[test]
fn compact_handles_revisions_and_product_sources_remain_distinct() {
    let reference = OkxReferenceHandle::from_ordinal(1);
    let market = PmMarketHandle::from_ordinal(1);
    let token = PmTokenHandle::from_ordinal(1);
    let instrument = PmInstrumentHandle::new(market, token);
    assert_eq!(instrument.market(), market);
    assert_eq!(instrument.token(), token);

    let source_handle = PmSourceHandle::from_ordinal(7);
    let okx = PmProductSource::okx_reference(source_handle, reference);
    let pm_market = PmProductSource::polymarket_market(source_handle, token);
    let pm_account =
        PmProductSource::polymarket_account(source_handle, PmAccountHandle::from_ordinal(2));
    assert_eq!(okx.venue(), Venue::Okx);
    assert_eq!(pm_market.venue(), Venue::Polymarket);
    assert_eq!(pm_account.venue(), Venue::Polymarket);
    assert_ne!(okx, pm_market);

    assert_eq!(ConnectionEpoch::new(4).value(), 4);
    assert_eq!(SnapshotRevision::new(5).value(), 5);
    assert_eq!(IngressSequence::new(6).value(), 6);
}
