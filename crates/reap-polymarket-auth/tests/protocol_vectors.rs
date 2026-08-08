use std::convert::Infallible;

use reap_pm_core::{
    EvmAddress, PmConditionId, PmOrderSalt, PmOrderSide, PmPrice, PmQuantity, PmTick, PmTokenId,
    U256,
};
use reap_polymarket_auth::{
    AuthenticatedUserSubscriptionSink, EoaPrivateKeyInput, FixedEoaSigner, FixedOrderId,
    FixedOwnedCancelRequestSink, FixedPlaceRequestSink, L2CredentialInput, L2Credentials,
    L2HeaderSink, L2Timestamp, OwnedCancelSemanticRequestCommitment,
    PlaceSemanticRequestCommitment, PmClobDomain, RuntimeExactBodyCommitment,
};
use reap_polymarket_wire::{PmLiveUserEvent, PmUnsignedClobV2Order, parse_live_user_frame};

const TEST_KEY: &str = "0xac0974bec39a17e36ba4a6b4d238ff944bacb478cbed5efcae784d7bf4f2ff80";
const ADDRESS: &str = "0xf39Fd6e51aad88F6F4ce6aB8827279cffFb92266";
const API_KEY: &str = "00000000-0000-4000-8000-000000000001";
const API_SECRET: &str = "AAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAA=";
const PASSPHRASE: &str = "synthetic-passphrase";
const OTHER_API_KEY: &str = "00000000-0000-4000-8000-000000000002";
const OTHER_API_SECRET: &str = "AQEBAQEBAQEBAQEBAQEBAQEBAQEBAQEBAQEBAQEBAQE=";
const OTHER_PASSPHRASE: &str = "other-synthetic-passphrase";
const AUTH_SECONDS: u64 = 1_780_449_126;
const EXPECTED_BUY_ID: &str = "0xfaf10599783c69b375a0f0d948d37eb711ec042dbf7d52fc2f8d8832d71af7f1";

fn signer() -> FixedEoaSigner {
    FixedEoaSigner::bind(EoaPrivateKeyInput::new(TEST_KEY.into()), ADDRESS).unwrap()
}

fn credentials() -> L2Credentials {
    credentials_with(API_KEY, API_SECRET, PASSPHRASE)
}

fn credentials_with(api_key: &str, api_secret: &str, passphrase: &str) -> L2Credentials {
    L2Credentials::bind(
        ADDRESS,
        L2CredentialInput::new(api_key.into(), api_secret.into(), passphrase.into()),
    )
    .unwrap()
}

fn user_order_owner(owner: &str) -> reap_polymarket_wire::PmLiveUserFrame {
    let raw = format!(
        r#"{{"event_type":"order","id":"0xaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa","market":"0x1111111111111111111111111111111111111111111111111111111111111111","asset_id":"1234","side":"BUY","original_size":"10.000000","size_matched":"0","price":"0.520000","type":"PLACEMENT","owner":"{owner}","order_owner":"{owner}","maker_address":"{ADDRESS}","expiration":"0","order_type":"GTC","outcome":"Yes","status":"LIVE","created_at":"1780449126","associate_trades":null,"timestamp":"1780449126930"}}"#
    );
    parse_live_user_frame(raw.as_bytes()).unwrap()
}

fn vector_order(side: PmOrderSide) -> PmUnsignedClobV2Order {
    let address = EvmAddress::parse(ADDRESS).unwrap();
    PmUnsignedClobV2Order::new_goal_f(
        PmOrderSalt::from_u64(479_249_096_354).unwrap(),
        address,
        address,
        PmTokenId::new(U256::from_u64(1_234)).unwrap(),
        side,
        PmPrice::parse_decimal("0.52").unwrap(),
        PmQuantity::parse_decimal("10").unwrap(),
        PmTick::parse_decimal("0.01").unwrap(),
        PmQuantity::parse_decimal("5").unwrap(),
        1_780_449_126_930,
    )
    .unwrap()
}

