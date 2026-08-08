# Task Dispatch Center 设计

- 状态：设计定稿；M1+M2+M4 已实施（2026-08-07，beta2.2）
  - M1：`buckyos-api/src/task_dispatcher.rs` + `task_manager/src/dispatcher/`
    （独立 RDB `task-dispatcher-main`、`/kapi/task-dispatcher` 同进程第二 path、
    scheduler 追加 rdb instance、boot_gateway.yaml 路由别名）+ 23 项单元测试
  - M2：OpenDAN `dispatch_adapter.rs`（`agent.delegate/v1` Target 注册 + 接收循环 +
    持久幂等绑定）；`agent_task_executor` 伪 inbox 删除（owner-only recovery 保留）
  - M4（同日实施，见 §10 M4）：人工放行（审批门）——`PendingApproval` /
    `DispatchApprovalPolicy` / `approve_dispatch` / `deny_dispatch` 全量落地；
    Dispatcher schema v1→v2（`dispatch_record.approval` 列，旧库启动就地迁移）；
    claim/accept/reject 状态守卫显式排除 `PendingApproval`；
    `/task_dispatcher/approvals` 提示通道；审批权 = zone 可信或 sudo 会话
    （与 `InteractiveCallers` 豁免共用同一判定）；+6 项单元测试
    （task-manager Dispatcher 31 项 / buckyos-api 协议 5 项全绿）
  - 未完：M3 的 DV 环境级故障注入验收；Control Panel 默认路由配置面与
    审批面 / WebUI Task Center dispatch 观察面 / websdk 封装
    （见 §10「影响入口」尾项）
- 目标读者：TaskMgr / OpenDAN / Workflow / buckyos-api 开发者
- 相关文档：
  - `doc/task_mgr/task_mgr.md`（TaskMgr 边界与数据模型）
  - `doc/opendan/Agent Task Executor.md`（首个 Dispatch Target 的接收侧设计）
  - `notepads/task-dispatch-center-todo.md`（历史决策与 beta2.2 收敛记录）

> 版本说明：原计划 Dispatcher 与 Workflow 同版本交付。由于 OpenDAN 的外部委托
> （Workflow/Agent/UI 把工作交给目标 Agent）依赖持久交接语义，且 beta2.2 删除伪
> Dispatch 后该能力出现空档，**Dispatcher 实施提前，不再绑定 Workflow 版本**。
> Workflow 仍然只是 Dispatch Center 的调用者之一，不是实施前置条件。

## 1. 定位

Task Dispatch Center（下称 Dispatcher）管理"把一项工作持久地交给某个负责人"这一独立语义：

```text
Caller
  -> Dispatcher.dispatch(target?, operation, input)
  -> target 未指定时，按管理员配置把 operation 解析到默认 Target
  -> DispatchRecord 持久保存（Target 离线也不丢）
  -> （Target 声明需人工放行时）记录停在 PendingApproval，
     等待管理面 approve/deny；放行前不产生任何 offer
  -> Target 实例上线/有容量时收到 offer
  -> Target 重新鉴权并幂等创建自己拥有的 Task
  -> Target.accept(dispatch_id, target_task_id)
  -> Dispatcher 进入 Accepted 终态，不再参与 Task 执行
  -> Caller 通过 target_task_id 只读观察业务进展
```

Dispatcher 只管理**接收之前**的交接生命周期。接收之后，业务 Task 完全归 Target 所有，
遵循 TaskMgr 的"谁创建，谁执行；谁拥有，谁更新"范式（`doc/task_mgr/task_mgr.md` §7）。
从执行语义看，Dispatcher 只是以可持久、可鉴权、可离线交接的方式调用 Executor 的
operation 接口，并拿回它创建的 `target_task_id`；它不是包在 Task 外面的重试型
supervisor。该 Task 一经创建就是 TaskMgr 的标准 Task，状态、进度、暂停、恢复和终态规则
与其它 Task 完全一致。

Dispatcher 同时提供一个受控的 **operation -> 实际执行后端** 解析面。同一个
`operation` 可以由多个 Target 实现；调用者需要固定后端时显式传 `target_id`，只关心
能力契约时可以省略 `target_id`，由 Dispatcher 按系统管理员维护的默认路由选择 Target。
这使业务协议稳定依赖 `operation`，而部署可以针对数据类型、成本、隐私或领域精度切换
实际 executor。例如，同为 `document.ocr/v1`，电子书系统可以默认指向通用文字 OCR，
扫描 CAD 图纸的系统则可以默认指向图纸专用 OCR。

为避免术语混淆，本文把一个可配置的实际 executor backend 建模为
`TargetRegistration`（逻辑 Target）；`TargetInstance` 仅表示同一 Target 的在线运行副本。
因此完整选择分两级：

```text
operation --管理员默认路由/调用者显式指定--> Target
Target    --DeliveryPolicy + capacity------> TargetInstance
```

### 1.1 与相邻组件的边界

| 组件 | 职责 | Dispatcher 与它的关系 |
| --- | --- | --- |
| TaskMgr | 长任务状态总账 | Dispatcher 不是 TaskMgr 的一部分抽象；同进程部署但独立 store、独立 RPC、独立授权。TaskMgr 对 Dispatcher 零依赖 |
| Scheduler | 节点资源放置 | Dispatcher 可以选择实现 operation 的逻辑 Target，但不决定其节点位置；Target 在哪个节点运行由 Scheduler/部署决定 |
| Workflow | 执行图编排 | Workflow 是调用者；DSL、分支、补偿、schedule 语义都不下沉到 Dispatcher |
| 业务 RPC | 在线服务调用 | Target 在线且无需持久交接时，直接调用 Target 业务接口，不经过 Dispatcher |
| MsgCenter | 消息投递 | 消息用于人类沟通与进展展示；接收确认、offer 重放、幂等交接不用消息协议表达 |

### 1.2 使用门槛

只有普通业务 RPC 无法覆盖，并且确实需要以下能力时，才使用 Dispatcher：

1. Caller 提交后，即使 Target 当前离线，请求也必须由独立组件持久保留。
2. Target 的一个或多个实例需要异步领取，并受 lease、capacity、instance epoch 约束。
3. 系统必须处理 offer/accept ACK 丢失、交接重放和 `dispatch_id` 级幂等。
4. 需要独立审计"谁把什么工作交给了哪个 Target"，且交接生命周期独立于业务 Task。
5. "低权限提交、高权限执行"之间需要一个显式的人工放行点，且放行前 executor
   不得看到请求（§7.1 人工放行）。

不满足门槛的场景（在线 RPC、Service 自己的后台任务、owner 内部恢复、Task 状态查询、
经 owner 业务接口的取消/暂停/恢复/审批）继续走既有模式，不迁移。失败后的重新执行是
一次新的业务请求、新 Task；外部委托场景还必须是一次新的 Dispatch。

关键安全不变式（延续 beta2.2 TaskMgr 收敛结论）：

```text
能够访问 TaskMgr      != 能够向任意 Service 投递工作
能够访问 Dispatcher   != 能够要求高权限 Service 代为执行
Dispatch requester    != Target Task owner
```

## 2. 命名、部署与服务形态

以下为定案（原 TODO §10 待决策项）：

| 项 | 决定 |
| --- | --- |
| 组件名 | Task Dispatch Center（文档/概念名） |
| 服务注册名 / RPC path | `task-dispatcher`，`POST /kapi/task-dispatcher` |
| 进程 | 与 task-manager 同进程、同端口 3380，`Runner.add_http_server` 挂第二个 path |
| 存储 | 独立 RDB instance `task-dispatcher-main`（默认 `sqlite://$appdata/dispatch.db`），独立 schema version，从 1 起 |
| 协议类型位置 | `src/kernel/buckyos-api/src/task_dispatcher.rs`（新文件），禁止 TaskMgr/Dispatcher 反向依赖 Workflow crate |
| 授权 | 独立 RPC path 上的独立授权策略；拥有 TaskMgr 权限不自动获得 Dispatcher 权限 |

部署关系：

```text
task-manager process (port 3380)
├── /kapi/task-manager       -> Task Service          (RDB: task-mgr-main)
│   └── task / task_note
└── /kapi/task-dispatcher    -> Task Dispatch Center  (RDB: task-dispatcher-main)
    └── dispatch_target / dispatch_operation_route / dispatch_instance / dispatch_record / dispatch_event
```

"同进程部署"不等于"同一个抽象"：两个 store 之间没有 SQL join、没有共享表、没有共享
schema version。Dispatcher 模块后续如需独立进程部署，只搬运不重构。

Scheduler 的 `system_config_builder` 在 `add_task_mgr()` 中为同一个 service spec 追加
`task-dispatcher-main` RDB instance 配置；不新增部署单元。

## 3. 数据模型

### 3.1 TargetRegistration（持久注册）

注册的不是任意字符串 runner，而是有所有权和能力约束的 Target：

