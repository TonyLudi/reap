use std::{
    convert::Infallible,
    future::Future,
    path::{Path, PathBuf},
};

use base64::{Engine as _, engine::general_purpose::URL_SAFE};
use reap_pm_core::{
    ConnectionEpoch, EvmAddress, IngressSequence, OkxInstrumentId, OkxReferenceInstrument,
    PmAccountScope, PmConnectionId, PmFunderId, PmOrderSalt, PmOrderSide, PmPrice, PmQuantity,
    PmSignerId, U256, exact_order_amounts,
};
use reap_pm_live_contracts::{
    PmAccountConnectivityConfig, PmConnectivityConfig, PmPublicConnectivityConfig,
};
use reap_polymarket_auth::{
    AuthenticatedUserSubscriptionSink, CredentialSlotId, EoaPrivateKeyInput, FixedEoaSigner,
    FixedPlaceRequestSink, L2CredentialInput, L2Credentials, L2Timestamp, PmClobDomain,
};
use reap_polymarket_wire::PmUnsignedClobV2Order;
use sha2::{Digest as _, Sha256};

use super::*;
use crate::authenticated_journal::{
    PmAuthenticatedJournalError, PmAuthenticatedJournalRecordV1,
    PmAuthenticatedJournalRecoveryError, PmAuthenticatedJournalScopeV1,
    PmAuthenticatedMutationJournal, PmAuthenticatedPendingRecord, PmAuthenticatedPlacePreparedV1,
    PmAuthenticatedPlaceResultAcknowledged, PmAuthenticatedPlaceResultV1,
    PmAuthenticatedPreparedAcknowledged, PmAuthenticatedReceiptPoll, PmAuthenticatedSendGrant,
};
use crate::journal::{
    PmJournalAuthenticatedPlaceResultV1, PmJournalAuthenticatedResultV1, PmJournalFillCursorV1,
    PmJournalFillOccurrenceV1, PmJournalFillWatermarkV1, PmJournalFingerprintV1, PmJournalHeaderV1,
    PmJournalImmediateFillsV1, PmJournalLineV1, PmJournalOrderProgressSourceV1,
    PmJournalOrderTerminalV1, PmJournalPlaceOutcomeV1, PmJournalPlaceResultV1,
    PmJournalQuoteIntentV1, PmJournalQuoteProfileV1, PmJournalRecordV1, PmJournalRecovery,
    PmJournalScopeV1, PmJournalTerminalStatusV1, recover_pm_mutation_journal,
};

const L2_TIMESTAMP_SECONDS: u64 = 1_760_000_000;
const EXPECTED_ORDER_ID: [u8; 32] = [0x55; 32];
const LARGE_JOURNAL_TEST_STACK_BYTES: usize = 8 * 1_024 * 1_024;

const ARTIFACT_PRIVATE_KEY_CANARY: &str =
    "0xac0974bec39a17e36ba4a6b4d238ff944bacb478cbed5efcae784d7bf4f2ff80";
const ARTIFACT_EOA_ADDRESS: &str = "0xf39Fd6e51aad88F6F4ce6aB8827279cffFb92266";
const ARTIFACT_API_KEY_CANARY: &str = "decafbad-0000-4000-8000-00000000cafe";
const ARTIFACT_HMAC_SECRET_CANARY: &str = "AQIDBAUGBwgJCgsMDQ4PEBESExQVFhcYGRobHB0eHyA=";
const ARTIFACT_PASSPHRASE_CANARY: &str = "artifact-canary-passphrase-2026";

#[derive(Default)]
struct AuthenticatedPlaceArtifactCapture {
    address: String,
    l2_signature: String,
    timestamp: String,
    api_key: String,
    passphrase: String,
    making_amount: Option<U256>,
    taking_amount: Option<U256>,
    body: Vec<u8>,
}

impl FixedPlaceRequestSink for AuthenticatedPlaceArtifactCapture {
    type Output = ();
    type Error = Infallible;

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
        self.address = poly_address.to_owned();
        self.l2_signature = poly_signature.to_owned();
        self.timestamp = poly_timestamp.to_owned();
        self.api_key = poly_api_key.to_owned();
        self.passphrase = poly_passphrase.to_owned();
        self.making_amount = Some(expected_making_amount);
        self.taking_amount = Some(expected_taking_amount);
        self.body.extend_from_slice(exact_body);
        Ok(())
    }
}

#[derive(Default)]
struct UserAuthFrameArtifactCapture(Vec<u8>);

impl AuthenticatedUserSubscriptionSink for UserAuthFrameArtifactCapture {
    type Output = ();
    type Error = Infallible;

    fn send_user_subscription(&mut self, exact_frame: &[u8]) -> Result<Self::Output, Self::Error> {
        self.0.extend_from_slice(exact_frame);
        Ok(())
    }
}

fn artifact_config(address: EvmAddress) -> PmConnectivityConfig {
    let base = crate::evidence::connectivity_config();
    let base_account = base.account().account_scope();
    let account_scope = PmAccountScope::new(
        base_account.environment(),
        base_account.chain(),
        PmSignerId::new(address),
        PmFunderId::new(address),
        base_account.handle(),
    );
    let public = base.public().clone();
    let account = PmAccountConnectivityConfig::derive_goal_f(
        &public,
        account_scope,
        base.account().account_route(),
    )
    .expect("artifact account configuration");
    PmConnectivityConfig::new(public, account).expect("artifact connectivity configuration")
}

fn lower_hex(bytes: &[u8]) -> String {
    const HEX: &[u8; 16] = b"0123456789abcdef";
    let mut encoded = String::with_capacity(bytes.len() * 2);
    for byte in bytes {
        encoded.push(char::from(HEX[usize::from(byte >> 4)]));
        encoded.push(char::from(HEX[usize::from(byte & 0x0f)]));
    }
    encoded
}

