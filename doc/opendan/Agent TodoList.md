# Agent Todo CLI 工具需求

> 来源：根据语音记录整理。本文档用于明确 Agent Todo CLI / 工具接口的核心概念、接口语义、状态流转和约束。

## 1. 背景与目标

Agent 在执行复杂任务时，需要把当前 work session 中的工作拆成一组较小的 Todo，并把每个 Todo 交给更干净的上下文去执行。Todo 工具的目标不是创建一个全局任务系统，而是在当前 session 内维护一个有序、可追踪、可回溯的工作列表。

核心目标：

1. 在 `plan` 模式下创建、查看、管理当前 session 的 Todo 列表。
2. 在执行模式下，Agent 只看到并执行当前 `current todo`。
3. Todo 执行完成后，主干上下文只接收该 Todo 的状态、简短摘要和最终报告。
4. 如需深入了解某个 Todo 的内部执行过程，可通过详情接口进入该 Todo 的工作记录目录继续探索。
5. 与系统级 `Task` 区分清楚：Task 是全局、长期、可分布式查询状态的任务；Todo 是 session 级、临时、只在当前 work session 内有效的工作项。

---

## 2. 核心概念

### 2.1 Task：系统级任务

`Task` 是系统级概念。

- 一旦创建，Task 可通过 `taskID` 在整个分布式系统中查询状态。
- Task 是长期存在的，不依赖当前 work session 生命周期。
- Task 可能被 delegate 到外部执行者或外部系统。
- Task 的状态变化不依赖 `plan` 模式本身。
- 如果 Agent 需要等待某个 delegated task 的结果，Agent 应等待、设置后续检查计划，或者在超时后决定是否重新 plan。

### 2.2 Todo：Session 级工作项

`Todo` 是当前 work session 内部的工作项。

- Todo 与当前 session 绑定。
- session 被删除后，Todo 列表及其 Todo 项也随之失效。
- Todo 的有效范围只在当前 session 内。
- Todo 用来承载 Agent 在当前 session 中拆出来的下一步执行单元。
- Todo 没有外部执行者选择，默认由当前 Agent 在执行模式下完成。

### 2.3 Task 与 Todo 的区别

| 维度 | Task | Todo |
| --- | --- | --- |
| 生命周期 | 长期存在 | 随 session 存在 |
| 作用域 | 系统级 / 分布式系统级 | 当前 work session |
| ID | 较长的 `taskID` | 较短的 `todoID` |
| 可查询性 | 可全局查询状态 | 仅当前 session 内有效 |
| 执行者 | 可 delegate 给外部执行者或系统 | 默认由当前 Agent 执行 |
| 删除 session 后 | 仍可能存在 | 不再存在 |
| 是否可混入列表展示 | 可在 TodoList 中展示 delegated task 状态 | TodoList 的主要对象 |

---

## 3. 模式与执行语义

### 3.1 Plan 模式

`plan` 模式负责规划和管理 Todo 列表。

在 plan 模式下：

- 可以多次调用 `createTodo`，一次性创建一组 Todo。
- Todo 不建立复杂依赖关系。
- Todo 按创建顺序依次执行。
- 可以调用 `todoList` 查看当前 session 的 Todo 情况。
- 每次重新进入 plan 模式时，系统应自动展示当前 Todo 列表状态，因此 `todoList` 命令虽然支持，但通常不是必须手动调用。
- plan 模式下可看到 delegated task 混入 TodoList 的状态展示。
- 未完成的 delegated task 应在列表中置顶或突出展示，并自动查询当前状态。

### 3.2 执行模式 / Worker 模式

执行模式只负责完成当前 `current todo`。

在执行模式下：

- Agent 只看到当前 Todo，而不是完整 Todo 列表。
- 当前 Todo 的提示词由 `current todo` 的模板渲染而来。
- Agent 应能看到该 Todo 被创建时所在回合的必要上下文，也就是“为什么创建这个 Todo”。
- Agent 基于当前 Todo 的详细描述、checklist、tags、skills 和创建上下文执行工作。
- Todo 完成后，Agent 需要设置最终状态，并给出摘要和最终报告。
- 主干上下文默认只看到 Todo 完成后的 report / summary，而不是完整内部执行轨迹。

### 3.3 Current Todo

