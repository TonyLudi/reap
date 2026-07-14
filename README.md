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
- OKX public/private parsers, expiring HMAC-signed REST order requests,
  supervised multi-websocket feeds, account-scoped channel-aware
  deduplication, lossless bounded ready/disconnect transitions, sequence
  recovery, and full order/fill/balance/position REST reconciliation with
  authoritative stale-state repair.
- Deterministic pre/post-trade risk, stale-stream fail-closed behavior, global,
  account, and symbol halt events, redundant USDT/USDC reference guards with
  durable depeg latching, submit/cancel order-state and per-fill account-state
  convergence deadlines, global/per-symbol active-order count ceilings, rolling
  exchange-order rejection and unfilled-IOC cancellation circuits, strict
  configured-account position and private-order ownership, restart-recovered
  one-to-one exchange/client order identity, timestamp-independent private
  fill/terminal deduplication, continuous derivative position margin-mode
  enforcement, forced-repayment risk blocking, and an event-loop enforcement
  layer that promotes terminal strategy safety halts into durable global risk
  latches.
- Bounded structured telemetry and JSONL storage for raw, normalized, intent,
  request, acknowledgement, order, fill, system, bootstrap, reconciliation,
  and write-ahead safety-latch records, including restart recovery and an
  exclusive canonical journal lease. Schema-6 live evidence classifies
  ambiguous operations, partial fills, convergence timeouts, restored latches,
  and typed safety-task failures.
- A fail-closed `reap-live` composition root with account-scoped REST bootstrap,
  exchange metadata/account-mode and zero-liability verification, redundant public sockets,
  isolated private sockets, account/positions data-round health, one strategy
  owner, prioritized gateway tasks, and graceful cancel-and-drain shutdown.
  Demo entry also validates exchange time, continuously detects authenticated
  account-configuration drift, polls announced OKX unified-account maintenance
  with the pinned Java service filter and lead time, continuously verifies
  strategy-critical instrument rules, hard single-order maxima, and configured
  fees against authenticated current OKX metadata, enforces the authenticated
  limit-order quantity/amount again before dispatch, and maintains OKX Cancel
  All After from an independent safety task.
- Authenticated read-only cash-account certification that embeds exact bounded
  OKX config/balance/position responses in a create-new mode-`0600` artifact,
  binds config/binary/host/Java/account provenance, and supports credential-free
  offline re-verification without printing sensitive raw account state.
- Read-only process-death certification that exclusively leases the stopped
  journal, binds recovered exchange/client order identities to exact OKX order
  details, requires Cancel All After source `20` and account-wide regular-order
  zero, and supports credential-free verification against the exact journal.
- Bounded asynchronous HTTPS webhook alerts and optional Linux journal-disk,
  available-memory, and kernel-clock guards, with preflight evidence and
  fail-closed periodic enforcement outside the strategy loop.
- A strategy-independent OKX emergency command that arms account-wide Cancel All
  After, batch-cancels regular orders on every symbol, and requires a post-trigger
  zero-order proof. Its create-new schema-versioned artifact binds the exact
  input file, binary, host, Java revision, account coverage, and task failures,
  plus hardened systemd templates with mode-specific restart policy.
- A loopback-only OKX demo fault proxy with separate REST, public, private, and
  order-command routes, owner-local control, deterministic disconnect/frame-drop/
  REST-response faults, and create-new typed injector evidence that never records
  credentials or raw private payloads.
- Deterministic backtest matching with `PendingNew`, delayed entry/cancel/update
  boundaries, `PostOnly`, `IOC`, conservative displayed-depth fills, trade
  fills, queue-ahead tracking, fee/turnover attribution, realized linear and
  inverse funding at exchange timestamps, receive-time raw replay, and
  mark-to-market accounting.
- CSV/normalized replay, raw-capture validation, configuration validation, and
  a release-mode hot-path benchmark.
- Credential-free public OKX capture with redundant websocket plans, raw-frame
  durability, a create-new run report, exact file/config provenance,
  report-aware raw/normalized verification, bounded-memory capture analysis,
  normalized diagnostic output, and direct raw-capture backtests.

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

