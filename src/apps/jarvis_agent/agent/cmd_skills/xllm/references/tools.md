#### Tool and time budgets

Tools default to off. `--tools` or `tools: {enabled: true}` enables read_file, write_file, edit_file, shell, wait_task, get_task_state and cancel_task. `filesystem_policy: workspace` limits file tools. Shell has the agent's permissions. `--max-tool-iterations` defaults to 8, `--timeout` to 3600 seconds total, `--llm-timeout` to 600 seconds per request, and `--max-tokens` limits one reply. The model can return a partial answer when the tool budget is spent. A concurrent tool-enabled run in the same directory exits 2.

The default loop is `function_call`. `actions`/`tools2actions` require `--loop-model behavior`. For XML behavior replies (`<report end="true">…`), use `--result-format result.report`.

#### Native desktop

Owner-permitted native sessions expose `computer` over local IPC. Start with `{"request":{"command":"inspect","binding_id":""}}` or `{"request":{"command":"observe","binding_id":"","mode":"auto"}}`. Empty binding selects the local display. `inspect` lists commands and observation modes. `auto` selects a usable semantic tree or a screenshot and reports `selection_reason`. Other modes are `visual`, `semantic` and `hybrid`.

Read `status.result.computer` for execution, feedback, errors and observation. Semantic output lists controls such as `[e12] tab "Gmail" selected (select) @x,y wxh`. `semantic.windows` lists window refs `wN` and access status. Scope another observation with `"scope":{"kind":"window","observation_id":…,"ref":"wN"}`. A screenshot is an absolute PNG path; function-call results also carry the latest images. Point targets use `space:"screenshot"` and the observation's width/height.

Act on an element using `{"kind":"element","observation_id":…,"ref":"eN"}`. `click` and `scroll` are physical. `type`/`keypress` focus first. `invoke`, `set_value`, `toggle`, `select`, `expand` and `collapse` use the element's advertised action and must appear in `inspect`. Use a current id for this binding before `expires_at_ms` and the new observation after each action. On `state_mismatch`, `stale_element`, `stale_observation` or a changed screen, observe again. Resolve an unknown execution outcome before another action. Desktop permission or IPC failures leave shell and file tools available.

#### Remote Agent Daemon

Add this to an authenticated runtime's `.llm_context`:

```yaml
tools:
  enabled: true
  tools:
    - groupname: bash
    - groupname: executor
```

Use `executor_list`, copy `executor_id`, `generation`, `store_id`, then choose `profile_id` from `profiles` for `executor_call`. Query/cancel an accepted operation by `operation_call_id`. After reconnection rediscover the generation and retain the original store. An unknown outcome or invalid locator requires inspection before another execution.

Linux targets expose commands/files. Windows commands use PowerShell 7 or Windows PowerShell. Desktop operations on native Windows/macOS use the separate SDK Computer protocol (`doc/arch/agent_daemon.md`). For remote commands inspect `buckyos command describe executor exec` / `buckyos command describe executor query` or `src/frame/agent_tool/src/runtime/executor.rs`.
