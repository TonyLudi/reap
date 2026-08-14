#!/usr/bin/env bash
set -euo pipefail

usage() {
    echo "usage: $0" >&2
    echo "Runs the Polymarket place -> fill/position -> exact-cancel workflow against local loopback fixtures." >&2
}

if [[ $# -ne 0 ]]; then
    usage
    exit 64
fi

script_directory=$(cd -- "$(dirname -- "${BASH_SOURCE[0]}")" && pwd -P)
repository_root=$(cd -- "$script_directory/.." && pwd -P)
cd -- "$repository_root"

if [[ ! -f Cargo.toml || ! -f Cargo.lock ]]; then
    echo "error: repository root does not contain Cargo.toml and Cargo.lock" >&2
    exit 65
fi

# This script is evidence-only. Make accidental live configuration unavailable
# even when the invoking shell has sourced a local trading environment.
unset -v \
    POLYMARKET_ALLOW_LIVE_ORDERS \
    POLYMARKET_PRIVATE_KEY \
    POLYMARKET_API_KEY \
    POLYMARKET_API_SECRET \
    POLYMARKET_API_PASSPHRASE \
    PRIVATE_KEY \
    POLY_ADDRESS \
    2>/dev/null || true

export CARGO_BUILD_JOBS=${CARGO_BUILD_JOBS:-1}
export CARGO_NET_OFFLINE=true
export TMPDIR=${TMPDIR:-"$repository_root/target/tmp"}
mkdir -p -- "$TMPDIR"

run_exact_test() {
    local stage=$1
    local package=$2
    local test_name=$3

    echo
    echo "[$stage] $test_name"
    cargo test \
        -p "$package" \
        --lib \
        --locked \
        --offline \
        "$test_name" \
        -- \
        --exact \
        --nocapture
}

echo "Polymarket manual order-workflow test"
echo "Repository: $repository_root"
echo "Network: disabled by Cargo; venue endpoints are loopback-only"
echo "Credentials: fixed synthetic fixtures only"

run_exact_test \
    "1/3 place + exact cancel wire contract" \
    "reap-polymarket-live-adapter" \
    "mutation::tests::pinned_predarb_t1_place_then_exact_cancel_is_one_attempt_each_without_retry"

echo "PASS: one synthetic type-1 POST /order and one exact DELETE /order; no retry."

run_exact_test \
    "2/3 fill + position reducer" \
    "reap-pm-live" \
    "private_monitor::live::tests::live_fill_advances_order_and_provisional_position_once_until_reconciliation"

echo "PASS: PartiallyFilled; provisional collateral=-2100000, outcome=+5000000, effective position=+5000025."
echo "PASS: replayed duplicate fill did not move the position a second time."

run_exact_test \
    "3/3 end-to-end lifecycle + restart" \
    "reap-pm-live" \
    "composition::product::authenticated_loopback::run::vertical_tests::pm_t2_proxy_place_fill_exact_cancel_and_restart_converge_without_resend"

echo "PASS: durable place, partial fill, authoritative position=10002500000, exact cancel, and restart recovery."
echo "PASS: restart sent neither a second place nor a second cancel."
echo
echo "All Polymarket loopback order-workflow checks passed."
echo "No live order was submitted and no live credential was read."