Capture backtest-ready OKX public data, including redundant USDT/USD and
USDC/USD risk references, without credentials or private/account connections.
The bounded command exits non-zero on parse, sequence, recovery, writer, or
end-of-run connectivity defects. Capture configuration rejects unknown fields
instead of silently defaulting a typo:

```bash
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

The one-hour command is an operational smoke, not funding-complete research
evidence. Production datasets must be continuous across exchange funding
boundaries. Schema-5 production research with swap candidates requires nonzero
realized funding settlements in both training and test aggregates, so short
captures cannot pass by carrying only forecasts.

Validate and backtest the raw capture directly. Raw replay runs the same OKX
adapter, redundant-feed deduplicator, sequence tracker, and book reducer used
by live trading. Use a new output path for each capture process; replay rejects
concatenated session IDs rather than treating downtime as continuous data.
Capture refuses an existing report, raw, or normalized path instead of
overwriting or appending. Verify the durable run report before standalone
analysis or strategy replay:

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

The optional `[backtest]` table controls market-data, order-entry, cancel,
order-update, and fill/account delays plus the displayed-depth over-cross
threshold, queue-ahead multiplier, historical-trade participation, and
displayed-depth capacity. Explicit `[[backtest.currency_rates]]` entries map
every named non-USD accounting currency to a direct currency/USD index and
freshness budget. A bounded `latency_profile` can replace each scalar fallback
with deterministic empirical samples by Java-mapped message class and symbol.
The report embeds those assumptions, actual sampled-latency usage, the replay
time basis, clock regressions, currency ledgers/rates, live orders, and work
still scheduled at the capture boundary. The example delay values are zero,
the threshold is the pinned Java default, capacity fractions are 100%, and
`calibrated = false`; these parity defaults are not an execution-quality or
profitability claim.

Backtest order entry follows the live startup boundary: new orders remain
blocked until every configured matching book and direct accounting-currency
rate is available and fresh. Cancels remain permitted. Reports retain the
first ready timestamp, terminal readiness, and the number of pre-ready new
orders suppressed.

Backtest reports also separate fee cost, funding PnL, turnover, and raw currency
cash. Exact private-fill fees retain their signed exchange amount and currency;
public-data simulated fills remain explicitly counted rate estimates. Spot,
derivative, funding, fee, and active-order exposure use fresh latency-delivered
currency indexes; missing/stale rates or conversions attempted before a rate is
usable make accounting incomplete. Reports also flag late, missed, invalid, or
failed funding accounting and retain funding settlements beyond the data
horizon as pending actions. The model assumes a zero initial portfolio and is
not an exchange-statement substitute.

Run the checked-in walk-forward plumbing fixture:

```bash
cargo run -p reap-cli -- research \
  --manifest examples/research-smoke.toml \
  --output /tmp/reap-research-smoke.json --require-pass --pretty
cargo run -p reap-cli -- verify-research \
  --manifest examples/research-smoke.toml \
  --report /tmp/reap-research-smoke.json \
  --output /tmp/reap-research-smoke-verification.json \
  --require-pass --pretty
```

`research` selects candidate configuration only from each fold's training
datasets, then runs the selected candidate on later test data under baseline
and conservative stress scenarios. Reports embed the selection rule and gate
thresholds, fingerprint the manifest, binary, candidate files, effective
strategies, and datasets, and include drawdown, position/pending delta, gross
position and active-order exposure, inventory duration, fee/funding accounting,
and pending-work gates. Manifest, executable, candidate, dataset, capture
configuration, capture-report, and optional normalized-artifact hashes are
verified again after all runs. Production raw datasets must pass the embedded
schema-3 run-report verifier, capture-config-bound source analysis, and an
independent zero-gap replay-integrity check before any candidate executes. The
smoke fixture intentionally uses permissive uncalibrated gates and is not
trading evidence. Production-candidate manifests use schema 5, predeclare one
`deployment_candidate_id`, and fail unless every fold independently selects
that candidate from training data. They also require nonzero training/test
realized-funding-settlement gates when any candidate trades a swap and a passed
`latency_calibration` artifact, and require the baseline's empirical latency
profile to match that artifact exactly.

`verify-research` treats the archived JSON as untrusted, binds it to the exact
manifest and byte-identical executable, re-runs every candidate/dataset/scenario
and fold, and compares the complete semantic report. JSON formatting and
verifier-observed archive paths may differ; fields, floating-point results,
manifest-declared paths, and all content hashes may not. Both generator and
verification outputs are owner-only create-new files with file and directory
durability. A report's internal `passed` flag is not release evidence without a
passing reconstruction.

Bind that reconstructed deployment candidate to the exact proposed production
live configuration before promotion:

```bash
cargo run -p reap-cli -- verify-research-deployment \
  --config /secure/config/reap-production-candidate.toml \
  --manifest /secure/research/production.toml \
  --report /secure/research/production.json \
  --output /secure/evidence/research-deployment.json \
  --require-pass --pretty
