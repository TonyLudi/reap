mod support;

use std::path::{Path, PathBuf};

use reap_pm_live::{
    PM_PUBLIC_CAPTURE_SCHEMA_VERSION, PmCaptureBookMarketBinding, PmCaptureHeader, PmCaptureScope,
    PmCaptureVerifyError, PmPublicCapture, PmPublicCaptureRecord, PmPublicCaptureRun,
    PmPublicCaptureRunError, PmPublicCaptureTerminalCause, PmPublicLaneService,
    PmPublicRawTransport, ServicedLaneItem, replay_pm_public_capture, verify_pm_public_capture,
};
use reap_polymarket_adapter::{PmAuthoritativeMetadata, PmMetadataRevisionInput};

use support::{
    authoritative, capture_header, instrument, market_metadata, pm_source, provenance,
    public_config, session_policy, snapshot_one,
};

const WALL_RECEIVE_NS: u64 = 1_700_000_000_000_000_100;

struct ConsumePublic;

impl PmPublicLaneService for ConsumePublic {
    fn on_pm_public_unavailable(
        &mut self,
        _item: ServicedLaneItem<reap_pm_live::PmPublicUnavailable>,
    ) {
    }

    fn on_okx_public_unavailable(
        &mut self,
        _item: ServicedLaneItem<reap_pm_live::OkxPublicUnavailable>,
    ) {
    }

    fn on_market(&mut self, _item: ServicedLaneItem<reap_pm_core::PmMarketEvent>) {}

    fn on_book(&mut self, _item: ServicedLaneItem<reap_pm_core::PmBookEvent>) {}

    fn on_reference(&mut self, _item: ServicedLaneItem<reap_pm_core::OkxReferenceEvent>) {}
}

fn service_all_public(run: &mut PmPublicCaptureRun, monotonic_ns: u64) {
    let mut consumer = ConsumePublic;
    while run.public_lane_metrics().depth() != 0 {
        assert!(
            run.service_lane_turn(monotonic_ns, &mut consumer).unwrap() > 0,
            "a nonempty public lane must make service progress"
        );
    }
}

async fn start_capture(path: PathBuf) -> PmPublicCaptureRun {
    start_capture_with(path, authoritative()).await
}

async fn start_capture_with(
    path: PathBuf,
    authoritative: PmAuthoritativeMetadata,
) -> PmPublicCaptureRun {
    PmPublicCapture::new(public_config())
        .unwrap()
        .start(path, authoritative, session_policy(), provenance())
        .await
        .unwrap()
}

