# Reap Architecture

This document describes the target architecture for `reap` as a Rust trading
system that can run the same strategy logic in live trading and backtest.

The workspace now implements the migration baseline and a demo-capable OKX
composition root: strategy/backtest parity, live feed/order boundaries,
deterministic risk, executable readiness/restart state, telemetry, and durable
capture. Exchange certification and production deployment controls remain
operational gates rather than strategy code.

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
    reap-capture/
    reap-storage/
    reap-telemetry/
    reap-live/
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
- Separate per-socket transport liveness from aggregate private-state health.
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
- Typed one-to-one exchange/client order-ID binding from REST acknowledgements,
  with contradictory journal history rejected and active bindings restored
  before startup REST reduction; private order/fill symbol ownership and
  immutable symbol/side checks occur before canonical state mutation.
- Semantic private-order deduplication treats repeated fill IDs and unchanged
  terminal order states as duplicates independently of exchange update time.
- REST reconciliation for open orders, fills, balances, and positions.
- Authoritative account-snapshot replacement with zero tombstones for balances
  and positions omitted after closure, plus per-row monotonic update guards.
- Account-scoped fill convergence: derivative fills await their position row;
  spot fills await both base and quote balance rows; expiry fails closed and
  requests full reconciliation.
- Typed position margin mode from both websocket and REST inputs, checked
  against each configured derivative trade mode before nonzero state is
  accepted and included in full-state reconciliation.
- Strict position ownership at the live boundary: spot routing is cash-only,
  and a nonzero position must be a configured derivative owned by the account
  that delivered it. Unmodeled exposure is never passed into strategy/risk.
- Typed per-currency forced-repayment indicator from account websocket and REST
  state, checked against the live risk threshold before account state is
  accepted and compared during reconciliation.
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
- Notional, delta, global/per-symbol active-order count, live-order notional,
  rolling exchange-rejection and unfilled-IOC cancellation, turnover, and
  drawdown limits.
- Global kill switch and symbol isolation controls.
- Freshness, integrity, and downside-depeg guards for configured stablecoin/USD
  references.
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

Deterministic strategy/risk enforcement used by both composition tests and the
live owner.

### `reap-live`

Live composition and lifecycle ownership.

Responsibilities:

- Wire feeds, book reducers, strategy, risk, order gateway, storage, telemetry.
- Own task topology.
- Own bounded channels and backpressure policy.
- Start/stop/restart strategy instances.
- Dispatch timers into strategy loops.
- Coordinate reconciliation and recovery.
- Reduce durable global/account/symbol latches, block halted routes, and
  guarantee scoped canonical cancellation.
- Promote terminal strategy safety halts into the global risk gate before
  dispatching intents from the triggering event; global cancellation always
  takes precedence over simultaneous symbol isolation.
- Validate exchange time, expire stale place requests at the venue, and own an
  independently scheduled exchange deadman lifecycle per account.
- Expose a separate minimal-config emergency composition that bypasses strategy,
  journal, websocket, and operator dependencies while cancelling and verifying
  the venue's regular order book account-wide.

