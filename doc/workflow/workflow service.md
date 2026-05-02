# BuckyOS Workflow Service 技术需求文档

> 配套文档：
> - [wokflow engine.md](./wokflow%20engine.md)（v0.4.0-draft，引擎语义层需求）
> - [executor list.md](./executor%20list.md)（executor 分类与一期实现优先级）
>
> 本文从**实体服务**的视角描述 Workflow Service：它在 BuckyOS Zone 内是一个长期运行的标准服务，对外提供 workflow 定义管理、workflow-task（运行实例）管理与执行驱动；对内承接编排器主循环，并通过 adapter / dispatcher 与其它服务协同。

## 0. 服务定位与一句话职责

**Workflow Service = 编排器。**

它管两类东西：

1. **Workflow 定义**（Definition）：DSL 形式的工作流模板，编译产物（Expr Tree / 静态分析报告）也由它持有。
2. **Workflow Task**（运行中的实例，对应 [wokflow engine.md](./wokflow%20engine.md) 中的 Run）：Run 状态机、节点输出、事件流、人类介入记录、ThunkObject 投递关系。

它**不**实现：

- 具体的 executor 能力（`service::aicc.*`、`service::msg_center.*` 等都是各自的服务，由 adapter 桥接）。
- Thunk 在节点上的真正执行（`func::*` / `optask::*` 由 Scheduler + Node Daemon Runner 完成）。
- 用户通知通道与 IM/邮件投递（由 Msg Center 完成）。
- 全局任务总账（由 Task Manager 完成；workflow service 把每个 Run 同步成 task_manager 的一条 task）。

简言之，workflow service 只对外承诺：**“给我一个合法的 Workflow 定义和外部输入，我返回一个可观测、可干预、可恢复的 Run。”**

从操作系统视角看，一个 Workflow Run 的核心就是这样一个循环：

> **检查依赖 task 的状态 → 创建并执行下一个 task → 直到全部终止。**

这意味着 workflow service **不需要为长任务管理另造一套 UI**：进度查看、状态钻取、暂停/恢复、错误展示、待办列表、用户审批、通知 —— 都由已有的 TaskMgr UI + 通知中枢承担。workflow service 在运行时实际只做四件 task_manager 做不了的事：把 DSL 编译成任务 DAG、解析 Reference 把上游输出穿到下游、解释 TaskData 中的 `human_action`、把对应 Apply 调度到 executor adapter 或 scheduler。除此之外的"用户怎么看到、怎么操作"完全融入 OS 的统一任务面。

从产品视角看也是同一结论。在一个企业级部署里，预期受众分布大致是：

- **10% 管理员 / 流程设计者**：他们才会打开 workflow 的 DSL / DAG 视图——这个视图认知门槛高，对普通人冲击力很强，本就不该作为日常入口暴露出去。
- **90% 普通用户**：他们应该**自始至终只看到 TaskMgr UI**——已经熟悉的待办列表、审批按钮、进度条；既不知道也不需要知道背后是不是有一个 workflow 在跑。

让 workflow runtime 完全不出现在普通用户面前，正是这条架构选择在产品层面的目的：复杂度留在系统内，呈现给绝大多数用户的仍然是统一、低门槛的任务体验。

## 0.1 设计原则：能用 task_manager 就用 task_manager

Workflow service 自身**不再造一套并行的状态总账**。Run / Step / Thunk / Map shard 这类“可被外部观测、可能跨进程协作”的执行单元，都建模为 task_manager 的任务，按父子关系挂在同一棵任务树下：

```
task_manager task tree
└── Run task            (root, type=workflow/run)
    ├── Step task       (child, type=workflow/step)
    │   ├── Thunk task  (grandchild, type=workflow/thunk，func::* / optask::* 才有)
    │   └── ...
    ├── Map shard task  (child, type=workflow/map_shard)
    └── ...
```

由此带来的几条具体约束：

