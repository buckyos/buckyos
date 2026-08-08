# OpenDAN Agent Task Executor 设计（标准 Executor 模式）

- 状态：beta 2.2 目标设计；现有实现尚未全部满足本文要求
- 目标读者：OpenDAN Runtime、Task Dispatcher、TaskMgr、Workflow、TaskCenter 集成开发者
- 相关文档：
  - `doc/task_mgr/task_mgr.md`
  - `doc/task_mgr/task_dispatch_center.md`
  - `doc/task_mgr/task data schema.md`
  - `doc/opendan/OpenDAN Long Task & Sub-Agent.md`
  - `doc/opendan/Agent 协作.md`
- 共享协议：
  - `src/kernel/buckyos-api/src/task_dispatcher.rs`
  - `src/kernel/buckyos-api/src/taskdata.rs`

## 1. 文档定位

OpenDAN Agent Task Executor 是 TaskMgr 重构后的第一个标准 Executor 场景：外部调用者通过
Task Dispatcher 把一项结构化工作交给目标 Agent；OpenDAN 作为 Target 重新鉴权、创建自己
拥有的 Task，再由 Agent Executor 建立或恢复 WorkSession 执行。

本文定义 beta 2.2 的目标设计和实现收敛要求。由于 beta 2.2 是 breaking change，本文不保留旧
TaskMgr inbox、`runner` 路由或外部直接创建 `agent.delegate` Task 的兼容语义。

核心链路：

```text
Caller
  -> Task Dispatcher                         持久交接、Target 选择、投递状态
  -> OpenDAN Dispatch Target Adapter         目标侧鉴权、幂等接收
  -> OpenDAN-owned agent.delegate Task       可观察的业务执行总账
  -> Agent Task Executor                     执行协调、恢复、控制
  -> WorkSession                             Agent 执行现场
```

这也是其它跨 Service Executor 的参考模式：Dispatcher 管理接收之前的交接；Target Service
管理接收之后的业务 Task；TaskMgr 只保存长任务状态，不承担投递、调度或抢占。

## 2. TaskMgr 重构后的边界

### 2.1 冻结原则

1. **谁创建，谁执行。** OpenDAN 只执行 OpenDAN 自己创建的 Task。
2. **谁拥有，谁更新。** Task 的执行状态只能由 owner Service 或其受控业务接口更新。
3. **TaskMgr 是状态总账，不是工作队列。** 它不提供 inbox、runner、claim 或跨 owner 接管。
4. **跨 Service 的持久交接使用 Dispatcher。** 调用者不能用 `TaskMgr.create_task` 模拟投递。
5. **Target 必须重新做业务鉴权。** Dispatcher ACL 和 approval gate 不替代 Agent、Workspace、
   capability 与 constraints 校验。
6. **Dispatch 接受与业务执行分离。** 只有 Dispatcher 确认 `Accepted(target_task_id)` 后，
   Executor 才能启动目标 Task。
7. **Dispatcher 不镜像执行状态。** `Accepted` 是 Dispatch 正常终态，后续通过
   `target_task_id` 查询 TaskMgr。

### 2.2 组件职责

| 组件 | 负责 | 不负责 |
| --- | --- | --- |
| Caller / Workflow | 提交 operation、input、幂等键和业务身份 | 创建或修改 OpenDAN Task |
| Task Dispatcher | 持久交接、Target/Instance 选择、offer lease、重投、approval、accept/reject | 创建 WorkSession、执行 Agent 工作、镜像 Task 状态 |
| OpenDAN Target Adapter | 目标侧鉴权、schema 校验、幂等准备 Task、确认接收 | 执行 behavior loop |
| Agent Task Executor | 接受门禁、Task↔Session 绑定、执行恢复和业务控制 | 发现其它 owner 的工作 |
| WorkSession | Agent 上下文、Workspace、behavior loop 和结果生成 | 跨 Service 交付 |
| TaskMgr | Task 状态、进度、结果、任务树和观察事件 | 调度、投递、运行实例选举 |

### 2.3 三类入口

#### 人类 IM

IM 先进入 Session，由 Agent 理解、追问和规划。需要独立长任务时，OpenDAN 在自身信任域内
创建 Task 和 WorkSession。它不是跨 Service Dispatch。

#### OpenDAN 内部派生工作

同一 OpenDAN 运行时中的 Session、Agent behavior 或内部组件可以调用受控的内部创建入口。
OpenDAN 完成授权并创建自己的 Task。内部入口不能暴露成任意 Service 可调用的通用 executor。

#### 外部结构化委托

Workflow、其它 Agent 或系统 Service 把独立工作交给目标 Agent 时，必须调用 Dispatcher：

