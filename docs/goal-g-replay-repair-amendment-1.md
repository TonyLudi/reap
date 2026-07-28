# Goal G-R Amendment 1 — Replacement Runner Bootstrap

## Authority And Purpose

The user authorized this amendment on `2026-07-28` after Goal G-R stopped at
the first Phase 1 runner invocation. This amendment is an overlay on
`docs/goal-g-replay-repair-prompt.md`; every original requirement remains in
force unless this document narrowly supersedes it.

The stopped state is anchored at:

```text
commit=a300a6990b0786939bd3b0aac551d4e5c8299622
tree=8b3ba1ba478570fc1affba273adfbf25f34274ce
parent=e992363c6aa75680b3479bb0a805db813355acbc
subject=docs: record goal g-r runner stop
```

The failed v1 runner assigned the top-level repository path to readonly
variable `root`. Its exact storage-preflight subshell then assigned the same
name and deterministically exited `65` before attempt creation or Cargo
launch. The tracked handoff records the complete stop proof.

Amendment 1 authorizes exactly one replacement main runner and one retained
no-Cargo bootstrap. It does not authorize a product, test, fixture,
dependency, capacity, timeout, Goal G, sibling-repository, credential,
network, order, cleanup, or push change.

## Immutable Legacy Anchors

The following v1 artifacts remain at their existing paths and may never be
edited, renamed, deleted, replaced, touched, or chmodded:

| Artifact | SHA-256 | Mode |
| --- | --- | --- |
| `target/tmp/goal-g-replay-repair/run-attempt.sh` | `16d61de14b41a5551d8632e9599aa6ee54fb68d2a7dc00eca5d74d7ac351d1fc` | `500` |
| `target/tmp/goal-g-replay-repair/run-attempt.sha256` | `6c109abc2e1e0f9792c8817b5d978f789d229f37390087f58ea52dfb60a94c43` | `400` |
| `target/tmp/goal-g-replay-repair/phase0-start-attestation.txt` | `e94acfd7299b32aacc1bd54f4ecfcd018103bd462b84de9d21f1da496b46cf67` | `400` |
| `target/tmp/goal-g-replay-repair/phase0-start-attestation.sha256` | `a4a6bac51877a44b1ed13e26b9d7aa0c68e8bf73c6c8b1787a45f3f1bf1d00e0` | `400` |

The stopped prompt and handoff at `a300a699` have SHA-256 values:

```text
goal-g-replay-repair-prompt.md=575147da720f01ca41eceeeeda2e4655dcce10fb1753c25d13b874a80a3cbcdc
goal-g-replay-repair-handoff.md=5c2fed7ca3de5ad515a203661ad689ee136b9e17ac02c572e970d920ddb36605
```

The absent v1 candidate path
`target/tmp/goal-g-replay-repair/e992363c6aa75680b3479bb0a805db813355acbc`
must remain absent. The failed shell invocation is not attempt 01 and may not
be represented by a fabricated attempt directory.

All four named Goal G artifacts, the complete Goal G evidence tree, and every
Goal F anchor remain governed by the original contract.

## V2 Runner Boundary

The one authorized replacement bundle is:

```text
target/tmp/goal-g-replay-repair/amendment-1-v2/
```

The bundle is a nonsymlink mode-`700` directory owned by the executing user.
Before bootstrap it contains exactly two nonsymlink regular files:

```text
run-attempt.sh          mode 500
run-attempt.sha256      mode 400
```

The hash file has exactly one line in the form
`<64-lowercase-hex><two spaces>run-attempt.sh`. After bootstrap the bundle
contains exactly those two files plus the nonsymlink
`runner-bootstrap-v2/` directory. No symlink, special file, or other
top-level entry is allowed. Existing real-attempt paths remain exactly:

```text
target/tmp/goal-g-replay-repair/<candidate-head>/<campaign>/<ordinal>-<label>/
```

The existing empty mode-`700` runtime root remains:

```text
target/tmp/goal-g-replay-repair-runtime
```

The v2 runner is mechanically derived from frozen v1. Its allowed behavioral
delta is closed to:

1. rename the parent-scope readonly repository variable from `root` to
   `repo_root` and update only parent-scope references;
2. retain the original eight-line `storage_preflight()` body byte-identical,
   including its subshell-local `root`;
