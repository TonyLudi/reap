# Polymarket Product And Connectivity Boundary

Status: normative for Goal F. This is a credential-free architecture and fake-
execution contract, not a production model, authenticated connectivity plan,
deployment specification, or trading authorization.

The governing rule is:

> OKX contributes only declared public reference observations. Polymarket
> contributes the configured market, account, and order state and is the only
> venue represented by the fake mutation path. A capability exists only when a
> named consumer in this product requires it.

This document is governed by the
[Goal F execution prompt](multi-venue-polymarket-foundation-goal-f-prompt.md).
The existing [Chaos connectivity boundary](chaos-connectivity-boundary.md)
continues to govern the separate Chaos/OKX product and is not broadened here.

## Product Topology

```text
configured OKX public reference observations
                         |
                         v
              validated reference state
                         +
configured Polymarket metadata + exact book
                         +
fixture/fake PM order/fill + complete account/position snapshots
                         |
                         v
             explicit pure quote-model type
                         |
                         v
        checked side-aware exact-price conversion
                         |
                         v
  PM readiness + risk + ownership + duplicate/slot policy
                         |
                         v
 durable PM journal acknowledgement before fake place/cancel
```

Polymarket is treated as one first-class venue, not as a special hedge attached
to another prediction venue. This product does not connect to Predict.fun,
mirror another prediction-market quote, execute on OKX, or assume that a crypto
price is itself a probability. The quote model owns the transformation from
reference and PM state to fair probability.

Goal F supplies no production probability model. Tests, replay, examples, and
the local benchmark may supply a deterministic fixture model. A release-shaped
composition has no default model: without an explicit model type it constructs
no private or mutation role.

### Exact Goal F fixture-model reach

The only concrete model used to prove Goal F composition is
`GoalFFixtureQuoteModel`. Its requirement set is frozen:

| Input | Exact fixture requirement |
| --- | --- |
| OKX reference | Public websocket `index-tickers`, configured index instrument `BTC-USDT`, exact positive decimal `idxPx`, connection epoch, venue timestamp and local receive times |
| PM market | One configured condition/CLOB-market/outcome-token membership plus the full metadata/readiness contract below |
| PM book | Public `GET /book?token_id=...` seed/resync and `/ws/market` for only that token; reached event kinds are `book`, `price_change`, `best_bid_ask`, and `tick_size_change` |
| Time | Coordinator-owned quote-evaluation and freshness timers |

The fixture model declares no OKX book, ticker-last, trade, mark, funding,
private, account, order, or reconciliation input. It declares no PM public
trade/`last_trade_price` input, so Goal F constructs and tests no PM public
trade role.

To prove that OKX value—not merely event arrival—crosses the model seam, the
test-only model returns fair probability `0.60` when exact `idxPx >=
50_000.00`, otherwise `0.40`, and returns no candidate unless current PM
metadata/book inputs are ready. This threshold is deliberately fixture-only,
has no configurable production default, and carries no economic or trading
claim. Quote policy, rather than the model, converts that fair value to an
exact passive PM candidate.

## Configured Identity Model

Raw strings are admitted only while parsing configuration or wire input. Before
coordinator ingress, every configured identity is validated and resolved to a
bounded compact handle. Runtime state is indexed by those handles and cannot
register new instruments, accounts, spenders, or metric labels.

The structural identities are:

| Identity | Required structure | Runtime rule |
| --- | --- | --- |
| Venue | `Okx` or `Polymarket` | Common source identity; old OKX encoding is unchanged |
| OKX reference instrument | venue, instrument kind, canonical OKX instrument ID | Resolves to `OkxReferenceHandle`; public roles only |
| PM condition | canonical 32-byte condition identity | Label/question text is never identity |
| PM CLOB market | distinct typed market identity plus its condition mapping | Equality with a condition ID, when true, is explicit metadata rather than type aliasing |
| PM outcome token | unsigned 256-bit token ID | Resolves to `PmTokenHandle`; outcome label is metadata only |
| PM instrument | PM market identity plus outcome-token identity | The token must be an authenticated member of the configured market snapshot |
| PM account scope | environment, chain, non-secret signer identity, funder identity, and account handle | No key, credential, session, or signer implementation is carried |
| Allowance spender | chain, exact spender address/domain, asset class, and account scope | One state entry per required spender; never “first map value” |
| PM client order | account scope plus fixed-width locally generated ID | Identity is bound to the approved canonical order fields |
| PM venue order | account scope plus exact venue order/hash ID | Remote IDs do not establish local ownership |
| PM fill/trade | account scope plus exact venue fill/trade ID | Deduplicated independently of order cumulative progress |
| Connection/snapshot | source, connection epoch, snapshot revision, and local ingress sequence | Epoch/revision changes invalidate dependent readiness |
| Reference mapping | one PM market handle plus a fixed bounded list of OKX reference handles | Declared by the explicit model requirements |

The configured maxima for Goal F are:

| Item | Maximum |
| --- | ---: |
| OKX reference instruments | 16 |
| PM markets | 32 |
| PM outcome tokens | 64 |
| PM account scopes | 4 |
| Required allowance spenders per configured market/account scope | 8 |
| Exact book levels per PM token | 2,048 |
| Live plus unresolved/ambiguous orders per account | 1,024 |
| Retained recent fill IDs per account | 8,192 |
| Scheduled actions | 4,096 |
| Outstanding raw frame bytes across ingress/capture | 32 MiB |
| Individual raw frame | 1 MiB |

Evicting a fill ID is legal only behind an authoritative reconciled watermark
that proves the fill cannot be replayed as new. Exhaustion without that proof
halts the affected account and requires complete reconciliation. No bound may
silently turn into a growing map.

## Exact PM Numeric Contract

Existing Chaos `Price`, `Quantity`, and `Symbol` remain unchanged and are not
used by PM executable state.

PM executable numerics use canonical fixed units:

- `PmPrice` is an exact unsigned probability in millionths. Its executable
  range is `1..=999_999`, corresponding to strict `(0, 1)`. It is heap-free,
  totally ordered, hashable, and serialized in canonical integral units.
- `PmQuantity` is a positive exact share amount in protocol micro-units backed
  by a heap-free, totally ordered, hashable fixed-width unsigned 256-bit
  representation.
- collateral, balance, ERC-20 collateral allowance, fills, cumulative fill,
  remaining quantity, positions, and reservations use fixed-width unsigned
  256-bit integral protocol units; signed fee/balance deltas add an explicit
  sign. ERC-1155 operator approval is a tagged boolean, never a numeric amount.
- the frozen CLOB V2 limit-order lot is `0.01` shares, or `10_000` protocol
  micro-units. Market minimum order size is a separate exact metadata value;
  an executable quantity must be lot-aligned and at least that minimum.
