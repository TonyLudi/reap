# Trading Readiness

Strategy parity and a tradable deployment are separate milestones. The iarb2
decision model and a fail-closed OKX demo composition are implemented. The
runtime has not completed a credentialed demo soak and must not be treated as a
production trading process.

## Current Gap

| Area | Current state | Trading impact |
| --- | --- | --- |
| Iarb2 decision model | Covered for the documented OKX parity boundary | Not a blocker |
| Deterministic backtest/data | Shared strategy code, immediate pending-order registration, arrival-time scheduler, Java-mapped class/symbol empirical latency profiles with sampled-usage reporting, versioned target-host/live collectors, deterministic calibration artifacts bound into production research, conservative depth/queue/trade capacity controls, event-clock drawdown/exposure/inventory metrics, per-currency depeg-sensitive valuation, exact private-fill fee currency plus explicit simulated-fee counts, fee/turnover attribution, authenticated recent-fill collection and verified offline fill/fee reconciliation, authenticated point-in-time cash/zero-liability collection and offline verification, scheduled linear/inverse funding, manifest-driven chronological walk-forward selection and stress gates, credential-free redundant public capture, exact provenance, streaming analysis, and raw/normalized replay | The evidence pipeline is implemented but execution/accounting assumptions remain uncalibrated; needs sustained full-depth and currency-index capture, a passing credentialed target-host/demo latency artifact, complete funding intervals, target-tier fee calibration, a real passing target-account certification and authenticated fill/fee artifact plus broader economic statement reconciliation, and production-candidate reports before capital decisions |
| Feed components | Redundant public sockets, isolated private sockets, transport/state freshness separation, account-plus-positions health rounds, ping/idle supervision, epoch-safe deduplication, reset-aware predecessor sequencing, and recovery are composed | Needs credentialed soak evidence |
| Order components | Event-loop client IDs/registration, exchange/client acknowledgement binding, account-scoped immutable private identity, semantic duplicate suppression across changed exchange timestamps, exchange-side place-request expiry, signed submit/cancel, pacing, monotonic private reduction, submit/cancel state-convergence deadlines, typed position margin mode, ambiguity handling, and bounded complete order/fill/balance/position REST reconciliation are composed | Needs demo exchange fault evidence |
| Runtime risk | Instrument models, authoritative startup positions, active-order count/notional ceilings, rolling submit-rejection and zero-fill IOC-cancel circuits, terminal strategy-halt promotion, position scope/mode enforcement, zero-liability enforcement, periodic authenticated account-config drift detection, forced-repayment blocking, account-scoped health, per-fill state-convergence deadlines, redundant stablecoin guards, durable safety latches, exchange-clock checks, Cancel All After, and all-exit fail-closed cancellation/reconciliation are wired | Needs target-account limits review and credentialed deadman/depeg/convergence evidence |
| Live process | `live` supports config-only `validate`, read-only `observe`, explicitly confirmed demo order entry, and strict bounded soak reports | Demo-capable; production entry intentionally unavailable |
| Instrument/account bootstrap | Account instruments/config/balance/positions are typed; economic snapshots preserve borrowing flags, liabilities, interest, and margin-loan fields; live spot and borrow limits are cash-only/zero; enabled borrowing, missing applicable evidence, nonzero liabilities, margin positions, and nonzero positions outside configured ownership/mode fail before strategy/risk application | Needs a passing artifact from the real target account; tooling alone is not evidence |
| Startup/restart gate | Executable phase state, engine-consumed account-snapshot invariant, fingerprinted JSONL checkpoint restore, missed-fill/terminal-order recovery, durable latch restore, authoritative account repair, second-pass clean REST reconciliation, and read-only journal-bound deadman-expiry certification | Needs process-kill demo evidence; tooling alone is not evidence |
| Event-loop profile | Allocation-aware raw OKX parity benchmark covers redundant wire input through strategy/risk and storage-record construction | Needs target-host capture and exchange-latency validation |
| Operator control and alerts | HMAC-authenticated local controls use fsynced write-ahead latches; OKX Cancel All After is maintained independently; a separate CLI can arm the deadman, cancel all regular orders account-wide, and prove post-trigger zero; another read-only CLI can prove source `20` after controlled process death | Must exercise target alert routing, deadman expiry, and the independent cancel procedure; algo/spread orders remain outside their scope |
| Process/host controls | Canonical journal ownership is exclusively locked before recovery or network setup; optional Linux disk, memory, and kernel-clock checks run at preflight and periodically; hardened systemd templates encode mode-specific restart policy | Must be installed, enabled, thresholded, monitored, and fault-tested on the target host |
| Build/supply chain | Rust `1.95.0` is pinned; least-privilege CI checks formatting, all-target lint, all workspace tests, a locked release build, and RustSec advisories; Cargo and Actions updates are proposed weekly | CI must remain green and dependency updates reviewed, but this does not replace credentialed exchange or target-host evidence |
| Exchange certification | Point-in-time account certification and journal-bound process-death deadman collection/offline replay are implemented, but no passing target-account artifact, OKX demo soak, deadman artifact, or broader fault campaign is recorded | Production blocker |

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
    account-wide zero after the trigger horizon. Its create-new schema-1 report
    binds the exact config file, executable, host, Java revision, matching
    pseudonymous exchange-account identity, selected-account coverage, and
    bounded task failures before returning its final exit status. Its
    deterministic tests cover a failed deadman, partial batch acknowledgement,
    hung REST transport, missing credentials, identity failure, and task loss.
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
    matched, and strict analysis/replay found no integrity defect. A later
    schema-3 run durably bound the exact config file and effective CLI overrides;
    report-aware verification matched raw and independently reconstructed
    normalized bytes/counters/hashes with no failure. This does not replace
    sustained capture, execution calibration, or credentialed evidence.
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
29. Private order reduction suppresses an already-seen `(symbol, fill_id)` when
    status is unchanged and cumulative fill does not advance, plus repeated
    unchanged terminal states by canonical order ID, even when OKX sends a
    different update timestamp. The instrument scope follows OKX `tradeId`
    uniqueness; restart journals persist scoped keys, while legacy unscoped IDs
    migrate as conservative wildcards.
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
    Private order/fill and REST paths additionally preserve the exact signed fee
    and currency, book it in the balance that changed, and report exact versus
    estimated fee-fill counts. Order-channel `fillFee` is per update; cumulative
    order-level `fee` is deliberately not treated as a last-fill charge.
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
    results. Production raw inputs must also pass an embedded schema-3 capture
    report verification that binds exact config/raw/optional-normalized evidence,
    capture-config-bound multi-source/candidate-channel analysis, and an
    independent zero-gap replay check before selection. The checked-in
    smoke fold validates plumbing with permissive uncalibrated gates and negative
    fee-adjusted PnL; it is not production evidence. Each dataset currently
    starts from zero rather than carrying Java's daily ending positions.