```text
dispatch(target_id, operation = "agent.delegate/v1", input)
```

外部调用者不能直接创建 `task_type = "agent.delegate"`，不能指定 Task `app_id`，也不能直接
操作 WorkSession。

## 3. Dispatch Target 与共享协议

本文直接采用 `buckyos-api` 中的共享类型，不再定义 OpenDAN 私有的近似协议。

### 3.1 operation

首个 operation 固定为：

```text
agent.delegate/v1
```

Wire input 使用 `AgentDelegateDispatchInput`：

```rust
pub struct AgentDelegateDispatchInput {
    pub title: Option<String>,
    pub purpose: String,
    pub input: Option<serde_json::Value>,
    pub owner_session_ref: Option<String>,
    pub context_refs: Vec<serde_json::Value>,
    pub workspace_hints: Vec<serde_json::Value>,
    pub constraints: Option<serde_json::Value>,
}
```

身份信息不能放在 input 中。以下字段只信任 Dispatcher 保存的不可变
`DispatchAuthEnvelope`：

- `requested_by_user` / `requested_by_app`
- `on_behalf_of`
- `zone_trusted_caller`
- `workflow_ref`
- `input_digest`
- `created_at` / `expires_at`

### 3.2 TargetRegistration

每个可接收委托的逻辑 Agent 注册一个 Target。Target 使用稳定 Agent DID 作为 `target_id`；
生产环境不得用可变 display name 充当稳定身份。

TargetRegistration 必须声明：

- `operations` 包含 `agent.delegate/v1` 及其 schema 引用。
- `auth_policy` 明确选择 `ZoneUsers` 或 `ZoneTrustedOnly`。
- `approval_policy` 明确选择 `Never`、`InteractiveCallers` 或 `AllCallers`。
- `idempotency_contract = IdempotentAccept`。
- `delivery_policy` 和 `max_concurrency` 与实际承载能力一致。
- `enabled` 反映 Agent 是否允许接受新委托。

`owner_user_id` 和 `owner_app_id` 由 Dispatcher 根据注册调用的已验证 token 写入，OpenDAN
不得在 payload 中伪造。`owner_app_id` 是实际 Agent AppService 的 app id，例如
`buckyos_jarvis`，不是固定字符串 `opendan`。

普通 Agent 委托可以配置 `approval_policy = Never`，前提是 Target 侧业务鉴权完整。若某类 Agent
或 constraints 代表敏感操作，应注册更严格的 approval policy 或独立 operation。approval 只
表示允许 Dispatcher 释放该记录，不产生额外业务权限。

### 3.3 TargetInstance

AgentRuntime attach 自己承载的 TargetInstance，并维护 lease、epoch 和 capacity：

- Target 离线时，DispatchRecord 留在 Dispatcher，不提前创建无人执行的 Task。
- 多实例承载同一 Target 时，Dispatcher 只向一个有效 instance 发放 offer lease。
- OpenDAN 仍需在自己的共享持久存储中处理并发接收，不能把 Dispatcher lease 当成唯一防重锁。
- 如果多个 TargetInstance 不共享 Target RDB，则该 Target 只能注册为单实例；不得依赖本机文件
  声称跨实例幂等。

### 3.4 Dispatch 状态

完整状态集合为：

```text
PendingApproval -> Queued -> WaitingForTarget -> Offered -> Accepted(target_task_id)
       |             |             |               |
       +-----------> Rejected / Expired / Canceled / Uncertain
```

实际迁移以 Dispatcher 状态机为准。需要冻结的语义是：

- `Accepted(target_task_id)` 是正常终态。
- `Rejected` 只用于稳定业务拒绝；临时无容量或依赖不可用使用 defer/等待。
- `Uncertain` 只适用于无法安全重投的 Target。Agent Target 声明
  `IdempotentAccept` 后必须能安全处理同一 `dispatch_id` 的任意重放。
- `Accepted` 后的 Task `Running`、`Paused`、`Completed`、`Failed` 等状态不写回 Dispatcher。

## 4. 两阶段鉴权

### 4.1 Dispatcher 层

Dispatcher 根据已验证调用上下文完成：

1. 调用者是否满足 Target `auth_policy`。
2. 普通调用者是否有权设置 `on_behalf_of`，禁止 payload 身份冒充。
3. 是否进入 `PendingApproval`。
4. operation、Target、过期时间和 caller idempotency key 的基本校验。

### 4.2 OpenDAN Target 层

OpenDAN 收到 offer 后必须重新校验：

