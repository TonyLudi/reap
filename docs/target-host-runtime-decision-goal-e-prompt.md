# Target-Host Runtime Qualification Goal E Execution Prompt

Status: deferred. Goal D is complete, but no target host or target-host
acceptance contract is declared. Do not execute this goal until both exist and
the candidate baseline is re-reviewed. Goal F may run first; Goal E remains a
Chaos-only qualification and does not qualify Polymarket.

Use this document as the complete instruction set. The short invocation is:

> Execute Goal E exactly as specified in
> `docs/target-host-runtime-decision-goal-e-prompt.md`. Continue phase by
> phase through every green gate and stop only at completion or a documented
> stop condition.

## Objective

Decide, from reproducible target-host evidence, whether Reap's current
deterministic single-writer architecture is adequate for the bounded Chaos
strategy or whether a separately reviewed latency-architecture redesign is
justified.

This is a measurement and decision goal. It may make narrowly reviewed,
profile-guided changes that reduce measured overhead without changing trading
semantics or ownership. It does not authorize a thread-per-core rewrite,
shared canonical state, SPSC/ring-buffer migration, busy spinning, kernel
bypass, credentialed order entry, or production approval.

The required outcomes are:

1. a source-bound target-host measurement contract;
2. replayable decision-to-prepared-wire latency, due-action lateness,
   queue-age, allocation, throughput, and headroom evidence;
3. burst, control-priority, and bounded-backpressure evidence;
4. profiles that identify the dominant costs rather than infer them;
5. an explicit keep/optimize/redesign decision with confidence and
   limitations; and
6. when redesign is justified, a separate proposed goal rather than an
   implementation hidden inside Goal E.

## Normative Baseline

The immutable semantic reference for Goal E is the clean Goal D completion
commit `8258deb4b6e3a52e7c58c792da913210e0877fbb`. Phase 0 MUST record that
full SHA and verify that the Goal D handoff states Phases 0 through 6 and the
global verification gate are green.

The measured candidate may be a later clean descendant containing separately
reviewed architecture work such as Goal F. Phase 0 MUST record both the Goal D
semantic-reference SHA and the exact candidate SHA, rerun the canonical Goal D
Chaos hashes and deterministic gates, and prove that Polymarket connectivity
and PM product work are absent from Goal E's measured workload. Goal E does
not qualify PM behavior, PM connectivity, or the PM runtime.

Treat these documents as normative:

- `docs/determinism-readiness-goal-d-handoff.md`;
- `docs/architecture.md`;
- `docs/performance.md`;
- `docs/trading-readiness.md`;
- `docs/chaos-connectivity-boundary.md`; and
- `docs/maintainability-refactor-goal-c-handoff.md`.

The behavioral reference remains the clean sibling checkout
`../imm-strategy` at
`b6b120c7b7c466d8431bf082f3229328c5d7b2ae`. Do not modify it. Only behavior
transitively reached by the supported Chaos/iarb2 path is normative. Generic
gateway processors, `ExecAlgo`, unrelated strategies, and Java session-pool
cardinality do not broaden Reap's scope.

Before any target-host run, create
`docs/target-host-runtime-decision-goal-e-handoff.md` and record:

- the Goal D semantic-reference commit and handoff status, plus the exact
  candidate source commit;
- the Reap and pinned-Java clean-state checks;
- target-host identity without secrets: provider/site label, CPU model and
  count, SMT/NUMA topology, memory, kernel, clock source, time service,
  filesystem, network-interface class, Rust/Cargo/LLVM versions, and active
  power/frequency policy;
- whether the host is shared, virtualized, burstable, throttled, or subject to
  noisy-neighbor scheduling;
- the exact candidate binary hash and build command;
- workload, configuration, capture, initialization-artifact, and fixture
  hashes;
- the existing Goal D local benchmark distributions as a comparison baseline;
- the shortest strategy scheduling interval, queue capacities, freshness
  thresholds, and operational deadlines relevant to the measured path; and