1. **状态查询走 task_manager**。任何“某个 Step / Thunk 现在跑成什么样了”的问题，外部一律通过 `task_manager.get_task` / 子任务列举得到答案；workflow service 不再额外暴露平行的状态查询。
2. **执行结果由 task_manager 反映，不需要专用回执 RPC**。例如 Scheduler 执行一个 Thunk，只要它把对应 task 的 status / payload / progress 更新到 task_manager，workflow service 通过订阅或轮询 task_manager 即可推进主循环。**不**为此单独定义 `workflow.notify_thunk_result` 之类的回灌接口。
3. **生命周期控制尽量复用 task_manager 的能力**。pause / resume / cancel / 进度上报 / 错误信息 / 树状导航这些通用能力直接借 task_manager；workflow 只在 task_manager 不能表达的语义上加自己的字段（例如 `node_id`、`attempt`、`shard_index`、`HumanActionKind`）。
4. **workflow service 私有的状态只保留两类**：
   - **编译态产物 / DSL 语义**：Definition、Expr Tree、Reference 解析、`node_outputs` 之间的有向图、Amendment 版本链 —— 这些是 task_manager 不理解的 workflow 语义，必须自己存。
   - **跨 task 的语义关联**：例如哪个 Run task 对应哪个 Definition、Step task 与 Expr 节点的映射、Thunk task 与 ThunkObject 的映射、副作用回执的指针。
5. **不重复，但允许镜像**。Run 的 high-level 状态（Running / WaitingHuman / Completed …）由 workflow 计算，但要写回到 Run task 的 status，使得只看 task_manager 的人也能知道 Run 在什么阶段；Step 和 Thunk 同理。
6. **用户写 TaskData 即下达 HumanAction**。BuckyOS 已经规定用户对其拥有的 SubTask 有写 TaskData 的权限，TaskMgr UI 会根据 task type / TaskData 渲染合适的按钮（approve / modify / reject / retry / skip / abort / rollback 等）。Workflow service **不**为最终用户单独再开 `workflow.approve_step` 这类公开 RPC——用户的写动作直接落到对应 Step / Run task 的 TaskData 中，workflow 通过订阅 task_manager 的 TaskData 变更来理解并执行：
   - 读出 TaskData 中的 `action` 字段（对应 `HumanActionKind`）与 payload；
   - 按 DSL 中该 Step 的 `output_schema` / stakeholder / 当前节点状态做合法性校验；
   - 校验通过 → 推进状态机 + 把结果回写到任务的 status / TaskData；
   - 校验失败 → 把错误原因写回 TaskData（例如 `last_error`），让 TaskMgr UI 重新呈现给用户修正。
7. **通知 = TaskMgr UI URL**。当 Run 进入 `WaitingHuman` 时，workflow 把对应 Step / Run task 标记为"需用户处理"；OS 的统一通知通道把该 task 的 TaskMgr UI URL 推给用户，用户在那里完成动作，不需要任何 workflow 专属客户端。

下文所有“状态 / 回执 / 查询 / 用户操作”相关的设计都遵循这几条原则。

> **小结**：从 OS 视角看 workflow service 就是一个"DAG 驱动器"——它把 DSL 编译成 task_manager 任务树，循环做"等依赖 task 完成 → 创建下一批 task"。整个长任务管理界面（进度、待办、审批、回滚、通知）都由 TaskMgr UI 和 OS 通知中枢统一承担。workflow 私有的 UI 只剩下"Definition 编辑/可视化"这种与 *运行* 无关的部分。

## 1. 服务边界

### 1.1 服务对外暴露

| 维度 | 内容 |
|------|------|
| 服务名 | `workflow`（standard service，可被其它服务以 `service::workflow.<method>` 调用） |
| 协议 | buckyos-api kRPC + REST（与 task_manager / aicc 一致），事件流走 kevent/kmsgqueue |
| 资源域 | Workflow 定义、Workflow Run、Run 事件、Run 上的人类介入记录 |
| 治理 | Zone owner / per-app namespace；写操作均走 ACL 校验 |

### 1.2 服务依赖

| 依赖 | 用途 | 失败影响 |
|------|------|----------|
| Named Object Store / named_store | 持久化 ThunkObject、idempotent cache、Stream checkpoint、副作用回执 | 无法投递新 Thunk，但不影响纯编排器侧 adapter 的 `service::` / `http::` / `appservice::` / `operator::` Apply 执行 |
| system_config | 读取 endpoint registry、AppService 注册、executor registry | 影响新 Run 启动；已运行 Run 仍可继续 |
| Task Manager (`task_manager.*`) | Run / Step / Thunk / Map shard 的状态总账（见 §0.1 与 §6.3） | task_manager 不可用时，workflow 只能依赖本地编译态状态推进，对外查询和 Thunk 进度感知严重降级；不阻塞已在编排器侧 adapter 中执行的 Apply |
| Msg Center | 经 task_manager 间接使用：task_manager 检测到 Step / Run task 进入 `WaitingForApproval` 后，统一构造 TaskMgr UI URL 并经 Msg Center 推送给 stakeholder（见 §6.4）。workflow 自身不再直连 Msg Center | 通知不达，但 Step task 仍然是 `WaitingForApproval`，用户在 TaskMgr UI 中仍可主动看到并处理 |
| Scheduler + Node Daemon Runner | 执行解析为 FunctionObject 的 `Apply`（一期非主路径） | 阻塞 `func::*` / `optask::*` 类节点，不影响一期 P0 主流程 |
| 其它标准服务 / AppService / HTTP endpoint | 编排器侧 adapter 直接调用 | 对应 Step 失败；按 retry / human fallback 处理 |

