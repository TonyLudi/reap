# Trading Readiness

Strategy parity and a tradable deployment are separate milestones. The iarb2
decision model and a fail-closed OKX demo composition are implemented. The
runtime has not completed a credentialed demo soak and must not be treated as a
production trading process.

The current broad connectivity implementation is being narrowed under the
[Chaos connectivity boundary](chaos-connectivity-boundary.md) and its
[refactor plan](chaos-connectivity-refactor-plan.md). Completing that refactor
is a design prerequisite, not proof of exchange certification or production
approval.

## Current Gap

| Area | Current state | Trading impact |
| --- | --- | --- |
| Iarb2 decision model | Covered for the documented OKX parity boundary | Not a blocker |
| Deterministic backtest/data | Shared strategy code, immediate pending-order registration, arrival-time scheduler, Java-mapped class/symbol empirical latency profiles with sampled-usage reporting, versioned target-host/live collectors, deterministic calibration artifacts bound into production research, conservative depth/queue/trade capacity controls, event-clock drawdown/exposure/inventory metrics, per-currency depeg-sensitive valuation, exact private-fill fee currency plus explicit simulated-fee counts, fee/turnover attribution, authenticated recent-fill and account-wide bill collection with verified offline trade/fee/funding reconciliation, journal-backed derivative close-PnL reconstruction, bracketed bill-to-cash continuity, authenticated point-in-time cash/zero-liability/equity-conversion collection and offline verification, forecast/realized funding separation with event-time linear/inverse settlement, manifest-driven chronological walk-forward selection and stress gates with exact independent report reconstruction, credential-free redundant public capture, exact provenance, streaming analysis, and raw/normalized replay | The evidence pipeline is implemented but execution/accounting assumptions remain uncalibrated; needs sustained full-depth and currency-index capture, a passing credentialed target-host/demo latency artifact, complete funding intervals, target-tier fee calibration, real passing target-account and fill/bill economic artifacts including authoritative position-basis/close and opening/closing cash samples, empirical cash-spot bill semantics, reviewed total-equity attribution, and production-candidate reports before capital decisions |
| Feed components | Redundant public sockets, isolated private sockets, transport/state freshness separation, independently aged index/funding/mark/price-limit sources, account-plus-positions health rounds, ping/idle supervision, epoch-safe deduplication, reset-aware predecessor sequencing, and recovery are composed | Needs credentialed soak evidence |
| Order components | Event-loop client IDs/registration, exchange/client acknowledgement binding, account-scoped immutable private identity, semantic duplicate suppression across changed exchange timestamps, exchange-side place-request expiry, an authenticated eight-session websocket command pool, constant-time per-underlying FIFOs, bounded cross-underlying command concurrency, shared account pacing, independent REST reconciliation, shutdown command flushing, monotonic private reduction, submit/cancel state-convergence deadlines, typed position margin mode, and ambiguity handling are composed | Needs demo exchange fault evidence |
| Runtime risk | Instrument models, authoritative startup positions, authenticated current instrument-rule, hard single-order maximum, and fee-group checks, final pre-trade exchange-limit enforcement, typed upcoming-change lead, active-order count/notional ceilings, rolling submit-rejection and zero-fill IOC-cancel circuits, terminal strategy-halt promotion, position scope/mode enforcement, zero-liability enforcement, periodic authenticated account-config drift detection, forced-repayment blocking, account-scoped health, per-fill state-convergence deadlines, redundant stablecoin guards, durable safety latches, exchange-clock and announced-maintenance checks, Cancel All After, and all-exit fail-closed cancellation/reconciliation are wired | Needs target-account limits review and credentialed instrument/fee/deadman/depeg/convergence evidence |
| Live process | `live` supports config-only `validate`, read-only `observe`, explicitly confirmed demo order entry, strict bounded soak reports, documented region/environment endpoint tuples, an exact demo-to-production config transition verifier, and a source-rebuilding cross-gate production evidence bundle | Demo-capable; production entry intentionally unavailable, and no passing target-host bundle exists |
| Instrument/account bootstrap | Account instruments/config/balance/positions are typed; authenticating API-key permissions and IP bindings are retained; exact configured scope is enforced with `withdraw` forbidden and production trade keys IP-bound; economic snapshots preserve borrowing flags, liabilities, interest, and margin-loan fields; live spot and borrow limits are cash-only/zero; enabled borrowing, missing applicable evidence, nonzero liabilities, margin positions, and nonzero positions outside configured ownership/mode fail before strategy/risk application | Needs a passing artifact from the real target account and host; tooling alone is not evidence |
| Startup/restart gate | Executable phase state, engine-consumed account-snapshot invariant, mandatory independently fresh strategy references, immediate canonical cancellation on reference-readiness loss, fingerprinted JSONL checkpoint restore, missed-fill/terminal-order recovery, durable latch restore, authoritative account repair, second-pass clean REST reconciliation, and read-only journal-bound deadman-expiry certification | Needs process-kill demo evidence; tooling alone is not evidence |
| Event-loop profile | Allocation-aware raw OKX parity benchmark covers redundant wire input through strategy/risk and storage-record construction | Needs target-host capture and exchange-latency validation |
| Operator control and alerts | HMAC-authenticated local controls use fsynced write-ahead latches; OKX Cancel All After is maintained independently; a separate CLI can arm regular and spread deadmen, exhaustively cancel regular/algo/spread orders account-wide, and prove every domain zero with offline exact-config verification; another read-only CLI can prove regular-order source `20` after controlled process death | Must exercise target alert routing, regular-order deadman expiry, and the account-wide independent cancel procedure on the target account |
| Process/host controls | Canonical journal ownership is exclusively locked before recovery or network setup; live and public capture share Linux disk, memory, and kernel-clock preflight/periodic checks; all official WebSocket handshakes on one host reserve through one owner-only process-shared pacer; capture schema 5 binds binary/host evidence and an exact process-global persisted-frame ordinal; hardened systemd templates encode mode-specific restart policy and bounded evidence capture | Must be installed, enabled, thresholded, monitored, and fault-tested on the target host; hosts sharing a NAT need isolated egress or an external IP-wide pacer |
| Demo fault injection | A loopback-only proxy independently routes OKX demo REST, public, private-state, and order-command traffic; strict owner-local commands inject disconnects, matched frame drops, and matched REST responses; create-new injector/run artifacts bind proxy config and pinned Java provenance, and the live fault matrix validates supported typed roles | Tooling is implemented and credential-free forwarding smokes passed, but no credentialed isolated campaign or target-host acceptance evidence exists |
| Build/supply chain | Rust `1.95.0` is pinned; least-privilege CI checks formatting, all-target lint, all workspace tests, a locked release build, and RustSec advisories; Cargo and Actions updates are proposed weekly | CI must remain green and dependency updates reviewed, but this does not replace credentialed exchange or target-host evidence |
| Exchange certification | Point-in-time account certification, journal-bound process-death deadman replay, exact fill/bill collection, and offline normal-trade/funding economics are implemented, but no passing target-account artifact, OKX demo soak, economic artifact, deadman artifact, or broader fault campaign is recorded | Production blocker |

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
    account-wide confirmations, arms regular and spread Cancel All After,
    exhaustively pages and cancels every regular, algo, and spread pending order,
    and verifies all domains zero after the trigger horizon. Its create-new
    schema-2 report
    binds the exact config file, executable, host, Java revision, matching
    pseudonymous exchange-account identity, selected-account coverage, and
    bounded task failures before returning its final exit status. Its
    deterministic tests cover regular/algo/spread cancellation, a failed deadman,
    partial batch acknowledgement, hung REST transport, missing credentials,
    identity failure, and task loss.
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
    sustained capture, execution calibration, or credentialed evidence, and the
    historical schema-3 artifact is not accepted by the current schema-5 gate.
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
    Funding forecasts continue to drive Java-parity strategy pricing but are no
    longer treated as realized cash. The OKX adapter retains
    `settFundingRate` plus observed `prevFundingTime`; a private preload pass
    lets accounting apply the realized rate to signed linear or inverse
    exposure at the original exchange timestamp without strategy look-ahead.
    Missing or conflicting realized rates fail closed, and schema-8 production
    research requires nonzero settled intervals in both training and test
    aggregates. The short accepted capture had rate updates but no fill or
    funding boundary, so formulas are tested but not empirically calibrated or
    reconciled to an account statement.
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
    results. Production raw inputs must also pass an embedded schema-5 capture
    report verification that binds exact config/raw/optional-normalized evidence,
    capture binary, latency-calibrated target host, pinned Java revision, and
    healthy host-guard evidence, plus capture-config-bound multi-source and
    candidate-channel analysis and an independent zero-gap replay check before
    selection. Schema 8 additionally
    requires one predeclared deployment candidate to win training-only selection
    in every fold. Candidate files omit opening capital; each raw capture chain
    root instead binds a unique, passing production account certification
    collected before capture on the exact research build and latency-calibrated host. Research
    re-derives raw OKX evidence, enforces account/instrument scope and a bounded
    handoff, rejects nonzero unmodeled state, and supplies the same certified
    balances, average costs, available/equity/loan, and margin fields to every
    candidate's strategy and accounting state. Order entry waits for its first
    complete valuation, reports distinguish opening/final equity from net PnL,
    and terminal strategy halts fail research evidence. This prevents final
    capital or candidate-specific inventory from being scored as strategy
    profit. A format-3 independent reconstruction exposes dataset opening-account
    identities and that candidate's effective strategy hash, and a separate production binding
    now requires it to equal the exact proposed live config's
    `strategy.effective()` hash. The checked-in
    smoke fold validates plumbing with permissive uncalibrated gates and negative
    fee-adjusted PnL; it is not production evidence. Schema-8 research can split
    one verified capture session into explicit adjacent ordinal ranges. It warms
    books plus latest index, funding, mark, and price-limit state from the parent
    prefix and carries independently validated settled
    balances, positions, derivative average costs, margin fields, currency rates,
    and funding state across the exact session/ordinal/time boundary. Terminal
    settlement mirrors Java's mark-to-market, average-cost reset, hold release,
    and cancel-all finish-up contract. Unchained datasets still reset
    independently. Cross-file or cross-process carry remains unavailable until
    capture rotation preserves one verifiable session and process-global ordinal.
    No passing target-account certification, target-host latency reconstruction,
    or representative economic research bundle has been collected, so this
    internal continuity work does not make production entry acceptable.
