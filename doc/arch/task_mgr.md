# TaskMgr 架构与实现说明

本文档整合 `src/kernel/task_manager/readme.md`、`src/kernel/task_manager/review.md` 的设计内容，并以当前代码实现为准描述 TaskMgr 的服务定位、数据模型、RPC 协议、事件语义、存储和运行方式。

当前实现入口：

- 协议与 SDK：`src/kernel/buckyos-api/src/task_mgr.rs`
- 服务实现：`src/kernel/task_manager/src/server.rs`
- 持久化：`src/kernel/task_manager/src/task_db.rs`
- 任务构造：`src/kernel/task_manager/src/task.rs`
- 下载任务执行器：`src/kernel/task_manager/src/download_executor.rs`
- 服务注册：`src/kernel/scheduler/src/system_config_builder.rs`

## 1. 定位

TaskMgr 是 BuckyOS 的通用长任务状态总账服务。它不负责定义业务执行逻辑，而是为系统服务、应用、Workflow、AICC、安装器等模块提供统一的任务创建、状态跟踪、进度上报、错误记录、任务树查询和事件订阅能力。

TaskMgr 的核心职责是：

1. 给长时间运行或需要人类介入的操作分配稳定的 `task_id`。
2. 维护任务状态快照，包括状态、进度、消息和业务扩展数据。
3. 维护父子任务关系，使复杂工作可以表现为一棵任务树。
4. 通过权限字段约束任务读写范围。
5. 在任务变化时发布 kevent 事件，避免调用方长期轮询。
6. 为常见系统任务提供可复用能力，目前内置了下载任务执行路径。

TaskMgr 不是调度器，也不是业务编排器。调度器、Workflow、AICC、Control Panel 等模块决定“做什么、怎么做、在哪里做”，TaskMgr 负责“外部如何看到这件事做到了哪里，以及如何把通用状态变更写回同一个可观察面”。

## 2. 服务与运行时

TaskMgr 是 kernel service：

- service unique id：`task-manager`
- service name：`task-manager`
- main port：`3380`
- HTTP kRPC path：`/kapi/task-manager`
- service doc show name：`Task Manager`
- selector type：`Single`

启动流程在 `start_task_manager_service` 中完成：

1. 以 `BuckyOSRuntimeType::KernelService` 初始化 runtime。
2. login 到系统并注册全局 runtime。
3. 从 service spec 中解析 TaskMgr RDB instance。
4. 初始化 `TaskDb` 并应用 schema。
5. 构造 `TaskManagerService`。
6. 在 `3380` 端口注册 `/kapi/task-manager` HTTP kRPC server。

Scheduler 在初始化 system-config 时通过 `add_task_mgr()` 写入 `services/task-manager/spec`，并在 `install_config.rdb_instances` 中写入 TaskMgr 的默认 RDB 配置。

## 3. 数据模型

当前公开任务模型定义在 `buckyos-api/src/task_mgr.rs`：

```rust
pub struct Task {
    pub id: i64,
    pub user_id: String,
    pub app_id: String,
    pub parent_id: Option<i64>,
    pub root_id: String,
    pub name: String,
    pub task_type: String,
    pub status: TaskStatus,
    pub progress: f32,
    pub message: Option<String>,
    pub data: serde_json::Value,
    pub permissions: TaskPermissions,
    pub created_at: u64,
    pub updated_at: u64,
}
```

字段语义：

| 字段 | 语义 |
| --- | --- |
| `id` | 自增任务 ID，是单个 Task 的主键 |
| `user_id` | 任务所属用户，创建时来自显式参数或 RPC session token |
| `app_id` | 任务所属应用，创建时来自显式参数、`app_name` 兼容字段或 RPC session token |
| `parent_id` | 父任务 ID，空表示根任务 |
| `root_id` | 任务树根 ID；根任务默认等于自身 `id` 的字符串形式 |
| `name` | 任务稳定名称；当前数据库按 `(app_id, user_id, name)` 建唯一索引 |
| `task_type` | 业务任务类型，如 `download`、`workflow/run`、`workflow/step` |
| `status` | 当前任务状态 |
| `progress` | 0 到 100 的进度百分比 |
| `message` | 面向用户或 UI 的当前状态说明 |
| `data` | 业务扩展 JSON，是 TaskMgr 不理解的结构化载荷 |
| `permissions` | 任务读写权限 |
| `created_at` | 创建时间，Unix timestamp 秒 |
| `updated_at` | 最近更新时间，Unix timestamp 秒 |

### 3.1 状态

`TaskStatus` 当前取值：

| 状态 | 语义 |
| --- | --- |
| `Pending` | 已创建，尚未开始执行 |
| `Running` | 正在执行 |
| `Paused` | 已暂停 |
| `Completed` | 成功结束 |
| `Failed` | 失败结束 |
| `Canceled` | 被取消，通常由用户或上层服务主动触发 |
| `WaitingForApproval` | 阻塞等待人工审批或确认 |

