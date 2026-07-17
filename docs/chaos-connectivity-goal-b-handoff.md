# Chaos Connectivity Goal B Handoff

Status: conditional Phase 6–9 handoff. Goal B is **not complete** while any
`[PENDING_*]` marker remains or any required gate is not green. This document
does not approve demo trading, production order entry, a credentialed campaign,
or target-host deployment.

Prepared: 2026-07-17.

This is the verification record for Goal B in
[chaos-connectivity-refactor-plan.md](chaos-connectivity-refactor-plan.md).
The normative capability contract is
[chaos-connectivity-boundary.md](chaos-connectivity-boundary.md), the frozen
before/after surface is
[chaos-connectivity-inventory.md](chaos-connectivity-inventory.md), and the
historical execution contract is
[chaos-connectivity-goal-b-prompt.md](chaos-connectivity-goal-b-prompt.md).
Goal B starts from the completed
[Goal A handoff](chaos-connectivity-goal-a-handoff.md) and preserves its
authority result.

## Completion Rule

Replace a placeholder only with directly observed evidence from the committed
documentation/verification base or, for final documentation-only checks, the
exact staged handoff-result tree. Goal B is complete only when:

- all Phase 6–8 focused gates and every Phase 9 command below pass;
- all commit, tree, patch, deterministic-hash, and clean-worktree fields are
  exact;
- Reap and `../imm-strategy` are clean at the recorded revisions;
- no credential or authenticated exchange request was used;
- no journal, report, evidence, configuration, or order-intent schema changed
  beyond the already documented backward-compatible Goal A migration; and
- the final result still says structural completion only, not trading
  readiness.

If a required result is red, retain the placeholder or record the exact
failure. Do not describe the goal as complete.

## Reference And Scope

| Item | Recorded state |
| --- | --- |
| Goal A boundary baseline | `418b06eb176b121cb8410c6d407427116277e717` |
| Goal A final implementation | `ab7842446b9cb4f48ccc70425b0c8731ac9eac5f` |
| Goal A documentation/handoff | `1fbf8955097fdb29fc38b04866005aa1f7095bee` |
| Goal B prompt/starting `HEAD` | `21d20e288c7de9e038550666fbb1f1d95763912a` |
| Final Goal B implementation tip | `246f5b21d046dc20fd84460c7b59346231d6107f` |
| Goal B documentation/verification-base commit | `[PENDING_GOAL_B_DOCS_BASE_SHA]` |
| Sibling behavior reference | clean `../imm-strategy` checkout at `b6b120c7b7c466d8431bf082f3229328c5d7b2ae` |
| Rust reference pin | `reap_core::PINNED_JAVA_REVISION` equals `b6b120c7b7c466d8431bf082f3229328c5d7b2ae` |
| Supported Java scope | Only `chaos/chaos-core`, `chaos/chaos-iarb2`, and supporting code transitively reached by the supported Chaos/iarb2 path |
| Excluded parity authority | Generic gateway/`ExecAlgo` features, unrelated strategies/venues, and Java's eight-session command-pool cardinality |
| Exchange access during Goal B | `[PENDING_CONFIRM_NO_CREDENTIALS_OR_AUTHENTICATED_EXCHANGE_REQUESTS]` |

The current Chaos strategy has exactly three executable purposes:

1. regular PostOnly, non-reduce-only Quote with no per-order STP and a verified
   account `acctStpMode = cancel_maker`;
2. regular IOC, non-reduce-only, `CancelMaker` Hedge; and
3. cancellation of one canonical owned regular order.

Goal B does not add amend/batch amend, any other regular profile, algo/spread
placement, margin-spot borrowing, master/group feeds, another venue, a generic
strategy framework, or production order entry.

## Phase Commits

