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
  deduplication, exact subscription-argument acknowledgement readiness,
  lossless bounded ready/disconnect transitions, sequence recovery, and full
  order/fill/balance/position REST reconciliation with authoritative stale-state
  repair.
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
  request, acknowledgement, order, fill, system, bootstrap, runtime-session,
  reconciliation, and write-ahead safety-latch records, including restart
  recovery and an exclusive canonical journal lease. Schema-8 live evidence
  classifies ambiguous operations, partial fills, convergence timeouts, restored
  latches, and typed safety-task failures.
- A fail-closed `reap-live` composition root with account-scoped REST bootstrap,
  exchange metadata/account-mode and zero-liability verification, exact
  plan-derived public replicas, one packed private-state socket per account,
  account/positions data-round health, in Demo one nonempty command lane per
  executing account, one strategy owner, prioritized gateway tasks, and
  graceful cancel-and-drain shutdown.
  Demo entry also validates exchange time, continuously detects authenticated
  account-configuration drift, polls only plan-relevant OKX unified-account
  maintenance with the configured lead time, continuously verifies
  strategy-critical instrument rules, hard single-order maxima, and configured
  fees against authenticated current OKX metadata, enforces the authenticated
  limit-order quantity/amount again before dispatch, and maintains OKX Cancel
  All After from an independent safety task. A per-account read-only sentinel
  continuously proves all seven pending algo families and pending spread orders
  empty before and during readiness; stale, nonzero, or unverifiable proof
  blocks placement and starts canonical owned-regular cancellation and
  reconciliation.
- Authenticated read-only cash-account certification that embeds exact bounded
  OKX config/balance/position and direct public currency-index responses in a
  create-new mode-`0600` artifact, binds config/binary/host/Java/account
  provenance, independently rebuilds per-currency USD equity, and supports
  credential-free offline re-verification without printing sensitive raw state.
- Authenticated account-wide OKX bill collection with exact raw-page retention,
  independent cursor/window reconstruction, and stopped-journal reconciliation
  of normal trades, derivative close PnL, and realized linear/inverse funding.
  Authoritative REST `avgPx` snapshots and exact critical fills reconstruct the
  pinned Java linear/inverse position basis. Tightly bracketed account
  certifications prove bill-by-bill cash continuity; unknown account bills fail
  closed.
- Read-only process-death certification that exclusively leases the stopped
  journal, binds recovered exchange/client order identities to exact OKX order
  details, requires Cancel All After source `20` and account-wide regular-order
  zero, and supports credential-free verification against the exact journal.
- Bounded asynchronous HTTPS webhook alerts and optional Linux journal-disk,
  available-memory, and kernel-clock guards, with preflight evidence and
  fail-closed periodic enforcement outside the strategy loop.
- A strategy-independent OKX emergency command that arms regular and spread
  Cancel All After and independently paginates and cancels regular, algo, and
  spread orders. Regular mitigation is kicked off first; each domain then owns
  its pacing, progress, and final-zero proof while independently enforcing the
  shared absolute per-account deadline. The existing report is merged in
  deterministic regular/algo/spread order. Its create-new schema-versioned
  artifact binds the exact
  input file, binary, host, Java revision, account coverage, and task failures,
  plus CI-verified hardened systemd templates with mode-specific restart policy,
  no capabilities, and a bounded offline security exposure.
- A loopback-only OKX demo fault proxy with separate REST, public, private, and
  order-command routes, owner-local control, deterministic disconnect/frame-drop/
  REST-response faults, and create-new typed injector evidence that never records
  credentials or raw private payloads. Schema-2 process reports bind the exact
  config, binary, host, session, timing, listener cleanup, and final proxy state;
  an offline verifier independently re-derives clean shutdown.