`Completed`、`Failed`、`Canceled` 是终态。SDK 的 `wait_for_task_end` 会轮询直到任务进入这些终态之一。

### 3.2 权限

权限模型来自旧 `review.md` 的 ACL 设计，当前实现保留为 `TaskPermissions`：

```rust
pub enum TaskScope {
    Private,
    User,
    System,
}

pub struct TaskPermissions {
    pub read: TaskScope,
    pub write: TaskScope,
}
```

默认权限：

```rust
read = TaskScope::User
write = TaskScope::Private
```

当前服务端判断规则：

| Scope | read/write 判定 |
| --- | --- |
| `Private` | `task.user_id == ctx.user_id && task.app_id == ctx.app_id` |
| `User` | `task.user_id == ctx.user_id` |
| `System` | 请求方 `app_id` 必须是 `kernel` 或 `system` |

兼容规则：

- 如果请求上下文的 `user_id` 和 `app_id` 都为空，当前实现视为允许访问，主要用于本地/in-process 或旧调用路径。
- 如果任务自身 `user_id` 为空，则只按 `app_id` 做弱隔离：任务 `app_id` 为空或等于请求方 `app_id` 时允许访问。

注意：当前 `create_task` / `create_download_task` 会从显式参数或 RPC session token 解析请求上下文；部分读写 handler 当前仍使用空 source context，因此真实权限收敛能力依赖调用方是否传入 `source_user_id` / `source_app_id` 或后续完善 RPC context 接入。

### 3.3 父子任务与 root_id

TaskMgr 支持任务树：

- `parent_id = None` 表示根任务。
- 创建子任务时必须对父任务有写权限。
- 子任务继承父任务的 `root_id`。
- 根任务如果没有显式 `root_id`，创建落库后会把 `root_id` 设置为自身 `id.to_string()`。

根 ID 的解析优先级：

1. 如果创建参数 `parent_id` 存在，使用父任务 `root_id`。
2. 否则如果 `CreateTaskOptions.root_id` 非空，使用该值。
3. 否则从 `data` 中尝试读取 `/root_id`、`/rootid`、`/meta/root_id`、`/meta/rootid`。
4. 否则根任务落库后用自身 `id` 作为 `root_id`。

这个模型用于 Workflow 等复杂任务：一个 Run 可以是根任务，Step、Thunk、Map shard 作为子孙任务挂在同一棵树下，外部通过 `root_id` 查询或订阅整棵树。

## 4. 存储

TaskMgr 使用 RDB 持久化，默认 backend 是 Sqlite，也定义了 Postgres 等价 schema。RDB instance ID 为：

```text
task-mgr-main
```

Schema version 当前为：

```text
1
```

默认 Sqlite 连接为空，由 rdb_mgr 在解析时生成 `sqlite://$appdata/main.db`。服务启动时会从 service spec 中获取 RDB instance，并应用 schema。测试场景可以直接用编译期默认 DDL 打开临时 Sqlite。

### 4.1 表结构

主表为 `task`，逻辑字段包括：

| 列 | 说明 |
| --- | --- |
| `id` | 自增主键 |
| `name` | 任务名称 |
| `title` | 当前写入为 `name`，保留列 |
| `task_type` | 任务类型 |
| `status` | 字符串形式的 `TaskStatus` |
| `progress` | 进度百分比 |
| `total_items` | 旧进度 API 的总项数 |
| `completed_items` | 旧进度 API 的已完成项数 |
| `error_message` | 错误消息，`update_task_error` 写入 |
| `data` | JSON 字符串 |
| `created_at` | 创建时间 |
| `updated_at` | 更新时间 |
| `user_id` | 所属用户 |
| `app_id` | 所属应用 |
| `parent_id` | 父任务 ID，外键，删除父任务会级联删除子任务 |
| `root_id` | 根任务 ID 字符串 |
| `permissions` | JSON 字符串 |
| `message` | 当前状态消息 |

主要索引：

- `idx_task_name_scope`：`(app_id, user_id, name)` 唯一索引
- `idx_task_root_status`：`(root_id, status)`
- `idx_task_parent`：`parent_id`
- `idx_task_app_created`：`(app_id, created_at DESC)`
- `idx_task_status_created`：`(status, created_at DESC)`
- `idx_task_type_created`：`(task_type, created_at DESC)`

### 4.2 JSON 更新语义

`update_task` 是当前推荐的快照更新接口，支持同时更新 `status`、`progress`、`message`、`data`。

`data` 字段使用 merge patch 语义：

- patch 是 object 时，递归合并到现有 `data`。
- patch 中某个 key 的值是 `null` 时，删除目标 object 中对应 key。
- patch 非 object 时，整体替换目标值。

