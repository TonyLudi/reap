# Polymarket Controlled Trial Goal (PM-T2)

Status: **current PM-T2 authority; implementation authorized, external mutation
not yet authorized**.

PM-T1 is the completed local connectivity baseline. Its accepted implementation
is preserved at `1c82ad8556923ef6096002b8078a7bf9334345cd`; the read-only
qualification baseline, including proxy-safe account balance loading, is
preserved at `d3c0162b6f0148ceb9bbb52d58103faf21289186`. Historical Goal G
and PM-T1 records remain evidence and are not rewritten by this goal.

PM-T2 adds one narrowly controlled production experiment for the reviewed
type-1 proxy account. It does not authorize a production strategy, continuous
quoting, generic order entry, or an order at the time this document is written.
The live boundary is split deliberately:

- **Phase A** proves one type-1 proxy, production CLOB V2, passive GTC
  post-only placement followed by exact-owned cancellation and complete
  reconciliation through the durable PM journals.
- **Phase B** may prove one minimum-capital passive fill and the resulting
  balance/position convergence, but only after Phase A is accepted and a new,
  separate exact live authorization is reviewed.

The goal starts with these facts:

```text
pm_t1_local_baseline_complete=true
pm_account_only_proxy_read_evidence_passed=true
pm_full_proxy_read_evidence_passed=false
pm_t2_phase_a_live_authorization_present=false
pm_t2_phase_b_live_authorization_present=false
production_order_entry_authorized=false
real_order_submission_authorized=false
```

No config boolean, CLI flag, environment variable, code owner, or prior
read-only artifact can change those last four facts. A short-lived external
authorization bound to the exact binary, configuration, account, order terms,
host, and time window is required at the explicit live hold described below.

## Authority And Precedence

This document is the only current PM-T2 execution authority. It has precedence
only for work explicitly named here. It does not retroactively alter:

- the immutable terminal records in the Goal G handoff;
- the accepted PM-T1 local evidence;
- either `reap-pm-readiness` read-only contract;
- old journal bytes, hashes, or meanings; or
- the standing prohibition on every other PM mutation.

Implementation, local loopback tests, synthetic protocol vectors, offline
verification, and secret-free artifact work may proceed under this goal. The
first production place request must stop at the live hold until the user
reviews and authorizes one exact Phase A record. Phase B must stop at a second
hold and requires a separate later authorization even if Phase A is green.

The following remain out of scope throughout PM-T2:

- continuous or strategy-driven quoting, automatic replacement, or repricing;
- market/FAK/FOK/GTD orders, immediate taker execution, batch placement, or
  amendments;
- arbitrary request, signer, route, host, wallet-type, chain, or contract APIs;
- cancel-market, batch cancel, or a Reap-owned cancel-all capability;
- allowance/approval mutation, pUSD wrapping, transfers, split/merge,
  redemption, settlement, bridge, withdrawal, or relayer mutation;
- builder attribution or nonzero `metadata`/`builder` fields;
- Safe, Deposit Wallet, EOA, session-key, or another account profile;
- use of the deterministic fixture quote model as a production strategy; and
- any second order under the same authorization, including a retry or
  replacement at a different price.

## Current Baseline And Blocking Gaps

PM-T1 already provides reusable fixed CLOB V2 order numerics, EIP-712/HMAC
primitives, exact `POST /order` and `DELETE /order` serialization, no-retry
loopback mutation transport, two durable barriers before a send, typed
acknowledgement-unknown handling, exact-owned cancellation, complete
credential-visible reconciliation, user-stream normalization, and
cross-journal recovery.

Those capabilities do not currently reach Phase A:

1. The production account is a type-1 proxy profile: the signer EOA and proxy
   funder/maker are distinct. The mutation signer, full private-read facade,
   live-contract configuration, coordinator authority, and both journal
   profiles currently require one type-0 EOA with
   `maker == signer == funder`.
2. Only the account-only balance reader supports `signature_type=1`. That
   narrow proof does not fetch metadata, positions, orders, trades, the user
   stream, closed-only state, or a complete reconciliation cut, and it does
   not remotely attest the configured proxy funder.
3. Mutation transport has only a feature-gated literal-loopback constructor.
   No production mutation composition or operator trial executable exists.
