# Polymarket Trading Connectivity Goal PM-T1

Status: current runnable product goal.

PM-T1 supersedes Goal G, Goal G-R, and their amendments as runnable authority.
Those documents remain unchanged historical evidence of a stopped approach;
they are not instructions, gates, source allowlists, or completion criteria for
this goal. Goal F remains the completed implementation baseline.

`production_order_entry_authorized = false` remains mandatory throughout this
goal.

## Runnable Prompt

> /goal Execute PM-T1 exactly as specified in
> `docs/polymarket-trading-connectivity-goal.md`. Build on the green Goal F
> Polymarket foundation, implement only the narrow authenticated PM vertical
> slice defined here, and verify it with deterministic vectors and local mock
> HTTP/WebSocket tests. Preserve the existing fake backend and Chaos behavior.
> Do not use Goal G/G-R runner or amendment machinery, real credentials,
> external authenticated requests, or real orders. Stop and report a concise
> blocker if any fixed boundary cannot be satisfied; do not create an
> amendment chain.

## Outcome

Implement a production-shaped but locally exercised Polymarket connectivity
slice that is ready for a separately authorized credentialed smoke test.

The slice must connect the existing PM product architecture to:

1. PM public market data;
2. authenticated order and fill lifecycle observation;
3. complete order reconciliation;
4. exact collateral, configured-token, allowance, and position observation;
5. one fixed GTC post-only place operation; and
6. cancellation of one exactly proven locally owned order.

This goal completes connectivity architecture and deterministic local proof. It
does not claim a production strategy, production readiness, exchange
certification, or trading authorization.

## Why This Is The Next Goal

The repository does not need another general venue refactor before PM work.
The existing strategy boundary is already narrow:

- `reap-pm-strategy` sees only an OKX public reference, PM instrument state,
  time, and its pure quote-model contract;
- PM executable prices already use exact millionths in strict `(0, 1)`;
- private lifecycle, reconciliation, account/position state, risk, readiness,
  ownership, durable intent, and take-once prepared effects already exist;
- all current PM private/account deliveries are fixture-backed; and
- the final PM place/cancel edge is in-process fake only.

The product gap is therefore authentication plus endpoint-connected adapters,
not a new state engine, universal exchange client, or strategy API.

“PM is another venue” means it follows the same architectural pattern as other
venues—normalized inputs, single-writer state, typed decisions, durable
effects, and a narrow adapter. It does not mean PM and OKX must share wire
types, numeric types, credentials, or one superclass.

## Authority And References

Use the following precedence when requirements differ:

1. this PM-T1 document;
2. [Polymarket product connectivity boundary](polymarket-product-connectivity-boundary.md),
   [architecture](architecture.md), and [trading readiness](trading-readiness.md);
3. the implemented Goal F contracts and tests;
4. current official Polymarket protocol documentation, revalidated during
   Phase 0; and
5. the two pinned sibling references below.

The official sources checked when this goal was written on 2026-08-04 are:

- <https://docs.polymarket.com/api-reference/authentication>;
- <https://docs.polymarket.com/v2-migration>;
- <https://docs.polymarket.com/api-reference/trade/post-a-new-order>;
- <https://docs.polymarket.com/api-reference/trade/get-user-orders>;
- <https://docs.polymarket.com/api-reference/trade/get-single-order-by-id>;
- <https://docs.polymarket.com/trading/orders/cancel>; and
- <https://docs.polymarket.com/market-data/websocket/user-channel>.

Official protocol documentation is authoritative for current wire behavior.
Pin the exact official pages or upstream source revisions used by the
implementation in tests or the PM-T1 handoff. If current official behavior
contradicts a fixed Reap safety invariant, stop and report the contradiction;
do not silently broaden the product.

### `../predarb`

Use only tracked object
`8222273a9c72033b760e1d2fec813bc77144556d` as a read-only implementation
reference. Inspect it with `git show <object>:<path>` so dirty working-tree,
runtime, capture, dashboard, and secret files are never read.

It is useful for reached protocol behavior: EIP-712 signing, L2 HMAC request
authentication, exact authenticated REST shapes, public and user WebSockets,
place/cancel, open-order/order-detail/fill reads, and balance/allowance reads.
Reimplement those behaviors behind Reap's boundaries and verify them
independently. Do not add a path dependency or copy its broad client API,
cloneable string credentials, generic time-in-force support, cancel-all,
floating/rounded order amounts, filtered-only recovery cuts, or fallback
allowance selection.

### `../imm-strategy`

Preserve the existing Chaos behavior pinned at
`b6b120c7b7c466d8431bf082f3229328c5d7b2ae`. It remains the reference only for
the supported Chaos/OKX behavior. It is not a PM protocol or PM architecture
authority. PM-T1 must not add OKX account, private, order, algo, or spread
capabilities to the PM product.

