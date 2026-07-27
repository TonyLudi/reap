# Polymarket Authenticated Execution Goal G Execution Prompt

Status: **amended and runnable under Amendments 1 and 2**. Goal F is complete. Goal E remains deferred
because no target host or target-host acceptance contract is declared. Goal G
may run before Goal E and does not complete or requalify it. The original
Phase 0 stop and its evidence remain in the
[Goal G handoff](polymarket-authenticated-execution-goal-g-handoff.md), but
the three stopped contracts are superseded by Amendment 1 below. Amendment 2
incorporates the fresh restarted-Phase-0 source audit without broadening the
product.

Use this document as the complete instruction set. The exact invocation is:

> /goal Execute Goal G exactly as specified in
> `docs/polymarket-authenticated-execution-goal-g-prompt.md`. Continue phase by
> phase through every green gate and stop only at completion or a documented
> stop condition.

## Amendment 1 — 2026-07-27

This amendment is user-authorized and normative everywhere this prompt, the
boundary, or the historical handoff conflicts with it. It makes three primary
unblock decisions, with supporting execution clarifications for evidence-only
construction, provider qualification, benchmark collection, and journal phase
ordering:

1. Goal G adds one credential-free, closed, read-only Polygon authorization
   source. It obtains the ERC-20 allowance and ERC-1155 operator approval
   directly at one finalized Polygon block. The authenticated CLOB
   `/balance-allowance` numeric map remains separate cache/diagnostic evidence
   and never becomes the boolean approval.
2. Inbound REST/WS lifecycle and timestamp conflicts are handled by a frozen,
   source/message-family-tagged compatibility union. Only explicitly
   enumerated semantic equivalents normalize. Unknown, ambiguous, malformed,
   cross-family, or out-of-profile input is quarantined and fails closed; it
   is never guessed into success.
3. The existing PM action benchmark's absolute `25,000 ns` p50 and
   `250,000 ns` p99.9 assertions are replaced by the paired local relative
   rule defined in Phases 0 and 6. Workload, logical, hash, allocation,
   memory, cardinality, and queue gates remain exact. No local result is a
   target-host SLO.

Goal G selects no production Polygon RPC provider or origin and sends no real
Polygon request. It implements the closed protocol and loopback-proven source;
Goal H must inject and qualify one exact deployment origin before that role is
constructible in a production composition. Absence of a target host, provider,
or provider credential is therefore not a Goal G stop.

## Amendment 2 — 2026-07-27

This amendment is user-authorized and normative wherever the prompt,
boundary, or handoff conflicts with it. Fresh public official documentation
and pinned current official client source exposed one normal settlement state
and several source-family distinctions that the first amendment did not
enumerate precisely:

1. Account trade reconciliation admits a distinct canonical
   `MatchedNotBroadcast` settlement fact for raw
   `TRADE_STATUS_MATCHED_NOT_BROADCASTED` and its separately tagged current
   guide/client compatibility spelling `MATCHED_NOT_BROADCASTED`. It means
   that orders matched before an on-chain transaction was broadcast. It is
   nonterminal and is never aliased to trade `Matched`, an order `MATCHED`, a
   POST `matched`, mined, confirmed, or placement success. Provisional fill
   exposure is retained, any open remainder stays reserved, and no transaction
   or finality fact is invented.
2. That sixth state is currently proven for account trade listings only.
   Direct raw user-WS trade input accepts the five ordinary settlement states
   in their exact prefixed or unprefixed current-client spellings. Either
   not-broadcast spelling on the raw user WS is a source-family violation:
   quarantine it, invalidate the private epoch, and force a complete REST
   reconciliation cut. SDK-normalized `topic/type/payload`, camel-case, and
   RFC-3339 objects are differential oracles, not additional wire envelopes.
3. The exact per-route lifecycle and field-local timestamp compatibility
   tables in the boundary are closed. There is no global status alias set,
   timestamp magnitude guesser, or SDK-output parser. Goal F's existing five
   settlement meanings and every fake/journal V1 byte stay unchanged. The new
   state is live-authenticated-only; the distinct authenticated journal V1
   represents it, while the frozen fake journal V1 remains unable to encode
   it.
4. Every stable requirement ID has exactly one canonical lane. Mutation
   result IDs use `Critical`; separately named child IDs own persistence
   dispatch barriers and recovery reconciliation. A row may not name two
   lanes.
5. Every Goal G `reqwest` client explicitly uses
   `reqwest::retry::never()`, `redirect::Policy::none()`, and `no_proxy()`.
   Read refreshes are the boundary's capped coordinator-owned fresh-attempt
   cycles. A mutation grant gets one application dispatch and is never
   replayed by the HTTP client, including the default protocol-NACK policy
   configured by locked `reqwest 0.12.28` when a relevant protocol feature is
   enabled. A later exact-owned recovery cancel is a separately journaled
   operation after a complete cut, never reuse of the prior request/grant.
6. The boundary's exact route/query/body/pagination table, dependency/feature
   table, request-header/mutation-response union, outbound-clock/geoblock
   policy, five literal JSON-RPC templates, dependency/feature table,
   source-manifest hashes, benchmark campaign, and independently authored
   protocol-vector specification are the Phase 0 implementation contract. A
   source conflict is resolved only by its listed narrow acceptance or
   fail-closed behavior; it does not authorize a live probe.

`PmFillSettlementStatus` may therefore gain `MatchedNotBroadcast`, with exact
transition/readiness tests, without changing the serialized fake journal V1.
Any formerly total fake-journal conversion must become a checked conversion
that proves this live-only state cannot enter the fake family. Skipped
settlement observations may be covered only by a complete authoritative
reconciliation cut; an ordinary event cannot silently jump, regress, or
collapse lifecycle states.

## Objective

Complete the production-shaped Polymarket trading-connectivity layer for the
bounded PM product created by Goal F, while preserving Reap's static
capability boundaries, exact PM numerics, single canonical mutation owner, and
existing Chaos/OKX product.

For Goal G, "authenticated execution complete" means that Reap implements and
proves locally:

```text
OKX public reference + PM public market data
                         |
PM authenticated user order/fill events
+ PM authenticated order/account reconciliation
+ PM exact CLOB collateral/token state
+ PM finalized-chain ERC20 allowance/ERC1155 approval state
+ PM monitored published positions
                         |
existing pure model, PM state, readiness, and risk
                         |
durable exact GTC post-only intent
                         |
EOA CLOB V2 order signature + L2 request authentication
                         |
narrow PM place-GTC-post-only / cancel-exact-owned transport
                         |
typed result, ambiguity repair, fill and position convergence
```

The required outcomes are:

1. a least-authority, redacted, zeroizing credential and EOA signer boundary;
2. byte-exact current CLOB V2 EOA order signing and L2 request
   authentication;
3. authenticated PM user WebSocket, account, and reconciliation roles, a
   separate credential-free finalized-chain authorization role, and a
   separate published-position observation role;
4. one live execution profile: one configured token, `GTC`,
   `postOnly = true`, `deferExec = false`, zero expiration, exact buy/sell
   amounts, and cancellation of a proven locally owned venue order only;
5. deterministic venue-order identity, durable pre-dispatch evidence, no
   blind mutation retry, and bounded fail-closed ambiguity reconciliation,
   with an explicit durable operator-required halt when identity cannot be
   proven;
6. a statically composed authenticated PM product that feeds the existing
   canonical reducers and never exposes credentials, a signer, a broad
   client, or arbitrary exchange commands;
7. bounded startup, reconnect, recovery, exact-owned cleanup, shutdown, and
   fault behavior;
8. credential-free official-vector, loopback, deterministic, security,
   allocation, bounded-memory, and local performance-regression evidence; and
9. unchanged Goal F fake behavior and unchanged Chaos/OKX behavior,
   authority, encodings, fingerprints, canonical outputs, and supported
   connectivity.

Goal G completes trading connectivity as a sealed library composition, not a
tradable strategy or an approved deployment. It deliberately remains
non-deployable and non-invocable against production until Goal H adds a
reviewed secret source and executable composition. It MUST NOT invent a
production fair-probability, spread, size, inventory, fee, or risk model. It
MUST NOT read real credentials, contact an authenticated production endpoint,
submit or cancel a real order, mutate an allowance, contact a real Polygon RPC
endpoint, claim target-account or target-host evidence, or authorize
production order entry.

Every newly introduced Goal G evidence/report artifact MUST carry five
boolean fields whose values are:

```text
production_order_entry_authorized: false
real_credentials_loaded: false
authenticated_external_request_sent: false
real_polygon_rpc_request_sent: false
real_order_submitted: false
```

Existing Goal F artifacts retain their frozen schema and
`production_order_entry_authorized: false`; the Goal G handoff separately
attests the other four facts for Goal F reruns.

A later separately authorized Goal H must supply the exact account, secret
source, qualified Polygon RPC origin, market/token, capital limits, controlled
trial procedure, permission for any real authenticated or Polygon request or
order, and independently reviewed proof of exclusive EOA/funder mutation
scope across every API key, UI, process, manual action, and on-chain actor.
It must also prove that the chosen provider supports chain `137`,
`finalized`, historical exact-block `eth_call`, the closed JSON-RPC contract,
and the stated bounds, and consume reviewed target-host wall-clock discipline
before chain evidence can grant readiness. Goal E may later produce host
qualification evidence, but it does not select secrets/providers or authorize
trading.

## Normative Baseline And References

The starting Reap implementation baseline is commit
`43970849267c0282d118a369a792066c4655deae`. Starting `HEAD` MUST contain that
commit. A reviewed documentation-only Goal G contract commit containing this
prompt, its boundary, and its handoff may also be present, provided it changes
no production or dependency file. The authorized Phase 0 benchmark-policy
commit `facd3a616fc20e7bc1abc627235588b7532ff8b1`, a documentation-only
Amendment 2 runnable-contract commit that explicitly does not claim Phase 0
green, and the later completed Phase 0 evidence/gate commit may also be
present. None changes the behavioral comparison baseline. The prompt-creation
`Cargo.lock` SHA-256 is
`2673d055c943c3bd5444531b67df280026c145cbbbc99b68a06f4ac0c2dbb0ff`;
Phase 0 records and reviews any dependency delta from it.

Goal F's final code/evidence tree is
`d16c3cbdac97fb43944e3a97d4f9b56e92206747`. Treat these Reap documents as
normative, in this order:

- this prompt, including its dated amendments;
- [polymarket-authenticated-execution-boundary.md](polymarket-authenticated-execution-boundary.md)
  for Amendment 2's exact route, lifecycle, time, dependency, vector, lane,
  and resource contracts;
- [polymarket-authenticated-execution-goal-g-handoff.md](polymarket-authenticated-execution-goal-g-handoff.md)
  for the current run state and explicitly labeled historical stopped-run
  evidence;
- [multi-venue-polymarket-foundation-goal-f-handoff.md](multi-venue-polymarket-foundation-goal-f-handoff.md)
  for the completed implementation and exact final evidence;
- [polymarket-product-connectivity-boundary.md](polymarket-product-connectivity-boundary.md)
  for PM identity, numeric, capability, ownership, state, and fixed EOA
  execution semantics;
- [architecture.md](architecture.md) for single-writer ownership, event flow,
  and async-edge rules;
- [trading-readiness.md](trading-readiness.md) for the distinction between
  implemented mechanics, credentialed evidence, and authorization;
- [performance.md](performance.md) for local measurement methodology and
  limitations;
- [multi-venue-polymarket-foundation-goal-f-prompt.md](multi-venue-polymarket-foundation-goal-f-prompt.md)
  for Goal F's frozen historical scope;
- [determinism-readiness-goal-d-handoff.md](determinism-readiness-goal-d-handoff.md)
  for the existing deterministic Chaos/OKX anchors; and
- [chaos-connectivity-boundary.md](chaos-connectivity-boundary.md) for the
  unchanged Chaos connectivity boundary.

The existing Chaos behavioral reference remains the clean sibling checkout
`../imm-strategy` at
`b6b120c7b7c466d8431bf082f3229328c5d7b2ae`. Do not modify it. It remains
normative only for the existing Chaos/iarb2 call path and does not define PM
authentication or broaden Goal G.

The historical Polymarket implementation reference remains the tracked Git
object in `../predarb` at
`8222273a9c72033b760e1d2fec813bc77144556d`. Use it only for reviewed lessons
from its L1/L2 authentication, V2 order signing, authenticated REST/user-WS,
order lifecycle, ambiguity, reconciliation, fill, and position code.

Inspect these pinned tracked paths first with Git object commands:

- `crates/venue-polymarket/src/auth/{l1,l2}.rs`;
- `crates/venue-polymarket/src/signing/{funder,order_v2}.rs`;
- `crates/venue-polymarket/src/rest/private.rs`;
- `crates/venue-polymarket/src/ws/user.rs`;
- `crates/venue-polymarket/src/{adapter,readiness,normalize}.rs`;
- `crates/venue-polymarket/src/raw/{rest,ws}.rs`;
- `src/order_gateway/{gateway,journal,polymarket,reconciler,reservations,user_events}.rs`;
- `src/position_gateway/mod.rs`; and
- `src/runtime/{polymarket_private_ws,private_ingress,reconcile}.rs`.

Use them to identify useful concepts such as exact-body HMAC, identity
binding, separate hot/reconciliation paths, persist-before-send,
acknowledgement-unknown reconciliation, structural fill deduplication, and
published-versus-derived position evidence. Re-derive all protocol bytes and
fit every concept into Reap's narrower ownership design.

The pinned historical parser-fixture SHA-256 anchors are:

```text
balance_allowance.json  7e1f683ac5032b137d8a2afdfafccce389198bb5d3a33ba6eb3cb478455fab96
market_book.json        8e671f14c4b1e8137b1dc1b0bd7d39c79d9c8f961a8483daa32151df99cbdf81
open_order.json         d0998ca29cf47ce4bcb1fb4d7183d1e895a044d859235230a6ebef464295baf2
user_order.json         e4c3cd7975b7dc16c4c8d014444fc2a96d927cf1b9089b33875a5450b4ff99fa
user_trade.json         042998055ec5dec2c69065d002b2619d8497faabd9bfcc36c27a1bcf7cfe224c
```