4. Geoblock, closed-only mode, CLOB fee parameters, matching-engine mode, and
   finalized Polygon allowance/operator approval are not reached production
   place gates. Current live metadata parsing observes but deliberately ignores
   several CLOB V2 fee and order-age fields.
5. The only concrete quote model is a deterministic fixture model. PM-T2 needs
   a manual, hash-bound, take-once trial plan, not a claim that the fixture
   model is suitable for trading.
6. Existing authenticated journals forbid production authority and encode the
   type-0 profile. Their old meaning must not be silently reinterpreted.
7. The account-only production pass is useful evidence of four bounded reads;
   it is not private-key possession, full-account readiness, or place/cancel
   authorization.

Until every applicable item is closed, the correct runtime result is a
pre-dispatch stop with `production_order_entry_authorized=false`.

## Official Protocol Source Pins

The source freeze date for this design is **2026-08-09**. Only official
Polymarket documentation is authority for external protocol claims:

| Official source | PM-T2 contract frozen from it |
| --- | --- |
| <https://docs.polymarket.com/trading/wallets-auth> | Wallet types `0` EOA, `1` Proxy, `2` Safe, and `3` Deposit Wallet; signer versus account-wallet separation; pUSD/Conditional Token approvals |
| <https://docs.polymarket.com/getting-started/api> | L1 credential derivation; L2 exact-body HMAC; `POLY_ADDRESS` is the Polygon signer; the five L2 headers |
| <https://docs.polymarket.com/trading/place-orders> | CLOB V2 order fields/domain, type-1 maker/signer mapping, tick/size/amount rules, `POST /order`, GTC/post-only semantics, and placement results |
| <https://docs.polymarket.com/trading/manage-orders> | Exact-order/open-order/trade reads, exact `DELETE /order`, partial cancellation results, closed-only mode, cancel-all scope, and account-wide order heartbeats |
| <https://docs.polymarket.com/trading/realtime-order-updates> | Authenticated user events, settlement lifecycle, terminal `CONFIRMED`/`FAILED`, and mandatory REST refresh after reconnect |
| <https://docs.polymarket.com/trading/matching-engine> | HTTP `425`, cancel-only/post-only restart modes, and continued cancel availability |
| <https://docs.polymarket.com/api-reference/geoblock> | Same-egress geographic eligibility check before placement |
| <https://docs.polymarket.com/api-reference/markets/get-clob-market-info> | Token/outcome membership, tick, minimum size, fee fields, delay flags, and minimum-order-age field |
| <https://docs.polymarket.com/trading/fees> | Per-market match-time fees and current maker-fee-zero policy |
| <https://docs.polymarket.com/v2-migration> | CLOB V2 domain/order changes and pUSD replacing USDC.e as collateral |
| <https://docs.polymarket.com/resources/contracts> | Current Polygon pUSD, Conditional Tokens, standard Exchange, and Neg Risk Exchange addresses |
| <https://docs.polymarket.com/api-reference/core/get-current-positions-for-a-user> | Address/condition-scoped Data API position projection and pagination bounds |
| <https://docs.polymarket.com/api-reference/rate-limits> | Current CLOB read/place/cancel request ceilings |

These are moving pages, not immutable revisions. Before protocol code changes,
Phase A0 must capture the exact retrieved bytes, final URL, retrieval time,
content type, length, and SHA-256 for every used page in a reviewed source
manifest. The manifest must also pin the exact official SDK revision used for
independent type-1 vectors. A changed byte or conflicting official source ends
the phase until the contract is reviewed and this document is amended; a
moving page is never silently treated as the previously reviewed pin.

The first PM-T2 freeze is recorded in
`docs/polymarket-controlled-trial-source-manifest.json`. The create-new,
exact-origin retriever is `scripts/freeze-pm-t2-sources.sh`; its matching raw
Markdown evidence is retained outside Git under the private `var/reap/pm-t2/`
evidence root. The committed manifest is evidence, not permission to trade.

The current protocol decisions are:

- chain `137`, CLOB V2 Exchange EIP-712 domain version `2`;
- type-1 Proxy Wallet order: `maker = proxy funder`,
  `signer = signer EOA`, `signatureType = 1`;
