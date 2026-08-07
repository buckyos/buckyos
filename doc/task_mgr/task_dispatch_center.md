# Task Dispatch Center 设计

- 状态：设计定稿（可实施）
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
  -> Dispatcher.dispatch(target, operation, input)
  -> DispatchRecord 持久保存（Target 离线也不丢）
  -> Target 实例上线/有容量时收到 offer
  -> Target 重新鉴权并幂等创建自己拥有的 Task
  -> Target.accept(dispatch_id, target_task_id)
  -> Caller 通过 target_task_id 只读观察业务进展
```

Dispatcher 只管理**接收之前**的交接生命周期。接收之后，业务 Task 完全归 Target 所有，
遵循 TaskMgr 的"谁创建，谁执行；谁拥有，谁更新"范式（`doc/task_mgr/task_mgr.md` §7）。

### 1.1 与相邻组件的边界

| 组件 | 职责 | Dispatcher 与它的关系 |
| --- | --- | --- |
| TaskMgr | 长任务状态总账 | Dispatcher 不是 TaskMgr 的一部分抽象；同进程部署但独立 store、独立 RPC、独立授权。TaskMgr 对 Dispatcher 零依赖 |
| Scheduler | 节点资源放置 | Dispatcher 不做资源调度；Target 在哪个节点运行由 Scheduler/部署决定 |
| Workflow | 执行图编排 | Workflow 是调用者；DSL、分支、补偿、schedule 语义都不下沉到 Dispatcher |
| 业务 RPC | 在线服务调用 | Target 在线且无需持久交接时，直接调用 Target 业务接口，不经过 Dispatcher |
| MsgCenter | 消息投递 | 消息用于人类沟通与进展展示；接收确认、重试、幂等交接不用消息协议表达 |

### 1.2 使用门槛

只有普通业务 RPC 无法覆盖，并且确实需要以下能力时，才使用 Dispatcher：

1. Caller 提交后，即使 Target 当前离线，请求也必须由独立组件持久保留。
2. Target 的一个或多个实例需要异步领取，并受 lease、capacity、instance epoch 约束。
3. 系统必须处理 offer/accept ACK 丢失、交接重放和 `dispatch_id` 级幂等。
4. 需要独立审计"谁把什么工作交给了哪个 Target"，且交接生命周期独立于业务 Task。

不满足门槛的场景（在线 RPC、Service 自己的后台任务、owner 内部恢复、Task 状态查询、
经 owner 业务接口的取消/重试/审批）继续走既有模式，不迁移。

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
    └── dispatch_target / dispatch_instance / dispatch_record / dispatch_event
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
    pub idempotency_contract: IdempotencyContract,
    pub delivery_policy: DeliveryPolicy,
    pub max_concurrency: u32,           // 该 Target 全局在途 offer 上限
    pub enabled: bool,
}

pub struct OperationDescriptor {
    pub operation: String,              // 含主版本，如 "agent.delegate/v1"
    pub input_schema_ref: Option<String>, // 可选 schema 引用，供校验与 UI
}

pub enum IdempotencyContract {
    IdempotentAccept,   // 相同 dispatch_id 重放返回相同 target_task_id（自动 re-offer 的前提）
    None,               // 不承诺幂等接收：offer lease 过期后进入 Uncertain，人工/业务恢复
}

pub struct DeliveryPolicy {
    pub offer_lease_ms: u64,            // 默认 30_000
    pub max_offer_attempts: u32,        // 默认 10；耗尽 -> Expired(detail=delivery_exhausted)
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
- 未注册 Target 的 dispatch 请求立即失败，不会落成无人负责的等待记录。

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

调用者提交：

```rust
pub struct DispatchRequestParams {
    pub target_id: String,
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

- 信封创建后不可修改。delivery retry（re-offer）复用同一个 `dispatch_id` 与信封。
- 调用者重试 `dispatch()`（网络错误重放）依赖 `idempotency_key`：
  `(target_id, operation, requested_by_user, requested_by_app, idempotency_key)`
  唯一索引，命中即返回既有 `dispatch_id`，不产生第二条记录。
- 业务重试（Rejected/Expired 后再来一次）必须是新 dispatch：新 `idempotency_key`、
  新 `dispatch_id`。delivery retry 与 business retry 的分界线就是 `Accepted` 之前 / 由
  Caller 显式重新提交。

`dispatch_id` 为服务端生成的全局唯一字符串（如 `dsp-{uuid}`），是跨系统引用与幂等
交接的锚点，不用自增整数。

### 3.4 DispatchRecord 与状态机

```rust
pub struct DispatchRecord {
    pub dispatch_id: String,
    pub target_id: String,
    pub operation: String,
    pub status: DispatchStatus,
    pub input: serde_json::Value,
    pub auth: DispatchAuthEnvelope,
    pub offer_instance_id: Option<String>,   // 当前指派实例
    pub offer_lease_expires_at: Option<u64>,
    pub offer_attempts: u32,
    pub target_task_id: Option<i64>,         // Accepted 后回填
    pub reject_reason: Option<DispatchRejectReason>,
    pub message: Option<String>,             // 面向 UI 的说明 / detail
    pub created_at: u64,
    pub updated_at: u64,
}
```

状态机独立于 `TaskStatus`，不复用：

```text
Queued
  -> WaitingForTarget                  # target 已注册但无可用实例/容量
  -> Offered(instance_id, lease)       # 已指派给某实例，等待 claim/accept
  -> Accepted(target_task_id)          # 正常终态

Queued | WaitingForTarget | Offered
  -> Rejected(reason)                  # Target 稳定拒绝，终态
  -> Expired                           # expires_at 到期或 delivery 耗尽，终态
  -> Canceled                          # Caller 取消，终态

Offered（offer lease 过期）
  -> IdempotentAccept 契约  -> 回到 Queued，重新评估指派（offer_attempts + 1）
  -> None 契约              -> Uncertain

Uncertain
  -> resolve_uncertain -> Accepted(task_id) | Canceled   # 管理/业务恢复，非自动
```

状态语义要点：

- `Accepted` 是 Dispatcher 的正常终态。之后 Dispatcher 不复制、不投影目标 Task 的
  执行状态机；业务进展经 `target_task_id` link 观察（§8）。
- `Rejected` 只表达稳定拒绝：`schema_mismatch` / `auth_denied` / `policy_denied` /
  `precondition_failed` / `unsupported_operation` / `target_disabled` / `invalid_input`。
  容量不足、实例离线**不是**拒绝，应停留在 `WaitingForTarget`。
- `Expired` 覆盖两种情况，`message` 区分：请求信封 `expires_at` 到期；
  `max_offer_attempts` 耗尽（`delivery_exhausted`）。
- `Uncertain` 表示 Target 可能已创建 Task 但确认丢失且无幂等契约兜底。禁止自动
  re-offer，禁止盲目创建第二个业务 Task。
- 取消与 accept 的竞态在 Dispatcher 侧原子裁决：accept 到达时记录若已 `Canceled` /
  `Expired`，accept 返回对应错误；Target 收到该错误后必须取消自己刚创建的本地 Task
  （见 §6.3 接收侧契约）。

### 3.5 存储表结构

RDB instance `task-dispatcher-main`，schema version 1：

| 表 | 关键列 | 索引 |
| --- | --- | --- |
| `dispatch_target` | `target_id` PK, `owner_user_id`, `owner_app_id`, `registration`(JSON), `enabled`, `created_at`, `updated_at` | — |
| `dispatch_instance` | PK(`target_id`,`instance_id`), `lease_epoch`, `lease_expires_at`, `capacity`, `available_capacity`, `attached_at`, `renewed_at` | `idx_di_lease`(`lease_expires_at`) |
| `dispatch_record` | `dispatch_id` PK, `target_id`, `operation`, `status`, `input`(JSON), `auth`(JSON), `idempotency_key`, `requested_by_user`, `requested_by_app`, `on_behalf_of`, `offer_instance_id`, `offer_lease_expires_at`, `offer_attempts`, `target_task_id`, `reject_reason`, `message`, `expires_at`, `created_at`, `updated_at` | UNIQUE(`target_id`,`operation`,`requested_by_user`,`requested_by_app`,`idempotency_key`)；`idx_dr_target_status`(`target_id`,`status`,`created_at`)；`idx_dr_due`(`status`,`expires_at`)；`idx_dr_requester`(`requested_by_user`,`requested_by_app`,`created_at` DESC) |
| `dispatch_event` | `id` PK 自增, `dispatch_id`, `ts`, `from_status`, `to_status`, `instance_id`, `detail` | `idx_de_dispatch`(`dispatch_id`,`ts`) |

`dispatch_event` 是投递审计的最小实现：谁、代表谁、经哪个 workflow/step、向哪个
Target 投递了什么，在 `dispatch_record.auth` 中；状态迁移轨迹在 `dispatch_event` 中。

## 4. RPC 协议

`POST /kapi/task-dispatcher`，方法分三组，授权策略互相独立（§7）：

### 4.1 Caller 侧

| Method | 说明 |
| --- | --- |
| `dispatch` | 提交 `DispatchRequestParams`，返回 `{dispatch_id, status}`；idempotency_key 命中时返回既有记录 |
| `get_dispatch` | 按 `dispatch_id` 查询记录（含 `target_task_id`） |
| `list_dispatches` | 按 requester / target / status / 时间过滤；普通调用者只能看到自己提交的记录 |
| `cancel_dispatch` | `Accepted` 之前取消；`Accepted` 之后返回错误并提示走 Target 业务接口 |

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
| `resolve_uncertain` | 把 `Uncertain` 裁决为 `Accepted(task_id)` 或 `Canceled`；zone owner / 可信服务专用 |
| `list_targets` / `get_target` | 注册面观察 |

SDK（`TaskDispatcherClient`）为 Target 侧提供组合封装：`run_target_instance` 内部
处理 attach → 订阅 kevent → claim/accept 循环 → renew → 兜底轮询，业务方只实现
"验证 + 幂等建 Task + 返回 task_id" 回调。

## 5. 分发模型：指派而非抢占

**归属判定始终在 Dispatcher 内部按 policy 完成，不因谁先 poll 而改变。**

设计上排除 FCFS 抢占模型（多实例自由 claim 未指派记录、先到先得）：dispatch policy
一旦下放给实例就无法收回，且抢占模型无法表达 capacity、epoch 和定向恢复。实例选择
是 Dispatcher 的集中决策：

```text
evaluate_target(target_id)：           # 唯一的指派路径
  输入触发点（任一）：
    - dispatch 落库
    - attach / renew / detach / capacity 变化
    - offer lease 过期、expires_at 到期（timer）
    - claim_next 到达（仅作为评估触发点）
    - 启动恢复扫描 / 低频兜底 sweep
  动作：
    1. 取该 target 下 Queued/WaitingForTarget 的 due 记录
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
```

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
   offer lease 过期不自动 re-offer，进入 `Uncertain` 等待人工/业务恢复。

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
| `dispatch` | 通过 target `auth_policy` 判定。`requested_by_*` 强制 = token 身份。`on_behalf_of`：普通调用者强制 = token.sub（代填即拒绝）；zone 可信调用者可代填已鉴权业务用户 |
| `get_dispatch` / `list_dispatches` | zone 可信调用者不受限；普通调用者只能查看 `requested_by` 或 `on_behalf_of` 是自己的记录 |
| `cancel_dispatch` | 记录的 requester、on_behalf_of 用户或 zone 可信调用者 |
| `register_target` / `disable_target` | 仅 zone 可信调用者；owner 绑定验签身份，更新时校验 owner 一致 |
| `attach/renew/detach/claim/accept/reject` | 仅 target owner 身份（`owner_user_id`/`owner_app_id` 与 token 一致的 zone 可信调用者） |
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
- 取消、重试、审批等写操作调用 Target 的业务接口（如 OpenDAN 的
  `pause/resume/cancel_agent_task`），不经过 Dispatcher，也不直接写 TaskMgr。
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

## 10. 实施阶段

### M1：Dispatcher 内核（buckyos-api + task_manager 进程）

1. `buckyos-api/src/task_dispatcher.rs`：`DispatchRequestParams` / `DispatchRecord` /
   `DispatchStatus` / `DispatchRejectReason` / `TargetRegistration` / `TargetInstance` /
   `DispatchAuthEnvelope` / `TaskDispatcherClient`。
2. `task_manager` crate 内新增 `dispatcher/` module：store（独立 RDB instance）、
   handler、状态机、evaluate_target、timer 堆、启动恢复、kevent 发布。
3. `/kapi/task-dispatcher` 挂载；scheduler `system_config_builder` 追加 RDB instance
   与授权配置。
4. 单元测试：dispatch/claim/accept/reject/cancel/renew/expiry/幂等键/epoch 校验/
   Uncertain 门控。

### M2：OpenDAN Dispatch Target Adapter + 删除伪 inbox

1. OpenDAN 实现 Target 注册、instance attach/renew、claim/accept 循环（SDK 封装）。
2. OpenDAN 持久 `dispatch_id -> task_id` 幂等存储；`AgentDelegateTaskData.request`
   增加 `dispatch_id`、`target_agent_id`。
3. 删除 `agent_task_executor` 的跨 owner 扫描路径与 `worksession-task-test` 直投方式；
   owner-only recovery（`app_id=opendan` + 非终态）保留。
4. 打通 UI/Workflow 侧最小调用方：`dispatch` + `get_dispatch` + 目标 Task 只读观察。

### M3：故障注入验收

覆盖：Target 离线提交、上线领取；OpenDAN 重启恢复；offer lease 超时 re-offer；
accept ACK 丢失重放；cancel 与 accept 竞态；`Uncertain` 进入与 `resolve_uncertain`；
Dispatcher 进程重启（timer/记录恢复）；kevent 全丢时兜底轮询仍收敛。

### 影响入口

- `src/kernel/buckyos-api/src/task_dispatcher.rs`（新增）
- `src/kernel/task_manager/src/dispatcher/`（新增 module）
- `src/kernel/task_manager/src/server.rs` / `main.rs`（挂第二个 path）
- `src/kernel/scheduler/src/system_config_builder.rs`
- `src/frame/opendan/src/agent_task_executor.rs`（伪 inbox 删除 + Adapter）
- `src/frame/opendan/src/main.rs`、OpenDAN 持久存储
- RBAC / ServiceDoc / WebUI Task Center（dispatch 观察面）、DV Test

## 11. 验收条件

1. 未注册 Target 或未注册 operation 的 dispatch 被明确拒绝。
2. Target 离线时记录持久等待；上线后能领取且不生成重复业务 Task。
3. 相同 `dispatch_id` 重放返回同一个 `target_task_id`；相同 `idempotency_key` 的
   `dispatch()` 重放返回同一个 `dispatch_id`。
4. 低权限调用者不能经 Dispatcher 触发高权限 operation，也不能经 `on_behalf_of`
   提升身份；仅调用 TaskMgr `create_task` 仍然无法触发任何 Target 执行。
5. 旧 epoch 实例的 claim/accept/reject 被拒绝。
6. `Accepted` 后 Dispatcher 不复制目标 Task 状态；UI 经 link 查询目标 Task。
7. Task Service 与 Dispatch Center 同进程但数据模型、RPC、授权规则独立；Dispatcher
   完全不启动时 TaskMgr 与既有业务接口不受影响。
8. delivery retry（Accepted 前，同 dispatch_id）与 business retry（新 dispatch）有
   可测试的明确分界。
9. kevent 不可用时，系统仅退化为轮询延迟，不丢失、不重复。

## 12. 决策记录

| 决策项 | 结论 |
| --- | --- |
| 名称与 RPC path | 概念名 Task Dispatch Center；服务名/路径 `task-dispatcher`、`/kapi/task-dispatcher` |
| 存储 | 独立 RDB instance `task-dispatcher-main`，与 task-mgr-main 无共享表/schema/join |
| 分发模型 | Dispatcher 集中指派（RoundRobin/LeastLoaded），排除实例 FCFS 抢占；claim 只是传输 |
| 领取通道 | kevent 通知加速 + `claim_next` 拉取权威 + 低频兜底轮询；通知与兜底汇入同一处理路径 |
| `Accepted` 终态 | 是；业务进展经 `dispatch_id + target_task_id` link 观察，不做只读 projection |
| TargetRegistration 真相源 | 已认证 zone 可信 Service 身份 + 其可信配置；系统级 capability 需要时再加 system-config allowlist |
| 跨 owner link 查询 | `get_dispatch` 给 `target_task_id`，Task data `request.dispatch_id` 反查；写操作走 Target 业务接口 |
| 首个试点 Target | OpenDAN `agent.delegate/v1`（App Install 明确不做试点） |
| 版本安排 | 实施提前，不绑定 Workflow 版本；Workflow 后续作为调用者接入 |
| operation 版本 | 版本并入 operation 字符串主版本（`name/v1`），不设独立 schema_version 字段 |
| 幂等门控 | `IdempotencyContract::IdempotentAccept` 是自动 re-offer 的前提；`None` 契约过期进 `Uncertain` |

## 13. 非目标

1. 不保存业务 Task 的完整进度、checkpoint 或输出；不成为目标 Task 状态的第二真相源。
2. 不代替 Target 做业务鉴权；不把业务执行失败自动解释为需要重新 dispatch。
3. 不实现 Workflow DSL、条件分支、补偿或 schedule 语义。
4. 不承担节点资源放置；不做通用分布式事务或 exactly-once 业务执行承诺。
5. 不成为 Service API gateway 或所有长任务的统一入口；在线业务 RPC 不迁移。
6. 不允许任意服务通过注册动态获得系统级 capability。
7. 不为旧 runner/task-ready 协议提供兼容层。
