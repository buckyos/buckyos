# OpenDAN Goal Session TODO

日期：2026-10-11

状态：第一段已实施（2026-10-11），测试与 DV 记录见 §11；第二段（§9）尚未细化，等第一段验收。

本文分两段：

- **第一段**（§1—§7）：把 Goal Session 的框架跑起来并验证。这是现在要实施的全部内容。
- **第二段**（§9）：基于验证过的 Goal Session 改造现有业务，包括 workflow 计划任务接入、Memory 整理 Goal 与删除 self_improve、self_check 退役等。第一段验收通过后，再把第二段细化成任务；在此之前**不要实施**第二段。

修订记录：

- 2026-10-11 初稿。
- 2026-10-11 按用户对 §8 的答复改版。
- 2026-10-11 按用户要求改为两段式，先聚焦第一段。
- 2026-10-11 第一段实施完成，§11 写入实施记录。

目标（第一段）：引入 **Goal 对象**和 **Goal Session**，使一个 Goal 能跨多次运行机会推进。每次 Activation 在预算内观察、评估，然后决定不行动、直接行动或创建 Work Session；Work 的结果在后续 Activation 中吸收，Goal 本身长期存在。验证用一个通用的 Goal behavior 和测试 Goal，不涉及现有业务。

依据：

- [OpenDAN Agent Session 架构设计](<../doc/opendan/OpenDAN Agent Session架构设计.md>)：§7、§11.2—§11.3、§11.6—§11.7、§12.1—§12.5、§12.12、§14.1—§14.5、§15.2 阶段三、§16.1 中的 Goal 验收行。
- [Agent State Protocol](<../doc/opendan/protocol/Agent State Protocol.md>)、[Session Directory Protocol](<../doc/opendan/protocol/Session Directory Protocol.md>)、[Session Input Protocol](<../doc/opendan/protocol/Session Input Protocol.md>)、[Session Control Protocol](<../doc/opendan/protocol/Session Control Protocol.md>)、[xAgent](../doc/opendan/xAgent.md)。
- [Agent 通知人 TODO](opendan-agent-notify-todo.md)：占位，方案待讨论。

读者：实施本文的 code agent。路径相对 `buckyos/`。行号是 2026-10-11 的源码，实施前先核对。

---

## 0. 结论先行（第一段）

| 事项 | 决定 | 依据 / 理由 |
|---|---|---|
| Goal 与 Goal Session | Goal 是 Agent State 中的持久对象，是唯一的真相。同一时刻一个 Goal 只有一个活动的 Goal Session，按 **epoch** 确定性派生；重新打开或开新上下文时 epoch 加一，旧 Session 及其记录存档 | 架构 §7.1；用户答复 5 |
| Session 类型 | 新增 `SessionKind::Goal`（id 前缀 `goal`）和内建模板 `goal`。`self_improve` / `self_check` 第一段不动 | 先跑通框架，不碰业务 |
| Activation | 1 个 Activation = Goal Session 的 1 个逻辑 Turn。Turn 打开时冻结 Goal 版本并装配 Goal 头；Turn 关闭时写 Activation 记录。每个 Activation 一个 TaskMgr turn task（现有机制） | 用户答复 3 |
| Activation 之间的状态 | `run_state = waiting`、`waiting_for = event`（复用从未设置过的 `WaitingKind::Event`） | 架构 §12.12.2 |
| Session 的建立 | 宿主按 Goal 列表补建（reconcile）。创建 Goal 只写 Goal 对象 | 一个创建者、幂等，崩溃可自愈 |
| 运行机会 | 手动 activate；周期定时（第一段用 libopendan 内的**本地计划后端**，放在 `ScheduleBackend` 抽象后面）；Work 结束；UI 转交的反馈。Goal Session 不自我唤醒 | 生产定时在第二段换成 workflow 计划任务（§9.1），Goal 核心代码不变 |
| 结果提交 | 复用 H4 的 `report`。在 Goal Session 中 `is_end` 表示“结束本次 Activation”，`result` 按 Goal schema 校验后写回 Goal State | 组合优于发明 |
| 失败语义 | 预算耗尽、WAIT_USER_MSG、出错都只关闭本次 Activation，不结束 Goal Session。archive 或达成时 Session 才结束，用户可以重新打开 | 架构 §7.3、§12.2 |
| 预算 | 单次 Activation（跨多个 run 累计）为硬约束；周期用量做统计，并在周期上限处拒绝定时 fire 和创建 Work | 架构 §12.2—§12.3 |
| 派生 Work | 复用 `create-worksession`（不加 `--wait`）。Work 的 `origin` 带 goal 引用；Goal Session **按 `goal_id`** 吸收 Work 结果，所以新 epoch 能接管旧 epoch 的 Work | 架构 §7.5—§7.6；用户答复 5 |
| 通知人 | 不实现。questions 只记录在 Goal State 中 | 用户答复 2 |
| 写入者 | Goal 定义：GoalStore（修订号 CAS）。Goal State 与 Activation 记录：Goal Session 驱动者。计划进度：宿主 | 单写者 |
| 权限 | 只按调用方类型作简单区分（§3.7） | 先能力后安全 |
| llm_context | 不修改 | run 级 `BudgetSpec.max_total_tokens` 足以表达剩余额度 |
| 协议版本 | 只是增加，不升 `session_config` / `session_state` 主版本 | `doc/opendan/protocol/README.md` |

---

## 1. 第一段范围

### 1.1 做

- Goal 对象：定义、修订、Goal State、Activation 记录、epoch 历史、计划进度；`AgentStateClient::goals()` 文件版与 kRPC 版。
- Goal Session：kind / 模板 / 按 `(goal_id, epoch)` 派生 sid / 宿主补建 / Goal 头 / Activation 结束语义 / 写回。
- 运行机会：手动 activate、本地周期定时（`ScheduleBackend` 抽象 + `LocalScheduleBackend`）、Work 结束、UI 反馈。
- 预算：单次 Activation、周期用量与上限。
- 派生 Work：goal 引用、按 `goal_id` 吸收、跨 epoch 接管。
- 状态控制：pause / resume / archive（级联停止 Work）/ achieved / reopen / restart、取消单次 Activation。
- 管理面：kRPC `goals.*`、`loader.status` 的 goals 模块、`xagent goal` 与 `agent-session goal` CLI。
- Jarvis：`[session.goal]` 与一个通用 `behaviors/goal.toml`，用于验证。
- 协议与文档反写、fixture、测试、DV 验证。

### 1.2 第一段不做

| 事项 | 去向 |
|---|---|
| workflow 计划任务接入（cron、时区、一次性、`opendan.command` 消费）以及 workflow 的修复 | §9.1 |
| Memory 整理 Goal（默认打开）、删除 self_improve 旧路径，以及为它需要的闸门、Agent 级锁、不写感知等扩展点 | §9.2 |
| self_check 退役；Notebook / attention signal 退役 | §9.3 |
| WebUI Goals 视图 | §9.4 |
| follow_up（Goal 请求“到某时再看”）、kevent 对象订阅、结晶类 Goal、意图分析扩展 | §9.5 |
| Agent 通知人 | [通知 TODO](opendan-agent-notify-todo.md) |
| Work 收紧（Task 就绪检查、Task 修订、每次推理前观察） | 架构 §6、§15.5，可并行 |

第一段的数据结构和接口不要把第二段堵死：Goal 定义要可扩展（新增字段属于同主版本内的增加），计划后端用 trait 隔开，Activation 准入留出挂点（§3.5）。

---

## 2. 当前基础与缺口（第一段相关）

