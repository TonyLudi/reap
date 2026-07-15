#!/usr/bin/env bash
set -euo pipefail

export LC_ALL=C

# systemd encodes the displayed 0.0..10.0 exposure as an integer 0..100.
readonly MAXIMUM_EXPOSURE_THRESHOLD=40
readonly INSTANCE=test_instance
readonly SYSTEMD_TREE=/usr/lib/systemd

binary="${1:-target/release/reap}"

fail() {
  printf 'systemd template verification failed: %s\n' "$*" >&2
  exit 1
}

require_single_line() {
  local file="$1"
  local line="$2"
  local count

  count="$(grep -Fxc -- "$line" "$file" || true)"
  [[ "$count" == 1 ]] || fail "$file must contain exactly one '$line'"
}

require_single_key() {
  local file="$1"
  local key="$2"
  local expected="$3"
  local count

  count="$(grep -Ec "^${key}=" "$file" || true)"
  [[ "$count" == 1 ]] || fail "$file must define $key exactly once"
  require_single_line "$file" "${key}=${expected}"
}

[[ "$(uname -s)" == Linux ]] || fail "systemd verification requires Linux"
command -v systemd-analyze >/dev/null || fail "systemd-analyze is unavailable"
[[ -d "$SYSTEMD_TREE" ]] || fail "$SYSTEMD_TREE is unavailable"
[[ -x "$binary" ]] || fail "release CLI is not executable at $binary"

units=(
  deploy/systemd/reap-observe@.service
  deploy/systemd/reap-demo@.service
  deploy/systemd/reap-capture@.service
)

common_lines=(
  'Wants=network-online.target'
  'After=network-online.target'
  'StartLimitIntervalSec=300'
  'Type=simple'
  'User=reap'
  'Group=reap'
  'Environment=RUST_BACKTRACE=1'
  'KillMode=control-group'
  'KillSignal=SIGTERM'
  'TimeoutStopSec=45s'
  'UMask=0077'
  'LimitNOFILE=65536'
  'TasksMax=512'
  'NoNewPrivileges=true'
  'CapabilityBoundingSet='
  'ProtectProc=invisible'
  'PrivateIPC=true'
  'RemoveIPC=true'
  'KeyringMode=private'
  'RestrictRealtime=true'
  'PrivateTmp=true'
  'PrivateDevices=true'
  'ProtectSystem=strict'
  'ProtectHome=true'
  'ProtectHostname=true'
  'ProtectKernelTunables=true'
  'ProtectKernelModules=true'
  'ProtectKernelLogs=true'
  'ProtectControlGroups=true'
  'RestrictSUIDSGID=true'
  'RestrictNamespaces=true'
  'LockPersonality=true'
  'MemoryDenyWriteExecute=true'
  'SystemCallArchitectures=native'
  'RestrictAddressFamilies=AF_UNIX AF_INET AF_INET6'
  'StandardOutput=journal'
  'StandardError=journal'
)

for unit in "${units[@]}"; do
  [[ -f "$unit" ]] || fail "missing $unit"
  for line in "${common_lines[@]}"; do
    require_single_key "$unit" "${line%%=*}" "${line#*=}"
  done
  require_single_key "$unit" Restart "$(
    case "$unit" in
      *reap-observe*) printf 'on-failure' ;;
      *) printf 'no' ;;
    esac
  )"
  grep -Fq -- '--mode production' "$unit" \
    && fail "$unit must not expose production order entry"
  grep -Eq '^ProtectClock=' "$unit" \
    && fail "$unit must permit the host guard's read-only adjtimex query"
done

require_single_key deploy/systemd/reap-observe@.service \
  WorkingDirectory '/var/lib/reap/live/%i'
require_single_key deploy/systemd/reap-observe@.service \
  EnvironmentFile '/etc/reap/live/%i.env'
require_single_key deploy/systemd/reap-observe@.service \
  ExecStartPre '/usr/local/bin/reap live --config /etc/reap/live/%i.toml --mode validate'
require_single_key deploy/systemd/reap-observe@.service \
  ExecStart '/usr/local/bin/reap live --config /etc/reap/live/%i.toml --mode observe'
