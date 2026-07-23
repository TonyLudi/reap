# Multi-Venue Polymarket Foundation Goal F Handoff

Status: in progress overall. Phase 0 is green at
`8d6581270b82f39293ccdb0cbeaead42d717e81c`; Phase 1 is green at
`eb71bc1b84cef6152dc010922641e9d5bb019e43`; Phase 2 is green at
`7014a611f997e0bec8e86051d56f333d57776fc1`; Phase 3 is green in the current
working tree and awaits its gate commit. This ledger is architecture and
deterministic local evidence, not authenticated-connectivity evidence or
trading authorization.

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
| 0. Baseline, product contract, dependency and measurement plan | Green | `8d6581270b82f39293ccdb0cbeaead42d717e81c` |
| 1. Exact PM domain and venue-aware envelopes | Green | `eb71bc1b84cef6152dc010922641e9d5bb019e43` |
| 2. Capability-specific venue framework seams | Green | `7014a611f997e0bec8e86051d56f333d57776fc1` |
| 3. PM public market data, integrity, capture, replay | Green (working tree) | Commit pending |
| 4. Read-only private lifecycle and position monitor | Pending | — |
| 5. Passive quote lifecycle and fake execution | Pending | — |
| 6. PM coordinator, quote-model seam, local evidence | Pending | — |
| 7. Documentation, global verification, final audit | Pending | — |

## Phase 3 Public-Wire Boundary Clarification

The configured one-token Polymarket market WebSocket is multiplexed. A raw
object with `event_type = "last_trade_price"` may therefore arrive even though
Goal F has no public-trade requirement. Phase 3 recognizes only that outer
discriminator and drops the object. It has no trade-field DTO/parser, typed
trade value, normalized trade event, plan entry, model input, role, or
trade-specific subscription. Raw capture may preserve the original multiplexed
frame, but replay produces only an ignored discriminator result.

The snapshot field also named `last_trade_price` is not a public-trade input. It
is lexical checksum evidence required by the venue snapshot hash. Phase 3
accepts exact unsigned decimal evidence throughout `[0, 1]`, including terminal
`0` and `1`, without constructing executable `PmPrice`; malformed or
out-of-range evidence fails closed.

Standalone `best_bid_ask` remains a price-only normalized integrity check. Its
optional `bid_size` and `ask_size` wire fields must be absent as a pair or both
validate as exact positive quantities. They are not retained, normalized, or
made into model input.

The Phase 0 boundary document has SHA-256
`861af08783076b5aec2a52f5a351c8a707971902cc58d7f0f1a6ba1795a58a05`
at its green gate.

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

Phase 0 passed:

```text
git add docs/multi-venue-polymarket-foundation-goal-f-handoff.md \
  docs/polymarket-product-connectivity-boundary.md
git diff --cached --check
all relative Markdown links resolve
only the two Goal F documentation files differ from HEAD
```

The documentation-only gate commit is
`8d6581270b82f39293ccdb0cbeaead42d717e81c`.

## Phase 1: Exact PM Domain And Venue-Aware Envelopes

Phase 1 adds one pure leaf package, `reap-pm-core`, and the minimum common
venue-identity change. It does not add a PM adapter, strategy, state reducer,
runtime, transport, credential, signer, authenticated client, or mutation
authority.

### Common identity and legacy fail-close

`reap-core::Venue` now has explicit stable encodings:

```text
Okx        -> "okx"
Polymarket -> "polymarket"
```

The old feed connection and partitioning paths reject
`Venue::Polymarket` with typed `UnsupportedVenue` errors before OKX
subscription serialization or task construction. They contain no PM DTO,
configuration, session, or execution behavior. Golden tests prove the
pre-existing OKX `Venue`, `Subscription`, `RawEnvelope`, and `SystemEvent`
bytes are unchanged.

### Pure PM domain

`reap-pm-core` contains:

- fixed-width structural condition, market, outcome-token, environment,
  chain, signer, funder, account, spender, account-scoped client-order,
  venue-order and fill keys, connection, source, snapshot, and compact
  configured-handle identities;
- an explicit OKX `index-tickers` reference instrument identity with no
  `.PM`/`.PF` suffix parsing and no mutable venue discriminator;
- a checked, sorted, nonempty mapping from one configured PM instrument
  handle to at most 16 configured OKX reference handles, with no pricing
  formula;
- exact heap-free `PmPrice`, `PmTick`, `PmQuantity`, `U256`, signed units,
  tagged ERC-1155 approval, explicit book deletion, JSON-safe order salt, and
  exact side-specific maker/taker amount conversion;
- checked metadata facts for condition/market/token membership, outcome
  label, lifecycle, tick, minimum/lot, chain/domain/exchange identity, and up
  to eight sorted, deduplicated exact spender/asset requirements whose
  outcome-token asset must match the configured metadata token;
- typed market, book, order, fill, balance, allowance, and position events;
- an exact, positive, canonical OKX reference price represented as a U256
  coefficient and decimal scale, plus a source-bound OKX reference event; and
- a statically typed `EventEnvelope<P>` retaining exact source, venue,
  connection, distinct venue/wall/monotonic clocks, connection epoch,
  optional real venue sequence/hash, snapshot revision, and separate local
  ingress sequence.

`PmPrice` represents an exact unapproved candidate in `1..=999_999`
millionths. It becomes tick-valid only through an explicit check against one
of the frozen ticks: `100_000`, `10_000`, `5_000`, `2_500`, `1_000`, or
`100` millionths. Consequently the minimum and maximum supported
tick-aligned prices are `0.0001` and `0.9999`; raw range tests also retain
`0.000001` and `0.999999` as non-executable candidates until checked against
market tick. Quantities are positive U256 protocol micro-units; the fixed lot
is 10,000 units. Decimal parsing rejects signs, exponent notation,
non-representable sub-units, underflow, and overflow. `PmPrice` deliberately
does not implement `Deserialize`, because a wire value has no metadata/tick
context with which to mint an executable price. `0.1`, `0.10`, and
`0.1000000` produce one unit/hash/serialized identity.

The OKX reference value is not a PM probability and carries no quote formula.
It accepts at most 128 input bytes and at most 18 significant fractional
digits, rejects zero/sign/exponent/rounding/overflow, removes coefficient
trailing zeroes, and serializes only as its canonical decimal string. This
closes the exact public-reference handoff needed by the frozen
`reap-okx-public-source` seam without adding another PM-core module.

The maker/taker calculation splits quantity into whole and fractional shares
before checked multiplication. It therefore distinguishes a true
non-integral half-unit from U256 overflow: `U256::MAX * 0.5` is rejected as
non-integral, while `(U256::MAX - 1) * 0.5` succeeds exactly. No lowering
operation rounds.

Unchecked price, tick, and quantity constructors are private.
`PmOrderAmounts` fields are private and the type implements neither raw-tuple
promotion nor `Deserialize`. Domain events themselves are not deserializable;
wire parsing in a later crate must call their checked constructors. The
generic envelope requires the static `PmSourceBound: Sized` contract and
checks exact payload/envelope source equality, including same-venue token and
account mismatches. Order and fill identity cannot be represented by a bare
venue ID: client-order, venue-order, and fill keys retain their exact account
scope, and event constructors reject cross-account combinations. There is no
payload erase/map operation or trait-object source-binding path.

### Dependency and source boundary

Locked metadata reports 24 workspace packages. The only new production
workspace edge is:

```text
reap-pm-core -> reap-core
```

All 23 Phase 0 package edges are unchanged. The complete sorted adjacency has
SHA-256
`7dab46fcdb8a906d541710c44104af6ac51a52e89f136f1460f99f246e9208b8`.
`reap-pm-core` has exactly `reap-core`, `serde`, and `thiserror` as production
dependencies and no binary. Every pre-existing Chaos/OKX package is tested
against acquiring a `reap-pm-core` dependency.

The PM core production-source policy rejects float state, JSON value escape,
filesystem/network/time/runtime clients, credentials/keys/signed requests,
dynamic dispatch, growing containers, shared mutation, unchecked wrapping or
saturating arithmetic, and unsafe code. Ten individually reviewed
compile-fail fixtures prove:

1. unchecked numeric constructors and float promotion are inaccessible;
2. exact maker/taker amounts cannot be forged, deserialized, or field-built;
3. client-order identity representation is private;
4. reference-mapping representation is private;
5. envelope fields and payload types cannot be forged, interchanged, mapped,
   or erased;
6. a modeled external PM-state consumer cannot mint unchecked values;
7. a modeled external PM-strategy consumer cannot import raw/auth types; and
8. Serde cannot mint an off-grid `PmPrice` candidate.

The modeled consumer tests prove the Phase 1 absence claims before the actual
state and strategy crates exist. Their owning phases must repeat the same
dependency, visibility, and authority checks against the real crates.

### Compatibility and structural evidence

The schema/version inventory remains exactly 47 lines with its Phase 0
SHA-256
`9aa8a21c5a8678508c3933de738e26232d0b7ba60882d75cb49c6e25b4bb211f`.
The public declaration inventory grows from 1,433 to 1,517 lines solely for
the authorized common venue/error additions and new PM leaf API; its Phase 1
SHA-256 is
`200fd43e6abd01709838aaf8ea6dbd1aaf746a5a8549004a2a30fcd25881614f`.

The production-source inventory grows from 170 to 177 files and has SHA-256
`b01c0a23fca1fe5015c95c81d1469bdb3693c8e5db9f2d28a532f6bcf3d9e143`.
New production module sizes are:

| Lines | Module |
| ---: | --- |
| 989 | `reap-pm-core/src/numeric.rs` |
| 936 | `reap-pm-core/src/event.rs` |
| 871 | `reap-pm-core/src/identity.rs` |
| 380 | `reap-pm-core/src/metadata.rs` |
| 329 | `reap-pm-core/src/envelope.rs` |
| 95 | `reap-pm-core/src/mapping.rs` |
| 37 | `reap-pm-core/src/lib.rs` |

No new production file exceeds 1,500 lines and no new production function
exceeds 250 lines. Exact-value sizes are 32 bytes for `U256` and
`PmQuantity`, four bytes for `PmPrice`, and require no drop.

The lockfile changes only by adding the new local package with its already
locked normal/dev dependencies. Its Phase 1 SHA-256 is
`4e3e3a8883e5c8b2a057eeb25fc418adca4ac5ad0ca44f923f249ac563782128`.
Every frozen example and Chaos fixture hash remains equal to Phase 0. The
Phase 0 boundary gate remains identified by
`861af08783076b5aec2a52f5a351c8a707971902cc58d7f0f1a6ba1795a58a05`.
Its current Phase 1 SHA-256 is
`14b9316460388cde6aa8c4787e2aba23d91176bc7f1275d51487ea819fb739c2`.
The only boundary-text change is an audited correction of the frozen
module-level edge from:

```text
reap-pm-core::identity -> reap-core::types
```

to:

```text
reap-pm-core::identity -> numeric + reap-core::types
```

`PmTokenId` must own the same exact 256-bit value defined by `numeric`; the
old diagram omitted that real dependency. The correction adds no crate,
capability, module, or runtime edge, and avoids duplicating or weakening the
exact token representation merely to preserve an inaccurate diagram.

### Phase 1 verification

The focused gate passed:

```text
cargo fmt --all -- --check
cargo clippy -p reap-core -p reap-feed -p reap-pm-core \
  --all-targets --locked -- -D warnings
cargo test -p reap-core -p reap-feed --locked --no-fail-fast
cargo test -p reap-pm-core --locked --no-fail-fast
cargo test -p reap-pm-core --test dependency_policy --locked
```

Results were 13 `reap-core` tests, 72 `reap-feed` tests plus its four-case
trybuild suite, and 60 `reap-pm-core` Rust test functions, including one
harness test that passed all ten individually reviewed compile-fail cases and
two dependency/source-policy tests. All passed.

Every pre-existing authority boundary was rerun:

```text
cargo test -p reap-strategy -p reap-venue -p reap-feed -p reap-order \
  -p reap-okx-live-adapter -p reap-live \
  --test compile_fail_boundaries --locked --no-fail-fast
cargo test -p reap-live --test dependency_policy --locked
```

All 23 legacy UI cases and all seven live dependency/source-policy checks
passed.

Deterministic compatibility checks passed:

```text
cargo test -p reap-engine --test decision_replay --locked
cargo test -p reap-live --lib coordinator::tests --locked
cargo test -p reap-storage -p reap-capture --lib --locked --no-fail-fast
```

The engine decision replay passed four tests with three fixture-authoring
helpers ignored; the live coordinator passed 37 tests with one authoring
helper ignored; capture passed 50 tests; and storage passed 31 tests. An
initial attempted integration target named `coordinator_unit` was rejected
because those tests are a path-mounted library test module; the corrected
library filter above exercised the intended suite.

The canonical CLI backtest ran twice. `cmp` returned zero and both outputs
retained the required SHA-256:

```text
38acf9f5e0c310f2ec5528974beffadf4c1a7f84d46efa8d9664ee7051e84691
```

Locked `cargo metadata`, `git diff --check`, the baseline-ancestor check, and
the Phase 0 fixture hash comparison passed. `../imm-strategy` remains clean at
`b6b120c7b7c466d8431bf082f3229328c5d7b2ae`; `../predarb` remains at
`8222273a9c72033b760e1d2fec813bc77144556d` with only its pre-existing
modified dashboard and untracked `.predarb/`. No sibling file or untracked
runtime byte was read or changed.

The independent final diff audit found no remaining blocker. Phase 1 is ready
for its gate commit.

## Phase 2: Capability-Specific Venue Framework Seams

Phase 2 is green in the working tree and awaits its gate commit. It adds
capability-specific framework seams and compatibility-preserving mechanical
extractions only. It does not add a Polymarket wire client, authenticated
session, signer, credential, live order method, production quote model, or
network-enabled PM binary.

### Neutral mechanics and existing-product compatibility

Eight workspace packages are added:

- `reap-transport` owns bounded delivery, reconnect/backoff, connection
  health, monotonic opaque shutdown, and process-shared connection pacing;
- `reap-capture-framing` owns schema-neutral bounded JSONL framing, hashing,
  verification, and writer mechanics;
- `reap-durable-writer` owns a schema-neutral exclusive lease, bounded
  admission, numeric progress, writer-task codec, flush/`sync_data`, and
  durable-result mechanics;
- `reap-okx-public-source` owns only configured OKX `index-tickers` public
  subscription/session/reference behavior;
- `reap-pm-strategy` owns the static pure Goal F model-requirement seam;
- `reap-polymarket-adapter` owns distinct public, private-read,
  reconciliation, account/position, and fake-owned-execution role types;
