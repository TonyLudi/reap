# Chaos Connectivity Refactor Plan

Status: proposed implementation plan, not yet implemented.

This is a finite, goal-ready refactor plan for enforcing
[chaos-connectivity-boundary.md](chaos-connectivity-boundary.md) and then
reducing the repository's structural coupling without changing Chaos behavior.
The boundary document is normative. This plan defines sequencing, evidence,
acceptance gates, and stop conditions.

Related context:

- [architecture.md](architecture.md) defines the single-writer runtime model.
- [chaos-mapping.md](chaos-mapping.md) records decision parity and intentional
  differences from Java.
- [operations.md](operations.md) defines live and emergency procedures.
- [trading-readiness.md](trading-readiness.md) remains the production-readiness
  gate; this refactor does not satisfy that gate.

## Goal Execution Contract

Objective:

> Constrain Reap's live exchange surface to capabilities required by the
> current Chaos/iarb2 implementation, preserve fail-closed risk observation,
> isolate account-wide emergency cleanup, and remove structural coupling that
> lets those authorities leak across crate and runtime boundaries.

The behavioral reference remains the sibling checkout `../imm-strategy` at:

```text
b6b120c7b7c466d8431bf082f3229328c5d7b2ae
```

The relevant Java scope is `chaos/chaos-core`, `chaos/chaos-iarb2`, and only
supporting classes reached by that strategy. Generic Luban/metcoin capabilities
are not requirements merely because they exist.

The sibling checkout must exist, be readable, have no tracked, staged, or
untracked changes, and resolve to the pinned full SHA. A clean HEAD alone is not
enough if the working tree changes the referenced source.

### Concurrency Rule

Pause any other code-writing production-readiness goal before Phase 0. A second
session may review or run read-only checks, but it MUST NOT modify overlapping
files, dependencies, configs, fixtures, or evidence schemas while this goal is
active. Resume and rebase the production-readiness work only after this
boundary refactor is merged and re-audited.

This rule does not make refactoring more important than production readiness;
it keeps the capability baseline and deterministic comparison trustworthy in a
shared worktree.

### Reviewed Baseline

This plan was prepared against Reap commit
`52e3c35adffe2e1229071e182707f6de872c198a` and a clean sibling Java checkout.
The documentation commit will move Reap's HEAD. Phase 0 MUST record the actual
starting Reap commit rather than requiring this reviewed SHA.

Known current sizes and coupling points are:

| Area | Reviewed state |
| --- | --- |
| `reap-live/src/runtime.rs` | 8,193 lines and multiple connectivity/readiness/shutdown responsibilities |
| `reap-live/src/emergency.rs` | 2,301 lines and broad emergency authority inside `reap-live` |
| `reap-capture/src/lib.rs` | 2,950 lines |
| `reap-backtest/src/research.rs` | 4,651 lines |
| `reap-backtest` dependencies | Directly depends on `reap-live` |
| OKX REST surface | One public, cloneable client exposes live, evidence, and emergency endpoint families |
| OKX signing surface | Public arbitrary request signing and public raw transport execution |
| Public exports | Wildcard exports in venue and live roots expose broad surfaces |

Line count alone is not a defect or an acceptance metric. The issue is mixed
authority and responsibility.

## Non-Negotiable Invariants

1. Do not change strategy math, event ordering, quote selection, hedge
   allocation, risk thresholds, halt decisions, or deterministic backtest
   semantics.
2. Keep `reap_core::PINNED_JAVA_REVISION` and every evidence binding at the
   exact Java SHA above.
3. Preserve the deterministic single-writer owner for strategy, risk, and
   canonical order state. Do not introduce a generic actor or strategy
   framework.
4. Chaos may mutate only regular PostOnly quotes, regular IOC
   `CancelMaker` hedges, and owned regular cancellations as defined by the
   boundary.
5. The live process may inspect algo/spread state only to prove it empty. It
   may not mutate those domains.
