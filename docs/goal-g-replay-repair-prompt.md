# Goal G-R: Combined-Replay Determinism Repair Prompt

Status: **proposed separately scoped prerequisite; not started**. Goal G
remains stopped on its immutable amended Phase 0 replay result. This goal does
not amend Goal G, create Amendment 3, resume Phase 0, or make the stopped
campaign green.

## Runnable Goal Prompt

> /goal Execute Goal G-R exactly as specified in
> `docs/goal-g-replay-repair-prompt.md`. Diagnose and, only if the proven
> repair is confined to the frozen evidence harness or test isolation, fix
> the two named combined-replay failures. Preserve Goal G's immutable red
> evidence and every frozen Goal F semantic and artifact anchor. Stop only at
> Goal G-R completion or a documented stop condition.

## Objective

Explain the exact causal mechanism behind both failures selected by Goal G's
clean Phase 0 replay:

1. `phase6_real_mutation_artifacts_recover_to_the_same_bounded_projection`
   returned
   `Invariant("PM fake-effect script does not match the next prepared effect")`
   in its isolated recovery child; and
2. `raw_frame_and_raw_count_bounds_are_exact` received `InvalidRecords` from
   `verify_pm_public_capture`.

Prove each cause with deterministic state/lifecycle evidence, add a regression
that exposes the diagnosed pre-repair defect, and make the smallest
evidence-harness/test-isolation repair that removes timing luck without
weakening a contract. A regression for a deliberately injected durability
fault passes only by proving the exact primary error is preserved; it must not
turn that fault into success. The two failures may have separate causes.

Goal G-R completes only when the repair is confined to the exact
evidence-harness/test allowlist below, every required fresh-process and global
gate is green with no discarded result, all frozen Goal F anchors remain
exact, and the handoff records the causal proof. If any live-product or other
production implementation change is required, stop after diagnosis and
propose the smallest production-repair goal. Do not widen Goal G-R in place.

## Frozen Starting Evidence

The required ancestor and stopped-run record are:

```text
4da8b43126e1b270758224ffa9f2bbe9f239f79d docs: record goal g phase 0 replay stop
```

The proposed-goal commit must be the direct child of that stop commit, have
the exact subject `docs: propose goal g replay repair`, and change exactly:

```text
docs/goal-g-replay-repair-prompt.md
docs/polymarket-authenticated-execution-goal-g-handoff.md
```

At Goal G-R start, require clean `HEAD` to be that direct child. Record its
full ID as the immutable Goal G-R base. Any intervening or additional commit
is an unexplained starting-state stop.

The Goal G Amendment 2 pre-gate commit is:

```text
66a6213301f9c9677f8137f545c11cfc0ff3c065 docs: freeze goal g amendment 2 contract
```

The frozen `Cargo.lock` SHA-256 is:

```text
2673d055c943c3bd5444531b67df280026c145cbbbc99b68a06f4ac0c2dbb0ff
```

Goal G's ignored evidence root is
`target/tmp/goal-g-phase0-amended`. It is read-only evidence for Goal G-R.
These files must exist and match before any diagnosis:

| Artifact | SHA-256 |
| --- | --- |
| `replay.selected` | `4168ac456d70361429967d7457e0d5850cd014c0b0ea7b8e45e3183372ec766d` |
| `replay/attempt-1/combined-replay.log` | `fe3e8c7323c52163345e6330ebd7587858990a49d1bc436a1a669792f6473cd9` |
| `replay/attempt-1/replay.meta` | `b2dc689182ea8c02fd340669b2b0f142b6cafd15d5ec38a04cda221f3aaa8f56` |
| `replay/attempt-1/replay.ps.tsv` | `fd77e0c1db9970bbe2c20eea70dc8836091a81e77d9bd66491c4d8150f4bf0c3` |

The metadata must still say attempt 1, exit 101,
`evidence_valid=true`, `gate_pass=false`, identical clean pre/post repository
identity, and no external process overlap. Never rerun
`target/tmp/goal-g-phase0-amended/run-phase0-replay.sh`.

Freeze the entire original evidence tree with these exact definitions:

```bash
root=target/tmp/goal-g-phase0-amended

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

The tree has 11,594 files and 12,253 entries. The resulting file-hash stream
and type/mode/size/path/link inventory SHA-256 values are:

```text
35a99a10c133fd680cef1f4e411dbc55490f4e41199411aae907cd348aced340
23c4b85375e2d27e657c38b4560c3ee1bfecae1c1b5c98baf4cf1462dc05f7b2
```

A read-only comparison already found that the failed test file, capture
verifier, and coordinator mutation implementation are unchanged from the
known-passing Goal F commit
`d16c3cbdac97fb43944e3a97d4f9b56e92206747`. Only the authorized PM
latency-policy bench, runner branch, and policy test differ under
`crates/reap-pm-live`. That narrows investigation but does not prove a race or
permit a test-only assumption.

## Authority Boundary

Goal G-R may:

- read tracked Reap source, tests, and documentation;
- hash the entire retained Goal G evidence tree read-only without interpreting
  file contents, and read the four replay files above plus selected baseline
  metadata needed to authenticate their context;
- create ignored diagnostic evidence only under
  `target/tmp/goal-g-replay-repair`;
- create process-owned runtime temporary files only under
  `target/tmp/goal-g-replay-repair-runtime`;
- edit `crates/reap-pm-live/tests/combined_replay.rs`;
- edit existing files below `crates/reap-pm-live/tests/support/` only when the
  causal proof requires a test-local deterministic lifecycle/synchronization
  primitive;
- edit `crates/reap-pm-live/src/evidence/workload.rs` only when the causal
  proof shows that its real-writer driver accepts a generic positive service
  count without proving the expected durable intent/fact or prepared-effect
  identity; and
- create/update `docs/goal-g-replay-repair-handoff.md`.

No other tracked path is authorized. In particular, Goal G-R may not edit any
other `src/**`, `Cargo.toml`, `Cargo.lock`, fixture, schema, benchmark, limit,
capacity, timeout, queue policy, expected artifact, Goal G document, sibling
repository, or production configuration. `workload.rs` remains a closed
evidence driver: it may gain exact acknowledgement/effect-identity
observation and diagnostics, but no product policy or report projection may
change. If a correct repair needs another change, stop and document why.

The ignored repair root may contain reviewed shell helpers, immutable raw
logs, metadata, process snapshots, hashes, and extracted reports. It may not
contain source copies, credentials, environment dumps, or mutable
pass/fail selectors that discard earlier outcomes.

## Safety And Non-Claims

Goal G-R authorizes no secret loading, external network request, Polygon
request, authentication, signing, live/authenticated order construction,
order placement, cancellation, allowance change, external mutation,
deployment, target-host tuning, push, or sibling-repository change. Existing
owner-local loopback TCP/WebSocket test traffic and fixture-only prepared fake
effects remain authorized.

It must not:

- delete, rename, rewrite, truncate, touch, or replace anything below
  `target/tmp/goal-g-phase0-amended`;
- create a new Goal G selector or evidence root;
- call the Goal G Phase 0 replay helper;
- claim the existing valid-red attempt was contaminated;
- claim Goal G or its Phase 0 is resumed, passed, repaired, or complete;
- create or adopt Amendment 3;
- retry until green, discard a failure, or select only passing repetitions;
- weaken, remove, invert, ignore, filter out, or reclassify a negative result
  as passing, acceptable, invalid evidence, or contamination; correcting a
  secondary error to the exact primary negative error remains required;
- add sleeps, increase a timeout, add retry/backoff, reduce a workload, raise a
  bound, accept a partial write, or suppress a shutdown/durability error;
- serialize the entire test binary, add a blanket/global mutex, or require
  top-level `--test-threads=1` as the repair; or
- change the existing isolated recovery child into an in-process test merely
  to alter scheduling.

Serial runs are diagnostic comparisons only. Default-parallel fresh-process
results are required acceptance evidence.

## Phase 0 — Identity, Storage, And Evidence Preservation

Start from a clean worktree on `master`. Record, without fetching:

- full `HEAD`, `HEAD^{tree}`, branch, local `origin/master` relation, and
  status;
- `Cargo.lock` hash;
- Rust/Cargo versions, host/kernel, CPU count, and UTC time;
- all worktrees and any existing Cargo, rustc, combined-replay, or Reap CLI
  processes;
- available filesystem bytes; and
- the exact changed-path comparison from Goal F commit
  `d16c3cbdac97fb43944e3a97d4f9b56e92206747` through current `HEAD`.

The stopped evidence is immutable even when ignored by Git. Hash the four
files and the complete tree/inventory streams before any other action and
again at every phase gate and completion. Any mismatch is an immediate stop.

Use one private temp root on the repository filesystem for every Cargo
command; never use the host's undersized `/tmp` tmpfs. Require at least
`2,147,483,648` available bytes on the repository filesystem before creating
the repair evidence/temp root, redirecting a fixed log, running Cargo, or
editing a tracked file. Run this exact fail-safe preflight immediately before
each such action and again before every build/global gate:

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

Insufficient storage is a stop, not cleanup authority. Never delete
`target/tmp`, a build cache, user data, or sibling data without separate
explicit approval.

Only after that check, set:

```bash
root=$(git rev-parse --show-toplevel)
repair_root="$root/target/tmp/goal-g-replay-repair"
repair_tmp="$root/target/tmp/goal-g-replay-repair-runtime"
mkdir -m 700 "$repair_root" "$repair_tmp"
```

Every Cargo invocation uses
`CARGO_NET_OFFLINE=true TMPDIR="$repair_tmp"`. Do not start while another
matching process exists. Once the repair evidence root is created, it is
append-only for attempts: every pre-fix and post-fix outcome remains recorded
and hashed.

The runtime temp root is not evidence and is not append-only. It must be empty
before and after each child process. If a child leaves any entry, retain it,
record a read-only type/mode/size/path/hash inventory in that attempt, and
stop without cleanup or another invocation. At successful completion remove
only the empty goal-owned runtime directory with `rmdir`; never copy its
transient contents into an accepted evidence report.

Before the first invocation, create one runner under the repair root, review
it against this prompt, record its SHA-256 in the handoff, and never change it.
It must bind each label to one exact command from the fixed matrices. It may
accept only an expected committed `HEAD`, campaign name, ordinal, and a
predeclared command label; arbitrary command text is forbidden.

Every invocation must create a previously absent directory:

```text
target/tmp/goal-g-replay-repair/<candidate-head>/<campaign>/<ordinal>-<label>/
```

and retain:

```text
stdout.log
stderr.log
process.ps.tsv
attempt.meta
attempt.sha256
```

`attempt.meta` records schema, campaign/ordinal/label, exact command, start/end
UTC, pre/post `HEAD` and tree, pre/post `Cargo.lock` hash, pre/post empty
status blocks, repository available bytes, fixed temp-root path,
toolchain/host/CPU, command exit, evidence-valid state, gate-pass state,
reason, validation result, and process-overlap result. The runner snapshots
`pid/ppid/comm` before, at least once per second during, and after the child;
matching includes Cargo, rustc, `combined_replay`, `pm_action_path`,
`decision_replay`, `reap_live`, `numeric_contrac`, and Reap/Reap-CLI names.
Any match outside the invoked process tree invalidates the attempt.

The universal names above are minimum required files, not an exclusive list.
Each attempt retains split `stdout.log` and `stderr.log`; label validators may
also create an extracted report and validation log. After label-specific
validation and metadata are final, `attempt.sha256` covers every regular file
in the attempt directory except itself using sorted relative paths. A
cross-attempt check such as backtest byte comparison is stored and hashed in
an append-only campaign manifest. An invalid or failed attempt remains
immutable. There is no mutable selector.

Create the repair handoff with:

```text
production_order_entry_authorized: false
real_credentials_loaded: false
authenticated_external_request_sent: false
real_polygon_rpc_request_sent: false
real_order_submitted: false
goal_g_red_evidence_modified: false
goal_g_resumed: false
```

Phase 0 is green only when repository identity, storage, process isolation,
and every stopped-evidence hash are proven. Commit the initialized repair
handoff as a focused documentation-only attestation so all recorded
executions can start from a clean tree.

## Phase 1 — Read-Only Causal Analysis

Trace each failure from its exact failing line through:

- the test-local resource identity and lifetime;
- asynchronous writer/journal ownership and shutdown completion;
- persistence acknowledgement identity, ordering, and reduction;
- prepared fake-effect kind and queue state;
- raw-capture preflight, accepted records, terminalization, writer shutdown,
  and verification;
- filesystem paths and temporary-directory ownership; and
- parent/isolated-child plus default test-harness concurrency.

For each failure, record an evidence table containing:

| Field | Required finding |
| --- | --- |
| Observed state | Exact state immediately before the failed assertion/call |
| Expected state | Exact invariant the test assumes |
| Divergence | First transition where observed and expected differ |
| Shared resource | Exact process/task/file/queue/lifecycle coupling, or proof there is none |
| Scheduling dependence | Why an allowed ordering can expose the divergence |
| Deterministic trigger | Barrier, injected ordering, or fixed state sequence that reproduces it |
| Repair owner | Test body, existing test support, or the explicitly allowlisted evidence driver; never inferred live-product ownership |

Do not label the cause “race,” “slow host,” “filesystem,” or “flaky test”
without this transition-level proof. Inspect writer shutdown errors rather
than accepting a broad terminal error match. Do not infer the fake-effect
front kind from a generic positive service count.

Before editing, run a fixed predeclared diagnostic matrix in fresh Cargo
processes under the new repair evidence root:

1. each named test once individually with exact-name selection and
   `--test-threads=1`;
2. the complete `combined_replay` binary once with `--test-threads=1` as
   comparison only; and
3. the complete binary exactly three times under default parallelism.

The four command forms are exactly:

```bash
env CARGO_NET_OFFLINE=true TMPDIR="$repair_tmp" \
  cargo test --locked -p reap-pm-live --test combined_replay \
  phase6_real_mutation_artifacts_recover_to_the_same_bounded_projection \
  -- --exact --test-threads=1 --nocapture

env CARGO_NET_OFFLINE=true TMPDIR="$repair_tmp" \
  cargo test --locked -p reap-pm-live --test combined_replay \
  raw_frame_and_raw_count_bounds_are_exact \
  -- --exact --test-threads=1 --nocapture

env CARGO_NET_OFFLINE=true TMPDIR="$repair_tmp" \
  cargo test --locked -p reap-pm-live --test combined_replay \
  -- --test-threads=1 --nocapture

env CARGO_NET_OFFLINE=true TMPDIR="$repair_tmp" \
  cargo test --locked -p reap-pm-live \
    --test combined_replay -- --nocapture
```

The evidence helper may wrap one exact command for metadata/log capture but
may not alter its arguments, environment beyond fixed diagnostic labels, or
exit status.

Record all six outcomes, stdout/stderr, exit codes, process snapshots, and
pre/post repository identity. These are a fixed investigation matrix, not
retry-until-green. No outcome may be discarded. A failure need not recur
naturally if the subsequent causal trigger is deterministic; natural passes
do not invalidate the selected Goal G red evidence.

Phase 1 is green only when both failures have causal explanations and
deterministic pre-repair triggers. If either explanation remains
timing-probabilistic, stop. Record and commit the causal analysis before
editing the repair. In particular, if deterministically controlling the
durable receipt/front ordering requires a new production test hook outside
the allowlist, stop and propose that exact hook in a narrower reviewed goal;
post-hoc aggregate validation is not a causal trigger.

The causal-analysis commit must freeze every new regression's exact full test
name and whether it is an integration test in `combined_replay.rs` or a
private library unit test in the allowlisted evidence driver. Then create,
review, and hash a second immutable regression runner under the repair
evidence root before editing the repair. It maps closed labels to those exact
names/targets and accepts no arbitrary test name. Record its SHA-256 in the
handoff and never change it. The original fixed-command runner remains
unchanged.

## Phase 2 — Deterministic Evidence-Harness/Test Repair

Add a regression for each proven cause. The mutation regression must fail
against the pre-repair behavior under its deterministic ordering trigger and
pass after the repair. Preserve the pre-repair failure logs or mechanically
demonstrated state transition; do not manufacture a different error.

For capture, distinguish a writer/shutdown failure from a valid accepted
prefix. The current named test accepts any `TerminalFinish` and discards its
`shutdown_error` before verification. A deterministic writer-fault regression
must prove that the old flow masks a primary shutdown error with a secondary
verification result and that the repaired flow surfaces/asserts the exact
primary error. The injected durability fault remains a failing durability
outcome; the regression passes because classification and propagation are
correct. If the retained evidence cannot establish the original primary cause
and no exact in-allowlist trigger can, stop rather than equating a newly
manufactured I/O fault with the historical `InvalidRecords`.

The repair must replace an implicit timing/order assumption with exact
test-local synchronization, lifecycle completion, unique resource ownership,
or precise state observation. It must propagate and assert shutdown/writer
errors. It may not change product semantics, force global serialization, or
wait an arbitrary duration.

Before continuing, prove with `git diff --name-only` that tracked changes are
limited to the authorized evidence-harness/test path(s) and the separate
repair handoff. Dependency, public-export, schema, fixture, and `Cargo.lock`
inventories must be byte-identical to Phase 0. If `workload.rs` changes,
record the exact production-content manifest delta and prove it contains only
that path; otherwise the production-content inventory must also be
byte-identical.

If the deterministic regression cannot be implemented without production
hooks, live-product behavior, or a tracked path outside the allowlist, revert
no evidence, leave the diagnostic handoff, and stop with a production-repair
proposal.

Commit one focused repair candidate without amending or deleting the
diagnostic commits. Phase 3 runs only against that clean committed revision.

## Phase 3 — Fixed Verification Campaign

Run formatting first:

```bash
env CARGO_NET_OFFLINE=true TMPDIR="$repair_tmp" \
  cargo fmt --all -- --check
```

Then run an append-only, fixed post-repair campaign in fresh processes:

1. each named test ten times individually with exact-name selection and
   `--test-threads=1`;
2. each new deterministic regression ten times individually in fresh
   processes;
3. the complete `combined_replay` test binary ten times under default
   parallelism;
4. the complete binary three times with `--test-threads=1` as diagnostic
   comparison;
5. one PM action evidence invocation;
6. `cargo test -p reap-pm-live --all-targets --locked`;
7. `cargo clippy -p reap-pm-live --all-targets --locked -- -D warnings`; and
8. `cargo test --workspace --all-targets --locked`.

Use the exact Phase 1 command forms for the named and complete
`combined_replay` runs. Each new regression uses the one form frozen for its
location:

```bash
env CARGO_NET_OFFLINE=true TMPDIR="$repair_tmp" \
  cargo test --locked -p reap-pm-live \
    --test combined_replay <exact-regression-name> \
    -- --exact --test-threads=1 --nocapture

env CARGO_NET_OFFLINE=true TMPDIR="$repair_tmp" \
  cargo test --locked -p reap-pm-live \
    --lib <exact-module-qualified-regression-name> \
    -- --exact --test-threads=1 --nocapture
```

The regression runner must prove that the outer harness matched exactly one
test and that exactly one passed; a zero-match exit is red.

The remaining exact commands are:

```bash
env CARGO_NET_OFFLINE=true TMPDIR="$repair_tmp" \
  cargo bench --locked -p reap-pm-live --bench pm_action_path
env CARGO_NET_OFFLINE=true TMPDIR="$repair_tmp" \
  cargo test --locked -p reap-pm-live --all-targets
env CARGO_NET_OFFLINE=true TMPDIR="$repair_tmp" \
  cargo clippy --locked -p reap-pm-live --all-targets -- -D warnings
env CARGO_NET_OFFLINE=true TMPDIR="$repair_tmp" \
  cargo test --locked --workspace --all-targets
```

Every predeclared post-repair run must pass. A clean failure is immutable red
for that candidate revision and stops its remaining campaign; it may not be
replaced by another run on the same revision. Goal G-R may continue only after
a causally justified repair change is committed as a new revision, with a
distinct append-only validation campaign. It may never rerun an unchanged
candidate merely to obtain a preferred result.

The real-writer recovery evidence must remain byte-identical across its two
passes and retain all Goal F anchors:

| Anchor | Exact value |
| --- | --- |
| Artifact lines | `35,012` |
| Artifact bytes | `22,791,589` |
| Writer SHA-256 | `83ced509c9ea180e66d957853f9ff7762ef3c0babc316c9251c12d4d1a5224eb` |
| Canonical recovery SHA-256 | `f98bf8a88f34fb6e3c4dcfd1919a2c1d4577b2da3960375e216e596d0746cd35` |
| Recovery peak / hard limit | `2,959,343` / `16,777,216` bytes |
| Recovery records / last sequence | `35,012` / `35,011` |
| Production order entry | `false` |

For every successful full or phase-6-only log, extract the report exactly:

```bash
awk '/^\{/{print}' "$log" |
  jq -c 'select(.target == "combined_replay")' >"$report"
test "$(wc -l <"$report")" -eq 1
jq -e --arg candidate_head "$candidate_head" '<combined-check>' \
  "$report" >/dev/null
```

`<combined-check>` is:

```jq
.schema_version == 1 and
.target == "combined_replay" and
.fixture_revision == "goal-f-phase6-option1-v1" and
.build_revision == $candidate_head and
.replay_working_limit_bytes == 16777216 and
.artifact_bytes == 22791589 and
.artifact_lines == 35012 and
.artifact_sha256 ==
  "83ced509c9ea180e66d957853f9ff7762ef3c0babc316c9251c12d4d1a5224eb" and
.first_recovery.canonical_sha256 ==
  "f98bf8a88f34fb6e3c4dcfd1919a2c1d4577b2da3960375e216e596d0746cd35" and
.second_recovery.canonical_sha256 ==
  "f98bf8a88f34fb6e3c4dcfd1919a2c1d4577b2da3960375e216e596d0746cd35" and
.first_recovery.peak_working_bytes == 2959343 and
.second_recovery.peak_working_bytes == 2959343 and
.first_recovery.record_count == 35012 and
.second_recovery.record_count == 35012 and
.first_recovery.last_sequence == 35011 and
.second_recovery.last_sequence == 35011 and
.first_recovery.owned_orders == 0 and
.second_recovery.owned_orders == 0 and
.first_recovery.fill_keys == 0 and
.second_recovery.fill_keys == 0 and
.first_recovery.unresolved_orders == 0 and
.second_recovery.unresolved_orders == 0 and
.first_recovery.requires_reconciliation == false and
.second_recovery.requires_reconciliation == false and
.byte_identical_projection == true and
.production_order_entry_authorized == false
```

Pass the exact candidate commit as `--arg candidate_head`. Zero or multiple
reports is red.

Freeze every combined setup/input/measured/recovery/resource field, not only
the selected assertions:

```bash
jq -S 'del(.build_revision, .rustc, .host)' "$report" \
  >"$combined_projection"
test "$(sha256sum "$combined_projection" | cut -d' ' -f1)" = \
  3fb6c3c24f2995f57d71be9ba5a4fd36c13ffe956d0ab91bc497370f6259b91a
```

Extract the PM action report exactly:

```bash
awk '/^\{/{print}' "$log" |
  jq -c 'select(.benchmark == "pm_action_path")' >"$report"
test "$(wc -l <"$report")" -eq 1
jq -e --arg candidate_head "$candidate_head" '<pm-action-check>' \
  "$report" >/dev/null
```

`<pm-action-check>` is:

```jq
.schema_version == 1 and
.benchmark == "pm_action_path" and
.warmup_runs == 1 and
.production_order_entry_authorized == false and
(.recorded_runs | length) == 3 and
all(.recorded_runs[];
  .schema_version == 1 and
  .benchmark == "pm_action_path" and
  .fixture_revision == "goal-f-phase6-option1-v1" and
  .build_revision == $candidate_head and
  .production_order_entry_authorized == false and
  .capacities.reserved_capacity_bytes == 58858352 and
  .capacities.reserved_capacity_limit_bytes == 67108864 and
  .owner_allocations.allocation_calls == 0 and
  .owner_allocations.allocated_bytes == 0 and
  .owner_allocations.peak_live_bytes_delta == 0 and
  .parser.fixture_sha256 ==
    "985332384ae2e7b2535c0fa2c214b40862997b0f80c450be87ac108fff9b550b" and
  .parser.projection_sha256 ==
    "588e14caac0d5a38c94f9ee121b0238f084a4e2c57dbcd1c7f8f5f052210e885" and
  .parser.matches_owner_corpus == true and
  (.repeated_passes | length) == 5 and
  all(.repeated_passes[];
    .journal_hash ==
      "389887a2d044867c6ad1f7b7b9ad52aa58c792864846fc42f220759fac111b85" and
    .logical_hash ==
      "4931af3e39ee291db82ba40da7a5e73473431801606565b5ad625c69beb70475" and
    .reserved_capacity_bytes == 58858352 and
    .terminal_state_lengths_zero == true))
```

The unchanged runner/contract validators must also accept every exact
setup/input/counter/capacity/queue/allocation projection; a successful bench
exit is part of the gate, not a substitute for the explicit checks.

Create and hash the exact frozen non-timing PM projection:

```bash
jq -S '
  .recorded_runs |= map(
    del(.action_latency_ns, .timer, .total_elapsed_ns,
        .external_observations_per_second,
        .owner_reductions_per_second, .build_revision, .rustc, .host,
        .parser.pm_latency_ns, .parser.okx_latency_ns)
  )
' "$report" >"$pm_projection"
test "$(sha256sum "$pm_projection" | cut -d' ' -f1)" = \
  cc90806d19c5d2a252acbd64f3439ece2a0cb1b9d44566b84aa421d8c37b708c
```

Run these additional frozen semantic commands once on the clean candidate:

```bash
env CARGO_NET_OFFLINE=true TMPDIR="$repair_tmp" \
  cargo test --locked -p reap-engine --test decision_replay
env CARGO_NET_OFFLINE=true TMPDIR="$repair_tmp" \
  cargo test --locked -p reap-live --lib \
    coordinator::tests::decision_parity::initialized_live_reduction_matches_engine_decisions_and_is_byte_stable \
    -- --exact
env CARGO_NET_OFFLINE=true TMPDIR="$repair_tmp" \
  cargo test --locked -p reap-pm-core --test numeric_contract
env CARGO_NET_OFFLINE=true TMPDIR="$repair_tmp" \
  cargo run --locked -q -p reap-cli -- \
    backtest --format normalized-jsonl \
    --config examples/iarb2-basic.toml \
    --data fixtures/normalized/chaos_quote_hedge.jsonl --pretty
```

Run the backtest command twice into separate retained stdout/stderr files;
require byte identity and canonical stdout SHA-256
`38acf9f5e0c310f2ec5528974beffadf4c1a7f84d46efa8d9664ee7051e84691`
for both. Hash these four Goal D inputs directly:

| Input | SHA-256 |
| --- | --- |
| `fixtures/decision_parity/risk_initialization_v1.json` | `7e0951c41f447b9f46a73b24a3fe85bdc8f2bb8a623385dab0c3655926e73780` |
| `fixtures/decision_parity/replay_events_v1.jsonl` | `dede17a546d4d717c78dc2b3b7aa7c3f3f785d552404160407c78fb87cec9101` |
| `fixtures/decision_parity/expected_engine_v1.jsonl` | `140c268619b889a19d779e1bdfd340c11901d2eb1d9e4d216d976ba3d8b0d37a` |
| `fixtures/decision_parity/expected_live_reduction_v1.json` | `aa66cc09bba29cde25ab2df66c018517b2c900f83373f95580150e8bcd442b60` |

No golden byte may be regenerated.

Rehash the four immutable Goal G evidence files and the complete evidence-tree
streams after the campaign. Review the full diff for assertion weakening,
hidden serialization, timing waits, unbounded work, or broadened authority.

## Completion And Handoff

Goal G-R is complete only when:

- both original failures have transition-level causal proofs;
- deterministic regressions fail before and pass after the repair;
- the tracked repair is within the explicit evidence-harness/test allowlist;
- every fixed post-repair run is green with no discarded result;
- all Goal F semantic, artifact, numerical, resource, and safety anchors are
  exact;
- all Goal G red evidence hashes are unchanged;
- the worktree is clean after focused commits; and
- `docs/goal-g-replay-repair-handoff.md` records commands, attempt hashes,
  causal proofs, diff inventories, commits, and non-claims.

Do not push unless the user separately asks.

Completion of Goal G-R does not change Goal G's stopped result. The handoff
must state that a separately user-reviewed Amendment 3, a distinct new Goal G
evidence root, and the existing Phase 1 storage gate are still required
before any future Goal G replay or implementation.

## Stop Conditions

Stop and report the exact conflict when:

- the starting repository/evidence identity is wrong or unexplained;
- storage is below the exact gate;
- another process overlaps a recorded attempt;
- either failure lacks a deterministic causal trigger;
- the correct fix requires any tracked path outside the allowlist or any
  live-product, dependency, schema, fixture, bound, timeout, capacity,
  artifact, or Goal G document change;
- a test passes only through serialization, sleeps, retries, reduced work, a
  relaxed assertion, accepted error, or ignored failure;
- a frozen Goal F anchor changes;
- a Goal G stopped-evidence hash changes;
- a fixed post-repair attempt fails and no causally justified new repair
  revision remains within scope; or
- completion would require a credential, external request, real order,
  target-host decision, cleanup authority, push, or scope expansion.

At a stop, preserve every result and propose only the smallest next owner.