They are parser seeds only, never signing or current-protocol authority.

`../predarb` is intentionally not a clean normative checkout. At prompt
creation it has exactly:

```text
 M resources/grafana/pf-maker-v2-dashboard.json
?? .predarb/
```

Goal G MUST:

- record its actual revision and dirty path names without reading the dirty
  files;
- inspect normative tracked source only from the pinned Git object, for
  example with `git show`, `git grep`, `git ls-tree`, or a clean detached
  read-only worktree at that exact commit;
- never read, parse, copy, move, delete, reset, clean, or interpret untracked
  `.predarb/` runtime state;
- never read `.env`, `.env_bk`, private keys, API credentials, session data,
  the modified dashboard, or other untracked bytes;
- never use a tracked operational account address or credential-shaped value
  as Reap configuration or a test secret;
- never modify either sibling repository; and
- never add a Cargo path dependency on either sibling.

Predarb is a historical reference, not protocol authority and not an
architectural template. In particular, do not copy its broad adapter, public
cloneable credential structures, arbitrary hosts, automatic create-then-
derive credential fallback, allowance update, cancel-all, float/rounded
amounts, timestamp-derived salt, shared mutex state, raw response logging, or
application runtime.

## Current Official Protocol Sources

Polymarket's protocol is time-sensitive. Phase 0 MUST retrieve, pin, hash, and
record the exact relevant official bytes and source revisions used by the
implementation. The minimum current primary-source set identified during
prompt design on 2026-07-26 UTC is:

- `https://docs.polymarket.com/getting-started/api#authentication`;
- `https://docs.polymarket.com/llms.txt`;
- `https://docs.polymarket.com/api-spec/clob-openapi.yaml`;
- `https://docs.polymarket.com/v2-migration`;
- `https://docs.polymarket.com/trading/wallets-auth`;
- `https://docs.polymarket.com/trading/matching-engine`;
- `https://docs.polymarket.com/trading/place-orders`;
- `https://docs.polymarket.com/trading/manage-orders`;
- `https://docs.polymarket.com/trading/realtime-order-updates`;
- `https://docs.polymarket.com/api-reference/trade/post-a-new-order`;
- `https://docs.polymarket.com/api-reference/trade/cancel-single-order`;
- `https://docs.polymarket.com/api-reference/trade/get-single-order-by-id`;
- `https://docs.polymarket.com/api-reference/trade/get-user-orders`;
- `https://docs.polymarket.com/api-reference/trade/get-trades`;
- `https://docs.polymarket.com/api-reference/wss/user`;
- `https://docs.polymarket.com/api-reference/wss/market`;
- `https://docs.polymarket.com/api-reference/markets/get-clob-market-info`;
- `https://docs.polymarket.com/api-reference/market-data/get-order-book`;
- `https://docs.polymarket.com/api-reference/data/get-server-time`;
- `https://docs.polymarket.com/api-reference/core/get-current-positions-for-a-user`;
- `https://docs.polymarket.com/api-reference/geoblock`;
- `https://docs.polymarket.com/api-reference/rate-limits`;
- `https://docs.polymarket.com/api-reference/trading-rate-limits`;
- `https://docs.polymarket.com/resources/contracts`;
- `https://docs.polymarket.com/resources/error-codes`;
- `https://docs.polymarket.com/concepts/order-lifecycle`;
- `https://ethereum.org/developers/docs/apis/json-rpc/`;
- `https://eips.ethereum.org/EIPS/eip-20`;
- `https://eips.ethereum.org/EIPS/eip-1155`;
- `https://docs.soliditylang.org/en/latest/abi-spec.html`;
- `https://docs.polygon.technology/pos/concepts/finality/finality`;
- `https://docs.polygon.technology/pos/reference/rpc-endpoints`;
- `https://www.okx.com/docs-v5/en/#public-data-websocket-index-tickers-channel`;
- `https://github.com/Polymarket/ctf-exchange-v2`;
- `https://github.com/Polymarket/ts-sdk`;
- `https://github.com/Polymarket/clob-client-v2`;
- `https://github.com/Polymarket/polymarket-cli`;
- `https://github.com/Polymarket/py-clob-client`;
- `https://github.com/Polymarket/py-clob-client-v2`;
- `https://github.com/Polymarket/py-sdk`;
- `https://github.com/Polymarket/polymarket-sdk`; and
- `https://github.com/Polymarket/rs-clob-client-v2`.

The boundary's 128-row authoritative source manifest is the exact
Amendment 2 cutoff, including every selected path, revision, blob/content
hash, dependency-source pin, and differential-oracle identity. This minimum
URL/repository list is only a retrieval checklist; it cannot widen or
override that manifest or the boundary's explicit conflict resolutions.

Do not treat a mutable documentation URL or an unpinned package version as a
permanent contract. Prefer and hash the documented Markdown endpoints
enumerated by `llms.txt` plus the machine-readable schema, rather than relying
only on rendered Mintlify HTML. Pin official client/contract source revisions
and independently author sanitized differential vectors from exact source
bytes. Record every requested and final URL, HTTP status, content type,
retrieval time, source revision where available, content hash, relevant path,
and selected interpretation in the Goal G handoff.

Known current documentation/reference conflicts make this freeze mandatory:

- Predarb uses order-detail and trades routes that differ from current
  documentation;
- current order examples and migration diff rendering do not consistently
  show the same outer fields;
- current documentation families do not uniformly use the same status and
  response field spellings;
- rate-limit values have changed and may change again; and
- current wallet documentation has evolved since the pinned Predarb commit.

Resolve auth, signing, route, query, request-byte, identity, ABI, and mutation
contracts only with a pinned current official implementation,
machine-readable schema, standard, or independently reproduced
official-client vector. If one of those reached contracts is still ambiguous,
stop in Phase 0. The already documented lifecycle/timestamp vocabulary
conflict is different: freeze the closed source-tagged compatibility union
required by Amendment 1 and quarantine anything outside it. A disagreement
between source families is not itself a stop when every allowed raw value can
be preserved without success promotion and every other value can be boundedly
quarantined. Do not guess, probe with real credentials or a real Polygon
origin, or choose Predarb merely because it has code.

## Exact Product And Capability Boundary

Goal G extends only Goal F's existing PM product. OKX remains public crypto
reference data only. Polymarket remains the sole execution venue. Predict.fun
remains absent.

The exact capability disposition is:

| Capability | Goal G role | Mutation authority | Product disposition |
| --- | --- | --- | --- |
| OKX configured reference | Endpoint-connected public `index-tickers` WebSocket observation | None | Add a closed live transport around the existing strict source/session state |
| PM configured metadata/book | Endpoint-connected public REST/market-WS observation | None | Add a live transport around the existing strict parsers/session state |
| PM user order/fill stream | Authenticated user-WS observation | None | Add exact configured account/condition subscription |
| PM open orders | Authenticated reconciliation read | None | Complete bounded unfiltered credential-visible inventory; no funder-wide claim |
| PM exact order detail | Authenticated reconciliation read | None | Exact configured-account/market venue identity needed for owned, ambiguous, or unmanaged exposure reconciliation |
| PM trades/fills | Authenticated reconciliation read | None | Complete bounded pagination and exact maker/taker leg linkage |
| PM collateral/token balance | Authenticated CLOB account read | None | Exact configured account/assets |
| PM CLOB spendability/cache values | Authenticated CLOB account read | None | Numeric per-selected-spender evidence retained separately; diagnostic/fail-close only |
| PM ERC-20 allowance and ERC-1155 approval | Credential-free closed Polygon read | None | Direct finalized-block `allowance(owner, exchange)` and `isApprovedForAll(owner, exchange)` for the frozen Goal F domain |
| PM published positions | `reap-polymarket-public-source` narrow Data API observation | None | Exact lexical parsing; monitored separately from authoritative balance/fill state |
| PM server time | `reap-polymarket-public-source` public readiness read | None | Clock-offset/skew evidence only |
| PM geographic availability | `reap-polymarket-public-source` public safety read | None | New-submit fail-close input only |
| PM place | Owned EOA execution | One GTC post-only profile | Exact prepared quote only |
| PM cancel | Owned EOA execution | Exact cancel-owned | Proven locally owned venue order only |
| PM recovery cleanup | L2-only recovery/cancel composition | Exact cancel-owned | Reconcile and cancel known owned orders without loading the order signer |

No role exposes a generic HTTP or JSON-RPC method, path, query, body,
WebSocket message, contract address, calldata, block tag, provider selector,
raw response, request signer, order signer, credential accessor, raw client,
endpoint selector, batch call, transaction method, or generic exchange
command.

The authenticated product admits only one configured PM condition/market/token
mapping and the exact OKX public references declared by its explicit model.
Do not broaden Goal G into dynamic market discovery, a multi-account manager,
an arbitrary token registry, or a universal venue runtime. Preserve the
existing compact configured handles and bounded collections.

## Frozen Account And Order Profile

Goal F deliberately froze an EOA-only unsigned profile. Goal G completes that
same profile and no other:

- Polygon chain ID `137`;
- CLOB V2;
- `signatureType = 0`;
- configured EOA `maker == signer == funder == L2 POLY_ADDRESS`;
- a pre-provisioned L2 credential bundle bound to that exact EOA;
- outer POST `owner` equal to the L2 API-key UUID, never the maker/funder
  address;
- the standard or negative-risk V2 verifying contract selected from the
  pinned current official V2 contract registry/source and the exact
  configured token's strict public `neg_risk` evidence, with any conflict
  failing closed;
- the same selected exchange is the only Polygon allowance spender/operator;
- the Polygon authorization owner is the configured EOA/funder, never the
  outer L2 API-key UUID;
- the ERC-20 target is Goal F's frozen pUSD contract and the ERC-1155 target
  is Goal F's frozen Conditional Tokens Framework contract, both obtained
  from the already validated `PmGoalFTradingDomain`, never caller-supplied;
- `GTC`;
- `postOnly = true`;
- `deferExec = false`;
- `expiration = 0`;
- `metadata = bytes32(0)`;
- `builder = bytes32(0)`;
- exact integral maker/taker protocol amounts already checked by Goal F;
- executable price strictly in `(0, 1)` and aligned to current tick metadata;
- quantity aligned to exact lot/minimum metadata; and
- cancellation of an exact venue-order identity carrying canonical local
  ownership proof.

Proxy, Gnosis Safe, deposit-wallet/POLY_1271, session signer, remote signer,
nonzero builder, nonzero metadata, another chain, another order type, or
another identity relationship is a different account/signature goal. Do not
add a dormant enum variant or generic implementation for it.

If current official evidence no longer proves that the fixed EOA/type-0
profile is supported, stop. Do not silently convert Goal F's persisted
identity or unsigned-order semantics to a newer wallet profile.

This type-0 EOA profile does not make Predarb's historical proxy/type-1
operational account usable and does not provision a new current Polymarket
account. Goal H must present an already-valid direct EOA with the required
pUSD/outcome tokens and approvals, or a separate wallet-profile goal must
explicitly supersede this profile.

## Authentication And Provisioning Boundary

Normal authenticated trading consumes:

1. one pre-provisioned L2 API key;
2. one L2 base64 HMAC secret;
3. one L2 passphrase; and
4. the one EOA private key used to sign each order.

Goal G implements only the authentication that the reached trading runtime
uses:

- exact L2 request HMAC construction and headers;
- exact authenticated user-WS subscription construction; and
- exact CLOB V2 EOA order EIP-712 signing.

The Polygon authorization source is not L1 or L2 authentication,
provisioning, or signing. It is a credential-free read-only safety source. It
cannot access the EOA key, L2 bundle, auth headers, signed body, prepared
effect, or mutation authority.

L1 `ClobAuth` belongs to the later provisioning plane. Phase 0 may record its
current contract as background needed to interpret official credential
identity, but a vector neither proves that an injected L2 bundle is bound to
an account nor grants runtime provisioning authority. API-key creation,
derivation, listing, deletion, rotation, and credential-file writing are
separate operator provisioning capabilities. Goal G MUST NOT implement or
call `POST /auth/api-key`,
`GET /auth/derive-api-key`, or another API-key mutation/read in the normal PM
product. It MUST NOT copy Predarb's automatic create-first/fallback behavior.

The later credential-provisioning goal may reuse the frozen L1 vectors, but it
must remain a separate executable and authority plane. This separation is
intentional: a quote strategy needs authenticated requests and order signing,
not permission to create or rotate credentials.

## Secret And Transport Security Contract

Credential and key material MUST:

- enter only at the authenticated composition root after non-secret config
  validation, authenticated journal lease acquisition, and local durable
  recovery; venue reconciliation occurs after loading the narrow L2 bundle
  and before loading the EOA signer;
- use bounded input buffers and non-`Clone`, non-`Copy`, non-`Debug`,
  non-`Display`, non-`Serialize`, zeroizing holders;
- be owned by one account-scoped authentication/signing edge owner;
- be inaccessible to PM core, state, strategy, quote policy, coordinator,
  journal, capture, replay, evidence, telemetry, configuration projection, and
  public DTOs;
- never appear in CLI arguments, environment dumps, normal config files,
  URLs, query strings, errors, panic messages, logs, metrics, captures,
  journals, reports, snapshots, fixture provenance, or hashes presented as
  credentials;
- never be returned through getters or a general signing oracle;
- never have its user-WS authentication frame captured as raw input;
- be cleared from every Reap-owned transient header, request, and subscription
  buffer as soon as practical after use; and
- be dropped and zeroized from every Reap-owned holder during bounded
  shutdown, without claiming erasure from unavoidable third-party
  crypto/TLS/HTTP, allocator, OS, or kernel copies or protection from a
  compromised host, swap, core dump, DMA, or privileged process.

An internal immutable shared secret vault is allowed only if Phase 0 proves
that it is private to the adapter, zeroized on final drop, contains no
canonical trading state, and exposes only closed purpose-specific operations.
It must not become a public `Arc<Credentials>`, an `Arc<Mutex<_>>`, or a
cloneable session.

