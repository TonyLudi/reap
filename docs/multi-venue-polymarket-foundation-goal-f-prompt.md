# Multi-Venue Polymarket Foundation Goal F Execution Prompt

Status: next runnable architecture goal. Goal E remains deferred because no
target host or target-host acceptance contract is declared. Goal F is
independent of Goal E, may run before it, and neither completes nor
requalifies Goal E.

Use this document as the complete instruction set. The exact invocation is:

> /goal Execute Goal F exactly as specified in
> `docs/multi-venue-polymarket-foundation-goal-f-prompt.md`. Continue phase by
> phase through every green gate and stop only at completion or a documented
> stop condition.

## Objective

Add Polymarket as a first-class, statically composed venue in Reap while
preserving the existing bounded Chaos/OKX product.

The new product topology is:

```text
OKX public crypto reference prices
                 +
Polymarket public book/market data
                 +
Polymarket private order/fill events
+ reconciled Polymarket account/position snapshots
                 |
                 v
pure PM quote-model boundary
                 |
                 v
Polymarket passive quote / cancel-owned effect
```

OKX is public reference data only in this product. Polymarket is the only
execution venue. There is no Predict.fun connectivity, exact quote mirroring
between prediction venues, or PF-maker/PM-hedge behavior.

This is an architecture, deterministic behavior, and credential-free
foundation goal. It MUST NOT invent or approve a production probability
formula, use credentials, make authenticated exchange requests, submit a live
order, claim target-host performance, or authorize demo/production trading.

The required outcomes are:

1. exact Polymarket identity, price, quantity, and lifecycle types;
2. narrow capability-specific venue contracts rather than one broad exchange
   adapter;
3. Polymarket public market-data parsing, integrity, capture, and deterministic
   replay;
4. a read-only Polymarket order, balance, allowance, and position convergence
   model;
5. a canonical passive-quote order lifecycle exercised only through fake
   transport;
6. a sibling PM coordinator and composition root with one mutation owner;
7. a statically supplied pure quote-model boundary consuming OKX public
   reference state and Polymarket state;
8. structural, deterministic, bounded-memory, authority, and local
   performance-regression evidence; and
9. unchanged Chaos/OKX behavior, connectivity, authority, existing encodings,
   fingerprints, semantics, and canonical outputs.

## Normative Baseline And References

The starting Reap implementation baseline is commit
`8258deb4b6e3a52e7c58c792da913210e0877fbb`. Starting `HEAD` MUST contain that
commit. A reviewed prompt-only commit may also be present.

Treat these Reap documents as normative:

- [architecture.md](architecture.md) for ownership, event flow, and async-edge
  rules;
- [chaos-connectivity-boundary.md](chaos-connectivity-boundary.md) for the
  existing Chaos/OKX capability boundary;
- [maintainability-refactor-goal-c-handoff.md](maintainability-refactor-goal-c-handoff.md)
  for source-size, ownership, and local benchmark gates;
- [determinism-readiness-goal-d-handoff.md](determinism-readiness-goal-d-handoff.md)
  for the completed deterministic and exact-order baseline;
- [performance.md](performance.md) for local measurement methodology and
  limitations; and
- [trading-readiness.md](trading-readiness.md) for the distinction between
  implemented mechanics, credentialed evidence, and trading approval.

The existing Chaos behavioral reference remains the clean sibling checkout
`../imm-strategy` at
`b6b120c7b7c466d8431bf082f3229328c5d7b2ae`. Do not modify it. Its supported
Chaos/iarb2 call path remains normative only for existing Chaos behavior; it
does not define Polymarket behavior or broaden Goal F.

The Polymarket protocol reference is the tracked Git object in `../predarb` at
`8222273a9c72033b760e1d2fec813bc77144556d`. Use it only for reached
Polymarket public wire formats, fixture-only private/order/account response
interpretation, canonical unsigned order fields, lifecycle fixtures, and
position-reconciliation lessons.

`../predarb` is not a clean normative checkout at prompt creation: its
dashboard has a tracked modification and its untracked `.predarb/` directory
contains runtime recovery state. Goal F MUST:

- record the actual revision and dirty paths without reading or exposing
  secrets;
- read normative tracked source from the pinned Git object, for example with
  `git show`, or from a clean detached read-only worktree at that exact commit;
- never delete, reset, move, ignore, interpret as fixtures, or copy untracked
  `.predarb/` runtime state;
- never read `.env`, `.env_bk`, private keys, API credentials, or other secret
  files;
- never treat the modified dashboard or any untracked byte as behavioral
  authority; and
- never modify `../predarb`.

Predict.fun adapters, PF strategy behavior, market-pair discovery, inverse
PM-maker/PF-hedge modes, cross-venue hedge accounting, settlement tooling,
Telegram reporting, and the Predarb application/runtime shape are not
normative.

Reap MUST NOT acquire a Cargo path dependency on `../predarb`, make it a
workspace member, or import its root application. Port only the minimum
reviewed behavior and fixtures, recording source commit, source path, and
fixture/content hashes in the Goal F handoff.

Before editing production code, verify and record:

- Reap revision, branch, clean state, and whether another session is writing
  overlapping files;
- the Reap baseline is an ancestor of `HEAD`;
- `../imm-strategy` is clean and exactly pinned;
- the tracked `../predarb` reference commit is available and its current dirty
  paths are recorded without alteration;
- `Cargo.lock`, workspace dependency graph, public exports, schema-version
  inventory, canonical Chaos backtest/replay hashes, and relevant fixture
  hashes;
- current largest production files, functions, and state aggregates;
- the existing source-policy and compile-fail ownership tests;
- one warm-up plus three recorded same-host runs of the existing engine, live,
  and action-path benchmarks; and
- the exact candidate crate/module dependency graph proposed for Goal F.

Phase 0 creates
`docs/multi-venue-polymarket-foundation-goal-f-handoff.md`. It records all
baseline facts, decisions, exact commands and results, fixture provenance,
phase commits, benchmark results, authorized schema additions, limitations,
and deferrals throughout the goal.

## Product And Semantic Boundary

For Goal F, “PM” means Polymarket.

The architecture MUST support this exact division of responsibility:

| Concern | OKX role | Polymarket role |
| --- | --- | --- |
| Public market data | Configured crypto reference prices only | Configured outcome-token metadata/books; trades only when a concrete model requirement declares them |
| Private data | None | Orders, fills, and session lifecycle through fixture/fake roles |
| Account state | None | Collateral, per-spender allowance, outcome-token inventory, and complete-snapshot state |
| Order reconciliation | None | Open orders, fills, and order detail through fixture/fake roles |
| Mutation | None | Fake-transport passive quote and cancel-owned only |

The product consumes OKX crypto prices to evaluate a Polymarket fair
probability. It does not copy an OKX price into a PM price, match a Predict.fun
quote, mirror a prediction-venue quote, or infer that one prediction price is
the executable price on another venue.

The quote-model boundary owns the transformation:

```text
OKX reference state
+ Polymarket market specification and lifecycle
+ Polymarket book
+ time/model inputs
+ inventory, allowance, reservation, and risk/readiness state
-> fair PM probability
-> validated PM passive quote candidates
```

Goal F MUST define and test this pure input/output boundary but MUST NOT invent
the production economic model. A deterministic fixture model may exist only
in tests, benches, examples, or an explicitly non-production composition used
to prove event flow. No default quote model exists. Release-shaped composition
requires an explicit model type, and absence of one constructs no private or
mutation roles. Goal F creates no deployed PM binary, systemd service, or live
CLI entry.

Binary complement arithmetic such as `1 - p` is allowed only in a
strategy-specific helper after explicit metadata proves that the outcomes are
complementary. It is not a generic venue or book rule.

## Target Dependency And Ownership Shape

Prefer the following responsibility boundaries unless Phase 0 proves a
smaller layout with the same dependency and authority separation:

- `reap-pm-core`: structural PM identities, exact values, metadata, normalized
  domain events, lifecycle enums, and no IO;
- `reap-pm-state`: pure PM book, order, reservation, position, and readiness
  reducers;
- `reap-polymarket-wire`: public REST/WS DTOs and parsing, fixture-only private
  response/event DTOs, and canonical unsigned order fields; it has no signer,
  key, authentication-header, API-key, authenticated-session, or signed-request
  implementation in Goal F;
- `reap-polymarket-adapter`: the public observation adapter plus fixture/fake
  private-read, reconciliation, position, and execution roles, with no broad
  client escape;
- `reap-pm-strategy`: the pure, statically dispatched quote-model and strategy
  boundary;
- `reap-pm-live-contracts`: secret-free configuration validation and the exact
  product connectivity/capability plan; and
- `reap-pm-live`: a sibling PM composition root and deterministic coordinator.

The existing `reap-live`, `reap-live-contracts`, `reap-order`,
`reap-okx-wire`, `reap-okx-live-adapter`, and Chaos strategy remain the
existing Chaos/OKX product. They MUST NOT gain PM credentials, DTOs,
configuration, order profiles, execution behavior, or dependencies on PM
product crates. When a shared exhaustive venue match requires it, an existing
product may add only an explicit fail-closed unsupported-Polymarket case.

Shared substrate may gain genuinely venue-neutral types and mechanics, such
as:

- a common venue/source identity that represents both OKX and Polymarket, plus
  structural account/instrument envelopes;
- bounded transport supervision;
- capture/replay framing;
- leased durable writer mechanics;
- monotonic timestamp and queue-age carriers;
- health/telemetry mechanics; and
- generic book containers when exact behavior and existing serialized bytes
  remain proven.

Do not extract a speculative universal order kernel, generic plugin registry,
dynamic venue registry, or one union adapter. Extract shared code only when
both existing OKX and new PM use exactly the same invariant and the extraction
preserves the existing public and serialized behavior.

The PM product has one canonical mutation owner. The coordinator owns inline,
by-value book, order, position, readiness/risk, quote-model, and scheduled
action state. These are separate responsibility modules, not independent
services with shared mutable state.

Before coordinator ingress, configured OKX instruments, PM markets/tokens,
accounts/funders, and allowance spenders are mapped to validated compact/static
handles. Canonical state and queues are preallocated or otherwise explicitly
bounded. Unknown identities are rejected or quarantined at the edge; the owner
loop cannot grow unbounded maps, clone raw dynamic IDs per event, or register
dynamic metric labels.

Network, parsing, capture, persistence, and telemetry workers stay at bounded
async edges and return immutable typed events or results. The coordinator
performs no network IO, file IO, blocking logging, credential access, or
unbounded work.

## Exact PM Identity And Numeric Contract

Do not reuse bare `Symbol = String` as PM identity and do not encode venue
identity with `.PM`/`.PF` suffixes.

Define structural, non-secret identities sufficient to distinguish:

- venue;
- condition/event where applicable;
- CLOB market where distinct;
- outcome token;
- outcome metadata/label without treating the label as identity;
- account scope, chain, signer identity, and funder identity;
- allowance spender;
- client order, venue order, trade/fill, connection epoch, and snapshot; and
- configured OKX reference instrument to PM market mapping.

Do not expose private key material or API credentials through any identity,
configuration projection, debug output, cloneable session, or strategy type.

PM executable values MUST use exact, heap-free value types:

- `PmPrice` is totally ordered, serializable, hashable, and exact;
- a live-tradable `PmPrice` is strictly greater than zero and strictly less
  than one;
- prices are validated against explicit venue/market tick metadata;
- `PmQuantity` is exact and validated against explicit lot, minimum, and wire
  units;
- executable order/fill/reservation quantities are positive; a zero
  book-delta quantity is admitted only by an explicit delete-level
  representation;
- book levels, order intents, fills, cumulative fill, reservations, balances,
  allowances, and positions never round-trip through `f64`;
- checked conversion rejects overflow, underflow, negative quantity,
  non-representable units, and unapproved rounding; and
- zero/one outcomes are terminal market/lifecycle facts, not executable quote
  prices.

Deserialization, configuration, replay fixtures, and raw venue parsing cannot
mint approved or executable order authority. Unchecked/executable constructors
stay private. An approval is bound to the exact market metadata/grid revision,
account scope, token, side, canonical units, and quote profile.

Numerically equal text such as `0.1` and `0.10` MUST canonicalize to the same
units, hash, idempotency identity, and canonical serialized value. Values that
produce different wire units MUST conflict. Invalid or off-grid
deserialization must fail or remain a clearly non-executable raw value.