### Historical Goal G Material

Useful synthetic vectors and security invariants may be extracted from
`polymarket-authenticated-execution-boundary.md` only after comparison with
current official sources and the pinned Predarb object. None of Goal G's
frozen runners, hashed runner copies, evidence namespaces, diff allowlists,
host margins, or amendment rules carry into PM-T1.

## Closed Product Boundary

The strategy/model remains data-in, decision-out. It must never receive a
credential, signer, HTTP/WebSocket client, route builder, transport handle, or
callable execution gateway.

The PM product may construct only these connectivity roles:

| Role | Exact responsibility | Consumer |
| --- | --- | --- |
| OKX public reference | Existing configured `index-tickers` reference only | PM quote model/readiness |
| PM public market source | Configured metadata, server time, exact book seed/resync, and configured-token market stream | PM public ingress/book readiness |
| PM private lifecycle source | Authenticated user-stream order, fill/trade, and connection-epoch observations for configured markets/account | Existing private reducer |
| PM order reconciliation source | Complete credential-visible open-order and fill cuts plus exact order detail | Existing reconciliation reducer |
| PM account/position source | Exact collateral, configured-token balance, every required spender allowance/approval, and configured position availability | Existing account/position reducer and risk |
| PM fixed place edge | Consume one take-once prepared GTC post-only quote after its exact durable barrier | Existing lifecycle owner |
| PM exact-owned cancel edge | Consume one take-once cancel for one journal-proven local venue-order identity | Existing lifecycle owner |

No common `ExchangeClient`, enlarged `VenueAdapter`, raw authenticated request
method, arbitrary URL/path method, or optional-method gateway may expose these
roles. Public read, authenticated read, private stream, fixed place, and owned
cancel are separate capabilities even if they share adapter-private transport
mechanics.

## Fixed Trading And Identity Profile

PM-T1 supports one profile:

- Polygon CLOB V2;
- one configured EOA signature profile with `maker == signer == funder`;
- an operator-provided, pre-provisioned L2 API key, HMAC secret, and
  passphrase bound to that EOA;
- exact configured condition, market, outcome token, account, domain, and
  required-spender identities;
- exact integer maker/taker amounts derived from existing PM numerics;
- executable `PmPrice` values only in `1..=999_999`, tick aligned and passive;
- GTC, post-only placement only; and
- cancellation by the exact journal-proven local venue-order ID only.

Signature type, domain, chain, verifying contract, negative-risk mode, token,
and account identity must be explicit and mutually consistent before signing.
The final place body is serialized once; L2 authentication signs those exact
body bytes and transport writes the same bytes. EIP-712 order signing and L2
request HMAC are distinct authorities and must be tested independently.

API-key creation/derivation is not a runtime capability in PM-T1. The trading
composition consumes a pre-provisioned L2 bundle. A future, separately reviewed
bootstrap tool may create or derive credentials if there is a named need.

## Target Dependency And Ownership Shape

Use the existing crates rather than creating a second PM domain stack:

- `reap-pm-core`, `reap-pm-state`, and `reap-pm-strategy` remain secret- and
  network-free;
- `reap-polymarket-wire` owns bounded, secret-free wire types, exact parsing,
  and canonical serialization, but no sockets or credentials;
- add a small `reap-polymarket-auth` edge for redacted, zeroizing,
  non-`Clone` secret holders, fixed EOA CLOB V2 order signing, and fixed L2
  request authentication; it owns no strategy, canonical state, or network;
- add a modular `reap-polymarket-live-adapter` edge for allowlisted HTTP and
  WebSocket transports and the separate roles in the capability table; it
  owns no strategy decisions or canonical PM state;
- evolve `reap-pm-live-contracts` from fake-named place/cancel requirements to
  backend-neutral fixed-place and owned-cancel requirements without adding
  order families; and
- keep `reap-pm-live` as the single coordinator/composition owner. It admits
  normalized adapter deliveries and routes prepared effects to either the
  existing fake backend or the authenticated adapter selected only at the
  outer composition root.

Do not put all roles in one large implementation file. Split the live adapter
by responsibility—configuration/credentials, public HTTP, public WebSocket,
user WebSocket, reconciliation reads, account reads, place, cancel, and
transport supervision. Shared private helpers may own connection pooling,
timeouts, bounded bodies, and error mapping; they must not become a public
all-capabilities client.

Keep endpoint I/O, body serialization, authentication, and cryptographic work
at adapter workers. The deterministic coordinator remains the only canonical
state mutator and must never block on network I/O. Every queue, response body,
frame, page count, retry count, and retained diagnostic collection is bounded
with an explicit fail-closed policy.

## Secret And Logging Contract

- Non-secret configuration contains only credential environment-variable
  names or opaque credential-slot identity, never values.
