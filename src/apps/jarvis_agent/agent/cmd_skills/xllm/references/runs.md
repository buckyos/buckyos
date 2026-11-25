#### Inspect routes and records

`--model` or `.llm_context`'s `model:` selects the route. Examples include `llm.swift`, `llm.summarize`, `llm.plan`, `llm.vision`, `llm.code`. Built-in plan/code prefer available OpenAI specs; inventory and overrides determine the actual model. `--json` disables provider web search. Under JSON mode the provider enforces a configured schema (otherwise a JSON object); xllm checks extracted JSON syntax. Check `response_model`, `usage` and the answer even on exit 0.

Family presets such as `llm.gpt-6-1-sol:medium` map effort to protocol parameters. `supported_efforts` comes from the Model Driver. SN maps OpenAI/GLM/Kimi/Qwen presets to Responses for token and device login. Check inventory `unmatched_models`, `unavailable_presets` and `missing_channel_mapping`. LLM family/spec membership is generated, so `logical_mounts` can be empty. `llm.fallback` chooses the cheapest capable tier satisfying the same requirements if the use case has no capable candidate; trace `fallback_applied`. An override with `items` or `fallback: {mode: strict}` disables fallback. Source: `src/frame/aicc/src/service/model_defaults.rs`, `src/frame/aicc/driver_metadata/models/openai.model.json`, `doc/aicc/provider_profile_schema.md`.

`status --run <id> --format json` includes prompt/input, effective configuration with sources, usage and last error. Inspect `<runs_dir>/<run_id>/run.json`, `snapshots/` and `exec/<call_id>/` for tool calls. CLI/source: `agent_tool xllm --help`, `src/frame/agent_tool/src/{run_local_llm,xllm}.rs`. For custom AICC calls, websdk exposes `helperLlmChat` in `aicc_client.d.ts` with buckyos-agent `lib/runtime.ts` for login.

#### Recover

- Exit 1 (`no_candidate_model`, `invalid_json_schema`, limits or run not found): fix the cause and start a new call. For `context_length_exceeded` or HTTP 413 (>32 MiB), xllm compacts and resends up to three times. Split input after terminal `context_capacity`.
- `resource_not_permitted`, `quota_exhausted`, `quota_budget_exceeded` are terminal. Check `buckyos ai quota` and `reset_at_ms`; a lower `--max-tokens` can fit a remaining budget.
- `unsupported_operation`: inspect the selected model and named request field. Change the unsupported option or explicitly select a suitable model; repeating the same request does not add that capability.
- Exit 2: check auth, invalid flags/input, unknown YAML keys (the error lists valid keys), or another run in that directory. A new authenticated session shell refreshes the token. A bare container shell lacks it.
- Exit 3 (pause) or 4 (SIGINT): inspect the trace and side effects. Resume with the printed `agent_tool xllm --resume --run <id> --runs-dir <dir>`. Model timeouts, retriable provider errors and HTTP 401/403 `credentials` can pause a run. Resuming resends an unfinished model request. HTTP 408/504 does not automatically replay. A 504 `response head timeout` warrants checking gateway/AICC logs with `buckyos_admin`; changing `--llm-timeout` alone cannot extend the gateway limit.
- Resume uses saved config and tool specs; model/tool overrides and edits to `.llm_context` do not replace them. Missing required saved tools make the run unresumable. You can raise time/token/iteration limits. `--system`, `--select`, files, or a currently executing run conflict with `--resume`. For `yield` or `tasks_running` wait for the event, then append `"<message>"` to the resume command to continue in context.
- Interrupted tool calls may already have acted. A recorded shell exit supplies its result; unconfirmed writes/commands return `result unknown` for inspection. Reads can run again. The model may try the action again after interruption.
- Exit 5/6: re-export the saved raw reply with `agent_tool xllm result --run <id> --result-format raw --output answer.txt`.

Original checks used BuckyOS/xllm 0.7.0, October 2026 DV Zone, and November 2026 host builds for provider JSON Schema.
