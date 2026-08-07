# Agent Runtime Task Executor 设计

- 状态：Draft
- 目标读者：OpenDAN Runtime / Agent Session / Workflow / TaskMgr 集成开发者
- 相关文档：
  - `doc/arch/task_mgr.md`
  - `doc/opendan/OpenDAN Long Task & Sub-Agent.md`
  - `doc/opendan/Agent 协作.md`


## 1. 背景

OpenDAN 当前已经具备 message / event dispatch、session、workspace、agent behavior loop 等基础设施，但 AgentRuntime 还没有成为一个标准 Task Executor。

这会导致 TaskMgr + Workflow 这条线缺少真正的 Agent 侧消费者：TaskMgr 能记录任务，Workflow 能描述执行图，但当任务需要由 Agent 执行时，系统还缺少一个稳定的运行时入口来拉取任务、绑定执行现场、推进状态、处理人工介入并写回结果。

本文定义 AgentRuntime 如何作为 TaskMgr 上的 Task Executor 工作。

## 2. 核心结论

AgentRuntime 应被视为一种标准 Task Executor：

```text
AgentRuntime = Msg/Event Dispatcher + Task Executor + Session Manager
```

其中：

- Message / Group 是通信面，用于人类输入、外部 A2A、通知和协作展示。
- TaskMgr 是执行状态事实源，用于任务归属、进度、错误、结果、权限和订阅。
- Workflow 是执行图和依赖图，用于决定步骤、并发、重试、等待人类和回滚。
- AgentRuntime 是 task consumer / executor，负责执行已经归属于自己的任务。

因此：

```text
IM 是 intent acquisition。
TaskMgr 是 execution coordination。
Workflow 是 execution graph。
AgentRuntime 是 executor。
```

内部 Agent delegate 不应主要建模为 `sendmsg`，而应建模为 TaskMgr task。`sendmsg` 仍用于外部协作和通知，但不作为内部执行状态的事实源。

## 3. 非目标

本文不试图：

1. 把 TaskMgr 扩展成泛聊天协作中心。
2. 定义完整 TaskMgr RDB schema 或替代 `doc/arch/task_mgr.md`。
3. 让 AgentRuntime 从公共 Pending 池里抢任务。
4. 让任务创建者理解 OpenDAN 内部 worksession / workspace 细节。
5. 用 Group message 作为执行状态同步协议。

TaskMgr 仍然是长任务状态总账，不是聊天系统，也不是业务编排器。

## 4. 心智模型

### 4.1 IM 给 Agent 任务

人类通过 IM 给 Agent 的任务，本质是对话中的意图输入。

特点：

- 输入自然语言化，可能模糊。
- 可以只是讨论、探索、纠错、补充信息，不一定已经形成可执行任务。
- 路由重点是找到正确的人际关系、UI session、worksession 和上下文。
- Agent 可以追问、澄清、拒绝、改写目标，必要时再创建 Task 或 Workflow。

典型路径：

```text
Human IM
  -> UI/session router
  -> Agent 理解、追问、规划
  -> 需要执行时创建 Task / Workflow
```

### 4.2 TaskMgr 给 Agent 任务

TaskMgr 委派给 Agent 的任务，本质是系统中的执行合约。

特点：

- 已经有 `task_id`。
- 已经有 `task_type`、`runner`、状态和结果回写位置。
- 状态事实源是 TaskMgr，而不是对话上下文。
- AgentRuntime 执行的是已经归属于自己的 task。
- 补充信息、人工审批、环境阻塞都应该通过任务状态和子任务表达。

典型路径：

```text
TaskMgr task
  -> TaskExecutor 发现归属于自己的 Pending task
  -> 机械路径：data 已是 OpenDAN schema  ─┐
                                          ├─> 业务 worksession 绑定 task_id
     task router 路径：LLM 综合 objective ─┘
  -> session 执行并双向同步状态
  -> 回写 TaskMgr
```

### 4.3 Task 的"一次性"语义

每个 `task_id` 在 AgentRuntime 视角下只能被"接收"一次：

- 接收的标志是 *task_id 被绑定到一个新的 WorkSession*。
- 一旦完成绑定，task 在 AgentRuntime 看来就从"未运行"进入"运行"，之后只允许由 Session ↔ Task 的双向同步推进，不允许再走"创建 worksession"路径。
- Task 中途可以被 `Paused`、阻塞在 `WaitingForApproval`、被 `Canceled`，但这些都通过原绑定恢复或终止，*不会*让同一个 `task_id` 重新走一次 inbox → 创建 session 的流程。