Production credential sources are not selected without a target-host
contract. Goal G may implement a narrow injected reader/descriptor boundary
and deterministic test source. It MUST NOT invent an environment-variable,
plaintext TOML, repository file, home-directory file, or cloud-KMS contract.
Goal H must select and qualify the real source. Goal E may supply separate
target-host evidence but does not own secret-source semantics or authority.

Every live exchange public or credentialed transport MUST use a closed
production origin allowlist plus only the constrained `local-evidence`
loopback seam:

- exact HTTPS/WSS schemes and official production hosts;
- redirects disabled;
- ambient proxy discovery disabled;
- no userinfo, alternate port, arbitrary base URL, downgrade, custom
  certificate bypass, or credential forwarding across origins;
- bounded connect, handshake, read, write, idle, and total deadlines;
- bounded request, response, frame, field, page, and normalized-expansion
  sizes; and
- centralized lower-than-official per-host and per-credential pacing with
  explicit queue-age limits.

Official exchange maxima are ceilings that can change, not Reap throughput
targets. Do not hard-code the published maximum as Reap's normal rate.
Default and production constructors/features MUST have no custom-origin or
loopback injection surface. The sole exception is a non-default
`local-evidence` feature on the relevant edge crates and a forwarding feature
on evidence roots. It may expose only constructors that accept an already
parsed numeric loopback socket address (`127.0.0.0/8` or `::1`), reject DNS,
non-loopback addresses, redirects, proxies, and production TLS origins, and
remain absent from every default feature, normal dependency edge, deployable
binary, and service build. Source/dependency-policy tests must prove those
limits. This feature exists only because Rust integration tests and external
bench targets compile the library without `cfg(test)`; it is not a general
custom-origin feature.

Every external integration-test or bench target that uses the seam declares
`required-features = ["local-evidence"]`; the explicit commands below enable
it, while the default workspace/global gate skips those targets and reruns
them through the frozen feature-enabled commands.

The Polygon source has no selected production origin in Goal G. Its protocol
implementation accepts only a privately bound origin transport and exposes no
provider or URL selector. Goal G provides only loopback-address-validated
`local-evidence` construction. Goal H must add the production composition for
one exact reviewed HTTPS origin, with redirects, ambient proxies, userinfo,
query credentials,
alternate ports, downgrade, and trust bypass rejected. If the selected
provider needs an API key or header secret, Goal H must define its custody;
Goal G neither invents nor accepts provider credentials. Goal H must prove
chain `137`, JSON-RPC `2.0`, provider-reported `finalized`, historical
exact-block calls, response/time/rate bounds, and disciplined host wall-clock
behavior before constructing the role.

## Exact Signing And Request-Byte Contract

Phase 0 must freeze and Phase 2 must prove:

- exact CLOB V2 order EIP-712 domain name, version, chain, and both reached
  verifying contracts;
- exact signed type string, field order, field widths, side encoding,
  timestamp unit, metadata, builder, and signature encoding;
- exact relation between signed order digest, local expected venue-order ID,
  and returned `orderID`;
- exact JSON integer/string representation and field spelling for every order
  and outer-body field, including outer `owner == L2 API-key UUID`;
- exact L2 message
  `timestamp + UPPERCASE_METHOD + signed_route_path + exact_body_bytes`;
- exact canonical padded base64url credential grammar, strict
  decode/re-encode equality, local decoded-size cap, HMAC-SHA256, and exact
  44-byte padded base64url output;
- exact lowercase 36-byte UUID grammar with no invented version/variant
  restriction, the Reap-local passphrase grammar/cap and raw-header/
  compact-JSON lowering, and canonical lowercase prefixed 32-byte private-key
  text with scalar/range/re-encode/configured-address checks;
- whether and how query parameters are excluded from the signed route for
  each reached read;
- the same timestamp and exact body bytes in the HMAC, headers, and transport;
  exact `Accept`, `Accept-Encoding`, body `Content-Type`, five `POLY_*`
  application headers, and absence of auth/JSON-body headers where forbidden;
  and
- that the exact final serialized HTTP body byte slice HMAC-signed by L2 is
  the byte slice transported, not a second serialization of an equivalent
  object.

Do not conflate the two signatures. EIP-712 signs the typed-order digest, and
the embedded order fields plus EOA signature must match that digest. L2 HMAC
separately authenticates the exact final serialized HTTP request body bytes.
The EOA typed-order digest and expected `orderID` must independently reproduce
the pinned Solidity `Structs.sol` order typehash/field layout and
`Hashing.sol::hashOrder` EIP-712 hash; agreement between SDK clients alone is
insufficient.

Wire lowering validates and serializes only. It never rounds, fills a default,
changes field order after signing, regenerates a timestamp, rewrites a query,
or normalizes equivalent JSON.

Use unmistakably public deterministic test-vector keys and synthetic L2
credentials not bound to any known real account. Compile-time/test-origin
controls must make them usable only with owner-local loopback. Never claim
that a valid EOA key is intrinsically incapable of later real authorization.
Require independent agreement with pinned current official TypeScript and/or
Rust client source for:

- L2 GET without a body;
- L2 POST with an exact body;
- EOA order signing for the standard domain;
- EOA order signing for the negative-risk domain;
- buy and sell amount orientation;
- exact post-only outer body;
- exact cancel-owned body; and
- expected venue-order identity.

Predarb's L1 parity vector is a useful historical cross-check. Predarb has no
sufficient exact official CLOB V2 order-signature golden and cannot be the
only authority.

## Timestamp, Salt, And Idempotency Contract

PM exposes no reached wire client-order-ID field. The existing PM client order
key remains journal-local and MUST NOT be invented as a request field.

The authenticated product must maintain:

- one durable local intent identity;
- one JSON-safe exact salt with collision proof across retained journal
  history;
- one valid bounded-current CLOB V2 order timestamp in milliseconds;
- one exact signed-order digest;
- one exact request-body SHA-256 commitment; and
- one expected venue-order identity when the current protocol proves it.

Do not derive salt solely from wall time. Audit Goal F's current salt and
action-sequence recovery before reusing it. The durable salt/intent identity
must be strictly unique across retained history. The boundary's outbound PM
clock algorithm is exact and normative: a fresh checked `/time` seconds anchor
plus floored monotonic elapsed milliseconds creates the conservative
`pm_now_ms`; one new order persists its exact canonical 13-digit millisecond
timestamp, while each final authenticated request uses canonical ten-digit
`floor(pm_now_ms/1000)` in both HMAC and header. The immediate pre-write check
requires the same-second L2 value, a signed-order value no more than 30
seconds old/five seconds future, and every earlier grant/geoblock deadline.

Current pinned sources prove no venue timestamp monotonicity/high-water rule.
Goal G therefore persists no timestamp high-water and never clamps or bumps a
timestamp. Within one in-memory clock epoch, compare a candidate at receipt
against the old live anchor projected to that same monotonic instant: reject
only when the candidate second is less than
`floor(old_pm_now_ms(receipt_m)/1000)`; equal seconds are allowed. A process
restart or clock transport/configuration epoch change discards comparison
state. Rollback rejection discards both anchors. Overflow, seconds-boundary
crossing, unavailable/expired anchor, or excessive skew makes clock readiness
false and sends no byte. The signed-order timestamp/body is never
regenerated. For mutations, a new L2 timestamp requires a separately
committed mutation commitment/grant. For reads, the unsent attempt is
discarded and only the capped coordinator read cycle may construct a wholly
fresh timestamp/HMAC attempt. Owned cancellation and reconciliation
obligations remain durable, but no authenticated request can be constructed
until the clock is ready.

Amendment 1's compatibility union applies only to inbound venue fields. It
does not change the fixed millisecond signed-order timestamp or seconds L2
authentication timestamp. Phase 0 must freeze, per exact REST/WS
source/message family and field, the JSON lexical kind, allowed raw status
tokens, allowed timestamp unit, semantic meaning, and whether the timestamp
is optional. A field may accept canonical 10-digit Unix seconds, 13-digit Unix
milliseconds, and/or another exact documented lexical form such as RFC 3339
only when its own source table explicitly says so; conversion to canonical
milliseconds uses checked arithmetic and a bounded current-time range. There
is no global magnitude-guessing timestamp parser, no cross-family ordering by
venue time, and local receive time is never relabeled as venue time.

## Request Durability And Ambiguity Contract

Preserve Goal F's durable-intent-before-effect ordering and strengthen it for
secret-side request preparation:

```text
coordinator approves and reserves
-> canonical intent is durably committed
-> signer creates exact signed order/body at the edge
-> edge returns a non-secret request commitment
-> commitment and dispatch-authorized/may-have-sent barrier are durably committed
-> one take-once dispatch grant returns to the edge
-> exact committed bytes receive one application dispatch attempt
-> typed result and post-result fact return to the coordinator
```

The adapter cannot send before the coordinator consumes the exact durable
dispatch grant and invokes its closed effect port. "One dispatch attempt"
means Reap performs no second application-level mutation send; TCP/TLS/HTTP
may partially write or retransmit below that boundary, and Phase 0 must audit
the selected client's lower transport behavior while explicitly configuring
zero HTTP-layer retries.
The coordinator never receives the private key, credentials, auth headers, or
a raw signed request. The journal records only the minimum non-secret identity
and commitments required for recovery; it never records replayable auth
headers, user-WS credentials, a private key, L2 secret/passphrase, or a full
signed order body.

The dispatch grant binds the exact method, route, query contract, body
commitment, auth timestamp, expected order identity, and, for placement, the
geoblock permit identity/epochs. Its monotonic send-before is the earliest of
the 250-millisecond dispatch deadline, five-second geoblock lease, and other
frozen deadlines. If pacing, fsync, queueing, clock seconds-boundary,
geoblock, or scheduling makes it stale before the first application write,
the edge MUST NOT send or regenerate headers under the same grant. It returns
a typed definitely-not-dispatched result for durable reduction.
Recovery conservatively classifies every consumed or durably granted
dispatch-authorized/may-have-sent barrier without a conclusive post-result
fact as acknowledgement-unknown.
A fresh auth timestamp/grant may be prepared only after the coordinator
durably records a definitely-not-dispatched result. It must retain the same
canonical intent and EIP-712 order/body; it is forbidden once any application
write may have occurred.

Classify at least:

- accepted-live POST result;
- known but out-of-profile POST `matched`, `delayed`, and `unmatched`
  results, retaining all bounded returned identities and forcing
  reconciliation without success promotion;
- separately tagged ordinary order and trade lifecycle facts from their exact
  REST/user-WS source families;
- definite rejection with a typed bounded reason;
- authentication failure;
- rate limited/too early/cancel-only/unavailable;
- protocol violation; and
- acknowledgement unknown.

The boundary's mutation table is exhaustive for HTTP status, exact JSON
shape, duplicate/unknown keys, POST cross-field combinations, expected order
identity, cancel `canceled` array/`not_canceled` map, documented duplicate and
timeout cases, and every unexpected `2xx`/`3xx`/`4xx`/`5xx`. Implementation
and tests may not replace it with a generic success/error deserializer or
infer success from HTTP status alone.

Parse a closed source/message-family-tagged union at the edge. At minimum,
keep these namespaces non-interchangeable:

- lowercase POST results `live|matched|delayed|unmatched`;
- REST order tokens, including the reached prefixed
  `ORDER_STATUS_*` family and the exact reached unprefixed `LIVE` alias;
- user-WS order occurrence types `PLACEMENT|UPDATE|CANCELLATION`; and
- REST/WS trade-settlement tokens, including reached `TRADE_STATUS_*` and
  unprefixed
  `MATCHED|MATCHED_NOT_BROADCASTED|MINED|CONFIRMED|RETRYING|FAILED`
  families, subject to Amendment 2's exact route-specific table.

Phase 0 records the exact allowed token for every route/envelope and each
explicitly proven semantic equivalence. Raw family, raw token, and timestamp
unit provenance remain available to bounded diagnostic/quarantine evidence.
`MATCHED` in a POST response, order state, and trade settlement is never one
implicit enum value. Only POST `live` is ordinary fixed-profile placement
acceptance. POST `matched|delayed|unmatched` and user-WS order
`DELAYED|UNMATCHED` are known but out-of-profile and halt placement while
retaining authority/reservations and forcing reconciliation. REST/user-WS
trade `Matched` is instead an ordinary provisional settlement fact, and the
REST-only `MatchedNotBroadcast` is its distinct earlier settlement fact;
neither is a POST acceptance. Unknown, malformed, cross-family, or ambiguous
values are boundedly quarantined; they are never mapped to pending, open,
zero, or success. Only a complete source-compatible reconciliation cut may
clear a quarantine. Capacity exhaustion or permanently ambiguous identity
becomes a durable operator-required halt. Quarantine stores only sanitized
bounded field/family/identity evidence, never a credential-bearing raw frame,
auth header, secret, or unbounded raw error body. Do not add marketable-order
execution semantics merely because the venue vocabulary contains them.

Raw user-WS order occurrence/status/quantity combinations use the exact
four-row live-only table in the boundary. Occurrence is authority, optional
status only corroborates, and omission remains omission. Any mismatched
cross-product invalidates the private epoch and reconciles; it does not widen
Goal F's fake parser or fixture schema.

An HTTP timeout, disconnect, partial response, malformed success, process
death after durable dispatch grant, or response loss after server acceptance
is acknowledgement-unknown. It is never ordinary rejection.

Fresh-attempt and mutation rules are:

- every HTTP client has zero retries;
- eligible reads use only the boundary's coordinator-owned three-attempt,
  30-second fresh-cycle/cooldown contract and route pacing;
- placement is never reissued after bytes may have reached the venue;
- an ambiguous place locks the quote slot and forces exact order/open-order/
  trade reconciliation;
- cancellation may be reissued only for the identical proven-owned venue
  order after a complete read-only reconciliation still proves it live, under
  a separately durable request commitment, fresh HMAC/timestamp, and new
  take-once grant;