1. `record.target_id` 是否等于本实例承载的稳定 Agent DID。
2. operation 是否精确等于支持的版本。
3. auth envelope 是否完整、未过期，`input_digest` 是否匹配持久 input。
4. `on_behalf_of` 是否拥有目标 Agent，或是否持有明确的 shared-agent/capability 授权。
5. `requested_by_app` 是否有权发起该业务 operation。
6. `context_refs`、`workspace_hints` 是否可由 `on_behalf_of` 读取。
7. `constraints` 是否在目标 Agent 的 policy 和运行资源范围内。
8. Agent 是否启用、是否允许接收该 operation。

默认安全规则是 `on_behalf_of == Agent owner`。共享 Agent、代理执行或跨用户委托必须有显式
capability/RBAC 规则，不能仅凭 `ZoneUsers` 或 `zone_trusted_caller` 放行。

拒绝原因使用共享 `DispatchRejectReason`。鉴权失败、schema 不支持、Target 禁用和业务前置条件
不满足属于稳定拒绝；TaskMgr/RDB 暂时不可用属于 defer，等待 offer lease 后重投。

## 5. 接收协议与执行门禁

### 5.1 目标侧状态

目标侧先完成接收协调，再进入独立的执行协调。两者都不是第二套业务 Task 状态：

```text
dispatch_binding:
  Prepared -> TaskCreated -> Accepted
      |            |
      +----------> Aborted

task_execution_binding（仅 Accepted 后存在）:
  Ready -> Routing / Running / WaitingForApproval / Paused -> Terminal
```

- `Prepared`：已通过目标侧鉴权并固化接收 intent，尚未确定 Task。
- `TaskCreated`：OpenDAN-owned Task 已存在，但 Dispatcher 尚未确认 Accepted；Task 必须保持惰性。
- `Accepted`：已从 Dispatcher 读到 `Accepted` 且 `target_task_id` 等于本地 Task。
- `Aborted`：Dispatcher 在接受前已取消/过期/拒绝，或发现绑定冲突；本地 Task 不得执行。
- `Ready` 及其后续 phase：Executor 的本地 checkpoint，外部业务状态仍以 TaskMgr 为准。

### 5.2 正常接收流程

```text
1. claim/offer DispatchRecord
2. 校验 Target、operation、auth、schema 和业务 policy
3. 在 Target RDB 中按 dispatch_id 插入或读取 Prepared intent
4. 若未绑定 task_id，使用 owner-scoped create key 幂等创建 OpenDAN-owned agent.delegate Task
5. 持久化 dispatch_id -> task_id，phase = TaskCreated
6. 调用 accept_dispatch(dispatch_id, target_task_id)
7. 根据 accept 成功响应确认 Accepted；响应丢失时调用 get_dispatch 读回，并确认
   DispatchRecord.target_task_id == 本地 task_id
8. phase = Accepted
9. 唤醒 Executor，建立或恢复 WorkSession
```

步骤 7 是执行门禁。不能在步骤 4 或 5 后把 Task 放入可执行队列；owner recovery 也不能把
`TaskCreated` 当成已接受任务。

### 5.3 accept ACK 丢失

`accept_dispatch` 网络错误不表示接受失败，也不表示接受成功。Adapter 必须调用
`get_dispatch(dispatch_id)` 读回：

- 状态为 `Accepted` 且 `target_task_id` 相同：持久化 `Accepted` 并启动 Executor。
- 状态仍为 `Offered`：保持 `TaskCreated`，由同一 offer 或重投再次 accept。
- 状态为 `Canceled` / `Expired` / `Rejected`：写 `Aborted` 并取消未执行的本地 Task。
- 状态为 `Accepted` 但 `target_task_id` 不同：这是不可自动覆盖的绑定冲突，记录严重错误并停止
  两个执行入口。
- Dispatcher 暂时不可达：保持惰性，恢复任务稍后继续对账，禁止启动 WorkSession。

### 5.4 重放与幂等契约

```text
同一 dispatch_id 重放 N 次
  -> 同一个 target_task_id
  -> 最多一个 OpenDAN Task
  -> 最多一个 WorkSession
```

`dispatch_id` 是 Target 侧接收幂等键；caller 的 `idempotency_key` 是 Dispatcher 侧创建记录的
幂等键，两者不能互相替代。

OpenDAN RDB 与 TaskMgr 不共享事务，因此接收是可恢复 saga，不宣称跨服务原子：

1. 先持久化 `Prepared` intent。
2. 用 `owner_create_key = "dispatch:" + dispatch_id` 幂等创建 Task。
3. 保存 `task_id`。
4. 确认 Dispatcher Accepted。