`update_task_data` 是旧接口，语义是把传入 JSON 整体写入 `data`，不做 merge。

## 5. TaskData Schema 约定

`Task.data` 是业务扩展 JSON。TaskMgr 核心服务只理解少量通用字段，其它字段由对应 `task_type` 的生产者和消费者约定。UI 应根据 `task_type` 优先选择结构化渲染；未知字段必须能退化为原始 JSON 展示。

TaskData schema 与 `task_type` 的绑定关系如下。一个任务的 `task_type` 决定它的 `data` 主 schema；通用字段可以叠加在任意 schema 上。

| task_type | 绑定 schema | 创建方 | 主要更新方 | 主要消费方 |
| --- | --- | --- | --- | --- |
| `download` | `download` schema | TaskMgr `create_download_task` | TaskMgr download executor | Control Panel、Repo/File 相关下载调用方、TaskMgr UI |
| `workflow/run` | `workflow.run` schema | Workflow `TaskManagerTaskTracker` | Workflow service | Workflow service、TaskMgr UI、Workflow WebUI deep link |
| `workflow/step` | `workflow.step` schema | Workflow `TaskManagerTaskTracker` | Workflow service、用户经 TaskMgr UI 写 `human_action` | Workflow service、TaskMgr UI |
| `workflow/map_shard` | `workflow.map_shard` schema | Workflow `TaskManagerTaskTracker` | Workflow service | Workflow service、TaskMgr UI |
| `workflow/thunk` | `workflow.thunk` schema | Workflow `TaskManagerTaskTracker` | Scheduler / node executor | Scheduler、node executor、Workflow service、TaskMgr UI |
| `aicc.compute` | `aicc` schema | AICC | AICC provider task event sink | AICC、OpenDAN、TaskMgr UI |
| `llm_behavior` | OpenDAN behavior schema | OpenDAN Behavior | OpenDAN Behavior | OpenDAN、TaskMgr UI |
| `app_install` | app lifecycle schema | Control Panel app installer | Control Panel app installer | Control Panel、TaskMgr UI |
| `app_uninstall` | app lifecycle schema | Control Panel app installer | Control Panel app installer | Control Panel、TaskMgr UI |
| `app_start` | app lifecycle schema | Control Panel app installer | Control Panel app installer | Control Panel、TaskMgr UI |
| `app_update` | app lifecycle schema | Control Panel app installer | Control Panel app installer | Control Panel、TaskMgr UI |
| `scheduler.dispatch_thunk` | node executor dispatch schema | Scheduler / node executor dispatch path | node executor | node executor、scheduler、TaskMgr UI |

绑定规则：

1. `task_type` 是选择 TaskData schema 的主键，UI 和服务都不应只靠字段猜类型。
2. 同一个 schema 可以绑定多个 `task_type`，例如 app lifecycle schema 同时服务 `app_install`、`app_uninstall`、`app_start`、`app_update`。
3. 同一个 task data 可以包含多个 namespace，但主 namespace 应与 `task_type` 对应。例如 `workflow/thunk` 可以同时包含 `workflow` 和 `executor`，但它仍按 `workflow/thunk` schema 解释。
4. 跨模块字段必须放在稳定 namespace 下；新增 schema 时应先明确 `task_type`、创建方、更新方、消费方。

### 5.1 通用字段

以下字段在多个模块中复用，不绑定单一 `task_type`：

```json
{
  "root_id": "optional-root-id",
  "rootid": "optional-root-id",
  "meta": {
    "root_id": "optional-root-id",
    "rootid": "optional-root-id"
  },
  "session_id": "optional-session-id",
  "owner_session_id": "optional-owner-session-id",
  "completed_items": 1,
  "total_items": 10
}
```

说明：

- `root_id` / `rootid` / `meta.root_id` / `meta.rootid`：创建根任务时 TaskMgr 会尝试读取这些字段作为 `Task.root_id`。如果请求参数中已有 `CreateTaskOptions.root_id`，则优先使用请求参数。
- `session_id` / `owner_session_id`：OpenDAN、AICC 等模块用于把任务归到一次会话或 agent loop。
- `completed_items` / `total_items`：`update_task_progress` 会把这两个字段同步写入 `data`，并据此计算 `Task.progress`。

新 schema 应避免把业务字段直接铺满根对象。推荐使用以模块名命名的顶层 namespace，例如 `workflow`、`download`、`aicc`、`executor`。

### 5.2 `download`

任务类型：

```text
download
```

绑定关系：

| 项 | 说明 |
| --- | --- |
| 绑定 task_type | `download` |
| data 主 namespace | 根字段 `download_url` / `urls` / `objid` 与 `download` |
| 创建方 | TaskMgr `create_download_task` |
| 更新方 | TaskMgr download executor |
| 消费方 | 发起下载的业务模块、TaskMgr UI |

