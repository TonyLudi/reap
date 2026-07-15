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
| Exchange funding forecast and funding overwrite precedence | `FundingRate.rate` and `funding_override` | Exact; OKX `fundingRate` remains the Java-parity strategy input |
| Earliest active funding window | `update_funding_window` | Exact |
| Ignore-best, quote-only, burst, price limit, and UTC halt behavior | Normalized market events and instrument state | Exact |
| Multi-level top/trailing quote targets and Java `Random(1)` sequence | `desired_quote_levels` and `JavaRandom` | Equivalent; optimizer churn differs |
| Quote debounce, force refresh, and top-level refill delay | Quote target state and fill timestamps | Exact for target decisions |
| Risk-group soft/hard/stop delta behavior | `RiskGroupState` quote and hedge permissions | Exact |
| Spot cash, equity, liability, loan, and borrow limits | Account-scoped balance state | Exact decision model; Rust live is intentionally stricter and rejects configured or observed borrowing until interest accounting exists |
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
| `Iarb2ChaosEntityFactory` binds the cached `Instrument`; `ChaosEntity` repeatedly uses its tick table and contract value while `OkEntity.getMinTradeSize` uses minimum/lot size | Exact authenticated OKX instrument metadata is verified at bootstrap and periodically across state, sizing and single-order maxima, valuation, currencies, family, and fee group; typed upcoming rule changes stop inside a configured lead | Current-contract hardening around Java's instrument assumption; Rust requires a reviewed config and clean restart after drift |
| `Iarb2Calculator.summarizeHedges` aggregates selected depth by symbol and `Iarb2Strategy.doHedge` submits one limit IOC per resulting target | The equivalent Rust summary remains intact, but every final post-only/IOC quantity and applicable spot USD amount is checked against authenticated `maxLmtSz`/`maxLmtAmt` immediately before dispatch | Stronger current-account safety boundary; exchange maxima do not replace strategy, account, or global risk limits |
| `StrategyToolkit.getTransactionFee` maker/taker inputs plus `ChaosEntity.sanityCheck` fee ordering | Per-instrument strategy cost rates, Java-equivalent taker-at-least-maker validation, and authenticated OKX `groupId`/`feeGroup` bootstrap plus periodic underpricing guard | Stronger current-contract lifecycle; conservative configured cost is allowed, while exact target-tier calibration and statement evidence remain operational gates |
| `PortfolioExchAcctCalculator.settleSwapFundingAt` linear/inverse formulas | Scheduled funding settlement using realized `settFundingRate`/`prevFundingTime` and exchange mark/depth fallback | Exact signed formulas with stronger accounting semantics; Rust does not book Java's forecast `curFunding` as realized cash |

The borrow decision row is cross-checked specifically against pinned Java
`ChaosEntity.isWithinBorrowLimit`, `OkEntity.getAvailCoinQty`, and
`Iarb2Calculator.checkLiability`. Java permits positive configured borrow limits
and stops only when observed liability exceeds them. Rust keeps that behavior
inside the shared strategy model for parity research, while `LiveConfig` now
requires both configured limits to be zero and the OKX bootstrap/certification
boundary requires borrowing disabled and liabilities zero. The read-only raw
account certification artifact and periodic authenticated config-drift check
have no pinned Java counterpart; they are deliberate production-safety
strengthenings, not claimed strategy parity.

## Account Bill Evidence Cross-Check

The offline economic evidence path is cross-checked against these pinned OKX
Java sources:

| Java reference | Rust implementation | Qualification |
| --- | --- | --- |
| `BillDetails` | Strict `OkxBill` parser for balance, position, price, quantity, PnL, fee, account, instrument, margin, execution, order, bill type/subtype, and timestamp fields, plus current trade/client identities | Rust rejects malformed numeric and enum fields instead of applying Java's zero/NaN defaults |
| `OkexV5BillFetchTaskImpl` | Authenticated account-wide closed-window collector with exact raw pages, bounded pacing, cursor reconstruction, and a required short terminal page | Java fetches one configured type, reverses each response into oldest-first publication, and advances `lastFetchedId`; Rust proves bounded historical completeness for evidence instead of publishing into the strategy loop |
| `OkexV5BillTypes` | Funding type `8`, subtypes `173` and `174` | Exact pinned mapping; Rust additionally validates normal trade type `2` for statement reconciliation and rejects every other bill in the controlled window |
| `OkexV5ExchBillConverter` | Bill identity and economic fields retained in the reconciliation report | Equivalent field boundary, with stricter source/config/account binding and no pooled live message |
| `OkexV5L1Subscriber.tryParseMarkPx` and `MarkPrice` | Normalized `mark-price` frames retain exchange timestamp and price in the canonical journal; funding evidence requires observations on both sides of bill `fillTime` | Java uses a dedicated mark-price session for futures/swaps and publishes exchange/server timestamps; Rust additionally deduplicates redundant sockets and proves a bounded same-session assessment bracket |

The Java samples `BillDetailsSwap.json` and `BillDetailsSwap2.json` also establish
that funding bills can arrive shortly after the scheduled settlement. Rust
therefore uses a bounded causal delay, treats `fillTime` as the assessment
boundary, matches the bill to the session-local journaled realized rate and
latest signed position, and recomputes the linear or inverse contract formula
over two public marks bracketing that boundary. The bill mark must also lie in
that independent range. Reap writes an account/config/account-identity-bound
`session_start` on every runtime start and rejects brackets that cross the next
such boundary. It does not reuse the forecast funding rate or assume
the scheduled funding timestamp is the exact assessment tick. Cash-spot
quantity/currency semantics are a
current API evidence boundary not established by these Java samples; a passing
credentialed demo collection for the exact target account is required before
that path can support production approval.

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
| `loadFirstRunInput` loads `inputPositionFileName`; `updateInputForNextRun` installs `lastResult.getEndingPositions()` after `finishUp()` settles mark-to-market, resets realized/unrealized PnL and derivative average costs at terminal marks, resets balance/margin availability, and releases holds | Strict chain-root opening state seeds balances, available/equity/loan/margin fields, and derivative average-cost positions from schema-8 account certification. Explicit adjacent ordinal ranges of one verified raw capture warm feed state from the prefix and carry independently validated terminal settlement, currency-rate, and funding state; live/pending orders and delayed non-funding actions are reported but not carried | First-run and same-session range continuity are implemented. Rotated files and separate capture processes still cannot prove a gap-free session/global-ordinal handoff, so they remain independent |
| `ChaosContext.tradeSymbols` uses `TreeSet` for cross-run consistency | Stable symbol/risk-group quote and hedge traversal plus ordered portfolio/order reductions | Equivalent determinism intent; exact JSON float round trips and `verify-research` add an independent Rust evidence boundary |

### Live Latency Evidence Cross-Check

The class names remain pinned to
`chaos/chaos-backtest/backtest-core/.../BackTestDelay.java` at Java revision
`b6b120c7b7c466d8431bf082f3229328c5d7b2ae`. The live measurement boundaries
were also checked against these concrete Java paths:

| Java reference | Rust live evidence | Qualification |
| --- | --- | --- |
| `AbstractOkxNitroL2Subscriber.createOrderBookPayloadHandler` records `StartProcessMarketDepth` before JSON parsing | accepted raw `recv_ts_ns` through parse, deduplication, sequencing/book reduction, and entry into the coordinator | Measures the additional target-host path needed after raw replay's receive-time boundary |
| `OkxNitroOrderClient` records websocket send/sent and order-message process stages | dispatch through the account task, pacing, authenticated websocket write, and correlated OKX acknowledgement | Same transport class and a conservative `MatchingNew`/`MatchingCancel` upper bound, not exchange matching-engine time |
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
integrity-checked artifact. `verify-latency-calibration` then re-hashes the exact
source reports, reruns live verification, and reconstructs every class/symbol
series and profile under the recorded options. Only source paths are normalized
to their content hashes, preserving the mapped `BackTestDelay`,
`OkxNitroOrderClient`, and private-state boundaries while allowing evidence to
move into an archive. A schema-8 production research manifest must point to that
artifact, run the byte-identical Reap executable, and use its exact baseline
profile. It also predeclares one deployable candidate and requires every fold to
select it from training data. This is an explicit Rust acceptance control around
the pinned Java `ChaosBackTestMultiRunService` per-run artifact model; the Java
service does not supply a verified held-out candidate-selection policy. The
schema also removes capital from candidate files and derives each capture chain
root's common opening state from a unique source-rebuilt OKX account certification on
the same build, calibrated host, production account, and pre-capture boundary.
Selection therefore cannot compare final account balances as profit. The machinery is
complete; no credentialed target-host artifact or passing
reconstruction has been produced yet.