```rust
pub struct TargetRegistration {
    pub target_id: String,              // 稳定 ID，如 "did:agent:jarvis"
    pub owner_user_id: String,          // 注册落库时由服务端从验签身份写入
    pub owner_app_id: String,           // 同上，如 "opendan"
    pub operations: Vec<OperationDescriptor>,
    pub auth_policy: DispatchAuthPolicy,
    pub approval_policy: DispatchApprovalPolicy, // 人工放行策略，默认 Never（§7.1）
    pub idempotency_contract: IdempotencyContract,
    pub delivery_policy: DeliveryPolicy,
    pub max_concurrency: u32,           // 该 Target 全局在途 offer 上限
    pub enabled: bool,
}

/// 谁的提交需要人工放行（审批门，§7.1）。判定只看**直接调用者**的
/// token 分级，与 on_behalf_of 代填规则同一原则。
pub enum DispatchApprovalPolicy {
    Never,               // 默认：不设人工门，直接进入自动分配
    InteractiveCallers,  // 交互会话（verify-hub 签发且非 sudo）提交先进入 PendingApproval；
                         // zone 可信调用者与 sudo 会话直接放行
    AllCallers,          // 所有提交都需人工放行（含 zone 可信服务 / Agent 自主发起）
}

pub struct OperationDescriptor {
    pub operation: String,              // 含主版本，如 "agent.delegate/v1"
    pub input_schema_ref: Option<String>, // 可选 schema 引用，供校验与 UI
}

pub enum IdempotencyContract {
    IdempotentAccept,   // 相同 dispatch_id 重放返回相同 target_task_id（offer redelivery 的前提）
    None,               // 不承诺幂等接收：offer lease 过期后进入 Uncertain，人工/业务恢复
}

pub struct DeliveryPolicy {
    pub offer_lease_ms: u64,            // 默认 30_000
    pub max_offer_deliveries: u32,      // 默认 10；耗尽 -> Expired(detail=delivery_exhausted)
    pub instance_selection: InstanceSelection, // RoundRobin（默认）| LeastLoaded
}
```

约束：

- 注册/更新必须绑定调用者的已认证 Service 身份：`owner_user_id` / `owner_app_id` 由
  服务端从验签 token 写入，payload 声明无效；禁止冒充其他 Target 的 owner。
- 只有 zone 可信调用者（owner/device key 自签 token 的 kernel/frame service，判定标准与
  TaskMgr 身份分级一致）可以注册 Target。verify-hub 签发的交互会话 token 不能注册。
- operation 是带主版本的封闭清单；未注册 operation 的 dispatch 立即失败。不存在
  "任意 `task_type + data`" 形态的通用 Target。
- 动态实例（3.2）不能声明超出 TargetRegistration 的 operation 或权限。
- 注册真相源 = 已认证 Service 身份 + 该 Service 自己的可信配置（如 OpenDAN 的 agent
  配置）。如未来出现需要系统级 capability 的 operation，再引入 system-config allowlist
  作为附加真相源；`agent.delegate/v1` 不涉及。
- 显式指定的 Target 未注册时，dispatch 请求立即失败，不会落成无人负责的等待记录；
  省略 Target 时的失败规则见 §3.1.1。
- 多个 Target 可以注册同一个 operation；它们是同一能力契约的不同实际执行后端，必须
  接受相同版本的 input schema 并产生该 operation 约定的结果语义。实现差异不能偷偷
  改变 operation 契约；不兼容的输入或结果必须使用新的 operation 主版本。
- `auth_policy` 先于 `approval_policy` 判定：前者回答"能不能提交"，后者回答"提交后
  是否要人工放行"。`ZoneTrustedOnly + InteractiveCallers` 组合中后者永不触发
  （不可信调用者进不了门），注册时可警告但不拒绝；"Agent/可信服务发起的危险操作
  也要人批"必须用 `AllCallers` 表达。v1 为 Target 级配置，per-operation 覆盖列为
  后续扩展（§13）。

#### 3.1.1 OperationRoute（管理员配置的默认后端）

```rust
pub struct OperationRoute {
    pub operation: String,
    pub default_target_id: String,
    pub revision: u64,              // 每次管理侧更新递增，供审计
    pub enabled: bool,
}
```

`OperationRoute` 是系统管理员维护的控制面配置，回答“调用者只指定 operation 时，默认交给
哪个实际执行后端”。它不由 Target 自己声明，避免某个 executor 通过抢先注册把自己变成
系统默认值。v1 对每个 operation 配置一个确定的 `default_target_id`：

- 写入配置时，目标 Target 必须已注册、启用并声明支持该 operation；否则拒绝配置。
- 调用者显式提供 `target_id` 时不读取默认路由；省略时必须存在启用的默认路由，否则
  `dispatch` 返回 `default_target_not_configured`，不创建等待记录。
- 默认路由只影响新请求。Dispatcher 在创建记录时一次性解析并永久固化 `target_id` 与
  `route_revision`；后续管理员切换默认后端，不迁移已有记录，也不改变幂等重放、暂停或
  恢复的目标。这个约束称为 **Target stickiness（Target 黏着）**。
- 选中的 Target 暂时离线或容量不足时，记录在该 Target 下等待，不因运行时波动自动切到
  另一个异构后端。跨后端 fallback 会改变成本、精度和副作用边界，必须由后续显式策略
  定义，不能混入 v1 的交接重放。
- 默认路由只决定实际 Target，不能绕过该 Target 的 `auth_policy`、operation ACL 或接收侧
  业务鉴权。

典型配置如下；不同 Zone 可以为同一个 operation 保存不同默认值：

```json
{
  "operation": "document.ocr/v1",
  "default_target_id": "did:service:ebook-ocr",
  "revision": 7,
  "enabled": true
}
```

### 3.2 TargetInstance（临时在线实例）

```rust
pub struct TargetInstance {
    pub target_id: String,
    pub instance_id: String,        // attach 时由服务端分配
    pub lease_epoch: u64,           // 每次 attach 递增；防旧实例僵尸操作
    pub lease_expires_at: u64,
    pub capacity: u32,              // 实例可承接的在途 offer 上限
    pub available_capacity: u32,    // renew 时由实例上报
}
```

- 实例上线时 `attach_instance`，在线期间周期 `renew_instance`，下线 `detach_instance`
  或 lease 过期后不再收到 offer。
- `lease_epoch` 由 Dispatcher 分配并单调递增。accept/reject/claim 必须携带匹配的
  `(instance_id, lease_epoch)`，不匹配即拒绝——暂停后恢复的旧实例、断连重放的旧连接
  都无法继续操作新记录。
- Target 注册、实例 attach/renew、capacity 释放都会触发对应 Target 的分发评估（§5），
  定向唤醒等待中的 DispatchRecord。

### 3.3 DispatchRequest（不可变请求信封）

`operation + input` 就是可分发任务类型的定义：`operation` 给出带主版本的类型名与契约边界，
`input` 给出该类型的载荷形状。宽度由 Target 自己注册，不是 Dispatcher 统一成一种自由任务——
Agent 的 `agent.delegate/v1` 是最宽的一面（核心意图可以是自然语言）；多数可分发任务应注册
字段封闭、可 schema 校验的窄 operation。具体端到端例子见 §9.1。

调用者提交：

```rust
pub struct DispatchRequestParams {
    pub target_id: Option<String>,      // None = 使用 operation 的管理员默认路由
    pub operation: String,              // 如 "agent.delegate/v1"
    pub input: serde_json::Value,
    pub idempotency_key: String,        // 调用者侧幂等键，必填
    pub expires_at: Option<u64>,        // 交接时限；到期未 Accepted -> Expired
    pub on_behalf_of: Option<String>,   // 业务用户；填写规则见 §7
    pub workflow_ref: Option<WorkflowStepRef>, // workflow/run/step 标识，纯审计
}
```

服务端落库时构造不可变 auth envelope，**全部字段来自验签上下文，不信任 payload**：

```rust
pub struct DispatchAuthEnvelope {
    pub requested_by_user: String,      // = token.sub
    pub requested_by_app: String,       // = token.appid
    pub on_behalf_of: String,           // 校验后的业务用户（§7）
    pub zone_trusted_caller: bool,      // 请求时的身份分级快照
    pub workflow_ref: Option<WorkflowStepRef>,
    pub input_digest: String,           // input 的稳定 hash
    pub created_at: u64,
    pub expires_at: Option<u64>,
}
```

- 信封创建后不可修改。offer redelivery 复用同一个 `dispatch_id` 与信封；它只是完成尚未
  确认的交接，不是重新执行 Task。
- 调用者因网络错误重放 `dispatch()` 依赖 `idempotency_key`：
  `(requested_by_user, requested_by_app, idempotency_key)` 唯一索引，命中后校验不可变请求
  摘要并返回既有 `dispatch_id`，不重新解析默认路由。相同 key 携带不同 operation、input、
  显式 target 或身份信封时返回 `idempotency_conflict`，不产生第二条记录。
- `Rejected` / `Expired` 后重新提交，或目标 Task `Failed` 后要求重新执行，都必须使用新
  `idempotency_key` 创建新 `dispatch_id`。这不是原 Dispatch 的 retry，而是新的业务请求。

`dispatch_id` 为服务端生成的全局唯一字符串（如 `dsp-{uuid}`），是跨系统引用与幂等
交接的锚点，不用自增整数。

### 3.4 DispatchRecord 与状态机

