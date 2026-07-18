# Maintainability Refactor Goal C Execution Prompt

Status: active execution contract until the required Goal C handoff records
every gate green. This goal is a source-structure and change-safety milestone.
It does not authorize demo trading, production order entry, or a low-latency
runtime redesign.

Use this document as the complete instruction set. The short invocation is:

> Execute Goal C exactly as specified in
> `docs/maintainability-refactor-goal-c-prompt.md`. Continue phase by phase
> through every green gate and stop only at completion or a documented stop
> condition.

## Objective

Reduce the repository's remaining god-file, god-function, and oversized-state
risks without changing trading behavior, authority, event ordering, serialized
formats, dependency direction, or measured hot-path characteristics.

Keep the existing deterministic single-writer architecture. Split source by
responsibility; do not split canonical trading state among concurrent services.

The required order is:

1. decompose `ChaosStrategy` and `InstrumentState`;
2. stage `LiveRuntime::build` and split runtime event handlers;
3. shorten coordinator feed and intent routing while retaining one owner;
4. decompose `BacktestRunner`;
5. split the oversized offline economic-statement and production-evidence
   validators; and
6. run global structural, deterministic, authority, and performance gates.

## Normative Baseline

The starting implementation baseline is commit
`13d5ac17197e758acd5195b58fea4e3440881f9c`. Starting `HEAD` MUST contain that
commit.

Treat these documents as normative:

- [chaos-connectivity-boundary.md](chaos-connectivity-boundary.md) for
  capability and authority;
- [chaos-connectivity-goal-b-handoff.md](chaos-connectivity-goal-b-handoff.md)
  for the completed structural baseline;
- [architecture.md](architecture.md) for ownership and event-flow invariants;
- [performance.md](performance.md) for the current measured regression
  baseline; and
- [trading-readiness.md](trading-readiness.md) for the distinction between
  structural completion and trading approval.

The behavioral reference remains the clean sibling checkout
`../imm-strategy` at
`b6b120c7b7c466d8431bf082f3229328c5d7b2ae`. Do not modify it. Only behavior
reached by the supported Chaos/iarb2 path is normative; generic gateway,
`ExecAlgo`, unrelated strategy, and eight-session-pool code does not broaden
Reap's scope.

Before editing, verify:

- Reap has no unexplained worktree changes and no other session is writing to
  overlapping files;
- the Goal C baseline and completed Goal B handoff are present;
- `../imm-strategy` is present, readable, clean, and at the pinned full SHA;
- `reap_core::PINNED_JAVA_REVISION` and checked-in evidence bindings equal that
  SHA;
- `Cargo.lock`, deterministic fixtures, example configurations, and current
  semantic output hashes are recorded;
- the current largest-file, largest-function, and state-field inventory is
  recorded in the Goal C handoff; and
- the two performance benchmarks below are run three times on an otherwise
  idle host to establish same-host pre-change medians and allocation counts.

## Non-Negotiable Invariants

Preserve exactly:

- the plan-derived Chaos connectivity boundary and three executable purposes:
  Quote, Hedge, and CancelOwned;
- ordered strategy intents, floating-point operation order, stable traversal
  order, Java-compatible random-number consumption, timestamps, and timer
  behavior;
- risk decisions, halt promotion, readiness transitions, reconciliation,
  cancel-before-reconcile shutdown, and fail-closed behavior;
- the one `LiveRuntime`/`LiveCoordinator` mutation owner and absence of
  `Arc<Mutex<_>>` canonical strategy, risk, book, portfolio, or order state;
- the current biased event-loop branch priority and all bounded-channel
  saturation behavior;
- synchronous canonical `PendingNew` reservation before submit dispatch,
  owned-order proof before cancel, durable safety-latch ordering, and storage
  record ordering;
- adapter-owned command transport, exact-plan private-feed bootstrap, evidence
  read-only authority, and separately composed emergency authority;
- public crate APIs, paths, and re-exports exactly;
- journal, report, evidence, configuration, CLI output, and order-intent
  schemas and canonical hashes;
