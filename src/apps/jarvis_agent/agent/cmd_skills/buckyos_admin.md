---
name: buckyos_admin
description: Inspect and manage a BuckyOS Zone — identify runtime, identity and versions, read or change settings and apps, check tasks, query logs and diagnostic bundles, and trace a fault into the source of the running version.
requires: the buckyos command (agent container: /usr/local/bin/buckyos with the host Tool mounted at /opt/buckyos/libexec/buckyos-tool; host: $BUCKYOS_ROOT/bin/buckyos); writes depend on the agent's and its owner's RBAC
baseline: standard
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
  On a host, `buckyos` uses the device identity on `127.0.0.1:3180` and asks for `--yes`. In a session on a node
  runtime, `buckyos` is the node's `$BUCKYOS_ROOT/bin/agent-tools/buckyos` entry: it runs the host's launcher with
  your agent's identity (asked of your agent over the session token in `.opendan/`), so `whoami` shows your agent.
- Runtimes of an Agent: `agent-session runtime list` shows each one's declared `baseline` (standard | extended),
  `platform` and its last tool check (`baseline_check.missing`); `agent-session runtime baseline` lists the standard
  commands and `xagent runtime baseline --check [--overlay]` checks this machine. A node's agent tool directory is
  `$BUCKYOS_ROOT/bin/agent-tools` (laid out by node-daemon with `opendan install-agent-tools`); missing entries mean
  that node's install lacks OpenDAN. The owner declares a runtime extended in the Agent's runtime settings
  (OpenDAN WebUI or control-panel `agent.update {runtimes: {"<id>": {"baseline": "extended"}}}`).
- Container paths are not host paths. Your `/opt/buckyos/logs` is the host's `$BUCKYOS_ROOT/logs/apps/<instance-hash>/`;
  `/opt/buckyos/etc/` is the host's configuration directory mounted read-only. Host `security/`, `bin/`, other
  containers and processes are not visible to you. Startup credentials are in the host's `security/start_config.json`;
  development CA keys and private scheduler templates also belong under `security/`, outside this mount.
- A browser link ending in `.bns.did`: check `web3_bridge.bns` in the container's `machine.json` against the host's
  policy. AppService loads that policy before name resolution; an absent file uses defaults. After updating
  node-daemon and aios, recreate the Agent container to apply its mount; `docker restart` keeps the old mounts.
- Container creation reports misplaced `etc/start_config.json` or `etc/ca_key.pem`: an authorized host operator must
  move those files into the security root and restore the public boot template before exposing `etc/`. See the repo's
  `doc/path_usage.md`; never copy startup credentials or private templates into a container.
- Agent creation defaults to `permission_mode: read_write`; the owner can choose `read_only` in Add Agent.
  `agent get <agent_id>` returns the selected mode. Owner home follows that mode; `$OPENDAN_AGENT_ROOT` is always
  writable for sessions, memory, workspaces, tools and skills. This grant is in AgentSpec, not agent settings.
- `buckyos` missing or ".../buckyos-tool/... not found": the Tool mount is not deployed in this container. Say so and
  give the owner the exact host commands instead of guessing results.
- Host-side `uv run src/check.py` (repo) checks processes and ports. "Activation Ready" means waiting for activation,
  not running; "Running" with exit 0 can still come with an error-heavy log warning below it. The host root uses
  `BUCKYOS_ROOT` when set, otherwise `C:\buckyos` on Windows and `/opt/buckyos` on macOS/Linux.
- OOD startup waits for an authenticated local system-config RPC before registering its device. Readiness and
  each registration write have a 60-second retry budget for transport failures; authentication and permission
  errors fail immediately. A missing `boot/config` is valid during first activation. If startup times out, inspect
  node-daemon and system-config logs and port 3200; do not clear identity or user data. A successful `start.py`
  launch or `local system-config RPC ready` log does not prove scheduler and the remaining services are ready;
  use the host check above to verify the complete startup.
