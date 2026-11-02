# MessageHub 运行视图改为以 Work Session 为主 TODO

日期：2026-11-02

状态：**已实施（2026-11-02，未提交），真实 Zone DV 见 §8.4**。裁决完毕后开工。用户 2026-11-02 口述思路（§1），Claude 对照 llm_context 冻结、简单 Work Session 全异步化（第一刀）、plan_simple 多 Turn（第二刀）之后的源码写成本单。同日用户 review：先定了 Q2（实时一句话写进 session state，function call 带 `intent` 时优先用它，§3.10），随后在 §4 写回 Q1–Q13 的裁决；Q3 / Q5 / Q6 / Q8 的问题 Claude 已核对源码答复，并回写 §3 / §5。

范围：MessageHub 原生 Agent 会话里，Agent 回复气泡下方的附属 UI，也就是今天的 `MessageTaskArea`（两行状态 + 可展开的 TaskMgr 树）。后端只补这件事需要的只读数据：libopendan 的卡片投影、opendan 的一个只读 RPC、work session 的标题与结束时间。不改：意图分析流程本身、Q/A 列表、消息详情里的 task 页、TaskMgr 取消 → stop 的桥、Telegram 等 tunnel、Goal。

路径均相对 `buckyos/`。实施前重读 `AGENTS.md` 与当前源码（工作区有第一刀 / 第二刀的未提交改动），不得覆盖。顺序：A（libopendan / opendan / xagent）→ B（Jarvis 提示词）→ C（Desktop）→ D（文档）。

## 0. 术语

| 词 | 含义 |
| --- | --- |
| UI Turn | UI session（chat_route）的一个 Turn。它的 TaskMgr task（`opendan.agent_turn/v1`）就是回复消息 `agent_task` 指向的 task |
| work session | `create-worksession` 建出的子 session：simple（behavior `work_simple`）或 plan（class `plan_simple`）。多 Turn，`report(is_end=true)` 结束 |
| Todo session | plan 的 Todo attempt 绑定的子 session（`TodoAttempt.child_sid`，behavior `work_simple` / `explore`） |
| 卡片 | 本单的“2 行条目卡”（§3.2） |
| 卡片树 | 一条气泡下的全部卡片：work session 卡，plan 下面再挂 TodoList（Todo 卡或 Todo 标题行） |

## 1. 用户思路（2026-11-02 原文要点 → 本单条目）

1. 从基于 task 改成基于 work session。→ §3.1
2. 用户关注的 work session 属性：标题；详细内容（work session 系统提示词的关键，即 objective）；当前状态（枚举，可以换成图标）+ 一句话；创建时间；token 消耗（尤其 output）；todolist（plan session 才有）。→ §3.2、§3.3、§3.4、§3.8
3. try_analyze_user_intent 不变。→ §3.1
4. try_analyze_user_intent 创建 work session 后（从 task_id 到 task data 能读到），消息的附属 UI 以 work session 为主。→ §3.1、§3.5
5. work session 显示 2 行条目卡；有 todolist 就显示，每个 Todo 一张 2 行条目卡；没有绑定 work session 的 Todo 只显示标题；todolist 可以收起。→ §3.4
6. 2 行条目卡：`状态图标 | 标题（有最长限制）| 运行时间 | 输入 token 数 输出 token 数`；第 2 行 `一句话实时状态（有最长限制）| 模型名`，模型名显示 `gpt-6.1-sol` 这种，不显示 `llm.code`。→ §3.2
7. （review 补充）实时一句话要更新进 session state（现有结构里已经有这个字段）。加一个特例：LLM 返回的 function call 带这个字段（`intent`）时优先用它，否则按老逻辑更新。→ §3.10

## 2. 现状（2026-11-02 源码核对；实施前复核）

| 主题 | 事实 | 位置 |
| --- | --- | --- |
| 气泡附属 UI | `renderers.tsx:558` 渲染 `MessageTaskArea`：`agent_task` → `useTaskSummary` → `taskAreaView`。第 1 行是状态 + task.message，第 2 行是子任务计数 / 等待原因 / 更新时间；展开后是 TaskMgr 树 `TaskTree`。成功且没有子任务的快速回复不显示 | `desktop/src/app/messagehub/conversation/tasks/{MessageTask.tsx,taskView.ts,index.ts}` |
| Turn 就是 task | 每个 Turn 一个 TaskMgr task。`TurnTasks::open` 的 input 是 `{agent_did, session_id, session_kind, turn, inputs}`，name 是 Turn 标题（缺省为 objective 截 120 字符）。子 session 的每个 Turn task 都挂在它的 `origin.parent_task` 下 | `frame/opendan/src/tasks.rs:330-360`、`lib_opendan/src/runner/turn_task.rs:83-150` |
| work session 与 UI Turn | analyze_intent 是 UI session 的 fork（同一个 session）。`create-worksession` 建子 session 时写 `origin.parent_session = UI sid`、`origin.parent_task = UI 当前开着的 Turn 的 task`；plan 派发 Todo 时同理挂在 plan 当时开着的 Turn task 下 | `lib_opendan/src/api.rs:946-965` |
| 多 Turn 之后 TaskMgr 树的形状 | 第一刀、第二刀之后 work 与 plan 都是多 Turn。一个 work session 有 N 个 Turn task，全部是 UI Turn task 的直接子节点；Todo session 的 Turn task 散在 plan 的各个 Turn task 下。气泡展开的树按 Turn 列，同一个 session 出现 N 次，名字都是 objective，看不出“哪个工作、做到哪一步”。**这是本单要解决的问题** | 同上 |
| Turn task 的实时一句话 | `status_of` 用内存里的当前工具 + intent 生成（`running shell: <intent>` / `waiting for 2 task(s)` / `waiting for input` / `paused: …`），每次变化写进 TaskMgr message。**不落盘**，只在 driver 进程的 `Shared.current_tool / current_intent` 里 | `turn_task.rs:152-206` |
| intent 的取得 | 工具派发时 `current_intent = agent_tool::intent::intent_of(&call)`：标准工具（`read` / `write_file` / `edit_file` / `shell`）可选带 `intent`，`call_intent = true` 的 behavior 每个调用必带 | `runner/tools.rs:224-227`、`agent_tool/src/intent.rs` |
| 会话状态（落盘） | `SessionState.one_line_status` 随 `report()` 进登记表 `SessionStatus{run_state, outcome, one_line_status, report_brief(≤500), needs_input, turn_open, paused, activity, updated_at_ms}`。`one_line_status` 今天只在 Turn 关闭、会话结束、工作区失效、Goal 结束时写（`one_line()`，带英文前缀：`waiting for input: <回复首行>`、`finished: …`），**Turn 进行中不更新**。读它的有：父 session 的子 session 事件 / 摘要、Goal 的 works 列表、session 列表文本、Todo 摘要缺省值 | `protocol/agent_state.rs:24-61`、`runner/outcome.rs:946,1246,1573`、`runner/drive.rs:302,882,912`；读者 `runner/children.rs:136,164,313,394-418`、`runner/goal.rs:322,509-527`、`runner/inputs.rs:222`、`todos.rs:330` |
| `activity.summary` | 另一个字段：session 声明“在做什么”（`ControlCommand::Activity`），Turn 打开时为空则填 objective，结束清空；其他 session 的背景行（`work <sid>: running — <summary>`）读它。心跳（`hook.rs` `heartbeat`）在 touching 变化或到期时提交 state 并 report | `runner/live.rs:1502`、`runner/inputs.rs:532-548`、`runner/background.rs:155-200`、`runner/hook.rs:115-133` |
| token | `static.json`（`SessionStatic`）的 input / output / total 只在 `after_run_end` 里累加。多 Turn Work 整个会话是同一个 run（第一刀 Q12），所以**运行中恒为 0**。`usage.jsonl`（`UsageRecord{at_ms, model, run_id, input/output/total}`）在每个成功 Round 后追加，是唯一的实时来源 | `runner/outcome.rs:1505`、`runner/rounds.rs:76-104`、`protocol/misc.rs:60-115` |
| 模型名 | `UsageRecord.model` 是 provider 报告实际服务的模型，没有时才用请求别名。AICC 路径取 `route_trace.final_model`，格式 `<model>@<provider_instance>`（例：`gpt-6.1-sol@cliproxy`）；没有 route trace 时会记成 `llm.code` 这类别名 | `agent_tool/src/xllm.rs:5377-5381`、`aicc/src/service/inference.rs:1740` |
| 标题 | work session 没有标题字段：`SubSessionSpec` 只有 objective，`create-worksession` 没有 `--title`，登记表只有 `objective`。Todo 有 `TodoRecord.title`，但它的 session 的 objective 是 `task_text(...)` 拼出的长文 | `api.rs:621`、`xagent_cli.rs:82`、`todos.rs:149` |
| 时间 | 有 `SessionSection.created_at_ms`；没有结束时间（登记表 `updated_at_ms` 会被之后的写覆盖） | `protocol/config.rs:295` |
| TodoList | `<SessionDir>/.opendan_agent_session/todos.json`（`opendan.todo_list/1`）：`TodoRecord{todo_id, seq, status, mode, title, after, blocked_by, block_reason, attempts[{attempt, child_sid, status, started/ended, paused/resumed, summary}]}` | `agent_tool/src/todo_tools.rs:42-240,471` |
| Agent 服务的只读 RPC | `session.read`（登记表 + 状态 + config + statistics + runs + children）、`session.worklog {sid, turn}`；只读白名单 `is_read`。Desktop 经 `agent.list` 把 DID 解析成 AgentId，再 `getServiceRpcClient(agent_id)` 调用，用的是登录者的权限 | `frame/opendan/src/service.rs:172-256,427-475`、`desktop/.../conversation/worklog/source.ts` |
| 已有 UI 原则 | “不为气泡提前读取日志，不逐条查询 TaskMgr”；TaskMgr 与 OpenDAN 都用当前登录者的权限 | `desktop/src/app/messagehub/UI_DATAMODEL.md:794-801` |
| TaskMgr 变化提示 | `TaskWatchSource.subscribe('tree', rootId)` 订阅 `/obj/taskmgr/<root_id>/tree`。UI Turn task 是整棵树（work Turn、plan Turn、Todo Turn）的根 | `desktop/src/api/task_mgr.ts:1318-1324` |
| 时钟 | `useMessageHubClock` 每 60 s 才跳一次 | `desktop/src/app/messagehub/store/index.ts:40-45` |

