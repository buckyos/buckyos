# OpenDAN Agent Task Executor 设计（Dispatch Target 模式）

- 状态：Draft（按 beta 2.2 TaskMgr 新边界重写）
- 目标读者：OpenDAN Runtime / Agent Session / Workflow / Dispatcher / TaskMgr 集成开发者
- 相关文档：
  - `doc/task_mgr/task_mgr.md`
  - `notepads/task-dispatch-center-todo.md`
  - `doc/opendan/OpenDAN Long Task & Sub-Agent.md`
  - `doc/opendan/Agent 协作.md`

## 1. 背景与版本边界

OpenDAN 已具备 message/event dispatch、Session、Workspace 和 Agent behavior loop，也需要把
长时间运行的 Agent 工作映射为 Task，供 TaskCenter 展示状态、进度、结果和人工介入点。

旧设计把 AgentRuntime 建模为 TaskMgr 的通用消费者：外部调用者先创建
`agent.delegate` Task，OpenDAN 再通过 TaskMgr 事件、Pending 查询和 runner 过滤发现并执行。
这与 beta 2.2 的 TaskMgr 边界冲突：TaskMgr 是长任务状态总账，不是跨 Service 工作队列，
也不负责把低权限调用者创建的 Task 交给高权限 Service 执行。

但 OpenDAN 与普通 TaskMgr 用户不同。Agent 的核心职责就是接受外部委托，而且天然存在：

- Target Agent 暂时离线，请求仍需持久等待。
- 多个 Agent / AgentRuntime 实例之间的目标选择和容量管理。
- 投递重放、接收确认丢失和幂等创建 WorkSession。
- 委托来源、on-behalf-of 身份和目标 Agent capability 的审计。

因此 OpenDAN Agent Task Executor 是一个**真正的 Dispatch Target**，不是“普通业务 RPC +
内部 TaskMgr”模式的典型用户。

版本边界：

- beta 2.2 只完成 TaskMgr 内核边界收敛，不实现 Workflow / Task Dispatch Center。
- beta 2.2 禁止继续用 TaskMgr runner、全局事件和 Pending 扫描模拟 Agent Dispatch。
- beta 2.2 中，OpenDAN 仅执行 IM/Session 路径和 OpenDAN 内部创建的 WorkSession Task。
- 下一版本由 Task Dispatch Center 向 OpenDAN / Agent 投递 `agent.delegate`。
- OpenDAN 接受 Dispatch 后，仍由 OpenDAN 创建、执行和恢复自己拥有的 Task。

## 2. 需要冻结的结论

### 2.1 当前版本

```text
Human IM / OpenDAN internal action
  -> OpenDAN 理解并接受工作
  -> OpenDAN 创建自己的 agent.delegate Task
  -> OpenDAN 创建并驱动 WorkSession
  -> TaskMgr 保存状态、进度、结果和任务树
```

- TaskMgr 没有 Agent inbox、runner、claim 或工作投递语义。
- 外部调用者直接 `TaskMgr.create_task(task_type = "agent.delegate")` 不会触发 OpenDAN。
- OpenDAN 可以在启动时恢复自己创建的非终态 Task，但不能扫描和接管其他 owner 的 Task。
- 当前版本不为了临时可用而新增一个对外 `delegate_agent_task` RPC 绕过未来 Dispatcher。

### 2.2 下一版本

```text
Workflow / Agent / Service
  -> Task Dispatch Center.dispatch(target_agent, "agent.delegate", input)
  -> OpenDAN TargetInstance claim / receive offer
  -> OpenDAN 重新鉴权并幂等创建自己的 Task
  -> OpenDAN accept(dispatch_id, target_task_id)
  -> OpenDAN WorkSession 执行业务
```

