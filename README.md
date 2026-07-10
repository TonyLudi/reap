# reap

`reap` is a Rust clean-room replication of the core trading loop from
`imm-strategy/chaos`. It keeps one deterministic strategy/event model across
backtest and live exchange boundaries.

Implemented:

- `iarb2`-style risk groups, delta checks, quote permissions, hedge ladders, and
  quote pricing.
- Maker quote replacement plus IOC delta hedges after quote fills.
- Shared book and order reducers for top-of-book state, taker liquidity, and
  idempotent order-state transitions.
- OKX public/private parsers, HMAC-signed REST order requests, supervised
  multi-websocket feeds, channel-aware deduplication, sequence recovery, and
  REST reconciliation.
- Deterministic pre/post-trade risk, stale-stream fail-closed behavior, kill
  switch and symbol halt events, and an event-loop enforcement layer.
- Bounded structured telemetry and JSONL storage for raw, normalized, intent,
  order, and fill records.
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

Run tests:

```bash
cargo test --workspace
cargo clippy --workspace --all-targets -- -D warnings
```

Profile the deterministic event loop:

```bash
cargo bench -p reap-engine --bench event_loop
```

The live order gateway is a library boundary and is not exposed as an
accidental one-command production launcher. Integrators must supply credentials,
regional OKX URLs, instrument trade modes, startup reconciliation, storage, and
health wiring described in the operations guide. Validate with OKX demo trading
before enabling production credentials.

Design docs:

- [docs/architecture.md](docs/architecture.md) describes the target HFT-style
  event-loop architecture, module split, websocket/dedup design, and migration
  plan.
- [docs/chaos-mapping.md](docs/chaos-mapping.md) maps the Java `chaos` logic to
  Rust modules and lists remaining strategy-model scope limits.
- [docs/operations.md](docs/operations.md) defines startup, fail-closed, recovery,
  and credential procedures.
- [docs/performance.md](docs/performance.md) records the initial hot-path
  benchmark and optimization decision.