- L2 credentials and `POLY_ADDRESS` remain signer-owned;
- BUY maps maker amount to exact pUSD cost and taker amount to shares; SELL
  reverses those assets; encoded amounts use six decimals;
- the Phase A/B order is `GTC`, `postOnly=true`, `deferExec=false`,
  `expiration="0"`, zero metadata, and zero builder;
- post-only rejects an order that would take immediately, but it can fill later
  as resting maker liquidity;
- placement is `POST /order`; exact-owned cancellation is `DELETE /order` with
  the exact compact body `{"orderID":"<id>"}`;
- a partially filled cancel removes only the unfilled remainder;
- current makers pay no venue fee, but the exact market fee parameters must be
  re-read and bound immediately before placement; and
- current collateral is pUSD. No PM-T2 artifact or label may call the reached
  collateral USDC.e.

## Fixed PM-T2 Account And Order Profile

PM-T2 reaches exactly one operator-reviewed type-1 account:

```text
wallet_profile = poly_proxy
signature_type = 1
signer = one configured EOA derived from the four-file private key
funder = one distinct configured legacy Proxy Wallet
maker = funder
order_signer = signer
POLY_ADDRESS = signer
owner = runtime-loaded L2 API-key UUID (never persisted)
chain_id = 137
```

The signer and funder are configuration values, never documentation examples.
The runtime must derive the signer from the private key and compare it to the
configured signer before it constructs any mutation capability. It must bind
the L2 bundle to that same signer. Because balance responses do not echo the
proxy wallet, a separate reviewed account-identity record must bind the signer
to the proxy funder. Repetition of the configured funder in artifacts is not
remote attestation.

One live authorization names exactly one condition, question identity, token,
outcome label, standard-versus-negative-risk domain, side, price, quantity,
maker amount, taker amount, maximum loss, and time envelope. No field may be a
wildcard, list, range chosen at runtime, or environment lookup.

Phase A may authorize either one BUY or one SELL, but the side is immutable for
the attempt. A BUY reserves at least its exact pUSD `makerAmount`; its worst
case resolution loss is bounded by the paid pUSD amount. A SELL additionally
requires authoritative configured-token inventory and a separately reviewed
share/payout-risk cap; `price * shares` alone is not an adequate SELL loss
bound. No expected rebate reduces either bound.

Phase B's first fill/position experiment is BUY-only. Expanding Phase B to a
SELL requires a later amendment because it tests inventory disposal rather
than position creation.

## Short-Lived Exact Live Authorization

The executable must consume a non-secret canonical trial config and a separate
operator-provided authorization record. The config cannot authorize itself.
The authorization record must contain or commit to all of the following:

### Build and host identity

- PM-T2 phase (`a_place_cancel` or `b_fill_position`);
- repository commit and clean-tree attestation;
- `Cargo.lock` SHA-256;
- release binary SHA-256 and length;
- canonical config SHA-256 and length;
- source-pin manifest SHA-256;
- trial-runbook revision/hash;
- target host identity, boot identity, runtime user, and egress identity;
- artifact directory and journal family/version; and
- one non-secret credential-slot ID/fingerprint that reveals no secret-derived
  material.

### Account and market identity

- chain `137`, `signature_type=1`, exact signer, and distinct proxy funder;
- non-secret credential-slot ID plus an independently reviewed
  signer-to-proxy evidence reference; the runner validates the request owner
  against the runtime-loaded API key without placing that value in the
  authorization or artifacts;
- exact condition/question identity, token ID, and outcome label;
- exact standard or negative-risk exchange/domain; and
- current pUSD and Conditional Tokens contract identity from the pinned
  contracts source.

### Economic terms

- exact side, decimal price, decimal share size, tick, and minimum order size;
- exact six-decimal maker and taker base-unit amounts;
- an exact pUSD maximum-loss/reservation cap and, for Phase A SELL, an exact
  outcome-share/payout-risk cap;
- `GTC`, `postOnly=true`, `deferExec=false`, `expiration=0`, zero metadata,
  zero builder, and no fee/rebate credit in the loss bound;
- exactly one place dispatch allowance and no replacement/reprice allowance;
  and
- exact primary/recovery cancel budgets.

### Time and approval

