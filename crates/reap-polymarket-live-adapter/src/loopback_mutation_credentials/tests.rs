use std::time::Duration;

use reap_pm_core::{
    ConnectionEpoch, EvmAddress, MAX_REQUIRED_SPENDERS, OkxInstrumentId, OkxReferenceInstrument,
    PmAccountHandle, PmAccountScope, PmAssetId, PmChainId, PmClientOrderId, PmClientOrderKey,
    PmConditionId, PmEnvironmentId, PmFunderId, PmGoalFTradingDomain, PmInstrumentHandle,
    PmInstrumentId, PmMarketHandle, PmMarketId, PmMarketLifecycle, PmMarketMetadata, PmOrderSalt,
    PmOrderSide, PmOutcomeLabel, PmOutcomeMetadata, PmPrice, PmPublicObservationGrant, PmQuantity,
    PmSignerId, PmSpenderDomain, PmSpenderRequirement, PmTick, PmTokenHandle, PmTokenId,
    PmVenueOrderId, PmVenueOrderKey, U256, exact_order_amounts,
};
use reap_polymarket_adapter::{
    PmExactOwnedCancelRequest, PmFixtureInstrumentScope, PmFixtureOwnedExecution,
    PmGtcPostOnlyPlaceRequest,
};
use reap_polymarket_auth::{
    CredentialSlotId, EoaPrivateKeyInput, FixedEoaSigner, L2CredentialInput, L2Credentials,
    L2Timestamp,
};
use reap_polymarket_wire::PmBookParserConfig;
use reap_polymarket_wire::PmWireScope;
use tokio::{
    io::{AsyncReadExt, AsyncWriteExt},
    net::TcpListener,
};

use super::*;
use crate::{
    PmLoopbackServerTimeScript, PmProductClockOwner, PmPublicHttpConfig, PmPublicHttpRole,
};

const TEST_KEY: &str = "0xac0974bec39a17e36ba4a6b4d238ff944bacb478cbed5efcae784d7bf4f2ff80";
const ADDRESS: &str = "0xf39Fd6e51aad88F6F4ce6aB8827279cffFb92266";
const FOREIGN_ADDRESS: &str = "0x70997970C51812dc3A010C7d01b50e0d17dc79C8";
const API_KEY: &str = "00000000-0000-4000-8000-000000000001";
const API_SECRET: &str = "AAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAA=";
const PASSPHRASE: &str = "synthetic-passphrase";
const CONDITION: &str = "0xaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa";
const MARKET: &str = "0xbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbb";
const FOREIGN_MARKET: &str = "0xcccccccccccccccccccccccccccccccccccccccccccccccccccccccccccccccc";
const VENUE_ORDER: &str = "0xfaf10599783c69b375a0f0d948d37eb711ec042dbf7d52fc2f8d8832d71af7f1";
const PUSD: &str = "0xC011a7E12a19f7B1f670d46F03B03f3342E82DFB";
const CONDITIONAL_TOKENS: &str = "0x4D97DCd97eC945f40cF65F87097ACe5EA0476045";
const STANDARD_EXCHANGE: &str = "0xE111180000d2663C0091e4f400237545B87B996B";
const AUTH_SECONDS: u64 = 1_780_449_126;
const CREDENTIAL_SLOT: &str = "loopback-primary-v1";

fn mutation_server_time(seconds: u64) -> PmAuthorizedMutationServerTime {
    let mut script = PmLoopbackServerTimeScript::new(&[seconds]).unwrap();
    script.issue_authorized_mutation_server_time().unwrap()
}

fn address(value: &str) -> EvmAddress {
    EvmAddress::parse(value).unwrap()
}

fn token(value: u64) -> PmTokenId {
    PmTokenId::new(U256::from_u64(value)).unwrap()
}

fn instrument() -> PmInstrumentHandle {
    PmInstrumentHandle::new(
        PmMarketHandle::from_ordinal(0),
        PmTokenHandle::from_ordinal(0),
    )
}

