# Chaos iarb2 Parity

This document defines the parity boundary between Java
`imm-strategy/chaos/chaos-iarb2` and Rust `reap`. The target is decision-level
parity for the iarb2 strategy on OKX: given equivalent normalized market,
account, position, order, and timer events, Rust should reach equivalent quote,
hedge, and stop decisions.

The current source cross-check is pinned to `imm-strategy` commit
`b6b120c7b7c466d8431bf082f3229328c5d7b2ae`. Re-audit this document and the
fixture-derived Rust tests whenever that reference revision changes.

It is not a byte-for-byte port of the Java runtime, exchange abstractions, or
control plane.

## Status Definitions

- **Exact**: the Java formula or state transition is represented directly and
  covered by a focused Rust test.
- **Equivalent**: the same trading decision is produced through a different
  runtime or order-management boundary.
- **Partial**: a local decision is represented, but a required external runtime
  input is not.
- **Not implemented**: the behavior is rejected at the live boundary rather
  than accepted with weaker semantics.
- **Delegated**: intentionally owned by another Rust module or the deployment.

## Strategy Matrix

| Java behavior | Rust implementation | Status |
| --- | --- | --- |
| `Iarb2ParamService` defaults and `applyRiskMult` | `ChaosConfig::effective` and `validate` | Exact |
| Spot reference selection | Required `ref_symbol` with spot-kind validation | Exact |
| Spot, linear, and inverse quantity/delta/notional conversion | `InstrumentState` conversion methods | Exact |
| Per-group buy/sell hedge heaps | `update_best_hedges` and `RiskGroupState` | Exact |
| Strategy-wide hedge heaps | `best_hedges` with Java ordering and priority tie-breaks | Exact |
| `updateTheoPxQtyForRiskGroup` quote formula | `update_theo_quotes` | Exact |
| `summarizeHedges` depth allocation | `summarize_hedges` | Exact |
| Exclude the source symbol from its hedge | `summarize_hedges(..., source_symbol)` | Exact |
| Pending IOC depth exclusion | `active_hedges` and pending-level filtering | Exact |
| Exclude own resting quotes from hedgeable depth | `active_quotes` subtraction in `hedge_levels` | Exact |
| Fixed and step inventory skew | `CoinConfig` and derivative skew integration | Exact |
| Exchange funding and funding overwrite precedence | `FundingRate` events and `funding_override` | Exact |
| Earliest active funding window | `update_funding_window` | Exact |
| Ignore-best, quote-only, burst, price limit, and UTC halt behavior | Normalized market events and instrument state | Exact |
| Multi-level top/trailing quote targets and Java `Random(1)` sequence | `desired_quote_levels` and `JavaRandom` | Equivalent; optimizer churn differs |
| Quote debounce, force refresh, and top-level refill delay | Quote target state and fill timestamps | Exact for target decisions |
| Risk-group soft/hard/stop delta behavior | `RiskGroupState` quote and hedge permissions | Exact |
| Spot cash, equity, liability, loan, and borrow limits | Account-scoped balance state | Exact |
| Derivative position and margin capacity | Position and settle-currency account state | Exact |
| OKX exchange CMR and calculated portfolio margin checks | Separate margin fields and debouncers | Exact |
| Balance sheet, turnover, live size, PnL, margin, index, and basis checks | `check_risk_limits` and debounced conditions | Exact |
| `StableCoinDepegCheckerImpl`, startup stablecoin check, and 5-second runtime debounce | `StablecoinGuardConfig`, `RiskGate`, and stablecoin-aware `StartupGate` | Equivalent with stricter immediate entry blocking and durable live latch |
| Java startup basis limit of one third | Startup basis check | Exact |
| Java runtime basis return value being diagnostic only | `basis_breaches` without runtime halt | Exact |
| Account/position update driven hedging | `on_account_update` | Exact |
| Timer-driven strategy delta hedging | Timer event handling | Exact |
| Master strategy suppresses automatic hedging and requires a live `StrategyUpdate` within three seconds | Static `master_strategy` suppression remains available to strategy/backtest tests; `LiveConfig` rejects it until the external liveness feed exists | Partial; fail-closed live |
| Strategy-group PnL aggregation and member-state transitions | `LiveConfig` rejects `strategy_group` because no external group state/PnL feed exists | Not implemented; fail-closed live |
| Missed IOC hedge records | `MissedHedge` records from cancelled hedge orders | Exact |
| No-hedge, all-halted, zombie-hedge, stale-depth, and anomalous-fill stops | Stateful timers and halt reason | Exact |
| Feed/symbol halt removes an instrument from pricing and hedging | `on_system_event` | Exact |
| Backtest and live strategy use the same event API | `StrategyEvent -> Vec<OrderIntent>` | Equivalent |
| Quote fill becomes an account/position update before hedging | Synthetic update in backtest; private account/position push in live | Equivalent |

