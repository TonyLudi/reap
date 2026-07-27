# Polymarket Authenticated Execution Boundary

Status: **amended runnable implementation contract for Goal G**. The original
Phase 0 stop remains historical evidence in the
[Goal G handoff](polymarket-authenticated-execution-goal-g-handoff.md).
Amendment 1 resolves it without weakening the typed core: CLOB numeric
spendability/cache evidence remains numeric, while a separate closed Polygon
source reads ERC-20 allowance and ERC-1155 operator approval directly at one
finalized block. The same amendment authorizes a strict source-tagged
lifecycle/time compatibility union and replaces the host-specific PM latency
ceiling with a paired local relative gate.

This document is subordinate to the
[Goal G execution prompt](polymarket-authenticated-execution-goal-g-prompt.md),
the completed
[Goal F boundary](polymarket-product-connectivity-boundary.md), and
[architecture](architecture.md). Nothing here authorizes production order
entry, a real credential source, an authenticated external request, a real
Polygon request, or a real order.

## Reference Pins

The unchanged Chaos behavioral reference is the clean `../imm-strategy`
revision `b6b120c7b7c466d8431bf082f3229328c5d7b2ae`. It is normative only for
the existing Chaos/iarb2 path and must not be modified.

The historical PM implementation reference is the tracked `../predarb` object
`8222273a9c72033b760e1d2fec813bc77144556d`. It is a differential/reference
source, never protocol authority, architecture, or a dependency. Inspect only
that object through Git object commands. Record dirty path names but never
open, copy, reset, clean, or interpret Predarb dirty/untracked/runtime/secret
state; in particular do not read its modified dashboard or `.predarb/`.

## Fixed Product Profile

The intended product remains exactly:

```text
OKX configured public index price
              +
Polymarket configured public metadata/book
              +
Polymarket credential-visible user/order/fill/account observations
              +
finalized Polygon allowance/operator-approval observations
              |
existing pure PM model/state/readiness/risk
              |
durable one-token GTC post-only intent
              |
narrow Polymarket EOA place / exact-owned cancel
```

The frozen execution profile is Polygon chain `137`, CLOB V2,
`signatureType = 0`, and one configured EOA for which
`maker == signer == funder == POLY_ADDRESS`. The L2 credential bundle is
pre-provisioned for that EOA. The outer order `owner` is the L2 API-key UUID,
not the EOA. Orders are only `GTC`, `postOnly = true`, `deferExec = false`,
`expiration = 0`, `metadata = bytes32(0)`, and `builder = bytes32(0)`.
Prices, quantities, maker/taker amounts, tick, lot, and minimum remain the
exact Goal F integral values, with executable price strictly in `(0, 1)`.

Proxy, Safe, deposit-wallet/POLY_1271, session signer, builder attribution,
provisioning, heartbeat, batch orders, marketable orders, FOK/FAK/GTD,
cancel-all, allowance mutation, redemption, settlement, Predict.fun, and OKX
private/trading connectivity remain absent.

## Cryptographic Contract Proven By Current Sources

The CLOB V2 EIP-712 domain is:

| Field | Standard market | Negative-risk market |
| --- | --- | --- |
| `name` | `Polymarket CTF Exchange` | `Polymarket CTF Exchange` |
| `version` | `2` | `2` |
| `chainId` | `137` | `137` |
| `verifyingContract` | `0xE111180000d2663C0091e4f400237545B87B996B` | `0xe2222d279d744050d28e00520010520000310F59` |

The signed type is:

```text
Order(
  uint256 salt,
  address maker,
  address signer,
  uint256 tokenId,
  uint256 makerAmount,
  uint256 takerAmount,
  uint8 side,
  uint8 signatureType,
  uint256 timestamp,
  bytes32 metadata,
  bytes32 builder
)
```

Its type hash is
`0xbb86318a2138f5fa8ae32fbe8e659f8fcf13cc6ae4014a707893055433818589`.
The signed side is `BUY = 0`, `SELL = 1`; the EOA signature type is `0`;
the signed order timestamp is Unix milliseconds. Wire `expiration` is not in
the V2 signed struct. The expected venue order ID is the 32-byte EIP-712
digest produced by the contract's `hashOrder`; hexadecimal case is not
identity.

L2 request authentication uses one Unix-seconds timestamp and the exact bytes:

```text
timestamp + UPPERCASE_METHOD + route_path + exact_body_bytes_if_any
```

Query parameters are excluded from the signed route. The API secret is
base64-decoded, the preimage is HMAC-SHA256 signed, and the result is
URL-safe base64 with `=` padding. The five headers are `POLY_ADDRESS`,
`POLY_SIGNATURE`, `POLY_TIMESTAMP`, `POLY_API_KEY`, and
`POLY_PASSPHRASE`. POST lowering must serialize once, HMAC that final slice,
and transport the same slice.

The fixed outer POST body contains `order`, `owner`, `orderType`,
`postOnly`, and `deferExec`. The embedded wire order contains the signed
fields plus `expiration` and `signature`; wire side is `BUY` or `SELL`.
The EIP-712 signature and L2 HMAC are distinct authorities.

## Closed Live Capability Matrix

These IDs are additions. They never replace, alias, or reuse Goal F's
`PM-FAKE-*` IDs or fake-effect identity.

| Requirement ID | Exact role and production origin | Closed route/channel | Owner | Canonical lane | Readiness use |
| --- | --- | --- | --- | --- | --- |
| `OKX-LIVE-PUBLIC-INDEX-WS` | Configured public OKX index observation at `wss://ws.okx.com:8443` | `/ws/v5/public`; subscribe only to `index-tickers` for the configured `instId` | `reap-okx-public-source` socket/session worker | Public | Required reference value, subscription success, freshness, and epoch |
| `PM-LIVE-PUBLIC-METADATA` | Configured public CLOB metadata at `https://clob.polymarket.com` | `GET /clob-markets/{conditionID}` | `reap-polymarket-public-source` metadata worker | Public | Membership, lifecycle, tick, minimum, neg-risk domain, spender set |
| `PM-LIVE-PUBLIC-BOOK-SNAPSHOT` | Configured public CLOB book | `GET /book?token_id={token}` | PM public book worker | Public | Seed/resync and book-integrity fence |
| `PM-LIVE-PUBLIC-MARKET-WS` | Configured public CLOB market stream at `wss://ws-subscriptions-clob.polymarket.com` | `/ws/market`, configured `assets_ids` only | PM public socket/session worker | Public | Current book epoch, integrity, freshness |
| `PM-LIVE-PUBLIC-SERVER-TIME` | Public CLOB clock observation | `GET /time` | PM public clock worker | Reconciliation | L2/order clock offset and skew evidence only |
| `PM-LIVE-PUBLIC-GEOBLOCK` | Public geographic safety observation at `https://polymarket.com` | `GET /api/geoblock` | PM public safety worker | Critical | New placement fail-close input only |
| `PM-LIVE-USER-WS` | Authenticated credential-visible user stream at `wss://ws-subscriptions-clob.polymarket.com` | `/ws/user`; one initial auth frame, then configured market updates only | `reap-polymarket-live-adapter` private socket/session worker | Private | Order/fill occurrence and private epoch; never sufficient alone |
| `PM-LIVE-ACCOUNT-CUT` | Authenticated account read at `https://clob.polymarket.com` | `GET /balance-allowance` for `COLLATERAL` and configured `CONDITIONAL` token, `signature_type=0` | Authenticated account-read worker | Reconciliation | Collateral/token balance and per-selected-spender numeric cache/spendability evidence; never typed operator approval |
| `PM-LIVE-POLYGON-AUTHORIZATION-CUT` | Credential-free Polygon read; production origin deferred to Goal H | Closed chain-ID/finalized-anchor checks plus exact-block ERC-20 `allowance` and ERC-1155 `isApprovedForAll` calls | `reap-polymarket-chain-source` | Reconciliation | One fresh indivisible typed allowance/operator-approval cut |
| `PM-LIVE-POSITION-OBSERVATION` | Public address-scoped Data API at `https://data-api.polymarket.com` | `GET /positions` with exact user/market, `sizeThreshold=0`, bounded `limit`/`offset` | PM public position worker | Reconciliation | Monitored projection only; never atomic completeness or sell authority |
| `PM-LIVE-OPEN-ORDERS` | Authenticated credential-visible inventory | `GET /data/orders` with unfiltered credential scope and `next_cursor` | Authenticated reconciliation worker | Reconciliation | Complete credential-visible open-order cut |
| `PM-LIVE-ORDER-DETAIL` | Authenticated exact identity read | `GET /data/order/{orderID}` | Authenticated reconciliation worker | Reconciliation | Resolve expected, owned, ambiguous, or unmanaged identity; never creates ownership |
| `PM-LIVE-TRADES` | Authenticated credential-visible trades | `GET /data/trades` with unfiltered credential scope and `next_cursor` | Authenticated reconciliation worker | Reconciliation | Complete credential-visible maker/taker-leg fill cut |
| `PM-LIVE-PLACE-GTC-POST-ONLY` | One prepared fixed-profile mutation | `POST /order` | Linear authenticated execution edge | Critical result + Journal dispatch | One take-once prepared quote only |
| `PM-LIVE-CANCEL-OWNED` | Exact locally proven owned cancel | `DELETE /order`, body `{"orderID":"…"}` | Linear authenticated execution edge | Critical result + Journal dispatch | Cancel only one journal-proven venue identity |
| `PM-LIVE-RECOVERY-CANCEL` | L2-only recovery composition | Same exact-owned `DELETE /order`; no place or EOA signer | Recovery adapter composition | Critical result + Reconciliation | Reconcile and cancel proven owned identities only |