6. Account-wide emergency regular/algo/spread cleanup remains available, but
   in a dedicated crate/executable outside live Chaos authority and with no
   submit capability.
7. Production order entry remains unavailable. Do not use real credentials,
   contact authenticated exchange endpoints, or generate/bless production
   evidence during this refactor.
8. Preserve journal, report, evidence, and fixture schemas. Phase 4 may make
   one explicitly documented, backward-compatible live-config migration for
   connection planning; no other serialized-schema change is allowed.
9. Preserve one public/private normalized event API for live and backtest.
   Moving ownership is allowed; silently changing meaning is not.
10. Work phase by phase. Do not mix production-readiness features or unrelated
    cleanup into a phase.

## Current Deviation Ledger

Phase 0 must verify and expand this ledger before code changes:

| ID | Current implementation | Target |
| --- | --- | --- |
| `D01` | Public `OkxSigner::signature`/`sign_request`/`websocket_login`/credential access, `OkxRestClient::signer`, `SignedRequest`, `HttpTransport::execute`, and the broad REST client are bypass paths | Private role-owned wire adapters and narrow authenticated session factories |
| `D02` | Wildcard `okx` and `reap-live` exports widen accidental reachability | Explicit exports grouped by role |
| `D03` | Rust live `MarketEvent::Trade` only advances time, but pinned Java uses public trades to invalidate implied OKX depth and schedule repricing | Retain plan-derived live trades and record the behavior gap; do not remove them in this no-semantic-change refactor |
| `D04` | Config defaults to eight order websocket sessions; each family hashes to one session, there is no safe failover, and readiness requires every session | Start with one non-idle command shard per account; add shards only for measured capacity/isolation and model real replicas separately |
| `D05` | The live maintenance filter treats spread trading as fatal to Chaos | Derive service/product relevance from the Chaos plan |
| `D06` | Live regular reconciliation does not prove pending algo/spread domains empty | Add a read-only startup and recurring forbidden-domain sentinel |
| `D07` | Emergency enumeration combines regular, seven algo query families, and spread before cancellation; an unsupported-domain failure can delay regular mitigation | Independent per-domain workflows and deadlines |
| `D08` | Emergency mutation code lives in `reap-live` and uses the same broad client surface | Dedicated emergency crate/executable and adapter absent from the live dependency graph |
| `D09` | `reap-backtest` depends on the entire `reap-live` crate for shared contracts | Move pure shared contracts down and remove the dependency |
| `D10` | Coordinator, connectivity, safety, evidence, and shutdown responsibilities are concentrated in large modules | Split by owned state and role while retaining one coordinator |
| `D11` | One global public replica count is applied without a per-requirement redundancy rationale | Resolve replica count per requirement and name the dedup/recovery consumer for every extra socket |
| `D12` | Private orders/account/positions/fills are partitioned one channel per authenticated socket | Pack compatible required channels after permutation/failure tests; split only for a tested ordering/isolation/capacity reason |

The ledger is not a complete endpoint inventory. Phase 0 must classify every
current OKX REST method, websocket operation/channel, maintenance filter, and
connection constructor, including test-only and offline-evidence use.

## Deliverables

The completed goal produces:

1. a checked-in `docs/chaos-connectivity-inventory.md` with requirement IDs,
   endpoint/channel, read/write classification, owning plane, production
   reachability, and disposition;
2. deterministic `ChaosConnectivityRequirements` and
   `ChaosConnectivityPlan` contracts plus separate narrow emergency, capture,
   evidence, and fault-tool contracts that are never unioned into live Chaos;
3. typed Chaos execution purposes and a validating regular execution policy;
4. role-specific OKX clients with a private raw wire layer;
5. plan-derived subscriptions, order-command lanes, readiness, and maintenance
   relevance;
6. a read-only forbidden-domain sentinel;
7. independently progressing emergency regular/algo/spread cleanup outside the
   live execution authority;
8. removal of the `reap-backtest -> reap-live` dependency and responsibility-
   based runtime/backtest modules;
