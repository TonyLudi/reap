# Determinism And Measured Runtime Goal D Execution Prompt

Status: active execution contract until the required Goal D handoff records
every gate green. After completion, preserve this prompt as a historical
contract; later latency-architecture work, credentialed exchange campaigns,
and production approval require separately reviewed goals.

Use this document as the complete instruction set. The short invocation is:

> Execute Goal D exactly as specified in
> `docs/determinism-readiness-goal-d-prompt.md`. Continue phase by phase
> through every green gate and stop only at completion or a documented stop
> condition.

## Objective

Harden the bounded Chaos implementation in five places identified by the
post-Goal-C architecture review:

1. make fail-closed owned-order cancellation order deterministic;
2. close the pinned-Java public-trade-driven implied-depth parity gap;
3. define and make replayable the exact backtest/live strategy-and-risk parity
   boundary without coupling backtest to the live runtime;
4. replace floating-point-to-string conversion at the regular order/wire
   boundary with an exact, typed, exchange-aligned representation; and
5. add action-producing latency, queue-age, allocation, liveness, and health
   evidence before considering a different runtime architecture.

Keep the current deterministic single-writer architecture and the exact
Chaos connectivity/authority boundary. This goal authorizes only the narrowly
listed semantic changes. It does not authorize broader exchange connectivity,
credentialed trading, production order entry, or a thread-per-core, SPSC,
kernel-bypass, or custom-runtime redesign.

## Normative Baseline

The starting implementation baseline is commit
`83aac2abad0beef4c7f3202d2d5b921828cdc311`. Starting `HEAD` MUST contain that
commit. A later prompt-only commit may also be present.

Treat these documents as normative:

- [chaos-connectivity-boundary.md](chaos-connectivity-boundary.md) for
  supported inputs, purposes, exchange operations, and the pinned Java scope;
- [chaos-connectivity-goal-b-handoff.md](chaos-connectivity-goal-b-handoff.md)
  for the completed capability and dependency baseline;
- [maintainability-refactor-goal-c-handoff.md](maintainability-refactor-goal-c-handoff.md)
  for the completed ownership, structural, deterministic, and current
  benchmark baseline;
- [architecture.md](architecture.md) for single-writer ownership and event
  ordering;
- [performance.md](performance.md) for benchmark methodology, while treating
  the later Goal C handoff measurements as authoritative where the documents
  differ; and
- [trading-readiness.md](trading-readiness.md) for the distinction between
  implemented tooling, credentialed evidence, and trading approval.

The behavioral reference remains the clean sibling checkout
`../imm-strategy` at
`b6b120c7b7c466d8431bf082f3229328c5d7b2ae`. Do not modify it. Only behavior
transitively reached by the supported Chaos/iarb2 path is normative. Generic
gateway processors, `ExecAlgo`, unrelated strategies, and the Java
eight-session command pool do not broaden Reap's scope.

Before editing, verify and record in the Goal D handoff:

- Reap has no unexplained changes and no other session is writing overlapping
  files;
- the implementation baseline and completed Goal B and Goal C handoffs are
  present;
- `../imm-strategy` is present, readable, clean, and at the pinned full SHA;
- `reap_core::PINNED_JAVA_REVISION` and every checked-in Java evidence binding
  equal that SHA;
- the current dependency graph, public authority surface, configuration and
  artifact schema versions, deterministic fixture hashes, canonical backtest
  output hash, and `Cargo.lock` hash;
- the current fail-closed cancellation order for deliberately permuted
  insertion orders and separate process executions;
- the reached Java call path and state touched by `OkEntity.onPublicTrade` and
  `Iarb2Strategy.onPublicTrade`;
- the current Rust path from exchange instrument metadata through
  `RegularExecutionPolicy`, `PreparedRegularSubmit`, REST serialization, and
  websocket serialization;
- the exact current live/backtest decision-path parity matrix; and
- one warm-up plus three recorded same-host runs of the existing engine and
  live benchmarks.

Phase 0 creates
`docs/determinism-readiness-goal-d-handoff.md`. It records commands, results,
hashes, benchmark distributions, phase commits, authorized output changes, and
deferrals throughout the goal.