`current todo` 是当前 session 中“第一个未完成的 Todo”。

规则：

1. Todo 按 `createTodo` 调用顺序排队。
2. 第一次调用 `createTodo` 后，第一个 Todo 自动成为 `current todo`。
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
- 如需查看某个 Todo 的内部工作记录，需要调用详情接口进入该 Todo 的工作目录。

---

## 5. 接口需求

### 5.1 `createTodo`

#### 用途

创建一个绑定到当前 session 的 Todo。

#### 语义

`createTodo` 是一个相对纯粹的创建接口，只负责把当前要做的工作作为 Todo 加入当前 session 的 Todo 队列。

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
| `description` | 是 | Todo 的详细描述，说明到底要做什么。 |
| `checklist` | 建议 | 执行该 Todo 时应满足的检查项、步骤或验收点。 |
| `tags` | 建议 | 与 Todo 相关的标签，用于分类、检索和展示。 |
| `skills` | 是 | 从当前系统已安装 skills 中选择的技能列表，最多 3 个。 |
| `context` | 建议 | 创建该 Todo 时的必要上下文，说明为什么创建它。 |

#### Skills 限制

- `skills` 必须从当前系统已安装的 skills 中选择。
- 每个 Todo 最多只能绑定 3 个 skills。
- skills 应根据 Todo 类型选择，不能随意附加。

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
agent todo create \
  --description "整理 Agent Todo CLI 工具需求文档" \
  --checklist "区分 Task/Todo；定义 createTodo/todoList/getDetail/updateState；说明 current todo；列出限制" \
  --tag "requirements" \
  --tag "agent-tooling" \
  --skill "docs" \
  --skill "product-spec"
```

---

### 5.2 `todoList`

#### 用途

查看当前 session 的 Todo 列表及相关 delegated task 状态。

#### 语义

- `todoList` 返回当前 session 的 Todo 队列。
- Todo 按创建顺序展示。
- 当前 `current todo` 应有明确标记。
- delegated task 可混入列表展示。
- 未完成 delegated task 应置顶或突出显示，并自动查询其当前状态。
- 每次进入 plan 模式时，系统应自动展示 TodoList，因此该命令更多是手动检查用。

#### 返回字段

建议返回：

| 字段 | 说明 |
| --- | --- |
| `id` | `todoID` 或 `taskID`。 |
| `type` | `todo` 或 `task`。 |
| `status` | 当前状态。 |
| `summary` | 简短摘要。 |
| `tags` | 标签。 |
| `skills` | Todo 绑定的 skills；Task 可为空。 |
| `isCurrent` | 是否是当前 Todo。 |
| `createdAt` | 创建时间。 |
| `updatedAt` | 最近更新时间。 |

#### 示例

```bash
agent todo list
```

---

### 5.3 详情接口：`getDetail`

> 命名建议：语音中出现了 `getTodo`、`getTodoDetail`、`getTaskDetail`。需求上应统一为一个详情接口，避免维护两个语义重复的函数。建议命名为 `getDetail(id)`，并兼容 TodoID 与 TaskID。若最终仍采用更具体的命名，则应保证 `getTodoDetail` / `getTaskDetail` 在底层是同一能力。

#### 用途

查看某个 Todo 或 Task 的详细信息与工作记录入口。

#### 语义

对于 Todo：

- 根据 `todoID` 获取该 Todo 的详细信息。
- 返回该 Todo 的工作记录目录说明。
- 详情接口不一定直接返回所有内部内容，而是返回一份“目录说明书”。
- 目录说明书应说明该 Todo 目录下有哪些文件、每个文件大致是什么内容、如何继续探索细节。
- 如果 Todo 已完成，应能看到最终状态、summary 和 report。

对于 Task：

- 根据 `taskID` 获取系统级 Task 的详细状态或关联记录。
- delegated task 的状态应来自系统级任务查询，而不是 plan 模式内部状态。

#### 返回字段

建议返回：

| 字段 | 说明 |
| --- | --- |
| `id` | TodoID 或 TaskID。 |
| `type` | `todo` 或 `task`。 |
| `status` | 当前状态。 |
| `description` | 详细描述。 |
| `checklist` | Todo 的 checklist。 |
| `tags` | 标签。 |
| `skills` | 绑定 skills。 |
| `summary` | 简短摘要。 |
| `report` | 完成后的最终报告；未完成时可为空。 |
| `workspace` | 工作记录目录路径或引用。 |
| `manifest` | 目录说明，包括文件列表、用途说明和可继续探索的入口。 |

#### 示例

```bash
agent todo detail abc
```

或统一形式：

```bash
agent detail abc
agent detail task_01J...
```

---

### 5.4 状态更新接口：`updateState`

> 命名建议：语音中出现了 `updateTaskState` 和 `updateTodoState`。需求上二者语义接近，建议统一为 `updateState(id, ...)`，并根据 ID 类型处理 Todo 或 Task。若保留两个命令，也应共享同一套状态枚举与参数结构。

#### 用途

手动更新 Todo 或 Task 的状态。

#### Worker 对 Todo 的使用场景

Worker 执行当前 Todo 后，需要调用该接口设置最终状态。

常见用途：

- 标记 Todo 成功完成。
- 标记 Todo 执行失败。
- 写入失败原因。
- 写入一句话 summary。
- 写入最终 report。

#### Plan 对 delegated task 的使用场景

delegated task 的真实执行者不会调用当前 session 的 Todo 工具，因此 plan 模式需要根据外部任务信息手动更新或标注状态。

常见用途：

- 外部任务长时间无更新时，将其标记为 `timeout`。
- 外部任务状态发生变化后，在当前列表中同步展示。
- 根据任务状态决定是否继续等待、设置后续检查计划，或者重新 plan。

#### 建议状态枚举

| 状态 | 说明 |
| --- | --- |
| `pending` | 已创建，尚未开始。 |
| `running` | 正在执行。 |
| `completed` | 已成功完成。 |
| `failed` | 已失败。 |
| `timeout` | 等待外部任务或执行过程超时。 |
| `blocked` | 被其他条件阻塞，暂不能继续。 |

最终状态通常是：

- `completed`
- `failed`
- `timeout`

#### 输入字段

建议字段：

| 字段 | 必填 | 说明 |
| --- | --- | --- |
| `id` | 是 | TodoID 或 TaskID。 |
| `status` | 是 | 新状态。 |
| `summary` | 是 | 一句话摘要，说明本次状态更新的结果。 |
| `reason` | 建议 | 状态变化原因，失败或 timeout 时尤其重要。 |
| `report` | 完成时建议 | Todo 的最终报告。 |

#### 示例

```bash
agent todo update-state abc \
  --status completed \
  --summary "已整理出 Agent Todo CLI 工具需求文档" \
  --report-file ./report.md
