# Goal G Amendment 3 Recorder And Command Contract

Status: **normative pre-activation construction contract**.

This document closes the executable choices left by
`polymarket-authenticated-execution-goal-g-amendment-3.md`. It defines the
pre-activation recorder bundle, command labels, attempt layout, validators,
source re-attestation successor, and Phase 0 replay helper.

No Cargo command or public fetch may run while this bundle is being created or
reviewed.

## Paths And Lifecycle

The pre-activation bundle path is:

```text
target/tmp/goal-g-amendment-3-recorder-bundle
```

It is separate from:

```text
evidence: target/tmp/goal-g-phase0-amendment-3
runtime:  target/tmp/goal-g-amendment-3-runtime
```

Before bundle creation, all three paths must be absent, canonical descendants
of the repository root, and not symlinks. Create only the bundle before the
Goal G Amendment 3 activation commit.

After the bundle is complete, statically reviewed, self-tested without Cargo,
hashed, and sealed, record its exact identities in the activation handoff.
Only after the activation commit may the executor create the evidence root
and runtime root.

If bundle creation or activation fails, preserve the bundle and commit the
specified Goal G Amendment 3 activation stop. The actual evidence root must
remain absent. Reusing or replacing that bundle requires a new reviewed
amendment.

## Bundle Layout

The sealed bundle contains exactly:

```text
run-attempt.sh
run-attempt.sha256
commands.tsv
validators.sh
validators.sha256
inventory.sh
inventory.sha256
source-reattest.sh
source-reattest.sha256
summarize-baseline.sh
summarize-baseline.sha256
run-phase0-replay.sh
run-phase0-replay.sha256
self-test/
  fixtures/
  self-test.meta
  self-test.stdout
  self-test.stderr
  self-test.sha256
bundle.manifest
bundle.sha256
```

During construction, the bundle directory and every descendant directory are
mode `700`. The final sealing operation, performed only after the self-test
and both static reviews pass, sets every directory to mode `500`,
executable scripts to mode `500`, and every other regular file to mode `400`.
Every regular file must have link count one. Symlinks, sockets, devices,
FIFOs, and any other file type are forbidden.

Each top-level script-specific one-line `*.sha256` covers the corresponding
`.sh` executable. The line format is exactly lowercase hexadecimal SHA-256,
two ASCII spaces, the executable's bundle-relative path, and one LF.
`self-test.sha256` and `bundle.sha256` instead follow their explicit
multi-file/root-digest rules below. `bundle.manifest` is not self-referential:
it contains one row in that same format for every regular bundle file except
`bundle.manifest` and `bundle.sha256`, sorted bytewise by bundle-relative
path with `LC_ALL=C`. `bundle.sha256` contains the one-line hash of
`bundle.manifest` in the same format. Every payload and sibling-hash file is
therefore covered by the manifest, the manifest is covered by
`bundle.sha256`, and `bundle.sha256` is the intentionally unhashed root digest
recorded verbatim in the activation handoff.

After the final modes are applied, compute the whole-bundle file stream as
the SHA-256 of `bundle.manifest` and compute the inventory stream from the
following exact NUL-delimited records, sorted bytewise by relative path:

```text
<type>\t<mode-octal>\t<size-decimal>\t<relative-path>\t<link-target>\0
```

`type` is `d` or `f`; directory size is recorded as `0`; regular-file size is
the exact byte count; `link-target` is empty because links are forbidden.
The inventory includes `.` and every descendant, including
`bundle.manifest` and `bundle.sha256`. Hash the complete record stream with
SHA-256. The activation handoff records every individual script hash, the
manifest and `bundle.sha256` hashes, the complete bundle regular-file count,
entry count including `.`, file-stream hash, inventory-stream hash, all final
modes, and two independent review results.

No bundle byte, mode, path, or link may change after activation.

## Recorder Interface

The only invocation is:

```text
run-attempt.sh <expected-head> <campaign> <ordinal> <label>
```

The recorder:

- requires `expected-head` to equal the exact Goal G Amendment 3 activation
  commit `G3`;
- accepts only one row in `commands.tsv` matching campaign, ordinal, and
  label;
- maps that row to one exact argv vector or one frozen bundle helper;
- accepts no arbitrary command text, environment value, path, or validator;
- uses `CARGO_NET_OFFLINE=true` for every Cargo child;
- resolves the Amendment 3 runtime path to an absolute canonical
  repository descendant and passes it as `TMPDIR`;
- creates exactly one previously absent attempt directory;
- owns and monitors the complete child process tree;
- preserves command exit separately from semantic validation; and
- stops after any invalid or red attempt.

The campaign order is exactly `attestation`, `source`, `confidence`,
`baseline`, then `replay`. Within a campaign, ordinal `01` requires no
existing attempt path; ordinal `NN > 01` requires every lower ordinal,
especially its immediate predecessor, sealed and green. Beginning a campaign
requires every earlier campaign sealed green. Before creating an attempt, the
recorder rejects a gap, any existing current-or-later ordinal, any later
campaign path, an already sealed campaign, or any prior red/invalid row. It
never creates a retry ordinal or a second attempt for a row. A direct
later-row invocation is rejected before child execution and is not authority
to skip, backfill, delete, or replace evidence.

The exact attempt path is:

```text
target/tmp/goal-g-phase0-amendment-3/<G3>/<campaign>/<ordinal>-<label>
```

Every attempt contains:

```text
stdout.log
stderr.log
process.ps.tsv
attempt.meta
attempt.sha256
```

Label-specific reports, projections, comparisons, and validator logs are
additional files covered by `attempt.sha256`.

An attempt directory is created at mode `700`; its regular outputs are
created with mode `600` and `umask 077`. No link or non-regular output is
allowed. After the sampler has been waited for, the validator has completed,
and metadata is final, set every regular output except `attempt.sha256` to
mode `400`. Then write `attempt.sha256` as one lowercase-hash/two-space/
attempt-relative-path/LF row for every other regular file, sorted bytewise by
path with `LC_ALL=C`; set it to mode `400`; verify every row; and set the
attempt directory to mode `500`. Neither a green, valid-red, nor invalid
sealed attempt may subsequently change.

`attempt.meta` uses a versioned `key=value` schema and records:

- schema, candidate, campaign, ordinal, label, exact argv/environment;
- start/end UTC;
- pre/post `HEAD`, tree, branch, status blocks, and `Cargo.lock` hash;
- repository, evidence, bundle, and runtime canonical paths;
- available bytes;
- rustc, Cargo, host, kernel, and CPU count;
- child PID/process-group identity and process-overlap result;
- command exit, validation result, evidence validity, and gate result;
- runtime-empty pre/post results;
- all relevant report/projection hashes; and
- these exact five booleans:

```text
production_order_entry_authorized=false
real_credentials_loaded=false
authenticated_external_request_sent=false
real_polygon_rpc_request_sent=false
real_order_submitted=false
```

Every `campaign.meta` carries the same five booleans.

## Universal Attempt Rules

Immediately before bundle creation, evidence creation, every redirected log,
every child, and every tracked edit/commit, run the Amendment 3 exact 2-GiB
preflight.

Before and after every child require:

- exact clean candidate identity;
- unchanged `Cargo.lock`;
- unchanged historical `target/tmp/goal-g-phase0`, amended
  `target/tmp/goal-g-phase0-amended`, and Goal G-R evidence;
- unchanged sealed bundle;
- empty runtime root;
- no matching process outside the recorder's process tree; and
- no symlink/path escape.

Before every campaign reject these inherited variables, recording names but
never values:

```text
REAP_PHASE6_RECOVERY_EVIDENCE_CHILD
REAP_PRIVATE_BATCH_VALIDATION_ALLOCATION_CHILD
REAP_PHASE6_OVERLOAD_ALLOCATION_CHILD
REAP_GOAL_D_CANCEL_PROBE_CHILD
RUST_TEST_THREADS
RUST_TEST_NOCAPTURE
TRYBUILD
CARGO_TARGET_DIR
```

The recorder starts its sampler before spawning the child and waits for the
sampler to exit before validation or sealing. `process.ps.tsv` begins with
this exact header:

```text
sample_utc	phase	pid	ppid	pgid	comm	argv_sha256
```

Every field is tab-separated and every row is LF-terminated. `sample_utc` is
UTC RFC 3339 with nanoseconds and `Z`; `phase` is exactly `pre`, `during`, or
`post`; numeric fields are unsigned decimal; `comm` has tabs/newlines replaced
by one space; and `argv_sha256` is lowercase SHA-256 of the process's
NUL-delimited argv bytes. Take one `pre` snapshot, one `during` snapshot
immediately after spawn and then at monotonic intervals no greater than one
second until the child tree exits, and one `post` snapshot after waiting for
the complete tree. Rows within a snapshot are sorted numerically by PID.

Process ownership is decided from the sampled PID/PPID ancestry rooted at the
recorded child PID and process group, not from a name substring alone.
Matching executable/argv identities include Cargo, rustc, every
benchmark/test binary named by this contract, Reap, and Reap CLI. Any matching
process not descended from the recorded child is overlap.

A clean nonzero command or semantic validation failure is valid red evidence.
It may not be overwritten, discarded, reclassified as contamination, or
retried on `G3`. Predeclared overlap detected before result inspection is
invalid retained evidence and stops this amendment; it is not automatic retry
authority.

Any runtime residue is inventoried read-only and stops without cleanup.

## No-Cargo Bundle Self-Test

`self-test/fixtures` contains exactly this closed layout and no other entry:

```text
reports/
  01-column-one-valid.log
  02-prefixed-valid.log
  03-zero-report.log
  04-duplicate-report.log
  05-non-whitespace-suffix.log
  06-one-exact-valid.log
  07-mutation-exact-valid.log
  08-zero-match.log
  09-failed-exact.log
  10-combined-valid.log
  11-drift-artifact-lines.log
  12-drift-artifact-bytes.log
  13-drift-artifact-sha256.log
  14-drift-recovery-sha256.log
  15-drift-recovery-peak.log
  16-drift-recovery-records.log
  17-drift-last-sequence.log
  18-drift-byte-identical.log
  19-drift-production-order-entry.log
  20-drift-normalized-projection.log
  21-wrong-build-revision.log
  22a-repository-attestation-valid.json
  22b-goal-g-attestation-valid.json
  22c-goal-gr-attestation-valid.json
  22d-siblings-attestation-valid.json
  23-inventory-valid.json
  24-source-valid.stdout
  25-engine-benchmark-valid.log
  26-live-benchmark-valid.log
  27-action-benchmark-valid.log
  28-pm-benchmark-valid.log
  29-pm-all-targets-valid.log
  30-workspace-all-targets-valid.log
  31-decision-replay-valid.log
  32-numeric-contract-valid.log
  33-chaos-valid.jsonl
  34-baseline-summary-valid.json
  35-phase0-replay-valid.tsv
  36-exit-zero.meta
  37-exit-nonzero.meta
inventory/
  cargo-metadata.json
  workspace-packages.tsv
  workspace-normal-edges.tsv
  outside-path-dependencies.tsv
  rustc-cfg.tsv
  production-paths.tsv
  production-content.tsv
  production-extent.tsv
  public-declarations.tsv
  schema-version.tsv
  functions.tsv
  state-declarations.tsv
  test-cardinality.tsv
  source-policy.tsv
  fixtures.tsv
  anchors.tsv
source/
  rebuilt-manifest.tsv
  fetch-metadata.tsv
  script-hashes.tsv
baseline/
  01-engine-warmup.log
  02-engine-run-1.log
  03-engine-run-2.log
  04-engine-run-3.log
  05-live-warmup.log
  06-live-run-1.log
  07-live-run-2.log
  08-live-run-3.log
  09-action-warmup.log
  10-action-run-1.log
  11-action-run-2.log
  12-action-run-3.log
  13-pm-warmup.log
  14-pm-run-1.log
  15-pm-run-2.log
  16-pm-run-3.log
replay/
  01-combined.stdout
  01-combined.stderr
  02-decision-replay.stdout
  02-decision-replay.stderr
  03-live-parity.stdout
  03-live-parity.stderr
  04-numeric-contract.stdout
  04-numeric-contract.stderr
  05-fixture-hashes.stdout
  05-fixture-hashes.stderr
  06-chaos-1.stdout
  06-chaos-1.stderr
  07-chaos-2.stdout
  07-chaos-2.stderr
runner/
  states.tsv
  expected.tsv
  environment.tsv
  environment.expected.tsv
scanner/
  root.rs
  nested.rs
  path-override.rs
  cycle-a.rs
  cycle-b.rs
  features.tsv
  functions.expected.tsv
  states.expected.tsv
  tests.expected.tsv
```

`01` and `02` contain the same valid combined object at column one and after a
same-line libtest prefix respectively. `03` contains no report prefix; `04`
contains two complete valid objects; `05` contains one valid object followed
by a non-whitespace byte. `06` contains one exact successful top-level test;
`07` contains the one outer/one isolated-child/two-summary shape; `08` selects
zero tests; `09` selects one failed test; and `10` contains the exact valid
outer-14/expected-child/combined-report shape. Files `11` through `20` each
derive from `10` by changing only the named anchor (the projection fixture
changes one otherwise-unanchored normalized key), and `21` changes only
`build_revision`. Files `22a` through `22d` are the four distinct complete
valid attestation kinds; no attestation object is reused under another
validator. `inventory/` contains all sixteen underlying streams and `23`
contains their exact rows/bytes/hashes; `inventory_v1` independently
recomputes `23` from that subtree. `source/` contains the exact rebuilt
128-row manifest, 28-row credential-free metadata table, and successor/old
script hashes; `24` is the exact five-line stdout, and `source_cutoff_v1`
validates all companion bytes together. `baseline/` contains all sixteen
individually valid warmup/run reports; `34` is the exact summary object
derived from those bytes, and `baseline_summary_v1` recomputes it rather than
trusting the summary alone. Files `25` through `33` provide one exact
successful authoritative input for the remaining standalone validator
families. In particular, `29` contains the exact target summaries, one valid
inherited-child combined object, the two exact library allocation-child
summaries, and one PM bench report; `30` contains the exact workspace
summaries, that same visible three-child set, and the four bench reports.
Their validators must derive report counts `2` and `5` from bytes, require
the exact child identities/cardinalities, and revalidate the combined
anchors/revision.
`replay/01` has the same bytes as report fixture `10`; `02` and `04` have the
same bytes as `31` and `32`; `03` is the exact named live-parity one-test
output; `05` contains the four exact Goal D `sha256sum` rows; and `06`/`07`
each have the same bytes as `33`. Every replay stderr file is empty. `35` is
the bytewise-sorted
`logical-path<TAB>sha256<TAB>expected-cardinality` index of all fourteen
replay stream files. The `phase0_replay_v3` fixture validator consumes those
actual replay files, recomputes `35`, and validates the same
child/report/hash relations as a real replay; an index-only shortcut is
forbidden. Files `36` and `37` differ only in command exit and gate result.
Every valid fixture binds `G3`; the self-test substitutes one fixed
64-hex synthetic `G3` consistently rather than reading the worktree.

