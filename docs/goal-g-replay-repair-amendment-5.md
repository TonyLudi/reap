# Goal G-R Amendment 5: Defect-Class Hardening

Status: **authorized separate hardening track**.

This amendment is authorized by the user's instruction on `2026-07-28` to
implement toward the retained second-agent repair summary. It starts from the
documented Goal G-R Phase 1 causal stop:

```text
06b8948c8b3a2982ba9898c5215abc17e4f95893 docs: record goal g-r phase 1 causal stop
```

## Meaning

Amendment 5 hardens the two proven evidence-harness defect classes without
claiming that it recovered the exact historical transition erased by the
original failure:

1. the real-writer workload must not treat a generic positive service count as
   proof that the expected durable acknowledgement prepared the matching fake
   effect; and
2. the raw-capture bounds test must not accept a terminal finish carrying a
   primary writer/shutdown error and then report only a secondary verifier
   error.

This is not Goal G-R Phase 2 or Phase 3, does not make Goal G-R complete, and
does not resume or amend Goal G. The original `goal_g_r_regression_contract`
remains `pending`.

The frozen Amendment 4 v5 runner cannot represent this post-stop contract:
its activation-prefix check requires the existing handoff bytes, including
the pending regression field, to remain unchanged, while its Phase 3 loader
requires that same field to occur exactly once with value `frozen`.
Amendment 5 therefore must not invoke, edit, copy, replace, or reinterpret v5,
and must not create the v5 regression-runner path.

## Tracked Authority

The authorization commit may change only:

```text
docs/goal-g-replay-repair-amendment-5.md
docs/goal-g-replay-repair-handoff.md
```

The implementation candidate may additionally change exactly:

```text
crates/reap-pm-live/src/evidence/workload.rs
crates/reap-pm-live/tests/combined_replay.rs
```

No other production source, test support, dependency, `Cargo.toml`,
`Cargo.lock`, fixture, schema, capacity, timeout, queue policy, report
projection, Goal G document, sibling repository, or ignored frozen evidence
path is authorized.

`workload.rs` remains a closed evidence driver. It may add exact
acknowledgement, prepared-effect, and failure observation plus diagnostics,
but it may not change product policy, production execution authority, or
report values.

## Frozen Regression Contract

The two exact Amendment 5 regression identities are:

```text
goal_g_r_amendment_5_regression_1_name=evidence::workload::tests::real_writer_acknowledgement_is_bound_to_expected_prepared_effect
goal_g_r_amendment_5_regression_1_target=lib
goal_g_r_amendment_5_regression_2_name=terminal_capture_finish_preserves_primary_shutdown_error_before_prefix_verification
goal_g_r_amendment_5_regression_2_target=combined_replay
```

Their exact commands are:

```bash
env CARGO_NET_OFFLINE=true \
  TMPDIR=target/tmp/goal-g-replay-repair-runtime \
  cargo test --locked -p reap-pm-live --lib \
    evidence::workload::tests::real_writer_acknowledgement_is_bound_to_expected_prepared_effect \
    -- --exact --test-threads=1 --nocapture

env CARGO_NET_OFFLINE=true \
  TMPDIR=target/tmp/goal-g-replay-repair-runtime \
  cargo test --locked -p reap-pm-live --test combined_replay \
    terminal_capture_finish_preserves_primary_shutdown_error_before_prefix_verification \
    -- --exact --test-threads=1 --nocapture
```

The capture regression may construct a typed shutdown failure solely to prove
that the test classifier surfaces it before a secondary verification result.
The constructed variant is not historical evidence and must not be described
as the original writer failure.

## Required Repair

### Acknowledgement identity

Around every sealed or real-writer acknowledgement, the evidence driver must
take immutable before/after cuts and prove:

- exactly one durable acknowledgement;
- exactly one owner reduction for that acknowledgement;
- exactly one prepared Quote for a Quote acknowledgement, exactly one
  prepared Cancel for a Cancel acknowledgement, and zero prepared fake
  effects for a fact acknowledgement;
- the prepared-stage product-effect kind and identity match the durable
  record being acknowledged;
- zero persistence durability, closed-writer, and age-fault deltas;
- zero mutation durability-failure and preparation-failure deltas; and
- diagnostics identify the cycle, expected acknowledgement kind, counter
  deltas, and primary failure class before fixture dispatch.

An injected failure remains an error. The hardening must not retry, wait
longer, turn a failed durability result into an acknowledgement, or execute a
fake effect after a missing or mismatched preparation.

### Capture terminal finish

The four terminal bounds subcases in
`raw_frame_and_raw_count_bounds_are_exact` must share one helper that accepts
only:

```text
TerminalFinish {
  cause: CaptureWriter,
  shutdown_error: None,
}
```

Any retained shutdown error must fail at that helper before
`verify_pm_public_capture` is called. The helper must not accept, suppress, or
reclassify a writer fault.

## Verification

Every Cargo command must use:

```text
CARGO_NET_OFFLINE=true
TMPDIR=target/tmp/goal-g-replay-repair-runtime
```

The fixed runtime directory must be empty before and after each command. Run,
without retrying a failed result on an unchanged revision:

1. `cargo fmt --all -- --check`;
2. each frozen regression exactly once;
3. each original affected test exactly once with `--exact`,
   `--test-threads=1`, and `--nocapture`:
   `phase6_real_mutation_artifacts_recover_to_the_same_bounded_projection`
   and `raw_frame_and_raw_count_bounds_are_exact`;
4. the full `combined_replay` integration binary once under default
   parallelism;
5. `cargo test --locked -p reap-pm-live --lib`;
6. `cargo test --locked -p reap-pm-live --all-targets`;
7. `cargo clippy --locked -p reap-pm-live --all-targets -- -D warnings`; and
8. `cargo test --locked -p reap-pm-live --test compile_fail_boundaries --
   --test-threads=1`.

Every required command must exit `0`. Each frozen exact regression command
must run exactly one test and pass; a zero-match, failure, or environment that
cannot execute a required command is a stop and prevents Amendment 5
completion. No result may be retried on the same revision to obtain a
preferred outcome.

The real-writer report must retain all frozen Goal F artifact and recovery
anchors. `Cargo.lock`, Goal G's four named artifacts, and both complete Goal G
tree streams must remain byte-identical.

## Storage And Safety

Immediately before every tracked edit, Cargo command, redirected validation
log, and commit, run the unchanged fail-safe:

```bash
(
  set -euo pipefail
  root=$(git rev-parse --show-toplevel)
  available_bytes=$(df --output=avail -B1 "$root" |
    awk 'NR == 2 {print $1}')
  [[ $available_bytes =~ ^[0-9]+$ ]]
  (( available_bytes >= 2147483648 ))
)
```

Insufficient storage is a stop, not cleanup authority.

Amendment 5 authorizes no credentials, network request, signing, Polygon
request, order placement/cancellation, allowance change, deployment, push,
sibling-repository change, cleanup, timeout increase, retry, reduced
workload, global serialization, or assertion weakening.

## Completion Meaning

Amendment 5 is complete only when the two-file hardening and its frozen
regressions are committed, the fixed verification set is honestly recorded,
the worktree is clean, and every frozen evidence/semantic anchor remains
exact.

Completion means only that future occurrences of these defect classes fail at
the primary evidence boundary with useful diagnostics. It does not prove
which exact primary transition caused the retained Goal G failures, does not
make Goal G-R green, and does not resume Goal G.
