# TaskMgr 2.0 设计

- 状态：设计基线
- 版本：2.0
- 兼容性：beta 2.2 breaking change，不提供旧 TaskMgr/Dispatcher 协议兼容层
- 目标读者：TaskMgr、Task Dispatch Center、Scheduler、Workflow、OpenDAN、Task Center UI 开发者
- 相关文档：
  - `doc/task_mgr/task_mgr.md`：当前 TaskMgr 1.x 实现说明
  - `doc/task_mgr/task_dispatch_center.md`：当前 beta 2.2 Dispatcher 实现与历史决策记录；不是
    TaskMgr 2.0 的并列规范，2.0 语义以本文为准
  - `doc/task_mgr/task data schema.md`：当前 TaskData 类型资产；2.0 实施时迁移为 Task Schema
  - `doc/arch/scheduler/scheduler.md`：Scheduler 的纯决策、原子写回和无自有队列边界

## 1. 设计目标

本文是 **TaskMgr 2.0 子系统的总设计文档**，覆盖 Task Core 和 Task Dispatch Center，不是只设计
Task 表或 `/kapi/task-manager`。内部模块关系为：

```text
TaskMgr 2.0 subsystem
├── Task Core
│   ├── Task model/state/tree/ACL/result/event
│   └── /kapi/task-manager + task-mgr-main RDB
└── Task Dispatch Center
    ├── route/queue/delivery/recovery/runner registration
    └── /kapi/task-dispatcher + task-dispatcher-main RDB
```

本文后续在与 Dispatcher 对照时使用 `Task Core` 指底层 Task 真相源；单独使用 `TaskMgr 2.0` 则指
包含两个模块的完整子系统。Dispatcher 对 Task Core 是单向依赖，Task Core 不反向依赖
Dispatcher。独立 RPC、RDB 和授权边界是为了约束耦合及允许内部独立演进，不表示 Dispatcher
位于 TaskMgr 2.0 之外，也不表示存在第二套面向用户的工作对象。

TaskMgr 2.0 为整个 BuckyOS 提供一个统一、稳定、可寻址的 `Task` 抽象。UI、Agent、系统服务、
Scheduler 和 Workflow 都可以使用同一个 `task_id` 查看一项工作的输入、当前状态、控制能力和
最终结果，并生成稳定链接：

```text
/tasks/{task_id}
```

一个 Task 可以从“已经承诺处理，但尚未找到执行者”一直存在到“已经由机器或人完成”。系统
不会因为分发、换 Runner、等待授权或暂停而切换到另一个公开对象，也不会在 Dispatcher activate
时产生第二个业务 Task ID。

TaskMgr 2.0 的核心职责是：

1. 生成并维护稳定、不可复用的 Task ID。
2. 保存 Task 的不可变 Input 和一次性提交的 Result。
3. 保存适合 UI 展示和通用控制的组合状态。
4. 保存当前执行绑定，包括 App Runner 或一组可提交结果的人。
5. 维护 Task Tree、批量控制请求和树级访问策略。
6. 通过 revision、runner epoch 和原子 Commit 提供分布式并发保护。
7. 提供状态查询、树查询、持久事件和 KEvent 加速通知。
8. 作为资源消耗、审计日志和外部系统记录的稳定关联 ID，但不承担计费和配额计算。

TaskMgr 不是 Workflow Engine。它不解释 Task 为什么存在父子关系，不执行表达式树、条件分支、
补偿、业务重试或资源放置算法。Workflow 可以把表达式树的一次执行投影成 Task Tree，但这些
Task 仍然只使用本文定义的通用状态、控制和权限语义。

### 1.1 为什么 Dispatcher 不能只做上层小重构

beta 2.2 先在旧 TaskMgr 之上增加 Dispatcher，验证了队列、Target route、lease、审批和离线交接
的必要性，也暴露了底层模型限制：旧 Task 把创建、拥有和执行绑定得过紧，Task 在 Target 接收时
才真正创建，导致 Caller intent 与 Target Task 成为两个对象；单一 status、可变 data 和旧权限
模型也无法稳定表达 `Promised -> bind -> Accepted -> Running`、换实例 fencing 及一次性 Result。

这些问题不能通过继续扩展 `DispatchRecord` 或增加传输状态解决，否则 Dispatcher 会逐渐复制
Task 的身份、权限和生命周期。TaskMgr 2.0 因此同时重做 Task Core 的身份、不可变性、组合状态、
executor 和控制协议，再让内部 Dispatch Center 只保存投递所需的队列状态。上一版 Dispatcher
仍是重要的实现输入，但不再约束 2.0 的底层模型或协议兼容性。

## 2. 核心设计原则

### 2.1 只有一种公开运行时对象

系统对外只有 `Task`。`Promised Task`、`Running Task`、人工任务和 App 执行任务都是同一个
类型在不同组合状态下的表现。

以下记录不是新的公开 Task 类型：

- Task Schema 是输入、输出和 UI 的版本化契约。
- ACL Grant 是 Task 的权限配置。
- Task Event 是 Task 的内部持久变更历史。
- Dispatch Record、Delivery Attempt 和 delivery lease 是 Dispatcher 的内部队列记录。
- Task Note 是不参与执行状态的旁路注释。

调用者通常只需要保存和传播 `task_id`。

### 2.2 Task 是工作状态的公共投影，不是业务编排器

TaskMgr 只保存对展示、控制、安全和恢复有意义的事实：

- 做什么：`schema + input`。
- 谁创建：`creator`。
- 当前由谁处理：`executor`。
- 做到哪里：组合状态、progress 和 message。
- 可以怎么控制：`control_profile`。
- 最终产生什么：`outcome + result/error`。
- 与哪些 Task 有父子关系：`parent_id + root_id`。

TaskMgr 不保存或解释“为什么选择这个 Runner”“为什么下一步依赖这个子 Task”“失败后应该
重试几次”等业务决策。

### 2.3 不可变事实与可变投影分离

创建后不可变：

- `task_id`、`name`、`creator`、`created_at`。
- `schema_id`、`schema_version`。
- `input`、`input_digest`。
- `idempotency_key`。
- 可选 `origin_ref`；仅作为不透明的创建来源引用，TaskMgr 不解释来源系统的状态。
- `parent_id`、`root_id`、`child_control_policy`。
- `policy_preset`、`permission_boundary`；后续授权变化通过显式 Grant 表达。
- `retry_of`、`supersedes` 等因果链接。

受协议约束可变：

- 当前 phase、wait reason、progress、message。
- 当前 executor、runner epoch。
- Runner 声明的 control profile。
- 当前待处理 control request。
- Assignees 和 ACL；修改必须有权限、CAS 和审计事件。

只能写入一次：

- 成功 Task 的 `result`。
- 终态 `outcome`、`error`、`completed_by`、`completed_at`。

TaskMgr 不提供能够绕过上述规则的通用 `update_task(status, data)` 接口。

### 2.4 TaskMgr 约束协议不变量，不判断业务可行性

数据库不使用 trigger 或枚举约束实现完整状态机。TaskMgr command/service 层负责保证：

- Result 只能 Commit 一次。
- Terminal 是吸收态。
- Runner 写入必须匹配当前 runner epoch 和 revision。
- Human Commit 必须来自当前 Assignees。
- 旧 Runner、重复提交和越权控制失败关闭。

TaskMgr 不判断底层操作此刻是否真的能暂停、取消或恢复。当前 Runner 通过 control profile
表达能力，并对控制请求返回 applied 或 rejected。

### 2.5 不自动业务重试

TaskMgr 不把 Terminal Task 重新变成 Running，也不自动创建重试。

- 同一个 idempotency key 重投返回原 Task。
- 重新执行使用新的 idempotency key 创建新 Task，并可记录 `retry_of`。
- 修改 Input 等价于取消或 supersede 原 Task 后创建新 Task。
- Runner 进程崩溃后的投递重放、delivery lease 恢复不是业务重试，可以继续使用同一 Task。

## 3. Task 身份、URL 与资源路径

`TaskId` 是 TaskMgr 生成的 URL-safe opaque string。调用方不得解析其格式、推断创建时间或
父子关系，也不得自行构造 ID。

Task 的 canonical URL 永远是：

```text
/tasks/{task_id}
```

父子关系不编码进 Task ID 或 canonical URL。权限系统可以把 `parent_id/root_id` 解释成逻辑
资源层级，但资源路径是授权计算的派生关系，不是 Task 的身份。例如：

```text
task:{root_id}/.../task:{task_id}
```

这样可以同时满足：

- UI 深链接稳定。
- 可以只凭 task_id 访问任意子 Task。
- ACL 可以表达 Self、Subtree 和 Tree。
- Task ID 不泄露树结构，也不因授权实现变化而变化。

## 4. Task 核心模型

下面是逻辑模型，不要求 Rust 和 RDB 严格按相同嵌套方式保存：

```rust
pub struct Task {
    // Stable identity and immutable relationship
    pub task_id: TaskId,
    pub name: String,
    pub parent_id: Option<TaskId>,
    pub root_id: TaskId,
    pub child_control_policy: ChildControlPolicy,

    // Immutable contract and payload
    pub schema_id: String,
    pub schema_version: u32,
    pub input: serde_json::Value,
    pub input_digest: String,

    // Immutable creation facts
    pub creator: ActorRef,
    pub idempotency_key: String,
    pub origin_ref: Option<TaskOriginRef>,
    pub retry_of: Option<TaskId>,
    pub supersedes: Option<TaskId>,

    // Current execution binding
    pub executor: TaskExecutor,
    pub runner_epoch: u64,

    // Composite task state
    pub phase: TaskPhase,
    pub wait_reason: Option<TaskWaitReason>,
    pub pending_control: Option<TaskControlRequest>,
    pub control_profile: TaskControlProfile,

    // Presentation snapshot
    pub progress: Option<serde_json::Value>,
    pub message: Option<String>,

    // Immutable completion once terminal
    pub outcome: Option<TaskOutcome>,
    pub result: Option<serde_json::Value>,
    pub error: Option<TaskError>,
    pub completed_by: Option<ActorRef>,

    // Access and concurrency
    pub policy_preset: String,
    pub permission_boundary: bool,
    pub revision: u64,

    pub created_at: u64,
    pub updated_at: u64,
    pub completed_at: Option<u64>,
    pub archived_at: Option<u64>,
}
```

### 4.1 ActorRef

```rust
pub struct ActorRef {
    pub user_id: String,
    pub app_id: String,
    pub app_instance_id: Option<String>,
}

pub struct TaskOriginRef {
    pub kind: String,
    pub id: String,
}
```

`creator.user_id/app_id` 必须来自通过验证的调用上下文，不能相信 payload。`app_instance_id`
主要用于执行和审计；它的字符串格式可以包含 app_id，但权限判断必须以注册信息和验签上下文
为准，不能仅靠解析字符串获得身份。

`origin_ref` 是可选、不透明的审计链接，例如 `{kind: "task-dispatcher", id: "dsp-..."}`。
TaskMgr 只保证它创建后不可变和同 kind/id 唯一，不根据 kind 调用来源系统或解释其状态。

### 4.2 Executor