```rust
pub enum TargetSelection {
    Explicit,
    DefaultRoute { route_revision: u64 },
}

pub struct DispatchRecord {
    pub dispatch_id: String,
    pub requested_target_id: Option<String>, // 调用者原始选择；None 表示请求默认路由
    pub target_id: String,                   // 创建记录时解析并固化的实际 Target
    pub target_selection: TargetSelection,
    pub operation: String,
    pub status: DispatchStatus,
    pub input: serde_json::Value,
    pub auth: DispatchAuthEnvelope,
    pub offer_instance_id: Option<String>,   // 当前指派实例
    pub offer_lease_expires_at: Option<u64>,
    pub offer_delivery_count: u32,
    pub target_task_id: Option<i64>,         // Accepted 后回填
    pub reject_reason: Option<DispatchRejectReason>,
    pub approval: Option<DispatchApproval>,  // 人工放行结果（仅经历过审批的记录有值）
    pub message: Option<String>,             // 面向 UI 的说明 / detail
    pub created_at: u64,
    pub updated_at: u64,
}

/// 人工放行的审计记录。审批人身份来自验签 token，不可由 payload 声明；
/// 同一决策同时落 dispatch_event。
pub struct DispatchApproval {
    pub decision: ApprovalDecision,     // Approved | Denied
    pub decided_by_user: String,
    pub decided_by_app: String,
    pub decided_at: u64,
    pub note: Option<String>,           // 审批备注，纯审计/UI
}
```

`DispatchRecord.target_id` 是不可变字段：从记录创建成功开始，任何状态迁移、offer
redelivery、进程重启恢复或管理配置更新都不得重写它。`offer_instance_id` 可以在同一 Target 的
实例间变化，但不能指向其他 Target；这一区别是防止执行后端漂移的基础。

状态机独立于 `TaskStatus`，不复用：

```text
dispatch 落库：
  approval_policy 命中  -> PendingApproval   # 人工放行门（§7.1）
  否则                  -> Queued

PendingApproval
  -> approve_dispatch  -> Queued             # 放行，进入正常评估指派
  -> deny_dispatch     -> Rejected(approval_denied)  # 终态
  -> cancel_dispatch   -> Canceled           # 提交者撤回，终态
  -> expires_at 到期    -> Expired            # 交接时限覆盖审批等待，终态

Queued
  -> WaitingForTarget                  # target 已注册但无可用实例/容量
  -> Offered(instance_id, lease)       # 已指派给某实例，等待 claim/accept
  -> Accepted(target_task_id)          # 正常终态

Queued | WaitingForTarget | Offered
  -> Rejected(reason)                  # Target 稳定拒绝，终态
  -> Expired                           # expires_at 到期或 delivery 耗尽，终态
  -> Canceled                          # Caller 取消，终态

Offered（offer lease 过期）
  -> IdempotentAccept 契约  -> 回到 Queued，重新评估指派（offer_delivery_count + 1）
  -> None 契约              -> Uncertain

Uncertain
  -> resolve_uncertain -> Accepted(task_id) | Canceled   # 管理/业务恢复，非自动
```

状态语义要点：

- `Accepted` 是 Dispatcher 的正常终态。之后 Dispatcher 不复制、不投影目标 Task 的
  执行状态机；业务进展经 `target_task_id` link 观察（§8）。目标 Task 后续进入 `Failed`
  不会反向改变 DispatchRecord，也不会触发重新 offer、重新 dispatch 或后端 fallback。
- `Rejected` 只表达稳定拒绝：`schema_mismatch` / `auth_denied` / `policy_denied` /
  `precondition_failed` / `unsupported_operation` / `target_disabled` / `invalid_input` /
  `approval_denied`（管理面拒绝放行）。
  容量不足、实例离线**不是**拒绝，应停留在 `WaitingForTarget`。
- `PendingApproval` 是**分发之前**的门：`evaluate_target` 不选取该状态、永不产生
  offer；Target 侧 `claim_next` / `accept_dispatch` / `reject_dispatch` 对该状态一律
  无效——即使 Target owner（zone 可信）能在 list 中看到记录，也不能经任何路径提前
  接收。accept 的状态守卫必须显式排除 `PendingApproval`。
- 审批不修改不可变 auth envelope：`requested_by_*` / `on_behalf_of` 保持提交时的
  原始（低权限）身份；放行 ≠ 提权，Target 接收时仍按 envelope 身份做业务鉴权并以
  `on_behalf_of` 记账。审批结果落独立的 `approval` 字段与 `dispatch_event`。
  高权限提交（zone 可信 / sudo 会话）在 `InteractiveCallers` 下不经历审批，
  `approval` 字段为空。
- `Expired` 覆盖两种情况，`message` 区分：请求信封 `expires_at` 到期；
  `max_offer_deliveries` 耗尽（`delivery_exhausted`）。
- `Uncertain` 表示 Target 可能已创建 Task 但确认丢失且无幂等契约兜底。禁止自动
  redelivery，禁止盲目创建第二个业务 Task。
- 取消与 accept 的竞态在 Dispatcher 侧原子裁决：accept 到达时记录若已 `Canceled` /
  `Expired`，accept 返回对应错误；Target 收到该错误后必须取消自己刚创建的本地 Task
  （见 §6.3 接收侧契约）。

#### 3.4.1 Fast-fail：失败终态与重新执行

Dispatcher 不拥有业务执行循环，也没有“目标 Task 失败后自动重启”的 retry policy。
它通过 Target 的 operation 接口创建并拿到 `target_task_id` 后，得到的是 TaskMgr 中一个
标准 Task；后续完全遵守 Task 的简单终态语义：

这里的 fast-fail 指**对外状态机保持简单、失败一旦落库即为终态**，不要求 Executor 把
每一次瞬时错误都立刻上报为 `Failed`；Executor 仍可在 Running 内部处理短暂故障。

```text
DispatchRecord: Accepted(target_task_id=101)       # 已完成交接，保持终态
Target Task 101: Running -> Failed                  # 业务执行失败，Failed 为终态

用户要求重新执行 / 改选后端：
  -> 新 idempotency_key
  -> 新 DispatchRecord（重新解析或显式指定 Target）
  -> 新 target_task_id
```

约束如下：

1. `Completed` / `Failed` / `Canceled` 沿用 TaskMgr 的终态定义。尤其禁止把同一个 Task 从
   `Failed` 改回 `Pending` / `Running`，也禁止 Dispatcher 因观察到 `Failed` 而重新启动它。
2. Dispatcher 不订阅目标 Task 来做 retry 决策，不保存 `execution_retry_count`、backoff、
   retryable error 或失败 fallback 等执行策略。`Accepted` 后它只保留只读 link。
3. 用户点击“重新下载”“重新执行”或选择另一个后端时，调用方必须创建新的 Dispatch。
   新请求拥有新的 `dispatch_id`、`idempotency_key` 和目标 Task；原 Dispatch 与原 Task
   保持原终态，不能被复活或改写。
4. Executor 可以在**尚未把 Task 标记为 Failed**之前实现自己的内部 retry，例如下载分片
   重连、Provider 瞬时错误重试或同 Target 内的 checkpoint 恢复。这是 Executor 私有实现，
   不进入 Dispatcher 协议，也不增加 Task 状态。
5. Executor 内部 retry 期间，标准 Task 对外仍是 `Running`；进度可以暂时不变，随后继续
   增长。Executor 可以更新普通 message 或内部日志，但不能对外呈现
   `Failed -> Running`，也不能为每次内部尝试创建新的 Dispatch。
6. Executor 自己决定何时耗尽内部 retry 并把 Task 标记为 `Failed`。一旦写入 `Failed`，
   本次 Task 执行结束；之后任何重新执行都回到第 3 条的新 Dispatch 语义。

本文仍允许 `Accepted` 之前对同一个 offer 做 redelivery，以处理 lease 过期或 accept ACK
丢失。该动作始终复用同一个 `dispatch_id`，并依赖 Target 幂等地返回同一个
`target_task_id`；它只是**交接协议恢复**，不会在 Task 失败后发生，不属于执行 retry。

### 3.5 存储表结构

RDB instance `task-dispatcher-main`，schema version 1：

| 表 | 关键列 | 索引 |
| --- | --- | --- |
| `dispatch_target` | `target_id` PK, `owner_user_id`, `owner_app_id`, `registration`(JSON), `enabled`, `created_at`, `updated_at` | — |
| `dispatch_operation_route` | `operation` PK, `default_target_id`, `revision`, `enabled`, `created_at`, `updated_at` | `idx_dor_target`(`default_target_id`) |
| `dispatch_instance` | PK(`target_id`,`instance_id`), `lease_epoch`, `lease_expires_at`, `capacity`, `available_capacity`, `attached_at`, `renewed_at` | `idx_di_lease`(`lease_expires_at`) |
| `dispatch_record` | `dispatch_id` PK, `requested_target_id`, `target_id`, `target_selection`(JSON), `operation`, `status`, `input`(JSON), `auth`(JSON), `idempotency_key`, `requested_by_user`, `requested_by_app`, `on_behalf_of`, `offer_instance_id`, `offer_lease_expires_at`, `offer_delivery_count`, `target_task_id`, `reject_reason`, `approval`(JSON), `message`, `expires_at`, `created_at`, `updated_at` | UNIQUE(`requested_by_user`,`requested_by_app`,`idempotency_key`)；`idx_dr_target_status`(`target_id`,`status`,`created_at`)；`idx_dr_due`(`status`,`expires_at`)；`idx_dr_requester`(`requested_by_user`,`requested_by_app`,`created_at` DESC) |
| `dispatch_event` | `id` PK 自增, `dispatch_id`, `ts`, `from_status`, `to_status`, `instance_id`, `detail` | `idx_de_dispatch`(`dispatch_id`,`ts`) |

