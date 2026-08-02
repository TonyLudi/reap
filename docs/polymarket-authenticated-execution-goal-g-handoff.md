# Polymarket Authenticated Execution Goal G Handoff

Status: **the historical amended Phase 0 replay remains stopped under
user-authorized Amendments 1 and 2; Goal G-R Amendment 6 completed at `R6`;
Amendment 4 committed `S4`; Amendment 3's failed activation lineage remains
terminally preserved; Amendment 5 is authorized but inactive; neither `G3`
nor `P0` has been created**. The historical clean replay
selected immutable attempt 1 as valid evidence with `gate_pass=false`: 11 of
13 tests passed and two existing Goal F semantic/non-timing tests failed. The
amended contract forbids discarding or retrying that valid red result, so
Phase 0 is not green and no Phase 1 implementation began. The source byte
re-attestation and all 16 baseline benchmark invocations were green before
the replay stop.

The historical stopped run correctly proved that an authenticated CLOB
`CONDITIONAL` numeric value cannot become the boolean ERC-1155 operator
approval required by Reap's typed core. Amendment 1 does not reinterpret it:
it adds a separate closed finalized-block Polygon source, authorizes a strict
source-tagged lifecycle/time compatibility union, and replaces the
host-specific PM latency ceiling with a paired local relative gate. Amendment
2 incorporates the restarted-run route/lifecycle audit, including a distinct
REST-only matched-before-broadcast settlement state, single-lane requirement
IDs, explicit HTTP no-retry construction, and exact source/vector/dependency
tables.

The reviewed design candidate is
[polymarket-authenticated-execution-boundary.md](polymarket-authenticated-execution-boundary.md).
It is now the amended runnable implementation boundary. The evidence below
remains the immutable history explaining why the amendment was necessary.

## Safety Attestation

```text
production_order_entry_authorized: false
real_credentials_loaded: false
authenticated_external_request_sent: false
real_polygon_rpc_request_sent: false
real_order_submitted: false
```

No real secret, authenticated request, Polygon RPC request, allowance update,
order, cancel, or external mutation was used. Only public documentation,
public official Git/dependency/package source, the local public Node/viem
vector oracle, owner-local repository tests, and local benchmarks were read
or run.

## Phase Status

| Phase | Status | Commit |
| --- | --- | --- |
| 0. Baseline, protocol freeze, threat model | Stopped; source re-attestation and 16-invocation baseline green; clean replay valid red at 11/13; phase gate not green | Pre-gate contract `66a6213301f9c9677f8137f545c11cfc0ff3c065`; supporting policy `facd3a616fc20e7bc1abc627235588b7532ff8b1`; stop record committed separately |
| 1. Backend-neutral prepared effects | Not started | None |
| 2. Secret custody/auth/signing | Not started | None |
| 3. Public/chain/authenticated read-only transports | Not started | None |
| 4. Authenticated journal, then place/cancel | Not started | None |
| 5. Product/recovery/shutdown | Not started | None |
| 6. Fault/security/performance evidence | Not started | None |
| 7. Global verification/handoff | Not started | None |

The original prompt required Phase 0 to map the CLOB response into numeric
`Erc20Allowance` versus boolean `Erc1155OperatorApproval`; stopping was the
only safe result. The amended prompt removes that invalid requirement and
instead obtains both typed facts directly from the closed chain cut. The
historical Phase 0 result is not green and is not retroactively relabeled.
The amended run restarted Phase 0, completed the benchmark-policy tranche and
Amendment 2 source/contract freeze, passed source and benchmark gates, and
then stopped at the replay gate. It cannot continue under the current frozen
campaign.

## User-Authorized Amendment 1 — 2026-07-27

The amendment freezes these decisions:

1. `PM-LIVE-ACCOUNT-CUT` retains CLOB collateral/token balances and numeric
   per-selected-spender cache/spendability evidence only.
2. `PM-LIVE-POLYGON-AUTHORIZATION-CUT`, owned by the new
   `reap-polymarket-chain-source`, obtains direct ERC-20
   `allowance(configuredEoa, selectedExchange)` and ERC-1155
   `isApprovedForAll(configuredEoa, selectedExchange)` at one
   provider-reported finalized Polygon block. Chain, contracts, owner,
   selected standard/negative-risk exchange, ABI, block/freshness, and result
   types are closed; arbitrary RPC and every mutation are impossible.
3. Every chain cut starts with `eth_chainId == 0x89`, then a finalized block
   anchor, two exact `eth_call`s at its explicit block number, and an
   exact-number block-hash recheck. The block may be at most five seconds
   future/thirty seconds old, and the complete cut expires after five
   monotonic seconds or an epoch change. Any partial, malformed, stale,
   reverted, wrong-chain, or hash-changing sequence is discarded as a unit.
4. CLOB numeric values never become boolean approval. The CLOB, Polygon, and
   Data API replies are non-atomic and join only through matching
   account/market/configuration epochs and independent freshness.
   An unproved CLOB conditional comparison unit leaves the bounded numeric
   value diagnostic-only and does not stop the goal.
5. Inbound lifecycle/time parsing is a closed source/message-family-tagged
   union. Exact raw provenance is retained; only enumerated semantic
   equivalences normalize. Unknown, ambiguous, cross-family, malformed, and
   out-of-profile input is quarantined and fails closed, never promoted to
   success.
6. The PM action benchmark's absolute `25,000 ns` p50 and `250,000 ns` p99.9
   exits are superseded. The completed policy tranche changed only the exact
   latency branch in `src/evidence/runner.rs`, the bench validator, and their
   policy tests, not the workload/report schema/non-timing gates; Goal G does
   not edit it again. Each side retains three Cargo
   invocation reports after one process-warmup invocation; each retained
   invocation already has three internal recorded distributions. Compare the
   median of each invocation's internal three, then the median of those three
   invocation medians. Phase 0/final p50 and p95 compare at `≤1.10×`, p99 at
   `≤1.20×`; p99.9/max are retained but not shared-host gates. Every
   logical/hash/allocation/memory/cardinality/queue gate remains exact.
7. Goal G chooses no production Polygon provider/origin or provider
   credential, exposes only the constrained non-default `local-evidence`
   loopback construction, and sends no real chain request. Goal H must inject
   one exact HTTPS origin and prove chain/finality/history/bounds/provider-
   credential and target-host clock assumptions before construction.
8. The distinct authenticated journal schema/lease/durable barriers/recovery
   projection land as the first Phase 4 tranche before any live place/cancel
   role. Phase 5 only composes that proven foundation.

The amendment design was checked against these primary official contracts.
The restarted Phase 0 retrieved, pinned, hashed, and successfully re-attested
the exact bytes/revisions recorded by the 128-row manifest; the gate did not
substitute moving replacements:

| Source | Amendment contract |
| --- | --- |
| `https://ethereum.org/developers/docs/apis/json-rpc/` | `eth_chainId`, provider-reported `finalized` block tag, exact-number `eth_getBlockByNumber`, and read-only `eth_call` |
| `https://docs.polygon.technology/pos/concepts/finality/finality` | Polygon milestone-finalized block semantics |
| `https://docs.polygon.technology/pos/reference/rpc-endpoints` | Polygon mainnet chain ID `137`; no provider selected |
| `https://eips.ethereum.org/EIPS/eip-20` | `allowance(owner, spender) -> uint256` |
| `https://eips.ethereum.org/EIPS/eip-1155` | `isApprovedForAll(owner, operator) -> bool` |
| `https://docs.soliditylang.org/en/latest/abi-spec.html` | selector, address-word, `uint256`, and canonical boolean ABI encoding |
| `https://docs.polymarket.com/resources/contracts` | Current Polygon pUSD, Conditional Tokens, standard exchange, and negative-risk exchange addresses |

These decisions are normative in the amended
[execution prompt](polymarket-authenticated-execution-goal-g-prompt.md) and
[boundary](polymarket-authenticated-execution-boundary.md). The entire
“Historical Stopped-Run Evidence” section below remains evidence, not active
instructions.

## User-Authorized Amendment 2 And Restarted Phase 0 — 2026-07-27

The restarted run began from clean `master` at
`facd3a616fc20e7bc1abc627235588b7532ff8b1`, two commits ahead of the
locally recorded `origin/master` tracking ref; no fetch was performed.
Required baseline
`43970849267c0282d118a369a792066c4655deae` and Goal F tree
`d16c3cbdac97fb43944e3a97d4f9b56e92206747` are ancestors. `Cargo.lock`
still hashes to
`2673d055c943c3bd5444531b67df280026c145cbbbc99b68a06f4ac0c2dbb0ff`.
Rust/Cargo are 1.95.0 on Linux aarch64 with two CPUs.

`../imm-strategy` remains clean at
`b6b120c7b7c466d8431bf082f3229328c5d7b2ae`. Predarb's pinned object
`8222273a9c72033b760e1d2fec813bc77144556d` remains available. Only its
dirty path names were observed: modified
`resources/grafana/pf-maker-v2-dashboard.json` and untracked `.predarb/`.
Neither path nor any dirty/runtime/secret byte was opened.

The authorized benchmark-policy tranche is the focused commit:

```text
facd3a616fc20e7bc1abc627235588b7532ff8b1 bench: use relative pm action latency policy
```

It removed only the two obsolete PM absolute-latency exits, preserved the
15,000-sample and all logical/hash/allocation/memory/cardinality/queue gates,
and passed:

```text
cargo fmt --all -- --check
cargo test -p reap-pm-live --test phase6_evidence_policy --locked
cargo bench -p reap-pm-live --bench pm_action_path --locked
```

The precommit benchmark log is diagnostic only: it reports build revision
`205ecca...` while exercising the later-committed diff, so it is not the
Phase 0 comparator.

Fresh public official captures and Git-source review found one additional
normal state: a credential-visible account trade may be
`MATCHED_NOT_BROADCASTED`, meaning the orders matched before an on-chain
transaction was broadcast. Current official Python SDK source states that
the value presently appears on account trade listings, not user-stream trade
events. Amendment 2 therefore:

1. adds a distinct nonterminal canonical `MatchedNotBroadcast` live fact;
2. accepts its prefixed/unprefixed compatibility spellings only on the
   account-trade REST family;
3. quarantines either spelling on raw user WS and repairs through the complete
   REST cut;
4. keeps POST/order/trade `MATCHED` namespaces distinct;
5. freezes every reached timestamp form per field rather than guessing by
   magnitude;
6. gives every stable requirement ID one lane and adds only the three
   responsibility-split dispatch/reconciliation child IDs;
7. explicitly disables reqwest retry, redirect, and ambient proxy behavior;
   and
8. freezes an acyclic five-new-crate edge/journal shape with only exact
   `k256 =0.13.4` and `sha3 =0.10.9` external additions.

The final implementability pass also froze details that may not be chosen
during coding:

- canonical padded base64url credential/output, exact application HTTP
  headers, and strict no-content-encoding behavior;
- one byte-once POST/cancel request contract and an exhaustive mutation
  HTTP/body/cross-field classification union;
- canonical increasing base64 offset cursors and exact one-pass query
  encoding;
- a Reap-local `/time` projection with exact order/L2 pre-write checks, no
  invented timestamp high-water, and a five-second geoblock permit;
- all five compact Polygon JSON-RPC request templates and result shapes;
- the raw user-order occurrence/status/quantity compatibility table;
- literal deterministic PM/OKX initial frames; and
- the exact Phase 0 command/log/overlap/comparator and byte-revalidation
  procedures.

The current V2 OpenAPI, unified TypeScript SDK, and current Rust client agree
that the signed/wire order has no `taker`, `nonce`, or `feeRateBps`. The older
`clob-client-v2` extra `taker` is rejected. Current type-0 EOA source proves
the fixed `maker == signer == funder == POLY_ADDRESS` profile and outer owner
as the L2 key UUID. No current auth/signing/body/identity/route stop remains.

The authoritative restarted source manifest has 128 rows and SHA-256
`f38625a6f2bb0a2c8e13598acf6ab7dc1eccc57f97a7f4a8c45fdb810e8fcb4d`;
its exact official revisions, critical blob/content hashes, dependency pins,
and full vector values are recorded in the boundary. The independently
authored vector candidate covers
standard/negative-risk BUY/SELL domain separator, struct hash, digest/order
ID, recoverable signature, exact POST/cancel bytes, GET query exclusion, and
L2 HMAC. Phase 2 must check in immutable literals and reproduce them with the
narrow Rust implementation; Node/viem remains evidence-only.

### Restarted inventory cutoff

| Inventory | Current result |
| --- | --- |
| Workspace packages / normal edges | 35 / 102 |
| Outside-workspace Cargo path dependencies | 0 |
| Adjacency / edge-list SHA-256 | `63cca672bf23690d042779967d2cb2c12414633924b20d296a269f2e63554c06` / `a798f2c320e364782d0dbb3b0d0cc4ffc248896adad3ee502a9ec9e87d59c28e` |
| Public declarations | 2,671; `0bb94c4dce0e896ce08e30d4fb3d4380e59a00e64a4ceaa04997d200f86bf1fc` |
| Schema declarations | 48; `e71cc793d56e5ba63199b6c341c97e04ac1ca18e8f68adf3742083de434a3d25` |
| Production Rust paths | 366; `2a091f16d8e8107bd61f6529fb785581c2e6cd43a509feb9f248d0b78c6a2ee6` |
| Crate-root public declarations | 143; `36fbdc52d7e688557a50aee1055216cba388450215ac04acc4c5ce9aff7ad673` |
| Current production-content manifest | `b17bc622c7c1cb09139deeb1ddc9509b87b88a70857dc383d666cdfc86e7e648` |
| Goal F fixture manifest/provenance | `765b5f2229215871c9bdc2c941601de5968a4e48633873b10c5d12d96a091306` / `cdd669d67fa10457dc0b2e5b832572c66616ef5b6f08634c6d9ca95c8b4a435e` |

The current production extent is 366 files and 172,503 lines. Existing
legacy files over 1,500 lines are unchanged. Goal G may not grow current
near-cap files `capture_roles.rs` (1,490),
`coordinator/mutation.rs` (1,466), `private_monitor.rs` (1,447), or
`public_session.rs` (1,440); they must be split before relevant growth.

The filesystem is 100% reported full with `313,618,432` bytes available after
the replay stop. The amended helpers require `268,435,456` bytes before each
Phase 0 execution; Phase 1 and every later build/global gate require
`2,147,483,648` bytes. Phase 1 therefore has an independent storage stop even
if the replay defect is later resolved. Falling below either threshold is a
documented stop, not permission to remove user/sibling data, `target/tmp`, or
valid or invalid evidence. Additional storage or explicit approval for a
retained-evidence-preserving build-cache cleanup is required before a later
implementation campaign.

### Amended Phase 0 Gate Results And Stop

The Amendment 2 documentation-only pre-gate commit is:

```text
66a6213301f9c9677f8137f545c11cfc0ff3c065 docs: freeze goal g amendment 2 contract
```

It preserved the clean tree and frozen `Cargo.lock` SHA-256
`2673d055c943c3bd5444531b67df280026c145cbbbc99b68a06f4ac0c2dbb0ff`.
The ignored evidence root is
`target/tmp/goal-g-phase0-amended`; it must remain intact.

The exact source re-attestation passed:

```text
verified retained document bodies: 33
verified retained Git bodies: 60
verified pinned addendum blobs by credential-free fetch: 28
verified cached crate archives: 3
verified authoritative source manifest: 128 rows
```

The re-attestation log SHA-256 is
`649a510599c591963c37dd1aaea579b9eefa2aba641a4dd155c5e70e21a4d9be`;
the checksum-file SHA-256 is
`e408e41fe6130d8ea1e3db739d196a83b5c8ad486fd3af6b2a2af37a37881709`.

All engine, live, Chaos action, and PM action warm-up plus three retained
invocations selected attempt 1: 16 of 16 had `evidence_valid=true` and
`gate_pass=true`, with no process overlap. The retained medians were:

| Baseline | Retained median |
| --- | ---: |
| Engine | 11,676.6 ns/event |
| Live wire / feed / coordinator / parity | 2,927.8 / 7,782.1 / 4,072.4 / 17,351.7 ns/unit |
| PM p50 / p95 / p99 / p99.9 / max | 23,565 / 45,021 / 57,418 / 78,546 / 176,300 ns |

`baseline-summary.json` retains the ten Chaos action workload latency
medians. The selected raw reports/logs retain the exact counters, allocations,
hashes, cardinalities, queue gates, and all other non-timing evidence; their
paths and hashes are covered by `baseline-campaign.sha256`. SHA-256 values
are:

| Baseline artifact | SHA-256 |
| --- | --- |
| `baseline-campaign.sha256` | `009a2faeaf2e6c777c3959d4cd92607f095036b42dcba2b90ea45a428b047a79` |
| `baseline-summary.json` | `3de85bbd7145d6692cc383ea60783f072af243981c09902d26aac9c1668929e6` |
| `pm-retained.json` | `8384d7637819107bae1bacabd580c09a19c185f2416f5f1e5bf6ff2d0741bac5` |
| `summarizer.log.sha256` | `f0619cf256ef8cc856db2c80b9f5184d1d8f8321eb9b86ef63ad73f9d2a95c96` |

The clean replay began at `2026-07-27T07:20:37Z` and ended at
`2026-07-27T07:21:26Z`. Its pre/post HEAD, tree, status, and `Cargo.lock`
were identical. Attempt 1 exited 101 and was selected with:

```text
schema=goal-g-phase0-replay-v2
evidence_valid=true
gate_pass=false
reason=combined-replay command exited 101
```

The test result was 11 passed and two failed:

- `phase6_real_mutation_artifacts_recover_to_the_same_bounded_projection`
  failed in its isolated recovery subprocess with
  `Invariant("PM fake-effect script does not match the next prepared effect")`;
- `raw_frame_and_raw_count_bounds_are_exact` failed when the capture verifier
  returned `InvalidRecords`.

The command stopped before its decision/backtest subsections and emitted no
new combined-replay report. The selector SHA-256 is
`4168ac456d70361429967d7457e0d5850cd014c0b0ea7b8e45e3183372ec766d`;
the combined log, metadata, and process-snapshot SHA-256 values are,
respectively,
`fe3e8c7323c52163345e6330ebd7587858990a49d1bc436a1a669792f6473cd9`,
`b2dc689182ea8c02fd340669b2b0f142b6cafd15d5ec38a04cda221f3aaa8f56`,
and
`fd77e0c1db9970bbe2c20eea70dc8836091a81e77d9bd66491c4d8150f4bf0c3`.

A read-only comparison from the known passing Goal F commit
`d16c3cbdac97fb43944e3a97d4f9b56e92206747` to the replayed HEAD found only
the three authorized PM latency-policy paths changed under
`crates/reap-pm-live`: `benches/pm_action_path.rs`,
`src/evidence/runner.rs`, and `tests/phase6_evidence_policy.rs`.
`combined_replay.rs`, its capture verifier, and coordinator mutation logic are
byte-unchanged. This supports, but does not prove, a pre-existing test
isolation or durability race exposed by the clean run; it is not evidence of
a Goal G semantic implementation regression.

The clean nonzero replay is valid red evidence. The frozen selector and
evidence must not be deleted, replaced, or retried. The smallest separately
scoped next goal is the proposed
[Goal G-R combined-replay repair](goal-g-replay-repair-prompt.md). It permits
only a causally proven repair within the closed evidence-harness/test
allowlist and stops if live-product or other source changes are required. It
must preserve this red campaign and Goal F's frozen artifact, semantic, and
non-timing hashes. At proposal time the repository filesystem had
`310,030,336` bytes available. Goal G-R avoids the undersized `/tmp` tmpfs
with a private repository-filesystem runtime directory and requires at least
`2,147,483,648` repository bytes before any edit or execution, so it is not
currently runnable. A reviewed Amendment 3 with a new evidence root is
required before Goal G can attempt a new replay; adequate Phase 1 storage is
separately required before implementation.

No production endpoint, authenticated call, real Polygon call, credential,
or order was used by these gates.

## Historical Stopped-Run Evidence — 2026-07-26 To 2026-07-27

Everything in this section through “Predarb Lessons And Rejections” preserves
the pre-amendment stopped run. Its HEAD/remote state, source cutoff, absolute
PM latency thresholds, red gates, and stop conclusions are historical only
and do not override the current Amendment 2 section above.

### Baseline And Workspace Identity

The baseline inventory was taken on 2026-07-26 UTC. The current date at this
stop record is 2026-07-27 UTC.

