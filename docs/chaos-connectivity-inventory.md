# Chaos Connectivity Inventory

Status: frozen Phase 0 baseline with the completed Goal A and conditional Goal
B disposition overlays. Goal B becomes a completed structural result only
after its handoff records green focused and Phase 9 gates.

This inventory freezes the exchange surface that existed before the Chaos
connectivity boundary was enforced. It is descriptive, not an authority grant.
The normative target remains
[chaos-connectivity-boundary.md](chaos-connectivity-boundary.md).
The verified phase records are the completed
[Goal A handoff](chaos-connectivity-goal-a-handoff.md) and the conditional
[Goal B handoff](chaos-connectivity-goal-b-handoff.md).
The matrix is intentionally retained in its Phase 0 form so that “current
reachability” means reachability at the start of the refactor. The
[Goal A disposition](#goal-a-disposition) records what Phases 0–5 changed, and
the [Goal B disposition](#goal-b-disposition) records the structural result
without rewriting that historical evidence.

## Frozen Baseline

The Phase 0 starting point was the clean documentation baseline:

| Item | Recorded value |
| --- | --- |
| Reap starting commit | `418b06eb176b121cb8410c6d407427116277e717` |
| Reap status | `git status --porcelain=v1` produced no output |
| Java checkout | `../imm-strategy` existed and was readable |
| Java commit | `b6b120c7b7c466d8431bf082f3229328c5d7b2ae` |
| Java status | `git -C ../imm-strategy status --porcelain=v1` produced no output |
| Rust pin | `reap_core::PINNED_JAVA_REVISION = b6b120c7b7c466d8431bf082f3229328c5d7b2ae` |
| Rust toolchain | `rustc 1.95.0 (59807616e 2026-04-14)`; `cargo 1.95.0 (f2d3ce0 2026-03-21)` |
| Toolchain file | channel `1.95.0`, minimal profile, `clippy` and `rustfmt` components |
| `Cargo.lock` SHA-256 | `9e5e928621dd6d9ca8dd31476014ddd76fba8a42ce85e9c2508adf82513141c0` |
| Locked no-deps metadata SHA-256 | `14f0a95d68becefe204a4d24a0b3eeada913e4c968a8f764e3f280e056c5fb1b` |

No other session-created or unexplained worktree change was present. Other
observed Cargo activity was read-only and did not overlap repository files.
Every subsequent phase must recheck both worktrees and stop on unexplained
changes.

## Deterministic Baseline

All commands below were credential-free. Exit code `0` means the command
completed successfully.

| Command | Result |
| --- | --- |
| `cargo test -p reap-strategy --locked chaos::tests::normalized_fixture_drives_quote_then_hedge_decisions -- --exact` | exit `0`; 1 passed |
| `cargo test -p reap-backtest --locked tests::normalized_fixture_replays_quote_and_hedge_path -- --exact` | exit `0`; 1 passed |
| `cargo test -p reap-live --locked config::tests` | exit `0`; 31 passed |
| `cargo run --locked -q -p reap-cli -- backtest --format normalized-jsonl --config examples/iarb2-basic.toml --data fixtures/normalized/chaos_quote_hedge.jsonl --pretty` | exit `0`; run twice; byte-identical output |

The frozen input/output hashes are:

| Artifact | SHA-256 |
| --- | --- |
| `fixtures/normalized/chaos_quote_hedge.jsonl` | `27f2eb4b9dba7ee600ed645ad8b7c88143e8b54531232991b492cb7595e8ccaa` |
| `fixtures/normalized/chaos_quote_hedge_later.jsonl` | `40453b8be283178b20531c84142dbaeeeca82b4723e5c13594df171c778cd8ee` |
| `fixtures/normalized/chaos_quote_hedge_intents.json` | `d95fa7f121e2e8c402c8108cf9fefb7c7d7b3dbd2b9742c58c234a521f0ee0ec` |
| `examples/iarb2-basic.toml` | `0fac5a3a35fe28cdc05118b7e22241077aa7f604a9a5436355797605b51b3b26` |
| Pretty backtest output from the command above | `38acf9f5e0c310f2ec5528974beffadf4c1a7f84d46efa8d9664ee7051e84691` |

The strategy test asserts the exact ordered intent arrays and every serialized
quote/hedge field against the checked-in intent fixture. The backtest output
recorded 4 input events, 5 orders sent, 2 fills, no cancels/rejections, and final
delta and pending delta of zero. These fixtures, values, and the output hash are
parity sentinels for every Goal A phase.

## Capability Matrix

The machine-readable counterpart is
`reap_venue::okx::OKX_CAPABILITY_REGISTRY`. REST convenience variants such as
`*_at`, `*_raw`, parsed, and paginated methods map to the same wire-operation
row. In-process mock transports exercise the registered operation they stand
in for; the loopback fault proxy is separately classified as `TestOnly`.

| `capability_id` | Endpoint or channel | Operation | Read/write | Trust plane | Mode | Requirement ID | Consumer | Java anchor or Reap safety rationale | Current reachability | Classification | Target disposition |
| --- | --- | --- | --- | --- | --- | --- | --- | --- | --- | --- | --- |
| `OKX-REST-PLACE-REGULAR` | `POST /api/v5/trade/order` | Place regular limit order | Write | Regular execution | Demo | `CHAOS-EXEC-QUOTE`<br>`CHAOS-EXEC-HEDGE` | Quote/hedge request realization | Current Reap quote/hedge policy | Public method on broad cloneable REST client; live gateway reachable | `ChaosExecution` | Keep only behind `RegularExecution` policy |
| `OKX-REST-CANCEL-REGULAR` | `POST /api/v5/trade/cancel-order` | Cancel one regular order | Write | Regular execution/live safety | Demo | `CHAOS-EXEC-CANCEL-OWNED`<br>`SAFE-REGULAR-CANCEL` | Owned cancellation and fail-closed safety | Canonical ownership/convergence invariant | Public broad-client method; gateway reachable | `ReadinessSafety` | Keep behind owned regular cancel roles |
| `OKX-REST-CANCEL-BATCH-REGULAR` | `POST /api/v5/trade/cancel-batch-orders` | Account-wide regular batch cancel | Write | Emergency account stop | Emergency | `OPS-EMERGENCY-REGULAR` | Emergency regular mitigation | Account-wide recovery safety invariant | Public broad-client method inside `reap-live` emergency path | `EmergencyCleanup` | Move to emergency adapter/executable |
| `OKX-REST-REGULAR-CAA` | `POST /api/v5/trade/cancel-all-after` | Arm/refresh/disable regular deadman | Write | Live safety/emergency | Demo, emergency | `SAFE-REGULAR-CAA`<br>`OPS-EMERGENCY-REGULAR` | Deadman protection | Regular CAA is required fail-closed safety | Public broad-client method shared by live and emergency | `ReadinessSafety` | Split live-safety and emergency role methods |
| `OKX-REST-PUBLIC-TIME` | `GET /api/v5/public/time` | Read exchange clock | Read | Live safety/emergency/evidence | Observe, demo, emergency, offline evidence | `SAFE-CLOCK-STATUS`<br>`OPS-EMERGENCY-REGULAR`<br>`OPS-EMERGENCY-ALGO`<br>`OPS-EMERGENCY-SPREAD`<br>`EVIDENCE-PUBLIC-READ` | Clock skew and signed-request bracketing | Reap fail-closed timing/evidence invariant | Public method on authenticated broad client despite unsigned endpoint | `ReadinessSafety` | Keep in narrow read roles |
| `OKX-REST-INDEX-TICKER` | `GET /api/v5/market/index-tickers` | Read one index ticker | Read | Offline evidence | Offline evidence | `SAFE-STABLECOIN`<br>`EVIDENCE-PUBLIC-READ` | Account/equity certification | Java index valuation anchor and Reap evidence | Broad client; current live strategy uses websocket references | `EvidenceOnly` | Move REST access to evidence adapter |
| `OKX-REST-SYSTEM-STATUS` | `GET /api/v5/system/status` | Read maintenance state | Read | Live safety/evidence | Observe, demo, offline evidence | `SAFE-CLOCK-STATUS`<br>`EVIDENCE-PUBLIC-READ` | Maintenance readiness | Reap fail-closed maintenance guard | Broad client and broad live filter | `ReadinessSafety` | Keep with plan-derived relevance |
| `OKX-REST-REGULAR-PENDING` | `GET /api/v5/trade/orders-pending` | Enumerate regular pending orders | Read | Reconciliation/emergency/evidence | Observe, demo, emergency, offline evidence | `SAFE-RECONCILE`<br>`OPS-EMERGENCY-REGULAR`<br>`EVIDENCE-ACCOUNT-READ` | State convergence and zero proof | Canonical regular-state safety invariant | Broad client shared across live, emergency, evidence | `ReadinessSafety` | Split role-specific readers |
| `OKX-REST-FILLS` | `GET /api/v5/trade/fills` | Enumerate regular fills | Read | Reconciliation/evidence | Observe, demo, offline evidence | `SAFE-RECONCILE`<br>`EVIDENCE-ACCOUNT-READ` | Fill convergence/statements | Reap authoritative fill safety/evidence | Broad client shared across live and offline collectors | `ReadinessSafety` | Split reconciliation and evidence readers |
| `OKX-REST-ORDER-DETAILS` | `GET /api/v5/trade/order` | Read one regular order | Read | Reconciliation/evidence | Observe, demo, offline evidence | `SAFE-RECONCILE`<br>`EVIDENCE-ACCOUNT-READ` | Restart ambiguity/deadman evidence | Reap convergence invariant | Broad client shared across live and evidence | `ReadinessSafety` | Split reconciliation and evidence readers |
| `OKX-REST-ACCOUNT-INSTRUMENTS` | `GET /api/v5/account/instruments` | Read authenticated instrument rules | Read | Live readiness/evidence | Observe, demo, offline evidence | `SAFE-METADATA`<br>`EVIDENCE-ACCOUNT-READ` | Rule bootstrap/drift | Java instrument binding plus Reap validation | Broad client shared across live/evidence | `ReadinessSafety` | Split metadata and evidence readers |
| `OKX-REST-ACCOUNT-TRADE-FEE` | `GET /api/v5/account/trade-fee` | Read authenticated fee rules | Read | Live readiness/evidence | Observe, demo, offline evidence | `SAFE-METADATA`<br>`EVIDENCE-ACCOUNT-READ` | Fee bootstrap/drift | Java fee binding plus Reap underpricing guard | Broad client shared across live/evidence | `ReadinessSafety` | Split metadata and evidence readers |
| `OKX-REST-ACCOUNT-CONFIG` | `GET /api/v5/account/config` | Read account/key/mode/STP config | Read | Readiness/emergency identity/evidence | Observe, demo, emergency, offline evidence | `SAFE-METADATA`<br>`OPS-EMERGENCY-IDENTITY`<br>`EVIDENCE-ACCOUNT-READ` | Account-mode/STP/key validation and identity | Reap fail-closed account binding | Broad client shared across all authenticated roots | `ReadinessSafety` | Split metadata, emergency identity, and evidence readers |
| `OKX-REST-ACCOUNT-BALANCE` | `GET /api/v5/account/balance` | Read account balances/margin | Read | Reconciliation/evidence | Observe, demo, offline evidence | `SAFE-RECONCILE`<br>`EVIDENCE-ACCOUNT-READ` | Cash/equity/margin convergence | Java account inputs plus Reap safety | Broad client shared across live/evidence | `ReadinessSafety` | Split reconciliation and evidence readers |
| `OKX-REST-ACCOUNT-POSITIONS` | `GET /api/v5/account/positions` | Read all/scoped positions | Read | Reconciliation/evidence | Observe, demo, offline evidence | `SAFE-RECONCILE`<br>`SAFE-ACCOUNT-POSITIONS`<br>`EVIDENCE-ACCOUNT-READ` | Position convergence/foreign exposure | Java positions plus account-wide Reap guard | Broad client shared across live/evidence | `ReadinessSafety` | Split reconciliation and evidence readers |
| `OKX-REST-ACCOUNT-BILLS` | `GET /api/v5/account/bills` | Enumerate account bills | Read | Offline evidence | Offline evidence | `EVIDENCE-ACCOUNT-READ` | Economic statement collection | Java bill-details evidence anchor | Broad client exposed from `reap-live` collector | `EvidenceOnly` | Move to evidence adapter |
| `OKX-REST-ALGO-PENDING` | `GET /api/v5/trade/orders-algo-pending` | Enumerate each pending algo family | Read | Forbidden observer/emergency/evidence | Observe, demo, emergency, offline evidence | `SAFE-FORBIDDEN-ZERO`<br>`OPS-EMERGENCY-ALGO`<br>`EVIDENCE-ACCOUNT-READ` | Unsupported-domain zero proof | Reap fail-closed exposure invariant | Broad client currently reached only by emergency | `ReadinessSafety` | Add read-only live observer; split emergency/evidence readers |
| `OKX-REST-CANCEL-ALGO` | `POST /api/v5/trade/cancel-algos` | Cancel pending algo orders | Write | Emergency account stop | Emergency | `OPS-EMERGENCY-ALGO` | Emergency algo mitigation | Account-wide recovery safety invariant | Public broad-client method inside `reap-live` | `EmergencyCleanup` | Move to emergency adapter/executable |
| `OKX-REST-SPREAD-PENDING` | `GET /api/v5/sprd/orders-pending` | Enumerate spread orders | Read | Forbidden observer/emergency/evidence | Observe, demo, emergency, offline evidence | `SAFE-FORBIDDEN-ZERO`<br>`OPS-EMERGENCY-SPREAD`<br>`EVIDENCE-ACCOUNT-READ` | Unsupported-domain zero proof | Reap fail-closed exposure invariant | Broad client currently reached only by emergency | `ReadinessSafety` | Add read-only live observer; split emergency/evidence readers |
| `OKX-REST-SPREAD-MASS-CANCEL` | `POST /api/v5/sprd/mass-cancel` | Cancel all spread orders | Write | Emergency account stop | Emergency | `OPS-EMERGENCY-SPREAD` | Emergency spread mitigation | Account-wide recovery safety invariant | Public broad-client method inside `reap-live` | `EmergencyCleanup` | Move to emergency adapter/executable |
| `OKX-REST-SPREAD-CAA` | `POST /api/v5/sprd/cancel-all-after` | Arm spread deadman | Write | Emergency account stop | Emergency | `OPS-EMERGENCY-SPREAD` | Emergency spread protection | Account-wide recovery safety invariant | Public broad-client method inside `reap-live` | `EmergencyCleanup` | Move to emergency adapter/executable |
| `OKX-WS-BOOKS` | `books` | Subscribe/parse full book | Read | Live observation/capture | Observe, demo, capture | `CHAOS-MD-BOOK`<br>`CAPTURE-PUBLIC-MARKET` | Quote/hedge book and capture | `ChaosStrategyBase` depth path | Generic adapter and configured live/capture subscriptions | `ChaosObservation` | Keep plan-derived |
| `OKX-WS-BOOKS-L2-TBT` | `books-l2-tbt` | Subscribe/parse capture depth variant | Read | Capture | Capture | `CAPTURE-PUBLIC-MARKET` | Credential-free market capture | Separate capture contract, not Chaos authority | Capture config admits the channel | `EvidenceOnly` | Keep only in separate capture contract |
| `OKX-WS-BOOKS50-L2-TBT` | `books50-l2-tbt` | Subscribe/parse capture depth variant | Read | Capture | Capture | `CAPTURE-PUBLIC-MARKET` | Credential-free market capture | Separate capture contract, not Chaos authority | Capture config admits the channel | `EvidenceOnly` | Keep only in separate capture contract |
| `OKX-WS-TRADES` | `trades` | Subscribe/parse public trades | Read | Live observation/capture | Observe, demo, capture | `CHAOS-MD-TRADE`<br>`CAPTURE-PUBLIC-MARKET` | Trade-driven Java behavior/capture | `OkEntity.onPublicTrade`, `Iarb2Strategy.onPublicTrade` | Generic adapter and configured live/capture subscriptions | `ChaosObservation` | Retain plan-derived despite current Rust behavior gap |
| `OKX-WS-TRADES-ALL` | `trades-all` | Subscribe/parse capture trade variant | Read | Capture | Capture | `CAPTURE-PUBLIC-MARKET` | Credential-free market capture | Separate capture contract, not Chaos authority | Capture config admits the channel | `EvidenceOnly` | Keep only in separate capture contract |
| `OKX-WS-FUNDING-RATE` | `funding-rate` | Subscribe/parse funding | Read | Live observation/capture | Observe, demo, capture | `CHAOS-REF-FUNDING`<br>`CAPTURE-PUBLIC-MARKET` | Funding-aware pricing/capture | `Iarb2Calculator.getFundingRate` | Config-derived live/capture subscription | `ChaosObservation` | Keep when configured |
| `OKX-WS-INDEX-TICKERS` | `index-tickers` | Subscribe/parse index | Read | Live observation/capture | Observe, demo, capture | `CHAOS-REF-INDEX`<br>`SAFE-STABLECOIN`<br>`CAPTURE-PUBLIC-MARKET` | Strategy/stablecoin references | Java index/depeg paths | Config-derived live/capture subscription | `ChaosObservation` | Keep when configured |
| `OKX-WS-PRICE-LIMIT` | `price-limit` | Subscribe/parse price limits | Read | Live observation/capture | Observe, demo, capture | `CHAOS-REF-LIMITS`<br>`CAPTURE-PUBLIC-MARKET` | Quote/hedge bounds | Explicit Reap execution safety | Config-derived live/capture subscription | `ChaosObservation` | Keep when configured |
| `OKX-WS-MARK-PRICE` | `mark-price` | Subscribe/parse mark | Read | Live observation/capture | Observe, demo, capture | `CHAOS-REF-MARK`<br>`CAPTURE-PUBLIC-MARKET` | Derivative valuation/safety | Explicit Reap valuation safety | Config-derived live/capture subscription | `ChaosObservation` | Keep when configured |
| `OKX-WS-ORDERS` | `orders` | Subscribe/parse regular order updates | Read | Live observation | Observe, demo | `CHAOS-STATE-ORDERS` | Canonical order/fill state | Java private order path and Reap convergence | One dedicated authenticated socket per channel/account | `ChaosObservation` | Pack onto planned private state socket |
| `OKX-WS-FILLS` | `fills` | Subscribe/parse fee-bearing fills | Read | Live observation | Observe, demo | `CHAOS-STATE-ORDERS`<br>`SAFE-RECONCILE` | Canonical optional fill consumer | Reap fee/convergence hardening | Optional dedicated authenticated socket per account | `ChaosObservation` | Keep only if configured and deduplicated |
| `OKX-WS-ACCOUNT` | `account` | Subscribe/parse account state | Read | Live observation | Observe, demo | `CHAOS-STATE-ACCOUNT` | Cash/equity/margin/risk | Java account state path | Dedicated authenticated socket per account | `ChaosObservation` | Pack onto planned private state socket |
| `OKX-WS-POSITIONS` | `positions` | Subscribe/parse positions | Read | Live observation | Observe, demo | `CHAOS-STATE-POSITIONS`<br>`SAFE-ACCOUNT-POSITIONS` | Position/fill convergence | Java position state plus Reap account-wide guard | Dedicated authenticated socket per account | `ChaosObservation` | Pack onto planned private state socket |
| `OKX-WS-SUBSCRIBE` | websocket `subscribe` op | Send channel subscription | Write/control | Observation/capture | Observe, demo, capture | Channel requirement being subscribed | Subscription bootstrap | Required to consume admitted channels | Generic adapter can serialize arbitrary custom channel | `ChaosObservation` | Permit only planned channel args |
| `OKX-WS-LOGIN` | websocket `login` op | Sign/authenticate private/order socket | Write/control | Observation/execution | Observe, demo | Private state or execution requirement | Authenticated session bootstrap | Required for admitted private state/regular execution | Public signer method can construct login anywhere | `ChaosObservation` | Make private to role session factories |
| `OKX-WS-PLACE-REGULAR` | business websocket `order` op | Place regular limit order | Write | Regular execution | Demo | `CHAOS-EXEC-QUOTE`<br>`CHAOS-EXEC-HEDGE` | Quote/hedge execution | Current Reap policy | Public request builder; live order pool reachable | `ChaosExecution` | Keep behind regular order session factory/policy |
| `OKX-WS-CANCEL-REGULAR` | business websocket `cancel-order` op | Cancel one regular order | Write | Regular execution/live safety | Demo | `CHAOS-EXEC-CANCEL-OWNED`<br>`SAFE-REGULAR-CANCEL` | Owned cancellation | Canonical ownership/convergence invariant | Public request builder; live order pool reachable | `ReadinessSafety` | Keep behind regular order session factory |
| `OKX-WS-LIVENESS` | websocket `ping`, `pong`, and `close` frames | Maintain, bound, and close sessions | Read/write control | Observation/execution/capture/fault | Observe, demo, capture, test | Every admitted public/private/command session binding in the registry | Feed/order/fault session liveness | Required for bounded readiness, reconnect, and shutdown | Generic feed, order-command, capture, and fault-proxy loops | `ReadinessSafety` | Keep inside admitted session implementations |
| `OKX-CONNECTION-PUBLIC` | Public websocket | Construct supervised live public session | Connect | Live observation | Observe, demo | `CHAOS-MD-BOOK`<br>`CHAOS-MD-TRADE` and configured references | Normalized public consumers | Pinned inputs and explicit Reap safety inputs | Global replica count copied to every subscription | `ChaosObservation` | Exact per-requirement plan/replicas |
| `OKX-CONNECTION-PRIVATE-STATE` | Private websocket | Construct authenticated state session | Connect | Live observation | Observe, demo | `CHAOS-STATE-ORDERS`<br>`CHAOS-STATE-ACCOUNT`<br>`CHAOS-STATE-POSITIONS` | Canonical private state | Required convergence | One channel per socket/account after partitioning | `ChaosObservation` | Pack compatible planned channels per account |
| `OKX-CONNECTION-ORDER-COMMAND` | Business websocket | Construct authenticated order session | Connect | Regular execution | Demo | `CHAOS-EXEC-QUOTE`<br>`CHAOS-EXEC-HEDGE`<br>`CHAOS-EXEC-CANCEL-OWNED` | Account dispatch families | Regular command path | Eight sessions by default; families hash to one; all gate readiness | `ChaosExecution` | One nonempty plan-derived shard/account baseline |
| `OKX-CONNECTION-CAPTURE-PUBLIC` | Public websocket | Construct capture sockets | Connect | Capture | Capture | `CAPTURE-PUBLIC-MARKET` | Credential-free market capture | Offline dataset requirement | Separate capture config and replica/chunk plan | `EvidenceOnly` | Preserve separate capture contract |
| `OKX-CONNECTION-FAULT-PROXY` | Loopback REST/public/private/order proxy | Forward/inject controlled failures | Connect/read/write | Fault tooling | Test | `TEST-FAULT-TRANSPORT` | Deterministic fault campaigns | Loopback-only test safety | Separate proxy executable and config | `TestOnly` | Keep outside Chaos plan |
| `OKX-MAINTENANCE-FILTER` | System-status service/environment/product fields | Decide readiness relevance | Filter | Live safety | Observe, demo | `SAFE-CLOCK-STATUS` | Maintenance guard | Fail closed for required connectivity | Hard-coded broad Java-style service set includes spread | `ReadinessSafety` | Derive relevance from exact plan |
| `OKX-AUTH-CREDENTIAL-GETTERS` | `OkxCredentials` getters | Read raw API key/passphrase/secret-adjacent material | Authority | Shared authenticated client | All authenticated callers | — | Bypass surface | No strategy requirement | Public getters | `Remove` | Private wire-only access |
| `OKX-AUTH-RAW-SIGNATURE` | `OkxSigner::signature` | Sign arbitrary prehash | Authority | Shared authenticated client | All authenticated callers | — | Bypass surface | No strategy requirement | Public method | `Remove` | Private wire-only access |
| `OKX-AUTH-SIGN-REQUEST` | `OkxSigner::sign_request`, `SignedRequest` | Construct arbitrary authenticated request | Authority | Shared authenticated client | All authenticated callers | — | Bypass surface | No strategy requirement | Public type/method | `Remove` | Private wire-only access |
| `OKX-AUTH-WS-LOGIN` | `OkxSigner::websocket_login` | Construct arbitrary login payload | Authority | Shared authenticated client | Observe, demo | — | Bypass surface | Login is needed only within admitted factories | Public method | `Remove` | Private to role-owned session factories |
| `OKX-AUTH-SIGNER-GETTER` | `OkxRestClient::signer` | Recover broad signer | Authority | Shared authenticated client | All authenticated callers | — | Bypass surface | No role needs signer escape | Public method | `Remove` | Remove |
| `OKX-AUTH-RAW-TRANSPORT` | `HttpTransport::execute` | Execute arbitrary signed request | Authority | Shared authenticated client | All authenticated callers/tests | — | Bypass/test surface | Tests need role fakes, not production raw access | Public trait/method | `Remove` | Private wire; inject narrow fakes |
| `OKX-AUTH-BROAD-REST-CLIENT` | `OkxRestClient` | Construct cloneable union of all endpoint families | Authority | Shared authenticated client | Live, emergency, evidence | — | Current composition roots | Union exceeds every narrow role | Public generic client and wildcard export | `Remove` | Replace with non-interchangeable role clients |

## Completeness Notes

- All REST path constants in `reap-venue/src/okx/rest.rs` and
  `rest/pending_orders.rs` resolve through registered rows.
- The OKX adapter's channel serializer/parser, capture-only aliases, regular
  order operations, and liveness control operations resolve through or are
  explicitly classified by registered rows.
- Public live, authenticated private-state, business order-command, capture,
  and loopback fault-proxy connection constructors are classified above.
- The broad maintenance filter is classified even though it is not an exchange
  endpoint.
- Raw signer, signed-request, transport, broad-client, and wildcard reachability
  are recorded because dormant authority still violates the target boundary.
- Test mocks do not add exchange capabilities: each is a deterministic stand-in
  for a registered operation. No credentialed endpoint was contacted to build
  this inventory.
- Registry requirement lists are exhaustive capability-family bindings, not
  examples. Plan resolution may select a subset only by resolving the exact
  consumer requirements for the current mode/configuration.

## Phase 0 Gate

Phase 0 changes only documentation, the secret-free registry, mechanical use of
registered endpoint/channel constants, and registry completeness tests. It does
not alter strategy decisions, serialized schemas, network composition, or
exchange request bytes.

Final gate commands and results:

| Command | Result |
| --- | --- |
| `cargo fmt --all -- --check` | exit `0` |
| `cargo test -p reap-venue --locked --no-fail-fast` | exit `0`; 56 passed |
| `cargo test -p reap-capture --locked --no-fail-fast` | exit `0`; 48 passed |
| `cargo test -p reap-live --locked --no-fail-fast` | exit `0`; 252 passed |
| `cargo test -p reap-strategy --locked chaos::tests::normalized_fixture_drives_quote_then_hedge_decisions -- --exact` | exit `0`; 1 passed; exact ordered-intent golden matched |
| `cargo test -p reap-backtest --locked tests::normalized_fixture_replays_quote_and_hedge_path -- --exact` | exit `0`; 1 passed |
| Frozen credential-free backtest command from the baseline table | exit `0`; output SHA-256 remained `38acf9f5e0c310f2ec5528974beffadf4c1a7f84d46efa8d9664ee7051e84691` |
| `git diff --check` | exit `0` |

The final Phase 0 recheck found the sibling checkout clean at
`b6b120c7b7c466d8431bf082f3229328c5d7b2ae`, equal to the Rust pin. The
`Cargo.lock` SHA-256 remained
`9e5e928621dd6d9ca8dd31476014ddd76fba8a42ce85e9c2508adf82513141c0`.

Gate result: green. The staged diff/tree and commit identifiers are recorded by
the Phase 0 Git commit; no strategy/runtime behavior or serialized schema
changed.

## Goal A Disposition

Goal A completed Phases 0–5 and the Tranche A gate. This overlay does not
rewrite the frozen matrix above and does not claim that Goal B or the
production-readiness gate is complete.

| Deviation | Goal A disposition |
| --- | --- |
| `D01` broad signer, raw transport, and broad REST authority | Enforced. Production wire details are private to role-owned adapter crates. Compile-fail fixtures reject raw authentication/transport/client recovery and unsupported mutation from narrow roles. |
| `D02` wildcard exports | Enforced for the audited OKX/live authority surface. Public exports are explicit and role-oriented. |
| `D03` Java public-trade behavior versus current Rust semantics | Intentionally retained and still documented. `CHAOS-MD-TRADE` remains plan-derived for every configured instrument; Goal A did not pretend to close the separate trade-driven implied-depth behavior gap. |
| `D04` copied eight-session command pool | Enforced. Each executing account receives exactly one plan-derived nonempty command lane; a reference-only account receives none. `order_websocket_sessions` is a backward-compatible maximum cap, not requested cardinality. |
| `D05` broad maintenance relevance | Enforced. Relevance is derived from the resolved regular Chaos plan. Spread-only maintenance is irrelevant; relevant and unknown/ambiguous status remains fail-closed. |
| `D06` missing forbidden-domain proof | Enforced. A read-only per-account observer exhaustively covers one seven-query-family algo domain plus the spread domain. Initial and recurring proof, expiry, strict pagination, and fail-closed cancellation/reconciliation behavior are composed without algo/spread mutation authority. The runtime attempts to queue a typed critical event to the configured alert sink when enabled; delivery remains an operational gate. |
| `D07` coupled emergency progress | Enforced. Regular mitigation starts first. Regular, algo, and spread own separate pacers, progress, incidents, and zero proofs, and independently enforce one shared absolute per-account deadline. Results merge deterministically and `all_clear` remains conjunctive. |
| `D08` emergency mutation inside live | Enforced. Emergency core, runner, and OKX adapter are separate crates; the normal `reap-live` dependency graph contains no emergency crate or emergency adapter. Live attempts to queue an operator alert through an enabled sink but cannot invoke emergency mutation. |
| `D09` backtest depends on live | Deferred to Goal B Phase 6. The normal `reap-backtest -> reap-live` edge still exists and must not be described as removed by Goal A. |
| `D10` concentrated large modules | Deferred to Goal B Phases 7–8. Goal A changed authority ownership where required but did not claim the responsibility-based module split. |
| `D11` one global public replica count | Enforced. Every book has two named sequencing/deduplication/recovery replicas; each trade and configured reference input has one. Legacy connection count is only a migration cap. |
| `D12` one private channel per socket | Enforced. Compatible required private channels are packed onto one exact plan-derived authenticated state socket per account, with positions alone for an unused observation-only account or account/orders/positions plus configured fills for an executing account. Order-channel fills remain canonical; no channel is added merely to make shapes uniform. |

The final implementation anchors before the documentation-only handoff commit
are:

| Item | Recorded value |
| --- | --- |
| Goal A implementation commit | `ab7842446b9cb4f48ccc70425b0c8731ac9eac5f` |
| Java checkout and Rust pin | `b6b120c7b7c466d8431bf082f3229328c5d7b2ae`; sibling checkout clean |
| `Cargo.lock` SHA-256 | `74ca0a2b8fd028250cc243832ee7b169dc21ba26e3cf49713add4c7ff8cea213` |
| Canonical sample Demo-plan SHA-256 | `6771c97a373f12f77093624ea4b2914d867aae6a710eddadde925fc288fc6477` |
| Ordered-intent fixture SHA-256 | `d95fa7f121e2e8c402c8108cf9fefb7c7d7b3dbd2b9742c58c234a521f0ee0ec` |
| Pretty backtest output SHA-256 | `38acf9f5e0c310f2ec5528974beffadf4c1a7f84d46efa8d9664ee7051e84691` |

The documentation commit containing this overlay is the final Goal A handoff
commit. Exact phase commits, commands, and results are recorded in
[chaos-connectivity-goal-a-handoff.md](chaos-connectivity-goal-a-handoff.md).
No credentialed exchange call, production evidence, or production order-entry
claim is part of this result.

## Goal B Disposition

Goal B preserves the frozen Phase 0 matrix and the Goal A authority graph while
removing structural coupling. It does not reinterpret a baseline
“current reachability” cell as current implementation truth. The normative
capability set remains
[chaos-connectivity-boundary.md](chaos-connectivity-boundary.md), under the
clean `../imm-strategy` checkout and
`reap_core::PINNED_JAVA_REVISION` at
`b6b120c7b7c466d8431bf082f3229328c5d7b2ae`.

The final scope is deliberately separated:

| Scope | Final disposition |
| --- | --- |
| Current Chaos capability | Consume only plan-derived normalized inputs. The only executable purposes are regular PostOnly quote, regular IOC `CancelMaker` hedge, and cancellation of a regular order with canonical Reap ownership proof. |
| Reap safety hardening | May perform narrow authenticated reads, regular reconciliation, canonical owned-regular cancellation, regular Cancel All After, host/readiness checks, and read-only algo/spread zero proof. Safety observation does not authorize unsupported order placement or account-wide mutation. |
| Evidence and research | Credential-free public capture and deterministic backtest/research, separately composed authenticated-read-only collection, and offline certification/verification have no live strategy mutation authority and cannot authorize trading. |
| Account-wide emergency recovery | The separate emergency executable may enumerate and cancel regular, algo, and spread orders and arm regular/spread deadmen. It can never submit and is absent from the normal live dependency graph. |
| Explicitly not implemented | Amend/batch amend, GTC/FOK/market order profiles, trigger/conditional/OCO/chase/iceberg/TWAP/smart-algo placement, spread placement, arbitrary reduce-only/STP combinations, margin-spot borrowing, master/group feeds, additional venues, generic strategy plugins, and production order entry. |

The regular-order capability chain is:

```text
Quote | Hedge
  -> RegularExecutionPolicy -> ApprovedRegularSubmit
  + gateway-bound GeneratedClientOrderId
  -> OwnedRegularOrders::reserve_local
  -> canonical PendingNew + ownership -> ReservedRegularSubmit
  -> OkxOrderGateway -> PreparedRegularSubmit

CancelOwned
  -> RegularExecutionPolicy -> ApprovedRegularCancel
  -> OkxOrderGateway -> PreparedRegularCancel

PreparedRegularSubmit | PreparedRegularCancel
  -> reap-okx-live-adapter order-command session
  -> private OKX wire DTO / websocket bytes
```

Only `RegularExecutionPolicy` can turn the supported Chaos purposes into
opaque `ApprovedRegular*` values. For submit, a gateway-bound generator and the
coordinator consume the approval while synchronously registering canonical
`PendingNew` ownership and produce `ReservedRegularSubmit`; only
`OkxOrderGateway` can consume that reservation. For cancel, the gateway
consumes the approved cancel directly. It validates the binding and applies
idempotency and trade mode as applicable to produce opaque
`PreparedRegular*` values. The dispatcher reserves pacing before adapter IO.
The live adapter owns normal-live command-session authentication, lifecycle,
acknowledgement correlation, and final private lowering to OKX DTO/JSON.
Strategy, coordinator, and gateway consumers cannot construct raw payloads or
recover a signer from these values. Evidence has no mutation authority;
emergency owns a separate cancel-only wire root and cannot receive this
prepared-command chain or construct a place request.

Normal authority is established only by the synchronous gateway-bound local
reservation or by one-shot recovery from the exclusively leased canonical
journal. Private-state rows, reconciliation, a client-ID prefix, and a
free-form reason remain evidence and cannot mint an approval or ownership
proof.

Only one-shot `recover_leased_jsonl(&mut StorageLease)` retains non-Clone recovery proofs from the exact canonical journal; ordinary path/byte recovery strips them. Proofs are consumed and rebound to the current gateway scope. This is a structural authority boundary rooted in an exclusively leased, operator-controlled journal, not cryptographic authentication of disk contents.

Goal B resolves the deviations that Goal A intentionally deferred:

| Deviation | Goal B disposition |
| --- | --- |
| `D09` backtest depends on live | Resolved in Phase 6. Pure live configuration, connectivity, account-certification, pacing, and key contracts were lowered into dependency-safe crates. `reap-backtest` depends on `reap-live-contracts`, not the runtime crate, and the shared contracts contain no networking or credential execution. |
| `D10` concentrated large modules | Resolved in Phase 7 without creating concurrent state writers. Live runtime responsibilities are split into composition, connectivity, dispatch, readiness/safety, reconciliation, and shutdown modules; research into configuration, execution, reporting, and verification; capture into configuration, runtime, writer, report, analysis, verification, hashing, error, and cleanliness responsibilities. `LiveRuntime`/`LiveCoordinator` remain the ordered owner of strategy, risk, and canonical order mutation. |
| Duplicated host-health classification | Resolved in Phase 8 by `HostHealthThresholdAssessment`, `HostGuardConfig::assess_host_health`, and `HostHealthSnapshot::threshold_assessment`; runtime and evidence consume the same pure threshold decision. |
| Duplicated capture-clean classification | Resolved in Phase 8 by the private `CaptureCleanRunInputs` and `capture_run_is_clean` contract used by runtime and verification. |
| Duplicated live soak/fault classification | Resolved in Phase 8 by private `LiveCleanSoakInputs`, `LiveFaultFailureCode`, and `LiveFaultFailureClass` contracts shared by runtime/fault reporting and independent verification. |
| Regular execution values retained too much wire authority | The Phase 8 implementation candidate uses the approved/reserved/prepared chain above. Values crossing strategy, coordinator, gateway, and adapter boundaries expose only the fields needed by the next role; raw authenticated wire construction remains adapter-private. This row is resolved only when the recorded focused and global gates pass. |
| Order-command websocket ownership outside the wire adapter | The implementation candidate moves the final hop above into the adapter. Consuming startup on an account-bound nonseparable gateway/session bundle validates the supplied destination/account, installs its private matching slot before spawn, and then returns the now-bound gateway. Besides that gateway, only typed lifecycle/status observation returns to live. The current plan has exactly one command shard per executing account. A tree in which connection/login/write/ack/reconnect/shutdown or prepared-to-wire lowering remains owned by `reap-live` must not pass the Phase 9 gate. |
| Recovery parsing could recreate mutation authority | The implementation candidate preserves proofs only through take-once leased recovery; ordinary path/byte recovery strips them. This requires no journal schema change and is Rust structural hardening with an explicitly operator-controlled, non-cryptographic trust root. |

Implementation commits through the named safety-contract work are:

| Phase | Commits |
| --- | --- |
| Phase 6 | `0915bdb`, `64b2d55`, `2ea108f`, `51ea2b0`, `b6eb537` |
| Phase 7 | `9eef036`, `aad0630`, `ad3e9b7`, `cc88a15`, `4951e70`, `1d2b0f8` |
| Phase 8 named safety contracts | `dd0d5db`, `2f172e8` |
| Phase 8 regular authority | `38babe6e4d12d598730d3c79aeeccbbec1ec018d` |
| Phase 8 adapter-owned command websocket | `246f5b21d046dc20fd84460c7b59346231d6107f` |
| Phase 9 documentation/verification base | `[PENDING_GOAL_B_DOCS_BASE_SHA]` |

Exact trees, patch IDs, commands, focused counts, deterministic anchors, and
global results are recorded conditionally in
[chaos-connectivity-goal-b-handoff.md](chaos-connectivity-goal-b-handoff.md).
No credentialed exchange call, production evidence, demo approval, or
production-readiness claim is part of Goal B.