`dispatch_event` 是投递审计的最小实现：谁、代表谁、经哪个 workflow/step、向哪个
Target 投递了什么，在 `dispatch_record.auth` 中；状态迁移轨迹在 `dispatch_event` 中。
`requested_target_id + target_id + target_selection` 额外回答“调用者是否指定后端；若没有，
当时由哪一版默认路由选中了谁”。

## 4. RPC 协议

`POST /kapi/task-dispatcher`，方法分三组，授权策略互相独立（§7）：

### 4.1 Caller 侧

| Method | 说明 |
| --- | --- |
| `dispatch` | 提交 `DispatchRequestParams`，`target_id=None` 时先解析 operation 默认路由；返回 `{dispatch_id, target_id, target_selection, status}`；idempotency_key 命中时返回既有记录。审批门命中时返回 `status=PendingApproval`，调用者需处理"已受理未放行" |
| `get_dispatch` | 按 `dispatch_id` 查询记录（含 `target_task_id`） |
| `list_dispatches` | 按 requester / target / status / 时间过滤；普通调用者只能看到自己提交的记录 |
| `cancel_dispatch` | `Accepted` 之前取消（含 `PendingApproval`：提交者可撤回待审批记录）；`Accepted` 之后返回错误并提示走 Target 业务接口 |

Caller 侧没有 `retry_dispatch`、`restart_dispatch` 或 `resume_failed_task`。重新执行必须再次
调用 `dispatch` 并提供新的 `idempotency_key`，从而创建新的 DispatchRecord；是否显式改选
Target 由这次新请求决定。

### 4.2 Target 侧

| Method | 说明 |
| --- | --- |
| `register_target` | 注册/更新 TargetRegistration（zone 可信 + owner 绑定验签身份） |
| `disable_target` | 停用 Target；在途记录停留在 `WaitingForTarget`，新 dispatch 被拒 |
| `attach_instance` | 实例上线，返回 `{instance_id, lease_epoch, lease_ttl_ms}` |
| `renew_instance` | 续 lease 并上报 `available_capacity`，返回 `{lease_ttl_ms, has_due}` |
| `detach_instance` | 实例下线；其在途 offer 立即回收重评估 |
| `claim_next` | `(target_id, instance_id, lease_epoch, max)` 拉取指派给本实例的 Offered 记录；幂等，可重复拉取 lease 内同批记录 |
| `accept_dispatch` | `(dispatch_id, instance_id, lease_epoch, target_task_id)` 原子转 `Accepted` |
| `reject_dispatch` | `(dispatch_id, instance_id, lease_epoch, reason, detail)` 转 `Rejected` |

### 4.3 管理侧

| Method | 说明 |
| --- | --- |
| `approve_dispatch` | 把 `PendingApproval` 放行为 `Queued` 并触发指派评估；审批人身份与可选 `note` 落 `approval` 字段与 `dispatch_event`；对已放行记录幂等 |
| `deny_dispatch` | 把 `PendingApproval` 拒绝为 `Rejected(approval_denied)` 终态；重新提交必须新 `idempotency_key` 新 dispatch |
| `resolve_uncertain` | 把 `Uncertain` 裁决为 `Accepted(task_id)` 或 `Canceled`；zone owner / 可信服务专用 |
| `list_targets` / `get_target` | 注册面观察 |
| `set_operation_route` / `disable_operation_route` | 设置或停用 operation 的默认 Target；仅影响新 dispatch |
| `list_operation_routes` / `get_operation_route` | 查看默认后端配置、revision 与目标有效性 |

审批队列没有独立方法：`list_dispatches(status=PendingApproval)` 即待审批列表
（管理面可见全部记录）。

SDK（`TaskDispatcherClient`）为 Target 侧提供组合封装：`run_target_instance` 内部
处理 attach → 订阅 kevent → claim/accept 循环 → renew → 兜底轮询，业务方只实现
"验证 + 幂等建 Task + 返回 task_id" 回调。

## 5. 分发模型：指派而非抢占

**归属判定始终在 Dispatcher 内部按 policy 完成，不因谁先 poll 而改变。**

一次 dispatch 有两次含义不同的确定性选择：

```text
resolve_target(operation, requested_target_id?)：
  0. idempotency_key 已命中 -> 校验不可变请求摘要并返回原记录，不再解析路由
  1. requested_target_id 有值 -> 校验该 Target 已注册、启用并支持 operation
  2. requested_target_id 为空 -> 读取启用的 OperationRoute，取得 default_target_id
  3. 对解析后的 Target 执行 auth_policy / operation ACL
  4. 按 approval_policy 与直接调用者 token 分级决定初始状态
     （命中 -> PendingApproval，否则 -> Queued）
  5. 原子写入 requested_target_id、target_id、TargetSelection 与 DispatchRecord

evaluate_target(target_id)：
  在同一 Target 的在线 TargetInstance 中按 delivery policy + capacity 指派
```

第一步是管理员可配置的**异构执行后端选择**，第二步是同一后端多个等价运行副本之间的
**投递实例选择**。前者可以让两个 Zone 分别把 `document.ocr/v1` 默认交给电子书 OCR 与
CAD OCR；后者只负责把已确定的 OCR 后端交给它的某个在线进程，不能改变后端类型。

### 5.1 Target 黏着：暂停与恢复不得漂移

**Target 只在 DispatchRecord 首次创建时选择一次。** 一旦 `target_id=A` 已经落库，这条
记录以及它接受后创建的业务 Task 都黏着在 A；管理员随后把该 operation 的默认 Target
改成 B，只会影响新 `idempotency_key` 创建的新 DispatchRecord。

具体约束：

1. `Queued` / `WaitingForTarget` / `Offered` 阶段发生 lease 回收、Dispatcher 重启或
   offer redelivery 时，只能再次执行 `evaluate_target(A)`。可以换到 A 的另一个
   `TargetInstance`，不能重新执行 `OperationRoute` 后改投 B。
2. `Accepted` 后的暂停/恢复属于 A 的业务 Task 生命周期，应通过 A 的业务接口和原
   `target_task_id` 完成，不再次调用 operation 默认路由。只要 A 的 executor 声明并实现
   pause/resume 与 checkpoint 恢复契约，恢复后仍由 A 执行。
3. TargetInstance 是运行副本而不是实际后端身份。恢复时物理实例可以变化，例如从 A-1
   恢复到 A-2，但二者必须属于同一 `target_id=A`；这属于同后端容灾，不是 Executor 漂移。
4. A 离线、被停用或暂时无法恢复时，尚未 Accepted 的记录只能继续在 A 下等待或到期；
   已 Accepted 的业务 Task 则由 A 的业务接口返回恢复错误。两种情况都不能静默改投 B，
   系统不能用“换后端重新执行”冒充“恢复原任务”。
5. 如果 A 不支持恢复，调用方只能明确结束原任务，再以新的 `idempotency_key` 发起一次
   新 dispatch；它可以按当时的默认路由选择 B，但这是新的业务执行，不是原任务
   的 resume。任何跨 Target 迁移都必须是另行设计、显式审计的管理操作，不属于 v1。
6. v1 不提供 `retarget_dispatch` 一类修改已有记录归属的 RPC；
   `set_operation_route` 也禁止批量改写既有 `DispatchRecord.target_id`。

因此“默认后端从 A 切到 B”与“已有任务从暂停状态恢复”是两条互不影响的控制路径：前者
修改未来请求的解析结果，后者沿已固化的 `dispatch_id -> target_id -> target_task_id`
链路恢复。

### 5.2 TargetInstance 指派

设计上排除 FCFS 抢占模型（多实例自由 claim 未指派记录、先到先得）：dispatch policy
一旦下放给实例就无法收回，且抢占模型无法表达 capacity、epoch 和定向恢复。实例选择
是 Dispatcher 的集中决策：

```text
evaluate_target(target_id)：           # 唯一的指派路径
  输入触发点（任一）：
    - dispatch 落库
    - approve_dispatch 放行
    - attach / renew / detach / capacity 变化
    - offer lease 过期、expires_at 到期（timer）
    - claim_next 到达（仅作为评估触发点）
    - 启动恢复扫描 / 低频兜底 sweep
  动作：
    1. 取该 target 下 Queued/WaitingForTarget 的 due 记录
       （PendingApproval 永不入选）
    2. 按 DeliveryPolicy.instance_selection 在活跃实例中选择
       （RoundRobin 默认 / LeastLoaded 按 available_capacity）
    3. 尊重 instance available_capacity 与 target max_concurrency
    4. 写入 Offered(instance_id, lease)，发 kevent 通知该实例
    5. 无可用实例/容量 -> WaitingForTarget
```

`claim_next` 只是传输通道：返回**已指派给本实例**的 Offered 记录，本身不做跨实例
选择。指派后实例在 offer lease 内未 accept/reject，记录回收并重新评估（可能指派给
其他实例）——这仍是集中策略下的故障转移，不是抢占。

单实例 Target（OpenDAN 每个 agent 通常一个 runtime 实例）在此模型下自然退化为
"指派给唯一实例"，协议形态不变。

## 6. 通知、唤醒与恢复

