# Polymarket Authenticated Execution Boundary

Status: **amended implementation contract for Goal G under Amendments 1-3;
Amendment 3 is authorized but inactive until Goal G-R Amendment 6
completes**. The original
Phase 0 stop remains historical evidence in the
[Goal G handoff](polymarket-authenticated-execution-goal-g-handoff.md).
Amendment 1 resolves it without weakening the typed core: CLOB numeric
spendability/cache evidence remains numeric, while a separate closed Polygon
source reads ERC-20 allowance and ERC-1155 operator approval directly at one
finalized block. The same amendment authorizes a strict source-tagged
lifecycle/time compatibility union and replaces the host-specific PM latency
ceiling with a paired local relative gate. Amendment 2 freezes the exact
route-specific union discovered by the restarted Phase 0 audit, including the
REST-only `MatchedNotBroadcast` settlement state, one canonical lane per
requirement ID, explicit no-retry HTTP construction, and the exact dependency
and protocol-vector plan.

## Amendment 3 Phase 0 Supersession — 2026-07-29

The old runnable Phase 0 helper/root instructions later in this file are
historical-only and must not be invoked. After Goal G-R Amendment 6 closes
prospectively, the exact activation, separate recorder bundle, fresh evidence
and runtime roots, source successor, current baseline, and one fresh replay
are controlled by:

- [Goal G Amendment 3](polymarket-authenticated-execution-goal-g-amendment-3.md);
- its [runner and command contract](polymarket-authenticated-execution-goal-g-amendment-3-runner-contract.md);
- and the [return-sequence prompt](polymarket-authenticated-execution-goal-g-resume-prompt.md).

The historical `target/tmp/goal-g-phase0-amended` root remains immutable and
red. This supersession changes no product, authentication, secret, Polygon,
order-entry, deployment, capability, or connectivity boundary below.

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

## Restarted Phase 0 Source And Vector Pins

Fresh public captures were taken on 2026-07-27 without credentials or a
production/Polygon request. All 33 official-document responses were HTTP 200.
The retained ignored evidence manifests are:

| Evidence | Rows | SHA-256 |
| --- | ---: | --- |
| `authoritative-source-manifest.tsv` | 128 | `f38625a6f2bb0a2c8e13598acf6ab7dc1eccc57f97a7f4a8c45fdb810e8fcb4d` |
| `official-docs/manifest.tsv` | 33 | `9f8c0543fe7c7b68a032eb82fc6682ecf2b4f38be40ff44cc07db669fc53f0c8` |
| `official-git/manifest.tsv` | 53 | `28557e41795e49e3259692eb2d7cf2f564a5100b12b6948c9adcf8a5a314229d` |
| `official-git/supplement-manifest.tsv` | 7 | `3d5902c0df75d600c13e6c3407496778767a186eb5860424930f3ad9c2cde6eb` |
| `official-git/addendum-manifest.tsv` | 28 | `7a6328ffbe8daa27f7f99cca2c3c2f34f7ed234faf797b6d1fa958c536014a67` |

The evidence root is exactly
`target/tmp/goal-g-phase0-amended`. These ignored executables are part of the
reviewed contract and may not be edited or replaced during the run:

| Executable | SHA-256 |
| --- | --- |
| `run-benchmark-invocation.sh` | `40842c9e0ece1c5990c074d093f016f9b8076f87e8dc921653f4765bb38a3747` |
| `summarize-baseline-campaign.sh` | `9f3c897fb0f9913ed7bafa882e3b3a6402d4b67a97b7d24c7e52b90f24d868f1` |
| `verify-source-cutoff.sh` | `ffa352b883f1d00b9f8dde6ce40566f4dcd137f0c90ea6aaca7f78bde900f713` |
| `run-phase0-replay.sh` | `d951f757890c1a270d593963d5d91393c56afdf99e62b1bc7c1eaa8301d207bb` |

Before any helper runs, execute this exact immutable-helper gate from the
repository root:

```bash
e=target/tmp/goal-g-phase0-amended
printf '%s  %s\n' \
  40842c9e0ece1c5990c074d093f016f9b8076f87e8dc921653f4765bb38a3747 "$e/run-benchmark-invocation.sh" \
  9f3c897fb0f9913ed7bafa882e3b3a6402d4b67a97b7d24c7e52b90f24d868f1 "$e/summarize-baseline-campaign.sh" \
  ffa352b883f1d00b9f8dde6ce40566f4dcd137f0c90ea6aaca7f78bde900f713 "$e/verify-source-cutoff.sh" \
  d951f757890c1a270d593963d5d91393c56afdf99e62b1bc7c1eaa8301d207bb "$e/run-phase0-replay.sh" |
  sha256sum -c -
```

The 128-row manifest is the restarted Phase 0 authority. It was generated by
the retained script whose SHA-256 is
`a25e2e3dcad149d774c24b7f367bd9ec7211a0e70dab4833f9b2fcbd269abcb6`.
The three input Git manifests remain historical/subset evidence rather than
being silently rewritten.

Re-attestation is byte verification, not merely concatenation. From the
evidence root, the gate must:

1. verify the four input-manifest hashes/row counts and the build-script,
   vector-generator, and vector hashes recorded here;
2. for each official-document row in order, verify the corresponding
   `official-docs/NNN-*.body` byte count and SHA-256;
3. verify `official-git/001.body` through `053.body` against the main Git
   manifest and `054.body` through `060.body` against the supplement,
   including byte count, Git blob ID from `git hash-object --stdin`, and
   SHA-256;
4. credential-free fetch every addendum path at its exact recorded revision,
   never a branch name, from its fixed public repository, and verify the same
   three values before discarding the temporary copy. The repository mapping
   is `Polymarket/{ctf-exchange-v2,ts-sdk,clob-client-v2,
   rs-clob-client-v2,polymarket-cli,py-clob-client}`,
   `seanmonstar/reqwest`, `RustCrypto/{elliptic-curves,hashes}`, and
   `sindresorhus/ky`;
5. rebuild the authoritative manifest, require exactly 128 sorted rows, one
   schema row, no duplicate non-schema row, and exact SHA-256
   `f38625a6f2bb0a2c8e13598acf6ab7dc1eccc57f97a7f4a8c45fdb810e8fcb4d`;
   and
6. retain the verifier log and its SHA-256. Public `HEAD`/package-version
   lookups after that are diagnostic drift checks only. They cannot replace
   pinned bytes or widen the cutoff; relevant drift requires a reviewed
   amendment.

Execute the verifier exactly once from the clean repository root after the
Amendment 2 pre-gate commit:

```bash
(
  set -e
  e=target/tmp/goal-g-phase0-amended
  available_bytes=$(df --output=avail -B1 "$e" | awk 'NR == 2 {print $1}')
  test "$available_bytes" -ge 268435456
  test ! -e "$e/source-reattest.log"
  test ! -e "$e/source-reattest.log.sha256"
  set +e
  "$e/verify-source-cutoff.sh" >"$e/source-reattest.log" 2>&1
  rc=$?
  set -e
  sha256sum "$e/source-reattest.log" >"$e/source-reattest.log.sha256"
  exit "$rc"
)
```

A nonzero verifier exit is valid red evidence and stops Phase 0; do not
delete or replace the log. The retained `viem/package.json` hash is package
identity evidence only, not a claim that the installed package tree is a
complete source archive. The independently reviewed generator and fixed
vectors, plus narrow-Rust independent reproduction in Phase 2, are the
cryptographic evidence.

Current public revisions were recorded as:

| Repository | Revision |
| --- | --- |
| `Polymarket/ctf-exchange-v2` | `ccc0596074f4dfd62c944fbca4de252893b82b4b` |
| `Polymarket/ts-sdk` | `0760f99f04e879164fafe79d8277395bb200cee9` |
| `Polymarket/clob-client-v2` | `f3e1a05f868a1fd0c34ef85dfc45c6ce78f5bb69` |
| `Polymarket/rs-clob-client-v2` | `222143d321eba97d5711a848265eb9aab3bc7ff4` |
| `Polymarket/polymarket-cli` | `9b18b5faf5493b945c48ca22efaf9645f0c69ab8` |
| `Polymarket/py-clob-client` | `b076b04d61135657e25dccc1bbd6866a96bd8c6e` |
| `Polymarket/py-clob-client-v2` | `215fc63a8fd6ec3a10c7edb73997c9772d8686d3` |
| `Polymarket/py-sdk` | `6a8f73267f3e776c1d2e8abed538dd5f3fbcda00` |
| `Polymarket/polymarket-sdk` | `a8401892976b3cbff0acfaf1c277aaddb241d5a4` |

Critical immutable source identities include:

| Contract evidence | Git blob / content SHA-256 |
| --- | --- |
| `ctf-exchange-v2/.../Structs.sol` | `0bbcd991063772a864bfe4c51679b7d589559d76` / `533fe017a934e9f7500519961f1b7d350c2e76732ca66cb837bd82406854c8c2` |
| `ctf-exchange-v2/.../Hashing.sol` | `a3dac60d83eef73893441bee174284d071346aa5` / `5f322bb6c3ea50843f00ae8cd66a7818256f09e5a05ac3873d9a0373d40f2100` |
| `ctf-exchange-v2/.../Signatures.sol` | `ff7c86a26c5a19be8148292c3c46bd4069bad105` / `2762905bfe7fedeafdf5ec6453c8f604f7ad0ec2a09f3299faa5f58738af9261` |
| `ts-sdk/.../hmac.ts` | `84084a47c55b8fc97a2f3ac2c1dd8bb8da5de90a` / `30d275108a73e6331b309827c59ded2513a9930e199ee0d775162027631f5f58` |
| `ts-sdk/.../post.ts` | `228db2114018d374cdbdff6e66e2ecee9b70c2a2` / `1ef134f22113dd27f66a487762c8eae43188ba0589044b7c06fa25401e2c1e9f` |
| `ts-sdk/.../cancel.ts` | `1351adae25b93428782ee2d54bf7a416a1f3d784` / `2447133eaabbb076f0bcee7005b649985fc4140b9b9e2e0602abb03184038def` |
| `rs-clob-client-v2/src/auth.rs` | `c99ff3f68cb35752716ff322ce3f6b717e6b1390` / `14beca313f27276591ac1f5ec1a629f6e9043b1532401880ebcb4237c3621221` |
| `py-sdk/.../account.py` | `d29be8a8d14d2dd340e250f567df3c6c87a1e089` / `ffe599cb2ff04e5b5a29d18e770489a43abdae92777d5cf84d366fe6135691ee` |

