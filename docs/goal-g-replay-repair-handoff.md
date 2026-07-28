# Goal G-R Replay Repair Handoff

## Status

Goal G-R is stopped at its Phase 1 causal-proof gate. Amendment 4's separately
hashed v5 runner passed its retained no-Cargo bootstrap, and the original
six-attempt diagnostic matrix then completed. All six Cargo commands exited
`0`; the two original failures did not recur. Source analysis proves two
places where the harness can mask a primary asynchronous failure, but the
frozen Goal G log did not retain the exact persistence transition or
writer/shutdown error variant. Goal G-R therefore cannot create the required
deterministic historical regressions without manufacturing missing evidence,
which its prompt expressly forbids.

No repair source, regression runner, Phase 2 regression, Phase 3 campaign,
bench, clippy, workspace gate, backtest, external request, or order was
started. Goal G remains stopped on its immutable valid-red Amendment 2
evidence; this repair goal neither resumes nor amends Goal G.

Current terminal status recorded at `2026-07-28T17:55:06Z`. The earlier
runner stops and their timestamps remain below as immutable history.

## Frozen Starting Identity

- base commit: `e426f9593844463e85b9a716f05116b3cdfe734a`
- base tree: `224d367c8f0a3d2cc6307f0ad446013d7c096965`
- base parent: `4da8b43126e1b270758224ffa9f2bbe9f239f79d`
- branch: `master`
- local `origin/master`: `e426f9593844463e85b9a716f05116b3cdfe734a`
- base versus local `origin/master`: `0` ahead / `0` behind
- starting tracked status: clean
- `Cargo.lock` SHA-256:
  `2673d055c943c3bd5444531b67df280026c145cbbbc99b68a06f4ac0c2dbb0ff`
- repository filesystem available bytes at the initial attestation:
  `3787317248`
- required storage floor: `2147483648` bytes
- host `/tmp` available bytes: `257224704`; it is not used

The complete environment, worktree, process, and Goal-F-to-base path
inventory is retained at
`target/tmp/goal-g-replay-repair/phase0-start-attestation.txt`, with SHA-256
`e94acfd7299b32aacc1bd54f4ecfcd018103bd462b84de9d21f1da496b46cf67`.

## Frozen Attempt Runner

The reviewed, ignored runner is:

```text
target/tmp/goal-g-replay-repair/run-attempt.sh
```

Its immutable SHA-256 is:

```text
16d61de14b41a5551d8632e9599aa6ee54fb68d2a7dc00eca5d74d7ac351d1fc
```

The runner is mode `500`, its one-line hash attestation is mode `400`, and
the Phase 0 attestation and its hash file are mode `400`. Three independent
reviews accepted this exact digest for:

- closed command, candidate, campaign, ordinal, and predecessor binding;
- append-only inventories, hashes, validators, and campaign commits;
- process ownership, bounded termination, signal handling, and no-seal
  behavior while a child, process group, wait state, or log remains mutable.

The runner must not change after this point.

## Immutable Goal G Red Evidence

The four named artifacts were rehashed immediately before this handoff:

| Artifact | SHA-256 |
| --- | --- |
| `replay.selected` | `4168ac456d70361429967d7457e0d5850cd014c0b0ea7b8e45e3183372ec766d` |
| `replay/attempt-1/combined-replay.log` | `fe3e8c7323c52163345e6330ebd7587858990a49d1bc436a1a669792f6473cd9` |
| `replay/attempt-1/replay.meta` | `b2dc689182ea8c02fd340669b2b0f142b6cafd15d5ec38a04cda221f3aaa8f56` |
| `replay/attempt-1/replay.ps.tsv` | `fd77e0c1db9970bbe2c20eea70dc8836091a81e77d9bd66491c4d8150f4bf0c3` |

The complete ignored Goal G tree still has `11594` regular files and `12253`
entries. Its exact streams remain:

- file-hash stream:
  `35a99a10c133fd680cef1f4e411dbc55490f4e41199411aae907cd348aced340`
- type/mode/size/path/link inventory:
  `23c4b85375e2d27e657c38b4560c3ee1bfecae1c1b5c98baf4cf1462dc05f7b2`

The selected attempt remains attempt `1`, command exit `101`,
`evidence_valid=true`, and `gate_pass=false`. Its two named failures remain:

1. `phase6_real_mutation_artifacts_recover_to_the_same_bounded_projection`
   reported a fake-effect script mismatch in its isolated recovery child.
2. `raw_frame_and_raw_count_bounds_are_exact` reported
   `InvalidRecords` from the PM public-capture verifier.

These are diagnosis targets, not yet causal conclusions.

## Phase 1 Contract

The next action is the six-attempt, append-only diagnostic matrix at the
committed handoff revision:

1. mutation test, exact and serial;
2. capture test, exact and serial;
3. complete `combined_replay`, serial;
4. complete `combined_replay`, default parallel;
5. complete `combined_replay`, default parallel;
6. complete `combined_replay`, default parallel.

A command failure may be retained as valid diagnostic evidence. Process
overlap, an invalid seal, changed repository identity, nonempty runtime temp,
insufficient storage, or changed Goal G evidence stops the matrix.

The deterministic-regression contract cannot be named or frozen before the
transition-level causal proof:

```text
goal_g_r_regression_contract_status=pending
```

## Phase 1 Bootstrap Stop

At `2026-07-27T17:09:49Z`, the first and only Phase 1 invocation was:

```text
target/tmp/goal-g-replay-repair/run-attempt.sh e992363c6aa75680b3479bb0a805db813355acbc phase1-diagnostic 01 mutation-exact
```

It exited `65` before Cargo launch with:

```text
target/tmp/goal-g-replay-repair/run-attempt.sh: line 26: root: readonly variable
Goal G-R storage gate failed before attempt creation
```

The failure is deterministic. The runner assigns and freezes its repository
path as a top-level readonly shell variable:

```bash
root=$(git rev-parse --show-toplevel)
readonly root
```

Its exact storage-preflight subshell later assigns the same identifier:

```bash
storage_preflight() (
  set -euo pipefail
  root=$(git rev-parse --show-toplevel)
  # ...
)
```

Bash propagates the readonly attribute into the subshell, so the assignment
fails before the filesystem check. Available repository bytes were
`3689611264`, above the required floor; capacity was not the cause.

Post-failure checks proved:

- no candidate or attempt directory was created;
- the goal-owned runtime directory remained empty;
- no Cargo, rustc, combined-replay, or Reap process remained;
- the tracked worktree remained clean at
  `e992363c6aa75680b3479bb0a805db813355acbc`;
- the frozen runner and Phase 0 attestation hashes remained exact; and
- all four Goal G files, both complete-tree streams, `11594` files, and
  `12253` entries remained exact.

The runner cannot be repaired inside the current contract. It was reviewed,
hashed, made immutable, recorded in the committed handoff, and then invoked;
the Goal G-R prompt says it must never change after the first invocation.
Rerunning it would reproduce the same pre-attempt failure, while changing it
would silently violate the frozen evidence contract.

The smallest next owner is a user-reviewed Goal G-R runner amendment. It
should preserve this bootstrap record, authorize one replacement runner and
new hash before any Cargo launch, remove the readonly-name collision without
changing the fixed matrices or validators, and state whether the unused
candidate path may remain absent. Goal G-R must not resume until that
amendment is committed.

## Non-Claims

```text
production_order_entry_authorized: false
real_credentials_loaded: false
authenticated_external_request_sent: false
real_polygon_rpc_request_sent: false
real_order_submitted: false
goal_g_red_evidence_modified: false
goal_g_resumed: false
```

## Amendment 1 Authorization

On `2026-07-28`, the user authorized
`docs/goal-g-replay-repair-amendment-1.md`. The historical handoff above is
unchanged. Amendment 1 preserves the stopped v1 runner and authorizes one
separately hashed v2 runner plus one retained no-Cargo bootstrap before the
original Phase 1 matrix resumes.

```text
goal_g_r_amendment_1_authority_status=authorized
goal_g_r_amendment_1_contract_schema=goal-g-r-runner-amendment-1-v1
goal_g_r_amendment_1_stop_commit=a300a6990b0786939bd3b0aac551d4e5c8299622
goal_g_r_amendment_1_stop_tree=8b3ba1ba478570fc1affba273adfbf25f34274ce
goal_g_r_amendment_1_original_execution_head=e992363c6aa75680b3479bb0a805db813355acbc
goal_g_r_amendment_1_original_execution_tree=6ccd7b8c1b79eeec754039e25a8eabd4b6d34450
goal_g_r_amendment_1_original_runner_path=target/tmp/goal-g-replay-repair/run-attempt.sh
goal_g_r_amendment_1_original_runner_sha256=16d61de14b41a5551d8632e9599aa6ee54fb68d2a7dc00eca5d74d7ac351d1fc
goal_g_r_amendment_1_original_runner_mode=500
goal_g_r_amendment_1_original_runner_hash_path=target/tmp/goal-g-replay-repair/run-attempt.sha256
goal_g_r_amendment_1_original_runner_hash_file_sha256=6c109abc2e1e0f9792c8817b5d978f789d229f37390087f58ea52dfb60a94c43
goal_g_r_amendment_1_original_runner_hash_mode=400
goal_g_r_amendment_1_original_phase0_attestation_sha256=e94acfd7299b32aacc1bd54f4ecfcd018103bd462b84de9d21f1da496b46cf67
goal_g_r_amendment_1_original_phase0_hash_file_sha256=a4a6bac51877a44b1ed13e26b9d7aa0c68e8bf73c6c8b1787a45f3f1bf1d00e0
goal_g_r_amendment_1_original_bootstrap_exit=65
goal_g_r_amendment_1_original_bootstrap_cargo_launched=false
goal_g_r_amendment_1_original_bootstrap_attempt_created=false
goal_g_r_amendment_1_original_candidate_path_state=absent
goal_g_r_amendment_1_v2_scope=readonly-parent-root-plus-self-path-bootstrap-and-anchor-verification-only
goal_g_r_amendment_1_v2_runner_status=pending
goal_g_r_amendment_1_bootstrap_status=pending
goal_g_r_amendment_1_cargo_authorized_before_activation=false
goal_g_r_amendment_1_phase1_matrix=original-goal-g-r-six-attempt-v1
```