fn account() -> PmAccountScope {
    let eoa = address(ADDRESS);
    PmAccountScope::new(
        PmEnvironmentId::new("loopback-mutation").unwrap(),
        PmChainId::new(137).unwrap(),
        PmSignerId::new(eoa),
        PmFunderId::new(eoa),
        PmAccountHandle::from_ordinal(7),
    )
}

fn metadata(market: &str, token_id: u64) -> PmMarketMetadata {
    let chain = PmChainId::new(137).unwrap();
    let exchange = address(STANDARD_EXCHANGE);
    let token = token(token_id);
    let collateral = PmAssetId::collateral(address(PUSD));
    let outcome = PmAssetId::outcome(address(CONDITIONAL_TOKENS), token);
    let mut spenders = [None; MAX_REQUIRED_SPENDERS];
    spenders[0] = Some(PmSpenderRequirement::new(
        chain,
        exchange,
        PmSpenderDomain::Standard,
        collateral,
    ));
    spenders[1] = Some(PmSpenderRequirement::new(
        chain,
        exchange,
        PmSpenderDomain::Standard,
        outcome,
    ));
    PmMarketMetadata::new(
        PmConditionId::parse(CONDITION).unwrap(),
        PmMarketId::parse(market).unwrap(),
        PmOutcomeMetadata::new(token, PmOutcomeLabel::new("Yes").unwrap()),
        PmMarketLifecycle::new(true, false, false, true, true),
        PmTick::parse_decimal("0.01").unwrap(),
        PmQuantity::parse_decimal("5").unwrap(),
        false,
        chain,
        exchange,
        spenders,
        2,
    )
    .unwrap()
}

fn instrument_scope(market: &str, token_id: u64) -> PmFixtureInstrumentScope {
    PmFixtureInstrumentScope::from_metadata(instrument(), metadata(market, token_id)).unwrap()
}

fn execution() -> PmFixtureOwnedExecution {
    PmFixtureOwnedExecution::new(account(), instrument())
}

fn client_order() -> PmClientOrderKey {
    PmClientOrderKey::new(
        account().handle(),
        PmClientOrderId::parse("01010101010101010101010101010101").unwrap(),
    )
}

fn place(scope: PmFixtureInstrumentScope) -> PmGtcPostOnlyPlaceRequest {
    execution()
        .place_command(
            scope,
            client_order(),
            PmOrderSalt::from_u64(479_249_096_354).unwrap(),
            PmOrderSide::Buy,
            PmPrice::parse_decimal("0.52").unwrap(),
            PmQuantity::parse_decimal("10").unwrap(),
            1_780_449_126_930,
        )
        .unwrap()
}

fn cancel(order_id: &str) -> PmExactOwnedCancelRequest {
    execution()
        .cancel_command(
            instrument_scope(MARKET, 1_234),
            client_order(),
            PmVenueOrderKey::new(account().handle(), PmVenueOrderId::new(order_id).unwrap()),
        )
        .unwrap()
}

fn signer() -> FixedEoaSigner {
    FixedEoaSigner::bind(EoaPrivateKeyInput::new(TEST_KEY.into()), ADDRESS).unwrap()
}

fn credentials(address: &str) -> L2Credentials {
    L2Credentials::bind(
        address,
        L2CredentialInput::new(API_KEY.into(), API_SECRET.into(), PASSPHRASE.into()),
    )
    .unwrap()
}