```

This command requires a production venue config, re-runs `verify-research`,
revalidates the schema-5 candidate binding, and requires the candidate and live
config to serialize to the same effective strategy SHA-256. It does not enable
order entry or replace transition, account, host, fault, fee, funding, statement,
deadman, and emergency evidence.

Validate the live demo configuration without reading credentials or opening a
network connection:

```bash
rm -f /tmp/reap-live-validate.json
cargo run -p reap-cli -- live \
  --config examples/live-okx-demo.toml \
  --mode validate \
  --output /tmp/reap-live-validate.json \
  --pretty
cargo run -p reap-cli -- verify-live-run \
  --config examples/live-okx-demo.toml \
  --report /tmp/reap-live-validate.json \
  --expected-mode validate \
  --require-valid \
  --pretty
```

Authenticated live and emergency configuration accepts only documented,
region-consistent OKX REST and WebSocket endpoint tuples. Production requires
HTTPS/WSS; URL user information, alternate paths, query strings, fragments,
unexpected ports, mixed regions, and arbitrary TLS hosts are rejected. A full
loopback tuple is accepted only in the demo environment for deterministic local
tests. Unknown live TOML fields are rejected, including nested strategy and
risk typos, rather than being silently dropped before validation.

`runtime.connection_attempt_interval_ms = 400` serializes initial and
reconnecting WebSocket handshakes across every public/private feed and
authenticated order-command session in one process. Official endpoint
configurations reject values below 334 ms because
OKX documents a limit of three WebSocket connection requests per second per IP
in the [API guide](https://www.okx.com/docs-v5/en/). Only complete demo
loopback configurations may use zero. Deployments running multiple Reap
processes behind one egress IP must coordinate that limit outside the process.

Before promoting an exact demo configuration to a production candidate, create
an owner-only transition artifact:

```bash
TRANSITION_REPORT="/secure/evidence/production-transition-$(date -u +%Y%m%dT%H%M%SZ).json"
cargo run -p reap-cli -- verify-production-transition \
  --demo-config /secure/evidence/exact-demo.toml \
  --production-config /secure/config/reap-production-candidate.toml \
  --output "$TRANSITION_REPORT" \
  --require-pass \
  --pretty
```

The verifier hashes both exact files and permits changes only to the documented
environment/endpoints, credential environment-variable names, journal/socket
paths, and operator/alert secret bindings. Strategy, risk, runtime, account
policy, execution, storage capacity, and safety settings must remain identical,
and both files must use the same official endpoint region. A pass does not read
credential values, authorize production, or enable production order entry.

Observe OKX demo feeds and account state without permitting any submit or
cancel request:

```bash
export REAP_OKX_API_KEY=...
export REAP_OKX_SECRET_KEY=...
export REAP_OKX_PASSPHRASE=...
export REAP_OPERATOR_TOKEN=... # at least 32 bytes from the secret provider
cargo run -p reap-cli -- certify-account \
  --config examples/live-okx-demo.toml \
  --account main \
  --output /secure/evidence/account-certification.json \
  --pretty
cargo run -p reap-cli -- verify-account-certification \
  --artifact /secure/evidence/account-certification.json \
  --require-pass \
  --pretty