`verify-research` then re-runs the exact manifest with the byte-identical
executable and compares every fold, selected candidate, Java-mapped execution
scenario, fill, fee, funding, exposure, and gate result. Only source paths
introduced while independently re-verifying capture and opening-account files are normalized to
content hashes. This preserves Java `ChaosBackTestMultiRunService`'s per-run
artifact intent while rejecting stale or forged aggregate JSON. It preserves
independent reset semantics for unchained datasets and independently rebuilds
same-session sequential carry for declared adjacent ranges. It does not supply
missing target-host, queue, statement, and profitability evidence.

`verify-research-deployment` carries the mapped strategy one step further: it
requires the reconstructed deployment candidate's effective `ChaosConfig` hash
to equal `strategy.effective()` from the exact proposed production live config.
The hash function is shared with research candidate provenance, so this is an
identity check rather than a second interpretation of Java strategy fields. The
pinned Java service has no equivalent production-config binding, making this a
documented Rust release control rather than a Java-parity claim.

Failure handling was cross-checked against the same pinned connectivity tree.
`AbstractOkxNitroL2Subscriber.onSocketDisconnected` clears the affected Java
book and its base subscriber resubscribes after a lost session;
`OkxNitroOrderClient` publishes `LOST` connection state and rejects new orders
with `REQUEST_BLOCKED` while disconnected. Its order-session disconnect hook
also invokes `cancelAll`. Rust likewise invalidates feed readiness, blocks
entry, supervises reconnect/recovery, reconciles private state, and preserves
cancellation while entry is blocked. `verify-live-fault-matrix` therefore
requires clean recovered public/private/order-command reconnect roles and zero-order shutdown
for disruptive order-path roles, all on one config/build/host/account identity.
Rust intentionally adds a stronger audit boundary: after a report-capable
runtime exists, an initialization, event-loop, or teardown error completes
fail-closed cancellation/reconciliation, persists the schema-8 failure code plus
pre/post-cleanup evidence, and only then returns the nonzero process error.

