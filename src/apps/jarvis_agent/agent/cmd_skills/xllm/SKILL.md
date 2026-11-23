---
name: xllm
description: Write or debug a model call in a macro or script using `agent_tool xllm`. Pass text, files or stdin, return text or structured JSON, and inspect or resume the run.
requires: `agent_tool` on PATH and the session shell's own token (BUCKYOS_APPCLIENT_SESSION_TOKEN, BUCKYOS_APP_ID); models come from the Zone's AICC routes
baseline: standard
---
#### Call a model from a macro or script

Use this CLI when the program needs a model step, such as extracting fields from text. It uses the session token or the node session's `.opendan/` token exchange. AI Workspace wish macros use the host's `aiws.ai.complete` or `aiws.llm.map` API. When the Agent itself owns a bounded task, execute and verify it directly in `work_simple` so progress, tool calls and the final report stay in one session. Use `llm_understand_media` for media or PDFs.

```sh
mkdir -p xllm/notes && cd xllm/notes
printf 'runs_dir: ./runs\n' > .llm_context
agent_tool xllm --file notes.txt "List the decisions and open questions"
agent_tool xllm --system "Translate to French. Output the translation." "Welcome home"
agent_tool xllm --output answer.md "Summarize the following material: …"
```

Use a new task directory for this example. Place inputs there or pass their paths. Repeat `--file` in order or pipe text on stdin. The default records under `~/.xllm/runs` are shared across sessions. `.llm_context` is YAML merged from parents to the working directory; setting it at the session root also affects `llm_understand_media`. See `agent_tool xllm --help` and `agent_tool xllm status --format json` for installed flags and effective config.

#### Structured output

`--json` requests a JSON object by default. Put `json_schema` in `.llm_context` for a provider-enforced schema. On a strict OpenAI route include every property in `required`, allow `null` for missing values and set `additionalProperties: false`.

```yaml
json_schema:
  type: object
  properties: { name: {type: string}, age: {type: [integer, "null"]} }
  required: [name, age]
  additionalProperties: false
```

`--format json` wraps the CLI result with `run_id`, `status`, `answer`, `usage`, `error`, `response_model` and `resume_command`; it can be combined with `--json`. `--result-format result.a.b` selects a JSON/XML field. Set `--model` to a Zone route (default `llm.chat`) or exact `model[:variant]@provider`. Check the actual `response_model`.

#### Bounded tool work

`agent_tool xllm --tools --max-tool-iterations 4 --timeout 300 "Inspect the files here and summarize them"` enables file tools and shell. By default files stay in the work directory. Shell commands have the agent's permissions. Run one tool-enabled job per work directory and avoid nesting. If a shell call outlives its wait, follow its task to `<<task_result>>`. For the native desktop `computer` tool, Agent Daemon executor, loop choices and limits, read `$OPENDAN_AGENT_ROOT/cmd_skills/xllm/references/tools.md`.

#### Check and recover

Inspect `agent_tool xllm status --run <id>`, read the answer, check extracted values against the input, and open generated files. A completed run can exhaust its tool budget or invent a schema value. Attach verified output paths in the Work report. The run's artifact count covers writes through file tools.

Use `agent_tool xllm list` and `agent_tool xllm result --run <id> --result-format raw` to find runs and re-export answers without another call. Exit 1 is terminal, 2 usage/config/busy, 3 paused, 4 interrupted, 5 output write failure, 6 extraction/JSON failure. For 3/4 inspect the original task and any tool side effects before using the printed resume command. For routing, quota, gateway timeouts, result recovery and resume options, read `$OPENDAN_AGENT_ROOT/cmd_skills/xllm/references/runs.md`.
