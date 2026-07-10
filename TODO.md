# Reap Migration TODO

This list tracks the migration from the current Rust scaffold toward the target
architecture in [docs/architecture.md](docs/architecture.md).

## Step 1: Workspace Split

- [x] Convert the single crate into a Cargo workspace.
- [x] Create `crates/reap-core` for shared market/order/config primitives.
- [x] Create `crates/reap-strategy` for pure chaos strategy logic.
- [x] Create `crates/reap-backtest` for replay, matching, and reports.
- [x] Create `crates/reap-cli` for user-facing binaries.
- [x] Preserve current CLI behavior:
  `cargo run -p reap-cli -- backtest --config examples/iarb2-basic.toml --data examples/market.csv --pretty`.
- [x] Preserve current tests and sample backtest output shape.

## Step 2: Normalized Event Boundary

- [x] Introduce `NormalizedEvent`, `StrategyEvent`, and `OrderIntent`.
- [x] Make backtest and live designs use the same strategy input/output types.
- [x] Remove direct strategy coupling to matcher-specific structs.
- [x] Add deterministic replay fixtures for quote and hedge decisions.

## Step 3: Book And Order Reducers

- [x] Add `reap-book` with single-writer book reducers.
- [x] Add book status states: empty, recovering, ready, stale, gapped.
- [x] Add `reap-order` with canonical order state reducer.
- [x] Add idempotent order update/fill handling tests.
- [x] Update backtest to use the shared reducers.

## Step 4: Live Feed Skeleton

- [x] Add `reap-venue` for exchange-specific parsers and request builders.
- [x] Add `reap-feed` for websocket supervision.
- [x] Implement multi-websocket subscription partitioning.
- [x] Implement deduplication by channel-aware event ids.
- [x] Implement sequence checks and snapshot recovery.
- [x] Add raw replay checker for captured websocket data.

## Step 5: Live Order Gateway

- [x] Implement signed submit/cancel for one venue.
- [x] Add private websocket order/fill/account reducer.
- [x] Add REST reconciliation for open orders and fills.
- [x] Add idempotent client-order-id handling.
- [x] Add rate-limit and request pacing policy.

## Step 6: Production Hardening

- [x] Add pre-trade and post-trade risk gates.
- [x] Add kill switch and manual symbol halt events.
- [x] Add stale feed/private stream fail-closed policy.
- [x] Add structured telemetry and health metrics.
- [x] Add storage for raw events, normalized events, intents, orders, and fills.
- [x] Profile the hot path before lower-level queue/thread optimizations.

Completion evidence is documented in [README.md](README.md),
[docs/operations.md](docs/operations.md), and
[docs/performance.md](docs/performance.md). The workspace test, lint, replay,
configuration, backtest, and benchmark commands are the acceptance gates.
