# Task Data Schema

> **历史版本（beta2.2 起）**：TaskMgr 2.0 已实施（`doc/task_mgr/task-mgr 2.0.md` 是当前
> 总规范）。本文描述的是 1.x 实现，仅作为历史实现输入保留；其中与 2.0 冲突的协议、
> 状态与数据模型描述一律以 2.0 文档和当前代码为准。

本文档以 `src/kernel/buckyos-api/src/taskdata.rs` 为准，说明 BuckyOS 对 `Task.data` 的强类型抽象。

TaskManager 的数据库字段仍然保存 JSON，核心服务只负责保存、合并和事件通知；业务侧应优先通过 `buckyos-api` 中的 `TaskDataType` / `TypedTaskData` / 各具体 `*TaskData` 类型读写，避免继续散落使用未约束的 `serde_json::Value`。

## Task Notes 边界

Task Notes 是 Task Manager 的平行功能，不属于 `Task.data`，也不参与任务的一次性运行语义。用户或系统可以通过 `add_task_note` 给某个 `task_id` 追加 note，并通过 `list_task_notes` 按 `task_id` 读取全部 notes。

Notes 持久化在独立的 `task_note` 表中，包含 `task_id`、`note_type`、`content`、可选 JSON `data`、作者信息和创建/更新时间。新增 note 不会修改 `task.data`、`task.status`、`task.progress` 或 task 的 `updated_at`，因此 Agent 可以把人类 notes 作为执行历史旁路参考，而不会污染任务自身的输入/进度/结果数据。

## 通用结构

新的 TaskData 语义统一分为三段：

```json
{
  "request": {},
  "progress": {},
  "result": {}
}
```

- `request`：请求区。给执行者或 UI 看的输入。
- `progress`：进度区。机器执行任务使用，核心字段是 item 和 byte 计数。
- `result`：结果区。任务完成或用户回填后的输出。

通用 `TaskData` 定义：

```rust
pub struct TaskData {
    pub request: Option<TaskDataRequest>,
    pub progress: Option<TaskDataProgress>,
    pub result: Option<TaskDataResult>,
    pub extra: BTreeMap<String, Value>,
}
```

`TaskDataProgress`：

```rust
pub struct TaskDataProgress {
    pub items: Option<TaskDataCounter>,
    pub bytes: Option<TaskDataCounter>,
    pub counters: BTreeMap<String, TaskDataCounter>,
    pub message: Option<String>,
    pub updated_at: Option<i64>,
}

pub struct TaskDataCounter {
    pub completed: u64,
    pub total: Option<u64>,
}
```

UI 计算百分比的规则：

- `total = Some(n)` 且 `n > 0`：可用 `completed / total` 计算百分比。
- `total = None`：只能展示“已推进多少”，不能展示总百分比。
- `TaskDataProgress::primary_percent()` 优先使用 `items`，其次使用 `bytes`。

## 机器任务和人工任务

通用请求区有两类：

```rust
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum TaskDataRequest {
    Machine { payload: Value },
    Human { prompt: HumanTaskPrompt },
}
```

机器任务使用 `Machine`，业务 payload 可进一步由具体 `*TaskData` 强类型约束。

人工任务使用 `HumanTaskPrompt` 描述结构化 UI：

```rust
pub struct HumanTaskPrompt {
    pub title: Option<String>,
    pub message: Option<String>,
    pub controls: Vec<HumanPromptControl>,
}
```

当前支持的 UI 控件：

- `choice`：单选或多选，包含 `id`、`label`、`options`、`multiple`、`required`。
- `text_input`：文本输入，包含 `id`、`label`、`placeholder`、`default_value`、`multiline`、`required`。

人工任务结果使用：

```rust
TaskDataResult::Human {
    action: Option<String>,
    values: BTreeMap<String, Value>,
    submitted_by: Option<String>,
    submitted_at: Option<i64>,
}
```

`values` 的 key 应对应 prompt control 的 `id`。

## 类型安全入口

强类型入口：

```rust
pub fn parse_typed_task_data(
    task_data_type: &str,
    data: Value,
) -> Result<TypedTaskData, TaskDataParseError>
```

解析流程：

1. 用 `task_data_type` 字符串解析为 `TaskDataType`。
2. 根据 `TaskDataType` 解析为 `TypedTaskData` 的具体枚举分支。
3. 优先解析新结构；如果失败，部分类型会尝试解析旧 JSON schema，并转换成新的 request / progress / result 语义结构。

未知 `task_data_type` 会返回 `TaskDataParseError::UnknownTaskDataType`。

## 绑定总表