## 3. 目标形状

### 3.1 气泡下面显示什么

```
┌ Agent 回复气泡 ──────────────────────────────────────────────┐
│ 已为你创建工作：……（正文不变）                                  │
│ [Q/A 列表（不变）]                                            │
│ ┌──────────────────────────────────────────────────────────┐ │
│ │ ⟳ 重写 llm_context 设计文档          4m12s  入 86k · 出 9.1k │ │
│ │   T02 进行中：编辑 §3 目标形状                  gpt-6.1-sol │ │
│ │ ▾ TodoList · 1/4                                         │ │
│ │   ✓ T01 梳理现状                 1m03s  入 21k · 出 2.3k   │ │
│ │     已完成：找到 3 处与源码不符                  gpt-6.1-sol │ │
│ │   ⟳ T02 改写 §3                  2m40s  入 40k · 出 5.0k   │ │
│ │     编辑 doc/llm_context/LLM Context 设计.md    gpt-6.1-sol │ │
│ │   ○ T03 补测试                                            │ │
│ │   ○ T04 验收                                              │ │
│ └──────────────────────────────────────────────────────────┘ │
└──────────────────────────────────────────────────────────────┘
```

规则：

1. 气泡带 `agent_task` 时按下面的顺序决定附属 UI：
   - UI Turn task 读不到，或没有子任务 → 维持现状（`MessageTaskArea` 不变，快速回复不显示、“回复同步中”照旧）。意图分析进行中（探索、问用户）走的就是这一支：**try_analyze_user_intent 不变**。
   - UI Turn task 有子任务 → 向 Agent 服务取这个 Turn 建出的 work session 卡（§3.5）。取到至少一张 → 附属 UI 换成卡片树，原来的两行状态和 TaskMgr 展开树不再显示在气泡里（树留在消息详情的 task 页，Q7）。取不到（没有权限、RPC 失败、0 张）→ 退回现状。
2. 一个 Turn 建了多个 work session → 按创建时间叠放多张卡。
3. 卡片树只有两层：work session 卡 →（plan 才有）Todo 卡。Todo session 自己再建的子 session 不在气泡里显示，到详情里看。
4. Goal（`goal_created`）不在本单（Q9）。

### 3.2 2 行条目卡

| 位置 | 内容 | 规则 |
| --- | --- | --- |
| 行 1 左 | 状态图标 | §3.3；颜色沿用 `MessageTask.tsx` 的 `toneColor` / `ToneIcon` |
| 行 1 | 标题 | `title`（§3.7）。后端截 60 字符（按 char 计）；UI 单行省略，完整文字放进 `title` 属性 |
| 行 1 右 | 运行时间 | `(finished_at_ms ?? now) − created_at_ms`，显示为 `45s` / `3m12s` / `1h05m` / `2d3h`，`tabular-nums`。`useMessageHubClock` 每 60 s 才跳，运行中的卡要自己挂一个 1 s 的计时器（只在有运行中卡片挂载时存在） |
| 行 1 右 | 输入 / 输出 token | `入 86k · 出 9.1k`（en：`in 86k · out 9.1k`）。用户说尤其关注 output：输出用正文色，输入用次要色。小于 1000 显示原数，否则 `k` / `M` 保留一位小数；还没有 Round 时不显示。plan 卡显示 plan 自身 + 全部 Todo session 之和（Q4），tooltip 拆“plan 自身 / Todo 合计”；其余卡显示自己的。tooltip 还给精确数和 Round 数 |
| 行 2 左 | 一句话实时状态 | 来源见 §3.3。后端截 160 字符；UI 单行省略，完整文字放进 `title` 属性 |
| 行 2 右 | 模型名 | 最近一个成功 Round 的实际模型，去掉 `@provider` 后缀（`gpt-6.1-sol@cliproxy` → `gpt-6.1-sol`）。`llm.` 开头的别名或空值一律不显示。一个 session 正常只有一个实际模型；AICC failover 后出现多个时仍显示最近的，tooltip 才列出全部（Q6） |

- 整张卡可点（§3.8）：`role=button`，Enter / Space 触发，焦点用 Desktop 全局 `:focus-visible`，不另加 outline。
- 窄宽（卡片容器 < 360px，手机）：行 1 隐藏输入 token，只留输出 token 和时间；行 2 可以隐藏模型名。按容器宽度判断（容器查询或 ResizeObserver），不按视口宽度，因为气泡宽度不等于视口宽度。
- Todo 卡和 work session 卡是同一个组件：Todo 卡缩进一级、字号小一号，标题为 `T02 <todo.title>`（Q11）。
- 悬停、按下等状态色按 Desktop 主题约定从 `--cp-*` 派生（悬停用 `color-mix(in srgb, var(--cp-text) 6%, transparent)`，同 `MessageTask.tsx`）。浅色、深色都要截图核对。

