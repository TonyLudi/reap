# Chaos Connectivity Goal B Execution Prompt

Status: ready to run after the completed Goal A handoff.

Use this document as the complete instruction set for Goal B. The short
invocation is:

> Implement Goal B exactly as specified in
> `docs/chaos-connectivity-goal-b-prompt.md`. Continue through green phase
> gates and stop only at completion or a documented stop condition.

## Objective

Implement Goal B, Phases 6–9, from
[chaos-connectivity-refactor-plan.md](chaos-connectivity-refactor-plan.md).
Reduce structural coupling without weakening or broadening the authority graph
enforced by Goal A.

Treat [chaos-connectivity-boundary.md](chaos-connectivity-boundary.md) as
normative. Use
[chaos-connectivity-goal-a-handoff.md](chaos-connectivity-goal-a-handoff.md)
as the verified starting record and
[chaos-connectivity-inventory.md](chaos-connectivity-inventory.md) as the
before/after capability inventory.

Goal B does not add production-readiness features and does not claim that the
repository is production ready.

## Required Starting State

The Goal A implementation and documentation baseline is commit
`1fbf8955097fdb29fc38b04866005aa1f7095bee`. The starting `HEAD` MUST contain
that commit. A local `goal-a-complete` tag may identify it but is not required.

Before editing, verify that:

- the Reap worktree is clean;
- the Goal A baseline is an ancestor of `HEAD`;
- `../imm-strategy` is present, readable, and clean at
  `b6b120c7b7c466d8431bf082f3229328c5d7b2ae`;
- `reap_core::PINNED_JAVA_REVISION` equals that full SHA;
- every checked-in evidence binding that names the Java reference equals that
  full SHA;
- no other session is writing to this worktree;
- the Goal A handoff and Tranche A gate record remain valid; and
- there is sufficient disk space for locked workspace tests and a release
  build.

Do not modify `../imm-strategy`.

## Execution Discipline

Work phase by phase. At each phase:

1. inspect the current dependency and authority surface before editing;
2. make the smallest responsibility-preserving change;
3. keep mechanical moves separate from interface or semantic changes;
4. run focused tests proportionate to the change;
5. verify affected deterministic fixtures, hashes, and visibility boundaries;
6. inspect the complete diff;
7. record the commands and results;
8. commit the phase separately only when its exit gate is green; and
9. continue automatically while green.

Preserve:

- the Goal A Chaos/iarb2 authority boundary;
- ordered Quote and Hedge intents and CancelOwned ownership checks;
- risk decisions, event ordering, shutdown ordering, and single-writer
  semantics;
- exact serialized journal, report, evidence, configuration, and order-intent
  formats;
- deterministic Java-parity fixtures and backtest semantic hashes;
- private-state convergence, regular Cancel All After, the read-only
  forbidden-domain sentinel, and independent emergency-domain progress; and
- the separation of live execution from emergency and raw wire authority.

Do not move broad authority into a lower-level crate or a cleaner-looking
module.

## Phase 6: Move Pure Contracts Below Live

- Inventory every reason `reap-backtest` depends on `reap-live`.
- Move only pure shared configuration, report, evidence, provenance, and
  verification contracts to the smallest appropriate lower-level crate.
- Prefer an existing pure crate. Add a narrowly named contracts crate only when
  correct ownership cannot fit without a dependency cycle.
- Keep networking, credentials, runtime tasks, host inspection, transport, and
  emergency authority out of shared contract crates.
- Preserve serialized formats, semantic hashes, and public behavior.
- Remove the normal `reap-backtest -> reap-live` dependency.
- Move one contract family per commit where practical, followed by a separate
  dependency-removal commit.

Phase 6 is complete only when backtest/research compiles without `reap-live`,
`cargo tree` shows no normal edge, and shared contract crates contain no
network or credential dependency. Gate Phase 6 before starting Phase 7.

## Phase 7: Split State By Responsibility

- Split `reap-live/src/runtime.rs` into composition, connectivity, dispatch,
  readiness/safety, reconciliation, and shutdown responsibilities.
- Give each subsystem explicit state and narrow inputs and outputs.
- Retain one `LiveRuntime` coordinator that owns mutation ordering.
- Do not introduce concurrent mutation of strategy, risk, or canonical order
  state, including through new `Arc<Mutex<_>>` wrappers.
- After Phase 6 dependency inversion, split backtest research into pure
  configuration, execution, reporting, and verification responsibilities.
