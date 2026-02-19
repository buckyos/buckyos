
## 0. 文档目标与范围

### 0.1 文档目标

* 把 Agent Workspace 从“目标描述”落到“**可设计、可落地、可交互**”的完整页面与组件规范。
* 让设计师/产品/研发能基于同一套信息架构与交互逻辑，**在 Figma 中一次性构建所有页面**（含空态、加载态、异常态、实时态）。

### 0.2 不做什么（非目标）

* 不把它做成传统 To-do 软件（不强调人工安排任务）。
* 不定义 Agent 本体的创建/训练/发布流程（除非你们系统已有入口，可后接）。
* 不定义具体后端链路、埋点实现细节（但会给出前端需要的字段与状态机）。

---

## 1. 核心概念与可视化对象映射

### 1.1 术语表（UI 展示口径）

* **Agent**：一个可运行实体（主 Agent 或 Sub-Agent）。
* **Workspace**：以 Agent 为单位的“观察与执行过程可视化”工作台。
* **Loop Run（一次 Loop）**：一次被事件触发的执行会话，有起止时间与状态。
* **Step**：Loop 内的阶段节点（串行推进的基本单位）。
* **Behavior**：Step 内更高层级行为过程（可选展示层）。
* **Task（LLM 推理）**：每次 LLM 推理产生一个 Task（1～6 次/Step）。
* **WorkLog**：全过程日志事件（Message/Reply、Function Call、Action、Sub-Agent Lifecycle/Comms）。
* **Todo List**：每个 Agent（含 Sub-Agent）独立待办推进记录（由 Loop 推进生成与完成）。

### 1.2 状态集合（必须可视化）

* Task 管理（关联 Step / Behavior）
* WorkLog 管理（按类型/时间/关联对象过滤与钻取）
* Todo List（按 Agent 展示、可追溯创建/完成发生在何处）
* Sub-Agent 并行展开（创建/销毁/休眠/激活 + 通信）

---

## 2. 体验原则（设计与交互约束）

1. **以 Agent 为中心**：任何信息都能回答“这个 Agent 在做什么/做到哪一步/做过什么/并行做了什么”。
2. **结构优先于堆列表**：先给 Loop/Step 的结构，再逐层展开 Task/WorkLog/Todo 细节。
3. **可追溯**：所有 Task / WorkLog / Todo 必须能回溯到：
   * 哪个 Session
   * 哪个 Agent（主/子）
   * 哪个 Loop Run
   * 哪个 Step（如果适用）
   * 发生时间、耗时、结果状态
4. **渐进披露**：默认视图给摘要；点击节点打开 Inspector/Drawer 看原始输入输出。
5. **实时不打扰**：实时流式更新要可见但不“乱跳”，提供“自动滚动/暂停更新”。
6. **并行可理解**：Sub-Agent 的并行不是“另一堆日志”，而是**可聚合、可切换、可关联主 Agent 的通信链路**。

---

## 3. 信息架构与页面清单（Figma 必建）

### 3.1 全局应用结构（建议）

* **A1. Agent 列表 / 入口页（Home）**
* **A2. Agent Workspace（核心）**

  * Tab1：Overview（Loop/Step 结构 + 当前进度）
  * Tab2：WorkLog（全量日志检索/过滤/时间线）
  * Tab3：Tasks（LLM 推理任务）
  * Tab4：Todos（待办推进）
  * Tab5：Sub-Agents（并行展开 + 通信）
* **A3. 全局搜索（可选）**：跨 Agent/Run 搜索 WorkLog/Task（如果系统需要）
* **A4. 设置（可选）**：显示偏好、字段脱敏、自动刷新策略

> 如果你们只做 Workspace 单页，也至少要在 Workspace 内提供“Agent 切换器 + Run 切换器”。

---

## 4. 通用布局与交互框架（App Shell）

### 4.1 布局建议（Desktop 1440）

* 顶部 Top Bar（56px）：

  * 左：产品名 Agent Workspace / 面包屑
  * 中：全局搜索（可选）
  * 右：时间范围、自动刷新、用户菜单
* 左侧 Sidebar（260–300px）：

  * Agent 列表（可分组：Main Agents / Sub-Agents / Favorites）
  * 每个 Agent 行：状态点 + 名称 + 当前 Run 状态