The fresh OpenAPI body SHA-256 is
`0f56ba4f6459d586636a18687fe05d3b5675bd7e707c7160f1a7aeb3306de070`.
It, the unified TypeScript SDK, and the current Rust client agree that the V2
inner wire order has no `taker`, `nonce`, or `feeRateBps`. The older
`clob-client-v2` still emits an extra `taker`; that conflict is explicitly
resolved in favor of the three-source current shape. The historical Python
client's order builder remains V1 and is used only to corroborate L2
exact-body behavior, never as a V2 order authority.

### Independently authored Phase 2 vector specification

The public Hardhat test key is
`0xac0974bec39a17e36ba4a6b4d238ff944bacb478cbed5efcae784d7bf4f2ff80`
and its account is `0xf39Fd6e51aad88F6F4ce6aB8827279cffFb92266`.
It is test-only data, not a claim that a valid key can never be authorized.
All order vectors use salt `479249096354`, token `1234`, millisecond
timestamp `1780449126930`, signature type zero, and zero metadata/builder.
BUY uses maker/taker `5200000/10000000`; SELL uses
`10000000/5200000`.

| Vector | Domain separator | Struct hash | Expected order ID | Signature |
| --- | --- | --- | --- | --- |
| Standard BUY | `0x3264e159346253e26a64e00b69032db0e7d32f94628de3e6eecb50304d7af3d2` | `0x600e0697b4d487190e10b8f3a79b4489c8d172ac41fefac6efc4a00b459a3b2e` | `0xfaf10599783c69b375a0f0d948d37eb711ec042dbf7d52fc2f8d8832d71af7f1` | `0xbb81b245ea7ebb9aa480ccbf15364a2cb2cd77d7adebcb56fd5f49b653683110055a3d5ad05adf1aa65b1701bf25c622275f098fd5724c7f782671829e6d4d0b1b` |
| Standard SELL | same | `0x8633966131c65c5cabe59dee955d024d6406ff2fee8fc4e6cb1c74c00a1f6866` | `0x4983a6499acac0e05a059b91ca92f61885b4d0327e1031570aa54ff85bc0af88` | `0x2a2a3b104cea6c5b4645ecddd73cec80ac82c8dd030d704be85f640a1dfefdb14d6fe935e87708d79625e97e660454292919af24f886b9f37ef3403b10962b101b` |
| Negative-risk BUY | `0x9b858f53327b0bd13af8ec14cfb35234fb9eb7b0504d1a4e61f433840d30e81a` | BUY hash above | `0x51541d6f12464aff462c280fc2fd0c73a0e0959752cc4e8f6e32c5c3107fc8e7` | `0xe3c2789e5a479cc64032caccf2e124eba6ae292d2e67e070df75bac13dfcfa5a65b1245dec0c40f91be50d1622bbfc079533c3e58d157ba73fcfd7799aa81f371b` |
| Negative-risk SELL | same | SELL hash above | `0x192ba059050c799921996285a4c182309e64843af24b77f1f7f0507dc3d15899` | `0x72ef19c759a0e9bd0ff6faa55e93ffc6c2f761605a128ad00599861d54413cda35302af513f457eac180d8259d8c320ee07f989a1addd8120713a2caea95ac211c` |

The synthetic L2 key ID is
`00000000-0000-4000-8000-000000000001`, secret
`AAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAA=`, and auth timestamp
`1780449126`. The GET signed route is `/data/orders`, its transport target is
`/data/orders?next_cursor=MA%3D%3D`, its 25-byte preimage SHA-256 is
`fadf7e28435ebfd0ea70589a8969ad414f6d22bafcb0cccb49078b5ba1bb2216`,
and its HMAC is
`-PRhfdrU6Jmzz04syaATDpRblz8zwfPYnigpmfrQVEE=`.
The compact POST body specified below is 685 bytes with SHA-256
`6a016c150aa3605270289489beff1d54d735952db64948d2bb3c1e01beb0ae9f`;
its 705-byte preimage SHA-256 is
`b3dea6c7c5efbacfd77c1a3b3a08242f33013b279ce5871d15164128b4402574`,
and HMAC is `rdkpCVcu-66xB2VbkOlUXQ2PLaCeqv3LjgFBfkrQdqo=`.

```json
{"deferExec":false,"order":{"builder":"0x0000000000000000000000000000000000000000000000000000000000000000","expiration":"0","maker":"0xf39Fd6e51aad88F6F4ce6aB8827279cffFb92266","makerAmount":"5200000","metadata":"0x0000000000000000000000000000000000000000000000000000000000000000","salt":479249096354,"side":"BUY","signature":"0xbb81b245ea7ebb9aa480ccbf15364a2cb2cd77d7adebcb56fd5f49b653683110055a3d5ad05adf1aa65b1701bf25c622275f098fd5724c7f782671829e6d4d0b1b","signatureType":0,"signer":"0xf39Fd6e51aad88F6F4ce6aB8827279cffFb92266","takerAmount":"10000000","timestamp":"1780449126930","tokenId":"1234"},"orderType":"GTC","owner":"00000000-0000-4000-8000-000000000001","postOnly":true}
```

The body is the bytes between braces only, with no trailing newline.

The 80-byte cancel body SHA-256 is
`90c066349138ab5e31d12e10006859cb239f8cbab5b46e5b0bec00c974536022`;
its 102-byte preimage SHA-256 is
`5862ace8738386840671e10398dab1e19a557b2c9235501942a3cb61e92fbea1`,
and HMAC is `beMewzBZba3V05Un8qQtGM0NYW4KFt4wKc8KqZcdo_M=`.

```json
{"orderID":"0xfaf10599783c69b375a0f0d948d37eb711ec042dbf7d52fc2f8d8832d71af7f1"}
```

The cancel body likewise has no trailing newline.

The ignored generator and vector JSON SHA-256 values are respectively
`51987e80fd70f87a80cb8b4d015727f68f05e93596ab319be09f84343ba34aee`
and `74a662b47fe33f57e3adda5826ba8b7ea09b3658aeaa5ce893d1e142ae1b1f9d`;
the oracle is `viem 2.46.3`, matching the pinned current official client.
Phase 2 must check in a reviewed fixture and independently reproduce Keccak,
RFC6979 low-s recoverable secp256k1 signatures, HMACs, bodies, and every
negative vector with the narrow Rust implementation. Agreement with this one
oracle alone is not a green gate.

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
canonical padded RFC 4648 base64url matching
`(?:[A-Za-z0-9_-]{4})*(?:[A-Za-z0-9_-]{2}==|[A-Za-z0-9_-]{3}=)?`,
nonempty, and limited by Reap to 128 decoded bytes. Decode with the padded
URL-safe engine, re-encode, and require byte-for-byte equality. Whitespace,
standard-base64 `+` or `/`, omitted/excess/interior padding, ignored junk,
empty input, and a decoded value above the local cap are rejected. The
preimage is HMAC-SHA256 signed. Its 32-byte output is padded base64url, so
`POLY_SIGNATURE` is exactly 44 ASCII bytes with one terminal `=`. The same
decode/re-encode equality is tested on output.

The five authentication headers are `POLY_ADDRESS`, `POLY_SIGNATURE`,
`POLY_TIMESTAMP`, `POLY_API_KEY`, and `POLY_PASSPHRASE`.
`POLY_ADDRESS` is the configured type-0 EOA in its canonical EIP-55 spelling;
the API key is exactly 36 lowercase ASCII bytes matching
`^[0-9a-f]{8}-[0-9a-f]{4}-[0-9a-f]{4}-[0-9a-f]{4}-[0-9a-f]{12}$`, with no
invented UUID version/variant restriction; `POLY_TIMESTAMP` is the canonical
ten-digit value defined below; and the passphrase follows the explicit
Reap-local holder grammar `^[A-Za-z0-9._~-]{1,128}$`. This passphrase grammar
is a conservative local acceptance policy, not a claim that the venue
documents every possible provisioned value. Whitespace, non-ASCII, control,
JSON-escape-requiring, empty, or over-cap passphrases are rejected at
injection. The HTTP header transports the semantic passphrase bytes raw. The
compact user-WS serializer consumes the same holder value; the grammar means
those bytes appear unchanged between JSON quotes.

The injected EOA private-key text is exactly lowercase
`^0x[0-9a-f]{64}$`, 66 ASCII bytes. Decode once, require a nonzero secp256k1
scalar strictly below the curve order, re-encode to the identical spelling,
and require its derived EIP-55 address to equal configured `POLY_ADDRESS`.
Unprefixed, uppercase, short/long, zero, out-of-range, or mismatched key text
is rejected before readiness. Drop the source text after constructing the
fixed 32-byte secret holder.

