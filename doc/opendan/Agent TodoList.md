# Agent Work Session Todo-List 需求

> 来源：根据语音记录整理。本文档用于明确 Agent Todo CLI / 工具接口的核心概念、接口语义、状态流转和约束。

## 1. 背景与目标

Agent 在执行复杂任务时，需要把当前 work session 中的工作拆成一组较小的 Todo，并把每个 Todo 交给更干净的上下文去执行。Todo 工具的目标不是创建一个全局任务系统，而是在当前 session 内维护一个有序、可追踪、可回溯的工作列表。

### 1.1 设计哲学

Todo **不是一个通用任务系统**，它本质上是一种 **LLM Behavior 引导机制 —— 可以理解为 CoT 的升级版**。

在我们的 LLM Behavior 状态机里：

- 旧的 PDCA 状态机是一个**图**。
- 现在的 Behavior 状态机更接近一棵**树**。

#### 为什么从图（Graph PDCA）转到树（Tree Plan-Do）

全图 PDCA 模式下我们观察到的核心问题是：**Do 不仅要干活，还要操心"下一步该把自己切到 Plan 还是 Check"**。Do 本来是整条工作流里最重要、最实际的执行层，结果它的 prompt 里被迫塞进了对整体工作流的判断 —— 这既污染了 Do 的上下文，又让 Do 的实现变重。

转到树模式之后，Do 的责任被收窄到**只负责"结束"**：

1. Do 是核心干活的环节，是真正的执行层。
2. Do 领到任务时，**任务和必要的 skills 都已经由 Plan 在一个纯不干活的环境里准备好**，直接交付。
3. Do 在一个**干净的、只属于自己的上下文**里专注做完这一件事。
4. 做完以后，Do **不需要决定下一步状态机走到哪里**，只需要告诉调用方"我这件事做完了"。
5. 上层是什么状态机、下一个分支是 Plan / Check / 还是再 fork 一个 Do，**Do 完全不关心**。

这就是为什么 Todo 在结构上是 Behavior 树的一个分支节点：

- `addTodo` 显式地写入下一个执行分支，作为上层 Behavior 切到 Execute / DO 状态的信号；
- DO 模块的实现非常简单 —— 它只需要调用 `currentTodo` 拿到那一个 Todo 并执行，不需要在多个候选里挑；
- 执行 Todo 相当于一次 **MContext 的 Fork**，创建出一个推理分支；
- 分支执行完毕后**自动回到主干**；主干视角下这等同于一次"函数调用"，直接看到上一次执行的结果（status + summary + report）。

正是因为采用了 **fork-join 的执行模型**，Todo 的状态管理才能保持轻量 —— 主干不需要维护一个复杂的状态机，只需要显式 push 下一步要做的事情；Do 也不需要维护它，只需要在分支结束时"返回"。

我们也刻意保持工具面**尽量小**：能交给分支自身推理历史承担的，就不挂在 Todo 工具上。

### 1.2 核心目标

1. 在 `plan` 模式下创建、查看、管理当前 session 的 Todo 列表。
2. 在执行模式下，Agent 只看到并执行当前 `current todo`。
3. Todo 执行完成后，主干上下文只接收该 Todo 的状态、简短摘要和最终报告。
4. 如需深入了解某个 Todo 的内部执行过程，**直接进入对应分支的推理历史 / 工作目录查看**，而不是查 Todo 工具的中间日志。
5. 与系统级 `Task` 区分清楚：Task 的**状态**和后续跟踪由标准 `task_management` 工具负责；Todo 工具只通过 `delegateTask` 发起外部任务，并在 Workspace 维度保存一个指针用于列表展示。Todo 是 session 级、临时、只在当前 work session 内有效的工作项。

---

## 2. 核心概念

### 2.1 Workspace 与 Session 的生命周期

理解 Task / Todo 持久化层级的前提，是先理清两层容器：

- **Workspace**：长期存在的工作空间（典型例子：一个代码目录）。生命周期远长于 Session，一个 Workspace 会被多个 Session 反复进入。
- **Session**：可以结束、可以被删除的会话。

由此引出一条核心原则：

> 凡是会对**外部产生影响**的事情，即使发起它的 Session 已经结束，下次另一个 Session 关联到同一个 Workspace 时，**也必须仍能看到它**。

这是 Task 和 Todo 持久化粒度不同的根本原因 —— delegate 出去的 Task 已经对外部造成影响，必须在 Workspace 维度可见；而 Todo 只是对本次 LLM 推理的引导，没有外部副作用，可以随 Session 一起消失。

### 2.2 Task：系统级任务

`Task` 是系统级概念。

- **状态管理在系统侧**：一旦创建，Task 可通过 `taskID` 在整个分布式系统中查询状态。
- **历史 / 记录归档在 Workspace 维度**：
  - 一旦 delegate 出去（例如发请求让另一位 Agent / 人帮忙 review），就已经产生了外部影响 —— 不能因为发起它的 Session 结束就让这条记录消失；
  - 后续任何关联到同一个 Workspace 的 Session 都应该能看到这个 Task 的存在与最新状态。
- 通过 `delegateTask` delegate 一个 Task 时，**必须显式记录**：
  - 由哪一个 Session 发起；
  - 基于什么目的（purpose / context）。
- Task 是长期存在的，不依赖单个 Session 的生命周期。
- Task 可能被 delegate 到外部执行者或外部系统。
- 如果 Agent 需要等待某个 delegated task 的结果，Agent 应使用标准 `task_management` 工具查询、等待、更新或安排后续检查；Todo 工具不重复实现 Task 跟踪流程。

### 2.3 Todo：Session 级工作项

`Todo` 是当前 work session 内部的工作项，本质上是 **LLM Behavior 树上的一个分支节点**。

