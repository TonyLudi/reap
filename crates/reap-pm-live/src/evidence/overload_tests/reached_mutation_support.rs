use std::time::Duration;

use reap_pm_core::{
    ConnectionEpoch, EventOrdering, IngressSequence, PmOrderSide, PmVenueOrderId, PmVenueOrderKey,
    ReceivedEventClock,
};
use reap_polymarket_adapter::{
    PmFakePlaceScript, PmFixtureCompletionOccurrence, PmFixtureFeeEvidence,
};

use crate::{PmProductEffect, PmScheduledActionKind};

use super::super::fixture::Phase6Model;
use super::super::{
    ReachedOverloadProfile, prepare_reached_overload_product, start_reached_overload_product_for,
};

pub(super) const OWNER_MEMORY_BOUND_BYTES: usize = 64 * 1024 * 1024;
const MAX_PRIVATE_FRAME_EVENTS: usize = 64;

pub(super) async fn start(
    case: &str,
    profile: ReachedOverloadProfile,
) -> (tempfile::TempDir, crate::PmProductRun<Phase6Model>) {
    let directory = tempfile::tempdir().expect("temporary evidence directory");
    let mut run = start_reached_overload_product_for(
        profile,
        directory.path().join(format!("{case}-capture.jsonl")),
        directory.path().join(format!("{case}-journal.jsonl")),
    )
    .await
    .expect("fixed reached product starts");
    prepare_reached_overload_product(&mut run)
        .await
        .expect("fixed product reaches quote-ready state");
    drain_outputs(&mut run);
    (directory, run)
}

pub(super) async fn establish_live_quote(
    run: &mut crate::PmProductRun<Phase6Model>,
    venue_id: &str,
) -> PmVenueOrderKey {
    run.schedule(
        PmOrderSide::Buy,
        PmScheduledActionKind::QuoteEvaluation,
        1_000,
        900,
        1_700_000_000_000,
    )
    .expect("fixed quote evaluation schedules");
    run.service_turn(1_000)
        .expect("scheduled quote reaches the mutation owner");
    drain_outputs(run);
    assert_eq!(run.mutation_counters().quote_intents(), 1);

    wait_for_ack_admission(run, 10, 1_001)
        .await
        .expect("quote intent becomes durable");
    run.service_turn(1_002)
        .expect("durable quote becomes prepared");
    drain_outputs(run);
    assert_eq!(run.mutation_counters().prepared_quotes(), 1);

    let venue = PmVenueOrderKey::new(
        super::super::fixture::account_scope().handle(),
        PmVenueOrderId::new(venue_id).expect("fixed venue order"),
    );
    run.execute_prepared_quote_fixture(
        completion(11, 1_003),
        PmFakePlaceScript::acknowledged(venue, Box::new([])).expect("valid fake ack"),
        1_003,
    )
    .expect("prepared quote executes through fixture");
    run.service_turn(1_004)
        .expect("fake place result reaches the mutation owner");
    drain_outputs(run);

    wait_for_ack_admission(run, 12, 1_005)
        .await
        .expect("place fact becomes durable");
    run.service_turn(1_006)
        .expect("place acknowledgement reaches the owner");
    drain_outputs(run);
    assert_eq!(run.persistence_metrics().depth(), 0);
    assert_eq!(run.mutation_counters().place_results(), 1);
    venue
}

pub(super) fn ingest_fill_batch(
    run: &mut crate::PmProductRun<Phase6Model>,
    venue: PmVenueOrderKey,
    count: usize,
    sequence: u64,
    service_ns: u64,
) {
    let batch_count = count.div_ceil(MAX_PRIVATE_FRAME_EVENTS);
    for batch_index in 0..batch_count {
        let first_attempt = batch_index * MAX_PRIVATE_FRAME_EVENTS + 1;
        let batch_length = MAX_PRIVATE_FRAME_EVENTS.min(count.saturating_sub(first_attempt - 1));
        let batch = fill_batch_from(venue, first_attempt, batch_length);
        let batch_index = u64::try_from(batch_index).expect("bounded batch index");
        let batch_count = u64::try_from(batch_count).expect("bounded batch count");
        run.ingest_private_fixture(
            completion(
                sequence + batch_index,
                service_ns - batch_count + batch_index,
            ),
            batch.as_bytes(),
            PmFixtureFeeEvidence::Unknown,
        )
        .expect("bounded owned fill batch reaches the private lane");
    }
    for _ in 0..batch_count {
        run.service_turn(service_ns)
            .expect("bounded owned fill frame reaches the canonical mutation owner");
        drain_outputs(run);
    }
}

pub(super) fn fill_batch(venue: PmVenueOrderKey, count: usize) -> String {
    fill_batch_from(venue, 1, count)
}

pub(super) fn fill_batches(venue: PmVenueOrderKey, count: usize) -> Vec<String> {
    let batch_count = count.div_ceil(MAX_PRIVATE_FRAME_EVENTS);
    (0..batch_count)
        .map(|batch_index| {
            let first_attempt = batch_index * MAX_PRIVATE_FRAME_EVENTS + 1;
            let batch_length =
                MAX_PRIVATE_FRAME_EVENTS.min(count.saturating_sub(first_attempt - 1));
            fill_batch_from(venue, first_attempt, batch_length)
        })
        .collect()
}

fn fill_batch_from(venue: PmVenueOrderKey, first_attempt: usize, count: usize) -> String {
    let venue_id = venue.id();
    let rows = (first_attempt..first_attempt + count)
        .map(|attempt| {
            format!(
                r#"{{"event_type":"trade","id":"phase6-fill-{attempt:04}","market":"{}","asset_id":"{}","side":"BUY","size":"0.01","price":"0.40","status":"MATCHED","maker_address":"{}","transaction_hash":"0xfeed","order_id":"{}","trader_side":"MAKER"}}"#,
                super::super::fixture::MARKET,
                super::super::fixture::TOKEN,
                super::super::fixture::PM_FUNDER,
                venue_id.as_str(),
            )
        })
        .collect::<Vec<_>>()
        .join(",");
    format!("[{rows}]")
}

pub(super) async fn wait_for_ack_admission(
    run: &mut crate::PmProductRun<Phase6Model>,
    sequence: u64,
    monotonic_ns: u64,
) -> Result<(), String> {
    let deadline = tokio::time::Instant::now() + Duration::from_secs(3);
    loop {
        match run.poll_persistence_fixture(completion(sequence, monotonic_ns), monotonic_ns) {
            Ok(true) => return Ok(()),
            Ok(false) if tokio::time::Instant::now() < deadline => tokio::task::yield_now().await,
            Ok(false) => return Err("timed out waiting for durable acknowledgement".to_string()),
            Err(error) => return Err(error.to_string()),
        }
    }
}

pub(super) fn drain_outputs(run: &mut crate::PmProductRun<Phase6Model>) -> usize {
    let mut fake = 0;
    while let Some(effect) = run.pop_effect() {
        if matches!(
            effect,
            PmProductEffect::FakePassiveQuote(_) | PmProductEffect::FakeCancelOwned(_)
        ) {
            fake += 1;
        }
    }
    fake
}

pub(super) fn completion(sequence: u64, monotonic_ns: u64) -> PmFixtureCompletionOccurrence {
    PmFixtureCompletionOccurrence::new(
        ReceivedEventClock::new(None, 1_700_000_000_000_000_000 + monotonic_ns, monotonic_ns)
            .expect("fixed receive clock"),
        EventOrdering::new(
            ConnectionEpoch::new(1),
            None,
            None,
            None,
            IngressSequence::new(sequence),
        )
        .expect("fixed ordering"),
    )
}