### 1.3 服务**不**承担

- **不**承担 DSL 设计层的扩展（DSL 演进与版本升级在 [wokflow engine.md](./wokflow%20engine.md) 中由引擎团队推进，此服务只承诺解析当前 schema_version 范围内的 DSL）。
- **不**承担 executor 的注册与发现实现细节（registry 由 system_config / 包管理协同，本服务只消费 registry 视图）。
- **不**承担 budget 计费的真实账本（service 只负责 budget 检查与扣减事件，账本由计费服务/AICC usage log 沉淀）。

## 2. 资源模型

服务对外只暴露两类一等资源，加两类附属流。

### 2.1 Workflow Definition

```
{
  "id": "wf-<ulid>",                     // 服务侧主键
  "schema_version": "0.4",
  "name": "kb_import_pipeline",
  "owner": { "user_id": "...", "app_id": "..." },
  "definition": { ...DSL JSON... },      // 原始 DSL
  "compiled": { ...CompiledWorkflow... },// 编译产物（Expr Tree + 节点表）
  "analysis": { ...AnalysisReport... },  // 静态分析报告
  "status": "draft" | "active" | "archived",
  "created_at": ts,
  "updated_at": ts,
  "tags": [...]
}
```

约束：

- DSL 入库前必须通过 `compile_workflow` + `analyze_workflow`，分析报告中存在 `Error` 级别 issue 时拒绝写入。
- 同名 Definition 的多次更新形成只增不减的 version 链；运行中的 Run 永远绑定到创建时的 version。
- Definition 的修改不会影响已存在的 Run（与 Amendment 区分：Amendment 修改的是 Run 内嵌的计划，不修改 Definition）。

### 2.2 Workflow Run（Workflow-Task）

对应 `WorkflowRun`（[runtime.rs](src/kernel/workflow/src/runtime.rs)）。对外可见的关键字段：

| 字段 | 说明 |
|------|------|
| `run_id` | 主键 |
| `workflow_id` / `workflow_name` / `plan_version` | 绑定到具体 Definition 版本 |
| `status` | `RunStatus` 枚举（`Created` / `Running` / `WaitingHuman` / `Completed` / `Failed` / `Paused` / `Aborted` / `BudgetExhausted`） |
| `node_states` | 每个节点的 `NodeRunState` |
| `node_outputs` | 节点输出，供下游 Reference 解析 |
| `pending_thunks` | 当前已投递、等待调度器回执的 ThunkObject |
| `human_waiting_nodes` | 当前需要人类介入的节点集合 |
| `map_states` / `par_states` | for_each 与 parallel 的运行时分片状态 |
| `metrics` | budget / token 用量等度量 |
| `seq` | 单调递增的事件序列号，外部断点续拉用 |

Run 是**服务的唯一执行单元**。Run 的生命周期与 task_manager 中对应 task 的生命周期一对一同步（见 §6.3）。

### 2.3 附属流

- **Event Stream**：`EventEnvelope`（同 runtime.rs），按 `seq` 严格递增，可断点续拉；外部仅可读取与订阅，不可写入。
- **Human Action**：`HumanAction`（approve / modify / reject / retry / skip / abort / rollback），是从外部世界写入 Run 的唯一通道（除了 Amendment）。
- **Amendment**：运行时计划修改请求，遵循 [wokflow engine.md §3.8] 的协议，必须经过审批后才生效。

## 3. 对外 API

API 命名沿用 buckyos-api 风格，并与 [wokflow engine.md §4.10] 对齐。一个 method 同时具备 RPC 与 REST 两种入口。

### 3.1 Workflow Definition 管理