fn decode_prefixed_hex(value: &str) -> Vec<u8> {
    let value = value.strip_prefix("0x").unwrap_or(value);
    assert_eq!(value.len() % 2, 0, "hex canary must contain whole bytes");
    value
        .as_bytes()
        .chunks_exact(2)
        .map(|pair| {
            let pair = std::str::from_utf8(pair).expect("ASCII hex canary");
            u8::from_str_radix(pair, 16).expect("valid hex canary")
        })
        .collect()
}

fn forbidden_artifact_encodings(raw: &[u8]) -> Vec<Vec<u8>> {
    assert!(!raw.is_empty(), "empty artifact canary");
    let digest: [u8; 32] = Sha256::digest(raw).into();
    let mut encodings = Vec::with_capacity(10);
    for value in [raw, digest.as_slice()] {
        encodings.push(value.to_vec());
        let lower = lower_hex(value);
        let upper = lower.to_ascii_uppercase();
        encodings.push(lower.as_bytes().to_vec());
        encodings.push(upper.as_bytes().to_vec());
        encodings.push(format!("0x{lower}").into_bytes());
        encodings.push(format!("0X{upper}").into_bytes());
    }
    encodings.sort();
    encodings.dedup();
    encodings
}

fn contains_bytes(haystack: &[u8], needle: &[u8]) -> bool {
    haystack
        .windows(needle.len())
        .any(|candidate| candidate == needle)
}

fn assert_canary_absent(artifacts: &[(&str, &[u8])], canary_name: &str, representations: &[&[u8]]) {
    for representation in representations {
        for forbidden in forbidden_artifact_encodings(representation) {
            for (artifact_name, artifact) in artifacts {
                assert!(
                    !contains_bytes(artifact, &forbidden),
                    "{artifact_name} persisted a forbidden {canary_name} representation"
                );
            }
        }
    }
}

fn collect_rust_sources(directory: &Path, sources: &mut Vec<PathBuf>) {
    for entry in std::fs::read_dir(directory).expect("read pm-live source directory") {
        let path = entry.expect("read pm-live source entry").path();
        if path.is_dir() {
            collect_rust_sources(&path, sources);
        } else if path.extension().is_some_and(|extension| extension == "rs") {
            sources.push(path);
        }
    }
}

fn assert_runtime_body_bytes_are_test_only() {
    let manifest = Path::new(env!("CARGO_MANIFEST_DIR"));
    let source_root = manifest.join("src");
    let this_test = source_root.join("coordinator/authenticated_recovery/tests.rs");
    let accessor = ["runtime_", "only_bytes()"].concat();
    let mut sources = Vec::new();
    collect_rust_sources(&source_root, &mut sources);

    let mut test_occurrences = 0;
    for source in sources {
        let contents = std::fs::read_to_string(&source).expect("read pm-live Rust source");
        let occurrences = contents.matches(&accessor).count();
        if source == this_test {
            test_occurrences += occurrences;
        } else {
            assert_eq!(
                occurrences,
                0,
                "production pm-live source {} accesses runtime-only body bytes",
                source.display()
            );
        }
    }
    assert_eq!(
        test_occurrences, 1,
        "canary test owns the sole accessor use"
    );
}

fn run_large_recovery_test<F, Fut>(name: &str, test: F)
where
    F: FnOnce() -> Fut + Send + 'static,
    Fut: Future<Output = ()> + 'static,
{
    std::thread::Builder::new()
        .name(name.into())
        .stack_size(LARGE_JOURNAL_TEST_STACK_BYTES)
        .spawn(|| {
            tokio::runtime::Builder::new_current_thread()
                .enable_all()
                .build()
                .expect("recovery test runtime")
                .block_on(test());
        })
        .expect("spawn bounded recovery test thread")
        .join()
        .expect("bounded recovery test thread panicked");
}

#[derive(Clone, Copy)]
enum AuthTail {
    Empty,
    PreparedOnly,
    GrantTail,
    Accepted,
}

fn scopes() -> (
    PmConnectivityConfig,
    PmJournalScopeV1,
    PmAuthenticatedJournalScopeV1,
) {
    let config = crate::evidence::connectivity_config();
    let goal_f = PmJournalScopeV1::from_config(&config).expect("Goal-F scope");
    let authenticated = PmAuthenticatedJournalScopeV1::from_config(&config, [0x44; 32])
        .expect("authenticated scope");
    assert_eq!(goal_f.account_scope(), authenticated.account_scope());
    assert_eq!(goal_f.instrument(), authenticated.instrument());
    assert_eq!(
        goal_f.configuration_fingerprint().bytes(),
        authenticated.configuration_fingerprint()
    );
    (config, goal_f, authenticated)
}

fn alternate_reference_config(base: &PmConnectivityConfig) -> PmConnectivityConfig {
    let public = PmPublicConnectivityConfig::derive_goal_f(
        OkxReferenceInstrument::index(
            OkxInstrumentId::new("ETH-USDT").expect("alternate reference instrument"),
        ),
        base.public().expected_metadata(),
        base.public().okx_route(),
        base.public().polymarket_route(),
    )
    .expect("alternate public configuration");
    let account = PmAccountConnectivityConfig::derive_goal_f(
        &public,
        base.account().account_scope(),
        base.account().account_route(),
    )
    .expect("alternate account configuration");
    PmConnectivityConfig::new(public, account).expect("alternate connectivity configuration")
}

fn quote(scope: &PmJournalScopeV1) -> PmJournalQuoteIntentV1 {
    let side = PmOrderSide::Buy;
    let price = PmPrice::from_units(500_000).expect("price");
    let quantity = PmQuantity::parse_decimal("5").expect("quantity");
    let amounts = exact_order_amounts(side, price, quantity).expect("exact order amounts");
    let account = scope.account_scope();
    PmJournalQuoteIntentV1 {
        intent_id: 1,
        client_order: scope.client_order_for_intent(1).expect("client order"),
        instrument: scope.instrument(),
        side: side.into(),
        price_units: price.units(),
        quantity,
        reserved_collateral: amounts.maker(),
        reserved_outcome: U256::ZERO,
        profile: PmJournalQuoteProfileV1::PassiveGtcPostOnlyEoa,
        metadata_revision: 1,
        book_revision: 2,
        model_revision: 3,
        book_readiness_revision: 4,
        private_readiness_revision: 5,
        expires_at_monotonic_ns: 10_000,
        salt: PmOrderSalt::from_u64(1).expect("salt"),
        timestamp_ms: 1_760_000_000_000,
        maker: account.funder().address(),
        signer: account.signer().address(),
        maker_amount: amounts.maker(),
        taker_amount: amounts.taker(),
    }
}

