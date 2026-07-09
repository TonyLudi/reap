# reap

`reap` is a Rust clean-room replication of the core trading loop from
`imm-strategy/chaos`, focused on strategy logic and backtesting rather than live
exchange/Spring infrastructure.

Implemented:

- `iarb2`-style risk groups, delta checks, quote permissions, hedge ladders, and
  quote pricing.
- Maker quote replacement plus IOC delta hedges after quote fills.
- Strategy-only matching book with `PostOnly`, `IOC`, current-depth fills, trade
  fills, queue-ahead tracking, and simple mark-to-market accounting.
- CSV replay runner and JSON backtest report.

Run the sample:

```bash
cargo run -- backtest --config examples/iarb2-basic.toml --data examples/market.csv --pretty
```

Run tests:

```bash
cargo test
```

See [docs/chaos-mapping.md](docs/chaos-mapping.md) for the Java-to-Rust mapping
and current scope limits.