## Amendment 1 Bootstrap Stop

The separately reviewed v2 runner was frozen before its first invocation:

| Artifact | SHA-256 | Mode |
| --- | --- | --- |
| `target/tmp/goal-g-replay-repair/amendment-1-v2/run-attempt.sh` | `221ab5f04f3a72047b0ff66ec70a827b608978baa99ba0f266f8cb30b99dd37c` | `500` |
| `target/tmp/goal-g-replay-repair/amendment-1-v2/run-attempt.sha256` | `b2ef7aa095142dbc693fa6d45d62c81105d30bc9e19e6bab600d8fc3860520ff` | `400` |

Its first and only Amendment 1 bootstrap invocation was:

```text
target/tmp/goal-g-replay-repair/amendment-1-v2/run-attempt.sh d23107df29ad318972a2ad8b869845cbf8fd3252 runner-bootstrap-v2 00 no-cargo
```

It exited `65` with exact stderr:

```text
Amendment 1 no-Cargo bootstrap failed and may not be retried
```

The deterministic cause is the top-level bundle-inventory predicate at
frozen runner line `941`. Its `LC_ALL=C sort` produces:

```text
f	run-attempt.sh
f	run-attempt.sha256
```

but the frozen literal expects:

```text
f	run-attempt.sha256
f	run-attempt.sh
```

`run-attempt.sh` is a complete prefix of `run-attempt.sha256`, so the shorter
name sorts first. Every preceding identity, authorization, v1, named Goal G,
and path-absence gate revalidated. The predicate is before bootstrap
timestamps, the process-overlap scan, the storage preflight, and pending
directory creation. The pending and final bootstrap paths therefore remain
absent; the frozen runner has no bootstrap delete path.

Control flow proves that no Cargo probe, Cargo workload, `setsid`, candidate
directory, campaign directory, or attempt directory was reachable. Activation
commit B was not created, and the original Phase 1 matrix did not resume.
Post-stop checks found the runtime root empty and no matching process. The
tracked worktree remained clean at authorization commit
`d23107df29ad318972a2ad8b869845cbf8fd3252`.

All four v1 files retain their frozen hashes and modes. The v1 failed
candidate and the authorization-commit candidate paths remain absent. Goal G
retains `11594` files and `12253` entries, file stream
`35a99a10c133fd680cef1f4e411dbc55490f4e41199411aae907cd348aced340`,
and inventory
`23c4b85375e2d27e657c38b4560c3ee1bfecae1c1b5c98baf4cf1462dc05f7b2`.

Static review also found the same reversed filename order in the unreachable
pending-bundle and activation-bundle predicates at frozen lines `1108`–`1109`
and `1275`–`1276`. A successor must correct all three expected literals, not
only line `941`.

Recorded after the stop at `2026-07-28T03:13:21Z`.

```text
goal_g_r_amendment_1_execution_status=stopped
goal_g_r_amendment_1_execution_schema=goal-g-r-runner-amendment-1-stop-v1
goal_g_r_amendment_1_execution_authorization_commit=d23107df29ad318972a2ad8b869845cbf8fd3252
goal_g_r_amendment_1_execution_v2_runner_sha256=221ab5f04f3a72047b0ff66ec70a827b608978baa99ba0f266f8cb30b99dd37c
goal_g_r_amendment_1_execution_v2_hash_file_sha256=b2ef7aa095142dbc693fa6d45d62c81105d30bc9e19e6bab600d8fc3860520ff
goal_g_r_amendment_1_execution_bootstrap_exit=65
goal_g_r_amendment_1_execution_failed_gate=bundle-inventory-lexical-order
goal_g_r_amendment_1_execution_pending_state=absent
goal_g_r_amendment_1_execution_final_state=absent
goal_g_r_amendment_1_execution_cargo_invoked=false
goal_g_r_amendment_1_execution_setsid_invoked=false
goal_g_r_amendment_1_execution_candidate_created=false
goal_g_r_amendment_1_execution_attempt_created=false
goal_g_r_amendment_1_execution_activation_created=false
goal_g_r_amendment_1_execution_phase1_resumed=false
goal_g_r_amendment_1_execution_goal_g_modified=false
goal_g_r_amendment_1_execution_next_authority=separately-user-reviewed-amendment
```

## Amendment 2 Authorization

On `2026-07-28`, the user reviewed the Amendment 1 stop, supplied the intended
evidence-backed diagnostic target, and directed Goal G-R to proceed.
`docs/goal-g-replay-repair-amendment-2.md` preserves both failed frozen
runners and authorizes one separately hashed v3 runner plus one retained
no-Cargo bootstrap. The shared async-writer masking explanation remains a
Phase 1 hypothesis until the original diagnostic matrix and transition-level
evidence prove it.

```text
goal_g_r_amendment_2_authority_status=authorized
goal_g_r_amendment_2_contract_schema=goal-g-r-runner-amendment-2-v1
goal_g_r_amendment_2_stop_commit=f93e5e450fb438a855a065a310e173940c5614ad
goal_g_r_amendment_2_stop_tree=1dbed0d66da931ffec6061c7986e7fbf7c70c248
goal_g_r_amendment_2_stop_parent=d23107df29ad318972a2ad8b869845cbf8fd3252
goal_g_r_amendment_2_stop_subject=docs: record goal g-r amendment 1 stop
goal_g_r_amendment_2_stop_handoff_sha256=933256bfb2b2a4c73cc9c950b439c7ac674ff33cc26c555221e0ff8de1000c59
goal_g_r_amendment_2_amendment_1_sha256=c6adb4f5fdb6fce031a42d3e07a0adea538a1a1538c88e05059fa14f25e74da8
goal_g_r_amendment_2_v2_runner_path=target/tmp/goal-g-replay-repair/amendment-1-v2/run-attempt.sh
goal_g_r_amendment_2_v2_runner_sha256=221ab5f04f3a72047b0ff66ec70a827b608978baa99ba0f266f8cb30b99dd37c
goal_g_r_amendment_2_v2_runner_mode=500
goal_g_r_amendment_2_v2_hash_path=target/tmp/goal-g-replay-repair/amendment-1-v2/run-attempt.sha256
goal_g_r_amendment_2_v2_hash_file_sha256=b2ef7aa095142dbc693fa6d45d62c81105d30bc9e19e6bab600d8fc3860520ff
goal_g_r_amendment_2_v2_hash_mode=400
goal_g_r_amendment_2_v2_hash_content=221ab5f04f3a72047b0ff66ec70a827b608978baa99ba0f266f8cb30b99dd37c  run-attempt.sh
goal_g_r_amendment_2_v2_bundle_mode=700
goal_g_r_amendment_2_v2_bundle_inventory=run-attempt.sh,run-attempt.sha256
goal_g_r_amendment_2_v2_bootstrap_exit=65
goal_g_r_amendment_2_v2_failed_gate=bundle-inventory-lexical-order
goal_g_r_amendment_2_v2_pending_state=absent
goal_g_r_amendment_2_v2_final_state=absent
goal_g_r_amendment_2_v2_cargo_invoked=false
goal_g_r_amendment_2_v2_candidate_created=false
goal_g_r_amendment_2_v2_attempt_created=false
goal_g_r_amendment_2_storage_preflight_block_sha256=fe88dc9df88c320b27b414f780eca2a3c99701fb214d9ef98cb46076caea99bb
goal_g_r_amendment_2_closed_matrix_function_block_sha256=157a47489216d0d9870fbf746945c21f0934c7ade941ae81ba5a46aa59145853
goal_g_r_amendment_2_closed_matrix_tsv_sha256=7dfd76a33829817ba3cd9aa9ec2035e3fa6e08f1db62346175a022bdbf502edc
goal_g_r_amendment_2_v3_scope=generation-anchor-self-path-plumbing-plus-three-lexical-inventory-literals-only
goal_g_r_amendment_2_v3_runner_status=pending
goal_g_r_amendment_2_bootstrap_status=pending
goal_g_r_amendment_2_cargo_authorized_before_activation=false
goal_g_r_amendment_2_phase1_matrix=original-goal-g-r-six-attempt-v1
```

