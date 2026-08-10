# TaskMgr 2.0 设计

- 状态：设计基线
- 版本：2.0
- 兼容性：beta 2.2 breaking change，不提供旧 TaskMgr/Dispatcher 协议兼容层
- 目标读者：TaskMgr、Task Dispatch Center、Scheduler、Workflow、OpenDAN、Task Center UI 开发者
- 相关文档：
  - `doc/task_mgr/task_mgr.md`：当前 TaskMgr 1.x 实现说明
  - `doc/task_mgr/task_dispatch_center.md`：当前 Dispatcher 实现说明；2.0 实施时需要按本文调整
  - `doc/task_mgr/task data schema.md`：当前 TaskData 类型资产；2.0 实施时迁移为 Task Schema

## 1. 设计目标

TaskMgr 2.0 为整个 BuckyOS 提供一个统一、稳定、可寻址的 `Task` 抽象。UI、Agent、系统服务、
Scheduler 和 Workflow 都可以使用同一个 `task_id` 查看一项工作的输入、当前状态、控制能力和
最终结果，并生成稳定链接：

```text
/tasks/{task_id}
```

一个 Task 可以从“已经承诺处理，但尚未找到执行者”一直存在到“已经由机器或人完成”。系统
不会因为分发、换 Runner、等待授权或暂停而切换到另一个公开对象，也不会在 Dispatcher accept
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

## 2. 核心设计原则

### 2.1 只有一种公开运行时对象

系统对外只有 `Task`。`Promised Task`、`Running Task`、人工任务和 App 执行任务都是同一个
类型在不同组合状态下的表现。

以下记录不是新的公开 Task 类型：

- Task Schema 是输入、输出和 UI 的版本化契约。
- ACL Grant 是 Task 的权限配置。
- Task Event 是 Task 的内部持久变更历史。
- Dispatch Record、offer 和 lease 是 Dispatcher 的内部队列记录。
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
- Runner 进程崩溃后的 offer redelivery、lease 恢复不是业务重试，可以继续使用同一 Task。

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
```

`creator.user_id/app_id` 必须来自通过验证的调用上下文，不能相信 payload。`app_instance_id`
主要用于执行和审计；它的字符串格式可以包含 app_id，但权限判断必须以注册信息和验签上下文
为准，不能仅靠解析字符串获得身份。

### 4.2 Executor

```rust
pub enum TaskExecutor {
    Unbound,
    AppInstance {
        app_instance_id: String,
        app_id: String,
    },
    HumanSet,
}
```

- `Unbound`：合法的 Promised Task，尚未找到或绑定执行者。
- `AppInstance`：当前由一个具体 App 实例负责执行。
- `HumanSet`：由 `task_assignee` 中任意一个当前有效用户提交 Result，首个成功 Commit 获胜。

一个 Task 同一时刻只有一个有效的执行模式。UI App 只是 Human Commit 的提交渠道，不因此
成为该人工 Task 的 Runner。`HumanSet` 必须至少包含一个有效 Assignee；如果移除最后一人，
必须在同一事务把 Task 改为 `Unbound/Promised` 并交回 Dispatcher，而不能留下不可完成的
HumanSet。

### 4.3 Creator、Control 权限和 Runner

三者语义不同：

- Creator 是不可变审计事实。
- Control 是 ACL action，不设置独立 controller 字段。
- Runner 是当前 AppInstance executor；HumanSet 则由 Assignees 共同构成可提交集合。

默认 policy 把整棵树的 Control 权限授予 root creator。后续委托、系统管理员介入或 Workflow
自定义权限通过 ACL Grant 表达，不修改 creator，也不需要引入 controller 对象。

2.0 不使用含义不稳定的 `owner` 字段：

- `creator.app_id` 是发起创建的 App。
- `executor.AppInstance.app_id` 是当前执行 App。
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
- `Reassign` 允许调整 HumanSet 或请求 Dispatcher 重新绑定 AppInstance。
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

当前 `TaskDataType`、`TypedTaskData` 和 `request/progress/result` 结构是迁移 Task Schema 的主要
资产。2.0 不再允许通过 merge patch 修改同一个 `data.result`。

### 9.2 Runner Registration 与 UI Catalog 分离

- Runner Registration 声明某个 Target/App 支持哪些 `schema_id/version range`，供 Dispatcher
  进行分发。
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

Dispatcher 或有 Reassign 权限的控制面重新绑定 Runner 时：

1. 撤销旧 instance lease；
2. 原子增加 `runner_epoch`；
3. 清除旧 executor 或绑定新 AppInstance；
4. 追加 RunnerReleased/RunnerBound 事件；
5. 拒绝所有旧 epoch 的迟到写入。

TaskMgr 不承诺跨 Runner exactly-once，也不判断旧 Runner 是否已经产生外部副作用。需要换 Runner
继续同一 Task 的 operation 必须由上层保证幂等、checkpoint 兼容或接受 at-least-once 风险。

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

## 11. Task Dispatch Center 2.0 边界

### 11.1 定位

Dispatcher 是 TaskMgr 同层的机械分发器，负责维护尚未绑定 Runner 的 Task 队列。它管理：

- 持久等待队列和公平性。
- Target/Runner 注册和支持的 Task Schema。
- 在线 AppInstance、capacity、offer、lease 和 redelivery。
- 根据上层给出的 target/constraints 进行机械匹配。
- 绑定已有 Task 的 executor 和 runner epoch。

Dispatcher 不负责：

- Workflow 分支、依赖、补偿和业务重试。
- 节点放置算法和系统资源配额。
- 因业务失败自动选择另一个异构 Runner。
- 创建第二个 target Task。

Scheduler 或 Workflow 可以给出 placement/target 约束；Dispatcher 只维护队列并在条件满足时执行
分发。资源暂时不足不是 Task 创建失败，Task 保持 `Promised`，wait reason 显示 `Capacity`。
硬配额或 admission 拒绝可以在创建前返回 `resource_exhausted`，但配额计算不进入 TaskMgr。

### 11.2 Dispatch 流程

```text
Caller dispatch_task(schema, input, constraints, idempotency_key)
  -> 创建唯一公开 Task，phase=Promised、executor=Unbound
  -> 原子写入 dispatch outbox
  -> Dispatcher 将 task_id 加入内部持久队列
  -> 没有容量时保持 Promised/Capacity
  -> 选择具体 app_instance 并产生 offer lease
  -> instance accept(task_id, offer_id, lease_epoch)
  -> TaskMgr 在 CAS 中绑定同一个 Task
       executor=AppInstance
       runner_epoch++
       phase=Accepted
  -> Runner 开始后报告 Running
