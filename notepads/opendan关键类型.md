# OpenDAN的类型

> 说明：本清单按你给的结构整理，区分了“系统层定义”和“OpenDAN内定义/扩展”。

## Task相关
由系统的 TaskMgr 定义，OpenDAN 在使用时扩展了部分 `Task.data` 约定。

- 系统原始类型（`src/kernel/buckyos-api/src/task_mgr.rs`）
  - `TaskStatus`
  - `TaskPermissions`
  - `Task`
  - `TaskFilter`
  - `CreateTaskOptions`
  - `TaskUpdatePayload`

- OpenDAN中的任务类型与数据约定
  - 行为任务类型常量：`LLM_BEHAVIOR_TASK_TYPE = "llm_behavior"`（`src/frame/opendan/src/behavior/behavior.rs`）
  - 创建行为任务时写入的 `task.data` 字段：
    - `trace_id`
    - `agent_did`
    - `behavior`
    - `step_idx`
    - `wakeup_id`
    - `kind`（固定为 `"behavior"`）
  - 读取 AICC 异步结果时支持的 `task.data` 形态（兼容多种历史/上游格式）：
    - 完整 `CompleteResponse`
    - `{ "result": AiResponseSummary }`
    - `{ "aicc": { "output": AiResponseSummary } }`
    - 直接 `AiResponseSummary`
  - 通过 `task.data` 中 `/aicc/external_task_id` 做 AICC 外部任务 ID 到 TaskMgr 任务 ID 的映射。

- 与 Task 的桥接字段（非 TaskMgr 原生字段，但在 OpenDAN 里关联任务执行）
  - Todo 中扩展字段：`task_id`、`task_status`（`src/frame/opendan/src/workspace/todo.rs`）

## Msg相关

由系统的 MsgCenter 定义，用来表示互联网传播的通讯信息。注意和 `kmsg` 的 `msg_queue::Message` 不是一个体系。

- 系统原始类型（`src/kernel/buckyos-api/src/msg_center_client.rs`）
  - `BoxKind`
  - `MsgState`
  - `MsgObject`
  - `MsgRecord`
  - `MsgRecordWithObject`
  - `MsgRecordPage`

- OpenDAN里的消息包装与消费类型
  - `PulledInboxMessage { input: Json, record_id: String }`（`src/frame/opendan/src/agent.rs`）
  - MsgCenter 拉取后被封装为：
    - `{ "source": "msg_center.krpc", "record": MsgRecord, "msg": MsgObject }`
  - `AgentSession::load_chat_history` 输出的消息项字段：
    - `record_id`
    - `session_id`（从 payload/meta 推断）
    - `box_kind`
    - `state`
    - `from`
    - `to`
    - `created_at_ms`
    - `payload`
    - `meta`

- 对比：kmsg/MsgQueue 类型（`src/kernel/buckyos-api/src/msg_queue.rs`）
  - `Message { index, created_at, payload: Vec<u8>, headers }`

## InputEvent相关

来自系统 event-bus 的可扩展信息，底层通常可基于 kmsg 实现。

- 现状
  - OpenDAN 当前没有独立 `InputEvent` struct/enum 类型。
  - 事件在 Agent 内以 `Json` 形式流转：`queued_events: VecDeque<Json>`（`src/frame/opendan/src/agent.rs`）。

- 关键输入类型/结构
  - 入口：`push_event(event: Json)`
  - 唤醒输入 payload 结构：
    - `{ "trigger": "on_wakeup", "inbox": [...], "events": [...] }`
  - loop 上下文：`WakeupLoopContext { session_id, event_id, recent_turns }`
  - 测试中示例事件：`{ "kind": "task_due", "task": "status_report" }`

## Workshop

- 环境与总配置
  - `AgentEnvironment`（`src/frame/opendan/src/agent_enviroment.rs`）
  - `AgentWorkshopConfig`
  - `AgentWorkshopToolsConfig`
  - `WorkshopToolConfig`
  - `AgentWorkshop`
  - Workshop 相关类型路径：`src/frame/opendan/src/workspace/workshop.rs`

- Todo
  - `TodoToolConfig`
  - `TodoTool`
  - `TodoCreateInput`
  - `TodoPatch`
  - `TodoItem`
  - 路径：`src/frame/opendan/src/workspace/todo.rs`

- worklog
  - `WorklogToolConfig`
  - `WorklogTool`
  - `WorklogAppendInput`
  - `WorklogListFilters`
  - `WorklogItem`
  - 路径：`src/frame/opendan/src/workspace/worklog.rs`