- macOS Desktop nodes run node_daemon as `$BUCKYOS_ROOT/bin/node-daemon/BuckyOS Runtime.app`, the LaunchAgent
  `com.buckyos.runtime` of the one OS user in `etc/desktop_runtime.json`. Host command for the owner:
  `"$BUCKYOS_ROOT/bin/node-daemon/BuckyOS Runtime.app/Contents/MacOS/node_daemon" service status` (bound user,
  launchd state, login autostart, Login Items disabled, the Runtime's own GUI/permission report); `service
  start|stop|restart|autostart on|off` controls it. Not running before the user logs in is expected; launchd output
  is in `logs/node_daemon/launchd.err.log`. Screen Recording and Accessibility are granted to "BuckyOS Runtime" (not
  the BuckyOS App or Terminal); Screen Recording applies after a Runtime restart.

#### Find the real commands
- Lightweight Linux/macOS/Windows executors: Settings → Cluster Manager → Nodes/Devices issues an access URL;
  `buckyos executor --help` describes device management and online/exec/read/write/edit/query/cancel.
  Admin Control Panel sessions manage devices; an authorized Agent App session executes operations.
  Select the current `executor_id`, `generation`, `profile_id` and `store_id` from online discovery;
  never substitute a Node DID or silently select a new generation. URL creation alone does not make a target online.
  On the target, `buckyos_agent_daemon doctor --json` and `stop --executor <id>` need no Zone login.
  URL expiry/revocation or foreground exit removes availability; Linux UI is `unsupported`.
  macOS 14+ desktop access is a separate profile grant (`computer_inspect`, `computer_observe`,
  `computer_act`) and requires Screen Recording/Accessibility readiness on the logged-in user's Mac.
  Windows uses `buckyos_agent_daemon.exe` in the logged-in user's foreground desktop session, with the same
  desktop grants; Session 0 services, locked/secure desktops and targets with higher integrity than the daemon
  are not supported. Elevation does not grant access across users or secure desktops.
  Its command backend uses PowerShell 7 from PATH or Windows PowerShell, and data defaults to
  `%LOCALAPPDATA%\buckyos-agent-daemon`. Run `doctor --json` before desktop work.
  The standalone Windows package build links the MSVC CRT statically; ordinary Cargo development binaries
  require the matching VC++ v14 runtime. `--binary` packages are unverified: inspect their manifest and
  provide the prebuilt binary's runtime dependencies instead of assuming VC runtime DLLs ship with Windows.
  Windows supports the primary display, visual/auto observations, click, Unicode type, keypress,
  line scrolling and delay waits; semantic/hybrid, dragging and pixel scrolling are unsupported.
  The owner can run `buckyos_agent_daemon pause|resume --executor <id>`; request acceptance is not
  cancellation confirmation. Inspect `instances[].capabilities` in local status, query the original operation,
  and take a new observation after resuming. Never replace the original store or replay uncertain input.
  Windows automatically pauses on detected human input; the owner must resume locally and observe again.
  If a crash loses the pending Unicode release data, resume stays paused/dirty and requires owner handling;
  do not delete its coordination file to bypass that state.
  A Windows input monitor heartbeat gap disables desktop access until daemon restart, including gaps after
  sleep or debugger pauses. Resume cannot repair this state; restart, check readiness and observe again,
  while querying the original operation instead of replaying uncertain input.
  The native driver and SDK `ExecutorCommandStream.computer` are implemented; use the source protocol
  in `src/kernel/buckyos-api/src/computer.rs`. Do not assume the planned `buckyos computer` CLI or `--present` bridge exists.
  Keep URL secrets out of logs/messages. Query an uncertain operation by its original store/op locator before
  deciding on a new action. Protocol and lifecycle: `doc/arch/agent_daemon.md`.
- `buckyos command list` (modules and verbs of this install), `buckyos command describe <module> <verb>` (syntax,
  options, `access` read/write/privileged, `async_mode`), `buckyos <module> --help`.
- Source: `/opt/buckyos/libexec/buckyos-tool/cli/modules/<module>.ts` (`system-config` is `system_config.ts`); identity
  and endpoint in `cli/core/{auth,config,identity,runtime}.ts`; error codes and exit codes in `cli/core/errors.ts`.
- stdout: one JSON envelope `{ok, data | error{code,message,retryable}, meta{trace_id}}`; `--output text` prints a system-config value as-is; progress of waiting verbs goes to stderr.

#### Settings: where each one is managed
- Agents: `buckyos agent list`, `agent get|profile-get <agent_id>` (raw host name like `xiaobai.test.buckyos.io`, not
  the `did:web:` DID), `agent profile-set <agent_id> --display-name <n> [--bio <t>]` (given fields only, `''` clears;
  returns `before` and `profile`, which is your revert). Users, agent create/delete/settings: Desktop "Users and Agents".
- Users (definition: `product/users_and_agents/UserType.md`): account types are `admin`, `user`, `limited` (zero
  permissions until an admin puts it in a system user group) and `root` (Zone owner record, never logs in). System user
  groups (`system/user_groups/<id>`, membership in `users/<u>/settings` `groups`) and contact projection rules
  (`system/user_groups/_projections`; absent = Admin and User see each other) change only through admin control-panel
  RPCs: `user_group.list|create|update|delete|set_members|set_projections|reset_projections`, `user.change_type`,
  `user.set_account_options` (`allow_password_change` / `allow_contact_edit`, `null` = type default); no CLI verb, use
  Desktop "Users and Agents". Don't write those keys with raw system-config: the RPCs also refresh RBAC and reload
  msg-center. A group carries no RBAC policy (`g, <u>, group:<id>` only): a Limited user in a group still has no
  mailbox, tasks or uploads; app access comes from `apps.availability` group rules (`group:<id>`).
- User removal is soft deletion: Desktop Users and Agents calls `user.delete`, then reads back the list.
  Admins can open Deleted users to restore the original account (`user.change_state`, `state: active`);
  its DID, keys, password, roles, groups and data are retained. A same-name `user.create` returns `user_deleted`;
  use the explicit restore flow, not an overwrite or raw key deletion. Root and the current account cannot be disabled.
  For deletion or state changes, inspect `state`, `rbac_refreshed` and `warning`: a saved state with
  `rbac_refreshed: false` needs permission refresh, and does not prove access has been revoked everywhere.
  Read back with `user.list` (`include_deleted: true`) or `user.get`; retrying the same state retries the refresh.
  A failed list read is not an empty list. Source and RPC contract: `src/frame/control_panel/src/user_mgr.rs`
  and `doc/control_panel/Control_Panel_Service.md` §4.1; no dedicated user CLI verb.
- External access (hybrid SSO, contract `doc/arch/verify-hub/混合SSO技术契约.md`): Zone switches `self_register_enabled` /
  `did_login_enabled` / `jump_base_url` in `system/sso/settings` via admin RPC `sso.settings.get|set` (Desktop Settings ›
  Sign-in & Sharing). Exclusive invites (a Limited account + password-free link, 48 h): admin RPCs
  `user.exclusive_invite.create|list|get|revoke|rotate|retry_resource` (Desktop Users and Agents › Exclusive invites);
  the link with its secret is returned only once by `create`/`rotate` — never log it or put it in a task/message. A
  customer's resend request arrives to the issuer as a System Notify; send the rotated link through the original
  channel yourself (the system sends no email/SMS). Limited and External sessions are restricted: a service that has
  not opened a method answers kRPC `code: "access_denied"` with `data.reason` (`session_kind_not_allowed` = product
  boundary, not a setting); branch on `data.reason`, never on the error text.
- Apps: `buckyos app list|get|status|start|stop|restart|install|upgrade|uninstall|cancel`. Availability
  (`apps.availability.*`) has no CLI verb and needs a user's Control Panel login.
- Devices, network, gateway, certificates: control-panel `zone.devices.list`, `network.overview`, `gateway.overview`,
  `zone.certificates.list` are read-only and not in the CLI. `devices/<id>/info` is node-reported Info, not a setting.
- DID account invitations (`user.invite.create|get|accept`, `/users/invite/:inviteId`) require the invited DID's
  Control Panel `did_federation` session before acceptance. An invite link alone or a password session is insufficient;
  the submitted OwnerDocument must match that DID and declare this Zone. These differ from exclusive invite links.
- The local gateway's RTCP listener follows the device document's `rtcp_port` (default `2980`), rendered into
  `etc/node_gateway_params.json` at activation and startup. Inspect the document and listening socket together.
- Storage: `system status` warns on disk/memory/swap of the control-panel host. Application data lives in object buckets
  hosted by nfs-server: `buckyos bucket ls [--appid <app_id>]` (on the host the device identity sees every bucket; an Agent sees its owner's), `bucket stat <bucket_id>` (key / version counts, bytes,
  `orphaned_at` of an uninstalled App's bucket, reconcile `issues`), `bucket get obj://bucket/<bucket_id>/<key>`.
  Named-store eviction (GC actually freeing bytes of unretained objects and chunks) is
  `services/nfs-server/settings.named_store_gc_evict` (default `true`); unretained chunk bytes survive at least 24 h
  after their last write or read. Pool and bucket quotas: kRPC `bucket.get_pool_quota|set_pool_quota|get_quota|set_quota`
  (admin) and `bucket.usage_report`; no CLI verb, Desktop Settings › Storage lists per-App usage.
- AI providers: `buckyos ai providers` (auth as configured/kind, health, models; never secrets), `ai routing`,
  `ai usage [--range last1d|last7d|last30d | --since <t>] [--group-by provider_instance_name,provider_model] [--bucket day]`
  (your own calls only; an admin's Desktop Usage page can show the whole Zone); per-model health and prices:
  `ai_provider health <exact_model>|models` (agent container). `ai quota [--app <app_id>]` lists the token quota
  rules that limit you (admin rules on you and your Apps / Agents, shared App rules, your own limits) with
  settled / reserved / available and `reset_at_ms` (`null`: a total account, only an admin recharge refills it).
  Which Providers / models each user group may use, quota rules and recharges are set in Desktop › AI Center
  (Quota, Visibility); no CLI write verb. Adding/editing providers or routing: no CLI verb, the owner uses Control
  Panel AI settings.
- Developer mode, Zone display, desktop appearance: Desktop UI (`system/buckyos_dev_config` needs a sudo token the CLI lacks).
- Node Executor (Agents running commands/files on a node, `doc/arch/node_exector.md`): the setting is the `executor`
  field of `nodes/<node>/settings` (`enabled`, `allowed_devices` = exact device DIDs from `devices/<id>/info`,
  `profiles.<id>` = `run_as{uid,gid}` (non-root; omitted for Windows nodes), `workdir_roots` (absolute node paths,
  e.g. `C:\Users\<u>\ws` on Windows), `env`, `principals` = exact Agent DIDs, `ops`, and
  `limits`, and `owner_agents` = the node's policy for the zone owner's Agents: `enabled`, default `workdir_roots`,
  `env`, `ops` (empty = all), `computer` max `none|observe|control`, optional `run_as`; a Desktop node seeds it open on
  first start, absent = never initialized, `enabled:false` = closed by the administrator); writing it needs an owner
  sudo token. Which Agent uses a node is the Agent's own setting: `agent.runtime_directory [agent_id]` lists the nodes
  offered to the owner (`host` = the desktop), `agent.update {agent_id, runtimes: {"host": {enabled, computer}},
  default_runtime_id}` enables one (targets come from the directory, never from the caller). Desktop's Add Agent
  permissions step asks where the Agent runs (host or its container) when the host is open and a node runs containers;
  the directory's `host_by_default` is true on a Windows / macOS host, false on Linux (there the native Agent Service
  would run as root), and with no container node (`container_available: false`) the host is the only choice.
  `agent.create` accepts these same runtime fields; for the container, use
  `runtimes: {"host": null}, default_runtime_id: "container"`. Create retries keep the saved choice.
- A Zone owner's Agent whose default runtime is a node's `host` runs its Agent Service natively on that node: no
  `buckyos-app-<agent>` container, no `container` runtime (`runtime_unavailable`), the node config shows
  `apps/<instance>/run_mode: "native"`, and `agent.runtime_directory <agent_id>` says `agent_service: {mode: "native",
  node_id}`. Changing the default between host and container restarts the Agent Service the other way; a default of
  `container` with no container node is refused (`container_unavailable`).
- BuckyOS Desktop Lite (`system.product.get` → `edition: "lite"`; installed as `$BUCKYOS_ROOT/etc/product.json`) has
  no container runtime: docker/wslc are never probed or called, whatever is installed. It runs the system services,
  static web apps users add, the product's preinstalled script apps (natively on the node's Deno: `run_mode: "native"`,
  record `local/app_native/<instance>.json`, log `logs/apps/<instance>/native.log`) and one Agent. `system/agent_slot`
  names the Agent holding the run slot; another `agent.create` fails with `agent_slot_occupied` until that Agent is
  deleted (the slot frees once its runtime App is uninstalled). Installing a docker or script app is refused at
  `apps.inspect` / submit (`product_container_app`, `product_service_app`, `product_mixed_packages`); a spec the
  product does not run is stopped and `services/<spec_id>/schedule_status` says why (`no_suitable_node` too, when no
  node can take a spec). A native app that cannot run reports it in `services/<spec_id>/instances/<node>`
  `deployment_error`: `port_conflict` (app, service, port; free the port, it retries by itself),
  `exited` (last log lines), `not_listening`, `interpreter_missing`; restarts back off 5 s … 5 min.
  Linux `dev` / `devtest_ood1` config generation seeds a bounded host workspace at
  `data/home/devtest/host-runtime`, with a non-root execution identity and no desktop control; explicit node settings
  take precedence. The scheduler multiplies
  both into `nodes/<node>/config.executor` (never edit): a profile `agent-<12 hex>` per enabled Agent, check
  `generation` grew, the Agent is in `principal_bindings` with `enabled`/`app_installed` true (a runtime shared by two
  AgentSpecs is not mapped), and `auth_valid_until_ms` keeps moving (30 s lease). On the node:
  `etc/node_executor_acl.json` lists the allowed devices, node_daemon logs `node executor opened, store_id=…`, the
  executor listens on `127.0.0.1:3184`, Agents enter through the gateway SOCKS entry `:3185`. Its ledger is
  `local/node_daemon/executor/`; a reinstall (`start.py --all`/`--reinstall`) clears `local/`, gives a new store_id, and
  earlier runs then report StoreMismatch instead of resuming. Command/file execution: a root Linux/macOS node_daemon
  (A-system) switches to the profile's `run_as`; on macOS give it a dedicated non-admin account (no no_new_privs
  there, and uid 65534 has no account). A desktop node_daemon (Windows desktop session, macOS `BuckyOS Runtime`
  LaunchAgent) cannot switch users: profiles omit `run_as`, operations run as the desktop user with all of that
  user's rights, the Windows shell is PowerShell, and only trusted Agents should get such a profile. A profile whose
  `run_as` does not fit the node (missing on a root node, present on a desktop node) and a non-root Unix node_daemon
  outside desktop mode expose only control capabilities (`open_stream`, `describe`, `query`, `cancel`); execution,
  file operations, and workdir resolution return `Capability`.
