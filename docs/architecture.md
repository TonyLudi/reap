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
    reap-fault/
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
- Lossless bounded delivery for ready/disconnect transitions; payload traffic
  does not flood or silently evict critical connection state.
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
- A command-transport boundary that keeps canonical identity independent from
  REST or authenticated websocket IO.
- Canonical order state reducer.
- Typed one-to-one exchange/client order-ID binding from exchange acknowledgements,
  with contradictory journal history rejected and active bindings restored
  before startup REST reduction; private order/fill symbol ownership and
  immutable symbol/side checks occur before canonical state mutation.
- Semantic private-order deduplication treats repeated instrument-scoped fill
  IDs and unchanged terminal order states as duplicates independently of
  exchange update time. Restart baselines persist `(symbol, fill_id)`; legacy
  unscoped journal IDs are read as conservative wildcards. If the optional
  fills channel arrives first without fee fields, it updates private state but
  leaves the journal key available for the fee-bearing orders-channel update.
- REST reconciliation for open orders, fills, balances, and positions. OKX
  fills are paced and paginated until a short page proves completion; repeated
  cursors/fills or the configured page bound fail the reconciliation.
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
- Separate typed OKX economic snapshots retain account borrowing flags,
  aggregate and per-currency liabilities/interest/borrow-frozen values, and
  margin-position loan fields. Bootstrap fails closed on missing applicable
  evidence, enabled borrowing, any nonzero liability, or a margin position;
  every later normalized account row rejects nonzero liability as well.
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
- Bind every path-launched live report to exact source-config bytes and effective
  fingerprints. Offline verification treats the report as untrusted, re-derives
  clean-soak status, and checks mode, build/Java provenance, host/account
  identities, readiness, failure/disconnect counters, and latency reservoirs.
- Aggregate isolated live fault reports through a strict manifest that requires
  the complete role matrix, unique sessions and injector artifacts, one
  config/build/host/account identity, recovered reconnects, and safe zero-order
  shutdown. Reap proxy evidence is parsed and validated for supported roles;
  opaque external injector records and certifications remain explicit
  limitations rather than inferred facts.
- Start/stop/restart strategy instances.
- Dispatch timers into strategy loops.
- Coordinate reconciliation and recovery.
- Collect authenticated, read-only recent-fill evidence with bracketed exchange
  clock/account-identity samples, bounded paced pagination to a proven short
  page, and exact create-new response/config/manifest hashes. Reconciliation
  independently replays that cursor chain, leases the canonical journal, and
  binds the collection config identity to the journal bootstrap identity.
- Collect account-wide OKX bills for an exact closed window through the same
  read-only provenance boundary. Offline reconstruction reopens every raw page,
  rebuilds the request cursor chain, and rejects duplicate IDs, out-of-window
  rows, a full terminal page, or any changed source.
- Reconcile normal trade and funding bills against the verified fill collection
  and a streaming pass over the stopped journal. Unknown bill types fail closed;
  trades bind on `(symbol, tradeId)`, one account-scoped critical journal fill,
  and exact fee currency. Every authoritative REST account replacement first
  writes a critical `account_snapshot` carrying the exchange `avgPx`. Derivative
  fills replay from the latest same-session snapshot using the pinned Java
  arithmetic average for linear contracts and harmonic average for inverse
  contracts. The snapshot exchange timestamp must strictly precede every
  replayed fill; close PnL and open/close subtype are then recomputed independently
  of the bill balance equation. Funding binds to a session-local journaled
  realized rate and assessment-time signed position.
  Two journaled `mark-price` observations must bracket the bill `fillTime`; the
  bill mark and linear/inverse funding PnL are then checked against the resulting
  independent ranges. Each runtime start writes a schema-7 `session_start` per
  account with session, strategy, config, and hashed OKX account identity; line
  boundaries prevent settlement, position, or mark evidence from crossing a
  restart. Separately verified account certifications must tightly bracket the
  bill window. Numeric bill IDs and post-bill balances form a per-currency chain
  from opening `cashBal` through every `balChg` to closing `cashBal`. This is
  intentionally outside the hot event loop.