If quote policy requires directional rounding, implement it once before
approval at an explicitly named quote-policy boundary, prove that it preserves
passivity, and produce a newly validated exact candidate. Wire lowering only
validates and serializes exact integral protocol units; it never rounds.
Do not inherit discretionary `round_dp`, float-to-string, or string-to-float
behavior from the reference.

Model fair value and executable price are different types. A fixture model may
produce `f64` only when the strategy boundary performs one finite, checked,
side-aware conversion into exact PM tick units before approval. PM books,
orders, fills, reservations, balances, allowances, and positions remain
float-free.

Existing Chaos strategy/model arithmetic remains `f64` and unchanged. A
mechanical extraction of an existing exact OKX wire representation is allowed
only if all existing APIs, fingerprints, request bytes, fixtures, and
canonical outputs remain identical.

## Capability And Authority Contract

Use narrow, non-interchangeable roles. The exact Rust names may follow the
code, but the architecture MUST distinguish:

1. public market-data observation;
2. private order/fill event observation;
3. read-only order reconciliation;
4. read-only balance/allowance/position snapshots; and
5. owned passive-quote execution.

A broad trait or object that combines connect, public data, private reads,
place, cancel, cancel-all, and arbitrary commands is forbidden.

The secret-free PM connectivity plan MUST be a typed composition of:

1. the statically supplied quote model's public/time/model input requirements;
2. the fixed fake passive-quote and cancel-owned execution profile; and
3. mandatory safety, private-lifecycle, reconciliation, account, allowance,
   and position-monitor requirements.

The pure economic quote model can declare only public/time/model inputs; it
cannot mint private-read, reconciliation, position, or mutation authority. The
composed plan MUST reject unused/undeclared config entries rather than taking a
configured union or adapter inventory. It enumerates every reached:

- configured OKX public reference instrument and channel;
- configured PM outcome token and public channel;
- PM private channel represented by fixture/fake roles;
- read-only endpoint/purpose represented by fixture/fake roles;
- passive quote profile;
- owned cancel purpose;
- account/funder/spender scope;
- connection and queue lane;
- readiness dependency; and
- capability/role constructor.

If no model is supplied, normal product construction creates no quote or
mutation path. Credential-free public capture and an explicitly composed
least-authority read-only PM account/position monitor remain independent tools;
their private/authenticated implementations are still fixture/fake-only in
Goal F. Tests MUST prove a one-to-one mapping from each typed requirement
source to plan entry to constructed role, with no unused plan or role entry
and no authority crossing between requirement sources.

At Goal F completion:

- OKX supplies no private, account, reconciliation, signer, or mutation role
  to the PM product;
- PM public observation can be constructed without any private role;
- PM private read roles are distinct from mutation;
- only an in-process fake transport can consume approved/prepared PM
  quote/cancel values;
- no live PM mutation role is constructible from `reap-pm-live`;
- no strategy, coordinator, state reducer, configuration type, or test fake
  can retrieve a signer, credential, arbitrary HTTP client, raw request
  executor, or broad adapter;
- cancellation requires canonical local ownership proof; and
- source-policy, dependency, visibility, and compile-fail tests reject bypass
  attempts.

Goal F adds no signer, private-key type, authentication-header builder,
API-key creation/derivation, authenticated-session factory, or signed-request
implementation. Move those capabilities and their vectors to the later
authenticated-execution goal. Goal F cannot load real secrets, open an
authenticated session, expose a live submit constructor, or send a signed
request.

## Event Lanes And Backpressure Contract

All queues are bounded and have semantic overflow behavior:

- raw snapshot/delta streams needed for book integrity are not silently
  dropped or coalesced; overflow invalidates readiness and requires explicit
  gap recovery/resynchronization;
- private order/fill events are lossless within the declared capacity or fail
  closed and force reconciliation;
- order submit/cancel results and safety/control events are lossless or stop
  the product;
- complete account snapshots are atomic and versioned; partial pages cannot
  masquerade as complete state;
- after a valid book/reference reducer owns the full state, explicitly
  documented reduced BBO or reference observations may be latest-wins;
- reconciliation and storage work have bounded age/starvation policies; and
- telemetry may be sampled/coalesced only when it cannot affect state,
  readiness, ownership, or recovery.

Define deterministic ordering for equal-time inputs. Preserve distinct:

- venue/event timestamp where present;
- local wall receive timestamp;
- monotonic receive/service timestamp;
- connection epoch;
- venue sequence/hash where actually supplied; and
- local ingress sequence.

Do not synthesize a PM venue predecessor sequence from local message order and
do not force PM through the OKX sequence contract. Exchange timestamps are
data, never elapsed-time clocks.

The coordinator's priority contract MUST be explicit and replayable. Private
fills/order lifecycle, safety/control, public state, reconciliation, scheduled
actions, persistence acknowledgements, and telemetry cannot depend on Tokio
select randomness or hash-map traversal order.

## Non-Negotiable Invariants

Preserve exactly:

- all completed Goal A through Goal D Chaos/OKX behavior, authority,
  dependency, source-policy, and compile-fail boundaries;
- the supported Chaos connectivity plan and its exact Quote, Hedge, and
  CancelOwned purposes;
- existing Chaos strategy decisions, traversal and RNG order, floating-point
  operation order, timers, risk decisions, serialized artifacts, fixtures,
  and canonical hashes;
- existing OKX regular PostOnly quote, IOC `CancelMaker` hedge, and
  cancel-owned semantics;
- one canonical writer per product and no shared mutable canonical strategy,
  book, order, position, portfolio, risk, or readiness state behind
  `Arc<Mutex<_>>` or `RwLock`;
- same-turn reservation before fake dispatch, storage-before-effect ordering,
  take-once authority, owned cancel proof, reconciliation, and deterministic
  shutdown;
- separate normal, evidence, and emergency authority planes;
- bounded channels and explicit saturation behavior;
- adapter-private raw wire types and credential material;
- unchanged existing Chaos/OKX encodings, fingerprints, semantics, and
  canonical bytes. New PM-specific schemas and explicitly versioned,
  backwards-readable shared envelope additions are allowed only with recorded
  migrations and old-reader/fixture compatibility proof; and
- `production_order_entry_authorized: false` in every evidence, report, and
  approval artifact.

Also require:

- no task per order or task per synthetic cancel;
- no dynamic plugin registry or new hot-path trait-object dispatch;
- no JSON parsing, JSON construction, formatted logging, blocking IO,
  persistence serialization, secret access, or network `.await` in canonical
  reducers;
- no silent unknown order status mapped to a normal state;
- no ambiguous remote order claimed or cancelled as locally owned;
- no new production module above 1,500 lines without a responsibility-based
  exception recorded before implementation; and
- no new function above 250 production lines without a focused,
  responsibility-based justification and decomposition review.

## Execution Discipline

Work phase by phase. For each phase:

1. inventory callers, data ownership, schemas, fixtures, capabilities,
   dependency rules, and source-policy guards before editing;
2. add focused failing tests or golden fixtures before semantic
   implementation;
3. separate mechanical extraction from semantic changes;
4. prefer static concrete composition and by-value state;
5. run focused unit, deterministic, authority, dependency, and benchmark gates;
6. inspect the complete diff, public exports, dependency graph, file/function
   sizes, allocations, and serialized bytes;
7. record exact commands, results, hashes, decisions, and limitations in the
   Goal F handoff;
8. commit the phase only when its gate is green; and
9. continue automatically while green.

Do not push or change a remote branch unless the user separately requests it.
Do not run another overlapping code-writing goal concurrently.

Do not use `TRYBUILD=overwrite` across a suite. Review every changed diagnostic
fixture individually and accept only the intended rejected operation and
privacy/type reason.

## Phase 0: Baseline, Product Contract, And Dependency Plan

Phase 0 changes documentation only.

Create the Goal F handoff and record all baseline/reference checks listed
above. Add a secret-free Polymarket product/connectivity boundary document or
an equivalent clearly delimited section in the handoff containing:

- the OKX-reference/PM-execution topology;
- the exact configured-instrument identity model;
- public, private-read, reconciliation, position, and fake-mutation capability
  matrices;
- endpoint/channel/purpose classifications derived only from the reached PM
  scope;
- the PM price and quantity invariants;
- the authoritative market-metadata readiness contract: configured
  condition/market/token membership, active/closed/accepting-orders and
  order-book-enabled lifecycle, tick, minimum/lot/wire units, neg-risk/domain
  identity, and required allowance spenders;
- the event-lane priority, capacity, saturation, and replay rules;
- account, funder, token, spender, and order ownership boundaries;
- the target crate/module dependency graph;
- public API and persisted-schema additions;
- known Predarb reference defects and behaviors that must not be copied;
- test/fixture provenance and differential-test plan;
- the absence of a production probability model;
- the exact local performance measurement contract; and
- explicit exclusions and stop conditions.

Phase 0 MUST fix the following stable target names for all later gates,
regardless of the private module layout:

```bash
cargo test -p reap-pm-live --test combined_replay --locked
cargo bench -p reap-pm-live --bench pm_action_path --locked
```

The measurement contract MUST predeclare PM nominal and overload workloads,
exact logical/action/record counts, allowed drops, allocation and
bounded-memory budgets, queue capacities/high-water/age rules, and run
validity. Nominal runs require zero drops and zero saturation. Overload runs
must reach the exact declared fail-closed/resync behavior without unbounded
state or memory growth.

Freeze the only Goal F fake execution profile as:

- configured Polymarket outcome token;
- `GTC`;
- passive/post-only;
- exact tick-aligned price strictly in `(0, 1)`;
- exact lot/minimum-aligned quantity;
- the sides admitted by the pure quote policy and readiness state; and
- cancellation only of a proven locally owned quote.

If the pinned reference or authoritative protocol fixtures do not prove this
profile's domain semantics and canonical unsigned order fields, stop in Phase
0 and record the missing contract. Do not substitute FOK, IOC/FAK, synthetic
IOC, market order, reduce-only, cancel-all, or another profile.

Record the known order/fill risk before implementation: the reference REST
conversion can treat an individual trade's size as cumulative filled quantity
and zero remaining quantity, while later validation expects cumulative
progress. Goal F MUST independently define and prove per-owned-order fill
normalization rather than porting this behavior.

Also record and turn into failing fixtures before porting:

- allowance parsing that selects an arbitrary first map value or collapses
  required spenders into one effective boolean/amount; and
- six-decimal maker/taker amount conversion that rounds rather than rejecting
  non-integral protocol units.

Record one warm-up and three same-host runs of:

```bash
cargo bench -p reap-engine --bench event_loop --locked
cargo bench -p reap-live --bench live_loop --locked
cargo bench -p reap-live --bench action_path --locked
```

Gate Phase 0 with a documentation-only commit.

## Phase 1: Exact PM Domain And Venue-Aware Envelopes

Implement the pure identity and numeric foundation.

Requirements:

- extend the common venue/source identity so it represents both OKX and
  Polymarket. Existing Chaos code may gain only a mechanical, explicit
  fail-closed unsupported-venue case; it cannot gain PM DTOs, configuration,
  execution logic, or PM product dependencies;
- add structural PM identities and explicit account/funder/spender scope;
- add exact `PmPrice` and `PmQuantity` with private unchecked constructors;
- enforce strict live price range, tick, lot, minimum, overflow, and conversion
  rules;
- add venue/source-aware generic envelopes without erasing concrete payload
  types;
- retain venue and account identity through normalized market, order, fill,
  balance, allowance, and position events;
- introduce generic book/event containers only where they remain statically
  typed and preserve existing Chaos aliases and serialized bytes;
- keep PM condition/outcome relationships in PM metadata rather than bloating
  generic core with binary-market assumptions; and
- add checked mappings from configured OKX reference instrument to configured
  PM market/outcome token without embedding a pricing formula.

Tests MUST cover:

- zero, one, negative, greater-than-one, NaN/infinity source rejection where
  conversion accepts floats, overflow, underflow, and non-representable units;
- minimum and maximum valid executable prices;
- tick and lot alignment;
- exact serialize/parse/wire round trips;
- invalid/off-grid deserialization and the inability of Serde to mint approval;
- canonical equivalence of numerically equal textual values and distinct
  idempotency for wire-distinct units;
- BUY/SELL canonical maker/taker integer-amount vectors covering
  price-times-quantity cross-products, half-unit cases, overflow, and exact
  unsigned bytes without rounding;
- structural identity collision resistance across venue, market, token,
  account, funder, and spender;