- Read secret values only at the authenticated composition root after
  non-secret validation.
- Private key, API key, HMAC secret, passphrase, derived material, auth headers,
  and authenticated WebSocket subscription bytes are non-`Clone`, zeroized on
  drop where supported, and redacted from `Debug` and errors.
- Secrets and secret-derived hashes never enter journals, capture, telemetry,
  metrics labels, panic messages, test snapshots, or handoff documents.
- Authentication exposes purpose-specific operations only. There is no generic
  sign-bytes, sign-arbitrary-EIP-712, HMAC-arbitrary-request, or secret getter.
- Tests use synthetic credentials only and prove redaction with positive and
  negative scans.

## Execution Plan

### Phase 0 — Baseline And Protocol Cut

1. Confirm the worktree and sibling reference revisions; preserve unrelated
   user changes.
2. Run the existing PM targeted tests before editing.
3. Revalidate the fixed profile and reached endpoints against current official
   documentation and the pinned Predarb object.
4. Record a short implementation map in the PM-T1 handoff. Do not create a new
   frozen runner, amendment, evidence namespace, or source diff allowlist.

Gate: the fixed EOA/GTC/post-only/exact-owned profile is internally consistent,
and the existing Goal F PM tests are green.

### Phase 1 — Backend-Neutral Effect Edge

1. Make the plan capability names backend-neutral while preserving their exact
   fixed semantics.
2. Introduce one take-once dispatch boundary for prepared place and one for
   prepared cancel.
3. Keep fake execution as a first-class implementation and preserve its
   deterministic results, replay identity, durability order, and compile-fail
   authority tests.
4. Prove no strategy/model or public/private observation role can mint,
   duplicate, inspect into raw authority, or dispatch a prepared effect.

Gate: every pre-existing Goal F fake test remains green and no live transport
or credential is needed to run it.

### Phase 2 — Authentication And Exact Wire Bytes

1. Add the minimal secret holders and validated EOA/L2 credential binding.
2. Implement exact CLOB V2 domain/struct hashing and recoverable EOA order
   signing for the fixed profile.
3. Implement exact L2 timestamp, method, route, body preimage, HMAC-SHA256,
   padded base64url, and header construction for allowlisted operations only.
4. Add canonical once-only place and cancel serialization.
5. Test official, independently authored, and pinned Predarb parity vectors;
   include changed-byte, wrong-key, wrong-address, wrong-domain, wrong-route,
   wrong-timestamp, and reserialization negatives.

Gate: signatures, order IDs, request bodies, HMACs, and headers match the pinned
vectors byte-for-byte, and no secret can cross into strategy/state/logging.

### Phase 3 — Public, Private, Reconciliation, And Account Sources

1. Connect the existing PM public parsers/session state to an allowlisted public
   HTTP and market-WebSocket source.
2. Add the authenticated user-WebSocket role with bounded subscription,
   heartbeat, reconnect, epoch replacement, and raw-to-normalized delivery.
3. Add bounded authenticated REST reads for a complete open-order cut, exact
   order detail, and a complete fill/trade cut.
4. Add exact collateral, configured-token, required-spender allowance/approval,
   and configured position observations. Floating Data API PnL views are not
   canonical position authority.
5. Feed only existing owner-bound deliveries into the existing reducers. Do not
   add a parallel order, fill, balance, allowance, or position store.

Gate: deterministic mock HTTP/WebSocket tests cover empty, paginated,
duplicate, out-of-order, reconnect, stale, malformed, oversized, foreign,
partial, and contradictory observations. Only a complete compatible cut can
restore readiness.

### Phase 4 — Fixed Place And Exact-Owned Cancel

1. Lower an existing prepared quote into the exact signed GTC post-only body.
2. Require the exact durable request commitment/barrier before any transport
   write.
3. Send each take-once grant at most once. Classify definite rejection,
   acceptance, out-of-profile response, and acknowledgement ambiguity without
   inventing success.
4. Lower cancel only for an exact journal-proven local venue-order identity.
   Private or reconciliation observations alone never mint ownership.
5. Keep owned cancellation available when new placement is suppressed, except
   when cancellation itself cannot be authenticated or safely formed.
6. On timeout, disconnect, partial response, conflict, or unknown lifecycle,
   retain authority/reservation evidence, halt new placement, and reconcile.

Gate: loopback tests prove durability-before-write, exact byte identity,
single-send authority, ambiguity handling, restart/reconciliation behavior, and
that unsupported mutation routes are unreachable.

### Phase 5 — Product Composition And Fault Matrix

1. Add a production-shaped library composition root that selects fake or
   authenticated PM edges statically. No public runtime backend selector or
   raw-client escape is allowed.
