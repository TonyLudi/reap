# Goal G Amendment 6: Forensic Inventory Recovery

Status: authorized for execution

Authorization date: 2026-08-02

Scope: retained-draft authentication, provenance-only rebinding, and one fresh
Goal G Amendment 3 activation lineage

## Purpose

Goal G Amendment 5 stopped during the first independent v3 review. The v3
parser regression had passed and no constructor or preview had run. The stop
was caused by the executor supplying forensic inventory expectations computed
with the wrong payload convention: raw file bytes and an empty directory
payload. The frozen convention requires a file's lowercase SHA-256 and the
literal byte `-` for a directory. The reviewer correctly reproduced the v2
inventory and rejected the wrong expectation before creating review scratch.

This amendment preserves that failure exactly. It does not retry, relabel, or
complete either Amendment 5 review. It authorizes two independent read-only
reauthentications under the exact inventory encoding below and, only if both
pass, a separately hashed v4 derived from immutable v3. Every v3-to-v4 change
must be provenance-only. The already-correct parser remains byte-identical.

For this recovery only, this amendment supersedes the conflicting activation
parent, direct-child, status-transition, draft, review-root, and preview-root
clauses in Amendments 3 through 5 and their resume contracts. They are
replaced only by this amendment's exact lineage, status rules, v4 paths, and
gates. Every other safety, evidence, workload, sealing, and Phase 0
requirement remains controlling.

## Unambiguous commit aliases

The existing Goal G-R Amendment 6 authorization is already called `A6` by
the recorder inputs. This amendment must not reuse that alias. The new Goal G
Amendment 6 authorization commit is called `G6_AUTH`. The retained Amendment
5 stop is called `G5_STOP`.

The immutable starting boundary is:

```text
G5_STOP_commit=dab6a252ffe25bb390da12a0459125cbeeacb7de
G5_STOP_tree=1f50b0d1ed8857de134b092848cb36e8e6bc8ff8
G5_STOP_parent=ba3b666d95d8097f60f8fc33a12b9844115edca8
G5_STOP_subject=docs: record goal g amendment 5 activation stop
G5_STOP_delta_path_count=1
G5_STOP_delta_paths=docs/polymarket-authenticated-execution-goal-g-handoff.md
G5_STOP_handoff_sha256=4f8d9cd5663e2e051ce0e34a73f06a154dce88c65c1f41894d54af1aaa3c41b4
```

`G5_STOP` is the direct child of immutable `A5`; `A5` is the direct child of
`T`; `T` is the direct child of `S4`; and `S4` is the direct child of `R6`.
The complete successor chain authorized here is:

```text
R6 -> S4 -> T -> A5 -> G5_STOP -> G6_AUTH -> G3 -> P0
```

The earlier Goal G-R `A6` remains the parent of `R6` and keeps its historical
name and identity. No commit may be amended, rebased, reset, replaced,
skipped, or assigned a second alias.

## Preserved Amendment 5 result

The complete Amendment 5 terminal block in
`docs/polymarket-authenticated-execution-goal-g-handoff.md` is immutable
historical evidence. In particular:

- `goal_g_amendment_3_status=activation-stopped-inactive` remains the one
  current Amendment 3 status before `G3`;
- `goal_g_amendment_5_status=activation-stopped-inactive` remains the one
  current Amendment 5 status permanently, including after a successful `G3`;
- review 1 remains `fail-input-authentication` before scratch creation;
- review 2 remains cancelled before scratch after review 1 stopped;
- both Amendment 5 review scratch roots remain absent-never-created;
- preview-v2 remains absent and has invocation count zero; and
- the official bundle, evidence, and runtime roots remain absent.

The failure class remains `review-input-identity-capture-error`. This
amendment makes no claim that the failed review passed, that v3 was reviewed,
that preview-v2 ran, or that any historical Goal G attempt succeeded.

The failed preview-v1, frozen v2, Amendment 5 provenance control, Amendment 5
patch, and v3 must remain byte-for-byte retained at their recorded paths.
They must not be edited, chmodded, removed, renamed, linked, mounted over,
invoked, promoted, or relabelled. Authenticated v3 file bytes may only be read
and copied into the distinct v4 root authorized below.