- authorization ID, issuing reviewer, review time, and exact purpose;
- UTC not-before and expiry, with a maximum authorization lifetime of fifteen
  minutes;
- maximum age of every preflight observation at dispatch;
- maximum resting duration: at most thirty seconds for Phase A and an exact
  separately reviewed value at most five minutes for Phase B;
- primary cancel deadline, cleanup-not-after deadline, and maximum remediation
  duration;
- explicit statements that one possible fill is within the loss cap and that
  post-only does not mean no fill; and
- explicit approval for only the named phase and one attempt.

The canonical record is opened from a protected local file with no symlink,
hard-link, ownership, mode, size, duplicate-key, or trailing-byte ambiguity.
The runner re-hashes every bound input immediately before it creates the live
capability. Expired, early, mismatched, incomplete, duplicated, or already
consumed authorization fails before secret loading. Consumption is durable and
take-once. Restart may recover/cancel the already authorized order but can
never mint a second place allowance.

The authorization record must not contain credentials or a private key. Its
presence is necessary but not sufficient: the online preflight must also pass.

## Four-File Credential Custody

The controlled-trial executable is separate from `reap-pm-readiness` and takes
exactly four secret file paths:

1. signer EOA private key;
2. CLOB L2 API key;
3. CLOB L2 HMAC secret; and
4. CLOB L2 passphrase.

It accepts no secret value in a command-line argument, environment variable,
TOML/JSON config, stdin, journal, log, panic, error, debug formatter, or
artifact. The files must be regular, owner-matching, mode `0600`, single-link,
non-symlink files under a fresh owner-only mode-`0700` runtime directory. The
runner opens with no-follow semantics, validates metadata before and after the
read, enforces strict byte/grammar bounds, binds once, and zeroizes source
buffers. Core dumps and secret-bearing tracing are disabled for the unit.

The operator stages copies; PM-T2 never reads from or mutates another
repository's credential store directly. The staged private-key file is removed
after the exact signed order and durable prepared commitment exist. The three
L2 files remain protected until the order is proven terminal because restart
recovery and exact cancellation intentionally need L2 credentials but no
private key. They are removed only after final reconciliation and offline
verification. Every error path states which files remain without printing
their contents.

No secret-derived hash is durable. Public signer/funder identities and the
non-secret credential-slot fingerprint may be journaled; the API key itself,
HMACs, auth headers, signed bodies, signatures, and private-key-derived
diagnostics may not.

## Mandatory Online Preflight

Preflight is one closed, ordered capability graph. A partial success never
becomes readiness, and no mutation constructor is released until the final
joined permit is green.

### Environment and authorization

1. Recheck binary/config/source/runbook/host hashes against the unconsumed live
   authorization.
2. Prove one owner process and exclusive leases for both PM journals and the
   artifact directory. Refuse a dirty repository or unreviewed binary.
3. Verify the authorization clock window from the target host and fresh CLOB
   `/time`; excessive skew, timestamp regression, or epoch change fails closed.
4. Query `https://polymarket.com/api/geoblock` from the same egress path used
   for CLOB placement. Require `blocked=false`; the permit is no more than five
   monotonic seconds old at dispatch. Geoblock gates new placement, never
   suppresses already-authorized cleanup.
5. Check CLOB health and ensure the run is not intentionally started during an
   announced/reported matching-engine restart. A `425` or restricted-mode
   `503` ends the one-place attempt; PM-T2 never follows the venue's general
   retry recommendation for this bounded trial.

### Market and book

6. Fetch and atomically join the exact long CLOB market response and abbreviated
   CLOB market-info response. Require matching condition/question/token/outcome,
   active, not closed, not archived, accepting orders, order book enabled,
   correct negative-risk domain, and current contract addresses.
7. Bind current tick, minimum order size, maker/taker fee fields, fee curve,
   order-delay flags, game/start/end time, and minimum-order-age field. Any
   unsupported/nonzero order-age or unexpected fee/mode field stops the first
   trial rather than being ignored.
8. Fetch a fresh exact-token book and establish the market stream. Require a
   valid two-sided book and no tick-size change after authorization. The exact
   price must remain tick-aligned and passive at dispatch: BUY strictly below
   best ask; SELL strictly above best bid. A changed book can stop the attempt
   but cannot select a new price.