9. negative reachability, allowlist, parity, and failure-isolation tests; and
10. updated architecture, mapping, operations, readiness, and config migration
    documentation.

## Execution Units

Run this plan as two goals, not one unbounded session:

- **Goal A — capability boundary:** Phases 0 through 5, followed by the Tranche
  A verification gate below. This is the prerequisite for resuming overlapping
  production-readiness code changes.
- **Goal B — structural decomposition:** Phases 6 through 9, started only from
  a clean, reviewed Goal A result.

Do not run Goal B concurrently with a resumed production-readiness writer. If
Goal B will follow immediately, keep that session paused through both goals.

Within either goal, “pause at a phase gate” means record the exact diff, tests,
hashes, and gate result, then continue autonomously when it is green. Human
approval is required only when a stop condition occurs or the user explicitly
requests per-phase approval.

## Tranche A: Enforce The Capability Boundary

Tranche A is mandatory and precedes broad module movement.

### Phase 0: Freeze And Inventory

Actions:

- Record the starting Reap SHA, `git status`, sibling Java SHA, Rust pin, Rust
  toolchain, and locked dependency state.
- Fail if `../imm-strategy` is absent/unreadable or
  `git -C ../imm-strategy status --porcelain` is nonempty.
- Verify no other session is modifying the worktree.
- Identify the deterministic Java-parity fixtures, ordered-intent snapshots,
  backtest semantic hashes, and relevant config validation tests. Run them and
  retain their exact commands, exit codes, fixture/report hashes, and results in
  the inventory's baseline section.
- Produce `docs/chaos-connectivity-inventory.md` with these columns:
  `capability_id`, endpoint or channel, operation, read/write, trust plane,
  mode, requirement ID, consumer, Java anchor or Reap safety rationale,
  current reachability, and target disposition.
- Classify every item as `ChaosExecution`, `ChaosObservation`,
  `ReadinessSafety`, `EmergencyCleanup`, `EvidenceOnly`, `TestOnly`, or
  `Remove`.
- Mark capabilities implemented by a shared raw client even when no current
  caller exists; dormant public authority is still authority.
- Add a machine-readable capability registry used by endpoint/channel/session
  constructors. A test MUST fail when a registered production operation has no
  inventory capability ID or when an allowed live operation has no boundary
  requirement ID. Phase 0 may use a reviewed manual inventory to bootstrap the
  registry, but the global gate is mechanical.

Exit gate:

- Every existing OKX endpoint, websocket operation/channel, subscription,
  connection, and maintenance filter is classified.
- The inventory records the exact baseline commands, results, and hashes.
- The sibling full SHA and Rust pin match.
- Baseline deterministic checks pass.
- There are no behavior changes.

Suggested commit: inventory and baseline tests only.

### Phase 1: Resolve One Concrete Chaos Plan

Actions:

- Add a concrete `ChaosConnectivityRequirements` derived from effective
  `ChaosConfig` plus explicit risk/account requirements. Do not introduce a
  generic multi-strategy capability framework.
- Resolve it into `ChaosConnectivityPlan` at the live composition boundary.
  The plan includes symbols, public and private subscriptions, authenticated
  reads, allowed regular mutations, forbidden-domain checks, order dispatch
  families, required command lanes, per-requirement public replicas, private
  channel packing/isolation, maintenance relevance, and mode.
- Attach a stable requirement ID from the boundary to every plan item.
- Keep venue-neutral decision requirements below `reap-live`; map them to OKX
  channels and endpoints only at the venue/live edge.
- Define a schema-versioned canonical secret-free JSON representation: sorted
  arrays, stable enum strings, no maps with unspecified order, and a SHA-256
  over the exact bytes. Make plan construction deterministic, deduplicated,
  printable, and available to config validation.
- Keep emergency, capture, offline evidence, and fault-tool contracts separate.
  Do not create a generic multi-mode plan or union them into
  `ChaosConnectivityPlan`.