| task_data_type 字符串 | TypedTaskData 分支 | 主结构 | legacy JSON 兼容 |
| --- | --- | --- | --- |
| `download` | `Download` | `DownloadTaskData` | 是 |
| `scheduler.dispatch_thunk` | `SchedulerDispatchThunk` | `ThunkTaskData` | 是 |
| `workflow/run` | `WorkflowRun` | `WorkflowRunTaskData` | 是 |
| `workflow/step` | `WorkflowStep` | `WorkflowStepTaskData` | 是 |
| `workflow/map_shard` | `WorkflowMapShard` | `WorkflowMapShardTaskData` | 是 |
| `workflow/thunk` | `WorkflowThunk` | `ThunkTaskData` | 是 |
| `workflow/schedule` | `WorkflowSchedule` | `WorkflowScheduleTaskData` | 是 |
| `workflow.send_message` | `WorkflowSendMessage` | `SendMessageTaskData` | 是 |
| `agent.delegate` | `AgentDelegate` | `AgentDelegateTaskData` | 是 |
| `human.input` | `HumanInput` | `HumanInputTaskData` | 是 |
| `opendan.async_tool` | `OpenDanAsyncTool` | `OpenDanAsyncToolTaskData` | 任意 payload 作为 request |
| `aicc.compute` | `AiccCompute` | `AiccComputeTaskData` | 是 |
| `app.install` | `AppInstall` | `AppInstallTaskData` | 是，旧名字段结构 |
| `app.uninstall` | `AppUninstall` | `AppUninstallTaskData` | 是，旧名字段结构 |
| `app.start` | `AppStart` | `AppStartTaskData` | 是，旧名字段结构 |
| `app.update` | `AppUpdate` | `AppUpdateTaskData` | 是，旧名字段结构 |
| `workflow.execute_rpc` | `ServiceRpc` | `ServiceRpcTaskData` | 是，旧 wrapper 为 `service_rpc` |
| `workflow.run` | `WorkflowRunTarget` | `WorkflowRunTargetTaskData` | 是，旧 wrapper 为 `workflow_run` |

不再纳入强类型 schema：

- `opendan.command`：当前未作为稳定执行协议使用，不在 `TaskDataType` 中暴露。
- `test`、`test_type`、`type1`、`type2` 等测试类型。
- `opendan.behavior` 等仅测试或临时使用的类型。

## `download`

强类型：

```rust
pub struct DownloadTaskData {
    pub request: DownloadTaskRequest,
    pub progress: Option<TaskDataProgress>,
    pub result: Option<DownloadTaskResult>,
    pub extra: BTreeMap<String, Value>,
}
```

请求区：

```rust
pub struct DownloadTaskRequest {
    pub download_url: Option<String>,
    pub urls: Vec<String>,
    pub objid: Option<String>,
    pub resolved_objid: Option<String>,
    pub options: Option<DownloadTaskOptions>,
}
```

进度区：

- legacy `download.downloaded_bytes` / `download.total_bytes` 会转换成 `progress.bytes`。

结果区：

```rust
pub struct DownloadTaskResult {
    pub state: Option<String>,
    pub mode: Option<String>,
    pub local_path: Option<String>,
    pub output: Option<Value>,
}
```

legacy 兼容：

- 顶层 `download_url`、`urls`、`objid`、`resolved_objid` 进入 `request`。
- `download_options` 进入 `request.options`。
- `download.state`、`mode`、`local_path`、`result` 进入 `result`。
- 未识别字段进入 `extra`。

## Thunk 执行任务

适用类型：

- `scheduler.dispatch_thunk`
- `workflow/thunk`

强类型：

```rust
pub struct ThunkTaskData {
    pub request: ThunkTaskRequest,
    pub progress: Option<TaskDataProgress>,
    pub result: Option<ThunkExecutionResult>,
    pub executor: Option<NodeExecutorTaskState>,
    pub extra: BTreeMap<String, Value>,
}
```

请求区：

```rust
pub struct ThunkTaskRequest {
    pub runner: Option<String>,
    pub node_id: Option<String>,
    pub thunk_obj_id: Option<String>,
    pub thunk: Option<ThunkObject>,
    pub function_object: Option<FunctionObject>,
    pub dispatch: Option<ThunkDispatch>,
    pub extra: BTreeMap<String, Value>,
}
```

结果区：

- `result` 使用 `ThunkExecutionResult`。
- node executor 的运行状态放在 `executor: Option<NodeExecutorTaskState>`，包含 `status`、`task_id`、`work_dir`、`result_path`。

legacy 兼容：