9. Prefer a non-sports, non-imminent-resolution market for both first trials.
   Sports start-time cancellation/delay or a near end/resolution requires a
   later reviewed authorization.

### Account and private state

10. Derive and bind the private-key signer, signer-owned L2 credentials, and
    reviewed distinct proxy funder. Require the exact type-1 profile everywhere.
11. Query `GET /auth/ban-status/closed-only`. Phase A/B BUY requires
    `closed_only=false`; any true/malformed/stale value blocks placement.
12. Obtain fresh type-1 collateral and configured-token balance/allowance
    replies. Require exact pUSD collateral for BUY or exact Conditional Token
    inventory for SELL and the selected exchange's required allowance.
13. At one fresh finalized Polygon block, independently verify pUSD ERC-20
    allowance and Conditional Tokens ERC-1155 `isApprovedForAll` for the proxy
    funder and selected standard/negative-risk Exchange. PM-T2 can read these
    exact calls but cannot construct an approval or arbitrary RPC operation.
14. Obtain a configured-funder Data API position observation with exact decimal
    parsing and bounded pagination. It is corroborating state, not atomic sell
    authority.
15. Establish the authenticated user stream for the exact condition, including
    application heartbeat, and then obtain complete unfiltered
    credential-visible open-order and trade cuts plus exact detail for every
    implicated ID. A quiet socket alone is not readiness.
16. Require no ambiguous/unmanaged order, no unresolved fill, and no existing
    order owned by the trial credentials. Credential-visible absence is not
    funder-wide absence, so the operator must also attest that no UI, API key,
    bot, or person will trade the proxy during the trial. Dedicated trial
    credentials are preferred.
17. Recompute exact amounts, reservation, loss cap, and risk decision from the
    frozen terms. Available balance is reduced by all remotely reserved open
    size; no reported balance is assumed entirely free.

Any reconnect after the complete cut invalidates the permit and requires fresh
open-order/trade/detail/account cuts. The permit is consumed by one place
decision and cannot be reused.

## Phase A — Passive Place And Exact-Owned Cancel

Phase A tests transport and lifecycle, not execution quality. The operator
selects one exact price intended to rest away from the opposite best price.
Post-only prevents immediate taker execution; it cannot prevent a later maker
fill. Therefore even Phase A must authorize the full possible fill loss.

The only allowed sequence is:

```text
exact live authorization durable + unconsumed
-> complete proxy preflight green
-> exact manual trial plan + reservation durable in the PM journal
-> type-1 signed expected order ID + secret-free semantic commitment
-> authenticated Prepared durable
-> DispatchAuthorized/may-have-sent durable
-> consume the sole place capability
-> at most one POST /order
-> typed result durable
-> exact-order/open-orders/trades/user-stream reconciliation
-> exact-owned cancel intent + authenticated cancel barriers durable
-> DELETE /order for that exact journal-owned ID
-> complete terminal reconciliation and account/position after-cut
-> offline verification and credential teardown
```

Ordinary placement acceptance is intentionally narrower than the venue:

- HTTP `200`;
- `success=true`;
- exact precomputed EIP-712 `orderID`;
- `status="live"`;
- exact expected maker/taker amounts; and
- no returned trade ID or transaction hash.

A documented rejection is a stopped, non-passing attempt. `matched`, `delayed`,
or `unmatched`; an identity/amount contradiction; `425`, `429`, `503`; timeout;
partial/lost response; or any malformed/unknown status halts placement and
enters reconciliation. No case automatically resends the order. A response
that is definitely pre-send may be reported for a new separately authorized
attempt, but this authorization is consumed and cannot be reused.

After ordinary live acceptance, cancel at the earlier of the exact authorized
rest duration or an operator stop. The ordinary cancel success is HTTP `200`
with the exact order ID as the sole `canceled` entry and no `not_canceled`
entry. The result alone is not terminal proof: exact detail, complete open
orders, complete trades, user events, and account observations must converge.

Phase A passes only when all of the following are true:

- the exact order was observed live under its journal ownership proof;
- its unfilled remainder was canceled before the deadline;
- no fill occurred;
- complete reconciliation proves no open remainder and no other trial-owned or
  unmanaged order;
- pUSD, configured-token, and position observations are consistent with no
  fill;
