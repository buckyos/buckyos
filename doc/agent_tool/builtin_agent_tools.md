# Builtin Agent Tools 设计整理

本文整理 `src/frame/agent_tool` 中 builtin agent tool 的设计约定，重点关注两件事：

1. 输入参数怎么组织
2. 输出格式怎么稳定落到 `AgentToolResult`

本文只覆盖当前工程内已经实现并对外暴露的 builtin tools，不覆盖外部 bash 命令，也不覆盖 MCP tool。

## 1. 统一输入模型

builtin agent tool 目前有三种入口：

- `bash`：以命令别名形式调用，例如 `read_file demo.txt 1-20`
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
- 同一个工具可以同时支持多种入口，但最终都应收敛到统一语义

## 2. 统一输出模型

builtin tool 的标准输出协议是 `AgentToolResult`，详细字段见：

- [agent_tool_result_protocol.md](/Users/liuzhicong/project/buckyos/src/frame/agent_tool/agent_tool_result_protocol.md)

当前约定可以简化成：

```json
{
  "is_agent_tool": true,
  "cmd_name": "bash style cmd line",
  "status": "success|error|pending",
  "summary": "human readable summary",
  "detail": {}
}
```

其中：

- `cmd_name` 对 builtin tool 采用 bash 风格的完整命令文本，用于 prompt / worklog / 调试
- CLI 对接层应根据 `status` 自动映射退出码：
  - `success` -> `0`
  - `error` -> 非 `0`
  - `pending` -> 非 `0` 或约定中的“未完成”退出码

补充字段按需出现：

- `task_id` / `pending_reason` / `check_after`：长任务轮询时使用
- `return_code`：仅当需要保留 shell 退出码语义时使用
- `output`：仅当工具明确要暴露 bash 主文本输出时使用，builtin tool 默认不依赖它


设计原则：

- builtin tool 的业务结果放 `detail`
- builtin tool 默认至少返回 `is_agent_tool / cmd_name / status / summary / detail`
- `summary` 给人读，不要求机读，但要稳定
- 不要把 `detail` 当纯文本 stdout 容器
- `output` 不是 builtin tool 的默认字段，普通 bash 命令才以 `output` 为主

### 2.1 `read_file` 的纯文本例外

`read_file` 在 CLI 下存在一个特例：

- 当“没有 agent 环境”且“stdout 不是 TTY”时
- 自动切换到纯文本模式
- 直接输出 `detail.content`

这个模式用于管道、流式消费、脚本场景，行为接近 `cat`。

## 3. 当前 builtin tools 一览

| Tool | 入口 | 主要用途 | 代码位置 |
|---|---|---|---|
| `read_file` | bash / llm_tool_call | 读取文件内容 | `src/file_tools.rs` |
| `write_file` | action | 覆盖/追加写文件 | `src/file_tools.rs` |
| `edit_file` | action | 基于锚点编辑文件 | `src/file_tools.rs` |
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

说明：

- `bind_external_workspace` / `list_external_workspaces` 当前主要走结构化调用
- `check_task` / `cancel_task` 是 CLI 暴露能力，不走 `AgentTool` trait 的常规注册路径
- `list_session` 常量已预留，但当前文档不把它当作已完成 builtin tool

## 4. 各工具输入 / 输出约定

### 4.1 `read_file`

用途：

- 读取文件
- 支持从 `first_chunk` 命中点开始读取
- 支持 1-based 行范围切片

输入：

```json
{
  "path": "string",
  "range": "string|number|array|object",
  "first_chunk": "string"
}
```

bash 形式：

```bash
read_file <path> [range] [first_chunk]
```

detail 关键字段：


- `content`


输出约定：

- 顶层固定字段至少包含 `is_agent_tool / cmd_name / status / summary / detail`
- `cmd_name` 应渲染成 bash 风格命令文本，例如 `read demo.txt range=1-20`
- 标准 builtin 模式下，内容放 `detail.content`
- `summary` 提供英文摘要和预览代码块
- 非交互纯文本模式下，CLI 直接输出 `detail.content`

### 4.2 `write_file`

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

- content 

输出约定：

- 顶层固定字段至少包含 `is_agent_tool / cmd_name / status / summary / detail`
- `cmd_name` create xxxx and write xxx line | append xxxx and write xxxx line | write xxx with xxx line
- `summary` 描述本次写入的方式和内容的缩写
- 主结果放 `detail`
- 不设置 `output`

### 4.3 `edit_file`

用途：

- 基于锚点字符串对文件做替换、前插、后插

输入：

```json
{
  "path": "string",
  "pos_chunk": "string",
  "new_content": "string",
  "mode": "replace|after|before"
}
```


detail 关键字段：


- update 说明文件修改的模式
- content 新内容

输出约定：

