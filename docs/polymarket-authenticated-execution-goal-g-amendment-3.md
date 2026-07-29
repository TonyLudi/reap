# Polymarket Authenticated Execution Goal G Amendment 3

## Defect-Class Adoption, Fresh Phase 0, And Conditional Resume

Status: **user-authorized, conditionally activatable after Goal G-R
Amendment 6 completes**.

The user authorized this amendment on `2026-07-29` to return from the stopped
Goal G-R investigation to the original Goal G without discarding its valid
red evidence or inventing historical facts.

This amendment and its exact recorder/command contract at
`docs/polymarket-authenticated-execution-goal-g-amendment-3-runner-contract.md`
supersede only the Phase 0 activation, evidence/runtime/recorder, source
re-attestation, confidence-campaign, current-baseline, replay, storage,
contamination/no-retry, and completion/return procedures expressly stated
there. Amendments 1-2 and every product, capability, protocol, safety,
historical-evidence, and exclusion boundary remain unchanged.

## Why A Goal G Amendment Is Required

Goal G's original amended Phase 0 selected a clean, valid attempt with two
combined-replay failures. That attempt remains immutable red evidence and
cannot be rerun, replaced, or relabeled.

Goal G-R later proved that both reported errors are secondary-capable:

1. a generic positive service count did not prove that the expected durable
   acknowledgement prepared the matching fake effect; and
2. the capture test discarded a primary writer/shutdown error before running
   the verifier.

Amendment 5 repaired those defect classes in exactly:

```text
crates/reap-pm-live/src/evidence/workload.rs
crates/reap-pm-live/tests/combined_replay.rs
```

The exact historical persistence transition and capture shutdown variant
were not retained. Goal G-R Amendment 6 may validate the prospective repair
without claiming historical equivalence.

Goal G itself previously froze the pre-Amendment-5 PM workload and timed
boundary. Because `workload.rs` now performs exact acknowledgement
observation, the old PM baseline is not a valid current-workload comparator
even though the timed endpoint, report schema, logical projections, and
frozen Goal F anchors remain exact. Goal G must therefore adopt the repair
and establish a new baseline under a separately reviewed evidence root before
Phase 1.

## Activation Preconditions

The authorization package begins from the clean repair tip:

```text
77ad6f30f79eb0b6d99881da97ec94e550364d1a
```

The shared authorization commit is its direct child with exact subject:

```text
docs: authorize goal g-r closure and conditional goal g return
```

Name that exact eight-document authorization commit `A`. The complete
successful revision chain is:

```text
A   shared authorization, direct child of 77ad6f3
R6  Goal G-R completion, direct child of A
G3  Goal G Amendment 3 activation, direct child of R6
P0  Goal G Phase 0 qualification, direct child of G3
```

No intervening commit is permitted. `A` changes exactly the eight paths
listed in Goal G-R Amendment 6; `R6` changes only the Goal G-R handoff; `G3`
changes only the Goal G handoff; and `P0` changes only the Goal G handoff.

Amendment 3 is not active merely because that package is committed. Before
activation require:

1. Goal G-R Amendment 6 ran its complete campaign against `A`;
2. its completion commit `R6` is the direct child of `A` with exact subject
   `docs: close goal g-r defect-class repair`;
3. its handoff says the historical contract remains stopped, defect-class
   validation passed, historical equivalence is not claimed, and Goal G is
   eligible for Amendment 3;
4. every original Goal G and prior Goal G-R evidence hash remains exact;
5. the repository and `../imm-strategy` are clean;
6. Predarb remains at its pinned object and only its previously recorded dirty
   path names are inspected;
7. `Cargo.lock` retains SHA-256
   `2673d055c943c3bd5444531b67df280026c145cbbbc99b68a06f4ac0c2dbb0ff`;
8. repository available space is at least `2,147,483,648` bytes; and
9. no overlapping Cargo, rustc, benchmark, replay, or Reap process exists.

After those checks, construct, no-Cargo self-test, statically review, hash,
and seal the separate pre-activation recorder bundle exactly as specified by
the runner contract. Do not create the evidence or runtime root yet. Append
the bundle identity and exact Goal G-R completion identity to the Goal G
handoff, update its status from authorized-inactive to active Phase 0, and
commit that handoff alone as the direct child of `R6` with exact subject:

```text
docs: activate goal g amendment 3
```

That activation commit is `G3`, changes exactly
`docs/polymarket-authenticated-execution-goal-g-handoff.md`, and makes no
source change. `G3` is the immutable Phase 0 candidate revision. Only after
it exists may the executor create the new evidence and runtime roots.