fn write_goal_f(
    path: &Path,
    scope: &PmJournalScopeV1,
    records: impl IntoIterator<Item = PmJournalRecordV1>,
) -> PmJournalRecovery {
    let mut bytes = Vec::new();
    append_goal_f(
        &mut bytes,
        scope,
        0,
        PmJournalRecordV1::Header(PmJournalHeaderV1::new(scope.clone())),
    );
    for (offset, record) in records.into_iter().enumerate() {
        append_goal_f(
            &mut bytes,
            scope,
            u64::try_from(offset).expect("bounded test record count") + 1,
            record,
        );
    }
    std::fs::write(path, bytes).expect("write Goal-F test journal");
    recover_pm_mutation_journal(path, scope).expect("recover Goal-F test journal")
}

fn append_goal_f(
    bytes: &mut Vec<u8>,
    scope: &PmJournalScopeV1,
    sequence: u64,
    record: PmJournalRecordV1,
) {
    serde_json::to_writer(
        &mut *bytes,
        &PmJournalLineV1::new(scope.fingerprint(), sequence, record),
    )
    .expect("encode Goal-F line");
    bytes.push(b'\n');
}

fn prepared(
    scope: &PmAuthenticatedJournalScopeV1,
    intent: PmJournalQuoteIntentV1,
) -> PmAuthenticatedPlacePreparedV1 {
    PmAuthenticatedPlacePreparedV1::new(
        scope,
        crate::authenticated_journal::PmAuthenticatedCoordinatorIdentityV1::new(
            intent.client_order,
            intent.instrument,
        ),
        1,
        [0x33; 32],
        EXPECTED_ORDER_ID,
        L2_TIMESTAMP_SECONDS,
    )
    .expect("valid authenticated Prepared")
}

async fn authenticated_recovery(
    path: &Path,
    scope: &PmAuthenticatedJournalScopeV1,
    intent: PmJournalQuoteIntentV1,
    tail: AuthTail,
) -> PmAuthenticatedJournalRecovery {
    let (mut journal, initial) =
        PmAuthenticatedMutationJournal::start(path.to_path_buf(), scope.clone())
            .await
            .expect("start authenticated journal");
    assert_eq!(initial.record_count(), 0);

    if !matches!(tail, AuthTail::Empty) {
        let value = prepared(scope, intent);
        let acknowledged = acknowledge_prepared(
            journal
                .try_record(PmAuthenticatedJournalRecordV1::PlacePrepared(value))
                .expect("record Prepared"),
        )
        .await;
        match tail {
            AuthTail::Empty => unreachable!("empty tail was excluded"),
            AuthTail::PreparedOnly => drop(acknowledged),
            AuthTail::GrantTail | AuthTail::Accepted => {
                let grant = acknowledge_send_grant(
                    journal
                        .try_authorize_dispatch(acknowledged)
                        .expect("record durable send grant"),
                )
                .await;
                let PmAuthenticatedSendGrant::Place(grant) = grant else {
                    panic!("place Prepared produced cancel grant")
                };
                if matches!(tail, AuthTail::Accepted) {
                    let result = PmAuthenticatedPlaceResultV1::accepted(
                        crate::authenticated_journal::PmAuthenticatedCoordinatorIdentityV1::new(
                            intent.client_order,
                            intent.instrument,
                        ),
                        grant.grant_sequence(),
                        EXPECTED_ORDER_ID,
                    );
                    drop(
                        acknowledge_place_result(
                            journal
                                .try_record_place_result(grant, result)
                                .expect("record accepted result"),
                        )
                        .await,
                    );
                } else {
                    drop(grant);
                }
            }
        }
    }
    journal.shutdown().await.expect("shutdown writer");

    let (journal, recovery) =
        PmAuthenticatedMutationJournal::start(path.to_path_buf(), scope.clone())
            .await
            .expect("recover authenticated journal");
    journal.shutdown().await.expect("shutdown recovery writer");
    recovery
}

async fn acknowledge_prepared(
    mut pending: PmAuthenticatedPendingRecord,
) -> PmAuthenticatedPreparedAcknowledged {
    for _ in 0..10_000 {
        match pending.poll() {
            PmAuthenticatedReceiptPoll::Pending(next) => {
                pending = next;
                tokio::task::yield_now().await;
            }
            PmAuthenticatedReceiptPoll::PreparedAcknowledged(acknowledged) => {
                return acknowledged;
            }
            PmAuthenticatedReceiptPoll::Failed(message) => panic!("durability failed: {message}"),
            PmAuthenticatedReceiptPoll::Closed => panic!("writer closed"),
            PmAuthenticatedReceiptPoll::SendGranted(_)
            | PmAuthenticatedReceiptPoll::PlaceResultAcknowledged(_)
            | PmAuthenticatedReceiptPoll::CancelResultAcknowledged(_)
            | PmAuthenticatedReceiptPoll::PlaceResultFailed { .. }
            | PmAuthenticatedReceiptPoll::CancelResultFailed { .. }
            | PmAuthenticatedReceiptPoll::PlaceResultClosed(_)
            | PmAuthenticatedReceiptPoll::CancelResultClosed(_) => {
                panic!("wrong acknowledgement kind")
            }
        }
    }
    panic!("Prepared acknowledgement did not arrive")
}

