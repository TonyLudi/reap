# Operations Guide

`reap` fails closed: missing or stale state can trigger cancels in demo mode but
cannot authorize a new order. `reap live` owns the implemented OKX lifecycle;
production order entry remains intentionally unavailable.

## Public Data Capture

`reap capture` is a separate composition root. It opens only the configured
public websocket URL, reads no credentials, creates no private subscription,
and has no dependency on the order gateway or strategy coordinator.

```bash
cargo run -p reap-cli -- capture \
  --config examples/capture-okx-public.toml \
  --duration-secs 3600 \
  --require-clean-capture \
  --pretty
```

The example captures spot and swap books/trades plus index, funding, mark, and
price-limit inputs used by iarb2. Every subscription has two independent
connections. Raw frames are canonical and append-only. The optional
`output.normalized_path` is intended for short diagnostics because full
400-level snapshots are much larger than raw deltas.

Every frame and run report carries a generated `capture_session_id`. Start a
new output path for each capture process. Strict replay and raw backtest reject
files containing more than one session ID, because process downtime is not a
continuous HFT market stream; split an accidentally concatenated file at the
session boundary before use.

`clean_capture` requires a bounded duration, all socket plans ready at least
once and at stop, a ready contiguous snapshot for every configured book,
non-empty raw output, and zero parse, gap, stale-book, recovery-request,
recovery-route, or recovery-failure counts. A redundant-socket disconnect can
remain clean only when the other replica preserves sequence continuity;
inspect disconnect, duplicate, and writer queue counts on every run.

Validate and consume the output directly:

```bash
cargo run -p reap-cli -- replay-check \
  --events var/reap/capture/okx-btc-raw.jsonl --strict --pretty
cargo run -p reap-cli -- backtest \
  --config examples/iarb2-okx-btc.toml \
  --data var/reap/capture/okx-btc-raw.jsonl \
  --format raw-capture --pretty
```

Run capture under a disk-capacity supervisor. JSONL is currently uncompressed
and append-only; rotate on every process start and before the filesystem reaches
its alarm threshold. Never place capture output under source control.

## Startup Gate

1. Run `reap live --config <path> --mode validate`. This reads no credentials
   and opens no network connection.
2. Start `--mode observe` with credentials supplied through the configured
   environment-variable names. Observe mode permits neither submits nor
   cancels.
   The example uses the global simulated hosts documented in the
   [OKX API guide](https://www.okx.com/docs-v5/en/); replace REST, public, and
   private domains together when the account belongs to another region.
3. The runtime recovers the critical JSONL log and binds its checkpoint to the
   strategy/config fingerprint. It rejects safety latches for accounts or
   symbols not owned by the current config. Do not share a storage path between
   strategy configs.
4. It compares midpoint-adjusted local time with OKX `/api/v5/public/time` and
   fails bootstrap when skew exceeds `max_exchange_clock_skew_ms`.
5. The runtime fetches account-scoped instruments, account configuration,
   balances, positions, open orders, recent fills, and exact status for any
   restored active order.
6. It verifies live instrument state, type, linear/inverse contract type,
   tick/lot/minimum size, contract value, currencies, configured trade mode,
   account level, and `net_mode` before metadata is ready.
7. It restores canonical active orders, fill identities, and active global,
   account, and symbol safety latches from JSONL. It applies missed known
   fills/terminal updates from REST and requires clean account-scoped
   reconciliation.
8. In demo mode it arms OKX Cancel All After for every account before starting
   that account's private feed or order task.
9. It starts redundant public plans and isolated orders, account, and positions
   sockets for every account. The dedicated fills channel is optional for
   eligible fee tiers; order-channel fills remain canonical. Every transition
   of the private socket set to ready invalidates the earlier reconciliation
   and requires a fresh REST order/fill check, closing the bootstrap-to-stream
   subscription window.
10. It waits for a contiguous sequenced book for every instrument and a
    complete, healthy, post-connect reconciled private stream for every account.
11. Only phase `ready`, writable storage, healthy risk, and explicit
   `--mode demo --confirm-demo` permit a new order.

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
- demo shutdown resolved every active canonical order.

The report also records time-to-ready, recovered readiness losses and maximum
outage, disconnects, stale-stream events, book recoveries, and the storage queue
high-water mark. It also reports authenticated operator commands and mutations.
Recovered disconnects do not by themselves fail acceptance,
but their counts must match the injected fault plan. In demo mode the final
`readiness` may be degraded by the deliberate shutdown kill switch; acceptance
uses the pre-shutdown `readiness_at_stop` snapshot.

A `clean_soak` result is evidence for this bounded runtime window only. Review
the JSONL log, account balances, positions, fills, and checkpoint restart before
checking off the sustained demo gate.