34. Backtest latency can now use bounded empirical samples for Java-mapped
    `market_depth`, `historical_trade`, `matching_new`, `matching_cancel`,
    `order_update`, and `order_fill` classes plus Rust `reference_data`, with
    optional symbol overrides and scalar fallback. Stable quantile sampling is
    reproducible and reported by class/symbol. Baseline/stress profiles require
    the same seed and stochastic dominance.
35. Versioned live reports now collect bounded per-class/per-symbol target-host
    visibility, REST-acknowledgement, private-update, and fill-to-account-state
    samples, binding the Rust executable plus pseudonymous host and exchange
    account identities. A deterministic CLI rejects mismatched
    config/code/host/account/session/clock or failed-operation evidence, emits a
    profile only after every required series passes, and binds the exact
    artifact/profile into schema-3 production research. Matching new/cancel
    measurements are explicitly retained as conservative REST-ack upper bounds.
    No representative credentialed report or passing calibration artifact has
    yet been certified.
36. The live CLI reserves its create-new evidence path before config,
    credentials, or network activity. Runtime and teardown failures complete
    fail-closed cleanup, persist a schema-7 report with a stable failure code,
    readiness, split public/private disconnect evidence, and post-cleanup order
    state, then preserve the original nonzero exit. Reports separately classify
    ambiguous submit/cancel outcomes, partial fills, order/fill convergence
    timeouts, restored latches, and periodic safety-task failures. Critical
    ready/disconnect transitions wait for bounded capacity instead of being
    dropped.
    This makes injected demo faults auditable; failures before runtime
    construction still require the reserved empty path and process log.
37. Order-channel fills now create the same canonical exact-fee journal record
    as the optional fills channel and cross-channel duplicates are suppressed by
    instrument-scoped `tradeId`; a fee-less fills-channel event arriving first
    cannot suppress the later exact order record. REST recovery requests 100
    rows per page and follows `billId` until a short page; duplicate fills/cursors
    or the bounded page limit fail closed. An authenticated read-only collector
    now retains exact response pages, brackets them with exchange-clock and
    pseudonymous account-identity samples, and emits a create-new manifest only
    after a short terminal page. Offline verification re-hashes the exact config,
    manifest, and pages, replays the request/cursor chain, leases the journal,
    binds its bootstrap config identity, compares fill fields and signed fees,
    and emits a schema-2 pass/fail artifact. No real demo artifact has yet been
    produced; manual older-history exports still require explicit account/window
    attestation.