这要求 TaskMgr 的 `create_task` 支持可选的 owner-scoped 幂等创建键，并对
`(user_id, app_id, owner_create_key)` 建唯一约束。相同 key 和相同不可变创建参数重放时返回原
Task；相同 key 但参数摘要不同时返回冲突。该能力是通用 Executor 能力，不包含 Agent 路由语义。

当前 TaskMgr API 尚无这个创建键。没有它时，如果进程在步骤 2 和 3 之间崩溃，只能通过扫描
OpenDAN-owned Task 的 `request.dispatch_id` 对账；这可以恢复大多数情况，但无法在多实例和 lease
切换竞争下严格保证物理 Task 只创建一次。因此本文把 owner-scoped idempotent create 作为
`IdempotentAccept` Target 的实现前置，而不是把“扫描后去重”描述成原子保证。

### 5.5 取消边界

- `Accepted` 前：Dispatcher 可以取消；Target 将未执行本地 Task 标记为 `Canceled`，不启动
  WorkSession。
- `Accepted` 后：Dispatcher 已结束交接。业务取消必须调用 OpenDAN owner API。
- Dispatcher 不能直接修改目标 Task，TaskMgr 事件也不能作为控制命令。

## 6. Target 自有持久数据

### 6.1 存储选择

结构化幂等绑定和执行绑定必须保存到平台提供的 RDB 实例，不能使用
`dispatch_bindings.json`、目录扫描或仅进程内 HashMap 作为真相源。

逻辑数据库名由 OpenDAN 部署配置确定，本文称为 `agent-executor-main`。同一逻辑 Agent 的所有
TargetInstance 必须连接同一个 RDB。数据库必须有显式 `schema_version`，并通过正常的 OpenDAN
RDB 注册/初始化流程创建；不引入新的数据库依赖。

### 6.2 schema version

```sql
CREATE TABLE executor_meta (
    key   TEXT PRIMARY KEY,
    value TEXT NOT NULL
);

-- executor_meta('schema_version') = '1'
```

beta 2.2 不做旧 JSON 文件的兼容读写。实现切换后删除文件存储；开发环境若需要恢复已有
accepted task，只允许从 OpenDAN-owned Task 的 `request.dispatch_id` 一次性重建，不保留双写。

### 6.3 `dispatch_binding`

```sql
CREATE TABLE dispatch_binding (
    dispatch_id    TEXT PRIMARY KEY,
    target_id      TEXT NOT NULL,
    operation      TEXT NOT NULL,
    input_digest   TEXT NOT NULL,
    task_id        INTEGER UNIQUE,
    phase          TEXT NOT NULL,
    accepted_at_ms INTEGER,
    last_error     TEXT,
    created_at_ms  INTEGER NOT NULL,
    updated_at_ms  INTEGER NOT NULL,
    CHECK (phase IN ('Prepared', 'TaskCreated', 'Accepted', 'Aborted')),
    CHECK (phase NOT IN ('TaskCreated', 'Accepted') OR task_id IS NOT NULL)
);

CREATE INDEX idx_dispatch_binding_recovery
    ON dispatch_binding(target_id, phase, updated_at_ms);
```

冻结字段为 `dispatch_id`、`target_id`、`operation`、`input_digest` 和首次确定的 `task_id`。
重放时任何冻结字段不一致都属于协议冲突，不能覆盖原记录。所有 phase 迁移和绑定写入必须使用
事务内 compare-and-set；进程内 mutex 只能减少竞争，不能提供跨实例正确性。

### 6.4 `task_execution_binding`

```sql
CREATE TABLE task_execution_binding (
    task_id          INTEGER PRIMARY KEY,
    dispatch_id      TEXT UNIQUE,
    target_agent_id  TEXT NOT NULL,
    session_id       TEXT UNIQUE,
    phase            TEXT NOT NULL,
    lease_owner      TEXT,
    lease_epoch      INTEGER NOT NULL DEFAULT 0,
    lease_expires_ms INTEGER,
    last_error       TEXT,
    created_at_ms    INTEGER NOT NULL,
    updated_at_ms    INTEGER NOT NULL,
    CHECK (phase IN (
        'Ready', 'Routing', 'Running', 'WaitingForApproval',
        'Paused', 'Terminal'
    ))
);

CREATE INDEX idx_task_execution_recovery
    ON task_execution_binding(target_agent_id, phase, updated_at_ms);

CREATE INDEX idx_task_execution_lease
    ON task_execution_binding(lease_expires_ms);
```

该表保存本地执行协调和恢复 checkpoint，不替代 TaskMgr 的业务状态：