初始 schema：

```json
{
  "download_url": "https://example.com/file.pkg",
  "urls": ["https://example.com/file.pkg"],
  "objid": "optional-cyfs-obj-id",
  "download_options": {
    "local_path": "/optional/output/path",
    "filename": "optional-name.pkg",
    "default_remote_url": "optional-remote",
    "timeout_ms": 60000,
    "timeout_secs": 60,
    "obj_id_in_host": false
  },
  "download": {
    "state": "pending",
    "mode": "named_store",
    "downloaded_bytes": 0,
    "total_bytes": 1024,
    "local_path": "/resolved/local/path",
    "result": {}
  }
}
```

字段说明：

| 字段 | 说明 |
| --- | --- |
| `download_url` | 主下载 URL |
| `urls` | 所有可用来源 URL；重复创建同一下载任务时会合并 |
| `objid` | CYFS ObjId；存在时下载模式为 `named_store` |
| `download_options.local_path` | 无 ObjId 时指定本地输出路径 |
| `download_options.filename` | 无 `local_path` 时指定输出文件名 |
| `download_options.default_remote_url` | NDN client 默认远端 |
| `download_options.timeout_ms` / `timeout_secs` | 下载超时 |
| `download_options.obj_id_in_host` | 透传给 NDN client |
| `download.state` | `pending` / `running` / `completed` / `failed` / `canceled` |
| `download.mode` | `named_store` 或 `local_file` |
| `download.downloaded_bytes` | 已下载字节数 |
| `download.total_bytes` | 总字节数，未知时可缺省 |
| `download.local_path` | 本地文件下载完成后的输出路径 |
| `download.result` | 下载完成后的执行结果摘要 |

TaskMgr 下载执行器会维护 `download.state`、字节进度和最终结果，同时更新 `Task.status`、`Task.progress` 和 `Task.message`。

### 5.3 `workflow/*`

任务类型：

```text
workflow/run
workflow/step
workflow/map_shard
workflow/thunk
```

绑定关系：

| task_type | data 主 namespace | 创建方 | 更新方 | 消费方 |
| --- | --- | --- | --- | --- |
| `workflow/run` | `workflow`，可选 `human_action` / `last_error` | Workflow tracker | Workflow service、用户动作入口 | Workflow service、TaskMgr UI |
| `workflow/step` | `workflow`，可选 `output` / `human_action` / `last_error` | Workflow tracker | Workflow service、用户动作入口 | Workflow service、TaskMgr UI |
| `workflow/map_shard` | `workflow`，可选 `output` / `last_error` | Workflow tracker | Workflow service | Workflow service、TaskMgr UI |
| `workflow/thunk` | `workflow`，可选 `executor` / `executor_result` | Workflow tracker | Scheduler / node executor | Scheduler、node executor、Workflow service |

Workflow 的运行期可观察面是一棵 TaskMgr 任务树：

```text
workflow/run
└── workflow/step
    ├── workflow/map_shard
    └── workflow/thunk
```

#### 5.3.1 `workflow/run`

`workflow/run` 的 `task_type` 固定绑定 Run 根任务 schema。该 schema 表示一次 Workflow Run 的整体状态，`Task.root_id` 通常等于 `workflow.run_id`。

```json
{
  "workflow": {
    "run_id": "run-...",
    "workflow_id": "workflow-...",
    "workflow_name": "Daily scan",
    "plan_version": 1,
    "status": "Running",
    "summary": {
      "Running": 1,
      "Completed": 3,
      "Failed": 0
    },
    "updated_at": 1730000000
  },
  "human_action": {
    "kind": "rollback",
    "payload": {
      "target_node_id": "scan"
    },
    "actor": "user-A",
    "submitted_at": 1730000000
  },
  "last_error": null
}
```

`workflow/run` 是根任务，通常使用 `run_id` 作为 `Task.root_id`。`human_action` 只在用户对整个 Run 执行图级动作时出现。

#### 5.3.2 `workflow/step`

`workflow/step` 的 `task_type` 固定绑定 Step / control-node schema。该 schema 是人类审批、修改、重试、跳过等动作的主要入口，用户写入的动作统一放在 `human_action`。

```json
{
  "workflow": {
    "run_id": "run-...",
    "node_id": "scan",
    "attempt": 2,
    "executor": "service::aicc.complete",
    "prompt": "请检查扫描结果",
    "output_schema": {},
    "subject": {},
    "subject_obj_id": "optional-object-id",
    "stakeholders": ["user-A", "role:reviewer"],
    "waiting_human_since": 1730000000
  },
  "output": {},
  "human_action": {
    "kind": "approve",
    "payload": {},
    "actor": "user-A",
    "submitted_at": 1730000000
  },
  "last_error": {
    "message": "invalid action payload",
    "ts": 1730000001
  }
}
```

