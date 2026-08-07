# 当前版本 TaskMgr 边界收敛与下一版本 Task Dispatch Center TODO

> **2026-08 更新：Task Dispatch Center 设计已定稿于 `doc/task_mgr/task_dispatch_center.md`，
> 并因 OpenDAN 外部委托依赖决定提前实施（不再绑定 Workflow 版本）。本文 §10 的待决策项
> 已全部在该文档 §12 决策记录中定案；§2～6 的设计草案以该文档为准。本文其余内容保留为
> beta2.2 TaskMgr 边界收敛的执行记录。**
>
> **2026-08-07 实施记录：M1+M2 已落地。**
> - M1：`buckyos-api/src/task_dispatcher.rs`（协议 + client + run_target_instance SDK）、
>   `task_manager/src/dispatcher/`（独立 RDB `task-dispatcher-main`、指派式 evaluate_target、
>   offer lease/expiry timer、启动恢复、kevent 双通道）、`/kapi/task-dispatcher` 挂载
>   （task-manager 进程 3380 第二 path；boot_gateway.yaml 加了 task-dispatcher→task-manager
>   路由别名）、scheduler `add_task_mgr` 追加第二个 rdb instance。23 个单测覆盖
>   解析/黏着/幂等/epoch/Uncertain/重启恢复/授权矩阵。
> - M2：OpenDAN `dispatch_adapter.rs`（register + attach/claim/accept 循环 + 文件化
>   `dispatch_id -> task_id` 幂等绑定 + create-then-crash 自愈扫描）；
>   `agent_task_executor` 伪 inbox 删除（owner-only sweep：app_id 过滤 +
>   `request.target_agent_id` 归属，全局 `/task_mgr/**` 订阅删除）；
>   `AgentDelegateTaskRequest` 增加 dispatch_id/target_agent_id/context_refs/constraints；
>   `--worksession-task-test` 直投入口删除。
> - 遗留：Control Panel 默认路由配置面 / WebUI Task Center dispatch 观察面 / websdk 封装 /
>   DV 环境故障注入（M3 的进程级验收）未做。

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
- [x] 本版本不得要求 Service 注册 Dispatch Target，也不得依赖 Dispatcher 的启动或可用性。（Dispatcher 未实现，内核不依赖）

OpenDAN Agent Task Executor 是少数例外：Agent 的核心职责就是接收外部委托，并且需要 Target
离线等待、capacity、claim/lease、ACK 丢失恢复和幂等交接，因此它属于真正的 Dispatch
Target。beta 2.2 只删除其基于 TaskMgr 的伪 inbox，不新增临时外部委托 RPC；下一版本通过
Dispatcher 投递 `agent.delegate`，OpenDAN 接受后再创建并执行自己的 Task。

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
- [x] TaskMgr 权限不自动包含 Dispatch Center 权限，两者使用独立 RPC 路径和授权策略。（独立 path 上独立 fail-closed 验签 + owner/route/admin 分面授权）
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
TaskMgr 用户的迁移清单。

### 2.0 使用门槛

只有普通业务 RPC 无法覆盖，并且确实需要以下能力时，才应引入 Dispatcher：

- [ ] Caller 提交后，即使 Target 当前离线，请求也必须由独立组件持久保留。
- [ ] Target 的一个或多个实例需要异步 claim，并受 lease、capacity、instance epoch 约束。
- [ ] 系统必须处理 offer/accept ACK 丢失、交接重放和 `dispatch_id` 级幂等。
- [ ] Workflow 需要独立审计“谁把什么工作交给了哪个 Target”，且该交接生命周期独立于业务 Task。

以下情况不使用 Dispatcher：在线 Target 的普通业务调用、Service 自己的后台任务、Service
内部 Task 恢复、Task 状态查询，以及通过 owner 业务 API 完成的取消/重试/审批。

### 2.1 应负责