| Phase/family | Commit | Result |
| --- | --- | --- |
| Goal B execution prompt | `21d20e288c7de9e038550666fbb1f1d95763912a` | Starting contract |
| Phase 6 pacing and OKX keys | `0915bdb857e3b7dfe5c922832a9cd438b78f1c30` | Pure lower contracts |
| Phase 6 host policy | `64b2d554b07e5a2dc107835aed849e691efd97c8` | Pure host assessment |
| Phase 6 live config/plan | `2ea108fd0e39dfc2b23159cc72823634d5735597` | Pure live contracts |
| Phase 6 account certification | `51ea2b0b017f3a6777c244c285c753c3627299e0` | Pure evidence contract |
| Phase 6 dependency inversion | `b6eb537b06a5e6f5a5a7d3fdc28c7969f9e8c09b` | Normal backtest-to-live edge removed |
| Phase 7 capture split | `9eef036499b48a31d3daa985637718786e46a11f` | Configuration/runtime/writer split |
| Phase 7 live split | `aad0630a1877ddf80eb3baac135176aa7a73a087` | Runtime responsibility modules |
| Phase 7 research split | `ad3e9b707485b5341230fb05bfb6674491aae00a` | Research responsibility modules |
| Phase 7 research ownership | `cc88a1590af061b6700decebb4b03585a57663c4` | Narrow research surface |
| Phase 7 capture ownership | `4951e70a0efb1c5ec8c4f80851444066a2c0c8cc` | Narrow capture surface |
| Phase 7 live subsystem state | `1d2b0f8b1ea5c1f9d58bce1869e6bca0bde0ce1c` | Explicit ordered coordinator state |
| Phase 8 host/capture decisions | `dd0d5db9cb4ee93a225e53361b18e5cb51a06996` | Shared pure decisions |
| Phase 8 live safety decisions | `2f172e8b6ce03f73505c79deb80330bccf59aef8` | Shared soak/fault decisions |
| Phase 8 regular authority/recovery | `38babe6e4d12d598730d3c79aeeccbbec1ec018d` | Linear gateway-bound mutation authority |
| Phase 8 adapter command websocket | `246f5b21d046dc20fd84460c7b59346231d6107f` | Adapter-owned single command session, sealed take-once authority, exact acknowledgement/fallback boundaries, and teardown release regression committed; final gates below remain pending |
| Phase 9 documentation/verification base | `[PENDING_GOAL_B_DOCS_BASE_SHA]` | README, capability inventory/deviation ledger, architecture, mapping, operations, readiness, prompt, CLI help, sample config, and handoff record before result fields are filled |

Immutable identities for every committed Phase 6–8 family:

| Commit | Tree | Stable patch ID |
| --- | --- | --- |
| `0915bdb857e3b7dfe5c922832a9cd438b78f1c30` | `d46496cdc6f7812422a23652f142c7425ede6d19` | `9347a6f83c1d0c5b78ca9c03f9e554e8d5fb340a` |
| `64b2d554b07e5a2dc107835aed849e691efd97c8` | `b179fe539ae674bf3b16fd7582f4dba30489939b` | `0c0f906591a1478c742e134e4eb3f87bde16bcf4` |
| `2ea108fd0e39dfc2b23159cc72823634d5735597` | `3b08ece1de08fb4a7a1a52ca0b2ae2975f8827d3` | `46fefc1e5d83beb0e357e8f96ea58c126ba6e58b` |
| `51ea2b0b017f3a6777c244c285c753c3627299e0` | `2fd2e85b9ac4269cd690d042002df40a63546b2a` | `a0e1fe45ffba60ef62fbad65cac3ec35f0da95e7` |
| `b6eb537b06a5e6f5a5a7d3fdc28c7969f9e8c09b` | `a6523033f23e9f84d7797468b02dacc1f586a91c` | `b3f5484c92d9f060345b1cc3e9ee5cdf1c80543d` |
| `9eef036499b48a31d3daa985637718786e46a11f` | `eb417f7e0400f2c0e69ce4968183fbc3851d253c` | `2d5f9bbd695e55fe95ac50633b3f72b97f2bccf7` |
| `aad0630a1877ddf80eb3baac135176aa7a73a087` | `6bcc026f00923f5d1ee46e5622fa3029aeca08dd` | `d736b0e606c88052c6f599531d86411196c0ad32` |
| `ad3e9b707485b5341230fb05bfb6674491aae00a` | `0a6b60c872a0a23540352ca8f80e3ed8f8c032bd` | `edd078524245adac65dfae3a5382b0a637e40adb` |
| `cc88a1590af061b6700decebb4b03585a57663c4` | `0d6565e7df2fac0bf70c709801a05ba13f862648` | `9a368b706dd0374bf1db54e17353cad22954c771` |
| `4951e70a0efb1c5ec8c4f80851444066a2c0c8cc` | `501454bd06f4f28043799499babdeac7afbcf168` | `031f79e58ca9406892de542acafcb06aec12132f` |
| `1d2b0f8b1ea5c1f9d58bce1869e6bca0bde0ce1c` | `b3be8cc0c94de56c14f69ef14d70c288544c77b3` | `ec050e207aee284d444c73dd21e00b90a946628c` |
| `dd0d5db9cb4ee93a225e53361b18e5cb51a06996` | `967481ee852daba2d2199d70800747ca6ad00fff` | `ecb6582b8d3b3bba6e685283600806186a2bded8` |
| `2f172e8b6ce03f73505c79deb80330bccf59aef8` | `7eb2c3c3d9f12adaeb5d261755ea254bde3dccd8` | `9ba5e0e0c2f5ac6e8bb30a3b2a8e514ba671fa24` |
| `38babe6e4d12d598730d3c79aeeccbbec1ec018d` | `92fdc4fe4224a073bc489c99c3f5a63f16d2166d` | `9ed5315dfb8d7825f34405d520044e5987b7b8c2` |