- Todo 与当前 session 绑定。
- session 被删除后，Todo 列表及其 Todo 项也随之失效（因为它从未对外部产生影响）。
- Todo 的有效范围只在当前 session 内。
- Todo 用来承载 Agent 在当前 session 中拆出来的下一步执行单元。
- Todo 没有外部执行者选择 —— 默认由当前 Agent 在执行模式下完成，对应推理分支的一次 fork-join。

### 2.4 Task 与 Todo 的区别

| 维度 | Task | Todo |
| --- | --- | --- |
| 生命周期 | 长期存在 | 随 session 存在 |
| 状态归属 | 系统级 / 分布式系统级 | 当前 work session |
| 历史归档 | Workspace 维度（跨 session 可见）| 随 session 消失 |
| ID | 较长的 `taskID` | 较短的 `todoID` |
| 可查询性 | 可全局查询状态 | 仅当前 session 内有效 |
| 执行者 | 可 delegate 给外部执行者或系统 | 默认由当前 Agent 执行 |
| 删除 session 后 | 仍可见（已对外部产生影响）| 不再存在 |
| 是否可混入列表展示 | 可在 TodoList 中展示 delegated task 状态 | TodoList 的主要对象 |

---

## 3. 模式与执行语义

> 整体心智模型见 §1.1：Todo 是 Behavior 树的分支节点；`addTodo` 切到 DO，DO 只 `currentTodo` 不做选择，执行=MContext fork，结束=自动 join 回主干。本节给出具体的模式语义。

### 3.1 Plan 模式

`plan` 模式负责规划和管理 Todo 列表。

在 plan 模式下：

- 可以多次调用 `addTodo`，一次性创建一组 Todo。
- Todo 不建立复杂依赖关系。
- Todo 按创建顺序依次执行。
- 可以调用 `listTodo` 查看当前 session 的 Todo 情况。
- 每次重新进入 plan 模式时，系统应自动展示当前 Todo 列表状态，因此 `listTodo` 命令虽然支持，但通常不是必须手动调用。
- plan 模式下可看到 delegated Task 混入 TodoList 的摘要展示。
- 未完成的 delegated Task 应在列表中置顶或突出展示；需要精确状态时由 Agent 调用 `task_management` 查询。

### 3.2 执行模式 / Worker 模式

执行模式只负责完成当前 `current todo`。这是一次 MContext 分支的生命周期。

在执行模式下：

- Agent 只看到当前 Todo，而不是完整 Todo 列表 —— **DO 模块的唯一动作就是 `currentTodo`**，不需要从候选里挑。
- 当前 Todo 的提示词由 `current todo` 的模板渲染而来。
- Agent 应能看到该 Todo 被创建时所在回合的必要上下文，也就是”为什么创建这个 Todo”。
- Agent 基于当前 Todo 的任务描述、skills 和创建上下文执行工作；checklist、标签等细节应尽量写在任务描述里，不再拆成一组必填/选填参数。
- Todo 完成后，Agent 调用 `finishTodo` 设置最终状态，并给出摘要和最终报告 —— 此时分支 join 回主干。
- 主干上下文默认只看到 Todo 完成后的 status / summary / report，**完整内部执行轨迹保留在该分支的推理历史 / 工作目录中**，需要时再追溯。

### 3.3 Current Todo

`current todo` 是当前 session 中“第一个未完成的 Todo”。

规则：

1. Todo 按 `addTodo` 调用顺序排队。
2. 第一次调用 `addTodo` 后，第一个 Todo 自动成为 `current todo`。
3. 如果连续创建多个 Todo，`current todo` 仍然是列表中第一个未完成的 Todo。
4. 当当前 Todo 完成后，系统自动将下一个未完成 Todo 作为新的 `current todo`。
5. 如果没有未完成 Todo，则 `current todo` 为空。
6. `current todo` 概念贯穿整个 session，用于从 plan 模式切换到执行模式时定位下一步要执行的 Todo。

---

## 4. Todo 执行路径与上下文回流

Todo 工具的设计目标之一，是支持树状执行路径。

- 主干上下文负责 plan。
- 每个 Todo 可以在更干净、有限的上下文中执行。
- Todo 执行完成后，结果回流到主干。
- 主干默认只保留该 Todo 的状态、简短 summary 和最终 report。
- 如果最终 report 较长，最后完成的 Todo 的完整 report 仍应直接保存在主干可见位置，因为主干通常需要立即看到它。
- 更早完成的 Todo 在主干上可以只展示状态和简短 summary。
- 如需查看某个 Todo 的内部工作记录，需要调用 `todo show` 进入该 Todo 的工作目录。

---

## 5. 接口需求

Todo 工具的 prompt-visible 接口应保持极小集合。原则是：**能写进自然语言任务描述里的信息，不再拆成独立参数**；Task 的创建后管理不在 Todo 工具里重复实现。

建议保留 6 个工具：

| 工具 | 用途 |
| --- | --- |
| `addTodo` | 创建当前 session 内的一个 Todo。 |
| `currentTodo` | DO 模式读取当前要执行的 Todo。 |
| `listTodo` | 查看当前 session 的 Todo，以及本 workspace 正在关注的 delegated Task 摘要。 |
| `finishTodo` | 结束当前 Todo，写入状态、summary 和 report。 |
| `cleanTodo` | **Replan 专用**：清空所有 `pending` Todo，给"刷一次 baseline 再重新 add"一个干净起点。仅删 `pending`，不动任何已动过的 Todo。详见 §8.2 例外。 |
| `delegateTask` | 发起一个系统级 Task，并把 task 指针登记到当前 workspace。 |