- 顶层固定字段至少包含 `is_agent_tool / cmd_name / status / summary / detail`
- `cmd_name` edit xxxx ,insert content at line:xxx | replace [xxxx:xxx] to new content
- `summary` 表达是否命中锚点、是否产生修改,如果产生修改，可以放git diff风格的修改记录
- 主结果放 `detail`

### 4.4 `get_session`

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

- 顶层固定字段至少包含 `is_agent_tool / cmd_name / status / summary / detail`
- `summary` 固定为 `ok`
- 完整 session 数据放 `detail.session`

### 4.5 `load_memory`

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

- 顶层固定字段至少包含 `is_agent_tool / cmd_name / status / summary / detail`
- 当前实现把 memory 预览文本直接放在 `detail`
- `summary` 用于表达装载了多少 memory items

备注：

- 这是少数 `detail` 不是 object、而是 string 的工具
- 后续如果要统一，也可以升级成 `{ "text": "...", "item_count": N }`

### 4.6 `todo`

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
- `action`
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

- 顶层固定字段至少包含 `is_agent_tool / cmd_name / status / summary / detail`
- `summary` 通常使用 action 名或紧凑操作结果
- 具体业务结果放 `detail`
- `todo` 的详细命令子协议建议未来单独出文档

### 4.7 `create_workspace`

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
- `session_id`
- `session_updated`

输出约定：

- 顶层固定字段至少包含 `is_agent_tool / cmd_name / status / summary / detail`
- `summary` 当前通常为 `ok`

### 4.8 `bind_workspace`

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
- `session_id`
- `session_updated`

输出约定：

- 顶层固定字段至少包含 `is_agent_tool / cmd_name / status / summary / detail`
- `summary` 当前通常为 `ok`

### 4.9 `bind_external_workspace`

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

- 顶层固定字段至少包含 `is_agent_tool / cmd_name / status / summary / detail`
- `summary` 当前通常为 `ok`

### 4.10 `list_external_workspaces`

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

- 顶层固定字段至少包含 `is_agent_tool / cmd_name / status / summary / detail`
- `summary` 当前通常为 `ok`

### 4.11 `worklog_manage`

用途：

- 结构化 worklog 增删查渲染

输入顶层字段：

```json
{
  "action": "append_worklog|append_step_summary|mark_step_committed|list_worklog|get_worklog|list_step|build_prompt_worklog|append|list|get|render_for_prompt",
  "record": {},
  "log_id": "string",
  "id": "string",
  "step_id": "string",
  "owner_session_id": "string",
  "workspace_id": "string",
  "todo_id": "string",
  "type": "GetMessage|ReplyMessage|FunctionRecord|ActionRecord|CreateSubAgent|StepSummary",
  "status": "string",
  "tag": "string",
  "limit": 20,
  "offset": 0,
  "token_budget": 1000
}
```

detail 常见字段：

- `ok`
- `action`
- `record`
- `records`
- `total`
- `text`
- `prompt_text`
- `updated`

输出约定：

- 顶层固定字段至少包含 `is_agent_tool / cmd_name / status / summary / detail`
- `summary` 默认使用 `detail.action`

### 4.12 `check_task`

用途：

- 对 `pending` 任务做轮询

CLI 输入：

```bash
check_task <task_id>
```

输出约定：

- 如果目标任务本身是 agent tool 任务，则继续返回 builtin 风格结果
- builtin 风格结果仍应优先满足 `is_agent_tool / cmd_name / status / summary / detail`
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

### 4.13 `cancel_task`

用途：

- 取消 pending task，可选递归取消

CLI 输入：

```bash
cancel_task <task_id> [--recursive]
```

输出约定：

- 返回取消后的 task 结果封装
- builtin 风格结果仍应优先满足 `is_agent_tool / cmd_name / status / summary / detail`
- detail 常见字段：
  - `task`
  - `recursive`
  - `interrupt_error`

## 5. 后续文档拆分建议

为了避免这份文档继续膨胀，建议后续按主题拆成几个子文档：

1. `file_tools_protocol.md`
   统一整理 `read_file / write_file / edit_file`
2. `todo_tool_protocol.md`
   详细整理 `todo` 子命令和 `apply_delta` 操作集
3. `workspace_tools_protocol.md`
   统一整理 workspace 相关工具
4. `task_tools_protocol.md`
   统一整理 `check_task / cancel_task` 和 pending 轮询模型

## 6. 文档维护原则

- 以当前代码为准，不追求历史兼容描述
- 参数名必须与 `ToolSpec.args_schema` 或 CLI 实现保持一致
- builtin tool 的顶层固定字段说明必须与 `cmd_name / status / summary / detail` 约定一致
- 输出字段必须与 `detail` 实际落盘/返回结构一致
- 如果工具存在“JSON 模式”和“纯文本模式”双轨行为，必须明确写清楚切换条件