| 方法 | 说明 | 幂等 |
|------|------|------|
| `workflow.submit_definition` | 提交 / 升级一个 Workflow 定义。服务编译并做静态分析，返回 `workflow_id` + `analysis` | 以 (`owner`, `name`, hash(definition)) 为键幂等 |
| `workflow.get_definition` | 按 id 或 name 拉取定义、编译产物、分析报告 | 是 |
| `workflow.list_definitions` | 列表 + 过滤（owner / tag / status） | 是 |
| `workflow.archive_definition` | 软删除，不允许新建 Run，对已有 Run 无影响 | 是 |
| `workflow.dry_run` | 仅做编译 + 静态分析，不建 Run；用于 Agent 提交前的自检 | 是 |

### 3.2 Workflow Run 生命周期

| 方法 | 说明 |
|------|------|
| `workflow.create_run` | 用某个 Definition + 触发输入创建 Run（不自动启动）。返回 `run_id` |
| `workflow.start_run` | 进入主循环；首次进入会发出 `run.started` |
| `workflow.tick_run` | 由内部调度器调用；外部无须直接使用，但保留作为运维入口 |
| `workflow.get_run_graph` | 返回当前展开后的 Workflow Graph（task_manager 不理解的 DSL 拓扑），给 UI 可视化 |
| `workflow.list_runs` | 按 owner / definition / status / 时间范围筛选；其它 Run 状态查询请走 `task_manager.get_task` |

> `create_run` 与 `start_run` 拆开，是为了给 Agent 留“提交计划但等人类启动”的工作流。一期默认实现中，`create_run` 后立刻调一次 `start_run` 也是合法路径。
>
> Run 整体级的 pause / resume / cancel / 状态读取都退化为 task_manager 在 Run task 上的标准操作（用户写 TaskData 或 status，workflow 订阅后级联到子任务），不再单独提供 `workflow.pause_run` / `workflow.abort_run` 这类 RPC。

### 3.3 用户操作 = 写 TaskData（不是 workflow RPC）

按 §0.1 第 6 条，最终用户对 Step / Run / Thunk task 执行 approve / modify / reject / retry / skip / abort / rollback 时**不调用 workflow 的 RPC**，而是经 TaskMgr UI 写到对应 task 的 TaskData。Workflow 订阅 task_manager 的 TaskData 变更并负责解释。

约定的 TaskData schema（workflow 写入的待办、用户写入的动作都落在同一个 TaskData 里）：

```json
{
  "workflow": {
    "run_id": "run-...",
    "node_id": "scan",
    "attempt": 2,
    "shard_index": null,
    "subject_obj_id": "...",
    "prompt": "请检查扫描结果",
    "output_schema": { ... },
    "stakeholders": ["user-A", "role:reviewer"]
  },
  "human_action": {                     // ← 用户写入这一段
    "kind": "modify",                   // approve / modify / reject / retry / skip / abort / rollback
    "payload": { ... },                 // 仅 modify / submit_output / rollback(target_node_id) 等需要
    "actor": "user-A",
    "submitted_at": 1730000000
  },
  "last_error": null                    // ← workflow 校验失败时写回这里
}
```

要点：

- 用户能写 TaskData 是 OS 给定的能力，ACL 由 task_manager 校验；workflow 在 DSL 中声明的 `stakeholders` 通过 task_manager 的子任务 ACL 表达，不重新做一套人/角色识别。
- TaskMgr UI 看到 `task.type == "workflow/step"` 且当前是 `WaitingForApproval` 时，按 TaskData.workflow 中的字段渲染按钮、表单、subject 预览；按钮点击 = 一次 TaskData 写入。
- workflow 收到 TaskData 变更后做合法性校验：失败时写回 `last_error` + 把 task status 仍保持在 `WaitingForApproval`，UI 据此提示用户修正；成功时按 `HumanActionKind` 推进状态机并更新 status。
- `rollback` 一类涉及多 Step task 的图级动作：用户在 Run task（或目标 Step task）的 TaskData 写 `{ kind: "rollback", payload: { target_node_id } }`，由 workflow 解析并级联回置子任务。

### 3.4 Agent / 外部系统集成入口

Agent 与外部回调走的是程序对程序的接口，不属于"普通用户写 TaskData"路径，因此仍由 workflow 提供专门的 RPC：

| 方法 | 说明 |
|------|------|
| `workflow.submit_step_output` | Agent / 外部 callback 直接写入步骤输出。语义等价于在该 Step task TaskData 中写一个 `human_action.kind = "submit_output"`，但走 RPC 便于鉴权与回调链路 |
| `workflow.report_step_progress` | Agent 报告进度，仅落事件 |
| `workflow.request_human` | Agent 主动把当前 Step 切到 `WaitingHuman`，由 workflow 写 TaskData 等待用户操作 |

### 3.4 Amendment