| Check | Result |
| --- | --- |
| Reap `HEAD` / branch | `43970849267c0282d118a369a792066c4655deae` on `master` |
| Remote relation | `HEAD == origin/master`; zero ahead, zero behind |
| Initial worktree | Only untracked `docs/polymarket-authenticated-execution-goal-g-prompt.md` |
| Required Goal G baseline | `43970849267c0282d118a369a792066c4655deae` is an ancestor |
| Goal F final tree | `d16c3cbdac97fb43944e3a97d4f9b56e92206747` is an ancestor |
| `Cargo.lock` SHA-256 | `2673d055c943c3bd5444531b67df280026c145cbbbc99b68a06f4ac0c2dbb0ff` |
| Existing Chaos reference | `../imm-strategy` clean at `b6b120c7b7c466d8431bf082f3229328c5d7b2ae` |
| Historical PM reference | Predarb commit object `8222273a9c72033b760e1d2fec813bc77144556d` is available and is checkout `HEAD` |
| Predarb dirty path names | Modified `resources/grafana/pf-maker-v2-dashboard.json`; untracked `.predarb/` |
| Worktree/index safety | One Reap worktree; no `index.lock`; neither sibling changed |

Predarb was inspected only through pinned Git-object commands (`git show`,
`git grep`, `git ls-tree`, `git cat-file`, and an isolated archive of that
object). Neither dirty path was opened. No `.env`, `.env_bk`, runtime state,
dashboard contents, private key, credential, tracked operational value, or
other untracked byte was read, moved, interpreted, reset, cleaned, or copied.

At the initial process check there was no overlapping Cargo/Rust process.
During the benchmark campaign, another Reap session later started two
CPU-saturating `combined_replay` processes on this two-vCPU host. Those
processes made the first PM benchmark attempts invalid as idle-host evidence.
They did not change the worktree. The overlap was allowed to finish; it was
not stopped or modified. A later no-overlap PM warm-up still failed the
frozen p50 limit, as recorded below.

The root filesystem was 99% used with approximately 633 MiB free. Goal G's
public-source captures and raw benchmark logs use approximately 22 MiB under
ignored `target/tmp/goal-g-phase0`; no user or sibling data was cleaned.

### Repository Inventory

| Inventory | Baseline result |
| --- | --- |
| Workspace packages | 35 |
| Outside-workspace Cargo path dependencies | 0 |
| Canonical 35-row normal adjacency SHA-256 | `63cca672bf23690d042779967d2cb2c12414633924b20d296a269f2e63554c06` |
| 102 individual normal workspace edges SHA-256 | `a798f2c320e364782d0dbb3b0d0cc4ffc248896adad3ee502a9ec9e87d59c28e` |
| 22 Goal F edges SHA-256 | `8944276c9572f6cb00b7db0830e2189199e979eba57d7189973ea62fc281b29f` |
| Public declarations | 2,671 lines; SHA-256 `0bb94c4dce0e896ce08e30d4fb3d4380e59a00e64a4ceaa04997d200f86bf1fc` |
| Schema/version inventory | 48 lines; SHA-256 `e71cc793d56e5ba63199b6c341c97e04ac1ca18e8f68adf3742083de434a3d25` |
| Production Rust paths | 366; SHA-256 `2a091f16d8e8107bd61f6529fb785581c2e6cd43a509feb9f248d0b78c6a2ee6` |
| Production content manifest SHA-256 | `ebdec7ad2706d6753ed8ea76df4f04838b8fd167ebc890e064de66577cfbc632` |
| Production extent stream SHA-256 | `878ee3d325f439891e5cbd0188e5365246e7f3b0b117ebd314fa1bfe6c1847c0` |
| Public wire fixtures | Manifest schema 1; 36 payloads |
| Fixture manifest / provenance SHA-256 | `765b5f2229215871c9bdc2c941601de5968a4e48633873b10c5d12d96a091306` / `cdd669d67fa10457dc0b2e5b832572c66616ef5b6f08634c6d9ca95c8b4a435e` |
| PM capture/journal/report schemas | PM public capture v1; `reap-pm-mutation-journal` v1; action and combined-replay schema 1 |
| Goal F capability surface | All 16 IDs unchanged, including `PM-FAKE-PLACE-GTC-PO` and `PM-FAKE-CANCEL-OWNED` |

The current public PM source implementation is still in
`reap-polymarket-adapter`; `reap-polymarket-wire` is pure bounded parsing and
fixture DTOs; `reap-okx-public-source` has the strict index
reference/session/subscription state but no endpoint-connected transport;
`reap-pm-live` exposes the fixture product, prepared fake effects, read-only
private monitor, and v1 journal. That is the exact mechanical extraction and
backend-neutral seam a resumed Goal G would change.

The existing structural guard families remain:

- `reap-pm-core/tests/dependency_policy.rs`;
- `reap-pm-state/tests/source_policy.rs`;
- `reap-pm-strategy/tests/source_policy.rs`;
- `reap-polymarket-wire/tests/dependency_policy.rs`;
- `reap-polymarket-adapter/tests/private_source_policy.rs`;
- `reap-okx-public-source/tests/source_policy.rs`; and
- `reap-pm-live/tests/dependency_policy.rs`.

Current compile-fail case counts are 12 PM-core, 3 live-contracts, 1 strategy,
3 wire, 9 adapter, and 41 PM-live. The existing
`product_has_no_live_mutation_authority` guard may later be replaced only by
stronger backend-mixing and authority guards; it may not simply be deleted.

#### File And Function Stops

These production files are already at or above 1,400 lines and cannot grow
under Goal G before a responsibility split:

| File | Lines |
| --- | ---: |
| `capture_roles.rs` | 1,490 |
| `coordinator/mutation.rs` | 1,466 |
| `private_monitor.rs` | 1,447 |
| `reap-polymarket-adapter/public_session.rs` | 1,440 |

`reap-pm-state/private.rs` is next at 1,395 lines. The largest production
functions are `enact_okx_reference_lane_failure` (240 lines) and
`enact_pm_lane_failure` (235 lines) in `run_lane_full.rs`; both already
require decomposition review and are below the 250-line hard stop.

### Focused Baseline Verification

| Command | Result |
| --- | --- |
| `cargo test -p reap-pm-live-contracts --test plan_contract --locked` | 10 passed |
| `cargo test -p reap-polymarket-wire --test fixture_provenance --locked` | 2 passed |
| `cargo test -p reap-pm-live --test dependency_policy --locked` | 13 passed |
| `cargo test -p reap-pm-live --test combined_replay --locked` | Serial idle rerun: 13 passed |

One earlier combined-replay attempt overlapped another benchmark and failed
one durability timing point. It is retained as a contaminated result, not
acceptance evidence. The idle rerun produced the frozen Goal F evidence:

| Evidence | Value |
| --- | --- |
| Combined writer | 35,012 lines; 22,791,589 bytes |
| Writer SHA-256 | `83ced509c9ea180e66d957853f9ff7762ef3c0babc316c9251c12d4d1a5224eb` |
| Canonical recovery SHA-256 | `f98bf8a88f34fb6e3c4dcfd1919a2c1d4577b2da3960375e216e596d0746cd35` |
| Peak bounded bytes | 2,959,343 |
| Production order entry | false |

The frozen canonical Chaos backtest output remains
`38acf9f5e0c310f2ec5528974beffadf4c1a7f84d46efa8d9664ee7051e84691`.
Phase 0 stopped before claiming a new two-run global backtest campaign.
Existing Goal D decision anchors remain:

| Artifact | SHA-256 |
| --- | --- |
| `risk_initialization_v1.json` | `7e0951c41f447b9f46a73b24a3fe85bdc8f2bb8a623385dab0c3655926e73780` |
| `replay_events_v1.jsonl` | `dede17a546d4d717c78dc2b3b7aa7c3f3f785d552404160407c78fb87cec9101` |
| `expected_engine_v1.jsonl` | `140c268619b889a19d779e1bdfd340c11901d2eb1d9e4d216d976ba3d8b0d37a` |
| `expected_live_reduction_v1.json` | `aa66cc09bba29cde25ab2df66c018517b2c900f83373f95580150e8bcd442b60` |

### Benchmark Baseline And Red Gate

Raw logs are retained under `target/tmp/goal-g-phase0`. Every command was a
separate process. The engine, live, and Chaos action campaigns completed one
warm-up and three runs:

| Target | Raw log SHA-256: warm-up / run 1 / run 2 / run 3 | Stable logical counts |
| --- | --- | --- |
| `reap-engine/event_loop` | `51556d6f27ffb8d7c125f5765458c772af17ca70a1c236aed7d12406f4b84efd` / `3ca222b7d327be83f2607f8969e6b3f0c3c867b8db8f28d316b1ddef543910d9` / `7bad542f3251be6e5c775d2bd5563c07d07f4ed6c0c8ce452ea1594669e58f0b` / `7911b4f1ce2316c37d250a02795d28945bb9fd3ae5b23435c863082a781da322` | 250,000 events; 999,996 intents |
| `reap-live/live_loop` | `5f4580444720bac739428ac7b8935e3589428e76ad3f241e6ce6897dbf7c6efa` / `49dc601dca46d2ad12552fe20d85190428ac05fb1331dd32df22e618b12f10a8` / `d6fa9c7ef47eaa85e75b902635a8909dc83620c8169e6bd1495979af12ca6ef1` / `4e386b07237d160eb7d881ed2c0bed35ed5a279890febd0e3084b774a7dde69f` | 50,204 parsed; 70,208 feed outputs; 65,130 records; zero actions |
| `reap-live/action_path` | `8956217dcd1e9946aaf6557dfbd98f827edc513a17f96460b3195335e4375513` / `35899810805ef52ad58cbec423e7add896969d9f4d57fb8bc17c19f2cea64ba4` / `001c38f8e8e0c4b148ba5b6bd3d0f9bd3c98418d8cef3adb7f2f970607cce8f4` / `89ab50e49a26f46820e64e64df813fa26045e85c3e019337c2c4ffabaa57fa5b` | 100,000 observations per workload; exact workload counters and allocation pass matched |

The action-path raw JSON reports p50, p95, p99, p99.9, max, exact logical
counters, allocation calls/bytes, queue capacity/high water/saturation, and
queue age for every workload. These logs are the retained evidence; no
percentile was inferred from a smaller sample.

The required PM campaign could not pass its warm-up binary:

| Attempt | Overlap state | p50 / p95 / p99 / p99.9 / max (ns) | Failed contract | Raw log SHA-256 |
| --- | --- | --- | --- | --- |
| Initial warm-up | Two competing `combined_replay` processes appeared after campaign start | 24,877 / 52,315 / 138,967 / 2,093,766 / 4,087,400 | p99.9 <= 250,000 | `8731695ee4f570876f433fe2bc861bf4df9a282a98d26be9e3c835532fcc345c` |
| Diagnostic retry | Competing replay still present | 23,811 / 46,325 / 59,478 / 270,066 / 2,590,782 | p99.9 <= 250,000 | `f813842821f0b7dbe7c6103c463f1e4e02ad834ae0e135fe0994f3b6afd468b8` |
| Fresh idle warm-up | No Cargo/Rust overlap | 27,495 / 55,908 / 70,891 / 90,870 / 420,053 | p50 <= 25,000 | `f0cda3c604542a1f3e012a59dcce6086535c8cb1e53ff42326e73328be6fec41` |

Each PM invocation internally uses 15,000 samples, one internal warm-up and
three recorded runs, exact nearest-rank percentiles, zero owner-loop
allocations, and frozen logical/hash checks. All three application processes
exited `101` at the benchmark's existing latency invariant. No result was
discarded, threshold weakened, or cherry-picked. Since the first clean
warm-up failed, Phase 0 has no valid three-run idle PM baseline and its
performance gate is independently red. This is host-local evidence, not proof
of a code regression: the source tree was unchanged.

### Official Documentation Capture

All sources below were fetched credential-free. The documentation batch was
retrieved at `2026-07-26T16:56:50Z`; requested and final URL are shown relative
to `https://docs.polymarket.com/` except the labeled OKX row. Every response
was HTTP 200. Exact response headers and bodies are retained under
`target/tmp/goal-g-phase0/docs`.

`https://docs.polymarket.com/llms.txt` was retrieved at the response date
`2026-07-26T16:56:26Z`, content type `text/plain; charset=utf-8`, 44,511
bytes, SHA-256
`2ead7fcd7730b7978969c577430c4dd1218faaf4e1594fca34afde09f4b50adb`.

`https://docs.polymarket.com/api-spec/clob-openapi.yaml` was retrieved at the
response date `2026-07-26T16:52:46Z`, content type
`application/octet-stream, text/yaml`, 218,860 bytes, SHA-256
`0f56ba4f6459d586636a18687fe05d3b5675bd7e707c7160f1a7aeb3306de070`,
ETag `"c8ba9373a2c2ec63e1fb062fd21e0b4f"`, and Last-Modified
`2026-06-16T22:55:17Z`.

| Requested path | Final path | Status | Content type | Bytes | SHA-256 |
| --- | --- | ---: | --- | ---: | --- |
| `getting-started/api.md` | `getting-started/api.md` | 200 | `text/markdown; charset=utf-8` | 9,391 | `6c397c66109852220b3f5d8033ea274061b3fc44b426edc9faa60673ecbef8fc` |
| `v2-migration.md` | `v2-migration.md` | 200 | `text/markdown; charset=utf-8` | 25,625 | `8dc52780b87a85faa22a030174cb28a2ee7bfd3c2797712527e3a83147452c7b` |
| `trading/wallets-auth.md` | `trading/wallets-auth.md` | 200 | `text/markdown; charset=utf-8` | 54,259 | `34095970a28c384375127aa481b3f928c3e3c2337414aa3d3af4a8b8bd43e8f5` |
| `trading/matching-engine.md` | `trading/matching-engine.md` | 200 | `text/markdown; charset=utf-8` | 6,864 | `f0d718b69509593654cb0085bb94fb96447c9b8d5ece6ddb990003d1e1f6f36c` |
| `trading/place-orders.md` | `trading/place-orders.md` | 200 | `text/markdown; charset=utf-8` | 59,742 | `057a19e82957de35c08ac9199230a4c9affcade06ad5186499ad9aa31ba291ea` |
| `trading/manage-orders.md` | `trading/manage-orders.md` | 200 | `text/markdown; charset=utf-8` | 52,401 | `e4a0238db31d5137b4d0da0d4333b1fb90be8f7c7b47d92968edfd993c8c4482` |
| `trading/realtime-order-updates.md` | same | 200 | `text/markdown; charset=utf-8` | 15,782 | `b5fde86d9fd5c63f5148b55af5357bcaafd67d090e08f054a2b0774ffc55b741` |
| `api-reference/trade/post-a-new-order.md` | same | 200 | `text/markdown; charset=utf-8` | 13,429 | `6c1924f515da4d960337a2db67b37c3d43965dbaa5b8616bd02d95a0a789e8f5` |
| `api-reference/trade/cancel-single-order.md` | same | 200 | `text/markdown; charset=utf-8` | 5,932 | `a12f96e0772df0b68d4fa194504e9778fd073a3cc64a4c2adcb7f67862ac0285` |
| `api-reference/trade/get-single-order-by-id.md` | same | 200 | `text/markdown; charset=utf-8` | 7,718 | `9644f4b22b029b486c2d6803720c9619c0ff3fd63d2b07310c3d8d35298ec1c1` |
| `api-reference/trade/get-user-orders.md` | same | 200 | `text/markdown; charset=utf-8` | 10,089 | `12f5e45ec908866cf8b1ebba687ad61458be79bcef75f7e3aa5d11cf4cdffb91` |
| `api-reference/trade/get-trades.md` | same | 200 | `text/markdown; charset=utf-8` | 11,928 | `a7b88859fbca99a55bb5c2d43fc21dedf1a44701dba9abc5383478df8883e3fb` |
| `api-reference/wss/user.md` | same | 200 | `text/markdown; charset=utf-8` | 27,402 | `9b6935e7ee56ec5a4f3e433d668e27c730cc6ea117d79fadb9eca5f4c9893c88` |
| `api-reference/wss/market.md` | same | 200 | `text/markdown; charset=utf-8` | 44,794 | `92a02634755fd92cc1c4a3f798ea64f050f76670e677003a9a595d8a8f4c616a` |
| `api-reference/markets/get-clob-market-info.md` | same | 200 | `text/markdown; charset=utf-8` | 5,703 | `b6e72949b7dc1c8c6cf97a1657a3dcd0f7aa145ca4ca9c82d9f9f77110b4e1a4` |
| `api-reference/market-data/get-order-book.md` | same | 200 | `text/markdown; charset=utf-8` | 5,690 | `fd98e9bea50208a07d4ea51a8d03e2048cb6cbf4db70149fb17deda8770815f7` |
| `api-reference/data/get-server-time.md` | same | 200 | `text/markdown; charset=utf-8` | 2,293 | `513b26c2d9237bd6ce641da1de738ecf99106665b149e298c6b658dc8a0571ec` |
| `api-reference/core/get-current-positions-for-a-user.md` | same | 200 | `text/markdown; charset=utf-8` | 5,424 | `ff5ae34274c305970f85741997f70149ed284350229a8d417386c3986a62db57` |
| `api-reference/geoblock.md` | same | 200 | `text/markdown; charset=utf-8` | 7,169 | `271b25f80b2a1c244afaab1396babf7196fb3e88f5e6ac7886e499b3f37a7172` |
| `api-reference/rate-limits.md` | same | 200 | `text/markdown; charset=utf-8` | 4,124 | `7ba9bdb5df4bfd12199349220dd3c0d0923f6fcc5352385d038ea4eb69ec051a` |
| `api-reference/trading-rate-limits.md` | same | 200 | `text/markdown; charset=utf-8` | 7,729 | `18851dde8fdbc782bd04d56a6eeb36523fef030daf61e98874c5d5c94c508f69` |
| `resources/contracts.md` | same | 200 | `text/markdown; charset=utf-8` | 7,987 | `ed59020bd28a24cbca9dbd2f92624a2a8ad7e403f0f08b6ff1529e33860c99a6` |
| `resources/error-codes.md` | same | 200 | `text/markdown; charset=utf-8` | 19,541 | `a0a93df3e5644349748692815cc9833319ccae3c829d218312df1a6771d08af3` |
| `concepts/order-lifecycle.md` | same | 200 | `text/markdown; charset=utf-8` | 8,341 | `854236c2602d4268c72d61f3c568f85ffb3f5e58071486abe8c88cecde9d8e76` |
| `market-data/websocket/overview.md` | `market-data/realtime-data.md` | 200 | `text/markdown; charset=utf-8` | 68,528 | `9347182b68b10e97bc587f30d206545c00b34e28d4c13d0afb316b8d76fabe3d` |
| `market-data/websocket/market-channel.md` | `market-data/realtime-data#market-stream.md` | 200 | `text/html; charset=utf-8` | 1,802,116 | `3da4c4537af0e3eaea64e14a25b62cd550e2127b33c79b61a74b7021bce6fc39` |
| `market-data/websocket/user-channel.md` | `trading/realtime-order-updates.md` | 200 | `text/markdown; charset=utf-8` | 15,782 | `b5fde86d9fd5c63f5148b55af5357bcaafd67d090e08f054a2b0774ffc55b741` |
| `OKX#public-data-websocket-index-tickers-channel` | same | 200 | `text/html; charset=UTF-8` | 5,207,183 | `c75edd5b041b36fc33f981b4c71b29f12f45104efd1e9c4e276881b84864b494` |

Interpretation was limited by role:

- authentication/signing sources establish EOA/type-0, L2 HMAC, request
  byte, outer owner, domain, field, and contract rules;
- trade/order/user-WS sources establish the candidate route and lifecycle
  unions but expose the conflicts recorded below;
- market/book sources retain Goal F's strict configured public contract;
- position sources prove pagination but no atomic complete cut;
- server-time/geoblock sources are readiness observations only;
- rate limits are ceilings, not Reap target throughput; and
- the OKX page establishes the public `/ws/v5/public` `index-tickers`
  subscription/acknowledgement contract for one configured `instId`.

### Pinned Official Git Sources

| Repository | Pinned current revision |
| --- | --- |
| `Polymarket/ctf-exchange-v2` | `ccc0596074f4dfd62c944fbca4de252893b82b4b` |
| `Polymarket/clob-client-v2` | `f3e1a05f868a1fd0c34ef85dfc45c6ce78f5bb69` |
| `Polymarket/rs-clob-client-v2` | `222143d321eba97d5711a848265eb9aab3bc7ff4` |
| `Polymarket/ts-sdk` | `0760f99f04e879164fafe79d8277395bb200cee9` |

Relevant immutable Git blob IDs:

| Repository/path | Blob |
| --- | --- |
| `ctf-exchange-v2/src/exchange/libraries/Structs.sol` | `0bbcd991063772a864bfe4c51679b7d589559d76` |
| `ctf-exchange-v2/src/exchange/mixins/Hashing.sol` | `a3dac60d83eef73893441bee174284d071346aa5` |
| `clob-client-v2/src/client.ts` | `19d9ed8e7515770db6868dae6ffd9438c672ec28` |
| `clob-client-v2/src/headers/index.ts` | `3b93d2a12b8019a4e2b2d0c6562c0f93fc33fbfc` |
| `clob-client-v2/src/order-utils/model/ctfExchangeV2TypedData.ts` | `99ab28242caf4a93385471b433832c0cb8a23aa3` |
| `clob-client-v2/src/signing/eip712.ts` | `1deaf4ff6e95b857dbc5689444815c30380c4472` |
| `clob-client-v2/src/types/ordersV2.ts` | `34c2977494274e121293f4cd5ed9548b49275288` |
| `clob-client-v2/src/types/clob.ts` | `3d87ab8c5d25078eae96d620470eabee7021086b` |
| `clob-client-v2/tests/signing/hmac.test.ts` | `84867e618c0c1699a9c9993c201c8556d2d0c1d1` |
| `clob-client-v2/tests/headers/index.test.ts` | `cc6379882f77d8ca39d624fab79fcd5346290ac0` |
| `rs-clob-client-v2/src/auth.rs` | `c99ff3f68cb35752716ff322ce3f6b717e6b1390` |
| `rs-clob-client-v2/src/clob/client.rs` | `3976d8c4daa85a2879ea55cb35dcd0f22609b2e9` |
| `rs-clob-client-v2/src/clob/order_builder.rs` | `56db923f56a42c3a3f55e101a66cb39db614632a` |
| `rs-clob-client-v2/src/clob/types/response.rs` | `80be8d09c26373c2af3b96068ab2a4742dbdce4e` |
| `rs-clob-client-v2/tests/clob.rs` | `321417692cd1e33ff776476f107975abb545c3d3` |
| `rs-clob-client-v2/tests/common/mod.rs` | `65df3341b506908c1f319e7f73cae46559718d71` |
| `ts-sdk/packages/client/src/authentication.ts` | `4d2b6b835ae4b0d7432638db4681122e5dcf9f9c` |
| `ts-sdk/packages/client/src/actions/orders/typed-data.test.ts` | `1bf793696ddbf3899bf1e39b0f8ff34e3299bcb1` |
| `ts-sdk/packages/client/src/actions/orders/prepare.ts` | `aaec7845f95fda01ca936d0f39739e60ff6d3532` |
| `ts-sdk/packages/client/src/actions/orders/post.ts` | `228db2114018d374cdbdff6e66e2ecee9b70c2a2` |
| `ts-sdk/packages/client/src/actions/orders/types.ts` | `cf7d21884e64b7822823db77b04ef2b925b57e71` |
| `ts-sdk/packages/client/src/exchange.ts` | `dad00e63d8456ad50de06c5009db16804225847c` |
| `ts-sdk/packages/bindings/src/clob/account.ts` | `962704ef41f7f5879d5a18c1e7664f27eb37182f` |
| `ts-sdk/packages/client/src/actions/orders/allowance.ts` | `dd66e53acb298bb0142af343674ee9f48c97a4b6` |
| `ts-sdk/packages/client/src/actions/orders/trade.ts` | `90adbd04f3784c68d623d05fa3f8e570636f82b3` |
| `ts-sdk/packages/client/src/actions/approvals.ts` | `d41f1f103f5d4cc805de0355549153be8d49d565` |

These clients are differential-vector oracles only. None was added as a Reap
dependency.

### Frozen Protocol Decisions That Did Resolve

| Contract | Phase 0 decision |
| --- | --- |
| Account profile | Current docs support type-0 EOA only when the EOA is allowlisted; library shape is `maker == signer == funder == POLY_ADDRESS`; Goal H must certify a real allowlisted/funded account |
| EIP-712 | Domain `Polymarket CTF Exchange`, version 2, chain 137; standard and neg-risk contracts recorded in the boundary |
| Signed order | Exact 11-field V2 type and type hash; `BUY=0`, `SELL=1`, `signatureType=0`, timestamp milliseconds |
| Expected venue ID | Contract `hashOrder` / EIP-712 digest, compared as 32-byte identity |
| Outer owner | L2 API-key UUID, not maker/funder address |
| L2 | Unix seconds; uppercase method; path without query; exact body bytes; base64-decoded secret; HMAC-SHA256; padded base64url |
| Place/cancel | `POST /order`; exact-owned `DELETE /order` with `orderID`; one serialization and one application dispatch attempt |
| Private reads | `/data/orders`, `/data/order/{id}`, `/data/trades`, `/balance-allowance`; path only is HMAC-signed |
| Pagination | Start `MA==`, terminal `LTE=` from current V2 client consensus; cycles/malformed/overflow are incomplete |
| Visibility | Credential-visible only; no funder-wide absence claim |
| User WS | One credential-bearing initial frame per epoch; no later subscribe/update capability; configuration change closes/reconnects; object/array event envelopes; no documented subscription ACK; REST reconciliation gates readiness |
| Position API | Exact lexical numerics, `sizeThreshold=0`, bounded offset/limit; monitored non-atomic projection only |
| Heartbeat | Excluded; official route/body/ID sources conflict |

The current official L2 regression vector is:

```text
secret: AAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAA=
timestamp: 1000000
method: test-sign
path: /orders
body: {"hash": "0x123"}
signature: ZwAdJKvoYRlEKDkNMwd5BuwNNtg93kNaR_oU2HrfVvc=
```

It is public synthetic test data. The upstream method is deliberately not an
HTTP method, so it proves the HMAC primitive/encoding only; Phase 2 would
still need independently authored GET and POST vectors.

### Exact Stop Conditions

#### A. Conditional Allowance Cannot Become Typed Operator Approval

The OpenAPI `BalanceAllowanceResponse` defines:

```text
balance: string
allowances: map<address, string>
```

for both `COLLATERAL` and `CONDITIONAL`. It gives amount examples and no
asset-kind-specific encoding. At the pinned official revisions:

1. the unified TypeScript SDK parses both kinds into
   `Record<Address,bigint>`;
2. its order path compares even a conditional value to `makerAmount`;
3. its separate on-chain approval path correctly models ERC-20 allowance as
   `uint256` and ERC-1155 `isApprovedForAll` as `bool`;
4. the legacy TypeScript client exposes only `Record<string,string>`;
5. the Rust client preserves `HashMap<Address,String>`; and
6. its only mock allowance `"1"` is a `COLLATERAL` request, not a
   conditional approval vector.

No source states whether conditional false/true is `0/1`,
`0/max_uint256`, another sentinel, or something else. SDK threshold behavior
is not a versioned server contract. Mapping arbitrary positive text to true
would reproduce the exact unsafe behavior prohibited by Goal G.

The pinned Predarb fixture
`balance_allowance.json` has SHA-256
`7e1f683ac5032b137d8a2afdfafccce389198bb5d3a33ba6eb3cb478455fab96`
and contains only scalar strings `"1000.00"` for balance/allowance, with no
asset kind, token, spender, or map. Predarb can divide a selected or arbitrary
first map value by one million and use `> 0` as approval. That is historical
code-path arithmetic, not fixture provenance or current protocol authority.

This exactly triggers:

> exact ERC20 allowance versus ERC1155 operator-approval semantics cannot be
> proven without weakening the typed core.

#### B. Lifecycle And Timestamp Sources Conflict

The current OpenAPI REST order enum uses
`ORDER_STATUS_LIVE|INVALID|CANCELED_MARKET_RESOLVED|CANCELED|MATCHED`;
guides/WS use unprefixed `LIVE|MATCHED|CANCELED`; the current Rust SDK also
models `DELAYED|UNMATCHED` and aliases. OpenAPI trade statuses use
`TRADE_STATUS_*`; WS/guides/SDKs use
`MATCHED|MINED|CONFIRMED|RETRYING|FAILED`. POST response status is a third,
lowercase namespace (`live|matched|delayed`).

User-WS prose labels timestamp as milliseconds while examples include
10-digit seconds-shaped values; current TypeScript schemas/tests use
millisecond-shaped values. A strict union parser can preserve and quarantine
these families, but current Goal G requires Phase 0 to resolve exact reached
status and time contracts rather than silently choose or probe with a real
credential. This is a second documented stop unless the prompt explicitly
authorizes a compatibility union with no success promotion.

#### C. Phase 0 Performance Gate Is Not Green

The existing PM benchmark failed its own frozen threshold on all three
attempts, including a clean no-overlap warm-up. The result cannot be hidden or
weakened. Because source was unchanged and no valid warm-up exists, this is
an unstable same-host baseline rather than proof of a regression, but the
required one-warm-up/three-run Phase 0 campaign is not green.

### Second Blocked Audit — 2026-07-27

A second independent, credential-free audit searched additional official
Polymarket Python clients, the Python SDK, the CLI, example repositories, and
the public schema/client history. It independently reproduced the same
mandatory Phase 0 stop. It found no versioned false/true encoding for
`CONDITIONAL` values and found positive evidence that the CLOB value is not
modeled as an ERC-1155 boolean:

- the official Python SDK's SELL/`CONDITIONAL` unit vector supplies `"777"`
  and expects the parsed integer `777`;
- its placement action compares that numeric CLOB value with maker amount,
  then invokes ERC-1155 approval as a separate boolean operation;
- the official CLI emits the authenticated CLOB allowance map unchanged,
  while its separate approval command reads `IERC20.allowance` as `U256` and
  `IERC1155.isApprovedForAll` as `bool`; and
- official Python, TypeScript, Rust, and example code either preserves the
  CLOB response or performs the direct on-chain boolean call. None defines a
  conversion between them.

The safe amended contract must preserve two separately sourced facts:

1. numeric, per-selected-spender CLOB-reported conditional spendability; and
2. boolean, on-chain ERC-1155 operator approval for the same owner and
   selected standard or neg-risk exchange.

The numeric fact cannot authorize the boolean fact. The pre-amendment Goal G
demanded the conversion but did not authorize the closed Polygon RPC read that
would obtain the boolean independently.

#### Supplemental Official Documentation Capture

The following pages were fetched without credentials at response dates from
`2026-07-27T02:03:49Z` through `2026-07-27T02:03:51Z`. Every response was
HTTP 200 with content type `text/markdown; charset=utf-8`. Exact bodies and
request metadata are retained in ignored Phase 0 evidence under
`target/tmp/goal-g-phase0/official-extra`.

| Requested path | Final path | Bytes | SHA-256 |
| --- | --- | ---: | --- |
| `trading/clients/l2.md` | `trading/manage-orders.md` | 52,401 | `e4a0238db31d5137b4d0da0d4333b1fb90be8f7c7b47d92968edfd993c8c4482` |
| `getting-started/python.md` | same | 10,363 | `bb1e8cc105eeccdb927a3591a16ddfd697699c2171f3d391aa31ce45b6af3cef` |
| `getting-started/typescript.md` | same | 9,733 | `aed3099526bf5f981d08cbb54abf4915898bb7b573b7301c6bc2793c9e327a80` |
| `getting-started/migrate-from-previous-sdks.md` | same | 60,918 | `7f35be1847b3f399e8bea8fedc9846f7a7b882aa9cd775a6f5740881a526eff9` |
| `trading/positions/how-positions-work.md` | same | 6,268 | `ab5ba581884ee412beaff60ab7203ebd0963faabf5c1880cb305c75a0196e99f` |
| `concepts/positions-tokens.md` | same | 4,127 | `bb131a3894d52149058f8554edaa6fafdd9ad72ab2351efbec605d4b3a6e2cc5` |

These pages add no asset-dependent allowance encoding.

#### Supplemental Official Git Evidence

| Repository | Revision | Relevant path | Blob |
| --- | --- | --- | --- |
| `Polymarket/py-clob-client-v2` | `215fc63a8fd6ec3a10c7edb73997c9772d8686d3` | `py_clob_client_v2/client.py` | `e0a7e6e3a8916222ddeb9c765ff4f1dbe2771b60` |
| same | same | `examples/account/approve_allowances.py` | `580127660affe09f7653e56ce38b3a91f57a5743` |
| `Polymarket/py-sdk` | `6a8f73267f3e776c1d2e8abed538dd5f3fbcda00` | `src/polymarket/models/clob/account.py` | `d29be8a8d14d2dd340e250f567df3c6c87a1e089` |
| same | same | `tests/unit/test_order_allowance.py` | `54aaa40a5448fef2dc4bbfd2f214cecea89e1335` |
| same | same | `src/polymarket/_internal/actions/orders/place.py` | `a995fbb025f7e20c2c97cec1f70d01c4e1b129d1` |
| `Polymarket/polymarket-sdk` | `a8401892976b3cbff0acfaf1c277aaddb241d5a4` | `src/utils/approveErc1155.ts` | `f014da58b71297c6e63bfa83a218f753fe0a8b21` |
| same | same | `src/abi/ERC1155.json` | `3918e0e587d9e059433a895f3b122d4f1f6b0714` |
| `Polymarket/py-clob-client` | `b076b04d61135657e25dccc1bbd6866a96bd8c6e` | `py_clob_client/client.py` | `e6be3c56c807b860d46bfbf0f23875f0c370cc08` |
| `Polymarket/polymarket-cli` | `9b18b5faf5493b945c48ca22efaf9645f0c69ab8` | `src/output/clob/account.rs` | `6c07b1f541ff59e2faa569a04026f651c4a6a9f9` |
| same | same | `src/commands/approve.rs` | `6de94be8b7a453891d6c9a1db7b66563f3a873f6` |
| `Polymarket/ts-sdk` | `0760f99f04e879164fafe79d8277395bb200cee9` | `packages/bindings/src/clob/account.ts` | `962704ef41f7f5879d5a18c1e7664f27eb37182f` |
| same | same | `packages/client/src/actions/orders/allowance.ts` | `dd66e53acb298bb0142af343674ee9f48c97a4b6` |
| same | same | `packages/client/src/actions/approvals.ts` | `d41f1f103f5d4cc805de0355549153be8d49d565` |

Additional official approval examples were pinned at these revisions:

| Repository | Revision | Relevant behavior |
| --- | --- | --- |
| `Polymarket/magic-safe-builder-example` | `479ca38fff10c72e3b9aafb83becfaaa44ae2216` | Direct on-chain `isApprovedForAll` |
| `Polymarket/wagmi-safe-builder-example` | `e16d88ec0cd6fa67eda5f4156c2db14a221eb9af` | Direct on-chain `isApprovedForAll` |
| `Polymarket/safe-wallet-integration` | `49ec9991b7f3e95197a4d53910f6086bf3ff2294` | Direct on-chain `isApprovedForAll` |
| `Polymarket/turnkey-safe-builder-example` | `df5dd8a5a149bdbe3e23179853ff5eabfcc93675` | Direct on-chain `isApprovedForAll` |

The public client history also fails to supply the missing contract:

- current `clob-client-v2` revision
  `f3e1a05f868a1fd0c34ef85dfc45c6ce78f5bb69` retains response-type blob
  `3d87ab8c5d25078eae96d620470eabee7021086b`;
- the map-shape transition at commit
  `e4bdef890c10322bd059b42314a634beed6d4ac4`, blob
  `b2f3a5cf77e2190e49a6a4a08f0c71e058b7310b`, says only that it matches
  the new server payload;
- the original endpoint commit
  `b7045899d4e55c629b58aeae4975e9679091512b`, blob
  `190b30e0afa4ab3f103d74895f335795c2625e78`, used one string field for
  both asset kinds; and
- the public Polymarket organization exposes clients and contracts but no
  CLOB server/backend or documentation-source repository that defines the
  server value semantics.

The current OpenAPI capture remains SHA-256
`0f56ba4f6459d586636a18687fe05d3b5675bd7e707c7160f1a7aeb3306de070`
and still supplies one `map<address,string>` shape with no asset-dependent
encoding. No exact `0/1`, `0/max_uint256`, or other encoding was found.

This was a second blocked audit, not an implementation phase. No real
credentials were loaded, no authenticated request was made, no external
mutation was sent, and no Reap production or dependency file was changed.
The active Goal G objective is not complete.

### Predarb Lessons And Rejections

Useful pinned historical concepts:

- sign and dispatch the same exact body bytes;
- keep mutation and reconciliation lanes separate;
- persist intent/dispatch state before effect;
- reconcile acknowledgement-unknown instead of blind placement retry;
- structurally deduplicate fills; and
- separate published, balance, reservation, and fill-derived position
  evidence.

Rejected Predarb patterns include cloneable/debuggable credentials, arbitrary
hosts, create-then-derive provisioning fallback, generic venue commands,
cancel-all, allowance mutation, heartbeat, broad account/runtime ownership,
timestamp-only salt, rounded/floating amounts, raw body logging, weak journal
durability, arbitrary allowance selection, `f64` positions, unbounded/unsafe
WS delivery, dropped pre-mapping events, and tuple-based ownership inference.

The five historical parser fixture hashes remain exactly:

| Fixture | SHA-256 |
| --- | --- |
| `balance_allowance.json` | `7e1f683ac5032b137d8a2afdfafccce389198bb5d3a33ba6eb3cb478455fab96` |
| `market_book.json` | `8e671f14c4b1e8137b1dc1b0bd7d39c79d9c8f961a8483daa32151df99cbdf81` |
| `open_order.json` | `d0998ca29cf47ce4bcb1fb4d7183d1e895a044d859235230a6ebef464295baf2` |
| `user_order.json` | `e4c3cd7975b7dc16c4c8d014444fc2a96d927cf1b9089b33875a5450b4ff99fa` |
| `user_trade.json` | `042998055ec5dec2c69065d002b2619d8497faabd9bfcc36c27a1bcf7cfe224c` |

They are parser seeds only.

## Historical Unblock And Then-Current Next Action

The user selected the closed Polygon-read option and authorized the complete
Amendment 1 contract recorded near the top of this handoff. The prompt and
boundary now freeze the ABI, standard/negative-risk exchange selection,
provider-reported finalized-block consistency, freshness, failure behavior,
bounded non-generic dependency edge, origin deferral, strict lifecycle/time
union, and paired benchmark policy. No CLOB conditional encoding is guessed.

At that point, Goal G was no longer blocked by the three historical findings.
The amended run completed the narrowly authorized benchmark-policy tranche,
Amendment 2 source/contract review, source re-attestation, and baseline
campaign. It then stopped correctly on the clean replay's immutable valid red
result. Goal G could not resume by retrying or discarding that result. The
separately scoped [Goal G-R](goal-g-replay-repair-prompt.md), a reviewed
Amendment 3 with a new evidence root, and the Phase 1 storage gate were the
then-current next actions. Later sections record Goal G-R's completion,
Amendment 3's conditional authorization, and its terminal activation stop.
Neither amendment claimed that Phase 0, an implementation phase, production
origin, account qualification, or trading authorization was complete.

## User-Authorized Amendment 3 — 2026-07-29

The user authorized the conditional return contract in
`docs/polymarket-authenticated-execution-goal-g-amendment-3.md` together with
its exact pre-activation runner/command contract at
`docs/polymarket-authenticated-execution-goal-g-amendment-3-runner-contract.md`,
Goal G-R Amendment 6 and the combined runnable prompt
`docs/polymarket-authenticated-execution-goal-g-resume-prompt.md`.

Amendment 3 is not active at authorization. It may activate only after Goal
G-R Amendment 6 commits a sealed green defect-class campaign while retaining
the original historical-causality stop and every old evidence hash.

The amendment:

1. preserves `target/tmp/goal-g-phase0-amended` byte-for-byte as immutable red
   evidence;
2. adopts only the exact two-file Amendment 5 repair without a historical
   equivalence claim;
3. supersedes the stale pre-repair PM workload cutoff and requires a fresh
   current-revision baseline;
4. authorizes a separate pre-activation recorder bundle and a distinct
   post-activation append-only evidence root;
5. requires the repeated defect-class confidence campaign, source/inventory
   re-attestation, a complete new 16-invocation baseline, and one fresh Goal G
   replay;
6. permits the original Goal G Phases 1-7 only after a separate green Phase 0
   gate commit; and
7. preserves every original product, capability, security, no-production,
   and completion boundary.