- no `.PM`/`.PF` suffix parsing;
- no `f64` round trip on executable PM state; and
- unchanged existing OKX/Chaos fixture and canonical bytes.

Add dependency/source-policy and compile-fail tests proving PM strategy/state
cannot construct unchecked values or access raw wire/auth types.

Phase 1 is green only when the pure domain has no network, filesystem,
credential, Tokio, live-runtime, or strategy-policy dependency.

## Phase 2: Capability-Specific Venue Framework Seams

Create the narrow shared seams needed to treat PM as another venue without
making all venue semantics identical.

Requirements:

- define separate public observation, private lifecycle, reconciliation,
  position snapshot, and owned execution contracts;
- use associated/concrete venue types rather than an authority-erasing
  universal order payload;
- retain static composition; no runtime plugin lookup or arbitrary venue
  command enum;
- move subscription serialization, acknowledgement classification,
  heartbeat, integrity/sequence interpretation, and resync policy behind a
  venue-owned session protocol;
- preserve common bounded transport supervision, reconnect/backoff, health,
  capture, and shutdown mechanics;
- define product event lanes and deterministic cross-lane ordering;
- make account and position state independently consumable from an order
  transport; and
- preserve the existing OKX/Chaos feed, live, order, configuration, and
  authority behavior exactly.

Add structural tests proving:

- `reap-live`, `reap-live-contracts`, and `reap-order` have no PM DTO,
  configuration, execution, or product dependency. A mechanical exhaustive
  match may explicitly reject the shared Polymarket venue identity but cannot
  compose or process PM behavior;
- the PM product cannot construct an OKX private/order role;
- an observation-only composition cannot obtain PM mutation;
- private read roles cannot be converted into mutation;
- raw wire clients and credentials cannot escape the adapter/wire boundary;
- all queues are bounded; and
- every capability-plan entry maps to one role and every constructed role maps
  back to an exact plan entry.

Do not force PM through OKX login, subscription-acknowledgement, or sequence
semantics merely to reuse an interface.

## Phase 3: Polymarket Public Market Data, Integrity, And Replay

Port or independently implement only the reached Polymarket public protocol
behavior.

Requirements:

- raw REST/WS DTOs remain in the wire crate;
- parsing produces exact typed PM values and structural token identity;
- authoritative metadata proves configured condition/CLOB-market/outcome-token
  membership, active/not-closed/accepting-orders and order-book-enabled
  lifecycle, tick size, minimum/lot/protocol units, neg-risk/domain identity,
  and the exact required allowance-spender set before readiness;
- missing, ambiguous, or drifted metadata invalidates readiness and triggers
  refresh/fail-closed handling;
- subscription scope is limited to configured outcome tokens;
- public trades are absent unless the Phase 0 model requirements name their
  exact use;
- public connection lifecycle, heartbeat, reconnect, and resubscribe behavior
  is venue-owned;
- book snapshots and deltas validate asset/token identity, sides, price range,
  quantity, crossed/empty state, integrity/hash information actually supplied
  by the venue, and connection epoch;
- any gap, invalid transition, overflow, disconnect, or stale threshold marks
  the book unavailable until explicit resynchronization;
- local ingress sequence never masquerades as exchange predecessor sequence;
- raw capture preserves enough information for deterministic parser/reducer
  replay; and
- no network availability is required for tests or completion.

Add pinned, hashed fixtures for:

- initial snapshots;
- valid incremental updates;
- best-bid/ask changes;
- trades if the reached quote-model input uses them;
- malformed, wrong-token, crossed, empty, stale, duplicate, reordered,
  reconnect, gap, and resync cases; and
- any price at or outside zero/one, which cannot become a live executable
  level; and
- metadata with wrong token membership, closed/inactive/not-accepting state,
  disabled order book, changed grid/minimum, neg-risk/domain mismatch, and
  changed required spender.

Exercise the actual bounded public capture path through a local fake WebSocket:

```text
fake public WS
-> raw bounded capture artifact
-> capture verification/hash
-> production parser and reducer replay
-> canonical logical projection
```

Two runs MUST produce identical capture-verification results, replay output,
logical counters, and artifact/projection hashes.

Add a combined deterministic replay containing interleaved:

- OKX public reference updates;
- Polymarket public book updates;
- disconnect/resync lifecycle; and
- timers needed only for freshness.

Two equal runs MUST produce byte-identical ordered logical projections and
identical counters. Integrity-bearing raw deltas cannot be silently coalesced.
If a reduced BBO/reference lane is latest-wins, prove that full book/reference
ownership has already consumed the required transitions and record the exact
coalescing contract.

## Phase 4: Read-Only Private Lifecycle And Position Monitor

Implement pure private-event parsing, reconciliation contracts, and canonical
state against fixtures/fakes only.

The position monitor MUST represent:

- collateral balance;
- outcome-token inventory by account/funder/token;
- allowance by account/funder, asset/token, and exact spender;
- snapshot ID/completeness, source, observation time, and freshness;
- a separately retained fill-derived provisional inventory;
- exact fill-fee state bound to account/order/fill and asset: a signed balance
  delta where a charge is negative and a rebate is positive, or typed
  Unknown/Incomplete;
- live-order collateral and token reservations;
- divergence and convergence between published and provisional state;
- resolved/unredeemed or otherwise non-tradable inventory as a distinct
  unavailable state; and
- fail-closed quote readiness with typed reasons.

Rules:

- absence means zero only when an explicit complete authoritative snapshot
  proves absence;
- incomplete, paginated-without-completion, failed, stale, or ambiguous
  snapshots never zero state;
- a fill updates provisional inventory immediately and triggers a bounded
  refresh/reconciliation request;
- duplicate fills do not move inventory twice;
- buy reservations use the exact policy-approved collateral requirement;
- sell reservations use exact available outcome-token inventory;
- reservation release requires a proven terminal order transition;
- missing or insufficient allowance fails closed for the exact required
  spender; and
- allowance lookup cannot use a first-map-value, owner/funder fallback,
  aggregate boolean, or effective scalar. Every metadata-derived required
  spender is checked independently; unknown extra entries grant no authority;
- account/address keys use one validated canonical representation before
  lookup;
- provisional collateral/inventory cannot claim convergence while a relevant
  fill fee is Unknown/Incomplete;
