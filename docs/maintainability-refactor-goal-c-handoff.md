# Maintainability Refactor Goal C Handoff

Status: active. Phase 0 is complete and green. Phases 1–5 and final global
verification remain pending. Nothing in this document approves demo trading,
production order entry, authenticated exchange activity, target-host
deployment, or a low-latency runtime redesign.

Prepared: 2026-07-18.

This is the execution record for
[maintainability-refactor-goal-c-prompt.md](maintainability-refactor-goal-c-prompt.md).
The normative authority contract remains
[chaos-connectivity-boundary.md](chaos-connectivity-boundary.md), and the
verified starting structure remains the completed
[Goal B handoff](chaos-connectivity-goal-b-handoff.md).

## Phase Status

| Phase | Status | Result |
| --- | --- | --- |
| Goal C execution contract | Complete | Commit `8fe5fad86ed29fcab5ba80ec3c7ac30e2332bfe0` |
| Phase 0: baseline and inventory | Complete | Green; commit is the documentation commit containing this record and is identified by Git history because a commit cannot self-reference its own SHA |
| Phase 1: Chaos strategy decomposition | Pending | Not started |
| Phase 2: live runtime decomposition | Pending | Not started |
| Phase 3: coordinator reductions | Pending | Not started |
| Phase 4: backtest runner decomposition | Pending | Not started |
| Phase 5: assurance decomposition | Pending | Not started |
| Final global verification | Pending | Not started |

## Phase 0 Reference And Clean State

| Item | Recorded state |
| --- | --- |
| Goal C implementation baseline | `13d5ac17197e758acd5195b58fea4e3440881f9c` |
| Baseline ancestry | `git merge-base --is-ancestor 13d5ac17197e758acd5195b58fea4e3440881f9c HEAD` exited `0` |
| Goal C prompt commit | `8fe5fad86ed29fcab5ba80ec3c7ac30e2332bfe0` |
| Goal B handoff | Present and completed |
| Sibling behavior reference | Clean `../imm-strategy` checkout at `b6b120c7b7c466d8431bf082f3229328c5d7b2ae` |
| Rust behavior pin | `reap_core::PINNED_JAVA_REVISION` is the same full SHA |
| Evidence bindings | Source scan found only the pinned full SHA where a Java revision is embedded |
| Overlapping writers | None; Phase 0 inventory agents were explicitly read-only |
| Authenticated activity | None; no credentials were loaded and no exchange request was made |
| Initial Reap worktree exception | Only the expected untracked Goal C execution prompt, committed before this handoff |

Do not modify `../imm-strategy` during any later phase.

## Structural Baseline

Physical line counts use literal `#[cfg(test)]` item/module ranges as test
lines and every other line as production. This convention is mechanical and
reproducible; it leaves three coordinator doc-comment lines immediately before
a test-only item in the production count.

| Target | Production | Test | Total |
| --- | ---: | ---: | ---: |
| `crates/reap-strategy/src/chaos.rs` | 4,603 | 2,586 | 7,189 |
| `crates/reap-live/src/runtime.rs` | 4,784 | 4,208 | 8,992 |
| `crates/reap-live/src/coordinator.rs` | 1,630 | 2,438 | 4,068 |
| `crates/reap-backtest/src/lib.rs` | 2,280 | 1,264 | 3,544 |
| `crates/reap-live/src/economic_statement.rs` | 4,001 | 1,033 | 5,034 |
| `crates/reap-cli/src/production_evidence.rs` | 3,436 | 451 | 3,887 |

Reproduction command:

```bash
count_ranges() {
  f=$1
  ranges=$2
  awk -v ranges="$ranges" '
    BEGIN {
      n=split(ranges,r,",")
      for(i=1;i<=n;i++){split(r[i],p,"-");lo[i]=p[1];hi[i]=p[2]}
    }
    {
      x=0
      for(i=1;i<=n;i++)if(NR>=lo[i]&&NR<=hi[i])x=1
      if(x)t++;else p++
    }
    END {
      printf "%s production=%d test=%d total=%d ranges=%s\n",
        FILENAME,p,t,p+t,ranges
    }' "$f"
}
count_ranges crates/reap-strategy/src/chaos.rs \
  3757-3767,4615-7189
count_ranges crates/reap-live/src/runtime.rs \
  15-16,21-22,74-75,4791-8992
count_ranges crates/reap-live/src/coordinator.rs \
  771-803,1470-1482,1676-1708,1710-4068
count_ranges crates/reap-backtest/src/lib.rs \
  52-53,899-903,2288-3544
count_ranges crates/reap-live/src/economic_statement.rs \
  4002-5034
count_ranges crates/reap-cli/src/production_evidence.rs \
  3437-3887
```

### Aggregate Size

| Aggregate | Fields | Production methods | Test-only methods | Source |
| --- | ---: | ---: | ---: | --- |
| `ChaosStrategy` | 34 | 47 | 0 | `chaos.rs:1105-3178` |
| `InstrumentState` | 54 | 50 | 1 | `chaos.rs:3181-4234` |
| `LiveRuntime` | 7 | 42 | 0 | `runtime.rs:1389-4126` |
| `LiveCoordinator` | 13 | 44 | 2 | `coordinator.rs:203-1708` |
| `BacktestRunner` | 67 | 48 | 1 | `backtest/src/lib.rs:254-2186` |

Named fields were counted from their exact struct ranges. Direct impl
functions were counted with:

```bash
sed -n 'START,ENDp' FILE |
  rg -c '^    (pub(\([^)]*\))? )?((async|const|unsafe) )*fn [A-Za-z_][A-Za-z0-9_]*'
```

### Largest Production Functions

| Function | Inclusive span | Lines | Current mixed responsibilities |
| --- | --- | ---: | --- |
| `ChaosConfig::validate` | `chaos.rs:200-746` | 547 | Numeric, identifier, instrument, fee, halt, group, coin, membership, and cross-config validation |
| `ChaosStrategy::check_risk_limits` | `chaos.rs:1445-1679` | 235 | Zombie hedge, delta, live orders, turnover, margin, PnL, liability, and balance-sheet halts |
| `ChaosStrategy::new` | `chaos.rs:1143-1285` | 143 | Validation and all aggregate/group/instrument/RNG construction |
| `ChaosStrategy::on_account_update` | `chaos.rs:2976-3108` | 133 | Positions, three balance families, margin, refresh, and delta hedge |
| `ChaosStrategy::on_order_update` | `chaos.rs:2851-2974` | 124 | Quote/hedge lifecycle, fills, turnover, delta, PnL, and anomaly handling |
| `LiveRuntime::build` | `runtime.rs:1400-2037` | 638 | Plan, lease/recovery, host, bootstrap, roles, feeds, gateways, tasks, records, and operator startup |
| `handle_runtime_event` | `runtime.rs:2831-3162` | 332 | Raw feed, connectivity, transport, submit/cancel, remote reconciliation, and fatal events |
| `bootstrap_accounts` | `runtime.rs:549-832` | 284 | Credentials/roles plus account, position, order, fill, clock, status, and instrument bootstrap |
| `run_loop` | `runtime.rs:2213-2400` | 188 | Biased selection, timer/convergence work, readiness, and stop policy |
| `shutdown_inner` | `runtime.rs:3926-4084` | 159 | Task teardown, storage, alerting, order resolution, and error aggregation |
| `LiveCoordinator::process_feed` | `coordinator.rs:456-685` | 230 | Private account/order/fill reduction, canonical identity, ownership, deduplication, and records |
| `process_normalized_at` | `coordinator.rs:1058-1181` | 124 | Startup/readiness transitions, engine call, records, and fail-closed cancellation |
| `route_chaos_intent` | `coordinator.rs:1269-1380` | 112 | Typed purpose routing, gates, approval, ID generation, and local reservation |
| `BacktestCarryState::validate_for` | `backtest/src/lib.rs:325-570` | 246 | Carry schema, config, portfolio, rates, funding, and replay-boundary checks |
| `BacktestRunner::finish_report` | `backtest/src/lib.rs:1932-2154` | 223 | Drain, valuation, completeness, pending actions, carry, metrics, and report |
| `validate_funding_bill` | `economic_statement.rs:3132-3697` | 566 | Funding identity, timing, marks, positions, signs, balances, formulae, tolerances, and samples |
| `validate_trade_bill` | `economic_statement.rs:2506-2879` | 374 | Journal/fill binding, execution identity, fees, interest, balances, and derivative PnL |
| `economic_statement::build_report` | `economic_statement.rs:929-1244` | 316 | Indexing, continuity, dispatch, completeness, counts, issues, and report |
| `evaluate_bindings` | `production_evidence.rs:1591-2323` | 733 | Every cross-artifact identity, account, scenario, config, certification, and economic binding |
| `verify_production_evidence_manifest_path` | `production_evidence.rs:617-1186` | 570 | Secure load/resolve, subordinate verifiers, reopen checks, gates, freshness, bindings, and report |

