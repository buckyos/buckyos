# 当前版本 TaskMgr 边界收敛与下一版本 Task Dispatch Center TODO

> **2026-08 更新：Task Dispatch Center 设计已定稿于 `doc/task_mgr/task_dispatch_center.md`，
> 并因 OpenDAN 外部委托依赖决定提前实施（不再绑定 Workflow 版本）。本文 §10 的待决策项
> 已全部在该文档 §12 决策记录中定案；§2～6 的设计草案以该文档为准。本文其余内容保留为
> beta2.2 TaskMgr 边界收敛的执行记录。**
>
> **2026-08-07 实施记录：M1+M2+M4 已落地。**
> - M1：`buckyos-api/src/task_dispatcher.rs`（协议 + client + run_target_instance SDK）、
>   `task_manager/src/dispatcher/`（独立 RDB `task-dispatcher-main`、指派式 evaluate_target、
>   offer lease/expiry timer、启动恢复、kevent 通知）、`/kapi/task-dispatcher` 挂载
>   （task-manager 进程 3380 第二 path；boot_gateway.yaml 加了 task-dispatcher→task-manager
>   路由别名）、scheduler `add_task_mgr` 追加第二个 rdb instance。当前有 25 个
>   task-manager Dispatcher 单测及 5 个 buckyos-api 协议模型单测，覆盖
>   解析/黏着/幂等/epoch/Uncertain/重启恢复/授权矩阵与协议序列化。
> - M2：OpenDAN `dispatch_adapter.rs`（register + attach/claim/accept 循环 + 文件化
>   `dispatch_id -> task_id` 幂等绑定 + create-then-crash 自愈扫描）；
>   `agent_task_executor` 伪 inbox 删除（owner-only sweep：app_id 过滤 +
>   `request.target_agent_id` 归属，全局 `/task_mgr/**` 订阅删除）；
>   `AgentDelegateTaskRequest` 增加 dispatch_id/target_agent_id/context_refs/constraints；
>   `--worksession-task-test` 直投入口删除。
> - 遗留：Control Panel 默认路由配置面与人工放行审批面 / WebUI Task Center dispatch 观察面 / websdk 封装 /
>   DV 环境故障注入（M3 的进程级验收）未做。
> - 同日增量：人工放行审批门（`PendingApproval` +
>   per-target `DispatchApprovalPolicy::{Never,InteractiveCallers,AllCallers}` +
>   `approve_dispatch`/`deny_dispatch`），解决低权限提交→高权限执行需显式人工决策；
>   设计并入 `doc/task_mgr/task_dispatch_center.md`（§7.1 汇总，§10 M4 实施清单），
>   **当日已实施（M4）**：
>   - 协议：`DispatchStatus::PendingApproval`（不进 `is_assignable`）、
>     `DispatchApprovalPolicy`（默认 `Never`）、`DispatchApproval`/`ApprovalDecision`、
>     `DispatchRejectReason::ApprovalDenied`、approve/deny RPC + client、
>     `/task_dispatcher/approvals` 通道 id。
>   - 存储：schema v1→v2（`TASK_DISPATCHER_RDB_SCHEMA_VERSION=2`，
>     `dispatch_record.approval` 列），旧库 open 时就地 `ALTER TABLE` 迁移
>     （DDL override 过期也兜底）；cancel/expire/due-deadline SQL 全部覆盖
>     `PendingApproval`；`approve_pending`/`deny_pending` 条件 UPDATE。
>   - 服务：落库按 policy + 直接调用者分级定初始状态；held 记录不评估、不
>     offer、不占并发、只发 approvals 提示（payload 仅 ids）；approve→Queued+
>     evaluate（幂等）；deny→Rejected(approval_denied) 终态（幂等）；提交者可
>     cancel 撤回；`expires_at` 覆盖审批等待；**accept/reject 服务层显式排除
>     `PendingApproval`（原 late-accept 绕过审批门的缺口已封）**，store 层条件
>     UPDATE 双重守卫；审批权 = `is_approval_admin`（zone 可信或 sudo 会话，
>     与 `InteractiveCallers` 豁免同一判定），`RequestContext` 新增 `sudo` 位。
>   - 可见性边界定案：门封死接收通道；zone 可信 get/list 只读可见性不收窄
>     （文档 §7.1/§12 已写明，读取面脱敏列后续扩展）。
>   - 测试：dispatcher 单测 25→31（hold/approve/deny/cancel/expire、豁免矩阵、
>     不占并发、审计链、approvals 通道、v1 就地迁移），buckyos-api 协议 5 项
>     扩展后全绿；OpenDAN `agent.delegate/v1` 注册显式 `approval_policy=Never`。
>   - 未做：Control Panel 审批面 UI（与默认路由配置面同属管理面遗留）。

> 背景：TaskMgr 原本是长任务、可恢复任务的统一基础设施，但当前 `runner`、
> `task_ready`、Pending 扫描等能力已经让它逐渐承担生产者消费者队列和跨服务调度职责。
> 这既模糊了 TaskMgr 与 Workflow 的边界，也形成了“低权限调用者构造 Task，诱导高权限
> runner 执行”的 confused deputy 风险。
>
> beta 2.2 是 breaking change，不保留旧 runner/dispatch 协议的兼容层。
>
> **版本边界：beta 2.2 只交付内核能力，Workflow 属于下一版本功能。** 因此本版本的
> 目标是收紧 TaskMgr 的职责和安全边界，而不是建设 Dispatcher，也不是把现有 TaskMgr
> 用户批量迁移到 Dispatcher。本文保留 Task Dispatch Center 的设计，作为下一版本
> Workflow 需要持久、异步、可离线交接工作时的后续方案。

## 当前实现进度核查（2026-08-07）

核查基线：当前仓库 `8ca8e55d`（`imporve task-dispatcher`，已包含同日实施的 M4
审批门）。进度口径以代码、测试和部署接线为准；“里程碑代码已合入”
不等于该里程碑的全部设计验收条件已经满足。