```text
goal_g_amendment_3_status=activation-stopped-inactive
goal_g_amendment_3_schema=goal-g-amendment-3-v1
goal_g_amendment_3_repair_tip=77ad6f30f79eb0b6d99881da97ec94e550364d1a
goal_g_amendment_3_repair_tree=9273cead973ecdd687ae11fa51d666f638e4a426
goal_g_amendment_3_authorization_parent=77ad6f30f79eb0b6d99881da97ec94e550364d1a
goal_g_amendment_3_authorization_subject=docs: authorize goal g-r closure and conditional goal g return
goal_g_amendment_3_contract_path=docs/polymarket-authenticated-execution-goal-g-amendment-3.md
goal_g_amendment_3_contract_sha256=7a1303d54c3210568c2e631bf3a2c6f0ab738f62f16cc40f9be9a4d84da4fa1c
goal_g_amendment_3_runner_contract_path=docs/polymarket-authenticated-execution-goal-g-amendment-3-runner-contract.md
goal_g_amendment_3_runner_contract_sha256=fbc09553c2418a61f04066754842db1d60464dbdd42e12d9cf809e3e6ae48165
goal_g_amendment_3_resume_prompt=docs/polymarket-authenticated-execution-goal-g-resume-prompt.md
goal_g_amendment_3_resume_prompt_sha256=e7a3a49b27fcaf0b46c94d3329fb09ada54acc23073d401cb0b08443461d9c44
goal_g_amendment_3_boundary_sha256=0e20d022f80c09eae223c5ef950b90f2ed3b903c6e7aeaefcb3f8e8d7cb81512
goal_g_amendment_3_authorization_path_count=8
goal_g_amendment_3_activation_tracked_allowlist=docs/polymarket-authenticated-execution-goal-g-handoff.md
goal_g_amendment_3_phase0_gate_tracked_allowlist=docs/polymarket-authenticated-execution-goal-g-handoff.md
goal_g_amendment_3_activation_subject=docs: activate goal g amendment 3
goal_g_amendment_3_phase0_gate_subject=docs: qualify goal g amendment 3 phase 0
goal_g_amendment_3_bundle_root=target/tmp/goal-g-amendment-3-recorder-bundle
goal_g_amendment_3_evidence_root=target/tmp/goal-g-phase0-amendment-3
goal_g_amendment_3_runtime_root=target/tmp/goal-g-amendment-3-runtime
goal_g_amendment_3_old_evidence_modified=false
goal_g_amendment_3_old_attempt_relabelled=false
goal_g_amendment_3_new_pm_baseline_required=true
goal_g_amendment_3_full_sixteen_invocation_baseline_required=true
goal_g_amendment_3_phase1_authorized_before_green_phase0=false
goal_g_amendment_3_goal_g_r_historical_equivalence_claimed=false
goal_g_amendment_3_goal_g_resumed=false
goal_g_amendment_3_production_order_entry_authorized=false
goal_g_amendment_3_real_credentials_loaded=false
goal_g_amendment_3_authenticated_external_request_sent=false
goal_g_amendment_3_real_polygon_rpc_request_sent=false
goal_g_amendment_3_real_order_submitted=false
goal_g_amendment_3_push_authorized=false
```

## User-Authorized Amendment 4 Storage Reset — 2026-07-30

The user authorized the narrow storage-reset contract in
`docs/polymarket-authenticated-execution-goal-g-amendment-4.md` after
Amendment 3's mandatory preactivation storage check stopped with
`1035091968` available bytes against a `2147483648`-byte minimum.

The stop occurred at clean Goal G-R closeout commit
`fc1ceba88fc91bc5c55d34fb639a4b575e584844`. The official Amendment 3 bundle,
Phase 0 evidence, and runtime paths were all absent; no tracked edit, index
edit, commit, or push had occurred. The user authorized removal of exactly
`/home/ubuntu/app/predarb/target` and
`/home/ubuntu/app/predarb-flatness-fix/target`. Both disposable build-cache
paths were removed and verified absent, leaving `11308576768` available
bytes. Source, captures, retained/non-cache artifacts, credentials, and
historical evidence were not modified.

For only the activation-parent/direct-child lineage and the procedure for
returning from this recorded preactivation storage stop, Amendment 4 has
precedence over conflicting clauses in the Goal G return-sequence prompt,
Amendment 3, and its runner contract. The conditional lineage authorized
before `S4` was:

```text
A -> R6 -> S4 -> G3 -> P0
```

Under that authorization, `S4` had to be the direct child of `R6`; `G3`
would have had to be the direct child of `S4`; and `P0`, if authorized, would
have had to be the direct child of `G3`. Amendment 3's full no-Cargo bootstrap
suite, two independent static reviews, official bundle construction and
sealing, fresh evidence, runtime-absence gates, safety boundaries, and
no-retry rules remained mandatory.

Files under `/tmp/reap-g3-draft` remain non-authoritative previews, not an
official bundle or evidence attempt. They may inform fresh, fully reviewed
construction but must not be blindly copied or relabeled.

The return-sequence prompt's Stage R6 is complete and must not be rerun.
The conditional Stage G3 path began only from clean `S4`; the terminal stop
below has now closed it. A post-`S4` preactivation failure before bundle
creation keeps bundle, evidence, and runtime absent, records
`bundle_state=absent-not-created`, and—when storage permits—commits only this
handoff directly after `S4` with exact subject
`docs: record goal g amendment 3 activation stop`. If bundle creation began,
the existing `partial-unsealed` or `sealed` states apply instead. No
activation stop or physical no-write exception permits a later `G3` without
a new reviewed, user-authorized amendment.

After `S4` is committed, the executor must re-authenticate its exact commit,
tree, parent, subject, two-path delta, contract hash, clean worktree, and hash
of this handoff at `S4`. Any activation handoff would have had to record those
immutable `S4` identities before activation was committed. None was created;
the terminal stop below supersedes that conditional path.

```text
goal_g_amendment_4_status=s4-committed-terminal-activation-stop
goal_g_amendment_4_schema=goal-g-amendment-4-v1
goal_g_amendment_4_parent=fc1ceba88fc91bc5c55d34fb639a4b575e584844
goal_g_amendment_4_parent_tree=6a198862a26c210ab1af68f5133a2f935fd4e6bb
goal_g_amendment_4_contract_path=docs/polymarket-authenticated-execution-goal-g-amendment-4.md
goal_g_amendment_4_contract_sha256=c4e08583fd51b9bbd5e18d17fe26390e3cdac489915cda43a68286307fb121ea
goal_g_amendment_4_subject=docs: authorize goal g amendment 3 storage reset
goal_g_amendment_4_tracked_allowlist=docs/polymarket-authenticated-execution-goal-g-amendment-4.md,docs/polymarket-authenticated-execution-goal-g-handoff.md
goal_g_amendment_4_stage_r6_rerun_authorized=false
goal_g_amendment_4_required_available_bytes=2147483648
goal_g_amendment_4_stop_available_bytes=1035091968
goal_g_amendment_4_stop_shortfall_bytes=1112391680
goal_g_amendment_4_reset_available_bytes=11308576768
goal_g_amendment_4_official_bundle_at_stop=absent-not-created
goal_g_amendment_4_official_evidence_at_stop=absent-not-created
goal_g_amendment_4_official_runtime_at_stop=absent-not-created
goal_g_amendment_4_cleanup_source_modified=false
goal_g_amendment_4_cleanup_captures_modified=false
goal_g_amendment_4_cleanup_retained_non_cache_artifacts_modified=false
goal_g_amendment_4_cleanup_historical_evidence_modified=false
goal_g_amendment_4_cleanup_authority_exhausted=true
goal_g_amendment_4_goal_g3_resumed=false
goal_g_amendment_4_production_order_entry_authorized=false
goal_g_amendment_4_real_credentials_loaded=false
goal_g_amendment_4_authenticated_external_request_sent=false
goal_g_amendment_4_real_polygon_rpc_request_sent=false
goal_g_amendment_4_real_order_submitted=false
goal_g_amendment_4_historical_goal_g_attempt_relabelled=false
goal_g_amendment_4_historical_goal_g_r_equivalence_claimed=false
goal_g_amendment_4_push_authorized=false
```

## Amendment 3 Terminal Activation Stop — 2026-07-30

After both independent static reviews passed the separately hashed
constructor, the executor ran the declared non-authoritative preview exactly
once. The preview failed during `construct_combined_fixtures`, before any
self-test case, finalization, official bundle construction, or sealing.
The preview tree is retained byte-for-byte and must not be modified, retried,
promoted, or relabelled.

The retained combined fixture contains exactly one valid `combined_replay`
JSON report. The frozen constructor nevertheless rejected it because line
1210 contains two backslash bytes before `{`; its Awk expression therefore
looks for a leading literal backslash and reports zero matches. This is a
deterministic constructor/parser defect, not a fixture, replay, load, or host
failure.

The preview is not the official recorder bundle or evidence. Official bundle
creation never began, and the official bundle, Phase 0 evidence, and runtime
roots remain absent. The normative official state is therefore
`bundle_state=absent-not-created`; the retained preview is separately
identified as non-authoritative partial-unsealed diagnostic evidence. This
stop terminates the `S4` lineage. A retry, repaired constructor, later `G3`,
or Phase 0 attempt requires a new reviewed, user-authorized amendment.

The forensic inventory hash below covers records ordered by raw relative-path
bytes in the form
`rel\0type\0mode4\0uid\0gid\0nlink\0size\0payload\n`. The root is `.`;
`type` is exactly `d` or `f`; `mode4` is the zero-padded four-digit octal
mode; `size` is `lstat.st_size` for every entry, including directories; and
`payload` is the file SHA-256 or `-` for a directory. The regular-file
manifest hash covers
`<sha256>  <relative-path>\n` records ordered by the relative-path bytes.
The argv stream concatenates all five displayed process arguments, each
followed by one NUL byte. The terminal-stderr and failed-constructor-line
hashes and byte counts include their terminating LF.

```text
goal_g_amendment_3_activation_stop_status=stopped
goal_g_amendment_3_activation_stop_schema=goal-g-amendment-3-activation-stop-v1
goal_g_amendment_3_activation_stop_stage=post-s4-preofficial-bundle-declared-preview
goal_g_amendment_3_activation_stop_s4_commit=706c4bd763647054264cdf3cb52d2355e0aa1b75
goal_g_amendment_3_activation_stop_s4_tree=415dc504849a5aa22704688fe348307f5938fbf4
goal_g_amendment_3_activation_stop_s4_parent=fc1ceba88fc91bc5c55d34fb639a4b575e584844
goal_g_amendment_3_activation_stop_s4_subject=docs: authorize goal g amendment 3 storage reset
goal_g_amendment_3_activation_stop_s4_path_count=2
goal_g_amendment_3_activation_stop_s4_paths=docs/polymarket-authenticated-execution-goal-g-amendment-4.md,docs/polymarket-authenticated-execution-goal-g-handoff.md
goal_g_amendment_3_activation_stop_s4_amendment_4_sha256=c4e08583fd51b9bbd5e18d17fe26390e3cdac489915cda43a68286307fb121ea
goal_g_amendment_3_activation_stop_s4_handoff_sha256=e14a753907296daded8d1334b1c9306342d8f6509322cdccff00a88ffda49a1a
goal_g_amendment_3_activation_stop_parent=706c4bd763647054264cdf3cb52d2355e0aa1b75
goal_g_amendment_3_activation_stop_subject=docs: record goal g amendment 3 activation stop
goal_g_amendment_3_activation_stop_tracked_allowlist=docs/polymarket-authenticated-execution-goal-g-handoff.md
goal_g_amendment_3_activation_stop_failed_gate=declared-preview-construct-combined-fixtures-report-extraction
goal_g_amendment_3_activation_stop_constructor_path=/var/tmp/reap-g3-draft-v2/construct-self-test.preview.sh
goal_g_amendment_3_activation_stop_constructor_sha256=2fe07168369ca726f17328b3d9142522ab2540d057b5d95dd9586a6ded952ee6
goal_g_amendment_3_activation_stop_constructor_bytes=362479
goal_g_amendment_3_activation_stop_constructor_review_1_result=pass
goal_g_amendment_3_activation_stop_constructor_review_1_reviewer=codex-root-g3-static-audit-a3
goal_g_amendment_3_activation_stop_constructor_review_1_session=stable-review-a3-20260730T091458Z
goal_g_amendment_3_activation_stop_constructor_review_2_result=pass
goal_g_amendment_3_activation_stop_constructor_review_2_reviewer=codex-g3-static-audit-b3
goal_g_amendment_3_activation_stop_constructor_review_2_session=root-g3-static-audit-b3-20260730
goal_g_amendment_3_activation_stop_command=/bin/busybox sh /var/tmp/reap-g3-draft-v2/construct-self-test.preview.sh preview /home/ubuntu/code/reap/target/tmp/goal-g-amendment-3-preview-v1
goal_g_amendment_3_activation_stop_preview_invocation_count=1
goal_g_amendment_3_activation_stop_argv_count=5
goal_g_amendment_3_activation_stop_argv_nul_sha256=97e00ee37bd12278536a903c246fa75af8ddb93691cfa7e06407594c844ffc52
goal_g_amendment_3_activation_stop_argv_nul_bytes=145
goal_g_amendment_3_activation_stop_exit=1
goal_g_amendment_3_activation_stop_terminal_stderr=goal-g-a3-constructor: could not extract exactly one combined fixture report
goal_g_amendment_3_activation_stop_terminal_stderr_sha256=e9f3c933894eae42d5ea7ef3364291e9e1ccea2ed2f2317f836500002e496ded
goal_g_amendment_3_activation_stop_terminal_stderr_bytes=77
goal_g_amendment_3_activation_stop_constructor_failed_line=1210
goal_g_amendment_3_activation_stop_constructor_failed_line_sha256=c9162b045263a9404e4d81cce87a4afbf9875cdaa37e4515953717fe0c5cc7e6
goal_g_amendment_3_activation_stop_parser_match_status=1
goal_g_amendment_3_activation_stop_preview_root=target/tmp/goal-g-amendment-3-preview-v1
goal_g_amendment_3_activation_stop_preview_official=false
goal_g_amendment_3_activation_stop_preview_state=partial-unsealed-retained-non-authoritative
goal_g_amendment_3_activation_stop_preview_dev=66305
goal_g_amendment_3_activation_stop_preview_inode=808763
goal_g_amendment_3_activation_stop_preview_uid=1000
goal_g_amendment_3_activation_stop_preview_gid=1000
goal_g_amendment_3_activation_stop_preview_root_mode=0700
goal_g_amendment_3_activation_stop_preview_entry_count_including_root=21
goal_g_amendment_3_activation_stop_preview_descendant_count=20
goal_g_amendment_3_activation_stop_preview_directory_count=13
goal_g_amendment_3_activation_stop_preview_regular_file_count=8
goal_g_amendment_3_activation_stop_preview_regular_bytes=615138
goal_g_amendment_3_activation_stop_preview_directory_modes=13x0700
goal_g_amendment_3_activation_stop_preview_regular_file_modes=6x0700,2x0600
goal_g_amendment_3_activation_stop_preview_link_or_other_type_count=0
goal_g_amendment_3_activation_stop_preview_bundle_and_self_test_seals=absent
goal_g_amendment_3_activation_stop_preview_trace_state=empty
goal_g_amendment_3_activation_stop_preview_result_parts_state=empty
goal_g_amendment_3_activation_stop_preview_forensic_inventory_sha256=82ac222e4932320ad14ce7ef7800bd8e39a373deaf6ce8205a9ab9ccbfd11747
goal_g_amendment_3_activation_stop_preview_regular_manifest_sha256=a86c192658af2e4edef79c70ae4f89e842ac9f57ba278f1b8c0ff835defe2df9
goal_g_amendment_3_activation_stop_preview_report_path=self-test/fixtures/reports/10-combined-valid.log
goal_g_amendment_3_activation_stop_preview_report_sha256=bbea695789a6c13ef3095f55622c0c9cf9108a1965f5010485f01628369a3d67
goal_g_amendment_3_activation_stop_preview_report_bytes=5347
goal_g_amendment_3_activation_stop_preview_report_lines=27
goal_g_amendment_3_activation_stop_preview_report_valid_json_count=1
goal_g_amendment_3_activation_stop_preview_report_valid_json_line=19
bundle_state=absent-not-created
goal_g_amendment_3_activation_stop_official_bundle_creation_started=false
goal_g_amendment_3_activation_stop_official_bundle_state=absent-not-created
goal_g_amendment_3_activation_stop_official_evidence_state=absent-not-created
goal_g_amendment_3_activation_stop_official_runtime_state=absent-not-created
goal_g_amendment_3_activation_stop_g3_created=false
goal_g_amendment_3_activation_stop_phase0_started=false
goal_g_amendment_3_activation_stop_cargo_invoked=false
goal_g_amendment_3_activation_stop_public_fetch_invoked=false
goal_g_amendment_3_activation_stop_preview_retry_authorized=false
goal_g_amendment_3_activation_stop_next_authority=new-reviewed-user-authorized-amendment
goal_g_amendment_3_activation_stop_push_authorized=false
```

## User-Authorized Amendment 5 — 2026-08-02

The user authorized the narrow constructor recovery contract in
`docs/polymarket-authenticated-execution-goal-g-amendment-5.md`. The failed
preview and frozen v2 draft remain immutable. Amendment 5 authorizes a fresh,
separately hashed v3 draft and a different one-shot preview root.

The original recommendation described a one-byte-only v3. Static boundary
review found that such a file could not run after a new authorization commit:
v2 also hard-binds preactivation and Phase 0 to the old `S4` parent. The sole
behavioral correction remains deletion of the excess Awk backslash. The
contract separately permits only the mechanical provenance work needed to
bind `A5 -> T -> S4 -> R6`, require `G3` as `A5`'s direct child, and document
and validate those facts. It permits no product, command, workload,
dependency, credential, network, or trading change.

The existing Amendment 3 activation-stop block remains unchanged historical
evidence. This new authorization does not relabel that stopped lineage or
permit reuse of its preview. A green successor must pass the parser regression,
the full retained no-Cargo suite, two independent v3 reviews, one fresh
preview invocation and its reviews, fresh official construction, two fresh
official reviews, and sealing before `G3` can exist.

