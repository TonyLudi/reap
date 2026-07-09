# reap

`reap` is a Rust clean-room replication of the core trading loop from
`imm-strategy/chaos`, focused on strategy logic and backtesting rather than live
exchange/Spring infrastructure.

Implemented:

- `iarb2`-style risk groups, delta checks, quote permissions, hedge ladders, and
  quote pricing.
- Maker quote replacement plus IOC delta hedges after quote fills.
- Shared book and order reducers for top-of-book state, taker liquidity, and
  idempotent order-state transitions.
- Backtest matching with `PostOnly`, `IOC`, current-depth fills, trade fills,
  queue-ahead tracking, and simple mark-to-market accounting.
- CSV replay runner and JSON backtest report.

Run the sample:

```bash
cargo run -p reap-cli -- backtest --config examples/iarb2-basic.toml --data examples/market.csv --pretty
```

Run the normalized JSONL fixture:

```bash
cargo run -p reap-cli -- backtest --format normalized-jsonl --config examples/iarb2-basic.toml --data fixtures/normalized/chaos_quote_hedge.jsonl --pretty
```

Run tests:

```bash
cargo test --workspace
```

Design docs:

- [docs/architecture.md](docs/architecture.md) describes the target HFT-style
  event-loop architecture, module split, websocket/dedup design, and migration
  plan.
- [docs/chaos-mapping.md](docs/chaos-mapping.md) maps the Java `chaos` logic to
  the current Rust scaffold and lists current scope limits.