- both journal chains and the bridge verify offline; and
- all staged secret files are removed after their permitted lifetime.

An accidental partial or full fill is within the authorized risk cap but makes
Phase A non-passing. Placement stays disabled, the unfilled remainder is
canceled, and the fill follows the Phase B-grade settlement and account
reconciliation procedure. It is retained as incident/safety evidence and does
not implicitly authorize Phase B.

## Exact Cancel And Remediation

Primary cancellation carries one journal-proven order ID and one take-once
dispatch. There is no arbitrary-ID input. A cancel timeout, response loss,
`not_canceled` reason, identity conflict, partial result, or non-200 response
never clears the slot.

Recovery may issue only the byte-identical exact-owned cancel and needs no
private key. Each recovery send requires:

- the live authorization's still-active cleanup grant;
- verified authenticated and Goal-F journal ownership;
- a fresh complete order/open-orders/trades cut proving the exact order still
  live with an unfilled remainder;
- a newly durable recovery dispatch barrier; and
- remaining exact recovery-attempt and cleanup-time budget.

The authorization sets the primary count and recovery count explicitly; the
implementation hard cap is one primary plus two recovery sends. There is no
timer-, HTTP-, SDK-, or library-level blind retry. If fresh reconciliation
does not prove the order live, do not send. If the budget expires without a
terminal state, retain an operator-required incident and keep all new placement
disabled.

Before the live hold, the operator must prove a second, independent manual
emergency cleanup method for the same account. It may use an official client or
UI, but it is outside Reap and its credentials are never provided to the trial
process. Account-wide cancel and CLOB order heartbeats are not added to PM-T2:
both affect other orders owned by the credential account. If later desired,
they require a separate authority and proof that the credential account is
exclusive and quiescent.

Cleanup authority survives expiry or revocation of new-placement authority
until the exact trial order is terminal or the operator takes over. A new
geoblock/closed-only/market stop blocks placement but does not voluntarily
disable exact cancel attempts that the venue still accepts.

## Phase B — Minimum-Capital Fill And Position Trial

Phase B is unreachable until Phase A has passed, its evidence has been
independently accepted, this document's Phase B hold has been reached, and the
user has provided a new exact `b_fill_position` authorization. A Phase A
authorization can never be relabeled or reused.

The first Phase B trial is exactly one passive BUY at the smallest authorized
two-decimal share quantity that:

- is at least the current market minimum;
- produces integral six-decimal maker/taker amounts under the current tick;
- fits the exact pUSD balance/allowance and configured loss cap; and
- is explicitly named, rather than calculated after approval.

It remains GTC and post-only. It neither crosses nor reprices to force a fill.
If no fill occurs before the exact deadline, cancel the order and record a safe
no-fill, non-passing result. A repeat needs another authorization.

For a partial or full fill:

1. bind the user-stream and REST trade IDs to the exact journal-owned order;
2. verify side, price, amount, maker role, current fee fields, and every maker
   leg without inferring amount from status alone;
3. cancel and prove terminal the unfilled remainder;
4. follow settlement through `MATCHED_NOT_BROADCASTED`, `MATCHED`, `MINED`, or
   `RETRYING` until terminal `CONFIRMED` or `FAILED`;
5. refresh complete open orders and trades after every reconnect;
6. reconcile pUSD and configured-token balance/allowance changes;
7. load the proxy-funder Data API position after-cut and compare token, size,
   average price/value fields as monitored evidence; and
8. retain any CLOB/Data API/on-chain timing divergence explicitly rather than
   fabricating an atomic snapshot.

Phase B passes only with a confirmed, exactly bounded fill; zero open remainder;
fully consistent journal, trade, balance, and configured-token state; an
observed position delta consistent with the fill; no unknown/nonzero maker fee;
and clean offline verification/teardown. A terminal failed settlement, missing
position convergence, unknown fee, or unresolved account delta is a safe stop,
not a pass.

## Implementation Work Packages

Work proceeds in order and commits only at a green gate. Parallel work may
prepare tests or docs but cannot bypass the dependency order.

### A0. Protocol freeze and baseline

- Commit the exact source manifest described above and reconcile it with the
  current V2 implementation.
- Preserve PM-T1 and account-only evidence unchanged.
- Add source-policy tests that keep `reap-pm-readiness` private-key-free and
  mutation-free.