`todo show` 属于追溯 / 调试用 CLI，常规 plan / do prompt 不需要主动使用，因此不放进核心 prompt-visible 工具集合。

### 5.1 `addTodo`

#### 用途

创建一个绑定到当前 session 的 Todo。**语义副作用**：这是显式地告诉上层 Behavior “已经规划出可执行分支，可以切到 DO”；工具本身只写数据，不直接切换 behavior。

#### 语义

`addTodo` 只负责把当前要做的工作加入当前 session 的 Todo 队列。

它不负责：

- 选择外部执行者；
- 创建系统级 Task；
- 建立复杂依赖关系；
- 修改已有 Todo；
- 删除已有 Todo。

#### 输入字段

建议字段：

| 字段 | 必填 | 说明 |
| --- | --- | --- |
| `title` | 是 | 一句话说明 Todo 的具体目的，用于列表展示和 DO 阶段锚点。 |
| `content` | 是 | 可执行正文。验收点、步骤、约束、标签和已知证据都优先写在这里。 |
| `skills` | 否 | 从当前系统已安装 skills 中选择的技能列表，最多 3 个。能由系统推断时可省略。 |
| `context` | 否 | 创建该 Todo 时的必要上下文，说明为什么创建它。默认由 runtime 截取当前回合上下文。 |

#### Skills 限制

- `skills` 必须从当前系统已安装的 skills 中选择。
- 每个 Todo 最多只能绑定 3 个 skills。
- skills 是提示增强，不是硬性执行参数；不要为了凑字段随意附加。

#### 返回字段

建议返回：

| 字段 | 说明 |
| --- | --- |
| `todoID` | 短字符串 ID，和 `taskID` 同构但更短，建议约 3 位左右。 |
| `status` | 初始状态，通常为 `pending`。 |
| `orderIndex` | Todo 在当前 session 队列中的顺序。 |
| `isCurrent` | 是否成为当前 `current todo`。 |

#### 示例

```bash
todo add "整理 Agent Todo CLI 工具需求文档" --content "区分 Task/Todo，收敛工具名和参数，说明 delegateTask 后续走 task_management。" --skill docs
```

---

### 5.2 `currentTodo`

#### 用途

DO 模块进入执行模式后，调用本接口拿到**当前要执行的那一个** Todo —— DO 不做选择。

#### 语义

- 返回当前 session 的 `current todo`（§3.3 给出的定义：第一个未完成的 Todo）。
- 同时返回执行该 Todo 所必需的上下文：任务描述、skills、创建上下文。
- 若没有未完成 Todo，返回空 —— 这意味着该 session 的 DO 阶段已经结束，Behavior 应回到 plan。
- 这是一个**只读**接口，不改变状态；状态由 `finishTodo` 收尾。

#### 返回字段

| 字段 | 说明 |
| --- | --- |
| `todoID` | 当前 Todo 的 ID。 |
| `status` | 当前状态。 |
| `title` | 一句话目的。 |
| `content` | 可执行正文。 |
| `skills` | 绑定 skills。 |
| `creationContext` | 创建该 Todo 时的回合上下文。 |
| `orderIndex` | 在队列中的顺序。 |

### 5.3 `listTodo`

#### 用途

查看当前 session 的 Todo 列表及当前 workspace 关注的 delegated Task 摘要。

#### 语义

- `listTodo` 返回当前 session 的 Todo 队列。
- Todo 按创建顺序展示。
- 当前 `current todo` 应有明确标记。
- delegated Task 可混入列表展示，但只展示摘要和 `taskID`。
- delegated Task 的详细状态、等待、更新、取消等流程交给标准 `task_management` 工具。
- 每次进入 plan 模式时，系统应自动展示 TodoList，因此该命令更多是手动检查用。

#### 返回字段

建议返回：

| 字段 | 说明 |
| --- | --- |
| `id` | `todoID` 或 `taskID`。 |
| `type` | `todo` 或 `task`。 |
| `status` | Todo 的状态；Task 为最近缓存状态或 `unknown`。 |
| `title` | Todo 标题；Task 为简短摘要。 |
| `skills` | Todo 绑定的 skills；Task 可为空。 |
| `isCurrent` | 是否是当前 Todo。 |
| `updatedAt` | 最近更新时间。 |

#### 示例

```bash
todo list
```

---

### 5.4 `finishTodo`

#### 用途

Worker 执行当前 Todo 后，调用该接口设置最终状态并回流结果。

#### 语义

- 默认作用于当前 `current todo`，因此通常不需要传 `todoID`。
- 成功完成时 status 默认为 `completed`。
- 失败、超时、阻塞时显式传状态，并在 summary / report 里写清原因。
- `finishTodo` 只更新 Todo；不能用于更新 Task。Task 状态变化走 `task_management`。

#### 建议状态枚举

| 状态 | 说明 |
| --- | --- |
| `completed` | 已成功完成。 |
| `failed` | 已失败。 |
| `timeout` | 执行过程超时。 |
| `blocked` | 被其他条件阻塞，暂不能继续。 |

#### 输入字段

建议字段：

| 字段 | 必填 | 说明 |
| --- | --- | --- |
| `summary` | 是 | 一句话摘要，说明本次执行结果。 |
| `status` | 否 | 默认 `completed`；失败、超时、阻塞时显式传。 |
| `report` | 否 | Todo 的最终报告。较长内容可通过文件传入。 |
| `todoID` | 否 | 默认当前 Todo；仅手动修正非当前 Todo 时使用。 |

#### 示例

```bash
todo done "已整理出 Agent Todo CLI 工具需求文档" --report-file ./report.md
```

```bash
todo finish --status failed "未能完成：缺少 task_management 标准接口文档"
```

---

### 5.5 `delegateTask`

#### 用途