## Frozen forensic inventory encoding

An inventory covers root `.` and every descendant. Records are ordered by raw
relative-path bytes. The root relative path is the single byte `.`. Each
record is exactly:

```text
rel NUL type NUL mode4 NUL uid NUL gid NUL nlink NUL size NUL payload LF
```

Equivalently, its bytes are described by:

```text
rel\0type\0mode4\0uid\0gid\0nlink\0size\0payload\n
```

The fields have these closed meanings:

- metadata comes from `lstat` on the exact entry being recorded;
- `type` is the one ASCII byte `d` for a directory or `f` for a regular file;
- links and every other file type are rejected;
- `mode4` is `(lstat.st_mode & 07777)` rendered as exactly four ASCII octal
  digits matching `[0-7]{4}`, including its leading zero;
- `uid`, `gid`, `nlink`, and `size` are canonical unsigned ASCII decimal
  matching `0|[1-9][0-9]*`;
- directory `size` is the actual `lstat.st_size`, not zero and not a derived
  descendant size;
- a directory payload is the single ASCII byte `-`;
- a file payload is the 64 lowercase ASCII hex digits of SHA-256 over the
  exact file bytes, never those file bytes themselves; and
- every record, including the last, ends with one LF.

Each descendant relative path is its raw basename bytes joined by ASCII `/`.
It has no leading `./` and no trailing `/`. Ordering is lexicographic by
unsigned octet, with the shorter string first when one is a prefix; root `.`
participates in that same ordering. No locale-sensitive sort,
newline-delimited pathname transport, shell command substitution containing
record bytes, implicit text conversion, or empty directory payload is
conforming.

Two fixed single-record vectors disambiguate the encoding. The v3 root record
is 28 bytes with SHA-256
`5c5f2aa15f151a1c1fd8285ee13c42e968e17889c99ad85c06e544080824ba81`.
The v3 `commands.tsv` record is 102 bytes with SHA-256
`3ca42fa79530d356d42a05c2324d7ea09132e0d8ae5882e9285e7cff5abd3bea`.

An implementation-independent two-record vector further freezes ordering and
concatenation. Its ordered tuples are:

```text
. d 0700 1000 1000 2 4096 -
a f 0644 1000 1000 1 3 ba7816bf8f01cfea414140de5dae2223b00361a396177a9cb410ff61f20015ad
```

The second payload is SHA-256 of the three bytes `abc`. The concatenated
encoded stream is 116 bytes with SHA-256
`63ed0e2d6f3f43abc06cce1dd215d166131f25132b645ec6c027b50d1629c9c0`.

A component-manifest row is exactly:

```text
lowercase-file-sha256 TAB canonical-decimal-byte-count TAB basename LF
```

Rows cover the ten direct regular children and are ordered by raw basename
bytes. Both v2 and v3 component manifests have exactly 10 rows and 933 bytes.

Before creating v4, its patch, either review scratch, the new preview, or any
official path, two distinct reviewers in distinct sessions must use
independent read-only implementations. Each implementation must reproduce
all of these identities from current bytes:

```text
v2_entry_count_including_root=11
v2_directory_count_including_root=1
v2_regular_file_count=10
v2_regular_bytes=1038407
v2_forensic_stream_bytes=1151
v2_component_manifest_rows=10
v2_component_manifest_bytes=933
v2_component_manifest_sha256=82fa2de7bc468a5a60fa3f795f336d621515557a5ee21b9828b09d1d526cf4a8
v2_forensic_inventory_sha256=062c306df0e3a5b331be79df841dc98eefeed1a9d1a5b899968bae662d59f0cb
v3_entry_count_including_root=11
v3_directory_count_including_root=1
v3_regular_file_count=10
v3_regular_bytes=1055725
v3_forensic_stream_bytes=1151
v3_component_manifest_rows=10
v3_component_manifest_bytes=933
v3_component_manifest_sha256=710ab62d5dbe846b21df74a4d78ee3f12d2a1883a22662d256bf751d411bc451
v3_forensic_inventory_sha256=9ab5b0695e60cf47fc41e9d2e110b291c56b856d80cbfad36b9d41b6a5c7d233
```