- Collect a self-contained point-in-time account certification from exact
  authenticated config, balance, and positions responses plus a public direct
  index response for every non-USD balance currency. Its mode-aware policy
  requires cash-only spot routing, zero configured borrowing, disabled venue
  borrowing, complete applicable zero-liability/interest evidence, no margin
  positions, stable bracketed identity/settings, bounded exchange time, strict
  `sum(eqUsd) = totalEq`, and independently bounded `eq * index = eqUsd`.
  Offline verification re-hashes and re-parses every embedded response.
- Certify controlled process-death deadman expiry through a separate read-only
  composition. It leases and fingerprints the stopped journal, recovers durable
  live exchange/client bindings, retains exact order-detail and account-wide
  pending-order responses, and requires OKX cancellation source `20`. Its
  credential-free verifier independently leases and replays the exact journal
  alongside every embedded response.
- Reduce durable global/account/symbol latches, block halted routes, and
  guarantee scoped canonical cancellation.
- Promote terminal strategy safety halts into the global risk gate before
  dispatching intents from the triggering event; global cancellation always
  takes precedence over simultaneous symbol isolation.
- Validate exchange time, continuously compare authenticated account config to
  its bootstrap identity/settings, poll announced OKX unified-account system
  maintenance, compare strategy-critical instrument rules and fee assumptions
  to authenticated current metadata, expire stale place requests at the venue,
  and own an independently scheduled exchange deadman lifecycle per account.
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
Before network feeds start, one unsigned `/api/v5/system/status` request must
also prove that no relevant maintenance is active or inside the configured lead
window. The first account safety task repeats that global check every 10 seconds
by default. Its service filter matches pinned Java
`OkxNitroUtils.getExchStatus`: unified trading, account-batch, product-batch,
spread, and other events are relevant; websocket, block, bot, and copy-trading
events are not. Rust additionally filters OKX's current `env` field and turns a
relevant status or failed poll into fail-closed cancel/reconcile shutdown rather
than Java's recoverable strategy pause.
Each exact account instrument must also provide current `upcChg` announcements
and the OKX trading-fee `groupId`. Bootstrap rejects non-live instruments or an
announced `tickSz`, `minSz` (and synchronous derivative `lotSz`), or `maxMktSz`
change inside the configured one-hour lead. It then queries
`/api/v5/account/trade-fee` with the exact spot instrument or derivative family,
selects that group from `feeGroup`, and converts OKX's signed balance rate into
the strategy's cost-rate convention. Configured maker and taker costs may be
more conservative, but may not understate a commission or assume a larger
rebate. A paced periodic full sweep first compares state, type, family,
currencies, contract type/value, tick/lot/minimum size, hard limit/market order
quantity maxima, applicable spot order-amount maxima, and fee group to the
bootstrap snapshot, checks announcements, and then rechecks fees. Bootstrap
rejects strategy quote maxima above the authenticated limit-order bounds. The
live risk gate also checks every post-only or IOC order against current
`maxLmtSz` and applicable `maxLmtAmt` before dispatch, covering Java-equivalent
hedges aggregated across depth levels. The metadata sweep runs as a child of
the account safety lifecycle so a blocked metadata or fee request cannot delay
Cancel All After heartbeats. Missing or unknown `upcChg` contracts and
deprecated top-level fee fields are rejected as insufficient evidence.
Tradable demo startup additionally requires every configured authenticated
order-command websocket session for every account. The default pool has eight
sessions, matching the pinned Java topology, and deterministically routes spot,
swap, and dated-future symbols with the same underlying to one session.
Orders, account, and positions transports are required, and account plus
positions must each deliver a real data payload before private recovery. A
completed pair of fresh state-channel payloads, rather than a socket pong or an
event-only order/fill message, refreshes account-scoped private health. The
dedicated fills channel is opt-in because OKX restricts it by fee tier; fills
from the orders channel remain canonical. Any lost invariant blocks new orders
while demo-mode cancels remain available.