- a known collateral-denominated fee/rebate updates provisional collateral; a
  known outcome-token-denominated fee/rebate updates that exact token
  inventory. A wrong/unmapped asset makes the affected readiness
  Unknown/Incomplete until authoritative reconciliation;
- reservations use the proven worst-case exact fee/collateral requirement when
  fees can affect available collateral; and
- the user order/trade stream is not treated as a balance/position stream.

Define a PM-specific exact risk contract rather than reusing the existing
spot/derivative `f64` risk model. At minimum it covers:

- maximum exact order quantity and collateral at risk;
- per-market and per-account inventory/exposure;
- live-order count and reserved collateral/token inventory;
- stale or unavailable reference/book/account/order inputs;
- market-scoped and global halt state; and
- deterministic cancel-owned behavior after a breached limit.

Reservations count against every limit before fake dispatch. A limit breach
suppresses new quotes and emits ordered cancel-owned decisions as required.

The order read model MUST:

- parse private order/fill lifecycle into exact typed events;
- retain unknown/external orders as quarantined facts rather than ignoring or
  claiming them;
- conservatively reserve an unmanaged live order's authoritative exact
  remaining collateral/token requirement, or fail account/market quote
  readiness closed when that requirement or ownership is ambiguous;
- reconcile open orders, fills, and order detail by structural identity;
- never claim ownership from an ambiguous side/price/quantity match;
- reject unknown venue statuses rather than defaulting them to Pending; and
- surface every normalization/contract violation as a typed fail-closed error
  and metric.

Fixture tests MUST cover incomplete snapshots, explicit absence, stale state,
multiple allowance entries, one missing required spender, unknown extra
spenders, mixed-case/canonical address input, independently insufficient
collateral/token allowances, fee charge, rebate, wrong/unmapped fee asset,
unknown fee, duplicate fee/fill, reconnect, duplicate fills, multiple partial
fills, out-of-order REST/WS observations, reservation pressure, risk-limit and
halt behavior, divergence, convergence, unknown orders, ambiguous ownership,
and resolved inventory.

Include both startup discovery and later appearance of an unmanaged/ambiguous
live order. It is never cancelled as owned, but it cannot allow quoting to
over-allocate collateral or token inventory.

No authenticated request, credential load, or external network call is
allowed.

## Phase 5: Passive Quote Lifecycle And Fake Execution

Implement one canonical PM order owner and a credential-free fake transport.

Required lifecycle:

```text
validated strategy candidate
-> risk/readiness approved quote
-> locally reserved quote
-> durably recorded intent/ownership
-> prepared exact PM quote
-> fake transport result
-> canonical order/fill/position reduction
```

Requirements:

- typed, non-Clone or otherwise take-once
  `Approved -> Reserved -> Prepared` mutation authority;
- account, market, token, side, exact price/quantity, profile, idempotency, and
  local ownership bound before dispatch;
- intent and ownership journaled before fake network dispatch;
- a PM-specific versioned journal/recovery schema whose lease identity binds
  product, environment, account/funder, and configured market scope;
- reuse of Reap's leased durable writer mechanics without adding PM records to
  the existing Chaos/OKX domain union;
- PM recovery rejects Chaos records and Chaos recovery rejects PM records;
- absent/failed durable acknowledgement prevents fake dispatch;
- passive `GTC`/post-only profile only;
- approval requires a fresh, validated, uncrossed PM book and proves on the
  exact grid that BUY price is strictly below best ask and SELL price is
  strictly above best bid;
- approval binds the exact metadata/grid and book revisions plus a monotonic
  expiry; preparation/fake dispatch fails closed if either revision changed or
  the approval expired;
- quote slots are structurally keyed by account/market/token/side, suppress
  duplicates, prevent crossing another owned quote, and use deterministic
  cancel-before-replace where replacement would otherwise overlap ownership;
- cancel requires canonical owned-order proof and exact venue order identity;
- ambiguous submit timeout causes reconciliation/Unknown, never blind retry;
- acknowledgement binds venue order identity and any immediate trade IDs;
- fill deduplication spans acknowledgement, private WS, REST, replay, and
  reconnect sources;
- cumulative fill is derived per owned order from authoritative cumulative
  data or durable local delta history;
- remaining quantity is checked against original size and cumulative fill;
- contract violation, overfill, backwards cumulative movement, or unresolved
  ownership fails closed;
- cancel/fill race converges to the proven terminal/fill state;
- reservations release only after a proven terminal transition; and
- quote replacement/cancel deadlines use one coordinator-owned scheduled
  action structure, not `tokio::spawn` per order.

Tests MUST cover at least:

- accepted resting quote;
- immediate full fill;
- multiple partial fills;
- one trade containing multiple maker legs;
- duplicate and out-of-order fill delivery;
- acknowledgement trade IDs followed by WS/REST duplicates;
- rejection;
- timeout before acknowledgement;
- acknowledgement after timeout;
- ambiguous remote open order;
- cancel accepted, rejected, already filled, and cancel/fill race;
- disconnect/reconnect and REST/WS convergence;
- journal replay and take-once authority; and
- process/reducer restart from a complete fixture state;
- durable-write and durable-ack failure before dispatch;
- journal corruption, truncation, duplicate replay, and lease/scope mismatch;
- storage and fake-effect queue saturation; and
- stale/crossed/locked book, duplicate quote, cancel-before-replace, and
  replace/cancel/fill races.

Do not copy the reference behavior that assigns each individual REST trade
`cumulative_filled = size` and `remaining = 0`. A validation failure cannot be
reduced to an ignored `false` while reconciliation continues.

Only the in-process fake transport may consume `PreparedPmQuote` or
`PreparedPmCancel`. Source-policy and compile-fail tests MUST prove that no
live signer, authenticated HTTP/WS session, arbitrary request executor, or
mutation constructor is reachable from the PM composition.

## Phase 6: PM Coordinator, Quote-Model Seam, And Local Architecture Evidence

Compose the sibling PM product without changing the existing Chaos product.

The coordinator owns:

- configured OKX public reference state and freshness;
- configured PM book and market lifecycle state;
- PM order lifecycle and ownership;
- PM published and provisional position state;
- reservations and fail-closed readiness/risk;
- the statically supplied pure quote model;
- deterministic timers/scheduled actions; and
- ordered fake quote/cancel effects and storage records.

