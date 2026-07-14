# Reap Migration TODO

This list tracks the migration from the current Rust scaffold toward the target
architecture in [docs/architecture.md](docs/architecture.md).

## Step 1: Workspace Split

- [x] Convert the single crate into a Cargo workspace.
- [x] Create `crates/reap-core` for shared market/order/config primitives.
- [x] Create `crates/reap-strategy` for pure chaos strategy logic.
- [x] Create `crates/reap-backtest` for replay, matching, and reports.
- [x] Create `crates/reap-cli` for user-facing binaries.
- [x] Preserve current CLI behavior:
  `cargo run -p reap-cli -- backtest --config examples/iarb2-basic.toml --data examples/market.csv --pretty`.
- [x] Preserve current tests and sample backtest output shape.

## Step 2: Normalized Event Boundary

- [x] Introduce `NormalizedEvent`, `StrategyEvent`, and `OrderIntent`.
- [x] Make backtest and live designs use the same strategy input/output types.
- [x] Remove direct strategy coupling to matcher-specific structs.
- [x] Add deterministic replay fixtures for quote and hedge decisions.

## Step 3: Book And Order Reducers

- [x] Add `reap-book` with single-writer book reducers.
- [x] Add book status states: empty, recovering, ready, stale, gapped.
- [x] Add `reap-order` with canonical order state reducer.
- [x] Add idempotent order update/fill handling tests.
- [x] Update backtest to use the shared reducers.

## Step 4: Live Feed Skeleton

- [x] Add `reap-venue` for exchange-specific parsers and request builders.
- [x] Add `reap-feed` for websocket supervision.
- [x] Implement multi-websocket subscription partitioning.
- [x] Implement deduplication by channel-aware event ids.
- [x] Implement sequence checks and snapshot recovery.
- [x] Add raw replay checker for captured websocket data.

## Step 5: Live Order Gateway

- [x] Implement signed submit/cancel for one venue.
- [x] Add private websocket order/fill/account reducer.
- [x] Add REST reconciliation for open orders and fills.
- [x] Add idempotent client-order-id handling.
- [x] Add rate-limit and request pacing policy.

## Step 6: Production Hardening

- [x] Add pre-trade and post-trade risk gates.
- [x] Add kill switch and manual symbol halt events.
- [x] Add stale feed/private stream fail-closed policy.
- [x] Add structured telemetry and health metrics.
- [x] Add storage for raw events, normalized events, intents, orders, and fills.
- [x] Profile the hot path before lower-level queue/thread optimizations.

## Step 7: Chaos Iarb2 Decision Parity

- [x] Cross-check Rust configuration, pricing, hedging, skew, funding, account
  risk, and stop behavior against Java `chaos-iarb2` source and fixtures.
- [x] Reject one-symbol and self-only hedge topologies before startup.
- [x] Cover spot, linear, and inverse quote/hedge vectors with parity tests.
- [x] Make account/position updates authoritative for hedge triggers.
- [x] Add funding, index, mark/limit, account, margin, and position events.
- [x] Preserve quote/hedge identity across REST and private websocket races.
- [x] Scope private feed health by account and fill deduplication by
  account/instrument, including scoped restart keys and legacy journal migration.
- [x] Document execution-policy and platform differences without claiming
  Binance or Java control-plane parity.

## Step 8: Demo-Tradable Runtime

- [x] Add a live composition crate/command with one strategy event-loop owner.
- [x] Verify exchange instrument metadata, account mode, trade mode, and risk
  valuation before subscriptions are considered ready.
- [x] Implement the complete startup and restart reconciliation state machine.
- [x] Wire accepted intents through registered submit/cancel and private
  feedback into the engine.
- [x] Add end-to-end fault tests for disconnects, gaps, duplicates, partial
  fills, IOC misses, ambiguous submits, rate limits, and restart recovery.
- [x] Profile the parity event loop with production-shaped captures and remove
  measured hot-path allocation/collection bottlenecks.
- [x] Add bounded observe/demo soak execution with machine-verifiable readiness,
  drift, storage, and shutdown evidence.
- [x] Make every demo exit disable new submits while preserving cancellation,
  then require zero active orders and post-cancel REST reconciliation even when
  storage has failed.