- [ ] 持久化 Target/Capability 注册信息。
- [ ] 管理 Target 在线实例、lease、capacity 和实例 epoch。
- [ ] 持久化 DispatchRecord，并管理 offer/claim/accept/reject/cancel/expiry 状态机。
- [ ] Target 离线时保留等待记录；Target 上线或恢复后唤醒对应记录。
- [ ] 提供投递审计：谁、代表谁、通过哪个 Workflow/Run/Step、向哪个 Target 投递了什么操作。
- [ ] 管理“接收前”的 delivery retry，防止重复接收造成重复业务 Task。
- [ ] 在 `Accepted` 后保存 `target_task_id`，供调用方建立关联和继续观察。

### 2.2 不应负责

- [ ] 不保存业务 Task 的完整进度、checkpoint 或输出。
- [ ] 不代替 Target 做业务鉴权。
- [ ] 不把业务执行失败自动解释成需要重新 dispatch。
- [ ] 不实现 Workflow DSL、条件分支、补偿或 schedule 语义。
- [ ] 不承担通用节点资源放置；资源调度仍属于 Scheduler。
- [ ] 不承诺通用分布式事务或 exactly-once 业务执行。

### 2.3 Workflow 中保留的能力

现有 `ExecutorAdapter` / `ExecutorRegistry` 可作为调用模型的起点，但要区分两类 executor：

- [ ] `operator::`、人工等待和 Workflow 进程内 adapter 继续由 Workflow 自己管理。
- [ ] 能同步完成的 `service::` / `http::` / `func::` adapter 可继续直接调用。
- [ ] 需要离线等待、异步领取、跨进程恢复的 target 改为调用 Dispatch Center。
- [ ] 共享 Dispatch 协议类型放到 `buckyos-api`，禁止让 TaskMgr 反向依赖 Workflow crate。

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
    pub idempotency_contract: IdempotencyContract,
    pub delivery_policy: DeliveryPolicy,
    pub max_concurrency: u32,
}
```

- [ ] 系统 Target 的 capability 以签名 ServiceDoc/system-config 等可信配置为真相源。
- [ ] 动态实例不能自行声明超出 TargetRegistration 的 operation 或权限。
- [ ] 注册和更新必须绑定调用者的已认证 Service 身份，禁止冒充其他 target。
- [ ] operation 包含版本化 input/output schema，不再接受任意 `task_type + data`。
- [ ] 未注册 target 的 dispatch 请求必须立即失败，不能落成无人负责的普通 Task。

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

- [ ] Target 实例上线时 register/attach，在线期间 renew lease，下线或过期后不再接收 offer。
- [ ] 同一 Target 多实例的挑选策略明确为 round-robin、least-loaded 或 operation 自定义策略之一。
- [ ] instance epoch 防止旧连接或暂停后恢复的实例继续 claim 新记录。
- [ ] Target 注册、实例上线、capacity 释放都能定向唤醒等待中的 DispatchRecord。

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
- [ ] Dispatch Center 校验 target/operation ACL；Target 接收时再次执行完整业务鉴权。
- [ ] 请求信封创建后不可修改；重试复用同一个 `dispatch_id` 和 idempotency key。

### 4.2 独立状态机

Dispatch 状态不要复用 `TaskStatus`：

```text
Queued
  -> WaitingForTarget
  -> Offered
  -> Accepted(target_id, instance_id, target_task_id)

Queued / WaitingForTarget / Offered
  -> Rejected | Expired | Canceled | Uncertain
```

- [ ] `WaitingForTarget` 表示 target 已注册但当前没有可用实例/capacity。
- [ ] `Offered` 带 offer lease；超时后可以按同一 dispatch 重新 offer。
- [ ] `Accepted` 是 Dispatch Center 的正常终态，不继续复制目标 Task 的完整状态机。
- [ ] `Rejected` 区分 schema/auth/policy/business-precondition 等稳定拒绝原因。
- [ ] `Uncertain` 表示 Target 可能已经创建 Task 但接收确认丢失，禁止盲目创建第二个业务 Task。

### 4.3 接收幂等契约

Target 接收必须满足：

```text
相同 dispatch_id 重放 N 次
    -> 返回相同 target_task_id
    -> 最多创建一个 Target 自己拥有的 Task
