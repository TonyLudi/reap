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

- [x] Add credential-free redundant OKX public capture, a reusable
  production-shaped absolute-path config with per-instance systemd outputs, and
  direct raw-capture replay/backtest through the live adapter, deduplication,
  sequence, and book reducer path.
- [x] Make OKX book continuity predecessor-based and reset-aware; preserve
  no-change updates, deduplicate byte-identical images across sequence epochs,
  fail closed on conflicting replicas, and cover reset/recovery with raw replay
  fixtures.
- [x] Scope OKX sequence trackers and full-book reducers by websocket `conn_id`,
  let global duplicates advance every source independently, arbitrate canonical
  reconstructed books, and route a source-local gap only to its failed socket.
- [x] Give capture output create-new owner-only semantics with per-run CLI path
  overrides, then complete a real public capture, strict replay, and raw
  backtest smoke.
- [x] Add exact raw/config fingerprints and streaming capture analysis for
  per-subscription source coverage, timing, depth, spread, movement, and trade
  distributions; verify its strict gate against a fresh public capture.
- [x] Reserve and fsync a versioned capture run report, bind exact config bytes
  plus effective CLI overrides, and independently verify raw and reconstructed
  normalized artifacts; exercise the gate on a fresh public capture.
- [x] Upgrade capture evidence to schema 5 with exact Reap build, pinned Java
  revision, pseudonymous host identity, Linux disk/memory/clock preflight and
  periodic enforcement, and a process-global persisted-frame ordinal that
  rejects missing, duplicated, skipped, or reordered writer records. Bind
  production research to the same binary and latency-calibrated host, and make
  the systemd collector bounded and create-new-report strict.
- [x] Enforce production-candidate capture host-guard floors in code: enabled,
  checks no slower than 10 seconds, at least 5 GiB available disk and 1 GiB
  available memory, and mandatory synchronized-clock enforcement. A lowered
  diagnostic threshold can no longer qualify solely because capture was clean.
- [x] Canonicalize and bound capture-config provenance, and require every
  configured logical stream to deliver data on its configured replica count
  plus at least one accepted event before runtime or offline verification can
  classify the capture as clean.
- [x] Bind each capture stream to the exact deterministic websocket plan IDs
  produced by subscription partitioning; count-equivalent data from a wrong
  replica/chunk now fails runtime clean status and offline verification.
- [x] Bound capture-writer enqueue, feed/host/writer teardown, cancellation, and
  failure-evidence scanning; validate before reserving the report, persist typed
  non-clean reports for handled failures, avoid adopting older output bytes, and
  reject failure reports from verification and production research.
- [x] Add a deterministic Java-referenced backtest scheduler with receive-time
  raw replay, explicit market/new/cancel/order/account delays, immediate
  `PendingNew` registration, and end-of-horizon execution provenance.
- [x] Port Java's conservative displayed-depth fill threshold, including its
  shallow-cross queue-ahead reset semantics and application default.
- [x] Add reportable queue-ahead, historical-trade participation, and
  displayed-depth capacity controls for conservative execution sensitivity.
- [x] Close raw-replay risk integrals at the persisted raw-record horizon, use
  that same horizon for observed duration, and fail if inventory-open time can
  exceed the denominator used by research gates.
- [x] Add manifest-driven walk-forward candidate selection, chronological
  train/test isolation, conservative sensitivity scenarios, deterministic
  provenance, and machine-verifiable accounting/risk/performance gates.
- [x] Add bounded deterministic empirical latency profiles by Java-mapped
  message class and instrument, actual sampled-usage reporting, and
  stochastic-dominance validation for coupled baseline/stress runs.
- [x] Collect bounded live latency evidence at the normalized strategy boundary,
  exchange order-acknowledgement boundary, and fill-to-account convergence boundary;
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
- [x] Derive one venue-neutral strategy reference contract for index, swap
  funding, derivative mark, and per-instrument price limits; map it to separate
  critical OKX sockets, require source-time freshness against live receive time,
  keep component clocks independent and non-regressing, and cancel canonical
  orders immediately when readiness loses a required source. This follows the
  pinned Java session topology while removing its shared `Ticker.timeMs` masking.
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
  that arms regular and spread Cancel All After, exhaustively enumerates and
  cancels regular, algo, and spread orders account-wide, and proves every
  domain zero after the trigger horizon.
- [x] Make emergency-cancel evidence create-new and fsynced, bind it to a
  schema, exact config file, binary, host, Java revision, and the same
  pseudonymous exchange-account identity as live reports; turn account task
  failures into bounded non-passing evidence instead of losing the report.
- [x] Add strict credential-free emergency-cancel verification that re-hashes
  exact config/report bytes, re-derives selected/configured account coverage,
  per-domain deadman/trigger-horizon and final-zero invariants, and can require
  every configured account without claiming raw exchange-response replay.