因此重复接收同一个已绑定 task 是一种错误行为，必须被 AgentRuntime 检测并跳过（见 [§7.2](#72-task-executor) 的幂等规则）。Inbox 拉到同一个 task 多次（event 抖动、轮询兜底、跨进程重启）是正常的；幂等检测才是关键，而不是去试图让 inbox 不再返回它。

### 4.4 WorkSession ↔ Task 1:1 关系是 WorkSession 自带的

需要特别澄清：WorkSession 和 Task 之间的一一对应关系，是 **OpenDAN WorkSession 基础设施本身的属性**，不是 Task Executor 发明的。

- 只要一个 WorkSession 开始跑，它就一定有且仅有一个 task_id。
- 如果调用方在 `create_worksession` 时传入了 `task_id`，WorkSession 就绑定那个已有 task；如果没传，WorkSession 会**自动创建**一个新的 task 把自己挂上去（参见 `create_task_for_work_session`）。
- 因此即使 OpenDAN 从头到尾没有用过 `agent.delegate`（没有任何外部 task 委派进来），每个 WorkSession 也仍然有自己的 task_id，1:1 关系仍然成立。

Task Executor 与这个不变量的关系很简单：

> Task Executor 的工作不是"建立 WorkSession↔Task 绑定"，而是"把外部 TaskMgr 已经存在的 task_id **喂给** WorkSession，让 WorkSession 用这个 task_id 而不是自己 mint 一个新的"。

这也是为什么"重复接收 = 错误"的语义来得很自然：因为 WorkSession 总是和**一个**新的 task 一一对应，所以一个外部 task_id 也只能被一个新的 WorkSession 收下一次，再"接收"就意味着要建第二个 WorkSession 去对应同一个 task，破坏 1:1。

## 5. 与 TaskMgr 的边界

AgentRuntime 必须遵守 TaskMgr 的通用 Task Executor 范式：

- `task_type` 决定任务协议和 `Task.data` schema。
- 顶层 `Task.runner` 是执行者 ID。
- executor 只查询归属于自己的任务，例如：

```text
TaskFilter {
  task_type: "agent.delegate",
  runner: self_agent_runtime_id,
  status: Pending
}
```

- 不允许先拉所有 Pending task，再在本地解析 `data` 抢任务。
- TaskMgr 的事件只是唤醒提示，不是状态事实源。醒来后必须 `get_task` 或 `list_tasks` 重新读取。

AgentRuntime 不负责全局调度。谁把 `runner` 指向某个 AgentRuntime，由生产者、Workflow 或 scheduler 决定。

注意：`doc/arch/task_mgr.md` 当前只把顶层 `runner` 约定为 `node_id` 或 `app_id`。本文中的 `target_agent_runtime_id` / `self_agent_runtime_id` 表示 AgentRuntime 视角下的逻辑执行者 ID；落地实现时应先映射到 TaskMgr 当前可接受的 `app_id` / runtime app id，或在扩展 TaskMgr runner 语义时再支持 Agent DID。

## 6. Agent Task 类型

### 6.1 `agent.delegate`

`agent.delegate` 表示一个由 AgentRuntime 承接的内部委派任务。

创建方可以是：

- 另一个 Agent。
- Workflow service。
- 系统服务。
- 人类输入经过 Agent 规划后生成的任务。

消费方是目标 AgentRuntime。

建议 data schema：

```json
{
  "agent_delegate": {
    "version": 1,
    "purpose": "完成某个明确任务",
    "requester_agent_id": "agent-a",
    "owner_session_id": "optional-origin-session",
    "capability": "code.review",
    "input": {},
    "context_refs": [],
    "workspace_hints": [],
    "constraints": {},
    "route": null,
    "execution": null,
    "result": null,
    "error": null
  }
}
```

字段说明：

| 字段 | 说明 |
| --- | --- |
| `purpose` | 面向 Agent 的任务目标描述 |
| `requester_agent_id` | 发起委派的 Agent |
| `owner_session_id` | 原始工作现场，可为空 |
| `capability` | 能力 hint，不替代顶层 `runner` |
| `input` | 结构化输入 |
| `context_refs` | 外部上下文引用，如文件、对象、消息、task |
| `workspace_hints` | 创建方能提供的 workspace 线索 |
| `constraints` | 成本、时间、权限、模型、工具等约束 |
| `route` | task_route session 的解析结果 |
| `execution` | 执行中的 session、workspace、runner 信息 |
| `result` | 成功结果 |
| `error` | 失败信息 |

### 6.2 `agent.route`

`agent.route` 是可选的内部子任务类型，用于记录 task_route session 的路由过程。MVP 可以先把 route 结果直接写回 `agent_delegate.route`。

适用场景：

- workspace 选择存在风险。
- task 需要较复杂的 session / workspace 解析。
- 需要把路由过程单独暴露给 TaskCenter 或调试工具。

### 6.3 `human.input`

`human.input` 表示 executor 遇到不可自动恢复的阻塞，需要人类补充信息或决策。

它不是 Agent 专属能力，而是通用 pipeline executor 模式。Agent 遇到 workspace 不明确、权限不足、磁盘满、缺依赖、需要用户确认时，都可以创建该类子任务。

建议 data schema：

```json
{
  "human_input": {
    "version": 1,
    "kind": "select_workspace",
    "question": "请选择这个任务应该在哪个 workspace 执行",
    "required_by": {
      "task_id": 456,
      "executor": "agent-runtime"
    },
    "candidates": [
      {
        "workspace_id": "ws-1",
        "label": "buckyos"
      }
    ],
    "response_schema": {
      "type": "object",
      "required": ["workspace_id"]
    },
    "response": null,
    "answered_by": null,
    "answered_at": null
  }
}
```

`human.input` 任务应创建为原任务的子任务：

```text
agent.delegate root task
├── agent.route
├── agent.execute
└── human.input
```

## 7. Runtime 组件

### 7.1 Task Inbox

Task Inbox 是 AgentRuntime 内部的任务输入源，负责监听和查询 TaskMgr。

职责：

1. 订阅 `/task_mgr/**` 任务变化事件作为唤醒源（beta2.2 起没有 runner task_ready inbox）。
2. 订阅已知 task/root channel 作为执行中任务变化的加速唤醒源，例如当前正在执行的 root task。
3. 启动时先订阅 runner inbox event，然后立即执行一次 `list_tasks(TaskFilter { task_type, runner, status: Pending })`，覆盖订阅前已经创建的任务。
4. 每次收到任务变化 event 后都重新执行 `list_tasks(task_type = agent.delegate)` 并按 data 内目标 agent（`progress.execution.runner`）本地过滤，event payload 只做唤醒 hint。
5. 保留周期性兜底轮询，避免 event 丢失或订阅中断导致任务永久无人处理。
6. 将候选任务交给 Task Executor。

Task Inbox 只做发现，不做执行。

### 7.2 Task Executor

Task Executor 是真正执行 TaskMgr 任务的组件。

目标用一句话讲清：**为一个 TaskMgr task 创建一个新的 OpenDAN WorkSession，并让这个 WorkSession 使用该 task_id（而不是自己再 mint 一个）**。绑定一旦建立，task 进入"运行"语义，后续生命周期由 Session ↔ Task 的双向同步驱动，不再走"创建"路径。

注意 WorkSession ↔ Task 的 1:1 关系本身是 WorkSession 基础设施自带的（见 [§4.4](#44-worksession--task-11-关系是-worksession-自带的)）；Task Executor 只是在"WorkSession 自己 mint task" 还是"使用外部传入 task_id" 之间选了后者。

职责：

1. 校验任务是否属于当前 AgentRuntime。
2. **幂等检测**：如果 task 已经被绑定（`data.agent_delegate.execution.session_id` 已存在）或 task 已经处于终态 / `Canceled` / `Paused`，按对应分支处理，**绝不再次创建 worksession**。
3. 任务首次接管时推进到 `Running`。
4. 选择创建路径（两条线之一，详见下文）：直接机械路径或 task router 路径。
5. 创建业务 worksession，并在 `data.agent_delegate.execution` 里持久化 `session_id`，作为"已绑定"的事实标记。
6. 等待 session 结束、阻塞或被外部控制（cancel/pause）。
7. 回写 `Completed` / `Failed` / `Canceled` / `WaitingForApproval`。

两条创建路径（互斥，选一条）：

**a) 机械路径 — direct schema**

```text
Known OpenDAN TaskData (data.agent_delegate.purpose 或 input.text 明确)
  -> Task Executor 直接调 create_worksession(task_id=<task_id>)
  -> 业务 WorkSession 立刻绑定原 Task
```

适用场景：任务创建者了解 OpenDAN 体系，已经在 `data.agent_delegate` 里把 objective、workspace_hints 等填好。识别条件刻意保守：必须有 `data.agent_delegate`，且能读到明确 objective（`purpose` 或 `input.text`），且不存在多 workspace 候选这种歧义输入。代码层判断见 `task_data_supports_direct_worksession`。

**b) task router 路径 — LLM 综合 objective**