* 主内容区（自适应）：

  * Workspace Header：Agent 名、状态、当前 Run、触发事件、开始时间、耗时
  * Tabs：Overview / WorkLog / Tasks / Todos / Sub-Agents
* 右侧 Inspector（360–420px，可折叠）：

  * 用于 Step/Task/Log/Todo/Sub-Agent 的详情钻取

### 4.2 通用交互

* 任意列表行点击：打开右侧 Inspector（不跳页）
* 任意对象（Step/Task/Log/Todo）都有：

  * “复制 ID”
  * “在 WorkLog 中定位”
  * “查看原始输入/输出（JSON）”
* 筛选器：

  * 类型（Message/Reply/Function/Action/Sub-Agent）
  * 状态（running/success/failed/partial/cancelled）
  * Agent（主/子）
  * Step（Step #）
  * 时间范围（绝对/相对）
* 实时更新：

  * 顶部开关：Auto-refresh ON/OFF
  * WorkLog 列表顶部提示：`Paused` / `Live`，新消息计数“+12 new”

---

## 5. 数据字段与 UI 需求（给研发/设计对齐）

### 5.1 Agent

* agent_id, agent_name
* agent_type: main | sub
* status: idle | running | sleeping | error | offline
* parent_agent_id（sub 才有）
* current_run_id（可空）
* last_active_at

### 5.2 Loop Run

* run_id
* trigger_event（唤醒事件等）
* status: running | success | failed | cancelled
* started_at, ended_at, duration
* current_step_index
* summary: step_count, task_count, log_count, todo_count, sub_agent_count

### 5.3 Step

* step_id, step_index, title（可选）
* status: running | success | failed | skipped
* started_at, ended_at, duration
* task_count（1～6+）
* log_count（按类型聚合：msg/function/action/sub-agent）
* output_snapshot（下一 Step 环境摘要，可选）

### 5.4 Task（LLM 推理）

* task_id
* step_id, behavior_id（可空）
* status: queued | running | success | failed
* model, tokens_in/out（可选）
* prompt_preview（摘要）
* result_preview（摘要）
* raw_input, raw_output（详情可折叠）
* created_at, duration

### 5.5 WorkLog（统一事件模型）

* log_id
* type: message_sent | message_reply | function_call | action | sub_agent_created | sub_agent_sleep | sub_agent_wake | sub_agent_destroyed | …
* agent_id（谁发起/谁发生）
* related_agent_id（通信对象，可空）
* step_id（可空）
* status: info | success | failed | partial
* timestamp, duration（可空）
* summary（单行摘要）
* payload（详情 JSON）

### 5.6 Todo

* todo_id
* agent_id
* title, description（可选）
* status: open | done
* created_at, completed_at
* created_in_step_id / completed_in_step_id（可选）

---

## 6. 页面级交互规格 + Figma 提示词

下面每个页面包含：**目的 / 布局 / 关键组件 / 交互 / 状态 / Figma Prompt**

---

# A1. Agent 列表 / 入口页（Home）

### 目的

* 让用户快速找到某个 Agent，并进入其 Workspace
* 支持按状态筛选：Running / Sleeping / Error / Idle
* 支持收藏与最近访问（可选）

### 布局

* 左：Agent 列表（也可复用全局 Sidebar）
* 主区：卡片式 Agent 总览（状态、当前 Run、最后活跃时间、最近 Step 摘要）

### 关键交互

* 点击 Agent 卡片或列表行 → 进入 A2 Workspace（默认打开 Overview Tab）
* 右键/更多：复制 agent_id、打开新标签页、收藏（可选）

### 状态

* 空态：没有 Agent（提示接入/配置）
* 错误态：加载失败（重试）

### Figma 生成提示词（可复制）

```text
Design a desktop web console home page for “Agent Workspace” (1440x900). Use a clean light theme, 12-column grid. Top bar with product name, global search, time range selector, auto-refresh toggle, user menu. Left sidebar 280px showing agent list with status dot (running/idle/sleeping/error), agent name, and small badge for current run status. Main area shows a header “Agents” with filters (Status dropdown, search input, sort by last active). Below, display a responsive grid of agent cards (3 columns). Each card includes agent name, type (Main/Sub), current run status, trigger event, current step progress indicator, counts (steps/tasks/logs/todos/sub-agents), and last active time. Include empty state and loading skeleton variants.
```

