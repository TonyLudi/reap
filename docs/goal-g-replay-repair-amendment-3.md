# Goal G-R Amendment 3 — Prepublication Tail Repair

## Authority And Purpose

The user explicitly approved this amendment on `2026-07-28` after reviewing
the Amendment 2 stop. This document is a narrow overlay on
`docs/goal-g-replay-repair-prompt.md`,
`docs/goal-g-replay-repair-amendment-1.md`, and
`docs/goal-g-replay-repair-amendment-2.md`. Every inherited requirement
remains in force unless this document expressly supersedes it.

Amendment 2 stopped after its v3 pending bootstrap evidence was completely
rendered, manifested, and sealed, but before atomic publication. The exact
post-seal predicate was not durably recorded. The `250000000`-nanosecond
publication-reserve checks are the leading hypothesis, not a proven cause.
Five `/proc/<pid>/stat` messages were separately proven to be benign
enumerate/open races caused by redirection order.

Amendment 3 authorizes one separately hashed v4 runner and one retained
no-Cargo bootstrap. V4 preserves the sealed v3 pending evidence, moves
redundant heavyweight verification before sampler shutdown, ensures that
only atomic publication and nonmutating builtin state recognition remain
after sealing, uses a closed failure-gate vocabulary, and corrects only the
sampler redirection order. It does not authorize any product, test, fixture,
dependency, capacity, timeout, Goal G, sibling-repository, credential,
network, order, cleanup, or push change.

## Immutable Starting Identity

The Amendment 2 stop, called E, is:

```text
commit=40137d7036546b57e3930252d732157e3db37283
tree=c8b63b25b8f0de0b70791d97c7aec26ddd82b9f2
parent=e9dd15017f15a6853513a517dcf41d80bdc8cf7f
subject=docs: record goal g-r amendment 2 stop
handoff_sha256=a8e9091da9d8926ce4e3752e1907959488e6b6c2d0d89843a56ae6bafec70107
handoff_blob=5c31cfb1da6e8050588583032efd6b2ca8bb608d
handoff_size=21355
```

E is a non-merge direct child of the Amendment 2 authorization commit C and
changes only `docs/goal-g-replay-repair-handoff.md`, mode `100644` to
`100644`, by appending the Amendment 2 stop record. Its sole raw diff is:

```text
:100644 100644 9055b6701d0307095f310d93a74ec6d5d4fdacbb 5c31cfb1da6e8050588583032efd6b2ca8bb608d M	docs/goal-g-replay-repair-handoff.md
```

The complete 15268-byte C handoff is a byte-identical prefix of the handoff
at E. No Amendment 2 activation commit or activation tail exists.

The frozen `Cargo.lock` SHA-256 remains:

```text
2673d055c943c3bd5444531b67df280026c145cbbbc99b68a06f4ac0c2dbb0ff
```

The v1 and v2 runner, hash-file, attestation, stop, absence, and Goal G
anchors inherited by Amendments 1 and 2 remain exact.

## Frozen V3 Stop Evidence

The complete frozen v3 root is the nonsymlink mode-`700` directory:

```text
target/tmp/goal-g-replay-repair/amendment-2-v3/
```

It has exactly 10 entries and nine regular files. Its top-level inventory is:

```text
d	.runner-bootstrap-v3.pending
f	run-attempt.sh
f	run-attempt.sha256
```

The immutable v3 runner artifacts are:

| Artifact | SHA-256 | Mode |
| --- | --- | --- |
| `run-attempt.sh` | `a2e5a7f77feffc8832616dd3c13d06eba80fb5fd2082dda7bf8d5c504d0ab8ec` | `500` |
| `run-attempt.sha256` | `de1b58342678678b92fa610380d5707554cd4530c468969878c2ecfbfe3e45bc` | `400` |

The hash file has exactly one newline-terminated line with the required
two-space grammar. V3's pending directory is mode `500`, owned by the
executing user, and contains exactly seven nonsymlink mode-`400` files:

| Pending artifact | SHA-256 |
| --- | --- |
| `bootstrap.meta` | `9dab6c18e2715356572d174cfd0d65acbb37f6e3435c326599762d1d1f3a5a37` |
| `bootstrap.sha256` | `71ca7b6807f808a07b545325e59efc07deceef7144ea3f2d6a5920fcb96d07b7` |
| `goal-g-preservation.meta` | `588a37b4a7f0425a8de5bf1a4cb0498f7da9fb4f7c195524ab925bde33722dbb` |
| `matrix.tsv` | `7dfd76a33829817ba3cd9aa9ec2035e3fa6e08f1db62346175a022bdbf502edc` |
| `no-cargo.meta` | `b2dd67acc729cd4fa685cb2a7fd098dec96a2a76e30a6f72db357fec4489a19e` |
| `process.ps.tsv` | `f338cb3f8dcaaac866552a598364b1d37da259bb6f9d3b19479cc422172825f4` |
| `v1-preservation.tsv` | `584df2493aa7b47d3fef32475d34e5cd9872d516fad5c8b205bb12d2d46180c8` |