```rust
pub enum TaskExecutor {
    Unbound,
    App {
        target_id: Option<String>,
        app_id: String,
        app_instance_id: Option<String>,
    },
    HumanSet,
}
```

- `Unbound`：合法的 Promised Task，尚未找到或绑定执行者。
- `App`：由一个 App 执行；`target_id` 是 Dispatcher 选择并冻结的逻辑执行后端，
  `app_instance_id` 是当前实际执行实例。直接由 Service 创建并执行的内部 Task 可以没有
  Dispatcher target；Dispatcher Task 在首次 bind 后必须有 target_id。
- `HumanSet`：由 `task_assignee` 中任意一个当前有效用户提交 Result，首个成功 Commit 获胜。

一个 Task 同一时刻只有一个有效的执行模式。UI App 只是 Human Commit 的提交渠道，不因此
成为该人工 Task 的 Runner。`HumanSet` 必须至少包含一个有效 Assignee；如果移除最后一人，
普通 `update_assignees` 必须拒绝。需要改变 executor kind 时必须走单独的系统级 Reassign
协议，不能靠空 Assignee list 隐式触发 Dispatcher。

对 Dispatcher Task，`target_id` 一旦首次 bind 即不可变；同一逻辑 Target 可以在自己的等价
AppInstance 之间恢复或重新绑定。`app_instance_id` 变化必须增加 Task 的 `runner_epoch`。
Dispatcher 的 instance `lease_epoch` 负责 delivery fencing，Task 的 `runner_epoch` 负责
执行状态和 Result 写入 fencing，两者不能混用。

### 4.3 Creator、Control 权限和 Runner

三者语义不同：

- Creator 是不可变审计事实。
- Control 是 ACL action，不设置独立 controller 字段。
- Runner 是当前 AppInstance executor；HumanSet 则由 Assignees 共同构成可提交集合。

默认 policy 把整棵树的 Control 权限授予 root creator。后续委托、系统管理员介入或 Workflow
自定义权限通过 ACL Grant 表达，不修改 creator，也不需要引入 controller 对象。

2.0 不使用含义不稳定的 `owner` 字段：

- `creator.app_id` 是发起创建的 App。
- `executor.App.app_id` 是当前执行 App。
- `completed_by.app_id` 是实际提交终态的 App 或人工 UI 渠道。

三者可以不同，任何地方都不能再用一个 `app_id` 同时表达创建、执行和结果提交。

## 5. 组合状态模型

### 5.1 Phase

```rust
pub enum TaskPhase {
    Promised,
    Accepted,
    Running,
    Waiting,
    Paused,
    Terminal,
}
```

| Phase | 语义 |
| --- | --- |
| `Promised` | Task 已存在并承诺处理，但没有当前 executor；通常在 Dispatcher 队列中等待 |
| `Accepted` | executor 已绑定，但尚未报告实际开始 |
| `Running` | executor 正在推进工作 |
| `Waiting` | executor 仍负责 Task，但正在等待授权、人类输入、子任务、依赖或外部事件 |
| `Paused` | Runner 已确认暂停；不是 Controller 刚发出 pause 请求 |
| `Terminal` | Task 已以 Succeeded、Failed 或 Canceled 结束 |

`Accepted` 不等于已经运行；Runner 被绑定后可以根据自身恢复逻辑自动改变 Task 状态。

### 5.2 WaitReason

```rust
pub enum TaskWaitReasonKind {
    Dispatch,
    Capacity,
    Authorization,
    HumanInput,
    ChildTask,
    Dependency,
    External,
    Other,
}

pub struct TaskWaitReason {
    pub kind: TaskWaitReasonKind,
    pub code: Option<String>,
    pub related_task_id: Option<TaskId>,
    pub message: Option<String>,
}
```

`kind` 是 Task Center 能稳定理解的通用分类；`code` 是 schema 或 Runner 自己定义的细分原因。
TaskMgr 保存并展示 wait reason，但不解释依赖关系，也不负责解除等待。

`wait_reason` 通常与 `Waiting` 一起使用，也允许出现在 `Promised`，用于说明尚未绑定 Runner 的
原因，例如 Dispatch、Capacity 或 Authorization。它不能单独改变 phase 或执行权。

Dispatcher 使用稳定 code 区分投递细节，例如 `target_offline`、`offer_retry`、`runner_busy` 和
`retry_backoff`；这些 code 只改善 UI 展示，不把 Dispatcher 内部状态变成 TaskMgr 状态机。

### 5.3 Outcome

```rust
pub enum TaskOutcome {
    Succeeded,
    Failed,
    Canceled,
}
```

- `Succeeded` 表示 Runner/Human 成功产生了符合 output schema 的业务结果。
- `Failed` 表示执行失败；错误信息写入 `error`，重新执行必须创建新 Task。
- `Canceled` 表示取消协议已完成。取消可能是 interrupt 或 safe，保证级别记录在终态事件中。

业务上的负面答案不等于执行失败。例如审批 Task 返回 `Reject`，仍然是一个成功产生的 Result。

### 5.4 状态投影

UI 不需要把所有内部字段直接显示给用户，可以按下面规则生成展示状态：

| 条件 | UI 状态 |
| --- | --- |
| `phase = Promised` | Pending / Waiting for runner |
| `phase = Running` 且无控制请求 | Running |
| `pending_control.action = Pause` | Pausing |
| `phase = Paused` 且无控制请求 | Paused |
| `pending_control.action = Resume` | Resuming |
| `pending_control.action = Cancel` | Canceling |
| `phase = Waiting` | Waiting，并显示 wait reason |
| `phase = Terminal` | Succeeded / Failed / Canceled |

`Pausing`、`Resuming` 和 `Canceling` 是组合状态的投影，不是 Runner 已经完成动作的声明。
`Resumed` 是事件，恢复完成后的稳定 phase 是 `Running`。

## 6. 通用控制协议

### 6.1 Runner 声明当前控制能力

```rust
pub struct TaskControlProfile {
    pub pause: ControlAvailability,
    pub resume: ControlAvailability,
    pub cancel: CancelCapability,
    pub updated_at: u64,
}

pub enum ControlAvailability {
    Available,
    Unavailable { reason: Option<String> },
}

pub enum CancelCapability {
    Unavailable { reason: Option<String> },
    Interrupt,
    Safe,
}
```

control profile 是当前 Runner 对自身实现能力的动态描述，不是 TaskMgr 的承诺。只有当前
AppInstance Runner 携带正确 runner epoch 才能修改。HumanSet 默认不支持 pause/resume；取消
由 TaskMgr 在 `result IS NULL` 的 CAS 中关闭该人工 Task，不承诺撤销 Assignee 已经在线下开展
的工作。

- `Interrupt`：停止继续执行，不承诺回滚已发生的副作用。
- `Safe`：Runner 承诺在确认 Canceled 前完成约定的副作用清理。

Safe cleanup 失败时 Runner 必须报告 `Terminal/Failed` 和 error，不能仍确认 Canceled。Interrupt
取消成功后，终态事件必须保留 cancel mode，UI 不得暗示副作用已经回滚。

UI 根据当前 profile 展示按钮和保证级别。profile 与实际执行可能并发变化，因此 Runner 仍可
对一个已经记录的请求返回 rejected，UI 必须展示真实回报。

### 6.2 Control Request

```rust
pub enum TaskControlAction {
    Pause,
    Resume,
    Cancel,
}

pub struct TaskControlRequest {
    pub request_id: String,
    pub action: TaskControlAction,
    pub requested_by: ActorRef,
    pub requested_at: u64,
}
```

控制流程是：

```text
Controller request
    -> TaskMgr 鉴权并原子记录 pending_control
    -> KEvent 通知 Runner
    -> Runner 执行动作
    -> Runner ack applied 或 rejected
    -> TaskMgr 更新 phase、清除 pending_control、写入 durable event
```

TaskMgr 不在请求到达时直接把 Task 写成 Paused 或 Canceled。

同一 Task 最多有一个 pending control。相同 `request_id` 重放返回原结果；Pause/Resume 不能覆盖
另一个未完成请求。Cancel 优先级最高，可以原子 supersede 尚未完成的 Pause/Resume，并产生
`ControlSuperseded` 事件；已经 pending 的 Cancel 不能被其它动作覆盖。

例外：

- `Promised + Unbound` Task 尚无执行副作用。Dispatcher 从持久队列移除成功后，可以作为当前
  队列所有者确认 Cancel，并把 Task 进入 `Terminal/Canceled`。
- `HumanSet` 没有需要响应控制请求的 App Runner。TaskMgr 可以用 revision CAS 在无人 Commit
  Result 时直接结束 Task；取消保证为 interrupt，不承诺撤销人已经在线下进行的工作。

### 6.3 树级批量控制

`request_control(task_id, action, recursive=true)` 是批量产生控制请求，不是批量写最终状态。
TaskMgr 遍历 subtree，按每条父子关系的 control policy 和每个 Task 的 ACL 生成请求，并返回：

```rust
pub struct BatchControlResult {
    pub requested: Vec<TaskId>,
    pub skipped_by_policy: Vec<TaskId>,
    pub denied: Vec<TaskId>,
    pub already_terminal: Vec<TaskId>,
}
```

每个 Runner 分别确认自己的最终状态。TaskMgr 不因部分子任务失败自动决定父任务应该继续、
失败还是补偿。

## 7. Task Tree

### 7.1 关系语义

Subtask 是为了支持 parent 的运行而创建的 Task。TaskMgr 维护：

- `parent_id`：直接父 Task。
- `root_id`：根 Task，由 TaskMgr 根据 parent 推导。
- `child_control_policy`：父级控制请求是否传播到当前子 Task。

`parent_id/root_id` 创建后不可修改。创建子任务需要对 parent 具有 `CreateChild` 权限。

### 7.2 控制传播策略

```rust
pub struct ChildControlPolicy {
    pub follow_pause: bool,
    pub follow_resume: bool,
    pub follow_cancel: bool,
}
```

默认值全部为 `true`，适用于一个父任务带若干协作子任务的常见场景。Workflow 在把表达式树
投影成 Task Tree 时，可以按步骤语义关闭某一项传播。

TaskMgr 不做以下推导：

- 不根据子任务状态自动完成或失败父任务。
- 不要求父任务必须等待所有子任务终态。
- 不自动聚合父任务的 phase 或 progress。
- 不把父任务的 Result 聚合为子任务 Result。
- 不解释兄弟任务之间的依赖。

这些决策属于 parent Runner 或 Workflow。

## 8. 权限模型

### 8.1 为什么使用 ACL/Policy

Task 权限依赖具体 Task、Creator、Runner、Assignees 和树关系，比单纯的传统角色更适合使用
ACL/relationship policy。系统级 RBAC 仍然可以决定某个 principal 是否具备 TaskAdmin 等
平台角色，但具体 Task 的访问由本文的 policy 计算。

### 8.2 Action

```rust
pub enum TaskAction {
    ReadMeta,
    ReadInput,
    ReadResult,
    ReportProgress,
    Control,
    Commit,
    CreateChild,
    Reassign,
    Grant,
    Archive,
}
```