`runner/states.tsv` contains exactly the synthetic cases
`first-empty`, `current-exists`, `predecessor-green`, `predecessor-red`,
`ordinal-gap`, `later-ordinal-exists`, `earlier-campaign-open`,
`later-campaign-exists`, `campaign-sealed`, and `prior-stop`. Each case is a
sorted path/state inventory with no filesystem side effect.
`runner/expected.tsv` maps only `first-empty` and `predecessor-green` to
`accept`; every other case maps to its exact rejection reason. The runner's
fixture-only state-machine branch must match all ten rows and prove a rejected
case creates no attempt or later path.

`states.tsv` has no header and exact columns
`<case>\t<relative-path-or-dot>\t<absent|open|sealed-green|sealed-red|invalid>`;
rows are sorted by case then path. `expected.tsv` has no header and exact
columns `<case>\t<accept|reject>\t<reason-or-none>`, one row per case in
bytewise case order.

`runner/environment.tsv` contains exactly the eight forbidden inherited
variable names above, one LF-terminated bytewise-sorted row. Its expected
file maps every name to `reject-before-child`. The fixture branch sets one
synthetic nonsecret value at a time and proves no attempt, child, or later
path is created; values are never logged.

The scanner fixtures collectively contain one example of every lexical case
listed in the inventory section. `root.rs` owns the normal/byte/raw
string/comment/character/multiline/inline-module cases and declares
`nested.rs`; it uses `#[path]` for `path-override.rs`. `cycle-a.rs` and
`cycle-b.rs` form the sole intentional closure cycle and must be rejected.
`features.tsv` fixes the exact default-feature truth table. The three expected
TSVs are the complete bytewise-sorted function, state, and active-test
projections for the non-cycle closure.

Before activation:

1. run `bash -n` on every executable;
2. prove `commands.tsv` has the exact campaigns, ordinals, labels, and row
   counts below with no duplicate key;
3. prove every row maps to one closed command/validator;
4. run validator fixtures covering:
   - column-one JSON;
   - JSON following a libtest prefix on the same line;
   - zero reports;
   - duplicate reports;
   - valid JSON with a non-whitespace suffix;
   - a normal one-test exact result;
   - the mutation test's one outer plus one isolated child result;
   - a zero-match exact result;
   - a failed exact result;
   - a valid combined report;
   - every single frozen-anchor drift; and
   - wrong build revision;
5. prove no fixture validator writes outside `self-test`;
6. run the fixture-only lexical-scanner branch of `inventory.sh` and prove it
   neither reaches Cargo metadata nor reads the worktree;
7. run the fixture-only runner state-machine branch and prove exact
   predecessor enforcement, later-path rejection, no retry, and rejection of
   every forbidden inherited variable;
8. snapshot processes before/after and prove no Cargo/rustc/test/benchmark
   child was invoked; and
9. obtain two independent static reviews of all bundle bytes and this
   contract.

`self-test.meta` records every fixture outcome and
`cargo_invoked=false`. `self-test.stdout` and `self-test.stderr` are exact
captured streams, and `self-test.sha256` uses the sorted hash-row format
defined above over every regular file below `self-test/` except
`self-test.sha256`. Failure prevents activation.

## Report Extraction

For combined reports, find the literal:

```text
{"schema_version":1,"target":"combined_replay"
```

anywhere in a line, take bytes from that offset, parse exactly one complete
JSON object, and permit only trailing whitespace. Reject zero or multiple
objects and any non-whitespace suffix.

Every report schema that carries `build_revision` must have
`build_revision == G3`. Textual engine/live reports and the action report do
not gain a field; their canonical validator projections bind the candidate
from immutable attempt metadata and require it to equal `G3`.

The combined semantic validator applies the full frozen check and normalized
projection from Goal G-R, including:

```text
artifact_lines=35012
artifact_bytes=22791589
artifact_sha256=83ced509c9ea180e66d957853f9ff7762ef3c0babc316c9251c12d4d1a5224eb
canonical_recovery_sha256=f98bf8a88f34fb6e3c4dcfd1919a2c1d4577b2da3960375e216e596d0746cd35
recovery_peak_bytes=2959343
recovery_records=35012
last_sequence=35011
byte_identical_projection=true
production_order_entry_authorized=false
normalized_projection_sha256=3fb6c3c24f2995f57d71be9ba5a4fd36c13ffe956d0ab91bc497370f6259b91a
```

The mutation-original exact test intentionally runs one outer test and one
isolated child with the same name. Its validator requires two successful
one-test summaries, exactly one outer selection, exactly one child execution,
and one combined report. It must not treat the child as a second top-level
match.

## Commands Manifest Schema

`commands.tsv` has six tab-separated columns:

```text
campaign ordinal label command_id validator_id expected_reports
```

It contains the exact rows defined below and no header. Ordinals are
zero-padded decimal strings. Repeated rows are materialized individually; the
runner never accepts a caller-supplied repetition count.

`expected_reports` is the exact decimal count of command-produced semantic
report units in the authoritative source location fixed by that validator. A
report unit may be one exact JSON object or one closed family of textual
benchmark records: the single engine line is one unit and the four named live
lines together are one unit. Validator-generated extracted copies do not
count; `inventory.json` is authoritative and its required byte-identical
stdout mirror does not count again. A report signature in any undeclared
source location is a duplicate and red. Cargo/libtest status text, normalized
Chaos JSONL events, metadata, manifests, projections, and validator results
do not count. The closed manifest is exactly these `86` LF-terminated rows,
with literal tab separators and no blank line:

```text
attestation	01	repository-identity	attest_repository	repository_v1	1
attestation	02	goal-g-evidence	attest_goal_g	goal_g_tree_v1	1
attestation	03	goal-gr-evidence	attest_goal_gr	goal_gr_trees_v1	1
attestation	04	siblings	attest_siblings	siblings_v1	1
attestation	05	inventories	run_inventory	inventory_v1	1
source	01	source-reattest	run_source_reattest	source_cutoff_v1	0
confidence	01	fmt-check	fmt_check	exit_zero	0
confidence	02	mutation-original-01	mutation_original	mutation_exact_v1	1
confidence	03	mutation-original-02	mutation_original	mutation_exact_v1	1
confidence	04	mutation-original-03	mutation_original	mutation_exact_v1	1
confidence	05	mutation-original-04	mutation_original	mutation_exact_v1	1
confidence	06	mutation-original-05	mutation_original	mutation_exact_v1	1
confidence	07	mutation-original-06	mutation_original	mutation_exact_v1	1
confidence	08	mutation-original-07	mutation_original	mutation_exact_v1	1
confidence	09	mutation-original-08	mutation_original	mutation_exact_v1	1
confidence	10	mutation-original-09	mutation_original	mutation_exact_v1	1
confidence	11	mutation-original-10	mutation_original	mutation_exact_v1	1
confidence	12	capture-original-01	capture_original	one_exact_v1	0
confidence	13	capture-original-02	capture_original	one_exact_v1	0
confidence	14	capture-original-03	capture_original	one_exact_v1	0
confidence	15	capture-original-04	capture_original	one_exact_v1	0
confidence	16	capture-original-05	capture_original	one_exact_v1	0
confidence	17	capture-original-06	capture_original	one_exact_v1	0
confidence	18	capture-original-07	capture_original	one_exact_v1	0
confidence	19	capture-original-08	capture_original	one_exact_v1	0
confidence	20	capture-original-09	capture_original	one_exact_v1	0
confidence	21	capture-original-10	capture_original	one_exact_v1	0
confidence	22	ack-regression-01	ack_regression	one_exact_v1	0
confidence	23	ack-regression-02	ack_regression	one_exact_v1	0
confidence	24	ack-regression-03	ack_regression	one_exact_v1	0
confidence	25	ack-regression-04	ack_regression	one_exact_v1	0
confidence	26	ack-regression-05	ack_regression	one_exact_v1	0
confidence	27	ack-regression-06	ack_regression	one_exact_v1	0
confidence	28	ack-regression-07	ack_regression	one_exact_v1	0
confidence	29	ack-regression-08	ack_regression	one_exact_v1	0
confidence	30	ack-regression-09	ack_regression	one_exact_v1	0
confidence	31	ack-regression-10	ack_regression	one_exact_v1	0
confidence	32	capture-regression-01	capture_regression	one_exact_v1	0
confidence	33	capture-regression-02	capture_regression	one_exact_v1	0
confidence	34	capture-regression-03	capture_regression	one_exact_v1	0
confidence	35	capture-regression-04	capture_regression	one_exact_v1	0
confidence	36	capture-regression-05	capture_regression	one_exact_v1	0
confidence	37	capture-regression-06	capture_regression	one_exact_v1	0
confidence	38	capture-regression-07	capture_regression	one_exact_v1	0
confidence	39	capture-regression-08	capture_regression	one_exact_v1	0
confidence	40	capture-regression-09	capture_regression	one_exact_v1	0
confidence	41	capture-regression-10	capture_regression	one_exact_v1	0
confidence	42	combined-default-01	combined_default	combined_v1	1
confidence	43	combined-default-02	combined_default	combined_v1	1
confidence	44	combined-default-03	combined_default	combined_v1	1
confidence	45	combined-default-04	combined_default	combined_v1	1
confidence	46	combined-default-05	combined_default	combined_v1	1
confidence	47	combined-default-06	combined_default	combined_v1	1
confidence	48	combined-default-07	combined_default	combined_v1	1
confidence	49	combined-default-08	combined_default	combined_v1	1
confidence	50	combined-default-09	combined_default	combined_v1	1
confidence	51	combined-default-10	combined_default	combined_v1	1
confidence	52	combined-serial-01	combined_serial	combined_v1	1
confidence	53	combined-serial-02	combined_serial	combined_v1	1
confidence	54	combined-serial-03	combined_serial	combined_v1	1
confidence	55	pm-live-all-targets	pm_all_targets	pm_all_targets_v1	2
confidence	56	pm-live-clippy	pm_clippy	exit_zero	0
confidence	57	workspace-all-targets	workspace_all_targets	workspace_all_targets_v1	5
confidence	58	decision-replay	decision_replay	decision_replay_v1	0
confidence	59	live-decision-parity	live_parity	one_exact_v1	0
confidence	60	numeric-contract	numeric_contract	numeric_contract_v1	0
confidence	61	chaos-backtest-1	chaos_backtest	chaos_v1	0
confidence	62	chaos-backtest-2	chaos_backtest	chaos_v1	0
baseline	01	engine-warmup	engine_bench	engine_benchmark_v1	1
baseline	02	engine-run-1	engine_bench	engine_benchmark_v1	1
baseline	03	engine-run-2	engine_bench	engine_benchmark_v1	1
baseline	04	engine-run-3	engine_bench	engine_benchmark_v1	1
baseline	05	live-warmup	live_bench	live_benchmark_v1	1
baseline	06	live-run-1	live_bench	live_benchmark_v1	1
baseline	07	live-run-2	live_bench	live_benchmark_v1	1
baseline	08	live-run-3	live_bench	live_benchmark_v1	1
baseline	09	action-warmup	action_bench	action_benchmark_v1	1
baseline	10	action-run-1	action_bench	action_benchmark_v1	1
baseline	11	action-run-2	action_bench	action_benchmark_v1	1
baseline	12	action-run-3	action_bench	action_benchmark_v1	1
baseline	13	pm-warmup	pm_bench	pm_benchmark_v1	1
baseline	14	pm-run-1	pm_bench	pm_benchmark_v1	1
baseline	15	pm-run-2	pm_bench	pm_benchmark_v1	1
baseline	16	pm-run-3	pm_bench	pm_benchmark_v1	1
baseline	17	baseline-summary	summarize_baseline	baseline_summary_v1	1
replay	01	current-phase0	run_phase0_replay	phase0_replay_v3	1
```

The self-test reconstructs that literal stream independently, requires
`5 + 1 + 62 + 17 + 1 == 86`, proves every ordinal is contiguous within its
campaign, and proves each `(campaign, ordinal, label)` and each label within a
campaign is unique. A row count, byte, order, command ID, validator ID, or
expected-report mismatch prevents activation.

## Attestation Campaign

The exact rows are:

| Ordinal | Label | Command ID | Validator |
| ---: | --- | --- | --- |
| 01 | `repository-identity` | `attest_repository` | `repository_v1` |
| 02 | `goal-g-evidence` | `attest_goal_g` | `goal_g_tree_v1` |
| 03 | `goal-gr-evidence` | `attest_goal_gr` | `goal_gr_trees_v1` |
| 04 | `siblings` | `attest_siblings` | `siblings_v1` |
| 05 | `inventories` | `run_inventory` | `inventory_v1` |

The campaign is `attestation`.

The first four attestation commands each emit exactly one compact `jq -S -c`
JSON object and one LF to stdout and emit empty stderr on success. The
recorder extracts that object to `<label>.json`; the extracted validator copy
is not a second command report. The object has exactly:

```json
{
  "schema": "goal-g-amendment-3-attestation-v1",
  "build_revision": "<G3>",
  "kind": "<repository|goal-g|goal-gr|siblings>",
  "facts": {}
}
```

The closed validator for each row fixes every allowed `facts` key and type;
unknown/missing/duplicate keys, a second object, non-whitespace suffix, or
`build_revision != G3` is red. The fifth command's one report is the
`goal-g-amendment-3-inventory-v1` object defined below; it does not also emit
an attestation wrapper or embed source bodies.

`attest_repository` runs only these read-only facts:

```bash
git rev-parse HEAD
git rev-parse HEAD^{tree}
git branch --show-current
git status --porcelain=v1 --untracked-files=all
git rev-list --left-right --count origin/master...HEAD
sha256sum Cargo.lock
rustc -Vv
cargo -V
uname -a
getconf _NPROCESSORS_ONLN
df --output=avail -B1 "$(git rev-parse --show-toplevel)"
git worktree list --porcelain
ps -eo pid=,ppid=,pgid=,comm=,args=
```

`attest_goal_g` computes the exact Goal G-R/A6 `find`-defined counts, file
stream, and inventory stream for both Goal G roots, plus the four named
amended-root hashes. Its validator requires both complete historical
aggregates, not merely the four named files:

```text
historical_root=target/tmp/goal-g-phase0
historical_regular_files=4158
historical_entries_excluding_root=5038
historical_file_stream_sha256=ad921fc06db0a68b6e0822208106df2d8c6d276b24d0f4bb342a84f8b738b8d9
historical_inventory_stream_sha256=4ba698c8804850eeafd3eaef333cf9a6b419d0a66df78a8bd001808eb4d30a4d
amended_root=target/tmp/goal-g-phase0-amended
regular_files=11594
entries_excluding_root=12253
file_stream_sha256=35a99a10c133fd680cef1f4e411dbc55490f4e41199411aae907cd348aced340
inventory_stream_sha256=23c4b85375e2d27e657c38b4560c3ee1bfecae1c1b5c98baf4cf1462dc05f7b2
replay_selected_sha256=4168ac456d70361429967d7457e0d5850cd014c0b0ea7b8e45e3183372ec766d
combined_replay_log_sha256=fe3e8c7323c52163345e6330ebd7587858990a49d1bc436a1a669792f6473cd9
replay_meta_sha256=b2dc689182ea8c02fd340669b2b0f142b6cafd15d5ec38a04cda221f3aaa8f56
replay_process_sha256=fd77e0c1db9970bbe2c20eea70dc8836091a81e77d9bd66491c4d8150f4bf0c3
source_manifest_sha256=f38625a6f2bb0a2c8e13598acf6ab7dc1eccc57f97a7f4a8c45fdb810e8fcb4d
source_reattest_log_sha256=649a510599c591963c37dd1aaea579b9eefa2aba641a4dd155c5e70e21a4d9be
baseline_campaign_sha256=009a2faeaf2e6c777c3959d4cd92607f095036b42dcba2b90ea45a428b047a79
baseline_summary_sha256=3de85bbd7145d6692cc383ea60783f072af243981c09902d26aac9c1668929e6
pm_retained_sha256=8384d7637819107bae1bacabd580c09a19c185f2416f5f1e5bf6ff2d0741bac5
```