```

```bash
agent task update-state task_01J... \
  --status timeout \
  --summary "外部任务超过预期时间未反馈" \
  --reason "等待时间过长，plan 将重新评估是否 replan"
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

- `createTodo` 创建后默认为 `pending`。
- 当 Todo 被选为当前执行项并进入执行模式，可转为 `running`。
- Worker 完成后必须写入最终状态。
- 失败或超时时应写入 reason 和 summary。
- 完成后应写入 report。

### 6.2 Current Todo 计算

```text
currentTodo = TodoList 中第一个 status 不属于 completed / failed / timeout 的 Todo
```

若业务上认为 `failed` 或 `timeout` 后仍需要人工处理，则可以把它们保留为未完成状态；但默认建议将 `completed`、`failed`、`timeout` 都视为终态。

---

## 7. Delegated Task 与 TodoList 的关系

### 7.1 展示关系

在 plan 模式下，Agent 可以在历史或 TodoList 中看到 delegated task。

要求：

- delegated task 可以混入 TodoList 展示。
- delegated task 必须明确标记为 `task`，避免与 session 内 Todo 混淆。
- 未完成 delegated task 应置顶或突出显示。
- 展示时应自动查询 delegated task 的当前状态。

### 7.2 执行关系

- delegated task 的状态变化不由 plan 模式直接驱动。
- 如果后续 Todo 依赖某个 delegated task 的结果，Agent 只能等待、安排稍后检查，或在超时后 replan。
- 如果 delegated task 太久没有反馈，Agent 可手动将其标记为 `timeout`，再决定是否重新 plan。
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

### 8.2 不支持删除 Todo / Task

不提供 `removeTodo` 或类似接口。

原因：