- `Commit` 只允许写入一次性 Result/Outcome，不等价于通用 Write。
- `Control` 只允许产生 pause/resume/cancel 请求。
- `ReportProgress` 只允许更新 progress/message/waiting 等执行投影。
- `Reassign` 允许调整 HumanSet，或授权冻结的逻辑 Target/系统控制面在同一 Target 内重新绑定
  AppInstance；它不允许跨 logical Target 改写 executor。
- `Grant` 允许增加、撤销 ACL Grant，所有变化必须写审计事件。

### 8.3 Subject、Scope 和 DataScope

```rust
pub enum TaskGrantSubject {
    RootCreator,
    Creator,
    Runner,
    Assignees,
    User(String),
    App(String),
    Principal { user_id: String, app_id: String },
    SystemRole(String),
}

pub enum TaskGrantScope {
    SelfOnly,
    Subtree,
    WholeTree,
}

pub enum TaskDataScope {
    MetaOnly,
    Payload,
    Full,
}
```

第一版策略使用 additive allow grants，不引入规则顺序和任意 deny 表达式。Task 上的
`permission_boundary=true` 会阻断祖先 grant 继续向当前 Task 及其后代传播。复杂 Workflow
在创建 Task Tree 时把自身权限计算结果编译成标准 grant 和 boundary；TaskMgr 不理解 Workflow
DSL。

### 8.4 Policy 计算

TaskMgr 对一次授权请求按以下固定顺序计算：

1. 从认证上下文构造精确 principal，并解析其 SystemRole。
2. 计算当前 Task 上的 Creator、RootCreator、Runner、Assignees 关系。
3. 展开 `policy_preset`，加载当前 Task 的显式 active grants。
4. 沿 parent chain 向上收集 scope 覆盖当前 Task 的 grants；遇到当前节点或祖先节点的
   `permission_boundary` 后停止继续继承。
5. 对所有匹配 subject 的 allow actions 求并集。
6. 根据 DataScope 对返回的 Task 字段做服务端裁剪；没有 ReadMeta 时返回 `task_not_found`，
   避免通过错误差异枚举不可见 Task。

Policy v1 没有 deny、优先级和任意条件表达式，因此计算结果与 grant 顺序无关。若 Workflow
需要更复杂的规则，应在创建树时计算并写入标准 grant/boundary，而不是把 Workflow 表达式交给
TaskMgr 运行。

### 8.5 默认策略

默认 preset 为 `collaborative-tree/v1`：

| Subject | Actions | Scope | DataScope |
| --- | --- | --- | --- |
| RootCreator | ReadMeta、ReadInput、ReadResult、Control、CreateChild、Reassign、Grant、Archive | WholeTree | Full |
| Creator | ReadMeta、ReadInput、ReadResult | SelfOnly | Full |
| Runner | ReadMeta、ReadInput、ReportProgress、Commit、CreateChild | SelfOnly | Full |
| Assignees | ReadMeta、ReadInput、Commit、Reassign | SelfOnly | Full |
| Creator / Runner / Assignees | ReadMeta | WholeTree | MetaOnly |

默认 preset 会把最后一行展开成三组 relation grant。它允许参与者看到整棵树的名称、结构、
状态和进度，但只读取自己 Task 的 Input/Result。RootCreator 也不能仅凭创建者身份 Commit
子 Task 的 Result；只有当前 Runner 或 Assignee 拥有 Commit。需要兄弟间共享完整 payload 的
简单业务可以选择更开放的 policy preset；复杂 Workflow 应显式生成 grant。

`RootCreator` 和 `Creator` 默认匹配创建时的精确 `{user_id, app_id}` principal；同一用户通过
其它 App 访问需要显式的 User grant。系统安全审计或恢复角色通过 SystemRole grant 获权，不能
依靠伪造 Creator 身份绕过 policy。

### 8.6 没有独立 Controller 字段

`controller` 不作为 Task 字段或生命周期角色。Controller 是“当前 principal 对该 Task 具有
Control action”的查询结果。默认 root creator 可以控制整棵树，也可以通过 grant 把控制权
委托给用户、App 或系统角色，而 creator 事实保持不变。

## 9. Task Schema、Input 和 Result

### 9.1 Task Schema

系统通过版本化 Task Schema 定义 Input、Result 和通用 UI：

```rust
pub struct TaskSchemaDefinition {
    pub schema_id: String,              // 含主版本，如 agent.command/v1
    pub schema_version: u32,            // 同一主版本内的不可变修订号
    pub input_schema: serde_json::Value,
    pub output_schema: serde_json::Value,
    pub presentation_schema: Option<serde_json::Value>,
    pub allowed_executor_kinds: Vec<TaskExecutorKind>,
    pub user_creatable: bool,
    pub publisher_app_id: String,
    pub enabled: bool,
}
```

规则：

1. `schema_id + schema_version` 一旦发布不可修改；变更产生新 version。
2. 创建 Task 时校验 Input 并冻结 schema version。
3. Commit 时校验 Result；校验失败不改变 Task。
4. output schema 必须明确；Input 确实自由时可以使用允许任意 JSON 的 schema。
5. `presentation_schema` 只负责生成表单和结果展示，不承载权限或执行语义。
6. Task Center 只展示 `user_creatable=true` 且当前用户有 Create 权限的 schema。

`schema_id` 同时作为 Dispatcher 的 operation route key，主版本直接包含在字符串中，例如
`agent.command/v1`；Dispatcher 不再维护另一套 Task 类型名。`schema_version` 是该主版本下
冻结到具体 Task 的 schema registry 修订号，不参与 OperationRoute 的主版本选择。破坏输入或
输出兼容性的变化必须发布新的 `/vN` schema_id，不能只增加 schema_version。

当前 `TaskDataType`、`TypedTaskData` 和 `request/progress/result` 结构是迁移 Task Schema 的主要
资产。2.0 不再允许通过 merge patch 修改同一个 `data.result`。

### 9.2 Runner Registration 与 UI Catalog 分离

- Runner Registration 声明某个 Target/App 暴露的投递函数及其支持的
  `schema_id/schema_version range`，供 Dispatcher 主动调用；至少要同时兼容 Input 和 Result
  contract。逻辑结构至少包含：

```text
RunnerFunctionRegistration {
  target_id
  app_id
  schema_id
  schema_version_range
  service_id
  offer_method
  activate_method
  registration_revision
}
```

- Registration 是版本化配置事实；具体在线 endpoint、AppInstance 和即时 capacity 来自服务发现
  与实例上报，不能反过来修改已经冻结的 Registration revision。
- Runner 不轮询 TaskMgr 或 Dispatcher，也不按 schema 扫描待领取 Task。Dispatcher 根据冻结的
  registration 主动调用 `offer_method`，绑定成功后再调用 `activate_method`。
- `user_creatable` 属于 Task Schema Catalog，决定 Task Center 是否提供通用创建 UI。
- 不在 Task Center 展示的内部 Runner，仍然必须注册后才能被自动 dispatch。
- 直接由 Service 创建并绑定给自身 AppInstance 的内部 Task 可以不经过 Dispatcher。

### 9.3 Input 和 Result 语义

Input 创建后不可变。需要修改 Input 时：

1. 取消或保留原 Task 的终态；
2. 使用新 idempotency key 创建新 Task；
3. 可用 `supersedes` 或 `retry_of` 关联原 Task。

Result 通过 `commit_result` 一次性写入。TaskMgr 在单个事务中：

1. 检查权限和 executor 身份；
2. 检查 runner epoch 和 expected revision；
3. 检查 `result IS NULL` 且 Task 非 Terminal；
4. 按 output schema 校验；
5. 写入 Result、Outcome、completed actor/time；
6. 增加 revision 并追加 durable event。

## 10. Runner、Assignees 与重新分配

### 10.1 App Runner

App Runner 写入执行状态时必须同时提供：

```text
task_id
app_instance_id
runner_epoch
expected_revision
```

首次分发由 Dispatcher 在收到幂等 `OfferAccepted` 后，验证
`(target_id, app_instance_id, lease_epoch, delivery_id)` 并绑定 Runner。TaskMgr 只接受
Dispatcher 的受信内部 bind command，不自行选择 Target 或实例。绑定后 Dispatcher 还必须主动
调用 `activate_task`，Runner 才能开始产生业务副作用。

当前 AppInstance 丢失后，冻结的逻辑 Target 或有 Reassign 权限的系统控制面可以在同一 Target
内部重新绑定实例：

1. 撤销旧 instance execution lease；
2. 原子增加 `runner_epoch`；
3. 保持 `target_id` 不变，清除旧 instance 时进入 `Waiting(Capacity/External)`；绑定同一 Target
   的新 AppInstance 时先进入 Accepted，由新实例确认 Running；
4. 追加 RunnerReleased/RunnerBound 事件；
5. 拒绝所有旧 epoch 的迟到写入。

Dispatcher 的初始 DispatchRecord 在首次 activate 确认后进入 Accepted 终态，不会因为后续实例
故障自动重新打开。activate 确认前的失败仍由该 DispatchRecord 的 DeliveryAttempt 恢复；确认后
同一 Target 的运行时恢复由 Target 自己的控制面负责。如需再次使用 Dispatcher，应创建一个只
针对既有 Task 和同一 Target 的内部 rebind generation，不能重新解析 OperationRoute。

TaskMgr 不承诺跨 AppInstance exactly-once，也不判断旧实例是否已经产生外部副作用。同一 Target
内继续 Task 必须由 Target 保证幂等、checkpoint 兼容或接受 at-least-once 风险。更换逻辑
Target 不属于 reassign：必须终止或保留原 Task，并以新 idempotency key 创建新 Task。

### 10.2 HumanSet

HumanSet 的 Assignees 是“任意一个人可以完成”的集合，不是会签列表：

- 每个有效 Assignee 都有 Commit 权限。
- 第一个通过 CAS 成功写入 Result 的 Assignee 获胜。
- 其它并发 Commit 返回 `task_already_completed`，并读取最终 Task。
- 实际提交用户和 App 写入 `completed_by` 与 Task Event。

不能处理的人可以在具有 Reassign 权限时原子修改 Assignees：

- 添加另一个人并保留自己，表达“扩大处理人集合”。
- 删除自己并添加另一个人，表达“转交”。
- Assignee 更新和 Result Commit 使用同一个 revision CAS；任一先成功，另一方必须重读。

如果需要多人会签、投票或多级审批，应由 Workflow 或一个专用 Runner 创建多个子 Task 并聚合，
不能复用 HumanSet 的“任意一人完成”语义。

## 11. TaskMgr 2.0 内部的 Task Dispatch Center

### 11.1 与 Scheduler、Task Core、Workflow 的边界

Dispatch Center 是 TaskMgr 2.0 内部一个带持久队列的机械投递模块。它可以与 Task Core 同进程
部署，但拥有独立的 RDB、RPC、授权策略和演进边界。Dispatcher 接收已经形成的投递意图，冻结
执行所需配置，按照确定性规则选择等价 TargetInstance，并主动调用 Runner 注册的服务接口。
它不决定业务为什么要执行，也不要求上游来自某一种组件。

上游有两条同等合法的入口：

```text
App / Workflow / Agent --已经知道投递意图或 Target--> Task Dispatch Center

App --需要系统级 placement/目标决策--> Scheduler
    --返回或直接提交 frozen DispatchPlan--> Task Dispatch Center
```