#[derive(Default)]
struct MutationCapture {
    address: String,
    signature: String,
    timestamp: String,
    api_key: String,
    passphrase: String,
    expected_making_amount: Option<U256>,
    expected_taking_amount: Option<U256>,
    body: Vec<u8>,
}

impl FixedPlaceRequestSink for MutationCapture {
    type Output = ();
    type Error = Infallible;

    #[allow(
        clippy::too_many_arguments,
        reason = "the test sink captures the complete fixed-purpose trait boundary"
    )]
    fn send_gtc_post_only(
        &mut self,
        poly_address: &str,
        poly_signature: &str,
        poly_timestamp: &str,
        poly_api_key: &str,
        poly_passphrase: &str,
        expected_making_amount: U256,
        expected_taking_amount: U256,
        exact_body: &[u8],
    ) -> Result<Self::Output, Self::Error> {
        self.expected_making_amount = Some(expected_making_amount);
        self.expected_taking_amount = Some(expected_taking_amount);
        self.capture(
            poly_address,
            poly_signature,
            poly_timestamp,
            poly_api_key,
            poly_passphrase,
            exact_body,
        );
        Ok(())
    }
}

impl FixedOwnedCancelRequestSink for MutationCapture {
    type Output = ();
    type Error = Infallible;

    fn send_exact_owned_cancel(
        &mut self,
        poly_address: &str,
        poly_signature: &str,
        poly_timestamp: &str,
        poly_api_key: &str,
        poly_passphrase: &str,
        exact_body: &[u8],
    ) -> Result<Self::Output, Self::Error> {
        self.capture(
            poly_address,
            poly_signature,
            poly_timestamp,
            poly_api_key,
            poly_passphrase,
            exact_body,
        );
        Ok(())
    }
}

impl MutationCapture {
    fn capture(
        &mut self,
        address: &str,
        signature: &str,
        timestamp: &str,
        api_key: &str,
        passphrase: &str,
        body: &[u8],
    ) {
        self.address = address.into();
        self.signature = signature.into();
        self.timestamp = timestamp.into();
        self.api_key = api_key.into();
        self.passphrase = passphrase.into();
        self.body = body.into();
    }
}

#[derive(Default)]
struct HeaderCapture(MutationCapture);

impl L2HeaderSink for HeaderCapture {
    type Error = Infallible;

    fn set_polymarket_l2_headers(
        &mut self,
        poly_address: &str,
        poly_signature: &str,
        poly_timestamp: &str,
        poly_api_key: &str,
        poly_passphrase: &str,
    ) -> Result<(), Self::Error> {
        self.0.capture(
            poly_address,
            poly_signature,
            poly_timestamp,
            poly_api_key,
            poly_passphrase,
            &[],
        );
        Ok(())
    }
}

#[derive(Default)]
struct FrameCapture(Vec<u8>);

impl AuthenticatedUserSubscriptionSink for FrameCapture {
    type Output = ();
    type Error = Infallible;

    fn send_user_subscription(&mut self, exact_frame: &[u8]) -> Result<Self::Output, Self::Error> {
        self.0.extend_from_slice(exact_frame);
        Ok(())
    }
}

fn capture_place(domain: PmClobDomain, side: PmOrderSide) -> MutationCapture {
    let signer = signer();
    let credentials = credentials();
    capture_place_with(&signer, &credentials, domain, side).transport
}

struct PlaceCommitmentCapture {
    runtime: RuntimeExactBodyCommitment,
    semantic: PlaceSemanticRequestCommitment,
    transport: MutationCapture,
}

