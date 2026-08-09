# Polymarket Trading Connectivity Goal PM-T1 Handoff

Status: implementation, final local acceptance gates, and authorized local
commit closeout green; production authorization remains false.

`production_order_entry_authorized = false`

No real credential was loaded, no external authenticated request was made, and
no real order was sent. All authenticated transport evidence uses synthetic
credentials and numeric owner-local loopback origins under the non-default
`loopback-evidence` feature.

## Authority And Reference Pins

- Runnable authority: `docs/polymarket-trading-connectivity-goal.md`.
- Reap baseline: `4a0ae61487b2d8d8996b797a2dd239dc877bc219`.
- Predarb differential reference: `../predarb` commit
  `8222273a9c72033b760e1d2fec813bc77144556d`.
- IMM/Chaos architecture reference: `../imm-strategy` commit
  `b6b120c7b7c466d8431bf082f3229328c5d7b2ae`.
- Official TypeScript CLOB reference:
  [`Polymarket/clob-client-v2` at `f3e1a05f868a1fd0c34ef85dfc45c6ce78f5bb69`](https://github.com/Polymarket/clob-client-v2/tree/f3e1a05f868a1fd0c34ef85dfc45c6ce78f5bb69).
- Official pages rechecked for the reached routes on 2026-08-08:
  [authentication](https://docs.polymarket.com/api-reference/authentication),
  [place order](https://docs.polymarket.com/api-reference/trade/post-a-new-order),
  [manage orders and account trades](https://docs.polymarket.com/trading/manage-orders),
  [cancel](https://docs.polymarket.com/trading/orders/cancel),
  [fees](https://docs.polymarket.com/trading/fees), and
  [user WebSocket](https://docs.polymarket.com/market-data/websocket/user-channel).
  The pinned TypeScript `Trade`/`MakerOrder` types carry `fee_rate_bps`; Reap
  treats only explicit zero as exact-zero fee authority.

Predarb is a wire/authentication differential reference only. Its broad client
API, credential containers, floating conversions, filtered reconciliation,
and application/runtime architecture are not Reap authority.

## Implemented Cut

The workspace now has 38 crates. PM-T1 added the edge crates
`reap-polymarket-auth` and `reap-polymarket-live-adapter`; the later
read-only-qualification milestone added the separate `reap-pm-readiness`
composition without changing PM-T1's mutation boundary. Goal F's fake root
remains first-class. A distinct feature-gated authenticated-loopback root
consumes the same product, destroys its fixture executor, and exposes no
runtime backend selector or production-origin constructor.

The reached product remains one EOA/type-0 account, one condition, one outcome
token, one exact GTC post-only place, and exact journal-owned cancellation.
OKX supplies only the configured public `index-tickers` reference. The PM graph
contains no OKX private/account/order/algo/spread role, generic request or
signer, batch/amend/cancel-all, allowance mutation, settlement, redemption, or
production credential capability.

- `reap-polymarket-auth` owns fixed EIP-712 CLOB V2 signing and purpose-specific
  L2 authentication with redacted, zeroizing, non-`Clone` secret holders.
- `reap-polymarket-live-adapter` owns the bounded allowlisted public,
  authenticated-read, user-WebSocket, fixed-place, and exact-owned-cancel
  transports. Redirects, proxies, and transport retries are disabled for the
  loopback mutation roles.
- One read-ingress supervisor owns public/user sockets, public book reads, and
  complete authenticated open-order/trade/order-detail/account reads. Partial
  pagination exposes no canonical rows.
- One split product clock supplies public/private/read, actor/control,
  OKX-reference, and separate place/cancel time roles. Application `PONG`
  receipt rebases the next public heartbeat deadline.
- Strict live normalization feeds the existing owner-bound reducers. An
  explicit `fee_rate_bps = "0"` proves an exact zero collateral-fee delta; an
  omitted rate remains `Unknown` and a nonzero rate remains `Incomplete`. Both
  retain the unresolved fill and block readiness/placement. No nonzero fee is
  inferred and PM-T1 contains no production fee model.
- The Goal F journal remains canonical. A separate secret-free authenticated
  journal adds durable `Prepared`, `DispatchAuthorized`, and Result records,
  followed by one durable Goal F Result bridge. The serialized body, runtime
  exact-body digest, credentials, signatures, HMACs, passphrases, authenticated
  frames, and hashes derived from them are excluded from both journals.
- Cross-journal startup validates exact scope, prior intent, operation identity,
  and sequence. It may repair one missing Goal F bridge from a conclusive
  authenticated Result and never resends.
- Journal recovery reconstructs each durable fill in the canonical shape of
  its original source: immediate acknowledgements retain exact local client
  identity, while private-WebSocket and REST observations retain venue identity
  only. The journal-bound owned lifecycle still validates the exact client
  order, so a fresh REST duplicate can enrich a replayed WebSocket fill without
  changing principal.
- Restart seeds the durable fill cursor before issuing fresh reads and
  reconstructs accepted-cancel detail obligations. Reconnect projects recovered
  typed refresh tickets in FIFO order, but leaves `MissingOrderDetail` pending
  until a complete OpenOrders cut establishes the current generation; bounded
  projection saturation fails closed.
- A recovered `GrantTail` is may-have-sent, fail-closed, and
  reconciliation-required. It is never resent and, without an accepted venue
  order ID, is not restart-cancelable through the exact-owned path.
- Controlled shutdown disables placement, gracefully joins post-dispatch
  mutation tasks without abort, continues read/durability service, and drives a
  bound live owned order through exact cancel and reconciliation when possible.
  Typed primary, secondary, and fixed unresolved evidence is retained.

## Focused Verification Recorded So Far

These exact focused commands were reported green on the final implementation
tree and remain useful pinpoint evidence alongside the aggregate gates below:

```text
cargo test -p reap-polymarket-live-adapter delayed_application_pong_rebases_two_cycle_transport_and_canonical_session --locked
# 1 passed

cargo test -p reap-pm-live --lib authenticated_live_cuts_reach_one_goal_f_durable_place_dispatch --features loopback-evidence --locked
# 1 passed

cargo test -p reap-pm-live --lib authenticated_place_fill_exact_cancel_and_restart_converge_without_resend --features loopback-evidence --locked -- --nocapture
# 1 passed

cargo test -p reap-pm-live --lib stale_or_unavailable_live_dependencies_suppress_place_but_preserve_exact_cancel --features loopback-evidence --locked
# 1 passed

cargo test -p reap-pm-live --lib collateral_success_then_conditional_failure_never_delivers_a_complete_account_cut --features loopback-evidence --locked
# 1 passed

cargo test -p reap-pm-live --lib authentication_failure_is_primary_retains_exact_neutral_place_and_writes_nothing --features loopback-evidence --locked
# 1 passed

cargo test -p reap-pm-live --lib durable_artifacts_exclude_secret_and_runtime_exact_body_canaries --features loopback-evidence --locked
# 1 passed
```

The authenticated vertical admitted one durable place and one transport POST,
observed the partial private fill plus exact account/trade cut, drove one exact
owned cancel and terminal detail, then restarted on epoch 2 with both journal
bridges and the causal fill cursor intact. Fresh restart cuts converged to the
runtime terminal projection while POST and DELETE counts remained unchanged;
neither mutation was resent.

The other four newly recorded focused tests cover the PM/OKX stale,
unavailable, and private-disconnect matrix with safe cancel retention; rejection
of a partial account cut; pre-send authentication failure with no durable
Prepared record or socket write; and raw, decoded, and derived secret-canary
scans over the public capture, authenticated journal, and Goal F journal.

Focused auth/vector, paired-journal recovery, durability-before-real-TCP-write,
single-send ambiguity, exact-owned cancellation, read-supervision, and
controlled-shutdown tests were also exercised during implementation. The
aggregate feature and workspace results below cover the final source tree.

## Final Acceptance Evidence

The feature acceptance gates were green:

```text
cargo clippy -p reap-polymarket-live-adapter -p reap-pm-live --all-targets --features reap-pm-live/loopback-evidence,reap-polymarket-live-adapter/loopback-evidence --locked -- -D warnings
# green (exit 0)

cargo test -p reap-pm-live --features loopback-evidence --locked
# green (exit 0)

cargo test -p reap-polymarket-live-adapter --features loopback-evidence --locked
# green (exit 0)
```

The standard workspace gates were green on the final source tree:

```text
cargo fmt --all -- --check
# green (exit 0)

cargo clippy --workspace --all-targets --locked -- -D warnings
# green (exit 0)

cargo test --workspace --locked --no-fail-fast
# green (exit 0)

cargo build --release --workspace --locked
# green (exit 0)

git diff --check
# green (exit 0; rerun after documentation closeout)
```

## Phase Commit Ledger And Historical Exception

- Baseline before PM-T1: `4a0ae61487b2d8d8996b797a2dd239dc877bc219`.
- Accepted PM-T1 implementation snapshot:
  `1c82ad8556923ef6096002b8078a7bf9334345cd`
  (`feat(pm): implement PM-T1 authenticated connectivity`). This atomic commit
  contains 265 non-documentation paths: the complete implementation, schemas,
  fixtures, and executable acceptance evidence.
- Documentation/ledger closeout:
  `b8e4f8801d37cb069de9a6e2d15a4d9a517ee9ad`
  (`docs(pm): record PM-T1 acceptance ledger exception`). The implementation
  and documentation commits were published to `origin/master` on 2026-08-09.

The phase work was performed and validated in one shared working tree. A final
Git audit found no recoverable per-phase refs, reflog checkpoints, stashes, or
dangling commits. Retroactively slicing overlapping files and hunks into seven
apparently historical phase commits would invent intermediate states and would
require a separate validation campaign. On 2026-08-08 the user explicitly
authorized the recommended two-commit closeout and this truthful ledger
exception instead: one atomic accepted implementation snapshot followed by one
documentation/ledger commit. No phase hash is inferred or fabricated.

## Next Authorization Gate

With PM-T1's local acceptance and standard gates green, the next step remains a
separately scoped, explicitly authorized credentialed read-only smoke test. A
minimal-capital place/cancel smoke is a second, later authorization after
read-only identity, account, allowance, position, private-stream, and
reconciliation evidence is accepted.
