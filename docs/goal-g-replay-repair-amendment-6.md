# Goal G-R Amendment 6: Defect-Class Validation And Goal G Return

Status: **user-authorized validation-and-closure overlay**.

The user authorized this amendment on `2026-07-29` after Amendment 5's
two-file repair and one-pass verification completed successfully. The
interactive verification output is useful diagnostic evidence, but it was
not retained as an append-only repository-owned campaign and the tracked
handoff still records Amendment 5 only as authorized. This amendment creates
that durable validation record without changing the repair.

This amendment supersedes only the post-stop recorder/evidence procedure,
fixed defect-class validation campaign, and completion/return semantics
expressly stated here. Every historical fact, preservation anchor, safety
boundary, and non-claim from the earlier contracts remains controlling.

## Frozen Starting Identity

The repair tip is:

```text
77ad6f30f79eb0b6d99881da97ec94e550364d1a
```

with tree:

```text
9273cead973ecdd687ae11fa51d666f638e4a426
```

and subject:

```text
test(pm): anchor capture regression tempdir
```

At authorization, `master` and local `origin/master` both point to that
commit, the tracked worktree is clean, and the frozen `Cargo.lock` SHA-256 is:

```text
2673d055c943c3bd5444531b67df280026c145cbbbc99b68a06f4ac0c2dbb0ff
```

The Amendment 5 authorization commit is:

```text
2f94e72b9636cdef69b9530fa6ac37626360864f
```

The complete tracked implementation delta from that authorization through
the repair tip is exactly:

```text
crates/reap-pm-live/src/evidence/workload.rs
crates/reap-pm-live/tests/combined_replay.rs
```

The four focused implementation commits are:

```text
eabbb2f fix(pm): bind replay acknowledgements to prepared effects
6c4c7f4 test(pm): anchor replay regression tempdir
44b26ff test(pm): isolate acknowledgement cycles
77ad6f3 test(pm): anchor capture regression tempdir
```

The authorization package must be the direct child of the repair tip and use
the exact subject:

```text
docs: authorize goal g-r closure and conditional goal g return
```

Its tracked changes are exactly these eight documentation paths, with no
rename or mode change:

```text
docs/goal-g-replay-repair-amendment-6.md
docs/goal-g-replay-repair-handoff.md
docs/polymarket-authenticated-execution-boundary.md
docs/polymarket-authenticated-execution-goal-g-amendment-3.md
docs/polymarket-authenticated-execution-goal-g-amendment-3-runner-contract.md
docs/polymarket-authenticated-execution-goal-g-handoff.md
docs/polymarket-authenticated-execution-goal-g-prompt.md
docs/polymarket-authenticated-execution-goal-g-resume-prompt.md
```

Its exact name-status map is:

```text
A	docs/goal-g-replay-repair-amendment-6.md
M	docs/goal-g-replay-repair-handoff.md
M	docs/polymarket-authenticated-execution-boundary.md
A	docs/polymarket-authenticated-execution-goal-g-amendment-3.md
A	docs/polymarket-authenticated-execution-goal-g-amendment-3-runner-contract.md
M	docs/polymarket-authenticated-execution-goal-g-handoff.md
M	docs/polymarket-authenticated-execution-goal-g-prompt.md
A	docs/polymarket-authenticated-execution-goal-g-resume-prompt.md
```

Amendment 6 execution must start from that clean committed authorization
revision.

Name that full authorization commit `A`. The repair tip remains `77ad6f3`;
`A` is the validation candidate and differs from it only by the eight
authorized documentation paths. Every runtime report must contain
`build_revision == A`. The successful commit chain is:

```bash
A=$(git rev-parse HEAD)
candidate_tree=$(git rev-parse HEAD^{tree})
```

Compute and verify both exact values before root creation. Immediately after
the owned root/layout is created, make `recorder/candidate.meta` and
`recorder/candidate.meta.sha256` the first evidence files: the former is a
sorted `key=value` record binding candidate head/tree/parent/subject and
repair tip/tree, and the latter is its one-line SHA-256. They are covered by
every subsequent recorder review/hash check.

```text
A   shared authorization, direct child of 77ad6f3
R6  Goal G-R completion, direct child of A
G3  Goal G Amendment 3 activation, direct child of R6
P0  Goal G Phase 0 qualification, direct child of G3
```