| 方法 | 说明 |
|------|------|
| `workflow.submit_amendment` | 提交对当前 Run 的计划修改 |
| `workflow.approve_amendment` | 通过审批，修改进入 `plan_version + 1` |
| `workflow.reject_amendment` | 拒绝 |

### 3.5 事件订阅

| 方法 | 说明 |
|------|------|
| `workflow.get_history` | 拉取 `seq >= since_seq` 的事件，分页 |
| `workflow.subscribe_events` | 经 kevent / kmsgqueue 投递事件流，订阅方按 `seq` 做断点续拉与对齐 |

### 3.6 Thunk / Step 状态获取（不新增专用回执 RPC）

按 §0.1 的原则，workflow service **不**为 Scheduler 单独提供回灌结果的 RPC。Scheduler 只需把它执行的 Thunk task 的状态、payload、progress、error 写回到对应的 task_manager 任务，workflow service 通过：

- 订阅 task_manager（首选）或对未结束子任务做轻量轮询；
- 命中状态变化后驱动一次 `tick`；

完成 Thunk 结果回灌。这样：

- Scheduler 不需要知道 workflow service 的存在，只需要懂 task_manager。
- 任何能查 task_manager 的工具（CLI / Dashboard / 其它服务）天然就能看到 Thunk 进度，不必经过 workflow。
- 外部如果想知道某 Step 的执行细节，直接查对应的 Step task / Thunk task，不需要额外问 workflow。

只有 task_manager 无法表达的纯 workflow 语义（如 `node_id`、`attempt`、`shard_index`、`HumanAction`）才会出现在前述 workflow.* API 中。

## 4. 内部模块组成

服务进程内的模块结构与现有 `src/kernel/workflow/` 一一对应：

| 模块 | 文件 | 职责 |
|------|------|------|
| DSL & Schema | [dsl.rs](src/kernel/workflow/src/dsl.rs)、[schema.rs](src/kernel/workflow/src/schema.rs) | DSL 类型、JSON Schema 定义 |
| Compiler | [compiler.rs](src/kernel/workflow/src/compiler.rs) | DSL → Expr Tree / CompiledWorkflow / WorkflowGraph |
| Static Analysis | [analysis.rs](src/kernel/workflow/src/analysis.rs) | 引用闭合、死节点、idempotent 一致性等检查，输出 `AnalysisReport` |
| Runtime State | [runtime.rs](src/kernel/workflow/src/runtime.rs) | `WorkflowRun`、`EventEnvelope`、`HumanAction` 等 |
| Orchestrator | [orchestrator.rs](src/kernel/workflow/src/orchestrator.rs) | `tick`、`schedule_apply`、`handle_thunk_result`、`enter_human_wait` 等主循环逻辑 |
| Executor Registry & Adapter | [executor_adapter.rs](src/kernel/workflow/src/executor_adapter.rs)、[adapters/](src/kernel/workflow/src/adapters/) | 编排器侧 Apply 直执行（`service::` / `http::` / `appservice::` / `operator::`） |
| Thunk Dispatcher | [dispatcher.rs](src/kernel/workflow/src/dispatcher.rs) | 把 ThunkObject 投递给 Scheduler，承接回执 |
| Object Store | [object_store.rs](src/kernel/workflow/src/object_store.rs) | ThunkObject / 大对象持久化（基于 Named Object Store） |
| Task Tracker | [task_tracker.rs](src/kernel/workflow/src/task_tracker.rs) | Run 状态向 Task Manager 同步 |
| RPC Frontend | 待实现：`src/frame/workflow_service/` | 把上述能力暴露成 buckyos-api method + REST handler |

“服务化”相对当前代码增加的部分主要在最后一行：把 `src/kernel/workflow` 这个库 crate 包成一个 `src/frame/workflow_service` 的二进制服务，处理 RPC 路由、ACL、并发与持久化加载。

## 5. 状态与持久化

### 5.1 持久化项

参考 [wokflow engine.md §4.5]，本服务必须持久化的最小集合：