| 入口 | 已有基础 | 缺口 |
|---|---|---|
| `src/frame/lib_opendan/src/protocol/config.rs:21-49` `SessionKind` | `ui / work / self_improve / self_check` | 没有 Goal |
| `src/frame/lib_opendan/src/template.rs:47-112` | 内建模板，`[session.<class>]` 可覆盖 | 没有 Goal 模板；`wait_user_msg` 缺少“只结束本次 Turn、Session 继续等待”的语义 |
| `src/frame/lib_opendan/src/ids.rs:30-58` | `sc-H("self_check", agent_did)` 单例派生；`api.rs:293-298` 要求幂等键和身份匹配 | 没有按 `(goal_id, epoch)` 的派生 |
| `src/frame/lib_opendan/src/api.rs:534-691` `create_sub_session` | 幂等，有并发上限 `max_sub_sessions`（默认 4）和深度上限 `max_session_depth`（默认 2），带 `origin`，可继承 Workspace | `origin` 没有 goal 引用；没有周期准入 |
| `src/frame/lib_opendan/src/runner/children.rs` | `poll_children`（151-251）只按 `origin.parent_session` 拉取，合成 `_child` 输入；`summary_of`（105-131） | 跨 epoch 接管需要按 `goal_id` 拉取，投递进度要放进 Goal State |
| `src/frame/lib_opendan/src/runner/reports.rs:110-128` | `report(is_end)` 在有活动 task 或子 Session 未收敛时拒绝 | Goal 结束 Activation 时要放行未收敛的 Work |
| `src/frame/lib_opendan/src/runner/outcome.rs` | `decide_end`（117-140）、`fail_unattended`（193-198）、`classify_done`（214-318）、`hold_for_children`（350-366） | Activation 失败会结束 Session；没有写回挂点 |
| `src/frame/lib_opendan/src/runner/drive.rs` | 主循环，受控输入批次，无事可做时标 Waiting/Input（1509-1524） | 没有 Goal 头和 Goal 状态闸门；没有 Activation 累计预算 |
| `src/frame/lib_opendan/src/host.rs` | `serve_one`（484-558）；`Supervisor::tick`（794-863）每 30s recheck 有队列的已卸载 Session；`ensure_task`（758-776） | 没有 Goal 补建、计划后端和 `goal_fire` 入口 |
| `src/frame/lib_opendan/src/bridge/timer.rs` | 周期 tick，不持久，卸载即停 | Goal **不使用**它 |
| `src/frame/lib_opendan/src/runner/turn_task.rs`、`src/frame/opendan/src/tasks.rs` | 每个 Turn 一个 TaskMgr task；`CancelBridge`（378-451）把取消转成 `stop`，会结束整个 Session | 取消一次 Activation 不应结束 Goal |
| `src/frame/lib_opendan/src/state/mod.rs:173-191`、`state/krpc.rs::serve_call`（125-285） | AgentStateClient 门面与 kRPC；driver-only 写在 kRPC 上拒绝 | 没有 `goals()` |
| `src/frame/opendan/src/loader.rs`、`service.rs` | Supervisor 托管、`loader.status`、`/kapi/opendan` | 没有 Goal 模块 |
| `src/apps/jarvis_runtime/agent/` | `[session.ui/group/work]`、现行结构的 behavior（例如 `task_route.toml`） | 没有 `[session.goal]` 和 Goal behavior |

---

## 3. 模型与契约（第一段）

### 3.1 文件布局（AgentRoot）

```text
<agent_root>/state/goals/<goal_id>/
  goal.json              当前定义 + 状态 + 当前 epoch。schema "opendan.goal/1"。写者：GoalStore（修订号 CAS，原子替换）
  revisions/<rev>.json   每个内容修订的不可变快照
  changes.jsonl          所有变更：内容修订、状态变化、reopen、restart。含 who / via / at / reason
  state.json             Goal State。schema "opendan.goal_state/1"。写者：当前 epoch 的 Goal Session 驱动者
  activations.jsonl      Activation 记录（带 epoch）。schema "opendan.goal_activation/1"。写者同上，按 (epoch, seq) 幂等追加
  epochs/<n>/state.json  restart / reopen 时存档的上一 epoch 的 Goal State
  schedule.json          计划进度。schema "opendan.goal_schedule/1"。写者：宿主
<agent_root>/sessions/<goal sid>/   各 epoch 的 Goal Session 目录；旧 epoch 为已结束状态，只读保留
```

- Goal Session 的 `session_config` 只保存 `session.goal = {goal_id, epoch}`，不复制目标定义。
- 登记表条目增加 `goal_id`（Goal Session 与 Goal 派生的 Work 都写），`RegistryQuery` 增加 `goal_id` 过滤。

### 3.2 字段草案（命名可调，语义不变）

**Goal 定义 `goal.json`**

| 字段 | 说明 |
|---|---|
| `goal_id` | `g-<yyyymmddThhmmss>-<uuid8>` |
| `revision` | 内容修订号。状态变化、reopen、restart 都不增加它 |
| `title`、`objective`、`scope{objects[], paths[], workspace?}` | 目标与作用范围 |
| `lifecycle` | `achieve` 或 `maintain` |
| `achieved_when` / `healthy_state`、`distance_evaluation`、`constraints[]`、`initial_skills[]` | 架构 §7.2 |
| `owner`、`created_by{principal, via}`、`beneficiaries[]` | via 取值：`owner / ui_session:<sid> / opendan` |
| `budget` | `activation{max_total_tokens, max_tool_iterations?, max_runs?}`；`period{window_secs, max_total_tokens?, max_activations?, max_works?}`。缺省取 `[session.goal]` |
| `schedule` | `{every_secs?, min_interval_secs, start_on_create}`。缺省不定时。cron / 时区 / 一次性在第二段随 workflow 加入 |
| `behavior` | 可选。覆盖 `[session.goal] default_behavior` |
| `status` | `active / paused / achieved / archived`，以及 `status_changed_at_ms / status_reason / status_by` |
| `session` | `{epoch, sid}` |
| `epochs[]` | `{epoch, sid, started_at_ms, ended_at_ms?, end?: archived / achieved / restarted, reason?}` |

**Goal State `state.json`**（驱动者写，`rev` 单调）：

- `epoch`、`goal_revision_seen`；
- `last_activation{epoch, seq, at_ms, reasons[], outcome, decision, summary}`；
- `distance{text, observed_at_ms, evidence_refs[], unknowns[]}`；
- `pending[]`、`questions[]`（只记录，不投递）；
- `active_works[]{sid, objective, epoch, created_in_activation, goal_revision}`；
- `absorbed[]`（已吸收 Work 的 sid，有上限，用于跨 epoch 去重）；
- `recent_works[]{sid, outcome, one_line_status, report_ref, absorbed_in}`；
- `current_skills[]{name, ref, version?, note?}`；
- `usage{period_start_ms, tokens, activations, works, work_tokens}`；
- `last_error`。

**Activation 记录**：

- `epoch`、`seq`（等于该 epoch Session 的 turn seq）、`goal_revision`、`started_at_ms / ended_at_ms`；
- `reasons[]`：`{kind: created / scheduled / child / feedback / manual / reopened / restarted, key, ref?, missed?}`；
- `outcome`：completed / failed / budget_exhausted / stopped / needs_input；
- `decision`：`no_action / acted / created_work / needs_info / unknown`；
- `summary`、`report_ref`、`works_created[]`、`cost{runs, total_tokens, tool_iterations}`、`end_reason`。

**计划进度 `schedule.json`**：`backend`（第一段为 `local`）、`last_slot`、`last_posted_key`、`skipped[]`（最近若干次跳过的原因：paused / period_budget / min_interval）、`last_error`。

### 3.3 Activation 的输入与上下文