- A strict production-evidence bundle verifier that reruns every source verifier
  and cross-binds exact official-demo/production configs, a deterministically
  derived routed fault config, the predeclared research candidate, the running
  release binary, target host, and separate demo and production account
  identities plus the predeclared approval-policy hash. Mandatory bounded
  source-time checks reject invalid, future, or
  stale operational evidence. A separate Ed25519 release-approval flow requires
  short-lived signatures from at least two distinct policy roles and reruns the
  complete bundle before accepting them. Both reports always leave production
  entry unauthorized; external target-host operations remain blockers.
- Deterministic backtest matching with `PendingNew`, delayed entry/cancel/update
  boundaries, `PostOnly`, `IOC`, conservative displayed-depth fills, trade
  fills, queue-ahead tracking, fee/turnover attribution, realized linear and
  inverse funding at exchange timestamps, receive-time raw replay, and
  mark-to-market accounting.
- CSV/normalized replay, raw-capture validation, configuration validation, and
  a release-mode hot-path benchmark.
- Credential-free public OKX capture with redundant websocket plans, raw-frame
  durability, fail-closed Linux disk/memory/clock checks, a create-new run
  report bound to the exact binary, host, pinned Java revision, and config,
  report-aware raw/normalized verification, a process-global persisted-frame
  ordinal, bounded-memory capture analysis, normalized diagnostic output, and
  direct raw-capture backtests.

The Phase 0–5 Goal A implementation enforces the
[Chaos connectivity boundary](docs/chaos-connectivity-boundary.md) against the
clean sibling `../imm-strategy` checkout pinned at
`b6b120c7b7c466d8431bf082f3229328c5d7b2ae`. Only behavior reachable from the
supported Chaos/iarb2 path is a strategy-parity requirement; generic Java
gateway features such as algo-order execution, and Java's eight-session regular
command pool, do not grant live Chaos authority or dictate connection count. The
[capability inventory](docs/chaos-connectivity-inventory.md) records the audited
surface, and the
[Goal A handoff](docs/chaos-connectivity-goal-a-handoff.md) records the
verification gate.

This is a least-authority capability boundary, not a production-readiness
claim. Production order entry remains unavailable, credentialed target-host
evidence remains external, and Goal B does not change that. Goal B Phases 6
and 7 have lowered pure shared contracts and split live/research/capture
responsibilities while preserving their ordered owners. Its Phase 8 authority
hardening and Phase 9 verification are complete only when every focused and
global gate is green in the
[Goal B handoff](docs/chaos-connectivity-goal-b-handoff.md); see the
[refactor plan](docs/chaos-connectivity-refactor-plan.md) for the exact scope.

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
USDC/USD risk references and independent spot/swap price-limit, swap funding,
and swap mark streams, without credentials or private/account connections.
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

For deployment, install the reusable
`deploy/capture/okx-btc-public.toml` and use
`deploy/systemd/reap-capture@.service`. The unit keeps those config bytes fixed
and assigns create-new `raw.jsonl` and `run-report.json` paths from each unique
instance name; see `deploy/systemd/README.md`.

Capture validates the effective configuration before reserving the report. Once
reserved, handled setup, runtime, host, or writer failures produce a typed
schema-5 non-clean report before the command exits nonzero. Writer saturation and
shutdown are deadline-bound and never treated as clean; independent verification
and production research reject every report carrying failure evidence.

The one-hour command is an operational smoke, not funding-complete research
evidence. Production datasets must be continuous across exchange funding
boundaries. Schema-8 production research with swap candidates requires nonzero
realized funding settlements in both training and test aggregates, so short
captures cannot pass by carrying only forecasts.

For a production-candidate dataset, quiesce the target account and run
`certify-account` with the exact production live config and Reap executable
immediately before starting the credential-free capture. Retain the unique
certification beside that capture. Research re-derives its raw OKX evidence and
requires the certification, capture, latency calibration, executable, host,
account, and instrument accounting contracts to agree.

