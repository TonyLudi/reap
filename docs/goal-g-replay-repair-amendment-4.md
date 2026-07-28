# Goal G-R Amendment 4 — Static Validation Scheduling Repair

## Authority And Purpose

The user explicitly approved proceeding with this amendment on `2026-07-28`
after reviewing the Amendment 3 stop. This document is a narrow overlay on
`docs/goal-g-replay-repair-prompt.md` and Amendments 1–3. Every inherited
requirement remains in force unless this document expressly supersedes it.

Amendment 3 stopped at the closed `publish-reserve` gate. Its v4 runner
completed and validated all seven writable pending evidence files, but the
post-sampler schedule reached the manifest at least `187081264` nanoseconds
after the unchanged reserve cutoff. The runner had not started sealing or
atomic publication.

Amendment 4 authorizes one separately hashed v5 runner and one retained
no-Cargo bootstrap. V5 moves static semantic validation under the active
sampler, captures the validated static hashes before sampler shutdown, and
uses only hash-stability checks for those files after shutdown. It does not
authorize a product, test, fixture, dependency, capacity, timeout, Goal F,
Goal G, sibling-repository, credential, network, order, cleanup, or push
change.

## Immutable Starting Identity

The Amendment 3 stop, called S3, is:

```text
commit=6d33ea80c863b424c89ddce964b5b4374460ee81
tree=a03d8737cef1f4ff1c20f1ce6de33c1651bc653f
parent=7582d9fd92dbf67e54d02320307de4435cb52136
subject=docs: record goal g-r amendment 3 stop
handoff_sha256=4a70145b151f4307ebf62bff899c92b284fac909521c459041a03663bb01e323
handoff_blob=d9bf1b59c65f5abf1127346b6294bc94942e5ef7
handoff_size=33213
```

S3 is a non-merge direct child of the Amendment 3 authorization commit A3
and changes only `docs/goal-g-replay-repair-handoff.md`, mode `100644` to
`100644`, by appending the Amendment 3 stop record. Its sole raw diff is:

```text
:100644 100644 ba8208df66affccfd5427fa5d664d5546c9ccf3f d9bf1b59c65f5abf1127346b6294bc94942e5ef7 M	docs/goal-g-replay-repair-handoff.md
```

The complete 25925-byte A3 handoff is a byte-identical prefix of the handoff
at S3. No Amendment 3 activation commit or activation tail exists.

The frozen Amendment 3 document and `Cargo.lock` SHA-256 values are:

```text
amendment_3_sha256=39814ed9fb2ad1992bc14b7fe62753cd8f786886efc6f5c498c3b4710228d9d6
cargo_lock_sha256=2673d055c943c3bd5444531b67df280026c145cbbbc99b68a06f4ac0c2dbb0ff
```

All v1–v3, Goal F, Goal G, stopped-candidate, and historical absence anchors
inherited through Amendment 3 remain exact.

## Frozen V4 Stop Evidence

The complete v4 root is the nonsymlink mode-`700` directory:

```text
target/tmp/goal-g-replay-repair/amendment-3-v4/
```

Its top-level inventory is exactly:

```text
d	.runner-bootstrap-v4.pending
f	run-attempt.sh
f	run-attempt.sha256
```

Recursively it has 10 entries and nine regular files. Its frozen file-stream
and inventory SHA-256 values are:

```text
file_stream_sha256=782fe607a94ab46399d09f70667efba160ec7df441ad14b4bf5be29d4b9f485c
inventory_sha256=71d2bf42a749cca9613b5c82b878d60a6c2abc8257801ef69124ec34bb363dcc
```

The immutable v4 runner artifacts are:

| Artifact | SHA-256 | Mode |
| --- | --- | --- |
| `run-attempt.sh` | `f2d0c9761ecee3084bd8711a1c372ad4d939ab27e1960ca8d59810ad587cfe08` | `500` |
| `run-attempt.sha256` | `3516def3e27206f214957eed103af41d91171bb3f6db80932239bdad191a6eb6` | `400` |

The hash file contains exactly one newline-terminated line:

```text
f2d0c9761ecee3084bd8711a1c372ad4d939ab27e1960ca8d59810ad587cfe08  run-attempt.sh
```

The retained pending directory is:

```text
target/tmp/goal-g-replay-repair/amendment-3-v4/.runner-bootstrap-v4.pending/
```

