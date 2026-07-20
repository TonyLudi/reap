# Multi-Venue Polymarket Foundation Goal F Handoff

Status: in progress. Phase 0 is documentation-complete and awaits its
documentation-only gate commit. No production code has changed. This ledger is
architecture and deterministic local evidence, not authenticated-connectivity
evidence or trading authorization.

The historical execution contract is the
[Goal F prompt](multi-venue-polymarket-foundation-goal-f-prompt.md). The
[Polymarket product and connectivity boundary](polymarket-product-connectivity-boundary.md)
is normative for the implementation.

## Scope

Goal F adds a sibling Polymarket product in which:

1. OKX contributes only explicitly declared public crypto reference data;
2. Polymarket contributes configured metadata/book state and fixture/fake
   private, account, position, and reconciliation state;
3. an explicitly supplied pure quote model transforms those inputs into a fair
   PM probability;
4. one checked quote-policy boundary creates exact executable candidates
   strictly inside `(0, 1)`; and
5. only a durable, fake-only Polymarket GTC post-only quote/cancel-owned
   lifecycle is reachable.

There is no Predict.fun connectivity, quote mirroring between prediction
venues, OKX execution, production probability model, live PM signing/auth,
deployed PM binary, target-host qualification, or production trading approval.

## Phase Status

| Phase | Status | Gate commit |
| --- | --- | --- |
| Prompt-only execution contract | Green | `d2593f6d85ce868b46e3c1f16b5a48f221e5e480` |
| 0. Baseline, product contract, dependency and measurement plan | Documentation complete; gate commit pending | The documentation-only commit containing this ledger and boundary |
| 1. Exact PM domain and venue-aware envelopes | Pending | — |
| 2. Capability-specific venue framework seams | Pending | — |
| 3. PM public market data, integrity, capture, replay | Pending | — |
| 4. Read-only private lifecycle and position monitor | Pending | — |
| 5. Passive quote lifecycle and fake execution | Pending | — |
| 6. PM coordinator, quote-model seam, local evidence | Pending | — |
| 7. Documentation, global verification, final audit | Pending | — |

The Phase 0 commit hash will be written into this table by the first later
phase commit, avoiding a self-referential commit hash.

## Phase 0 Baseline Identity

Recorded on 2026-07-20 UTC before production edits.

| Check | Command/evidence | Result |
| --- | --- | --- |
| Reap HEAD | `git rev-parse HEAD` | `d2593f6d85ce868b46e3c1f16b5a48f221e5e480` |
| Branch and remote relation | `git status --short --branch`; `git rev-list --count origin/master..HEAD` | `master`, clean, one prompt-only commit ahead of `origin/master` |
| Required implementation baseline | `git merge-base --is-ancestor 8258deb4b6e3a52e7c58c792da913210e0877fbb HEAD` | Exit `0` |
| Existing behavioral reference | `git -C ../imm-strategy rev-parse HEAD`; status | Clean at `b6b120c7b7c466d8431bf082f3229328c5d7b2ae` |
| PM reference object | `git -C ../predarb cat-file -t 8222273a9c72033b760e1d2fec813bc77144556d` | Available tracked commit object |
| Predarb checkout HEAD | `git -C ../predarb rev-parse HEAD` | `8222273a9c72033b760e1d2fec813bc77144556d` |
| Predarb dirty paths | `git -C ../predarb status --porcelain=v1` | Modified `resources/grafana/pf-maker-v2-dashboard.json`; untracked `.predarb/` |
| Concurrent writers | Goal-session inventory | Phase 0 audit workers were read-only; no overlapping code writer was authorized |

Only tracked Predarb Git objects at the pinned revision were inspected with
`git show`, `git grep`, `git ls-tree`, and `git cat-file`. No `.env`, `.env_bk`,
untracked `.predarb/` data, key, credential, dashboard modification, or other
untracked byte was read. Neither sibling repository was changed, reset, moved,
cleaned, or made a Cargo dependency.

## Host And Toolchain

| Item | Phase 0 value |
| --- | --- |
| Kernel/architecture | Linux `7.0.0-1004-aws`, `aarch64` |
| CPU | 2 vCPU, ARM Neoverse-N1 |
| Rust | `rustc 1.95.0 (59807616e 2026-04-14)`, LLVM 22.1.2 |
| Cargo | `cargo 1.95.0 (f2d3ce0bd 2026-03-21)` |
| Workspace build cache | `target/` 3.7 GiB |
| Free filesystem at record time | approximately 970 MiB of 38 GiB, 98% used |

The existing cache is retained. Temporary inventory writes encountered the
tight filesystem, so later phases must stream evidence where practical and may
remove only verified reproducible build artifacts after confirming no process
uses them. No user or sibling runtime data may be cleaned.

## Deterministic Compatibility Anchors

| Artifact | Phase 0 SHA-256 |
| --- | --- |
| `Cargo.lock` | `319268c86f94883e19668aa4835da615bbecbabfe32902019129e6e40caf894d` |
| `examples/iarb2-basic.toml` | `0fac5a3a35fe28cdc05118b7e22241077aa7f604a9a5436355797605b51b3b26` |
| `examples/live-okx-demo.toml` | `caea78e0a75d2586ecbd16d5b4414f9606a7064b6e1684f82fff2d132a197195` |
| `fixtures/normalized/chaos_quote_hedge.jsonl` | `27f2eb4b9dba7ee600ed645ad8b7c88143e8b54531232991b492cb7595e8ccaa` |
| `fixtures/normalized/chaos_quote_hedge_later.jsonl` | `40453b8be283178b20531c84142dbaeeeca82b4723e5c13594df171c778cd8ee` |
| `fixtures/normalized/chaos_quote_hedge_intents.json` | `d95fa7f121e2e8c402c8108cf9fefb7c7d7b3dbd2b9742c58c234a521f0ee0ec` |
| Goal F prompt | `a9f73453a50d048b56fe1bccda1e0f243bcd5fb30da6b2995ac0b532d0377293` |
| 47-line schema/version inventory | `9aa8a21c5a8678508c3933de738e26232d0b7ba60882d75cb49c6e25b4bb211f` |

Decision-parity anchors remain:

| Artifact | SHA-256 |
| --- | --- |
| `risk_initialization_v1.json` | `7e0951c41f447b9f46a73b24a3fe85bdc8f2bb8a623385dab0c3655926e73780` |
| `replay_events_v1.jsonl` | `dede17a546d4d717c78dc2b3b7aa7c3f3f785d552404160407c78fb87cec9101` |
| `expected_engine_v1.jsonl` | `140c268619b889a19d779e1bdfd340c11901d2eb1d9e4d216d976ba3d8b0d37a` |
| `expected_live_reduction_v1.json` | `aa66cc09bba29cde25ab2df66c018517b2c900f83373f95580150e8bcd442b60` |
| Complete live projection | `847c6f8ba5177cf456d0dc2c7c31df74a9b189c107e7167d06dd48bf09b7762b` |