---

# A2. Agent Workspace（核心骨架）

## A2-0 Workspace 通用头部（所有 Tab 共用）

### 头部信息区（Workspace Header）

* Agent 名称 + 状态（running/sleeping/error）
* Agent 类型：Main / Sub（若 Sub 显示 Parent）
* 当前 Run 选择器（下拉：当前运行 + 历史 runs）
* Trigger Event（唤醒事件）
* Started At / Duration
* 快捷操作：

  * Copy IDs
  * 跳转 WorkLog（定位当前 Step）
  * Pause/Resume live（只影响 UI）

### Figma Prompt（Workspace Header 组件）

```text
Create a reusable “Workspace Header” component for an agent observability console. Include: agent name with status pill, agent type tag (Main/Sub) and parent reference if Sub, a Run selector dropdown (show run id and start time), trigger event label, started time, duration timer, summary KPI chips (Steps, Tasks, WorkLogs, Todos, Sub-Agents). Add action buttons: Copy IDs, Locate in Logs, Live toggle (Live/Paused). Provide variants for running/success/failed/sleeping.
```

---

## A2-1 Overview Tab（默认）

### 目的

* 一屏回答：
  **现在进行到哪个 Step、每个 Step 做了什么、并行 Sub-Agent 怎么展开、有哪些关键事件与待办变化**

### 页面布局（推荐三段式）

* 上：Loop Run 概览条（Run 状态、进度、关键 KPI）
* 中：**Step Timeline / Flow（核心可视化）**
* 下：摘要面板（三列）

  1. 当前 Step 摘要（Tasks/WorkLog/Actions）
  2. Todo 变化（最近新增/完成）
  3. Sub-Agent 活跃概览（谁在跑、最新消息）

### Step Timeline 组件（核心）

* 纵向时间线或横向流程图（建议纵向更适合长步骤）
* 每个 Step 节点显示：

  * Step # + 状态
  * 起止时间/耗时
  * 本 Step 的计数徽标：Tasks、Messages、Function Calls、Actions、Sub-Agent Events
* 当前 Step 高亮（强调“做到哪一步”）
* 支持展开 Step（accordion）显示：

  * Behaviors（可选）
  * 本 Step 产生的 Tasks 列表（摘要）
  * 本 Step 关键 WorkLog（只取重要类型/错误优先）
  * Step 结束生成的 Action 批次执行结果（支持 partial）

### 并行 Sub-Agent 展示（Overview 内）

* 在某个 Step 节点旁显示“分叉”标识
* 点击分叉 → 右侧 Inspector 打开 Sub-Agent 列表与通信摘要
* Sub-Agent 卡片显示：

  * 状态、当前 todo、最后日志时间
  * 与主 Agent 的最后一条 message 摘要
  * “进入该 Sub-Agent Workspace”

### 关键交互

* 点击 Step 节点 → 右侧 Inspector 显示 Step Detail（不离开 Overview）
* Hover Step → tooltip：耗时、错误数、并行数
* 点击某个计数徽标（如 Messages 5）→ 跳转到 WorkLog Tab，并自动带过滤器（step_id + type）
* 点击 Todo 变化项 → 打开 Todo detail（右侧）
* 点击 Sub-Agent 卡片 → 切换 Workspace 到该 Sub-Agent（或新标签页）

### 状态

* Running：持续更新当前 Step 状态、末尾实时追加
* Finished：显示 Run 总结（成功/失败原因）
* Empty：Run 无 Step（异常/数据缺失）
* Partial Failure：Actions 部分失败高亮显示

### Figma 生成提示词（Overview 页面）