fn configs() -> (PmPrivateHttpConfig, PmUserWsConfig) {
    let scope = PmWireScope::new(
        PmConditionId::parse(CONDITION).unwrap(),
        PmMarketId::parse(MARKET).unwrap(),
        token(1_234),
    );
    (
        PmPrivateHttpConfig::loopback_evidence(
            "http://127.0.0.1:18080",
            Duration::from_millis(100),
            Duration::from_secs(1),
            scope,
        )
        .unwrap(),
        PmUserWsConfig::loopback_evidence(
            "ws://127.0.0.1:18081/ws/user",
            scope.condition(),
            Duration::from_millis(100),
            Duration::from_millis(500),
            Duration::from_millis(100),
            Duration::from_millis(50),
            4_096,
            1,
            Duration::from_millis(10),
            8,
            ConnectionEpoch::new(1),
        )
        .unwrap(),
    )
}

fn observation_grant_for(
    okx_instrument: &str,
    market: &str,
    token_id: u64,
) -> PmPublicObservationGrant {
    PmPublicObservationGrant::derive_goal_f(
        OkxReferenceInstrument::index(OkxInstrumentId::new(okx_instrument).unwrap()),
        PmInstrumentId::new(PmMarketId::parse(market).unwrap(), token(token_id)),
    )
}

fn observation_grant(okx_instrument: &str) -> PmPublicObservationGrant {
    observation_grant_for(okx_instrument, MARKET, 1_234)
}

fn owner_with_slot_and_configuration(
    slot: &str,
    okx_instrument: &str,
) -> PmLoopbackMutationConnectivityOwner {
    let execution = execution();
    let scope = instrument_scope(MARKET, 1_234);
    let (http, user_ws) = configs();
    PmLoopbackMutationConnectivityOwner::new(
        http,
        user_ws,
        account(),
        instrument(),
        PmGoalFTradingDomain::from_metadata(scope.metadata()).unwrap(),
        execution.place_profile(),
        execution.cancel_purpose(),
        observation_grant(okx_instrument),
        CredentialSlotId::new(slot.into()).unwrap(),
        signer(),
        credentials(ADDRESS),
    )
    .unwrap()
}

fn owner_with_slot(slot: &str) -> PmLoopbackMutationConnectivityOwner {
    owner_with_slot_and_configuration(slot, "BTC-USDT")
}

fn owner() -> PmLoopbackMutationConnectivityOwner {
    owner_with_slot(CREDENTIAL_SLOT)
}

#[tokio::test]
async fn exact_neutral_place_and_cancel_are_signed_authenticated_and_retained() {
    let debug = format!("{:?}", owner());
    assert!(debug.contains("[REDACTED]"));
    assert!(!debug.contains(TEST_KEY));
    assert!(!debug.contains(API_KEY));
    assert!(!debug.contains(API_SECRET));
    assert!(!debug.contains(PASSPHRASE));

    let roles = owner().split().unwrap();
    let (_http, _user_ws, mut place_role, mut cancel_role, binding, fingerprint, supervisor) =
        roles.into_roles();
    assert_eq!(
        binding.configuration_fingerprint(),
        observation_grant("BTC-USDT").configuration_fingerprint()
    );
    assert_eq!(binding.account(), account());
    assert_eq!(binding.instrument(), instrument());
    assert_eq!(
        binding.trading_domain(),
        PmGoalFTradingDomain::from_metadata(metadata(MARKET, 1_234)).unwrap()
    );
    assert_eq!(binding.wire_scope(), configs().0.exact_order_scope());
    assert_ne!(
        fingerprint.into_authenticated_journal_scope_bytes(),
        [0; 32]
    );
    let request = place(instrument_scope(MARKET, 1_234));
    let expected =
        exact_order_amounts(request.side(), request.price(), request.quantity()).unwrap();
    let retained = place_role
        .authenticate_place(request, mutation_server_time(AUTH_SECONDS))
        .await
        .unwrap();
    assert_eq!(retained.l2_timestamp_seconds(), AUTH_SECONDS);
    assert_eq!(retained.expected_making_amount(), expected.maker());
    assert_eq!(retained.expected_taking_amount(), expected.taker());

    let retained = cancel_role
        .authenticate_cancel(cancel(VENUE_ORDER), mutation_server_time(AUTH_SECONDS + 1))
        .await
        .unwrap();
    assert_eq!(retained.order_id().to_string(), VENUE_ORDER);
    assert_eq!(retained.l2_timestamp_seconds(), AUTH_SECONDS + 1);

    supervisor.shutdown().await.unwrap();
    let failure = cancel_role
        .authenticate_cancel(cancel(VENUE_ORDER), mutation_server_time(AUTH_SECONDS + 2))
        .await
        .unwrap_err();
    assert_eq!(
        failure.reason(),
        PmLoopbackMutationAuthError::AuthorityClosed
    );
    assert_eq!(
        failure.into_request().venue_order().id().as_str(),
        VENUE_ORDER
    );
}