No intervening commit is permitted.

## Completion Meaning

The original Goal G-R historical-causality contract remains stopped. The
retained Goal G failure did not preserve:

- the exact persistence transition, prepared-effect guard, or queue-front
  state behind the later fake-effect mismatch; or
- the exact `TerminalFinish.shutdown_error` variant behind the later
  `InvalidRecords`.

The retained evidence and tests authorized here cannot authenticate those
erased historical facts. Amendment 6 therefore does not claim that either
Amendment 5 regression reproduces the historical failure. Its prospective
defect-class completion replaces the old historical-equivalence completion
condition; the original historical investigation remains stopped.

Amendment 6 may conclude:

```text
original historical Goal G-R contract: stopped
historical mutation transition: unknown, not retained
historical capture shutdown variant: unknown, not retained
defect-class repair: validated or failed
historical equivalence: not claimed
```

A green result returns authority only to Goal G's separately reviewed
Amendment 3. It does not relabel the old Goal G attempt, make the old Phase 0
green, activate Goal G, or complete any Goal G implementation phase.

## Tracked And Runtime Authority

Amendment 6 validation may update only:

```text
docs/goal-g-replay-repair-handoff.md
```

It may create ignored, append-only validation evidence only below:

```text
target/tmp/goal-g-r-amendment-6
```

It must use the repository-owned runtime directory:

```text
target/tmp/goal-g-replay-repair-runtime
```

resolved to an absolute path before passing it as `TMPDIR`.

Before evidence-root creation, require that root to be absent, the runtime
root to be an existing empty directory, both paths/parents to resolve inside
the repository, and neither path nor any existing ancestor below
`target/tmp` to be a symlink. The evidence root may be created once only.

It must not edit source, tests, dependencies, fixtures, schemas, reports,
capacities, timeouts, queue policies, either Goal G document, a sibling
repository, any prior Goal G-R evidence, or the frozen Goal G evidence root.

The Amendment 4 v5 runner and all v1-v5 artifacts remain immutable. Never
invoke, copy, patch, replace, or reinterpret v5. Never invoke the frozen Goal
G Phase 0 replay helper.

## Immutable Evidence

For each complete evidence root named below, compute the two aggregate hashes
with these exact definitions after substituting its repository-relative path:

```bash
root=<evidence-root>

(
  cd "$root"
  find . -type f -print0 |
    LC_ALL=C sort -z |
    xargs -0 sha256sum |
    sha256sum
)

(
  cd "$root"
  find . -mindepth 1 -printf '%y\t%m\t%s\t%P\t%l\n' |
    LC_ALL=C sort |
    sha256sum
)
```

Before runner creation, before every validation attempt, and after the
campaign, verify the four named Goal G artifacts:

| Artifact | SHA-256 |
| --- | --- |
| `replay.selected` | `4168ac456d70361429967d7457e0d5850cd014c0b0ea7b8e45e3183372ec766d` |
| `replay/attempt-1/combined-replay.log` | `fe3e8c7323c52163345e6330ebd7587858990a49d1bc436a1a669792f6473cd9` |
| `replay/attempt-1/replay.meta` | `b2dc689182ea8c02fd340669b2b0f142b6cafd15d5ec38a04cda221f3aaa8f56` |
| `replay/attempt-1/replay.ps.tsv` | `fd77e0c1db9970bbe2c20eea70dc8836091a81e77d9bd66491c4d8150f4bf0c3` |

The complete `target/tmp/goal-g-phase0-amended` tree must remain at `11,594`
regular files and `12,253` entries, with:

```text
file-hash stream:
35a99a10c133fd680cef1f4e411dbc55490f4e41199411aae907cd348aced340

type/mode/size/path/link inventory:
23c4b85375e2d27e657c38b4560c3ee1bfecae1c1b5c98baf4cf1462dc05f7b2
```

The historical `target/tmp/goal-g-phase0` tree must remain at `4,158`
regular files and `5,038` entries, with:

```text
file-hash stream:
ad921fc06db0a68b6e0822208106df2d8c6d276b24d0f4bb342a84f8b738b8d9

type/mode/size/path/link inventory:
4ba698c8804850eeafd3eaef333cf9a6b419d0a66df78a8bd001808eb4d30a4d
```

