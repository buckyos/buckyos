# Linux Agent Daemon acceptance

The TypeScript tests use the shipped WebSDK against the real control-panel access
service, verify-hub and system-config. The target is a clean Ubuntu cloud-image VM
with an ordinary `agenttester` user, no `/opt/buckyos`, no Node.js and no Docker.
The control-plane fixture listens on a separate port and does not restart the
installed services. It reads the local Zone identity to obtain a short-lived
service session; owner and Agent sessions are verified through the real login
flow. It does not disable authorization or TLS verification.

The fixture is intentionally ignored by normal unit tests. In a development Zone:

```sh
cd src
cargo test -p control_panel agent_daemon::tests::manual_zone_access_fixture --no-run
BUCKYOS_ROOT=/opt/buckyos \
BUCKYOS_EXECUTOR_FIXTURE_ADDR=127.0.0.1:14020 \
BUCKYOS_EXECUTOR_FIXTURE_ORIGIN=wss://HOST:14443 \
  target/debug/deps/control_panel-TEST_HASH --ignored --exact \
  agent_daemon::tests::manual_zone_access_fixture --nocapture
```

Use the actual test-binary path printed by Cargo; the workspace may configure a
different target directory. Run the binary directly so a long-lived fixture does
not retain Cargo's target lock. `tls_proxy.py` forwards the test TLS port to the
fixture. Generate a dedicated test CA and server certificate with the host's IP
SAN; install that CA only in the test VM. Never use insecure TLS flags.

```sh
python3 test/agent_daemon/tls_proxy.py --listen HOST --port 14443 \
  --backend-port 14020 --cert server.crt --key server.key
deno check --no-config --unstable-sloppy-imports \
  test/agent_daemon/e2e.ts test/agent_daemon/service_e2e.ts
NODE_EXTRA_CA_CERTS=/path/to/test-ca.crt node --experimental-strip-types \
  test/agent_daemon/e2e.ts /path/to/private-config.json
NODE_EXTRA_CA_CERTS=/path/to/test-ca.crt node --experimental-strip-types \
  test/agent_daemon/service_e2e.ts /path/to/private-config.json
python3 -m unittest discover -s test/agent_daemon -p 'test_*.py'
```

Build `buckyos-websdk` with `pnpm build` first. The private configuration must have
mode 0600 and supplies `control_url`, `owner_token`, `agent_token`, `agent_did`,
`ssh_key`, `known_hosts`, `vm_ip`, `remote_binary`, and `report_file`.
`launcher_command` can run the release launcher with a predownloaded archive and
its verified SHA-256; URLs go through stdin. Optional `platform` and `vm_id` label
the report. `reconnect_check: true` uses PVE guest-agent `ss -K` only inside that
test VM to interrupt its WSS sockets and verify same-process reconnection without
replaying an accepted action. Do not enable that option for an existing machine.

The service test additionally checks install, stop, verified upgrade, a failed
upgrade rollback, URL expiry without automatic restart, and uninstall preserving
user files. Test-created management records and saved access URLs are removed in
`finally`; business files and ledgers remain available for inspection. Delete
temporary session-token configuration after testing and stop the isolated fixture,
TLS proxy and VMs. Retain images and signed-out VM disks for reproducibility.

The checked-in `acceptance-linux-x86_64.json` records 14 successful checks on PVE
VM 111 (Ubuntu 24.04, x86_64 musl, uid 1000). `acceptance-user-service.json` records
the six user-service lifecycle checks. `acceptance-linux-arm64.json` records 13
checks on VM 112 (Ubuntu 24.04.5, aarch64 GNU, glibc 2.39) using QEMU TCG on the
x86_64 PVE host; physical ARM hardware and ARM user-service installation were not
tested. Both VMs are stopped with their disks retained. These are real packaged-binary tests;
macOS, Windows and GUI backends are outside this delivery.

# Desktop (macOS / Windows) acceptance

`desktop_e2e.ts` repeats the access/command/file lifecycle above against the
real Zone entry (`wss://<zone host>`, the production cyfs-gateway 443 port and
the installed control-panel; no fixture, no TLS proxy) and adds the Computer
protocol: `inspect`, `observe` (the PNG is read back through the ledger
`content` channel and its IHDR size is compared with the observation),
point click with `feedback: after`, a focus-targeted `wait_for` (on a bare
desktop Windows answers `focus_unverifiable`, which the test records instead of
guessing), Unicode `type` into Notepad (Windows), a click against a superseded
observation (`stale_observation`), local `pause`/`resume`, and the
restart/revocation/expiry checks. Because the daemon must run inside the
logged-in desktop session, launches go through a slot launcher:

- macOS: `mac_launcher.sh`, started once from a Terminal window
  (`open ~/s.command`, which exports `SSL_CERT_FILE` for the Zone's dev CA and
  execs the launcher). The daemon inherits Terminal's GUI audit session and TCC
  attribution, so Screen Recording and Accessibility are granted to Terminal in
  System Settings before the run. Everything else goes over SSH.
- Windows: `win_remote.py` drives the VM through the PVE guest agent
  (`qm guest exec`, SYSTEM in session 0) and runs daemon launches and daemon
  CLI calls as Task Scheduler tasks registered with `/RU <user> /IT` and no
  stored password (a password-backed task fails to start with "element not
  found"). The VM needs the Zone CA in the machine Root store and a hosts entry
  for the Zone host.

```sh
deno check --no-config --unstable-sloppy-imports test/agent_daemon/desktop_e2e.ts
python3 -I test/agent_daemon/win_remote.py <vmid> setup <user> <password>
NODE_EXTRA_CA_CERTS=/opt/buckyos/etc/ca.cert node --experimental-strip-types \
  test/agent_daemon/desktop_e2e.ts /path/to/private-config.json
```

The private configuration (mode 0600) carries `control_url`
(`https://<zone host>/kapi/control-panel`), `owner_token`, `agent_token`,
`agent_did`, `platform` (`macos` | `windows`), `vm_id`, `report_dir`,
`remote_binary`, `home`, plus `vm_ip`/`ssh_key`/`known_hosts` for macOS. Owner
and Agent session tokens expire within about 30 minutes, so refresh them right
before a run. The report directory receives `report.json`, `doctor.json`,
`describe.json`, `inspect.json` and the `observe.png` / `click.png` /
`typed.png` feedback screenshots.

`acceptance-windows-x86_64.json` records the 20 checks that passed on PVE VM 113
(Windows 10 Pro 19045, console session 1) on 2026-10-24 with a static-CRT debug
build compiled on the VM. The expiry check issues a 45 s URL because a Task
Scheduler launch reaches the Zone roughly 10 s after issue.
`acceptance-macos-x86_64.json` records the 19 checks that passed on PVE VM 114
(macOS 26.7.1 x86_64 under QEMU, console user logged in, Terminal granted
Screen Recording and Accessibility) with a debug build compiled on the VM
after the audit-session fix; the focus-targeted wait succeeds on the macOS
desktop. Neither run is a physical-machine, signed-binary or release-package
result.