34. Backtest latency can now use bounded empirical samples for Java-mapped
    `market_depth`, `historical_trade`, `matching_new`, `matching_cancel`,
    `order_update`, and `order_fill` classes plus Rust `reference_data`, with
    optional symbol overrides and scalar fallback. Stable quantile sampling is
    reproducible and reported by class/symbol. Baseline/stress profiles require
    the same seed and stochastic dominance.
35. Versioned live reports now collect bounded per-class/per-symbol target-host
    visibility, websocket-order-acknowledgement, private-update, and fill-to-account-state
    samples, binding the Rust executable plus pseudonymous host and exchange
    account identities. A deterministic CLI rejects mismatched
    config/code/host/account/session/clock or failed-operation evidence, emits a
    profile only after every required series passes, and binds the exact
    artifact/profile into schema-8 production research. An independent verifier
    now re-hashes an explicit complete source-report set, reruns live verification,
    rebuilds every series/profile with recorded options, and compares the result
    after content-hash path normalization. Matching new/cancel measurements are
    explicitly retained as conservative order-ack upper bounds. No representative
    credentialed report, passing calibration, or passing reconstruction artifact
    has yet been certified.
36. The live CLI reserves its create-new evidence path before config,
    credentials, or network activity. Runtime and teardown failures complete
    fail-closed cleanup, persist a schema-8 report with a stable failure code,
    readiness, split public/private/order-transport disconnect evidence, and post-cleanup order
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
    `clean_soak`. Latency calibration schema 4 also requires the exact source
    bytes and independently verified source reports.
40. The schema-3 `verify-live-fault-matrix` manifest requires one isolated
    schema-8 run for every documented live role, including distinct exchange
    status, instrument, fee, and account-config failures. It hashes a distinct
    injector record for each fault, rejects typed-failure substitution and
    session/artifact reuse, and binds all runs to one exact config, executable,
    host, and account identity. Reconnect roles must recover cleanly; disruptive
    order-path roles must retain zero-order, no-drop shutdown evidence. Its
    output explicitly excludes process-death causality, deadman expiry,
    emergency cancellation, fill/fee reconciliation, and deployment approval,
    so no credential-free fixture can satisfy the remaining demo gate.
41. `verify-emergency-cancel` independently re-hashes exact config/report bytes,
    rejects symlinks and duplicate or mismatched account coverage, re-derives
    regular/algo/spread zero only after the Cancel All After trigger horizon, and
    can require every configured account. Emergency evidence and its verification
    artifact are owner-only and directory-durable. The report does not embed raw
    REST bodies or prove that all external order producers stopped. OKX documents
    no algo CAA, so algo proof requires explicit cancellation and authoritative
    zero polling under the producer-stop attestation.
42. Authenticated live and emergency REST configuration now rejects arbitrary
    TLS hosts, cleartext production, URL credentials, alternate paths/ports,
    query/fragment data, environment mismatches, and mixed Global, US/AU, EEA,
    or Turkey tuples. Only a complete demo loopback tuple is accepted for local
    deterministic tests.
43. `verify-production-transition` hashes two exact validated configs and
    recursively compares their typed effective values. It permits only reviewed
    endpoint and deployment/secret-binding changes, requires one official
    endpoint region, reports bounded digest-backed JSON-Pointer differences,
    and preserves all strategy, risk, account-policy, runtime, execution,
    storage-capacity, and safety settings. Format 2 now also rejects either
    config unless fatal alerts, the operator service, production host-guard
    floors, at least two public and order-command sessions, and an absolute
    shared connection pacer are configured. It does not inspect secrets, prove
    the controls were exercised, or authorize production.