Scheduler 可以在一次决策中产生大量 DispatchPlan，并调用 Dispatcher；也可以把计划返回给应用
再由应用投递。无论走哪条路径，队列所有权、离线等待、主动调用和恢复都只属于 Dispatcher。
Dispatcher 不根据 caller 类型切换协议，同一个 frozen DispatchPlan 具有同样的投递语义。

不是所有 Task 都经过 Dispatcher。在线 RPC、Service 自己创建并执行的后台 Task、HumanSet Task
以及 Service 自身执行恢复可以直接使用 TaskMgr；需要离线持久交接、capacity 排队、主动推送和
delivery redelivery 的工作才进入 Dispatcher。Task Core 对 Dispatcher 保持零依赖；Dispatcher
通过 Task Core API 创建和绑定标准 Task。

Scheduler 的硬边界来自 `doc/arch/scheduler/scheduler.md`：Scheduler 是算法型轻状态组件，
每轮基于 frozen system-config snapshot 做纯内存推导，并把全部决策以单笔 `exec_tx` 原子写回。
Scheduler 不拥有队列、lease、redelivery、恢复扫描或独占持久状态；崩溃只停止产生新决策，
已经执行的动作和已排队的 Task 不受影响。

| 组件 | 负责 | 不负责 |
| --- | --- | --- |
| Scheduler | 系统级 placement、过滤、打分和目标状态决策 | 持久队列、offer、业务执行、Task 状态机 |
| Dispatcher | 执行 frozen DispatchPlan、持久队列、等价实例指派、主动 RPC、capacity、delivery lease、redelivery | 节点 placement、业务 fallback、Workflow 决策 |
| Task Core | Task ID、快照、权限、树、控制、Result 和事件 | 选择 Target/实例、维护分发队列 |
| Workflow | 业务图、依赖、条件、补偿和是否创建新 Task | 系统节点放置、Dispatcher lease |

这里存在三种不同层次的选择，不能混用。`DispatchPlan` 可以由 Scheduler 或明确知道目标的应用
产生；调用方省略 Target 时，OperationRoute 也只能按版本化配置机械补全并立即冻结：

```text
Scheduler/部署配置 --决定服务实例应该存在于哪些 Node-->
DispatchPlan/OperationRoute --把 schema 固化到逻辑 Target 和配置 revision-->
DeliveryPolicy --在该 Target 的等价 TargetInstance 中机械投递-->
```

```text
DispatchPlan {
  schema_id
  schema_version
  target_id
  target_config_revision
  runner_function
  delivery_policy_id
  delivery_policy_revision
}
```

RoundRobin/LeastLoaded 只是在等价实例中的交付策略，不是新的节点 placement 决策。Dispatcher
只消费实例上报的 available capacity，不建立 CPU、内存、GPU 等系统资源账本。硬配额或
admission 可以在创建前返回 `resource_exhausted`，但配额计算不进入 TaskMgr 或 Dispatcher。

Scheduler 的 `run_thunk` 若恢复使用，正确组合是 Scheduler 产生显式 DispatchPlan，Dispatcher
持久投递，Runner 执行；不能把 Dispatcher 的队列和恢复逻辑补进 Scheduler。

### 11.2 “确定性机械投递”的严格含义

Scheduler 是基于 frozen system-config snapshot 的纯决策器；Dispatcher 则是一个有持久状态的
确定性状态机。它的确定性定义为：

```text
next_state = reduce(
  DispatchRecord,
  persisted_queue_state,
  frozen DispatchPlan/DeliveryPolicy,
  ordered external event
)
```

在相同的 DispatchRecord、冻结配置 revision、持久队列状态和有序外部事件序列下，Dispatcher
必须得到相同的实例选择、状态迁移和下一次投递时间。这里的“结果”是投递决策和状态机结果，
不是 Runner 的业务 Result；网络可达性、实例上线和 RPC 返回属于显式外部事件，不能假装它们
永远相同。

为保证该语义：

- 队列使用稳定排序，例如
  `(priority DESC, ready_at ASC, created_at ASC, dispatch_id ASC)`；禁止依赖数据库未声明的返回顺序。
- RoundRobin 的 cursor 必须持久化；LeastLoaded 使用已记录的 capacity snapshot，并以
  `app_instance_id` 作为稳定 tie-breaker。
- 每次 RPC 调用前先持久化 DeliveryAttempt；成功、Busy、Rejected、transport error、timeout
  和 timer firing 都按顺序记录后再推进状态。
- DeliveryPolicy 在 DispatchRecord 创建时按 revision 冻结，至少定义 RPC timeout、backoff、
  max attempts 或 expires_at。需要抖动时只能使用由 `hash(dispatch_id, attempt_no)` 导出的
  deterministic jitter，不能直接使用进程随机数。
- 新任务、实例上线、capacity 释放、配置生效和 retry timer 只负责产生唤醒事件。服务发现和
  liveness cache 可以优化性能，但不是语义真相源，丢失后必须能从 durable data 和当前配置重建。

因此“同样配置下总是产生一样结果”不等于 Dispatcher 无状态，而是所有会影响决策的状态和
外部事实都被显式化、排序并可恢复。

### 11.3 Dispatch 流程：主动 Offer、绑定与激活

2.0 保留 Dispatcher 的独立 DispatchRecord 和 `Accepted` 交接终态，但把业务 Task 提前到
dispatch 被接受时创建：

请求在认证、operation/schema、显式 Target 或默认路由解析阶段被同步拒绝时不创建 Task；一旦
请求被 Dispatcher 接受为 PendingApproval、Queued 或 WaitingForTarget，就必须已经拥有稳定
task_id。

```text
Caller dispatch_task(operation/schema_id, input, target?, idempotency_key)
  -> Dispatcher 鉴权，接受调用方 DispatchPlan 或按配置解析并冻结 logical target/route revision
  -> Dispatcher 持久化不可变 DispatchRecord + auth envelope
  -> Dispatcher 幂等调用 Task Core 创建唯一公开 Task
       creator = 直接认证调用者
       phase = Promised
       executor = Unbound
  -> DispatchRecord 回填 task_id
  -> approval gate（如有）或进入 Queued/WaitingForTarget
  -> Dispatcher 按稳定队列顺序选择同一 Target 的具体 app_instance
  -> 先持久化 DeliveryAttempt(delivery_id, attempt_no, lease_epoch, endpoint, deadline)
  -> 主动调用 Runner offer_task(...)
       OfferAccepted(reservation_token) | Busy(retry_after?) | Rejected(stable_reason)
  -> OfferAccepted 只预留 capacity，不允许产生业务副作用
  -> TaskMgr 原子绑定同一个 Task
       executor=App(target_id, app_id, app_instance_id)
       runner_epoch++
       phase=Accepted
  -> Dispatcher 主动调用 Runner activate_task(task_id, delivery_id, runner_epoch, reservation_token)
  -> activate ACK 或观察到该 runner_epoch 的 report_started
  -> DispatchRecord=Accepted(task_id)，机械投递终态
  -> Runner report_started -> Running -> Terminal
```

DispatchRecord 的内部状态机固定为投递协议，而不是业务状态机：

```text
CreatingTask
  -> PendingApproval -> Queued
  -> Queued

Queued -> WaitingForTarget
Queued/WaitingForTarget -> Offering -> Binding -> Activating -> Accepted

PendingApproval/Queued/WaitingForTarget/Offering
  -> Rejected | Failed | Canceled | Expired

WaitingForTarget --实例或 capacity 可用--> Offering
Offering --Busy/可恢复失败--> Queued/WaitingForTarget
Binding --实例失效且旧 epoch 已 fencing--> Queued/WaitingForTarget
Activating --实例失效且旧 epoch 已 fencing--> Queued/WaitingForTarget/Binding
```

- `Queued/WaitingForTarget` 都属于持久队列；后者显式表示当前无在线实例或 capacity，原因保存在
  记录中，不建立另一套队列。
- `Offering` 表示 DeliveryAttempt 已持久化，offer RPC 正在进行或其结果仍不确定。
- `Binding` 表示 Runner 已预留 capacity，Dispatcher 正在幂等绑定 Task executor。
- `Activating` 表示 Task 已绑定，Dispatcher 正在确保主动启动通知到达。
- `Accepted` 是机械投递的吸收态；之后的 Task Running/Waiting/Terminal 不再回写 DispatchRecord。
- `Rejected/Failed/Canceled/Expired` 是未完成投递的吸收态，并按 §11.6 原子收敛公开 Task。

Runner 不提供 `claim_next`，也不轮询 Dispatcher/TaskMgr 找工作。`offer_task` 和 `activate_task`
都是 Dispatcher 根据 RunnerFunctionRegistration 主动调用的 kRPC：

```text
offer_task(
  delivery_id, task_id, schema_id, schema_version,
  input_or_ref, auth_envelope, lease_epoch, deadline
) -> OfferAccepted { app_instance_id, reservation_token }
   | Busy { retry_after? }
   | Rejected { stable_reason }

activate_task(
  delivery_id, task_id, runner_epoch, reservation_token
) -> Activated
```

Runner 必须按 `delivery_id` 幂等保存 offer 决定，并按
`task_id + runner_epoch + delivery_id` 幂等处理 activate。重复 RPC 返回原决定，不能再次预留
capacity 或启动第二次业务执行。Runner 只有在收到 activate 且确认 runner epoch 仍有效后才能
产生业务副作用；offer 阶段只做校验和短期预留。

Task ID 是调用者、UI、Workflow 和 Runner 使用的主要句柄。`dispatch_id` 仍可作为 Dispatcher
内部恢复、审计和管理 API 的 ID，但不再指向另一个 `target_task_id`。2.0 的 link 是一条
DispatchRecord 对一个预先存在的 Task；Task 只保存可选、不可变的反向
`origin_ref={kind:task-dispatcher,id:dispatch_id}`，Target 接收投递时不再创建业务 Task。

因此 2.0 也取消当前 Caller-owned mirror Task 与 Target-owned execution Task 的跨 owner 双对象
模式：Task Creator 保持为原调用者，Target bind 后只通过 Runner relation 获得
ReadInput/ReportProgress/Commit 等受限权限，不取得 Creator 身份或通用 Write 权限。

### 11.4 跨 Store 一致性：可恢复 Saga，不反向依赖

Task Core 和 Dispatcher store 独立，不做 SQL join，也不引入跨 RDB 分布式事务。Create 与 Enqueue
由 Dispatcher 自己的持久状态机完成：

1. 先按调用者 idempotency key 持久化 DispatchRecord，状态为内部 `CreatingTask`。
2. 用由 `dispatch_id` 确定生成的 Task Core idempotency key 调用受信 `create_promised_task`。
3. Task Core 重放始终返回同一个 Task；Dispatcher 把 task_id 回填到 DispatchRecord。
4. 根据审批策略进入 PendingApproval 或 Queued，并开始实例评估。
5. 只有 DispatchRecord 和 Task 都持久化后才向调用者返回成功，返回值以 task_id 为主。

崩溃发生在任意一步时，Dispatcher `startup_recovery` 从自己的非终态记录继续上述流程。若 Task
已经创建但回填 ACK 丢失，重放 Task Core idempotency key 找回同一个 Task。Task Core 不保存
dispatch outbox，也不扫描 Dispatcher store；Dispatcher 完全不启动时，直接创建和执行的普通
TaskMgr Task 不受影响。