### 3.3 状态枚举与一句话

状态枚举由后端在一处定义，UI 只负责映射图标：

| `state` | 条件（自上而下取第一条） | 图标 / tone | 一句话 |
| --- | --- | --- | --- |
| `stopped` | Finished ∧ outcome = Stopped | `Ban` / canceled | `report_brief` 首行，空则“已停止” |
| `failed` | Finished ∧ outcome = Failed | `CircleX` / failed | `last_error.message`，没有则 `report_brief` 首行 |
| `succeeded` | Finished ∧ outcome = Succeeded | `CircleCheck` / success | `report_brief` 首行 |
| `paused` | `paused` | `CirclePause` / paused | 暂停原因，空则“已暂停” |
| `needs_input` | `needs_input` | `MessageCircleQuestion` / waiting | “等待你的回答” |
| `queued` | Created / Ready，还没有开过 Turn | `Clock` / waiting | “排队中” |
| `running` | 其余：Running；Waiting 但在跟自己的任务或子 session；Turn 之间在等 timer 或任务 | `LoaderCircle` 旋转 / active | Turn 开着：`state.one_line_status`（实时，§3.10：调用带 `intent` 时就是 intent）；Turn 之间：上一个 Turn 回复的首行，也就是多 Turn Work 的进度回复（例：“两个构建还在跑……”） |

- “运行中、在等自己的任务”对用户来说就是运行中，不单独成一类，行 2 的一句话会说“等待 2 个任务”。
- 一句话要么是模型写的 intent（模型的语言），要么是老逻辑的英文机器文案。UI 对老逻辑的已知文案做本地化：`running <tool>` → “运行 `<tool>`”；`working (<behavior>)` / `working` → “处理中”；`waiting for N task(s)` → “等待 N 个任务”；`waiting for N sub session(s)` → “等待 N 个子工作”；其余原样显示。
- 后端给的 Turn 之间那一句不用 `one_line_status`（Turn 关闭时老逻辑写的是 `waiting for input: …` 这种带英文前缀的句子），改为取最后一个已关闭 Turn 绑定的 `summary`（`state.turn_tasks`，由 `close_turn_task` 写，≤400）的首行。实施时核对 `commit_kept` 关 Turn 时写进 summary 的是不是该 Turn 的回复。

### 3.4 TodoList（plan 才有）

- 位置：plan 卡下方。标题行 `▾ TodoList · 已完成/总数`（总数不含 cancelled），点击收起或展开。
- 默认 plan 未结束时展开、结束后收起。用户手动的选择按 viewer + 锚点 + session 记在 localStorage，照搬 `useQaExpanded` 的写法和上限。
- Todo 按 `seq` 排列：
  - 最新 attempt 绑定了 `child_sid` → 2 行 Todo 卡（内容是该 Todo session 的卡，标题换成 Todo 标题）。retry 过的只显示最新 attempt，标题后加 `#2`。
  - 没有绑定（pending 未就绪、blocked、没有 attempt 就 cancelled）→ 只显示一行：小图标 + `T03 标题`。图标：pending 用 `Circle`；blocked 用 `CircleSlash`，tooltip 给 `blocked_by` / `block_reason`；cancelled 用 `Ban`，加删除线、次要色（Q12）。
- Todo 超过 12 条时，默认只显示未结束的和最近结束的 3 条，其余折叠成“还有 N 项”。

### 3.5 数据通路

1. 气泡：`agent_task` → `useTaskSummary`（已有）→ 看 UI Turn task 有没有子任务（`children.items.length > 0`）。没有子任务就不调 RPC，快速回复和纯问答都不调。这样仍守住“不为气泡提前读日志”的原则，只有真的建了子 session 的 Turn 多一次调用。
2. 新的只读 RPC（opendan 服务，Q1）`session.work_cards {sid, task_id}`：
   - `sid`：UI session，即 UI Turn task 的 `input.session_id`；`task_id`：UI Turn 的 task id，即气泡的 `agent_task`。
   - 选 `children_of(&[sid])` 中 `origin.parent_task == task_id` 的 session，按 `created_at_ms` 排序。不按 turn 号查 `turn_tasks`，因为那个列表只保留最近 64 条已结的绑定（`TURN_TASKS_KEEP_SETTLED`）。
   - 返回：

```jsonc
{
  "agent_did": "...", "session_id": "<ui sid>", "task_id": "<ui turn task>",
  // 全部卡片（含 Todo 卡）都已终态且 TodoList 已收敛 → UI 停止刷新
  "complete": false,
  "sessions": [WorkCard]
}
// WorkCard
{
  "session_id": "...",
  "shape": "simple | plan | explore",   // class plan_simple → plan；behavior explore → explore；其余 simple
  "title": "...",                       // ≤60 chars，§3.7
  "created_at_ms": 0,
  "finished_at_ms": null,
  "state": "queued | running | needs_input | paused | succeeded | failed | stopped",
  "line": "...",                        // ≤160 chars，一句话（§3.3、§3.10），全部来自 session state
  "usage": {
    "input_tokens": 0, "output_tokens": 0, "rounds": 0,
    "models": [{ "model": "gpt-6.1-sol@cliproxy", "input_tokens": 0, "output_tokens": 0 }]
  },
  "model": "gpt-6.1-sol@cliproxy",      // 最近一个成功 Round 的，没有为 null；UI 负责去后缀和过滤别名
  "subtree_usage": null,                // plan：自身 + 全部 Todo session 之和（Q4），形状同 usage
  "todos": null                         // plan：见下
}
// todos
{
  "rev": 7, "total": 5, "done": 2,
  "items": [{
    "todo_id": "T01", "seq": 1, "title": "...", "status": "completed", "mode": "serial | explore",
    "attempt": 1, "blocked_by": [], "block_reason": null,
    "card": null                        // 绑定的 Todo session 的 WorkCard（不再嵌 todos），没有为 null
  }]
}
```

   - `usage` 由该 session 的 `usage.jsonl` 全量求和得出。每次调用读一遍即可（一行约 150 B，几百个 Round 也不到 100 KB）；需要时在服务内按 (path, len) 缓存。不读 `static.json`（§2：运行中为 0）。xllm 接手的 run 的推理只记在 run.json（第一刀已知缺口），本单不补，卡片会少算，记进 §8。
   - 卡片投影写成 libopendan 的纯函数（输入登记表条目 + SessionDir，输出 `WorkCard`），opendan 只做路由与鉴权，方便单测。
   - 鉴权：加入 `is_read`，能 `session.read` 这个 UI session 的调用者才能调。被拒时 UI 退回现状。

3. 实时一句话：由 libopendan 写进 session state（§3.10），卡片投影直接读 `state.json`，UI 不为一句话再去读每个 Turn task。Turn task 的 TaskMgr message 用同一句话，所以它的变化仍会触发 §3.6 的 tree 提示。

### 3.6 刷新

