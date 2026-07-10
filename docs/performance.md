# Hot-Path Baseline

The first profiling gate was run on 2026-07-10 before introducing pinned
threads, custom queues, CPU affinity, or specialized allocators.

## Workload

`cargo bench -p reap-engine --bench event_loop` sends 250,000 alternating spot
and perpetual depth events through:

```text
NormalizedEvent -> ChaosStrategy -> RiskGate -> accepted OrderIntent
```

The benchmark uses the checked-in `examples/iarb2-basic.toml`, release
optimization, normal Rust collections, and the same strategy and risk code used
by the engine. It excludes websocket IO, gateway HTTP latency, telemetry, and
storage.

## Environment

- CPU: 2 vCPU Arm Neoverse-N1, one thread per core
- Architecture: `aarch64`
- Rust: `rustc 1.95.0 (59807616e 2026-04-14)`
- Events per run: 250,000
- Intents produced per run: 999,996

## Result

Three consecutive runs measured:

| Run | Elapsed | Time/event |
| --- | ---: | ---: |
| 1 | 1,810.925 ms | 7,243.7 ns |
| 2 | 1,822.969 ms | 7,291.9 ns |
| 3 | 1,825.108 ms | 7,300.4 ns |

This is a regression baseline, not an exchange-to-exchange latency claim. The
next optimization gate should use production-shaped captures and sampling or
hardware counters to attribute allocation, strategy, reducer, and channel
costs separately.

## Decision

Keep Tokio at the IO edges, bounded channels at ownership handoffs, and the
single-writer event loop. There is no evidence yet that custom SPSC queues,
pinned threads, or lower-level IO would improve the dominant cost. Re-run this
benchmark and collect profiles before making those changes.