2. Keep the OKX side of the PM product public-reference-only.
3. Exercise one deterministic vertical slice: OKX reference plus PM book,
   validated quote, durable place, private lifecycle/fill, account/position
   change, reconciliation, and exact-owned cancel.
4. Exercise auth failure, public/private disconnect, stale reference/book,
   incomplete account snapshot, writer failure, response ambiguity, duplicate
   fill/order, and shutdown with a live owned order.
5. Prove that primary transport/writer failures remain the reported failures;
   the test harness must not mask them with secondary fixture mismatches.

Gate: fake and authenticated-loopback paths converge through the same canonical
reducers and lifecycle semantics without sharing credentials or wire DTOs with
strategy code.

### Phase 6 — Documentation And Standard Verification

1. Update architecture, capability-boundary, trading-readiness, and operations
   documents to describe what is implemented and what remains unauthorized.
2. Add `docs/polymarket-trading-connectivity-goal-handoff.md` with phase commits,
   exact tests, protocol/reference pins, limitations, and next authorization
   gate. Keep it concise; do not embed generated evidence or runner source.
3. Run the standard repository checks:

   ```bash
   cargo fmt --all -- --check
   cargo clippy --workspace --all-targets --locked -- -D warnings
   cargo test --workspace --locked --no-fail-fast
   cargo build --release --workspace --locked
   git diff --check
   ```

Gate: all checks are green and the handoff truthfully says
`production_order_entry_authorized = false`.

## Required Acceptance Tests

Completion requires explicit tests proving:

- strategy and PM state crates cannot import auth, signer, transport, or live
  adapter types;
- the PM product constructs no OKX private/account/order/algo/spread role;
- only GTC post-only place and exact-owned cancel are constructible;
- market/FOK/FAK/GTD, batch, amend, cancel-all, allowance mutation, settlement,
  redemption, generic signing, and arbitrary request attempts fail at compile
  time or construction;
- `PmPrice` remains strict `(0, 1)`, tick aligned, exact, and passive, with no
  `f64` round trip after quote-policy conversion;
- synthetic EIP-712 and L2 vectors match independently and every signed body is
  the transported body;
- no place/cancel write precedes durable acknowledgement;
- only journal-proven ownership permits cancel;
- incomplete account/reconciliation pages cannot claim completeness;
- private/reconciliation duplicates converge exactly once;
- stale or failed PM/OKX inputs suppress placement while safe owned cancel is
  retained;
- credentials and authenticated frames are redacted and absent from durable
  artifacts; and
- deterministic fake and authenticated-loopback vertical slices preserve
  canonical state and replay semantics.

## Explicit Exclusions

PM-T1 does not implement or authorize:

- real credentials, external authenticated calls, demo orders, or live orders;
- a production probability, spread, size, inventory, fee, or risk model;
- Predict.fun connectivity or cross-prediction-market quoting;
- OKX execution, private data, account data, algo orders, or spread orders in
  the PM product;
- a universal venue client, universal order DTO, dynamic plugin registry, or
  runtime capability discovery;
- FOK, FAK/IOC, GTD, marketable orders, batch place/cancel, cancel-all,
  cancel-by-market, or amend;
- proxy/Safe/POLY_1271 wallets, builder attribution, session signers, or API-key
  provisioning;
- allowance/approval mutation, transfers, split/merge, settlement, redemption,
  bridge, withdrawal, or arbitrary Polygon RPC;
- target-host selection, CPU affinity, kernel tuning, deployment, alerting,
  production latency SLOs, or capacity certification; or
- production approval or a claim that PM trading is production ready.

## Stop Conditions

Stop with one concise handoff instead of inventing an amendment when:

- current official protocol behavior conflicts with the fixed product profile
  or existing exact/safety invariants;
- signing/body/HMAC parity cannot be independently established;
- endpoint behavior cannot produce the complete exact state required by the
  existing reducers without rounding or silently filtering;
- a required implementation would expose raw credentials, generic signing,
  arbitrary transport, unsupported mutation, or multiple mutation owners;
- Goal F fake behavior or Chaos behavior cannot be preserved; or
- unexplained overlapping changes prevent safe work in an in-scope file.

Ordinary implementation defects, test failures, or slow local builds are not
reasons to create amendments. Diagnose and fix them in the normal repository
workflow. If the fixed scope itself must change, stop and ask the goal owner to
edit this single document before resuming.

## Completion Definition

PM-T1 is complete only when the fixed authenticated PM vertical slice exists,
all deterministic vector/mock/fault and standard workspace gates are green,
the fake backend still works, and the repository clearly remains unauthorized
for real PM mutation.

The next step after PM-T1 is a separately scoped and explicitly authorized
credentialed read-only smoke test. A minimal-capital place/cancel smoke test is
a second, later authorization after read-only identity, account, allowance,
position, private-stream, and reconciliation evidence is accepted.
