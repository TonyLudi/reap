# Determinism And Measured Runtime Goal D Handoff

Status: Phase 0 green; Phase 1 is next. This is an evidence ledger, not a trading
authorization. Until every required Goal D phase and global gate below is
recorded green, the active execution contract remains
[determinism-readiness-goal-d-prompt.md](determinism-readiness-goal-d-prompt.md).

## Scope

Goal D is limited to:

1. deterministic risk-derived fail-closed cancellation order;
2. pinned-Java public-trade implied-depth and private 100-microsecond deferred
   reprice parity;
3. credential-free, completely initialized engine/risk decision replay;
4. exact regular-order tick/lot identity and wire fields;
5. action-producing tail-latency, allocation, queue-age, and bounded runtime
   health evidence; and
6. final documentation and global verification.

It does not authorize credentials, authenticated exchange requests,
target-host deployment, production entry, new exchange connectivity,
algo/spread mutation in normal live, emergency/live authority merging, or a
runtime concurrency redesign.

## Phase Status

| Phase | Status | Result commit |
| --- | --- | --- |
| Prompt-only execution contract | Green | `b4a3752` |
| 0. Baseline, call-path audit, and measurement contract | Green | `768ee0a` |
| 1. Deterministic fail-closed cancellation | Pending | Pending |
| 2. Pinned-Java public-trade parity | Pending | Pending |
| 3. Explicit decision/risk replay parity | Pending | Pending |
| 4. Exact regular order-to-wire numeric boundary | Pending | Pending |
| 5. Action-path performance and runtime health | Pending | Pending |
| 6. Global verification and documentation | Pending | Pending |

## Phase 0 Baseline Identity

Recorded on 2026-07-19 UTC.

| Check | Evidence | Result |
| --- | --- | --- |
| Reap implementation baseline | `git merge-base --is-ancestor 83aac2abad0beef4c7f3202d2d5b921828cdc311 HEAD` | Exit `0` |
| Reap baseline commit | `git show -s --format='%H %cI %s' 83aac2a` | `83aac2abad0beef4c7f3202d2d5b921828cdc311`, `2026-07-18T20:56:20Z`, `docs: complete Goal C handoff` |
| Goal D prompt commit | `git rev-parse b4a3752` | `b4a37522171b44d75162a3fff842384456b58062` |
| Reap branch | `git status --short --branch` before the prompt commit | Local `master`, 95 commits ahead of `origin/master`; only the reviewed Goal D prompt was untracked |
| Reap clean state | `git status --porcelain=v1` after the prompt commit | Clean before Phase 0 handoff creation |
| Java reference | `git -C ../imm-strategy rev-parse HEAD` | `b6b120c7b7c466d8431bf082f3229328c5d7b2ae` |
| Java clean state | `git -C ../imm-strategy status --short` | Empty |
| Rust Java pin | `crates/reap-core/src/types.rs` | `PINNED_JAVA_REVISION = b6b120c7b7c466d8431bf082f3229328c5d7b2ae` |
| Checked-in literal evidence binding | `examples/research-smoke.toml` | Exact same full Java SHA |
| Goal B and Goal C handoffs | File presence and baseline history | Present |
| Concurrent work | Collaboration session inventory | Goal D audit workers are read-only during Phase 0 inventory; no overlapping writer is authorized |

`../imm-strategy` is a read-only behavioral reference. No Goal D phase may
modify it.

## Host And Toolchain

| Item | Phase 0 value |
| --- | --- |
| Kernel/host | Linux `7.0.0-1004-aws`, `aarch64` |
| CPU | 2 vCPU, ARM Neoverse-N1 |
| Rust | `rustc 1.95.0 (59807616e 2026-04-14)`, LLVM 22.1.2 |
| Cargo | `cargo 1.95.0 (f2d3ce0bd 2026-03-21)` |
| Workspace build cache | `target/` 4.4 GiB: debug 2.8 GiB, tests 904 MiB, release 689 MiB |
| Free filesystem space at start | 1.3 GiB of 38 GiB, 97% used |

The existing build cache is retained because the filesystem is tight. Any
later cleanup must remove only reproducible build artifacts and must first
verify that no concurrent process is using them.

## Deterministic Anchors

| Artifact | Phase 0 SHA-256 |
| --- | --- |
| `Cargo.lock` | `d8a19fb100aeb4e542a2135d546edfb5ae24629717f5ab65e285cf9bfe483b02` |
| `fixtures/normalized/chaos_quote_hedge.jsonl` | `27f2eb4b9dba7ee600ed645ad8b7c88143e8b54531232991b492cb7595e8ccaa` |
| `fixtures/normalized/chaos_quote_hedge_later.jsonl` | `40453b8be283178b20531c84142dbaeeeca82b4723e5c13594df171c778cd8ee` |
| `fixtures/normalized/chaos_quote_hedge_intents.json` | `d95fa7f121e2e8c402c8108cf9fefb7c7d7b3dbd2b9742c58c234a521f0ee0ec` |
| `examples/iarb2-basic.toml` | `0fac5a3a35fe28cdc05118b7e22241077aa7f604a9a5436355797605b51b3b26` |
| `examples/live-okx-demo.toml` | `caea78e0a75d2586ecbd16d5b4414f9606a7064b6e1684f82fff2d132a197195` |
| Goal D prompt | `da9945ac93b8730c4557261c5f12bee618364a8149333735751fef0150203398` |

The canonical CLI backtest was run twice:

```text
cargo run --locked -q -p reap-cli -- backtest \
  --format normalized-jsonl \
  --config examples/iarb2-basic.toml \
  --data fixtures/normalized/chaos_quote_hedge.jsonl \
  --pretty
cmp target/tmp/goal-d-backtest-1.json target/tmp/goal-d-backtest-2.json
sha256sum target/tmp/goal-d-backtest-{1,2}.json
```

`cmp` exited `0`. Both outputs are exactly:

```text
38acf9f5e0c310f2ec5528974beffadf4c1a7f84d46efa8d9664ee7051e84691
```

This no-trade fixture and output hash are immutable throughout Goal D. Phase 2
must use a separate Java-bound trade fixture.

## Workspace Dependency Baseline

`cargo metadata --locked --no-deps --format-version 1` reports 23 workspace
packages. The canonical sorted direct-workspace adjacency below has SHA-256
`fe98cedfaa2653e09afd57293eb71372ea476c1997e56b5ce9f27b314f5a432b`:

```text
reap-backtest -> reap-book + reap-capture + reap-core + reap-feed + reap-live-contracts + reap-order + reap-strategy + reap-venue
reap-book -> reap-core
reap-capture -> reap-book + reap-core + reap-feed + reap-telemetry + reap-venue
reap-cli -> reap-backtest + reap-capture + reap-core + reap-emergency-core + reap-fault + reap-feed + reap-live + reap-okx-evidence-adapter + reap-strategy + reap-telemetry
reap-core -> -
reap-emergency-core -> reap-core
reap-emergency-runner -> reap-core + reap-emergency-core + reap-okx-emergency-adapter + reap-order + reap-telemetry
reap-engine -> reap-core + reap-risk + reap-strategy
reap-evidence-core -> -
reap-fault -> reap-core + reap-feed + reap-live
reap-feed -> reap-book + reap-core + reap-venue
reap-live -> reap-core + reap-engine + reap-evidence-core + reap-feed + reap-live-contracts + reap-okx-live-adapter + reap-order + reap-risk + reap-storage + reap-strategy + reap-telemetry + reap-venue
reap-live-contracts -> reap-core + reap-risk + reap-strategy + reap-venue
reap-okx-emergency-adapter -> reap-emergency-core + reap-okx-wire + reap-venue
reap-okx-evidence-adapter -> reap-evidence-core + reap-okx-wire + reap-venue
reap-okx-live-adapter -> reap-core + reap-feed + reap-okx-wire + reap-order + reap-risk + reap-strategy + reap-venue
reap-okx-wire -> -
reap-order -> reap-core + reap-risk + reap-storage + reap-strategy + reap-venue
reap-risk -> reap-core
reap-storage -> reap-core
reap-strategy -> reap-core
reap-telemetry -> reap-core
reap-venue -> reap-core
```

It was generated with:

```text
cargo metadata --locked --no-deps --format-version 1 |
  jq -r '
    .packages as $packages
    | ($packages | map(.name) | INDEX(.)) as $workspace
    | $packages[]
    | .name as $from
    | ([.dependencies[]
        | select($workspace[.name] != null)
        | .name] | unique | sort) as $deps
    | "\($from) -> \(if ($deps|length)==0
        then "-"
        else ($deps|join(" + "))
      end)"' |
  sort >target/tmp/goal-d-workspace-adjacency.txt
sha256sum target/tmp/goal-d-workspace-adjacency.txt
```

The graph is acyclic. Normal live has no dependency on either emergency
adapter. Backtest has no normal dependency on live. Only the approved live,
evidence, and emergency adapters reach raw OKX wire authority. The exact
metadata output and final dependency diff will be recorded at each phase gate.

## Public Authority Baseline

The capability-bearing public types are currently:

- `ChaosExecutionPurpose` and non-Clone `ChaosExecutionIntent` with Quote,
  Hedge, and CancelOwned variants;
- `RegularApprovalScope`, `ApprovedRegularSubmit`,
  `ReservedRegularSubmit`, and `ApprovedRegularCancel`;
- `PreparedRegularSubmit` and `PreparedRegularCancel`;
- `RegularExecutionPolicy`, `OwnedRegularOrders`, and `OkxOrderGateway`;
- role-specific live-adapter `ObserveRoles`, `DemoRoles`,
  `BoundRegularOrderGateway`, readiness/reconciliation/safety handles, and
  private-session factory; and
- one normal order-command websocket allowlist containing only `order` and
  `cancel-order`.

Constructors and bindings that mint authority remain private or crate-private.
There is no public signer, raw request builder, command transport, or
algo/spread mutation operation in normal live.

For phase-to-phase drift detection, the sorted, line-number-independent
declaration surface of `reap-order`, `reap-okx-live-adapter`, and `reap-live`
contains 355 declarations and has SHA-256
`8254498e229b551f9bb1b3d1de274d77e748904b68809636e55d13cfa57ae25d`:

```text
rg -H --no-heading --no-line-number \
  '^pub(\([^)]*\))? (struct|enum|trait|fn|type|mod|use) ' \
  crates/reap-order/src crates/reap-okx-live-adapter/src crates/reap-live/src |
  sort >target/tmp/goal-d-authority-public-surface.txt
```

This hash is a guard over declaration presence and location, not a substitute
for the capability/source-policy tests or review of fields and constructors.

## Schema Baseline

The schema constants that Goal D must preserve unless an explicitly authorized
phase says otherwise are listed below. The canonical 42-line source inventory
(41 version constants plus the pinned Java revision) has SHA-256
`496653f1bba2b859a12ede28067a44a76935789d9cae7f39b4a59f95270333de`
and was generated with:

```text
rg -n --glob '*.rs' \
  '(pub(\([^)]*\))? )?const [A-Z][A-Z0-9_]*(VERSION|REVISION)[A-Z0-9_]*(: [^=]+)? =' \
  crates |
  sort >target/tmp/goal-d-version-constants.txt
```

`AlertEvent` is the one production artifact in this inventory whose
constructor uses a literal rather than a named version constant. Its exact
`schema_version: 1,` initializer is guarded separately by:

```text
rg --no-line-number '^\s*schema_version:\s*1,' \
  crates/reap-telemetry/src/alerts.rs \
  >target/tmp/goal-d-literal-schema-guards.txt
sha256sum target/tmp/goal-d-literal-schema-guards.txt
```

The one-line guard hash is
`b7038321dbc71e3d8c3f41f6adae0fce53221cd4fc701cc76b173aefb2f48b41`.