- Dispatcher 管理“OpenDAN 接受之前”的持久交接。
- OpenDAN 管理“接受之后”的 Task 和 WorkSession 生命周期。
- Dispatcher 不替 OpenDAN 创建业务 Task，也不复制 Task 执行状态机。
- `Accepted(target_task_id)` 是 Dispatch 的正常终态；后续状态通过 Task link 观察。

### 2.3 组件职责

```text
IM                 = intent acquisition
Workflow           = execution graph
Task Dispatcher    = durable work handoff
OpenDAN             = target ownership, authorization and execution
WorkSession         = execution context
TaskMgr             = observable task state
```

## 3. 非目标

本文不试图：

1. 把 TaskMgr 扩展成 Agent 工作队列或泛聊天协作中心。
2. 让 OpenDAN 扫描所有 `agent.delegate` Task，再从 data 中挑选目标 Agent。
3. 用一个临时 OpenDAN RPC 重新实现半套 Dispatcher。
4. 让 Dispatcher 代替 OpenDAN 做 Agent、Workspace 或 capability 鉴权。
5. 让调用者指定 Task owner、`app_id`、执行身份或高权限运行选项。
6. 用 Group message 作为任务状态或接收确认协议。

## 4. 三类工作入口

### 4.1 人类 IM

IM 是自然语言意图输入，可能只是讨论、探索或追问，不一定立即形成 Task：

```text
Human IM
  -> UI Session / message router
  -> Agent 理解、追问和规划
  -> 需要独立执行时，由 OpenDAN 内部创建 Task + WorkSession
```

这不是跨 Service Dispatch，不经过 Task Dispatch Center。

### 4.2 OpenDAN 内部派生工作

同一 OpenDAN 信任域内，UI Session、WorkSession 或 Agent behavior 可以调用内部
`create_worksession` / delegate 能力。OpenDAN 自己完成授权、创建 Task 和绑定 Session。

内部派生工作不能被包装成一个允许任意外部 Service 调用的通用 Task 投递入口。

### 4.3 外部结构化委托

Workflow、其它 Agent 或系统 Service 把一项独立工作交给目标 Agent，属于真正的 Dispatch：

```text
External caller
  -> Dispatcher
  -> target Agent / OpenDAN TargetInstance
  -> OpenDAN-owned Task
```

外部调用者提交 operation 和业务 input，不创建 OpenDAN Task，不指定 Task owner，也不直接
控制 WorkSession。

## 5. Dispatch Target 模型

### 5.1 TargetRegistration

建议每个可接收委托的逻辑 Agent 注册为一个 Target，而不是把整个 OpenDAN 注册成一个无法
区分目标的通用 runner：

```rust
pub struct AgentTargetRegistration {
    pub target_id: String,          // Agent DID 或稳定 Agent ID
    pub owner_app_id: String,       // opendan
    pub owner_did: String,
    pub operations: Vec<String>,    // 至少包含 agent.delegate
    pub capability_refs: Vec<String>,
    pub max_concurrency: u32,
    pub auth_policy: String,
}
```

约束：

- `target_id` 必须来自可信 Agent 配置 / DID Document，不能由运行实例任意声明。
- `owner_app_id` 固定为 OpenDAN Service 身份。
- operation 使用版本化 schema，例如 `agent.delegate/v1`。
- capability 是权限与路由约束，不是仅供 LLM 参考的自由文本标签。
- 禁止注册一个接受任意 `task_type + data` 的通用 Agent Target。

### 5.2 TargetInstance

AgentRuntime 启动后，代表其承载的 Agent attach 对应 Target：

```rust
pub struct AgentTargetInstance {
    pub target_id: String,
    pub instance_id: String,
    pub lease_epoch: u64,
    pub capacity: u32,
}
```

- TargetInstance renew lease，并上报可用 capacity。
- Target 离线时 DispatchRecord 保留在 Dispatcher，不创建无人执行的 TaskMgr Task。
- 多实例承载同一 Agent 时，由 Dispatcher 的 claim/lease 选定一个实例。
- OpenDAN 内部 WorkSession 数量和 Workspace 约束仍由 OpenDAN 最终校验。

