# Determinism And Measured Runtime Goal D Handoff

Status: Phases 0 through 5 green; Phase 6 global verification and documentation
is next. This is an evidence ledger, not a trading authorization. Until every
required Goal D phase and global gate below is recorded green, the active
execution contract remains
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
| 1. Deterministic fail-closed cancellation | Green | `4c49cae` |
| 2. Pinned-Java public-trade parity | Green | `6d446da` |
| 3. Explicit decision/risk replay parity | Green | `e167c63` |
| 4. Exact regular order-to-wire numeric boundary | Green | `85af455` |
| 5. Action-path performance and runtime health | Green | `80a38fe` |
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

Production fallback behavior still treats a missing model as Spot and skips a
missing instrument cap. The Phase 3 replay validator rejects either omission;
it never calls those permissive fallbacks evidence.

Live startup initializes risk in an ordered sequence: authenticated
models/limits, durable latches, recovered active owned orders sorted by ID,
recent fills/open orders, authoritative account snapshots, reconciliation,
latch reapplication, then explicit feed/private health events. Replay must
drive production transitions in a fixed order instead of directly assigning
copied risk logic.

Completed checked-in fixture family:

```text
fixtures/decision_parity/risk_initialization_v1.json
fixtures/decision_parity/replay_events_v1.jsonl
fixtures/decision_parity/expected_engine_v1.jsonl
fixtures/decision_parity/expected_live_reduction_v1.json
```

The initialization artifact binds schema 1, Reap/Java revisions, the complete
embedded effective configuration, fully expanded risk inputs, source clocks,
and ordered seed events. The whole-artifact SHA is the exact configuration
binding; embedding and fixed-point validation are stronger than the tentative
Phase 0 plan for a bare config digest. Replay envelopes use canonical
contiguous case blocks with exact sequences and typed normalized,
PendingFeedback, or due-reprice inputs. Expected engine rows use structured
typed purpose, legacy projection, rejection, system-event, and
safety-candidate fields rather than Debug text. The live projection is taken
after coordinator reduction but before dispatch/commit and retains genuine
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
and the exact `arrival_ns + 100,000` deadline. At the Phase 0 baseline, the
production engine recorded zero immediate intents and zero due actions because
private due scheduling/service did not yet exist. Phase 2 subsequently
extended this same row to service the private due action and assert a nonzero
reprice result.

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

Only the narrowly authorized Phase 1, Phase 2, and Phase 4 trading behaviors
have changed. Phase 5 adds observation-only progress and structured health
output without changing decisions or authority.

| Phase | Authorized change | Before evidence | After evidence |
| --- | --- | --- | --- |
| 1 | Stable risk-derived cancellation order only | Four insertion orders across 24 child processes produced distinct generic/typed traversal bytes | Every child and insertion order produces `reap-02,reap-10,reap-2,reap-a1,reap-perp,reap-z0`; strategy cancels remain first |
| 2 | Pinned public-trade implied depth and private 100-microsecond reprice only | Phase 0 public-trade workload produced zero trade-reprice actions and Rust retained no reached implied-depth trade state | Java-bound fixtures now produce exact implied-depth state, one private callback per qualifying trade, shared five-millisecond worker behavior, and identical ordered live/replay projections |
| 3 | No default behavior change; new credential-free decision trace | Backtest has no production risk/live dependency and the live decision boundary was only documented | A strict shared test harness now proves identical production strategy/risk batches and separately goldens the genuine coordinator reduction; all production tracing is `#[cfg(test)]` |
| 4 | Canonical exact numeric identity and `px`/`sz` bytes only | Exact exchange decimals were discarded after `f64` parsing; accepted order identity used raw `f64` bits; normal submit lowering derived `px`/`sz` with direct `f64::to_string()` | Exact tick/lot/min metadata now survives privately through bootstrap and policy; accepted adjacent `f64` values with one canonical wire value reuse idempotency identity, wire-distinct values conflict, and the REST-shaped body and websocket `args[0]` use identical canonical plain-decimal bytes |
| 5 | No decisions; bounded metrics/health snapshots only | No versioned runtime snapshot; storage writer and live queue/connectivity/order progress were not available as one bounded record | Private schema-version-1 snapshots are emitted through the existing structured log every fixed five seconds and once during finalization; predeclared numeric/enum progress is observation-only and does not allocate or lock per event |

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

### Phase 1

The gated phase implementation/evidence commit is `4c49cae`.

The focused regression was made red before the production change:

```text
TMPDIR=/home/ubuntu/code/reap/target/tmp \
  cargo test -p reap-engine --locked --lib \
  tests::goal_d_fail_closed_process_order_probe -- --exact --nocapture
```

The strengthened child required the exact bytewise target but observed, for
example:

```text
left:  ["reap-10", "reap-2", "reap-02", "reap-a1", "reap-z0", "reap-perp"]
right: ["reap-02", "reap-10", "reap-2", "reap-a1", "reap-perp", "reap-z0"]
```

The implementation adds one private helper in `reap-engine`. It retains only
risk-live IDs not already emitted by the strategy, sorts that remaining
borrowed slice with `left.as_bytes().cmp(right.as_bytes())`, and deduplicates
adjacent IDs in place. Both generic and typed Chaos fail-closed branches call
the helper only inside their existing `if fail_closed` block. It does not
replace `RiskGate`'s maps, sort any ordinary path, allocate a sort key, change
risk scope selection, or reorder strategy intents.

The now-green 24-child exact golden is identical for forward, reverse,
interleaved, and rotated registration:

```text
generic = typed =
["reap-02","reap-10","reap-2","reap-a1","reap-perp","reap-z0"]
```

A second exact paired generic/typed test registers strategy quotes and
risk-only observations in a deliberately mixed order. On
`SymbolHalted(BTC-USDT)`, the legacy projection is exactly:

```text
strategy-z/quote_disabled
strategy-a/quote_disabled
synth-02/fail_closed
synth-10/fail_closed
synth-2/fail_closed
```

The first two remain ordered strategy `CancelOwned` intents. Only the final
three become typed safety-cancel candidates; the duplicate risk observations
for the first two are suppressed, and the BTC-PERP observation is excluded.
Global kill continues to select all symbols.

The coordinator authority boundary remains unchanged. The exact live test
`unproven_private_orders_are_observed_but_never_become_cancel_authority`
passes after engine ordering: a candidate still has to traverse
`route_safety_cancel -> route_cancel_owned ->
RegularExecutionPolicy::authorize_cancel`, where canonical owned-regular proof
is required. `RiskGate` intentionally continues to allow cancellations, as
proved by `stale_private_stream_blocks_new_orders_but_not_cancels`. Account,
readiness, reconciliation, and shutdown cancellation paths are not
risk-derived and were not changed.

Green focused/full gates:

```text
cargo test -p reap-engine --locked
cargo test -p reap-risk --locked
cargo test -p reap-live --locked --no-fail-fast
cargo clippy -p reap-engine -p reap-live --all-targets --locked -- -D warnings
cargo fmt --all -- --check
git diff --check
```

Results: engine 7/7, risk 29/29, live 237/237, both live compile-fail
authority fixtures, four dependency/source-policy tests, and all doc tests
passed. Clippy passed with warnings denied.

The required same-host benchmark gate used one warm-up plus three recorded
runs for the existing engine and live benchmarks:

| Benchmark | Warm-up | Recorded runs | Median | Phase 0 median | Delta |
| --- | ---: | --- | ---: | ---: | ---: |
| Engine event loop (ns/event) | 11,225.4 | 11,945.5; 11,218.0; 11,272.2 | 11,272.2 | 11,314.5 | -0.37% |
| Complete live parity (ns/raw) | 17,059.5 | 17,008.4; 17,421.0; 17,318.2 | 17,318.2 | 17,880.3 | -3.14% |

Every engine run retained 250,000 events and 999,996 intents. Every live run
retained 50,204 raw frames, 70,208 feed outputs, 65,130 records, zero actions,
4,193,771 allocation calls, and 1,871,951,969 requested bytes.

The action benchmark also used one warm-up and three required recorded runs.
Two initial p99.9 comparisons crossed the investigation threshold, so two
additional complete recorded runs were retained. The table compares the
five-run median Phase 1 distributions with the five-run Phase 0 baseline:

| Workload | p50 Phase 0 -> 1 | p99 Phase 0 -> 1 | p99.9 Phase 0 -> 1 |
| --- | ---: | ---: | ---: |
| Queue storm | 140 -> 140 (0.00%) | 7,615 -> 7,581 (-0.45%) | 7,688 -> 7,639 (-0.64%) |
| Coordinator reduction | 5,620 -> 5,563 (-1.01%) | 8,845 -> 7,893 (-10.76%) | 13,603 -> 11,839 (-12.97%) |
| Global fail-close | 763 -> 779 (+2.10%) | 821 -> 845 (+2.92%) | 2,289 -> 1,846 (-19.35%) |
| IOC hedge | 23,942 -> 23,672 (-1.13%) | 34,715 -> 32,270 (-7.04%) | 77,660 -> 42,961 (-44.68%) |
| Public trade | 148 -> 148 (0.00%) | 172 -> 164 (-4.65%) | 181 -> 181 (0.00%) |
| Quote creation | 19,175 -> 18,806 (-1.92%) | 30,605 -> 30,063 (-1.77%) | 89,541 -> 86,776 (-3.09%) |
| Quote replacement | 12,980 -> 12,882 (-0.76%) | 18,273 -> 17,903 (-2.02%) | 23,212 -> 22,293 (-3.96%) |
| Raw recovery action | 26,724 -> 26,461 (-0.98%) | 37,522 -> 35,658 (-4.97%) | 54,169 -> 50,986 (-5.88%) |
| Risk rejection | 11,865 -> 11,913 (+0.40%) | 16,787 -> 16,853 (+0.39%) | 21,874 -> 24,475 (+11.89%) |
| Symbol fail-close | 640 -> 640 (0.00%) | 665 -> 665 (0.00%) | 1,337 -> 1,075 (-19.60%) |