- a duplicate-order response triggers exact identity reconciliation, never a
  new timestamp/body; and
- if expected venue identity or an unambiguous exact match cannot be proven,
  the slot remains halted for operator resolution.

Immediate response trade IDs are reconciliation evidence. Do not fabricate
fill price, amount, cumulative quantity, fee, maker leg, or position from a
status string alone.

## Authenticated Read And Position Contract

Use separate, non-interchangeable roles for:

1. user order/fill WebSocket observation;
2. open-order and exact-order reconciliation;
3. trade/fill reconciliation;
4. authenticated CLOB collateral/token and numeric spendability/cache reads;
5. credential-free finalized-chain ERC-20 allowance/ERC-1155 approval reads;
6. published position observation; and
7. owned order execution.

The endpoint-connected public role reuses Goal F's strict public metadata,
book, integrity, session, capture, and normalized-event contracts. It adds
only the exact configured metadata REST and market-WS network supervision,
closed routes, bounded loopback-tested transport, and owner-bound live
delivery. Public endpoint connection was not implemented by Goal F and is not
treated as already complete.

The user-WS role:

- sends exactly one credential-bearing initial frame per connection epoch and
  no dynamic subscribe/unsubscribe/update frame; a configuration lifecycle
  change closes the session and reconnects with a new epoch;
- subscribes only to the configured condition/account scope;
- parses only the raw `/ws/user` wire family, framed as either one event
  object or a bounded array of event objects, with exact inner
  `event_type = order|trade`; SDK-normalized `topic/type/payload`,
  camel-case, and RFC-3339 projections remain differential oracles only;
- never invents a subscription acknowledgement; if the frozen protocol has
  none, authenticated readiness requires the exact protocol-proven evidence
  plus complete REST reconciliation;
- verifies every documented authentication failure and close shape;
- maintains explicit connection epochs, ping/pong, handshake, idle, and
  reconnect state;
- never blocks its socket read/ping loop indefinitely on a full downstream
  queue;
- fails closed and forces full reconciliation on overflow, parse failure,
  input outside the frozen compatibility union, unproven gap,
  authentication failure, or
  reconnect; and
- quarantines bounded pre-mapping or ambiguous order/fill observations rather
  than silently dropping them.

Open orders and trades must exhaust bounded pagination to a proven terminal
cursor. Partial pages never claim completeness. Order detail is requested only
for an exact configured-account/market venue identity needed to resolve
expected, owned, ambiguous, or unmanaged exposure. Reading the exact ID never
promotes ownership: unknown/unmanaged remote orders remain unmanaged and are
never claimed or cancelled.

For account readiness:

- read collateral and the configured outcome-token balance;
- retain every exact selected-spender CLOB numeric value independently as
  cache/diagnostic evidence;
- require one fresh finalized-chain cut containing the exact ERC-20
  `allowance` amount and exact ERC-1155 `isApprovedForAll` boolean for the
  configured EOA and selected standard/negative-risk exchange;
- never take the first allowance map entry;
- never convert a positive, maximum, or otherwise numeric CLOB conditional
  value to boolean approval, and never convert chain approval to an amount;
- allow an exact source-proven CLOB numeric value to add an independent
  insufficient-spendability fail-close fence, but never to grant readiness;
- never treat a balance-cache update endpoint as a read;
- never mutate allowance or approval;
- include all canonical configured-product reservations in collateral
  availability, without claiming visibility into another credential/process;
  and
- make missing, stale, wrong-kind, false, insufficient, conflicting, or
  partial evidence unready.

The public Data API position endpoint may feed a separate published-position
projection using exact lexical numeric parsing and `sizeThreshold = 0`. It
MUST NOT use `f64`. Current documentation does not prove an atomic snapshot
cut, so Data API absence or numeric equality alone cannot establish zero,
grant sell authority, or release a reservation. Unless Phase 0 pins stronger
official completeness semantics, trading readiness uses exact CLOB token
balance plus canonical confirmed fills and reservations; the published
position remains monitored evidence whose divergence fails closed.

Goal F's `PmCompleteAccountSnapshot` is a frozen fixture carrier whose
completeness includes account, allowance, and position. Live code MUST NOT
fabricate it from separately timed CLOB, Polygon, and Data API replies.
Introduce a
source-neutral live account cut for exact CLOB collateral/token balances and
all per-spender numeric cache/spendability values that makes no typed
operator-approval or position-completeness claim; a separate block-bound
`PmPolygonAuthorizationCut`; and a separate
`PmPublishedPositionObservation`/`PmPublishedPositionSnapshot` projection.
Feed all through explicit reducers/readiness joins while keeping the Goal F
carrier and bytes unchanged.

Assemble multi-endpoint account/reconciliation state through request-boundary
and private-epoch cut semantics. Do not label separately timed HTTP replies as
one atomic venue snapshot. Preserve later user-WS facts when applying a
completed earlier cut.

## Closed Polygon Authorization Contract

`PM-LIVE-POLYGON-AUTHORIZATION-CUT` is one indivisible, credential-free,
read-only source. It is owned by `reap-polymarket-chain-source`, not by the
authenticated CLOB adapter. It performs exactly this sequence:

1. at the start of every cut, `eth_chainId` must return canonical `0x89`; a
   mismatch also invalidates the transport epoch;
2. `eth_getBlockByNumber("finalized", false)` selects one non-null anchor
   `{number, hash, timestamp}`;
3. at that exact hexadecimal block number, one `eth_call` to Goal F's pUSD
   contract executes `allowance(owner, selectedExchange)` using selector
   `0xdd62ed3e`;
4. at the same exact block number, one `eth_call` to Goal F's Conditional
   Tokens contract executes `isApprovedForAll(owner, selectedExchange)` using
   selector `0xe985e9c5`; and
5. `eth_getBlockByNumber(exactNumber, false)` must return the same number and
   hash as the anchor.

The owner is the configured EOA/funder. The exchange is selected only by the
already validated Goal F standard/negative-risk domain. Contract addresses,
owner, exchange, selectors, two left-zero-padded address arguments, block tag,
JSON-RPC methods, request IDs, and call order are constructed by closed
private code; a caller cannot supply or alter them. Batching and fallback to
`latest`, `safe`, `pending`, or a different block are forbidden.
The frozen targets are pUSD
`0xC011a7E12a19f7B1f670d46F03B03f3342E82DFB` and Conditional Tokens
`0x4D97DCd97eC945f40cF65F87097ACe5EA0476045`; the standard and
negative-risk exchanges are the two Goal F V2 addresses already frozen above.
Each transport request is one bounded JSON-RPC `2.0` POST object with a
deterministic nonzero integer ID and no notification or batch; the response
must carry `jsonrpc = "2.0"` and the exact matching ID.
The boundary's five compact request templates, member order, lowercase
quantity/address/calldata grammar, `word(address)` construction, exact
request-body reuse, application headers, top-level response union, and
per-call result shapes are normative. No generic serializer choice or
provider-specific request extension is left to implementation.

Each `eth_call` result must be exactly one 32-byte ABI word. ERC-20 allowance
is an unscaled `U256`. ERC-1155 approval is canonical ABI boolean: all zero
for false, or all zero except a final byte of one for true. Empty, short,
long, trailing, non-hex, overflowed, or noncanonical-boolean results are
rejected.
The complete fact binds chain, finalized block number/hash/timestamp, local
monotonic observation time, transport epoch, account/configuration epoch,
owner, exchange, both token contracts, typed allowance, and typed approval.

The anchor timestamp may be at most five seconds in the future and at most
thirty seconds old against the disciplined local wall clock. A completed cut
expires after five monotonic seconds and immediately on transport,
account/configuration, or market-metadata epoch change. These are conservative
Goal G safety bounds, not Polygon protocol promises or target-host SLOs.
Wrong chain, null block, revert, JSON-RPC error, ID mismatch, hash change,
stale/future block, timeout, redirect, proxy use, oversized or malformed
response, unsupported finalized/historical call, or any partial sequence
discards the whole cut and makes account readiness false. Retrying starts a
fresh whole cut; no partial result is reused.

Clock access is a private edge capability, not caller-supplied canonical state.
`local-evidence` uses a fixed deterministic synthetic wall/monotonic clock for
freshness and replay tests. Goal H must bind reviewed target-host clock
discipline. Process-monotonic instants are never serialized, journaled, or
compared across restarts; only typed freshness and epoch transitions enter the
canonical durable projection.

The CLOB account cut and Polygon cut are not an atomic venue snapshot.
Readiness joins independently fresh facts only when account, market,
instrument, owner, selected exchange, and configuration epochs match. The
Polygon cut is typed readiness authority; the CLOB conditional numeric value
cannot substitute for its boolean. Recovery may inspect a fresh cut, but
exact-owned cancellation does not depend on a positive approval, and
placement cannot reopen without a new valid cut.

The source exposes no generic JSON-RPC request, method, address, calldata,
block tag, raw response, transaction, signer, `approve`,
`setApprovalForAll`, `eth_send*`, CTF operation, or provider selector. Goal G
implements only `local-evidence` loopback construction and makes no real
Polygon request.
Goal H supplies and qualifies the one exact non-test HTTPS origin.

## Shutdown And Recovery Safety Contract

Goal G deliberately excludes the credential-wide order heartbeat. The fixed
strategy does not call it, and current official documentation and client
implementations disagree on its route/body/response and ID lifecycle. Do not
guess, add a dormant heartbeat role, or broaden exact-owned cancellation into
a credential-wide command. A later separately scoped safety goal may add it
only after the current protocol and exclusive credential scope are proven.

Goal G instead requires a narrow L2-only recovery composition that can
reconcile the complete unfiltered inventory visible to the injected L2
credential, identify only journal-proven locally owned orders, and cancel
those exact identities without loading the EOA order signer or enabling
placement. Unmanaged credential-visible orders make placement unready and
remain uncancelled. This is not funder-wide completeness: another API key, UI,
process, manual action, or on-chain actor sharing the EOA may remain invisible.

Normal and fatal shutdown use one bounded path:

```text
halt new placement
-> retain private/reconciliation/exact-owned-cancel capability
-> cancel every proven owned live order
-> reconcile order/fill/account state
-> prove no owned live order or report exact unresolved identities
-> close network roles
-> zeroize secrets
-> release journal lease
```

## Target Dependency And Ownership Shape

Use this responsibility shape:

- `reap-polymarket-auth`:
  secret holders, exact EOA CLOB V2 order signing, expected-order identity,
  exact L2 HMAC/header creation, and no network or strategy;
- `reap-polymarket-public-source`:
  mechanically extracted strict configured PM lifecycle/CLOB metadata, book,
  integrity, public session, REST snapshot and market-WS transport, plus the
  narrow Data API position observation, server-time, and geographic-
  availability reads; it is credential-free and has no private/account/order
  mutation role;
- `reap-polymarket-chain-source`:
  the closed credential-free Polygon chain/anchor/allowance/approval protocol,
  private bounded JSON-RPC/ABI DTOs, and the indivisible finalized-block
  authorization cut; it has no auth, signer, arbitrary RPC, transaction,
  public-market, strategy, or canonical-state role and no Goal G default or
  production origin constructor;
- `reap-polymarket-live-adapter`:
  closed authenticated private REST/user-WS transports, strict bounded
  response parsing, CLOB account/reconciliation roles, GTC-post-only place,
  owned cancel, and no chain/public-market duplication, economic model, or
  canonical state;
- `reap-pm-authenticated-mutation-journal`:
  distinct non-secret authenticated schema V1, lease, durable barriers,
  request commitments, typed post-results, and recovery projection on
  `reap-durable-writer`/`reap-pm-core`; it has no credential, signer, network,
  request-construction, Goal F journal-reuse, or canonical-state role;
- existing `reap-polymarket-wire`:
  remains credential-free; it may gain strict real public/private/order
  response DTO parsing and non-secret unsigned values, but no full signed
  outer order body, API-key `owner`, order signature, private key, API secret,
  auth header builder, network client, or signer;
- existing `reap-polymarket-adapter`:
  retains fixture/fake roles only; its current public implementation, callers,
  and tests move mechanically to `reap-polymarket-public-source`, with no
  compatibility re-export or dependency that would give the fixture/fake
  crate a network/auth capability;
- existing `reap-okx-public-source`:
  gains only the endpoint-connected configured public `index-tickers`
  WebSocket transport, reusing its strict session/parser state and never
  importing or depending on `reap-okx-live-adapter`;
- existing `reap-pm-core`, `reap-pm-state`, and `reap-pm-strategy`:
  remain pure and credential/network-free;
- existing `reap-pm-live-contracts`:
  gains secret-free authenticated plan/route requirements without accepting
  credentials or arbitrary endpoints; and
- existing `reap-pm-live`:
  retains the sole coordinator/state owner, consumes every take-once prepared
  effect and durable dispatch grant itself, composes both credential-free
  public sources, and statically invokes the linear authenticated adapter
  through a closed lower-level effect port.

The authenticated live adapter MUST NOT depend on `reap-pm-live` or
`reap-polymarket-adapter`. The default acyclic shape is:

```text
reap-pm-live
  -> reap-polymarket-public-source -> wire/core/transport
  -> reap-okx-public-source -> core/transport
  -> reap-polymarket-chain-source -> core/transport
  -> reap-polymarket-live-adapter -> auth/wire/core/transport
  -> reap-pm-authenticated-mutation-journal -> durable-writer/pm-core
  -> reap-polymarket-adapter -> wire/core/transport
```

To preserve that DAG, edge crates do not depend on
`reap-pm-live-contracts`. They emit only closed untagged typed facts/results;
`reap-pm-live` alone attaches the stable requirement ID and its one canonical
lane at composition. An edge cannot select, alias, or multiply requirement
identity.