| 范围 | 当前状态 | 已核实实现 | 尚未闭环 |
| --- | --- | --- | --- |
| M1 Dispatcher 内核 | **主体已实现，验收部分完成** | 协议/client/Target SDK、独立 RDB、第二 RPC path、默认路由与 Target 黏着、集中指派、lease/expiry/Uncertain、启动恢复、kevent 加速 + sweep、scheduler/gateway 接线 | 管理权限仍只有粗粒度 `zone_trusted`；幂等重放未比较完整不可变信封；状态迁移与审计事件未原子提交；late accept/reject 的实例归属与状态守卫需收紧 |
| M2 OpenDAN Target | **接入主链已实现，幂等与业务鉴权部分完成** | `agent.delegate/v1` 注册、attach/renew/claim/accept 循环、owner-only recovery、旧 TaskMgr 伪 inbox 与直投测试入口删除 | 文件绑定不是与 Task 创建原子提交，且当前写法没有 fsync、损坏时从空状态继续；接收侧只校验 envelope 非空与 input schema，尚无可验证的原始授权证据/per-agent 业务策略；无 Adapter 直接单测和进程级故障注入 |
| M3 故障注入 | **未执行** | 协议级单测覆盖了部分状态迁移和重启恢复 | OpenDAN/Dispatcher/kevent/TaskMgr 多进程组合的离线、重启、ACK 丢失、崩溃窗口和暂停恢复尚未在 DV 环境验收 |
| M4 高权限实体人工放行 | **内核已实现（除管理 UI）** | 协议类型、schema v1→v2 就地迁移、hold/approve/deny/cancel/expire 服务逻辑、接收侧 `PendingApproval` 守卫、approvals 提示通道、sudo 审批权判定、6 项单测（含迁移测试） | Control Panel 审批面 UI；生产环境的细粒度 dispatcher 管理 capability 仍需从通用 `zone_trusted` 拆出。per-operation `approval_policy` 是后续扩展，不计入 v1 验收 |
| 调用方与观察面 | **未开始/未迁移** | Rust client 可供调用 | Workflow 尚未调用 Dispatcher，send-message 仍有 TaskMgr `list_tasks` 扫描；Control Panel 默认路由/审批面、WebUI Task Center dispatch 观察面和 websdk 均不存在 |

本轮实际验证：

- [x] `cargo test -p task_manager dispatcher:: -- --nocapture`：M4 落地后 31 passed
  （原 25 + 审批门 6），0 failed，40 filtered；有 2 个 `dispatcher/mod.rs` 未使用
  re-export warning。全量 `cargo test -p task_manager` 71 passed，0 failed。
- [x] `cargo test -p buckyos-api task_dispatcher::tests`：5 passed（已扩展覆盖
  PendingApproval round-trip / approval_policy 默认值 / approvals 通道 id），0 failed，
  122 filtered。
- [x] `cargo test -p opendan dispatch`：构建通过；lib 10 passed、0 failed、242 filtered，
  bin 0 passed、10 filtered；有 2 个 `main.rs` 未使用 import warning。当前没有
  `dispatch_adapter.rs` 自身的单元测试。
- [ ] 本轮未执行 workspace 全量 `cargo test`、`buckyos-build.py` 或 DV Test，不能沿用
  “全量构建/环境验收已通过”的口径。

### 设计与实现 review 后的闭环清单

P0（生产环境启用高权限 operation 前必须解决；M4 已于 2026-08-07 实施，其中
可见性边界项已随实施定案）：

- [ ] **定义 Target 二次业务鉴权的证据链。** 定稿设计要求 Target 对 envelope 再做完整
  业务鉴权，但当前 envelope 只有 Dispatcher 写入的身份快照，没有原 token、可验证授权
  引用或 Dispatcher 签名 attestation；OpenDAN 当前只检查 `on_behalf_of` 非空。需要明确
  Target 到底验证什么、信任谁，以及证据的过期/撤销语义。
- [ ] **把管理能力从 `zone_trusted` 中拆出。** 当前 `approve_dispatch`/`deny_dispatch`、
  `set/disable_operation_route`、`list/get_target`、`resolve_uncertain` 的管理档位仍过粗；
  zone 可信 Target owner 可能同时获得自我放行或修改默认路由的能力，与设计中的
  “Target owner 身份本身不含审批/路由管理权”不一致。应接 zone-owner/dispatcher-admin/
  system-config-admin capability，并补“Target owner 不能自批、不能改路由”的回归测试。
- [x] **重新定义 M4 的可见性边界。**（2026-08-07 定案并随 M4 实施）审批门只封
  **接收通道**：评估/offer/target 通知/claim/accept/reject 对未放行记录全部不可达；
  zone 可信调用者 get/list 的只读可见性遵循既有查询授权，不因审批门收窄——
  “executor 全程不可见”指接收循环，不指 list 查询。对 owner 隐藏未放行 input 属
  读取面脱敏，列为后续扩展。文档 §7.1 边界口径与 §12 决策记录已同步。
- [ ] **让 OpenDAN 的 `IdempotentAccept` 契约可证明。** 当前 JSON 文件 `write + rename`
  没有 fsync，解析失败会静默从空绑定开始，TaskMgr 建 Task 与落绑定也不在同一事务；
  owner-task 扫描只能缩小崩溃窗口，不能证明“最多一个 Task”。需要可原子声明
  `dispatch_id` 唯一性的 Target 本地存储/TaskMgr 幂等创建接口，或把注册契约降为 `None`。

P1（M1 完整验收前应解决）：

- [ ] idempotency replay 除 operation/requested target/input digest/on_behalf_of 外，还要比较
  `expires_at` 与 `workflow_ref`，否则同 key 的不同不可变信封会被静默当成同一请求。
- [ ] DispatchRecord 状态迁移与 `dispatch_event` 应在同一数据库事务提交；当前事件插入
  失败只记 warning，会留下无法完整审计的状态变更。
- [ ] 明确 late accept 的合法凭证并收紧状态守卫：当前任意有效同 Target instance 都可从
  `Queued`/`WaitingForTarget`/`Uncertain` accept，reject 也允许未 Offered 的记录；这会绕过
  集中指派、offer instance 和 capacity 约束。M4 不能只增加 `PendingApproval` 排除。
  （M4 已落地 `PendingApproval` 的服务层显式排除 + store 条件 UPDATE 双重守卫，
  审批门缺口已封；Queued/WaitingForTarget/Uncertain 的 late-accept 收紧仍未做）
- [x] M4 新增状态、策略与 `approval` 列时提升 Dispatcher schema version 并写迁移。
  （已做：`TASK_DISPATCHER_RDB_SCHEMA_VERSION` 1→2，`dispatch_record.approval` 列，
  open 时就地 `ALTER TABLE` 迁移并有 v1 库迁移单测；DDL override 过期时兜底）

## 0. 需要冻结的结论

### 0.1 TaskMgr 的第一范式

TaskMgr 为一个 `service.fun()` 内部实现长时间、可恢复执行提供统一基础设施。默认模型是：

```text
Service 收到经过认证和授权的业务请求
    -> Service 创建自己的 Task
    -> Service 执行、暂停、恢复自己的 Task
    -> TaskMgr 持久化状态、进度、checkpoint、事件和任务关系
```

- [x] TaskMgr 文档明确“谁创建，谁执行；谁拥有，谁更新”是默认范式。（doc/task_mgr/task_mgr.md §7 重写）
- [x] TaskMgr 只负责 Task 的状态、进度、checkpoint/data、事件、恢复信息和可观测性。
- [x] TaskMgr 默认不提供事务、分布式调度、生产者消费者队列或 exactly-once 语义。（runner/task_ready/按 runner 查询全部删除）
- [x] Task 的 parent/subtask 关系只表示业务关系，不自动赋予 TaskMgr 跨服务分发职责。
- [x] 普通 Service 默认只能创建和修改本 Service 拥有的 Task，不能指定另一个模块为执行者。（runner 字段已删；普通调用者身份强制=token，zone 可信服务可代填已鉴权业务 owner）

