# Goal G-R Amendment 2 — Lexical Inventory Runner Repair

## Authority And Purpose

The user authorized this amendment on `2026-07-28` after reviewing the
Amendment 1 stop and directing Goal G-R to proceed. This document is a narrow
overlay on `docs/goal-g-replay-repair-prompt.md` and
`docs/goal-g-replay-repair-amendment-1.md`. Every original requirement remains
in force unless this document expressly supersedes it.

Amendment 1 stopped before its pending bootstrap directory was created. Its
frozen v2 runner rendered the required two-file bundle with `LC_ALL=C sort`
but compared the result with the two filenames reversed. The same reversal
also exists in its unreachable pending and final bundle predicates.

Amendment 2 authorizes one separately hashed v3 runner, correction of all
three lexical inventory literals, and one retained no-Cargo bootstrap. It
does not authorize a product, test, fixture, dependency, timeout, capacity,
Goal G, sibling-repository, credential, network, order, cleanup, or push
change.

## Immutable Starting Identity

The Amendment 1 stop, called S, is:

```text
commit=f93e5e450fb438a855a065a310e173940c5614ad
tree=1dbed0d66da931ffec6061c7986e7fbf7c70c248
parent=d23107df29ad318972a2ad8b869845cbf8fd3252
subject=docs: record goal g-r amendment 1 stop
handoff_sha256=933256bfb2b2a4c73cc9c950b439c7ac674ff33cc26c555221e0ff8de1000c59
handoff_blob=9c14ae636e925a26431eb6daf58abbdb392c3f19
handoff_size=12405
```

S is a non-merge direct child of the Amendment 1 authorization commit and
changes only `docs/goal-g-replay-repair-handoff.md`, mode `100644` to
`100644`, by appending the Amendment 1 stop record. The complete handoff at
its parent is a byte-identical prefix of the handoff at S.

The frozen `Cargo.lock` SHA-256 remains:

```text
2673d055c943c3bd5444531b67df280026c145cbbbc99b68a06f4ac0c2dbb0ff
```

## Immutable Predecessor Runners And Evidence

The four original v1 files remain governed by Amendment 1 and may never be
edited, renamed, deleted, replaced, touched, or chmodded:

| Artifact | SHA-256 | Mode |
| --- | --- | --- |
| `target/tmp/goal-g-replay-repair/run-attempt.sh` | `16d61de14b41a5551d8632e9599aa6ee54fb68d2a7dc00eca5d74d7ac351d1fc` | `500` |
| `target/tmp/goal-g-replay-repair/run-attempt.sha256` | `6c109abc2e1e0f9792c8817b5d978f789d229f37390087f58ea52dfb60a94c43` | `400` |
| `target/tmp/goal-g-replay-repair/phase0-start-attestation.txt` | `e94acfd7299b32aacc1bd54f4ecfcd018103bd462b84de9d21f1da496b46cf67` | `400` |
| `target/tmp/goal-g-replay-repair/phase0-start-attestation.sha256` | `a4a6bac51877a44b1ed13e26b9d7aa0c68e8bf73c6c8b1787a45f3f1bf1d00e0` | `400` |

The complete frozen v2 bundle also remains immutable:

```text
target/tmp/goal-g-replay-repair/amendment-1-v2/  mode 700
```

It contains exactly:

| Artifact | SHA-256 | Mode |
| --- | --- | --- |
| `run-attempt.sh` | `221ab5f04f3a72047b0ff66ec70a827b608978baa99ba0f266f8cb30b99dd37c` | `500` |
| `run-attempt.sha256` | `b2ef7aa095142dbc693fa6d45d62c81105d30bc9e19e6bab600d8fc3860520ff` | `400` |

The v2 hash file has the exact one-line content:

```text
221ab5f04f3a72047b0ff66ec70a827b608978baa99ba0f266f8cb30b99dd37c  run-attempt.sh
```

The paths below remain absent:

```text
target/tmp/goal-g-replay-repair/e992363c6aa75680b3479bb0a805db813355acbc
target/tmp/goal-g-replay-repair/d23107df29ad318972a2ad8b869845cbf8fd3252
target/tmp/goal-g-replay-repair/amendment-1-v2/.runner-bootstrap-v2.pending
target/tmp/goal-g-replay-repair/amendment-1-v2/runner-bootstrap-v2
target/tmp/goal-g-replay-repair/amendment-2-v3
```

The v3 bundle path is absent at S and remains absent until it is created from
a clean authorization commit C.

The v1 and v2 bootstrap failures are not attempt ordinal 01. Neither may be
invoked again, and no fabricated candidate, attempt, pending, or final
directory may represent them.

All named and complete-tree Goal G anchors remain exact:

```text
replay.selected=4168ac456d70361429967d7457e0d5850cd014c0b0ea7b8e45e3183372ec766d
combined-replay.log=fe3e8c7323c52163345e6330ebd7587858990a49d1bc436a1a669792f6473cd9
replay.meta=b2dc689182ea8c02fd340669b2b0f142b6cafd15d5ec38a04cda221f3aaa8f56
replay.ps.tsv=fd77e0c1db9970bbe2c20eea70dc8836091a81e77d9bd66491c4d8150f4bf0c3
file_count=11594
entry_count=12253
file_stream_sha256=35a99a10c133fd680cef1f4e411dbc55490f4e41199411aae907cd348aced340
inventory_sha256=23c4b85375e2d27e657c38b4560c3ee1bfecae1c1b5c98baf4cf1462dc05f7b2
```

Every frozen Goal F anchor and original Goal G-R rule remains inherited.

## V3 Runner Boundary

The one authorized successor bundle is:

```text
target/tmp/goal-g-replay-repair/amendment-2-v3/
```

It is a nonsymlink mode-`700` directory owned by the executing user. Before
bootstrap it contains exactly these two nonsymlink regular files:

```text
run-attempt.sh          mode 500
run-attempt.sha256      mode 400
```

The hash file contains exactly one line, terminated by one newline, with
grammar `<64-lowercase-hex><two spaces>run-attempt.sh`. Existing real-attempt
paths remain unchanged:

```text
target/tmp/goal-g-replay-repair/<candidate-head>/<campaign>/<ordinal>-<label>/
```

The v3 runner is mechanically derived from frozen v2. Necessary
generation-only plumbing is closed to:

1. bind Amendment 2 authorization and activation commits;
2. verify S and the complete frozen v2 stop evidence;
3. point self, hash, pending, and final paths at the v3 bundle;
4. substitute Amendment 2/v3 only in the activation, bootstrap,
   goal-preservation, no-Cargo, runner-generation, and amendment-anchor
   schemas/fields explicitly enumerated by this document;
5. substitute `Amendment 2`, `v3`, and the exact new subjects/paths only in
   generation-specific diagnostics and metadata; and
6. add the v2 identities and stop anchors to bootstrap, launch, post-run, and
   attempt-metadata verification.

The only behavioral delta is correction of the three exact C-sort inventory
literals:

```text
f	run-attempt.sh
f	run-attempt.sha256
```

```text
d	.runner-bootstrap-v3.pending
f	run-attempt.sh
f	run-attempt.sha256
```

```text
d	runner-bootstrap-v3
f	run-attempt.sh
f	run-attempt.sha256
```

The exact storage-preflight body, fixed matrix functions, and rendered matrix
remain anchored by:

```text
storage_preflight_block_sha256=fe88dc9df88c320b27b414f780eca2a3c99701fb214d9ef98cb46076caea99bb
closed_matrix_function_block_sha256=157a47489216d0d9870fbf746945c21f0934c7ade941ae81ba5a46aa59145853
closed_matrix_tsv_sha256=7dfd76a33829817ba3cd9aa9ec2035e3fa6e08f1db62346175a022bdbf502edc
```

All other v2 command mappings, validators, bounds, attempt paths,
predecessor rules, process ownership, signal handling, sampler grammar,
one-second seal tail, ctime publication proof, same-filesystem atomic
`mv -nT`, manifest, regression insertion, and backtest finalization remain
behaviorally unchanged. Unrelated schemas containing `v2` are not renamed.

## Authorization, Bootstrap, And Activation

The Amendment 2 authorization commit, called C, must be the non-merge direct
child of S, have exact subject:

```text
docs: authorize goal g-r runner amendment 2
```

and change exactly:

```text
docs/goal-g-replay-repair-amendment-2.md
docs/goal-g-replay-repair-handoff.md
```

Both paths remain mode `100644`; no rename or other tracked delta is allowed.
The raw diff has exactly two records: the Amendment 2 document is added from
mode `000000` and the zero blob to mode `100644`; the handoff is modified
from mode `100644`, blob
`9c14ae636e925a26431eb6daf58abbdb392c3f19`, to mode `100644`. C has one
parent and no rename.

The complete handoff at C equals the 12405-byte handoff at S followed by
exactly the reviewed `## Amendment 2 Authorization` tail in this commit.
Every `goal_g_r_amendment_2_*` authorization key in that tail occurs exactly
once in the complete handoff. No
`goal_g_r_amendment_2_activation_*` key exists at C. After C is created, v3
pins C's tree, both new blobs, the Amendment 2 SHA-256, and the complete
C-handoff SHA-256.

After clean C, create, independently review, hash, and freeze v3. Its first
and only bootstrap invocation is:

```text
run-attempt.sh <C> runner-bootstrap-v3 00 no-cargo
```

The branch is structurally before real candidate definitions, real attempt
traps/directories, Cargo probing, `setsid`, and workload launch. It renders
but never evaluates or executes all 49 original fixed matrix rows. It uses
the unchanged two-GiB storage gate, empty runtime gate, complete v1/v2/Goal G
verification, process sampler, deadline proof, and atomic no-clobber
publication.

Pending and final paths are:

```text
target/tmp/goal-g-replay-repair/amendment-2-v3/.runner-bootstrap-v3.pending/
target/tmp/goal-g-replay-repair/amendment-2-v3/runner-bootstrap-v3/
```