每个 DeliveryAttempt 同样使用“先落库、后调用”协议。RPC response、timeout 或进程崩溃后，
Dispatcher 根据持久 attempt、lease epoch 和 frozen DeliveryPolicy 重放同一调用或生成下一次
attempt。offer timeout 具有结果不确定性时，应先用同一 `delivery_id` 重放以取得 Runner 保存的
原决定；只有旧 lease 被明确 fencing 后，才能增加 attempt/lease epoch 并选择另一等价实例。
activate timeout 时重放同一个 activate；Task 已进入 Running 且 runner epoch 匹配也可以作为
激活成功的权威证据。

DispatchRecord 可以继续保存用于鉴权和投递的不可变 input/auth envelope 副本，但创建 Task
时必须校验其 digest 与 Task `input_digest` 一致；此后两边都不可修改。Task Input 是 UI、Result
contract 和 Task 历史的规范载荷，DispatchRecord 副本只服务于交接和审计，不能演化成第二份
可变业务状态。

KEvent 只能加速 Dispatcher 自身的重评估；权威状态来自 Dispatcher RDB、冻结配置和 Task Core
snapshot。Runner 的正常接单协议始终是 Dispatcher 主动 RPC，而不是收到事件后自行扫描队列。

对仍在队列中的 Promised Task，通用 UI 调用 TaskMgr `request_control(Cancel)`。TaskMgr 只持久化
请求，不反向调用 Dispatcher；Dispatcher 通过 KEvent 加速加低频 sweep 读取该 Task，原子撤销
自己的非终态 DispatchRecord 和 delivery lease 后再调用 `cancel_promised_task` 确认终态。
`cancel_dispatch` 若保留，也必须收敛到同一协议，不能直接留下一个仍可被投递的 Canceled Task。

### 11.5 Target stickiness、实例切换与两个 epoch

Dispatcher 在创建 DispatchRecord 时根据显式 target 或管理员 OperationRoute 固化 `target_id`
和 `route_revision`。默认路由更新、delivery redelivery、进程重启和 Task pause/resume 都不能改变
已有记录的 logical Target。

OperationRoute 是管理员/系统控制面的配置，不由 TargetRegistration 自己声明；Target 不能通过
注册某个 schema/operation 把自己变成默认后端。显式 target 未注册、未启用或不支持对应 schema
时同步拒绝，省略 target 且没有有效默认路由时也同步拒绝，不创建 Task。

- Task executor 绑定前可以在同一 Target 的等价 TargetInstance 间重新 offer。
- `lease_epoch` 由 Dispatcher 管理，防止旧实例响应或激活新一代 DeliveryAttempt。
- bind 时 Task Core 验证 Dispatcher 授权并增加 `runner_epoch`。
- activate 确认后 DispatchRecord 进入 Accepted 终态，不再跟踪业务 phase/progress/result。
- 同一 Target 的控制面可以因实例故障重新绑定 AppInstance，但必须增加 runner epoch。
- 更换 logical Target 是新的业务执行：新 idempotency key、新 DispatchRecord、新 Task；不在原
  Task 上热迁移。

因为 Target 不再创建本地业务 Task，当前 Dispatcher 用来处理“目标 Task 已创建但 ACK 丢失”的
`Uncertain` 分支可以在 2.0 收敛为可重放 DeliveryAttempt：读取 DispatchRecord、DeliveryAttempt
和 Task executor 即可确定阶段，不会产生第二个 target Task。

TargetInstance 在 `activate_task` 到达并确认自己仍是当前 executor/runner epoch 之前，禁止启动
任何业务副作用。旧 activate 即使迟到，也必须被 runner epoch fencing；不能盲目执行，也不能
创建本地替代 Task。该约束使 delivery lease 过期后可以在同一 Target 内安全 redelivery，但仍不
承诺跨实例 exactly-once 消除所有外部副作用。

首次 activate 完成之前 selected instance 丢失时，Dispatcher 可以释放旧绑定、增加
lease/runner epoch，并在同一 Target 内继续投递。DispatchRecord 已进入 Accepted 后，初始投递
职责结束；之后的进程恢复、checkpoint 和同 Target 实例切换属于 Target 自己的运行控制面，不能
自动重新打开原 DispatchRecord。

### 11.6 离线、背压和失败的公开投影

Dispatcher 内部状态比 Task 公共状态更细，但只能按稳定规则投影，不能把传输细节泄露成新的
公开 Task 类型：

| Dispatcher 事实 | Dispatcher 动作 | Task 投影 |
| --- | --- | --- |
| 没有在线实例/连接失败 | 保留队列，等待服务发现事件或 retry timer | `Promised + Dispatch(code=target_offline)` |
| Runner 返回 Busy | 按 frozen policy 计算 ready_at 并重新排队 | `Promised + Capacity(code=runner_busy)` |
| offer transport timeout | 持久化 timeout，优先重放同一 delivery_id | `Promised + Dispatch(code=offer_retry)` |
| Runner 稳定 Rejected 且 policy 无其它合法实例 | 结束 DispatchRecord | `Terminal/Failed`，保存 stable dispatch error |
| offer 已接受，Task 已绑定，等待 activate | 幂等重放 activate | `Accepted`，message 可显示 Activating |
| activate 已确认 | DispatchRecord 进入 Accepted 终态 | Runner 随后报告 `Running` |
| delivery deadline/attempt budget 耗尽 | DispatchRecord 进入 Expired/Failed | `Terminal/Failed` |
| Caller 请求取消且 Task 尚未绑定 | 原子撤销队列和 lease | `Terminal/Canceled` |

Task 已绑定但仍处于 Activating 时，取消已经是普通 App Task 的 Control Request：Dispatcher 继续
幂等完成 activate，Runner 在产生副作用前读取并处理 pending Cancel。不能把已绑定 Task 当作
Promised/Unbound 直接删除绑定或绕过 runner epoch。

Scheduler 瞬时投递大量 Task 时，Dispatcher 只按上述队列顺序和 capacity 做背压；Scheduler 不
等待单个 Runner 上线，也不保存补偿队列。离线 endpoint、liveness 和 route lookup cache 都只是
可丢弃优化，不能改变 durable queue 的最终处理顺序和状态机结果。

### 11.7 分发审批门与执行期人工任务

Dispatcher 的人工放行和执行中的 HumanSet Task 是两个不同语义：

1. **分发审批门**回答“是否允许把请求交给高权限 Target”。命中
   `DispatchApprovalPolicy` 时，DispatchRecord 停在 PendingApproval，不评估、不 offer、不占
   capacity，Dispatcher 不调用 Target。对应公开 Task 保持
   `Promised + wait_reason=Authorization`。
2. `approve_dispatch` 只允许记录进入正常队列；它不修改不可变 auth envelope、不提升调用者
   权限，也不豁免 Target 在 `offer_task` 时重新做业务鉴权。`deny/expire/reject` 使公开 Task 进入
   `Terminal/Failed` 并记录稳定 dispatch error；Caller cancel 则进入 `Terminal/Canceled`。
3. **执行期人工任务**发生在 Runner 已 activate 之后，用 HumanSet 子 Task 表达，例如命令执行
   中等待用户 Approve/Reject。它不复用 Dispatcher PendingApproval，也不由 Dispatcher 决定。

Dispatch auth envelope 保存直接调用者、校验后的 on_behalf_of、input digest、审批上下文和创建
时间。Task 的 Creator 永远是直接认证调用者，不能被 on_behalf_of 替换；业务用户的读取、控制
或计量关系由创建时 ACL grant 和 DispatchRecord 审计表达。

## 12. 通用状态迁移和写入权限

TaskMgr command 层至少支持以下通用迁移：

| 操作 | 前置状态 | 结果 | 写入方 |
| --- | --- | --- | --- |
| Create dispatched task | 无 | `Promised/Unbound` | Dispatcher 入口代表 Creator |
| Create direct App task | 无 | `Accepted/App(instance=self)` | 已认证 App |
| Create human task | 无 | `Waiting/HumanSet` | Creator/parent Runner |
| Bind runner | `Promised/Unbound` | `Accepted/App(target, instance)`，epoch++ | Dispatcher |
| Start | `Accepted` | `Running` | 当前 Runner |
| Enter waiting | `Accepted/Running` | `Waiting(reason)` | 当前 Runner |
| Leave waiting | `Waiting` | `Running` | 当前 Runner |
| Request pause | 非 Terminal | 记录 PauseRequest | 具有 Control 权限者 |
| Ack pause | 有 PauseRequest | `Paused`，清除 request | 当前 Runner |
| Request resume | `Paused` | 记录 ResumeRequest | 具有 Control 权限者 |
| Ack resume | 有 ResumeRequest | `Running`，清除 request | 当前 Runner |
| Request cancel | 非 Terminal App Task | 记录 CancelRequest | 具有 Control 权限者 |
| Ack cancel | 有 CancelRequest | `Terminal/Canceled` | 当前 Runner |
| Cancel unbound/human | `Promised/Unbound` 或 `HumanSet` | CAS 后 `Terminal/Canceled` | Dispatcher/Task Core，见 §6.2 |
| Commit result | 非 Terminal、result 为空 | `Terminal/Succeeded` | 当前 Runner 或 Assignee |
| Fail | 非 Terminal | `Terminal/Failed` | 当前 Runner |
| Release instance | 非 Terminal/App | `Waiting`，保留 target、清除 instance，epoch++ | 当前 Target/系统 Reassign 权限者 |
| Rebind instance | App 且 instance 为空 | 绑定同 Target 新 instance，`Accepted`，epoch++ | 当前 Target/系统 Reassign 权限者 |
| Reassign humans | 非 Terminal/HumanSet | 修改 Assignees，revision++ | Reassign 权限者 |
| Archive | Terminal | 设置 archived_at | Archive 权限者 |

TaskMgr 可以拒绝显然违反协议不变量的迁移，例如从 Terminal resume、没有 PauseRequest 却 ack
pause、旧 Runner Commit。至于底层工作是否真的可以暂停、何时进入 waiting，由 Runner 决定。

## 13. kRPC 协议草案

TaskMgr 仍通过 `/kapi/task-manager` 暴露 kRPC。2.0 删除通用 status/data update，按职责划分为
以下命令。

### 13.1 创建与查询

| Method | 关键参数 | 返回 | 说明 |
| --- | --- | --- | --- |
| `create_task` | schema、input、executor、parent、policy、idempotency_key | Task | 创建直接绑定或 HumanSet Task |
| `get_task` | task_id | Task | 按 ACL 返回允许读取的字段 |
| `list_tasks` | creator/schema/phase/root/executor/time/cursor | TaskSummary[] | 分页查询 |
| `get_task_tree` | root_id、depth、cursor | TaskSummary[] | 查询树结构和状态摘要 |
| `get_subtasks` | task_id、cursor | TaskSummary[] | 查询直接子任务 |
| `archive_task` | task_id、expected_revision | Task | 从默认列表隐藏，不物理删除 |

`dispatch_task` 属于 `/kapi/task-dispatcher`，但返回同一个标准 Task。