## Amendment 2 Bootstrap Stop

The independently reviewed v3 runner was frozen before its first invocation:

| Artifact | SHA-256 | Mode |
| --- | --- | --- |
| `target/tmp/goal-g-replay-repair/amendment-2-v3/run-attempt.sh` | `a2e5a7f77feffc8832616dd3c13d06eba80fb5fd2082dda7bf8d5c504d0ab8ec` | `500` |
| `target/tmp/goal-g-replay-repair/amendment-2-v3/run-attempt.sha256` | `de1b58342678678b92fa610380d5707554cd4530c468969878c2ecfbfe3e45bc` | `400` |

Its first and only Amendment 2 bootstrap invocation was:

```text
target/tmp/goal-g-replay-repair/amendment-2-v3/run-attempt.sh e9dd15017f15a6853513a517dcf41d80bdc8cf7f runner-bootstrap-v3 00 no-cargo
```

It exited `65`. The captured stderr contained exactly five transient
process-enumeration diagnostics followed by the terminal stop:

```text
/home/ubuntu/code/reap/target/tmp/goal-g-replay-repair/amendment-2-v3/run-attempt.sh: line 698: /proc/195950/stat: No such file or directory
/home/ubuntu/code/reap/target/tmp/goal-g-replay-repair/amendment-2-v3/run-attempt.sh: line 698: /proc/196014/stat: No such file or directory
/home/ubuntu/code/reap/target/tmp/goal-g-replay-repair/amendment-2-v3/run-attempt.sh: line 698: /proc/196081/stat: No such file or directory
/home/ubuntu/code/reap/target/tmp/goal-g-replay-repair/amendment-2-v3/run-attempt.sh: line 698: /proc/196145/stat: No such file or directory
/home/ubuntu/code/reap/target/tmp/goal-g-replay-repair/amendment-2-v3/run-attempt.sh: line 698: /proc/196211/stat: No such file or directory
Amendment 2 no-Cargo bootstrap failed and may not be retried
```

Those five diagnostics are benign PID-exit races: the sampler globbed a live
`/proc/<pid>/stat` path and the process exited before Bash opened it. Input
redirection is installed before the following stderr redirection, so the open
failure remained visible; `|| continue` kept the sampler alive. The sealed
process log has a pre row, 17 clear sampler rows, and one seal-tail-start row.
Every data row is `sample-ok`, with no overlap.

The durable failure envelope is narrower than the exact cause. The retained
pending directory reached mode `500`; its seven files reached mode `400`;
and its six-entry manifest verifies:

| Pending artifact | SHA-256 |
| --- | --- |
| `bootstrap.meta` | `9dab6c18e2715356572d174cfd0d65acbb37f6e3435c326599762d1d1f3a5a37` |
| `bootstrap.sha256` | `71ca7b6807f808a07b545325e59efc07deceef7144ea3f2d6a5920fcb96d07b7` |
| `goal-g-preservation.meta` | `588a37b4a7f0425a8de5bf1a4cb0498f7da9fb4f7c195524ab925bde33722dbb` |
| `matrix.tsv` | `7dfd76a33829817ba3cd9aa9ec2035e3fa6e08f1db62346175a022bdbf502edc` |
| `no-cargo.meta` | `b2dd67acc729cd4fa685cb2a7fd098dec96a2a76e30a6f72db357fec4489a19e` |
| `process.ps.tsv` | `f338cb3f8dcaaac866552a598364b1d37da259bb6f9d3b19479cc422172825f4` |
| `v1-preservation.tsv` | `584df2493aa7b47d3fef32475d34e5cd9872d516fad5c8b205bb12d2d46180c8` |

The final `runner-bootstrap-v3` path is absent, so atomic publication did not
succeed. The failure occurred after pending seal/chmod and before successful
`mv -nT`. V3 did not persist the last completed post-seal gate, so the
evidence cannot distinguish its silent inventory/hash, identity, process,
reserve-deadline, storage, and other pre-publication checks. Metadata records
last sample `1785236520379204000`, seal-tail start
`1785236520601642101`, and deadline `1785236521379204000`; from seal-tail
start, `527561899` nanoseconds remained before the `250000000`-nanosecond
publication reserve. The reserve-deadline checks are the leading hypothesis,
not a proven failed predicate. The prepublication `bootstrap_pass=true`
metadata does not override exit `65`, retained pending, or absent final.

Frozen v3 control flow and retained metadata prove that this invocation
reached no Cargo probe, Cargo workload, `setsid`, candidate directory,
campaign directory, or attempt directory. Activation commit D was not
created and Phase 1 did not resume.
Post-stop checks found an empty runtime root, no matching process, a clean
tracked tree at authorization commit
`e9dd15017f15a6853513a517dcf41d80bdc8cf7f`, and the exact frozen
`Cargo.lock`.

All v1 and v2 artifacts remain frozen. Goal G still has `11594` files and
`12253` entries, file stream
`35a99a10c133fd680cef1f4e411dbc55490f4e41199411aae907cd348aced340`,
and inventory
`23c4b85375e2d27e657c38b4560c3ee1bfecae1c1b5c98baf4cf1462dc05f7b2`.

Recorded after the stop at `2026-07-28T11:10:00Z`.

The execution booleans below are scoped to this single v3 invocation; they do
not claim continuous system-wide process absence after sampler shutdown.

```text
goal_g_r_amendment_2_execution_status=stopped
goal_g_r_amendment_2_execution_schema=goal-g-r-runner-amendment-2-stop-v1
goal_g_r_amendment_2_execution_authorization_commit=e9dd15017f15a6853513a517dcf41d80bdc8cf7f
goal_g_r_amendment_2_execution_v3_runner_sha256=a2e5a7f77feffc8832616dd3c13d06eba80fb5fd2082dda7bf8d5c504d0ab8ec
goal_g_r_amendment_2_execution_v3_hash_file_sha256=de1b58342678678b92fa610380d5707554cd4530c468969878c2ecfbfe3e45bc
goal_g_r_amendment_2_execution_bootstrap_exit=65
goal_g_r_amendment_2_execution_failure_envelope=sealed-pending-before-atomic-publication
goal_g_r_amendment_2_execution_exact_subgate=not-durably-recorded
goal_g_r_amendment_2_execution_pending_state=sealed-retained
goal_g_r_amendment_2_execution_pending_mode=500
goal_g_r_amendment_2_execution_pending_manifest_sha256=71ca7b6807f808a07b545325e59efc07deceef7144ea3f2d6a5920fcb96d07b7
goal_g_r_amendment_2_execution_pending_manifest_valid=true
goal_g_r_amendment_2_execution_final_state=absent
goal_g_r_amendment_2_execution_cargo_invoked=false
goal_g_r_amendment_2_execution_setsid_invoked=false
goal_g_r_amendment_2_execution_workload_invoked=false
goal_g_r_amendment_2_execution_candidate_created=false
goal_g_r_amendment_2_execution_attempt_created=false
goal_g_r_amendment_2_execution_activation_created=false
goal_g_r_amendment_2_execution_phase1_resumed=false
goal_g_r_amendment_2_execution_goal_g_modified=false
goal_g_r_amendment_2_execution_next_authority=separately-user-reviewed-amendment-3
```

## Amendment 3 Authorization

On `2026-07-28`, the user explicitly approved the narrowly scoped Amendment 3
recommended by the Amendment 2 stop review.
`docs/goal-g-replay-repair-amendment-3.md` preserves v3 and its sealed pending
evidence, authorizes one separately hashed v4 runner and retained no-Cargo
bootstrap, keeps the original timing bounds, and permits only prepublication
tail ordering, closed failure attribution, and the benign `/proc` redirection
ordering correction before the original Phase 1 matrix resumes.

