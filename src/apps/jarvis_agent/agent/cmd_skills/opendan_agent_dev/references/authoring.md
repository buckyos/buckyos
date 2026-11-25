#### AgentRootFS and classes
The Loader overlays template files on `data/home/<owner>/agents/<agent_id>/`. In addition to `agent.toml`, use `role.md` and `self.md` for identity, `behaviors/<name>.toml` (`__INCLUDE(rel/path)__` for shared snippets), `i18n/<lang>.toml` (`[outbound]` texts) and `tools/` for Agent commands. A template auto-update adds new files and replaces untouched ones; locally edited or retired files and installed skills stay. `AgentSpec.permission_mode` governs owner home (`read_write` default or `read_only`); AgentRoot stays writable.

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
on = "msg.chat"
session_class = "ui"
sid_strategy = "per_peer"
[loader.goals]
enabled = false
[session.ui]
default_behavior = "chat_route"
default_cmd_skills = ["ui_core"]
[session.work]
default_behavior = "plan_simple"
completion = "explicit_report"
tick_secs = 30
tick_max_secs = 600
[session.plan_simple]
base = "work"
default_behavior = "plan_simple"
input = "queue"
tick_secs = 1800
tick_max_secs = 14400
waits_idle = true
max_life_secs = 172800
```

Use `msg.group`, `group` and `per_group` for groups. Class sections are strict; `base` inherits built-in defaults only. Set `turns`, `objective`, `completion`, `input` (`queue` for messages, `none` for controls only), `tick_secs` / `tick_max_secs` (a waiting Work is ticked that long after its last reply, then after twice as long each time; another input starts over), `waits_idle` (it may wait for something outside while nothing runs), `max_life_secs` (a Work older than this only wraps up), `max_process_depth` and `max_sub_sessions` as needed. Validate each `default_behavior`, `meta.next` and `behavior:<name>` or Work creation fails. Goal scheduling: `min_interval_secs` (900), `guarantee_secs` (259200, 0 off, otherwise ≥3600), `check_every_secs` (300). Cap activations via `budget.activation` (tokens, runs, wall time), periods via `budget.period` (window/tokens/activations/works), or all goals plus their work via `agent_budget`. Built-in goal checks live in `goal_checks/<name>.ts`; a user goal can use `agent-session goal check set` (Agent State Protocol §9.4).
`[loader.browser]` enables on-demand Chromium from ext-core; defaults `enabled=true`, `idle_stop_secs=600`, `max_tabs=12`. Private sessions use `agent-browser` / `$OPENDAN_BROWSER_RPC`. The owner uses Agent Browser. Native profile `.cache/browser/`, tabs `state/browser/groups.json`; missing Chromium affects browser availability.

#### Behavior loops
Use the package's `work_simple.toml` / `plan_simple.toml` as templates. Define `[meta] name` (same as filename), `objective`, `next`; `[prompt] mode` (`function_call` or `behavior`), `system` and input templates. `[capabilities] tool_whitelist` / `action_whitelist` expose tools, groups or other behaviors. Set `workspace="read_only"` for read-only entry. With `call_intent=true` each native function call needs an `intent`; it is logged and stripped before dispatch. Optional intents on standard read/write/shell tools appear as progress. `{{ background_context }}` in `on_init`, `on_input` or `on_context_switch` delivers changed subscriptions, new task output (up to 2 KB each) and UI/Work memory hints. Omitting it hides updates; hints recur until shown. Other sections include `[model] preferred`, `[entry] mode`, `[tool]` and `[reply] schema`.

`[budget] max_tool_iterations` counts batches, refilling each Turn. At one fifth remaining a budget line is emitted. At exhaustion an explicit-report function loop gets up to two wrap-up batches (default `report` and `ask_user` when available; `wrap_up_tools` overrides). `max_wallclock_ms` and `max_total_tokens` cap the run. Context defaults to its routed model's `max_context_tokens` (fallback 128k). `context_window` caps it. At `context_yield_ratio` (0.80) history is saved to the worklog and summarized with opening context and recent rounds to `prompt.context_target_ratio` (0.30); `history_budget_tokens` is the floor. Session files persist. `llm_context.timeout` (3600 s) counts continuous running, resets on resume and excludes waits. Reaching time/token budget allows one wrap-up per Turn.

#### Skills and command implementations
`agent-session skill list --kind work|plan|goal` / `--scope ui|work` discovers manuals. Work fronts use one line each for `name`, `description`, `requires`, `commands`, `scope`, `baseline` and `platforms` as applicable. `commands` feeds `skill status` diagnostics. The selected bodies (≤8 unique work/UI names) freeze at creation. UI uses its owner's set or `default_cmd_skills`. `work_simple`/`explore` children inherit the parent's choice; plan Todos get `todo add --skill` choices. `cmd_skills="candidates"` freezes candidate names only, appearing in `<skill_candidates>`. A plan manual provides decomposition, ordering and acceptance, without shell commands or concrete work skills. It is read by the plan, never given with `--cmd-skill`.

Develop TypeScript tools with the installed `/opt/buckyos/bin/opendan/buckyos-agent/commands/gen_image.ts` recipe: argv, typed request, `initRuntime()`, SDK call, fetch, `emit()` with one `AgentToolResult`. Use the same `deno.json` for its `lib/` helpers. Keep `tools/<cmd>` executable on the private runtime:

```sh
#!/bin/sh
# tools/<cmd>
exec deno run --config /opt/buckyos/libexec/buckyos-tool/cli/deno.json --allow-net --allow-read --allow-write --allow-env "$(dirname "$0")/src/<cmd>.ts" "$@"
```

In `tools/src/<cmd>.ts` a minimal call is:
```ts
import { buckyos } from "buckyos";
const rpc = new buckyos.kRPCClient(`${Deno.env.get("BUCKYOS_TOOL_ENDPOINT")}/kapi/<service>`, Deno.env.get("BUCKYOS_APPCLIENT_SESSION_TOKEN"), Date.now());
console.log(JSON.stringify(await rpc.call("<method>", {})));
```

Print JSON on stdout and use nonzero exit for failure. Consult `/opt/buckyos/libexec/buckyos-tool/cli/modules/*.ts` for methods and params. Skills ship no executables. See `doc/sdk/skill-dev-quickstart.md` for drafts and `buckyos pikg init --kind agent-skills` for a skills package.