Risk rejection's p99.9 moved by 2,601 ns while its p50 and p99 remained
within 0.4%. That workload never enters the new fail-closed branch; its exact
logical/allocation projection is unchanged. Its five individual p99.9 values
were 19,463, 24,475, 34,050, 19,396, and 36,618 ns, showing the shared-host
scheduler tail rather than a reachable sort-path cost. No optimization or
retry-based sample removal was performed.

All five Phase 1 action runs retained the exact Phase 0 logical/allocation
projection hash
`19c1740a4e9113e12c0bc1215cbd62d3dd78c1a53d37f334e35a2fc6f42455ab`.
In particular, ordinary non-action rows and both fail-closed rows retained
their exact allocation totals. The workspace adjacency, version inventory,
authority declaration, `Cargo.lock`, and canonical no-trade backtest anchors
remain unchanged. No stop condition was reached.

### Phase 2

The gated Phase 2 implementation/evidence commit is `6d446da`.

The regression was established before the production path existed. The
Phase 0 `public_trade_implied_depth_reprice` workload consumed 100,000 public
trades but recorded zero trade-reprice actions, zero typed intents, and the
exact Phase 0/1 logical projection. The new focused tests then required the
checked-in Java truth tables, a private receipt-time-plus-100,000-nanosecond
callback, causal live/backtest service, and exact typed/legacy output; those
requirements could not pass against the Phase 1 parent, which ignored trades
for strategy repricing.

#### Pinned Java evidence and implemented boundary

Every Phase 2 Java fixture binds the clean sibling checkout at
`b6b120c7b7c466d8431bf082f3229328c5d7b2ae`. The reached call path remains:

```text
ChaosStrategyBase.onPublicTrade
  -> OkEntity.onPublicTrade / isDepthUpdatedOnTrade
  -> Iarb2Strategy.onPublicTrade
  -> TimerResource.scheduleWithMicro(() -> pricingWorker.work(), 100L)
  -> ChaosTimedConflationWorker.work
  -> ChaosConflationWorker.runWork
  -> Iarb2Strategy.updatePricingAndQuote
  -> Iarb2Strategy.updatePricing(false)
```

The fixtures also bind the reached `OkEntity`/`ExchEntityBase` implied-depth
selection, `ChaosEntity.updateOurHedge`, `NumberUtil` fuzzy comparisons,
`FakeRandomProviderImpl`, and `ChaosMassQuoter` RNG consumption. Their exact
hashes are:

| Fixture | SHA-256 |
| --- | --- |
| `fixtures/java/chaos_trade_implied_depth_v2.json` | `e878270db08f223e3a70da58030953724d42e4a84bfb87ed5bd80397e17b858e` |
| `fixtures/java/chaos_pricing_worker_clock_v1.json` | `af97ac1c237b433af5762d9b286006696c1fe0bc03dc96a28a92171e0b78344b` |
| `fixtures/java/chaos_trade_rng_interleaving_v1.json` | `76cfa935b47835add52ccbfb668d198195e10cb92802e1c45fd9d807004fe3f0` |
| `fixtures/normalized/chaos_trade_implied_depth.jsonl` | `d447ad85a3e31bc35b73d844029068e68ee89cc6eaf04dfef27f5ab8ba670cf8` |
| `fixtures/normalized/chaos_trade_implied_depth_intents_v2.json` | `de11a8d56534bfc10d6da598e5fc6cfd717227d45a777220759454f9b3a624de` |

The implied-depth fixture has 10 atomic and 11 sequence cases. It fixes the
reversed taker-to-book side, strict aggressive crossing, equality and exact
half-level-quantity boundaries, arrival-order rather than exchange-timestamp
state, depth clearing, repeated and alternating trades, ignored-best and
pending-own-hedge interactions, the 30-millisecond pending-hedge expiry, the
100-microsecond callback, and the shared five-millisecond pricing worker. The
clock fixture separately fixes Java's decision, actual work-start, and finish
clock reads for immediate depth, immediate callback, and direct scheduled
timer paths. The RNG fixture fixes a 13-step interleaving, five service points,
and five exact draws.

The implementation is deliberately private to the existing ownership tree:

- `InstrumentState` owns the reached implied-depth cache, last-trade state,
  and pending locally sent hedge state.
- A qualifying public trade updates that state, reverses taker side to raw
  book side, and schedules one private callback from captured local receipt
  time plus exactly 100,000 nanoseconds only while the existing live gate is
  open. Exchange timestamps remain input data.
- The callback queue and shared pricing-worker timer queue are bounded at
  65,536. Common one/two-wake and one-timer cases are inline; the rare overlap
  is FIFO. Multiple qualifying trades retain callback multiplicity while
  pricing work follows Java's five-millisecond trailing conflation.
- Live uses one captured monotonic origin and services one due private action
  after shutdown, safety/control, operator, reconciliation, and periodic timer
  branches, immediately before ordinary feed receive. It does not add a
  socket, channel, subscription, public event, timer, configuration, journal,
  capture, or report field.
- Backtest uses its existing nanosecond scheduler and global sequence
  tie-breaker. It activates the shared worker only when the first qualifying
  trade makes the Java behavior reachable, promotes only causally accumulated
  depth timing state, and never scans future input.
- The live and replay paths distinguish receipt, processing, Java decision,
  actual work-start, finish, and post-local-reservation send clocks exactly.
  The ordinary compatibility wrapper derives a causal deterministic
  nanosecond clock from its event timestamp and fails closed on overflow.
- A hedge's implied-depth transition is compact, crate-private, non-Clone,
  non-serializable, and committed only after the genuine typed intent is
  accepted by the existing local reservation seam. A compile-fail fixture
  prevents public extraction. `ChaosExecutionIntent` remains exactly 80 bytes.

No public trade is reinterpreted as `BurstSignal`. Raw trade
matching/calibration remains separate from the strategy reaction. The
plan-derived trade subscription was already mandatory, and no connectivity,
exchange operation, dependency, command lane, public authority token, or
schema version was added.

#### Deterministic and authority gates

The final-tree focused/full commands were:

```text
TMPDIR=/home/ubuntu/code/reap/target/tmp CARGO_BUILD_JOBS=1 \
  cargo test -p reap-strategy --locked
TMPDIR=/home/ubuntu/code/reap/target/tmp CARGO_BUILD_JOBS=1 \
  cargo test -p reap-engine --locked
TMPDIR=/home/ubuntu/code/reap/target/tmp CARGO_BUILD_JOBS=1 \
  cargo test -p reap-backtest --locked
TMPDIR=/home/ubuntu/code/reap/target/tmp CARGO_BUILD_JOBS=1 \
  cargo test -p reap-live --locked --no-fail-fast
TMPDIR=/home/ubuntu/code/reap/target/tmp CARGO_BUILD_JOBS=1 \
  cargo clippy -p reap-strategy -p reap-engine -p reap-backtest -p reap-live \
  --all-targets --locked -- -D warnings
cargo fmt --all -- --check
cargo metadata --locked --format-version 1 >/dev/null
git diff --check
```

Results: strategy 107/107 plus three compile-fail boundary fixtures; engine
7/7; backtest 131/131; live 248/248 plus two compile-fail authority fixtures,
four dependency/source-policy tests, its runtime compatibility test, and all
doc tests. Clippy passed with warnings denied; formatting, metadata, and diff
checks passed.

The canonical no-trade CLI backtest was rerun twice. The two outputs compare
byte-identically with one another and with Phase 0, all at:

```text
38acf9f5e0c310f2ec5528974beffadf4c1a7f84d46efa8d9664ee7051e84691
```

The five new fixture files parse completely and bind the exact Java SHA.
Existing immutable anchors remain:

| Guard | Phase 2 result |
| --- | --- |
| Workspace adjacency, 23 lines | `fe98cedfaa2653e09afd57293eb71372ea476c1997e56b5ce9f27b314f5a432b` |
| Version inventory, 42 lines | `496653f1bba2b859a12ede28067a44a76935789d9cae7f39b4a59f95270333de` |
| Literal `AlertEvent` schema guard, one line | `b7038321dbc71e3d8c3f41f6adae0fce53221cd4fc701cc76b173aefb2f48b41` |
| `Cargo.lock` | `d8a19fb100aeb4e542a2135d546edfb5ae24629717f5ab65e285cf9bfe483b02` |

The scoped authority-declaration projection moves from 355 declarations and
`8254498e...` to 356 declarations and
`bf7af463d3c92836d58682d518942c1e2e3c52c62235fcd59eb3333f0a62e144`.
The sole addition is
`crates/reap-live/src/runtime/scheduling.rs: pub(super) fn
monotonic_now_ns(...)`, a parent-module-only clock helper with no execution
authority. The sibling checkout is still clean at the pinned full SHA.

#### Same-host benchmark gate

The existing benchmarks used one warm-up and three retained recorded runs:

| Benchmark | Warm-up | Recorded runs | Median | Phase 1 median | Delta |
| --- | ---: | --- | ---: | ---: | ---: |
| Engine event loop (ns/event) | 11,815.8 | 11,847.3; 11,795.9; 11,786.4 | 11,795.9 | 11,272.2 | +4.65% |
| Complete live parity (ns/raw) | 17,448.0 | 17,264.0; 18,651.7; 17,288.6 | 17,288.6 | 17,318.2 | -0.17% |