| 数据 | 存储 | 原因 |
|------|------|------|
| Workflow Definition + 编译产物 + 分析报告 | service-local（SQLite/sled，与现有 frame service 一致） | 提交后可被多个 Run 引用，也参与审计 |
| Run 编译态视图 + 节点输出 + Reference 解析 + Amendment 版本链 | service-local | task_manager 不理解 workflow 语义，必须自己存（不重复 task_manager 已有的 status / progress / error 等通用字段） |
| Run / Step / Thunk / Map shard 的执行状态、progress、error_message、payload | task_manager（任务树形式，见 §6.3） | 单一总账，避免与本地状态出现分歧 |
| Run 事件流（`EventEnvelope`） | service-local，按 `run_id` 分段，按 `seq` 索引 | 支撑 history、subscribe、断点重放 |
| ThunkObject、Apply 输入输出大对象 | Named Object Store | 内容寻址、与调度器共享、cache 复用 |
| 副作用回执 (`side_effect_receipt`) | service-local + Named Object Store | rollback 决策依据 |
| 人类介入记录 (`HumanAction`) | service-local | 审计与回放 |

### 5.2 写入语义

- **单 Run 串行**：单个 Run 的 tick / 外部 action 必须串行执行（per-run lock + 顺序事件 seq）。多 Run 之间天然并发。
- **事件先持久化再可见**：emit 事件时先落盘 + bump seq，再回 RPC / 推送订阅者；保证订阅者看到的事件流可重放。
- **恢复**：服务进程启动时，从持久化加载所有非终止态 Run，按 task_manager 中对应任务树的现状对齐本地状态后，再依次 `tick` 直到无新进展。`idempotent: false` 的步骤恢复时**不**自动重跑，强制进入 `WaitingHuman`（与 [wokflow engine.md §4.5] 一致）。

### 5.3 Run 主循环线程模型

- 单服务进程内多 Run 用 `tokio` 异步并发执行。
- 每个 Run 有一个独占的 in-memory state 与一把 `Mutex`，避免对外 RPC 与 task_manager 状态变更并发改写。
- 主循环触发源：
  1. `start_run` / `tick_run` 显式调用。
  2. Agent 调用 `submit_step_output` / `report_step_progress` / `request_human` 等 RPC 后立即 tick。
  3. task_manager 中关注的 task **status 或 TaskData** 变更：通过订阅或轻量轮询获知，统一转换成 tick 触发——既覆盖 scheduler 写入 Thunk 状态，也覆盖用户写入 Step / Run task 的 `human_action` TaskData，不再有专用回执 / 动作 RPC。
  4. 定时器：人类等待节点超时、retry backoff 到期，由专用 timer wheel 触发 tick。

## 6. 与其他服务的协作协议

### 6.1 与 Executor 适配（`service::` / `http::` / `appservice::` / `operator::`）

- 通过 `ExecutorRegistry` 注册具名 adapter；每个 adapter 提供 `supports(executor)` + `invoke(executor, input) -> Value`。
- 主路径：`schedule_apply` 检测到 registry 命中 → 同步 `invoke` → 把结果回填 `node_outputs` → emit `step.completed` → 推进下游。
- 失败语义：adapter 返回 `WorkflowError` 即按节点的 retry / human fallback 处理；不直接抛崩 Run。
- registry 加载来源：服务启动时从 system_config + 内置编译期注册的 adapter 合并；语义链接 (`/agent/`、`/skill/`、`/tool/`) 在 P0 阶段直接由 registry 展开为实际 executor 定义。

### 6.2 与 Scheduler（Thunk 路径）

- 仅 `Apply` 解析为 FunctionObject (`func::*` / 后续 `optask::*`) 时使用。
- 投递时：workflow 在 task_manager 创建 Thunk 任务（挂在对应 Step 任务下），把 `thunk_obj_id` 与执行参数写入 task payload，再调用 `schedule_thunk(thunk_obj_id)`。
- 取结果时：**不**额外调用 workflow 的回执接口；scheduler 把 Thunk 任务的 status / progress / payload / error 写回 task_manager，workflow 通过订阅 / 轮询 task_manager 来推进主循环。
- 错误分类（`node_failure` / `execution_error` / `timeout` / `cancelled`）由 scheduler 写入 Thunk 任务的 error 字段；workflow 按 [wokflow engine.md §4.7] 的语义解释。
- 一期范围内 `func::*` 不作为 P0；服务必须保留这条通道但允许它处于“尚未接入具体 runner”的状态。

### 6.3 与 Task Manager（状态总账）

按 §0.1 的原则展开。Workflow service 把执行单元映射为 task_manager 中的一棵任务树：

