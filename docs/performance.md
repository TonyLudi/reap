# Performance Evidence

## Current Authoritative Regression Baselines

The completed Goal C handoff is the authoritative pre-Goal-D comparison
baseline. Measurements retained below are historical attribution evidence,
not the current regression numbers.

| Benchmark | Goal C median | Logical/allocation identity |
| --- | ---: | --- |
| Engine event loop | 11,058.7 ns/event | 250,000 events; 999,996 intents |
| Complete live parity | 17,082.6 ns/raw | 50,204 raw frames; 70,208 feed outputs; 65,130 storage records; zero actions |

The Goal C complete-live path made 4,193,771 allocation calls requesting
1,871,951,969 bytes per run.

## Goal D Phase 5 Local Final Measurements

The final Goal D local gate ran on 2026-07-20 on the same two-vCPU Arm
Neoverse-N1 host with `rustc 1.95.0 (59807616e 2026-04-14)`. Each benchmark
had one warm-up followed by five retained runs:

| Benchmark | Warm-up | Five recorded runs | Median | Phase 4 delta |
| --- | ---: | --- | ---: | ---: |
| Engine event loop (ns/event) | 11,972.2 | 12,298.8; 11,590.1; 11,618.3; 11,654.1; 11,674.7 | 11,654.1 | -0.97% |
| Complete live parity (ns/raw) | 17,783.1 | 17,198.2; 17,249.2; 17,929.8; 17,309.3; 17,361.3 | 17,309.3 | -0.17% |

Every engine run retained exactly 250,000 events and 999,996 intents. Every
live run retained exactly 50,204 raw frames, 70,208 feed outputs, 65,130
storage records, zero actions, 4,193,771 allocation calls, and
1,871,951,969 requested bytes. Runtime-health snapshot assembly and the
production `LiveRuntime` select loop are not part of either synthetic
benchmark. Their no-lock/no-allocation progress contract is source- and
state-transition-tested; it is not inferred from these elapsed times.

### Action-producing tail baseline

`reap-live/action_path` uses 10,000 warm-up and 100,000 post-warm-up
observations per workload. It retains every process-monotonic
`std::time::Instant` sample and computes exact nearest-rank percentiles with no
histogram, reservoir, interpolation, downsampling, drop, or overflow. Timing
and allocation use separate freshly initialized passes whose logical counters
must match. The table reports component-wise five-run medians and the highest
maximum retained across the five runs:

| Workload | p50 | p95 | p99 | p99.9 | Highest max |
| --- | ---: | ---: | ---: | ---: | ---: |
| Quote creation through prepared submit | 19,478 | 24,968 | 32,172 | 105,008 | 121,484,156 |
| Quote replacement through prepared cancel | 13,013 | 13,251 | 18,083 | 21,809 | 293,787 |
| IOC hedge through prepared submit | 24,771 | 28,454 | 36,381 | 66,296 | 59,094,222 |
| Risk rejection | 12,020 | 12,348 | 16,828 | 20,898 | 403,380 |
| Symbol fail-close through prepared cancel | 640 | 640 | 649 | 1,223 | 306,439 |
| Global fail-close through prepared cancels | 771 | 796 | 813 | 1,977 | 2,077,234 |
| Coordinator normalized/storage reduction | 5,596 | 5,703 | 6,137 | 12,825 | 1,445,697 |
| Raw gap recovery/action record | 26,403 | 29,432 | 36,938 | 57,804 | 6,273,554 |
| Public-trade implied-depth reprice | 12,078 | 12,390 | 17,001 | 22,202 | 6,489,787 |
| Bounded biased control/feed storm | 140 | 189 | 7,606 | 7,688 | 30,900 |

All values are nanoseconds. The storm's queue-age medians were
`11,922/16,066/16,713/20,964 ns` at p50/p95/p99/p99.9, with a highest
retained maximum of 47,244 ns. It processed exactly 20,000 control and 80,000
feed dequeues, observed 20,000 control preemptions, reached its capacity and
high-water mark of 80, and recorded 30,000 full-queue attempts. Its queues are
bench-private and do not claim to time the production runtime select loop.

The timer-read-overhead median remained `33/41/41/42 ns` at
p50/p95/p99/p99.9. All action runs had the same logical/allocation projection
SHA-256:

```text
aa7eaa6e9bb6727b4d52e6f1488591904c4d823a45e31e6829f23d938def4ce6
```