Both historical aggregates are pass/fail byte evidence. They are never
regenerated from a subset and never rewritten. Recompute and validate both
before/after every child and at the final pass/stop gate.

`attest_goal_gr` independently computes the exact A6 `find`-defined
hashes/counts/streams for:

```text
prior_root=target/tmp/goal-g-replay-repair
prior_regular_files=70
prior_entries_excluding_root=85
prior_file_stream_sha256=54d59957045444e32488a9dda0619440e983b5be779e3004045aac3e68662246
prior_inventory_sha256=32c47a75092a8a0598f0205e53f495023e80ee6d7279d406059c685401d83171
```

For the newly completed Amendment 6 root, bundle construction requires the
direct-child `R6` commit and exactly one completion block containing these
unique keys:

```text
goal_g_r_amendment_6_completed_evidence_root
goal_g_r_amendment_6_completed_evidence_regular_files
goal_g_r_amendment_6_completed_evidence_entries_excluding_root
goal_g_r_amendment_6_completed_evidence_file_stream_sha256
goal_g_r_amendment_6_completed_evidence_inventory_sha256
```

It rejects a duplicate/missing/malformed key, imports those exact values into
the sealed validator, and binds them in the bundle manifest and `G3`
activation handoff. `attest_goal_gr` recomputes and compares the completed
root before/after every child and at the final pass/stop gate; it never trusts
the handoff values without recomputation.

`attest_siblings` runs only Git object/status commands. It verifies clean
`../imm-strategy` at
`b6b120c7b7c466d8431bf082f3229328c5d7b2ae`, verifies Predarb object
`8222273a9c72033b760e1d2fec813bc77144556d` exists, and records only Predarb
dirty path names. It never opens dirty contents.

`inventory.sh` receives no caller-controlled argument. The recorder sets one
internal absolute `REAP_ATTEMPT_STAGING` path below the current attempt and
the helper writes only the files named below there. It runs from the
repository root with `LC_ALL=C`, `TZ=UTC`, `CARGO_NET_OFFLINE=true`, and the
activation candidate checked out. Its only Cargo subprocess is exactly:

```bash
cargo metadata --locked --offline --format-version 1
```

The script first obtains the tracked path set with exactly
`git ls-files -z`. It defines a production Rust path as either
`crates/*/src/**/*.rs`, `crates/*/src/*.rs`, or `crates/*/build.rs` in that
tracked set. Every path emitted below is repository-relative, slash
separated, and bytewise sorted; source text is used byte-for-byte except
where a record explicitly says `trim`.

It creates exactly these immutable data files:

| File | Exact records |
| --- | --- |
| `cargo-metadata.json` | `jq -S -c` canonical form of the complete metadata object, one LF |
| `workspace-packages.tsv` | one `<package-name>\t<relative-manifest-path>` row for each workspace member |
| `workspace-normal-edges.tsv` | one `<from-package>\t<dependency-name>\t<to-package>` row for each normal (`kind == null`) edge whose target is a workspace member |
| `outside-path-dependencies.tsv` | one `<from-package>\t<dependency-name>\t<canonical-manifest-path>` row for each `source == null` dependency whose manifest is not below the repository root |
| `rustc-cfg.tsv` | exact `rustc --print cfg` output, one bytewise-sorted unique LF-terminated row per active target predicate |
| `production-paths.tsv` | one production Rust path per row |
| `production-content.tsv` | one `<sha256>\t<byte-count>\t<path>` row per production Rust path |
| `production-extent.tsv` | one `<line-count>\t<path>` row per production Rust path, where line count is the number of LF-delimited lines plus one only for a nonempty non-LF-terminated tail |
| `public-declarations.tsv` | one `<path>\t<line>\t<trim>` row for each production line matching `^[[:space:]]*pub([[:space:](]|$)`; `trim` removes leading/trailing ASCII whitespace only |
| `schema-version.tsv` | one `<path>\t<line>\t<trim>` row for each tracked Rust line containing the case-sensitive token `SCHEMA_VERSION`, `schema_version`, or `schema=` |
| `functions.tsv` | one `<path>\t<start-line>\t<end-line>\t<name>` row from a brace-balanced lexical scan of every production `fn` item; comments and string/character/raw-string bodies are excluded before brace counting; malformed/unbalanced input is red |
| `state-declarations.tsv` | one `<path>\t<line>\t<kind>\t<name>` row for every production `struct`, `enum`, `union`, `static`, or `thread_local!` declaration found by the same lexical scanner |
| `test-cardinality.tsv` | one `<package>\t<target-kind>\t<target-name>\t<expected-tests>` row for every workspace lib, bin, example, and integration-test target; expected tests are the exact count of active `#[test]` and `#[tokio::test]` items in the target's tracked source closure; harness-free benches are represented separately by their semantic report validators |
| `source-policy.tsv` | one `<sha256>\t<byte-count>\t<path>` row for each tracked Rust path below `crates/*/tests/` whose basename is `source_policy.rs`, `private_source_policy.rs`, `dependency_policy.rs`, or `compile_fail_boundaries.rs`, plus every tracked path below a `tests/compile_fail/` directory |
| `fixtures.tsv` | one `<sha256>\t<byte-count>\t<path>` row for every tracked regular file below `fixtures/` |
| `anchors.tsv` | the four Goal D fixture hashes, Goal F fixture manifest/provenance hashes, Goal F combined-replay anchors, and canonical Chaos hash as exact `name\tsha256-or-value` rows frozen by Amendment 3 |

The lexical scanner is implemented inside the sealed `inventory.sh`; its
fixture corpus in `self-test/fixtures` must cover nested braces, attributes,
visibility forms, comments, normal/byte/raw strings, character literals, and
multiline function signatures. Both reviewers compare the scanner and
fixtures byte-for-byte. No heuristic tool version or host-dependent parser is
allowed.

For `test-cardinality.tsv`, target roots and kinds come only from canonical
Cargo metadata. A tracked source closure begins at `target.src_path` and
recursively follows active out-of-line `mod IDENT;` declarations to
`IDENT.rs` or `IDENT/mod.rs` by Rust's module path rules; duplicate paths are
counted once per target. The helper records exact `rustc --print cfg` output
for `G3`. The scanner recursively evaluates `all`, `any`, and `not` over only
the literal atoms `test`, `unix`, `target_os = "linux"`, and package
`feature = "..."`; `unix` and `target_os` truth come from that recorded rustc
set, and feature truth comes from canonical metadata for the exact
all-targets command. These atoms cover the current tracked workspace. Any
other conditional enclosing a test is red. Inactive test items do not count.
The self-test covers closure cycles, `#[path]`, nested modules, target/Unix
and feature predicates, recursive Boolean operators, and inactive tests.

The helper then writes `inventory.json` as one `jq -S -c` object followed by
one LF with exactly this schema:

```json
{
  "schema": "goal-g-amendment-3-inventory-v1",
  "build_revision": "<G3>",
  "streams": {
    "<data-file-name>": {
      "rows": 0,
      "bytes": 0,
      "sha256": "<64 lowercase hex>"
    }
  }
}
```

`streams` contains exactly the fifteen data files above other than
`cargo-metadata.json`, plus `cargo-metadata.json` itself, for sixteen keys.
`rows` is the number of LF-terminated records (`1` for canonical metadata);
`bytes` and `sha256` cover the exact file. The script's
stdout is exactly the byte content of `inventory.json`; stderr is empty on
success. The validator independently recomputes all sixteen counts/hashes,
requires `build_revision == G3`, requires zero outside-path rows, and hashes
the exact `inventory.json`. Any disagreement is red.

## Source Campaign And Successor Verifier

The campaign contains:

| Ordinal | Label | Command ID | Validator |
| ---: | --- | --- | --- |
| 01 | `source-reattest` | `run_source_reattest` | `source_cutoff_v1` |

The new `source-reattest.sh` may read, but never write, these retained inputs:

```text
target/tmp/goal-g-phase0-amended/authoritative-source-manifest.tsv
target/tmp/goal-g-phase0-amended/official-docs/**
target/tmp/goal-g-phase0-amended/official-git/**
target/tmp/goal-g-phase0-amended/vector-oracle/**
target/tmp/goal-g-phase0-amended/build-authoritative-source-manifest.sh
target/tmp/goal-g-phase0-amended/verify-source-cutoff.sh
```

Reading the last two scripts is authorized solely for byte/hash and
line-by-line successor review. Neither old script may be invoked, sourced, or
copied into an executable location.

The successor verifier:

- verifies the old verifier SHA-256
  `ffa352b883f1d00b9f8dde6ce40566f4dcd137f0c90ea6aaca7f78bde900f713`;
- verifies the old build-script SHA-256
  `a25e2e3dcad149d774c24b7f367bd9ec7211a0e70dab4833f9b2fcbd269abcb6`;
- verifies the 33/53/7/28-row manifest hashes frozen in the boundary;
- verifies all 33 retained document body byte counts and SHA-256 values;
- verifies all 60 retained Git bodies by byte count, Git blob, and SHA-256;
- credential-free fetches exactly the 28 addendum paths at their pinned
  revisions using HTTPS, no proxy, TLS 1.2 minimum, and a 60-second bound;
- verifies cached `k256 0.13.4`, `reqwest 0.12.28`, and `sha3 0.10.9` crate
  archives against the frozen hashes;
- verifies the frozen viem package/vector/generator hashes and version;
- independently rebuilds the canonical sorted unique 128-row manifest in the
  Amendment 3 runtime root without executing an old helper; and
- compares it byte-for-byte with the retained authoritative manifest whose
  SHA-256 is
  `f38625a6f2bb0a2c8e13598acf6ab7dc1eccc57f97a7f4a8c45fdb810e8fcb4d`.

Its exact successful stdout, including capitalization, spaces, and LF after
the last line, is:

```text
verified retained document bodies: 33
verified retained Git bodies: 60
verified pinned addendum blobs by credential-free fetch: 28
verified cached crate archives: 3
verified authoritative source manifest: 128 rows
```

Its stderr is empty on success. Every network response body, header capture,
checkout, rebuilt intermediate, and temporary manifest is created only below
the canonical Amendment 3 runtime root. The verifier may not use `/tmp`,
the host default temporary directory, the bundle, the repository worktree, or
the evidence attempt as scratch space. Before exit it copies only the
successfully rebuilt canonical 128-row manifest and a credential-free
metadata table of URL host, pinned revision/path, HTTP status, byte count,
Git blob, and SHA-256 into the attempt staging path; it never copies a fetched
body or response header. It then removes its own runtime children and proves
the runtime root empty. Runtime residue, including on failure, is retained
in place and is a stop rather than cleanup authority.

The attempt retains stdout, stderr, successor-script hash, rebuilt
manifest/hash, the metadata table without headers/tokens, and semantic
validation. No response replaces a retained body.

The successor script's exact hash and line-by-line equivalence review are
frozen in `G3` before any public fetch.

## Confidence Campaign

The campaign is `confidence` and contains exactly `62` rows:

| Ordinals | Label pattern | Command ID | Validator |
| --- | --- | --- | --- |
| 01 | `fmt-check` | `fmt_check` | `exit_zero` |
| 02-11 | `mutation-original-01` … `-10` | `mutation_original` | `mutation_exact_v1` |
| 12-21 | `capture-original-01` … `-10` | `capture_original` | `one_exact_v1` |
| 22-31 | `ack-regression-01` … `-10` | `ack_regression` | `one_exact_v1` |
| 32-41 | `capture-regression-01` … `-10` | `capture_regression` | `one_exact_v1` |
| 42-51 | `combined-default-01` … `-10` | `combined_default` | `combined_v1` |
| 52-54 | `combined-serial-01` … `-03` | `combined_serial` | `combined_v1` |
| 55 | `pm-live-all-targets` | `pm_all_targets` | `pm_all_targets_v1` |
| 56 | `pm-live-clippy` | `pm_clippy` | `exit_zero` |
| 57 | `workspace-all-targets` | `workspace_all_targets` | `workspace_all_targets_v1` |
| 58 | `decision-replay` | `decision_replay` | `decision_replay_v1` |
| 59 | `live-decision-parity` | `live_parity` | `one_exact_v1` |
| 60 | `numeric-contract` | `numeric_contract` | `numeric_contract_v1` |
| 61 | `chaos-backtest-1` | `chaos_backtest` | `chaos_v1` |
| 62 | `chaos-backtest-2` | `chaos_backtest` | `chaos_v1` |

The exact Cargo argv vectors are:

```bash
cargo fmt --all -- --check

cargo test --locked -p reap-pm-live --test combined_replay \
  phase6_real_mutation_artifacts_recover_to_the_same_bounded_projection \
  -- --exact --test-threads=1 --nocapture

cargo test --locked -p reap-pm-live --test combined_replay \
  raw_frame_and_raw_count_bounds_are_exact \
  -- --exact --test-threads=1 --nocapture

cargo test --locked -p reap-pm-live --lib \
  evidence::workload::tests::real_writer_acknowledgement_is_bound_to_expected_prepared_effect \
  -- --exact --test-threads=1 --nocapture

cargo test --locked -p reap-pm-live --test combined_replay \
  terminal_capture_finish_preserves_primary_shutdown_error_before_prefix_verification \
  -- --exact --test-threads=1 --nocapture

cargo test --locked -p reap-pm-live --test combined_replay -- --nocapture

cargo test --locked -p reap-pm-live --test combined_replay \
  -- --test-threads=1 --nocapture

cargo test --locked -p reap-pm-live --all-targets

cargo clippy --locked -p reap-pm-live --all-targets -- -D warnings

cargo test --locked --workspace --all-targets

cargo test --locked -p reap-engine --test decision_replay

cargo test --locked -p reap-live --lib \
  coordinator::tests::decision_parity::initialized_live_reduction_matches_engine_decisions_and_is_byte_stable \
  -- --exact

cargo test --locked -p reap-pm-core --test numeric_contract

cargo run --locked -q -p reap-cli -- \
  backtest --format normalized-jsonl \
  --config examples/iarb2-basic.toml \
  --data fixtures/normalized/chaos_quote_hedge.jsonl --pretty
```

Every mutation/full-combined report is semantically validated. The two
backtest stdout logs must be byte-identical and each hash to
`38acf9f5e0c310f2ec5528974beffadf4c1a7f84d46efa8d9664ee7051e84691`.

