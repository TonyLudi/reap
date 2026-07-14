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
| Signed fill fee/rebate and fee currency | Per-update order `fillFee`, per-fill websocket/REST `fee`, canonical order state, and durable fill records | Equivalent current-wire handling; cumulative legacy order fee is intentionally not charged as one fill |
| Derivative position and margin capacity | Position and settle-currency account state | Exact |
| OKX exchange CMR and calculated portfolio margin checks | Separate margin fields and debouncers | Exact |
| Balance sheet, turnover, live size, PnL, margin, index, and basis checks | `check_risk_limits`, debounced conditions, and generic engine safety-halt promotion | Exact thresholds with a stricter durable global latch and all-order cancellation on terminal halt |
| `EntityOpenOrderSafeguard.EntityMaxOpenOrder` | Projected pre-trade and authoritative post-trade global/per-symbol active-order count limits | Equivalent per-symbol containment with an additional global ceiling; Rust post-trade breach latches globally |
| `GatewayOrderStatsSafeguard` submit-rejection controls (`OrderMaxReject`, `SymbolMaxReject`, and total reject scopes) | Deduplicated rolling global/per-symbol canonical `Rejected` order thresholds | Equivalent submit-reject containment with a stricter durable global latch; amend is unsupported and cancel failures reconcile immediately |
| `GatewayOrderStatsSafeguard.SymbolMaxIOCCancelled` | Deduplicated rolling per-symbol canonical IOC `Cancelled` thresholds when cumulative fill is exactly zero | Equivalent Java zero-fill semantics with multi-socket duplicate suppression and a stricter durable global latch |
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
| Local order registration precedes exchange acknowledgement | Synchronous canonical `PendingNew` in live and backtest; pending quotes/hedges count as working state | Equivalent |
| Quote fill becomes an account/position update before hedging | Synthetic update in backtest; private account/position push in live | Equivalent |
| Maker/taker transaction cost | Per-instrument fill fee with explicit fee-cost and turnover attribution | Equivalent; fee-tier calibration remains operational |
| `PortfolioExchAcctCalculator.settleSwapFundingAt` linear/inverse formulas | Scheduled funding settlement using latest rate and exchange mark/depth fallback | Exact signed formulas; Rust schedules the advertised exchange timestamp instead of Java's configured time-of-day list |

## Backtest Execution Cross-Check

The Rust scheduler was cross-checked against the pinned Java
`BackTestDelay`, `FeeAndDelaySpec`, `QueueMatchingEngine`, and
`QueueMatchingEngineFactory` classes under `chaos/chaos-backtest/backtest-core`.

| Java behavior | Rust implementation | Status |
| --- | --- | --- |
| `MatchingNew` schedules matching eligibility | `order_entry_latency_ms` and separate matcher `prepare_submit`/`activate` phases | Equivalent |
| `MatchingCancel` leaves an order matchable until effective | `cancel_latency_ms` scheduled cancel action | Equivalent |
| `OrderUpdate` and `OrderFill` publishers have separate delays | `order_update_latency_ms` and `fill_account_latency_ms` | Equivalent at the strategy boundary; Rust has no separate strategy fill event |
| `PendingNew` then live/terminal update | Immediate canonical `PendingNew`, followed by delayed matcher activation and delayed exchange update | Equivalent to the live gateway boundary; earlier than Java matching publication by design |
| Queue ahead is consumed by matching trades | Per-order queue-ahead metadata consumed by maker-side trade events | Equivalent |
| Aggressive entry takes current displayed depth; IOC remainder cancels | Activation matches current book and emits taker fills plus terminal IOC update | Equivalent |
| Per-instrument maker/taker fee map | Maker/taker fee fields on each Rust instrument | Equivalent |
| Separate market depth/quote/trade and order-path delays | Java-mapped class/symbol empirical `latency_profile` with scalar fallbacks and raw replay ordered by persisted receive time | Model coverage complete for represented Rust event classes; target-host capture/demo samples remain calibration work |
| Conservative depth-fill threshold and queue reset on a shallow cross | `depth_fill_conservative_threshold`; basic cross clears queue-ahead and threshold cross controls displayed-depth fill | Exact formula and pinned Java application default; empirical calibration remains required |
| Exact displayed queue and full historical trade/depth capacity | `queue_ahead_multiplier`, `historical_trade_fill_fraction`, and `displayed_depth_fill_fraction` | `1.0` preserves Java parity; conservative Rust sensitivity overlays are intentional and require empirical calibration |
| Date-partitioned multi-run service and per-run result files | Manifest folds, immutable input fingerprints, candidate training selection, and test scenario reports | Rust extends the Java artifact boundary with explicit leakage and acceptance gates |
| Carry ending positions into the following daily run | Every Rust research dataset starts from a zero portfolio and reports that semantic | Intentional current difference; use a continuous dataset and terminal exposure gates rather than assuming cross-file carry |