Final adapter and documentation identity:

| Family | Commit | Tree | Stable patch ID |
| --- | --- | --- | --- |
| Adapter-owned command websocket | `246f5b21d046dc20fd84460c7b59346231d6107f` | `8da0cee31acc15f64a32c75e70808279d840537a` | `ab22b63351b3a17a44da360ed686289ad0d58159` |
| Goal B documentation/verification base | `[PENDING_GOAL_B_DOCS_BASE_SHA]` | `[PENDING_GOAL_B_DOCS_BASE_TREE_SHA]` | `[PENDING_GOAL_B_DOCS_BASE_PATCH_ID]` |

The later commit that replaces the final verification placeholders is the
handoff-result commit. It is intentionally not self-referenced in this file;
identify it from Git history.

The Phase 6 launch-tree inventory was taken directly from
`git show 21d20e288c7de9e038550666fbb1f1d95763912a:crates/reap-backtest/src/research.rs`.
The normal backtest edge reached `reap-live` only for
`AccountCertificationArtifact`,
`ACCOUNT_CERTIFICATION_SCHEMA_VERSION`,
`verify_account_certification_artifact_path`, `LiveConfig`,
`OkxTradeModeConfig`, and `TradingEnvironment`. Phase 6 lowered precisely those
pure artifact/verifier/configuration families and then removed the normal
dependency; it did not move runtime, network, credential, or emergency
authority into a shared crate.

## Final Structural Properties

These claims become final only when their checks below pass:

- Pure live configuration, connectivity, account-certification, pacing, key,
  and host-policy contracts sit below the runtime.
  `reap-backtest` normally depends on `reap-live-contracts`, not `reap-live`;
  shared contract crates contain no networking, credential execution, host
  inspection, task ownership, or emergency authority.
- Live composition, connectivity, dispatch, readiness/safety,
  reconciliation, and shutdown are separate responsibilities while one
  `LiveRuntime`/`LiveCoordinator` remains the ordered writer of strategy, risk,
  and canonical order state. Research and capture retain their deterministic
  scheduler and writer owners.
- `HostHealthThresholdAssessment`,
  `CaptureCleanRunInputs`/`capture_run_is_clean`,
  `LiveCleanSoakInputs`, `LiveFaultFailureCode`, and
  `LiveFaultFailureClass` each own one repeated pure decision.
- `RegularExecutionPolicy` alone consumes the gateway-bound policy role to
  issue a non-Clone, take-once `ApprovedRegular*`. For submit, a gateway-bound
  `GeneratedClientOrderId` and `ApprovedRegularSubmit` are consumed by
  `OwnedRegularOrders::reserve_local`, which creates canonical `PendingNew`
  ownership and one `ReservedRegularSubmit`. `OkxOrderGateway` validates and
  consumes that reservation to create `PreparedRegularSubmit`. An owned
  `ApprovedRegularCancel` goes directly through the gateway to
  `PreparedRegularCancel`. The dispatcher reserves pacing before adapter IO.