## Non-Negotiable Invariants

Preserve:

- the plan-derived Chaos inputs and exactly three executable purposes: Quote,
  Hedge, and CancelOwned;
- regular PostOnly quote placement, regular IOC `CancelMaker` hedge placement,
  and canonical owned-regular cancellation only;
- read-only algo/spread zero proof as a live safety observation, with no
  algo/spread submit, amend, or normal-live cancel authority;
- separate emergency executable and adapter authority, with no dependency or
  reachability from normal live execution;
- one `LiveRuntime`/`LiveCoordinator` canonical mutation owner and the absence
  of shared mutable canonical strategy, risk, book, portfolio, or order state;
- event priority, bounded-channel saturation behavior, same-turn
  `PendingNew` reservation, pacing-before-IO, storage-first commits, durable
  latch ordering, cancel-before-reconcile shutdown, and fail-closed behavior;
- typed, non-Clone, take-once regular-order authority and adapter-owned
  prepared-to-wire lowering;
- exact-plan private connectivity and the current one command lane per
  executing account;
- deterministic timer behavior, Java-compatible random-number consumption,
  floating-point operation order inside strategy/model calculations, and
  stable intent traversal except for the explicitly corrected cancellation
  order and the private 100-microsecond trade-reprice deadline authorized in
  Phase 2;
- the normal absence of `reap-backtest -> reap-live`;
- journal, report, evidence, configuration, and public CLI schemas unless a
  phase explicitly requires and separately gates a versioned migration; and
- `production_order_entry_authorized: false` in every evidence and approval
  artifact.

The public-trade phase uses an input already required by the normative
connectivity plan. It MUST NOT add a socket, channel, subscription family, or
exchange operation. The numeric phase changes only the validated regular
order-to-wire representation; it MUST NOT change Chaos model arithmetic into a
workspace-wide fixed-point model.

Do not use `TRYBUILD=overwrite` across a suite. Review every changed diagnostic
fixture individually and accept only the intended ownership, path, or type
change.

## Narrowly Authorized Behavior Changes

Only these semantic changes are authorized:

- fail-closed cancellations synthesized from risk state become bytewise
  lexicographically ordered by canonical owned client order ID in the
  Chaos/live path and by the existing canonical order-ID string projection in
  the generic engine path, after any already-emitted strategy cancellations
  and with duplicates removed;
- public trades may invalidate Java-defined implied depth and cause the exact
  reached Chaos/iarb2 repricing behavior. One private, typed, deterministic
  100-microsecond scheduled trade-reprice action is authorized without changing
  public market/timer schemas;
- a credential-free replay path may apply the same `TradingEngine<ChaosStrategy>`
  and `RiskGate` decisions as live, while the existing default economic
  backtest remains byte-identical;
- REST and websocket regular submits may use a canonical decimal
  representation that is numerically identical to the policy-approved
  tick/lot value rather than direct `f64::to_string()`. A bounded internal
  addition may retain exact tick/lot/min-size metadata through
  venue/bootstrap/policy types, while serialized configuration, journal,
  report, and evidence schemas remain frozen. Idempotency/order-fingerprint
  equality may bind canonical numeric units instead of raw `f64` bits, so two
  accepted bit patterns with the same wire value share numeric identity and
  wire-distinct values conflict; and
- low-overhead counters, monotonic timestamps, health state, and structured
  heartbeat output may be added without affecting trading decisions.

Any other output or behavior change is a stop condition.

## Execution Discipline

Work phase by phase. For each phase:

1. inventory the affected state, callers, authority values, fixtures, and
   source-policy guards before editing;
2. add a focused failing regression or golden fixture first;
3. make the smallest change that satisfies the phase;
4. keep correctness changes separate from mechanical moves and performance
   changes;
5. run focused deterministic, authority, and benchmark gates;
6. inspect the complete diff, dependency graph, public exports, allocations,
   and serialized output;
7. record exact commands and results in the Goal D handoff;
8. commit the phase only when its gate is green; and
9. continue automatically while green.

Do not hide nondeterminism by retrying a flaky test. Do not update a golden
hash until the handoff identifies the exact authorized semantic difference and
an independent fixture proves it.