Every engine run retained 250,000 events and 999,996 intents. Every live run
retained 50,204 raw frames, 70,208 feed outputs, 65,130 records, zero actions,
4,193,771 allocation calls, and 1,871,951,969 requested bytes. Ordinary live
allocation counts therefore remain byte-for-byte at the Phase 1 baseline. The
18,651.7 ns/raw run is retained as a shared-host outlier.

The final-source action benchmark used a successful formal warm-up and five
retained recorded runs. Each workload has 10,000 internal warm-up and 100,000
timed observations, exact all-sample nearest-rank percentiles, a separate
fresh allocation pass, and zero dropped/overflowed samples. The tuple order
below is `p50/p95/p99/p99.9/max` nanoseconds:

| Workload | Runs 1 through 5 |
| --- | --- |
| Quote creation | `19035/24229/31450/182617/77816689`; `19224/26026/32311/94709/80280945`; `18937/23540/30465/74830/78265782`; `20176/24574/31088/118718/79603493`; `19093/23704/29940/105565/84602577` |
| Quote replacement/cancel | `13300/16565/19568/28233/304675`; `13235/16483/19159/24066/71990`; `13177/13645/18650/24442/99920`; `13957/14433/19151/24279/507952`; `13341/14540/19413/56270/5262242` |
| IOC hedge | `24311/29743/34289/47014/47045629`; `24352/30137/34330/46218/47933054`; `24180/26493/33328/47056/46194506`; `25624/27896/33485/43946/44986219`; `24410/27363/34149/65976/50051295` |
| Risk rejection | `12119/12726/17025/21341/78226`; `14720/21858/26797/44454/153646`; `11995/12390/17288/25862/2606872`; `12668/13128/17542/23951/84298`; `12021/12488/16976/22662/170245` |
| Symbol fail-close | `640/657/673/1518/32959`; `640/2601/4078/6917/17164`; `632/657/665/1354/16844`; `714/730/739/1690/48836`; `632/664/673/1075/12160` |
| Global fail-close | `796/829/853/4324/10461`; `796/2798/4250/7269/85307`; `787/813/829/1887/56007`; `878/911/936/4267/26649`; `788/829/853/3356/145285` |
| Coordinator reduction | `5736/5981/9617/16360/86038`; `5982/11749/15442/24213/108018`; `5727/5891/9231/14835/47974`; `5883/6088/7450/12808/104597`; `5751/5981/8853/13096/130870` |
| Raw recovery action | `26732/30309/36504/51568/6150388`; `28906/42026/48991/65680/6370634`; `26494/28758/37127/63104/6393116`; `27462/32853/41066/95801/6289881`; `26518/28906/35298/50026/6272511` |
| Public-trade reprice | `12283/12808/17124/21275/82730`; `12095/12529/16902/20275/79613`; `12160/12578/17246/21300/81000`; `12947/13579/17771/21111/76733`; `12201/12628/17198/22580/106722` |
| Bounded control/feed storm | `140/189/7622/7738/33033`; `140/197/7696/7762/13604`; `140/189/7655/7746/13637`; `140/205/7589/7704/15524`; `140/189/7672/7762/79678` |

The control/feed queue-age tuples were
`11954/16098/16968/43396/50985`,
`11938/15984/16476/20767/26535`,
`11971/16098/16706/21037/27413`,
`11930/16197/16886/21908/27676`, and
`11980/16172/17304/24910/96359` ns. Every run retained 20,000 control and
80,000 feed dequeues, 20,000 control preemptions, capacity/high-water 80, and
30,000 saturated offers.

The new public-trade workload is now a real action path: every run has 100,000
inputs, 100,000 private trade-reprice actions, and 400,000 ordered quote
intents/produced actions. Its five-run medians are
`12,201/12,628/17,198/21,275/81,000` ns for
`p50/p95/p99/p99.9/max` by independently taking the median of each statistic.
Its allocation projection is exactly 15,000,264 calls and 615,207,392
requested bytes, or 37.50066 calls and 1,538.01848 bytes per produced action.

All five runs and the final formal warm-up have the exact logical/allocation
projection SHA-256:

```text
451af08d7fa061f55f16b099faf85ca934d577fdf5fb8b219668716f9fc1015c
```

The projection includes every workload name, complete logical counters, and
total allocation calls/bytes. All ordinary rows retain their Phase 1
allocations. The IOC row adds exactly one allocation per authorized hedge
action (23,033,342 calls versus 22,933,342) because its compact private
transition retains an `Arc<str>` symbol while legacy lowering still creates
the unchanged public `String`; requested bytes fall by exactly 22,000,000
(1,228,030,376 versus 1,250,030,376). The permanent 80-byte intent-size guard
prevents this proof from inflating every quote/rejection row.

The five-run Phase 2 medians against Phase 1 were:

| Workload | p50 delta | p99 delta | p99.9 delta |
| --- | ---: | ---: | ---: |
| Queue storm | 0.00% | +0.98% | +1.40% |
| Coordinator reduction | +3.38% | +16.95% | +25.31% |
| Global fail-close | +2.18% | +0.95% | +131.15% |
| IOC hedge | +2.87% | +5.82% | +9.43% |
| Quote creation | +1.53% | +3.41% | +21.65% |
| Quote replacement | +3.24% | +7.02% | +9.64% |
| Raw recovery action | +1.02% | +4.12% | +23.77% |
| Risk rejection | +1.73% | +2.58% | -2.14% |
| Symbol fail-close | 0.00% | +1.20% | +41.21% |

The over-10% tails are explained shared-host noise rather than an unreviewed
reachable cost. Run 2 degraded unrelated paths together (for example risk
rejection p50 rose about 22% and fail-close p95 rose roughly fourfold), and
run 4 also showed broad unrelated slowing. During investigation the two-vCPU
host reported 96-97% idle CPU and zero steal but continuing swap-in activity
and concurrent Codex/Alloy processes. The affected ordinary source paths and
their exact allocations/logical counters are unchanged; the new trade row's
own p99/p99.9 is stable. Every run is retained, no retry is substituted, and
no target-host latency claim is made.

The updated connectivity/mapping documents now state the exact reached trade
behavior and retain all exclusions. Independent final fixture/guard review is
green. No Goal D stop condition was reached.

### Phase 3

The gated Phase 3 implementation/evidence commit is `e167c63`.

Phase 3 changes no production decision, connectivity, authority, storage, or
wire behavior. It adds a strict credential-free test harness, checked
fixtures, and a `#[cfg(test)]` trace at the existing
`LiveCoordinator::handle_engine_output` boundary. The trace is compiled out
of normal and release builds.

#### Executable equality boundary

The engine and live fixtures start each independent case from the same clean,
fully specified schema-1 initialization. They execute the production
`TradingEngine<ChaosStrategy>` and `RiskGate`; the live side additionally
continues through the genuine `LiveCoordinator`, `RegularExecutionPolicy`,
client-ID generator, owned-order reservation, canonical private
`PendingNew`, storage-record projection, and logical action projection.

Equality is exact for:

1. the canonical normalized or private due-reprice input and its declared
   scheduling clocks;
2. ordered typed Chaos purposes and their one-way legacy projections;
3. ordered engine/risk rejections, system events, and safety-cancel
   candidates; and
4. the engine batches captured before live routing.

The downstream live reduction is separately deterministic and golden. It is
not equated with simulated matching/accounting, and the default economic
backtest remains untouched. Raw private-feed identity, durable commit,
dispatch, reconciliation, exchange ambiguity, emergency behavior, network IO,
and credentials remain excluded.

The replay has eight contiguous, bytewise lexicographically ordered cases and
41 exact sequence rows. It covers quote, IOC hedge, public-trade reprice,
risk rejection, symbol halt, global kill, normalized fill/order state, and a
forced-repayment system-event path. A post-fill halt makes the filled
`client#1` absence visible in the exact cancellation set. The quote case
separates source/reservation time at 10 ms from observed/local-send time at
12 ms, proving that same-turn `PendingNew` and the private post-send strategy
transition use their distinct production clocks.

Generated client IDs are produced by the real generator and alpha-renamed
only in the evidence projection. Every identity link, ownership field,
record/action field, and order remains exact. The live test runs the entire
129,098-byte projection twice and compares bytes; the checked artifact is a
strict digest manifest rather than a duplicate large JSONL file.

#### Strict initialization and fixture hashes

The initialization artifact embeds the complete effective `ChaosConfig`,
every `RiskLimits` field, exact instrument models/limits, account/bootstrap
state, declared decision state, live readiness, and the ordered transitions
that produce it. The whole artifact hash is the exact configuration binding:
the authoring test reconstructs the embedded config from the checked source
plus explicit replay-only adjustments, calls `effective()`, and requires
structural equality; the parser separately requires the embedded config to be
an effective fixed point.

Unknown fields, emitted-field omissions, non-canonical replay case order,
sequence gaps, non-clean transition history, incomplete coverage, invalid
market/account numerics, duplicate identity rows, unscoped or extra private
snapshots, bootstrap/seed disagreement, stale health/reference state, and
readiness/seed contradictions all fail before construction.