### 6.1 kevent 是加速通道，不是真相源

沿用 kevent 使用纪律：通知只加速，权威状态永远从 Dispatcher 拉取，且通知路径与
兜底路径汇入同一个处理函数。

kevent channel：

```text
/task_dispatcher/target/{target_key}     # 面向 Target 实例："有 due 记录"提示
/task_dispatcher/{dispatch_id}           # 面向 Caller：记录状态变更事件
/task_dispatcher/approvals               # 面向管理面："有新待审批记录"提示（可选加速）
```

审批队列的权威始终是 `list_dispatches(status=PendingApproval)` 拉取；
`/task_dispatcher/approvals` 只是管理 UI 的唤醒提示，payload 仅带 ids，
丢失时退化为管理面轮询，不丢积压。

- `target_key` 必须通过 kevent event id 校验；`target_id` 含非法字符（如 DID 的 `:`
  不被允许时）使用稳定 slug/hash 映射，映射关系在 attach 返回值中给出。
- 事件 payload 只带 `dispatch_id / target_id / from / to / target_task_id? / ts`，
  不内联 input。

### 6.2 Target 实例侧接收循环

```text
1. attach_instance -> {instance_id, lease_epoch, target_key}
2. 订阅 /task_dispatcher/target/{target_key}
3. 循环：
   - pull_event 带超时等通知
   - 收到通知或超时 -> claim_next(...)        # 同一个入口，通知只是提前唤醒
   - 处理返回记录：验证 -> 幂等建 Task -> accept / reject
   - 周期 renew_instance 上报 available_capacity
4. 低频兜底：即使 kevent 全丢，按 renew 周期附带的 has_due 提示或固定间隔 claim_next
```

### 6.3 接收侧契约（对所有 Target）

1. 先重新鉴权（Target 自己的业务鉴权，Dispatcher 的 ACL 不能替代），再幂等创建
   自己拥有的 Task，最后 `accept`。
2. `dispatch_id -> target_task_id` 绑定必须保存在 Target 自己的持久存储中，且与建
   Task 原子；仅把 dispatch_id 写进 Task.data 再扫描去重不满足并发幂等。
3. 相同 `dispatch_id` 重放 N 次：返回相同 `target_task_id`，最多创建一个 Task。
4. 建议在 `accept` 成功返回之后才开始执行。`accept` 返回 `Canceled` / `Expired` 时，
   Target 必须取消刚创建的本地 Task（已投入执行的按本地取消处理）。
5. 无法满足第 2、3 条的 Target 注册时声明 `IdempotencyContract::None`，代价是
   offer lease 过期不自动 redeliver，进入 `Uncertain` 等待人工/业务恢复。
6. operation 若在 Target 业务接口中声明支持 pause/resume，恢复必须复用持久化的
   `dispatch_id -> target_task_id` 绑定和该 Target 自己的 checkpoint；不得把 resume 实现为
   一次省略 `target_id` 的新 dispatch。Target 内部可以换运行实例，但不能换 Target。

### 6.4 Dispatcher 内部唤醒与恢复

- 正常路径全部靠定向唤醒：record insert、attach/renew、capacity 释放、accept/reject
  即时触发 `evaluate_target`；进程内存中维护 per-target waker。
- timer/最小堆管理三类时限：offer lease 过期、请求 `expires_at`、instance lease 过期。
- 启动时做一次恢复扫描：重建 timer 堆、把 lease 已过期的 Offered 记录回收重评估、
  把 `expires_at` 已过的记录转 `Expired`。
- 保留基于 `idx_dr_due` 的低频 due scan（分钟级）作为丢通知和 timer 漂移的兜底；
  禁止高频全表扫描。
- "查询尚未 dispatch 的记录"只是 Dispatcher 内部 Store 接口。它是单一权威组件内的
  恢复循环，与 beta2.2 删除的"多个服务跨进程扫 TaskMgr"有本质区别，不对外暴露。

## 7. 鉴权模型

身份分级完全沿用 TaskMgr beta2.2 收敛后的模型：服务端 fail-closed 验签 session token；
owner/device key 自签 token（kernel/frame service）为 zone 可信调用者；verify-hub 签发
的交互会话 token 为普通调用者。

| 操作 | 授权规则 |
| --- | --- |
| `dispatch` | 先解析并固化 Target，再通过该 Target 的 `auth_policy` 判定。`requested_by_*` 强制 = token 身份。`on_behalf_of`：普通调用者强制 = token.sub（代填即拒绝）；zone 可信调用者可代填已鉴权业务用户 |
| `get_dispatch` / `list_dispatches` | zone 可信调用者不受限；普通调用者只能查看 `requested_by` 或 `on_behalf_of` 是自己的记录 |
| `cancel_dispatch` | 记录的 requester、on_behalf_of 用户或 zone 可信调用者 |
| `register_target` / `disable_target` | 仅 zone 可信调用者；owner 绑定验签身份，更新时校验 owner 一致 |
| `set/disable_operation_route` | 仅 zone owner / 具备系统配置管理权限的可信调用者；Target owner 不能仅凭注册权限修改系统默认后端 |
| `approve_dispatch` / `deny_dispatch` | 仅 zone owner / 具备 dispatcher 管理能力的调用者（与 route 配置同级）。当前实现档位：zone 可信调用者，或 verify-hub 签发且 `sudo=true` 的交互会话。Target owner 身份**本身不含**审批权（否则注册方可自我放行）；提交者不能因为是提交者而获得审批权 |
| `attach/renew/detach/claim/accept/reject` | 仅 target owner 身份（`owner_user_id`/`owner_app_id` 与 token 一致的 zone 可信调用者）；对 `PendingApproval` 记录一律无效 |
| `resolve_uncertain` | zone owner / zone 可信调用者 |

`DispatchAuthPolicy`（per-target，v1 取值）：

```rust
pub enum DispatchAuthPolicy {
    ZoneUsers,          // 任何已认证 zone 用户（含交互会话）可 dispatch —— agent.delegate 默认
    ZoneTrustedOnly,    // 仅 zone 可信服务可 dispatch
}
```

反身份洗白规则：

- Workflow/中间层只能传递已有身份与授权证据，不能把普通用户请求"洗"为 system 身份；
  `on_behalf_of` 的代填资格只看**直接调用者**的 token 分级。
- Dispatcher 校验 target/operation ACL 后，Target 接收时仍执行完整业务鉴权
  （envelope 里的身份是输入，不是免检凭证）。
- Dispatcher 的 ACL 不能替代 Target 的业务鉴权，Target 的接受也不反向扩大 Caller
  对业务 Task 的权限。
- 审批不是身份信封的修改途径：`approve_dispatch` 只推进状态机，不重写
  `requested_by_*` / `on_behalf_of`，不给记录附加任何等价于提权的标记，也不豁免
  Target 的业务鉴权。审批解决的是"这次交接是否被允许发生"，不是"以谁的身份执行"。

### 7.1 人工放行（审批门）

解决的问题：`ZoneUsers` 允许低权限用户的提交**直接**触发高权限 executor 执行，
对危险 operation 过于开放；`ZoneTrustedOnly` 又让低权限用户完全无法提交。审批门是
中间档——低权限可以提交，但把工作交给 executor 之前必须有一次显式的人工决策，
使"低权限提交 → 高权限执行"从自动行为变成有完整审计的例外通道（谁提交、代表谁、
谁批准、执行成什么，四段都可查）。

```text
bob（交互会话）
  -> dispatch(apps.install/v1, ...)        # Target 声明 approval_policy=InteractiveCallers
  -> DispatchRecord(PendingApproval)       # 不评估、不 offer，executor 全程不可见
管理员（zone owner / sudo 会话）
  -> list_dispatches(status=PendingApproval)
  -> approve_dispatch(dispatch_id, note?)  # 或 deny_dispatch -> Rejected(approval_denied)
  -> Queued -> evaluate_target 正常指派 -> Offered -> installer accept
```

语义边界（与既有决策的关系）：

1. **这是审批门，不是手工指派。** 放行后实例选择仍由 `evaluate_target` 集中完成
   （§5.2 排除 FCFS/人工挑实例的决策不变）；`approve_dispatch` 不能改
   `target_id`、不能指定实例（Target 黏着不破，§5.1）。管理员想换后端时，
   deny/cancel 原记录，再以新 `idempotency_key` 显式指定 `target_id` 重新
   dispatch——与"重新执行必须新 Dispatch"同一语义。
2. **门在指派之前。** 未放行的记录不进入评估、不产生 offer、不占
   `max_concurrency`、不计 delivery；Target 侧对它的任何操作
   （claim/accept/reject）都被状态守卫拒绝。最小暴露：未放行的请求不该到达
   executor。**边界口径（实施定案）：审批门封死的是接收通道**——评估、offer、
   kevent target 通知、claim/accept/reject 全部不可达，executor 的接收循环
   永远见不到该请求；zone 可信调用者经 get/list 的只读可见性遵循既有查询授权，
   不因审批门收窄（§3.4"能看到但不能接收"即此义）。若未来要求对 Target owner
   隐藏未放行请求的 input，属读取面脱敏扩展，另行设计，不改变本条接收边界。
3. **审批 ≠ 提权。** envelope 不变；Target 接收后创建的业务 Task 仍以
   `on_behalf_of` 业务用户记账；Target 自己的业务鉴权照做（它看得到
   `approval` 字段，可以要求高危 operation 必须带审批记录，但审批不是免检凭证）。
