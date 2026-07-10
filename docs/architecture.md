# Reap Architecture

This document describes the target architecture for `reap` as a Rust trading
system that can run the same strategy logic in live trading and backtest.

The workspace now implements the migration baseline described here: strategy
and backtest parity, live OKX feed/order boundaries, deterministic risk,
telemetry, and durable capture. Exchange certification and deployment-specific
orchestration remain operational responsibilities rather than strategy code.

## Goals

- Replicate the important `imm-strategy/chaos` decision logic in Rust.
- Keep strategy behavior deterministic and replayable.
- Support live exchange connectivity with multiple websocket connections.
- Handle duplicated, out-of-order, stale, and gapped exchange streams.
- Keep the hot path lock-free or single-writer wherever practical.
- Preserve backtest/live parity by using the same normalized events.
- Make failures explicit: feed gap, stale private stream, rejected order,
  missed cancel, risk breach, reconciliation drift.

## Non-Goals

- A generic trading framework for arbitrary strategies.
- A full actor framework in the strategy hot path.
- Shared mutable order book or portfolio state behind `Arc<Mutex<_>>`.
- Exact reproduction of Spring, Redis control plane, Luban bootstrapping, or
  Java infrastructure.
- Premature thread-per-core/io_uring optimization before profiling.

## Runtime Model

Modern HFT systems are event-loop/state-machine systems, but usually not one
large async event loop. `reap` should use async IO at the edges and deterministic
single-writer loops for trading state.

```text
exchange websocket tasks
  -> raw parsers
  -> exchange adapters
  -> dedup + sequencing + gap recovery
  -> book/order reducers
  -> strategy shard event loop
  -> risk gate
  -> order gateway
  -> exchange order API
```

The strategy loop should not perform network IO, file IO, blocking logging, or
unbounded allocation. It receives normalized events, updates owned state, emits
order intents, and returns.

Recommended initial stack:

- `tokio` for websocket/REST IO, timers, supervision, and async channels.
- `tokio-tungstenite` for live websocket clients.
- `reqwest` or venue-specific clients for REST snapshots and reconciliation.
- `tracing` for structured logs and spans.
- `serde` for config and replay formats.
- `rtrb` or bounded channels later if profiling shows `tokio::mpsc` is too
  expensive for a specific handoff.

## Workspace Layout

Target structure:

```text
reap/
  crates/
    reap-core/
    reap-venue/
    reap-feed/
    reap-book/
    reap-order/
    reap-risk/
    reap-strategy/
    reap-engine/
    reap-backtest/
    reap-storage/
    reap-telemetry/
    reap-cli/
```

### `reap-core`

Shared primitive types and pure utilities.

Responsibilities:

- Symbols, venue ids, account ids, strategy ids.
- Time types: exchange timestamp, receive timestamp, monotonic timestamp.
- Side, order type, time-in-force, liquidity, order status.
- Normalized market events and private events.
- Price/quantity helpers, tick/lot rounding.
- Small fixed-capacity containers if needed by hot path.

Example types:

```rust
pub struct EventClock {
    pub exch_ts_ns: Option<i64>,
    pub recv_ts_ns: i64,
    pub seq: Option<u64>,
}

pub enum NormalizedEvent {
    BookDelta(BookDelta),
    BookSnapshot(BookSnapshot),
    Trade(Trade),
    OrderUpdate(OrderUpdate),
    Fill(Fill),
    Account(AccountUpdate),
    Timer(TimerEvent),
}
```

### `reap-venue`

Venue abstraction and exchange-specific adapters.

Responsibilities:

- Venue trait definitions.
- Raw websocket message envelopes.
- Exchange parser modules such as OKX, Binance, Hyperliquid, etc.
- Normalization from exchange payloads into `reap-core` events.
- Venue-specific sequence, checksum, and channel semantics.
- Venue-specific order request signing and response parsing.

Suggested structure:

```text
reap-venue/
  src/
    lib.rs
    traits.rs
    okx/
      ws.rs
      rest.rs
      parse.rs
      order.rs
    binance/
      ws.rs
      rest.rs
      parse.rs
      order.rs
```

### `reap-feed`

Live market/private data connectivity.

Responsibilities:

- Websocket connection lifecycle.
- Multi-websocket subscription partitioning.
- Redundant websocket support for critical channels.
- Ping/pong, reconnect, exponential backoff, and stale-feed detection.
- Raw message timestamping.
- Deduplication.
- Sequence checking and gap detection.
- Snapshot fetch and buffered delta replay.
- Channel-level health metrics.

Key design point: `reap-feed` emits normalized events only after dedup and
sequence handling. The strategy should not know which websocket produced an
event.