## Phase 0: Baseline, Call-Path Audit, And Measurement Contract

Create the Goal D handoff and record the required starting checks.

For cancellation determinism:

- construct the same live-order set in multiple insertion orders;
- run the same fail-closed fixture in separate processes;
- record the current generic and typed Chaos outputs; and
- specify the exact expected post-fix sequence.

For Java trade parity, audit at least:

- `chaos/chaos-core/src/main/java/app/metcoin/chaos/ChaosStrategyBase.java`;
- `chaos/chaos-core/src/main/java/app/metcoin/chaos/model/entity/OkEntity.java`;
- `chaos/chaos-iarb2/src/main/java/app/metcoin/chaos/iarb2/Iarb2Strategy.java`;
- every transitively reached implied-depth field, timer/debounce path, and
  quote-refresh call; and
- any existing Java test or fixture that fixes side, price, quantity,
  timestamp, and equality behavior.

Document a truth table before implementation: irrelevant trade, passive trade,
aggressive buy, aggressive sell, the Java trade-side-to-book-side reversal,
equality at the implied level, the exact half-level-quantity boundary, stale or
out-of-order timestamp, repeated trade, trade before depth, state after depth
clearing, ignored-best-level/own-hedge interaction, and a trade that causes
repricing. If the pinned code does not define one of these cases, record that
fact rather than inventing behavior.

For decision parity, document four separate layers:

1. normalized input and scheduling;
2. ordered typed Chaos strategy intents;
3. engine/risk allowed, rejected, system-event, and safety-cancel output; and
4. simulated matching/accounting versus live coordination, persistence, and
   exchange IO.

For numeric authority, inventory the exact source representation of tick size,
lot size, minimum size, price, and quantity, including whether exact exchange
decimal text is discarded before policy validation.

Define the new action-producing benchmark workloads and timestamp boundaries
before optimizing. At minimum include:

- quote creation;
- quote replacement and owned cancel;
- IOC hedge creation;
- risk rejection;
- symbol-scoped fail-closed cancellation;
- global fail-closed cancellation;
- a public trade that changes implied depth and triggers repricing; and
- a burst/control-storm workload that exercises branch priority and bounded
  queues.

Add the credential-free action-producing benchmark harness in Phase 0 and run
one warm-up plus five recorded baseline runs before any semantic fix. Reuse
production reducers, strategy, engine/risk, coordinator decisions, order
policy/gateway preparation, and serializers; do not copy their logic into the
benchmark. Where one end-to-end workload would blur boundaries, publish
separate raw-to-decision/action-record and prepared-request-serialization
measurements. Record p50/p95/p99/p99.9/max, logical action counts, allocations,
and every included/excluded stage. A benchmark-only queue harness may measure
burst/control-storm age without changing production queue ownership.

The Phase 0 trade workload MUST exercise the currently missing behavior and
record zero trade-reprice actions as the baseline; its action assertion becomes
nonzero only after the pinned Phase 2 fixture defines the expected result.
Benchmark support must remain private, test-only, or bench-only. Do not expose
approval bindings, constructors, prepared values, signers, transports, or raw
serializers to make the harness compile.

Phase 0 changes documentation, benchmark/test support, and fixtures that
demonstrate the baseline only. It MUST NOT change production trading behavior.
Gate it before production-code changes.

## Phase 1: Deterministic Fail-Closed Cancellation

Make every risk-derived fail-closed cancellation sequence deterministic in
both the generic and typed Chaos engine paths.

Requirements:

- retain the original order of strategy-emitted cancellations;
- deduplicate against those cancellations without reordering them;
- filter global or symbol-scoped risk live orders as today;
- sort only the remaining synthesized client order IDs using bytewise
  lexicographic order;
- apply risk to those cancellations in that exact order;
- do not replace unrelated hot-path maps or add sorting to the normal
  non-fail-closed path; and
- preserve each cancellation's existing reason and, in the Chaos/live path,
  its canonical owned-order proof.