- [x] Paginate regular OKX pending orders during normal reconciliation and all
  three pending-order domains during emergency cancellation with strict page,
  cursor, duplicate-ID, and terminal-short-page bounds.
- [x] Restrict authenticated live and emergency endpoints to exact documented
  OKX Global, US/AU, EEA, or Turkey environment tuples; reject arbitrary TLS
  hosts and permit cleartext loopback only for complete demo-test tuples.
- [x] Add an exact-file demo-to-production configuration transition verifier
  that permits only reviewed endpoint/deployment bindings and rejects strategy,
  risk, runtime, account-policy, execution, storage-capacity, or safety drift.
- [x] Make production transition policy explicitly reject disabled/non-fatal
  alerts, a disabled operator service, weak host-guard resource/clock floors,
  fewer than two public or order-command sessions, and a non-absolute shared
  connection pacer in either exact config.
- [x] Reject ignored fields throughout live TOML parsing so nested configuration
  typos cannot silently default or evade production-transition comparison.
- [x] Add hardened systemd templates with bounded observe restart and manual
  demo/capture restart, hermetic syntax/mode/security verification in CI, and the
  stop/cancel/reconcile operating procedure.
- [x] Persist current OKX order-channel fills and exact per-fill fees once across
  private channels, paginate recent-fill recovery to a proven short page with a
  fail-closed bound, and add authenticated create-new collection plus verified
  journal-to-raw-OKX fill/fee evidence.
- [x] Add authenticated account-wide OKX bill collection with exact raw-page and
  cursor reconstruction, then reconcile normal trade and realized funding bills
  against verified fills and a streaming pass over the stopped journal. Bind one
  passing economic gate per demo account into production evidence schema 8,
  preserve the pinned Java bill field/type mapping, and independently bracket
  each funding assessment with journaled mark-price observations bounded by an
  explicit per-account runtime-session start record on every restart.
- [x] Persist every authoritative REST account replacement as a critical
  account-scoped schema-7 journal record, bind derivative bills to exact
  critical fills, reconstruct linear/inverse average entry using the pinned
  Java arithmetic/harmonic rules, and independently verify realized close PnL.
  Production evidence requires a nonzero reviewed derivative-close count.
- [x] Pin the Rust toolchain and add least-privilege GitHub CI for formatting,
  all-target lint, workspace tests, release build, and RustSec audit, with weekly
  Cargo and Actions update proposals.
- [x] Add mode-aware OKX account economics, reject live borrowing/nonzero
  liability at bootstrap and during account updates, continuously detect
  authenticated account-config drift, and add create-new point-in-time account
  certification with credential-free raw-evidence verification.
- [x] Retain current OKX API-key label, permissions, and IP bindings; enforce an
  exact per-account scope at bootstrap, forbid `withdraw`, require IP-bound trade
  keys for production, detect periodic drift, and independently re-derive the
  policy in schema-3 account certification and aggregate production evidence.
- [x] Make websocket ready/disconnect transitions lossless under bounded status
  backpressure, remove redundant per-frame status traffic, and split public and
  private disconnect counts in schema-8 live evidence for fault campaigns.
- [x] Bind websocket readiness to the exact unique subscription-argument
  acknowledgement set in each socket plan, matching Java's `WsSubArg` context
  transition. Duplicate acknowledgements cannot hide a missing subscription,
  malformed or unexpected acknowledgements reconnect, and duplicate local plans
  fail fatally before network startup.
- [x] Pace public/private/order-command and fault-proxy-upstream initial and
  reconnect handshakes through one owner-only, process-shared host file under
  the documented OKX per-IP limit; production evidence requires one absolute
  demo/production path. Reject unknown capture fields and gate backtest order
  entry on complete books plus fresh accounting routes. Hosts sharing one NAT
  still require external IP-wide coordination or isolated egress.
- [x] Bound feed websocket handshakes and control writes, make establishment
  shutdown/recovery cancellable, retain a recovery-channel owner for every
  socket including non-book plans, and treat unexpected owner loss as fatal.
- [x] Reset exponential feed reconnect backoff after exact subscription
  readiness, and bound lossless private-feed queue delivery so a stalled event
  loop forces typed disconnect/reconciliation instead of blocking websocket
  heartbeat and shutdown indefinitely.
- [x] Add a separately configured whole-runtime teardown deadline after bounded
  cancel/reconcile, signal every task owner before awaiting it, abort rather than
  detach remaining tasks, retain typed failure evidence, preserve the exchange
  deadman, sync the stopped journal, and reserve report/exit time inside the
  hardened systemd stop budget.