- 订阅 `getTaskWatchSource().subscribe('tree', <UI Turn task id>)`：整棵树（work Turn、plan Turn、Todo Turn）任何变化都触发重取 `session.work_cards`，防抖 1 s，两次调用至少间隔 2 s。
- 兜底轮询：`complete = false` 时每 15 s 一次。`complete = true` 后停掉一切订阅和轮询（同 TaskWatch 拿到终态后不再订阅）。
- 返回 0 张卡（UI Turn 的子任务不是 work session，例如别的子任务）：继续显示现状任务区，只在 tree 提示时重取，UI Turn task 终态后停止。
- 页面重新可见或 `online` 时立即重取一次。
- 只对挂载中的气泡（虚拟列表的可见区）生效，卸载即释放。同一 (sid, task_id) 在多处共享一份缓存，照 `getTaskWatch()` 的单例 + `useSyncExternalStore` 写。
- 失败：沿用上次的结果并标记 stale（图标停止旋转，行 2 尾部加“状态可能已过期”）；一次都没成功过就退回现状。

### 3.7 标题

- 链路：`SubSessionSpec.title` → `SessionSection.title: Option<String>` → 登记表 `RegistryEntry.title: Option<String>`（serde default；DV 可全新安装，不写兼容）。超过 60 chars 截断。
- 独立创建（Q3）：`create-worksession --title <t>` **必填**，缺了就像缺 `--objective` 一样报参数错误。调用方都要给：analyze_intent、`goal.toml`（只改提示词，Q9）、测试、cmd_skills 里的示例。
- Todo 创建（Q3）：今天 `todos.rs::child_spec` 只给 objective（`task_text`，首行 `Todo T01 (attempt 1, serial): <title>`）和整段 msg，session 没有标题。改为 `title = TodoRecord.title`。
- 没有标题的 session（UI session、Goal session 本身）：卡片用不到；万一出现，用 objective 第一个非空行截 60。
- Turn task 的 name：今天取 Turn 打开批次的第一条消息正文，没有再用 Todo 更新摘要，最后才是 objective（`runner/live.rs:1359`、`turn_task.rs:83`）。work session 第 1 个 Turn 的第一条消息是 `--msg` 长文，后面的 Turn 是 objective，都不适合当名字。改为：有 `title` 的 session 一律用 title；UI session 照旧（用户消息就是合适的 Turn 名）；Goal 的 `goal … · activation …` 照旧。
- 提示词写法要求（analyze_intent、goal.toml）：用户的语言；不超过 20 个汉字或 40 个英文字符；说清做什么。analyze_intent 的结果 `work` 里回填 `title`（schema 加字段）。

### 3.8 点击与详情（“详细内容”）

- 点 work session 卡或 Todo 卡 → 打开消息详情侧栏的「工作日志」页签，显示**该 session 的连续日志**（Q8，P0）。`AgentWorklog` 已经支持 `{target, title}` 目的地，要让 `MessageDetails` 接受一个初始目的地，`WorklogTarget.turn` 改为可选。
- `session.worklog {sid, turn?}`：`turn` 改为可选。
  - 不给 `turn`：按提交顺序返回整个 session 的记录，没有 turn 的记录（控制、拒收等）也返回；`complete` = session 已 Finished 且读到了已提交的末尾。
  - 给 `turn`：现有行为不变。Agent 回复气泡默认打开的 UI Turn 日志仍用它（UI session 的历史很长，按 Turn 定位是对的）。
  - libopendan `Worklog::page` 的 `turn: u64` 改为 `Option<u64>`，倒序分页里“遇到更早的 Turn 就停”那条只在给了 `turn` 时生效。
  - UI 的 `worklogRows` 在没有 turn 时不过滤；已有的 `turn_started` / `turn_ended` 行就是 Turn 之间的分隔，不另做 Turn 切换。
- 工作日志头部加一个可折叠的「工作内容」块，内容是用户要看的“详细内容”：objective 全文（work session 系统提示词的关键）、创建时间（绝对时间）、runtime / workspace、按模型分列的 token、结束后的最终报告。数据用已有的 `session.read {sid, worklog: 0, report: true}`，不新增接口。

### 3.9 降级与保持不变的部分

- 读不到卡（没有权限、Agent 服务不可达、旧版 Agent 不认识这个 RPC）→ 现状任务区，不报错。
- Q/A 列表（`AgentQuestions.tsx`）、“回复同步中”的推断、消息详情的 task 页、TaskMgr 取消 → stop 的桥都不动。
- Telegram 等 tunnel 不动，这是原生 UI 的增强。

### 3.10 实时一句话写进 session state（用户 2026-11-02 review 定）

- 字段：`SessionState.one_line_status`，经 `report()` 同步到登记表 `SessionStatus.one_line_status`。不新增字段。`activity.summary` 不动：它是“在做什么”的声明，其他 session 的背景行读它，每次工具调用都改会让父 session 的背景行跟着变。
- 生成：把 `turn_task.rs::status_of` 里生成 message 的那段提成一个函数（建议 `live_line(state, tool, intent) -> String`），Turn task 的 message 和 `one_line_status` 都用它，两边永远是同一句话。规则按老逻辑的顺序，只在“正在调工具”这一支加特例：
  1. paused → `paused: <reason>` / `paused`（老逻辑）
  2. Waiting 的几支 → `waiting for N sub session(s)` / `waiting for N task(s)` / `waiting for input` / `waiting`（老逻辑）
  3. 正在调工具，**且这次 function call 带非空的 `intent`** → 就是 intent 本身（trim 后截 160），不再拼成 `running <tool>: <intent>`
  4. 正在调工具但没有 intent → `running <tool>`（老逻辑）
  5. 没有工具 → `working (<behavior>)` / `working`（老逻辑）
- 什么时候写：
  - 工具派发时（`tools.rs` 设 `current_intent` 的地方）算一次 `live_line`，和 `one_line_status` 不同就更新 state、提交、`report()`。
  - 进入 Waiting、暂停这些今天已经会提交 state 的地方，也用 `live_line` 写一次。
  - Turn 关闭、会话结束、工作区失效、Goal 结束这些老逻辑写 `one_line_status` 的地方**不变**，照旧覆盖实时那句。所以子 session 结束 / needs_input 时父 session 看到的仍是老逻辑的结局句子。
- 节流：变化就提交，但距上次 state 提交不到 1 s 的只改内存，留到下一个边界（工具结果后的 hook、`heartbeat`、Turn 关闭的提交）一起提交。最后一句不能丢。
- intent 一律用 `agent_tool::intent::intent_of(&call)` 取，标准工具的可选 intent 和 `call_intent` behavior 的必填 intent 都算。xllm 接手的 run 如果不经过 libopendan 的工具派发，就拿不到 intent，实施时核对；拿不到就走老逻辑，记进 §8。
- 读者核对：父 session 的子 session 事件 / 摘要（`children.rs`）、Goal 的 works 列表（`goal.rs`）、session 列表文本（`inputs.rs:222`）、Todo 摘要缺省值（`todos.rs:330`）运行中会看到实时那句。确认它们在运行中读到进度句没有问题（事件都在结束或 needs_input 时生成，那时老逻辑刚覆盖过）。

## 4. 裁决记录（2026-11-02 用户在文中回答；Claude 整理）

“用户原话”一列照录用户写回的内容；“结论”一列是据此落到正文的做法。

