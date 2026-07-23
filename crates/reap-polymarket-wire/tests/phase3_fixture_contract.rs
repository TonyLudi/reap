mod support;

use std::collections::BTreeSet;

use reap_polymarket_wire::{
    PmWireError, PmWsEvent, parse_clob_metadata, parse_lifecycle_metadata, parse_ws_frame,
};
use serde::Deserialize;

#[test]
fn pinned_public_market_data_fixtures_drive_the_production_parser() {
    let initial = parse_ws_frame(
        include_bytes!("../fixtures/phase3_initial_snapshot.json"),
        support::book_config(),
    )
    .unwrap();
    let [PmWsEvent::BookSnapshot(snapshot)] = initial.events() else {
        panic!("one initial snapshot event");
    };
    assert_eq!(snapshot.token(), support::scope().token());
    assert_eq!(
        snapshot.verified_hash().to_string(),
        "6ac95ffad569774202496c914c0753fc43279c4c"
    );

    let incremental = parse_ws_frame(
        include_bytes!("../fixtures/phase3_valid_incremental.json"),
        support::book_config(),
    )
    .unwrap();
    let [PmWsEvent::PriceChanges(batch)] = incremental.events() else {
        panic!("one incremental batch");
    };
    assert_eq!(batch.changes().len(), 2);
    assert_eq!(batch.final_best_prices().bid().to_string(), "0.5");
    assert_eq!(batch.final_best_prices().ask().to_string(), "0.6");

    let top = parse_ws_frame(
        include_bytes!("../fixtures/phase3_valid_bbo.json"),
        support::book_config(),
    )
    .unwrap();
    let [PmWsEvent::BestBidAsk(top)] = top.events() else {
        panic!("one best-bid/ask event");
    };
    assert_eq!(top.prices().bid().to_string(), "0.5");
    assert_eq!(top.prices().ask().to_string(), "0.6");
}

#[test]
fn pinned_public_negative_fixtures_fail_at_the_exact_wire_boundary() {
    for (raw, expected) in [
        (
            include_bytes!("../fixtures/phase3_malformed_ws.json").as_slice(),
            PmWireError::MalformedJson,
        ),
        (
            include_bytes!("../fixtures/phase3_wrong_token_incremental.json").as_slice(),
            PmWireError::TokenMismatch,
        ),
        (
            include_bytes!("../fixtures/phase3_crossed_snapshot.json").as_slice(),
            PmWireError::CrossedBook,
        ),
        (
            include_bytes!("../fixtures/phase3_empty_snapshot.json").as_slice(),
            PmWireError::EmptyBook,
        ),
        (
            include_bytes!("../fixtures/phase3_price_zero_snapshot.json").as_slice(),
            PmWireError::InvalidNumeric("price"),
        ),
        (
            include_bytes!("../fixtures/phase3_price_one_snapshot.json").as_slice(),
            PmWireError::InvalidNumeric("price"),
        ),
        (
            include_bytes!("../fixtures/phase3_price_negative_snapshot.json").as_slice(),
            PmWireError::InvalidNumeric("price"),
        ),
        (
            include_bytes!("../fixtures/phase3_price_above_one_snapshot.json").as_slice(),
            PmWireError::InvalidNumeric("price"),
        ),
    ] {
        assert_eq!(
            parse_ws_frame(raw, support::book_config()),
            Err(expected),
            "{expected:?}"
        );
    }
}

#[test]
fn pinned_lifecycle_and_clob_drift_remains_exact_evidence_for_authority_join() {
    let lifecycle_cases = [
        (
            include_bytes!("../fixtures/phase3_lifecycle_inactive.json").as_slice(),
            (false, false, false, true, true),
        ),
        (
            include_bytes!("../fixtures/phase3_lifecycle_closed.json").as_slice(),
            (true, true, false, true, true),
        ),
        (
            include_bytes!("../fixtures/phase3_lifecycle_archived.json").as_slice(),
            (true, false, true, true, true),
        ),
        (
            include_bytes!("../fixtures/phase3_lifecycle_not_accepting.json").as_slice(),
            (true, false, false, false, true),
        ),
        (
            include_bytes!("../fixtures/phase3_lifecycle_book_disabled.json").as_slice(),
            (true, false, false, true, false),
        ),
    ];
    for (raw, expected) in lifecycle_cases {
        let observed = parse_lifecycle_metadata(raw, support::scope())
            .unwrap()
            .lifecycle();
        assert_eq!(
            (
                observed.active(),
                observed.closed(),
                observed.archived(),
                observed.accepting_orders(),
                observed.order_book_enabled(),
            ),
            expected
        );
    }

    let grid = parse_clob_metadata(
        include_bytes!("../fixtures/phase3_clob_grid_drift.json"),
        support::scope(),
    )
    .unwrap();
    assert_eq!(grid.tick().to_string(), "0.001");

    let minimum = parse_clob_metadata(
        include_bytes!("../fixtures/phase3_clob_minimum_drift.json"),
        support::scope(),
    )
    .unwrap();
    assert_eq!(minimum.minimum_order_size().to_string(), "6");

    let negative_risk = parse_clob_metadata(
        include_bytes!("../fixtures/phase3_clob_negative_risk_drift.json"),
        support::scope(),
    )
    .unwrap();
    assert!(negative_risk.negative_risk());

    assert_eq!(
        parse_clob_metadata(
            include_bytes!("../fixtures/phase3_clob_wrong_membership.json"),
            support::scope(),
        ),
        Err(PmWireError::ConfiguredTokenMissing)
    );
}

