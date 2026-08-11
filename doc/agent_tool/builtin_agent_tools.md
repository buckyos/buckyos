# Builtin Agent Tools 设计整理

本文整理 `src/frame/agent_tool` 中 builtin agent tool 的设计约定，重点关注三件事：

1. 执行函数如何收敛到统一契约
2. 输入参数 schema 如何接入 function call
3. 输出格式怎么稳定落到 `AgentToolResult`

本文只覆盖当前工程内已经实现并对外暴露的 builtin tools，不覆盖外部 bash 命令，也不覆盖 MCP tool。

## 1. 统一执行模型

builtin agent tool 的核心契约是：

```rust
execute(arguments: AgentArguments) -> AgentToolResult
```

其中：

- `arguments` 是实体化后的工具参数对象，不是 prompt 文本，也不是历史记录片段
- `AgentToolResult` 是唯一标准返回体
- 这是所有 Agent Tool 都应满足的基础 trait 语义

设计约定：

- 每个工具先定义自己的 typed `AgentArguments`，再实现“typed arguments -> AgentToolResult”的执行函数
- bash / action / llm_tool_call 都只是适配入口，负责把外部调用解析成同一个 arguments
- 同一个工具可以支持多种入口，但入口之间不应各自实现一套业务逻辑
- `ToolSpec.args_schema` 必须和 arguments 保持一致，能直接接到 function call
- 所有输入参数都属于 arguments，不属于 `AgentToolResult.detail`
- `cmd_args` 是这次 arguments 的 bash 风格文本表达，用于 Full 展示和调试；它不是结构化参数容器

### 1.1 入口适配

builtin agent tool 目前常见三种入口：

- `bash`：以命令别名形式调用，例如 `agent_tool read uri=demo.txt`
- `action`：以结构化 JSON 参数调用，主要用于写操作
- `llm_tool_call`：以 `ToolSpec.args_schema` 声明的 JSON 调用

每个工具都会通过 `AgentTool` trait 声明自己支持哪些入口：

- `support_bash()`
- `support_action()`
- `support_llm_tool_call()`

设计约定：

- bash 模式优先面向“像命令行工具一样使用”
- action 模式优先面向“结构化写操作”
- llm_tool_call 模式优先面向“function calling”
- 入口层只做解析、校验和调用转发，最终都应收敛到统一 arguments

### 1.2 实现边界

Agent Tool 的实现不能反向依赖 Agent 相关基础设施。

也就是说，工具主体应当看起来像一个普通功能函数，在一个普通文件系统和普通网络环境里工作：

- 不依赖 Agent Loop、prompt renderer、WorkLog、memory、todo 调度等上层编排语义
- 不从 Agent 上下文里偷取隐式输入；需要的输入必须出现在 arguments 或明确的执行环境里
- 需要操作状态时，直接操作文件、数据库文件或发起网络请求
- 需要路径、URL、token、session id、workspace id 等上下文时，应作为 arguments 或显式 runtime env 输入进入工具
- 工具返回 `AgentToolResult`，但不应该知道这个结果之后会被哪个 Agent 或哪种 prompt 压缩策略消费
- 即使工具处理的是 session、todo、worklog 这类 Agent 领域对象，实现上也应把它们当作普通文件、数据库记录或网络资源来操作

## 2. Function Call Schema

每个支持 `llm_tool_call` 的 builtin tool 都必须提供合适的 `ToolSpec.args_schema`。

设计约定：

- schema 描述的就是 typed arguments
- schema 必须覆盖执行函数需要的全部输入
- schema 中的字段名、类型、枚举值、必填项必须和执行函数实际接收的 arguments 一致
- schema 只描述输入，不描述 `detail` 返回结构
- 对可枚举操作使用 `enum`，例如 `mode: "replace|after|before"`
- 对复杂操作优先定义明确的 object，而不是把 JSON 字符串塞进 string 字段
- 不能为了方便 prompt 展示，把输入字段复制到 `detail`

## 3. 统一输出模型

builtin tool 的标准输出协议是 `AgentToolResult`，详细字段见：

- [agent_tool_result_protocol.md](agent_tool_result_protocol.md)

当前约定可以简化成：