4. **时限与积压。** `expires_at` 覆盖审批等待（到期未放行 -> `Expired`）；
   不设策略级审批超时，积压治理靠提交方时限 + 管理面 deny + 审批队列可见性。
5. **与业务层审批分层。** Dispatcher 审批回答"要不要把工作交给 executor"；
   执行过程中的人工介入（如 OpenDAN `human.input` 子任务、
   `TaskStatus::WaitingForApproval`）仍属 Target 业务层，两者不重叠、不互替。

## 8. 跨所有者 Task 关联

Caller（如 Workflow）与 Target 的 Task 属于不同 owner，不共享写权限，不使用可写的
跨 owner `parent_id`：

```text
Caller-owned step/mirror task (可选)
    ├── dispatch_id ----------------┐
    └── target_task_id -------------┼--> Target-owned execution task
                                    │        └── data.request.dispatch_id
                DispatchRecord ─────┘
```

- link 协议就是不可变的 `dispatch_id + target_task_id` 双向引用：
  `get_dispatch(dispatch_id)` 给出 `target_task_id`；Target Task 的
  `data.request.dispatch_id` 反向指回。
- Caller 对目标 Task 只做只读观察（`task_mgr.get_task` + 订阅 `/task_mgr/{task_id}`）。
  可见性由 Target 建 Task 时的 owner 与 permissions 决定：如 OpenDAN 以
  `user_id = on_behalf_of 业务用户` 建 Task，read scope `User` 即可让委托人观察。
- 取消、暂停、恢复、审批等对**当前 Task**的写操作调用 Target 的业务接口（如 OpenDAN 的
  `pause/resume/cancel_agent_task`），不经过 Dispatcher，也不直接写 TaskMgr。
- `Failed` Task 不存在恢复或原地重试。用户要求重新执行时必须发起新 Dispatch，由新的
  `target_task_id` 表达；调用方可以显式选择另一个 Target，原 link 保持不变。
- 确需跨 owner 树形展示时，Caller 用自己的 mirror task 承载展示层级；如未来需要真正
  的跨 owner attach，另行设计受限 capability，不靠伪造 owner/parent。

## 9. 首个 Target：OpenDAN `agent.delegate/v1`

接收侧完整设计见 `doc/opendan/Agent Task Executor.md`。Dispatcher 视角的对接要点：

- 每个可委托 Agent 注册为独立 Target（`target_id` = Agent DID/稳定 ID），
  `owner_app_id = opendan`；不注册"整个 OpenDAN"这种无法区分目标的通用 Target。
- OpenDAN 是 frame service（zone 可信），满足 register/attach/claim/accept 的授权门槛。
- operation 只有 `agent.delegate/v1`；input 为 `AgentDelegateDispatchInput`
  （title/purpose/input/context_refs/workspace_hints/constraints）。requested-by、
  on-behalf-of、workflow_ref 一律来自 envelope，不在 input 中。
- OpenDAN 声明 `IdempotencyContract::IdempotentAccept`：在自己的持久存储中原子保存
  `dispatch_id -> task_id`，接受重放返回原 task_id，最多绑定一个 WorkSession。
- Accept 后 OpenDAN 创建 `task_type = agent.delegate`、`app_id = opendan`、
  `user_id = 验证后的 on_behalf_of` 的 Task，走既有 WorkSession 1:1 执行与恢复路径。
- 三类工作入口的分工不变：人类 IM 与 OpenDAN 内部派生工作不经过 Dispatcher；只有
  外部结构化委托（Workflow、其它 Agent、UI 发起的"交给某个 Agent"）走 dispatch。
- 旧伪 Dispatch（`agent_task_executor` 按 `task_type=agent.delegate` 全量扫描 + 按
  `data.progress.execution.runner` 过滤）随本设计落地一并删除，由 Dispatch Target
  Adapter 取代；这同时清掉 beta2.2 收敛清单的最后一项。

### 9.1 端到端例子：Workflow 把审查工作交给 Jarvis

场景：用户 `alice` 触发的 Workflow Run 走到一步「请 Jarvis 审查指定变更并给出结论」。
Jarvis 当前可能暂时离线，因此走 Dispatcher，而不是直接调 OpenDAN 在线 RPC。人类 IM
对话不走这条路径（见 OpenDAN 文档「三类工作入口」）。

**1. OpenDAN 事先注册 Target（zone 可信身份）**

```json
{
  "target_id": "did:agent:jarvis",
  "operations": [{ "operation": "agent.delegate/v1" }],
  "auth_policy": "ZoneUsers",
  "idempotency_contract": "IdempotentAccept",
  "delivery_policy": {
    "offer_lease_ms": 30000,
    "max_offer_deliveries": 10,
    "instance_selection": "RoundRobin"
  },
  "max_concurrency": 4,
  "enabled": true
}
```

`owner_user_id` / `owner_app_id` 由服务端从 OpenDAN 的验签 token 写入（`opendan`），
payload 不能冒充。Jarvis 的 AgentRuntime 上线后 `attach_instance`，周期
`renew_instance` 上报 `available_capacity`。

**2. Workflow（zone 可信调用者）提交 dispatch**

```json
{
  "target_id": "did:agent:jarvis",
  "operation": "agent.delegate/v1",
  "idempotency_key": "wf-run-9f3a/step-review/attempt-1",
  "on_behalf_of": "alice",
  "workflow_ref": {
    "workflow_id": "code-review",
    "run_id": "wf-run-9f3a",
    "step_id": "review"
  },
  "input": {
    "title": "Review changes",
    "purpose": "检查指定变更并输出结论：是否可合并、主要风险、建议修改点",
    "input": {
      "repo": "buckyos/buckyos",
      "base": "main",
      "head": "feat/dispatch-center"
    },
    "context_refs": [
      { "kind": "git.diff", "ref": "pr://buckyos/buckyos/128" }
    ],
    "workspace_hints": [
      { "kind": "prefer", "workspace": "code-review" }
    ],
    "constraints": { "max_wall_time_sec": 1800 }
  }
}
```

要点：

- **任务类型** = `agent.delegate/v1` + 上面的 `input`。`purpose` 可以是自然语言，这是
  Agent 面故意留宽的部分；`input` / `context_refs` 仍可带结构化线索。
- **身份不在 input 里**：`on_behalf_of`、`workflow_ref` 走请求参数，最终落入服务端
  `DispatchAuthEnvelope`（`requested_by_*` 强制来自 token，不信任 payload）。
- 未注册的 `target_id` / `operation` 会被立即拒绝。

Dispatcher 返回例如
`{ "dispatch_id": "dsp-7c2e...", "target_id": "did:agent:jarvis", "target_selection": "Explicit", "status": "WaitingForTarget" }`
（若 Jarvis 实例当时无容量/离线）；有容量时直接进入 `Offered`。

**3. Jarvis 实例领取并接受**

```text
attach_instance(did:agent:jarvis)
  -> { instance_id, lease_epoch, target_key }

kevent / claim_next
  -> Offered 记录（含 dispatch_id、operation、input、auth envelope）

OpenDAN Adapter（见 Agent Task Executor §6）：
  1. 重新鉴权 envelope + Agent policy
  2. 校验 purpose / workspace_hints / constraints
  3. 原子查/建本地绑定 dispatch_id -> task_id
  4. 创建 OpenDAN 拥有的 Task：
       task_type = "agent.delegate"
       app_id    = "opendan"
       user_id   = "alice"          // 验证后的 on_behalf_of
  5. accept_dispatch(dispatch_id, instance_id, lease_epoch, target_task_id=42)
```

Accept 后 `DispatchRecord` 进入终态 `Accepted(42)`。此后 Dispatcher 不再复制执行状态；
Workflow 用 `get_dispatch` 拿到 `target_task_id`，再只读观察 TaskMgr / 订阅
`/task_mgr/42`。取消、暂停走 OpenDAN 的 `pause/resume/cancel_agent_task`，不回写
Dispatcher。

**4. OpenDAN 落库的 Task.data.request（示意）**

```json
{
  "version": 1,
  "source": "task-dispatcher",
  "dispatch_id": "dsp-7c2e...",
  "target_agent_id": "did:agent:jarvis",
  "title": "Review changes",
  "purpose": "检查指定变更并输出结论：是否可合并、主要风险、建议修改点",
  "input": {
    "repo": "buckyos/buckyos",
    "base": "main",
    "head": "feat/dispatch-center"
  },
  "workspace_hints": [{ "kind": "prefer", "workspace": "code-review" }],
  "reason_messages": []
}
```

`dispatch_id` 与 `target_task_id` 构成跨 owner 只读 link；相同 `dispatch_id` 重放必须
返回同一个 `task_id`，最多绑定一个 WorkSession。

**5. 与窄 operation 的对比（非本试点，仅说明宽度）**

若未来某个安装服务注册 `apps.install/v1`，其 `input` 应是封闭字段（如
`app_id` / `version` / `channel`），未知字段直接 `schema_mismatch`——不必、也不应抄
`agent.delegate` 的自由文本 `purpose`。宽度是 Target 的产品决策；Dispatcher 只要求
operation 已注册且形状可校验。

### 9.2 可配置实际执行后端：电子书 OCR 与 CAD OCR

