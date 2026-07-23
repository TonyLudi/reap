#[allow(dead_code)]
mod support;

use reap_pm_core::{
    ConnectionEpoch, IngressSequence, PmBookUpdate, SnapshotRevision, VenueEventHash,
};
use reap_pm_state::{
    PmBookBatchEvidence, PmBookReducer, PmBookTransition, PmDomainFingerprint, PmExternalBookFault,
    PmMetadataContract, PmMetadataFingerprint, PmMetadataObservation, PmPublicReadinessReason,
};
use serde::Deserialize;

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
struct LogicalCorpus {
    schema: u64,
    provenance: String,
    cases: Vec<LogicalCase>,
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
struct LogicalCase {
    name: String,
    observations: Vec<LogicalObservation>,
    freshness_limit_ns: u64,
    expected: String,
}

#[derive(Debug, Clone, Deserialize)]
#[serde(deny_unknown_fields)]
struct LogicalObservation {
    kind: String,
    epoch: u64,
    ingress: u64,
    snapshot_revision: u64,
    monotonic_ns: u64,
}

#[test]
fn pinned_logical_cases_drive_the_production_session_and_book_reducer() {
    let corpus: LogicalCorpus = serde_json::from_str(include_str!(
        "../../reap-polymarket-wire/fixtures/phase3_logical_sequence_cases.json"
    ))
    .unwrap();
    assert_eq!(corpus.schema, 1);
    assert!(
        corpus
            .provenance
            .contains("not claimed venue wire captures")
    );

    let (snapshot, snapshot_hash, delta) = normalized_fixture_updates();
    for case in &corpus.cases {
        let mut reducer = ready_for_first_snapshot(case.freshness_limit_ns);
        let mut last_result = Ok(PmBookTransition::EpochStarted {
            epoch: ConnectionEpoch::new(11),
        });

        for observation in &case.observations {
            let epoch = ConnectionEpoch::new(observation.epoch + 10);
            last_result = match observation.kind.as_str() {
                "begin_epoch" => reducer.begin_epoch(epoch),
                "snapshot" => {
                    reducer.apply_update(evidence(observation, Some(snapshot_hash)), &snapshot)
                }
                "delta" => reducer.apply_update(evidence(observation, None), &delta),
                "freshness_timer" => reducer.check_freshness(observation.monotonic_ns),
                "external_gap_fault" => {
                    reducer.apply_external_fault(epoch, PmExternalBookFault::Gap)
                }
                "disconnect" => {
                    reducer.apply_external_fault(epoch, PmExternalBookFault::Disconnect)
                }
                other => panic!("unknown pinned logical observation {other}"),
            };
        }

        match case.expected.as_str() {
            "ready_after_explicit_snapshot" => {
                assert!(last_result.is_ok(), "{}", case.name);
                assert!(reducer.readiness().is_ready(), "{}", case.name);
            }
            "book_stale_until_explicit_snapshot" => assert_unavailable(
                &case.name,
                last_result,
                reducer.readiness().reason(),
                PmPublicReadinessReason::BookStale,
            ),
            "duplicate_ingress_until_explicit_snapshot" => assert_unavailable(
                &case.name,
                last_result,
                reducer.readiness().reason(),
                PmPublicReadinessReason::DuplicateIngress,
            ),
            "reordered_ingress_until_explicit_snapshot" => assert_unavailable(
                &case.name,
                last_result,
                reducer.readiness().reason(),
                PmPublicReadinessReason::ReorderedIngress,
            ),
            "gap_until_explicit_snapshot" => assert_unavailable(
                &case.name,
                last_result,
                reducer.readiness().reason(),
                PmPublicReadinessReason::Gap,
            ),
            "awaiting_snapshot_in_new_epoch" => {
                assert!(last_result.is_ok(), "{}", case.name);
                assert_eq!(
                    reducer.readiness().reason(),
                    Some(PmPublicReadinessReason::SnapshotMissing),
                    "{}",
                    case.name
                );
            }
            other => panic!("unknown pinned logical expectation {other}"),
        }
    }
}

fn normalized_fixture_updates() -> (PmBookUpdate, VenueEventHash, PmBookUpdate) {
    let mut session = support::capture_session();
    session.mark_subscription_sent(60).unwrap();
    let snapshot_batch = session
        .classify(
            include_bytes!("../../reap-polymarket-wire/fixtures/phase3_initial_snapshot.json"),
            1_700_000_000_000_000_100,
            100,
        )
        .unwrap();
    let snapshot_delivery = &snapshot_batch.events()[0];
    let snapshot_hash = snapshot_delivery
        .ordering()
        .venue_hash()
        .expect("pinned production snapshot carries SHA-1 evidence");
    let snapshot = snapshot_delivery.payload().update().clone();
    session
        .open_protocol_flow_after_snapshot(snapshot_batch.snapshot_flow_token().unwrap())
        .unwrap();

    let delta_batch = session
        .classify(
            include_bytes!("../../reap-polymarket-wire/fixtures/phase3_valid_incremental.json"),
            1_700_000_000_000_000_200,
            200,
        )
        .unwrap();
    let delta = delta_batch.events()[0].payload().update().clone();
    assert!(matches!(snapshot, PmBookUpdate::Snapshot(_)));
    assert!(matches!(delta, PmBookUpdate::DeltaBatch(_)));
    (snapshot, snapshot_hash, delta)
}

fn ready_for_first_snapshot(book_freshness_ns: u64) -> PmBookReducer {
    let authority = support::authoritative();
    let fingerprint = PmMetadataFingerprint::new(authority.metadata_fingerprint()).unwrap();
    let domain = PmDomainFingerprint::new(authority.domain_fingerprint()).unwrap();
    let contract = PmMetadataContract::goal_f_clob_v2(support::market_metadata(), domain);
    let mut reducer = PmBookReducer::new(
        support::instrument(),
        fingerprint,
        contract,
        reap_pm_state::PmBookFreshness::new(10_000, book_freshness_ns).unwrap(),
    )
    .unwrap();
    assert!(matches!(
        reducer
            .apply_metadata(
                PmMetadataObservation::new(
                    support::instrument(),
                    SnapshotRevision::new(7),
                    fingerprint,
                    contract,
                    50,
                )
                .unwrap(),
            )
            .unwrap(),
        PmBookTransition::MetadataAccepted { .. }
    ));
    reducer.begin_epoch(ConnectionEpoch::new(11)).unwrap();
    reducer
}

fn evidence(
    observation: &LogicalObservation,
    venue_hash: Option<VenueEventHash>,
) -> PmBookBatchEvidence {
    PmBookBatchEvidence::new(
        support::instrument(),
        ConnectionEpoch::new(observation.epoch + 10),
        SnapshotRevision::new(7),
        SnapshotRevision::new(observation.snapshot_revision),
        IngressSequence::new(observation.ingress),
        observation.monotonic_ns,
        venue_hash,
    )
    .unwrap()
}

fn assert_unavailable(
    case: &str,
    result: Result<PmBookTransition, PmPublicReadinessReason>,
    readiness: Option<PmPublicReadinessReason>,
    expected: PmPublicReadinessReason,
) {
    assert_eq!(result, Err(expected), "{case}");
    assert_eq!(readiness, Some(expected), "{case}");
}