- `reap-okx-live-adapter` alone owns command websocket connect, login, write,
  acknowledgement correlation, reconnect, shutdown, and prepared-to-private
  OKX DTO/JSON conversion for normal live. It consumes an account-bound
  nonseparable bundle. Consuming startup validates the supplied destination
  and account, installs the private matching slot before spawn, and returns the
  now-bound gateway. Besides that gateway, only typed lifecycle/status
  observation is returned. There is no transport getter or late-install hook.
- The current normative plan creates exactly one command shard per executing
  account. The pinned Java eight-session pool is reference-only.
- Each account's private-state session authority is non-Clone and taken once.
  Its consuming factory binds reconnect-capable bootstrap to the exact private
  destination, account, connection identity, and complete packed subscription
  set in the resolved plan. A count other than one, a duplicate identity, or a
  split/substituted plan is rejected. The admitted channel set remains
  plan-minimal: positions alone for an unused observation-only account, or
  account/orders/positions plus configured fills for an executing account.
- Mismatched nonempty acknowledgement account/symbol/client ID and an accepted
  zero exchange-order ID are ambiguous/fail-closed. An empty or `"0"` client ID
  may normalize only to the already expected pending identity.
- Emergency and evidence authorities remain in separate composition roots;
  normal live cannot reach emergency mutation, raw signing, transport, login
  construction, arbitrary authenticated requests, amend, or algo/spread
  placement.
- No serialized journal/report/evidence/configuration/order-intent schema
  changed during Goal B.

### Journal Recovery Authority

Normal mutation ownership arises only from a synchronous gateway-bound local
reservation or the exact canonical journal under its exclusive lease. Private
updates, reconciliation rows, client-ID prefixes, reasons, and ordinary
parsers remain evidence and cannot mint a proof.

Only one-shot `recover_leased_jsonl(&mut StorageLease)` retains non-Clone recovery proofs from the exact canonical journal; ordinary path/byte recovery strips them. Proofs are consumed and rebound to the current gateway scope. This is a structural authority boundary rooted in an exclusively leased, operator-controlled journal, not cryptographic authentication of disk contents.

At startup one-shot leased recovery retains proofs without consuming or
rebinding them. Authenticated bootstrap then creates the current gateway
scopes; the coordinator consumes and rebinds the proofs while restoring
canonical orders. Only after that restore does the same continuously held
lease transfer to the sole storage writer. The lease excludes cooperating
users/processes that honor the same canonical lock and protects against path
aliases. It does not cryptographically authenticate disk contents and cannot
prevent a noncooperating same-user writer, path substitution, or host/process
compromise. The journal and its parent remain operator-controlled security
state.

## Deterministic Anchors

Do not copy forward a prior value for a mutable final artifact. Recompute every
`[PENDING_*]` hash from the final committed tree or exact final command output.

| Artifact | Final SHA-256 |
| --- | --- |
| `Cargo.lock` | `[PENDING_FINAL_CARGO_LOCK_SHA256]` |
| `fixtures/normalized/chaos_quote_hedge.jsonl` | `27f2eb4b9dba7ee600ed645ad8b7c88143e8b54531232991b492cb7595e8ccaa` |
| `fixtures/normalized/chaos_quote_hedge_later.jsonl` | `40453b8be283178b20531c84142dbaeeeca82b4723e5c13594df171c778cd8ee` |
| `fixtures/normalized/chaos_quote_hedge_intents.json` | `d95fa7f121e2e8c402c8108cf9fefb7c7d7b3dbd2b9742c58c234a521f0ee0ec` |
| `examples/iarb2-basic.toml` | `0fac5a3a35fe28cdc05118b7e22241077aa7f604a9a5436355797605b51b3b26` |
| Canonical sample Demo plan | `[PENDING_FINAL_DEMO_PLAN_SHA256]` |
| `examples/live-okx-demo.toml` raw bytes | `[PENDING_FINAL_DEMO_CONFIG_SHA256]` |
| Pretty CLI backtest output, two byte-identical runs | `[PENDING_FINAL_BACKTEST_OUTPUT_SHA256]` |
| Account-certification fixed-artifact result | `[PENDING_FINAL_ACCOUNT_CERT_FIXED_ARTIFACT_SHA256]` |
| Account identity result | `[PENDING_FINAL_ACCOUNT_IDENTITY_SHA256]` |
| Locked release executable `target/release/reap` | `[PENDING_FINAL_RELEASE_SHA256]` |
| Rebuilt root CLI help | `[PENDING_FINAL_ROOT_HELP_SHA256]` |
| Rebuilt `live` CLI help | `[PENDING_FINAL_LIVE_HELP_SHA256]` |