It is a nonsymlink mode-`700` directory owned by the executing user and has
exactly seven nonsymlink regular mode-`600` files:

| Pending artifact | SHA-256 |
| --- | --- |
| `bootstrap.meta` | `926ff4f5665322b3147e89e1d8ac03b664535774cbef33a99626e98812a6d95d` |
| `bootstrap.sha256` | `3df5cf16715e849ed332bf6f24d25f2822175c2ef26a533047de25c7abc02003` |
| `goal-g-preservation.meta` | `cbdc7792311e06b45a8d56d9bbdc5d66ffdcdcf8b121c8bd6981c36bfd1c7675` |
| `matrix.tsv` | `7dfd76a33829817ba3cd9aa9ec2035e3fa6e08f1db62346175a022bdbf502edc` |
| `no-cargo.meta` | `6edd5bb1da6f2da3110d244d7b356fa4ed792ec0659d7d701af17e10fe4af8ee` |
| `process.ps.tsv` | `2a4aa0cc914115131482849d816767240f8d9a96040785728e09c8e5cedf17ac` |
| `v1-preservation.tsv` | `db82b18fa3795f4d482747cd2602dabe09f64f02d5cc48a126f4b3f4fc92964f` |

The pending file-stream and inventory SHA-256 values are:

```text
file_stream_sha256=2e2c5c0035500076d7a53bc367213ad9ed4dcaab3fda53ec07432e9cf9bc7806
inventory_sha256=089d3cd69a31a95070014fcf19b3216190e31a205d38646518c8142a0263f3bf
```

The six-entry manifest has the inherited exact sorted grammar and passes
`sha256sum -c`. The process log has a header, one pre row, 16 sampler rows,
and one `seal-tail-start` row; all 18 data rows are `sample-ok`. Both
independent post-stop process checks are recorded clear.

The exact timing evidence is:

```text
last_sample_epoch_ns=1785242873256426000
seal_tail_start_epoch_ns=1785242873531854187
seal_tail_deadline_epoch_ns=1785242874256426000
publish_reserve_ns=250000000
reserve_cutoff_epoch_ns=1785242874006426000
manifest_mtime_epoch_ns=1785242874193507264
reserve_miss_minimum_ns=187081264
```

The final `runner-bootstrap-v4` path is absent. V4 exited `65` at the exact
`publish-reserve` gate, with complete writable prepublication evidence and
without seal or atomic-publication attempt. During that sole v4 invocation,
no Cargo probe, Cargo workload, `setsid`, candidate, campaign, attempt, or
activation was created.

Despite their physical mode-`700`/`600` writability, the v4 runner and
pending directory are contract-immutable. V1, v2, v3, v4, the sealed v3
pending tree, and the writable v4 pending tree may never be invoked, retried,
edited, touched, chmodded, renamed, deleted, cleaned, promoted, or copied as
success.

The named and complete-tree Goal G anchors remain:

```text
selected_sha256=4168ac456d70361429967d7457e0d5850cd014c0b0ea7b8e45e3183372ec766d
log_sha256=fe3e8c7323c52163345e6330ebd7587858990a49d1bc436a1a669792f6473cd9
meta_sha256=b2dc689182ea8c02fd340669b2b0f142b6cafd15d5ec38a04cda221f3aaa8f56
process_sha256=fd77e0c1db9970bbe2c20eea70dc8836091a81e77d9bd66491c4d8150f4bf0c3
file_count=11594
entry_count=12253
file_stream_sha256=35a99a10c133fd680cef1f4e411dbc55490f4e41199411aae907cd348aced340
inventory_sha256=23c4b85375e2d27e657c38b4560c3ee1bfecae1c1b5c98baf4cf1462dc05f7b2
```

## V5 Runner Boundary

The one authorized successor bundle is:

```text
target/tmp/goal-g-replay-repair/amendment-4-v5/
```

It is absent at S3. After authorization and before bootstrap, it is a
nonsymlink mode-`700` directory owned by the executing user and contains
exactly:

```text
f	run-attempt.sh
f	run-attempt.sha256
```

The runner is mode `500`; its hash file is mode `400` and contains exactly
one newline-terminated line with grammar
`<64-lowercase-hex><two spaces>run-attempt.sh`.

The pending and final bootstrap paths are:

```text
target/tmp/goal-g-replay-repair/amendment-4-v5/.runner-bootstrap-v5.pending/
target/tmp/goal-g-replay-repair/amendment-4-v5/runner-bootstrap-v5/
```

V5 is mechanically derived from frozen v4. Necessary plumbing is closed to:

1. bind S3, Amendment 4 authorization, and Amendment 4 activation;
2. verify the complete frozen v4 stop evidence and final-path absence;
3. point self, hash, pending, final, and generation-specific metadata at v5;
4. add S3/v4 identities to bootstrap, launch, post-run, and attempt metadata;
5. substitute Amendment 4/v5 only in generation-specific schemas,
   diagnostics, subjects, and paths;
6. split and reschedule the existing static-evidence semantic verifier as
   defined below;
7. capture four validated static SHA-256 values before sampler shutdown and
   compare them after shutdown;
8. add only the closed gates and metadata fields defined by this amendment;
9. expand `v1-preservation.tsv` to cover the v4 runner, hash file, and all
   seven retained v4 pending files; bind the v4 stop facts in
   `no-cargo.meta` and `bootstrap.meta`; and
10. retain all inherited post-seal behavior byte-for-byte apart from
   generation-specific state names.

V5 must verify S3, every v1–v4 artifact, both retained pending trees, all
historical absence anchors, and Goal G before bootstrap, during the
active-sampler prepublication revalidation, during activation, and before
every real attempt. Activation and real-attempt checks occur outside the
one-second bootstrap tail.

The exact storage-preflight body, fixed matrix functions, and rendered
49-row matrix remain anchored by:

```text
storage_preflight_block_sha256=fe88dc9df88c320b27b414f780eca2a3c99701fb214d9ef98cb46076caea99bb
closed_matrix_function_block_sha256=157a47489216d0d9870fbf746945c21f0934c7ade941ae81ba5a46aa59145853
closed_matrix_tsv_sha256=7dfd76a33829817ba3cd9aa9ec2035e3fa6e08f1db62346175a022bdbf502edc
```

The sampler cadence, grammar, matching allowlist, PID/start-tick/PPID
ownership, signal ownership, reap behavior, and corrected `/proc` redirection
order remain unchanged. All real-attempt command mappings, validators,
paths, regression insertion, backtest finalization, Phase 1, and Phase 3
behavior remain unchanged.

## Static Validation Scheduling

While the sampler is active, after all static evidence has been rendered,
v5 must semantically validate:

```text
goal-g-preservation.meta
matrix.tsv
no-cargo.meta
v1-preservation.tsv
```

This list is exact: the live `process.ps.tsv` is not a static file.
`v1-preservation.tsv` retains every inherited row and adds the nine frozen v4
regular files. Directory inventories and modes remain separately verified.
`no-cargo.meta` records the v4 runner/hash, exit `65`, `publish-reserve`
subgate, complete-writable-retained manifest, absent final, unattempted
seal/publication, and no-Cargo/`setsid`/workload/candidate/attempt/activation
facts. `bootstrap.meta` binds S3, A4, all v4 retention anchors, and the static
validation phase booleans.

The validator retains the inherited pending directory inventory, file type,
mode, owner, matrix, Goal G metadata, no-Cargo metadata, and preservation
comparison checks. At this stage `process.ps.tsv` may be checked only for its
expected presence, regular-file type, mode, and owner. Its bytes, grammar,
tail, row count, and hash must not be inspected while the sampler can append.

After semantic validation succeeds and before sampler shutdown, v5 captures
the SHA-256 of each of the four static files. Each captured value must be
lowercase 64-hex; the matrix value must also equal the frozen matrix hash.
No later writer may modify a static file.

The complete inherited heavy identity, repository, predecessor, Goal G,
runtime, process, and storage checks remain under the active sampler. The
pre-stop matching-process scan remains after all moved static work.

After a clean sampler stop, the order is:

1. append exactly one `seal-tail-start` row;
2. derive the unchanged deadline from the final sampler timestamp;
3. validate the now-final process log and check interruption state;
4. run the first matching-process scan and record
   `post_stop_process_clear=true`;
5. recompute the four static hashes and require exact equality with their
   pre-stop values, without semantic reparse, preservation rerender, live
   evidence-tree walk, or reconstructed `cmp`;
6. hash the final process log;
7. run the second matching-process scan and record
   `pre_seal_process_clear=true`;
