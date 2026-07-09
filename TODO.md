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

- [ ] Add `reap-venue` for exchange-specific parsers and request builders.
- [ ] Add `reap-feed` for websocket supervision.
- [ ] Implement multi-websocket subscription partitioning.
- [ ] Implement deduplication by channel-aware event ids.
- [ ] Implement sequence checks and snapshot recovery.
- [ ] Add raw replay checker for captured websocket data.

## Step 5: Live Order Gateway

- [ ] Implement signed submit/cancel for one venue.
- [ ] Add private websocket order/fill/account reducer.
- [ ] Add REST reconciliation for open orders and fills.
- [ ] Add idempotent client-order-id handling.
- [ ] Add rate-limit and request pacing policy.

## Step 6: Production Hardening

- [ ] Add pre-trade and post-trade risk gates.
- [ ] Add kill switch and manual symbol halt events.
- [ ] Add stale feed/private stream fail-closed policy.
- [ ] Add structured telemetry and health metrics.
- [ ] Add storage for raw events, normalized events, intents, orders, and fills.
- [ ] Profile the hot path before lower-level queue/thread optimizations.