If recorder construction, self-test, review, sealing, or activation fails,
preserve the exact partial/unsealed bundle byte-for-byte when sealing did not
complete, or the exact sealed bundle when it did. Never relabel a partial
bundle as sealed. Keep the evidence root absent, record
`bundle_state=partial-unsealed|sealed` plus available hashes/inventory in only
the Goal G handoff when the storage gate permits, and commit it as the direct
child of `R6` with exact subject:

```text
docs: record goal g amendment 3 activation stop
```

## Immutable Historical Evidence

Never modify or invoke anything below the original root:

```text
target/tmp/goal-g-phase0-amended
```

Its selected attempt remains:

```text
attempt=1
command_exit=101
evidence_valid=true
gate_pass=false
```

The four named hashes remain:

| Artifact | SHA-256 |
| --- | --- |
| `replay.selected` | `4168ac456d70361429967d7457e0d5850cd014c0b0ea7b8e45e3183372ec766d` |
| `replay/attempt-1/combined-replay.log` | `fe3e8c7323c52163345e6330ebd7587858990a49d1bc436a1a669792f6473cd9` |
| `replay/attempt-1/replay.meta` | `b2dc689182ea8c02fd340669b2b0f142b6cafd15d5ec38a04cda221f3aaa8f56` |
| `replay/attempt-1/replay.ps.tsv` | `fd77e0c1db9970bbe2c20eea70dc8836091a81e77d9bd66491c4d8150f4bf0c3` |

The complete original tree remains at `11,594` files and `12,253` entries,
with file and inventory stream hashes:

```text
35a99a10c133fd680cef1f4e411dbc55490f4e41199411aae907cd348aced340
23c4b85375e2d27e657c38b4560c3ee1bfecae1c1b5c98baf4cf1462dc05f7b2
```

The prior baseline/source evidence remains immutable historical evidence. It
is neither deleted nor rewritten when the new Phase 0 establishes a current
baseline.

The earlier historical `target/tmp/goal-g-phase0` root remains exactly
`4,158` regular files and `5,038` entries, with:

```text
file-hash stream:
ad921fc06db0a68b6e0822208106df2d8c6d276b24d0f4bb342a84f8b738b8d9

type/mode/size/path/link inventory:
4ba698c8804850eeafd3eaef333cf9a6b419d0a66df78a8bd001808eb4d30a4d
```

The complete prior Goal G-R root
`target/tmp/goal-g-replay-repair` remains exactly `70` regular files and
`85` entries, with:

```text
file-hash stream:
54d59957045444e32488a9dda0619440e983b5be779e3004045aac3e68662246

type/mode/size/path/link inventory:
32c47a75092a8a0598f0205e53f495023e80ee6d7279d406059c685401d83171
```

## New Evidence And Runtime Roots

Amendment 3 exclusively owns the pre-activation bundle and post-activation
paths:

```text
target/tmp/goal-g-amendment-3-recorder-bundle
target/tmp/goal-g-phase0-amendment-3
target/tmp/goal-g-amendment-3-runtime
```

All three paths must be absent before bundle construction. Create and seal
only the bundle before `G3`. After `G3`, require the evidence and runtime
paths still absent, verify the activation-sealed bundle inventory/hash, then
create both exactly once. A preexisting evidence/runtime path or a bundle
identity different from the one recorded by `G3` is a stop. The evidence
root is append-only after creation. The runtime root is not evidence; resolve
it to an absolute repository descendant and require it empty before and
after every child. Never create/delete temporary verifier material below the
append-only evidence root; use only the runtime root and append final
logs/manifests to evidence.

Do not copy or invoke the old Phase 0 helper or any old Goal G-R runner. The
new closed-command bundle, layout, modes, interface, exact 86-row command
map, validators, hashes, source successor, baseline helper, replay helper,
and evidence seals are fixed exclusively by the runner contract.

Every attempt retains at minimum:

```text
stdout.log
stderr.log
process.ps.tsv
attempt.meta
attempt.sha256
```

Metadata must include exact command/label/ordinal, start/end UTC,
pre/post revision/tree/status/lock hash, storage, toolchain/host/CPU, runtime
path, child-process ownership, exit, semantic validation, evidence validity,
and gate result. No failed or invalid attempt may be overwritten, discarded,
or retried on an unchanged candidate. Only predeclared external process
overlap detected before result inspection may create an invalid attempt, and
that evidence remains retained.