| Q | 问题 | 用户原话 | 结论 |
| --- | --- | --- | --- |
| Q1 | 卡片数据从哪来 | 按建议 | 新的只读 RPC `session.work_cards`，一次调用拿到整棵卡片树（§3.5） |
| Q2 | Turn 开着时的实时一句话 | （review 时口述）实时 1 句话要更新 session state，function call 带这个字段时优先用它，否则用老逻辑 | 写 `SessionState.one_line_status`，带 `intent` 用 intent，否则走 `status_of` 的老文案（§3.10） |
| Q3 | 标题 | simple-plan 下，worksession 是通过创建 todo 的时候创建的，现在没有标题么？独立的 worksession 创建要有标题 | 答：Todo 创建的 work session 今天**没有**自己的标题，只有 `TodoRecord.title`；它的 objective 首行是 `Todo T01 (attempt 1, serial): <title>`，Turn task 名是整段 task 文本。改为：Todo 派发时 `title = TodoRecord.title`；独立创建 `create-worksession --title` **必填**（§3.7） |
| Q4 | plan 卡的 token | 显示之和 | plan 卡显示 plan 自身 + 全部 Todo session 之和，tooltip 拆“plan 自身 / Todo 合计”；Todo 卡显示自己的 |
| Q5 | 运行时间 | 根据 session 的创建时间推算？ | 答：是，按 `created_at_ms` 起算的墙钟，运行中算到现在。结束后要停表就得有结束时刻，现有数据给不了可靠值：`state.updated_at_ms` 在结束后还会被提交覆盖（例如 `flush_turn_tasks` 关 task 后会再 commit）；最后一条 `turn_ended` 在“Turn 之间被 stop”时不存在。所以仍新增 `SessionState.finished_at_ms`，只在置 Finished 的提交里写（§5 A2） |
| Q6 | 用过多个模型 | workession 现在有机会多模型么？ | 答：单个 work session 只用自己 behavior 的一个模型别名（`work_simple` 是 `llm.chat`、`plan_simple` 是 `llm.plan`、`explore` 是 `llm.code`），正常只有一个实际模型；只有 AICC 运行时 failover（`runtime_failover` + `fallback_candidates`）或改了路由时，同一别名会落到另一个模型。plan 的合计里会有多个模型，但 plan 卡显示的是 plan 自己的模型。结论：卡片显示最近一个实际模型，不做多模型的专门 UI，tooltip 在不止一个时才列出 |
| Q7 | 气泡里的 TaskMgr 展开树 | 按你的建议 | 有卡片时从气泡里去掉，消息详情的 task 页保留 |
| Q8 | 详情的日志 | session.worklog 是 session 级别的，不是 turn 级别的。用户应该在 worklog 里看到连续的 | 答：今天 RPC 是按 session 寻址的，但 `turn` 是必填的过滤参数（`service.rs:220`、`Worklog::page(.., turn: u64, ..)`）。改为 `turn` 可选，点卡打开**整个 session 的连续日志**，P0 就做，不做 Turn 切换（§3.8） |
| Q9 | Goal | goal 相关 UI 修改可以不做，还没重构过去 | 不做 Goal UI。`goal.toml` 也调 `create-worksession`，只补 `--title` 这一句提示词，否则 Q3 的必填会让 Goal 建 Work 失败 |
| Q10 | 群聊 / 非 owner 观察者 | 按建议 | RPC 被拒就退回现状 |
| Q11 | Todo 卡标题 | `T02 <todo.title>` | 同 |
| Q12 | cancelled 的 Todo | 按建议 | 显示，次要色 + 删除线，不计入总数 |
| Q13 | 刷新方式 | 按建议 | tree 提示驱动 + 15 s 兜底轮询，`complete` 后停止 |

## 5. 实施清单

### A. 后端（libopendan / opendan / xagent）

- [x] A0 实时一句话（§3.10）：从 `status_of` 提出 `live_line`，加 intent 特例；工具派发、进入 Waiting、暂停时写 `state.one_line_status` 并 `report()`，带 1 s 节流；Turn task message 改用 `live_line`；老逻辑的写入点不动。单测：带 intent 的调用 → `one_line_status` 和 Turn task message 都是 intent；不带 intent → `running shell`；转后台任务后 → `waiting for N task(s)`；Turn 关闭后被老逻辑覆盖；1 s 内连续两次调用，最后一句在下一个边界提交。
- [x] A1 标题（§3.7）：`SubSessionSpec.title`、`SessionSection.title`、`RegistryEntry.title`；`create-worksession --title` 必填（`xagent_cli.rs` 的用法行、解析、help）；`todos.rs::child_spec` 设 `title = TodoRecord.title`；Turn task name 在有 title 时用 title（`live.rs` 算 `title` 处）；截断统一 60。现有调用 `create-worksession` 的测试补 `--title`。
- [x] A2 结束时间：`SessionState.finished_at_ms: Option<u64>`，在所有把 `run_state` 置为 `Finished` 的提交里写（`commit_run_end`、`drive.rs:302` 的工作区失效路径、stop 路径；实施时 grep `RunState::Finished` 的全部赋值点）。先确认没有现成字段可用。
- [x] A3 卡片投影：libopendan 新模块（建议 `src/work_card.rs`：`pub fn work_card(entry, dir) -> Result<WorkCard>`、`fn usage_of(dir)`），包含 §3.3 的状态判定、Turn 之间的一句话（最后关闭的 Turn 绑定的 summary 首行）、`usage.jsonl` 求和与最近模型、读 `todos.json`（用 `agent_tool::todo_tools` 的读函数，只读）、Todo 卡递归一层、`subtree_usage`、`complete` 判定。类型带 `Serialize + JsonSchema`。
- [x] A4 RPC：opendan `service.rs` 加 `session.work_cards`（进 `is_read`），`children_of(&[sid])` 后按 `origin.parent_task` 过滤。
- [x] A5 连续日志（§3.8，Q8）：`Worklog::page` 的 `turn` 改 `Option<u64>`；opendan `session.worklog` 的 `turn` 改可选，不给时 `complete` = Finished 且读到已提交末尾、`task_id` 返回 null。
- [x] A6 测试：libopendan 单测（7 种状态、Turn 之间的一句话、usage 求和与模型、Todo 已绑定 / 未绑定 / retry / cancelled、Todo session 的 title、`complete`、不带 turn 的 worklog 分页跨 Turn 连续且含无 turn 记录）；opendan `tests/loader.rs` 加一例（UI session 建一个 simple、一个 plan，plan 派 2 个 Todo；按 task_id 只返回该 Turn 建的 session；权限被拒；`session.worklog` 不带 turn）；xagent `--title` 缺失报错与截断。

### B. Jarvis 包与提示词

- [x] B1 `analyze_intent.toml`：Creating 段的 `create-worksession` 加 `--title "<short title>"` 和写法要求；Finishing 的 `work` 加 `title`；schema 的 `work.required` 加 `title`。
- [x] B2 `goal.toml:18`：`create-worksession` 加 `--title`（只这一句，Goal 的其他部分不动，Q9）。
- [x] B3 `plan_simple.toml`：确认 `todo add --title` 有写法要求（短、动宾结构，它现在就是卡片标题），没有就补一句。
- [x] B4 cmd_skills（AGENTS.md 联动规则）：grep `cmd_skills/` 里的 `create-worksession` 与 `session.worklog` 用法（至少 `opendan_agent_dev.md`、`ui_core.md`），示例补 `--title`、说明 `turn` 可选；不受影响的写明依据。
- [x] B5 按 `AGENTS.md` 重生成 fixtures / schema（`cargo run -p libopendan --example fixtures -- <绝对路径>/doc/opendan/protocol/fixtures`、`xagent schema doc/opendan/protocol/schema`），只保留真正变化的文件。

### C. Desktop（`src/frame/desktop/src/app/messagehub/`）

