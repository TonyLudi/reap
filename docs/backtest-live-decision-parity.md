# Backtest And Live Decision Parity

Status: Goal D Phase 3 gate green. This document defines a credential-free
equality proof. It is not a live mode, a production configuration, or trading
approval.

## The Boundary

Reap has four distinct layers. Equality is claimed only where the same
production decision owner can be replayed from complete state:

| Layer | Phase 3 treatment | Equality claim |
| --- | --- | --- |
| Normalized input and scheduling | Checked-in events carry exact order, source milliseconds, local arrival nanoseconds, and explicit private due-wake service | Equal for the fixture's declared normalized inputs and schedule |
| Chaos strategy | Both sides execute the production `ChaosStrategy` through `TradingEngine<ChaosStrategy>` | Ordered typed purposes and their one-way legacy projections are byte-identical |
| Engine and risk | Both sides execute the production `RiskGate` transitions, staleness checks, rejections, system events, and deterministic fail-closed synthesis | Ordered intents, rejections, system events, and safety-cancel candidates are byte-identical |
| Downstream execution | The live fixture continues through the production coordinator's policy, canonical reservation, same-turn `PendingNew`, and pre-dispatch records/actions; economic backtest continues through matching/accounting | Deliberately different; the live logical reduction is separately golden, not called equal to simulated execution |

The proof does not route the economic `BacktestRunner` through live code.
`reap-backtest` has no dependency on `reap-live`, and the default economic
backtest remains unchanged.

## Complete Initialization

The checked-in schema-1 initialization artifact is a self-contained, strict,
clean transition-history genesis. It records:

- the pinned Java revision and exact configuration/evidence bindings;
- every effective `RiskLimits` field, with no serde-default fallback;
- exactly one valid risk model and one valid order-limit row for every
  executable symbol;
- feed and private readiness keys, timestamps, and stale state;
- risk marks and every strategy index, funding, mark, price-limit, and depth
  reference needed by the configured instruments;
- stablecoin observations, missing/conflict state, and breach-start state;
- complete account balances, equity, peak equity, account equity, and
  positions;
- live orders, rejection and unfilled-IOC windows and their ID sets,
  turnover, and seen-fill identities;
- kill-switch, halted-symbol, and halted-account state;
- storage/public/private/order-transport/reconciliation/forbidden-proof
  readiness;
- gateway-action accounts, order-entry state, session identity, and decision
  sequence; and
- the exact ordered production transitions used to reach that state.

Required arrays, scalars, and state partitions are explicit. Semantically
absent optional values follow the production type's canonical serde omission;
the raw JSON shape must exactly match the parsed value's canonical shape, so
unknown fields and omission of emitted fields fail closed. Duplicate
identity-bearing rows, incomplete symbol/account coverage, non-effective
strategy configuration, invalid numeric state, bootstrap/seed disagreement,
or an out-of-order replay sequence also fail before an engine is constructed.
Every private snapshot row is scoped to exactly one initialized account, and
there is a one-to-one correspondence between initialized accounts and
authoritative private seeds. Declared feed/private/order-transport readiness,
marks, references, stablecoins, positions, equity, and source clocks are
recomputed from the ordered seeds.

The fixture never hydrates private `RiskGate` fields directly: it creates a
real gate, installs checked instrument metadata, and drives production
normalized transitions. Schema 1 intentionally starts with empty live-order
and transition-history sets; later state is reached through replay events.

The artifact embeds the complete effective strategy rather than trusting a
bare source-config digest. Its authoring test reconstructs that value from the
checked `examples/iarb2-basic.toml` source plus explicit replay-only
freshness/account/debounce adjustments, calls `effective()`, and requires
structural equality. The parser separately requires the embedded config to be
an effective fixed point. The artifact SHA-256 therefore binds the exact
decision configuration; this is the self-contained replacement for Phase 0's
tentative plan to store only a config hash.

## Replay And Comparison

The shared harness lives only under `crates/reap-engine/tests/support/`.
`reap-engine` runs it directly, while a nested `reap-live` coordinator test
path-includes the same source. This creates no feature, public replay API,
runtime mode, or normal dependency edge.

Case blocks are contiguous and bytewise lexicographically ordered, and
sequence numbers are exact within each block. Each named case is independent
and starts from a fresh copy of the same initialization. The cases cover at
least:

- allowed quote creation;
- an IOC hedge decision;
- aggressive public-trade invalidation and explicit service of the private
  100-microsecond due wake;
- ordinary risk rejection;
- engine/risk and coordinator handling of an already-canonical normalized
  order/fill update, followed by a halt probe proving the filled order has
  left production live-order state;
- symbol-scoped halt and fail-closed cancellation; and
- global kill and deterministic global cancellation.

Every engine batch is projected structurally:

```text
case + sequence + input
  + ordered typed {purpose, legacy intent}
  + ordered structured rejections
  + ordered system events
  + ordered safety-cancel {order_id, reason}
```

For an allowed submit, the replay applies the same production local-send
transition and requires the immediately following `PendingNew` feedback to
match every order field and its generated-identity link. The live test
captures that batch at the existing
`LiveCoordinator::handle_engine_output` boundary before routing. It then lets
the unchanged coordinator perform regular policy, generate the real client
identity, reserve canonical ownership, register `PendingNew` in the same
turn, and produce its normal records/actions. The returned full logical
reduction is serialized twice and compared byte-for-byte before runtime
storage commit or dispatch. The checked-in live fixture is a strict manifest
of that full projection's schema, row count, canonical byte count, and SHA-256
digest, rather than a second 129-kilobyte copy of the engine-linked evidence.

One quote case deliberately separates the source/reservation clock
(`10 ms`) from the local observed/send clock (`12 ms`). This proves that the
genuine same-turn `PendingNew` timestamp follows the source event while the
private post-send strategy transition uses the local clock.

Generated client IDs are alpha-renamed by first occurrence (`client#1`,
`client#2`, and so on) only in the evidence projection. Equality links,
ordering, account/symbol ownership, action fields, and record fields remain
exact. The real generator and ownership state are not replaced.

## Explicitly Different Or Excluded

The replay does not claim equality for:

- raw websocket parsing, deduplication, sequence recovery, or runtime channel
  priority beyond the declared normalized schedule;
- authenticated bootstrap, credentials, sockets, transports, or exchange IO;
- private-feed ambiguity and identity resolution outside the canonical
  normalized events supplied by the fixture;
- private-state fill journaling or account scoping before the canonical
  normalized order update used by the fill case;
- storage enqueue, fsync, durable commit, pacing, preparation, dispatch,
  acknowledgement, reconciliation, or emergency behavior;
- simulated matching, latency, portfolio, funding, fee, or accounting
  behavior; or
- target-host latency, liveness, or production evidence.

Those differences remain owned by their existing production or simulation
layers. The Phase 3 proof grants no order authority and sets no production
authorization field.

## Evidence

The final Phase 3 gate records here and in
`determinism-readiness-goal-d-handoff.md`:

- initialization, replay-input, engine-output, and live-reduction SHA-256
  values;
- two byte-identical engine runs and two byte-identical live reductions;
- strict omission/completeness regressions;
- focused package, authority, dependency, and canonical-backtest results; and
- same-host benchmark and allocation checks.

The checked-in Phase 3 fixture family is:

| Artifact | SHA-256 |
| --- | --- |
| `fixtures/decision_parity/risk_initialization_v1.json` | `7e0951c41f447b9f46a73b24a3fe85bdc8f2bb8a623385dab0c3655926e73780` |
| `fixtures/decision_parity/replay_events_v1.jsonl` | `dede17a546d4d717c78dc2b3b7aa7c3f3f785d552404160407c78fb87cec9101` |
| `fixtures/decision_parity/expected_engine_v1.jsonl` | `140c268619b889a19d779e1bdfd340c11901d2eb1d9e4d216d976ba3d8b0d37a` |
| `fixtures/decision_parity/expected_live_reduction_v1.json` | `aa66cc09bba29cde25ab2df66c018517b2c900f83373f95580150e8bcd442b60` |

The live manifest binds 41 canonical rows and 129,098 bytes at
`847c6f8ba5177cf456d0dc2c7c31df74a9b189c107e7167d06dd48bf09b7762b`.
Both the engine and live harness produce their complete projections twice and
compare exact bytes. The canonical economic backtest remains
`38acf9f5e0c310f2ec5528974beffadf4c1a7f84d46efa8d9664ee7051e84691`.
Exact commands, package counts, dependency/schema guards, phase commits, and
same-host measurements are retained in
`determinism-readiness-goal-d-handoff.md`.