- 旧 `executor_result` 转为 `result`。
- 旧 `executor` 转为 `executor`。
- `workflow/thunk` 旧 schema 中的 `workflow.run_id`、`attempt`、`shard_index` 会转入 `extra` 的 `workflow_run_id`、`workflow_attempt`、`workflow_shard_index`。
- `workflow.node_id` / `workflow.thunk_obj_id` 会作为 request 中缺省的 `node_id` / `thunk_obj_id`。

## `workflow/run`

强类型：

```rust
pub struct WorkflowRunTaskData {
    pub request: WorkflowRunTaskRequest,
    pub progress: Option<TaskDataProgress>,
    pub result: Option<WorkflowRunTaskResult>,
    pub human_action: Option<TaskHumanAction>,
    pub last_error: Option<TaskDataErrorInfo>,
}
```

请求区：

- `run_id`
- `workflow_id`
- `workflow_name`
- `plan_version`

进度区：

- legacy `workflow.summary` 会转为 `progress.items`。
- `completed` 取 `summary["Completed"]`。
- `total` 取 `summary` 所有计数之和。

结果区：

- `status`
- `summary`
- `updated_at`

人工动作：

```rust
pub struct TaskHumanAction {
    pub kind: String,
    pub payload: Option<Value>,
    pub actor: Option<String>,
    pub submitted_at: Option<i64>,
}
```

`human_action` 是用户通过 TaskMgr UI 写回的动作入口。

## `workflow/step`

强类型：

```rust
pub struct WorkflowStepTaskData {
    pub request: WorkflowStepTaskRequest,
    pub progress: Option<TaskDataProgress>,
    pub result: Option<Value>,
    pub human_action: Option<TaskHumanAction>,
    pub last_error: Option<TaskDataErrorInfo>,
}
```

请求区：

- `run_id`
- `node_id`
- `attempt`
- `executor`
- `prompt`
- `output_schema`
- `subject`
- `subject_obj_id`
- `stakeholders`
- `waiting_human_since`

结果区：

- legacy `output` 转为 `result`。

人工动作：

- legacy `human_action` 保持为 `human_action`。
- 支持 `approve`、`modify`、`reject`、`retry`、`skip`、`abort`、`rollback`、`submit_output` 等业务动作。

## `workflow/map_shard`

强类型：

```rust
pub struct WorkflowMapShardTaskData {
    pub request: WorkflowMapShardTaskRequest,
    pub progress: Option<TaskDataProgress>,
    pub result: Option<Value>,
    pub last_error: Option<TaskDataErrorInfo>,
}
```

请求区：

- `run_id`
- `node_id`
- `shard_index`
- `attempt`
- `item`

结果区：

- legacy `output` 转为 `result`。

## `workflow/schedule`

强类型：

```rust
pub struct WorkflowScheduleTaskData {
    pub request: WorkflowScheduleTaskRequest,
    pub progress: Option<TaskDataProgress>,
    pub result: Option<WorkflowScheduleTaskResult>,
}
```

请求区：

- `schedule_id`
- `name`
- `status`
- `schedule`
- `target`

结果区：

- `next_fire_at`
- `last_fire_at`
- `last_task_id`
- `last_run_id`
- `consecutive_failures`
- `last_error`

legacy 兼容：

- 旧 wrapper `schedule` 中的字段按上述语义拆入 request / result。

## `workflow.send_message`

强类型：

```rust
pub struct SendMessageTaskData {
    pub request: SendMessageTaskRequest,
    pub result: Option<Value>,
}
```

请求区：

```rust
pub struct SendMessageTaskRequest {
    pub to: String,
    pub text: String,
    pub trigger: Option<ScheduleTriggerContext>,
}
```

legacy 兼容：

- 旧 wrapper `send_message` 转为 `request`。

## `agent.delegate`

强类型：

```rust
pub struct AgentDelegateTaskData {
    pub request: AgentDelegateTaskRequest,
    pub progress: Option<AgentDelegateProgress>,
    pub result: Option<AgentDelegateTaskResult>,
    pub route: Option<Value>,
    pub blocker: Option<Value>,
    pub human_input: Option<Value>,
    pub error: Option<TaskDataErrorInfo>,
}
```

请求区：

- `version`
- `source`
- `title`
- `purpose`
- `requester_agent_id`
- `owner_session_id`
- `input`
- `workspace_hints`
- `reason_messages`
- `trigger`

进度区：

```rust
pub struct AgentDelegateProgress {
    pub execution: Option<Value>,
    pub one_line_status: Option<String>,
    pub updated_at_ms: Option<i64>,
}
```

结果区：

- `status`
- `report`
- `next_behavior`
- `extra`

附加状态：

- `route`：路由信息。
- `blocker`：等待中的子任务。
- `human_input`：父任务记录的人类输入回填。
- `error`：错误信息。

legacy 兼容：