```text
Unstructured / ambiguous TaskData
  -> 启动内部 task_route session（behavior = "task_route"），它在 OpenDAN 内部承担 task router 角色
  -> LLM 过程：
       * 读取 task.data、笔记、相关 global 状态
       * 完成必要的参数收集（"读取新邮件"指哪个邮箱？要什么格式的总结？）
       * 综合分析出可执行的 objective
  -> LLM 调用 create_worksession 工具，并传入原 task_id
  -> 业务 WorkSession 绑定原 Task
```

适用场景：通用结构化任务描述，包括"Review 我邮箱里的新邮件给我个总结"这种自然语言意图直接落到 task data 的情况。task_route session 自身不执行业务，只完成"任务装载 + 创建业务 session"。它不可见地承担两个职责：workspace/session 路由（原文档语义）+ objective loader（本次新增明确的语义）。

不论走哪条路径，结果上都得到：**一个新的业务 WorkSession，其 SessionMeta.task_binding.task_id = 原 task.id**。

MVP 中，一个 `runner + task_type` 下应只有一个权威 AgentRuntime 进程。若未来允许多个副本共享同一 runner，必须补 `claim` / `lease` / compare-and-set 语义。

### 7.3 task_route session (a.k.a. task router)

task_route session 类似 UI session，但它处理的是 task 装载和路由，不处理人类对话。它在 OpenDAN 内部同时承担两个角色：**workspace/session 路由器** + **objective task router**。