The current authenticated order/trade documents prove only
credential-visible scope. A complete unfiltered page walk proves absence only
for that credential. It does not prove funder-wide absence across another API
key, the UI, another process, a manual actor, or on-chain activity.

## Authorization Evidence Separation

The account route returns one shape for both asset kinds:

```text
balance: decimal string
allowances: map<spender address, decimal string>
```

Current official sources do not define how a `CONDITIONAL` map value encodes
ERC-1155 `isApprovedForAll`. In particular, they do not state that false/true
is `0/1`, `0/max_uint256`, or any other exact set. The official unified
TypeScript client parses both kinds as `bigint` and compares a conditional
value to maker amount, while its separate on-chain approval path correctly
decodes ERC-20 `allowance` as `uint256` and ERC-1155
`isApprovedForAll` as `bool`. The legacy TypeScript and current Rust clients
preserve the CLOB value as an opaque string. The OpenAPI schema adds no
conditional mapping.

An independent audit of additional official clients makes the distinction
stronger rather than resolving it. The current official Python SDK has a
SELL/`CONDITIONAL` unit vector whose CLOB allowance text is `"777"` and whose
expected parsed value is the integer `777`. The official Python CLOB clients
otherwise return the response unchanged. The official CLI likewise prints
the authenticated CLOB allowance map unchanged, while a separate approval
command reads ERC-20 `allowance` as `U256` and ERC-1155
`isApprovedForAll` as `bool` directly on-chain. Official examples use the
same direct boolean ERC-1155 call. No official source converts the numeric
CLOB value into that boolean.

The amended safe model is therefore distinct facts, not a conversion:

1. CLOB-reported balances and numeric, per-selected-spender
   spendability/cache values;
2. direct on-chain ERC-20 allowance for the configured EOA and selected
   exchange; and
3. direct on-chain ERC-1155 operator approval for the same owner and exchange.

The first cannot establish either direct chain fact. The two chain results are
the typed readiness authority. An exact source-proven CLOB numeric amount may
only add an insufficient-spendability fail-close fence; it can never grant
readiness or become a boolean. If its comparison unit remains unproved, retain
the canonical bounded selected-spender number as diagnostic evidence and do
not compare it; that ambiguity is no longer a Goal G stop.

Therefore the following are forbidden:

- treating any positive conditional value as approved;
- treating the first allowance-map entry as the selected exchange;
- converting an amount threshold to a boolean;
- using Predarb's historical positive-value inference;
- calling the allowance-cache update route; or
- weakening Goal F's tagged `Erc1155OperatorApproval`;
- exposing a caller-selected spender, contract, calldata, block tag, or
  JSON-RPC method; or
- falling back from a failed chain cut to CLOB cache state.

The documented current-source conflicts also include:

- REST order/trade status spellings differ between OpenAPI, guides, WS
  documentation, and current SDK models;
- user-WS timestamp prose says milliseconds while examples include
  seconds-shaped values;
- older rendered route families redirect to newer consolidated documents;
  and
- the public Data API has no atomic multi-page snapshot/fence.