- **Goal 头**：每个 Activation 的第一个受控输入批次之前，Runtime 装配一条 Goal 头消息，内容为：
  - 当前定义（修订号，有变化时附变更摘要）；
  - Goal State 摘要；
  - active_works 及其在登记表中的最新状态，并标出每个 Work 依据的 Goal 修订号；
  - current_skills 列表；
  - 本次预算与周期用量；
  - 本次原因（包括 reopen / restart 的理由）。
- 外部读取先完成并冻结。Goal 头与本批输入同一次提交（receipt），恢复时不重复追加（架构 §11.8）。
- 实现方式：新增模板槽（建议 `prompt.goal_activation`，内建默认模板，可替换）和模板视图 `goal`。Goal 头由 Runtime 保证装配，不依赖 `on_input` 引用它。
- 历史：旧的 Goal 头在历史渲染中只保留一行引用（修订号 + Activation 序号）。
- 输入来源：
  - 定时与手动 activate：Runtime 投递 `AgentEvent`，`source = system:goal`，`event = scheduled / activate / created / reopened / restarted`。由模板的隐式 Active 订阅 `_goal_activation`（`System{id:"goal"}`）接收。不要用 `Timer` 来源，否则 TimerBridge 也会按缺省 300s 投递。
  - Work 结束：`_child` 输入（来源扩展见 §3.8）。
  - UI 反馈：来自 Agent 自身的 `msg`，meta 记录来源 UI sid。
- 积压的多条原因合成一次 Activation 的输入批次。

### 3.4 Activation 的结束语义

| 情况 | 本次 Activation（Turn） | Goal Session | Goal State / 记录 |
|---|---|---|---|
| `report(is_end=true)` 或 `<report end="true">` 被接受 | Completed | 回到 waiting / event | `result` 校验后写回 |
| 模型 Done 但没有最终 report | Completed | 同上 | decision = `unknown`，记警告 |
| `is_end` 时有未收敛的派生 Work | **放行** | 同上 | Work 留在 active_works |
| 本 run 有 PendingTool | 照旧拒绝 `is_end` | — | — |
| WAIT_USER_MSG | needs_input | 同上 | 写入 questions，不投递 |
| 预算耗尽 | budget_exhausted | 同上（不走 `fail_unattended`） | 记录未完成原因 |
| 不可恢复的错误 | failed | 同上 | `last_error`，下次照常 |
| 取消本次 Activation | stopped | 同上 | 记录原因 |
| archive / achieved | 在可提交边界结束 | finished（Stopped / Succeeded） | 最终记录；epoch 记录 end |
| restart | 在可提交边界结束 | 旧 Session 以 superseded 结束（**不级联停止 Work**），新 epoch 接手 | §3.8 |

- Goal `result` schema（Runtime 校验，不合格时让 `report` 调用返回错误）：`{decision, distance?, pending?, questions?, skills?: {add?, remove?}}`。
- `no_action` 是合法结果，不等于达成。

### 3.5 运行机会与准入

- **计划后端抽象**：`trait ScheduleBackend { ensure(goal_id, spec) -> ScheduleRef; pause; resume; archive; }`。fire 统一进入 `host::goal_fire(goal_id, key, reason)`，在那里依次做准入、`post_input`（用稳定 key）、`ensure_task`。
- **第一段的后端**：libopendan 的 `LocalScheduleBackend`，供 Loader（zone 与 `--dev`）、`xagent` 和测试共用。
  - 只支持 `every_secs`，由 Supervisor tick 驱动；
  - `slot = floor(now / every_secs)`，`last_slot < slot` 时投递一次 `scheduled`；
  - 停机期间错过的多个槽**合并为一次**（`missed = n`）；
  - 先写 `schedule.json` 的待投递记录再 post，重启后补投同一个 key。实施前核对 `channel/` 与 receipt 对重复 key 的去重语义。
  - 时钟可注入，便于测试。
- **手动 activate**：kRPC `goals.activate` 和 CLI，key 为 `goal:<goal_id>:activate:<request_id>`。
- **准入**（只作用于定时 fire，不唤醒、不调用 LLM）：Goal 状态为 active；距上次 Activation 不少于 `min_interval_secs`；周期上限未到。不通过时记入 `schedule.skipped`。
- **挂点**：准入写成可扩展的检查链。第二段会在这里加入廉价闸门（如 Memory 的 `pending_work`），第一段只实现上面三项。
- **补建**：对 active 或 paused 的 Goal，如果 `goal-H("goal", agent_did, goal_id, epoch)` 不存在，就用幂等键 `goal:<goal_id>:<epoch>` 创建 Goal Session（目录 `<agent_root>/sessions/`，driver 为宿主身份，**没有 parent**）。`start_on_create` 时投递 `created`。
- **Work 结束**：Goal Session 有队列，卸载后由 Supervisor recheck（≤30s）重新装载，第一段接受这个延迟。

### 3.6 预算

- **单次 Activation**：Turn 打开时在 state 中记录用量基线（取自 `static.json`）。每次新建或恢复 run 前计算剩余额度：
  - 剩余 ≤ 0：关闭 Activation，结果为 budget_exhausted；
  - 否则把 run 的 `max_total_tokens` 设为 `min(behavior budget, 剩余)`。
- **周期**：按 `period.window_secs` 滚动。Activation 关闭时累加本次消耗；Work 被吸收时累加它的总量。达到上限后，定时 fire 被准入拒绝，Goal Session 创建 Work 被拒（`goal_budget_exhausted`）；事件型 Activation 仍可运行，以便吸收结果。
- Work 的并发上限按 `goal_id` 下未结束的 Work 计数（跨 epoch）。
- restart / reopen 不重置周期用量。
- Goal 头展示剩余额度。

### 3.7 写入者与权限（只按调用方类型区分）

| 调用方 | 可做 |
|---|---|
| owner（kRPC / `xagent goal`） | 全部：create / revise / set_status / reopen / restart / activate / post |
| UI Session（`agent-session goal ...`） | create；revise 自己创建的 Goal；pause / resume / archive / reopen / restart / activate / post |
| Goal Session（自己的 Goal） | 读；经 Activation 结果写 distance / pending / questions / skills；lifecycle 为 achieve 时标记 achieved（须附证据，且没有未收敛 Work）；归档自己（须附理由）。不能 reopen / restart，也不能改定义、预算或计划 |
| Work Session | 只读 origin 中的 Goal 摘要 |

- Goal 自己做的 archive / achieved 记入 `changes.jsonl`（`via = goal_session`），用户可以 reopen。
- kRPC 上，Goal State、Activation 记录和 `schedule.json` 属于 driver-only，外部写入一律拒绝。

### 3.8 状态、epoch 与 Work 接管

- **paused**：
  - 计划被暂停；
  - drive 的 Goal 状态闸门不开新 Activation，输入留在队列；
  - 正在进行的 Activation 跑到自然结束，也可以单独取消。
- **resume**：积压的输入合成一次 Activation。
- **archived**：
  - 计划被归档；
  - 当前 Session 以 Stopped 结束；
  - 仍在运行的派生 Work **级联停止**（复用 `stop_children`，并补上按 `goal_id` 找到的旧 epoch Work）。
- **achieved**：只在没有未收敛 Work 时允许；当前 Session 以 Succeeded 结束。
- **reopen**（achieved / archived → active）：
  - epoch 加一，补建新 Session；
  - Goal State 默认保留（可选 `fresh_state`）；
  - 第一次 Activation 的原因为 `reopened`，附理由。
  
  旧 Session 不会 reopen。
