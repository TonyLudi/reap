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
    PmExactOwnedCancelRequest, PmFixedMutationPreparation, PmFixtureInstrumentScope,
    PmFixtureOwnedExecution, PmGtcPostOnlyPlaceRequest,
};
use reap_polymarket_auth::{
    CredentialSlotId, EoaPrivateKeyInput, FixedEoaSigner, L2CredentialInput, L2Credentials,
    L2Timestamp,
};
use reap_polymarket_wire::{PmBookParserConfig, PmClobV2SignatureType, PmWireScope};
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

fn proxy_account() -> PmAccountScope {
    let base = account();
    PmAccountScope::new(
        base.environment(),
        base.chain(),
        base.signer(),
        PmFunderId::new(address(FOREIGN_ADDRESS)),
        base.handle(),
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

fn proxy_place(scope: PmFixtureInstrumentScope) -> PmGtcPostOnlyPlaceRequest {
    PmFixedMutationPreparation::new_pm_t2_proxy(proxy_account(), instrument())
        .prepare_place(
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

fn proxy_cancel(order_id: &str) -> PmExactOwnedCancelRequest {
    PmFixedMutationPreparation::new_pm_t2_proxy(proxy_account(), instrument())
        .prepare_cancel(
            instrument_scope(MARKET, 1_234),
            client_order(),
            PmVenueOrderKey::new(
                proxy_account().handle(),
                PmVenueOrderId::new(order_id).unwrap(),
            ),
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
    configs_with_http_origin("http://127.0.0.1:18080")
}

fn configs_with_http_origin(origin: &str) -> (PmPrivateHttpConfig, PmUserWsConfig) {
    let scope = PmWireScope::new(
        PmConditionId::parse(CONDITION).unwrap(),
        PmMarketId::parse(MARKET).unwrap(),
        token(1_234),
    );
    (
        PmPrivateHttpConfig::loopback_evidence(
            origin,
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

fn proxy_owner_with_configs(
    http: PmPrivateHttpConfig,
    user_ws: PmUserWsConfig,
) -> PmLoopbackMutationConnectivityOwner {
    let execution = execution();
    let scope = instrument_scope(MARKET, 1_234);
    PmLoopbackMutationConnectivityOwner::new_pm_t2_proxy(
        http,
        user_ws,
        proxy_account(),
        instrument(),
        PmGoalFTradingDomain::from_metadata(scope.metadata()).unwrap(),
        execution.place_profile(),
        execution.cancel_purpose(),
        observation_grant("BTC-USDT"),
        CredentialSlotId::new(CREDENTIAL_SLOT.into()).unwrap(),
        signer(),
        credentials(ADDRESS),
    )
    .unwrap()
}

fn try_proxy_owner(
    account: PmAccountScope,
    credentials: L2Credentials,
) -> Result<PmLoopbackMutationConnectivityOwner, PmLoopbackMutationAuthError> {
    let execution = execution();
    let scope = instrument_scope(MARKET, 1_234);
    let (http, user_ws) = configs();
    PmLoopbackMutationConnectivityOwner::new_pm_t2_proxy(
        http,
        user_ws,
        account,
        instrument(),
        PmGoalFTradingDomain::from_metadata(scope.metadata()).unwrap(),
        execution.place_profile(),
        execution.cancel_purpose(),
        observation_grant("BTC-USDT"),
        CredentialSlotId::new(CREDENTIAL_SLOT.into()).unwrap(),
        signer(),
        credentials,
    )
}

async fn mock_http_server(
    responses: Vec<(u16, String)>,
) -> (String, mpsc::Receiver<Vec<u8>>, tokio::task::JoinHandle<()>) {
    let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let address = listener.local_addr().unwrap();
    let (requests, receiver) = mpsc::channel(responses.len());
    let task = tokio::spawn(async move {
        for (status, body) in responses {
            let (mut stream, _) = listener.accept().await.unwrap();
            let request = read_http_request(&mut stream).await;
            requests.send(request).await.unwrap();
            let reason = if status == 200 { "OK" } else { "Unauthorized" };
            stream
                .write_all(
                    format!(
                        "HTTP/1.1 {status} {reason}\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{body}",
                        body.len()
                    )
                    .as_bytes(),
                )
                .await
                .unwrap();
        }
    });
    (format!("http://{address}"), receiver, task)
}

async fn read_http_request(stream: &mut tokio::net::TcpStream) -> Vec<u8> {
    let mut request = Vec::new();
    let mut chunk = [0_u8; 1_024];
    loop {
        let read = stream.read(&mut chunk).await.unwrap();
        request.extend_from_slice(&chunk[..read]);
        let Some(header_end) = request
            .windows(4)
            .position(|window| window == b"\r\n\r\n")
            .map(|index| index + 4)
        else {
            assert_ne!(read, 0, "request closed before headers completed");
            continue;
        };
        let headers = std::str::from_utf8(&request[..header_end]).unwrap();
        let content_length = headers
            .lines()
            .find_map(|line| {
                line.to_ascii_lowercase()
                    .strip_prefix("content-length: ")
                    .map(str::to_owned)
            })
            .map_or(0, |value| value.parse().unwrap());
        if request.len() >= header_end + content_length {
            return request;
        }
        assert_ne!(read, 0, "request closed before body completed");
    }
}

fn request_header<'a>(request: &'a [u8], name: &str) -> &'a str {
    let request = std::str::from_utf8(request).unwrap();
    request
        .lines()
        .find_map(|line| {
            let (candidate, value) = line.split_once(": ")?;
            candidate.eq_ignore_ascii_case(name).then_some(value)
        })
        .unwrap()
}

fn request_body(request: &[u8]) -> &[u8] {
    let header_end = request
        .windows(4)
        .position(|window| window == b"\r\n\r\n")
        .unwrap()
        + 4;
    &request[header_end..]
}

fn exact_order_body(maker: &str) -> String {
    format!(
        r#"{{"id":"{VENUE_ORDER}","market":"{CONDITION}","asset_id":"1234","side":"BUY","original_size":"10","size_matched":"0","price":"0.52","status":"LIVE","maker_address":"{maker}","owner":"{API_KEY}","expiration":"0","created_at":1700000000,"outcome":"Yes","order_type":"GTC"}}"#
    )
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
    assert_eq!(binding.signature_profile(), PmClobV2SignatureType::Eoa);
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
async fn pm_t2_proxy_custody_binds_reads_place_semantics_and_exact_cancel() {
    let balance_body = r#"{"balance":"1000","allowances":{}}"#.to_owned();
    let (private_origin, mut private_requests, private_server) = mock_http_server(vec![
        (200, balance_body),
        (200, exact_order_body(FOREIGN_ADDRESS)),
    ])
    .await;
    let (http_config, user_ws_config) = configs_with_http_origin(&private_origin);
    let roles = proxy_owner_with_configs(http_config, user_ws_config)
        .split()
        .unwrap();
    let (mut http, _user_ws, mut place_role, mut cancel_role, binding, _, supervisor) =
        roles.into_roles();

    assert_eq!(binding.account(), proxy_account());
    assert_eq!(binding.signature_profile(), PmClobV2SignatureType::Proxy);
    let read_time = crate::product_clock::test_support_read_server_time(AUTH_SECONDS);
    http.account()
        .collateral_balance_allowance(read_time)
        .await
        .unwrap();
    let observation = http
        .reconciliation()
        .exact_local_order_detail(
            crate::product_clock::test_support_read_server_time(AUTH_SECONDS),
            FixedOrderId::parse(VENUE_ORDER).unwrap(),
        )
        .await
        .unwrap();
    assert!(matches!(
        observation,
        crate::PmExactOrderObservation::Present(_)
    ));

    let balance_request = private_requests.recv().await.unwrap();
    assert_eq!(
        std::str::from_utf8(&balance_request)
            .unwrap()
            .lines()
            .next()
            .unwrap(),
        "GET /balance-allowance?asset_type=COLLATERAL&signature_type=1 HTTP/1.1"
    );
    assert_eq!(request_header(&balance_request, "poly_address"), ADDRESS);
    let detail_request = private_requests.recv().await.unwrap();
    assert_eq!(
        std::str::from_utf8(&detail_request)
            .unwrap()
            .lines()
            .next()
            .unwrap(),
        format!("GET /data/order/{VENUE_ORDER} HTTP/1.1")
    );
    assert_eq!(request_header(&detail_request, "poly_address"), ADDRESS);
    private_server.await.unwrap();

    let retained_place = place_role
        .authenticate_place(
            proxy_place(instrument_scope(MARKET, 1_234)),
            mutation_server_time(AUTH_SECONDS),
        )
        .await
        .unwrap();
    let semantic_commitment = retained_place.semantic_request_commitment();
    assert_ne!(semantic_commitment.bytes(), [0; 32]);
    let (place_origin, mut place_requests, place_server) =
        mock_http_server(vec![(401, "{}".to_owned())]).await;
    let mutation_config = crate::PmLoopbackMutationConfig::loopback_evidence(
        &place_origin,
        Duration::from_secs(1),
        Duration::from_secs(1),
    )
    .unwrap();
    let outcome = crate::PmFixedPlaceLoopbackRole::new(mutation_config)
        .unwrap()
        .send(retained_place)
        .await;
    assert_eq!(outcome.semantic_request_commitment(), semantic_commitment);
    let place_request = place_requests.recv().await.unwrap();
    assert_eq!(
        std::str::from_utf8(&place_request)
            .unwrap()
            .lines()
            .next()
            .unwrap(),
        "POST /order HTTP/1.1"
    );
    assert_eq!(request_header(&place_request, "poly_address"), ADDRESS);
    let body = std::str::from_utf8(request_body(&place_request)).unwrap();
    for required in [
        format!(r#""maker":"{FOREIGN_ADDRESS}""#),
        format!(r#""signer":"{ADDRESS}""#),
        r#""signatureType":1"#.to_owned(),
        r#""orderType":"GTC""#.to_owned(),
        r#""postOnly":true"#.to_owned(),
        r#""deferExec":false"#.to_owned(),
    ] {
        assert!(
            body.contains(&required),
            "missing proxy place fact {required}"
        );
    }
    place_server.await.unwrap();

    let retained_cancel = cancel_role
        .authenticate_cancel(
            proxy_cancel(VENUE_ORDER),
            mutation_server_time(AUTH_SECONDS + 1),
        )
        .await
        .unwrap();
    assert_eq!(retained_cancel.order_id().to_string(), VENUE_ORDER);
    let cancel_semantic = retained_cancel.semantic_request_commitment();
    let (cancel_origin, mut cancel_requests, cancel_server) =
        mock_http_server(vec![(401, "{}".to_owned())]).await;
    let mutation_config = crate::PmLoopbackMutationConfig::loopback_evidence(
        &cancel_origin,
        Duration::from_secs(1),
        Duration::from_secs(1),
    )
    .unwrap();
    let outcome = crate::PmExactOwnedCancelLoopbackRole::new(mutation_config)
        .unwrap()
        .send(retained_cancel)
        .await;
    assert_eq!(outcome.semantic_request_commitment(), cancel_semantic);
    let cancel_request = cancel_requests.recv().await.unwrap();
    assert_eq!(
        std::str::from_utf8(&cancel_request)
            .unwrap()
            .lines()
            .next()
            .unwrap(),
        "DELETE /order HTTP/1.1"
    );
    assert_eq!(request_header(&cancel_request, "poly_address"), ADDRESS);
    assert_eq!(
        std::str::from_utf8(request_body(&cancel_request)).unwrap(),
        format!(r#"{{"orderID":"{VENUE_ORDER}"}}"#)
    );
    cancel_server.await.unwrap();
    supervisor.shutdown().await.unwrap();
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
    let (_, _, http_clock, _, _, _, _, _, _, mut validator) =
        clock.split().into_loopback_mutation_views();
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
fn pm_t2_proxy_constructor_rejects_equal_swapped_and_wrong_l2_identities() {
    assert!(matches!(
        try_proxy_owner(account(), credentials(ADDRESS)),
        Err(PmLoopbackMutationAuthError::InvalidConfiguration(
            "PM-T2 proxy funder must be nonzero and distinct from the signer EOA"
        ))
    ));

    let base = account();
    let swapped = PmAccountScope::new(
        base.environment(),
        base.chain(),
        PmSignerId::new(address(FOREIGN_ADDRESS)),
        PmFunderId::new(address(ADDRESS)),
        base.handle(),
    );
    assert!(matches!(
        try_proxy_owner(swapped, credentials(ADDRESS)),
        Err(PmLoopbackMutationAuthError::InvalidConfiguration(
            "private-key signer, L2 credentials, and account signer must be one EOA"
        ))
    ));

    assert!(matches!(
        try_proxy_owner(proxy_account(), credentials(FOREIGN_ADDRESS)),
        Err(PmLoopbackMutationAuthError::InvalidConfiguration(
            "private-key signer, L2 credentials, and account signer must be one EOA"
        ))
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