- The rest goes through raw `system-config get|list|set|set-file|append`. Settings (user-adjustable, fine to edit):
  `users/<u>/apps/<app>/settings`, `users/<u>/agents/<agent>/{settings,profile}`, `services/<svc>/settings`,
  `users/<u>/desktop/<session>/<state>`. Config (derived by the scheduler, rewritten or broken by hand edits):
  `nodes/<node>/{config,gateway_info,gateway_config}`, `services/<svc>/info`, `system/rbac/policy` (despite the CLI's
  `append` example), `system/app_registry`, `system/scheduler/*`, installed apps' `spec` (use `app` verbs).
  `*/instances/*` and `devices/*/info` are runtime reports; `*/doc` and `boot/config` are signed identity documents.

#### Change loop

For AICC or `llm_understand_media` failures with `HTTP 504 Gateway Timeout` / `response head timeout`,
correlate the gateway log with the AICC task/trace. The shipped `etc/boot_gateway.yaml` sets
`servers.node_gateway.response_timeout: 900s`; this covers all upstream responses through that HTTP server.
Inspect the effective gateway configuration for overrides, then reload and read it back after a configuration
change. Provider and client timeouts remain independent. A timed-out caller does not prove the provider
cancelled the original request; check its outcome before repeating it. Configuration source and lifecycle:
`doc/arch/gateway/boot_gateway的配置生成.md`.
If AICC reports a provider HTTP error containing `response head timeout`, inspect the provider's proxy too:
the local gateway setting does not change a remote proxy's response budget. Older gateways may return this
diagnostic with HTTP 500; use the upstream address and AICC task/trace to locate the failing hop.

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
  A natively run Agent Service has no container: its log is the host's `$BUCKYOS_ROOT/logs/opendan/opendan.<pid>.log`
  (service `opendan`), its process record `$BUCKYOS_ROOT/local/app_native/<instance>.json` (pid, start time, port).