The Goal F combined-replay anchors remain:

| Anchor | Exact value |
| --- | --- |
| Artifact lines | `35,012` |
| Artifact bytes | `22,791,589` |
| Writer SHA-256 | `83ced509c9ea180e66d957853f9ff7762ef3c0babc316c9251c12d4d1a5224eb` |
| Canonical recovery SHA-256 | `f98bf8a88f34fb6e3c4dcfd1919a2c1d4577b2da3960375e216e596d0746cd35` |
| Recovery peak | `2,959,343` bytes |
| Recovery records | `35,012` |
| Last sequence | `35,011` |
| Projection | byte-identical |
| Production order entry | `false` |

The normalized combined report projection, after deleting only
`build_revision`, `rustc`, and `host`, remains:

```text
3fb6c3c24f2995f57d71be9ba5a4fd36c13ffe956d0ab91bc497370f6259b91a
```

The complete prior Goal G-R root
`target/tmp/goal-g-replay-repair` remains at `70` regular files and `85`
entries, with:

```text
file-hash stream:
54d59957045444e32488a9dda0619440e983b5be779e3004045aac3e68662246

type/mode/size/path/link inventory:
32c47a75092a8a0598f0205e53f495023e80ee6d7279d406059c685401d83171
```

## Storage And Process Preflight

Immediately before every ignored evidence creation, redirected validation
log, Cargo command, tracked edit, and commit, run:

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

Insufficient storage is a stop, not cleanup authority.

Before every command require:

- `HEAD == A`, `HEAD^{tree} == candidate_tree`, and a clean tracked tree;
- `A^ == 77ad6f30f79eb0b6d99881da97ec94e550364d1a`;
- `77ad6f3..A` changes exactly the eight authorization paths above;
- `2f94e72..77ad6f3` changes exactly the two implementation paths above;
- unchanged `Cargo.lock`;
- an empty absolute runtime root;
- no Cargo, rustc, `combined_replay`, `pm_action_path`, Reap, or Reap CLI
  process outside the recorder's child tree; and
- unchanged frozen evidence hashes.

The runtime root must also be empty after every child. Retained residue is a
stop and may not be deleted merely to continue.

Process detection must use executable/argv identity plus PID ancestry, not a
substring-only `pgrep -f` check that can match the recorder itself. Reject
the following inherited variables before the campaign, recording names but
never values:

```text
REAP_PHASE6_RECOVERY_EVIDENCE_CHILD
REAP_PRIVATE_BATCH_VALIDATION_ALLOCATION_CHILD
REAP_PHASE6_OVERLOAD_ALLOCATION_CHILD
REAP_GOAL_D_CANCEL_PROBE_CHILD
RUST_TEST_THREADS
RUST_TEST_NOCAPTURE
TRYBUILD
CARGO_TARGET_DIR
```

## Validation Recorder

The fixed campaign name is `validation`. Create this exact layout once:

```text
target/tmp/goal-g-r-amendment-6/
  <A>/
    recorder/
      run-validation.sh
      run-validation.sh.sha256
      commands.tsv
      commands.tsv.sha256
      validators.sh
      validators.sh.sha256
      candidate.meta
      candidate.meta.sha256
      self-test/
      review-1.txt
      review-2.txt
    validation/
```

The top-level root and `<A>` directory begin mode `700`. Before the first
attempt, recorder scripts are mode `500`, hashes/map/self-test outputs and
review reports are mode `400`, and the recorder subtree is sealed against
content changes with its directory mode `500`. The `validation` directory is
mode `700` only while append-only attempts are being created, then mode `500`
after the campaign; the final `<A>` and top-level directories are also mode
`500`. Each review report names a distinct reviewer/session, binds
the exact runner, validator, and command-map hashes, and contains an explicit
pass/fail checklist. Verify owner, modes, and hashes before every invocation.

The recorder must:

- map ten closed labels to the exact commands below;
- accept only the expected committed candidate, campaign, ordinal, and label;
- require every lower ordinal attempt to exist, have a valid seal, and record
  `gate_pass=true`, and require every current-or-higher attempt path to be
  absent before launch;
- require the live `validation/` inventory to contain exactly the expected
  lower-ordinal sealed directories and no unknown file, directory, or link;
