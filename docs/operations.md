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
REPORT_PATH="var/reap/capture/okx-btc-${RUN_ID}.report.json"
cargo run -p reap-cli -- capture \
  --config examples/capture-okx-public.toml \
  --output "$REPORT_PATH" \
  --raw-path "$RAW_PATH" \
  --duration-secs 3600 \
  --require-clean-capture \
  --pretty
```

The example captures spot and swap books/trades plus index, swap funding, swap
mark, and separate spot/swap price-limit inputs used by iarb2. Every
subscription has two independent connections. Raw frames are canonical and
written sequentially within one run.
The capture config loader accepts at most 16 MiB from one regular non-symlink
file, canonicalizes its source path, and records that path with the exact bytes
and SHA-256 in run evidence.
The capture event loop assigns each frame a process-global `capture_record_seq`
before the bounded writer, independent of OKX's channel-specific book sequence.
The optional
`output.normalized_path` is intended for short diagnostics because full
400-level snapshots are much larger than raw deltas.

`examples/capture-okx-public.toml` is runnable from the repository root.
`deploy/capture/okx-btc-public.toml` is the production-shaped equivalent: it
uses the absolute shared connection pacer and an absolute placeholder raw path.
The systemd unit always overrides that placeholder with a unique instance path.
For a direct deployment-host run, use the same reviewed file and explicit
create-new outputs:

```bash
RUN_ID="btc-public-$(date -u +%Y%m%dT%H%M%SZ)"
sudo install -d -o reap -g reap -m 0750 \
  /var/lib/reap/connectivity "/var/lib/reap/capture/${RUN_ID}"
sudo -u reap /usr/local/bin/reap capture \
  --config /etc/reap/capture/okx-btc-public.toml \
  --output "/var/lib/reap/capture/${RUN_ID}/run-report.json" \
  --raw-path "/var/lib/reap/capture/${RUN_ID}/raw.jsonl" \
  --duration-secs 86400 \
  --require-clean-capture
```

This command remains credential-free. A new `RUN_ID` is mandatory for every
attempt, including attempts that fail before websocket startup.

For a production-candidate dataset, collect the opening boundary first while
the account is quiescent, then start the public capture immediately:

```bash
ACCOUNT_PATH="var/reap/capture/okx-btc-${RUN_ID}.opening-account.json"
cargo run -p reap-cli -- certify-account \
  --config /secure/config/reap-production-candidate.toml \
  --account main \
  --output "$ACCOUNT_PATH" \
  --pretty
# Run the capture command above without allowing account activity in between.
```

The certification command is authenticated and read-only; `capture` remains
credential-free. Use a unique certification for each process session. Schema-7
research requires the certification to finish before capture and within the
manifest's `maximum_opening_account_gap_ms` budget.

The checked-in config enables the Linux host guard. Capture creates the raw
parent directory, then checks that filesystem capacity, `MemAvailable`, and
kernel clock synchronization before opening an output file or websocket. It
repeats the check outside the feed loop and exits non-zero on any breach. The
configured thresholds must leave room for the full bounded run and shutdown,
not only the next flush; enabled check intervals are capped at 60 seconds. A
copied local diagnostic config may disable the guard; production-candidate
research rejects that evidence.

At pinned `imm-strategy` revision
`b6b120c7b7c466d8431bf082f3229328c5d7b2ae`,
`AbstractOkxNitroL2Subscriber` owns the Java order-book websocket lifecycle and
its `onSocketDisconnected` path participates in reconnect handling. The Java
collector does not provide this Rust run-report, executable/host provenance, or
host guard, and the pinned collector has no versioned persisted-frame ordinal.
Those controls harden the Java-referenced connectivity boundary; they are not a
parity claim.

`runtime.connection_attempt_interval_ms` defaults to 400 ms. Official endpoint
configs also require `runtime.connection_attempt_pacer_path`. Every public,
private, authenticated order-command, capture, and fault-proxy-upstream initial
or reconnect attempt reserves through the owner-only advisory-locked file, so
separate Reap processes on one host share a single schedule. State is a bounded,
fixed-format Linux boot ID plus `CLOCK_BOOTTIME` next-slot timestamp, so a
wall-clock step cannot compress attempts and stale state resets after reboot. A
symlink, non-regular or multiply linked file, foreign owner, group/other access,
malformed content, I/O failure, lock contention lasting one second, or a
reservation more than 15 minutes ahead fails closed. The parent directory must
already exist. Official endpoint runtimes fail preflight outside Linux rather
than substitute a wall clock. Official OKX endpoints require at least 334 ms
because OKX documents a maximum of three WebSocket connection requests per
second per IP in the [API guide](https://www.okx.com/docs-v5/en/). Zero without a
file is accepted only for loopback tests. The generated fault-routed live config
uses zero for its local proxy handshakes because the proxy reserves each
official upstream connection through the source config's exact interval and
path. A pacer failure discovered during reconnect is terminal for capture and
live runtimes; it is not downgraded to a redundancy-tolerated disconnect.

After a paced slot is reserved, each feed handshake must complete within 10
seconds and remains interruptible by shutdown or source recovery. Private
bootstrap, subscription, ping/pong, and close writes each have a 5-second
deadline. Every socket plan retains its recovery-channel owner for the complete
supervisor lifetime; unexpected owner loss is a fatal invariant breach, not a
reconnect loop. Once ready, the feed sends the OKX application ping every 15
seconds and reconnects after observing that no frame has arrived beyond the
30-second threshold. This is transport liveness. Required book, index, funding,
mark, price-limit, account, and position freshness remains independently aged,
so heartbeat traffic cannot make stale strategy state tradable. These controls
were cross-checked against pinned Java `OkxNitroSubscriberBase`,
`SharedSessionSubscriberBase`, `WsConnectOption`, and the Netty/Vtx websocket
clients' stalled-INIT and ping/pong/data checks.

Failures before exact subscription readiness use exponential reconnect delays
from 250 ms through 30 seconds. Once a socket has delivered the complete
acknowledgement set, that historical delay resets, so a later healthy-session
disconnect starts again at 250 ms subject to the shared connection-attempt
pacer. Public payload saturation disconnects immediately. Private account/order
payload saturation is never silent: delivery may wait up to one second for
bounded downstream capacity, after which the socket emits a typed disconnect,
private readiness is revoked, active orders are cancelled, and recovery requires
a clean REST reconciliation. Treat this as event-loop overload evidence;
increasing queue capacity without profiling only postpones the same failure.

The checked-in capture/live/proxy examples share
`var/reap/okx-connection-attempt.pacer` when run from the repository root.
Production capture research requires an absolute path, and production evidence
requires the exact demo and production configs to use the same absolute path.
Set all deployed modes to
`/var/lib/reap/connectivity/okx-global.pacer`; the systemd templates create and
expose only that shared directory. This file coordinates processes on one host,
not multiple hosts behind one egress IP. Use isolated egress or an external
IP-wide coordinator for that topology. The interval and path are part of exact
config provenance, and the total paced startup must fit inside live readiness.

Every frame and run report carries a generated `capture_session_id`; every raw
frame also carries a process-global `capture_record_seq` assigned from `1` at
the single capture event-loop/writer boundary. The report, raw output, and
normalized output use create-new mode-`0600` semantics on Unix:
startup refuses an existing path and never appends a second process session.
Capture config parsing, CLI path overrides, path-collision checks, and effective
config validation complete before the report path is reserved. An invalid
configuration therefore creates no report. After reservation, a handled setup,
event-loop, drain, host-guard, or writer failure emits a schema-5 report with
`stop_reason = "runtime_failure"`, a bounded stable `failure.code` and diagnostic
`failure.message`, and `clean_capture = false` before the command exits nonzero.
Startup failure reports use zero-record empty-hash output evidence and never hash
an older raw or normalized file that this process did not create. If report
serialization or its filesystem durability fails, or the process is killed,
the reserved report can still be empty or incomplete; page on that condition and
never reuse the instance path. Use unique paths for each process. Strict replay
and raw backtest also reject files containing more than one session ID, because
process downtime is not a continuous HFT market stream.

The schema-current verifier requires exactly one ordinal for every persisted
raw record, with first `1`, last equal to `raw_records`, and no missing,
duplicate, skipped, regressed, or reordered value. This detects writer-boundary
loss across all channels. It complements rather than replaces OKX book
`prevSeqId` continuity, because non-book channels do not share one exchange
sequence and frames can arrive concurrently from redundant sockets. Legacy raw
files without ordinals remain readable for diagnostics but cannot pass current
capture or production-research evidence gates.

Book deduplication keys exact redundant images by action, `prevSeqId`, `seqId`,
exchange timestamp, and raw-payload hash. Global duplicates remain visible to
each socket's independent sequence tracker and full-book reducer. Continuity on
each `conn_id` requires `prevSeqId` to equal that source's last `seqId`; the next
`seqId` may be equal for a no-change update or lower after exchange maintenance.
Those per-source observations increment `same_sequence_updates` or
`sequence_resets`. A predecessor mismatch requests a fresh snapshot from only
that source while another ready replica preserves the canonical book. Different
reconstructed books at the same exchange timestamp and sequence fail closed and
restart every conflicting source.

OKX has deprecated the order-book checksum and documents that the field remains
zero. Capture integrity therefore relies on WSS transport, sequence links,
fresh snapshots, crossed-book rejection, age checks, and strict replay rather
than the old CRC algorithm. See the current [OKX API guide](https://www.okx.com/docs-v5/en/)
and [checksum deprecation announcement](https://www.okx.com/en-us/help/okx-order-book-channels-checksum-field-deprecation).

`clean_capture` requires a bounded duration, all socket plans ready at least
once and at stop, a ready contiguous snapshot for every configured book, and
data from every exact deterministic replica/chunk socket-plan ID plus at least
one accepted event for every configured book, trade, index, funding, mark, and
price-limit stream. A count-equivalent frame from the wrong plan, unclassified
data, or an unexpected stream fails the contract. It also requires
non-empty raw output; zero parse, gap, stale-book, recovery-request,
recovery-route, or recovery-failure counts; and host evidence matching the
configured guard, including a healthy preflight and every completed periodic
check. `--require-clean-capture` applies this full contract at runtime, and
`verify-capture` independently reconstructs it from raw frames. A
redundant-socket disconnect can
remain clean only when the other replica preserves sequence continuity;
inspect disconnect, duplicate, and writer queue counts on every run.

Raw and optional normalized writers use bounded channels. Event-loop enqueue
waits at most one second; saturation stops capture instead of dropping a frame or
waiting forever. Teardown allows five seconds for supervised-feed shutdown plus
channel drain. It then closes the writer queues and, concurrently with a
five-second host-guard shutdown bound, allows 30 seconds for writer flush and
`sync_data`, one additional second for task cancellation after a timeout, and
five seconds for a best-effort partial-file count/hash scan. Any such failure is
retained in the non-clean report. The collector unit's
`TimeoutStopSec=45s` remains the external hard stop for a kernel or filesystem
operation that cannot be cancelled in process. A `failure` report is diagnostic:
`verify-capture --require-pass` and production research reject it even if someone
forges `clean_capture = true`.

Production-candidate research applies a code-level host-guard policy in
addition to checking the report: the guard must be enabled, run at least every
10 seconds, require at least 5 GiB available disk and 1 GiB available memory,
and require synchronized kernel-clock status. Lower thresholds may be useful
for an explicitly diagnostic run on a constrained workstation, but that run is
not admissible as production research evidence even if `clean_capture` is true.

The schema-5 run report includes the Reap version, pinned Java revision, exact
capture-executable SHA-256, pseudonymous host identity when guarded, host
preflight/periodic evidence within explicit session wall-clock bounds, exact
source-config byte count/SHA-256, effective
config fingerprint after CLI output overrides, and byte count/SHA-256 for every
writer. Successful capture files are data-synced and their directory entries are
synced before the report is emitted. Handled failures add the optional typed
`failure` object described above; absence of that object does not by itself make
a run clean. Archive the report with the config and capture manifest.

`verify-capture` reconstructs the report's effective path overrides while
allowing artifacts to be relocated. It requires the supplied config's exact
bytes and effective fingerprint, raw session/counters/bytes/hash, replay-derived
book and integrity state, build/Java/host evidence shape, and the report's clean
flag to agree. Production research additionally binds the reported executable
to its own binary and the reported host to latency calibration. When normalized
output was enabled, pass `--normalized-events`; verification replays the raw
frames and requires the normalized record count, bytes, and SHA-256 to match the
independently reconstructed JSONL exactly. `analyze-capture` remains a standalone
quality report; its config fingerprint intentionally reflects its own input-path
override and is not the run-provenance gate.

Validate and consume the output directly:

```bash
cargo run -p reap-cli -- verify-capture \
  --config examples/capture-okx-public.toml \
  --report "$REPORT_PATH" \
  --events "$RAW_PATH" --require-pass --pretty
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
For raw captures, risk integrals and `observed_duration_ns` both close at the
selected range's maximum persisted receive timestamp, even when the final raw
record is a duplicate or control frame that emits no normalized event.
Always inspect `input_clock_regressions`, `live_orders`, `max_active_orders`,
maximum/final active-order notional, maximum/final position and pending delta,
`pending_scheduled_actions`, and the five pending-action category counts.
Concurrent socket tasks can reach the single raw writer in a different order
from their receive stamps even when each source is monotonic. Replay clamps
those global regressions, preserves file order as the tie-breaker, and reports
their exact nanosecond count and maximum; investigate large values separately
from the analyzer's per-source monotonicity gate.

Normalized capture is diagnostic, not the authoritative HFT replay format.
Exchange timestamps from independent channels are not one global arrival clock,
so replaying the normalized file in writer order can report large cross-stream
clock regressions and can change strategy scheduling. Production research must
use raw capture receive time through the adapter/deduplication/reducer path.

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
`turnover_usd`, `cash_by_currency`, `currency_rates`,
`currency_conversion_failures`, `funding_rate_events`,
`funding_settlement_observations`, `funding_settlements`, and
`accounting_complete`. OKX `fundingRate` remains the forecast delivered to the
strategy, matching the pinned Java decision path. It is never booked as cash.
Raw replay first extracts `settFundingRate` with its observed
`prevFundingTime`, then replays chronologically and books that realized rate at
the original exchange settlement time. The realized map is private to the
accounting scheduler and is not exposed to strategy decisions. Conflicting
realized rates are rejected. A due forecast without a realized rate, invalid
data, or a missing mark for a nonzero swap position makes accounting
incomplete. A first forecast up to 60 seconds late is applied on the current
clock but marks accounting incomplete; older first forecasts are counted as
missed. A future funding settlement beyond the capture horizon is reported as
`pending_funding_actions` and is not itself a defect for the observed interval.

