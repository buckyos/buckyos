---
name: xllm
description: One-shot model calls from the shell with `agent_tool xllm` — analysis, rewriting, translation and JSON/schema extraction over text, files or piped output, plus small tool-assisted jobs in a scratch directory, with run records you can inspect, re-export and resume.
requires: `agent_tool` on PATH and the session shell's own token (BUCKYOS_APPCLIENT_SESSION_TOKEN, BUCKYOS_APP_ID); models come from the Zone's AICC routes
baseline: standard
---
#### When to use
- One-off work on material you already have: classify, summarize, compare, rewrite, translate, pull fields
  into JSON, get a second opinion. The answer comes back on stdout (or a file) in seconds to minutes.
- A bounded helper with tools: a model inspects or transforms files in a scratch directory under a small budget.
- Durable work, PLAN/DO, or anything that needs the user fits a Work session better: xllm has no conversation
  and cannot ask anyone. Nor does it drive or take over a session: session runs
  (`.opendan_agent_session/runs`) are driven by their host, and xllm refuses them while the host runs them
  or still has to validate/commit their result.
- Understanding images, PDFs, audio or video: `llm_understand_media`. Generating media: the `aigc_*` manuals.

#### Before you start
- `xllm` is not on PATH; call it as `agent_tool xllm …`.
- In a session shell it logs in to AICC with the agent's token from the environment (refreshed per shell call,
  ~15 min life); on a node session-runtime the `agent_tool` entry asks your agent for that token over the session's
  `.opendan/` token and loads Zone configuration through the node gateway; no local `.buckycli` or private key is needed.
  Without either (a bare `docker exec`) you get exit 2 `cannot initialize BuckyOS runtime …`.
- Check: `agent_tool xllm --run-logs warn "Reply with: ok"` prints `ok`, exit 0.
- A directory per task keeps runs and files apart:
  `mkdir -p xllm/<task> && cd xllm/<task> && printf 'runs_dir: ./runs\n' > .llm_context`.
  Records then stay with the session; the default `~/.xllm/runs` sits inside the container, shared by all
  sessions. A `.llm_context` at the session root would also be merged into `llm_understand_media` calls there.

#### Find the real commands
- `agent_tool xllm --help`: every flag and exit code. `agent_tool xllm status [--run <id>]`: effective config
  with the source of each value, limits, usage, last error, resumability; `--format json` adds prompt and input.
- `.llm_context` is YAML, looked up from the work dir upward and merged parent → child. An unknown key fails
  with exit 2 and lists the keys your build accepts — the quickest way to see what it supports.

#### Shortest path
```sh
agent_tool xllm "Classify the sentiment of: '…' (positive/negative/mixed, one-sentence reason)"
agent_tool xllm --file notes.txt "List the decisions and open questions"   # --file repeatable, order kept
some_command | agent_tool xllm "Summarize the errors in this log"         # stdin becomes material
agent_tool xllm --system "Translate to French. Output the translation." "…"
agent_tool xllm --output answer.md "…"                                    # save instead of print
agent_tool xllm --tools --max-tool-iterations 4 --timeout 300 "…"         # file tools + shell in the work dir
```
Structured result with a schema: there is no CLI flag; put it in the work dir's `.llm_context`, and stdout
is the JSON value:
```yaml
json_schema:
  type: object
  properties: { name: {type: string}, age: {type: [integer, "null"]} }
  required: [name, age]
  additionalProperties: false
```
The Zone's `llm.chat` lands on an OpenAI strict route: every property listed in `required`,
`additionalProperties: false`, optional fields as a `null` type — otherwise exit 1 `invalid_json_schema`.

#### Inputs and outputs
- Model: `llm.chat` by default, a logical name AICC routes. This Zone also routes `llm.swift` (fast, cheap),
  `llm.summarize`, `llm.plan`, `llm.vision`, `llm.code` (xhigh reasoning, slow); an exact
  `model[:variant]@provider` skips routing. Set it with `--model` or `model:`; `--format json` shows
  `response_model`, the model that answered.