Every Goal G HTTP request adds
`Accept: application/json` and `Accept-Encoding: identity`. A JSON body
request (`POST /order`, `DELETE /order`, or Polygon JSON-RPC POST) additionally
adds `Content-Type: application/json`; a GET has no `Content-Type` and no
body. Every JSON response requires MIME essence `application/json`; the only
allowed parameter is case-insensitive `charset=utf-8`. Public requests carry
no `POLY_*` header. Protocol-managed `Host`, `Content-Length`, connection, and
TLS headers are never caller-set and are outside application byte-equality
tests. A non-identity `Content-Encoding` response, duplicate application
header, caller-supplied extra header, control byte, or leading/trailing
header-value whitespace fails closed. Credential values follow only the
explicit grammars above and are transported byte-exact. POST lowering must
serialize once, HMAC that final slice, and transport the same slice.

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
| `PM-LIVE-PUBLIC-METADATA` | Configured public CLOB metadata at `https://clob.polymarket.com` | `GET /clob-markets/{condition_id}` | `reap-polymarket-public-source` metadata worker | Public | Membership, lifecycle, tick, minimum, neg-risk domain, spender set |
| `PM-LIVE-PUBLIC-BOOK-SNAPSHOT` | Configured public CLOB book | `GET /book?token_id={token}` | PM public book worker | Public | Seed/resync and book-integrity fence |
| `PM-LIVE-PUBLIC-MARKET-WS` | Configured public CLOB market stream at `wss://ws-subscriptions-clob.polymarket.com` | `/ws/market`, configured `assets_ids` only | PM public socket/session worker | Public | Current book epoch, integrity, freshness |
| `PM-LIVE-PUBLIC-SERVER-TIME` | Public CLOB clock observation | `GET /time` | PM public clock worker | Reconciliation | L2/order clock offset and skew evidence only |
| `PM-LIVE-PUBLIC-GEOBLOCK` | Public geographic safety observation at `https://polymarket.com` | `GET /api/geoblock` | PM public safety worker | Critical | New placement fail-close input only |
| `PM-LIVE-USER-WS` | Authenticated credential-visible user stream at `wss://ws-subscriptions-clob.polymarket.com` | `/ws/user`; exactly one initial auth frame for the one configured condition per connection epoch | `reap-polymarket-live-adapter` private socket/session worker | Private | Order/fill occurrence and private epoch; never sufficient alone |
| `PM-LIVE-ACCOUNT-CUT` | Authenticated account read at `https://clob.polymarket.com` | `GET /balance-allowance` for `COLLATERAL` and configured `CONDITIONAL` token, `signature_type=0` | Authenticated account-read worker | Reconciliation | Collateral/token balance and per-selected-spender numeric cache/spendability evidence; never typed operator approval |
| `PM-LIVE-POLYGON-AUTHORIZATION-CUT` | Credential-free Polygon read; production origin deferred to Goal H | Closed chain-ID/finalized-anchor checks plus exact-block ERC-20 `allowance` and ERC-1155 `isApprovedForAll` calls | `reap-polymarket-chain-source` | Reconciliation | One fresh indivisible typed allowance/operator-approval cut |
| `PM-LIVE-POSITION-OBSERVATION` | Public address-scoped Data API at `https://data-api.polymarket.com` | `GET /positions` with exact user/market, `sizeThreshold=0`, bounded `limit`/`offset` | PM public position worker | Reconciliation | Monitored projection only; never atomic completeness or sell authority |
| `PM-LIVE-OPEN-ORDERS` | Authenticated credential-visible inventory | `GET /data/orders` with unfiltered credential scope and `next_cursor` | Authenticated reconciliation worker | Reconciliation | Complete credential-visible open-order cut |
| `PM-LIVE-ORDER-DETAIL` | Authenticated exact identity read | `GET /data/order/{orderID}` | Authenticated reconciliation worker | Reconciliation | Resolve expected, owned, ambiguous, or unmanaged identity; never creates ownership |
| `PM-LIVE-TRADES` | Authenticated credential-visible trades | `GET /data/trades` with unfiltered credential scope and `next_cursor` | Authenticated reconciliation worker | Reconciliation | Complete credential-visible maker/taker-leg fill cut |
| `PM-LIVE-PLACE-GTC-POST-ONLY` | One prepared fixed-profile mutation and its typed result | `POST /order` | Linear authenticated execution edge | Critical | One take-once prepared quote only |
| `PM-LIVE-PLACE-GTC-POST-ONLY-DISPATCH` | Durable request-commitment and dispatch-authorized barrier | Closed internal authenticated-journal channel; no network route | Authenticated journal writer/coordinator | Persistence | No place send until the exact commitment/barrier is durable |
| `PM-LIVE-CANCEL-OWNED` | Exact locally proven owned cancel and its typed result | `DELETE /order`, body `{"orderID":"…"}` | Linear authenticated execution edge | Critical | Cancel only one journal-proven venue identity |
| `PM-LIVE-CANCEL-OWNED-DISPATCH` | Durable cancel commitment and dispatch-authorized barrier | Closed internal authenticated-journal channel; no network route | Authenticated journal writer/coordinator | Persistence | No cancel send until the exact commitment/barrier is durable |
| `PM-LIVE-RECOVERY-CANCEL` | L2-only recovery cancel mutation/result; no place or EOA signer | Same exact-owned `DELETE /order` | Recovery adapter composition | Critical | Cancel a proven-owned identity only after the child reconciliation fact |
| `PM-LIVE-RECOVERY-CANCEL-RECONCILIATION` | Complete recovery inventory/detail/trade cut and exact-owned decision | Closed internal reconciliation delivery channel | Recovery coordinator | Reconciliation | Prove identity and continued liveness before reissuing the identical cancel |

The current authenticated order/trade documents prove only
credential-visible scope. A complete unfiltered page walk proves absence only
for that credential. It does not prove funder-wide absence across another API
key, the UI, another process, a manual actor, or on-chain activity.

## Closed Route, Query, And Result Contract

All query order shown below is canonical transport order. Dynamic values are
strictly validated typed configuration or page state, percent-encoded once,
and included in the durable request commitment. Authenticated HMAC input
contains the path but excludes the query. There is no caller-supplied filter,
route, method, body, origin, or JSON-RPC operation.

| Requirement | Exact operation/request | Pagination and visibility | Accepted result or fail-closed behavior |
| --- | --- | --- | --- |
| `OKX-LIVE-PUBLIC-INDEX-WS` | Connect `wss://ws.okx.com:8443/ws/v5/public`; send `{"id":"1","op":"subscribe","args":[{"channel":"index-tickers","instId":"<configured-inst-id>"}]}` | No pagination; exactly one public `instId` | Readiness needs the matching subscribe acknowledgement and a valid matching row. Wrong ID/channel/instrument, malformed price/time, error, idle, overflow, or reconnect invalidates the epoch. |
| `PM-LIVE-PUBLIC-METADATA` | `GET https://clob.polymarket.com/clob-markets/{condition_id}`; no query/body | One configured public condition | Exact `200` schema and configured membership/lifecycle/tick/minimum/neg-risk/exchange match only. Anything else makes market readiness false. |
| `PM-LIVE-PUBLIC-BOOK-SNAPSHOT` | `GET https://clob.polymarket.com/book?token_id=<configured-token-id>` | One configured public token | Exact complete bounded `200` snapshot. `404` means no usable book, never an empty book; mismatch, invalid numeric/book integrity, or malformed/oversized response invalidates readiness. |
| `PM-LIVE-PUBLIC-MARKET-WS` | Connect `wss://ws-subscriptions-clob.polymarket.com/ws/market`; initial frame `{"assets_ids":["<configured-token-id>"],"type":"market","initial_dump":true,"level":2,"custom_feature_enabled":false}` | Exact one configured token; no dynamic widening | No acknowledgement is specified. Socket-open is not ready. A matching valid snapshot/event plus REST seed/resync and live ping/pong establish the epoch; unknown/out-of-scope input ends it. |
| `PM-LIVE-PUBLIC-SERVER-TIME` | `GET https://clob.polymarket.com/time`; no query/body | Public single observation | Exact bounded `200` JSON integer Unix seconds, used only for offset/skew. Failure, redirect, stale/malformed value, or excessive skew makes clock readiness false. |
| `PM-LIVE-PUBLIC-GEOBLOCK` | `GET https://polymarket.com/api/geoblock`; no query/body | Scope is the requesting egress IP | Exact bounded `blocked/ip/country/region` object. `blocked=true`, failure, stale/malformed input, or redirect blocks new placement; cancellation remains independently gated. |
| `PM-LIVE-USER-WS` | Connect `wss://ws-subscriptions-clob.polymarket.com/ws/user`; send the credential frame once with exact configured `markets` | Authenticated API-key scope intersected with configured conditions; not account/funder-wide | No acknowledgement is specified. Auth/scope failure, unknown lifecycle/time, malformed/private overflow, idle, or reconnect ends the private epoch and forces one complete epoch-bound open-orders cut, exact detail for every implicated identity, and the complete trades cut. Never capture the auth frame. |
| `PM-LIVE-ACCOUNT-CUT` | In order: `GET /balance-allowance?asset_type=COLLATERAL&signature_type=0`; then `GET /balance-allowance?asset_type=CONDITIONAL&token_id=<configured-token-id>&signature_type=0` | API-key/type-0 configured-account scope; no pagination; bracketed but non-atomic | Each reply is exact bounded `{balance:<decimal-string>,allowances:<address,decimal-string>}`. Select only the configured exchange key. Missing/wrong/invalid spender data makes the cut unready; values remain numeric and never become approval. |
| `PM-LIVE-POLYGON-AUTHORIZATION-CUT` | Five single-object JSON-RPC POSTs with deterministic IDs `1..5`: chain ID; finalized block; exact-block pUSD allowance call; same-block CTF approval call; exact-number block recheck | One closed owner/exchange/contracts/block sequence; Goal G production origin absent | Require JSON-RPC 2.0/matching IDs, chain `0x89`, non-null fresh anchor, unchanged number/hash, exact 32-byte allowance, and canonical boolean 0/1. Any partial/error/revert/mismatch/unsupported/stale/malformed result discards the whole cut. |
| `PM-LIVE-POSITION-OBSERVATION` | `GET https://data-api.polymarket.com/positions?user=<configured-address>&market=<configured-condition-id>&sizeThreshold=0&limit=500&offset=<0,500,...>&sortBy=TOKENS&sortDirection=DESC` | Public address/condition scope; advance by 500, stop only when row count `<500`; offset at most 10,000; no atomic fence | Parse exact JSON numerics without `f64`. Scope conflict, duplicate/conflicting asset, malformed numeric, page/bound failure, or divergence discards the observation. Success remains monitored evidence, never balance/sell authority/completeness. |
| `PM-LIVE-OPEN-ORDERS` | Authenticated `GET /data/orders?next_cursor=<cursor>` with no `id`, `market`, or `asset_id` filter | API-key-visible only. Start `MA==`; require `{limit,count,next_cursor,data}` and `count == data.length`; stop only at `LTE=`; enforce all page/aggregate bounds | Cycle/malformed/empty-unexpected cursor, wrapper/count conflict, duplicate/conflicting row, partial page, or cap exhaustion discards the cut. Successful absence is credential-visible only. |
| `PM-LIVE-ORDER-DETAIL` | Authenticated `GET /data/order/{orderID}`; no query/body | One expected, locally owned, ambiguous, or detected-unmanaged ID | `200` must match exact ID/account/market/token and the REST-order union. `404` is one negative observation only and needs complete order/trade cuts; all other conflicts quarantine and halt placement. |
| `PM-LIVE-TRADES` | Authenticated `GET /data/trades?next_cursor=<cursor>` with no `id`, `maker_address`, `market`, `asset_id`, `before`, or `after` filter | API-key-visible only. Same `MA==`/wrapper/count/`LTE=` contract; maximum 8,192 retained fills | Incomplete/cyclic pages, duplicate/conflicting identity, unresolved maker/taker link, scope conflict, unknown lifecycle/time, or cap exhaustion discards the cut. Status alone never creates a fill amount. |
| `PM-LIVE-PLACE-GTC-POST-ONLY` | Authenticated `POST /order`; no query; exact once-serialized body contract below | One take-once, durably granted configured intent | Only `success:true`, `status:"live"`, and exact expected EIP-712 `orderID` is ordinary acceptance. `matched`, `delayed`, or `unmatched` is known but out of fixed profile; typed rejection is non-success; unknown/partial/lost response is acknowledgement-unknown. |
| `PM-LIVE-CANCEL-OWNED` | Authenticated `DELETE /order`; no query; exact body `{"orderID":"<journal-proven-order-id>"}` | One take-once exact locally owned ID | Success requires the exact ID in `canceled` and absent from `not_canceled`. The exact ID in `not_canceled`, both/neither, unrelated IDs, partial/lost result, or transport ambiguity forces reconciliation. |
| `PM-LIVE-RECOVERY-CANCEL` | The same exact DELETE/body, constructed by the L2-only recovery composition without an EOA signer | Exact identity first proven by complete open-orders/detail/trades reconciliation | Same strict result rules. Reissue only the identical proven-owned ID after a fresh complete cut still proves it live; otherwise retain a durable operator-required slot. |
| Three `*-DISPATCH`/`*-RECONCILIATION` child IDs | Internal typed commitment/barrier or complete-cut delivery only | No URL, socket, RPC, signer, or mutation authority | A child can authorize only its named parent operation; it cannot construct a generic request or widen scope. |