By default the portfolio starts at zero. For a realistic run, add one complete,
single-account opening snapshot. Every configured spot base/quote and derivative
settlement currency must have a balance row. Positive spot base inventory names
one spot valuation symbol; derivative rows are optional when flat and otherwise
require exchange average cost and margin mode:

```toml
[initial_portfolio]
account_id = "strategy-subaccount" # omit when risk groups omit account_id

[[initial_portfolio.balances]]
currency = "BTC"
total = 0.002
valuation_symbol = "BTC-USDT"

[[initial_portfolio.balances]]
currency = "USDT"
total = 10000.0

[[initial_portfolio.positions]]
symbol = "BTC-USDT-SWAP"
qty = -1.0
avg_price = 65000.0
margin_mode = "cross"
```

Negative balances, duplicate currencies/symbols, spot rows in `positions`,
multi-account strategy mappings, missing balance rows, unknown valuation
symbols, and opening quantities outside configured position limits fail before
replay. This scope intentionally excludes borrowing, account holds, and a
dynamic margin engine. Manual snapshots default omitted available/equity fields
to total. Certified research snapshots preserve exchange `availBal`, `eq`,
`maxLoan`, forced-repayment indicator, `mgnRatio`, `adjEq`, and `notionalUsd`;
liability must remain zero. Synthetic post-fill account updates preserve the
opening available/equity offsets as cash changes.
The snapshot is delivered to strategy state at the first replay timestamp. The
runner then blocks order entry until every book and currency conversion needed
for an exact opening valuation is ready. A pre-baseline fill aborts. Reports
retain the opening snapshot, opening valuation timestamp/equity, final account
balances and position average prices, final equity, and opening-adjusted
`net_pnl_usd`; a terminal strategy safety halt also fails research evidence.

Configure one direct USD-per-unit index for every named non-USD
quote/settlement or unvalued cash currency:

```toml
[[backtest.currency_rates]]
currency = "USDT"
index_symbol = "USDT-USD"
max_age_ms = 75000
```

The raw capture configuration must contain the matching redundant
`index-tickers` stream. The rate becomes effective after simulated reference
data latency, while age remains measured from the retained source timestamp.
Missing, stale, invalid, or post-fill-only observations make the run
non-passing; the report exposes the raw currency cash, source/effective times,
and rate age used.

As in the live coordinator, the backtest processes warmup events but rejects
new order intents until every strategy matcher has a book and every configured
accounting route has a fresh positive observation. Cancels are never blocked by
this startup gate. Inspect `order_entry_ready_at_ns`,
`order_entry_ready_at_end`, and `new_orders_blocked_not_ready`; this prevents
startup ordering between independent sockets from creating positions or active
notional before valuation is available.

Walk-forward scenarios inherit routes from each candidate; leave scenario
`currency_rates` empty or repeat the exact candidate set. A scenario cannot
substitute a different valuation source or freshness budget.

This depeg-sensitive research extension does not change the Java-parity strategy
decision model. Private order/fill and REST records now retain exact signed fee
amounts and fee currency; public-data simulated fills still use the configured
maker/taker fee tier and reports distinguish exact from estimated fee fills. The
offline `reconcile-fills` command now compares canonical fills and exact fees
with raw OKX responses, but it does not reconcile balances, funding, equity,
liabilities, borrowing interest, liquidation, margin discounts, or taxes. Live
spot is cash-only; certify zero target-account liabilities, and do not enable
margin spot without adding and validating a borrow-interest model. A research
acceptance run must span held funding boundaries, use the target fee tier, and
reconcile fill, fee, funding, cash, equity, currency conversion, and active-order
notional attribution to demo account statements before profitability metrics
are trusted.
Use `--require-complete-accounting` in automated research runs so any reported
accounting defect also makes the command fail.

### Fill And Fee Statement Reconciliation

