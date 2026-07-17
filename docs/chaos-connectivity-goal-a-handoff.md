# Chaos Connectivity Goal A Handoff

Status: Goal A complete at implementation commit
`ab7842446b9cb4f48ccc70425b0c8731ac9eac5f`; Goal B and the
production-readiness gate remain pending.

Verified: 2026-07-17.

This is the handoff for Phases 0–5 and the Tranche A gate in
[chaos-connectivity-refactor-plan.md](chaos-connectivity-refactor-plan.md).
The normative authority contract is
[chaos-connectivity-boundary.md](chaos-connectivity-boundary.md), and the
before/after capability record is
[chaos-connectivity-inventory.md](chaos-connectivity-inventory.md).

Goal A constrains the live Chaos surface; it does not approve a credentialed
demo campaign, expose production order entry, claim target-host readiness, or
complete the Phase 6–9 structural decomposition.

## Reference And Scope

| Item | Verified state |
| --- | --- |
| Reap implementation commit | `ab7842446b9cb4f48ccc70425b0c8731ac9eac5f` |
| Goal A starting documentation commit | `418b06eb176b121cb8410c6d407427116277e717` |
| Sibling behavior reference | clean `../imm-strategy` checkout at `b6b120c7b7c466d8431bf082f3229328c5d7b2ae` |
| Rust reference pin | `reap_core::PINNED_JAVA_REVISION` equals the same full SHA |
| Java scope | `chaos/chaos-core`, `chaos/chaos-iarb2`, and supporting code transitively reached by the supported Chaos/iarb2 path |
| Excluded parity authority | generic gateway/`ExecAlgo` features, unrelated strategies/venues, and Java's eight-session regular command-pool cardinality |
| Exchange access during Goal A | no exchange credential supplied or used and no authenticated exchange request; `cargo audit` fetched only the public RustSec advisory database and crates.io index |

The final implementation has these authority properties:

- Validate resolves a secret-free plan without constructing credentials or
  network roles.
- Observe constructs the planned public/private observation and authenticated
  read roles, including the forbidden-order observer, but no command lane,
  regular mutation, or Cancel All After role.
- Demo constructs exactly one nonempty command lane for each executing account;
  reference-only accounts receive no mutation lane. Legacy connection-count
  fields are maximum migration caps, not cardinality requests.
- Chaos can place only policy-validated regular Quote and Hedge orders and can
  cancel only proven owned regular orders. It cannot reach amend, arbitrary
  signing/requests, algo/spread mutation, or emergency authority.
- Every account has one packed private-state session and exact plan-derived
  public subscriptions. Books have two named recovery replicas; trades and
  configured reference inputs have one.
- The read-only sentinel starts bounded tagged scans at the fixed half-age
  cadence even when an older scan hangs or its output channel is full. Algo and
  spread retain independent pacers and 60-second domain bounds. Failure or
  expiry invalidates every already-started zero generation, so stale or
  out-of-order completion cannot re-arm readiness.
- A forbidden-state fault is recorded per account, but the current live gate is
  global: it leaves `Ready` and blocks all new placement. Reconciliation and,
  in Demo, owned-regular cancellation target the affected account. The typed
  runtime attempts to queue the critical event only when the alert sink is
  enabled; successful external delivery remains an operational gate.
- Emergency mutation lives in separate core, runner, and OKX adapter crates.
  Regular mitigation starts first; regular, algo, and spread own independent
  pacing, progress, incidents, and zero proof while independently enforcing the
  shared absolute per-account deadline. Report merge order is
  regular/algo/spread and `all_clear` remains conjunctive.

## Phase Commits

| Phase | Commit | Result |
| --- | --- | --- |
| Boundary baseline | `418b06eb176b121cb8410c6d407427116277e717` | Normative boundary and goal-ready plan |
| Phase 0 | `36cdca3354016c39c5d9688fd2366a0c1f766a5e` | Frozen capability inventory and registry |
| Phase 1 | `e3aaad09c06a7d74707f319f317e67430c0f2d52` | Deterministic `ChaosConnectivityPlan` |
| Phase 2 | `a78e316864504c527dda081c715931d4d2bd40c0` | Typed Quote/Hedge/CancelOwned execution boundary |
| Phase 3 | `ae2ff4ec0baa7423b2339e102a57b5a3fe5b01ef` | Private wire layer and role-specific OKX authority |
| Phase 4 | `97b4c75b51f66927c278094d062b37b17a60762b` | Plan-derived sockets, lanes, readiness, and maintenance |
| Phase 5a | `ffe5f62424a84a0db9b7951807856421f0b0f050` | Read-only forbidden-order zero proof |
| Phase 5b | `2cfc7c96c5cac8f86725537792e24a972e09140e` | Independent emergency-domain progress |
| Phase 5c | `ab7842446b9cb4f48ccc70425b0c8731ac9eac5f` | Fixed half-age scan cadence and out-of-order safety |

The documentation-only commit containing this file is the final Tranche A
record.

## Deterministic Anchors