The websocket command tasks correlate bounded request IDs, attach exchange
`expTime` to place requests, and classify failures at the write boundary.
Unavailable before send is explicit; a write followed by timeout, disconnect,
or malformed correlation remains pending for private/REST reconciliation. A
command-session loss invalidates both transport and reconciliation readiness,
emits canonical account cancels, and requests full reconciliation. REST remains
independent for snapshots, reconciliation, Cancel All After, pre-send cancel
fallback, and emergency cancellation.

Each account has separate command and REST-reconciliation tasks. The command
owner keeps idempotency and submit finalization single-threaded, but dispatches
IO through constant-time per-underlying FIFOs. One operation per underlying may
be in flight, while different underlying families run concurrently up to the
configured websocket-session count. This preserves submit/cancel order within a
family, matches the pinned Java family routing, and prevents one acknowledgement
from blocking unrelated families. Every completion returns to the command owner
and then to the single-writer coordinator; IO futures never mutate strategy
state.

The reconciliation task uses a cloned authenticated REST client and cannot be
blocked by websocket acknowledgement latency. Command and reconciliation
clients share account pacing reservations without holding a lock across an
await. During fail-closed shutdown, an explicit command flush waits for all
earlier cancels and command completions before zero-order REST reconciliation is
queued. The command channel, per-family pending queues, total in-flight work,
and reconciliation channel are all bounded.

Private fill fees use one normalized contract: the amount is the signed balance
delta, so a charge is negative and a rebate is positive, and the currency names
the balance that changed. The orders channel maps current per-update `fillFee`
and `fillFeeCcy`; it intentionally ignores order-level `fee` and `rebate`
because those are cumulative. The fills channel and REST reconciliation map the
per-fill `fee` and `feeCcy`. This follows the pinned Java distinction between
the cumulative `OrderDetailUpdate` fields (differenced by `ExchFillUtils`) and
the per-fill REST `UserTrade` fields, while using the newer OKX per-update order
fields directly. Canonical fill deduplication prevents a fills-channel copy from
charging the same trade again, and both canonical order records and REST/fills
journal records retain the exact fee evidence.

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
venue procedure. Its optional CLI evidence path is reserved before config or
network work and fsynced before exit. The schema-versioned report hashes the
exact input file without invoking the live parser, uses the same executable and
host identity hashes as live evidence, samples the same pseudonymous OKX
UID/main-UID account identity only after zero is proven, records the pinned Java
revision, and converts account-task join failures into bounded evidence.
`all_clear` requires both account-complete regular-order zero proof and complete
provenance; an early parse/validation failure leaves only an empty reserved path.

The deadman-certification composition is also outside the strategy loop but has
the opposite authority boundary from emergency cancellation: it uses only
public time and authenticated GET operations. The canonical journal lease is
acquired before credentials or network setup, and any `pending_new`, unbound,
unmapped, or truncated recovered state fails closed. This path provides causal
demo evidence for already-armed Cancel All After; it cannot arm or refresh the
timer and must never delay incident cancellation.

Later, if the strategy loop becomes latency-critical, replace the async receive
with a pinned OS thread and a bounded SPSC queue.

### `reap-fault`

Deterministic OKX demo fault-injection infrastructure, launched as a separate
process from `reap-live` and excluded from the strategy event loop.

Responsibilities:

- Accept only an official, region-consistent OKX demo REST/public/private
  upstream tuple and bind four distinct loopback listeners: REST, public market
  data, private account state, and authenticated order commands.
- Render an exact validated loopback live config. `venue.order_ws_url` is
  optional for ordinary deployments and falls back to `private_ws_url`; the
  routed test config sets it explicitly so private-state and order-transport
  faults remain independently attributable.
- Forward REST and websocket traffic without logging authorization headers,
  login frames, raw account/order payloads, or injected response bodies. Evidence
  retains only bounded metadata, byte counts, and SHA-256 digests.
- Accept strict bounded commands over a mode-`0600` Unix socket. Supported
  primitives are targeted websocket disconnects, matched one-shot or bounded
  frame drops, and matched REST responses.