| Artifact | SHA-256 |
| --- | --- |
| `fixtures/decision_parity/risk_initialization_v1.json` | `7e0951c41f447b9f46a73b24a3fe85bdc8f2bb8a623385dab0c3655926e73780` |
| `fixtures/decision_parity/replay_events_v1.jsonl` | `dede17a546d4d717c78dc2b3b7aa7c3f3f785d552404160407c78fb87cec9101` |
| `fixtures/decision_parity/expected_engine_v1.jsonl` | `140c268619b889a19d779e1bdfd340c11901d2eb1d9e4d216d976ba3d8b0d37a` |
| `fixtures/decision_parity/expected_live_reduction_v1.json` | `aa66cc09bba29cde25ab2df66c018517b2c900f83373f95580150e8bcd442b60` |

The live manifest binds 41 rows, 129,098 canonical bytes, and full-projection
SHA-256
`847c6f8ba5177cf456d0dc2c7c31df74a9b189c107e7167d06dd48bf09b7762b`.

#### Determinism, authority, and dependency gates

The final-tree commands included:

```text
TMPDIR=/home/ubuntu/code/reap/target/tmp CARGO_BUILD_JOBS=1 \
  cargo test -p reap-engine --test decision_replay --locked
TMPDIR=/home/ubuntu/code/reap/target/tmp CARGO_BUILD_JOBS=1 \
  cargo test -p reap-live --lib \
  coordinator::tests::decision_parity::initialized_live_reduction_matches_engine_decisions_and_is_byte_stable \
  --locked -- --exact
TMPDIR=/home/ubuntu/code/reap/target/tmp CARGO_BUILD_JOBS=1 \
  cargo test -p reap-live --lib \
  coordinator::tests::production_coordinator_keeps_single_owner_responsibility_state \
  --locked -- --exact
TMPDIR=/home/ubuntu/code/reap/target/tmp CARGO_BUILD_JOBS=1 \
  cargo test -p reap-strategy --locked
TMPDIR=/home/ubuntu/code/reap/target/tmp CARGO_BUILD_JOBS=1 \
  cargo test -p reap-engine --locked
TMPDIR=/home/ubuntu/code/reap/target/tmp CARGO_BUILD_JOBS=1 \
  cargo test -p reap-risk --locked
TMPDIR=/home/ubuntu/code/reap/target/tmp CARGO_BUILD_JOBS=1 \
  cargo test -p reap-backtest --locked
TMPDIR=/home/ubuntu/code/reap/target/tmp CARGO_BUILD_JOBS=1 \
  cargo test -p reap-live --locked --no-fail-fast
TMPDIR=/home/ubuntu/code/reap/target/tmp CARGO_BUILD_JOBS=1 \
  cargo clippy -p reap-engine -p reap-live --all-targets --locked -- -D warnings
cargo fmt --all -- --check
cargo metadata --locked --format-version 1 >/dev/null
git diff --check
```

Results: strategy 107/107 plus three compile-fail fixtures; engine 7/7 plus
four decision-replay tests and three ignored authoring helpers; risk 29/29;
backtest 131/131; live 249/249 with one ignored authoring helper, two
compile-fail boundaries, four dependency/source-policy tests, one runtime
compatibility test, and doc tests. Focused parity and single-owner tests,
clippy with warnings denied, formatting, metadata, and diff checks passed.

The canonical CLI backtest ran twice from the final source. Both outputs were
byte-identical at the immutable hash:

```text
38acf9f5e0c310f2ec5528974beffadf4c1a7f84d46efa8d9664ee7051e84691
```

The Phase 3 `Cargo.lock` hash is
`319268c86f94883e19668aa4835da615bbecbabfe32902019129e6e40caf894d`.
Its only delta adds already locked `serde` and `serde_json` as direct
`reap-engine` dev dependencies for strict fixture parsing/canonical JSONL. No
package/version or normal dependency edge was added.

The version inventory intentionally moves from 42 lines at `496653...` to 46
lines at
`a6201c31ccbd802fbd7b915eea2200b30fd3ade5df6a09662be35ac01363ab2c`.
The four additions are test-only schema-1 constants for initialization,
replay, engine projection, and live projection; this is not a production
schema migration.

Existing guards remain:

| Guard | Phase 3 result |
| --- | --- |
| Workspace adjacency, 23 lines | `fe98cedfaa2653e09afd57293eb71372ea476c1997e56b5ce9f27b314f5a432b` |
| Public authority surface, 356 declarations | `bf7af463d3c92836d58682d518942c1e2e3c52c62235fcd59eb3333f0a62e144` |
| Literal `AlertEvent` schema guard, one line | `b7038321dbc71e3d8c3f41f6adae0fce53221cd4fc701cc76b173aefb2f48b41` |

There is still no `reap-backtest -> reap-live` edge, public replay/runtime
mode, normal emergency reachability, or added exchange operation. The sibling
checkout remained clean at
`b6b120c7b7c466d8431bf082f3229328c5d7b2ae`.

#### Same-host benchmark gate

The existing whole-program benchmarks used a final warm-up and three retained
runs:

| Benchmark | Warm-up | Recorded runs | Median | Phase 2 median | Delta |
| --- | ---: | --- | ---: | ---: | ---: |
| Engine event loop (ns/event) | 11,754.7 | 13,432.3; 11,843.7; 11,773.4 | 11,843.7 | 11,795.9 | +0.41% |
| Complete live parity (ns/raw) | 17,489.4 | 17,356.6; 17,251.9; 17,180.6 | 17,251.9 | 17,288.6 | -0.21% |

Every engine run retained 250,000 events and 999,996 intents. Every live run
retained 50,204 raw frames, 70,208 feed outputs, 65,130 records, zero actions,
4,193,771 allocation calls, and 1,871,951,969 requested bytes.

The action benchmark used one formal warm-up and three initially required
runs. Quote-creation p99.9 crossed the investigation threshold, so all three
runs were retained and the series was extended to five warm recorded runs.
Each workload has 10,000 internal warm-up and 100,000 timed observations, a
separate fresh allocation pass, exact nearest-rank percentiles, and zero
dropped/overflowed samples. Tuple order is
`p50/p95/p99/p99.9/max` nanoseconds:

| Workload | Runs 1 through 5 |
| --- | --- |
| Quote creation | `19011/25075/32016/261246/80599598`; `19158/25279/34001/227736/79489079`; `18929/24180/31031/115238/76062930`; `18970/24065/31302/210170/75676798`; `18920/23876/31014/182905/75683112` |
| Quote replacement/cancel | `13424/15089/20176/61479/9582118`; `13161/13653/18559/22933/102480`; `13185/13563/18453/25460/118750`; `13210/14228/18683/24007/618924`; `13177/13858/18567/27831/132543` |
| IOC hedge | `24271/29932/34248/56139/46047190`; `24172/27093/33649/49451/45266432`; `24295/26510/32401/40327/45516702`; `24320/29038/37324/79162/46160141`; `24238/28217/33780/45759/50048759` |
| Risk rejection | `12004/12479/16812/20200/78193`; `12029/12439/16993/20956/120071`; `11971/12397/16779/20299/89196`; `12176/12775/17189/21268/91206`; `11955/13227/23343/26666/108331` |
| Symbol fail-close | `632/656/665/3093/28365`; `632/656/664/1354/11093`; `632/657/672/1263/11109`; `632/657/672/3224/8812`; `632/657/672/1312/11438` |
| Global fail-close | `788/813/829/3569/9788`; `788/829/846/4004/59068`; `788/821/845/1986/33468`; `788/829/853/3610/63399`; `788/821/862/1887/33172` |
| Coordinator reduction | `5735/5940/8197/12840/55868`; `5727/5940/8058/12972/79719`; `5719/5956/8705/13227/77291`; `5735/5990/8992/13070/87162`; `5768/5965/7877/12422/32426` |
| Raw recovery action | `26928/30325/37234/57467/6221903`; `26650/28726/36561/47638/6229697`; `26822/30169/37373/59412/6198494`; `26494/29472/36873/68783/6268063`; `26806/28996/36479/49615/6103480` |
| Public-trade reprice | `12193/12710/17058/20857/79334`; `12258/12685/17469/23746/2137605`; `12168/12561/16820/20127/86866`; `12275/12726/17173/20833/1231278`; `12291/13251/17920/25419/2494127` |
| Bounded control/feed storm | `140/205/7590/7664/36529`; `140/189/7623/7721/26543`; `140/196/7656/7746/12890`; `140/197/7680/7746/16565`; `140/189/7672/7746/20052` |

Queue-age tuples were
`11971/16328/16960/21710/53390`,
`11996/16263/16903/21300/43544`,
`11955/16155/16984/20882/26970`,
`11946/16081/16796/20766/26108`, and
`11954/16090/16829/21054/26461` ns.

Every warm-up and recorded action run retained exact logical counters and
allocation totals at:

```text
451af08d7fa061f55f16b099faf85ca934d577fdf5fb8b219668716f9fc1015c
```

The five-run medians are:

| Workload | p50 | p99 | p99.9 |
| --- | ---: | ---: | ---: |
| Quote creation | 18,970 | 31,302 | 210,170 |
| Quote replacement/cancel | 13,185 | 18,567 | 25,460 |
| IOC hedge | 24,271 | 33,780 | 49,451 |
| Risk rejection | 12,004 | 16,993 | 20,956 |
| Symbol fail-close | 632 | 672 | 1,354 |
| Global fail-close | 788 | 846 | 3,569 |
| Coordinator reduction | 5,735 | 8,197 | 12,972 |
| Raw recovery action | 26,806 | 36,873 | 57,467 |
| Public-trade reprice | 12,258 | 17,173 | 20,857 |
| Bounded control/feed storm | 140 | 7,656 | 7,746 |