- `reap-pm-live-contracts` owns secret-free checked connectivity
  configuration, model-requirement translation, and exact capability plans;
  and
- `reap-pm-live` owns the Phase 2 composition roots and deterministic bounded
  lane seam.

The existing facades remain product-specific:

- `reap-feed` delegates only neutral supervision/pacing to `reap-transport`.
  Its public `watch::Receiver<bool>` compatibility API is bridged one-way into
  an opaque monotonic shutdown signal, and sender loss fails closed.
- `reap-venue` delegates only exact legacy index-ticker field extraction to
  `reap-okx-public-source`. It does not expose the new session or subscription
  role, and its legacy normalized value and serialized bytes remain frozen.
- `reap-capture` alone enables the feature-gated
  `legacy-reap-capture` compatibility surface. Default framing reserves a
  tracked worst-case byte slab before serialization, performs a capped
  zero-frame-allocation counting pass and a fixed-capacity second pass, retains
  the exact actual byte charge until the encoded frame is hashed and dropped,
  and decrements evidence before releasing permits. Whole-file verification
  opens one regular-file handle and reads at most `limit + 1`. The workspace
  root and every other crate are denied the explicitly named uncapped legacy
  writer, encoder, and scanner symbols.
- `reap-storage` retains schema 7, its legacy lock bytes, recovery and
  authority rules, and its public error facade while delegating only leased
  writer mechanics to `reap-durable-writer`. The neutral writer acknowledges
  durability only after complete write, flush, and `sync_data`; progress is a
  read-only numeric snapshot and cannot expose recovery or mutation authority.

The OKX public session validates and binds one expected `ConnId`. Every raw
delivery must carry that identity. Only the exact successful subscription ACK
establishes readiness; heartbeat is liveness evidence only. Malformed,
unknown, unsubscribe, zero-count, wrong-connection, and invalid data controls
invalidate readiness. Every reconnect advances a checked connection epoch and
uses session-owned ACK history for backoff. Epoch overflow is terminal and
cannot be cleared by later ACK, data, invalidation, or another failure.

### Exact PM capability and plan boundary

The three roots remain least-authority compositions:

| Root | Reached concrete role kinds | Plan entries |
| --- | ---: | ---: |
| `PmPublicCapture` | 2 | 5 |
| `PmReadOnlyMonitor` | 3 | `8 + N` for `N = 1..=8` exact spenders |
| `PmProduct<Model>` | 6 plus one internal schedule owner | `16 + N`: `15 + N` connectivity plus one internal timer |

The endpoint inventory has exactly 16 stable connectivity purpose IDs.
Fifteen are singleton entries; `PM-ACCOUNT-ALLOWANCE` expands to one exact
entry for each configured spender/asset scope. `QuoteEvaluationTimer` adds one
internal entry owned by `PmPlanOwner::QuoteSchedule`, mapped to the scheduled
lane, with no connectivity role or source/connection route. The product
cardinality is therefore exactly `16 + N`, not a fixed 17. An independent
explicit full-table oracle uses both a collateral ERC-20 requirement and an
outcome ERC-1155 requirement under the same market-selected Standard
domain/exchange (`N = 2`, 18 rows) and checks every
requirement key, full scope, origin, consumer, owner, lane, readiness
dependency, and route. Production positive bindings are derived from the
fields of actually constructed roles; self-attested binding factories are
used only to exercise negative validation.

Public configuration requires one checked `PmReferenceMapping` and two exact
routes: the configured OKX reference source/connection and the configured PM
token source/connection. Account configuration retains the complete
environment, Polygon chain 137, signer, funder, compact handle, instrument,
account source/connection, and sorted exact spender set. Goal F requires the
EOA signer address to equal the funder address. Spenders are bounded by
`MAX_REQUIRED_SPENDERS`, deduplicated, and checked against the full account and
chain. Public capture takes neither a model nor a private/mutation capability.

The explicit model requirements are translated rather than bypassed. The
fixture model names exactly OKX reference, PM metadata, PM book, and
quote-evaluation timer inputs. PM public trade remains absent.

Reconciliation shapes are distinct and bounded:

- atomic complete open orders: at most 1,024 events;
- one exact known-order detail;
- one complete fill page: at most 8,192 events; and
- requested/resulting opaque 32-byte fixture watermarks bound to the complete
  `PmAccountScope`.

No cursor/last-fill ordering is invented. A regression proves that a watermark
with the same compact account handle but a different funder scope is rejected.

`ConstructedRoleBinding::account_snapshot` rejects an adversarial spender slice
before allocation when it exceeds the domain bound. Binding validation rejects
short or long slices before cloning and clones only the exact bounded plan
cardinality.

### Deterministic bounded lane seam

Phase 3 materializes exactly one scheduler container: the private
`PmPublicLaneState` owned by `PmPublicCaptureRun`. It is one preallocated
`BinaryHeap` plus one preallocated key `HashSet`. There is no production
`PmLaneSet`, no `lanes/scheduled.rs`, and no materialized private, account,
position, reconciliation, model, timer, scheduled-action, journal, or
fake-effect lane.

The independent 11-row lane oracle and seven-rank service priority remain
frozen prospective policy for the later atomic scheduler. They pin the
eventual plan-lane mapping, capacity, nominal high-water, maximum age,
saturation action, service burst, and the rule that telemetry alone may
coalesce; they do not claim that 11 runtime containers exist in Phase 3.

Received-event ordering is exactly:

```text
(monotonic_receive_ns, source_handle, source_kind_rank,
 source_scope_ordinal, connection_epoch, local_ingress_sequence,
 variant_rank)
```

Source-kind rank is stable (`OKX reference = 0`, `Polymarket market = 1`,
`Polymarket account = 2`, `internal signal = 3`) and the scope ordinal is the
configured reference, token, or account handle. This prevents identity-class
ordinal collisions. The observation lane, full source, and variant rank are
derived by sealed concrete event implementations. Callers cannot select a
lane, construct a service key, invoke lane mutation through the public
observation trait, or replay a key from one serviced event onto another event.

Public route conversion is owned atomically with the configured PM and OKX
sessions; raw adapter deliveries, session getters, and the route converter do
not escape that owner. Final capability-specific route deliveries are
move-only. Every lane-facing public producer owns its final
authority-forming transition and lane admission as one operation:
authoritative PM metadata is issued and enqueued with no delivery on success,
and an OKX reference reports only `ReferenceEnqueued`. Raw PM capture may
return sealed book/flow obligations, but the Run records their exact order as
pending; callers can discharge them only through
`commit_then_enqueue_pm_snapshot` or `reduce_then_enqueue_pm_book`, which
commit or reduce before enqueuing that same capability. The former commit-only
book APIs are not public.

Public `Full` still returns the exact unconsumed delivery inside a private
move-only proof while the Run retains the matching pending obligation. An age
failure reports non-consuming evidence for the exact current oldest delivery.
The active Run maps authenticated `Full` to the matching owned session's
`Overflow` transition and authenticated `Aged` to `Stale`, using real
detection wall/monotonic evidence and exact current-epoch checks. Before a PM
obligation can escape, the Run begins the matching pending reducer `Overflow`
or `BacklogAged` fault, making composite book readiness unavailable.