### 5.3 operation

首个 operation：

```text
agent.delegate/v1
```

逻辑 input：

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

以下信息来自 Dispatcher 的不可变 auth envelope，不由 input 提供：

- requested-by user/app/agent。
- on-behalf-of 身份。
- Workflow / Run / Step 标识。
- `dispatch_id`、schema version、input digest 和过期时间。

## 6. Dispatch 接收协议

### 6.1 接收流程

OpenDAN Dispatch Adapter 收到 offer 后：

1. 验证 TargetRegistration、instance lease/epoch 和 operation version。
2. 验证 auth envelope、调用者 capability 和 on-behalf-of 身份。
3. 校验 target Agent 当前是否允许接受该类工作。
4. 校验 `purpose`、Workspace hint、context reference 和 constraints。
5. 查询本地 `dispatch_id -> task_id` 幂等绑定。
6. 如不存在，原子创建 OpenDAN-owned `agent.delegate` Task 并保存绑定。
7. 将 `task_id` 放入 OpenDAN 内部执行队列。
8. 返回 `accept(dispatch_id, target_task_id)`。

只有第 6 步完成后才能返回 Accepted。

### 6.2 幂等契约

```text
相同 dispatch_id 重放 N 次
  -> 返回相同 target_task_id
  -> 最多创建一个 OpenDAN Task
  -> 最多绑定一个 WorkSession
```

OpenDAN 必须在自己的持久存储中原子保存 `dispatch_id -> task_id`。仅把 `dispatch_id` 写进
Task.data 后再扫描去重不足以防止并发重复创建。

如果 Task 已创建但 accept ACK 丢失，重新投递返回原 `task_id`。无法判断是否创建成功时，
Dispatch 进入 `Uncertain`，禁止盲目创建第二个 WorkSession。

### 6.3 拒绝

稳定拒绝原因包括：

- auth / capability 不满足。
- target Agent 不存在、已禁用或不接受该 operation。
- schema version 不支持。
- input / context / Workspace hint 不合法。
- constraints 超出 Agent policy。

容量暂时不足或 Target 离线不是业务拒绝，应由 Dispatcher 保持等待或重新 offer。

### 6.4 取消边界

- `Accepted` 之前：Dispatcher 可以取消 DispatchRecord。
- `Accepted` 之后：业务 Task 已归 OpenDAN 所有，取消必须通过 OpenDAN 的受权控制 operation。
- Dispatcher 不直接修改目标 Task 状态。

## 7. Task 所有权与 TaskData

### 7.1 所有权

OpenDAN 接受 Dispatch 后创建 Task：

- `task_type = "agent.delegate"`。
- `app_id = opendan`，表示 OpenDAN 是 Task owner 和主要更新方。
- `user_id` 来自验证后的 on-behalf-of 业务用户。
- 调用者按授权获得只读观察能力，不能直接修改执行状态。
- pause/resume/cancel/input 等动作通过 OpenDAN 控制接口完成。

```text
Dispatch requester != Task owner

Task.user_id = verified business user
Task.app_id  = opendan
```

任何调用者直接在 TaskMgr 中创建相同 data，只会得到属于调用者自己的普通 Task；OpenDAN
不发现、不接管、不执行它。

### 7.2 `agent.delegate` TaskData

建议沿用 `AgentDelegateTaskData` 的 `request / progress / result / error` 分层，并增加稳定的
Dispatch 引用：

```json
{
  "request": {
    "version": 1,
    "source": "task-dispatcher",
    "dispatch_id": "dispatch-123",
    "target_agent_id": "did:agent:jarvis",
    "title": "Review changes",
    "purpose": "检查指定变更并输出结论",
    "requester_agent_id": "did:agent:planner",
    "owner_session_id": "optional-origin-session",
    "input": {},
    "workspace_hints": [],
    "reason_messages": []
  },
  "progress": {
    "execution": {
      "session_id": null,
      "workspace_id": null,
      "status": "accepted"
    },
    "one_line_status": "Accepted by OpenDAN"
  },
  "result": null,
  "route": null,
  "blocker": null,
  "human_input": null,
  "error": null
}
```

