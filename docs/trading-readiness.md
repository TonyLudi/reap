# Trading Readiness

Strategy parity and a tradable deployment are separate milestones. The iarb2
decision model is implemented, but `reap` does not yet expose a live command and
must not be treated as a production trading process.

## Current Gap

| Area | Current state | Trading impact |
| --- | --- | --- |
| Iarb2 decision model | Covered for the documented OKX parity boundary | Not a blocker |
| Deterministic backtest | Shared strategy code, depth matching, queue-ahead model, fees, and normalized replay | Needs venue-data calibration before capital decisions |
| Feed components | Multi-websocket planning, deduplication, sequencing, recovery, and OKX parsing exist as libraries | Must be composed and soak-tested |
| Order components | Signed submit/cancel, local registration, private reduction, pacing, and REST reconciliation exist as libraries | Must be composed and restart-tested |
| Runtime risk | Pre/post-trade limits, account-scoped health, kill switch, symbol halt, and fail-closed cancellation exist | Instrument models and account scopes must be wired from production config |
| Live process | No `live` CLI command or composition root | **Demo-trading blocker** |
| Instrument/account bootstrap | Sample config is for replay; no exchange metadata/account-mode verifier | **Demo-trading blocker** |
| Startup/restart gate | Procedure is documented but no process owns the complete state machine | **Demo-trading blocker** |
| Operator control and alerts | Typed events and telemetry primitives exist; no authenticated operator service | Production blocker |
| Exchange certification | No recorded OKX demo soak, fault injection, or production account-mode certification | Production blocker |

## Demo-Trading Critical Path

1. Add a `reap-live` composition crate or `reap live` command. It must own the
   single strategy event loop and route feed, private, timer, risk, storage, and
   gateway events without concurrent strategy mutation.
2. Load and verify exchange instrument metadata. Map every symbol to spot,
   linear, or inverse risk valuation; tick/lot/min size; contract value; settle
   currency; trade mode; and position mode.
3. Start all public and account-scoped private sockets, obtain sequenced books,
   fetch initial balances and positions, and reconcile open orders and recent
   fills before declaring readiness.
4. Route accepted `NewOrder` intents through
   `OkxOrderGateway::submit_registered`. Route cancels idempotently, and feed
   every private acknowledgement/fill back through the canonical reducer and
   engine.
5. Persist raw input, normalized input, intent, request, acknowledgement, fill,
   system event, and reconciliation result with enough identity to replay one
   account independently from another.
6. Run OKX demo trading at minimal size with disconnect, duplicate, gap,
   delayed-private-stream, partial-fill, IOC-miss, rate-limit, and process-restart
   fault tests.

## Production Gate

Production enablement additionally requires:

- Full-depth historical data and calibrated queue, latency, fee, funding, and
  slippage assumptions.
- Walk-forward and out-of-sample evaluation, parameter sensitivity, capacity,
  inventory-duration, and stressed-liquidity reports.
- Stablecoin depeg and exchange-rate pause policy, strategy-group risk, master
  liveness, and independent account/exchange kill controls.
- Clock synchronization monitoring, CPU/thread placement, bounded backpressure,
  memory and disk capacity alarms, and restart supervision.
- Long-running demo soak with zero unexplained order, fill, position, or balance
  reconciliation drift.
- Explicit operator approval of credentials, account mode, limits, symbols,
  and the production rollout/rollback procedure.

The first safe milestone is demo-tradable, not production-tradable. Production
capital should remain disabled until every startup, recovery, and reconciliation
invariant has executable acceptance evidence.