- Built-in `llm.plan` and `llm.code` prefer available OpenAI specs; Zone/session overrides and provider
  availability can change the result. Check `response_model`, and use AICC's
  `src/frame/aicc/src/service/model_defaults.rs` and
  `src/frame/aicc/driver_metadata/models/openai.model.json` for current priorities and GPT family/effort bindings.
- A fixed family preset such as `llm.gpt-6-1-sol:medium` is an AICC routing name. The remote model ID stays
  unchanged; AICC converts the selected effort to protocol parameters. Custom providers without dedicated
  rules use the matching protocol's default reasoning mappings. A preset with no supported mapping remains
  unavailable; see `doc/aicc/provider_profile_schema.md` for the selection boundary.
- Model Driver `supported_efforts` defines each model's reasoning levels. SN maps OpenAI, GLM, Kimi and Qwen
  presets to Responses parameters with either API Token or device login, without a separate model whitelist.
  If `llm.chat` / `llm.code` / `llm.plan` has no candidates, check inventory `unmatched_models` and
  `unavailable_presets` before changing routing. `missing_channel_mapping` means the preset lacks a channel
  conversion; an empty LLM `logical_mounts` alone is normal because family/spec membership is generated.
- Here `llm.chat`/`llm.plan` add provider web search: a trivial call bills ~4.7k input tokens (mostly cached)
  against ~0.3k on `llm.swift`. `--json` turns web search off for that run (JSON mode excludes it).
- `--json` (or `json: true`) constrains the model result: JSON mode at the provider, the schema when
  `json_schema` is set, and a JSON check on the extracted result (exit 6 if not JSON). xllm itself does not
  check the reply against the schema; the provider enforces it.
- `--format json` wraps the CLI result in an envelope (run_id, status, is_terminal, resumable, answer, usage,
  error, response_model, resume_command). It does not constrain the model; scripts can combine both.
- `--result-format result.a.b` takes one field of a JSON (or XML) reply. `--loop-model behavior` replies in
  XML (`<report end="true">…`); `--result-format result.report` gives its text.
- Records: `<runs_dir>/<run_id>/run.json`, `snapshots/`, tool commands under `exec/<call_id>/`. `list` shows
  runs of the current work dir; `result --run <id> [--result-format …] [--output f]` re-exports with no
  model call.
- Hand results over by naming the files in your Work report artifacts. The run's own `artifacts` counts files
  written through its file tools, not files written by shell commands.

#### Tools, budgets and boundaries
- Tools are off unless you pass `--tools` or set `tools: {enabled: true}`. You then get read_file, write_file,
  edit_file, shell (+ wait_task, get_task_state, cancel_task). File tools stay inside the work dir
  (`filesystem_policy: workspace`, the default; elsewhere: `path not allowed by policy`); shell is not
  sandboxed and runs with the agent's permissions.
- The default loop is `function_call`; `actions`/`tools2actions` need `--loop-model behavior`.
- To target an online Agent Daemon from an authenticated BuckyOS runtime, add
  `groupname: executor` to the `.llm_context` tool list:

  ```yaml
  tools:
    enabled: true
    tools:
      - groupname: bash
      - groupname: executor
  ```

  This adds `executor_list` and `executor_call` while keeping the current runtime.
  Copy the discovered `executor_id`, `generation` and `store_id`, then choose
  `profile_id` from `profiles`. Query or cancel an accepted operation by its returned
  `operation_call_id`; after reconnection rediscover the generation and retain
  the original store. Unknown outcomes and invalid locators must not be replayed.
  Linux targets expose commands and files only; UI is unsupported. The current
  tool schema comes from `src/frame/agent_tool/src/runtime/executor.rs`.
  Windows target commands use PowerShell 7 from PATH or Windows PowerShell; do not send Bash syntax.
  Native Windows/macOS desktop operations use the SDK Computer protocol described in
  `doc/arch/agent_daemon.md`; this does not add Computer commands to `executor_call` or deliver PNGs to the model.
  Shell users can inspect the equivalent CLI with
  `buckyos command describe executor exec` and `buckyos command describe executor query`.