async fn acknowledge_send_grant(
    mut pending: PmAuthenticatedPendingRecord,
) -> PmAuthenticatedSendGrant {
    for _ in 0..10_000 {
        match pending.poll() {
            PmAuthenticatedReceiptPoll::Pending(next) => {
                pending = next;
                tokio::task::yield_now().await;
            }
            PmAuthenticatedReceiptPoll::SendGranted(grant) => return grant,
            PmAuthenticatedReceiptPoll::Failed(message) => panic!("durability failed: {message}"),
            PmAuthenticatedReceiptPoll::Closed => panic!("writer closed"),
            PmAuthenticatedReceiptPoll::PreparedAcknowledged(_)
            | PmAuthenticatedReceiptPoll::PlaceResultAcknowledged(_)
            | PmAuthenticatedReceiptPoll::CancelResultAcknowledged(_)
            | PmAuthenticatedReceiptPoll::PlaceResultFailed { .. }
            | PmAuthenticatedReceiptPoll::CancelResultFailed { .. }
            | PmAuthenticatedReceiptPoll::PlaceResultClosed(_)
            | PmAuthenticatedReceiptPoll::CancelResultClosed(_) => {
                panic!("wrong acknowledgement kind")
            }
        }
    }
    panic!("send-grant acknowledgement did not arrive")
}

async fn acknowledge_place_result(
    mut pending: PmAuthenticatedPendingRecord,
) -> PmAuthenticatedPlaceResultAcknowledged {
    for _ in 0..10_000 {
        match pending.poll() {
            PmAuthenticatedReceiptPoll::Pending(next) => {
                pending = next;
                tokio::task::yield_now().await;
            }
            PmAuthenticatedReceiptPoll::PlaceResultAcknowledged(acknowledged) => {
                return acknowledged;
            }
            PmAuthenticatedReceiptPoll::Failed(message) => panic!("durability failed: {message}"),
            PmAuthenticatedReceiptPoll::Closed => panic!("writer closed"),
            PmAuthenticatedReceiptPoll::PlaceResultFailed { message, .. } => {
                panic!("place-result durability failed: {message}")
            }
            PmAuthenticatedReceiptPoll::PlaceResultClosed(_) => {
                panic!("place-result writer closed")
            }
            PmAuthenticatedReceiptPoll::PreparedAcknowledged(_)
            | PmAuthenticatedReceiptPoll::SendGranted(_)
            | PmAuthenticatedReceiptPoll::CancelResultAcknowledged(_)
            | PmAuthenticatedReceiptPoll::CancelResultFailed { .. }
            | PmAuthenticatedReceiptPoll::CancelResultClosed(_) => {
                panic!("wrong acknowledgement kind")
            }
        }
    }
    panic!("result acknowledgement did not arrive")
}

fn only_recovered_result(
    recovery: &PmAuthenticatedJournalRecovery,
) -> PmAuthenticatedRecoveredResultV1 {
    let [result] = recovery.classified_results() else {
        panic!("expected exactly one recovered classified result")
    };
    *result
}

fn exact_bridge(recovery: &PmAuthenticatedJournalRecovery) -> PmJournalAuthenticatedResultV1 {
    bridge_from_recovered(&only_recovered_result(recovery)).expect("exact bridge")
}

fn changed_commitment_bridge(
    exact: PmJournalAuthenticatedResultV1,
) -> PmJournalAuthenticatedResultV1 {
    let PmJournalAuthenticatedResultV1::Place(exact) = exact else {
        panic!("place fixture produced cancel bridge")
    };
    let mut changed_commitment = exact.request_commitment();
    changed_commitment[0] ^= 0xff;
    PmJournalAuthenticatedResultV1::Place(
        PmJournalAuthenticatedPlaceResultV1::new(
            exact.auth_prepared_sequence(),
            exact.auth_grant_sequence(),
            exact.auth_result_sequence(),
            exact.prior_goal_f_sequence(),
            exact.canonical().client_order,
            exact.instrument(),
            changed_commitment,
            exact.expected_order_id(),
            exact.observed_order_id(),
            exact.canonical().venue_order,
            exact.classification(),
        )
        .expect("shape-valid mismatched bridge"),
    )
}

#[tokio::test]
async fn prepared_only_is_definitely_unsent_but_restart_requires_reconciliation() {
    let directory = tempfile::tempdir().expect("temporary directory");
    let (_, goal_scope, auth_scope) = scopes();
    let intent = quote(&goal_scope);
    let goal_f = write_goal_f(
        &directory.path().join("goal-f-prepared.jsonl"),
        &goal_scope,
        [PmJournalRecordV1::QuoteIntent(intent)],
    );
    let authenticated = authenticated_recovery(
        &directory.path().join("auth-prepared.jsonl"),
        &auth_scope,
        intent,
        AuthTail::PreparedOnly,
    )
    .await;

    let gate = PmAuthenticatedRecoveryGate::validate(&goal_f, &authenticated)
        .expect("valid Prepared-only pairing");
    assert_eq!(authenticated.prepared_without_grant().len(), 1);
    assert!(authenticated.unresolved_operations().is_empty());
    assert!(gate.missing_bridges().is_empty());
    assert!(gate.requires_reconciliation());
}

#[tokio::test]
async fn durable_grant_tail_requires_reconciliation_and_never_mints_a_bridge() {
    let directory = tempfile::tempdir().expect("temporary directory");
    let (_, goal_scope, auth_scope) = scopes();
    let intent = quote(&goal_scope);
    let goal_f = write_goal_f(
        &directory.path().join("goal-f-grant.jsonl"),
        &goal_scope,
        [PmJournalRecordV1::QuoteIntent(intent)],
    );
    let authenticated = authenticated_recovery(
        &directory.path().join("auth-grant.jsonl"),
        &auth_scope,
        intent,
        AuthTail::GrantTail,
    )
    .await;

    let gate = PmAuthenticatedRecoveryGate::validate(&goal_f, &authenticated)
        .expect("valid grant-tail pairing");
    assert!(authenticated.prepared_without_grant().is_empty());
    assert_eq!(authenticated.unresolved_operations().len(), 1);
    assert!(gate.missing_bridges().is_empty());
    assert!(
        gate.missing_bridge_records()
            .expect("bridge projection")
            .is_empty()
    );
    assert!(gate.requires_reconciliation());
}

