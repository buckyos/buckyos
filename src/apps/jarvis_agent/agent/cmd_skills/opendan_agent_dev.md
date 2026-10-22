---
name: opendan_agent_dev
description: Develop OpenDAN Agents — AgentRootFS layout, agent.toml, behaviors, cmd_skills and TypeScript CLI tools on the buckyos SDK, sessions and report artifacts, debugging, packaging an Agent template PIKG, registering it and creating an Agent from it, and extending agents with skill packages.
requires: an OpenDAN session (agent-session on PATH) and the buckyos command; registering templates and creating Agents is done by the owner (Control Panel/Desktop or host CLI); host debugging needs the BuckyOS repo
---
#### When to use
Making a new Agent template or changing one (identity text, behaviors, session classes), writing a cmd_skill with its commands, packaging/installing skill packages, or explaining how Agents, templates and Apps relate. Packaging, test installs, waiting for tasks and publish limits are shared with Apps and live in `buckyos_dev_release`; when it is not in this session, read `$OPENDAN_AGENT_ROOT/cmd_skills/buckyos_dev_release.md`. Ordinary Apps are `buckyos_app_dev`.

#### The pieces
- Template = a PIKG with `categories:["agent"]` (`app_type agent`), sub package `agent` (the AgentRootFS files) and optionally `agent_skills`; it needs a `www` http endpoint (Jarvis: inner_port 4060). Jarvis is the built-in one (`bundled:jarvis.buckyos.bns.did`); your own package working copy is readable at `/opt/buckyos/bin/<agent_id>/`.
- Agent = its own DID created at `agent.create`: AgentId `<name>.<zone>` = AppId of a per-Agent App built from the template, AppInstanceId `<agent_id>@<owner>`, `AgentSpec.binding` → that instance and service `www`. Control Panel generates the key (system-config `users/<owner>/agents/<agent_id>/key`); the Loader writes `.meta/identity.json` and `.meta/role_supplement.md`. A template therefore carries no keys, sessions, memory or state.
- AgentRoot (`$OPENDAN_AGENT_ROOT`, host `data/home/<owner>/.local/share/<agent_id>/agents/<agent_id>/`) = template files + instance data (`.meta/`, `state/`, `sessions/`, `memory/`, `notebook/`, `workspace/`, installed skills). On a template update (agents with `template_auto_update`), new files are added and files you never changed are replaced; files changed locally are kept (with a warning), files dropped from the package stay, installed skills are untouched, and running sessions keep what they froze.
- The owner's creation choice `permission_mode` is `read_write` by default or `read_only`. Owner home follows that mode; your own `$OPENDAN_AGENT_ROOT` always stays writable for sessions, memory, workspaces, tools and skills. This is stored in `AgentSpec`, not in `agent.toml` or editable agent settings; template updates preserve it.

#### Before you start
```
ls $OPENDAN_AGENT_ROOT; cat $OPENDAN_AGENT_ROOT/agent.toml
agent-session skill list            # installed cmd/goal skills: name, description, requires, origin
```

#### AgentRootFS and agent.toml
Files: `agent.toml`, `role.md` + `self.md` (identity text), `behaviors/<name>.toml` (`__INCLUDE(rel/path)__` for shared `.inc`), `cmd_skills/<name>.md`, `goal_skills/`, `tools/`, `i18n/<lang>.toml` (`[outbound]` texts). The fastest start is a copy of the Jarvis package, trimmed. Shape of `agent.toml` (sections are strict, unknown keys fail):
```toml
[runtime]
language = "en"
[llm_context]
model = "llm.chat"
[llm_context.provider]
type = "buckyos"
[loader]
agent_state_service = true
webui = true
[[loader.ui]]
on = "msg.chat"            # msg.group for groups (session_class "group", sid_strategy "per_group")
session_class = "ui"
sid_strategy = "per_peer"
[loader.goals]
enabled = false
[session.ui]
default_behavior = "chat_route"
default_cmd_skills = ["ui_core"]   # ≤ 8, files in cmd_skills/
[session.work]
default_behavior = "plan"          # Work manuals come from create-worksession --cmd-skill
```
Other `[session.<class>]` keys: `base`, `turns`, `objective`, `ask_user_timeout_secs`, `max_process_depth`, `max_sub_sessions`, goal `budget.*`. Every `default_behavior`, its `meta.next` chain and each `behavior:<name>` reference have to exist, or session creation fails ("the agent has no behavior …").