The canonical CLI backtest was run twice:

```text
cargo run --locked -q -p reap-cli -- backtest \
  --format normalized-jsonl \
  --config examples/iarb2-basic.toml \
  --data fixtures/normalized/chaos_quote_hedge.jsonl \
  --pretty
cmp target/tmp/goal-f-phase0-backtest-1.json \
    target/tmp/goal-f-phase0-backtest-2.json
```

`cmp` exited `0`. Both outputs have SHA-256:

```text
38acf9f5e0c310f2ec5528974beffadf4c1a7f84d46efa8d9664ee7051e84691
```

Every later phase must preserve these existing Chaos artifacts and hashes.

## Existing Workspace Dependency Baseline

`cargo metadata --locked --no-deps --format-version 1` reports 23 workspace
packages. The canonical sorted direct-workspace adjacency has SHA-256
`fe98cedfaa2653e09afd57293eb71372ea476c1997e56b5ce9f27b314f5a432b`:

```text
reap-backtest -> reap-book + reap-capture + reap-core + reap-feed + reap-live-contracts + reap-order + reap-strategy + reap-venue
reap-book -> reap-core
reap-capture -> reap-book + reap-core + reap-feed + reap-telemetry + reap-venue
reap-cli -> reap-backtest + reap-capture + reap-core + reap-emergency-core + reap-fault + reap-feed + reap-live + reap-okx-evidence-adapter + reap-strategy + reap-telemetry
reap-core -> -
reap-emergency-core -> reap-core
reap-emergency-runner -> reap-core + reap-emergency-core + reap-okx-emergency-adapter + reap-order + reap-telemetry
reap-engine -> reap-core + reap-risk + reap-strategy
reap-evidence-core -> -
reap-fault -> reap-core + reap-feed + reap-live
reap-feed -> reap-book + reap-core + reap-venue
reap-live -> reap-core + reap-engine + reap-evidence-core + reap-feed + reap-live-contracts + reap-okx-live-adapter + reap-order + reap-risk + reap-storage + reap-strategy + reap-telemetry + reap-venue
reap-live-contracts -> reap-core + reap-risk + reap-strategy + reap-venue
reap-okx-emergency-adapter -> reap-emergency-core + reap-okx-wire + reap-venue
reap-okx-evidence-adapter -> reap-evidence-core + reap-okx-wire + reap-venue
reap-okx-live-adapter -> reap-core + reap-feed + reap-okx-wire + reap-order + reap-risk + reap-strategy + reap-venue
reap-okx-wire -> -
reap-order -> reap-core + reap-risk + reap-storage + reap-strategy + reap-venue
reap-risk -> reap-core
reap-storage -> reap-core
reap-strategy -> reap-core
reap-telemetry -> reap-core
reap-venue -> reap-core
```

The graph is acyclic. Goal F keeps every edge above and adds a sibling PM DAG
specified in the boundary document. No existing Chaos/OKX/live/order crate may
depend on a PM crate.

## Existing Public, Authority, And Schema Boundary

Phase 0 found:

- `reap-core` currently wildcard-exports f64 `Price`/`Quantity`, `String`
  `Symbol`, and `Venue::Okx`. Only common venue/source identity is eligible for
  a mechanical `Polymarket` addition; PM exact types live in `reap-pm-core`.
- `Subscription` and `RawEnvelope` are useful untrusted edge carriers.
- the current public `VenueAdapter` combines URL, parse, frame classification,
  and subscription serialization, so it must not become a universal PM
  adapter.
- `reap-feed` supervision is bounded, but its session protocol hard-codes OKX
  subscription comparison, login, ACK, text ping/pong, and heartbeat rules.
  Only transport mechanics may be separated; venue protocol stays owned.
- the existing `BookReducer` is f64/Chaos state and is not reused for PM.
- `reap-order` regular execution/reconciliation and `OkxOrderGateway` return
  concrete OKX DTOs and are not widened.
- `LiveConfig`, `ChaosConnectivityPlan`, `LiveCoordinator`, and `LiveRuntime`
  are Chaos/OKX product types and are not widened.
- existing `StorageRecord` is a Chaos/f64 union at schema 7. PM uses a separate
  exact typed journal; only writer/lease mechanics may be shared beneath it.

Relevant root-source SHA-256 anchors:

| Source | SHA-256 |
| --- | --- |
| `reap-core/src/lib.rs` | `338876a73358f608daec564f09a750a15cbcc5d270e293e5bd4df0768dd45d7c` |
| `reap-venue/src/lib.rs` | `b52b11b663b9537852f80f864d297611a7158b7d57928543243d64e7e75d553a` |
| `reap-feed/src/lib.rs` | `b5a9180e4e53f4fd0b4798312cd421cb6f37c15bba02897688a764f44850e94e` |
| `reap-book/src/lib.rs` | `77bcc416809f93c6d1c80894d769ec321597905053bddd1720029875d7ca32a4` |
| `reap-order/src/lib.rs` | `be777600bf192921704f5a4b3d11def506de08fe12c0dc6bd2e060228ae09a75` |
| `reap-storage/src/lib.rs` | `367cc69b92b2c4de32d9c19d2a3f5e6df65868ecaabde94169d806dd36492534` |
| `reap-live-contracts/src/lib.rs` | `95c0780dd4eac178f40dd94bbce62f01d1154089f297be0287d7eeb98609e155` |
| `reap-live/src/lib.rs` | `3268546423f2bc8bdd209761de22bc61b7961c6835083869384c5a131fde1c2c` |
| `reap-capture/src/lib.rs` | `a8c3cf9fb91a1c45a3f297e0b708dd68a8675b280cb64078a9e58b92e929cdbc` |
| `reap-okx-live-adapter/src/lib.rs` | `735fec80c5d7055990dff11d9a2d6ca3ee8b9886ae76144d4d379dde7ee3718e` |

The complete workspace public-declaration inventory is reproducible with:

```text
rg -H --no-heading --no-line-number --glob '*.rs' \
  '^pub(\([^)]*\))? ' crates | sort
```

The sorted stream contains 1,433 lines and has SHA-256
`27e958b1fbfda38b7a3e9cb4ffbffe2028386f11208afde5db9a19264e27be23`.
It includes qualified forms such as `pub async fn` and scoped forms such as
`pub(crate)`. This is the complete top-level public-visibility syntactic
baseline; later source-policy checks compare against the generated inventory
rather than a hand-selected crate subset.

The complete schema/version inventory is reproducible with:

```text
rg -n --no-heading \
  '^(pub(\([^)]*\))? )?const [A-Z0-9_]*((SCHEMA|FORMAT|POLICY|PROTOCOL)_VERSION|PINNED_JAVA_REVISION)' \
  crates --glob '*.rs' | sort
```

Its exact 47-line sorted output, including trailing newlines, has SHA-256
`9aa8a21c5a8678508c3933de738e26232d0b7ba60882d75cb49c6e25b4bb211f`:

```text
crates/reap-backtest/src/calibration.rs:10:pub const LATENCY_CALIBRATION_SCHEMA_VERSION: u32 = 4;
crates/reap-backtest/src/lib.rs:59:pub const BACKTEST_CARRY_STATE_SCHEMA_VERSION: u16 = 1;
crates/reap-backtest/src/research.rs:57:pub const RESEARCH_SCHEMA_VERSION: u32 = 8;
crates/reap-backtest/src/research_verification.rs:15:pub const RESEARCH_VERIFICATION_FORMAT_VERSION: u16 = 3;
crates/reap-capture/src/analysis.rs:15:const ANALYSIS_FORMAT_VERSION: u16 = 5;
crates/reap-capture/src/report.rs:6:pub const CAPTURE_RUN_REPORT_FORMAT_VERSION: u16 = 5;
crates/reap-capture/src/verification.rs:20:pub const CAPTURE_VERIFICATION_FORMAT_VERSION: u16 = 3;
crates/reap-cli/src/deployment.rs:12:pub(crate) const RESEARCH_DEPLOYMENT_VERIFICATION_FORMAT_VERSION: u16 = 2;
crates/reap-cli/src/latency.rs:24:pub(crate) const LATENCY_CALIBRATION_VERIFICATION_FORMAT_VERSION: u16 = 1;
crates/reap-cli/src/production_approval.rs:22:const APPROVAL_POLICY_SCHEMA_VERSION: u16 = 1;
crates/reap-cli/src/production_approval.rs:23:const APPROVAL_KEY_FORMAT_VERSION: u16 = 1;
crates/reap-cli/src/production_approval.rs:24:const APPROVAL_POLICY_VERIFICATION_FORMAT_VERSION: u16 = 1;
crates/reap-cli/src/production_approval.rs:25:const APPROVAL_REQUEST_FORMAT_VERSION: u16 = 1;
crates/reap-cli/src/production_approval.rs:26:const APPROVAL_SIGNATURE_FORMAT_VERSION: u16 = 1;
crates/reap-cli/src/production_approval.rs:27:const APPROVAL_VERIFICATION_FORMAT_VERSION: u16 = 1;
crates/reap-cli/src/production_evidence.rs:40:pub(crate) const PRODUCTION_EVIDENCE_MANIFEST_SCHEMA_VERSION: u16 = 8;
crates/reap-cli/src/production_evidence.rs:41:pub(crate) const PRODUCTION_EVIDENCE_REPORT_FORMAT_VERSION: u16 = 9;
crates/reap-cli/src/production_evidence.rs:42:pub(crate) const PRODUCTION_EVIDENCE_APPROVAL_SUBJECT_FORMAT_VERSION: u16 = 1;
crates/reap-core/src/types.rs:8:pub const PINNED_JAVA_REVISION: &str = "b6b120c7b7c466d8431bf082f3229328c5d7b2ae";
crates/reap-emergency-core/src/lib.rs:26:pub const EMERGENCY_CANCEL_REPORT_SCHEMA_VERSION: u32 = 2;
crates/reap-emergency-core/src/verification.rs:15:pub const EMERGENCY_CANCEL_VERIFICATION_FORMAT_VERSION: u16 = 3;
crates/reap-engine/tests/support/decision_replay.rs:16:pub const INITIALIZATION_SCHEMA_VERSION: u32 = 1;
crates/reap-engine/tests/support/decision_replay.rs:17:pub const REPLAY_SCHEMA_VERSION: u32 = 1;
crates/reap-engine/tests/support/decision_replay.rs:18:pub const PROJECTION_SCHEMA_VERSION: u32 = 1;
crates/reap-fault/src/config.rs:13:const CONFIG_SCHEMA_VERSION: u32 = 1;
crates/reap-fault/src/protocol.rs:10:pub const INJECTOR_EVIDENCE_FORMAT_VERSION: u32 = 1;
crates/reap-fault/src/protocol.rs:11:pub const RUN_REPORT_FORMAT_VERSION: u32 = 2;
crates/reap-fault/src/protocol.rs:9:pub const CONTROL_FORMAT_VERSION: u32 = 1;
crates/reap-fault/src/verification.rs:12:pub const FAULT_PROXY_RUN_VERIFICATION_FORMAT_VERSION: u16 = 1;
crates/reap-live-contracts/src/account_certification.rs:20:pub const ACCOUNT_CASH_POLICY_VERSION: u32 = 1;
crates/reap-live-contracts/src/account_certification.rs:21:pub const ACCOUNT_CERTIFICATION_SCHEMA_VERSION: u32 = 3;
crates/reap-live-contracts/src/connectivity_plan.rs:14:pub const CHAOS_CONNECTIVITY_PLAN_SCHEMA_VERSION: u32 = 1;
crates/reap-live/src/bill_collection.rs:26:pub const BILL_COLLECTION_SCHEMA_VERSION: u32 = 1;
crates/reap-live/src/deadman_certification.rs:30:pub const DEADMAN_EXPIRY_CERTIFICATION_SCHEMA_VERSION: u32 = 1;
crates/reap-live/src/economic_statement.rs:35:pub const ECONOMIC_RECONCILIATION_SCHEMA_VERSION: u32 = 5;
crates/reap-live/src/fault_campaign.rs:17:pub const LIVE_FAULT_MATRIX_MANIFEST_SCHEMA_VERSION: u32 = 3;
crates/reap-live/src/fault_campaign.rs:18:pub const LIVE_FAULT_MATRIX_REPORT_FORMAT_VERSION: u32 = 5;
crates/reap-live/src/fill_collection.rs:26:pub const FILL_COLLECTION_SCHEMA_VERSION: u32 = 1;
crates/reap-live/src/latency.rs:6:pub const LIVE_LATENCY_EVIDENCE_SCHEMA_VERSION: u32 = 2;
crates/reap-live/src/live_verification.rs:17:pub const LIVE_RUN_VERIFICATION_FORMAT_VERSION: u16 = 2;
crates/reap-live/src/operator.rs:16:const PROTOCOL_VERSION: u16 = 2;
crates/reap-live/src/production_transition.rs:15:pub const PRODUCTION_TRANSITION_FORMAT_VERSION: u16 = 3;
crates/reap-live/src/runtime.rs:120:pub const LIVE_RUN_REPORT_SCHEMA_VERSION: u32 = 8;
crates/reap-live/src/runtime/health.rs:12:const RUNTIME_HEALTH_SCHEMA_VERSION: u32 = 1;
crates/reap-live/src/statement.rs:19:pub const FILL_STATEMENT_REPORT_SCHEMA_VERSION: u32 = 2;
crates/reap-live/tests/coordinator_unit/decision_parity.rs:19:const LIVE_PROJECTION_SCHEMA_VERSION: u32 = 1;
crates/reap-storage/src/lib.rs:813:const CURRENT_SCHEMA_VERSION: u16 = 7;
```

