# Polymarket one-shot production order

`reap-pm-controlled-trial-runner production-place-then-exact-cancel` is an
explicit production command for the small legacy type-1 proxy account already
configured in `../predarb/.env`. It does not copy credentials into Reap or
write them to its ledger.

The command:

- derives only the current `btc-updown-5m-{window_start}` slug and resolves it
  through Polymarket's fixed production Gamma event-by-slug endpoint;
- requires one live, open, accepting BTC Up/Down market with exact Up/Down
  token mapping, tick, minimum size, negative-risk flag, question ID, and
  condition ID;
- verifies both condition-bound CLOB books and chooses whichever outcome has
  the higher ask, then refreshes that exact book after a public position read;
- fixes the trial to a 5-share post-only BUY at 0.01 and requires that price to
  remain at least 0.20 below the fresh best ask with at least 75 seconds left
  in the market window;
- accepts only signature type `1` and verifies the private-key signer, L2
  credential owner, configured funder, and deterministic legacy proxy relation;
- hard-caps and fixes quantity at 5 shares;
- constructs only a GTC post-only order;
- creates and fsyncs one fixed ledger before the only place attempt;
- reserves canonical `pending_new` ownership before that dispatch, using the
  same reserve-before-I/O invariant as the OKX regular-order path;
- never retries or resumes placement;
- precommits the exact deterministic order ID and attempts only that cancel if
  placement was accepted or its acknowledgement is uncertain;
- uses only `POST /order` and exact-owned `DELETE /order`, with a fixed current
  `clob.polymarket.com` peer and selected Linux interface/source address;
- reduces place/cancel acknowledgements, one subsequent authenticated exact
  order read, and exact fill-ID observations through `PmOwnedOrderLifecycle`;
- walks a complete authenticated account trade cut, selects only maker/taker
  legs bound to the deterministic owned order ID, and deduplicates them by
  `PmFillKey`;
- calculates a fill-based position from the pre-order position plus the signed
  total of exact owned fills; and
- polls the public configured-token position independently for up to five
  bounded attempts, requiring it to equal the fill-based position before
  reporting convergence.

The credential file must be a regular owner-held `0600` file with one hard
link. The state directory must not contain the fixed ledger from a previous
attempt; a second invocation against the same directory fails closed.

Inspect the current command surface without sending anything:

```sh
cargo run -p reap-pm-controlled-trial-runner -- \
  production-place-then-exact-cancel --help
```

Before an actual trial, independently review one current CLOB IP, interface,
and local source address. Market identity and order parameters are not CLI
inputs. Then invoke the command with a fresh state directory:

```sh
cargo run --release -p reap-pm-controlled-trial-runner -- \
  production-place-then-exact-cancel \
  --credential-env ../predarb/.env \
  --state-directory /absolute/owner-only/new-trial-state \
  --fixed-peer-ip CURRENT_CLOB_IPV4 \
  --interface-name ens5 \
  --local-source-ip LOCAL_INTERFACE_IP \
  --authorization-phrase I_ACCEPT_TOTAL_LOSS_AND_ONE_REAL_POLYMARKET_ORDER
```

The terminal JSON reports the place and cancel classifications, canonical order
state, exact-order reconciliation, and fill-position reconciliation. An
uncertain place acknowledgement can therefore converge to an accepted,
canceled order without operator inference. `manual_reconciliation_required`
remains true if the canonical state is inconsistent, live, or still missing
exact fill legs; if the exact read cannot settle ambiguity; if the complete
trade cut disagrees with cumulative matched quantity; or if the independently
polled position does not equal the position derived from the pre-order baseline
and exact fills. In that case, do not create a new state directory and place
again.

The Data API position has no atomic fence with the CLOB trade query. It is a
venue-published reconciliation target, not permission to discard a locally
observed fill. A position poll that lags a fill therefore leaves the command
unreconciled and triggers the next bounded read-only attempt. Reap's general
private reducer uses the same accounting rule in `effective_position()`:
published position plus deduplicated fills not yet covered by a later snapshot.

## Order-state parity with OKX

The two products intentionally retain venue-specific reducers and exact PM
numeric types. They nevertheless enforce the same operational lifecycle:

| Invariant | OKX | Polymarket production trial |
| --- | --- | --- |
| Ownership before I/O | Canonical `PendingNew` plus owned regular-order proof | `PmOwnedQuoteAdmission::Admitted` before the place send |
| Accepted submit | Bind exchange ID and become live | Bind deterministic venue ID and become `live` |
| Unknown submit acknowledgement | Pending reconciliation; never resubmit blindly | `AmbiguousOwned`; exact cancel/read allowed, placement never retried |
| Partial/complete fills | Monotonic cumulative quantity plus scoped fill-ID deduplication | Exact monotonic cumulative units plus `PmFillKey` deduplication |
| Cancel acknowledgement | Does not by itself prove zero fill | Terminal cancel remains reconciliation-required until exact order/fill evidence |
| Fill/cancel race | Filled terminal state wins | `filled_race` converges to filled without resurrection |
| Position | Authoritative account snapshot plus deduplicated fills | Pre-order published position plus exact fill-ID deltas, reconciled against a later published position poll |

Canceled and rejected projections retain their remaining quantity, matching the
OKX snapshot convention; terminal status determines liveness. Any terminal PM
order whose cumulative quantity exceeds its known exact fill total stays
reconciliation-required, so an order-status row cannot silently manufacture a
position delta or fee fact.

The runner also exposes a read-only exact-order reconciliation command. It
loads the same protected credential file but constructs no place, cancel, or
signer capability after deriving the credential owner:

```sh
cargo run -p reap-pm-controlled-trial-runner -- \
  production-reconcile-exact-order \
  --credential-env ../predarb/.env \
  --condition-id CONDITION_ID \
  --question-id QUESTION_ID \
  --token-id TOKEN_ID \
  --order-id EXACT_ORDER_ID
```

`cancellation_verified` is true only for a present, identity/scope/maker-bound
order whose exact venue status is one of the reviewed canceled spellings.