Successful Full/Aged enactment performs lifecycle transition, exact-route
purge, reducer finalization where applicable, and admission of the one
must-deliver unavailable occurrence internally. Its public result contains
only copied completion facts: Full retains the rejected ordering, and both
paths retain the venue fault, reducer reason where applicable, and purge
count. It exposes neither the rejected delivery, spent proof/evidence, nor the
newly admitted unavailable delivery.
If that must-deliver notification cannot enter the lane even after the exact
invalidated route is purged, the Run terminalizes and retains a copied
notification-admission failure that `finish` must report.

Explicit disconnect and authenticated PM heartbeat-timeout paths likewise
admit their one unavailable occurrence before returning copied facts.
Unavailable notifications are non-expiring. When one is at the head,
`service_lane_turn` transfers exactly that one occurrence and returns
`Ok(1)`, independent of the ordinary public burst.

Scheduled ordering is a separate exact key:

```text
(monotonic_deadline_ns, action_variant_rank, account_handle,
 token_handle, side_rank, local_action_sequence)
```

This scheduled ordering and the seven-rank cross-lane service rule are
prospective until the later scheduler is introduced atomically with all of
its typed producers. Phase 3 services only its public lane. Its static generic
consumer has five mandatory concrete callbacks and no trait object or
owner-loop allocation. A callback's normal synchronous return commits transfer
of that exact occurrence. An unwind leaves a Run-owned poison flag set;
readiness, later mutation, service, and normal finish then fail closed. A
preflight or per-pop clock failure after earlier successful callbacks returns
the exact prior `Ok(count)` and leaves the failing head for retry.

The lane container is deterministic storage, not a connectivity authorizer.
Phase 3 public producers bind adapter deliveries to their configured
role-issued source/connection route before enqueue; a caller-created
`PmIngressOrder` is not route proof. Phase 4 and Phase 6 must introduce their
private/account/position/reconciliation/model/timer/scheduled-action producers
and the complete scheduler together, preserving the 11-row/seven-rank oracle.
Auxiliary journal and fake-effect markers must likewise be replaced by typed
worker payloads only in the phases that connect those workers.

### Authority and source-policy evidence

Fifteen new Phase 2 compile-fail cases prove:

1. public capture and read-only monitoring cannot obtain mutation;
2. PM role families are not interchangeable and external implementations are
   sealed;
3. the fake owned-execution marker is linear;
4. a product cannot reach OKX private/order authority and requires an explicit
   model;
5. model requirements cannot request private or mutation inputs;
6. all real `PmPlanEntry` fields and fake-profile feature selection are
   private;
7. callers cannot select an observation lane, forge a service key/rank, or
   replay a serviced key into another event; and
8. raw/auth clients and arbitrary venue commands do not escape their owner.

The PM dependency policy checks all four Phase 2 PM crates recursively for
dynamic-dispatch, unbounded-channel, shared-mutation, credential, raw-client,
and broad existing-authority escape tokens. No PM crate reaches
`reap-order`, `reap-live`, `reap-live-contracts`, broad `reap-venue`,
`reap-okx-wire`, any authenticated OKX adapter, `reqwest`, `hmac`, or
`base64`.

Conversely, all 23 Phase 0 packages are checked against acquiring a PM
dependency. The production-source Polymarket occurrence scan recursively
covers all 22 non-core Phase 0 `src` roots and permits exactly the 19
fail-closed legacy feed matches. The six foundational `reap-core` common-venue
identity occurrences are separately and exactly pinned.

Independent audits found and closed:

- reversible/sender-loss shutdown behavior;
- heartbeat-derived readiness and caller-selected reconnect history;
- detachable raw-delivery evidence, permissive controls, unbound connection
  identity, unchecked reconnect epoch, and a nonterminal fatal state;
- frame allocation before byte admission, frame lifetime after permit
  release, evidence/permit races, unbounded verification, and uncapped default
  encoder/scanner escape hatches;
- self-attested positive plan bindings, incomplete plan tables, fake timer and
  constructor ownership, dropped account/source/connection scopes, and
  incomplete reconciliation shapes;
- compact-handle-only fill watermarks;
- forgeable lane/service keys and weak compile-fail fixtures; and
- incomplete legacy source scans and adversarially unbounded public binding
  helpers.

The final independent transport, capture/storage, PM authority, and PM
mechanical re-audits reported no remaining Phase 2 blocker.

### Dependency and structural inventory

Locked metadata reports 32 workspace packages: the 23-package Phase 0 graph,
`reap-pm-core`, and the eight Phase 2 packages. The canonical sorted direct
workspace adjacency has SHA-256
`554a688b3d27f495a9638766f298c2f53a49e399084f181bd5744cad9c4f6f49`:

```text
reap-backtest -> reap-book + reap-capture + reap-core + reap-feed + reap-live-contracts + reap-order + reap-strategy + reap-venue
reap-book -> reap-core
reap-capture -> reap-book + reap-capture-framing + reap-core + reap-feed + reap-telemetry + reap-venue
reap-capture-framing -> -
reap-cli -> reap-backtest + reap-capture + reap-core + reap-emergency-core + reap-fault + reap-feed + reap-live + reap-okx-evidence-adapter + reap-strategy + reap-telemetry
reap-core -> -
reap-durable-writer -> -
reap-emergency-core -> reap-core
reap-emergency-runner -> reap-core + reap-emergency-core + reap-okx-emergency-adapter + reap-order + reap-telemetry
reap-engine -> reap-core + reap-risk + reap-strategy
reap-evidence-core -> -
reap-fault -> reap-core + reap-feed + reap-live
reap-feed -> reap-book + reap-core + reap-transport + reap-venue
reap-live -> reap-core + reap-engine + reap-evidence-core + reap-feed + reap-live-contracts + reap-okx-live-adapter + reap-order + reap-risk + reap-storage + reap-strategy + reap-telemetry + reap-venue
reap-live-contracts -> reap-core + reap-risk + reap-strategy + reap-venue
reap-okx-emergency-adapter -> reap-emergency-core + reap-okx-wire + reap-venue
reap-okx-evidence-adapter -> reap-evidence-core + reap-okx-wire + reap-venue
reap-okx-live-adapter -> reap-core + reap-feed + reap-okx-wire + reap-order + reap-risk + reap-strategy + reap-venue
reap-okx-public-source -> reap-core + reap-transport
reap-okx-wire -> -
reap-order -> reap-core + reap-risk + reap-storage + reap-strategy + reap-venue
reap-pm-core -> reap-core
reap-pm-live -> reap-pm-core + reap-pm-live-contracts + reap-pm-strategy + reap-polymarket-adapter + reap-transport
reap-pm-live-contracts -> reap-pm-core + reap-pm-strategy
reap-pm-strategy -> reap-pm-core
reap-polymarket-adapter -> reap-pm-core
reap-risk -> reap-core
reap-storage -> reap-core + reap-durable-writer
reap-strategy -> reap-core
reap-telemetry -> reap-core
reap-transport -> reap-core
reap-venue -> reap-core + reap-okx-public-source
```

The graph is acyclic. No path dependency points outside the workspace.
`Cargo.lock` changes only in local workspace-package stanzas: it adds the
eight local `0.1.0` packages, records the intended new local edges, and moves
the already locked `libc` mechanics edge from feed to transport. No external
version or checksum changes. Its Phase 2 SHA-256 is
`b1ec49a141b2dfa38ba32f1d76c49031da84eceb360b76007545df632aa20bdd`.

