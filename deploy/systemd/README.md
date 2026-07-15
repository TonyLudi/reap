# systemd Deployment

These templates are a reviewed baseline, not target-host certification. Install
the release binary and documentation as root, create an unprivileged `reap`
user/group, and keep configuration and credentials outside the repository.
The repository gate instantiates all templates in an alternate root, requires
their mode-specific command/restart/path policy, runs `systemd-analyze verify`,
and rejects an offline security exposure above `4.0`. The current templates
score `2.9` on the development systemd release.

Expected instance layout for an instance named `btc-demo`:

```text
/usr/local/bin/reap
/etc/reap/live/btc-demo.toml       root:reap 0640
/etc/reap/live/btc-demo.env        root:reap 0640
/var/lib/reap/live/btc-demo/       reap:reap 0750
/var/lib/reap/connectivity/        reap:reap 0750
```

Capture run instance `btc-public-20260715T000000Z` reuses the reviewed
`/etc/reap/capture/okx-btc-public.toml`, has a matching `.env` containing only
`REAP_CAPTURE_DURATION_SECS=<positive integer>`, and writes to a unique instance
directory:

```text
/etc/reap/capture/okx-btc-public.toml                         root:reap 0640
/etc/reap/capture/btc-public-20260715T000000Z.env             root:reap 0640
/var/lib/reap/capture/btc-public-20260715T000000Z/raw.jsonl   reap:reap 0600
/var/lib/reap/capture/btc-public-20260715T000000Z/run-report.json
```

Never put credentials in the capture environment file. Do not copy or edit the
capture TOML per run: the unit overrides its placeholder raw path with the
instance path and reserves both outputs with create-new semantics. This keeps
the exact source-config bytes stable across datasets while each process receives
a unique session and artifact boundary.

Every deployed live TOML and the shared capture TOML must set
`connection_attempt_pacer_path =
"/var/lib/reap/connectivity/okx-global.pacer"`. `StateDirectory` creates that
shared persistent directory, and each template exposes it through the otherwise
read-only filesystem policy. The pacer file is created owner-only by Reap. It
coordinates service processes on this host only; hosts sharing one NAT require
isolated egress or an external IP-wide coordinator.

Install and validate the units:

```bash
cargo build --release --locked
deploy/systemd/verify-units.sh target/release/reap
sudo install -o root -g root -m 0755 target/release/reap /usr/local/bin/reap
sudo install -d -o root -g root -m 0755 /usr/local/share/doc/reap
sudo install -o root -g root -m 0644 docs/*.md /usr/local/share/doc/reap/
sudo install -d -o root -g reap -m 0750 /etc/reap/capture
sudo install -o root -g reap -m 0640 deploy/capture/okx-btc-public.toml /etc/reap/capture/
sudo install -o root -g root -m 0644 deploy/systemd/*.service /etc/systemd/system/
sudo systemd-analyze verify /etc/systemd/system/reap-*.service
sudo systemctl daemon-reload
```

The common policy removes every capability, prevents clock writes and realtime
scheduler acquisition, isolates process visibility, IPC, devices, temporary
files, and keyrings, and retains only Internet and Unix socket families. It does
not use `ProtectClock=true`, because that blocks the host guard's read-only
`adjtimex()` synchronization query; the empty capability set still prevents clock
mutation. It also omits `ProcSubset=pid` because the guard reads global Linux
memory state. Any future realtime scheduling, memory locking, device, or
additional writable-path requirement is a reviewed deployment-policy change;
do not weaken the unit ad hoc.

Start one mode only for a live instance:

```bash
sudo systemctl start reap-observe@btc-demo.service
sudo systemctl start reap-demo@btc-demo.service
sudo systemctl start reap-capture@btc-public-20260715T000000Z.service
```

`observe` is exchange-read-only and may restart after failure, capped by the
unit start limit. `demo` never restarts automatically: every abnormal exit needs
exchange/account reconciliation and operator approval. `capture` also never
restarts automatically. Its unit requires a positive duration, requests
`--require-clean-capture`, and reserves `run-report.json` in the instance
directory. Every process requires a fresh instance name and directory, raw path,
report path, and session identity, but reuses the exact reviewed capture config.
Capture uses create-new file semantics and will fail before opening feed sockets
if an output already exists; an early CLI failure can leave an empty reserved
report that must not be reused.

Configure the host monitoring system to page on unit activation failure,
non-zero exit, start-limit exhaustion, forced `SIGKILL`, and host clock/disk/
memory alerts. Review `systemd-analyze security`, resource limits, CPU affinity,
time service, filesystem capacity, and kernel/network tuning on the actual host.
Run the source gate and `systemd-analyze verify` again on that exact OS after all
drop-ins are installed, then archive the merged unit, exposure report, binary and
config hashes, unit lifecycle, and paging exercise. The source gate cannot prove
those target-host controls or external alert delivery.

At pinned `imm-strategy` revision
`b6b120c7b7c466d8431bf082f3229328c5d7b2ae`,
`MetCoinGatewayWsClientsOkexV5Config` constructs separate public, position, and
order websocket clients but does not define an external process supervisor. The
units are deployment hardening around the Java-referenced connectivity and Chaos
strategy behavior, not a Java parity claim.
Do not enable demo order entry until the procedures in
`docs/trading-readiness.md` have produced credentialed acceptance evidence.