Add exact-output tests covering randomized/permuted registration order,
symbol-scoped halts, global kill, an existing strategy cancellation, and
generic/typed equivalence. Separately retain a coordinator authority test
showing that an unproven safety-cancel candidate is rejected after engine
ordering; do not invent a `RiskGate` cancellation rejection because
`CancelOrder` is intentionally allowed by pre-trade risk. Add a child-process
or equivalent executable golden fixture proving identical serialized output
across process hash seeds.

The phase gate requires identical non-fail-closed output and allocation counts,
stable exact fail-closed output, and all engine/risk/live authority tests green.

## Phase 2: Pinned-Java Public-Trade Implied-Depth Parity

Implement only the public-trade behavior reached by the supported pinned
Chaos/iarb2 call path.

Requirements:

- retain the existing plan-derived trade subscription and normalized
  `MarketEvent::Trade`; add no connectivity;
- reproduce the reached Java state transition for aggressive trade
  invalidation of implied depth and any resulting refresh scheduling;
- preserve Java's live-state gate and reversed taker-side-to-book-side mapping,
  including invalidation of cached first-valid levels and the opposite-side
  last-trade state only under the reached aggressive-crossing condition;
- preserve Java comparison boundaries, timestamp handling, debounce/timer
  behavior, intent order, floating-point operation order, and RNG consumption;
- preserve the reached Java 100-microsecond deferred-pricing behavior with one
  private typed scheduled action. Represent its due time at microsecond or
  finer precision without changing public `TimeMs`, `MarketEvent`,
  `TimerEvent`, journal, capture, or configuration schemas;
- schedule from monotonic event-receipt/replay-arrival time plus exactly 100
  microseconds; exchange timestamps remain input data and do not measure the
  local delay;
- keep the private scheduled-action queue bounded and owned by the existing
  single writer. In live, service a due trade-reprice branch immediately before
  ordinary feed receive but after the existing shutdown, safety/control,
  operator, reconciliation, and periodic-timer priorities. In backtest/replay,
  use the existing nanosecond scheduler and sequence tie-breaker;
- preserve the pinned Java multiplicity/conflation behavior for several
  qualifying trades rather than inventing coalescing, and give the private
  action no public constructor or serialized form;
- keep raw trade matching/calibration behavior in backtest separate from the
  strategy's implied-depth reaction;
- do not reinterpret trades as `BurstSignal`;
- preserve by-value owned-event and borrowed-event equivalence; and
- keep all new state private inside the cohesive Chaos strategy/instrument
  ownership tree.

Create checked-in fixtures derived from the pinned Java revision for every
reached truth-table case. Each fixture MUST bind the full Java SHA and explain
the source method/call path. Test identical ordered typed intents and legacy
projections for live-style and backtest-style delivery. Existing fixtures that
contain no reached trade-driven state transition MUST remain byte-identical.

Update `chaos-connectivity-boundary.md` only after the fixtures pass, replacing
the known Rust behavior-gap statement with the exact implemented behavior and
remaining exclusions.

If the Java behavior requires an input not present in the current
`ChaosConnectivityPlan`, stop. Do not broaden connectivity to complete this
phase. Also stop if exact deferred behavior would require a public
timestamp/schema change or a second canonical mutation owner; do not silently
approximate 100 microseconds as an immediate or millisecond event.

## Phase 3: Explicit Backtest/Live Decision And Risk Parity

Create `docs/backtest-live-decision-parity.md` and make the parity boundary
executable rather than aspirational.

Implement a deterministic, credential-free replay harness that:

- consumes the same normalized events and explicit timer/control events as the
  live decision path;
- uses the production `TradingEngine<ChaosStrategy>` and `RiskGate`, not a
  copied risk implementation;
- starts from a checked-in, hashed initialization artifact containing the
  complete effective `RiskLimits`, instrument risk models/order limits,
  feed/private readiness and timestamps, marks/reference/stablecoin state,
  account equity/positions, live orders, and every other state that can affect
  a decision; missing required state fails closed, and permissive test/default
  initialization is forbidden;
- captures an ordered canonical projection of typed intents, rejections,
  system events, and safety-cancel candidates;
- can be run twice with byte-identical output;
- covers quote, hedge, trade-driven reprice, risk rejection, symbol halt,
  global kill, fill/order state, and fail-closed cancellation; and