```text
goal_g_amendment_5_status=activation-stopped-inactive
goal_g_amendment_5_schema=goal-g-amendment-5-v1
goal_g_amendment_5_parent_commit=ed7d34ea504cae9d7dbb4524f6f6ebf494f5648d
goal_g_amendment_5_parent_tree=bb98356880d8f088aa179e9fb8a84c1af068c7ef
goal_g_amendment_5_parent_parent=706c4bd763647054264cdf3cb52d2355e0aa1b75
goal_g_amendment_5_parent_subject=docs: record goal g amendment 3 activation stop
goal_g_amendment_5_parent_path_count=1
goal_g_amendment_5_parent_paths=docs/polymarket-authenticated-execution-goal-g-handoff.md
goal_g_amendment_5_parent_handoff_sha256=ed031f2465e5d4684ac4101b9be2e5fd54c68e1ea3632bf2657062abbc4a9032
goal_g_amendment_5_contract_path=docs/polymarket-authenticated-execution-goal-g-amendment-5.md
goal_g_amendment_5_contract_sha256=f1a2f5d2cdb5d0f9999ac365652608d1c0a5d42768b84e05abd42b62e5b97675
goal_g_amendment_5_authorization_subject=docs: authorize goal g amendment 5 constructor repair
goal_g_amendment_5_authorization_path_count=2
goal_g_amendment_5_authorization_paths=docs/polymarket-authenticated-execution-goal-g-amendment-5.md,docs/polymarket-authenticated-execution-goal-g-handoff.md
goal_g_amendment_5_activation_subject=docs: activate goal g amendment 3
goal_g_amendment_5_activation_tracked_allowlist=docs/polymarket-authenticated-execution-goal-g-handoff.md
goal_g_amendment_5_activation_a3_status_from=activation-stopped-inactive
goal_g_amendment_5_activation_a3_status_to=active-phase0
goal_g_amendment_5_activation_a5_status_from=authorized-inactive
goal_g_amendment_5_activation_a5_status_to=activation-complete-phase0-active
goal_g_amendment_5_phase0_subject=docs: qualify goal g amendment 3 phase 0
goal_g_amendment_5_activation_stop_subject=docs: record goal g amendment 5 activation stop
goal_g_amendment_5_lineage=S4->T->A5->G3->P0
goal_g_amendment_5_stage_r6_rerun_authorized=false
goal_g_amendment_5_failed_preview_root=target/tmp/goal-g-amendment-3-preview-v1
goal_g_amendment_5_failed_preview_state=partial-unsealed-retained-non-authoritative
goal_g_amendment_5_failed_preview_dev=66305
goal_g_amendment_5_failed_preview_inode=808763
goal_g_amendment_5_failed_preview_forensic_inventory_sha256=82ac222e4932320ad14ce7ef7800bd8e39a373deaf6ce8205a9ab9ccbfd11747
goal_g_amendment_5_failed_preview_regular_manifest_sha256=a86c192658af2e4edef79c70ae4f89e842ac9f57ba278f1b8c0ff835defe2df9
goal_g_amendment_5_failed_preview_report_path=self-test/fixtures/reports/10-combined-valid.log
goal_g_amendment_5_failed_preview_report_sha256=bbea695789a6c13ef3095f55622c0c9cf9108a1965f5010485f01628369a3d67
goal_g_amendment_5_failed_preview_retry_authorized=false
goal_g_amendment_5_failed_preview_promotion_authorized=false
goal_g_amendment_5_v2_root=/var/tmp/reap-g3-draft-v2
goal_g_amendment_5_v2_constructor_sha256=2fe07168369ca726f17328b3d9142522ab2540d057b5d95dd9586a6ded952ee6
goal_g_amendment_5_v2_constructor_bytes=362479
goal_g_amendment_5_v2_component_manifest_schema=sha256-tab-bytes-tab-basename-lf-ordered-by-basename-bytes
goal_g_amendment_5_v2_component_manifest_rows=10
goal_g_amendment_5_v2_component_manifest_bytes=933
goal_g_amendment_5_v2_component_manifest_sha256=82fa2de7bc468a5a60fa3f795f336d621515557a5ee21b9828b09d1d526cf4a8
goal_g_amendment_5_v2_root_dev=66305
goal_g_amendment_5_v2_root_inode=305347
goal_g_amendment_5_v2_root_mode=0700
goal_g_amendment_5_v2_root_uid=1000
goal_g_amendment_5_v2_root_gid=1000
goal_g_amendment_5_v2_root_nlink=2
goal_g_amendment_5_v2_entry_count_including_root=11
goal_g_amendment_5_v2_regular_file_count=10
goal_g_amendment_5_v2_regular_bytes=1038407
goal_g_amendment_5_v2_regular_file_modes=9x0664,1x0700
goal_g_amendment_5_v2_forensic_inventory_sha256=062c306df0e3a5b331be79df841dc98eefeed1a9d1a5b899968bae662d59f0cb
goal_g_amendment_5_v2_mutation_authorized=false
goal_g_amendment_5_v2_invocation_or_promotion_authorized=false
goal_g_amendment_5_provenance_control_root=/var/tmp/reap-g3-draft-v3-provenance-control
goal_g_amendment_5_provenance_patch=/var/tmp/reap-g3-draft-v3-provenance.patch
goal_g_amendment_5_provenance_patch_file_sections=5
goal_g_amendment_5_provenance_control_invocation_authorized=false
goal_g_amendment_5_provenance_patch_bundle_input=false
goal_g_amendment_5_review_1_scratch_root=/var/tmp/reap-g3-draft-v3-review-1-scratch
goal_g_amendment_5_review_2_scratch_root=/var/tmp/reap-g3-draft-v3-review-2-scratch
goal_g_amendment_5_review_scratch_create_limit_each=1
goal_g_amendment_5_review_scratch_remove_authorized_after_pass=true
goal_g_amendment_5_review_scratch_preserve_on_failure=true
goal_g_amendment_5_v3_root=/var/tmp/reap-g3-draft-v3
goal_g_amendment_5_v3_changed_file_count=5
goal_g_amendment_5_v3_changed_files=SELF-TEST-DESIGN.md,SELF-TEST-SCHEMA.md,construct-self-test.preview.sh,run-attempt.sh,validators.sh
goal_g_amendment_5_v3_unchanged_file_count=5
goal_g_amendment_5_v3_unchanged_files=commands.tsv,inventory.preview.sh,run-phase0-replay.preview.sh,source-reattest.preview.sh,summarize-baseline.preview.sh
goal_g_amendment_5_repository_fact_fields=candidate_parent,t_commit,t_tree,t_parent,t_subject,t_handoff_sha256,a5_commit,a5_tree,a5_parent,a5_subject,a5_contract_sha256,a5_handoff_sha256
goal_g_amendment_5_phase0_meta_fields=t_commit,t_tree,t_parent,t_subject,t_handoff_sha256,a5_commit,a5_tree,a5_parent,a5_subject,a5_contract_sha256,a5_handoff_sha256
goal_g_amendment_5_activation_handoff_t_fields=goal_g_amendment_5_t_commit,goal_g_amendment_5_t_tree,goal_g_amendment_5_t_parent,goal_g_amendment_5_t_subject,goal_g_amendment_5_t_handoff_sha256
goal_g_amendment_5_activation_handoff_a5_fields=goal_g_amendment_5_a5_commit,goal_g_amendment_5_a5_tree,goal_g_amendment_5_a5_parent,goal_g_amendment_5_a5_subject,goal_g_amendment_5_a5_contract_sha256,goal_g_amendment_5_a5_handoff_sha256
goal_g_amendment_5_parser_deleted_byte=0x5c
goal_g_amendment_5_parser_deleted_offset_zero_based=51826
goal_g_amendment_5_parser_old_line_sha256=c9162b045263a9404e4d81cce87a4afbf9875cdaa37e4515953717fe0c5cc7e6
goal_g_amendment_5_parser_corrected_line_sha256=107cbbb11918f7bf6144f32a718ca10b6eabb328100721dc42dfbef0248393e1
goal_g_amendment_5_parser_only_constructor_bytes=362478
goal_g_amendment_5_parser_only_constructor_sha256=c6722bb7936564b427baa7822ba4a491166416f4dccfa5b5aa44d6f0a1051b45
goal_g_amendment_5_parser_regression_old_status=1
goal_g_amendment_5_parser_regression_old_output_bytes=0
goal_g_amendment_5_parser_regression_new_status=0
goal_g_amendment_5_parser_regression_new_output_count=1
goal_g_amendment_5_parser_regression_new_output_bytes=3790
goal_g_amendment_5_parser_regression_new_output_sha256=9e89454c35c52a823506f4f77d070d410ca5f504007754d7d0258944fa7a9f5d
goal_g_amendment_5_new_preview_root=target/tmp/goal-g-amendment-3-preview-v2
goal_g_amendment_5_new_preview_invocation_limit=1
goal_g_amendment_5_new_preview_argv_count=5
goal_g_amendment_5_new_preview_argv_nul_bytes=145
goal_g_amendment_5_new_preview_argv_nul_sha256=545ea1c137866eb41949219d931a8a4f8ef785992b68514045e0b1f407d0d4f2
goal_g_amendment_5_new_preview_review_count=2
goal_g_amendment_5_new_preview_distinct_reviewers_required=true
goal_g_amendment_5_new_preview_distinct_sessions_required=true
goal_g_amendment_5_official_bundle_root=target/tmp/goal-g-amendment-3-recorder-bundle
goal_g_amendment_5_official_evidence_root=target/tmp/goal-g-phase0-amendment-3
goal_g_amendment_5_official_runtime_root=target/tmp/goal-g-amendment-3-runtime
goal_g_amendment_5_official_bundle_state=absent-not-created
goal_g_amendment_5_official_evidence_state=absent-not-created
goal_g_amendment_5_official_runtime_state=absent-not-created
goal_g_amendment_5_production_order_entry_authorized=false
goal_g_amendment_5_real_credentials_loaded=false
goal_g_amendment_5_authenticated_external_request_sent=false
goal_g_amendment_5_real_polygon_rpc_request_sent=false
goal_g_amendment_5_real_order_submitted=false
goal_g_amendment_5_historical_goal_g_attempt_relabelled=false
goal_g_amendment_5_historical_goal_g_r_equivalence_claimed=false
goal_g_amendment_5_pre_g3_cargo_authorized=false
goal_g_amendment_5_pre_g3_test_or_benchmark_authorized=false
goal_g_amendment_5_pre_g3_public_fetch_authorized=false
goal_g_amendment_5_pre_g3_network_authorized=false
goal_g_amendment_5_push_authorized=false
```

## Amendment 5 Terminal Activation Stop — 2026-08-02

Amendment 5 passed its required read-only parser regression and produced a
separate provenance control, a five-section full-index provenance patch, and
a v3 draft whose sole control-relative behavioral change is deletion of the
excess Awk backslash. No constructor, preview, official bundle, Cargo command,
test, benchmark, public fetch, authenticated request, Polygon RPC, or order
entry was invoked.

The first independent review stopped before scratch creation while
authenticating the supplied control identity. The executor had calculated the
control and v3 forensic inventories with file bytes as `payload` and an empty
directory payload. The frozen forensic schema instead requires each file's
SHA-256 as `payload` and `-` for a directory. The reviewer's implementation
reproduced the exact v2 inventory and correctly rejected the supplied control
expectation. A later read-only recomputation established the correct control
and v3 forensic hashes, but Amendment 5 makes a static-review failure terminal;
the corrected calculation was not used to retry or relabel the review.

Both authorized review scratch roots remain absent and were never created.
The second review was cancelled immediately after the first stop. The control,
patch, and v3 bytes are retained as non-authoritative diagnostic artifacts.
The new preview root and all official roots remain absent. This stop terminates
the `A5` activation lineage before `G3`; any successor requires another
reviewed, user-authorized amendment.

```text
goal_g_amendment_5_activation_stop_status=stopped
goal_g_amendment_5_activation_stop_schema=goal-g-amendment-5-activation-stop-v1
goal_g_amendment_5_activation_stop_stage=post-a5-v3-prepreview-review-1-input-authentication
goal_g_amendment_5_activation_stop_parent_commit=ba3b666d95d8097f60f8fc33a12b9844115edca8
goal_g_amendment_5_activation_stop_parent_tree=adce063ce141af8cc53b28d2a5ae4ba0d36cebba
goal_g_amendment_5_activation_stop_parent_parent=ed7d34ea504cae9d7dbb4524f6f6ebf494f5648d
goal_g_amendment_5_activation_stop_parent_subject=docs: authorize goal g amendment 5 constructor repair
goal_g_amendment_5_activation_stop_parent_contract_sha256=f1a2f5d2cdb5d0f9999ac365652608d1c0a5d42768b84e05abd42b62e5b97675
goal_g_amendment_5_activation_stop_parent_handoff_sha256=49aada8d7ee901c660160724d22375d4c1ce1534c1c52182133c9fd6ccb74720
goal_g_amendment_5_activation_stop_tracked_allowlist=docs/polymarket-authenticated-execution-goal-g-handoff.md
goal_g_amendment_5_activation_stop_failed_gate=review-1-control-forensic-inventory-expectation-mismatch
goal_g_amendment_5_activation_stop_failure_class=review-input-identity-capture-error
goal_g_amendment_5_activation_stop_forensic_schema=rel-nul-type-nul-mode4-nul-uid-nul-gid-nul-nlink-nul-size-nul-file-sha256-or-directory-dash-lf
goal_g_amendment_5_activation_stop_incorrect_forensic_schema=file-bytes-and-empty-directory-payload
goal_g_amendment_5_activation_stop_parser_regression_result=pass
goal_g_amendment_5_activation_stop_parser_regression_old_status=1
goal_g_amendment_5_activation_stop_parser_regression_old_output_bytes=0
goal_g_amendment_5_activation_stop_parser_regression_new_status=0
goal_g_amendment_5_activation_stop_parser_regression_new_output_count=1
goal_g_amendment_5_activation_stop_parser_regression_new_output_line=19
goal_g_amendment_5_activation_stop_parser_regression_new_output_bytes=3790
goal_g_amendment_5_activation_stop_parser_regression_new_output_sha256=9e89454c35c52a823506f4f77d070d410ca5f504007754d7d0258944fa7a9f5d
goal_g_amendment_5_activation_stop_control_root=/var/tmp/reap-g3-draft-v3-provenance-control
goal_g_amendment_5_activation_stop_control_state=retained-non-authoritative-not-invoked
goal_g_amendment_5_activation_stop_control_dev=66305
goal_g_amendment_5_activation_stop_control_inode=310092
goal_g_amendment_5_activation_stop_control_mode=0700
goal_g_amendment_5_activation_stop_control_uid=1000
goal_g_amendment_5_activation_stop_control_gid=1000
goal_g_amendment_5_activation_stop_control_nlink=2
goal_g_amendment_5_activation_stop_control_entry_count_including_root=11
goal_g_amendment_5_activation_stop_control_regular_file_count=10
goal_g_amendment_5_activation_stop_control_regular_bytes=1055726
goal_g_amendment_5_activation_stop_control_component_manifest_rows=10
goal_g_amendment_5_activation_stop_control_component_manifest_bytes=933
goal_g_amendment_5_activation_stop_control_component_manifest_sha256=50f7de09cb5f19de4a9f1375a4a4a5a1acf40b4f831e65004a0651664df3db61
goal_g_amendment_5_activation_stop_control_forensic_inventory_incorrect_sha256=4f2039164a8403a0ff9692f358fb513fb2b2e209ee3e179a0bff04d24814cd6e
goal_g_amendment_5_activation_stop_control_forensic_inventory_sha256=2f05254afe092859bcae96711f993cfd88165820896b0287441f2251206b9d51
goal_g_amendment_5_activation_stop_control_constructor_bytes=366813
goal_g_amendment_5_activation_stop_control_constructor_sha256=22677a1ebcdb6fa9bb59b885db6ef0133d62f9d28056e4bc4b632cfa4fde73db
goal_g_amendment_5_activation_stop_control_run_attempt_sha256=86a79706b6aa8253b7d8fb298c5016535aab33a2cd91f4c842b3c2d06c72ddcd
goal_g_amendment_5_activation_stop_control_validators_sha256=897f3bb05418397d8d17944dea70501a1bb2adbbf65c73acc06035726eab678b
goal_g_amendment_5_activation_stop_control_design_sha256=4f739c6f49d90418ba1e1576bf2f4015f1da9a4b9b8eed9ffa3de9414d21c5a4
goal_g_amendment_5_activation_stop_control_schema_sha256=a4d8e7ae085bd2517678e0762690c813d2e69232d463e3df83ec9956faf27ecd
goal_g_amendment_5_activation_stop_patch_path=/var/tmp/reap-g3-draft-v3-provenance.patch
goal_g_amendment_5_activation_stop_patch_state=retained-non-authoritative-not-applied-by-reviewer
goal_g_amendment_5_activation_stop_patch_sections=5
goal_g_amendment_5_activation_stop_patch_bytes=56207
goal_g_amendment_5_activation_stop_patch_sha256=fc340abca04400d0aff3fce73dcf5a309bdfaec838fc22bfd82aca5a46f55daf
goal_g_amendment_5_activation_stop_v3_root=/var/tmp/reap-g3-draft-v3
goal_g_amendment_5_activation_stop_v3_state=retained-non-authoritative-not-invoked
goal_g_amendment_5_activation_stop_v3_dev=66305
goal_g_amendment_5_activation_stop_v3_inode=310585
goal_g_amendment_5_activation_stop_v3_mode=0700
goal_g_amendment_5_activation_stop_v3_uid=1000
goal_g_amendment_5_activation_stop_v3_gid=1000
goal_g_amendment_5_activation_stop_v3_nlink=2
goal_g_amendment_5_activation_stop_v3_entry_count_including_root=11
goal_g_amendment_5_activation_stop_v3_regular_file_count=10
goal_g_amendment_5_activation_stop_v3_regular_bytes=1055725
goal_g_amendment_5_activation_stop_v3_component_manifest_rows=10
goal_g_amendment_5_activation_stop_v3_component_manifest_bytes=933
goal_g_amendment_5_activation_stop_v3_component_manifest_sha256=710ab62d5dbe846b21df74a4d78ee3f12d2a1883a22662d256bf751d411bc451
goal_g_amendment_5_activation_stop_v3_forensic_inventory_incorrect_sha256=cf5eb07c85af4721c586c90a778a2fb902c32d7bcd2748632062ec13a193e63c
goal_g_amendment_5_activation_stop_v3_forensic_inventory_sha256=9ab5b0695e60cf47fc41e9d2e110b291c56b856d80cbfad36b9d41b6a5c7d233
goal_g_amendment_5_activation_stop_v3_constructor_bytes=366812
goal_g_amendment_5_activation_stop_v3_constructor_sha256=7f16928835d296353d6cc94501bd3cabd6f7febc7da044606673d7ee287c9bba
goal_g_amendment_5_activation_stop_v3_deleted_offset_zero_based=54621
goal_g_amendment_5_activation_stop_v3_deleted_byte=0x5c
goal_g_amendment_5_activation_stop_v3_corrected_line_sha256=107cbbb11918f7bf6144f32a718ca10b6eabb328100721dc42dfbef0248393e1
goal_g_amendment_5_activation_stop_control_to_v3_changed_files=1
goal_g_amendment_5_activation_stop_control_to_v3_deleted_bytes=1
goal_g_amendment_5_activation_stop_changed_file_count=5
goal_g_amendment_5_activation_stop_changed_files=SELF-TEST-DESIGN.md,SELF-TEST-SCHEMA.md,construct-self-test.preview.sh,run-attempt.sh,validators.sh
goal_g_amendment_5_activation_stop_unchanged_file_count=5
goal_g_amendment_5_activation_stop_unchanged_files=commands.tsv,inventory.preview.sh,run-phase0-replay.preview.sh,source-reattest.preview.sh,summarize-baseline.preview.sh
goal_g_amendment_5_activation_stop_review_1_result=fail-input-authentication
goal_g_amendment_5_activation_stop_review_1_reviewer=root-g5-provenance-review-1
goal_g_amendment_5_activation_stop_review_1_session=root-g5-provenance-review-1-20260802T-current
goal_g_amendment_5_activation_stop_review_1_expected_control_forensic_inventory_sha256=4f2039164a8403a0ff9692f358fb513fb2b2e209ee3e179a0bff04d24814cd6e
goal_g_amendment_5_activation_stop_review_1_actual_control_forensic_inventory_sha256=2f05254afe092859bcae96711f993cfd88165820896b0287441f2251206b9d51
goal_g_amendment_5_activation_stop_review_1_v2_forensic_inventory_sha256=062c306df0e3a5b331be79df841dc98eefeed1a9d1a5b899968bae662d59f0cb
goal_g_amendment_5_activation_stop_review_1_scratch_state=absent-never-created
goal_g_amendment_5_activation_stop_review_2_result=cancelled-before-scratch-after-review-1-stop
goal_g_amendment_5_activation_stop_review_2_scratch_state=absent-never-created
goal_g_amendment_5_activation_stop_preview_v2_state=absent-not-created
goal_g_amendment_5_activation_stop_preview_invocation_count=0
goal_g_amendment_5_activation_stop_official_bundle_state=absent-not-created
goal_g_amendment_5_activation_stop_official_evidence_state=absent-not-created
goal_g_amendment_5_activation_stop_official_runtime_state=absent-not-created
goal_g_amendment_5_activation_stop_g3_created=false
goal_g_amendment_5_activation_stop_phase0_started=false
goal_g_amendment_5_activation_stop_cargo_invoked=false
goal_g_amendment_5_activation_stop_test_or_benchmark_invoked=false
goal_g_amendment_5_activation_stop_public_fetch_invoked=false
goal_g_amendment_5_activation_stop_network_invoked=false
goal_g_amendment_5_activation_stop_real_credentials_loaded=false
goal_g_amendment_5_activation_stop_authenticated_external_request_sent=false
goal_g_amendment_5_activation_stop_real_polygon_rpc_request_sent=false
goal_g_amendment_5_activation_stop_real_order_submitted=false
goal_g_amendment_5_activation_stop_production_order_entry_authorized=false
goal_g_amendment_5_activation_stop_historical_attempt_relabelled=false
goal_g_amendment_5_activation_stop_retry_authorized=false
goal_g_amendment_5_activation_stop_next_authority=new-reviewed-user-authorized-amendment
goal_g_amendment_5_activation_stop_push_authorized=false
```

## User-Authorized Amendment 6 — 2026-08-02

The user authorized the narrow forensic-inventory recovery contract in
`docs/polymarket-authenticated-execution-goal-g-amendment-6.md`. Amendment 5
remains honestly stopped: its first review failed input authentication before
scratch creation, its second review was cancelled, v3 was not approved, and
preview-v2 never ran. Every retained Amendment 5 byte and both historical
terminal records remain immutable.

Amendment 6 freezes the exact inventory encoding, requires two independent
read-only implementations to reproduce the v2 and v3 identities before any
new path is created, and authorizes only a provenance-rebound v4 derived from
authenticated v3. The existing Goal G-R `A6` alias is not reused; this
authorization commit is `G6_AUTH`, and the retained Amendment 5 stop is
`G5_STOP`.

Only if both v4 reviews, the fresh one-shot preview, both post-preview reviews,
fresh official construction, both official reviews, and sealing are green may
the original Goal G activation continue. Amendment 5 stays stopped even on
success; `G3` transitions only Amendment 3 and Amendment 6.

