# Reap Architecture

This document describes the target architecture for `reap` as a Rust trading
system that can run the same strategy logic in live trading and backtest.

The workspace now implements the migration baseline and a demo-capable OKX
composition root: strategy/backtest parity, live feed/order boundaries,
deterministic risk, executable readiness/restart state, telemetry, and durable
capture. Exchange certification and production deployment controls remain
operational gates rather than strategy code.

The current Chaos exchange-authority contract is
[chaos-connectivity-boundary.md](chaos-connectivity-boundary.md). It controls
which of the generic venue/runtime capabilities described here may be composed
for Chaos. The staged enforcement and responsibility split are tracked in
[chaos-connectivity-refactor-plan.md](chaos-connectivity-refactor-plan.md).

## Goals

- Replicate the important `imm-strategy/chaos` decision logic in Rust.
- Keep strategy behavior deterministic and replayable.
- Support only plan-derived live exchange connections, with multiple websocket
  connections only for explicit sharding, isolation, capacity, or tested
  redundancy requirements.
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
- Ping/pong, reconnect, readiness-aware exponential backoff, and stale-feed
  detection. Repeated failures before subscription readiness back off, while a
  successfully acknowledged session resets historical delay before its next
  disconnect.
- Fail-closed connection-attempt pacing shared by public, private, order-command,
  capture, and fault-proxy upstream sockets across processes on one host. The
  owner-only advisory-lock file reserves Linux `CLOCK_BOOTTIME` slots under the
  current kernel boot ID before sleeping, so a reconnect storm cannot race
  independent process-local timers or a wall-clock correction.
- Lossless bounded delivery for ready/disconnect transitions; payload traffic
  does not flood or silently evict critical connection state.
- Exact per-socket subscription readiness: the acknowledged argument set,
  including channel and instrument selectors, must equal the unique serialized
  request set. Duplicate acknowledgements are idempotent, unknown or malformed
  success frames fail the connection, and a duplicate local plan is fatal before
  network startup.
- Bounded socket lifecycle: a feed handshake is shutdown/recovery cancellable
  and limited to 10 seconds, while bootstrap, subscription, heartbeat, and close
  writes are limited to 5 seconds. Every socket keeps its recovery channel
  owned for the full supervisor lifetime; an unexpected owner loss is fatal.
- Private payload delivery is lossless while capacity is available, but waiting
  on the bounded event queue is limited to one second. Saturation then produces
  a typed disconnect so private readiness is revoked, active orders are
  cancelled, and REST reconciliation is required after recovery; the socket
  reader cannot remain trapped behind a stalled event loop.
- Separate per-socket transport liveness from aggregate private-state health.
- Raw message timestamping.
- Cross-socket deduplication without suppressing per-socket sequence advancement.
- Per-socket sequence checking, full-book reduction, and gap detection.
- Source-scoped snapshot recovery and canonical cross-replica book arbitration.
- Channel-level health metrics.

Key design point: each websocket advances its own sequence tracker and full-book
reducer before a canonical book is selected. `reap-feed` emits normalized events
only after that arbitration. The strategy should not know which websocket
produced an event.

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
- Authenticated websocket command sessions with bounded handshakes, login,
  control writes, request expiry, and acknowledgements. Heartbeat telemetry is
  best-effort at both bounded status queues so it cannot stall command IO;
  ready/disconnected/fatal transitions remain lossless and drive fail-closed
  cancellation plus reconciliation.
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
- A venue-neutral reference-data contract for configured index prices, swap
  funding, derivative marks, and price limits, with independent source clocks.
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
- Separate the order-safety deadline from the runtime-owner teardown deadline.
  Signal host, operator, feed, order-command, command, reconciliation, and
  safety owners before awaiting any one owner. A teardown timeout aborts every
  remaining owned task, retains Cancel All After, releases local sockets and
  journal ownership, and becomes typed non-clean report evidence. Successful
  journal close flushes and data-syncs before the runtime report is built.