The authenticated adapter never receives `PreparedPmQuote`,
`PreparedPmCancel`, or a coordinator dispatch-grant type. `reap-pm-live`
consumes those upper-layer authorities and lowers their exact facts into a
method on one already-bound linear adapter role. An additional thin upper PM
runtime/composition crate is allowed only if Phase 0 proves that the listed
graph cannot express the fixed worker composition without a cycle; it
contains composition, not business logic, and must not make upper authority
public.
Do not put PM auth into `reap-live`, `reap-order`, `reap-venue`, the Chaos
runtime, or `reap-cli`.

The current `PreparedPmQuote`/cancel path embeds fake-specific command data and
current negative tests prove no live mutation consumer exists. Goal G must
replace that exact boundary deliberately:

- first introduce a narrow, backend-neutral, take-once prepared PM effect;
- migrate the fake backend and prove identical Goal F behavior;
- then add one statically selected live consumer;
- keep constructors private/guarded so callers cannot forge preparation or
  dispatch authority; and
- replace old negative tests with stronger tests rejecting signer/client/raw
  request access and backend mixing.

Do not merely delete the old compile-fail guards, make fields public, add a
runtime backend enum, or let the live adapter call coordinator internals.

The coordinator continues to own by value all canonical book, order, fill,
position, reservation, readiness, risk, schedule, and model state. Network,
parsing, signing, persistence serialization, and telemetry remain bounded
edge work. There is:

- no socket/file IO, secret access, JSON, crypto, blocking work, or network
  `.await` in the canonical reducer;
- no `Arc<Mutex<_>>`/`RwLock` canonical state;
- no task per order, cancel, fill, or timer;
- no new hot-path trait-object dispatch; and
- no second shadow order or position owner in the adapter.

## Event Lanes And Backpressure

Extend Goal F's deterministic lane contract without erasing its priorities.
The exact ranking and capacity table is frozen in Phase 0 and must include:

- critical authentication/safety failure;
- private order/fill occurrence;
- place/cancel result and acknowledgement-unknown;
- durable request-preparation/dispatch acknowledgement;
- reconciliation completion;
- CLOB account, finalized-chain authorization, and position completion;
- public PM and OKX reference state;
- scheduled quote/cancel evaluation; and
- telemetry.

Every lane is bounded and has semantic overflow behavior:

- private order/fill, mutation results, durability acknowledgements, and
  safety events are lossless within capacity or stop/fail closed;
- user-WS ingress overflow terminates the epoch and forces reconciliation;
- reconciliation/account pages are complete or discarded as incomplete;
- a partial or stale finalized-chain cut is discarded as a unit and never
  latest-wins/coalesced into approval;
- no sequence/integrity-bearing fact is silently dropped or latest-wins;
- a full command lane disables new submit before it can lose an approved
  effect;
- telemetry alone may sample/coalesce; and
- repeated identical input produces the same ordered canonical projection.

Separate the socket reader/ping owner from bounded normalization delivery so a
slow reducer cannot silently starve protocol liveness. Bounded overload must
produce an explicit reconnect/resync or stop, not unbounded buffering.

## Non-Negotiable Invariants

Preserve exactly:

- every completed Goal A through Goal F behavioral, deterministic, numeric,
  ownership, fake-product, and serialized-artifact boundary except the exact
  backend-neutral effect seam, credential-free live public roles, private
  authenticated roles, source-neutral live ingress, distinct live
  plan/requirement IDs, and distinct authenticated journal family explicitly
  authorized here; every existing Goal F artifact/schema byte remains
  individually unchanged;
- strict PM executable prices in `(0, 1)` and exact tick, lot, minimum, and
  six-decimal protocol amounts without `f64` or rounding;
- one PM canonical mutation/state owner;
- durable intent and dispatch evidence before possible network effect;
- take-once quote/cancel/dispatch authority;
- exact owned cancellation and unmanaged-order quarantine;
- individual fill deltas, structural fill deduplication, monotonic cumulative
  progress, exact maker-leg linkage, and settlement-state validation;
- separate published, fill-derived, balance, reservation, and readiness
  evidence;
- separate CLOB numeric spendability/cache and finalized-chain typed
  authorization evidence, with no numeric-to-boolean conversion;
- bounded queues, deterministic equal-time ordering, and explicit saturation;
- adapter-private wire and auth internals;
- no broad adapter, generic command, arbitrary request surface, or dynamic
  venue/plugin registry;
- no mutation blind retry;
- no unknown status mapped to pending, normal, zero, or success;
- no source-family/status/timestamp provenance erased before the strict
  compatibility-union decision or quarantine;
- no generic JSON-RPC, arbitrary contract/call/block/provider, chain
  transaction, allowance mutation, or raw chain client;
- no secret or replayable signed request in durable/evidence output;
- no real credential, authenticated external request, or real Polygon request
  in Goal G evidence;
- unchanged existing Chaos/OKX schemas, schema-defined bytes, fingerprints,
  fixtures, decisions, RNG/floating-point order, timers, and canonical
  semantic hashes; a rerun may differ only in explicitly revision-bound
  provenance; and
- `production_order_entry_authorized: false`.

Also require:

- no existing production file at or above 1,400 lines grows during Goal G
  without first being split by responsibility;
- no new Goal G production file exceeds 1,000 lines without a Phase 0
  responsibility-based exception, and 1,500 lines remains a hard stop;
- no new production function exceeds 200 lines without a focused
  decomposition review, and 250 lines remains a hard stop;
- auth, user-WS, account, reconciliation, execution, response, and composition
  responsibilities remain separate modules;
- dependency-policy tests allow crypto/auth dependencies only at the
  authenticated edge and narrowly reviewed JSON-RPC/ABI dependencies only at
  the chain edge; and
- source-policy/compile-fail tests reject any credential, signer, client,
  prepared-effect, dispatch-grant, raw DTO, arbitrary-request, arbitrary-RPC,
  chain-mutation, or provider-selector escape.

## Execution Discipline

Work phase by phase. For each phase:

1. inventory current callers, ownership, schemas, public exports, fixtures,
   dependencies, source-policy guards, and file/function sizes before editing;
2. add focused failing tests or golden fixtures before semantic code;
3. separate mechanical extraction from semantic behavior changes;
4. prefer static concrete composition and by-value canonical state;
5. run focused unit, integration, deterministic, authority, dependency,
   security, and benchmark gates;
6. inspect the complete diff, public exports, dependency graph, serialized
   bytes, secret surfaces, file/function sizes, allocations, and queue bounds;
7. record exact commands, results, hashes, decisions, and limitations in the
   Goal G handoff;
8. commit the phase only when its gate is green; and
9. continue automatically while green.

The user-authorized, documentation-only Amendment 2 runnable-contract commit
is the sole pre-gate exception to item 8. It freezes instructions needed to
run the remaining Phase 0 gates and explicitly does not claim that Phase 0 is
green. The later Phase 0 evidence/gate commit remains mandatory.

Do not push or change a remote branch unless the user separately requests it.
Do not run another overlapping code-writing goal concurrently.

Do not use `TRYBUILD=overwrite` across a suite. Review each changed diagnostic
fixture and accept only the intended rejected operation and privacy/type
reason.

No test, benchmark, example, build script, or documentation generator may
read a real secret, contact an authenticated external endpoint, or contact a
real Polygon RPC endpoint. Network tests use owner-local loopback servers with
the constrained non-default `local-evidence` feature only.

## Phase 0: Baseline, Protocol Freeze, And Threat Model

Phase 0 changes documentation plus one narrowly authorized benchmark-policy
tranche. That tranche is already complete at
`facd3a616fc20e7bc1abc627235588b7532ff8b1`; the restarted run must verify it
but MUST NOT change its workload, timed boundary, report schema, counters,
hashes, production runtime code, or policy again. Its historical authorized
edit scope was only:

- the latency-validation branch at
  `crates/reap-pm-live/src/evidence/runner.rs:81`;
- `crates/reap-pm-live/benches/pm_action_path.rs`; and
- their benchmark-policy tests.

The completed tranche removed only the legacy absolute p50/p99.9 exits,
retained the exact
`15,000`-sample check and every logical/hash/allocation/memory/cardinality/
queue assertion, and emitted the full existing report. The workload, timed
boundary, report schema, counters, hashes, and production runtime code MUST
remain unchanged after that tranche. Its exact diff and focused gate stay
recorded separately.

Maintain and complete:

- `docs/polymarket-authenticated-execution-goal-g-handoff.md`; and
- `docs/polymarket-authenticated-execution-boundary.md`.

Record:

- Reap revision, branch, clean state, remote relation, and concurrent-session
  overlap;
- proof that the Goal G baseline and Goal F final tree are ancestors of
  `HEAD`;
- clean exact `../imm-strategy` revision;
- availability of the pinned Predarb Git object and only its dirty path names;
- current `Cargo.lock`, workspace package/dependency graph, public export,
  schema/version, production-content, file/function/state, and fixture
  inventories;
- current Goal F combined-replay and PM action-path evidence;
- the canonical Chaos backtest and Goal D deterministic anchors;
- one discarded process-warmup Cargo invocation plus three retained same-host
  Cargo invocations of the existing engine, live, Chaos action, and PM action
  benchmarks;
- exact available-storage preflight values and any storage stop without
  deleting retained evidence or user/sibling data;
- the mandated target crate/module DAG above and exact dependency additions;
- a purpose-by-purpose capability/endpoint/channel/closed-JSON-RPC matrix;
- exact Polygon chain, ABI selectors/arguments/results, fixed contracts,
  owner/spender selection, finalized-block consistency, freshness, RPC epoch,
  timeout, size, pacing, origin deferral, and failure bounds;
- the secret lifecycle and threat model;
- request, response, frame, page, lane, retry, timeout, clock-skew, and pacing
  bounds;
- exact application headers, canonical cursor encoding/progression, outbound
  PM clock/geoblock leases, mutation status/body/cross-field union, and all
  five compact Polygon request/response shapes;
- a distinct authenticated journal scope/family and recovery-state plan;
- stable live requirement IDs, closed routes/channels, role ownership, and
  lane assignments that do not reuse any Goal F `PM-FAKE-*`/`FakeEffect`
  identity;
- proof that every individual Goal F requirement ID, fake plan byte, fixture
  provenance, artifact/schema byte, and semantic evidence anchor remains
  frozen; global dependency/public-export/source-policy/content inventories
  are expected to change and must be recomputed before and after Goal G;
- exact benchmark workloads, four separate serial overlap-controlled
  shared-host Cargo invocations (one complete process-warmup suite plus three
  retained invocation reports),
  per-invocation raw-log retention, timed
  boundaries, logical counters, allocation/byte/cardinality/queue-age bounds,
  p50/p95/p99/p99.9/max reporting, and acceptance rules;
- a source/message-family compatibility table for every reached inbound
  status and timestamp field, including raw lexical kind, unit, normalized
  meaning, retained provenance, quarantine behavior, and clearing authority;
- every known protocol conflict and its exact resolution; and
- all deferrals and non-claims.

The boundary supplies the exact runnable Phase 0 command/log layout, overlap
monitoring rule, PM JSON extraction/JQ comparator, and byte-verifying
128-row source re-attestation procedure. Phase 0 executes those contracts
verbatim rather than inventing an evidence script or comparator during the
run. Its exact storage thresholds are stop conditions: no phase may infer
permission to delete `target/tmp`, invalid evidence, user data, or
sibling-repository data.

At minimum, freeze these distinct stable live requirement IDs:

```text
OKX-LIVE-PUBLIC-INDEX-WS
PM-LIVE-PUBLIC-METADATA
PM-LIVE-PUBLIC-BOOK-SNAPSHOT
PM-LIVE-PUBLIC-MARKET-WS
PM-LIVE-PUBLIC-SERVER-TIME
PM-LIVE-PUBLIC-GEOBLOCK
PM-LIVE-USER-WS
PM-LIVE-ACCOUNT-CUT
PM-LIVE-POLYGON-AUTHORIZATION-CUT
PM-LIVE-POSITION-OBSERVATION
PM-LIVE-OPEN-ORDERS
PM-LIVE-ORDER-DETAIL
PM-LIVE-TRADES
PM-LIVE-PLACE-GTC-POST-ONLY
PM-LIVE-PLACE-GTC-POST-ONLY-DISPATCH
PM-LIVE-CANCEL-OWNED
PM-LIVE-CANCEL-OWNED-DISPATCH
PM-LIVE-RECOVERY-CANCEL
PM-LIVE-RECOVERY-CANCEL-RECONCILIATION
```

Phase 0 maps each ID to exactly one role, route/channel, lane, readiness use,
and owner. It may add only a responsibility-split child ID, never replace an
ID with a generic request capability.

Freeze official sources as required above. Phase 0 records vector
specifications and their source hashes only in the two Phase 0 documents;
checked-in executable vector/fixture files land in Phase 2 or Phase 3. Phase 0
must resolve:

- current EOA/type-0 support, exact signer/funder/auth-address relation, and
  outer POST `owner == L2 API-key UUID`;
- standard and negative-risk CLOB V2 domain contracts;
- signed order type/value and expected order-ID derivation;
- exact configured PM metadata, book-snapshot and market-WS contracts;
- exact configured OKX `index-tickers` WebSocket origin, subscription,
  acknowledgement/readiness, liveness, and reconnect contract;
- exact `POST order`, owned `DELETE order`, open-orders, exact-order, trades,
  balance/allowance, user-WS, positions, server-time, and geoblock
  method/path/query/application-header/body/response contracts, including the
  exhaustive mutation status/cross-field union;
- exact visibility scope of every private feed/read (API-key,
  condition-filtered, EOA/funder, or another proven scope), with no promotion
  from credential-visible absence to funder-wide absence;
- per-`asset_type` CLOB balance units and selected-spender binding; freeze a
  spendability unit/comparison only where official evidence proves it,
  otherwise retain the canonical bounded numeric value as non-authoritative
  diagnostic evidence without inferring conditional-token approval or
  stopping Goal G;
- exact chain `137`, finalized-block selection/recheck, pUSD and Conditional
  Tokens contracts, EOA owner, standard/negative-risk exchange,
  `allowance(address,address)` and `isApprovedForAll(address,address)`
  selectors/arguments, canonical `U256`/boolean results, freshness, epoch, and
  all fail-closed cases;