| Artifact | SHA-256 |
| --- | --- |
| `Cargo.lock` | `74ca0a2b8fd028250cc243832ee7b169dc21ba26e3cf49713add4c7ff8cea213` |
| `fixtures/normalized/chaos_quote_hedge.jsonl` | `27f2eb4b9dba7ee600ed645ad8b7c88143e8b54531232991b492cb7595e8ccaa` |
| `fixtures/normalized/chaos_quote_hedge_later.jsonl` | `40453b8be283178b20531c84142dbaeeeca82b4723e5c13594df171c778cd8ee` |
| `fixtures/normalized/chaos_quote_hedge_intents.json` | `d95fa7f121e2e8c402c8108cf9fefb7c7d7b3dbd2b9742c58c234a521f0ee0ec` |
| `examples/iarb2-basic.toml` | `0fac5a3a35fe28cdc05118b7e22241077aa7f604a9a5436355797605b51b3b26` |
| Canonical sample Demo plan | `6771c97a373f12f77093624ea4b2914d867aae6a710eddadde925fc288fc6477` |
| Pretty CLI backtest output, two byte-identical runs | `38acf9f5e0c310f2ec5528974beffadf4c1a7f84d46efa8d9664ee7051e84691` |

The exact strategy and backtest fixtures still produce the same ordered
Quote/Hedge intents. Journal schema 7, emergency report schema 2, live report
and evidence formats, serialized order intents, and checked fixtures are
unchanged. The only connectivity-config compatibility change is the documented
Phase 4 interpretation of legacy connection counts as caps.

## Tranche A Verification

All final commands below ran from
`ab7842446b9cb4f48ccc70425b0c8731ac9eac5f` with only the documentation handoff
changes present in the worktree.

| Command | Final result |
| --- | --- |
| `cargo fmt --all -- --check` | exit `0` |
| `TMPDIR=/home/ubuntu/code/reap/target/tmp cargo clippy --workspace --all-targets --locked -- -D warnings` | exit `0` |
| `TMPDIR=/home/ubuntu/code/reap/target/tmp cargo test --workspace --locked --no-fail-fast --quiet` | exit `0`; all workspace unit, integration, role-visibility, dependency-policy, and doc tests passed |
| `TMPDIR=/home/ubuntu/code/reap/target/tmp cargo build --release --workspace --locked` | exit `0` |
| `TMPDIR=/home/ubuntu/code/reap/target/tmp deploy/systemd/verify-units.sh target/release/reap` | exit `0`; observe, demo, and capture exposure each `2.9 OK` |
| `TMPDIR=/home/ubuntu/code/reap/target/tmp cargo audit --deny warnings` | exit `0`; 1,160 advisories loaded and 248 lockfile dependencies scanned |
| `cargo metadata --locked --format-version 1 >/dev/null` | exit `0` |
| `git diff --check` | exit `0` |

An earlier unit-verifier attempt used the small `/tmp` tmpfs and failed while
copying its temporary systemd tree. No Reap unit failed. The final command above
uses the repository build temporary directory and passed all three unit
profiles.

Focused boundary results:

| Check | Result |
| --- | --- |
| `cargo test -p reap-live --locked --no-fail-fast` | 292 library tests, the live authority compile-fail fixture, and the dependency-policy test passed |
| `cargo test -p reap-live --locked forbidden_orders::tests -- --nocapture` | 15 sentinel tests passed, including fixed cadence under a hung scan and full output channel, expiry, strict pagination, fast peer failure, and both out-of-order completion directions |
| Venue/live/strategy role-visibility fixtures | all eight compile-fail source fixtures passed |
| Emergency/evidence/live adapter `allowlist` filters | 2 emergency, 1 evidence, and 3 live allowlist tests passed |
| `cargo test -p reap-live --locked config::tests` | 35 config and migration tests passed |
| Exact strategy fixture | 1 passed |
| Exact backtest fixture | 1 passed |
| Exact sample Demo-plan hash test | 1 passed |
| Exact forbidden-observer request test | 1 passed |
| Exact capability-registry/inventory test | 1 passed |
| `cargo tree -p reap-live --locked -e normal` emergency-edge check | no emergency core, runner, or OKX emergency adapter in the normal live graph |
| `git -C ../imm-strategy status --porcelain=v1` | no output |

## Explicit Deferrals

Goal A does not complete or claim:

- Phase 6 removal of the normal `reap-backtest -> reap-live` dependency. The
  edge is still present and is `D09` in the inventory.
- Phase 7–9 responsibility-based module decomposition and the final Goal B
  global acceptance gate.
- Credentialed target-account evidence, target-host soak/fault campaigns,
  enabled external-alert delivery evidence, or production deployment.
- Production order entry, which remains unavailable regardless of these test
  results.

An overlapping production-readiness writer should rebase onto the Goal A
implementation and documentation commits and re-audit its assumptions against
the narrower role graph. Keep that writer paused if Goal B will run next; do not
continue production-readiness changes from the pre-refactor authority surface.