- market tick and minimum metadata are exact integral units. Goal F admits the
  proven CLOB V2 tick set `0.1`, `0.01`, `0.005`, `0.0025`, `0.001`, and
  `0.0001` (100,000, 10,000, 5,000, 2,500, 1,000, and 100 price units). The
  configured token's current metadata selects one member; an unknown tick
  fails configuration/readiness rather than extending the profile implicitly.
- `0`, `1`, negative values, overflow, underflow, off-grid values,
  sub-protocol-unit values, and values requiring unapproved rounding cannot
  become executable prices or quantities.

Decimal text is parsed exactly. `0.1` and `0.10` canonicalize to the same
units, hash, idempotency identity, and serialized value. A book-level deletion
uses an explicit delete representation; it does not mint a zero executable
quantity.

For an approved order, the side-specific maker and taker amounts must both be
positive integral protocol units:

| Side | Maker amount | Taker amount |
| --- | --- | --- |
| Buy | exact collateral units | exact outcome-share units |
| Sell | exact outcome-share units | exact collateral units |

If `quantity × price` is not exactly representable in the required protocol
units, approval rejects it. Wire lowering validates and formats already exact
integral values; it never rounds. With at most four price decimals and two
quantity decimals, a valid frozen-profile limit order is representable at six
protocol decimals. Divisibility and overflow remain checked defensive
invariants below the outer lot/grid approval boundary.

`PmOrderSalt` is checked in `0..=9_007_199_254_740_991` (`2^53 - 1`).
Although its signed type is `uint256`, the CLOB JSON field is a number and Goal
F rejects values outside the JavaScript safe-integer range. Signed-order
`timestamp_ms` is a Unix wall-clock millisecond value. It is never reused as,
or derived from, the coordinator's monotonic approval expiry.

A fixture quote model may return a finite `f64` fair probability because model
arithmetic and executable identity are deliberately separate. The quote-policy
boundary performs exactly one checked conversion:

- buy candidates round toward zero onto the declared tick grid;
- sell candidates round toward one onto the declared tick grid;
- conversion rejects non-finite and out-of-range input;
- the resulting exact candidate is checked again for strict `(0, 1)`, grid,
  integral maker/taker units, risk, and passivity.

No PM book, order, fill, balance, allowance, reservation, or position value
round-trips through `f64`. Binary complement arithmetic (`1 - p`) is not a
venue rule and is available only to a model-specific helper after metadata
proves complementary outcomes.

## Market Metadata And Readiness

A PM token is quote-ready only when one atomic, versioned metadata snapshot
proves all of the following:

1. the configured condition, CLOB market, and outcome token are mutually
   consistent and the token is a member of that market;
2. the configured outcome label is metadata attached to that token, not an
   inferred identity;
3. the market is active, not closed, not archived, accepting orders, and has
   its order book enabled;
4. tick size, minimum order size, quantity/lot units, collateral units, and
   wire units are present, exact, supported, and internally consistent;
5. negative-risk status and the applicable non-secret protocol/domain identity
   are present and consistent;
6. the complete required allowance-spender set is derived for that exact
   chain/domain/account/asset combination;
7. book, private, account, reconciliation, and clock freshness dependencies
   name compatible market/account revisions; and
8. no unknown metadata field or lifecycle status was normalized into a
   ready state.

Any missing, partial, stale, contradictory, or changed dependency fails closed.
A tick, minimum, membership, neg-risk/domain, or spender-set revision
invalidates all approvals bound to the prior revision, suppresses new quotes,
and schedules deterministic cancellation of proven owned quotes.

The public book additionally must be validated, fresh, non-crossed, tied to the
current connection epoch and metadata revision, and free of an unresolved
snapshot/delta gap. PM messages are not assigned a synthetic venue predecessor
sequence. A venue-supplied book hash is retained and validated according to
that channel's real contract; local ingress sequence is not misrepresented as
a venue sequence.

## Capability Matrix

The roles below are separate concrete capabilities. They are not methods on one
exchange client and cannot be recovered from a common adapter escape hatch.

| Role | OKX | Polymarket | Goal F transport | Mutation |
| --- | --- | --- | --- | --- |
| Public observation | Declared reference instruments/channels only | Declared metadata, snapshot, and book channels; trades only if the model declares them | Public parser/capture/replay; public network edge may be credential-free | None |
| Private lifecycle observation | None | Order, fill/trade, session/epoch events | Fixture/fake only | None |
| Order reconciliation | None | Complete open-order snapshot, exact order detail, fills since watermark | Fixture/fake only | None |
| Account/position snapshot | None | Collateral, token inventory, every required spender allowance, complete position revision | Fixture/fake only | None |
| Owned passive execution | None | One GTC post-only place profile and cancel by proven local ownership | In-process fake only | Fake place/cancel |

The typed product plan is the exact union of three sources:

1. public/time/model inputs declared by the explicit quote-model type;
2. the fixed passive quote and owned-cancel fake execution profile; and
3. mandatory safety, private lifecycle, reconciliation, account, allowance,
   position, persistence, and readiness dependencies implied by that profile.

The reached connectivity inventory has exactly 16 stable purpose IDs. Fifteen
produce one plan entry each. `PM-ACCOUNT-ALLOWANCE` produces one independently
scoped plan entry for each of the configured required spenders, so with
`N = 1..=8` spenders there are `15 + N` connectivity entries. Every resulting
`(stable ID, exact scope)` requirement has one consumer, one constructed role
binding derived from its concrete connectivity role, one queue lane, one
readiness dependency, and, where applicable, one exact source/connection
route. The six reached concrete connectivity role kinds may each emit several
purpose-specific bindings.

The model-declared quote-evaluation timer adds one internal plan entry. It is
owned by the product quote schedule, maps to the scheduled lane, and has
neither a connectivity role nor a source/connection route. A product plan
therefore has exactly `16 + N` entries. The plan-to-construction bijection
covers every scoped binding/owner without pretending the internal timer is an
exchange endpoint. Extra configuration, plan entries, roles, channels,
instruments, accounts, or spenders are errors. Model requirements cannot
request a private read or mint mutation authority.

Three composition roots remain independent:

- `PmPublicCapture` constructs only the exact OKX/PM public roles, raw capture,
  verification, and replay. It takes no model, private, account, reconciliation,
  journal, or mutation role.
- `PmReadOnlyMonitor` is an explicitly requested least-authority fixture/fake
  composition of PM private lifecycle, reconciliation, collateral,
  authorization, and position roles. It takes no quote model, OKX source,
  journal, or mutation role. Goal F provides no authenticated implementation.
- `PmProduct<Model>` constructs the full deterministic fake-quote product only
  when the explicit model is supplied. With no model, normal product
  construction creates no private or mutation role.

### Reached endpoint/channel classifications

Names here classify protocol purposes; they do not authorize authenticated
network construction in Goal F.

