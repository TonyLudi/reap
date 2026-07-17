use std::alloc::{GlobalAlloc, Layout, System};
use std::collections::{HashMap, HashSet};
use std::hint::black_box;
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use std::time::{Duration, Instant};

use reap_core::{
    AccountUpdate, Balance, Channel, ConnId, NormalizedEvent, RawEnvelope, SystemEvent,
    SystemEventKind, Venue,
};
use reap_feed::{FeedOutput, FeedProcessor, payload_hash};
use reap_live::{
    LiveConfig, LiveCoordinator, ReconciliationResult, VerifiedBootstrap, VerifiedInstrument,
};
use reap_risk::{InstrumentOrderLimits, InstrumentRiskModel};
use reap_storage::StorageRecord;
use reap_strategy::InstrumentKindConfig;
use reap_venue::okx::{OkxAdapter, OkxInstrumentType};
use reap_venue::{ParsedEvent, VenueAdapter};
use serde_json::{Value, json};

const LOGICAL_BOOK_UPDATES: usize = 20_000;
const SNAPSHOT_LEVELS: usize = 400;
const ACCOUNT_ID: &str = "main";

static TRACK_ALLOCATIONS: AtomicBool = AtomicBool::new(false);
static ALLOCATION_CALLS: AtomicU64 = AtomicU64::new(0);
static ALLOCATED_BYTES: AtomicU64 = AtomicU64::new(0);

struct TrackingAllocator;

#[global_allocator]
static GLOBAL_ALLOCATOR: TrackingAllocator = TrackingAllocator;

unsafe impl GlobalAlloc for TrackingAllocator {
    unsafe fn alloc(&self, layout: Layout) -> *mut u8 {
        // SAFETY: this delegates the allocation unchanged to the system allocator.
        let pointer = unsafe { System.alloc(layout) };
        track_allocation(layout.size());
        pointer
    }

    unsafe fn alloc_zeroed(&self, layout: Layout) -> *mut u8 {
        // SAFETY: this delegates the allocation unchanged to the system allocator.
        let pointer = unsafe { System.alloc_zeroed(layout) };
        track_allocation(layout.size());
        pointer
    }

    unsafe fn dealloc(&self, pointer: *mut u8, layout: Layout) {
        // SAFETY: the pointer and layout came from the system allocator above.
        unsafe { System.dealloc(pointer, layout) };
    }

    unsafe fn realloc(&self, pointer: *mut u8, layout: Layout, new_size: usize) -> *mut u8 {
        // SAFETY: the pointer and layout came from the system allocator above.
        let pointer = unsafe { System.realloc(pointer, layout, new_size) };
        track_allocation(new_size);
        pointer
    }
}

fn track_allocation(bytes: usize) {
    if TRACK_ALLOCATIONS.load(Ordering::Relaxed) {
        ALLOCATION_CALLS.fetch_add(1, Ordering::Relaxed);
        ALLOCATED_BYTES.fetch_add(bytes as u64, Ordering::Relaxed);
    }
}

#[derive(Debug, Clone, Copy)]
struct AllocationSnapshot {
    calls: u64,
    bytes: u64,
}

fn start_allocation_tracking() {
    ALLOCATION_CALLS.store(0, Ordering::Relaxed);
    ALLOCATED_BYTES.store(0, Ordering::Relaxed);
    TRACK_ALLOCATIONS.store(true, Ordering::SeqCst);
}

fn stop_allocation_tracking() -> AllocationSnapshot {
    TRACK_ALLOCATIONS.store(false, Ordering::SeqCst);
    AllocationSnapshot {
        calls: ALLOCATION_CALLS.load(Ordering::Relaxed),
        bytes: ALLOCATED_BYTES.load(Ordering::Relaxed),
    }
}

#[derive(Debug)]
struct Measurement<T> {
    value: T,
    elapsed: Duration,
    allocations: AllocationSnapshot,
}

fn measure<T>(run: impl FnOnce() -> T) -> Measurement<T> {
    start_allocation_tracking();
    let started = Instant::now();
    let value = run();
    let elapsed = started.elapsed();
    let allocations = stop_allocation_tracking();
    Measurement {
        value,
        elapsed,
        allocations,
    }
}

#[derive(Debug, Clone, Copy)]
enum Source {
    Public,
    Private,
}

#[derive(Debug)]
struct CaptureFrame {
    source: Source,
    envelope: RawEnvelope,
}

struct Adapters {
    public: OkxAdapter,
    private: OkxAdapter,
}