Record the exact command used for each derived hash:

| Derived artifact | Command |
| --- | --- |
| Demo plan | `[PENDING_DEMO_PLAN_HASH_COMMAND]` |
| Demo config source | `sha256sum examples/live-okx-demo.toml` |
| Backtest output | `[PENDING_BACKTEST_HASH_COMMAND]` |
| Account-certification fixed artifact/identity | `[PENDING_ACCOUNT_ARTIFACT_HASH_COMMAND]` |
| Release executable and help | `sha256sum target/release/reap target/tmp/reap-help.txt target/tmp/reap-live-help.txt` after the locked release build |

## Focused Verification

The authority commit was observed green before the adapter ownership move:

| Package/check | Pre-adapter observed result |
| --- | --- |
| `reap-storage` | 24 unit/integration tests plus docs passed |
| `reap-order` | 59 tests, 5 compile-fail cases, and docs passed |
| `reap-feed` | 66 tests, 2 compile-fail cases, and docs passed |
| `reap-strategy` | 74 tests, 2 compile-fail cases, and docs passed |
| `reap-live` | 238 tests, 2 compile-fail cases, 3 dependency/source checks, config compatibility, and docs passed |
| `reap-okx-live-adapter` | 13 tests, 3 compile-fail cases, and docs passed |

Those observations do not replace final verification after the adapter move.
Record the exact final commands/counts here:

The final tree also re-proves the earlier responsibility phases explicitly:

| Phase | Exact final command | Final result |
| --- | --- | --- |
| Phase 6 pure contracts and consumers | `TMPDIR=/home/ubuntu/code/reap/target/tmp cargo test -p reap-core -p reap-venue -p reap-feed -p reap-order -p reap-telemetry -p reap-live-contracts -p reap-backtest --locked --no-fail-fast && TMPDIR=/home/ubuntu/code/reap/target/tmp cargo test -p reap-live --test dependency_policy --locked authenticated_okx_authority_obeys_the_workspace_dependency_policy -- --exact` | `[PENDING_PHASE6_FINAL_RESULT]` |
| Phase 7 capture/research/live owners | `TMPDIR=/home/ubuntu/code/reap/target/tmp cargo test -p reap-capture -p reap-backtest -p reap-live --locked` | `[PENDING_PHASE7_FINAL_RESULT]` |
| Phase 8 shared safety decisions | `TMPDIR=/home/ubuntu/code/reap/target/tmp cargo test -p reap-core -p reap-capture -p reap-live-contracts -p reap-live --locked` | `[PENDING_PHASE8_SAFETY_RESULT]` |