#### Behaviors
`behaviors/<name>.toml`: `[meta] name` (= file name), `objective`, `next`; `[prompt] mode = "function_call"|"behavior"`, `system`, `on_init`…; `[capabilities] tool_whitelist` / `action_whitelist` (`group:bash`, `group:bash_core`, `report`, `behavior:<name>`); `[budget]`, `[model] preferred` (`llm.chat`, `llm.plan`, `llm.code`, `llm.swift`); `[entry] mode`; `[tool]` to expose a behavior as a tool; `[reply] schema`. Copy `work_simple.toml` (one bounded task) or `plan.toml`/`do.toml`. Try a change with a throw-away Work: `agent-session create-worksession --objective "<t>" --behavior <name>` (add `--wait` to watch it run); a parse or reference error comes back at creation.

#### cmd_skills and their commands
- `cmd_skills/<name>.md`: front matter `name`/`description`/`requires` (one line each), body with `####` headings; the whole body goes into the system prompt of sessions that chose it, frozen for their life. A Work picks ≤ 8 at creation: `agent-session create-worksession --objective "<t>" --cmd-skill <name> [--cmd-skill …]`; a child Work inherits its parent Work's choice.
- Commands of a skill: executables in `tools/<name>/bin/`, on PATH for sessions that chose `<name>`; `$OPENDAN_AGENT_ROOT/tools/` itself is on PATH for every session.
- TS + SDK is the preferred way to reach BuckyOS: the model is `/opt/buckyos/bin/opendan/buckyos-agent/` (`commands/gen_image.ts` = annotated 6-step recipe: argv → typed request → `initRuntime()` login → call → fetch results → one `AgentToolResult` JSON; helpers in `lib/`). A small skill command:
```sh
#!/bin/sh
# tools/<name>/bin/<cmd>
exec deno run --config /opt/buckyos/libexec/buckyos-tool/cli/deno.json --allow-net --allow-read --allow-write --allow-env "$(dirname "$0")/../src/<cmd>.ts" "$@"
```
In `src/<cmd>.ts` the smallest call (it listed control-panel apps from the host with a user token; in a container it runs as your agent):
```ts
import { buckyos } from "buckyos";
const rpc = new buckyos.kRPCClient(`${Deno.env.get("BUCKYOS_TOOL_ENDPOINT")}/kapi/<service>`, Deno.env.get("BUCKYOS_APPCLIENT_SESSION_TOKEN"), Date.now());
console.log(JSON.stringify(await rpc.call("<method>", {})));
```
For login handling, results and errors reuse buckyos-agent `lib/` (`initRuntime()`, `emit()`) through that tool's `deno.json`. RPC names and params are in the CLI sources `/opt/buckyos/libexec/buckyos-tool/cli/modules/*.ts`. Print JSON on stdout, exit non-zero on failure. Keep the bin wrapper executable (`chmod +x`).

#### Sessions, workspace, deliverables
A Work runs in its session dir (`$OPENDAN_SESSION_DIR`) or its bound workspace (`--workspace inherit|none|<id>`); write outputs there and list them in the final report (`report` → `artifacts`, or `<artifacts>["rel/path"]</artifacts>` in behavior mode); files outside the work dir are refused, accepted ones are copied and get object ids. A Work's history: `agent-session read-session <sid> --report --worklog 50`; files under `<session>/.opendan_agent_session/` (`worklog.jsonl`, `session_config.json` with the frozen config).

#### Package a template
```
mkdir -p tpl/dapp_meta && cp -r <your agent dir> tpl/agent      # agent.toml, role.md, behaviors/, cmd_skills/, tools/, i18n/
# tpl/dapp_meta/app.json: copy Jarvis's (src/apps/jarvis_agent/dapp_meta/app.json) and set did "did:bns:<name>.<owner>", name, owner/author, show_name, version; keep categories ["agent"] and the www endpoint
# tpl/dapp_meta/pikg.json: {"schema_version":1,"output_dir":"../dapp_dist","pikg_file":"<name>-<ver>.pikg","sub_pkgs":{"agent":{"required":true,"source":{"type":"path","path":"../agent"}}}}
cd tpl && buckyos pikg build ./dapp_meta && buckyos pikg pack ./dapp_dist && buckyos pikg info ./dapp_dist/<name>-<ver>.pikg
```
There is no `pikg init --kind agent`. Leave `.meta/`, `state/`, `sessions/`, `memory/`, keys and tokens out of `agent/`. A source outside the cwd needs `--allow-read <dir>`.