- query inclusion/exclusion in every L2 signature;
- canonical padded base64url secret/output and every rejected alias;
- fixed milliseconds for signed orders, fixed seconds for L2 auth, the exact
  outbound anchor/rounding/pre-write policy, geoblock receipt lease, and the
  source/field-tagged inbound seconds/milliseconds/nanoseconds
  compatibility union, pair-consistency rules, and history/future bounds from
  the boundary, with no magnitude guessing;
- the closed source/message-family-tagged POST, REST order/trade, and user-WS
  vocabulary union plus explicit equivalences and quarantine behavior;
- user-WS one-initial-auth-frame lifecycle, absence or presence of
  acknowledgement, configuration-change close/reconnect, and its exact raw
  single-object-or-bounded-array framing;
- position endpoint pagination and its lack or presence of atomic completeness;
  and
- current rate-limit/error classes used only to set stricter Reap bounds.

Use minimal reviewed cryptographic crates plus independently implemented
narrow Reap signing/authentication. Current official SDK clients are
differential vector oracles only, not production dependencies: their broad
clients/credential access, automatic mutation/heartbeat behavior,
rounding/truncation paths, or lag handling do not satisfy this boundary. Do
not add unused Data/Gamma/bridge/CTF/RFQ/relayer features.

Locked `reqwest 0.12.28` configures a default protocol-NACK retry policy
(effective when a relevant protocol feature is enabled) and follows up to ten
redirects. The current workspace feature set does not enable HTTP/2 or
HTTP/3, but capability safety must not depend on that incidental fact. Every
new Goal G client construction must instead set
`retry(reqwest::retry::never())`,
`redirect(reqwest::redirect::Policy::none())`, and `no_proxy()`. Source-policy
and loopback tests must prove those calls remain present. Reads may be retried
only through the exact capped fresh-attempt cycle after typed classification;
mutation bytes may receive only the one application dispatch authorized by
the durable grant. A separately reconciled recovery cancel receives a new
commitment/grant and does not reuse the old request.

Freeze these stable acceptance targets for later phases:

```bash
cargo test -p reap-polymarket-auth --test protocol_vectors --locked
cargo test -p reap-okx-public-source --features local-evidence \
  --test live_index_loopback --locked
cargo test -p reap-polymarket-public-source --features local-evidence \
  --test live_public_loopback --locked
cargo test -p reap-polymarket-chain-source --features local-evidence \
  --test authorization_loopback --locked
cargo test -p reap-polymarket-live-adapter --features local-evidence \
  --test authenticated_loopback --locked
cargo test -p reap-pm-live --features local-evidence \
  --test authenticated_product --locked
cargo test -p reap-pm-live --test combined_replay --locked
cargo bench -p reap-polymarket-live-adapter --features local-evidence \
  --bench pm_signed_request_path --locked
cargo bench -p reap-polymarket-chain-source --features local-evidence \
  --bench authorization_cut --locked
cargo bench -p reap-pm-live --bench pm_action_path --locked
```

The Phase 0 and final PM action campaigns use the same host, locked toolchain,
release profile, workload, and timed boundaries. Each campaign is four
separate Cargo invocations: retain the complete first invocation as a
discarded process-warmup suite, then retain three comparison invocation
reports. The existing PM evidence runner itself performs one internal warm-up
and emits three internal recorded distributions per invocation. For each
retained invocation and quantile, first take the median of its three internal
values; the campaign value is then the median of the three invocation
medians. Thus each side's comparator uses nine internal recorded
distributions, not an ambiguous three.

Overlap with another benchmark/build or a toolchain, profile, workload, or
boundary mismatch invalidates an invocation before values are inspected and
is retained as invalid evidence; ordinary scheduler variance is not
contamination. The final p50 and p95 campaign values must each be at most
`1.10 ×` baseline and final p99 at most `1.20 ×` baseline. p99.9 and max are
retained and reported but are not local shared-host pass/fail gates. All exact
logical, hash, allocation, memory, cardinality, queue, and authorization gates
remain hard gates. The engine/live/Chaos campaigns retain their existing
relative methodology. New signed-request and chain-source benchmarks
establish versioned local baselines with hard correctness/resource gates but
no latency threshold. Neither campaign makes a target-host or network SLO
claim.

Stop in Phase 0 if:

- EOA/type-0 production support is not current and independently provable;
- exact order digest/order-ID identity cannot be proven;
- embedded order fields/signature cannot be proven to match the EIP-712
  digest, or exact final body bytes HMAC-signed by L2 cannot be transported
  unchanged;
- authenticated endpoint/pagination semantics needed for safe reconciliation
  remain ambiguous, or a reached normal lifecycle/time value cannot be
  represented by the frozen union and bounded fail-closed quarantine;
- the visibility scope of any private feed/read needed for recovery cannot be
  proven;
- exact chain, contract, owner, selected exchange, ABI, finalized-block,
  freshness, result, and failure semantics for the closed authorization cut
  cannot be proven without weakening the typed core;
- the secret source would need to be invented for a target host;
- a production Polygon origin/provider credential would need to be selected
  in Goal G;
- a real credential, authenticated request, or real Polygon request is
  required to resolve the protocol; or
- the mandated dependency graph cannot be implemented without a broad adapter
  or cycle.

The focused evidence-policy commit already gates the benchmark-policy
tranche. Before execution, verify that the user-authorized Amendment 2
runnable-contract documentation-only pre-gate commit (subject
`docs: freeze goal g amendment 2 contract`) is an ancestor of `HEAD`; it does
not mark Phase 0 green and Goal G must not recreate or amend it. If it is
absent, stop because the reviewed prompt package is incomplete. After the
remaining benchmark/replay and evidence checks pass, gate the completed
Phase 0 documents/evidence with a separate documentation-only commit.

## Phase 1: Backend-Neutral Prepared Effects And Fake Parity

Refactor only the existing fake-bound PM effect seam.

Required changes:

- separate transport-neutral exact place/cancel facts from
  `PmFakePlaceCommand`/`PmFakeCancelCommand`;
- retain private construction from existing approval, reservation, readiness,
  revision, expiry, journal, and ownership proofs;
- preserve move-only/take-once preparation and consumption;
- add a sealed/static consumer boundary that admits the existing fake
  consumer and later one live consumer, with no runtime backend selector;
- keep live signer/auth/network capability absent in this phase; and
- migrate every fake caller, result, recovery, replay, benchmark, and
  compile-fail test without changing logical or serialized behavior.

Tests must prove:

- callers cannot construct, clone, inspect private fields, reuse, or mix
  prepared quote/cancel effects;
- a fake role cannot consume a live-scoped grant and vice versa;
- no model/state/config caller can choose a backend;
- storage-before-effect order is unchanged;
- cancel-before-replace is unchanged;
- duplicate/partial/immediate/multiple/cancel-fill fake cases are unchanged;
  and
- Goal F combined replay, journal/recovery, allocation, queue, memory, and PM
  action semantic projection hashes, counters, allocation/queue/memory bounds,
  and schema-defined bytes excluding explicitly revision-bound provenance
  remain exact. Reruns at a new build revision record the expected
  provenance-only difference.

Phase 1 adds no authenticated journal fields, salt/timestamp high-water, live
requirement ID, live route, or live effect-lane value to any Goal F schema.
All V1 fake journal bytes, fixture provenance, fake plan identities, and
schema-defined frozen Goal F hashes excluding explicitly revision-bound
provenance remain unchanged.

Do not combine fake and live result enums into a broad venue result. Normalize
both through the same narrow exact PM lifecycle facts.

Gate Phase 1 with a focused commit and record all before/after public and
source-policy changes.

## Phase 2: Secret Custody, L2 Authentication, And EOA Order Signing

Add the `reap-polymarket-auth` crate.

Implement:

- bounded zeroizing L2 credential and EOA key holders;
- exact public account/profile binding without secret getters;
- exact EOA CLOB V2 standard and negative-risk typed data/digest/signature;
- exact expected venue-order identity;
- exact canonical signed order and outer request bytes;
- exact outer `owner == L2 API-key UUID` binding;
- exact L2 HMAC preimage, canonical padded base64url input/output, application
  HTTP headers, and five auth headers;
- a single-use non-secret request commitment;
- a redacted, typed, bounded error taxonomy;
- durable unique salt/intent identity and validated timestamp inputs supplied
  by the coordinator, never generated ad hoc inside serialization; and
- purpose-specific operations only.

Do not implement API-key provisioning, generic EIP-712 signing, `sign(bytes)`,
arbitrary HMAC input, arbitrary method/path/body signing, builder signing, or
another wallet mode. Polygon JSON-RPC, ABI encoding/decoding, block selection,
and authorization observations do not enter the auth crate.

Required tests include:

- every official/differential vector frozen in Phase 0;
- standard versus negative-risk domain separation;
- buy versus sell amount orientation;
- wrong chain/domain/address/profile/body/path/method/query/timestamp failure;
- noncanonical base64, padding, signature, address, JSON, and numeric failure;
- embedded order fields/signature reproduce the expected EIP-712 digest, and
  the exact final body bytes HMAC-signed by L2 equal the bytes delivered to a
  test sink;
- repeated identical canonical inputs produce identical digest/signature/body;
- different material inputs change commitments;
- salt collision/rollback/overflow and the exact outbound anchor,
  same-second auth, order-age, geoblock, freshness/skew/overflow rejection;
- secret types cannot Clone, Copy, Debug, Display, Serialize, or expose bytes;
- canary secrets are absent from all formatted errors, tracing, journal,
  capture, evidence, and panic-safe test output;
- every Reap-owned temporary secret/transient buffer is zeroized on success
  and every injected failure, with no claim about unavoidable third-party,
  TLS, allocator, or kernel copies; and
- public test-vector private keys are unmistakably test-only.

Gate Phase 2 with a focused commit.

## Phase 3: Public, Chain, And Authenticated Read-Only Sources

First add `reap-polymarket-public-source` by mechanically moving the existing
strict PM public metadata/book/session implementation out of
`reap-polymarket-adapter`, preserving exact public behavior and compatibility
tests before adding network code. Migrate callers; do not re-export the source
from the fixture/fake crate. Then add its closed configured metadata REST/book
snapshot/market-WS edges. Add `reap-polymarket-live-adapter` for
authenticated private/account reads only, initially with no place/cancel
consumer. Add `reap-polymarket-chain-source` for the exact closed Polygon
authorization cut, with loopback-only construction in Goal G. Add the closed
configured `index-tickers` network edge to the existing
`reap-okx-public-source`; do not import `reap-okx-live-adapter`.

Implement distinct sealed roles for:

- `reap-polymarket-public-source`: configured PM lifecycle/CLOB metadata REST,
  book snapshot, market WS, Data API published-position observation,
  server-time readiness, and geographic-availability safety;
- `reap-okx-public-source`: configured public `index-tickers` WS; and
- `reap-polymarket-chain-source`: chain-ID/finalized-anchor consistency and
  the two fixed direct authorization calls defined above; and
- `reap-polymarket-live-adapter`: authenticated user order/fill WS,
  open-order/exact-order/trade reconciliation, and
  CLOB collateral/token/numeric spendability-cache reads.

Use separate bounded order-hot and reconciliation/account HTTP pools if Phase
0 measurement justifies them. Each edge crate owns only its closed route
registry/call set; shared neutral per-host/per-credential pacing remains coordinated
without combining their capabilities. A private low-level request helper may
exist only inside its owning edge crate and must accept a closed route enum,
not arbitrary method/path/body. The chain helper accepts only the fixed
sequence's private states, never an arbitrary JSON-RPC method/address/data/tag.

Normalize strict live replies into source-neutral exact facts consumed by the
existing reducers. Add distinct move-only live delivery seals/connection
epochs; live ingress MUST NOT construct
`PmFixtureCompletionOccurrence`, `PmFixtureAggregateDelivery`, a fixture lane
variant, or any fixture provenance. Keep raw DTOs private and retain Goal F
fixture carriers unchanged. The live CLOB account cut carries complete exact
balances and all per-spender numeric cache/spendability values but no typed
operator-approval or position-completeness claim. The separate Polygon cut
carries the direct typed allowance/approval facts. Published position
observations use the separate projection required above.
Add independently authored current official fixtures; retain pinned Predarb
fixtures only with source commit/path/hash provenance and never treat them as
sufficient current truth.

Required loopback cases include:

- exact configured PM metadata/book REST and market-WS
  subscribe/readiness/resync/reconnect behavior;
- exact configured OKX `index-tickers`
  subscribe/readiness/liveness/reconnect behavior;
- exact application/auth headers/path/query/body on every reached read,
  including identity encoding and wrong/extra/content-encoded failures;
- terminal and multi-page pagination, canonical base64 offset
  decode/re-encode, strictly increasing cursor, exact one-pass query encoding,
  repetition/rollback, page cap, partial page, duplicate row, out-of-scope
  row, and response-size cap;
- user-WS one initial auth frame per epoch, rejection of every later outbound
  subscription/update frame, configuration-change close/reconnect, exact raw
  single-event-object and bounded-array framing with
  `event_type = order|trade`, rejection of
  SDK-normalized envelopes, auth rejection, protocol-proven readiness without
  an invented ack, initial dump if specified, ping/pong, idle, typed close,
  reconnect epoch,
  malformed/unknown frame, pre-mapping event, multi-maker-leg fill, and
  slow/full downstream lane;
- every exact source-tagged lifecycle token and field-local inbound
  seconds/milliseconds/nanoseconds lexical form frozen in Phase 0, including
  `match_time_nano` pair consistency and retained remainder, the REST-only
  distinct `MatchedNotBroadcast` state, family-preserving `MATCHED` meanings,
  checked conversion, history/future bounds,
  unknown/cross-family/out-of-profile quarantine, retained reservations,
  complete-reconciliation clearing, and bounded-quarantine exhaustion to
  durable halt;
- outbound `/time` anchor projection, exact lexical order/auth times,
  same-second first-write check, rollback/expiry/overflow, and geoblock permit
  age `4.999s` pass versus `5.000s` expiry plus blocked/IP/epoch/queued-grant
  invalidation;