impl Adapters {
    fn new() -> Self {
        Self {
            public: OkxAdapter::default(),
            private: OkxAdapter::default().with_account_id(ACCOUNT_ID),
        }
    }

    fn parse(&self, frame: &CaptureFrame) -> Vec<ParsedEvent> {
        match frame.source {
            Source::Public => self.public.parse(&frame.envelope),
            Source::Private => self.private.parse(&frame.envelope),
        }
        .expect("benchmark wire payload must parse")
    }
}

#[derive(Debug, Default, Clone, Copy)]
struct WorkCounters {
    parsed: u64,
    feed_outputs: u64,
    records: u64,
    actions: u64,
}

fn main() {
    let workload = build_workload(LOGICAL_BOOK_UPDATES);
    assert!(
        workload
            .iter()
            .filter(|frame| matches!(frame.source, Source::Public))
            .count()
            .is_multiple_of(2),
        "every public wire event must have a redundant copy"
    );

    let parse = benchmark_parse(&workload);
    print_measurement("wire_parse_and_raw_record", workload.len(), &parse);

    let feed = benchmark_feed(&workload);
    print_measurement("dedup_sequence_and_book", workload.len(), &feed);

    let coordinator = benchmark_coordinator(&workload);
    print_measurement(
        "coordinator_strategy_risk_storage_records",
        coordinator.value.feed_outputs as usize,
        &coordinator,
    );

    let parity = benchmark_parity(&workload);
    print_measurement("live_parity_observe", workload.len(), &parity);

    assert!(parity.value.parsed > 0);
    assert!(parity.value.feed_outputs > 0);
    assert!(parity.value.records > 0);
    black_box(parity.value);
}

fn benchmark_parse(workload: &[CaptureFrame]) -> Measurement<WorkCounters> {
    let adapters = Adapters::new();
    measure(|| {
        let mut counters = WorkCounters::default();
        for frame in workload {
            black_box(raw_storage_record(frame));
            let parsed = adapters.parse(frame);
            counters.parsed += parsed.len() as u64;
            black_box(parsed);
        }
        counters
    })
}

fn benchmark_feed(workload: &[CaptureFrame]) -> Measurement<WorkCounters> {
    let parsed = preparse(workload);
    let mut processor = FeedProcessor::new(100_000, 4_096);
    measure(|| {
        let mut counters = WorkCounters::default();
        for (source, events) in parsed {
            counters.parsed += events.len() as u64;
            for event in events {
                let outputs = processor.process_from(&source, event);
                counters.feed_outputs += outputs.len() as u64;
                black_box(outputs);
            }
        }
        counters
    })
}

fn benchmark_coordinator(workload: &[CaptureFrame]) -> Measurement<WorkCounters> {
    let outputs = precompute_feed_outputs(workload);
    let output_count = outputs.len() as u64;
    let mut coordinator = benchmark_coordinator_state();
    measure(|| {
        let mut counters = WorkCounters {
            feed_outputs: output_count,
            ..WorkCounters::default()
        };
        for output in outputs {
            let output = coordinator
                .process_feed(output)
                .expect("benchmark feed output must be accepted");
            counters.records += output.record_count() as u64;
            counters.actions += output.action_count() as u64;
            black_box(output);
        }
        counters
    })
}

fn benchmark_parity(workload: &[CaptureFrame]) -> Measurement<WorkCounters> {
    let adapters = Adapters::new();
    let mut processor = FeedProcessor::new(100_000, 4_096);
    let mut coordinator = benchmark_coordinator_state();
    measure(|| {
        let mut counters = WorkCounters::default();
        for frame in workload {
            black_box(raw_storage_record(frame));
            for event in adapters.parse(frame) {
                counters.parsed += 1;
                for output in processor.process_from(&frame.envelope.conn_id, event) {
                    counters.feed_outputs += 1;
                    let output = coordinator
                        .process_feed(output)
                        .expect("benchmark feed output must be accepted");
                    counters.records += output.record_count() as u64;
                    counters.actions += output.action_count() as u64;
                    black_box(output);
                }
            }
        }
        counters
    })
}

fn print_measurement(name: &str, units: usize, measurement: &Measurement<WorkCounters>) {
    let elapsed_ms = measurement.elapsed.as_secs_f64() * 1_000.0;
    let nanos_per_unit = measurement.elapsed.as_nanos() as f64 / units as f64;
    let allocations_per_unit = measurement.allocations.calls as f64 / units as f64;
    let bytes_per_unit = measurement.allocations.bytes as f64 / units as f64;
    println!(
        "{name}: units={units} elapsed_ms={elapsed_ms:.3} ns_per_unit={nanos_per_unit:.1} \
         allocations={} allocations_per_unit={allocations_per_unit:.2} allocated_bytes={} \
         bytes_per_unit={bytes_per_unit:.1} parsed={} feed_outputs={} records={} actions={}",
        measurement.allocations.calls,
        measurement.allocations.bytes,
        measurement.value.parsed,
        measurement.value.feed_outputs,
        measurement.value.records,
        measurement.value.actions,
    );
}