The six-line manifest has exact sorted grammar and passes `sha256sum -c`.
The final v3 bootstrap path is absent. The v3 process log contains a header,
one pre row, 17 sampler rows, and one seal-tail-start row; every data row is
`sample-ok` and no overlap was recorded.

V3 and its pending directory may never be invoked, patched, copied as
success, renamed, chmodded, deleted, or cleaned. V4 must verify all of the
identities above before bootstrap, before publication, during activation,
and before every real attempt.

All named and complete-tree Goal G anchors remain exact:

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

## V4 Runner Boundary

The one authorized successor bundle is:

```text
target/tmp/goal-g-replay-repair/amendment-3-v4/
```

It is absent at E. After authorization and before bootstrap it is a
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
target/tmp/goal-g-replay-repair/amendment-3-v4/.runner-bootstrap-v4.pending/
target/tmp/goal-g-replay-repair/amendment-3-v4/runner-bootstrap-v4/
```

V4 is mechanically derived from frozen v3. Necessary plumbing is closed to:

1. bind E, Amendment 3 authorization, and Amendment 3 activation;
2. verify the complete frozen v3 stop evidence and final-path absence;
3. point self, hash, pending, final, and generation-specific metadata at v4;
4. add E/v3 identities to bootstrap, launch, post-run, and attempt metadata;
5. substitute Amendment 3/v4 only in generation-specific schemas,
   diagnostics, subjects, and paths;
6. move the redundant heavyweight immutable, complete Goal G, and duplicate
   storage verification before sampler shutdown, while retaining both
   inherited post-stop/pre-seal matching-process scans and every inherited
   storage preflight immediately before file or log creation;
7. close and report the active bootstrap failure gate;
8. after sealing, permit only `mv -nT`, nonmutating builtin final/pending
   recognition, state assignment, EXIT-cleanup-trap removal, and immediate
   return, with no other external command or filesystem write; and
9. reorder only the sampler's transient `/proc` read redirections.

The exact sampler source delta is:

```diff
-      IFS= read -r stat_line <"$stat_path" 2>/dev/null || continue
+      IFS= read -r stat_line 2>/dev/null <"$stat_path" || continue
```

The glob, parser, matching allowlist, process-log grammar, cadence,
PID/start-tick/PPID ownership, signal ownership, and reap behavior remain
unchanged. Publication-state signal resolution is made explicit: before
commit, caught `INT`, `TERM`, or `HUP` reports its closed signal token and
retains pending; while committing, the handler uses only builtin final/pending
recognition, treats final-present plus pending-absent as published, and
otherwise reports the signal stop without a filesystem write.

The exact storage-preflight body, fixed matrix functions, and rendered
49-row matrix remain anchored by:

```text
storage_preflight_block_sha256=fe88dc9df88c320b27b414f780eca2a3c99701fb214d9ef98cb46076caea99bb
closed_matrix_function_block_sha256=157a47489216d0d9870fbf746945c21f0934c7ade941ae81ba5a46aa59145853
closed_matrix_tsv_sha256=7dfd76a33829817ba3cd9aa9ec2035e3fa6e08f1db62346175a022bdbf502edc
```

The exact timing constants remain:

```text
seal_tail_bound_ns=1000000000
publish_reserve_ns=250000000
timeout_change_authorized=false
```

The deadline continues to equal the final sampler timestamp plus
`1000000000`; Amendment 3 does not redefine its origin or increase either
bound.

All other v3 command mappings, validators, real-attempt paths, attempt
sealing, regression insertion, backtest finalization, and Phase 1/Phase 3
behavior remain unchanged. Unrelated schemas containing `v2` or `v3` are not
renamed.

## Closed Bootstrap Failure Gates

Every runner-detected bootstrap failure and every caught `INT`, `TERM`, or
`HUP` that does not resolve by builtin recognition to the already-published
state reports one of these exact tokens. A signal that resolves to
final-present plus pending-absent preserves the successful publication
outcome without a failure diagnostic. `SIGKILL`, power loss, and host loss
cannot emit a diagnostic:

```text
bootstrap-input
authorization
v1-preservation
v2-preservation
v3-preservation
goal-g-named
bundle-precondition
process-precondition
runtime-precondition
repository-precondition
storage-precondition
pending-create
sampler-start
evidence-render
heavy-revalidation
sampler-stop
seal-tail
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
Amendment 3 v4 bootstrap failed at <closed-gate-token>; retained state is immutable and may not be retried
```

A pending or partial path is retained. Nothing is deleted, retried, promoted,
or rewritten. After the seven evidence files and pending directory are
sealed, the normal-flow active token is `atomic-publication`. The only
permitted filesystem mutation is same-parent atomic no-clobber `mv -nT` to
the final path. Because `mv -n` may return success after declining to replace
an existing destination, it is followed by the required nonmutating builtin
recognition `[[ -d $bootstrap_final && ! -e $bootstrap_pending ]]`, then
state assignment, removal of only the EXIT cleanup trap, and immediate
return. State-aware signal handlers remain installed through that return. No
external command or write is permitted after successful publication. Sealed
pending plus absent final therefore identifies atomic publication in normal
flow; a separately emitted closed signal token identifies a caught
asynchronous interruption.

## Authorization, Bootstrap, And Activation

The Amendment 3 authorization commit, called A3, must be the non-merge direct
child of E, have exact subject:

```text
docs: authorize goal g-r runner amendment 3
```

and change exactly:

```text
docs/goal-g-replay-repair-amendment-3.md
docs/goal-g-replay-repair-handoff.md
```

The Amendment 3 document is added at mode `100644`; the handoff remains mode
`100644`. No rename or other tracked delta is allowed. The complete handoff
at A3 equals the 21355-byte handoff at E followed by exactly the reviewed
`## Amendment 3 Authorization` tail in A3. Every
`goal_g_r_amendment_3_*` authorization key occurs exactly once. No
`goal_g_r_amendment_3_activation_*` key exists at A3.