- TaskMgr 是外部可观察状态真相源。
- `phase` 只决定本 Executor 应恢复哪个本地执行现场。
- `lease_*` 只防止同一 Task 被多个本地/远端实例同时执行；过期 lease 可被恢复器接管。
- `session_id` 一旦非空即冻结，禁止把同一 Task 重新绑定到另一个 WorkSession。

### 6.5 持久与临时数据

| 数据 | 类型 | 原因 |
| --- | --- | --- |
| Dispatch intent、`dispatch_id -> task_id`、accept phase | 持久 | 重放和崩溃恢复的正确性依据 |
| `task_id -> session_id`、执行 checkpoint、lease epoch | 持久 | 保证一个 Task 只有一个执行现场 |
| Task 状态、progress、result、error | TaskMgr 持久 | 外部观察总账 |
| kevent、进程内 mutex、capacity 快照 | 临时 | 仅用于加速，丢失后可恢复 |

## 7. Task 所有权与 TaskData

### 7.1 所有权

OpenDAN 接受委托时创建：

```text
Task.task_type = "agent.delegate"
Task.user_id   = verified DispatchAuthEnvelope.on_behalf_of
Task.app_id    = 当前 Agent AppService 的实际 app_id
```

`Task.app_id` 可能是 `buckyos_jarvis`，不能硬编码为 `opendan`。调用者是 Dispatch requester，
不是 Task writer。即使某个 zone-trusted token 技术上能绕过普通 scope，架构上也必须通过 OpenDAN
业务 API 控制 Task，不能直接写 TaskMgr。

### 7.2 `AgentDelegateTaskData`

TaskData 使用 `buckyos-api::AgentDelegateTaskData`。关键 request 示例：

```json
{
  "request": {
    "version": 1,
    "source": "task-dispatcher",
    "dispatch_id": "dispatch-123",
    "target_agent_id": "did:agent:jarvis",
    "title": "Review changes",
    "purpose": "检查指定变更并输出结论",
    "requester_agent_id": null,
    "owner_session_id": "optional-origin-session",
    "input": {},
    "context_refs": [],
    "workspace_hints": [],
    "constraints": null,
    "reason_messages": []
  },
  "progress": {
    "execution": {
      "status": "prepared",
      "session_id": null,
      "workspace_id": null
    },
    "one_line_status": "Waiting for dispatch acceptance"
  },
  "result": null,
  "route": null,
  "blocker": null,
  "human_input": null,
  "error": null
}
```

规则：

- `request.*` 由 OpenDAN 根据已验证 DispatchRecord 构造，创建后不可变。
- `request.dispatch_id` 是跨 owner 的稳定审计链接。
- `request.target_agent_id` 是目标 Agent 的唯一规范字段。
- 旧 `progress.execution.runner` 不能再用于路由或 owner 判定；如保留，只能是可观察信息。
- `progress`、`route`、`blocker`、`human_input`、`result`、`error` 由 OpenDAN 更新。
- 内部创建的任务可以没有 `dispatch_id`，但仍必须设置 `target_agent_id`。

`doc/task_mgr/task data schema.md` 必须与共享 Rust 类型同步，不能保留缺少
`dispatch_id`、`target_agent_id`、`context_refs` 或 `constraints` 的旧 schema。

## 8. Executor 与 WorkSession

### 8.1 Executor 允许的输入

Executor 只接受两类 `task_id`：

1. OpenDAN 内部受控入口创建的 Task；没有 `dispatch_id`，创建完成后可进入 `Ready`。
2. Dispatch Adapter 创建且已通过 Accepted 门禁的 Task；RDB `dispatch_binding.phase` 必须为
   `Accepted`，且 Dispatcher 中的 `target_task_id` 与 Task 一致。

任何仅出现在 TaskMgr owner scan、但不满足上述条件的 Task 都不能启动。

### 8.2 执行前校验

Executor 每次取得执行 lease 后重新读取 Task 并校验：

- `task_type == "agent.delegate"`。
- `task.app_id == 当前 Agent AppService app_id`。
- `request.target_agent_id == 当前稳定 Agent DID`。
- Task 处于可恢复的非终态。
- 有 `dispatch_id` 时，接收绑定已确认 `Accepted`。
- 已有 `session_id` 时，只能恢复该 WorkSession。

### 8.3 Task ↔ WorkSession 1:1

```text
one task_id     -> zero or one session_id
one session_id  -> exactly one task_id
```

实现必须同时使用：

- RDB `UNIQUE(task_id)` / `UNIQUE(session_id)` 约束。
- 每 Task 的执行 lease 或 single-flight lock。
- 创建 Session 前后的可恢复 checkpoint。