```

Task ID 同时是调用者幂等重试、UI 链接和 Dispatcher 业务关联锚点。Dispatcher 可以保留内部
`dispatch_record/offer_id`，但 2.0 删除当前 `dispatch_id -> target_task_id` 的双业务对象交接。

### 11.3 Create 与 Enqueue 的原子性

TaskMgr store 与 Dispatcher store 独立时，`dispatch_task` 必须使用 transactional outbox 保证：

- Task 与 dispatch intent 在 TaskMgr 同一事务写入。
- Dispatcher 按 task_id 幂等消费 outbox 并 upsert 队列记录。
- 调用者重放相同 idempotency key 会返回同一 Task，并修复未完成 enqueue。
- TaskMgr/Dispatcher 启动恢复会重新扫描未确认 outbox。
- 对外只在 Task 和 dispatch intent 都已持久化后返回成功。

KEvent 只能加速消费，不能作为唯一 enqueue 通道。

### 11.4 主动指派与 Claim/Lease

Dispatcher 主动选择具体 Target/AppInstance；Worker 的 claim/lease 是接收该指派的可靠性协议，
不是面向业务的抢任务模型。它用于背压、断线恢复、旧实例 fencing 和通知丢失后的 sweep。

### 11.5 授权等待

执行前需要用户授权时，负责该策略的上层可以：

1. 让父 Task 保持 `Promised` 或 `Waiting(Authorization)`；
2. 创建一个 `HumanSet` 子 Task，其 output schema 为 `Approve | Reject`；
3. 用户 Commit 子 Task；
4. 策略执行者根据 Result 决定把父 Task 入队或终止。

Dispatcher 只执行“批准后入队”的机械结果，不内置审批工作流。

## 12. 通用状态迁移和写入权限

TaskMgr command 层至少支持以下通用迁移：

| 操作 | 前置状态 | 结果 | 写入方 |
| --- | --- | --- | --- |
| Create dispatched task | 无 | `Promised/Unbound` | Dispatcher 入口代表 Creator |
| Create direct App task | 无 | `Accepted/AppInstance(self)` | 已认证 App |
| Create human task | 无 | `Waiting/HumanSet` | Creator/parent Runner |
| Bind runner | `Promised/Unbound` | `Accepted/AppInstance`，epoch++ | Dispatcher |
| Start | `Accepted` | `Running` | 当前 Runner |
| Enter waiting | `Accepted/Running` | `Waiting(reason)` | 当前 Runner |
| Leave waiting | `Waiting` | `Running` | 当前 Runner |
| Request pause | 非 Terminal | 记录 PauseRequest | 具有 Control 权限者 |
| Ack pause | 有 PauseRequest | `Paused`，清除 request | 当前 Runner |
| Request resume | `Paused` | 记录 ResumeRequest | 具有 Control 权限者 |
| Ack resume | 有 ResumeRequest | `Running`，清除 request | 当前 Runner |
| Request cancel | 非 Terminal AppInstance Task | 记录 CancelRequest | 具有 Control 权限者 |
| Ack cancel | 有 CancelRequest | `Terminal/Canceled` | 当前 Runner |
| Cancel unbound/human | `Promised/Unbound` 或 `HumanSet` | CAS 后 `Terminal/Canceled` | Dispatcher/TaskMgr，见 §6.2 |
| Commit result | 非 Terminal、result 为空 | `Terminal/Succeeded` | 当前 Runner 或 Assignee |
| Fail | 非 Terminal | `Terminal/Failed` | 当前 Runner |
| Release runner | 非 Terminal/AppInstance | `Promised/Unbound`，epoch++ | Dispatcher/Reassign 权限者 |
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

普通 `create_task` 只能把 AppInstance executor 直接绑定为经过认证的调用实例自身；绑定其它
AppInstance 必须经过 Dispatcher 或显式系统级 Reassign 权限，防止 Task 创建接口变成任意
Service 的工作投递入口。幂等摘要必须覆盖 schema/version、Input、parent、executor、policy 和
dispatch spec；相同 key 携带不同不可变信封时返回 `idempotency_conflict`。

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

### 13.4 通用错误

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

Service：TaskMgr 2.0。TaskMgr 使用平台 RDB 持久化 Task snapshot、执行绑定、Assignees、ACL、
Task Schema、事件和 Notes。结构化数据不得绑定具体 SQLite/PostgreSQL 特性。Dispatcher 使用
独立 RDB 保存队列和 lease；TaskMgr 通过 transactional outbox 与其可靠衔接。

### 15.2 Data Classification

| 数据 | 分类 | 原因 |
| --- | --- | --- |
| Task snapshot、Input、Result | Durable | 用户可见真相源，必须跨重启和升级保留 |
| Task Assignees | Durable | 决定 Human Commit 权限 |
| Task ACL Grants | Durable | 决定访问和控制权限 |
| Task Events | Durable | 审计、恢复和状态变更历史 |
| Task Schema Definitions | Durable | Input/Result 的长期解释契约 |
| Task Notes | Durable | 用户/Agent 的旁路参考信息 |
| Dispatch Outbox | Durable until acknowledged | 保证 Task 创建和入队之间不丢失 |
| KEvent notification | Disposable | 只用于加速，丢失后可回读 Task |
| Tree/ACL query cache | Disposable | 可由 durable data 重建 |
| Runner heartbeat、即时容量 | Disposable，Dispatcher 所有 | 由 Runner 重新 attach/renew |
| Dispatcher queue/offer lease | Durable，Dispatcher 所有 | 需要崩溃恢复，但不属于 TaskMgr schema |

### 15.3 Storage Strategy

- 所有结构化 durable data 使用平台提供的 `task-mgr-main` RDB instance。
- 不使用文件路径作为 Task、Result 或事件的核心存储模型。
- 过大的 Input/Result 可以由后续版本使用 object ID 间接引用；Task 中仍保存 digest 和引用。
- KEvent、缓存和 Runner liveness 不进入 TaskMgr durable schema。
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
| `parent_id` | TEXT FK | YES | | 不可变直接父 Task；不级联删除 |
| `root_id` | TEXT | NO | | 不可变根 Task ID |
| `child_control_policy_json` | TEXT/JSON | NO | | 控制传播策略 |
| `retry_of` | TEXT | YES | | 业务重试来源 |
| `supersedes` | TEXT | YES | | 被当前 Task 替代的 Task |
| `executor_kind` | TEXT | NO | `Unbound` | Unbound/AppInstance/HumanSet |
| `runner_instance_id` | TEXT | YES | | 当前 App Runner instance |
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
- `idx_task_root_created(root_id, created_at, task_id)`：树查询。
- `idx_task_parent_created(parent_id, created_at, task_id)`：直接子任务查询。
- `idx_task_phase_updated(phase, updated_at, task_id)`：状态扫描和恢复。
- `idx_task_creator_created(creator_user_id, creator_app_id, created_at, task_id)`：Creator 列表。
- `idx_task_schema_created(schema_id, schema_version, created_at, task_id)`：类型查询。
- `idx_task_runner_phase(runner_instance_id, runner_epoch, phase)`：Runner 恢复和审计。

Constraints：

- parent 不使用 `ON DELETE CASCADE`；2.0 不提供普通 hard delete。
- Input、Result、Terminal、epoch/revision 等不变量由 TaskMgr transaction command 层保证。
- `root_id` 必须等于根 Task 自身 ID，或等于 parent 的 root_id。

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

#### Table: `task_dispatch_outbox`

| Column | Type | Nullable | Description |
| --- | --- | --- | --- |
| `outbox_id` | TEXT PK | NO | Outbox message ID |
| `task_id` | TEXT UNIQUE FK | NO | 要入队的 Promised Task |
| `dispatch_spec_json` | TEXT/JSON | NO | target/constraints/route 的冻结信封 |
| `status` | TEXT | NO | Pending/Delivered |
| `delivery_count` | BIGINT | NO | 投递次数 |
| `last_error` | TEXT | YES | 最近失败 |
| `created_at` | BIGINT | NO | 创建时间 |
| `updated_at` | BIGINT | NO | 更新时间 |

Index：`idx_dispatch_outbox_status_created(status, created_at)`，支持启动恢复和后台投递。

### 15.5 Schema Version

TaskMgr 2.0 durable schema 使用 `schema_version = 7`，延续当前代码中的 TaskMgr schema 版本序列。
版本由平台 RDB schema/meta 机制保存。Dispatcher 的独立 store 在实施本设计时升级到自己的下一
版本，不与 TaskMgr 共用 schema version。

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
| 按 root_id 展示整棵树 | `idx_task_root_created` |
| 查询直接子任务 | `idx_task_parent_created` |
| 用户查看自己创建的 Task | `idx_task_creator_created` |
| 人查看可处理 Task | `idx_task_assignee_user_active` join task phase |
| Runner 恢复当前 Task | `idx_task_runner_phase` |
| 按 schema/phase/time 列表 | `idx_task_schema_created`、`idx_task_phase_updated` |
| 计算 Task ACL | ancestor/root 查询 + `idx_task_acl_task_active`；结果可缓存 |
| 订阅单 Task 事件 | `idx_task_event_task` |
| 订阅整棵树事件 | `idx_task_event_root` |
| Dispatcher 恢复未投递 intent | `idx_dispatch_outbox_status_created` |

所有 list API 必须分页。树级批量控制允许遍历 subtree，但必须设置最大节点数和 continuation，
禁止无界单事务更新超大 Task Tree。

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
Agent dispatch command
  -> Task A = Promised/Unbound
  -> wait_reason = Capacity
  -> UI URL /tasks/A 立即可用
  -> Runner 有容量
  -> Dispatcher bind A -> Accepted
  -> Runner -> Running -> Terminal
```