发起一个系统级 Task，把它交给外部执行者或外部系统，并把返回的 `taskID` 登记到当前 workspace，供后续 plan 模式展示。

#### 语义

`delegateTask` 是 Todo 工具与系统级 Task 的唯一交界面。它只做三件事：

1. 调用标准 `task_management` 能力创建 / delegate 一个 Task。
2. 在 workspace 维度记录 `taskID`、发起 session 和 purpose。
3. 返回 `taskID`，让后续流程直接使用 `task_management` 查询、等待、更新或取消。

它不负责：

- 提供 `track / untrack / refresh` 等二次跟踪命令；
- 在 Todo 工具里维护 Task 状态机；
- 用 Todo 状态更新接口修改 Task；
- 替代 `task_management` 的查询、等待、超时、取消、完成等能力。

#### 输入字段

建议字段：

| 字段 | 必填 | 说明 |
| --- | --- | --- |
| `task` | 是 | 一段自然语言任务描述，说明要 delegate 什么。 |
| `to` | 否 | 目标执行者或系统；省略时由 `task_management` 默认路由。 |
| `context` | 否 | 委托原因 / 背景；省略时由 runtime 带上当前 session 和 workspace 上下文。 |

#### 返回字段

| 字段 | 说明 |
| --- | --- |
| `taskID` | 系统级 Task ID。 |
| `status` | 初始状态，通常为 `pending` 或 `running`。 |
| `purpose` | 登记到 workspace 的目的说明。 |

#### 示例

```bash
delegateTask "请 reviewer 检查 Agent TodoList 文档的新工具命名和 task_management 边界是否清楚" --to reviewer
```

后续查询和处理：

```bash
task_management status task_01J...
task_management wait task_01J...
task_management update task_01J... --status timeout --summary "reviewer 超时未反馈"
```

---

## 6. 状态流转

### 6.1 Todo 状态流转

建议流转：

```text
pending -> running -> completed
pending -> running -> failed
pending -> running -> timeout
pending -> blocked -> running -> completed
```

说明：

- `addTodo` 创建后默认为 `pending`。
- 当 Todo 被选为当前执行项并进入执行模式，可转为 `running`。
- Worker 完成后必须写入最终状态。
- 失败或超时时应在 summary / report 中写明原因。
- 完成后建议写入 report。

### 6.2 Current Todo 计算

```text
currentTodo = TodoList 中第一个 status 不属于 completed / failed / timeout 的 Todo
```

若业务上认为 `failed` 或 `timeout` 后仍需要人工处理，则可以把它们保留为未完成状态；但默认建议将 `completed`、`failed`、`timeout` 都视为终态。

---

## 7. Delegated Task 与 TodoList 的关系

### 7.0 持久化层级与归属

- Delegated Task 一旦发出，已经对外部产生影响 —— 它的记录必须保存在 **Workspace 维度**，跨 session 可见（详见 §2.1 / §2.2）。
- 即使发起它的 session 已经结束，下次另一个 session 关联到同一 Workspace 时，TodoList 仍应能列出这个 Task 及其最新状态。
- 每个 delegated Task 必须显式记录：
  - **发起 session**：由哪一个 session 发起；
  - **目的（purpose / context）**：基于什么目的发出。
- Workspace 维度的隔离是硬要求：不同 Workspace 之间不互相看见对方 delegate 出去的 Task。

#### 归属信息落地：workspace 内的小 meta 文件

Task 的**状态**仍由系统级 `TaskMgr` 管理，全分布式可查（§2.2）；但"**哪些 Task 属于本 workspace**"这条归属关系，落在 workspace 自身目录里的一个小 meta 文件上，而**不是**绕 TaskMgr 加一个 `workspace_id` 索引。

这样做的核心理由是：**workspace 的状态可以完全用传统 bash 工具理解**（`cat`、`jq`、`ls`、`grep`），不依赖 RPC、不依赖任何 agent runtime 在线。任何人 / 任何工具直接看 workspace 目录就知道本工作空间在等哪些外部任务。

落地约定：

- meta 文件路径：`<workspace>/.agent/tasks.json`（workspace 级，跨 session 保留）和 `$AGENT_ROOTFS/sessions/<session_id>/todos.json`（session 级，session 结束可清）。
- meta 文件只保存"指针"，不存 Task 的全部状态：
  - `taskID`（TaskMgr 的真实 ID）
  - `purpose` / `originSessionID` / `label`
  - `cachedStatus` + `cachedAt`（最近一次从 TaskMgr 拉到的状态快照，便于离线 / 加速）
- Task 的真实状态以 `task_management` / TaskMgr 为准。`listTodo` 只展示最近缓存状态和 `taskID`，需要精确状态时直接调用 `task_management status <taskID>`。
- workspace 之间的隔离由"各自有各自的 meta 文件"自然完成，不再需要 TaskMgr 侧加 workspace 维度查询。
- session 结束 / 删除时，只清 `todos.json`，**不动 `tasks.json`** —— delegated Task 已经对外部产生影响，归属记录必须保留。

### 7.1 展示关系

在 plan 模式下，Agent 可以在历史或 TodoList 中看到 delegated task。

要求：

- delegated task 可以混入 TodoList 展示。
- delegated task 必须明确标记为 `task`，避免与 session 内 Todo 混淆。
- 未完成 delegated task 应置顶或突出显示。
- 展示时可以使用缓存状态；需要精确状态时应调用 `task_management status <taskID>`。

### 7.2 执行关系

- delegated Task 的状态变化不由 Todo 工具直接驱动。
- 如果后续 Todo 依赖某个 delegated Task 的结果，Agent 使用标准 `task_management` 等待、安排稍后检查，或在超时后 replan。
- 如果 delegated Task 太久没有反馈，Agent 通过 `task_management` 将其标记为 `timeout` 或追加说明，再决定是否重新 plan。
- 已 delegate 出去的任务不应被删除，因为它可能已经产生外部影响。

