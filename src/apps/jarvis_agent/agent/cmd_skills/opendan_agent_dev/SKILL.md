---
name: opendan_agent_dev
description: Develop OpenDAN Agents — AgentRootFS layout, agent.toml, behaviors, work and plan skills and TypeScript CLI tools on the buckyos SDK, sessions and report artifacts, debugging, packaging an Agent template PIKG, registering it and creating an Agent from it, and where skill packages and skill drafts fit.
requires: an OpenDAN session (agent-session on PATH) and the buckyos command; registering templates and creating Agents is done by the owner (Control Panel/Desktop or host CLI); host debugging needs the BuckyOS repo
baseline: standard
commands: deno, node
---
#### Explicit remote executor tools
The `executor` tool group provides `executor_list` and `executor_call` without moving the Agent Session.
Discover a permitted online executor and use its exact executor_id, generation, profile_id and store_id.
Linux supports exec/read/write/edit/query/cancel; UI, input and capture remain unsupported.
macOS 14+ and Windows interactive user sessions add native ComputerUse drivers through the authenticated Command Stream and SDK
`ExecutorCommandStream.computer`; it does not add an Agent tool or change Session Runtime binding.
Windows runs the foreground `buckyos_agent_daemon.exe`, uses PowerShell for command execution and
stores its ledger under `%LOCALAPPDATA%\buckyos-agent-daemon` by default. Its desktop driver supports
the primary display with visual/semantic/hybrid/auto observations, click, Unicode type, keypress, line
scrolling, delay waits, and UI Automation element targets with invoke/set_value. Dragging and pixel
scrolling are unsupported. Do not run it in Session 0. macOS offers the same semantic observation and
element targets through Accessibility. The semantic implementation entry points are
`src/kernel/computer_driver/src/semantic.rs` (common names, UIA and AX mappings, outline),
`desktop.rs` (observe/auto, element resolution), the UIA worker in `native_windows.cpp` and the AX
backend in `native.m` (both export `bucky_semantic_*`).
Desktop grants, local pause/resume and recovery are documented in `doc/arch/agent_daemon.md`.
On Windows, node startup and OpenDAN helpers launch console tools without opening a console window;
use the service logs and command stdout/stderr for diagnostics. Rust launch sites reuse
`src/kernel/buckyos-api/src/process_util.rs`.
For `session.helper` on Windows, long Session working directories use the same directory's 8.3 path
when starting xagent; relative file arguments still resolve inside the Session. If the filesystem
provides no usable short path, use a shorter BuckyOS data location and inspect
`src/frame/lib_opendan/src/session_context.rs` and the reported directory.
If the Windows input monitor stops being healthy after sleep, a debugger pause or a scheduling stall,
restart the daemon and check readiness; local resume alone cannot restore it. Query old operations and observe again.
On a native runtime the `computer` tool attaches each screenshot to its tool result as an image
(`AgentToolResult.images` → `Observation::Success.images`, never parsed from command output); a
function-call context keeps the newest `MAX_TOOL_RESULT_IMAGES`, a behavior (text) loop drops them.
A semantic observation attaches no image: its outline is printed as plain lines after the result JSON
(`src/frame/agent_tool/src/runtime/computer.rs`).
The planned `buckyos computer` CLI and its `--present` bridge for shell commands are not yet available;
do not publish a `computer_use` recipe requiring those commands.
The CLI alternative is `buckyos executor --help`; inspect the installed command schema before use.
Management URL credentials authenticate the target daemon only, while tool calls use the Agent's App session.
After connection loss, rediscover the active generation and query the original operation using its
operation_call_id and original store_id; do not replay commands. A different store_id cannot recover
the previous machine's ledger. See `doc/arch/agent_daemon.md`.