#[tokio::test]
async fn bounded_time_response_is_the_exact_retained_l2_header_timestamp() {
    let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let address = listener.local_addr().unwrap();
    let server = tokio::spawn(async move {
        let (mut stream, _) = listener.accept().await.unwrap();
        let mut request = Vec::new();
        let mut chunk = [0_u8; 512];
        loop {
            let read = stream.read(&mut chunk).await.unwrap();
            request.extend_from_slice(&chunk[..read]);
            if read == 0 || request.windows(4).any(|window| window == b"\r\n\r\n") {
                break;
            }
        }
        assert!(request.starts_with(b"GET /time HTTP/1.1\r\n"));
        let body = AUTH_SECONDS.to_string();
        stream
            .write_all(
                format!(
                    "HTTP/1.1 200 OK\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{body}",
                    body.len()
                )
                .as_bytes(),
            )
            .await
            .unwrap();
    });
    let scope = configs().0.exact_order_scope();
    let http_config = PmPublicHttpConfig::loopback_evidence(
        &format!("http://{address}"),
        Duration::from_secs(1),
        Duration::from_secs(1),
    )
    .unwrap();
    let clock = PmProductClockOwner::test_support_scripted(&[
        (1_700_000_000_000_000_100, 100),
        (1_700_000_000_000_000_101, 101),
    ])
    .unwrap();
    let (_, _, http_clock, _, _, _, _, _, _, mut validator) = clock.split().into_views();
    let time_role = PmPublicHttpRole::with_product_clock(
        http_config,
        PmBookParserConfig::new_condition_bound(
            scope,
            PmTick::parse_decimal("0.01").unwrap(),
            PmQuantity::parse_decimal("5").unwrap(),
            false,
        ),
        http_clock,
    )
    .unwrap();
    let authorized = validator
        .authorize(time_role.fresh_mutation_server_time().await.unwrap())
        .unwrap();

    let roles = owner().split().unwrap();
    let (_http, _user_ws, mut place_role, _cancel, _, _, supervisor) = roles.into_roles();
    let retained = place_role
        .authenticate_place(place(instrument_scope(MARKET, 1_234)), authorized)
        .await
        .unwrap();
    assert_eq!(retained.l2_timestamp_seconds(), AUTH_SECONDS);
    supervisor.shutdown().await.unwrap();
    server.await.unwrap();
}

#[tokio::test]
async fn foreign_instrument_and_noncanonical_cancel_id_fail_before_authentication() {
    let roles = owner().split().unwrap();
    let (_http, _user_ws, mut place_role, mut cancel_role, _, _fingerprint, supervisor) =
        roles.into_roles();
    let failure = place_role
        .authenticate_place(
            place(instrument_scope(FOREIGN_MARKET, 9_999)),
            mutation_server_time(AUTH_SECONDS),
        )
        .await
        .unwrap_err();
    assert_eq!(
        failure.reason(),
        PmLoopbackMutationAuthError::PlaceScopeMismatch
    );
    let retained = failure.into_request();
    assert_eq!(
        retained.instrument_id().market(),
        PmMarketId::parse(FOREIGN_MARKET).unwrap()
    );
    assert_eq!(retained.instrument_id().token(), token(9_999));

    let failure = cancel_role
        .authenticate_cancel(
            cancel("not-canonical-order-id"),
            mutation_server_time(AUTH_SECONDS),
        )
        .await
        .unwrap_err();
    assert_eq!(
        failure.reason(),
        PmLoopbackMutationAuthError::Auth(PmAuthError::InvalidOrderId)
    );
    assert_eq!(
        failure.into_request().venue_order().id().as_str(),
        "not-canonical-order-id"
    );
    supervisor.shutdown().await.unwrap();
}