- Create each completed injector artifact once with mode `0600`, pinned Java and
  proxy-config provenance, exact command summary, effect timing, and hashed
  payload metadata. Failed disconnect completion is explicit and cannot satisfy
  the live fault matrix.
- Bind typed artifacts to the exact matrix role: transport ownership for
  reconnects, acknowledgement operation for ambiguity, private-state channel for
  convergence, and method/path for periodic REST safety checks. Genuine partial
  fills and restart-latch persistence are outside the proxy's causal boundary,
  so typed proxy artifacts cannot satisfy those roles.
- Track every listener and connection task through shutdown. A clean run report
  requires no proxy error, pending fault, active websocket, or stale control
  socket.

This crate is test infrastructure, not an exchange gateway abstraction and not
a production sidecar. Configuration validation rejects production upstreams,
non-loopback listeners, mixed endpoint tuples, and reused evidence. Runtime
fault switches are deliberately absent from `reap-live`; the process boundary
keeps campaign authority out of normal trading state.

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
| `latency_profile.rules` | `MarketDepth`, `HistoryTrade`, `MatchingNew`, `MatchingCancel`, `OrderUpdate`, and `OrderFill` | Bounded uniform empirical samples by class and optional symbol; `reference_data` is a Rust extension for index/funding/limit/burst inputs |
| `depth_fill_conservative_threshold` | `backtest.depth.fill.conservative.threshold` | Required relative over-cross before a resting order fills from displayed depth; trade fills and new taker orders do not use the threshold |
| `queue_ahead_multiplier` | Java displayed queue at `1.0`; Rust sensitivity overlay | Multiplies displayed quantity ahead of each newly resting order; values below `1.0` are rejected |
| `historical_trade_fill_fraction` | Java consumes all matching-trade quantity at `1.0`; Rust sensitivity overlay | Haircuts historical trade quantity before queue consumption and maker filling |
| `displayed_depth_fill_fraction` | Java uses all displayed quantity at `1.0`; Rust sensitivity overlay | Caps each displayed level available to resting-depth and taker matching; fractional capacity is shared and unchanged snapshots do not replenish it |

Latency and threshold fields default to zero, while the queue and capacity
fields default to `1.0`, preserving backward-compatible deterministic fixtures.
The OKX example explicitly uses the pinned Java application's conservative
depth default of `0.0001`, but does not claim it is calibrated. Scalar latency
fields remain backward-compatible fallbacks. A class-wide profile rule replaces
its scalar, and a symbol rule replaces the class-wide rule. Samples are sorted
and selected by a stable seed/class/symbol/event-ordinal quantile, so runs are
reproducible regardless of rule order. Reports include actual per-class/symbol
sample count, total, minimum, maximum, and mean latency.

Strategy quote and hedge command assembly traverses symbols and risk groups in
stable order. This follows the pinned Java `ChaosContext`, whose `tradeSymbols`
is explicitly a `TreeSet` to keep results consistent between runs. Portfolio
currency/position reductions, matching-engine order reductions, and matcher
aggregation are likewise ordered. Workspace JSON parsing enables exact `f64`
round trips so writing and re-reading evidence cannot move a PnL or risk value
by one ULP.

The live composition collects corresponding measurements only after an input
survives parsing, deduplication, sequencing, and reduction. Reports use a
bounded deterministic reservoir per class/symbol/semantics and bind the full
serialized live config, Rust executable, pseudonymous machine/account identity,
and pinned Java revision. `calibrate-latency` validates clean bounded sessions,
synchronized clocks, complete per-instrument coverage, zero measured-operation
failures, and explicit acceptance of REST-ack matching upper bounds before
producing a profile. Production research treats that JSON as untrusted
evidence, requires the byte-identical executable and an exact baseline-profile
match, and rechecks its content hash after execution.