- **restart**（active / paused）：开全新上下文的 Goal Session。
  1. 当前 Activation 在可提交边界结束；
  2. 旧 Session 以 superseded 结束，**不级联停止 Work**。需要给 stop 控制加不级联的选项，或由宿主在空闲边界直接提交；
  3. 旧 Goal State 存档到 `epochs/<n>/state.json`；
  4. 新 epoch 默认只保留不可丢失的事实：`active_works / absorbed / usage / recent_works`。其余清空，current_skills 回到 initial_skills。可选 `carry_state`；
  5. 第一次 Activation 的原因为 `restarted`。
  
  旧 epoch 的 Activation 记录保留，按 epoch 区分。
- **Work 接管**：Goal Session 拉取 `children_of(self)` ∪ 登记表中 `goal_id = 本 Goal` 且不在 `absorbed` 中的 Work。投递进度以 Goal State 的 `absorbed` 为准，新 epoch 不重复吸收，也不漏掉。
- **revise**：运行中的 Work 保留创建时的 `origin.goal.revision`；下一次 Goal 头标出变化和受影响的 Work。

---

## 4. 第一段实施 TODO

### G1：协议类型与模板

- [ ] `protocol/config.rs`：
  - `SessionKind::Goal`（`goal / goal`）；
  - `SessionSection.goal: Option<GoalBinding{goal_id, epoch}>`；
  - `Origin.goal: Option<GoalOrigin{goal_id, revision, epoch, activation_seq}>`；
  - WaitPolicy 增加“只结束本次 Turn、Session 继续等待”的变体（如 `end_turn`）。
- [ ] `template.rs`：内建 `goal` 模板为 kind Goal、Unbounded、Queue、`end_turn`、`natural`、`EventsAndActive`、`load_hints = true`，带隐式 Active 订阅 `_goal_activation`（`System{id:"goal"}`）。`[session.goal]` 可覆盖，并新增预算缺省值。
- [ ] `ids.rs`：`goal-H("goal", agent_did, goal_id, epoch)`。
- [ ] `protocol/agent_state.rs`：`RegistryEntry.goal_id`、`RegistryQuery.goal_id`；`GoalDefinition / GoalState / GoalActivation / GoalSchedule` 与 schema 常量。
- [ ] `protocol/state.rs`：Activation 基线与原因；两次 Activation 之间设 `waiting_for = event`；写回待处理标记。
- [ ] 检查所有 `SessionKind` 的 match 与过滤（`drive.rs:1118`、`outcome.rs:1012`、`state/perception.rs:383`、`memory/consolidation.rs:141`、`opendan/src/config.rs:151` 附近、WebUI `types.ts`），确保 Goal 不会误走 self_improve 或 ui 的分支。

### G2：GoalStore

- [ ] `state/goals.rs`：`trait GoalStore`。
  - 外部操作：list / get / revision / changes / create（幂等键）/ revise（expected_rev）/ set_status / reopen / restart / state / activations / schedule；
  - 只给驱动者和宿主用的写操作：`write_state / append_activation / archive_epoch_state / write_schedule`。
  
  在 `AgentStateClient` 上加 `goals()`。
- [ ] 文件版按 §3.1 实现，原子写，CAS 冲突返回明确错误。
- [ ] kRPC：`state/krpc.rs::serve_call` 增加 `goals.*`，同步 `is_write`、`driver_only`；`state/connect.rs` 跟进。

### G3：补建、epoch 与状态迁移

- [ ] `api.rs`：`ensure_goal_session(agent, goal, epoch, who, channels)`：确定性 sid、幂等键 `goal:<goal_id>:<epoch>`、`session.goal`、无 parent、`via = opendan`、目录在 `<agent_root>/sessions/`。
- [ ] `host.rs`：Supervisor tick 中补建；处理状态和 epoch 迁移（archive / achieved 时结束 Session；reopen / restart 时结束旧 Session、存档 state、补建新 Session、投递 `reopened / restarted`）。
- [ ] 不级联的结束方式（restart 用），同步 Session Control Protocol。

### G4：Activation 输入装配

- [ ] `runner/assembler.rs` / `input_view.rs`：`goal` 视图与 `prompt.goal_activation` 内建模板（XML 风格，与现有 `render_*_xml` 一致，正文不能伪造结构）。
- [ ] `runner/drive.rs`：打开 Turn 时冻结 Goal 定义、State、预算并渲染 Goal 头，随批次提交；恢复路径不重复装配；paused 时的状态闸门。
- [ ] 旧 Goal 头的历史压缩。
- [ ] Goal Session 写出的感知带 `source_ref = goal:<goal_id>`（`protocol/agent_state.rs:178` 已预留这个语义）。

### G5：Activation 结束、写回与记录

- [ ] `runner/reports.rs`：在 Goal Session 中，`is_end` 放行未收敛的 Work；按 schema 校验 Goal `result`。
- [ ] `runner/outcome.rs`：对 Goal kind，`decide_end` 只关闭 Turn 并进入等待；`fail_unattended` 与 `end_turn` 都不结束 Session；预算耗尽和错误只关闭 Turn；`hold_for_children` 不作用于 Goal。
- [ ] 写回：Turn 关闭的同一次 state 提交中记下“待写回”标记，然后写 Goal State、按 (epoch, seq) 幂等追加 Activation 记录，再清除标记。仿照 `flush_turn_tasks` / outbox 的模式，崩溃后补写。
- [ ] 故障注入点（`fault.rs`）：Turn 已关闭、写回之前；记录已追加、标记清除之前；restart 中旧 Session 已结束、新 Session 尚未创建。

### G6：预算

- [ ] Activation 基线、run 前的剩余额度；周期滚动与上限（接入 `goal_fire` 准入与 `create_sub_session`）；Goal 头展示剩余额度。

### G7：运行机会

- [ ] `ScheduleBackend` trait、`LocalScheduleBackend`（可注入时钟）、`host::goal_fire`（准入链、稳定 key 投递、`ensure_task`）。
- [ ] 手动 activate；UI 反馈 post（以 Agent 自身的 msg 形态投递）。

### G8：派生 Work

- [ ] `create_sub_session`：父为 Goal Session 时，填 `origin.goal` 与登记表 `goal_id`，检查周期上限，并按 `goal_id` 计算并发上限。Work 缺省 `report = final`。
- [ ] `runner/children.rs`：Goal Session 拉取 `children_of(self)` ∪ `query(goal_id)` 中不在 `absorbed` 的 Work。`_child`（finished）提交时更新 active_works / absorbed / recent_works 与用量。
- [ ] Work 的 report.md 与 `state.result` 标识带 goal 引用。

### G9：状态控制与取消

- [ ] `set_status / reopen / restart` 的联动（§3.8），包括 archive 级联停止、achieved 的前置检查。
- [ ] 取消单次 Activation：Goal Session 的 TaskMgr 取消（`opendan/src/tasks.rs::CancelBridge`）和 kRPC `session.stop` 只结束当前 Turn。可以给 stop 加 `scope = turn`，同步 Session Control Protocol。

### G10：CLI、Loader 与 Jarvis

- [ ] `xagent goal`：
  - `create --agent ... --objective ... [--lifecycle] [--every <secs>] [--budget ...] [--key]`；
  - `list`、`show <id> [--state] [--activations n] [--revisions] [--epochs]`；
  - `revise`、`pause | resume | archive`、`reopen <id> --reason [--fresh-state]`、`restart <id> --reason [--carry-state]`；
  - `activate <id> [--reason]`、`post <id> --msg`。
- [ ] `agent-session goal`：按 `$OPENDAN_SESSION_ID` 的 kind 判断权限（§3.7）。
  - UI 用 create / list / show / post / pause / resume / archive / reopen / restart / activate；
  - Goal Session 用 show、`skills add|remove`、`achieved --evidence`、`archive --reason`。