A source/message-family-tagged union plus quarantine is the explicit amended
contract. Phase 0 freezes the exact lexical kind, token, unit, normalized
meaning, and provenance for every reached field. The union keeps POST result,
REST order, user-WS order, and REST/WS trade-settlement namespaces distinct.
Only enumerated equivalents normalize. A timestamp accepts canonical
10-digit seconds, 13-digit milliseconds, and/or another exact documented
lexical form such as RFC 3339 only when its own field table allows it; checked
conversion never guesses by magnitude. Unknown, malformed, cross-family,
out-of-profile, or ambiguous values are boundedly quarantined, halt placement,
retain reservations, and force reconciliation. They never become pending,
open, zero, or success. A complete source-compatible cut alone may clear
quarantine; overflow or permanent ambiguity is an operator halt. Quarantine
retains only sanitized bounded field/family/identity evidence, never
credential-bearing raw frames, auth material, or unbounded raw errors.

## Closed Polygon Authorization Cut

The chain source performs one exact ordered sequence:

1. at the start of every cut, `eth_chainId` returns canonical `0x89`; mismatch
   also invalidates the transport epoch;
2. `eth_getBlockByNumber("finalized", false)` returns a non-null anchor;
3. at the anchor's exact hex block number, `eth_call` targets Goal F pUSD with
   selector `0xdd62ed3e` for
   `allowance(configuredEoa, selectedExchange)`;
4. at that same exact block, `eth_call` targets Goal F Conditional Tokens with
   selector `0xe985e9c5` for
   `isApprovedForAll(configuredEoa, selectedExchange)`; and
5. `eth_getBlockByNumber(exactNumber, false)` returns the same number/hash.

The selected exchange comes only from the already validated Goal F standard
or negative-risk metadata. Chain `137`, owner, pUSD/CTF contracts, exchange,
selectors, left-zero-padded arguments, JSON-RPC IDs/order, and block tag are
closed private values. There is no batch, caller-supplied address/data/tag,
fallback to another block, or generic RPC surface.
The fixed call targets are pUSD
`0xC011a7E12a19f7B1f670d46F03B03f3342E82DFB` and Conditional Tokens
`0x4D97DCd97eC945f40cF65F87097ACe5EA0476045`; the selected spender/operator
is the standard or negative-risk V2 exchange already listed in the signing
table.
Each request is one bounded JSON-RPC `2.0` POST object with a deterministic
nonzero integer ID and no notification or batch; each response must carry
`jsonrpc = "2.0"` and the exact matching ID.

Each result is exactly one 32-byte ABI word. ERC-20 allowance is unscaled
`U256`; ERC-1155 approval accepts only canonical zero or one. The whole fact
binds chain, finalized block number/hash/timestamp, local monotonic observation
time, transport/account/configuration epochs, owner, exchange, contracts, and
both typed results.

The finalized timestamp may be at most five seconds in the future and thirty
seconds old; the completed cut expires after five monotonic seconds or any
transport/account/configuration/metadata epoch change. Wrong chain, null or
changed block, revert/error/ID mismatch, malformed/noncanonical/oversized
response, timeout, stale/future evidence, unsupported finalized or historical
call, redirect/proxy, or any partial sequence discards the cut and makes
readiness false. A retry is a fresh whole cut.

Clock access is a private edge capability, not caller-supplied canonical state.
`local-evidence` uses a fixed deterministic synthetic wall/monotonic clock for
freshness and replay tests. Goal H must bind reviewed target-host clock
discipline. Process-monotonic instants are never serialized, journaled, or
compared across restarts; only typed freshness and epoch transitions enter the
canonical durable projection.

Goal G selects no provider, production origin, or provider credential and
sends no real chain request. It exposes only the non-default `local-evidence`
constructor, which accepts numeric loopback addresses and rejects DNS and
non-loopback targets. Goal H must add one reviewed exact HTTPS origin and
prove chain `137`, JSON-RPC `2.0`, `finalized`, historical exact-block calls,
response/time/rate bounds, provider credential custody if any, and disciplined
target-host wall-clock behavior. The source
cannot access auth/signers or expose transactions, `approve`,
`setApprovalForAll`, `eth_send*`, CTF mutations, raw clients/responses, or
provider selection.

## User WebSocket And Pagination Contract

The current user-WS initial frame is exactly:

```json
{"auth":{"apiKey":"…","secret":"…","passphrase":"…"},"type":"user"}
```

A configured condition subset adds `markets`. Later changes use exactly:

```json
{"operation":"subscribe","markets":["0x…"]}
```

or `unsubscribe`. Wire events may be one object or an array. Raw event
families use `event_type = order|trade`; order `type` is
`PLACEMENT|UPDATE|CANCELLATION`; trade lifecycle is
`MATCHED|MINED|CONFIRMED|RETRYING|FAILED`. Text `PING` is sent every ten
seconds and expects `PONG`. No official subscription acknowledgement is
specified, so transport-open is not account readiness. Full REST
reconciliation is still required. The credential-bearing initial frame is
never captured.

Current V2 clients start order/trade pagination with cursor `MA==` and stop
at `LTE=`. Reap must reject a repeated/cyclic/malformed cursor, an unexpected
terminal convention, page-limit exhaustion, partial page, or aggregate bound
overflow. Data positions use bounded `limit`/`offset`, `sizeThreshold=0`, and
remain non-atomic monitored evidence.

## Dependency And Ownership Shape

The intended acyclic graph is:

```text
reap-pm-live
  -> reap-polymarket-public-source -> reap-polymarket-wire/core/transport
  -> reap-okx-public-source        -> reap-core/transport
  -> reap-polymarket-chain-source  -> reap-pm-core/transport
  -> reap-polymarket-live-adapter  -> reap-polymarket-auth/wire/core/transport
  -> reap-polymarket-adapter       -> reap-polymarket-wire/core/transport
```

Responsibilities are fixed:

- `reap-polymarket-auth`: non-cloneable secret holders, L2 HMAC, EOA V2
  signing, expected order identity; no network or strategy.
- `reap-polymarket-public-source`: extracted PM public metadata/book/session
  plus public position/time/geoblock transports; credential-free.
- `reap-polymarket-chain-source`: closed chain-ID/finalized-anchor and two
  exact authorization calls; private bounded JSON-RPC/ABI, no auth, mutation,
  arbitrary RPC, canonical state, or Goal G production-origin constructor.
- `reap-polymarket-live-adapter`: closed private REST/user-WS, account and
  reconciliation parsers, CLOB numeric account evidence, one place profile,
  exact-owned cancel; no chain/public-market duplication or canonical state.
- `reap-polymarket-wire`: credential-free DTO/parsing only; no full signed
  outer body, API-key owner, signature, secret, signer, or client.
- `reap-polymarket-adapter`: fixture/fake roles only after mechanical public
  extraction; no compatibility re-export to a network/auth capability.
- `reap-pm-live-contracts`: secret-free requirement and route identities.
- `reap-pm-live`: sole canonical owner and consumer of prepared effects and
  durable dispatch grants.

The authenticated adapter never depends on `reap-pm-live`, receives an upper
prepared-effect/grant type, exposes an arbitrary request, or owns canonical
order/position state. No PM auth or Polygon chain role enters Chaos
`reap-live`, `reap-order`, `reap-venue`, or `reap-cli`.

No Goal G production file may grow any existing file at or above 1,400 lines.
The four current protected files are `capture_roles.rs` (1,490),
`coordinator/mutation.rs` (1,466), `private_monitor.rs` (1,447), and
`reap-polymarket-adapter/public_session.rs` (1,440). New production files are
limited to 1,000 lines without an approved responsibility exception and
hard-stop at 1,500; functions require decomposition review above 200 lines and
hard-stop at 250.

## Secret Lifecycle And Threat Model

Secrets enter only at the authenticated composition root after non-secret
configuration validation, journal lease acquisition, and local recovery.
Reconciliation loads the narrow L2 bundle; the EOA signer is loaded only
after reconciliation and immediately before the execution edge is eligible.

Each input is bounded and held in a non-`Clone`, non-`Copy`, non-`Debug`,
non-`Display`, non-`Serialize`, zeroizing value. One account-scoped edge owner
owns the values. Purpose-specific methods may create an L2 header set,
user-WS auth frame, or EIP-712 signature, but no getter or general signing
oracle exists.

Secrets, auth frames, headers, and replayable signed bodies are excluded from
configuration projections, URLs, queries, logs, errors, panic messages,
metrics, capture, journal, snapshots, fixtures, and evidence. Reap-owned
transient buffers are cleared promptly and final owners zeroize on drop.
This does not claim erasure from third-party crypto/TLS/HTTP libraries,
allocators, the OS/kernel, swap, core dumps, DMA, privileged processes, or a
compromised host.

