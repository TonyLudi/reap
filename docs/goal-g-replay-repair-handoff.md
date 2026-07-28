# Goal G-R Replay Repair Handoff

## Status

Goal G-R is stopped at the first Phase 1 runner invocation. No replay, test,
bench, build, clippy, or backtest command ran. The frozen runner exited during
its own storage preflight before it created an attempt directory or launched
Cargo. Goal G remains stopped on its immutable valid-red Amendment 2
evidence; this repair goal neither resumes nor amends Goal G.

Recorded at `2026-07-27T17:07:23Z`.

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