All other target production functions of at least 100 lines were also
inventoried before movement and remain available from Git history in the Phase
0 task record.

## Dependency Baseline

Target production dependencies are:

| Crate | Direct workspace dependencies |
| --- | --- |
| `reap-strategy` | `reap-core` |
| `reap-live` | `reap-core`, `reap-engine`, `reap-evidence-core`, `reap-feed`, `reap-live-contracts`, `reap-okx-live-adapter`, `reap-order`, `reap-risk`, `reap-storage`, `reap-strategy`, `reap-telemetry`, `reap-venue` |
| `reap-backtest` | `reap-book`, `reap-capture`, `reap-core`, `reap-feed`, `reap-live-contracts`, `reap-order`, `reap-strategy`, `reap-venue` |
| `reap-cli` | `reap-backtest`, `reap-capture`, `reap-core`, `reap-emergency-core`, `reap-fault`, `reap-feed`, `reap-live`, `reap-okx-evidence-adapter`, `reap-strategy`, `reap-telemetry` |

The exact dependency assertions are in
`crates/reap-live/tests/dependency_policy.rs`. In particular:

- strategy workspace dependencies equal only `{reap-core}`;
- backtest reaches `reap-live-contracts` but not `reap-live`;
- live does not reach emergency adapters or raw emergency authority;
- live contracts have an exact pure dependency set; and
- raw command transport remains adapter-owned.

`cargo metadata --locked --format-version 1` exited `0` during Phase 0.

## Public Surface Baseline

These hashes make accidental changes to the current root surfaces easy to
detect:

| Source | SHA-256 |
| --- | --- |
| `crates/reap-strategy/src/lib.rs` | `4e8eeb31f80e3283b327004c6d0b69ecf02eb2402a10750ea826681b681b4151` |
| `crates/reap-live/src/lib.rs` | `3268546423f2bc8bdd209761de22bc61b7961c6835083869384c5a131fde1c2c` |
| `crates/reap-backtest/src/lib.rs` | `9a8112811e574378c2a8cc1e92d81f43539d26ba269c2f3cc9789b321c0d20fa` |
| `crates/reap-cli/src/main.rs` | `eead751e3deed54f783a0ff44a6bdcb33c6b6ba1079c79e61e3d21e58d7b3a80` |

The following root exports/signatures are the required compatibility surface:

- `reap-strategy` retains the exact Chaos root re-export list and `Strategy`
  trait. Public `ChaosStrategy::entity` and `risk_group` signatures expose
  hidden-module types, so moving their definitions must not make those types
  inaccessible or add broader root exports.
- `reap-live` retains exact coordinator, economic statement, and runtime
  re-export blocks. `LiveRuntime` itself remains private; `PreparedLiveRun` and
  the existing run/prepare functions remain public.
- `reap-backtest` retains its existing module re-exports and root-defined
  carry/report/runner API. A moved runner must be re-exported at the identical
  path without gaining a live dependency.
- CLI `production_evidence` remains a private binary module consumed only by
  the existing CLI dispatch.