```text
goal_g_r_amendment_3_authority_status=authorized
goal_g_r_amendment_3_contract_schema=goal-g-r-runner-amendment-3-v1
goal_g_r_amendment_3_stop_commit=40137d7036546b57e3930252d732157e3db37283
goal_g_r_amendment_3_stop_tree=c8b63b25b8f0de0b70791d97c7aec26ddd82b9f2
goal_g_r_amendment_3_stop_parent=e9dd15017f15a6853513a517dcf41d80bdc8cf7f
goal_g_r_amendment_3_stop_subject=docs: record goal g-r amendment 2 stop
goal_g_r_amendment_3_stop_handoff_sha256=a8e9091da9d8926ce4e3752e1907959488e6b6c2d0d89843a56ae6bafec70107
goal_g_r_amendment_3_stop_handoff_blob=5c31cfb1da6e8050588583032efd6b2ca8bb608d
goal_g_r_amendment_3_stop_handoff_size=21355
goal_g_r_amendment_3_amendment_2_sha256=686e84f5dbacbe0242e0b1a9be7c62a12072a33c1375f2d87dd02a914fb4e978
goal_g_r_amendment_3_v3_bundle_path=target/tmp/goal-g-replay-repair/amendment-2-v3
goal_g_r_amendment_3_v3_bundle_mode=700
goal_g_r_amendment_3_v3_bundle_inventory=.runner-bootstrap-v3.pending,run-attempt.sh,run-attempt.sha256
goal_g_r_amendment_3_v3_runner_path=target/tmp/goal-g-replay-repair/amendment-2-v3/run-attempt.sh
goal_g_r_amendment_3_v3_runner_sha256=a2e5a7f77feffc8832616dd3c13d06eba80fb5fd2082dda7bf8d5c504d0ab8ec
goal_g_r_amendment_3_v3_runner_mode=500
goal_g_r_amendment_3_v3_hash_path=target/tmp/goal-g-replay-repair/amendment-2-v3/run-attempt.sha256
goal_g_r_amendment_3_v3_hash_file_sha256=de1b58342678678b92fa610380d5707554cd4530c468969878c2ecfbfe3e45bc
goal_g_r_amendment_3_v3_hash_mode=400
goal_g_r_amendment_3_v3_hash_content=a2e5a7f77feffc8832616dd3c13d06eba80fb5fd2082dda7bf8d5c504d0ab8ec  run-attempt.sh
goal_g_r_amendment_3_v3_pending_path=target/tmp/goal-g-replay-repair/amendment-2-v3/.runner-bootstrap-v3.pending
goal_g_r_amendment_3_v3_pending_mode=500
goal_g_r_amendment_3_v3_pending_inventory=bootstrap.meta,bootstrap.sha256,goal-g-preservation.meta,matrix.tsv,no-cargo.meta,process.ps.tsv,v1-preservation.tsv
goal_g_r_amendment_3_v3_pending_manifest_sha256=71ca7b6807f808a07b545325e59efc07deceef7144ea3f2d6a5920fcb96d07b7
goal_g_r_amendment_3_v3_pending_manifest_valid=true
goal_g_r_amendment_3_v3_pending_process_sha256=f338cb3f8dcaaac866552a598364b1d37da259bb6f9d3b19479cc422172825f4
goal_g_r_amendment_3_v3_pending_matrix_sha256=7dfd76a33829817ba3cd9aa9ec2035e3fa6e08f1db62346175a022bdbf502edc
goal_g_r_amendment_3_v3_final_state=absent
goal_g_r_amendment_3_v3_bootstrap_exit=65
goal_g_r_amendment_3_v3_failure_envelope=sealed-pending-before-atomic-publication
goal_g_r_amendment_3_v3_exact_subgate=not-durably-recorded
goal_g_r_amendment_3_v3_cargo_invoked=false
goal_g_r_amendment_3_v3_setsid_invoked=false
goal_g_r_amendment_3_v3_workload_invoked=false
goal_g_r_amendment_3_v3_candidate_created=false
goal_g_r_amendment_3_v3_attempt_created=false
goal_g_r_amendment_3_v3_activation_created=false
goal_g_r_amendment_3_storage_preflight_block_sha256=fe88dc9df88c320b27b414f780eca2a3c99701fb214d9ef98cb46076caea99bb
goal_g_r_amendment_3_closed_matrix_function_block_sha256=157a47489216d0d9870fbf746945c21f0934c7ade941ae81ba5a46aa59145853
goal_g_r_amendment_3_closed_matrix_tsv_sha256=7dfd76a33829817ba3cd9aa9ec2035e3fa6e08f1db62346175a022bdbf502edc
goal_g_r_amendment_3_v4_bundle_path=target/tmp/goal-g-replay-repair/amendment-3-v4
goal_g_r_amendment_3_v4_bundle_state=absent
goal_g_r_amendment_3_v4_scope=generation-anchor-self-path-plumbing-plus-v3-retention-preseal-tail-ordering-closed-gate-attribution-and-proc-redirection-order-only
goal_g_r_amendment_3_v4_bootstrap_layout=seven-files-six-entry-manifest-v1
goal_g_r_amendment_3_v4_seal_tail_bound_ns=1000000000
goal_g_r_amendment_3_v4_publish_reserve_ns=250000000
goal_g_r_amendment_3_v4_timeout_change_authorized=false
goal_g_r_amendment_3_v4_failure_attribution=closed-gate-token
goal_g_r_amendment_3_v4_post_seal_scope=atomic-rename-plus-builtin-recognition-only
goal_g_r_amendment_3_v4_runner_status=pending
goal_g_r_amendment_3_v4_bootstrap_status=pending
goal_g_r_amendment_3_cargo_authorized_before_activation=false
goal_g_r_amendment_3_phase1_matrix=original-goal-g-r-six-attempt-v1
```

## Amendment 3 Bootstrap Stop

The independently reviewed v4 runner was frozen before its first invocation:

| Artifact | SHA-256 | Mode |
| --- | --- | --- |
| `target/tmp/goal-g-replay-repair/amendment-3-v4/run-attempt.sh` | `f2d0c9761ecee3084bd8711a1c372ad4d939ab27e1960ca8d59810ad587cfe08` | `500` |
| `target/tmp/goal-g-replay-repair/amendment-3-v4/run-attempt.sha256` | `3516def3e27206f214957eed103af41d91171bb3f6db80932239bdad191a6eb6` | `400` |

Its first and only Amendment 3 bootstrap invocation was:

```text
target/tmp/goal-g-replay-repair/amendment-3-v4/run-attempt.sh 7582d9fd92dbf67e54d02320307de4435cb52136 runner-bootstrap-v4 00 no-cargo
```

It exited `65` and emitted exactly:

```text
Amendment 3 v4 bootstrap failed at publish-reserve; retained state is immutable and may not be retried
```

The closed failure gate makes the cause exact. V4 completed and validated a
writable pending bundle but could not prove the required
`250000000`-nanosecond publication reserve before the unchanged deadline.
The retained pending directory is mode `700`; its seven nonsymlink regular
files are mode `600`; and the six-entry manifest verifies:

| Pending artifact | SHA-256 |
| --- | --- |
| `bootstrap.meta` | `926ff4f5665322b3147e89e1d8ac03b664535774cbef33a99626e98812a6d95d` |
| `bootstrap.sha256` | `3df5cf16715e849ed332bf6f24d25f2822175c2ef26a533047de25c7abc02003` |
| `goal-g-preservation.meta` | `cbdc7792311e06b45a8d56d9bbdc5d66ffdcdcf8b121c8bd6981c36bfd1c7675` |
| `matrix.tsv` | `7dfd76a33829817ba3cd9aa9ec2035e3fa6e08f1db62346175a022bdbf502edc` |
| `no-cargo.meta` | `6edd5bb1da6f2da3110d244d7b356fa4ed792ec0659d7d701af17e10fe4af8ee` |
| `process.ps.tsv` | `2a4aa0cc914115131482849d816767240f8d9a96040785728e09c8e5cedf17ac` |
| `v1-preservation.tsv` | `db82b18fa3795f4d482747cd2602dabe09f64f02d5cc48a126f4b3f4fc92964f` |

The process log has a header, one pre row, 16 clear sampler rows, and one
seal-tail-start row. Every data row is `sample-ok`, and both independent
post-stop process checks are recorded `true`.

The final sampler timestamp was `1785242873256426000`; the unchanged
deadline was `1785242874256426000`; and the latest valid reserve check was
therefore `1785242874006426000`. The manifest completed at
`1785242874193507264`, already `187081264` nanoseconds after the reserve
cutoff, with only `62918736` nanoseconds left before the hard deadline. Full
writable verification occurred later. This proves the `publish-reserve`
failure independently of the terminal diagnostic. The collective post-tail
schedule exhausted the reserve; the evidence does not attribute all elapsed
time to any one verifier.

Because the directory and files remain mode `700`/`600`, the runner never
entered its mode-`500`/`400` seal. The final `runner-bootstrap-v4` path is
absent, and atomic publication was never attempted. Despite their physical
writability, both the v4 runner and its pending directory are frozen retained
evidence: they must not be retried, invoked, changed, renamed, deleted,
cleaned, promoted, or copied as success. During this single v4 invocation,
no Cargo probe, Cargo workload, `setsid`, candidate directory, campaign
directory, attempt directory, activation tail, or B3 commit was created.
HEAD remained clean A3, the runtime root remained empty, and all Goal G and
v1-v3 anchors remained exact.

The narrow next design is a separately reviewed Amendment 4 with a separately
hashed v5 runner. It must preserve v4 and this complete writable pending
evidence, keep the one-second deadline and 250-millisecond reserve unchanged,
and move the expensive static semantic validation under the active sampler.
V5 can capture hashes of the validated static files before sampler shutdown,
then retain after shutdown the final process-log validation, both process
scans, cheap static-hash stability checks, metadata and manifest rendering,
complete seven-file writable verification, reserve gate, seal, and atomic
publication. This is a recommendation only and grants no Amendment 4
authority.

Recorded after the stop at `2026-07-28T12:56:01Z`.

