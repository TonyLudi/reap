# Chaos Connectivity Boundary

Status: normative for the current Chaos/iarb2 implementation.

This document defines the exchange capabilities that Reap may expose to the
current Chaos strategy. It is deliberately narrower than the capabilities
available in OKX, the generic Java platform, or Reap's emergency and evidence
tools. If a generic example in [architecture.md](architecture.md), a historical
mapping in [chaos-mapping.md](chaos-mapping.md), or an existing implementation
detail conflicts with this document, this document controls the Chaos
connectivity refactor.

The governing principle is:

> Grant the least authority needed for normal mutation, retain complete
> read-only coverage for risk observation, and isolate broad emergency recovery.

The implementation plan is
[chaos-connectivity-refactor-plan.md](chaos-connectivity-refactor-plan.md).
The words MUST, MUST NOT, SHOULD, and MAY are normative.

## Pinned Java Reference

The behavioral reference is the sibling repository at `../imm-strategy`,
resolved relative to the Reap repository root, at the exact revision:

```text
b6b120c7b7c466d8431bf082f3229328c5d7b2ae
```

That revision MUST agree with `reap_core::PINNED_JAVA_REVISION`. A goal MUST
stop and request a new audit if the sibling checkout, the Rust constant, or an
evidence binding names a different revision.

Only behavior transitively reached by the supported Chaos/iarb2 configuration
is a parity requirement. The primary source anchors are:

| Concern | Pinned Java source |
| --- | --- |
| Strategy event contract | `chaos/chaos-core/src/main/java/app/metcoin/chaos/ChaosStrategy.java` |
| Actual strategy initialization and subscriptions | `chaos/chaos-core/src/main/java/app/metcoin/chaos/ChaosStrategyBase.java` |
| OKX trade-driven implied depth | `chaos/chaos-core/src/main/java/app/metcoin/chaos/model/entity/OkEntity.java` |
| Quote TIF/STP selection | `chaos/chaos-core/src/main/java/app/metcoin/chaos/quoter/ChaosQuoteOptimizerBase.java` |
| Regular amend/cancel behavior | `chaos/chaos-core/src/main/java/app/metcoin/chaos/quoter/ChaosAmdOnceBatchQuoteOptimizer.java` |
| Iarb2 hedge creation | `chaos/chaos-iarb2/src/main/java/app/metcoin/chaos/iarb2/Iarb2Strategy.java` |
| Quote, hedge, index, and funding calculations | `chaos/chaos-iarb2/src/main/java/app/metcoin/chaos/iarb2/calculator/Iarb2Calculator.java` |
| Instrument and fee binding | `chaos/chaos-iarb2/src/main/java/app/metcoin/chaos/iarb2/model/Iarb2ChaosEntityFactory.java` |
| Per-entity regular cancellation | `chaos/chaos-core/src/main/java/app/metcoin/chaos/model/entity/ExchCancelAllFactory.java` |
| Stablecoin reference handling | `chaos/chaos-core/src/main/java/app/metcoin/chaos/fx/StableCoinDepegCheckerImpl.java` |

`ChaosLiveSub` is a standalone diagnostic subscriber and is not the
authoritative iarb2 subscription graph. `ChaosStrategyBase.doInit` is the
appropriate call-path reference.

The following do not grant a capability merely because they exist in
`../imm-strategy`:

- generic Luban/metcoin gateway wiring, including `ExecAlgo` processors;
- other strategies, venues, account types, or Binance-specific behavior;
- exact websocket pool cardinality or generic dispatch infrastructure;
- Spring, Redis, master/group control feeds, and deployment machinery; or
- exchange features that are not reached by the supported iarb2 path.

No algo-order or exchange-spread mutation is reachable from the pinned
Chaos/iarb2 strategy path. Pinned Java does, however, use regular batch amend
for one non-Binance live quote optimizer, chooses GTC for quote targets at or
inside its computed mid and PostOnly otherwise, sends explicit `CancelMaker`
STP on quotes. Generic iarb2 delegates hedge reduce-only selection to the
exchange entity, and the supported `OkEntity` returns false; Reap's
non-reduce-only OKX hedge matches that path. Reap currently realizes quote
changes with cancel/new and always uses PostOnly with no per-order quote STP.
Those quote behaviors are intentional current execution-policy differences,
not behavior absent from Java.