The public-declaration inventory command from Phase 0 now reports 1,711 lines
with SHA-256
`fa7a786967e49825c0c5b5943c9387375ed538f9ab66e511d90060439f6a6dde`.
The schema/version inventory remains exactly 47 lines. Its line-number-bearing
stream now has SHA-256
`22a6987684040d291967cce7f81d5b4c0dcbfe22183de6bb914efbbee7a78583`;
all 47 values are unchanged. The line-number-bearing stream changes because
`CURRENT_SCHEMA_VERSION: u16 = 7` moves within the mechanically extracted
storage facade and unchanged
`CAPTURE_VERIFICATION_FORMAT_VERSION: u16 = 3` shifts from line 20 to line 19.

The sorted production Rust path inventory:

```text
find crates -path '*/src/*.rs' -type f -print | LC_ALL=C sort
```

contains 219 files and has SHA-256
`61e67e21200b51b94a67c28542baf7767a51eff5c830bc95c2cb7a26a93d68c9`.
Hashing the corresponding sorted `sha256sum` manifest yields
`fab339a262a57179888ec61210c465a67cf1809173046af4508bb1a6a98572c7`.

Largest Phase 2 production modules are:

| Lines | Module |
| ---: | --- |
| 1,396 | `reap-capture-framing/src/bounded_writer.rs` |
| 1,025 | `reap-pm-live/src/lanes.rs` |
| 749 | `reap-pm-live-contracts/src/plan.rs` |
| 656 | `reap-transport/src/supervisor.rs` |
| 421 | `reap-pm-live-contracts/src/requirements.rs` |
| 389 | `reap-okx-public-source/src/session.rs` |
| 383 | `reap-durable-writer/src/writer.rs` |
| 352 | `reap-polymarket-adapter/src/reconcile_fixture.rs` |
| 324 | `reap-capture-framing/src/verify.rs` |
| 286 | `reap-okx-public-source/src/public_wire.rs` |
| 260 | `reap-pm-live/src/composition.rs` |
| 260 | `reap-durable-writer/src/bounded.rs` |

No Phase 2 production file exceeds 1,500 lines and no reviewed production
function exceeds 250 lines. The largest PM lane function span is 104 lines.

The normative boundary SHA-256 advances from the Phase 1 gate value
`14b9316460388cde6aa8c4787e2aba23d91176bc7f1275d51487ea819fb739c2`
to
`5007e54a945a969f0b759788ec4d40130e714b23b1280cab0b2c401a66cca65d`.
The audited corrections record the actual transport/capture/storage/venue
delegation edges, the private lane submodules and their real imports, the
16-stable-purpose plan with one allowance entry per exact spender plus one
internal timer, exact OKX connection/epoch semantics, default-bounded versus
feature-gated legacy capture APIs, and the later-phase configured-route
admission gate. They add no authenticated connectivity, mutation authority,
endpoint, or product scope.

### Phase 2 verification

The focused implementation gate passed:

```text
cargo fmt --all -- --check
cargo clippy -p reap-core -p reap-pm-core -p reap-transport -p reap-feed \
  -p reap-okx-public-source -p reap-venue -p reap-capture-framing \
  -p reap-capture -p reap-durable-writer -p reap-storage \
  -p reap-pm-strategy -p reap-polymarket-adapter \
  -p reap-pm-live-contracts -p reap-pm-live \
  --all-targets --all-features --locked -- -D warnings
cargo test -p reap-core -p reap-pm-core -p reap-transport -p reap-feed \
  -p reap-okx-public-source -p reap-venue -p reap-capture-framing \
  -p reap-capture -p reap-durable-writer -p reap-storage \
  -p reap-pm-strategy -p reap-polymarket-adapter \
  -p reap-pm-live-contracts -p reap-pm-live \
  --all-features --locked --no-fail-fast
```

Notable focused results were:

- transport: 16 runtime/structural tests;
- OKX public source: 17 exact session/source/subscription tests;
- framing: 17 feature-enabled tests; capture: 50 tests;
- durable writer: nine tests; storage facade: 24 tests;
- PM live contracts: six plan tests plus three compile-fail cases;
- PM adapter: three role tests plus five compile-fail cases;
- PM live: one composition, five dependency-policy, nine lane, and seven
  compile-fail cases; and
- existing feed and venue suites: 71 and 36 unit tests respectively, plus
  compatibility and compile-fail suites.

All 23 legacy UI cases and all seven existing live dependency-policy checks
passed. The first combined legacy UI attempt completed feed, live, and
OKX-live-adapter successfully, then exhausted the approximately 874 MiB free
filesystem while building additional independent trybuild workspaces. The
remaining order, strategy, and venue suites were rerun individually after
removing only `target/tests/trybuild`; all passed. This was a build-artifact
capacity event, not a test or code failure.

Deterministic compatibility checks passed:

```text
cargo test -p reap-engine --test decision_replay --locked
cargo test -p reap-live --lib coordinator::tests --locked
cargo test -p reap-storage -p reap-capture --lib --locked --no-fail-fast
```

The engine replay passed four tests with three authoring helpers ignored; the
existing live coordinator passed 37 with one authoring helper ignored; capture
passed 50; storage passed 24, with nine extracted neutral-writer tests passing
separately.

The canonical CLI backtest ran twice, `cmp` returned zero, and both outputs
retained:

```text
38acf9f5e0c310f2ec5528974beffadf4c1a7f84d46efa8d9664ee7051e84691
```

All Phase 0 examples, normalized fixtures, decision-parity fixtures, and the
raw OKX reset/gap fixtures retain their exact hashes. The latter two are:

```text
e3475b91fb89040452165d7ba53d0370326fa10a0ff840979b24ad4491018a2b  fixtures/raw/okx/depth-reset.jsonl
90c153851ec63452465eef0e0c6e9c8e718aa70ae49be69e13679fbd529f7fa2  fixtures/raw/okx/depth-gap.jsonl
```

Locked metadata, `git diff --check`, the required baseline-ancestor check, and
the no-outside-path check pass. `../imm-strategy` remains clean at
`b6b120c7b7c466d8431bf082f3229328c5d7b2ae`; `../predarb` remains at
`8222273a9c72033b760e1d2fec813bc77144556d` with only its pre-existing
modified dashboard and untracked `.predarb/`. No sibling file or untracked
runtime byte was read or changed.

## Phase 3: Public Capture And Deterministic Replay

The Phase 3 capture/replay implementation is green in the current working tree
and awaits its gate commit. The complete all-target test gate, 27-case
compile-fail boundary, clippy, check, formatting, source inventory, and
independent authority/obligation audit are recorded below. This is still
deterministic local and fake/loopback evidence only; it adds no authenticated
connectivity or trading authorization.

### Capture contract

`reap-pm-live` owns a version-1, secret-free
`okx_reference_polymarket` JSONL schema. Its required first record binds:

- the exact raw-to-compact observation grant and its configuration
  fingerprint;
- the configured OKX reference and one-token Polymarket identities, sources,
  connections, and routes;
- complete checked PM market metadata, metadata revision/receive clock, and
  recomputable metadata and EIP-712 domain fingerprints;
- initial epochs, last snapshot revision, reconnect, heartbeat, and freshness
  policies; and