The CLI reserves a create-new live evidence path before configuration,
credential, or network work. After the report-capable runtime exists, every
initialization, event-loop, or teardown error follows the same
cancel/reconcile/shutdown path and then returns a schema-versioned report
containing the primary stable failure code, bounded diagnostic, pre-cleanup
readiness snapshot, post-cleanup active-order count, and all accumulated
evidence. The report is persisted before the original error produces a nonzero
exit. Failures before runtime construction cannot claim runtime state and leave
only an empty reserved path plus process logs.

Baseline and stress profiles must share a seed. Research accepts a stress
distribution only when it first-order stochastically dominates the effective
baseline distribution for every class and every configured symbol override;
shared quantiles then make each sampled stress delay no smaller than baseline.
The report embeds every effective value and a `calibrated` declaration. That
flag must remain false for guessed sensitivity values. Production research
still requires populated venue/instrument/message-class distributions from
representative target-host processing measurements and demo order traces.

Events already due strictly before the next input are drained first. Actions due
at the same instant as a market event execute after that market event, which is
the conservative choice for entry and cancel races. The runner never advances
past the final observed clock just to empty its scheduler because doing so would
match future actions against a stale final book. Reports therefore expose live
orders and every category of pending action at the dataset boundary.

#### Deterministic accounting

Every simulated fill records absolute turnover and maker/taker fee cost from the
instrument configuration. Fee cost may be negative for a maker rebate. The
portfolio applies spot and linear cash flows to explicit quote/settlement
currency ledgers and keeps inverse contract cash in coin by symbol. It reports
those raw ledgers alongside fee cost, funding PnL, turnover, positions, and
marked final equity.

Every non-USD accounting currency requires an explicit
`[[backtest.currency_rates]]` route naming a direct index whose price is USD per
currency unit and a maximum age. Empty legacy currency fields mean USD; no
named non-USD currency silently receives par treatment. Index observations
become usable when their latency-scheduled `ReferenceData` event reaches the
strategy boundary; freshness still ages from the retained source timestamp, so
transport and simulated processing delay consume the budget. Spot position
value, linear and inverse notional, active orders, fees, turnover, and funding
are then converted through the same fresh rate. Missing, invalid, or stale rates
make valuation/accounting incomplete;
fills or funding settled before a usable rate increment
`currency_conversion_failures`. A non-passing report retains the latest rate,
or par only if none was ever observed, for best-effort numeric output and marks
that fallback explicitly through the completeness fields.

Research scenarios inherit these routes from each candidate configuration.
They may repeat an exact route set but cannot replace it, so latency/capacity
stress never changes the valuation source or freshness policy selected for that
candidate.

Funding-rate events retain two distinct values: OKX `fundingRate` is the
forecast delivered to the shared strategy, while an optional
`settFundingRate` plus observed `prevFundingTime` is a realized accounting
observation. Before chronological execution, raw replay makes a read-only pass
that builds the realized `(symbol, funding_time_ms)` map. Forecast events still
schedule one action at the advertised exchange time, but the action can book
only the realized map value. The map is private to the portfolio scheduler and
cannot affect strategy decisions before the settlement observation arrives on
the event stream. Conflicting realized values for one key reject the run.

Linear swap funding cost is `position * contract_value * rate * mark`; inverse
cost is `position * contract_value * rate / mark` in coin. Positive signed cost
is paid, so report `funding_pnl_usd` is its negation. Explicit exchange mark
events take precedence over depth midpoint fallback. Duplicate forecasts do not
schedule duplicate settlements.

A first forecast up to 60 seconds late is settled immediately and reported as
late, matching the Java tolerance window. Older first forecasts are not applied
to a potentially different position and instead increment
`missed_funding_settlements`. A due forecast with no realized rate, invalid
rates, missing marks for nonzero positions, and other settlement failures make
`accounting_complete=false`. A funding time beyond the dataset remains a
categorized pending action and is not a failure for the observed horizon.

The pinned Java portfolio already separates spot base and quote accounts and
settles derivative funding in the instrument settlement account, but its
account summary treats `Currencies.isUsdEquivalent` at one. Java's separate
`StableCoinDepegCheckerImpl` is a live safety guard. Rust preserves that
strategy-decision parity and adds depeg-sensitive conversion only at the
research accounting boundary.