require_single_key deploy/systemd/reap-observe@.service RestartSec 10s
require_single_key deploy/systemd/reap-observe@.service StartLimitBurst 3
require_single_key deploy/systemd/reap-observe@.service \
  ReadWritePaths '/var/lib/reap/live/%i'

require_single_key deploy/systemd/reap-demo@.service \
  WorkingDirectory '/var/lib/reap/live/%i'
require_single_key deploy/systemd/reap-demo@.service \
  EnvironmentFile '/etc/reap/live/%i.env'
require_single_key deploy/systemd/reap-demo@.service \
  ExecStartPre '/usr/local/bin/reap live --config /etc/reap/live/%i.toml --mode validate'
require_single_key deploy/systemd/reap-demo@.service \
  ExecStart '/usr/local/bin/reap live --config /etc/reap/live/%i.toml --mode demo --confirm-demo'
require_single_key deploy/systemd/reap-demo@.service StartLimitBurst 2
require_single_key deploy/systemd/reap-demo@.service \
  ReadWritePaths '/var/lib/reap/live/%i'

require_single_key deploy/systemd/reap-capture@.service \
  WorkingDirectory '/var/lib/reap/capture/%i'
require_single_key deploy/systemd/reap-capture@.service \
  EnvironmentFile '/etc/reap/capture/%i.env'
require_single_key deploy/systemd/reap-capture@.service \
  ExecStart '/usr/local/bin/reap capture --config /etc/reap/capture/%i.toml --output /var/lib/reap/capture/%i/run-report.json --duration-secs ${REAP_CAPTURE_DURATION_SECS} --require-clean-capture'
require_single_key deploy/systemd/reap-capture@.service StartLimitBurst 2
require_single_key deploy/systemd/reap-capture@.service \
  ReadWritePaths '/var/lib/reap/capture/%i'

root="$(mktemp -d)"
trap 'rm -rf "$root"' EXIT

install -d \
  "$root/etc/systemd/system" \
  "$root/etc/reap/live" \
  "$root/etc/reap/capture" \
  "$root/usr/lib" \
  "$root/usr/local/bin" \
  "$root/usr/local/share/doc/reap" \
  "$root/var/lib/reap/live/$INSTANCE" \
  "$root/var/lib/reap/capture/$INSTANCE"
cp -a "$SYSTEMD_TREE" "$root/usr/lib/"
install -m 0755 "$binary" "$root/usr/local/bin/reap"
install -m 0644 docs/operations.md "$root/usr/local/share/doc/reap/operations.md"
install -m 0644 "${units[@]}" "$root/etc/systemd/system/"
printf '%s\n' '[venue]' 'environment = "demo"' \
  >"$root/etc/reap/live/$INSTANCE.toml"
printf '%s\n' 'REAP_OKX_API_KEY=verification-placeholder' \
  >"$root/etc/reap/live/$INSTANCE.env"
printf '%s\n' '[[subscriptions]]' 'channel = "books"' 'symbol = "BTC-USDT"' \
  >"$root/etc/reap/capture/$INSTANCE.toml"
printf '%s\n' 'REAP_CAPTURE_DURATION_SECS=86400' \
  >"$root/etc/reap/capture/$INSTANCE.env"

instantiated_units=(
  "reap-observe@$INSTANCE.service"
  "reap-demo@$INSTANCE.service"
  "reap-capture@$INSTANCE.service"
)
verify_log="$root/systemd-verify.log"
if ! systemd-analyze verify \
  --root="$root" \
  --recursive-errors=no \
  "${instantiated_units[@]}" \
  >"$verify_log" 2>&1; then
  cat "$verify_log" >&2
  fail "systemd-analyze verify rejected a template"
fi

for unit in "${instantiated_units[@]}"; do
  if ! security="$({
    systemd-analyze security \
      --offline=yes \
      --root="$root" \
      --threshold="$MAXIMUM_EXPOSURE_THRESHOLD" \
      --no-pager \
      "$unit"
  } 2>&1)"; then
    printf '%s\n' "$security" >&2
    fail "$unit exceeds exposure 4.0"
  fi
  printf '%s\n' "$(tail -n 1 <<<"$security")"
done