fn preparse(workload: &[CaptureFrame]) -> Vec<(ConnId, Vec<ParsedEvent>)> {
    let adapters = Adapters::new();
    workload
        .iter()
        .map(|frame| (frame.envelope.conn_id.clone(), adapters.parse(frame)))
        .collect()
}

fn precompute_feed_outputs(workload: &[CaptureFrame]) -> Vec<FeedOutput> {
    let mut processor = FeedProcessor::new(100_000, 4_096);
    let mut outputs = Vec::new();
    for (source, events) in preparse(workload) {
        for event in events {
            outputs.extend(processor.process_from(&source, event));
        }
    }
    outputs
}

fn raw_storage_record(frame: &CaptureFrame) -> StorageRecord {
    StorageRecord::Raw {
        account_id: matches!(frame.source, Source::Private).then(|| ACCOUNT_ID.to_string()),
        envelope: frame.envelope.clone(),
    }
}

fn benchmark_coordinator_state() -> LiveCoordinator {
    let config = LiveConfig::from_toml(include_str!("../../../examples/live-okx-demo.toml"))
        .expect("benchmark live config");
    let instruments = config
        .strategy
        .instruments
        .iter()
        .map(|instrument| {
            let account = config
                .account_for_symbol(&instrument.symbol)
                .expect("configured symbol must have an account");
            let risk_model = if instrument.kind.is_spot() {
                InstrumentRiskModel::Spot
            } else if instrument.kind.is_inverse() {
                InstrumentRiskModel::InverseDerivative {
                    contract_value: instrument.contract_value,
                }
            } else {
                InstrumentRiskModel::LinearDerivative {
                    contract_value: instrument.contract_value,
                }
            };
            let instrument_type = match instrument.kind {
                InstrumentKindConfig::Spot => OkxInstrumentType::Spot,
                InstrumentKindConfig::LinearSwap | InstrumentKindConfig::InverseSwap => {
                    OkxInstrumentType::Swap
                }
                InstrumentKindConfig::Future
                | InstrumentKindConfig::LinearFuture
                | InstrumentKindConfig::InverseFuture => OkxInstrumentType::Futures,
            };
            (
                instrument.symbol.clone(),
                VerifiedInstrument {
                    account_id: account.id.clone(),
                    symbol: instrument.symbol.clone(),
                    instrument_type,
                    trade_mode: account.trade_modes[&instrument.symbol],
                    risk_model,
                    order_limits: InstrumentOrderLimits {
                        max_limit_quantity: 1_000_000.0,
                        max_limit_notional_usd: instrument.kind.is_spot().then_some(1_000_000.0),
                    },
                    tick_size: instrument.tick_size,
                    lot_size: instrument.lot_size,
                    min_size: instrument.min_trade_size,
                    contract_value: instrument
                        .kind
                        .is_derivative()
                        .then_some(instrument.contract_value),
                },
            )
        })
        .collect();
    let account_update = AccountUpdate {
        ts_ms: 1_700_000_000_000,
        balances: vec![Balance {
            account_id: Some(ACCOUNT_ID.to_string()),
            currency: "USDT".to_string(),
            total: 100_000.0,
            available: 100_000.0,
            equity: 100_000.0,
            liability: 0.0,
            max_loan: 0.0,
            forced_repayment_indicator: None,
        }],
        positions: Vec::new(),
        margins: Vec::new(),
    };
    let verified = VerifiedBootstrap {
        instruments,
        account_updates: HashMap::from([(ACCOUNT_ID.to_string(), account_update.clone())]),
        baseline_fill_ids: HashMap::from([(ACCOUNT_ID.to_string(), HashSet::new())]),
        quote_stp_verified_accounts: HashSet::from([ACCOUNT_ID.to_string()]),
    };
    let mut coordinator =
        LiveCoordinator::new(config, verified, HashMap::new(), "benchmark-session")
            .expect("benchmark coordinator");
    coordinator.mark_storage_ready(true, "benchmark sink ready");
    coordinator.mark_public_connectivity(true, "redundant benchmark sources ready");
    coordinator
        .process_feed(FeedOutput::PrivateAccount {
            account_id: Some(ACCOUNT_ID.to_string()),
            update: account_update,
        })
        .expect("benchmark account snapshot");
    coordinator
        .on_reconciliation(ReconciliationResult {
            account_id: ACCOUNT_ID.to_string(),
            ts_ms: 1_700_000_000_000,
            clean: true,
            local_live_orders: 0,
            remote_live_orders: 0,
            remote_recent_fills: 0,
            reason: "benchmark bootstrap is clean".to_string(),
        })
        .expect("benchmark reconciliation");
    coordinator.process_event(NormalizedEvent::System(SystemEvent {
        ts_ms: 1_700_000_000_000,
        kind: SystemEventKind::PrivateStreamRecovered,
        venue: Some(Venue::Okx),
        account_id: Some(ACCOUNT_ID.to_string()),
        symbol: None,
        reason: "benchmark private stream ready".to_string(),
    }));
    coordinator
}