The validator cardinalities are exact:

- `one_exact_v1` accepts exactly one selected top-level named test, one
  `1 passed; 0 failed` summary, no child execution, and zero semantic JSON
  reports.
- `mutation_exact_v1` accepts exactly one selected outer test, exactly one
  isolated child execution of that same test, exactly two
  `1 passed; 0 failed` summaries, and exactly one combined report. The child
  is not a second top-level selection.
- `combined_v1` accepts exactly one final outer
  `14 passed; 0 failed; 0 ignored; 0 measured; 0 filtered out` summary,
  exactly one expected isolated execution of the mutation test with its own
  successful one-test child summary, and exactly one combined report. It
  rejects any other child or summary. The default rows must not contain
  `--test-threads`; the serial rows must contain exactly
  `--test-threads=1`.
- `decision_replay_v1` accepts exactly `7 passed; 0 failed` and zero semantic
  reports.
- `numeric_contract_v1` accepts exactly `16 passed; 0 failed` and zero
  semantic reports.
- `pm_all_targets_v1` requires the library summary `167 passed; 0 failed`;
  exactly one inherited-stdio successful one-test child for each of
  `evidence::overload_tests::batch_validation::repeated_private_batch_validation_is_allocation_free`
  and
  `lanes::phase6_overload_tests::allocation::thirteen_pm_live_overload_mechanisms_are_allocation_free`;
  these exact integration summaries:

  ```text
  active_reducer_contract=6
  combined_replay=14
  compile_fail_boundaries=1
  composition_contract=1
  dependency_policy=13
  lane_contract=8
  phase3_active_lane_enactment_contract=16
  phase3_fixture_reducer_contract=1
  phase3_reducer_ownership_contract=7
  phase3_socket_terminal_evidence=3
  phase3_terminal_contract=4
  phase4_private_monitor_contract=8
  phase6_evidence_policy=3
  pm_schedule_contract=3
  public_route_contract=6
  ```

  It permits no missing, duplicate, ignored, or failed target and requires
  exactly one inherited-child combined report plus exactly one
  `pm_action_path` report from the harness-free bench. It validates the
  combined report's full frozen anchors, projection, and
  `build_revision == G3`. The integration counts sum to `94`.
- `workspace_all_targets_v1` requires one successful summary for every
  target and the exact count in the sealed attestation
  `test-cardinality.tsv`; rejects an unexpected, missing, duplicate,
  filtered, or ignored target; accepts a zero-test summary if and only if that
  target's sealed expected count is zero, and rejects zero for every positive
  expected count; requires exactly the same two inherited-stdio library
  allocation children as `pm_all_targets_v1`; and requires exactly four
  harness-free reports, one each
  for `event_loop`, `live_loop`, `action_path`, and `pm_action_path`, plus
  exactly one inherited-child combined report with the full frozen anchors,
  projection, and `build_revision == G3`.
- `engine_benchmark_v1` requires exactly one complete `event_loop:` line;
  `live_benchmark_v1` requires exactly one line each for
  `wire_parse_and_raw_record`, `dedup_sequence_and_book`,
  `coordinator_strategy_risk_storage_records`, and `live_parity_observe` and
  treats that closed four-line set as one report;
  `action_benchmark_v1` requires exactly one `ACTION_PATH_JSON=` object; and
  `pm_benchmark_v1` requires exactly one `pm_action_path` object. Each
  validator produces a canonical JSON projection with
  `build_revision == G3`, requires the frozen
  workload/sample/cardinality/allocation/queue/resource fields, and rejects a
  missing/duplicate family member, second report, or non-whitespace suffix
  after a JSON object.
- `baseline_summary_v1` requires exactly one summary object constructed only
  from the twelve retained run reports, plus four warmup validations that are
  excluded from medians.
- `phase0_replay_v3` requires exactly one combined report in its nested
  combined log, exact child/test cardinalities stated in the replay section,
  and no other semantic report object.

All report counts are independently derived from bytes, then compared with
the row's `expected_reports`. Cargo exit zero or a matching test summary
cannot compensate for a report-count or semantic mismatch.

## Baseline Campaign

The campaign is `baseline` and contains exactly `17` rows:

| Ordinal | Label | Exact Cargo argv |
| ---: | --- | --- |
| 01 | `engine-warmup` | `cargo bench --locked -p reap-engine --bench event_loop` |
| 02 | `engine-run-1` | same |
| 03 | `engine-run-2` | same |
| 04 | `engine-run-3` | same |
| 05 | `live-warmup` | `cargo bench --locked -p reap-live --bench live_loop` |
| 06 | `live-run-1` | same |
| 07 | `live-run-2` | same |
| 08 | `live-run-3` | same |
| 09 | `action-warmup` | `cargo bench --locked -p reap-live --bench action_path` |
| 10 | `action-run-1` | same |
| 11 | `action-run-2` | same |
| 12 | `action-run-3` | same |
| 13 | `pm-warmup` | `cargo bench --locked -p reap-pm-live --bench pm_action_path` |
| 14 | `pm-run-1` | same |
| 15 | `pm-run-2` | same |
| 16 | `pm-run-3` | same |
| 17 | `baseline-summary` | frozen `summarize-baseline.sh` |

Warmup outcomes are retained but excluded from medians. Every invocation
passes its exact logical/resource validator.

The old-to-new bridge is deliberately schema/non-timing only. It requires:

```text
engine events=250000
engine intents=999996
live non-timing projection sha256=0fc1f8c034cf568b4effcc84791264e1b7aedf81e2b793feba015ab7ef3dedaa
action non-timing projection sha256=0c6d3e818cc9ad9b37c1576973f1a634e2a1fc33f199382b1537d59a58de2c02
pm non-timing projection sha256=cc90806d19c5d2a252acbd64f3439ece2a0cb1b9d44566b84aa421d8c37b708c
```

For each family, the validator also requires the historical and new report
schema versions, workload identity, timed-boundary identity,
sample/cardinality counters, allocation/byte/queue/resource limits, and
non-timing normalized keys to be identical. The historical medians
(`23,565`, `45,021`, `57,418`, `78,546`, and `176,300` ns for PM) are retained
as historical context only. No old-to-new elapsed-time, quantile, or maximum
comparison is a gate, because these are different shared-host campaigns.

After that bridge passes, `baseline-summary` applies each original
family-specific method. Engine, live, and action each contribute their
original metric or per-workload metric from each retained invocation, then
take the median of the three retained values. PM first takes each
invocation/quantile's median across its three internal recorded runs and then
the median across the three retained invocation medians. All arithmetic is
checked integer arithmetic where the original schema is integral. The
summary records every timing field but freezes these fresh values as the sole
Amendment 3 baseline. Only the later final-candidate same-host campaign
applies the original relative timing gates against this fresh baseline.

## Replay Campaign

The campaign contains exactly:

| Ordinal | Label | Command ID | Validator |
| ---: | --- | --- | --- |
| 01 | `current-phase0` | `run_phase0_replay` | `phase0_replay_v3` |

The frozen `run-phase0-replay.sh` performs, in this exact order:

1. complete default-parallel `combined_replay`, exactly `14 passed; 0 failed`
   and exactly one combined report;
2. Goal D `decision_replay`, exactly `7 passed; 0 failed`;
3. exact initialized live parity, exactly `1 passed; 0 failed`;
4. PM `numeric_contract`, exactly `16 passed; 0 failed`;
5. direct hashes of all four Goal D fixtures;
6. canonical Chaos backtest run 1;
7. canonical Chaos backtest run 2;
8. byte comparison and exact Chaos SHA-256;
9. combined report/normalized projection validation; and
10. final repository/evidence/runtime/process checks.