- [x] Add a bounded Unix-socket operator service with environment-keyed HMAC,
  freshness/replay checks, status, global/account kill, symbol halt/resume, and
  graceful stop.
- [x] Persist operator and risk latches as write-ahead, fsynced journal records;
  restore them before live readiness and do not treat normal shutdown as a
  persistent kill.
- [ ] Complete an OKX demo soak with no unexplained reconciliation drift.

## Step 9: Production Confidence

- [x] Add credential-free redundant OKX public capture and direct raw-capture
  replay/backtest through the live adapter, deduplication, sequence, and book
  reducer path.
- [x] Make OKX book continuity predecessor-based and reset-aware; preserve
  no-change updates, deduplicate byte-identical images across sequence epochs,
  fail closed on conflicting replicas, and cover reset/recovery with raw replay
  fixtures.
- [x] Give capture output create-new semantics with per-run CLI path overrides,
  then complete a real public capture, strict replay, and raw backtest smoke.
- [x] Add exact raw/config fingerprints and streaming capture analysis for
  per-subscription source coverage, timing, depth, spread, movement, and trade
  distributions; verify its strict gate against a fresh public capture.
- [x] Add a deterministic Java-referenced backtest scheduler with receive-time
  raw replay, explicit market/new/cancel/order/account delays, immediate
  `PendingNew` registration, and end-of-horizon execution provenance.
- [x] Port Java's conservative displayed-depth fill threshold, including its
  shallow-cross queue-ahead reset semantics and application default.
- [x] Add reportable queue-ahead, historical-trade participation, and
  displayed-depth capacity controls for conservative execution sensitivity.
- [x] Add manifest-driven walk-forward candidate selection, chronological
  train/test isolation, conservative sensitivity scenarios, deterministic
  provenance, and machine-verifiable accounting/risk/performance gates.
- [x] Add bounded deterministic empirical latency profiles by Java-mapped
  message class and instrument, actual sampled-usage reporting, and
  stochastic-dominance validation for coupled baseline/stress runs.
- [x] Collect bounded live latency evidence at the normalized strategy boundary,
  REST acknowledgement boundary, and fill-to-account convergence boundary;
  generate config/code/source-bound calibration artifacts and require an exact
  artifact/profile match for production-candidate research.
- [x] Reserve bounded-live evidence output before network startup and persist a
  schema-versioned failure report after fail-closed runtime/teardown cleanup,
  while preserving the original nonzero process exit.
- [x] Attribute fill fees/turnover and port signed linear/inverse swap funding
  settlement with mark fallback, late/missed coverage signals, and explicit
  end-of-horizon funding actions.
- [x] Add explicit quote/settlement currency ledgers and latency/freshness-aware
  direct USD index valuation for fills, fees, funding, positions, and active
  orders; fail production research on missing conversion evidence.
- [x] Add Java-referenced USDT/USDC depeg protection with redundant critical
  index feeds, conflict-aware deduplication, startup readiness, immediate entry
  blocking, and a debounced durable global risk latch.
- [x] Require account snapshots to pass through strategy/risk before readiness,
  and reject live master/group topology until its external Java coordination
  feed exists.
- [x] Separate private transport liveness from account-state freshness: require
  real account and positions payload rounds, and prevent pongs or event-only
  order/fill traffic from masking a stale state channel.
- [x] Reconcile REST balances and positions with websocket-derived state on
  startup, recovery, ambiguity, and shutdown; apply omitted-row tombstones,
  reject stale account rows, and require a second clean pass after repair.
- [x] Require local submits and dispatched cancels to converge through the
  event-only OKX orders channel or REST recovery within a configured deadline;
  retry cancellation and reconcile fail-closed on timeout.
- [x] Enforce configured global and per-symbol active-order count ceilings in
  both projected pre-trade state and authoritative post-trade order state.
- [x] Add Java-referenced rolling global/per-symbol exchange submit-rejection
  thresholds that persist the global risk latch and cancel active orders.
- [x] Add a deduplicated rolling per-symbol zero-fill IOC cancellation threshold
  that preserves local time-in-force through canonical private order updates,
  persists the global risk latch, and cancels active orders.
- [x] Promote every terminal chaos strategy halt into the engine-owned global
  risk path before intent dispatch, persist the latch, and make global
  cancellation scope override simultaneous symbol-only isolation.