The exact once-serialized place body has this field order and representation:

```text
{"deferExec":false,"order":{"builder":"<bytes32>","expiration":"0",
"maker":"<EOA>","makerAmount":"<base-unit-string>","metadata":"<bytes32>",
"salt":<exact-u53-json-number>,"side":"BUY|SELL","signature":"<0x-signature>",
"signatureType":0,"signer":"<EOA>","takerAmount":"<base-unit-string>",
"timestamp":"<unix-ms-string>","tokenId":"<configured-token>"},
"orderType":"GTC","owner":"<L2-api-key-uuid>","postOnly":true}
```

Whitespace/newlines above are explanatory only; the transported vector is the
compact byte string frozen below. `salt` is a lossless JSON integer in
`1..=2^53-1`; every other integer-sized signed-order field shown as a string
is a canonical nonzero or allowed-zero base-10 string. The final body slice is
HMAC-signed and transported without another serialization.

The open-order/trade pagination conflict is resolved in favor of current V2
client consensus: start `MA==`, terminate at `LTE=`, and require the paginated
wrapper. An order/trade cursor is canonical padded RFC 4648 standard-base64 of
a canonical ASCII offset, not arbitrary text. Raw length is 4 through 28
bytes, is a multiple of four, and contains no whitespace/control. Decode
strictly, re-encode with the padded STANDARD engine, and require byte equality.
Decoded bytes must be exact `-1`, exact `0`, or a positive decimal matching
`[1-9][0-9]{0,19}` and fitting checked `u64`. Thus start is exactly `MA==`
(zero), only terminal is `LTE=` (negative one), and terminal is never sent.
After page one, a nonterminal offset must be strictly greater than the
previous requested offset. Cycle identity is the decoded `u64`; repeat,
rollback/nonincrease, another negative, empty, leading zero, alternate
alphabet/padding, or overflow discards the whole cut.

The query encoder preserves RFC 3986 unreserved bytes and emits every other
byte once as uppercase `%HH`; thus the first cursor is `MA%3D%3D`, `+` is
`%2B`, and `/` is `%2F`. It never accepts pre-escaped input, emits form-style
`+`, or double encodes. HMAC signs only `/data/orders` or `/data/trades`;
canonical query bytes are excluded from HMAC but enter the durable commitment
and exact transport target. Although the OpenAPI marks trade `maker_address`
required, current official clients and guide permit the unfiltered
credential-visible call needed for recovery. Goal G therefore sends no
filter; a venue `400` makes the cut incomplete and does not authorize a
filtered fallback.

All JSON decoders in this section reject duplicate object keys before typed
deserialization. Unless a route-specific table explicitly allows an optional
field, missing, null, wrong-kind, duplicate, or unknown fields are malformed.
Strings and arrays use their field-specific aggregate caps; an error string is
nonempty UTF-8, has no control character, and is at most 512 bytes. Hex
identities use their canonical lowercase wire form after case-insensitive
comparison with the already known expected identity.

### Closed mutation response union

For `POST /order`, an HTTP `200` body is one object with required
`success: bool`, `orderID: string`, and `status: string`. The only optional
keys are `errorMsg: string`, `makingAmount: string`, `takingAmount: string`,
`transactionsHashes: array<string>`, and `tradeIDs: array<string>`. The
amounts, when nonempty, use the field-local exact decimal grammar
`^(?:0|[1-9][0-9]{0,77})(?:\.[0-9]{1,6})?$`: no sign, exponent, or leading
integer zero, and at most six fractional digits. Spellings such as `100.0`
remain accepted and retain their raw provenance. Convert checked to a
six-decimal `U256` by right-padding the fraction. Outside the exact
accepted-live echo cases below, these are opaque bounded reconciliation
amounts and never infer a fill.
Hashes are canonical 32-byte hex values; IDs are bounded nonempty strings.
The cross-field union is:

| Exact `200` case | Classification |
| --- | --- |
| `success:true`, exact expected `orderID`, `status:"live"`, absent/empty `errorMsg`, and exactly one amount pair: both keys absent; both keys present with empty strings; both keys present as exact `"0"`; or both keys present and exactly equal to the committed maker/taker base-unit strings. Both arrays are absent or empty. | Ordinary accepted-live. The source-proven amount spellings are retained with provenance; none implies a fill. A mixed/partial/other pair is ambiguous. |
| `success:true`, exact expected `orderID`, `status:"matched"`, `"delayed"`, or `"unmatched"`, absent/empty `errorMsg`, and otherwise structurally valid bounded optional evidence | Known out-of-profile acknowledgement. Retain every returned ID/hash/amount only as reconciliation evidence, retain the reservation, halt placement, and reconcile. |
| `success:false`, empty `orderID` and `status`, nonempty bounded `errorMsg`, absent/empty amounts, and absent/empty arrays | Definite typed rejection. |
| `success:false`, empty `orderID`, `status:"unmatched"`, nonempty bounded `errorMsg`, absent/empty amounts, and absent/empty arrays | Known out-of-profile definite rejection; durably reduce it and reconcile before reusing the slot. |
| Any other combination, including identity mismatch, `success`/ID/status/error contradiction, live with any mixed/partial/other nonzero amount pair or any nonempty evidence array, unknown status, malformed optional evidence, or an unrelated identity | Protocol violation and acknowledgement unknown. |

For `DELETE /order`, an HTTP `200` body has exactly two required keys:
`canceled` is an array of order-ID strings and `not_canceled` is a map from
order-ID strings to nonempty bounded reason strings. The only success is
`canceled:[<exact-expected-id>]` and an empty map. An empty array plus exactly
the expected ID as the map's sole key is typed non-success and forces
reconciliation. Both collections containing the ID, neither containing it,
an unrelated ID, duplicate ID/key, more than one entry across the two
collections, a null/defaulted collection, or any other shape is a protocol
violation and acknowledgement unknown.

An error response is exactly an object with required nonempty `error` and only
optional bounded `code: string` and `retry_after_seconds: integer`. For either
mutation, a well-formed error object at `400` is a definite rejection, except
that POST error text exactly matching
`order <expected-order-id> is invalid. Duplicated.` after the bounded
case-insensitive expected-ID substitution is a duplicate acknowledgement and
forces exact identity reconciliation. `401` invalidates the auth epoch.
`425`, `429`, and `503` with a well-formed error object are typed transient
non-success and force durable reduction/reconciliation; they never trigger a
client retry. Only POST `500` with exact error `order timed out`, no `code`,
and no `retry_after_seconds` is definitely rejected. Every other mutation
`5xx`, every unlisted `3xx`/`4xx`, an unexpected `2xx`, wrong content type,
malformed/oversized body, duplicate JSON key, redirect, timeout, partial
response, or response loss after a possible write is protocol-failure
acknowledgement-unknown. A cancel `5xx` has no definite-rejection exception.

Route-specific status handling is mandatory. `401` invalidates the auth
epoch. Exact-order `404` is one negative observation; book `404` is book
unavailability; `404` elsewhere is a protocol/configuration failure. Read
`408`, `425`, `429`, and `500..=504`, or a read-only connect/TLS/write/first-
byte timeout keep the cut incomplete and may schedule only a fresh bounded
coordinator attempt. A read `400`, `401`, route-specific `404`, malformed
success, wrong origin, redirect, TLS/proxy-policy failure, or other status is
not transient. The route-specific mutation table above is exhaustive.

Every Goal G `reqwest 0.12.28` builder explicitly calls
`retry(reqwest::retry::never())`,
`redirect(reqwest::redirect::Policy::none())`, and `no_proxy()`. The locked
client configures a default protocol-NACK policy when a relevant protocol
feature is enabled and otherwise follows redirects/ambient proxies by
default; the current workspace happens not to enable HTTP/2 or HTTP/3, but
least authority cannot rely on that. HTTP-layer retry is zero for every
request. A read refresh cycle permits at most three separately constructed
coordinator attempts in 30 monotonic seconds, with delays of at least one then
two seconds and all route pacing still applied. Each attempt receives a fresh
L2 timestamp where required and discards every partial prior cut. A canonical
integer `Retry-After`/`retry_after_seconds` in `1..=30` increases the next
delay; an HTTP-date, zero, noncanonical, conflicting, or greater value ends
the cycle. Exhaustion emits one typed `ReadRefreshExhausted` fact, leaves the
cut unready, and permits no new cycle until a 30-second cooldown or an
explicit configuration/transport/auth epoch change.

One durable mutation grant authorizes at most one application send. A place
may receive a later fresh grant only after the prior attempt is durably proven
definitely not dispatched; once any application byte may have been written,
the place body is never replayed. Recovery may later issue the identical
canonical cancel body only after a fresh complete order/detail/trade cut still
proves the order owned and live, and only under a separately committed
request, fresh L2 HMAC/timestamp, and new take-once grant. That is a new
journaled recovery operation, not an HTTP retry or reuse of a prior grant.

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

### Lifecycle compatibility table