- create a previously absent attempt directory for every label;
- retain separate `stdout.log`, `stderr.log`, `process.ps.tsv`,
  `attempt.meta`, and `attempt.sha256`;
- record start/end UTC, exact command, pre/post revision/tree/status/lock
  hash, storage, toolchain, host, CPU count, runtime path, process overlap,
  exit status, validator result, and evidence-valid/gate-pass states;
- include all seven safety booleans in the final block of this document in
  every attempt metadata file and the campaign manifest;
- sample process ownership during every child;
- stop after the first invalid or failed attempt;
- never select, discard, replace, overwrite, or retry an outcome; and
- after all ten pass, rerun every repository/lock/evidence/runtime/process
  preservation check, then and only then create the sorted pass manifest and
  seal the campaign/root.

Before any Cargo execution, run and retain `bash -n`, closed-label/command-map
inspection, hash attestation, no-Cargo validator fixtures, and at least two
independent static reviews of the recorder. This is a static pre-execution
review, not another mutable v5 bootstrap protocol.

The retained no-Cargo fixture matrix must prove: all ten exact mappings
accept; unknown label, wrong ordinal, wrong candidate, and each forbidden
environment variable reject before child launch; prefix and column-one
single reports accept; truncated, duplicate, non-whitespace-suffixed, wrong
build-revision, and wrong-projection reports reject; command 04's exact
outer/child/two-summary shape accepts while extra children reject; command
06's nested child plus final `14 passed` shape accepts; and command 08 passes
with exactly the inherited child combined report. It must also prove
out-of-order launch,
prior-red continuation, and a preexisting later attempt reject. The self-test
must also reject an unknown live-inventory entry and must record zero
Cargo/rustc processes and zero attempt directories.

Its closed labels and ordinals are:

| Ordinal | Label |
| ---: | --- |
| 01 | `fmt-check` |
| 02 | `ack-regression-exact` |
| 03 | `capture-regression-exact` |
| 04 | `mutation-original-exact` |
| 05 | `capture-original-exact` |
| 06 | `combined-default` |
| 07 | `pm-live-lib` |
| 08 | `pm-live-all-targets` |
| 09 | `pm-live-clippy` |
| 10 | `compile-fail-boundaries` |

Each attempt path is exactly:

```text
target/tmp/goal-g-r-amendment-6/<A>/validation/<ordinal>-<label>
```

The recorder must accept a JSON report that follows a libtest prefix on the
same line. Report extraction must locate the literal object prefix
`{"schema_version":1,"target":"combined_replay"` anywhere in a line and
extract from that byte onward, parse exactly one complete JSON object, and
reject any non-whitespace suffix. Requiring JSON to begin in column one
repeats the known v5 validator defect and is a stop.

The process sampler must be terminated and waited for before sealing an
attempt. Create `attempt.sha256` exactly as a relative-path SHA-256 manifest:

```bash
(
  cd "$attempt"
  find . -type f ! -name attempt.sha256 -print0 |
    LC_ALL=C sort -z |
    xargs -0 sha256sum
) >"$attempt/attempt.sha256"
```

After validation, regular attempt files become mode `400` and the attempt
directory mode `500`; a sealed attempt is immutable. There is no mutable
selector. The final `campaign.tsv` contains ordinal, label, attempt-relative
path, and `sha256sum attempt.sha256` in ordinal order as four tab-separated
columns with no header.
`campaign.tsv.sha256` is the one-line `sha256sum campaign.tsv`.
`campaign.meta` is a bytewise key-sorted `key=value` record binding schema,
candidate head/tree, campaign, expected/actual count, result, first/last UTC,
and all seven safety booleans from the final block; its
`campaign.meta.sha256` is the one-line SHA-256. All four campaign files
become mode `400`. Compute the final whole-root file stream and
type/mode/size/path/link inventory only after the root is final, and record
those two hashes outside the ignored root in the tracked handoff.

## Fixed Ten-Command Campaign

Resolve:

```bash
root=$(git rev-parse --show-toplevel)
repair_tmp="$root/target/tmp/goal-g-replay-repair-runtime"
```

Every command is a fresh process with:

```text
CARGO_NET_OFFLINE=true
TMPDIR=<absolute repair_tmp>
```

Run each exactly once, in order:

```bash
cargo fmt --all -- --check

cargo test --locked -p reap-pm-live --lib \
  evidence::workload::tests::real_writer_acknowledgement_is_bound_to_expected_prepared_effect \
  -- --exact --test-threads=1 --nocapture

cargo test --locked -p reap-pm-live --test combined_replay \
  terminal_capture_finish_preserves_primary_shutdown_error_before_prefix_verification \
  -- --exact --test-threads=1 --nocapture

cargo test --locked -p reap-pm-live --test combined_replay \
  phase6_real_mutation_artifacts_recover_to_the_same_bounded_projection \
  -- --exact --test-threads=1 --nocapture

cargo test --locked -p reap-pm-live --test combined_replay \
  raw_frame_and_raw_count_bounds_are_exact \
  -- --exact --test-threads=1 --nocapture

cargo test --locked -p reap-pm-live --test combined_replay -- --nocapture

cargo test --locked -p reap-pm-live --lib

cargo test --locked -p reap-pm-live --all-targets

cargo clippy --locked -p reap-pm-live --all-targets -- -D warnings

cargo test --locked -p reap-pm-live --test compile_fail_boundaries \
  -- --test-threads=1
```

Commands 02, 03, and 05 must each run exactly one top-level named test and
report one pass. Command 04 intentionally launches the isolated recovery
child: require exactly one top-level selected test, exactly one expected child
execution of the same name, two successful one-test summaries, and exactly
one combined report. Treating its expected child as a second top-level match
is a validator defect.

Command 06 must run all `14` current combined tests under default
parallelism, report `14 passed; 0 failed`, and emit exactly one combined
report. Commands 04, 06, and 08 must each yield exactly one combined report
whose `build_revision` equals `A` and which satisfies every frozen anchor and
projection hash above. A successful Cargo exit without successful semantic
validation is red.

For each report-producing attempt, extract exactly one complete JSON object
using a prefix-aware decoder, reject any non-whitespace suffix, require
every original combined-report invariant plus `.build_revision == A`, and
produce the normalized projection exactly with:

```bash
jq -S 'del(.build_revision, .rustc, .host)' "$report"
```

Command 07 must finish with the outer `167 passed; 0 failed` library summary
and exactly two visible inherited-stdio one-test child summaries, for:

```text
evidence::overload_tests::batch_validation::repeated_private_batch_validation_is_allocation_free
lanes::phase6_overload_tests::allocation::thirteen_pm_live_overload_mechanisms_are_allocation_free
```

Command 08 must require those same two library children plus the one expected
combined mutation child/report; none is an extra outer selection.

Command 10 must run exactly the outer test
`composition_boundaries_are_enforced_by_the_type_system` and finish with one
passing outer libtest summary. The validator permits trybuild's expected
inner `test ... ok` lines only as inner compiler-fixture output; it must not
count them as additional outer tests.

## Completion Handoff

After all ten attempts, run the final read-only preservation checks while the
campaign directory remains owned and appendable. Only if they pass may the
recorder create the pass manifest and seal the root. After that seal, append
a complete execution record to `docs/goal-g-replay-repair-handoff.md`,
including:

- authorization/candidate/repair identities, candidate tree,
  authorization parent/subject, exact eight-path authorization delta, and
  this contract's SHA-256;
- recorder path, hash, review results, and closed map;
- every attempt path, command, exit, validator result, and attempt hash;
- campaign file and inventory hashes;
- exact allowlisted implementation diff;
- `Cargo.lock`, Goal F, Goal G, and prior G-R preservation results;
- runtime/process/storage results; and
- these exact semantic fields:

```text
goal_g_r_amendment_6_execution_status=complete
goal_g_r_amendment_6_campaign_status=passed
goal_g_r_amendment_6_candidate_head=<A>
goal_g_r_amendment_6_candidate_tree=<candidate-tree>
goal_g_r_amendment_6_completed_evidence_root=target/tmp/goal-g-r-amendment-6
goal_g_r_amendment_6_completed_evidence_regular_files=<decimal>
goal_g_r_amendment_6_completed_evidence_entries_excluding_root=<decimal>
goal_g_r_amendment_6_completed_evidence_file_stream_sha256=<64-lowercase-hex>
goal_g_r_amendment_6_completed_evidence_inventory_sha256=<64-lowercase-hex>
goal_g_r_original_historical_contract_status=stopped
goal_g_r_historical_mutation_transition_status=unknown-not-retained
goal_g_r_historical_capture_shutdown_variant_status=unknown-not-retained
goal_g_r_resolution_scope=defect-class-only
goal_g_r_amendment_5_validation_status=passed
goal_g_r_defect_class_resolution_status=complete
goal_g_r_historical_equivalence_claimed=false
goal_g_r_goal_g_return_status=eligible-for-separately-reviewed-amendment-3
goal_g_r_goal_g_resumed=false
```