#### Register the template and create an Agent (owner)
- Register: `buckyos app fetch <pikg> --plan p.json --policy local-developer` then `buckyos --yes app install <pikg> --plan p.json --policy local-developer` → `action template_registered`, `template_id installed:<app_id>` (no task; a newer version → `template_updated`; from the code, not run on 2026-10-17). There is no command to unregister a template; use throw-away names in tests.
- Create: Desktop "Add Agent" (pick the template and read-write/read-only access) or control-panel kRPC `agent.create {idempotency_key, name, template_id, permission_mode?, template_auto_update?, allow_group?}` then `agent.create.status {agent_id}`; `permission_mode` accepts `read_write` (default) or `read_only`. Read `agent.list_templates`, `agent.list`, `agent.get`. No `buckyos` CLI verb exists for these. Hand the owner these steps; your agent identity is not expected to create Agents.
- Check: the new Agent answers in Message Center, `agent.get` shows `template installed:<app_id>`, `loaded_version` and the selected `permission_mode`, its AgentRoot has a fresh `.meta/identity.json`. Verify read-only user data and writable AgentRoot in the deployed container; host debugging uses host process permissions.

#### Extend with skill packages
```
mkdir -p skills/cmd_skills skills/tools/<name>/bin        # skills/cmd_skills/<name>.md, skills/tools/<name>/bin/<cmd>
buckyos --non-interactive pikg init . --name <pkg> --owner did:bns:<owner> --kind agent-skills --source ./skills
buckyos pikg build ./dapp_meta && buckyos pikg pack ./dapp_dist
agent-session skill install ./dapp_dist/<pkg>-<ver>.pikg     # or a skills/ dir; --replace for name clashes
buckyos skill install <pikg> --agent <agent_id>               # same from outside (owner), also list|remove
agent-session skill remove <name> [--kind cmd|goal]
```
Installed files go to `cmd_skills/`, `goal_skills/`, `tools/<name>/` and `.meta/skills.json` (package, version, digest); a `tools/<name>` without a matching manual is refused. Sessions created afterwards see the change; running ones keep their frozen text. A template can also ship skills as its own `agent_skills` sub package. `app publish` rejects `agent_tools` sub packages: ship commands inside a skill package.

#### Debug
- Inside: `agent-session sessions --children --active`, `read-session`, your logs under `/opt/buckyos/logs/`.
- Host, in `buckyos/src`: `./debug_jarvis.sh <owner> --agent <agent_id> [--no-build]` runs that Agent's Loader in the foreground from source; `JARVIS_PACKAGE_ROOT=<agent dir>` points it at your package. `opendan --dev --agent-root <dir> --agent-did <did> --queue-dir <dir>` runs without a zone. Container changes need `./build_aios --local-test` and a recreated container (`buckyos_dev_release`). Tests: `cargo test -p opendan --test loader -- --test-threads=1`, `cargo test -p libopendan -- --test-threads=1`.

#### When it fails
- `unknown method skills.list|install` from `buckyos skill`: that Agent's container predates the skills kRPC; it needs a newer aios image and a recreated container.
- WebUI `503 service_not_ready:opendan`: check that the app uses `/api/opendan` on its own host. The zone gateway reserves `/kapi/<name>` for named services; cross-app callers use `/kapi/<agent_id>` with the full AgentId. See `src/frame/opendan/README.md` for endpoint and login details.
- Session creation errors name the missing behavior or cmd_skill, or `> 8` / duplicate manual names.
- Template install refused: `app_type` not `agent`, App DID/AppId mismatch, or no `www` endpoint.
- Your edits to template files in your own AgentRoot survive template updates as local changes; to share them, put them into the template package.

#### Source and extending
Current repository contracts: [PIKG CLI](https://github.com/buckyos/buckyos-websdk/blob/main/doc/modules/pikg.md), [package format](https://github.com/buckyos/buckyos/blob/main/doc/pikg/format.md), [App configuration](https://github.com/buckyos/buckyos/blob/main/doc/sdk/app-package-config.md). This manual supplies the task recipe; schemas and historical proposals are maintained in those documents.
Loader `src/frame/opendan/src/{loader,rootfs,config}.rs`, sessions/skills `src/frame/lib_opendan/src/{template.rs,skills.rs,state/behaviors.rs,bin/xagent.rs}`, templates/agents `src/frame/control_panel/src/agent_mgr.rs` (BuckyOS repo, host). In the container: buckyos-agent sources, the Tool CLI and SDK `.d.ts` under `/opt/buckyos/libexec/buckyos-tool/`.

Checked against: BuckyOS 0.7.0, Tool/SDK 0.7.131, opendan/lib_opendan working tree; Jarvis template packaged and fetched as a plan on test.buckyos.io, template registration/agent creation/skill install not run, 2026-10-17.
