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

## Step 7: Chaos Iarb2 Decision Parity

- [x] Cross-check Rust configuration, pricing, hedging, skew, funding, account
  risk, and stop behavior against Java `chaos-iarb2` source and fixtures.
- [x] Reject one-symbol and self-only hedge topologies before startup.
- [x] Cover spot, linear, and inverse quote/hedge vectors with parity tests.
- [x] Make account/position updates authoritative for hedge triggers.
- [x] Add funding, index, mark/limit, account, margin, and position events.
- [x] Preserve quote/hedge identity across REST and private websocket races.
- [x] Scope private feed health and deduplication by account.
- [x] Document execution-policy and platform differences without claiming
  Binance or Java control-plane parity.

## Step 8: Demo-Tradable Runtime

- [x] Add a live composition crate/command with one strategy event-loop owner.
- [x] Verify exchange instrument metadata, account mode, trade mode, and risk
  valuation before subscriptions are considered ready.
- [x] Implement the complete startup and restart reconciliation state machine.
- [x] Wire accepted intents through registered submit/cancel and private
  feedback into the engine.
- [x] Add end-to-end fault tests for disconnects, gaps, duplicates, partial
  fills, IOC misses, ambiguous submits, rate limits, and restart recovery.
- [x] Profile the parity event loop with production-shaped captures and remove
  measured hot-path allocation/collection bottlenecks.
- [x] Add bounded observe/demo soak execution with machine-verifiable readiness,
  drift, storage, and shutdown evidence.
- [x] Make every demo exit disable new submits while preserving cancellation,
  then require zero active orders and post-cancel REST reconciliation even when
  storage has failed.
- [x] Add a bounded Unix-socket operator service with environment-keyed HMAC,
  freshness/replay checks, status, global/account kill, symbol halt/resume, and
  graceful stop.
- [ ] Complete an OKX demo soak with no unexplained reconciliation drift.

Completed-step evidence is documented in [README.md](README.md),
[docs/operations.md](docs/operations.md), and
[docs/performance.md](docs/performance.md). The workspace test, lint, replay,
configuration, backtest, and benchmark commands are the acceptance gates.
The remaining path to trading is tracked in
[docs/trading-readiness.md](docs/trading-readiness.md).