- has a fixture compared from the exact same initialized state with a
  canonical projection of the equivalent `LiveCoordinator` reduction output
  before dispatch and commit. Do not extract a new pre-storage owner or weaken
  same-turn `PendingNew` registration merely to make the comparison easier.

Keep the harness in `reap-engine` test/bench support and compare it through a
`reap-live` integration fixture. Do not add a normal backtest dependency,
runtime type, public configuration mode, or product API for this evidence.
`reap-backtest -> reap-live`, live runtime types in backtest, credentials,
transports, storage leases, and emergency authority remain forbidden.

Do not silently route the existing economic backtest through a new risk path.
Its default configuration and canonical output MUST remain byte-identical.
Document that the separate risk replay does not simulate exchange ambiguity,
durable commit ordering, reconciliation, or emergency behavior.

This phase is complete when the shared decision/risk path is independently
replayable and the documentation precisely states which layers are equal,
modeled, or intentionally different.

## Phase 4: Exact Regular Order-To-Wire Numeric Boundary

Introduce opaque exact price and quantity representations when
`RegularExecutionPolicy` creates `ApprovedRegularSubmit`. Carry them through
reservation, idempotency, preparation, and serialization.

Requirements:

- add the bounded internal venue/bootstrap/policy metadata needed to preserve
  exact exchange tick, lot, and minimum-size decimal text or an equivalent
  lossless scale/integer representation instead of treating parsed `f64` as
  the source of truth; this internal propagation is explicitly authorized,
  but serialized configuration, journal, report, evidence, and public CLI
  schemas are not;
- compute the checked nearest tick/lot unit count from the model `f64` and
  accept it only when the existing documented ULP/alignment predicate accepts
  the original value; canonicalization is representation, never discretionary
  order rounding;
- prove that each accepted model `f64` price and quantity resolves to an
  integral, bounded tick/lot count and retain the same acceptance/rejection
  fixtures as the current policy;
- reject non-finite, non-positive, misaligned, overflowed, underflowed, or
  unrepresentable values before creating prepared authority;
- carry the opaque canonical representation through
  `ApprovedRegularSubmit`, `ReservedRegularSubmit`, and
  `PreparedRegularSubmit`, so it exists before canonical local reservation and
  gateway idempotency;
- replace raw-f64 numeric identity in order fingerprints with canonical unit
  counts/decimal bytes: the same canonical wire `px`/`sz` must have the same
  numeric identity, and different canonical wire values must have different
  numeric identity;
- make REST and websocket adapters serialize the same canonical price and
  quantity bytes through one narrow lowering helper;
- remove direct `price.to_string()` and `qty.to_string()` from normal regular
  submit lowering, and add a negative/source-policy test preventing normal
  submit serializers from deriving `px` or `sz` from raw `f64`;
- preserve the one-shot authority chain and prohibit public construction or
  parsing from untrusted serialized order intents; and
- leave strategy, risk-notional, backtest-model, and Java-parity arithmetic as
  `f64` in this goal.

Golden tests MUST cover integer contracts, `0.1`, `0.05`, `0.0001`, an
exchange-supported very small increment, large bounded values, floating-point
near-misses, scientific-notation inputs, and REST/websocket `px`/`sz` field
byte equality. Include two distinct accepted `f64` bit patterns that resolve
to the same canonical wire value and two accepted wire-distinct values,
asserting the intended idempotency equality/conflict result.
Property or exhaustive bounded tests SHOULD cover round-trip tick/lot counts
and rejection immediately either side of an increment.

Prefer repository code over a new dependency. Any new numeric dependency,
`Cargo.lock` change, public serialized schema migration, or expansion beyond
the explicitly authorized internal tick/lot/min-size metadata propagation
requires a separately recorded design justification and a phase-local review.
If exactness cannot be achieved without a broad serialized schema migration,
stop and propose that migration as its own goal rather than retaining a
false-exact wrapper.

## Phase 5: Action-Producing Performance, Queue-Age, And Health Evidence

Extend and finalize the Phase 0 reproducible benchmark, and add low-overhead
production observability for the actual decision path.

The benchmark MUST report, for each defined workload:

- p50, p95, p99, p99.9, and maximum elapsed time;
- events/frames, normalized outputs, typed intents, risk rejections,
  safety-cancel candidates, prepared submits/cancels, and storage records;
- allocation calls and requested bytes per input and per produced action;
- bounded-queue age and high-water marks; and
- the exact included/excluded boundary for parsing, channel scheduling,
  coordinator reduction, policy/gateway preparation, serialization, storage
  enqueue, disk IO, network IO, and exchange acknowledgement.

Use monotonic process-local timing for elapsed/queue-age measurements. Exchange
or wall-clock timestamps remain data and MUST NOT be used as the duration
clock. Each recorded workload result MUST contain at least 100,000 post-warmup
timed observations. Record the percentile algorithm, histogram/reservoir
precision, timer-read overhead, dropped/overflowed sample count, host/toolchain,
and machine-readable JSON output. Five whole-program elapsed times alone do
not constitute p99.9 evidence.

Add one private, versioned `RuntimeHealthSnapshot` assembled by
`LiveRuntime` from bounded numeric/enum state. Emit schema version 1 through
the existing structured logging path every fixed five seconds and once during
normal finalization. Do not add a listener, socket, HTTP server, supervisor
protocol, or configuration field in this goal.

The snapshot MUST contain:

- feed/private/order-command connectivity and last-progress age;
- runtime event-loop and order-task heartbeat;
- queue capacity, depth/high-water mark, age, and saturation;
- readiness state and active durable safety latches;
- submitted, cancelled, rejected, ambiguous, and reconciled counts; and
- storage commit progress/failure.

Progress updates MUST use predeclared IDs and numeric/enum atomics or
single-writer fields. They may not take a mutex/RwLock, format a string, clone a
symbol, or allocate on a per-event path. In particular, do not call the current
allocating `HealthRegistry::set` from progress paths; existing telemetry types
may be reused only where they satisfy this budget. String/JSON construction is
allowed only in the five-second and final snapshots. Add state-transition,
cadence, bounded-cardinality, and serialization tests. The snapshot grants no
control or order authority and is not production evidence.

Run one warm-up and at least five recorded action-producing runs on an
otherwise idle host. The result is a regression baseline, not a target-host
SLO. Profile before optimizing. A profile-guided optimization is allowed only
as a separate commit when the profile identifies a specific dominant
allocation or reduction cost and the change preserves every invariant above.

Do not introduce pinned trading threads, busy spinning, SPSC/ring buffers,
custom allocators, `io_uring`, kernel bypass, hardware timestamps, a custom
Tokio runtime, or a new concurrency owner. If p99.9 evidence indicates such a
change may be necessary, record a separate proposed goal with an explicit
target-host decision threshold.

Update `performance.md` so its current summary uses the authoritative Goal C
baseline and clearly separates historical measurements, the new
action-producing local results, and the still-missing target-host
decision-to-wire evidence.

## Phase 6: Global Verification, Documentation, And Handoff

Update:

- `architecture.md` with the exact decision/risk parity boundary, exact
  numeric order boundary, and current crate inventory;
- `chaos-connectivity-boundary.md` with the completed trade parity result and
  unchanged connectivity/authority surface;
- `performance.md` with authoritative current measurements and benchmark
  exclusions;
- `trading-readiness.md` with the new code/evidence status while retaining all
  credentialed and target-host blockers; and
- the Goal D handoff with exact phase commits, commands, hashes,
  before/after outputs, benchmark distributions, allocation counts, and
  explicit deferrals.

Documentation MUST continue to say:

- production order entry is unavailable;
- algo/spread live access is read-only zero proof, not strategy connectivity;
- emergency mutation is separate;
- local benchmarks do not certify target-host tail latency;
- no credentialed demo soak or production evidence was created by Goal D; and
- pinned-thread/SPSC or deeper HFT runtime work remains conditional on measured
  target-host evidence.

## Determinism And Performance Gates

Run the canonical CLI backtest twice before Phase 1, after Phase 2, after Phase
3, and at completion:

```bash
mkdir -p /home/ubuntu/code/reap/target/tmp
cargo run --locked -q -p reap-cli -- \
  backtest \
  --format normalized-jsonl \
  --config examples/iarb2-basic.toml \
  --data fixtures/normalized/chaos_quote_hedge.jsonl \
  --pretty >target/tmp/goal-d-backtest-1.json
cargo run --locked -q -p reap-cli -- \
  backtest \
  --format normalized-jsonl \
  --config examples/iarb2-basic.toml \
  --data fixtures/normalized/chaos_quote_hedge.jsonl \
  --pretty >target/tmp/goal-d-backtest-2.json
cmp target/tmp/goal-d-backtest-1.json target/tmp/goal-d-backtest-2.json
sha256sum target/tmp/goal-d-backtest-1.json
```

The Goal C baseline hash is
`38acf9f5e0c310f2ec5528974beffadf4c1a7f84d46efa8d9664ee7051e84691`.
The canonical `chaos_quote_hedge.jsonl` fixture contains no public trade and
this hash MUST remain exact throughout Goal D. Phase 2 adds a separate
`fixtures/normalized/chaos_trade_implied_depth.jsonl` fixture and records its
new pinned-Java input and output hashes in the handoff; do not alter the
canonical no-trade fixture to demonstrate the new behavior.

Run one warm-up and three recorded existing benchmark runs after Phases 1–4,
then one warm-up and five recorded runs for all final benchmarks:

```bash
cargo bench -p reap-engine --bench event_loop --locked
cargo bench -p reap-live --bench live_loop --locked
cargo bench -p reap-live --bench action_path --locked
```

Phase 0 MUST create the `reap-live` `action_path` benchmark target so this
command is stable for every later gate.

Require:

- exact logical counters for repeated runs of the same revision;
- byte-identical deterministic fixtures and replay projections;
- no allocation increase in the ordinary non-action path unless Phase 5's
  observability budget documents and proves an unavoidable bounded cost;
- no per-event allocation from metrics or health recording;
- investigation when a three-run same-host median regresses by more than 5%;
- investigation when same-workload p99 or p99.9 regresses by more than 5%,
  using the same sample count and percentile method;
- after eliminating host noise and repeating with five warm recorded runs, no
  unexplained median, p99, or p99.9 regression greater than 10%; and
- no latency claim based only on a median or on a benchmark that omits the
  relevant queue, gateway, serialization, or IO stage.

## Final Verification

Run focused package suites after each phase. At completion, run at minimum:

```bash
mkdir -p /home/ubuntu/code/reap/target/tmp
cargo fmt --all -- --check
TMPDIR=/home/ubuntu/code/reap/target/tmp \
  cargo clippy --workspace --all-targets --locked -- -D warnings
TMPDIR=/home/ubuntu/code/reap/target/tmp \
  cargo test --workspace --locked --no-fail-fast
TMPDIR=/home/ubuntu/code/reap/target/tmp \
  cargo build --release --workspace --locked
TMPDIR=/home/ubuntu/code/reap/target/tmp \
  deploy/systemd/verify-units.sh target/release/reap
TMPDIR=/home/ubuntu/code/reap/target/tmp \
  cargo audit --deny warnings
cargo metadata --locked --format-version 1 >/dev/null
git diff --check
```

Also run explicitly:

```bash
cargo test -p reap-strategy --locked
cargo test -p reap-engine --locked
cargo test -p reap-risk --locked
cargo test -p reap-backtest --locked
cargo test -p reap-order --locked
cargo test -p reap-okx-live-adapter --locked
cargo test -p reap-live --locked
cargo test -p reap-live --test dependency_policy --locked
cargo bench -p reap-engine --bench event_loop --locked
cargo bench -p reap-live --bench live_loop --locked
cargo bench -p reap-live --bench action_path --locked
```

Verify:

- all pinned-Java parity, strategy determinism, backtest semantic, role
  visibility, authority, storage-proof, compile-fail, adapter allowlist, and
  shutdown tests;
- no normal live reachability to emergency mutation or raw wire authority;
- no algo/spread placement, amend, or generic request method;
- no normal backtest-to-live dependency;
- one live canonical mutation owner and unchanged relative priority among all
  pre-existing event-loop branches, with only the Phase 2 private due-reprice
  branch inserted at its specified location;