- [x] Keep order-command heartbeat telemetry best-effort under bounded queue
  saturation while preserving lossless disconnect/fatal transitions; bound its
  shutdown close and the fault proxy's client/upstream handshakes and writes,
  with connection unregister guaranteed on every bridge exit. Canonicalize and
  regular-file-check the proxy config so relative runbook invocations retain an
  independently verifiable effective fingerprint.
- [x] Add structured live fault evidence for ambiguous submit/cancel outcomes,
  partial fills, order/fill convergence timeouts, restored durable latches, and
  typed deadman, periodic clock, and authenticated account-config failures.
- [x] Add read-only process-death certification that leases and fingerprints
  the stopped journal, requires every recovered live regular order to report
  OKX Cancel All After source `20`, proves account-wide regular-order zero, and
  supports credential-free verification against the exact journal/raw evidence.
- [x] Bind schema-8 live reports to exact source-config bytes, reserve them
  owner-only with file/directory durability, and add offline verification that
  re-derives report, mode, host, identity, latency, and clean-soak invariants.
- [x] Validate live config/run options and bind source/build/host provenance
  before report reservation; persist handled runtime-build failures as typed
  pre-session diagnostic reports without account/runtime claims, keep them
  ineligible for acceptance, and abort untransferred startup task owners.
- [x] Add a strict live fault-matrix manifest/verifier that requires every
  documented schema-8 role, unique sessions and injector evidence, one exact
  config/build/host/account identity, production-shaped demo guards, and safe
  zero-order shutdown without claiming process-death or economic proof.
- [x] Add a loopback-only OKX demo REST/public/private/order-command fault proxy
  with strict owner-local control, independently routed order sessions,
  deterministic disconnect/frame-drop/response faults, create-new typed
  evidence, exact fault-matrix bindings for every proxy-expressible role, and
  explicit rejection for genuine partial-fill and restart-latch proof.
- [ ] Run credentialed bounded observe and minimal-size demo fault campaigns,
  including process death, deadman expiry, clock skew, websocket order
  ambiguity, partial fill, public/private/order-command reconnect, and
  durable-latch restart recovery.
- [x] Add an authenticated OKX websocket order-command pool with the pinned
  Java default of eight sessions, stable underlying dispatch, bounded request
  correlation, exchange expiry, aggregate readiness, supervised reconnect,
  explicit pre-send versus ambiguous-write classification, REST cancel
  fallback, and a required clean reconciliation after transport loss.
- [x] Remove account order-path head-of-line blocking: isolate REST
  reconciliation from command dispatch and support bounded concurrent
  underlying-routed websocket operations with shared account pacing and
  deterministic canonical completion.
- [x] Separate OKX funding forecasts from realized settlement accounting,
  retain `settFundingRate` with observed `prevFundingTime`, reject conflicting
  realized rates, settle at the original exchange timestamp without strategy
  look-ahead, and require nonzero training/test settlement evidence in schema-8
  production research.
- [x] Match pinned Java OKX system-status monitoring with a typed current-wire
  `/api/v5/system/status` parser, environment-aware bootstrap and 10-second
  periodic checks, a 60-second maintenance lead, Java-equivalent service
  filtering, and typed fail-closed live cleanup.
- [x] Bind every live strategy fee to the authenticated current OKX instrument
  `groupId` and `feeGroup` rate, reject configured maker/taker costs that
  understate commissions or overstate rebates, and repeat a paced full-account
  sweep without allowing a blocked fee request to delay Cancel All After.
- [x] Revalidate each exact authenticated OKX instrument before its periodic fee
  check, reject state, sizing, valuation, currency, family, or fee-group drift,
  and fail before typed `upcChg` rule changes enter a one-hour review lead.
- [x] Retain and continuously compare authenticated `maxLmtSz`, `maxMktSz`,
  `maxLmtAmt`, and `maxMktAmt`; reject incompatible configured quote sizes at
  bootstrap and enforce current limit-order quantity/amount before every live
  post-only or IOC dispatch.
- [x] Add independent full research reconstruction bound to the exact manifest,
  executable, and archived inputs; reject unknown/stale/forged/non-passing or
  numerically drifting reports, and persist owner-only durable evidence.
- [x] Bind schema-8 production research to one manifest-declared deployment
  candidate and fail unless every fold selects that candidate from training data.
- [x] Mirror Java first-run account/position input with a strict single-account
  backtest opening snapshot, seed strategy and portfolio from the same balances
  and derivative average costs, block entry until opening valuation, report true
  opening-adjusted PnL and ending state, reject terminal strategy halts, and
  require identical positive opening capital in production research.
- [x] Derive each schema-8 production chain root's opening portfolio from a unique,
  independently rebuilt account certification; bind exact build, calibrated
  host, proposed production-config bytes, production environment, account
  identity, instrument accounting scope, spot valuation mapping, and bounded
  pre-capture timing, while retaining certified available/equity/loan/margin
  fields in strategy state.