The same pinned Java tree makes stop/cancel ordering explicit:
`StrategyEngine.tryToStop` pauses before `cancelAll`, while
`StrategyOrderSender.cancelAll` emits account-level cancel-all requests across
the account's routing groups. Reap's out-of-process emergency command is a
stronger operational boundary for regular OKX orders because it does not depend
on the strategy process or journal and enumerates unmanaged symbols. Its offline
verifier re-derives exact-config, account-coverage, trigger-horizon, and final-zero
invariants; it does not expand the venue scope to algo/spread orders or turn
self-reported REST outcomes into replayable raw exchange evidence.
Deadman heartbeat, periodic clock skew/check, exchange instrument/fee
drift/check, and authenticated account-config drift/check failures have
distinct stable codes. This report does not make the fault acceptable; it makes
the required demo fault campaign machine-reviewable.

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
| `OkxNitroSubscriberBase` 15-second application ping, `SharedSessionSubscriberBase` session ownership, `WsConnectOption` three-miss default, and low-level INIT/ping/pong/data liveness checks | Shutdown/recovery-cancellable 10-second feed handshake, 5-second bounded control writes, 15-second application ping, 30-second no-frame threshold, retained per-socket recovery ownership, reconnect supervision, book/reference freshness, and lossless bounded ready/disconnect delivery | Current-contract hardening: Rust prevents an unbounded handshake/write or closed recovery receiver from stalling/spinning the supervisor. Transport frames establish socket liveness; independently aged strategy and account components prevent pongs from masking stale required data. Low-frequency funding channels are not assigned an invalid global payload cadence. |
| Clear/rebuild book on resubscribe or crossed-book failure | Invalid/crossed-book detection plus sequence state and fresh websocket snapshot recovery | Equivalent with additional explicit sequence validation |
| `AbstractOkV5L2Subscriber` owns `FullOrderBook` and `lastChangeId` by session type, connection index, and symbol; `checkSeqNo` applies predecessor compare-and-set, equal-sequence no-change, and lower-sequence maintenance cases | Each Rust `conn_id` owns a `SequenceTracker` and `BookReducer`; global duplicates still advance source state, valid full books arbitrate canonically, and a mismatch restarts only the failed socket while a ready replica remains available | Exact per-connection continuity rule with bounded recovery buffering and explicit cross-replica conflict handling |
| Nitro checksum validation block commented out; legacy V5 CRC validation active | No CRC validation after OKX checksum deprecation; WSS, sequence, snapshot, crossed-book, and stale checks remain mandatory | Current-contract adaptation |
| Pinned Java subscribers deliver socket payloads to handlers but do not emit a versioned process-global raw-writer ordinal | `capture_record_seq` is assigned before Reap's bounded single writer and verified as exact `1..raw_records` | Rust evidence hardening: proves persisted multi-channel writer completeness separately from OKX book-channel sequencing and is a prerequisite for same-session segmentation |
| Separate receive and exchange latency tracking | Raw `recv_ts_ns`, exchange timestamps, bounded-memory capture timing distributions, capture health counters, and bounded live webhook alerts | Equivalent data retained; receive delay includes host clock/scheduling and alert routing is a deployment concern |
| `MetCoinGatewayMktOkxNitroBaseConfig` configurable `okx.nitro.md.connect.interval.ms` (default zero) | One `connection_attempt_interval_ms` schedule shared through an owner-only advisory-lock file by public/private/order-command, capture, and fault-proxy-upstream initial and reconnect attempts | Current-contract hardening: Rust defaults to 400 ms and official endpoints enforce at least 334 ms for OKX's documented three connection requests/second/IP; the Java creation context remains an in-process reference. Rust coordinates processes on one host, while multiple hosts behind one NAT still require an external coordinator or isolated egress. |
| `OkxNitroExchStatusClient` 10-second `/api/v5/system/status` polling plus `ExchStatusSafeguard` 60-second lead and `OkxNitroUtils.getExchStatus` service filter | Typed unsigned bootstrap and periodic status checks with the same unified service scope, current `env` filtering, endpoint-rate validation, and typed failure | Stronger lifecycle: Java pauses and may resume the strategy; Rust enters fail-closed cancel/reconcile shutdown and requires a clean restart |
| Java strategy entities retain an `Instrument` from `instrumentCache` and use its tick, lot/minimum size, currencies, and contract value throughout their lifetime | Strict exact-row `/api/v5/account/instruments` bootstrap plus paced periodic comparison of those fields and current hard order maxima, typed current `upcChg` parsing, and final limit-order maximum enforcement | Current-contract hardening: missing/unknown rules, non-live state, rule drift, an imminent announced change, or an oversized final order fails closed; the poll is isolated from the deadman heartbeat |
| Java strategy entities receive maker/taker fees from `StrategyToolkit.getTransactionFee`; `ChaosEntity.sanityCheck` rejects taker below maker | Current private-instrument `groupId` plus authenticated `/api/v5/account/trade-fee` `feeGroup` selection for each exact spot symbol or derivative family, startup verification, and paced periodic checks | Current-contract hardening: Rust rejects fee underpricing and unreadable/deprecated-only fee data; the fee poll is isolated from the deadman heartbeat |
| Eight `OkxNitroOrderSessionKeeper` instances with `BY_UNDERLYING` dispatch, websocket order protocol, and `OkxNitroOrderClient.onOrderSessionDisconnected` immediate `cancelAll()` | Eight authenticated command sessions by default, deterministic underlying routing, bounded handshake/login/control/request/acknowledgement lifecycle, aggregate readiness, supervised reconnect, best-effort heartbeat telemetry, and lossless disconnect/fatal delivery | Equivalent command topology and fail-closed disconnect intent; Rust additionally prevents heartbeat backpressure from stalling command IO, classifies the pre-send/write ambiguity boundary, and retains REST for reconciliation and cancel safety |
| Batch subscription manager, per-`WsSubArg` `EVENT_SUB` context updates, and retry limits | Bounded socket partitioning, exact unique serialized subscription-argument acknowledgement-set readiness, acknowledgement timeout, and exponential reconnect | Equivalent identity-bound lifecycle with different batching policy; duplicate acknowledgements are idempotent, while malformed or unexpected acknowledgements fail closed |
| `OrderDetailUpdate` `tradeId`/`fillSz`/`fillPx` and `UserTrade` `tradeId`/`fee`/`feeCcy`/`execType` | Current OKX order-channel `tradeId` plus per-fill `fillFee`/`fillFeeCcy`, optional fills channel with fee-bearing order-update precedence under either arrival order, instrument-scoped once-only journal record, and raw-statement comparison | Current-contract strengthening; the pinned Java order-session fill derivation says fee is unavailable and its separate user-trade processing is commented out |
| `OkxNitroRestClient.getFills` 100-row pages, 200 ms pacing, descending cursor pagination, and short-page completion | Authenticated read-only `/api/v5/trade/fills` collector with the same page size/pacing/completion rule, fail-closed page bound, raw response retention, bracketed account identity, and offline cursor replay | Lifecycle mapping with a current-contract endpoint adaptation; pinned Java reads Nitro spread trades while Reap certifies regular strategy fills |
| `StaleOrderUpdateSafeguard` and `GatewayOrderStatsSafeguard.BookTotalPendingOrder` | Per-order submit-to-private-state and cancel-to-terminal-state deadlines, cancel retry, account block, and full REST reconciliation | Equivalent fail-closed intent with explicit OKX REST/private-state convergence |
| `PositionGatewaySafeguard.OrsFillDelayToPositionUpdate` and the fill-derived position reconciler | Per-fill derivative-position or spot-currency convergence deadline plus monotonic rows, full REST comparison, tombstone repair, and second-pass confirmation | Equivalent fail-closed intent; Rust uses an explicit post-fill deadline rather than Java's two-cycle timestamp-difference check |
| `PositionGatewaySafeguard.PosnMgnModeMismatch` | Required OKX `mgnMode` on websocket/REST positions, configured-mode bootstrap/runtime enforcement, and reconciliation comparison | Equivalent fail-closed intent; Rust aborts the live lifecycle instead of pausing inside a separate gateway process |
| `PositionGatewaySafeguard.ForcedRepaymentIndicator` | Typed OKX balance `twap`, configured `1..=5` limit, bootstrap/runtime account-state rejection, tombstone clearing, and reconciliation comparison | Stronger: Java emits alert-only, while Rust fails closed at or above the configured level |
| `ChaosStrategyEngine.tryToStop`, `ChaosStrategyBase.cancelAll(entity)`, `ExchCancelAll`, and `OkxNitroOrderClient.onOrderSessionDisconnected` | In-process canonical cancel/reconcile plus a separate account-wide regular-order emergency CLI | Equivalent normal/disconnect cancellation with an additional process-independent final-zero safety layer |