#[test]
fn constructor_rejects_a_second_l2_identity_and_surface_is_redacted() {
    let execution = execution();
    let scope = instrument_scope(MARKET, 1_234);
    let (http, user_ws) = configs();
    assert!(matches!(
        PmLoopbackMutationConnectivityOwner::new(
            http,
            user_ws,
            account(),
            instrument(),
            PmGoalFTradingDomain::from_metadata(scope.metadata()).unwrap(),
            execution.place_profile(),
            execution.cancel_purpose(),
            observation_grant("BTC-USDT"),
            CredentialSlotId::new(CREDENTIAL_SLOT.into()).unwrap(),
            signer(),
            credentials(FOREIGN_ADDRESS),
        ),
        Err(PmLoopbackMutationAuthError::InvalidConfiguration(_))
    ));
}

#[test]
fn constructor_rejects_an_observation_grant_for_a_different_pm_instrument() {
    let execution = execution();
    let scope = instrument_scope(MARKET, 1_234);
    let (http, user_ws) = configs();
    assert!(matches!(
        PmLoopbackMutationConnectivityOwner::new(
            http,
            user_ws,
            account(),
            instrument(),
            PmGoalFTradingDomain::from_metadata(scope.metadata()).unwrap(),
            execution.place_profile(),
            execution.cancel_purpose(),
            observation_grant_for("BTC-USDT", FOREIGN_MARKET, 9_999),
            CredentialSlotId::new(CREDENTIAL_SLOT.into()).unwrap(),
            signer(),
            credentials(ADDRESS),
        ),
        Err(PmLoopbackMutationAuthError::InvalidConfiguration(
            "public observation grant must match the bound instrument and trading domain"
        ))
    ));
}

#[tokio::test]
async fn credential_slot_fingerprint_is_derived_by_the_sole_credential_owner() {
    let first = owner_with_slot("same-slot").split().unwrap();
    let second = owner_with_slot("same-slot").split().unwrap();
    let changed = owner_with_slot("different-slot").split().unwrap();
    let (_, _, _, _, _, first_fingerprint, first_supervisor) = first.into_roles();
    let (_, _, _, _, _, second_fingerprint, second_supervisor) = second.into_roles();
    let (_, _, _, _, _, changed_fingerprint, changed_supervisor) = changed.into_roles();

    assert_eq!(first_fingerprint, second_fingerprint);
    assert_ne!(first_fingerprint, changed_fingerprint);
    first_supervisor.shutdown().await.unwrap();
    second_supervisor.shutdown().await.unwrap();
    changed_supervisor.shutdown().await.unwrap();
}

#[tokio::test]
async fn role_bundle_preserves_the_exact_typed_configuration_fingerprint() {
    let first = owner_with_slot_and_configuration("same-slot", "BTC-USDT")
        .split()
        .unwrap();
    let changed = owner_with_slot_and_configuration("same-slot", "ETH-USDT")
        .split()
        .unwrap();
    let (_, _, _, _, first_binding, _, first_supervisor) = first.into_roles();
    let (_, _, _, _, changed_binding, _, changed_supervisor) = changed.into_roles();

    assert_eq!(
        first_binding.configuration_fingerprint(),
        observation_grant("BTC-USDT").configuration_fingerprint()
    );
    assert_eq!(
        changed_binding.configuration_fingerprint(),
        observation_grant("ETH-USDT").configuration_fingerprint()
    );
    assert_ne!(
        first_binding.configuration_fingerprint(),
        changed_binding.configuration_fingerprint()
    );
    first_supervisor.shutdown().await.unwrap();
    changed_supervisor.shutdown().await.unwrap();
}