可以使用由 `task_id` 派生的确定性 Session ID，也可以使用随机 ID 后在同一 RDB 事务中冻结绑定；
无论采用哪种方式，WorkSession 创建后、绑定写回前崩溃都必须通过 Session 索引找到原现场，不能
盲目创建第二个 Session。

### 8.4 Direct 与 task router

Direct 路径：

```text
validated AgentDelegateTaskData
  -> create_or_resume_worksession(task_id)
  -> behavior loop
```

task router 路径：

```text
ambiguous input / workspace selection needed
  -> route objective and workspace
  -> optional human.input child Task
  -> create_or_resume_worksession(task_id)
```

task router 只负责 OpenDAN 内部 objective/Workspace 路由，不承担跨 Service Dispatch。

### 8.5 Task 状态映射

| Task 状态 | Executor 语义 |
| --- | --- |
| `Pending` | 已创建但尚未运行；dispatch Task 可能仍处于接受门禁内 |
| `Running` | 正在路由或运行 WorkSession |
| `WaitingForApproval` | 等待人类输入或审批 |
| `Paused` | WorkSession 已暂停，可恢复原现场 |
| `Completed` | 成功终态 |
| `Failed` | 失败终态 |
| `Canceled` | 取消终态，禁止继续执行 |

内部自动重试期间 Task 保持 `Running` 并更新 progress；重试耗尽后进入 `Failed`。`Failed` Task
不重新投递，若业务需要重试，Caller 创建新的 DispatchRecord 和新的 Task。

## 9. Owner 控制 API

`Accepted` 后，控制动作必须通过 OpenDAN 的 task-id keyed KRPC：

```text
pause_agent_task(task_id)
resume_agent_task(task_id)
cancel_agent_task(task_id)
submit_agent_task_input(task_id, child_task_id, input)
```

这些 API 的要求：

1. 根据已验证 token 重新校验业务用户、Agent owner/capability 和 Task 归属。
2. 读取 `task_execution_binding` 定位唯一 WorkSession。
3. 先让 WorkSession 完成相应状态转换，再把结果写入 TaskMgr；失败时保留可恢复 checkpoint。
4. 重放同一控制请求应幂等。
5. `submit_agent_task_input` 必须完成对应 `human.input` 子任务并恢复原 WorkSession，不能让 UI
   直接修改 root Task 的 `human_action` 字段。

已有 `pause_session` / `resume_session` 只面向 Session，不等价于上述 Task owner API。TaskCenter
和 Web SDK 应只调用 OpenDAN 控制 API，不直接写 TaskMgr 模拟业务控制。

## 10. 事件、扫描与恢复

### 10.1 事件定位

- Dispatcher Target kevent 是“可能有 offer”的加速提示；Target 仍通过 `claim_next` 获取真相。
- TaskMgr kevent 是状态观察提示，不是 Executor inbox。
- 任何 kevent 丢失都不能影响最终恢复正确性。

### 10.2 OpenDAN 启动恢复

```text
1. 打开并迁移 Target RDB schema
2. 恢复 Prepared / TaskCreated 绑定，与 TaskMgr 和 Dispatcher 对账
3. 仅将确认 Accepted 的 dispatch Task 转为 Ready
4. 查询当前 app_id + target_agent_id 的自有非终态 Task
5. 恢复已有 task_execution_binding 和 WorkSession
6. 对内部任务补建 Ready 绑定
7. attach TargetInstance，开始 claim/offer 循环
```

owner scan 只允许查询当前 app id 的 Task，并按规范字段 `request.target_agent_id` 过滤。它是恢复
兜底，不是新工作发现机制。

### 10.3 扫描频率

- 启动时执行一次完整的 indexed owner recovery。
- 正常接收由 Adapter 直接唤醒 Executor。
- 可保留低频扫描作为进程内丢唤醒兜底，频率必须显著低于当前执行循环，不允许每秒级全局轮询。
- 多 Agent 共用 app id 时，应优先增加可索引的 owner/target 查询能力，不能长期依赖把全部 Task
  拉回进程再解析 JSON。

### 10.4 典型崩溃窗口

| 崩溃位置 | 恢复动作 |
| --- | --- |
| `Prepared` 后、Task 创建前 | 重放时继续创建 |
| Task 创建后、`task_id` 落库前 | 按自有 Task 的 `request.dispatch_id` 对账并补绑定 |
| `task_id` 落库后、accept 前 | 保持惰性并重试 accept |
| accept 成功、ACK 丢失 | `get_dispatch` 读回 Accepted |
| Accepted 后、Executor 唤醒前 | 启动恢复扫描从 RDB 唤醒 |
| Session 创建后、绑定落库前 | 用 task/session 索引找回原 Session |
| 执行中进程退出 | 过期 execution lease 后恢复原 WorkSession |

