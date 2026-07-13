# Trading Readiness

Strategy parity and a tradable deployment are separate milestones. The iarb2
decision model and a fail-closed OKX demo composition are implemented. The
runtime has not completed a credentialed demo soak and must not be treated as a
production trading process.

## Current Gap

| Area | Current state | Trading impact |
| --- | --- | --- |
| Iarb2 decision model | Covered for the documented OKX parity boundary | Not a blocker |
| Deterministic backtest/data | Shared strategy code, immediate pending-order registration, arrival-time scheduler, configurable market/entry/cancel/order/account delays, Java-referenced conservative depth-fill threshold, queue/trade/depth capacity haircuts, event-clock drawdown/exposure/inventory metrics, fee/turnover attribution, scheduled linear/inverse funding, manifest-driven chronological walk-forward selection and stress gates, credential-free redundant public capture, exact provenance, streaming analysis, and raw/normalized replay | Execution/accounting assumptions remain uncalibrated; needs sustained full-depth capture, complete funding intervals, depeg-sensitive quote valuation, borrow/fee calibration, statement reconciliation, and real production-candidate reports before capital decisions |
| Feed components | Redundant public sockets, isolated private sockets, transport/state freshness separation, account-plus-positions health rounds, ping/idle supervision, epoch-safe deduplication, reset-aware predecessor sequencing, and recovery are composed | Needs credentialed soak evidence |
| Order components | Event-loop client IDs/registration, exchange/client acknowledgement binding, account-scoped immutable private identity, semantic duplicate suppression across changed exchange timestamps, exchange-side place-request expiry, signed submit/cancel, pacing, monotonic private reduction, submit/cancel state-convergence deadlines, typed position margin mode, ambiguity handling, and full order/fill/balance/position REST reconciliation are composed | Needs demo exchange fault evidence |
| Runtime risk | Instrument models, authoritative startup positions, active-order count/notional ceilings, rolling submit-rejection and zero-fill IOC-cancel circuits, terminal strategy-halt promotion, position scope/mode enforcement, forced-repayment blocking, account-scoped health, per-fill state-convergence deadlines, redundant stablecoin guards, durable safety latches, exchange-clock checks, Cancel All After, and all-exit fail-closed cancellation/reconciliation are wired | Needs target-account limits review and credentialed deadman/depeg/convergence evidence |
| Live process | `live` supports config-only `validate`, read-only `observe`, explicitly confirmed demo order entry, and strict bounded soak reports | Demo-capable; production entry intentionally unavailable |
| Instrument/account bootstrap | Account instruments/config/balance/positions are typed; live spot is cash-only; nonzero positions require configured account ownership and derivative mode before strategy/risk application | Needs target-account certification |
| Startup/restart gate | Executable phase state, engine-consumed account-snapshot invariant, fingerprinted JSONL checkpoint restore, missed-fill/terminal-order recovery, durable latch restore, authoritative account repair, and second-pass clean REST reconciliation | Needs process-kill demo test |
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
15. A real 5-minute public OKX capture reached all 12 baseline redundant socket
    plans, wrote 36,402 frames, split exactly into 18,201 accepted and 18,201
    duplicates, passed strict replay with no integrity defect, and completed
    raw-capture backtest replay. A revised 75-second run reached all 14 plans and
    captured redundant USDT/USD and USDC/USD references without a conflicting
    same-timestamp value. Deterministic fixtures separately cover maintenance
    sequence resets, no-change updates, conflicting replicas, and missed-reset
    recovery. A later 60-second run exercised the streaming analyzer: all ten
    configured streams had both expected sources, both books retained 400
    levels per side, capture/analysis config fingerprints and raw SHA-256
    matched, and strict analysis/replay found no integrity defect. This does not
    replace sustained capture, execution calibration, or credentialed evidence.
16. Live risk subscribes to configured stablecoin/USD indexes on redundant
    critical routes. Missing, stale, invalid, conflicting, or downside-depegged
    data blocks entry immediately; a sustained 5-second failure persists a
    global risk latch and cancels live orders. Startup readiness requires every
    guard, and production validation requires guards for used USDT/USDC
    currencies.