```

- [ ] Target 在自己的持久存储中原子保存 `dispatch_id -> target_task_id` 绑定。
- [ ] Target 先重新鉴权，再幂等创建自己的 Task，然后返回 `accept`。
- [ ] Task 已创建但 ACK 丢失时，重新投递返回原 `target_task_id`。
- [ ] 无法满足幂等接收契约的 Target 不允许使用自动重新 offer；进入 `Uncertain` 后人工/业务恢复。
- [ ] 区分 delivery retry 与 business retry：前者只发生在 `Accepted` 之前，后者由 Workflow 或 Target 的业务策略决定。

## 5. 离线领取与内部扫描

Target 离线时推荐流程：

```text
1. Caller 创建持久 DispatchRecord
2. 没有在线实例 -> WaitingForTarget
3. TargetInstance register/renew
4. Dispatch Center 唤醒该 target 的 due records
5. Target 通过 stream/long-poll/KEvent 收到通知并 claim_next
6. Target 鉴权并创建自己的 Task
7. Target accept(dispatch_id, target_task_id)
```

- [ ] “查询尚未 dispatch 的记录”只作为 Dispatch Center 内部 Store 接口，不再暴露为通用 TaskMgr 接口。
- [ ] Dispatch Center 可以内部轮询 DispatchRecord，因为这是单一权威组件内的恢复循环，不是多个服务跨进程扫 TaskMgr。
- [ ] 正常路径优先使用数据库通知/内存唤醒：record insert、target register、lease/capacity 变化时触发。
- [ ] 使用 timer/最小堆处理最近 due time、offer lease 和 expiry；启动时做一次恢复扫描。
- [ ] 保留基于索引的低频 due scan 作为丢通知和进程恢复兜底，禁止无条件高频全表扫描。
- [ ] Target 使用定向 stream、long-poll 或 KEvent 加 `claim_next(target_id, instance_id)`，不再调用 `TaskMgr.list_tasks`。

## 6. Task 树与跨所有者关联

Workflow Task 与 Target Task 属于不同所有者，不应直接依赖可写的跨所有者 `parent_id`：

```text
Workflow-owned step/mirror task
    ├── dispatch_id
    └── target_task_id --------------> Target-owned execution task