8. render bootstrap metadata and the six-entry manifest;
9. verify the complete seven-file mode-`600` writable pending directory,
   ownership, inventory, hashes, and manifest;
10. pass the unchanged reserve check;
11. chmod the seven files to `400` and pending to `500`;
12. execute the inherited same-parent atomic no-clobber `mv -nT`;
13. use only builtin final-present/pending-absent recognition; and
14. assign published state, remove only the EXIT cleanup trap, and return.

The complete writable verifier and manifest continue to hash all published
evidence. Static semantics are not re-evaluated after shutdown; byte equality
to the semantically validated pre-stop hashes supplies that proof.

The exact timing remains:

```text
deadline_origin=final-sampler-timestamp
seal_tail_bound_ns=1000000000
publish_reserve_ns=250000000
timeout_change_authorized=false
```

After sealing, the only filesystem mutation is the unguarded `mv -nT`.
Only failure-gate/publication-state assignment, nonmutating builtin
final/pending recognition, EXIT-cleanup-trap removal, and immediate return
are permitted around it. No post-publication external command or write is
allowed.

## Closed Bootstrap Failure Gates

Every runner-detected bootstrap failure and every caught `INT`, `TERM`, or
`HUP` that does not resolve by builtin recognition to an already-published
state reports one exact token:

```text
bootstrap-input
authorization
v1-preservation
v2-preservation
v3-preservation
v4-preservation
goal-g-named
bundle-precondition
process-precondition
runtime-precondition
repository-precondition
storage-precondition
pending-create
sampler-start
evidence-render
static-semantic-validation
static-hash-capture
heavy-revalidation
sampler-stop
seal-tail
static-hash-stability
bootstrap-metadata
manifest
writable-verification
publish-reserve
seal
atomic-publication
signal-int
signal-term
signal-hup
```

The runner rejects any other token. On failure it emits:

```text
Amendment 4 v5 bootstrap failed at <closed-gate-token>; retained state is immutable and may not be retried
```

A pending or partial path is retained. Nothing is deleted, retried, promoted,
or rewritten. State-aware signal handlers and atomic-publication recognition
remain as defined by Amendment 3.

## Authorization, Bootstrap, And Activation

The Amendment 4 authorization commit, called A4, must be the non-merge direct
child of S3, have exact subject:

```text
docs: authorize goal g-r runner amendment 4
```

and change exactly:

```text
docs/goal-g-replay-repair-amendment-4.md
docs/goal-g-replay-repair-handoff.md
```

The Amendment 4 document is added at mode `100644`; the handoff remains mode
`100644`. No rename or other tracked delta is allowed. The complete handoff
at A4 equals the 33213-byte handoff at S3 followed by exactly the reviewed
`## Amendment 4 Authorization` tail. Every
`goal_g_r_amendment_4_*` authorization key occurs exactly once. No
`goal_g_r_amendment_4_activation_*` key exists at A4.

After clean A4, create, independently review, hash, and freeze v5. Its first
and only bootstrap invocation is:

```text
run-attempt.sh <A4> runner-bootstrap-v5 00 no-cargo
```

The bootstrap branch is structurally before real candidate definitions,
real-attempt traps and directories, Cargo probing, `setsid`, and workload
launch. It renders but never evaluates or executes the 49 inherited matrix
rows.

The writable pending and successful final layout remains:

```text
bootstrap.meta
bootstrap.sha256
goal-g-preservation.meta
matrix.tsv
no-cargo.meta
process.ps.tsv
v1-preservation.tsv
```

Pending is mode `700` with mode-`600` files. The six-entry manifest excludes
itself and covers every other file. A successful final bundle is mode `500`
with mode-`400` files.

After a passing bootstrap, append the exact block below to the handoff. Every
key occurs exactly once:

```text
goal_g_r_amendment_4_activation_status=active
goal_g_r_amendment_4_activation_schema=goal-g-r-runner-amendment-4-activation-v1
goal_g_r_amendment_4_activation_authorization_commit=<A4>
goal_g_r_amendment_4_activation_authorization_tree=<A4-tree>
goal_g_r_amendment_4_activation_authorization_parent=6d33ea80c863b424c89ddce964b5b4374460ee81
goal_g_r_amendment_4_activation_authorization_subject=docs: authorize goal g-r runner amendment 4
goal_g_r_amendment_4_activation_amendment_sha256=<Amendment-4-document-SHA-256>
goal_g_r_amendment_4_activation_v5_runner_path=target/tmp/goal-g-replay-repair/amendment-4-v5/run-attempt.sh
goal_g_r_amendment_4_activation_v5_runner_sha256=<v5-runner-SHA-256>
goal_g_r_amendment_4_activation_v5_runner_mode=500
goal_g_r_amendment_4_activation_v5_hash_path=target/tmp/goal-g-replay-repair/amendment-4-v5/run-attempt.sha256
goal_g_r_amendment_4_activation_v5_hash_file_sha256=<v5-hash-file-SHA-256>
goal_g_r_amendment_4_activation_v5_hash_mode=400
goal_g_r_amendment_4_activation_v5_hash_content=<v5-runner-SHA-256>  run-attempt.sh
goal_g_r_amendment_4_activation_bundle_mode=700
goal_g_r_amendment_4_activation_bootstrap_path=target/tmp/goal-g-replay-repair/amendment-4-v5/runner-bootstrap-v5
goal_g_r_amendment_4_activation_bootstrap_mode=500
goal_g_r_amendment_4_activation_bootstrap_manifest_sha256=<bootstrap.sha256-SHA-256>
goal_g_r_amendment_4_activation_bootstrap_manifest_mode=400
goal_g_r_amendment_4_activation_bootstrap_preservation_sha256=<v1-preservation.tsv-SHA-256>
goal_g_r_amendment_4_activation_bootstrap_goal_g_sha256=<goal-g-preservation.meta-SHA-256>
goal_g_r_amendment_4_activation_bootstrap_matrix_sha256=7dfd76a33829817ba3cd9aa9ec2035e3fa6e08f1db62346175a022bdbf502edc
goal_g_r_amendment_4_activation_bootstrap_no_cargo_sha256=<no-cargo.meta-SHA-256>
goal_g_r_amendment_4_activation_bootstrap_process_sha256=<process.ps.tsv-SHA-256>
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

The activation block has one leading blank line, heading
`## Amendment 4 Activation`, one blank line, an opening `text` fence, the
fields above in displayed order with placeholders replaced, and a closing
fence followed by one newline.

The activation commit, called B4, is the non-merge direct child of A4 with
exact subject:

```text
docs: activate goal g-r amendment 4 runner
```

It changes only `docs/goal-g-replay-repair-handoff.md`, mode `100644` to
`100644`, by appending the exact activation block. B4 cannot record its own
ID. V5 derives B4 as the first ancestry-path commit after A4 and verifies its
parent, subject, raw diff, exact tail, and prefix preservation.

The first real candidate is clean B4 at:

```text
phase1-diagnostic/01-mutation-exact
```

The bootstrap consumes no ordinal. The other five diagnostic attempts,
causal proof, regression freeze, allowlisted repair, Phase 3, and final
handoff resume exactly under the original Goal G-R contract.

## Amendment 4 Stop Conditions

Stop without cleanup, retry, fallback, runner mutation, activation, or Cargo
when:

- S3, A3, their ancestry/diffs/handoffs, any v1–v4 artifact, either retained
  pending tree, Cargo.lock, Goal F, or Goal G differs;
- the v3 or v4 final path appears or a historical absent candidate path
  appears;
- v5 has an unexpected path, entry, owner, mode, hash, or source delta;
- v5 differs outside the closed derivation and scheduling boundary;
- `process.ps.tsv` is parsed or hashed while the sampler can append, a static
  baseline is captured before semantic validation, or a static hash changes
  after capture;
- either timing constant or the deadline origin changes;
- storage is below `2147483648` bytes or runtime is nonempty;
- bootstrap has a process overlap, signal, identity, matrix, manifest,
  attribution, deadline, seal, or publication failure;
- bootstrap reaches Cargo, `setsid`, candidate, campaign, or attempt creation;
- A4 or B4 has wrong ancestry, subject, paths, modes, bytes, or handoff tail;
- the first real attempt is not clean B4 at Phase 1 ordinal 01; or
- any inherited command, validator, bound, artifact, stop rule, or authority
  is relaxed.

A v5 bootstrap failure requires a separately user-reviewed Amendment 5.
V1, v2, v3, v4, and v5 may never be patched or rerun.