## One-Symbol Topology

Iarb2 is a cross-instrument strategy. The quoting instrument is excluded from
its own hedge set, and Java hedge-availability logic assumes multiple valid
entities. A one-symbol configuration therefore cannot produce a valid quoting
and hedging topology even though the Java parser does not reject it early.

Rust makes this invariant explicit:

- At least two instruments are required.
- Every quote-enabled instrument must have a distinct, non-`RefOnly`,
  hedge-enabled instrument.
- Symbols and account/currency ownership cannot be duplicated across risk
  groups.

Invalid topology is rejected by `config-check` and `ChaosStrategy::new` before
any order can be generated.

## Runtime Boundaries

The following differences do not change the covered quote/hedge calculations:

- Java `ChaosMassQuoter` and quote optimizers can amend orders, retain trailing
  orders, and allow two physical orders per target level. Rust emits canonical
  target levels and currently realizes changes with cancel/new intents. Exact
  exchange order churn is an execution-policy difference.
- `useL1Quoter` is represented by configuring one quote level. The Java quoter's
  optimizer-specific amend/refill mechanics are not copied.
- Rust live protocol support currently targets OKX. Binance-specific account,
  reduce-only, fee-asset, and position freshness behavior is not claimed.
- Java's `OrderRateLimitAlert` callback adds a three-second recovery margin and
  system-halts only Binance entities; it logs non-Binance entities as
  unexpected. The Rust OKX path instead uses bounded proactive pacing,
  exchange-response handling, and reconciliation, and does not claim callback
  parity.
- Master-strategy liveness and strategy-group PnL/state aggregation require an
  external `StrategyUpdate` control-plane feed. Live validation rejects both
  settings until that feed and its fail-closed freshness policy are implemented.
  Redis controls and process restart policy remain runtime/operations concerns.
- Qubyte history readers are not copied. Backtests consume normalized JSONL or
  the documented CSV format, and now consume public OKX raw captures through
  the Rust adapter/reducer path.

## Connectivity Cross-Check

| Java reference | Rust implementation | Result |
| --- | --- | --- |
| `OkxNitroL2SubscriberGroupFactory` subscriber groups | `partition_subscriptions` replica/socket plans | Equivalent |
| `AbstractOkxNitroL2Subscriber` TBT and 400-level modes | Explicit `books-l2-tbt`, `books50-l2-tbt`, or `books` capture subscriptions | Equivalent, entitlement remains operational |
| Ping, disconnect, resubscribe, and stale checkers | Feed connection loop, reconnect supervisor, idle timeout, and book-age recovery | Equivalent |
| Clear/rebuild book on resubscribe or crossed-book failure | Invalid/crossed-book detection plus sequence state and fresh websocket snapshot recovery | Equivalent with additional explicit sequence validation |
| `AbstractOkV5L2Subscriber.checkSeqNo` predecessor compare-and-set, equal-sequence no-change case, and lower-sequence maintenance case | `SequenceTracker` requires `prevSeqId == last`, accepts equal/lower `seqId`, records both cases, and recovers on mismatch | Exact continuity rule with bounded recovery buffering |
| Nitro checksum validation block commented out; legacy V5 CRC validation active | No CRC validation after OKX checksum deprecation; WSS, sequence, snapshot, crossed-book, and stale checks remain mandatory | Current-contract adaptation |
| Separate receive and exchange latency tracking | Raw `recv_ts_ns`, exchange timestamps, capture health counters, and bounded live webhook alerts | Equivalent data retained; alert routing is a deployment concern |
| Batch subscription manager and retry limits | Bounded socket partitioning, acknowledgement timeout, exponential reconnect | Equivalent lifecycle with different batching policy |
| `ChaosStrategyEngine.tryToStop`, `ChaosStrategyBase.cancelAll(entity)`, and `ExchCancelAll` | In-process canonical cancel/reconcile plus a separate account-wide regular-order emergency CLI | Equivalent normal stop with an additional process-independent safety layer |