职责：

1. **任务装载 (loader 角色)**：根据 task data 综合出一个可执行的 objective。可以读取 agent 的笔记和相关 global 状态、完成必要的参数收集（比如澄清"读取新邮件"指的是哪个邮箱、报告期望的格式等）。
2. **执行现场路由 (router 角色)**：根据 task data、owner session、workspace hints、recent activity、workspace registry 解析 workspace；决定复用已有 worksession 还是新建 headless worksession；判断 workspace 选择是否需要用户确认。
3. 调用业务 `create_worksession(task_id=<task_id>)`，把装载结果落成一个真正的业务 WorkSession，由该 session 绑定原 task。
4. 输出结构化 route 结果用于审计。

task_route session 自身**不执行业务任务**；它的产物是"一个被原 task_id 绑定的业务 worksession"。

输出示例：

```json
{
  "status": "resolved",
  "target_session_id": "work-123",
  "workspace_id": "ws-buckyos",
  "workspace_path": "/Users/example/project/buckyos",
  "confidence": 0.86,
  "evidence": [
    "matched workspace_hints repo name",
    "recent owner_session activity"
  ],
  "requires_confirmation": false
}
```

如果无法安全解析：

```json
{
  "status": "need_human_input",
  "reason": "workspace_ambiguous",
  "candidates": []
}
```

此时 Task Executor 应创建 `human.input` 子任务并挂起原任务。

### 7.4 Session Manager

Session Manager 负责执行现场的生命周期：

- 复用已有 worksession。
- 创建 headless worksession。
- 绑定 workspace。
- 在任务完成后归档一次性 session。
- 在任务取消时中断 session。

Session Manager 是 OpenDAN 内部能力，任务创建者不应被要求直接指定 worksession 或 workspace。

## 8. 执行流程

### 8.1 正常执行