### Live Latency Evidence Cross-Check

The class names remain pinned to
`chaos/chaos-backtest/backtest-core/.../BackTestDelay.java` at Java revision
`b6b120c7b7c466d8431bf082f3229328c5d7b2ae`. The live measurement boundaries
were also checked against these concrete Java paths:

| Java reference | Rust live evidence | Qualification |
| --- | --- | --- |
| `AbstractOkxNitroL2Subscriber.createOrderBookPayloadHandler` records `StartProcessMarketDepth` before JSON parsing | accepted raw `recv_ts_ns` through parse, deduplication, sequencing/book reduction, and entry into the coordinator | Measures the additional target-host path needed after raw replay's receive-time boundary |
| `OkxNitroOrderClient` records websocket send/sent and order-message process stages | dispatch through the account task, pacing, HTTP request, and successful REST acknowledgement | Current Rust gateway uses REST; this is a conservative `MatchingNew`/`MatchingCancel` upper bound, not exchange matching-engine time |
| `MatchingOrderUpdatePublisher` schedules both `OrderUpdate` and `BackTestOrderUpdate` with `OrderUpdate` delay | exchange order/fill timestamp to canonical strategy visibility | Requires synchronized host and exchange clocks; the REST reconciliation path is excluded |
| `MatchingOrderFillPublisher` schedules `OrderFill` with `OrderFill` delay; `BackTestDelay` notes account/position publication is coupled to order update | canonical fill visibility until the derivative position or both spot balances are visible | Matches the Rust strategy's authoritative account-state hedge boundary; it is not a claim that Java's internal publisher topology is identical |

The Rust profile has no separate `quote` or `matching_trade` event class because
those Java message boundaries are not distinct normalized inputs in the current
Rust strategy loop. `reference_data` is a documented Rust extension for index,
funding, mark, price-limit, and burst inputs. `MatchingAmend` remains absent
because the live policy realizes changes with cancel/new. Calibration requires
every represented class for every relevant symbol, records these omissions
instead of silently inventing samples, and rejects private classes from
read-only observe runs.

Live reports carry the full serialized config fingerprint, Reap executable
SHA-256, pinned Java revision, pseudonymous machine/account identities, unique
session, synchronized host snapshots, and deterministic bounded sample
reservoirs. `calibrate-latency` rejects failed operations or malformed/missing
series, rounds delays upward into backtest milliseconds, and creates an
integrity-checked artifact. A schema-2 production research manifest must point
to that artifact, run the byte-identical Reap executable, and use its exact
baseline profile. The machinery is complete; no credentialed target-host
artifact has passed yet.

Failure handling was cross-checked against the same pinned connectivity tree.
`AbstractOkxNitroL2Subscriber.onSocketDisconnected` clears the affected Java
book and its base subscriber resubscribes after a lost session;
`OkxNitroOrderClient` publishes `LOST` connection state and rejects new orders
with `REQUEST_BLOCKED` while disconnected. Rust likewise invalidates feed
readiness, blocks entry, supervises reconnect/recovery, and reconciles private
state. Rust intentionally adds a stronger audit boundary: after a report-capable
runtime exists, an initialization, event-loop, or teardown error completes
fail-closed cancellation/reconciliation, persists the schema-4 failure code plus
pre/post-cleanup evidence, and only then returns the nonzero process error. This
report does not make the fault acceptable; it makes the required demo fault
campaign machine-reviewable.