Define a typed product input boundary containing only reached:

- OKX public reference updates;
- PM public book/market lifecycle;
- PM private order/fill lifecycle;
- PM read-only snapshots/reconciliation results;
- timers/scheduled actions;
- persistence acknowledgements; and
- safety/control events with no hidden mutation authority.

Define typed product effects containing only:

- fake passive PM quote;
- fake cancel-owned PM quote;
- bounded reconciliation/refresh request;
- durable record;
- health/metric/audit update; and
- fail-closed halt/cancel intent.

Use a statically supplied deterministic fixture quote model to prove the
architecture, not economics. Tests MUST demonstrate:

- an OKX public reference update can cause quote evaluation;
- PM book state constrains passive quote admissibility;
- executable PM prices remain exact, tick-aligned, and strictly `(0, 1)`;
- stale/unready OKX reference or PM book suppresses new quotes and emits
  deterministic cancel-owned decisions when required;
- positions, exact allowances, and reservations constrain each side;
- fills update provisional exposure before the next decision;
- recovery/reconciliation changes readiness deterministically;
- the product has no OKX private/account/order handle;
- equal replays produce identical ordered decisions, effects, records, and
  counters; and
- all queue capacities, high-water marks, saturation decisions, and event ages
  are observable without per-event formatted logging or dynamic label
  registration.

Add a production-shaped local benchmark/replay target that uses the real
normalizers, reducers, coordinator, quote-model seam, risk/readiness, quote
policy, exact preparation, storage-record construction, and fake transport
boundary.

Run the stable targets declared in Phase 0:

```bash
cargo test -p reap-pm-live --test combined_replay --locked
cargo bench -p reap-pm-live --bench pm_action_path --locked
```

The local measurement contract MUST record:

- included and excluded work;
- exact event counts and input mix;
- queue capacities and overflow policy;
- deterministic action/record counters;
- allocation calls and requested bytes on the normalized-event-to-effect path;
- queue high-water, saturation, age, and drops;
- throughput plus p50/p95/p99/p99.9/max where the existing harness supports
  them;
- one warm-up and three recorded same-host runs;
- timer-read overhead; and
- host/toolchain identity sufficient for local comparison.

The PM gate is green only when:

- repeated equal replay has exact expected input, decision, effect, record,
  fill, reservation, and readiness counters;
- the nominal workload has zero drops, zero saturation, and satisfies every
  Phase 0 allocation, memory, queue-age, and high-water budget;
- the overload workload reaches the exact declared fail-closed/resync and
  private-lifecycle recovery behavior;
- repeated replay/recovery does not grow canonical map, identity, queue,
  journal-replay, or memory cardinality beyond configured bounds;
- fake-effect and storage saturation prevent dispatch or fail closed exactly
  as declared; and
- two logical replay projections and their hashes are byte-identical.

This is regression evidence only. It is not a latency SLO, capacity
certification, target-host result, or claim that Tokio/channel/runtime choices
are optimal.

After Phase 6 and at final completion, rerun one warm-up and three recorded
same-host runs of the existing Chaos benchmarks:

```bash
cargo bench -p reap-engine --bench event_loop --locked
cargo bench -p reap-live --bench live_loop --locked
cargo bench -p reap-live --bench action_path --locked
```

Require:

- identical existing logical/action/allocation counters;
- no new allocation from metrics/health on existing per-event paths;
- investigation when a three-run same-host median or supported tail percentile
  regresses by more than 5%;
- after eliminating host noise and repeating with five warm recorded runs
  where necessary, no unexplained median or supported tail regression greater
  than 10%; and
- no performance optimization mixed into a semantic or architecture commit.

Goal F MUST NOT add CPU affinity, thread pinning, isolated cores, scheduler
policy, busy spinning, SPSC/ring buffers, a custom allocator, a custom Tokio
runtime, `io_uring`, hardware timestamping, kernel bypass, or unsafe lock-free
state. Those remain future profile-driven decisions.

## Phase 7: Documentation, Global Verification, And Handoff

Update at minimum:

- `docs/architecture.md` with the multi-venue substrate versus concrete product
  boundary, PM ownership/event flow, and absence of a universal adapter;
- `docs/performance.md` with only the local structural/regression evidence and
  explicit target-host exclusions;
- `docs/trading-readiness.md` with the PM credentialed/live/order-model,
  settlement, operational, and approval gaps;
- the Goal F handoff with all phase commits, commands, results, hashes,
  dependency graphs, source provenance, file/function sizes, benchmark
  evidence, limitations, and deferrals; and
- the secret-free PM connectivity/product boundary created in Phase 0.

Run focused package gates after each phase. At completion run at minimum:

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

Rerun the stable PM replay/benchmark targets, then run the canonical Chaos
backtest twice and require byte equality and the exact expected hash:

```bash
cargo test -p reap-pm-live --test combined_replay --locked
cargo bench -p reap-pm-live --bench pm_action_path --locked
cargo run --locked -q -p reap-cli -- \
  backtest \
  --format normalized-jsonl \
  --config examples/iarb2-basic.toml \
  --data fixtures/normalized/chaos_quote_hedge.jsonl \
  --pretty >target/tmp/goal-f-backtest-1.json
cargo run --locked -q -p reap-cli -- \
  backtest \
  --format normalized-jsonl \
  --config examples/iarb2-basic.toml \
  --data fixtures/normalized/chaos_quote_hedge.jsonl \
  --pretty >target/tmp/goal-f-backtest-2.json
cmp target/tmp/goal-f-backtest-1.json target/tmp/goal-f-backtest-2.json
sha256sum target/tmp/goal-f-backtest-1.json
```

Also require:

- the canonical Chaos backtest output remains
  `38acf9f5e0c310f2ec5528974beffadf4c1a7f84d46efa8d9664ee7051e84691`;
- all Goal D deterministic decision/risk replay projections remain
  byte-identical;
- existing Chaos/Java parity, exact-order, source-policy, authority,
  compile-fail, dependency, schema, capture/replay, storage, and emergency
  suites remain green;
- two equal PM combined replays are byte-identical;
- PM fixture provenance is pinned to exact tracked source bytes or independently
  authored golden behavior;
- no Cargo dependency path points outside Reap;
- `reap-live`, `reap-live-contracts`, and `reap-order` have no PM DTO,
  configuration, execution, or product dependency. Any shared-identity match
  only rejects unsupported PM input;