| Contract | Constant | Version |
| --- | --- | ---: |
| Journal/storage | `CURRENT_SCHEMA_VERSION` | 7 |
| Backtest latency calibration | `LATENCY_CALIBRATION_SCHEMA_VERSION` | 4 |
| Backtest carry state | `BACKTEST_CARRY_STATE_SCHEMA_VERSION` | 1 |
| Research | `RESEARCH_SCHEMA_VERSION` | 8 |
| Research verification | `RESEARCH_VERIFICATION_FORMAT_VERSION` | 3 |
| Capture analysis | `ANALYSIS_FORMAT_VERSION` | 5 |
| Capture run report | `CAPTURE_RUN_REPORT_FORMAT_VERSION` | 5 |
| Capture verification | `CAPTURE_VERIFICATION_FORMAT_VERSION` | 3 |
| Research deployment verification | `RESEARCH_DEPLOYMENT_VERIFICATION_FORMAT_VERSION` | 2 |
| Latency-calibration verification | `LATENCY_CALIBRATION_VERIFICATION_FORMAT_VERSION` | 1 |
| Approval policy | `APPROVAL_POLICY_SCHEMA_VERSION` | 1 |
| Approval key | `APPROVAL_KEY_FORMAT_VERSION` | 1 |
| Approval-policy verification | `APPROVAL_POLICY_VERIFICATION_FORMAT_VERSION` | 1 |
| Approval request | `APPROVAL_REQUEST_FORMAT_VERSION` | 1 |
| Approval signature | `APPROVAL_SIGNATURE_FORMAT_VERSION` | 1 |
| Approval verification | `APPROVAL_VERIFICATION_FORMAT_VERSION` | 1 |
| Production evidence manifest | `PRODUCTION_EVIDENCE_MANIFEST_SCHEMA_VERSION` | 8 |
| Production evidence report | `PRODUCTION_EVIDENCE_REPORT_FORMAT_VERSION` | 9 |
| Production evidence approval subject | `PRODUCTION_EVIDENCE_APPROVAL_SUBJECT_FORMAT_VERSION` | 1 |
| Emergency cancel report | `EMERGENCY_CANCEL_REPORT_SCHEMA_VERSION` | 2 |
| Emergency cancel verification | `EMERGENCY_CANCEL_VERIFICATION_FORMAT_VERSION` | 3 |
| Fault proxy configuration | `CONFIG_SCHEMA_VERSION` | 1 |
| Fault-control protocol | `CONTROL_FORMAT_VERSION` | 1 |
| Fault-injector evidence | `INJECTOR_EVIDENCE_FORMAT_VERSION` | 1 |
| Fault run report | `RUN_REPORT_FORMAT_VERSION` | 2 |
| Fault-proxy verification | `FAULT_PROXY_RUN_VERIFICATION_FORMAT_VERSION` | 1 |
| Account cash policy | `ACCOUNT_CASH_POLICY_VERSION` | 1 |
| Account certification | `ACCOUNT_CERTIFICATION_SCHEMA_VERSION` | 3 |
| Chaos connectivity plan | `CHAOS_CONNECTIVITY_PLAN_SCHEMA_VERSION` | 1 |
| Bill collection | `BILL_COLLECTION_SCHEMA_VERSION` | 1 |
| Deadman-expiry certification | `DEADMAN_EXPIRY_CERTIFICATION_SCHEMA_VERSION` | 1 |
| Economic reconciliation | `ECONOMIC_RECONCILIATION_SCHEMA_VERSION` | 5 |
| Live fault-matrix manifest | `LIVE_FAULT_MATRIX_MANIFEST_SCHEMA_VERSION` | 3 |
| Live fault-matrix report | `LIVE_FAULT_MATRIX_REPORT_FORMAT_VERSION` | 5 |
| Fill collection | `FILL_COLLECTION_SCHEMA_VERSION` | 1 |
| Live latency evidence | `LIVE_LATENCY_EVIDENCE_SCHEMA_VERSION` | 2 |
| Live-run verification | `LIVE_RUN_VERIFICATION_FORMAT_VERSION` | 2 |
| Operator protocol | `PROTOCOL_VERSION` | 2 |
| Production transition | `PRODUCTION_TRANSITION_FORMAT_VERSION` | 3 |
| Live run report | `LIVE_RUN_REPORT_SCHEMA_VERSION` | 8 |
| Fill statement | `FILL_STATEMENT_REPORT_SCHEMA_VERSION` | 2 |
| Telemetry alert | literal `AlertEvent::new` initializer | 1 |

Phase 0 also introduces two non-production, measurement-only JSON schemas:
`reap-live/action_path` version 1 and
`reap-okx-live-adapter/prepared_serializer` version 1. They are emitted only
by a bench target and an ignored `#[cfg(test)]` release test respectively;
they are not configuration, journal, evidence, approval, or runtime schemas.

Phase 4 explicitly allows only bounded internal exact tick/lot/min-size
metadata propagation. It does not authorize changing the serialized contracts
above.

## Phase 0 Audit Records

### Fail-Closed Cancellation

`RiskGate.live_orders` is a
`HashMap<String, LiveOrderRisk>`. `live_order_ids()` returns raw key iteration,
and `live_order_ids_for(symbol)` filters that same iteration. Both engine paths
collect those iterators without sorting:

- the generic path appends `OrderIntent::CancelOrder` values after the already
  risk-processed strategy intents;
- the typed Chaos path appends non-authoritative `SafetyCancelCandidate`
  values after its already risk-processed typed strategy intents.

The `HashSet` of existing strategy cancellations is used only for membership
and is never iterated. Strategy cancellation order and reasons therefore
remain intact, and a matching risk-derived duplicate is suppressed. The
current synthetic reason is exactly `fail_closed`.

Scope semantics to preserve:

- a killed `RiskGate` cancels every observed live order even when the input is
  symbol-scoped;
- a non-killed explicit `SymbolHalted` input selects only that symbol; and
- other cancel-requiring inputs or generated system events select all live
  orders.

The typed candidate does not itself prove ownership. `LiveCoordinator` routes
strategy CancelOwned and safety candidates through the same
`RegularExecutionPolicy::authorize_cancel` path. That path requires a matching
`OwnedRegularOrders` entry, bound profile/account, canonical private state,
matching symbol, and active order status before it can create a cancel action.
`OwnedRegularOrders` is a `BTreeMap` populated only by policy-approved local
reservation or opaque durable recovery. The existing unproven-private-order
test proves foreign/prefix-looking/algo/spread observations cannot become
cancel authority.

Phase 0 added the passing test-only executable probe:

```text
TMPDIR=/home/ubuntu/code/reap/target/tmp \
  cargo test -p reap-engine --locked --lib \
  tests::goal_d_fail_closed_process_order_probe -- --exact --nocapture
```

It launches 24 independent copies of the test process. Inside each child it
constructs fresh paired generic and typed engines for forward, reverse,
interleaved, and rotated insertions of the same six live IDs, checks exact
membership and reason, and records ordered compact JSON. All 24 four-case
process projections were distinct, and insertion permutations within a
process also changed the raw map traversal. One representative child:

```json
{"cases":[{"insertion":"forward","generic":["reap-02","reap-z0","reap-2","reap-perp","reap-10","reap-a1"],"typed":["reap-a1","reap-z0","reap-perp","reap-10","reap-2","reap-02"]},{"insertion":"reverse","generic":["reap-2","reap-a1","reap-02","reap-z0","reap-10","reap-perp"],"typed":["reap-z0","reap-perp","reap-10","reap-02","reap-a1","reap-2"]},{"insertion":"interleaved","generic":["reap-2","reap-z0","reap-a1","reap-perp","reap-10","reap-02"],"typed":["reap-a1","reap-02","reap-z0","reap-perp","reap-10","reap-2"]},{"insertion":"rotated","generic":["reap-02","reap-10","reap-2","reap-z0","reap-perp","reap-a1"],"typed":["reap-2","reap-perp","reap-a1","reap-02","reap-z0","reap-10"]}]}
```

This is direct evidence of both cross-process nondeterminism and independent
generic/typed map seeds. Phase 1's global exact target is:

```text
reap-02, reap-10, reap-2, reap-a1, reap-perp, reap-z0
```

For a symbol halt that first produces existing strategy cancels, Phase 1 must
retain those strategy cancels in their current order/reason and append only
the remaining symbol IDs in bytewise lexicographic order. It must sort the
already allocated fail-closed vector only; it must not change `RiskGate` maps
or add normal-path sorting.

### Pinned-Java Public Trade

The audited sibling remained clean at
`b6b120c7b7c466d8431bf082f3229328c5d7b2ae`.

Reached call path:

1. `Iarb2ChaosEntityFactory.java:36-95` creates `OkEntity` with the configured
   `ignoreBestLevel`.
2. `ChaosStrategyBase.java:264-333` subscribes each configured trading symbol
   to depth and public market data.
3. Trade delivery at `ChaosStrategyBase.java:302-331` first mutates the entity
   through `ChaosEntity.onPublicTrade`/`OkEntity.onPublicTrade`, then calls
   `Iarb2Strategy.onPublicTrade`, then separately updates the live implied-fill
   tracker.
4. `Iarb2Strategy.java:204-215` schedules one independent 100-microsecond
   callback for each strict raw-top crossing while the strategy is Live.
5. The callback runs the normal `pricingWorker`; `Iarb2Strategy.java:427-434`
   and `581-775` recheck Live state, update pricing/hedge/theoretical state, and
   refresh quoters.
6. `Iarb2Calculator.java:417-424`, `455-508`, and `642-725` reach the entity's
   implied-level selection during hedge/theoretical calculations.

Entity mutation precedes the Iarb2 Live-state scheduling check. With
`S = reverse(taker_side)`, every trade:

```text
last_trade[S] = trade
first_valid_level[S] = empty

crossed =
  S == Sell: raw_best_ask < trade_price
  S == Buy:  raw_best_bid > trade_price

if last_trade[reverse(S)] exists and crossed:
  clear last_trade[reverse(S)]
  clear first_valid_level[reverse(S)]
```

This is `OkEntity.java:94-115`. Taker Buy therefore updates ask/Sell state;
taker Sell updates bid/Buy state. Crossing is strict. Trade IDs and timestamps
are not inspected.

| Case | Pinned Java result |
| --- | --- |
| Non-trade payload | Rejected before both public-trade callbacks |
| Unconfigured symbol | Outside the reached subscription graph; no invented fallback |
| Passive taker Buy/Sell | Overwrite mapped ask/bid last trade and clear its cache; do not clear the other side or schedule |
| Aggressive taker Buy/Sell | Same mapped-side mutation; clear prior opposite-side trade/cache if present; schedule one callback only when Live |
| Trade exactly at raw best | Not crossing; mapped-side state still changes |
| Repeated identical aggressive trade | No deduplication; overwrite state and schedule one callback per Live event |
| Several same-side trades | Latest trade remains; every qualifying event retains its callback |
| Alternating passive trades | Both side records can coexist |
| Alternating crossing trade | Current side replaces its record and clears the opposite record |
| Stale/out-of-order timestamp | Accepted without comparison |
| Trade before depth | State retained; crossing false and no schedule; next depth clears it |
| Any later depth, even older | Replace depth and clear both last trades/caches; already scheduled callbacks remain |
| Trade while not Live | Entity state changes; no callback |
| Leave Live before callback | Callback remains, but pricing rechecks state and returns |

`OkEntity.getFirstValidLevel` at `OkEntity.java:166-213`:

- returns the cached value first;
- with no last trade, caches level 1 when `ignoreBestLevel`, otherwise 0;
- with a trade, scans physical depth, skipping level 0 when configured;
- accepts a level only when the price is more passive than the trade, or raw
  `level_price == trade_price` and
  `trade_qty < level_qty / 2`, and the level is equal-or-more-passive than the
  pending opposite-side hedge price;
- treats exact half quantity as invalid; and
- returns/caches the final physical level when none passes.

The equality branch uses raw Java `==`; the more-passive and own-hedge
comparisons use the magnitude-dependent `NumberUtil` epsilon
(`NumberUtil.java:26-46`, `156-207`, `281-308`), not Rust's current broader
`approx_eq`. A fuzzy-equal but raw-unequal price satisfies neither branch.

Ask/Sell implied depth consults a pending Buy hedge and requires ask price at
or above it. Bid/Buy consults a pending Sell hedge and requires bid price at or
below it. `Iarb2Strategy.java:846-869` records the original consumed book price
after IOC send, and `ChaosEntity.java:255-262` invalidates the reverse cache.
Pending hedge expiry is examined only on depth and is strict age greater than
30 ms.

Concrete ask-side anchor:

```text
asks = 101@10, 102@10, 103@10
taker side = Buy
trade = 102@5
ignore best = false
no pending Buy hedge
```

Raw ask 101 is strictly crossed, so a Live strategy schedules. Level 101 is
invalid as more aggressive than the trade. Level 102 is invalid because 5 is
exactly half of 10. Level 103 is first valid. A quantity immediately below 5
makes level 102 valid. The mirrored bid fixture uses 99, 98, 97 with a taker
Sell at 98.