```text
goal_g_amendment_6_status=activation-stopped-inactive
goal_g_amendment_6_schema=goal-g-amendment-6-v1
goal_g_amendment_6_parent_alias=G5_STOP
goal_g_amendment_6_parent_commit=dab6a252ffe25bb390da12a0459125cbeeacb7de
goal_g_amendment_6_parent_tree=1f50b0d1ed8857de134b092848cb36e8e6bc8ff8
goal_g_amendment_6_parent_parent=ba3b666d95d8097f60f8fc33a12b9844115edca8
goal_g_amendment_6_parent_subject=docs: record goal g amendment 5 activation stop
goal_g_amendment_6_parent_path_count=1
goal_g_amendment_6_parent_paths=docs/polymarket-authenticated-execution-goal-g-handoff.md
goal_g_amendment_6_parent_handoff_sha256=4f8d9cd5663e2e051ce0e34a73f06a154dce88c65c1f41894d54af1aaa3c41b4
goal_g_amendment_6_contract_path=docs/polymarket-authenticated-execution-goal-g-amendment-6.md
goal_g_amendment_6_contract_sha256=b398d31151029bbc6c530082ff29502f7c899cf273787dcf4fbf2b356bbb180f
goal_g_amendment_6_authorization_alias=G6_AUTH
goal_g_amendment_6_authorization_subject=docs: authorize goal g amendment 6 forensic inventory recovery
goal_g_amendment_6_authorization_path_count=2
goal_g_amendment_6_authorization_paths=docs/polymarket-authenticated-execution-goal-g-amendment-6.md,docs/polymarket-authenticated-execution-goal-g-handoff.md
goal_g_amendment_6_activation_subject=docs: activate goal g amendment 3
goal_g_amendment_6_phase0_subject=docs: qualify goal g amendment 3 phase 0
goal_g_amendment_6_activation_stop_subject=docs: record goal g amendment 6 activation stop
goal_g_amendment_6_lineage=R6->S4->T->A5->G5_STOP->G6_AUTH->G3->P0
goal_g_amendment_6_activation_a3_status_from=activation-stopped-inactive
goal_g_amendment_6_activation_a3_status_to=active-phase0
goal_g_amendment_6_activation_a5_status_retained=activation-stopped-inactive
goal_g_amendment_6_activation_g6_status_from=authorized-inactive
goal_g_amendment_6_activation_g6_status_to=activation-complete-phase0-active
goal_g_amendment_6_forensic_schema=rel-nul-type-nul-mode4-nul-uid-nul-gid-nul-nlink-nul-size-nul-file-sha256-or-directory-dash-lf
goal_g_amendment_6_forensic_order=raw-relative-path-bytes-root-dot-included
goal_g_amendment_6_forensic_metadata_source=lstat
goal_g_amendment_6_forensic_directory_payload=dash
goal_g_amendment_6_forensic_file_payload=lowercase-sha256-of-exact-file-bytes
goal_g_amendment_6_forensic_two_record_vector_bytes=116
goal_g_amendment_6_forensic_two_record_vector_sha256=63ed0e2d6f3f43abc06cce1dd215d166131f25132b645ec6c027b50d1629c9c0
goal_g_amendment_6_forensic_reviewer_count=2
goal_g_amendment_6_forensic_distinct_reviewers_required=true
goal_g_amendment_6_forensic_distinct_sessions_required=true
goal_g_amendment_6_v2_forensic_stream_bytes=1151
goal_g_amendment_6_v2_entry_count_including_root=11
goal_g_amendment_6_v2_directory_count_including_root=1
goal_g_amendment_6_v2_regular_file_count=10
goal_g_amendment_6_v2_regular_bytes=1038407
goal_g_amendment_6_v2_component_manifest_rows=10
goal_g_amendment_6_v2_component_manifest_bytes=933
goal_g_amendment_6_v2_component_manifest_sha256=82fa2de7bc468a5a60fa3f795f336d621515557a5ee21b9828b09d1d526cf4a8
goal_g_amendment_6_v2_forensic_inventory_sha256=062c306df0e3a5b331be79df841dc98eefeed1a9d1a5b899968bae662d59f0cb
goal_g_amendment_6_v3_root=/var/tmp/reap-g3-draft-v3
goal_g_amendment_6_v3_state=retained-non-authoritative-not-invoked
goal_g_amendment_6_v3_forensic_stream_bytes=1151
goal_g_amendment_6_v3_entry_count_including_root=11
goal_g_amendment_6_v3_directory_count_including_root=1
goal_g_amendment_6_v3_regular_file_count=10
goal_g_amendment_6_v3_regular_bytes=1055725
goal_g_amendment_6_v3_component_manifest_rows=10
goal_g_amendment_6_v3_component_manifest_bytes=933
goal_g_amendment_6_v3_component_manifest_sha256=710ab62d5dbe846b21df74a4d78ee3f12d2a1883a22662d256bf751d411bc451
goal_g_amendment_6_v3_forensic_inventory_sha256=9ab5b0695e60cf47fc41e9d2e110b291c56b856d80cbfad36b9d41b6a5c7d233
goal_g_amendment_6_root_record_bytes=28
goal_g_amendment_6_root_record_sha256=5c5f2aa15f151a1c1fd8285ee13c42e968e17889c99ad85c06e544080824ba81
goal_g_amendment_6_commands_record_bytes=102
goal_g_amendment_6_commands_record_sha256=3ca42fa79530d356d42a05c2324d7ea09132e0d8ae5882e9285e7cff5abd3bea
goal_g_amendment_6_rejected_control_forensic_inventory_sha256=4f2039164a8403a0ff9692f358fb513fb2b2e209ee3e179a0bff04d24814cd6e
goal_g_amendment_6_rejected_v3_forensic_inventory_sha256=cf5eb07c85af4721c586c90a778a2fb902c32d7bcd274863206ec13a193e63c
goal_g_amendment_6_correct_control_forensic_inventory_sha256=2f05254afe092859bcae96711f993cfd88165820896b0287441f2251206b9d51
goal_g_amendment_6_v4_root=/var/tmp/reap-g3-draft-v4
goal_g_amendment_6_v4_patch=/var/tmp/reap-g3-draft-v4-provenance.patch
goal_g_amendment_6_v4_changed_file_count=5
goal_g_amendment_6_v4_changed_files=SELF-TEST-DESIGN.md,SELF-TEST-SCHEMA.md,construct-self-test.preview.sh,run-attempt.sh,validators.sh
goal_g_amendment_6_v4_unchanged_file_count=5
goal_g_amendment_6_v4_unchanged_files=commands.tsv,inventory.preview.sh,run-phase0-replay.preview.sh,source-reattest.preview.sh,summarize-baseline.preview.sh
goal_g_amendment_6_v4_corrected_line_sha256=107cbbb11918f7bf6144f32a718ca10b6eabb328100721dc42dfbef0248393e1
goal_g_amendment_6_v4_combined_fixture_body_bytes=3025
goal_g_amendment_6_v4_combined_fixture_body_sha256=7c1f62087f71572805426f0209c536e8c10310596292ac32e709974f05c8fa70
goal_g_amendment_6_v4_validator_redirection_manifest_rows=179
goal_g_amendment_6_v4_fixture_case_count=116
goal_g_amendment_6_v4_fixture_subcase_count=1240
goal_g_amendment_6_v4_patch_sections=5
goal_g_amendment_6_v4_control_path_authorized=false
goal_g_amendment_6_v4_root_mode=0700
goal_g_amendment_6_v4_root_uid=1000
goal_g_amendment_6_v4_root_gid=1000
goal_g_amendment_6_v4_root_nlink=2
goal_g_amendment_6_v4_child_count=10
goal_g_amendment_6_v4_regular_file_modes=9x0664,1x0700
goal_g_amendment_6_v4_component_manifest_schema=sha256-tab-bytes-tab-basename-lf-ordered-by-raw-basename-bytes
goal_g_amendment_6_v4_redirection_manifest_rows=179
goal_g_amendment_6_v4_redirection_manifest_normalized_bytes=17554
goal_g_amendment_6_v4_redirection_manifest_normalized_sha256=b2734fc048d6e536cd2c4fdabe6975f5da77cee1b061a28e4eac97d4e51ef924
goal_g_amendment_6_repository_fact_fields=g5_stop_commit,g5_stop_tree,g5_stop_parent,g5_stop_subject,g5_stop_handoff_sha256,g6_auth_commit,g6_auth_tree,g6_auth_parent,g6_auth_subject,g6_auth_contract_sha256,g6_auth_handoff_sha256
goal_g_amendment_6_phase0_meta_fields=g5_stop_commit,g5_stop_tree,g5_stop_parent,g5_stop_subject,g5_stop_handoff_sha256,g6_auth_commit,g6_auth_tree,g6_auth_parent,g6_auth_subject,g6_auth_contract_sha256,g6_auth_handoff_sha256
goal_g_amendment_6_activation_handoff_prefixes=goal_g_amendment_6_g5_stop_,goal_g_amendment_6_g6_auth_
goal_g_amendment_6_v4_review_1_scratch=/var/tmp/reap-g3-draft-v4-review-1-scratch
goal_g_amendment_6_v4_review_2_scratch=/var/tmp/reap-g3-draft-v4-review-2-scratch
goal_g_amendment_6_v4_review_scratch_create_limit_each=1
goal_g_amendment_6_v4_review_scratch_preserve_on_failure=true
goal_g_amendment_6_v4_review_scratch_remove_authorized_after_pass=true
goal_g_amendment_6_v4_review_count=2
goal_g_amendment_6_v4_review_distinct_reviewers_required=true
goal_g_amendment_6_v4_review_distinct_sessions_required=true
goal_g_amendment_6_new_preview_root=target/tmp/goal-g-amendment-3-preview-v3
goal_g_amendment_6_new_preview_invocation_limit=1
goal_g_amendment_6_new_preview_argv_count=5
goal_g_amendment_6_new_preview_argv_nul_bytes=145
goal_g_amendment_6_new_preview_argv_nul_sha256=72828d50c317fab81c471ed8020c8580d9a17c1dabba21a2fc11dbe138e941d7
goal_g_amendment_6_new_preview_review_count=2
goal_g_amendment_6_new_preview_distinct_reviewers_required=true
goal_g_amendment_6_new_preview_distinct_sessions_required=true
goal_g_amendment_6_official_review_count=2
goal_g_amendment_6_official_distinct_reviewers_required=true
goal_g_amendment_6_official_distinct_sessions_required=true
goal_g_amendment_6_official_bundle_root=target/tmp/goal-g-amendment-3-recorder-bundle
goal_g_amendment_6_official_evidence_root=target/tmp/goal-g-phase0-amendment-3
goal_g_amendment_6_official_runtime_root=target/tmp/goal-g-amendment-3-runtime
goal_g_amendment_6_official_bundle_state=absent-not-created
goal_g_amendment_6_official_evidence_state=absent-not-created
goal_g_amendment_6_official_runtime_state=absent-not-created
goal_g_amendment_6_pre_g3_cargo_authorized=false
goal_g_amendment_6_pre_g3_test_or_benchmark_authorized=false
goal_g_amendment_6_pre_g3_public_fetch_authorized=false
goal_g_amendment_6_pre_g3_network_authorized=false
goal_g_amendment_6_production_order_entry_authorized=false
goal_g_amendment_6_real_credentials_loaded=false
goal_g_amendment_6_authenticated_external_request_sent=false
goal_g_amendment_6_real_polygon_rpc_request_sent=false
goal_g_amendment_6_real_order_submitted=false
goal_g_amendment_6_historical_goal_g_attempt_relabelled=false
goal_g_amendment_6_historical_goal_g_r_equivalence_claimed=false
goal_g_amendment_6_amendment_5_review_retry_authorized=false
goal_g_amendment_6_preview_v2_reuse_authorized=false
goal_g_amendment_6_push_authorized=false
```

## Amendment 6 Terminal Activation Stop — 2026-08-02

Both required post-authorization forensic reviewers passed independently from
exact clean `G6_AUTH`. They reproduced the frozen v2 and v3 component
manifests and inventories, the pure two-record vector, stable metadata cuts,
the exact authorization lineage, and absence of every new path. Neither
review created a report, scratch root, or artifact.

Immediately before the intended v3-to-v4 copy, the executor added an
uncontracted aggregate assertion over sorted v3 component hash rows. Its
invented expected digest was wrong, so the shell exited at that assertion.
The assertion's actual digest is stable and all contract-frozen per-file,
component-manifest, and forensic identities still match. Nevertheless, it ran
as part of the immediate pre-copy authentication sequence. Amendment 6 makes
any authentication failure terminal and permits no retry or continuation.
Two independent read-only contract interpretations therefore required this
stop.

The final adjacent storage preflight and `cp` were never reached. v4, its
patch, both review scratches, preview-v3, and all official paths remain absent.
No constructor, preview, official construction, Cargo command, test,
benchmark, public fetch, network child, credential load, authenticated
request, Polygon RPC, or order entry occurred.

```text
goal_g_amendment_6_activation_stop_status=stopped
goal_g_amendment_6_activation_stop_schema=goal-g-amendment-6-activation-stop-v1
goal_g_amendment_6_activation_stop_stage=post-g6-auth-post-forensic-pass-pre-v4-construction
goal_g_amendment_6_activation_stop_parent_commit=c20a95a3a45caa1cab66f878267469bff59481bf
goal_g_amendment_6_activation_stop_parent_tree=9b19215d6560858adb3fb0427fe92a6e3e928d92
goal_g_amendment_6_activation_stop_parent_parent=dab6a252ffe25bb390da12a0459125cbeeacb7de
goal_g_amendment_6_activation_stop_parent_subject=docs: authorize goal g amendment 6 forensic inventory recovery
goal_g_amendment_6_activation_stop_parent_path_count=2
goal_g_amendment_6_activation_stop_parent_paths=docs/polymarket-authenticated-execution-goal-g-amendment-6.md,docs/polymarket-authenticated-execution-goal-g-handoff.md
goal_g_amendment_6_activation_stop_parent_contract_sha256=b398d31151029bbc6c530082ff29502f7c899cf273787dcf4fbf2b356bbb180f
goal_g_amendment_6_activation_stop_parent_handoff_sha256=ffd248a10f20b6d955d81aed83fa769e72d4b745de8c18192b3fb32d56fbecf1
goal_g_amendment_6_activation_stop_tracked_allowlist=docs/polymarket-authenticated-execution-goal-g-handoff.md
goal_g_amendment_6_activation_stop_failed_gate=immediate-pre-copy-authentication-extra-component-row-aggregate-expectation
goal_g_amendment_6_activation_stop_failure_class=executor-added-authentication-expectation-error
goal_g_amendment_6_activation_stop_failure_required_contract_identity_mismatch=false
goal_g_amendment_6_activation_stop_failure_source_corruption=false
goal_g_amendment_6_activation_stop_extra_aggregate_expected_sha256=48bd3e112607431d7b442103921d94c0e65ca0812a15a51d593e7e8f28e34200
goal_g_amendment_6_activation_stop_extra_aggregate_actual_sha256=be8ecd49614fd0a14d3f30f05e7380e55f077c1c912175daa70451ecd3301abc
goal_g_amendment_6_activation_stop_extra_aggregate_contract_field=false
goal_g_amendment_6_activation_stop_extra_aggregate_assertion_status=1
goal_g_amendment_6_activation_stop_final_copy_preflight_reached=false
goal_g_amendment_6_activation_stop_copy_invoked=false
goal_g_amendment_6_activation_stop_forensic_review_1_result=pass
goal_g_amendment_6_activation_stop_forensic_review_1_reviewer=root-g6-forensic-review-1
goal_g_amendment_6_activation_stop_forensic_review_1_session=root-g6-forensic-review-1-20260802
goal_g_amendment_6_activation_stop_forensic_review_1_implementation_sha256=d63920d4332a8b305cb1b5d893a48e1933608a615ae397a72e7b3bc1befb4331
goal_g_amendment_6_activation_stop_forensic_review_1_v2_inventory_sha256=062c306df0e3a5b331be79df841dc98eefeed1a9d1a5b899968bae662d59f0cb
goal_g_amendment_6_activation_stop_forensic_review_1_v3_inventory_sha256=9ab5b0695e60cf47fc41e9d2e110b291c56b856d80cbfad36b9d41b6a5c7d233
goal_g_amendment_6_activation_stop_forensic_review_1_two_record_vector_sha256=63ed0e2d6f3f43abc06cce1dd215d166131f25132b645ec6c027b50d1629c9c0
goal_g_amendment_6_activation_stop_forensic_review_2_result=pass
goal_g_amendment_6_activation_stop_forensic_review_2_reviewer=root-g6-forensic-review-2
goal_g_amendment_6_activation_stop_forensic_review_2_session=root-g6-forensic-review-2-20260802
goal_g_amendment_6_activation_stop_forensic_review_2_implementation_sha256=e4b884cbf4d85be2b36593092bae5dc35c50f1192f30ddba78c5c4e9b39f2fe2
goal_g_amendment_6_activation_stop_forensic_review_2_v2_inventory_sha256=062c306df0e3a5b331be79df841dc98eefeed1a9d1a5b899968bae662d59f0cb
goal_g_amendment_6_activation_stop_forensic_review_2_v3_inventory_sha256=9ab5b0695e60cf47fc41e9d2e110b291c56b856d80cbfad36b9d41b6a5c7d233
goal_g_amendment_6_activation_stop_forensic_review_2_two_record_vector_sha256=63ed0e2d6f3f43abc06cce1dd215d166131f25132b645ec6c027b50d1629c9c0
goal_g_amendment_6_activation_stop_v2_component_manifest_sha256=82fa2de7bc468a5a60fa3f795f336d621515557a5ee21b9828b09d1d526cf4a8
goal_g_amendment_6_activation_stop_v2_forensic_inventory_sha256=062c306df0e3a5b331be79df841dc98eefeed1a9d1a5b899968bae662d59f0cb
goal_g_amendment_6_activation_stop_v3_component_manifest_sha256=710ab62d5dbe846b21df74a4d78ee3f12d2a1883a22662d256bf751d411bc451
goal_g_amendment_6_activation_stop_v3_forensic_inventory_sha256=9ab5b0695e60cf47fc41e9d2e110b291c56b856d80cbfad36b9d41b6a5c7d233
goal_g_amendment_6_activation_stop_v4_state=absent-not-created
goal_g_amendment_6_activation_stop_v4_patch_state=absent-not-created
goal_g_amendment_6_activation_stop_v4_review_1_scratch_state=absent-not-created
goal_g_amendment_6_activation_stop_v4_review_2_scratch_state=absent-not-created
goal_g_amendment_6_activation_stop_preview_v3_state=absent-not-created
goal_g_amendment_6_activation_stop_preview_invocation_count=0
goal_g_amendment_6_activation_stop_official_bundle_state=absent-not-created
goal_g_amendment_6_activation_stop_official_evidence_state=absent-not-created
goal_g_amendment_6_activation_stop_official_runtime_state=absent-not-created
goal_g_amendment_6_activation_stop_g3_created=false
goal_g_amendment_6_activation_stop_phase0_started=false
goal_g_amendment_6_activation_stop_constructor_invoked=false
goal_g_amendment_6_activation_stop_cargo_invoked=false
goal_g_amendment_6_activation_stop_test_or_benchmark_invoked=false
goal_g_amendment_6_activation_stop_public_fetch_invoked=false
goal_g_amendment_6_activation_stop_network_invoked=false
goal_g_amendment_6_activation_stop_real_credentials_loaded=false
goal_g_amendment_6_activation_stop_authenticated_external_request_sent=false
goal_g_amendment_6_activation_stop_real_polygon_rpc_request_sent=false
goal_g_amendment_6_activation_stop_real_order_submitted=false
goal_g_amendment_6_activation_stop_production_order_entry_authorized=false
goal_g_amendment_6_activation_stop_historical_attempt_relabelled=false
goal_g_amendment_6_activation_stop_retry_authorized=false
goal_g_amendment_6_activation_stop_next_authority=new-reviewed-user-authorized-amendment
goal_g_amendment_6_activation_stop_push_authorized=false
```

## User-Authorized Amendment 7 — 2026-08-02

The user authorized the narrow closed pre-copy recovery contract in
`docs/polymarket-authenticated-execution-goal-g-amendment-7.md`. Amendment 6
remains honestly stopped. Its two successful forensic reviews remain
historical Amendment 6 evidence, while its executor-added aggregate failure,
unreached final preflight, and uninvoked copy remain unchanged.

Amendment 7 removes the executor-controlled success gap. One reviewed,
separately hashed launcher performs one new closed current-state
authentication and, after its final content predicate, flows directly through
the retained storage preflight into an exact BusyBox v3-to-v5 copy. It may not
return successful authentication to the executor before the copy begins.

No v4 or preview-v3 namespace is reused. Exact v3 remains the sole source and
control for a provenance-only v5. The Amendment 3, 5, and 6 statuses and all
three historical terminal blocks remain unchanged at authorization.