38. A process-death certification composition acquires the stopped journal's
    exclusive lease before credentials/network work, recovers durable live
    exchange/client bindings, and performs only public-time and authenticated
    GET requests. A pass requires at least one recovered live/partial regular
    order, terminal `canceled` details with OKX source `20` for every order,
    account-wide regular pending-order zero, stable account/settings/time, and
    no pending/unbound/unmapped/truncated journal state. The credential-free
    verifier takes its own lease and replays the exact external journal plus all
    embedded raw responses. No real demo artifact has yet been produced.
39. Path-launched live reports now bind the exact source-config byte count and
    SHA-256 in addition to both effective fingerprints. Owner-only create-new
    output is synced with its parent directory. `verify-live-run` re-hashes the
    supplied config/report, rejects legacy or unknown report fields, checks the
    pinned Java/build/mode/host/account/session boundaries, validates readiness,
    failure, disconnect, and latency evidence, and independently re-derives
    `clean_soak`. Latency calibration schema 3 also requires the exact source
    bytes and independently verified source reports.
40. `verify-live-fault-matrix` now requires one isolated schema-7 run for every
    documented live role, hashes a distinct injector record for each fault,
    rejects session/artifact reuse, and binds all runs to one exact config,
    executable, host, and account identity. Reconnect roles must recover cleanly;
    disruptive order-path roles must retain zero-order, no-drop shutdown
    evidence. Its output explicitly excludes process-death causality, deadman
    expiry, emergency cancellation, fill/fee reconciliation, and deployment
    approval, so no credential-free fixture can satisfy the remaining demo gate.

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
   run with `--duration-secs <seconds> --output <create-new-report>
   --require-clean-soak` so the result is machine-verifiable. Confirm both
   stablecoin references remain fresh and inject a transient guard failure
   without creating a durable latch.
3. Run minimal-size `demo` orders, then inject socket disconnect, process kill,
   deadman expiry, exchange-clock skew, IOC miss, partial fill, and REST
   timeout/rate-limit conditions. Suppress submit and cancel order pushes to
   exercise order-state convergence, then suppress derivative position updates
   and each side of a spot balance update to exercise fill convergence. Verify
   cancel retry, `expTime`, and latch restoration from exchange/account
   evidence. In one controlled process-death iteration, do not issue a cancel,
   wait for expiry, and archive a passing `certify-deadman-expiry` artifact plus
   `verify-deadman-certification --require-pass` result. In a separate forced-
   death iteration, exercise the independent emergency command and archive its
   zero-order report; incident cancellation must never wait for certification.
   Populate `examples/live-fault-matrix.toml` with the isolated reports and
   injector records, then require `verify-live-fault-matrix --require-pass`.
4. Complete a sustained soak with zero unexplained order, fill, balance,
   position, or checkpoint drift. `clean_soak` covers runtime readiness,
   full-state reconciliation, storage drops, alert delivery, and shutdown
   orders; restart checkpoint state still requires log/account review. Generate
   a passing `calibrate-latency` artifact from synchronized target-host observe
   and demo reports, archive every source hash/file, and reconcile its private
   timing populations against exchange/account records. Run `collect-fills` for
   the closed bounded-demo window and require a passing manifest-backed
   `reconcile-fills` artifact with a reviewed nonzero minimum-fill threshold.

## Production Gate

Production enablement additionally requires:

- Full-depth historical data and calibrated queue, latency, fee, funding, and
  slippage assumptions, including empirical per-message/per-instrument delay
  distributions and empirical validation of the displayed-depth fill threshold.
  Latency requires a passed source-bound calibration artifact; the implemented
  REST-ack matching measurements must remain labeled and approved as upper
  bounds unless a closer exchange boundary is added.
- A passing target-account `certify-account` artifact, independently rechecked
  with `verify-account-certification --require-pass`, immediately before
  approval. This is point-in-time evidence and must be combined with economic
  statement reconciliation. Margin spot remains unsupported; enabling it
  requires an explicit borrow-rate/interest model and demo reconciliation first.
- A passing target-host demo `certify-deadman-expiry` artifact, independently
  rechecked against the exact stopped journal with
  `verify-deadman-certification --require-pass`, plus separate supervisor/fault-
  injector and emergency-cancel evidence.
- Sustained redundant direct currency/USD index coverage for every non-USD
  accounting currency, with zero conversion failures and fee/cash/funding/equity
  reconciliation against target-tier demo statements.
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
