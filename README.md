# reap

`reap` is a Rust clean-room replication of the core trading loop from
`imm-strategy/chaos`. It keeps one deterministic strategy/event model across
backtest and live exchange boundaries.

Implemented:

- Decision-level parity with the documented OKX `iarb2` boundary: risk groups,
  spot/linear/inverse conversion, quote pricing, hedge allocation, funding,
  inventory skew, account limits, and stop conditions.
- Explicit rejection of one-symbol and self-only hedge topologies.
- Maker quote targets plus account/position-driven and timer-driven IOC delta
  hedges, including pending-liquidity exclusion and missed-hedge records.
- Shared book and order reducers for top-of-book state, taker liquidity, and
  idempotent order-state transitions.
- OKX public/private parsers, HMAC-signed REST order requests, supervised
  multi-websocket feeds, account-scoped channel-aware deduplication, sequence
  recovery, and REST reconciliation.
- Deterministic pre/post-trade risk, stale-stream fail-closed behavior, kill
  switch and symbol halt events, and an event-loop enforcement layer.
- Bounded structured telemetry and JSONL storage for raw, normalized, intent,
  request, acknowledgement, order, fill, system, bootstrap, and reconciliation
  records, including restart checkpoint recovery.
- A fail-closed `reap-live` composition root with account-scoped REST bootstrap,
  exchange metadata/account-mode verification, redundant public sockets,
  isolated private sockets, one strategy owner, prioritized gateway tasks, and
  graceful cancel-and-drain shutdown.
- Backtest matching with `PostOnly`, `IOC`, current-depth fills, trade fills,
  queue-ahead tracking, and simple mark-to-market accounting.
- CSV/normalized replay, raw-capture validation, configuration validation, and
  a release-mode hot-path benchmark.

Run the sample:

```bash
cargo run -p reap-cli -- backtest --config examples/iarb2-basic.toml --data examples/market.csv --pretty
```

Run the normalized JSONL fixture:

```bash
cargo run -p reap-cli -- backtest --format normalized-jsonl --config examples/iarb2-basic.toml --data fixtures/normalized/chaos_quote_hedge.jsonl --pretty
```

Validate a captured websocket stream and strategy config:

```bash
cargo run -p reap-cli -- replay-check --events fixtures/raw/okx/depth-gap.jsonl --strict --pretty
cargo run -p reap-cli -- config-check --config examples/iarb2-basic.toml --pretty
```

Validate the live demo configuration without reading credentials or opening a
network connection:

```bash
cargo run -p reap-cli -- live --config examples/live-okx-demo.toml --mode validate --pretty
```

Observe OKX demo feeds and account state without permitting any submit or
cancel request:

```bash
export REAP_OKX_API_KEY=...
export REAP_OKX_SECRET_KEY=...
export REAP_OKX_PASSPHRASE=...
export REAP_OPERATOR_TOKEN=... # at least 32 bytes from the secret provider
cargo run -p reap-cli -- live --config examples/live-okx-demo.toml --mode observe
```

From another shell with the same operator token, inspect or stop the runtime:

```bash
cargo run -p reap-cli -- operator --config examples/live-okx-demo.toml status --pretty
cargo run -p reap-cli -- operator --config examples/live-okx-demo.toml shutdown --reason "planned stop"
```

Run a bounded observe soak and return a non-zero status unless the runtime
reaches readiness, finishes the requested window, records no reconciliation
drift or storage drops, and shuts down with no active orders:

```bash
cargo run -p reap-cli -- live --config examples/live-okx-demo.toml --mode observe --duration-secs 3600 --require-clean-soak --pretty
```

Enable demo order entry only with the explicit confirmation flag:

```bash
cargo run -p reap-cli -- live --config examples/live-okx-demo.toml --mode demo --confirm-demo
```

Run tests:

```bash
cargo test --workspace
cargo clippy --workspace --all-targets -- -D warnings
```

Profile the deterministic event loop:

```bash
cargo bench -p reap-engine --bench event_loop
cargo bench -p reap-live --bench live_loop
```

`demo` mode rejects a production exchange configuration. `observe` is strictly
read-only. Production order entry is intentionally not exposed: a credentialed
OKX demo soak, fault campaign, latency profile, and operator rollout approval
remain required before production capital.

Design docs:

- [docs/architecture.md](docs/architecture.md) describes the target HFT-style
  event-loop architecture, module split, websocket/dedup design, and migration
  plan.
- [docs/chaos-mapping.md](docs/chaos-mapping.md) maps the Java `chaos` logic to
  Rust modules and lists remaining strategy-model scope limits.
- [docs/operations.md](docs/operations.md) defines startup, fail-closed, recovery,
  and credential procedures.
- [docs/trading-readiness.md](docs/trading-readiness.md) lists the exact gap from
  the current libraries to demo and production trading.
- [docs/performance.md](docs/performance.md) records the strategy and complete
  live-parity benchmarks, allocation profile, and measured optimizations.