Every Live strict-cross trade creates an independent
`scheduleWithMicro(..., 100)` callback. No handle is retained or cancelled.
Callbacks read the latest shared state. `ChaosConflationWorker.java:46-103`
may conflate the resulting pricing work, but the private callbacks themselves
remain multiplicative. Live Java uses a relative executor delay
(`TimerResourceImpl.java:62-71`); its interface explicitly disclaims precise
microsecond accuracy. Java simulation collapses the delay to the current
millisecond and uses creation-order tie-breaking
(`SimTimerResource.java:134-143`, `SimTimerHandle.java:13-18`, and
`SimTimerResourceTest.java:81-109`). Goal D's arrival-time-plus-100,000-ns
private replay action is the deterministic replacement for that simulation
limitation.

No direct pinned Java `OkEntity`/Iarb2 public-trade test exists. Goal D must
therefore bind new Rust fixtures to the source call path above. The separate
Java live implied-fill tracker is not part of the strategy implied-depth state
and remains outside Phase 2.

### Decision And Risk Parity

| Layer | Live | Backtest/replay baseline | Phase 0 disposition |
| --- | --- | --- | --- |
| Normalization and scheduling | Raw storage, venue adapter, production `FeedProcessor`, biased runtime event loop | Deterministic arrival-ns scheduler; matching occurs before strategy delivery for depth/trades | Conditionally equal only for an explicitly identical ordered normalized/arrival fixture |
| Chaos strategy | Typed `on_owned_execution_event` through `TradingEngine` | Legacy `Strategy::on_event` directly | Same implementation and one-way typed-to-legacy projection; equal only for identical event/time order |
| Engine/risk | Event reduction into `RiskGate`, staleness, strategy, pre-trade checks, fail-closed synthesis | Absent; intents go directly to simulated matching | Not equal today; Goal D adds a separate exact initialized replay, not a BacktestRunner dependency |
| Downstream | Startup gate, account route, regular policy, owned proof, same-turn PendingNew, records/actions, storage-first commit, gateway/exchange | Latency, matching, portfolio, funding, and accounting simulation | Intentionally different; compare a named logical coordinator projection only |

The existing generic/typed engine differential test disables feed/private
health and relies on permissive model/limit defaults. It is useful strategy
coverage but is not full live initialization evidence.

A complete decision replay must account for every `RiskGate` field that can
affect the immediate or a later decision:

- fully expanded effective `RiskLimits`, including every health, rolling
  window, count, notional, drawdown, stablecoin, and repayment field;
- kill-switch reason and every halted-symbol reason;
- all feed-health `(Venue, Symbol)` and private-health
  `(Venue, Option<AccountId>)` keys, last-ready timestamps, and stale state;
- marks, plus an explicit instrument model and instrument order-limit row for
  every managed/executable symbol;
- positions and active live-order rows;
- ordered submit-rejection window, rejected-ID set, and last rejection time;
- ordered unfilled-IOC-cancel window, ID set, and last cancellation time;
- turnover, equity, equity by account including `None`, and peak equity;
- every seen-fill identity used for deduplication;
- stablecoin observation value/timestamp/conflict state and breach-start time,
  including explicit missing-rate state; and
- source event milliseconds plus local arrival nanoseconds.

Missing models currently default to Spot and missing instrument limits skip the
cap. The Goal D replay validator must reject either omission; it cannot call
those permissive fallbacks evidence.

Live startup initializes risk in an ordered sequence: authenticated
models/limits, durable latches, recovered active owned orders sorted by ID,
recent fills/open orders, authoritative account snapshots, reconciliation,
latch reapplication, then explicit feed/private health events. Replay must
drive production transitions in a fixed order instead of directly assigning
copied risk logic.

Planned checked-in fixture family:

```text
fixtures/decision_parity/risk_initialization_v1.json
fixtures/decision_parity/replay_events_v1.jsonl
fixtures/decision_parity/expected_engine_v1.jsonl
fixtures/decision_parity/expected_live_reduction_v1.jsonl
```

The initialization artifact will bind schema 1, Reap/Java/config hashes,
fully expanded risk inputs, `seed_now_ms`, and ordered seed events. Replay
envelopes use `{sequence, arrival_ns, event}`. Expected engine rows use
structured typed purpose, legacy projection, rejection, system-event, and
safety-candidate fields rather than Debug text. The live projection is taken
after coordinator reduction but before dispatch/commit; it must retain
same-turn PendingNew behavior.

Generated client IDs contain process-time/PID state. The expected logical live
projection may alpha-rename them deterministically (`client#1`, and so on)
without weakening or replacing the real generator. The comparison must also
fully initialize StartupGate, gateway-action accounts, order-entry flag,
account halts, canonical private orders, owned proofs, and VerifiedBootstrap
through production seams.

### Exact Numeric Authority

| Stage | Current representation and decision |
| --- | --- |
| OKX wire parse | `OkxInstrumentWire` receives `tickSz`, `lotSz`, and `minSz` as exact strings, then `TryFrom` immediately parses and retains only `f64` in public `OkxInstrument` |
| Live bootstrap | Compares configured/exchange floats with scaled tolerances, checks min/lot alignment, and copies floats into serialized `VerifiedInstrument` |
| Drift check | Compares tick/lot/min floats directly |
| Policy composition | The sole live seam copies those floats into `RegularExecutionProfile`; source guards pin construction to that seam |
| Approval | Quote/Hedge floats pass finite/positive, minimum, existing ULP alignment, quantity, and notional checks; `ApprovedRegularSubmit` retains only `NewOrder` floats |
| Reservation | Same-turn `reserve_local` moves the same floats into `ReservedRegularSubmit` and canonical PendingNew |
| Idempotency | `OrderFingerprint` hashes `qty.to_bits()` and `price.to_bits()` before prepared authority |
| Prepared command | `PreparedRegularSubmit` still carries only the float `NewOrder` |
| Adapter | Maps to `OkxPlaceOrder { price: f64, qty: f64 }`; inner place JSON and websocket args derive `px`/`sz` with direct `to_string()` |

Source anchors are:

- `reap-venue/src/okx/rest.rs:1271-1352` for the exact-string loss;
- `reap-live/src/bootstrap.rs:203-325` and `434-440` for bootstrap comparison;
- `reap-live/src/regular_execution.rs:11-74` for the sole live policy seam;
- `reap-order/src/authority.rs:602-742` and `905-911` for policy and the
  existing `8 * EPSILON * max(abs(units), 1)` alignment predicate;
- `reap-order/src/client_id.rs:155-253` for raw-bit fingerprinting;
- `reap-order/src/gateway.rs:353-405` for idempotency before Prepared; and
- `reap-okx-live-adapter/src/lib.rs:1136-1172` and `1256-1306` for final
  lowering.

