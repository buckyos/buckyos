# Worklog 简化计划

## 1. 新定位

Worklog 的核心用途是从审计、开发调试角度观察 Agent 行为。它应当更接近 syslog / audit log：

- 保存 Agent 行为的全量事件数据。
- 不进入 prompt。
- 删除、清理或过期不会影响 Agent 行为。
- Self-Improve 可以显式读取其中的错误记录，但这是一种事后分析行为，不是 Agent 正常运行上下文的一部分。
- AgentTool 实现不依赖 Worklog，也不主动操作 Worklog；只有 Agent Runtime 在调度、观察工具调用和工具结果时，才把相关行为写入 Worklog。

因此 Worklog 不再承担“记忆”“上下文恢复”“prompt 压缩摘要”“安全观察区”等职责。

## 2. 当前复杂度来源

现有实现里复杂度主要来自旧定位：Worklog 曾经被设计成可能进入 prompt 的 Workspace Worklog。

相关复杂度包括：

- `prompt_view`、`build_prompt_worklog`、token budget、prompt-safe render。
- `StepSummary` 的特殊逻辑：自动收集 refs、omitted_event_types、step 折叠渲染。
- `commit_state / mark_step_committed` 这类为 prompt 选择和 step 生命周期服务的状态。
- 封闭的 `WorklogRecordType` 枚举和每种类型的 prompt renderer。
- 写入前把消息、工具结果、action result 压缩成 digest，导致不能满足全量审计目标。
- kRPC AgentWorkspace 观察接口暴露 worklog 查询和 `worklog_db_path`。

## 3. 已完成的第一步

已删除原 AgentWorkspace 开发观察工具遗留的 worklog kRPC 接口：

- 删除 `list_workshop_worklogs` client 方法、server dispatcher 分支和 handler trait 方法。
- 删除 `OpenDanWorklogItem`、`OpenDanWorkspaceWorklogsResult`、`OpenDanListWorkshopWorklogsReq`。
- 删除 `ai_runtime` 里的 worklog 查询函数、worklog type filter 解析和 workspace summary 的 `worklog_total`。
- `OpenDanWorkspaceInfo` 不再暴露 `worklog_db_path`。

这一步只移除了远程观察面，未改变 runtime 本地写入 worklog 的能力。

## 4. 目标形态

Worklog 收敛成一个 append-only event log：

```text
agent runtime / tool audit / self-improve
        -> append full event JSON
        -> indexed by session / workspace / step / event_type / status / time
        -> UI / debug / self-improve query
```

保留能力：

- append event
- list events
- get event
- query errors
- 按 agent、session、workspace、step、event_type、status、time、keyword 过滤

删除能力：

- prompt render
- prompt_view
- token budget / prompt truncation
- StepSummary refs / omitted_event_types
- commit_state / mark_step_committed
- 事件类型注册和 prompt renderer
- session 内存 worklog 截断缓存

## 5. 简化步骤

### Step 1: 停止 Worklog 进入 Prompt

修改 `behavior/prompt.rs`：

- 移除从 workspace worklog DB 加载 prompt timeline 的逻辑。
- 不再调用 `WorklogService::list_worklog_records` 构造 memory timeline。
- Prompt 只保留三类上下文：Agent Memory、History Message、Session Step Records。

修改 `agent_session.rs`：

- 移除 `render_worklog_prompt_line_from_session_item` 和 `render_worklog_prompt_line` 相关依赖。
- append worklog 时只做落盘和普通日志输出，不生成 prompt line。

预期结果：

- Worklog 不再影响 Agent 行为。
- Worklog 删除或丢失不影响 prompt 构造。

### Step 2: 删除 Prompt 专用字段和接口

修改 `worklog.rs`：

- 删除 `WorklogPromptView`。
- 删除 `prompt_view` 字段。
- 删除 `build_prompt_worklog` action。
- 删除 `PromptBuildInput`、`query_prompt_candidates`、`build_prompt_text` 及相关 renderer。
- 删除 `build_prompt_view_by_type`、`parse_prompt_view`、`sanitize_digest` 中仅用于 prompt 安全的逻辑。

同时更新 `agent_tool` crate 中 `WorklogTool` 的 schema：

- action 列表删除 `build_prompt_worklog / render_for_prompt`。
- description 从 “prompt-safe rendering” 改成审计日志描述。

### Step 3: 降级 StepSummary

当前 `StepSummary` 可以有两种处理方式：

1. 直接删除 `append_step_summary`，使用 `LLMStepRecord` 表达 step 级摘要。
2. 保留 `StepSummary` 作为普通 event_type，但不再自动收集 refs / omitted_event_types。

建议选择 1，原因：

- step 级行为记录已经有 `LLMStepRecord`。
- Worklog 的职责是全量事件流，不需要再维护一套 step 聚合模型。
- 可以减少 `insert_step_summary`、`collect_step_event_refs`、`list_step_records` 等逻辑。