| 层级 | task type | 何时创建 | 状态映射 | 备注 |
|------|-----------|----------|---------|------|
| Run | `workflow/run` | `create_run` 时一次 | `RunStatus` → `TaskStatus`（沿用 `task_tracker.rs::map_run_status`） | 现有实现 |
| Step | `workflow/step` | 节点首次进入 `Ready` 时创建 | `NodeRunState` → `TaskStatus` | 新增；TaskData 含 `node_id`、`attempt`、`executor`、`subject_obj_id`、`prompt`、`output_schema`、`stakeholders` |
| Map shard | `workflow/map_shard` | 进入 `enter_map` 后按需创建 | 同 Step | 新增；TaskData 含 `shard_index` |
| Thunk | `workflow/thunk` | 投递 Thunk 前创建 | scheduler 直接写 | 新增；TaskData 含 `thunk_obj_id` |

写入分工：

- **workflow 写**：建立任务树、写 task type、写描述性 TaskData（哪个节点、第几 attempt、subject 引用、prompt、output_schema、stakeholders）、写自己负责的状态镜像（Run / Step status）。
- **scheduler 写**：Thunk 任务的 status / progress / error / 结果 payload。workflow 只读，不抢写。
- **用户写**：根据 OS ACL 在 Step / Run task 的 TaskData 中写入 `human_action`（schema 见 §3.3）。workflow 订阅后解释并执行。

要点：

- **统一通过订阅 task_manager 推动状态机**：scheduler 改 Thunk status、用户改 Step TaskData、其它服务改 Map shard payload，统一表现为"task_manager 上的一次变更"，workflow 只有一种处理路径。
- **结构性 metadata 写在 TaskData 里**，让 task_manager 的 viewer 不依赖 workflow 也能给出有意义的信息；TaskMgr UI 据此渲染按钮 / 表单 / subject 预览。
- **失败降级**：task_manager 短暂不可用时，workflow 缓存 pending 的状态变更，恢复后批量补发；Thunk / 用户动作的感知会延迟，但不会丢失（scheduler 与用户都仍写到 task_manager）。
- **避免重复**：进度 / 错误 / payload 等 task_manager 已有的字段不再在 workflow 内复制；workflow 只持有 task_manager 不能表达的语义（节点拓扑、Reference 解析、Amendment 版本链、HumanAction 校验规则）。

### 6.4 与 Msg Center / TaskMgr UI（推送 + 主动发现 双通道）

整条链路：

1. **创建 task 即落 ACL**：workflow 在创建 Step / Run task 时，把 DSL 中声明的 stakeholders 写入该 task 的写权限列表。这是用户后续能写 `human_action` TaskData 的基础。
2. **进入等待态**：workflow 把 task 的 status 切到 `WaitingForApproval`，TaskData 中带上 `subject_obj_id` / `prompt` / `output_schema` / `waiting_human_since`。workflow 自己**不**调 Msg Center。
3. **推送通道**：task_manager（或 OS 通知中枢）检测到关键 status 变更后，构造 TaskMgr UI 的深链（如 `https://.../taskmgr/<task_id>`），经 Msg Center 推一条形如"有任务需要您确认 \<url\>"的消息给有权限的用户。
4. **主动发现通道**：即便用户错过推送，他打开 TaskMgr UI 时，按"我有写权限的待处理 task"过滤就能看到这条；TaskMgr UI 按 task type / TaskData 渲染合适按钮。
5. **回到 workflow**：用户点击按钮 = 写 TaskData 中的 `human_action`。workflow 通过订阅 task_manager 的 TaskData 变更接管处理，与 §3.3 一致。

要点：

- **ACL 在创建时一次性落定**，所有后续推送、列表、写入都基于 task_manager 的权限模型；workflow 不再做二次校验。
- **推送是通知，不是协议**。即使 Msg Center 不可用，task 仍处于 `WaitingForApproval`，主动发现通道照常工作；这也是 §1.2 把 Msg Center 列为软依赖的原因。
- **workflow 与 Msg Center 之间没有直接依赖**：通知链路完全由 task_manager / 通知中枢统一处置，workflow 退出这一面。

### 6.5 外部系统集成

- `POST /agent/task` 形态的入口（[wokflow engine.md §4.10.3]）作为 workflow service 的便捷封装：内部相当于 `submit_definition` + `create_run` + 注册 callback。
- `callback_url` 在 Run 终止时由 service 主动 POST。

## 7. 安全与多租户

