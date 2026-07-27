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