| Stable requirement | Source/shape | Consumer | Goal F mode |
| --- | --- | --- | --- |
| `PM-OKX-REF` | OKX `index-tickers` for configured index `BTC-USDT`; exact `idxPx` and source times/epoch | Fixture quote-model input reducer | Public observe/capture/replay |
| `PM-META-LIFECYCLE` | Public `GET /markets/{condition_id}` for only the configured condition: market/token lifecycle and membership | Metadata readiness | Public fixture/REST parser |
| `PM-META-CLOB` | Public `GET /clob-markets/{condition_id}` for only the configured condition: token, tick, minimum, units, neg-risk/domain | Metadata readiness | Public fixture/REST parser |
| `PM-MD-BOOK-SNAPSHOT` | Exact configured token snapshot, hash/revision, levels | PM book integrity | Public observe/capture/replay |
| `PM-MD-BOOK-DELTA` | Exact configured token changes with real venue integrity fields | PM book integrity | Public observe/capture/replay |
| `PM-MD-TRADE` | Public trade/`last_trade_price` | No Goal F consumer | Absent; no plan entry, role, parser reach, or subscription |
| `PM-PRIVATE-ORDER` | User order lifecycle and connection epoch | Order reducer/readiness | Fixture/fake |
| `PM-PRIVATE-FILL` | Individual trade/fill identity and exact delta | Fill/order/position reducers | Fixture/fake |
| `PM-RECON-OPEN` | Atomic complete open-order snapshot | Ownership/reservation convergence | Fixture/fake |
| `PM-RECON-ORDER` | Exact order detail by known ID | Ambiguity repair | Fixture/fake |
| `PM-RECON-FILLS` | Complete bounded fill page with a full-account-scoped opaque fixture watermark; no cursor/last-fill ordering is invented | Dedup and cumulative convergence | Fixture/fake |
| `PM-ACCOUNT-COLLATERAL` | Complete exact collateral balance/revision | Buy readiness/risk | Fixture/fake |
| `PM-ACCOUNT-TOKEN` | Complete exact outcome-token inventory/revision | Sell readiness/risk | Fixture/fake |
| `PM-ACCOUNT-ALLOWANCE` | Exact allowance per required spender and asset | Entry readiness | Fixture/fake |
| `PM-POSITION-SNAPSHOT` | Atomic complete position snapshot | Position convergence/risk | Fixture/fake |
| `PM-FAKE-PLACE-GTC-PO` | Prepared owned passive quote | Fake effect worker | In-process fake only |
| `PM-FAKE-CANCEL-OWNED` | Prepared cancel carrying local ownership proof | Fake effect worker | In-process fake only |

There is no PM cancel-all, cancel-market, arbitrary raw request, arbitrary
order-type selector, authenticated session factory, signer, API-key builder,
private-key type, or generic HTTP client in this composition.

## Frozen Fake Execution Profile

The only place profile in Goal F is:

- one configured PM outcome token and exact account scope;
- `GTC`;
- `postOnly = true`;
- `deferExec = false`;
- the unsigned fake account profile is EOA-only: `signatureType = 0` and
  `maker == signer == configured funder`;
- `expiration = 0`, `metadata = bytes32(0)`, and `builder = bytes32(0)`;
- an exact tick-aligned price strictly in `(0, 1)`;
- a positive exact minimum/lot-aligned quantity whose side-specific maker and
  taker amounts are integral protocol units;
- buy and/or sell only as admitted by the explicit quote policy, exact
  inventory/allowance/reservation state, and PM-specific risk limits; and
- cancellation only for an exact venue order that carries canonical local
  ownership proof.

The official Polymarket order contract defines GTC as a resting limit-order
type and post-only as reject-on-crossing for GTC/GTD. The order request example
and the pinned Predarb V2 source agree on the unsigned field family used by the
Goal F fake wire representation:

`salt`, `maker`, `signer`, `tokenId`, `makerAmount`, `takerAmount`, `side`,
`signatureType`, millisecond `timestamp`, `metadata`, and `builder`;
`expiration` remains a POST body field but is not part of the CLOB V2 signed
struct. The outer request adds `owner`, `orderType`, `postOnly`, and
`deferExec`.

Goal F models canonical unsigned fields only. It deliberately implements no
signature, EIP-712 domain hashing, key access, authentication header, signed
request, or live body dispatch. The later authenticated-execution goal must
revalidate the protocol revision before introducing any of those.

CLOB V2 exposes no client-order-ID field in this reached order request. The PM
client ID is a journal-local ownership/idempotency identity and must not be
invented as a wire field. A successful fake acknowledgement binds its returned
venue order ID to that local proof.

The authenticated outer API `owner` value is not part of the fake unsigned
DTO. Fake effects bind a compact local account handle instead. Proxy, Safe,
deposit-wallet, nonzero builder/metadata, or another signature profile is
deferred to the authenticated-execution goal.

Before fake dispatch, approval is bound to the account, funder, market/token,
side, exact units, metadata/grid revision, quote profile, model input revision,
book revision, readiness revision, and monotonic expiry. Authority is
take-once. Reservation and the durable PM journal acknowledgement occur in the
same deterministic workflow before the fake effect can consume it.

For a valid non-crossed book:

- a buy quote must be strictly below the current best ask;
- a sell quote must be strictly above the current best bid;
- an empty opposite side fails quote approval closed because strict passivity
  cannot be proved against a present best price;
- one configured quote slot per account/token/side prevents duplicate live
  quotes;
- identical canonical candidates are suppressed; and
- a changed candidate schedules cancel-before-replace. Replacement cannot
  reuse the old slot until cancellation or complete reconciliation proves it
  free.

## Order, Fill, Position, And Fee Convergence

The coordinator maintains two distinct facts:

1. authoritative state from atomic complete snapshots; and
2. a fill-derived provisional ledger between complete snapshots.

An individual fill contributes its own positive exact delta once, keyed by fill
identity. It is never interpreted as the order's cumulative filled quantity.
Order cumulative progress is monotonic and bounded by original quantity.
Duplicate, out-of-order, websocket-only, REST-only, terminal-reconcile, and
journal-replay paths must converge to the same state. A regression,
overfill, unknown terminal status, wrong account/token, or inconsistent
cumulative value fails closed and forces reconciliation.

Unmanaged remote orders are never claimed as local. A complete snapshot must
either reserve their exact remaining amount conservatively or make the
affected account/token unready. Ambiguous remote order identity is never
cancelled through the normal owned-cancel path.

Allowance is retained as:

```text
(account scope, exact chain/asset contract, exact required spender,
 snapshot revision) -> tagged authorization value
```

The tagged values are:

```text
CollateralAuthorization::Erc20Allowance(PmCollateralUnits)
OutcomeAuthorization::Erc1155OperatorApproval(bool)
```

For a buy, every required pUSD spender entry must contain a numeric ERC-20
allowance at least as large as the proven collateral plus reservation/fee
requirement. For a sell, the exact exchange operator must have ERC-1155
`isApprovedForAll == true`; an arbitrary numeric value cannot stand in for
that boolean. Missing, partial, stale, wrong-kind, false, insufficient,
duplicate/conflicting canonical-key, or contradictory values make the
affected readiness unknown.