Every attempt, campaign manifest, baseline summary, replay record, source
record, and inventory record includes:

```text
production_order_entry_authorized=false
real_credentials_loaded=false
authenticated_external_request_sent=false
real_polygon_rpc_request_sent=false
real_order_submitted=false
```

## Storage And Safety Preflight

Immediately before every ignored evidence creation, redirected log, Cargo
command, tracked edit, and commit, run:

```bash
(
  set -euo pipefail
  root=$(git rev-parse --show-toplevel)
  available_bytes=$(df --output=avail -B1 "$root" |
    awk 'NR == 2 {print $1}')
  [[ $available_bytes =~ ^[0-9]+$ ]]
  (( available_bytes >= 2147483648 ))
)
```

Falling below the floor is a stop, not cleanup authority.

All Cargo commands use:

```text
CARGO_NET_OFFLINE=true
TMPDIR=<absolute target/tmp/goal-g-amendment-3-runtime>
```

The credential-free pinned source re-attestation may perform only the exact
public official document/Git/blob reads already authorized by Goal G's
128-row manifest procedure. It must not read credentials or contact an
authenticated endpoint or a real Polygon RPC endpoint.

## Adopted Amendment 5 Cutoff

Amendment 3 accepts the exact Amendment 5 implementation at repair tip
`77ad6f3` as the new Phase 0 starting behavior:

- every persistence acknowledgement is bound to exact immutable counter
  cuts, expected prepared-effect identity, queue transition, and primary
  failure classification;
- facts prepare no fake effect;
- capture prefix verification occurs only after an exact clean
  capture-writer terminal finish; and
- injected failures remain failures and are classified before secondary
  fixture/verifier errors.

This adoption does not authorize any further acknowledgement, product,
report, workload, timeout, queue, or test-semantic change during Phase 0.

`crates/reap-pm-live/src/evidence/workload.rs` is `1,494` lines at the repair
tip. It may not grow. If a later Goal G phase must change it for the
backend-neutral effect migration, first make a separate mechanical
responsibility split that leaves it below `1,400` lines, leaves every newly
extracted production file at or below `1,000` lines, and keeps every function
within the original `200`-line target and `250`-line hard limit. The split
must preserve the exact workload/timed boundary and pass all current
semantic/resource gates. No semantic change may hide inside that extraction.

## Phase 0A: Identity, Sources, And Inventories

Re-run the original Goal G Phase 0 identity and source contracts at the
activation revision:

- current Reap revision/tree/branch/status/remote relation and ancestry;
- clean exact `../imm-strategy` revision;
- pinned Predarb object availability and only its known dirty path names;
- `Cargo.lock`, workspace dependency graph, public exports, schemas, source
  policy, production-content, file/function/state, and fixture inventories;
- Goal D, Goal F, and Chaos anchors;
- all original Goal G and prior Goal G-R evidence hashes;
- exact toolchain, host, CPU, UTC, process, runtime, and storage facts; and
- byte re-attestation of the frozen 128-row official-source manifest.

Moving official replacements may not silently replace pinned bytes. A
protocol conflict or missing pinned source is a stop under the original Goal
G rules.

## Phase 0B: Fixed Defect-Class Confidence Campaign

Run on the clean activation revision, in fresh processes, with immutable
ordinal attempt directories and no retry on an unchanged revision:

1. `cargo fmt --all -- --check`;
2. each original affected test ten times individually with `--exact`,
   `--test-threads=1`, and `--nocapture`;
3. each Amendment 5 regression ten times individually with `--exact`,
   `--test-threads=1`, and `--nocapture`;
4. the complete `combined_replay` binary ten times under default parallelism;
5. the complete binary three times with `--test-threads=1`;
6. `cargo test --locked -p reap-pm-live --all-targets`;
7. `cargo clippy --locked -p reap-pm-live --all-targets -- -D warnings`;
8. `cargo test --locked --workspace --all-targets`;
9. `cargo test --locked -p reap-engine --test decision_replay`;
10. the exact initialized live-reduction decision-parity library test;
11. `cargo test --locked -p reap-pm-core --test numeric_contract`; and
12. the canonical Chaos backtest twice into separate retained outputs.

The exact four test identities are:

```text
phase6_real_mutation_artifacts_recover_to_the_same_bounded_projection
raw_frame_and_raw_count_bounds_are_exact
evidence::workload::tests::real_writer_acknowledgement_is_bound_to_expected_prepared_effect
terminal_capture_finish_preserves_primary_shutdown_error_before_prefix_verification
```