- Bind every path-launched live report to exact source-config bytes and effective
  fingerprints. Validate config/run options and capture build/host provenance
  before report reservation. After reservation, represent a handled pre-session
  failure without claiming account identity or runtime state, and abort any
  startup-owned tasks that have not transferred into `LiveRuntime`. Offline
  verification treats every report as untrusted, re-derives clean-soak status,
  and checks mode, build/Java provenance, host/account identities, readiness,
  failure/disconnect counters, and latency reservoirs.
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
- Validate exchange time, enforce exact API-key permissions and required
  exchange-reported IP binding, continuously compare authenticated account
  config to its bootstrap identity/key-security/settings, poll announced OKX
  unified-account system
  maintenance, compare strategy-critical instrument rules and fee assumptions
  to authenticated current metadata, expire stale place requests at the venue,
  and own an independently scheduled exchange deadman lifecycle per account.
- Expose a separate minimal-config emergency composition that bypasses strategy,
  journal, websocket, and operator dependencies while cancelling and verifying
  the regular, algo, and spread pending-order domains account-wide.

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
stablecoin reference, every required strategy reference, and all configured
private channels for every account. Live validation requires an explicit
`strategy.reference_data_stale_threshold_ms`. The strategy derives one typed
requirement set from that policy: price limits for every OKX instrument, mark
price for derivatives, funding rate for swaps, and every configured index.
`reap-live` maps those venue-neutral requirements to separate critical OKX
subscriptions, matching the pinned Java subscriber's separate PriceRange,
MarkPrice, FundingRate, and index sessions. Unlike Java's shared mutable
`Ticker.timeMs`, each Rust component retains its own non-regressing source
timestamp, so activity on one channel cannot mask another channel's silence.
Startup evaluates source age against host receive time; an old retained frame
cannot open readiness. Missing or stale input degrades readiness, blocks entry,
and immediately synthesizes canonical account-wide cancels. The pure strategy
also removes stale instruments and withdraws quotes on its timer, so live and
backtest decisions share the same freshness behavior.
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
per-account deadline, separate regular/spread deadman arms, bounded request
pacing, regular/algo batch cancellation, spread mass cancel, and strict repeated
pagination of all three pending-order domains. This separation keeps the kill
path usable after live-process or strategy-state failure. Its optional CLI
evidence path is reserved before config or network work and fsynced before exit.
The schema-versioned report hashes the
exact input file without invoking the live parser, uses the same executable and
host identity hashes as live evidence, samples the same pseudonymous OKX
UID/main-UID account identity only after zero is proven, records the pinned Java
revision, and converts account-task join failures into bounded evidence.
`all_clear` requires regular, algo, spread, and aggregate account-wide zero plus
complete provenance; an early parse/validation failure leaves only an empty
reserved path. OKX documents no algo-order CAA, so this boundary requires the
explicit producer-stop confirmation before it cancels and polls algo orders.

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
- Bound local/upstream websocket handshakes and each forwarding write by the
  configured request timeout, make them shutdown-cancellable, and bound paired
  close writes. Every registered bridge is removed on clean, protocol-error,
  timeout, and shutdown exits so process evidence cannot retain a phantom
  active connection.
- Accept the proxy configuration only as a non-symlink regular file and
  canonicalize its source before resolving artifact paths or deriving the
  effective fingerprint. Relative and absolute invocations of the same config
  therefore produce independently comparable run evidence.
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

The CLI first validates the exact config and run options and captures source,
executable, and optional host provenance. It then reserves a create-new live
evidence path before credential or network work. A handled build failure after
reservation becomes a source-bound pre-session report: no session ID, account
identity, host-health observation, or runtime counters are claimed, baseline
readiness is retained, and clean-soak acceptance is impossible. Zero-valued
startup counters are explicitly not exchange-zero proof. Startup task guards
abort raw feed, order, reconciliation, safety, and order-status workers unless
their ownership transfers into the completed runtime. After the report-capable
runtime exists, every initialization, event-loop, or teardown error follows the
same cancel/reconcile/shutdown path and then returns a schema-versioned report
containing the primary stable failure code, bounded diagnostic, pre-cleanup
readiness snapshot, post-cleanup active-order count, and all accumulated
evidence. The report is persisted before the original error produces a nonzero
exit. An empty or incomplete reserved path therefore indicates persistence
failure or forced process termination, not a valid startup report.

`runtime.shutdown_timeout_ms` bounds canonical cancellation and authoritative
REST reconciliation. The separate `runtime.teardown_timeout_ms` bounds all task
joins, websocket owners, host/operator services, journal flush plus `sync_data`,
and the nested alert drain. Cancellation-safe owner destructors signal and abort
their child tasks if that future is dropped. Production evidence requires both
runtime deadlines to total at most 40 seconds under the hardened 45-second
systemd stop boundary, reserving five seconds for report durability and exit.

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