#### When to use
Making a new Agent template or changing one (identity text, behaviors, session classes), writing a work skill with its commands, packaging skill packages, or explaining how Agents, templates and Apps relate. Packaging, test installs, waiting for tasks and publish limits are shared with Apps and live in `buckyos_dev_release`; when it is not in this session, read `$OPENDAN_AGENT_ROOT/cmd_skills/buckyos_dev_release/SKILL.md`. Ordinary Apps are `buckyos_app_dev`.

#### The pieces
- Template = a PIKG with `categories:["agent"]` (`app_type agent`), sub package `agent` (the AgentRootFS files) and optionally `agent_skills`; it needs a `www` http endpoint (Jarvis: inner_port 4060). Jarvis is the built-in one (`bundled:jarvis.buckyos.bns.did`); your own package working copy is readable at `/opt/buckyos/bin/<agent_id>/`.
- Agent = its own DID created at `agent.create`: AgentId `<name>.<zone>` = AppId of a per-Agent App built from the template, AppInstanceId `<agent_id>@<owner>`, `AgentSpec.binding` → that instance and service `www`. Control Panel generates the key (system-config `users/<owner>/agents/<agent_id>/key`); the Loader writes `.meta/identity.json` and `.meta/role_supplement.md`. A template therefore carries no keys, sessions, memory or state.
- AgentRoot (`$OPENDAN_AGENT_ROOT`, host `data/home/<owner>/agents/<agent_id>/`) = template files + instance data (`.meta/`, `state/`, `sessions/`, `memory/`, `workspace/`, installed skills). On a template update (agents with `template_auto_update`), new files are added and files you never changed are replaced; files changed locally are kept (with a warning), files dropped from the package stay, installed skills are untouched, and running sessions keep what they froze.
- The owner's creation choice `permission_mode` is `read_write` by default or `read_only`. Owner home follows that mode; your own `$OPENDAN_AGENT_ROOT` always stays writable for sessions, memory, workspaces, tools and skills. This is stored in `AgentSpec`, not in `agent.toml` or editable agent settings; template updates preserve it.

#### Before you start
```
ls $OPENDAN_AGENT_ROOT; cat $OPENDAN_AGENT_ROOT/agent.toml
agent-session skill list            # work / ui / plan / goal skills: name, description, what each needs, origin (--kind work|plan|goal, --scope ui|work)
```