Implemented topology:

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
    let input = prioritized_rx.recv().await?;
    let output = coordinator.on_input(input)?;
    critical_storage.record_bounded(output.normal_records)?;
    critical_storage.record_durable(output.safety_latches).await?;
    order_queues.try_dispatch_all(output.actions)?;
}
```

The coordinator owns strategy, risk, readiness, account-scoped private
reducers, client-order-id generation, and intent routing. It synchronously
records `PendingNew` before a submit action can reach an account gateway task.
After every strategy callback, the engine polls the generic terminal safety
state. A newly reported halt becomes `RiskBreach` plus `KillSwitchActivated`,
rejects new intents from that callback, persists through the coordinator's
global safety latch, and synthesizes cancellation for every canonical order.
The runtime separately tracks submit-to-private-state and
cancel-to-terminal-state convergence. A missed event-only order update cannot
leave either transition pending indefinitely: expiry releases cancel
deduplication, blocks the account, retries cancellation, and starts full REST
reconciliation.
REST and websocket IO never borrow strategy state across `.await`.
Normal event records use the bounded non-blocking path. A safety-latch mutation
is the rare write-ahead exception: it is flushed and synced before its cancel
actions can leave the coordinator, with a bounded timeout that fails the runtime
closed.

Startup moves through configured, reconciling, awaiting-streams, ready, and
degraded phases. Ready requires verified account-scoped instrument metadata,
matching account/position modes, an authoritative account snapshot already
applied to the strategy and risk engine, a writable critical log, clean
checkpoint/REST reconciliation, every sequenced book, every configured healthy
stablecoin reference, and all configured private channels for every account.
Orders, account, and positions transports are required, and account plus
positions must each deliver a real data payload before private recovery. A
completed pair of fresh state-channel payloads, rather than a socket pong or an
event-only order/fill message, refreshes account-scoped private health. The
dedicated fills channel is opt-in because OKX restricts it by fee tier; fills
from the orders channel remain canonical. Any lost invariant blocks new orders
while demo-mode cancels remain available.

The strategy core retains static `master_strategy` and `strategy_group` fields
for parity experiments and backtests. The live configuration boundary rejects
both because the external Java `StrategyUpdate` liveness, member-state, and
aggregate-PnL feed is not implemented; the runtime does not silently substitute
local-only risk semantics.

The emergency composition is intentionally not part of the strategy loop. It
uses its own REST transport, exchange-adjusted signing clock, absolute
per-account deadline, deadman arm, bounded request pacing, batch cancellation,
and repeated account-wide pending-order queries. This separation keeps the kill
path usable after live-process or strategy-state failure. OKX algo and spread
orders are outside this regular-order scope and remain an operationally explicit
venue procedure.

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

#### Deterministic execution clock

`reap-backtest` owns a single deterministic scheduler. Normalized JSONL and CSV
use each event's millisecond timestamp as arrival time. Raw captures instead use
the persisted host `recv_ts_ns` at full nanosecond resolution, so exchange
timestamp skew cannot reorder the local event loop and sub-millisecond arrival
gaps are retained. Existing strategy/order event timestamps are projected to
milliseconds at delivery. Input clock regressions are clamped and reported
rather than silently moving simulation time backwards.

Each strategy `NewOrder` creates a canonical `PendingNew` immediately. The order
becomes eligible for matching only after `order_entry_latency_ms`, against the
book then visible to the matcher. Likewise, a live order remains matchable until
`cancel_latency_ms` expires. Exchange lifecycle updates and synthetic
fill-derived account state reach the strategy after their independently
configured delays. `PendingNew` counts as working quote/hedge state, matching the
live coordinator boundary and preventing repeated market events from creating
duplicate intents while an acknowledgement is in flight.

The `[backtest]` fields map to the pinned Java backtest model as follows:

| Rust field | Java reference | Scope |
| --- | --- | --- |
| `market_data_latency_ms` | `MarketDepth`, `Quote`, `HistoryTrade`, and `MatchingTrade` | One additional local visibility delay; raw receive time already contains capture-side transport and scheduling |
| `order_entry_latency_ms` | `MatchingNew` | Intent to matching eligibility |
| `cancel_latency_ms` | `MatchingCancel` | Cancel intent to matching ineligibility |
| `order_update_latency_ms` | `OrderUpdate` | Exchange order transition to strategy visibility |
| `fill_account_latency_ms` | `OrderFill` plus live fill-to-account convergence | Exchange fill to synthetic authoritative position visibility; Java couples position/account publication to order-update delay |
| `depth_fill_conservative_threshold` | `backtest.depth.fill.conservative.threshold` | Required relative over-cross before a resting order fills from displayed depth; trade fills and new taker orders are unchanged |

All fields default to zero for backward-compatible deterministic fixtures. The
OKX example explicitly uses the pinned Java application's conservative depth
default of `0.0001`, but does not claim it is calibrated. The report embeds the
effective values and a `calibrated` declaration. That flag must remain false for
guessed sensitivity values. Assumptions are global in the current implementation;
production research still requires venue, instrument, message class, and
percentile distributions from representative captures and demo order traces.

Events already due strictly before the next input are drained first. Actions due
at the same instant as a market event execute after that market event, which is
the conservative choice for entry and cancel races. The runner never advances
past the final observed clock just to empty its scheduler because doing so would
match future actions against a stale final book. Reports therefore expose live
orders and every category of pending action at the dataset boundary.

#### Deterministic accounting

Every simulated fill records absolute turnover and maker/taker fee cost from the
instrument configuration. Fee cost may be negative for a maker rebate. The
portfolio applies linear and inverse position cash flows separately and reports
fee cost, funding PnL, turnover, cash, positions, and marked final equity.

Funding-rate events update the latest rate for `(symbol, funding_time_ms)` and
schedule one settlement at the exchange funding time. Linear swap funding cost
is `position * contract_value * rate * mark`; inverse cost is
`position * contract_value * rate / mark` in coin. Positive signed cost is paid,
so report `funding_pnl_usd` is its negation. Explicit exchange mark events take
precedence over depth midpoint fallback. Duplicate forecasts update the rate
without scheduling duplicate settlements.

A first forecast up to 60 seconds late is settled immediately and reported as
late, matching the Java tolerance window. Older first forecasts are not applied
to a potentially different position and instead increment
`missed_funding_settlements`. Invalid rates, missing marks for nonzero positions,
and other settlement failures make `accounting_complete=false`. A funding time
beyond the dataset remains a categorized pending action and is not a failure for
the observed horizon.

This is still research accounting, not an exchange statement. It assumes a zero
initial portfolio, does not model borrowing interest, liquidation, margin
discounts, tax, or coin-denominated fee drift, and cannot infer a missing funding
event when the source dataset never contained one. Production evaluation must
reconcile these outputs against demo account statements and complete funding
coverage across every held interval.

### `reap-capture`

Credential-free public market-data composition.

Responsibilities:

- Build explicit public-only subscription plans from TOML.
- Capture the stablecoin/index references required to reproduce live risk
  inputs alongside strategy market data.
- Run redundant websocket connections through `reap-feed` supervision.
- Persist every received raw frame through a bounded lossless writer.
- Stamp every frame with a process-session identity so replay cannot hide
  capture downtime.
- Fingerprint the effective capture config and exact persisted bytes.
- Optionally persist deduplicated normalized events for short diagnostics.
- Report connection readiness, queue high-water marks, parser failures,
  duplicates, gaps, recoveries, and final book health.
- Stream raw files through the live adapter/reducer path to measure configured
  source coverage, receive/exchange timing, depth, spread, movement, and trade
  distributions without retaining the dataset in memory.

Long-running collection uses raw JSONL as the canonical format. Backtest raw
replay reconstructs full books through the same adapter, deduplication,
sequencing, and reducer path as live; this avoids materializing a 400-level
book snapshot for every captured delta.

### `reap-storage`

Durable capture and replay data.

Responsibilities:

- Raw websocket capture.
- Normalized event logs.
- Strategy decisions and order intents.
- Order/fill/account events.
- Write-ahead operator/risk safety latches and restart reduction.
- Canonical journal-path validation and a process-lifetime exclusive writer
  lease acquired before recovery or network setup.
- Book snapshots.
- JSONL initially, Parquet or binary logs later.

Normal storage must never block the hot path. Use bounded queues and explicit
drop or degrade policies. Safety-control mutations are infrequent and may await
durable media before dispatch because losing a kill across process restart is
more dangerous than control-path latency.

### `reap-telemetry`

Observability.

Responsibilities:

- Metrics: feed lag, duplicate rate, gap count, reconnect count, book age,
  command latency, ack latency, fill latency, queue depth.
- Structured logs.
- Health endpoints.
- Panic and task death reporting.
- Bounded webhook alert delivery with timeout/retry limits and delivery-failure
  feedback to the live coordinator.

Metrics should be cheap to emit and safe to drop under pressure.

### `reap-cli`

User-facing binaries.

Target command surface:

```text
reap live --config config/live.toml
reap capture --config config/capture.toml
reap emergency-cancel --config config/live.toml --account account-id
reap operator --config config/live.toml status
reap backtest --config config/backtest.toml --events data/events.jsonl
reap replay-check --events data/events.jsonl
reap analyze-capture --config config/capture.toml --events data/events.jsonl
reap inspect-book --capture raw/ws.jsonl --symbol BTC-USDT
reap config-check --config config/live.toml
```

`live`, `capture`, `emergency-cancel`, `operator`, `backtest`, `replay-check`,
`analyze-capture`, and `config-check` are implemented. `inspect-book` remains
planned; see [trading-readiness.md](trading-readiness.md).

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
sequence layer decides whether each candidate is an exact redundant image, a
contiguous update, stale, gapped, or requires recovery.

## Deduplication

Dedup must be channel-aware. There is no universal exchange id that works for
all events.

For a sequenced book, the concrete key is `(action, prev_seq_id, seq_id,
exchange_ts_ms, raw_payload_hash)`, scoped by venue, channel, and symbol.
Including the predecessor and timestamp is required for redundant sockets: OKX
can reuse a sequence number after maintenance and can emit a later no-change
update with `prevSeqId == seqId`. The payload hash prevents replicas with
conflicting contents from being silently suppressed. A key containing only
`seqId` would collapse valid events from different sequence epochs.

Dedup rules:

- Book snapshots/deltas: use the complete sequence-transition key above; only
  a byte-identical image from another socket is a duplicate. Conflicting content
  with the same transition reaches sequencing and forces recovery.
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

For every channel with sequence semantics, continuity is defined by the
predecessor link, not by numerical ordering of sequence IDs:

```text
ready update:
  if prev_seq == last_seq:
    apply
    if seq == last: count no-change update
    if seq < last: count maintenance reset
  else:
    enter recovering and request a fresh snapshot