字段说明：

| 字段 | 说明 |
| --- | --- |
| `workflow.run_id` | 所属 Run |
| `workflow.node_id` | Workflow 节点 ID |
| `workflow.attempt` | 当前执行尝试次数 |
| `workflow.executor` | 节点执行器标识 |
| `workflow.prompt` | 人类确认或 LLM 类步骤的提示文本 |
| `workflow.output_schema` | 输出 schema，供 UI 渲染表单或校验提示 |
| `workflow.subject` / `subject_obj_id` | 待处理对象的内联数据或对象引用 |
| `workflow.stakeholders` | 可处理该任务的用户或角色提示 |
| `workflow.waiting_human_since` | 进入人工等待态的时间 |
| `output` | step 成功输出 |
| `human_action` | 用户经 TaskMgr UI 写入的动作 |
| `last_error` | workflow 校验失败或执行失败时写回 |

`human_action.kind` 当前约定值包括：

```text
approve
modify
reject
retry
skip
abort
rollback
submit_output
```

`modify`、`rollback`、`submit_output` 等动作通常需要 `payload`。非法动作由 workflow 写回 `last_error`，并让 task 保持 `WaitingForApproval`。

#### 5.3.3 `workflow/map_shard`

`workflow/map_shard` 的 `task_type` 固定绑定 for_each shard schema。该 schema 只描述单个 shard 的 item、输出和错误，不承载整 Run 的图级动作。

```json
{
  "workflow": {
    "run_id": "run-...",
    "node_id": "for_each_node",
    "shard_index": 0,
    "attempt": 1,
    "item": {}
  },
  "output": {},
  "last_error": {
    "message": "shard failed",
    "ts": 1730000001
  }
}
```

`workflow/map_shard` 表示 `for_each` 展开后的单个 shard。它通常挂在对应 step 下；如果 step task 尚未创建，当前实现会退回挂到 run 根任务下。

#### 5.3.4 `workflow/thunk`

`workflow/thunk` 的 `task_type` 固定绑定 Thunk 执行 schema。Workflow 创建时只写 `workflow` 描述字段；调度和执行阶段会叠加 node executor 相关字段。

```json
{
  "workflow": {
    "run_id": "run-...",
    "node_id": "scan",
    "thunk_obj_id": "obj-...",
    "attempt": 1,
    "shard_index": null
  },
  "node_id": "node-1",
  "runner": "optional-runner",
  "thunk_obj_id": "obj-...",
  "executor": {
    "status": "running",
    "task_id": 123,
    "work_dir": "/path/to/workdir",
    "result_path": "/path/to/executor_result.json"
  },
  "executor_result": {}
}
```

Workflow 只负责创建 `workflow/thunk` task 并写入描述性字段；scheduler / node executor 后续负责更新 status、progress、payload、error。node executor 要求 task data 中存在 `thunk`、`function_object`、`node_id` 等执行字段，并会写回 `executor` 与 `executor_result`。

### 5.4 `aicc.compute`

任务类型：

```text
aicc.compute
```

绑定关系：

| 项 | 说明 |
| --- | --- |
| 绑定 task_type | `aicc.compute` |
| data 主 namespace | `aicc`，外加根字段 `rootid` / `session_id` / `owner_session_id` |
| 创建方 | AICC |
| 更新方 | AICC `TaskAuditSink` / provider task event sink |
| 消费方 | AICC、OpenDAN、TaskMgr UI |

初始 schema：

```json
{
  "rootid": "session-or-default-root",
  "session_id": "optional-session-id",
  "owner_session_id": "optional-session-id",
  "aicc": {
    "version": 1,
    "external_task_id": "external-task-id",
    "status": "pending",
    "created_at_ms": 1730000000000,
    "updated_at_ms": 1730000000000,
    "tenant_id": "user-or-tenant",
    "event_ref": "optional-event-ref",
    "rootid": "session-or-default-root",
    "session_id": "optional-session-id",
    "request": {},
    "provider_input": null,
    "route": {},
    "output": null,
    "provider_output": null,
    "error": null,
    "events": []
  }
}
```

运行过程中 AICC 会维护：

| 字段 | 说明 |
| --- | --- |
| `aicc.status` | `pending` / `queued` / `running` / `succeeded` / `failed` / `canceled` |
| `aicc.route.primary_instance_id` | 主 provider instance |
| `aicc.route.fallback_instance_ids` | fallback provider instances |
| `aicc.route.provider_model` | provider 实际模型 |
| `aicc.output` | 最终 `AiResponseSummary` 或 provider 返回摘要 |
| `aicc.provider_input` / `provider_output` | provider IO 调试数据 |
| `aicc.error` | 失败或取消 payload |
| `aicc.events` | 最近一批 AICC task events，长度由 AICC 保留策略控制 |