- connection/read/write/total timeout and bounded reconnect backoff;
- per-route frozen semantics for 401, 404, 425, 429, 5xx, cancel-only,
  redirect, wrong origin, TLS/proxy configuration rejection, and malformed
  bounded error; never normalize one status class globally, because an exact
  order-detail 404 may mean absence while the same status elsewhere is a
  protocol/configuration failure;
- compile/source-policy proof that custom-origin/loopback injection is absent
  from default/production constructors and features, and that
  `local-evidence` accepts only numeric loopback addresses and is enabled by
  no deployable root;
- exact byte vectors for all five compact JSON-RPC requests, per-cut
  `eth_chainId == 0x89`, finalized anchor, explicit same-block calls,
  anchor-hash recheck, deterministic request IDs/order, ABI selectors/address
  padding, fixed owner/exchange/contracts, top-level result union, and
  canonical 32-byte `U256`/boolean decoding;
- Polygon false/zero/maximum, wrong chain/owner/exchange/contract, null or
  stale/future block, changed hash/reorg, revert/error/ID mismatch, empty,
  short, long, trailing, non-hex/noncanonical boolean, timeout, redirect,
  proxy, response-cap, unsupported finalized/historical-call, partial-cut,
  configuration-epoch, and freshness-expiry cases;
- source/compile proof that the chain crate exposes no production Goal G
  origin constructor, provider credential, generic/batched RPC, raw client or
  response, signer, transaction, `approve`, `setApprovalForAll`, `eth_send*`,
  or CTF mutation;
- dependency/source-policy proof that `reap-polymarket-public-source` cannot
  access auth/private/account/mutation roles and
  `reap-polymarket-adapter` neither depends on nor re-exports its network
  constructors;
- complete unfiltered credential-visible open-order inventory sufficient to
  expose bounded out-of-configured-condition identities as unmanaged/unready
  without claiming funder-wide completeness or cancelling them;
- exact allowance-per-spender and per-asset-kind behavior, including zero,
  one, maximum, garbage, missing/wrong spender, and rejection of arbitrary
  positive-string-to-boolean approval inference; a CLOB conditional value
  `"777"` remains numeric diagnostic evidence and cannot make chain approval
  true;
- readiness matrices joining sufficient/insufficient CLOB balances and
  reservations with fresh/stale/missing direct ERC-20 allowance and
  true/false direct ERC-1155 approval, including non-atomic CLOB/chain epoch
  mismatch;
- all canonical configured-product collateral reservation inputs plus tests
  proving that another credential/process is outside the evidence boundary;
- Data API exact lexical numeric parsing, `sizeThreshold = 0`, bounded
  pagination, absence, duplicate/conflict, and divergence from CLOB
  balance/fill state;
- a bracketed reconciliation cut that preserves later WS occurrences; and
- live-normalized versus fixture-normalized canonical equivalence.

The authenticated read-only composition requires L2 credentials but cannot
obtain the EOA order signer, prepared effect, dispatch grant, cancel/place
mutation, or execution role. The chain composition requires no L2 credential
and cannot obtain any credential, signer, prepared effect, mutation, arbitrary
RPC, or provider-selection role.

Gate the mechanical public-source extraction with its own focused commit, then
gate the chain source with a second focused commit, and gate the
public/authenticated read-only network roles with a third focused commit.

## Phase 4: Authenticated Journal, Exact Live Place, And Owned Cancel

Phase 4 has two ordered tranches. Before any live mutation role exists, add
the distinct `reap-pm-authenticated-mutation-journal` version-1 schema, lease,
durable writer/barriers, recovery projection, header bindings, redaction, and
crash/fault tests specified by this prompt. It must not reuse or reinterpret
Goal F journal V1 and introduces no network/send authority. Gate that journal
foundation with its own focused commit. Only then add the mutation-adjacent
roles and test dispatch against the real authenticated journal durability
path.

Add exactly two mutation-adjacent roles:

1. EOA `GTC` post-only place;
2. exact owned cancel.

The upper PM coordinator consumes the sealed prepared effect and durable
dispatch grant, then invokes one already-bound linear adapter role through the
closed effect port with only the exact lowered place/cancel facts. The adapter
never receives `PreparedPmQuote`, `PreparedPmCancel`, or the coordinator
dispatch-grant type. It cannot create an order candidate, change price or
quantity, choose another token/profile, change metadata, or create another
request.

Implement:

- secret-side signed-request preparation and non-secret commitment return;
- durable commitment/dispatch handshake;
- one application-level dispatch attempt of the exact committed bytes, with
  every Goal G `reqwest` builder explicitly using
  `retry(reqwest::retry::never())`,
  `redirect(reqwest::redirect::Policy::none())`, and `no_proxy()`;
- strict bounded placement/cancellation parsing implementing the boundary's
  exhaustive HTTP/body/duplicate-key/cross-field tables exactly;
- the fixed-profile accepted/rejected/unknown classification and fail-closed
  parsing of every other frozen venue status;
- expected-order-ID and body-commitment correlation;
- ambiguity-triggered exact-order/open-order/trade reconciliation;
- exact-owned cancellation and cancel/fill/late-ack convergence;
- fixed bounded command/result queues and pacing.

Required tests include process/network failure at every transition:

- before intent fsync;
- after intent fsync and before signing;
- after signing and before commitment fsync;
- after commitment fsync and before durable dispatch grant;
- after dispatch grant and before socket write;
- dispatch grant expiry immediately before socket write and expiry caused by
  pacing/fsync/queue/clock-seconds/geoblock delay, proving typed
  definitely-not-dispatched handling with no auth regeneration under that
  grant;
- partial/full write followed by disconnect;
- venue accept followed by lost response;
- response received followed by result-lane loss/process death;
- accepted POST `live`, known out-of-profile POST
  `matched|delayed|unmatched`, ordinary separately tagged trade settlement
  `Matched`/REST-only `MatchedNotBroadcast`, definite post-only-cross,
  duplicate, invalid signature, insufficient balance/allowance, cancel-only,
  425, 429, 5xx, oversized/malformed/unknown response;
- every POST `success/orderID/status/errorMsg/amount/ID/hash` cross-product in
  the closed union, cancel arrays/maps with both/neither/unrelated/duplicate
  IDs, unexpected status classes, and duplicate/unknown JSON keys;
- every frozen source/message-family spelling and inbound timestamp encoding
  for those cases, proving identical semantics only where explicitly mapped
  and quarantine rather than widening success everywhere else;
- user event before REST acknowledgement and REST acknowledgement before user
  event;
- partial, multiple, duplicate, out-of-order, maker/taker, retrying,
  confirmed, and failed fills;
- cancel accepted/not-canceled/unknown racing fill/terminal state;
- exact expected order present, absent, duplicated, conflicting, unmanaged,
  and permanently ambiguous during repair.

Prove zero blind placement retries and zero cancellation of an unmanaged
remote order in every workload. Also prove that one recovery cancel after a
fresh complete cut has an identical body/ID but a new durable commitment,
fresh L2 timestamp/HMAC, and new take-once grant; it never reuses the prior
request.

Gate the place/cancel tranche with a second focused Phase 4 commit.

## Phase 5: Static Authenticated Product, Recovery, And Shutdown

Extend the secret-free PM connectivity plan with a statically separate
authenticated plan. Do not convert the Goal F fixture plan into a runtime
union.

Construct:

- the existing credential-free/fake `PmProduct<Model>` unchanged;
- a sealed credential-free Polygon authorization-source blueprint with no
  Goal G default/production origin constructor;
- an authenticated read-only blueprint with no signer/mutation authority;
- `PmAuthenticatedOwnedRecovery`, a non-secret blueprint for the exact journal
  lease, complete credential-visible reconciliation, and exact-owned cancel
  only, with no model, EOA signer, place, generic cancel, or other mutation
  authority; and
- `PmAuthenticatedProduct<Model>`, a non-secret blueprint requiring the exact
  EOA profile and explicit model type.

The authenticated blueprints own only validated non-secret configuration,
closed plans, and one-shot loader descriptors/capabilities that reveal no
secret bytes. Their consuming `start`/`run` path performs validation, lease
acquisition, and local journal recovery before invoking an externally owned
L2 loader. The full product invokes the separately supplied EOA signer loader
only after credential-visible recovery converges. Only the resulting linear
run/driver owns L2/signing sessions and secret holders. Recovery and full
product runs are mutually exclusive compositions competing for the same
authenticated journal lease; they cannot coexist or share a session.
The chain blueprint is separately owned and carries only frozen chain,
contract, account, selected-exchange, freshness, and bound-capacity facts. It
cannot receive L2/EOA loaders or create an origin/provider.

Production placement additionally requires an opaque
`ExclusivePmMutationScopeGrant` proving exclusive EOA/funder mutation across
all API keys, UIs, processes, manual actions, and on-chain actors. Goal G
provides only a `local-evidence` loopback grant with no production
constructor; Goal H must define and certify the real grant.

No model means no quote or place role. The deterministic fixture model may
construct the authenticated product only in tests/benchmarks with loopback
transport and public test secrets. There remains no default production model
and no top-level CLI command that can place a real PM order in Goal G.

Add one autonomous in-process authenticated owner driver (`run_until_shutdown`
or the exact Phase 0-frozen equivalent). It owns a fixed, bounded worker set
for PM public metadata/market WS, OKX public index WS, user WS, account and
reconciliation reads, the finalized-chain authorization cut, execution/cancel,
persistence, and canonical reduction.
It runs until typed shutdown/fatal halt and does not require an external caller
to inject bytes or repeatedly call `service_turn`. Do not create a task per
order, cancel, fill, page, or timer.

Startup order is:

1. validate all non-secret configuration, profile, routes, capacities, and
   fingerprints;
2. acquire a distinct authenticated journal lease;
3. recover authenticated request commitments, salt identity, ambiguity, and
   owned-order facts;
4. only then invoke the injected L2 loader;
5. construct public, chain-authorization, authenticated read-only, and L2-only
   recovery roles;
6. connect observations and obtain complete metadata, live CLOB
   balance/numeric-spendability cut, one fresh finalized-chain
   allowance/approval cut, complete unfiltered credential-visible open-order/
   fill inventory, and separate published-position evidence without claiming
   atomicity or funder-wide completeness;
7. reject/quarantine unmanaged or ambiguous state and cancel only exact
   journal-proven owned orders when recovery requires it;
8. only after recovery convergence require the opaque exclusive-mutation
   grant, invoke the EOA signer loader, and construct place authority; and
9. enable quote mutation only when every existing readiness/risk conjunction
   is green.

Compose and finalize the distinct authenticated journal scope/family
introduced before Phase 4 mutation for live request commitments,
dispatch-authorized/may-have-sent barriers, post-result facts, salt identity,
ambiguity, and reconciliation. Its header MUST bind
`authentication_enabled = true`,
`production_order_entry_authorized = false`, the fixed EOA/type-0 profile,
CLOB V2 domains/contracts, live requirement/route table, product/config
fingerprints, Polygon authorization requirement/contracts/freshness policy,
and schema version. No production Polygon origin or provider credential is
persisted. Goal F V1 remains byte-identical and
readable by its existing readers, but authenticated runtime MUST refuse to
open V1 directly as a live journal or reinterpret V1 fake provenance as live.
No V1 pending intent, fake venue-order ID, fake dispatch fact, or recovered
fake ownership becomes live place/cancel authority. Salt uniqueness state is
new authenticated-family state and is never inferred from fake provenance.
The frozen current protocol requires no timestamp high-water, so none is
persisted or imported. Every V1 mutation tail remains quarantined/closed. No
persisted fact reconstructs executable authority or causes automatic
resubmission.

On reconnect/restart:

- advance the private connection epoch;
- immediately disable new placement;
- retain cancel/reconciliation authority;
- rebuild exact state through complete bounded reads;
- obtain a new whole finalized-chain authorization cut; never reuse a prior
  transport/configuration epoch's cut;
- preserve later WS facts across the cut;
- restore known-owned identities only from the exact leased journal;
- never claim unmanaged state; and
- re-enable placement only after a clean second-pass convergence.

Implement the one bounded graceful/fatal shutdown path specified above.
Product tests cover missing/revoked exclusive-mutation grant and simulated
second-key, UI/manual, process, and on-chain-writer races. Each case prevents
signer loading/new placement, preserves exact-owned recovery authority, and
never treats credential-visible absence as funder-wide absence.

Required compile-fail/source-policy tests prove:

- core/state/strategy/model/config/journal/evidence/telemetry cannot access
  credentials, signer, raw client, auth builder, route helper, request bytes,
  prepared effect, dispatch grant, or live mutation role;
- the coordinator may own only the sealed non-secret prepared effect, request
  commitment, and take-once dispatch grant required by the canonical mutation
  protocol; it cannot access secrets, auth headers, a signer/client, raw
  request bytes, or construct an adapter role;
- read-only roles cannot reach mutation;
- chain roles cannot reach any credential/signer, arbitrary RPC, production
  origin construction, raw client/response, transaction, `approve`,
  `setApprovalForAll`, or `eth_send*`;
- `PmAuthenticatedOwnedRecovery` cannot reach the EOA signer, place, a model,
  cancel an unmanaged order, or construct the full authenticated product;
- recovery and full-product drivers cannot hold the authenticated journal
  lease/session concurrently;
- no caller can construct `ExclusivePmMutationScopeGrant` from configuration,
  credential text, successful auth, or an empty credential-visible inventory;
- the live consumer cannot consume fake-scoped authority;
- no heartbeat/deadman or generic cancel role exists;
- no OKX private/account/order role enters the PM product; and
- no new PM dependency enters the Chaos runtime/order gateway.

Gate Phase 5 with a focused commit.

## Phase 6: Credential-Free Fault, Security, And Local Performance Evidence

All evidence remains deterministic or owner-local loopback. Do not use real
credentials, authenticated external connectivity, or a real Polygon RPC
origin.