Production exchange transports allow only the exact HTTPS/WSS origins in the
matrix, disable redirects and ambient proxies, and reject userinfo/alternate
ports, custom trust bypass, downgrade, and cross-origin credential forwarding.
The sole Goal G origin seam is the non-default `local-evidence` feature:
numeric `127.0.0.0/8` or `::1` only, no DNS/non-loopback/proxy/redirect, and
enabled by no default, deployable binary, service, or production dependency.
This evidence feature supports external integration tests/benches without
becoming arbitrary origin injection. Goal G has no default/production Polygon
origin constructor. Goal H must bind one exact HTTPS origin under the same
redirect/proxy/userinfo/downgrade/trust rules and define custody separately if
the provider needs a credential.

Every external target using the seam declares
`required-features = ["local-evidence"]` and is rerun explicitly; no default
workspace or deployable target enables it.

Threats explicitly fail closed:

| Threat | Required response |
| --- | --- |
| Secret in a public/debug/serialized value | Compile/source-policy failure |
| Auth failure, wrong credential scope, or WS reconnect | Halt placement; replace epoch; reconcile |
| Body serialized again after HMAC | Impossible by type/ownership; test must fail |
| Queue overflow or stale dispatch grant | Do not send; persist typed failure/halt |
| Timeout/partial write/disconnect after possible send | Acknowledgement unknown; never blind retry |
| Unknown order/fill/status/timestamp shape | Quarantine and halt/reconcile |
| Unknown/unmanaged remote order | Keep unmanaged; never claim or cancel |
| Partial page/cut | Discard as incomplete; never mark ready |
| Position API absence/equality | Monitored divergence only; never grants authority |
| Missing/wrong/unknown allowance kind | Unready |
| CLOB numeric value used as boolean | Compile/source-policy or readiness failure |
| Chain wrong/stale/partial/reorg/malformed cut | Discard whole cut; unready |
| Arbitrary RPC, provider selection, or chain mutation | Compile/source-policy failure |

## Journal And Recovery Plan

Goal F's `reap-pm-mutation-journal` version 1 and every byte remain frozen.
Authenticated execution requires a distinct family, provisionally named
`reap-pm-authenticated-mutation-journal` version 1, bound to the public EOA
account scope, chain, environment, configured market/token, and an
operator-provided non-secret credential-slot identity. It must not record an
API key, secret-derived hash, passphrase, private key, auth header, user-WS
frame, or full signed body.

This schema, lease, durable writer/barriers, and recovery projection must land
as the first Phase 4 tranche before any live place/cancel role or crash test.
Phase 5 composes the already-proven journal into startup, recovery, and
shutdown; it does not introduce durability after mutation exists.

The minimum durable transition is:

```text
canonical intent + reservation
-> intent durable
-> signed-order identity + body SHA-256 commitment returned from edge
-> request commitment durable
-> dispatch-authorized/may-have-sent barrier durable
-> take-once grant
-> at most one application dispatch attempt
-> typed post-result fact durable
```

The commitment binds method, route, exact query contract, body commitment,
auth timestamp, expected order ID, and monotonic send-before deadline.
Recovery treats any consumed or durably granted barrier without a conclusive
post-result as acknowledgement unknown. It reconciles exact expected order,
all credential-visible open orders, and trades. It either converges to one
known state or durably retains the exact slot/identity as operator-required.
Cancellation may be repeated only for the identical proven-owned order after
read-only reconciliation still proves it live and the frozen protocol permits
idempotent exact cancellation.

## Lane And Bounded-Resource Plan

Existing Goal F service priority remains
`Critical > Persistence > Private > Scheduled > Public > Reconciliation >
Telemetry`. New facts receive deterministic subranks within those lanes;
equal-rank facts use the existing canonical identity/ingress ordering.