| Source and field | Exact accepted raw values | Normalized meaning and fixed-profile treatment |
| --- | --- | --- |
| `POST /order` result `status` | Required string `live`, `matched`, or `delayed`; current guide compatibility also admits `unmatched`; empty string only accompanies `success=false` and is absence of lifecycle | `live` is the only ordinary fixed-profile acceptance. The other three are known but out-of-profile, retain authority/reservations, halt placement, and reconcile. Empty never becomes pending/open. |
| REST open-order/detail `status` | Required string: prefixed `ORDER_STATUS_LIVE`, `ORDER_STATUS_INVALID`, `ORDER_STATUS_CANCELED_MARKET_RESOLVED`, `ORDER_STATUS_CANCELED`, or `ORDER_STATUS_MATCHED`; exact fresh guide alias `LIVE` only | Apply the closed status/quantity cross-product below. `ORDER_STATUS_CANCELED_MARKET_RESOLVED` is a cancellation, not a match or rejection. The `LIVE` alias preserves raw provenance. Missing/null/wrong-kind and other unprefixed REST order values quarantine until a future reviewed amendment proves them. |
| Raw user-WS order occurrence `type` | Required `PLACEMENT`, `UPDATE`, or `CANCELLATION` | Occurrence reason only: accepted creation, some/all quantity matched, or remaining quantity canceled. It is never an order status or POST acknowledgement. |
| Raw user-WS order `status` | Optional string `LIVE`, `MATCHED`, `DELAYED`, `UNMATCHED`, or `CANCELED`; omission contributes no corroborating raw-status assertion | A tagged WS order state. Occurrence plus quantities still assert the normalized state in the closed cross-product below. `DELAYED`/`UNMATCHED` on a locally owned fixed-profile order are known protocol violations. Present null/wrong kind and all unknowns quarantine. |
| REST trade `status` | Required string: `TRADE_STATUS_MATCHED` or `MATCHED`; `TRADE_STATUS_MATCHED_NOT_BROADCASTED` or `MATCHED_NOT_BROADCASTED`; `TRADE_STATUS_MINED` or `MINED`; `TRADE_STATUS_CONFIRMED` or `CONFIRMED`; `TRADE_STATUS_RETRYING` or `RETRYING`; `TRADE_STATUS_FAILED` or `FAILED` | Settlement namespace only. Each separately tagged pair maps one-for-one while retaining provenance. `MatchedNotBroadcast` is distinct and nonterminal: a match exists, but no on-chain transaction is yet broadcast. Missing/null/wrong-kind status quarantines. |
| Raw user-WS trade `status` | Required string: `TRADE_STATUS_MATCHED` or `MATCHED`; `TRADE_STATUS_MINED` or `MINED`; `TRADE_STATUS_CONFIRMED` or `CONFIRMED`; `TRADE_STATUS_RETRYING` or `RETRYING`; `TRADE_STATUS_FAILED` or `FAILED` | Same five settlement meanings with spelling provenance retained. Missing/null/wrong-kind status quarantines. Either `TRADE_STATUS_MATCHED_NOT_BROADCASTED` or `MATCHED_NOT_BROADCASTED` is a route-family violation under the current official account-listing-only contract; quarantine, invalidate the epoch, and reconcile. |

REST and raw user-WS `original_size`/`size_matched` are strings using the same
exact decimal grammar and checked six-decimal-`U256` conversion frozen for
POST response amounts above. Compare only the converted protocol-unit
integers: require `original_size > 0` and
`0 <= size_matched <= original_size`. REST order state is exactly:

| REST status and quantity | Canonical `PmOrderStatus` |
| --- | --- |
| `ORDER_STATUS_LIVE` or `LIVE`, `size_matched == 0` | Open |
| `ORDER_STATUS_LIVE` or `LIVE`, `0 < size_matched < original_size` | Partially filled |
| `ORDER_STATUS_MATCHED`, `size_matched == original_size` | Filled |
| `ORDER_STATUS_INVALID`, `size_matched == 0` | Rejected |
| `ORDER_STATUS_CANCELED` or `ORDER_STATUS_CANCELED_MARKET_RESOLVED`, any `size_matched <= original_size` | Cancelled, retaining cumulative fill |

Every other REST status/quantity cross-product quarantines, retains the
reservation, halts placement, and requires one complete epoch-bound
open-orders cut plus exact order detail for each implicated identity and the
complete trades cut.

Raw user-WS order occurrence and optional status are likewise not an
unrestricted cross-product. After the same quantity conversion, apply:

| Occurrence/quantity | Only allowed optional status | Normalized occurrence |
| --- | --- | --- |
| `PLACEMENT`, `size_matched == 0` | omitted or `LIVE` | Open |
| `UPDATE`, `0 < size_matched < original_size` | omitted or `LIVE` | Partially filled |
| `UPDATE`, `size_matched == original_size` | omitted or `MATCHED` | Filled |
| `CANCELLATION`, any valid cumulative `size_matched` | omitted or `CANCELED` | Cancelled, retaining cumulative fill |

The occurrence is authority and status is corroboration; omission stays
recorded as omission and never invents a raw token. Every other cross-product
is a private protocol conflict that quarantines, invalidates the WS epoch,
halts placement, and requires one complete epoch-bound open-orders cut, exact
detail for every implicated identity, and the complete trades cut. `DELAYED`
and `UNMATCHED` are known fixed-profile violations regardless of occurrence;
present-null, wrong-kind, or unknown status also conflicts. This live-only
normalizer does not widen or rewrite Goal F fixture bytes or fake behavior.

The current SDK `topic/type/payload` camel/snake-case envelopes and
RFC-3339/numeric projections are differential semantic oracles. Reap connects
directly to `/ws/user`, so those normalized SDK outputs are not accepted as
wire envelopes. Current official sources conflict on whether a future
SDK-normalized user event might expose not-broadcast; the more precise current
account model says it is account-listing-only. The narrower REST-only rule
therefore fails closed without a real credential probe.

The canonical settlement graph distinguishes:

```text
MatchedNotBroadcast -> Matched
Matched -> Mined -> Confirmed
MatchedNotBroadcast | Matched | Mined -> Retrying
Retrying -> Mined | Failed
```

Those are ordinary event transitions; duplicate same-state observations are
idempotent. This preserves Goal F's exact five-state
`Matched | Mined -> Retrying` behavior. `Confirmed` and `Failed` are terminal.
A complete authoritative
reconciliation cut may cover a skipped intermediate only along the same
forward reachability relation, while retaining source and cut provenance.
Any regression, terminal escape, or other event jump conflicts. The new state
applies the exact provisional fill exposure just as a proven match does, but
does not invent transaction/finality evidence or release an open remainder.
It is representable only in the authenticated live journal family. Goal F's
five-value `PmJournalFillSettlementV1` and its bytes stay frozen; conversion
into that fake family is checked and rejects `MatchedNotBroadcast`.

### Field-local timestamp compatibility table

The exact whole-token forms are: `N10`, an unquoted JSON number token matching
`[1-9][0-9]{9}`; `S10`, a JSON string whose contents match
`[1-9][0-9]{9}`; `S13`, a string matching `[1-9][0-9]{12}`; `S19`, a
string matching `[1-9][0-9]{18}`; and `S0`, the exact string `"0"` only
where the table names it. Parse the raw JSON token/string before a library can
normalize it. Leading zero, whitespace, sign, fraction, exponent, Unicode
digit, alternate JSON kind, and overflow are rejected. These are field-local
forms, not a magnitude parser. Seconds use checked multiplication by 1,000,
milliseconds are unchanged, and nanoseconds retain their exact remainder
while their millisecond projection uses checked division by 1,000,000.

| Source/field | Required | Exact form and unit | Meaning |
| --- | ---: | --- | --- |
| POST result | n/a | No venue timestamp | Never import request auth/order time into the response fact. |
| REST order `created_at` | Yes | `N10` Unix seconds | Order creation time. A 13-digit value conflicts with the current raw docs/Rust source and quarantines. |
| REST order `expiration` | Yes | `S0` or `S10` Unix seconds | Zero is GTC/no-expiry. A locally owned Goal G order must be zero; nonzero is out-of-profile. |
| REST trade `match_time` | Yes | `S10` Unix seconds | Match instant at seconds precision. |
| REST trade `match_time_nano` | No | `S19` Unix nanoseconds | Same match instant; when present require `floor(ns/1e9) == match_time`, retain the remainder. |
| REST trade `last_update` | Yes | `S10` Unix seconds | Venue settlement-record update, not fill time. |
| Raw user-WS order `created_at` | No | `S10` Unix seconds | Order creation time. |
| Raw user-WS order `expiration` | No | `S0` or `S10` Unix seconds | Same sentinel/expiry semantics; locally owned fixed profile requires zero. |
| Raw user-WS order/trade `timestamp` | Yes | Separately tagged `S10` seconds or `S13` milliseconds | Event emission/update time; the union resolves current example/prose conflict without guessing. |
| Raw user-WS trade `match_time` / `last_update` | No | `S10` Unix seconds | Match and settlement-record update time, respectively. |
| PM market WS `book`, `price_change`, `last_trade_price`, `tick_size_change` `timestamp` | Yes | `S13` Unix milliseconds | Tagged public event time. Custom-only event families remain disabled and out of profile. |
| PM REST book `timestamp` | Yes | `S10` documented seconds-shaped value | Tagged REST-book snapshot time; never ordered against WS solely after conversion. |
| OKX index ticker `data[*].ts` | Yes | `S13` Unix milliseconds | Configured index update time; subscription acknowledgement has no price freshness. |
| Public PM `/time` response | Yes | `N10` Unix seconds | Server offset/skew anchor only. |

The time bounds are exact:

- PM user-WS `timestamp`, PM market-WS `timestamp`, and PM REST-book
  `timestamp` are current observations. At receipt they may be at most
  30 seconds behind and five seconds ahead of a fresh PM `/time`-adjusted
  clock.
- OKX index `ts` is also current: it may be at most 30 seconds behind and five
  seconds ahead of the injected local wall clock. Goal H must qualify that
  clock before target-host use; `local-evidence` uses a deterministic
  synthetic clock.
- A PM `/time` result is usable only when it differs from the injected local
  wall clock by at most five seconds; its derived offset expires after
  30 monotonic seconds.