### 0.2 当前版本默认模式：业务接口 + Service 内部 TaskMgr

现有 TaskMgr 用户几乎都不应迁移到 Dispatcher。默认改造方式是由真正执行工作的
Service 提供明确的业务功能接口，并在接口内部创建、执行和恢复自己的 Task：

```text
Caller
    -> TargetService.feature_api(request)
    -> TargetService 完成认证、授权和业务参数校验
    -> TargetService 创建自己拥有的 Task
    -> TargetService 在内部执行或恢复该 Task
    -> 返回 task_id / status handle
```

- [x] 跨 Service 调用必须使用明确的业务 operation，例如 `apps.install`、
  `execute_thunk`，不能继续暴露 `runner + task_type + data` 形式的通用投递入口。（runner 通用投递协议已删）
- [x] 业务接口可以返回 `accepted + task_id` 后异步执行；“异步”本身不构成引入 Dispatcher 的理由。
- [x] Target Service 可以在内部扫描和恢复**自己拥有的 Task**，但不能跨进程扫描其他 owner
  创建的 Task，也不能把这种内部恢复查询重新包装成公共队列接口。（各消费者改按自身 task_type + 本地归属过滤）
- [x] Caller 不预先替 Target 创建 Task，不指定 Target 的 runner/owner，也不从 payload 构造
  高权限身份或业务选项。（身份来自验签 token；普通调用者代填 owner 被拒绝）
- [x] Task 的取消、重试、审批等业务动作通过 Target Service 的功能接口完成；只有 Task owner
  可以直接更新对应 Task。（scope 检查收紧，空 ctx 放行已删除）
- [x] 本版本不得要求普通 Service 注册 Dispatch Target，也不得依赖 Dispatcher 的启动或可用性。
  （Dispatcher 后来提前实现为 task-manager 的可选第二服务面；启动失败时 TaskMgr 继续运行，
  只有 OpenDAN 外部结构化委托这一真实 Target 依赖它）

OpenDAN Agent Task Executor 是少数例外：Agent 的核心职责就是接收外部委托，并且需要 Target
离线等待、capacity、claim/lease、ACK 丢失恢复和幂等交接，因此它属于真正的 Dispatch
Target。原计划是 beta 2.2 只删除伪 inbox、下一版本再接 Dispatcher；实际因外部委托能力
空档已提前实现为通过 Dispatcher 投递 `agent.delegate/v1`，OpenDAN 接受后再创建并执行
自己的 Task。

### 0.3 下一版本：Dispatch 是 Workflow 所需的独立语义

当下一版本 Workflow 需要在 Target 离线时仍持久保留请求，并支持异步领取、lease、ACK
丢失恢复和幂等交接时，“把一项工作交给某个负责人”才成为独立的 Dispatch 语义，应由
**Task Dispatch Center** 管理：

```text
Workflow
    -> Task Dispatch Center.dispatch(target, operation, input)
    -> 已注册的 Target 接收 DispatchRecord
    -> Target 重新鉴权并幂等创建自己的 Task
    -> Target.accept(dispatch_id, target_task_id)
```

普通的在线 Service 调用，只要 Target 能通过业务 RPC 接受请求并返回 `task_id`，就继续采用
0.2 的默认模式，不经过 Dispatcher。Dispatcher 不是 Service API gateway，也不是所有
TaskMgr 用户的新入口。

- [x] Dispatch Center 放入 `task-manager` 的进程/部署单元，降低依赖和运维复杂度。（同进程 3380 第二 path，dispatcher 启动失败不影响 TaskMgr）
- [x] TaskMgr 不依赖 Workflow；Workflow 只作为 Dispatch Center 的调用者。
- [x] “同进程部署”不等于“同一个抽象”：Task Store 与 Dispatch Store 必须保持独立。（独立 RDB instance `task-dispatcher-main`，无共享表/schema/join）
- [ ] TaskMgr 权限不自动包含 Dispatch Center 权限，两者使用独立 RPC 路径和授权策略。
  （独立 path + fail-closed 验签已完成；Target owner 分面已完成，但 route/admin 目前都只
  检查 `zone_trusted`，尚未接独立管理 capability，不能算完整分面授权）
- [x] Dispatch Center 不执行业务，也不成为目标 Task 执行状态的第二真相源。（Accepted 终态，只保留 target_task_id link）

建议部署关系：

```text
task-manager process
├── /kapi/task-manager       -> Task Service
│   └── task / task_note
└── /kapi/task-dispatcher    -> Task Dispatch Center（暂定名称）
    └── dispatch_target / dispatch_instance / dispatch_record / dispatch_audit

workflow process
├── Workflow Orchestrator
├── Workflow 本地 ExecutorAdapter
└── Dispatch Center Client（仅外部异步 target 使用）
```

## 1. 当前问题与安全风险

### P0：确认并封住当前入口

当前 TaskMgr 的通用 Task 协议包含 `runner`，Task Manager 会发布
`/task_mgr/runner/{runner}/task_ready`；部分执行器还会定期查询指定 runner 的非终态或
Pending Task。这已经等价于一个缺少严格队列边界的生产者消费者模型。

- [x] 形成一份当前 runner 使用清单，至少覆盖：（见本文末尾"beta2.2 收敛记录"）
  - Control Panel App Installer。
  - Workflow scheduled task / send-message executor。
  - Node Daemon Node Executor。
  - OpenDAN Agent Task Executor。
- [x] 为每个入口记录：Task 创建者、Task 所有者、runner 身份、执行身份、鉴权位置和恢复方式。（见"beta2.2 收敛记录"）
- [x] 增加安全回归测试：仅调用 TaskMgr `create_task`，即使提供合法的
  `runner = app.control_panel`、`task_type = app.install` 和格式正确的 data，也不能触发安装。
  （task_manager server 测试 `test_create_task_with_runner_payload_gets_no_dispatch_semantics` 等）
- [x] 检查 `SYSTEM_INTERNAL + auto_confirm` 等高权限业务选项，确保只能由可信业务入口构造，
  不能从 Task data 直接获得权限提升。（安装路径只认 Control Panel 业务接口创建+进程内 dispatch 的任务；TaskMgr 直建的 app.install 数据无人消费）
- [x] `user_id`、`app_id`、creator/owner 等身份字段必须来自认证上下文，不能信任请求 payload。
  （服务端 fail-closed 验签；普通调用者身份强制=token，zone 可信服务代填已鉴权业务 owner 时才允许不同值）

本版本对现有入口的默认收敛方向是：