- the tracked Predarb provenance commit
  `8222273a9c72033b760e1d2fec813bc77144556d`, seed blob
  `bbb5bc143a914ba8c96d84342321b3dba30ec0fc`, seed SHA-256
  `8e671f14c4b1e8137b1dc1b0bd7d39c79d9c8f961a8483daa32151df99cbdf81`,
  and exact fixture SHA-256.

Both `authenticated` and `production_order_entry_authorized` are fixed false.
The remaining record kinds are exact PM raw frames, exact OKX raw frames, PM
and OKX connection lifecycle events, and deterministic freshness-timer
firings. Raw bytes are captured before parsing and are preserved byte-for-byte
with length, SHA-256, and base64 encoding. The OKX legacy-compatible
`raw_hash` is derived internally from the first eight SHA-256 bytes and cannot
be supplied by a caller. Base64 is a capture-schema-only dependency; the
dependency policy rejects it from PM strategy, contracts, adapter, wire, and
state crates.

The independent limits are:

| Resource | Bound |
| --- | ---: |
| Raw PM or OKX frame | 1 MiB |
| Encoded JSONL record | 1.5 MiB |
| Combined PM and OKX raw-frame count | 8,192 |
| Total record count | 16,384 |
| Cumulative decoded raw payload | 32 MiB |
| Encoded artifact and writer byte queue | 48 MiB |
| Writer record queue | 8,192 |

Every record is measured before admission. The writer separately accounts for
encoded bytes, decoded raw bytes, raw-frame count, and total record count. The
verifier rechecks the exact expected header, recomputes all authoritative
metadata and identity fingerprints, requires contiguous artifact sequence and
per-epoch PM/OKX ingress, validates route/token scope and raw length/hash,
enforces every count and byte bound, rejects partial tails, and requires the
file to remain stable while it is scanned. The neutral scanner opens a
no-follow, nonblocking descriptor, verifies that descriptor is the intended
regular file, enforces the total cap against the same open file, and compares
size, identity, modification, and change-time evidence after the scan.
Deserialized session and reconnect policies are revalidated to the same
invariants as their constructors.

These public schema, verification, and replay APIs are deterministic
unauthenticated offline evidence. They validate bounds, hashes, ordering,
scope equality against the caller-supplied expected header, and semantic
replay; they do not prove that the active composition produced the bytes.
Public `PmCaptureProvenance` is checked pinned-reference metadata, not
active-run provenance. The low-level `PmPublicCaptureWriter` is now
crate-private and absent from the crate-root exports, so only
`PmPublicCaptureRun` owns active artifact construction/mutation. A caller that
constructs public schema values and obtains a successful offline verification
does not gain route, lane, session, or active-run authority.

`PmPublicCaptureRun` is the sole active owner of both venue sessions, writer,
per-epoch raw ingress counters, route authority, and its canonical PM book
reducer. Its PM and OKX ingress paths admit raw bytes to bounded capture before
session parsing and live route conversion. A caller cannot parse an unrecorded
frame or substitute a same-shaped session or reducer through these paths.

Each constructed run allocates a process-unique opaque route-authority seal.
All five move-only public route deliveries, the correlated PM snapshot-flow
value, and the route portion of retained public age evidence carry that exact
seal. It proves which active run issued a delivery. Admission, snapshot commit,
and lane-fault enactment compare it with the owning run, so a
same-configuration sibling run is not interchangeable.

The one private `PmPublicLaneState` owned by the Run allocates its own
process-unique opaque lane-instance seal and maintains a checked generation
that changes on queue mutation. This second seal proves which exact public
lane state observed Full or Aged; it is not derived from the route seal and
cannot stand in for one. No second Phase 3 scheduler container exists. Both
opaque identities are runtime-only authority and are deliberately excluded
from serialization, artifact hashing, canonical replay projection, and
logical ordering.

Raw capture/writer admission, backpressure, byte/count capacity, or storage
failure terminally closes the shared artifact; it is not the recoverable
public-lane `Overflow` path. Raw classification/session parsing and live route
conversion failures are terminal as well. Where exact receive clocks exist,
the run may retain typed unavailable evidence, but normal mutation cannot
continue in the artifact.

The current schema records lifecycle state for both PM and OKX, including
connection start, subscription, disconnect, and reconnect scheduling.
Lifecycle phase and exact epoch are checked before a record is written. The
typed disconnect reasons are:

| Venue | Exact same-artifact disconnect reasons |
| --- | --- |
| PM | `Disconnect`, `Gap`, `Overflow`, `Stale`, `HeartbeatTimeout` |
| OKX | `Disconnect`, `Overflow`, `Stale` |

Reconnect scheduling is admitted only after the matching disconnect record,
and verifier/replay require the exact next epoch and a fresh subscription
before later raw frames. A lifecycle write failure is terminal; a session
transition failure after a successful write is also terminal because the
artifact has already recorded a transition it cannot enact.

PM `HeartbeatTimeout` is accepted only from the owned session's heartbeat
poll after it proves an outstanding ping and expiration of the exact session
deadline, with real receive evidence. It cannot be caller-stamped. Replay
reconstructs that session state: a structurally valid timeout record is
rejected when no ping is outstanding or when it occurs one nanosecond before
the deadline, while the exact deadline is accepted and projects exactly one
`heartbeat_timeouts` and one `external_faults` count. Although schema and
replay retain an explicit PM `Gap` reason, active composition must expose no
caller-selected gap operation. The reducer's ingress is local occurrence
ordering, so a forward jump is legal and does not prove a venue gap. Active
Gap emission remains absent unless a later sealed external detector supplies
real gap evidence.

### Active snapshot and lane authority

`PmPublicCaptureRun::start` constructs a pristine canonical `PmBookReducer`
from the validated product configuration, authoritative metadata, exact
metadata/domain fingerprints, initial epoch, and capture freshness policy.
The run owns that reducer privately for its full lifetime. No constructor or
mutator on the active Run accepts a caller-created reducer, and callers cannot
replace it with a same-configuration sibling.

One atomic `PmPublicCaptureRun` operation consumes the exact sealed snapshot
route delivery and its correlated move-only flow value, applies the snapshot
to that privately owned reducer, and checks the reducer's move-only commit
proof. Instrument, full metadata contract and fingerprint, connection epoch,
metadata/snapshot revisions, local ingress, and exact verified snapshot hash
must all agree before the owned session opens protocol delta flow. The reducer
proof alone is intentionally insufficient authority.

The public readiness boundary is the copied, typed
`PmPublicBookReadiness`, not the reducer's readiness in isolation. It combines
artifact terminality, lifecycle availability, pending book reduction or
terminal-tick cleanup, pending Full/Aged lane faults, reducer readiness, and
session/reducer agreement. Diagnostic revisions remain visible while
`is_ready()` is the only positive readiness signal. Book levels are available
only through `ready_pm_book_view()`, which returns a borrowed
`PmPublicReadyBookView` that cannot outlive or be mutated independently of the
run. Any ordinary captured book reduction, retained terminal-tick cleanup, or
typed Full/Aged obligation suppresses that view even if the reducer retains a
previously valid book.

A public-lane `Full` result retains the exact rejected delivery inside a
private, move-only proof of lane-instance seal, generation, capacity, rejected
key, and policy action. Before that result escapes, the owning run records the
exact typed pending lane obligation; state-bearing PM Full additionally begins
the reducer's matching pending external fault. The Run-owned
`PmPublicLaneState` must consume and authenticate that proof while it is still
the same generation, still Full at the same capacity, and still lacks the
rejected key.

