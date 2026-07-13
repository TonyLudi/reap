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

Validate and consume the output directly:

```bash
cargo run -p reap-cli -- replay-check \
  --events "$RAW_PATH" --strict --pretty
cargo run -p reap-cli -- backtest \
  --config examples/iarb2-okx-btc.toml \
  --data "$RAW_PATH" \
  --format raw-capture --pretty
```

Run capture under a disk-capacity supervisor. JSONL is currently uncompressed
and can grow quickly; rotate on every process start and before the filesystem
reaches its alarm threshold. Never place capture output under source control.

### Recorded Public Smoke

On 2026-07-13, the public configuration completed a bounded 20-second run with
all 12 redundant socket plans ready at stop. It wrote 3,443 raw frames
(3,111,900 bytes), accepted 1,727 events, and classified 1,716 exact redundant
images as duplicates. Capture and strict replay both reported zero gaps,
recovery failures, parse errors, stale books, disconnects, and unrecovered
streams. The raw file SHA-256 was
`227acd3e3f21e84fb8c3a6fa9866c500be95bb5eeaca6925aa6d574d7c8ece30`.

Raw-capture backtest replay then completed and generated 26 simulated orders
with no fills in the short window. This is connectivity and replay plumbing
evidence only. It is not a sustained capture, execution-model calibration,
profitability result, credentialed soak, or production approval.

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
   fills/terminal updates from REST and requires clean account-scoped
   reconciliation.
9. In demo mode it arms OKX Cancel All After for every account before starting
   that account's private feed or order task.
10. It starts redundant public plans and isolated orders, account, and positions
   sockets for every account. The dedicated fills channel is optional for
   eligible fee tiers; order-channel fills remain canonical. Every transition
   of the private socket set to ready invalidates the earlier reconciliation
   and requires a fresh REST order/fill check, closing the bootstrap-to-stream
   subscription window.
11. It waits for a contiguous sequenced book for every instrument and a
    complete, healthy, post-connect reconciled private stream for every account.
12. Only phase `ready`, writable storage, healthy risk, and explicit
   `--mode demo --confirm-demo` permit a new order.

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
| Private stream stale | Account blocked; live orders cancelled | Reconnect, REST reconcile pending orders/fills, then emit recovery |
| Reconcile drift | Account blocked; live orders cancelled | Resolve local/remote order and fill differences before recovery |
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
canonical orders plus clean account reconciliation permits task teardown.
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