- Split capture only where real responsibility boundaries exist. Do not chase
  an arbitrary line-count target.
- Keep emergency workflow code with the emergency roles established by Goal A.
- Commit mechanical moves before separately narrowing interfaces.

Phase 7 is complete only when module ownership follows the trust/capability
planes, the single-writer architecture remains explicit in types and tests,
and event-order and shutdown/cancel-before-reconcile fixtures remain
unchanged. Coordinator tests MUST continue to cover disconnect, partial fill,
ambiguity, storage failure, and restart.

## Phase 8: Centralize Safety Semantics And Public Surfaces

- Centralize duplicated safety-event classification and fail-closed transition
  decisions used by configuration, runtime, and verification.
- Keep pure decisions separate from side effects and transport.
- Replace remaining exports that cross the defined authority boundaries with
  explicit, role-oriented exports.
- Do not turn this into workspace-wide API-style cleanup.
- Document crate ownership and forbidden dependency directions.
- Add dependency and visibility checks that prevent emergency or raw wire
  authority from leaking back into live execution.
- Preserve every Goal A compile-fail boundary and capability restriction.

Phase 8 is complete only when one named contract owns each safety
classification, public APIs correspond to supported roles, and no semantic
fixture changes.

## Phase 9: Global Verification And Documentation

Update the capability inventory and deviation ledger with the final
implementation. Update architecture, mapping, operations, trading readiness,
sample configuration, and CLI help to reflect the final implementation and make
the distinctions below.

The final documentation MUST distinguish:

- current Chaos strategy capabilities;
- Reap safety hardening;
- offline evidence;
- account-wide emergency recovery; and
- capabilities explicitly not implemented.

Record exact commands, results, hashes, phase commit SHAs, and remaining
explicit deferrals in a Goal B handoff document.

Create the repository-local temporary directory and run at minimum:

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

Also run:

- all role-visibility and compile-fail fixtures;
- endpoint/channel and adapter allowlist tests;
- deterministic Java-parity strategy fixtures;
- backtest semantic snapshots;
- configuration migration tests;
- a dependency check proving `reap-backtest` has no normal `reap-live`
  dependency;
- checks proving shared contract crates contain no network or credential
  dependency; and
- checks proving emergency and raw wire authority remain absent from normal
  live execution.

## Stop Conditions

Stop and report the exact conflict when:

- `../imm-strategy` is absent, unreadable, dirty, or differs from the pinned
  full SHA;
- `reap_core::PINNED_JAVA_REVISION` or any checked-in evidence binding names a
  different Java revision;
- another session creates overlapping or unexplained worktree changes;
- a removal or move changes ordered intents, backtest output, risk decisions,
  event timing, or shutdown ordering;
- a required capability cannot be traced to the pinned Chaos scope or an
  explicit fail-closed Reap safety invariant;
- narrowing would weaken private-state convergence, regular Cancel All After,
  forbidden-domain detection, or independent emergency recovery;
- a narrow API remains bypassable through a public signer, raw client,
  transport, constructor, export, or transitive dependency;
- completion requires a journal/report/evidence schema migration, a
  configuration change beyond the bounded Phase 4 migration, real credentials,
  authenticated exchange access, target-host deployment, or production
  approval;
- the work would add a venue, strategy, order type, generic framework,
  master/group feed, or production-readiness feature; or
- a safe mechanical split cannot preserve single-writer ownership.

Do not weaken an invariant to finish the goal. Record the blocker and propose a
separately scoped follow-up.

## Explicit Exclusions

Do not:

- use credentials or make authenticated exchange requests;
- regenerate or relabel production evidence;
- add production order entry or deployment support;
- add amend, algo/spread placement, or new regular order profiles;
- add another venue, strategy, or generic plugin/framework surface;
- perform a broad numeric/domain-newtype migration;
- route backtest through a different engine or risk path when that changes
  semantics; or
- claim production readiness.

## Completion

Goal B is complete only when:

- Phases 6–9 and the full-plan structural acceptance criteria are green;
- `reap-backtest` no longer normally depends on `reap-live`;
- shared contracts contain no runtime, network, credential, or emergency
  authority;
- responsibility-based modules preserve explicit single-writer ownership;
- all Goal A authority boundaries remain enforced;
- all required final gates pass;
- the handoff accurately records hashes, commands, results, and exclusions; and
- Reap and `../imm-strategy` are clean.

Completion means that the defined boundary is easier to change safely. It does
not mean the repository is production ready.