`reap-fault` has no Java strategy counterpart and is not a parity claim. It is a
separate demo test process built to exercise the mapped connectivity behavior:
`OkxNitroL2SubscriberGroupFactory` and `AbstractOkxNitroL2Subscriber` market-data
recovery, `OkxNitroBatchSubscribeManager` subscription lifecycle, and the eight
`OkxNitroOrderSessionKeeper`/`OkxNitroOrderClient` command-session loss path. The
routed Rust config gives order commands their own loopback endpoint while normal
configs still share the private endpoint. This preserves the Java-inspired
transport roles and lets one campaign disconnect public, private-state, or order
traffic independently; the stronger typed injector artifact and loopback-only
upstream policy are Rust testing controls.

The proxy's scenario bindings preserve the same ownership boundaries. Dropping
an exchange-to-client private `orders` frame exercises the convergence deadline
mapped to Java `StaleOrderUpdateSafeguard`; dropping `positions` for derivatives
or `account` for spot exercises the boundary mapped to
`PositionGatewaySafeguard.OrsFillDelayToPositionUpdate`. The status-response
template targets the endpoint polled by `OkxNitroExchStatusClient`, while the
order route exercises `OkxNitroOrderClient` independently of private account
state. Cancel All After, exchange-clock, authenticated instrument/fee/account
guards, and durable restart latches remain intentional Rust safety hardening,
not Java parity claims. A proxy frame drop cannot create a genuine partial fill,
and a running proxy cannot prove latch persistence across process restart, so
the matrix rejects typed proxy artifacts for those two roles.