This is still research accounting, not an exchange statement. It assumes a zero
initial portfolio. Private normalized fills can carry exact signed fee and
currency evidence; those fees are booked in spot base/quote balances, inverse
settlement coin, or the reported third currency as applicable. Public-data
matching has no account fee event, so it estimates fees from configured
maker/taker rates. Reports expose exact and estimated fee-fill counts. The
backtest model does not import statements, borrowing interest, liquidation,
margin discounts, or tax, and it cannot infer a missing funding event when the
source dataset never contained one. A separate offline production-evidence path
now reconstructs normal trade/funding bills and derivative close PnL from
same-session authenticated REST position basis plus critical fills. That is
local journal provenance, not remote process attestation. The supported live
spot boundary is cash mode; production certification must prove zero
liabilities. Enabling margin
spot later requires a separate borrow-rate and interest model first. The
operational gate now proves tightly bracketed bill/cash continuity and endpoint
currency conversion; production evaluation must still review total-equity
attribution and sustained index coverage across every held interval.

#### Deterministic walk-forward research

The pinned Java `ChaosBackTestMultiRunService` partitions a requested interval
by day, loads `strategy-%s.json` for each date, carries `endingPositions` into
the next run, and writes date/run-scoped order, fill, market-value, and result
artifacts. `ChaosBackTestResult` itself contains engine state, ending time, and
ending positions. Java's external Python scenario queue is not part of the
pinned repository, so it is not treated as verified walk-forward logic.

Rust keeps that per-run artifact discipline but makes research selection and
acceptance explicit in a versioned TOML manifest:

- Candidate configurations are evaluated only on each fold's training data.
- The deterministically selected candidate alone reaches the later test window.
- Production manifests predeclare one deployment candidate and fail unless that
  same candidate wins training-only selection in every fold.
- Exactly one baseline and zero or more no-less-conservative execution stress
  scenarios run on test data.
- Dataset IDs, canonical paths, and byte-identical content cannot be reused to
  disguise train/test leakage; event-time windows must be non-overlapping and
  strictly chronological.
- Production raw captures must name their capture-only config and schema-4 run
  report. The report verifier binds exact config bytes, capture executable,
  host, pinned Java revision, host-guard evidence, and effective output overrides
  to raw and optional reconstructed normalized bytes. It cross-checks replayable
  counters, session, and book health, and retains runtime-only stop, readiness,
  and queue evidence. Production research requires the same executable and the
  latency-calibrated target host plus at least one completed periodic host
  check. It must pass alongside a one-session,
  parse-clean, zero-gap replay check before any candidate runs.
  Every configured stream must have at least two sources; candidate instruments
  must have book and trade coverage plus applicable strategy index, accounting
  currency index, mark, limit, and funding streams.
- Reports embed selection/gate policy, fingerprint the manifest, executable,
  candidate files, effective strategies, and data, and preserve every
  underlying `BacktestReport`.
- Gates cover data/fill duration, accounting and valuation completeness,
  pending actions/order transitions, clock regressions, net PnL, drawdown,
  position and pending-hedge delta, gross position and active-order exposure,
  and inventory-open duration. When any candidate trades a swap, every
  training and test fold aggregate must also meet a nonzero realized funding
  settlement gate.

Each Rust dataset currently starts from a zero portfolio and independent
strategy instance, which is emitted as
`independent_zero_initial_portfolio` in the report. This differs from Java's
daily position carry. Use one continuous capture as an evaluation dataset when
inventory continuity matters, and constrain terminal delta/gross exposure in
the manifest. Cross-file position carry must not be inferred from aggregate
PnL. A schema-5 `production_candidate` manifest additionally requires one
predeclared deployment candidate, at least three folds, two stress scenarios,
nonzero event, fill, and duration gates, calibrated baseline execution whose
latency profile exactly matches a passed source-bound calibration artifact,
complete accounting, and explicit bounds on non-funding work censored by each
data horizon. Stress scenarios may use explicitly uncalibrated deterministic
haircuts.