- [x] C1 数据层 `conversation/worksessions/`：
  - `model.ts`：`WorkCard` 类型、`displayModel`（去掉 `@…`，过滤 `llm.` 和空值）、`formatDuration`、`formatTokens`、`localizeLine`（§3.3 的已知前缀）、状态 → tone / 图标。
  - `source.ts`：照 `worklog/source.ts` 写，`agent.list` 解析 AgentId → `session.work_cards`，校验返回的身份。
  - `watch.ts`：单例缓存 + tree 订阅 + 防抖 + 兜底轮询 + stale（§3.6）。
  - `index.ts`：`useWorkCards(sid, taskId, enabled)`。
- [x] C2 mock：`mock/worksessions.ts` 三个场景（simple 运行中；plan 运行中，2 条已绑定、2 条未绑定、1 条 cancelled；全部结束），接入 `isMessageHubMock()`，并与 `mock/tasks.ts` 的 task 树对得上（UI Turn task 要有子任务）。
- [x] C3 组件 `conversation/worksessions/WorkSessionCards.tsx`：`WorkCardRow`（2 行，Todo 卡复用）、`TodoListSection`（折叠、记忆、“还有 N 项”）、`UnboundTodoRow`。`ToneIcon` / `toneColor` 从 `MessageTask.tsx` 导出复用，不要复制一份。
- [x] C4 接入：把 `renderers.tsx:558` 换成一个分派组件（有卡 → 卡片树，否则 `MessageTaskArea`），Q/A 列表位置不变。
- [x] C5 详情（§3.8）：`MessageDetails` 接受初始 worklog 目的地；`WorklogTarget.turn` 可选，`source.ts` / `useWorklog` / `worklogRows` 支持不带 turn 的连续日志（身份校验相应放宽）；`AgentWorklog` 头部加「工作内容」折叠块（`session.read`）。气泡默认打开的 UI Turn 日志不变。
- [x] C6 i18n：`src/i18n/messagehub.ts` 补 en + zh，键放在 `messagehub.work.*` 下（状态、入 / 出、TodoList 标题、还有 N 项、排队中、等待你的回答、已暂停、状态可能已过期、前缀本地化）。
- [x] C7 测试：
  - deno 数据模型测试：`model.ts` 的全部纯函数；`watch.ts` 的终态停止和防抖。
  - Playwright mock（`tests/e2e/pages/messagehub-agent-task.spec.ts` 新增用例）：simple 卡两行内容；plan 的 TodoList 展开 / 收起与记忆；未绑定的 Todo 只一行；窄宽只留输出 token；点卡打开对应 session 的连续日志（跨 Turn、有 Turn 分隔行）；终态后不再请求；RPC 被拒退回旧任务区；意图分析阶段仍是旧任务区。
  - 浅色 / 深色、桌面 / 手机截图。

### D. 文档

- [x] D1 `src/frame/opendan/README.md`：`session.work_cards` 的契约（参数、筛选规则、字段、`complete` 语义、鉴权）；`session.worklog` 的 `turn` 改为可选及不带时的 `complete` 语义。
- [x] D2 `src/frame/desktop/src/app/messagehub/UI_DATAMODEL.md`：在第 794 行附近补一节“work session 卡片”（什么时候调、怎么刷新、怎么降级），并说明它是“不为气泡提前读取日志”原则的有界例外。
- [x] D3 `doc/opendan/protocol`：Session Directory / Agent State Protocol 补 `title`、`finished_at_ms`；`doc/opendan/xAgent.md` 补 `create-worksession --title`。

## 6. 验证与 DV

- 单测（在 `src/` 下）：`cargo test -p libopendan -- --test-threads=1`（含新单测，以及回归 `work_simple_async` / `plan_simple_async` / `turn_task`）、`cargo test -p opendan -- --test-threads=1`、`cargo test -p agent_tool --lib todo_tools`。并发跑多个 cargo 时加 `-j 8`，否则会被 OOM 杀掉。
- Desktop：tsc / eslint / deno 数据模型测试 / Playwright（`--workers=2`，分批跑）。
- 真实 Zone DV（本机 DV Zone，Jarvis 原生私聊；部署方式和端口见 `messagehub-agent-chat-task-todo.md` §7.5）：
  1. 发一个会建 simple work 的请求（例如并行跑两个慢构建）。检查：意图分析阶段是旧任务区，建出 work 后变成卡；行 2 的一句话随工具变化，模型带了 intent 的调用显示的就是 intent，和 TaskMgr 里那个 Turn task 的 message 一致；运行中 token 在涨（不是 0）；模型名是 `gpt-…` 而不是 `llm.…`；结束后显示 ✓，时间停止，没有后续请求。
  2. 发一个会建 plan_simple 的请求。检查：plan 卡 + TodoList；未派发的 Todo 只一行，派发后变成 2 行卡；plan 卡的 token 等于自身 + Todo 合计；结束后 TodoList 自动收起；刷新页面后状态一致。
  3. 在消息详情里检查「工作内容」，以及多 Turn work session 的日志从头到尾连续、Turn 之间有分隔。TaskMgr 里 work session 的 Turn task 名是标题，Todo session 的是 Todo 标题。
  4. 手机宽度与深色主题截图。
- 证据放 `/home/lzc/project/messagehub_work_ui_dv_runs/`（截图、RPC 返回样本）。

## 7. 关联

- [MessageHub Agent Chat：可编辑回复与 task 状态展示](./messagehub-agent-chat-task-todo.md)（两行任务区、§8 Turn worklog）
- [简单 Work Session 全异步化](./opendan-simple-worksession-async-todo.md)（多 Turn Work、同一 run）
- [plan_simple 多 Turn 化与 TodoList 自动调度](./opendan-plan-simple-async-todo.md)
- [意图分析 try_analyze_user_intent](./opendan-analyze-user-intent-todo.md)
- [llm_context 冻结](./llm-context-freeze-todo.md)

## 8. 实施记录

### 8.1 落点（后端 / 提示词 / 文档）