Unknown extra map entries are retained for bounded telemetry but grant no
authority and cannot satisfy a missing required spender. They do not by
themselves invalidate a complete exact required-spender set.

For the Polygon CLOB V2 domain retrieved on 2026-07-20, the exact Goal F
asset/domain/spender contract is:

| Item | Exact identity |
| --- | --- |
| Chain | Polygon `137` |
| CLOB protocol | `ClobV2` |
| Non-secret domain name/version | `Polymarket CTF Exchange` / `2` |
| pUSD collateral contract | `0xC011a7E12a19f7B1f670d46F03B03f3342E82DFB` |
| Conditional Tokens contract | `0x4D97DCd97eC945f40cF65F87097ACe5EA0476045` |
| Standard verifying/exchange/spender | `0xE111180000d2663C0091e4f400237545B87B996B` |
| Negative-risk verifying/exchange/spender | `0xe2222d279d744050d28e00520010520000310F59` |

The required exchange applies to both the numeric pUSD buy authorization and
the boolean CTF sell authorization for its market domain. Condition and
outcome-token identities are scoped under chain 137 and the exact CTF contract.
A V1/V2, chain, collateral, CTF, or standard/negative-risk domain mismatch
invalidates metadata/readiness. Goal F carries this non-secret identity but
does not construct or hash an EIP-712 domain.

The old Neg Risk Adapter
`0xd91E80cF2E7be2e162c6513ceD06f1dD0dA35296` is explicitly not a Goal F
CLOB V2 trading spender: the current official registry labels it CLOB V1 and
deprecated. CTF collateral adapters used for split/merge/redeem are also not
order-entry spenders and those operations are outside scope. A later
authenticated goal must revalidate this entire table against a newly pinned
protocol snapshot rather than inheriting it indefinitely.

Fee accounting uses a signed exact balance delta tied to fill identity,
account, asset, and fee convention:

- a charge is negative;
- a rebate is positive;
- zero is explicit;
- unknown sign, wrong asset/account, missing fee identity, or an unsupported
  convention makes affected position/balance readiness unknown.

Goal F does not infer a fee from gross/notional arithmetic when the represented
event does not prove it.

## PM-Specific Readiness And Risk

The PM risk gate is separate from the existing Chaos risk gate and uses exact
units. Configuration must set finite explicit limits for at least:

- maximum exact order quantity and collateral notional;
- maximum aggregate open buy reservation per account/token and account;
- maximum aggregate open sell reservation per account/token;
- maximum absolute token inventory and provisional position drift;
- maximum live owned quotes per account/token/side (frozen to one quote slot in
  Goal F);
- maximum unresolved/ambiguous orders and fills;
- metadata, book, OKX reference, private, account, reconciliation, persistence,
  and effect-result freshness; and
- maximum scheduled-action count and lateness.

There are no permissive defaults. Missing limits prevent construction.
Readiness is a typed conjunction whose component reason and revision are
observable; it is not one boolean that erases which dependency failed.

## Canonical Owner And Event Ordering

One `PmCoordinator<Model>` owns by value:

- compact identity/config tables;
- exact PM books and OKX reference state;
- private order/fill and local-ownership state;
- authoritative snapshots and provisional position ledger;
- allowances, reservations, readiness, and PM risk;
- the explicit model instance and quote slots;
- the deterministic action schedule; and
- pending journal/effect correlations.

These responsibilities live in focused modules but are not independently
mutable services. Canonical state is not behind `Arc<Mutex<_>>` or `RwLock`.
The owner loop performs no network or file IO, blocking logging, JSON
construction/parsing, secret access, or unbounded work.

Every input carries applicable venue/event time, local wall receive time,
monotonic receive time, monotonic service time, connection epoch, real venue
sequence/hash, and local ingress sequence as separate fields. Exchange time is
data, never an elapsed-time clock. Service time measures queue age only; it
never reorders inputs.

Configuration sorts each identity class by its canonical encoded identity and
assigns compact handles as the zero-based `u16` ordinal. The configuration
fingerprint binds those ordered tables. Handle assignment never depends on
input declaration order, hash-map iteration, discovery order, or arrival.

Ready inputs use this stable service rank:

1. stop/safety/control and fake effect results;
2. persistence acknowledgements required to release or reject an effect;
3. PM private fill/order lifecycle;
4. due scheduled quote/cancel/freshness actions;
5. integrity-bearing PM/OKX public state;
6. complete reconciliation/account/position snapshots;
7. telemetry.

Within an input rank, ordering is `(monotonic_receive_ns, source_handle,
connection_epoch, local_ingress_sequence, variant_rank)`. Scheduled work uses
`(monotonic_deadline_ns, action_variant_rank, account_handle, token_handle,
side_rank, local_action_sequence)`. Replay uses the captured receive/deadline
values and never depends on service time, Tokio selection, hash-map traversal,
task completion race, or wall time.

The generic bounded lane container is not a connectivity authorizer. Phase 3
public producers and Phase 4 private producers must bind each delivery to the
configured role-issued source/connection route before enqueue. A caller-stamped
source or `PmIngressOrder` alone is not route proof. The Phase 2 seam preserves
the typed connection and ordering facts but deliberately does not make lanes
own the capability plan.

Stable variant ranks are:

| Lane | Variant order |
| --- | --- |
| Critical | shutdown/global stop; market/account halt; fake cancel result; fake place result |
| Persistence | durable failure; durable success |
| Private | connection unavailable; fill/trade; order lifecycle |
| Scheduled | cancel-owned; reconciliation/refresh; freshness; quote evaluation |
| Public | connection unavailable; market metadata/lifecycle; PM book snapshot; PM book delta/price change; PM BBO; OKX reference |
| Reconciliation | open orders; order detail; fill page/watermark; collateral; authorization; position |

One service turn admits at most 512 critical, 512 persistence, 64 private, 16
due scheduled, 256 public, eight reconciliation, and one telemetry item in
that fixed rank order. If critical work remains after its 512-item bounded
burst, the product globally stops before servicing lower ranks. Any backlog
that breaches its age limit takes the lane's fail-closed action rather than
changing ordering opportunistically.

## Bounded Lanes And Saturation

Capacities are compile-time/config-validation ceilings for Goal F:

| Lane/state | Capacity | Nominal high-water ceiling | Nominal maximum age | Saturation/age action |
| --- | ---: | ---: | ---: | --- |
| Critical safety and effect result | 512 | 32 | 250 ms | Reject producer, globally stop, no new fake dispatch |
| Persistence acknowledgement | 512 | 32 | 250 ms | Reject producer, globally stop, retain the effect permit, no fake dispatch |
| PM private lifecycle | 4,096 | 64 | 250 ms | Halt account, close epoch, require complete reconciliation |
| Integrity-bearing public snapshot/delta | 8,192 | 256 | 500 ms | Mark stream unavailable, invalidate epoch/readiness, explicit snapshot and resubscribe |
| Complete reconciliation/account snapshots | 128 | 16 | 5 s | Keep account unready and retry; partial pages never enter |
| Outbound reconciliation/refresh requests | 128 | 16 | 1 s | Keep affected account/token unready, retain one bounded pending-required bit, and retry; never declare a lost request complete |
| Raw capture frames | 8,192 entries / 32 MiB / 1 MiB each | 256 entries / 8 MiB | 500 ms | Mark capture invalid and resynchronize the source |
| PM journal/effect records awaiting durable acknowledgement | 1,024 | 128 | 1 s | Suppress dispatch and halt new quote creation |
| Fake place/cancel effects | 256 | 32 | 250 ms | Do not journal or dispatch the new effect; halt quoting; retain already scheduled owned cancels |
| Scheduled actions | 4,096 | 64 | 100 ms lateness | Suppress quote, deterministically schedule owned cancellation, globally halt if safety cannot be represented |
| Telemetry | 128 | 32 | Not readiness-bearing | Latest/coalesce allowed; never changes state or recovery |

A fake-effect permit is reserved before writing its intent record. If none is
available, there is no record claiming dispatch and no dispatch. If the intent
record is rejected before enqueue, no record exists and the unused permit is
released. Once the intent record is accepted, its permit remains bound and is
not released on a missing or failed durable acknowledgement; only durable
success may advance it to the prepared fake-effect queue.

State-bearing messages are never silently dropped or coalesced. Telemetry is
the only coalescing lane. Latest-wins public BBO/reference observations may be
introduced only after the integrity reducer owns a valid complete state; they
are derived observations and cannot repair or conceal a raw gap.

## Target Crate And Dependency Shape

Every existing workspace edge remains unchanged. The exact candidate
direct-workspace adjacency added by Goal F is:

```text
reap-transport -> reap-core
reap-capture-framing -> -
reap-durable-writer -> -
reap-pm-core -> reap-core
reap-pm-state -> reap-pm-core
reap-polymarket-wire -> reap-pm-core
reap-polymarket-adapter -> reap-pm-core + reap-polymarket-wire + reap-transport
reap-okx-public-source -> reap-core + reap-transport
reap-pm-strategy -> reap-pm-core + reap-pm-state
reap-pm-live-contracts -> reap-pm-core + reap-pm-strategy
reap-pm-live -> reap-capture-framing + reap-durable-writer + reap-okx-public-source + reap-pm-core + reap-pm-live-contracts + reap-pm-state + reap-pm-strategy + reap-polymarket-adapter + reap-transport
reap-capture -> reap-book + reap-capture-framing + reap-core + reap-feed + reap-telemetry + reap-venue
reap-feed -> reap-book + reap-core + reap-transport + reap-venue
reap-storage -> reap-core + reap-durable-writer
reap-venue -> reap-core + reap-okx-public-source
```

`reap-feed` adopts `reap-transport` for extracted supervision mechanics.
`reap-venue` keeps its existing public API and normalized event construction,
delegating only exact index-ticker field extraction to
`reap-okx-public-source`; it does not re-export the new session or subscription
surface. `reap-okx-public-source` owns only public OKX session, subscription,
parser, integrity, and exact reference behavior; it has no dependency on
authenticated `reap-okx-wire`, broad `reap-venue`, or any private/order
adapter. No existing workspace-crate edge is removed. The direct `libc`
mechanics dependency moves from `reap-feed` to the neutral transport crate
that now owns the process-shared pacer.

Goal F mechanically extracts two already required neutral mechanisms:

- `reap-capture-framing` owns bounded typed JSONL framing/writing, hash
  accumulation, trailing-record detection, and byte/record verification.
  Existing `reap-capture` wraps it with the unchanged Chaos `RawCapture` and
  normalized schemas; `reap-pm-live` wraps it with PM raw-capture schema 1.
  Default framing APIs reserve a tracked worst-case byte slab before a capped
  counting pass and fixed-capacity serialization pass. Only `reap-capture`
  enables the explicitly named `legacy-reap-capture` compatibility feature;
  the workspace root and every other crate are denied that feature and its
  uncapped writer, encoder, and scanner symbols by an exact recursive
  allowlist.
- `reap-durable-writer` owns canonical path/lease locking, bounded
  enqueue/progress, static record-codec invocation on the writer task,
  flush/`sync_data`, durable result delivery, and deterministic shutdown.
  Existing `reap-storage` wraps it with `StorageRecord` schema 7 and
  byte-identical public behavior; `reap-pm-live` wraps it with the distinct PM
  journal schema 1.

Neither neutral crate knows a venue, product record union, domain authority,
or recovery semantics. Extraction is mechanical and gated by byte-identical
existing capture/journal fixtures, existing public API compatibility, and the
canonical Chaos hashes before either PM wrapper is admitted. PM code never
depends on broad `reap-capture` or `reap-storage`.

The responsibility/module DAG is also frozen:

| Crate | Production modules admitted in Goal F |
| --- | --- |
| `reap-transport` | `bounded`, `supervisor`, `backoff`, `health`, `shutdown`; no venue protocol |
| `reap-capture-framing` | `frame`, `bounded_writer`, `hash`, `verify`; no venue/schema DTO |
| `reap-durable-writer` | `lease`, `bounded`, `progress`, `writer`; no product record/recovery module |
| `reap-okx-public-source` | `session`, `subscription`, `public_wire`, `reference`; no private/auth/order module |
| `reap-pm-core` | `identity`, `numeric`, `metadata`, `mapping`, `event`, `envelope` |
| `reap-pm-state` | `book`, `order`, `reservation`, `position`, `readiness`, `risk` |
| `reap-polymarket-wire` | `public_rest`, `public_ws`, `private_fixture`, `unsigned_order`; no auth/signing |
| `reap-polymarket-adapter` | `public`, `private_fixture`, `reconcile_fixture`, `account_fixture`, `fake_execution` |
| `reap-pm-strategy` | `model`, `quote_policy`; pure/static only |
| `reap-pm-live-contracts` | `config`, `requirements`, `plan` |
| `reap-pm-live` | `coordinator`, `lanes` with private `lanes::{bounded,policy,scheduled}`, `schedule`, `journal`, `capture`, `replay`, `fake_effect`, `composition` |

No production module may absorb another row's responsibility to evade the
1,500-line file or 250-line function review.

The exact candidate production-module dependency edges below are complete for
all new modules and every existing module whose direct workspace dependency
changes. Untouched existing internal edges remain the Phase 0 baseline:

```text
reap-transport::supervisor -> bounded + backoff + health + shutdown
reap-transport::bounded -> reap-core::types
reap-capture-framing::bounded_writer -> frame + hash
reap-capture-framing::verify -> frame + hash
reap-durable-writer::bounded -> progress
reap-durable-writer::writer -> bounded + lease + progress
reap-feed::supervisor -> reap-transport::{backoff,bounded,health,shutdown,supervisor}
reap-venue::okx::public -> reap-okx-public-source::reference
reap-capture::hashing -> reap-capture-framing::hash
reap-capture::writer -> reap-capture-framing::bounded_writer
reap-capture::verification -> hashing + reap-capture-framing::{frame,verify}
reap-storage::lib -> reap-durable-writer::{bounded,lease,progress,writer}
reap-okx-public-source::session -> public_wire + reference + subscription + reap-transport::{backoff,bounded,health,shutdown,supervisor}
reap-okx-public-source::public_wire -> reap-core::types
reap-okx-public-source::reference -> public_wire + reap-core::types
reap-okx-public-source::subscription -> reap-core::types
reap-pm-core::identity -> numeric + reap-core::types
reap-pm-core::metadata -> identity + numeric
reap-pm-core::mapping -> identity
reap-pm-core::event -> identity + metadata + numeric
reap-pm-core::envelope -> identity + reap-core::types
reap-pm-state::book -> reap-pm-core::{identity,numeric}
reap-pm-state::order -> reap-pm-core::{identity,numeric}
reap-pm-state::reservation -> order + reap-pm-core::numeric
reap-pm-state::position -> reap-pm-core::{identity,numeric}
reap-pm-state::readiness -> book + order + position + reservation + reap-pm-core::metadata
reap-pm-state::risk -> order + position + readiness + reservation
reap-polymarket-wire::public_rest -> reap-pm-core::{identity,metadata,numeric}
reap-polymarket-wire::public_ws -> reap-pm-core::{event,identity,numeric}
reap-polymarket-wire::private_fixture -> reap-pm-core::{event,identity,numeric}
reap-polymarket-wire::unsigned_order -> reap-pm-core::{identity,numeric}
reap-polymarket-adapter::public -> reap-pm-core::{event,identity,metadata,numeric} + reap-polymarket-wire::{public_rest,public_ws} + reap-transport::{backoff,bounded,health,shutdown,supervisor}
reap-polymarket-adapter::private_fixture -> reap-pm-core::{event,identity,numeric} + reap-polymarket-wire::private_fixture
reap-polymarket-adapter::reconcile_fixture -> reap-pm-core::{event,identity,numeric} + reap-polymarket-wire::private_fixture
reap-polymarket-adapter::account_fixture -> reap-pm-core::{event,identity,numeric} + reap-polymarket-wire::private_fixture
reap-polymarket-adapter::fake_execution -> reap-pm-core::{identity,numeric} + reap-polymarket-wire::unsigned_order
reap-pm-strategy::model -> reap-pm-core::{mapping,metadata,numeric} + reap-pm-state::{book,position,readiness}
reap-pm-strategy::quote_policy -> model + reap-pm-core::numeric + reap-pm-state::risk
reap-pm-live-contracts::config -> reap-pm-core::{identity,mapping,numeric}
reap-pm-live-contracts::requirements -> config + reap-pm-strategy::model
reap-pm-live-contracts::plan -> config + requirements
reap-pm-live::lanes -> lanes::{bounded,policy,scheduled} + reap-pm-core::{envelope,event,identity} + reap-polymarket-adapter::reconcile_fixture + reap-transport::bounded
reap-pm-live::lanes::bounded -> policy
reap-pm-live::lanes::policy -> reap-pm-live-contracts::requirements
reap-pm-live::lanes::scheduled -> policy + reap-pm-core::identity
reap-pm-live::schedule -> reap-pm-core::identity + reap-pm-live-contracts::plan
reap-pm-live::journal -> reap-durable-writer::{lease,progress,writer} + reap-pm-core::{event,identity,numeric}
reap-pm-live::capture -> reap-capture-framing::{bounded_writer,frame,hash,verify} + reap-okx-public-source::session + reap-pm-core::{envelope,event,identity,numeric} + reap-polymarket-adapter::public
reap-pm-live::coordinator -> lanes + schedule + reap-pm-state::{book,order,position,readiness,reservation,risk} + reap-pm-strategy::{model,quote_policy}
reap-pm-live::fake_effect -> coordinator + journal + reap-polymarket-adapter::fake_execution
reap-pm-live::replay -> capture + coordinator + journal
reap-pm-live::composition -> capture + coordinator + fake_effect + journal + lanes + replay + schedule + reap-pm-live-contracts::plan + reap-pm-state::{order,position,readiness,reservation} + reap-polymarket-adapter::{account_fixture,private_fixture,reconcile_fixture}
```

Modules with no listed outgoing production edge are leaves. `reap-storage` and
`reap-capture` retain their existing internal domain/module shape and wrap only
the corresponding neutral mechanism; their old schemas remain above it.

The exact constructor reach inside `composition` is narrower than its module's
full static dependency set:

| Composition root | Directly constructed roles |
| --- | --- |
| `PmPublicCapture` | `capture`, which alone constructs `reap-okx-public-source::session` and `reap-polymarket-adapter::public` |
| `PmReadOnlyMonitor` | `reap-polymarket-adapter::{private_fixture,reconcile_fixture,account_fixture}` plus PM order/position/reservation/readiness reducers; no `capture`, model, journal, or fake effect |
| `PmProduct<Model>` | `capture`, the three fixture-only read roles, coordinator/model, journal, and `fake_effect`; fake effect alone constructs `reap-polymarket-adapter::fake_execution` |

Source-policy and compile-fail tests pin these three constructor call graphs
and prove every plan entry has exactly one validated constructed-role binding
and every concrete role emits only the exact bindings assigned to it. The
composition module cannot access raw wire DTO modules directly.

Responsibilities:

- `reap-pm-core`: structural identities, exact values, normalized events,
  metadata/lifecycle types; no IO.
- `reap-pm-state`: pure exact book/order/reservation/position/readiness
  reducers.
- `reap-polymarket-wire`: public DTO/parser, fixture-only private DTO/parser,
  canonical unsigned order fields; no auth, signer, key, session, or network
  client.
- `reap-polymarket-adapter`: public observation and fixture/fake narrow roles;
  no broad client.
- `reap-transport`: bounded transport supervision, reconnect/backoff,
  timestamp/queue-age delivery, shutdown and health mechanics only. It knows
  no OKX/PM subscription, ACK, heartbeat, integrity, DTO, or execution rule.
- `reap-okx-public-source`: the only PM-product-facing OKX dependency. It
  exports configured public reference observation and no OKX private,
  account, reconciliation, signer, order, or emergency/evidence role. Its
  venue-owned session binds every raw delivery to the configured connection,
  advances a checked epoch on every reconnect, treats epoch overflow as
  terminal, and lets only the exact subscription acknowledgement establish
  readiness; heartbeat is liveness evidence, not subscription readiness.
- `reap-pm-strategy`: static pure quote-model and quote-policy boundary.
- `reap-pm-live-contracts`: secret-free config validation and typed capability
  plan.
- `reap-pm-live`: PM coordinator, replay/composition root, fake effects,
  PM-specific journal, stable integration test and benchmark targets.
