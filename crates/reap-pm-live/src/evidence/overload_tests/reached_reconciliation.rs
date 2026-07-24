use reap_pm_core::{
    ConnectionEpoch, EventOrdering, IngressSequence, PmAllowanceValue, PmErc1155OperatorApproval,
    PmPositionAvailability, PmSnapshotEvidence, ReceivedEventClock, SnapshotRevision, U256,
};
use reap_polymarket_adapter::{
    PmFixtureAllowanceRow, PmFixtureBalanceRow, PmFixtureCompletionOccurrence,
    PmFixtureInstrumentScope, PmFixturePositionRow,
};

use crate::{PmAccountFixtureInput, PmFixtureQueryOccurrence, PmLaneKind, SaturationAction};

const OWNER_MEMORY_BOUND_BYTES: usize = 64 * 1024 * 1024;

fn completion(sequence: u64, snapshot_revision: u64) -> PmFixtureCompletionOccurrence {
    PmFixtureCompletionOccurrence::new(
        ReceivedEventClock::new(None, 1_700_000_000_000_000_000 + sequence, 1_000 + sequence)
            .expect("fixed receive clock"),
        EventOrdering::new(
            ConnectionEpoch::new(1),
            Some(SnapshotRevision::new(snapshot_revision)),
            None,
            None,
            IngressSequence::new(sequence),
        )
        .expect("fixed ordering"),
    )
}

#[test]
fn complete_snapshot_row_reaches_product_129_times_and_retains_one_retry() {
    super::run_product_test(|| async {
        let directory = tempfile::tempdir().expect("temporary evidence directory");
        let mut run = super::super::start_reached_overload_product(
            directory.path().join("complete-capture.jsonl"),
            directory.path().join("complete-journal.jsonl"),
        )
        .await
        .expect("fixed reached product starts");
        let reserved = run.reserved_capacity_bytes();
        run.connect_private_fixture(completion(1, 1))
            .expect("configured private fixture connection");
        run.service_turn(1_001)
            .expect("private fixture connection reaches its owner");
        while run.pop_effect().is_some() {}

        let config = super::super::fixture::connectivity_config();
        let account = config.account();
        let domain = account.trading_domain();
        let balances = [
            PmFixtureBalanceRow::new(domain.collateral(), U256::from_u64(1_000_000_000)),
            PmFixtureBalanceRow::new(domain.outcome(), U256::from_u64(100_000_000)),
        ];
        let spenders = account.required_spenders();
        let allowances = [
            PmFixtureAllowanceRow::new(
                spenders[0],
                match spenders[0].requirement().asset() {
                    asset if asset == domain.collateral() => {
                        PmAllowanceValue::Erc20(U256::from_u64(1_000_000_000))
                    }
                    _ => PmAllowanceValue::Erc1155Operator(PmErc1155OperatorApproval::from_bool(
                        true,
                    )),
                },
            ),
            PmFixtureAllowanceRow::new(
                spenders[1],
                match spenders[1].requirement().asset() {
                    asset if asset == domain.collateral() => {
                        PmAllowanceValue::Erc20(U256::from_u64(1_000_000_000))
                    }
                    _ => PmAllowanceValue::Erc1155Operator(PmErc1155OperatorApproval::from_bool(
                        true,
                    )),
                },
            ),
        ];
        let instrument = PmFixtureInstrumentScope::from_metadata(
            account.instrument(),
            account.expected_metadata(),
        )
        .expect("fixed instrument scope");
        let positions = [PmFixturePositionRow::new(
            instrument,
            U256::from_u64(100_000_000),
            PmPositionAvailability::Tradable,
        )];

        for attempt in 1..=129_u64 {
            let request_sequence = IngressSequence::new(attempt * 2 - 1);
            let completion_sequence = attempt * 2;
            let snapshot =
                PmSnapshotEvidence::new(SnapshotRevision::new(attempt)).expect("snapshot evidence");
            let occurrence = PmFixtureQueryOccurrence::new(
                ConnectionEpoch::new(1),
                request_sequence,
                snapshot,
                completion(completion_sequence, attempt),
                2_000 + completion_sequence,
            )
            .expect("complete fixture occurrence");
            let result = run.ingest_account_fixture(PmAccountFixtureInput::new(
                occurrence,
                &balances,
                &allowances,
                &positions,
            ));
            if attempt <= 128 {
                result.expect("first 128 complete snapshots");
            } else {
                assert!(result.is_err(), "129th complete snapshot must be retained");
            }
        }

        let metrics = run.scheduler_metrics(10_000).expect("scheduler metrics");
        let lane = metrics
            .lane(PmLaneKind::Reconciliation)
            .expect("complete snapshot lane");
        assert_eq!(lane.queue().depth(), 128);
        assert_eq!(lane.queue().high_water(), 128);
        assert_eq!(lane.queue().rejected_full(), 1);
        assert_eq!(
            metrics
                .fail_closed()
                .transitions(SaturationAction::KeepUnreadyAndRetry),
            1
        );
        assert!(metrics.fail_closed().account_unready());
        assert!(metrics.fail_closed().retry_pending());
        assert!(!metrics.fail_closed().global_stopped());
        assert_eq!(run.halt(), None);
        assert_eq!(run.mutation_halt(), None);
        assert_eq!(run.fake_effect_metrics().serviced(), 0);
        assert_eq!(run.reserved_capacity_bytes(), reserved);
        assert!(reserved <= OWNER_MEMORY_BOUND_BYTES);
        let _ = run.shutdown().await;
    });
}