Suggested modules:

```text
reap-feed/
  src/
    connection.rs      # socket lifecycle
    supervisor.rs      # restart and health policy
    subscription.rs    # channel/symbol partitioning
    raw.rs             # raw envelope with conn_id/recv_ts
    dedup.rs           # bounded recent-id cache
    sequence.rs        # update-id and gap logic
    snapshot.rs        # REST snapshot recovery
    mux.rs             # active-active or active-passive feed merge
```

### `reap-book`

Single-writer book reducers.

Responsibilities:

- L1/L2/L3 book state.
- Snapshot plus delta application.
- Checksum validation where supported.
- Top-of-book and depth-at-size queries.
- Own-order exclusion if needed by strategy pricing.
- Book quality state: ready, stale, gapped, recovering.

This crate should be usable in both live and backtest.

### `reap-order`

Order command routing and canonical order state.

Responsibilities:

- Client order id generation.
- Idempotent new/cancel/replace handling.
- Per-venue/account order gateway.
- Rate limiting and request pacing.
- Canonical order state reducer.
- REST reconciliation for open orders, fills, balances, and positions.
- Missed cancel and unknown-order handling.

Strategy code sends intents. The order layer owns what actually happened.

```text
OrderIntent -> RiskGate -> OrderCommand -> VenueGateway
VenueGateway -> RawAck/RawUpdate -> OrderReducer -> OrderEvent
```

### `reap-risk`

Pre-trade and post-trade risk.

Responsibilities:

- Static config sanity checks.
- Notional, delta, live-order, turnover, and drawdown limits.
- Kill switch and manual symbol halt.
- Per-risk-group quote permission.
- Reject or clip unsafe order intents before they hit the gateway.
- Post-fill exposure checks.

Risk should be deterministic and replayable. Live-only control inputs should be
modeled as normal events.

### `reap-strategy`

Pure strategy logic.

Responsibilities:

- Chaos/iarb2 risk groups.
- Hedge ladder construction.
- Theo price and quote quantity calculation.
- Quote replacement decisions.
- Delta hedge target selection.
- Strategy state snapshots for replay debugging.

The strategy API should stay small:

```rust
pub trait Strategy {
    fn on_event(&mut self, event: &StrategyEvent) -> Vec<OrderIntent>;
}
```

No websocket clients, REST clients, async tasks, filesystem writes, or global
state should live here.

### `reap-engine`

Live orchestration.

Responsibilities:

- Wire feeds, book reducers, strategy, risk, order gateway, storage, telemetry.
- Own task topology.
- Own bounded channels and backpressure policy.
- Start/stop/restart strategy instances.
- Dispatch timers into strategy loops.
- Coordinate reconciliation and recovery.

Recommended first topology:

```text
tokio runtime
  feed tasks per websocket
  order gateway tasks per venue/account
  telemetry/storage tasks
  one strategy shard task per strategy instance
```

Strategy shard task:

```rust
loop {
    let event = rx.recv().await?;
    let intents = strategy.on_event(&event);
    let commands = risk.check(intents, &state);
    order_tx.send(commands).await?;
}
```

Later, if the strategy loop becomes latency-critical, replace the async receive
with a pinned OS thread and a bounded SPSC queue.

### `reap-backtest`

Replay and simulation.

Responsibilities:

- Load normalized events or raw captures.
- Feed the same book/order/strategy reducers as live.
- Simulate matching, queue position, latency, slippage, and fees.
- Produce deterministic reports.
- Compare strategy outputs across commits.

Backtest should use the same `StrategyEvent` and `OrderEvent` types as live.

### `reap-storage`

Durable capture and replay data.

Responsibilities:

- Raw websocket capture.
- Normalized event logs.
- Strategy decisions and order intents.
- Order/fill/account events.
- Book snapshots.
- JSONL initially, Parquet or binary logs later.

Storage must never block the hot path. Use bounded queues and explicit drop or
degrade policies.

### `reap-telemetry`

Observability.

Responsibilities:

- Metrics: feed lag, duplicate rate, gap count, reconnect count, book age,
  command latency, ack latency, fill latency, queue depth.
- Structured logs.
- Health endpoints.
- Panic and task death reporting.

Metrics should be cheap to emit and safe to drop under pressure.

### `reap-cli`

User-facing binaries.

Target command surface:

```text
reap live --config config/live.toml
reap backtest --config config/backtest.toml --events data/events.jsonl
reap replay-check --events data/events.jsonl
reap inspect-book --capture raw/ws.jsonl --symbol BTC-USDT
reap config-check --config config/live.toml
```

`backtest`, `replay-check`, and `config-check` are implemented. `live` and
`inspect-book` remain planned; see [trading-readiness.md](trading-readiness.md).