假设两个 Target 都注册同一稳定契约 `document.ocr/v1`：

```text
did:service:ebook-ocr  -> 针对正文、目录、脚注和纵排文本优化
did:service:cad-ocr    -> 针对图框、尺寸标注、符号和旋转文字优化
```

电子书产品的系统管理员配置：

```json
{
  "operation": "document.ocr/v1",
  "default_target_id": "did:service:ebook-ocr",
  "revision": 7,
  "enabled": true
}
```

扫描 CAD 图纸的 Zone 则把同一配置项的 `default_target_id` 设为
`did:service:cad-ocr`。上层应用可以在两种系统中提交完全相同的能力请求，不需要知道实际
安装了哪一种 OCR：

```json
{
  "operation": "document.ocr/v1",
  "idempotency_key": "ocr:sha256:8af3...",
  "input": {
    "content_ref": "obj://sha256/8af3...",
    "output_format": "structured-text"
  }
}
```

`target_id` 被省略后，Dispatcher 在创建记录时解析默认路由，并在返回值及记录中明确给出
实际选择，例如 `target_id=did:service:cad-ocr`、
`target_selection=DefaultRoute(route_revision=12)`。如果调用者在测试、对比或专用流程中
必须固定后端，仍可显式传入 `target_id`；显式选择也必须通过目标的 ACL，不是越权入口。

这里可配置的是“哪一种 OCR 后端”。选定 `did:service:cad-ocr` 后，再由该 Target 的
`DeliveryPolicy` 从它的多个 `TargetInstance` 中选择在线实例。两级选择必须保持分离。

例如某份 PDF 创建任务时默认路由仍为 `did:service:ebook-ocr`，任务执行一半后暂停；此时
管理员把 `document.ocr/v1` 的默认 Target 改成 `did:service:cad-ocr`。随后恢复这份 PDF
时，系统必须沿原 `dispatch_id` 和 `target_task_id` 回到 `did:service:ebook-ocr`。若该
Target 支持跨实例恢复，可以由另一个 ebook-ocr TargetInstance 接续 checkpoint，但绝不能
因为当前默认值已经是 cad-ocr 而把原任务改投 cad-ocr。只有新建的 OCR dispatch 才使用
新的默认 Target。

### 9.3 下载失败后更换后端：新 Dispatch、新 Task

假设 `content.download/v1` 同时由下载后端 A、B 实现。第一次请求选择 A：

```text
Dispatch dsp-001 -> Target A -> Task 101
Task 101: Pending -> Running -> Failed(network_unreachable)
Dispatch dsp-001: Accepted(101)                    # 不变，不重新启动
```

用户随后选择 B 并点击“重新下载”。正确语义是创建全新的执行链：

```text
Dispatch dsp-002 -> Target B -> Task 205
Task 205: Pending -> Running -> Completed

dsp-001 / Task 101 仍保持原终态，用于历史与审计
```

`dsp-002` 必须使用新的 `idempotency_key`；它不是 `dsp-001` 的状态迁移，也不能复用
Task 101。如果下载后端 A 在第一次执行中自行进行了三次断线重连，则这些尝试都属于
Task 101 内部：Task 101 对外一直是 `Running`，进度可能停在 37% 一段时间后继续；只有 A
最终放弃时才一次性进入 `Failed`。Dispatcher 不感知这三次内部尝试。

### 9.4 人工放行：低权限用户请求高权限安装

假设安装服务注册了窄 operation `apps.install/v1`，并声明
`auth_policy=ZoneUsers` + `approval_policy=InteractiveCallers`：普通用户可以提交
安装请求，但不能直接触发这个高权限 executor。

```text
1. bob（verify-hub 交互会话）dispatch(apps.install/v1, {app_id:"foo"})
   -> 返回 { dispatch_id: "dsp-9a..", status: "PendingApproval" }
   -> 记录持久保存；不评估、不 offer，installer 完全看不到该请求

2. 管理员在 Control Panel 审批面看到队列（list_dispatches(status=PendingApproval)）
   - approve_dispatch("dsp-9a..", note="ok, 周五窗口装")
     -> Queued -> evaluate_target 指派 -> Offered -> installer 重新鉴权后
        幂等建 Task 并 accept -> Accepted(task_id)
   - 或 deny_dispatch("dsp-9a..", note="该应用未过安全评估")
     -> Rejected(approval_denied)，终态；bob 重新申请 = 新 idempotency_key 新 dispatch

3. 审计链：envelope（requested_by=bob, on_behalf_of=bob）不变；
   approval 字段记录 {decision, decided_by, decided_at, note}；
   installer 建的业务 Task 仍以 bob 为业务用户记账。
```

同一个 Target 若把 `approval_policy` 设为 `AllCallers`，则 Agent（zone 可信的
OpenDAN）自主发起的 `apps.install/v1` 同样会停在 `PendingApproval`——这是
"Agent 的危险操作也要人批"的表达方式；`InteractiveCallers` 拦不住可信服务。

## 10. 实施阶段

### M1：Dispatcher 内核（buckyos-api + task_manager 进程）

1. `buckyos-api/src/task_dispatcher.rs`：`DispatchRequestParams` / `DispatchRecord` /
   `DispatchStatus` / `DispatchRejectReason` / `TargetRegistration` / `TargetInstance` /
   `OperationRoute` / `TargetSelection` / `DispatchAuthEnvelope` / `TaskDispatcherClient`。
2. `task_manager` crate 内新增 `dispatcher/` module：store（独立 RDB instance）、
   handler、状态机、evaluate_target、timer 堆、启动恢复、kevent 发布。
3. `/kapi/task-dispatcher` 挂载；scheduler `system_config_builder` 追加 RDB instance
   与授权配置。
4. 单元测试：显式 Target / 默认路由解析与切换、Target 黏着、Task 失败不触发重新执行、
   dispatch/claim/accept/reject/cancel/renew/expiry/幂等键/epoch 校验/Uncertain 门控。

### M2：OpenDAN Dispatch Target Adapter + 删除伪 inbox

1. OpenDAN 实现 Target 注册、instance attach/renew、claim/accept 循环（SDK 封装）。
2. OpenDAN 持久 `dispatch_id -> task_id` 幂等存储；`AgentDelegateTaskData.request`
   增加 `dispatch_id`、`target_agent_id`。
3. 删除 `agent_task_executor` 的跨 owner 扫描路径与 `worksession-task-test` 直投方式；
   owner-only recovery（`app_id=opendan` + 非终态）保留。
4. 打通 UI/Workflow 侧最小调用方：`dispatch` + `get_dispatch` + 目标 Task 只读观察。

### M3：故障注入验收

覆盖：Target 离线提交、上线领取；OpenDAN 重启恢复；offer lease 超时 redelivery；
accept ACK 丢失重放；cancel 与 accept 竞态；`Uncertain` 进入与 `resolve_uncertain`；
任务暂停后切换默认路由再恢复仍命中原 Target、旧幂等键仍命中原 Target；Dispatcher 进程重启
（timer/记录恢复）；目标 Task 失败后不产生新 offer/Dispatch/Task；Executor 内部重试期间
Task 始终保持 Running；kevent 全丢时兜底轮询仍收敛。

### M4：人工放行（审批门）——2026-08-07 增量，同日实施完成

1. 协议（buckyos-api）：`DispatchApprovalPolicy` / `DispatchApproval` /
   `DispatchRejectReason::ApprovalDenied` / `approve_dispatch` / `deny_dispatch`
   RPC 与 client 方法；`DispatchStatus::PendingApproval`。均已落地；
   `PendingApproval` 不进 `is_assignable()`，`ApproveDispatchReq` /
   `DenyDispatchReq` 带可选 `note`，返回完整记录。
2. 服务（task_manager/dispatcher）：resolve 落库时按 policy + 直接调用者 token 分级
   决定初始状态；approve -> Queued + evaluate_target；deny -> Rejected 终态；
   `expires_at` due scan 覆盖 `PendingApproval`；**accept/claim/reject 状态守卫显式
   排除 `PendingApproval`**（服务层显式分支 + store 层条件 UPDATE 双重守卫；
   原 late-accept 守卫仅收 Queued/WaitingForTarget/Uncertain 的语义保持不变）；
   `approval` 列（schema v1→v2，`TASK_DISPATCHER_RDB_SCHEMA_VERSION=2`，
   旧库 open 时就地 `ALTER TABLE` 迁移，含 DDL override 过期时的兜底）+
   `/task_dispatcher/approvals` 通道（payload 仅 ids）。
3. 审批人判定：`authenticate()` 保留 verify-hub token 的 `sudo` 位
   （`RequestContext.sudo`）；审批权与 `InteractiveCallers` 的豁免共用同一判定
   `is_approval_admin`（zone 可信或 sudo 会话）；Target owner / 提交者身份
   本身不含审批权。
4. 管理 UI：Control Panel 审批面（与默认后端配置面同属管理面遗留项，未做）。
5. 单元测试（已全部落地，`dispatcher/tests.rs` +6 项）：hold/approve/deny/cancel/
   expire 全迁移；放行后正常指派且 envelope 不变；PendingApproval 不可被 Target
   claim/accept/reject 且不占并发额度；审批人身份与 note 落 `approval` +
   `dispatch_event`；zone 可信与 sudo 会话在 `InteractiveCallers` 下不被 hold、
   `AllCallers` 下被 hold；幂等重放返回 PendingApproval 状态；非管理身份
   approve/deny 被拒；v1 库就地迁移后审批流可用；approvals 提示通道可订阅。

