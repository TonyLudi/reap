# Fault Command Templates

These strict JSON files target the loopback-only OKX demo fault proxy configured
by `../okx-demo-fault-proxy.toml`.

`status.json` and `shutdown.json` can be submitted directly. For every injected
campaign role, copy the relevant template into the private campaign directory
and assign a unique `command_id` and `evidence_file`. Do not delete prior
evidence or reuse the checked-in static artifact name; create-new enforcement is
part of the audit boundary.

Arm one expected fault per isolated live session. A frame-drop or REST-response
artifact is written only after all requested matches occur. A websocket
disconnect artifact is written immediately and records whether every selected
connection acknowledged the injected close.

The checked-in proxy-backed campaign roles are:

| Role | Template and exact match |
| --- | --- |
| Public/private/order reconnect | `public-reconnect.json`, `private-reconnect.json`, or `order-transport-reconnect.json`; one selected bridge disconnect |
| Submit/cancel ambiguity | `ambiguous-submit.json` or `ambiguous-cancel.json`; one exchange-to-client order-command acknowledgement |
| Order convergence | `order-convergence-timeout.json`; one exchange-to-client private `orders` frame |
| Fill convergence | `fill-convergence-timeout.json`; one exchange-to-client private `positions` frame; change the channel to `account` for a spot campaign |
| Deadman heartbeat | `deadman-heartbeat-failure.json`; one `POST /api/v5/trade/cancel-all-after` response |
| Clock/status/instrument/fee/account configuration | The corresponding `exchange-*.json` or `account-config-failure.json`; one exact periodic REST endpoint response |

The clock/status/instrument/fee/account-configuration templates inject `503` and
therefore exercise `*_check` failures; the deadman template exercises
`deadman_heartbeat`. A separate reviewed command with a valid changed `200` body
is required for clock skew, maintenance, metadata drift, fee drift,
account-configuration drift, and `upcChg` behavior. Typed evidence records the
endpoint and response hash, not the semantic correctness of that body.

There is intentionally no proxy template for a genuine exchange partial fill or
for restart with a durable safety latch. Those roles require external exchange
and stopped-process evidence. The matrix rejects Reap proxy evidence for them.

See [the demo fault proxy runbook](../../docs/operations.md#demo-fault-proxy) for
the complete process and evidence limitations.