Use the exact command forms frozen by the original Goal G-R Phase 3 contract
for those tests and stable semantic commands. Do not change arguments,
serialize the default-parallel runs, add sleeps, increase a timeout, reduce
the workload, or accept a failure.

Every phase-6/full combined log must contain exactly one report and preserve:

| Anchor | Exact value |
| --- | --- |
| Artifact lines / bytes | `35,012` / `22,791,589` |
| Writer SHA-256 | `83ced509c9ea180e66d957853f9ff7762ef3c0babc316c9251c12d4d1a5224eb` |
| Canonical recovery SHA-256 | `f98bf8a88f34fb6e3c4dcfd1919a2c1d4577b2da3960375e216e596d0746cd35` |
| Recovery peak / limit | `2,959,343` / `16,777,216` bytes |
| Records / last sequence | `35,012` / `35,011` |
| Normalized projection SHA-256 | `3fb6c3c24f2995f57d71be9ba5a4fd36c13ffe956d0ab91bc497370f6259b91a` |
| Byte-identical projection | `true` |
| Production order entry | `false` |

Report extraction must accept the JSON object after an outer libtest prefix;
it may not require column-one JSON.

The two Chaos outputs must be byte-identical and each hash to:

```text
38acf9f5e0c310f2ec5528974beffadf4c1a7f84d46efa8d9664ee7051e84691
```

The four Goal D inputs retain:

| Input | SHA-256 |
| --- | --- |
| `fixtures/decision_parity/risk_initialization_v1.json` | `7e0951c41f447b9f46a73b24a3fe85bdc8f2bb8a623385dab0c3655926e73780` |
| `fixtures/decision_parity/replay_events_v1.jsonl` | `dede17a546d4d717c78dc2b3b7aa7c3f3f785d552404160407c78fb87cec9101` |
| `fixtures/decision_parity/expected_engine_v1.jsonl` | `140c268619b889a19d779e1bdfd340c11901d2eb1d9e4d216d976ba3d8b0d37a` |
| `fixtures/decision_parity/expected_live_reduction_v1.json` | `aa66cc09bba29cde25ab2df66c018517b2c900f83373f95580150e8bcd442b60` |

Any clean failure is selected red evidence for the activation revision and
stops Amendment 3 before a fresh Goal G replay.

## Phase 0C: Fresh Current-Revision Baseline

The old 16-invocation baseline remains historical evidence. Establish a new
coherent baseline on the activation revision by running all four original
benchmark families as four separate serial invocations each:

1. process warmup;
2. retained run 1;
3. retained run 2; and
4. retained run 3.

The families and exact commands remain:

```bash
cargo bench --locked -p reap-engine --bench event_loop
cargo bench --locked -p reap-live --bench live_loop
cargo bench --locked -p reap-live --bench action_path
cargo bench --locked -p reap-pm-live --bench pm_action_path
```

Use a new Amendment-3-owned helper and baseline directory. Preserve the
original overlap, metadata, selector, report-count, logical, allocation,
cardinality, queue, workload, and hash validators. A clean nonzero command is
valid red evidence and cannot be replaced.

All PM invocations must retain the exact non-timing projection SHA-256:

```text
cc90806d19c5d2a252acbd64f3439ece2a0cb1b9d44566b84aa421d8c37b708c
```

The new campaign must also pass a one-time schema and non-timing projection
bridge against the frozen original baseline before it can become the new
comparator. The original PM medians are retained only as historical
diagnostics:

| Quantile | Original median |
| --- | ---: |
| p50 | `23,565 ns` |
| p95 | `45,021 ns` |
| p99 | `57,418 ns` |
| p99.9 | `78,546 ns` |
| max | `176,300 ns` |

No old PM timing value gates the changed workload, and no old-to-new timing
inequality is a Phase 0 pass condition. The bridge requires exact compatible
schema, logical/non-timing projections, family/workload/cardinality fields,
and frozen report hashes as defined by the runner contract. Old engine,
live, and Chaos-action timing values are likewise diagnostic during this
baseline reset; their schema, logical, allocation, queue, and other
non-timing gates remain exact.

The new baseline summary becomes the sole local comparator for Goal G's final
Phase 6/7 campaign. Use the unchanged median-of-three internal runs followed
by median-of-three retained invocation medians. Final p50 and p95 must each
be at most `1.10x` the new baseline; final p99 must be at most `1.20x`.
p99.9 and max remain reported, not shared-host gates. Every non-timing gate
remains exact.