That is also the Phase 4 hash, so Phase 5 changed no action-workload counter or
allocation total. The full per-run distributions, counters, allocation bytes,
and regression deltas are recorded in the Goal D handoff.

The positive Phase 4-to-Phase-5 movements over 5% were quote p99
`+5.01%`, quote p99.9 `+19.04%`, hedge p99 `+7.39%`, and symbol fail-close
p99.9 `+7.94%` (90 ns). The timed workload bodies did not change, p50 stayed
within 1.68%, no p95 regression exceeded 3.19%, logical/allocation work was
exact, unrelated tails moved in both directions, and timer maxima showed host
interruptions. The retained results therefore support a shared-host
tail-variance classification, not a Phase 5 hot-path regression or target-host
claim.

### Prepared-request serialization

Adapter-private serialization is measured separately so serializers and
authority constructors remain private. Each workload again used 10,000
warm-up and 100,000 timed observations, exact nearest-rank percentiles, no
dropped samples, and a separate allocation pass:

| Workload | p50 | p95 | p99 | p99.9 | Highest max | Calls/bytes per action |
| --- | ---: | ---: | ---: | ---: | ---: | ---: |
| Prepared submit to REST-shaped inner body | 665 | 673 | 681 | 1,469 | 1,293,914 | 6 / 429 |
| Prepared submit to websocket order request | 2,248 | 2,355 | 2,626 | 7,130 | 1,054,436 | 24 / 1,550 |

The exact serializer logical/allocation projection SHA-256 was
`1d4633a2d2634573a2d3cc790ed67b49427ef530ff3b115126e26e5199f3a0bb`
in all five runs. Relative to Phase 4, REST p50/p99/p99.9 moved
`+2.62%/+1.19%/+33.67%`; websocket moved
`+1.86%/+16.40%/+5.33%`. The measured adapter source and reachable
serializer body did not change in Phase 5, while central percentiles and exact
work remained stable. These upper-tail observations are retained as
shared-host scheduling/code-layout variance rather than discarded or presented
as production latency.

### Included and excluded boundaries

The quote, replacement, and hedge rows include the production Chaos strategy,
engine/risk decisions, typed intent traversal, policy authorization, same-turn
reservation/ownership, gateway idempotency, and preparation. Risk rejection
ends at the engine/risk boundary. Coordinator reduction includes canonical
storage-record construction; raw recovery additionally includes actual
credential-free OKX parsing and feed reduction. Public-trade reprice includes
the private monotonic 100-microsecond schedule and due service. The storm is a
bench-only bounded-channel harness. The serializer rows begin with an already
prepared submit and include the actual adapter-private REST-shaped or
websocket serializer.

No one row is an end-to-end exchange measurement. Depending on the row,
excluded stages include socket receive, production Tokio/channel scheduling,
the production select loop, storage enqueue and disk IO, signing, transport
queues, network IO, and exchange acknowledgement. The machine-readable
schema-version-1 output carries the exact row-specific boundary.

## Goal F Phase 6 Local PM And Chaos Evidence

Goal F Phase 6 is green on local structural and regression evidence. The PM
artifacts were built from
`e11d51bfebe157b31d6f6d8ee8a8c4981f6c8768` on a two-vCPU Arm Neoverse-N1
host running Linux `7.0.0-1004-aws`, Rust
`1.95.0 (59807616e 2026-04-14)`, and LLVM `22.1.2`. The Phase 6 acceptance
commit `c76ccbd22cb4d25e121ac5bb3bc9b6dcd9e16f47` modifies only a test
allowlist.

### PM replay identity and bounds

The combined replay gate uses the real filesystem writer for one complete
nominal artifact and performs two independent read-only recoveries:

| Evidence boundary | Exact accepted result |
| --- | --- |
| Artifact | 35,012 lines; 22,791,589 bytes; SHA-256 `83ced509c9ea180e66d957853f9ff7762ef3c0babc316c9251c12d4d1a5224eb` |
| Independent recovery | Byte-identical projections; both canonical SHA-256 `f98bf8a88f34fb6e3c4dcfd1919a2c1d4577b2da3960375e216e596d0746cd35` |
| Recovery memory | 2,959,343 peak working bytes, below the fixed 16-MiB bound |
| Recovered terminal state | 35,012 records; last sequence 35,011; zero retained owned orders, fill keys, or unresolved orders |
| Action determinism | Journal SHA-256 `389887a2d044867c6ad1f7b7b9ad52aa58c792864846fc42f220759fac111b85`; logical SHA-256 `4931af3e39ee291db82ba40da7a5e73473431801606565b5ad625c69beb70475` |
| Parser identity | Fixture SHA-256 `985332384ae2e7b2535c0fa2c214b40862997b0f80c450be87ac108fff9b550b`; projection SHA-256 `588e14caac0d5a38c94f9ee121b0238f084a4e2c57dbcd1c7f8f5f052210e885` |
| Authorization | `production_order_entry_authorized = false` |