```json
{
  "agent_tool_protocol": "1",
  "status": "success|error|pending",
  "cmd_name": "tool_name",
  "cmd_args": "bash style argument text",
  "title": "one line compressed view",
  "summary": "multi-line compressed view",
  "detail": {}
}
```

其中：

- `agent_tool_protocol` 标识这是 AgentToolResult 协议结果，当前版本是 `"1"`
- `cmd_name` 是工具名或命令名
- `cmd_args` 是 arguments 的 bash 风格参数文本
- `title` / `summary` 是压缩展示字段，不承载机器可读业务语义
- `detail` 或 `output` 承载主返回体；默认只填其中一个
- CLI 对接层应根据 `status` 自动映射退出码：
  - `success` -> `0`
  - `error` -> 非 `0`
  - `pending` -> 非 `0` 或约定中的“未完成”退出码

补充字段按需出现：

- `task_id` / `pending_reason` / `check_after`：长任务轮询时使用
- `return_code`：仅当需要保留 shell 退出码语义时使用
- `output`：仅当工具明确要暴露 bash 主文本输出时使用，builtin tool 默认不依赖它


设计原则：

- builtin tool 默认至少返回 `agent_tool_protocol / status / cmd_name / cmd_args / title / summary`
- 结构化业务结果放 object / array `detail`
- 文本型主结果可以放字符串 `detail`，也可以放 `output`
- `summary` 给人读，不要求机读，但要稳定
- `detail` 只放执行结果，不放 arguments 的重复拷贝；旧实现若仍在 `detail` 回显输入参数，应按本约定逐步修正
- 不要把 `detail` 当纯文本 stdout 容器，除非该工具的主结果本身就是文本
- `output` 不是 builtin tool 的默认字段，普通 bash 命令才以 `output` 为主

### 3.1 `detail` 和 arguments 的分工

`detail` 不是输入回显区。它只描述执行后的结果。

例如：

- `read` 的 `content` 是读取结果，可以放在 `detail`
- `write_file` 的 `content` 是输入参数，不应放在 `detail`
- `edit_file` 的 `old_string` / `new_string` 是输入参数，不应放在 `detail`
- `todo` / `worklog_manage` 的 `action` 是输入参数，不应只为了回显而放在 `detail`
- `path`、`mode`、`range` 等输入参数默认不应复制到 `detail`，需要追踪调用时读取 arguments 或 `cmd_args`

如果某个结果字段和输入参数同名，必须确认它表达的是执行后事实，而不是简单回显。

### 3.2 legacy `read_file` 的纯文本例外

legacy CLI `read_file` 在 CLI 下存在一个特例：

- 当“没有 agent 环境”且“stdout 不是 TTY”时
- 自动切换到纯文本模式
- 直接输出读取到的文件内容

这个模式用于管道、流式消费、脚本场景，行为接近 `cat`。

## 4. 当前 builtin tools 一览

| Tool | 入口 | 主要用途 | 代码位置 |
|---|---|---|---|
| `read` | action / llm_tool_call | 按 uri/path 读取内容；无 `://` 时默认文件路径 | `src/read_tool.rs` |
| `write_file` | action | 覆盖/追加写文件 | `src/file_tools.rs` |
| `edit_file` | action | 基于唯一 old_string 替换文件 | `src/file_tools.rs` |
| `get_session` | bash | 读取 session 状态 | `src/lib.rs` |
| `load_memory` | bash / llm_tool_call | 加载记忆摘要 | `src/lib.rs` |
| `todo` | bash | 工作项 PDCA 管理 | `src/agent_todo_tool.rs` |
| `create_workspace` | bash | 创建并绑定 workspace | `src/lib.rs` |
| `bind_workspace` | bash | 切换当前 workspace | `src/lib.rs` |
| `bind_external_workspace` | call | 注册外部 workspace | `src/lib.rs` |
| `list_external_workspaces` | call | 列出外部 workspace | `src/lib.rs` |
| `worklog_manage` | bash / call | worklog 结构化管理 | `src/lib.rs` |
| `check_task` | CLI | 轮询 pending task | `src/cli.rs` |
| `cancel_task` | CLI | 取消 pending task | `src/cli.rs` |
| `finish_task` | CLI | 结束 task（完成/失败） | `src/cli.rs` |