OpenDAN 会通过 `/aicc/external_task_id` 建立 AICC 外部任务 ID 到 TaskMgr 任务 ID 的映射，并兼容读取多种历史结果形态：完整 `CompleteResponse`、`{ "result": AiResponseSummary }`、`{ "aicc": { "output": AiResponseSummary } }` 或直接 `AiResponseSummary`。

### 5.5 `llm_behavior`

任务类型：

```text
llm_behavior
```

绑定关系：

| 项 | 说明 |
| --- | --- |
| 绑定 task_type | `llm_behavior` |
| data 主 namespace | 当前为根字段，`kind` 固定为 `behavior` |
| 创建方 | OpenDAN Behavior |
| 更新方 | OpenDAN Behavior |
| 消费方 | OpenDAN、TaskMgr UI |

OpenDAN Behavior 创建 LLM 行为任务时写入：

```json
{
  "trace_id": "trace-...",
  "agent_did": "agent-or-process-name",
  "behavior": "behavior-name",
  "step_idx": 0,
  "wakeup_id": "wakeup-...",
  "kind": "behavior",
  "session_id": "session-...",
  "owner_session_id": "session-...",
  "rootid": "agent-session-root"
}
```

该 schema 用于把一次 agent behavior loop 的推理步骤挂到 TaskMgr 可观察链路中。

### 5.6 Control Panel App Lifecycle

任务类型：

```text
app_install
app_uninstall
app_start
app_update
```

绑定关系：

| task_type | data 主字段 | 创建方 | 更新方 | 消费方 |
| --- | --- | --- | --- | --- |
| `app_install` | `app_id`、`user_id`、`version`、`content_id` | Control Panel app installer | Control Panel app installer | Control Panel、TaskMgr UI |
| `app_uninstall` | `app_id`、`user_id`、`remove_data` | Control Panel app installer | Control Panel app installer | Control Panel、TaskMgr UI |
| `app_start` | `app_id`、`user_id` | Control Panel app installer | Control Panel app installer | Control Panel、TaskMgr UI |
| `app_update` | `app_id`、`user_id`、`from_version`、`to_version`、`content_id` | Control Panel app installer | Control Panel app installer | Control Panel、TaskMgr UI |

Control Panel app installer 当前写入的 schema：

```json
{
  "app_id": "app-id",
  "user_id": "user-id",
  "version": "1.0.0",
  "from_version": "0.9.0",
  "to_version": "1.0.0",
  "content_id": "obj-or-content-id",
  "remove_data": false
}
```

字段按任务类型使用：

| task_type | 字段 |
| --- | --- |
| `app_install` | `app_id`、`user_id`、`version`、`content_id` |
| `app_uninstall` | `app_id`、`user_id`、`remove_data` |
| `app_start` | `app_id`、`user_id` |
| `app_update` | `app_id`、`user_id`、`from_version`、`to_version`、`content_id` |

安装和升级任务会把下载 app package 的 `download` 子任务挂在同一任务树下。

### 5.7 `scheduler.dispatch_thunk`

任务类型：

```text
scheduler.dispatch_thunk
```

绑定关系：

| 项 | 说明 |
| --- | --- |
| 绑定 task_type | `scheduler.dispatch_thunk` |
| data 主 namespace | 根字段 `thunk` / `function_object` / `node_id`，可选 `dispatch`、`executor`、`executor_result` |
| 创建方 | Scheduler / node executor dispatch path |
| 更新方 | node executor |
| 消费方 | node executor、scheduler、TaskMgr UI |

node executor 消费该类 task 时要求：

```json
{
  "node_id": "node-id",
  "runner": "optional-runner",
  "thunk_obj_id": "obj-...",
  "thunk": {},
  "function_object": {},
  "dispatch": {
    "node_id": "node-id",
    "runner": "optional-runner",
    "details": {
      "thunk_obj_id": "obj-..."
    }
  },
  "executor": {
    "status": "running",
    "task_id": 123,
    "work_dir": "/path/to/workdir",
    "result_path": "/path/to/executor_result.json"
  },
  "executor_result": {}
}
```

读取规则：

- `thunk` 必须存在，并能解析为 `ThunkObject`。
- `function_object` 必须存在，并能解析为 `FunctionObject`。
- `node_id` 可来自根字段 `node_id` 或 `dispatch.node_id`。
- `runner` 可来自根字段 `runner` 或 `dispatch.runner`。
- `thunk_obj_id` 可来自根字段 `thunk_obj_id` 或 `dispatch.details.thunk_obj_id`。

