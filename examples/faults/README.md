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

See [the demo fault proxy runbook](../../docs/operations.md#demo-fault-proxy) for
the complete process and evidence limitations.