## 11. 端到端流程

### 11.1 正常外部委托

```text
1. Caller.dispatch(agent.delegate/v1, target_agent_did, input, idempotency_key)
2. Dispatcher 持久化 DispatchRecord
3. OpenDAN TargetInstance claim offer
4. Adapter 完成目标侧鉴权并持久化 Prepared intent
5. Adapter 创建 OpenDAN-owned Task，保存 task_id
6. Adapter accept_dispatch
7. Adapter 读回并确认 Accepted(target_task_id)
8. Executor 获取 execution lease
9. Executor 创建或恢复唯一 WorkSession
10. WorkSession 更新 Task progress/result/error
11. Caller 通过 target_task_id 观察 Task
```

### 11.2 Target 离线

```text
DispatchRecord -> WaitingForTarget
TargetInstance 上线 -> Offered
```

离线期间没有 OpenDAN Task。只有 Target 实际准备接收时才创建业务 Task。

### 11.3 人类介入

```text
WorkSession 需要输入
  -> OpenDAN 创建同 owner 的 human.input child Task
  -> child/root WaitingForApproval
  -> 用户调用 submit_agent_task_input
  -> OpenDAN 鉴权并完成 child
  -> root Running
  -> 恢复原 WorkSession
```

### 11.4 取消

```text
before Accepted: Caller -> Dispatcher.cancel_dispatch
after Accepted:  Caller -> OpenDAN.cancel_agent_task(target_task_id)
```

取消边界由 DispatchRecord 状态决定，不能同时向 Dispatcher 和 TaskMgr 盲写。

## 12. Workflow 与其它调用者集成

Workflow 的 Agent step 和定时任务 fire 必须使用 Dispatcher：

```text
Schedule fire / Workflow step
  -> Dispatcher.dispatch(agent.delegate/v1)
  -> Workflow-owned run/step mirror or link
  -> Accepted(target_task_id)
  -> OpenDAN-owned agent.delegate Task
```

Workflow 可以维护自己拥有的 run/step Task，并保存 `dispatch_id` 和 `target_task_id` 链接；它不能
在 TaskMgr 中直接创建一个期待 OpenDAN 执行的 `agent.delegate` child Task。

Agent Tool、dcrontab 模板和测试工具也遵守同一规则。旧
`execution.runner + TaskMgr.create_task(agent.delegate)` 路径应删除，不提供兼容分支。

## 13. 可观测性与审计

至少记录以下稳定关联：

```text
dispatch_id
  -> requested_by_user / requested_by_app / on_behalf_of / workflow_ref
  -> target_id / operation / input_digest
  -> target_task_id
  -> session_id
```

日志和指标至少覆盖：

- offer、defer、reject 和 accept 延迟。
- accept 读回次数、绑定冲突、重放命中。
- `Prepared` / `TaskCreated` 长时间滞留数量。
- execution lease 抢占、恢复和重复 Session 防护。
- Task 终态、执行耗时、人工介入次数。

不得在日志中输出完整敏感 input、context 或人类输入；优先记录 `input_digest` 和稳定 ID。

## 14. 现有实现差距与修改要求

当前代码已经具备 Dispatcher Target 注册、attach/claim、创建自有 Task、accept callback 和
owner-only recovery，方向正确，但还不能作为本文定义的标准 Executor 实现。需要以下修改。

### 14.1 P0：接收正确性与权限

1. **补齐 Target 侧业务鉴权。**
   `dispatch_adapter.rs` 当前主要校验 operation、target、非空 `on_behalf_of` 和 input schema；还需
   校验 Agent owner/capability、requester app、context、Workspace 和 constraints。
2. **增加 Accepted 执行门禁。**
   当前 Task 创建后会被 owner 周期扫描发现，存在 accept 完成前启动的窗口。恢复扫描必须读取
   Target RDB phase，只有已确认 `Accepted` 的 dispatch Task 才可执行。
3. **处理 accept ACK 丢失。**
   SDK/Adapter 在 `accept_dispatch` 出错后必须 `get_dispatch` 对账，不能只记录错误等待未知结果。
4. **把 JSON binding 改为平台 RDB。**
   当前 `dispatch_bindings.json` 不能提供 schema、事务、损坏检测或多实例唯一性，必须替换为
   §6 的版本化 RDB。
5. **给 TaskMgr 增加 owner-scoped idempotent create。**
   当前 `create_task` 没有创建幂等键，无法关闭“Task 已创建、Target 绑定尚未落库”的跨服务
   崩溃窗口。应增加可选 `owner_create_key`、唯一约束和参数冲突检查；不增加任何 runner 或
   Dispatch 业务字段。