| Required focused gate | Final command | Final result |
| --- | --- | --- |
| Order authority, leased recovery, and compile-fail boundaries | `[PENDING_ORDER_AUTHORITY_COMMAND]` | `[PENDING_ORDER_AUTHORITY_RESULT]` |
| Adapter command lifecycle, routing, identity, ambiguity, and compile-fail boundaries | `[PENDING_ADAPTER_COMMAND]` | `[PENDING_ADAPTER_RESULT]` |
| Live runtime integration, source/dependency guards, and configuration compatibility | `[PENDING_LIVE_COMMAND]` | `[PENDING_LIVE_RESULT]` |
| Feed pacer and private-bootstrap boundaries | `[PENDING_FEED_COMMAND]` | `[PENDING_FEED_RESULT]` |
| Strategy role visibility and exact Java-parity fixture | `[PENDING_STRATEGY_COMMAND]` | `[PENDING_STRATEGY_RESULT]` |
| Exact backtest snapshot and two-run byte identity | `[PENDING_BACKTEST_COMMAND]` | `[PENDING_BACKTEST_RESULT]` |
| Exact sample Demo-plan hash and endpoint/channel allowlist | `[PENDING_PLAN_ALLOWLIST_COMMAND]` | `[PENDING_PLAN_ALLOWLIST_RESULT]` |
| Configuration migration compatibility | `[PENDING_CONFIG_COMMAND]` | `[PENDING_CONFIG_RESULT]` |
| `reap-backtest` normal graph excludes `reap-live` | `[PENDING_BACKTEST_TREE_COMMAND]` | `[PENDING_BACKTEST_TREE_RESULT]` |
| Shared contracts exclude network/credential dependencies | `[PENDING_CONTRACT_TREE_COMMAND]` | `[PENDING_CONTRACT_TREE_RESULT]` |
| `reap-live` excludes raw command websocket and emergency authority | `[PENDING_LIVE_NEGATIVE_COMMAND]` | `[PENDING_LIVE_NEGATIVE_RESULT]` |
| Adapter is sole normal-live prepared-regular command DTO/wire owner and exposes no transport/signer | `[PENDING_ADAPTER_SOURCE_GUARD_COMMAND]` | `[PENDING_ADAPTER_SOURCE_GUARD_RESULT]` |
| Serialized config, journal/report/evidence, and order-intent compatibility | `[PENDING_SCHEMA_COMPAT_COMMAND]` | `[PENDING_SCHEMA_COMPAT_RESULT]` |
| Emergency workflows remain independent and cancel-only; forbidden sentinel remains read-only/fail-closed | `[PENDING_EMERGENCY_SENTINEL_COMMAND]` | `[PENDING_EMERGENCY_SENTINEL_RESULT]` |
| Baseline-to-final capability/schema diff review | `[PENDING_BASELINE_DIFF_COMMAND]` | `[PENDING_BASELINE_DIFF_RESULT]` |
| Release CLI root/live help states the bounded Demo/safety/emergency split | `[PENDING_CLI_HELP_COMMAND]` | `[PENDING_CLI_HELP_RESULT]` |
| Raw sample Demo config hash | `sha256sum examples/live-okx-demo.toml` | `[PENDING_DEMO_CONFIG_HASH_RESULT]` |
| `../imm-strategy` full SHA, Rust pin, evidence bindings, and clean tree | `[PENDING_JAVA_PIN_COMMAND]` | `[PENDING_JAVA_PIN_RESULT]` |

### Full-Plan Acceptance Crosswalk

| Acceptance cluster | Final evidence above |
| --- | --- |
| Pinned Java identity, ordered parity, exact Quote/Hedge profiles, and CancelOwned proof rejection | Java-pin, strategy, order-authority, and deterministic backtest rows |
| Stable requirement/consumer IDs; closed Validate/Observe/Demo role sets; plan-minimal trades, replicas, private sockets, and command lane | Demo-plan/allowlist, configuration, feed, and live-negative rows |
| Raw signing/wire isolation and absence of amend, algo/spread placement, and emergency authority from normal live | Adapter source guard, live-negative, compile-fail, and dependency rows |
| Read-only fail-closed forbidden sentinel and independent cancel-only emergency domains with conjunctive `all_clear` | Emergency/sentinel row |
| Pure-contract dependency inversion, explicit single writer, and serialized compatibility | Phase 6, Phase 7, contract-tree, schema-compatibility, and baseline-diff rows |
| Repository-wide green state and no new readiness feature, credentialed evidence, venue, strategy, or order capability | Phase 9 global table, baseline-diff row, no-auth audit, and explicit deferrals |

## Phase 9 Global Verification