---

## 8. 不支持的能力

以下接口明确不支持：

### 8.1 不支持修改 Todo / Task

不提供 `modifyTodo` 或类似接口。

原因：

- Todo 一旦创建，就代表一次明确的计划记录。
- 修改已有 Todo 会破坏执行路径和审计记录。
- 如果计划变化，应创建新的 Todo 或通过状态说明旧 Todo 的结果。

### 8.2 不支持删除 Todo / Task（`cleanTodo` 是唯一例外）

不提供 `removeTodo` 或类似按 ID 删除的接口。

原因：

- 已创建或已 delegate 的任务可能已经产生外部影响。
- 删除会掩盖历史行为。
- 更合适的做法是显式标记为 `failed`、`timeout` 或其他终态，并说明原因。

**唯一例外：`cleanTodo`（replan 专用）**

- 只删 `pending` Todo —— 这些只是 plan 阶段写入的"下一步分支声明"，从未执行过，**没有外部副作用，也不构成执行审计的一部分**，因此不在"防掩盖历史"约束之内。
- **不动** `running` / `blocked` 和任何终态（`completed` / `failed` / `timeout`）。已经动过的 Todo 仍然按 §8.1 的规则保留。
- **不按 ID 删**。`cleanTodo` 是一次"整批清空 pending"，不提供"删某个 pending"的语义 —— 那等价于 §8.1 的"修改计划"。
- 设计意图是给 replan 一个干净 baseline：`cleanTodo` 之后连续 `addTodo` 出来的新队列对应一次明确的"重新规划事件"，主干和后续 session 看 list 时无歧义。

> 在 §6.2 的 currentTodo 计算下，`pending` 的删除不会改变"current = 第一个未完成"的语义，但会让 `orderIndex` 重排（实现侧应在 clean 后把剩余 Todo 压紧到 `0..N-1`）。

### 8.3 不支持复杂依赖图

Todo 队列不维护复杂依赖关系。

- Todo 按创建顺序执行。
- 若发生依赖等待，应通过 plan 逻辑、状态更新或 scheduled check 处理。
- 不在 Todo 工具本身实现 DAG / dependency graph。

### 8.4 不支持选择执行者

`addTodo` 不包含 executor / assignee 字段。

- Todo 默认由当前 Agent 在执行模式完成。
- 如果需要外部执行，应调用 `delegateTask` 创建系统级 Task，而不是创建 Todo。

---

## 9. 建议数据模型

### 9.1 Todo

```ts
type Todo = {
  todoID: string;              // 短 ID，建议约 3 位
  sessionID: string;           // 所属 session
  orderIndex: number;          // 创建顺序
  status: TodoStatus;
  title: string;                // 一句话说明具体目的
  content: string;              // 可执行正文，包含范围 / 验收点 / 约束 / 已知证据等
  skills?: string[];           // 从已安装 skills 中选择，最多 3 个
  creationContext?: string;    // 创建该 Todo 的上下文 / 原因
  report?: string;             // 最终报告
  workspace?: string;          // Todo 内部工作目录
  createdAt: string;
  updatedAt: string;
};
```

### 9.2 Task 展示项

```ts
type TaskListItem = {
  taskID: string;
  type: "task";
  status: TaskStatus;
  purpose: string;
  summary?: string;
  updatedAt?: string;
};
```

### 9.3 TodoList 展示项

```ts
type TodoListItem = {
  id: string;
  type: "todo" | "task";
  status: string;
  summary?: string;
  isCurrent?: boolean;
  orderIndex?: number;
  skills?: string[];
  updatedAt?: string;
};
```

---

## 10. 典型流程

### 10.1 创建多个 Todo 并顺序执行

```text
进入 plan 模式
  -> addTodo(A)
  -> addTodo(B)
  -> addTodo(C)
  -> TodoList 显示 A/B/C
  -> current todo = A
退出 plan 模式，进入执行模式
  -> Worker 执行 A
  -> finishTodo(summary, report)
回到 plan 模式
  -> 自动展示 TodoList
  -> 判断是否需要replan或则是已经完成
退出plan模式，进入执行模式
  -> current todo = B
  -> 继续执行 B
...
回到 plan 模式
  -> 自动展示 TodoList
  -> 判断完成  
```

### 10.2 等待 delegated task

```text
plan 模式中看到 delegated task T
  -> T 未完成，在TodoList置顶展示(可以看到summary状态)
  -> 用 task_management tool 查询 T 详细状态
  -> 如果后续 Todo 依赖 T：
       - 等待；或
       - 设置稍后检查计划；或
       - 若长时间无反馈，task_management update(T, timeout)
       - 决定是否 replan
```

### 10.3 查看某个 Todo 的内部记录

```text
Todo A 已完成
主干只看到：状态 + summary + report
如需查看内部执行过程：
  -> todo show A
  -> 返回 Todo 工作目录说明
  -> 根据 manifest 继续查看文件和细节
```

---

## 11. 验收标准