- Limits: `--max-tool-iterations` (default 8), `--timeout` total seconds (3600), `--llm-timeout` per request
  (600), `--max-tokens` per reply. With the tool budget spent the model still answers, perhaps saying it
  could not finish (exit 0).
- One tool-enabled run per work dir at a time; a second gets exit 2 `already has a tool-enabled run`.
- Nothing stops a tool run's shell from calling xllm again; leaving nesting out avoids stacked cost under
  separate budgets. Split the job yourself instead.
- A call that outlives your shell call's wait continues as a task: give the shell call a larger `wait_ms`, or
  follow it (`agent-session task wait <task_id>` or the `wait_task` tool) rather than starting a second copy.

#### When it fails
Exit codes: 0 done · 1 failed, limit reached, or run not found · 2 usage/config/input error or busy (no model
call) · 3 paused, resumable · 4 interrupted, resumable · 5 done but `--output` not written · 6 done but result
extraction or JSON check failed (raw reply saved).
- `no_candidate_model` (unknown model), `invalid_json_schema`, and
  `413 Payload Too Large` (request over AICC's 1 MiB RPC body: split the material) end the run with exit 1.
  Fix the call and start a new one.
- Exit 3 (model timeout, transient provider error, rejected or expired token) → the printed hint
  `agent_tool xllm --resume --run <id> --runs-dir <dir>`. Resume reuses the saved config:
  later `.llm_context` edits and `--model`/`--tools` are ignored; `--llm-timeout`, `--timeout`, `--max-tokens`,
  `--max-tool-iterations` can be raised. Resume resends the unfinished model request; an interrupted HTTP
  connection does not prove the provider stopped processing it. Check the original task/trace before retrying.
- `HTTP 504 Gateway Timeout` with `response head timeout` means the gateway stopped waiting for the service.
  Check `buckyos_admin` and the gateway/AICC logs; increasing only `--llm-timeout` cannot extend a gateway limit.
  HTTP 408/504 keeps the upstream error text and pauses the run; it does not automatically replay the request.
- Exit 4 (Ctrl-C/SIGINT): resumable. The interrupted tool call is not replayed, but the model may run it
  again; mind commands that are not idempotent.
- Last error kind `quota_exhausted` / `quota_budget_exceeded` / `resource_not_permitted` (exit 1,
  `status --run <id>`): AICC refused the call under the AI quota or model permissions of the user it is
  charged to. Retrying does not help until the quota resets or the user changes the model; report it
  (`buckyos ai quota` shows the rules and `reset_at_ms`). A lower `--max-tokens` can fit `quota_budget_exceeded`.
- Exit 1 runs are final: `--resume` on them shows the saved record (with limit flags: exit 1 `is terminal`).
- Exit 5/6: `agent_tool xllm result --run <id> --result-format raw [--output f]`.
- `--resume` with a new question, or on a run another process is executing: exit 2.

#### Check the result
- Exit 0 means the run completed, not that the answer is right. A schema can force invented values
  (`"age": 0` for a text without an age); with the budget spent the model may report it could not finish.
  Read the answer, spot-check extractions against the source, open the files a tool run wrote.
- `agent_tool xllm status --run <id>` shows status, model, usage and the last error.

#### Source and extending
- The implementation is Rust and not shipped in the agent runtime (buckyos
  `src/frame/agent_tool/src/run_local_llm.rs` for the CLI, `xllm.rs` for the SDK); in the runtime, `--help`,
  `status --format json` and `run.json` are the readable truth.
- For a call xllm cannot express, websdk's AICC client has `helperLlmChat` (types:
  `/opt/buckyos/libexec/buckyos-tool/dist/aicc_client.d.ts`; login: buckyos-agent's
  `lib/runtime.ts`). A derived TS script goes to `$OPENDAN_AGENT_ROOT/tools/` and runs with
  `deno run --config /opt/buckyos/bin/opendan/buckyos-agent/deno.json`; not yet exercised for LLM calls.

Checked against: BuckyOS 0.7.0, xllm 0.7.0 (aios image 2026-10-12 in the agent container; host build 2026-10-17 for json_schema, `--json` without web search and the resume hint), DV Zone routes via codex-proxy, 2026-10-17.
