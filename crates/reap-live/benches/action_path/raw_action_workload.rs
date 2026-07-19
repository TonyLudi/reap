use super::*;

struct RawGapPair {
    recovery_snapshot: RawEnvelope,
    gap_update: RawEnvelope,
}

struct RawActionScenario {
    adapter: OkxAdapter,
    processor: FeedProcessor,
    coordinator: LiveCoordinator,
    pairs: Vec<RawGapPair>,
}

pub(super) fn raw_sequence_gap_action_record_workload() -> WorkloadResult {
    let result = run_workload(
        "raw_sequence_gap_recovery_action_record",
        "two credential-free raw OKX book frames per observation; production OkxAdapter parsing; \
         FeedProcessor deduplication, sequence tracking, recovery/system outputs, and canonical \
         book reduction; LiveCoordinator process_feed decisions; produced RecoverBook action and \
         canonical normalized/system storage-record projection",
        "socket receive and production channel scheduling; storage enqueue/disk; regular-order \
         policy/gateway preparation and adapter command serialization; network IO; exchange \
         acknowledgement",
        || RawActionScenario {
            adapter: OkxAdapter::default(),
            processor: FeedProcessor::new(100_000, 4_096),
            coordinator: benchmark_coordinator(),
            pairs: build_raw_gap_pairs(WARMUP_OBSERVATIONS + TIMED_OBSERVATIONS),
        },
        |scenario, index| {
            let pair = &scenario.pairs[index];
            let mut counters = LogicalCounters {
                inputs: 1,
                ..LogicalCounters::default()
            };
            for envelope in [&pair.recovery_snapshot, &pair.gap_update] {
                counters.frames += 1;
                let parsed = scenario
                    .adapter
                    .parse(envelope)
                    .expect("benchmark raw OKX book frame must parse");
                counters.parsed_events += parsed.len() as u64;
                for event in parsed {
                    let outputs = scenario.processor.process_from(&envelope.conn_id, event);
                    counters.feed_outputs += outputs.len() as u64;
                    for output in outputs {
                        if matches!(&output, FeedOutput::Event(_) | FeedOutput::System(_)) {
                            counters.normalized_outputs += 1;
                        }
                        let reduced = scenario
                            .coordinator
                            .process_feed(output)
                            .expect("benchmark feed output must reduce");
                        let action_count = reduced.action_count() as u64;
                        counters.coordinator_actions += action_count;
                        counters.produced_actions += action_count;
                        counters.storage_records += reduced.record_count() as u64;
                        black_box(reduced);
                    }
                }
            }
            Observation {
                counters,
                queue_age_ns: None,
            }
        },
    );
    assert_eq!(result.counters.frames, (TIMED_OBSERVATIONS * 2) as u64);
    assert_eq!(
        result.counters.parsed_events,
        (TIMED_OBSERVATIONS * 2) as u64
    );
    assert_eq!(
        result.counters.feed_outputs,
        (TIMED_OBSERVATIONS * 6) as u64
    );
    assert_eq!(
        result.counters.normalized_outputs,
        (TIMED_OBSERVATIONS * 5) as u64
    );
    assert_eq!(
        result.counters.coordinator_actions, TIMED_OBSERVATIONS as u64,
        "every deliberate source sequence gap must produce one RecoverBook action"
    );
    assert_eq!(
        result.counters.storage_records,
        (TIMED_OBSERVATIONS * 9 + 3) as u64,
        "raw-to-action reduction must retain its exact canonical record projection"
    );
    result
}

fn build_raw_gap_pairs(observations: usize) -> Vec<RawGapPair> {
    (0..observations)
        .map(|index| {
            let sequence = 10_000_i64
                .checked_add((index as i64).saturating_mul(10))
                .expect("benchmark sequence");
            let ts_ms = BASE_TS_MS
                .checked_add(index as u64)
                .expect("benchmark timestamp");
            RawGapPair {
                recovery_snapshot: raw_book_envelope(
                    ts_ms,
                    book_payload("snapshot", ts_ms, -1, sequence),
                ),
                gap_update: raw_book_envelope(
                    ts_ms,
                    book_payload("update", ts_ms, sequence + 5, sequence + 6),
                ),
            }
        })
        .collect()
}

fn book_payload(action: &'static str, ts_ms: u64, previous_sequence: i64, sequence: i64) -> String {
    json!({
        "arg": {"channel": "books", "instId": "BTC-USDT"},
        "action": action,
        "data": [{
            "asks": [["50001.0", "10", "0", "1"]],
            "bids": [["50000.0", "10", "0", "1"]],
            "ts": ts_ms.to_string(),
            "prevSeqId": previous_sequence,
            "seqId": sequence
        }]
    })
    .to_string()
}

fn raw_book_envelope(ts_ms: u64, payload: String) -> RawEnvelope {
    RawEnvelope {
        venue: Venue::Okx,
        conn_id: ConnId::new("action-gap-primary"),
        channel: Channel::Books,
        symbol: Some("BTC-USDT".to_string()),
        recv_ts_ns: ts_ms.saturating_mul(1_000_000),
        raw_hash: payload_hash(payload.as_bytes()),
        payload,
    }
}