fn capture_place_with(
    signer: &FixedEoaSigner,
    credentials: &L2Credentials,
    domain: PmClobDomain,
    side: PmOrderSide,
) -> PlaceCommitmentCapture {
    let signed = signer
        .sign_clob_v2_order(domain, vector_order(side))
        .unwrap();
    let body = credentials.serialize_gtc_post_only(signed).unwrap();
    let runtime = body.runtime_exact_body_commitment();
    let semantic = body.semantic_request_commitment();
    let request = credentials
        .authenticate_place(L2Timestamp::from_unix_seconds(AUTH_SECONDS).unwrap(), body)
        .unwrap();
    assert_eq!(request.runtime_exact_body_commitment(), runtime);
    assert_eq!(request.semantic_request_commitment(), semantic);
    let mut transport = MutationCapture::default();
    request.dispatch(&mut transport).unwrap();
    PlaceCommitmentCapture {
        runtime,
        semantic,
        transport,
    }
}

struct CancelCommitmentCapture {
    runtime: RuntimeExactBodyCommitment,
    semantic: OwnedCancelSemanticRequestCommitment,
    transport: MutationCapture,
}

fn capture_cancel_with(
    credentials: &L2Credentials,
    order_id: FixedOrderId,
) -> CancelCommitmentCapture {
    let body = credentials.serialize_owned_cancel(order_id).unwrap();
    let runtime = body.runtime_exact_body_commitment();
    let semantic = body.semantic_request_commitment();
    let request = credentials
        .authenticate_owned_cancel(L2Timestamp::from_unix_seconds(AUTH_SECONDS).unwrap(), body)
        .unwrap();
    assert_eq!(request.runtime_exact_body_commitment(), runtime);
    assert_eq!(request.semantic_request_commitment(), semantic);
    let mut transport = MutationCapture::default();
    request.dispatch(&mut transport).unwrap();
    CancelCommitmentCapture {
        runtime,
        semantic,
        transport,
    }
}

/// Frozen from the independently authored viem/Solidity vector recorded in
/// the PM-T1 local protocol cut. The pinned Predarb object corroborates the
/// separate timestamp+method+route+body L2 construction, not the V2 digest.
#[test]
fn standard_buy_body_signature_identity_and_l2_hmac_match_exactly() {
    let capture = capture_place(PmClobDomain::Standard, PmOrderSide::Buy);
    let expected_body = r#"{"deferExec":false,"order":{"builder":"0x0000000000000000000000000000000000000000000000000000000000000000","expiration":"0","maker":"0xf39Fd6e51aad88F6F4ce6aB8827279cffFb92266","makerAmount":"5200000","metadata":"0x0000000000000000000000000000000000000000000000000000000000000000","salt":479249096354,"side":"BUY","signature":"0xbb81b245ea7ebb9aa480ccbf15364a2cb2cd77d7adebcb56fd5f49b653683110055a3d5ad05adf1aa65b1701bf25c622275f098fd5724c7f782671829e6d4d0b1b","signatureType":0,"signer":"0xf39Fd6e51aad88F6F4ce6aB8827279cffFb92266","takerAmount":"10000000","timestamp":"1780449126930","tokenId":"1234"},"orderType":"GTC","owner":"00000000-0000-4000-8000-000000000001","postOnly":true}"#;
    assert_eq!(capture.body, expected_body.as_bytes());
    assert_eq!(capture.body.len(), 685);
    assert_eq!(
        capture.signature,
        "rdkpCVcu-66xB2VbkOlUXQ2PLaCeqv3LjgFBfkrQdqo="
    );
    assert_eq!(capture.address, ADDRESS);
    assert_eq!(capture.timestamp, AUTH_SECONDS.to_string());
    assert_eq!(capture.api_key, API_KEY);
    assert_eq!(capture.passphrase, PASSPHRASE);
    assert_eq!(
        capture.expected_making_amount,
        Some(U256::from_u64(5_200_000))
    );
    assert_eq!(
        capture.expected_taking_amount,
        Some(U256::from_u64(10_000_000))
    );

    let body: serde_json::Value = serde_json::from_slice(&capture.body).unwrap();
    assert_eq!(
        body["order"]["signature"],
        "0xbb81b245ea7ebb9aa480ccbf15364a2cb2cd77d7adebcb56fd5f49b653683110055a3d5ad05adf1aa65b1701bf25c622275f098fd5724c7f782671829e6d4d0b1b"
    );
}