```text
Design the “Agent Workspace – Overview” tab (desktop 1440x900) for an agent execution observability tool. Use an app shell with left agent sidebar, top bar, and a collapsible right inspector panel. In the main content: show the Workspace Header at top, then a tab bar (Overview, WorkLog, Tasks, Todos, Sub-Agents). Overview tab content: 
1) A Run summary strip with status (Running/Success/Failed), progress (current step / total steps), trigger event, started time, duration, and KPI chips. 
2) A central Step Timeline (vertical) with step cards: each card shows Step number, title, status icon, duration, and small count badges for Tasks, Messages, Function Calls, Actions, Sub-Agent events. Highlight the current step. Allow expanded state showing: key tasks list (LLM inferences), key logs (errors first), and action batch results (with partial failure). 
3) Bottom section with three panels: Current Step Summary, Recent Todo Changes (added/completed), Active Sub-Agents list (status, last message preview, open workspace button). 
Include variants: running live updates indicator, finished run summary, empty state, error state. Add interaction annotations for clicking badges to deep-link to WorkLog with filters and clicking a step to open the right inspector details.
```

---

## A2-2 WorkLog Tab（日志检索与钻取）

### 目的

* 支持用户回答：**发生过什么？何时发生？谁触发？结果如何？上下文是什么？**
* 适用于排障/审计：Message 异步、Function Call 同步依赖、Action 批量执行、Sub-Agent 生命周期。

### 布局

* 顶部：筛选器工具条（Sticky）

  * 时间范围、Agent 选择（主/子）、Step 选择、类型多选、状态多选、关键词搜索
* 左侧（可选）：Facet 面板（类型计数、状态计数）
* 主区：WorkLog 时间线列表（支持分组）

  * 分组方式切换：按 Step 分组 / 按时间分组 / 按 Agent 分组
* 右侧 Inspector：Log Detail（根据类型展示不同模板）

### WorkLog 列表行规范

* 左：类型图标（Message/Reply/Function/Action/SubAgent）
* 中：一行摘要（summary）
* 右：timestamp、duration、status tag（success/failed/partial）、关联 Step #、关联 Agent
* 错误行：红色强调 + “查看错误堆栈/输出”

### Log Detail（按类型模板）

1. **Message Sent / Reply**

* 展示 Thread（对话气泡或双栏）
* 字段：from/to、correlation id、payload、发送时间、回复时间、超时/未回复状态
* CTA：在 Sub-Agent/外部 Agent 中定位该线程

2. **Function Call**

* 展示：函数名、输入参数（格式化 JSON）、输出结果、耗时、错误
* CTA：复制 cURL/复制输入输出（可选）

3. **Action Batch**

* 展示 Action 组（串行/并行标识）
* 每个 Action 子项：name、status、耗时、错误（允许部分失败）
* 汇总：成功数/失败数/跳过数

4. **Sub-Agent Lifecycle / Comms**

* 展示生命周期时间线：created → active/sleep → destroyed
* 通信摘要：最近消息、未读/未回复

### 关键交互

* 点击任一 Log 行 → 右侧打开详情
* 点击“定位到 Step” → 跳转 Overview 并定位 Step
* 支持“Pin 重要日志”到顶部（可选）
* 实时模式：顶部出现“新日志 +N”按钮，点击滚动到底部

### 状态

* Loading skeleton
* Empty result（给出建议过滤条件）
* Error（重试）

### Figma 生成提示词（WorkLog 页面）

```text
Design the “Agent Workspace – WorkLog” tab (desktop 1440x900) for an agent runtime log explorer. Include a sticky filter toolbar with: time range picker, agent selector (main + sub agents), step selector, type multi-select (Message Sent, Message Reply, Function Call, Action, Sub-Agent Lifecycle), status multi-select, and a search input. Main area shows a chronological log list with optional grouping toggles (Group by Step / Time / Agent). Each log row has: type icon, summary text, timestamp, duration, status pill (success/failed/partial/info), related Step badge, agent badge. Failed items are visually emphasized. Right side is a collapsible Inspector that shows log details with templates: 
- Message thread view (bubbles, from/to, correlation id, payload JSON collapsible, reply status). 
- Function call detail (function name, inputs/outputs formatted JSON, duration, error). 
- Action batch detail (parallel/serial indicator, action items list with partial failures and rollup counts). 
- Sub-agent lifecycle timeline (created/active/sleep/destroyed) and comms summary. 
Include empty state, loading skeleton, and “Live updates paused/new logs” banner variants.
```

---

## A2-3 Tasks Tab（LLM 推理任务）