普通 `create_task` 只能把 App executor 直接绑定为经过认证的调用实例自身；绑定其它
AppInstance 必须经过 Dispatcher 或显式系统级 Reassign 权限，防止 Task 创建接口变成任意
Service 的工作投递入口。幂等摘要必须覆盖 schema/version、Input、parent、executor、policy 和
适用时的 dispatch envelope digest；相同 key 携带不同不可变信封时返回
`idempotency_conflict`。

### 13.2 控制与权限

| Method | 关键参数 | 返回 |
| --- | --- | --- |
| `request_control` | task_id、action、recursive、expected_revision、request_id | Task/BatchControlResult |
| `update_assignees` | task_id、add/remove、expected_revision | Task |
| `grant_task_access` | task_id、grant、expected_revision | Task |
| `revoke_task_access` | task_id、grant_id、expected_revision | Task |

### 13.3 Runner/Human 写入

| Method | 关键参数 | 返回 |
| --- | --- | --- |
| `report_started` | task_id、runner_epoch、expected_revision | Task |
| `report_progress` | task_id、progress、message、runner_epoch、expected_revision | Task |
| `report_waiting` | task_id、reason、runner_epoch、expected_revision | Task |
| `report_running` | task_id、runner_epoch、expected_revision | Task |
| `update_control_profile` | task_id、profile、runner_epoch、expected_revision | Task |
| `ack_control` | task_id、request_id、applied/rejected、runner_epoch、expected_revision | Task |
| `commit_result` | task_id、result、runner_epoch?、expected_revision | Task |
| `fail_task` | task_id、error、runner_epoch、expected_revision | Task |

Human Commit 不提供 runner epoch，由服务端从 token 验证 `user_id ∈ active assignees`。App Runner
写操作必须同时验证 token/AppInstance、runner epoch 和 revision。

### 13.4 受信 Promise/Executor 控制面命令

Task Core 提供通用的受信控制面 capability，不把方法绑定到 Dispatcher 类型。Task Dispatch
Center 是当前主要调用者；未来其它内核组件若获显式授权，也复用相同协议。普通用户、Runner
和未授权 Service 不能直接调用，TaskMgr 只信任已认证 SystemRole/capability，不信任 payload
自报身份。

| Method | 关键参数 | 作用 |
| --- | --- | --- |
| `create_promised_task` | schema/input、delegated creator envelope、policy、origin_ref、幂等键 | 以原直接调用者为 Creator 创建 Promised Task |
| `set_promise_wait` | task_id、通用 wait reason、expected_revision | 更新未绑定 Task 的展示投影 |
| `bind_app_executor` | task_id、target/app/instance、delivery_id、expected_revision | 原子绑定 App executor、epoch++、进入 Accepted，并返回 runner_epoch |
| `release_app_executor` | task_id、expected instance/runner_epoch、reason、expected_revision | fencing 旧实例，保留 Target、清除 instance、epoch++ |
| `finish_promise_failure` | task_id、stable error、expected_revision | Promise 无法兑现时进入 Failed |
| `cancel_promised_task` | task_id、expected_revision | 来源系统撤回持久承诺后进入 Canceled |

Dispatcher 在自己的 store 中验证 DeliveryAttempt、instance lease epoch、Target owner 和业务
鉴权后调用 `bind_app_executor`。Task Core 不读取 Dispatcher RDB，也不重新判断 route；它只验证
当前 Task 仍是可绑定状态、调用者具有对应 bind capability，并执行本地 CAS。

### 13.5 Dispatcher 与 Runner 的主动投递协议

`/kapi/task-dispatcher` 至少提供以下面向上游的操作；它不因调用方是 Scheduler、Workflow 或普通
App 而切换语义：

| Method | 关键参数 | 返回 | 说明 |
| --- | --- | --- | --- |
| `dispatch_task` | schema/input、target 或 DispatchPlan、policy、auth envelope、idempotency_key | task_id、dispatch_id、Task | 接受并持久化投递意图 |
| `get_dispatch` | dispatch_id 或 task_id | DispatchSummary | 查看队列/投递状态，不替代 Task 查询 |
| `approve_dispatch` | dispatch_id、decision、expected_revision | DispatchSummary | 只处理分发前审批门 |
| `cancel_dispatch` | dispatch_id、expected_revision | DispatchSummary | 收敛到 Task 通用取消协议 |

RunnerFunctionRegistration 指向的 Target service 必须实现以下由 Dispatcher 主动调用的接口：

| Method | 幂等键/Fencing | 约束 |
| --- | --- | --- |
| `offer_task` | delivery_id、lease_epoch | 只校验、鉴权和预留 capacity；重复调用返回原 OfferAccepted/Busy/Rejected 决定 |
| `activate_task` | delivery_id、task_id、runner_epoch | 只有确认当前 runner epoch 后才启动；重复调用不得重复执行 |

两个接口都必须验证已认证的 Dispatcher service identity/capability，并核对冻结的 target、schema、
input digest 和 auth envelope；不能信任 payload 自报来源。transport timeout 不是新的业务请求，
Dispatcher 必须按同一 delivery_id 重放。协议版本不兼容、schema 不支持和原 auth envelope 被拒绝
是稳定 Rejected；临时离线、Busy 和 timeout 进入 DeliveryPolicy 控制的队列重试。

### 13.6 通用错误

| Error | 语义 |
| --- | --- |
| `task_not_found` | Task 不存在或调用者不可见 |
| `permission_denied` | 缺少所需 action |
| `revision_conflict` | expected revision 已过期，调用者必须重读 |
| `stale_runner_epoch` | 当前 AppInstance 已失去执行权 |
| `invalid_task_phase` | 违反 Task 通用协议不变量 |
| `control_not_available` | 当前 control profile 不支持该动作 |
| `control_already_pending` | 已有未完成控制请求 |
| `task_already_completed` | Result/终态已被另一个提交者写入 |
| `input_schema_mismatch` | 创建 Input 不符合 schema |
| `result_schema_mismatch` | Commit Result 不符合 schema |
| `idempotency_conflict` | 相同 key 对应的不可变请求摘要不同 |

## 14. 事件、订阅与 UI

### 14.1 Durable Event 与 KEvent

Task 快照是当前真相源，`task_event` 是不可变审计和恢复历史。任何 Task 状态、executor、ACL、
Assignees、Control 或 Result 变化必须在同一 RDB 事务中：

1. CAS 更新 Task snapshot；
2. 增加 revision；
3. 追加 Task Event；
4. 事务提交后发布 KEvent。

KEvent 是加速通知，不是真相源。订阅方收到事件、超时或重连后都重新读取 Task；事件丢失不能
导致状态永远不再处理。

订阅路径保持简单：

```text
/task_mgr/{task_id}
/task_mgr/tree/{root_id}
```

事件至少包含：

```text
event_id, task_id, root_id, revision, event_type,
actor, phase, outcome, created_at
```

默认事件不内联 Input/Result；客户端按权限回读，避免 KEvent 泄露敏感 payload。

### 14.2 主要 EventType

```text
TaskCreated
RunnerBound
RunnerReleased
PhaseChanged
WaitReasonChanged
ProgressReported
ControlProfileChanged
ControlRequested
ControlSuperseded
ControlApplied
ControlRejected
AssigneesChanged
AccessGranted
AccessRevoked
ResultCommitted
TaskFailed
TaskCanceled
TaskArchived
PayloadRedacted
```

### 14.3 UI 要求

Task Center 应：

- 所有详情页使用 `/tasks/{task_id}`。
- 根据组合状态生成 Pausing/Resuming/Canceling 等展示状态。
- 只根据当前 control profile 和调用者 ACL 展示可用操作。
- 对 Waiting 展示通用 wait reason 和 schema-specific message。
- 用 Task Schema presentation schema 生成 Input/Result UI；未知 schema 退化为受权限控制的 JSON。
- 树视图默认只加载 metadata，展开详情时再按 ACL 获取 Input/Result。
- 明确显示取消保证是 interrupt 还是 safe。

## 15. 持久数据设计

### 15.1 Overview

Service：TaskMgr 2.0 subsystem。Task Core 使用平台 RDB 持久化 Task snapshot、执行绑定、
Assignees、ACL、Task Schema、事件和 Notes。结构化数据不得绑定具体 SQLite/PostgreSQL 特性。
内部 Dispatcher 使用独立 RDB 保存 DispatchRecord、稳定队列顺序、DeliveryAttempt、delivery
lease、重试时间和策略 revision，并通过可恢复 Saga 幂等调用 Task Core/Runner；Task Core store
不保存 Dispatcher outbox 或队列状态。

### 15.2 Data Classification

| 数据 | 分类 | 原因 |
| --- | --- | --- |
| Task snapshot、Input、Result | Durable | 用户可见真相源，必须跨重启和升级保留 |
| Task Assignees | Durable | 决定 Human Commit 权限 |
| Task ACL Grants | Durable | 决定访问和控制权限 |
| Task Events | Durable | 审计、恢复和状态变更历史 |
| Task Schema Definitions | Durable | Input/Result 的长期解释契约 |
| Task Notes | Durable | 用户/Agent 的旁路参考信息 |
| KEvent notification | Disposable | 只用于加速，丢失后可回读 Task |
| Tree/ACL query cache | Disposable | 可由 durable data 重建 |
| Runner endpoint、heartbeat、即时容量 cache | Disposable，Dispatcher 所有 | 由服务发现和 Runner 重新 attach/renew |
| RunnerFunctionRegistration/OperationRoute revision | Durable，Dispatcher/系统配置所有 | 决定 frozen DispatchPlan 的长期解释 |
| Dispatcher queue/DeliveryAttempt/delivery lease/retry cursor | Durable，Dispatcher 所有 | 决定投递顺序、幂等重放和崩溃恢复，不属于 Task Core schema |

### 15.3 Storage Strategy

- 所有结构化 durable data 使用平台提供的 `task-mgr-main` RDB instance。
- 不使用文件路径作为 Task、Result 或事件的核心存储模型。
- 过大的 Input/Result 可以由后续版本使用 object ID 间接引用；Task 中仍保存 digest 和引用。
- KEvent、endpoint cache 和 Runner liveness 不进入 Task Core durable schema，也不能成为 Dispatcher
  队列的唯一真相源。
- Dispatcher 继续使用独立 RDB instance，两个 store 不做 SQL join。

### 15.4 Schema Definitions

#### Table: `task`

