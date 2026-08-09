#!/usr/bin/env bash
set -euo pipefail

if [[ $# -ne 1 ]]; then
    echo "usage: $0 OUTPUT_DIRECTORY" >&2
    exit 64
fi

output_directory=$1
if [[ -e "$output_directory" ]]; then
    echo "PM-T2 source output must not already exist" >&2
    exit 65
fi

umask 077
mkdir -m 0700 -- "$output_directory"
entries="$output_directory/.entries.jsonl"
manifest="$output_directory/manifest.json"
retrieved_at=$(date -u +'%Y-%m-%dT%H:%M:%SZ')

sdk_repositories=(
    'clob-client-v2|https://github.com/Polymarket/clob-client-v2.git|refs/tags/v1.1.0|1.1.0|https://registry.npmjs.org/@polymarket/clob-client-v2/-/clob-client-v2-1.1.0.tgz|a3a56de6d1df809d6ce5e23f29f2dde0982b5f2b38850d763459e600c392f622'
    'py-clob-client-v2|https://github.com/Polymarket/py-clob-client-v2.git|refs/tags/v1.1.0|1.1.0|https://files.pythonhosted.org/packages/5e/c9/a4d6f78aeb1b961f728815f54a22c4cb355e9608635b692f3215afea1776/py_clob_client_v2-1.1.0.tar.gz|7821e2f3ced3651da0956a1225e89ec94434736a8e16086c1c6a14351774088d'
)

sources=(
    'wallets_auth|https://docs.polymarket.com/trading/wallets-auth.md'
    'api_authentication|https://docs.polymarket.com/getting-started/api.md'
    'place_orders|https://docs.polymarket.com/trading/place-orders.md'
    'manage_orders|https://docs.polymarket.com/trading/manage-orders.md'
    'realtime_order_updates|https://docs.polymarket.com/trading/realtime-order-updates.md'
    'matching_engine|https://docs.polymarket.com/trading/matching-engine.md'
    'geoblock|https://docs.polymarket.com/api-reference/geoblock.md'
    'clob_market_info|https://docs.polymarket.com/api-reference/markets/get-clob-market-info.md'
    'fees|https://docs.polymarket.com/trading/fees.md'
    'v2_migration|https://docs.polymarket.com/v2-migration.md'
    'contracts|https://docs.polymarket.com/resources/contracts.md'
    'current_positions|https://docs.polymarket.com/api-reference/core/get-current-positions-for-a-user.md'
    'rate_limits|https://docs.polymarket.com/api-reference/rate-limits.md'
    'polygon_rpc_endpoints|https://docs.polygon.technology/pos/reference/rpc-endpoints.md'
    'polygon_finality|https://docs.polygon.technology/pos/concepts/finality/finality.md'
    'ethereum_json_rpc|https://ethereum.org/developers/docs/apis/json-rpc/'
    'eip_20|https://eips.ethereum.org/EIPS/eip-20'
    'eip_1155|https://eips.ethereum.org/EIPS/eip-1155'
)

cleanup() {
    rm -f -- "$entries" "$output_directory"/*.headers "$output_directory"/*.download
}
trap cleanup EXIT

for source in "${sources[@]}"; do
    id=${source%%|*}
    requested_url=${source#*|}
    case "$id" in
        wallets_auth | api_authentication | place_orders | manage_orders | \
            realtime_order_updates | matching_engine | geoblock | clob_market_info | \
            fees | v2_migration | contracts | current_positions | rate_limits)
            expected_final_url=$requested_url
            expected_content_type='text/markdown'
            extension='md'
            ;;
        polygon_rpc_endpoints | polygon_finality)
            expected_final_url=$requested_url
            expected_content_type='text/markdown'
            extension='md'
            ;;
        ethereum_json_rpc)
            expected_final_url='https://ethereum.org/developers/docs/apis/json-rpc/'
            expected_content_type='text/html'
            extension='html'
            ;;
        eip_20 | eip_1155)
            expected_final_url=$requested_url
            expected_content_type='text/html'
            extension='html'
            ;;
        *)
            echo "PM-T2 source is outside the reviewed official allowlist: $id" >&2
            exit 66
            ;;
    esac
    body="$output_directory/$id.download"
    headers="$output_directory/$id.headers"

    mapfile -t result < <(
        curl \
            --proto '=https' \
            --tlsv1.2 \
            --location \
            --max-redirs 3 \
            --connect-timeout 10 \
            --max-time 60 \
            --max-filesize 4194304 \
            --silent \
            --show-error \
            --fail \
            --dump-header "$headers" \
            --output "$body" \
            --write-out $'%{http_code}\n%{url_effective}\n%{content_type}\n%{size_download}\n' \
            -- "$requested_url"
    )

    if [[ ${#result[@]} -ne 4 || ${result[0]} != 200 ]]; then
        echo "PM-T2 source retrieval did not return one HTTP 200 result: $id" >&2
        exit 67
    fi
    final_url=${result[1]}
    content_type=${result[2]}
    reported_length=${result[3]}
    if [[ $final_url != "$expected_final_url" ]]; then
        echo "PM-T2 source escaped its exact reviewed official URL: $id" >&2
        exit 68
    fi
    if [[ $content_type != "$expected_content_type"* ]]; then
        echo "PM-T2 source returned an unexpected content type: $id" >&2
        exit 69
    fi
    byte_length=$(stat -c '%s' -- "$body")
    if [[ $byte_length -eq 0 || $byte_length -gt 4194304 || $reported_length != "$byte_length" ]]; then
        echo "PM-T2 source length is invalid or inconsistent: $id" >&2
        exit 70
    fi

    case "$id" in
        polygon_rpc_endpoints)
            rg --fixed-strings --quiet 'https://polygon.drpc.org' "$body" \
                && rg --fixed-strings --quiet '| `137`' "$body" \
                || {
                    echo "PM-T2 Polygon RPC source lacks the reviewed endpoint/chain: $id" >&2
                    exit 71
                }
            ;;
        polygon_finality)
            rg --fixed-strings --quiet 'eth_getBlockByNumber' "$body" \
                && rg --fixed-strings --quiet '"finalized"' "$body" \
                && rg --fixed-strings --quiet 'irreversible' "$body" \
                || {
                    echo "PM-T2 Polygon finality source lacks the reviewed semantics: $id" >&2
                    exit 71
                }
            ;;
        ethereum_json_rpc)
            for method in eth_chainId eth_getBlockByNumber eth_call; do
                if ! rg --fixed-strings --quiet "$method" "$body"; then
                    echo "PM-T2 Ethereum JSON-RPC source lacks method $method" >&2
                    exit 71
                fi
            done
            ;;
        eip_20)
            rg --fixed-strings --quiet 'id="allowance"' "$body" \
                && rg --fixed-strings --quiet 'uint256' "$body" \
                || {
                    echo "PM-T2 EIP-20 source lacks the allowance contract: $id" >&2
                    exit 71
                }
            ;;
        eip_1155)
            rg --fixed-strings --quiet 'isApprovedForAll' "$body" \
                && rg --fixed-strings --quiet '_operator' "$body" \
                || {
                    echo "PM-T2 EIP-1155 source lacks the operator-approval contract: $id" >&2
                    exit 71
                }
            ;;
        *) ;;
    esac

    sha256=$(sha256sum -- "$body" | cut -d' ' -f1)
    mv -- "$body" "$output_directory/$id.$extension"

    jq -cn \
        --arg id "$id" \
        --arg requested_url "$requested_url" \
        --arg final_url "$final_url" \
        --arg content_type "$content_type" \
        --arg retrieved_at "$retrieved_at" \
        --arg sha256 "$sha256" \
        --argjson byte_length "$byte_length" \
        '{id:$id,requested_url:$requested_url,final_url:$final_url,retrieved_at_utc:$retrieved_at,content_type:$content_type,byte_length:$byte_length,sha256:$sha256}' \
        >>"$entries"
done

sdk_pins='[]'
for sdk in "${sdk_repositories[@]}"; do
    IFS='|' read -r component repository git_reference package_version package_url expected_package_sha256 <<<"$sdk"
    resolved=$(git ls-remote --exit-code "$repository" "$git_reference")
    commit=${resolved%%[[:space:]]*}
    if [[ ! $commit =~ ^[0-9a-f]{40}$ ]]; then
        echo "PM-T2 official SDK tag is not one canonical Git object: $component" >&2
        exit 72
    fi

    package_download="$output_directory/$component.package.download"
    mapfile -t package_result < <(
        curl \
            --proto '=https' \
            --tlsv1.2 \
            --location \
            --max-redirs 3 \
            --connect-timeout 10 \
            --max-time 60 \
            --max-filesize 16777216 \
            --silent \
            --show-error \
            --fail \
            --output "$package_download" \
            --write-out $'%{http_code}\n%{url_effective}\n%{content_type}\n%{size_download}\n' \
            -- "$package_url"
    )
    if [[ ${#package_result[@]} -ne 4 || ${package_result[0]} != 200 ]]; then
        echo "PM-T2 official SDK package retrieval failed: $component" >&2
        exit 73
    fi
    package_final_url=${package_result[1]}
    package_content_type=${package_result[2]}
    package_length=$(stat -c '%s' -- "$package_download")
    if [[ $package_length -eq 0 || $package_length -gt 16777216 || ${package_result[3]} != "$package_length" ]]; then
        echo "PM-T2 official SDK package length is invalid: $component" >&2
        exit 74
    fi
    package_sha256=$(sha256sum -- "$package_download" | cut -d' ' -f1)
    if [[ $package_sha256 != "$expected_package_sha256" ]]; then
        echo "PM-T2 official SDK package changed from its reviewed digest: $component" >&2
        exit 75
    fi
    mv -- "$package_download" "$output_directory/$component.package"

    sdk_pins=$(jq -cn \
        --argjson pins "$sdk_pins" \
        --arg component "$component" \
        --arg repository "$repository" \
        --arg git_reference "$git_reference" \
        --arg commit "$commit" \
        --arg package_version "$package_version" \
        --arg package_url "$package_url" \
        --arg package_final_url "$package_final_url" \
        --arg package_content_type "$package_content_type" \
        --arg package_sha256 "$package_sha256" \
        --argjson package_byte_length "$package_length" \
        '$pins + [{component:$component,repository:$repository,git_reference:$git_reference,commit:$commit,package_version:$package_version,package_url:$package_url,package_final_url:$package_final_url,package_content_type:$package_content_type,package_byte_length:$package_byte_length,package_sha256:$package_sha256}]')
done

jq -s \
    --arg retrieved_at "$retrieved_at" \
    --argjson sdk_pins "$sdk_pins" \
    '{schema_family:"reap-pm-controlled-trial-official-sources",schema_version:1,retrieved_at_utc:$retrieved_at,official_sdk_pins:$sdk_pins,source_count:length,sources:.}' \
    "$entries" >"$manifest"
chmod 0600 -- \
    "$output_directory"/*.html \
    "$output_directory"/*.md \
    "$output_directory"/*.package \
    "$manifest"
rm -f -- "$entries"
trap - EXIT

echo "$manifest"