This is still research accounting, not an exchange statement. A candidate may
provide one strict account-style `[initial_portfolio]`: a complete set of
non-negative balances plus derivative quantity, average cost, and margin mode.
Spot base balances name one valuation instrument so the ledger cannot count the
same asset as both cash and inventory. The runner seeds that snapshot into both
the portfolio and `ChaosStrategy` at the first replay timestamp. It blocks order
entry until every configured book and direct currency rate can establish an
opening valuation; a fill before that baseline aborts replay. Reports retain the
exact snapshot, opening valuation time/equity, reconstructed ending account
balances, arithmetic linear and harmonic inverse position averages, final
equity, and `net_pnl_usd = final - opening`. Post-fill account events update both
spot balances and derivative positions rather than leaving opening balances
stale. Private normalized fills can carry exact signed fee and currency
evidence; those fees are booked in spot base/quote balances, inverse settlement
coin, or the reported third currency as applicable. Public-data
matching has no account fee event, so it estimates fees from configured
maker/taker rates. Reports expose exact and estimated fee-fill counts. The
opening model intentionally supports one account. Manual snapshots default
available/equity to cash total, while certified snapshots retain OKX
`availBal`, `eq`, `maxLoan`, forced-repayment indicator, `mgnRatio`, `adjEq`, and
`notionalUsd`. Post-fill updates and a deterministic 10-second publisher send
full balances, derivative positions, native-currency mark-to-market equity, and
recalculated margin state to the strategy. The periodic path waits behind any
latency-delayed fill-account update, so it cannot reveal a fill early. The
calculated margin ratio is equity divided by derivative notional. The simulated
exchange ratio follows pinned Java `PortfolioExchAcctCalculator`: equity divided
by notional-over-leverage, multiplied by its unusually named
`EXCH_CMR_RATIO_DISCOUNT` value `50`; the Rust configuration therefore calls it
`exchange_cmr_multiplier`. Zero notional and zero liability omit the ratio rather
than creating an unsafe infinity. This remains a research margin model, not OKX
tier, collateral-discount, liquidation, or interest accounting. Cash total
remains non-negative at accepted carry boundaries and liability must be zero
because the model does not support borrowing interest, liquidation, venue
margin discounts, or tax. It cannot
infer a missing funding event when the source dataset never contained one. A
separate offline production-evidence path
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
- Dataset IDs and copied byte-identical content cannot be reused to disguise
  train/test leakage; event-time windows must be non-overlapping and strictly
  chronological. One canonical raw file may appear more than once only through
  disjoint explicit process-global record ranges.
- Production raw captures must name their capture-only config and schema-5 run
  report. The report verifier binds exact config bytes, capture executable,
  host, pinned Java revision, host-guard evidence, and effective output overrides
  to raw and optional reconstructed normalized bytes. It cross-checks replayable
  counters, session, and book health, and retains runtime-only stop, readiness,
  and queue evidence. Production research requires the same executable and the
  latency-calibrated target host plus at least one completed periodic host
  check. Its capture guard is also constrained by code-level policy: enabled,
  no slower than 10 seconds, at least 5 GiB available disk, at least 1 GiB
  available memory, and mandatory synchronized-clock enforcement. A clean
  diagnostic capture with weaker thresholds is not production evidence. The
  report must pass alongside a one-session,
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

Schema 8 retains independent runs from zero state
(`independent_zero_initial_portfolio`), a candidate snapshot
(`independent_configured_initial_portfolio`), or a dataset certification
(`independent_certified_dataset_portfolio`). It also supports
`sequential_settled_carry` for explicit ranges of one raw capture. A continuation
names `continuation_of`, uses the same canonical raw/config/report/normalized
evidence as its parent, begins at exactly the next `capture_record_seq`, stays in
the same non-empty capture session, and cannot regress receive time. Ranges must
be disjoint and each parent has at most one child. Replay warms deduplication and
book reducers from the immutable parent prefix, restores the latest index,
funding, mark, and price-limit state, then emits ready boundary books before
selected deltas. Burst and trade events are not replayed as snapshots. This lets
the fresh strategy instance receive stateful subscription equivalents before it
can quote without inventing historical executions.