#[tokio::test]
async fn durable_result_without_goal_f_bridge_projects_one_exact_repair() {
    let directory = tempfile::tempdir().expect("temporary directory");
    let (_, goal_scope, auth_scope) = scopes();
    let intent = quote(&goal_scope);
    let goal_f = write_goal_f(
        &directory.path().join("goal-f-missing-bridge.jsonl"),
        &goal_scope,
        [PmJournalRecordV1::QuoteIntent(intent)],
    );
    let authenticated = authenticated_recovery(
        &directory.path().join("auth-result.jsonl"),
        &auth_scope,
        intent,
        AuthTail::Accepted,
    )
    .await;

    let gate = PmAuthenticatedRecoveryGate::validate(&goal_f, &authenticated)
        .expect("valid result-to-intent pairing");
    assert_eq!(gate.missing_bridges(), authenticated.classified_results());
    assert_eq!(
        gate.missing_bridge_records().expect("repair projection"),
        vec![exact_bridge(&authenticated)]
    );
    assert!(!gate.requires_reconciliation());
}

#[test]
fn exact_goal_f_bridge_closes_the_only_repairable_crash_gap() {
    run_large_recovery_test("pm-auth-exact-bridge", || async {
        let directory = tempfile::tempdir().expect("temporary directory");
        let (_, goal_scope, auth_scope) = scopes();
        let intent = quote(&goal_scope);
        let authenticated = authenticated_recovery(
            &directory.path().join("auth-exact-bridge.jsonl"),
            &auth_scope,
            intent,
            AuthTail::Accepted,
        )
        .await;
        let bridge = exact_bridge(&authenticated);
        let goal_f = write_goal_f(
            &directory.path().join("goal-f-exact-bridge.jsonl"),
            &goal_scope,
            [
                PmJournalRecordV1::QuoteIntent(intent),
                PmJournalRecordV1::AuthenticatedResult(bridge),
            ],
        );

        let gate = PmAuthenticatedRecoveryGate::validate(&goal_f, &authenticated)
            .expect("exact bridge pairing");
        assert!(gate.missing_bridges().is_empty());
        assert!(!gate.requires_reconciliation());
    });
}

