# Goal G Amendment 5: Constructor Parser Repair

Status: authorized for execution

Authorization date: 2026-08-02

Scope: pre-activation recorder construction and provenance rebinding only

## Purpose

This amendment authorizes one new Goal G Amendment 3 activation lineage after
the declared preview recorded by commit
`ed7d34ea504cae9d7dbb4524f6f6ebf494f5648d` stopped deterministically in the
constructor. The retained fixture contains exactly one valid
`combined_replay` report, but the frozen constructor used an Awk expression
with one excess backslash and therefore counted zero reports.

For only the parser repair, the new activation ancestry, and the fresh bundle
bytes required to bind that ancestry, this later authorization supersedes the
following conflicting clauses: Amendment 3's original activation-parent,
direct-child, and inactive-to-active status gates; Amendment 4's
`S4 -> G3 -> P0` direct-child and Stage G3 starting gates; the resume prompt's
Stage G3 exact-S4 parent and status-transition gates; and the terminal
handoff's ban on any successor `G3`. Those clauses are replaced only by the
`S4 -> T -> A5 -> G3 -> P0` lineage and the two exact status transitions
defined below. Every other requirement of Amendments 3 and 4, the runner
contract, the resume prompt, and the terminal handoff remains controlling.

This amendment does not reinterpret any Goal G or Goal G-R result. It does not
authorize a retry of the retained failed preview. It authorizes a separately
hashed successor draft, a different one-shot preview root, and—only after all
new gates pass—fresh official bundle construction.

## Immutable starting point

The authorization parent (`T`) is the pushed terminal-stop commit:

```text
commit=ed7d34ea504cae9d7dbb4524f6f6ebf494f5648d
tree=bb98356880d8f088aa179e9fb8a84c1af068c7ef
parent=706c4bd763647054264cdf3cb52d2355e0aa1b75
subject=docs: record goal g amendment 3 activation stop
handoff_sha256=ed031f2465e5d4684ac4101b9be2e5fd54c68e1ea3632bf2657062abbc4a9032
```

`T` is immutable. Its parent remains historical `S4`; `S4` remains the direct
child of `R6`; and all Goal G-R and historical Goal G evidence identities stay
unchanged.

The failed preview remains retained at
`target/tmp/goal-g-amendment-3-preview-v1` with these frozen identities:

```text
state=partial-unsealed-retained-non-authoritative
entry_count_including_root=21
directory_count=13
regular_file_count=8
regular_bytes=615138
forensic_inventory_sha256=82ac222e4932320ad14ce7ef7800bd8e39a373deaf6ce8205a9ab9ccbfd11747
regular_manifest_sha256=a86c192658af2e4edef79c70ae4f89e842ac9f57ba278f1b8c0ff835defe2df9
combined_report_sha256=bbea695789a6c13ef3095f55622c0c9cf9108a1965f5010485f01628369a3d67
terminal_stderr_sha256=e9f3c933894eae42d5ea7ef3364291e9e1ccea2ed2f2317f836500002e496ded
```

It must not be edited, chmodded, removed, renamed, copied into an official
path, retried, or relabelled. The frozen v2 draft at
`/var/tmp/reap-g3-draft-v2` must not be edited, chmodded, removed, renamed,
linked, mounted over, invoked, promoted, relabelled, or gain or lose an entry.
Authenticated bytes may only be read and copied into the distinct control and
v3 draft roots authorized below.

The v2 root is device `66305`, inode `305347`, mode `0700`, UID/GID
`1000/1000`, link count `2`, and contains exactly ten direct regular-file
children and no other descendant. Nine files are mode `0664`; only
`validators.sh` is mode `0700`; every file is UID/GID `1000/1000` with link
count `1`. Total regular bytes are `1038407`. Its forensic inventory is
SHA-256 `062c306df0e3a5b331be79df841dc98eefeed1a9d1a5b899968bae662d59f0cb`
over records ordered by raw relative-path bytes in the same
`rel\0type\0mode4\0uid\0gid\0nlink\0size\0payload\n` format defined by the
terminal handoff, including root `.` and actual directory `lstat.st_size`.

## Defect classification and functional repair

The frozen v2 constructor is 362479 bytes with SHA-256
`2fe07168369ca726f17328b3d9142522ab2540d057b5d95dd9586a6ded952ee6`.
Its bad matcher is unique. In the v2 byte stream, the repair is exactly:

```text
v3_parser_only = v2[0:51826] + v2[51827:]
deleted_byte=0x5c
parser_only_bytes=362478
parser_only_sha256=c6722bb7936564b427baa7822ba4a491166416f4dccfa5b5aa44d6f0a1051b45
corrected_line_1210_sha256=107cbbb11918f7bf6144f32a718ca10b6eabb328100721dc42dfbef0248393e1
```

The corrected line hash includes its terminating LF. This deletion is the
only authorized behavioral change.

Before any new preview root is created, a read-only regression against the
retained combined fixture must prove:

1. the frozen matcher exits `1` and emits zero bytes;
2. the corrected matcher exits `0`;
3. it selects exactly the retained line 19;
4. the selected line plus LF is 3790 bytes with SHA-256
   `9e89454c35c52a823506f4f77d070d410ca5f504007754d7d0258944fa7a9f5d`;
5. no other constructor matcher has the same excess escape; and
6. the already-correct matcher at the former line 1240 remains unchanged.

The regression is diagnostic and read-only. It must not invoke the
constructor, any installed bundle helper, Cargo, a public fetch, or an
official path. It is the first post-`A5` diagnostic and must pass before the
control root, provenance patch, or v3 draft root is created.

## Necessary provenance rebinding

A literal one-byte copy cannot run after this authorization: v2 hard-binds
preactivation to `S4` and later requires `G3` to be the direct child of `S4`.
The new lineage necessarily contains `T` and this authorization commit
(`A5`). Therefore Amendment 5 separately authorizes strictly provenance-only
changes in the fresh v3 draft.

Exactly these five v3 inputs may differ from v2:

- `construct-self-test.preview.sh`;
- `run-attempt.sh`;
- `validators.sh`;
- `SELF-TEST-DESIGN.md`; and
- `SELF-TEST-SCHEMA.md`.

The constructor changes comprise the one-byte functional repair plus
provenance logic and the hashes of changed inputs. The other four files may
change only to describe, emit, or validate the new provenance. These five
files must add explicit `A5` and `T` identities; they must not overwrite or
reinterpret the historical `S4` and `R6` identities.

The v3 preactivation gate must require clean `master` at exact `A5` and prove
the complete direct-child chain:

```text
R6 -> S4 -> T -> A5
```

The post-activation runner must require exact `G3` as the direct child of
`A5`, while continuing to prove the historical chain. Runtime facts,
fixtures, validators, schemas, design text, and the activation handoff must
bind both the current `A5` activation parent and the historical `S4` boundary
without using one field name for two different commits.

The provenance proof uses two retained audit artifacts outside the ten v3
inputs:

```text
control_root=/var/tmp/reap-g3-draft-v3-provenance-control
patch=/var/tmp/reap-g3-draft-v3-provenance.patch
```

The control root is constructed from authenticated v2 bytes and applies only
the authorized provenance changes while deliberately retaining the bad
113-byte matcher line. It contains exactly the same ten basenames as v2. The
patch is a standard Git full-index binary patch with exactly five file
sections, paths exactly `a/<basename>` and `b/<basename>`, and no rename,
mode, binary-payload, or parser-line change. It is audit material, not an
eleventh v3 or bundle input, and neither the control nor patch may be invoked.

After construction, freeze the patch byte count/SHA-256 and the control's ten
component hashes, component-manifest hash, entry metadata, and forensic
inventory. The two review scratch roots are exactly:

```text
/var/tmp/reap-g3-draft-v3-review-1-scratch
/var/tmp/reap-g3-draft-v3-review-2-scratch
```

Each must be absent, canonical, and non-linked before its distinct review.
That reviewer may create its root once to apply the patch forward to exact v2
and produce the exact control hashes, apply it in reverse to the control and
reproduce all ten v2 hashes plus the v2 forensic inventory, and perform the
one-byte proof. Each of the five unchanged inputs must already be
byte-identical in v2 and control.

If every proof in one review passes, this amendment authorizes removal of
exactly that reviewer's scratch root after its final hash is captured in the
review report. If any proof or removal gate fails, preserve that scratch root
byte-for-byte and stop the lineage. This is the only cleanup authority granted
by Amendment 5. Both scratch roots must be absent again before preview.

The runnable v3 is derived from the authenticated control and differs from it
only by deleting one `0x5c` from the unique bad matcher. Reinserting that byte
in fresh scratch must reproduce the exact control constructor; reversing the
provenance patch must then reproduce exact v2. The final control-to-v3 binary
diff must contain one deletion in one file and no other byte. Exactly two
independent reviewers must inspect every patch hunk as provenance-only and
independently reproduce the forward, reverse, and one-byte proofs.