- Windows background services and their tool processes should run without a console window, including
  `opendan.exe install-agent-tools` during node startup. A flashing window warrants checking the launch site:
  Rust callers reuse `src/kernel/buckyos-api/src/process_util.rs`; Python launchers use `CREATE_NO_WINDOW`
  without `DETACHED_PROCESS` or `CREATE_NEW_CONSOLE`. Check logs to diagnose startup failures.
- `buckyos diagnostic collect --services a,b --since <RFC3339>` (privileged; waits for the task; result has `bundle_id`,
  expiring in about an hour) → `buckyos diagnostic export <bundle_id> --path $OPENDAN_SESSION_DIR/diag.zip` (sha256
  checked). Scheduler logs run to tens of MB per hour; narrow the window.
- Logs and bundles come redacted (`[REDACTED_TOKEN]`). Don't print tokens, `BUCKYOS_APPCLIENT_SESSION_TOKEN`, key
  files or `users/<u>/settings` (it holds a password hash); summarize the fields you need.
- "My file / image / message attachment disappeared": find the binding first — `buckyos bucket stat <bucket_id>` and
  `bucket get obj://bucket/<bucket_id>/<key>` (a delete marker or `no_such_key` = the App unbound it; an `evicted`
  availability = content gone although bound, report it). nfs-server logs one `bucket bind|unbind <bucket_id> key=…
  obj_id=… version=…` line per change: `buckyos log query --service nfs-server --keyword "<key>"`; the calling service
  logs the same line on its side.