Zero-delay and full-capacity fixture compatibility is intentional, but it is
optimistic evidence, not a calibrated execution claim. Backtest reports retain
the effective delay/capacity configuration, local time basis, clock regressions,
active orders, and scheduled work left at the capture horizon.

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
- Rust live spot routing is deliberately cash-only. A nonzero OKX position must
  be a configured derivative owned by the delivering account; margin-spot or
  foreign exposure is rejected because the iarb2 live risk model does not
  safely value it. Backtest strategy math remains independent of this live
  account-isolation policy.
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
| Separate receive and exchange latency tracking | Raw `recv_ts_ns`, exchange timestamps, bounded-memory capture timing distributions, capture health counters, and bounded live webhook alerts | Equivalent data retained; receive delay includes host clock/scheduling and alert routing is a deployment concern |
| Batch subscription manager and retry limits | Bounded socket partitioning, acknowledgement timeout, exponential reconnect | Equivalent lifecycle with different batching policy |
| `OrderDetailUpdate` `tradeId`/`fillSz`/`fillPx` and `UserTrade` `tradeId`/`fee`/`feeCcy`/`execType` | Current OKX order-channel `tradeId` plus per-fill `fillFee`/`fillFeeCcy`, optional fills channel with fee-bearing order-update precedence under either arrival order, instrument-scoped once-only journal record, and raw-statement comparison | Current-contract strengthening; the pinned Java order-session fill derivation says fee is unavailable and its separate user-trade processing is commented out |
| `OkxNitroRestClient.getFills` 100-row pages, 200 ms pacing, descending cursor pagination, and short-page completion | Authenticated read-only `/api/v5/trade/fills` collector with the same page size/pacing/completion rule, fail-closed page bound, raw response retention, bracketed account identity, and offline cursor replay | Lifecycle mapping with a current-contract endpoint adaptation; pinned Java reads Nitro spread trades while Reap certifies regular strategy fills |
| `StaleOrderUpdateSafeguard` and `GatewayOrderStatsSafeguard.BookTotalPendingOrder` | Per-order submit-to-private-state and cancel-to-terminal-state deadlines, cancel retry, account block, and full REST reconciliation | Equivalent fail-closed intent with explicit OKX REST/private-state convergence |
| `PositionGatewaySafeguard.OrsFillDelayToPositionUpdate` and the fill-derived position reconciler | Per-fill derivative-position or spot-currency convergence deadline plus monotonic rows, full REST comparison, tombstone repair, and second-pass confirmation | Equivalent fail-closed intent; Rust uses an explicit post-fill deadline rather than Java's two-cycle timestamp-difference check |
| `PositionGatewaySafeguard.PosnMgnModeMismatch` | Required OKX `mgnMode` on websocket/REST positions, configured-mode bootstrap/runtime enforcement, and reconciliation comparison | Equivalent fail-closed intent; Rust aborts the live lifecycle instead of pausing inside a separate gateway process |
| `PositionGatewaySafeguard.ForcedRepaymentIndicator` | Typed OKX balance `twap`, configured `1..=5` limit, bootstrap/runtime account-state rejection, tombstone clearing, and reconciliation comparison | Stronger: Java emits alert-only, while Rust fails closed at or above the configured level |
| `ChaosStrategyEngine.tryToStop`, `ChaosStrategyBase.cancelAll(entity)`, `ExchCancelAll`, and `OkxNitroOrderClient.onOrderSessionDisconnected` | In-process canonical cancel/reconcile plus a separate account-wide regular-order emergency CLI | Equivalent normal/disconnect cancellation with an additional process-independent final-zero safety layer |

The reviewed Java OKX subscriber and `chaos-iarb2` classes do not provide the
Rust runtime's place-request `expTime`, `/public/time` skew gate, Cancel All
After heartbeat, fsynced restart-latch lifecycle, exclusive journal lease,
bounded authenticated fill-evidence collection and verification, webhook alert worker,
Linux host guard, hardened supervisor policy, or strategy-independent
account-wide regular-order cancellation. Those are intentional
deployment-safety additions around the parity strategy, not claims of Java
strategy equivalence. Re-check exchange-facing controls against both the pinned
Java revision and the current OKX API contract whenever connectivity is
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
does not claim coverage of OKX algo or spread order classes. At the pinned
revision, `OkxNitroOrderClient` also rejects new sends with `REQUEST_BLOCKED`
while its websocket state is not `CONNECTED`; on order-session disconnect it
calls `cancelAll()`, whose implementation combines maker mass-cancel, local
open-order-cache cancellation, and exchange open-order polling. The independent
Rust command strengthens the audit boundary with an exact-config, binary, host,
Java-revision, exchange-account-identity, account-coverage, and
task-failure-bound artifact written before its final exit status.

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

Account reconciliation is intentionally account-scoped and authoritative.
Differences are measured before replacement, omitted REST rows become zero
events for strategy/risk, and a dirty repair cannot restore readiness until a
later full-state pass agrees. This closes restart/reconnect state drift but does
not copy Java's exact timer arithmetic: Rust explicitly waits for each fill's
affected derivative position or both affected spot balances and reconciles the
whole account on timeout. Each nonzero configured derivative position must also
carry the configured `cross` or `isolated` mode; missing, unsupported, or
mismatched mode fails before state application. Account balance `twap` is
retained as a `0..=5` forced-repayment risk level; unlike Java's alert-only
action, Reap fails the live lifecycle when it reaches the configured limit.

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