- Reject `act_on_burst = true` in live modes until an explicit burst provider
  is implemented.

Tests:

- Equal effective configs produce byte-identical canonical JSON and plan hash.
- Spot, linear swap, inverse derivative, duplicate underlying families, and
  stablecoin guard cases resolve exactly.
- No resolved item lacks a requirement ID or consumer.
- Live plans include `CHAOS-MD-TRADE` for every configured instrument and
  `SAFE-ACCOUNT-POSITIONS` for every live account.
- Mode tests prove Validate constructs no credentials/network roles, Observe
  constructs no regular mutation/CAA role, Demo constructs only the approved
  live roles, and production order entry remains unavailable.
- Invalid or unsupported settings fail before credentials or network setup.

Exit gate:

- Existing constructors can be compared against one exact plan.
- The plan contains no generic algo/spread mutation requirement.

Suggested commit: pure requirements/plan types and tests.

### Phase 2: Constrain Intent And Regular Execution

Actions:

- Introduce typed `Quote`, `Hedge`, and `CancelOwned` purposes at the Chaos
  output/execution boundary.
- Preserve the existing serialized `OrderIntent` shape if changing it would
  migrate journals or evidence. An internal typed purpose may validate and then
  map to the existing record. Legacy/free-form intents are untrusted
  evidence/backtest records and cannot reach live transport without the typed
  policy's full validation.
- Move final venue-field construction to a regular execution policy that
  receives instrument/account ownership and authenticated exchange limits.
- Encode the exact Reap profiles:
  - Quote: regular limit PostOnly, non-reduce-only, no per-order STP, allowed
    only after bootstrap/periodic verification that account
    `acctStpMode = cancel_maker`.
  - Hedge: regular limit IOC, non-reduce-only, `CancelMaker`.
  - CancelOwned: one canonical owned regular order.
- Reject unsupported TIF, order type, amend, reduce-only, STP, identifier
  domain, symbol, account, and ownership combinations before transport.
- Keep risk checks and synchronous canonical pending registration in their
  current order.

Tests:

- Existing normalized inputs produce the same ordered intents and reasons.
- Quote/hedge request snapshots are unchanged.
- Account bootstrap and drift checks reject any quote-capable account whose
  `acctStpMode` is not `cancel_maker`.
- Unknown, foreign, algo, and spread identifiers cannot pass CancelOwned.
- Property/table tests reject every unsupported field combination.
- Backtest matching remains deterministic and unchanged.

Exit gate:

- Only the three typed Chaos purposes can enter the live executable policy.
  A legacy serialized intent may remain representable but is not executable
  authority.
- No persisted schema changed.

Suggested commit: typed purposes and execution policy.

### Phase 3: Make Exchange Authority Role-Specific

Actions:

- Move arbitrary `signature`/`sign_request`, signer and credential getters,
  websocket login signing, signed request construction, and raw production
  transport execution behind private role-owned OKX wire adapters.
- Enforce this dependency graph (crate names may differ, but the edges may not):

  `reap-live -> live OKX adapter -> private wire/auth`

  `emergency executable -> emergency OKX adapter -> private wire/auth`

  `offline evidence executables -> evidence OKX adapter -> private wire/auth`

  All three may depend on a credential-free parser/contracts crate. There is no
  `reap-live -> emergency/evidence adapter` edge and no broad authenticated
  client in the shared parser/contracts crate.
- Replace the broad production `OkxRestClient` surface at live call sites with
  non-interchangeable `RegularExecution`, `RegularReconciliation`,
  `LiveSafety`, and `ForbiddenOrderObserver` ports.
- Give the live adapter narrow authenticated `PrivateStateSessionFactory` and
  `RegularOrderSessionFactory` factories. They expose only planned private
  channels and approved regular place/cancel websocket operations.
- Move `EmergencyAccountStop` and `EvidenceReadOnly` behind their separate
  adapter/composition crates in this phase, preserving behavior mechanically.
  Phase 5 changes emergency workflow semantics after the authority graph exists.