| 条目 | 位置 |
| --- | --- |
| A0 实时一句话 | `lib_opendan/src/runner/turn_task.rs`：`live_line(s, tool, intent)`（`status_of` 改为调用它）、`note_live_line`（Turn 开着且未结束时同步 `one_line_status`）、`live_line_changed`（工具派发后算一次，距上次 state 提交 ≥1 s 才 `commit_and_report`，否则置 `Shared.line_pending`）；`runner/tools.rs` 派发处调用 `live_line_changed` + `sync_turn_task`；`runner/hook.rs` 心跳在 `line_pending` 时提交；进入 Waiting（`drive.rs` 无事可做转 Waiting、`outcome.rs::commit_kept` 保活不关 Turn）、暂停 / 恢复（`inputs.rs`）、输入批次提交（`live.rs`，`working (<behavior>)`）各写一次 |
| A1 标题 | `SessionSpec.title` / `SubSessionSpec.title`（`api.rs`）→ `SessionSection.title`（`protocol/config.rs`，`clip_title` 截 60、合并空白）→ `RegistryEntry.title`（`protocol/agent_state.rs`）；`create_sub_session` 透传；`xagent create-worksession --title` 必填（用法行、help、缺失 / 空白报 `missing --title`）；`todos.rs::child_spec` `title = TodoRecord.title`；`turn_task.rs::ensure_turn_task` 有 title 时 Turn task 名用 title |
| A2 结束时间 | `SessionState.finished_at_ms`（`protocol/state.rs`），三个置 Finished 的点（`outcome.rs::finish_run`、`drive.rs` stop 路径、`drive.rs` 工作区失效路径）写入 |
| A3 卡片投影 | 新模块 `lib_opendan/src/work_card.rs`：`work_card(entry, children)`（纯函数，读 SessionDir）、`usage_of(sd)`、`work_cards(agent, sid, task_id)`（登记表查询 + 排序 + `complete`）；类型 `WorkCards / WorkCard / CardState / CardShape / CardUsage / CardTodos / CardTodo`（`Serialize + JsonSchema`） |
| A4 RPC | `opendan/src/service.rs`：`session.work_cards {sid, task_id}`，进 `is_read` 与只读 Agent 测试清单 |
| A5 连续日志 | `session/worklog.rs::Worklog::page(.., turn: Option<u64>, ..)`；`service.rs::read_worklog` 的 `turn` 可选，不给时 `turn` / `task_id` 为 null、`complete` = Finished 且 `next_after == committed` |
| B 提示词 | `behaviors/analyze_intent.toml`（`--title` 与写法、`work.title`、schema `required`）、`goal.toml`（`--title`）、`plan_simple.toml`（Todo 标题写法）；cmd_skill `opendan_agent_dev.md`（两个示例补 `--title`、标题与实时一句话的说明）；`ui_core.md` 不受影响（不引用 `create-worksession` / `session.worklog`） |
| B5 fixtures / schema | schema 只变 `registry_entry` / `session_config` / `session_state` 三个；fixtures 只保留 02 / 08 / 09 / 16（结束的 session 多了 `finished_at_ms`）与 03（见 8.2） |
| D 文档 | `opendan/README.md`（`session.work_cards` 契约、`session.worklog` 的 `turn` 可选）、`doc/opendan/protocol/{Session Directory,Agent State} Protocol.md`（`title`、`finished_at_ms`、`one_line_status` 语义）、`doc/opendan/xAgent.md`（`--title`） |
| C Desktop | 新目录 `desktop/src/app/messagehub/conversation/worksessions/`：`model.ts`（类型、解析、`displayModel` 去 `@provider` 与 `:variant`、`formatDuration`、`formatTokens`、`localizeLine`、状态 → tone、Todo 标题与“还有 N 项”）、`source.ts`（`session.work_cards`，校验应答身份）、`watch.ts`（单例缓存、tree 提示防抖、最小间隔、兜底轮询、stale、从未成功的读退避重试、`complete` 停止）、`index.ts`（`useWorkCards`、只在有运行中卡片时跳的 1 s 时钟、`useTodoExpanded`）、`WorkSessionCards.tsx`（`MessageAgentWork` 分派、卡片行、TodoList、未绑定 Todo 行）；`history/renderers.tsx` 任务区位置换成 `MessageAgentWork`；`history/actions.ts`、`ConversationView.tsx`、`MessageHubView.tsx`、`MessageDetails.tsx`（详情可落在指定 session 的工作日志）；`worklog/{model,source,AgentWorklog}`（`turn` 可选、身份校验放宽、「整个 session」视图与「工作内容」块、DID → AgentId 按 DID 缓存）；`tasks/{MessageTask.tsx,taskView.ts,index.ts}`（导出 `ToneIcon` 带 `spin`、`toneColor` 移到 `taskView.ts`、问答展开记忆抽成共享 helper 供 TodoList 复用）；`messagehub.css`；`i18n/messagehub.ts`（`messagehub.work.*` en / zh）；mock `mock/worksessions.ts` + `mock/{data,store,tasks,worklog}.ts`（会话 `session-coder-work`：simple 运行中、plan 运行中、14 个 Todo 的已结束 plan、意图分析中的 Turn；`localStorage` `buckyos.messagehub.mock.workCardsDenied` 模拟拒绝） |
| C 测试 | `tests/datamodel/messagehub-worksessions.test.ts`（11 项）、`tests/e2e/pages/messagehub-agent-task.spec.ts`（新增 7 例）、`messagehub-worklog.spec.ts`（子 session 链接改为整段日志）、`tests/e2e/real/messagehub-agent-task.real.spec.ts`（树气泡改为断言卡片、点卡看整段日志、light / dark / 手机截图） |
| D2 | `desktop/src/app/messagehub/UI_DATAMODEL.md`「work session 卡片」一节，子 session 链接一句改为连续日志 |

### 8.2 与原文不同的决定

1. 工具派发时除了写 `one_line_status`，还立即 `sync_turn_task`（原来 Turn task 的 message 要等到下一个推理前边界才更新，长命令期间 TaskMgr 一直是上一句）。每次工具调用最多一次 TaskMgr update；state 提交按 1 s 节流。
2. 实时一句话的写入点比 §3.10 多两处：输入批次提交（Turn 打开或新输入进来时写 `working (<behavior>)`）、`resume`。否则新 Turn 打开后、恢复后卡片仍显示上一句（例如 Turn 关闭时的结局句、`paused`）。`finish_run` 里 Turn 不关闭的等待（子 session 未结束时延后成功）仍走老逻辑 `one_line(next)`，没有改。
3. 结束卡片的一句话不用 `report_brief` 首行：`report.md` 首行是生成的 `# Report — <objective 首行>`，DV 第一例里卡片显示的就是这句。改为取结局首个非空行：`state.result.answer` → 提交的 report → `report.md` 去掉头部（`# Report — …`、`- session:` / `- goal:` / `- outcome:` / `- turns:`）后的正文。
4. `queued` / `needs_input` / 没有原因的 `paused` / 没有结局的 `stopped` 的 `line` 是空串，由 UI 用本地化文案；Finished 但没有 outcome 按 `stopped`。
5. `complete` 还要求 UI Turn 已关闭：Turn 开着时可能还会再建 work session。0 张卡时 `complete = false`。
6. `subtree_usage` = plan 自身 + plan 的全部直接子 session（含 retry 前的 attempt 的 session）。`CardTodo.attempt` 为 0 表示还没有 attempt。
7. 模型名还要去掉 `:variant`：真实 AICC 记的是 `gpt-6.1-sol:reasoning-medium@codex-proxy`、`gpt-6-astra:reasoning-high@codex-proxy`，只去 `@provider` 仍很长。卡片显示 `gpt-6.1-sol`，tooltip 给原始名（多个模型时列出全部）。
8. Desktop：从未成功的读按 3 s、6 s、12 s 退避自己重试（默认 4 次失败后放弃）。DV 中网关间歇返回 `503 service_not_ready`（`debug_jarvis` 接管端口期间），一个已结束的 Turn 收不到 tree 提示，原设计下第一次读失败就永远停在旧任务区。
9. Desktop：工作日志里子 session 的链接也改为打开整个 session 的连续日志（Q8 一致性）；「工作内容」默认展开、高度上限 35vh；会话日志头部不显示状态（点击时的快照会过期）；unbound Todo 的状态只放在读屏文字里。
10. `analyze_intent` 的结果 schema 里 `work.title` 设为 required（模型必须回填）。
11. 卡片的一句话去掉 Markdown 标题符号（结局首行常是 `## …`）。
12. fixtures：仓库里的 03 / 16 在 HEAD 上就失败（03 目录里同时有两代 run，`f03_orphan_run` 读到 2 个 run）。重生成后保留 02 / 08 / 09 / 16（结束的 session 多了 `finished_at_ms`）与 03，其余 12 个 case 还原；`cargo test -p libopendan --test fixtures` 19/19。

### 8.3 测试

