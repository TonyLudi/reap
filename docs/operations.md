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
   with the account. Never infer a regional domain from a symbol.
4. Start isolated private and partitioned public websocket plans. Authenticate
   private sockets before subscriptions.
5. Wait for a sequenced book snapshot for every traded symbol. A
   `FeedRecovered` event marks the public side ready.
6. Reconcile pending orders and recent fills over REST, then mark the private
   stream ready only when the report is clean.
7. Confirm storage queue depth, feed age, private age, and kill-switch state.
   Only then may the risk gate authorize new orders.

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
