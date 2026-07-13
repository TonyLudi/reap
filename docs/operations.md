# Operations Guide

`reap` fails closed: missing or stale state can trigger cancels in demo mode but
cannot authorize a new order. `reap live` owns the implemented OKX lifecycle;
production order entry remains intentionally unavailable.

## Public Data Capture

`reap capture` is a separate composition root. It opens only the configured
public websocket URL, reads no credentials, creates no private subscription,
and has no dependency on the order gateway or strategy coordinator.

```bash
mkdir -p var/reap/capture
RUN_ID="$(date -u +%Y%m%dT%H%M%SZ)"
RAW_PATH="var/reap/capture/okx-btc-${RUN_ID}.jsonl"
cargo run -p reap-cli -- capture \
  --config examples/capture-okx-public.toml \
  --raw-path "$RAW_PATH" \
  --duration-secs 3600 \
  --require-clean-capture \
  --pretty
```

The example captures spot and swap books/trades plus index, funding, mark, and
price-limit inputs used by iarb2. Every subscription has two independent
connections. Raw frames are canonical and written sequentially within one run.
The optional
`output.normalized_path` is intended for short diagnostics because full
400-level snapshots are much larger than raw deltas.

Every frame and run report carries a generated `capture_session_id`. Raw and
normalized outputs use create-new semantics: startup refuses either existing
path and never appends a second process session. Use a unique output path for
each process. Strict replay and raw backtest also reject files containing more
than one session ID, because process downtime is not a continuous HFT market
stream.

Book deduplication keys exact redundant images by action, `prevSeqId`, `seqId`,
exchange timestamp, and raw-payload hash. A replica conflict is not suppressed;
it fails predecessor validation and forces recovery. Continuity requires
`prevSeqId` to equal the last accepted `seqId`; the next `seqId` may be equal for
a no-change update or lower after exchange maintenance. Those valid cases increment
`same_sequence_updates` or `sequence_resets`. A predecessor mismatch remains a
gap and forces fresh-snapshot recovery.