PM schemas begin independently at version 1.

## Source Policy And Compile-Fail Baseline

`crates/reap-live/tests/dependency_policy.rs` contains seven structural guard
families:

1. workspace/authenticated-authority DAG;
2. exact regular-authority owner allowlists;
3. raw OKX DTO/transport owner allowlists;
4. canonical numeric lowering;
5. bounded storage telemetry;
6. private bounded runtime health; and
7. terminal external-test marker integrity.

There are 23 existing UI rejection cases: strategy 3, venue 3, feed 4, order 5,
OKX live adapter 6, and live 2. Goal F adds parallel PM dependency,
visibility, authority, exactness, and role-separation guards. It cannot weaken
an existing allowlist or overwrite a trybuild suite wholesale.

The complete Phase 0 structural baseline was run with:

```text
cargo test -p reap-live --test dependency_policy --locked
cargo test -p reap-strategy --test compile_fail_boundaries --locked
cargo test -p reap-venue --test compile_fail_boundaries --locked
cargo test -p reap-feed --test compile_fail_boundaries --locked
cargo test -p reap-order --test compile_fail_boundaries --locked
cargo test -p reap-okx-live-adapter --test compile_fail_boundaries --locked
cargo test -p reap-live --test compile_fail_boundaries --locked
```

All seven dependency-policy tests and all 23 UI cases passed. The first OKX
adapter attempt exhausted the temporary filesystem while compiling `ring`;
after removing only reproducible Cargo trybuild artifacts, the unchanged
six-case adapter suite and two-case live suite both passed.

## Source Size And Ownership Baseline

Production-file extent ends immediately before an adjacent terminal
`#[cfg(test)]` / `mod tests {` pair; earlier test-gated helpers do not
accidentally truncate the file. The complete 170-file inventory is generated
by:

```text
while IFS= read -r file; do
  awk -v file="$file" \
    'previous == "#[cfg(test)]" && $0 ~ /^mod tests[[:space:]]*\{/ \
       { count -= 1; exit }
     { count += 1; previous = $0 }
     END { printf "%d\t%s\n", count, file }' "$file"
done < <(rg --files crates -g '*.rs' | rg '/src/' | sort) |
  sort -k1,1nr -k2,2
```

The exact sorted stream has SHA-256
`59f94141fa0672f79d8636898930d0de7d8a2146f4aa9a521f52db6a918620dd`.
Every production source at least 1,000 physical lines is:

| Lines | Existing production source |
| ---: | --- |
| 2,288 | `crates/reap-venue/src/okx/rest.rs` |
| 1,872 | `crates/reap-cli/src/main.rs` |
| 1,813 | `crates/reap-live-contracts/src/config.rs` |
| 1,688 | `crates/reap-live-contracts/src/connectivity_plan.rs` |
| 1,479 | `crates/reap-emergency-runner/src/lib.rs` |
| 1,422 | `crates/reap-storage/src/lib.rs` |
| 1,397 | `crates/reap-risk/src/lib.rs` |
| 1,372 | `crates/reap-live/src/deadman_certification.rs` |
| 1,361 | `crates/reap-okx-live-adapter/src/lib.rs` |
| 1,349 | `crates/reap-live/src/fault_campaign.rs` |
| 1,315 | `crates/reap-cli/src/production_evidence/bindings.rs` |
| 1,291 | `crates/reap-backtest/src/research/verification.rs` |
| 1,281 | `crates/reap-live/src/runtime/health.rs` |
| 1,169 | `crates/reap-okx-live-adapter/src/order_ws.rs` |
| 1,167 | `crates/reap-live/src/bill_collection.rs` |
| 1,154 | `crates/reap-strategy/src/chaos/instrument.rs` |
| 1,152 | `crates/reap-cli/src/latency.rs` |
| 1,141 | `crates/reap-live-contracts/src/account_certification.rs` |
| 1,108 | `crates/reap-live/src/runtime/startup.rs` |
| 1,105 | `crates/reap-live/src/fill_collection.rs` |
| 1,094 | `crates/reap-strategy/src/chaos/config.rs` |
| 1,068 | `crates/reap-live/src/statement.rs` |
| 1,067 | `crates/reap-cli/src/production_approval.rs` |
| 1,062 | `crates/reap-order/src/authority.rs` |
| 1,048 | `crates/reap-venue/src/okx/public.rs` |
| 1,028 | `crates/reap-venue/src/okx/capabilities.rs` |
| 1,021 | `crates/reap-live/src/runtime.rs` |
| 1,017 | `crates/reap-capture/src/analysis.rs` |
| 1,015 | `crates/reap-fault/src/proxy.rs` |
| 1,014 | `crates/reap-feed/src/supervisor.rs` |

Existing large files are baseline exceptions, not precedents. No new Goal F
production module may exceed 1,500 lines.

Function candidates were discovered with:

```text
rg -n --glob '*.rs' \
  '^[[:space:]]*(pub(\([^)]*\))?[[:space:]]+)?(async[[:space:]]+)?fn[[:space:]]' \
  crates
```

Each reported production span is the physical declaration line through its
matching closing brace, inclusive, after masking comments and string/character
literals and excluding the terminal test module defined above. The canonical
ten-row `lines<TAB>path:start<TAB>name` inventory has SHA-256
`50049ee328702182b292bda8c2b6467671b97867a1e71378df022c3dbadc64e4`:

| Lines | Source | Function |
| ---: | --- | --- |
| 1,005 | `crates/reap-cli/src/main.rs:736` | `main` |
| 442 | `crates/reap-live/src/runtime/startup.rs:617` | `RuntimeResources::start` |
| 284 | `crates/reap-live/src/runtime/bootstrap.rs:37` | `bootstrap_accounts` |
| 277 | `crates/reap-capture/src/runtime.rs:137` | `run_capture` |
| 271 | `crates/reap-live/src/runtime/dispatch.rs:357` | `run_order_task` |
| 224 | `crates/reap-live/src/runtime.rs:614` | `LiveRuntime::run_loop` |
| 182 | `crates/reap-feed/src/supervisor.rs:832` | `supervise_connection` |
| 175 | `crates/reap-feed/src/pipeline.rs:253` | `process_source_book` |
| 172 | `crates/reap-order/src/private.rs:339` | `PrivateStateReducer::apply_order` |
| 171 | `crates/reap-live/src/coordinator.rs:655` | `LiveCoordinator::process_event` |

No new Goal F production function may exceed 250 lines without a pre-recorded
responsibility exception and decomposition review.

Principal state aggregates were found with:

```text
rg -n --glob '*.rs' \
  '^(pub(\([^)]*\))? )?struct (InstrumentState|ChaosStrategy|LiveCoordinator|LiveRuntime|BacktestRunner|OkxOrderGateway|FeedProcessor)\b' \
  crates
```