node executor 开始执行时写入 `executor.status = "running"`、`work_dir`、`result_path`；终态时写入 `executor.status = "finished"` 和 `executor_result`，并同步更新 Task 状态。

## 6. RPC 协议

TaskMgr 通过 kRPC 暴露在：

```text
POST /kapi/task-manager
```

RPC method 清单：

| Method | 请求结构 | 返回 | 说明 |
| --- | --- | --- | --- |
| `create_task` | `TaskManagerCreateTaskReq` | `CreateTaskResult` | 创建普通任务 |
| `create_download_task` | `TaskManagerCreateDownloadTaskReq` | `CreateDownloadTaskResult` | 创建或复用下载任务，并提交下载执行器 |
| `get_task` | `TaskManagerGetTaskReq` | `GetTaskResult` | 获取单个任务 |
| `list_tasks` | `TaskManagerListTasksReq` | `ListTasksResult` | 按字段过滤任务 |
| `list_tasks_by_time_range` | `TaskManagerListTasksByTimeRangeReq` | `ListTasksResult` | 按创建时间范围过滤任务 |
| `get_subtasks` | `TaskManagerGetSubtasksReq` | `ListTasksResult` | 获取直接子任务 |
| `update_task` | `TaskManagerUpdateTaskReq` | `()` | 原子快照更新 |
| `update_task_status` | `TaskManagerUpdateTaskStatusReq` | `()` | 更新状态 |
| `update_task_progress` | `TaskManagerUpdateTaskProgressReq` | `()` | 用 completed/total 更新进度 |
| `update_task_error` | `TaskManagerUpdateTaskErrorReq` | `()` | 标记失败并写错误消息 |
| `update_task_data` | `TaskManagerUpdateTaskDataReq` | `()` | 整体替换 data |
| `cancel_task` | `TaskManagerCancelTaskReq` | `()` | 取消单任务或整棵任务树 |
| `delete_task` | `TaskManagerDeleteTaskReq` | `()` | 删除任务 |

SDK 额外提供便捷方法：

| SDK 方法 | 说明 |
| --- | --- |
| `wait_for_task_end` | 以 500ms 默认间隔轮询直到进入终态 |
| `wait_for_task_end_with_interval` | 自定义轮询间隔 |
| `pause_task` | 设置 `Paused` |
| `resume_task` | 设置 `Running` |
| `complete_task` | 设置 `Completed` |
| `mark_task_as_waiting_for_approval` | 设置 `WaitingForApproval` |
| `mark_task_as_failed` | 调用 `update_task_error` 后设置 `Failed` |
| `pause_all_running_tasks` | 查找 Running 任务并逐个暂停 |
| `resume_last_paused_task` | 查找 Paused 任务并恢复最后一个 |

## 7. 任务变更事件

TaskMgr 在任务状态、错误、进度或数据变化后发布 kevent 事件。

订阅路径：

```text
/task_mgr/{task_id}
/task_mgr/{root_id}
```

语义：

- `/task_mgr/{task_id}`：订阅单个任务变化。
- `/task_mgr/{root_id}`：订阅整棵任务树变化；子孙任务变化会 fanout 到 root channel。
- 根任务的 `root_id == task_id` 时不会重复发布同一事件到两个相同 channel。
- `root_id` 中不能包含 `/`，且必须能通过 kevent event id 校验，否则不会发布 root fanout。

事件 payload 当前包含：

| 字段 | 说明 |
| --- | --- |
| `task_id` | 发生变化的任务 ID |
| `root_id` | 所属任务树根 ID |
| `parent_id` | 父任务 ID |
| `user_id` | 任务所属用户 |
| `app_id` | 任务所属应用 |
| `task_type` | 任务类型 |
| `from_status` | 变化前状态 |
| `to_status` | 变化后状态 |
| `progress` | 变化后进度 |
| `message` | 变化后消息 |
| `updated_at` | 更新时间 |
| `source_method` | 触发变化的服务端方法 |
| `change_kind` | `status`、`error`、`data`、`progress` |
| `data` | 小体积时内联完整 task data |
| `data_omitted` | data 过大时为 `true` |
| `data_size` | data 过大时记录字节数 |

为了适配 shared-ringbuffer slot 大小，事件中内联的 `data` 有大小限制：

```text
TASK_EVENT_DATA_INLINE_LIMIT_BYTES = 1300
```

如果 task data 超过限制，事件只携带 `data_omitted=true` 和 `data_size`。订阅方需要通过 `get_task(task_id)` 回拉完整 task。

事件限流：

- `status` 和 `error` 变化总是发布。
- `data` 和 `progress` 变化按 task id 以 1 秒为间隔限流。

## 8. 下载任务

当前实现内置 `download` task type，并提供 `create_download_task`。
> 从下载任务可以看出,task_mgr需要通过task_name字段的唯一性，提供“有的任务只能有一个在运行”的保障
> 下载任务的演示，也希望能说明 分布式 task-executor 模式