The CLI independently verifies a schema-4 latency artifact before release use.
It re-hashes an explicit complete set of archived live reports, reruns each
live-report verifier against the exact config, rebuilds all Java-mapped series
with the artifact's recorded options, and compares the complete result after
normalizing source paths to content hashes. This separates generator output from
acceptance evidence and permits archive relocation without permitting byte or
profile drift.

The CLI also independently verifies a research report. It reads bounded regular
manifest/report files, rejects symlinks and path collision, requires the pinned
Java revision plus the current executable/version, re-runs the complete
manifest, and compares the full report after normalizing only canonical paths
introduced by capture verification to their content hashes. Unknown, duplicate,
omitted, stale, forged, non-passing, or numerically different results fail closed. The
verification artifact carries the exact source hashes and a bounded first
difference diagnostic; it remains simulation evidence rather than production
authorization.

The format-2 research verifier also derives one deployment candidate ID and
effective strategy hash from the report, requiring exactly one matching
candidate provenance row and the same training-selected ID in every production
fold. `verify-research-deployment` then loads an exact production `LiveConfig`,
hashes `strategy.effective()` through the same function used by research, and
requires equality. This closes the research-to-live strategy identity boundary;
it deliberately does not aggregate or replace host, account, transition, fault,
statement, deadman, or emergency evidence.

### `reap-capture`

Credential-free public market-data composition.

Responsibilities:

- Build explicit public-only subscription plans from TOML.
- Capture the stablecoin/index references required to reproduce live risk
  inputs alongside strategy market data.
- Run redundant websocket connections through `reap-feed` supervision.
- Persist every received raw frame through a bounded lossless writer.
- Fail before network startup and during collection when Linux disk, available
  memory, or kernel clock state breaches configured thresholds.
- Stamp every frame with a process-session identity so replay cannot hide
  capture downtime.
- Fingerprint the effective capture config and exact persisted bytes.
- Reserve and fsync a versioned run report that also binds the exact source
  config file, executable, host, pinned Java revision, and host-health evidence.
- Optionally persist deduplicated normalized events for short diagnostics.
- Report connection readiness, queue high-water marks, parser failures,
  duplicates, gaps, recoveries, and final book health.
- Stream raw files through the live adapter/reducer path to measure configured
  source coverage, receive/exchange timing, depth, spread, movement, and trade
  distributions without retaining the dataset in memory.
- Verify a retained run report against relocated raw/normalized artifacts,
  including exact normalized output reconstructed from raw replay.

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
- Per-account runtime-session boundaries with config and account provenance.
- Write-ahead operator/risk safety latches and restart reduction.
- Canonical journal-path validation and a process-lifetime exclusive writer
  lease acquired before recovery or network setup.