recovering:
  buffer deltas
  fetch snapshot
  apply snapshot
  discard buffered messages older than the snapshot
  replay buffered deltas by prev_seq links
  if every remaining message is contiguous:
    ready
  else:
    refetch or restart socket
```

This matches the OKX contract's no-change and maintenance-reset cases. A lower
snapshot may replace the previous epoch while explicitly recovering; an
unsolicited lower snapshot cannot rewind a ready book. Crossed or otherwise
invalid books independently force the same fresh-snapshot path.

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
measured event-loop benchmark. The live composition also has exclusive journal
ownership and optional Linux disk/memory/clock guards. These are reusable
components, not a completed production deployment.

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

Status: implemented for fail-closed OKX demo validation, observation, and
explicitly confirmed order entry. Credentialed exchange soak acceptance remains
the deployment blocker.

- Add the live composition process and single-writer event-loop owner. Done.
- Implement executable startup, readiness, reconciliation, and restart gates.
  Done.
- Verify instrument metadata, account mode, and risk valuation. Done.
- Profile the wire-to-action parity loop and remove measured collection churn.
  Done.
- Emit bounded soak evidence for readiness recovery, reconciliation drift,
  storage pressure, and zero-order shutdown. Done.
- Route every demo exit through submit-disabled cancellation and per-account
  post-cancel REST reconciliation, even when persistence has failed. Done.
- Authenticate bounded local operator commands outside the strategy loop and
  reduce accepted control events on the single writer. Done.
- Isolate one account without resetting healthy account routes, while removing
  its instruments from strategy pricing/hedging and cancelling its canonical
  orders. Done.
- Capture redundant public data without credentials and replay it through the
  live adapter/dedup/sequence/book path. Done.
- Add exchange-side request expiry, server-clock validation, independent Cancel
  All After heartbeats, and durable restart latches. Done.
- Add exclusive journal ownership, bounded external alerts, and host resource
  preflight/periodic guards. Done; target-host deployment evidence remains.
- Add strategy-independent account-wide regular-order cancellation and hardened
  mode-specific process supervision templates. Done; target-host/account fault
  exercise and separate algo/spread handling remain.
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
are more dangerous than missing public ticks. Reconciliation compares canonical
orders, known fills, balances, and positions before applying the REST account
snapshot. A dirty pass repairs local state but remains failed; only a later
clean pass restores the reconciliation gate. Independently, each canonical fill
starts a bounded account-state convergence deadline, so healthy aggregate
heartbeats cannot conceal a missing symbol/currency update.