- Host-side evidence (processes, ports, docker, node_daemon, `etc/`, `security/`, other apps' container logs): name the
  host command for the owner, e.g. `uv run src/check.py`, `docker ps`, `ls $BUCKYOS_ROOT/logs/apps/`.

#### When it fails
- HTTP 503 `service_not_ready:<service>` from `/kapi/<service>` means the gateway has no registered route for that
  service. Verify the requested service name, `services/<service>/info`, the node's `gateway_info`, and service/scheduler
  logs. For an Agent, the gateway service name is its full AgentId: use `/kapi/<agent_id>`; its own WebUI uses
  `/api/opendan` on the app host. `/kapi/opendan` is only for direct Loader access and has no global gateway route.
  For a valid service name during startup, retry after instance reporting and route publication.
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
- Agent stops replying with `bind_failed` or refresh failures: inspect `loader.status.authentication` and `errors`
  through the Agent's existing kRPC endpoint, plus `session.read` input ACK/outbox state. `retry_pending` is transient;
  `reauthorization_required`/`unavailable` needs host diagnosis. Container `Up` and old service `Healthy` do not prove
  working authentication. Keep tokens out of the evidence; record the exact owner, AppInstanceId, message ID and error.
- For authorized recovery, use the existing App lifecycle: check `buckyos command describe app restart`, then
  `buckyos app restart <full_app_instance_id> --strategy recreate`. node-daemon rechecks the active owner, current
  enabled installed App and Agent binding before issuing a fresh LoginAssertion. Do not auto-recreate on a revocation
  or denial; resolve authorization through the owner/admin first. `docker restart` keeps the expired startup assertion.
  Preserve AgentRootFS, message storage and Session ACK/outbox state. Check the original pending message is consumed
  and answered once, then observe subsequent renewal; a successful restart task alone is insufficient.
  AgentRootFS is `$BUCKYOS_ROOT/data/home/<owner>/agents/<agent_id>/` (full AgentId, not a short name or `did:web:` string).
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

Checked against: BuckyOS 0.7.0 (build261017.main.1b6e277fadd9.dirty), Tool/SDK 0.7.131 (`system info`, `log services`, `agent`, `ai` verbs from websdk source, run on the host as device and as devtest), test.buckyos.io; agent-identity results read from policy and code, not run in a container, 2026-10-17. User types, system user groups and projection RPCs: read from source and unit tests, not run on a Zone, 2026-10-21.

Provider 接入诊断：在 AI Center 查看发现模型、身份匹配、不可用预设及聊天候选。自定义 Provider 的高级配置可设置渠道规则、模型身份别名和明确的服务端默认用途；协议兼容不会自动继承厂商预设。配置验证不发推理请求，不能代替真实对话验收。具体字段以当前 `src/kernel/buckyos-api/src/aicc_client.rs` 和 `doc/aicc/provider_server_defaults.md` 为准；不要用模型特定参数修改 Agent 的媒体调用方式。