请求字段：

- `download_url`
- `objid`
- `download_options`
- `parent_id`
- `permissions`
- `root_id`
- `priority`
- `user_id`
- `app_id`

创建语义：

1. `download_url` 不能为空。
2. 如果 URL 能解析出 CYFS ObjId，服务会自动补全 `objid`。
3. 服务先在当前请求作用域内查找同 ObjId 或同 URL 的既有 download task。
4. 如果找到既有任务，会合并 URL / ObjId / download_options 等来源信息，并在需要时重新入队。
5. 如果不存在，按稳定名称创建新任务：
   - 有 ObjId：`download:objid:{objid}`
   - 无 ObjId：`download:url:{hash}`
6. 任务创建后提交到共享下载执行器。

下载任务初始 `data` 结构：

```json
{
  "download_url": "...",
  "urls": ["..."],
  "objid": "...",
  "download_options": {},
  "download": {
    "state": "pending",
    "mode": "named_store",
    "downloaded_bytes": 0
  }
}
```

`download.mode` 取值：

- `named_store`：有 ObjId，下载到 named store。
- `local_file`：无 ObjId，下载到本地文件。

下载执行器行为：

- 共享单例执行器，最大并发数当前为 `1024`。
- 同一 task id 不会重复入队。
- 开始执行时写入 `Running`、`progress=0`、`message="Download started"`。
- 执行中按字节进度写回 `progress` 和 `data.download.downloaded_bytes`。
- 成功后写入 `Completed`、`progress=100`、`message="Download completed"`。
- 失败后写入 `Failed` 和错误消息。
- 如果任务已经是终态或 `Paused`，执行器跳过。

## 9. 当前主要使用者

当前仓库中 TaskMgr 被多个模块作为基础能力使用：

- Control Panel app installer：创建安装任务、下载 app package、等待任务终态。
- AICC：把异步模型调用、流式输出、外部任务绑定到 TaskMgr 任务。
- Workflow service：把 Run / Step / Thunk / Map shard 映射为 TaskMgr 任务树，并订阅 `/task_mgr/{run_id}` 接收人类动作或执行结果变化。
- node_daemon：通过 `/kapi/task-manager` 获取任务，用于 function execution 相关路径。

对 Workflow 来说，TaskMgr 是运行期 UI 与状态总账。Workflow service 自身保留 DSL、拓扑、Reference、Amendment 等 workflow 语义；通用状态、进度、错误、等待人工处理等由 TaskMgr task tree 表达。

## 10. 设计原则

### 10.1 Task 是状态容器

TaskMgr 中的 Task 是长任务的状态快照，而不是执行体。业务执行方负责推进任务；TaskMgr 只提供可观察、可订阅、可权限控制的共享状态。

### 10.2 优先使用快照更新

新代码应优先使用 `update_task` 同时提交状态、进度、消息和 data patch，避免 UI 看到多个中间态。旧的 `update_task_status`、`update_task_progress`、`update_task_error`、`update_task_data` 仍保留用于兼容和简单调用。

### 10.3 TaskData 承载业务语义

TaskMgr 不理解每种业务任务的完整 schema。业务模块应把结构化信息写入 `data`，TaskMgr UI 或业务 UI 再按 `task_type` 和 `data` 渲染专门视图。未知 schema 至少可以展示基础字段和原始 JSON。

### 10.4 复杂工作使用任务树

长 workflow、多阶段安装、批量同步等复杂工作应使用父子任务组织成一棵树。外部入口通常展示根任务，通过 `get_subtasks` 或 `root_id` 向下钻取。

### 10.5 事件用于驱动观察，不替代持久状态

事件是通知机制，不是状态源。订阅方收到事件后可以直接使用 payload；如果 `data_omitted=true`，或者需要严格一致的完整状态，应调用 `get_task` 回拉。

## 11. 已知边界与后续改进

1. `priority` 已在创建参数中保留，但当前服务端没有调度语义。
2. `title`、`total_items`、`completed_items`、`error_message` 是数据库列，公开 `Task` 模型只暴露 `message`、`progress` 和 `data`；`update_task_progress` 会把 completed/total 同步写入 `data`。
3. 权限模型已经存在，但部分 handler 当前没有完整使用 RPC session token，仍依赖显式 source 字段或空上下文兼容路径。
4. `list_tasks_by_time_range` 当前先按 app/type 查库，再在内存中过滤时间范围；数据量变大后应下推到 SQL。
5. `delete_task` 依赖数据库外键级联删除子任务；当前不会发布删除事件。
6. `update_task_data` 是整体替换，和 `update_task` 的 merge patch 语义不同，新代码需要明确选择。
7. 事件只对变更发布；任务创建当前不发布 create event。