| Column | Type | Nullable | Default | Description |
| --- | --- | --- | --- | --- |
| `task_id` | TEXT PK | NO | | URL-safe opaque Task ID |
| `schema_id` | TEXT | NO | | 版本化 Task 类型 |
| `schema_version` | BIGINT | NO | | 冻结的 schema revision |
| `name` | TEXT | NO | | UI 展示名称 |
| `input_json` | TEXT/JSON | NO | | 不可变 Input |
| `input_digest` | TEXT | NO | | Input 和不可变信封摘要 |
| `result_json` | TEXT/JSON | YES | | 一次性 Result |
| `error_json` | TEXT/JSON | YES | | 失败信息 |
| `creator_user_id` | TEXT | NO | | 来自认证上下文 |
| `creator_app_id` | TEXT | NO | | 来自认证上下文 |
| `creator_instance_id` | TEXT | YES | | 创建实例审计 |
| `idempotency_key` | TEXT | NO | | Creator 范围幂等键 |
| `origin_kind` | TEXT | YES | | 不透明来源类型，如 task-dispatcher |
| `origin_id` | TEXT | YES | | 来源系统内部 ID；TaskMgr 不解释 |
| `parent_id` | TEXT FK | YES | | 不可变直接父 Task；不级联删除 |
| `root_id` | TEXT | NO | | 不可变根 Task ID |
| `child_control_policy_json` | TEXT/JSON | NO | | 控制传播策略 |
| `retry_of` | TEXT | YES | | 业务重试来源 |
| `supersedes` | TEXT | YES | | 被当前 Task 替代的 Task |
| `executor_kind` | TEXT | NO | `Unbound` | Unbound/App/HumanSet |
| `runner_target_id` | TEXT | YES | | Dispatcher 固化的 logical Target；首次 bind 后不可变 |
| `runner_instance_id` | TEXT | YES | | 当前实际 App Runner instance，可在同 Target 内变化 |
| `runner_app_id` | TEXT | YES | | Runner App 快照 |
| `runner_epoch` | BIGINT | NO | `0` | Runner fencing token |
| `phase` | TEXT | NO | | TaskPhase |
| `wait_reason_json` | TEXT/JSON | YES | | 当前等待原因 |
| `control_request_json` | TEXT/JSON | YES | | 当前未完成控制请求 |
| `control_profile_json` | TEXT/JSON | NO | | Runner 当前声明 |
| `progress_json` | TEXT/JSON | YES | | 可变进度快照 |
| `message` | TEXT | YES | | 面向 UI 的当前说明 |
| `outcome` | TEXT | YES | | Terminal outcome |
| `completed_by_user_id` | TEXT | YES | | 最终提交用户 |
| `completed_by_app_id` | TEXT | YES | | 最终提交 App |
| `policy_preset` | TEXT | NO | `collaborative-tree/v1` | 默认 policy |
| `permission_boundary` | BOOLEAN | NO | `false` | 是否阻断祖先 grant |
| `revision` | BIGINT | NO | `1` | 每次成功写入递增 |
| `created_at` | BIGINT | NO | | Unix timestamp ms |
| `updated_at` | BIGINT | NO | | Unix timestamp ms |
| `completed_at` | BIGINT | YES | | 终态时间 |
| `archived_at` | BIGINT | YES | | 从默认 UI 列表隐藏时间 |

Indexes：

- `uq_task_creator_idempotency(creator_user_id, creator_app_id, idempotency_key)` UNIQUE。
- `uq_task_origin(origin_kind, origin_id)` UNIQUE；仅对非 NULL origin 生效。
- `idx_task_root_created(root_id, created_at, task_id)`：树查询。
- `idx_task_parent_created(parent_id, created_at, task_id)`：直接子任务查询。
- `idx_task_phase_updated(phase, updated_at, task_id)`：状态扫描和恢复。
- `idx_task_creator_created(creator_user_id, creator_app_id, created_at, task_id)`：Creator 列表。
- `idx_task_schema_created(schema_id, schema_version, created_at, task_id)`：类型查询。
- `idx_task_runner_phase(runner_target_id, runner_instance_id, phase)`：Target 恢复和审计。

Constraints：

- parent 不使用 `ON DELETE CASCADE`；2.0 不提供普通 hard delete。
- Input、Result、Terminal、epoch/revision 等不变量由 Task Core transaction command 层保证。
- `root_id` 必须等于根 Task 自身 ID，或等于 parent 的 root_id。
- `origin_kind/origin_id` 必须同时为空或同时非空；TaskMgr 只做唯一性和不可变性检查。
- Dispatcher Task 的 `runner_target_id` 一旦首次绑定不可修改；`runner_instance_id` 每次变化都必须
  同事务增加 runner_epoch 并写 Task Event。

#### Table: `task_assignee`

| Column | Type | Nullable | Description |
| --- | --- | --- | --- |
| `task_id` | TEXT FK | NO | HumanSet Task |
| `user_id` | TEXT | NO | 可提交结果的用户 |
| `granted_by_user_id` | TEXT | NO | 授权者 |
| `granted_by_app_id` | TEXT | NO | 授权 App |
| `created_at` | BIGINT | NO | 生效时间 |
| `revoked_at` | BIGINT | YES | 撤销时间；保留审计 |

Indexes：

- Primary key：`(task_id, user_id)`；重新授权复用该行并清空 revoked_at，历史由 Task Event 保留。
- `idx_task_assignee_user_active(user_id, revoked_at, task_id)`：人的待办列表。

#### Table: `task_acl_grant`

| Column | Type | Nullable | Description |
| --- | --- | --- | --- |
| `grant_id` | TEXT PK | NO | Grant ID |
| `task_id` | TEXT FK | NO | Grant 起点 |
| `subject_kind` | TEXT | NO | relation/user/app/principal/system_role |
| `subject_relation` | TEXT | YES | RootCreator/Creator/Runner/Assignees |
| `subject_user_id` | TEXT | YES | User/Principal 参数 |
| `subject_app_id` | TEXT | YES | App/Principal 参数 |
| `subject_system_role` | TEXT | YES | SystemRole 参数 |
| `actions_json` | TEXT/JSON | NO | 允许的 TaskAction 集合 |
| `scope` | TEXT | NO | SelfOnly/Subtree/WholeTree |
| `data_scope` | TEXT | NO | MetaOnly/Payload/Full |
| `created_by_user_id` | TEXT | NO | 授权者 |
| `created_by_app_id` | TEXT | NO | 授权 App |
| `created_at` | BIGINT | NO | 生效时间 |
| `revoked_at` | BIGINT | YES | 撤销时间 |

Indexes：

- `idx_task_acl_task_active(task_id, revoked_at)`：按 Task 计算权限。
- `idx_task_acl_user_active(subject_user_id, revoked_at, task_id)`：按用户过滤候选 Task。
- `idx_task_acl_app_active(subject_app_id, revoked_at, task_id)`：按 App 过滤候选 Task。
- `idx_task_acl_role_active(subject_system_role, revoked_at, task_id)`：按系统角色过滤候选 Task。

#### Table: `task_event`

| Column | Type | Nullable | Description |
| --- | --- | --- | --- |
| `event_id` | TEXT PK | NO | 时间有序 opaque event ID |
| `task_id` | TEXT FK | NO | Task |
| `root_id` | TEXT | NO | Tree fanout 查询 |
| `task_revision` | BIGINT | NO | 变更后的 Task revision |
| `event_type` | TEXT | NO | 稳定 EventType |
| `actor_user_id` | TEXT | YES | 操作者用户 |
| `actor_app_id` | TEXT | YES | 操作者 App |
| `actor_instance_id` | TEXT | YES | 操作者实例 |
| `payload_json` | TEXT/JSON | NO | 小型结构化事件数据 |
| `created_at` | BIGINT | NO | 事件时间 |

Indexes：

- `uq_task_event_revision(task_id, task_revision)` UNIQUE：每次 Task revision 只写一个主事件；同一次
  变更的其它事实放进该事件 payload。
- `idx_task_event_task(task_id, event_id)`：单 Task 事件游标。
- `idx_task_event_root(root_id, event_id)`：整棵树事件游标。

#### Table: `task_schema`

| Column | Type | Nullable | Description |
| --- | --- | --- | --- |
| `schema_id` | TEXT | NO | 含主版本的稳定 ID |
| `schema_version` | BIGINT | NO | 不可变修订号 |
| `input_schema_json` | TEXT/JSON | NO | Input JSON Schema |
| `output_schema_json` | TEXT/JSON | NO | Result JSON Schema |
| `presentation_schema_json` | TEXT/JSON | YES | 通用 UI 描述 |
| `executor_kinds_json` | TEXT/JSON | NO | 允许的 executor kinds |
| `user_creatable` | BOOLEAN | NO | Task Center 是否展示 |
| `publisher_app_id` | TEXT | NO | 发布并维护该 Schema 的 App |
| `enabled` | BOOLEAN | NO | 是否允许新建 |
| `created_at` | BIGINT | NO | 创建时间 |

Primary key：`(schema_id, schema_version)`。已发布行的 schema、publisher 和 executor kinds 不可原地
修改；`enabled` 是独立的可变 catalog 开关，变化需要系统权限和配置审计，禁用只影响新 Task。

#### Table: `task_note`

沿用现有 Task Note 的旁路语义。Note 不修改 Task revision、phase、Input 或 Result；它有独立作者、
时间和 ACL 检查。Task 不级联删除 Note，因为普通 hard delete 已取消。

### 15.5 Schema Version

TaskMgr 2.0 durable schema 使用 `schema_version = 7`，延续当前代码中的 TaskMgr schema 版本序列。
版本由平台 RDB schema/meta 机制保存。Dispatcher 的独立 store 在实施本设计时升级到自己的下一
版本，不与 Task Core 共用 schema version。

未来任何字段语义、索引或表结构变化都增加 schema version。Task Schema 自身的
`schema_id/schema_version` 与数据库 schema version 是两个不同概念。

### 15.6 Upgrade Compatibility Strategy

beta 2.2 当前处于 breaking-change 开发阶段，TaskMgr 1.x 到 2.0 使用 No-compat 策略：

- 开发和 DV 环境允许清空 TaskMgr/Dispatcher RDB 后重建 schema 7。
- 不保留旧 `TaskStatus`、可变 `Task.data`、read/write scope、hard delete 或
  `dispatch_id -> target_task_id` 兼容协议。
- 实施 PR 必须同步修改所有生产者、消费者、前端、文档和共享类型，不能只修改数据库。

2.0 正式发布后改用显式 migration：启动时按版本顺序迁移；迁移失败必须停止服务并保留原数据，
不得静默重建。

### 15.7 Extensibility Rules

Frozen semantics：

- Task ID、Creator、Input、schema binding、parent/root、Result 一次性提交和 Terminal 吸收态。
- HumanSet 任意一人成功 Commit 的语义。
- runner epoch fencing 和 revision CAS。
- Dispatcher Task 首次 bind 后的 logical Target stickiness。
- `Commit`、`Control`、`Reassign` 权限的边界。

Extensible fields：

- WaitReason kind、EventType、TaskError detail、Progress JSON、ControlProfile 可增加可选字段。
- Task Schema 可以增加新版本，不修改已发布版本。
- ACL 可以增加新的 subject/action，但旧 action 语义不得改变。

核心 `task` 表不提供可随意修改的通用 `extra` 以绕过不可变性。业务扩展必须进入版本化 Input、
Result、Progress 或 Event payload。

### 15.8 Query Patterns

| 查询 | 支持索引/策略 |
| --- | --- |
| 按 task_id 打开稳定 URL | `task` PK |
| 按不透明 origin_ref 反查 Task | `uq_task_origin` |
| 按 root_id 展示整棵树 | `idx_task_root_created` |
| 查询直接子任务 | `idx_task_parent_created` |
| 用户查看自己创建的 Task | `idx_task_creator_created` |
| 人查看可处理 Task | `idx_task_assignee_user_active` join task phase |
| Runner 恢复当前 Task | `idx_task_runner_phase` |
| 按 schema/phase/time 列表 | `idx_task_schema_created`、`idx_task_phase_updated` |
| 计算 Task ACL | ancestor/root 查询 + `idx_task_acl_task_active`；结果可缓存 |
| 订阅单 Task 事件 | `idx_task_event_task` |
| 订阅整棵树事件 | `idx_task_event_root` |