- Freeze a secret-free canonical PM-T2 config and authorization schema with
  duplicate-key rejection and stable hashing.

### A1. Type-1 cryptographic and account profile

- In `reap-polymarket-wire`, represent the fixed Proxy profile explicitly:
  maker/funder distinct from signer and `signatureType=1`. Do not turn it into
  an arbitrary signature-type API.
- In `reap-polymarket-auth`, add a separate fixed type-1 signer/body path. The
  private key must match `order.signer`; the L2 owner/header must match signer;
  maker must match the reviewed proxy funder. Semantic commitments bind both
  addresses, signature type, domain, and exact outer profile.
- Add independent official-client vectors for standard/negative-risk BUY and
  SELL plus negative vectors for swapped maker/signer, wrong type, wrong
  domain, wrong L2 owner, and type-0/type-1 cross-use.
- Preserve every type-0 API/vector and keep both profiles non-interchangeable.

### A2. Full proxy read and preflight graph

- Extend `reap-pm-live-contracts` and the full read-only private owner with an
  explicit type-1 profile; do not widen the account-only owner.
- Bind balance queries to `signature_type=1`, positions to the proxy funder,
  and user/order/trade scope to signer-owned credentials plus configured
  account/market checks.
- Add exact geoblock and closed-only read capabilities.
- Promote current market fee, delay, order-age, and relevant lifecycle fields
  from ignored input to typed preflight facts.
- Implement the closed finalized-Polygon pUSD allowance and Conditional Token
  operator-approval cut without arbitrary RPC or mutation.
- Prove a complete proxy full-read certification locally, then under a
  separately authorized external read-only run before the Phase A live hold.

### A3. Versioned journals and manual trial authority

- Introduce a new journal version/profile for passive GTC post-only Proxy
  execution. Do not reinterpret old EOA records or evidence tags.
- Bind the live authorization digest, exact account/market/order/loss/time
  terms, credential slot, and source/config/binary hashes in the new scope.
- Admit `production_order_entry_authorized=true` only in this new live journal
  after the take-once authorization and complete preflight are proven. Local
  and dry-run evidence remains false.
- Add a manual `PmControlledTrialPlan` that yields exactly the reviewed
  candidate. It reuses readiness, risk, reservation, durability, and recovery
  but has no model callback, timer, reprice, or replacement path.

### A4. Production one-shot edge and operator executable

- Add an exact-host production mutation config fixed to
  `https://clob.polymarket.com`, HTTPS default port, root origin, no redirects,
  no ambient proxy, no HTTP/library retry, bounded response, and one request
  body serialization.
- Keep loopback and production types distinct. No caller-controlled origin,
  route, method, raw body, headers, signature type, or order type reaches the
  production edge.
- Add a dedicated `reap-pm-controlled-trial` executable. Do not add mutation to
  `reap-pm-readiness` or the general `reap` CLI.
- Implement the four-file owner, exact live-authorization consumer,
  preflight-only mode, Phase A mode, recovery-only mode, and offline verifier.
- Ensure recovery-only construction accepts L2 files and durable ownership but
  cannot accept a private key or create a place request.

### A5. Local, fault, and target-host evidence

- Exercise exact accepted-live/cancel, typed rejection, partial fill, immediate
  unexpected fill, duplicate, timeout/partial response, `425`/`429`/`503`,
  auth failure, reconnect, stale geoblock/closed-only/metadata/account state,
  process death after each durable boundary, and all recovery budgets.
- Prove one place send maximum, no blind retry, no secret retention, and no
  new placement after any ambiguous or filled state.
- Run formatting, tests, lints, dependency/security checks, release build,
  artifact verification, and target-host dry-run gates prescribed by the
  repository. Evidence commands are not trading authority.
- Produce the exact Phase A binary/config/source/runbook hashes and stop.

### A6. Phase A live hold and execution

The implementation must stop here and present the exact authorization
checklist. No production-authenticated mutation request is sent until the user
supplies and reviews the complete Phase A authorization record. General assent
to PM-T2, access to credentials, a green dry run, or permission to continue
coding is not that authorization.