- exact REST/websocket canonical `px`/`sz` field equality;
- unchanged schema versions unless an explicitly authorized phase-local
  migration passed;
- updated documentation has no stale current benchmark claim; and
- Reap and `../imm-strategy` are clean at completion.

## Stop Conditions

Stop and report the exact conflict when:

- Reap or `../imm-strategy` has unexplained or overlapping changes, or the Java
  checkout differs from the pinned revision;
- a required behavior cannot be traced through the supported pinned
  Chaos/iarb2 call path;
- trade parity requires a new input, connection, subscription, exchange
  operation, or `BurstSignal` substitution;
- a cancellation-order fix weakens owned-order proof, changes existing
  strategy-cancel order, or adds normal-path sorting;
- decision parity requires backtest to depend on live runtime, networking,
  credentials, persistence authority, or emergency code;
- exact order numbers cannot be derived without a broad or lossy numeric
  migration;
- an authority token becomes public, Clone, serializable, forgeable, reusable,
  or constructible below its policy owner;
- event priority, RNG consumption outside the reached Java trade behavior,
  timer behavior outside the private Phase 2 trade-reprice action,
  storage/action ordering, shutdown behavior, or default backtest semantics
  change;
- a source-policy guard passes only by broadening an allowlist;
- benchmark logical counters differ nondeterministically, metrics allocate per
  event, or a repeated regression exceeds the allowed threshold;
- completion requires credentials, authenticated exchange access, target-host
  deployment, production approval, or modification of `../imm-strategy`; or
- a safe fix requires a pinned-thread/SPSC, allocator, kernel, public
  timestamp/schema, persistence, or concurrency redesign beyond the private
  Phase 2 due-time representation.

Do not weaken an invariant or relabel evidence to finish. Record the blocker
and propose the smallest separately scoped follow-up.

## Explicit Exclusions

Do not:

- add another venue, strategy, order type, regular profile, generic plugin, or
  gateway framework;
- add algo/spread placement or normal-live cancellation, amend/batch amend,
  borrowing, master/group feeds, or additional command lanes;
- remove read-only forbidden-domain zero proof;
- merge normal live and emergency authority;
- route the default economic backtest through a behavior-changing risk or live
  runtime path;
- perform a workspace-wide `f64` replacement, symbol interning, book-storage
  rewrite, or broad public API cleanup;
- split the single-writer runtime into actors or introduce shared mutable
  canonical state;
- add a health listener, HTTP server, Unix socket, supervisor protocol, or new
  live configuration surface;
- use credentials, contact authenticated exchange endpoints, deploy to a
  target host, or run a credentialed demo/fault/soak campaign;
- create or relabel production evidence or set any production authorization
  field true; or
- claim production readiness, a target-host latency SLO, or colocated-HFT
  readiness.

CLI/catalog file splitting, wildcard re-export cleanup, target-host deployment,
credentialed demo/fault/soak execution, and deeper runtime architecture are
separate follow-up goals.

## Completion

Goal D is complete only when:

- every phase and focused gate is green;
- fail-closed cancellation bytes are stable across insertion orders and
  processes;
- the reached Java public-trade implied-depth behavior and private
  100-microsecond deferred reprice have pinned fixtures and identical
  live/backtest ordered results;
- the strategy/risk parity boundary is documented and independently
  replayable without backtest-to-live coupling;
- regular order REST and websocket `px`/`sz` fields use the same exact typed
  tick/lot-aligned representation;
- the action-producing benchmark reports tail distributions, allocations,
  logical actions, and queue age under normal and stressed workloads;
- the versioned live health heartbeat snapshot is emitted only through
  existing structured logs/finalization, with bounded cardinality and no
  hot-path allocation or lock;
- the existing one-writer, authority, dependency, and emergency boundaries
  remain enforced;
- the Goal D handoff records exact evidence and remaining operational
  blockers; and
- Reap and `../imm-strategy` are clean.

Completion means the bounded Chaos decision path is more deterministic,
closer to the pinned Java behavior, exact at the order wire boundary, and
measurable enough to inform the next runtime decision. It does not approve a
credentialed demo campaign, production trading, or a low-latency architecture
redesign.