旧 `progress.execution.runner` 不再承担目标 Agent 归属。目标身份应使用明确的
`request.target_agent_id`；两者都不是 TaskMgr runner，TaskMgr 也不提供对应查询或投递语义。

字段写入方：

| 字段 | 来源 / 更新方 |
| --- | --- |
| `request.*` | OpenDAN 根据已验证 DispatchRecord 构造，创建后不可变 |
| `progress.*` | OpenDAN Executor / WorkSession |
| `route` | OpenDAN task router |
| `blocker` / `human_input` | OpenDAN Executor |
| `result` / `error` | OpenDAN WorkSession / Executor |

## 8. Runtime 组件

### 8.1 Dispatch Target Adapter（下一版本）

职责：

- 注册/attach Agent Target 与 TargetInstance。
- 接收 offer 或执行 `claim_next`。
- 验证 auth envelope 和 operation schema。
- 幂等创建 OpenDAN Task。
- 返回 accept/reject。

Adapter 只负责接收边界，不执行 Agent 工作。

### 8.2 Owner Task Recovery

OpenDAN 接受 Dispatch 后，可以恢复自己创建的 Task：

```text
TaskFilter {
  app_id: "opendan",
  task_type: "agent.delegate",
  status: non_terminal
}
```

恢复规则：

- 进程启动时扫描一次 OpenDAN-owned 非终态 Task。
- 正常路径由 Dispatch Adapter 或内部 Session 路径直接唤醒 Executor。
- 活跃 Task 可订阅 `/task_mgr/{task_id}` 作为状态变化提示。
- 可保留低频 owner-only scan 作为进程内丢唤醒兜底。
- 禁止列出所有 `agent.delegate` Task 后按 data 中 target Agent 本地挑选。

这是 owner 内部恢复，不是 Dispatch。接收前的等待记录仍只存在于 Dispatcher。

### 8.3 Agent Task Executor

Executor 只处理由以下两个可信入口提供的 OpenDAN-owned `task_id`：

- OpenDAN 内部 Session/Agent 创建路径。
- Dispatch Target Adapter 已接受的记录。

职责：

1. 重新读取 Task，验证 `app_id == opendan`、`task_type == agent.delegate`。
2. 校验 `request.target_agent_id` 与本 AgentRuntime 匹配。
3. 已绑定 `session_id` 时恢复原 WorkSession，不创建新 Session。
4. 首次执行时选择 direct 或 task router 路径。
5. 创建 WorkSession，持久化 `session_id`、Workspace 和执行状态。
6. 将进度、结果和错误写回 Task。
7. 响应 OpenDAN 控制接口触发的 pause/resume/cancel/input。

### 8.4 WorkSession 创建路径

Direct：

```text
Known AgentDelegateTaskData
  -> create_worksession(task_id=<opendan-owned-task-id>)
  -> WorkSession 绑定原 Task
```

Task router：

```text
Unstructured / ambiguous input
  -> task_route Session 综合 objective 并选择 Workspace
  -> 必要时创建 human.input 子任务
  -> create_worksession(task_id=<opendan-owned-task-id>)
```

task router 只负责 OpenDAN 内部 objective/Workspace 路由，不负责跨 Service Dispatch。

### 8.5 WorkSession ↔ Task 1:1

- 每个 `agent.delegate` Task 最多绑定一个业务 WorkSession。
- `progress.execution.session_id` 是已绑定标记。
- `Paused`、`WaitingForApproval` 和进程重启只恢复原绑定。
- WorkSession 创建后、绑定写回前崩溃时，必须通过本地 Session 索引恢复，不能盲目创建第二个 Session。