This refactor preserves those current Reap request profiles. Because
[OKX uses the account-level `acctStpMode` when an order omits `stpMode`](https://www.okx.com/docs-v5/en/),
live bootstrap and periodic account-config checks MUST require
`acctStpMode = cancel_maker` before a quote with no per-order STP can be sent.
Changing quote TIF/STP semantics is a separately reviewed parity change.

## Capability Admission Rule

Every connection, subscription, authenticated read, and mutation MUST have all
of the following:

1. a stable requirement identifier from this document or an approved extension;
2. one named consumer in the current Chaos, risk, readiness, recovery, or
   evidence flow;
3. one owning trust plane and a role-specific interface;
4. the narrowest mode in which it is needed; and
5. a test proving that the resolved connectivity plan includes it when required
   and excludes it otherwise.

Capability presence in a shared client, an exchange SDK, or the pinned Java
platform is not a requirement. A capability without a traceable consumer MUST
be removed from that composition root.

## Trust Planes

| Plane | Purpose | Mutation authority |
| --- | --- | --- |
| Chaos decision | Consume normalized state and emit typed purposes | None; it cannot construct venue requests |
| Live observation | Connect, normalize, sequence, deduplicate, and report feed health | None |
| Regular execution | Realize approved Chaos quote, hedge, and owned-cancel purposes | Regular limit place and owned regular cancel only |
| Live safety and readiness | Bootstrap, reconcile, maintain regular deadman protection, and consume forbidden-domain proof | Canonical owned regular cancel and regular Cancel All After only |
| Forbidden-domain observer | Prove that algo and spread pending order sets are empty | None |
| Emergency account stop | Make an uncertain account safe independently of the live process | Enumerate/cancel regular, algo, and spread orders; arm regular/spread deadmen; never submit |
| Offline evidence | Collect and verify account/exchange evidence outside the trading loop | None |

The emergency plane is not a superset object that the live process may hold in
reserve. It MUST have a separate composition root and MUST NOT be constructible
or importable from the Chaos execution composition. Its mutation authority MUST
live in a dedicated crate and executable absent from the `reap-live` dependency
graph. Separate credentials, subaccount isolation, and an endpoint-filtering
proxy SHOULD be used because OKX API permission labels are coarser than the
Rust type boundary.

## Chaos Input Contract

The target implementation MUST resolve one concrete
`ChaosConnectivityPlan` from the effective Chaos, risk, account, and runtime
configuration. The names below are stable requirement identifiers, not a
mandate for a particular Rust type layout.

| Requirement | Input | When required | Consumer |
| --- | --- | --- | --- |
| `CHAOS-MD-BOOK` | Sequenced full book for each configured instrument | Always | Quote and hedge calculations |
| `CHAOS-MD-TRADE` | Public trades for each configured instrument | Always in the pinned live scope | Java OKX implied-depth invalidation and repricing; known Rust behavior gap |
| `CHAOS-REF-INDEX` | Configured instrument index ticker | Only for configured index symbols | Index deviation, valuation, and pricing |
| `CHAOS-REF-FUNDING` | Funding rate | Only for configured swaps | Funding-aware pricing and risk |
| `CHAOS-REF-MARK` | Mark price | Only for configured derivatives | Derivative valuation and safety |
| `CHAOS-REF-LIMITS` | Exchange price limits | Only for configured instruments that enforce them | Quote and hedge price bounds |
| `CHAOS-STATE-ORDERS` | Account-scoped regular order updates | Live execution | Canonical order state, fill identity, and convergence |
| `CHAOS-STATE-ACCOUNT` | Account and balance state | Live execution | Cash, equity, liability, margin, and risk checks |
| `CHAOS-STATE-POSITIONS` | Position state | Live derivatives | Delta, margin, ownership, and fill convergence |
| `CHAOS-TIMER` | Deterministic timer events | Always | Time-based hedge, debounce, stale, and stop logic |
| `SAFE-STABLECOIN` | Configured stablecoin/USD index | Only for currencies named by a risk guard | Entry block and durable risk latch |
| `SAFE-METADATA` | Instrument, fee, and account-mode metadata | Live bootstrap and periodic drift checks | Request validation and fail-closed readiness |
| `SAFE-CLOCK-STATUS` | Exchange time and relevant service status | Live bootstrap and periodic safety checks | Clock and relevant maintenance guard |
| `SAFE-RECONCILE` | Regular pending orders, fills, balances, and positions | Startup, recovery, and periodic/on-demand repair | Authoritative state convergence |
| `SAFE-ACCOUNT-POSITIONS` | Account-wide position observation | Every live account, including spot-only Chaos | Detect foreign, unmanaged, or wrong-mode derivative exposure |
| `SAFE-FORBIDDEN-ZERO` | All pending algo query families and pending spread orders | Startup and periodically in live modes | Prove unsupported exposure is absent |

### Live Versus Capture Inputs

Raw public trades are part of capture and backtest matching/calibration. They
are also a pinned live Chaos input. `ChaosStrategyBase.doInit` delivers them to
`OkEntity.onPublicTrade` and `Iarb2Strategy.onPublicTrade`; an aggressive
trade can invalidate implied depth and schedule repricing. Current Rust
`MarketEvent::Trade` only advances strategy time, so trade-driven implied
depth is a documented parity gap.

This no-semantic-change refactor MUST retain the plan-derived live trade
subscription for every configured instrument. Removing it requires a separate
parity decision that either implements the reached Java behavior or explicitly
narrows the parity claim with new fixtures and approval. Capture keeps its
explicit public-trade subscriptions, and backtest keeps its offline trade
matching/data requirements; neither is folded into a generic live connectivity
framework. A normalized full book satisfies the current Rust BBO use.

`act_on_burst = true` requires a real, explicitly modeled `BurstSignal`
provider. Raw trades are not a substitute. Live configuration MUST reject that
setting until such a provider, freshness rule, and acceptance tests are in
scope.

Private fills MAY arrive through the order stream and an additional
fee-bearing fill channel when both feed one canonical, deduplicated fill
consumer. A second channel MUST NOT be opened merely because the venue offers
it.

## Chaos Output Contract

The strategy's executable surface contains exactly three purposes:

| Requirement and purpose | Regular order profile | Required fields |
| --- | --- | --- |
| `CHAOS-EXEC-QUOTE` — `Quote` | Limit, PostOnly | Configured owned symbol/account, `reduce_only = false`, no per-order STP; verified account `acctStpMode = cancel_maker` |
| `CHAOS-EXEC-HEDGE` — `Hedge` | Limit, IOC | Configured owned symbol/account, `reduce_only = false`, `CancelMaker` STP |
| `CHAOS-EXEC-CANCEL-OWNED` — `CancelOwned` | Cancel one regular order | Canonical order identity proven to belong to this strategy/account/symbol |

The table describes the current Reap contract. The hedge reduce-only profile
matches pinned OKX `OkEntity`; quote amend/TIF/per-order-STP policy remains an
intentional difference from the Java quoter.

The Chaos decision layer SHOULD emit typed `Quote`, `Hedge`, and
`CancelOwned` values. A regular execution policy MUST construct the final venue
request and validate the complete field profile immediately before transport.
Free-form strings such as the current `reason` field MAY remain as evidence,
but MUST NOT be the authority that selects an order type.

Normal live safety MAY cancel a regular order reconstructed from durable state
or authoritative reconciliation only when a documented ownership predicate
proves its account, symbol, and Reap client identity. Unknown or foreign regular
orders block readiness and require operator handling; they do not widen Chaos
authority.

The Chaos and in-process live execution planes MUST NOT represent or reach:

- market, GTC, FOK, trigger, conditional, OCO, chase, iceberg, TWAP, or smart
  algo placement;
- exchange-spread placement;
- amend or batch amend;
- arbitrary `reduce_only` or STP combinations;
- cancel of an unknown, foreign, algo, or spread identifier; or
- arbitrary authenticated paths or raw request signing.

A strategy halt, disconnect, shutdown, or risk breach may widen the action from
one owned cancel to cancellation of all canonical owned regular orders and
regular Cancel All After. It does not grant algo/spread mutation to the live
process.

The additional mutation/recovery identifiers are:

| Requirement | Authority |
| --- | --- |
| `SAFE-REGULAR-CANCEL` | Cancel canonical owned regular orders during fail-closed live safety |
| `SAFE-REGULAR-CAA` | Arm, refresh, and safely disable regular Cancel All After |
| `OPS-EMERGENCY-REGULAR` | Enumerate/cancel all regular account orders and arm regular CAA |
| `OPS-EMERGENCY-ALGO` | Enumerate and cancel all pending algo families |
| `OPS-EMERGENCY-SPREAD` | Enumerate/cancel spread orders and arm spread CAA |
| `OPS-EMERGENCY-IDENTITY` | Read account configuration only to bind the emergency report to account identity |
| `CAPTURE-PUBLIC-MARKET` | Credential-free books, trades, and configured reference capture |
| `EVIDENCE-PUBLIC-READ` | Credential-free time, status, index, and market evidence |
| `EVIDENCE-ACCOUNT-READ` | Authenticated read-only account config, instruments, fees, orders, fills, bills, balances, and positions |
| `TEST-FAULT-TRANSPORT` | Loopback-only fault proxy connectivity used by controlled tests |

`ChaosConnectivityPlan` is the live strategy plan. Capture, offline evidence,
emergency, and fault tooling MUST resolve their own narrow plans/contracts and
MUST NOT union their capabilities into the Chaos plan.

## Mode Composition

| Mode | Allowed composition |
| --- | --- |
| Validate | Resolve, validate, hash, and print the secret-free plan; no credential read, network connection, signer, or role client |
| Observe | Planned public/private observation, authenticated reads, reconciliation observation, and forbidden-domain proof; no place, cancel, or Cancel All After role is constructed |
| Demo order entry | Observe capabilities plus approved regular execution and regular live-safety cancel/CAA |
| Production order entry | Unavailable |
| Capture | Separate credential-free `CAPTURE-PUBLIC-MARKET` contract; no live/private role |
| Emergency | Separate confirmed emergency plan and executable; no strategy or submit role |
| Offline evidence | Separate read-only evidence plan/executable; no strategy or mutation role |

Mode tests MUST prove absent roles are not merely disabled by a boolean after
construction; their credentials, clients, sessions, and methods are not
constructed or reachable in that composition.

## Connectivity Allowlist

The resolved plan MUST express paths and websocket channels explicitly. The
following is the target allowlist by role; blank cells mean no authority.

| Capability family | Live observation | Regular execution | Live safety/readiness | Forbidden observer | Emergency stop | Offline evidence |
| --- | --- | --- | --- | --- | --- | --- |
| Public books and required references | Subscribe |  | Observe health |  |  | Read/capture |
| Private orders, account, positions, required fills | Subscribe/normalize |  | Reconcile health |  |  | Read/verify |
| Public time and relevant system status |  |  | Read |  | Read time | Read |
| Account config, instruments, and fees |  |  | Read |  | Identity read only | Read |
| Regular pending orders/details/fills |  |  | Read |  | Enumerate | Read |
| Regular place |  | Approved Quote/Hedge only |  |  |  |  |
| Regular single cancel |  | Approved CancelOwned only | Canonical owned safety cancel |  | Cancel any account order |  |
| Regular Cancel All After |  |  | Arm/refresh/disable under shutdown rules |  | Arm |  |
| Pending algo families |  |  |  | Enumerate only | Enumerate | Read if an evidence contract requires it |
| Pending spread orders |  |  |  | Enumerate only | Enumerate | Read if an evidence contract requires it |
| Algo cancel |  |  |  |  | Cancel |  |
| Spread mass cancel/deadman |  |  |  |  | Cancel/arm |  |
| Algo or spread submit |  |  |  |  |  |  |
| Account bills and historical evidence |  |  |  |  |  | Read |
| Transfer, withdrawal, leverage, borrowing, or account administration |  |  |  |  |  |  |

Live readiness MUST remain false and regular order placement MUST remain
disabled when `SAFE-FORBIDDEN-ZERO` finds a nonzero algo/spread set or cannot
prove zero because of timeout, malformed data, duplicates, pagination failure,
or endpoint failure. The observer has no cancellation method.

The forbidden-domain proof is per account. Its default maximum age is 30
seconds, its configured maximum MUST NOT exceed 60 seconds, and a recurring
scan MUST start no later than half of the maximum-age interval. The initial
complete zero proof is required before readiness. An expired proof disables
placement immediately. The current readiness gate is conjunctive and global:
one account's invalid proof blocks all new placement, while canonical
owned-regular cancellation and reconciliation remain targeted to the affected
account. Observe has no mutation role; only Demo can dispatch the owned
cancellation.

Regular cancellation, regular Cancel All After, and regular reconciliation
have priority over the sentinel. Algo and spread scans MUST use bounded
per-domain deadlines and a pacing budget that cannot consume or hold the
regular-safety budget. A nonzero or unverifiable result MUST create a typed
critical alert event instructing operators to run the separate emergency command;
an evidence-bearing Demo configuration MUST enable its delivery sink. The live
process does not invoke or import that command.

## Connection And Status Planning

Connections are a result of the requirement plan, not a user-selected pool that
the strategy must fill.

- Public subscriptions MUST be the deduplicated union of exact plan
  requirements. Readiness waits for every required subscription and no unused
  subscription.
- Public replica cardinality is explicit per requirement. A second socket is
  allowed only when a named redundancy consumer performs independent
  sequencing, deduplication/arbitration, and ready-replica recovery. Otherwise
  the requirement has one socket. A global replica count is not itself a
  requirement.
- Compatible private orders, account, positions, and required-fill channels
  share one authenticated state socket per account by default after
  permutation and failure-domain tests prove the coordinator remains correct.
  Additional private sockets require a documented ordering, isolation, or
  rate/capacity rationale; splitting one channel per socket by convention is
  forbidden.
- Each executing account starts with one order-command shard. Account-scoped
  dispatch families are derived from configured Chaos instruments, currently
  using the canonical OKX underlying key, and assigned deterministically to
  that shard.
- Additional shards require an explicit measured capacity or fault-isolation
  requirement in the plan. Every shard MUST own at least one dispatch family,
  and tests MUST enforce configured exchange connection bounds.
- `replicas_per_shard`, if introduced, MUST be separate from shard count.
  More than one replica may be claimed only after pre-write failover semantics
  make it safe to route before any bytes could have reached the exchange.
- Without that failover, each shard has one active command lane. A dispatch
  family has exactly one owning lane, and idle authenticated sessions are
  forbidden.
- A disconnected unused lane MUST NOT affect readiness because the plan must
  not create it.

The pinned Java eight-session pool is a connectivity reference, not a
strategy-parity cardinality requirement.

Maintenance relevance MUST also be plan-derived. A status event blocks Chaos
only if its environment, service, product, and configured instrument scope can
affect a required read or regular-order action. Spread-only maintenance cannot
halt regular Chaos trading. Unknown or ambiguous status that may affect a
required capability still fails closed.

## Emergency Semantics

Account-wide emergency recovery intentionally covers regular, algo, and spread
orders because incident response must handle exposure not created by Chaos.
Each domain MUST run as an independent mitigation workflow with its own
enforcement of the one absolute per-account deadline, progress, incidents, and
final-zero proof. The deadline remains anchored before exchange-clock sampling
so a failed or hung clock request cannot reset the account timeout.

- Failure or slowness in algo/spread enumeration MUST NOT delay the first
  regular enumeration or cancellation attempt.
- Failure in one domain MUST NOT stop mitigation in another domain.
- Each internal domain workflow MUST expose typed progress and be independently
  observable to its coordinator, telemetry, and deterministic tests. The
  existing serialized report keeps its current per-domain counts/zero flags and
  aggregate enumeration attempt/failure fields; this refactor does not claim or
  require new per-domain enumeration fields.
- `all_clear` is true only when every domain is authoritatively zero and the
  required evidence is complete.
- Evidence incompleteness MUST be reported separately from whether a mitigation
  action was attempted or acknowledged.
- The emergency plane MUST never submit a new order.

Goal A deliberately replaced the pre-refactor combined enumeration loop, where
failure of any algo/spread query could prevent that iteration from reaching
regular cancellation.

## Structural Enforcement

Role names are not sufficient if a caller can bypass them. The target design
MUST enforce:

- arbitrary `SignedRequest` construction, `signature`, `sign_request`,
  signer/credential getters, websocket login signing, and raw production
  `HttpTransport::execute` are private to role-owned wire adapters;
- role clients expose explicit methods and cannot be converted into a broader
  client;
- authenticated private-feed factories expose only the planned
  orders/account/positions/required-fill channels, and order-command factories
  expose only approved regular place/cancel operations;
- the live Chaos composition root cannot depend on emergency mutation types;
- emergency and evidence adapters live in crates absent from the `reap-live`
  dependency graph; shared lower crates expose only credential-free
  contracts/parsers, not authenticated execution;
- the strategy crate remains venue-independent;
- wildcard exports do not re-export raw or emergency authority;
- test fakes implement role ports rather than gaining production signing
  access; and
- one coordinator remains the single writer of strategy, risk, and canonical
  order state.

Rust visibility reduces accidental authority. Operational containment still
requires dedicated accounts/keys where possible, secret separation, and
network policy because a compromised process holding an OKX trade key may have
coarser exchange-side permissions than its Rust interface exposes.

## Change Control

A new strategy or exchange capability is admitted only as an explicitly
reviewed vertical slice containing:

1. a pinned-reference or Reap safety requirement;
2. a stable requirement identifier and connectivity-plan entry;
3. a typed strategy purpose or role method;
4. pre-trade/risk validation and ownership rules;
5. live and deterministic backtest/matcher semantics where applicable;
6. endpoint/channel allowlist tests and negative reachability tests; and
7. updated mapping, operations, and acceptance evidence.

The existence of a generic Java implementation or OKX endpoint is not enough.
Algo placement, spread placement, amend, new venues, a generic strategy
framework, and master/group control feeds are separate design decisions, not
extensions hidden inside this refactor.

## Boundary Acceptance

The boundary is structurally enforced when all of the following are true:

- the Java checkout and Rust pin agree on the full SHA;
- every live connection, subscription, read, and mutation resolves from a
  requirement identifier;
- the normalized ordered Chaos intents and deterministic backtest fixtures are
  unchanged;
- only Quote, Hedge, and CancelOwned can reach regular execution;
- Validate constructs no credential/network role, Observe constructs no
  mutation/CAA role, and Demo constructs only the approved live roles;
- quote-capable accounts are blocked unless `acctStpMode = cancel_maker`;
- live execution cannot import arbitrary signing, algo/spread mutation, amend,
  or trigger APIs;
- forbidden-domain checks are read-only and fail closed;
- the sample configuration creates no idle order-command lane, retains the
  pinned `CHAOS-MD-TRADE` input, and opens no unplanned subscription;
- every extra public/private/command socket has a tested
  redundancy/ordering/isolation/capacity consumer;
- spread-only maintenance is irrelevant to the resolved Chaos plan;
- emergency failure in algo/spread cannot delay regular mitigation, while any
  unverified domain keeps `all_clear` false; and
- production order entry remains disabled and no credentialed operation is
  required to complete the refactor.
