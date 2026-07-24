use std::time::Instant;

use reap_okx_public_source::OkxPublicSessionEvent;
use reap_pm_core::{OkxReferencePrice, PmBookUpdate};
use sha2::{Digest, Sha256};

use super::PmEvidenceError;
use super::contract::{PARSER_SAMPLES_PER_VENUE, PARSER_WARMUP_PER_VENUE};
use super::fixture::{
    okx_ack_frame, okx_reference_frame, public_sessions, snapshot_frame, top_frame,
};
use super::report::{AllocationReport, LatencySummary, ParserReport, hex, sha256_hex};

pub(crate) fn run_parser_segment() -> Result<ParserReport, PmEvidenceError> {
    let snapshot = snapshot_frame();
    let top = top_frame();
    let (mut pm, mut okx) = public_sessions();
    pm.mark_subscription_sent(60)
        .map_err(|error| PmEvidenceError::invariant(error.to_string()))?;
    let snapshot_batch = pm
        .classify(snapshot.as_bytes(), wall(100), 100)
        .map_err(|error| PmEvidenceError::invariant(error.to_string()))?;
    let token = snapshot_batch
        .snapshot_flow_token()
        .ok_or_else(|| PmEvidenceError::invariant("parser setup snapshot omitted flow token"))?;
    pm.open_protocol_flow_after_snapshot(token)
        .map_err(|error| PmEvidenceError::invariant(error.to_string()))?;
    let ack = okx
        .classify_captured_payload(okx_ack_frame(), wall(100), 100, 1)
        .map_err(|error| PmEvidenceError::invariant(error.to_string()))?;
    if !matches!(
        ack.payload(),
        OkxPublicSessionEvent::SubscriptionAcknowledged(_)
    ) {
        return Err(PmEvidenceError::invariant(
            "OKX parser setup did not acknowledge the fixed subscription",
        ));
    }

    let mut projection = Sha256::new();
    for ordinal in 0..PARSER_WARMUP_PER_VENUE {
        let monotonic = 200 + ordinal as u64;
        consume_pm(&mut pm, top.as_bytes(), wall(monotonic), monotonic, None)?;
        consume_okx(
            &mut okx,
            okx_reference_frame(),
            wall(monotonic),
            monotonic,
            10 + ordinal as u64,
            None,
        )?;
    }

    let mut pm_latency = Vec::with_capacity(PARSER_SAMPLES_PER_VENUE);
    let mut okx_latency = Vec::with_capacity(PARSER_SAMPLES_PER_VENUE);
    let allocation_window = reap_benchmark_allocator::start_measurement()
        .map_err(|error| PmEvidenceError::invariant(error.to_string()))?;
    for ordinal in 0..PARSER_SAMPLES_PER_VENUE {
        let monotonic = 10_000 + ordinal as u64;
        consume_pm(
            &mut pm,
            top.as_bytes(),
            wall(monotonic),
            monotonic,
            Some((&mut projection, &mut pm_latency)),
        )?;
        consume_okx(
            &mut okx,
            okx_reference_frame(),
            wall(monotonic),
            monotonic,
            20_000 + ordinal as u64,
            Some((&mut projection, &mut okx_latency)),
        )?;
    }
    let allocation: AllocationReport = allocation_window
        .stop()
        .map_err(|error| PmEvidenceError::invariant(error.to_string()))?
        .into();

    let fixture_sha256 = {
        let mut fixtures = Vec::with_capacity(top.len() + okx_reference_frame().len());
        fixtures.extend_from_slice(top.as_bytes());
        fixtures.extend_from_slice(okx_reference_frame().as_bytes());
        sha256_hex(&fixtures)
    };
    let digest = projection.finalize();
    let mut projection_hash = [0_u8; 32];
    projection_hash.copy_from_slice(&digest);
    Ok(ParserReport {
        warmup_pm_frames: PARSER_WARMUP_PER_VENUE,
        warmup_okx_frames: PARSER_WARMUP_PER_VENUE,
        warmup_pm_bytes: top
            .len()
            .checked_mul(PARSER_WARMUP_PER_VENUE)
            .ok_or_else(|| PmEvidenceError::invariant("PM warmup parser bytes overflow"))?,
        warmup_okx_bytes: okx_reference_frame()
            .len()
            .checked_mul(PARSER_WARMUP_PER_VENUE)
            .ok_or_else(|| PmEvidenceError::invariant("OKX warmup parser bytes overflow"))?,
        measured_pm_frames: PARSER_SAMPLES_PER_VENUE,
        measured_okx_frames: PARSER_SAMPLES_PER_VENUE,
        measured_pm_bytes: top
            .len()
            .checked_mul(PARSER_SAMPLES_PER_VENUE)
            .ok_or_else(|| PmEvidenceError::invariant("PM measured parser bytes overflow"))?,
        measured_okx_bytes: okx_reference_frame()
            .len()
            .checked_mul(PARSER_SAMPLES_PER_VENUE)
            .ok_or_else(|| PmEvidenceError::invariant("OKX measured parser bytes overflow"))?,
        pm_latency_ns: LatencySummary::from_samples(&mut pm_latency),
        okx_latency_ns: LatencySummary::from_samples(&mut okx_latency),
        allocation,
        fixture_sha256,
        projection_sha256: hex(projection_hash),
        matches_owner_corpus: false,
    })
}