| Lane | Capacity | Nominal high water | Max age | Goal G contents | Saturation |
| --- | ---: | ---: | ---: | --- | --- |
| Critical | 512 | 32 | 250 ms | auth/safety faults, mutation result, acknowledgement unknown | Global/account stop; never drop |
| Persistence | 512 | 32 | 250 ms | request-preparation and dispatch durability acknowledgements | Global stop; no dispatch |
| Private | 4,096 | 64 | 250 ms | user order/fill occurrences | End epoch, halt account, reconcile |
| Scheduled | 4,096 | 64 | 100 ms | quote/cancel evaluation | Suppress quote and cancel owned |
| Public | 8,192 | 256 | 500 ms | PM and OKX public observations | Invalidate stream and resync |
| Reconciliation | 128 | 16 | 5 s | complete order/trade/CLOB-account/finalized-chain/position cuts | Remain unready; retry boundedly |
| Telemetry | 128 | 32 | none | non-authoritative metrics | Coalesce/sample only |
| Reconciliation request | 128 | 16 | 1 s | exact refresh requests | Retain pending refresh |
| Capture | 8,192 | 256 | 500 ms | credential-free raw public frames only | Invalidate capture and resync |
| Journal | 1,024 | 128 | 1 s | non-secret durable mutations | Halt quote/dispatch |
| Prepared effect | 256 | 32 | 250 ms | move-only quote/cancel authority | Reject/halt; never lose approved effect |

Additional hard bounds for the resumed Phase 0 are one MiB per raw
frame/HTTP response, 32 MiB aggregate raw bytes, 64 KiB mutation request body,
8 KiB per header/field aggregate, 500 rows per venue page, 64 pages per
request cut, 1,024 live/unresolved orders, and 8,192 retained fills. Aggregate
bounds win over page maxima.

The target-host-neutral deadline plan is five seconds each for connect/TLS,
write, and first byte; ten seconds total REST; ten-second WS ping, five-second
pong, and thirty-second idle/reconnect fault; five-second maximum server skew
with a thirty-second offset TTL. A dispatch grant expires after 250 ms before
the first application write. Reads may use bounded classified retry;
placement never retries after bytes may have reached the venue.

Pacing must remain below official ceilings: at most two public REST requests
per second, five credentialed reads per second, five exact mutations per
second, and one reconnect attempt per five seconds, all with a burst no
greater than the same one-second allowance and an explicit one-second queue
age. These are conservative library bounds, not target-host performance
claims; a resumed Phase 0 must revalidate them against the then-current
official sources.

The chain source permits one in-flight cut and at most one fresh whole-cut
attempt per second. It does not retry an individual call or reuse a partial
result; a classified retry schedules a new whole sequence. These local bounds
are independent of any future provider ceiling.

## Local Performance Contract

The legacy PM action `25,000 ns` p50 and `250,000 ns` p99.9 absolute exits are
superseded. Phase 0 may change only the latency branch at
`crates/reap-pm-live/src/evidence/runner.rs:81`,
`crates/reap-pm-live/benches/pm_action_path.rs`, and their policy tests to
remove those two exits, preserve the `15,000`-sample and every
logical/hash/allocation/memory/cardinality/queue gate, leave workload/timed
boundary/report schema unchanged, and emit the complete report.

The unchanged Phase 0 workload and final candidate each run as four separate
idle-host Cargo invocations on the same host/toolchain/profile/boundary: one
complete process-warmup suite is discarded from comparison but retained, then
three invocation reports are compared. Each PM invocation already contains
one internal warm-up and three internal recorded distributions. For each
invocation/quantile take the median of its three internal values, then take
the median of the three retained invocation medians. Final p50 and p95 must
each be at most `1.10 ×` baseline and p99 at most `1.20 ×` baseline.
p99.9 and max are retained but not shared-host pass/fail gates. Predeclared
overlap/toolchain/profile/workload mismatch invalidates a run before values are
read and remains recorded; scheduler variance is not contamination. New
signed-request and chain-source benchmarks have hard correctness/resource
gates and report all quantiles/max, but establish local baselines rather than
absolute latency gates. None of this is a target-host or network SLO.

## Amendment Adopted

The user-authorized Amendment 1 is complete at the contract level:

1. the closed finalized-chain authorization cut above supplies the two typed
   facts without a CLOB numeric-to-boolean conversion;
2. CLOB numeric values remain separate diagnostic/fail-close evidence;
3. the strict source-tagged lifecycle/time union makes known documentation
   disagreement representable without guessing;
4. the paired local PM benchmark rule replaces the invalid host-specific
   ceiling while preserving exact work/resource gates; and
5. the capability, dependency, lane, failure, and production-origin boundaries
   have been revised consistently.

Goal G may resume at Phase 0. A production Polygon origin, real-account probe,
real credential, authenticated external call, chain call, or order remains
outside Goal G.