OKX has deprecated the order-book checksum and documents that the field remains
zero. Capture integrity therefore relies on WSS transport, sequence links,
fresh snapshots, crossed-book rejection, age checks, and strict replay rather
than the old CRC algorithm. See the current [OKX API guide](https://www.okx.com/docs-v5/en/)
and [checksum deprecation announcement](https://www.okx.com/en-us/help/okx-order-book-channels-checksum-field-deprecation).

`clean_capture` requires a bounded duration, all socket plans ready at least
once and at stop, a ready contiguous snapshot for every configured book,
non-empty raw output, and zero parse, gap, stale-book, recovery-request,
recovery-route, or recovery-failure counts. A redundant-socket disconnect can
remain clean only when the other replica preserves sequence continuity;
inspect disconnect, duplicate, and writer queue counts on every run.

The versioned run report includes the effective config fingerprint and SHA-256
of the exact bytes emitted by each writer. Archive that report with the capture
manifest. `analyze-capture` independently hashes the input and fingerprints the
provided config after applying the input path override, so both values must
match the original run report.

Validate and consume the output directly:

```bash
cargo run -p reap-cli -- analyze-capture \
  --config examples/capture-okx-public.toml \
  --events "$RAW_PATH" --strict --pretty
cargo run -p reap-cli -- replay-check \
  --events "$RAW_PATH" --strict --pretty
cargo run -p reap-cli -- backtest \
  --config examples/iarb2-okx-btc.toml \
  --data "$RAW_PATH" \
  --format raw-capture --require-complete-accounting --pretty
```

### Backtest Execution Assumptions

Strategy TOML may include:

```toml
[backtest]
calibrated = false
market_data_latency_ms = 0
order_entry_latency_ms = 0
cancel_latency_ms = 0
order_update_latency_ms = 0
fill_account_latency_ms = 0
depth_fill_conservative_threshold = 0.0001
queue_ahead_multiplier = 1.0
historical_trade_fill_fraction = 1.0
displayed_depth_fill_fraction = 1.0
```

Scalar latency values are backward-compatible fallbacks. To replay bounded
uniform empirical samples by Java-mapped class and optional instrument, add:

```toml
[backtest.latency_profile]
seed = 20260713

[[backtest.latency_profile.rules]]
class = "market_depth"
samples_ms = [1, 1, 2, 3]

[[backtest.latency_profile.rules]]
class = "market_depth"
symbol = "BTC-USDT-SWAP"
samples_ms = [1, 2, 2, 4]

[[backtest.latency_profile.rules]]
class = "matching_new"
samples_ms = [8, 10, 14, 25]
```

Rules support `market_depth`, `historical_trade`, `reference_data`,
`matching_new`, `matching_cancel`, `order_update`, and `order_fill`. A symbol
rule takes precedence over its class-wide rule, then the scalar field is used.
Duplicate sample values preserve empirical weight. Rules, samples, symbols, and
latency magnitudes are bounded and validated before replay. The deterministic
sampler sorts values and couples a stable seed/class/symbol/event ordinal to a
quantile. `latency_usage` reports actual count, total, minimum, maximum, and mean
for every sampled class/symbol. Symbol rules must name a configured instrument,
reference symbol, or instrument index symbol; unknown names fail construction
instead of silently falling back.

Raw replay orders the local event loop by persisted `recv_ts_ns`; CSV and
normalized replay use event timestamps. The runner applies market data first
when an activation or cancel is due at the same scheduler instant. It stops at
the last observed input instead of executing future actions against stale depth.
Always inspect `input_clock_regressions`, `live_orders`, `max_active_orders`,
maximum/final active-order notional, maximum/final position and pending delta,
`pending_scheduled_actions`, and the five pending-action category counts.
Concurrent socket tasks can reach the single raw writer in a different order
from their receive stamps even when each source is monotonic. Replay clamps
those global regressions, preserves file order as the tie-breaker, and reports
their exact nanosecond count and maximum; investigate large values separately
from the analyzer's per-source monotonicity gate.

`calibrated = true` is an evidence declaration, not a behavior switch. Set it
only when the values are derived from representative target-host processing and
demo order traces. Guessed values may be used with `calibrated = false` for
sensitivity tests. Raw replay already begins at persisted local `recv_ts_ns`, so
do not add exchange-to-host receive delay again as market latency. Measure the
additional receive-to-strategy path on the target host; use demo traces for
matching, cancel, private order, and fill/account classes. Archive the exact
TOML, raw SHA-256, code revision, report, and source distributions. The example
threshold is inherited from the pinned Java backtest:
a resting sell needs bid at least `order_px * (1 + threshold)`, while a resting
buy needs ask at most `order_px * (1 - threshold)`. A shallower cross clears
queue-ahead without filling. Zero latency and an inherited threshold preserve
parity scaffolding but cannot support a capital decision.

The three capacity controls default to Java-parity assumptions: exact displayed
queue, all historical trade quantity, and all displayed depth. For conservative
sensitivity runs, increase `queue_ahead_multiplier` and reduce either fill
fraction. These are global deterministic haircuts, not a reconstruction of
exchange queue priority, hidden liquidity, cancellation flow, or competing
orders. Haircutted displayed capacity is shared across matching actions; an
unchanged depth image does not replenish it, while new levels and displayed
size increases add capacity. Keep `calibrated = false` until representative
private order traces and full-depth captures justify per-venue and
per-instrument values.

For every run, also inspect `fee_cost_usd`, `funding_pnl_usd`,
`turnover_usd`, `funding_rate_events`, and `accounting_complete`. Funding uses
the latest rate for each advertised settlement time. A first rate up to 60
seconds late is applied immediately but marks accounting incomplete; older
first rates are counted as missed and are not applied to a potentially changed
position. Invalid data or a missing mark for a nonzero swap position also makes
accounting incomplete. A future funding settlement beyond the capture horizon
is reported as `pending_funding_actions` and is not itself a defect for the
observed interval.

The portfolio starts at zero and treats USD-equivalent quote currencies at par;
it does not model depeg-sensitive valuation, borrowing interest, liquidation,
margin discounts, taxes, or the future USD drift of coin-denominated fees. Live
stablecoin guards are not a substitute for that backtest limitation. A research
acceptance run must span held funding boundaries, use the target fee tier, and
reconcile fill, fee, funding, cash, equity, and active-order notional attribution
to demo account statements before profitability metrics are trusted.
Use `--require-complete-accounting` in automated research runs so any reported
accounting defect also makes the command fail.

### Walk-Forward And Sensitivity Research

Verify the manifest-to-report path without claiming profitability:

```bash
rm -f /tmp/reap-research-smoke.json
cargo run -p reap-cli -- research \
  --manifest examples/research-smoke.toml \
  --output /tmp/reap-research-smoke.json --require-pass --pretty
```

The smoke manifest has one tiny fold, one candidate, uncalibrated execution,
permissive risk gates, and negative fee-adjusted baseline PnL. Its only purpose
is testing candidate selection, chronology, stress propagation, provenance,
artifact creation, and exit status.

For real research, create a new manifest alongside immutable capture files:

1. Set `mode = "production_candidate"` and retain the pinned Java revision.
2. List explicit full candidate TOML files; the runner does not mutate arbitrary
   strategy fields or generate an implicit parameter grid.
3. Give every capture a unique dataset ID/path/content hash. Every test window
   must occur strictly after its fold's training windows, and a test dataset can
   belong to only one fold. Set each raw dataset's `capture_config` to the exact
   capture-only TOML used for that session. Production mode reruns and retains
   strict capture analysis plus a zero-gap raw replay check before candidate
   evaluation. It also requires at least two connections per stream and the
   book/trade/index/mark/limit/funding channels needed by every candidate.
4. Define exactly one baseline using empirically calibrated execution values
   and at least two stress scenarios. Profile stress uses the same seed and must
   first-order stochastically dominate baseline for every effective class/symbol
   distribution. Threshold and queue can only increase; trade/depth
   participation can only decrease.
5. Set nonzero event, fill, and duration evidence minimums plus explicit PnL,
   drawdown, terminal/maximum position and pending-hedge delta,
   terminal/maximum gross position and active-order exposure, active-order
   count, inventory-duration, clock-regression, accounting, and pending-work
   gates.
6. Run with a create-new `--output` path and `--require-pass`, then archive the
   JSON beside the exact capture/config files and demo calibration evidence.

Candidate scores use training runs only (`net_pnl_usd` or
`pnl_per_turnover_bps`). Only the selected candidate is evaluated on test data.
The report contains the selection rule, gate thresholds, every underlying run,
aggregate gate failures, and SHA-256 for the manifest, executable, candidate
files, effective strategies, and datasets. Candidates with identical effective
strategy configuration are rejected even when file comments or `[backtest]`
settings differ. Every file-backed input hash is checked again after all runs so
input mutation aborts artifact creation. Existing output paths are refused so an
acceptance artifact cannot be silently replaced.

The embedded capture analysis binds the capture-config file hash, expected
subscriptions, redundant source coverage, per-source timing, depth, session,
and data hash to the dataset. The independent raw replay check also proves no
sequence gap/recovery/failure and ready terminal books. File analysis cannot
reconstruct runtime writer queue pressure, dropped-record counters, or the
process exit path, so archive the original clean bounded-capture run report as
separate required evidence.

The three pending-work maxima bound delayed non-funding scheduler work,
exchange `PendingNew` orders, and unresolved cancel requests at a data cutoff.
They need not be zero: positive calibrated latency naturally censors the last
captured event, and the runner does not execute future actions against stale
depth. Resting live quotes are also valid at an arbitrary dataset boundary;
bound them with `maximum_test_active_orders`, active-order notional gates, and
pending-delta gates instead of treating them as completed work.

Each dataset is an independent zero-initial-portfolio run. The report states
`independent_zero_initial_portfolio`; unlike Java
`ChaosBackTestMultiRunService`, Rust does not carry ending positions between
files. Use a single continuous capture for an evaluation segment when position
continuity matters and gate terminal delta/gross exposure. Never sum cross-file
PnL as though positions had been carried.

Strict analysis requires one capture session, every configured stream on its
configured number of source connections, a ready book for every configured
book stream, no unexpected data stream, monotonic per-source receive time, and
no parse, sequence, or unrecovered-book defect. It reports
receive and exchange cadence, signed receive-minus-exchange delay, book depth,
spread, absolute midpoint movement, trade quantity, and price-times-quantity.
Quantiles use a deterministic 8,192-value reservoir per metric; counts, means,
and bounds cover every finite sample. Signed receive delay includes host clock
skew and scheduling and is not websocket latency or order round-trip time.
For derivatives, price-times-quantity is in contract units and is not USD
notional without instrument metadata. Integrity success does not establish
dataset duration, execution calibration, or strategy profitability.

Run capture under a disk-capacity supervisor. JSONL is currently uncompressed
and can grow quickly; rotate on every process start and before the filesystem
reaches its alarm threshold. Never place capture output under source control.

### Recorded Public Acceptance

On 2026-07-13, the baseline public configuration completed a bounded 5-minute
run with all 12 redundant socket plans ready at stop. It wrote 36,402 raw
frames (29,497,890 bytes), accepted 18,201 events, and classified 18,201
byte-identical replica frames as duplicates. Capture and strict replay both
reported zero gaps, recoveries, recovery failures, parse errors, stale books,
disconnects, and unrecovered streams. Raw-capture backtest replay completed
with 84 simulated orders and no fills. The raw SHA-256 was
`d47821b16b6fbbd78b6058a701678074f988a863db68110a18ada74042330c58`.

After adding the stablecoin risk inputs, the current public configuration ran
for 75 seconds with all 14 socket plans ready. It wrote 7,592 frames (5,687,373
bytes), accepted 3,803 events, and classified 3,789 as duplicates, with the
same zero-defect capture and strict-replay result. The file contained 713
USDT/USD and USDC/USD frames: 142 unique USDT updates and 215 unique USDC
updates, with zero conflicting values at the same symbol/timestamp.
Raw-capture backtest replay completed with 20 simulated orders and no fills.
The raw SHA-256 was
`45e666b9b633696cce739d7c4bb029247306839f4994e186c138ebf8ed2ab145`.

A fresh 45-second end-to-end smoke on 2026-07-13 used the current capture-only
configuration and again reached all 14 plans. It wrote 4,856 frames (4,133,829
bytes), accepted 2,429 events, classified 2,427 replica duplicates, and had
zero gaps, recoveries, parse errors, stale books, or disconnects. Strict replay
reproduced both ready books with no errors, and raw-capture backtest replay
completed with 16 simulated orders and no fills. The raw SHA-256 was
`f8e4ab61946c113162263733f35c42329d6120712e2b1ee66afbb616cdbb9453`.

After adding provenance and analysis, a fresh 60-second run on 2026-07-13
reached and retained all 14 socket plans, wrote 5,633 frames (4,311,684 bytes),
accepted 2,822 events, classified 2,811 replica duplicates, and reported a raw
writer queue high-water mark of 22. Capture, strict analysis, and strict replay
reported zero gaps, recoveries, recovery failures, parse errors, stale books,
disconnects, or unrecovered books. Analysis observed both configured sources
for every one of the ten streams and reconstructed 400 bid and ask levels for
both books. Writer, analyzer, and independent SHA-256 values matched at
`0054cd3daf322cecd03c08e6e5d93ce8051534d1cc8a4ed128f58801645109b8`;
capture and analyzer config fingerprints matched at
`48b582c00352efce667114449b8f10242ee2737a8ee2fd97a68d0db1d2acf3c8`.
Raw-capture backtest replay produced 14 simulated orders and no fills. After
the deterministic latency scheduler was added, receive-time replay of the same
file reported 3,958 normalized inputs, three cross-socket writer-order clock
regressions (maximum 148,017 ns), 14 exchange activations, 10 effective
cancellations, four live quotes at the capture horizon, and no pending scheduled
order actions under the explicit uncalibrated zero-delay configuration with the
pinned Java `0.0001` depth threshold and 100% capacity fractions. The two
funding-rate updates produced one scheduled funding action beyond the dataset
horizon; no fee, turnover, funding settlement, or funding PnL was generated,
and accounting was complete through the observed interval. An uncalibrated
sensitivity pass using 2 ms market,
20 ms entry, 15 ms cancel, 25 ms order-update, and 50 ms fill/account delays
plus 2x queue-ahead, 25% historical-trade participation, and 50% displayed-depth
capacity kept the same no-fill result and exposed three delayed strategy events
plus the future funding action beyond the capture horizon. This short,
fill-free run validates scheduling/provenance and assumption propagation only;
it cannot calibrate any latency or fill assumption.

The research runner then used the 45-second file as a training window and the
later 60-second file as its test window. It independently reran config-bound
capture analysis and raw replay for both inputs: all ten expected streams had
complete redundant source coverage, both analyses were healthy, and both
replays had zero gaps, recoveries, or errors. Baseline and latency/capacity
stress runs remained fill-free; the latter retained three horizon-censored
strategy events within its explicit pending-work limit. The permissive smoke
artifact passed and has SHA-256
`f8f6caf0e733d156a8fbd4adc21fd2fc669a1e00629c5cf944721c8bd1c08780`.
This validates research orchestration and evidence binding only, not candidate
quality or a production gate.

These are connectivity, integrity, and replay-plumbing evidence only. They are
not a sustained full-depth dataset, execution-model calibration, profitability
result, credentialed soak, or production approval. Raw acceptance files remain
outside source control.

## Startup Gate

1. Run `reap live --config <path> --mode validate`. This reads no credentials
   and opens no network connection.
2. Start `--mode observe` with credentials supplied through the configured
   environment-variable names. Observe mode permits neither submits nor
   cancels.
   The example uses the global simulated hosts documented in the
   [OKX API guide](https://www.okx.com/docs-v5/en/); replace REST, public, and
   private domains together when the account belongs to another region.
3. Before recovery, credentials, or network setup, the runtime canonicalizes
   the journal path and exclusively locks its sibling `<journal>.lock` file.
   When enabled, the Linux host guard then checks journal-filesystem space,
   available memory, and kernel clock synchronization.
4. The runtime recovers the critical JSONL log and binds its checkpoint to the
   strategy/config fingerprint. It rejects safety latches for accounts or
   symbols not owned by the current config. Do not share a storage path between
   strategy configs.
5. It compares midpoint-adjusted local time with OKX `/api/v5/public/time` and
   fails bootstrap when skew exceeds `max_exchange_clock_skew_ms`.
6. The runtime fetches account-scoped instruments, account configuration,
   balances, positions, open orders, recent fills, and exact status for any
   restored active order.
7. It verifies live instrument state, type, linear/inverse contract type,
   tick/lot/minimum size, contract value, currencies, configured trade mode,
   account level, and `net_mode` before metadata is ready.
8. It restores canonical active orders, fill identities, and active global,
   account, and symbol safety latches from JSONL. It applies missed known
   fills/terminal updates from REST, applies the authoritative account snapshot
   to the strategy and risk engine, and marks that snapshot ready only after
   engine application. It then requires clean account-scoped reconciliation.
9. In demo mode it arms OKX Cancel All After for every account before starting
   that account's private feed or order task.
10. It starts redundant public plans and isolated orders, account, and positions
   sockets for every account. The dedicated fills channel is optional for
   eligible fee tiers; order-channel fills remain canonical. Transport
   acknowledgements alone are not private readiness: account and positions must
   each deliver a real data payload. Every transition of that complete private
   state to ready invalidates the earlier reconciliation and requires a fresh
   REST order/fill/balance/position check, closing the bootstrap-to-stream
   subscription window.
11. It waits for a contiguous sequenced book for every instrument and a
    complete, healthy, post-connect reconciled private stream for every account.
12. It also waits for every configured stablecoin guard to receive a fresh,
    internally consistent index value within its downside threshold.
13. Only phase `ready`, writable storage, healthy risk, and explicit
   `--mode demo --confirm-demo` permit a new order.

Live validation rejects `strategy.master_strategy` and
`strategy.strategy_group`. The pinned Java implementation requires external
`StrategyUpdate` heartbeat, member-state, and aggregate-PnL inputs for those
settings; accepting only their static flags would weaken live stop behavior.

## Private Stream Health

- Orders, account, positions, and optional fills use isolated sockets. Every
  socket must acknowledge its subscription; any disconnect immediately emits
  account-scoped `PrivateStreamStale` and blocks new orders.
- The [OKX API guide](https://www.okx.com/docs-v5/en/) documents account and
  positions as initial/regular state channels, while orders and fills are
  event-only. Reap therefore requires one valid account payload and one valid
  positions payload to complete each private-health round. Empty state arrays
  count as valid data frames; subscription and connection-count control frames
  do not.
- Ping/pong proves only that one transport is responsive. Pongs, order updates,
  and fill updates never refresh aggregate account-state health. Repeated data
  from one state channel cannot mask silence on the other.
- A completed account/positions round emits `PrivateStreamRecovered` initially
  or after a disconnect, and `PrivateStreamHeartbeat` thereafter. Risk marks
  the account stale when no complete round arrives within `max_private_age_ms`.
  The demo example uses 30 seconds, above OKX's documented regular state-push
  cadence. Recovery after a transport loss also requires fresh REST
  reconciliation before readiness returns.

## Account State Reconciliation

- Startup, private-stream recovery, ambiguous gateway outcomes, and shutdown
  fetch open orders, recent fills, account balances, and all positions under
  the reconciliation request pacer. An empty balance response is rejected as
  non-authoritative; an empty position list is a valid flat account.
- The runtime compares the websocket-derived state before applying REST. Any
  order, fill, balance-field, position-quantity, or open-position average-price
  difference emits account-scoped reconciliation drift and cancels live orders.
- REST then replaces the account reducer and is delivered through the same
  strategy/risk event path. Locally present balances or positions omitted by
  REST generate zero tombstones, so a closed position cannot remain in the
  engine. Older per-currency, per-symbol, or margin websocket rows are ignored.
- Repair is not proof of agreement. A dirty pass keeps the account degraded and
  retries; only a subsequent clean full-state pass restores the reconciliation
  gate. This intentionally favors a brief false stop over trading stale state.

## Order-State Convergence

- OKX's event-only orders channel sends no initial snapshot. Its current API
  contract also states that a successful cancel response accepts the request
  but does not prove the order is cancelled; final state comes from the orders
  channel or an order query. See the [OKX API guide](https://www.okx.com/docs-v5/en/).
- Every locally registered `PendingNew` must advance to a private or recovered
  REST state, and every dispatched cancel must advance to `filled`, `cancelled`,
  or `rejected`, within `runtime.order_state_convergence_timeout_ms`. The limit
  must cover `rest_request_timeout_ms`; the demo baseline is five seconds.
- A timeout emits account-scoped `ReconcileDrift`, blocks new entry, cancels all
  canonical active orders, and starts full REST reconciliation. An expired
  cancel is removed from in-flight deduplication first, so the fail-closed path
  retries it instead of suppressing it as a duplicate.
- A nonterminal REST recovery does not clear an outstanding cancel. Terminal
  private or REST state clears it. Failed reconciliation remains degraded and
  follows the existing bounded retry policy.
- This maps the pinned Java `StaleOrderUpdateSafeguard` and pending-order gateway
  control into the single-owner runtime. Demo testing must suppress submit and
  cancel order pushes separately and verify cancel retry, REST repair, and no
  readiness recovery before authoritative convergence.

## Fill-To-State Convergence

- Every deduplicated canonical fill starts account-scoped pending targets.
  Derivative fills require a covering update for that instrument's position;
  spot fills require covering updates for both base and quote balances.
- Coverage uses exchange update time and also handles the case where the state
  channel is processed before the order channel. Aggregate account/positions
  health rounds, socket pongs, unrelated currencies, and unrelated symbols do
  not clear a target.
- If any target remains pending for `fill_state_convergence_timeout_ms`, the
  event loop emits `ReconcileDrift`, blocks the account, cancels its canonical
  live orders, and requests full REST reconciliation. It reports once while the
  account is unresolved; normal reconciliation retry policy handles REST
  failure. An authoritative account snapshot clears pending targets.
- This guard is transient rather than a durable operator latch. Readiness can
  return only through the normal private-health and clean-reconciliation gates.
  A demo fault campaign must delay or suppress both derivative and spot state
  updates and verify the drift alert, cancels, REST repair, and clean recovery.

## Position Scope And Margin Mode

- OKX REST positions and private position updates must carry `mgnMode` as
  `cross` or `isolated`. Reap retains that field in canonical position state.
- Live spot symbols must use `cash` trade mode. Every nonzero position returned
  for an account must be a configured derivative owned by that account. A
  margin-spot position, a symbol assigned to another configured account, or an
  unmanaged symbol is a fatal policy violation because strategy/risk cannot
  safely value that exposure.
- Every nonzero managed derivative position must match the symbol's configured
  account trade mode. A mismatch aborts bootstrap or the running live
  lifecycle before applying the bad position; demo shutdown then enters the
  normal cancel/reconciliation path and does not disarm the exchange deadman
  unless cleanup proves zero orders and clean state.
- Zero positions do not fail this check, allowing a closed mismatched position
  or foreign position to report closure. Full-state reconciliation independently
  reports a websocket/REST margin-mode disagreement for any nonzero position.
- Treat any scope or mode violation as account configuration or external-position
  contamination, not a reconnect issue. Keep order entry stopped, inspect and
  close or correct the OKX position outside Reap, and start again from a clean
  bootstrap. Use a dedicated subaccount for this strategy.

## Private Order Identity And Account Scope

- Accepted REST submit and cancel acknowledgements bind the exchange order ID
  to the already registered client order ID. The acknowledgements are journaled;
  active bindings are reconstructed before startup REST reduction, and
  contradictory journal history aborts startup. Private order/fill rows with an
  empty client ID, and the OKX fill sentinel `"0"`, resolve through that binding
  in live processing, restart recovery, cancel convergence, and full REST
  reconciliation.
- Before mutation, every private order and fill symbol must route to the account
  that delivered it. A known order's symbol and side are immutable. Neither an
  exchange order ID nor a client order ID can be rebound to a different peer.
  The same checks run before fill IDs or cumulative quantities are recorded.
- A correctly scoped but unknown private order still enters canonical state and
  immediately requests reconciliation so fail-closed cancellation can discover
  it. Wrong-account, conflicting-binding, symbol-change, and side-change rows
  are fatal lifecycle errors; the raw frame is already journaled, no corrupted
  canonical state is applied, and normal runtime failure cleanup cancels and
  reconciles managed accounts.
- OKX may repeat order-channel messages with a different `uTime`. Reap suppresses
  a repeated fill ID when status is unchanged and cumulative fill does not
  advance, and suppresses an unchanged terminal status by canonical order ID. A
  genuine state transition or cumulative-fill increase still reaches the event
  loop. This follows the current [OKX order-channel duplicate guidance](https://www.okx.com/docs-v5/log_en/#order-channel-revamp).

## Forced Repayment Risk

- OKX account websocket and REST balance rows expose `twap`, a forced-repayment
  risk indicator from `0` through `5`. Reap retains the per-currency value; see
  the current [OKX balance contract](https://www.okx.com/docs-v5/en/#trading-account-rest-api-get-balance).
- Set `risk.forced_repayment_indicator_limit` to `1..=5`. The default and demo
  baseline are `1`, so any nonzero indicator blocks bootstrap or aborts the
  running live lifecycle before applying that account row.
- This is intentionally stricter than the pinned Java safeguard, which is
  alert-only. Demo cleanup enters the normal cancel/reconciliation path and
  leaves Cancel All After armed unless it proves zero orders and clean state.
- REST reconciliation compares the websocket and REST indicator, treating an
  omitted value as zero. An authoritative omitted-currency tombstone clears a
  prior value. Keep entry stopped until OKX reports every currency below the
  configured limit and a clean bootstrap succeeds.

## Active Order Count Limits

- `risk.max_live_order_count` caps all canonical `PendingNew`, `Live`, and
  `PartiallyFilled` orders. `risk.max_live_order_count_per_symbol` applies a
  second ceiling to each symbol and must not exceed the global value. Both must
  be positive.
- Pre-trade risk includes the proposed order and rejects it before registration
  if either projected count would exceed its limit. This complements notional
  limits because many minimum-size orders can exhaust local queues or exchange
  order capacity while carrying little aggregate notional.
- Canonical private websocket and REST-recovered order state is authoritative.
  If it pushes actual count above either ceiling, post-trade risk persists a
  global risk latch and cancels active orders. This is stricter than the pinned
  Java per-entity pause action. A remote-only order is not admitted into
  strategy/risk state; full-state reconciliation reports it as drift and keeps
  the account blocked independently of these count limits.
- The demo baseline is 64 active orders globally and 16 per symbol. Review the
  limits against configured quote levels, hedge concurrency, account-wide
  exchange limits, and shutdown cancellation capacity before credentialed use.

## Exchange Order Failure Circuits

- `risk.order_reject_count_limit` and
  `risk.order_reject_count_per_symbol_limit` bound canonical exchange submit
  rejections inside `risk.order_reject_window_ms`. All settings must be
  positive, and the symbol threshold cannot exceed the global threshold. The
  demo baseline is five global or three same-symbol rejects in 60 seconds.
- Canonical `Rejected` updates include explicit non-ambiguous REST submit
  failures and private `order_failed` state. An order ID counts once within the
  rolling window; the event loop uses monotonic observed rejection time so an
  out-of-order exchange timestamp cannot corrupt expiry ordering.
- Reaching either threshold persists the global risk latch and cancels every
  canonical active order. This maps the pinned Java submit-reject controls into
  the single-owner risk gate and is intentionally stronger than a transient
  strategy pause.
- `risk.unfilled_ioc_cancel_count_per_symbol_limit` bounds distinct canonical
  IOC cancellations with exactly zero cumulative fill per symbol inside
  `risk.unfilled_ioc_cancel_window_ms`. Both settings must be positive. The demo
  baseline is three zero-fill IOC cancellations on one symbol in 60 seconds and
  must be calibrated from demo execution evidence before production use.
- Canonical order updates retain an optional time-in-force copied from locally
  registered order state. The optional serde field keeps older JSONL journals
  readable. Private updates and repeated websocket frames cannot erase the
  local value, and a given order ID counts once inside the rolling window.
- Partially filled IOC cancellations do not enter this Java-equivalent counter;
  chaos still records their residual quantity as `MissedHedge`. Reaching the
  zero-fill IOC threshold persists the same global risk latch and cancels all
  active orders.
- Cancel request failures are not added to this submit-reject counter: one
  cancel failure immediately degrades the account, requests full REST
  reconciliation, and remains under the cancel-to-terminal convergence guard.
  Amend routing is unsupported.

## Strategy Safety Halt Propagation

- Every terminal chaos `halt_reason`, including delta, PnL, balance-sheet,
  margin, index, hedge-availability, anomalous-fill, and stuck-hedge stops, is
  exposed through the generic strategy safety contract. The engine checks that
  contract after every callback and before dispatching callback-generated
  intents.
- The first halt becomes `RiskBreach` plus `KillSwitchActivated`. New orders
  from the triggering callback are rejected, the coordinator persists a global
  risk latch, and every canonical active order is cancelled. Global kill scope
  takes precedence if a symbol halt occurs in the same event.
- The halt is terminal for that strategy instance. A reset event cannot reopen
  risk while the same instance still reports the halt, and live global reset is
  intentionally unavailable. Diagnose and correct the cause, independently
  verify orders and exposure, then use the stopped-process latch-clear procedure
  before returning through `observe`.

## Stablecoin Depeg Guard

- Configure `[[risk.stablecoin_guards]]` with the OKX index symbol and maximum
  downside deviation. The demo example checks `USDT-USD` and `USDC-USD` at 1%,
  matching the pinned Java defaults. A production config that uses either
  currency is invalid without its corresponding guard.
- Each guard is an `index-tickers` critical subscription with the configured
  replica count. Exact payloads deduplicate. Two different values with the same
  exchange timestamp are an integrity conflict and remain unhealthy until a
  newer timestamp arrives.
- Missing, invalid, conflicting, stale, or downside-depegged references remove
  startup/runtime readiness and reject new orders immediately. They never block
  cancels. A continuously unhealthy guard for
  `stablecoin_breach_debounce_ms` emits a durable global risk latch and cancels
  canonical live orders. Feed recovery does not clear a durable latch.
- OKX documents [`index-tickers`](https://www.okx.com/docs-v5/en/#public-data-websocket-index-tickers-channel)
  updates every 100 ms when changed and once per minute otherwise. The example
  therefore uses a 75-second
  `stablecoin_max_age_ms` websocket budget. Route connectivity is monitored
  separately; losing every replica emits public-feed stale, degrades readiness,
  and requests immediate fail-closed cancellation. Do not reduce the age below
  the unchanged-value interval unless another independently supervised refresh
  source is implemented.
- The live guard checks downside deviation after a fresh value; an upside move
  alone is not a depeg failure, matching the pinned Java final check. Backtests
  have no live stablecoin guards unless explicitly configured at a live risk
  boundary.

## Process Ownership And Host Guard

- The journal's sibling lock file contains PID and acquisition-time metadata
  and is mode `0600` on Unix. The kernel file lock, not the file's existence or
  metadata, is authoritative. A stale lock file after a crash is expected; do
  not delete it while any runtime may still hold the lock.
- Canonical parent resolution means relative paths and symlinked directory
  aliases contend for the same lease. The lease remains owned by the runtime
  even if the storage writer task fails and is released only during teardown or
  failed startup cleanup.
- Alert routing and host thresholds are deployment-only and are excluded from
  checkpoint identity, so enabling them does not invalidate an existing
  reconciled journal. Trading, account, runtime, storage, and operator changes
  remain fingerprint-bound.
- Set `[host_guard].enabled = true` for deployment. Choose disk and memory
  thresholds above the amount needed for a full fault/reconciliation window,
  not merely enough for the next flush. A failed preflight prevents credential
  reads and network startup; a failed periodic check is a fatal runtime event.
- The host clock check reads Linux kernel synchronization state. It complements,
  rather than replaces, the independent midpoint-adjusted OKX server-time
  checks. The deployment supervisor must still monitor the actual NTP/PTP
  service and clock offset.

## Process Supervision

Hardened baseline units and installation notes live in
[`deploy/systemd`](../deploy/systemd/README.md). They run as an unprivileged
`reap` user with a strict read-only filesystem view, one instance-specific
writable directory, bounded file descriptors/tasks, and no privilege or
namespace acquisition.

- `reap-observe@.service` may restart on failure because observe mode cannot
  submit or cancel. A start limit bounds repeated bootstrap failures.
- `reap-demo@.service` never restarts automatically. Any abnormal exit requires
  independent exchange reconciliation and operator approval first.
- `reap-capture@.service` never restarts automatically. Rotate to a new raw
  output path before every start so a file cannot contain multiple capture
  session IDs.
- Set `TimeoutStopSec` above the configured runtime shutdown plus alert-drain
  deadlines. A forced kill leaves the last exchange deadman in force but must be
  treated as an incident, followed by the emergency procedure below.
- Monitor activation failures, non-zero exits, start-limit exhaustion, forced
  kills, and host resource/time alarms outside this process. Validate the unit
  files and `systemd-analyze security` on the actual target OS.

Use absolute storage, operator-socket, and capture paths below the instance's
writable directory in deployed TOML. Config and environment files must be
readable only by root and the `reap` group; the environment file is populated by
the deployment secret provider.

## Independent Emergency Cancel

`reap emergency-cancel` is a separate composition root. It does not acquire or
trust the strategy journal, operator socket, live event loop, websocket state,
or strategy configuration. It parses only the OKX environment/REST settings,
request pacing/timeouts, account credential environment names, and configured
symbol keys. This keeps the exchange cancellation path available when the live
strategy config or process is unhealthy.

The command has a deliberately narrow scope: all **regular pending orders** on
each selected OKX account, including symbols not configured in the strategy.
It does not enumerate or cancel algo orders or spread orders. OKX Cancel All
After is account-wide for the regular order book but also excludes spread
orders. Accounts that use algo or spread orders need a separate, tested venue
procedure before this command can be considered a complete kill.

Incident procedure:

1. Stop the order-producing unit and every other process or operator that can
   use the selected account. Require `systemctl is-active` to show that the unit
   is not active and verify its main PID is zero. If the stop deadline forced a
   kill, record that fact and continue without restarting it.
2. Confirm the account is dedicated to this deployment. The CLI cannot prove
   that another host, strategy, manual trader, or API key has stopped; the
   `--confirm-order-producers-stopped` flag is an operator attestation.
3. From a restricted operator shell populated by the same secret provider, run
   the direct command. Prefer explicit accounts during an incident:

```bash
umask 077
/usr/local/bin/reap emergency-cancel \
  --config /etc/reap/live/btc-demo.toml \
  --account main \
  --confirm-account-wide-cancel \
  --confirm-order-producers-stopped \
  --confirm-production \
  --account-timeout-secs 30 \
  --deadman-timeout-secs 10 \
  --pretty > /var/lib/reap/live/btc-demo/emergency-cancel.json
```

`--confirm-production` is mandatory for a production venue config and harmless
for demo. `--all-configured-accounts` is available only when every configured
account's producers have been stopped and replaces all `--account` arguments.

4. For each account, the command samples exchange time, arms Cancel All After,
   enumerates pending regular orders account-wide, sends batches of at most 20,
   and re-enumerates until it observes zero after the deadman trigger horizon.
   Every REST call and pacing delay shares one absolute account timeout. A batch
   acknowledgement is only request acceptance; the final REST zero observation
   is authoritative.
5. Require process exit zero, top-level `all_clear = true`, and each account's
   `deadman_armed = true` and `verified_zero_after_deadman = true`. Review all
   incidents, partial/rejected/unacknowledged cancel counts, unmanaged symbols,
   and remaining-order details even when the final zero check succeeds.
6. Independently inspect OKX regular, algo, and spread orders plus balances and
   positions. Archive the report with the incident journal. A non-zero command
   exit or missing/failed account report means zero was not proven and requires
   venue UI/API escalation.
7. Leave demo trading stopped. Recover the immutable journal, reconcile exchange
   orders/fills/positions, and start `observe` first. Restore demo entry only
   after clean readiness and explicit operator approval; never auto-restart from
   this procedure.

This is an out-of-process safety layer, not a production certification or a
replacement for exchange-side account limits and operator access controls.

## External Alerts

- Set `[alerts].enabled = true` and provide the URL through
  `alerts.endpoint_env`. The endpoint must use HTTPS; loopback HTTP is accepted
  only for local testing. Set `bearer_token_env` only when the receiver requires
  a bearer token. Never put the URL credential or token directly in TOML.
- System transitions such as stale/gapped feeds, book recovery, reconciliation
  drift, risk breaches, operator kills, and account/symbol halts map to typed
  warning or critical JSON events. Runtime and periodic host-guard failures
  after alert startup produce a critical event. Routine graceful-stop kill
  records are not paged; bootstrap failures rely on the process supervisor's
  non-zero-exit alert.
- Alert HTTP runs in a separate bounded worker. The strategy loop only performs
  `try_send`; queue saturation is fail-stop. Requests use bounded connect and
  total timeouts with bounded retries; redirects are refused, event fields are
  size-capped, and transport errors omit the secret endpoint URL. With
  `delivery_failure_is_fatal = true`, terminal delivery failure enters the same
  fail-closed cancellation/reconciliation lifecycle as other runtime faults.
- Teardown drains queued events within `alerts.shutdown_timeout_ms`. Monitor the
  report fields `alerts_delivered`, `alert_delivery_failures`,
  `alert_failure_notifications_dropped`, and `max_alert_queue_depth`. A process
  supervisor must page on non-zero exit as the independent fallback when the
  alert destination itself is unavailable.

## Order Path

- The coordinator generates the client order ID and synchronously records a
  canonical `PendingNew` before dispatching to the account gateway task. The
  intent, pending state, and request are enqueued to critical storage before
  REST IO begins.
- Route explicit REST rejections back through the gateway state. Treat timeout
  and transport ambiguity as pending until REST/private reconciliation resolves
  it; do not blindly resubmit.
- Every place-order request carries an OKX `expTime` derived from
  `order_request_expiry_ms`; the exchange must discard a request that reaches
  its matching engine after that deadline.
- Feed every order acknowledgement, fill, account update, and position update
  into the single-writer event loop. Strategy state must not be mutated from a
  websocket task.
- Keep private deduplication and health account-scoped. One healthy account must
  never mask another stale account.
- Set `fill_state_convergence_timeout_ms` above `timer_interval_ms`, no higher
  than `risk.max_private_age_ms`, and calibrate it from demo
  fill-to-account/position latency. The demo baseline is 5 seconds; lowering it
  without latency evidence creates avoidable account-wide stops.

## Exchange Request Safety

- Bootstrap and a dedicated per-account safety task compare local time with the
  OKX public time endpoint. Excess skew or a failed periodic check is fatal.
- In demo mode the safety task refreshes Cancel All After independently of the
  order queue and strategy loop. `cancel_all_after_heartbeat_ms` must respect the
  endpoint rate limit and remain below `cancel_all_after_timeout_secs`.
- Cancel All After is account-wide. Use a dedicated OKX account/subaccount and
  do not run unrelated strategies or duplicate runtime credentials against it.
- A failed deadman heartbeat is fatal. The last armed exchange timer remains in
  force while the runtime enters fail-closed cancellation and reconciliation.
- Cancel All After is disabled only after a graceful demo stop has reached zero
  canonical active orders and every account has returned a clean zero-order
  REST reconciliation. Unsafe shutdown never disables it.

## Fail-Closed Matrix

| Condition | Automatic state | Required action |
| --- | --- | --- |
| Public sequence gap | Book recovering; new orders blocked | Obtain a fresh snapshot and replay contiguous buffered deltas |
| Public feed stale | Symbol blocked; live orders cancelled | Restore at least one healthy feed and verify sequence continuity |
| Private stream stale | Account blocked; live orders cancelled | Reconnect, REST reconcile orders/fills/balances/positions, then emit recovery |
| Submit/cancel order-state convergence timeout | Account blocked; active orders cancelled; expired cancel retried; full reconciliation requested | Suppress each orders-channel transition independently, then verify REST repair and no early readiness recovery |
| Fill/account-state convergence timeout | Account blocked; live orders cancelled; full reconciliation requested | Inspect the missing symbol/currency target and private-channel latency, then require authoritative repair and a clean pass |
| Position scope or margin-mode violation | Bootstrap/runtime abort; demo cleanup attempts cancel/reconcile and retains the deadman unless safe | Keep entry disabled; close/correct the unmodeled position or mode outside Reap, then require a clean bootstrap |
| Forced-repayment indicator at/above limit | Bootstrap/runtime abort; demo cleanup attempts cancel/reconcile and retains the deadman unless safe | Keep entry disabled; reduce borrowing/risk outside Reap and require all currencies below limit plus a clean bootstrap |
| Reconcile drift | Account blocked; live orders cancelled | Inspect the recorded full-state differences and require a later clean pass before recovery |
| Stablecoin reference missing/stale/conflicting/depegged | New entry blocked immediately; durable global kill and live-order cancellation after debounce | Verify both redundant index routes and an independent venue reference; use the stopped-process latch-clear procedure after a sustained breach |
| Risk breach | Durable global kill active; live orders cancelled | Reduce exposure externally if needed, diagnose, and follow the stopped-process latch-clear procedure; restart alone does not clear it |
| Manual account kill | Durable account route latch; its instruments are removed from pricing/hedging and its live orders are cancelled | Reconcile the account and dependent exposure; restart alone does not clear it |
| Exchange clock/deadman failure | Runtime fatal; new entry blocked; armed Cancel All After remains effective | Verify host time and OKX reachability, then reconcile before restart |
| Host disk/memory/clock failure | Startup refused or runtime fatal; new entry blocked | Restore capacity/time synchronization, inspect journal integrity, and reconcile before restart |
| Alert queue/delivery failure | Runtime fail-stop when fatal delivery is configured | Verify the external route and supervisor fallback before restart |
| Journal lease contention | Second process refuses startup before credentials/network | Identify the owning PID/process; never bypass the lock or share the journal |
| Critical storage loss/backpressure | Runtime fail-stop; checkpoint reconciliation required on restart | Investigate disk/queue capacity; critical records are never silently dropped |

## Operator Controls

- The enabled operator service binds only a Unix-domain socket, refuses to
  replace a non-socket path, and changes the socket mode to `0600`.
- Requests are bounded JSON lines signed with HMAC-SHA256 using the secret named
  by `operator.token_env`. The signed payload includes protocol version,
  request ID, timestamp, nonce, and command. Stale timestamps, reused nonces,
  invalid signatures, oversized requests, and control-channel backpressure are
  rejected. Responses are signed and verified by the CLI before acceptance is
  displayed.
- Socket parsing and authentication run outside the strategy loop. Accepted
  commands enter a bounded channel and are reduced by the same single writer as
  exchange events. A typed safety-latch record is flushed and `sync_data` has
  completed before control-generated cancellation actions are dispatched;
  normalized/system audit records retain the same request ID. The wait is
  bounded by `safety_latch_sync_timeout_ms`; timeout is a fail-stop condition
  and prevents graceful shutdown from disarming Cancel All After.
- Protocol version 2 commands are read-only `status`, global `kill`, account
  `kill-account`, symbol `halt`, symbol `resume`, and reconciled `shutdown`.
  Status includes the global kill state and account-halt reasons. Older protocol
  versions are rejected.
- Global, account, and symbol stops are reduced from explicit `SafetyLatch`
  journal records during restart. Account kills block new requests to that
  account, halt every routed instrument inside the strategy, and cancel all
  canonical `PendingNew`, `Live`, and `PartiallyFilled` orders for it. A
  dependent quote on another account may also be withdrawn when the strategy
  loses a valid hedge. Post-trade risk breaches persist the global latch once.
- A symbol belonging to a killed account cannot be resumed. Live global and
  account reset commands are intentionally unavailable. Restarting with the
  same journal restores the latch before readiness and dispatches canonical
  cancels again. A normal graceful-shutdown kill is not a persistent latch.
- To clear a global or account latch, stop the process, independently verify
  zero exchange orders and acceptable exposure, archive the immutable journal,
  configure a fresh storage path, and restart in `observe`. Re-enable demo entry
  only after clean REST reconciliation and operator approval. Never edit or
  truncate the existing journal to clear a latch.

Supply the same secret to the runtime and operator shell through the deployment
secret provider. It must contain at least 32 bytes:

```bash
export REAP_OPERATOR_TOKEN=...
cargo run -p reap-cli -- operator \
  --config examples/live-okx-demo.toml \
  status --pretty
cargo run -p reap-cli -- operator \
  --config examples/live-okx-demo.toml \
  kill-account --account main --reason "unexpected account exposure"
cargo run -p reap-cli -- operator \
  --config examples/live-okx-demo.toml \
  halt --symbol BTC-USDT --reason "manual market pause"
cargo run -p reap-cli -- operator \
  --config examples/live-okx-demo.toml \
  resume --symbol BTC-USDT --reason "market reviewed"
cargo run -p reap-cli -- operator \
  --config examples/live-okx-demo.toml \
  kill --reason "unexpected exposure"
cargo run -p reap-cli -- operator \
  --config examples/live-okx-demo.toml \
  shutdown --reason "planned deployment stop"
```

Synchronize the host clock. Operator requests outside `max_clock_skew_ms` are
rejected, and exchange skew outside `max_exchange_clock_skew_ms` is fatal.
Observe mode remains exchange-read-only, so kill/halt events update local state
but cannot cancel exchange orders.

Control and health events must be captured as normalized records so the exact
live decision path can be replayed.

For multi-account strategies, a private stale or reconciliation drift event
must carry `account_id`. A venue-wide event without an account scope is treated
as affecting every tracked account on that venue.

## Credentials

- Load API key, secret, and passphrase from the deployment secret provider, not
  TOML or source control.
- Credential debug output is redacted. Do not add raw request-header logging.
- Synchronize UTC time before signing. Treat authentication timestamp failures
  as unhealthy private connectivity.
- Use OKX demo trading first. Production enablement requires exchange/account
  mode verification, instrument trade-mode mapping, and an explicit operator
  change.

## Shutdown

Every demo event-loop exit, including `SIGINT`, `SIGTERM`, bounded completion,
adapter failure, gateway-task failure, channel failure, and storage failure,
uses one fail-closed path. It disables new submits without disabling cancels,
activates the kill switch, dispatches every canonical cancel, and requires a
post-cancel REST reconciliation result for every account. Only zero active
canonical orders plus clean order/fill/balance/position reconciliation permits
task teardown.
The runtime then explicitly disables Cancel All After unless safety-latch
durability failed. If any part of shutdown is unresolved or latch durability is
uncertain, it leaves the exchange timer armed and terminates the safety task so
the last timeout can expire.

The `shutdown_timeout_ms` deadline covers cancel queueing, reconciliation, and
event processing. A persistence failure is retained as an error but does not
suppress cancel or reconciliation commands. Any unresolved order/account or
secondary teardown failure is included in the returned lifecycle error and
must be treated as an incident. Observe mode performs no exchange mutation and
shuts down directly.

Host-guard teardown runs before feed/order task teardown. Alert teardown runs
last so runtime teardown failures can still be queued, and is independently
bounded by `alerts.shutdown_timeout_ms`.

## Bounded Soak Acceptance

Use a bounded run for evidence that can be evaluated without an operator-timed
signal. An observe soak never permits submit or cancel requests:

```bash
cargo run -p reap-cli -- live \
  --config examples/live-okx-demo.toml \
  --mode observe \
  --duration-secs 3600 \
  --require-clean-soak \
  --pretty
```

After observe acceptance, run a deliberately short minimal-size demo window
before increasing its duration:

```bash
cargo run -p reap-cli -- live \
  --config examples/live-okx-demo.toml \
  --mode demo \
  --confirm-demo \
  --duration-secs 900 \
  --require-clean-soak \
  --pretty
```

`--require-clean-soak` requires `--duration-secs` and exits non-zero unless all
of these invariants hold:

- the bounded duration, rather than validation, readiness timeout, or an
  operator signal, ended the run;
- the runtime reached `ready` and `readiness_at_stop` is still `ready`;
- no `ReconcileDrift` event or best-effort storage drop occurred; and
- no authenticated operator mutation occurred; and
- demo shutdown resolved every active canonical order; and
- no external alert delivery failed.

The report also records time-to-ready, recovered readiness losses and maximum
outage, disconnects, stale-stream events, book recoveries, and the storage queue
high-water mark. It also reports authenticated operator commands and mutations.
When enabled, it includes host preflight/last snapshots and check count plus
alert delivery and queue evidence.
Recovered disconnects do not by themselves fail acceptance,
but their counts must match the injected fault plan. In demo mode the final
`readiness` may be degraded by the deliberate shutdown kill switch; acceptance
uses the pre-shutdown `readiness_at_stop` snapshot.

A `clean_soak` result is evidence for this bounded runtime window only. Review
the JSONL log, account balances, positions, fills, and checkpoint restart before
checking off the sustained demo gate.