Freeze a fixed end-to-end loopback workload that reaches:

- public OKX and PM readiness;
- user-WS order/fill delivery;
- complete live CLOB balance/numeric-spendability cut, one whole
  finalized-chain authorization cut, complete credential-visible
  open-order/trade reconciliation, and separate published-position
  observation without an atomic or funder-wide claim;
- quote approval, reservation, intent fsync, signing, request commitment
  fsync, dispatch, accepted live result;
- partial and final fill convergence;
- cancel-before-replace;
- lifecycle/time compatibility-union normalization, quarantine, ambiguity,
  Polygon false/stale/reorg fault, and reconnect repair;
- graceful owned-order cleanup; and
- journal recovery.

Two equal runs must produce byte-identical secret-free canonical projections,
journal commitments, logical counters, and evidence hashes. Source-family/time
provenance that participates in semantics is canonical; bounded diagnostics
must have a separately deterministic projection. Test fake and live loopback
backends against the same normalized lifecycle projection.

Run the full bounded fault matrix from Phases 3–5 and scan every produced
stdout/stderr/log/capture/journal/report/evidence file using unique canary
secrets. Require zero canary occurrence and zero raw auth subscription frame.

The new signed-request benchmark must separately report:

- exact order digest/signature;
- canonical body construction;
- L2 HMAC/header construction;
- bounded edge queue handoff;
- loopback transport write/response parse; and
- included/excluded stages.

The chain-source benchmark must separately report fixed JSON-RPC/ABI
construction, finalized-anchor/result parsing, consistency/freshness checks,
bounded edge queue handoff, loopback transport, allocations/bytes, and
included/excluded stages.

Both new edge benchmarks establish versioned local baselines with correctness,
fixed logical-count, bounded queue/byte/cardinality, and no-unbounded-
per-request-allocation gates; Goal G makes no absolute or target-host latency
pass/fail claim for either. Signing, JSON, chain IO, and ABI work remain
outside the canonical owner loop. The existing PM action benchmark must retain
its exact logical count, zero owner allocations, bounded memory, deterministic
hashes, and pass the Phase 0 paired relative rule. Existing engine/live/Chaos
benchmarks retain their recorded relative methodology. Report every frozen
quantile and max from every recorded invocation.

Run every affected benchmark as four separate serial overlap-controlled
shared-host Cargo invocations: one complete process-warmup suite followed by
three retained invocation reports, retaining the raw log from each. For PM
action use the specified median-of-three-internal-runs then
median-of-three-invocations comparator. Investigate host noise; do not discard
a valid bad result or raise a threshold merely to finish.

Gate Phase 6 with a focused code/evidence commit.

## Phase 7: Documentation, Global Verification, And Handoff

Update at minimum:

- [architecture.md](architecture.md) with authenticated PM role ownership,
  separate chain-source ownership and dual CLOB/chain authorization facts,
  strict lifecycle/time union, secret boundary, two-stage request durability,
  and static fake/live composition;
- [polymarket-product-connectivity-boundary.md](polymarket-product-connectivity-boundary.md)
  by adding only a dated Goal G supersession pointer/current-layer section;
  do not rewrite Goal F's historical requirements, evidence, phase claims, or
  hashes;
- `docs/polymarket-authenticated-execution-boundary.md` with the final
  normative capability, route, Polygon call/finality/freshness, compatibility
  union, profile, secret, recovery, and exclusion contracts;
- [performance.md](performance.md) with only local edge/regression evidence
  including the paired PM comparison and explicit target-host exclusions;
- [trading-readiness.md](trading-readiness.md) with authenticated mechanics
  implemented but real credential/account/model/economic/settlement/host/
  trial/approval gates still open; and
- the Goal G handoff with all phase commits, commands, results, source
  revisions/hashes, fixture provenance, dependency/public/schema inventories,
  file/function/state sizes, security scans, faults, benchmarks, limitations,
  and deferrals.

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

Rerun every stable Goal G target frozen in Phase 0. Then run the canonical
Chaos backtest twice and require byte equality and exact expected SHA-256:

```bash
cargo run --locked -q -p reap-cli -- \
  backtest \
  --format normalized-jsonl \
  --config examples/iarb2-basic.toml \
  --data fixtures/normalized/chaos_quote_hedge.jsonl \
  --pretty >target/tmp/goal-g-backtest-1.json
cargo run --locked -q -p reap-cli -- \
  backtest \
  --format normalized-jsonl \
  --config examples/iarb2-basic.toml \
  --data fixtures/normalized/chaos_quote_hedge.jsonl \
  --pretty >target/tmp/goal-g-backtest-2.json
cmp target/tmp/goal-g-backtest-1.json target/tmp/goal-g-backtest-2.json
sha256sum target/tmp/goal-g-backtest-1.json
```

The expected canonical Chaos backtest SHA-256 remains:

```text
38acf9f5e0c310f2ec5528974beffadf4c1a7f84d46efa8d9664ee7051e84691
```

Also require:

- all Goal D deterministic decision/risk and exact-order projections remain
  byte-identical;
- Goal F combined replay, PM action, journal/recovery, bounded-memory,
  allocation, overload, compile-fail, dependency, and source-policy gates
  remain green under the Amendment 1 PM relative timing policy;
- every current official protocol vector has pinned provenance;
- Predarb fixture hashes remain historical parser seeds, not signing truth;
- no outside-workspace Cargo path dependency exists;
- crypto/auth/network dependencies occur only in approved authenticated,
  public-source, and closed chain-source edge crates;
- `reap-polymarket-wire`, core, state, strategy, coordinator, journal,
  capture, evidence, and telemetry contain no secret type or key access;
- `reap-live`, `reap-live-contracts`, `reap-order`, and the Chaos product gain
  no PM auth/order behavior;
- every authenticated route and Polygon call is in the closed capability
  matrix and no arbitrary request/RPC method exists;
- secret-canary scans are clean;
- no real credential, authenticated external request, or real Polygon request
  was used;
- every queue and page collection is bounded;
- before/after dependencies, public exports, schemas, files/functions/state,
  deterministic hashes, and benchmarks are recorded;
- Reap and `../imm-strategy` are clean; and
- the known Predarb dirty paths are unchanged and were never read.

Commit final documentation only after the global gate is green. Do not push
without separate user instruction.

## Stop Conditions

Stop and report the exact conflict when:

- Reap or `../imm-strategy` has unexplained or overlapping changes;
- the pinned sibling reference objects are unavailable;
- progress would require reading or changing Predarb dirty/runtime/secret
  state;
- current official protocol sources cannot resolve a reached auth, signing,
  route, query, body, order-ID, response, or pagination contract, or a reached
  lifecycle/time value cannot be safely preserved by the frozen tagged union
  and fail-closed quarantine;
- the fixed Polygon chain/contracts/owner/exchange, ABI, finalized-block
  consistency/freshness, or typed result contract cannot be proven;
- EOA/type-0 support or `maker == signer == funder == auth address` is not
  current and provable;
- a real credential, authenticated external call, real Polygon call, or real
  order is needed;
- an API credential must be created/derived/rotated by the product;
- a production quote/economic/risk model must be invented;
- embedded fields/signature can diverge from the EIP-712 digest, or final body
  bytes HMAC-signed by L2 can diverge from transported bytes;
- expected venue-order identity or durable salt/intent uniqueness cannot be
  proven;
- an acknowledgement-unknown placement would need blind resubmission;
- an unmanaged or ambiguous remote order would need to be claimed/cancelled;
- a mutation must be retried without intervening exact reconciliation;
- a private event, mutation result, durability acknowledgement, or
  integrity-bearing fact must be silently dropped;
- a partial page/cut must masquerade as a complete snapshot;
- Data API position evidence must use `f64` or grant authority without
  completeness;
- allowance must be selected from an arbitrary map entry or updated by the
  read path;
- a CLOB numeric conditional value must become boolean approval, a chain
  result must be inferred from CLOB cache state, or a partial/stale chain cut
  must grant readiness;
- secrets or replayable auth/request bytes enter a log, error, capture,
  journal, evidence artifact, public API, config projection, or generic
  signer;
- PM integration requires widening an OKX/Chaos gateway, a broad adapter,
  arbitrary request/RPC surface, provider selector, dynamic plugin, shared
  canonical mutation, task per order, or new hot-path dynamic dispatch;
- an existing Goal F/Chaos artifact byte, semantic anchor, fake behavior,
  authority, or performance gate changes outside the explicitly authorized
  PM effect seam, the Amendment 1 PM absolute-to-relative benchmark-policy
  migration, and new live-only roles/family; the PM workload and every
  non-timing correctness/resource gate remain frozen, and global dependency,
  public-export, source-policy, and content inventories may change only by the
  reviewed additions required here;
- a compile-fail/source-policy gate passes only by broad export or allowlist
  widening;
- repeated equal replays differ;
- a new source monolith violates the file/function limits without a
  pre-approved responsibility exception; or
- completion requires target-host tuning/evidence, real account
  certification, settlement/redemption, external provisioning, live trial, or
  production approval.

The lack of a production Polygon provider/origin or target host is not a stop:
Goal H owns that selection. It becomes a stop only if Goal G code cannot
remain loopback-proven and origin-sealed without inventing one.
Likewise, failure to prove a CLOB `CONDITIONAL` numeric false/true encoding or
comparison unit is not a stop: that value remains selected-spender diagnostic
evidence and the direct chain cut supplies typed authorization.

Do not weaken an invariant, broaden a capability, hide a protocol conflict,
discard a valid bad result, or rewrite a negative test merely to finish.
Record the blocker and propose the smallest separately scoped next goal.

## Explicit Exclusions

Goal G does not authorize or implement:

- Predict.fun connectivity, quote mirroring, or cross-prediction-venue
  arbitrage;
- OKX private data, account reads, reconciliation, order placement, or cancel
  in the PM product;
- more than the configured Goal F PM token/product scope;
- production fair probability, quote economics, sizing, inventory policy, fee
  calibration, profitability, or capital approval;
- proxy, Safe, deposit-wallet/POLY_1271, session, remote, builder, or another
  signer/funder profile;
- L1 API-key create, derive, list, delete, rotate, automatic fallback, or
  credential persistence;
- order heartbeat/deadman, credential-wide cancellation, or a dormant
  strategy-inaccessible implementation of either;
- FOK, FAK/IOC, GTD, marketable/market order, batch place/cancel, amend,
  cancel-market, cancel-all, RFQ, combo, reward, or arbitrary order behavior;
- generic Gamma/Data/CLOB access beyond the exact reached roles;
- balance-cache update, on-chain pUSD wrap, allowance/operator-approval
  mutation (`approve`/`setApprovalForAll`), transfer, split, merge, redemption,
  resolution mutation, relayer, bridge, withdrawal, or settlement operation;
- generic/batched JSON-RPC, arbitrary `eth_call`, caller-selected
  address/calldata/block/provider, any chain transaction or `eth_send*`, and
  any on-chain capability beyond the two exact reads and anchor checks;
- a universal venue adapter, dynamic plugin/venue registry, arbitrary command,
  raw request executor, or public authenticated client;
- importing Predarb application/runtime code or depending on the sibling;
- a real secret source or production Polygon provider/origin selected for a
  target host;
- a deployed PM trading service or real-order CLI;
- authenticated production/demo smoke, controlled-capital trial, or exchange
  certification;
- target-host selection, CPU pinning, thread-per-core, SPSC/ring-buffer
  conversion, busy spin, allocator/runtime replacement, `io_uring`, kernel
  bypass, or latency-architecture work;
- modifying either sibling repository;
- changing production order-entry authorization; or
- claiming production readiness, economic validity, target-host latency,
  target-account validity, or trading approval.

## Completion

Goal G is complete only when:

- every phase and focused/global gate is green;
- the fixed Goal F EOA/type-0, one-token GTC post-only profile has current
  exact CLOB V2 signing and L2 authentication;
- endpoint-connected PM/OKX public observations, authenticated user
  order/fill, live CLOB balance/numeric-spendability cuts, separate whole
  finalized-chain typed authorization cuts, reconciliation, and separate
  published-position roles feed source-neutral exact canonical PM reducers;
- one narrow live place and owned-cancel consumer preserves take-once
  authority and durable pre-dispatch ordering;
- the L2-only recovery composition can reconcile and cancel exact owned orders
  without a signer/model/place role, and the fixed autonomous owner driver
  runs every bounded role until shutdown;
- place/cancel/user-WS/REST/journal/restart paths converge deterministically
  or retain an exact durable operator-required unresolved halt, without blind
  retry, invented fill data, unmanaged cancellation, float rounding, or
  loss/erasure of ambiguity;
- no heartbeat, generic cancel, marketable order, provisioning, or unrelated
  exchange capability was added;
- fake and loopback-live paths share the same normalized lifecycle semantics
  while remaining statically separate implementations; the strict
  source/message-family status/time union retains provenance and quarantines
  every unknown/out-of-profile value without success promotion;
- secrets remain confined to the authenticated edge and all canary scans,
  visibility, dependency, and compile-fail gates pass;
- canonical state remains one bounded deterministic owner with no network,
  JSON, crypto, secret, or blocking work;
- every existing Goal F/Chaos artifact byte, semantic anchor, fake behavior,
  authority, and regression gate remains green under the explicitly amended
  PM paired-relative timing policy; only the reviewed live-only
  dependency/public/schema/source-policy inventory additions differ;
- no generic RPC/chain mutation/provider-selection capability exists, and all
  chain-source construction/evidence in Goal G is credential-free and
  loopback-only;
- all other evidence is credential-free and loopback-only;
- documentation records the exact implementation and every open model,
  economic, provisioning, account, settlement, host, trial, and approval
  gate; and
- Reap and `../imm-strategy` are clean while Predarb's known dirty paths remain
  unchanged and unread.

Completion means Reap has a production-shaped, narrowly authenticated
Polymarket connectivity and execution implementation consistent with its
architecture. It does not mean the PM strategy is economically valid, a real
account is provisioned, a target host is qualified, a live order has been
tested, or trading is approved.
