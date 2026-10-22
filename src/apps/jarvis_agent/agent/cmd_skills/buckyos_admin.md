---
name: buckyos_admin
description: Inspect and manage a BuckyOS Zone — identify runtime, identity and versions, read or change settings and apps, check tasks, query logs and diagnostic bundles, and trace a fault into the source of the running version.
requires: the buckyos command (agent container: /usr/local/bin/buckyos with the host Tool mounted at /opt/buckyos/libexec/buckyos-tool; host: $BUCKYOS_ROOT/bin/buckyos); writes depend on the agent's and its owner's RBAC
---
#### When to use
The Zone itself: what runs, which version, is it healthy, change a setting, start/stop an app, why something fails,
which code produced an error. App building/publishing, AIGC and model calls: `buckyos_app_dev`, `buckyos_dev_release`, `aigc_*`, `xllm`.

#### Before you start
```
command -v buckyos && buckyos --version --verbose   # local, no network call
buckyos auth whoami                                 # principal, appid, zone: proves endpoint + token
buckyos system info                                 # BuckyOS version, build_version (commit), release channel
```
- `--version --verbose` gives `buckyos` (system version in the Tool manifest), `buckyos_build_id`, `tool_version`,
  `sdk_version`, `protocol_version`, `runtime_executable`, `package_root`. Tool under `/opt/buckyos/libexec/buckyos-tool`
  run by the image's `/usr/local/bin/deno` = agent container; `$BUCKYOS_ROOT/libexec/buckyos-tool/runtime/deno` = host.
- In the container the Tool scripts are the host's files mounted read-only (same version as the system), the endpoint
  is `BUCKYOS_TOOL_ENDPOINT` (host NodeGateway `http://host.docker.internal:3180`), the identity is
  `BUCKYOS_APPCLIENT_SESSION_TOKEN`: the agent app, `principal` = your owner, `appid` = your agent id.
  On a host, `buckyos` uses the device identity on `127.0.0.1:3180` and asks for `--yes`.
- Container paths are not host paths. Your `/opt/buckyos/logs` is the host's `$BUCKYOS_ROOT/logs/apps/<instance-hash>/`;
  the host's `etc/`, `security/`, `bin/`, other containers and processes are not visible to you.
- Agent creation defaults to `permission_mode: read_write`; the owner can choose `read_only` in Add Agent.
  `agent get <agent_id>` returns the selected mode. Owner home follows that mode; `$OPENDAN_AGENT_ROOT` is always
  writable for sessions, memory, workspaces, tools and skills. This grant is in AgentSpec, not agent settings.
- `buckyos` missing or ".../buckyos-tool/... not found": the Tool mount is not deployed in this container. Say so and
  give the owner the exact host commands instead of guessing results.
- Host-side `uv run src/check.py` (repo) checks processes and ports. "Activation Ready" means waiting for activation,
  not running; "Running" with exit 0 can still come with an error-heavy log warning below it.

#### Find the real commands
- `buckyos command list` (modules and verbs of this install), `buckyos command describe <module> <verb>` (syntax,
  options, `access` read/write/privileged, `async_mode`), `buckyos <module> --help`.
- Source: `/opt/buckyos/libexec/buckyos-tool/cli/modules/<module>.ts` (`system-config` is `system_config.ts`); identity
  and endpoint in `cli/core/{auth,config,identity,runtime}.ts`; error codes and exit codes in `cli/core/errors.ts`.
- stdout: one JSON envelope `{ok, data | error{code,message,retryable}, meta{trace_id}}`; `--output text` prints a system-config value as-is; progress of waiting verbs goes to stderr.

#### Settings: where each one is managed
- Agents: `buckyos agent list`, `agent get|profile-get <agent_id>` (raw host name like `xiaobai.test.buckyos.io`, not
  the `did:web:` DID), `agent profile-set <agent_id> --display-name <n> [--bio <t>]` (given fields only, `''` clears;
  returns `before` and `profile`, which is your revert). Users, agent create/delete/settings: Desktop "Users and Agents".