After exact authorization, run one Phase A attempt, enter cleanup regardless of
success, verify offline, remove staged credentials, and stop. Do not begin
Phase B in the same run or authorization.

### B0. Phase B hold

Review and accept Phase A evidence. Freeze one minimum-capital Phase B BUY,
binary/config hashes, market/account state, loss cap, and time window. Stop for
a new exact user authorization.

### B1. Phase B execution and final handoff

After the separate authorization, run one fill/position attempt, cancel any
remainder, reconcile through terminal settlement and position/account state,
verify artifacts, remove staged credentials, and publish a redacted handoff.
No further order follows.

## Live Stop Matrix

| Observation | Required action |
| --- | --- |
| Missing/expired/mismatched/consumed authorization | Stop before secret loading or mutation construction |
| Signer/private-key/L2/proxy mismatch | Zeroize, stop, no request |
| Geoblock blocked/stale/failure | Suppress place; retain cleanup capability |
| Closed-only true/stale/failure | Suppress opening BUY/place; retain cleanup capability |
| Market/token/outcome/domain/contract/tick/minimum/fee/mode drift | Suppress place; do not choose replacement terms |
| Incomplete account, position, open-order, trade, or user-stream state | Suppress place and reconcile |
| Any pre-existing, unmanaged, ambiguous, or unresolved credential-visible order/fill | Suppress place; operator review |
| Book crossed/one-sided/stale or price no longer passive | Consume no place grant; stop this authorization |
| Definite place rejection | Durable red result; no retry under this authorization |
| May-have-sent or contradictory place result | Halt placement; reconcile expected ID/open orders/trades |
| Phase A fill | Cancel remainder, settle/reconcile, mark Phase A non-pass, stop |
| Cancel ambiguity or nonterminal remainder | Recovery-only exact cancel under fresh proof and budget |
| Cleanup budget exhausted | Durable operator-required incident; all placement remains disabled |
| User revokes or asks to stop | Disable placement immediately; finish exact-owned cleanup only |
| Phase A pass | Stop and request independent acceptance; Phase B remains unauthorized |
| Phase B no fill | Exact cancel, safe non-pass, stop; no repricing/retry |
| Phase B confirmed bounded fill and converged position | Pass attempt, verify/teardown, stop; no second order |

## Evidence And Handoff Contract

Every run writes create-new private artifacts. Public/redacted evidence may
retain only:

- exact commit/binary/config/source/runbook hashes;
- non-secret account, market, order, and authorization identities;
- preflight pass/fail facts with bounded timestamps and source provenance;
- exact expected/observed order IDs and economic terms already approved as
  non-secret;
- place/cancel classifications and response-body commitments, never raw auth
  material;
- complete-cut and user-event identities needed for reconciliation;
- fill, fee, settlement, balance, and position deltas approved for the private
  artifact; and
- journal chain roots, cleanup result, secret-file teardown facts, and the
  invariant `place_dispatch_count <= 1`.

Raw authenticated bodies, balances/positions not approved for disclosure,
private keys, L2 values, HMACs, headers, signed request bodies, signatures, and
secret-derived fingerprints remain private and ignored. Offline verification
recomputes all non-secret hashes and journal transitions without credentials or
network access.

The final handoff must distinguish implementation evidence, authorized
external read evidence, Phase A live evidence, and Phase B live evidence. A
green earlier class never implies a later class. It records all stopped/red
attempts and unresolved cleanup explicitly.

## Completion Conditions

PM-T2 is complete only when:

1. the current official source manifest and type-1 vectors are reviewed;
2. full proxy read/preflight, four-file custody, versioned journals, exact-host
   one-shot transport, recovery-only cancel, and offline verification exist;
3. all local/fault/security/target-host gates are green;
4. a separately exact-authorized Phase A run passes with no fill and proven
   exact cancellation;
5. Phase A evidence is independently accepted;
6. a later separately exact-authorized Phase B run produces one bounded
   confirmed fill and converged configured-token position/account evidence; and
7. no unresolved order, fill, credential file, journal ambiguity, or operator
   cleanup item remains.

If Phase B is not separately authorized, PM-T2 may stop successfully at an
accepted Phase A milestone, but the overall fill/position objective remains
open and `production_order_entry_authorized=false` is restored after cleanup.
No completion state grants standing production order authority.
