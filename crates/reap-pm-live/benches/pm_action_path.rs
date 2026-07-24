use reap_benchmark_allocator::TrackingAllocator;
use serde_json::Value;

#[global_allocator]
static ALLOCATOR: TrackingAllocator = TrackingAllocator;

fn main() {
    let encoded = reap_pm_live::run_pm_action_path_evidence()
        .expect("fixed PM action-path suite must succeed");
    let suite: Value =
        serde_json::from_str(&encoded).expect("fixed PM action-path suite must emit valid JSON");
    assert_eq!(number(&suite, "warmup_runs"), 1);
    assert_eq!(suite["production_order_entry_authorized"], false);
    let recorded = suite["recorded_runs"]
        .as_array()
        .expect("recorded_runs must be an array");
    assert_eq!(recorded.len(), 3);
    let mut baseline_journal_hashes: Option<Vec<String>> = None;
    let mut baseline_logical_hashes: Option<Vec<String>> = None;
    for (index, report) in recorded.iter().enumerate() {
        let ordinal = index + 1;
        validate_recorded_run(ordinal, report);
        let journal_hashes = pass_strings(report, "journal_hash");
        let logical_hashes = pass_strings(report, "logical_hash");
        match (&baseline_journal_hashes, &baseline_logical_hashes) {
            (Some(expected_journal), Some(expected_logical)) => {
                assert_eq!(
                    &journal_hashes, expected_journal,
                    "recorded run {ordinal} changed the full sealed journal projection"
                );
                assert_eq!(
                    &logical_hashes, expected_logical,
                    "recorded run {ordinal} changed the normalized effect projection"
                );
            }
            (None, None) => {
                baseline_journal_hashes = Some(journal_hashes);
                baseline_logical_hashes = Some(logical_hashes);
            }
            _ => unreachable!("baseline projections are initialized together"),
        }
    }
    println!("{encoded}");
}

fn validate_recorded_run(ordinal: usize, report: &Value) {
    assert_eq!(report["production_order_entry_authorized"], false);
    let capacities = object(report, "capacities");
    assert_eq!(number(capacities, "raw_entry_capacity"), 8_192);
    assert_eq!(number(capacities, "raw_entry_high_water"), 0);
    assert_eq!(
        number(capacities, "raw_payload_byte_capacity"),
        32 * 1_024 * 1_024
    );
    assert_eq!(number(capacities, "raw_payload_byte_high_water"), 0);
    let latency = object(report, "action_latency_ns");
    assert_eq!(number(latency, "samples"), 15_000);
    assert!(
        number(latency, "p50") <= 25_000,
        "recorded run {ordinal} exceeded the 25us p50 limit"
    );
    assert!(
        number(latency, "p99_9") <= 250_000,
        "recorded run {ordinal} exceeded the 250us p99.9 limit"
    );
    let allocations = object(report, "owner_allocations");
    assert_eq!(
        number(allocations, "allocation_calls"),
        0,
        "recorded run {ordinal} allocated in the normalized owner loop"
    );
    assert_eq!(number(allocations, "allocated_bytes"), 0);
    let passes = report["repeated_passes"]
        .as_array()
        .expect("repeated_passes must be an array");
    assert_eq!(passes.len(), 5);
    assert!(passes.iter().all(|pass| {
        pass["terminal_state_lengths_zero"]
            .as_bool()
            .unwrap_or(false)
    }));
}

fn pass_strings(report: &Value, field: &str) -> Vec<String> {
    report["repeated_passes"]
        .as_array()
        .expect("repeated_passes must be an array")
        .iter()
        .map(|pass| {
            pass[field]
                .as_str()
                .unwrap_or_else(|| panic!("{field} must be a string"))
                .to_owned()
        })
        .collect()
}

fn object<'a>(value: &'a Value, field: &str) -> &'a Value {
    value
        .get(field)
        .unwrap_or_else(|| panic!("missing report field {field}"))
}

fn number(value: &Value, field: &str) -> u64 {
    value[field]
        .as_u64()
        .unwrap_or_else(|| panic!("{field} must be a u64"))
}