3. point runner self-path and hash-path checks at the amendment bundle;
4. verify this amendment, all v1 anchors, and the sealed bootstrap before any
   real attempt; and
5. add the one reserved no-Cargo bootstrap tuple below.

All original fixed label-to-argv mappings, runtime path strings, ordinals,
validators, projections, process/signal lifecycle, storage floor, regression
insertion, attempt paths, prior-attempt rules, and backtest finalization remain
unchanged. V2 attempt metadata may add amendment, bootstrap, legacy-runner,
and `runner_generation=v2` anchors without changing
`schema=goal-g-r-attempt-v1`.

## Retained No-Cargo Bootstrap

The authorization commit, called A, must be the direct child of
`a300a6990b0786939bd3b0aac551d4e5c8299622`, have exact subject
`docs: authorize goal g-r runner amendment 1`, and change exactly:

```text
docs/goal-g-replay-repair-amendment-1.md
docs/goal-g-replay-repair-handoff.md
```

No rename, mode change, merge parent, or other tracked delta is allowed.
After clean commit A, create, review, hash, and freeze v2. Its first and only
bootstrap invocation is:

```text
run-attempt.sh <authorization-head> runner-bootstrap-v2 00 no-cargo
```

The runner recognizes only this exact tuple. Bootstrap validates the actual
frozen v2 self-path, hash, lock, repository identity, clean status, storage
gate, empty runtime, absent v1 candidate, immutable v1 files, and Goal G
anchors. It renders—but never evaluates or executes—the original 49 fixed
matrix rows through the actual v2 mapping functions.

The bootstrap branch is structurally before candidate/attempt definitions,
traps, attempt-directory creation, the Cargo version probe, `setsid`, and
workload launch. It may invoke no Cargo command. It records:

```text
cargo_invocation_attempted=false
cargo_version_probe_invoked=false
setsid_invoked=false
attempt_directory_created=false
bootstrap_pass=true
```

Bootstrap builds a previously absent pending directory and atomically renames
it to:

```text
target/tmp/goal-g-replay-repair/amendment-1-v2/.runner-bootstrap-v2.pending/
target/tmp/goal-g-replay-repair/amendment-1-v2/runner-bootstrap-v2/
```

The first path is the exact pending path and the second is the exact final
path. Both must be absent before invocation. Pending and final share the same
parent filesystem. Bootstrap creates pending as a nonsymlink mode-`700`
directory owned by the executing user. It creates only the seven named
nonsymlink regular files below, makes every file mode `400`, changes pending
to mode `500`, completes and verifies it, then publishes with one
same-filesystem `mv` to final. The final directory remains owner-local mode
`500`, contains no symlink or special file, and contains exactly:

```text
bootstrap.meta
goal-g-preservation.meta
matrix.tsv
no-cargo.meta
process.ps.tsv
v1-preservation.tsv
bootstrap.sha256
```

`bootstrap.sha256` contains exactly these six lexically sorted lines using
the standard sha256sum format
`<64-lowercase-hex><two spaces>./<filename>`:

```text
./bootstrap.meta
./goal-g-preservation.meta
./matrix.tsv
./no-cargo.meta
./process.ps.tsv
./v1-preservation.tsv
```

It covers every regular file except itself. From both pending before publish
and final after publish, `sha256sum -c bootstrap.sha256` must pass and the
actual sorted regular-file inventory excluding `./bootstrap.sha256` must
equal the manifest inventory. A pending path, partial result, existing final
path, signal, identity mismatch, storage failure, process overlap, matrix
mismatch, or invalid inventory is an immutable stop. The pending path is
retained on failure; nothing may be deleted or retried.

Process snapshots prove absence of overlapping or lingering matching
processes during bootstrap. The historical v1 no-Cargo conclusion remains
deductive from its frozen control flow, exit `65`, tracked stop record, absent
candidate/attempt paths, and empty runtime; it is not inferred merely from a
later process snapshot.

## Activation And Original Matrix

After bootstrap passes, append the exact activation block below to
`docs/goal-g-replay-repair-handoff.md`. Every key must occur exactly once in
the complete handoff; the authorization-time `...status=pending` keys remain
unchanged historical fields and are not duplicated.