```text
goal_g_amendment_7_status=activation-stopped-inactive
goal_g_amendment_7_schema=goal-g-amendment-7-v1
goal_g_amendment_7_parent_alias=G6_STOP
goal_g_amendment_7_parent_commit=f06e42623d9680dbe9c2012d6300a32ae17853c5
goal_g_amendment_7_parent_tree=b44895964430bb25d0a6c2c0786cbfcf26c983ec
goal_g_amendment_7_parent_parent=c20a95a3a45caa1cab66f878267469bff59481bf
goal_g_amendment_7_parent_subject=docs: record goal g amendment 6 activation stop
goal_g_amendment_7_parent_path_count=1
goal_g_amendment_7_parent_paths=docs/polymarket-authenticated-execution-goal-g-handoff.md
goal_g_amendment_7_parent_handoff_sha256=3b15c0cdf89bf017d681e5bc89cf581d50431db186b247f77a656dbf57102589
goal_g_amendment_7_contract_path=docs/polymarket-authenticated-execution-goal-g-amendment-7.md
goal_g_amendment_7_contract_sha256=aba29b88604aa2f71b79ee6a8a1b090744740742f796252b38a1d4eafa6fe287
goal_g_amendment_7_authorization_alias=G7_AUTH
goal_g_amendment_7_authorization_subject=docs: authorize goal g amendment 7 closed pre-copy recovery
goal_g_amendment_7_authorization_path_count=2
goal_g_amendment_7_authorization_paths=docs/polymarket-authenticated-execution-goal-g-amendment-7.md,docs/polymarket-authenticated-execution-goal-g-handoff.md
goal_g_amendment_7_activation_subject=docs: activate goal g amendment 3
goal_g_amendment_7_phase0_subject=docs: qualify goal g amendment 3 phase 0
goal_g_amendment_7_activation_stop_subject=docs: record goal g amendment 7 activation stop
goal_g_amendment_7_lineage=R6->S4->T->A5->G5_STOP->G6_AUTH->G6_STOP->G7_AUTH->G3->P0
goal_g_amendment_7_activation_a3_status_from=activation-stopped-inactive
goal_g_amendment_7_activation_a3_status_to=active-phase0
goal_g_amendment_7_activation_a5_status_retained=activation-stopped-inactive
goal_g_amendment_7_activation_a6_status_retained=activation-stopped-inactive
goal_g_amendment_7_activation_g7_status_from=authorized-inactive
goal_g_amendment_7_activation_g7_status_to=activation-complete-phase0-active
goal_g_amendment_7_bootstrap_interpreter=/usr/bin/python3
goal_g_amendment_7_bootstrap_flags=-I,-S,-c
goal_g_amendment_7_bootstrap_bytes=3623
goal_g_amendment_7_bootstrap_sha256=7fb4bb36a4a5a666c60d89c62037184b853daf077a7c0a84971163b88166d633
goal_g_amendment_7_precopy_launcher_interpreter=/bin/bash
goal_g_amendment_7_precopy_launcher_flags=--noprofile,--norc,-c
goal_g_amendment_7_precopy_launcher_bytes=31518
goal_g_amendment_7_precopy_launcher_sha256=d42320de72049a32d84710ae0b2944ee6fcb656b8910bd44e67f987a3ad73934
goal_g_amendment_7_precopy_launcher_environment=PATH=/usr/bin:/bin,LC_ALL=C,LANG=C,TZ=UTC,GIT_OPTIONAL_LOCKS=0,GIT_NO_REPLACE_OBJECTS=1
goal_g_amendment_7_precopy_launcher_assertion_count=26
goal_g_amendment_7_source_review_1_result=pass
goal_g_amendment_7_source_review_1_reviewer=g5-contract-design-review-1
goal_g_amendment_7_source_review_1_session=g5-contract-design-a7-20260802
goal_g_amendment_7_source_review_1_contract_sha256=aba29b88604aa2f71b79ee6a8a1b090744740742f796252b38a1d4eafa6fe287
goal_g_amendment_7_source_review_1_bootstrap_sha256=7fb4bb36a4a5a666c60d89c62037184b853daf077a7c0a84971163b88166d633
goal_g_amendment_7_source_review_1_precopy_launcher_sha256=d42320de72049a32d84710ae0b2944ee6fcb656b8910bd44e67f987a3ad73934
goal_g_amendment_7_source_review_2_result=pass
goal_g_amendment_7_source_review_2_reviewer=g5-repo-boundary-review-2
goal_g_amendment_7_source_review_2_session=g5-repo-boundary-a7-20260802
goal_g_amendment_7_source_review_2_contract_sha256=aba29b88604aa2f71b79ee6a8a1b090744740742f796252b38a1d4eafa6fe287
goal_g_amendment_7_source_review_2_bootstrap_sha256=7fb4bb36a4a5a666c60d89c62037184b853daf077a7c0a84971163b88166d633
goal_g_amendment_7_source_review_2_precopy_launcher_sha256=d42320de72049a32d84710ae0b2944ee6fcb656b8910bd44e67f987a3ad73934
goal_g_amendment_7_precopy_launcher_success_returns_before_copy=false
goal_g_amendment_7_precopy_launcher_report_created=false
goal_g_amendment_7_precopy_launcher_extra_predicate_authorized=false
goal_g_amendment_7_precopy_launcher_extra_digest_authorized=false
goal_g_amendment_7_precopy_launcher_forbidden_a6_expected_aggregate_sha256=48bd3e112607431d7b442103921d94c0e65ca0812a15a51d593e7e8f28e34200
goal_g_amendment_7_precopy_launcher_forbidden_a6_actual_aggregate_sha256=be8ecd49614fd0a14d3f30f05e7380e55f077c1c912175daa70451ecd3301abc
goal_g_amendment_7_forensic_schema=rel-nul-type-nul-mode4-nul-uid-nul-gid-nul-nlink-nul-size-nul-file-sha256-or-directory-dash-lf
goal_g_amendment_7_v2_component_manifest_sha256=82fa2de7bc468a5a60fa3f795f336d621515557a5ee21b9828b09d1d526cf4a8
goal_g_amendment_7_v2_forensic_inventory_sha256=062c306df0e3a5b331be79df841dc98eefeed1a9d1a5b899968bae662d59f0cb
goal_g_amendment_7_control_component_manifest_sha256=50f7de09cb5f19de4a9f1375a4a4a5a1acf40b4f831e65004a0651664df3db61
goal_g_amendment_7_control_forensic_inventory_sha256=2f05254afe092859bcae96711f993cfd88165820896b0287441f2251206b9d51
goal_g_amendment_7_v3_component_manifest_sha256=710ab62d5dbe846b21df74a4d78ee3f12d2a1883a22662d256bf751d411bc451
goal_g_amendment_7_v3_forensic_inventory_sha256=9ab5b0695e60cf47fc41e9d2e110b291c56b856d80cbfad36b9d41b6a5c7d233
goal_g_amendment_7_a5_patch_bytes=56207
goal_g_amendment_7_a5_patch_sha256=fc340abca04400d0aff3fce73dcf5a309bdfaec838fc22bfd82aca5a46f55daf
goal_g_amendment_7_failed_preview_forensic_inventory_sha256=82ac222e4932320ad14ce7ef7800bd8e39a373deaf6ce8205a9ab9ccbfd11747
goal_g_amendment_7_failed_preview_regular_manifest_sha256=a86c192658af2e4edef79c70ae4f89e842ac9f57ba278f1b8c0ff835defe2df9
goal_g_amendment_7_forensic_two_record_vector_bytes=116
goal_g_amendment_7_forensic_two_record_vector_sha256=63ed0e2d6f3f43abc06cce1dd215d166131f25132b645ec6c027b50d1629c9c0
goal_g_amendment_7_copy_source=/var/tmp/reap-g3-draft-v3
goal_g_amendment_7_v5_root=/var/tmp/reap-g3-draft-v5
goal_g_amendment_7_v5_patch=/var/tmp/reap-g3-draft-v5-provenance.patch
goal_g_amendment_7_v5_review_1_scratch=/var/tmp/reap-g3-draft-v5-review-1-scratch
goal_g_amendment_7_v5_review_2_scratch=/var/tmp/reap-g3-draft-v5-review-2-scratch
goal_g_amendment_7_v5_changed_file_count=5
goal_g_amendment_7_v5_changed_files=SELF-TEST-DESIGN.md,SELF-TEST-SCHEMA.md,construct-self-test.preview.sh,run-attempt.sh,validators.sh
goal_g_amendment_7_v5_unchanged_file_count=5
goal_g_amendment_7_v5_unchanged_files=commands.tsv,inventory.preview.sh,run-phase0-replay.preview.sh,source-reattest.preview.sh,summarize-baseline.preview.sh
goal_g_amendment_7_v5_patch_sections=5
goal_g_amendment_7_v5_control_path_authorized=false
goal_g_amendment_7_copy_argv_count=6
goal_g_amendment_7_copy_argv_nul_bytes=74
goal_g_amendment_7_copy_argv_nul_sha256=18d707d79567219d0ca519b9e8de54a56d595682de8b1b2f739792c76f15806d
goal_g_amendment_7_busybox_sha256=c2f279d1d5640a0f327890d41cad594c0f059f3fed3f96dd72fdcc4f5e18ec02
goal_g_amendment_7_repository_fact_fields=g5_stop_commit,g5_stop_tree,g5_stop_parent,g5_stop_subject,g5_stop_handoff_sha256,g6_auth_commit,g6_auth_tree,g6_auth_parent,g6_auth_subject,g6_auth_contract_sha256,g6_auth_handoff_sha256,g6_stop_commit,g6_stop_tree,g6_stop_parent,g6_stop_subject,g6_stop_handoff_sha256,g7_auth_commit,g7_auth_tree,g7_auth_parent,g7_auth_subject,g7_auth_contract_sha256,g7_auth_handoff_sha256
goal_g_amendment_7_phase0_meta_fields=g5_stop_commit,g5_stop_tree,g5_stop_parent,g5_stop_subject,g5_stop_handoff_sha256,g6_auth_commit,g6_auth_tree,g6_auth_parent,g6_auth_subject,g6_auth_contract_sha256,g6_auth_handoff_sha256,g6_stop_commit,g6_stop_tree,g6_stop_parent,g6_stop_subject,g6_stop_handoff_sha256,g7_auth_commit,g7_auth_tree,g7_auth_parent,g7_auth_subject,g7_auth_contract_sha256,g7_auth_handoff_sha256
goal_g_amendment_7_new_preview_root=target/tmp/goal-g-amendment-3-preview-v4
goal_g_amendment_7_new_preview_invocation_limit=1
goal_g_amendment_7_new_preview_argv_count=5
goal_g_amendment_7_new_preview_argv_nul_bytes=145
goal_g_amendment_7_new_preview_argv_nul_sha256=461c5989b5ccc0b2a4931051a7f215ad2fc7088f3945876048dcb7b860837e73
goal_g_amendment_7_retained_no_cargo_bootstrap_required=true
goal_g_amendment_7_v5_review_count=2
goal_g_amendment_7_preview_review_count=2
goal_g_amendment_7_official_review_count=2
goal_g_amendment_7_distinct_reviewers_and_sessions_required=true
goal_g_amendment_7_official_bundle_root=target/tmp/goal-g-amendment-3-recorder-bundle
goal_g_amendment_7_official_evidence_root=target/tmp/goal-g-phase0-amendment-3
goal_g_amendment_7_official_runtime_root=target/tmp/goal-g-amendment-3-runtime
goal_g_amendment_7_official_bundle_state=absent-not-created
goal_g_amendment_7_official_evidence_state=absent-not-created
goal_g_amendment_7_official_runtime_state=absent-not-created
goal_g_amendment_7_pre_g3_cargo_authorized=false
goal_g_amendment_7_pre_g3_test_or_benchmark_authorized=false
goal_g_amendment_7_pre_g3_public_fetch_authorized=false
goal_g_amendment_7_pre_g3_network_authorized=false
goal_g_amendment_7_production_order_entry_authorized=false
goal_g_amendment_7_real_credentials_loaded=false
goal_g_amendment_7_authenticated_external_request_sent=false
goal_g_amendment_7_real_polygon_rpc_request_sent=false
goal_g_amendment_7_real_order_submitted=false
goal_g_amendment_7_historical_goal_g_attempt_relabelled=false
goal_g_amendment_7_historical_goal_g_r_equivalence_claimed=false
goal_g_amendment_7_amendment_6_retry_or_completion_claimed=false
goal_g_amendment_7_v4_or_preview_v3_reuse_authorized=false
goal_g_amendment_7_push_authorized=false
```

## Amendment 7 Terminal Activation Stop — 2026-08-02

Both pre-authorization source reviews passed the exact frozen contract,
bootstrap, and launcher. Exact `G7_AUTH` was committed, and the canonical
launcher then passed all 26 assertions against clean `G7_AUTH`. It emitted the
exact authorization identities and replaced itself with the exact BusyBox
v3-to-v5 copy. The copy exited zero. Fresh v5 was observed byte-identical to
v3 with the frozen component manifest and forensic inventory.

The first post-copy verification command, however, placed one exact storage
preflight before a sequence of three external children: `git status`, `stat`,
and a child-free Python inventory verifier. The preflight was adjacent to
`git status`, but no new preflight ran immediately before `stat`. Amendment 7
retains Amendment 4's mandatory per-external-child preflight boundary. The
successful read-only result cannot cure that sequencing violation, and the
lineage must not continue into v5 construction or review.

Additional read-only inspection began before the violation was recognized.
No v5 byte was edited or invoked; no patch, review scratch, preview, official
root, constructor, Cargo command, test, benchmark, public fetch, network
child, credential load, authenticated request, Polygon RPC, or order entry
occurred. The exact copied v5 is preserved as non-authoritative diagnostic
evidence. A later attempt requires another reviewed, user-authorized
amendment.

```text
goal_g_amendment_7_activation_stop_status=stopped
goal_g_amendment_7_activation_stop_schema=goal-g-amendment-7-activation-stop-v1
goal_g_amendment_7_activation_stop_stage=post-g7-auth-post-copy-pre-v5-construction
goal_g_amendment_7_activation_stop_parent_commit=32f449d3ff3db3043f3547105b9f7e1965289080
goal_g_amendment_7_activation_stop_parent_tree=4a23e8894ee236b206f9134dfb7959eed91ab7dc
goal_g_amendment_7_activation_stop_parent_parent=f06e42623d9680dbe9c2012d6300a32ae17853c5
goal_g_amendment_7_activation_stop_parent_subject=docs: authorize goal g amendment 7 closed pre-copy recovery
goal_g_amendment_7_activation_stop_parent_path_count=2
goal_g_amendment_7_activation_stop_parent_paths=docs/polymarket-authenticated-execution-goal-g-amendment-7.md,docs/polymarket-authenticated-execution-goal-g-handoff.md
goal_g_amendment_7_activation_stop_parent_contract_sha256=aba29b88604aa2f71b79ee6a8a1b090744740742f796252b38a1d4eafa6fe287
goal_g_amendment_7_activation_stop_parent_handoff_sha256=ee464339c72e0b6a462141a69b79cba168f8adc59310e916c1927e8dbe3f3543
goal_g_amendment_7_activation_stop_tracked_allowlist=docs/polymarket-authenticated-execution-goal-g-handoff.md
goal_g_amendment_7_activation_stop_failed_gate=post-copy-per-external-child-storage-preflight-boundary
goal_g_amendment_7_activation_stop_failure_class=executor-storage-boundary-sequencing-error
goal_g_amendment_7_activation_stop_first_nonconforming_child=stat
goal_g_amendment_7_activation_stop_first_nonconforming_child_mutating=false
goal_g_amendment_7_activation_stop_first_post_copy_sequence=git-status,stat,python3-inventory
goal_g_amendment_7_activation_stop_first_post_copy_sequence_preflight_count=1
goal_g_amendment_7_activation_stop_required_preflight_scope=immediately-before-every-external-child
goal_g_amendment_7_activation_stop_additional_read_only_inspection_after_violation=true
goal_g_amendment_7_activation_stop_canonical_authentication_result=pass
goal_g_amendment_7_activation_stop_canonical_authentication_assertion_count=26
goal_g_amendment_7_activation_stop_g7_auth_commit=32f449d3ff3db3043f3547105b9f7e1965289080
goal_g_amendment_7_activation_stop_g7_auth_tree=4a23e8894ee236b206f9134dfb7959eed91ab7dc
goal_g_amendment_7_activation_stop_g7_auth_parent=f06e42623d9680dbe9c2012d6300a32ae17853c5
goal_g_amendment_7_activation_stop_g7_auth_subject=docs: authorize goal g amendment 7 closed pre-copy recovery
goal_g_amendment_7_activation_stop_g7_auth_contract_sha256=aba29b88604aa2f71b79ee6a8a1b090744740742f796252b38a1d4eafa6fe287
goal_g_amendment_7_activation_stop_g7_auth_handoff_sha256=ee464339c72e0b6a462141a69b79cba168f8adc59310e916c1927e8dbe3f3543
goal_g_amendment_7_activation_stop_copy_invoked=true
goal_g_amendment_7_activation_stop_copy_exit=0
goal_g_amendment_7_activation_stop_copy_argv_nul_sha256=18d707d79567219d0ca519b9e8de54a56d595682de8b1b2f739792c76f15806d
goal_g_amendment_7_activation_stop_v5_root=/var/tmp/reap-g3-draft-v5
goal_g_amendment_7_activation_stop_v5_state=retained-non-authoritative-exact-copy-not-edited-not-invoked
goal_g_amendment_7_activation_stop_v5_root_dev=66305
goal_g_amendment_7_activation_stop_v5_root_inode=310596
goal_g_amendment_7_activation_stop_v5_root_mode=0700
goal_g_amendment_7_activation_stop_v5_root_uid=1000
goal_g_amendment_7_activation_stop_v5_root_gid=1000
goal_g_amendment_7_activation_stop_v5_root_nlink=2
goal_g_amendment_7_activation_stop_v5_root_size=4096
goal_g_amendment_7_activation_stop_v5_regular_bytes=1055725
goal_g_amendment_7_activation_stop_v5_component_manifest_rows=10
goal_g_amendment_7_activation_stop_v5_component_manifest_bytes=933
goal_g_amendment_7_activation_stop_v5_component_manifest_sha256=710ab62d5dbe846b21df74a4d78ee3f12d2a1883a22662d256bf751d411bc451
goal_g_amendment_7_activation_stop_v5_forensic_stream_bytes=1151
goal_g_amendment_7_activation_stop_v5_forensic_inventory_sha256=9ab5b0695e60cf47fc41e9d2e110b291c56b856d80cbfad36b9d41b6a5c7d233
goal_g_amendment_7_activation_stop_v5_patch_state=absent-not-created
goal_g_amendment_7_activation_stop_v5_review_1_scratch_state=absent-not-created
goal_g_amendment_7_activation_stop_v5_review_2_scratch_state=absent-not-created
goal_g_amendment_7_activation_stop_preview_v4_state=absent-not-created
goal_g_amendment_7_activation_stop_preview_invocation_count=0
goal_g_amendment_7_activation_stop_official_bundle_state=absent-not-created
goal_g_amendment_7_activation_stop_official_evidence_state=absent-not-created
goal_g_amendment_7_activation_stop_official_runtime_state=absent-not-created
goal_g_amendment_7_activation_stop_g3_created=false
goal_g_amendment_7_activation_stop_phase0_started=false
goal_g_amendment_7_activation_stop_constructor_invoked=false
goal_g_amendment_7_activation_stop_cargo_invoked=false
goal_g_amendment_7_activation_stop_test_or_benchmark_invoked=false
goal_g_amendment_7_activation_stop_public_fetch_invoked=false
goal_g_amendment_7_activation_stop_network_invoked=false
goal_g_amendment_7_activation_stop_real_credentials_loaded=false
goal_g_amendment_7_activation_stop_authenticated_external_request_sent=false
goal_g_amendment_7_activation_stop_real_polygon_rpc_request_sent=false
goal_g_amendment_7_activation_stop_real_order_submitted=false
goal_g_amendment_7_activation_stop_production_order_entry_authorized=false
goal_g_amendment_7_activation_stop_historical_attempt_relabelled=false
goal_g_amendment_7_activation_stop_retry_authorized=false
goal_g_amendment_7_activation_stop_next_authority=new-reviewed-user-authorized-amendment
goal_g_amendment_7_activation_stop_push_authorized=false
```

## User-Authorized Amendment 8 — 2026-08-02

The user authorized the narrow per-child-preflight recovery contract in
`docs/polymarket-authenticated-execution-goal-g-amendment-8.md`. Amendment 7
and its exact v5 copy remain terminal evidence: v5 is not a source, control,
preview input, or promotable artifact. Exact v3 remains the sole source for a
fresh v6.

Two distinct preauthorization reviews passed the same frozen contract,
bootstrap, and supervisor launcher. The launcher keeps control across copy,
binds the same contract and handoff before and after it, post-verifies v3, v5,
and v6, then separately rechecks HEAD, tree, and clean status before releasing
success. Authorization does not activate Goal G and does not authorize a push.