It retains separate nested stdout/stderr files for every child plus replay
metadata, process snapshots, reports, projections, comparisons, and hashes.
The outer attempt is valid green only when every child and validator is green.
A clean failure is selected immutable red evidence.

The helper invokes exactly seven direct ordered child commands: four Cargo
test commands, one direct four-fixture hash command, and two Reap CLI
commands. The combined test command owns exactly its expected isolated
mutation child and no other nested test child. No direct or nested child is
retried. The four outer test summaries, the one expected mutation-child
summary, one combined semantic report, four direct fixture hashes, and two
byte-identical Chaos streams are all mandatory; the two Chaos streams each
hash to
`38acf9f5e0c310f2ec5528974beffadf4c1a7f84d46efa8d9664ee7051e84691`.
Only the one combined object counts toward this row's
`expected_reports=1`.

## Campaign Completion

The evidence root, `<G3>` directory, and active campaign directory are created
mode `700` with `umask 077`. A completed campaign contains exactly its
ordinal attempt directories plus:

```text
campaign.tsv
campaign.meta
campaign.sha256
```

`campaign.tsv` has this exact tab-separated header:

```text
ordinal	label	attempt_sha256	command_exit	validation_result	evidence_valid	gate_pass
```

It then has one row per attempt in ascending ordinal order. `attempt_sha256`
is the lowercase SHA-256 of that attempt's `attempt.sha256`;
`validation_result` is `pass`, `fail`, or `invalid`; booleans are lowercase;
and every row is LF-terminated. `campaign.meta` is an ASCII `key=value`
record with keys bytewise sorted and exactly one LF per record. It binds
schema `goal-g-amendment-3-campaign-v1`, `G3`, campaign name, expected and
actual row counts, first/last UTC, bundle hashes, runtime-empty result,
process-overlap result, campaign result, and the exact five safety booleans.

`campaign.sha256` has exactly two sorted standard hash rows, one each for
`campaign.meta` and `campaign.tsv`. After verifying it, all three files become
mode `400`, every attempt is already mode `500`, and the campaign directory
becomes mode `500`. A green campaign has all `pass/true/true` rows. A
valid-red or invalid campaign is also sealed, stops the amendment, and may not
be replaced.

## Stop Capsule And Partial-Root Seal

After the initial post-activation storage/path gate succeeds, create the
evidence root and `<G3>` directory before the first campaign. Every later
preflight, invalid-evidence, valid-red command, or valid-red validator stop
must close the partial root without running another goal command.

First terminate and wait for the recorder-owned Cargo/test process group,
every descendant, and the sampler. Only after all are proven exited may the
recorder seal any started attempt. If the active campaign directory already
exists, write its `campaign.tsv`, `campaign.meta`, and `campaign.sha256` using
its actual prefix of rows and result `valid-red`, `invalid`, or
`preflight-stop`; a zero-attempt existing campaign has only the TSV header.
Seal that campaign mode `500`. A stop between campaigns creates no campaign
directory. Do not create any later campaign directory.

Write exactly these three files at `<G3>`:

```text
stop.manifest
stop.meta
stop.sha256
```

`stop.manifest` is a bytewise-sorted standard repository-relative hash stream
over every regular file then present below `<G3>` except those three stop
files. `stop.meta` contains exactly these bytewise-sorted, one-LF
`key=value` records:

```text
authenticated_external_request_sent=false
build_revision=<G3>
bundle_sha256=<64 lowercase hex>
campaign=<campaign-or-none>
candidate_tree=<40 lowercase hex>
cargo_lock_sha256=<64 lowercase hex>
command_exit=<signed-decimal-or-not-started>
evidence_valid=<true|false>
gate_pass=false
label=<label-or-none>
last_green_campaign=<campaign-or-none>
last_green_ordinal=<two-decimal-or-none>
ordinal=<two-decimal-or-none>
partial_manifest_sha256=<64 lowercase hex>
process_overlap=<true|false>
production_order_entry_authorized=false
real_credentials_loaded=false
real_order_submitted=false
real_polygon_rpc_request_sent=false
reason_code=<closed-reason>
runtime_empty=<true|false>
runtime_inventory_sha256=<64-lowercase-hex-or-none>
schema=goal-g-amendment-3-stop-v1
stop_class=<preflight|valid-red|invalid>
utc=<RFC3339-nanoseconds-Z>
validation_result=<not-run|pass|fail|invalid>
```

`closed-reason` is exactly `preflight:<gate-id>`,
`command:<command_id>:nonzero`, `validator:<validator_id>:fail`, or
`runner:<runner-id>`. `command_id` and `validator_id` must occur in the
86-row manifest. `gate-id` is one of `storage`, `candidate`, `tree`,
`worktree`, `cargo-lock`, `bundle`, `historical-evidence`, `path`,
`runtime`, `process`, `sibling`, or `source-network`.
`runner-id` is one of `ordinal-gap`, `current-exists`, `later-exists`,
`campaign-order`, `campaign-sealed`, `prior-stop`, `sampler`, `hash`,
`seal`, or `internal-contract`.

`stop.sha256` has exactly two sorted standard hash rows for `stop.manifest`
and `stop.meta`. Verify it, set all three files mode `400`, set every
remaining partial campaign directory, `<G3>`, and the evidence root mode
`500`, then compute the final whole-root file stream and
type/mode/size/path/link inventory. Record their counts and hashes in the
tracked stop handoff. The stopped `G3` can never resume, retry, or acquire a
later path.

If the failing gate itself proves there is insufficient storage to write the
capsule, that the evidence path/bundle cannot be trusted without following a
symlink or executing changed bytes, or that a recorder-owned child/descendant
cannot be proven exited, perform no further evidence-root mutation. Preserve
the partial root byte-for-byte and record the unsealed closeout blocker in
the tracked handoff when that can be done safely. These are the only physical
closeout exceptions; none is retry or cleanup authority.

After the replay campaign passes, rerun every repository, lock, bundle,
historical-evidence, sibling, runtime-empty, process, path, and storage check
before creating a Phase 0 pass manifest. Any failure enters the stop-capsule
path above; no pass artifact exists yet. Only a fully green final check may
write, verify, and seal:

```text
phase0.manifest
phase0.meta
phase0.sha256
```

`phase0.manifest` uses the standard lowercase-hash/two-space/
repository-relative-path/LF format and covers `bundle.sha256`, every campaign
`campaign.sha256`, every attempt `attempt.sha256`, every retained
label-specific report/projection/comparison, and the attestation records for
the original/new evidence aggregates. It excludes itself, `phase0.meta`, and
`phase0.sha256` to avoid self-reference. `phase0.meta` is sorted `key=value`,
contains the exact five safety booleans, and covers:

- bundle identity;
- attestation, source, confidence, baseline, and replay campaign manifests;
- externally attested historical Goal G, prior Goal G-R, and completed
  Amendment 6 evidence aggregates;
- current inventories;
- new baseline values and old-to-new bridge results; and
- exact `G3` identity.

`phase0.sha256` has exactly two standard hash rows for `phase0.manifest` and
`phase0.meta`. All three become mode `400`; `<G3>` and the evidence root
become mode `500`. The final whole-root file stream is the SHA-256 of a
bytewise-sorted standard hash stream over every regular file; there is no
exclusion because this whole-root digest is recorded outside the root. The
final type/mode/size/path/link inventory uses the exact NUL record schema
defined for the bundle and includes the evidence root. Both hashes and counts
are recorded in the tracked Phase 0 handoff after the root is sealed.
No artifact inside the A3 root claims or embeds that root's final digest.

No mutable selector exists. The replay row's valid-green, valid-red, or
invalid state is immutable in its attempt and campaign records. A successful
command without a successful validator never passes.