## 9. 执行流程

### 9.1 下一版本正常 Dispatch

```text
1. Caller 创建 DispatchRecord(target_agent, agent.delegate/v1, input)
2. Target 离线时 Dispatcher -> WaitingForTarget
3. OpenDAN TargetInstance 上线/有 capacity
4. Dispatcher offer，OpenDAN Adapter 验证 auth/schema/policy
5. OpenDAN 幂等创建 app_id=opendan 的 agent.delegate Task
6. OpenDAN 保存 dispatch_id -> task_id
7. OpenDAN accept(dispatch_id, task_id)
8. Executor 创建或恢复 WorkSession
9. WorkSession 执行并写回 Task 状态
10. Caller 通过 target_task_id 观察结果
```

### 9.2 OpenDAN 重启恢复

```text
1. OpenDAN 启动
2. 恢复本地 dispatch_id -> task_id 绑定
3. owner scan 查询 app_id=opendan + agent.delegate + non_terminal
4. 已绑定 session_id -> 恢复原 WorkSession
5. 未绑定 -> 继续首次创建路径
6. TargetInstance attach Dispatcher，重新处理未确认 offer
```

### 9.3 人类介入

```text
1. task_route / WorkSession 需要信息或审批
2. OpenDAN 创建同 owner 的 human.input 子任务
3. child/root -> WaitingForApproval
4. 用户调用 OpenDAN submit_agent_task_input
5. OpenDAN 鉴权并写入 response
6. child -> Completed，root -> Running
7. 恢复原 WorkSession
```

### 9.4 暂停、恢复和取消

`Accepted` 后由 OpenDAN 控制接口管理业务 Task：

```text
pause_agent_task(task_id)  -> pause WorkSession  -> Task.Paused
resume_agent_task(task_id) -> resume WorkSession -> Task.Running
cancel_agent_task(task_id) -> stop WorkSession   -> Task.Canceled
```

调用者不能先直接更新 TaskMgr 状态，再等待 OpenDAN 通过全局轮询发现控制动作。

## 10. 状态与可观测性

### 10.1 Dispatch 状态

```text
Queued -> WaitingForTarget -> Offered -> Accepted(target_task_id)
                                  \-> Rejected | Uncertain
```

`Accepted` 后 Dispatcher 不复制 Task 状态；UI 通过 link 查询 OpenDAN Task。

### 10.2 Task 状态

| Task 状态 | OpenDAN 语义 |
| --- | --- |
| `Pending` | OpenDAN 已接受，尚未建立执行现场 |
| `Running` | 正在路由或执行 |
| `WaitingForApproval` | 等待人类输入或审批 |
| `Paused` | WorkSession 已暂停 |
| `Completed` | WorkSession 成功结束 |
| `Failed` | 执行失败 |
| `Canceled` | OpenDAN 已停止继续执行 |

TaskMgr kevent 是观察和恢复提示，不是外部工作投递控制面。

## 11. 与 MsgCenter / Group 的关系

适合使用 message / group：

- 人类给 Agent 输入自然语言意图。
- Agent 向人类解释进展或请求继续沟通。
- 外部 Agent / Human 的 A2A 协作。
- 将 TaskMgr 状态投影到 Group 做共享报告。

不适合使用 message / group：

- 替代结构化 `agent.delegate` Dispatch。
- 判断 Task 是否完成。
- 表达接收确认、重试、取消、恢复和权限。
- 让多个 Agent 通过群聊抢 Task。

## 12. 分版本实施范围

### 12.1 beta 2.2：只收敛旧 TaskMgr inbox