如果短期需要 UI 折叠，可先选择 2，后续再删除。

### Step 4: 简化存储 schema

当前表同时保存大量索引列和 `record_json`，字段偏重旧 prompt / UI 模型。

目标表建议保留：

```text
log_id TEXT PRIMARY KEY
timestamp INTEGER NOT NULL
seq INTEGER NOT NULL
agent_id TEXT
owner_session_id TEXT
workspace_id TEXT
behavior TEXT
step_id TEXT
step_index INTEGER
event_type TEXT NOT NULL
status TEXT NOT NULL
trace_id TEXT
task_id TEXT
record_json TEXT NOT NULL
created_at INTEGER NOT NULL
```

建议索引：

```text
idx_worklogs_timestamp(timestamp DESC, created_at DESC)
idx_worklogs_session(owner_session_id, timestamp DESC)
idx_worklogs_workspace(workspace_id, timestamp DESC)
idx_worklogs_step(step_id, timestamp DESC)
idx_worklogs_type(event_type, timestamp DESC)
idx_worklogs_status(status, timestamp DESC)
```

可以删除的一等列：

- `scope`
- `subagent_did`
- `related_agent_id`
- `todo_id`
- `impact_level`
- `impact_domain_json`
- `impact_importance`
- `summary`
- `tags_json`
- `payload_json`
- `artifacts_json`
- `error_json`
- `prompt_view_json`
- `commit_state`

这些信息如仍有价值，应作为 `record_json` 内部字段保存。

### Step 5: 开放 event_type

把 `WorklogRecordType` 从封闭 enum 改成 string。

推荐事件命名：

```text
agent.message.in
agent.message.out
agent.tool.call
agent.action.result
agent.subagent.create
agent.error
agent.file.write
```

好处：

- 允许新事件类型自然扩展。
- 不需要注册 prompt renderer。
- Self-Improve 查错误只需要 `status=FAILED` 或 `error != null`。

### Step 6: 写入全量 payload

修改 `agent.rs` 里 worklog 生产端：

- incoming message 保存完整 message 关键信息，不只保存 `content_digest`。
- reply message 保存完整发送内容或可追溯的 artifact。
- action/tool result 保存完整 `AgentToolResult`。
- stdout/stderr 或大对象过大时保存 artifact ref，但 worklog 必须能追溯完整内容。

原则：

- Worklog 自身可以只索引少数字段。
- `record_json` 应保留审计需要的全量结构。
- 不再为了 prompt token 预算提前截断。

### Step 7: 统一落盘路径，删除 session 内存缓存

当前未绑定 workspace 时写入 `AgentSession.worklog` 内存数组，最多 256 条；绑定 workspace 后再 drain 到 workspace DB。

建议改成：

- 所有 work session 都直接写固定 worklog DB（每个agent有且仅有1个worklog.db)。
- workspace_id 只是过滤维度，不决定是否落盘。
- 删除 `AgentSession.worklog` 的内存截断缓存逻辑。

这样符合审计日志模型，避免“未绑定 workspace 时只保存最近 256 条”的数据丢失。

## 6. 验证计划

每一步至少做：

- `cargo check -p buckyos-api -p opendan`
- 相关单测：`cargo test -p opendan worklog`
- 检查 prompt 构造路径，确认不再读取 Worklog。
- 启动 Jarvis 后跑一次典型消息 -> tool/action -> reply 流程，确认 worklog 仍落盘。

最终完成后应额外验证：

- 删除 worklog DB 后 Agent 仍可正常工作。
- Self-Improve 显式查询错误记录仍可工作。
- UI 或调试工具如需要查看 worklog，走本地文件/DB 或新的审计查询接口，而不是旧 AgentWorkspace kRPC。

## 7. 风险与联动

- 如果前端或调试工具依赖 `summary / prompt_view / StepSummary` 展示，需要改成直接从 `record_json.payload` 渲染。
- 如果某些测试依赖 `build_prompt_worklog`，应改为验证 `Session Step Records` 或直接删除该测试。
- 如果旧数据库已有数据，需要确认是否需要迁移；如果当前仍是开发期，可以明确采用 no-compat，直接重建 worklog DB。
- 文档需要同步更新：
  - `doc/opendan/Agent Worklog.md`
  - `doc/opendan/Render_Prompt_Template_Variables.md`
  - `doc/agent_tool/builtin_agent_tools.md`
  - `doc/agent_tool/OpenDAN AgentTool 开发指南.md`

## 8. 建议执行顺序

1. 已完成：删除 AgentWorkspace worklog kRPC 观察接口。
2. 删除 prompt 构造中读取 worklog 的路径。
3. 删除 `prompt_view / build_prompt_worklog`。
4. 降级或删除 `StepSummary` 特殊逻辑。
5. 改 `WorklogRecordType` 为开放 string。
6. 调整写入端保存全量 payload。
7. 简化 DB schema 和 session 内存缓存。
8. 更新文档和测试。