#[test]
fn durable_artifacts_exclude_secret_and_runtime_exact_body_canaries() {
    run_large_recovery_test("pm-auth-durable-artifact-canaries", || async {
        let directory = tempfile::tempdir().expect("temporary directory");
        let signer = FixedEoaSigner::bind(
            EoaPrivateKeyInput::new(ARTIFACT_PRIVATE_KEY_CANARY.into()),
            ARTIFACT_EOA_ADDRESS,
        )
        .expect("synthetic fixed EOA signer");
        let credentials = L2Credentials::bind(
            ARTIFACT_EOA_ADDRESS,
            L2CredentialInput::new(
                ARTIFACT_API_KEY_CANARY.into(),
                ARTIFACT_HMAC_SECRET_CANARY.into(),
                ARTIFACT_PASSPHRASE_CANARY.into(),
            ),
        )
        .expect("synthetic L2 credentials");
        let config = artifact_config(signer.address().as_core());
        let public_capture_path = directory.path().join("public-canary.jsonl");
        let public_capture = crate::PmPublicCapture::new(config.public().clone())
            .expect("construct canary public capture")
            .start(
                public_capture_path.clone(),
                crate::evidence::authoritative(),
                crate::evidence::session_policy(),
                crate::evidence::provenance(),
            )
            .await
            .expect("start canary public capture");
        let public_capture_outcome = public_capture
            .finish()
            .await
            .expect("close canary public capture");
        assert_eq!(public_capture_outcome.verification().records, 1);
        assert_eq!(public_capture_outcome.path(), public_capture_path);
        let goal_scope = PmJournalScopeV1::from_config(&config).expect("artifact Goal-F scope");
        let credential_slot = credentials
            .authenticated_journal_credential_slot(
                CredentialSlotId::new("artifact-canary-slot-v1".into())
                    .expect("artifact credential slot"),
            )
            .into_authenticated_journal_scope_bytes();
        let auth_scope = PmAuthenticatedJournalScopeV1::from_config(&config, credential_slot)
            .expect("artifact authenticated scope");
        let intent = quote(&goal_scope);
        let metadata = config.public().expected_metadata();
        let domain = if metadata.negative_risk() {
            PmClobDomain::NegativeRisk
        } else {
            PmClobDomain::Standard
        };
        let unsigned = PmUnsignedClobV2Order::new_goal_f(
            intent.salt,
            intent.maker,
            intent.signer,
            metadata.outcome().token(),
            PmOrderSide::Buy,
            PmPrice::from_units(intent.price_units).expect("intent price"),
            intent.quantity,
            metadata.tick(),
            metadata.minimum_order_size(),
            intent.timestamp_ms,
        )
        .expect("artifact unsigned order");
        let signed = signer
            .sign_clob_v2_order(domain, unsigned)
            .expect("artifact signed order");
        let expected_order_id = signed.expected_order_id();
        let serialized = credentials
            .serialize_gtc_post_only(signed)
            .expect("artifact serialized place request");
        let runtime_commitment = serialized.runtime_exact_body_commitment();
        let semantic_commitment = serialized.semantic_request_commitment();
        let authenticated = credentials
            .authenticate_place(
                L2Timestamp::from_unix_seconds(L2_TIMESTAMP_SECONDS)
                    .expect("artifact L2 timestamp"),
                serialized,
            )
            .expect("artifact authenticated place request");
        assert_eq!(
            authenticated.runtime_exact_body_commitment(),
            runtime_commitment
        );
        assert_eq!(
            authenticated.semantic_request_commitment(),
            semantic_commitment
        );
        assert_eq!(authenticated.expected_order_id(), expected_order_id);
        let mut place_capture = AuthenticatedPlaceArtifactCapture::default();
        authenticated
            .dispatch(&mut place_capture)
            .expect("capture authenticated place request");
        assert_eq!(place_capture.address, ARTIFACT_EOA_ADDRESS);
        assert_eq!(place_capture.timestamp, L2_TIMESTAMP_SECONDS.to_string());
        assert_eq!(place_capture.api_key, ARTIFACT_API_KEY_CANARY);
        assert_eq!(place_capture.passphrase, ARTIFACT_PASSPHRASE_CANARY);
        assert_eq!(place_capture.making_amount, Some(intent.maker_amount));
        assert_eq!(place_capture.taking_amount, Some(intent.taker_amount));

        let mut user_frame = UserAuthFrameArtifactCapture::default();
        credentials
            .user_subscription(metadata.condition())
            .expect("artifact user subscription")
            .dispatch(&mut user_frame)
            .expect("capture authenticated user frame");
        let body_json: serde_json::Value =
            serde_json::from_slice(&place_capture.body).expect("parse authenticated place body");
        let eip712_signature = body_json["order"]["signature"]
            .as_str()
            .expect("place body carries an EIP-712 signature")
            .to_owned();

        let authenticated_path = directory.path().join("authenticated-canary.jsonl");
        let (mut journal, initial) =
            PmAuthenticatedMutationJournal::start(authenticated_path.clone(), auth_scope.clone())
                .await
                .expect("start canary authenticated journal");
        assert_eq!(initial.record_count(), 0);
        let coordinator = crate::authenticated_journal::PmAuthenticatedCoordinatorIdentityV1::new(
            intent.client_order,
            intent.instrument,
        );
        let prepared = PmAuthenticatedPlacePreparedV1::new(
            &auth_scope,
            coordinator,
            1,
            semantic_commitment.bytes(),
            expected_order_id.bytes(),
            L2_TIMESTAMP_SECONDS,
        )
        .expect("semantic-only authenticated Prepared");
        let acknowledged = acknowledge_prepared(
            journal
                .try_record(PmAuthenticatedJournalRecordV1::PlacePrepared(prepared))
                .expect("record canary Prepared"),
        )
        .await;
        let grant = acknowledge_send_grant(
            journal
                .try_authorize_dispatch(acknowledged)
                .expect("record canary send grant"),
        )
        .await;
        let PmAuthenticatedSendGrant::Place(grant) = grant else {
            panic!("place Prepared produced cancel grant")
        };
        assert!(grant.matches_retained_request(
            semantic_commitment.bytes(),
            expected_order_id.bytes(),
            L2_TIMESTAMP_SECONDS,
        ));
        let result = PmAuthenticatedPlaceResultV1::accepted(
            coordinator,
            grant.grant_sequence(),
            expected_order_id.bytes(),
        );
        drop(
            acknowledge_place_result(
                journal
                    .try_record_place_result(grant, result)
                    .expect("record canary accepted result"),
            )
            .await,
        );
        journal.shutdown().await.expect("shutdown canary writer");

        let (journal, authenticated_recovery) =
            PmAuthenticatedMutationJournal::start(authenticated_path.clone(), auth_scope.clone())
                .await
                .expect("recover canary authenticated journal");
        journal
            .shutdown()
            .await
            .expect("shutdown canary recovery writer");
        assert_eq!(authenticated_recovery.classified_results().len(), 1);
        assert!(authenticated_recovery.prepared_without_grant().is_empty());
        assert!(authenticated_recovery.unresolved_operations().is_empty());

        let bridge = exact_bridge(&authenticated_recovery);
        let PmJournalAuthenticatedResultV1::Place(place_bridge) = bridge else {
            panic!("place fixture produced cancel bridge")
        };
        let bridge_commitment = place_bridge.request_commitment();
        let goal_f_path = directory.path().join("goal-f-canary.jsonl");
        let goal_f = write_goal_f(
            &goal_f_path,
            &goal_scope,
            [
                PmJournalRecordV1::QuoteIntent(intent),
                PmJournalRecordV1::AuthenticatedResult(bridge),
            ],
        );
        let gate = PmAuthenticatedRecoveryGate::validate(&goal_f, &authenticated_recovery)
            .expect("canary exact bridge pairing");
        assert!(gate.missing_bridges().is_empty());
        assert!(!gate.requires_reconciliation());
        assert_eq!(goal_f.authenticated_results().len(), 1);

        let authenticated_artifact =
            std::fs::read(&authenticated_path).expect("read authenticated canary journal");
        let goal_f_artifact = std::fs::read(&goal_f_path).expect("read Goal-F canary journal");
        let public_capture_artifact =
            std::fs::read(&public_capture_path).expect("read public canary capture");
        assert!(contains_bytes(
            &authenticated_artifact,
            lower_hex(&semantic_commitment.bytes()).as_bytes(),
        ));
        assert!(contains_bytes(
            &goal_f_artifact,
            lower_hex(&bridge_commitment).as_bytes(),
        ));
        let artifacts = [
            ("authenticated journal", authenticated_artifact.as_slice()),
            ("Goal-F journal", goal_f_artifact.as_slice()),
            ("public capture", public_capture_artifact.as_slice()),
        ];

        let private_key_raw = decode_prefixed_hex(ARTIFACT_PRIVATE_KEY_CANARY);
        let hmac_secret_raw = URL_SAFE
            .decode(ARTIFACT_HMAC_SECRET_CANARY)
            .expect("decode HMAC canary");
        let l2_signature_raw = URL_SAFE
            .decode(place_capture.l2_signature.as_bytes())
            .expect("decode L2 signature");
        let eip712_signature_raw = decode_prefixed_hex(&eip712_signature);
        let secret_l2_headers = format!(
            "POLY_SIGNATURE: {}\nPOLY_API_KEY: {}\nPOLY_PASSPHRASE: {}",
            place_capture.l2_signature, place_capture.api_key, place_capture.passphrase,
        );
        let runtime_only_bytes = runtime_commitment.runtime_only_bytes();
        assert_ne!(runtime_only_bytes, semantic_commitment.bytes());

        assert_canary_absent(
            &artifacts,
            "private key",
            &[
                ARTIFACT_PRIVATE_KEY_CANARY.as_bytes(),
                private_key_raw.as_slice(),
            ],
        );
        assert_canary_absent(&artifacts, "API key", &[ARTIFACT_API_KEY_CANARY.as_bytes()]);
        assert_canary_absent(
            &artifacts,
            "HMAC secret",
            &[
                ARTIFACT_HMAC_SECRET_CANARY.as_bytes(),
                hmac_secret_raw.as_slice(),
            ],
        );
        assert_canary_absent(
            &artifacts,
            "passphrase",
            &[ARTIFACT_PASSPHRASE_CANARY.as_bytes()],
        );
        assert_canary_absent(
            &artifacts,
            "secret-bearing L2 headers",
            &[secret_l2_headers.as_bytes()],
        );
        assert_canary_absent(
            &artifacts,
            "L2 signature",
            &[
                place_capture.l2_signature.as_bytes(),
                l2_signature_raw.as_slice(),
            ],
        );
        assert_canary_absent(
            &artifacts,
            "authenticated place body",
            &[place_capture.body.as_slice()],
        );
        assert_canary_absent(
            &artifacts,
            "EIP-712 order signature",
            &[eip712_signature.as_bytes(), eip712_signature_raw.as_slice()],
        );
        assert_canary_absent(
            &artifacts,
            "authenticated user frame",
            &[user_frame.0.as_slice()],
        );
        assert_canary_absent(
            &artifacts,
            "runtime exact-body commitment",
            &[runtime_only_bytes.as_slice()],
        );
        assert_runtime_body_bytes_are_test_only();
    });
}