- backtest scheduling, matching, funding, accounting, tie-breaking, replay
  order, and current partial live/backtest parity boundary; and
- every Goal A and Goal B dependency, source-location, visibility, and
  compile-fail guard.

Moving a constructor to a new file does not authorize broadening a lexical
source allowlist. Update an allowlist only to the exact new owning file, retain
the same crate and visibility boundary, and add or retain a negative bypass
test.

Do not use `TRYBUILD=overwrite` across a suite. Review every changed diagnostic
fixture and accept only path or line-span changes with the same rejected
operation and privacy/type reason.

## Execution Discipline

Work phase by phase. For each phase:

1. inventory responsibilities, callers, visibility, tests, and source-policy
   guards before moving code;
2. make mechanical moves before state grouping or interface cleanup;
3. preserve Git history with focused moves and small commits;
4. run focused tests and deterministic comparisons;
5. run the affected benchmark after hot-path phases;
6. inspect the complete diff, public exports, dependency graph, and generated
   diagnostics;
7. record commands, results, file/function sizes, and any justified deferral
   in `docs/maintainability-refactor-goal-c-handoff.md`;
8. commit the phase only when its gate is green; and
9. continue automatically while green.

Do not opportunistically fix strategy parity, trading behavior, production
readiness, or unrelated defects. Record them as separately scoped follow-ups.

For structural measurements, "production lines" means non-test code excluding
`#[cfg(test)]` modules and test-only files. Record test lines separately. A
target stated as "total lines" includes both production and locally co-located
tests. Do not game either measure by moving an unchanged monolith behind
`include!`, renaming it, or transferring every test to one new oversized file.

## Phase 0: Baseline And Structural Inventory

Create the Goal C handoff with placeholders and record:

- exact Reap and Java revisions and clean-state checks;
- production and test line counts for the target files;
- field and method counts for `ChaosStrategy`, `InstrumentState`,
  `LiveRuntime`, `LiveCoordinator`, and `BacktestRunner`;
- spans and responsibilities of the largest functions;
- current crate dependencies and public re-exports;
- source-location and compile-fail tests affected by moves;
- deterministic fixture and CLI-output hashes; and
- one warm-up plus three recorded results for:

```bash
cargo bench -p reap-engine --bench event_loop --locked
cargo bench -p reap-live --bench live_loop --locked
```

Phase 0 changes documentation only. Gate it before production-code movement.

## Phase 1: Decompose Chaos Strategy State And Logic

Replace the `reap-strategy/src/chaos.rs` monolith with responsibility-oriented
modules. The exact private layout may follow the code, but it MUST distinguish:

- configuration, defaults, and validation;
- aggregate event dispatch and deterministic refresh ordering;
- reference freshness, basis, and interval health;
- inventory, balances, position, PnL, and risk state;
- theoretical pricing, skew, and quote construction;
- hedge candidates, selection, sizing, and missed-hedge tracking;
- active quote/hedge and fill execution state; and
- per-instrument state and calculations.

Keep `ChaosStrategy` as the public aggregate and preserve existing public
imports through narrow re-exports. Use inline, by-value private substates where
grouping improves cohesion. Do not add trait objects, heap-owned services,
locks, async work, channels, or cloned shadow state.

Preserve the exact call order in every refresh and event path. Avoid
"equivalent" iterator, collection, or floating-point rewrites that can change
rounding, traversal, allocation, or RNG consumption.

Structural exit criteria:

- `chaos.rs` is no longer a renamed monolith; the aggregate facade and modules
  have named responsibilities;
- co-located tests are split by responsibility instead of preserving one
  multi-thousand-line test module, and no facade file remains large only
  because all tests stayed in it;
- no new strategy production module exceeds 1,500 lines without a written,
  reviewed justification in the handoff;
- `ChaosConfig::validate` is an orchestration method delegating named checks
  rather than one approximately 547-line function;
- `ChaosStrategy::check_risk_limits` and the largest instrument calculations
  delegate cohesive decisions rather than mixing all state families;