- an explicit measurement contract and decision thresholds before observing
  final results.

If there is no declared target host or Goal D is incomplete, stop without
editing production code. Record the unmet prerequisite; do not substitute the
development host and label it target-host evidence.

## Scope And Authority Boundary

Preserve exactly:

- plan-derived Chaos inputs and the three executable purposes Quote, Hedge,
  and CancelOwned;
- regular PostOnly quotes, regular IOC `CancelMaker` hedges, and canonical
  owned-regular cancellation only;
- read-only algo/spread zero proof with no normal-live mutation authority;
- separate emergency authority with no normal-live dependency or
  reachability;
- one `LiveRuntime`/`LiveCoordinator` canonical mutation owner;
- deterministic event priority and replay ordering;
- same-turn `PendingNew`, pacing-before-IO, storage-first commit, durable
  latch, reconciliation, and shutdown ordering;
- typed take-once authority from strategy purpose through policy, reservation,
  gateway preparation, and wire lowering;
- the exact Chaos connectivity plan and one command lane per executing
  account;
- the default economic backtest and its canonical output hash;
- public schemas, configuration formats, journal/report/evidence versions, and
  dependency direction; and
- `production_order_entry_authorized: false` in every artifact.

Goal E MUST NOT add an exchange subscription, socket, venue, strategy, order
type, command method, account, or connectivity family. Public market capture
may be used only through the existing credential-free capture boundary.

No credential, signer, authenticated REST call, websocket login, demo submit,
or production endpoint is authorized. A credentialed observe/demo/fault/soak
campaign is a separate operational goal requiring explicit operator
authorization.

## Phase 0: Preconditions And Measurement Contract

Verify the baseline and create the handoff before changing code.

Define each measured boundary precisely. At minimum include:

1. normalized event arrival to ordered typed strategy intents;
2. arrival to engine/risk decision;
3. arrival to coordinator action and storage-record construction;
4. arrival to prepared REST and websocket request bytes, excluding signing
   and network IO unless an existing credential-free transport harness can
   measure them without exposing authority;
5. private scheduled-action due time to actual service time;
6. ownership-handoff enqueue time to dequeue/service time;
7. burst/control-storm queue age and high-water mark; and
8. sustained throughput at the expected peak input mix with CPU and memory
   headroom.

For each boundary, record included and excluded work. Exchange timestamps are
data and MUST NOT be used as elapsed-time clocks. Use monotonic target-host
timestamps and record timer-read overhead.

The measurement contract MUST define, before the final recorded runs:

- expected peak and stress input rates and their source;
- warm-up duration and sample count;
- percentile algorithm and precision;
- p50, p95, p99, p99.9, and maximum reporting;
- allowed sample drops and histogram overflow, normally zero;
- decision-to-prepared-wire and due-action-lateness budgets;
- bounded-queue age, high-water, saturation, and loss budgets;
- CPU/memory headroom requirements;
- same-host regression thresholds against the exact unoptimized candidate
  selected in Phase 0, while retaining Goal D local distributions only as
  historical comparison;
- run invalidation criteria for throttling, host contention, clock changes, or
  background maintenance; and
- the rule used to choose Keep, Optimize, or Propose Redesign.

Thresholds must come from the declared strategy/operational deadline or an
explicitly reviewed target-host acceptance contract. Do not reverse-engineer
thresholds from the measured result.

Gate Phase 0 with a documentation-only commit.

## Phase 1: Reproducible Target-Host Harness

Reuse the production-shaped Goal D action benchmark and runtime health
instrumentation. Extend it only where the Phase 0 boundary cannot otherwise be
measured.

Requirements:

- reuse production normalization, strategy, engine/risk, coordinator,
  regular-order policy, gateway preparation, and serializer code;
- use a complete, hashed initialization artifact; permissive defaults or
  missing readiness/risk/account state fail closed;
- keep benchmark helpers private, test-only, or bench-only;
- generate no signing, network, storage, emergency, or approval authority for
  the harness;
