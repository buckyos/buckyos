# Issue: 拆分 behavior 的 `tool_whitelist` 和 `action_whitelist`

## 背景

当前 behavior 配置使用同一个 `[capabilities].tool_whitelist` 同时表达两类能力：

- 由 `ToolManager` 暴露的 provider-native / 传统 function-call tools；
- 从 `<actions>...</actions>` 中解析出来的 XML behavior-loop actions。

当前运行时可以这样工作，是因为 XML actions 会被降级为 `AiToolCall`，然后经过同一条 tool policy 路径。但在 behavior schema 里，这混合了两个不同概念。

## 问题

从 behavior 作者的角度看，一个 behavior 在概念上可以在同一个 step 里使用两种调用界面：

- **tools**：model/provider-native function calls，通过 `ToolManager` 注册并暴露；
- **actions**：behavior-loop XML action tags，例如 `exec_bash`、`read`、`write_file`、`edit_file`、`subscribe_event`、`unsubscribe_event`。

即使大多数实际 behavior 在实践中只会选择其中一种风格，使用单一 `tool_whitelist` 仍然会让 schema 产生歧义：

- 不清楚某个条目到底是 function tool 还是 XML action；
- 未来新增 action 时，可能会和 function tool 名称冲突；
- prompt 作者无法明确表达一个 behavior 应该允许 function tools 但禁止 XML actions，或反过来；
- `approval_required` 的语义更难推理，因为不明显它到底适用于哪一种调用界面。

## 建议方向

引入独立的能力声明：

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

建议语义：

- `tool_whitelist`：只表示 provider-native / ToolManager function tools。
- `action_whitelist`：只表示 在执行层是否方向，如果在提示词中不暴露action，通常不会被调用。
- 空列表有明确语义: 相当于disable,在behavior-loop里，tool_whitelist经常为空
- `approval_required` 要么也拆分，要么明确文档化为：在 tools 和 actions 都降级后，应用到规范化的 invocation name 上。

## 兼容性 / 迁移

不需要做兼容性考虑，breaking change

## 验收标准

- Behavior config 可以独立允许 function tools 和 XML actions。
- 未列在 `action_whitelist` 中的 XML actions 会被拒绝，或被忽略并返回清晰的 observation。
- 未列在 `tool_whitelist` 中的 provider-native tool calls 仍然会被现有 tool policy 拒绝。
- 现有 Jarvis behaviors 会显式声明它们预期使用的 action surface。