The reviewed Java OKX subscriber and `chaos-iarb2` classes do not provide the
Rust runtime's place-request `expTime`, `/public/time` skew gate, Cancel All
After heartbeat, fsynced restart-latch lifecycle, exclusive journal lease,
webhook alert worker, Linux host guard, hardened supervisor policy, or
strategy-independent account-wide regular-order cancellation. Those are
intentional deployment-safety additions around the parity strategy, not claims
of Java strategy equivalence. Re-check exchange-facing controls against both the
pinned Java revision and the current OKX API contract whenever connectivity is
upgraded.

The pinned Nitro subscriber rebuilds on a crossed book, but its
`OkxNitroUtils.validateOrderBook` call is commented out. The pinned legacy V5
subscriber still contains an active CRC check and provides the direct sequence
reference: it resubscribes when `prevSeqId` does not match, treats
`seqId == prevSeqId` as valid, and identifies `seqId < prevSeqId` as possible
maintenance. OKX subsequently deprecated the checksum field, so Rust follows
the current [OKX API contract](https://www.okx.com/docs-v5/en/) and
[deprecation notice](https://www.okx.com/en-us/help/okx-order-book-channels-checksum-field-deprecation)
instead of copying obsolete CRC behavior.

The pinned Java stop path changes engine state and invokes per-entity
`cancelAll()` inside the running process, with `ExchCancelAll` providing
rate-limit debounce. Rust preserves normal in-process cancellation but does not
depend on it for the incident path: the separate command arms the exchange
deadman, enumerates all regular pending orders for the selected account, cancels
configured and unmanaged symbols, and proves zero after the trigger horizon.
This is intentionally broader than strategy-owned Java entities and explicitly
does not claim coverage of OKX algo or spread order classes.

## Live Event Requirements

Decision parity depends on delivering all required normalized events. For each
configured account and instrument, live composition must provide:

- Sequenced books and trades.
- Funding rate, index ticker, mark price, and price-limit updates where used.
- Redundant stablecoin/USD index tickers for every configured live risk guard.
- Account balances, margin snapshots, positions, orders, and fills.
- Account-scoped private stream heartbeat, stale, and recovery events derived
  from complete account/positions payload rounds, separately from per-socket
  ping/pong transport liveness.
- Timer and system events through the same single-writer strategy loop.

Private order reasons must be registered before REST submission so websocket
acknowledgements preserve `quote` versus `hedge` identity.

## Stablecoin Guard Cross-Check

At the pinned Java revision, `StableCoinDepegCheckerImpl` checks
`.USDT-USD.OK` and `.USDC-USD.OK` with a one-second cache. A missing value or
absolute deviation first forces a fresh fetch; the final rejection is missing
data or a downside move beyond the configured threshold. `ChaosParamService`
defaults both thresholds to 1%. `ChaosStrategyBase` requires a passing check at
startup, while `CalculatorBase` skips the check in backtests and stops live
validity after a continuously failing 5-second debounce.

Rust keeps the strategy/backtest boundary free of live stablecoin policy by
default. Live `RiskLimits` can configure one or more references; the demo
configuration requires `USDT-USD` and `USDC-USD` at the Java 1% thresholds.
Each reference is subscribed on redundant critical `index-tickers` routes.
Byte-identical replica frames deduplicate, while different payloads at the same
exchange timestamp reach `RiskGate` and mark that reference conflicting until a
newer update arrives. Missing, stale, invalid, conflicting, or downside-depegged
data blocks new orders immediately. A continuously unhealthy reference for
5 seconds emits `RiskBreach`, activates the durable global safety latch, and
cancels canonical live orders; cancels remain allowed throughout.

The websocket freshness default is 75 seconds because OKX documents changed
index values at up to 100 ms and unchanged values once per minute. Redundant
route connectivity is a separate immediate readiness condition. Production
validation requires a corresponding `USDT-USD` or `USDC-USD` guard when those
currencies appear in instrument metadata or symbols. See the current
[OKX index-tickers contract](https://www.okx.com/docs-v5/en/#public-data-websocket-index-tickers-channel).

## Evidence

Parity tests are in `crates/reap-strategy/src/chaos.rs`. They include the Java
calculator fixture values for spot, linear, and inverse pricing; hedge
summaries; risk multiplier behavior; funding; skew; account risk; debounce;
and stop conditions. Transport, deduplication, reconciliation, and fail-closed
tests live in their owning crates.

Run all Rust acceptance tests with:

```bash
cargo test --workspace --no-fail-fast
cargo clippy --workspace --all-targets -- -D warnings
```

The local Java source was cross-checked directly. Running its tests additionally
requires the private Java build environment and dependencies; this workspace
does not include Maven or a Maven wrapper.