The reviewed Java OKX subscriber and `chaos-iarb2` classes do not provide the
Rust runtime's place-request `expTime`, `/public/time` skew gate, Cancel All
After heartbeat, fsynced restart-latch lifecycle, exclusive journal lease,
bounded authenticated fill-evidence collection and verification, webhook alert
worker, Linux host guard, hermetically verified capability-free supervisor
policy, or strategy-independent account-wide regular-order cancellation. Those
are intentional
deployment-safety additions around the parity strategy, not claims of Java
strategy equivalence. Re-check exchange-facing controls against both the pinned
Java revision and the current OKX API contract whenever connectivity is
upgraded.

The pinned Java subscriber clears its socket-owned book and stale-check state in
`AbstractOkxNitroL2Subscriber.onSocketDisconnected`. Rust has shared redundant
books rather than one book per socket, so it retains the healthy replica but now
waits for bounded status capacity when publishing every `Ready` or
`Disconnected` transition. Per-frame payload heartbeats are not placed on that
queue. Schema-8 live reports retain total, public, and private disconnect counts
so a demo reconnect campaign can prove which transport class was exercised.
Schema 8 additionally records ambiguous submit/cancel outcomes, partial-fill
transitions, order/fill convergence timeouts, and restored durable latches.
Startup-replayed order state is excluded from the per-session outcome counters.

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

The pinned Java `MetCoinGatewayUrlOkxNitroConfig` and
`MetCoinGatewayUrlOkxNitroSimConfig` accept Spring-provided host, port, TLS, and
base-path values; simulation adds `x-simulated-trading: 1`. They do not enforce
an exchange-host allowlist in those configuration classes. Rust intentionally
strengthens this deployment boundary: authenticated REST/public/private URLs
must form one documented OKX region/environment tuple, and the emergency parser
reuses the same REST-origin policy. This is a credential-exfiltration guard,
not a claimed Java behavior match. A structured demo-to-production verifier
then requires the Java-mapped strategy/risk/account/runtime settings to remain
unchanged while only reviewed deployment bindings move.

The aggregate production-evidence bundle is also intentionally outside the Java
parity claim. At pinned revision
`b6b120c7b7c466d8431bf082f3229328c5d7b2ae`,
`ChaosBackTestMultiRunService` loads day-specific strategy input, runs days in
sequence, carries ending account positions, and rotates output filenames. It has
no held-out candidate-selection, exact live-config binding, target-host identity,
account certification, fault-matrix, or release-decision layer.
`MetCoinGatewayWsClientsOkexV5Config` remains a connectivity reference for the
separate market-data, position, and order websocket clients plus round-robin/hash
dispatch. It likewise does not compose operational evidence. Rust continues to
map strategy and connectivity behavior to those sources while treating
`verify-production-evidence` as a separate fail-closed deployment control.

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
cargo fmt --all -- --check
cargo clippy --workspace --all-targets --locked -- -D warnings
cargo test --workspace --locked --no-fail-fast
cargo build --release --locked -p reap-cli
cargo audit --deny warnings
```

These commands run in least-privilege GitHub CI under the repository's exact
Rust toolchain. This verifies the Rust implementation and dependency lockfile;
it does not execute the private Java build or establish exchange certification.

The local Java source was cross-checked directly. Running its tests additionally
requires the private Java build environment and dependencies; this workspace
does not include Maven or a Maven wrapper.