cargo run -p reap-cli -- live --config examples/live-okx-demo.toml --mode observe
```

The certification command has no order-entry path. It requires disabled
mode-appropriate borrowing flags, zero applicable aggregate/per-currency
liability, interest, and borrow-frozen fields, no OKX `MARGIN` positions, zero
configured strategy borrow limits, stable bracketed identity/settings, and
bounded exchange-clock evidence. The artifact contains sensitive raw account
responses and embedded config, so keep it in restricted evidence storage. It is
sequential point-in-time evidence, not an atomic snapshot, historical borrowing
proof, or full statement reconciliation; quiesce account activity while it runs.

From another shell with the same operator token, inspect or stop the runtime:

```bash
cargo run -p reap-cli -- operator --config examples/live-okx-demo.toml status --pretty
cargo run -p reap-cli -- operator --config examples/live-okx-demo.toml kill-account --account main --reason "unexpected account exposure"
cargo run -p reap-cli -- operator --config examples/live-okx-demo.toml shutdown --reason "planned stop"
```

After stopping every order producer for an account, the independent emergency
path can cancel and verify all regular pending orders without loading strategy or
journal state. It intentionally excludes OKX algo and spread orders:

```bash
EMERGENCY_REPORT="/tmp/reap-emergency-$(date -u +%Y%m%dT%H%M%SZ).json"
cargo run -p reap-cli -- emergency-cancel \
  --config examples/live-okx-demo.toml \
  --account main \
  --confirm-account-wide-cancel \
  --confirm-order-producers-stopped \
  --output "$EMERGENCY_REPORT" \
  --pretty

EMERGENCY_VERIFICATION="/tmp/reap-emergency-verification-$(date -u +%Y%m%dT%H%M%SZ).json"
cargo run -p reap-cli -- verify-emergency-cancel \
  --config examples/live-okx-demo.toml \
  --report "$EMERGENCY_REPORT" \
  --output "$EMERGENCY_VERIFICATION" \
  --require-all-configured-accounts \
  --require-pass \
  --pretty
```

The output path is reserved before parsing credentials or starting REST work.
`all_clear = true` requires both `regular_orders_all_clear = true` and complete
config/binary/host/exchange-account/task evidence; early configuration failures
leave an empty reserved path, which is never a report. Both report paths are
owner-only and synced with their parent directory. Offline verification hashes
the exact config/report bytes and re-derives account coverage, trigger-horizon,
final-zero, provenance, and completion invariants. It cannot replay raw exchange
responses or prove the operator stopped every external order producer, and the
regular-order scope still excludes OKX algo and spread orders.

For a controlled minimal-size demo process-kill campaign, wait for the already
armed deadman to expire without issuing another cancel, then collect and verify
the stronger causal evidence before restarting against the journal:

```bash
DEADMAN_REPORT="/tmp/reap-deadman-$(date -u +%Y%m%dT%H%M%SZ).json"
cargo run -p reap-cli -- certify-deadman-expiry \
  --config examples/live-okx-demo.toml \
  --account main \
  --confirm-order-producers-stopped \
  --output "$DEADMAN_REPORT" \
  --pretty
cargo run -p reap-cli -- verify-deadman-certification \
  --artifact "$DEADMAN_REPORT" \
  --journal var/reap/live-events.jsonl \
  --require-pass \
  --pretty
```

This path sends only GET requests and requires every durably recovered live
regular order to be `canceled` with OKX `cancelSource = "20"`. It is for a
planned demo test, not a reason to postpone emergency cancellation during an
incident. See [docs/operations.md](docs/operations.md) for the complete
procedure and evidence limitations.

Run a bounded observe soak and return a non-zero status unless the runtime
reaches readiness, finishes the requested window, records no reconciliation
drift, storage drops, or alert delivery failures, and shuts down with no active
orders:

```bash
OBSERVE_REPORT="/tmp/reap-live-observe-$(date -u +%Y%m%dT%H%M%SZ).json"
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

Enable demo order entry only with the explicit confirmation flag and a bounded,
minimal-size configuration:

```bash
DEMO_REPORT="/tmp/reap-live-demo-$(date -u +%Y%m%dT%H%M%SZ).json"
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

After the bounded demo is stopped and the window has been closed for at least 60
seconds, collect authenticated recent-fill evidence and reconcile the canonical
journal's fills and exact signed fees offline:

```bash
BEGIN_MS=1783987200000
END_MS=1783990800000
FILL_EVIDENCE="/secure/evidence/okx-fills-$(date -u +%Y%m%dT%H%M%SZ)"
FILL_REPORT="/tmp/reap-fill-reconciliation-$(date -u +%Y%m%dT%H%M%SZ).json"
cargo run -p reap-cli -- collect-fills \
  --config examples/live-okx-demo.toml \
  --account main \
  --begin-ms "$BEGIN_MS" \
  --end-ms "$END_MS" \
  --output "$FILL_EVIDENCE" \
  --pretty