```text
goal_g_r_amendment_3_execution_status=stopped
goal_g_r_amendment_3_execution_schema=goal-g-r-runner-amendment-3-stop-v1
goal_g_r_amendment_3_execution_authorization_commit=7582d9fd92dbf67e54d02320307de4435cb52136
goal_g_r_amendment_3_execution_v4_runner_sha256=f2d0c9761ecee3084bd8711a1c372ad4d939ab27e1960ca8d59810ad587cfe08
goal_g_r_amendment_3_execution_v4_hash_file_sha256=3516def3e27206f214957eed103af41d91171bb3f6db80932239bdad191a6eb6
goal_g_r_amendment_3_execution_bootstrap_exit=65
goal_g_r_amendment_3_execution_failure_envelope=complete-writable-pending-before-seal
goal_g_r_amendment_3_execution_exact_subgate=publish-reserve
goal_g_r_amendment_3_execution_pending_state=complete-writable-retained
goal_g_r_amendment_3_execution_pending_mode=700
goal_g_r_amendment_3_execution_pending_files_mode=600
goal_g_r_amendment_3_execution_pending_inventory=bootstrap.meta,bootstrap.sha256,goal-g-preservation.meta,matrix.tsv,no-cargo.meta,process.ps.tsv,v1-preservation.tsv
goal_g_r_amendment_3_execution_pending_bootstrap_meta_sha256=926ff4f5665322b3147e89e1d8ac03b664535774cbef33a99626e98812a6d95d
goal_g_r_amendment_3_execution_pending_manifest_sha256=3df5cf16715e849ed332bf6f24d25f2822175c2ef26a533047de25c7abc02003
goal_g_r_amendment_3_execution_pending_goal_g_sha256=cbdc7792311e06b45a8d56d9bbdc5d66ffdcdcf8b121c8bd6981c36bfd1c7675
goal_g_r_amendment_3_execution_pending_matrix_sha256=7dfd76a33829817ba3cd9aa9ec2035e3fa6e08f1db62346175a022bdbf502edc
goal_g_r_amendment_3_execution_pending_no_cargo_sha256=6edd5bb1da6f2da3110d244d7b356fa4ed792ec0659d7d701af17e10fe4af8ee
goal_g_r_amendment_3_execution_pending_process_sha256=2a4aa0cc914115131482849d816767240f8d9a96040785728e09c8e5cedf17ac
goal_g_r_amendment_3_execution_pending_preservation_sha256=db82b18fa3795f4d482747cd2602dabe09f64f02d5cc48a126f4b3f4fc92964f
goal_g_r_amendment_3_execution_pending_manifest_valid=true
goal_g_r_amendment_3_execution_prepublication_evidence_complete=true
goal_g_r_amendment_3_execution_last_sample_epoch_ns=1785242873256426000
goal_g_r_amendment_3_execution_seal_tail_start_epoch_ns=1785242873531854187
goal_g_r_amendment_3_execution_seal_tail_deadline_epoch_ns=1785242874256426000
goal_g_r_amendment_3_execution_publish_reserve_ns=250000000
goal_g_r_amendment_3_execution_reserve_cutoff_epoch_ns=1785242874006426000
goal_g_r_amendment_3_execution_manifest_mtime_epoch_ns=1785242874193507264
goal_g_r_amendment_3_execution_reserve_miss_minimum_ns=187081264
goal_g_r_amendment_3_execution_seal_attempted=false
goal_g_r_amendment_3_execution_atomic_publication_attempted=false
goal_g_r_amendment_3_execution_final_state=absent
goal_g_r_amendment_3_execution_cargo_invoked=false
goal_g_r_amendment_3_execution_cargo_version_probe_invoked=false
goal_g_r_amendment_3_execution_setsid_invoked=false
goal_g_r_amendment_3_execution_workload_invoked=false
goal_g_r_amendment_3_execution_candidate_created=false
goal_g_r_amendment_3_execution_attempt_created=false
goal_g_r_amendment_3_execution_activation_created=false
goal_g_r_amendment_3_execution_phase1_resumed=false
goal_g_r_amendment_3_execution_goal_g_modified=false
goal_g_r_amendment_3_execution_next_authority=separately-user-reviewed-amendment-4
```

## Amendment 4 Authorization

The user approved the separately reviewed Amendment 4 on `2026-07-28`.
`docs/goal-g-replay-repair-amendment-4.md` is the controlling narrow
overlay. It preserves every v1–v4 artifact and retained pending tree,
authorizes one separately hashed v5 runner plus one retained no-Cargo
bootstrap, keeps the original timing bounds, and permits only the static
semantic-validation scheduling and hash-stability change defined there.

```text
goal_g_r_amendment_4_authority_status=authorized
goal_g_r_amendment_4_contract_schema=goal-g-r-runner-amendment-4-v1
goal_g_r_amendment_4_stop_commit=6d33ea80c863b424c89ddce964b5b4374460ee81
goal_g_r_amendment_4_stop_tree=a03d8737cef1f4ff1c20f1ce6de33c1651bc653f
goal_g_r_amendment_4_stop_parent=7582d9fd92dbf67e54d02320307de4435cb52136
goal_g_r_amendment_4_stop_subject=docs: record goal g-r amendment 3 stop
goal_g_r_amendment_4_stop_handoff_sha256=4a70145b151f4307ebf62bff899c92b284fac909521c459041a03663bb01e323
goal_g_r_amendment_4_stop_handoff_blob=d9bf1b59c65f5abf1127346b6294bc94942e5ef7
goal_g_r_amendment_4_stop_handoff_size=33213
goal_g_r_amendment_4_amendment_1_sha256=c6adb4f5fdb6fce031a42d3e07a0adea538a1a1538c88e05059fa14f25e74da8
goal_g_r_amendment_4_amendment_2_sha256=686e84f5dbacbe0242e0b1a9be7c62a12072a33c1375f2d87dd02a914fb4e978
goal_g_r_amendment_4_amendment_3_sha256=39814ed9fb2ad1992bc14b7fe62753cd8f786886efc6f5c498c3b4710228d9d6
goal_g_r_amendment_4_cargo_lock_sha256=2673d055c943c3bd5444531b67df280026c145cbbbc99b68a06f4ac0c2dbb0ff
goal_g_r_amendment_4_goal_g_file_stream_sha256=35a99a10c133fd680cef1f4e411dbc55490f4e41199411aae907cd348aced340
goal_g_r_amendment_4_goal_g_inventory_sha256=23c4b85375e2d27e657c38b4560c3ee1bfecae1c1b5c98baf4cf1462dc05f7b2
goal_g_r_amendment_4_v3_pending_manifest_sha256=71ca7b6807f808a07b545325e59efc07deceef7144ea3f2d6a5920fcb96d07b7
goal_g_r_amendment_4_v3_pending_state=sealed-retained
goal_g_r_amendment_4_v4_bundle_path=target/tmp/goal-g-replay-repair/amendment-3-v4
goal_g_r_amendment_4_v4_bundle_mode=700
goal_g_r_amendment_4_v4_bundle_owner=executing-user
goal_g_r_amendment_4_v4_bundle_inventory=.runner-bootstrap-v4.pending,run-attempt.sh,run-attempt.sha256
goal_g_r_amendment_4_v4_bundle_file_count=9
goal_g_r_amendment_4_v4_bundle_entry_count=10
goal_g_r_amendment_4_v4_bundle_file_stream_sha256=782fe607a94ab46399d09f70667efba160ec7df441ad14b4bf5be29d4b9f485c
goal_g_r_amendment_4_v4_bundle_inventory_sha256=71d2bf42a749cca9613b5c82b878d60a6c2abc8257801ef69124ec34bb363dcc
goal_g_r_amendment_4_v4_runner_path=target/tmp/goal-g-replay-repair/amendment-3-v4/run-attempt.sh
goal_g_r_amendment_4_v4_runner_sha256=f2d0c9761ecee3084bd8711a1c372ad4d939ab27e1960ca8d59810ad587cfe08
goal_g_r_amendment_4_v4_runner_mode=500
goal_g_r_amendment_4_v4_hash_path=target/tmp/goal-g-replay-repair/amendment-3-v4/run-attempt.sha256
goal_g_r_amendment_4_v4_hash_file_sha256=3516def3e27206f214957eed103af41d91171bb3f6db80932239bdad191a6eb6
goal_g_r_amendment_4_v4_hash_mode=400
goal_g_r_amendment_4_v4_hash_content=f2d0c9761ecee3084bd8711a1c372ad4d939ab27e1960ca8d59810ad587cfe08  run-attempt.sh
goal_g_r_amendment_4_v4_pending_path=target/tmp/goal-g-replay-repair/amendment-3-v4/.runner-bootstrap-v4.pending
goal_g_r_amendment_4_v4_pending_state=complete-writable-retained
goal_g_r_amendment_4_v4_pending_mode=700
goal_g_r_amendment_4_v4_pending_files_mode=600
goal_g_r_amendment_4_v4_pending_owner=executing-user
goal_g_r_amendment_4_v4_pending_inventory=bootstrap.meta,bootstrap.sha256,goal-g-preservation.meta,matrix.tsv,no-cargo.meta,process.ps.tsv,v1-preservation.tsv
goal_g_r_amendment_4_v4_pending_file_count=7
goal_g_r_amendment_4_v4_pending_entry_count=7
goal_g_r_amendment_4_v4_pending_file_stream_sha256=2e2c5c0035500076d7a53bc367213ad9ed4dcaab3fda53ec07432e9cf9bc7806
goal_g_r_amendment_4_v4_pending_inventory_sha256=089d3cd69a31a95070014fcf19b3216190e31a205d38646518c8142a0263f3bf
goal_g_r_amendment_4_v4_pending_bootstrap_meta_sha256=926ff4f5665322b3147e89e1d8ac03b664535774cbef33a99626e98812a6d95d
goal_g_r_amendment_4_v4_pending_manifest_sha256=3df5cf16715e849ed332bf6f24d25f2822175c2ef26a533047de25c7abc02003
goal_g_r_amendment_4_v4_pending_goal_g_sha256=cbdc7792311e06b45a8d56d9bbdc5d66ffdcdcf8b121c8bd6981c36bfd1c7675
goal_g_r_amendment_4_v4_pending_matrix_sha256=7dfd76a33829817ba3cd9aa9ec2035e3fa6e08f1db62346175a022bdbf502edc
goal_g_r_amendment_4_v4_pending_no_cargo_sha256=6edd5bb1da6f2da3110d244d7b356fa4ed792ec0659d7d701af17e10fe4af8ee
goal_g_r_amendment_4_v4_pending_process_sha256=2a4aa0cc914115131482849d816767240f8d9a96040785728e09c8e5cedf17ac
goal_g_r_amendment_4_v4_pending_preservation_sha256=db82b18fa3795f4d482747cd2602dabe09f64f02d5cc48a126f4b3f4fc92964f
goal_g_r_amendment_4_v4_pending_manifest_valid=true
goal_g_r_amendment_4_v4_bootstrap_exit=65
goal_g_r_amendment_4_v4_failure_envelope=complete-writable-pending-before-seal
goal_g_r_amendment_4_v4_exact_subgate=publish-reserve
goal_g_r_amendment_4_v4_prepublication_evidence_complete=true
goal_g_r_amendment_4_v4_last_sample_epoch_ns=1785242873256426000
goal_g_r_amendment_4_v4_seal_tail_start_epoch_ns=1785242873531854187
goal_g_r_amendment_4_v4_seal_tail_deadline_epoch_ns=1785242874256426000
goal_g_r_amendment_4_v4_publish_reserve_ns=250000000
goal_g_r_amendment_4_v4_manifest_mtime_epoch_ns=1785242874193507264
goal_g_r_amendment_4_v4_reserve_miss_minimum_ns=187081264
goal_g_r_amendment_4_v4_seal_attempted=false
goal_g_r_amendment_4_v4_atomic_publication_attempted=false
goal_g_r_amendment_4_v4_final_state=absent
goal_g_r_amendment_4_v4_cargo_invoked=false
goal_g_r_amendment_4_v4_cargo_version_probe_invoked=false
goal_g_r_amendment_4_v4_setsid_invoked=false
goal_g_r_amendment_4_v4_workload_invoked=false
goal_g_r_amendment_4_v4_candidate_created=false
goal_g_r_amendment_4_v4_attempt_created=false
goal_g_r_amendment_4_v4_activation_created=false
goal_g_r_amendment_4_v4_phase1_resumed=false
goal_g_r_amendment_4_v4_goal_g_modified=false
goal_g_r_amendment_4_storage_preflight_block_sha256=fe88dc9df88c320b27b414f780eca2a3c99701fb214d9ef98cb46076caea99bb
goal_g_r_amendment_4_closed_matrix_function_block_sha256=157a47489216d0d9870fbf746945c21f0934c7ade941ae81ba5a46aa59145853
goal_g_r_amendment_4_closed_matrix_tsv_sha256=7dfd76a33829817ba3cd9aa9ec2035e3fa6e08f1db62346175a022bdbf502edc
goal_g_r_amendment_4_v5_bundle_path=target/tmp/goal-g-replay-repair/amendment-4-v5
goal_g_r_amendment_4_v5_bundle_state=absent
goal_g_r_amendment_4_v5_scope=generation-anchor-self-path-plumbing-plus-v4-retention-static-semantic-validation-under-sampler-prestop-hash-capture-and-poststop-hash-stability-only
goal_g_r_amendment_4_v5_bootstrap_layout=seven-files-six-entry-manifest-v1
goal_g_r_amendment_4_v5_deadline_origin=final-sampler-timestamp
goal_g_r_amendment_4_v5_seal_tail_bound_ns=1000000000
goal_g_r_amendment_4_v5_publish_reserve_ns=250000000
goal_g_r_amendment_4_v5_timeout_change_authorized=false
goal_g_r_amendment_4_v5_static_files=goal-g-preservation.meta,matrix.tsv,no-cargo.meta,v1-preservation.tsv
goal_g_r_amendment_4_v5_static_semantic_validation_phase=active-sampler
goal_g_r_amendment_4_v5_static_hash_capture_phase=after-semantic-validation-before-sampler-stop
goal_g_r_amendment_4_v5_post_stop_static_semantic_validation=false
goal_g_r_amendment_4_v5_post_stop_static_validation=sha256-stability-only
goal_g_r_amendment_4_v5_failure_attribution=closed-gate-token
goal_g_r_amendment_4_v5_post_seal_scope=atomic-rename-plus-builtin-recognition-only
goal_g_r_amendment_4_v5_runner_status=pending
goal_g_r_amendment_4_v5_bootstrap_status=pending
goal_g_r_amendment_4_cargo_authorized_before_activation=false
goal_g_r_amendment_4_phase1_matrix=original-goal-g-r-six-attempt-v1
```