Stop the live process first. For a window inside the recent-fill retention
boundary, wait at least 60 seconds after its inclusive end and use the
authenticated read-only collector. The endpoint contract is documented in the
[OKX API guide](https://www.okx.com/docs-v5/en/).

```bash
BEGIN_MS=1783987200000
END_MS=1783990800000
EVIDENCE="var/reap/evidence/okx-fills-$(date -u +%Y%m%dT%H%M%SZ)"
REPORT="var/reap/evidence/fills-$(date -u +%Y%m%dT%H%M%SZ).json"
cargo run -p reap-cli -- collect-fills \
  --config examples/live-okx-demo.toml \
  --account main \
  --begin-ms "$BEGIN_MS" \
  --end-ms "$END_MS" \
  --output "$EVIDENCE" \
  --pretty

cargo run -p reap-cli -- reconcile-fills \
  --journal var/reap/live-events.jsonl \
  --collection-manifest "$EVIDENCE/manifest.json" \
  --account main \
  --begin-ms "$BEGIN_MS" \
  --end-ms "$END_MS" \
  --minimum-fills 10 \
  --output "$REPORT" \
  --require-pass \
  --pretty
```

`collect-fills` uses a public exchange-time GET plus the selected account
credentials for signed account-config and recent-fill GETs; it has no order
entry path. It reserves a mode-`0700` directory before credentials or network
access, writes exact response pages as create-new mode-`0600` files, paces
100-row requests at least 200 ms apart, samples pseudonymous account identity
before and after, and requires a short terminal page. Windows are conservatively
limited to 70 hours even though the endpoint retention is 72 hours. A failed
collection deliberately leaves no complete manifest.

`reconcile-fills` is offline and reads no credentials. It verifies schema,
pinned Java revision, file hashes and bounds, exact request/cursor chain,
terminal-page completeness, exchange clock bounds, account mode, and config
fingerprint before taking the canonical journal lease. The schema-2 report binds
the collection manifest and pseudonymous account identity to the journal
bootstrap config, then compares order id, side, price, quantity, liquidity when
journaled, signed fee amount, and normalized fee currency by strict
`(symbol, tradeId)`. It fails on malformed or duplicate records, either missing
side, absent exact fees, config mismatch, a truncated journal tail, or fewer than
`--minimum-fills` comparisons. Tolerances default to zero.

For older windows, preserve every unmodified `/api/v5/trade/fills-history`
response and use repeated `--statement` arguments plus
`--confirm-statement-account-and-window-complete`. Because those bodies do not
echo account or request parameters, this manual path remains an operator
attestation and is weaker than the authenticated collector. Reconciliation
output is create-new, mode `0600` on Unix, and fsynced before `--require-pass` is
enforced. A failure before report serialization can leave an empty reserved
output, which is not evidence.

This artifact closes only the fill/fee comparison. Archive it with the raw
pages, live report, journal hash, and account export. Use the account-wide bill
workflow below for trade/funding economics, independently derived close PnL,
and bounded cash/equity endpoints. Positions, taxes, unsupported account flows,
and total-equity attribution remain separate review boundaries.

### Trade And Funding Economic Reconciliation

Run this only after stopping every process that can append to the canonical
journal. Use a controlled demo-account window with no deposits, withdrawals,
transfers, conversions, liquidation, borrowing, interest, or unrelated trading.
The account-bills endpoint is deliberately queried without a type filter, and
the verifier rejects every bill other than normal trades and swap funding. That
fail-closed scope is what makes an unexplained balance change visible.

The fill collection must begin at least `maximum_trade_bill_delay_ms` before the
bill window. Both collections must use the same exact live config and resolve to
the same environment, account settings, and pseudonymous account identity.
Quiesce the account and collect a passing opening account certification no more
than 60 seconds before `BEGIN_MS`. Stop every producer at `END_MS` and collect a
passing closing certification no more than 60 seconds later. Do not resume
activity until the closing artifact exists.

```bash
BEGIN_MS=1783987200000
END_MS=1784016000000
MAX_TRADE_BILL_DELAY_MS=60000
FILL_BEGIN_MS=$((BEGIN_MS - MAX_TRADE_BILL_DELAY_MS))
FILL_EVIDENCE="var/reap/evidence/okx-fills-economic-$(date -u +%Y%m%dT%H%M%SZ)"
BILL_EVIDENCE="var/reap/evidence/okx-bills-$(date -u +%Y%m%dT%H%M%SZ)"
BILL_VERIFICATION="var/reap/evidence/okx-bills-verification-$(date -u +%Y%m%dT%H%M%SZ).json"
ECONOMIC_REPORT="var/reap/evidence/okx-economics-$(date -u +%Y%m%dT%H%M%SZ).json"
OPENING_ACCOUNT="var/reap/evidence/okx-opening-account-$(date -u +%Y%m%dT%H%M%SZ).json"
CLOSING_ACCOUNT="var/reap/evidence/okx-closing-account-$(date -u +%Y%m%dT%H%M%SZ).json"

# While quiesced immediately before BEGIN_MS; start the bounded demo afterward.
cargo run -p reap-cli -- certify-account \
  --config examples/live-okx-demo.toml --account main \
  --output "$OPENING_ACCOUNT" --pretty

# Stop the demo at END_MS and run this before collecting pages.
cargo run -p reap-cli -- certify-account \
  --config examples/live-okx-demo.toml --account main \
  --output "$CLOSING_ACCOUNT" --pretty

cargo run -p reap-cli -- collect-fills \
  --config examples/live-okx-demo.toml \
  --account main \
  --begin-ms "$FILL_BEGIN_MS" \
  --end-ms "$END_MS" \
  --output "$FILL_EVIDENCE" \
  --pretty

cargo run -p reap-cli -- collect-bills \
  --config examples/live-okx-demo.toml \
  --account main \
  --begin-ms "$BEGIN_MS" \
  --end-ms "$END_MS" \
  --output "$BILL_EVIDENCE" \
  --pretty

cargo run -p reap-cli -- verify-bill-collection \
  --manifest "$BILL_EVIDENCE/manifest.json" \
  --output "$BILL_VERIFICATION" \
  --pretty

cargo run -p reap-cli -- reconcile-economics \
  --journal var/reap/live-events.jsonl \
  --fill-collection-manifest "$FILL_EVIDENCE/manifest.json" \
  --bill-collection-manifest "$BILL_EVIDENCE/manifest.json" \
  --opening-account-certification "$OPENING_ACCOUNT" \
  --closing-account-certification "$CLOSING_ACCOUNT" \
  --account main \
  --begin-ms "$BEGIN_MS" \
  --end-ms "$END_MS" \
  --minimum-trade-bills 10 \
  --minimum-derivative-close-bills 5 \
  --minimum-funding-bills 1 \
  --maximum-trade-bill-delay-ms "$MAX_TRADE_BILL_DELAY_MS" \
  --maximum-funding-bill-delay-ms 60000 \
  --maximum-funding-mark-bracket-distance-ms 1000 \
  --maximum-account-boundary-gap-ms 60000 \
  --trade-pnl-absolute-tolerance 0.0000000001 \
  --trade-pnl-relative-tolerance 0.00000001 \
  --funding-mark-absolute-tolerance 0.00000001 \
  --funding-mark-relative-tolerance 0.00001 \
  --output "$ECONOMIC_REPORT" \
  --require-pass \
  --pretty
```

`collect-bills` uses public exchange time plus signed read-only account-config
and `/api/v5/account/bills` GETs. It reserves a mode-`0700` directory before
credentials or network access, writes exact create-new mode-`0600` responses,
paces 100-row pages at least 500 ms apart, samples account identity before and
after, and requires a short terminal page. The endpoint retains seven days; the
collector uses a conservative 166-hour maximum age so verification and close
delay do not race retention. A failed collection leaves no complete manifest.

`verify-bill-collection` reads no credentials. It reopens and bounds every
source, re-hashes exact config/page/manifest bytes, reconstructs every request
path and `after=billId` cursor, rejects duplicate IDs/cursors and out-of-window
rows, and refuses a full final page or a changed source. The optional summary
and the economic report use create-new mode-`0600` files with file and parent
directory durability.

`reconcile-economics` independently reruns both collection verifiers, leases and
streams the stopped journal, and checks exact config/account identity. Normal
trade bills bind to fills by `(symbol, tradeId)` and validate side, order IDs,
instrument/margin/execution mode, price, size, liquidity, signed fee/currency,
causal delay, and the bill balance equation. They must also bind to one exact
account-scoped critical journal fill. Every authoritative REST account
replacement writes a critical `account_snapshot` with exchange `avgPx`;
derivative fills replay from the latest same-session snapshot using the pinned
Java arithmetic average for linear contracts and harmonic average for inverse
contracts. The snapshot exchange timestamp must strictly precede every replayed
fill, preventing a reconciliation-race fill already reflected in `avgPx` from
being applied twice. The verifier independently checks open/close subtype and realized
close PnL before accepting the bill. Funding type `8` subtypes `173` and
`174` follow pinned Java `OkexV5BillTypes`; the verifier binds each bill to a
unique session-local journaled realized rate and the latest signed position at
the bill's `fillTime`. It also requires valid same-session `mark-price`
observations immediately before and after that assessment timestamp. The bill
mark must lie inside the bracket, and linear `position * ctVal * rate * mark` or
inverse `position * ctVal * rate / mark` PnL must lie inside the independently
derived funding range with the configured sign. The session is not inferred
from the one-time bootstrap record: every runtime start emits a schema-7
`session_start` for each account, binding the session ID, strategy/config, and
hashed OKX account identity. Reconciliation uses journal line boundaries and
refuses to combine positions, settlements, or marks across the next start.

Schema-5 reconciliation also reopens and independently verifies both schema-3
account certifications. It requires exact config/environment/account identity,
an opening finish before `BEGIN_MS`, a closing start after `END_MS`, and each gap
within `maximum_account_boundary_gap_ms`. For every currency it orders bills by
exchange time and numeric `billId`, requires finite `balChg` and post-bill `bal`,
checks the opening-to-first, every adjacent post/pre, and last-to-closing links,
and separately proves `opening cashBal + sum(balChg) = closing cashBal`. A
currency absent from an unfiltered boundary response is authoritative zero. The
report retains native and USD equity at both endpoints and the total-equity
delta.

The pinned Java mapping is explicit: `OkexV5PositionConverter` maps OKX
`avgPx` to `ExchPosn.avgCost`; `RiskCalculator.getAvgPrice` keeps basis on a
reduction, resets it to fill price on a flip, uses arithmetic weighting for
linear products, and harmonic weighting for inverse products;
`PortfolioExchPosnCalculator` applies the corresponding signed close-PnL
formulas. Current OKX documentation describes `avgPx` as current average entry
price and publishes the linear/inverse fill-PnL formulas in the
[API guide](https://www.okx.com/docs-v5/en/) and signed long/short formulas in
[Futures PnL calculation rules](https://www.okx.com/en-us/help/futures-pnl-calculation-rules).
Cash continuity follows pinned Java `OkexV5ExchBillConverter` (`balChg`, `bal`,
and `posBalChg`) and `OkexV5TradePosKeeper` bill-driven cash snapshots rather
than inferring cash from fills alone.

Require nonzero thresholds for trades, derivative closes, and funding. Start the
controlled trade window only after the first authoritative `account_snapshot`
whose exchange timestamp strictly precedes the first retained fill;
restored fills before that basis intentionally fail closed. A settlement with a zero
position can legitimately have no bill, so completeness is demonstrated by
matched nonzero funding bills rather than by requiring one bill per scheduled
timestamp. The final trade-delay guard is excluded from fill-to-bill
completeness because a valid bill can arrive just after `END_MS`.
Keep the mark-price subscription healthy until a post-assessment observation is
journaled; a one-sided bracket, duplicate exchange timestamp within one session,
cross-session mark, or bracket outside the configured distance fails closed.

The derivative basis is authenticated REST state persisted by the local runtime,
not remote process attestation. Expiry-futures `avgPx` can reset at settlement;
unexplained settlement bills fail the controlled scope, but settlement-PnL
reconstruction remains separate. The venue's exact internal funding assessment
tick cannot be reproduced; instead, the
bill-reported mark must agree with a narrow two-sided public exchange-time
bracket. Boundary currency conversions are independently checked with direct
public indexes, but those sequential requests do not expose OKX's exact internal
valuation tick. Total-equity movement is reported rather than attributed to
mark-to-market PnL, taxes, deposits/withdrawals, or unsupported bill classes.
The current OKX documentation is not sufficient evidence for every cash-spot
bill unit combination, so archive a credentialed minimal cash buy and sell demo
sample and require this verifier to pass before admitting spot production data.

### API Key, Account Cash, And Liability Certification

Before the first credentialed observe/demo session, after any account-setting
change, and immediately before a production approval review, collect and then
independently verify current account state. Quiesce every order producer and
account-setting change for the collection window; the authenticated GETs are
bounded and bracketed, but they are not an atomic exchange snapshot.

```bash
ARTIFACT="var/reap/evidence/account-main-$(date -u +%Y%m%dT%H%M%SZ).json"
cargo run -p reap-cli -- certify-account \
  --config examples/live-okx-demo.toml \
  --account main \
  --output "$ARTIFACT" \
  --pretty

cargo run -p reap-cli -- verify-account-certification \
  --artifact "$ARTIFACT" \
  --require-pass \
  --pretty
```

`certify-account` uses public-time/direct-index and authenticated read-only
account configuration, balance, and positions GETs. The current
[OKX account-configuration contract](https://www.okx.com/docs-v5/en/#trading-account-rest-api-get-account-configuration)
reports the requesting key's `perm` and bound `ip` values. It reserves the
output before credentials or network access, writes it create-new with Unix mode `0600`, and
fsyncs both file and parent directory. The CLI prints only a redacted summary;
the artifact embeds the exact bounded responses and exact live TOML, so treat it
as sensitive account evidence and never publish it in logs or source control.

The schema-3 verifier needs no credentials. It checks the pinned Java revision,
re-hashes the embedded config and responses, re-derives the account identity
hash and policies, and checks endpoint/response bounds, exchange-clock skew, the
maximum 30-second collection span, bracketed UID/main-UID and settings stability,
the API-key label, exact permission-set equality, required exchange-reported IP
binding, configured account/position modes, cash-only spot routing, zero
strategy borrow limits, and mode-aware OKX economics. It derives the exact expected direct
`CCY-USD` index set from balance details, rejects missing/duplicate/stale ticker
evidence, checks each reported `eqUsd` against `eq * idxPx`, and checks the strict
sum of `eqUsd` against `totalEq`. Collector binary and host hashes are
recorded provenance identifiers; the self-contained artifact cannot
independently authenticate them. Spot mode requires explicit
`enableSpotBorrow = false`; multi-currency and portfolio modes require explicit
`autoLoan = false`. Any enabled borrowing flag, missing applicable liability
evidence, nonzero `borrowFroz`, `notionalUsdForBorrow`, `liab`, `crossLiab`,
`isoLiab`, `uplLiab`, or `interest`, or any returned OKX `MARGIN` position makes
the policy fail. Documented Futures-mode inapplicable empty fields do not fail
solely for being absent.

The account parity points are pinned Java `OkexV5AccountConverter` for `totalEq`,
`cashBal`, `eq`, `upl`, and `disEq`, plus `OkexV5RestClient.getIndex` and
`OkexV5L1Subscriber` for direct index bootstrap. Schema 2 additionally retains
current-wire `eqUsd` so offline verification can compare the venue conversion to
the independently captured index.

This is deliberately point-in-time proof. Public-index and authenticated account
requests are sequential, not an atomic valuation tick. It does not establish historical
absence of loans, reconcile borrow/repay or accrued-interest history, or replace
cash, position, funding, PnL, deposit/withdrawal, tax, and statement
reconciliation. Archive a passing artifact as one production gate input, not as
complete economic certification.

### Walk-Forward And Sensitivity Research

Verify the manifest-to-report path without claiming profitability:

```bash
rm -f /tmp/reap-research-smoke.json /tmp/reap-research-smoke-verification.json
cargo run -p reap-cli -- research \
  --manifest examples/research-smoke.toml \
  --output /tmp/reap-research-smoke.json --require-pass --pretty
cargo run -p reap-cli -- verify-research \
  --manifest examples/research-smoke.toml \
  --report /tmp/reap-research-smoke.json \
  --output /tmp/reap-research-smoke-verification.json \
  --require-pass --pretty
```

The smoke manifest has one tiny fold, one candidate, uncalibrated execution,
permissive risk gates, and negative fee-adjusted baseline PnL. Its only purpose
is testing candidate selection, chronology, stress propagation, provenance,
artifact creation, and exit status.

For real research, create a new manifest alongside immutable capture files:

1. Set `schema_version = 8`, `mode = "production_candidate"`, retain the pinned
   Java revision, and point `latency_calibration` to the passed create-new JSON
   artifact whose profile is embedded exactly in the baseline scenario.
2. List explicit full candidate TOML files and set `deployment_candidate_id` to
   the one strategy intended for deployment before any test-window result is
   inspected. The runner does not mutate arbitrary strategy fields or generate
   an implicit parameter grid.
   Candidate files must omit `[initial_portfolio]`; production opening capital
   belongs to dataset evidence, not a tunable candidate.
3. Give every logical window a unique dataset ID. Normally, give every capture
   session a unique immutable source path and content hash. One verified raw
   capture may instead be partitioned into disjoint inclusive
   `capture_record_range` windows. Link every later adjacent window with
   `continuation_of`; production chain roots must start at record `1`, and chains
   cannot branch. Every test window must occur strictly after its fold's training
   windows, and a test dataset can belong to only one fold. Set each raw dataset's `capture_config` and
   `capture_report` to the exact capture-only TOML and schema-5 report from that
   session. When the report declares normalized output, also set
   `normalized_path` to that retained artifact. Production mode verifies and
   embeds the report, reruns and retains strict capture analysis, and performs a
   zero-gap raw replay check before candidate evaluation. It also requires at
   least two connections per stream, an enabled host guard with at least one
   completed periodic check, the same executable as research, the same target
   host as latency calibration, and the
   book/trade/index/mark/limit/funding channels needed by every candidate.
   Quiesce the exact production account and collect one unique, passing
   `certify-account` artifact immediately before each capture session. Bind it
   only to that session's chain root; continuations derive their opening state
   from the settled parent. For example, a held-out window in one continuous
   capture can be declared as:

   ```toml
   [[datasets]]
   id = "train-20260715"
   path = "capture-20260715.jsonl"
   format = "raw_capture"
   capture_record_range = { first = 1, last = 500000 }
   capture_config = "capture-okx-public.toml"
   capture_report = "capture-20260715.report.json"

   [datasets.opening_account]
   certification = "account-20260715.json"
   spot_valuation_symbols = { BTC = "BTC-USDT" }

   [[datasets]]
   id = "test-20260715"
   path = "capture-20260715.jsonl"
   format = "raw_capture"
   capture_record_range = { first = 500001, last = 750000 }
   continuation_of = "train-20260715"
   capture_config = "capture-okx-public.toml"
   capture_report = "capture-20260715.report.json"
   ```

   Set `gates.maximum_opening_account_gap_ms` to the reviewed quiescent handoff
   budget, never above 15 minutes. Research re-verifies raw account evidence and
   requires the certification to finish before capture on the same exact build,
   calibrated host, production account, and instrument accounting scope. It
   rejects reused certifications, nonzero unmodeled currencies/positions, and
   candidate-dependent derived state. Every member of a range chain must resolve
   to the same raw/config/report/optional-normalized paths. Replay independently
   verifies the full parent file's session and process-global record sequence,
   warms parser/deduplication/book state plus the latest index, funding, mark, and
   price-limit state from the prefix, and otherwise emits only events in the
   selected range.
4. Define exactly one baseline using empirically calibrated execution values,
   set `calibrated = true`, and add at least two stress scenarios. Profile stress
   uses the same seed and must first-order stochastically dominate baseline for
   every effective class/symbol distribution. Threshold and queue can only
   increase; trade/depth participation can only decrease.
5. Set nonzero event, fill, and duration evidence minimums for both training and
   test windows plus explicit PnL,
   drawdown, terminal/maximum position and pending-hedge delta,
   terminal/maximum gross position and active-order exposure, active-order
   count, inventory-duration, clock-regression, accounting, and pending-work
   gates. When any candidate trades a swap, also set nonzero realized funding
   settlement minimums for both training and test folds.
6. Run with a create-new `--output` path and `--require-pass`. The output is
   reserved before research, written owner-only, synced, and followed by a
   parent-directory sync.
7. Run `verify-research` with the exact manifest, report, and byte-identical
   executable. Require `acceptance_passed = true`, identical normalized hashes,
   and no failures, then archive both JSON files beside the exact capture/config
   files and demo calibration evidence.
8. Run `verify-research-deployment` with that manifest/report and the exact
   proposed production live config. Require the format-3 research
   reconstruction, format-2 deployment binding, exact opening-config hashes,
   and effective strategy hashes to pass, then archive its create-new output
   with the production-transition evidence.

Candidate scores use training runs only (`net_pnl_usd` or
`pnl_per_turnover_bps`). Only the selected candidate is evaluated on test data.
Every production fold must training-select the predeclared deployment candidate;
one different or missing selection fails the fold and the aggregate report. The
manifest hash binds that declaration before the run, but operators must still
freeze the manifest before inspecting held-out results.
`verify-research-deployment` refuses smoke research and demo configs. It uses the
same effective `ChaosConfig` serialization used for candidate provenance, so
backtest-only execution settings cannot hide a strategy mismatch and a live-only
deployment wrapper cannot silently change strategy economics.

The report contains the selection rule, gate thresholds, every underlying run,
aggregate gate failures, and SHA-256 for the manifest, executable, candidate
files, effective strategies, and datasets. Candidates with identical effective
strategy configuration are rejected even when file comments or `[backtest]`
settings differ. Every file-backed input hash is checked again after all runs so
input mutation aborts artifact creation. Existing output paths are refused so an
acceptance artifact cannot be silently replaced.

The independent verifier accepts JSON whitespace/key-order changes and archive
relocation of verifier-observed capture and opening-account source paths, but it preserves the manifest's
declared paths and every result. It uses exact JSON `f64` round trips, re-runs
the entire manifest, and emits the first mismatching JSON pointer if a stale or
forged report differs. It deliberately does not infer profitability, venue
representativeness, target-account identity, or production authorization from a
successful deterministic reconstruction.

The embedded capture verification binds exact capture-config bytes and effective
output overrides to the run report, raw bytes, and optional independently
reconstructed normalized bytes. It cross-checks replayable counters, session,
and terminal book health while retaining runtime-only writer queue depth,
process stop reason, and connection-readiness evidence from the durable report.
The retained capture analysis adds source coverage, timing, and depth; the
independent raw replay check proves no sequence gap/recovery/failure and ready
terminal books. Any verifier failure aborts before candidate evaluation. The
report, config, raw file, and optional normalized file are re-hashed after all
candidate/scenario runs before the acceptance artifact can be created.

The three pending-work maxima bound delayed non-funding scheduler work,
exchange `PendingNew` orders, and unresolved cancel requests at a data cutoff.
They need not be zero: positive calibrated latency naturally censors the last
captured event, and the runner does not execute future actions against stale
depth. Resting live quotes are also valid at an arbitrary dataset boundary;
bound them with `maximum_test_active_orders`, active-order notional gates, and
pending-delta gates instead of treating them as completed work. A nonempty
portfolio run also leaves the next deterministic 10-second full-account refresh
scheduled beyond the cutoff. That refresh is counted as pending strategy work,
so calibrate `maximum_pending_non_funding_actions_per_fold` for at least those
per-run refreshes plus legitimate latency-censored tail actions.

Unchained datasets remain independent runs. They reset to zero, the exact
candidate snapshot, or a source-rebuilt certified chain-root snapshot and report
the corresponding independent portfolio semantics. Schema-8 adjacent raw ranges
instead report `sequential_settled_carry`. At every range boundary Reap mirrors
Java `ChaosBackTestMultiRunService.finishUp()`: it excludes live/pending orders
and delayed non-funding actions from carry, marks balances and derivative
positions at terminal prices, resets derivative average cost to that mark,
recomputes available/equity/margin fields, releases holds, and preserves pending
funding plus its settlement watermark. A terminal strategy halt or incomplete
accounting invalidates the carry. The child must have the same nonempty capture session, the exact next
process-global record ordinal, and a receive-time boundary no earlier than the
settled parent. The selected candidate's final training carry may seed the first
held-out test range when it is the immediate continuation; every stress scenario
starts from that same baseline economic state with its configured margin
assumptions rebound and revalidated.

This continuity applies only to explicit ranges of one verified capture file.
Rotated files and separate capture processes are still independent because the
writer does not yet preserve a verifiable session and process-global ordinal
across rotation. Research aggregation sums each run's `net_pnl_usd`, not final
equity; continuity does not turn final capital into profit.

Strict analysis requires one capture session, every configured stream on its
exact deterministic replica/chunk source connections, a ready book for every
configured book stream, no blank/comment records or unexpected data stream,
monotonic per-source receive time, and no parse, sequence, or unrecovered-book
defect. Analysis format 5 reports each stream's expected, missing, and
unexpected plan IDs in addition to source counts. It reports
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

After making ready/disconnect delivery lossless, a fresh 45-second run on
2026-07-14 reached and retained all 14 socket plans. It wrote 3,496 frames
(2,829,546 bytes), accepted 1,748 events, and classified 1,748 exact replica
duplicates. Capture, strict analysis, and strict replay reported zero gaps,
recoveries, failures, parse errors, stale books, disconnects, timestamp
regressions, or unrecovered books. Analysis found both configured sources for
all ten streams and reconstructed 400 bid and ask levels for both books. All
three SHA-256 checks matched at
`5317fee38e3beaad6d2ebb7eb81a9c73fac7fcbd31726fcbc1ccdd34fc572255`.
Raw-capture backtest replay consumed 2,584 normalized inputs, submitted and
activated eight simulated orders, cancelled four, and produced no fills. It
reported complete accounting and valuation, two cross-socket arrival-order
clock regressions, and one funding action beyond the capture horizon under the
explicit uncalibrated zero-delay execution model. This is a post-change
connectivity and replay smoke, not execution calibration or strategy evidence.

After adding durable report verification, a final 30-second run on 2026-07-14
used schema 3 and retained all 14 socket plans with no disconnect. It wrote
2,029 raw frames (1,453,359 bytes), accepted 1,015 events, and classified 1,014
replica duplicates. The mode-`0600` report bound the exact 1,611-byte config at
SHA-256
`10d812c7f9b0980cdf1bb0774310fef6415ae3a407e6ec20eed256ca11b5b2c5`.
Report-aware verification passed with no failure and matched the effective
config fingerprint, raw SHA-256
`7b4e6387be9f0ee19417d627472d8079b9c7f2ed6d45cbf10de3310a04e7707e`,
and all 1,539 reconstructed normalized records (12,208,209 bytes) at SHA-256
`4355383aa4a06b9f6f0a5f468b31fbcab062f3e1e962f5dbbd749a6d64a13adf`.
Strict analysis and replay found zero gap, recovery, parse, source-coverage, or
terminal-book defect; both books retained 400 levels per side. Receive-time raw
backtest submitted four simulated orders, retained four at the horizon, and had
complete accounting with no input-clock regression or fill. Diagnostic
normalized replay reported 221 global cross-stream exchange-time regressions,
including one of 41.937 seconds, despite monotonic per-source receive time. This
is direct evidence for keeping raw receive-time replay authoritative.
That historical schema-3 smoke predates binary/host guard provenance and is not
admissible to the current schema-5 production-candidate gate.

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
   fails bootstrap when skew exceeds `max_exchange_clock_skew_ms`. It then
   checks `/api/v5/system/status` and refuses startup when relevant
   unified-account maintenance is active or begins within
   `exchange_status_lead_ms`.
6. The runtime fetches account-scoped instruments, account configuration
   including the authenticating API key's label, permission set, and IP bindings,
   full economic balances, positions including margin-loan fields, open orders,
   recent fills, exact status for any restored active order, and the
   authenticated current fee group for every configured instrument.
7. It verifies live instrument state, type, linear/inverse contract type,
   tick/lot/minimum and maximum single-order sizes, applicable spot maximum
   order amounts, contract value, currencies, configured trade mode,
   account level, `net_mode`, the exact configured API-key permissions, any
   required exchange-reported IP binding, disabled mode-appropriate borrowing,
   complete
   applicable zero-liability/interest evidence, absence of margin positions,
   absence of imminent announced instrument-rule changes, and that configured
   maker/taker costs do not understate the applicable exchange commissions or
   overstate rebates before metadata is ready.
8. It restores canonical active orders, fill identities, and active global,
   account, and symbol safety latches from JSONL. It applies missed known
   fills/terminal updates from REST, applies the authoritative account snapshot
   to the strategy and risk engine, and marks that snapshot ready only after
   engine application. Before writing any of those restored/runtime events, it
   appends one schema-7 `session_start` per account with the generated session
   ID, config fingerprint, and hashed OKX account identity. Every subsequent
   authoritative REST account replacement is also persisted as a critical
   account-scoped `account_snapshot`. It then requires
   clean account-scoped reconciliation.
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
13. Live configuration must set
    `strategy.reference_data_stale_threshold_ms`. Startup waits independently
    for every derived index, swap-funding, derivative-mark, and instrument
    price-limit observation, checking exchange source time against host receive
    time. Activity on one reference socket cannot refresh another. A stale
    source degrades readiness, blocks entry, and immediately cancels canonical
    working orders; strategy timers independently withdraw stale quotes.
14. Only phase `ready`, writable storage, healthy risk, and explicit
    `--mode demo --confirm-demo` permit a new order.

Live validation rejects `strategy.master_strategy` and
`strategy.strategy_group`. The pinned Java implementation requires external
`StrategyUpdate` heartbeat, member-state, and aggregate-PnL inputs for those
settings; accepting only their static flags would weaken live stop behavior.

## Private Stream Health

- Orders, account, positions, and optional fills use isolated sockets. Every
  socket must acknowledge its exact unique serialized argument plan; duplicate
  acknowledgements do not advance readiness, and malformed or unexpected
  acknowledgements force reconnect. Any disconnect immediately emits
  account-scoped `PrivateStreamStale` and blocks new orders. Acknowledged-ready
  and disconnected transitions wait for bounded status capacity and are never
  silently dropped. Raw payloads do not emit redundant status-queue heartbeats.
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
  the reconciliation request pacer. Regular pending-order pages request the OKX
  maximum of 100 rows and continue by `ordId`; fill pages continue by `billId`.
  Each requires a short terminal page. A repeated cursor/order/fill or a full
  page at `runtime.max_order_reconciliation_pages` or
  `runtime.max_fill_reconciliation_pages` fails closed instead of accepting a
  partial snapshot. A fee-less optional
  fills-channel event does not consume the canonical journal key before the
  fee-bearing orders-channel event arrives. An empty balance response is
  rejected as non-authoritative; an empty position list is a valid flat account.
- The bootstrap economic snapshot is stricter than the normalized strategy
  update. It retains applicable aggregate/per-currency borrowing and interest
  fields and margin-position loan fields, then applies the same mode-aware cash
  policy used by `certify-account`. A violation prevents live task startup.
- Bootstrap also applies the configured API-key policy before task startup.
  Production evidence requires exactly `read_only` plus `trade`, rejects
  `withdraw`, and requires at least one exchange-reported IP binding. A
  successful authenticated request with a non-empty binding proves this egress
  was accepted by OKX under a bound key; secret custody and network ownership
  remain external controls.
- Every later normalized websocket or REST balance update rejects any nonzero
  liability before state application. This protects the running process even
  though the hot strategy event intentionally carries fewer economic fields
  than the certification artifact.
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
  a repeated `(symbol, fill_id)` when status is unchanged and cumulative fill
  does not advance, and suppresses an unchanged terminal status by canonical
  order ID. Fill identity is instrument-scoped because OKX guarantees each
  `tradeId` only within an instrument. Current journals retain that scope;
  legacy unscoped bootstrap IDs are conservative restart wildcards. A genuine
  state transition or cumulative-fill increase still reaches the event loop.
  This follows the current [OKX order-channel duplicate guidance](https://www.okx.com/docs-v5/log_en/#order-channel-revamp).

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
  do not run this live downside guard. Their `backtest.currency_rates` routes
  provide depeg-sensitive valuation and completeness evidence, not a simulated
  kill switch.

## Process Ownership And Host Guard

- The journal's sibling lock file contains PID and acquisition-time metadata
  and is mode `0600` on Unix. The kernel file lock, not the file's existence or
  metadata, is authoritative. A stale lock file after a crash is expected; do
  not delete it while any runtime may still hold the lock.
- Canonical parent resolution means relative paths and symlinked directory
  aliases contend for the same lease. The lease remains owned by the runtime
  even if the storage writer task fails and is released only during teardown or
  failed startup cleanup.
- Live config/run-option validation and exact source/build/host provenance are
  completed before `--output` reservation. After reservation, raw startup task
  handles remain abort-on-drop until ownership transfers into `LiveRuntime`, so
  a later account/bootstrap setup failure cannot detach those workers.
- Alert routing and host thresholds are deployment-only and are excluded from
  checkpoint identity, so enabling them does not invalidate an existing
  reconciled journal. Trading, account, runtime, storage, and operator changes
  remain fingerprint-bound. Live evidence reports separately hash every
  effective setting, so changing either guard invalidates latency calibration
  compatibility even though checkpoint recovery remains valid.
- Set `[host_guard].enabled = true` for deployment. Choose disk and memory
  thresholds above the amount needed for a full fault/reconciliation window,
  not merely enough for the next flush. Enabled intervals are capped at 60
  seconds. A failed preflight prevents credential reads and network startup; a
  failed periodic check is a fatal runtime event.
- The host clock check reads Linux kernel synchronization state. It complements,
  rather than replaces, the independent midpoint-adjusted OKX server-time
  checks. The deployment supervisor must still monitor the actual NTP/PTP
  service and clock offset.

## Process Supervision

Hardened baseline units and installation notes live in
[`deploy/systemd`](../deploy/systemd/README.md). They run as an unprivileged
`reap` user with a strict read-only filesystem view, one instance-specific
writable directory, bounded file descriptors/tasks, no capabilities, no
realtime scheduler acquisition or clock-write authority, and isolated process
visibility, IPC, devices, temporary files, keyrings, and namespace acquisition.
`ProtectClock=true` is deliberately absent because it also blocks the host
guard's read-only `adjtimex()` synchronization probe. The
hermetic `deploy/systemd/verify-units.sh` gate validates every mode-specific
command/restart/path invariant, runs `systemd-analyze verify`, and caps reported
offline security exposure at `4.0`; the checked-in units currently score `2.9`.
All three units additionally share the persistent owner-only
`/var/lib/reap/connectivity` state directory. Deployed capture and live configs
must set `connection_attempt_pacer_path =
"/var/lib/reap/connectivity/okx-global.pacer"`.

- `reap-observe@.service` may restart on failure because observe mode cannot
  submit or cancel. A start limit bounds repeated bootstrap failures.
- `reap-demo@.service` never restarts automatically. Any abnormal exit requires
  independent exchange reconciliation and operator approval first.
- `reap-capture@.service` never restarts automatically. Its instance environment
  contains only `REAP_CAPTURE_DURATION_SECS`; the command always requests a
  bounded clean capture and create-new `raw.jsonl` and `run-report.json`. Every
  instance reuses `/etc/reap/capture/okx-btc-public.toml`; use a new instance
  name, environment file, and output directory for every run so no artifact can
  contain multiple capture session IDs or overwrite prior evidence.
- Set `TimeoutStopSec` above the configured runtime cancel/reconcile plus
  whole-runtime teardown deadlines; alert drain is nested inside teardown. The
  checked-in policy uses 15 + 15 seconds under a 45-second unit boundary. A
  forced kill leaves the last exchange deadman in force but must be treated as
  an incident, followed by the emergency procedure below.
- Monitor activation failures, non-zero exits, start-limit exhaustion, forced
  kills, and host resource/time alarms outside this process. Validate the unit
  files, merged drop-ins, and `systemd-analyze security` on the actual target OS.
  The hermetic source gate is not runtime, paging, clock, resource, or restart
  evidence; archive those independently from the target host.

The pinned Java `MetCoinGatewayWsClientsOkexV5Config` owns in-process public,
position, and order websocket construction, but it does not define an external
process supervisor. This policy wraps the Java-referenced connectivity and
strategy behavior with Rust deployment controls rather than claiming parity.

Use absolute live storage and operator-socket paths below the instance's
writable directory in deployed TOML, plus the exact shared pacer path above.
The capture unit supplies its absolute instance output paths on the command
line; its checked-in TOML remains byte-identical between runs. Config and
environment files must be
readable only by root and the `reap` group; the environment file is populated by
the deployment secret provider.

## Independent Emergency Cancel

`reap-emergency` is a separate executable and composition root. It does not acquire or
trust the strategy journal, operator socket, live event loop, websocket state,
or strategy configuration. It parses only the OKX environment/REST settings,
request pacing/timeouts, account credential environment names, and configured
symbol keys. This keeps the exchange cancellation path available when the live
strategy config or process is unhealthy.

This command belongs to the isolated emergency plane defined by
[chaos-connectivity-boundary.md](chaos-connectivity-boundary.md). Its
regular/algo/spread authority is deliberately broader than live Chaos and MUST
NOT be used as evidence that the strategy needs those mutation APIs. The
[refactor plan](chaos-connectivity-refactor-plan.md) moves that authority out
of the live dependency surface and makes the three domain workflows progress
independently.

The command covers all documented pending-order domains on each selected OKX
account, including regular symbols not configured in the strategy: regular
order-book orders, untriggered algo orders, and Nitro spread orders. It pages
each endpoint to a short terminal page under a strict bound and rejects repeated
cursors or order IDs. OKX regular Cancel All After excludes spread, so the
command arms both the regular and spread CAA endpoints. OKX documents no algo
CAA; the required producer-stop confirmation, explicit algo cancellation, and
authoritative post-cancel polling form that domain's safety boundary.

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
REPORT=/var/lib/reap/live/btc-demo/emergency-cancel-$(date -u +%Y%m%dT%H%M%SZ).json
VERIFICATION=/var/lib/reap/live/btc-demo/emergency-cancel-verification-$(date -u +%Y%m%dT%H%M%SZ).json
/usr/local/bin/reap-emergency \
  --config /etc/reap/live/btc-demo.toml \
  --account main \
  --confirm-account-wide-cancel \
  --confirm-order-producers-stopped \
  --confirm-production \
  --account-timeout-secs 40 \
  --deadman-timeout-secs 10 \
  --output "$REPORT" \
  --pretty
/usr/local/bin/reap verify-emergency-cancel \
  --config /etc/reap/live/btc-demo.toml \
  --report "$REPORT" \
  --output "$VERIFICATION" \
  --require-all-configured-accounts \
  --require-pass \
  --pretty
```

`--confirm-production` is mandatory for a production venue config and harmless
for demo. `--all-configured-accounts` is available only when every configured
account's producers have been stopped and replaces all `--account` arguments.

4. For each account, the command samples exchange time, arms regular and spread
   Cancel All After, then exhaustively enumerates regular, algo, and spread
   orders. It cancels regular orders in batches of at most 20, algo orders in
   batches of at most 10, invokes spread mass cancel, and re-enumerates every
   domain until all are zero after the deadman trigger horizon. Every page,
   cancel, pacing delay, and final query shares one absolute account timeout.
   Cancel acknowledgements are only request acceptance; the final REST zero
   snapshot is authoritative.
5. Both output paths are owner-only create-new files, synced with their parent
   directory before final status. `verify-emergency-cancel` rejects symlinks and
   oversized files, hashes the exact config/report bytes, rejects unknown JSON
   fields, re-runs the command's pure REST-origin/pacing/timing-budget checks,
   re-derives configured/selected/report account coverage, validates both
   deadmen, the trigger horizon, and final zero in every order domain, and
   recomputes every top-level
   completion flag. Use `--require-all-configured-accounts` for gate evidence.
   Require schema 2, the pinned Java revision, expected Reap binary and host
   hashes, each pseudonymous `account_identity_sha256` matching the corresponding
   live report, and `acceptance_passed = true`. Review all
   provenance, execution, and account incidents, plus
   regular/algo partial, rejected, and unacknowledged cancel counts, spread
   mass-cancel attempts, unmanaged symbols, and per-domain remaining-order
   details even when final zero succeeds. An account-task
   failure is retained as non-passing evidence.
6. A failure before report construction leaves the reserved file empty; archive
   it with process logs but never treat it as JSON evidence. Independently
   independently inspect OKX regular, algo, and spread orders plus balances and
   positions.
   Archive the report with the incident journal. A non-zero command exit or
   missing/failed account report means zero was not proven and requires venue
   UI/API escalation.
7. Leave demo trading stopped. Recover the immutable journal, reconcile exchange
   orders/fills/positions, and start `observe` first. Restore demo entry only
   after clean readiness and explicit operator approval; never auto-restart from
   this procedure.

The verifier can establish report/config integrity and internal consistency; it
cannot replay raw REST bodies, authenticate the recorded executable/host hashes,
or prove the producer-stop attestation. This is an out-of-process safety layer,
not a production certification or a replacement for exchange-side account
limits and operator access controls.

## Process-Death Deadman Certification

`reap certify-deadman-expiry` is a read-only evidence path for one controlled,
minimal-size demo fault campaign. It proves a narrower and stronger claim than
the live report or emergency command: orders that were durably `live` or
`partially_filled` immediately before process death were later cancelled by OKX
Cancel All After itself. OKX documents cancellation source `20` as Cancel All
After, and the pinned Java repository contains the same
`cancelSource = "20"` / `Cancel all after triggered` order-detail fixture.

Do not delay cancellation during a real incident to collect this proof. Run the
independent emergency command immediately whenever exposure is uncertain. Do
not run emergency cancellation before a planned deadman-expiry certification;
an explicit cancel changes the causal evidence and should make the campaign
fail.

Controlled demo procedure:

1. Use a dedicated demo account with no algo or spread orders and the smallest
   configured order size. Start `live --mode demo` under the target supervisor,
   wait until at least one order has a durable exchange acknowledgement and is
   live, and archive the PID, supervisor status, config, binary hash, and fault
   injector record.
2. Send `SIGKILL` to the Reap main PID. Confirm the unit is inactive, its main
   PID is zero, restart is disabled, and every other API/manual order producer
   for the account is stopped. Preserve the canonical journal without editing,
   truncating, rotating, or restarting against it.
3. Wait at least `runtime.cancel_all_after_timeout_secs` plus the exchange
   cancellation-processing allowance used by the campaign. Ten additional
   seconds is the baseline; record the actual timestamps. Inspecting exchange
   state is allowed, but do not issue a cancel or refresh Cancel All After.
4. From the restricted credential shell, collect create-new evidence:

```bash
umask 077
REPORT=/var/lib/reap/live/btc-demo/deadman-expiry-$(date -u +%Y%m%dT%H%M%SZ).json
/usr/local/bin/reap certify-deadman-expiry \
  --config /etc/reap/live/btc-demo.toml \
  --account main \
  --confirm-order-producers-stopped \
  --output "$REPORT" \
  --pretty
```

5. Before any restart or journal rotation, independently re-derive the result.
   The verifier needs no credentials and takes its own exclusive journal lease:

```bash
/usr/local/bin/reap verify-deadman-certification \
  --artifact "$REPORT" \
  --journal /var/lib/reap/live/btc-demo/live-events.jsonl \
  --require-pass \
  --pretty
```

The collector acquires the canonical journal lease before reading credentials
or starting network work. It then uses only public time and authenticated GETs
for bracketed account configuration, each recovered order detail, and the
unfiltered regular `orders-pending` endpoint. It never arms, refreshes, or
disables Cancel All After and never sends an order or cancel request. The
create-new owner-only artifact embeds the exact config and raw bounded OKX
responses and fingerprints, but does not embed the potentially large journal.

A pass requires all of the following:

- The exact journal has a complete tail and matching account/strategy/config
  bootstrap identity, and the collector held its exclusive lease.
- At least one recovered order was `live` or `partially_filled`; no selected
  order was only `pending_new`, and every recovered live order had a durable
  exchange/client binding.
- Every bound order-detail response matches client/exchange IDs, symbol, side,
  price, and size; its update/fill state does not regress behind the journal;
  it is `canceled`; and it has `cancelSource = "20"`. The human-readable reason
  is retained but is not used as the invariant.
- The account-wide regular pending-order response is empty, bracketed account
  identity/settings are stable and configured as expected, exchange-clock
  evidence is valid, and the stopped-producer attestation is present.
- Offline verification reproduces the journal hash/recovery, response hashes,
  parsers, query identities, failure list, summary, and pass result.

This process-death evidence covers regular orders only. The producer attestation cannot prove
that unrelated hosts, keys, or manual traders were stopped, and the lease only
excludes cooperating Reap processes that use the same journal. Archive target
supervisor/injector records plus fill/fee, balance, position, and account-statement
evidence. After collection, run the account-wide emergency cancel to prove
regular/algo/spread zero, reconcile the stopped journal, and restart in `observe`
only after review.

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
- Teardown drains queued events within `alerts.shutdown_timeout_ms`, itself
  bounded by `runtime.teardown_timeout_ms`. Monitor the report fields
  `alerts_delivered`, `alert_delivery_failures`,
  `alert_failure_notifications_dropped`, and `max_alert_queue_depth`. A process
  supervisor must page on non-zero exit as the independent fallback when the
  alert destination itself is unavailable.

## Order Path

- The coordinator generates the client order ID and synchronously records a
  canonical `PendingNew` before dispatching to the account gateway task. The
  intent, pending state, and request are enqueued to critical storage before
  authenticated websocket write begins.
- Demo order entry requires every nonempty account command lane in the resolved
  Chaos connectivity plan. The current plan derives one lane per executing
  account and stable underlying keys deduplicate related spot/swap/future
  symbols into one dispatch family. Reference-only accounts construct no
  mutation role or order-command session.
- `order_websocket_sessions` is accepted for one migration window only as a
  per-account upper bound on derived lanes. It cannot create an idle session.
  `public_connections_per_subscription` is likewise only a maximum cap on each
  plan-derived replica count: books require two named recovery replicas, while
  trades and references require one. Validation emits both deprecations and
  rejects a cap below a mandatory plan count.
- Connection/login and control writes are bounded, including the shutdown close.
  Pong-derived heartbeat status uses non-blocking delivery at both bounded
  telemetry queues; saturation drops only that redundant heartbeat. Ready,
  disconnect, and fatal transitions remain lossless, so telemetry backpressure
  cannot stall order commands or delay the Java-mapped disconnect cancellation
  and reconciliation path.
- Each account command owner keeps a FIFO per underlying and permits one
  operation per family. The current single planned lane bounds total in-flight
  work to one acknowledgement at a time, while same-family submit/cancel order
  remains stable and all idempotency completion stays single-owner. Adding a
  lane requires a measured nonempty capacity or isolation requirement; the
  sample 4,096-command queue exceeds its documented 2,400-command
  four-ack-horizon threshold.
- Full REST reconciliation runs on an independent bounded account task. It
  shares account pacing reservations with command IO but cannot sit behind a
  websocket acknowledgement. Fail-closed shutdown flushes every earlier
  command before requesting the authoritative zero-order snapshot.
- Route explicit exchange rejections back through gateway state. A session
  unavailable before send is an explicit non-submit. Treat write, timeout,
  disconnect, and correlation ambiguity as pending until REST/private
  reconciliation resolves it; never blindly resubmit.
- `order_websocket_ack_timeout_ms` bounds post-write acknowledgement waiting.
  The ambiguous-submit grace must cover both `order_request_expiry_ms` and this
  acknowledgement window; the order-state convergence budget must also cover
  the acknowledgement window. A command-session loss blocks entry, initiates
  canonical account cancels, invalidates the last clean reconciliation, and
  requires both session recovery and a fresh clean REST pass.
- Normal cancellation uses websocket first and falls back to authenticated REST
  only when the routed session is known unavailable before send. An ambiguous
  websocket cancel is reconciled instead of being silently retried.
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
- Bootstrap and the first account safety task poll the official
  [OKX system-status endpoint](https://www.okx.com/docs-v5/en/#status-get-status). Defaults
  match pinned Java: a 10-second interval and 60-second lead. Relevant
  unified-account trading, account-batch, product-batch, spread, and `99`
  events for the configured demo/production environment trigger normal
  fail-closed cleanup. Websocket, block, bot, and copy-trading events retain
  Java's non-blocking treatment. A failed or malformed status response is
  fatal. OKX permits one status request per five seconds, which configuration
  validation enforces. One Reap process polls once regardless of its account
  count. Operators running multiple processes from one source IP must
  coordinate their polling intervals so the aggregate request rate remains
  within that limit.
- After each successful periodic clock check, the safety task fetches
  authenticated account configuration and compares the typed identity, API-key
  label/permissions/IP bindings, account and position modes, STP setting, and
  borrowing flags to bootstrap. Any drift
  or failed check is fatal and enters normal fail-closed cleanup.
- Bootstrap and the periodic account fee guard use the official authenticated
  [OKX fee-rates endpoint](https://www.okx.com/docs-v5/en/#trading-account-rest-api-get-fee-rates).
  Every private instrument `groupId` is matched to a current `feeGroup` row for
  the exact spot instrument or derivative family. OKX reports a negative rate
  for commission and a positive rate for rebate; Reap converts this to its cost
  convention and permits only equal or more conservative configured costs.
  Missing groups, deprecated-only responses, request failures, understated
  commissions, or overstated rebates are fatal. The full sweep defaults to one
  minute and is paced for the five-requests-per-two-seconds user limit. Its
  request runs in an isolated child task and therefore cannot delay Cancel All
  After. OKX notes that Open API rates may not reflect temporary zero-fee pairs,
  so this is a conservative safety gate rather than calibration evidence.
- Before each periodic fee lookup, the same isolated child re-queries the exact
  authenticated [OKX account instrument](https://www.okx.com/docs-v5/en/#trading-account-rest-api-get-instruments).
  It compares live state, type, family, underlying, currencies, contract
  type/value, tick/lot/minimum size, `maxLmtSz`, `maxMktSz`, applicable spot
  `maxLmtAmt`/`maxMktAmt`, and fee group to the bootstrap snapshot.
  It also strictly parses every current `upcChg` announcement: `tickSz`,
  `minSz` (which synchronously changes derivative `lotSz`), and `maxMktSz`.
  Unknown or missing announcement contracts fail closed. A change entering
  `exchange_instrument_change_lead_ms` is fatal even before it takes effect;
  the default one-hour lead leaves time to stop, review configuration, and
  cleanly restart. Exact metadata requests are paced within OKX's documented 20
  requests per two seconds per user and instrument type. A blocked metadata
  request, like a blocked fee request, cannot delay Cancel All After.
- Every outgoing order is a limit-family order (`post_only`, `ioc`, or plain
  `limit`). Bootstrap rejects configured quote quantity/amount maxima above the
  authenticated `maxLmtSz`/`maxLmtAmt`, and the live pre-trade gate rechecks the
  final emitted quantity and spot USD amount. This final check also covers
  Java-equivalent IOC hedge summaries that aggregate multiple depth levels.
- Validation requires the exchange deadman horizon to exceed three complete
  REST request timeouts plus one heartbeat interval, so delayed
  clock/config/status checks cannot consume the last armed Cancel All After
  window.
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

## Demo Fault Proxy

`reap fault-proxy` is an out-of-process campaign tool. It accepts only official
OKX demo upstreams, exposes separate loopback REST, public, private-state, and
order-command routes, and cannot produce a production-eligible endpoint tuple.
The proxy forwards authenticated traffic in memory but does not log or retain
authorization headers, login frames, raw account/order payloads, or injected
response bodies. Run it under the same protected service account as other demo
evidence tooling and never expose its listeners or Unix socket beyond the host.
`request_timeout_ms` bounds REST requests, local and official-upstream websocket
handshakes, and every websocket forwarding write. Handshakes and writes are
shutdown-cancellable; paired close writes are independently capped at one
second. Clean, peer-close, protocol-error, timeout, and shutdown exits all remove
the bridge from the active-connection registry. Shutdown cancellation itself is
not recorded as a proxy error, while a genuine timeout remains a non-passing
campaign result.

The proxy config must be a non-symlink regular file. Its source is canonicalized
before artifact paths and the effective fingerprint are derived, so running the
checked-in relative-path commands and later verifying with an absolute path does
not create false config drift.

Prepare one exact routed config for the campaign. Output paths are create-new;
do not delete or overwrite artifacts from an earlier run:

```bash
RUN_ID="$(date -u +%Y%m%dT%H%M%SZ)"
FAULT_ROOT="var/reap/fault"
ROUTED_CONFIG="${FAULT_ROOT}/live-${RUN_ID}.toml"
install -d -m 700 "$FAULT_ROOT"
reap render-fault-live-config \
  --live-config examples/live-okx-demo.toml \
  --proxy-config examples/okx-demo-fault-proxy.toml \
  --output "$ROUTED_CONFIG" \
  --pretty
reap live --config "$ROUTED_CONFIG" --mode validate --pretty
```

The renderer requires the source live endpoints to match the proxy's official
demo upstream exactly. It adds `venue.order_ws_url` so private account-state and
order-command faults are attributable to different sockets. It also requires
the proxy and source live config to declare the same connection-attempt interval
and pacer path, then clears both from the generated loopback config. Local live
handshakes therefore do not consume a slot twice; each proxy bridge reserves one
slot immediately before its official OKX upstream handshake. Outside a routed
test config `venue.order_ws_url` is optional and defaults to `private_ws_url`.

For each matrix role, start a fresh proxy process and one fresh bounded live
session. Keep the routed config bytes identical across roles, but use unique
session, command, evidence, live-report, and proxy-report names:

```bash
ROLE="public-reconnect"
PROXY_REPORT="${FAULT_ROOT}/proxy-${RUN_ID}-${ROLE}.json"
LIVE_REPORT="${FAULT_ROOT}/live-${RUN_ID}-${ROLE}.json"

# Process 1
reap fault-proxy \
  --config examples/okx-demo-fault-proxy.toml \
  --output "$PROXY_REPORT" \
  --duration-secs 1800 \
  --require-clean-shutdown \
  --pretty

# Process 2, after the proxy listeners are ready
reap live \
  --config "$ROUTED_CONFIG" \
  --mode observe \
  --duration-secs 900 \
  --output "$LIVE_REPORT" \
  --pretty
```

Use `demo --confirm-demo` instead of `observe` only for the minimum-size roles
that require order entry, after account certification and clean observe
acceptance. Query proxy state before injection:

```bash
reap fault-proxy-control \
  --socket var/reap/fault/control.sock \
  --command examples/faults/status.json \
  --require-accepted \
  --pretty
```

The files under `examples/faults/` are protocol-checked templates. Before each
fault, place a copy in the campaign directory and change both `command_id` and
`evidence_file` to unique role/run values. Arm exactly one expected condition
and require command acceptance:

```bash
reap fault-proxy-control \
  --socket var/reap/fault/control.sock \
  --command "${FAULT_ROOT}/${RUN_ID}-${ROLE}.command.json" \
  --require-accepted \
  --pretty
```

Disconnect evidence is completed when the selected bridge acknowledges closure.
REST-response and websocket-drop evidence appears only after every requested
match occurs. A pending rule, failed disconnect, proxy error, active websocket,
or stale control socket makes the proxy report unclean. Do not arm speculative
rules in the same run.

Use each command at the boundary it is intended to test:

| Template | Arming point and required review |
| --- | --- |
| `public-reconnect.json`, `private-reconnect.json`, `order-transport-reconnect.json` | Arm only after the live runtime is ready. Confirm the selected transport disconnects, readiness changes as expected for its redundancy, and recovery includes the required fresh reconciliation. |
| `ambiguous-submit.json`, `ambiguous-cancel.json` | Arm immediately before one reviewed minimum-size order action. These drop the matching order-command acknowledgement from exchange to client; they do not prove whether the exchange accepted, rejected, filled, or cancelled the request. |
| `order-convergence-timeout.json` | Arm immediately before one reviewed post-only submit or cancel. It drops one exchange-to-client private `orders` frame; require the live timeout, account block, cancel/retry behavior, full REST repair, and no early readiness recovery. |
| `fill-convergence-timeout.json` | For derivatives, arm the checked-in private `positions` drop immediately before an action expected to fill. For spot, change only the matcher channel to `account`. Require the live timeout and authoritative repair; one dropped frame does not prove a fill or economic state. |
| `deadman-heartbeat-failure.json` | Arm only after ready and after a successful Cancel All After heartbeat. It fails the next matching heartbeat. Verify the previously armed exchange timer remains effective while fail-closed cancellation/reconciliation runs. This is not process-death expiry certification. |
| `exchange-clock-failure.json`, `exchange-status-failure.json`, `exchange-instrument-failure.json`, `exchange-fee-failure.json`, `account-config-failure.json` | Arm only after ready so the artifact covers a periodic runtime check rather than bootstrap refusal. Leave the one-shot rule pending until its exact endpoint is called, then require the corresponding typed runtime failure and zero-order shutdown. |

The checked-in clock/status/instrument/fee/account-configuration commands inject
`503`, so they prove the documented `*_check` branch only; the deadman command
proves `deadman_heartbeat`. For skew, maintenance, instrument/fee/account drift,
or an imminent `upcChg`, use a separately reviewed command with the exact
endpoint and a valid changed `200` response. Archive that command and independent
source response: typed proxy evidence stores response metadata and a body hash,
not a claim that the injected body represents a valid exchange state.

Stop the live process first and let its cancel/reconcile lifecycle complete.
Require proxy status to show zero pending faults and zero errors, then submit
`examples/faults/shutdown.json`. After the proxy exits, independently reconstruct
its schema-2 process report:

```bash
PROXY_VERIFICATION="${FAULT_ROOT}/proxy-${RUN_ID}-${ROLE}-verification.json"
reap verify-fault-proxy-run \
  --config examples/okx-demo-fault-proxy.toml \
  --report "$PROXY_REPORT" \
  --output "$PROXY_VERIFICATION" \
  --require-pass \
  --pretty
```

The verifier rejects legacy/unknown fields, config drift, invalid build/host/
session provenance, inconsistent wall/monotonic timing, listener or control
socket cleanup failure, pending faults, active connections, retained proxy
errors, and a forged clean flag. Archive the exact source and routed configs,
live report, raw proxy run report, its verification, completed injector artifact,
executable hash, supervisor record, and any separate exchange/account evidence.
Add only the live report and completed injector artifact to the schema-3 fault
matrix; add the raw proxy report to the scenario-matched `fault_proxy_runs` entry
in the production-evidence manifest. The aggregate reruns this verifier and does
not trust the standalone verification JSON.

For supported Reap artifacts the matrix verifier checks strict structure, effect
count/timing/hash, and binds the command to its exact reconnect, ambiguity,
convergence, deadman, clock, status, instrument, fee, or account-configuration
role. It rejects Reap proxy artifacts for clean runs, genuine partial fills, and
restored-latch runs. Other injector formats are only hashed and remain
operator-reviewed evidence.

This proxy closes a reproducibility gap; it does not establish fault causality
by itself. A genuine partial fill and a durable latch restored after restart
remain external campaign roles. Process death, deadman expiry, emergency
cancellation, partial-fill economics, and account-statement reconciliation still
require their independent procedures and artifacts.

## Fail-Closed Matrix

| Condition | Automatic state | Required action |
| --- | --- | --- |
| Public sequence gap | Book recovering; new orders blocked | Obtain a fresh snapshot and replay contiguous buffered deltas |
| Public feed stale | Symbol blocked; live orders cancelled | Restore at least one healthy feed and verify sequence continuity |
| Private stream stale | Account blocked; live orders cancelled | Reconnect, REST reconcile orders/fills/balances/positions, then emit recovery |
| Order-command websocket stale | Account blocked; live orders cancelled; prior reconciliation invalidated | Reconnect every command session and require a new clean REST reconciliation before entry |
| Submit/cancel order-state convergence timeout | Account blocked; active orders cancelled; expired cancel retried; full reconciliation requested | Suppress each orders-channel transition independently, then verify REST repair and no early readiness recovery |
| Fill/account-state convergence timeout | Account blocked; live orders cancelled; full reconciliation requested | Inspect the missing symbol/currency target and private-channel latency, then require authoritative repair and a clean pass |
| Position scope or margin-mode violation | Bootstrap/runtime abort; demo cleanup attempts cancel/reconcile and retains the deadman unless safe | Keep entry disabled; close/correct the unmodeled position or mode outside Reap, then require a clean bootstrap |
| Forced-repayment indicator at/above limit | Bootstrap/runtime abort; demo cleanup attempts cancel/reconcile and retains the deadman unless safe | Keep entry disabled; reduce borrowing/risk outside Reap and require all currencies below limit plus a clean bootstrap |
| Reconcile drift | Account blocked; live orders cancelled | Inspect the recorded full-state differences and require a later clean pass before recovery |
| Stablecoin reference missing/stale/conflicting/depegged | New entry blocked immediately; durable global kill and live-order cancellation after debounce | Verify both redundant index routes and an independent venue reference; use the stopped-process latch-clear procedure after a sustained breach |
| Risk breach | Durable global kill active; live orders cancelled | Reduce exposure externally if needed, diagnose, and follow the stopped-process latch-clear procedure; restart alone does not clear it |
| Manual account kill | Durable account route latch; its instruments are removed from pricing/hedging and its live orders are cancelled | Reconcile the account and dependent exposure; restart alone does not clear it |
| Exchange clock/deadman failure | Runtime fatal; new entry blocked; armed Cancel All After remains effective | Verify host time and OKX reachability, then reconcile before restart |
| Relevant or unreadable OKX system status | Startup refused or runtime fatal; new entry blocked and normal cancel/reconcile cleanup runs | Review the official maintenance notice and configured environment; require a clean status check and full bootstrap after service recovery |
| Instrument becomes non-live, metadata or hard order maxima drift, an announced rule change enters its lead, the exact response is unreadable, or a final order exceeds current maxima | Startup refused, runtime fatal, or pre-trade order rejection; new entry remains guarded and normal cancel/reconcile cleanup applies to runtime drift | Stop through the effective change, review tick/lot/minimum/maximum size, spot amount limits, valuation, and account ownership; update and re-approve config if required, then require a clean exact bootstrap |
| Configured fee underprices authenticated OKX group, or fee check is unreadable | Startup refused or runtime fatal; new entry blocked and normal cancel/reconcile cleanup runs | Review the exact instrument group, account tier, configured cost sign, and current API response; recalibrate and require a clean bootstrap rather than weakening the guard |
| Host disk/memory/clock failure | Startup refused or runtime fatal; new entry blocked | Restore capacity/time synchronization, inspect journal integrity, and reconcile before restart |
| Alert queue/delivery failure | Runtime fail-stop when fatal delivery is configured | Verify the external route and supervisor fallback before restart |
| Journal lease contention | Second process refuses startup before credentials/network | Identify the owning PID/process; never bypass the lock or share the journal |
| Critical storage loss/backpressure | Runtime fail-stop; checkpoint reconciliation required on restart | Investigate disk/queue capacity; critical records are never silently dropped |

For a credentialed fault campaign, archive the report and require the following
structured evidence rather than matching log text:

| Injected condition | Required schema-8 evidence |
| --- | --- |
| Public websocket loss | `public_connection_disconnect_events > 0`; verify the expected readiness impact for the configured replica count |
| Private websocket loss | `private_connection_disconnect_events > 0`, a readiness loss, and later ready state after REST reconciliation |
| Order-command websocket loss | `order_transport_disconnect_events > 0`, `order_transport_stale_events > 0`, a readiness loss, and later ready state after session recovery plus REST reconciliation |
| Ambiguous websocket submit or cancel | The corresponding `ambiguous_submit_events` or `ambiguous_cancel_events` counter is nonzero |
| Exchange partial fill | `partial_fill_events > 0`, plus canonical fill/fee statement reconciliation for the run window |
| Suppressed fill/account state | `fill_convergence_timeout_events > 0` and a matching reconciliation-drift response |
| Suppressed order state | `order_convergence_timeout_events > 0` and a matching reconciliation-drift response |
| Restart with a durable halt | `restored_safety_latches > 0`; startup-replayed partial orders do not increment `partial_fill_events` |
| Cancel All After heartbeat failure | `stop_reason = "runtime_failure"` and `failure.code = "deadman_heartbeat"` |
| Periodic exchange-clock skew/check failure | `failure.code = "exchange_clock_skew"` or `"exchange_clock_check"` |
| Relevant or failed exchange-status check | `failure.code = "exchange_status"` or `"exchange_status_check"` |
| Authenticated instrument drift/check failure | `failure.code = "exchange_instrument_drift"` or `"exchange_instrument_check"` |
| Authenticated fee underpricing/check failure | `failure.code = "exchange_fee_drift"` or `"exchange_fee_check"` |
| Authenticated account-config drift/check failure | `failure.code = "account_config_drift"` or `"account_config_check"` |

Run each role in isolation, preserve its external injector record, populate the
checked-in manifest template, and create a durable aggregate result:

```bash
MATRIX=/var/lib/reap/live/btc-demo/live-fault-matrix.toml
RESULT=/var/lib/reap/live/btc-demo/live-fault-matrix-$(date -u +%Y%m%dT%H%M%SZ).json
reap verify-live-fault-matrix \
  --config /etc/reap/live/btc-demo.toml \
  --manifest "$MATRIX" \
  --output "$RESULT" \
  --require-pass \
  --pretty
```

The schema-3 manifest template is `examples/live-fault-matrix.toml`; relative
artifact paths resolve from the manifest directory. The verifier emits a
format-4 report with optional validated Reap proxy summaries and requires distinct
status, instrument, fee, and account-config failure roles and rejects missing
or duplicate roles, typed-failure substitution, report/session reuse, injector
path or content reuse, config byte drift, invalid schema-8 evidence, and
cross-run build, host, or account identity changes. Clean reconnect roles must
recover to a clean bounded soak.
Ambiguity and convergence roles may retain their expected drift counters but
must finish a bounded run back at ready with no storage drops, alert failures,
or active orders. Typed safety-task failures must start only after readiness and
preserve the same zero-order evidence.

These fields prove that Reap observed and handled the named condition. They do
not prove the external injector was configured correctly, that Cancel All After
expired after process death, or that exchange state is economically reconciled.
Use the process-death certification above for deadman source `20`, and retain
injector, supervisor, emergency-cancel, and account-statement evidence for the
remaining claims.

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

## Authenticated Endpoint Trust

Live configuration validates the complete REST/public-WebSocket/private-
WebSocket tuple before credentials or network access. The emergency cancel
parser applies the same REST-origin trust policy independently. Every official
endpoint must use HTTPS/WSS, REST must be an origin, WebSockets must use port
8443 and the exact `/ws/v5/public` or `/ws/v5/private` path, and no endpoint may
contain user information, a query, or a fragment. Arbitrary TLS hosts and mixed
regional tuples are rejected to prevent authenticated requests from being sent
outside the reviewed exchange boundary.

The accepted profiles follow the current official OKX guides:

| Registration region | REST origin | Demo WebSocket host | Production WebSocket host |
| --- | --- | --- | --- |
| Global | `openapi.okx.com` or continuing legacy `www.okx.com` | `wspap.okx.com` | `ws.okx.com` |
| US/AU | `us.okx.com` | `wsuspap.okx.com` | `wsus.okx.com` |
| EEA | `eea.okx.com` | `wseeapap.okx.com` | `wseea.okx.com` |
| Turkey | `tr.okx.com` | Not accepted; no demo tuple is documented | `ws.okx.com` |

Sources: [Global API guide](https://www.okx.com/docs-v5/en/),
[US/AU API guide](https://app.okx.com/docs-v5/en/),
[EEA API guide](https://my.okx.com/docs-v5/en/), and
[Turkey API guide](https://tr.okx.com/docs-v5/en/). The
[OKX changelog](https://www.okx.com/docs-v5/log_en/) records
`openapi.okx.com` as the recommended Global REST origin while retaining
`www.okx.com`. Re-review these primary sources before production promotion; an
endpoint change requires a code review and new demo evidence, not a runtime
configuration bypass.

A tuple whose REST and all effective WebSocket hosts are loopback may use
cleartext only with `environment = "demo"` for deterministic tests. Loopback
cannot be mixed with official endpoints and is ineligible for
production-transition evidence.

The live runtime shares one connection-attempt pacer across its public feed and
all account-private feeds. It covers initial connection, reconnect, and book
recovery handshakes. The default 400 ms interval is intentionally stricter than
the documented three-requests-per-second/IP boundary; official profiles reject
less than 334 ms. The readiness timeout includes this serialized startup time.
Multiple processes sharing an IP still require deployment-level coordination.

## Production Configuration Transition

Run the structured transition verifier against the exact demo file used for
evidence and the proposed production file:

```bash
TRANSITION_REPORT="/secure/evidence/production-transition-$(date -u +%Y%m%dT%H%M%SZ).json"
cargo run -p reap-cli -- verify-production-transition \
  --demo-config /secure/evidence/exact-demo.toml \
  --production-config /secure/config/reap-production-candidate.toml \
  --output "$TRANSITION_REPORT" \
  --require-pass \
  --pretty
```

Both inputs must be bounded regular non-symlink files and individually valid.
Live parsing rejects every ignored TOML field, including unknown nested
strategy/risk settings, before comparison so a typo cannot disappear from the
effective values.

The report records their canonical paths, byte counts, SHA-256 hashes, effective
fingerprints, environments, endpoint regions, every changed JSON Pointer, the
pinned Java revision, and the policy result. Output is create-new, mode `0600`
on Unix, and file/directory durable.

Allowed changes are limited to:

- `venue.environment` and the three region-matched endpoint URLs;
- each account's API-key, secret-key, and passphrase environment-variable names;
- `storage.path`;
- `operator.socket_path` and `operator.token_env`; and
- `alerts.endpoint_env` and `alerts.bearer_token_env`.

All strategy economics, risk limits, runtime timing/capacity, account IDs and
mode/routing policy, client-ID policy, VIP-fill behavior, storage durability,
operator/alert behavior, and host guards must be value-identical after typed
parsing. Account array order remains significant. Demo and production endpoint
regions must match; a Global demo file cannot certify an EEA, US/AU, or Turkey
candidate.

Format-3 transition policy also requires both files to enable the operator
service, fatal external alerts, and the production host-guard floors; configure
at least two redundant public connections per subscription and two independent
order-command sessions, use an absolute connection-attempt pacer path, and
declare exact `read_only` plus `trade` API-key permissions with required IP
binding for every account. The
checked-in demo example intentionally leaves external alerts and host guarding
disabled and uses a repository-relative pacer, so it must be copied, deployed,
and exercised before it can be the exact evidence-bearing demo config.

The transition verifier never reads credential values; it proves only the
declared credential policy. The separately required schema-3 account
certification replays the authenticated `perm`, `ip`, UID, and main-UID evidence
for the exact production config. Neither artifact proves deployment-secret
separation, human custody, or runtime behavior. Archive both, but do not treat
them as production authorization. Production order entry remains unavailable
until the separate readiness gates pass.

The transition report and research-deployment report answer different questions.
The transition report proves that the evidence-bearing demo config and proposed
production config differ only at approved deployment paths. The research binding
independently proves that the proposed production config's effective strategy is
the one predeclared and training-selected in every reconstructed research fold.
Both must pass against the same production config bytes.

## Production Evidence Bundle

Do not approve a collection of independent `passed` JSON files by inspection.
After all source evidence exists, make a private copy of
`examples/production-evidence.toml`, replace every placeholder, and run the exact
candidate binary on the declared target host:

```bash
BUNDLE_MANIFEST="/secure/evidence/production-evidence.toml"
BUNDLE_REPORT="/secure/evidence/production-evidence-$(date -u +%Y%m%dT%H%M%SZ).json"
target/release/reap verify-production-evidence \
  --manifest "$BUNDLE_MANIFEST" \
  --output "$BUNDLE_REPORT" \
  --require-pass \
  --pretty
```

Schema 8 requires the intended Reap version, candidate executable SHA-256,
target-host identity SHA-256, predeclared deployment candidate ID, exact
approval-policy SHA-256, and separate demo and production exchange-account
identity maps. It also requires the exact
fault-proxy config and the create-new routed demo config used by the fault
campaign; the latter must be typed-value-identical to a fresh deterministic
reconstruction from the official-endpoint demo config and proxy routes. Its
exact file bytes remain separately bound to every fault run. Paths may be
absolute or relative to the bundle manifest. List exactly one production account
certification, demo deadman certification, authenticated fill reconciliation,
and account-wide economic reconciliation for every account in the exact configs.
Each economic input must reuse that account's exact fill collection and stopped
journal while adding one independently verified bill collection plus distinct
opening and closing account certifications. Boundary gaps are capped at 60
seconds and every boundary collector is bound to the exact demo config, release
binary, target host, and account identity. The dedicated
clean demo soak must be a different session from every fault-matrix run.
Reviewed nonzero `minimum_fills`, `minimum_trade_bills`,
`minimum_derivative_close_bills`, and `minimum_funding_bills` thresholds are
mandatory. Production fill comparisons
must use zero price/quantity/fee tolerances. Economic price tolerance must also
be zero; quantity is capped at `1e-8`, fee at `1e-10`, balance and absolute
trade/funding PnL at `1e-8`, and relative trade/funding PnL at `1e-6`. These hard limits are
ceilings. Funding mark brackets are capped at 2,000 ms per side, with absolute
mark tolerance capped at `1e-8` and relative tolerance at `1e-4`. These are not
recommended defaults; justify every nonzero value from the instrument's
published precision, observed mark cadence, and credentialed samples.
It also requires exactly one raw schema-2 fault-proxy process report for every
matrix scenario. Each must independently pass, use the exact proxy config/build/
host, have a unique proxy session, enclose exactly its assigned live session and
no other matrix session, and report one completed command only when that role has
typed injector evidence.

The mandatory `[freshness]` table declares the accepted age for each
operational source. The verifier records its wall time once, rejects zero age
windows, rejects completed times beyond the configured future tolerance, and
rejects stale sources. Configuration cannot weaken these hard maxima:

| Source | Hard maximum age |
| --- | ---: |
| Dedicated demo soak | 24 hours |
| Every fault live run, typed proxy command, and proxy process report | 7 days |
| Every latency source live run | 7 days |
| Production account certification | 15 minutes |
| Demo deadman certification | 7 days |
| Emergency cancel report | 7 days |
| Reconciled fill window end | 24 hours |
| Reconciled account-bill window end | 24 hours |

Economic opening/closing certifications use the account-bill freshness policy in
addition to their enforced 60-second window brackets. `future_tolerance_ms`
cannot exceed five minutes. The checked-in template uses
stricter one-day fault/latency/deadman/emergency limits and six-hour soak/fill/
bill limits. Tighten them further when the complete campaign can be rerun faster;
do not expand the code-level bounds to make old evidence pass.

Soak, fault, and latency completion times are `session_started_at_ms + elapsed_ms`
from independently reverified live reports. Account and deadman completion times
use the validated final OKX server-clock sample. Emergency completion uses its
validated start plus elapsed interval. Fill and bill age use their authenticated
collection windows' ends, so collecting old retained windows again does not make
them fresh.

The aggregate command directly reruns, rather than consumes output from:

- `verify-production-transition`;
- `verify-research-deployment`, including the full research reconstruction;
- `verify-live-run --expected-mode demo` for the dedicated clean soak;
- routed fault-config reconstruction plus `verify-live-fault-matrix` against that
exact loopback config;
- `verify-fault-proxy-run` for one scenario-matched raw proxy process report per
  matrix role;
- `verify-latency-calibration` from every raw source live report;
- account and deadman artifact verification from their embedded raw responses;
- `verify-emergency-cancel --require-all-configured-accounts`; and
- fill-collection pagination verification plus journal-backed fill/fee
  reconciliation; and
- independent fill and account-bill collection reconstruction plus
  journal-backed normal-trade, derivative close-PnL, and realized-funding
  economic reconciliation.

Every proxy-supported matrix role must expose parsed Reap injector evidence with
the exact supplied proxy-config fingerprint and a unique proxy session and
command ID. Its validated arm/completion interval must be fresh and contained
inside the corresponding reverified live session. Opaque external injector
records are accepted only for genuine partial-fill and restored-latch roles,
whose causality cannot be manufactured by the current proxy. Freshness applies
to those roles' live reports, but their external causality remains an operator
review. Schema 8 directly reconstructs every raw proxy process report and derives
clean shutdown from listener joins, control-socket removal, pending rules, active
connections, and proxy errors. External supervisor lifecycle evidence remains a
separate review.

The controlling manifest plus official-demo, production, routed-fault, and
fault-proxy configs are reopened after those potentially long reconstructions.
The bundle fails if any changed, if the routed config is not the deterministic
typed route transform, if a subordinate source gate fails, if account coverage
is missing or duplicated, or if any config/build/host/account/candidate binding
differs. The output records a semantic SHA-256 of each in-memory reconstruction
and always sets `production_order_entry_authorized = false`.

A pass is deliberately narrower than production approval. Schema 8 enforces
bounded age from validated session, exchange-clock, emergency-report, fill, and
bill window timestamps, but those clocks are not remotely attested. It does not
remotely attest the host, exchange identity, or locally journaled derivative
opening basis. It proves controlled-window bill/cash continuity and independently
checks point-in-time currency equity, but it does not attribute total-equity
movement, prove external supervisor/paging state, authenticate opaque external
fault causality, or by itself record human rollout approval. Re-run immediately
before review and keep production entry disabled until those external controls
are independently signed off.

## Production Approval

Use `examples/production-approval-policy.toml` only as a fail-closed schema
template. The checked-in public-key placeholders are deliberately invalid. The
reviewed deployment policy must contain sorted unique approver IDs, at least two
sorted required roles, one or more approvers for every role, distinct Ed25519
public keys, and a request lifetime no greater than 15 minutes.
Its exact file SHA-256 must be placed in
`expected_approval_policy_sha256` before collecting the passing schema-8 bundle;
request preparation and final verification reject any substituted policy.

Validate the populated policy and archive its exact hash before updating the
production-evidence manifest:

```bash
POLICY_VERIFICATION="/secure/evidence/approval-policy-verification-$(date -u +%Y%m%dT%H%M%SZ).json"
reap verify-production-approval-policy \
  --policy /secure/policy/production-approval.toml \
  --output "$POLICY_VERIFICATION" \
  --pretty
```

Use the report's `policy.sha256` as `expected_approval_policy_sha256`; do not hash
an unvalidated policy or normalize/reformat it after predeclaration.

1. On each independent approval workstation, generate a separate key:

```bash
reap generate-production-approval-key \
  --private-key /secure/approver/private.json \
  --public-key /secure/approver/public.json \
  --pretty
```

Both files are create-new mode `0600`; only the public-key value belongs in the
deployment policy. Do not place approval private keys on the trading host, in a
shared secret manager role used by that host, or in source control. Key identity,
role assignment, revocation, and proof of separate human control remain external
governance responsibilities.

2. On the exact candidate host, after the schema-8 bundle passes, prepare the
short-lived review request:

```bash
APPROVAL_REQUEST="/secure/evidence/approval-request-$(date -u +%Y%m%dT%H%M%SZ).json"
reap prepare-production-approval \
  --manifest /secure/evidence/production-evidence.toml \
  --policy /secure/policy/production-approval.toml \
  --request-id CHANGE-1234 \
  --ttl-secs 600 \
  --output "$APPROVAL_REQUEST" \
  --pretty
```

Preparation reruns every source verifier and refuses a non-passing bundle. The
request binds the exact policy and a typed stable subject containing source
timestamps, freshness decisions and limits, gate hashes, configs, candidate,
binary, host, account identities, and proxy runs. Only verifier wall time and
derived age are omitted so a later rerun can match without weakening freshness.

3. Each approver reviews that exact request and policy on their independent
workstation, then emits one role-bound signature:

```bash
reap sign-production-approval \
  --request "$APPROVAL_REQUEST" \
  --policy /secure/policy/production-approval.toml \
  --private-key /secure/approver/private.json \
  --approver operations-approver \
  --output /secure/evidence/approval-operations.json \
  --pretty
```

The signer rejects an expired or far-future request, a policy mismatch, an
unknown approver, insecure private-key permissions, or a private key that does
not match the policy. Its Ed25519 payload domain-binds the exact request bytes,
policy hash, approver, role, public key, and signing time.

4. Back on the candidate host, run final verification before the request expires:

```bash
APPROVAL_VERIFICATION="/secure/evidence/approval-verification-$(date -u +%Y%m%dT%H%M%SZ).json"
reap verify-production-approval \
  --manifest /secure/evidence/production-evidence.toml \
  --policy /secure/policy/production-approval.toml \
  --request "$APPROVAL_REQUEST" \
  --approval /secure/evidence/approval-operations.json \
  --approval /secure/evidence/approval-risk.json \
  --output "$APPROVAL_VERIFICATION" \
  --require-pass \
  --pretty
```

Final verification reruns the complete production bundle, requires exact stable
subject equality, reopens policy/request/signatures after the expensive work,
verifies every Ed25519 signature, rejects duplicate approvers or keys, and
requires every policy role. It always emits
`production_order_entry_authorized = false`; a pass is an auditable approval gate,
not an order-entry switch. Expiration, source freshness, target-host controls,
venue status, and rollback readiness must all still be valid at actual rollout.
The verifier does not keep a replay ledger; the deployment coordinator must
enforce globally unique change IDs and one-time use of each request.

At pinned Java revision `b6b120c7b7c466d8431bf082f3229328c5d7b2ae`,
`ChaosBackTestMultiRunService` sequences daily inputs, carries ending positions,
and writes per-run files; it does not perform held-out deployment selection or
compose live release evidence. `MetCoinGatewayWsClientsOkexV5Config` constructs
separate public, position, and order websocket clients and dispatch policies, but
does not provide a production evidence decision. The Rust bundle is therefore a
release-safety control around the Java-referenced strategy/connectivity behavior,
not a claim of Java parity.

## Credentials

- Load API key, secret, and passphrase from the deployment secret provider, not
  TOML or source control.
- Credential debug output is redacted. Do not add raw request-header logging.
- Configure `accounts.api_key_policy.expected_permissions` as exactly
  `read_only` and `trade`; never grant `withdraw`. Set
  `require_ip_binding = true`, bind the key to the reviewed target egress, and
  require a passing schema-3 account certification from that host.
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

The `runtime.shutdown_timeout_ms` deadline covers cancel queueing,
reconciliation, and event processing. The separate
`runtime.teardown_timeout_ms` deadline covers the complete host/operator/feed,
order-command, command/reconciliation/safety task, journal, and alert-owner
teardown after that safety phase. Every owner receives shutdown before the
runtime awaits any one owner. A persistence failure is retained as an error but
does not suppress cancel or reconciliation commands. Any unresolved
order/account or secondary teardown failure is included in the returned
lifecycle error and must be treated as an incident. Observe mode performs no
exchange mutation and shuts down directly.

If the teardown deadline expires, cancellation-safe owner destructors abort all
remaining tasks rather than detaching them, remove the local operator socket,
release journal ownership, leave Cancel All After armed, and return stable
`teardown_timeout` failure evidence for the schema-8 report. A normal storage
close drains queued records, flushes, and calls `sync_data` before teardown can
pass. The production evidence policy limits
`shutdown_timeout_ms + teardown_timeout_ms` to 40 seconds under the hardened
45-second systemd stop boundary, leaving five seconds for report serialization,
file/directory sync, and process exit.

With `--output`, the CLI first validates the exact config and run options and
captures source/build/host provenance. Invalid input creates no report. It then
reserves the create-new path before credentials or network startup. A handled
failure while constructing the report-capable runtime writes and fsyncs a
schema-8 pre-session report with `session_id = null`, empty account identities,
baseline readiness, zero runtime evidence, `clean_soak = false`, and
`stop_reason = "runtime_failure"`. Its bounded `failure` explains that zero
counters are not exchange-zero proof. `verify-live-run` can validate this as
diagnostic evidence, but `acceptance_passed` is always false; reconcile exchange
orders/account state before retrying.

If initialization after runtime construction, the event loop, or teardown
fails, Reap completes fail-closed cleanup, writes and fsyncs the full report,
prints it, and returns the original nonzero error. Review
`readiness_at_stop` separately from final `readiness`, and require
`active_orders_after_shutdown = 0`; a failure report is incident evidence and
can never be a clean soak. An empty or incomplete reserved path indicates
report-filesystem failure, forced process death, or a defect. Archive it with
the process log, but never parse or present it as run evidence.

Host, operator, feed, and order owners are signalled together; joins still
collect host/operator evidence before feed/order task evidence. Alert teardown
runs last so runtime teardown failures can still be queued, is independently
bounded by `alerts.shutdown_timeout_ms`, and must fit inside
`runtime.teardown_timeout_ms` when alerts are enabled.

## Bounded Soak Acceptance

Use a bounded run for evidence that can be evaluated without an operator-timed
signal. An observe soak never permits submit or cancel requests:

```bash
mkdir -p var/reap/evidence
OBSERVE_REPORT="var/reap/evidence/observe-$(date -u +%Y%m%dT%H%M%SZ).json"
cargo run -p reap-cli -- live \
  --config examples/live-okx-demo.toml \
  --mode observe \
  --duration-secs 3600 \
  --output "$OBSERVE_REPORT" \
  --require-clean-soak \
  --pretty
cargo run -p reap-cli -- verify-live-run \
  --config examples/live-okx-demo.toml \
  --report "$OBSERVE_REPORT" \
  --expected-mode observe \
  --require-clean-soak \
  --pretty
```

After observe acceptance, run a deliberately short minimal-size demo window
before increasing its duration:

```bash
DEMO_REPORT="var/reap/evidence/demo-$(date -u +%Y%m%dT%H%M%SZ).json"
cargo run -p reap-cli -- live \
  --config examples/live-okx-demo.toml \
  --mode demo \
  --confirm-demo \
  --duration-secs 900 \
  --output "$DEMO_REPORT" \
  --require-clean-soak \
  --pretty
cargo run -p reap-cli -- verify-live-run \
  --config examples/live-okx-demo.toml \
  --report "$DEMO_REPORT" \
  --expected-mode demo \
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

The schema-8 report also records time-to-ready, recovered readiness losses and
maximum outage, total/public/private/order-transport disconnects,
order-transport stale events, stale-stream events, book
recoveries, and the storage queue high-water mark. The total disconnect count
must equal the public, private, and order-transport counts combined. It also reports authenticated
operator commands and mutations, ambiguous submit/cancel outcomes, partial-fill
transitions, order/fill convergence timeouts, and restored durable latches.
Restored startup order state does not increment per-session partial-fill or
ambiguity counters. Deadman heartbeat, periodic exchange-clock skew/check,
exchange-status block/check, authenticated instrument drift/check, authenticated
exchange-fee drift/check, and authenticated account-config drift/check failures
use distinct stable failure codes; do not classify them by matching
`failure.message`.
When enabled, it includes host preflight/last snapshots and check count plus
alert delivery and queue evidence. A runtime/teardown failure additionally
records its stable code and bounded message after cleanup while retaining a
nonzero command exit.
Recovered disconnects do not by themselves fail acceptance,
but their counts must match the injected fault plan. In demo mode the final
`readiness` may be degraded by the deliberate shutdown kill switch; acceptance
uses the pre-shutdown `readiness_at_stop` snapshot.

A `clean_soak` result is evidence for this bounded runtime window only. Review
the JSONL log, account balances, positions, fills, and checkpoint restart before
checking off the sustained demo gate. After a demo fill window, run the offline
fill/fee statement reconciliation above; `clean_soak` does not perform that
economic comparison and does not prove optional host/alert controls were
deployed when they are disabled.

## Live Latency Evidence

`--output` reserves an owner-only versioned JSON report before config,
credentials, or network access and syncs both file and parent directory before
enforcing `--require-clean-soak`; existing paths are refused. The report binds
the exact source-config byte count/SHA-256, a unique session, start time, Reap
version and executable SHA-256, pinned Java revision, pseudonymous
machine/account identities, checkpoint identity, and a second fingerprint over
every serialized live setting, including host and alert guards. Raw machine IDs
and OKX user IDs are not emitted. `verify-live-run` treats both supplied files as
untrusted, checks exact/effective config identity and structural evidence, and
re-derives clean-soak acceptance. The report also
retains a deterministic uniform reservoir of at most 8,192 samples for each
class/symbol/semantics series. Archive the exact live TOML, binary, and original
reports; the calibration artifact retains their hashes but is not a replacement
for those source files.

The measurements map to the backtest scheduler as follows:

| Class | Live boundary | Calibration constraint |
| --- | --- | --- |
| `market_depth` | accepted websocket host receive to entry into the strategy coordinator | Raw replay already starts at host receive, so exchange-to-host time is deliberately excluded |
| `historical_trade` | accepted websocket host receive to strategy visibility | Same local visibility boundary as depth |
| `reference_data` | accepted index/funding/mark/limit input at host receive to strategy visibility | Rust class with no direct Java `BackTestDelay` member |
| `matching_new` | event-loop dispatch through storage, account queue, pacing, websocket write, and successful place acknowledgement | Demo only; conservative upper bound for Java `MatchingNew`, requiring explicit acceptance |
| `matching_cancel` | event-loop dispatch through storage, account queue, pacing, websocket write, and successful cancel acknowledgement | Demo only; acknowledgement does not prove terminal cancellation and is an upper bound for Java `MatchingCancel` |
| `order_update` | OKX exchange event timestamp to canonical strategy visibility | Demo only; cross-clock measurement requires synchronized host-guard snapshots |
| `order_fill` | canonical fill visibility to the covering derivative position or both spot balances | Demo only; zero is valid when covering state arrived first |

Enable `[host_guard]` on the target host before collecting calibration reports.
Calibration rejects a validation report, a non-clean or non-ready bounded run,
different full config fingerprints, duplicate sessions/series, wrong schemas or
Java revision, mixed binaries/hosts/accounts, unsynchronized preflight/final
clocks, runtime failure evidence, dropped evidence, rejected clock samples,
over-limit samples, malformed reservoirs, or any failed measured exchange
operation. Every configured
instrument needs depth, trade, new,
cancel, order-update, and fill series; derivative/index/stablecoin reference
symbols also need reference-data series. Private classes are accepted only from
demo runs. The default minimum is 1,000 valid observations for every required
series, not 1,000 observations in aggregate.

An authoritative REST recovery clears unresolved fill-convergence tracking but
does not manufacture a websocket latency sample. The report counts each such
observation as dropped, so it cannot enter a passing calibration with a
right-censored slow tail.

After multiple representative bounded runs using the same exact config:

```bash
CALIBRATION="var/reap/evidence/latency-$(date -u +%Y%m%dT%H%M%SZ).json"
PROFILE="var/reap/evidence/latency-profile-$(date -u +%Y%m%dT%H%M%SZ).toml"
VERIFICATION="var/reap/evidence/latency-verification-$(date -u +%Y%m%dT%H%M%SZ).json"
cargo run -p reap-cli -- calibrate-latency \
  --config examples/live-okx-demo.toml \
  --report "$OBSERVE_REPORT" \
  --report "$DEMO_REPORT" \
  --output "$CALIBRATION" \
  --profile-output "$PROFILE" \
  --minimum-samples-per-series 1000 \
  --accept-matching-upper-bounds \
  --require-pass \
  --pretty
cargo run -p reap-cli -- verify-latency-calibration \
  --config examples/live-okx-demo.toml \
  --artifact "$CALIBRATION" \
  --report "$OBSERVE_REPORT" \
  --report "$DEMO_REPORT" \
  --output "$VERIFICATION" \
  --require-pass \
  --pretty
```

The generator merges complete samples exactly and otherwise produces a bounded,
population-weighted deterministic quantile approximation. Nanoseconds are
rounded up to microseconds during collection and microseconds are rounded up to
backtest milliseconds. A failed calibration still writes its diagnostic JSON,
but the CLI refuses to emit a TOML profile from it.

`verify-latency-calibration` treats the schema-4 artifact and source reports as
untrusted bounded regular files, rejects symlinks, path/content reuse, missing or
unexpected report hashes, and config drift, then reruns independent live-report
verification and reconstructs the calibration using its recorded seed, sample
minimum, and acknowledgement-upper-bound decision. It compares the complete
rebuild after replacing only source paths with their SHA-256 identities, so an
archived report may move but its bytes cannot change. The optional verification
output is an owner-only create-new file synced with its parent directory. Archive
the exact config, calibration, every source report, and this passing verification
artifact together; the calibration's internal `passed` flag is insufficient.

A production-candidate research manifest must use schema 8, predeclare its
`deployment_candidate_id`, set
`latency_calibration` to the JSON artifact, set the baseline execution
`calibrated = true`, and embed exactly the artifact's profile. Research treats
the artifact as untrusted input: it checks source/config hashes, sessions,
binary/host/account identity, class semantics, demo provenance for private
classes, sample counts, matching upper-bound acceptance, and exact
series-to-profile equality. The research executable must be byte-identical to
the one that collected the live evidence, and the artifact hash is rechecked
after all runs. Require a separate passing reconstruction artifact before
approving the research manifest. Stress profiles must still conservatively
dominate that baseline. No credentialed latency report, passing calibration, or
passing reconstruction artifact has been recorded yet.