- [x] Bind REST submit/cancel acknowledgements into canonical exchange/client
  order identity, recover the one-to-one bindings from the journal, resolve
  empty/`0` private IDs consistently across live and reconciliation, and reject
  wrong-account or immutable symbol/side changes before state mutation.
- [x] Suppress OKX private order duplicates by fill ID or unchanged terminal
  order state even when repeated channel messages carry a different update
  timestamp.
- [x] Require every canonical derivative fill to converge to its position row
  and every spot fill to both currency balances within a configured deadline;
  fail closed and reconcile the account on timeout.
- [x] Retain OKX position `mgnMode`, reject configured derivative margin-mode
  mismatch at bootstrap and runtime, and compare the field during REST
  reconciliation.
- [x] Enforce the modeled live-account boundary: cash-only spot routing and no
  nonzero position outside the configured account's derivative universe.
- [x] Retain OKX balance `twap`, enforce a configured forced-repayment indicator
  limit at bootstrap/runtime, clear it with authoritative tombstones, and
  compare it during REST reconciliation.
- [x] Reject crossed books and force fresh-snapshot recovery, matching the
  reviewed Java OKX subscriber behavior with explicit sequence validation.
- [x] Add order `expTime`, startup/periodic exchange-clock validation, and an
  independently scheduled OKX Cancel All After heartbeat for demo order entry.
- [x] Pin the Java strategy/connectivity audit to `imm-strategy` commit
  `b6b120c7b7c466d8431bf082f3229328c5d7b2ae`.
- [x] Acquire an exclusive canonical journal lease before recovery,
  credentials, or network setup; retain it for the full runtime lifecycle.
- [x] Add bounded asynchronous webhook alerts and Linux disk, memory, and
  kernel-clock preflight/periodic guards with fail-closed runtime integration.
- [x] Add a strategy-independent, explicitly confirmed OKX emergency command
  that arms Cancel All After, cancels regular orders account-wide, and proves
  zero after the trigger horizon; document the algo/spread exclusion.
- [x] Make emergency-cancel evidence create-new and fsynced, bind it to a
  schema, exact config file, binary, host, Java revision, and the same
  pseudonymous exchange-account identity as live reports; turn account task
  failures into bounded non-passing evidence instead of losing the report.
- [x] Add hardened systemd templates with bounded observe restart and manual
  demo/capture restart, plus the stop/cancel/reconcile operating procedure.
- [x] Persist current OKX order-channel fills and exact per-fill fees once across
  private channels, paginate recent-fill recovery to a proven short page with a
  fail-closed bound, and add authenticated create-new collection plus verified
  journal-to-raw-OKX fill/fee evidence.
- [ ] Run credentialed bounded observe and minimal-size demo fault campaigns,
  including process death, deadman expiry, clock skew, REST ambiguity, partial
  fill, reconnect, and durable-latch restart recovery.
- [ ] Calibrate queue position, latency, fees, funding, and slippage from
  captured full-depth data, then run and archive production-candidate
  walk-forward, capacity, and stressed-liquidity reports. The orchestration and
  configurable execution model and latency evidence pipeline are implemented
  but have no credentialed calibration artifact; empirical queue and populated
  per-class/per-instrument latency distributions, target-tier simulated-fee
  calibration, complete funding intervals, zero-liability cash-account
  certification, a passing real authenticated fill/fee reconciliation artifact,
  and broader economic statement reconciliation are still required. Exact signed
  private-fill fee amount/currency is retained end to end; collection and
  verification are implemented but no demo evidence exists. Margin spot is
  unsupported and would require a borrow-interest model before enablement.
- [ ] Deploy and exercise the webhook/host guards, systemd supervision, external
  unit-failure paging, and independent exchange cancel procedure on the target
  host/account; add a separate algo/spread kill path if those order classes are
  enabled.
- [ ] Certify production credentials/account mode/limits and expose production
  order entry only after every gate in `docs/trading-readiness.md` is signed off.

Completed-step evidence is documented in [README.md](README.md),
[docs/operations.md](docs/operations.md), and
[docs/performance.md](docs/performance.md). The workspace test, lint, replay,
configuration, backtest, and benchmark commands are the acceptance gates.
The remaining path to trading is tracked in
[docs/trading-readiness.md](docs/trading-readiness.md).