所有 list API 必须分页。树级批量控制允许遍历 subtree，但必须设置最大节点数和 continuation，
禁止无界单事务更新超大 Task Tree。

Dispatcher 的 CreatingTask、PendingApproval、Queued、DeliveryAttempt、delivery lease 和 retry
查询使用 `task-dispatcher-main` 自己的 schema/index，不进入 TaskMgr Query Patterns。其实现至少
需要等价于以下稳定索引：队列
`(status, ready_at, priority, created_at, dispatch_id)`、attempt
`(dispatch_id, attempt_no)`、registration `(target_id, schema_id, registration_revision)`；任何查询
都必须显式排序，不能依赖底层数据库的偶然顺序。

### 15.9 Retention、Archive 与 Redaction

- 普通 API 不提供 hard delete。
- Archive 只影响默认列表，不改变 Task ID、Result 和事件。
- 系统 retention policy 可以在到期后清除敏感 Input/Result 正文，但必须保留 task_id、digest、
  schema、Creator、Outcome、时间和 `PayloadRedacted` 事件。
- Redaction 是受系统权限控制的保留策略操作，不是修改 Input/Result 的业务接口。
- Zone 重置、开发环境 schema 重建等管理操作不属于普通 Task 生命周期。

## 16. 典型场景

### 16.1 系统服务的长任务

```text
Service 收到请求
  -> create_task(executor=self instance)
  -> Accepted
  -> Runner report_started -> Running
  -> report_progress
  -> commit_result
  -> Terminal/Succeeded
```

短调用可以同步返回结果；一旦接口返回 Pending，则同时返回稳定 task_id，后续通过相同 Task 查询
和订阅。

### 16.2 Dispatcher 因容量不足排队

```text
Agent 直接 dispatch command
  -> DispatchRecord 冻结 Target T
  -> Task A = Promised/Unbound
  -> wait_reason = Capacity
  -> UI URL /tasks/A 立即可用
  -> T 的实例 I 有容量
  -> Dispatcher 持久化 DeliveryAttempt，并主动 offer_task(A) 给 I
  -> I 返回 OfferAccepted，只预留 capacity
  -> Dispatcher bind A -> App(target=T, instance=I) -> Accepted
  -> Dispatcher activate_task(A, runner_epoch)
  -> I 幂等确认 Activated
  -> Runner -> Running -> Terminal
```

整个过程只有 Task A。DeliveryAttempt/lease 是 Dispatcher 内部记录，I 不轮询队列；Scheduler
只负责让实例 I 按目标状态存在于合适 Node，不持有 A 的队列或执行状态。

### 16.3 两种上游入口与 Scheduler 批量投递

```text
应用已经知道 Target T
  -> 直接 dispatch_task(frozen plan T)
  -> Dispatcher queue

应用只知道系统级目标
  -> Scheduler 基于同一 snapshot 产生 N 个 frozen DispatchPlan
  -> Scheduler 直接调用 Dispatcher，或把 plans 返回给应用后调用
  -> Dispatcher 按 durable stable order 接受 N 个 Task
  -> Target 离线的 Task 保持 Promised/target_offline
  -> 实例上线/capacity 释放事件逐批唤醒主动 offer
```

如果两条路径最终提交的 DispatchPlan、DeliveryPolicy、队列状态和外部事件序列相同，Dispatcher
选择和状态迁移必须相同。Scheduler 的批量请求不会把 Runner 队列上移到 Scheduler；缓存失效或
Dispatcher 重启也不能改变已经持久化的 Task 顺序。

### 16.4 Dispatcher 分发前审批

```text
bob dispatch apps.install/v1
  -> DispatchRecord = PendingApproval
  -> Task P = Promised/Authorization
  -> Dispatcher 不调用 installer Target
管理员 approve_dispatch
  -> auth envelope 和 Task Creator 仍是 bob
  -> DispatchRecord -> Queued -> Offering -> Activating -> Accepted(P)
  -> installer 仍按原 envelope 做业务鉴权
```

如果管理员 deny，DispatchRecord 进入 Rejected，Task P 进入 `Terminal/Failed` 并记录
`dispatch_approval_denied`。这不是执行期 HumanSet Task，也不赋予 bob 更高权限。

### 16.5 人工任务转交

```text
Task H = Waiting/HumanSet([Alice])
Alice 判断无法处理
  -> update_assignees(remove Alice, add Bob, expected_revision)
Bob commit_result(text)
  -> CAS 成功
  -> H = Terminal/Succeeded
```

如果业务希望 Alice 仍可兜底，则只添加 Bob。Alice/Bob 并发提交时只有一个 Commit 成功。

### 16.6 Agent 命令等待用户批准

```text
Command Task C，Runner=Agent Cmd Runner
  -> C = Waiting(Authorization, related_task_id=A)
  -> 创建子 Task A
       schema=human.approval/v1
       executor=HumanSet([user])
       output=Approve | Reject
  -> 用户 Commit A
  -> Cmd Runner 观察 A.result
       Approve: C -> Running
       Reject:  C 提交约定的拒绝结果或取消
```

TaskMgr 不依赖 Workflow 就能表达该模式。Workflow 也可以复用完全相同的 Task Tree。

### 16.7 失败后重新执行

```text
Download Task D1 -> Terminal/Failed
调用方决定重试
  -> 新 idempotency key
  -> 创建 D2，retry_of=D1，Input 可以相同
```

D1 保持终态和原始结果，不被重新打开。Runner 内部进程重连并继续 D2 的同一次 execution lease
恢复不算新的业务重试。

## 17. 资源计量边界

`task_id` 是资源和计费记录的 correlation/ref：

```text
usage_event.task_id
usage_event.root_task_id
```

真实资源使用由资源系统或计费账本以 append-only event 记录。TaskMgr 可以展示聚合结果，但
Task 行不是账本真相源，也不负责配额扣减。父 Task 聚合子 Task 消耗时不得重复记录同一份实际
资源使用。

## 18. 非目标

1. 不实现 Workflow DSL、表达式树、条件分支、循环、补偿或 schedule。
2. 不自动业务重试，不把 Terminal Task 重新打开。
3. 不承诺跨 Runner exactly-once 或自动消除外部副作用。
4. 不承担节点 placement、资源配额、计费和通用资源管理。
5. 不根据业务失败自动选择另一个异构 Runner。
6. 不用 Notes、Progress 或 Event 绕过 Input/Result schema 和不可变性。
7. 不把 KEvent 当持久队列或唯一真相源。
8. 不提供普通 hard delete 或父任务级联删除。
9. 不引入独立 Promise、Assignment、Run、HumanTask 等公开运行时对象。
10. 不为 TaskMgr 1.x 和当前 Dispatcher 双 Task 交接保留兼容层。
11. 不把 Dispatcher 的持久队列、lease 或 startup recovery 下沉到 Scheduler。
12. 不在已有 Task 上跨 logical Target 热迁移；换 Target 必须创建新 Task。
13. 不提供 Runner 拉取队列、按 schema 扫描或 `claim_next` 接单协议。

## 19. 2.0 实施影响

2.0 实施至少需要联动：

- `buckyos-api/src/task_mgr.rs`：替换单一 TaskStatus、TaskPermissions 和通用 update API。
- `task_manager/src/task_db.rs`：schema 7、CAS、不可变 Input/Result、事件和 ACL。
- `task_manager/src/server.rs`：命令式状态写入、控制请求、树策略和权限计算。
- `buckyos-api/src/task_dispatcher.rs` 与 `task_manager/src/dispatcher/`：从
  `dispatch_id -> target_task_id` 改为先持久 DispatchRecord、再幂等创建并绑定已有 task_id；
  增加 CreatingTask、稳定队列顺序、frozen DeliveryPolicy、DeliveryAttempt、retry timer 和
  delivery lease 恢复；保留 PendingApproval、Target stickiness 和 Accepted 投递终态，删除 Runner
  `claim_next` 路径，收敛 Uncertain/IdempotentAccept 的“重复创建目标 Task”语义。
- Runner service 与注册协议：发布版本化 RunnerFunctionRegistration，实现 Dispatcher 主动调用的
  幂等 `offer_task`/`activate_task`，在 activate 和 runner epoch 确认前禁止业务副作用。
- `buckyos-api/src/taskdata.rs`：现有 TaskData 类型迁移为版本化 Task Schema。
- Task Center、Workflow、OpenDAN、Scheduler、Control Panel：同步共享类型、API、状态投影和
  深链接。
- Scheduler 不新增 store、队列或恢复协议；如接入异步 thunk 或批量动作，仅输出 frozen
  DispatchPlan 并调用 Dispatcher。
- 本文是 TaskMgr 2.0 的总规范；`doc/task_mgr/task_mgr.md`、`task_dispatch_center.md`、
  `task data schema.md` 仅作为 1.x/beta 2.2 实现输入，实施后标记为历史版本或拆成不重复定义
  2.0 语义的实现说明。

实施验收必须覆盖：

1. Result/Terminal/Runner epoch/Revision 的并发竞态测试。
2. Dispatcher `CreatingTask -> PendingApproval/Queued` Saga 在每个崩溃点都能恢复，相同
   idempotency key 和派生 TaskMgr key 都返回原 Task。
3. 相同 DispatchRecord、配置 revision、持久队列和外部事件序列，在重启前后产生相同的实例
   选择、attempt 顺序和 retry_at；RoundRobin cursor 和 tie-breaker 可重复。
4. Runner 不轮询或 claim；Dispatcher 主动 `offer -> bind -> activate`，offer 阶段不能产生业务
   副作用，重复 delivery_id/activate 不重复预留或执行。
5. offline、Busy、offer timeout、activate timeout 和进程崩溃都先持久化再确定性重放；Scheduler
   批量投递不会丢 Task 或改变稳定队列顺序。
6. Dispatcher delivery lease epoch 与 Task runner epoch 分别 fencing；旧实例不能响应新 attempt、
   接受旧 activate 或写 Task。
7. Scheduler 入口和应用直连入口提交相同 frozen DispatchPlan 时，产生相同投递语义；Dispatcher
   不依赖 Task 来源。
8. HumanSet 多人同时 Commit 只有一个成功。
9. 树级控制只产生 request，并遵守 child control policy。
10. ACL 的 Self/Subtree/WholeTree、permission boundary 和字段级读取。
11. KEvent 和 endpoint cache 丢失时通过 durable snapshot/event 正常恢复，Runner 仍无需轮询。
12. Input/Result schema 校验和不可变性。
13. OperationRoute 更新不改变已有 DispatchRecord/Task 的 Target，同 Target 可以换实例，跨 Target
   必须新建 Task。
14. PendingApproval 不产生 offer；approve 不改变 auth envelope，deny/expire 正确终结公开 Task。
15. Scheduler 崩溃只停止新 placement 决策，不影响 Dispatcher 队列和已经运行的 Task。