Each implementation must also reproduce both single-record vectors, bind its
own implementation identity or source hash, reproduce the two-record vector,
and take matching metadata cuts before and after hashing. These two reviews
run only after exact clean `G6_AUTH` authentication and before creation of any
v4, patch, scratch, review-report, preview, or official path. A mismatch is
terminal before construction.

The historical wrong-schema expectations
`4f2039164a8403a0ff9692f358fb513fb2b2e209ee3e179a0bff04d24814cd6e`
for the control and
`cf5eb07c85af4721c586c90a778a2fb902c32d7bcd274863206ec13a193e63c`
for v3 are rejected evidence, never acceptable gate values. The correctly
recomputed control inventory is
`2f05254afe092859bcae96711f993cfd88165820896b0287441f2251206b9d51`.

## Immutable v3 source

The sole v4 source root is `/var/tmp/reap-g3-draft-v3`. Its root is device
`66305`, inode `310585`, mode `0700`, UID/GID `1000/1000`, link count `2`,
actual size `4096`, and has exactly ten direct regular-file children. Nine are
mode `0664`; only `validators.sh` is mode `0700`. Every file is UID/GID
`1000/1000` with link count `1`.

The exact source hashes and sizes are:

```text
4f739c6f49d90418ba1e1576bf2f4015f1da9a4b9b8eed9ffa3de9414d21c5a4 44806 SELF-TEST-DESIGN.md
a4d8e7ae085bd2517678e0762690c813d2e69232d463e3df83ec9956faf27ecd 24089 SELF-TEST-SCHEMA.md
89d0e03b192d03ba34d8680616f0c5484010cb06ec3cc59813b66a8c4b0abb7f 5509 commands.tsv
7f16928835d296353d6cc94501bd3cabd6f7febc7da044606673d7ee287c9bba 366812 construct-self-test.preview.sh
d102c9ddc68cf0eb7fad72308bd86fa986dca52e2dbc0c8346e98a11fe9cf84c 53408 inventory.preview.sh
86a79706b6aa8253b7d8fb298c5016535aab33a2cd91f4c842b3c2d06c72ddcd 217156 run-attempt.sh
f4b7a52322a0568b19b1e515cb3ec998e827ccbd0ac25abcce0ddd11eddbb2a7 100443 run-phase0-replay.preview.sh
ff1a11823e39b73682c0b77a614f356c17a17907b29855e7d2c7dbeca9bfbd76 22544 source-reattest.preview.sh
8c4a006f1eea1c077322bb2baaec195fc2cc8bac52d4ca7fe3d03b6772799f2d 82593 summarize-baseline.preview.sh
897f3bb05418397d8d17944dea70501a1bb2adbbf65c73acc06035726eab678b 138365 validators.sh
```

Re-authenticate all metadata, file hashes, the component manifest, and the
forensic inventory before and after each read-only authentication and again
immediately before copying any source byte.

## G6_AUTH commit contract

`G6_AUTH` must be the direct child of exact `G5_STOP` and use exact subject:

```text
docs: authorize goal g amendment 6 forensic inventory recovery
```

It may change only:

- `docs/polymarket-authenticated-execution-goal-g-amendment-6.md`; and
- `docs/polymarket-authenticated-execution-goal-g-handoff.md`.

The handoff adds exactly one
`goal_g_amendment_6_status=authorized-inactive` field and must not change the
existing Amendment 3 or Amendment 5 status fields or either terminal block.
The commit contains documentation only.

After committing, authenticate exact `G6_AUTH` commit, tree, parent, subject,
two-path delta, Amendment 6 contract hash, `G6_AUTH` handoff hash, clean
`master`, complete ancestry, every retained artifact identity, and every
required absent root. Record those identities outside the tracked tree until
`G3` or an honest Amendment 6 stop. No intervening tracked edit, staging
operation, or commit is allowed.

This amendment does not authorize a push. A later explicit user request is
required to push `G6_AUTH`, `G3`, `P0`, or a stop commit.

## Provenance-only v4

The new paths are exactly:

```text
v4_root=/var/tmp/reap-g3-draft-v4
v4_patch=/var/tmp/reap-g3-draft-v4-provenance.patch
review_1_scratch=/var/tmp/reap-g3-draft-v4-review-1-scratch
review_2_scratch=/var/tmp/reap-g3-draft-v4-review-2-scratch
preview_root=target/tmp/goal-g-amendment-3-preview-v3
```

All five must be absent, canonical at their nearest existing ancestor, and
non-linked before the first reauthentication. The official bundle, evidence,
and runtime roots must also remain absent.

v4 is built only by copying authenticated v3 file bytes into the distinct v4
root and applying the closed changes below. It must not copy a byte from v2,
the Amendment 5 control, the Amendment 5 patch, preview-v1, preview-v2, an
official path, or any review scratch.

Exact v3 is the sole control/base for v4. No separate v4 control path exists
or is authorized. v4 must retain v3 filesystem structure and metadata: root
mode `0700`, UID/GID `1000/1000`, link count `2`, exactly ten direct regular
children and no other descendant; nine children mode `0664` and only
`validators.sh` mode `0700`; every child UID/GID `1000/1000` and link count
`1`. Final v4 file sizes, component rows, component-manifest hash, and
forensic inventory are frozen immediately after construction.

Exactly these five v4 inputs may differ from v3:

- `construct-self-test.preview.sh`;
- `run-attempt.sh`;
- `validators.sh`;
- `SELF-TEST-DESIGN.md`; and
- `SELF-TEST-SCHEMA.md`.

Exactly these five inputs must remain byte-identical to v3:

- `commands.tsv`;
- `inventory.preview.sh`;
- `run-phase0-replay.preview.sh`;
- `source-reattest.preview.sh`; and
- `summarize-baseline.preview.sh`.

The executable edit surface is closed. In
`construct-self-test.preview.sh`, only top-level lineage and changed-input-hash
constants, `verify_preactivation_repository`, repository fixture 22a inside
`construct_attestation_fixtures`, the `EXPECTED_MANIFEST` literal inside
`verify_helper_redirection_manifest`, and lineage/status/evidence emission
inside `seal_official_bundle` may change. In `run-attempt.sh`, only top-level
lineage and changed-input-hash constants, `verify_repository`, the
lineage/status/evidence portion of `verify_bundle`, `phase0_meta_keys`,
`validate_phase0_meta_values`, and Phase 0 provenance writes inside
`seal_phase0_pass` may change. In `validators.sh`, only top-level lineage
constants, `validate_repository_attestation`, and
`emit_repository_attestation` may change. Design and schema may change only
their provenance sections. Every other function body and every other
non-allowlisted byte in those five files must be byte-identical to v3.

Within that exact surface, every changed hunk must do only one of the
following:

1. bind exact `G5_STOP` and `G6_AUTH` identities and their direct-child
   ancestry while retaining all historical `S4`, `T`, `A5`, `R6`, and Goal
   G-R `A6` identities;
2. change preactivation from exact clean `A5` to exact clean `G6_AUTH`;
3. change the required `G3` parent from `A5` to `G6_AUTH`;
4. change `repository.json.facts.candidate_parent` from `A5` to `G6_AUTH`;
5. add the closed provenance fields defined below to repository facts and
   `phase0.meta`;
6. keep Amendment 5 stopped and define the exact Amendment 3/6 activation
   transitions and handoff fields;
7. describe or validate those provenance changes; or
8. update the hashes of changed v4 inputs after all other bytes are final.

The corrected constructor matcher must be byte-identical to v3. Its line plus
LF has SHA-256
`107cbbb11918f7bf6144f32a718ca10b6eabb328100721dc42dfbef0248393e1`.
It must occur exactly once. The complete v3 `construct_combined_fixtures`
function body is 3025 bytes with SHA-256
`7c1f62087f71572805426f0209c536e8c10310596292ac32e709974f05c8fa70`;
v4 must reproduce that body byte-for-byte.