- [ ] `xagent new --class goal` 拒绝直接创建，并提示改用 `xagent goal create`。
- [ ] Loader：
  - `opendan/src/config.rs` 增加 `[loader.goals] enabled`（缺省开启：没有 Goal 时零开销）；
  - `loader.rs` 接入补建与本地计划后端，`loader.status.modules` 增加 `goals`（数量、active、paused、最近 fire、跳过与错误）；
  - `service.rs` 转发 `goals.*`，写操作后 `ensure_task`；README 的 kRPC 契约同步。
- [ ] Jarvis 包：
  - `agent.toml` 增加 `[session.goal]`（`default_behavior = "goal"`、预算缺省值）。
  - 新增 `behaviors/goal.toml`（现行结构，`function_call`）。系统提示按架构 §7.5 写“观察 → 判断 → 按预算决定”。可用命令有 `agent-session goal show`、`sessions --children`、`read-session --report`、`create-worksession --report final`（不加 `--wait`），以及 `result` 的格式。明确：允许不行动；不要把整个目标交给 Work；需要人决定时写 questions。
  - `task_route` / `chat_route` 不改。

### G11：协议、文档与 fixture（随实现同步）

- [ ] `Agent State Protocol.md`：布局加 `state/goals/`，新增 Goals 一节（文件、写者、epoch、kRPC），登记表 `goal_id`。
- [ ] `Session Directory Protocol.md`：kind goal、`session.goal`、`origin.goal`、`waiting_for = event`、Goal 头与写回标记。`Session Input Protocol.md`：`system:goal` 事件与 key。`Session Control Protocol.md`：stop 的范围或不级联选项。
- [ ] `README.md` 的版本说明；用 `cargo run -p libopendan --bin xagent -- schema <dir>` 重新生成 schema，新增 `goal*.schema.json`。
- [ ] 新 fixture `16_goal_session`：Goal 存在、Goal Session 处于 waiting / event、有一个已结束的派生 Work、有待写回标记。用 `cargo run -p libopendan --example fixtures -- <绝对路径>/doc/opendan/protocol/fixtures` 生成，`tests/fixtures.rs` 断言。
- [ ] `doc/opendan/xAgent.md`（CLI 与模板表）、`src/frame/opendan/README.md`（kRPC 与 `loader.status`）。
- [ ] 架构设计 §15.5 / §15.6：补“Goal 的现有实现”行。§17.2 中“Goal 与 Goal Session 的绑定”标注当前实现。

### G12：验证（第一段的退出条件）

- [ ] 测试：§6 的命令全部通过，§5 的验收逐条有对应用例。
- [ ] DV：用 `./debug_jarvis.sh devtest --agent <agent_id>` 托管 Agent，然后：
  1. 用 `xagent goal create --every 300` 或 kRPC 创建一个测试 Goal，例如“每 5 分钟检查 Workspace X 的 `TODO.md`，有未完成项就创建一个 Work 处理其中一项，处理完把该项勾掉”。
  2. 观察 Activation 记录、Work 的创建与吸收、no_action 轮、pause / resume、archive 后 reopen、restart 后新 epoch 吸收旧 Work、重启 Loader 后错过的槽只补一次。
- [ ] 结果、命令与发现的问题写入 §11。第一段验收通过后，再细化 §9。

---

## 5. 第一段验收矩阵

| 编号 | 场景 | 必须观察到的结果 |
|---|---|---|
| V01 | 创建 Goal，重复同一幂等键创建 | 只有一个 Goal、一个 Goal Session（sid 由 goal_id 与 epoch 确定），没有 parent |
| V02 | 宿主在写入 Goal 后、创建 Session 前崩溃 | 下一次 tick 补建，不重复 |
| V03 | 定时到点 | 一次 `scheduled`，Activation 打开，Goal 头完整；结束后 waiting / event |
| V04 | 宿主停机期间错过 3 个槽 | 恢复后只有一次 Activation，`missed = 3` |
| V05 | 本轮无事可做 | `no_action`，Goal 仍为 active，下次定时照常 |
| V06 | Activation 预算耗尽 | budget_exhausted，Session 未结束，下次机会正常 |
| V07 | 周期上限到达 | 定时被跳过并记录；创建 Work 被拒；Work 的结果仍能吸收 |
| V08 | WAIT_USER_MSG | needs_input，questions 写入，不投递，Session 未结束 |
| V09 | Activation 中创建 2 个 Work 后结束；Work 先后结束 | `is_end` 被接受；`origin.goal` 正确；事件型 Activation 吸收结果，用量计入 |
| V10 | Goal 在 Work 运行中被修订 | Work 的 revision 不变；下一次 Goal 头标出变化 |
| V11 | pause 后到点、Work 结束；然后 resume | 不开新 Activation，输入留在队列；resume 后合成一次 |
| V12 | archive 时有 Work 在运行 | Session 以 Stopped 结束，Work 被级联停止 |
| V13 | Goal Session 把自己 archive，然后用户 reopen | `changes.jsonl` 中可见 `via = goal_session`；reopen 后 epoch 加一，State 保留，原因为 `reopened` |
| V14 | Work 运行中执行 restart | 旧 Session 以 superseded 结束，Work 未被停止；旧 state 已存档；新 Session 的上下文是全新的；Work 结束后由新 Session 吸收，且只吸收一次 |
| V15 | Activation 中 kill -9 | 恢复后在同一个 (epoch, seq) 中继续，不重复装配，不重复写记录 |
| V16 | Turn 关闭已提交、写回未完成时崩溃 | 补写一次，不重新推理 |
| V17 | 取消一次 Activation | 只关闭本次 Turn，Session 仍在 |
| V18 | 多次 Activation 后历史被压缩 | 当前 Skills、修订、未完成事项与 active works 都能从 Goal 头恢复 |
| V19 | Goal Session 试图修改定义、预算、计划或执行 reopen | 被拒绝 |
| V20 | `loader.status` / kRPC `goals.*` | 能看到 Goal、State、Activation、epoch；外部写 Goal State 被拒绝 |
| V21 | 现有 UI / Work / self_* 路径 | 回归测试全部通过，行为不变 |

---

## 6. 验证命令（在 `src/` 下）

- `cargo test -p libopendan -- --test-threads=1`：新增 `tests/goal_session.rs`（`ScriptedLlm` + 本地后端 + 假时钟）；崩溃用例按 `tests/crash.rs` 的子进程 + `LIBOPENDAN_FAULT` 手法；CLI 用例放在 `tests/xagent_cli.rs`。
- `cargo test -p opendan -- --test-threads=1`：`tests/loader.rs` 覆盖 Goal 模块、`loader.status.goals`、kRPC `goals.*`。
- `cargo test -p llm_context`、`cargo test -p agent_tool --lib`：应不受影响。
- 协议：重新生成 schema 与 fixtures，`tests/fixtures.rs` 通过（f04 / f06 / f07 / f12 会随时间失败，那时需要重新生成，与本改动无关）。
- `cargo check --workspace`。没有执行的检查要在 §11 中写明。

---

## 7. 第一段内部顺序

| 步骤 | 内容 | 退出条件 |
|---|---|---|
| 1 | G1、G2、G3（不含 reopen / restart）、G4、G5、G6、G7、`xagent goal create/show/activate` | V01—V06、V08、V15、V16、V18 |
| 2 | G8 派生 Work | V07、V09、V10 |
| 3 | G9、G3 的 reopen / restart、G10 其余部分 | V11—V14、V17、V19、V20 |
| 4 | G11 定稿、G12 | V21，DV 记录 |

---

## 8. 决定与待确认

### 8.1 用户答复（2026-10-11）