### 目的

* 以“每次 LLM 推理”为粒度可追溯：在哪个 Step、属于哪个 Behavior、输入输出是什么、耗时与失败原因。

### 布局

* 顶部：筛选器（Step、Status、Model、关键词）
* 主区：Task 表格/列表
* 右侧 Inspector：Task 详情（Prompt/Response/工具调用摘要等）

### Task 列表字段（建议）

* Task ID（短）
* Step #
* Behavior（可空）
* Status
* Model
* Tokens（可选）
* Duration
* Created at
* Prompt preview / Result preview

### Task 详情结构

* 顶部：基本信息（状态、耗时、Step 链接）
* 中：Prompt（折叠，支持复制）
* 中：Response（折叠，支持复制）
* 下：关联 WorkLog 快捷入口（例如该 Task 触发的 function call/action/message）

### Figma 生成提示词（Tasks 页面）

```text
Design the “Agent Workspace – Tasks” tab (desktop 1440x900) for viewing LLM inference tasks. Top includes filter bar: Step dropdown, Status dropdown (queued/running/success/failed), Model dropdown, and search. Main content uses a table with columns: Task ID, Step #, Behavior, Status, Model, Tokens in/out, Duration, Created time, Prompt preview, Result preview. Clicking a row opens the right Inspector showing Task details: header with status and metadata, collapsible sections for Prompt (formatted text/JSON), Response, raw input/output JSON, and a panel of related WorkLogs with deep links (open in WorkLog tab filtered). Include loading, empty, and failed task variants.
```

---

## A2-4 Todos Tab（待办推进）

### 目的

* 让用户看到每个 Agent（含 Sub-Agent）的 Todo 变化：何时创建、何时完成、由哪个 Step 推进完成。

### 交互前提（建议）

* 默认 Todo **只读**（由 Agent Loop 推进写入），支持：

  * 查看详情
  * 筛选 open/done
  * 按 Agent 分组
  * 定位到创建/完成对应 Step 与 WorkLog

### 布局

* 左：Agent 分组切换（主/子）
* 主：Todo 列表（可按状态分段）
* 右：Todo Detail

### Todo 列表行

* 标题
* 状态 open/done
* created_at / completed_at
* created_in_step / completed_in_step 徽标
* 关联 Agent 徽标

### Figma 生成提示词（Todos 页面）

```text
Design the “Agent Workspace – Todos” tab (desktop 1440x900) for showing agent-driven todo progression. Layout: left sub-panel listing agents (main + sub) with counts of open/done todos. Main area shows todo list with segmented tabs (Open / Done / All). Each todo item row includes title, status checkbox indicator (read-only), created time, completed time (if done), badges for created-in-step and completed-in-step, and agent badge. Clicking an item opens the right Inspector with details: description, timeline (created -> completed), links to related Step and WorkLogs, and copy id action. Include empty states (no todos, no done todos) and loading skeleton.
```

---

## A2-5 Sub-Agents Tab（并行展开与通信）

### 目的

* 让用户理解并行：有哪些 Sub-Agent、它们在做什么、与主 Agent 的交互如何发生。

### 布局（推荐双视图切换）

* 视图切换：Graph / List

1. **Graph 视图（默认）**

   * 主 Agent 节点居中
   * Sub-Agent 节点环绕或树状展开
   * 边表示消息通信（可点击边查看通信摘要）
2. **List 视图**

   * 表格展示 Sub-Agent 状态、当前 Step、当前 Todo、最后活动、错误数

### Sub-Agent 节点/卡片信息

* 名称/ID（短）
* 状态：active/sleeping/error/destroyed
* 当前 Run（如有）
* 当前 Step（如有）
* Todo open 数
* 最近一条 message 摘要

### 关键交互

* 点击 Sub-Agent → 右侧 Inspector 展示：

  * 生命周期时间线
  * Todo 概览
  * 最近通信（消息线程）
  * “进入该 Sub-Agent Workspace”
* 点击消息边 → 打开消息 thread（WorkLog Detail 模板）

### Figma 生成提示词（Sub-Agents 页面）