The example enables the Linux host guard. It fails before opening outputs or
websockets when disk, available memory, or kernel clock synchronization is
unhealthy, and stops on a failed periodic check. Disable it only in a copied
non-production diagnostic config. Production-candidate research requires the
guard, checks at least every 10 seconds, at least 5 GiB of available disk and
1 GiB of available memory, and synchronized kernel-clock enforcement; weaker
or disabled guards are rejected even when the capture itself completed cleanly.

Validate and backtest the raw capture directly. Raw replay runs the same OKX
adapter, redundant-feed deduplicator, sequence tracker, and book reducer used
by live trading. Use a new output path for each capture process; replay rejects
concatenated session IDs rather than treating downtime as continuous data.
Capture refuses an existing report, raw, or normalized path instead of
overwriting or appending. Verify the durable run report before standalone
analysis or strategy replay. Current captures also require an exact
`capture_record_seq` sequence from `1` through the report's raw record count;
missing, duplicate, skipped, or reordered writer records fail verification.
On Unix, all three artifacts are created mode
`0600` independently of the caller's umask:

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
An optional `[initial_portfolio]` section supplies one complete account-style
opening snapshot. Balances carry spot inventory through an explicit
`valuation_symbol`; nonzero derivative positions require average cost and
margin mode. The runner seeds the same snapshot into strategy risk state and
the accounting ledger, blocks order entry until the first complete book/rate
valuation, and reports both opening/final equity and net PnL. Schema-8
production research rejects candidate-file capital and derives one positive
opening portfolio per capture chain root from its independently verified account
certification. Adjacent ordinal ranges of one verified capture can continue from
the parent's settled portfolio and funding state. The report embeds those assumptions, actual sampled-latency usage,
the replay time basis, clock regressions, currency ledgers/rates, strategy halt
reason, live orders, and work still scheduled at the capture boundary. The
example delay values are zero,
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
horizon as pending actions. A manual run defaults to zero unless configured;
neither manual nor certified opening state is an exchange-statement substitute.

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
schema-5 run-report verifier, capture-config-bound source analysis, and an
independent zero-gap replay-integrity check before any candidate executes. The
capture report must identify the same executable as research and the same host
as the bound latency calibration, with at least one completed periodic host
check. The smoke fixture intentionally uses
permissive uncalibrated gates and is not
trading evidence. Production-candidate manifests use schema 8, predeclare one
`deployment_candidate_id`, and fail unless every fold independently selects
that candidate from training data. Candidate files must omit
`initial_portfolio`; every independent dataset or carry-chain root names a
unique, passing account certification collected on the same build and
calibrated host before capture. Explicit `capture_record_range` and
`continuation_of` fields may split one verified capture session into a linear
settled-carry chain; exact session, ordinal, receive-time, and source identity
are enforced.
The manifest supplies explicit spot valuation-symbol mappings, and research
derives identical candidate state from certified balances, derivative average
costs, and margin fields. Run PnL is final equity minus opening equity, not final
account capital. They also require nonzero training/test
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
revalidates the schema-8 candidate binding, and requires the candidate and live
config to serialize to the same effective strategy SHA-256. It also requires
every chain-root opening certification to embed those exact production-config
bytes. It does not enable order entry or replace transition, account, host,
fault, fee, funding, statement, deadman, and emergency evidence.

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

Every live strategy must set `reference_data_stale_threshold_ms`. That one
policy derives the exact critical websocket reference plan and gates both
startup and ongoing strategy validity. Index, funding, mark, and price-limit
timestamps are aged independently; a missing or stale source blocks entry and
triggers immediate cancellation of canonical working orders. The checked-in
120-second value accommodates OKX's variable funding update cadence and must be
reviewed against captured target-region behavior before promotion.