fn consume_pm(
    session: &mut reap_polymarket_adapter::PmPublicSession,
    raw: &[u8],
    wall_ns: u64,
    monotonic_ns: u64,
    measured: Option<(&mut Sha256, &mut Vec<u64>)>,
) -> Result<(), PmEvidenceError> {
    let started = measured.as_ref().map(|_| Instant::now());
    let batch = session
        .classify(raw, wall_ns, monotonic_ns)
        .map_err(|error| PmEvidenceError::invariant(error.to_string()))?;
    if let (Some(started), Some((projection, latencies))) = (started, measured) {
        latencies.push(elapsed_ns(started));
        let event = batch
            .events()
            .first()
            .ok_or_else(|| PmEvidenceError::invariant("PM parser emitted no top-check event"))?;
        let PmBookUpdate::TopCheck(top) = event.payload().update() else {
            return Err(PmEvidenceError::invariant(
                "PM parser corpus did not normalize to a top check",
            ));
        };
        projection.update(b"pm");
        projection.update(
            top.bid()
                .ok_or_else(|| PmEvidenceError::invariant("PM top omitted bid"))?
                .units()
                .to_be_bytes(),
        );
        projection.update(
            top.ask()
                .ok_or_else(|| PmEvidenceError::invariant("PM top omitted ask"))?
                .units()
                .to_be_bytes(),
        );
    }
    Ok(())
}

fn consume_okx(
    session: &mut reap_okx_public_source::OkxPublicSession,
    raw: &str,
    wall_ns: u64,
    monotonic_ns: u64,
    raw_hash: u64,
    measured: Option<(&mut Sha256, &mut Vec<u64>)>,
) -> Result<(), PmEvidenceError> {
    let started = measured.as_ref().map(|_| Instant::now());
    let delivery = session
        .classify_captured_payload(raw, wall_ns, monotonic_ns, raw_hash)
        .map_err(|error| PmEvidenceError::invariant(error.to_string()))?;
    if let (Some(started), Some((projection, latencies))) = (started, measured) {
        latencies.push(elapsed_ns(started));
        let OkxPublicSessionEvent::Reference(reference) = delivery.payload() else {
            return Err(PmEvidenceError::invariant(
                "OKX parser corpus did not normalize to a reference",
            ));
        };
        projection.update(b"okx");
        let price = OkxReferencePrice::parse_decimal(reference.index_price_lexeme())
            .map_err(|error| PmEvidenceError::invariant(error.to_string()))?;
        projection.update(price.coefficient().to_be_bytes());
        projection.update([price.decimal_scale()]);
    }
    Ok(())
}

fn elapsed_ns(started: Instant) -> u64 {
    u64::try_from(started.elapsed().as_nanos()).unwrap_or(u64::MAX)
}

fn wall(monotonic_ns: u64) -> u64 {
    1_700_000_000_000_000_000_u64.saturating_add(monotonic_ns)
}