fn build_workload(logical_book_updates: usize) -> Vec<CaptureFrame> {
    let symbols = ["BTC-USDT", "BTC-USDT-SWAP"];
    let bases = [45_000.0, 45_002.0];
    let mut sequence = [1_000_000_i64, 2_000_000_i64];
    let mut recv_ts_ns = 1_700_000_000_000_000_000_u64;
    let mut frames = Vec::with_capacity(logical_book_updates * 3);

    for (index, symbol) in symbols.iter().enumerate() {
        let payload = book_snapshot(symbol, bases[index], sequence[index]);
        push_public_pair(
            &mut frames,
            Channel::Books,
            symbol,
            payload,
            &mut recv_ts_ns,
        );
    }

    for index in 0..logical_book_updates {
        let symbol_index = index % symbols.len();
        let symbol = symbols[symbol_index];
        let previous = sequence[symbol_index];
        sequence[symbol_index] += 1;
        let level_index = (index / symbols.len()) % SNAPSHOT_LEVELS;
        let payload = book_update(
            symbol,
            bases[symbol_index],
            level_index,
            previous,
            sequence[symbol_index],
            index,
        );
        push_public_pair(
            &mut frames,
            Channel::Books,
            symbol,
            payload,
            &mut recv_ts_ns,
        );

        if index.is_multiple_of(4) {
            push_public_pair(
                &mut frames,
                Channel::Trades,
                symbol,
                trade(symbol, bases[symbol_index], index),
                &mut recv_ts_ns,
            );
        }
        if index.is_multiple_of(1_000) {
            push_pricing_updates(&mut frames, index, bases[symbol_index], &mut recv_ts_ns);
        }
        if index.is_multiple_of(500) {
            push_private_account(&mut frames, index, &mut recv_ts_ns);
        }
    }
    frames
}

fn book_snapshot(symbol: &str, mid: f64, sequence: i64) -> String {
    let bids = (0..SNAPSHOT_LEVELS)
        .map(|level| {
            json!([
                format!("{:.1}", mid - 0.5 - level as f64 * 0.1),
                format!("{:.4}", 1.0 + level as f64 * 0.01),
                "0",
                "1"
            ])
        })
        .collect::<Vec<Value>>();
    let asks = (0..SNAPSHOT_LEVELS)
        .map(|level| {
            json!([
                format!("{:.1}", mid + 0.5 + level as f64 * 0.1),
                format!("{:.4}", 1.0 + level as f64 * 0.01),
                "0",
                "1"
            ])
        })
        .collect::<Vec<Value>>();
    json!({
        "arg": {"channel": "books", "instId": symbol},
        "action": "snapshot",
        "data": [{
            "asks": asks,
            "bids": bids,
            "ts": "1700000000000",
            "prevSeqId": -1,
            "seqId": sequence
        }]
    })
    .to_string()
}

fn book_update(
    symbol: &str,
    mid: f64,
    level: usize,
    previous_sequence: i64,
    sequence: i64,
    index: usize,
) -> String {
    let ts_ms = 1_700_000_000_001_u64 + index as u64;
    json!({
        "arg": {"channel": "books", "instId": symbol},
        "action": "update",
        "data": [{
            "asks": [[
                format!("{:.1}", mid + 0.5 + level as f64 * 0.1),
                format!("{:.4}", 1.0 + (index % 37) as f64 * 0.01),
                "0",
                "1"
            ]],
            "bids": [[
                format!("{:.1}", mid - 0.5 - level as f64 * 0.1),
                format!("{:.4}", 1.0 + (index % 41) as f64 * 0.01),
                "0",
                "1"
            ]],
            "ts": ts_ms.to_string(),
            "prevSeqId": previous_sequence,
            "seqId": sequence
        }]
    })
    .to_string()
}