Each configured account also requires an initial, complete read-only proof that
all seven OKX pending algo query families and pending spread orders are empty.
The sentinel starts a scan every 15 seconds, treats a proof as usable for at
most 30 seconds, and gives the algo and spread scans independent 60-second
domain timeouts. Slow scans may overlap, but the default schedule is bounded
at five generations and all generations share the two domain pacers. A failure
invalidates zero results from every scan that was already in flight. Nonzero
state, proof expiry, endpoint/timeout failure,
malformed or unknown data, duplicate order identity, repeated pagination
cursor, or page-limit exhaustion fails closed. The coordinator blocks new
regular placement globally, targets regular reconciliation to the affected
account, and, when the Demo mutation role exists, issues canonical cancels only
for that account's owned regular orders. Regular Cancel All After, control
events, cancellation, and reconciliation use separate priority paths and
pacing from the read-only scans. The typed critical event tells an enabled
alert sink to direct the operator to the separate `reap-emergency` executable;
live neither imports nor invokes its algo/spread mutation authority. Placement
recovers only when both a fresh complete zero proof and a clean regular
reconciliation are present, in either completion order.

`runtime.connection_attempt_interval_ms = 400` serializes initial and
reconnecting WebSocket handshakes across every public/private feed and
authenticated order-command session. Official endpoint configurations also
require `runtime.connection_attempt_pacer_path`; owner-only advisory locking and
a fixed-format Linux boot-ID/`CLOCK_BOOTTIME` next-slot record make every Reap
process using that file reserve from one host-wide monotonic schedule. Unsafe
permissions, malformed state, or a
reservation more than 15 minutes ahead fails closed. Values below 334 ms are
rejected because
OKX documents a limit of three WebSocket connection requests per second per IP
in the [API guide](https://www.okx.com/docs-v5/en/). Only complete demo
loopback configurations may use zero without a state file; official endpoint
runtimes fail preflight outside Linux rather than weakening the monotonic-clock
guarantee. The checked-in examples use a relative development path; production
research and the aggregate production gate require an absolute path, and the
systemd baseline uses
`/var/lib/reap/connectivity/okx-global.pacer`. Fault-proxy upstream handshakes
reserve through that same file while its generated loopback live config does
not double-reserve. This coordinates one host only; multiple hosts sharing an
egress IP still require an external IP-wide coordinator or isolated egress.

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
and both files must use the same official endpoint region. Both configs must
also enable fatal alert delivery, the operator service, and the production host
guard floors; satisfy the exact plan-derived public redundancy and one nonempty
command lane for each executing account, and name an absolute process-shared
connection pacer. Legacy connection-count fields are migration caps and cannot
create replicas or idle command sessions. A pass does not read
credential values, prove those controls were exercised, authorize production,
or enable production order entry.

After every underlying credentialed and research artifact exists, replace the
placeholders in `examples/production-evidence.toml` and run the aggregate check
with the exact candidate binary on the declared target host:

```bash
target/release/reap verify-production-evidence \
  --manifest /secure/evidence/production-evidence.toml \
  --output /secure/evidence/production-evidence-verification.json \
  --require-pass --pretty
```

The command reconstructs transition, research deployment, dedicated demo soak,
the routed config from the exact official-demo and fault-proxy configs, fault
matrix, latency calibration, production account certification, demo deadman,
emergency cancel, authenticated fill/fee reconciliation, and account-wide
trade/funding economic reconciliation evidence.
It rejects missing or duplicate per-account coverage, a demo soak reused as a
fault session, config drift during verification, and any mismatch against the
manifest-declared build, host, candidate, or environment-specific account
identities. The demo and production configs must also name the same absolute
process-shared connection-pacer path. Schema-8 research opening certifications
must identify that same production account, build, and host; their verified raw
evidence fingerprints are carried through the format-3 reconstruction. Proxy-supported fault roles must carry
typed records with the exact proxy fingerprint, unique proxy session/command
IDs, fresh timestamps, and a command interval inside the corresponding verified
live session; only genuine partial-fill and restart-latch roles may use external
evidence. Schema 8 also
enforces reviewed age limits under hard code-level maxima for the demo soak,
every fault and latency source, production account certification, deadman,
emergency cancel, and the reconciled fill and bill windows. It requires one
independently verified clean proxy-process report per fault role, with a unique session that
encloses exactly that live run and the expected completed-command count. It does
not trust previously emitted verification JSON. The checked-in manifest is a
schema template with deliberately invalid placeholder identities, not evidence.

After a passing bundle exists, use independent approval workstations and the
fail-closed policy shape in `examples/production-approval-policy.toml`:

```bash
reap generate-production-approval-key \
  --private-key /secure/approver/private.json \
  --public-key /secure/approver/public.json --pretty

reap verify-production-approval-policy \
  --policy /secure/policy/production-approval.toml \
  --output /secure/evidence/approval-policy-verification.json --pretty

reap prepare-production-approval \
  --manifest /secure/evidence/production-evidence.toml \
  --policy /secure/policy/production-approval.toml \
  --request-id CHANGE-1234 --ttl-secs 600 \
  --output /secure/evidence/approval-request.json --pretty

reap sign-production-approval \
  --request /secure/evidence/approval-request.json \
  --policy /secure/policy/production-approval.toml \
  --private-key /secure/approver/private.json \
  --approver operations-approver \
  --output /secure/evidence/approval-operations.json --pretty

reap verify-production-approval \
  --manifest /secure/evidence/production-evidence.toml \
  --policy /secure/policy/production-approval.toml \
  --request /secure/evidence/approval-request.json \
  --approval /secure/evidence/approval-operations.json \
  --approval /secure/evidence/approval-risk.json \
  --output /secure/evidence/approval-verification.json \
  --require-pass --pretty
```

The policy requires sorted, unique approvers, distinct Ed25519 public keys, and
at least two required roles. A request is valid for at most 15 minutes and binds
the exact policy predeclared by SHA-256 in the schema-8 evidence manifest plus
stable source/config/build/host/account/candidate and gate evidence. Verification
rejects stale requests, missing roles, duplicate keys or approvers, signature or
binding changes, and any newly failing or changed source.
Key custody and proof that named approvers are independent humans remain external
governance controls. A pass still does not expose production order entry.

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
configured strategy borrow limits, exact configured API-key permissions, any
required exchange-reported IP binding, stable bracketed identity/settings, and
bounded exchange-clock evidence. For every non-USD balance currency it retains a
direct `CCY-USD` index ticker, rechecks `eqUsd` against `eq * idxPx`, and proves
that currency `eqUsd` values sum to `totalEq` within strict tolerances. The
artifact contains sensitive raw account responses and embedded config, so keep it
in restricted evidence storage. It is sequential point-in-time evidence, not an
atomic snapshot, historical borrowing proof, or full statement reconciliation;
quiesce account activity while it runs.

From another shell with the same operator token, inspect or stop the runtime:

```bash
cargo run -p reap-cli -- operator --config examples/live-okx-demo.toml status --pretty
cargo run -p reap-cli -- operator --config examples/live-okx-demo.toml kill-account --account main --reason "unexpected account exposure"
cargo run -p reap-cli -- operator --config examples/live-okx-demo.toml shutdown --reason "planned stop"
```

After stopping every order producer for an account, the independent emergency
path can cancel and verify all regular, algo, and spread pending orders without
loading strategy or journal state:

```bash
EMERGENCY_REPORT="/tmp/reap-emergency-$(date -u +%Y%m%dT%H%M%SZ).json"
cargo run -p reap-emergency-runner --bin reap-emergency -- \
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
The regular workflow is kicked off first. It releases the algo and spread
workflows immediately before attempting regular Cancel All After, or after
determining that request cannot be issued; regular enumeration continues in
either case. Regular, algo, and spread then advance independently with separate
pacers, progress, incidents, and final-zero proofs. Each workflow enforces the
same absolute per-account deadline independently, so a slow or failed
unsupported domain cannot consume the regular workflow's sequential request
budget or stop another domain. The existing report is merged deterministically
in regular, algo, then spread order.

`all_clear = true` is conjunctive: regular, algo, spread, and aggregate
account-wide all-clear flags plus complete config/binary/host/exchange-account/
task evidence must all pass. A verified domain can therefore retain valid
evidence while another domain remains unverified and keeps `all_clear = false`.
Early configuration failures leave an empty reserved path, which is never a
report. Both report paths are owner-only and synced with their parent directory.
Offline verification hashes the exact config/report bytes and re-derives
account coverage, each domain's trigger-horizon/final-zero pair, provenance,
and completion invariants. It cannot replay raw exchange responses or prove the
operator stopped every external order producer. OKX documents no algo-order
Cancel All After, so algo safety relies on that producer-stop confirmation,
explicit cancellation, and authoritative polling.

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

For an economic-evidence run, quiesce the account and collect a passing opening
account certification immediately before `BEGIN_MS`. After stopping the demo at
`END_MS`, keep every producer stopped and collect the closing certification
immediately. Each certification must be within 60 seconds of its boundary. Once
the bill window has been closed for at least 60 seconds, collect authenticated
fills and account-wide bills. The fill window starts one maximum trade-bill delay
earlier so a bill at the left boundary can still be matched causally:

```bash
BEGIN_MS=1783987200000
END_MS=1783990800000
MAX_TRADE_BILL_DELAY_MS=60000
FILL_BEGIN_MS=$((BEGIN_MS - MAX_TRADE_BILL_DELAY_MS))
FILL_EVIDENCE="/secure/evidence/okx-fills-$(date -u +%Y%m%dT%H%M%SZ)"
BILL_EVIDENCE="/secure/evidence/okx-bills-$(date -u +%Y%m%dT%H%M%SZ)"
FILL_REPORT="/tmp/reap-fill-reconciliation-$(date -u +%Y%m%dT%H%M%SZ).json"
ECONOMIC_REPORT="/secure/evidence/economics-$(date -u +%Y%m%dT%H%M%SZ).json"
OPENING_ACCOUNT="/secure/evidence/opening-account-$(date -u +%Y%m%dT%H%M%SZ).json"
CLOSING_ACCOUNT="/secure/evidence/closing-account-$(date -u +%Y%m%dT%H%M%SZ).json"

# Run while quiesced immediately before BEGIN_MS, then start the bounded demo.
cargo run -p reap-cli -- certify-account \
  --config examples/live-okx-demo.toml --account main \
  --output "$OPENING_ACCOUNT" --pretty

# Stop the demo at END_MS, keep the account quiesced, and run immediately.
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

cargo run -p reap-cli -- reconcile-fills \
  --journal var/reap/live-events.jsonl \
  --collection-manifest "$FILL_EVIDENCE/manifest.json" \
  --account main \
  --begin-ms "$FILL_BEGIN_MS" \
  --end-ms "$END_MS" \
  --minimum-fills 10 \
  --output "$FILL_REPORT" \
  --require-pass \
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
  --output "$ECONOMIC_REPORT" \
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
is weaker. The fill report covers fills and fees only.

`collect-bills` is also authenticated and read-only. It captures the account-wide
`/api/v5/account/bills` result for the exact closed window, paces requests at
least 500 ms apart, and requires a short terminal page within the conservative
166-hour age bound. `reconcile-economics` rebuilds both collections, leases and
streams the stopped journal, rejects every unexplained bill type, validates
trade identities/fees/balance equations, binds each trade to one account-scoped
critical fill, and recomputes derivative close PnL from the latest same-session
authoritative REST `avgPx` snapshot whose exchange timestamp strictly precedes
the fill. Linear basis uses arithmetic weighting and
inverse basis uses harmonic weighting, matching pinned Java `RiskCalculator`.
It also recomputes realized linear or inverse funding from the journaled settled
rate, assessment-time signed
position, configured contract value, and two same-session mark-price samples
bracketing the bill's `fillTime`. The bill mark must lie inside that independent
exchange-time bracket. Schema-7 `session_start` and critical `account_snapshot`
journal records bind those observations to one session, account, config,
and hashed OKX account identity, so evidence cannot cross a restart. The schema-5
economic report also reverifies the opening/closing schema-3 account artifacts,
requires every numeric bill ID and post-bill `bal`, checks every adjacent balance
edge, and proves `opening cashBal + sum(balChg) = closing cashBal` per currency.
It reports native/USD equity at both boundaries and the total-equity delta. Run a
controlled demo window containing nonzero trades
and at least one nonzero derivative close and funding settlement. The basis is
authenticated local runtime evidence, not remote attestation. Direct public
currency indexes constrain each point-in-time conversion but do not expose the
venue's exact internal valuation or funding-assessment tick. The report does not
attribute total-equity movement to mark-to-market PnL, taxes, deposits,
withdrawals, unsupported bill classes, or profitability.
Cash-spot bill units must be confirmed with credentialed demo
evidence for the exact target account before production review.

Each create-new schema-8 live report contains the exact source-config byte
count/SHA-256, checkpoint and full evidence config fingerprints, Reap executable
hash, pinned Java revision, and optional pseudonymous host identity. Established
sessions additionally contain pseudonymous account identity,
session/readiness/host evidence, and bounded per-class/per-symbol latency
samples. The CLI validates config/run options and captures exact source,
executable, and host provenance before reserving `--output` mode `0600`, then
syncs the report and parent directory. A handled runtime-construction failure
after reservation writes a source-bound pre-session report with
`session_id = null`, no account-identity or runtime-state claims, baseline
readiness, `clean_soak = false`, and a bounded stable failure code/message.
Its zero counters are not proof that the exchange has zero orders. Once the
report-capable runtime is constructed, initialization, event-loop, or teardown
failure still writes a full schema-versioned report after fail-closed cleanup
and exits non-zero. `verify-live-run` accepts a well-formed pre-session report as
diagnostic evidence but never as clean-soak acceptance; it independently
re-hashes exact config/report bytes, re-derives effective fingerprints and
acceptance, and checks mode, identity, readiness, failure, disconnect, host,
and latency invariants. An empty or incomplete reserved file means report
persistence failed or the process was forcibly terminated and is never evidence.

Live shutdown uses separate safety and ownership deadlines. The first bounds
cancel/reconcile; `runtime.teardown_timeout_ms` then bounds every host,
operator, websocket, command, storage, and alert owner. All owners are signalled
before joins, successful journal close flushes and data-syncs, and timeout
aborts remaining tasks while retaining Cancel All After and emitting typed
`teardown_timeout` evidence. Production evidence reserves five seconds of the
hardened 45-second systemd stop budget for report durability and exit.

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

- [docs/chaos-connectivity-boundary.md](docs/chaos-connectivity-boundary.md)
  defines the normative exchange capability allowlist for the current
  Chaos/iarb2 strategy, live safety, emergency recovery, and
  evidence/research separation.
- [docs/chaos-connectivity-refactor-plan.md](docs/chaos-connectivity-refactor-plan.md)
  is the phased, goal-ready plan for enforcing that boundary and reducing
  repository coupling without changing strategy behavior.
- [docs/chaos-connectivity-inventory.md](docs/chaos-connectivity-inventory.md)
  inventories each audited exchange capability, role, requirement, consumer,
  and disposition.
- [docs/chaos-connectivity-goal-a-handoff.md](docs/chaos-connectivity-goal-a-handoff.md)
  records the Goal A Phase 0–5 and Tranche A verification commands, hashes, and
  results.
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