The constructor's shell-aware validator-redirection source manifest retains
exactly 179 seven-column rows. Normalize its source-line column 2 to literal
`<SOURCE_LINE>` and its preflight-line column 6 to literal
`<PREFLIGHT_LINE>`, retaining every TAB, every other column, and every LF. The
normalized v3 stream is 17554 bytes with SHA-256
`b2734fc048d6e536cd2c4fdabe6975f5da77cee1b061a28e4eac97d4e51ef924`;
v4 must reproduce it exactly. For reference, the unnormalized v3 stream is
13642 bytes with SHA-256
`e20c121714dca1d0c1811adabaa112a7019bfeefcd77e647288446c6bd7042b7`.
Only mechanically shifted decimal line numbers in columns 2 and 6 may differ.
The review must also prove that the 116 fixture cases, 1240 subcases, all
established file/path counts, schema identifiers, workload bytes, and
safety-false fields remain unchanged.

There may be no parser, command row, workload, fixture case, cardinality,
expected result, process boundary, storage rule, sealing rule, production
source, dependency, credential, network, authentication, Polygon RPC, or
trading behavior change.

The v4 patch is a standard Git full-index text patch from exact v3 to exact
v4. It has exactly five file sections with paths exactly `a/<basename>` and
`b/<basename>`, no rename/copy/mode/binary-payload marker, and no extra file.
It is audit material, not an eleventh input, and must never be invoked or used
as an official bundle input.

The exact repository-fact and `phase0.meta` additions are:

```text
g5_stop_commit g5_stop_tree g5_stop_parent g5_stop_subject g5_stop_handoff_sha256
g6_auth_commit g6_auth_tree g6_auth_parent g6_auth_subject g6_auth_contract_sha256 g6_auth_handoff_sha256
```

These names deliberately avoid the existing Goal G-R `a6` identity. Every
existing `s4_*`, `t_*`, and `a5_*` field keeps its exact name and value.
`candidate_parent` becomes exact `G6_AUTH`.

The activation handoff retains all existing historical S4, T, and A5 identity
fields and adds exact prefixes:

```text
goal_g_amendment_6_g5_stop_commit
goal_g_amendment_6_g5_stop_tree
goal_g_amendment_6_g5_stop_parent
goal_g_amendment_6_g5_stop_subject
goal_g_amendment_6_g5_stop_handoff_sha256
goal_g_amendment_6_g6_auth_commit
goal_g_amendment_6_g6_auth_tree
goal_g_amendment_6_g6_auth_parent
goal_g_amendment_6_g6_auth_subject
goal_g_amendment_6_g6_auth_contract_sha256
goal_g_amendment_6_g6_auth_handoff_sha256
```

All commit/tree/parent values are lowercase 40-hex; hashes are lowercase
64-hex; subjects are exact. No synonym or additional provenance location is
authorized.

The activation evidence schema is also closed. For forensic review number
`N` in `1,2`, `G3` contains exactly these fields:

```text
goal_g_amendment_6_forensic_review_N_result
goal_g_amendment_6_forensic_review_N_reviewer
goal_g_amendment_6_forensic_review_N_session
goal_g_amendment_6_forensic_review_N_implementation_sha256
goal_g_amendment_6_forensic_review_N_v2_inventory_sha256
goal_g_amendment_6_forensic_review_N_v3_inventory_sha256
goal_g_amendment_6_forensic_review_N_two_record_vector_sha256
```

For v4 static review number `N` in `1,2`, it contains exactly:

```text
goal_g_amendment_6_v4_review_N_result
goal_g_amendment_6_v4_review_N_reviewer
goal_g_amendment_6_v4_review_N_session
goal_g_amendment_6_v4_review_N_scratch_final_inventory_sha256
goal_g_amendment_6_v4_review_N_scratch_state
goal_g_amendment_6_v4_review_N_v3_inventory_sha256
goal_g_amendment_6_v4_review_N_v4_inventory_sha256
goal_g_amendment_6_v4_review_N_patch_sha256
```

The ten v4 component fields are exactly:

```text
goal_g_amendment_6_v4_self_test_design_sha256
goal_g_amendment_6_v4_self_test_schema_sha256
goal_g_amendment_6_v4_commands_sha256
goal_g_amendment_6_v4_construct_self_test_sha256
goal_g_amendment_6_v4_inventory_sha256
goal_g_amendment_6_v4_run_attempt_sha256
goal_g_amendment_6_v4_run_phase0_replay_sha256
goal_g_amendment_6_v4_source_reattest_sha256
goal_g_amendment_6_v4_summarize_baseline_sha256
goal_g_amendment_6_v4_validators_sha256
```

They are accompanied exactly once by:

```text
goal_g_amendment_6_v4_component_manifest_rows
goal_g_amendment_6_v4_component_manifest_bytes
goal_g_amendment_6_v4_component_manifest_sha256
goal_g_amendment_6_v4_forensic_stream_bytes
goal_g_amendment_6_v4_forensic_inventory_sha256
goal_g_amendment_6_v4_patch_sections
goal_g_amendment_6_v4_patch_bytes
goal_g_amendment_6_v4_patch_sha256
```

For post-preview review number `N` in `1,2`, the fields are exactly:

```text
goal_g_amendment_6_preview_review_N_result
goal_g_amendment_6_preview_review_N_reviewer
goal_g_amendment_6_preview_review_N_session
goal_g_amendment_6_preview_review_N_component_manifest_sha256
goal_g_amendment_6_preview_review_N_bundle_manifest_sha256
goal_g_amendment_6_preview_review_N_bundle_sha256_sha256
goal_g_amendment_6_preview_review_N_inventory_sha256
goal_g_amendment_6_preview_review_N_regular_files
goal_g_amendment_6_preview_review_N_entries_including_root
goal_g_amendment_6_preview_review_N_self_test_result
```

Both results in each pair are `pass`, reviewer and session values are nonempty
and pairwise distinct, repeated hashes/counts are equal, v4/patch values equal
the frozen inputs, and scratch state is `removed-after-pass`. Existing closed
Amendment 3 official-review fields remain controlling. `verify_bundle` and the
activation validator must consume this complete schema and reject missing,
duplicate, synonymous, malformed, or extra Amendment 6 activation fields.

Finalize the design, schema, runner, and validator bytes before embedding
their hashes in the constructor. The constructor is finalized last. No
self-referential hash field is authorized.

## v4 static reviews

Before either review scratch may be created, that reviewer must independently
pass the complete v2/v3 forensic gate above. Reviewers and sessions must be
distinct. Each scratch root may be created at most once.

In its own root, each reviewer must independently:

1. authenticate the immutable v3 source, the v4 target, and the patch
   identities;
2. apply the five-section patch forward to exact v3 copies and reproduce all
   ten exact v4 file hashes, component manifest, metadata, and inventory;
3. reverse the patch from those reproduced v4 bytes and reproduce all ten
   exact v3 file hashes, component manifest, metadata, and inventory;
4. inspect every patch hunk and classify it under the eight-item closed
   provenance list above;
5. prove the corrected matcher line and every functional anchor are unchanged;
6. run all retained source-static no-Cargo Bash syntax, embedded-Python
   compile, count, closed-path, process, storage, failure-preservation,
   sealing-adjacency, and shell-aware redirection-manifest scanners; and
7. rehash v3, v4, and the patch after all checks.

These gates may read inputs but must not source or execute any v4 script. The
complete no-Cargo self-test runs only inside the one authorized preview or
official constructor invocation. No constructor, preview, Cargo command,
test, benchmark, public fetch, network action, credential load, authenticated
request, RPC, or order entry is permitted during static review.

On a complete pass, capture the final scratch inventory, complete every other
gate and hash, remove only that reviewer's v4 scratch root, and prove its
absence. On a failure before removal begins, preserve that scratch
byte-for-byte and stop. If removal starts but does not complete, preserve the
remaining honest state, record `partial-removal-failed`, and stop without
retry. This is the only cleanup authority granted by this amendment.

Both reviews must pass and both successful scratch roots must again be absent
before preview. Rehash every v4 input and retained artifact after both reviews.

## Fresh one-shot preview

The preview-v2 path remains absent and unauthorized. The only new declared
preview root is:

```text
target/tmp/goal-g-amendment-3-preview-v3
```

The exact five preview argv are:

```text
/bin/busybox
sh
/var/tmp/reap-g3-draft-v4/construct-self-test.preview.sh
preview
/home/ubuntu/code/reap/target/tmp/goal-g-amendment-3-preview-v3
```

Their trailing-NUL stream is 145 bytes with SHA-256
`72828d50c317fab81c471ed8020c8580d9a17c1dabba21a2fc11dbe138e941d7`.
Invoke that argv exactly once, only after every prior gate passes. A green
preview is diagnostic and non-authoritative; it must never be copied, renamed,
linked, or promoted into the official bundle.

Exactly two new post-preview reviewers with distinct identities and sessions
must independently bind identical preview component hashes, manifests,
inventory, modes, counts, process evidence, and self-test result. Any preview
or review failure is terminal and preserves every created byte.

## Official construction and G3

Only after the fresh preview and both post-preview reviews are green may the
still-absent official bundle be constructed freshly at:

```text
target/tmp/goal-g-amendment-3-recorder-bundle
```

Run the complete retained no-Cargo self-test, obtain two new independent
official-byte reviews, bind them, and seal exactly under the unchanged
Amendment 3 runner contract as modified only by this amendment's provenance.
The official evidence and runtime roots remain absent through `G3`.

`G3` must be the direct child of exact `G6_AUTH`, modify only
`docs/polymarket-authenticated-execution-goal-g-handoff.md`, and use exact
subject:

```text
docs: activate goal g amendment 3
```

At `G3`:

- change the unique Amendment 3 status from `activation-stopped-inactive` to
  `active-phase0`;
- leave the unique Amendment 5 status exactly
  `activation-stopped-inactive`;
- change the unique Amendment 6 status from `authorized-inactive` to
  `activation-complete-phase0-active`;
- retain both complete historical terminal blocks unchanged; and
- record the complete historical and current lineage, all ten v4 hashes, v4
  manifest/inventory, patch identity, all review identities, sealed official
  identities, absent evidence/runtime roots, and every safety-false field.

`P0`, if reached, must be the direct child of `G3`, modify only the handoff,
and use exact subject:

```text
docs: qualify goal g amendment 3 phase 0
```

Only after valid `G3` may the original Goal G Phase 0 commands become
available. Only after valid `P0` may the remaining original Goal G phases
resume.

## Failure semantics

Any authentication, construction, patch, review, preview, official
construction, self-test, sealing, or activation failure stops this lineage.
Preserve every created byte and keep every path in its honest state. Do not
retry, repair in place, reuse another root, or continue to `G3`.

Whenever storage permits, a stop commit must be the direct child of
`G6_AUTH`, modify only the Goal G handoff, and use exact subject:

```text
docs: record goal g amendment 6 activation stop
```

Relative to the `G6_AUTH` handoff it must replace exactly one
`goal_g_amendment_6_status=authorized-inactive` with
`goal_g_amendment_6_status=activation-stopped-inactive`, append exactly one
Amendment 6 terminal block describing the failed gate and actual path states,
and leave every other pre-existing byte—including both historical terminal
blocks and the Amendment 3/5 statuses—unchanged. If the storage preflight
prevents that edit or commit, make no further mutation.

A later attempt requires another reviewed, user-authorized amendment.

## Storage, safety, and non-claims

The exact Amendment 4 2-GiB storage preflight remains mandatory immediately
before every external child, write, redirect, executable-bit change, tracked
edit, staging operation, and commit. The preflight's own `git`, `df`, and
`awk` children are its only recursion exception.

From `G6_AUTH` through valid `G3`, Cargo, rustc, rustdoc, rustfmt, test and
benchmark binaries, public fetches, network children, credentials,
authenticated requests, Polygon RPC, and production order entry are
prohibited. The unchanged Phase 0 Cargo campaigns remain unavailable until
after valid `G3`.

```text
production_order_entry_authorized=false
real_credentials_loaded=false
authenticated_external_request_sent=false
real_polygon_rpc_request_sent=false
real_order_submitted=false
historical_goal_g_attempt_relabelled=false
historical_goal_g_r_equivalence_claimed=false
amendment_5_review_retry_authorized=false
preview_v2_reuse_authorized=false
push_authorized=false
```