The PM action-path gate used one discarded warm-up and three recorded runs.
Each run retained 15,000 exact nearest-rank latency samples:

| Recorded run | p50 | p95 | p99 | p99.9 | max |
| ---: | ---: | ---: | ---: | ---: | ---: |
| 1 | 22,909 ns | 42,173 ns | 53,660 ns | 75,625 ns | 123,091 ns |
| 2 | 23,524 ns | 44,463 ns | 56,614 ns | 83,067 ns | 175,758 ns |
| 3 | 23,467 ns | 43,995 ns | 55,302 ns | 73,943 ns | 158,003 ns |

Every run was below the frozen local regression bounds of `25,000 ns` at p50
and `250,000 ns` at p99.9. The normalized owner interval requested exactly
zero allocation calls and zero allocated bytes. It used 58,858,352 reserved
bytes, below its fixed 64-MiB bound. All five repeated passes returned owned
orders, fill identities, queues, schedules, and other terminal cardinalities
to zero. Queue high-water was at most one, with zero drops and zero saturation.

Every recorded run also retained exactly:

- 100,000 external observations;
- 20,010 internal fact acknowledgements and 120,010 owner reductions;
- 35,010 measured mutation records;
- 15,000 quote evaluations, 10,000 quote intents, and 5,000 cancel intents;
- 5,000 unique fills, 10,000 suppressed duplicate fill rows, and ten watermark
  advances; and
- 27,309 overload attempts: 14,633 through nine product-reached rows and
  12,676 through four sealed mechanism-capacity rows.

The same journal, logical, parser-fixture, and parser-projection hashes were
present in every run. `production_order_entry_authorized` remained `false`.

### Post-Phase 6 Chaos regression

The Chaos regression campaign used one warm-up followed by three recorded
runs from `29aefe3348a406da9b00154d61413722b11882ee`:

| Benchmark/component | Recorded median | Phase 0 median | Delta |
| --- | ---: | ---: | ---: |
| Engine event loop | 11,827.0 ns/event | 11,783.5 ns/event | +0.37% |
| Live wire parse/raw record | 2,923.5 ns/unit | 2,948.6 ns/unit | -0.85% |
| Live dedup/sequence/book | 7,723.4 ns/unit | 7,712.6 ns/unit | +0.14% |
| Live coordinator/strategy/risk/storage | 4,111.2 ns/unit | 4,137.7 ns/unit | -0.64% |
| Complete live parity | 17,427.5 ns/unit | 17,635.5 ns/unit | -1.18% |

Every engine run retained exactly 250,000 events and 999,996 intents. Every
live run retained exactly 50,204 parsed frames, 70,208 feed outputs, 65,130
storage records, and zero actions. Allocation calls/requested bytes were exact
in every run: wire `1,673,504 / 158,570,992`; dedup
`670,868 / 1,349,274,641`; coordinator
`1,849,399 / 364,106,336`; and complete parity
`4,193,771 / 1,871,951,969`.

An initially noisy coordinator tail was resolved with a same-host, same-toolchain
comparison: five warm recorded Phase 0 runs from
`8d6581270b82f39293ccdb0cbeaead42d717e81c`, immediately followed by five
warm recorded current runs. The final current results were:

| Action workload | Current p50 | p50 delta | Current p99.9 | p99.9 delta | Exact allocation calls / requested bytes |
| --- | ---: | ---: | ---: | ---: | ---: |
| Quote creation | 19,175 ns | -0.64% | 89,155 ns | -30.60% | 19,733,342 / 836,475,432 |
| Quote replacement | 12,922 ns | -0.45% | 21,571 ns | -49.34% | 16,600,000 / 773,500,000 |
| IOC hedge | 24,295 ns | -2.63% | 62,899 ns | -69.92% | 23,433,342 / 1,243,175,432 |
| Risk rejection | 11,856 ns | -0.96% | 21,308 ns | -35.04% | 15,300,000 / 622,400,000 |
| Symbol fail-close | 664 ns | +3.75% | 1,509 ns | -49.62% | 1,300,000 / 46,000,000 |
| Global fail-close | 788 ns | +2.20% | 4,283 ns | +8.76% | 1,600,000 / 52,600,000 |
| Coordinator reduction | 5,546 ns | -1.18% | 12,644 ns | -1.53% | 3,900,000 / 158,800,000 |
| Raw recovery | 26,223 ns | +0.25% | 84,437 ns | -9.74% | 21,980,027 / 1,777,312,151 |
| Trade reprice | 11,905 ns | -0.48% | 21,751 ns | +1.73% | 15,000,264 / 615,207,392 |
| Bounded storm | 140 ns | 0.00% | 7,713 ns | 0.00% | 0 / 0 |

Every action run preserved its exact logical counters. No median or supported
tail regressed by more than 10%; the largest positive comparison was global
fail-close p99.9 at `+8.76%`.

### Evidence exclusions

The PM action timer covers the normalized single-owner path through sealed
durable-ack evidence and fake prepared quote or cancel effects. Parser work is
measured separately; filesystem serialization and `fsync` are untimed. The
combined replay gate exercises local file writing and recovery, not a live PM
endpoint or account. Neither PM target includes credentials, request signing,
production network IO, exchange service time, acknowledgement round trips,
real fills, settlement, or a production quote model.

The Chaos rows retain the row-specific synthetic boundaries described above
and do not become PM evidence. The overload tests prove fixed local mechanism
bounds; they are not throughput or deployment-capacity measurements. These
Phase 6 results establish local structural identity, determinism, allocation
bounds, and regression control only. They are not a target-host measurement,
latency SLO, capacity certification, economic validation, production-readiness
claim, or authorization to enter orders.

## Missing Target-Host Decision-To-Wire Evidence

These local results are reproducible regression and allocation evidence, not a
latency SLO, capacity certification, production-readiness claim, or
colocated-HFT claim. Reap still lacks target-host, production-shaped evidence
across actual socket receipt, queueing, decision, gateway preparation,
serialization, network transmission, and exchange acknowledgement. Any
pinned-thread, SPSC/ring-buffer, allocator, kernel-bypass, or custom-runtime
decision remains conditional on that evidence.

## Historical Measurements (Non-Authoritative)

### Strategy-Only Baseline

The first profiling gate was run on 2026-07-10 before introducing pinned
threads, custom queues, CPU affinity, or specialized allocators.

#### Workload

`cargo bench -p reap-engine --bench event_loop` sends 250,000 alternating spot
and perpetual depth events through:

```text
NormalizedEvent -> ChaosStrategy -> RiskGate -> accepted OrderIntent
```

The benchmark uses the checked-in `examples/iarb2-basic.toml`, release
optimization, normal Rust collections, and the same strategy and risk code used
by the engine. It excludes websocket IO, gateway HTTP latency, telemetry, and
storage.

#### Environment

- CPU: 2 vCPU Arm Neoverse-N1, one thread per core
- Architecture: `aarch64`
- Rust: `rustc 1.95.0 (59807616e 2026-04-14)`
- Events per run: 250,000
- Intents produced per run: 999,996

#### Result

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

#### Decision

Keep Tokio at the IO edges, bounded channels at ownership handoffs, and the
single-writer event loop. The production-shaped follow-up profile and the
resulting collection changes are recorded below.

### Live Parity Profile

The live-path profiling gate was run on 2026-07-12 with:

```bash
cargo bench -p reap-live --bench live_loop
```

#### Workload

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

#### Attribution

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

#### Result

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

#### Boundary

This benchmark includes raw-record cloning, JSON adaptation, redundant-input
deduplication, sequence/book reduction, strategy/risk evaluation, and storage
record construction. It excludes websocket/Tokio scheduling, channel enqueue
latency, storage serialization and disk IO, REST latency, and exchange
round-trip time. It is a regression and allocation gate, not a latency SLO or
capacity claim.

At that profiling gate, the next evidence step was to rerun the benchmark with
recorded target-market captures on the deployment host and add end-to-end
timestamping around the actual sockets and gateway. The current operational
blockers and runtime decision threshold are stated in the authoritative
sections above.

### Determinism Regression Gate

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