| 维度 | 要求 |
|------|------|
| 身份 | 复用 buckyos-api 的 zone-internal kRPC 身份；外部入口经过 cyfs gateway 鉴权 |
| Owner | 每个 Definition / Run 带 `(user_id, app_id)`；list / get / mutate 一律按 owner 过滤 |
| ACL | Step / Run task 的 TaskData 写权限由 task_manager 校验；workflow 把 DSL 中声明的 stakeholders 同步到对应子任务的 ACL 上，不再在 workflow 自己实现一套 step-level ACL |
| 调度器身份 | scheduler 不直接调 workflow 的 RPC，但写 task_manager 中归属于本 Run 任务树的 Thunk task；ACL 由 task_manager 校验“写者必须是 owner 该子任务的服务身份” |
| Secrets | DSL 中不能出现明文 secret；`http::` adapter 的认证信息必须存放在 endpoint registry，按 endpoint id 引用 |
| 审计 | 所有 mutate API 写入事件流；事件流不可篡改（追加日志） |

## 8. 可观测性

- **任务树是首选可观测面**：进度、状态、子任务展开、错误信息、payload 都通过 task_manager 暴露，TaskMgr UI 直接复用，workflow 不再单独提供运行期 UI。
- **事件流补充任务树看不到的语义层迁移**：节点状态机内部转换、retry 内部决策、Amendment 审批轨迹等仍走 workflow 自己的事件流，由审计与回放消费。
- **指标**：每 Run 上报 token 用量、wall-clock、Step 失败率；服务级别上报 RPC QPS、registry 命中率、Thunk 投递延迟。
- **日志**：与现有 frame service 统一走 slog，结构化字段必须包含 `run_id` / `node_id` / `attempt`。
- **追踪**：Run-level trace span 在 `start_run` 创建，向下传递给 adapter / dispatcher，便于跨服务串链。

## 9. 一期范围（与 executor list 对齐）

| 项 | 一期目标 |
|----|---------|
| Definition CRUD + 静态分析 | 全量交付 |
| Run 生命周期 + Step 级 human action | 全量交付 |
| Executor Registry 接入 `service::` adapter | 全量交付（首批包括 `aicc.complete`、`msg_center.notify_user`、`system_config.*`、`task_manager.create_task`、`kb.*`） |
| `http::` / `appservice::` adapter | 框架就位 + 至少一个示例 endpoint 接入 |
| `/agent/` / `/skill/` / `/tool/` | 通过 executor registry 展开到 P0 实际 executor，不要求展开到 FunctionObject |
| `human_confirm` / `human_required` | 全量交付：写到 Step task 的 TaskData，由 TaskMgr UI 渲染交互、Msg Center 经 task_manager 推 URL；workflow 仅负责解释 TaskData 中的 `human_action` |
| `operator::` | 提供 `OperatorAdapter` 与最小 operator 表（`json.pick` / `list.sort` / `list.pluck` / `list.len` / `rank.topk` / `gen.ulid`），P1 |
| `func::*` / `optask::*` | 仅保留通道，不作为一期主路径 |
| Amendment | 接口与状态机就位；UI 协作可在 v0.2 完善 |
| 持久化与恢复 | 必须支持进程重启后从持久化恢复非终止态 Run |

后续版本（参考 [wokflow engine.md §7]）的演进重点：FunctionObject registry 接入、Stream output_mode 全功能、跨 zone Run、企业 IM 集成。

## 10. 验收标准

服务可视为达到一期可用，当且仅当下列条件全部满足：

1. 在测试 zone 内可以提交 KB 素材库导入示例（[wokflow engine.md §5]），全程不需要手工补 RPC，仅依靠服务自有 adapter。
2. 服务进程被强行 kill 后重启，所有 `Running` 状态 Run 能从最近一次落盘点继续，`idempotent: false` 步骤被冻结到 `WaitingHuman`。
3. 同一 Run 的 RPC mutate 调用与 task_manager 推送的 Step / Thunk 状态变更并发到达时，事件 seq 严格单调递增，外部订阅者可基于 `since_seq` 完整重放。
4. 用户**仅通过在 TaskMgr UI 中写 TaskData** 就能完成 approve / modify / reject / retry / skip / abort / rollback 端到端流程，无需调用 workflow 自己的 RPC；非法 TaskData 由 workflow 写回 `last_error` 让 UI 重新提示。
5. Workflow Definition 通过 `dry_run` 给出的 `AnalysisReport` 与正式 `submit_definition` 一致，避免 Agent 端用一个语义、服务端用另一个语义。
6. Task Manager / Msg Center 任一不可用时，Run 进入降级模式，但仍可继续推进其它能继续的节点；task_manager 恢复后能补齐期间缺失的状态写入。
7. 外部只用 `task_manager.get_task` 沿任务树向下钻取，能完整看到一个 Run 的所有 Step / Thunk / Map shard 的当前状态与终态，无需调用 workflow 的私有查询接口。