```text
goal_g_amendment_8_status=activation-stopped-inactive
goal_g_amendment_8_schema=goal-g-amendment-8-v1
goal_g_amendment_8_parent_alias=G7_STOP
goal_g_amendment_8_parent_commit=49210315169fa7ec3e3c02b4e70a745105bf9476
goal_g_amendment_8_parent_tree=4e6657c3de48726e73157f35d1b14bb695bdca59
goal_g_amendment_8_parent_parent=32f449d3ff3db3043f3547105b9f7e1965289080
goal_g_amendment_8_parent_subject=docs: record goal g amendment 7 activation stop
goal_g_amendment_8_parent_path_count=1
goal_g_amendment_8_parent_paths=docs/polymarket-authenticated-execution-goal-g-handoff.md
goal_g_amendment_8_parent_handoff_bytes=126290
goal_g_amendment_8_parent_handoff_sha256=31dfeb5f9b872a6c57d7318bed6763d882d57885ab3c30e625e714c075442ef8
goal_g_amendment_8_contract_path=docs/polymarket-authenticated-execution-goal-g-amendment-8.md
goal_g_amendment_8_contract_bytes=48001
goal_g_amendment_8_contract_sha256=ca8a45bd372e6cb617d88d1b39e13e3f395bdf1bf5ad0280a2a97e98ab3cc72a
goal_g_amendment_8_authorization_alias=G8_AUTH
goal_g_amendment_8_authorization_subject=docs: authorize goal g amendment 8 per-child preflight recovery
goal_g_amendment_8_authorization_path_count=2
goal_g_amendment_8_authorization_paths=docs/polymarket-authenticated-execution-goal-g-amendment-8.md,docs/polymarket-authenticated-execution-goal-g-handoff.md
goal_g_amendment_8_activation_subject=docs: activate goal g amendment 3
goal_g_amendment_8_phase0_subject=docs: qualify goal g amendment 3 phase 0
goal_g_amendment_8_activation_stop_subject=docs: record goal g amendment 8 activation stop
goal_g_amendment_8_lineage=R6->S4->T->A5->G5_STOP->G6_AUTH->G6_STOP->G7_AUTH->G7_STOP->G8_AUTH->G3->P0
goal_g_amendment_8_activation_a3_status_from=activation-stopped-inactive
goal_g_amendment_8_activation_a3_status_to=active-phase0
goal_g_amendment_8_activation_a5_status_retained=activation-stopped-inactive
goal_g_amendment_8_activation_a6_status_retained=activation-stopped-inactive
goal_g_amendment_8_activation_a7_status_retained=activation-stopped-inactive
goal_g_amendment_8_activation_g8_status_from=authorized-inactive
goal_g_amendment_8_activation_g8_status_to=activation-complete-phase0-active
goal_g_amendment_8_bootstrap_interpreter=/usr/bin/python3
goal_g_amendment_8_bootstrap_flags=-I,-S,-c
goal_g_amendment_8_bootstrap_bytes=3596
goal_g_amendment_8_bootstrap_sha256=43c25666d22c115845a7e51f57d1d491ea09c50908f5efbb6e2c06a0b5b6026a
goal_g_amendment_8_launcher_interpreter=/bin/bash
goal_g_amendment_8_launcher_flags=--noprofile,--norc,-c
goal_g_amendment_8_launcher_bytes=31211
goal_g_amendment_8_launcher_sha256=067df66f5899a0401455893fff19a6aff6bc115414a44a6a66e583f292751abb
goal_g_amendment_8_launcher_environment=PATH=/usr/bin:/bin,LC_ALL=C,LANG=C,TZ=UTC,GIT_OPTIONAL_LOCKS=0,GIT_NO_REPLACE_OBJECTS=1
goal_g_amendment_8_launcher_external_child_count=17
goal_g_amendment_8_launcher_storage_preflight_count=17
goal_g_amendment_8_launcher_child_ids=repository-root,repository-branch,repository-clean,g8-commit,g8-tree,g8-parent,g8-subject,g8-two-path-delta,g7-stop-object,g7-auth-object,first-parent-lineage,pre-copy-verifier,v3-to-v6-copy,post-copy-verifier,final-g8-commit,final-g8-tree,final-repository-clean
goal_g_amendment_8_launcher_one_fresh_preflight_per_child=true
goal_g_amendment_8_launcher_preflight_reuse_possible=false
goal_g_amendment_8_launcher_unlisted_external_child_possible=false
goal_g_amendment_8_launcher_post_copy_success_gap=false
goal_g_amendment_8_launcher_pre_post_contract_hash_bound=true
goal_g_amendment_8_launcher_pre_post_handoff_hash_bound=true
goal_g_amendment_8_launcher_final_head_tree_status_bound=true
goal_g_amendment_8_exclusive_mutation_interval_required=true
goal_g_amendment_8_review_count=2
goal_g_amendment_8_distinct_reviewers_required=true
goal_g_amendment_8_distinct_sessions_required=true
goal_g_amendment_8_source_review_1_result=pass
goal_g_amendment_8_source_review_1_reviewer=g8-contract-review-1
goal_g_amendment_8_source_review_1_session=g8-contract-review-1-20260802
goal_g_amendment_8_source_review_1_contract_sha256=ca8a45bd372e6cb617d88d1b39e13e3f395bdf1bf5ad0280a2a97e98ab3cc72a
goal_g_amendment_8_source_review_1_bootstrap_sha256=43c25666d22c115845a7e51f57d1d491ea09c50908f5efbb6e2c06a0b5b6026a
goal_g_amendment_8_source_review_1_launcher_sha256=067df66f5899a0401455893fff19a6aff6bc115414a44a6a66e583f292751abb
goal_g_amendment_8_source_review_2_result=pass
goal_g_amendment_8_source_review_2_reviewer=g8-boundary-review-2
goal_g_amendment_8_source_review_2_session=g8-boundary-review-2-20260802
goal_g_amendment_8_source_review_2_contract_sha256=ca8a45bd372e6cb617d88d1b39e13e3f395bdf1bf5ad0280a2a97e98ab3cc72a
goal_g_amendment_8_source_review_2_bootstrap_sha256=43c25666d22c115845a7e51f57d1d491ea09c50908f5efbb6e2c06a0b5b6026a
goal_g_amendment_8_source_review_2_launcher_sha256=067df66f5899a0401455893fff19a6aff6bc115414a44a6a66e583f292751abb
goal_g_amendment_8_v3_root=/var/tmp/reap-g3-draft-v3
goal_g_amendment_8_v3_root_dev=66305
goal_g_amendment_8_v3_root_inode=310585
goal_g_amendment_8_v3_state=sole-source-and-control-not-invoked
goal_g_amendment_8_v5_root=/var/tmp/reap-g3-draft-v5
goal_g_amendment_8_v5_root_dev=66305
goal_g_amendment_8_v5_root_inode=310596
goal_g_amendment_8_v5_state=retained-non-authoritative-exact-copy-not-edited-not-invoked
goal_g_amendment_8_v5_component_manifest_sha256=710ab62d5dbe846b21df74a4d78ee3f12d2a1883a22662d256bf751d411bc451
goal_g_amendment_8_v5_forensic_inventory_sha256=9ab5b0695e60cf47fc41e9d2e110b291c56b856d80cbfad36b9d41b6a5c7d233
goal_g_amendment_8_copy_source=/var/tmp/reap-g3-draft-v3
goal_g_amendment_8_v6_root=/var/tmp/reap-g3-draft-v6
goal_g_amendment_8_v6_patch=/var/tmp/reap-g3-draft-v6-provenance.patch
goal_g_amendment_8_v6_review_1_scratch=/var/tmp/reap-g3-draft-v6-review-1-scratch
goal_g_amendment_8_v6_review_2_scratch=/var/tmp/reap-g3-draft-v6-review-2-scratch
goal_g_amendment_8_v6_initial_state=absent-not-created
goal_g_amendment_8_copy_argv_count=6
goal_g_amendment_8_copy_argv_nul_bytes=74
goal_g_amendment_8_copy_argv_nul_sha256=80f7c5c38d836c51cf7868f9957c0b072c2966faa4767f644ecb38b5b8ecd7ff
goal_g_amendment_8_busybox_sha256=c2f279d1d5640a0f327890d41cad594c0f059f3fed3f96dd72fdcc4f5e18ec02
goal_g_amendment_8_v6_changed_file_count=5
goal_g_amendment_8_v6_changed_files=SELF-TEST-DESIGN.md,SELF-TEST-SCHEMA.md,construct-self-test.preview.sh,run-attempt.sh,validators.sh
goal_g_amendment_8_v6_unchanged_file_count=5
goal_g_amendment_8_v6_unchanged_files=commands.tsv,inventory.preview.sh,run-phase0-replay.preview.sh,source-reattest.preview.sh,summarize-baseline.preview.sh
goal_g_amendment_8_v6_patch_sections=5
goal_g_amendment_8_repository_fact_field_count=33
goal_g_amendment_8_repository_fact_fields=g5_stop_commit,g5_stop_tree,g5_stop_parent,g5_stop_subject,g5_stop_handoff_sha256,g6_auth_commit,g6_auth_tree,g6_auth_parent,g6_auth_subject,g6_auth_contract_sha256,g6_auth_handoff_sha256,g6_stop_commit,g6_stop_tree,g6_stop_parent,g6_stop_subject,g6_stop_handoff_sha256,g7_auth_commit,g7_auth_tree,g7_auth_parent,g7_auth_subject,g7_auth_contract_sha256,g7_auth_handoff_sha256,g7_stop_commit,g7_stop_tree,g7_stop_parent,g7_stop_subject,g7_stop_handoff_sha256,g8_auth_commit,g8_auth_tree,g8_auth_parent,g8_auth_subject,g8_auth_contract_sha256,g8_auth_handoff_sha256
goal_g_amendment_8_phase0_meta_field_count=33
goal_g_amendment_8_phase0_meta_fields=g5_stop_commit,g5_stop_tree,g5_stop_parent,g5_stop_subject,g5_stop_handoff_sha256,g6_auth_commit,g6_auth_tree,g6_auth_parent,g6_auth_subject,g6_auth_contract_sha256,g6_auth_handoff_sha256,g6_stop_commit,g6_stop_tree,g6_stop_parent,g6_stop_subject,g6_stop_handoff_sha256,g7_auth_commit,g7_auth_tree,g7_auth_parent,g7_auth_subject,g7_auth_contract_sha256,g7_auth_handoff_sha256,g7_stop_commit,g7_stop_tree,g7_stop_parent,g7_stop_subject,g7_stop_handoff_sha256,g8_auth_commit,g8_auth_tree,g8_auth_parent,g8_auth_subject,g8_auth_contract_sha256,g8_auth_handoff_sha256
goal_g_amendment_8_candidate_parent=G8_AUTH
goal_g_amendment_8_a6_synonym_authorized=false
goal_g_amendment_8_preview_root=target/tmp/goal-g-amendment-3-preview-v5
goal_g_amendment_8_preview_invocation_limit=1
goal_g_amendment_8_preview_argv_count=5
goal_g_amendment_8_preview_argv_nul_bytes=145
goal_g_amendment_8_preview_argv_nul_sha256=d3485ecae8399e7b6f7bd97ea206a1aeec4ef1f3527d9d82a671cde764e28fa6
goal_g_amendment_8_retained_no_cargo_bootstrap_required=true
goal_g_amendment_8_v6_review_count=2
goal_g_amendment_8_preview_review_count=2
goal_g_amendment_8_official_review_count=2
goal_g_amendment_8_official_bundle_root=target/tmp/goal-g-amendment-3-recorder-bundle
goal_g_amendment_8_official_evidence_root=target/tmp/goal-g-phase0-amendment-3
goal_g_amendment_8_official_runtime_root=target/tmp/goal-g-amendment-3-runtime
goal_g_amendment_8_official_bundle_state=absent-not-created
goal_g_amendment_8_official_evidence_state=absent-not-created
goal_g_amendment_8_official_runtime_state=absent-not-created
goal_g_amendment_8_pre_g3_cargo_authorized=false
goal_g_amendment_8_pre_g3_test_or_benchmark_authorized=false
goal_g_amendment_8_pre_g3_public_fetch_authorized=false
goal_g_amendment_8_pre_g3_network_authorized=false
goal_g_amendment_8_production_order_entry_authorized=false
goal_g_amendment_8_real_credentials_loaded=false
goal_g_amendment_8_authenticated_external_request_sent=false
goal_g_amendment_8_real_polygon_rpc_request_sent=false
goal_g_amendment_8_real_order_submitted=false
goal_g_amendment_8_historical_goal_g_attempt_relabelled=false
goal_g_amendment_8_historical_goal_g_r_equivalence_claimed=false
goal_g_amendment_8_v5_mutation_or_promotion_authorized=false
goal_g_amendment_8_push_authorized=false
```

## Amendment 8 Terminal Activation Stop — 2026-08-02

Both preauthorization reviews passed the exact frozen Amendment 8 contract,
bootstrap, and supervisor. Exact `G8_AUTH` was committed, and the canonical
17-child launcher authenticated the repository and retained evidence, copied
exact v3 to fresh v6, post-verified v3/v5/v6, rebound the pre/post document
hashes, and rechecked final HEAD, tree, and clean status before releasing
success.

Before any v6 edit or invocation, a child-free static audit found that the
inherited constructor cannot satisfy Amendment 8's mandatory per-descendant-
child preflight boundary. Its bootstrap invokes `/bin/busybox stat` at source
line 8 without an immediately preceding exact storage preflight; additional
unpreflighted children occur at lines 11, 14, 28, and 41, while the constructor
does not define `storage_preflight` until line 175. The bootstrap region is
outside Amendment 8's provenance-only edit allowlist. An outer preview
preflight cannot be reused by these descendant children, so invoking or
editing around this conflict would violate the contract.

The audit also identified an inherited evidence-schema conflict: Amendment 6
requires static-review scratch inventory/removal fields, while Amendment 8
requires child-free read-only reviewers and reserves both scratch paths as
absent. Amendment 8 defines no closed replacement value for the inherited
scratch evidence. The execution-boundary conflict is independently terminal;
the schema conflict must also be resolved by any later amendment.

Fresh v6 remains an exact, unedited, uninvoked copy of v3. No patch, review
scratch, preview, official artifact, constructor run, Cargo command, test,
benchmark, network child, credential load, authenticated request, Polygon RPC,
or order entry occurred. A later attempt requires a new reviewed,
user-authorized amendment.

```text
goal_g_amendment_8_activation_stop_status=stopped
goal_g_amendment_8_activation_stop_schema=goal-g-amendment-8-activation-stop-v1
goal_g_amendment_8_activation_stop_stage=post-g8-auth-post-copy-pre-v6-construction
goal_g_amendment_8_activation_stop_parent_commit=4fa757f9ff2c6d4e748b30a87f664c2710f57848
goal_g_amendment_8_activation_stop_parent_tree=063e991a51923655bdc8cd63fc7850543bad56a6
goal_g_amendment_8_activation_stop_parent_parent=49210315169fa7ec3e3c02b4e70a745105bf9476
goal_g_amendment_8_activation_stop_parent_subject=docs: authorize goal g amendment 8 per-child preflight recovery
goal_g_amendment_8_activation_stop_parent_path_count=2
goal_g_amendment_8_activation_stop_parent_paths=docs/polymarket-authenticated-execution-goal-g-amendment-8.md,docs/polymarket-authenticated-execution-goal-g-handoff.md
goal_g_amendment_8_activation_stop_parent_contract_sha256=ca8a45bd372e6cb617d88d1b39e13e3f395bdf1bf5ad0280a2a97e98ab3cc72a
goal_g_amendment_8_activation_stop_parent_handoff_sha256=f3f1f3d6c47e30f85e27c739478876f846a7d8e25ee399eb349f941e46d4c345
goal_g_amendment_8_activation_stop_tracked_allowlist=docs/polymarket-authenticated-execution-goal-g-handoff.md
goal_g_amendment_8_activation_stop_failed_gate=constructor-descendant-per-external-child-storage-preflight-boundary
goal_g_amendment_8_activation_stop_failure_class=authorized-edit-surface-cannot-satisfy-execution-boundary
goal_g_amendment_8_activation_stop_first_nonconforming_script=/var/tmp/reap-g3-draft-v6/construct-self-test.preview.sh
goal_g_amendment_8_activation_stop_first_nonconforming_child=/bin/busybox-stat
goal_g_amendment_8_activation_stop_first_nonconforming_source_line=8
goal_g_amendment_8_activation_stop_first_nonconforming_child_mutating=false
goal_g_amendment_8_activation_stop_first_nonconforming_child_preflight_immediately_before=false
goal_g_amendment_8_activation_stop_additional_nonconforming_source_lines=11,14,28,41
goal_g_amendment_8_activation_stop_storage_preflight_definition_line=175
goal_g_amendment_8_activation_stop_outer_preflight_reuse_cures_descendants=false
goal_g_amendment_8_activation_stop_constructor_bootstrap_edit_authorized=false
goal_g_amendment_8_activation_stop_secondary_conflict=static-review-scratch-evidence-schema-versus-required-absence
goal_g_amendment_8_activation_stop_canonical_authentication_result=pass
goal_g_amendment_8_activation_stop_launcher_external_child_count=17
goal_g_amendment_8_activation_stop_launcher_storage_preflight_count=17
goal_g_amendment_8_activation_stop_g8_auth_commit=4fa757f9ff2c6d4e748b30a87f664c2710f57848
goal_g_amendment_8_activation_stop_g8_auth_tree=063e991a51923655bdc8cd63fc7850543bad56a6
goal_g_amendment_8_activation_stop_g8_auth_parent=49210315169fa7ec3e3c02b4e70a745105bf9476
goal_g_amendment_8_activation_stop_g8_auth_subject=docs: authorize goal g amendment 8 per-child preflight recovery
goal_g_amendment_8_activation_stop_g8_auth_contract_sha256=ca8a45bd372e6cb617d88d1b39e13e3f395bdf1bf5ad0280a2a97e98ab3cc72a
goal_g_amendment_8_activation_stop_g8_auth_handoff_sha256=f3f1f3d6c47e30f85e27c739478876f846a7d8e25ee399eb349f941e46d4c345
goal_g_amendment_8_activation_stop_copy_invoked=true
goal_g_amendment_8_activation_stop_copy_exit=0
goal_g_amendment_8_activation_stop_copy_argv_nul_sha256=80f7c5c38d836c51cf7868f9957c0b072c2966faa4767f644ecb38b5b8ecd7ff
goal_g_amendment_8_activation_stop_v6_root=/var/tmp/reap-g3-draft-v6
goal_g_amendment_8_activation_stop_v6_state=retained-non-authoritative-exact-copy-not-edited-not-invoked
goal_g_amendment_8_activation_stop_v6_root_dev=66305
goal_g_amendment_8_activation_stop_v6_root_inode=310607
goal_g_amendment_8_activation_stop_v6_root_mode=0700
goal_g_amendment_8_activation_stop_v6_root_uid=1000
goal_g_amendment_8_activation_stop_v6_root_gid=1000
goal_g_amendment_8_activation_stop_v6_root_nlink=2
goal_g_amendment_8_activation_stop_v6_root_size=4096
goal_g_amendment_8_activation_stop_v6_regular_bytes=1055725
goal_g_amendment_8_activation_stop_v6_component_manifest_rows=10
goal_g_amendment_8_activation_stop_v6_component_manifest_bytes=933
goal_g_amendment_8_activation_stop_v6_component_manifest_sha256=710ab62d5dbe846b21df74a4d78ee3f12d2a1883a22662d256bf751d411bc451
goal_g_amendment_8_activation_stop_v6_forensic_stream_bytes=1151
goal_g_amendment_8_activation_stop_v6_forensic_inventory_sha256=9ab5b0695e60cf47fc41e9d2e110b291c56b856d80cbfad36b9d41b6a5c7d233
goal_g_amendment_8_activation_stop_constructor_sha256=7f16928835d296353d6cc94501bd3cabd6f7febc7da044606673d7ee287c9bba
goal_g_amendment_8_activation_stop_v6_patch_state=absent-not-created
goal_g_amendment_8_activation_stop_v6_review_1_scratch_state=absent-not-created
goal_g_amendment_8_activation_stop_v6_review_2_scratch_state=absent-not-created
goal_g_amendment_8_activation_stop_preview_v5_state=absent-not-created
goal_g_amendment_8_activation_stop_preview_invocation_count=0
goal_g_amendment_8_activation_stop_official_bundle_state=absent-not-created
goal_g_amendment_8_activation_stop_official_evidence_state=absent-not-created
goal_g_amendment_8_activation_stop_official_runtime_state=absent-not-created
goal_g_amendment_8_activation_stop_g3_created=false
goal_g_amendment_8_activation_stop_phase0_started=false
goal_g_amendment_8_activation_stop_constructor_invoked=false
goal_g_amendment_8_activation_stop_cargo_invoked=false
goal_g_amendment_8_activation_stop_test_or_benchmark_invoked=false
goal_g_amendment_8_activation_stop_public_fetch_invoked=false
goal_g_amendment_8_activation_stop_network_invoked=false
goal_g_amendment_8_activation_stop_real_credentials_loaded=false
goal_g_amendment_8_activation_stop_authenticated_external_request_sent=false
goal_g_amendment_8_activation_stop_real_polygon_rpc_request_sent=false
goal_g_amendment_8_activation_stop_real_order_submitted=false
goal_g_amendment_8_activation_stop_production_order_entry_authorized=false
goal_g_amendment_8_activation_stop_historical_attempt_relabelled=false
goal_g_amendment_8_activation_stop_retry_authorized=false
goal_g_amendment_8_activation_stop_next_authority=new-reviewed-user-authorized-amendment
goal_g_amendment_8_activation_stop_push_authorized=false
```