#[tokio::test]
async fn configuration_fingerprint_mismatch_fails_before_attempt_pairing() {
    let directory = tempfile::tempdir().expect("temporary directory");
    let (base, goal_scope, _) = scopes();
    let alternate = alternate_reference_config(&base);
    let alternate_auth = PmAuthenticatedJournalScopeV1::from_config(&alternate, [0x44; 32])
        .expect("alternate auth scope");
    assert_eq!(goal_scope.account_scope(), alternate_auth.account_scope());
    assert_eq!(goal_scope.instrument(), alternate_auth.instrument());
    assert_ne!(
        goal_scope.configuration_fingerprint().bytes(),
        alternate_auth.configuration_fingerprint()
    );
    let goal_f = write_goal_f(
        &directory.path().join("goal-f-config.jsonl"),
        &goal_scope,
        [],
    );
    let intent = quote(&goal_scope);
    let authenticated = authenticated_recovery(
        &directory.path().join("auth-config.jsonl"),
        &alternate_auth,
        intent,
        AuthTail::Empty,
    )
    .await;

    assert!(matches!(
        PmAuthenticatedRecoveryGate::validate(&goal_f, &authenticated),
        Err(PmAuthenticatedRecoveryError::ScopeMismatch)
    ));
}

#[tokio::test]
async fn duplicate_authenticated_attempts_cannot_reuse_one_goal_f_prior() {
    let directory = tempfile::tempdir().expect("temporary directory");
    let (_, goal_scope, auth_scope) = scopes();
    let intent = quote(&goal_scope);
    let path = directory.path().join("auth-duplicate-prior.jsonl");
    let (mut journal, initial) =
        PmAuthenticatedMutationJournal::start(path.clone(), auth_scope.clone())
            .await
            .expect("start authenticated journal");
    assert_eq!(initial.record_count(), 0);
    let value = prepared(&auth_scope, intent);
    drop(
        acknowledge_prepared(
            journal
                .try_record(PmAuthenticatedJournalRecordV1::PlacePrepared(value))
                .expect("record first Prepared"),
        )
        .await,
    );
    drop(
        acknowledge_prepared(
            journal
                .try_record(PmAuthenticatedJournalRecordV1::PlacePrepared(value))
                .expect("append duplicate Prepared for restart evidence"),
        )
        .await,
    );
    journal.shutdown().await.expect("shutdown writer");

    let error = match PmAuthenticatedMutationJournal::start(path, auth_scope).await {
        Ok((journal, _)) => {
            journal
                .shutdown()
                .await
                .expect("shutdown unexpected writer");
            panic!("duplicate Prepared unexpectedly recovered")
        }
        Err(error) => error,
    };
    assert!(matches!(
        error,
        PmAuthenticatedJournalError::Recovery(
            PmAuthenticatedJournalRecoveryError::DuplicatePreparedRequest
        )
    ));
}

#[test]
fn mismatched_goal_f_bridge_is_terminally_rejected() {
    run_large_recovery_test("pm-auth-mismatched-bridge", || async {
        let directory = tempfile::tempdir().expect("temporary directory");
        let (_, goal_scope, auth_scope) = scopes();
        let intent = quote(&goal_scope);
        let authenticated = authenticated_recovery(
            &directory.path().join("auth-mismatched-bridge.jsonl"),
            &auth_scope,
            intent,
            AuthTail::Accepted,
        )
        .await;
        let goal_f = write_goal_f(
            &directory.path().join("goal-f-mismatched-bridge.jsonl"),
            &goal_scope,
            [
                PmJournalRecordV1::QuoteIntent(intent),
                PmJournalRecordV1::AuthenticatedResult(changed_commitment_bridge(exact_bridge(
                    &authenticated,
                ))),
            ],
        );

        assert!(matches!(
            PmAuthenticatedRecoveryGate::validate(&goal_f, &authenticated),
            Err(PmAuthenticatedRecoveryError::BridgeMismatch)
        ));
    });
}