1. 定时使用系统已有的计划任务能力。调研结论：计划任务单元在 workflow 服务中，`dcrontab` 是它的 CLI。生产接入放到第二段（§9.1）；第一段用本地后端跑通框架。
2. Agent 通知人另写 TODO，暂时留空。本文不实现任何投递。
3. 每个 Activation 一个 TaskMgr turn task：同意。
4. achieve 型 Goal 可由 Goal Session 自行标记 achieved：同意。
5. archive 时级联停止 Work：同意。另外要能重新打开误归档的 Goal，并能开全新上下文的 Goal Session、旧记录存档，用 epoch 实现，属于第一段。
6. 打开感知 → 认知的整理：属于第二段的第一项业务改造（§9.2）。
7. TODO 先聚焦把 Goal Session 框架跑起来，验证后再改业务。

### 8.2 本文的实现决定（如需调整，请在实施前提出）

- 第一段 Loader 也用本地计划后端，第二段换成 workflow 计划任务，之后本地后端只留给 dev 和测试。
- restart 时 Goal State 默认只保留不可丢失的事实；reopen 时默认全部保留。两者都可以用参数反转。
- 第一段的定时只支持 `every_secs`。

### 8.3 已知风险（按“先能力后安全”记录，不加机制）

- Goal 可以无人值守地创建 Work，只靠周期上限、并发上限和提示词约束。
- questions 在通知方案确定之前只能通过 kRPC / CLI 看到。
- 权限只按调用方的 Session kind 区分。
- Goal 头的 token 成本随 Goal 数量线性增长；Supervisor 对有队列的 Goal Session 每 30s recheck 一次，带来 IO 开销。

---

## 9. 第二段：基于 Goal Session 改造现有业务（第一段验收后细化，现在不实施）

以下是已经查到的事实和已经做出的决定，作为细化时的输入。

### 9.1 workflow 计划任务接入（生产定时）

- 计划任务单元在 `src/kernel/workflow`（`scheduled_task_manager.rs`、`server.rs`，kRPC `/kapi/workflow`），记录存在 task-mgr 中。
  - 支持的规格：`ScheduleSpec{Cron{expr, timezone}, Once{run_at}, RunEvery{every_sec}}`；`MisfirePolicy` 缺省 RunOnce。
  - 上一次 fire 的子任务未终结时跳过本次，正好提供停机期间的合并语义。
  - 已有内建目标 `opendan.command`，task-mgr 中登记了 `opendan.command/v1`。
- 映射：`every_secs → RunEvery`；cron 加时区 → `Cron`；follow_up → `Once`。目标为 `opendan.command{command: "goal.activate", args: {goal_id, slot}}`，owner 为 Agent App 实例。OpenDAN 按 `runner_app_id` 列出待执行的 fire task（kevent `/task_mgr/tree/{root}` 只做加速），调用 `goal_fire`，然后完成 task。实现为 `WorkflowScheduleBackend`。
- 需要先修的 workflow 问题：
  1. `opendan.command` 的 fire task 被绑定给 workflow 自己（`create_delegated_app_task` :578-638），没有执行方；
  2. 成功 fire 后用旧的 fire_time 计算下一次（`server.rs:1370`），停机后会逐个补发；
  3. `buckyos-api/src/workflow_service.rs` 的状态枚举和 id 类型与服务端不一致；
  4. `load_schedules` 只读第一页。
- 时区只有硬编码表，完整的 IANA 支持需要 workflow 引入时区库，引入前要确认。
- `dcrontab` 不在 Session 的 PATH 上，owner 格式有问题，归通知 TODO。

### 9.2 Memory 整理 Goal（感知 → 认知，默认打开）与删除 self_improve

- 用户已决定：默认打开，同时删除 self_improve 旧路径。
- 需要在 Goal 上增加的扩展点：
  - 准入挂点上的廉价闸门 `memory_pending`：先 `sweep_expired`；有 `retry_cleanup` 时直接 `cleanup()`；`needs_run()` 时放行；否则以 `next_deadline` 作为 follow_up；
  - Goal 声明的 Agent 级锁（泛化 `drive.rs:1118-1136` 的 kind lease，`CONSOLIDATION_LEASE` 从 `self_improve` 改名为 `memory_consolidation`，注意 Memory TODO §8.5 记录的 lease epoch 风险）；
  - `records_perception = false`：整理 Goal 不写感知，防止自我回声；
  - follow_up。
- 模型入口倾向用 runner 进程内的**宿主工具**（`memory_consolidation_begin / commit`，与 `report` 同类），因为 lease 由 runner 持有。这会取代 Memory TODO 中计划的 `agent-memory commit --plan`。细化时再确认。
- 终局判定：取到了材料却没有实际 commit，decision 记为 `incomplete`，不能只推进游标（TD-03 / TD-19）。
- 要删除的旧路径：
  - `api.rs::create_self_improve_session`；
  - `perception_window`（`drive.rs:1449-1455`、`outcome.rs:1141-1160`、`assembler.rs`）；
  - `outcome.rs:1010-1035` 的游标提交，`state/cognition.rs::commit_consolidation`；
  - `state/perception.rs` 的 `.cursor.json` / backlog / `commit_cursor`，kRPC `perception.cursor / backlog`；
  - 按 kind 排除自身（`memory/consolidation.rs:137-142`、`state/perception.rs:383-384`）；
  - `SessionKind::SelfImprove`、模板、`si-`、`[loader.self_improve]`、`tests/self_improve.rs`；
  - Jarvis 的 `self_improve_signals.toml` / `self_improve_set_memory.toml`；
  - WebUI 中的 kind。

### 9.3 self_check 退役

- 基于 Notebook 的提醒判断改为具体的业务 Goal（例如架构 §8.1 的活动提醒），依赖通知 TODO 的结论。
- Notebook 与 attention signal 按 Memory 需求 TD-09 / TD-20 退役，调用方清单见 [Memory TODO](agent-memory-multi-session-simulation-todo.md) §8.5。
- 删除 `SessionKind::SelfCheck`、模板、`_self_check_timer`、`sc-`、`[loader.self_check]`、Jarvis `self_check.toml`。

### 9.4 WebUI Goals 视图

- Home 增加 Goals 区域；Goal 详情页包括定义与修订、changes（突出 Goal 自己做的 archive / achieved）、State、按 epoch 分组的 Activation、关联 Work。
- 按钮：pause / resume / activate / reopen / restart。
- 架构 §13.4 的数量提示。

### 9.5 其它

- follow_up（如果 9.2 没有带上）；kevent 对象订阅驱动 Goal；Skill / Manual / XLLM 结晶 Goal、离线 Self Correct Goal（替代 `improve_skills.toml`、`try_create_new_skill.toml`）；UI 意图分析产出 Goal 草案（架构 §5）；Agent 通知人。

---

## 10. 第一段禁止的捷径

- 不要用 TimerBridge 加 `timer:` 订阅驱动 Goal。
- 不要把 Goal 定义复制到 Goal 头之外的地方当真相。
- 不要把 Goal Session 挂在 UI 下面（不能用 `create_sub_session` 创建它）。
- 不要让 Activation 失败、预算耗尽或 WAIT_USER_MSG 结束 Goal Session。
- 不要把 `no_action` 记成 achieved，也不要把“没观察到”记成“没有变化”。
- 不要为每条唤醒原因各开一次 Activation，也不要逐个重放错过的定时。
- 不要在 Goal 内部实现自我唤醒循环。
- 不要改动现有业务（self_improve / self_check / Notebook / task_route / chat_route）；也不要实现任何通知投递。

---

## 11. 实施记录（2026-10-11，第一段）

### 11.1 落点