17. Account-snapshot readiness is set only after the scoped REST/private update
    has passed through the strategy and risk engine. Live validation rejects
    master/group strategy topology until its external heartbeat, state, and PnL
    feed exists.
18. Private transport and account-state health are separate. Every socket must
    remain connected, while only a complete account/positions data round
    refreshes account health; pongs and event-only order/fill traffic cannot
    mask a silent state channel.
19. Every REST reconciliation compares balances and positions as well as orders
    and fills before replacing account state. Omitted rows clear through zero
    tombstones, stale websocket rows cannot regress the engine, and a repaired
    dirty pass remains degraded until a later full-state pass is clean.
20. Every canonical derivative fill must be covered by its position row, while
    every spot fill must be covered by both currency balances. A configured
    deadline emits account-scoped drift, cancels live orders, and starts full
    reconciliation independently of aggregate private-stream heartbeat.
21. Both OKX position sources retain `mgnMode`. Bootstrap and every later
    nonzero derivative position must match the configured `cross` or `isolated`
    trade mode, while full-state reconciliation also compares local and remote
    mode; mismatch fails the live lifecycle before applying the position.
22. Live position scope is total and fail-closed: spot order routing is
    cash-only, and every nonzero account position must be a configured
    derivative owned by the account that produced the update. Unmodeled
    exposure aborts before strategy/risk application; zero closure rows remain
    admissible.
23. OKX account balance `twap` is retained as a typed per-currency
    forced-repayment indicator. Values at or above the configured `1..=5`
    threshold abort bootstrap/runtime before state application, while REST
    reconciliation compares lower values and authoritative tombstones clear
    omitted currencies.
24. Local `PendingNew` orders and dispatched cancels have an explicit
    order-state convergence deadline. Timeout blocks the account, releases the
    expired cancel from deduplication so it can be retried, and requires full
    REST reconciliation; only a terminal private or recovered state clears a
    pending cancel.
25. Global and per-symbol active-order count ceilings are checked against the
    projected pre-trade order set. Canonical private or REST-recovered state
    above either ceiling triggers the durable post-trade risk kill, preventing
    low-notional order proliferation from bypassing the live-order notional
    limit; remote-only orders remain a separate reconciliation blocker.
26. Canonical exchange submit rejections are deduplicated by order ID and
    counted in configured rolling global/per-symbol windows. Reaching either
    threshold persists the global risk latch and cancels active orders. The
    Java-referenced per-symbol zero-fill IOC cancellation window uses canonical
    local time-in-force, also deduplicates order IDs, and drives the same durable
    stop; partially filled IOC residuals remain separate `MissedHedge` records.
27. Every terminal chaos strategy halt is observed through the generic strategy
    safety contract after its callback and before intent dispatch. The engine
    activates global risk, rejects same-callback new orders, cancels all active
    orders, and causes the coordinator to persist the latch; a reset cannot
    reopen the same still-halted strategy instance.
28. REST submit/cancel acknowledgements bind exchange IDs one-to-one with
    registered client IDs and active bindings recover from the journal before
    REST reduction. Empty/`0` private IDs resolve through the same map;
    wrong-account symbols, either-direction rebinding, and known-order
    symbol/side changes fail before canonical order or fill state mutates.
29. Private order reduction suppresses an already-seen fill ID when status is
    unchanged and cumulative fill does not advance, plus repeated unchanged
    terminal states by canonical order ID, even when OKX sends a different
    update timestamp.
30. Backtest order entry and cancellation are deterministic scheduled exchange
    actions rather than immediate function calls. Raw replay uses persisted
    receive time, pending quotes/hedges suppress duplicate intents, and reports
    retain every effective delay, clock regression, live order, and action left
    beyond the final input. Defaults remain explicitly uncalibrated, and the
    current short public capture had no fills from which to estimate execution.
    Displayed-depth matching also applies Java's relative over-cross threshold
    and clears queue-ahead on a shallow cross, but its value is still an
    inherited conservative default rather than target-venue evidence.