fn rest_snapshot() -> String {
    snapshot_one().replace(r#""event_type":"book","#, "")
}

fn live_authoritative() -> PmAuthoritativeMetadata {
    let long = format!(
        r#"{{"condition_id":"{}","question_id":"{}","active":true,"closed":false,"archived":false,"accepting_orders":true,"enable_order_book":true,"accepting_order_timestamp":"2026-08-08T00:00:00Z","end_date_iso":"2027-01-01T00:00:00Z","game_start_time":null,"seconds_delay":0,"minimum_order_size":5,"minimum_tick_size":0.01,"tokens":[]}}"#,
        support::CONDITION,
        support::MARKET,
    );
    let short = format!(
        r#"{{"c":"{}","t":[{{"t":"123","o":"Yes"}},{{"t":"456","o":"No"}}],"mts":0.01,"mos":5,"nr":false,"fd":{{"r":0.02,"e":2,"to":true}},"mbf":0,"tbf":0,"ao":true,"sd":0,"gst":null,"cbos":true,"aot":"2026-08-08T00:00:00Z","rfqe":false,"itode":false,"ibce":true,"oas":0}}"#,
        support::CONDITION,
    );
    PmAuthoritativeMetadata::join_live_clob_v2_raw(
        instrument(),
        pm_source(),
        market_metadata(),
        long.as_bytes(),
        short.as_bytes(),
        PmMetadataRevisionInput::new(reap_pm_core::SnapshotRevision::new(7), 50).unwrap(),
    )
    .unwrap()
}

fn live_capture_header() -> PmCaptureHeader {
    let authoritative = live_authoritative();
    let scope = PmCaptureScope::new(&public_config(), &authoritative).unwrap();
    PmCaptureHeader::new(scope, session_policy(), provenance()).unwrap()
}

fn live_rest_snapshot() -> String {
    rest_snapshot()
        .replace(support::MARKET, support::CONDITION)
        .replace(
            "6ac95ffad569774202496c914c0753fc43279c4c",
            "c01a8fba7c82df0da51e13c6f3b5d898eb99e32f",
        )
}

async fn write_rest_capture(path: PathBuf) {
    let mut run = start_capture(path).await;
    run.record_pm_connection_started(60).await.unwrap();
    run.record_pm_subscription_sent(80).await.unwrap();

    let mut batch = run
        .capture_pm_rest_book(WALL_RECEIVE_NS, 100, rest_snapshot().as_bytes())
        .await
        .unwrap();
    let flow = batch.take_snapshot_flow().unwrap();
    let delivery = batch.into_books().into_iter().next().unwrap();
    run.commit_then_enqueue_pm_snapshot(delivery, flow).unwrap();
    service_all_public(&mut run, 100);
    run.finish().await.unwrap();
}

fn raw_record(path: &Path) -> (serde_json::Value, PmPublicCaptureRecord) {
    let line = std::fs::read_to_string(path)
        .unwrap()
        .lines()
        .find(|line| line.contains(r#""record_type":"raw_public_frame""#))
        .unwrap()
        .to_owned();
    (
        serde_json::from_str(&line).unwrap(),
        serde_json::from_str(&line).unwrap(),
    )
}

#[tokio::test]
async fn rest_book_capture_verify_and_replay_are_byte_deterministic() {
    let directory = tempfile::tempdir().unwrap();
    let first_path = directory.path().join("rest-first.jsonl");
    let second_path = directory.path().join("rest-second.jsonl");

    write_rest_capture(first_path.clone()).await;
    write_rest_capture(second_path.clone()).await;

    assert_eq!(
        std::fs::read(&first_path).unwrap(),
        std::fs::read(&second_path).unwrap()
    );
    let (encoded, record) = raw_record(&first_path);
    assert_eq!(encoded["frame"]["transport"], "book_rest");
    let PmPublicCaptureRecord::RawPublicFrame { frame, .. } = record else {
        panic!("selected record must be a PM raw public frame");
    };
    assert_eq!(frame.transport(), PmPublicRawTransport::BookRest);
    assert_eq!(frame.decode_raw().unwrap(), rest_snapshot().as_bytes());

    let header = capture_header();
    let first_verification = verify_pm_public_capture(&first_path, &header).unwrap();
    let second_verification = verify_pm_public_capture(&second_path, &header).unwrap();
    assert_eq!(first_verification, second_verification);
    assert_eq!(
        first_verification.schema_version,
        PM_PUBLIC_CAPTURE_SCHEMA_VERSION
    );
    assert_eq!(PM_PUBLIC_CAPTURE_SCHEMA_VERSION, 1);
    assert_eq!(first_verification.raw_public_frames, 1);

    let first_projection = replay_pm_public_capture(&first_path, &header).unwrap();
    let second_projection = replay_pm_public_capture(&second_path, &header).unwrap();
    assert_eq!(first_projection, second_projection);
    assert_eq!(first_projection.counters().snapshots_committed, 1);
}

#[tokio::test]
async fn malformed_rest_book_is_captured_before_parser_failure() {
    let directory = tempfile::tempdir().unwrap();
    let path = directory.path().join("malformed-rest.jsonl");
    let mut run = start_capture(path.clone()).await;
    run.record_pm_connection_started(60).await.unwrap();
    run.record_pm_subscription_sent(80).await.unwrap();

    assert!(matches!(
        run.capture_pm_rest_book(WALL_RECEIVE_NS, 100, b"{").await,
        Err(PmPublicCaptureRunError::PmClassify { .. })
    ));
    assert!(matches!(
        run.finish().await,
        Err(PmPublicCaptureRunError::TerminalFinish {
            cause: PmPublicCaptureTerminalCause::IngressSessionClassification,
            shutdown_error: None,
        })
    ));

    let (_, record) = raw_record(&path);
    let PmPublicCaptureRecord::RawPublicFrame { frame, .. } = record else {
        panic!("selected record must be a PM raw public frame");
    };
    assert_eq!(frame.transport(), PmPublicRawTransport::BookRest);
    assert_eq!(frame.decode_raw().unwrap(), b"{");
    let verification = verify_pm_public_capture(&path, &capture_header()).unwrap();
    assert_eq!(verification.raw_public_frames, 1);
}

#[tokio::test]
async fn schema_v1_websocket_record_without_transport_still_verifies_and_replays() {
    let directory = tempfile::tempdir().unwrap();
    let path = directory.path().join("schema-v1-websocket.jsonl");
    let mut run = start_capture(path.clone()).await;
    run.record_pm_connection_started(60).await.unwrap();
    run.record_pm_subscription_sent(80).await.unwrap();

    let mut batch = run
        .capture_pm_public(WALL_RECEIVE_NS, 100, snapshot_one().as_bytes())
        .await
        .unwrap();
    let flow = batch.take_snapshot_flow().unwrap();
    let delivery = batch.into_books().into_iter().next().unwrap();
    run.commit_then_enqueue_pm_snapshot(delivery, flow).unwrap();
    service_all_public(&mut run, 100);
    run.finish().await.unwrap();

    let (encoded, record) = raw_record(&path);
    assert!(
        encoded["frame"].get("transport").is_none(),
        "the default discriminator must remain absent from schema-v1 bytes"
    );
    let PmPublicCaptureRecord::RawPublicFrame { frame, .. } = record else {
        panic!("selected record must be a PM raw public frame");
    };
    assert_eq!(frame.transport(), PmPublicRawTransport::MarketWebSocket);

    let header = capture_header();
    let verification = verify_pm_public_capture(&path, &header).unwrap();
    assert_eq!(verification.schema_version, 1);
    assert_eq!(verification.raw_public_frames, 1);
    assert_eq!(
        replay_pm_public_capture(&path, &header)
            .unwrap()
            .counters()
            .snapshots_committed,
        1
    );
}

#[test]
fn legacy_capture_scope_omits_binding_and_roundtrips_byte_identically() {
    let header = capture_header();
    assert_eq!(
        header.scope().book_market_binding(),
        PmCaptureBookMarketBinding::LegacyMarketId
    );
    let before = serde_json::to_vec(header.scope()).unwrap();
    let json: serde_json::Value = serde_json::from_slice(&before).unwrap();
    assert!(json.get("book_market_binding").is_none());

    let decoded: PmCaptureScope = serde_json::from_slice(&before).unwrap();
    assert_eq!(decoded, *header.scope());
    assert_eq!(serde_json::to_vec(&decoded).unwrap(), before);
    assert!(
        !decoded
            .recorded_metadata()
            .unwrap()
            .uses_condition_bound_books()
    );
}

#[tokio::test]
async fn condition_bound_capture_verifies_and_replays_with_the_same_binding() {
    let directory = tempfile::tempdir().unwrap();
    let path = directory.path().join("condition-bound.jsonl");
    let expected_header = live_capture_header();
    assert_eq!(
        expected_header.scope().book_market_binding(),
        PmCaptureBookMarketBinding::ConditionId
    );
    assert!(
        expected_header
            .scope()
            .recorded_metadata()
            .unwrap()
            .uses_condition_bound_books()
    );

    let mut run = start_capture_with(path.clone(), live_authoritative()).await;
    assert_eq!(run.header(), &expected_header);
    run.record_pm_connection_started(60).await.unwrap();
    run.record_pm_subscription_sent(80).await.unwrap();
    let mut batch = run
        .capture_pm_rest_book(WALL_RECEIVE_NS, 100, live_rest_snapshot().as_bytes())
        .await
        .unwrap();
    let flow = batch.take_snapshot_flow().unwrap();
    let delivery = batch.into_books().into_iter().next().unwrap();
    run.commit_then_enqueue_pm_snapshot(delivery, flow).unwrap();
    service_all_public(&mut run, 100);
    run.finish().await.unwrap();

    let first_line = std::fs::read_to_string(&path)
        .unwrap()
        .lines()
        .next()
        .unwrap()
        .to_owned();
    let header_json: serde_json::Value = serde_json::from_str(&first_line).unwrap();
    assert_eq!(
        header_json["header"]["scope"]["book_market_binding"],
        "condition_id"
    );
    verify_pm_public_capture(&path, &expected_header).unwrap();
    assert_eq!(
        replay_pm_public_capture(&path, &expected_header)
            .unwrap()
            .counters()
            .snapshots_committed,
        1
    );

    assert!(matches!(
        verify_pm_public_capture(&path, &capture_header()),
        Err(PmCaptureVerifyError::InvalidRecords)
    ));
}

#[test]
fn unknown_capture_binding_discriminator_is_rejected() {
    let encoded = serde_json::to_string(live_capture_header().scope()).unwrap();
    let unknown = encoded.replace(
        r#""book_market_binding":"condition_id""#,
        r#""book_market_binding":"future_identity""#,
    );
    assert!(serde_json::from_str::<PmCaptureScope>(&unknown).is_err());
}