```text
Design the “Agent Workspace – Sub-Agents” tab (desktop 1440x900) focusing on parallel execution visibility. Provide a toggle between Graph view and List view. Graph view: show a central node for the Main Agent and surrounding nodes for Sub-Agents, with connecting lines representing asynchronous messages. Nodes display status (active/sleeping/error/destroyed), last active time, and a small badge for open todos. Clicking a node opens the right Inspector with: lifecycle timeline (created/wake/sleep/destroy), current run/step summary, recent todo list, and recent communication threads. Clicking a connection line opens a message thread detail. List view: table with columns Sub-Agent name/id, status, current run, current step, open todos, last active, error count, and open workspace action. Include empty state (no sub-agents) and mixed states.
```

---

# A2-6 右侧 Inspector（通用详情面板）规范

### 目的

* 统一承载 Step/Task/Log/Todo/Sub-Agent 的详情查看（避免频繁跳页）

### 交互规范

* 右侧抽屉固定宽度，可折叠
* 顶部：对象标题 + 类型 tag + 状态 pill + copy id + close
* 内容区：分段（Accordion）
* 底部：相关跳转（Locate in Logs / Open related Step / Switch to agent workspace）

### Figma Prompt（Inspector 组件）

```text
Create a reusable right-side Inspector drawer component (width 400px) for an observability console. It supports multiple content templates: Step Detail, Task Detail, WorkLog Detail, Todo Detail, Sub-Agent Detail. Common header includes: title, type tag, status pill, copy id button, open-in-new-tab icon, and close button. Content uses accordions with sections for Summary, Timeline, Related items, Raw JSON (collapsible with code styling). Provide empty state and loading state variants.
```

---

## 7. 关键对象的“详情页内容”规范（用于 Inspector 模板）

### 7.1 Step Detail（Inspector 内）

* Summary：状态、耗时、开始/结束、产出计数
* Tasks：该 Step 产生的 Task 列表（可点击进一步看 Task detail）
* WorkLog Highlights：错误优先、关键事件（Message/Function/Action/Sub-agent）
* Actions Result：action batch 展开（串行/并行、partial 标识）
* Environment Snapshot（可选）：下一 Step 环境摘要（折叠）

### 7.2 Message Thread Detail

* Thread header：from/to、correlation id、状态（replied/pending/timeout）
* 消息体：按时间气泡
* 原始 payload（折叠 JSON）

### 7.3 Function Call Detail

* function name、inputs、outputs、duration、error
* retry 次数（可选）

### 7.4 Action Batch Detail

* batch summary：并行/串行、总数、成功/失败/跳过
* action item rows：name、status、duration、error（可展开详情）

### 7.5 Sub-Agent Detail

* lifecycle timeline
* current status & current run/step
* todo overview
* communication threads（最近 N 条）

---

## 8. 全局状态与细节交互（必须在 Figma 里画出来）

### 8.1 Loading Skeleton

* Sidebar agent list skeleton
* Overview step list skeleton
* WorkLog table skeleton
* Inspector skeleton

### 8.2 Empty State

* 无 run：提示“该 Agent 尚未触发 Loop”
* 无 logs：提示调整筛选
* 无 sub-agents：提示“该 run 未创建并行分支”
* 无 todos：提示“该 Agent 未产出待办”

### 8.3 Error State

* 数据加载失败（网络/权限/服务错误）
* WorkLog payload 解析失败（展示 raw + 错误提示）

### 8.4 Live 模式交互细则

* 默认开启 Live（如果 run 正在 running）
* 当用户滚动离开底部：自动暂停自动滚动，但仍可接收新日志计数
* 提供按钮：“Jump to latest（+N）”
* 提供按钮：“Pause updates”（冻结列表，直到恢复）

### Figma Prompt（状态页/组件集合）

```text
Create a page in Figma named “States” for the Agent Workspace console. Include reusable variants for: loading skeletons (sidebar, timeline, table, inspector), empty states (no runs, no logs, no sub-agents, no todos), error states (failed fetch with retry, invalid JSON payload), and live mode banners (Live, Paused, +N new items, jump to latest). Ensure consistent spacing, typography, and component reuse.
```

---

## 9. 组件库（Design System + 业务组件）清单

### 9.1 基础设计系统（建议做成 Figma Library）

