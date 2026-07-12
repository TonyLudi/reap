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
| Risk breach | Kill switch active; live orders cancelled | Reduce exposure externally if needed, diagnose, and obtain operator reset |
| Critical storage loss/backpressure | Runtime fail-stop; checkpoint reconciliation required on restart | Investigate disk/queue capacity; critical records are never silently dropped |

## Operator Controls

- A symbol halt blocks new orders for that symbol and generates cancellation
  intents for its live orders through the event loop.
- The global kill switch blocks all new orders. Cancels remain permitted.
- Reset is a separate typed event. Do not reset until feeds are ready, private
  reconciliation is clean, exposure is within limits, and the initiating cause
  is understood.

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

`SIGINT` or `SIGTERM` in demo mode activates the kill switch, dispatches
cancels, keeps private sockets and REST reconciliation running, and waits for
zero active canonical orders and clean accounts. It then flushes critical
storage and stops sockets/tasks. Exceeding `shutdown_timeout_ms` returns an
error with unresolved counts; treat that as an incident. Observe mode performs
no exchange mutation and shuts down directly.

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
- demo shutdown resolved every active canonical order.

The report also records time-to-ready, recovered readiness losses and maximum
outage, disconnects, stale-stream events, book recoveries, and the storage queue
high-water mark. Recovered disconnects do not by themselves fail acceptance,
but their counts must match the injected fault plan. In demo mode the final
`readiness` may be degraded by the deliberate shutdown kill switch; acceptance
uses the pre-shutdown `readiness_at_stop` snapshot.

A `clean_soak` result is evidence for this bounded runtime window only. Review
the JSONL log, account balances, positions, fills, and checkpoint restart before
checking off the sustained demo gate.