fn lower_hex(bytes: [u8; 32]) -> String {
    bytes
        .into_iter()
        .map(|byte| format!("{byte:02x}"))
        .collect()
}

#[test]
fn runtime_exact_body_commitments_match_the_frozen_sha256_vectors() {
    let signer = signer();
    let credentials = credentials();
    let signed = signer
        .sign_clob_v2_order(PmClobDomain::Standard, vector_order(PmOrderSide::Buy))
        .unwrap();
    let place = credentials.serialize_gtc_post_only(signed).unwrap();
    assert_eq!(
        lower_hex(place.runtime_exact_body_commitment().runtime_only_bytes()),
        "6a016c150aa3605270289489beff1d54d735952db64948d2bb3c1e01beb0ae9f"
    );
    assert_eq!(
        format!("{:?}", place.runtime_exact_body_commitment()),
        "RuntimeExactBodyCommitment([REDACTED; NON_DURABLE])"
    );
    assert_eq!(place.expected_order_id().to_string(), EXPECTED_BUY_ID);

    let cancel = credentials
        .serialize_owned_cancel(FixedOrderId::parse(EXPECTED_BUY_ID).unwrap())
        .unwrap();
    assert_eq!(
        lower_hex(cancel.runtime_exact_body_commitment().runtime_only_bytes()),
        "90c066349138ab5e31d12e10006859cb239f8cbab5b46e5b0bec00c974536022"
    );
}

#[test]
fn place_semantics_ignore_credentials_while_runtime_body_and_hmac_change_as_applicable() {
    let signer = signer();
    let baseline = capture_place_with(
        &signer,
        &credentials_with(API_KEY, API_SECRET, PASSPHRASE),
        PmClobDomain::Standard,
        PmOrderSide::Buy,
    );
    let changed_api_key = capture_place_with(
        &signer,
        &credentials_with(OTHER_API_KEY, API_SECRET, PASSPHRASE),
        PmClobDomain::Standard,
        PmOrderSide::Buy,
    );
    let changed_secret = capture_place_with(
        &signer,
        &credentials_with(API_KEY, OTHER_API_SECRET, PASSPHRASE),
        PmClobDomain::Standard,
        PmOrderSide::Buy,
    );
    let changed_passphrase = capture_place_with(
        &signer,
        &credentials_with(API_KEY, API_SECRET, OTHER_PASSPHRASE),
        PmClobDomain::Standard,
        PmOrderSide::Buy,
    );

    for candidate in [&changed_api_key, &changed_secret, &changed_passphrase] {
        assert_eq!(candidate.semantic, baseline.semantic);
    }

    assert_ne!(changed_api_key.runtime, baseline.runtime);
    assert_ne!(changed_api_key.transport.body, baseline.transport.body);
    assert_ne!(
        changed_api_key.transport.signature,
        baseline.transport.signature
    );
    assert_ne!(
        changed_api_key.transport.api_key,
        baseline.transport.api_key
    );

    assert_eq!(changed_secret.runtime, baseline.runtime);
    assert_eq!(changed_secret.transport.body, baseline.transport.body);
    assert_ne!(
        changed_secret.transport.signature,
        baseline.transport.signature
    );

    assert_eq!(changed_passphrase.runtime, baseline.runtime);
    assert_eq!(changed_passphrase.transport.body, baseline.transport.body);
    assert_eq!(
        changed_passphrase.transport.signature,
        baseline.transport.signature
    );
    assert_ne!(
        changed_passphrase.transport.passphrase,
        baseline.transport.passphrase
    );
}