- `reap-capture-framing`: schema-neutral bounded framing, writer evidence,
  hashing, and verification mechanics used by both capture wrappers.
- `reap-durable-writer`: statically typed leased writer mechanics used by both
  journal wrappers; serialization runs on the writer task, never in an owner
  reducer.

Existing `reap-live`, `reap-live-contracts`, `reap-order`,
`reap-okx-live-adapter`, and Chaos strategy cannot depend on PM crates. They may
only gain an explicit fail-closed `Polymarket` match where adding the common
venue variant mechanically requires it.

`reap-pm-live` cannot depend directly on `reap-okx-live-adapter`,
`reap-order`, OKX evidence/emergency adapters, or a broad existing live
composition. The narrow OKX public-source crate keeps transitive raw parsing
private and exposes only the configured reference role.

Allowed shared work is limited to genuinely identical mechanics: common
venue/source identity, untrusted raw envelopes, bounded transport supervision,
capture/replay framing, monotonic queue-age carriers, and leased writer
mechanics. Venue-owned subscription bytes, ACK/login/heartbeat rules, parsing,
integrity, and capability constructors stay venue-specific.

Do not genericize the f64 Chaos `BookReducer`, OKX gateway, Chaos live config or
connectivity plan, `LiveCoordinator`, `LiveRuntime`, or the existing
`StorageRecord`.

## Public API And Schema Boundary

Authorized shared public change:

- add `Polymarket` to common venue/source identity while preserving the exact
  existing serialized OKX representation and adding explicit fail-closed old-
  product matches.
- export from `reap-transport` only the concrete bounded supervision,
  backoff/health, shutdown, and typed immutable-delivery mechanics needed by
  both venue-owned protocols.
- export from `reap-okx-public-source` only configured public observation
  requirements/events and the public-session constructor; no credentials,
  login, private DTO, raw client, arbitrary subscription, account,
  reconciliation, or order role.
- export from `reap-capture-framing` by default only schema-neutral bounded
  frame/writer configuration, immutable frame evidence, hash/verification
  results, and the statically typed writer runtime. It exports no OKX or PM DTO
  and no parser. Explicitly named uncapped compatibility symbols exist only
  behind `legacy-reap-capture` and are source-allowlisted solely to the existing
  Chaos capture facade.
- export from `reap-durable-writer` only lease identity/configuration, the
  static `JournalCodec<Record>` contract, bounded sink/runtime, durability
  result, and numeric progress snapshot. It exports no domain record enum,
  recovery proof, venue capability, or dynamically dispatched codec.

Authorized new APIs are capability-bearing types exported only by the PM crate
that owns them. Executable constructors, approval minting, ownership binding,
prepared fake effects, and fake transport consumption remain private or
crate-private and are protected by compile-fail/source-policy tests.

PM persistence uses a new PM-specific journal/schema and scoped lease. Its
non-secret lease identity binds product `reap-pm`, environment, chain,
account/funder identity, the canonical sorted configured market/token scope,
schema family, and schema version through one exact scope fingerprint. The
descriptor is checked against the first journal header before recovery or
append. It records exact identity/units, source/connection/revision/ordering
fields, intent, reservation, durable acknowledgement, fake result, fill
deduplication, and terminal/safety facts required for deterministic ownership
recovery. Complete account/order/position snapshot payloads belong to the PM
capture/replay schema, not the mutation journal. Restart begins snapshot
readiness as unknown and requires a fresh complete reconciliation before
quoting. A reconciled fill-ID eviction watermark is journaled only when it
actually advances; the nominal benchmark advances none. The journal is
backwards-readable from its first checked-in version.

The existing Chaos `StorageRecord` and schema 7 bytes are unchanged.
`reap-storage` retains its existing lease/public facade and legacy lock-file
bytes while delegating the extracted lock/write mechanics underneath it. PM
recovery rejects Chaos envelopes before domain interpretation and Chaos
recovery rejects PM envelopes. Both typed schemas share only
`reap-durable-writer`.

## Local Measurement Contract

These target names are stable:

```bash
cargo test -p reap-pm-live --test combined_replay --locked
cargo bench -p reap-pm-live --bench pm_action_path --locked
```

The benchmark is local architecture evidence, not a production latency claim.
Run with locked dependencies and optimized bench profile on the same idle host:
one unrecorded warm-up and three recorded runs. A run is invalid if counters,
capacity configuration, fixture hash, build revision, timer metadata, or
toolchain/host identity is missing or differs between recorded runs.

The primary timed boundary begins when an immutable normalized event is
delivered to the coordinator and includes exact reducers, readiness/risk,
fixture-model evaluation, checked quote conversion, ownership/reservation,
PM-record construction and bounded enqueue, consumption of an injected durable
acknowledgement, and preparation/enqueue of the fake effect. It excludes socket
receive, websocket framing, JSON parsing, filesystem serialization/fsync,
network IO, and fake exchange service time. Real PM parser work is timed and
reported as a separate segment with its own allocations and cannot be folded
into or used to hide owner-loop allocations.

### Nominal workload

Warm-up is 1,000 ten-observation cycles (10,000 observations). The measured
pass is 10,000 ten-observation cycles (100,000 observations), alternating
5,000 fill cycles and 5,000 cancel/replace cycles.

Every cycle includes:

1. PM book observation;
2. OKX reference observation;
3. quote-evaluation timer observation, which invokes model evaluation and an
   exact quote decision;
4. durable quote-intent acknowledgement; and
5. fake place acceptance.

A fill cycle then includes one full fill, the same duplicate fill, one complete
position snapshot, one complete empty-open-orders snapshot, and one freshness
timer. A cancel cycle instead includes one replace timer, durable cancel-intent
acknowledgement, fake cancel acceptance, one complete empty-open-orders
snapshot, and one complete account snapshot.

The exact measured counters are:

| Counter | Required |
| --- | ---: |
| Input observations | 100,000 |
| Quote decisions / durable quote intents / fake place effects | 10,000 each |
| Cancel decisions / durable cancel intents / fake cancel effects | 5,000 each |
| Refresh/reconciliation requests | 5,000 |
| PM journal records | 35,000 |
| Unique fills applied | 5,000 |
| Duplicate fills suppressed | 5,000 |
| Terminal filled orders | 5,000 |
| Terminal cancelled orders | 5,000 |
| State-bearing drops | 0 |
| Queue saturations | 0 |

The 35,000 journal records derive exactly as follows:

| Cycle | Count | Records per cycle | Total |
| --- | ---: | --- | ---: |
| Fill | 5,000 | quote intent, place acknowledgement, unique fill | 15,000 |
| Cancel | 5,000 | quote intent, place acknowledgement, cancel intent, cancel acknowledgement | 20,000 |

Duplicate fills and complete snapshot inputs are captured/replayed but do not
create mutation-journal records. No fill-eviction watermark advances in this
workload.

After construction and ingress parsing, normalized-event-to-record/effect
owner-loop work must allocate zero heap calls and zero bytes. Preallocated
canonical state plus queues must reserve no more than 64 MiB; replay/recovery
working state must reserve no more than 16 MiB; completed cycles must not grow
cardinality.