Root-field counts include only declaration-level named fields, not fields
nested inside owned types. The canonical seven-row
`fields<TAB>path:start<TAB>name` inventory has SHA-256
`ac71cac3d1828682292b0d2e8199c4b4271420d2753850cde60b92c7526e528c`:
`InstrumentState` 41, `LiveCoordinator` 13, `ChaosStrategy` 12,
`BacktestRunner` 11, `LiveRuntime` 11, `OkxOrderGateway` 8, and
`FeedProcessor` 5. The PM coordinator may own multiple focused reducers by
value; it must not become one file containing all their responsibilities or a
shared-lock service graph.

## Phase 0 Architecture Decisions

The normative boundary freezes:

- structural PM/account/order identities and compact prevalidated handles;
- exact fixed-width integer PM price, quantity, collateral, balance, ERC-20
  allowance, position, reservation, and fill state, plus tagged ERC-1155
  operator-approval state;
- the CLOB V2 standard/negative-risk trading-spender set;
- authoritative metadata, complete-snapshot, book-integrity, ownership,
  fill-normalization, fee-sign, readiness, and PM-specific risk contracts;
- five non-interchangeable capability roles;
- one GTC, `postOnly=true`, `deferExec=false` fake place profile and
  cancel-one-owned profile;
- one by-value PM coordinator;
- bounded lanes, deterministic priority/bursts, capacity/age saturation, and
  replay ordering;
- the sibling PM target dependency shape, neutral transport,
  capture-framing, and leased durable-writer crates, plus one
  capability-narrow OKX public-source crate;
- PM-specific schema-v1 artifacts rather than a Chaos schema union;
- no default or production probability model; and
- exact nominal/overload benchmark workloads and bounds.

The full matrices, limits, identities, DAG, schemas, and stop conditions are in
[polymarket-product-connectivity-boundary.md](polymarket-product-connectivity-boundary.md).

## Authoritative Protocol Evidence

### Official retrievals

The following public, secret-free official pages were retrieved on 2026-07-20
UTC. SHA-256 is over the complete compressed-decoded HTTP response bytes
returned by `curl -L --compressed -sS URL` on that date.

| Official page | SHA-256 | Reached evidence |
| --- | --- | --- |
| `https://docs.polymarket.com/trading/orders/overview` | `3ea497d8782c3110bac4a12aae2d10771aa2c116e4e804410650c3ad92099025` | GTC rests; post-only rejects crossing; tick behavior; CLOB V2 exchange allowance |
| `https://docs.polymarket.com/api-reference/trade/post-a-new-order` | `27bec1e59bc22daf9d12e47b44c7f43d590c2d43c74ea260acfa9f91b450047e` | CLOB V2 unsigned/body fields and outer order request example |
| `https://docs.polymarket.com/api-reference/markets/get-clob-market-info` | `7e69d12436352bbfd93992901fffdf9c268358b4305667bf9183682118e8104b` | Token membership, minimum order size, tick and fee metadata |
| `https://docs.polymarket.com/trading/clients/l2` | `9a8ad2c1ebc6952bc397153a44f23a155cdf681fd60a4d38bcd113784d8bda9a` | Post-order type/post-only role and read endpoints |
| `https://docs.polymarket.com/resources/contracts` | `36da5a4f9c0853b59646b88e36fab078583e8345a48ceec4e6293f5c52f7b8d7` | Polygon CLOB V2 exchanges; old neg-risk adapter explicitly deprecated |
| `https://docs.polymarket.com/market-makers/getting-started` | `c9860eb12f16469e952574438f36bed7ef60e9baa71410dc6a2acda90a755964` | Required trading approvals and current exchange addresses |
| `https://docs.polymarket.com/changelog` | `90e8ba2003fa723d5ac882065c95cf523ba8cd11c2ba6e9980f35d38cf141507` | CLOB V2 cutover and unsigned-field/domain migration |
| `https://docs.polymarket.com/v2-migration` | `c4e05ccb7ec35470f6a4396b7ac4e7528fb90e2a22a5c5b4f2b61321f6f2c6bd` | Millisecond timestamp, V2 signed-field set and expiration body distinction |
| `https://docs.polymarket.com/trading/orders/create` | `c3146d42f8d834f5d3c87af47767e39e49d5ee3b3cc4a025f6331c3363b2bad1` | GTC/tick/neg-risk and funder allowance semantics |
| `https://docs.polymarket.com/trading/clients/public` | `9289d397dba406dba11126177a4da332d97a38271606cff5aeeec282b7008e25` | Market lifecycle fields `active`, `closed`, `archived`, `accepting_orders`, and `enable_order_book` |
| `https://docs.polymarket.com/api-reference/market-data/get-order-book` | `0662b2dfe5328a7a8a4fbe5cd19f6675e1a509571900e1145f88288c67b23405` | Exact book snapshot fields, minimum, tick, neg-risk and hash |

This closes the Phase 0 fake-profile gate in combination with the pinned source
and independent golden specifications below. These pages are mutable; later
authenticated execution must pin and re-audit the then-current protocol.

Official client and exchange sources were also retrieved as immutable Git
objects. SHA-256 is over the raw file bytes at the exact revision:

| Official repository and revision | Source | SHA-256 | Reached evidence |
| --- | --- | --- | --- |
| `Polymarket/rs-clob-client-v2@222143d321eba97d5711a848265eb9aab3bc7ff4` | `src/clob/order_builder.rs` | `7071149a34578310c1a6f0c52eac121e75f5eb238a0b8633b10dc7ae8e04af7e` | Two-decimal lot scale, maker/taker construction, and salt masking below `2^53` |
| same | `src/clob/types/mod.rs` | `1311f0b29b4f013eb60582cb0f716a2bcc30b708816c6763624f694526e9e814` | CLOB V2 order types, tick enum, and numeric salt serialization |
| same | `src/clob/utilities.rs` | `33ed6a3704b5ae5a0fa464883388ca743b1a9d2019a8bd13040655c2a6659930` | Six-decimal collateral units and price validation across the tick enum |
| `Polymarket/clob-client-v2@f3e1a05f868a1fd0c34ef85dfc45c6ce78f5bb69` | `src/order-builder/helpers/roundingConfig.ts` | `0fd2d5020c1dd9b717788fc4f58d5a4ea28b790ad97170a7b4042b6e9864001f` | Supported tick set, including `0.005` and `0.0025` |
| `Polymarket/ctf-exchange-v2@ccc0596074f4dfd62c944fbca4de252893b82b4b` | `src/exchange/libraries/Structs.sol` | `533fe017a934e9f7500519961f1b7d350c2e76732ca66cb837bd82406854c8c2` | On-chain order struct and amount representation |
| same | `src/exchange/mixins/AssetOperations.sol` | `320a0b48a78843a8a68769166a00b82cca15bec3216169d81015e7750833d7a4` | ERC-20 collateral versus ERC-1155 outcome-token authorization |
| same | `src/exchange/mixins/Trading.sol` | `dd8d18fca897e664583a93944b379435e6f70e84f4190c39d669b2be62596012` | Exact integer maker/taker ratio, fill, and remaining-amount constraints |