```

- [ ] Workflow 保留自己拥有的 step/mirror Task，用于流程树展示和恢复。
- [ ] Target 接收后创建自己拥有的 execution Task。
- [ ] 通过不可变 `dispatch_id + target_task_id` link 展开查询，不共享 Task 写权限。
- [ ] 如确实需要跨所有者 parent/subtask，另行设计受限的 attach capability，不能靠伪造 owner/parent 建立。

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
- [x] 使用独立 RPC path、表、索引和权限配置；可以复用同一个进程/RDB 实例。（独立 RDB instance）
- [x] 完成 dispatch/claim/accept/reject/cancel/renew/heartbeat 的单元测试。（dispatcher/tests.rs，23 项）

### 下一版本 F2：Workflow 试点与迁移

- [x] 以 OpenDAN Agent Task Executor 作为首个真正的 Dispatch Target：Agent 本身就是接收任务的
  主力，并且天然需要离线等待、能力约束和幂等交接；App Install 不作为首个试点。（dispatch_adapter.rs）
- [x] 落实注册来源、实例 lease、幂等接收和离线恢复。
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

### 8.2 下一版本 Dispatcher（单测级验收已过，DV 环境级验收待跑）

- [x] 未注册 Target 的 dispatch 被明确拒绝。（dispatch_requires_registered_enabled_supporting_target）
- [x] Target 离线时记录持久等待，上线后能领取且不会生成重复业务 Task。（restart_recovers_waiting_records_and_timers + 幂等绑定）
- [x] 相同 `dispatch_id` 重放返回同一个 `target_task_id`。（accept 幂等重放 + OpenDAN 绑定存储）
- [x] 低权限调用者不能经 Workflow/Dispatcher 触发高权限 operation。（ZoneTrustedOnly 策略 + on_behalf_of 反洗白测试）
- [x] Task Service 与 Dispatch Center 即使同进程，也有独立的数据模型、RPC 和授权规则。
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

以下其余决策属于下一版本 Dispatcher 设计，不阻塞 beta 2.2 的 TaskMgr 边界收敛：

- [ ] 对外正式名称与 RPC path：`task-dispatcher`、`task-dispatch-center` 或其他名称。
- [ ] Dispatch Store 与 Task Store 复用同一个 RDB 实例，还是进程内使用独立 RDB 实例。
- [ ] Target 的主要领取通道：stream、long-poll、KEvent 通知加 claim，或其组合。
- [ ] `Accepted` 是否始终为 Dispatch 终态；如 UI 需要目标结果，应采用查询 link 还是只读 projection。
- [ ] 系统 TargetRegistration 的最终真相源：ServiceDoc、system-config 或二者组合。
- [ ] 跨 owner Task link 的统一查询协议。
- [ ] 首个迁移试点 Target。

## 11. 非目标

- 不为旧 runner/task-ready 协议提供兼容层。
- 当前版本不要求普通 TaskMgr 用户迁移到 Dispatcher 或注册 Dispatch Target；OpenDAN 作为真
  Dispatch Target 的接入属于下一版本。
- 不用 Dispatcher 替代普通业务 RPC、Service 内部异步执行或 Service 自己的 Task 恢复机制。
- 不把 Dispatcher 建设成通用 Service Bus 或所有长任务的统一入口。
- 不在 Dispatch Center 中实现业务 Task 状态机。
- 不提供通用分布式事务或 exactly-once 执行保证。
- 不把 Workflow DSL、schedule 或补偿逻辑下沉到 TaskMgr/Dispatch Center。
- 不允许任意服务通过注册动态获得系统级 capability。

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
| OpenDAN Agent Task Executor | agent runner_id / full_appid | runner inbox kevent + 按 runner 5 态扫描 | 临时收敛曾改为按 task_type + data runner 过滤，但仍属于伪 Dispatch；beta2.2 应删除/禁用外部 inbox，下一版本改接真正 Dispatcher |
| Workflow send-message | `workflow` | 按 runner 10s 轮询 | 按自身 task_type=workflow.send_message 扫描 + schedule owner 一致性校验（task owner 必须等于所引用 schedule 的 owner） |
| Workflow scheduled task | 模板 runner（任意字符串投递入口） | —（生产者） | 模板/ScheduleTarget/CLI 的 runner 字段删除；fire subtask 以 schedule owner 身份创建（zone 可信代填），身份降级重试删除 |
| Node Daemon Node Executor | node_id | 按 runner 2s 轮询（从未接入主流程） | 死代码整体删除（无生产者：scheduler 不创建 dispatch_thunk task） |

### 12.3 协议与存储

- `Task.runner` / `CreateTaskOptions.runner` / `TaskFilter.runner` / 建表列 / `idx_task_runner_status` 全删；
  RDB schema v5→v6，启动迁移自动 drop 旧列与索引（沿用 title 列删除的迁移模式）。
- `/task_mgr/runner/{runner}/task_ready` 发布与 payload 构造删除；task-changed 事件 payload 去掉 runner 字段。
- `TaskManagerClient::InProcess` 保留为测试注入通道（fake handler）；生产路径全部走 KRPC + 服务端验签。
- DV-07 (`test/kevent_kmsg/task_mgr`) 重写为 task-changed 事件验收（订阅 `/task_mgr/{task_id}`）。