- [x] For explicit ordinal ranges of one verified capture session, carry ending
  balances, derivative quantity, terminal average cost, current valuation state,
  pending/settled funding state, recalculated margin, and latest stateful
  index/funding/mark/price-limit inputs into the next range.
  Match Java `updateInputForNextRun` after terminal mark-to-market,
  balance/margin reset, derivative average-cost reset, strategy stop/cancel, and
  hold release. Schema-8 research requires a linear `continuation_of` contract,
  exact next process-global ordinal, non-regressing receive time, the same
  canonical raw/config/report evidence, and a source-rebuilt certified root.
- [x] Recalculate and publish full simulated account/position/margin state on a
  deterministic 10-second schedule, matching the Java portfolio service, while
  preventing that path from bypassing delayed fill-to-account visibility.
- [ ] If capture file rotation is introduced, preserve one session ID and
  process-global ordinal across rotated files before allowing cross-file carry.
  Separate capture processes and restarted sessions remain independent.
- [x] Independently bind the reconstructed deployment candidate's effective
  strategy hash to the exact proposed production live config, rejecting smoke,
  demo, invalid reconstruction, ambiguous provenance, and strategy drift.
- [x] Add a strict source-rebuilding production evidence bundle that predeclares
  and cross-binds the exact official-demo/production configs, deterministically
  derived routed fault config, release binary, target host, deployment candidate,
  and separate demo/production account identities across transition, research,
  soak, fault, latency, account, deadman, emergency, authenticated fill/fee, and
  account-wide trade/funding economic gates. It never authorizes production
  entry.
- [x] Require a bounded schema-2 freshness policy for every operational source
  in the aggregate bundle. Reverify fault/latency live reports, preserve typed
  proxy command times, require those commands to occur inside their live
  sessions, and reject invalid, future, or stale timestamps under code-level
  maximum ages.
- [x] Add binary/host provenance and independently re-derived clean-shutdown
  fields to schema-2 fault-proxy process reports, expose an offline source
  verifier, and require schema-8 production bundles to provide one unique proxy
  session enclosing exactly each fault-matrix live run.
- [ ] Calibrate queue position, latency, fees, funding, and slippage from
  captured full-depth data, then run and archive production-candidate
  walk-forward, capacity, and stressed-liquidity reports. The orchestration and
  configurable execution model, source-rebuilding latency verifier, and exact
  research reconstruction verifier are implemented but have no credentialed
  calibration artifact; empirical queue and populated per-class/per-instrument
  latency distributions, target-tier simulated-fee calibration, complete
  funding intervals, a passing real target-account cash/zero-liability artifact,
  and passing authenticated fill/bill economic artifacts are still required.
  Exact signed private-fill fee amount/currency is retained end to end and
  normal trade/funding bill reconstruction, journal-backed derivative close PnL,
  tightly bracketed opening/closing cash continuity, and source-rebuilt
  point-in-time currency/equity conversion are implemented, but no demo evidence
  exists. Cash-spot bill units and authoritative position-basis behavior need
  empirical target-account proof. Total-equity attribution across mark-to-market,
  taxes, deposits/withdrawals, and other deliberately unsupported bill types
  remains outside the controlled-window gate. The
  bill-reported funding mark is now checked against a narrow two-sided public
  journal bracket rather than accepted alone; the exact internal tick remains
  unobservable. Margin
  spot is unsupported and would require a borrow-interest model before
  enablement.
- [x] Require a verified schema-5 capture run report for every
  production-candidate raw dataset, bind optional normalized output, embed the
  verification result, and recheck all artifact hashes after research runs.
- [ ] Deploy and exercise the webhook/host guards, systemd supervision, external
  unit-failure paging, and independent exchange cancel procedure on the target
  host/account, including regular, algo, and spread final-zero evidence.
- [ ] Certify production credentials/account mode/limits and expose production
  order entry only after every gate in `docs/trading-readiness.md` is signed off.
  Permission/IP policy and source-verifiable account evidence are implemented,
  but no passing target-host credential artifact exists.
  The source-rebuilding bundle now has a short-lived Ed25519 approval request,
  offline signer, strict two-role policy, and final source-rerunning verifier.
  No passing target-host bundle or real independently signed approval artifact
  exists, so production entry remains unavailable.

Completed-step evidence is documented in [README.md](README.md),
[docs/operations.md](docs/operations.md), and
[docs/performance.md](docs/performance.md). The workspace test, lint, replay,
configuration, backtest, and benchmark commands are the acceptance gates.
The remaining path to trading is tracked in
[docs/trading-readiness.md](docs/trading-readiness.md).