- Apps: `buckyos app list|get|status|start|stop|restart|install|upgrade|uninstall|cancel`. Availability
  (`apps.availability.*`) has no CLI verb and needs a user's Control Panel login.
- Devices, network, gateway, certificates: control-panel `zone.devices.list`, `network.overview`, `gateway.overview`,
  `zone.certificates.list` are read-only and not in the CLI. `devices/<id>/info` is node-reported Info, not a setting.
- Storage: no settings API; `system status` warns on disk/memory/swap of the control-panel host.
- AI providers: `buckyos ai providers` (auth as configured/kind, health, models; never secrets), `ai routing`,
  `ai usage [--range last1d|last7d|last30d | --since <t>] [--group-by provider_instance_name,provider_model] [--bucket day]`
  (your own calls only); per-model health and prices: `ai_provider health <exact_model>|models` (agent container).
  Adding/editing providers or routing: no CLI verb, the owner uses Control Panel AI settings.
- Developer mode, Zone display, desktop appearance: Desktop UI (`system/buckyos_dev_config` needs a sudo token the CLI lacks).
- Node Executor (Agents running commands/files on a node, `doc/arch/node_exector.md`): the setting is the `executor`
  field of `nodes/<node>/settings` (`enabled`, `allowed_devices` = exact device DIDs from `devices/<id>/info`,
  `profiles.<id>` = `run_as{uid,gid}` (non-root; omitted for Windows nodes), `workdir_roots` (absolute node paths,
  e.g. `C:\Users\<u>\ws` on Windows), `env`, `principals` = exact Agent DIDs, `ops`, and
  `limits`); writing it needs an owner sudo token. The scheduler turns it into `nodes/<node>/config.executor` (never
  edit): check `generation` grew, the Agent is in `principal_bindings` with `enabled`/`app_installed` true (a runtime
  shared by two AgentSpecs is not mapped), and `auth_valid_until_ms` keeps moving (30 s lease). On the node:
  `etc/node_executor_acl.json` lists the allowed devices, node_daemon logs `node executor opened, store_id=…`, the
  executor listens on `127.0.0.1:3184`, Agents enter through the gateway SOCKS entry `:3185`. Its ledger is
  `local/node_daemon/executor/`; a reinstall (`start.py --all`/`--reinstall`) clears `local/`, gives a new store_id, and
  earlier runs then report StoreMismatch instead of resuming. Command/file execution requires a Linux or macOS
  node_daemon running as root, or a Windows node_daemon in the user's desktop session; on macOS give `run_as` a
  dedicated non-admin account (no no_new_privs there, and uid 65534 has no account). Windows cannot switch users:
  operations run as the desktop user with all of that user's rights, the shell is PowerShell, and only trusted
  Agents should get a Windows profile. A profile whose `run_as` does not fit the host (missing on Unix, present on
  Windows) and non-root Unix hosts expose only control capabilities (`open_stream`, `describe`, `query`, `cancel`);
  execution, file operations, and workdir resolution return `Capability`.
- The rest goes through raw `system-config get|list|set|set-file|append`. Settings (user-adjustable, fine to edit):
  `users/<u>/apps/<app>/settings`, `users/<u>/agents/<agent>/{settings,profile}`, `services/<svc>/settings`,
  `users/<u>/desktop/<session>/<state>`. Config (derived by the scheduler, rewritten or broken by hand edits):
  `nodes/<node>/{config,gateway_info,gateway_config}`, `services/<svc>/info`, `system/rbac/policy` (despite the CLI's
  `append` example), `system/app_registry`, `system/scheduler/*`, installed apps' `spec` (use `app` verbs).
  `*/instances/*` and `devices/*/info` are runtime reports; `*/doc` and `boot/config` are signed identity documents.

#### Change loop
1. Read and keep the current state: `buckyos system-config get <key> > $OPENDAN_SESSION_DIR/before.json` (value and
   `version`), or `buckyos app status <app_instance_id>`.