cargo run -p reap-cli -- reconcile-fills \
  --journal var/reap/live-events.jsonl \
  --collection-manifest "$FILL_EVIDENCE/manifest.json" \
  --account main \
  --begin-ms "$BEGIN_MS" \
  --end-ms "$END_MS" \
  --minimum-fills 10 \
  --output "$FILL_REPORT" \
  --require-pass \
  --pretty
```

`collect-fills` performs authenticated read-only REST requests and cannot submit
orders. It samples exchange time and account identity before and after paging,
uses the current recent `/api/v5/trade/fills` endpoint, paces 100-row pages at
least 200 ms apart, and requires a short terminal page within a fail-closed
bound. The verifier re-hashes the exact config, manifest, and response bytes and
replays the cursor chain. Reconciliation then takes the exclusive journal lease,
requires the journal and collection config fingerprints to match, compares
strict `(symbol, tradeId)` identities, and refuses duplicate, missing,
malformed, fee-less, or field-mismatched fills. Older `fills-history` exports can
still be supplied manually with repeated `--statement` and the explicit
`--confirm-statement-account-and-window-complete` attestation, but that coverage
is weaker. The report covers fills and fees only; it is not balance, position,
funding, equity, liability, tax, or currency-conversion reconciliation.

Each create-new schema-8 live report contains the exact source-config byte
count/SHA-256, checkpoint and full evidence config fingerprints, Reap executable
hash, pinned Java revision, pseudonymous host/account identity,
session/readiness/host evidence, and bounded per-class/per-symbol latency
samples. The CLI reserves `--output` mode `0600` before config, credential, or
network work, then syncs the report and parent directory. Once the
report-capable runtime is constructed, an
initialization, event-loop, or teardown failure still writes a schema-versioned
report after fail-closed cleanup, with a bounded stable failure code/message,
and then exits non-zero. A failure before runtime construction leaves the
reserved file empty and must be diagnosed from the process log; an empty file
is never evidence. `verify-live-run` independently re-hashes exact config/report
bytes, re-derives effective fingerprints and clean-soak acceptance, and checks
mode, identity, readiness, failure, disconnect, host, and latency invariants.
The demo-only fault proxy can produce a validated loopback live config and run
outside the strategy process:

```bash
RUN_ID="$(date -u +%Y%m%dT%H%M%SZ)"
install -d -m 700 var/reap/fault
cargo run -p reap-cli -- render-fault-live-config \
  --live-config examples/live-okx-demo.toml \
  --proxy-config examples/okx-demo-fault-proxy.toml \
  --output "var/reap/fault/live-${RUN_ID}.toml" \
  --pretty
cargo run -p reap-cli -- fault-proxy \
  --config examples/okx-demo-fault-proxy.toml \
  --output "var/reap/fault/proxy-${RUN_ID}.json" \
  --duration-secs 3600 \
  --require-clean-shutdown \
  --pretty