- Control Panel App Installer：保留 `apps.*` 等已鉴权业务接口，由 Control Panel 内部创建并恢复安装 Task。
- Node Daemon Node Executor：提供显式的 Node Daemon 功能接口，由 Node Daemon 创建并恢复自己的 Task。
- OpenDAN Agent Task Executor：当前版本删除基于 TaskMgr 扫描的伪 Dispatch，只保留 OpenDAN
  内部创建和恢复的 Task；下一版本作为真正的 Agent Dispatch Target 接收外部委托。
- Workflow scheduled task / send-message：属于下一版本 Workflow 范围；现存代码如需保留，只能使用
  本地 adapter 或目标 Service 的显式业务接口，不能成为当前内核对 Dispatcher 的依赖。

关键安全原则：

```text
能够访问 TaskMgr
!= 能够向任意 Service 投递工作
!= 能够要求高权限 Service 代为执行
```

## 2. 下一版本 Dispatch Center 的职责边界

第 2～6 节描述下一版本 Workflow 所需的 Dispatch Center，不是 beta 2.2 的交付项或现有
TaskMgr 用户的迁移清单。2026-08-07 Dispatcher 提前实现后，本节同时作为实现核对表：
`[x]` 表示 `src/kernel/task_manager/src/dispatcher/` 或共享协议已有代码与单测依据；跨
Workflow/OpenDAN/RBAC 的端到端条件仍保持未勾选，并在条目后注明已完成的内核部分。

### 2.0 使用门槛

只有普通业务 RPC 无法覆盖，并且确实需要以下能力时，才应引入 Dispatcher：

- [x] Caller 提交后，即使 Target 当前离线，请求也由 Dispatcher 独立持久保留。
  （无实例时 `WaitingForTarget`，启动恢复与 Target 上线后重评估已有单测）
- [x] Target 的一个或多个实例异步 claim，并受 lease、capacity、instance epoch 约束。
- [x] Dispatcher 内核处理 offer/accept ACK 丢失、交接重放和 `dispatch_id` 级幂等。
  （同 `dispatch_id` late/replayed accept、offer lease redelivery、`IdempotencyContract::None`
  进入 `Uncertain` 已实现；Target 端“最多一个业务 Task”仍见 §4.4 未完成项）
- [ ] Workflow 需要独立审计“谁把什么工作交给了哪个 Target”，且该交接生命周期独立于业务 Task。
  （Dispatcher 已持久化 auth/workflow_ref 与 dispatch_event；Workflow 尚未接入，且状态迁移与
  audit event 尚未原子提交）
- [x] 低权限主体可以提交、但不能自动触发高权限 executor 时，由高权限管理实体在分发前
  人工放行或拒绝。（`PendingApproval` + `approve_dispatch`/`deny_dispatch` 已实现）

以下情况不使用 Dispatcher：在线 Target 的普通业务调用、Service 自己的后台任务、Service
内部 Task 恢复、Task 状态查询，以及通过 owner 业务 API 完成的取消/重试/执行中审批。
这里的“执行中审批”与 Dispatcher 在 offer 前的人工放行门是两层语义，不能互相替代。

### 2.1 应负责

- [x] 持久化 Target/Capability 注册信息。（`dispatch_target.registration` JSON + owner/enabled 列）
- [x] 管理 Target 在线实例、lease、capacity 和实例 epoch。
- [x] 持久化 DispatchRecord，并管理 offer/claim/accept/reject/cancel/expiry 状态机。
- [x] Target 离线时保留等待记录；Target 上线或恢复后唤醒对应记录。
- [x] 提供最小投递审计：auth envelope 记录谁、代表谁、Workflow/Run/Step 和目标操作，
  `dispatch_event` 记录状态迁移。（事件与状态变更非同一事务，完整可靠性仍见文首 P1）
- [x] 管理“接收前”的 delivery retry，并用 Target 的 `IdempotencyContract` 决定自动重投或
  进入 `Uncertain`。（不重复业务 Task 仍依赖 Target 侧契约）
- [x] 在 `Accepted` 后保存 `target_task_id`，供调用方建立关联和继续观察。
- [x] 按 Target 的 `approval_policy` 把需要人工决策的记录停在 `PendingApproval`，只允许
  高权限管理实体 approve/deny，并持久化审批人、决定、时间和备注。

### 2.2 不应负责

- [x] 不保存业务 Task 的完整进度、checkpoint 或输出。
- [x] 不代替 Target 做业务鉴权。（只做 Dispatcher ACL；Target 二次鉴权证据链仍待定义）
- [x] 不把业务执行失败自动解释成需要重新 dispatch。
- [x] 不实现 Workflow DSL、条件分支、补偿或 schedule 语义。
- [x] 不承担通用节点资源放置；资源调度仍属于 Scheduler。
- [x] 不承诺通用分布式事务或 exactly-once 业务执行。
- [x] 不把人工放行扩展成多级审批工作流，也不允许审批人借 approve 改选 Target 或实例；
  放行后仍由 `evaluate_target` 按集中策略指派。

### 2.3 Workflow 中保留的能力

现有 `ExecutorAdapter` / `ExecutorRegistry` 可作为调用模型的起点，但要区分两类 executor：

- [ ] `operator::`、人工等待和 Workflow 进程内 adapter 继续由 Workflow 自己管理。
- [ ] 能同步完成的 `service::` / `http::` / `func::` adapter 可继续直接调用。
- [ ] 需要离线等待、异步领取、跨进程恢复的 target 改为调用 Dispatch Center。
- [x] 共享 Dispatch 协议类型放到 `buckyos-api`，TaskMgr/Dispatcher 未反向依赖 Workflow crate。

## 3. Target 注册模型

### 3.1 持久 TargetRegistration

建议注册的不是任意字符串 runner，而是有所有权和能力约束的 Target：

```rust
pub struct TargetRegistration {
    pub target_id: String,
    pub owner_app_id: String,
    pub owner_did: String,
    pub operations: Vec<OperationDescriptor>,
    pub auth_policy: AuthPolicyRef,
    pub approval_policy: DispatchApprovalPolicy,
    pub idempotency_contract: IdempotencyContract,
    pub delivery_policy: DeliveryPolicy,
    pub max_concurrency: u32,
}
```

- [ ] 系统 Target 的 capability 以签名 ServiceDoc/system-config 等可信配置为真相源。
- [x] 动态实例不能自行声明超出 TargetRegistration 的 operation 或权限。
  （attach 只接收 `target_id + capacity`，operation 清单只来自持久注册）
- [x] 注册和更新绑定调用者的已认证 Service 身份，禁止更新其他 owner 已注册的 target。
  （owner 字段由验签身份覆盖；首次注册 target_id 的外部可信来源仍是上一条未完成项）
- [ ] operation 包含版本化 input/output schema，不再接受任意 `task_type + data`。
  （operation 名称当前只强制包含 `/`，约定使用 `name/vN`，并注册为封闭清单；但
  schema_ref 仍可选、无 output schema，Dispatcher 不执行 schema 校验）