fn trade(symbol: &str, price: f64, index: usize) -> String {
    let ts_ms = 1_700_000_000_001_u64 + index as u64;
    json!({
        "arg": {"channel": "trades", "instId": symbol},
        "data": [{
            "instId": symbol,
            "tradeId": format!("trade-{index}"),
            "px": format!("{price:.1}"),
            "sz": "0.002",
            "side": if index.is_multiple_of(2) { "buy" } else { "sell" },
            "ts": ts_ms.to_string()
        }]
    })
    .to_string()
}

fn push_pricing_updates(
    frames: &mut Vec<CaptureFrame>,
    index: usize,
    price: f64,
    recv_ts_ns: &mut u64,
) {
    let ts_ms = 1_700_000_000_001_u64 + index as u64;
    let updates = [
        (
            Channel::Custom("funding-rate".to_string()),
            "BTC-USDT-SWAP",
            json!({
                "arg": {"channel": "funding-rate", "instId": "BTC-USDT-SWAP"},
                "data": [{
                    "instId": "BTC-USDT-SWAP",
                    "fundingRate": "0.0001",
                    "fundingTime": (ts_ms + 28_800_000).to_string(),
                    "ts": ts_ms.to_string()
                }]
            }),
        ),
        (
            Channel::Custom("index-tickers".to_string()),
            "BTC-USDT",
            json!({
                "arg": {"channel": "index-tickers", "instId": "BTC-USDT"},
                "data": [{
                    "instId": "BTC-USDT",
                    "idxPx": format!("{price:.1}"),
                    "ts": ts_ms.to_string()
                }]
            }),
        ),
        (
            Channel::Custom("price-limit".to_string()),
            "BTC-USDT-SWAP",
            json!({
                "arg": {"channel": "price-limit", "instId": "BTC-USDT-SWAP"},
                "data": [{
                    "instId": "BTC-USDT-SWAP",
                    "buyLmt": format!("{:.1}", price * 1.1),
                    "sellLmt": format!("{:.1}", price * 0.9),
                    "ts": ts_ms.to_string()
                }]
            }),
        ),
        (
            Channel::Custom("mark-price".to_string()),
            "BTC-USDT-SWAP",
            json!({
                "arg": {"channel": "mark-price", "instId": "BTC-USDT-SWAP"},
                "data": [{
                    "instId": "BTC-USDT-SWAP",
                    "markPx": format!("{price:.1}"),
                    "ts": ts_ms.to_string()
                }]
            }),
        ),
    ];
    for (channel, symbol, payload) in updates {
        push_public_pair(frames, channel, symbol, payload.to_string(), recv_ts_ns);
    }
}

fn push_private_account(frames: &mut Vec<CaptureFrame>, index: usize, recv_ts_ns: &mut u64) {
    let ts_ms = 1_700_000_000_001_u64 + index as u64;
    let payload = json!({
        "arg": {"channel": "account"},
        "data": [{
            "uTime": ts_ms.to_string(),
            "mgnRatio": "20",
            "adjEq": "100000",
            "notionalUsd": "0",
            "details": [{
                "ccy": "USDT",
                "cashBal": "100000",
                "availBal": "100000"
            }]
        }]
    })
    .to_string();
    frames.push(CaptureFrame {
        source: Source::Private,
        envelope: envelope("private-main", Channel::Account, None, *recv_ts_ns, payload),
    });
    *recv_ts_ns += 1;
}

fn push_public_pair(
    frames: &mut Vec<CaptureFrame>,
    channel: Channel,
    symbol: &str,
    payload: String,
    recv_ts_ns: &mut u64,
) {
    for connection in ["public-primary", "public-redundant"] {
        frames.push(CaptureFrame {
            source: Source::Public,
            envelope: envelope(
                connection,
                channel.clone(),
                Some(symbol.to_string()),
                *recv_ts_ns,
                payload.clone(),
            ),
        });
        *recv_ts_ns += 1;
    }
}

fn envelope(
    connection: &str,
    channel: Channel,
    symbol: Option<String>,
    recv_ts_ns: u64,
    payload: String,
) -> RawEnvelope {
    RawEnvelope {
        venue: Venue::Okx,
        conn_id: ConnId::new(connection),
        channel,
        symbol,
        recv_ts_ns,
        raw_hash: payload_hash(payload.as_bytes()),
        payload,
    }
}