1. 删除 `/task_mgr/**` 加 `list_tasks(task_type=agent.delegate)` 的跨 owner 接收路径。
2. 删除按 `progress.execution.runner` 本地过滤后接管外部 Task 的逻辑。
3. 外部直接创建 `agent.delegate` Task 不触发 OpenDAN。
4. OpenDAN 内部创建的 WorkSession Task 继续由 OpenDAN 创建、执行和 owner-only recovery。
5. 保留 direct/task router、WorkSession 绑定、状态同步和 `human.input` 内部能力。
6. 删除或禁用 `worksession-task-test` 直接向 TaskMgr 投递 Agent 工作的测试方式。
7. 不新增临时 `delegate_agent_task` 对外 RPC，不实现 Dispatcher Target 注册。

### 12.2 下一版本：接入 Task Dispatch Center

1. 在 `buckyos-api` 增加共享 Dispatch 协议和 `agent.delegate/v1` input 类型。
2. 为 Agent 建立可信 TargetRegistration 和 capability policy。
3. OpenDAN 实现 TargetInstance attach/renew/capacity。
4. 实现 offer/claim/accept/reject 和 `dispatch_id -> task_id` 幂等存储。
5. `AgentDelegateTaskData` 增加 `dispatch_id`、`target_agent_id` 和审计引用。
6. OpenDAN 接受后创建自己的 Task，并把 `target_task_id` 返回 Dispatcher。
7. 增加 pause/resume/cancel/input 等 OpenDAN 业务控制接口。
8. 验证 Target 离线、OpenDAN 重启、offer 超时、ACK 丢失和重复投递。

### 12.3 主要影响入口

当前版本：

- `src/frame/opendan/src/agent_task_executor.rs`
- `src/frame/opendan/src/agent.rs`
- `src/frame/opendan/src/main.rs`
- OpenDAN / TaskMgr 相关测试

下一版本：

- `src/kernel/buckyos-api/src/opendan_client.rs`
- `src/kernel/buckyos-api/src/taskdata.rs`
- Dispatcher 共享协议与客户端
- OpenDAN Dispatch Target Adapter 和持久幂等存储
- Workflow Agent executor adapter

## 13. 验收条件

### 13.1 beta 2.2

1. 调用者直接创建 `agent.delegate` Task 不会触发 OpenDAN 执行。
2. OpenDAN 不订阅全局 TaskMgr 事件来发现 Agent 工作。
3. OpenDAN 不扫描或接管其它 owner 的 Task。
4. OpenDAN 内部 WorkSession Task 仍由 OpenDAN 创建并可在重启后恢复。
5. Dispatcher 完全不存在时，当前 IM/Session 能力正常工作。

### 13.2 下一版本

1. 每个可委托 Agent 有可信 TargetRegistration 和明确 operation/capability。
2. Target 离线时 Dispatch 持久等待，上线后能接收。
3. OpenDAN 重新鉴权后才创建 Task。
4. 创建出的 Task `app_id = opendan`，业务用户来自可验证 auth envelope。
5. 相同 `dispatch_id` 始终返回同一个 `target_task_id`。
6. 同一 `target_task_id` 最多绑定一个 WorkSession。
7. Dispatcher 在 `Accepted` 后不复制目标 Task 状态机。
8. pause/resume/cancel/input 经过 OpenDAN 控制接口并正确影响 WorkSession。

## 14. 设计原则摘要

1. OpenDAN Agent Task Executor 是真正的 Dispatch Target。
2. TaskMgr 不提供 Agent 工作投递能力。
3. beta 2.2 删除伪 Dispatch，但不提前实现下一版本 Dispatcher。
4. 外部委托由 Dispatcher 持久交接，OpenDAN 接受后创建自己的 Task。
5. 谁创建，谁执行；谁拥有，谁更新。
6. Dispatcher 管接收前，OpenDAN 管接收后。
7. WorkSession ↔ Task 保持 1:1，同一 Dispatch 不能重复创建执行现场。
8. owner-only recovery 只恢复已接受的 OpenDAN Task，不能替代 Dispatcher。
9. 人类 IM 是意图入口，不等于结构化 Dispatch。
10. Message / Group 可以展示进展，但不能成为 Dispatch 或 Task 状态协议。