- 已创建或已 delegate 的任务可能已经产生外部影响。
- 删除会掩盖历史行为。
- 更合适的做法是显式标记为 `failed`、`timeout` 或其他终态，并说明原因。

### 8.3 不支持复杂依赖图

Todo 队列不维护复杂依赖关系。

- Todo 按创建顺序执行。
- 若发生依赖等待，应通过 plan 逻辑、状态更新或 scheduled check 处理。
- 不在 Todo 工具本身实现 DAG / dependency graph。

### 8.4 不支持选择执行者

`createTodo` 不包含 executor / assignee 字段。

- Todo 默认由当前 Agent 在执行模式完成。
- 如果需要外部执行，应创建系统级 Task，而不是 Todo。

---

## 9. 建议数据模型

### 9.1 Todo

```ts
type Todo = {
  todoID: string;              // 短 ID，建议约 3 位
  sessionID: string;           // 所属 session
  orderIndex: number;          // 创建顺序
  status: TodoStatus;
  description: string;
  checklist?: string[];
  tags?: string[];
  skills: string[];            // 从已安装 skills 中选择，最多 3 个
  creationContext?: string;    // 创建该 Todo 的上下文 / 原因
  summary?: string;            // 简短结果摘要
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
  summary?: string;
  source?: "delegated" | "system" | "external";
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
  tags?: string[];
  skills?: string[];
  updatedAt?: string;
};
```

---

## 10. 典型流程

### 10.1 创建多个 Todo 并顺序执行

```text
进入 plan 模式
  -> createTodo(A)
  -> createTodo(B)
  -> createTodo(C)
  -> TodoList 显示 A/B/C
  -> current todo = A
退出 plan 模式，进入执行模式
  -> Worker 执行 A
  -> updateState(A, completed, summary, report)
回到 plan 模式
  -> 自动展示 TodoList
  -> current todo = B
继续执行 B
```

### 10.2 等待 delegated task

```text
plan 模式中看到 delegated task T
  -> T 未完成，被置顶展示
  -> 自动查询 T 当前状态
  -> 如果后续 Todo 依赖 T：
       - 等待；或
       - 设置稍后检查计划；或
       - 若长时间无反馈，updateState(T, timeout)
       - 决定是否 replan
```

### 10.3 查看某个 Todo 的内部记录

```text
Todo A 已完成
主干只看到：状态 + summary + report
如需查看内部执行过程：
  -> getDetail(A)
  -> 返回 Todo 工作目录说明
  -> 根据 manifest 继续查看文件和细节
```

---

## 11. 验收标准

1. `createTodo` 能创建 session 级 Todo，返回短 `todoID`。
2. Todo 在 session 删除后不再可用。
3. `createTodo` 必须支持详细描述、tags、skills，且 skills 最多 3 个。
4. skills 必须从已安装 skills 中选择。
5. plan 模式可以连续创建多个 Todo，Todo 按创建顺序执行。
6. `current todo` 始终是第一个未完成 Todo。
7. 执行模式只能看到当前 Todo 及必要创建上下文。
8. Todo 完成后，主干可看到状态、summary 和最终 report。
9. `todoList` 能展示当前 session 的 Todo 状态，并标记 current todo。
10. `todoList` 能混入展示 delegated task，并自动查询未完成 delegated task 状态。
11. 详情接口能通过 TodoID / TaskID 获取详细信息或工作目录说明。
12. 状态更新接口能设置成功、失败、超时等状态，并记录 summary / reason / report。
13. 系统不提供 modify/remove 接口。
14. delegated task 已经发出后不能通过删除来抹掉，只能通过状态表达结果。

---

## 12. 待确认事项

以下内容语音中已有方向，但需要最终定稿：

1. 统一详情接口最终命名：`getDetail`、`getTodoDetail`，还是兼容 `getTaskDetail`。
2. 统一状态更新接口最终命名：`updateState`，还是保留 `updateTodoState` / `updateTaskState` 两个外壳。
3. `todoID` 的具体长度：语音中倾向非常短，约 3 位；需确认是否固定长度。
4. Todo 工作目录结构的具体文件命名与 manifest 格式。
5. `blocked` 是否作为正式状态，还是只保留 `pending/running/completed/failed/timeout`。
6. “设置稍后检查计划”的能力是否属于 Todo CLI，还是由另一个 scheduler / task 工具负责。