## Amendment 4 Activation

```text
goal_g_r_amendment_4_activation_status=active
goal_g_r_amendment_4_activation_schema=goal-g-r-runner-amendment-4-activation-v1
goal_g_r_amendment_4_activation_authorization_commit=18137ad595278478ed2cb0989e4352292157fe25
goal_g_r_amendment_4_activation_authorization_tree=91a1e9cb7c70d6f5faa3d38bba80b2a95109fe5f
goal_g_r_amendment_4_activation_authorization_parent=6d33ea80c863b424c89ddce964b5b4374460ee81
goal_g_r_amendment_4_activation_authorization_subject=docs: authorize goal g-r runner amendment 4
goal_g_r_amendment_4_activation_amendment_sha256=3d1eb79d295f9ba3175a73bb0df27a575eea21706e341c033a9d59023c0fc2c2
goal_g_r_amendment_4_activation_v5_runner_path=target/tmp/goal-g-replay-repair/amendment-4-v5/run-attempt.sh
goal_g_r_amendment_4_activation_v5_runner_sha256=a35412493bf9b62f5795b8864306b43794816b77d824c22643c3bdd305a81f88
goal_g_r_amendment_4_activation_v5_runner_mode=500
goal_g_r_amendment_4_activation_v5_hash_path=target/tmp/goal-g-replay-repair/amendment-4-v5/run-attempt.sha256
goal_g_r_amendment_4_activation_v5_hash_file_sha256=215b051156b3cfcce13daf7d4eb386a19d6d612fce1e274b41f60b5a5eb3c846
goal_g_r_amendment_4_activation_v5_hash_mode=400
goal_g_r_amendment_4_activation_v5_hash_content=a35412493bf9b62f5795b8864306b43794816b77d824c22643c3bdd305a81f88  run-attempt.sh
goal_g_r_amendment_4_activation_bundle_mode=700
goal_g_r_amendment_4_activation_bootstrap_path=target/tmp/goal-g-replay-repair/amendment-4-v5/runner-bootstrap-v5
goal_g_r_amendment_4_activation_bootstrap_mode=500
goal_g_r_amendment_4_activation_bootstrap_manifest_sha256=ba9ee56fa085c05ee8600500bf23ce04c419c09f8219688b0c0f64be5e87c45e
goal_g_r_amendment_4_activation_bootstrap_manifest_mode=400
goal_g_r_amendment_4_activation_bootstrap_preservation_sha256=845107fa6289d3d7c069734ae0715bcbaaf7ab74696c4a5402cf654b557909ee
goal_g_r_amendment_4_activation_bootstrap_goal_g_sha256=683d67f444b9239cf790f0a3f26b75dd0702d6729d25fc6e9fda65d6bcc263ca
goal_g_r_amendment_4_activation_bootstrap_matrix_sha256=7dfd76a33829817ba3cd9aa9ec2035e3fa6e08f1db62346175a022bdbf502edc
goal_g_r_amendment_4_activation_bootstrap_no_cargo_sha256=30cb37e54c6b0e733cb76e6461c1a39a8587b6ef4bf3c7ab418a86000cff9b4c
goal_g_r_amendment_4_activation_bootstrap_process_sha256=2790e7436a24ba288ba5b9bf71c1420546a0d560848a28c8ccfee0073a78ee9e
goal_g_r_amendment_4_activation_v3_pending_manifest_sha256=71ca7b6807f808a07b545325e59efc07deceef7144ea3f2d6a5920fcb96d07b7
goal_g_r_amendment_4_activation_v4_runner_sha256=f2d0c9761ecee3084bd8711a1c372ad4d939ab27e1960ca8d59810ad587cfe08
goal_g_r_amendment_4_activation_v4_pending_manifest_sha256=3df5cf16715e849ed332bf6f24d25f2822175c2ef26a533047de25c7abc02003
goal_g_r_amendment_4_activation_v4_pending_file_stream_sha256=2e2c5c0035500076d7a53bc367213ad9ed4dcaab3fda53ec07432e9cf9bc7806
goal_g_r_amendment_4_activation_v4_pending_inventory_sha256=089d3cd69a31a95070014fcf19b3216190e31a205d38646518c8142a0263f3bf
goal_g_r_amendment_4_activation_v4_pending_state=complete-writable-retained
goal_g_r_amendment_4_activation_deadline_origin=final-sampler-timestamp
goal_g_r_amendment_4_activation_seal_tail_bound_ns=1000000000
goal_g_r_amendment_4_activation_publish_reserve_ns=250000000
goal_g_r_amendment_4_activation_static_semantic_validation_under_sampler=true
goal_g_r_amendment_4_activation_static_hashes_captured_before_sampler_stop=true
goal_g_r_amendment_4_activation_post_stop_static_hash_stability=true
goal_g_r_amendment_4_activation_post_stop_static_semantic_validation=false
goal_g_r_amendment_4_activation_cargo_invoked_between_a4_and_b4=false
goal_g_r_amendment_4_activation_setsid_invoked_between_a4_and_b4=false
goal_g_r_amendment_4_activation_workload_invoked_between_a4_and_b4=false
goal_g_r_amendment_4_activation_first_campaign=phase1-diagnostic
goal_g_r_amendment_4_activation_first_ordinal=01
goal_g_r_amendment_4_activation_first_label=mutation-exact
```