说明：

- `bind_external_workspace` / `list_external_workspaces` 当前主要走结构化调用
- `check_task` / `cancel_task` / `finish_task` 是 CLI 暴露能力，不走 `AgentTool` trait 的常规注册路径
- `read_file` 是 legacy CLI 兼容工具，不再是 v2 Agent Action；当前 Action 应使用 `read`
- `list_session` 常量已预留，但当前文档不把它当作已完成 builtin tool

## 5. 各工具输入 / 输出约定

### 5.1 `read`

用途：

- 读取文件
- 支持 `file://` 显式文件 URI
- 支持无协议头路径，默认按文件路径处理
- 读取结构化数据并返回 LLM 可处理的文本
- 支持 line offset / limit 分页
- token limit 是 session 属性，默认 20K，不是本函数参数

输入：

```json
{
  "uri": "string",
  "offset": "number|string",
  "limit": "number|string"
}
```

action 形式：

```xml
<read uri="src/foo.rs" offset="0" limit="200"/>
```

detail 关键字段：


- `content`
- `offset`
- `limit`
- `lines_read`
- `total_lines`
- `eof`
- `unchanged`
- `token_limit`
- `token_truncated`

同一个 `session_id` 中，对同一个文件、同一 `offset/limit` 窗口重复 `read` 时，如果文件内容和上一次相比没有变化，`detail.content` 和顶层 `output` 返回 `和上一次read相比没有变化`，`detail.unchanged` 为 `true`。状态文件保存在系统临时目录下。


输出约定：

- 顶层固定字段至少包含 `agent_tool_protocol / status / cmd_name / cmd_args / title / summary`
- `cmd_name` 应为 `read`
- `cmd_args` 应渲染成 bash 风格参数文本，例如 `src/foo.rs offset=0 limit=200`
- 读取结果放 `detail.content`
- `summary` 提供读取字节数、offset、total 和 EOF 状态

### 5.2 `write_file`

用途：

- 创建、覆盖或追加文件

输入：

```json
{
  "path": "string",
  "content": "string",
  "mode": "new|append|write"
}
```

detail 关键字段：

- `created`
- `bytes_written`
- `line_count`

输出约定：

- 顶层固定字段至少包含 `agent_tool_protocol / status / cmd_name / cmd_args / title / summary`
- `cmd_name` 应为 `write_file`
- `cmd_args` 表达 path / mode / content 等输入参数
- `summary` 描述本次写入结果，不要把完整写入内容塞进 `summary`
- 主结果放 `detail`
- `detail` 不应包含输入参数 `content`
- 不设置 `output`

### 5.3 `edit_file`

用途：

- 基于唯一 `old_string` 对文件做替换

输入：

```json
{
  "path": "string",
  "old_string": "string",
  "new_string": "string"
}
```

`new_string` 必须不同于 `old_string`。`old_string` 必须在文件中只出现一次；没有命中或命中多处都会失败。


detail 关键字段：


- `matched`
- `changed`
- `line`
- `diff`

输出约定：

- 顶层固定字段至少包含 `agent_tool_protocol / status / cmd_name / cmd_args / title / summary`
- `cmd_name` 应为 `edit_file`
- `cmd_args` 表达 path / old_string / new_string 等输入参数
- `summary` 表达是否产生修改
- 主结果放 `detail`
- `detail` 不应包含输入参数 `old_string` 或 `new_string`

### 5.4 `get_session`

用途：

- 读取当前 session 的快照

输入：

```json
{
  "session_id": "string"
}
```

bash 形式：

```bash
get_session [session_id]
```

detail 关键字段：

- `ok`
- `session`

输出约定：

- 顶层固定字段至少包含 `agent_tool_protocol / status / cmd_name / cmd_args / title / summary`
- `cmd_name` 应为 `get_session`
- `cmd_args` 表达可选的 `session_id`
- `summary` 固定为 `ok`
- 完整 session 数据放 `detail.session`

### 5.5 `load_memory`

用途：

- 按默认检索策略返回 memory 摘要

输入：

```json
{
  "token_limit": "number",
  "tags": ["string"],
  "current_time": "string"
}
```

输出约定：