整个过程只有 Task A。offer/lease 是 Dispatcher 内部记录。

### 16.3 人工任务转交

```text
Task H = Waiting/HumanSet([Alice])
Alice 判断无法处理
  -> update_assignees(remove Alice, add Bob, expected_revision)
Bob commit_result(text)
  -> CAS 成功
  -> H = Terminal/Succeeded
```

如果业务希望 Alice 仍可兜底，则只添加 Bob。Alice/Bob 并发提交时只有一个 Commit 成功。

### 16.4 Agent 命令等待用户批准

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

### 16.5 失败后重新执行

```text
Download Task D1 -> Terminal/Failed
调用方决定重试
  -> 新 idempotency key
  -> 创建 D2，retry_of=D1，Input 可以相同
```

D1 保持终态和原始结果，不被重新打开。Runner 内部进程重连并继续 D2 的同一次 lease 恢复不算
新的业务重试。

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

## 19. 2.0 实施影响

2.0 实施至少需要联动：

- `buckyos-api/src/task_mgr.rs`：替换单一 TaskStatus、TaskPermissions 和通用 update API。
- `task_manager/src/task_db.rs`：schema 7、CAS、不可变 Input/Result、事件和 ACL。
- `task_manager/src/server.rs`：命令式状态写入、控制请求、树策略和权限计算。
- `buckyos-api/src/task_dispatcher.rs` 与 `task_manager/src/dispatcher/`：从
  `dispatch_id -> target_task_id` 改为绑定已有 task_id。
- `buckyos-api/src/taskdata.rs`：现有 TaskData 类型迁移为版本化 Task Schema。
- Task Center、Workflow、OpenDAN、Scheduler、Control Panel：同步共享类型、API、状态投影和
  深链接。
- `doc/task_mgr/task_mgr.md`、`task_dispatch_center.md`、`task data schema.md`：实施后更新为
  2.0 事实文档或标记为历史版本。

实施验收必须覆盖：

1. Result/Terminal/Runner epoch/Revision 的并发竞态测试。
2. Dispatcher create-enqueue 崩溃恢复和相同 idempotency key 重放。
3. Runner lease 过期后旧实例不能写入。
4. HumanSet 多人同时 Commit 只有一个成功。
5. 树级控制只产生 request，并遵守 child control policy。
6. ACL 的 Self/Subtree/WholeTree、permission boundary 和字段级读取。
7. KEvent 丢失时通过 durable snapshot/event 正常恢复。
8. Input/Result schema 校验和不可变性。