- [x] 未注册、未启用或不支持 operation 的 target dispatch 立即失败，不落等待记录。
- [x] `auth_policy` 先判断调用者能否提交，`approval_policy` 再决定已受理请求是否进入
  `PendingApproval`；支持 `Never`、`InteractiveCallers`、`AllCallers`，默认 `Never`。
  （v1 为 per-target；per-operation 覆盖是后续扩展，不属于当前验收缺口）

### 3.2 临时 TargetInstance

```rust
pub struct TargetInstance {
    pub target_id: String,
    pub instance_id: String,
    pub lease_epoch: u64,
    pub lease_expires_at: u64,
    pub capacity: u32,
    pub endpoint_or_session: Option<String>,
}
```

- [x] Target 实例上线时 register/attach，在线期间 renew lease，下线或过期后不再接收 offer。
- [x] 同一 Target 多实例支持 `RoundRobin`（默认）或 `LeastLoaded` 集中指派。
- [x] instance epoch 防止旧连接或暂停后恢复的实例继续 claim/accept/reject 新记录。
- [x] Target 注册、实例 attach/renew/detach、accept/reject 释放 capacity 都会触发
  `evaluate_target(target_id)`，重评估该 Target 的等待记录。

## 4. Dispatch 协议与状态机

### 4.1 请求信封

建议使用不可变、可审计的请求信封：

```rust
pub struct DispatchRequest {
    pub target_id: String,
    pub operation: String,
    pub schema_version: u32,
    pub input: serde_json::Value,
    pub idempotency_key: String,
    pub expires_at: Option<u64>,
    pub workflow_ref: Option<WorkflowStepRef>,
    pub auth: DispatchAuthEnvelope,
}
```

`DispatchAuthEnvelope` 至少绑定：

- `requested_by_user` / `requested_by_app`。
- `on_behalf_of`。
- Workflow、Run、Step 标识。
- 原始授权证据或可验证引用。
- input digest、schema version、创建时间和过期时间。

- [ ] Workflow 只能传递已有身份和授权证据，不能把普通用户请求“洗”为 system 身份。
  （Dispatcher 已拒绝普通调用者代填 `on_behalf_of`；原始授权证据/可验证引用尚未进入 envelope）
- [ ] Dispatch Center 校验 target/operation ACL；Target 接收时再次执行完整业务鉴权。
  （Dispatcher ACL 已完成；Target 二次业务鉴权的证据链和 OpenDAN policy 未完成）
- [ ] 请求信封创建后不可修改；重试复用同一个 `dispatch_id` 和 idempotency key。
  （记录不可变与同 key 返回原 dispatch 已实现；replay conflict 尚未比较 `expires_at`、
  `workflow_ref`，因此完整信封验收未过）
- [x] 人工 approve/deny 不能修改 `requested_by_*`、`on_behalf_of`、Target、operation 或 input；
  审批结果独立写入 `approval` 和 `dispatch_event`，放行不等于提权或免除 Target 二次鉴权。

### 4.2 独立状态机

Dispatch 状态不要复用 `TaskStatus`：

```text
dispatch 落库
  -> PendingApproval                    # approval_policy 命中
  -> Queued                             # 无需人工放行

PendingApproval
  -> Queued                             # 高权限实体 approve，随后集中指派
  -> Rejected(approval_denied)          # 高权限实体 deny
  -> Expired | Canceled

Queued
  -> WaitingForTarget
  -> Offered
  -> Accepted(target_id, instance_id, target_task_id)

Queued / WaitingForTarget / Offered
  -> Rejected | Expired | Canceled | Uncertain
```

- [x] `WaitingForTarget` 表示 target 已注册但当前没有可用实例/capacity。
- [x] `Offered` 带 offer lease；超时后按同一 dispatch 重新 offer，并受 delivery 次数上限约束。
- [x] `Accepted` 是 Dispatch Center 的正常终态，不继续复制目标 Task 的完整状态机。
- [x] `Rejected` 区分 schema/auth/policy/business-precondition 等稳定拒绝原因。
- [x] `Uncertain` 表示 Target 可能已经创建 Task 但接收确认丢失，禁止自动重投；由
  `resolve_uncertain` 或受控 late accept 收敛。
- [x] `PendingApproval` 是指派前状态：不进入 `evaluate_target`、不产生 Target 通知/offer、
  不占 capacity；Target 的 claim/accept/reject 全部拒绝，提交者仍可 cancel，`expires_at`
  仍可把记录转为 `Expired`。

### 4.3 高权限实体人工放行

- [x] `DispatchApprovalPolicy::InteractiveCallers` 只 hold 非 sudo 交互会话；zone 可信调用者
  和 sudo 会话直接通过。`AllCallers` 连 zone 可信/Agent 自主提交也 hold。
- [x] `approve_dispatch`/`deny_dispatch` 只接受高权限管理身份；审批人身份来自验签 token，
  不接受 payload 代填，普通提交者不会因为拥有记录而取得审批权。
- [x] approve 幂等地执行 `PendingApproval -> Queued` 并触发 `evaluate_target`；deny 幂等地
  执行 `PendingApproval -> Rejected(approval_denied)`，再次申请必须使用新 idempotency key。
- [x] 审批决定和 note 写入 `DispatchApproval` 与 `dispatch_event`；原 auth envelope 保持不变。
- [x] 该接口是“是否允许进入自动指派”的人工决策，不是手工挑选后端/实例：approve 不接受
  `target_id`/`instance_id`，不破坏 Target 黏着；换后端必须结束原记录后创建新 Dispatch。
- [ ] Control Panel 提供待审批列表、approve/deny 操作和审计展示；内核提供
  `/task_dispatcher/approvals` 提示与 `list_dispatches(status=PendingApproval)` 真相查询，UI 未做。
- [ ] 审批/路由管理权接入细粒度 dispatcher-admin capability；当前实现档位仍是 zone 可信或
  sudo 会话，不能证明普通 Target owner 一定无自批权限。

### 4.4 接收幂等契约

Target 接收必须满足：

```text
相同 dispatch_id 重放 N 次
    -> 返回相同 target_task_id
    -> 最多创建一个 Target 自己拥有的 Task
```

- [ ] Target 在自己的持久存储中原子保存 `dispatch_id -> target_task_id` 绑定。
- [ ] Target 先重新鉴权，再幂等创建自己的 Task，然后返回 `accept`。
- [ ] Task 已创建但 ACK 丢失时，重新投递返回原 `target_task_id`。
  （Dispatcher 支持相同 task_id 的 accept replay；OpenDAN 绑定持久性的端到端验收未过）
- [x] 无法满足幂等接收契约的 Target 声明 `IdempotencyContract::None` 后不自动重新 offer；
  offer lease 过期进入 `Uncertain`，由管理/业务恢复。
- [x] 区分 delivery retry 与 business retry：前者只发生在 `Accepted` 之前且复用
  `dispatch_id`；Dispatcher 在 `Accepted` 后不观察或重启业务 Task。