- 顶层固定字段至少包含 `agent_tool_protocol / status / cmd_name / cmd_args / title / summary`
- `cmd_name` 应为 `load_memory`
- `cmd_args` 表达 token_limit / tags / current_time 等输入参数
- 当前实现把 memory 预览文本直接放在 `detail`
- `summary` 用于表达装载了多少 memory items

备注：

- 这是少数 `detail` 不是 object、而是 string 的工具
- 后续如果要统一，也可以升级成 `{ "text": "...", "item_count": N }`

### 5.6 `todo`

用途：

- workspace 级 todo / note / result / error / PDCA 流转管理

顶层输入：

```json
{
  "action": "list|get|apply_delta|query_pending|render_for_prompt|render_current_details|get_next_ready_todo",
  "workspace_id": "string",
  "todo_ref": "string",
  "agent_id": "string",
  "filters": {},
  "limit": 10,
  "offset": 0,
  "states": ["WAIT"],
  "token_budget": 800,
  "session_id": "string",
  "delta": {
    "op_id": "string",
    "ops": [{}]
  },
  "actor_ctx": {
    "kind": "root_agent|sub_agent|user|system",
    "did": "string",
    "session_id": "string",
    "trace_id": "string"
  }
}
```

bash 形式是一套独立 CLI 子命令体系，例如：

```bash
todo add "title"
todo start T001
todo done T001 "reason"
todo ls --all
todo next
```

detail 常见字段：

- `ok`
- `items`
- `item`
- `notes`
- `deps`
- `version`
- `new_version`
- `before_version`
- `op_id`
- `errors`
- `counts_by_status`
- `has_pending`
- `text`

输出约定：

- 顶层固定字段至少包含 `agent_tool_protocol / status / cmd_name / cmd_args / title / summary`
- `cmd_name` 应为 `todo`
- `cmd_args` 表达 action 和对应输入参数
- `summary` 通常使用 action 名或紧凑操作结果
- 具体业务结果放 `detail`
- `todo` 的详细命令子协议建议未来单独出文档

### 5.7 `create_workspace`

用途：

- 创建本地 workspace
- 写入 `SUMMARY.md`
- 绑定到当前 session

输入：

```json
{
  "name": "string",
  "summary": "string"
}
```

bash 形式：

```bash
create_workspace <name> <summary>
```

detail 关键字段：

- `ok`
- `workspace`
- `binding`
- `summary_path`
- `session_updated`

输出约定：

- 顶层固定字段至少包含 `agent_tool_protocol / status / cmd_name / cmd_args / title / summary`
- `cmd_name` 应为 `create_workspace`
- `cmd_args` 表达 name / summary 等输入参数
- `summary` 当前通常为 `ok`

### 5.8 `bind_workspace`

用途：

- 将当前 session 绑定到指定 workspace

输入：

```json
{
  "workspace": "string"
}
```

bash 形式：

```bash
bind_workspace <workspace_id|workspace_path>
```

detail 关键字段：

- `ok`
- `binding`
- `session_updated`

输出约定：

- 顶层固定字段至少包含 `agent_tool_protocol / status / cmd_name / cmd_args / title / summary`
- `cmd_name` 应为 `bind_workspace`
- `cmd_args` 表达 workspace 输入参数
- `summary` 当前通常为 `ok`

### 5.9 `bind_external_workspace`

用途：

- 将用户提供目录注册为当前 agent 可见的 external workspace

输入：

```json
{
  "name": "string",
  "workspace_path": "string",
  "agent_did": "string"
}
```

detail 关键字段：

- `ok`
- `binding`

输出约定：

- 顶层固定字段至少包含 `agent_tool_protocol / status / cmd_name / cmd_args / title / summary`
- `cmd_name` 应为 `bind_external_workspace`
- `cmd_args` 表达 name / workspace_path / agent_did 等输入参数
- `summary` 当前通常为 `ok`

### 5.10 `list_external_workspaces`

用途：

- 列出 agent 当前可见的 external workspace

输入：

```json
{
  "agent_did": "string"
}
```

detail 关键字段：

- `ok`
- `workspaces`

输出约定：