- REST order `created_at`, REST trade `match_time`/`match_time_nano`/
  `last_update`, and the optional corresponding raw user-WS record fields are
  historical identity/lifecycle evidence, not freshness gates. Their
  10/19-digit lexical forms are their lower/range bound; they have no
  additional past-age rejection, but any non-expiration value more than five
  seconds ahead of the fresh PM `/time`-adjusted clock quarantines. If that
  offset is unavailable or expired, the field cannot pass time validation:
  the containing private cut/event is incomplete or quarantined and placement
  remains unready.
- `expiration` is never a freshness observation. Exact zero is required for a
  locally owned fixed-profile order; a nonzero canonical value is retained as
  out-of-profile evidence and quarantines that identity.

All conversions and time-window arithmetic are checked integers. Every value
additionally passes its field-specific current/history rule above.
Missing-required, present-null optional, wrong JSON kind, noncanonical length,
overflow, inconsistent nano/seconds pair, future/history violation, or a
form borrowed from another family quarantines. Retain only bounded parsed
value/unit/source/identity evidence, never the raw private frame. Private
quarantine clears only after the complete epoch-bound open-order, exact-detail
when implicated, and trade cut. Public PM clears only after valid REST
book/resync plus a new compatible WS epoch; OKX clears after reconnect,
matching subscribe acknowledgement, and fresh ticker. Permanent ambiguity or
capacity exhaustion is an operator halt.

### Outbound PM clock contract

The venue sources prove milliseconds for signed-order time, seconds for L2
auth, and integer flooring; the windows below are explicitly Reap-local
fail-closed policy, not a claim about undocumented venue tolerance.
A usable `/time` anchor is `(server_s, received_wall_ns, received_mono_ns,
clock_epoch)`.
`server_s` first passes `N10`; require
`abs_diff(server_s, floor(received_wall_ns / 1_000_000_000)) <= 5`. At
monotonic instant `m`, require `m >= received_mono_ns` and
`m - received_mono_ns <= 30_000_000_000`, then define exactly:

```text
pm_now_ms(m) =
  checked_add(
    checked_mul(server_s, 1000),
    floor((m - received_mono_ns) / 1_000_000))
```

This is a conservative lower-bound projection. It never rounds to nearest,
uses a caller timestamp, or relabels local receipt time as venue time.

For one new order, take one `m`, require `pm_now_ms(m)` to match `S13`, and
persist that exact millisecond string with the canonical intent/body. There is
no durable timestamp high-water or artificial clamp: current pinned venue
sources prove none. Within one in-memory `clock_epoch`, evaluate a candidate
anchor at its receipt monotonic instant `m_new`. If a prior anchor is live,
compute `old_s = floor(pm_now_ms_old(m_new) / 1000)` and reject the candidate
exactly when `candidate.server_s < old_s`; equality is allowed because
`/time` is integral seconds. Rejection, overflow, or a noncanonical candidate
discards both anchors and makes clock readiness false. Process restart or a
transport/configuration clock-epoch change discards the old anchor and
comparison state before accepting a fresh normally validated anchor; no
comparison state is durable across that boundary. The exact persisted order
timestamp is never regenerated. Immediately before the first application
write, using a still-valid anchor, require checked
`pm_now_ms(now) - 30_000 <= order_ms <= pm_now_ms(now) + 5_000` and the
250-millisecond grant. Failure returns definitely-not-dispatched without
writing.

For every authenticated read, place, or cancel, after the final route/query
and body bytes exist, take a fresh `m` and set
`auth_s = floor(pm_now_ms(m) / 1000)`. It must match `S10`; the identical ten
ASCII digits enter the HMAC and `POLY_TIMESTAMP`. Immediately before the
first application write, the anchor must remain valid and
`auth_s == floor(pm_now_ms(now) / 1000)`. For place/cancel, a seconds-boundary
crossing, expiration, rollback, or failed arithmetic returns
definitely-not-dispatched. The edge does not regenerate a header under that
mutation grant; a fresh L2 timestamp requires a separately committed mutation
commitment/grant. An authenticated read has no durable dispatch grant: the
same failure discards that unsent read attempt, and the capped
coordinator-owned read cycle may construct one fresh route/query, timestamp,
and HMAC attempt under its ordinary attempt limit. It never reuses bytes or
retries the failed attempt. An unavailable clock prevents construction of
every authenticated request. It does not erase a durable cancellation or
reconciliation obligation, and public `/time` refresh remains available.

The geoblock object has no venue timestamp, so its freshness is exclusively a
Reap-local conservative policy, not a venue guarantee. An exact successful
four-key object is required with `blocked: bool`; canonical numeric
IPv4/IPv6 `ip` whose standard-library parse/re-render is byte-identical;
`country` matching `[A-Z]{2}`; and `region` matching
`[A-Z0-9-]{0,16}`. An exact `blocked:false` response creates a permit bound to
parsed `ip`, local
monotonic receipt, public-safety transport epoch, and configuration epoch. Its
half-open validity is exactly five monotonic seconds and cannot be configured
or extended by use. Redirect, failure, malformed/oversized object,
`blocked:true`, a different observed IP, or epoch/configuration change
invalidates it immediately.

Placement readiness, preparation, and every place grant require one live
permit. The grant binds its identity/epochs and has a send-before no later
than `permit.received_at + 5s` or any earlier auth/grant deadline. The edge
rechecks immediately before its first application write. Expiry or change
before that write sends zero bytes, regenerates no header under the grant, and
returns a durably reduced definitely-not-dispatched result. Invalidation
revokes every unconsumed place grant and suppresses new signing/preparation.
After any application byte may have been written, a later change cannot
retroactively prove non-dispatch; ordinary acknowledgement-unknown and
reconciliation rules apply. Geoblock never gates exact-owned cancellation,
recovery cleanup, reconciliation, or risk-reducing shutdown.

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

`PmPolygonAuthorizationSource` owns one compact UTF-8 serializer. Every
request is one individual body in the member order below with no whitespace,
BOM, or trailing newline and no second serialization. IDs restart at one for
each whole cut. Let `word(A)` mean 24 lowercase zero hex digits followed by
the address's 40 lowercase hex digits, one ABI word. Let `O` be the configured
EOA digits, `X` the selected exchange digits, and `B` the canonical block
quantity from request two (`0x0|0x[1-9a-f][0-9a-f]*`). Requests three through
five reuse the exact `B` bytes:

```text
{"jsonrpc":"2.0","method":"eth_chainId","params":[],"id":1}
{"jsonrpc":"2.0","method":"eth_getBlockByNumber","params":["finalized",false],"id":2}
{"jsonrpc":"2.0","method":"eth_call","params":[{"to":"0xc011a7e12a19f7b1f670d46f03b03f3342e82dfb","data":"0xdd62ed3e<word(O)><word(X)>"},"<B>"],"id":3}
{"jsonrpc":"2.0","method":"eth_call","params":[{"to":"0x4d97dcd97ec945f40cf65f87097ac5ea0476045","data":"0xe985e9c5<word(O)><word(X)>"},"<B>"],"id":4}
{"jsonrpc":"2.0","method":"eth_getBlockByNumber","params":["<B>",false],"id":5}
```

Placeholders are replaced before the one serialization and are not wire
bytes. Each calldata is exactly 68 bytes (`0x` plus 136 lowercase hex
digits). No `from`, gas, state override, caller-provided address/data/tag, or
extra call member exists.

Responses are semantically parsed, never JSON-member-order compared. Each is
one bounded object with no duplicate key or trailing non-whitespace, exact
`jsonrpc:"2.0"`, exact outstanding integer `id`, exactly one non-null
`result`, no `error`, and no unknown top-level member. Bounded extra members
inside block results are permitted and ignored. Results are respectively:

1. exact string `0x89`;
2. a non-null block with required canonical lowercase
   `number:B`, nonzero `hash:"0x" + 64 hex digits`, and canonical quantity
   `timestamp:T`;
3. exact lowercase `"0x" + 64 hex digits`, parsed as unscaled `U256`;
4. exact lowercase JSON string `"0x" + 64 zero hex digits` or
   `"0x" + 63 zero hex digits + "1"`; and
5. a non-null block whose required number/hash/timestamp equal `B/H/T`.

Any HTTP non-`200`, wrong MIME essence, JSON-RPC error, null, wrong
ID/version/kind/length/case, malformed/oversized body, noncanonical
quantity/data, empty/reverted call result, changed block, timeout,
redirect/proxy violation, stale/future cut, or partial sequence discards the
entire cut. A classified fresh whole-cut attempt starts again at request one;
it never retries one member or reuses `B`.

Each `eth_call` result is exactly one 32-byte ABI word. ERC-20 allowance is
unscaled `U256`; ERC-1155 approval accepts only canonical zero or one. The
whole fact binds chain, finalized block number/hash/timestamp, local monotonic
observation time, transport/account/configuration epochs, owner, exchange,
contracts, and both typed results.