The closed provenance fields are:

```text
t_commit t_tree t_parent t_subject t_handoff_sha256
a5_commit a5_tree a5_parent a5_subject a5_contract_sha256 a5_handoff_sha256
```

They are added with exactly those names to `repository.json` under `.facts`
and to `phase0.meta`; every existing `s4_*` field keeps its historical name
and value. `repository.json.facts.candidate_parent` changes only from `S4` to
`A5`. The activation handoff adds the same fields with exact prefixes
`goal_g_amendment_5_t_` and `goal_g_amendment_5_a5_`. Commit/tree/parent
values are lowercase 40-hex, document hashes are lowercase 64-hex, and
subjects are the exact subjects frozen by this amendment. No other
provenance field name or location is authorized.

These five v2 hashes are the comparison anchors:

```text
construct-self-test.preview.sh=2fe07168369ca726f17328b3d9142522ab2540d057b5d95dd9586a6ded952ee6
run-attempt.sh=fc5253b789f7ada0e7ba4e016d4ce59551ac03235376c4a9d5e2b3246df93411
validators.sh=4d254d326676ef685d36cb666f8475e3e15d0cb24c4c7ac24c55525e54e0c121
SELF-TEST-DESIGN.md=83ed16b84d8d2f9ef2865eecca2d8fc431636da776c32a800e575ebf2fb20c7d
SELF-TEST-SCHEMA.md=5e5d90b7b568e53e5f3366717108071ff7b3473bdab567beb16ebdf02845d5f0
```

Every other v3 input must be byte-identical to v2:

```text
commands.tsv=89d0e03b192d03ba34d8680616f0c5484010cb06ec3cc59813b66a8c4b0abb7f
inventory.preview.sh=d102c9ddc68cf0eb7fad72308bd86fa986dca52e2dbc0c8346e98a11fe9cf84c
source-reattest.preview.sh=ff1a11823e39b73682c0b77a614f356c17a17907b29855e7d2c7dbeca9bfbd76
summarize-baseline.preview.sh=8c4a006f1eea1c077322bb2baaec195fc2cc8bac52d4ca7fe3d03b6772799f2d
run-phase0-replay.preview.sh=f4b7a52322a0568b19b1e515cb3ec998e827ccbd0ac25abcce0ddd11eddbb2a7
```

No command row, validator rule unrelated to provenance, non-provenance
fixture or case expectation, fixture workload, cardinality, expected result,
trading behavior, production source, dependency, or historical evidence byte
may change. The synthetic repository fixture and its validator may change
only `candidate_parent` and the exact closed `T`/`A5` provenance fields above.

## Revised lineage

The only authorized successor chain is:

```text
R6 -> S4 -> T -> A5 -> G3 -> P0
```

- `T` is the immutable terminal stop above.
- `A5` is the Amendment 5 authorization commit.
- `G3` remains the Amendment 3 activation commit and uses exact subject
  `docs: activate goal g amendment 3`.
- `P0`, if reached, remains the Amendment 3 Phase 0 commit and uses exact
  subject `docs: qualify goal g amendment 3 phase 0`.

`A5` must be the direct child of `T`. `G3` must be the direct child of `A5`
and may modify only the Goal G handoff. `P0` must be the direct child of `G3`.
No commit may be amended, rebased, reset, replaced, or skipped.

## A5 authorization commit

`A5` must use exact subject:

```text
docs: authorize goal g amendment 5 constructor repair
```

It may modify only:

- `docs/polymarket-authenticated-execution-goal-g-amendment-5.md`; and
- `docs/polymarket-authenticated-execution-goal-g-handoff.md`.

It contains documentation only. After committing it, re-authenticate its
commit, tree, parent, subject, exact two-path delta, Amendment 5 hash, `A5`
handoff hash, clean worktree, retained preview hashes, v2 hashes,
official-root absence, and storage gate before creating v3. Record those
results only in the execution transcript and the two read-only review reports
outside the tracked tree, v3/control/patch paths, preview roots, and official
roots. No post-`A5` tracked edit, staging operation, or intervening commit is
allowed before `G3` or a terminal Amendment 5 stop commit.

This authorization does not authorize a push. A later explicit user request
is required to push `A5`, `G3`, `P0`, or any stop commit.