44. WebSocket connection attempts are serialized across public, private,
    order-command, capture, and fault-proxy-upstream supervisors for initial
    startup and recovery. An owner-only advisory-lock file reserves one schedule
    across Reap processes on the target host using the kernel boot ID and
    `CLOCK_BOOTTIME`; production research requires an absolute path and the
    aggregate bundle requires the same path in demo and
    production configs. Unsafe or malformed state fails closed. This strengthens
    pinned Java's in-process `connectIntervalMs` creation context but cannot
    coordinate another host sharing the same NAT. A
    pre-change 30-second public run encountered one `alb-qps-limited` HTTP 503,
    recovered, and ended with 14/14 sockets ready, one disconnect, 1,728 raw
    records, 867 accepted events, and 861 duplicates. On 2026-07-15, two
    simultaneous 40-second captures sharing one state file each reached and
    retained 14/14 sockets, exercising 28 supervised sockets across two
    processes. Both had zero disconnect, gap, recovery, parse, stale-book, or
    terminal-book defect. They wrote 3,524 and 4,209 raw records, accepted 1,786
    and 2,158 events, and deduplicated 1,738 and 2,051. Independent exact-report
    verification and strict replay passed at raw SHA-256
    `3013f304ee707726e589dc473ec7bebd8566f007a6e963e2a6d55b17adf5df54` and
    `1c6790bdce95945acd4f5e3d01d4c103d3b865344cfbde1eaafa3253698cc182`;
    the state file remained mode `0600`, single-linked, and fixed at 103 bytes.
    Receive-time backtests reconstructed 2,447 and 2,928 inputs, modeled 18
    orders in each run with no fills, and ended order-entry-ready with complete
    accounting, valuation, and currency-rate coverage. This same-host,
    public-only smoke validates cross-process startup pacing, capture integrity,
    replay readiness, and accounting plumbing; it is not a complete funding
    interval, sustained data, execution calibration, credentialed demo evidence,
    multi-host NAT coordination, or production approval.
45. Live bootstrap and one periodic safety task now parse and poll the current
    unsigned OKX system-status contract. The 10-second poll, 60-second lead, and
    unified service filter follow pinned Java `OkxNitroExchStatusClient`,
    `OkxNitroUtils.getExchStatus`, and `ExchStatusSafeguard`; current OKX
    environment scoping prevents production-only notices from stopping demo and
    vice versa. Relevant maintenance or an unreadable response fails closed
    into cancel/reconcile cleanup. This is implemented evidence, not proof that
    the target-host path has encountered and recovered from real maintenance.
46. Live bootstrap now binds every configured strategy instrument to the
    authenticated current OKX private-instrument `groupId` and matching
    `/api/v5/account/trade-fee` `feeGroup` row. It converts OKX's signed
    commission/rebate convention to Java/Rust strategy cost rates and refuses
    configured costs that understate commission or overstate rebate. A paced
    periodic full-account sweep repeats the check with stable typed failures;
    its request runs separately from Cancel All After and a blocking-transport
    test proves the deadman continues. This follows Java
    `StrategyToolkit.getTransactionFee` and `ChaosEntity.sanityCheck` while
    adapting to OKX's post-2025 fee groups. It is a safety bound, not a real
    target-tier calibration or fill/statement reconciliation artifact.