Target manifest hashes, which MUST remain unchanged:

| Manifest | SHA-256 |
| --- | --- |
| `crates/reap-strategy/Cargo.toml` | `173bfea6e0911dfe4c3fd589e5badd2175c5867e96f98ec51ba66c31093272e8` |
| `crates/reap-live/Cargo.toml` | `5beba5a762c01102698d33771688c9c88967be193542880698d0da71953a86e8` |
| `crates/reap-backtest/Cargo.toml` | `41ae4335cda079b7e9af92cdeb8777cc77318324d636106a8f16c9e9d4c33f93` |
| `crates/reap-cli/Cargo.toml` | `4283a54bb05355deb41e974ffb36a1981c04a5b468b3d2f6506c6551183f61b2` |

## Source, Visibility, And Authority Guard Baseline

`crates/reap-live/tests/dependency_policy.rs` lexically scans production
sources and compares exact owner sets. Relevant current owners are:

| Token/operation | Exact current production owners |
| --- | --- |
| `OwnedRegularOrders` | live coordinator; order authority and explicit order root export |
| `.reserve_local(` | live coordinator; order authority |
| `.register_recovered(` | live coordinator; order authority |
| `ProvenRegularSubmitRequest` | live coordinator, live runtime, order authority, storage root |
| `.take_approval_scope(` | live runtime, live adapter root, order gateway |
| `.take_command_dispatcher(` | live runtime dispatch module, order gateway |
| `.start_and_install(` | exactly once in live runtime |
| Private bootstrap bind/login validation | exactly the live adapter root |

Moving one of these seams requires a one-for-one exact path replacement. The
owner-set cardinality may not increase and no directory or wildcard may be
admitted.

The runtime one-owner regression currently scans the runtime facade and six
responsibility modules. It requires:

- exactly one `coordinator: LiveCoordinator`;
- all six explicit responsibility-state fields;
- no `Arc<Mutex<_>>` in production runtime ownership; and
- no `use super::*` in responsibility modules.

Every new runtime production module must be added to that scan.

Important Phase 2/3 test-splitting constraint:
`production_rust_source` currently truncates only the literal co-located
`#[cfg(test)]` followed by `mod tests`. A separate `src/**/tests.rs` file would
be scanned as production even when its parent declaration is test-only.
Test extraction must retain an exact production/test distinction without
broadening an authority allowlist or hiding production source.

Relevant compile-fail suites:

- strategy: infrastructure independence and unforgeable typed intents;
- live: no broad authenticated authority and no public regular actions;
- order: opaque/linear authority, no raw gateway, and storage-proven recovery;
- feed: private/linear bootstrap and private connection seam;
- live adapter: role isolation, unsupported-mutation rejection, private and
  linear mutation authority; and
- venue: no raw authentication, transport, or outbound websocket builder.

`TRYBUILD=overwrite` is forbidden by the Goal C contract.

## Deterministic Baseline

| Artifact | SHA-256 |
| --- | --- |
| `Cargo.lock` | `d8a19fb100aeb4e542a2135d546edfb5ae24629717f5ab65e285cf9bfe483b02` |
| `fixtures/normalized/chaos_quote_hedge.jsonl` | `27f2eb4b9dba7ee600ed645ad8b7c88143e8b54531232991b492cb7595e8ccaa` |
| `fixtures/normalized/chaos_quote_hedge_later.jsonl` | `40453b8be283178b20531c84142dbaeeeca82b4723e5c13594df171c778cd8ee` |
| `fixtures/normalized/chaos_quote_hedge_intents.json` | `d95fa7f121e2e8c402c8108cf9fefb7c7d7b3dbd2b9742c58c234a521f0ee0ec` |
| `examples/iarb2-basic.toml` | `0fac5a3a35fe28cdc05118b7e22241077aa7f604a9a5436355797605b51b3b26` |
| `examples/live-okx-demo.toml` | `caea78e0a75d2586ecbd16d5b4414f9606a7064b6e1684f82fff2d132a197195` |
| Canonical pretty CLI backtest | `38acf9f5e0c310f2ec5528974beffadf4c1a7f84d46efa8d9664ee7051e84691` |

