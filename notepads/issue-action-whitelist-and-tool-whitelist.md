# Issue: split behavior `tool_whitelist` and `action_whitelist`

## Background

Current behavior config uses one `[capabilities].tool_whitelist` for both:

- provider-native / traditional function-call tools exposed by `ToolManager`;
- XML behavior-loop actions parsed from `<actions>...</actions>`.

This works in the current runtime because XML actions are lowered into `AiToolCall` and then pass through the same tool policy path. However, it mixes two different concepts in the behavior schema.

## Problem

From the behavior author's point of view, a behavior can conceptually use both surfaces in the same step:

- **tools**: model/provider-native function calls, registered and advertised through `ToolManager`;
- **actions**: behavior-loop XML action tags such as `exec_bash`, `read`, `write_file`, `edit_file`, `subscribe_event`, `unsubscribe_event`.

Even if most real behaviors choose one style in practice, using a single `tool_whitelist` makes the schema ambiguous:

- it is unclear whether an entry is intended as a function tool or an XML action;
- future additions to the action set may collide with function tool names;
- prompt authors cannot explicitly state that a behavior should allow function tools but disallow XML actions, or the reverse;
- `approval_required` semantics become harder to reason about because it is not obvious which invocation surface it applies to.

## Proposed Direction

Introduce separate capability declarations:

```toml
[capabilities]
tool_whitelist = [
  "try_create_worksession",
  "forward_msg",
]

action_whitelist = [
  "read",
  "exec_bash",
  "write_file",
  "edit_file",
  "subscribe_event",
  "unsubscribe_event",
]
```

Suggested semantics:

- `tool_whitelist`: only provider-native / ToolManager function tools.
- `action_whitelist`: only behavior-loop XML action tags.
- Empty list should have explicit meaning rather than accidentally meaning "all"; recommended default should be decided during schema migration.
- `approval_required` should either be split as well, or documented as applying to the normalized invocation name after both tools and actions are lowered.

## Compatibility / Migration

For the current v0 schema, keep `tool_whitelist` as the existing combined whitelist until a planned breaking config migration.

During migration:

1. Add `action_whitelist` to `CapabilitiesCfg`.
2. Keep parsing legacy `tool_whitelist` as combined whitelist only during the migration window, or intentionally break without compatibility if beta config policy allows it.
3. Update XML behavior parser / dispatcher so action filtering happens against `action_whitelist` before lowering or dispatch.
4. Keep ToolManager advertisement filtered by `tool_whitelist`.
5. Update Jarvis behavior TOMLs and `doc/opendan/Agent配置改进.md`.

## Acceptance Criteria

- Behavior config can independently allow function tools and XML actions.
- XML actions not listed in `action_whitelist` are rejected or ignored with a clear observation.
- Provider-native tool calls not listed in `tool_whitelist` are still rejected by the existing tool policy.
- Existing Jarvis behaviors declare their intended action surface explicitly.