Each chain range starts a fresh strategy and matcher, as Java starts a new run.
At the boundary Rust marks the portfolio to terminal depth/exchange marks, sets
account balances and availability to equity, resets nonzero derivative average
costs to those marks, recomputes margin, and carries current currency
observations plus pending and settled funding watermarks. Java `tryToStop()`
cancels orders before `finishUp()`; correspondingly, delayed non-funding actions
and resting orders remain visible in the source report but are not installed in
the next runner. A terminal strategy safety halt or incomplete accounting
prevents carry.

Training sequences must begin from a declared root. A test sequence may begin
independently or immediately continue the selected candidate's final training
range; non-selected candidate state never enters held-out evaluation. Every test
stress scenario starts from that same selected baseline economic state and
rebinds only validated execution-dependent margin fields. Aggregation sums each
range's opening-adjusted `net_pnl_usd`, never final account equity.

Schema-8 production candidate files omit opening capital. Every independent
dataset or chain root references one unique certification collected before the
capture and supplies an explicit currency-to-spot valuation mapping;
continuations omit opening certification. Research independently rebuilds the
embedded raw OKX responses and requires a passing cash/zero-liability policy,
production environment, identical Reap executable and pinned Java revision, the
latency-calibrated capture host, one account-bound candidate instrument universe,
a bounded certification-to-capture gap, and no nonzero unmodeled currency or
position. The derived root portfolio must be identical for every candidate. A
schema-8 `production_candidate` manifest additionally requires one predeclared
deployment candidate, at least three folds, two stress scenarios, nonzero event,
fill, and duration gates, calibrated baseline execution whose latency profile
exactly matches a passed source-bound calibration artifact, complete accounting,
and explicit bounds on non-funding work censored by each data horizon. Stress
scenarios may use explicitly uncalibrated deterministic haircuts.

The certified opening snapshot remains point-in-time evidence, not an exchange
statement or an atomic venue tick. The account must remain quiescent between
certification and capture, and no passing target-account artifact exists yet.
Carry is permitted only inside one verified process session and canonical raw
file. Restarted collectors reset session identity and the process-global ordinal;
cross-process or independently rotated files remain separate until rotation can
preserve both values without an event gap.

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
manifest, and compares the full report after normalizing only source paths
introduced by capture and opening-account verification to their content hashes. Unknown, duplicate,
omitted, stale, forged, non-passing, or numerically different results fail closed. The
verification artifact carries the exact source hashes and a bounded first
difference diagnostic; it remains simulation evidence rather than production
authorization.

The format-3 research verifier also derives one deployment candidate ID,
the dataset opening-account identity/build/host summaries, and
effective strategy hash from the report, requiring exactly one matching
candidate provenance row and the same training-selected ID in every production
fold. `verify-research-deployment` then loads an exact production `LiveConfig`,
requires every opening certification's embedded config SHA-256 to match those
exact bytes, hashes `strategy.effective()` through the same function used by
research, and requires equality. This closes the research-to-live strategy
identity boundary;
it deliberately does not aggregate or replace host, account, transition, fault,
statement, deadman, or emergency evidence.

### `reap-capture`

Credential-free public market-data composition.

Responsibilities:

- Build explicit public-only subscription plans from TOML.
- Load at most 16 MiB from one canonical regular config file, reject symbolic
  links, and retain that canonical source path with the exact byte evidence.
- Provide one reviewed production-shaped public config with an absolute shared
  connection pacer; systemd reuses its exact bytes and supplies unique
  create-new raw/report paths for each capture instance.
- Capture the stablecoin/index references required to reproduce live risk
  inputs alongside strategy market data.
- Run redundant websocket connections through `reap-feed` supervision.
- Persist every received raw frame through a bounded lossless writer. A full
  writer queue has a one-second deadline and fails the run instead of dropping a
  frame or blocking the event loop indefinitely.
- Assign a process-global record ordinal before that writer and independently
  require the persisted artifact to contain exactly `1..raw_records`; this
  proves writer-boundary completeness separately from venue book sequences.
- Fail before network startup and during collection when Linux disk, available
  memory, or kernel clock state breaches configured thresholds.
- Stamp every frame with a process-session identity so replay cannot hide
  capture downtime.