- Give each role an explicit endpoint/operation allowlist and a narrow
  constructor. A role MUST NOT expose its signer, wire transport, or conversion
  to a broader role.
- Keep request parsers reusable where they are pure and credential-free.
- Replace wildcard exports with explicit exports. Keep emergency role types out
  of the live composition root and strategy dependencies.
- Let tests inject role fakes without exposing arbitrary production signing.

Tests:

- `trybuild` compile-fail fixtures prove live/strategy code cannot import raw
  signing/credentials, authenticated session signing, algo/spread mutation,
  amend, or emergency/evidence constructors.
- Mock wire tests assert exact method, path, query, and websocket operation
  allowlists per role.
- A `cargo metadata`-based dependency-policy test enforces the graph above and
  keeps `reap-strategy` venue-independent.

Exit gate:

- No public/transitive bypass can turn a narrow live role into arbitrary
  authenticated endpoint access.
- `reap-live` cannot import emergency or offline-evidence authenticated
  authority.
- Existing deterministic behavior is unchanged.

Suggested commits: private wire layer, then one small commit per migrated role.

### Phase 4: Build Only Planned Live Connectivity

Actions:

- Construct public and private subscriptions from the resolved plan.
- Retain live `Trades` as the explicit `CHAOS-MD-TRADE` requirement. Record the
  current Rust no-op/implied-depth parity gap without changing it. Preserve the
  existing capture trade subscriptions and offline backtest trade matching;
  do not create capture/backtest connectivity plans.
- Resolve public replica cardinality per requirement. Every replica above one
  must name and test its sequencing/deduplication/recovery consumer.
- Pack compatible orders/account/positions/required-fill subscriptions into one
  private state socket per account only after event-permutation and
  failure-domain tests pass. Additional private sockets need an explicit tested
  ordering/isolation/capacity requirement.
- Derive account-scoped order dispatch families from configured instruments.
  Start with one active command shard per account, assign every family to
  exactly one shard, and add a shard only for a measured capacity/isolation
  requirement. Every shard must be nonempty.
- Separate `shard_count` from any future `replicas_per_shard`. Keep replicas at
  one until safe pre-write failover exists and is tested.
- Make readiness depend on every planned lane/subscription and on no unused
  lane.
- Replace `public_connections_per_subscription` with per-requirement
  `replica_count` in the resolved plan. For one migration window, accept the
  legacy field only as a maximum cap: it cannot create a replica by itself,
  validation fails if it is below a mandatory tested redundancy requirement,
  and it conflicts with any new exact override. Update production/fault
  validators to inspect the resolved requirement/consumer/replica plan instead
  of requiring a global count.
- Replace `order_websocket_sessions` with a derived plan. For backward
  compatibility, accept the legacy field for one documented migration window
  only as an upper bound on derived command shards, emit a deprecation
  diagnostic, and reject conflicting legacy/new fields. It never forces idle
  sessions to open.
- Derive maintenance relevance from required regular products/services.
  Spread-only maintenance must not halt Chaos.
- Emit the resolved, secret-free plan in validation/diagnostics so operators
  can audit why each connection exists.

Tests:

- For each account, the sample spot/swap configuration that shares `BTC-USDT`
  creates one dispatch family and one non-idle command session.
- Multiple distinct underlyings share the one account lane unless an explicit
  capacity/isolation fixture requires and justifies more.
- A credential-free command benchmark proves the planned peak rate fits the
  single lane's queue/acknowledgement budget; otherwise the smallest measured
  nonempty shard count is recorded in the plan.
- Duplicate symbols/families do not create duplicate lanes.
- A required lane disconnect revokes readiness; no unplanned lane exists to do
  so.
- Live plan includes exactly one trade requirement per configured instrument;
  existing capture subscriptions and backtest trade matching are unchanged.
- Public replica tests name the readiness/recovery consumer for every extra
  socket; private packing tests cover all permissible order/account/position/
  fill event permutations and each channel/socket failure.