Medians and p99 values are stable against Phase 2. Quote-creation p99.9 is
about 99% higher, while its p50 is 0.64% lower and p99 only 0.69% higher.
This isolated tail is retained as shared-host noise evidence: Phase 3 changes
no release production path, every allocation/logical count is exact, and the
investigation observed 91-97% idle CPU but active swap-in with about 3.48 GiB
swapped plus multiple unrelated Codex and Alloy processes. No retry replaced
an observation and no target-host latency claim is made.

The exact decision/risk boundary and exclusions are recorded in
`backtest-live-decision-parity.md`. No Goal D stop condition was reached.

### Phase 4

The gated Phase 4 implementation commit is `85af455`. It replaces the
regular-submit numeric representation only after the existing policy has
accepted an order. Strategy/model arithmetic, risk notional, `NewOrder`, the
economic backtest, and pinned-Java parity remain `f64`.

#### Exact metadata and canonical representation

`OkxExactDecimal` is a normalized checked `u128` coefficient and bounded
base-ten exponent. Its parser accepts ordinary and scientific notation,
normalizes leading/trailing decimal zeroes, limits source input to 512 bytes
and the absolute exponent to 400, and rejects empty, malformed, negative,
zero, coefficient-overflowing, underflowing, or overflowing values. It has
`Copy`, exact equality/hash, and canonical plain-decimal `Display`, but no
Serde implementation. `OkxRegularOrderRules` binds exact tick, lot, and
minimum-size values and rejects a minimum that is not an integral number of
lots.

The account-instrument parser now constructs those rules directly from the
exchange `tickSz`, `lotSz`, and `minSz` strings while retaining the existing
`f64` companions. The rules are a private, skipped/defaulted
`OkxInstrument` sidecar. Serializing and deserializing an instrument therefore
does not persist or recreate exact metadata. Bootstrap fails closed when the
sidecar is absent, requires each exact value's `f64` projection to have the
same bits as its parsed companion, and copies the rules into a private skipped
`VerifiedInstrument` sidecar. Its checked serialized key set remains exactly:

```text
account_id
contract_value
instrument_type
lot_size
min_size
order_limits
risk_model
symbol
tick_size
trade_mode
```

Deserializing that schema intentionally yields no exact rules and cannot
recreate live numeric authority. Periodic instrument drift now compares the
exact rules in addition to all prior fields; missing exact metadata is fatal.
A fixture proves that `0.1` and `0.10000000000000001`, whose `f64`
projections have the same bits, remain distinct drift metadata.

`VerifiedInstrument::new` replaces external struct-literal construction now
that the sidecar is private; it requires exact rules, while its rules accessor
is crate-private. `RegularExecutionProfile::new` likewise requires the rules.
Neither constructor mints the take-once `RegularApprovalScope` or its private
binding, so these inspectable data values cannot enter the one-shot submit
chain on their own.

The sole live composition seam requires these rules when it creates a
`RegularExecutionProfile`. Profile validation requires exact tick/lot/min
projections to match the authenticated floats bit-for-bit and proves the exact
minimum is an integral lot multiple. For each submit, the previous validation
order remains:

1. finite and positive quantity/price;
2. minimum quantity;
3. the existing ULP/alignment predicate;
4. exchange quantity limit;
5. risk-model notional limit; and then
6. exact lowering.

Lowering computes the nearest integral unit count only after those checks,
requires a positive exactly representable `u64` no larger than
`2^53 - 1`, and checked-multiplies it by the authenticated exact increment.
Zero, non-finite, misaligned, underflowed, overflowed, or otherwise
unrepresentable values cannot create prepared authority.

`CanonicalRegularOrderNumbers` is crate-private, non-Clone, and non-Serde.
It moves with the existing one-shot
`ApprovedRegularSubmit -> ReservedRegularSubmit -> PreparedRegularSubmit`
chain, so it exists before same-turn canonical `PendingNew` registration and
gateway idempotency. Prepared values expose only read-only references to the
canonical price and quantity. The idempotency registry and its reservation
result were tightened from public to crate-private. `OrderFingerprint` now
compares normalized canonical decimals: bit-distinct model floats that lower
to the same `px`/`sz` reuse an identity even if metadata expresses that value
with a different unit count, while `0.3` and `0.4` conflict.

The venue exact-decimal/rules types are public inspectable metadata values,
not send capabilities. Their lack of Serde does not by itself establish
authority; they cannot construct the private canonical payload, approval
binding, reservation, or prepared command. The adapter transport, request
builder, serializer, and command installation seam remain private.

#### Adapter and wire seam

`OkxPlaceOrder.price` and `.qty` now hold `OkxExactDecimal`.
`regular_place_order` assigns them exactly from
`PreparedRegularSubmit::canonical_price()` and `canonical_qty()`. The private
REST-shaped inner serializer uses one local `Serialize::collect_str` wrapper
over the canonical type. The websocket builder continues to place that same
serialized inner body in `args[0]`, so both paths receive byte-identical
plain-decimal `px` and `sz`.

The negative source-policy test rejects direct
`order.price.to_string()`, `order.qty.to_string()`,
`prepared.order().{price,qty}.to_string()`, and raw
`price: order.price`/`qty: order.qty` assignments. It also pins the exact
canonical assignments at the only normal submit lowering seam.

This did not add REST placement. The regular REST execution allowlist remains
only `POST /api/v5/trade/cancel-order`; the order websocket operation
allowlist remains exactly `order` and `cancel-order`. Algo/spread submit,
amend, and normal-live cancellation remain absent, and the emergency
authority boundary is unchanged.

#### Canonical behavior and regressions

The checked fixtures cover:

- integers, trailing-zero forms, `0.1`, `0.05`, `0.0001`, `1e-12`,
  scientific notation, and `9007199254740991`;
- malformed, negative, zero, too-long, coefficient-overflowing,
  decimal-underflowing, and decimal-overflowing metadata;
- exact minimum-in-lot divisibility and checked unit multiplication;
- the existing positive/finite, minimum, alignment, quantity-limit, and
  notional-limit policy results;
- two adjacent accepted `f64` price bit patterns lowering to canonical
  `0.3`, two-sided values 64 bit patterns outside the accepted tolerance,
  and a wire-distinct `0.4` idempotency conflict;
- the maximum `2^53 - 1` units, rejection above that bound, and
  multiplication overflow;
- 50,000 bounded exhaustive unit round trips: 10,000 counts for each of
  `1`, `0.1`, `0.05`, `0.0001`, and `1e-12`;
- exact REST-shaped/websocket field equality for all required numeric shapes;
  and
- compile-fail opacity of regular authority fields and constructors.

The inherited alignment predicate is intentionally unchanged:

```text
abs(value / increment - round(value / increment))
    <= 8 * EPSILON * max(abs(value / increment), 1)
```

At unit ratios above roughly `2^48`, that compatibility tolerance can admit a
wider fractional-unit offset than it does at ordinary magnitudes. Phase 4
does not tighten an existing acceptance rule. It only selects the nearest
bounded unit for a value that the predicate already accepted; this is not new
discretionary strategy rounding. Changing that large-unit acceptance edge
would be a separately reviewed policy change.

The authorized wire change is correspondingly narrow: an accepted value now
emits the canonical exchange increment multiple without scientific notation
or insignificant zeroes. Two accepted binary representations with one
canonical value can therefore change from distinct direct-`f64` strings to
one identical `px`/`sz`. No rejected order becomes accepted, and no
strategy/risk/backtest arithmetic changes.

#### Red/green and compatibility gates

The retained staged red gate is
`target/tmp/goal-d-phase4-live-red.txt`. Before the exact types were
implemented, it failed compilation with unresolved
`OkxExactDecimal`, `OkxExactDecimalError`, and `OkxRegularOrderRules`
imports. The subsequently added metadata, policy, idempotency, wire-equality,
source-policy, and opacity regressions are green. Diagnostic fixtures were
reviewed individually; no suite-wide `TRYBUILD=overwrite` was used.

Final-tree commands included:

```text
TMPDIR=/home/ubuntu/code/reap/target/tmp CARGO_BUILD_JOBS=1 \
  cargo check -p reap-live -p reap-okx-live-adapter --all-targets --locked
TMPDIR=/home/ubuntu/code/reap/target/tmp CARGO_BUILD_JOBS=1 \
  cargo test -p reap-venue --locked
TMPDIR=/home/ubuntu/code/reap/target/tmp CARGO_BUILD_JOBS=1 \
  cargo test -p reap-order --locked
TMPDIR=/home/ubuntu/code/reap/target/tmp CARGO_BUILD_JOBS=1 \
  cargo test -p reap-okx-live-adapter --locked
TMPDIR=/home/ubuntu/code/reap/target/tmp CARGO_BUILD_JOBS=1 \
  cargo test -p reap-live --locked
TMPDIR=/home/ubuntu/code/reap/target/tmp CARGO_BUILD_JOBS=1 \
  cargo test -p reap-live --test dependency_policy --locked
TMPDIR=/home/ubuntu/code/reap/target/tmp CARGO_BUILD_JOBS=1 \
  cargo clippy -p reap-venue -p reap-order -p reap-okx-live-adapter \
    -p reap-live --all-targets --locked -- -D warnings
cargo fmt --all -- --check
cargo metadata --locked --format-version 1 >/dev/null
git diff --check
```

Results: venue 36 tests plus compile-fail/doc tests; order 66 tests plus
compile-fail/doc tests; live adapter 38 passed, one ignored benchmark, and six
compile-fail fixtures; live 251 passed and one ignored helper, plus two
compile-fail boundaries, five dependency/source-policy tests, and the runtime
compatibility test. The focused exact-number, two-sided near-miss,
idempotency, serializer-equality, source-policy, bootstrap, and drift tests
passed. Check, relevant four-package all-target clippy with warnings denied,
formatting, metadata, and diff gates passed.