An `Aged` result is non-consuming and retains a separate private, move-only
proof of the typed oldest head, service key, route facts, connection, ordering,
receive clock, original observation time, lane seal, and generation. The
only service path that may inspect a nonempty public lane is
`PmPublicCaptureRun::service_lane_turn`. For evidence issued by that exact
current Run, it registers the typed pending Aged obligation before returning
the evidence and, for a PM metadata/book head, begins the reducer's matching
pending external fault first. The lane state and its raw service operation are
crate-private. Exact enactment consumes Aged evidence and rechecks the same
generation, exact current head, policy action, and original over-age fact.

Sibling-run or sibling-lane evidence, altered route facts, and other
nonmatching Full/Aged evidence are returned untouched. Rejection performs no
lifecycle write and no run, queue, session, or reducer mutation; an already
registered exact obligation remains pending until its own evidence is enacted
or the run terminalizes.

Only after lane-proof authentication does the active-run sequence check the
owned route seal/source, current epoch, detection clocks, and, for PM, reducer
instrument/fingerprint/full-metadata contract. It then records the matching
typed disconnect using the real detection wall/monotonic clocks, applies the
session transition, finalizes the exact pending PM reducer fault where
applicable, and performs a bounded exact purge of only `(route seal, source,
connection, epoch)`. Sibling, unrelated, and later-epoch deliveries remain
queued. It then admits the one routed unavailable notification itself.
Successful Full/Aged results expose only copied completion facts; Full
includes the rejected ordering, while both include fault/reducer/purge facts
as applicable. The new unavailable delivery and consumed proof/evidence do
not escape. An already-unavailable notification retains its original fault
and is re-admitted after the exact purge without minting a second occurrence.
Failure to admit a must-deliver unavailable notification terminalizes the Run,
retains the copied admission failure, and forces
`NotificationAdmissionTerminalFinish` after writer shutdown.

Unavailable notifications bypass age rejection. If one is the current head,
the service turn transfers exactly that notification and returns `Ok(1)`.
Every callback is mandatory and receives a concrete typed occurrence. Normal
synchronous return commits consumption; unwind preserves the Run-owned
consumer-transfer poison, which suppresses readiness and blocks later
mutation, service, and successful finish.

The exact recoverable mappings are:

| Condition | Session transition | PM reducer transition |
| --- | --- | --- |
| PM public-lane `Full` | `Overflow` | `Overflow` |
| PM public-lane `Aged` | `Stale` | `BacklogAged` |
| OKX public-lane `Full` | `Overflow` | Not applicable |
| OKX public-lane `Aged` | `Stale` | Not applicable |

Lane counters separate `rejected_full` and `invalidated_purged`. PM state
separately counts `overflows`, `heartbeat_timeouts`, and
`backlog_aged_faults`. A backlog-age fault increments
`backlog_aged_faults` plus `stale_invalidations`; a deterministic freshness
timer may increment `stale_invalidations` but never
`backlog_aged_faults`.

Tick-size drift remains terminal even if its route delivery would otherwise
be a Full/Aged head. Capture returns the exact move-only delivery, retains its
exact pending reduction identity inside the Run, and marks terminal cleanup
`Pending`; terminalization does not pretend reducer invalidation already
happened. The sole permitted post-terminal mutation,
`apply_terminal_tick_invalidation`, accepts only that exact authenticated
delivery, applies its preserved `old` and `new` values to the run-owned
reducer, and marks cleanup `Applied` without reopening capture or protocol
flow. Mismatched cleanup evidence is returned without mutation. `finish`
drains the writer but reports `TerminalTickCleanupIncomplete` while cleanup is
pending; after cleanup it still returns the ordinary typed terminal
non-success. Tick drift cannot be normalized into a recoverable `Overflow` or
`Stale` transition.

### Artifact fault taxonomy

The shared run/artifact is terminal and must rotate on:

- capture/writer admission, backpressure, byte/count cap, storage, shutdown
  evidence, or lifecycle-write failure;
- raw PM/OKX classification, parser, or session-admission failure;
- live route conversion failure;
- snapshot commit, reducer configuration, correlated-flow, or reducer-proof
  mismatch;
- hash mismatch, invalid transition, or reducer rejection; and
- tick metadata drift requiring refreshed authoritative metadata.

Same-artifact recovery is limited to explicit disconnect, public-lane
Full/Overflow, public-lane Aged/Stale, PM heartbeat timeout, and an explicit
externally proved PM Gap. The pure PM reducer can represent same-epoch snapshot
repair after `InvalidTransition` or `HashMismatch`; this is not active capture
authority. `PmPublicCaptureRun` makes those faults artifact-terminal because a
continued live history would not be reproducible by normal verified replay.
The active Run gates all later mutators after a terminal fault:
capture/classify, lifecycle, reconnect, metadata, snapshot commit, lane
admission/enactment, and timer recording. Read-only path/header/subscription
inspection and terminal shutdown remain possible. The only mutation exception
is the one exact authenticated terminal-tick cleanup described above; it
cannot resume the run. `finish` must drain/close the writer but return typed
non-success; it cannot return a normal verified/replayed outcome.

`reap-pm-live` depends directly on neither legacy `reap-core` nor
`reap-polymarket-wire`. Narrow production helpers in the PM adapter and OKX
public source expose only the parser/session behavior needed by capture and
replay. Tokio Tungstenite is a test-only dependency for the local fake server.

### Replay contract

Replay first verifies the artifact against the caller-supplied expected header,
then reconstructs its PM role, authoritative metadata, session, reducer,
freshness policy, and configured OKX public session from that same header. It
processes records in exact artifact order through the production PM and OKX
parsers and the production PM reducer. The replay scan independently rechecks
the header and must match the verification scan's SHA-256, byte count, record
count, complete tail, and stability evidence before its events can be
attributed to the verified artifact.

During live capture, a PM snapshot opens protocol delta flow only through the
atomic `PmPublicCaptureRun` operation described above. Replay does not consume
the runtime-only route seal; from the verified artifact it performs the
equivalent reducer-before-flow validation. In both paths, synchronous reducer
application must prove all of:

- reducer readiness;
- exact connection epoch and metadata/snapshot revisions;
- the exact snapshot transition ingress; and
- the exact last verified venue snapshot hash.

Protocol flow state is not treated as product readiness. Recoverable
disconnect invalidates the reducer, reconnect advances the epoch, and a new
exact snapshot is required before deltas resume. Tick-size drift projects its
exact `old`/`new` invalidation and terminally requires a new authoritative
metadata artifact; later PM frames or freshness timers fail closed instead of
silently resuming the old grid.

The canonical projection contains ordered PM lifecycle, heartbeat,
snapshot/delta/top, ignored public-trade discriminator, OKX ACK/reference, tick
invalidation, and freshness events plus exact counters and a projection
SHA-256. Replay performs no batch coalescing; the
`integrity_batches_coalesced` counter is pinned to zero.

Current focused assertions pin exact replay counts rather than merely checking
nonzero activity. They are retained by the green Phase 3 gate:

| Scenario | Exact asserted replay/verification counters |
| --- | --- |
| Loopback PM disconnect/resnapshot | verification: PM raw `5`, OKX raw `0`, PM lifecycle `7`, freshness timers `1`; replay: snapshots `2`, resync snapshots `1`, delta batches `1`, delta changes `2`, delta-top confirmations `1`, top confirmations `1`, disconnects `1`, reconnects `1`, coalesced `0` |
| Interleaved PM+OKX | OKX raw `4`, OKX lifecycle `6`, subscription ACKs `2`, references `2`, OKX disconnects/reconnects `1`/`1`; PM snapshots `2`, delta batches `2`, delta changes `4`, coalesced `0` |
| Authenticated heartbeat deadline | heartbeat timeouts `1`, external faults `1` |
| Tick drift | tick invalidations `1`, metadata refresh required `true` |
| Ignored multiplexed trade stress | public trades ignored `64`, coalesced `0`; reserved projection bytes equal event-capacity plus payload-capacity bytes and remain at most `16 MiB` |

The reconnect/local-ingress contract also pins PM and OKX
disconnects/reconnects at `1`/`1`, snapshots at `2`, and PM gaps at `0`, proving
that forward local-ingress jumps do not fabricate a gap.

### Phase 3 evidence gate

The final local gate is green:

| Command | Final result |
| --- | --- |
| `cargo test -p reap-pm-live --all-targets --locked` | Green: 86 Rust tests, 0 failures; the trybuild harness passed all 27 UI cases |
| `cargo test -p reap-pm-state --locked` | Green: 20 tests, 0 failures |
| `cargo test -p reap-capture-framing --locked` | Green |
| `cargo clippy -p reap-pm-live --all-targets --locked -- -D warnings` | Green |
| `cargo check -p reap-pm-live --lib --locked` | Green |
| `cargo metadata --locked --format-version 1` | Green |
| `cargo fmt --all -- --check` | Green |
| `git diff --check` | Green |

The combined suite reaches the required loopback PM subscription,
`PING`/`PONG`, disconnect/reconnect/resnapshot, interleaved PM+OKX capture,
deterministic replay, raw-capture overflow, exact boundary capacities, tick
drift, partial-tail/hash/policy rejection, heartbeat-deadline authentication,
legal ingress jumps without invented Gap, distinct delta-batch evidence, zero
coalescing, and typed terminal finish paths. Same-configuration sibling runs
and sibling lane authority cannot substitute route, snapshot-flow, Full, or
Aged proof. Changed generations, noncurrent heads, altered route facts, and
wrong epochs fail before lifecycle mutation.

The final obligation tests additionally pin the boundary that was previously
implicit:

- metadata issue, OKX reference capture, PM snapshot commit, and PM delta/top
  reduction admit their successful data delivery internally;
- public Full retains the exact rejected delivery only on its move-only
  failure/proof path, while successful Full/Aged enactment exposes copied
  completion facts only;
- explicit disconnect, authenticated heartbeat timeout, Full enactment, and
  Aged enactment admit the must-deliver unavailable occurrence internally;
- unavailable notifications do not expire and a head unavailable notification
  is the sole transfer in an `Ok(1)` turn;
- partial service returns exact prior progress and leaves the failing head;
- normal callback return commits exact transfer, while unwind poisons the
  Run-owned lane and blocks readiness, mutation, service, and successful
  finish; and
- failed must-deliver notification admission terminalizes, retains its copied
  reason, and is reported by terminal finish.

Dependency/source-policy coverage retains direct-dependency and legacy-DAG
rules, base64 confinement, PM-state purity, constructor ownership, and
authority non-escape. Compile-fail coverage keeps raw route/lane/service
authority and `PmPublicCaptureWriter` private, proves every public callback is
mandatory, and proves the former commit-only PM book APIs are not public,
while offline schema/verification/replay remain public.

### Final Phase 3 structural inventory

The reproducible inventories at the green working-tree gate are:

| Inventory | Count | SHA-256 |
| --- | ---: | --- |
| Goal F source-set size audit | 85 files / 31,059 physical lines / 0 files above 1,500 | — |
| Goal F direct dependency edges | 20 edges | `556719df2e183422a487aaafcf80b48a1de06e8ea56e8a4d57cd375725802f28` |
| Sorted workspace public declarations | 1,951 lines | `94c443da52fb2db1fa517b1b0bc0a9addbdd82e5d7936411cdbcba78b97efc4a` |
| Sorted schema/version declarations | 48 lines | `4bbde207bc0279ef90e9a88365567f49332665cafd11f192964483a89a5c6940` |
| Sorted production Rust paths | 255 files | `c01ef82cde5603c676a4c802edd6edba61c739b76f21111f4646e9f42784c77e` |
| Sorted production `sha256sum` manifest | 255 files | `4db8efc7316dd70b496769df5728c8f48402d77c9c808fb30af5dc2fa2cf10bb` |
| Sorted production-source extent stream | 255 files | `373d56ea4842a9a5c30d64ac619e897a3c788d14c6748e0b67288d4caf41cd51` |
| `Cargo.lock` | — | `fcca183e6d1eea4aeb977d4d03232710e98fe56594371d3fdb953854a1a4daf1` |
| Normative connectivity-boundary document | — | `3c756ecfbd3dddce6caf93529ae950a3d72e7f60b2f83d546a7192e85a2e004b` |

These use the Phase 0 public/schema commands, the Phase 2 sorted production
path and content-manifest commands, and the production-extent command recorded
above. The 85-file source-set and 20-edge rows match the canonical boundary
audit; the extent stream and lock hash are supplemental handoff inventories.
Existing pre-Goal-F files above 1,500 lines remain baseline exceptions; no
Goal F production file exceeds 1,500 lines.

The actual `reap-pm-live` production tree has 28 Rust files. Its root contains
`capture`, `capture_roles`, `composition`, `fake_effect`, `lanes`,
`public_routes`, `replay`, and `schedule`, plus the crate facade. Its private
children are:

- `capture::{validation,verify,writer}`;
- `capture_roles::reducer_freshness`;
- `lanes::{bounded,failure,policy,public,service}`; and
- `composition::{lane_enact,run_capture,run_lane_aged,run_lane_full,
  run_lane_service,run_lifecycle,run_reduce,run_state,run_terminal_tick,
  run_types}`.

`lanes::public` contains the sole materialized Phase 3 queue,
`PmPublicLaneState`; there is no `lanes::scheduled` and no aggregate
`PmLaneSet`. `schedule` retains policy types/oracle data, not a scheduled
runtime container.

`capture_roles.rs` is exactly 1,490 production lines, ten below the Goal F
ceiling. It is frozen: Phase 4 and Phase 6 must add no responsibility to that
file. Before any production growth, its role/session, route, reducer, and
outcome responsibilities must be split into narrower modules while preserving
the private authority boundary.

### Required Phase 4 and Phase 6 continuation

Phase 4 remains a read-only PM private lifecycle and position monitor. It must
retain complete account/funder scope, bounded reconciliation shapes, fixture
or fake-only connectivity, and no order mutation. Its typed producers may not
leak an unqueued capability or be attached to ad hoc containers that bypass
the frozen lane policy.

Phase 6 must introduce the complete scheduler atomically with the remaining
typed private/account/position/reconciliation/model/timer/scheduled-action
producers. It must enact the prospective 11-row lane policy and seven-rank
priority as one coherent Run-owned design, preserve non-expiring safety
notifications and exact partial-progress semantics, and prove that all
mandatory callbacks make a total deterministic transition in the sealed
coordinator. Phase 3 proves authenticated ordering and exact occurrence
transfer; it deliberately does not claim the later strategy-state consumption
proof, model behavior, fake quote lifecycle, or production authorization.