The build, test, release, audit, and metadata commands run from the committed
documentation/verification base, which contains the final implementation plus
the intended documentation, CLI-help, and sample-config changes. Run
`git diff --check` both on that base and again on the exact staged
handoff-result tree. The no-placeholder scan can pass only on that staged
result tree after every directly observed result is recorded. Use the
repository-local temporary directory exactly as shown.

| Command | Final result |
| --- | --- |
| `mkdir -p /home/ubuntu/code/reap/target/tmp` | `[PENDING_MKDIR_RESULT]` |
| `cargo fmt --all -- --check` | `[PENDING_FMT_RESULT]` |
| `TMPDIR=/home/ubuntu/code/reap/target/tmp cargo clippy --workspace --all-targets --locked -- -D warnings` | `[PENDING_CLIPPY_RESULT]` |
| `TMPDIR=/home/ubuntu/code/reap/target/tmp cargo test --workspace --locked --no-fail-fast` | `[PENDING_WORKSPACE_TEST_RESULT]` |
| `TMPDIR=/home/ubuntu/code/reap/target/tmp cargo build --release --workspace --locked` | `[PENDING_RELEASE_BUILD_RESULT]` |
| `TMPDIR=/home/ubuntu/code/reap/target/tmp deploy/systemd/verify-units.sh target/release/reap` | `[PENDING_SYSTEMD_VERIFY_RESULT]` |
| `TMPDIR=/home/ubuntu/code/reap/target/tmp cargo audit --deny warnings` | `[PENDING_CARGO_AUDIT_RESULT]` |
| `cargo metadata --locked --format-version 1 >/dev/null` | `[PENDING_LOCKED_METADATA_RESULT]` |
| `git diff --check` | `[PENDING_DIFF_CHECK_RESULT]` |
| `placeholder_rx='\[''PENDING_[A-Z0-9_]+\]'; ! rg -n --hidden --glob '!.git/**' "$placeholder_rx" README.md Cargo.toml Cargo.lock crates docs examples deploy` | `[PENDING_NO_PLACEHOLDER_RESULT]` |

## Verification-Base Repository State

The documentation/handoff result commit necessarily changes `HEAD`, so this
table records the implementation-and-verification base observed before that
result commit. The post-result-commit `HEAD` and clean status belong in the
final goal report and Git history rather than a self-reference here.

| Check | Recorded result |
| --- | --- |
| Reap implementation/verification-base status | `[PENDING_REAP_VERIFICATION_BASE_STATUS]` |
| Reap implementation/verification-base `HEAD` | `[PENDING_REAP_VERIFICATION_BASE_HEAD]` |
| `git -C ../imm-strategy status --porcelain=v1` | `[PENDING_IMM_CLEAN_STATUS]` |
| `git -C ../imm-strategy rev-parse HEAD` | `[PENDING_IMM_HEAD]` |
| Rust pin/evidence-binding scan | `[PENDING_PIN_BINDING_RESULT]` |
| Credential/authenticated-exchange audit | `[PENDING_NO_CREDENTIAL_OR_AUTH_CALL_CONFIRMATION]` (local build/test commands and public Cargo/advisory network access only; no credential source or authenticated exchange endpoint was invoked) |

## Explicit Deferrals

Even after every placeholder is replaced by green evidence, Goal B does not
complete or claim:

- a credentialed target-account observe/demo soak or fault campaign;
- sustained target-host capture, latency calibration, systemd installation, or
  external alert delivery evidence;
- exchange certification, fee/account/economic evidence, process-death
  deadman evidence, or production evidence bundle approval;
- demo trading approval or production rollout governance;
- production order entry, which remains unavailable;
- amend/batch amend, new regular order profiles, algo/spread placement,
  margin-spot borrowing, master/group feeds, another venue, or a generic
  strategy/plugin framework; or
- regeneration or relabeling of production evidence;
- a broad numeric/domain-newtype migration;
- rerouting backtest through a different engine or risk path with changed
  semantics; or
- cryptographic authentication of the local journal.

When the final gate is green, the accurate claim is:

> Goal B structurally narrows the current Chaos connectivity and authority
> implementation while preserving the pinned `../imm-strategy` behavior,
> deterministic semantics, and single-writer ownership. It does not make Reap
> demo-approved, production-ready, or authorized for production order entry.