#[test]
fn cancel_semantics_ignore_credentials_and_exact_body_stays_secret_free() {
    let order_id = FixedOrderId::parse(EXPECTED_BUY_ID).unwrap();
    let baseline =
        capture_cancel_with(&credentials_with(API_KEY, API_SECRET, PASSPHRASE), order_id);
    let changed_api_key = capture_cancel_with(
        &credentials_with(OTHER_API_KEY, API_SECRET, PASSPHRASE),
        order_id,
    );
    let changed_secret = capture_cancel_with(
        &credentials_with(API_KEY, OTHER_API_SECRET, PASSPHRASE),
        order_id,
    );
    let changed_passphrase = capture_cancel_with(
        &credentials_with(API_KEY, API_SECRET, OTHER_PASSPHRASE),
        order_id,
    );

    for candidate in [&changed_api_key, &changed_secret, &changed_passphrase] {
        assert_eq!(candidate.semantic, baseline.semantic);
        assert_eq!(candidate.runtime, baseline.runtime);
        assert_eq!(candidate.transport.body, baseline.transport.body);
    }
    assert_eq!(
        changed_api_key.transport.signature,
        baseline.transport.signature
    );
    assert_ne!(
        changed_api_key.transport.api_key,
        baseline.transport.api_key
    );
    assert_ne!(
        changed_secret.transport.signature,
        baseline.transport.signature
    );
    assert_eq!(
        changed_passphrase.transport.signature,
        baseline.transport.signature
    );
    assert_ne!(
        changed_passphrase.transport.passphrase,
        baseline.transport.passphrase
    );
}

#[test]
fn zero_salt_remains_valid_under_the_existing_goal_f_contract() {
    let address = EvmAddress::parse(ADDRESS).unwrap();
    let order = PmUnsignedClobV2Order::new_goal_f(
        PmOrderSalt::from_u64(0).unwrap(),
        address,
        address,
        PmTokenId::new(U256::from_u64(1_234)).unwrap(),
        PmOrderSide::Buy,
        PmPrice::parse_decimal("0.52").unwrap(),
        PmQuantity::parse_decimal("10").unwrap(),
        PmTick::parse_decimal("0.01").unwrap(),
        PmQuantity::parse_decimal("5").unwrap(),
        1_780_449_126_930,
    )
    .unwrap();
    assert!(
        signer()
            .sign_clob_v2_order(PmClobDomain::Standard, order)
            .is_ok()
    );
}

#[test]
fn all_domain_side_vectors_are_separated_byte_for_byte() {
    let cases = [
        (
            PmClobDomain::Standard,
            PmOrderSide::Buy,
            EXPECTED_BUY_ID,
            "0xbb81b245ea7ebb9aa480ccbf15364a2cb2cd77d7adebcb56fd5f49b653683110055a3d5ad05adf1aa65b1701bf25c622275f098fd5724c7f782671829e6d4d0b1b",
        ),
        (
            PmClobDomain::Standard,
            PmOrderSide::Sell,
            "0x4983a6499acac0e05a059b91ca92f61885b4d0327e1031570aa54ff85bc0af88",
            "0x2a2a3b104cea6c5b4645ecddd73cec80ac82c8dd030d704be85f640a1dfefdb14d6fe935e87708d79625e97e660454292919af24f886b9f37ef3403b10962b101b",
        ),
        (
            PmClobDomain::NegativeRisk,
            PmOrderSide::Buy,
            "0x51541d6f12464aff462c280fc2fd0c73a0e0959752cc4e8f6e32c5c3107fc8e7",
            "0xe3c2789e5a479cc64032caccf2e124eba6ae292d2e67e070df75bac13dfcfa5a65b1245dec0c40f91be50d1622bbfc079533c3e58d157ba73fcfd7799aa81f371b",
        ),
        (
            PmClobDomain::NegativeRisk,
            PmOrderSide::Sell,
            "0x192ba059050c799921996285a4c182309e64843af24b77f1f7f0507dc3d15899",
            "0x72ef19c759a0e9bd0ff6faa55e93ffc6c2f761605a128ad00599861d54413cda35302af513f457eac180d8259d8c320ee07f989a1addd8120713a2caea95ac211c",
        ),
    ];

    for (domain, side, expected_id, expected_signature) in cases {
        let signer = signer();
        let signed = signer
            .sign_clob_v2_order(domain, vector_order(side))
            .unwrap();
        assert_eq!(signed.expected_order_id().to_string(), expected_id);
        let credentials = credentials();
        let body = credentials.serialize_gtc_post_only(signed).unwrap();
        let request = credentials
            .authenticate_place(L2Timestamp::from_unix_seconds(AUTH_SECONDS).unwrap(), body)
            .unwrap();
        let mut capture = MutationCapture::default();
        request.dispatch(&mut capture).unwrap();
        let value: serde_json::Value = serde_json::from_slice(&capture.body).unwrap();
        assert_eq!(value["order"]["signature"], expected_signature);
        assert_eq!(
            value["order"]["makerAmount"],
            if side == PmOrderSide::Buy {
                "5200000"
            } else {
                "10000000"
            }
        );
        assert_eq!(
            value["order"]["takerAmount"],
            if side == PmOrderSide::Buy {
                "10000000"
            } else {
                "5200000"
            }
        );
    }
}