1. `addTodo` 能创建 session 级 Todo，返回短 `todoID`。
2. Todo 在 session 删除后不再可用。
3. `addTodo` 必须支持 `title`、`content` 和可选 skills，且 skills 最多 3 个。
4. skills 必须从已安装 skills 中选择。
5. plan 模式可以连续创建多个 Todo，Todo 按创建顺序执行。
6. `current todo` 始终是第一个未完成 Todo。
7. 执行模式只能看到当前 Todo 及必要创建上下文。
8. Todo 完成后，主干可看到状态、summary 和最终 report。
9. `listTodo` 能展示当前 session 的 Todo 状态，并标记 current todo。
10. `listTodo` 能混入展示 delegated Task 摘要和 `taskID`，但不重复实现 Task 跟踪逻辑。
11. `todo show` 能通过 TodoID 获取详细信息或工作目录说明；Task 详情走 `task_management`。
12. `finishTodo` 能设置成功、失败、超时等 Todo 状态，并记录 summary / report。
13. 系统不提供按 ID 的 modify/remove 接口；`cleanTodo` 是唯一例外，仅整批清 `pending`、不动其它状态（详见 §8.2）。
14. `cleanTodo` 执行后剩余 Todo 的 `orderIndex` 必须压紧到 `0..N-1`，保证后续 `addTodo` 重建 baseline 时编号语义正确。
15. `delegateTask` 是 Todo 工具唯一新增的 Task 相关能力；后续查询、等待、更新、取消统一走 `task_management`。

---

## 12. 待确认事项

以下内容已有方向，但需要最终定稿：

1. `todoID` 的具体长度：语音中倾向非常短，约 3 位；需确认是否固定长度。
2. Todo 工作目录结构的具体文件命名与 manifest 格式。
3. `blocked` 是否作为正式状态，还是只保留 `pending/running/completed/failed/timeout`。
4. “设置稍后检查计划”的能力归属：默认交给标准 `task_management` / scheduler，不放进 Todo CLI。
5. **Todo 是否提供 note 接口（中间记录）**：PDCA 有 notes。Todo 当前倾向**不加** —— 工具面尽量小，中间过程交给分支自身的推理历史 / 工作目录承担，要看细节直接进对应分支历史查。仍在考虑中。
6. **Worker 完成被主干否决后的回流路径**：旧版的 `CHECK_FAILED` 在新版状态机里已被砍掉。当主干评审 Worker 产物不通过时，是新建一个 Todo 来重做（更符合§8.1 “不修改 / 要重做就新建”原则），还是补一个 `needs_revision` 态？默认走前者，需文档显式声明。
7. **Workspace 与 Session 关联模型**：同一 Workspace 在不同时刻被不同 Session 关联时，TodoList 中"前任 session 留下的 delegated Task"如何展示（按时间倒序？按未完成置顶？），需要补一份具体规则。
8. **meta 文件并发写**：同一 workspace 同时被两个 session 关联时，对 `.agent/tasks.json` 的并发写策略（文件锁？追加日志 + 周期 compact？）需补一条具体规则。

---

## 13. CLI 命令设计

Todo 工具以 CLI shim 形式落地，对接 Behavior 层的 `exec_bash` 通道（参见 `doc/opendan/Agent Actions.md`）。prompt 中优先暴露短工具名：

- `todo`：session 级 Todo 管理。
- `delegateTask`：系统级 Task 委托入口。
- `task_management`：Task 创建后的标准查询、等待、更新、取消能力。

不再把 `task track / untrack / refresh` 这类 Task 跟踪命令放进 Todo 工具。只要是 Task，后续流程都走 `task_management`。

### 13.1 Todo 子命令

#### `todo add`

创建 Todo。标题和正文必填，其他信息优先写进正文，减少提示词里的参数判断。

```bash
todo add "<title>" --content <text> [--skill <name>]... [--context <text>]
```

说明：

- `<title>` 是一句话目的，用于列表和 DO 阶段锚点。
- `--content` 是可执行正文，可以包含验收点、步骤、标签、约束和已知证据。
- `--skill` 可重复，最多 3 个，必须来自已安装 skills。
- `--context` 可省略；省略时 runtime 自动带上当前回合上下文。

#### `todo current`

返回当前要执行的 Todo。

```bash
todo current
```

无未完成 Todo 时返回 `null`，这是 DO 阶段结束信号。

#### `todo list`

查看当前 Todo 队列和 workspace 正在关注的 delegated Task 摘要。

```bash
todo list
```

说明：

- 默认展示当前 session 的 Todo。
- 默认混入 `.agent/tasks.json` 里由 `delegateTask` 登记的 Task 摘要。
- Task 只显示 `taskID`、purpose、缓存状态和更新时间；精确查询走 `task_management status <taskID>`。

#### `todo done`

完成当前 Todo。成功路径尽量短。

```bash
todo done "<summary>" [--report <text> | --report-file <path>]
```

#### `todo finish`

非成功终态或手动修正时使用。

```bash
todo finish --status <completed|failed|timeout|blocked> "<summary>" [--report <text> | --report-file <path>] [--id <todoID>]
```

说明：

- 默认作用于当前 Todo。
- `--id` 仅用于修正非当前 Todo，正常 DO 流程不需要。
- Task 状态不能通过 `todo finish` 修改。

#### `todo show`

查看 Todo 详情和工作目录说明。

```bash
todo show [<todoID>]
```

说明：

- 不传 `todoID` 时展示当前 Todo。
- 只处理 Todo；Task 详情走 `task_management show <taskID>`。

#### `todo clean`

Replan 专用：清空所有 `pending` Todo，给"刷一次 baseline 再 add 新计划"一个干净起点。

```bash
todo clean
```

说明：

- **只删 `pending`**（plan 阶段写入、未执行过、无外部副作用）。
- **不动** `running` / `blocked` 和任何终态（`completed` / `failed` / `timeout`）—— 已动过的 Todo 仍按 §8.1 / §8.2 规则保留。
- 不按 ID 删；不提供 `--id` 选项。要删某个具体 pending，等价于"修改计划"，按 §8.1 走 `addTodo` 即可。
- 清理后剩余 Todo 的 `orderIndex` 压紧到 `0..N-1`，保证后续 `addTodo` 编号语义正确。
- 返回：被清掉的 `todoID` 列表 + 保留下来的条数，便于主干日志记录"这次 replan 抹掉了哪些待执行项"。