```

Use `fault-proxy-control` from a separate process with one checked-in
`examples/faults/*.json` command at a time. Copy each fault template to the
campaign directory and give it a unique `command_id` and `evidence_file`; the
proxy refuses reused IDs, existing evidence, non-loopback listeners, and any
non-demo upstream. See `docs/operations.md` for the isolated campaign sequence.

After isolated target-host fault runs, populate
`examples/live-fault-matrix.toml` and verify that every role used one exact
config, binary, host, account identity, and unique session:

```bash
FAULT_MATRIX_REPORT="/tmp/reap-live-fault-matrix-$(date -u +%Y%m%dT%H%M%SZ).json"
cargo run -p reap-cli -- verify-live-fault-matrix \
  --config /etc/reap/live/btc-demo.toml \
  --manifest /var/lib/reap/live/btc-demo/live-fault-matrix.toml \
  --output "$FAULT_MATRIX_REPORT" \
  --require-pass \
  --pretty
```

The matrix requires enabled fatal alerts, the synchronized host guard, the
operator service, redundant public websockets, clean observe/demo/reconnect
runs, safe zero-order shutdown for injected ambiguity/convergence faults, and
the documented typed runtime failures. It hashes every injector record; strict
Reap proxy evidence is also structurally validated and tied to every
proxy-expressible role. Proxy evidence is rejected for genuine partial fills and
restart-latch proof, while records from external injectors remain opaque.
Process-death, deadman-expiry, emergency-cancel, fill/fee, target-host, and
production approval remain separate gates.

Calibration emits schema-4 artifacts and admits only independently verified,
clean target-host reports with the same exact config and binary:

```bash
CALIBRATION="/tmp/reap-latency-calibration-$(date -u +%Y%m%dT%H%M%SZ).json"
PROFILE="/tmp/reap-latency-profile-$(date -u +%Y%m%dT%H%M%SZ).toml"
cargo run -p reap-cli -- calibrate-latency \
  --config examples/live-okx-demo.toml \
  --report "$OBSERVE_REPORT" \
  --report "$DEMO_REPORT" \
  --output "$CALIBRATION" \
  --profile-output "$PROFILE" \
  --accept-matching-upper-bounds \
  --require-pass \
  --pretty
```

Independently rebuild the archived calibration before using it in production
research. Supply every source report; files may move after archival because the
verifier binds their bytes and normalizes provenance paths before comparison:

```bash
CALIBRATION_VERIFICATION="/tmp/reap-latency-verification-$(date -u +%Y%m%dT%H%M%SZ).json"
cargo run -p reap-cli -- verify-latency-calibration \
  --config examples/live-okx-demo.toml \
  --artifact "$CALIBRATION" \
  --report "$OBSERVE_REPORT" \
  --report "$DEMO_REPORT" \
  --output "$CALIBRATION_VERIFICATION" \
  --require-pass \
  --pretty
```

Public data classes measure host receive to strategy visibility. Private order
updates use synchronized exchange time, fills measure canonical fill to all
required account-state rows, and matching new/cancel samples include local
queueing, pacing, REST transport, and successful acknowledgement. The latter
are conservative upper bounds, not direct OKX matching-engine latency. Failed
operations, clock defects, dropped evidence, missing expected classes, or fewer
than 1,000 valid samples per required series prevent profile output.

Run tests:

```bash
cargo fmt --all -- --check
cargo clippy --workspace --all-targets --locked -- -D warnings
cargo test --workspace --locked --no-fail-fast
cargo build --release --locked -p reap-cli
cargo audit --deny warnings
```

The repository pins Rust `1.95.0`. GitHub CI runs every command above with a
read-only token and a full-SHA-pinned checkout action; `cargo-audit` is pinned to
`0.22.2`. Dependabot proposes weekly Cargo and Actions updates for review. A
green repository gate is required but does not replace the credentialed demo,
target-host, or exchange/account evidence below.

Profile the deterministic event loop:

```bash
cargo bench -p reap-engine --bench event_loop
cargo bench -p reap-live --bench live_loop
```

`demo` mode rejects a production exchange configuration. `observe` is strictly
read-only. Production order entry is intentionally not exposed: passing real
account certification and fill reconciliation, a credentialed OKX demo soak,
fault campaign, calibrated research, target-host exercises, and operator
rollout approval remain required before production capital.

Design docs:

- [docs/architecture.md](docs/architecture.md) describes the target HFT-style
  event-loop architecture, module split, websocket/dedup design, and migration
  plan.
- [docs/chaos-mapping.md](docs/chaos-mapping.md) maps the Java `chaos` logic to
  Rust modules and lists remaining strategy-model scope limits.
- [docs/operations.md](docs/operations.md) defines startup, fail-closed, recovery,
  supervision, emergency cancellation, and credential procedures.
- [docs/trading-readiness.md](docs/trading-readiness.md) lists the exact gap from
  the current libraries to demo and production trading.
- [docs/performance.md](docs/performance.md) records the strategy and complete
  live-parity benchmarks, allocation profile, and measured optimizations.