#### AgentRootFS and agent.toml
Files: `agent.toml`, `role.md` + `self.md` (identity text), `behaviors/<name>.toml` (`__INCLUDE(rel/path)__` for shared `.inc`), `cmd_skills/<name>/SKILL.md`, `plan_skills/<name>/SKILL.md`, `goal_skills/<name>/SKILL.md`, `tools/` (the Agent's own commands), `i18n/<lang>.toml` (`[outbound]` texts). The fastest start is a copy of the Jarvis package, trimmed. Shape of `agent.toml` (sections are strict, unknown keys fail):
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
default_cmd_skills = ["ui_core"]   # ≤ 8, cmd_skills/<name>/SKILL.md with scope ui; the owner's set (`buckyos skill ui-add`) replaces it
[session.work]
default_behavior = "plan"          # Work manuals come from create-worksession --cmd-skill
completion = "explicit_report"     # only report(is_end=true) finishes a work session
tick_secs = 30                     # while its tasks run, a tick opens a Turn this often (0: never)
[session.plan_simple]              # a class of the package: base + overrides
base = "work"
default_behavior = "plan_simple"
input = "queue"                    # it takes messages: the user's can be forwarded to it
tick_secs = 600
```
A work session has many Turns: a reply (or a yield) while work it started still runs — tasks, sub sessions it must hear from — closes the Turn as the progress its caller sees, and the run stays where it is; the end of a task (`<<task_result task=… call_id=… status=finished|failed|unknown>>`, its full result), a sub session's end, or a tick (`timer:task_watch`, every `tick_secs`) opens the next Turn in that same run with the whole tool budget. A reply with nothing running and no final report gets one correction (`system:runner`, `nudge`); a second one fails the session (`missing_end_report`). `report(is_end=true)` is refused while a task it started is still running.

Other `[session.<class>]` keys: `turns`, `objective`, `completion` (`natural` | `explicit_report`), `input` (`queue`: the session takes messages; `none`: its queue only takes controls — every session has a queue, so pause, resume and stop reach it — and `agent-session post` to it is refused), `tick_secs`, `max_process_depth`, `max_sub_sessions`; a class with `base` takes only the built-in's defaults, not another class's overrides. `[session.goal]` also sets when a goal's run opportunity becomes an inference: `min_interval_secs` (hard interval, default 900), `guarantee_secs` (hard guarantee, default 259200, 0 off, otherwise at least 3600), `check_every_secs` (a goal with a check script and no timetable, default 300), `budget.activation` (`max_total_tokens`, `max_runs`, `max_wall_secs`), `budget.period` (`window_secs`, `max_total_tokens`, `max_activations`, `max_works`), `agent_budget` (`window_secs`, `max_total_tokens`: every goal and its work together); a goal's check script is `goal_checks/<name>.ts` for a built-in goal, or what the goal sets with `agent-session goal check set` (contract in Agent State Protocol §9.4). Every `default_behavior`, its `meta.next` chain and each `behavior:<name>` reference have to exist, or session creation fails ("the agent has no behavior …").

Run limits in `[llm_context]` (or a session's `llm_context`): `timeout` (seconds, default 3600) caps one continuous stretch of running — it restarts whenever the run resumes, time between Turns and waiting for tasks or sub sessions is not counted, and a session itself has no lifetime cap. `context_window` (tokens) is normally left out: a run takes the window of the model its alias routes to (AICC `max_context_tokens`, 128k when unknown); write it to cap the window (a large window means every Round resends up to its 80% line, slower and costlier). At `context_yield_ratio` of the window (default 0.80) the run is compacted in place — its history so far goes to the worklog, and the session history rebuilt from it is compressed by an LLM summary (`llm.summarize`) into the opening input and first round, one summary with the exact strings kept verbatim, and the most recent rounds as they were — and goes on at about `prompt.context_target_ratio` of it (session config, default 0.30; `history_budget_tokens` is the least history kept). Files written in the session directory are untouched by compaction: re-read only what you need. A run out of time, or out of `[budget] max_total_tokens`, is not failed at once: once per Turn its clock restarts and it gets one `<<budget>>` line and only its wrap-up tools, as when its tool budget is used up.

#### Behaviors
`behaviors/<name>.toml`: `[meta] name` (= file name), `objective`, `next`; `[prompt] mode = "function_call"|"behavior"`, `system`, `on_init`…; `[capabilities] tool_whitelist` / `action_whitelist` (`group:bash`, `group:bash_core`, `report`, `behavior:<name>`); `[capabilities] call_intent = true` (function_call only: every native call must carry a non-empty `intent` argument — added as required to each function's schema, checked before the call runs, kept in the worklog, removed before the tool gets the arguments); `{{ background_context }}` in an input template (`on_init` / `on_input` / `on_context_switch`) places the background: what changed since it was last shown — semi-subscription state, the new output of the tasks the session follows (only what is new, at most 2 KB per task), the Memory clues of a UI or Work session's batch (`hint: …`, `noted, not yet consolidated: …`, revisions; a clue counts as shown only when the input contains its line, otherwise it comes again; `{{ hints }}` lists the same lines) — as a `<<background_context>>` block; semi-subscription is never a message of its own, so a template that does not place it leaves the session blind to it (the built-in templates place it); `[capabilities] workspace = "read_only"` (a session entered with the behavior only reads its workspace); standard tools (`read`, `read_file`, `write_file`, `edit_file`, `shell`) take an optional `intent` without `call_intent`, shown as the Turn's progress (the Turn task's message and, while the Turn is open, the session's `one_line_status`; a call without one shows `running <tool>`); `[budget]`: `max_tool_iterations` counts tool batches and is runaway protection, not a work quota — a multi-Turn Work refills it every Turn; with a fifth left the model gets one `<<budget>>` line (never a worklog entry); used up, an explicit-report function call run keeps only `report` (and `ask_user` where it is a function; `wrap_up_tools = [...]` sets the list) for at most 2 batches, any other tool then fails the run (`budget_exhausted`); `max_wallclock_ms` / `max_total_tokens` override the wallclock / token cap of the whole run and are normally left out; `[model] preferred` (`llm.chat`, `llm.plan`, `llm.code`, `llm.swift`); `[entry] mode`; `[tool]` to expose a behavior as a tool; `[reply] schema`. Copy `work_simple.toml` (one bounded task; its `on_input` template shows how to word a task's end, a Todo update, a tick and the nudge) or `plan_simple.toml` (TodoList plan; its `on_input` words a work session's end, a forwarded user message and a tick). Try a change with a throw-away Work: `agent-session create-worksession --title "<t>" --objective "<t>" --behavior <name>` (its end comes back as your input; `agent-session read-session <sid> --worklog 50` shows it running); a parse or reference error comes back at creation.

#### cmd_skills and their commands
- Work skill = `cmd_skills/<name>/SKILL.md` (+ optional `references/*.md`, read on demand with `read $OPENDAN_AGENT_ROOT/cmd_skills/<name>/references/<file>`): front matter `name`/`description`/`requires`/`commands`/`scope`/`baseline`/`platforms`, one line each. `commands: ffmpeg, jq` names what the manual relies on beyond the standard layer, so `agent-session skill status` / `buckyos skill status` can say what is missing and which extension package brings it; it never blocks anything. `scope: work` (default) is for work sessions, `scope: ui` for conversations only, `scope: ui, work` for both; a ui skill (`ui_core`) is a work skill with that scope. Body with `####` headings; the whole body goes into the system prompt of sessions that chose it, frozen for their life. A Work picks ≤ 8 at creation: `agent-session create-worksession --title "<t>" --objective "<t>" --cmd-skill <name> [--cmd-skill …]`; a child of a `work_simple` / `explore` Work inherits its choice, a plan's Todo gets only what `todo add --skill` names. A single `.md` file directly under `cmd_skills/` is not a skill.
- Plan skill = `plan_skills/<name>/SKILL.md`: a method for splitting, ordering and accepting one kind of objective, read by `plan_simple` in its first Turn. Front matter `name`/`description` only (no `scope`, no `commands`); no concrete commands and no named work skills in the body. Never given with `--cmd-skill`.
- A behavior with `[capabilities] cmd_skills = "candidates"` (`plan_simple`) freezes no manual: the names its creator gave with `--cmd-skill` appear in its first input as `<skill_candidates>`.
- A skill is text only: it brings no command, no script and no binary, and installing one only copies its `SKILL.md` and `references/`. Commands come from the standard layer, from extension packages the owner enables (`buckyos ext-tools`), or from the Agent package itself: executables in `$OPENDAN_AGENT_ROOT/tools/` are on PATH for every session of this Agent on its private runtime.
- TS + SDK is the preferred way to reach BuckyOS: the model is `/opt/buckyos/bin/opendan/buckyos-agent/` (`commands/gen_image.ts` = annotated 6-step recipe: argv → typed request → `initRuntime()` login → call → fetch results → one `AgentToolResult` JSON; helpers in `lib/`). A small command of the Agent package:
```sh
#!/bin/sh
# tools/<cmd>
exec deno run --config /opt/buckyos/libexec/buckyos-tool/cli/deno.json --allow-net --allow-read --allow-write --allow-env "$(dirname "$0")/src/<cmd>.ts" "$@"
```
In `tools/src/<cmd>.ts` the smallest call (it listed control-panel apps from the host with a user token; in a container it runs as your agent):
```ts
import { buckyos } from "buckyos";
const rpc = new buckyos.kRPCClient(`${Deno.env.get("BUCKYOS_TOOL_ENDPOINT")}/kapi/<service>`, Deno.env.get("BUCKYOS_APPCLIENT_SESSION_TOKEN"), Date.now());
console.log(JSON.stringify(await rpc.call("<method>", {})));
```
For login handling, results and errors reuse buckyos-agent `lib/` (`initRuntime()`, `emit()`) through that tool's `deno.json`. RPC names and params are in the CLI sources `/opt/buckyos/libexec/buckyos-tool/cli/modules/*.ts`. Print JSON on stdout, exit non-zero on failure. Keep the wrapper executable (`chmod +x`).

#### Sessions, workspace, deliverables
`agent-session workspace list`, `runtime list` and `sessions` print compact, readable lists by default. Add `--json` when a script needs the full workspace/runtime records or structured session summaries; filters such as `--available`, `--children` and `--active` work in either format. `workspace get <id>` / `runtime get <id>` return a single complete record.
A host Session's service commands obtain the Agent's current identity from `.opendan/session.json`; `agent_tool dcrontab`, `agent_tool check_task` and `agent_tool cancel_task` use that Agent token through the node gateway. Installed aliases use the same identity. They need no local `.buckycli`, device key or container Zone configuration; injected Agent identity takes precedence over development RPC overrides. Inspect arguments with `agent_tool <command> --help`; implementations are in `src/frame/agent_tool/src/dcrontab_tool.rs` and `src/frame/agent_tool_cli_dev/src/lib.rs`.
A Zone host adds `## Objects` to each session's system text (before `## Commands`): the Agent `obj://<agent_id>`, the session `obj://<agent_id>/sessions/<sid>`, its workspace, the Agent's and the owner's message sessions, the owner's HomeStation home and files, the visible AI Workspaces (`obj://aiworkspace`). A session object publishes `changed` on every reported state change and `finished` once (kevents of `obj://<agent_id>/sessions/<sid>`). `read` follows these and other system URLs with the Agent's identity; a memory whose scope names an object URL brings it back on its hint (`object="…"`).
Automatic session IDs use `<kind>-YYYYMMDD-N` (for example `work-20260110-23`), with the allocator host's local date and the next daily number shared by that Agent's session kinds. Session directories remain `sessions/<session_id>/`; retries reuse the ID reserved for the same creation key. A Work runs on one working runtime for its whole life, frozen at creation (`session_config.json` → `runtime.frozen`: `runtime_id`, `connector`, `caps` with the declared `baseline`, `grant`): a chat always runs on the agent's **private runtime** (`private`; `container` / `local` are its old names — the agent's own environment, its aios container or the Agent Service process when it runs natively, always available, with the full `$BUCKYOS_ROOT` layout; `extended` in the aios container, the host's declared baseline when native); any other session takes `--runtime <id>`, else the parent's, else the private runtime — the owner's default runtime (`host` = the owner's computer) only names where the owner's absolute paths are read and where work goes when the owner asks for it on their machine. When the Agent Service runs natively as the owner's OS user on that node (Desktop Lite), `private` and owner default refer to one native runtime; `host` is a settings alias. Shell and files run locally without a network connection. Desktop operations use the `computer` tool over local IPC to node_daemon's existing driver, with the same profile authorization and operation ledger. The private and owner default entries in chat context have the same runtime ID. `$BUCKYOS_ROOT/tools/bin` and `$BUCKYOS_ROOT/bin/agent-tools` are on both the command and tool-check PATH after the session and Agent skill tools. A session whose working runtime is the private one is a multi-runtime session: it has `agent-session` and reaches other runtimes with `agent-session run --runtime <id|workspace> [--cwd <dir>] -- <cmd>` and `agent-session copy`; a session on another runtime is a single-machine session with only that machine's `shell` and tools and no `agent-session` (nothing is installed on nodes for it). The workspace never chooses the runtime: a workspace is `runtime + directory + url` on its own runtime (`agent-session workspace create --runtime <id> --directory <abs path on it> --name <n> [--url https://…] --key <k>`, `import`, `set-url`; a runtime whose grant has no `workspace` refuses one); a Todo takes `--runtime` / `--workspace` independently. Commands always start in the session's working directory on its working runtime (`$OPENDAN_SESSION_DIR` here; `<account home>/.opendan/<agent>/sessions/<sid>` on a node, its OS and paths — `C:\…` on Windows); the workspace directory is in the runtime context (and `$OPENDAN_WORKSPACE_DIR` here): `cd` to it when it is on the same runtime, otherwise move files with `agent-session copy --from workspace:<path> --to session:<path>` and back (one regular file per copy, hash-verified, `--replace` to overwrite, `copy status|cancel <id>`) and run commands there with `agent-session run --runtime workspace -- <cmd>`. On a node the standard commands (`llm_understand_media`, `buckyos`, `aicc-tool`, …) are the node's own (`$BUCKYOS_ROOT/bin/agent-tools`) and take the agent's identity from the token in `.opendan/`. Runtime records (`agent-session runtime list|get|check [--commands a,b]`) carry `connector` (`native | node_executor | ssh`), `caps` (platform, baseline, tool checks), `grant` (ops, roots, computer, install, workspace, expiry), `usage` and `lifecycle` (`retired` records are kept, never candidates). Write outputs in the session's working directory and list them in the final report (`report` → `artifacts`, or `<artifacts>["rel/path"]</artifacts>` in behavior mode); files outside it are refused. An artifact is a relative file, an `obj://bucket/…` a command returned (the AIGC commands keep their results in that bucket and print it as `detail.files[].url`), or an object id. Accepted files are stored and bound in the Agent app's `artifacts` bucket (`documents`: the owner's, kept when the Agent is removed, backed up; key `<hash>/<name>`; `buckyos bucket ls obj://bucket/user/<agent app id>/<owner>/artifacts`); an `obj://bucket/…` bound elsewhere (an AICC result) is copied into it; a storage or binding failure is an `artifact` error. A `report` tool call returns it for correction and retry; a behavior XML report keeps the live run and records `last_error.kind = "artifact"`. Restore storage access and resume that session so the saved report can finish without repeating its completed actions; inspect `read-session <sid> --report --worklog 50` before starting replacement work. Intermediate files that should not stay go to the 7-day `scratch` bucket of the same app (`buckyos bucket put`, or `uploadNamed(…, { bucket: "scratch" })` in `/opt/buckyos/bin/opendan/buckyos-agent/lib/bucket.ts`); a local file of a bucket object: `materialize_resource 'obj://bucket/…' <path>`. Say where a delivery into the workspace landed. `create-worksession --title` (required, cut to 60 chars) names the work where the owner sees it — its Turn tasks and its card under the conversation's reply; a Todo's work session takes the Todo's title. A Work's history: `agent-session read-session <sid> --report --worklog 50`; files under `<session>/.opendan_agent_session/` (`worklog.jsonl`, `session_config.json` with the frozen config).

#### Plans with Todos: plan_simple
A Work is a plan session by default (`[session.work] default_behavior = "plan_simple"`; `--class plan_simple` lets it take the user's forwarded messages and gives it a 600 s tick): it only reads and keeps its TodoList (`agent-session todo …`, list in `.opendan_agent_session/todos.json`), native function calls each with an `intent`, many Turns, ends with `report(is_end=true)`. Its workspace binding is read-only (`[capabilities] workspace = "read_only"`); the work sessions of its Todos write the workspace.
- `todo add` (work, `work_simple`, inherits the workspace) and `todo explore` (read-only, `explore`, no workspace). Without `--after` a Todo runs after the Todo added before it; `--after none` starts it at once; `--after T01 --after T03` names prerequisites. There is no dispatch: the host starts every Todo whose prerequisites completed, at every change of the list and whenever a work session ends (before the plan infers again), as many at once as `--max-children` / `max_sub_sessions` allow. A prerequisite that failed, timed out, was stopped or cancelled blocks the Todos after it (`blocked_by`) until `todo retry`, `todo update --after …` or `todo cancel`; a work session that cannot be created blocks the Todo with the reason.
- `todo update <id> --reason <why> [--content -|--accept|--scope|--evidence|--after|--runtime|--workspace|--skill]` changes a Todo (history in `updates`); a running Todo's work session gets an Input event `todo:<id>` / `updated` with the whole new task and adjusts (`work_simple`'s `on_input` has the `is_todo_update` branch). `todo pause|resume|stop <id> --reason <why>` reaches that work session as `control(pause|resume|stop)`: paused, it infers no more (a run stops at its next inference boundary, its tasks run on), resumed it goes on from its checkpoint. `xagent ctl <sid> pause|resume` is the same control for any session (the owner's side panel and home page use the service's `session.pause` / `session.resume`). A stop has a scope: `turn` stops the Turn in progress and keeps the session; `session` ends a Work with its sub sessions, puts a Goal Session's Goal to sleep, and is refused by a conversation (a conversation ends only when it is archived: `/clean`, `ui.rotate`). After the host restarts, sessions that were working are not continued by themselves: they are paused as `interrupted` and go on when the owner resumes them (a resume reaches the sub sessions interrupted with them).
- There is no review: the plan accepts a result on the evidence in its report, or checks it with a verification Todo (`todo explore "verify …" --after <id>`) when that is not enough; the plan may end whenever it judges the objective done or impossible, and work still running is stopped with it (its attempts end `stopped`). A work session's end, its questions and the user's forwarded messages open the plan's Turns; a Todo has no event or background line of its own (its work session's end event names the Todo and what became of the Todos after it; `agent-session todo list` shows the whole list).

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
- Check: wait for `agent.create.status` to report `ready`, then verify the new Agent answers its first message in Message Center. Readiness includes msg-center contact synchronization even without Telegram; `msg_center_reload_failed` at `start` is retryable with `agent.create.retry` after restoring msg-center. `agent.get` shows `template installed:<app_id>`, `loaded_version` and the selected `permission_mode`, its AgentRoot has a fresh `.meta/identity.json`. Verify read-only user data and writable AgentRoot in the deployed container; host debugging uses host process permissions.

#### Extend with skill packages
Only the owner installs, removes, disables and enables skills, and edits the set conversations get: on the agent's Skills page (OpenDAN WebUI) or with `buckyos skill install|remove|disable|enable|ui-add|ui-remove … --agent <agent_id>`. An Agent has `agent-session skill list|status|revise` and no install command; when a user asks for a skill, tell them the page or the command. What an Agent can produce is a draft under `$OPENDAN_AGENT_ROOT/skills_drafts/<slug>/` (behavior `skill_import`, manual `skill_authoring`), which the owner reads and installs.

Writing a skill, trying it from a directory, lint rules, packaging (`buckyos pikg init --kind agent-skills`) and publishing: read `doc/sdk/skill-dev-quickstart.md` in the BuckyOS repo (https://github.com/buckyos/buckyos/blob/main/doc/sdk/skill-dev-quickstart.md); the format reference inside an Agent is the `skill_authoring` manual. A template can also ship skills as its own `agent_skills` sub package.

#### Debug
- Inside: `agent-session sessions --children --active`, `read-session`, your logs under `/opt/buckyos/logs/`.
- Host, in `buckyos/src`: `./debug_jarvis.sh <owner> --agent <agent_id> [--no-build]` runs that Agent's Loader in the foreground from source; `JARVIS_PACKAGE_ROOT=<agent dir>` points it at your package. `opendan --dev --agent-root <dir> --agent-did <did> --queue-dir <dir>` runs without a zone. Container changes need `./build_aios --local-test` and a recreated container (`buckyos_dev_release`); a natively run Agent Service is the node's own `$BUCKYOS_ROOT/bin/opendan/opendan`, so it follows the node install (`buckyos-build.py` + `start.py`), not the image. Tests: `cargo test -p opendan --test loader -- --test-threads=1`, `cargo test -p libopendan -- --test-threads=1`.

#### When it fails
- MessageHub links ending in `.bns.did`: inspect `/opt/buckyos/etc/machine.json` and `web3_bridge.bns`. The file is
  part of the host's read-only `etc/` mount, and AppService uses it to initialize DID hostname conversion.
  Updating this behavior requires both node-daemon and aios, then a recreated container; see `buckyos_admin`.
- Auth failure: read `loader.status.authentication` (`state`, `last_error`, `error_code`, `denial`, `next_retry_time`)
  and `errors`, then `session.read` for persistent `bind_failed`, input ACK and outbox. Renewal is serialized in the
  shared Rust runtime; new mail operations and tool calls take its latest token. Keep auth bind errors recoverable,
  preserve unacknowledged input and existing outbox IDs, and do not report recovery before a successful credential exchange.
- `reauthorization_required` or `unavailable`: use an authorized owner/host session and the existing
  `buckyos app restart <full_app_instance_id> --strategy recreate` flow after checking current owner/App/Agent grants
  (`buckyos_admin`). This recreates with a newly signed startup assertion and retains AgentRootFS; `docker restart`
  reuses the old assertion. Do not copy device keys into the container or automatically relogin after revocation.
  Verify the same pending message has one reply and later renewal succeeds; `Up` is not an Agent health check.
- `unknown method skills.list|install` from `buckyos skill`: that Agent's container predates the skills kRPC; it needs a newer aios image and a recreated container.
- WebUI `503 service_not_ready:opendan`: check that the app uses `/api/opendan` on its own host. The zone gateway reserves `/kapi/<name>` for named services; cross-app callers use `/kapi/<agent_id>` with the full AgentId. See `src/frame/opendan/README.md` for endpoint and login details.
- A provider that is down or overloaded (timeout, 503 / 429 marked retriable): the run is kept and the host retries it after 30 s, doubling up to 10 min (`read-session` shows `retry` in its state, the registry `retry_not_before_ms`); a user message retries at once, other inputs wait. After 6 retries in a row or 30 minutes the Turn fails with `last_error.kind = "provider_unavailable"` (`retries_exhausted`, `attempts`, the last error) — a session nobody answers ends failed, and its Todo shows `provider_unavailable: … gave up after N retries`. This is not llm_context's `error_policy.max_consecutive_errors` (errors inside one run).
- Session creation errors name the missing behavior or cmd_skill, or `> 8` / duplicate manual names.
- Template install refused: `app_type` not `agent`, App DID/AppId mismatch, or no `www` endpoint.
- Your edits to template files in your own AgentRoot survive template updates as local changes; to share them, put them into the template package.

#### Source and extending
Current repository contracts: [PIKG CLI](https://github.com/buckyos/buckyos-websdk/blob/main/doc/modules/pikg.md), [package format](https://github.com/buckyos/buckyos/blob/main/doc/pikg/format.md), [App configuration](https://github.com/buckyos/buckyos/blob/main/doc/sdk/app-package-config.md). This manual supplies the task recipe; schemas and historical proposals are maintained in those documents.
Loader `src/frame/opendan/src/{loader,rootfs,config}.rs`, sessions/skills `src/frame/lib_opendan/src/{template.rs,skills.rs,state/behaviors.rs,bin/xagent.rs}`, templates/agents `src/frame/control_panel/src/agent_mgr.rs` (BuckyOS repo, host). In the container: buckyos-agent sources, the Tool CLI and SDK `.d.ts` under `/opt/buckyos/libexec/buckyos-tool/`.

Checked against: BuckyOS 0.7.0, Tool/SDK 0.7.131, opendan/lib_opendan working tree; Jarvis template packaged and fetched as a plan on test.buckyos.io, template registration/agent creation/skill install not run, 2026-10-17.