典型用法（见 §14.6）：

```bash
todo clean
todo add "<新 plan 第 1 步标题>" --content "<新 plan 第 1 步正文>"
todo add "<新 plan 第 2 步标题>" --content "<新 plan 第 2 步正文>"
```

### 13.2 `delegateTask`

`delegateTask` 是 Todo 工具唯一新增的 Task 相关入口：创建 / 委托系统级 Task，并把 `taskID` 指针登记到当前 workspace。

```bash
delegateTask "<task>" [--to <target>] [--context <text>]
```

说明：

- `<task>` 是自然语言任务描述。
- `--to` 可省略，省略时由 `task_management` 默认路由。
- `--context` 可省略，省略时 runtime 自动带上当前 session / workspace / 当前 plan 上下文。
- 返回 `taskID` 后，后续一律走 `task_management`。

示例：

```bash
delegateTask "请 reviewer 检查 Agent TodoList 文档的新工具命名和 task_management 边界是否清楚" --to reviewer
task_management status task_01J...
task_management wait task_01J...
```

### 13.3 环境变量与路径解析

Todo CLI 是一个独立子进程，**不能假设自己在哪一个 session / 哪一个 workspace 里**。所有路径解析都靠 runtime 在 `exec_bash` 时注入的 env 变量。这套变量同时也是其他 agent CLI 工具的通用上下文，复用 [llm_bash.rs](../../src/frame/agent_tool/src/llm_bash.rs) 已有的 per-call env 注入通道。

| 环境变量 | 含义 | 由谁注入 |
| --- | --- | --- |
| `BUCKYOS_ROOT` | BuckyOS 安装根（已存在，见 [paths.rs](../../src/frame/opendan/src/paths.rs)） | 进程启动时 |
| `AGENT_ROOTFS` | 当前 agent 在 host 上的可写根（典型：`$BUCKYOS_ROOT/tools/<agent_id>`），承载 sessions/、workspaces/ 等子目录 | runtime 启动 session 时 |
| `WORK_SESSION_ID` | 当前 work session 的 id | runtime 每次 `exec_bash` 注入 |
| `WORKSPACE_ID` | 当前 session 绑定到的 workspace id（可选；缺省时按 cwd 反查 `session_workspace_bindings.json`） | runtime 每次 `exec_bash` 注入 |
| `WORKSPACE_DIR` | 当前 workspace 在 host 上的实际目录（绝对路径），通常 == `exec_bash` 的 cwd | runtime 每次 `exec_bash` 注入 |

面向 LLM 的主流程不暴露 `--ws` / `--session` / `--agent` / `--op-id` 等全局 flag。它们可以作为隐藏调试参数存在，但不写入 skill 提示词，避免模型在必填/选填之间做无意义判断。

#### 路径解析规则

- **`todos.json`（session 级）** —— 落在 agent runtime fs 里，不污染用户工作目录：

  ```
  $AGENT_ROOTFS/sessions/$WORK_SESSION_ID/todos.json
  ```

  缺少 `AGENT_ROOTFS` 或 `WORK_SESSION_ID` 时，CLI 必须以非零退出码并打印明确错误，不做静默回退。

- **`tasks.json`（workspace 级）** —— 落在 workspace 自身目录里，让 `cat` / `jq` / `grep` 从普通终端就能看：

  ```
  $WORKSPACE_DIR/.agent/tasks.json
  ```

  若 `WORKSPACE_DIR` 未注入但 `WORKSPACE_ID` 存在，从 `$AGENT_ROOTFS/workspaces/$WORKSPACE_ID/` 的 manifest 反查实际目录；都不存在则按 cwd 向上找最近一层带 `.agent/` 的目录。

#### 为什么把两者拆到不同位置

- `todos.json` 是 LLM Behavior 引导记录，session 结束就该消失，放进 `$AGENT_ROOTFS/sessions/<id>/` 跟着 session 生命周期一起清理；放进用户工作目录会留垃圾。
- `tasks.json` 是对外部已产生影响的指针，必须跨 session 可见，**而且**要能脱离 agent runtime 用纯 bash 看 —— 必须落在 workspace 自身目录里（§7.0 已说明）。

### 13.4 meta 文件结构

```
$AGENT_ROOTFS/
  sessions/
    <WORK_SESSION_ID>/
      todos.json        # session 级。session 删除时一起清

$WORKSPACE_DIR/
  .agent/
    tasks.json          # workspace 级。session 结束不动
```

`tasks.json` 单条 entry 示意：

```json
{
  "taskID": "task_01JHKABCDE...",
  "purpose": "等 reviewer 反馈 v2 protocol 草案",
  "originSessionID": "sess_2026_05_16_xxx",
  "label": "v2-proto-review",
  "cachedStatus": "running",
  "cachedAt": "2026-05-16T08:12:00Z",
  "addedAt": "2026-05-15T19:40:00Z"
}
```

`todos.json` 内 Todo 字段对齐 §9.1 的 `Todo` 数据模型；为便于 bash 工具扫读，建议数组顺序与 `orderIndex` 一致。

### 13.5 与 Behavior 模式的协作

Todo 工具本身**不切换 behavior**，只动数据。Plan→DO 的模式切换由 LLM 在同一回合发 `<next_behavior>do</next_behavior>` 触发，runtime 已经支持基于 `next_behavior` 的 `switch_behavior` 与 `fork_and_run`（子上下文 fork-join，执行完自动 join 回主干）。所以 `todo add` 在工具侧只负责"写一条 pending Todo"，模式切换通过 prompt-coupled 的一级标签承担 —— 这与 v2 Actions 协议的 prompt-coupled 固化集合原则一致。