There is no normal REST place-order transport. The REST execution allowlist
contains cancel only, `/api/v5/trade/order` is forbidden, and websocket
operations remain exactly `order` and `cancel-order`. Goal D's “REST/websocket
field equality” is therefore interpreted as equality between the private
REST-shaped inner-body serializer and the websocket `args[0]` `px`/`sz`
values. It must not add REST placement authority.

Phase 4's bounded plan is:

1. preserve normalized positive exact decimal metadata as checked
   coefficient-plus-scale values while retaining the existing float companions
   for model arithmetic and acceptance;
2. propagate only internal exact tick/lot/min data through
   venue/bootstrap/policy while keeping serialized `VerifiedInstrument`,
   config, NewOrder, journal, report, and evidence schemas frozen;
3. after the existing float checks pass unchanged, derive checked nearest
   integral tick/lot counts and reject non-finite, non-positive, unaligned,
   overflowed, underflowed, or unrepresentable values;
4. carry a private, non-Clone canonical numeric payload through
   Approved -> Reserved -> Prepared;
5. replace raw-bit numeric fingerprint identity with canonical unit/decimal
   identity; and
6. have one adapter-private formatter feed both inner body and websocket args,
   with a source guard rejecting raw float `px`/`sz` lowering.

Tests must include exact metadata in ordinary, trailing-zero, scientific,
small, and large forms; min-in-lot consistency; current authority
acceptance/rejection; bit-distinct accepted floats with one wire value;
wire-distinct conflict; exact inner-body/websocket field equality; unchanged
allowlists; and compile-fail proof that numeric authority remains opaque.

## Performance Baseline

The completed Goal C handoff is the authoritative pre-Goal-D regression
baseline:

| Benchmark | Goal C median | Logical counters |
| --- | ---: | --- |
| Engine event loop | 11,058.7 ns/event | 250,000 events; 999,996 intents |
| Complete live parity | 17,082.6 ns/raw | 50,204 raw; 70,208 feed outputs; 65,130 records; 0 actions |

The Goal C live path used 4,193,771 allocation calls and requested
1,871,951,969 bytes.

An initial Phase 0 engine warm-up measured 11,356.5 ns/event. Three runs made
while read-only audit workers were active measured 11,242.3, 12,233.8, and
12,857.1 ns/event, each with exactly 999,996 intents. They are retained as
diagnostic host-noise evidence but are not the required otherwise-idle Phase 0
record.

The required otherwise-idle Phase 0 engine run used:

```text
TMPDIR=/home/ubuntu/code/reap/target/tmp \
  cargo bench -p reap-engine --bench event_loop --locked
```

The warm-up measured 11,848.4 ns/event. The three recorded runs measured
11,301.1, 11,314.5, and 11,464.4 ns/event; their median was
11,314.5 ns/event. Every run processed 250,000 events and produced exactly
999,996 intents. The median is 2.31% above the Goal C 11,058.7 ns/event
baseline, below the 5% investigation threshold.

The required otherwise-idle Phase 0 live run used:

```text
TMPDIR=/home/ubuntu/code/reap/target/tmp \
  cargo bench -p reap-live --bench live_loop --locked
```

The warm-up complete-live result was 17,831.7 ns/raw. The three recorded
complete-live results were 17,880.3, 19,885.0, and 17,619.1 ns/raw; their
median was 17,880.3 ns/raw. Every run processed 50,204 raw frames, produced
70,208 feed outputs and 65,130 storage records, produced zero actions, and
performed exactly 4,193,771 allocation calls requesting 1,871,951,969 bytes.
The median is 4.67% above the Goal C 17,082.6 ns/raw baseline, below the 5%
investigation threshold. The 19,885.0 ns/raw observation is retained as host
variance rather than discarded or retried.

### Action-Path Measurement Contract

Phase 0 adds the stable command:

```text
TMPDIR=/home/ubuntu/code/reap/target/tmp CARGO_BUILD_JOBS=1 \
  cargo bench -p reap-live --bench action_path --locked
```

The thin six-line entry point delegates to seven cohesive bench-only modules;
the largest is 373 lines. Each workload uses 10,000 warm-up observations and
100,000 post-warm-up observations. Latency and allocation are intentionally
separate, freshly initialized passes. The timing pass runs with allocation
tracking disabled; the allocation pass runs the same inputs with tracking
enabled and asserts that its complete logical counters equal the timing pass.
This prevents the tracking allocator's atomic increments from contaminating
the latency distributions.

All elapsed and queue-age samples use process-local monotonic
`std::time::Instant`. The harness retains one `u64` nanosecond sample per
observation and computes exact nearest-rank p50, p95, p99, p99.9, and max over
all 100,000 values. There is no histogram, interpolation, reservoir,
downsampling, dropped sample, or overflowed sample. Its schema-version-1
`ACTION_PATH_JSON` record contains the complete counters, allocations, and
per-workload boundaries.

The otherwise-idle baseline used one warm-up followed by five recorded runs:

```text
target/tmp/goal-d-action-warmup-final.txt
target/tmp/goal-d-action-run-{1,2,3,4,5}.txt
```

Each tuple below is `p50/p95/p99/p99.9/max` nanoseconds; tuples are recorded
runs 1 through 5:

| Workload | Five recorded distributions (ns) |
| --- | --- |
| Quote creation through Prepared submit | `18855/23770/30605/89541/81641996`; `20069/26576/38621/171238/77180032`; `18683/23343/30014/72564/80882775`; `19191/25287/32065/105762/83687066`; `19175/23212/29193/78866/80826210` |
| Quote replacement/owned cancel through Prepared cancel | `12980/13505/18248/24008/5849685`; `14079/14572/19561/26338/1074579`; `12849/13259/18034/22933/140978`; `12963/13629/18273/23212/86964`; `13268/13710/18510/22719/90459` |
| IOC hedge through Prepared submit | `23720/26625/32376/42986/47131420`; `25206/28258/35773/77660/52287701`; `23712/26576/34715/107863/48203526`; `23942/29185/37677/165560/48848535`; `24205/26232/32188/48590/47774718` |
| Risk rejection | `11709/12127/16541/20480/80851`; `12726/13653/18313/38177/2768411`; `11815/12242/16590/20980/91403`; `11865/12324/16787/21874/88868`; `12062/12701/17722/24246/3072733` |
| Symbol fail-close through Prepared cancel | `632/649/665/1337/83879`; `738/747/763/1797/56549`; `640/648/665/1330/30908`; `632/640/657/1362/18297`; `657/665/681/1001/41977` |
| Global fail-close through Prepared cancels | `763/821/862/1698/29596`; `862/878/895/4070/21735`; `755/788/813/1559/35035`; `755/787/804/3233/407286`; `780/804/821/2289/310148` |
| Coordinator normalized/storage reduction | `5620/5916/9370/15516/130819`; `5859/6121/9255/13718/39482`; `5563/5752/7064/12750/77118`; `5596/5867/8845/13301/378897`; `5653/5875/8689/13603/879973` |
| Raw gap through recovery action/storage record | `26765/30178/37661/54169/6397991`; `27093/29332/36528/46605/6184333`; `26141/28980/36864/61636/6140485`; `26724/30432/37522/53644/6209907`; `26666/32188/40910/104342/6061562` |
| Public-trade zero-reprice baseline | `148/164/165/181/12693`; `156/165/180/189/60831`; `148/164/172/181/12504`; `148/164/172/181/47350`; `148/156/172/181/9288` |
| Bounded biased control/feed storm | `140/197/7598/7655/17690`; `148/205/7803/7877/18526`; `140/189/7614/7680/13990`; `148/213/7655/7696/13325`; `140/189/7615/7688/12496` |

The five timer-read-overhead tuples were
`33/41/41/42/12333`, `41/49/50/50/14023`,
`33/41/41/42/11700`, `33/41/41/2109/12431`, and
`33/41/41/42/10125` ns. The fourth run's timer p99.9 is retained as observed.
The storm queue-age tuples were
`11971/16262/16992/20718/25715`,
`12226/16673/17452/21719/28209`,
`11922/16155/16746/21448/30686`,
`12111/16525/17156/21826/39048`, and
`11930/16123/16656/21004/27593` ns.

The complete distributions are retained without retrying away host noise.
Several maximums and some upper percentiles show scheduler/preemption
outliers on this shared two-vCPU host (most visibly quote creation, IOC hedge,
and the second recorded run). They are local regression evidence, not a
target-host latency claim or SLO.

Logical and allocation counts were exact across all five runs. The canonical
JSON projection of every workload's name, counters, and allocation totals had
SHA-256
`19c1740a4e9113e12c0bc1215cbd62d3dd78c1a53d37f334e35a2fc6f42455ab`
in each run.

| Workload | Key logical counts per run | Total calls / bytes | Calls/bytes per input | Calls/bytes per produced action |
| --- | --- | ---: | ---: | ---: |
| Quote creation | 100,000 inputs; 400,000 typed quotes/actions; 100,000 Prepared submits | 19,333,342 / 821,130,376 | 193.33342 / 8,211.30376 | 48.333355 / 2,052.82594 |
| Quote replacement | 100,000 inputs; 400,000 quotes + 100,000 CancelOwned actions; 100,000 Prepared cancels | 16,600,000 / 773,500,000 | 166 / 7,735 | 33.2 / 1,547 |
| IOC hedge | 100,000 inputs; 400,000 quotes + 100,000 hedge actions; 100,000 Prepared submits | 22,933,342 / 1,250,030,376 | 229.33342 / 12,500.30376 | 45.866684 / 2,500.060752 |
| Risk rejection | 100,000 inputs; 400,000 rejections; zero actions | 15,300,000 / 622,400,000 | 153 / 6,224 | N/A |
| Symbol fail-close | 100,000 inputs/candidates/actions/Prepared cancels | 1,300,000 / 46,000,000 | 13 / 460 | 13 / 460 |
| Global fail-close | 100,000 inputs; 200,000 candidates/actions/Prepared cancels | 1,600,000 / 52,600,000 | 16 / 526 | 8 / 263 |
| Coordinator reduction | 100,000 normalized inputs/storage records; zero actions | 3,900,000 / 158,800,000 | 39 / 1,588 | N/A |
| Raw gap recovery | 200,000 frames/parsed events; 600,000 feed outputs; 500,000 normalized outputs; 100,000 RecoverBook actions; 900,003 records | 21,980,027 / 1,769,312,151 | 219.80027 / 17,693.12151 | 219.80027 / 17,693.12151 |
| Public trade | 100,000 normalized inputs; zero immediate intents and zero due actions | 100,000 / 800,000 | 1 / 8 | N/A |
| Queue storm | 20,000 control + 80,000 feed dequeues; 20,000 preemptions; capacity/high-water 80; 30,000 saturations | 0 / 0 | 0 / 0 | N/A |

The engine action workloads include production `ChaosStrategy`,
`TradingEngine<ChaosStrategy>`, `RiskGate`, typed intent traversal, regular
policy approval, canonical reservation/ownership, gateway idempotency, and
lowering through `PreparedRegularSubmit`/`PreparedRegularCancel` as applicable.
The coordinator rows use the production coordinator reduction and storage
projection. The raw row additionally includes two credential-free OKX book
frames per observation, actual `OkxAdapter` parsing, `FeedProcessor`
deduplication/sequence/recovery/book reduction, and
`LiveCoordinator::process_feed`. It excludes socket receive, production
channel scheduling, storage enqueue/disk, command serialization, network IO,
and exchange acknowledgement. The storm is explicitly bench-private and does
not claim to measure the production select loop.

The quote, replacement, and hedge rows deliberately retain all
same-session reservations and gateway idempotency entries across their 110,000
observations. They therefore measure a progressively aged session, not a
bounded steady-state working set. Goal D uses this exact source state as its
regression baseline and does not infer target-host steady-state latency from
it.

The public-trade row is deliberately the missing semantic case rather than a
same-best-price no-op: BTC-USDT asks are 50,001/50,002/50,003 with quantity 10,
then a taker Buy arrives at 50,002 with quantity 5. It strictly crosses the raw
best and, under the pinned Java exact-half rule, moves the first-valid ask from
50,001 to 50,003. Each observation declares a process-monotonic replay arrival
and the exact `arrival_ns + 100,000` deadline. Phase 0 delivers the trade
through the production engine and records zero immediate intents and zero due
actions because private due scheduling/service does not yet exist. Phase 2
must extend this same row to service the private due action and assert a
nonzero reprice result.

The adapter-owned serializers remain excluded from the action-path binary
rather than being exposed. They are measured by this private ignored release
test:

```text
TMPDIR=/home/ubuntu/code/reap/target/tmp CARGO_BUILD_JOBS=1 \
  cargo test --release -p reap-okx-live-adapter --locked \
  tests::goal_d_prepared_serializer_benchmark -- \
  --ignored --exact --nocapture
```

Its untimed fixture setup traverses the existing typed
strategy -> policy -> reservation -> gateway seam once. Each measured
observation starts from an already-created `PreparedRegularSubmit`, calls the
adapter-private `regular_place_order`, then calls either the actual
REST-shaped inner-body serializer or the actual websocket order-request
serializer. Credentials, signing, transport queues, network IO, exchange
acknowledgement, and every cancel/algo/spread operation are excluded. No
serializer or authority constructor became public.

One warm-up and five recorded runs are retained in
`target/tmp/goal-d-serializer-{warmup,run-1,run-2,run-3,run-4,run-5}.txt`.
Each workload has 10,000 warm-up and 100,000 timed observations, exact
all-sample nearest-rank percentiles, zero dropped/overflowed samples, and a
separate allocation pass. Each tuple is
`p50/p95/p99/p99.9/max` nanoseconds:

| Serializer workload | Five recorded distributions (ns) | Calls/bytes per input and action |
| --- | --- | ---: |
| Prepared -> REST-shaped inner body | `714/731/747/1493/388693`; `690/706/714/1239/188304`; `697/714/2823/5136/396250`; `714/730/739/1329/431023`; `722/739/755/3094/176390` | 6 / 436 |
| Prepared -> websocket order request | `2207/2231/2248/6736/270657`; `2240/2257/2281/6868/441033`; `2240/2306/5210/9025/204279`; `2207/2224/2248/7056/463244`; `2224/2248/2272/6983/128621` | 24 / 1,557 |

The serializer timer-overhead tuples were
`33/33/41/42/20119`, `33/33/41/42/18207`,
`33/33/41/42/16992`, `33/33/41/42/4570`, and
`33/33/41/42/16894` ns. The exact logical/allocation projection had SHA-256
`1a0bf7d15fcbd62373aab0586f2d296e014c033fa6af6fef702cfca19495059b`
in every recorded run: 100,000 prepared inputs/actions, exactly 100,000
REST-shaped bodies or websocket requests as applicable, 600,000 allocation
calls/43,600,000 bytes for REST-shaped bodies, and 2,400,000
calls/155,700,000 bytes for websocket requests. A fixture assertion also
proves byte-identical `px` and `sz` fields between both serializers.

## Authorized Output-Change Ledger

No Goal D production behavior has changed.

| Phase | Authorized change | Before evidence | After evidence |
| --- | --- | --- | --- |
| 1 | Stable risk-derived cancellation order only | Pending | Pending |
| 2 | Pinned public-trade implied depth and private 100-microsecond reprice only | Pending | Pending |
| 3 | No default behavior change; new credential-free decision trace | Pending | Pending |
| 4 | Canonical exact numeric identity and `px`/`sz` bytes only | Pending | Pending |
| 5 | No decisions; bounded metrics/health snapshots only | Pending | Pending |

## Phase Gate Ledger

### Phase 0

Phase 0 changes only documentation, `#[cfg(test)]`/ignored test support, and a
bench-only target. Review of the complete diff found no production control
flow, authority, connectivity, schema, or serialized-output change.
The gated phase implementation/evidence commit is `768ee0a`.

Green commands and results:

```text
cargo fmt --all -- --check
TMPDIR=/home/ubuntu/code/reap/target/tmp CARGO_BUILD_JOBS=1 \
  cargo clippy -p reap-engine -p reap-live -p reap-okx-live-adapter \
  --all-targets --locked -- -D warnings
TMPDIR=/home/ubuntu/code/reap/target/tmp CARGO_BUILD_JOBS=1 \
  cargo test -p reap-engine --locked
TMPDIR=/home/ubuntu/code/reap/target/tmp CARGO_BUILD_JOBS=1 \
  cargo test -p reap-okx-live-adapter --locked
TMPDIR=/home/ubuntu/code/reap/target/tmp CARGO_BUILD_JOBS=1 \
  cargo test -p reap-live --locked
TMPDIR=/home/ubuntu/code/reap/target/tmp CARGO_BUILD_JOBS=1 \
  cargo test -p reap-live --test dependency_policy --locked
cargo metadata --locked --format-version 1 >/dev/null
git diff --check
```

Results: engine 6/6; live adapter 37 passed, one intentionally ignored
release benchmark, six compile-fail fixtures passed; live 237/237, two
compile-fail fixtures passed, four dependency/source-policy tests passed; all
clippy and formatting checks passed with warnings denied. The exact ignored
serializer benchmark and the action benchmark's warm-up plus five recorded
runs are documented above.

Immediately before Phase 1, the canonical CLI backtest was run twice and
`cmp` exited zero. Both SHA-256 values remained
`38acf9f5e0c310f2ec5528974beffadf4c1a7f84d46efa8d9664ee7051e84691`.
`Cargo.lock` remained
`d8a19fb100aeb4e542a2135d546edfb5ae24629717f5ab65e285cf9bfe483b02`.
The Java sibling remained clean at
`b6b120c7b7c466d8431bf082f3229328c5d7b2ae`.

The complete direct workspace adjacency hash is
`fe98cedfaa2653e09afd57293eb71372ea476c1997e56b5ce9f27b314f5a432b`;
the complete version-constant inventory hash is
`496653f1bba2b859a12ede28067a44a76935789d9cae7f39b4a59f95270333de`;
the literal telemetry-alert guard hash is
`b7038321dbc71e3d8c3f41f6adae0fce53221cd4fc701cc76b173aefb2f48b41`;
and the authority declaration baseline hash is
`8254498e229b551f9bb1b3d1de274d77e748904b68809636e55d13cfa57ae25d`.

No stop condition was reached. The only observed Phase 0 semantic gap is the
one intentionally demonstrated by the public-trade zero-action row, which
Phase 2 is authorized to close.

## Remaining Operational Blockers

Goal D cannot clear:

- credentialed OKX demo observation/trading soak;
- target-host latency, queue, paging, supervision, deadman, and process-death
  evidence;
- target-account instrument/fee/position/cash/economic evidence;
- production approval or production order entry; or
- any future pinned-thread/SPSC/kernel-bypass decision.