```text
goal_g_r_amendment_1_activation_status=active
goal_g_r_amendment_1_activation_schema=goal-g-r-runner-amendment-1-activation-v1
goal_g_r_amendment_1_activation_authorization_commit=<A>
goal_g_r_amendment_1_activation_authorization_tree=<A-tree>
goal_g_r_amendment_1_activation_authorization_parent=a300a6990b0786939bd3b0aac551d4e5c8299622
goal_g_r_amendment_1_activation_authorization_subject=docs: authorize goal g-r runner amendment 1
goal_g_r_amendment_1_activation_amendment_sha256=<A amendment document SHA-256>
goal_g_r_amendment_1_activation_v2_runner_path=target/tmp/goal-g-replay-repair/amendment-1-v2/run-attempt.sh
goal_g_r_amendment_1_activation_v2_runner_sha256=<v2 runner SHA-256>
goal_g_r_amendment_1_activation_v2_runner_mode=500
goal_g_r_amendment_1_activation_v2_hash_path=target/tmp/goal-g-replay-repair/amendment-1-v2/run-attempt.sha256
goal_g_r_amendment_1_activation_v2_hash_file_sha256=<v2 hash-file SHA-256>
goal_g_r_amendment_1_activation_v2_hash_mode=400
goal_g_r_amendment_1_activation_v2_hash_content=<v2 SHA-256 two-spaces run-attempt.sh>
goal_g_r_amendment_1_activation_bundle_mode=700
goal_g_r_amendment_1_activation_bootstrap_path=target/tmp/goal-g-replay-repair/amendment-1-v2/runner-bootstrap-v2
goal_g_r_amendment_1_activation_bootstrap_mode=500
goal_g_r_amendment_1_activation_bootstrap_manifest_sha256=<bootstrap.sha256 SHA-256>
goal_g_r_amendment_1_activation_bootstrap_manifest_mode=400
goal_g_r_amendment_1_activation_cargo_invoked_between_a_and_b=false
goal_g_r_amendment_1_activation_first_campaign=phase1-diagnostic
goal_g_r_amendment_1_activation_first_ordinal=01
goal_g_r_amendment_1_activation_first_label=mutation-exact
```

Commit B must be the direct child of A, have exact subject below, and change
only `docs/goal-g-replay-repair-handoff.md` with no rename, mode change, merge
parent, or other tracked delta:

```text
docs: activate goal g-r amendment 1 runner
```

Commit B cannot record its own ID without circularity. The v2 runner derives B
as the first ancestry-path commit after A and verifies its direct parent,
exact subject, and exact handoff-only diff. The first real v2 candidate is
clean B. Every v2 attempt must verify:

- A is the exact direct child of the stopped commit with the exact two-file
  authorization diff;
- derived B is the exact direct child of A with the exact handoff-only
  activation diff;
- A and B are ancestors of the candidate, and B equals the candidate for the
  first Phase 1 attempt;
- the tracked amendment fields match actual v1, v2, and bootstrap identities;
- the bootstrap exact inventory and seal remain valid;
- the v1 candidate path remains absent;
- current HEAD/tree/status, `Cargo.lock`, runtime, storage, and process gates
  are exact; and
- the original Goal G-R predecessor chain is exact.

Phase 1 then resumes at the original
`phase1-diagnostic/01-mutation-exact`. The bootstrap consumes no attempt
ordinal. Ordinals 02–06, regression freezing, repairs, Phase 3, Goal F
anchors, and Goal G evidence preservation proceed exactly under the original
contract.

## Amendment Stop Conditions

Stop without cleanup, retry, fallback, or runner mutation when:

- any starting commit/tree/branch/status or immutable v1/Goal G anchor is
  wrong;
- the historical handoff through the stop record is rewritten rather than
  appended;
- the v1 candidate path exists;
- the v2 bundle or bootstrap path preexists unexpectedly, is a symlink, has
  the wrong owner/mode, or has an extra entry;
- v2 differs outside the closed delta above;
- any new Cargo or runner invocation after Amendment 1 work begins occurs
  before A authorization and v2 freeze;
- bootstrap invokes Cargo, creates a candidate/attempt directory, or fails
  any gate;
- the activation commit changes anything except the handoff;
- the first real attempt is not original Phase 1 ordinal 01 at the clean
  activation revision; or
- any original command, validator, bound, artifact, stop rule, or authority
  boundary is relaxed.

The original v1 runner may never be used again. A v2 bootstrap failure
requires a separately user-reviewed amendment; it may not be patched and
rerun under Amendment 1.