* Color tokens（Light theme）

  * Background / Surface / Border / Text
  * Status colors：running / success / failed / warning / sleeping
* Typography

  * H1/H2/H3、Body、Mono（用于 JSON）
* Spacing（8px 系统）
* Icon set（Message/Function/Action/SubAgent/Task/Todo）

### 9.2 业务组件（必须组件化）

* Agent List Item（含状态点、badge）
* Run Selector（dropdown）
* Step Card（collapsed/expanded）
* Count Badge（可点击跳转过滤）
* WorkLog Row（按类型变体）
* Status Pill（统一状态显示）
* Filter Bar（chips + dropdown）
* Inspector Drawer（多模板）
* Message Thread Viewer
* JSON Viewer（折叠/复制/搜索，可选）
* Action Batch Table（支持 partial）
* Sub-Agent Graph Node（含状态与摘要）

### Figma Prompt（组件库页）

```text
Design a Figma component library for “Agent Workspace”. Include foundational styles (color tokens, typography scale, spacing), and components with variants: Status pill (running/success/failed/partial/sleeping), Agent list item (main/sub, selected/unselected), Run selector dropdown, Step card (collapsed/expanded, current highlight, error state), Count badge (clickable), WorkLog row variants (message/reply/function/action/sub-agent), Filter bar (chips, dropdowns), Right inspector drawer (templates), Message thread viewer, JSON viewer block, Action batch table (partial failures), Sub-agent graph node. Use auto layout and component variants extensively.
```

---

## 10. 原型连线（Figma Prototype 交互说明）

在 Figma 里建议这样连：

* Home → 点击 Agent 卡片 → Workspace Overview
* Overview：

  * 点击 Step → 右侧打开 Step detail
  * 点击 Messages badge → 跳 WorkLog Tab 并预置过滤（Step + Type）
  * 点击 Sub-Agent 卡片 → 切换到 Sub-Agents Tab 并选中该 Sub-Agent
* WorkLog：

  * 点击某行 → 右侧 log detail
  * 在 log detail 点击 “Locate Step” → 回 Overview 并定位 Step
* Tasks/Todos：

  * 点击行 → 右侧 detail
  * detail 内 “Open related logs” → 跳 WorkLog + filter

---

## 11. 你可以直接用的「一键总 Prompt」（用于让 Figma AI 先生成整套框架）

> 如果你希望 Figma AI 先“生成一个可编辑的全套初稿”，可以先用下面这个总 Prompt，然后再逐页用上面更细的 Prompt 迭代。

```text
Generate a complete desktop web app design (1440x900) for an “Agent Workspace” observability console. The product visualizes an Agent Loop execution with Steps, LLM inference Tasks, WorkLogs (Message/Reply, Function Call, Action batches, Sub-Agent lifecycle & communication), and per-agent Todo progression. 
Include: a top bar (product name, global search, time range, live toggle), left sidebar (agent list with statuses), main workspace header (agent info + run selector + KPI chips), and tabs (Overview, WorkLog, Tasks, Todos, Sub-Agents). 
Overview tab: step timeline with expandable step cards and summary panels for current step, todo changes, active sub-agents. 
WorkLog tab: filter toolbar + log list + right inspector with different detail templates for message threads, function calls, action batches, and sub-agent lifecycle. 
Tasks tab: task table + inspector showing prompt/response and raw JSON. 
Todos tab: per-agent todo lists + inspector. 
Sub-Agents tab: graph + list toggle and inspector with lifecycle timeline and recent communications. 
Also include a “States” page with loading skeletons, empty states, error states, and live update banners. Use a clean light theme, clear status colors, and reusable components with variants and auto layout.
```

---

## 12. 交付物清单（你后续在 Figma 里应当有哪些 Page）

建议 Figma 文件结构：

1. `00 - Cover & Notes`
2. `01 - Foundations`（颜色、字体、间距、icon）
3. `02 - Components`（组件库）
4. `03 - Home`
5. `04 - Workspace - Overview`
6. `05 - Workspace - WorkLog`
7. `06 - Workspace - Tasks`
8. `07 - Workspace - Todos`
9. `08 - Workspace - Sub-Agents`
10. `09 - States`
11. `10 - Prototype Links`