#[tokio::test]
async fn saturated_place_and_cancel_admission_return_the_exact_neutral_requests() {
    let (place_sender, _place_receiver) = mpsc::channel(1);
    let (first_response, _first_receive) = oneshot::channel();
    place_sender
        .try_send(PlaceAuthenticationRequest {
            request: Arc::new(place(instrument_scope(MARKET, 1_234))),
            timestamp: L2Timestamp::from_unix_seconds(AUTH_SECONDS).unwrap(),
            response: first_response,
        })
        .unwrap();
    let mut place_role = PmLoopbackPlaceAuthenticationRole {
        sender: place_sender,
    };
    let expected_place = place(instrument_scope(FOREIGN_MARKET, 9_999));
    let expected_market = expected_place.instrument_id().market();
    let failure = place_role
        .authenticate_place(expected_place, mutation_server_time(AUTH_SECONDS + 1))
        .await
        .unwrap_err();
    assert_eq!(
        failure.reason(),
        PmLoopbackMutationAuthError::AuthoritySaturated
    );
    assert_eq!(
        failure.into_request().instrument_id().market(),
        expected_market
    );

    let (cancel_sender, _cancel_receiver) = mpsc::channel(1);
    let (first_response, _first_receive) = oneshot::channel();
    cancel_sender
        .try_send(CancelAuthenticationRequest {
            request: Arc::new(cancel(VENUE_ORDER)),
            timestamp: L2Timestamp::from_unix_seconds(AUTH_SECONDS).unwrap(),
            response: first_response,
        })
        .unwrap();
    let mut cancel_role = PmLoopbackCancelAuthenticationRole {
        sender: cancel_sender,
    };
    let expected_cancel = cancel("not-canonical-order-id");
    let failure = cancel_role
        .authenticate_cancel(expected_cancel, mutation_server_time(AUTH_SECONDS + 1))
        .await
        .unwrap_err();
    assert_eq!(
        failure.reason(),
        PmLoopbackMutationAuthError::AuthoritySaturated
    );
    assert_eq!(
        failure.into_request().venue_order().id().as_str(),
        "not-canonical-order-id"
    );
}

#[tokio::test]
async fn closed_place_and_cancel_channels_return_the_exact_neutral_requests() {
    let (place_sender, place_receiver) = mpsc::channel(1);
    drop(place_receiver);
    let mut place_role = PmLoopbackPlaceAuthenticationRole {
        sender: place_sender,
    };
    let expected_place = place(instrument_scope(FOREIGN_MARKET, 9_999));
    let expected_token = expected_place.instrument_id().token();
    let failure = place_role
        .authenticate_place(expected_place, mutation_server_time(AUTH_SECONDS))
        .await
        .unwrap_err();
    assert_eq!(
        failure.reason(),
        PmLoopbackMutationAuthError::AuthorityClosed
    );
    assert_eq!(
        failure.into_request().instrument_id().token(),
        expected_token
    );

    let (cancel_sender, cancel_receiver) = mpsc::channel(1);
    drop(cancel_receiver);
    let mut cancel_role = PmLoopbackCancelAuthenticationRole {
        sender: cancel_sender,
    };
    let failure = cancel_role
        .authenticate_cancel(cancel(VENUE_ORDER), mutation_server_time(AUTH_SECONDS))
        .await
        .unwrap_err();
    assert_eq!(
        failure.reason(),
        PmLoopbackMutationAuthError::AuthorityClosed
    );
    assert_eq!(
        failure.into_request().venue_order().id().as_str(),
        VENUE_ORDER
    );
}