After clean A3, create, independently review, hash, and freeze v4. Its first
and only bootstrap invocation is:

```text
run-attempt.sh <A3> runner-bootstrap-v4 00 no-cargo
```

The branch is structurally before real candidate definitions, real attempt
traps/directories, Cargo probing, `setsid`, and workload launch. It renders
but never evaluates or executes all 49 inherited fixed matrix rows.

The writable pending directory initially has mode `700`, and each evidence
file is mode `600` under the inherited `umask 077`. Its successful
sealed/final layout remains the inherited seven files:

```text
bootstrap.meta
bootstrap.sha256
goal-g-preservation.meta
matrix.tsv
no-cargo.meta
process.ps.tsv
v1-preservation.tsv
```

The six-entry manifest excludes itself and covers every other file. While the
sampler is active, v4 performs complete repository, v1/v2/v3, Goal G,
runtime, process-precondition, storage, matrix, and static-content
validation. The final process-log bytes and manifest do not exist until the
sampler stops. After a clean sampler stop:

1. append exactly one `seal-tail-start` row;
2. derive the unchanged deadline from the final sampler timestamp;
3. verify the final process log;
4. run a matching-process scan and record `post_stop_process_clear=true`;
5. verify the writable static evidence and perform every remaining
   non-process pre-seal check;
6. run a second matching-process scan in the final writable-verification
   phase and record `pre_seal_process_clear=true`;
7. render bootstrap metadata with both clear booleans and render the
   six-entry manifest;
8. verify complete mode-`600` writable pending content, hashes, ownership,
   inventory, and manifest;
9. pass the unchanged deadline check with the `250000000` reserve;
10. chmod all seven files to `400` and the pending directory to `500`;
11. set committing state and execute atomic no-clobber `mv -nT`;
12. use only builtin final/pending recognition; and
13. assign published state, remove only the EXIT cleanup trap, and return
    immediately with state-aware signal handlers still installed.

After successful publication there is no external command, filesystem write,
or cleanup. The builtin recognition detects both a successful rename and a
no-clobber skip. On success, activation verifies final inventory, hashes,
publication ctime, process evidence, all predecessors, and the unchanged
deadline. Bootstrap metadata records
`prepublication_evidence_complete=true`; it does not claim publication
success before the rename.

After a passing bootstrap, append the exact block below to the handoff. Every
key occurs exactly once:

```text
goal_g_r_amendment_3_activation_status=active
goal_g_r_amendment_3_activation_schema=goal-g-r-runner-amendment-3-activation-v1
goal_g_r_amendment_3_activation_authorization_commit=<A3>
goal_g_r_amendment_3_activation_authorization_tree=<A3-tree>
goal_g_r_amendment_3_activation_authorization_parent=40137d7036546b57e3930252d732157e3db37283
goal_g_r_amendment_3_activation_authorization_subject=docs: authorize goal g-r runner amendment 3
goal_g_r_amendment_3_activation_amendment_sha256=<Amendment-3-document-SHA-256>
goal_g_r_amendment_3_activation_v4_runner_path=target/tmp/goal-g-replay-repair/amendment-3-v4/run-attempt.sh
goal_g_r_amendment_3_activation_v4_runner_sha256=<v4-runner-SHA-256>
goal_g_r_amendment_3_activation_v4_runner_mode=500
goal_g_r_amendment_3_activation_v4_hash_path=target/tmp/goal-g-replay-repair/amendment-3-v4/run-attempt.sha256
goal_g_r_amendment_3_activation_v4_hash_file_sha256=<v4-hash-file-SHA-256>
goal_g_r_amendment_3_activation_v4_hash_mode=400
goal_g_r_amendment_3_activation_v4_hash_content=<v4-runner-SHA-256>  run-attempt.sh
goal_g_r_amendment_3_activation_bundle_mode=700
goal_g_r_amendment_3_activation_bootstrap_path=target/tmp/goal-g-replay-repair/amendment-3-v4/runner-bootstrap-v4
goal_g_r_amendment_3_activation_bootstrap_mode=500
goal_g_r_amendment_3_activation_bootstrap_manifest_sha256=<bootstrap.sha256-SHA-256>
goal_g_r_amendment_3_activation_bootstrap_manifest_mode=400
goal_g_r_amendment_3_activation_v3_pending_manifest_sha256=71ca7b6807f808a07b545325e59efc07deceef7144ea3f2d6a5920fcb96d07b7
goal_g_r_amendment_3_activation_seal_tail_bound_ns=1000000000
goal_g_r_amendment_3_activation_publish_reserve_ns=250000000
goal_g_r_amendment_3_activation_cargo_invoked_between_a3_and_b3=false
goal_g_r_amendment_3_activation_setsid_invoked_between_a3_and_b3=false
goal_g_r_amendment_3_activation_workload_invoked_between_a3_and_b3=false
goal_g_r_amendment_3_activation_first_campaign=phase1-diagnostic
goal_g_r_amendment_3_activation_first_ordinal=01
goal_g_r_amendment_3_activation_first_label=mutation-exact
```

The activation block has one leading blank line, heading
`## Amendment 3 Activation`, one blank line, opening ```` ```text ````, the
fields above in displayed order with placeholders replaced, and closing
```` ``` ```` followed by one newline.

The activation commit, called B3, is a non-merge direct child of A3 with
exact subject:

```text
docs: activate goal g-r amendment 3 runner
```

It changes only `docs/goal-g-replay-repair-handoff.md`, mode `100644` to
`100644`, by appending the exact activation block. B3 cannot record its own
ID. V4 derives B3 as the first ancestry-path commit after A3 and verifies
parent, subject, raw diff, exact tail, and prefix preservation.

The first real candidate is clean B3 at:

```text
phase1-diagnostic/01-mutation-exact
```

The bootstrap consumes no ordinal. The other five diagnostic attempts,
causal proof, regression freeze, allowlisted repair, Phase 3, and final
handoff resume exactly under the original Goal G-R contract.

## Amendment 3 Stop Conditions

Stop without cleanup, retry, fallback, runner mutation, activation, or Cargo
when:

- E, its ancestry/diff/handoff, C/S, any v1/v2/v3 artifact, the sealed v3
  pending evidence, Cargo.lock, Goal F, or Goal G differs;
- the v3 final path appears or any historical absent candidate path appears;
- v4 has an unexpected path, entry, owner, mode, hash, or source delta;
- v4 differs outside the closed derivation boundary;
- either timing constant or the deadline origin changes;
- storage is below `2147483648` bytes or runtime is nonempty;
- bootstrap has a process overlap, signal, identity, matrix, manifest,
  attribution, deadline, seal, or publication failure;
- bootstrap reaches Cargo, `setsid`, candidate, campaign, or attempt creation;
- A3 or B3 has wrong ancestry, subject, paths, modes, bytes, or handoff tail;
- the first real attempt is not clean B3 at Phase 1 ordinal 01; or
- any inherited command, validator, bound, artifact, stop rule, or authority
  is relaxed.

A v4 bootstrap failure requires a separately user-reviewed Amendment 4.
V1, v2, v3, and v4 may never be patched or rerun.