- 旧 wrapper `agent_delegate` 会按上述字段拆分。
- `agent_delegate.execution.one_line_status` 和 `updated_at_ms` 会提升到 `progress`。

## `human.input`

强类型：

```rust
pub struct HumanInputTaskData {
    pub request: HumanInputTaskRequest,
    pub result: Option<HumanInputTaskResult>,
}
```

请求区：

- `version`
- `kind`
- `question`
- `required_by`
- `candidates`
- `response_schema`

结果区：

- `response`
- `answered_by`
- `answered_at`

legacy 兼容：

- 旧 wrapper `human_input` 会拆成 request / result。
- 当旧 `response` 为 `null` 或不存在时，`result = None`。

## `opendan.async_tool`

强类型：

```rust
pub struct OpenDanAsyncToolTaskData {
    pub request: Value,
    pub result: Option<Value>,
}
```

该类型没有固定业务 wrapper。旧实现直接把调用方 payload 作为 `Task.data`，新 parser 会把任意 JSON payload 作为 `request`。

## `aicc.compute`

强类型：

```rust
pub struct AiccComputeTaskData {
    pub request: AiccComputeTaskRequest,
    pub progress: Option<AiccComputeProgress>,
    pub result: Option<AiccComputeTaskResult>,
    pub error: Option<Value>,
}
```

请求区：

- `version`
- `external_task_id`
- `tenant_id`
- `event_ref`
- `session_id`
- `owner_session_id`
- `request`
- `provider_input`
- `route`
- `created_at_ms`

进度区：

- `status`
- `updated_at_ms`
- `events`

结果区：

- `output`
- `provider_output`

legacy 兼容：

- 旧顶层 `session_id` / `owner_session_id` 和 wrapper `aicc` 会拆入新结构。
- `aicc.error` 转为 `error`。

## App lifecycle

新的 task_data_type 字符串使用点号命名：

- `app.install`
- `app.uninstall`
- `app.start`
- `app.update`

### `app.install`

```rust
pub struct AppInstallTaskData {
    pub request: AppInstallTaskRequest,
    pub result: Option<String>,
}
```

请求区：

- `app_id`
- `user_id`
- `version`
- `content_id`

### `app.uninstall`

请求区：

- `app_id`
- `user_id`
- `remove_data`

### `app.start`

请求区：

- `app_id`
- `user_id`

### `app.update`

请求区：

- `app_id`
- `user_id`
- `from_version`
- `to_version`
- `content_id`

legacy 兼容：

- 旧裸 JSON 字段会整体解析为对应 `request`。

## `workflow.execute_rpc`

强类型分支名为 `ServiceRpc`，task_data_type 字符串为 `workflow.execute_rpc`。

```rust
pub struct ServiceRpcTaskData {
    pub request: ServiceRpcTaskRequest,
    pub result: Option<Value>,
}
```

请求区：

- `service`
- `method`
- `params`
- `trigger`

legacy 兼容：

- 旧 wrapper `service_rpc` 会转为 `request`。

## `workflow.run`

这是计划任务 target 特殊值，不是 workflow tracker 创建的 `workflow/run` run task。

强类型：

```rust
pub struct WorkflowRunTargetTaskData {
    pub request: WorkflowRunTargetTaskRequest,
    pub result: Option<Value>,
}
```

请求区：

- `workflow_id`
- `input`
- `trigger`

legacy 兼容：

- 旧 wrapper `workflow_run` 会转为 `request`。

## 写入和兼容规则

TaskManager 写入语义不变：

- `update_task(..., data_patch)`：当 patch 是 object 时递归合并；patch 中某 key 为 `null` 时删除目标 object 中该 key；patch 非 object 时整体替换。
- `update_task_data(id, data)`：整体替换 `Task.data`。

`update_task_progress` 的历史行为仍会额外合并：

```json
{
  "completed_items": 1,
  "total_items": 10
}
```

新代码应优先写：

```json
{
  "progress": {
    "items": {
      "completed": 1,
      "total": 10
    }
  }
}
```

兼容读取策略：

- parser 会支持当前 `taskdata.rs` 中显式实现的 legacy wrapper。
- 新增业务类型应先扩展 `TaskDataType`、`TypedTaskData` 和具体 `*TaskData` 类型，再更新本文档。
- 默认不要把未建模的业务字段放在顶层；确实需要保留时使用对应结构中的 `extra`。

## 文档和代码联动要求

修改 `src/kernel/buckyos-api/src/taskdata.rs` 时，应同步检查并更新本文档，尤其是：

- `TASK_DATA_TYPE_*` 字符串。
- `TaskDataType` 和 `TypedTaskData` 分支。
- 各 `request` / `progress` / `result` 字段。
- legacy parser 的兼容行为。