#[test]
fn unknown_lifecycle_clob_token_and_authority_fields_are_denied() {
    assert_eq!(
        parse_lifecycle_metadata(
            include_bytes!("../fixtures/phase3_lifecycle_unknown_field.json"),
            support::scope(),
        ),
        Err(PmWireError::MalformedJson)
    );

    for raw in [
        include_bytes!("../fixtures/phase3_token_unknown_field.json").as_slice(),
        include_bytes!("../fixtures/phase3_domain_authority_injection.json").as_slice(),
        include_bytes!("../fixtures/phase3_spender_authority_injection.json").as_slice(),
    ] {
        assert_eq!(
            parse_clob_metadata(raw, support::scope()),
            Err(PmWireError::MalformedJson)
        );
    }
}

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
fn logical_sequence_fixture_distinguishes_local_order_from_external_gap_evidence() {
    let corpus: LogicalCorpus = serde_json::from_str(include_str!(
        "../fixtures/phase3_logical_sequence_cases.json"
    ))
    .unwrap();
    assert_eq!(corpus.schema, 1);
    assert!(
        corpus
            .provenance
            .contains("not claimed venue wire captures")
    );

    let expected_names = BTreeSet::from([
        "duplicate",
        "gap",
        "reconnect",
        "reordered",
        "resync",
        "stale",
    ]);
    let names = corpus
        .cases
        .iter()
        .map(|case| case.name.as_str())
        .collect::<BTreeSet<_>>();
    assert_eq!(names, expected_names);

    for case in &corpus.cases {
        assert!(case.freshness_limit_ns > 0);
        assert!(!case.expected.is_empty());
        assert!(
            case.observations
                .windows(2)
                .all(|pair| pair[0].monotonic_ns < pair[1].monotonic_ns)
        );
        assert!(
            case.observations
                .iter()
                .all(|observation| observation.epoch > 0
                    && observation.snapshot_revision > 0
                    && (observation.ingress > 0 || observation.kind == "begin_epoch"))
        );
    }

    let stale = case(&corpus, "stale");
    assert!(
        stale.observations.last().unwrap().monotonic_ns
            - stale.observations.first().unwrap().monotonic_ns
            > stale.freshness_limit_ns
    );

    let duplicate = case(&corpus, "duplicate");
    assert_eq!(
        duplicate.observations[1].ingress,
        duplicate.observations[2].ingress
    );

    let reordered = case(&corpus, "reordered");
    assert!(reordered.observations[1].ingress > reordered.observations[2].ingress);

    let gap = case(&corpus, "gap");
    assert!(
        gap.observations
            .iter()
            .any(|observation| observation.kind == "external_gap_fault")
    );
    assert!(
        gap.observations
            .windows(2)
            .all(|pair| pair[1].ingress <= pair[0].ingress + 1),
        "a local ingress jump is not represented as exchange gap evidence"
    );

    let reconnect = case(&corpus, "reconnect");
    assert_eq!(reconnect.observations[1].kind, "disconnect");
    assert_eq!(reconnect.observations[2].kind, "begin_epoch");
    assert_eq!(
        reconnect.observations[2].epoch,
        reconnect.observations[1].epoch + 1
    );

    let resync = case(&corpus, "resync");
    assert_eq!(resync.observations.last().unwrap().kind, "snapshot");
    assert!(
        resync.observations.last().unwrap().snapshot_revision
            > resync.observations.first().unwrap().snapshot_revision
    );
}

fn case<'a>(corpus: &'a LogicalCorpus, name: &str) -> &'a LogicalCase {
    corpus.cases.iter().find(|case| case.name == name).unwrap()
}
