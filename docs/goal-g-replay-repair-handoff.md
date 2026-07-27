# Goal G-R Replay Repair Handoff

## Status

Goal G-R Phase 0 is initialized. No replay, test, bench, build, clippy, or
backtest command has run under Goal G-R. Goal G remains stopped on its
immutable valid-red Amendment 2 evidence; this repair goal neither resumes nor
amends Goal G.

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