- Fingerprint the effective capture config and exact persisted bytes.
- Validate the effective configuration before reserving the create-new report.
  After reservation, convert handled setup/runtime/teardown failures into a
  typed non-clean report; never adopt bytes from a pre-existing capture output.
- Bound writer flush/sync shutdown at 30 seconds, cancellation wait at one
  second, and best-effort failure-evidence scanning at five seconds. Bound feed
  shutdown/drain and host-guard shutdown at five seconds each; the host and
  writer phases run concurrently. Retain systemd's 45-second stop deadline as
  the out-of-process hard boundary.
- Reserve and fsync a versioned run report that also binds the exact source
  config file, executable, host, pinned Java revision, and host-health evidence.
- Optionally persist deduplicated normalized events for short diagnostics.
- Report connection readiness, queue high-water marks, parser failures,
  duplicates, gaps, recoveries, and final book health.
- Classify stream identity once for runtime and offline analysis. A clean run
  requires every configured logical stream to produce data from its exact
  deterministic replica/chunk socket-plan IDs and at least one accepted event;
  a count-equivalent wrong source, unclassified frame, or unexpected data
  stream fails the same contract.
- Stream raw files through the live adapter/reducer path to measure configured
  source coverage, receive/exchange timing, depth, spread, movement, and trade
  distributions without retaining the dataset in memory.
- Verify a retained run report against relocated raw/normalized artifacts,
  including exact normalized output reconstructed from raw replay.
- Reject every report carrying typed runtime failure evidence from independent
  verification and production research, regardless of its reported clean flag.

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

Each socket builds a unique expected subscription-argument set from the exact
serialized request before opening the connection. It publishes `Ready` only
after OKX has acknowledged every channel and selector, including `instId` or
`instType`. Counting acknowledgement frames is insufficient because a
retransmitted acknowledgement for one instrument must not conceal a missing
acknowledgement for another instrument. This follows pinned Java
`OkxNitroSubscriberBase.tryParseNonData`, where `EVENT_SUB` updates the context
selected by the returned `WsSubArg`, while making malformed and unexpected
success frames explicitly fail closed.

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

The adapter parses raw payloads into normalized candidate events. Global dedup
classifies exact redundant images for reporting, but every book image also passes
through deduplication scoped by `conn_id` and then that socket's sequence tracker
and full-book reducer. This mirrors the pinned Java subscriber's
`sessionType/connectionIdx/symbol` ownership and prevents asynchronous snapshots
from one replica from advancing another replica's predecessor state.

Canonical arbitration sees only valid reconstructed books. A newer exchange
timestamp becomes canonical. At the same timestamp and sequence, an equivalent
full book is an ignored replica and a different full book is an integrity
conflict. Same-timestamp forward predecessor continuity can advance the
canonical book; a lagging reverse transition is ignored. A source-local gap
requests a fresh snapshot from only that `conn_id`; another ready source keeps
the canonical book available. An aggregate stale state or replica conflict
fails closed and recovers all relevant book sockets.

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
  a byte-identical image from another socket is a global duplicate. It still
  initializes or advances that socket's independent sequence/book state.
  Conflicting content reaches canonical full-book arbitration and forces
  recovery of every conflicting source.
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
supervised socket plans, process-shared handshake pacing, bounded deduplication,
sequence recovery, and the raw capture checker.

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

Production-transition evidence makes the operational baseline non-optional:
both exact demo and production configs require fatal alerts, the operator
service, production host-guard floors, redundant public and order-command
sessions, and an absolute process-shared connection pacer. Runtime exercise and
external delivery remain deployment evidence rather than config claims.

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
- Add strategy-independent account-wide regular/algo/spread cancellation and
  hardened mode-specific process supervision templates. Done; exact
  config/report bytes and all-configured-account, per-domain zero invariants are
  independently verifiable. Target-host/account fault exercise and raw
  REST-response replay remain; algo safety has no documented venue deadman and
  therefore also requires the external producer-stop attestation.
- Add independently routed demo fault injection with durable typed evidence.
  Done. Execute and accept the credentialed target-host campaign and OKX demo
  soak; tooling alone does not close that gate.

The implemented Phase 8 surface is a record of current capability, not a grant
of permanent authority to the Chaos process. In particular, broad emergency
coverage, generic endpoint support, and Java-inspired connection pool size are
being separated from normal strategy connectivity by the
[Chaos connectivity refactor](chaos-connectivity-refactor-plan.md).

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