- retain exact logical counters and ordered canonical action projections;
- collect queue age at existing ownership handoffs with predeclared numeric
  IDs and monotonic timestamps;
- do not allocate, format strings, clone symbols, or acquire a lock merely to
  record a hot-path sample;
- report histogram overflow and sampling loss explicitly;
- pin capture/config/fixture/binary hashes in every machine-readable result;
  and
- make two runs with the same seed and input produce byte-identical logical
  outputs even though timing distributions differ.

Add source-policy and structural tests proving that benchmark support cannot
mint or reach live transport authority.

## Phase 2: Target-Host Baseline And Stress Runs

Build the exact candidate in locked release mode on the target host. Record an
unmeasured warm-up, then at least five valid recorded runs for every workload.
Each latency distribution must contain at least 1,000,000 post-warm-up
observations unless the Phase 0 contract justifies a larger count.

Run at minimum:

- steady production-shaped public input;
- quote creation;
- quote replacement and owned cancel;
- IOC hedge creation;
- public-trade implied-depth reprice;
- risk rejection;
- symbol halt and global kill;
- symbol-scoped and global fail-closed cancellation;
- private order/fill/account convergence;
- scheduled-action collision and timer/control priority;
- expected-peak burst;
- two-times expected-peak burst for a bounded interval; and
- control storm while ordinary feed remains saturated.

For each run record:

- p50/p95/p99/p99.9/max latency and due-action lateness;
- input, normalized-output, intent, rejection, safety-cancel, prepared-action,
  storage-record, and callback counts;
- allocation calls and requested bytes per input and produced action;
- queue depth, high-water, age, saturation, and drops;
- owner-loop and order-task heartbeat progress;
- CPU utilization per relevant thread/core, migrations, involuntary context
  switches, frequency/throttling, memory high-water, and page faults; and
- whether the run met every predeclared validity and acceptance rule.

Run the canonical backtest twice and require byte equality and the Goal D
canonical no-trade hash. Run the Goal D deterministic decision/risk replay
twice and require byte equality.

Do not tune the host between individual recorded runs. A second host-policy
cohort is allowed only when it is declared and recorded as a separate
experiment.

## Phase 3: Profile Before Optimizing

Collect target-host profiles for every missed or marginal threshold. Prefer
sampling profiles and hardware counters available without kernel or security
policy changes.

Attribute at least:

- parsing and normalization;
- book reduction;
- strategy pricing/hedging;
- engine/risk;
- coordinator reduction;
- policy and gateway preparation;
- serialization;
- allocation and deallocation;
- channel scheduling and queue wait;
- timer wake/service delay; and
- OS scheduling, migration, throttling, and page-fault noise.

The handoff must identify the dominant contributors with measured percentages
or samples. Do not infer a queue/runtime problem merely because a tail is
high.

Classify the outcome:

- **Keep**: all thresholds pass with required headroom and no unexplained
  multimodal tail.
- **Optimize**: architecture and ownership remain adequate, but a measured
  local cost can be removed without changing semantics or authority.
- **Propose Redesign**: a threshold repeatedly fails after run-validity issues
  are eliminated, the dominant cause is structural, and bounded local
  optimization cannot supply the required headroom.

If the result is Keep, make no production performance change.

## Phase 4: Narrow Profile-Guided Optimization

Run this phase only for an Optimize result.

Each optimization must:

- have one measured hypothesis and one focused commit;
- preserve logical output bytes, RNG consumption, floating-point operation
  order, event priority, authority, schema, and dependency gates;
- avoid per-event allocation or locking in observability;
- include a regression test and before/after profile;
- rerun one warm-up and five recorded target-host measurements;
- preserve or improve every workload, not merely the selected median; and
- be reverted if the original bottleneck does not materially improve or a
  tail/queue/allocation gate regresses.

Allowed examples include eliminating a measured temporary allocation,
precomputing immutable lookup state, reducing a proven redundant projection,
or avoiding repeated serialization work inside the same action.

Goal E does not authorize:

- additional canonical state owners or actor decomposition;
- `Arc<Mutex<_>>`/`RwLock` around strategy, book, risk, portfolio, or order
  state;
- core affinity, scheduler policy changes, busy polling, SPSC/ring buffers,
  custom allocators, unsafe lock-free structures, `io_uring`, kernel bypass,
  hardware timestamping, or a custom Tokio runtime;
- a public API/config/schema migration; or
- a broad cleanup mixed into a performance commit.

If one of those is required, classify Propose Redesign and continue to Phase 5
without implementing it.

## Phase 5: Architecture Decision

Create `docs/target-host-runtime-architecture-decision.md` containing:

- the exact target use case and measurement contract;
- result hashes and a compact comparison table;
- dominant cost attribution;
- determinism, authority, and bounded-backpressure results;
- Keep, Optimize, or Propose Redesign;
- confidence, host-specific limitations, and invalidated runs;
- whether the current single-writer/Tokio-edge design has sufficient
  headroom; and
- the next smallest goal.

For Keep or Optimize, explicitly retain the current architecture and state why
the evidence does not justify deeper machinery.

For Propose Redesign, create a separate draft goal file. It must identify the
specific measured bottleneck and compare at least:

1. current single writer with host/core placement only;
2. staged normalization with the same canonical writer;
3. bounded SPSC handoffs with immutable messages; and
4. any more invasive option supported by the profile.

The proposal must define ownership, ordering, backpressure, shutdown,
recovery, replay, authority, migration, rollback, and A/B benchmark gates. It
must not implement the redesign or assume that thread-per-core is the answer.

## Phase 6: Verification And Handoff

Update `architecture.md`, `performance.md`, and `trading-readiness.md` with the
measured target-host result and its exclusions. Keep production entry
unavailable.

Run at minimum:

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

Also rerun all Goal D benchmark targets, canonical backtest/replay projections,
authority/compile-fail/dependency-policy tests, and the five recorded
target-host cohorts after the final candidate build.

Record exact commands, result hashes, invalidated runs, phase commits, and
deferrals in the Goal E handoff.

## Stop Conditions

Stop and report the exact conflict when:

- Goal D is incomplete or Reap/`../imm-strategy` has unexplained changes;
- no declared target host or predeclared acceptance contract exists;
- a required workload needs credentials or authenticated exchange access;
- the selected pre-optimization candidate fails the recorded Goal D canonical
  Chaos gates or changes their measured-workload decisions/outputs;
- a change introduced during Goal E alters an output, decision, RNG draw,
  floating-point result, timer order, authority, schema, dependency, or
  canonical hash outside a separately reviewed narrow optimization;
- observability allocates, formats, clones dynamic labels, or locks on the
  per-event path;
- a source-policy guard passes only by broadening an authority allowlist;
- benchmark logical counters differ nondeterministically;
- valid repeated runs cannot be obtained because the host is throttled,
  contended, unstable, or lacks the required monotonic timing facility;
- the measured bottleneck requires a concurrency, queue, allocator, kernel,
  timestamp, or public-schema redesign; or
- completion would require demo/production credentials, capital, external
  coordination, or production approval.

Do not weaken a threshold, discard a valid bad run, or relabel local/dev-host
evidence as target-host evidence to finish.

## Completion

Goal E is complete only when:

- every applicable phase and global gate is green;
- target-host results are source-bound, replayable, and machine-readable;
- logical outputs remain deterministic and byte-identical;
- latency tails, due-action lateness, queue age, allocations, throughput, and
  headroom are reported against predeclared thresholds;
- the dominant costs are profiled;
- the Keep/Optimize/Propose Redesign decision is explicit and justified;
- no unauthorized connectivity, order authority, schema, dependency, or
  canonical owner was added;
- documentation and the Goal E handoff record exact evidence and limitations;
  and
- Reap and `../imm-strategy` are clean.

Completion qualifies a runtime architecture decision for one declared target
host and workload. It does not authorize a credentialed demo, production
trading, or a general colocated-HFT claim.
