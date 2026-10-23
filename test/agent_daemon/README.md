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