---

## 14. 典型使用方法

### 14.1 Plan：连续创建多个 Todo

```bash
todo add "整理 v2 actions 协议文档" --content "对齐 §1 7-action 列表并补 examples。背景：让 opendan 接入 beta2.2 Behavior 协议" --skill docs --skill product-spec
todo add "agent_todo_tool CLI flags 对齐新文档" --content "更新 CLI 参数说明，确保 add/done/finish 与实现一致。" --skill code-rust
todo add "补 e2e 测试覆盖 addTodo→DO fork→finishTodo 路径" --content "覆盖 addTodo 创建、DO fork 执行、finishTodo 回流主干。" --skill code-rust --skill testing

todo list
# T01  pending  current   整理 v2 actions 协议文档
# T02  pending            agent_todo_tool CLI flags 对齐新文档
# T03  pending            补 e2e 测试覆盖 ...
```

随后 LLM 在同一回合内发 `<next_behavior>do</next_behavior>`，runtime 进入 DO behavior 并 `fork_and_run` 一个子上下文执行 T01。

### 14.2 DO：在子上下文里执行单个 Todo

```bash
todo current
# {
#   "todoID": "T01",
#   "title": "整理 v2 actions 协议文档",
#   "content": "对齐 §1 7-action 列表并补 examples。背景：让 opendan 接入 beta2.2 Behavior 协议",
#   "skills": ["docs", "product-spec"],
#   "orderIndex": 0
# }

# ... 干活 ...

todo done "v2 actions 协议文档已整理，§1 列表对齐" --report-file ./report.md
```

子上下文结束、join 回主干 —— 主干 plan 回合自动看到 TodoList 刷新后的状态，进入 T02。

### 14.3 跟踪 delegated Task

```bash
# delegateTask 创建系统级 Task，并自动登记到当前 workspace
delegateTask "请 reviewer 反馈 v2 protocol 草案" --to reviewer

# 下次进 plan 模式时
todo list
# [TASK]  v2-proto-review  running   task_01JHKABCDE...
# T01     pending          current   整理 v2 actions 协议文档
# ...

# 精确状态和后续处理走 task_management
task_management status task_01JHKABCDE...
task_management update task_01JHKABCDE... --status timeout --summary "等了 3 天 reviewer 未响应"
```

注意：Todo 工具不提供 `task track / untrack / refresh`。`delegateTask` 之后，只要是 Task 的状态查询、等待、超时、取消、完成，都交给标准 `task_management`。

### 14.4 完全用 bash 检查 workspace 状态

不启 agent runtime、不连 RPC 的情况下也能看清楚本 workspace 在等什么 —— `tasks.json` 落在 workspace 目录里，普通 shell session 直接读：

```bash
# 在 workspace 目录中
cd "$WORKSPACE_DIR"          # 或就在该 workspace 的 cwd 下

# 所有未到终态的 delegated task
jq '.[] | select(.cachedStatus != "completed"
              and .cachedStatus != "failed"
              and .cachedStatus != "timeout")' \
  .agent/tasks.json

# 历次 session 留下的 delegated task 起源
jq '.[] | {taskID, purpose, originSessionID, cachedStatus}' \
  .agent/tasks.json
```

session 级 `todos.json` 落在 agent runtime fs，需要走 `AGENT_ROOTFS`：

```bash
jq '.[] | select(.status == "pending" or .status == "running")' \
  "$AGENT_ROOTFS/sessions/$WORK_SESSION_ID/todos.json"
```

如果手头没有 env，可以列所有 session 的 `todos.json`：

```bash
find "$AGENT_ROOTFS/sessions" -name todos.json -print
```

### 14.5 查看某个完成 Todo 的内部记录

```bash
todo show T01
# {
#   "id": "T01", "type": "todo", "status": "completed",
#   "summary": "...", "report": "...",
#   "workspace": ".agent/todos/T01/",
#   "manifest": [
#     {"file": "thoughts.md",      "purpose": "推理过程"},
#     {"file": "tool_calls.jsonl", "purpose": "工具调用流水"},
#     {"file": "report.md",        "purpose": "最终报告"}
#   ]
# }

# 按 manifest 继续展开
cat .agent/todos/T01/thoughts.md
```

### 14.6 Replan：清空旧 pending 再重建 baseline

```bash
# Plan 阶段评估 list 后认为前面规划已经失效，重新来一遍
todo list
# T01  completed             整理 v2 actions 协议文档
# T02  pending     current   agent_todo_tool CLI flags 对齐
# T03  pending               补 e2e 测试覆盖

todo clean
# { "removed": ["T02", "T03"], "kept": 1 }

# 重新规划 —— 新 Todo 接着 baseline 重新编号
todo add "重写 §1 7-action 列表后端 schema 对齐脚本" --content "按最新协议重写 schema 对齐脚本，并说明输入输出。" --skill code-rust
todo add "补 protocol mismatch 的回归测试" --content "覆盖协议字段不一致时的失败路径和错误提示。" --skill testing

todo list
# T01  completed             整理 v2 actions 协议文档     （保留：已是终态）
# T02  pending     current   重写 §1 7-action 列表后端 ...
# T03  pending               补 protocol mismatch 回归测试
```

关键点：

- `T01` 完成态保留，replan 不抹掉已执行轨迹。
- 旧 `T02 / T03`（pending）被清空，新 `T02 / T03` 是"replan 后的第一轮 Todo"，编号与位置都对得上一次新的 plan 事件。
- `cleanTodo` 返回值里的 `removed` 列表可以被主干写进当次 plan 回合的推理日志，便于后续审计"这次 replan 抹掉了哪些待执行项"。