These immutable sources support the exact integer, lot, tick, salt, and tagged
authorization rules. The independently authored goldens below remain the
implementation oracle; no client implementation is copied.

### Independent protocol goldens

These compact single-line JSON specifications contain no newline, signature,
credential, or secret. Their hashes freeze independently authored Goal F
expectations; later phases turn them into checked-in parser/authority fixtures.

Passive fake profile:

```json
{"deferExec":false,"orderType":"GTC","postOnly":true,"price":"0.40","size":"10.000000","tokenId":"123"}
```

SHA-256:
`448c5e0fb87cea02fd09b529e4e1d2fa36dead8e3433cb47f9d97a97d9c8a356`.

Canonical unsigned CLOB V2 fields (no expiration or signature in the signed
field set):

```json
{"builder":"0x0000000000000000000000000000000000000000000000000000000000000000","maker":"0x1111111111111111111111111111111111111111","makerAmount":"4000000","metadata":"0x0000000000000000000000000000000000000000000000000000000000000000","salt":123456789,"side":"BUY","signatureType":0,"signer":"0x1111111111111111111111111111111111111111","takerAmount":"10000000","timestamp":"1760000000000","tokenId":"123"}
```

SHA-256:
`4eebc7d683c2763159a49be87120ce63dc3d58866200578086bb4a43ef7db6b0`.

Standard and negative-risk CLOB V2 trading-spender sets:

```json
{"authorizations":[{"asset":"0xC011a7E12a19f7B1f670d46F03B03f3342E82DFB","kind":"erc20_allowance","spender":"0xE111180000d2663C0091e4f400237545B87B996B"},{"asset":"0x4D97DCd97eC945f40cF65F87097ACe5EA0476045","kind":"erc1155_operator_approval","spender":"0xE111180000d2663C0091e4f400237545B87B996B"}],"chain":137,"negRisk":false}
{"authorizations":[{"asset":"0xC011a7E12a19f7B1f670d46F03B03f3342E82DFB","kind":"erc20_allowance","spender":"0xe2222d279d744050d28e00520010520000310F59"},{"asset":"0x4D97DCd97eC945f40cF65F87097ACe5EA0476045","kind":"erc1155_operator_approval","spender":"0xe2222d279d744050d28e00520010520000310F59"}],"chain":137,"negRisk":true}
```

SHA-256, respectively:
`e6fa93aa8cb827bb3588b6fd93b6b556767513e27b81d08d11f3c031d4952b10`
and
`dbc952558b70ec13ed84c7ec90852dd212469c39d4993e8f33c35d2f19cbfd91`.

Off-lot quantity rejection without rounding:

```json
{"expected":"reject_off_lot_quantity_without_rounding","lot":"0.01","price":"0.50","quantity":"10.000002","tick":"0.0001"}
```

SHA-256:
`6e18e0044cf1d68de6ae153fcfcc867c1e2c86ce51860c703c515a4cd4345878`.

JSON-safe salt rejection:

```json
{"expected":"reject_salt_outside_json_safe_integer","salt":9007199254740992}
```

SHA-256:
`c171f0916ff8a8fbb10455cf004866a63adb1745c4f689d4b96ebaf9f2df4292`.

The outer API example contains an `expiration` body field, while official CLOB
V2 documentation states that expiration is not in its signed order struct.
Goal F's fixed GTC fake effect uses an explicit zero/non-expiring body value if
that field is represented; it does not implement signing.

## Pinned Predarb Provenance

The following tracked fixtures were hashed from
`8222273a9c72033b760e1d2fec813bc77144556d:path`:

| Pinned fixture | SHA-256 | Goal F use |
| --- | --- | --- |
| `crates/venue-polymarket/tests/fixtures/balance_allowance.json` | `7e1f683ac5032b137d8a2afdfafccce389198bb5d3a33ba6eb3cb478455fab96` | Parser seed only; not spender proof |
| `crates/venue-polymarket/tests/fixtures/market_book.json` | `8e671f14c4b1e8137b1dc1b0bd7d39c79d9c8f961a8483daa32151df99cbdf81` | Exact snapshot seed |
| `crates/venue-polymarket/tests/fixtures/open_order.json` | `d0998ca29cf47ce4bcb1fb4d7183d1e895a044d859235230a6ebef464295baf2` | Open-order parser seed |
| `crates/venue-polymarket/tests/fixtures/user_order.json` | `e4c3cd7975b7dc16c4c8d014444fc2a96d927cf1b9089b33875a5450b4ff99fa` | Simple live-order seed |
| `crates/venue-polymarket/tests/fixtures/user_trade.json` | `042998055ec5dec2c69065d002b2619d8497faabd9bfcc36c27a1bcf7cfe224c` | Trade parser seed; insufficient ownership linkage |
| `test_issue/fill_contract_fixtures/cumulative_overfill_rejection.json` | `205faeb8470200dceaa6352fef99ff622c7fec9a81c836b54cd9ac8801faee22` | Differential fill failure |
| `test_issue/fill_contract_fixtures/duplicate_push_reconcile.json` | `05e3d10ddc461779b9880039be93c22589a65faae371bad46ff6e0acbeaae96e` | Push/reconcile dedup seed |
| `test_issue/fill_contract_fixtures/journal_replay_duplicate_fill.json` | `d9818a85894eb83faee8d9728076ddc81738d45c5687ad10fe8e942f49fc0b8f` | Recovery dedup seed |
| `test_issue/fill_contract_fixtures/out_of_order_reconcile.json` | `0ff00658168e0365ca6dd27c91a6e9a9e444b45f9a9d1e4091571d9758d9992d` | Ordering/convergence seed |
| `test_issue/fill_contract_fixtures/push_only_fill.json` | `24b0f1fcfa21889b1c9e8eb1280bffd6fc1984eb2c10ee0d52ae6c60db402f23` | Private-only fill seed |
| `test_issue/fill_contract_fixtures/submitted_success_duplicate.json` | `ab5cf69eda58bbbec5eac2fe52adb66d717bc0f87acd5f965b57536740c5a7de` | Submit/push duplicate seed |
| `test_issue/fill_contract_fixtures/success_only_fill.json` | `ae6787d0ca69632871ba08c2d300c9f1a1af6f80c84cf417bc9284163cc74947` | Submit response seed |
| `test_issue/fill_contract_fixtures/terminal_reconcile_fill.json` | `1dfe935efd6c052ab23c24e1bb9e58bf867c6d98ac729adb297f5fbe0d046dd1` | Terminal reconciliation seed |

Important tracked source content SHA-256:

| Pinned source | SHA-256 |
| --- | --- |
| `crates/venue-polymarket/src/adapter.rs` | `212f7018150f124df30cf80b70b1ad0248138ecb65c7ca1defca8fb6590f8da1` |
| `crates/venue-polymarket/src/raw/rest.rs` | `bd58ae73031202df4e92c71914b78c6007eac7d6c108adf5467b9df388f3af29` |
| `crates/venue-polymarket/src/raw/ws.rs` | `f5f2fe05394ee583e5320380b18633b745dc3115abcd5c924a712f04b4c45b9d` |
| `crates/venue-polymarket/src/readiness.rs` | `dbcbbdc654cdf91e6175cd58a9920c6da8a08000dae7a399605b5d9584e93c66` |
| `crates/venue-polymarket/src/rest/private.rs` | `8ec857e96f46cca1bccd2bc6253d28519ea996eeba3e88449e8968e92b768b3d` |
| `crates/venue-polymarket/src/rest/public.rs` | `16d47df2cef1a11e1f50c76acd325a00ab394128eb20fde5189dd8adc3a01ba8` |
| `crates/venue-polymarket/src/signing/order_v2.rs` | `ec50bd07594972482a82edeacaceb7ddfb92ff29f42a1272066d0ab0f25fddc1` |
| `crates/venue-polymarket/src/ws/market.rs` | `4b1be0c67d80641481dd5be2058ca2f24aee0002d47f5cc223d813fd4bffb598` |
| `crates/venue-polymarket/src/ws/user.rs` | `8f091ddf3c3d77ca7b3e70d02af9562a982292f3e698d233232e2dc4eba25e50` |
| `src/market_monitor.rs` | `33961cea0aa511c06b61c2bad0496a2fd8ff6c3e71903d11c5f8c36fabb5e227` |
| `src/order_gateway/user_events.rs` | `3e9126227057dccea846433b44098e7150d2e87a42dba872387dfd4daef6965b` |
| `src/order_gateway/polymarket.rs` | `f38e4a8efa6aaa196dc53869d8e0c0081d737eda89c37598ef66a4d4f04a015c` |
| `src/order_gateway/fills.rs` | `7cbc2d8d04002ba94666fe700c3190b02d3929fecd1b09cac0b761da3f8785e4` |
| `src/order_gateway/store.rs` | `8e9aff286614f5e3c44b370f3c6f520d161eb5416f28b196442aadbaaf0a2244` |
| `src/position_gateway/mod.rs` | `27734b1469b895752c04495e188b1f2e996d73bda79a10a30fc7cf9e506db606` |

The pinned reference proves the public/private response shapes, fixture
semantics, a live-smoke GTC/post-only combination, and the unsigned CLOB V2
field family. It is not copied wholesale and is not authority for application
shape, Predict.fun behavior, authentication, or production execution.

## Reference Defects Converted To Required Tests

Before porting each affected behavior, later phases must introduce a focused
failing fixture or compile-fail/source-policy test for:

1. an individual REST trade being treated as cumulative fill with zero
   remaining, followed by a silently ignored cumulative-contract failure;
2. allowance resolution choosing an arbitrary first map value, resolving by
   funder instead of exact spender, or collapsing required spenders;
3. six-decimal maker/taker conversion rounding non-integral units;
4. missing `price < 1`, exact tick divisibility, minimum/lot, and integral
   maker/taker validation;
5. unknown order status becoming `Pending`, private status being silently
   dropped, and required fields defaulting empty;
6. immediate matched submit responses discarding trade IDs and making/taking
   amounts;
7. PM/account executable state entering through `f64`;
8. a purported read-only role deriving credentials or invoking
   balance-allowance update;
9. `.PM`/`.PF` suffix identity or formatted position keys;
10. invalid public JSON/event/decimal yielding a successful empty report;
11. negative book delta interpreted as deletion instead of explicit
    zero/delete; and
12. the pinned CLOB V1 Neg Risk Adapter remaining in the CLOB V2 trading
    spender set after its official deprecation.

Useful positive tests to translate include snapshot/delta/resync and hash
integrity; crossed/mismatched BBO rejection; explicit zero delete; full
snapshot replacement; private order/trade array parsing; taker/maker ownership
references; cancel/reject/expire/open plus unknown rejection; partial,
multiple, duplicate, immediate, reconnect and out-of-order fills; complete
pagination and 404/null order detail; scalar and per-spender allowances;
atomic complete snapshots; buy/sell reservations; and authoritative versus
provisional position convergence.

## Existing Local Performance Baseline

Method for each target: locked bench profile, one unrecorded warm-up followed
by three same-host recorded runs.

### Engine event loop

```text
cargo bench -p reap-engine --bench event_loop --locked
```

| Run | Events | Intents | ns/event |
| ---: | ---: | ---: | ---: |
| 1 | 250,000 | 999,996 | 11,968.8 |
| 2 | 250,000 | 999,996 | 11,783.5 |
| 3 | 250,000 | 999,996 | 11,746.3 |
| Median | — | — | 11,783.5 |

### Live loop

```text
cargo bench -p reap-live --bench live_loop --locked
```

| Segment | Run 1 ns/unit | Run 2 | Run 3 | Median |
| --- | ---: | ---: | ---: | ---: |
| Wire parse and raw record | 2,955.6 | 2,926.2 | 2,948.6 | 2,948.6 |
| Dedup, sequence, book | 7,712.6 | 8,426.0 | 7,706.8 | 7,712.6 |
| Coordinator, strategy, risk, record | 4,140.3 | 4,051.3 | 4,137.7 | 4,137.7 |
| Full live parity observe | 17,098.2 | 18,507.9 | 17,635.5 | 17,635.5 |

Logical and allocation counters were identical across runs:

| Segment | Units | Allocations | Requested bytes | Outputs |
| --- | ---: | ---: | ---: | --- |
| Wire | 50,204 | 1,673,504 | 158,570,992 | parsed 50,204 |
| Dedup/book | 50,204 | 670,868 | 1,349,274,641 | feed 70,208 |
| Coordinator | 70,208 | 1,849,399 | 364,106,336 | records 65,130 |
| Full | 50,204 | 4,193,771 | 1,871,951,969 | parsed 50,204, feed 70,208, records 65,130 |

### Existing action path

```text
cargo bench -p reap-live --bench action_path --locked
```

The table reports the median of the three recorded exact-nearest-rank results:

| Workload | p50 ns | p99.9 ns | Allocation calls | Requested bytes |
| --- | ---: | ---: | ---: | ---: |
| Quote creation/prepare | 19,298 | 128,473 | 19,733,342 | 836,475,432 |
| Quote replacement/owned cancel | 12,980 | 42,583 | 16,600,000 | 773,500,000 |
| IOC hedge decision/prepare | 24,951 | 209,095 | 23,433,342 | 1,243,175,432 |
| Risk rejection | 11,971 | 32,803 | 15,300,000 | 622,400,000 |
| Symbol fail-close/owned cancel | 640 | 2,995 | 1,300,000 | 46,000,000 |
| Global fail-close/owned cancel | 771 | 3,938 | 1,600,000 | 52,600,000 |
| Coordinator normalized storage reduction | 5,612 | 12,841 | 3,900,000 | 158,800,000 |
| Raw sequence-gap recovery/action record | 26,157 | 93,545 | 21,980,027 | 1,777,312,151 |
| Public-trade implied-depth reprice | 11,962 | 21,382 | 15,000,264 | 615,207,392 |
| Bounded biased control/feed storm | 140 | 7,713 | 0 | 0 |

