# systemd Deployment

These templates are a reviewed baseline, not target-host certification. Install
the release binary and documentation as root, create an unprivileged `reap`
user/group, and keep configuration and credentials outside the repository.

Expected instance layout for an instance named `btc-demo`:

```text
/usr/local/bin/reap
/etc/reap/live/btc-demo.toml       root:reap 0640
/etc/reap/live/btc-demo.env        root:reap 0640
/var/lib/reap/live/btc-demo/       reap:reap 0750
```

Capture instance `btc-public` uses `/etc/reap/capture/btc-public.toml` and
`/var/lib/reap/capture/btc-public/`. Use absolute storage, operator-socket, and
capture-output paths inside deployed TOML; each must remain under that
instance's writable directory.

Install and validate the units:

```bash
cargo build --release --locked
sudo install -o root -g root -m 0755 target/release/reap /usr/local/bin/reap
sudo install -d -o root -g root -m 0755 /usr/local/share/doc/reap
sudo install -o root -g root -m 0644 docs/*.md /usr/local/share/doc/reap/
sudo install -o root -g root -m 0644 deploy/systemd/*.service /etc/systemd/system/
sudo systemd-analyze verify /etc/systemd/system/reap-*.service
sudo systemctl daemon-reload
```

Start one mode only for a live instance:

```bash
sudo systemctl start reap-observe@btc-demo.service
sudo systemctl start reap-demo@btc-demo.service
sudo systemctl start reap-capture@btc-public.service
```

`observe` is exchange-read-only and may restart after failure, capped by the
unit start limit. `demo` never restarts automatically: every abnormal exit needs
exchange/account reconciliation and operator approval. `capture` also never
restarts automatically because every process requires a fresh output path and
session identity. Rotate its configured output before starting it again.

Configure the host monitoring system to page on unit activation failure,
non-zero exit, start-limit exhaustion, forced `SIGKILL`, and host clock/disk/
memory alerts. Review `systemd-analyze security`, resource limits, CPU affinity,
time service, filesystem capacity, and kernel/network tuning on the actual host.
Do not enable demo order entry until the procedures in
`docs/trading-readiness.md` have produced credentialed acceptance evidence.