- Legacy public/order connection fields have deterministic migration tests,
  deprecation diagnostics, conflict rejection, and cannot force idle sockets.
- Spread-only maintenance is irrelevant; ambiguous relevant maintenance fails
  closed.

Exit gate:

- Every opened live connection and subscription has a current plan consumer.
- The Java eight-session pool is no longer treated as a parity requirement.

Suggested commit: plan-driven live composition and config migration.

### Phase 5: Enforce Forbidden Observation And Emergency Independence

Actions:

- Add `ForbiddenOrderObserver` startup and recurring checks covering every
  pending algo query family and pending spread orders.
- Make the proof per account with a 30-second default maximum age, a hard
  60-second configured maximum, and scan start no later than half the
  maximum-age interval. Require one complete zero proof before readiness and
  block placement immediately when proof expires.
- Treat nonzero, timeout, malformed response, duplicate ID, pagination error,
  and unknown enum as failure to prove zero. Block readiness and order
  placement; trigger normal canonical regular cancellation according to
  existing safety rules.
- Ensure the observer has no cancel or submit method.
- Give regular cancel/CAA/reconciliation priority over the observer. Algo and
  spread scans use bounded per-domain timeouts and a pacing budget that cannot
  hold the regular-safety budget.
- Emit a typed fatal alert on nonzero/unverifiable state that instructs an
  operator to run the separate emergency executable. Live does not invoke it.
- Run regular, algo, and spread enumeration/cancellation as independent
  workflows with independent deadlines, pacers, and progress. Schedule regular
  mitigation before awaiting either unsupported domain; do not let seven algo
  queries or spread consume its deadline.
- Keep regular and spread Cancel All After where the emergency contract
  requires them. Never add algo/spread submit.
- Expose typed internal progress for each domain to the coordinator, telemetry,
  and deterministic tests. Preserve the serialized report's existing
  per-domain counts/zero flags, aggregate enumeration attempt/failure fields,
  and `evidence_complete`/`all_clear` semantics; do not claim new per-domain
  enumeration evidence.

Tests:

- Nonzero or unverifiable forbidden state prevents any new regular placement.
- Fake-clock tests cover the 30-second default, 60-second cap, half-age scan
  start, expiry block, and per-account isolation.
- The sentinel can enumerate but cannot cancel.
- With algo and spread transports held forever, the regular-domain request
  trace through cancellation/final-zero is identical to its standalone trace
  except timestamps and completes within its standalone deadline plus one
  regular pacing quantum.
- Spread failure does not prevent regular or algo mitigation.
- One domain timing out does not consume another domain's deadline.
- Every unverified domain makes `all_clear = false` even when another domain is
  zero.
- Emergency roles cannot submit any order.

Exit gate:

- Live Chaos has read-only forbidden-domain authority.
- Emergency mutation is absent from the live execution dependency/composition
  root.
- Each emergency domain makes monotonic progress independently.

Suggested commits: sentinel first, then emergency workflow independence.

### Tranche A Verification Gate

Update the capability inventory and all affected mapping/operations/config
documentation. Run formatting, all-target clippy, the full locked workspace
tests, the release build, the systemd unit verifier, role/dependency tests,
deterministic parity/backtest hashes, and `git diff --check`. Record exact
commands and results in the Goal A handoff.

Goal A is complete when every boundary acceptance item is structurally
enforced. The production-readiness session can then be reassessed and rebased
onto the narrower authority graph; keep it paused if Goal B will follow. Goal A
does not wait for or claim the Phase 6–9 structural cleanup.

## Tranche B: Reduce Structural Coupling

Begin this tranche only after Tranche A is green. These moves preserve the
enforced authority graph instead of moving broad authority into cleaner-looking
files.

### Phase 6: Move Pure Contracts Below Live

Actions:

- Inventory why `reap-backtest` depends on `reap-live`.
- Move only pure shared config, report, evidence, provenance, and verification
  contracts to the smallest appropriate lower-level crate. Prefer an existing
  pure crate; add a narrowly named contracts crate only if ownership cannot fit
  without a dependency cycle.