The canonical CLI backtest ran twice from the Phase 4 source. The outputs were
byte-identical and retained the immutable SHA-256:

```text
38acf9f5e0c310f2ec5528974beffadf4c1a7f84d46efa8d9664ee7051e84691
```

#### Dependency, schema, and authority inventories

No dependency was added. `Cargo.lock` remains:

```text
319268c86f94883e19668aa4835da615bbecbabfe32902019129e6e40caf894d
```

| Guard | Phase 4 result |
| --- | --- |
| Workspace adjacency, 23 lines | `fe98cedfaa2653e09afd57293eb71372ea476c1997e56b5ce9f27b314f5a432b` |
| Public authority declaration guard, 358 lines | `d40d247e8354f7fbca63df24a503017e2d9328250a7308181bcde174f8604cd8` |
| Version/revision inventory, 46 lines | `a6201c31ccbd802fbd7b915eea2200b30fd3ade5df6a09662be35ac01363ab2c` |
| Literal `AlertEvent` schema guard, one line | `b7038321dbc71e3d8c3f41f6adae0fce53221cd4fc701cc76b173aefb2f48b41` |

The declaration guard moved from 356 to 358 because it counts crate-private
declarations as well as public ones: Phase 4 adds the crate-private canonical
numeric struct/test constructor while converting `IdempotencyRegistry` and
`Reservation` from public to crate-private. Review found no new public
constructor for prepared authority, signer, raw request serializer, or
transport. There is still no `reap-backtest -> reap-live` edge, schema-version
change, new normal-live operation, or emergency reachability.

The sibling Java checkout remained clean at
`b6b120c7b7c466d8431bf082f3229328c5d7b2ae`.

#### Existing whole-program benchmark gate

The required existing benchmarks used one warm-up and three retained,
otherwise-idle runs:

| Benchmark | Warm-up | Recorded runs | Median | Phase 3 median | Delta |
| --- | ---: | --- | ---: | ---: | ---: |
| Engine event loop (ns/event) | 11,691.5 | 11,638.4; 11,775.6; 11,768.2 | 11,768.2 | 11,843.7 | -0.64% |
| Complete live parity (ns/raw) | 17,549.1 | 17,339.3; 17,327.8; 17,393.2 | 17,339.3 | 17,251.9 | +0.51% |

Every engine run retained 250,000 events and 999,996 intents. Every live run
retained 50,204 raw frames, 70,208 feed outputs, 65,130 records, zero actions,
4,193,771 allocation calls, and 1,871,951,969 requested bytes. The ordinary
non-action path therefore has exactly zero allocation drift from Phase 3.

#### Action-path benchmark and tail investigation

The formal action-path warm-up was followed by the three mandated runs.
Correlated p99/p99.9 excursions crossed the investigation threshold, so those
runs were retained and two more warm recorded runs were added rather than
replacing an observation. Each tuple below is
`p50/p95/p99/p99.9/max` nanoseconds for runs 1 through 5:

| Workload | Five recorded distributions (ns) |
| --- | --- |
| Quote creation | `19626/24632/30613/82057/83053950`; `19586/25066/31557/126684/85185540`; `19765/25780/33526/97967/83916942`; `19626/24114/30538/88212/84181857`; `19560/23778/30637/67026/83616930` |
| Quote replacement/owned cancel | `13177/13637/18256/22342/104671`; `13645/19823/23376/30925/482846`; `13201/13702/19634/127431/4782154`; `13251/13850/18322/21842/84331`; `13227/13670/18576/24336/89089` |
| IOC hedge | `24968/27577/33878/91157/45960682`; `27453/36085/42583/124929/50958133`; `24812/28061/35601/76979/47148833`; `24788/26929/33189/46900/45705898`; `24869/27158/33485/48680/45922872` |
| Risk rejection | `11922/12447/16918/23688/357072`; `12250/18010/21579/28972/181165`; `11946/12415/16730/20463/96769`; `11987/12447/16952/31565/2064663`; `12291/12742/17157/20324/82632` |
| Symbol fail-close | `632/640/665/1034/80146`; `632/681/3061/5498/137302`; `624/632/640/1133/11561`; `632/640/657/1173/26051`; `632/640/656/1083/53915` |
| Global fail-close | `763/788/813/3110/285410`; `772/820/3068/5226/17321`; `763/788/813/1814/15130`; `763/788/812/1903/22038`; `771/796/820/2002/28652` |
| Coordinator reduction | `5677/5924/7934/12603/76790`; `5744/10002/12570/18042/186638`; `5670/5973/9649/17756/63711`; `5662/5949/9296/14990/44750`; `5670/5973/9009/13104/38506` |
| Raw recovery action | `26584/31884/42067/175504/6179024`; `26854/34108/41435/140239/6362856`; `27035/32550/37694/63490/6405210`; `26649/32163/38965/92707/6229829`; `27043/29021/35371/49164/6384927` |
| Public-trade reprice | `12184/12627/17222/24623/175825`; `11979/14293/17411/44717/2670648`; `12135/12923/17287/22670/286640`; `11963/12496/16821/20751/623093`; `12045/12488/17328/34322/3228417` |
| Bounded control/feed storm | `140/189/7581/7639/18871`; `140/197/7614/7688/16853`; `140/189/7614/7713/17378`; `140/205/7606/7672/49255`; `140/205/7622/10076/15418` |

The five storm queue-age tuples were
`11914/16090/16574/20413/24106`,
`11946/16115/17370/22235/30514`,
`12143/16615/19117/21588/24319`,
`11939/16147/17198/24919/57549`, and
`12767/18576/21579/26436/38735` ns. Timer-read-overhead tuples were
`33/41/41/42/28069`, `33/41/41/42/5342`,
`33/41/41/42/41099`, `33/41/41/42/9091`, and
`33/41/41/42/30293` ns.

Component-wise five-run medians and their Phase 3 comparisons are:

| Workload | p50 | Delta | p99 | Delta | p99.9 | Delta |
| --- | ---: | ---: | ---: | ---: | ---: | ---: |
| Quote creation | 19,626 | +3.46% | 30,637 | -2.12% | 88,212 | -58.03% |
| Quote replacement/owned cancel | 13,227 | +0.32% | 18,576 | +0.05% | 24,336 | -4.41% |
| IOC hedge | 24,869 | +2.46% | 33,878 | +0.29% | 76,979 | +55.67% |
| Risk rejection | 11,987 | -0.14% | 16,952 | -0.24% | 23,688 | +13.04% |
| Symbol fail-close | 632 | 0.00% | 657 | -2.23% | 1,133 | -16.32% |
| Global fail-close | 763 | -3.17% | 813 | -3.90% | 2,002 | -43.91% |
| Coordinator reduction | 5,670 | -1.13% | 9,296 | +13.41% | 14,990 | +15.56% |
| Raw recovery action | 26,854 | +0.18% | 38,965 | +5.67% | 92,707 | +61.32% |
| Public-trade reprice | 12,045 | -1.74% | 17,287 | +0.66% | 24,623 | +18.06% |
| Bounded control/feed storm | 140 | 0.00% | 7,614 | -0.55% | 7,688 | -0.75% |

The investigation found a correlated shared-host tail signature rather than a
numeric-path throughput regression. Run 2 simultaneously inflated hedge,
risk, symbol/global cancel, coordinator, raw, and public-trade tails,
including paths whose Phase 4 production logic did not change. Runs 4 and 5
returned most p99 values toward the prior range. On the two exact-preparation
paths, quote/hedge p50 changed only +3.46%/+2.46% and p99
-2.12%/+0.29%; the quote p99.9 improved while the hedge p99.9 moved with the
same broad tail noise. Unchanged coordinator, raw, risk, and public-trade
rows also retained component p99.9 medians above 10%, which prevents
attributing those tails to canonical lowering. All samples, including
outliers, remain recorded; this shared-host regression gate is not a
target-host claim.

Every recorded action run retained exact logical counters and allocation
totals at:

```text
aa7eaa6e9bb6727b4d52e6f1488591904c4d823a45e31e6829f23d938def4ce6
```

Allocation changes from Phase 3 are:

| Workload | Calls delta | Requested-byte delta | Interpretation |
| --- | ---: | ---: | --- |
| Quote creation | +400,000 | +15,345,056 | +4 calls and about +153.45 bytes per prepared submit |
| IOC hedge | +400,000 | +15,145,056 | +4 calls and about +151.45 bytes per prepared submit |
| Raw recovery action | 0 | +8,000,000 | unchanged call count; +80 requested bytes per input |
| All other action rows | 0 | 0 | Exact |

Source review accounts for the submit-call increase in the checked decimal
product/display and finite-projection path: two canonical products are formed
per prepared submit. It is confined to action-producing preparation; the
complete non-action live benchmark proves no allocation increase. The raw
row's byte-only delta is retained rather than hidden. Its call count,
decisions, actions, records, and median remain exact/stable. Phase 5 may
profile these measured costs before considering an optimization; Phase 4 made
no speculative performance rewrite.

#### Prepared serializer benchmark

The adapter-private ignored release benchmark used one warm-up and three
recorded runs after canonical serialization. Each tuple is
`p50/p95/p99/p99.9/max` nanoseconds:

| Serializer workload | Warm-up | Three recorded runs | Median p50/p99/p99.9 |
| --- | --- | --- | --- |
| Prepared -> REST-shaped inner body | `681/698/706/1230/274661` | `640/649/657/1099/329536`; `648/657/673/4668/480639`; `665/673/681/1067/292581` | `648/673/1099` |
| Prepared -> websocket order request | `2354/2380/2412/7303/199265` | `2207/2232/2249/6564/350212`; `2207/2232/2256/7507/2071597`; `2216/2240/2257/6769/241833` | `2207/2256/6769` |

Against the Phase 0 five-run serializer medians, REST-shaped p50/p99/p99.9
changed -9.24%/-9.91%/-26.39%; websocket changed
-0.76%/-0.70%/-3.06%. Allocation calls remain exactly 6 per REST-shaped
body and 24 per websocket request. Requested bytes fell by exactly seven per
input/action: REST-shaped from 436 to 429, websocket from 1,557 to 1,550.
Serialized output remains exactly 142 bytes and 204 bytes per observation,
respectively.

Every recorded serializer run retained 100,000 prepared inputs/actions and
the exact logical/allocation projection SHA-256:

```text
1d4633a2d2634573a2d3cc790ed67b49427ef530ff3b115126e26e5199f3a0bb
```

Fixture assertions still compare `px` and `sz` bytes directly between both
serializers. Credentials, signing, transport queues, network IO, exchange
acknowledgement, and cancel/algo/spread operations remain outside this
benchmark and outside the newly changed numeric seam.

No Goal D stop condition was reached.

### Phase 5

The gated Phase 5 implementation is split into three reviewable commits:

| Commit | Scope |
| --- | --- |
| `f7de633` | Read-only, bounded storage-writer progress |
| `97a9970` | Private schema-version-1 runtime-health model and focused tests |
| `80a38fe` | Live integration, tracked bounded queues, source-policy guards, and final benchmark-boundary correction |

#### Runtime-health contract

`RuntimeHealthSnapshot` is private, `Serialize`-only, and grants no control,
configuration, transport, or order authority. `LiveRuntime` assembles it only
at the existing structured-log boundary every fixed five seconds with Tokio
`MissedTickBehavior::Skip`, and once after normal/error finalization. There is
no listener, socket, HTTP endpoint, supervisor protocol, configuration field,
new input, subscription, connection, exchange operation, or dynamic component
registration.

The schema-version-1 payload has fixed cardinality:

- feed, private, and order-command connectivity state and last-progress age,
  including an explicit `not_required` state while retaining `failed`;
- runtime-event-loop and per-order-task heartbeat instance/progress state;
- feed-ingress, control-ingress, and per-order-command-lane capacity, depth,
  high-water mark, continuous-backlog age, residence age, saturation, and
  saturation-event state;
- live readiness and active durable-safety-latch count;
- submit/cancel request and accepted-ack counts, local/exchange rejection,
  ambiguity, and total/clean reconciliation counts; and
- storage queue, enqueue/write/outstanding/durable-sync progress, writer age,
  drop count, and write/sync failures.

Progress paths use only predeclared enum IDs, fixed arrays, bounded
per-command-lane slices, monotonic timer reads, and numeric/enum atomics or
single-writer fields. They do not take a mutex/RwLock, format a string, clone a
symbol, allocate, or call the allocating `HealthRegistry::set`. JSON/string
construction happens only when a periodic or final snapshot is emitted.
Order counters advance only after the corresponding `StorageRecord` was
successfully queued, so telemetry cannot claim an uncommitted action.

Tracked queues retain Tokio mpsc as the sole capacity/ordering authority; no
second semaphore or gate was added. Drop guards account for messages discarded
with a receiver. Predeclared per-lane state prevents an order lane from hiding
saturation behind capacity in another lane. A nonzero `backlog_token` prevents
an equal-monotonic-timestamp drain/refill ABA from clearing the replacement
backlog's age. Snapshot assembly is explicitly a bounded, conservative fold of
adjacent lock-free lane states, never a synchronization, readiness, or control
boundary.

`StorageProgressSnapshot` is read-only numeric observation state. Its queue
depth counts channel residency separately from outstanding writer work; its
high-water, write, sync, durable, drop, and writer-progress counters are
saturating atomics. Receiver-drop, write-failure, and sync-failure tests keep
the counts truthful. Durable-latch resolution is nonfallible after its durable
ack and health-counter application follows that resolution, so observability
cannot block or reorder a safety commit.

The existing single writer, pre-existing select priorities, same-turn
reservation, storage-first action ordering, durable latches, shutdown order,
normal/emergency separation, exact connectivity plan, and regular-only
authority remain unchanged. Source-policy tests pin all eleven select
branches, placing the health tick after the existing ordinary timer and before
the private due-reprice/feed branches, and pin the actual snapshot emission
call and finalization sites.

#### Red/green and focused gates

The staged red evidence is retained as
`target/tmp/goal-d-phase5-storage-progress-red.txt` and
`target/tmp/goal-d-phase5-health-integration-red.txt`. The final-tree commands
included:

```text
TMPDIR=/home/ubuntu/code/reap/target/tmp CARGO_BUILD_JOBS=1 \
  cargo check -p reap-live --lib --locked
TMPDIR=/home/ubuntu/code/reap/target/tmp CARGO_BUILD_JOBS=1 \
  cargo test -p reap-live --lib --locked runtime::health::tests
TMPDIR=/home/ubuntu/code/reap/target/tmp CARGO_BUILD_JOBS=1 \
  cargo test -p reap-storage --locked
TMPDIR=/home/ubuntu/code/reap/target/tmp CARGO_BUILD_JOBS=1 \
  cargo test -p reap-live --lib --locked
TMPDIR=/home/ubuntu/code/reap/target/tmp CARGO_BUILD_JOBS=1 \
  cargo test -p reap-live --locked
TMPDIR=/home/ubuntu/code/reap/target/tmp CARGO_BUILD_JOBS=1 \
  cargo test -p reap-live --test dependency_policy --locked
TMPDIR=/home/ubuntu/code/reap/target/tmp CARGO_BUILD_JOBS=1 \
  cargo clippy -p reap-storage -p reap-live --all-targets --locked -- \
  -D warnings
cargo fmt --all -- --check
git diff --check
```

Results: all 18 focused health tests passed; storage passed 31 tests plus doc
tests; the live library passed 271 tests with one explicitly ignored helper;
the complete live package also passed both compile-fail UI fixtures, seven
dependency/source-policy tests, the compatibility test, and doc tests. The
focused all-target clippy, check, formatting, and diff gates passed. Independent
read-only audits of queue accounting/ABA, storage truthfulness/durable ordering,
and the completed health architecture were green. No suite-wide
`TRYBUILD=overwrite` was used.

#### Existing whole-program benchmark gate

The final existing benchmarks used one warm-up and five retained runs on the
otherwise-idle shared host:

| Benchmark | Warm-up | Five recorded runs | Median | Phase 4 median | Delta |
| --- | ---: | --- | ---: | ---: | ---: |
| Engine event loop (ns/event) | 11,972.2 | 12,298.8; 11,590.1; 11,618.3; 11,654.1; 11,674.7 | 11,654.1 | 11,768.2 | -0.97% |
| Complete live parity (ns/raw) | 17,783.1 | 17,198.2; 17,249.2; 17,929.8; 17,309.3; 17,361.3 | 17,309.3 | 17,339.3 | -0.17% |

Every engine run retained exactly 250,000 events and 999,996 intents. Every
live run retained exactly 50,204 raw frames, 70,208 feed outputs, 65,130
storage records, zero actions, 4,193,771 allocation calls, and
1,871,951,969 requested bytes. The ordinary non-action path therefore has zero
logical or allocation drift. These benches exercise the engine/coordinator
paths, not the production `LiveRuntime` select loop or snapshot assembly.

#### Action-path distributions and regression investigation

One warm-up and five recorded `reap-live/action_path` runs are retained in
`target/tmp/goal-d-phase5-action-{warmup,1,2,3,4,5}.txt`, with their
schema-version-1 JSON records beside them. Each workload has 10,000 warm-up and
100,000 post-warm-up observations. Every monotonic nanosecond sample is
retained and exact nearest-rank p50/p95/p99/p99.9/max is used; there is no
histogram, interpolation, reservoir, downsampling, drop, or overflow. Each
tuple is `p50/p95/p99/p99.9/max` nanoseconds, runs 1 through 5:

| Workload | Five recorded distributions |
| --- | --- |
| Quote creation | `19388/25886/43831/101815/84997355`; `19487/24968/32172/120359/85459483`; `19478/23909/30957/105008/91031430`; `19766/27897/34748/205345/121484156`; `19322/24188/31343/94587/84933455` |
| Quote replacement/owned cancel | `13013/13251/18083/21809/116855`; `15827/23114/27905/43470/164772`; `12947/13160/17821/21374/79990`; `16148/23794/28808/44504/293787`; `13013/13227/18043/21316/64278` |
| IOC hedge | `24574/26962/34124/46817/46812882`; `27085/39162/46769/85200/59094222`; `24984/28454/36381/107912/49965794`; `24557/26797/33952/49197/44897287`; `24771/31286/37726/66296/45531260` |
| Risk rejection | `12020/12348/16845/21661/384624`; `11897/12225/16616/20586/82796`; `11905/12233/16615/20332/403380`; `12119/12382/16828/20898/84298`; `12570/18830/22268/29957/135284` |
| Symbol fail-close | `632/640/648/1428/306439`; `632/640/656/1058/14334`; `640/640/649/1075/271281`; `648/657/665/1223/34986`; `640/640/649/1568/301384` |
| Global fail-close | `763/788/812/1977/303707`; `771/796/821/3389/39277`; `771/804/821/1863/285024`; `772/796/813/3980/44102`; `763/788/804/1920/2077234` |
| Coordinator reduction | `5596/5711/6130/12825/256503`; `5596/5703/6154/12964/100199`; `5595/5702/6096/11898/234301`; `5596/5702/6137/12792/75830`; `5571/7852/9977/14827/1445697` |
| Raw recovery action | `26010/28233/36685/56500/6211597`; `25961/29432/36635/57804/6147173`; `26535/33649/39819/96465/6273554`; `26478/31302/39679/84298/6135522`; `26403/28759/36938/57213/6186934` |
| Public-trade reprice | `12012/12324/17001/24377/6489787`; `12200/12472/17157/26675/2150298`; `12135/14917/17328/22202/149929`; `12078/12390/16730/19774/63195`; `12045/12332/16681/19741/79670` |
| Bounded control/feed storm | `140/189/7623/7860/30900`; `140/189/7589/7647/15688`; `140/197/7589/7688/16516`; `140/189/7614/7680/19683`; `140/197/7606/7697/13407` |