#[test]
fn predarb_parity_get_route_excludes_transport_query() {
    let credentials = credentials();
    let headers = credentials
        .authenticate_open_orders(L2Timestamp::from_unix_seconds(AUTH_SECONDS).unwrap())
        .unwrap();
    let mut capture = HeaderCapture::default();
    headers.apply_to(&mut capture).unwrap();
    assert_eq!(
        capture.0.signature,
        "-PRhfdrU6Jmzz04syaATDpRblz8zwfPYnigpmfrQVEE="
    );
    assert_eq!(capture.0.address, ADDRESS);
    assert_eq!(capture.0.timestamp, AUTH_SECONDS.to_string());
}

#[test]
fn exact_owned_cancel_body_and_l2_hmac_match() {
    let credentials = credentials();
    let order_id = FixedOrderId::parse(EXPECTED_BUY_ID).unwrap();
    let body = credentials.serialize_owned_cancel(order_id).unwrap();
    let request = credentials
        .authenticate_owned_cancel(L2Timestamp::from_unix_seconds(AUTH_SECONDS).unwrap(), body)
        .unwrap();
    let mut capture = MutationCapture::default();
    request.dispatch(&mut capture).unwrap();
    assert_eq!(
        capture.body,
        br#"{"orderID":"0xfaf10599783c69b375a0f0d948d37eb711ec042dbf7d52fc2f8d8832d71af7f1"}"#
    );
    assert_eq!(capture.body.len(), 80);
    assert_eq!(
        capture.signature,
        "beMewzBZba3V05Un8qQtGM0NYW4KFt4wKc8KqZcdo_M="
    );
}

#[test]
fn fixed_read_routes_have_distinct_auth_and_no_route_escape() {
    let credentials = credentials();
    let timestamp = L2Timestamp::from_unix_seconds(AUTH_SECONDS).unwrap();
    let order_id = FixedOrderId::parse(EXPECTED_BUY_ID).unwrap();
    let operations = [
        credentials.authenticate_open_orders(timestamp).unwrap(),
        credentials.authenticate_trades(timestamp).unwrap(),
        credentials
            .authenticate_balance_allowance(timestamp)
            .unwrap(),
        credentials
            .authenticate_order_detail(timestamp, order_id)
            .unwrap(),
    ];
    let mut signatures = Vec::new();
    for operation in operations {
        let mut capture = HeaderCapture::default();
        operation.apply_to(&mut capture).unwrap();
        signatures.push(capture.0.signature);
    }
    signatures.sort();
    signatures.dedup();
    assert_eq!(signatures.len(), 4);
}