- Keep networking, credentials, runtime tasks, host inspection, and emergency
  authority out of the shared layer.
- Remove the normal dependency from `reap-backtest` to `reap-live`.
- Preserve serialized formats and public behavior.

Tests:

- Existing report round trips and semantic hashes are unchanged.
- `cargo tree` shows no normal `reap-backtest -> reap-live` edge.
- Shared contract crates contain no network or credential dependency.

Exit gate:

- Backtest/research can compile without the live runtime crate.

Suggested commits: one contract family at a time, then dependency removal.

### Phase 7: Split State By Responsibility

Actions:

- Split `reap-live/src/runtime.rs` into modules for composition, connectivity,
  dispatch, readiness/safety, reconciliation, and shutdown.
- Give each subsystem an explicit state struct and narrow inputs/outputs.
- Retain one `LiveRuntime` coordinator as the owner that orders mutations.
  Subsystems may perform IO but may not mutate strategy/risk/canonical order
  state concurrently.
- Split backtest research into pure configuration, execution, report, and
  verification responsibilities after the dependency inversion.
- Split capture only where responsibility boundaries are clear; do not chase a
  target line count.
- Keep emergency workflow code with the emergency role established in Phase 5.

Tests:

- Event-order and shutdown/cancel-before-reconcile fixtures remain unchanged.
- No new `Arc<Mutex<_>>` wraps strategy, risk, or canonical order state, and no
  concurrent mutation of that state appears.
- Coordinator tests still cover disconnect, partial fill, ambiguity, storage
  failure, and restart.

Exit gate:

- Module ownership follows the trust/capability planes.
- The single-writer architecture is still explicit in types and tests.

Suggested commits: mechanical moves first, then narrow interfaces in separate
commits.

### Phase 8: Centralize Safety Semantics And Public Surface

Actions:

- Centralize duplicated safety-event classification and fail-closed transition
  semantics used by config, runtime, and verification.
- Keep pure decision functions separate from side effects and transport.
- Replace remaining glob exports that cross the defined authority/dependency
  boundaries with explicit, role-oriented exports; do not turn this into
  workspace-wide API-style churn.
- Document crate ownership and forbidden dependency directions.
- Add dependency/visibility checks that fail when emergency or raw wire
  authority leaks back into live execution.

Exit gate:

- One named contract owns each safety classification.
- Public APIs correspond to supported roles, not source-file contents.
- No semantic fixture changes.

Suggested commit: one safety contract/export surface at a time.

## Phase 9: Global Verification And Documentation

Update the capability inventory and deviation ledger with the final
implementation. Update architecture, mapping, operations, trading readiness,
sample config, and CLI help to distinguish:

- current Chaos strategy capabilities;
- Reap safety hardening;
- offline evidence;
- account-wide emergency recovery; and
- capabilities explicitly not implemented.

Run at minimum:

```bash
cargo fmt --all -- --check
cargo clippy --workspace --all-targets --locked -- -D warnings
cargo test --workspace --locked --no-fail-fast
cargo build --release --workspace --locked
deploy/systemd/verify-units.sh target/release/reap
cargo audit --deny warnings
git diff --check
```

Also run the role visibility fixtures, endpoint/channel allowlist tests,
deterministic Java-parity fixtures, backtest semantic snapshots, config
migration tests, and a dependency check proving `reap-backtest` no longer
depends on `reap-live`.

No credentialed smoke test is part of this refactor. Existing production
evidence MUST NOT be relabeled or regenerated to bless the new structure.

## Global Acceptance Criteria

The goal is complete only when:

- the sibling Java checkout, Rust pin, fixtures, and evidence bindings agree on
  the full pinned SHA;
- normalized fixtures produce the same ordered Chaos intents and deterministic
  backtest results;