Component-wise medians and Phase 4 comparisons are:

| Workload | p50 / delta | p95 / delta | p99 / delta | p99.9 / delta | Highest max |
| --- | ---: | ---: | ---: | ---: | ---: |
| Quote creation | 19,478 / -0.75% | 24,968 / +1.36% | 32,172 / +5.01% | 105,008 / +19.04% | 121,484,156 |
| Quote replacement | 13,013 / -1.62% | 13,251 / -3.29% | 18,083 / -2.65% | 21,809 / -10.38% | 293,787 |
| IOC hedge | 24,771 / -0.39% | 28,454 / +3.18% | 36,381 / +7.39% | 66,296 / -13.88% | 59,094,222 |
| Risk rejection | 12,020 / +0.28% | 12,348 / -0.80% | 16,828 / -0.73% | 20,898 / -11.78% | 403,380 |
| Symbol fail-close | 640 / +1.27% | 640 / 0.00% | 649 / -1.22% | 1,223 / +7.94% | 306,439 |
| Global fail-close | 771 / +1.05% | 796 / +1.02% | 813 / 0.00% | 1,977 / -1.25% | 2,077,234 |
| Coordinator reduction | 5,596 / -1.31% | 5,703 / -4.52% | 6,137 / -33.98% | 12,825 / -14.44% | 1,445,697 |
| Raw recovery | 26,403 / -1.68% | 29,432 / -8.49% | 36,938 / -5.20% | 57,804 / -37.65% | 6,273,554 |
| Public-trade reprice | 12,078 / +0.27% | 12,390 / -1.88% | 17,001 / -1.65% | 22,202 / -9.83% | 6,489,787 |
| Control/feed storm | 140 / 0.00% | 189 / -4.06% | 7,606 / -0.11% | 7,688 / 0.00% | 30,900 |

Storm queue-age tuples were
`11946/16057/16713/20898/47244`,
`11914/16090/16746/20964/27479`,
`12004/16205/18896/21973/27331`,
`11922/16066/16672/21046/26182`, and
`11921/16032/16590/20766/25123` ns. Their component-wise median was
`11922/16066/16713/20964 ns`, stable or improved from Phase 4. Timer-read
tuples were
`33/33/41/42/22662`,
`33/33/41/42/160670`,
`33/41/41/42/18289`,
`33/41/41/50/43010`, and
`33/41/41/42/151299` ns; the p50/p95/p99/p99.9 median remained exactly
`33/41/41/42 ns`.

Every recorded run had the same canonical name/counter/allocation projection:

```text
aa7eaa6e9bb6727b4d52e6f1488591904c4d823a45e31e6829f23d938def4ce6
```

That is exactly the Phase 4 hash:

| Workload | Total allocation calls / requested bytes | Core logical counts |
| --- | ---: | --- |
| Quote creation | 19,733,342 / 836,475,432 | 100,000 inputs; 400,000 intents/actions; 100,000 prepared submits |
| Quote replacement | 16,600,000 / 773,500,000 | 100,000 inputs; 500,000 intents/actions; 100,000 prepared cancels |
| IOC hedge | 23,433,342 / 1,243,175,432 | 100,000 inputs; 500,000 intents/actions; 100,000 hedges/prepared submits |
| Risk rejection | 15,300,000 / 622,400,000 | 100,000 inputs; 400,000 rejections; zero produced actions |
| Symbol fail-close | 1,300,000 / 46,000,000 | 100,000 safety candidates/cancels/actions |
| Global fail-close | 1,600,000 / 52,600,000 | 200,000 safety candidates/cancels/actions |
| Coordinator reduction | 3,900,000 / 158,800,000 | 100,000 inputs/storage records; zero actions |
| Raw recovery | 21,980,027 / 1,777,312,151 | 200,000 frames/parses; 600,000 feed outputs; 500,000 normalized outputs; 100,000 actions; 900,003 records |
| Public-trade reprice | 15,000,264 / 615,207,392 | 100,000 inputs/reprices; 400,000 intents/actions |
| Control/feed storm | 0 / 0 | 20,000 control and 80,000 feed dequeues; 20,000 preemptions; capacity/high-water 80; 30,000 saturations |

The only positive Phase 4 comparisons over 5% were quote p99/p99.9
`+5.01%/+19.04%`, hedge p99 `+7.39%`, and symbol p99.9 `+7.94%` (90 ns).
The Phase 4-to-5 benchmark diff changes only reported tool/sample metadata and
corrects the declared preparation boundary; no timed workload body executes
the new runtime-health code. All p50 changes are within 1.68%, no p95
regression exceeds 3.19%, exact work/allocation hashes match, queue age is
stable or improved, and unrelated tails move in both directions while timer
maxima show host interruptions. All samples remain retained. The evidence
supports shared-host tail variation rather than an unexplained Phase 5
production-path regression; it is not a target-host latency claim.

The preparation rows explicitly use `PREPARED_ACTION_EXCLUDED` for quote,
replacement, hedge, and fail-close workloads; risk rejection retains
`ENGINE_EXCLUDED`. The raw/coordinator/public-trade/storm rows carry their own
actual boundaries. No benchmark row silently claims parsing, production
channel scheduling, storage enqueue/disk, adapter serialization, network IO,
or exchange acknowledgement that it does not include.

#### Prepared serializer distributions

The adapter-private release test used one warm-up and five recorded runs:

```text
TMPDIR=/home/ubuntu/code/reap/target/tmp CARGO_BUILD_JOBS=1 \
  cargo test --release -p reap-okx-live-adapter --locked \
  tests::goal_d_prepared_serializer_benchmark -- \
  --ignored --exact --nocapture
```

Each tuple is `p50/p95/p99/p99.9/max` nanoseconds:

| Serializer workload | Five recorded distributions |
| --- | --- |
| Prepared submit to REST-shaped inner body | `665/673/681/1132/315079`; `681/697/705/1124/140445`; `664/672/681/5628/54834`; `673/689/697/1469/1293914`; `657/673/681/4505/188263` |
| Prepared submit to websocket order request | `2248/2347/2626/6745/272339`; `2272/2363/2568/6826/81861`; `2232/2314/2617/10182/53415`; `2256/2355/2732/9082/1054436`; `2224/2355/3052/7130/126110` |

Component-wise medians are REST `665/673/681/1469 ns` and websocket
`2248/2355/2626/7130 ns` at p50/p95/p99/p99.9. Timer-read tuples were
`33/41/41/42/18289`,
`33/41/41/42/15269`,
`33/41/41/42/16574`,
`33/41/41/42/10798`, and
`33/41/41/42/12791` ns.

All five retained exactly 100,000 inputs/actions, 14,200,000 or 20,400,000
serialized bytes, and the same logical/allocation projection:

```text
1d4633a2d2634573a2d3cc790ed67b49427ef530ff3b115126e26e5199f3a0bb
```

REST remained exactly six allocation calls/429 requested bytes per action;
websocket remained exactly 24/1,550. Against Phase 4, REST p50/p99/p99.9
moved `+2.62%/+1.19%/+33.67%`, and websocket moved
`+1.86%/+16.40%/+5.33%`. Phase 5 changed neither the adapter source nor the
reachable serializer body. Exact work and central percentiles remained stable;
the two-vCPU shared host retained scheduling/code-layout tails. No observation
was discarded or retried away, and these upper-tail movements are explicitly
local variance rather than a target-host or decision-to-wire claim.

The measured boundary starts from an already-created
`PreparedRegularSubmit`, uses adapter-private `regular_place_order`, and ends
after the actual REST-shaped inner-body or websocket order-request serializer.
Typed strategy/policy/reservation/gateway preparation is untimed fixture setup.
Credentials, signing, transport queues, network IO, exchange acknowledgement,
and cancel/algo/spread operations remain excluded. No serializer, signer,
transport, prepared value, or authority constructor became public.

#### Phase conclusion

`performance.md` now separates the authoritative Goal C baseline, historical
attribution results, final Goal D local action/serializer evidence, exact
included/excluded boundaries, and the still-missing target-host
decision-to-wire evidence. Phase 5 introduced no trading behavior,
connectivity, normal/emergency reachability, schema migration, or dependency
change. No Goal D stop condition was reached.

## Remaining Operational Blockers Outside Goal D

Goal D does not and cannot clear:

- credentialed OKX demo observation/trading soak;
- target-host latency, queue, paging, supervision, deadman, and process-death
  evidence;
- target-account instrument/fee/position/cash/economic evidence;
- production approval or production order entry; or
- any future pinned-thread/SPSC/kernel-bypass decision.