## Phase 0D: One Fresh Goal G Replay

Only after Phases 0A-0C are green, run one new immutable Goal G replay through
the Amendment-3-owned helper. It must run and semantically validate:

- the complete `combined_replay` suite and exact combined report;
- Goal D decision replay and initialized live parity;
- the PM exact numeric contract;
- all four Goal D input hashes; and
- two byte-identical canonical Chaos backtests with the frozen hash.

The helper must require clean identical pre/post repository identity, lock
hash, empty runtime, process ownership, storage, and every frozen anchor. It
creates an immutable attempt and selector only inside the new Amendment 3
root. A clean nonzero command is valid red evidence and stops; it is never
retried or replaced.

The original selected red attempt stays red. A new green attempt proves only
that the current repaired candidate passes the current Phase 0 contract.

## Phase 0 Completion And Resume

Phase 0 is provisionally green only when:

- Goal G-R Amendment 6 is complete;
- all new identity/source/inventory checks pass;
- the fixed confidence campaign is entirely green;
- the fresh 16-invocation baseline is green and sealed;
- the one fresh Goal G replay is selected valid green evidence;
- all original evidence and Goal D/F/Chaos anchors remain exact;
- the new root and every attempt/campaign manifest are sealed;
- the runtime root is empty and no process overlaps; and
- the tracked worktree is clean before the completion record is appended.

Then append exact commands, hashes, revisions, results, non-claims, and new
baseline values to only the Goal G handoff. At that point exactly that handoff
may be modified. Review it, commit it, and require the worktree clean again.

Commit the Phase 0 gate with exact subject:

```text
docs: qualify goal g amendment 3 phase 0
```

That commit is `P0`, must be the direct child of `G3`, and updates exactly
`docs/polymarket-authenticated-execution-goal-g-handoff.md` from active
Phase 0 to Phase 0 green/Phase 1 ready. It does not include implementation
source. Phase 0 is finally green only after `P0` exists and the worktree is
clean.

Only then continue automatically through the original Goal G Phases 1-7
under Amendments 1-3. All original focused commits, phase gates, static
capability boundaries, fake parity, signing/authentication, closed
read-only-source, journal-before-live-mutation, recovery, security, benchmark,
global verification, and documentation requirements remain mandatory.

Phase 1 must recheck the `2,147,483,648`-byte floor immediately before any
implementation edit or build. Phase 0 completion is not cleanup authority and
does not guarantee later storage availability.

## Stop Conditions

In addition to every original Goal G stop condition, stop when:

- Goal G-R Amendment 6 is absent, incomplete, red, or claims historical
  equivalence;
- the activation is not the direct child of the exact Goal G-R completion;
- either old evidence root changes;
- the new evidence/runtime root existed before its authorized post-`G3`
  creation, any attempt path preexists, or the sealed bundle differs from
  the identity recorded by `G3`;
- the adopted two-file repair delta differs;
- the old PM baseline is used as the final comparator;
- any confidence, source, baseline, replay, semantic, hash, process, runtime,
  storage, or worktree gate fails;
- a validation bug appears after an attempt starts;
- passing requires retrying an unchanged candidate, discarding evidence,
  column-one JSON, serialization, sleeps, timeout growth, workload reduction,
  assertion weakening, or golden regeneration;
- `workload.rs` grows before an accepted mechanical split;
- a Phase 0 source or test change is proposed merely to obtain green; or
- progress requires a credential, authenticated production request, real
  Polygon request, real order, target-host decision, production economic
  model, or authority expansion.

At a stop, retain every attempt and record the exact blocker. Do not continue
to the original Goal G implementation phases.

When storage permits, a post-activation Phase 0 stop updates only the Goal G
handoff and is committed as the direct child of `G3` with exact subject:

```text
docs: record goal g amendment 3 phase 0 stop
```

That stop record never authorizes retry on the same candidate.

## Safety And Non-Claims

Amendment 3 authorizes credential-free public source re-attestation,
owner-local loopback traffic, tests, benchmarks, documentation, and—only
after a green Phase 0—the original Goal G library implementation.

It authorizes no real credential, authenticated external request, real
Polygon RPC request, real order, allowance mutation, deployment, target-host
claim, production model, production order entry, sibling edit, or cleanup.

```text
production_order_entry_authorized=false
real_credentials_loaded=false
authenticated_external_request_sent=false
real_polygon_rpc_request_sent=false
real_order_submitted=false
historical_goal_g_attempt_relabelled=false
historical_goal_g_r_equivalence_claimed=false
```