- Quote remains PostOnly/non-reduce-only with no per-order STP and a verified
  `cancel_maker` account default, Hedge remains
  IOC/non-reduce-only/`CancelMaker`, and CancelOwned rejects unproven ownership;
- every live plan item has one stable requirement ID and consumer;
- Validate has no credential/network role, Observe has no mutation/CAA role,
  and Demo has only the approved live roles;
- raw signing and arbitrary production request execution are unreachable
  outside the private wire layer;
- live execution cannot access amend, trigger, algo/spread mutation, or
  emergency authority;
- the forbidden-domain sentinel is read-only and any nonzero/unverifiable state
  blocks placement;
- live subscriptions, command lanes, readiness, and maintenance relevance are
  exactly plan-derived;
- every socket above the minimum has a tested
  redundancy/ordering/isolation/capacity consumer;
- the sample plan contains no idle command session, retains the pinned
  `CHAOS-MD-TRADE` input, and opens no unplanned subscription;
- emergency regular mitigation is independent of algo/spread failures and any
  unverified domain keeps `all_clear` false;
- `reap-backtest` has no normal dependency on `reap-live`;
- the single-writer architecture plus journal/report/evidence schemas are
  preserved; only the documented backward-compatible Phase 4 config migration
  is present;
- all repository checks pass; and
- the final diff contains no production-readiness feature, credentialed
  evidence, new strategy, venue, or order capability.

Completion means that the boundary is enforced and the design is easier to
change safely. It does not mean the repository is production ready.

## Stop Conditions

Stop the goal and report the exact conflict when:

- the sibling Java checkout is absent, unreadable, dirty, or differs from the
  pinned full SHA;
- another session creates overlapping or unexplained worktree changes;
- a removal changes ordered intents, backtest output, risk decisions, event
  timing, or shutdown ordering;
- a required capability cannot be traced to the pinned Chaos scope or an
  explicit fail-closed Reap safety invariant;
- narrowing would weaken private-state convergence, regular Cancel All After,
  forbidden-domain detection, or independent emergency recovery;
- a narrow API remains bypassable through a public signer, raw client, glob
  export, constructor, or transitive dependency;
- completion requires a journal/report/evidence schema migration, a config
  change beyond the bounded Phase 4 migration, real credentials, authenticated
  network access, target-host deployment, or production approval;
- the proposed work adds a venue, strategy, order type, generic framework,
  master/group feed, or production-readiness feature; or
- a safe mechanical split cannot preserve single-writer ownership.

Do not weaken an invariant to make the goal finish. Record the blocker and
propose a separately scoped follow-up.

## Explicit Follow-Ups, Not This Goal

- routing backtest through a different engine/risk path if it changes semantics;
- broad numeric/domain-newtype migration;
- amend support, algo/spread placement, or additional regular order profiles;
- new exchange adapters or a generic strategy/plugin framework;
- master strategy and group PnL/control-plane feeds;
- evidence schema evolution or production campaign execution; and
- declaring production readiness.

## Ready-To-Run Goals

Run Goal A first:

> Implement Goal A (Phases 0–5 and the Tranche A verification gate) from
> `docs/chaos-connectivity-refactor-plan.md`. Treat
> `docs/chaos-connectivity-boundary.md` as normative. Preserve the clean pinned
> `../imm-strategy` revision and all deterministic Chaos behavior. At each
> phase gate, record the diff/tests/hashes and continue when green. Commit each
> phase separately. Do not expand scope, use credentials, change
> journal/report/evidence schemas, or claim production readiness. Stop and
> report any listed stop condition.

After Goal A is clean and reviewed, run Goal B:

> Implement Goal B (Phases 6–9) from
> `docs/chaos-connectivity-refactor-plan.md` on top of the completed Goal A
> result. Preserve the enforced
> `docs/chaos-connectivity-boundary.md` authority graph, pinned
> `../imm-strategy` behavior, serialized evidence, and single-writer
> semantics. Record and continue through green phase gates; stop and report any
> listed stop condition. Do not add production-readiness features.