Those five completed-evidence keys occur exactly once in the handoff. Counts
and hashes use the exact `find` definitions in this amendment after final
modes are applied; `entries_excluding_root` is the `find . -mindepth 1`
count. `R6` may not be committed without all five.

Preserve the existing single
`goal_g_r_regression_contract_status=pending` field unchanged as historical
state. Do not rewrite it to imply the original contract passed.

Commit the completion record with exact subject:

```text
docs: close goal g-r defect-class repair
```

That commit is `R6`, must be the direct child of `A`, and may change only
`docs/goal-g-replay-repair-handoff.md`. Amendment 6 is complete only at `R6`.
Goal G remains stopped until its Amendment 3 is separately activated from
that completion commit.

If the campaign stops red, append the exact retained stop record to only the
Goal G-R handoff and commit it as the direct child of `A` with subject:

```text
docs: record goal g-r amendment 6 stop
```

That stop commit does not authorize `G3`.

If and only if this campaign successfully created and owns the A6 root and
the storage preflight still permits new bytes, write `<A>/stop.meta` with a
versioned `key=value` schema containing candidate, stage (`preflight`,
`static-review`, `self-test`, or `validation`), ordinal and label or `none`,
reason code, child exit or `not-started`, validator result, attempt seal or
`none`, all preservation results, and all seven safety booleans. Write
`stop.sha256` as the one-line SHA-256 of `stop.meta`; both are mode `400`.
First terminate and wait for the recorder-owned Cargo/test process group,
every descendant, and the sampler. Only after all have exited may the
recorder seal every retained attempt, `validation`, `recorder`, `<A>`, and
the root to mode `500`, then compute final whole-root file/inventory hashes.

For a stop before owned-root creation, an unexpected preexisting/unowned
root, or a failing storage gate, do not create metadata there and never touch
that root. Record only read-only observations externally in the tracked
handoff when the storage gate later permits. If an owned child/process cannot
be proven exited, retain the partial root without a false seal and record
that exact condition externally.

When storage permits the handoff-only stop record, it must include candidate
head/tree, failed ordinal/label, child exit, validator result, attempt seal,
all preservation results, and:

```text
goal_g_r_amendment_6_execution_status=stopped
goal_g_r_amendment_6_campaign_status=failed
goal_g_r_goal_g_return_status=blocked
goal_g_r_goal_g_resumed=false
```

Creating that documentation commit does not reset no-retry eligibility.
Once a validation child starts on `A`, neither a documentation/recorder
change nor a new commit permits the campaign to be retried on the same
repair/product tree.

## Stop Conditions And Safety

Stop without another attempt on the unchanged candidate when:

- repository identity, ancestry, worktree, allowlisted diff, or lock hash
  differs;
- storage is below the exact floor;
- runtime residue or an overlapping process exists;
- any prior evidence byte, mode, path, count, or hash changes;
- any command or semantic validator fails;
- any command deviates from its expressly stated test/child cardinality;
- commands 04, 06, or 08 yield zero or multiple combined reports, or any
  other command unexpectedly exposes a combined report that its validator
  does not allow;
- a recorder defect appears after execution starts;
- completion would require source, dependency, fixture, timeout, workload,
  assertion, capacity, or authority changes; or
- anyone attempts to equate an injected variant with a historical failure.

Amendment 6 authorizes no credentials, authenticated request, real Polygon
request, signing, order placement/cancellation, external mutation,
deployment, target-host claim, cleanup, sibling change, timeout increase,
retry, reduced workload, global serialization, or assertion weakening.

```text
production_order_entry_authorized=false
real_credentials_loaded=false
authenticated_external_request_sent=false
real_polygon_rpc_request_sent=false
real_order_submitted=false
goal_g_red_evidence_modified=false
goal_g_resumed=false
```