### 影响入口

- `src/kernel/buckyos-api/src/task_dispatcher.rs`（新增）
- `src/kernel/task_manager/src/dispatcher/`（新增 module）
- `src/kernel/task_manager/src/server.rs` / `main.rs`（挂第二个 path）
- `src/kernel/scheduler/src/system_config_builder.rs`
- `src/frame/opendan/src/agent_task_executor.rs`（伪 inbox 删除 + Adapter）
- `src/frame/opendan/src/main.rs`、OpenDAN 持久存储
- RBAC / ServiceDoc / Control Panel 默认后端配置面 / WebUI Task Center（dispatch 观察面）、DV Test

## 11. 验收条件

1. 显式 Target 未注册/未启用/不支持 operation，或省略 Target 时没有有效默认路由，
   dispatch 被明确拒绝。
2. Target 离线时记录持久等待；上线后能领取且不生成重复业务 Task。
3. 相同 `dispatch_id` 重放返回同一个 `target_task_id`；相同 `idempotency_key` 的
   `dispatch()` 重放返回同一个 `dispatch_id`。
4. 低权限调用者不能经 Dispatcher 触发高权限 operation，也不能经 `on_behalf_of`
   提升身份；仅调用 TaskMgr `create_task` 仍然无法触发任何 Target 执行。
5. 旧 epoch 实例的 claim/accept/reject 被拒绝。
6. `Accepted` 后 Dispatcher 不复制目标 Task 状态；UI 经 link 查询目标 Task。
7. Task Service 与 Dispatch Center 同进程但数据模型、RPC、授权规则独立；Dispatcher
   完全不启动时 TaskMgr 与既有业务接口不受影响。
8. `Accepted` 前的 offer redelivery 始终复用同一 `dispatch_id` 并最多创建一个目标 Task；
   它不得延伸为 `Accepted` 后的业务执行 retry。
9. kevent 不可用时，系统仅退化为轮询延迟，不丢失、不重复。
10. 同一 operation 可以注册多个 Target；省略 `target_id` 时命中管理员配置的默认 Target，
    修改默认路由只影响新 dispatch，既有记录和相同幂等键重放仍指向原 Target。
11. operation 默认路由选择与 TargetInstance 选择可分别审计，Target owner 不能自行把自己
    设置为系统默认后端。
12. 一条记录选定 Target A 后，在 Queued/Offered 重投、Dispatcher 重启及 Target 业务 Task
    暂停/恢复期间始终保持 A；把默认路由切换到 B 只影响新 dispatch。同一 Target 内允许
    TargetInstance 故障转移，但不得表现为跨 Target 漂移。
13. 目标 Task 进入 `Failed` 后保持终态，Dispatcher 不重新 offer、不重新选择 Target、
    不创建第二个 Task；用户重新执行必须得到新的 `dispatch_id` 与 `target_task_id`。
14. Executor 内部 retry 不新增 Task 状态：对外保持 `Running`，允许进度暂时停滞；一旦
    写入 `Failed`，同一 Task 不得回到 `Running`。
15. （M4）`approval_policy` 命中的提交停在 `PendingApproval`：不进入评估、不产生
    offer、不占并发额度，Target 无法经 claim/accept/reject 任何路径接触它。
16. （M4）approve 后记录进入正常指派且 auth envelope 逐字节不变；deny 得到
    `Rejected(approval_denied)` 终态，重新提交必须新 `idempotency_key`。
17. （M4）非管理身份不能 approve/deny；审批人身份与备注可审计
    （`approval` 字段 + `dispatch_event`），且审批不豁免 Target 业务鉴权。
18. （M4）`InteractiveCallers` 下 zone 可信调用者与 sudo 会话不被 hold；
    `AllCallers` 下包括可信服务在内的所有提交都被 hold；`expires_at` 到期的
    待审批记录转 `Expired`。

## 12. 决策记录

| 决策项 | 结论 |
| --- | --- |
| 名称与 RPC path | 概念名 Task Dispatch Center；服务名/路径 `task-dispatcher`、`/kapi/task-dispatcher` |
| 存储 | 独立 RDB instance `task-dispatcher-main`，与 task-mgr-main 无共享表/schema/join |
| 分发模型 | Dispatcher 集中指派（RoundRobin/LeastLoaded），排除实例 FCFS 抢占；claim 只是传输 |
| operation 后端选择 | 同一 operation 可由多个 Target 实现；显式 `target_id` 固定后端，省略时按管理员 `OperationRoute` 固化默认 Target |
| Target 黏着 | `DispatchRecord.target_id` 创建后不可变；默认路由更新、offer redelivery、暂停/恢复均不得跨 Target 漂移 |
| 两级选择边界 | `OperationRoute` 选择异构 Target；`DeliveryPolicy` 只在该 Target 的等价 TargetInstance 中选择，不与 Scheduler 节点放置混用 |
| 领取通道 | kevent 通知加速 + `claim_next` 拉取权威 + 低频兜底轮询；通知与兜底汇入同一处理路径 |
| `Accepted` 终态 | 是；拿到 `target_task_id` 即完成交接，目标 Task 是 TaskMgr 标准 Task；业务进展只经 link 观察，不做 projection |
| 失败与重新执行 | Dispatcher 无业务执行 retry；目标 Task 的 `Failed` 是终态，重新执行必须创建新 Dispatch 与新 Task；Executor 内部 retry 对外保持 Running |
| TargetRegistration 真相源 | 已认证 zone 可信 Service 身份 + 其可信配置；系统级 capability 需要时再加 system-config allowlist |
| 跨 owner link 查询 | `get_dispatch` 给 `target_task_id`，Task data `request.dispatch_id` 反查；写操作走 Target 业务接口 |
| 首个试点 Target | OpenDAN `agent.delegate/v1`（App Install 明确不做试点） |
| 版本安排 | 实施提前，不绑定 Workflow 版本；Workflow 后续作为调用者接入 |
| operation 版本 | 版本并入 operation 字符串主版本（`name/v1`），不设独立 schema_version 字段 |
| 幂等门控 | `IdempotencyContract::IdempotentAccept` 是 offer redelivery 的前提；`None` 契约过期进 `Uncertain` |
| 人工放行建模（2026-08-07 增量） | 分发前状态 `PendingApproval`（审批门），不是手工指派：approve 只放行，实例选择仍走集中指派，不能改 Target/实例（黏着不破）；换后端 = deny/cancel 后新 dispatch |
| 审批策略 | per-target `DispatchApprovalPolicy::{Never, InteractiveCallers, AllCallers}`，默认 `Never`；判定只看直接调用者 token 分级（zone 可信 / sudo 会话豁免 `InteractiveCallers`）；per-operation 覆盖列为后续扩展 |
| 审批与身份 | approve/deny 只推进状态机：envelope 不变、不提权、不豁免 Target 业务鉴权；审批人落 `approval` 字段与 `dispatch_event`；审批权与 Target owner 身份、提交者身份都不挂钩 |
| 审批门可见性边界（2026-08-07 实施定案） | 门封死**接收通道**：未放行记录不评估、不 offer、不发 target 通知、claim/accept/reject 全拒；zone 可信调用者 get/list 只读可见性遵循既有查询授权不收窄；对 owner 隐藏未放行 input 属读取面脱敏，列后续扩展（§7.1） |

## 13. 非目标

1. 不保存业务 Task 的完整进度、checkpoint 或输出；不成为目标 Task 状态的第二真相源。
2. 不代替 Target 做业务鉴权；不因业务执行失败自动重启、重新 dispatch 或选择 fallback。
3. 不实现 Workflow DSL、条件分支、补偿或 schedule 语义。
4. 不承担节点资源放置；不做通用分布式事务或 exactly-once 业务执行承诺。
5. 不成为 Service API gateway 或所有长任务的统一入口；在线业务 RPC 不迁移。
6. 不允许任意服务通过注册动态获得系统级 capability。
7. 不为旧 runner/task-ready 协议提供兼容层。
8. v1 不根据单次输入内容自动识别“电子书还是 CAD”并动态挑选后端，也不在异构 Target
   之间做隐式失败切换；它提供管理员可配置的确定默认值和调用者显式覆盖能力。
9. 不提供已有任务的跨 Target 热迁移。若未来需要把 A 上的任务迁到 B，必须定义 checkpoint
   兼容性、幂等、副作用和审计协议，不能复用普通 pause/resume 或默认路由切换表达。
10. 审批门不做审批工作流：无多级会签、委托审批、条件自动批；一条记录只有一次
    approve/deny 决策。per-operation 级 `approval_policy` 覆盖是可能的后续扩展，
    v1 只做 Target 级。
11. `approve_dispatch` 不提供改指 Target 或实例的能力；"批的时候换后端"不存在，
    等价操作是 deny/cancel 后以新 `idempotency_key`（可显式 `target_id`）重新
    dispatch。
12. Dispatcher 审批不用于业务执行中的人工介入；执行中的审批/补充输入仍属 Target
    业务层（如 OpenDAN `human.input` 子任务与 `TaskStatus::WaitingForApproval`）。