- 顶层固定字段至少包含 `agent_tool_protocol / status / cmd_name / cmd_args / title / summary`
- `cmd_name` 应为 `list_external_workspaces`
- `cmd_args` 表达 agent_did 输入参数
- `summary` 当前通常为 `ok`

### 5.11 `worklog_manage`

用途：

- Append-only 审计日志的写入与查询。Worklog 不再进入 Prompt，仅供调试 / 事后分析。

输入顶层字段：

```json
{
  "action": "append_worklog|list_worklog|get_worklog",
  "record": {},
  "id": "string",
  "step_id": "string",
  "owner_session_id": "string",
  "workspace_id": "string",
  "type": "string (open enum, e.g. GetMessage / ReplyMessage / agent.file.write)",
  "status": "string",
  "keyword": "string",
  "limit": 20,
  "offset": 0
}
```

detail 常见字段：

- `ok`
- `record`
- `records`
- `total`

输出约定：

- 顶层固定字段至少包含 `agent_tool_protocol / status / cmd_name / cmd_args / title / summary`
- `cmd_name` 应为 `worklog_manage`
- `cmd_args` 表达 action 和对应输入参数
- `summary` 默认使用 action 名或紧凑操作结果

> 历史接口 `append_step_summary / mark_step_committed / list_step / build_prompt_worklog / render_for_prompt`
> 已在 beta2.2 简化中移除，调用会返回 `unsupported action` 错误。详见 `notepads/worklog简化.md`。

### 5.12 `check_task`

用途：

- 对 `pending` 任务做轮询

CLI 输入：

```bash
check_task <task_id>
```

输出约定：

- 如果目标任务本身是 agent tool 任务，则继续返回 builtin 风格结果
- builtin 风格结果仍应优先满足 `agent_tool_protocol / status / cmd_name / cmd_args / title / summary`
- `status` 会映射成 `success|error|pending`
- 可能带：
  - `task_id`
  - `pending_reason`
  - `check_after`
  - `return_code`
- 只有在任务本身不是 builtin agent tool、而是 bash 任务代理时，才可能继续带 `output`

detail 常见字段：

- 规范化后的 task detail
- `task`

### 5.13 `cancel_task`

用途：

- 取消 pending task，可选递归取消

CLI 输入：

```bash
cancel_task <task_id> [--recursive]
```

输出约定：

- 返回取消后的 task 结果封装
- builtin 风格结果仍应优先满足 `agent_tool_protocol / status / cmd_name / cmd_args / title / summary`
- detail 常见字段：
  - `task`
  - `interrupt_error`

### 5.14 `finish_task`

用途：

- 把指定 task 结束为完成或失败

CLI 输入：

```bash
finish_task <task_id> [failed] [--message <text>]
```

输出约定：

- 默认调用 TaskManager `update_task` 写入 `Completed` 和 `progress=100.0`
- `failed` / `--failed` 调用 TaskManager `update_task_error` 写入 `Failed` 和错误消息
- 返回更新后的 task 结果封装
- builtin 风格结果仍应优先满足 `agent_tool_protocol / status / cmd_name / cmd_args / title / summary`
- detail 常见字段：
  - `task`
  - `finish_outcome`

## 6. 后续文档拆分建议

为了避免这份文档继续膨胀，建议后续按主题拆成几个子文档：

1. `file_tools_protocol.md`
   统一整理 `read_file / write_file / edit_file`
2. `todo_tool_protocol.md`
   详细整理 `todo` 子命令和 `apply_delta` 操作集
3. `workspace_tools_protocol.md`
   统一整理 workspace 相关工具
4. `task_tools_protocol.md`
   统一整理 `check_task / cancel_task / finish_task` 和 pending 轮询模型

## 7. 文档维护原则

- 以当前代码为准，不追求历史兼容描述
- 参数名必须与 `ToolSpec.args_schema` 或 CLI 实现保持一致
- builtin tool 的顶层固定字段说明必须与 `agent_tool_protocol / status / cmd_name / cmd_args / title / summary / detail|output` 约定一致
- 输出字段必须与 `detail` 实际返回结构一致
- `detail` 只记录执行结果，不重复 arguments
- 输入参数必须能从 arguments schema 和 `cmd_args` 找到
- 如果工具存在“JSON 模式”和“纯文本模式”双轨行为，必须明确写清楚切换条件