#[test]
fn user_subscription_is_exact_market_bound_and_consume_once() {
    let credentials = credentials();
    let market =
        PmConditionId::parse("0x1111111111111111111111111111111111111111111111111111111111111111")
            .unwrap();
    let frame = credentials.user_subscription(market).unwrap();
    let mut capture = FrameCapture::default();
    frame.dispatch(&mut capture).unwrap();
    assert_eq!(
        capture.0,
        br#"{"auth":{"apiKey":"00000000-0000-4000-8000-000000000001","secret":"AAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAA=","passphrase":"synthetic-passphrase"},"markets":["0x1111111111111111111111111111111111111111111111111111111111111111"],"type":"user"}"#
    );
}

#[test]
fn credential_owner_matching_never_exposes_the_api_key() {
    let credentials = credentials();
    let matching = user_order_owner(API_KEY);
    let different = user_order_owner("00000000-0000-4000-8000-000000000002");
    let PmLiveUserEvent::Order(matching_order) = &matching.events()[0] else {
        panic!("matching owner fixture must be an order")
    };
    let PmLiveUserEvent::Order(different_order) = &different.events()[0] else {
        panic!("different owner fixture must be an order")
    };

    assert!(credentials.matches_credential_owner(matching_order.owner()));
    assert!(
        matching_order
            .order_owner()
            .is_some_and(|owner| credentials.matches_credential_owner(owner))
    );
    assert!(!credentials.matches_credential_owner(different_order.owner()));
    for rendered in [format!("{credentials:?}"), format!("{matching:?}")] {
        assert!(rendered.contains("REDACTED"));
        assert!(!rendered.contains(API_KEY));
        assert!(!rendered.contains(API_SECRET));
    }
}

#[test]
fn wrong_identity_timestamp_and_wire_encodings_fail_closed() {
    assert!(
        FixedEoaSigner::bind(
            EoaPrivateKeyInput::new(TEST_KEY.into()),
            "0x1111111111111111111111111111111111111111"
        )
        .is_err()
    );
    assert!(
        FixedEoaSigner::bind(
            EoaPrivateKeyInput::new(TEST_KEY.into()),
            &ADDRESS.to_ascii_lowercase()
        )
        .is_err()
    );
    assert!(L2Timestamp::from_unix_seconds(0).is_err());
    assert!(FixedOrderId::parse(&EXPECTED_BUY_ID.to_ascii_uppercase()).is_err());
    assert!(
        L2Credentials::bind(
            ADDRESS,
            L2CredentialInput::new(
                API_KEY.into(),
                API_SECRET.trim_end_matches('=').into(),
                PASSPHRASE.into()
            )
        )
        .is_err()
    );
}

#[test]
fn changed_domain_side_timestamp_and_material_change_outputs() {
    let standard = capture_place(PmClobDomain::Standard, PmOrderSide::Buy);
    let negative = capture_place(PmClobDomain::NegativeRisk, PmOrderSide::Buy);
    let sell = capture_place(PmClobDomain::Standard, PmOrderSide::Sell);
    assert_ne!(standard.body, negative.body);
    assert_ne!(standard.body, sell.body);
    assert_ne!(standard.signature, negative.signature);
    assert_ne!(standard.signature, sell.signature);

    let credentials = credentials();
    let first = credentials
        .authenticate_open_orders(L2Timestamp::from_unix_seconds(AUTH_SECONDS).unwrap())
        .unwrap();
    let second = credentials
        .authenticate_open_orders(L2Timestamp::from_unix_seconds(AUTH_SECONDS + 1).unwrap())
        .unwrap();
    let mut first_capture = HeaderCapture::default();
    let mut second_capture = HeaderCapture::default();
    first.apply_to(&mut first_capture).unwrap();
    second.apply_to(&mut second_capture).unwrap();
    assert_ne!(first_capture.0.signature, second_capture.0.signature);
}