6. **固化 Task↔WorkSession 唯一绑定。**
   增加 `task_execution_binding` 和 execution lease，消除并发恢复或创建 Session 的竞争窗口。

### 14.2 P1：Owner API 与调用链

1. 在 OpenDAN KRPC 和 `opendan_client.rs` 增加 task-id keyed
   pause/resume/cancel/submit-input API，并完成二次鉴权。
2. TaskCenter/Web SDK 改为调用 owner API，删除直接写 TaskData `human_action` 的控制方式。
3. OpenDAN 内部创建者统一写 `request.target_agent_id`，删除以
   `progress.execution.runner` 为目标身份的 fallback。
4. Workflow scheduled Agent fire 改为 Dispatcher；Workflow 保留自己拥有的 run/step Task 与链接。
5. Agent Tool、dcrontab 模板和相关测试删除直接 `TaskMgr.create_task(agent.delegate)` 的旧路径。

### 14.3 P2：恢复效率与文档同步

1. owner recovery 从固定短周期轮询改为直接唤醒 + 启动扫描 + 低频 indexed backstop。
2. 若多 Agent 共用 app id，补充按 target Agent 的可索引恢复数据，避免反序列化全部 TaskData。
3. 同步 `doc/task_mgr/task data schema.md`、共享 API 文档和 TaskCenter 交互文档。

## 15. 验证与验收

### 15.1 单元测试

1. auth matrix：owner、shared capability、跨用户、不同 requester app、非法 context/workspace。
2. 同一 `dispatch_id` 顺序重放和并发重放始终得到同一 `task_id`。
3. 相同 owner create key 重放返回同一 Task，参数不一致时明确报冲突。
4. 两个 TargetInstance 并发接收时最多创建一个 Task。
5. 在 §10.4 每个崩溃点重启，最终只得到一个 Task 和一个 WorkSession。
6. accept ACK 丢失、Dispatcher 取消竞争和绑定冲突不会提前执行。
7. owner scan 看见 `TaskCreated` 但未 Accepted 的 Task 时不会启动。
8. 内部无 `dispatch_id` Task 可以正常执行，但必须有 `target_agent_id`。
9. pause/resume/cancel/submit-input 重放幂等并正确驱动同一 WorkSession。

### 15.2 集成 / DV Test

1. Target 离线时 Dispatch 持久等待，上线后接收。
2. Dispatcher、OpenDAN、TaskMgr 分别在关键窗口重启后可恢复。
3. kevent 丢失时 claim 和 owner recovery 仍能收敛。
4. Workflow 普通 step 与 scheduled fire 都经 Dispatcher 创建 OpenDAN-owned Task。
5. `Accepted` 后 Task 状态只在 TaskMgr 变化，Dispatcher 保持终态链接。
6. Task 失败不触发旧 Dispatch 再投；新重试产生新的 Dispatch 和 Task。
7. TaskCenter 的控制与人工输入全部经过 OpenDAN owner API。

### 15.3 完成标准

本文设计全部落地需同时满足：

- 不存在 TaskMgr inbox、runner 路由或跨 owner 扫描接管。
- 相同 Dispatch 最多一个目标 Task，相同 Task 最多一个 WorkSession。
- 未确认 Accepted 的 Task 永不执行。
- Target 侧权限检查不依赖 Dispatcher ACL 代替。
- 所有业务控制通过 owner API。
- Workflow、Agent Tool 和定时入口不再直接创建 OpenDAN 执行 Task。
- TaskMgr owner-scoped create key、Target RDB、共享 TaskData schema、TaskCenter 和文档保持一致。

## 16. 设计摘要

1. Agent Task Executor 是 TaskMgr 重构后的标准 Dispatch Target + owner Executor 模式。
2. Dispatcher 管接收前，OpenDAN 管接收后，TaskMgr 只做状态总账。
3. Target 先二次鉴权并幂等创建自有 Task，再确认 Accepted，最后启动执行。
4. `dispatch_id -> task_id -> session_id` 必须由 TaskMgr owner create key、版本化 Target RDB 和
   唯一约束共同持久保证。
5. `request.target_agent_id` 是唯一目标字段，旧 runner 路由不兼容保留。
6. owner recovery 只恢复已接受的自有 Task，不能替代 Dispatcher。
7. pause/resume/cancel/input 通过 OpenDAN owner API，不由调用者直接修改 TaskMgr。
8. Workflow 和定时任务必须经 Dispatcher，不能继续把 TaskMgr 当成 Agent 队列。