## v3 construction, review, and preview

The control root, provenance patch, fresh draft root
`/var/tmp/reap-g3-draft-v3`, and new preview root must all be absent before
the parser regression and construction. v3 must be built from authenticated
v2 input bytes; it must not source or execute v2 at runtime, and it must not
copy any byte from the failed preview.

The new declared one-shot preview root is:

```text
target/tmp/goal-g-amendment-3-preview-v2
```

Before the preview:

1. run the read-only parser regression;
2. verify the exact five-file changed set and five-file unchanged set;
3. run all retained no-Cargo syntax, embedded-Python, count, path, process,
   storage, failure-preservation, and sealing-adjacency checks;
4. obtain two fresh independent static reviews over every v3 input byte,
   this amendment, the runner contract, the functional delta, and normalized
   provenance equivalence;
5. rehash every v3 input after both reviews; and
6. prove the failed preview is unchanged, all official roots and the new
   preview root are absent, `A5` is exact clean `HEAD`, and no forbidden
   process exists.

The exact five preview argv are:

```text
/bin/busybox
sh
/var/tmp/reap-g3-draft-v3/construct-self-test.preview.sh
preview
/home/ubuntu/code/reap/target/tmp/goal-g-amendment-3-preview-v2
```

Their trailing-NUL argv stream is 145 bytes with SHA-256
`545ea1c137866eb41949219d931a8a4f8ef785992b68514045e0b1f407d0d4f2`.
Run that invocation exactly once. A green result is diagnostic only. Exactly
two distinct post-preview reviewers with distinct sessions must bind the
same preview component hashes, manifest hash, inventory hash, modes, counts,
and self-test result. Never copy, rename, hard-link, or promote the preview
into the official bundle.

## Official construction and activation

After a green preview and its reviews, construct the official bundle freshly
at the existing absent Amendment 3 bundle root. Run the complete retained
no-Cargo self-test, obtain two new independent reviews of every official
bundle byte, bind both reviews, and seal exactly as required by the Amendment
3 runner contract as modified only by this amendment's ancestry.

Keep the official evidence and runtime roots absent through `G3`. The `G3`
handoff must record `T`, `A5`, historical `S4`/`R6`, v3 inputs, sealed bundle,
both official reviews, root absence, and every safety false field. Only then
may the unchanged Amendment 3 Phase 0 campaign sequence begin.

`G3` must change exactly one existing
`goal_g_amendment_3_status=activation-stopped-inactive` field to
`active-phase0` and exactly one
`goal_g_amendment_5_status=authorized-inactive` field to
`activation-complete-phase0-active`. It must retain the complete historical
Amendment 3 activation-stop block without changing or deleting any field.

## Failure semantics

Any v3 construction, regression, static-review, preview, official
construction, self-test, review, sealing, or activation failure stops this
lineage. Preserve every created byte and keep all paths in their honest
states. Do not retry, repair in place, reuse another root, or continue to
`G3`.

If failure occurs before official bundle creation, record official
`bundle_state=absent-not-created` separately from any retained partial
diagnostic draft or preview. If official construction began, use the existing
`partial-unsealed` or `sealed` state rules.

When storage permits, the stop commit must be the direct child of `A5`,
modify only the Goal G handoff, and use exact subject:

```text
docs: record goal g amendment 5 activation stop
```

A later attempt requires another reviewed, user-authorized amendment.

## Storage, safety, and non-claims

The exact Amendment 4 2-GiB storage preflight remains mandatory immediately
before every child, write, redirect, executable-bit change, tracked edit,
staging operation, and commit. Amendment 5 grants no cleanup authority except
the two successful-review scratch removals explicitly defined above.

From `A5` through `G3`, the entire Amendment 5 workflow prohibits Cargo,
rustc, rustdoc, rustfmt, test or benchmark binaries, public fetches, network
children, credentials, authenticated requests, Polygon RPC, and production
order entry. The unchanged Phase 0 Cargo campaigns remain unavailable until
after a valid `G3` exists.

```text
production_order_entry_authorized=false
real_credentials_loaded=false
authenticated_external_request_sent=false
real_polygon_rpc_request_sent=false
real_order_submitted=false
historical_goal_g_attempt_relabelled=false
historical_goal_g_r_equivalence_claimed=false
failed_preview_retry_authorized=false
failed_v2_mutation_authorized=false
push_authorized=false
```