- the public capability surface is unchanged; and
- strategy, engine, live, backtest, Java-parity, determinism, compile-fail, and
  benchmark gates are green.

## Phase 2: Stage Live Runtime Construction And Dispatch

Retain one private `LiveRuntime` with the explicit composition, connectivity,
dispatch, readiness/safety, reconciliation, shutdown, and coordinator state.

Extract named startup stages for:

- plan and preflight validation;
- storage lease/recovery and host checks;
- authenticated account bootstrap;
- coordinator construction and restoration;
- public/private feed construction;
- order-command gateway installation;
- safety, reconciliation, alert, operator, and host tasks; and
- final ownership transfer into a completely built runtime.

Partially built stages MUST retain abort-on-drop or explicit rollback behavior.
Do not leak tasks, credentials, gateways, storage leases, or command transports
on an early return.

Split `RuntimeEvent` handling into narrow raw-feed, connectivity,
order-transport, submit/cancel result, reconciliation, operator, safety, commit,
and shutdown handlers. Keep the outer event loop and all canonical mutation on
the same owner.

Split the current co-located runtime tests by the same responsibilities. Do not
increase production visibility only to make moved tests compile.

The source guards in `reap-live/tests/dependency_policy.rs` currently pin
`ProvenRegularSubmitRequest`, `take_approval_scope`, and the sole
`.start_and_install(` call to exact owning files. If construction moves, change
each expected path as a one-for-one replacement in a separately reviewed
commit. Never broaden the expected set, increase its cardinality, change its
count, or admit a directory or wildcard.

The
`production_runtime_keeps_single_owner_responsibility_state` regression test
parses the runtime source and names its responsibility modules. Extend it to
scan every new production runtime module while retaining exactly one
`LiveCoordinator`, no canonical `Arc<Mutex<_>>`, and no broad
`use super::*` production import.

Structural exit criteria:

- `LiveRuntime::build` is a readable orchestration function, targeted at no
  more than 150 lines;
- `handle_runtime_event` delegates event families and is targeted at no more
  than 120 lines;
- the top-level runtime facade is targeted at no more than 2,500 total lines,
  including its local tests, and no extracted production module exceeds 1,500
  lines without a written justification;
- normal market-data processing gains no new `.await`, blocking call, clone,
  allocation, or dynamic dispatch;
- event priority, storage/action ordering, startup rollback, and shutdown
  ordering are unchanged;
- source allowlists name exact new owners rather than directories or
  wildcards; and
- live, feed, order, adapter, storage, dependency-policy, compile-fail, fault,
  shutdown, and performance gates are green.

## Phase 3: Shorten Coordinator Reductions Without Splitting Authority

Keep `LiveCoordinator` as the sole aggregate that owns the engine, startup,
private reducers, regular execution policy, owned-order proofs, client IDs,
halts, and decision sequence.

Extract private helpers or by-value reducers for:

- private order/fill canonicalization and journal deduplication;
- account snapshot application and reconciliation transitions;
- engine-output record construction;
- typed Quote/Hedge/CancelOwned routing; and
- owned cancellation and fail-closed cancellation construction.

Do not introduce a second mutable owner or a lower-level type able to mint
approval, reservation, prepared-command, or recovered-ownership authority.

The phase is complete when the public coordinator entry points read as
ordered state transitions, long handlers delegate cohesive private decisions,
and all same-turn registration, ambiguity, fill deduplication, restart,
disconnect, storage-failure, and shutdown fixtures remain unchanged.
Split its co-located tests by responsibility so `coordinator.rs` is targeted at
no more than 2,500 total lines; keep test-only access private or `pub(crate)`,
never public.

## Phase 4: Decompose Backtest Runner

Keep `BacktestRunner` as the public deterministic facade, but group its
approximately 67 fields and split implementation responsibility among:

- replay/input normalization;
- deterministic scheduled-action ordering;
- matching and order lifecycle;
- portfolio, currency conversion, and valuation;
- funding and carry state;
- accounting completeness and failure tracking; and
- metrics and report construction.