- Streaming recovery with a validated-record visitor so offline evidence tools
  can retain only the needed event class instead of materializing a long journal.
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
reap capture --config config/capture.toml --output capture-report.json
reap emergency-cancel --config config/live.toml --account account-id --output emergency.json
reap verify-emergency-cancel --config config/live.toml --report emergency.json --require-pass
reap certify-deadman-expiry --config config/live.toml --account account-id --output deadman.json
reap verify-deadman-certification --artifact deadman.json --journal live-events.jsonl
reap operator --config config/live.toml status
reap backtest --config config/backtest.toml --events data/events.jsonl
reap replay-check --events data/events.jsonl
reap analyze-capture --config config/capture.toml --events data/events.jsonl
reap verify-capture --config config/capture.toml --report capture-report.json --events data/events.jsonl
reap calibrate-latency --config config/live.toml --report live.json --output latency.json
reap verify-fault-proxy-run --config fault-proxy.toml --report proxy-run.json --require-pass
reap verify-production-evidence --manifest production-evidence.toml --require-pass
reap verify-production-approval-policy --policy approval.toml
reap prepare-production-approval --manifest production-evidence.toml --policy approval.toml --request-id CHANGE-123 --output request.json
reap sign-production-approval --request request.json --policy approval.toml --private-key private.json --approver operations --output approval.json
reap verify-production-approval --manifest production-evidence.toml --policy approval.toml --request request.json --approval operations.json --approval risk.json --require-pass
reap inspect-book --capture raw/ws.jsonl --symbol BTC-USDT
reap config-check --config config/live.toml
```

`live`, `capture`, both emergency-cancel commands, both deadman-certification
commands, `operator`, `backtest`, `research`, `replay-check`, `analyze-capture`,
`verify-capture`, `calibrate-latency`, `verify-fault-proxy-run`,
`verify-production-evidence`, all production-approval commands, and
`config-check` are implemented.
`inspect-book` remains planned; see [trading-readiness.md](trading-readiness.md).

Cross-crate production evidence composition belongs in `reap-cli`: `reap-live`
must not depend on `reap-backtest`, while the aggregate verifier must call both
live and research source verifiers. The strict manifest predeclares one release
binary, target host, deployment candidate, approval-policy SHA-256, and
environment-specific account identities. The verifier reruns each source gate,
reopens every deployment config and the controlling manifest, and reconstructs
the loopback fault config from the exact official-demo and fault-proxy configs
before cross-binding all returned identities. It hashes the typed in-memory
reconstructions instead of accepting prior verification JSON. Schema 8
re-verifies every fault/latency live
source, derives completion times from validated sessions or exchange-clock
samples, enforces explicit age limits under hard maxima, and requires each typed
proxy command interval to fall inside its live session. It also reconstructs one
schema-2 proxy process report per scenario, requiring exact config/build/host,
unique sessions, independently derived clean shutdown, and unambiguous live-run
enclosure. One fill/fee and one account-wide trade/funding economic
reconciliation are required per demo account, with exact collection, journal,
opening/closing account boundaries, config, executable, host, and pseudonymous
account bindings. Boundary gaps are capped at 60 seconds and bill freshness is
independently bounded to 24 hours.

Release approval remains a separate `reap-cli` composition layer. A stable typed
subject removes only verifier wall time and derived age while preserving every
source timestamp, freshness limit/result, gate hash, config, candidate, build,
host, account identity, and proxy run. Schema 8 also requires a reviewed nonzero
count of independently recomputed derivative closes. A strict policy requires at least two
sorted roles and distinct Ed25519 keys. Offline signatures bind the exact policy,
request bytes, role, approver, and signing time; final verification reruns the
entire bundle, requires its predeclared policy hash, and requires exact subject
equality inside a hard 15-minute window.
This is deliberately asymmetric rather than reusing the runtime operator HMAC:
the target host can verify approvals without gaining signing capability. Even a
passing approval leaves production entry unauthorized because remote attestation,
external supervision, remote attestation of the locally journaled position
basis, total-equity attribution outside the controlled bill/cash window, and
actual rollout governance remain outside this composition. Funding's bill-reported mark is
checked against same-session public observations, but the exact internal
assessment tick cannot be reproduced externally.

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
- Private fills: prefer execution id or fill id, scoped by venue, account, and
  instrument because OKX only guarantees `tradeId` uniqueness per instrument.
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
- Verify instrument metadata, account mode, and risk valuation at startup, then
  continuously reject exact-metadata drift and imminent rule changes. Done.
- Profile the wire-to-action parity loop and remove measured collection churn.
  Done.
- Emit bounded soak evidence for readiness recovery, reconciliation drift,
  storage pressure, and zero-order shutdown. Done.
- Classify ambiguous order operations, partial fills, convergence timeouts,
  restored durable latches, and periodic safety-task failures in versioned live
  evidence without parsing diagnostic messages. Done.
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
  mode-specific process supervision templates. Done; exact config/report bytes
  and all-configured-account zero invariants are independently verifiable.
  Target-host/account fault exercise, raw REST-response replay, and separate
  algo/spread handling remain.
- Add independently routed demo fault injection with durable typed evidence.
  Done. Execute and accept the credentialed target-host campaign and OKX demo
  soak; tooling alone does not close that gate.

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