## 5. 离线领取与内部扫描

Target 离线时推荐流程：

```text
1. Caller 创建持久 DispatchRecord
2. approval_policy 命中 -> PendingApproval
3. 高权限管理实体 approve -> Queued；或 deny -> Rejected 终止
4. 无需审批/审批通过后，没有在线实例 -> WaitingForTarget
5. TargetInstance register/renew
6. Dispatch Center 唤醒该 target 的 due records
7. Target 通过 stream/long-poll/KEvent 收到通知并 claim_next
8. Target 鉴权并创建自己的 Task
9. Target accept(dispatch_id, target_task_id)
```

- [x] “查询尚未 dispatch 的记录”只存在于 Dispatch Store 内部，不暴露为通用 TaskMgr 接口。
- [x] Dispatch Center 在单一权威组件内扫描 DispatchRecord 做恢复，不让多个服务跨进程扫 TaskMgr。
- [x] 正常路径由 record insert、target register、instance/capacity 变化直接触发定向
  `evaluate_target`；KEvent 只负责加速 Target 拉取。
- [x] 使用 earliest-deadline timer 处理 offer lease、请求 expiry 和 instance lease，并在
  启动时恢复扫描。（当前通过索引 + SQL `MIN` 计算最近 deadline，不维护内存最小堆）
- [x] 保留最长 60s 的低频 maintenance sweep 作为丢通知和 timer 漂移兜底，未做高频全表扫描。
- [x] Target 接收使用定向 KEvent + `claim_next(target_id, instance_id, lease_epoch)`，不再以
  `TaskMgr.list_tasks` 作为外部工作 inbox。（OpenDAN 仅为自身幂等崩溃恢复扫描 own tasks）
- [x] 待人工放行记录只通知 `/task_dispatcher/approvals` 管理通道，且 payload 仅含 ids；
  Target 通道不通知。审批队列以 `list_dispatches(status=PendingApproval)` 为真相源。

## 6. Task 树与跨所有者关联

Workflow Task 与 Target Task 属于不同所有者，不应直接依赖可写的跨所有者 `parent_id`：

```text
Workflow-owned step/mirror task
    ├── dispatch_id
    └── target_task_id --------------> Target-owned execution task
```

- [ ] Workflow 保留自己拥有的 step/mirror Task，用于流程树展示和恢复。
- [x] 首个 Target OpenDAN 接收后创建自己 app/业务用户名下的 execution Task。
- [x] Dispatcher 通过不可变 `dispatch_id + target_task_id` link 建立关联，不共享 Task 写权限、
  不复制目标 Task 状态。
- [x] 当前实现未引入跨所有者 parent/subtask 或伪造 owner/parent；如未来需要仍须另行设计
  受限 attach capability。

## 7. 实施阶段

### 当前版本 P0：边界与紧急安全收口

- [x] 更新 TaskMgr/Workflow/App Install 设计文档，冻结本文职责边界。
- [x] TaskMgr 创建 Task 时从认证上下文确定 creator、owner、user/app，不接受 payload 覆盖。
  （普通调用者 payload 与 token 不符即拒绝；zone 可信服务代填视为已鉴权业务身份）
- [x] 默认拒绝调用者创建其他 Service/owner 的 Task。
- [x] App Installer 只接受 Control Panel 已鉴权业务接口创建的安装 Task；此路径不等待 Dispatcher。
- [x] 为当前 runner 入口补充权限测试，证明不能通过 TaskMgr 直接触发高权限行为。

### 当前版本 P1：现有 TaskMgr 用户收敛到业务接口

- [x] Control Panel 移除通用 runner inbox；已有业务 RPC 在鉴权后创建并执行 Control Panel 自己的 Task。
  （task_ready kevent 订阅删除；list_active 改按 task_type=app.install/app.update；MsgQueue+启动扫描+sweep 保留）
- [x] Node Executor 移除跨 owner 的 TaskMgr runner 扫描。（node_executor.rs 为从未接入主流程且无生产者的死代码，整体删除；未来节点执行走 Node Daemon 显式接口再立项）
- [x] OpenDAN Agent Task Executor 移除通用 runner inbox；此前“按 `task_type=agent.delegate`
  扫描并以 data 内 `progress.execution.runner` 判定归属”的实现仍是在 TaskMgr 上模拟 Dispatch，
  应在 beta 2.2 删除或禁用。当前版本只恢复 OpenDAN 自己创建的 Task，外部委托留待下一版本 Dispatcher。
  （已删除：sweep 加 app_id owner 过滤 + `request.target_agent_id` 归属，全局 `/task_mgr/**`
  订阅删除；外部委托已直接接入提前实施的 Dispatcher，见文首实施记录）
- [x] Workflow 相关现存调用使用本地 adapter 或目标 Service 的业务接口；本版本不为它们引入 Dispatcher 依赖。
  （send_message executor 按自身 task_type 扫描 + schedule owner 一致性校验；schedule 模板删除 runner 通用投递参数；create_fire_subtask 删除身份降级重试）
- [x] 调用方可以只读观察目标 Task；取消、重试、审批等写操作仍调用 Task owner 的业务接口。
- [x] 逐项记录替代接口、Task owner、鉴权位置和 Service 内部恢复方式，确认没有把 runner
  换名后继续作为通用投递参数。（schedule target/模板/CLI 的 runner 字段全部删除；执行者信息只存在于各业务 data schema）

### 当前版本 P2：删除 TaskMgr 的调度语义

- [x] 从通用 Task API 删除或严格私有化 `runner` 字段。（决策：彻底删除，含 DB 列与索引，schema v5→v6 自动迁移）
- [x] 删除 `/task_mgr/runner/{runner}/task_ready`。
- [x] 删除按 runner 扫 Pending、claim/lease 等面向消费者的 TaskMgr API。（TaskFilter.runner、list_tasks 的 source_user_id/source_app_id 身份声明参数一并删除）
- [x] 删除遗留执行循环、索引、配置、文档和测试。（node_executor.rs、runner 单测、DV-07 重写为 task-changed 验收）
- [x] 更新 Task Center/UI，使其只表达 Task 执行状态；不预埋对 Dispatcher 的运行时依赖。
  （desktop task_mgr.ts/mock/ScheduledTasksPage 移除 runner 引用）

完成当前版本不以 Dispatcher 实现、Target 注册或 Workflow 迁移为前置条件。

### 下一版本 F1：Dispatcher 共享协议与存储（已提前实施，2026-08-07）

- [x] 在 `buckyos-api` 增加 DispatchRequest、DispatchRecord、TargetRegistration、TargetInstance 和客户端类型。（task_dispatcher.rs）
- [x] 在 `task_manager` crate 内新增独立 dispatcher module、store、handler 和恢复循环。（dispatcher/）
- [ ] 使用独立 RPC path、表、索引和权限配置；可以复用同一个进程/RDB 实例。
  （path/表/索引/独立 RDB instance 已完成；owner 权限已分面，但 route/admin 仍共用
  `zone_trusted` 粗粒度门，独立管理 capability 未完成）