| 内容 | 位置 |
|---|---|
| 协议类型：Goal 定义 / 变更 / Goal State / Activation 记录 / 计划进度 / Goal 结果 / `OpenActivation` / `GoalSessionState` | `src/frame/lib_opendan/src/protocol/goal.rs`（schema `goal*.schema.json` 由 `json_schemas()` 导出） |
| `SessionKind::Goal`、`session.goal`（`GoalBinding`）、`origin.goal`（`GoalOrigin`）、`WaitPolicy::EndTurn`、模板槽 `goal_activation` | `protocol/config.rs`、`protocol/behavior.rs`（behavior 的 `prompt.goal_activation`） |
| `state.goal`、`state.turn_stop_requested` | `protocol/state.rs` |
| 登记表 / 查询 `goal_id` | `protocol/agent_state.rs`、`state/registry.rs` |
| `control(stop).scope`（`StopScope`）、`control(goal_skills)`、receipt part `brief`、worklog `user_message.brief` | `protocol/input.rs`、`protocol/worklog.rs`、`runner/history.rs` |
| `goal_session_id` / `new_goal_id` | `ids.rs` |
| 内建模板 `goal` 与 `GoalDefaults`（`[session.goal] budget / min_interval_secs`） | `template.rs` |
| `GoalStore`（文件版 CAS、权限、驱动者 / 宿主写）与 `AgentStateClient::goals()`、kRPC `goals.*` | `state/goals.rs`、`state/{mod,fs_client,connect,krpc}.rs` |
| 共用：`ensure_goal_session`、运行机会事件与 key、有效预算 / 统计窗口、Work 准入 | `src/goal.rs`；`api.rs`（`SessionSpec.goal`、登记 `goal_id`、`create_sub_session` 的 Goal 分支） |
| Runner：Goal State 载入 / 发布、状态闸门、Goal 头、Activation 打开（receipt `extra.goal_activation`）/ 并入（`extra.goal_reasons`）/ 预算 / 关闭与写回、Work 吸收 | `runner/goal.rs`；接入 `runner/{drive,outcome,live,receipts,reports,children,inputs,hook,assembler}.rs` |
| 宿主：`GoalHost`（reconcile、epoch 迁移与结束、启动事件、准入链、`ScheduleBackend` / `LocalScheduleBackend`、`ManualClock`）、`Supervisor::with_goals / goal_changed / goal_activate / goal_feedback`、`stopped_from_above`（Goal 的 Work 只随 Goal 归档停止） | `host/goals.rs`、`host/mod.rs`（原 `host.rs` 移为目录） |
| CLI：`xagent goal create / list / show / revise / pause / resume / archive / achieved / reopen / restart / activate / post / skills / serve`；`new --class goal` 拒绝 | `src/bin/xagent.rs` |
| Loader：`[loader.goals] enabled`（缺省开）、`goals` 模块、`loader.status.goals`、`goals.activate` / `goals.post`、写后立即 reconcile、`session.stop` 的 `scope`、TaskMgr 取消 Goal 的 Turn 用 `scope = turn` | `src/frame/opendan/src/{config,loader,service,tasks}.rs` |
| Jarvis：`[loader.goals]`、`[session.goal]`（`default_behavior = "goal"`、预算缺省）、`behaviors/goal.toml`（function_call） | `src/apps/jarvis_runtime/agent/` |
| WebUI：只加了 `goal` kind 的类型与标签（Goals 视图属第二段） | `src/frame/opendan/web/src/{model/types.ts,i18n.ts,pages/Home.tsx}` |
| 文档 | 协议 README（版本说明、fixture 16）、Agent State Protocol §1/§2/§8/§9（新）、Session Directory / Input / Control Protocol、`doc/opendan/xAgent.md`（模板表、CLI、退出码）、`src/frame/opendan/README.md`、`src/frame/lib_opendan/README.md`、架构设计 §15.5（“Goal 的现有实现”行）/ §15.6 / §17.2 |
| fixture | `doc/opendan/protocol/fixtures/16_goal_session`（生成器 `examples/fixtures.rs`，断言 `tests/fixtures.rs::f16_goal_session`）；重新生成时其余 fixture 的时间戳 / run id 一并刷新 |

### 11.2 与本文不同的决定

1. **Goal State 的写回方式**：Goal Session 的 `state.json.goal.state` 是 Goal State 的真相（随 Session 每次提交原子落盘），`state/goals/<id>/state.json` 是提交之后发布的投影（`published_rev`）；Activation 记录先放进 `state.goal.unwritten`，发布 state 后按 `(epoch, seq)` 幂等追加，再提交清除。比“待写回标记 + 事后写”更强：吸收 Work、`goal_skills` 等中途变化也与消费同一次提交。
2. **Activation 预算的计量**：基线是 Session `usage.jsonl` 的长度（`usage_offset`），不是 `static.json`（后者只在 run 结束时累加，暂停 / 可重试错误的 run 用量看不到）。runs = 这些记录中不同的 run_id。`max_tool_iterations` 未纳入 Activation 预算（每个 run 仍受 behavior 的 `max_tool_iterations`）。
3. **结束 Goal Session 的方式**：archive / achieved / restart / reopen 都不是控制命令，而是 Session 在每个提交点读 `goal.json` 的状态闸门（`Gate::End` → `state.goal.ending` + `stop_requested`），在提交点结束、不打断进行中的 run 段。不级联由两处保证：`stop_children` 只在 `ending.cascade`（archive）时执行；托管进程（Supervisor / ChildDriver）的“父停止则停子”对带 `origin.goal` 的 Work 改为“Goal 被归档才停”。因此 stop 控制没有加“不级联”选项。
4. **stop 的范围**：`control(stop)` 增加 `scope: session | turn`；Goal Session 收到的任何 stop（kRPC `session.stop`、TaskMgr 取消、`xagent ctl stop`、驱动进程的 SIGINT）都只结束当前 Activation。
5. **带幂等键的 goal_id**：`g-k<24 hex>` = H(agent, 创建者, key)，同键重复创建返回同一个 Goal（不比较内容）；无键时为 `g-<时间>-<8 hex>`。
6. **`agent-session goal skills`**：经新增的 `control(goal_skills)` 由驱动者应用（保持 Goal State 单写者），Activation 结果里的 `skills` 是另一条路。
7. **Goal 结果校验**：只对 function call 的 `report` 工具（`is_end`）强制（缺少或不合格 → 调用返回错误，模型可改）；behavior 循环的 `<report end>` 不拒绝，不合格时 decision 记 `unknown`、`end_reason` 写原因。
8. **并入的运行机会**：Activation 打开期间消费的输入（例如 run 在可重试错误后续跑时到达的定时 fire）并入该 Activation，其原因经 receipt `extra.goal_reasons` 记入 `reasons[]`。原因类型另加 `event`（其它订阅事件）。
9. **epoch 的结束时间**由宿主在旧 Session 结束且其 Goal State 全部发布后写入（`end_epoch`）；`prepare_epoch` 在此之后才存档 / 写新 state 并建新 Session。restart 的 `epochs[n].end = restarted` 在 restart 时即写入。
10. **周期统计窗口**是按 `window_secs` 对齐的固定窗口（`period_start = floor(now / window) * window`）。
11. **宿主在 CLI 中的形态**：新增 `xagent goal serve`（Supervisor + goals 模块 + 本地计划，无 Loader）；`xagent goal activate / post` 不经宿主时直接向当前 epoch 的 Session 投递。
12. **顺带修复的既有问题**：① kill -9 打断工具后恢复，在途调用被物化为“结果未知”时 run 处于工具批次中间，此前 drive 会立即尝试注入排队的输入而报 “the input batch could not be placed into the context” 并卡住；现在与回填挂起调用相同，先跑完该批次（`LiveCtx.filled`）。② 检查点钩子在 `turn_stop_requested` 时也打断推理（否则被检查点先消费掉的 Turn stop 不会打断正在进行的工具循环）。

