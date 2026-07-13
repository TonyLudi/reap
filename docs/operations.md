# Operations Guide

`reap` fails closed: missing or stale state can trigger cancels in demo mode but
cannot authorize a new order. `reap live` owns the implemented OKX lifecycle;
production order entry remains intentionally unavailable.

## Startup Gate

1. Run `reap live --config <path> --mode validate`. This reads no credentials
   and opens no network connection.
2. Start `--mode observe` with credentials supplied through the configured
   environment-variable names. Observe mode permits neither submits nor
   cancels.
   The example uses the global simulated hosts documented in the
   [OKX API guide](https://www.okx.com/docs-v5/en/); replace REST, public, and
   private domains together when the account belongs to another region.
3. The runtime opens the critical JSONL log before sockets and binds its
   checkpoint to the strategy/config fingerprint. Do not share a storage path
   between strategy configs.
4. The runtime fetches account-scoped instruments, account configuration,
   balances, positions, open orders, recent fills, and exact status for any
   restored active order.
5. It verifies live instrument state, type, linear/inverse contract type,
   tick/lot/minimum size, contract value, currencies, configured trade mode,
   account level, and `net_mode` before metadata is ready.
6. It restores canonical active orders and fill identities from JSONL, applies
   missed known fills/terminal updates from REST, and requires clean
   account-scoped reconciliation.
7. It starts redundant public plans and isolated orders, account, and positions
   sockets for every account. The dedicated fills channel is optional for
   eligible fee tiers; order-channel fills remain canonical. All configured
   private sockets must authenticate and remain live.
8. It waits for a contiguous sequenced book for every instrument and a healthy
   complete private connection set for every account.
9. Only phase `ready`, writable storage, healthy risk, and explicit
   `--mode demo --confirm-demo` permit a new order.

## Order Path

- The coordinator generates the client order ID and synchronously records a
  canonical `PendingNew` before dispatching to the account gateway task. The
  intent, pending state, and request are enqueued to critical storage before
  REST IO begins.
- Route explicit REST rejections back through the gateway state. Treat timeout
  and transport ambiguity as pending until REST/private reconciliation resolves
  it; do not blindly resubmit.
- Feed every order acknowledgement, fill, account update, and position update
  into the single-writer event loop. Strategy state must not be mutated from a
  websocket task.
- Keep private deduplication and health account-scoped. One healthy account must
  never mask another stale account.

## Fail-Closed Matrix

| Condition | Automatic state | Required action |
| --- | --- | --- |
| Public sequence gap | Book recovering; new orders blocked | Obtain a fresh snapshot and replay contiguous buffered deltas |
| Public feed stale | Symbol blocked; live orders cancelled | Restore at least one healthy feed and verify sequence continuity |
| Private stream stale | Account blocked; live orders cancelled | Reconnect, REST reconcile pending orders/fills, then emit recovery |
| Reconcile drift | Account blocked; live orders cancelled | Resolve local/remote order and fill differences before recovery |
| Risk breach | Kill switch active; live orders cancelled | Reduce exposure externally if needed, diagnose, and restart only after approval; live global reset is intentionally unavailable |
| Manual account kill | Account route latched off; its instruments are removed from pricing/hedging and its live orders are cancelled | Reconcile the account and review dependent strategy exposure; no live account reset is available |
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
  exchange events. Mutations are persisted as normalized/system records with
  their request ID.
- Protocol version 2 commands are read-only `status`, global `kill`, account
  `kill-account`, symbol `halt`, symbol `resume`, and reconciled `shutdown`.
  Status includes the global kill state and account-halt reasons. Older protocol
  versions are rejected.
- Account kills are process-latched and journaled as typed `AccountHalted`
  events. They block new requests to that account, halt every instrument routed
  to it inside the strategy, and cancel all canonical `PendingNew`, `Live`, and
  `PartiallyFilled` orders for it. A dependent quote on another account may also
  be withdrawn when the strategy loses a valid hedge.
- A symbol belonging to a killed account cannot be resumed. Live global and
  account reset commands are intentionally unavailable; restart only after the
  initiating cause and exchange state are reviewed and reconciled. The local
  latch does not replace exchange-side account controls or a supervisor policy
  that prevents an automatic restart from clearing it.

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

Synchronize the host clock: signed requests outside `max_clock_skew_ms` are
rejected. Observe mode remains exchange-read-only, so kill/halt events update
local state but cannot cancel exchange orders.

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