Timer-read median was p50 33 ns, p95/p99 41 ns, and p99.9 42 ns. The storm
workload had capacity/high-water 80 and exactly 30,000 saturations, 20,000
control dequeues, and 80,000 feed dequeues in every run.

Logical counters were also identical across all three recorded runs. The table
lists every nonzero field; every omitted field in the benchmark schema is
zero:

| Workload | Exact nonzero logical counters |
| --- | --- |
| Quote creation/prepare | inputs 100,000; normalized 100,000; typed intents 400,000; quote intents 400,000; prepared submits 100,000; produced actions 400,000 |
| Quote replacement/owned cancel | inputs 100,000; normalized 100,000; typed intents 500,000; quote intents 400,000; cancel-owned intents 100,000; prepared cancels 100,000; produced actions 500,000 |
| IOC hedge decision/prepare | inputs 100,000; normalized 100,000; typed intents 500,000; quote intents 400,000; hedge intents 100,000; prepared submits 100,000; produced actions 500,000 |
| Risk rejection | inputs 100,000; normalized 100,000; risk rejections 400,000 |
| Symbol fail-close/owned cancel | inputs 100,000; normalized 100,000; safety-cancel candidates 100,000; prepared cancels 100,000; produced actions 100,000 |
| Global fail-close/owned cancel | inputs 100,000; normalized 100,000; safety-cancel candidates 200,000; prepared cancels 200,000; produced actions 200,000 |
| Coordinator normalized storage | inputs 100,000; normalized 100,000; storage records 100,000 |
| Raw sequence-gap recovery | inputs 100,000; frames 200,000; parsed events 200,000; feed outputs 600,000; normalized 500,000; coordinator actions 100,000; storage records 900,003; produced actions 100,000 |
| Public-trade reprice | inputs 100,000; normalized 100,000; typed intents 400,000; quote intents 400,000; trade-reprice actions 100,000; produced actions 400,000 |
| Bounded biased storm | inputs 100,000; control dequeues 20,000; feed dequeues 80,000; biased-control preemptions 20,000; capacity/high-water 80; saturations 30,000 |

The action benchmark explicitly excludes adapter serialization and delegates
that to the existing adapter-private ignored release test. The PM benchmark
does not use this exclusion to hide PM owner-loop allocations; its exact
included/excluded boundary is frozen in the PM boundary document.

### Raw output hashes

| Output | SHA-256 |
| --- | --- |
| Engine warm-up | `cd8996ae6e5b335c1ac10358e699131e99412a8271c49e1955010fd39b7b36f4` |
| Engine run 1 | `9e265dcc29d7ba273810ee9635a514bfcfbf9911b549bedb679ae17ea1dde049` |
| Engine run 2 | `2d10a1b4fe1c51a041726f13c1792d220c5bdfd131089f3e9daad4de2afeda5d` |
| Engine run 3 | `912232605149252cfb269423de806cc57045ef0ef2111c05e8e26543eab4c5dd` |
| Live warm-up | `3d9893dcd96770abb4470e1caad2459eaf37435e1428f137c38f819dc8a9a78c` |
| Live run 1 | `c80fb65d2a649363d66fc5a58828014b921524ec62f5d3da51163d41d059eacf` |
| Live run 2 | `26bd5fb6241d754da6edeab9680479baba9db7b03f68a84409e620fd4590a5fc` |
| Live run 3 | `2a6fdd807c79ce0b500c201b2acdb14b4d8840bf3897585bf9d4e93d74ba0c4c` |
| Action warm-up | `7dd39cbbff2002989d33c1c7d33ce0a0b22b2c8cbda372820e8ee5f85e5d93f3` |
| Action run 1 | `68184d914df403fe09bba648e5980da4f591349297df5d5347fbc77426a0f2de` |
| Action run 2 | `8a7f9c6fb9dddb8d0c9c77bea46de53426a0ad98b9199d4c60ea8c214ed5443c` |
| Action run 3 | `c94a1643f86b9579a463a72a9745b180df5a60f335c69349ba1e734f1c051057` |

These files are reproducible local evidence under `target/tmp` and are not
tracked source artifacts.

## PM Performance And Replay Contract

Stable later targets:

```text
cargo test -p reap-pm-live --test combined_replay --locked
cargo bench -p reap-pm-live --bench pm_action_path --locked
```

The boundary document predeclares the exact 100,000-observation nominal
workload, 10,000 quote/5,000 cancel lifecycle, 35,000 journal records, fill and
duplicate counts, zero state drops/saturations/steady-state allocations,
64 MiB canonical+queue and 16 MiB replay bounds, exact per-lane high-water and
age rules, and thirteen fresh-state overload cases totaling 27,309 attempts.
The local regression gates
are p50 at most 25 microseconds and p99.9 at most 250 microseconds for every
recorded normalized-event-to-prepared-effect run. This is same-host regression
evidence only.

## Authorized Schema Additions

Goal F may add version 1 of:

- strict secret-free PM configuration;
- PM raw public capture;
- exact PM journal;
- PM deterministic replay projection; and
- PM action-path benchmark output.

Each PM artifact includes product identity, version, configuration and
structural scope fingerprints, exact integral units, deterministic record
sequence, and `production_order_entry_authorized: false`. Deserialization
reconstructs facts only, never executable authority. Existing Chaos/OKX schema
constants, bytes, fingerprints, fixtures, and readers remain unchanged.

## Limitations And Deferrals

- The filesystem is tight and no target host has been selected.
- Phase 0 measurements describe only this local 2-vCPU ARM host.
- No PM network-private session, real account, credential, signature, order,
  fill, or durable production deployment was exercised.
- Official web evidence is a dated protocol snapshot, not a permanent
  compatibility promise.
- The deterministic fixture model to be added later proves composition only;
  it has no production economic validity.
- Goal E target-host work remains deferred and is neither completed nor
  requalified by Goal F.
- The later authenticated-execution goal must re-audit CLOB V2 fields,
  exchange/domain addresses, signer/funder/signature-type behavior, allowance
  spenders, and authoritative private endpoint semantics.
- Production order entry remains unauthorized:
  `production_order_entry_authorized: false`.

## Phase 0 Gate

Phase 0 is green when this handoff and the boundary document pass:

```text
git add docs/multi-venue-polymarket-foundation-goal-f-handoff.md \
  docs/polymarket-product-connectivity-boundary.md
git diff --cached --check
all relative Markdown links resolve
only the two Goal F documentation files differ from HEAD
```

The gate commit is documentation-only. Production edits begin only from its
clean committed state.