The canonical backtest command was run twice. `cmp` exited `0`, and both files
had the recorded hash.

Focused Phase 0 checks:

| Command | Result |
| --- | --- |
| Exact typed ordered-intent fixture | 1 passed; 73 filtered |
| `reap-live` dependency policy | 3 passed |
| Runtime single-owner responsibility test | 1 passed; 229 filtered |
| Locked Cargo metadata | Exit `0` |

## Performance Baseline

Environment:

- Rust `1.95.0 (59807616e 2026-04-14)`;
- LLVM `22.1.2`;
- `aarch64-unknown-linux-gnu`;
- 2 vCPU Arm Neoverse-N1, one thread per core;
- Linux `7.0.0-1004-aws`; and
- one unrecorded warm-up followed by three recorded runs on the same host.

### Engine Event Loop

| Run | ns/event | Intents |
| --- | ---: | ---: |
| Warm-up | 11,367.5 | 999,996 |
| Recorded 1 | 11,649.0 | 999,996 |
| Recorded 2 | 11,360.3 | 999,996 |
| Recorded 3 | 11,458.4 | 999,996 |
| Recorded median | 11,458.4 | 999,996 |

### Complete Live-Parity Observe Path

| Run | ns/raw frame | Allocation calls/raw | Requested bytes/raw | Parsed | Feed outputs | Records | Actions |
| --- | ---: | ---: | ---: | ---: | ---: | ---: | ---: |
| Warm-up | 17,850.2 | 83.53 | 37,286.9 | 50,204 | 70,208 | 65,130 | 0 |
| Recorded 1 | 16,900.0 | 83.53 | 37,286.9 | 50,204 | 70,208 | 65,130 | 0 |
| Recorded 2 | 17,079.4 | 83.53 | 37,286.9 | 50,204 | 70,208 | 65,130 | 0 |
| Recorded 3 | 17,113.2 | 83.53 | 37,286.9 | 50,204 | 70,208 | 65,130 | 0 |
| Recorded median | 17,079.4 | 83.53 | 37,286.9 | 50,204 | 70,208 | 65,130 | 0 |

Exact complete-path totals were `4,193,771` allocation calls and
`1,871,951,969` requested bytes per run.

Recorded stage medians:

| Stage | Median ns/unit | Allocation calls/unit | Requested bytes/unit |
| --- | ---: | ---: | ---: |
| Wire parse and raw record | 2,878.0 | 33.33 | 3,158.5 |
| Dedup, sequence, and book | 7,688.1 | 13.36 | 26,875.8 |
| Coordinator, strategy, risk, and records | 3,985.5 | 26.34 | 5,186.1 |
| Complete live parity | 17,079.4 | 83.53 | 37,286.9 |

The current measurements differ from the older historical checkpoint in
`performance.md`, which predates later Goal B changes. The three Phase 0 runs
have identical logical and allocation counters, so these current numbers are
the authoritative Goal C before-values. Later phases compare against this
table. The historical performance document will be reconciled during final
documentation; no performance implementation change is authorized by this
observation.

## Phase 0 Exit Gate

Phase 0 is green because:

- both repositories and the Java pin match the required baseline;
- the only initial Reap change was the expected Goal C prompt;
- target responsibilities, sizes, public surfaces, dependencies, authority
  guards, and compile-fail suites are inventoried;
- deterministic hashes and the canonical CLI output are exact;
- focused authority/determinism tests pass;
- current same-host performance counters and medians are reproducible; and
- Phase 0 changed documentation only.

No production source may move until this Phase 0 record is committed.

## Pending Completion Evidence

Each later phase must append:

- exact phase commits and before/after responsibility measurements;
- focused commands and results;
- public-surface, dependency, deterministic, source-policy, and compile-fail
  comparisons;
- required post-phase benchmark runs;
- any justified structural exception or explicit deferral; and
- the final full-workspace verification and clean-tree evidence.