The benchmark installs the workspace counting allocator immediately after
preallocation. It reports call/byte deltas for the timed owner path.
`reserved_capacity_bytes()` deterministically sums every canonical container,
queue, slab, schedule, and retained-ID allocation and must be at most 64 MiB.
Replay runs from a fresh process/state and reports allocator peak-live minus
post-construction live bytes, which must be at most 16 MiB. Five repeated
nominal passes compare exact container capacities, lengths, retained IDs,
orders, schedules, and allocator live bytes with the first reconciled-terminal
baseline.

Each nominal lane's high-water mark must remain at or below the exact ceiling
in the lane table. Raw capture reports and enforces both entry and byte
high-water. Each recorded run reports exact nearest-rank
p50/p95/p99/p99.9/max latency, timer-read overhead, queue age, allocations,
bytes, counters, total elapsed nanoseconds, and observations/second over all
100,000 reductions. The 15,000 action-latency samples are correlation spans:
10,000 quote-evaluation receive times through durable quote ack to prepared
fake-place enqueue, plus 5,000 replace-timer receive times through durable
cancel ack to prepared fake-cancel enqueue. On the Phase 0 local host, every
recorded run's 15,000-sample distribution must have p50 no greater than 25
microseconds and p99.9 no greater than 250 microseconds. A regression above
either budget stops the phase for architecture review; it is not waived by
averaging. Max latency is reported but is not a local-host acceptance
percentile.

### Overload workload

Each case starts from fresh preallocated state:

| Case | Attempts | Exact required result |
| --- | ---: | --- |
| Public integrity | 8,193 | 8,192 accepted/high-water, one rejected, one epoch invalidation/resync |
| Private lifecycle | 4,097 | 4,096 accepted, one rejected, one account halt and one complete-reconcile request |
| Critical | 513 | 512 accepted, one rejected, one global stop |
| Persistence acknowledgements | 513 | 512 accepted, one rejected, one global stop and zero new fake dispatch |
| Complete snapshots | 129 | 128 accepted/high-water, one rejected, affected account unready and one bounded retry requirement |
| Reconciliation/refresh effects | 129 | 128 accepted/high-water, one rejected, affected scope unready with one pending-required bit |
| Raw capture entries | 8,193 one-byte frames | 8,192 accepted/high-water, one rejected, capture invalid and one stream resync |
| Raw capture bytes | 33 one-MiB frames | 32 accepted to the 32-MiB slab, one rejected, capture invalid and one stream resync |
| Oversize raw frame | One frame of 1 MiB + 1 byte | Zero accepted, one rejected, capture invalid and one stream resync |
| Storage | 1,025 | Queue 1,024 distinct `FillApplied` records of `0.01` share for one preseeded proven-owned `10.24`-share order while injected durable results are withheld; reject the final attempted quote-intent record, release its reserved effect permit, and perform zero fake dispatch |
| Fake effect | 257 | 256 queued only after 256 durable records, one rejected with no record/dispatch |
| Scheduled actions | 4,097 | 4,096 inserted, one rejected, one global stop and quote suppression; the fresh fixture owns no live order, so exactly zero cancel candidates |
| Telemetry | 129 | 128 retained, one coalesced, zero readiness/recovery transition |

The overload suite therefore makes exactly 27,309 attempts across thirteen
fresh cases.
Allowed state-bearing drops are zero. The only allowed coalescing count is the
single telemetry item in its overload case. Every case remains within the same
64 MiB bound, has no post-construction owner-loop allocations, and performs no
unbounded retry or state growth. Every state-bearing case reaches exactly one
declared fail-closed/resync transition; telemetry reaches none. Synthetic
monotonic-time tests exercise every state-bearing declared age boundary with
the same fail-closed outcome as capacity saturation.

## Provenance And Known Reference Defects

The pinned tracked Predarb object
`8222273a9c72033b760e1d2fec813bc77144556d` is reference material, not a
dependency. Only reviewed Polymarket paths and fixtures from that object may be
ported, with content hashes recorded in the Goal F handoff.

The following reference behavior must become failing fixtures before semantic
implementation and must not be copied:

1. REST trade conversion may use an individual trade size as cumulative filled
   quantity and set remaining to zero, while later state expects cumulative
   progress.
2. Allowance parsing may select an arbitrary first map value or collapse
   required spenders into one effective value.
3. Six-decimal maker/taker conversion rounds instead of rejecting values that
   are not integral protocol units.
4. Unknown order status may be normalized into an ordinary pending state.
5. Its metadata preflight does not by itself prove every lifecycle, minimum,
   grid, and spender dependency required by this boundary.
6. Its negative-risk spender list retains the old Neg Risk Adapter even though
   the official registry now marks that adapter CLOB V1 and deprecated.

The official Polymarket protocol pages retrieved on 2026-07-20 close the Phase
0 GTC/post-only, canonical-field, tick/minimum, CLOB V2 domain, and exact
trading-spender evidence gate when combined with the independently specified
goldens in the Goal F handoff. They are recorded there with retrieval hashes.
They do not authorize copying an SDK, adding authentication, or assuming that
future protocol revisions are compatible.

## Explicit Exclusions And Stop Conditions

Excluded from Goal F:

- Predict.fun and all cross-prediction-venue pairing/hedging;
- a production probability formula or default model;
- OKX private/account/order/reconciliation roles in the PM product;
- PM FOK, FAK/IOC, GTD, market, synthetic IOC, reduce-only, batch, cancel-all,
  cancel-market, or arbitrary request authority;
- real keys, credentials, signer/auth/header/session code, authenticated
  requests, live mutation, a deployed PM binary, CLI, or systemd service;
- settlement, redeem/split/merge, relayer, wallet deployment, and allowance
  mutation;
- target-host qualification, CPU affinity, custom runtime, thread-per-core,
  ring buffers, kernel bypass, or other host tuning;
- dynamic plugin/venue registries, universal adapters, task-per-order, and
  shared mutable canonical state; and
- production-order-entry authorization. Every evidence artifact remains
  `production_order_entry_authorized: false`.

Stop the goal or current phase when:

- pinned/official evidence no longer proves the frozen fake profile or fields;
- reference revisions, dirty-state handling, or fixture provenance cannot be
  established without reading secrets/untracked runtime state;
- an implementation requires a signer, authenticated private network role, or
  live request to prove correctness;
- existing Chaos behavior, serialized bytes, canonical hashes, authority
  boundaries, or dependency policy would change;
- exact units, complete snapshot semantics, required spenders, ownership, or
  deterministic ordering cannot be proven;
- a queue can only make progress through silent state loss or unbounded growth;
- the stable replay/benchmark counters, memory/allocation bounds, or local
  regression budgets cannot be met; or
- a new production module would exceed 1,500 lines or function 250 lines
  without a recorded responsibility-based exception and decomposition review.