47. The same isolated safety child now re-fetches every exact authenticated
    account instrument before its periodic fee check. It rejects non-live
    state or drift in type, family, underlying, currencies, contract
    type/value, tick/lot/minimum size, or fee group. Current `upcChg`
    announcements are mandatory and strictly typed for `tickSz`, `minSz`
    (including OKX's synchronous derivative `lotSz` change), and `maxMktSz`;
    entry is refused when a change enters the default one-hour lead. This
    protects Java's `Instrument` assumptions used by `Iarb2ChaosEntityFactory`,
    `ChaosEntity`, and `OkEntity.getMinTradeSize`. Blocking metadata and fee
    requests are independently tested not to delay Cancel All After. This is
    implemented protection, not target-account fault evidence.
48. Current authenticated `maxLmtSz`, `maxMktSz`, and applicable spot
    `maxLmtAmt`/`maxMktAmt` are now mandatory typed metadata and part of the
    periodic drift snapshot. Bootstrap rejects configured quote maxima above
    the limit-order bounds. Because pinned Java's
    `Iarb2Calculator.summarizeHedges` may combine several depth levels into one
    `Iarb2Strategy.doHedge` IOC, Rust additionally checks each final post-only
    or IOC quantity and applicable spot USD amount immediately before dispatch.
    This prevents an aggregated hedge from relying only on earlier strategy
    sizing, while preserving Java's target calculation. It still needs a real
    target-account response and oversized-order demo fault artifact.
49. `reap-fault` now runs outside the trading process and accepts only an
    official OKX demo upstream behind distinct loopback REST, public, private,
    and order-command listeners. It injects targeted disconnects, matched frame
    drops, and bounded REST responses through an owner-only Unix socket, then
    writes create-new typed evidence without retaining credentials or raw
    private payloads. The matrix independently validates artifact structure,
    effects, hashes, and the exact reconnect, ambiguity, convergence, deadman,
    clock, status, instrument, fee, or account-configuration role. It rejects
    proxy artifacts for genuine partial fills and restart-latch proof because
    those require exchange and stopped-process evidence. Credential-free REST
    and public-websocket forwarding smokes passed, but no authenticated account
    path or strategy response has been certified.
50. `verify-latency-calibration` no longer relies on a calibration artifact's
    internal consistency or `passed` flag. It binds exact config and source bytes,
    independently re-verifies each source live report, reconstructs all
    Java-mapped class/symbol series and the final profile, permits only source-path
    relocation, and emits an owner-only source-bound verification artifact. The
    implementation and forged-profile tests pass; target-host evidence does not
    yet exist.
51. `verify-research` independently binds a bounded archived report to the exact
    manifest and byte-identical executable, re-runs every fold and scenario,
    permits only content-hash normalization of verifier-observed capture paths,
    and rejects unknown or duplicate fields, stale inputs, forged metrics, non-passing runs,
    and even one-ULP numeric drift. Research generation and verification outputs
    are owner-only and directory-durable. The determinism audit also restored
    the pinned Java `TreeSet` traversal intent and ordered portfolio/order
    reductions. The smoke reconstruction passes; no production-candidate report
    exists.
52. `verify-production-evidence` consumes one strict manifest but does not trust
    subordinate verification JSON. It directly reconstructs transition,
    production research binding, a dedicated demo soak, the full fault matrix,
    latency calibration, production account certifications, demo deadman and
    emergency evidence, authenticated fill/fee reconciliation, and account-wide
    trade/funding economic reconciliation. It requires
    exact per-account coverage and predeclared candidate/build/host plus separate
    demo/production account identities, reconstructs the routed fault config from
    the exact official-demo/proxy files, reopens the manifest and every config
    after reconstruction, requires unique exact-proxy typed evidence for every
    proxy-supported fault, requires each typed command interval inside its
    reverified live session, and rejects reuse of the dedicated soak as a fault
    session. It also reconstructs one raw schema-2 proxy process report per role,
    requiring exact config/build/host, unique sessions, independently derived
    clean shutdown, exact completed-command counts, and unambiguous live-session
    enclosure. Mandatory schema-8 age windows have hard maxima of 15 minutes for
    production account state, 24 hours for soak/fills/bills, seven days for
    fault/latency/deadman/emergency evidence, and five minutes of future
    tolerance. Schema 8 also predeclares the exact approval-policy SHA-256. The
    checked-in schema template cannot pass, no target-host bundle
    or human approval exists, and the output explicitly leaves production order
    entry unauthorized.
53. Production approval is now a separate asymmetric gate. The strict policy
    requires at least two sorted roles and distinct Ed25519 keys; request
    preparation accepts only a fresh passing schema-8 bundle, offline signatures
    bind exact request/policy/role/time, and final verification reruns all sources
    before requiring role coverage. Requests have a hard 15-minute lifetime,
    private keys require owner-only files, every input is reopened, and all output
    still reports production entry unauthorized. No real approver policy, signed
    target-host request, or passing approval verification exists.
54. A schema-1 authenticated account-wide bill collector retains exact
    `/api/v5/account/bills` pages for one closed window, brackets them with
    exchange-clock/account-identity samples, and proves a bounded cursor chain to
    a short terminal page. Its credential-free verifier reopens and re-hashes
    every source. `reconcile-economics` then independently verifies the guarded
    fill collection and exact bill collection, leases and streams the stopped
    journal, rejects every unexplained bill, validates normal trade identities,
    fees, currencies, and balance equations, and recomputes linear/inverse
    funding from a session-local journaled realized rate, the latest signed
    position at bill `fillTime`, and public marks on both sides of that assessment.
    A schema-7 runtime-session record emitted per account on every start binds
    the settlement, position, and mark lines to one config and hashed OKX account
    identity, preventing evidence from crossing a restart.
    Funding type `8` subtypes
    `173`/`174` and source fields are mapped to pinned Java `BillDetails`,
    `OkexV5BillFetchTaskImpl`, `OkexV5BillTypes`, and
    `OkexV5ExchBillConverter`, plus Java's dedicated `MarkPrice` session/parser.
    Every authoritative REST replacement also writes a critical account-scoped
    `account_snapshot`; strictly later exact critical fills replay from its `avgPx` under the
    pinned Java linear/inverse basis rules, and derivative bill close PnL is
    independently recomputed. Production evidence schema 8 requires one passing
    economic gate and a nonzero derivative-close count per demo account, and caps
    PnL/mark tolerances. No credentialed artifact exists, and cash-spot bill units
    plus target-account position-basis behavior remain empirical gates.
55. Schema-2 account certification retains current-wire `eqUsd` and an exact raw
    direct `CCY-USD` public index response for every non-USD balance currency.
    Offline verification rejects stale/missing conversion evidence, checks each
    `eqUsd = eq * idxPx` within tolerance, and tightly checks `sum(eqUsd) =
    totalEq`. Schema-5 economics requires separately verified opening/closing
    certifications around the bill window, numeric bill ordering, every `bal`
    post/pre link, and per-currency `opening cashBal + sum(balChg) = closing
    cashBal`. Production evidence schema 8 binds both artifacts to the demo
    config/build/host/account and caps each boundary gap at 60 seconds. No
    credentialed target-account boundary pair exists.
56. Live strategy configuration now requires an explicit reference-data maximum
    age. One venue-neutral requirement set drives both strategy validity and the
    OKX public plan: price limits for spot and derivatives, mark prices for
    derivatives, funding rates only for swaps, and every configured index.
    Startup uses source timestamp versus actual receive time, retains each
    component clock independently and monotonically, and cannot become ready on
    an old retained frame. Losing a source blocks entry and immediately
    synthesizes canonical account-wide cancels; the pure strategy also withdraws
    stale quotes on replay/live timer events. This preserves the pinned Java
    subscriber's separate session topology while intentionally avoiding the
    shared mutable `Ticker.timeMs` freshness mask.
    A 30-second credential-free OKX diagnostic on 2026-07-15 targeted six
    spot/swap book and non-index reference streams across four planned sockets:
    888 accepted frames, including 87 spot and 90 swap price-limit frames, 137
    mark-price frames, and one funding-rate snapshot. Strict verification and
    replay found zero duplicates, gaps, recoveries, timestamp regressions, or
    parse errors, and both books ended ready. The index stream was outside this
    targeted run and the host guard was disabled, so this confirms current
    compatibility of the newly required channels only and is not production
    evidence.
57. Redundant books now keep sequence and full-book state per websocket
    `conn_id`, matching pinned Java `AbstractOkV5L2Subscriber` ownership by
    session type, connection index, and symbol. Global duplicates still advance
    each source; only valid reconstructed books reach canonical arbitration. An
    equivalent same-version replica is ignored, a different full book fails
    closed, and a source-local predecessor gap restarts only that socket while a
    ready replica remains available. The change was prompted by a guarded
    60-second diagnostic in which one replica's newer startup snapshot arrived
    milliseconds before the other replica's delta ending at the same sequence;
    shared sequence state incorrectly classified the lagging valid delta as a
    gap. Replaying all 9,435 persisted frames with per-source state reduced that
    run from one gap/recovery to zero without changing its 4,757 accepted and
    4,678 duplicate classifications.

    A fresh optimized-release 60-second run on 2026-07-15 covered all 11
    configured logical spot/swap book, trade, index, stablecoin, price-limit,
    funding, and mark streams from two sources each. All 12 socket plans were
    ready at stop, both 400-level books were ready, six periodic
    disk/memory/clock checks passed, and 6,749 exact ordinals contained 3,397
    accepted plus 3,352 duplicate events with zero disconnects, gaps,
    recoveries, stale books, parse errors, or missing recovery routes.
    Independent strict verification and replay passed at raw SHA-256
    `a0d5513af66cb8b82aa811fbf6358d7b7f1a5a0187c7adcf6ae49fb7770f1de2`,
    report SHA-256
    `34c41c04c01e4596ad93c658b11defd6f398592f129891f13eef6ef95be9c17c`,
    and executable SHA-256
    `e0d912477aea711ae77e9e2026e99107925a6f387a96f2a27746f216e5d8c265`.
    That executable hash identifies the diagnostic capture; it is not the later
    rebuilt candidate after the report-only backtest horizon correction.
    Raw backtest reconstructed 4,555 inputs, reached terminal order-entry
    readiness, modeled 22 orders, and passed complete accounting with no fills.
    That replay also exposed a 2.77 ms trailing duplicate/control-frame horizon:
    risk integrals already closed at the final raw receive timestamp while the
    reported duration stopped at the last normalized event. The denominator now
    uses the same raw horizon and fails closed if inventory-open time exceeds it;
    rerun inventory duration and observed duration both equal
    `58,463,130,332` ns and the fraction is exactly `1.0`.
    The diagnostic copy retained the 5 GiB disk and synchronized-clock guards
    but used a 700 MiB memory floor for this 1.8 GiB shared workspace host; the
    checked-in config remains at 1 GiB. The short public run does not cover a
    funding settlement, calibrated execution, credentials, target-host latency,
    or production authorization.
58. A subsequent optimized-release 300-second public run on 2026-07-15 covered
    the same 11 logical streams from two sources each. All 12 socket plans and
    both books were ready at stop; 29 periodic host checks passed. Its 42,968
    exact raw ordinals contained 21,522 accepted and 21,446 duplicate events,
    with zero disconnects, gaps, recoveries, recovery failures, parse errors,
    stale books, or missing recovery routes. Strict report verification,
    replay, capture analysis, and complete-accounting raw backtest all passed.
    The report, raw, normalized, and executable SHA-256 values are respectively
    `d1b635897ca2e79cce163b7883af85a0977a2f522e4ab09aabcc11fa14955e37`,
    `77184d0cc436dfa5d5e5952026a18111d0a6a219f0960773a61a2f69f15991a3`,
    `5ddb3d134058366cd1304cf95b662f95c461d63486525bc68853953c838e60fa`,
    and `c0c213afb28294282c1564f8489b443125161a3bd49c438ae65bda77bbc745ee`.
    The last hash identifies the capture binary before the policy-enforcing
    rebuild and is retained only as provenance for this diagnostic run.
    Backtest reconstructed 27,420 inputs over `298,885,567,638` ns, modeled
    112 orders and 108 cancels, observed seven funding messages and one funding
    settlement, and closed observed and inventory-open duration at the same raw
    horizon. It still produced zero fills and ended at the data boundary with
    four live orders and two scheduled actions, so it does not validate fills,
    fees, queue assumptions, or shutdown behavior.

    This diagnostic again used a 700 MiB memory floor because the shared host
    had less than 1 GiB available. Production research now rejects such a
    configuration in code: capture host guards must be enabled, check no slower
    than 10 seconds, retain at least 5 GiB disk and 1 GiB memory floors, and
    require synchronized-clock enforcement. The run strengthens public
    connectivity and deterministic replay evidence but remains deliberately
    inadmissible for production-candidate research. The exact demo-to-production
    transition now applies the same host floors and additionally requires fatal
    alerts, the operator service, redundant public and order-command sessions,
    and an absolute shared pacer in both configs, so the aggregate bundle cannot
    admit the checked-in development defaults.
59. A source-level connectivity audit against pinned Java
    `OkxNitroSubscriberBase`, `SharedSessionSubscriberBase`, `WsConnectOption`,
    and the Netty/Vtx websocket clients found that Rust created one recovery
    watch channel per socket but retained its sender only for book routes. A
    non-book receiver could therefore be closed from startup and leave an
    always-ready branch in its connection loop. Every socket now retains an
    owner for the supervised-feed lifetime, and unexpected route closure is a
    fatal invariant violation. Feed connection establishment is also bounded to
    10 seconds and cancellable by shutdown/recovery; bootstrap, subscription,
    heartbeat, and close writes are bounded to 5 seconds. The existing
    15-second OKX application ping and independently aged strategy/account
    components remain separate transport and data-health contracts. Focused
    connection deadline, route-ownership, and fatal-classification tests plus
    all 63 `reap-feed` tests pass.

    A fresh optimized-release 60-second diagnostic then exercised all 11
    configured logical streams from two sources across 12 socket plans. All 12
    were ready at stop, both books were ready, and six host checks passed. Its
    5,647 exact raw ordinals contained 2,835 accepted and 2,812 duplicate events
    with zero disconnects, gaps, recoveries, recovery failures, route misses,
    parse errors, or stale books. Strict report verification, replay, and
    capture analysis passed. Raw backtest reconstructed 3,991 inputs, modeled
    18 orders and 14 cancels with no fills, and passed accounting, currency-rate,
    and final-valuation checks. It retained four live orders and two scheduled
    actions at the data boundary and reported one clamped 1,330 ns global
    cross-task receive-stamp regression; per-source timestamps remained
    monotonic. Report, raw, normalized, and executable SHA-256 values are
    respectively
    `09ceb119d787f3cd1bca81261c45f94361007e6fe9cbc6fe806392bdb6911f5b`,
    `782f518e0edf7a4f996eb6aa14e21118672091dbf5fbf11449209a4c286285a7`,
    `f68359e35795226b1f7a294b8c69be74bc4290c5e2571a9e94e8a2a8ca2ac1dd`,
    and `58cc2e5bb1bd942965cd4d221c591a0f3ce88710fb3478f5a951d0addc34fa0e`.
    The diagnostic used a 700 MiB memory floor because this shared host cannot
    reliably retain the checked-in 1 GiB production floor. It closes a real
    local lifecycle defect but is not production evidence and does not replace
    credentialed demo fault campaigns.
60. The next source audit followed pinned Java
    `OkxNitroOrderClient.onOrderSessionDisconnected`, which immediately invokes
    `cancelAll()`, through Rust's eight-session command transport and demo fault
    proxy. Rust heartbeat status previously used awaited sends at both bounded
    telemetry queues, so a stalled observer could stop a command session before
    it processed shutdown or delivered the disconnect that triggers canonical
    cancellation. Heartbeats are now best-effort at both boundaries, while
    ready, disconnect, and fatal transitions remain lossless; the order-session
    close uses the existing 5-second control-write deadline.

    The fault proxy also had unbounded local/upstream websocket handshakes and
    forwarding writes, and protocol `?` returns could bypass active-connection
    unregister. Both handshakes and every write now use the configured request
    deadline and observe shutdown, paired close writes are capped at one second,
    and every registered bridge exits through unregister. Shutdown cancellation
    is clean rather than counted as an injector error. Focused order-session
    saturation/lifecycle tests, stalled-proxy-handshake tests, the existing
    disconnect/drop integration test, and focused lint all pass.

    An optimized-release smoke held a raw TCP client established on the public
    websocket listener without completing its handshake through a bounded proxy
    run. The first independent verification exposed that relative and canonical
    proxy-config paths produced different effective fingerprints. The loader now
    rejects symlinks/non-files and canonicalizes the source before resolving
    artifact paths. Regression tests cover aliased paths and symlinks. Repeating
    the smoke with the documented relative config path produced session
    `fault-1665842-25ea16f8ad4155d3`: it stopped after 20,001 ms with clean task
    joins and control-socket removal, zero active bridges, and zero proxy errors.
    Independent verification passed; report and executable SHA-256 values are
    `c82854c4ce6952ecf1e066c986dcb3b1411eae326f92743178797edb60ae9f05`
    and `39ecc4be4e10b73673b17b82c3b7dc9bf73df99fc63ef4145c3aa586ea71d65b`.
    The routed three-websocket demo config also passed `live --mode validate`.
    Workspace formatting, warnings-denied Clippy, all 725 tests, the optimized
    release build, systemd unit verification, and RustSec audit pass.
    No configured demo credential, operator-token, or alert-endpoint environment
    binding is present on this host, so this is implementation evidence rather
    than the still required credentialed observe/fault campaign.
61. Runtime capture status previously required socket and book readiness but
    did not require non-book subscriptions to appear, even though strict
    offline analysis rejected an absent trade, index, stablecoin, funding,
    mark, or price-limit stream. Runtime and verifier clean derivation now share
    one stream identity rule and require every configured logical stream to
    contain data from exactly its configured source count plus at least one
    accepted event. Unclassified and unexpected data also fail closed. Capture
    config loading now rejects symlinks/non-files and files over 16 MiB,
    canonicalizes the source, and binds that canonical path to exact byte
    evidence. Focused runtime, analyzer, verifier, path-alias, symlink, and size
    tests pass.

    A fresh optimized-release 60-second public diagnostic exercised all 11
    configured logical streams from two sources each. The runtime
    `--require-clean-capture` gate passed with all 12 socket plans ready, both
    books ready, six host checks, and 5,229 exact raw ordinals containing 2,625
    accepted plus 2,604 duplicate events. There were zero disconnects, gaps,
    recoveries, recovery failures, stale books, parse errors, timestamp
    regressions, unexpected streams, or missing recovery routes. Every stream
    had two observed source connections; even the low-frequency funding stream
    had four data frames and two accepted events. Independent verification,
    strict analysis, and strict replay passed. Raw-capture backtest reconstructed
    3,776 inputs, reached order-entry readiness, modeled eight orders and four
    cancels, and passed accounting and currency-rate checks with no fills.
    Report, raw, config, and executable SHA-256 values are respectively
    `c64b8405b17a97f90b82d61eb5eb251cded0f56648f589fedc5ddbc8cfd44bce`,
    `f91aca6164366ebe3edc205648a4e587663974f23bab0df0d89513f42124981d`,
    `20d98819819877c9fa4725f990e37189deb9508c81fcaf776742b7ede764ac47`,
    and `7df01637d84e3ee31235732535ecca43f5b9bc2ca06c6579099d73be0449f024`.
    This shared host had only about 5.20 GB free disk and therefore used a
    diagnostic 4 GiB disk floor plus 700 MiB memory floor; the checked-in 5 GiB
    and 1 GiB production floors remain unchanged. The uncalibrated no-fill run
    is implementation/connectivity evidence, not production authorization.
    Workspace formatting, warnings-denied Clippy, all 731 tests, the optimized
    release build, systemd unit verification, and RustSec audit pass.
62. Capture redundancy previously compared only the number of distinct
    `conn_id` values observed for each stream. Two wrong internal sources could
    therefore satisfy a two-replica count even though they were not the socket
    plans generated for that channel, priority, replica, symbol chunk, and
    exact config. Runtime and offline analysis now derive the same expected
    source set directly from deterministic `partition_subscriptions` plans and
    require exact set equality. Format-5 analysis records each stream's
    expected, missing, and unexpected plan IDs. This strengthens the mapping to
    Java `OkxNitroL2SubscriberGroupFactory` groups and
    `AbstractOkV5L2Subscriber` connection-index ownership while adding explicit
    retained evidence that Java does not emit. Focused tests prove correct
    plans, capacity-forced symbol chunks, incomplete partition rejection,
    count-equivalent wrong plans, and independent verifier rejection.

    A fresh optimized-release 60-second public diagnostic passed the runtime
    exact-plan gate across all 11 logical streams and 12 socket plans. Its 5,674
    exact raw ordinals contained 2,859 accepted and 2,815 duplicate events;
    both books were ready after six host checks, with zero disconnects, gaps,
    recoveries, recovery failures, stale books, parse errors, or missing
    recovery routes. Independent verification, strict replay, and strict
    format-5 analysis passed. Every stream listed both exact expected plan IDs
    with empty missing/unexpected source lists. Raw backtest reconstructed 3,975
    inputs, reached order-entry readiness, modeled eight orders and four
    cancels, and passed accounting/currency-rate checks with no fills. It
    reported two globally clamped cross-task receive-order regressions with a
    maximum of 1,058 ns; per-stream capture timestamps remained monotonic.
    Report, raw, config, and executable SHA-256 values are respectively
    `48c98fe75d11eccc5db17cd5aff934bc887dbfa5f8118e77dbd3ad578d36e5c0`,
    `cdc3f98dd33632d0c4578f3cb0e9ea7aa9b6c8d3ba96e975befe61b1e83b475c`,
    `20d98819819877c9fa4725f990e37189deb9508c81fcaf776742b7ede764ac47`,
    and `38cd09c4b8ffccc1310203df10c9e5a8d19398964e3568c3c973ddd4444cd083`.
    The run reused the diagnostic 4 GiB disk and 700 MiB memory floors because
    this shared host remains below production policy. It is connectivity and
    evidence-pipeline validation, not production authorization.
    Workspace formatting, warnings-denied Clippy, all 735 tests, the optimized
    release build, systemd unit verification, and RustSec audit pass.
63. Feed reconnect backoff previously increased after every failed socket run
    but never reset after the exact subscription acknowledgement set had made a
    session ready. Historical startup failures could therefore leave a later
    healthy-session disconnect waiting the 30-second maximum. The supervisor
    now receives an explicit run outcome, keeps exponential backoff for repeated
    pre-readiness failures, resets to 250 ms after exact readiness, and still
    reserves every attempt through the process-shared OKX pacer. This preserves
    the fixed per-disconnect lifecycle in pinned Java
    `SharedSessionSubscriberBase` while retaining Rust's stricter storm control.

    Private socket payload forwarding also previously awaited bounded channel
    capacity without a deadline. A stalled event loop could therefore trap the
    websocket reader indefinitely, preventing ping/pong, reconnect, and prompt
    shutdown. Private delivery now waits at most one second, then emits a typed
    non-fatal disconnect. The live coordinator revokes private readiness,
    cancels active orders, and requires clean REST reconciliation after
    recovery. The exact transport error is retained in the journaled stale
    event instead of being reduced to a generic cause. Focused tests cover
    bounded no-silent-drop behavior, non-fatal supervisor classification,
    readiness-reset backoff, exact-ready provenance, and public/private journal
    reason propagation.

    A fresh optimized-release 45-second public diagnostic exercised the exact
    ready path with all 12 socket plans ready and both books healthy. It
    persisted 3,553 exact ordinals containing 1,788 accepted and 1,765 duplicate
    events, with a maximum raw queue depth of 23 and zero disconnects, gaps,
    recoveries, recovery failures, stale books, parse errors, or missing
    recovery routes. Independent capture verification and strict replay passed;
    all 11 streams had their exact two planned sources. Raw backtest rebuilt
    2,624 inputs, reached entry readiness, modeled 12 orders and eight cancels,
    and passed complete accounting/currency coverage with no fills. Report,
    raw, config, and executable SHA-256 values are respectively
    `08d9dbbc3b42d8009c5d3417c79a576ea01d16695b5572d6b51fe0cdedb0e5e4`,
    `a2cff3abc86642ea8b01b098c09014cefcc6c78c9d9b7801162507d3cdf18aad`,
    `20d98819819877c9fa4725f990e37189deb9508c81fcaf776742b7ede764ac47`,
    and `bed4d0d501d794d259b7b11279b50d6c2d2d7d809a283920369e5cb1f20e82b9`.
    This shared host again used diagnostic 4 GiB disk and 700 MiB memory floors;
    production floors remain unchanged. Formatting, warnings-denied Clippy, all
    736 tests, optimized release build, hardened systemd verification, and
    RustSec audit pass. The private-overload path is deterministic local
    evidence and still requires a credentialed target-host demo campaign.
64. Capture persistence now has explicit application-level lifecycle deadlines.
    Raw and optional normalized enqueue waits fail after one second instead of
    dropping a frame or blocking the event loop. Supervised feed shutdown/drain
    and host-guard shutdown are capped at five seconds, writer flush/sync at 30
    seconds, post-timeout abort wait at one second, and best-effort partial-file
    evidence scanning at five seconds. Writer and host shutdown run concurrently;
    the hardened collector unit retains its 45-second external hard stop.

    Effective config, duration, and canonicalized prospective report/raw/
    normalized path identities now validate before report reservation. Handled
    setup or runtime failures produce runtime_failure, bounded stable code and
    message, and clean_capture = false; pre-existing output bytes are never
    adopted as this run's evidence. Independent capture verification and
    production research reject a reported failure even if the clean flag is
    forged. This strengthens pinned Java ChaosWriter/AsyncBufferedWriter:
    Java owns append/flush/close on ioInbox but only logs IO exceptions and
    delays production close by three seconds, whereas Rust makes capture storage
    failure typed, bounded, durable, and inadmissible.

    Release failure smokes proved invalid config creates neither report nor raw
    output, while an impossible host floor produced a mode-0600 non-clean
    host_guard report before nonzero exit and left an older raw file byte
    identical. The failure report and preserved raw SHA-256 values were
    769fa5f0ff203f7fa63ba9f7d001e5e1478a84a1d36469a3490cea7b8e641151
    and
    effd5be4724920be6970ab5a76a733314ac3c591458c0e8effd2aa87605e1793.

    A fresh optimized-release 45-second public run then proved the success path.
    All 12 socket plans and both books ended ready; 3,096 exact ordinals contained
    1,599 accepted and 1,497 duplicate events with raw queue high-water 23 and
    zero disconnects, gaps, recoveries, recovery failures, stale books, parse
    errors, missing routes, or timestamp regressions. Independent verification,
    strict format-5 analysis, and strict replay passed with all 11 streams
    complete from both exact sources. Raw backtest rebuilt 2,431 inputs, reached
    entry readiness, emitted six orders and two cancels, and passed complete
    accounting with no fills. Report, raw, diagnostic config, analysis, and
    executable SHA-256 values are respectively
    6b827391d1e0458fb9dd847fa199c5f3096ec62bd7f87357f64d7953cd6aea43,
    55c3ee51d841c0cb770d9438b218382a1b49604acc56aa49854b247774155013,
    9a46596af8f6e88c915ef886209b8100f7fabffb12b3f1ae95ebd1a16c2e0242,
    38c6b51e1f3d5133eddb748129d9b2d2a2ecf984807ec0819ee7ddd6b9558ae4,
    and
    10ee4903b059c6c27b16431fc150b992732ab340b12c219a86f85b3cbf0d6fad.
    The shared host required diagnostic 256 MiB disk and 512 MiB memory floors,
    so this is pipeline evidence, not production qualification. Formatting,
    workspace warnings-denied Clippy, all 748 tests, optimized release build,
    hardened systemd verification, and RustSec audit pass. Credentialed
    target-host demo, economic, calibration, and approval gates remain open.
65. Live process teardown is now application-deadline-bound separately from
    order safety. `runtime.shutdown_timeout_ms` retains 15 seconds for canonical
    cancel plus authoritative reconciliation; the new
    `runtime.teardown_timeout_ms` gives all host, operator, feed,
    order-command, command/reconciliation/safety, storage, and alert owners one
    additional 15-second budget. Every owner is signalled before joins. If the
    budget expires, cancellation-safe owner drops abort remaining tasks, remove
    the operator socket, release the journal lease, preserve Cancel All After,
    and produce stable `teardown_timeout` non-clean schema-8 evidence. A normal
    journal close now flushes and calls `sync_data`.

    Production-evidence config policy limits the two in-process deadlines to 40
    seconds under the checked-in 45-second systemd stop boundary, preserving at
    least five seconds for report serialization, file/directory sync, and exit;
    enabled alert drain must fit inside the teardown deadline. A deterministic
    stalled-task regression returned the typed report inside 25 ms, observed
    cancellation, and reacquired the aborted writer's journal lease. Targeted
    warnings-denied Clippy and 339 feed/live/storage/telemetry tests pass.
    Workspace formatting, warnings-denied Clippy, all 751 tests, optimized CLI
    build, release config-validation smoke, hardened systemd verification, and
    RustSec audit also pass. The release executable SHA-256 is
    `1b4b3e7ed3647c05c8870b45136ba2332b94abbb61ec50e799bb91ca4bfa3cf3`.

    This follows pinned Java `ChaosStrategyBase.doStop` ordering across quoter,
    writer, timer/subscription, context, and calculator release, and
    `ChaosStrategyEngine.clear` dispatcher ownership. Java schedules delayed
    `ChaosWriter.close` work on `ioInbox` without a versioned whole-runtime
    completion deadline; the Rust deadline and failure report are explicit
    lifecycle hardening rather than claimed parity. Credentialed target-host
    process-death and supervisor evidence remains required.
66. Live process preparation now validates the exact config and run options and
    captures source-file, executable, and optional host provenance before the
    CLI reserves a report path. Invalid configuration and zero duration create
    no report. A handled failure after reservation but before a reportable
    runtime session writes a schema-8 diagnostic report with no session ID,
    account identities, host-health observations, or runtime counters, baseline
    readiness, typed failure evidence, and `clean_soak = false`. Its message
    explicitly states that zero counters are not exchange-zero proof. Offline
    verification accepts this shape only as diagnostic evidence and can never
    derive clean-soak acceptance; forged runtime state or session promotion is
    rejected.

    Raw feed, order, reconciliation, order-status, and safety task handles remain
    in abort-on-drop startup groups until ownership transfers into
    `LiveRuntime`. This closes the detached-worker path when a later account
    setup step fails. The lifecycle maps to pinned Java
    `ChaosStrategyEngine.init/checkReadiness/start`,
    `ChaosStrategyBase.onReady`, and `ChaosStrategyEngine.clear`, while the
    versioned pre-session artifact and explicit task ownership are Rust
    hardening rather than parity claims.

    Process tests prove invalid config and zero duration leave no output and an
    impossible Linux host floor writes a mode-0600, source-bound, independently
    verifiable non-acceptance report. An optimized release smoke with missing
    operator credentials produced the same contract and preserved the original
    nonzero error; its report SHA-256 was
    `365a8bc63bd952704de08b92694e35f3e04c5487720e343043bbdfe165d1c947`.
    Formatting, warnings-denied Clippy, all 757 workspace tests, optimized CLI
    build, release validation/startup smokes, hardened systemd verification at
    exposure 2.9, and RustSec audit pass. The release executable SHA-256 is
    `1cef2b7e423a58dc36b3a823dc37c817e12a46a62fd0f11f51e38cf8c72dda83`.
    Credentialed target-host demo, process-death, economic, calibration, and
    approval gates remain open; this does not make the strategy tradable now.
67. Authenticated account configuration now retains the requesting API key's
    label, normalized `perm` set, and normalized `ip` bindings. Each account
    declares an exact permission policy; configuration always rejects
    `withdraw`, production trade configuration requires IP binding, bootstrap
    rejects permission or binding mismatch before task startup, and the
    periodic account-config comparison makes later key-security drift fatal.
    Production-qualified demo and production configs must declare exactly
    `read_only` plus `trade` and require a binding.

    Account certification is schema 3 and independently re-parses the exact raw
    before/after responses, re-derives permission equality and binding presence,
    and rejects a re-hashed permission tamper. The aggregate production bundle
    already re-verifies that artifact against the exact production config. This
    strengthens pinned Java `AccountConfig`/`OkUtils.loadAccountConfig`, which
    retain account/position mode but not current `perm`/`ip` fields.

    Formatting, warnings-denied Clippy, all 761 workspace tests, optimized CLI
    build, hardened systemd verification at exposure 2.9, and RustSec audit
    pass. The release executable SHA-256 is
    `708f299f4b9ca3a21bec14d5eba9ef5b395d184d62e0f714be2d4aa37064b62b`.
    No target-host credential artifact exists, and production order entry is
    still unavailable.

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
3. Run minimal-size `demo` orders, then inject public, private, and order-command
   socket disconnects, process kill,
   deadman expiry, exchange-clock skew, IOC miss, partial fill, and REST
   timeout/rate-limit conditions. Inject a non-live instrument, changed tick,
   and imminent `upcChg` response through the fault proxy. Suppress submit and
   cancel order pushes to
   exercise order-state convergence, then suppress derivative position updates
   and each side of a spot balance update to exercise fill convergence. Verify
   cancel retry, `expTime`, and latch restoration from exchange/account
   evidence. In one controlled process-death iteration, do not issue a cancel,
   wait for expiry, and archive a passing `certify-deadman-expiry` artifact plus
   `verify-deadman-certification --require-pass` result. In a separate forced-
   death iteration, exercise the independent emergency command and archive its
   zero-order report plus a passing `verify-emergency-cancel --require-pass`
   result with `--require-all-configured-accounts`; incident cancellation must
   never wait for certification.
   Populate `examples/live-fault-matrix.toml` with the isolated reports and
   injector records, then require `verify-live-fault-matrix --require-pass`.
4. Complete a sustained soak with zero unexplained order, fill, balance,
   position, or checkpoint drift. `clean_soak` covers runtime readiness,
   full-state reconciliation, storage drops, alert delivery, and shutdown
   orders; restart checkpoint state still requires log/account review. Generate
   a passing `calibrate-latency` artifact from synchronized target-host observe
   and demo reports, archive every source hash/file, require a passing
   `verify-latency-calibration` reconstruction, and reconcile its private timing
   populations against exchange/account records. Run `collect-fills` with a
   leading trade-bill delay guard and `collect-bills` for the exact closed
   bounded-demo window. Require passing manifest-backed `reconcile-fills`,
   `verify-bill-collection`, and schema-5 `reconcile-economics` artifacts with reviewed
   nonzero trade, derivative-close, fill, and funding thresholds. Start the
   controlled trade window only after the first schema-7 authoritative account
   snapshot, span a real nonzero funding settlement, retain same-session
   `mark-price` samples before and after the bill `fillTime`, and contain no
   unrelated account bills. Collect passing schema-3 account certifications no
   more than 60 seconds before and after the exact bill window, then inspect every
   per-currency bill-balance link and endpoint equity conversion.

## Production Gate

Production enablement additionally requires:

- A passing owner-only `verify-production-evidence --require-pass` artifact made
  by the exact candidate binary on the declared target host after every source
  artifact below exists. Review every subordinate artifact and schema-8
  freshness observation; the aggregate does not supply remote attestation,
  external supervision, remote attestation of local position-basis evidence,
  total-equity attribution beyond the controlled bill/cash window, authenticated
  external partial-fill/restart causality, or human approval and never
  authorizes entry.
- A passing owner-only `verify-production-transition --require-pass` artifact
  binding the exact demo config used by the accepted evidence to the exact
  production candidate. Review every allowed change, independently verify
  production credentials/account identity, and reject any disallowed drift.
- Full-depth historical data and calibrated queue, latency, fee, funding, and
  slippage assumptions, including empirical per-message/per-instrument delay
  distributions and empirical validation of the displayed-depth fill threshold.
  Latency requires a passed source-bound calibration artifact; the implemented
  websocket-order-ack matching measurements must remain labeled and approved as upper
  bounds unless a closer exchange boundary is added.
- Authenticated fee-group startup/periodic checks must pass for the exact
  target account and symbols. The configured rates must then be calibrated to
  the reviewed tier and reconciled to nonzero fills; a conservative safety
  bound alone is not profitability evidence, and OKX states that API rates may
  omit temporary zero-fee treatment.
- Authenticated exact-instrument startup/periodic checks must pass for every
  target symbol with no state, sizing, single-order maximum, valuation,
  currency, family, or fee-group drift and no change inside the configured
  announcement lead. Demonstrate that a final post-only and aggregated IOC over
  `maxLmtSz` or applicable `maxLmtAmt` is rejected before transport. Archive a
  demo proxy fault proving both drift and announced changes produce typed
  fail-closed cleanup while the deadman heartbeat remains active.
- A passing target-account `certify-account` artifact, independently rechecked
  with `verify-account-certification --require-pass`, immediately before
  approval. Require exact `read_only` plus `trade`, no `withdraw`, and a
  non-empty exchange-reported IP binding for the key accepted from the target
  host. This is point-in-time evidence and must be combined with a passing
  per-account schema-5 economic reconciliation included in the schema-8
  production bundle. Margin spot remains unsupported; enabling it requires an
  explicit borrow-rate/interest model and demo reconciliation first.
- A passing per-account `reconcile-economics --require-pass` reconstruction from
  exact guarded fills, account-wide bills, and the stopped canonical journal.
  Require the reviewed nonzero derivative-close threshold and inspect the
  same-session REST snapshot/fill/PnL samples and all opening/closing cash links.
  Independently review a credentialed cash buy/sell sample before admitting spot
  semantics. Review the funding mark bracket width;
  it constrains accepted bill marks but cannot reproduce the venue's internal
  assessment tick.
- A passing target-host demo `certify-deadman-expiry` artifact, independently
  rechecked against the exact stopped journal with
  `verify-deadman-certification --require-pass`, plus separate supervisor/fault-
  injector and emergency-cancel evidence, including a passing exact-config
  `verify-emergency-cancel --require-pass` result with
  `--require-all-configured-accounts`.
- Sustained redundant direct currency/USD index coverage for every non-USD
  accounting currency, with zero conversion failures and fee/cash/funding/equity
  reconciliation against target-tier demo statements.
- Completed `production_candidate` walk-forward and out-of-sample manifests
  using calibrated assumptions, sustained captures, parameter sensitivity,
  capacity, inventory-duration, and stressed-liquidity reports. Re-run each
  archived report with `verify-research --require-pass` and retain its passing
  source-bound verification. The runner and verifier are implemented; no
  qualifying report has been produced.
- Target-account calibration and independent exercise of the implemented
  stablecoin guard; either implementation and exercise of external
  strategy-group/master coordination or continued rejection of those settings;
  deployed external alert routing; and target-host exercise of the
  out-of-process account-wide regular/algo/spread kill path.
- Target-host time-service monitoring, CPU/thread placement, bounded
  backpressure, calibrated memory/disk thresholds, installed restart
  supervision, and external unit-failure paging.
- Long-running demo soak with zero unexplained order, fill, position, or balance
  reconciliation drift.
- Explicit operator approval of credentials, account mode, limits, symbols,
  and the production rollout/rollback procedure, represented by a fresh passing
  `verify-production-approval --require-pass` artifact with separate operations
  and risk key custody. Cryptographic role coverage does not replace review of
  approver identity or actual rollback readiness.

The first safe milestone is demo-tradable, not production-tradable. Production
capital should remain disabled until every startup, recovery, and reconciliation
invariant has executable acceptance evidence.