31. Backtest fills now attribute configured maker/taker fee cost and turnover.
    Funding forecasts schedule one signed linear or inverse swap settlement at
    the advertised exchange time, update to the latest rate, prefer exchange
    mark over midpoint fallback, and expose late, missed, failed, and
    post-horizon states. The short accepted capture had two rate updates but no
    fill or funding boundary, so formulas are tested but not empirically
    calibrated or reconciled to an account statement.
32. Queue-ahead, historical-trade participation, and displayed-depth capacity
    are explicit reportable assumptions. Their `1.0` defaults preserve Java
    behavior; higher queue and lower participation/capacity values support
    deterministic stress runs. They remain global heuristics and do not model
    hidden liquidity, cancellation flow, or venue queue priority.
33. A versioned research manifest now selects candidates from training data
    only, enforces chronological non-overlap, applies conservative baseline and
    stress scenarios to the selected candidate's test data, and emits immutable
    manifest/binary/config/effective-strategy/data fingerprints plus embedded
    selection and gate policy, accounting, drawdown, position/pending delta,
    gross position/active-order exposure, inventory-duration, and pending-work
    results. Production raw inputs must also pass embedded capture-config-bound
    multi-source/candidate-channel analysis and an independent zero-gap replay
    check before selection. The checked-in
    smoke fold validates plumbing with permissive uncalibrated gates and negative
    fee-adjusted PnL; it is not production evidence. Each dataset currently
    starts from zero rather than carrying Java's daily ending positions.

## Remaining Demo Gate

1. Review `examples/live-okx-demo.toml` against the actual demo account and
   current fee tier. Confirm the subaccount has no margin-spot or unmanaged
   position and every nonzero derivative position has the configured owner and
   margin mode. Confirm every currency's forced-repayment indicator is below
   the configured limit. Review global/per-symbol active-order counts against
   quote levels and hedge concurrency. Enable and threshold `[host_guard]`, and
   route `[alerts]` to a monitored test destination.
2. Run `observe` through reconnects and verify every account reaches `ready`
   with no reconciliation drift or critical storage backpressure. Use a bounded
   run with `--duration-secs <seconds> --require-clean-soak` so the result is
   machine-verifiable. Confirm both stablecoin references remain fresh and
   inject a transient guard failure without creating a durable latch.
3. Run minimal-size `demo` orders, then inject socket disconnect, process kill,
   deadman expiry, exchange-clock skew, IOC miss, partial fill, and REST
   timeout/rate-limit conditions. Suppress submit and cancel order pushes to
   exercise order-state convergence, then suppress derivative position updates
   and each side of a spot balance update to exercise fill convergence. Verify
   cancel retry, `expTime`, latch restoration, and Cancel All After from
   exchange/account evidence. Exercise the independent emergency command after
   forced process death and archive its zero-order report.
4. Complete a sustained soak with zero unexplained order, fill, balance,
   position, or checkpoint drift. `clean_soak` covers runtime readiness,
   full-state reconciliation, storage drops, alert delivery, and shutdown
   orders; fill-convergence latency and restart checkpoint state still require
   log/account review.

## Production Gate

Production enablement additionally requires:

- Full-depth historical data and calibrated queue, latency, fee, funding, and
  slippage assumptions, including empirical per-message/per-instrument delay
  distributions and empirical validation of the displayed-depth fill threshold.
- Completed `production_candidate` walk-forward and out-of-sample manifests
  using calibrated assumptions, sustained captures, parameter sensitivity,
  capacity, inventory-duration, and stressed-liquidity reports. The runner is
  implemented; no qualifying report has been produced.
- Target-account calibration and independent exercise of the implemented
  stablecoin guard; either implementation and exercise of external
  strategy-group/master coordination or continued rejection of those settings;
  deployed external alert routing; and target-host exercise of the
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