The bootstrap layout, seven filenames, six-file manifest, modes, ownership,
hash verification, and process evidence remain exactly as defined by
Amendment 1. A failure before pending creation legitimately leaves both paths
absent; a failure after creation retains pending. Nothing is deleted or
retried.

After a passing bootstrap, append the exact block below to the handoff. Every
key must occur exactly once in the complete handoff. Existing Amendment 1
fields and the Amendment 2 authorization-time `pending` fields remain
unchanged historical records and are not duplicated:

```text
goal_g_r_amendment_2_activation_status=active
goal_g_r_amendment_2_activation_schema=goal-g-r-runner-amendment-2-activation-v1
goal_g_r_amendment_2_activation_authorization_commit=<C>
goal_g_r_amendment_2_activation_authorization_tree=<C-tree>
goal_g_r_amendment_2_activation_authorization_parent=f93e5e450fb438a855a065a310e173940c5614ad
goal_g_r_amendment_2_activation_authorization_subject=docs: authorize goal g-r runner amendment 2
goal_g_r_amendment_2_activation_amendment_sha256=<Amendment-2-document-SHA-256>
goal_g_r_amendment_2_activation_v3_runner_path=target/tmp/goal-g-replay-repair/amendment-2-v3/run-attempt.sh
goal_g_r_amendment_2_activation_v3_runner_sha256=<v3-runner-SHA-256>
goal_g_r_amendment_2_activation_v3_runner_mode=500
goal_g_r_amendment_2_activation_v3_hash_path=target/tmp/goal-g-replay-repair/amendment-2-v3/run-attempt.sha256
goal_g_r_amendment_2_activation_v3_hash_file_sha256=<v3-hash-file-SHA-256>
goal_g_r_amendment_2_activation_v3_hash_mode=400
goal_g_r_amendment_2_activation_v3_hash_content=<v3-runner-SHA-256>  run-attempt.sh
goal_g_r_amendment_2_activation_bundle_mode=700
goal_g_r_amendment_2_activation_bootstrap_path=target/tmp/goal-g-replay-repair/amendment-2-v3/runner-bootstrap-v3
goal_g_r_amendment_2_activation_bootstrap_mode=500
goal_g_r_amendment_2_activation_bootstrap_manifest_sha256=<bootstrap.sha256-SHA-256>
goal_g_r_amendment_2_activation_bootstrap_manifest_mode=400
goal_g_r_amendment_2_activation_cargo_invoked_between_c_and_d=false
goal_g_r_amendment_2_activation_setsid_invoked_between_c_and_d=false
goal_g_r_amendment_2_activation_workload_invoked_between_c_and_d=false
goal_g_r_amendment_2_activation_first_campaign=phase1-diagnostic
goal_g_r_amendment_2_activation_first_ordinal=01
goal_g_r_amendment_2_activation_first_label=mutation-exact
```

The activation block has one leading blank line, heading
`## Amendment 2 Activation`, one blank line, opening ```` ```text ````, the
fields above in their displayed order with the placeholders replaced by
actual lowercase hashes, and closing ```` ``` ```` followed by one newline.

The activation commit, called D, must be the non-merge direct child of C,
have exact subject:

```text
docs: activate goal g-r amendment 2 runner
```

and change only `docs/goal-g-replay-repair-handoff.md`, mode `100644` to
`100644`. D cannot record its own ID. V3 derives D as the first ancestry-path
commit after C and verifies its parent, subject, raw diff, exact appended
activation tail, and prefix preservation.

The first real candidate is clean D at:

```text
phase1-diagnostic/01-mutation-exact
```

The bootstrap consumes no ordinal. The other five diagnostic attempts,
causal proof, regression freeze, allowlisted repair, Phase 3, and final
handoff then resume exactly under the original Goal G-R contract. The
user-supplied shared async-writer masking explanation is a Phase 1 hypothesis,
not a predetermined result; the retained transition evidence must prove or
reject it.

## Amendment 2 Stop Conditions

Stop without cleanup, retry, fallback, runner mutation, activation, or Cargo
when:

- S, its ancestry/diff/handoff, any v1/v2 artifact, either historical absent
  candidate/bootstrap path, Cargo.lock, Goal F, or Goal G differs;
- v3 has an unexpected path, entry, owner, mode, hash, or source delta;
- v3 differs behaviorally outside the three lexical literals;
- storage is below `2147483648` bytes or runtime is nonempty;
- bootstrap has process overlap, a signal, identity/matrix/manifest/deadline
  mismatch, or non-atomic publication;
- bootstrap reaches Cargo, `setsid`, candidate, campaign, or attempt creation;
- C or D has wrong ancestry, subject, paths, modes, bytes, or activation tail;
- the first real attempt is not clean D at original Phase 1 ordinal 01; or
- any original command, validator, bound, artifact, or authority is relaxed.

A v3 bootstrap failure requires a separately user-reviewed Amendment 3. The
frozen v3 runner may not be patched or rerun under Amendment 2.