- [x] 完成 dispatch/claim/accept/reject/cancel/renew/heartbeat/approve/deny 的协议级单元测试。
  （dispatcher/tests.rs 当前 31 项：M1 25 + M4 6；另有 buckyos-api 协议模型 5 项）

### M4：高权限实体人工放行（已提前实施，2026-08-07）

- [x] 增加 `PendingApproval`、`DispatchApprovalPolicy`、`DispatchApproval`、
  `ApprovalDenied` 以及 approve/deny RPC/client。
- [x] Dispatcher schema v1→v2，持久化审批决定；旧库支持就地迁移并有迁移单测。
- [x] policy 命中时在指派前 hold；approve 后进入集中指派，deny/cancel/expire 正确终结，
  Target 无法通过 claim/accept/reject 绕过审批门。
- [x] 审批身份来自验签 token，auth envelope 不变；审批记录与状态事件可查询。
- [ ] Control Panel 实现审批队列、操作入口和审计展示。
- [ ] 把 approve/deny 权限从通用 `zone_trusted` 收紧为明确的 dispatcher-admin capability。

### 下一版本 F2：Workflow 试点与迁移

- [x] 以 OpenDAN Agent Task Executor 作为首个真正的 Dispatch Target：Agent 本身就是接收任务的
  主力，并且天然需要离线等待、能力约束和幂等交接；App Install 不作为首个试点。（dispatch_adapter.rs）
- [ ] 落实注册来源、实例 lease、幂等接收和离线恢复。
  （注册/lease/owner-only recovery 主链已完成；Target 注册来源未由 Dispatcher 验证，
  OpenDAN 文件绑定与 Task 创建非原子且没有 fsync，尚未达到 `IdempotentAccept` 验收口径）
- [ ] 验证 Workflow 重启、Target 重启、ACK 丢失、offer 超时和重复投递。（协议级路径已有单测覆盖；
  Workflow 调用方与 DV 环境级故障注入待 Workflow 版本）
- [ ] Workflow scheduled task 不再通过 TaskMgr 创建任意 runner Task；改为 dispatch 或直接调用本地 adapter。
- [ ] Workflow send-message executor 移除 TaskMgr Pending 轮询。
- [ ] 仅当某项 Workflow operation 满足 2.0 的使用门槛时，才让对应 Service 额外注册为
  Dispatch Target；Service 原有业务接口和内部 Task 所有权模型保持不变。
- [ ] 只有 Workflow 确实需要离线、持久地发起安装时，才为 Control Panel 增加受限、显式
  operation 的 Dispatch Target；不能为了统一形式迁移 App Installer。

## 8. 验收条件

### 8.1 当前版本

- [x] 调用者不能通过 TaskMgr 构造 `app.install` Task 触发 Control Panel 安装。
- [x] 任何 Service 都不再跨进程轮询 TaskMgr 领取另一个模块创建的 Task。
- [x] 除真正的 Dispatch Target 外，原 runner 消费者都有显式、可鉴权的业务功能接口，并由目标 Service 创建、执行和恢复自己的 Task。
- [x] OpenDAN 当前版本不再通过 TaskMgr 全局扫描接收外部 `agent.delegate`；外部委托已改经提前实施的 Dispatcher 接收。
- [x] 异步业务接口可直接返回 `task_id`，不需要通过 Dispatcher 才能完成长任务。
- [x] Dispatcher 完全不存在或未启动时，当前内核及上述业务接口仍能正常工作。
- [x] TaskMgr 对 Workflow/Dispatcher 没有 crate、RPC 启动顺序或运行时强依赖。
- [x] `cargo test`、BuckyOS Rust 构建通过；DV-07 已按新契约重写（待环境跑通）。

### 8.2 下一版本 Dispatcher（核心协议单测通过，完整验收未过）

- [x] 未注册 Target 的 dispatch 被明确拒绝。（dispatch_requires_registered_enabled_supporting_target）
- [ ] Target 离线时记录持久等待，上线后能领取且不会生成重复业务 Task。
  （Dispatcher 等待/恢复已过单测；OpenDAN “不重复建 Task”缺少原子唯一约束、Adapter
  单测与进程故障注入）
- [ ] 相同 `dispatch_id` 重放返回同一个 `target_task_id`。
  （Dispatcher accept replay 已过单测；OpenDAN 文件绑定的崩溃/损坏路径未达到可证明口径）
- [ ] 低权限调用者不能经 Workflow/Dispatcher 触发高权限 operation。
  （`ZoneTrustedOnly` 与 `on_behalf_of` 反洗白已过单测；M4 审批门已实施并过单测——
  `ZoneUsers + InteractiveCallers/AllCallers` 可拦低权限直触；尚未定义证据链的
  Target 二次业务鉴权仍未闭环，故整条验收保持未勾）
- [x] `approval_policy` 命中的记录停在 `PendingApproval`：不评估、不产生 Target 通知/offer、
  不占并发，Target owner 不能经 claim/accept/reject 绕过；cancel/expiry 正常收敛。
- [x] 高权限管理身份可以幂等 approve/deny，审批身份和备注可审计；普通交互调用者不能审批。
- [x] approve 不改变 auth envelope、Target 或实例，只把记录放入既有集中指派路径；
  `Rejected(approval_denied)` 不能复活，重新申请需要新 idempotency key。
- [ ] 生产环境证明 Target owner 仅凭 owner 身份不能自批；当前 `zone_trusted` 管理档位尚未
  接细粒度 dispatcher-admin capability，因此权限隔离的完整验收未过。
- [ ] Task Service 与 Dispatch Center 即使同进程，也有独立的数据模型、RPC 和授权规则。
  （数据模型/RPC 已独立；route/admin capability 尚未与通用 `zone_trusted` 分离）
- [x] delivery retry 和 business retry 有可测试的明确分界。（Accepted 终态后 maintenance 不再触碰；重投只发生在 Accepted 前且复用 dispatch_id）

## 9. 预计影响入口

当前版本：

- `src/kernel/task_manager/src/server.rs`
- `src/kernel/task_manager/src/task_db.rs`
- `src/kernel/buckyos-api/src/task_mgr.rs`
- `src/kernel/buckyos-api/src/taskdata.rs`
- `src/kernel/node_daemon/src/node_executor.rs`
- `src/frame/opendan/src/agent_task_executor.rs`
- `src/frame/control_panel/src/app_install_runner.rs`
- `src/frame/control_panel/src/app_install_engine.rs`
- `doc/task_mgr/**`

下一版本 Workflow/Dispatcher：

- `src/kernel/workflow/src/executor_adapter.rs`
- `src/kernel/workflow/src/scheduled_task_manager.rs`
- `src/kernel/workflow/src/send_message_executor.rs`
- `doc/workflow/**`