Do not route backtest through `reap-live`, add authenticated/runtime
dependencies, or adopt a different engine/risk path in this goal. Preserve the
documented partial parity boundary and all output bytes, hashes, timestamps,
tie-breaking, and sampled-latency usage.

No new backtest production module may exceed 1,500 lines without a written
justification. The normal `reap-backtest -> reap-live` dependency MUST remain
absent.

## Phase 5: Split Offline Assurance Validators

Split `economic_statement.rs` by artifact loading, trade-bill validation,
funding-bill validation, position-basis reconstruction, cash continuity, and
report construction.

Split `production_evidence.rs` by manifest loading, source-verifier execution,
cross-artifact bindings, policy/time checks, and report construction.

Preserve strict validation order, error classification/text where externally
observed, source freshness rules, fail-closed unknown-field behavior, canonical
serialization, and artifact hashes. Do not weaken a check to make extraction
easier.

Target no assurance validation function above 250 lines and no new production
module above 1,500 lines. Any exception requires a responsibility-based
justification and focused tests in the handoff.

## Performance And Determinism Gate

After Phases 1–3 and again at completion, run one unrecorded warm-up and then
each benchmark three times on the same idle host and toolchain:

```bash
cargo bench -p reap-engine --bench event_loop --locked
cargo bench -p reap-live --bench live_loop --locked
```

Require:

- identical workload counters and intent/action/record counts;
- exactly `999,996` engine benchmark intents unless the Phase 0 baseline stops
  the goal first;
- no increase in allocation calls or requested bytes per unit;
- investigation when the three-run median regresses by more than 5%;
- after eliminating host noise and repeating with five recorded warm runs when
  necessary, no greater than 10% same-host median regression; and
- a separately reviewed optimization rather than mixing performance changes
  into a mechanical extraction.

This goal MUST NOT introduce CPU affinity, isolated cores, a custom allocator,
busy spinning, SPSC/ring-buffer queues, kernel bypass, hardware timestamps, a
custom Tokio runtime, or other latency architecture. Those require target-host
end-to-end p50/p99/p99.9 and burst evidence plus a separate goal.

Run the canonical CLI backtest twice and require byte-identical output:

```bash
mkdir -p /home/ubuntu/code/reap/target/tmp
cargo run --locked -q -p reap-cli -- \
  backtest \
  --format normalized-jsonl \
  --config examples/iarb2-basic.toml \
  --data fixtures/normalized/chaos_quote_hedge.jsonl \
  --pretty >target/tmp/goal-c-backtest-1.json
cargo run --locked -q -p reap-cli -- \
  backtest \
  --format normalized-jsonl \
  --config examples/iarb2-basic.toml \
  --data fixtures/normalized/chaos_quote_hedge.jsonl \
  --pretty >target/tmp/goal-c-backtest-2.json
cmp target/tmp/goal-c-backtest-1.json target/tmp/goal-c-backtest-2.json
sha256sum target/tmp/goal-c-backtest-1.json
```

The expected Goal B semantic output hash is
`38acf9f5e0c310f2ec5528974beffadf4c1a7f84d46efa8d9664ee7051e84691`.
If the baseline command at Phase 0 differs, stop and explain before editing.

These checked-in deterministic anchors MUST also remain exact:

| Artifact | SHA-256 |
| --- | --- |
| `Cargo.lock` | `d8a19fb100aeb4e542a2135d546edfb5ae24629717f5ab65e285cf9bfe483b02` |
| `fixtures/normalized/chaos_quote_hedge.jsonl` | `27f2eb4b9dba7ee600ed645ad8b7c88143e8b54531232991b492cb7595e8ccaa` |
| `fixtures/normalized/chaos_quote_hedge_later.jsonl` | `40453b8be283178b20531c84142dbaeeeca82b4723e5c13594df171c778cd8ee` |
| `fixtures/normalized/chaos_quote_hedge_intents.json` | `d95fa7f121e2e8c402c8108cf9fefb7c7d7b3dbd2b9742c58c234a521f0ee0ec` |
| `examples/iarb2-basic.toml` | `0fac5a3a35fe28cdc05118b7e22241077aa7f604a9a5436355797605b51b3b26` |
| `examples/live-okx-demo.toml` | `caea78e0a75d2586ecbd16d5b4414f9606a7064b6e1684f82fff2d132a197195` |

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