- the PM product has no OKX private/order capability;
- no PM live mutation, signer, credential, authenticated client, or arbitrary
  request executor is reachable;
- no broad adapter or command union can bypass role-specific capability
  planning;
- all queues are bounded and every saturation path is tested;
- before/after public exports, dependencies, schemas, largest
  files/functions/state aggregates, deterministic hashes, and benchmark
  results are recorded; and
- Reap and `../imm-strategy` are clean while the pre-existing `../predarb`
  dirty paths remain unmodified and explicitly recorded.

Commit the final documentation/handoff only after the global gate is green.
Do not push without separate user instruction.

## Stop Conditions

Stop and report the exact conflict when:

- Reap or `../imm-strategy` has unexplained or overlapping changes;
- the pinned `../imm-strategy` or tracked `../predarb` Git object is
  unavailable;
- progress would require modifying, cleaning, deleting, or interpreting
  untracked runtime state in either sibling repository;
- PM condition/market/token/account identity, tick, lot, protocol integer
  units,
  snapshot completeness, spender allowance, order status, or fill semantics
  cannot be proven;
- an executable PM order price can escape strict `(0, 1)`, bypass exact tick
  validation, or round-trip through `f64`;
- a production probability formula would need to be invented or inferred from
  Predarb;
- completion requires credentials, private key access, API-key derivation,
  authenticated exchange access, a signed live request, or live/demo submit;
- PM integration requires widening `OkxOrderGateway`, `ChaosConnectivityPlan`,
  existing `LiveRuntime`, or a broad arbitrary command/request surface;
- the PM product can reach an OKX private/account/order role;
- a sequence/integrity-bearing event or private order/fill event must be
  silently dropped or coalesced to pass bounded-queue tests;
- canonical PM state would require shared mutation, a second shadow owner,
  `Arc<Mutex<_>>`, `RwLock`, task-per-order scheduling, or new hot-path dynamic
  dispatch;
- existing Chaos decisions, RNG/floating-point order, timers, risk, authority,
  fixture/canonical hashes, encodings, fingerprints, semantics, or serialized
  bytes change. New PM schemas or a shared envelope addition are allowed only
  under the separately documented versioned compatibility contract;
- an ambiguous open order must be claimed/cancelled or an unknown status must
  be treated as normal to continue;
- a fill normalization/reconciliation violation would be ignored rather than
  fail closed;
- a source-policy/compile-fail test passes only by broadly exporting raw
  wire/auth types or widening an allowlist;
- repeated equal PM replays differ in logical counters, ordered outputs, or
  bytes;
- existing benchmark logical/allocation counts change or repeated same-host
  results exceed the regression gate without explained host noise;
- a new source monolith violates the size/function limits without an approved
  responsibility-based exception; or
- completion would require target-host evidence, CPU placement, production
  approval, settlement/redemption, or external operational coordination.

Do not weaken an invariant, silently broaden scope, discard a valid bad result,
or rewrite a negative test merely to finish. Record the blocker and propose
the next smallest separately scoped goal.

## Explicit Exclusions

Goal F does not authorize:

- Predict.fun connectivity, types, fixtures, strategy, or market discovery;
- exact quote mirroring or arbitrage between prediction venues;
- the Predarb PF-maker/PM-hedge or inverse PM-maker/PF-hedge strategy;
- PM hedge submission, FOK, IOC/FAK, synthetic-IOC cancellation, reduce-only,
  market orders, amend/batch, or account-wide cancel;
- OKX private feeds, account reads, reconciliation, order placement, or cancel
  in the PM product;
- production fair-probability or quoting economics;
- real PM credentials, signing sessions, authenticated network requests,
  demo/live submit, or a credentialed smoke;
- API-key creation/derivation in normal runtime, allowance mutation, token
  approval, transfers, bridge operations, market administration, redemption,
  settlement, or withdrawal;
- arbitrary Gamma/Data/CLOB endpoint exposure beyond the exact fixture-backed
  read contracts;
- a generic venue plugin system, universal exchange adapter, arbitrary venue
  command enum, or dynamic hot-path dispatch;
- wholesale workspace conversion from `f64` to decimal/fixed point;
- importing Predarb application/runtime monoliths or depending on the sibling
  checkout;
- target-host deployment, CPU binding, thread-per-core, SPSC/ring buffers,
  custom allocator/runtime, busy spin, `io_uring`, kernel bypass, or other
  latency-architecture work;
- modifying `../predarb` or `../imm-strategy`;
- changing production order-entry authorization; or
- claiming production readiness, economic validity, profitability,
  target-host latency, or trading approval.

## Completion

Goal F is complete only when:

- every phase and focused/global gate is green;
- Polymarket is represented as a first-class statically composed venue through
  the same bounded supervision, capture/replay, health, persistence, and
  least-authority framework mechanics used by Reap;
- venue-specific PM identity, exact numeric, lifecycle, integrity, allowance,
  position, and order semantics remain explicit rather than erased;
- OKX public reference state and PM public/private/read-only state drive one
  deterministic sibling PM coordinator;
- a pure, statically supplied fixture quote model proves the
  OKX-reference-to-PM-passive-quote event path without inventing production
  economics;
- passive PM quote/cancel ownership is proven end to end through exact policy,
  take-once authority, durable intent, fake transport, order/fill reduction,
  reservation, and provisional position convergence;
- partial, multiple, immediate, duplicate, out-of-order, ambiguous, reconnect,
  and cancel/fill cases fail closed or converge deterministically;
- all PM mutation remains fake-only and real credentials/live submission are
  unconstructible from the product composition;
- one canonical writer, bounded queues, deterministic ordering, modular source
  size, and local regression evidence are recorded;
- existing Chaos/OKX behavior, authority, canonical outputs, dependencies, and
  performance gates remain green;
- documentation and the handoff state the exact structural result and every
  remaining live/economic/operational exclusion; and
- Reap and `../imm-strategy` are clean and the pre-existing `../predarb` dirty
  state is unchanged.

Completion means Reap has a sound, performance-conscious architecture for a
second venue and an offline-proven PM quote path. It does not mean the PM
pricing model is economically valid, authenticated PM connectivity has been
exercised, live order placement is enabled, the target runtime has been
qualified, or the repository is production ready.
