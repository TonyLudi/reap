# Operations Guide

`reap` fails closed: missing or stale state can cancel exposure-reducing orders
but cannot authorize a new order. The live gateway remains a library so a
deployment must explicitly wire credentials, health, storage, and operator
controls before it can trade.

## Startup Gate

1. Run `reap config-check` and reject any invalid symbol, sizing, or risk-group
   relationship.
2. Start structured telemetry and bounded storage before feed or order tasks.
3. Build the OKX adapter with the regional websocket/REST domains associated
   with each account. Call `with_account_id` for every private adapter; never
   infer account identity or a regional domain from a symbol.
4. Start isolated private and partitioned public websocket plans. Public plans
   must include sequenced books for every instrument, trades where required,
   funding rates for swaps, configured index tickers, and derivative mark/price
   limits. Private plans for every account must include orders, fills, account,
   and positions. Authenticate private sockets before subscriptions.
5. Wait for a sequenced book snapshot for every traded symbol. A
   `FeedRecovered` event marks the public side ready.
6. Fetch initial account balances, margins, and positions. Reconcile pending
   orders and recent fills over REST, then mark each account's private stream
   ready only when its report is clean.
7. Register each symbol's `InstrumentRiskModel` as spot, linear derivative, or
   inverse derivative with the correct contract value. A derivative must not
   inherit the risk gate's spot default.
8. Confirm storage queue depth, every feed age, every private account age,
   position/account freshness, exchange and calculated margin ratios, and
   kill-switch state.
   Only then may the risk gate authorize new orders.

The repository currently provides these steps as library boundaries; no live
composition process owns the complete startup gate yet.

## Order Path

- Register the local `NewOrder` with the private reducer before awaiting the
  network request. Use `OkxOrderGateway::submit_registered` so an early
  websocket acknowledgement cannot lose its `quote` or `hedge` reason.
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
| Storage loss | Counter/health degradation | Investigate capacity; critical intent/order/fill records must not be dropped |

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

Activate the kill switch, send cancels, wait for private acknowledgements or
REST reconciliation, flush critical storage records, then stop sockets and
telemetry. A process exit with unresolved live orders is an incident, not a
successful shutdown.
