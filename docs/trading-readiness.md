# Trading Readiness

Strategy parity and a tradable deployment are separate milestones. The iarb2
decision model and a fail-closed OKX demo composition are implemented. The
runtime has not completed a credentialed demo soak and must not be treated as a
production trading process.

## Current Gap

| Area | Current state | Trading impact |
| --- | --- | --- |
| Iarb2 decision model | Covered for the documented OKX parity boundary | Not a blocker |
| Deterministic backtest/data | Shared strategy code, depth matching, queue-ahead model, fees, credential-free redundant public capture, and raw/normalized replay | Needs sustained full-depth capture and venue-data calibration before capital decisions |
| Feed components | Redundant public sockets, isolated private sockets, ping/idle supervision, account-scoped deduplication, sequencing, and recovery are composed | Needs credentialed soak evidence |
| Order components | Event-loop client IDs/registration, exchange-side place-request expiry, signed submit/cancel, pacing, private reduction, ambiguity handling, and REST reconciliation are composed | Needs demo exchange fault evidence |
| Runtime risk | Instrument models, authoritative startup positions, account-scoped health, durable safety latches, exchange-clock checks, Cancel All After, and all-exit fail-closed cancellation/reconciliation are wired | Needs target-account limits review and credentialed deadman evidence |
| Live process | `live` supports config-only `validate`, read-only `observe`, explicitly confirmed demo order entry, and strict bounded soak reports | Demo-capable; production entry intentionally unavailable |
| Instrument/account bootstrap | Account instruments/config/balance/positions are typed and verified before readiness | Needs target-account certification |
| Startup/restart gate | Executable phase state, fingerprinted JSONL checkpoint restore, missed-fill/terminal-order recovery, durable latch restore, and clean REST reconciliation | Needs process-kill demo test |
| Event-loop profile | Allocation-aware raw OKX parity benchmark covers redundant wire input through strategy/risk and storage-record construction | Needs target-host capture and exchange-latency validation |
| Operator control and alerts | HMAC-authenticated local controls use fsynced write-ahead latches; OKX Cancel All After is maintained independently; a separate CLI can arm the deadman, cancel all regular orders account-wide, and prove post-trigger zero | Must exercise target alert routing and the independent cancel procedure; algo/spread orders remain outside its scope |
| Process/host controls | Canonical journal ownership is exclusively locked before recovery or network setup; optional Linux disk, memory, and kernel-clock checks run at preflight and periodically; hardened systemd templates encode mode-specific restart policy | Must be installed, enabled, thresholded, monitored, and fault-tested on the target host |
| Exchange certification | No recorded OKX demo soak, fault injection, or production account-mode certification | Production blocker |

## Implemented Demo Path

1. `reap-live` owns one strategy coordinator and routes feed, private, timer,
   risk, storage, and gateway events without concurrent strategy mutation.
2. Bootstrap verifies exchange instrument metadata and maps every symbol to
   spot, linear, or inverse risk valuation; tick/lot/min size; contract value;
   settle currency; trade mode; and position mode.
3. The runtime starts all public and account-scoped private sockets, obtains
   sequenced books, fetches initial balances and positions, and reconciles open
   orders and recent fills before declaring readiness. It also rejects excessive
   exchange-clock skew.
4. Accepted `NewOrder` intents receive a client ID and canonical `PendingNew`
   synchronously, then route through the account gateway. Cancels are deduplicated,
   every place request carries an OKX `expTime`, and every private
   acknowledgement/fill returns through the reducer/engine.
5. The critical log persists account-scoped raw input, normalized input,
   intent, request, acknowledgement, fill, system event, reconciliation result,
   and safety-latch mutation with enough identity to replay one account
   independently from another. Latches are synced before their actions dispatch.
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
   Demo mode arms and refreshes OKX Cancel All After from a separate task, and
   disables it only after clean zero-order shutdown reconciliation.
9. A `0600` Unix socket accepts bounded HMAC-signed operator commands with
   timestamp and nonce replay protection. Status and control responses are
   typed, mutations are persisted, and authenticated shutdown enters the same
   reconciled lifecycle path.
10. A journal-latched account kill blocks only that account's order route,
    removes its instruments from pricing and hedge selection, guarantees
    cancellation of its canonical active orders, rejects symbol resume while
    the account remains halted, survives restart, and exposes the latch reason
    in signed status. Global operator kills and post-trade risk breaches also
    survive restart; normal shutdown does not create a durable latch.
11. Startup canonicalizes the journal path and acquires a sibling OS file lock
    before reading credentials, recovery state, or network configuration. The
    runtime retains that lease until storage teardown, so aliases cannot start
    a second writer against the same journal.
12. Optional host guards check journal-filesystem capacity, Linux
    `MemAvailable`, and kernel clock synchronization before credentials or
    network I/O, then repeat outside the strategy loop. Optional webhook alerts
    use a bounded queue, HTTPS, bounded retry/timeouts, and report delivery
    failures back to the coordinator; production configuration should make
    those failures fatal.
13. A strategy-independent emergency command parses only exchange/account safety
    settings, refuses implicit account selection, requires producer-stop and
    account-wide confirmations, arms Cancel All After, batch-cancels every
    regular pending order across configured and unmanaged symbols, and verifies
    account-wide zero after the trigger horizon. Its deterministic tests cover a
    failed deadman, partial batch acknowledgement, and hung REST transport.
14. Hardened systemd templates permit bounded restart only for read-only observe
    mode. Demo and capture require operator-controlled restart so account
    reconciliation and capture-session rotation cannot be bypassed.

## Remaining Demo Gate

1. Review `examples/live-okx-demo.toml` against the actual demo account and
   current fee tier. Enable and threshold `[host_guard]`, and route `[alerts]`
   to a monitored test destination.
2. Run `observe` through reconnects and verify every account reaches `ready`
   with no reconciliation drift or critical storage backpressure. Use a bounded
   run with `--duration-secs <seconds> --require-clean-soak` so the result is
   machine-verifiable.
3. Run minimal-size `demo` orders, then inject socket disconnect, process kill,
   deadman expiry, exchange-clock skew, IOC miss, partial fill, and REST
   timeout/rate-limit conditions. Verify `expTime`, latch restoration, and
   Cancel All After from exchange/account evidence. Exercise the independent
   emergency command after forced process death and archive its zero-order
   report.
4. Complete a sustained soak with zero unexplained order, fill, balance,
   position, or checkpoint drift. `clean_soak` covers runtime readiness,
   reconciliation, storage drops, alert delivery, and shutdown orders;
   balances, positions, fills, and restart checkpoint state still require
   log/account review.

## Production Gate

Production enablement additionally requires:

- Full-depth historical data and calibrated queue, latency, fee, funding, and
  slippage assumptions.
- Walk-forward and out-of-sample evaluation, parameter sensitivity, capacity,
  inventory-duration, and stressed-liquidity reports.
- Stablecoin depeg and exchange-rate pause policy, strategy-group risk, master
  liveness, deployed external alert routing, and target-host exercise of the
  out-of-process regular-order kill plus any required algo/spread kill path.
- Target-host time-service monitoring, CPU/thread placement, bounded
  backpressure, calibrated memory/disk thresholds, installed restart
  supervision, and external unit-failure paging.
- Long-running demo soak with zero unexplained order, fill, position, or balance
  reconciliation drift.
- Explicit operator approval of credentials, account mode, limits, symbols,
  and the production rollout/rollback procedure.

The first safe milestone is demo-tradable, not production-tradable. Production
capital should remain disabled until every startup, recovery, and reconciliation
invariant has executable acceptance evidence.