```text
1. Producer 创建 agent.delegate task
   task_type = "agent.delegate"
   runner = target_agent_runtime_id
   status = Pending

2. AgentRuntime Task Inbox 被 `/task_mgr/**` 任务变化 event 或 timer 唤醒

3. Task Inbox list_tasks:
   task_type = "agent.delegate"
   runner = self_agent_runtime_id
   status in { Pending, Running, WaitingForApproval, Paused, Canceled }

4. Task Executor 对每个 task 做幂等分流：
   - 已绑定 session_id          -> 唤醒原 session，不创建新 session
   - Canceled / Paused          -> 反向同步到原 session（interrupt / pause），写回控制状态
   - WaitingForApproval         -> 检查 human.input 子任务是否完成；完成则 resume
   - 终态 (Completed/Failed)    -> 跳过
   - 首次出现的 Pending         -> 进入步骤 5

5. Task Executor 选择创建路径：
   - 机械路径：task.data 满足 direct schema
       -> create_worksession(task_id=<task_id>)，立刻绑定
   - task router 路径：其它情况
       -> 启动 task_route session，由它在 LLM 过程中
          完成 objective 综合与 create_worksession(task_id=<task_id>)

6. 业务 WorkSession 创建完成后，
   data.agent_delegate.execution.session_id 被写回；
   该字段是"task 已绑定 / 已运行"的事实标记。

7. WorkSession 执行业务任务，期间持续把进度/状态镜像回 Task（见 §9）。

8. session 成功结束
   status = Completed
   progress = 100
   data.agent_delegate.result = ...
```

注意第 4 步的幂等分流：同一个 `task_id` 多次出现在 inbox 里（事件抖动、轮询兜底、重启恢复）都是正常的，但 *只有第一次* 会创建 worksession；之后只会唤醒、同步或忽略。这就是 §4.3"一次性"语义在执行层的体现。

### 8.2 路由需要补充信息

```text
1. task_route 无法安全选择 workspace
2. Task Executor 创建 human.input 子任务
3. human.input.status = WaitingForApproval
4. agent.delegate.status = WaitingForApproval
5. 用户在 TaskCenter 或相关 UI 中补充信息
6. human.input -> Completed
7. AgentRuntime 被事件或轮询唤醒
8. 重新读取 root task 和 human.input 结果
9. agent.delegate -> Running
10. 继续执行
```

### 8.3 执行中遇到外部阻塞

外部阻塞包括：

- 磁盘空间不足。
- 缺少权限。
- 需要用户确认风险操作。
- workspace lease 冲突。
- 依赖服务不可用且需要人工判断。

处理原则：

```text
任何 executor 遇到不可自动恢复的阻塞
  -> 创建 WaitingForApproval 子任务
  -> 原任务进入 WaitingForApproval 或保持 Running 但 message 标明 blocked
  -> 子任务完成后恢复执行
```

AgentRuntime 不应为这些场景发明独立聊天协议。它和普通 pipeline executor 一样，通过任务树表达阻塞和恢复。

### 8.4 取消

当 root task 或当前执行 task 被 `Canceled`：

1. AgentRuntime 必须停止继续推进业务执行。
2. 如果已启动 session，应请求 session 中断。
3. 如果有子任务，也应根据业务语义取消或保留审计记录。
4. 最终写回 `Canceled`，并在 `data.agent_delegate.execution` 中记录中断原因。

`Paused` / `Canceled` 也要被 poll fallback 覆盖。AgentRuntime 轮询到已绑定
`data.agent_delegate.execution.session_id` 的 `Paused` / `Canceled` task 时，应中断对应
WorkSession，并把 `data.agent_delegate.execution.status` 写成 `paused` / `canceled`，避免下次轮询重复处理同一次控制动作。

## 9. 状态映射

### 9.0 双向同步原则

绑定建立之后，Task 和 WorkSession 之间是**双向影响**的：

- **Session → Task（执行驱动）**：业务 WorkSession 在自己的事件循环里推进时，会实时把状态、progress、结果或错误写回 `task_mgr.update_task(task.id, …)`，刷新到 `data.agent_delegate.execution.*`。这是 TaskCenter / Workflow 看到任务进展的来源。
- **Task → Session（控制驱动）**：用户或上游 Workflow 在 TaskMgr 上做的状态变更（`Cancel`、`Pause`、`Resume`、`WaitingForApproval` 子任务回填等），由 Task Executor 在 inbox sweep 时观察到，然后反向作用到已绑定的 session 上（`interrupt`、`pause`、`enqueue_pending` 等）。

这两条方向都通过 *同一个绑定关系* (`task_id ↔ session_id`) 工作；任何一方都不应试图绕过绑定去新建另一组 session/task。

### 9.1 状态对照表

| Task 状态 | AgentRuntime 语义 |
| --- | --- |
| `Pending` | 已分配给该 Agent，但尚未开始执行 |
| `Running` | AgentRuntime 已接手，正在路由或执行 |
| `WaitingForApproval` | 等待人类输入、审批或外部不可自动恢复条件 |
| `Paused` | 暂停执行，可由用户或系统恢复 |
| `Completed` | session 成功结束，结果已写回 |
| `Failed` | 执行失败，错误已写回 |
| `Canceled` | 用户或系统取消，WorkSession 必须停止继续推进 |

WorkSession 的内部状态会镜像到 Task：

| WorkSession 状态 | Task 写回 |
| --- | --- |
| `Running` / `WaitingTool` | `Running`，并刷新 `data.agent_delegate.execution.session_status` |
| `WaitingInput` | `WaitingForApproval`，必要时创建 `human.input` 子任务 |
| `Ended` | `Completed`，progress 写到 100 |
| `Error` | `Failed` |
| `Idle` | 不覆盖 Task 状态，只刷新 `data.agent_delegate.execution.session_status` |

镜像前必须读取 Task 当前状态；如果 Task 已是终态或 `Paused`，WorkSession 不应把它覆盖回 `Running`。

父任务和子任务的状态关系：

- 如果根任务因人类输入阻塞，根任务应进入 `WaitingForApproval`，子任务也为 `WaitingForApproval`。
- 如果根任务仍有其它可并行工作，可以保持 `Running`，但 `message` 必须说明当前存在 blocker。
- TaskCenter 首页应优先展示 `WaitingForApproval` 子任务；根任务详情页展示完整任务树。

## 10. 与 Workflow 的关系

Workflow 负责执行图，AgentRuntime 负责执行其中分配给 Agent 的节点。

可以有两种映射：

1. Workflow 直接创建 `agent.delegate` task，`runner` 指向目标 AgentRuntime。
2. Workflow 创建 `workflow/thunk`，调度器再派生或转换为 `agent.delegate` task。

无论哪种方式：

- Workflow 仍负责依赖、重试、回滚和 run 状态推进。
- AgentRuntime 只负责执行分配给自己的 task。
- AgentRuntime 的结果通过 TaskMgr 写回，Workflow 通过事件或轮询读取结果后继续推进。

Agent 可以作为 Workflow executor backend，但不应把 Workflow 的依赖图隐藏在 Agent 间消息里。

## 11. 与 MsgCenter / Group 的关系

MsgCenter 和 Group 是通信与展示面，不是内部执行事实源。

适合用 message / group 的场景：

- 人类给 Agent 下达自然语言意图。
- Agent 向人类解释进展。
- 外部 Agent / Human 的 A2A 协作。
- 将 TaskMgr 状态投影到 Group 中做共享 report。

不适合用 message / group 的场景：

- 表达内部执行图。
- 判断 task 是否完成。
- 表达重试、取消、恢复、权限和审计。
- 让多个 Agent 通过群聊协商抢任务。

Group 中展示的执行进度应是 TaskMgr / Workflow 状态的投影，而不是状态本身。

## 12. 人类介入模型

TaskMgr 不需要通用自由 note / comment 流来承载 Agent 追问。人类介入应优先建模为子任务。

原因：

- 子任务天然有状态、权限、事件、UI 入口。
- 可以被 TaskCenter 首页识别为待处理项。
- 可以审计谁创建、谁处理、处理结果是什么。
- 可以适用于 AgentRuntime、node executor、download、app install 等所有 executor。

推荐通用模式：

```text
executor 遇到阻塞
  -> create_task(task_type = "human.input", parent_id = current_task)
  -> child.status = WaitingForApproval
  -> parent.status = WaitingForApproval
  -> 用户或系统写入 response
  -> child.status = Completed
  -> executor 恢复 parent
```

`human.input` 的 response 写入子任务 `data.human_input.response`，不直接覆盖父任务业务输入。父任务 executor 恢复后读取子任务结果并决定是否继续、失败或再次请求补充信息。

## 13. 可靠性要求

### 13.1 Event 只做唤醒

AgentRuntime 收到 task_mgr kevent 后必须重新读取 TaskMgr 状态。不能只相信 event payload 推进业务。

对于新任务发现，AgentRuntime 应订阅：

```text
/task_mgr/**
```

收到该事件后仍必须调用 `list_tasks(TaskFilter { task_type, runner, status: Pending })`。该事件只表示“这个 runner 可能有新的 Pending task”，不表示某个 task 已被当前 executor 接手。

### 13.2 Poll fallback

Task Inbox 必须有 timer / poll fallback，避免 event 丢失导致任务永久无人处理。

### 13.3 幂等

AgentRuntime 对同一个 task 的处理必须严格幂等。同一个 task_id 会被 inbox 拉到多次（事件抖动、轮询兜底、重启恢复），这是正常的；幂等检测保证它只被绑定一次：

- 启动前先 `get_task` 读取当前状态，不要相信 inbox event payload。
- 终态任务（`Completed` / `Failed` / `Canceled`）不再执行。
- **`data.agent_delegate.execution.session_id` 存在即视为已绑定**：此时不允许再创建新 worksession；只能走"唤醒原 session / 反向同步控制状态 / 检查 human.input 子任务"分支。
- 同时把"已绑定"看作 task 进入"运行"语义的标志：一旦写入，重新拉到的同一个 task_id 都按已运行处理，不再回到 inbox 的"创建"路径。
- 写结果时使用 `update_task` 一次提交状态、message 和 data patch。

### 13.4 单 runner 单 executor

在缺少原子 `claim` / `lease` API 前，部署约束必须保证同一 `runner + task_type` 只有一个权威 executor 进程。

如果要支持多个副本共享 runner，需要新增 TaskMgr 能力：

- `assign_runner`
- `claim_task`
- `heartbeat_task`
- `release_task`
- `lease_until`

这些能力属于后续增强，不是 MVP 的前提。

## 14. MVP 实现范围

第一阶段只做最小闭环：

1. 定义 `agent.delegate` TaskData schema。
2. AgentRuntime 增加 Task Inbox。
3. AgentRuntime 轮询 `TaskFilter { task_type: "agent.delegate", runner: self_agent_runtime_id, status: Pending }`。
4. 对任务执行 `Pending -> Running`。
5. 对可识别的 OpenDAN TaskData 直接 `create_worksession(task_id=...)`。
6. 对不可识别或有歧义的 TaskData，通过 task_route session 解析执行现场。
7. 创建 headless worksession 执行任务。
8. 成功写回 `Completed`，失败写回 `Failed`。
9. 路由不确定时创建 `human.input` 子任务并进入 `WaitingForApproval`。
10. 子任务完成后恢复 root task。

MVP 不做：

- 多 executor 副本抢同一个 runner。
- 完整 claim / lease。
- 自由 note / comment。
- 用 Group 驱动执行图。
- 要求 producer 指定 worksession / workspace。

## 15. 后续增强

1. 补 TaskMgr 原子 claim / lease，用于多副本 AgentRuntime。
2. 将 `human.input` 提升为 TaskMgr / TaskCenter 通用可渲染 schema。
3. 增加 `agent.route` 子任务，用于可审计的路由过程。
4. 支持 Agent capability registry，让 scheduler 根据能力选择 `runner`。
5. 支持 TaskCenter 对 `agent.delegate` / `human.input` 的结构化展示。
6. 支持 Group report 订阅 TaskMgr root task tree，把执行状态投影到群组。

## 16. 设计原则摘要

1. AgentRuntime 是 Task Executor，不是只处理 message/event 的聊天运行时。
2. TaskMgr 委派是执行合约，IM 委派是意图输入。
3. `runner` 表示任务归属，`data` 表示业务语义。
4. Executor 的目标只有一件事：**为外部 task_id 创建一个新的业务 WorkSession，并让它使用这个 task_id**。
5. WorkSession↔Task 的 1:1 关系是 WorkSession 基础设施自带的（没有 task_id 传入时 WorkSession 会自动 mint 一个）；Executor 只是把外部 task_id 喂进去，而不是建立这个不变量。
6. Task 是一次性的：绑定即"已运行"；重复接收同一 task_id 必须被幂等检测拦下，不允许再建第二个 WorkSession 去对应同一个 task。
7. 两条创建路径：机械路径（direct schema → `create_worksession(task_id=...)`）和 task router 路径（task_route session 负责 objective 综合 + workspace 路由）；运行结果都收敛为同一个绑定形态。
8. 绑定建立后 Task ↔ Session 双向影响：执行进度从 Session 写回 Task，控制状态从 Task 反向作用到 Session。
9. 人类介入用 `WaitingForApproval` 子任务表达，不用自由聊天 note 表达。
10. Event 只做唤醒，TaskMgr state 才是事实源。
11. Group 可以展示任务进展，但不能成为执行状态协议。
