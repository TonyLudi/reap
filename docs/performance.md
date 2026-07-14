# Performance Evidence

## Strategy-Only Baseline

The first profiling gate was run on 2026-07-10 before introducing pinned
threads, custom queues, CPU affinity, or specialized allocators.

### Workload

`cargo bench -p reap-engine --bench event_loop` sends 250,000 alternating spot
and perpetual depth events through:

```text
NormalizedEvent -> ChaosStrategy -> RiskGate -> accepted OrderIntent
```

The benchmark uses the checked-in `examples/iarb2-basic.toml`, release
optimization, normal Rust collections, and the same strategy and risk code used
by the engine. It excludes websocket IO, gateway HTTP latency, telemetry, and
storage.

### Environment

- CPU: 2 vCPU Arm Neoverse-N1, one thread per core
- Architecture: `aarch64`
- Rust: `rustc 1.95.0 (59807616e 2026-04-14)`
- Events per run: 250,000
- Intents produced per run: 999,996

### Result

Three consecutive runs measured:

| Run | Elapsed | Time/event |
| --- | ---: | ---: |
| 1 | 1,810.925 ms | 7,243.7 ns |
| 2 | 1,822.969 ms | 7,291.9 ns |
| 3 | 1,825.108 ms | 7,300.4 ns |

After the Chaos iarb2 parity work, the same benchmark was rerun on the same
host and toolchain shape:

| Run | Elapsed | Time/event |
| --- | ---: | ---: |
| 1 | 3,204.350 ms | 12,817.4 ns |
| 2 | 3,220.288 ms | 12,881.2 ns |
| 3 | 3,208.327 ms | 12,833.3 ns |

The parity implementation therefore costs about `12.84 us/event` in this
synthetic workload, roughly 76% above the scaffold baseline. The benchmark
emits nearly four intents per event and exercises the full quote, hedge, and
risk recalculation path; it is not a market-data-only latency measurement.

This is a regression baseline, not an exchange-to-exchange latency claim. The
next optimization gate should use production-shaped captures and sampling or
hardware counters to attribute allocation, strategy, reducer, and channel
costs separately.

### Decision

Keep Tokio at the IO edges, bounded channels at ownership handoffs, and the
single-writer event loop. The production-shaped follow-up profile and the
resulting collection changes are recorded below.

## Live Parity Profile

The live-path profiling gate was run on 2026-07-12 with:

```bash
cargo bench -p reap-live --bench live_loop
```

### Workload

The benchmark prebuilds a deterministic raw OKX workload outside the measured
interval and runs the same adapters, feed processor, coordinator, Chaos
strategy, and risk gate as `reap-live`:

- 50,204 raw frames across `BTC-USDT` and `BTC-USDT-SWAP`.
- Two 400-level book snapshots and 20,000 contiguous one-level-per-side book
  updates.
- A redundant copy of every public book, trade, funding, index, price-limit,
  and mark-price frame, exercising account-scoped/channel-aware deduplication.
- 5,000 logical trades, 80 logical pricing updates, and 40 private account
  updates.
- 70,208 feed outputs and 65,153 coordinator storage records.

The coordinator runs in observe mode. Strategy and pre-trade risk decisions are
executed, but order entry is disabled so a synthetic benchmark cannot create an
ever-growing set of pending REST orders.

The custom single-thread allocator counter records allocation calls and bytes
requested by `alloc`, `alloc_zeroed`, and `realloc`. Requested bytes are not
resident or peak memory.

### Attribution

The first attribution run used 50-level snapshots. It is retained here to show
why the coordinator/strategy path was changed, but it is not the final
acceptance workload:

| Stage | Time/unit | Allocation calls/unit | Requested bytes/unit |
| --- | ---: | ---: | ---: |
| Wire parse and raw record | 4,597.8 ns/raw frame | 50.81 | 3,966.0 B |
| Dedup, sequence, and book | 1,144.7 ns/raw frame | 5.39 | 1,388.1 B |
| Coordinator, strategy, and risk | 74,462.1 ns/feed output | 451.85 | 94,613.7 B |
| Complete live parity | 63,684.2 ns/raw frame | 688.46 | 136,756.4 B |

The dominant work was after normalization. Every accepted book update caused a
full quote recalculation for both its health heartbeat and its depth event, and
each recalculation built owned symbols for an entire hedge ladder before
retaining only the required levels.

The measured changes were:

- Reprice only when feed/system state changes; risk freshness still consumes
  every accepted heartbeat.
- Move the normalized event into the strategy view instead of cloning it, and
  scan feed health directly instead of collecting a temporary vector.
- Build hedge candidates with shared immutable symbol keys, retain owned values
  only for selected levels, and reuse one single-owner scratch vector across
  recalculations.
- Bound candidate generation by the current quote/delta hedge requirement and
  calculate theoretical quotes through immutable entity references instead of
  cloning the complete instrument state and order book.
- Consume owned OKX `data` values during typed adaptation instead of deep
  cloning each JSON value first.

No queue replacement, thread pinning, unsafe strategy code, or custom runtime
allocator was introduced.

The configured `Channel::Books` maps to OKX `books`, the standard 400-depth
incremental class in the [OKX API guide](https://www.okx.com/docs-v5/en/), so
the acceptance workload was raised from 50 to 400 levels per side. Before the
two depth-scaling changes above, that corrected workload measured:

| Stage | Time/unit | Allocation calls/unit | Requested bytes/unit |
| --- | ---: | ---: | ---: |
| Wire parse and raw record | 2,886.8 ns/raw frame | 33.33 | 3,030.5 B |
| Dedup, sequence, and book | 2,272.2 ns/raw frame | 6.19 | 10,950.0 B |
| Coordinator, strategy, and risk | 36,416.5 ns/feed output | 41.10 | 63,857.9 B |
| Complete live parity | 54,303.6 ns/raw frame | 97.03 | 103,333.7 B |

### Result

Three consecutive post-build runs on the same 2 vCPU Arm Neoverse-N1 host
measured:

| Run | Wire parse | Feed reduction | Coordinator | Complete parity |
| --- | ---: | ---: | ---: | ---: |
| 1 | 2,932.6 ns/raw | 2,449.3 ns/raw | 3,488.8 ns/output | 11,283.5 ns/raw |
| 2 | 2,940.1 ns/raw | 2,406.2 ns/raw | 3,490.3 ns/output | 11,251.8 ns/raw |
| 3 | 2,919.2 ns/raw | 2,395.8 ns/raw | 3,490.5 ns/output | 12,062.3 ns/raw |

The median complete-path result is `11.28 us/raw frame` on the 400-level
workload, with an observed range of `11.25-12.06 us`. Against the same 400-level
pre-change checkpoint, this is 4.8 times faster. Allocation calls fell from
`97.03` to `76.60` per raw frame (21.1%), and requested bytes fell from
`103,333.7` to `21,208.8` per raw frame (79.5%). The final 400-level path is also
faster than the original unoptimized 50-level attribution run, although those
different workloads are not used for a percentage comparison.

### Boundary

This benchmark includes raw-record cloning, JSON adaptation, redundant-input
deduplication, sequence/book reduction, strategy/risk evaluation, and storage
record construction. It excludes websocket/Tokio scheduling, channel enqueue
latency, storage serialization and disk IO, REST latency, and exchange
round-trip time. It is a regression and allocation gate, not a latency SLO or
capacity claim.

The local profiling gate is complete. Before production tuning, rerun this
benchmark with recorded target-market captures on the deployment host and add
end-to-end timestamping around the actual sockets and gateway. The immediate
trading-readiness blocker remains the credentialed OKX demo soak, not a queue or
runtime rewrite.

## Determinism Regression Gate

The research-verification determinism audit was profiled on 2026-07-14 against
pre-change commit `bacd132`. Stable quote and hedge traversal is precomputed at
strategy construction; the event loop retains hash lookups and does not sort or
allocate a risk-group symbol list on each book update.

An initial implementation allocated that temporary list on every recalculation.
The live benchmark exposed exactly 40,204 extra coordinator allocations, raising
the complete path from `76.56` to `77.36` allocation calls per raw frame. That
implementation was discarded. The final implementation matched the pre-change
`76.56` calls per raw frame and reduced requested bytes from `21,482.9` to
`21,444.5` per raw frame by retaining references in temporary quote state.

Three consecutive final live-path runs measured `11,627.2`, `11,620.2`, and
`11,684.7 ns/raw frame`; the median was `11.63 us`. Three warm strategy-only
runs measured `10,893.0`, `10,921.7`, and `10,882.6 ns/event`, each producing
999,996 intents. This is evidence against a local regression, not a target-host
latency or throughput certification.
