# Goal G Amendment 4: Storage Reset After Amendment 3 Preactivation Stop

Status: authorized for execution

Authorization date: 2026-07-30

Scope: documentation and activation-lineage reset only

## Purpose

This amendment permits Goal G Amendment 3 to resume after its mandatory
storage preflight stopped activation before any official Amendment 3 bundle,
runtime, evidence, tracked edit, staging operation, or activation commit was
created.

For only the two matters named here—returning from the recorded preactivation
storage stop and the resulting activation-parent/direct-child lineage—this
later user authorization has precedence over the conflicting clauses in the
Goal G return-sequence prompt, Amendment 3, and the Amendment 3 runner
contract. All other requirements in those documents remain controlling,
including their product scope, safety boundaries, retained no-Cargo bootstrap
checks, two independent static reviews, bundle layout and sealing rules,
evidence requirements, runtime-absence gates, and no-retry rules.

The return-sequence prompt's Stage R6 is already complete at exact `R6` and
must not be rerun. Its Stage G3 begins only from clean `S4`.

## Recorded preactivation stop

The stopped activation was inspected at the following immutable repository
state:

- Goal G-R closeout commit (`R6`):
  `fc1ceba88fc91bc5c55d34fb639a4b575e584844`
- `R6` tree:
  `6a198862a26c210ab1af68f5133a2f935fd4e6bb`
- `R6` parent (`A`):
  `5aaa6c622f0880d6f5ff473f1674cb1f7418cf1f`
- required available bytes: `2147483648`
- observed available bytes: `1035091968`
- shortfall: `1112391680`
- worktree state: clean
- official Amendment 3 recorder bundle:
  `target/tmp/goal-g-amendment-3-recorder-bundle` — absent
- official Amendment 3 Phase 0 evidence:
  `target/tmp/goal-g-phase0-amendment-3` — absent
- official Amendment 3 runtime:
  `target/tmp/goal-g-amendment-3-runtime` — absent
- official bundle state: `absent-not-created`
- tracked edits made by the stopped activation: none
- staging/index edits made by the stopped activation: none
- commits made by the stopped activation: none
- pushes made by the stopped activation: none

The stop was therefore a preactivation resource stop, not a failed or partial
Amendment 3 attempt.

## Authorized cleanup and result

The user authorized removal of exactly these disposable Rust build caches:

- `/home/ubuntu/app/predarb/target`
- `/home/ubuntu/app/predarb-flatness-fix/target`

Both paths were removed and verified absent. After removal, available storage
was `11308576768` bytes.

The cleanup did not authorize or modify source, captures, retained/non-cache
artifacts, credentials, historical Goal G evidence, or any other path.

## Non-authoritative development material

Files under `/tmp/reap-g3-draft` are non-authoritative development previews.
They are not an official Amendment 3 bundle, attempt, runtime, or evidence
record. They may inform fresh construction, but the resulting official bundle
bytes must then pass the complete retained no-Cargo self-test and independent
review process required by Amendment 3 before sealing and activation. Drafts
must not be blindly copied, relabeled, or treated as sealed evidence.

## Revised immutable lineage

The authorized lineage is now:

```text
A -> R6 -> S4 -> G3 -> P0
```

Where:

- `A` is `5aaa6c622f0880d6f5ff473f1674cb1f7418cf1f`.
- `R6` is `fc1ceba88fc91bc5c55d34fb639a4b575e584844`.
- `S4` is this amendment commit.
- `G3` is the Amendment 3 activation commit.
- `P0` is the later Phase 0 result commit, if Amendment 3 authorizes it.

The following lineage rules are mandatory:

1. `S4` must be the direct child of `R6`, with no intervening commit.
2. `G3` must be the direct child of `S4`, with no intervening commit.
3. `G3` must use the exact subject
   `docs: activate goal g amendment 3`.
4. `G3` may modify only the Goal G handoff document.
5. `P0`, if authorized, must be the direct child of `G3`.
6. Any mismatch is a stop condition; it must not be repaired by retrying or
   rewriting an official attempt.

`S4`, `G3`, any activation-stop commit, and any later official lineage commit
must not be amended, rebased, reset, replaced, or otherwise rewritten.
Official bundle or evidence bytes must not be replaced or rewritten outside
the exact lifecycle already authorized by Amendment 3. Recovery from any such
condition requires a new reviewed, user-authorized amendment.

If recorder construction, self-test, review, sealing, or activation fails
after `S4`, the existing Amendment 3 activation-stop contract remains exact
except that its documentation-only stop commit must be the direct child of
`S4`, not `R6`. Such a stop commit ends this lineage; it does not permit a
later `G3` without new user authorization.

If a post-`S4` preactivation gate fails before the official bundle is
created, keep the bundle, evidence, and runtime paths absent and record
`bundle_state=absent-not-created`. When the storage gate permits, update only
the Goal G handoff and commit the terminal record as the direct child of `S4`
with exact subject:

```text
docs: record goal g amendment 3 activation stop
```

`absent-not-created` is valid only when the bundle never existed. If bundle
creation started, preserve and report Amendment 3's existing
`partial-unsealed` or `sealed` state instead.

If the storage failure itself prevents that handoff edit or commit, make no
further mutation. That physical no-write exception and every committed
activation stop require new user authorization before any later `G3`.

## S4 commit contract

The `S4` commit:

- must use the exact subject
  `docs: authorize goal g amendment 3 storage reset`;
- may modify only:
  - `docs/polymarket-authenticated-execution-goal-g-amendment-4.md`
  - `docs/polymarket-authenticated-execution-goal-g-handoff.md`;
- must contain documentation only;
- must not modify production code, tests, dependencies, credentials, runtime
  data, or prior evidence;
- must not grant production, authentication, credential, order-placement, or
  push authority.

## Resumption gates

The storage-gated operation lists in the Goal G return-sequence prompt,
Amendment 3, and its runner contract are cumulative; this amendment does not
narrow any of them. Immediately before every official bundle creation or
edit, ignored evidence creation or edit, redirected log, child command
(Cargo or otherwise), executable-bit or seal edit, tracked edit,
staging/index edit, and commit, run exactly:

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

If storage falls below that threshold again, execution must stop before the
gated operation. The one-time cleanup authority for the two paths listed
above has been exercised and is exhausted. This amendment grants no further
cleanup authority, including for those same paths.

After `S4` is committed, first re-authenticate that it is `HEAD`, its parent
is exact `R6`, its subject and two-path delta match this contract, and the
worktree is clean. Capture its exact commit, tree, parent, subject, Amendment
4 contract hash, and the hash of the `S4` version of the Goal G handoff.
The later `G3` handoff must record all six identities before activation is
committed.

Amendment 3 may then resume from fresh bundle construction. Its full no-Cargo
bootstrap suite and both independent static reviews must pass before the
bundle is sealed and before `G3` activation. The official evidence and
runtime paths must still be absent at activation, and all Amendment 3
sealing, inventory, provenance, failure-classification, and no-retry
requirements remain in force.

## Safety and non-claims

```text
production_order_entry_authorized=false
real_credentials_loaded=false
authenticated_external_request_sent=false
real_polygon_rpc_request_sent=false
real_order_submitted=false
historical_goal_g_attempt_relabelled=false
historical_goal_g_r_equivalence_claimed=false
push_authorized=false
```