### 11.3 验证（在 `src/` 下）

| 命令 | 结果 |
|---|---|
| `cargo test -p libopendan -- --test-threads=1` | 全部通过；新增 `tests/goal_session.rs` 24 个（§5 的 V01–V20 与崩溃窗口：建 Session 前、Activation 中 kill -9、Turn 关闭后 / 发布后 / 追加后、fire 记录后未投递；另有并入原因与“kill -9 后排队输入等批次跑完”），`tests/fixtures.rs` 16 个（含 `f16_goal_session`），`tests/xagent_cli.rs` 新增 `goals_from_the_command_line`（真实二进制 + OpenAI 兼容 mock：`goal create` 幂等、`new --class goal` 拒绝、`goal serve` 托管到 Activation、`goal show / activate / revise / pause`、Goal Session 内越权退出码 6、修订冲突退出码 5） |
| `cargo test -p opendan -- --test-threads=1` | 17 + 10 全部通过；新增 `goals_are_hosted_and_a_canceled_activation_keeps_the_goal`（真实 kRPC：`goals.create` → Loader 建 Session 跑 Activation、`loader.status.goals`、`goals.write_state` 被拒、`goals.activate`、每个 Activation 一个 task、TaskMgr 取消只结束该 Activation）与 `goal_defaults_come_from_session_goal` |
| `cargo test -p llm_context` / `cargo test -p agent_tool --lib` | 222 / 244 通过（未改动） |
| `cargo check --workspace` | 通过 |
| fixtures / schema | `cargo run -p libopendan --example fixtures -- <绝对路径>/doc/opendan/protocol/fixtures` 已重新生成（新增 `goal*.schema.json` 与 `16_goal_session`） |
| WebUI | `pnpm exec tsc --noEmit -p .` 通过 |

V21：上述全量回归通过，UI / Work / self_* 的既有用例行为不变。

### 11.4 DV（2026-10-11，`./debug_jarvis.sh devtest --agent xiaobai.test.buckyos.io`，真实 AICC）

测试 Goal：经 kRPC `goals.create` 创建（`start_on_create`，`every_secs = 300`），目标是维护 `/opt/buckyos/data/home/devtest/goal_dv/TODO.md`（3 个小任务，每项由一个 Work 完成并勾掉）。观察到：

- Loader 建 Session、投 `created`，Activation 1 读 TODO 后建 Work；Work 结束触发 child Activation，按 `goal_id` 吸收（`absorbed` 计数、`work_tokens` 计入周期用量）。5 个 Activation、4 个 Work 后 3 项全部勾掉，最后一轮 `no_action`，Session 回到 waiting / event。每个 Activation 在 zone TaskMgr 有一个 task（完成并上报）。
- 第一个 Work 没有动手就交了报告（见 §11.5 第 1 条），Goal 下一轮读报告并核对目录后重建了一个明确要求先用工具的 Work，后者完成——吸收与判断按设计工作。
- pause 期间到点：`schedule.skipped` 记 `paused`；resume 后继续。restart 后一个 300 s 槽因离上次 Activation 太近被记 `min_interval`。
- archive：Session `finished + stopped`（“ended with its goal”），epoch 1 `end = archived`。reopen（同时在 TODO 里加一个 `sleep 120` 的项）：epoch 2、`reopened` Activation、Goal State 保留。
- restart：epoch 2 的 Session 以 stopped 结束（“superseded by epoch 3”），`epochs/1`、`epochs/2` 存档；epoch 3 的首个 Activation 原因为 `restarted` + `child`，接管了 epoch 2 创建的 Work。再次在 Work 运行中 restart：Work 未被停止，epoch 4 的 `restarted` Activation 在 `<works>` 里看到它并决定 `no_action`；它结束后由 epoch 4 吸收（一次）。
- 计划改为 60 s 后停 Loader 3 分多钟再启动：Loader 在 Activation 进行中被 SIGINT，重启后同一 `(epoch 4, seq 3)` 续完（一条记录）；停机期间错过的槽合并为一次 fire（key `scheduled:<slot>:3`）。那次 fire 并入了续跑的 Activation，而旧二进制没有把并入的原因记进记录——已按 §11.2 第 8 条修正并用新二进制复验（之后每分钟一次 `scheduled`，`min_interval` 跳过有记录）。
- 结束时已把测试 Goal 归档（epoch 4 `end = archived`），并停掉宿主机上的 Loader，app 交回 node-daemon 的容器。
- **本地测试镜像（同日）**：`./build_aios --local-test`（`local/aios-test:latest-amd64`，devenv.json 原本就指向它）+ `cd src && uv run start.py --skip-update`。镜像只带二进制；Agent 的包由 bundled 模板 `data/cache/jarvis.buckyos.bns.did-0.7.0.pikg` 构造，所以另用 `uv run buckyos-build -s jarvis_runtime` 只重建模板并把这一个文件装到 `/opt/buckyos/data/cache/`（旧文件备份在会话 scratchpad），再 `start.py --skip-update`：control_panel 的预装 reconciler 发现模板 AppDoc 变化，自动重建 `template_auto_update` 的 Agent，AgentRoot 同步出 `[session.goal]` 与 `behaviors/goal.toml`。容器形态验证（经 `https://test.buckyos.io/kapi/xiaobai.test.buckyos.io`，devtest 登录）：`loader.status` 的 `goals` 模块运行；reopen → epoch 5，Session 冻结了 `goal` behavior，Activation 原因 `reopened` + 并入的 `scheduled`；它报告 `needs_info`——容器里看不到宿主机的 `goal_dv` 目录，Goal 如实说“无法观察”，没有当成“没有变化”。之后再次归档。

### 11.5 发现的问题与遗留

1. **Work 在 behavior 循环里不动手（第一段之外，已单独修复）**：`agent_tool/src/xllm.rs` 的 behavior 协议文本在“只有原生工具、没有 XML action”时写 “No actions are available in this run: reply with a single `<response>` whose `<report end=\"true\">` contains the complete final result”（2026-10-10 `upgrade opendan report` 引入），同时又说原生工具已声明。Jarvis 的 `plan` behavior 正是这种组合，模型常按前一句直接交最终报告、不调用工具（DV 中 6 个 Work 有 2 个如此）。影响所有 Work，不只 Goal。按“llm_context / agent_tool 层修改单列 TODO 先行”的约定单列在 [xllm behavior 原生工具协议 TODO](xllm-behavior-native-tools-protocol-todo.md)，用户确认后已于同日修复（只有原生工具时改为说明用工具行动、观察结果后再交结束报告）。
2. **Goal Session 的历史逐轮增长**：DV 中连续 `no_action` 轮的 token 每轮约 +1k（机械历史，未到压缩阈值）。架构 §11.2 要求 Goal 的压缩策略单独设计，留给第二段。
3. **镜像**：本地测试镜像已构建并在 DV 容器形态验证（见 §11.4 末条）；正式镜像发布（`build_aios --push`）与模板包发布未做。
4. **未覆盖**：`fail_workspace`（Workspace 绑定失效使 Session failed）不写 Activation 记录；Goal Session 内 `agent-memory` 观察还不会自动带 `goal_ref`（只有运行时的 run_digest 感知带 `source_ref.goal_ref`）；Activation 预算不含 `max_tool_iterations`；WebUI Goals 视图属 §9.4。
5. 已知风险同 §8.3。第一段验收后再把 §9 细化成任务。