Also verify explicitly:

- all Java-parity, strategy determinism, backtest semantic, configuration, and
  canonical-plan tests;
- `cargo test -q -p reap-strategy --locked
  chaos::tests::normalized_fixture_typed_output_preserves_exact_ordered_intents
  -- --exact`;
- all role-visibility, authority, private-bootstrap, storage-proof, and
  compile-fail suites;
- `cargo test -p reap-live --test dependency_policy --locked`;
- all dependency and lexical source-location policies;
- no `Cargo.toml`, workspace dependency, or `Cargo.lock` change;
- no normal backtest-to-live dependency;
- no live reachability to emergency mutation or raw command transport;
- no schema, fixture, sample configuration, or pinned-Java change;
- before/after public re-exports and visibility;
- before/after largest production files, functions, state aggregates, and test
  locations; and
- the final performance and deterministic-output gates above.

Create `docs/maintainability-refactor-goal-c-handoff.md` and record exact phase
commits, commands, results, hashes, benchmark runs, structural measurements,
remaining justified large components, and explicit deferrals.

## Stop Conditions

Stop and report the exact conflict when:

- Reap or `../imm-strategy` has unexplained/overlapping changes or the Java
  checkout differs from the pinned revision;
- a move changes intent order, RNG consumption, floating-point result,
  timestamp, risk decision, report bytes, backtest output, event priority,
  storage/action ordering, or shutdown behavior;
- a split requires new authority, a broader export, a dependency inversion, a
  schema/configuration migration, or a new workspace dependency;
- a source-policy guard can pass only by broadening its allowlist;
- a partially built runtime can leak a task, lease, credential, gateway, or
  transport;
- a benchmark changes logical counters or allocations, or repeatedly exceeds
  the timing-regression threshold;
- a safe split would require concurrent canonical mutation, `Arc<Mutex<_>>`,
  new hot-path dynamic dispatch, or an additional hot-path `.await`;
- completion requires credentials, authenticated exchange access, target-host
  deployment, production approval, or modification of `../imm-strategy`; or
- a discovered defect requires a semantic fix rather than a mechanical
  extraction.

Do not weaken an invariant or rewrite a test to finish. Record the blocker and
propose a separately scoped follow-up.

## Explicit Exclusions

Do not:

- implement the documented public-trade-driven implied-depth parity gap;
- add production order entry, another venue, strategy, order type, regular
  profile, algo/spread placement, amend, or generic plugin/framework support;
- redesign Tokio, queues, threads, core placement, network IO, timestamps,
  persistence durability, or command concurrency;
- merge live, backtest, evidence, capture, or emergency composition roots;
- alter journal/report/evidence/configuration/order-intent schemas;
- use credentials or make authenticated exchange requests;
- regenerate, relabel, or claim passing operational/production evidence; or
- claim that structural completion makes Reap production ready or
  colocated-HFT ready.

## Completion

Goal C is complete only when:

- every phase and focused gate is green;
- the named god files and functions are decomposed by responsibility rather
  than merely renamed;
- one deterministic live writer and one deterministic backtest writer remain
  explicit;
- all Goal A and Goal B capability and authority boundaries remain enforced;
- deterministic outputs, schemas, fixtures, dependencies, and public behavior
  are unchanged;
- benchmark logical/allocation gates are unchanged and median timing remains
  within the defined same-host tolerance;
- the full final verification passes;
- the handoff records exact evidence and remaining deferrals; and
- Reap and `../imm-strategy` are clean.

Completion means the current bounded Chaos implementation is easier and safer
to change. It does not approve a demo campaign, production trading, or a
low-latency runtime redesign.