2. Smallest change: edit the one field and write the whole JSON with `buckyos system-config set-file <key> --file new.json`
   (`set` replaces the full value; the CLI has no compare-and-set or json-path verb).
3. Confirmation: `app install|upgrade|uninstall` print a preflight and need `--yes` in a non-interactive shell; run
   `--dry-run` first. `start|stop|restart` and system-config writes do not prompt. Within what the user asked for,
   don't re-ask at every step; ask before a write they did not ask for.
4. Check the effect, not exit 0: read back (`version` +1, value as written); for apps `app status` until desired,
   scheduled and runtime instances agree and `readiness` is `READY`, or `--timeout 10m task wait <task_id>`; for a
   service setting, look for its reload or error with `log query --service <svc> --since <write time>`.
5. Report before → after, the evidence and the revert: re-read that the value is still yours, then
   `set-file <key> --file before.json`, so you don't clobber a newer change.

#### Diagnose
Keep a timeline: symptom (user words, time) → evidence (command, `trace_id`, quoted log lines) → hypothesis → check
→ advice. Quote what you fetched; when something could not be fetched, say so rather than filling it in.
- `buckyos system status`: `state` and `warnings` from the control-panel host's resources, the keys under `services/`
  (each `status` is `unknown` whatever its health: not a health check) and `buckyos` (version record).
- `buckyos app status <app_instance_id | app_did>` (e.g. `xiaobai.test.buckyos.io@devtest`; a short name like
  `xiaobai` becomes `did:bns:xiaobai` and fails with `APP_NOT_INSTALLED`).
- `buckyos task list --state running --limit 20`, `task list --type app.install/v1`, `task get <id>`.
- `buckyos log query --service <id> [--level error|warning|info] [--keyword <text>] [--since <RFC3339>]
  [--direction backward] --limit 100`. `buckyos log services` lists the ids (`scheduler`, `node_daemon`, `control-panel`,
  `aicc`, `opendan`, ...; wrong id: `Unknown log service: <id>`). `--level warn` = `warning`. Entries carry host-local
  `MM-DD HH:MM:SS.mmm` without a year; page with `--cursor <next_cursor>`.
- `buckyos --timeout 2m log tail --service <id> --from end` (JSONL);
  `buckyos log export --services a,b --since <RFC3339> --path $OPENDAN_SESSION_DIR/logs.zip`.
- App and agent container logs are out of reach of `log`: `apps` holds per-instance subdirectories that query does
  not read, and `opendan` there is the host's. Your own runtime logs are files under `/opt/buckyos/logs/` in your container.
- `buckyos diagnostic collect --services a,b --since <RFC3339>` (privileged; waits for the task; result has `bundle_id`,
  expiring in about an hour) → `buckyos diagnostic export <bundle_id> --path $OPENDAN_SESSION_DIR/diag.zip` (sha256
  checked). Scheduler logs run to tens of MB per hour; narrow the window.
- Logs and bundles come redacted (`[REDACTED_TOKEN]`). Don't print tokens, `BUCKYOS_APPCLIENT_SESSION_TOKEN`, key
  files or `users/<u>/settings` (it holds a password hash); summarize the fields you need.
