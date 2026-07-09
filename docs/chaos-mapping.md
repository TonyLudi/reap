# Chaos Mapping

This repo ports the decision flow, not the production runtime.

The strategy-facing boundary is now `StrategyEvent -> Vec<OrderIntent>`, with
live and backtest inputs represented as `NormalizedEvent`.

| Java chaos area | Rust location | Notes |
| --- | --- | --- |
| `Iarb2Strategy` market/fill loop | `crates/reap-strategy/src/chaos.rs` | Depth updates refresh risk, hedges, theo quotes, and quote orders. Quote fills trigger IOC hedges. |
| `Iarb2Calculator.updateHedgeBySide` | `ChaosStrategy::update_best_hedges` | Builds per-risk-group and strategy-wide hedge ladders. Buy ladders sort cheapest first; sell ladders sort richest first. |
| `Iarb2Calculator.updateTheoPxQtyForRiskGroup` | `ChaosStrategy::update_theo_quotes` | Uses opposite-side hedge ladders, quote/hedge margins, maker/taker fees, fair-value offset, and inventory skew. |
| `Iarb2Calculator.summarizeHedges` | `ChaosStrategy::summarize_hedges` | Converts hedge ladder notional into per-symbol IOC hedge orders, excluding the just-filled symbol. |
| `RiskGroup` | `RiskGroupState` | Soft/hard delta quote gates and group-only hedge behavior are represented. |
| `QueueMatchingEngine` | `crates/reap-backtest/src/matching.rs`, `crates/reap-book`, `crates/reap-order` | Supports `PostOnly`, `IOC`, current-depth taker fills, later maker fills from trades/depth, queue-ahead tracking, and shared canonical book/order reducers. |
| `BackTestEngine` | `BacktestRunner` | Drives replay events through matcher -> strategy -> matcher feedback until commands drain. |

Out of scope for this first repo:

- Live exchange connectors, Redis control plane, Spring/Luban bootstrapping, alerting, and flight recorders.
- Full spot account borrowing/margin model and OKX/Binance-specific fee assets.
- Funding-rate manager and index-deviation stop logic beyond configurable fair-value offsets.
- Multi-level historical depth file formats from Qubyte. The runner uses a small CSV replay format that is easy to convert into.

Replay CSV columns:

```text
ts_ms,symbol,bid_px,bid_qty,ask_px,ask_qty,trade_px,trade_qty,taker_side
```

Rows can contain a depth update, a trade, or both. `taker_side` is `buy` or
`sell` when a trade is present.

Normalized JSONL replay fixtures use one `NormalizedEvent` per line. See
`fixtures/normalized/chaos_quote_hedge.jsonl` for a deterministic quote-then-
hedge decision fixture.