改协议、字段、权限和存储结构时，需要同步检查 ServiceDoc/system-config、RBAC、SDK、WebUI 和 DV Test。

## 10. 待决策项

当前版本决定（已执行）：`runner` **彻底删除**（字段、DB 列、索引、查询、事件全删）。
理由：beta2.2 是 breaking change 版本；没有任何 Service 需要它作为私有字段（执行者信息
已在各业务 data schema，如 OpenDAN 的 `progress.execution.runner`）；保留私有字段只会
留下"换名后继续当通用投递参数"的诱惑。该决定不引入 Dispatcher。

以下其余决策已在 `doc/task_mgr/task_dispatch_center.md` §12 定案；实现缺口以本文文首
“当前实现进度核查”为准：

- [x] 对外正式名称与 RPC path：概念名 Task Dispatch Center，服务名/path 为
  `task-dispatcher` / `/kapi/task-dispatcher`。
- [x] Dispatch Store 使用独立 RDB instance `task-dispatcher-main`，与 Task Store 无共享
  表、schema 或 join。
- [x] Target 领取通道：KEvent 通知加速 + `claim_next` 权威拉取 + 低频兜底轮询。
- [x] `Accepted` 始终为 Dispatch 终态；目标结果和进度通过 `target_task_id` link 查询，
  Dispatcher 不做 projection。
- [x] TargetRegistration 真相源设计为“已认证 zone 可信 Service 身份 + 其可信配置”；
  系统级 capability 后续再加 system-config allowlist。（当前实现尚未验证“可信配置”）
- [x] 跨 owner link 使用 `get_dispatch -> target_task_id` 与 Target Task
  `request.dispatch_id` 双向引用，写操作走 Target 业务接口。
- [x] 首个试点 Target：OpenDAN `agent.delegate/v1`。
- [x] 人工放行采用指派前 `PendingApproval` 门：高权限实体 approve/deny；approve 只放行，
  不改 Target/实例，随后仍由 `evaluate_target` 集中指派。
- [x] 审批策略为 per-target `Never` / `InteractiveCallers` / `AllCallers`，默认 `Never`；
  per-operation 覆盖留作后续扩展，不属于 v1。
- [x] 审批不修改身份信封、不提权、不免除 Target 业务鉴权；审批人和备注进入持久审计。
- [x] 审批门封死接收通道，但不改变 zone 可信管理调用者既有 get/list 可见性；如需隐藏
  input，另做读取面脱敏设计。

## 11. 非目标

- 不为旧 runner/task-ready 协议提供兼容层。
- 当前版本不要求普通 TaskMgr 用户迁移到 Dispatcher 或注册 Dispatch Target；OpenDAN 作为真
  Dispatch Target 原属下一版本计划，现已因外部委托能力空档提前接入。
- 不用 Dispatcher 替代普通业务 RPC、Service 内部异步执行或 Service 自己的 Task 恢复机制。
- 不把 Dispatcher 建设成通用 Service Bus 或所有长任务的统一入口。
- 不在 Dispatch Center 中实现业务 Task 状态机。
- 不提供通用分布式事务或 exactly-once 执行保证。
- 不把 Workflow DSL、schedule 或补偿逻辑下沉到 TaskMgr/Dispatch Center。
- 不允许任意服务通过注册动态获得系统级 capability。
- 不把人工放行实现成多级审批工作流、审批委托或按 input 动态审批规则；v1 一条记录只有
  一次 approve/deny 决策。
- 不允许 `approve_dispatch` 手工选择/改写 Target 或实例，也不借审批破坏 Target 黏着；
  换后端必须结束原记录并以新 idempotency key 创建新 Dispatch。
- Dispatcher 的分发前人工放行不替代 Target 业务执行中的人工输入或审批状态。

## 12. beta2.2 收敛记录（P0/P1/P2 执行情况）

### 12.1 身份与鉴权收口

- `task_manager/src/server.rs`：所有 handler fail-closed 验签 session token
  （`SessionTokenVerifier` 抽象，生产走 `runtime.verify_trusted_session_token`）。
- 身份分级：owner/device key 自签 token（kernel/frame service，`iss != "verify-hub"`）
  为 zone 可信调用者，可代已鉴权业务用户填 Task owner 且读写不受 scope 限制；
  verify-hub 签发的交互会话 token 身份强制 = token(sub, appid)，代填即拒绝。
- 协议删除了 `source_user_id` / `source_app_id` / `app_name` 等 payload 身份声明字段。
- `TaskScope::System` 判定从 app_id 字符串比对改为 zone 可信判定；空 ctx 全放行分支删除。
- 已知边界：跨机 device 自签 token 依赖 runtime trust key 集合（当前只含本机 device key），
  多节点服务直连 task-manager 需 runtime 层支持按 device doc 动态加载信任键（独立跟踪）。

### 12.2 原 runner 入口清单与替代

| 入口 | 原 runner | 原领取方式 | 收敛后 |
| --- | --- | --- | --- |
| Control Panel App Installer | `app.control_panel` | task_ready kevent + runner 扫描 + MsgQueue + sweep | 业务接口（apps.*）鉴权后创建；RPC 直接 dispatch + MsgQueue + 启动扫描 + 60s sweep；list 按 task_type=app.install/app.update |
| OpenDAN Agent Task Executor | agent runner_id / full_appid | runner inbox kevent + 按 runner 5 态扫描 | 外部 inbox 已删除；owner-only recovery 按 app_id + target_agent_id 过滤；外部结构化委托已提前改接 Dispatcher `agent.delegate/v1` |
| Workflow send-message | `workflow` | 按 runner 10s 轮询 | 按自身 task_type=workflow.send_message 扫描 + schedule owner 一致性校验（task owner 必须等于所引用 schedule 的 owner） |
| Workflow scheduled task | 模板 runner（任意字符串投递入口） | —（生产者） | 模板/ScheduleTarget/CLI 的 runner 字段删除；fire subtask 以 schedule owner 身份创建（zone 可信代填），身份降级重试删除 |
| Node Daemon Node Executor | node_id | 按 runner 2s 轮询（从未接入主流程） | 死代码整体删除（无生产者：scheduler 不创建 dispatch_thunk task） |

### 12.3 协议与存储

- `Task.runner` / `CreateTaskOptions.runner` / `TaskFilter.runner` / 建表列 / `idx_task_runner_status` 全删；
  RDB schema v5→v6，启动迁移自动 drop 旧列与索引（沿用 title 列删除的迁移模式）。
- `/task_mgr/runner/{runner}/task_ready` 发布与 payload 构造删除；task-changed 事件 payload 去掉 runner 字段。
- `TaskManagerClient::InProcess` 保留为测试注入通道（fake handler）；生产路径全部走 KRPC + 服务端验签。
- DV-07 (`test/kevent_kmsg/task_mgr`) 重写为 task-changed 事件验收（订阅 `/task_mgr/{task_id}`）。
