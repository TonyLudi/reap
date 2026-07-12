# Trading Readiness

Strategy parity and a tradable deployment are separate milestones. The iarb2
decision model and a fail-closed OKX demo composition are implemented. The
runtime has not completed a credentialed demo soak and must not be treated as a
production trading process.

## Current Gap

| Area | Current state | Trading impact |
| --- | --- | --- |
| Iarb2 decision model | Covered for the documented OKX parity boundary | Not a blocker |
| Deterministic backtest | Shared strategy code, depth matching, queue-ahead model, fees, and normalized replay | Needs venue-data calibration before capital decisions |
| Feed components | Redundant public sockets, isolated private sockets, ping/idle supervision, account-scoped deduplication, sequencing, and recovery are composed | Needs credentialed soak evidence |
| Order components | Event-loop client IDs/registration, signed submit/cancel, pacing, private reduction, ambiguity handling, and REST reconciliation are composed | Needs demo exchange fault evidence |
| Runtime risk | Instrument models, authoritative startup positions, account-scoped health, kill switch, symbol halt, and all-exit fail-closed cancellation/reconciliation are wired | Needs limits review against the target demo account |
| Live process | `live` supports config-only `validate`, read-only `observe`, explicitly confirmed demo order entry, and strict bounded soak reports | Demo-capable; production entry intentionally unavailable |
| Instrument/account bootstrap | Account instruments/config/balance/positions are typed and verified before readiness | Needs target-account certification |
| Startup/restart gate | Executable phase state, fingerprinted JSONL checkpoint restore, missed-fill/terminal-order recovery, and clean REST reconciliation | Needs process-kill demo test |
| Event-loop profile | Allocation-aware raw OKX parity benchmark covers redundant wire input through strategy/risk and storage-record construction | Needs target-host capture and exchange-latency validation |
| Operator control and alerts | Typed events and telemetry primitives exist; no authenticated operator service | Production blocker |
| Exchange certification | No recorded OKX demo soak, fault injection, or production account-mode certification | Production blocker |

## Implemented Demo Path

1. `reap-live` owns one strategy coordinator and routes feed, private, timer,
   risk, storage, and gateway events without concurrent strategy mutation.
2. Bootstrap verifies exchange instrument metadata and maps every symbol to
   spot, linear, or inverse risk valuation; tick/lot/min size; contract value;
   settle currency; trade mode; and position mode.
3. The runtime starts all public and account-scoped private sockets, obtains
   sequenced books, fetches initial balances and positions, and reconciles open
   orders and recent fills before declaring readiness.
4. Accepted `NewOrder` intents receive a client ID and canonical `PendingNew`
   synchronously, then route through the account gateway. Cancels are deduplicated,
   and every private acknowledgement/fill returns through the reducer/engine.
5. The critical log persists account-scoped raw input, normalized input,
   intent, request, acknowledgement, fill, system event, and reconciliation
   result with enough identity to replay one account independently from another.
6. Component and coordinator tests cover disconnect, duplicate, gap,
   delayed-private-stream, partial-fill, IOC-miss, rate-limit, and process-restart
   behavior.
7. The production-shaped live benchmark covers raw-record cloning, OKX JSON
   adaptation, redundant-feed deduplication, sequencing, 400-level books,
   strategy/risk evaluation, and coordinator record construction. The measured
   optimizations and exclusions are recorded in `docs/performance.md`.
8. Normal stops and runtime failures share one bounded shutdown path. New
   submits are disabled independently from cancel permission; every account
   must return a post-cancel REST reconciliation result before teardown.
   Integration coverage injects a fatal runtime error and closed storage while
   a canonical order is live, then verifies cancel-before-reconcile ordering.

## Remaining Demo Gate

1. Review `examples/live-okx-demo.toml` against the actual demo account and
   current fee tier.
2. Run `observe` through reconnects and verify every account reaches `ready`
   with no reconciliation drift or critical storage backpressure. Use a bounded
   run with `--duration-secs <seconds> --require-clean-soak` so the result is
   machine-verifiable.
3. Run minimal-size `demo` orders, then inject socket disconnect, process kill,
   IOC miss, partial fill, and REST timeout/rate-limit conditions.
4. Complete a sustained soak with zero unexplained order, fill, balance,
   position, or checkpoint drift. `clean_soak` covers runtime readiness,
   reconciliation, storage drops, and shutdown orders; balances, positions,
   fills, and restart checkpoint state still require log/account review.

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