## Phase 1 Diagnostic Matrix And Causal Stop

The original six-attempt matrix completed at committed Amendment 4 activation
revision:

```text
goal_g_r_phase1_status=stopped
goal_g_r_phase1_schema=goal-g-r-phase1-causal-stop-v1
goal_g_r_phase1_head=ef231e049312b34c4b3527784afe86b4ac1595ce
goal_g_r_phase1_tree=c2d4cca03bbaad26e6c9dabbf47602f59c922792
goal_g_r_phase1_parent=18137ad595278478ed2cb0989e4352292157fe25
goal_g_r_phase1_subject=docs: activate goal g-r amendment 4 runner
goal_g_r_phase1_branch=master
goal_g_r_phase1_origin_ahead=6
goal_g_r_phase1_origin_behind=0
goal_g_r_phase1_cargo_lock_sha256=2673d055c943c3bd5444531b67df280026c145cbbbc99b68a06f4ac0c2dbb0ff
goal_g_r_phase1_evidence_root=target/tmp/goal-g-replay-repair/ef231e049312b34c4b3527784afe86b4ac1595ce/phase1-diagnostic
goal_g_r_phase1_evidence_file_count=37
goal_g_r_phase1_evidence_entry_count=43
goal_g_r_phase1_evidence_file_stream_sha256=1f79f73503bec5e329dffc72ae70936a832005d4aca405ed50fbc6a6dae5d091
goal_g_r_phase1_evidence_inventory_sha256=f2184d540cc03b6cd37f92c2671bbfc65a206bbc62ab3645584bfa807e79c68a
goal_g_r_phase1_runtime_root_empty=true
goal_g_r_phase1_process_overlap=false
```

The aggregate file stream is defined by:

```bash
(
  cd target/tmp/goal-g-replay-repair/ef231e049312b34c4b3527784afe86b4ac1595ce/phase1-diagnostic
  find . -type f -print0 |
    LC_ALL=C sort -z |
    xargs -0 sha256sum |
    sha256sum
)
```

The aggregate inventory is defined by:

```bash
(
  cd target/tmp/goal-g-replay-repair/ef231e049312b34c4b3527784afe86b4ac1595ce/phase1-diagnostic
  find . -mindepth 1 -printf '%y\t%m\t%s\t%P\t%l\n' |
    LC_ALL=C sort |
    sha256sum
)
```

Every attempt retained clean, identical pre/post repository identity,
unchanged `Cargo.lock`, no external process overlap, and an empty runtime
root. The attempt seals and outcomes are:

| Ordinal | Attempt path / label | SHA-256 of `attempt.sha256` | Command exit | Validation result | Evidence valid | Gate |
| --- | --- | --- | ---: | --- | --- | --- |
| 01 | `01-mutation-exact` / `mutation-exact` | `3e928880e4862b24cb2615c6560be36d5e002a489e866dc74a45ad6275c844e4` | 0 | `mutation-test-or-report-invalid` | true | false |
| 02 | `02-capture-exact` / `capture-exact` | `e5653eb300c1b1d109be55c9ca3164d434170b3c90b16b5293428323fe2ea625` | 0 | `capture-test-exact` | true | true |
| 03 | `03-combined-serial` / `combined-serial` | `983277a04f7814c7630e06fc77057234a9ffca082f3d39561f5d172e6cca3673` | 0 | `combined-report-invalid` | true | false |
| 04 | `04-combined-parallel` / `combined-parallel` | `7ff65d744942babac57b348531f24b82d47beda5c73e54b240e8c3a63f4742d6` | 0 | `combined-report-exact` | true | true |
| 05 | `05-combined-parallel` / `combined-parallel` | `8dc36a76184f7c655b76e0e4ea9719d1d594f015894dec4c256c344b35714224` | 0 | `combined-report-exact` | true | true |
| 06 | `06-combined-parallel` / `combined-parallel` | `b38fd6e9a724ccef161e3d800f708d35a2b1b3154979581995ac100be2ce6c7a` | 0 | `combined-report-exact` | true | true |

Attempts 01 and 03 are not test failures. Their Cargo processes exited `0`.
The frozen v5 validator requires the report JSON and named-test result to
start at column one, while the isolated recovery child writes its report
between libtest's `test <name> ... ` prefix and final `ok`. Consequently the
extractor sees zero column-one report lines. This is retained runner
validation evidence, not a reason to discard, replace, or relabel either
attempt.

### Failure 1: prepared fake-effect mismatch

Frozen source anchors used for this trace are:

```text
crates/reap-pm-live/src/evidence/workload.rs=891948103fa26c4d555f2e462be981a433c374f4d803c8115dc74d461b3cf05d
crates/reap-pm-live/src/coordinator/mutation.rs=dd9fcd0b9bbfffa6a0e5da59468674985bdc83707abc02baaff344ecae2c4b4a
crates/reap-pm-live/src/coordinator/product.rs=63dddd505091dccba46a0620f8d99f19339022b5fbe87cda8164024cfb673177
crates/reap-pm-live/src/coordinator/persistence.rs=e4f96e740eeb74d66e406847b6eb71e29559e29c7b56d94858061bccd468c052
```

| Field | Finding |
| --- | --- |
| Observed state | The frozen child reached either the Quote or Cancel fixture-dispatch path and returned the shared `EffectKindMismatch`. The pre-pop guards return that error when `next_effect_kind()` is not the requested kind; identical defensive post-pop guards also use the same error. Without a backtrace, the log does not identify Quote versus Cancel, the exact guard, an empty queue versus another front kind, or the preceding persistence-service variant. |
| Expected state | The acknowledgement immediately before fixture execution must correspond to that exact pending intent and must commit exactly one matching prepared Quote or Cancel effect. A fact acknowledgement must commit no prepared fake effect. |
| Divergence | `acknowledge_one_undrained` returns after `poll_persistence_fixture == true` and an ensuing service turn, while its callers accept any positive aggregate `service_turn().total()` as the requested acknowledgement. Neither observation carries the reduced `PmPersistenceService` variant, intent identity, nor resulting prepared-effect identity. The fixture guard is the first retained classifier of the mismatch; the frozen evidence does not identify which earlier transition left the matching effect absent or displaced. |
| Shared resource | The persistence queue, pending correlations, and prepared fake-effect queue are owned by the one product inside the isolated recovery child. The child runtime and temporary journal path are unique; there is no cross-test object or path collision. |
| Scheduling dependence | Real-writer receipt readiness is asynchronous. If more than one persistence record is pending, `poll_one` moves a polled-yet-pending front entry to the back of the product-local persistence queue, so readiness and task scheduling can affect which typed poll is admitted next. The frozen evidence does not prove that this multi-entry prerequisite existed in the historical run. |
| Deterministic trigger | No exact historical trigger is authenticated. A fixed fact-before-intent sequence would demonstrate the generic-count defect class, but the retained log cannot prove that `FactAcknowledged`, another intent, an invalidated quote, or a failed intent was the historical transition. Manufacturing one of those states would not meet the prompt's historical-cause requirement. |
| Repair owner | If a separately reviewed goal authorizes defect-class hardening without claiming historical equivalence, the smallest owner is only `crates/reap-pm-live/src/evidence/workload.rs`: retain and validate the exact prepared-stage product-effect kind and identity produced by each intent acknowledgement, and fail before fixture dispatch when it is missing or wrong. No coordinator or live-product semantic change is justified by current evidence. |

The source-level defect class is concrete: `IntentFailed`,
`QuoteInvalidated`, `FactAcknowledged`, and a matching `PreparedQuote` or
`PreparedCancel` can all produce a serviced owner transition, but only the
matching prepared cases authorize the next fake-effect fixture. What remains
unknown is which concrete transition occurred in the frozen failure.

### Failure 2: secondary `InvalidRecords`

Frozen source anchors used for this trace are:

```text
crates/reap-pm-live/tests/combined_replay.rs=cf151a8372eddc9765f19c793d5f449f81e2633de509fc0ddbe1722da691c6dc
crates/reap-pm-live/src/composition/run_lifecycle.rs=916f53f8b843486545dc7c3ad480142640197b0ba8df7fd17c5b7fd1f9c9c548
crates/reap-pm-live/src/composition/run_types.rs=8737fb07635d393e60db1839a436d876e131b1057968b80eb1900160ba853c31
crates/reap-pm-live/src/capture/writer.rs=967b89c62dd6a2121561e6332ad7baacf32143129aba3553d8d695abf2c0658d
crates/reap-capture-framing/src/bounded_writer.rs=d3b339d5863db405bb039df98cc35c31ef80987bda90d97530b66092b5d5d6b3
crates/reap-pm-live/src/capture/verify.rs=7efafe34248231e092d84e9f4ef7aecb40d2f5e6b081a2363ff663df856737dd
```

| Field | Finding |
| --- | --- |
| Observed state | The aggregate subcase accepted 32 one-MiB raw frames, rejected the next byte with `RawPayloadTooLarge`, broadly accepted any `Err(TerminalFinish { .. })`, and then the independent verifier returned `InvalidRecords` at the frozen failing line. The exact `TerminalFinish.shutdown_error` was discarded. |
| Expected state | The capacity rejection should terminalize with `cause == CaptureWriter`; `finish()` must complete with `shutdown_error == None`; only then may the test verify the accepted 32-MiB prefix and its exact counters. |
| Divergence | Each successful `capture_pm_public` awaits bounded queue admission, not physical file durability. `finish()` does preserve writer-task I/O, join, timeout, and lifecycle/scan failures in `shutdown_error`, but four broad matches in the named test discard that field before verification. A truncated or otherwise invalid on-disk artifact can consequently surface later as secondary `InvalidRecords`. |
| Shared resource | The capture run owns a test-local tempfile path, bounded writer queue, and async writer task; source inspection shows no shared filename. Relevant shared host resources include filesystem capacity/throughput and task/runtime scheduling, but retained evidence does not identify which, if any, caused the historical shutdown failure. |
| Scheduling dependence | The aggregate admits roughly 44.8 MB (about 42.7 MiB) of encoded frames; the writer may drain concurrently, and `finish()` is the barrier that waits for any remaining writes under a 30-second shutdown timeout. The historical full binary lasted 39.86 seconds on a two-CPU host, but that whole-suite duration does not measure this writer's shutdown and therefore neither proves nor rules out a timeout. Writer I/O, join, timeout, and lifecycle/scan errors remain indistinguishable after the broad match. |
| Deterministic trigger | None is established in retained or currently allowlisted evidence. No historical tempdir artifact is retained, the exact `shutdown_error` was never logged, all five current executions containing this test passed, and the allowlisted support can observe an exact flushed sequence but cannot pause, stall, or inject a failure into the already-open private writer. |
| Repair owner | Unassigned under the current goal. A generic test hardening can assert `cause == CaptureWriter` and `shutdown_error == None` at the four sites before verification, but an exact primary-fault regression requires either a separately reviewed defect-class goal or a narrowly authorized test-only writer fault/barrier hook outside this allowlist. |

`JsonlWriterError::ShutdownTimeout` is a hypothesis, not a finding.
`InvalidRecords` is proven to be secondary-capable, but the retained evidence
does not distinguish shutdown timeout from writer I/O, join, or
lifecycle/evidence-scan failure. Goal G-R lines 410–419 explicitly prohibit
equating a newly manufactured I/O fault with this historical result.

### Stop decision and next owner

Phase 1 requires both exact causal explanations and deterministic pre-repair
triggers. Both defect classes are narrowed, but neither frozen failure retains
the exact transition needed by that gate. The stop condition is therefore:

```text
goal_g_r_phase1_stop_condition=historical-primary-state-not-retained-and-no-exact-in-allowlist-trigger
goal_g_r_phase1_historical_mutation_transition=unknown
goal_g_r_phase1_historical_capture_shutdown_error_variant=unknown
goal_g_r_phase1_shutdown_timeout_claimed=false
goal_g_r_phase1_manufactured_io_fault_equated=false
goal_g_r_phase1_regression_runner_created=false
goal_g_r_phase1_repair_source_edited=false
goal_g_r_phase1_phase2_started=false
goal_g_r_phase1_phase3_started=false
goal_g_r_phase1_goal_g_resumed=false
goal_g_r_phase1_goal_g_modified=false
goal_g_r_phase1_product_semantics_changed=false
goal_g_r_phase1_live_product_semantics_changed=false
```

The smallest next owner is a separately user-reviewed goal or amendment with
one of two explicit meanings:

1. Recommended: authorize an observability/defect-class hardening only in
   `workload.rs` and `combined_replay.rs`. It would validate exact
   acknowledgement/prepared-effect counter cuts and assert a clean writer
   terminal finish before prefix verification, while explicitly not claiming
   to reproduce the erased historical variants.
2. If exact fault reproduction remains mandatory: additionally authorize a
   private, test-only capture-writer barrier/fault hook and its colocated
   library regression. This widens the current allowlist and still requires
   the reviewed goal to name the injected primary variant rather than
   retroactively claiming it was historical.

Until one of those scopes is reviewed, the single pending regression-contract
field in the Phase 1 contract above remains authoritative. No regression
name, target, runner hash, or repair candidate is frozen by this stop record.

### Preservation and non-claims at stop

Immediately before this record, the fixed runtime root was empty, no matching
Cargo/rustc/combined-replay/Reap process remained, the tracked tree was clean,
and repository available bytes were `3416379392`, above the exact
`2147483648`-byte floor. The Goal G evidence anchors remained:

| Artifact | SHA-256 |
| --- | --- |
| `replay.selected` | `4168ac456d70361429967d7457e0d5850cd014c0b0ea7b8e45e3183372ec766d` |
| `replay/attempt-1/combined-replay.log` | `fe3e8c7323c52163345e6330ebd7587858990a49d1bc436a1a669792f6473cd9` |
| `replay/attempt-1/replay.meta` | `b2dc689182ea8c02fd340669b2b0f142b6cafd15d5ec38a04cda221f3aaa8f56` |
| `replay/attempt-1/replay.ps.tsv` | `fd77e0c1db9970bbe2c20eea70dc8836091a81e77d9bd66491c4d8150f4bf0c3` |

The complete Goal G file-hash and inventory streams remain
`35a99a10c133fd680cef1f4e411dbc55490f4e41199411aae907cd348aced340`
and
`23c4b85375e2d27e657c38b4560c3ee1bfecae1c1b5c98baf4cf1462dc05f7b2`,
with `11594` files and `12253` entries.

```text
production_order_entry_authorized=false
real_credentials_loaded=false
authenticated_external_request_sent=false
real_polygon_rpc_request_sent=false
real_order_submitted=false
goal_g_red_evidence_modified=false
goal_g_resumed=false
```

## Amendment 5 Authorization

The user authorized the recommended defect-class hardening track on
`2026-07-28` after identifying the target as an uncommitted repair summary
from a second agent. The controlling scope is
`docs/goal-g-replay-repair-amendment-5.md`.

Amendment 5 is separate from the stopped Goal G-R Phase 1 contract. It leaves
the single existing `goal_g_r_regression_contract_status=pending` field
unchanged, never invokes or mutates the frozen v5 runner, and cannot claim
Goal G-R or Goal G completion.

```text
goal_g_r_amendment_5_status=authorized
goal_g_r_amendment_5_schema=goal-g-r-amendment-5-v1
goal_g_r_amendment_5_base_commit=06b8948c8b3a2982ba9898c5215abc17e4f95893
goal_g_r_amendment_5_base_tree=d6cb7845616bec8d480c6c199956a79b58619687
goal_g_r_amendment_5_base_subject=docs: record goal g-r phase 1 causal stop
goal_g_r_amendment_5_cargo_lock_sha256=2673d055c943c3bd5444531b67df280026c145cbbbc99b68a06f4ac0c2dbb0ff
goal_g_r_amendment_5_regression_contract_schema=goal-g-r-amendment-5-regression-contract-v1
goal_g_r_amendment_5_regression_1_name=evidence::workload::tests::real_writer_acknowledgement_is_bound_to_expected_prepared_effect
goal_g_r_amendment_5_regression_1_target=lib
goal_g_r_amendment_5_regression_2_name=terminal_capture_finish_preserves_primary_shutdown_error_before_prefix_verification
goal_g_r_amendment_5_regression_2_target=combined_replay
goal_g_r_amendment_5_v5_invocation_authorized=false
goal_g_r_amendment_5_v5_mutation_authorized=false
goal_g_r_amendment_5_v5_regression_runner_creation_authorized=false
goal_g_r_amendment_5_historical_mutation_transition_claimed=false
goal_g_r_amendment_5_historical_capture_shutdown_variant_claimed=false
goal_g_r_amendment_5_goal_g_r_completion_claimed=false
goal_g_r_amendment_5_goal_g_resumed=false
goal_g_r_amendment_5_production_order_entry_authorized=false
goal_g_r_amendment_5_real_credentials_loaded=false
goal_g_r_amendment_5_external_request_authorized=false
goal_g_r_amendment_5_push_authorized=false
```