The finalized timestamp may be at most five seconds in the future and thirty
seconds old; the completed cut expires after five monotonic seconds or any
transport/account/configuration/metadata epoch change. Wrong chain, null or
changed block, revert/error/ID mismatch, malformed/noncanonical/oversized
response, timeout, stale/future evidence, unsupported finalized or historical
call, redirect/proxy, or any partial sequence discards the cut and makes
readiness false. Any later eligible attempt is a fresh whole cut.

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
{"auth":{"apiKey":"…","secret":"…","passphrase":"…"},"type":"user","markets":["<configured-condition-id>"]}
```

Goal G's fixed profile has exactly one token and one condition. The token is a
canonical nonzero decimal `U256`; the condition is canonical lowercase `0x`
plus 64 hex digits. Duplicate or additional configured values are invalid,
not silently removed. The market initial frame is exact compact UTF-8:

```json
{"assets_ids":["<configured-token-id>"],"type":"market","initial_dump":true,"level":2,"custom_feature_enabled":false}
```

It has no whitespace/trailing newline. Goal G sends no dynamic public-market
or user subscribe/unsubscribe/update frame. Reconnect or configuration
lifecycle change closes the old session, starts a new epoch, and repeats the
one applicable initial frame. The user frame always sends the one configured
condition even though the venue field is optional; omission would widen
visibility. Wire events may be one object or an array. Raw event
families use `event_type = order|trade`; order `type` is
`PLACEMENT|UPDATE|CANCELLATION`; trade `type` is `TRADE`. Raw trade lifecycle
accepts the separately tagged prefixed/unprefixed ordinary five states in the
table above. `MatchedNotBroadcast` is account-trade-REST-only under the
current contract. Text `PING` is sent every ten seconds and expects `PONG`.
No official subscription acknowledgement is specified, so transport-open is
not account readiness. Full REST reconciliation is still required. The
credential-bearing initial frame is never captured.

The OKX source performs exactly one client control operation per connection
epoch. Its message ID is literal `"1"`, a connection-local correlation value,
and the compact frame is exactly:

```json
{"id":"1","op":"subscribe","args":[{"channel":"index-tickers","instId":"<configured-inst-id>"}]}
```

The argument array has one element and the validated configured `instId`
spelling is transported unchanged. A bounded semantic acknowledgement must
contain exact string `id:"1"`, `event:"subscribe"`, and matching
`arg.channel`/`arg.instId`; server member order and `connId` are not compared.
Wrong/type-confused ID, error code/event, wrong scope, or a second client
control operation ends the epoch. Ping/pong has no ID and is not a
subscription operation.

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
  -> reap-pm-authenticated-mutation-journal
                                    -> reap-durable-writer/reap-pm-core
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
- `reap-pm-authenticated-mutation-journal`: non-secret authenticated schema
  V1, lease, barriers, recovery projection, and hashes only; no network,
  credential, signer, request construction, or Goal F journal reuse.

`reap-pm-live-contracts` owns the stable requirement/route constants but edge
crates do not depend on it. Each edge emits only its closed, untagged typed
fact or result. At the composition boundary, `reap-pm-live` attaches the one
stable requirement ID and its canonical lane from the matrix before enqueue.
An edge therefore cannot select, alias, or multiply requirement identity, and
the constants crate does not create a dependency cycle.

The authenticated adapter never depends on `reap-pm-live`, receives an upper
prepared-effect/grant type, exposes an arbitrary request, or owns canonical
order/position state. No PM auth or Polygon chain role enters Chaos
`reap-live`, `reap-order`, `reap-venue`, or `reap-cli`.

### Exact dependency and feature freeze

Only two external packages are new to the workspace:

| Package | Exact selected line | Features | Purpose and exclusion |
| --- | --- | --- | --- |
| `k256` | exact `=0.13.4`; crate checksum `f6e3919bbaa2945715f0bb6d3934a173d1e9a59ac23767fbaaef277265a7411b` | `default-features = false`, `ecdsa` | Narrow secp256k1 deterministic ECDSA/recovery for type-0 EOA orders. No PKCS8/PEM, Schnorr, ECDH, serde, getrandom, precomputed tables, wallet, provider, or Ethereum client. This compatible line shares the workspace's existing digest/SHA-2 generation instead of introducing the newer duplicate digest family. |
| `sha3` | exact `=0.10.9`; crate checksum `77fd7028345d415a4034cf8777cd4f8ab1851274233b45f84e3d955502d93874` | `default-features = false` | `sha3::Keccak256` for address, EIP-712 domain/struct/digest, and order identity only; never NIST `Sha3_256`. No asm/oid/generic Ethereum/ABI/provider package. |

The exact locked existing packages reused at the Phase 0 cutoff are
`base64 0.22.1`, `hmac 0.12.1`, `sha2 0.10.9`, `serde 1.0.228`,
`serde_json 1.0.150`, `zeroize 1.9.0`, `reqwest 0.12.28`,
`tokio 1.52.3`, `tokio-tungstenite 0.27.0`, `bytes 1.12.1`,
`futures-util 0.3.32`, `thiserror 2.0.18`, and `url 2.5.8`.
No `alloy`, `ethers`, generic ABI/RPC, wallet, retry, UUID, or broad
Polymarket SDK dependency is added. A bounded exact UUID parser and hex/ABI
codec are owned locally by their narrow edge.

| Crate | Allowed normal dependency additions |
| --- | --- |
| `reap-polymarket-auth` | `reap-pm-core`; base64, hmac, k256, sha2, sha3, serde, serde_json, thiserror, zeroize. No async/network/runtime dependency. |
| `reap-polymarket-public-source` | `reap-polymarket-wire`, `reap-pm-core`, `reap-transport`; bytes, futures-util, reqwest, serde, serde_json, thiserror, tokio, tokio-tungstenite, url. No auth/live-adapter/chain dependency. |
| `reap-polymarket-chain-source` | `reap-pm-core`, `reap-transport`; bytes, reqwest, serde, serde_json, thiserror, tokio, url. No auth, signer, generic ABI/RPC, or canonical-state dependency. |
| `reap-polymarket-live-adapter` | `reap-polymarket-auth`, `reap-polymarket-wire`, `reap-pm-core`, `reap-transport`; bytes, futures-util, reqwest, serde, serde_json, thiserror, tokio, tokio-tungstenite, url, zeroize. No public-source, chain-source, strategy, journal, or coordinator dependency. |
| `reap-pm-authenticated-mutation-journal` | `reap-durable-writer`, `reap-pm-core`; serde, serde_json, sha2, thiserror. No auth/network/signer dependency. |
| `reap-pm-live` | `reap-polymarket-public-source`, `reap-polymarket-chain-source`, `reap-polymarket-live-adapter`, and `reap-pm-authenticated-mutation-journal`, plus its existing Goal F dependencies. Auth remains behind the live-adapter edge. It remains the only joining/composition owner. |
| `reap-okx-public-source` | Its existing pure/transport edges plus only the existing WebSocket/runtime packages needed for the closed index source. It never imports `reap-okx-live-adapter`. |

Each network edge defines `local-evidence = []`; `reap-pm-live` only forwards
that feature explicitly to the three PM edge sources and OKX public source.
No default feature, default member, deployable binary, service, or normal
dependency enables it. Every external integration test/bench using loopback
declares `required-features = ["local-evidence"]`. The authenticated journal
and auth crate have no origin feature.

This graph is acyclic by construction: pure core/wire/transport and durable
writer are leaves; auth and the journal do not depend on an edge or
coordinator; edge crates do not depend on one another or on `reap-pm-live`;
only `reap-pm-live` joins them. Phase 2's lockfile review must confirm the two
new packages' complete transitive graph and `cargo audit` result before their
commit.

The signer calls `sign_prehash_recoverable` on the already computed
EIP-712/Keccak digest; it never invokes an ordinary `Signer::sign` path that
would hash again with SHA-256. Wire output is canonical low-s `r || s || v`,
with `v = 27 + recovery_id` and only recovery IDs 0/1 accepted. Phase 2
negative vectors must reject high-s/noncanonical, bad recovery, wrong domain,
wrong side/amount orientation, and any field/body mismatch.

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
Authenticated execution requires the distinct
`reap-pm-authenticated-mutation-journal` version-1 family, bound to the public EOA
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
auth timestamp, expected order ID, geoblock permit identity/epochs for place,
and the earliest monotonic send-before deadline.
Recovery treats any consumed or durably granted barrier without a conclusive
post-result as acknowledgement unknown. It reconciles exact expected order,
all credential-visible open orders, and trades. It either converges to one
known state or durably retains the exact slot/identity as operator-required.
Cancellation may be repeated only for the identical proven-owned order after
read-only reconciliation still proves it live and the frozen protocol permits
idempotent exact cancellation, under the separately journaled recovery
operation and fresh grant described above.

## Lane And Bounded-Resource Plan

Existing Goal F service priority remains
`Critical > Persistence > Private > Scheduled > Public > Reconciliation >
Telemetry`. New facts receive deterministic subranks within those lanes;
equal-rank facts use the existing canonical identity/ingress ordering.

| Canonical lane | Capacity | Nominal high water | Max age | Goal G contents | Saturation |
| --- | ---: | ---: | ---: | --- | --- |
| Critical | 512 | 32 | 250 ms | auth/safety faults, mutation result, acknowledgement unknown | Global/account stop; never drop |
| Persistence | 512 | 32 | 250 ms | request-preparation and dispatch durability acknowledgements | Global stop; no dispatch |
| Private | 4,096 | 64 | 250 ms | user order/fill occurrences | End epoch, halt account, reconcile |
| Scheduled | 4,096 | 64 | 100 ms | quote/cancel evaluation | Suppress quote and cancel owned |
| Public | 8,192 | 256 | 500 ms | PM and OKX public observations | Invalidate stream and resync |
| Reconciliation | 128 | 16 | 5 s | complete order/trade/CLOB-account/finalized-chain/position cuts | Remain unready; retry boundedly |
| Telemetry | 128 | 32 | none | non-authoritative metrics | Coalesce/sample only |

The following are bounded auxiliary queues, not canonical lanes and never a
second lane assignment for a stable requirement ID:

| Auxiliary queue | Capacity | Nominal high water | Max age | Goal G contents | Saturation |
| --- | ---: | ---: | ---: | --- | --- |
| Reconciliation request | 128 | 16 | 1 s | exact refresh requests | Retain pending refresh |
| Capture | 8,192 | 256 | 500 ms | credential-free raw public frames only | Invalidate capture and resync |
| Journal | 1,024 | 128 | 1 s | non-secret durable mutations | Halt quote/dispatch |
| Prepared effect | 256 | 32 | 250 ms | move-only quote/cancel authority | Reject/halt; never lose approved effect |

Additional hard bounds for the resumed Phase 0 are one MiB per raw
frame/HTTP response, 64 events per parsed WS frame, 32 MiB aggregate raw
bytes, 64 KiB mutation request body, 8 KiB per header/field aggregate,
500 rows per venue page, 64 pages per request cut, 1,024 live/unresolved
orders, and 8,192 retained fills. Aggregate bounds win over frame/page maxima.

The target-host-neutral deadline plan is five seconds each for connect/TLS,
write, and first byte; ten seconds total REST; ten-second WS ping, five-second
pong, and thirty-second idle/reconnect fault; five-second maximum server skew
with a thirty-second offset TTL. A dispatch grant expires after 250 ms before
the first application write. Reads may use only the bounded coordinator-owned
fresh-attempt cycle above; every HTTP client has zero retries. Placement is
never reissued after bytes may have reached the venue.

Pacing must remain below official ceilings: at most two public REST requests
per second, five credentialed reads per second, five exact mutations per
second, and one reconnect attempt per five seconds, all with a burst no
greater than the same one-second allowance and an explicit one-second queue
age. These are conservative library bounds, not target-host performance
claims. They were validated against the 128-row Amendment 2 cutoff. The active
Phase 0 may re-attest but cannot widen that cutoff; source drift or a need to
change these bounds requires a reviewed amendment.

The chain source permits one in-flight cut and at most one fresh whole-cut
attempt per second. It does not retry an individual call or reuse a partial
result; a classified retry schedules a new whole sequence. These local bounds
are independent of any future provider ceiling.

## Local Performance Contract

The legacy PM action `25,000 ns` p50 and `250,000 ns` p99.9 absolute exits are
superseded. The completed policy tranche at
`facd3a616fc20e7bc1abc627235588b7532ff8b1` changed only the latency branch at
`crates/reap-pm-live/src/evidence/runner.rs:81`,
`crates/reap-pm-live/benches/pm_action_path.rs`, and their policy tests. It
removed those two exits, preserved the `15,000`-sample and every
logical/hash/allocation/memory/cardinality/queue gate, left the workload,
timed boundary, and report schema unchanged, and emits the complete report.
Goal G verifies but does not edit that policy again.

From the clean Amendment 2 pre-gate commit, run these sixteen invocations
serially and verbatim:

```bash
h=target/tmp/goal-g-phase0-amended/run-benchmark-invocation.sh
b=target/tmp/goal-g-phase0-amended/baseline
(
set -euo pipefail
"$h" "$b" engine warmup 1 -- cargo bench -p reap-engine --bench event_loop --locked
"$h" "$b" engine run-1 1 -- cargo bench -p reap-engine --bench event_loop --locked
"$h" "$b" engine run-2 1 -- cargo bench -p reap-engine --bench event_loop --locked
"$h" "$b" engine run-3 1 -- cargo bench -p reap-engine --bench event_loop --locked
"$h" "$b" live warmup 1 -- cargo bench -p reap-live --bench live_loop --locked
"$h" "$b" live run-1 1 -- cargo bench -p reap-live --bench live_loop --locked
"$h" "$b" live run-2 1 -- cargo bench -p reap-live --bench live_loop --locked
"$h" "$b" live run-3 1 -- cargo bench -p reap-live --bench live_loop --locked
"$h" "$b" action warmup 1 -- cargo bench -p reap-live --bench action_path --locked
"$h" "$b" action run-1 1 -- cargo bench -p reap-live --bench action_path --locked
"$h" "$b" action run-2 1 -- cargo bench -p reap-live --bench action_path --locked
"$h" "$b" action run-3 1 -- cargo bench -p reap-live --bench action_path --locked
"$h" "$b" pm warmup 1 -- cargo bench -p reap-pm-live --bench pm_action_path --locked
"$h" "$b" pm run-1 1 -- cargo bench -p reap-pm-live --bench pm_action_path --locked
"$h" "$b" pm run-2 1 -- cargo bench -p reap-pm-live --bench pm_action_path --locked
"$h" "$b" pm run-3 1 -- cargo bench -p reap-pm-live --bench pm_action_path --locked
available_bytes=$(df --output=avail -B1 "$b" | awk 'NR == 2 {print $1}')
test "$available_bytes" -ge 268435456
test ! -e "$b/summarizer.log"
test ! -e "$b/summarizer.log.sha256"
set +e
target/tmp/goal-g-phase0-amended/summarize-baseline-campaign.sh "$b" \
  >"$b/summarizer.log" 2>&1
rc=$?
set -e
sha256sum "$b/summarizer.log" >"$b/summarizer.log.sha256"
test "$rc" -eq 0
)
```

Each attempt writes immutable combined stdout/stderr, metadata, and sanitized
`pid/ppid/comm` snapshots as
`<target>-<ordinal>-attempt-<N>.{log,meta,ps.tsv}`. A clean uncontaminated
attempt atomically writes `<target>-<ordinal>.selected`; the summarizer reads
only that selector. If and only if the helper reports predeclared
contamination and creates no selector, retain that attempt, rerun only that
same target/ordinal line with `N+1`, and then resume at the following line.
A clean nonzero command is valid red evidence,
creates the selector, and stops Phase 0; it is not replaceable contamination.
The target argument is bound to its one exact Cargo command.

Before every invocation the helper requires at least `268,435,456` available
filesystem bytes, a clean tracked/index/untracked worktree, and no
pre-existing Cargo, rustc, benchmark, combined-replay, or Reap CLI process.
During the invocation it retains one process snapshot per second plus one
final snapshot and invalidates any matching process outside the invoked Cargo
PID's descendant tree. The same-stem `.meta` records pre/post UTC time,
`HEAD`, `HEAD^{tree}`, `Cargo.lock` hash, empty status blocks, `rustc -Vv`,
`cargo -V`, `uname -a`, CPU count, exact command, command result, separate
evidence-valid/gate-pass states, and immutable log/snapshot hashes. Pre/post
repository identity must match. The raw log is never changed after its hash is
recorded. This is a serial overlap-controlled shared-host campaign; it does
not claim host idleness.

Each retained PM log must contain exactly one parseable JSON object with
`benchmark == "pm_action_path"`, three `recorded_runs`, and all in-benchmark
hard gates already passed. Extract only that object from `run-1..3`, then use
this exact comparator:

```jq
def median: sort | .[(length / 2 | floor)];
def invocation($q):
  [.recorded_runs[].action_latency_ns[$q]] | median;
{
  p50_ns:  ([.[] | invocation("p50")]  | median),
  p95_ns:  ([.[] | invocation("p95")]  | median),
  p99_ns:  ([.[] | invocation("p99")]  | median),
  p99_9_ns:([.[] | invocation("p99_9")]| median),
  max_ns:  ([.[] | invocation("max")]  | median)
}
```

The reviewed summarizer stores the three extracted PM objects as a JSON array
and applies that program. It rejects zero/multiple reports, a wrong
revision/workload/boundary/toolchain/host, a non-three-length inner run, or
any failed hard counter. It also requires engine `250,000` events and
`999,996` intents; the exact live logical/allocation projection SHA-256
`0fc1f8c034cf568b4effcc84791264e1b7aedf81e2b793feba015ab7ef3dedaa`;
and the exact Chaos-action workload/logical/allocation projection SHA-256
`0c6d3e818cc9ad9b37c1576973f1a634e2a1fc33f199382b1537d59a58de2c02`.
All four PM invocations must have exact non-timing projection SHA-256
`cc90806d19c5d2a252acbd64f3439ece2a0cb1b9d44566b84aa421d8c37b708c`.
The summary records the retained three-run medians for engine, all four live
measurements, every Chaos-action workload quantile, and PM p50/p95/p99/p99.9/
max. Retain its extracted array, summary, selectors, metadata, snapshots, and
hash manifest.

The unchanged Phase 0 workload and final candidate each run as four separate
serial overlap-controlled shared-host Cargo invocations on the same
host/toolchain/profile/boundary: one complete process-warmup suite is
discarded from comparison but retained, then three invocation reports are
compared. Each PM invocation already contains
one internal warm-up and three internal recorded distributions. For each
invocation/quantile take the median of its three internal values, then take
the median of the three retained invocation medians. Final p50 and p95 must
each be at most `1.10 ×` baseline and p99 at most `1.20 ×` baseline.
Compare integers without floating point: `final_p50 * 10 <= baseline_p50 *
11`, `final_p95 * 10 <= baseline_p95 * 11`, and `final_p99 * 5 <=
baseline_p99 * 6`, with checked `u128` products. Equality passes.
p99.9 and max are retained but not shared-host pass/fail gates. Predeclared
overlap/toolchain/profile/workload mismatch invalidates a run before values are
read and remains recorded; scheduler variance is not contamination. New
signed-request and chain-source benchmarks have hard correctness/resource
gates and report all quantiles/max, but establish local baselines rather than
absolute latency gates. Engine/live/Chaos-action use the exact investigation
and final 5%/10% same-host rules under “Determinism And Performance Gates” in
`docs/determinism-readiness-goal-d-prompt.md`, with the summary's frozen
medians and unchanged sample/percentile methods; no PM rule is applied to
them. None of this is a target-host or network SLO.

Run the Phase 0 replay gate once after the benchmark campaign:

```bash
target/tmp/goal-g-phase0-amended/run-phase0-replay.sh 1
```

It retains immutable attempt-specific evidence, monitors every Cargo/test/CLI
process tree, requires clean identical pre/post `HEAD`, tree, and `Cargo.lock`,
and atomically selects only uncontaminated evidence in `replay.selected`.
Only predeclared contamination without a selector may be rerun as attempt
`N+1`; a clean nonzero result is selected valid red evidence and stops.
The helper runs and validates the combined replay report at exactly 35,012
lines, 22,791,589 bytes, writer SHA-256
`83ced509c9ea180e66d957853f9ff7762ef3c0babc316c9251c12d4d1a5224eb`,
and recovery SHA-256
`f98bf8a88f34fb6e3c4dcfd1919a2c1d4577b2da3960375e216e596d0746cd35`;
the Goal D engine/live decision projections; the PM exact-order numeric
contract; all four frozen Goal D input hashes; and two byte-identical Chaos
backtests with SHA-256
`38acf9f5e0c310f2ec5528974beffadf4c1a7f84d46efa8d9664ee7051e84691`.

Storage is an explicit local evidence stop, not cleanup authority. Each Phase
0 executable requires at least `268,435,456` available bytes before starting.
Before Phase 1 and before every later build/global gate, require at least
`2,147,483,648` available bytes. If either check fails, stop with all evidence
intact and obtain additional storage or explicit approval for a
retained-evidence-preserving build-cache cleanup. Never delete `target/tmp`,
user data, sibling-repository data, or an invalid/selected attempt.

## Amendments Adopted

The user-authorized Amendments 1 and 2 are complete at the contract level:

1. the closed finalized-chain authorization cut above supplies the two typed
   facts without a CLOB numeric-to-boolean conversion;
2. CLOB numeric values remain separate diagnostic/fail-close evidence;
3. the exact strict source-tagged lifecycle/time union makes known
   documentation disagreement representable without guessing, including the
   distinct account-trade-REST-only `MatchedNotBroadcast` state;
4. every stable requirement ID has one lane and the closed route, dispatch,
   retry, redirect, proxy, dependency, and vector contracts are frozen;
5. the paired local PM benchmark rule replaces the invalid host-specific
   ceiling while preserving exact work/resource gates; and
6. the capability, dependency, lane, failure, and production-origin boundaries
   have been revised consistently.

The active Goal G run is in Phase 0 until its fresh benchmark, replay, and
documentation gates pass. A production Polygon origin, real-account probe,
real credential, authenticated external call, chain call, or order remains
outside Goal G.