- Tool相关
  - `ToolSpec`
  - `ToolCall`
  - `ToolError`
  - `AgentTool`（trait）
  - `MCPToolConfig`
  - `MCPTool`
  - `ToolManager`
  - 路径：`src/frame/opendan/src/agent_tool.rs`

  - `ToolContext`
    - 语义上对应两层：
      - 调用追踪上下文：`TraceCtx`（`src/frame/opendan/src/behavior/types.rs`）
      - 工具循环上下文：`ToolContext { tool_calls, observations }`（`src/frame/opendan/src/behavior/tool_loop.rs`）

  - `ToolCallResult`
    - 当前没有叫这个名字的独立结构体。
    - 当前结果表示方式：
      - 工具直接返回：`Result<Json, ToolError>`
      - loop 观测层：`Observation`
      - 统计追踪层：`ToolExecRecord`

- Skills相关
  - 当前 `src/frame/opendan/src` 内未发现独立 `Skill` 类型定义。

## Agent

- Agent主类型（`src/frame/opendan/src/agent.rs`）
  - `AIAgentError`
  - `AIAgentConfig`
  - `AIAgentDeps`
  - `WakeupStatus`
  - `WakeupReport`
  - `AIAgent`
  - 内部状态/流程类型（私有）
    - `AIAgentState`
    - `PreparedWakeup`
    - `AgentLoopState`
    - `ModeSelectionResult`
    - `WakeupLoopContext`
    - `ResolveRouterResult`
    - `PulledInboxMessage`
    - `AgentPolicy`

- AgentSession（`src/frame/opendan/src/agent_session.rs`）
  - `AgentSessionConfig`
  - `AgentSession`
  - `AgentSessionRecord`
  - `SessionLink`
  - `CreateSessionRequest`
  - `UpdateSessionPatch`

- memory相关（`src/frame/opendan/src/agent_memory.rs`）
  - `AgentMemoryConfig`
  - `AgentMemory`
  - 内部数据类型
    - `KvEntry`
    - `FactEntry`
    - `EventEntry`
    - `ThingsSnapshot`
    - `DeletedThingsSummary`

- 运行时管理（可归到 Agent 生态）
  - `AiRuntimeError`
  - `AiRuntimeConfig`
  - `RuntimeAgentInfo`
  - `CreateSubAgentRequest`
  - `CreateSubAgentResult`
  - `ExternalWorkspaceBinding`
  - `BindExternalWorkspaceRequest`
  - `AiRuntime`
  - 路径：`src/frame/opendan/src/ai_runtime.rs`

## Agent Loop

- behavior
  - 行为配置（`src/frame/opendan/src/behavior/config.rs`）
    - `BehaviorConfigError`
    - `BehaviorConfig`
    - `BehaviorToolMode`
    - `BehaviorToolsConfig`
    - `BehaviorOutputProtocol`
    - `BehaviorOutputProtocolStructured`
  - 行为执行器（`src/frame/opendan/src/behavior/behavior.rs`）
    - `LLMBehaviorDeps`
    - `LLMBehavior`
    - `LLMRawResponse`
    - `LLMComputeError`
    - `AiccRequestBuilder`
  - Prompt相关（`src/frame/opendan/src/behavior/prompt.rs`）
    - `PromptPack`
    - `ChatRole`
    - `ChatMessage`
    - `PromptBuilder`
    - `Truncator`
  - 观测事件（`src/frame/opendan/src/behavior/observability.rs`）
    - `Event`

- behavior config
  - 关键配置实际落在：`BehaviorConfig` + `LLMBehaviorConfig` + `ModelPolicy` + `StepLimits`
  - 一些约定的behavior_name : `resolve_router`,`on_wakeup`
- Input
  - `InboxPack = Json`
  - `MemoryPack = Json`
  - `TraceCtx`
  - `EnvKV`
  - `StepLimits`
  - `BehaviorExecInput`
  - `ToolContext`
  - 路径：`src/frame/opendan/src/behavior/types.rs`、`src/frame/opendan/src/behavior/tool_loop.rs`

- Output (Output protocol)
  - `BehaviorLLMResult`（标准步骤输出）
  - `ExecutorReply`
  - `ActionSpec`（含 `ActionKind`、`ActionExecutionMode`、`FsScope`）
  - `ResolveRouterResult`（`src/frame/opendan/src/agent.rs`，resolve_router 阶段输出）
  - 追踪与观测输出
    - `TokenUsage`
    - `LLMOutput`
    - `Observation`（含 `ObservationSource`）
    - `ToolExecRecord`
    - `TrackInfo`
    - `LLMTrackingInfo`