- Host-side evidence (processes, ports, docker, node_daemon, `etc/`, `security/`, other apps' container logs): name the
  host command for the owner, e.g. `uv run src/check.py`, `docker ps`, `ls $BUCKYOS_ROOT/logs/apps/`.

#### When it fails
- Exit codes: 2 usage; 3 auth (`AUTH_REQUIRED`, `INVALID_SESSION`, `SESSION_EXPIRED`); 4 permission
  (`PERMISSION_DENIED`, `CONFIRMATION_REQUIRED`); 5 `SERVICE_UNAVAILABLE` (`fetch failed`, retryable);
  6 operation (`RESOURCE_NOT_FOUND`, `OPERATION_FAILED`); 8 timeout.
- Agent identity: system-config checks both the agent app (`app:<agent_id>`) and the owner. The app inherits the
  owner's authorized operations according to `permission_mode`; this does not grant operations the owner lacks.
  `read_only` blocks user-data/configuration writes, including agent settings/profile, while retaining the Agent's
  own info, service-instance reporting and mailbox delivery. Local AgentRoot writes still work. Check the actual result before
  reporting a change; `No read permission` / `No write permission` is a refusal.
  Control-panel RPC, Desktop writes and sharing also enforce read-only Agent grants; switching from a raw
  system-config write to a higher-level API does not authorize it.
  Read-only grants exclude raw login credentials: `users/<u>/settings` (password hash),
  `users/<u>/agents/<agent>/key` and `security/*`. An API that needs those records can also be refused.
  Control-panel methods still enforce their user-type and user-login requirements; for example a normal user's
  agent cannot collect an administrator diagnostic bundle. `task list` shows the tasks your agent created.
- A refusal is an answer: report the code and the owner-side path; don't retry as another identity or route around
  it through raw keys.
- `SESSION_EXPIRED` in a long `tail`/`wait`: the injected token lasts minutes and is refreshed per shell call; rerun in
  a new call. `CONNECTION_REQUIRED`: no endpoint; add `--endpoint http://host.docker.internal:3180`.
- Re-running writes: pass `--idempotency-key <k>` to app and diagnostic verbs; re-read before repeating a set.

#### Source of the running version
1. Record versions first: `buckyos --version --verbose` (`buckyos`, `buckyos_build_id`, `tool_version`, `sdk_version`,
   `runtime_version`) and `buckyos system info` → `build_version` = `build<YYMMDD>.<branch>.<commit12>[.dirty]`.
2. Client layer: the mounted `cli/modules/*.ts` is the code that runs; it names the service and RPC
   (`ctx.clients.call('control-panel', 'system.logs.query', ...)`, `system_config` `sys_config_get`). Check the files are
   the shipped ones: `sha256sum` them against `npm_files[].sha256` in `/opt/buckyos/libexec/buckyos-tool/distribution.json`.
3. Server layer: look the RPC name up in BuckyOS source of the same commit: control-panel dispatch
   `src/frame/control_panel/src/main.rs`, system-config `src/kernel/sys_config_service/src/main.rs`, RBAC
   `src/kernel/buckyos-api/src/rbac_config.rs`, verify-hub `src/kernel/verify_hub/src/main.rs`. Without a matching
   local tree, fetch a read-only snapshot into the session dir:
   `git clone --filter=blob:none --branch main https://github.com/buckyos/buckyos src && git -C src checkout <commit12>`.
4. What the ids tell you: Tool `build_id` `dev-<uuid>` = local build from websdk source, random, no commit; rely on
   `tool_version` and the per-file sha256. `<commit>:<platform>:<arch>` = Tool prepared from the published npm package
   while building that BuckyOS commit. System `.dirty` = uncommitted changes were built in, so the matching commit is
   close but not exact. Commit missing upstream = evidence gap.
5. Latest `main` is not the running version. Without matching source, say so, give the versions you recorded, and
   keep the conclusion to what logs and the client source show.

#### Extending
SDK types: `/opt/buckyos/libexec/buckyos-tool/dist/*.d.ts` (`node.d.ts`, `task_mgr_client.d.ts`,
`verify-hub-client.d.ts`, ...). For a control-panel RPC without a verb (`zone.devices.list`, `user.list`,
`network.overview`, ...) you can copy the call pattern of `cli/modules/system.ts` into `$OPENDAN_AGENT_ROOT/tools/<name>.ts`
and run it with `deno run --config /opt/buckyos/libexec/buckyos-tool/cli/deno.json` (maps `buckyos` to the mounted SDK),
using `BUCKYOS_TOOL_ENDPOINT` and the session token; this recipe is untested. Leave the mounted Tool files unchanged.

Checked against: BuckyOS 0.7.0 (build261017.main.1b6e277fadd9.dirty), Tool/SDK 0.7.131 (`system info`, `log services`, `agent`, `ai` verbs from websdk source, run on the host as device and as devtest), test.buckyos.io; agent-identity results read from policy and code, not run in a container, 2026-10-17.