- `cargo test -p libopendan -- --test-threads=1`：全量通过，唯一失败的 `fixtures`（03 / 16，HEAD 上同样失败）经 8.2-12 重生成后 19/19。新增 / 改动：`tests/work_card.rs`（5 项：7 种状态与一句话、结局句取 answer / 去头部 report、Turn 之间的一句话、usage 求和与最近模型、plan 的 TodoList（已绑定 / 未绑定 / retry / cancelled）、Turn 过滤与 `complete`、Todo session 标题截断）、`tests/turn_task.rs`（实时一句话：intent / `running shell` / `waiting for 1 task(s)` / Turn 关闭被老逻辑覆盖 / 1 s 内第二句在边界提交；Turn task 名用 title）、`tests/l1.rs`（不带 turn 的 worklog 分页跨 Turn 连续且含无 turn 记录）、`tests/xagent_cli.rs`（`--title` 缺失 / 空白报错、截断 60）、`tests/plan_simple_async.rs`（Todo session 的登记表 title = Todo 标题）、`tests/work_simple_async.rs`（Turn task message 断言改为 intent 本身）。
- `cargo test -p opendan -- --test-threads=1`：全过；`tests/loader.rs` 新增 `work_cards_and_the_continuous_worklog_over_krpc`（Turn 过滤、plan TodoList、缺 `task_id`、不带 turn 的 worklog），`web_api_…` 补 `session.work_cards` 需要 token。
- `cargo test -p agent_tool --lib todo_tools`：7/7。
- Desktop：`pnpm check`、eslint（messagehub 与改动的测试）通过；deno `messagehub-worksessions.test.ts` 11/11（`messagehub-*.test.ts` 全量 91 过 7 败，7 个都在未改动的 `messagehub-guest.test.ts`）；Playwright mock `messagehub-agent-task` + `messagehub-worklog` 22/22；其余 messagehub spec 的 4 个失败与本单无关（3 个 HEAD 上同样失败，1 个是 Playwright 50 MB buffer 限制）；`pnpm build` 通过。未跑：upload、voice-emoji、extensions spec 与仓库级 `pnpm lint`。

### 8.4 真实 Zone DV（2026-11-02，本机 test.buckyos.io）

环境：`test/test_opendan/agent_target.ts --create xiaobai` 新建 Agent `xiaobai.test.buckyos.io`，`./debug_jarvis.sh devtest --agent xiaobai.test.buckyos.io` 宿主托管（本单的 opendan / libopendan / Jarvis 包，kRPC `http://127.0.0.1:10048/kapi/opendan`）；Desktop `pnpm build` 后 `cp -r dist/. /opt/buckyos/bin/control-panel/web/`。驱动：消息经 msg-center `msg.post_send` 发出（`test_agent_chat_task.ts` 与本次临时脚本），`session.work_cards` 每 3 s 轮询记录变化。证据在 `/home/lzc/project/messagehub_work_ui_dv_runs/`（`case*_cards.jsonl` 每次变化的完整应答、`case*_work_cards_final.json`、`case*_taskmgr_tree.txt`、`real/` 与 `real/plan/` 截图、`mock/` 截图）。

1. simple（“建工作会话并行跑 `sleep 45` / `sleep 70`”）：意图分析阶段气泡是旧任务区；建出 `work-20261102-3`（标题“并行运行两个慢命令”）后变成卡片。行 2 依次是 `working (work_simple)` → 模型写的 intent“并行启动命令 A，等待实际完成输出。”→“并行启动命令 B…”→ Turn 之间显示该 Turn 的回复（“两个命令已在本工作会话目录中并行启动，目前均在运行…”）→ 新 Turn 的 intent → 结束句。token 从第一个 Round 起就在涨（7.9k / 156 → 52.6k / 850，6 Round），模型 `gpt-6.1-sol:reasoning-medium@codex-proxy`，卡片显示 `gpt-6.1-sol`。结束后 ✓、时间停在 2m11s（`finished_at_ms − created_at_ms` = 131 s）、`complete = true`，轮询停止。TaskMgr 里 work session 的 3 个 Turn task 名都是标题。点卡打开整个 session 的连续日志（多个 Turn 的 `turn_started / turn_ended` 分隔）与「工作内容」。
2. plan（“plan_simple 分三个 Todo 依次完成”）：卡片“三步顺序执行与总结”+ TodoList。Todo 未派发时只一行，派发后变成 2 行 Todo 卡（排队中 → 运行中 → ✓），T01→T02→T03 依次执行；plan 卡 token = 自身 116.2k / 1.5k + Todo 合计 = 204.9k / 3.7k（17 Round，tooltip 拆分），plan 自己的模型是 `gpt-6-astra`（`llm.plan`），Todo 是 `gpt-6.1-sol`。结束后 TodoList 自动收起，手动展开后记住。TaskMgr：plan 的 Turn task 名是 plan 标题，各 Todo session 的 Turn task 名是 Todo 标题。
3. Playwright 真实 Zone（`tests/e2e/real/messagehub-agent-task.real.spec.ts`，`MESSAGEHUB_REAL_E2E=1 MESSAGEHUB_REAL_ZONE_IP=127.0.0.1 BUCKYOS_TEST_AGENT_DID=did:web:xiaobai.test.buckyos.io AGENT_TASK_TREE_MSG=<锚点> WORK_CARDS_SHOTS=<dir>`）：两个锚点各 3/3 通过（卡片替代任务区、`complete`、模型名不是别名、点卡看整段日志与「工作内容」、返回 Turn 日志、消息详情页签；light / dark × 桌面 / 手机截图，手机宽度只留输出 token、无横向滚动）。
4. DV 修掉的问题：结束卡片显示 `# Report — <objective>`（8.2-3）；模型名 `:reasoning-medium` 后缀（8.2-7）；网关间歇 `503 service_not_ready` 时已结束 Turn 永远停在旧任务区（8.2-8，默认 4 次、3 / 6 / 12 s 退避）；Markdown 标题作为一句话时带 `##`（去掉标题符号）。
5. 坑：`debug_jarvis` 托管期间网关的 `SERVICE_INFO` 会间歇不含这个 Agent（503 `service_not_ready`，持续数秒到二十秒），容器形态不会这样；Zone 里 memory maintenance 的 goal session 一直是 `created`，`test_agent_chat_task.ts` 的“全部 session 空闲”判断永远不成立，只能等 `OPENDAN_FOLLOW_S` 到期；`pkill -f` 会匹配到自己的 shell，杀 opendan 用 `kill -INT $(pgrep -x opendan)`。

### 8.5 未做与风险

- xllm 接手的 run：工具在 `XllmRun` 内派发，不经 `SessionToolManager`，拿不到 intent，`one_line_status` 停在接手前那句；它的推理只记在 run.json，卡片 token 少算（第一刀已知缺口）。
- 每次工具调用一次 TaskMgr update；state 提交 1 s 节流；`semi:session:<sid>` 订阅者在对方运行中会看到更多版本变化（只进背景，不成为输入）。
- 容器形态未验：DV 用 `debug_jarvis` 宿主托管；容器里的 opendan / xagent 需 `./build_aios --local-test` 重建镜像后才有本单功能，旧镜像的 Agent 不认识 `session.work_cards`，Desktop 退回旧任务区（mock 已测这条降级）。
- Goal UI 不做（Q9）；群聊 / 非 owner 观察者被拒时退回旧任务区只在 mock 与 opendan 测试里验过。
- Desktop 未跑：upload、voice-emoji、extensions 三组 Playwright 与仓库级 `pnpm lint`；`messagehub-guest.test.ts` 7 个 deno 失败与本单无关。

