# Polymarket and OKX execution infrastructure summary

Updated after the Polymarket production execution infrastructure and
credential-owner lifecycle work landed in commits `4d38e80` and `a6798a3`.

## Current status

The repository contains strategy-neutral execution infrastructure for a
condition-scoped Polymarket BTC Up/Down five-minute market. It is not itself an
operator-runnable strategy. The strategy, public-feed composition, market
rollover, monitoring, and deployment process remain application-level work.

The Polymarket runtime now provides:

- fixed-scope authenticated HTTP and private user-WebSocket roles;
- purpose-specific server-time and L2 authentication;
- post-only placement and exact-owned cancellation;
- durable order intent/state journaling and restart recovery;
- private WebSocket order/trade observations;
- complete polling of open orders, trades/fills, and both outcome positions;
- fill-ID and order-leg deduplication;
- an independent fill-derived position projection;
- reconciliation against authoritative polled Polymarket positions;
- readiness gates that remain closed across incomplete or contradictory cuts;
- fixed heartbeat/deadman supervision;
- exact-owned cancellation and terminal zero-open-order shutdown; and
- composite production lifecycle ownership for the trading supervisor and
  credential-authority task.

The composite owner is [`PmProductionExecutionRuntime`](../crates/reap-pm-live/src/production_execution_runtime.rs).
It performs supervisor shutdown first, then bounded credential-authority
shutdown, and preserves cleanup evidence when either stage fails.

## Main venue differences

| Concern | OKX | Polymarket |
| --- | --- | --- |
| Private state | Private WebSocket includes account, orders, positions, and optional fills | User WebSocket supplies order/trade observations; position authority remains polling-based |
| Mutation transport | Dedicated private order-command WebSocket for place/cancel | Authenticated HTTP place/cancel path |
| Position handling | Private stream plus REST position snapshot | Local fill projection must converge with an authoritative position poll |
| Order identity | Instrument, client order ID, and venue order ID | Signed token/condition/maker/funder facts derive deterministic order identity |
| Authentication | API key, secret, and passphrase login | L1/L2 credentials, EIP-712 signing, request authentication, and server-time proofs |
| Deadman | Native `cancel-all-after` plus private-session lifecycle | Authenticated `/v1/heartbeats` order-heartbeat protocol |
| Scope | General instrument/account APIs | Intentionally condition/token scoped for the active event |
| Rollover | Instruments are normally persistent | Every five-minute condition must be retired, reconciled, and replaced |

## Shared safety invariants

Both venues use the same important execution invariants:

1. durable ownership is established before mutation I/O;
2. placement is never blindly retried after an ambiguous result;
3. fills are monotonic and deduplicated by venue-specific identity;
4. order status cannot invent a fill or position delta;
5. exact-owned cancellation is retained through reconciliation;
6. readiness closes on stale, incomplete, contradictory, or out-of-scope data;
7. startup repairs durable state before admitting new placement; and
8. shutdown cancels only owned orders and requires a terminal reconciliation
   cut.

## Strategy boundary

The execution layer can expose a common place/cancel/order-state interface, but
strategy policy must remain venue-specific.

Polymarket strategy policy must account for binary settlement, complementary
Up/Down tokens, time-to-expiry, token inventory, payout economics, thinner
liquidity, and slower REST/poll confirmation. OKX policy instead reasons about
continuous instrument exposure, contract/notional risk, leverage/funding,
private-WS latency, and persistent instruments.

In particular, the PM user WebSocket must not be treated as an OKX-equivalent
authoritative position stream. It is a low-latency observation source; polling
is the authoritative position repair boundary.

## Remaining production work

The infrastructure is complete for the current scoped execution design. The
remaining work before unattended production strategy operation is:

- compose the runtime with public PM books and the OKX reference feed;
- provide a reviewed PM-specific pricing, inventory, expiry, and risk model;
- implement market rollover and retired-condition position handling;
- add operator telemetry and alerting for poll age, convergence, fills,
  heartbeat, and credential cleanup;
- perform target-host reconnect, partial-fill, restart, and SIGTERM soak tests;
- configure deployment-level secret injection and process supervision; and
- retain the explicit production-order-entry gate until those operational and
  strategy checks pass.