#[test]
fn goal_f_bridge_without_authenticated_result_is_terminally_rejected() {
    run_large_recovery_test("pm-auth-extra-bridge", || async {
        let directory = tempfile::tempdir().expect("temporary directory");
        let (_, goal_scope, auth_scope) = scopes();
        let intent = quote(&goal_scope);
        let result_recovery = authenticated_recovery(
            &directory.path().join("auth-extra-bridge-source.jsonl"),
            &auth_scope,
            intent,
            AuthTail::Accepted,
        )
        .await;
        let goal_f = write_goal_f(
            &directory.path().join("goal-f-extra-bridge.jsonl"),
            &goal_scope,
            [
                PmJournalRecordV1::QuoteIntent(intent),
                PmJournalRecordV1::AuthenticatedResult(exact_bridge(&result_recovery)),
            ],
        );
        let empty = authenticated_recovery(
            &directory.path().join("auth-empty.jsonl"),
            &auth_scope,
            intent,
            AuthTail::Empty,
        )
        .await;

        assert!(matches!(
            PmAuthenticatedRecoveryGate::validate(&goal_f, &empty),
            Err(PmAuthenticatedRecoveryError::ExtraGoalFBridge)
        ));
    });
}

#[test]
fn exact_bridge_remains_valid_after_goal_f_terminal_compaction() {
    run_large_recovery_test("pm-auth-compaction-recovery", || async {
        let directory = tempfile::tempdir().expect("temporary directory");
        let (_, goal_scope, auth_scope) = scopes();
        let intent = quote(&goal_scope);
        let authenticated = authenticated_recovery(
            &directory.path().join("auth-compacted.jsonl"),
            &auth_scope,
            intent,
            AuthTail::Accepted,
        )
        .await;

        assert_exact_bridge_after_terminal_compaction(
            directory.path(),
            &goal_scope,
            intent,
            &authenticated,
        );
    });
}

// Keep the large Goal-F record enum values out of the async test future. The
// named 8 MiB test thread isolates this known bounded schema value without a
// process-wide RUST_MIN_STACK override; recovery still streams one record at
// a time from the journal.
#[allow(
    clippy::vec_init_then_push,
    reason = "one-at-a-time heap construction avoids one stack array of four large journal records"
)]
fn assert_exact_bridge_after_terminal_compaction(
    directory: &Path,
    goal_scope: &PmJournalScopeV1,
    intent: PmJournalQuoteIntentV1,
    authenticated: &PmAuthenticatedJournalRecovery,
) {
    let PmJournalAuthenticatedResultV1::Place(bridge) = exact_bridge(authenticated) else {
        panic!("place fixture produced cancel bridge")
    };
    let venue_order = bridge
        .canonical()
        .venue_order
        .expect("accepted bridge binds exact venue identity");
    let terminal = PmJournalOrderTerminalV1 {
        client_order: intent.client_order,
        venue_order,
        status: PmJournalTerminalStatusV1::Cancelled,
        cumulative: U256::ZERO,
        remaining: intent.quantity.protocol_units(),
        source: PmJournalOrderProgressSourceV1::PrivateWebsocket,
        occurrence: PmJournalFillOccurrenceV1 {
            owner_sequence: IngressSequence::new(1),
            connection: Some(PmConnectionId::new("pm-private").expect("connection")),
            connection_epoch: Some(ConnectionEpoch::new(1)),
            ingress_sequence: Some(IngressSequence::new(1)),
            snapshot_revision: None,
            monotonic_service_ns: 10,
        },
    };
    let watermark = PmJournalFillWatermarkV1 {
        cursor: PmJournalFillCursorV1 {
            account_scope: goal_scope.account_scope(),
            opaque: PmJournalFingerprintV1::from_bytes([0x99; 32]),
        },
    };
    // Build one large record at a time on the heap. A fixed array of four
    // journal records makes the async test future needlessly exceed the
    // default test-thread stack; it is not representative of journal replay.
    let mut records = Vec::with_capacity(4);
    records.push(PmJournalRecordV1::QuoteIntent(intent));
    records.push(PmJournalRecordV1::AuthenticatedResult(
        PmJournalAuthenticatedResultV1::Place(bridge),
    ));
    records.push(PmJournalRecordV1::OrderTerminal(terminal));
    records.push(PmJournalRecordV1::FillWatermarkAdvanced(watermark));
    let goal_f = write_goal_f(
        &directory.join("goal-f-compacted.jsonl"),
        goal_scope,
        records,
    );
    assert!(goal_f.recovered_order(intent.client_order).is_none());
    assert_eq!(goal_f.authenticated_results().len(), 1);

    let gate = PmAuthenticatedRecoveryGate::validate(&goal_f, authenticated)
        .expect("compacted exact bridge remains valid");
    assert!(gate.missing_bridges().is_empty());
    assert!(!gate.requires_reconciliation());
}

#[tokio::test]
async fn ordinary_ambiguous_result_cannot_be_reinterpreted_as_authenticated_attempt() {
    let directory = tempfile::tempdir().expect("temporary directory");
    let (_, goal_scope, auth_scope) = scopes();
    let intent = quote(&goal_scope);
    let ordinary = PmJournalPlaceResultV1 {
        client_order: intent.client_order,
        outcome: PmJournalPlaceOutcomeV1::AmbiguousTimeout,
        reject_reason: None,
        venue_order: None,
        immediate_fills: PmJournalImmediateFillsV1::empty(),
    };
    let goal_f = write_goal_f(
        &directory.path().join("goal-f-ordinary.jsonl"),
        &goal_scope,
        [
            PmJournalRecordV1::QuoteIntent(intent),
            PmJournalRecordV1::PlaceResult(ordinary),
        ],
    );
    let authenticated = authenticated_recovery(
        &directory.path().join("auth-after-ordinary.jsonl"),
        &auth_scope,
        intent,
        AuthTail::GrantTail,
    )
    .await;

    assert!(matches!(
        PmAuthenticatedRecoveryGate::validate(&goal_f, &authenticated),
        Err(PmAuthenticatedRecoveryError::PriorAlreadyCompletedByOrdinaryResult)
    ));
}