## Multi-Websocket Design

Each exchange adapter should describe its channels:

```rust
pub struct Subscription {
    pub venue: Venue,
    pub channel: Channel,
    pub symbol: Option<Symbol>,
    pub priority: FeedPriority,
}
```

The feed supervisor partitions subscriptions into socket groups:

- Public L2 depth sockets grouped by venue and symbol count.
- Public trades possibly separated from depth if trade bursts cause delays.
- Private order/account sockets isolated from public data.
- Redundant sockets for critical symbols where the venue allows it.

Every raw message gets an envelope:

```rust
pub struct RawEnvelope {
    pub venue: Venue,
    pub conn_id: ConnId,
    pub channel: Channel,
    pub symbol: Option<Symbol>,
    pub recv_ts_ns: i64,
    pub raw_hash: u64,
    pub payload: Bytes,
}
```

The adapter parses raw payloads into normalized candidate events. The dedup and
sequence layer decides whether each candidate is new, duplicate, stale, gapped,
or requires recovery.

## Deduplication

Dedup must be channel-aware. There is no universal exchange id that works for
all events.

```rust
pub struct EventId {
    pub venue: Venue,
    pub channel: Channel,
    pub symbol: Option<Symbol>,
    pub exchange_seq: Option<u64>,
    pub exchange_event_ts: Option<i64>,
    pub trade_id: Option<String>,
    pub order_id: Option<String>,
    pub fill_id: Option<String>,
    pub raw_hash: u64,
}
```

Dedup rules:

- Book deltas: prefer sequence or update id.
- Trades: prefer trade id.
- Private fills: prefer execution id or fill id.
- Order updates: reduce idempotently by order id plus venue update version.
- BBO/ticker fallback: hash `(symbol, exchange_ts, bid, bid_qty, ask, ask_qty)`.
- Final fallback: bounded `(channel, symbol, raw_hash)` cache.

Dedup state should be bounded:

```text
per channel-symbol:
  recent sequence range
  recent event-id ring
  last accepted exchange timestamp
  duplicate counters
```

Duplicates are normal during reconnects and redundant websockets. They should be
counted, not treated as errors unless the rate indicates a bad subscription.

## Sequence And Gap Recovery

For every channel with sequence semantics:

```text
ready:
  if next_seq == expected:
    apply
  if next_seq <= last_seq:
    duplicate_or_stale
  if next_seq > expected:
    enter recovering

recovering:
  buffer deltas
  fetch snapshot
  apply snapshot
  replay buffered deltas after snapshot sequence
  if contiguous:
    ready
  else:
    refetch or restart socket
```

Book state should expose quality:

```rust
pub enum BookStatus {
    Empty,
    Recovering,
    Ready,
    Stale,
    Gapped,
}
```

Strategy quote generation should require `Ready` books. Risk can choose whether
to allow hedging on stale books, but the default should be no.

## Hot Path Rules

For code that runs on every market event:

- Prefer single ownership over locks.
- Use bounded queues.
- Avoid unbounded `Vec` growth.
- Avoid blocking logs, blocking file writes, and DNS/REST calls.
- Avoid hidden allocations in tight loops once profiling identifies them.
- Keep event handlers short and measurable.
- Treat every `.await` in a trading loop as a design decision.
- Keep slow diagnostics on side channels.

Acceptable first implementation:

- Tokio tasks for IO and routing.
- Owned strategy state in one task.
- Bounded `tokio::mpsc`.
- Normal Rust collections.

Optimize only after measurement:

- Pinned threads for strategy/order loops.
- `rtrb` or custom SPSC queues.
- Preallocated books and command buffers.
- Custom allocators.
- CPU affinity and isolated cores.

## Backpressure Policy

Backpressure must be explicit per channel.

Recommended defaults:

- Market data into book reducer: bounded, drop only if the book will enter
  recovery immediately.
- Book snapshots into strategy: coalesce by symbol if strategy only needs latest
  BBO/depth summary.
- Private order/fill events: never drop. If queue is full, fail closed and halt.
- Telemetry/logging: bounded and droppable.
- Storage: bounded with loss counters, except compliance-critical records.

## Failure Modes

Core failures should produce typed events:

```text
FeedStale
FeedGap
BookRecoveryStarted
BookRecoveryFailed
PrivateStreamStale
OrderReject
CancelReject
UnknownOrder
ReconcileDrift
RiskBreach
KillSwitch
```

The engine can route these into strategy/risk as normal events. This keeps live
behavior replayable.

## Testing Strategy

Test layers:

- Parser golden tests using captured raw exchange messages.
- Dedup tests with duplicated messages from multiple connections.
- Sequence tests with stale, out-of-order, and gapped deltas.
- Snapshot recovery tests with buffered delta replay.
- Book reducer tests against expected top levels and checksums.
- Order reducer idempotency tests.
- Strategy deterministic replay tests.
- Backtest/live parity tests using normalized event logs.
- Soak tests with reconnect loops and synthetic message bursts.

Useful fixtures:

```text
fixtures/
  raw/okx/depth-gap.jsonl
  normalized/chaos_quote_hedge.jsonl
```

## Migration Plan

### Phase 1: Documented Workspace Split

Status: implemented as `reap-core`, `reap-strategy`, `reap-backtest`, and
`reap-cli`.

- Convert current single crate into a Cargo workspace.
- Move common types to `reap-core`.
- Move current strategy to `reap-strategy`.
- Move current backtest/matcher to `reap-backtest`.
- Keep `reap-cli` behavior unchanged.

### Phase 2: Normalized Event Boundary

Status: implemented for the current strategy/backtest scaffold with
`NormalizedEvent`, `StrategyEvent`, and `OrderIntent` in `reap-core`.

- Introduce `NormalizedEvent`, `StrategyEvent`, and `OrderIntent`.
- Make backtest feed normalized events into strategy.
- Remove direct coupling between strategy and current matcher structs.

### Phase 3: Book And Order Reducers

Status: implemented as `reap-book` and `reap-order`, with `reap-backtest`
using the shared reducers for book state, liquidity taking, and canonical order
state transitions.

- Add `reap-book` with single-writer book state.
- Add `reap-order` with canonical order state reducer.
- Update backtest to use these reducers.

### Phase 4: Live Feed Skeleton

Status: implemented as `reap-venue` and `reap-feed`, with OKX books/trades,
supervised socket plans, bounded deduplication, sequence recovery, and the raw
capture checker.

- Add `reap-venue` and `reap-feed`.
- Implement one venue public depth/trade adapter.
- Implement reconnect, dedup, sequence checks, and snapshot recovery.
- Add `reap replay-check` for raw captures.

### Phase 5: Live Order Gateway

Status: implemented for OKX REST and private websocket contracts, including
signing, submit/cancel, canonical private reduction, idempotency, pacing, and
pending-order/fill reconciliation.

- Implement signed order submit/cancel for one venue.
- Add private websocket order/fill reducer.
- Add reconciliation and idempotency.

### Phase 6: Runtime Safeguards

Status: implemented as `reap-risk`, `reap-engine`, `reap-telemetry`, and
`reap-storage`, with configuration validation, an operations guide, and a
measured event-loop benchmark. These are reusable components, not a completed
production deployment.

- Add risk gates, kill switch, stale feed policy, metrics, storage.
- Add operational runbooks and config checks.
- Profile hot path before low-level optimization.

### Phase 7: Chaos Iarb2 Decision Parity

Status: implemented for the documented OKX decision boundary, including
explicit rejection of one-symbol/self-only hedge topologies. Exact Java quote
optimizer order churn and non-OKX platform behavior remain outside this
boundary.

- Cross-check configuration, quote, hedge, account risk, and stop behavior.
- Cover Java fixture vectors and edge-case timing in deterministic tests.
- Normalize all pricing, account, position, order, and fill inputs needed by the
  strategy.

### Phase 8: Demo-Tradable Composition

Status: planned. This is the current deployment blocker.

- Add the live composition process and single-writer event-loop owner.
- Implement executable startup, readiness, reconciliation, and restart gates.
- Verify instrument metadata, account mode, and risk valuation.
- Complete fault injection and OKX demo soak acceptance.

See [trading-readiness.md](trading-readiness.md) for the detailed gate.

## Architectural Decisions

### Use Tokio At Edges

Tokio is the default runtime for exchange connectivity. It is mature and good
for websocket/REST IO. The strategy hot path should remain a small deterministic
loop and should not depend on arbitrary async task scheduling.

### Own State In One Place

Order books, order state, and strategy state should each have one owner. Other
tasks send events or commands. This avoids lock contention and makes replay
behavior easier to reason about.

### Normalize Before Strategy

Exchange adapters absorb venue differences. Strategy logic should see internal
events only. This keeps strategy tests small and protects the strategy from
exchange-specific message quirks.

### Same Events For Live And Backtest

Backtest should not call strategy-only shortcuts. It should feed the same event
types that live trading uses. This is the main guardrail against backtest/live
drift.

### Fail Closed On Private State Drift

If private order/fill/account streams are stale or reconciliation detects drift,
the default behavior should halt quoting and reconcile. Missing private events
are more dangerous than missing public ticks.
