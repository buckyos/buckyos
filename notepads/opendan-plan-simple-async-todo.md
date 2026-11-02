# plan_simple 多 Turn 化与 TodoList 自动调度 TODO（agent-session 重构第二刀）

日期：2026-11-02

状态：**已实施（2026-11-02，未提交）**，实施记录见 §8。用户 2026-11-02 口述思路（§1），Claude 对照 [简单 Work Session 全异步化 TODO](./opendan-simple-worksession-async-todo.md)（已实施，§9 是实施记录）与当前源码写成本单；同日用户在 §4 裁决 Q1–Q14（Q1–Q4、Q9、Q10、Q12–Q14 按建议；Q5/Q6 确认事件路径并说明目的；**Q7 pause = llm_context 语义的停止推理、resume 从 checkpoint 续**；**Q8 plan 只从 Todo 角度控制**；**Q11 不再对单个 Todo review：验证自己是一条 Todo，plan 觉得完成即可结束**），Claude 据此回写 §3 / §5–§6；**用户同日确认 §3.9 的推断：PLAN/DO 执行器机制一并删除**。

范围：`src/apps/jarvis_agent/agent/behaviors/plan_simple.toml` 及其依赖的 TodoList / 子 session 机制（agent_tool `todo_tools`、libopendan `todos.rs` / `runner/todos.rs` / `runner/children.rs` / `xagent_cli.rs`）、`work_simple.toml` 的一个新输入分支、`chat_route.toml` 的转发规则、`agent.toml` 的默认 behavior。第一刀的 class 级改动（多 Turn、explicit_report、timer tick、推理前注入、`waiting_for` 删除、Session 宿主无 `task wait`）已经对 plan_simple 父会话生效，本单不重做，只在其上改 plan 的职责与调度。UI / group / goal 会话除 §3.7 的转发规则外不动。

路径均相对 `buckyos/`。实施前重新读 `AGENTS.md` 与当前源码（工作区有大量未提交改动：第一刀 B15、DV 修正、`get_task_state` 去除等），不得覆盖。先做 A（agent_tool，纯数据模型与规则，单独可测），再 B（libopendan），再 C（Jarvis 包）、D（文档）。

## 0. 术语（沿用第一刀 §0，补充）

| 词 | 含义 |
| --- | --- |
| plan session | class `plan_simple` 的 Work Session（父）。只读、不干活、管 TodoList；多 Turn；`report(is_end=true)` 结束 |
| work session | 一个 Todo attempt 的执行者：独立子 session（behavior `work_simple`，探索为 `explore`），有自己的 Turn task、工作目录、报告 |
| Todo | TodoList 里的一条，`todo_tools::TodoRecord`；它的 attempt 绑定 `child_sid` |
| 就绪 | Todo `pending`、依赖满足（§3.3）、未被并发闸挡住 → 宿主**立刻**派发 |
| 调度点 | 列表可能产生新就绪 Todo 的时刻：`todo add / update / retry / cancel`、子 session 结束回写（`todos::sync`）、并发闸释放 |

## 1. 用户思路（2026-11-02 原文要点 → 本单条目）

1. plan 模式只 Readonly、不干活，核心是管理 session 的 TodoList。→ §3.1
2. 鼓励一口气创建完整的 TodoList。→ §3.2、提示词 C1
3. 创建 Todo 后，如果 Todo 满足条件，就会立刻创建 work session 开始执行。→ §3.3、Q1、Q3、Q4
4. Todo 默认依赖其上一个 Todo（串行），可通过参数指定依赖的 Todo；不依赖就立刻执行。→ §3.2、Q2
5. 通过 work session 的控制命令可以暂停 / 恢复 / 终止 work session。→ §3.6、Q7、Q8
6. 可以对 Todo update（其实就是对 work session 绑定的 task 进行更新），更新后 work session 能拿到事件并调整后续处理。→ §3.5、Q5、Q6
7. plan session 支持多 Turn 后，可以自然地处理 Input（用户消息）；chat-router 可以把用户消息转发到 plan session。plan session 每次收到 Input（任务终态、更新、用户事件等），都会考虑要不要修改 TodoList，或改变正在运行中的 work session。→ §3.4、§3.7、Q9、Q10
8. plan session 也是通过 report 结束的。→ §3.8、Q11
9. 这次修改后，opendan 默认不使用基于 behavior loop 的 session，规避现代 LLM 对 CoT 的处理造成的影响。→ §3.9、Q12

## 2. 现状（2026-11-02 源码核对；实施前复核）

| 主题 | 事实 | 位置 |
| --- | --- | --- |
| plan_simple 的 Turn 形状 | 第一刀后 class `work` 内建模板 = `Turns::Unbounded` + `ExplicitReport` + `_task_watch_timer`；plan_simple 父会话已是多 Turn（11-02 回归 DV：5 个 Turn，回复在子会话运行时关闭 Turn、子会话结束打开下一个）。但 **提示词仍写"The whole plan is one Turn of this session"**，并要求 `agent-session wait <child_sid>` | `template.rs:140-166`、`plan_simple.toml` system 第一段、第一刀 §9.6 |
| 子 session 的结局怎么回到父 | `poll_children` 看**全部** children（`children_of`），attention 变化（finished / needs_input / needs_decision）→ Active 输入事件 `source = session:<sid>`，summary 带 Todo 绑定（`bound_children`）；与是否 `wait` 过无关。`agent-session wait <sid>` 只是 `pending_on_session` 立即返回 + 登记 `wait_children` | `runner/children.rs:242-350`、`xagent_cli.rs:1735` |
| 工作区交接 | 子 session 持父的 workspace（`SubWorkspace::Inherit`，ReadWrite）；"父调用在飞行中时子不能写"的说法在 `children.rs:359` 注释，实施前核对 `runner/drive.rs:401` 与 `tests/workspace_binding.rs` 的真实规则。plan 只读（§3.1）后，父不再有工作区写操作，这条规则的影响只剩父 Turn 内有调用在飞行时的时间窗 | `runner/children.rs:359`、`runner/drive.rs:401` |
| 派发 | 只有模型显式 `todo dispatch <id>`（串行）或 `todo explore`（立即）。串行准入 `TodoList::admit_serial`：pending ∧ **没有其它串行 Todo 在执行** ∧ 所有已结束结果都已 review ∧ `after` 全部 **accepted**。并发闸 `max_sub_sessions` / `--max-children` 拒绝后 Todo 留 pending、要模型再派 | `todo_tools.rs:761-806`、`todos.rs:200-310` |
| 依赖 | `after: Vec<String>` 显式给，缺省**空**（无依赖）；满足 = 被 review accept；`SerialPick` 的 Waiting / Ready 由它算 | `todo_tools.rs:729-760` |
| Todo 修改 | **不支持**修改正文（TodoList 文档 §10 第一条）；只有 review / retry / cancel / clean | `doc/opendan/Agent TodoList.md` §10、`todo_tools.rs` 无 `update` |
| Todo 执行者的输入通道 | Todo 派出的子 session 是 class `work`：`InputChannel::None`，`post` 被拒（"no input queue"）；只有 `create-worksession --interactive` 给 Queue + `WaitPolicy::Allowed` | `api.rs:1012`、`xagent_cli.rs:847-858` |
| 会话控制 | `ControlCommand` 只有 `stop {scope: session|turn} / decide / subscribe / unsubscribe / activity / perceive / goal_skills`；**没有 pause / resume**（`goal pause|resume` 是 Goal 的生命周期，不是 session）。`RunState = Created|Ready|Running|Waiting|Finished`。父 stop 级联到 children（`stop_children`） | `protocol/input.rs:240-300`、`protocol/state.rs:28`、`runner/drive.rs:987` |
| 用户消息进 Work | UI 会话（chat_route）把工作交给 `try_analyze_user_intent` → `create-worksession`；之后"A new request goes to a new analysis; it does not change earlier work"。没有向运行中 Work 转发消息的路径（Work 无队列） | `chat_route.toml`、`analyze_intent.toml` §Rules |
| 背景 | plan_simple `[input] background = "after_tools"`（只在工具结果后注入 Todo 行与未绑定子 session 行）；第一刀新增 `before_inference`（Turn 第一次推理前也注入，且带在跟任务增量），work_simple 已用 | `runner/observe.rs`、`plan_simple.toml [input]` |
| 结束检查 | `report(is_end=true)` 在列表未收敛（有 pending / running / 未 review / need_evidence）或有未听取的子 session 时拒绝；收敛 = 每个 Todo cancelled 或最新 attempt 已结束且 review 为 accept / reject | `runner/reports.rs`、`todo_tools.rs::unconverged` |
| 默认 behavior | `agent.toml [session.work] default_behavior = "plan"`（XML behavior loop，PLAN/DO）；`[session.plan_simple]` 是单独的类，只在 `create-worksession --class plan_simple` 时用；analyze_intent 的 "plan and do (the default, no `--behavior`)"；goal.toml 建 Work 不带 `--behavior` → 走默认 | `agent.toml:61-78`、`analyze_intent.toml` §Shape、`goal.toml:18` |
| 真实模型基线 | 11-02 plan_simple 回归 DV 40/40，309 s、534 k tokens（父 5 Turn + 4 子会话） | 第一刀 §9.6 |

## 3. 目标形状

### 3.1 plan session 只读、只管 TodoList

plan session 自己不改文件、不跑构建、不产出交付物；它的工具调用只有三类：读（`read`、只读 shell：`cat` / `ls` / `git diff` / `agent-session todo show|list` / `read-session`）、TodoList 管理命令（§3.2、§3.5、§3.6）、`report`。"干活"一律是 Todo。提示词明确写出；宿主不做工具级白名单（`group:bash` 不可按命令过滤），靠提示词 + review（Q13 讨论要不要用 `WorkspaceAccess::ReadOnly` 把 plan 的工作区绑定设成只读，让写入在宿主层被拒）。

### 3.2 一口气建完整 TodoList；依赖缺省串行

- 提示词鼓励：第一个 Turn 就把现在能看清的 Todo **全部**建出来（探索 + 串行工作 + 验收），而不是一个一个来；后面的 Turn 按结果增删改。
- **依赖缺省 = 上一个 Todo**（用户第 4 条）：`todo add` 不带 `--after` 时，`after = [列表中 seq 最大的、非 cancelled 的 Todo]`（第一个 Todo 没有依赖）；`--after none` 显式声明无依赖（立刻可执行）；`--after T01 --after T02` 显式指定。探索（`todo explore`）缺省仍是无依赖、立即派发（保持），也接受 `--after`。Q2。
- 这就是"串行"的新含义：串行是**依赖链**，不再是"同一时刻只有一个串行 Todo 在跑"的全局锁（Q3）。两条无依赖的 Todo 并行执行，并发由 `max_sub_sessions` / `--max-children` 兜底。
- 一个 Todo 一旦有 attempt 在跑，它的依赖不可改；pending 的 Todo 可以 `todo update --after …` 改依赖（§3.5）。

### 3.3 自动派发：满足条件立刻建 work session

宿主调度，模型不再 `todo dispatch`（Q4：`dispatch` 命令从提示词删除；保留 CLI 作为开发 / 恢复入口还是直接删，见 Q4）。

| 条件 | 含义 |
| --- | --- |
| Todo `pending` | 新建、retry 后、或被并发闸挡回 |
| 依赖满足 | `after` 里每个 Todo 的最新 attempt **`completed`**（执行成功；Q1 裁决：不等任何验收）；上游 `cancelled` 视为不满足（Todo 进 `blocked`，等 plan 决定，§3.4） |
| 并发闸 | 未结束子 session 数 < `max_sub_sessions`（宿主 `--max-children`）。挡住时 Todo 留 pending，**不记 refusals、不要模型重派**，下一个调度点再试 |
| 父未 paused / 未结束 | plan session 被 stop 时不再派发 |

调度点（§0）每次都做一遍 `schedule()`：扫描 pending Todo 按 `seq` 顺序，就绪的依次派（同一把列表锁内记 attempt，复用现有 `dispatch_as` 的 dispatching → create → bind 幂等路径，key 仍是 `todo:<id>:<attempt>`）。

- 工具进程侧（`agent-session todo add|update|retry|cancel`）：命令完成后在同一进程里调 `schedule()`，于是"一口气建 5 个 Todo"时 T01（及其它无依赖的）在 `todo add` 返回前就已派出，命令输出里带 `dispatched: [T01 → work-…]`。
- runner 侧：`todos::sync` 回写子 session 结局之后（`drive.rs:1463 / 1595 / 1631`、`observe.rs:330`）调 `schedule()`——上游 `completed` 立刻派下游，不等 plan 推理；并发闸释放也在这里补派。
- 上游 `failed / timeout / stopped`：下游**不**自动派，状态改 `blocked`（复用 `TodoStatus::Blocked`，含义改为"依赖未成功，等 plan 决定"），背景行显示 `blocked by T01 (failed)`；plan 在收到 T01 的结束事件的 Turn 里决定：`todo retry T01`（成功后下游自动解锁回 pending）、`todo cancel T02`、或 `todo update T02 --after none` 等。
- 派发失败（runtime 不满足 skill、workspace 不存在等 `InvalidArgument`）：attempt 撤回、Todo 改 `blocked` 并记原因（不是 pending，否则每个调度点都重试），plan 的下一个 Turn 看到原因后 update / cancel。
- 子 session 的创建参数不变：串行 `work_simple` + 继承 workspace；探索 `explore` + 无 workspace；`--runtime / --workspace / --skill / --behavior` 按 Todo 记录。**新增**：Todo 派出的子 session 带输入队列（§3.5，Q6）。

### 3.4 plan session 的多 Turn：每个输入都是一次"要不要改计划"

plan session 的 Turn 由下面这些输入打开（第一刀机制已有，本单补齐模板与提示词）：

| 输入 | 来源 | 现状 | 本单 |
| --- | --- | --- | --- |
| 首条：目标 + runtime 上下文 | 创建者 | 有 | 保留 |
| 子 session 结束（`session:<sid>` `finished`，summary 带 `Todo Tx attempt n: completed/failed — 一句话`） | `poll_children` | 有（Active） | `on_input` 专门段落：Todo id、状态、一句话结果、"需要时读 `todo show --report`；下游 Todo 已自动开始 / 被 blocked；要验证就建一条验证 Todo" |
| 子 session `needs_input` / `needs_decision` | `poll_children` | 有 | 段落：子 session 在问什么；回答用 `agent-session post <sid> --msg`（子现在有队列）或 `todo update`；或 `todo stop` |
| 用户消息（chat_route 转发，§3.7） | 队列 `SessionInput::Msg` | **无**（plan 无队列） | 段落："用户在对话里补充 / 变更了需求：…；判断它影响哪些 Todo：update 正在跑的、cancel 不再需要的、add 新的；不需要改就回复一句说明" |
| timer tick | `_task_watch_timer` | 有（30 s） | plan 的 `tick_secs` 单独配置（Q10）；段落："N 个 Todo 在跑（见背景），没有新结果；没事可做就回复一句现状" |
| nudge | runner | 有 | 同 work_simple |
| 背景（推理前注入） | `observe.rs` | `after_tools`，只有 Todo 行 + 未绑定子 session | 改 `before_inference`（Turn 第一次推理前也注入）；Todo 行加 `blocked_by`、`paused`、`dispatched_at`；仍不含子 session 的工具输出 |

每个 Turn 的任务只有一个：看完输入与背景，决定 (a) TodoList 要不要改（add / update / cancel / retry，含为验证新建 Todo），(b) 正在跑的 Todo 要不要干预（update / pause / resume / stop，§3.6），(c) 是否可以 `report(is_end=true)`；都不需要就回复一句现状关闭 Turn（Turn task 结果 + worklog）。plan 自己**不**派发——它改完列表，宿主按 §3.3 派。

### 3.5 `todo update`：更新 Todo = 更新 work session 的任务

目的（用户 Q5/Q6）：plan 收到用户消息后可能要调整**正在运行**的 work session，而 work session 没有 inbox——它唯一能收到的外部变化就是"它的任务被更新了"。所以 update 是 plan 影响运行中 work session 的唯一正式通道，形状是事件。子 session 为此带的输入队列只给宿主 / 父的 Todo 事件用，**不是**给用户消息或任意文本的 inbox（提示词与 CLI 都不提供 `post` 到 Todo 子 session 的用法）。

新命令 `agent-session todo update <id> [--title] [--content - | --content <t>] [--accept <c>]... [--clear-accept] [--scope] [--evidence <e>]... [--after <id>|none]... [--runtime] [--workspace] [--skill]... --reason "<why>"`：

- 任何状态（除 cancelled）都可改正文类字段（title / content / accept / scope / evidence）；`rev` +1，`updates: [{at_ms, reason, fields, by}]` 记进 Todo（有历史，不覆盖原因）。
- `after / runtime / workspace / skill` 只对**没有 attempt 在跑**的 Todo 可改（pending / blocked / 已结束待 retry）；在跑的拒绝（原规则：换机器 = 新 attempt）。（Q5 按 §3.5）
- **有 attempt 在跑时**：更新写入列表后，工具进程向该 attempt 的子 session 队列 post 一条 Active 事件 `source = todo:<id>, event = updated, summary = "<reason>; changed: content, accept", data = {新正文 task_text, 变化字段}`（Q6），打开子 session 的一个新 Turn；`work_simple.toml` 的 `on_input` 加 `is_todo_update` 分支："你正在做的任务被更新了（原因）：<新的完整任务正文>；对照已做的工作调整后续步骤，不再需要的任务用 `cancel_task` 停掉；按新的验收标准报告"。子 session 的 Turn task（TaskMgr）title / 描述同步更新（用户原话"对 work session 绑定的 task 进行更新"在 MessageHub 上可见）。
- pending / blocked 的 Todo 被 update 后立刻过一次 `schedule()`（改了 `--after none` 就会马上派）。
- TodoList 文档 §10 "不修改 Todo 正文"这条**撤销**；"不删除 Todo"、"不做任意依赖图（只有 after）"保留。

### 3.6 Todo 的控制：pause / resume / stop（内部传导到 work session）

plan **只从 Todo 的角度**控制（Q8）：`agent-session todo pause|resume|stop <id> --reason "<why>"`。宿主找到该 Todo 在跑 attempt 的 `child_sid`，向子 session 发控制命令（新 `ControlCommand::Pause { reason } / Resume { reason }`，stop 复用现有 `stop`），并在 attempt 上记 `paused_at / resumed_at`；stop 后 attempt 由 `sync` 回写为 `stopped`，Todo 需 `todo retry` 才再派（不自动回 pending，避免无限重跑）。`xagent ctl <sid> pause|resume` 作为底层 / owner 侧命令存在（WebUI、开发用），不进 plan 的提示词。

pause 语义（Q7，用户：llm_context 语义的停止，就是不再推理；resume 从 checkpoint 续）：

- pause = 该会话**不再推理**。Turn 之间收到：不再打开新 Turn，到达的输入留在队列 / pending。Turn 内收到：当前在飞行的工具调用返回并写入 checkpoint 后不再派下一轮推理，run 停在工具调用前 / 后的 checkpoint（冻结 TODO 的唯一快照事实来源），Turn 保持打开。
- resume = 从 checkpoint 续推理（既有 `open_state_live_run → resume_live_run` 路径）：Turn 内暂停的接着同一 run 推理；Turn 之间暂停的把积压输入作为一批打开新 Turn。
- 在跟的 shell 任务不受影响、按 OS 进程语义继续跑（[feedback-os-process-semantics]），结局进 pending，resume 时一并呈现；tick 在 paused 期间不产生。
- 子 session 的子 session 级联 pause / resume（与 stop 级联同一处 `stop_children`）。
- 状态：`SessionState.paused: Option<{at_ms, reason, by}>`，登记表 `status.paused: bool`；`RunState` 不加枚举值（`Finished` 判定、`follows_work()` 不受影响）；Turn task 文案 "paused: <reason>"；`xagent sessions` / WebUI 显示；Todo 背景行显示 `paused`。
- 对已结束会话 pause / resume 是空操作并提示；对 pending / blocked 的 Todo pause 被拒（没有在跑的 attempt）。

### 3.7 用户消息转发到 plan session

- `[session.plan_simple]` 的会话有输入队列：class 覆盖键 `input_channel = "queue"`（新增 agent.toml / `SessionPolicy` 键，内建 `work` 模板仍 `None`；Q9），`WaitPolicy` 仍 `FinishFailed`（plan 不"等用户"，用户消息只是输入之一）。
- chat_route（UI）规则：对话里有活跃的 plan 子 session（背景 `work <sid>: running …` 行 + `agent-session sessions --children --active`）且用户的消息是对该工作的补充、变更、催促或回答时，`agent-session post <plan_sid> --msg "<用户原话 + 必要的对话上下文>"`，并回复用户"已转给正在进行的工作"；与该工作无关的新请求仍走 `try_analyze_user_intent`。analyze_intent 的 "it does not change earlier work" 改为"由对话判断是否转发"。只转发给 plan session（它是入口），不直接转给 Todo 的 work session。
- plan 处理用户消息的 Turn 回复（一句现状 / 调整说明）写进 Turn task 结果与 worklog（第一刀 B7），用户在 MessageHub 的 Turn task 卡片上看到；chat_route 不自动转述（Q9b）。
- 对所有 Work 都开队列不在本单（`work_simple` 子 session 开队列是为 §3.5 的 update 事件，用户消息不进它）。

### 3.8 验证是 Todo；结束由 plan 自己判断

用户 Q11：**不再对单个 Todo 做 review**。plan 要验证一个结果，就建一条验证 Todo（通常是 `todo explore`：只读、无 workspace、`--after <被验证的 Todo>`，正文写验收标准与怎么检查），它的报告就是验证结论；plan 读报告后决定 retry / update / cancel / 新建修复 Todo。于是：

- `todo review`、`ReviewVerdict`、`ReviewState`、`TodoRecord.reviews`、`TodoReview`、`--followup`、"结果未 review 不得派发 / 不得结束"全部删除（A2 / A6 / B10）。探索 attempt 结束时不再写子 session 的 acceptance（原 review accept → `accepted` 的路径删；实施前核对 acceptance 是否影响父 report 引用子报告的 artifacts——若影响，子 session 以 `report` 结束即视为 accepted）。
- 只有 `report(is_end=true)` 结束 plan session（已是）。**宿主不再按列表收敛拒绝结束**：plan 觉得任务完成（或不可完成）就可以结束，报告里写清每个 Todo 的去向（完成 / 失败 / 未做 / 取消及原因）。仍在跑的 attempt 随父会话结束被 stop 级联（现有 `stop_children`），`sync` 记为 `stopped`；提示词写明"结束前先 `todo stop` 或等它们结束，否则它们会被停掉"。原"有未听取的子 session 不得结束"的检查也删（子结束本来就是输入，plan 不需要逐个确认）。
- `report(is_end=false)` 仍只记进度。

### 3.9 默认不用 behavior loop

- `agent.toml [session.work] default_behavior = "plan_simple"`；`[session.plan_simple]` 保留为同义类（`--class plan_simple` 兼容、`tick_secs` / `input_channel` 的覆盖放在它上面还是 work 上，见 Q9 / Q10）。
- analyze_intent：`--behavior work_simple` 用于简单任务；其余不带 `--behavior` = plan_simple；文字里 "plan and do" 改为 "plan (a TodoList of work sessions)"。goal.toml 不改（已不带 `--behavior`）。
- Jarvis 包里的 `plan.toml` / `do.toml` 删除（Q12，[feedback-llm-context-docs-internal]、[feedback-dv-fresh-install] 的风格：不留 legacy）。`opendan_agent_dev.md` 的 "plan / do (the default)" 段删。
- **PLAN/DO 执行器机制一并删除**（用户 2026-11-02 确认 Claude 据 Q11 的推断）：Q11 删掉 review 后，`serial_pick` 的 Unreviewed 分支与准入规则失去依据，留下的是没有使用方、规则又与新列表不一致的半残路径。本单删 libopendan 的 `[entry] todo` 执行器机制（`next_behavior = do` 准入 `review_handover`、`serial_pick`、`begin_serial / end_serial`、`ChildCall.todo`、`review_decision` 钩子的 Todo 分支）与 `tests/plan_do_todo.rs`；llm_context 的 `review_decision` 钩子本身保留（核对其它使用方）；`TodoMode::Serial` 的注释改为"work session 执行、按依赖链"（名字不改，避免 schema 搅动）。
- `explore.toml` 已是 function_call；其提示词补一句多 Turn（长命令 / tick）与 `report` 结束（第一刀 §7 遗留）。

### 3.10 可观察结果（验收时就看这几条）

1. plan 第一个 Turn 建出完整 TodoList，`todo add` 返回时无依赖的 Todo 已在跑；plan 全程没有 `todo dispatch`、没有 `agent-session wait`。
2. 上游 Todo 的子 session 结束 → 下游 Todo 在 plan 推理之前已被宿主派出（worklog 时间戳可证）；plan 的 Turn 只做判断 / 调整，需要验证就建验证 Todo。
3. 用户在对话里改需求 → chat_route `post` 到 plan → plan 的下一个 Turn `todo update` 正在跑的 Todo → 该子 session 收到 `todo:…#updated` 打开新 Turn 并调整。
4. `todo pause/resume/stop` 传导到子 session：pause 后子不再推理、resume 从 checkpoint 续；MessageHub 的 Turn task 显示 paused。
5. 不带 `--class` / `--behavior` 的 Work 默认是 plan_simple；Jarvis 包里没有 behavior loop 的 session 在默认路径上。
6. 父会话推理次数 ≤ 子会话结束次数 + 用户消息次数 + 1（建列表）+ 1（report），tick 不额外推理（`tick_secs` 600）；全程没有 `todo review`。

## 4. 裁决记录（2026-11-02 用户在文中回答；Claude 整理）

| Q | 问题 | 裁决 | 落点 |
| --- | --- | --- | --- |
| Q1 依赖满足 | 下游自动启动看上游 `completed` 还是 review accepted | **`completed`**（按建议） | §3.3、A2 |
| Q2 缺省依赖 | 不带 `--after` 依赖 seq 最大的非 cancelled Todo；`--after none` 无依赖；explore 缺省无依赖 | 按建议 | §3.2、A1 |
| Q3 一次一个串行 | 去掉"没有其它串行在执行"与"所有结果已 review"两条 | 按建议：去掉；同 workspace 并行写由 plan 建依赖避免（提示词写明） | A2、C1 |
| Q4 `todo dispatch` | 自动派发后是否保留 | 按建议：提示词删、CLI 删；`todo explore` 保留为 add + 无依赖的简写 | B1、C1 |
| Q5 `update` 可改字段 | 正文类随时可改；after / runtime / workspace / skill 只在无 attempt 在跑时 | 按 §3.5。用户说明目的：plan 收到用户消息后要能调整**运行中**的 work session，work session 没有 inbox，update task 是唯一通道 | §3.5、A4、B3 |
| Q6 update 怎么到子 session | 子 session 带队列、Active 事件 `todo:<id>#updated` | **通过事件**（按 §3.5）；队列只给宿主 / 父的 Todo 事件，不是 inbox | §3.5、B2、B3、C2 |
| Q7 pause 语义 | — | **llm_context 语义的停止：不再推理；resume 从 checkpoint 续推理。** 在跟任务不动 | §3.6、B4 |
| Q8 控制入口 | ctl 通用 + todo 门面 | **plan 只从 Todo 角度控制**（`todo pause|resume|stop`），内部传导到 work session；`ctl <sid> pause|resume` 只作底层 / owner 侧 | §3.6、B4、C1 |
| Q9 plan 的队列与转发 | `[session.plan_simple] input_channel = "queue"`；chat_route 判断并 `post`；plan 回复不自动转述 | 按建议 | §3.7、B5、C4 |
| Q10 plan 的 tick | — | 按建议：600 | B5、C3 |
| Q11 结束收敛 | completed 需 accept、失败需 reject | **不对单个 Todo review：验证单独建 Todo；plan 觉得任务完成随时可以结束。** review 机制整体删除，宿主不再按收敛拒绝结束 | §3.8、A2、A6、B10、C1 |
| Q12 plan/do 去向 | Jarvis 包删 plan.toml / do.toml；默认 plan_simple | 按建议；**并确认**（用户同日）：Q11 使 review 从数据模型消失，libopendan 的 PLAN/DO 执行器机制一并删 | §3.9、B11、C5 |
| Q13 plan 只读的宿主保障 | plan 的 workspace 绑定 ReadOnly | 按建议：做 | B9 |
| Q14 `agent-session wait` | Session 宿主删 `wait <sid>` 与 `--wait` | 按建议：删 | B8 |

## 5. 实施清单

### A. agent_tool `todo_tools.rs`（先行，纯规则，单测）

- **A1 缺省依赖**（Q2）：`NewTodo.after: Option<Vec<String>>`（`None` = 缺省上一个，`Some(vec![])` = `--after none`）；`TodoList::add` 解析缺省；explore 缺省 `Some(vec![])`。
- **A2 就绪规则 + 删 review**（Q1、Q3、Q11）：`TodoList::ready() -> Vec<String>`（pending ∧ 每个 after 的最新 attempt `completed` ∧ 无 after 为 cancelled），按 seq；`admit_serial` 改名 `admit`，只剩 pending + 依赖满足两条（Explore 同一函数）。删 `ReviewVerdict / ReviewState / TodoReview / TodoRecord.reviews / review() / unreviewed() / review_state() / accepted()`，`view_line` 去 `review=`；`SerialPick` 随执行器机制删（§3.9）。
- **A3 blocked**：`TodoList::settle_dependents(todo_id)`：上游终态非 completed → 下游 pending 改 `blocked`（`blocked_by: Vec<String>`），上游 retry 成功 → 下游回 pending；派发失败 → `blocked` + `block_reason`。`TodoStatus::Blocked` 注释改写。
- **A4 update**：`TodoList::update(id, TodoPatch, reason, now) -> Result<UpdateOutcome{ running_child: Option<sid>, changed: Vec<&str> }>`；`TodoRecord.updates: Vec<TodoUpdate>`；在跑时拒绝改 after / runtime / workspace / skills；`task_text()` 用最新正文（子 session 收到的事件正文由它生成）。
- **A5 attempt 的 paused**：`TodoAttempt.paused_at_ms / resumed_at_ms`、`view_line` 显示 `paused` / `blocked_by`。
- **A6 列表摘要**（Q11）：`unconverged()` 删；新增 `TodoList::summary()`（各状态计数 + 在跑 / blocked / paused 的 id），供 `report(is_end)` 时写进 worklog 与 Turn task 结果，不作门。
- **A7 测试** `cargo test -p agent_tool --lib todo_tools`：缺省依赖链、`--after none`、ready 不受"其它在跑"影响、completed 即满足、上游失败 → 下游 blocked → retry 成功解锁、update 字段规则与 updates 历史、summary；删 review 相关用例，旧 `serial_admission_and_a_kept_behavior` 按新规则改。

### B. libopendan

- **B1 调度器** `todos::schedule(agent, who, channels, parent, opts) -> Vec<Dispatched|Blocked>`：锁内取 `ready()`，并发闸只挡不记 refusals，复用 `dispatch_as`；派发失败改 blocked。调用点：`xagent_cli.rs` 的 `todo add / update / retry / cancel` 之后（输出带 `dispatched`）；`runner/todos.rs::sync` 返回 changed 后（`drive.rs` 三处 + `observe.rs:330`）；并发闸释放 = 任一子 session 结束的同一处。父 paused / finished 时不派。
- **B2 子 session 队列**（Q6）：`todos::child_spec` 置 `SubSessionSpec.queued = true`（新字段，不等于 `interactive`：队列有、`WaitPolicy::FinishFailed` 不变），`api::create_sub_session` 据此设 `InputChannel::Queue`；`post` 的 `Event` 路径（`agent_input` 的 `--event`）用于 update 事件，`SessionInput::Event{source: todo:<id>, event: updated}` 写入子队列；settle 时视为外部 Active 事件。
- **B3 `todo update` CLI** + 子事件投递 + Turn task 更新：`xagent_cli.rs::cmd_todo` 加 `update`；有在跑 attempt 时 post 事件并调 opendan Turn task 的 title / description 更新（`turn_task.rs` 的 sink 加 `retitle`；无 TaskMgr 时跳过）。
- **B4 pause / resume**（Q7、Q8）：`ControlCommand::Pause / Resume`；`SessionState.paused`；drive：取输入前检查 paused → 不打开 Turn（输入留队列 / pending）；Turn 内收到 pause → 当前在飞行的工具调用返回、checkpoint 写入后不再推理，run 停在 checkpoint、Turn 保持打开（`Next` 新结局 `Paused`，与 `keep_while_running` 的"Turn 关闭 run Paused"区分：这里 Turn 不关）；resume → `open_state_live_run → resume_live_run` 从 checkpoint 续，Turn 之间暂停的把积压输入作为一批打开新 Turn；paused 期间 `poll_ticks` 不产生 tick；级联在 `stop_children` 旁加 `pause_children / resume_children`；登记表 `SessionStatus.paused`；`xagent ctl <sid> pause|resume`（底层）；`agent-session todo pause|resume|stop <id> --reason`（plan 唯一入口：找在跑 attempt 的 child_sid 发控制，attempt 记 `paused_at / resumed_at`；stop 走现有 `ctl stop`，attempt → stopped 由 `sync` 回写）；WebUI 显示 paused。
- **B5 plan 类的队列与 tick**（Q9、Q10）：`SessionPolicy.input_channel: Option<InputChannel>` 新键（agent.toml `[session.<class>] input_channel = "queue"`），`template.rs` 应用；`[session.plan_simple] tick_secs = 600`。
- **B6 输入模板变量**：`InputView.items` 加 `is_child_end / is_child_needs_input / is_todo_update / is_user_msg` 标记与 `todo_id / attempt / child_sid / result_line / blocked_dependents` 字段（第一刀 B12 的同款做法：upon 模板不能比较字符串）；fixtures `14_input_bus/rendering/vars.json` 更新。
- **B7 背景**：`observe.rs` 的 Todo 行加 `blocked_by / paused / started`；plan_simple 改 `before_inference`（`after_tools` 若无使用方则删枚举值）。
- **B8 删 `wait`**（Q14）：Session 宿主拒绝 `agent-session wait <sid>`（文案"its end comes back to you as input"），`create-worksession --wait` 删；`wait_children` 登记改为自动（所有 children 都在 `watched_children` 已是），`SessionState.wait_children` 若只剩这个用途则删；提示词、`ui_core.md`、`opendan_agent_dev.md`、`tools/buckyos-agent/readme.md` grep "agent-session wait"。
- **B9 只读 plan**（Q13）：`child_spec` / `create-worksession` 对 class plan_simple 的 workspace 绑定 `access = ReadOnly`；Todo 子 session 仍 ReadWrite。核对 `runner/drive.rs:401` 的 ReadOnly 处理与 `verify_workspace_access`。
- **B10 结束检查**（Q11）：`reports.rs` 删 TodoList 收敛检查与"子 session 未听取"检查（`unsettled_children` 若只剩此用途则删）；`report(is_end=true)` 通过时把 A6 的 `summary()` 写进 worklog / Turn task 结果；父结束对在跑子 session 的 stop 级联已有，确认 `sync` 在级联后把 attempt 记 `stopped`（`todos::sync` 的 "not registered / stopped" 分支）。`decide_unqueued` 写子 acceptance 的 review 路径删。
- **B11 默认 behavior 与执行器**（Q12，已确认）：`agent.toml` 改；`template.rs` / loader 对 `default_behavior` 的断言测试（`opendan --test loader`、`plan_simple.rs` 的 "`[session.work]` 仍是 `plan`" 断言反转）；删 libopendan 的 Todo 执行器机制：`runner/todos.rs` 的 `review_handover / decision_step / caller_of / current / wait_for` 与 `ChildCall.todo` 绑定、`todos.rs` 的 `end_serial / begin_serial / needs_own_session` 分叉、`BehaviorEntry.todo` 键（`[entry] todo`）、`tests/plan_do_todo.rs`；llm_context 的 `review_decision` 钩子保留（libopendan 的 Todo 分支删，其它使用方核对）。
- **B12 测试** `cargo test -p libopendan --test plan_simple_async -- --test-threads=1`（新建，ScriptedLlm + 真实 Jarvis 包，ChildDriver 驱动子会话，shell 用 `sleep`，不许测试线程 `std::thread::sleep`）：
  1. 一口气 `todo add` T01..T04（T03 `--after none`）：返回时 T01、T03 已派（两个子 session 并行），T02 / T04 pending；plan 全程无 `dispatch` / `wait`；
  2. T01 子 session `report` 结束 → `sync` 后 T02 在 plan 下一次推理**之前**已派（worklog 顺序断言）；plan 的 Turn 看到结束事件段落，建一条验证 Todo（`todo explore --after T01`）；
  3. T01 失败 → T02 blocked（背景行 `blocked_by=T01`）→ plan `todo retry T01` → 成功 → T02 自动派；
  4. 并发闸 `--max-children 1`：T03 被挡（无 refusals）→ T01 结束 → T03 自动派；
  5. `todo update T02 --content …` 在 T02 跑时：子 session 队列收到 `todo:T02#updated` 打开新 Turn、模板段落含新正文；Turn task 标题更新（TurnTaskSink 用例）；在跑时改 `--after` 被拒；
  6. `todo pause T02`：Turn 内暂停 → 在飞行的工具调用返回后不再推理（ScriptedLlm 调用次数不增）、run 停在 checkpoint、Turn 仍打开；其在跟 shell 任务照跑并结束 → `todo resume T02` 从 checkpoint 续推理、同一 run 看到结局；Turn 之间暂停 → 积压输入在 resume 时一批打开 Turn；paused 期间无 tick；`todo stop T02` → attempt stopped → `retry` 重派；对 pending Todo pause 被拒；父 pause 级联子；
  7. 用户消息：向 plan 队列 `post --msg` → plan 新 Turn 的模板段落 `is_user_msg`；plan `todo add` 新 Todo 立即派；
  8. 结束：有 pending / blocked / completed 未验证的 Todo 时 `report(is_end=true)` 照样通过（worklog 带 summary）；有在跑 attempt 时结束 → 子被 stop 级联、attempt `stopped`；验证 Todo：`todo explore --after T02` 在 T02 completed 后自动派、其报告作为输入回到 plan；
  9. plan 的 `write_file` 到 workspace 被 ReadOnly 拒（Q13）；
  10. tick：`tick_secs = 1` 下 plan 等子时每秒一个 Turn（机制回归），`[session.plan_simple] tick_secs = 0` 不产生；
  11. 崩溃恢复：`todo add` 后调度前杀工具进程 → runner 开头 `sync + schedule` 补派，不重复（幂等 key）；update 事件 post 前后杀 → 事件只投一次；
  12. 默认类：`create-worksession --objective` 不带 class/behavior → plan_simple；Jarvis 包无 `plan` / `do`；
  13. 回归：`--test plan_simple`（改断言：去 review / wait / dispatch）、`--test work_simple_async`、`--test ui_chat`、`--test xagent_cli`、`--test xagent_lib`、`--test crash` / `context_switch` / `runner_more`（核对对 plan/do fixture 的依赖）、`cargo test -p opendan`；`--test plan_do_todo` 随执行器删除（B11）；schema `xagent schema doc/opendan/protocol/schema` 重生成（`todo_list`、`session_state`、`session_status`、`control_command`、`session_config`、`behavior_entry`）。

### C. Jarvis 包与提示词

- **C1 `plan_simple.toml` 重写**（系统提示词按 §3 全部改写，英文，结构参考 work_simple）：
  - 开头：你是 plan session：只读、管 TodoList，不干活；多 Turn；每个输入都是一次"要不要改计划"；只有 `report(is_end=true)` 结束。
  - Calls：intent 必填（不变）；去掉 "whole plan is one Turn" 与 `wait` 相关全部文字。
  - What you see：首条输入；每个 Turn 的输入种类（§3.4 表）；背景（推理前注入，Todo 行字段含 blocked_by / paused）；按需 `todo list / show --report / read-session`。
  - TodoList：`todo add`（缺省依赖上一个、`--after none`、`--after Tx`；**不需要 dispatch：就绪即自动开始**）、`todo explore`、`todo update … --reason`（含"在跑的 Todo 会在它的下一个 Turn 收到更新；这是你影响运行中工作的唯一方式"）、`todo retry`、`todo cancel`、`todo pause|resume|stop`（pause = 它不再推理、resume 接着上次的地方）。没有 `review`：要验证就建验证 Todo（`todo explore --after <id>`，写清验收标准与检查方法）。
  - Where a Todo runs：保留现有 runtime / workspace 段，删"dispatch it, wait for it and end your reply"，补"两条会写同一 workspace 的 Todo 要有依赖关系，宿主不替你排队"。
  - Working the plan：1) 第一个 Turn 读状态后**一口气**建出完整列表（探索并行、工作按依赖链、最后一条验证 Todo）；2) 之后每个 Turn：读输入 → 看结束的结果（需要就读报告、建验证 Todo）→ 处理 blocked（retry / update / cancel）→ 按用户消息 update / add / cancel → 没事就一句现状；3) 永不自己做 Todo 的工作（含不用 `create-worksession`）；两条会写同一 workspace 的 Todo 要有依赖关系。
  - Completion：§3.8——你觉得目标达成或不可能达成时就 `report(is_end=true)`，写清每个 Todo 的去向；还在跑的先 `todo stop` 或等它结束，否则会被停掉；never report a result you did not verify（验证 Todo 的报告是证据）。
  - Asking the user：保留 `ask_user`；补"用户的补充消息会以 `<message>` 作为输入到来"。
  - `[input] on_input` 模板：分支 child end / child needs input / todo update ack（可选）/ user msg / tick / nudge / other；`[input] background = "before_inference"`；`[budget]` 不变；`[model] llm.plan`。
- **C2 `work_simple.toml`**：`on_input` 加 `is_todo_update` 分支（§3.5 文案）；系统提示词补一句"你的任务可能被更新，更新以输入到来"。
- **C3 `agent.toml`**：`[session.work] default_behavior = "plan_simple"`；`[session.plan_simple] tick_secs = 600`、`input_channel = "queue"`；注释改写。
- **C4 `chat_route.toml`**：§3.7 的转发规则一段（何时 `post`、回复用户什么、不转发什么）。`analyze_intent.toml`：Shape 段改 "plan_simple (the default)"、Rules 段改转发说法。
- **C5 删 `plan.toml` / `do.toml`**（Q12）；`explore.toml` 补多 Turn 一句。
- **C6 手册**：`cmd_skills/ui_core.md`（`post` 到活跃 plan 的用法行）、`cmd_skills/opendan_agent_dev.md`（§Plans with Todos 重写：自动派发、update、pause；删 PLAN/DO 段与 `agent-session wait`）。

### D. 文档

- **D1** `doc/opendan/Agent TodoList.md`：§4 / §6（PLAN/DO 执行器、等待=不做决策）随 B11 删；§4a 重写为主章节（自动派发、依赖缺省、update、blocked、pause、验证 Todo）；§8 "Review 与收敛" 删，改为"结束由 plan 判断"；§9 命令表（去 dispatch / wait / review，加 update / pause / resume / stop）；§10 不支持能力（撤"不修改正文"）；§11 验证。
- **D2** `doc/opendan/OpenDAN Agent Session架构设计.md`：控制命令加 pause / resume；class 的 `input_channel` 键；plan session 的输入种类。
- **D3** 协议文档：`Session Input Protocol`（`todo:<id>#updated` 事件、Control pause/resume）、`Agent State Protocol`（`SessionStatus.paused`、`[session] input_channel / tick_secs`）、`Session Directory Protocol`（todos.json 新字段）。
- **D4** `doc/sdk/OpenDAN_Agent_Dev_Guide.md` §5.1 默认 behavior；本单 §8 写实施记录。

## 6. DV（真实模型，本机）

- 驱动：`test/plan_do_todo_dv/main.py --mode plan_simple`（现有；旧 PLAN/DO 模式随 B11 删）扩展用例：`normal`（目标要求 ≥ 4 个 Todo，含两个可并行的探索与一条写 workspace 的链）、`user-update`（驱动在 T02 运行中向 plan 队列 post 一条用户变更消息，验收：T02 子 session 收到 updated 事件并在报告里体现新要求）、`failure`（现有；验收改为 plan 建了验证 Todo 并在报告里写明失败）。
- 验收（对照 §3.10）：`parent/intents.json` 无 `todo dispatch` / `agent-session wait` / `todo review`，有至少一条验证 Todo；`todos.json` 里下游的 `started_at_ms` 早于父会话下一次推理；Turn 数 ≈ 子结束数 + 用户消息数 + 2；token 与 11-02 基线（534 k）对比记进 §8；无 403 / invalid_prompt。
- Zone 侧（Loader 托管、MessageHub Turn task 卡片显示 paused / 更新标题、chat_route 转发）：本机 DV Zone `./debug_jarvis.sh devtest --agent <test agent>`（跑法见 [plan_simple TODO](./opendan-plan-simple-todo.md) §10.4），在 MessageHub 里对一个进行中的工作追加需求，观察 plan 的 Turn 卡片回复与子 session 的调整。
- 回归：`PLAN_DO_DV=1 python3 test/work_simple_dv/main.py all --variant normal`。

## 7. 关联

- [简单 Work Session 全异步化 TODO](./opendan-simple-worksession-async-todo.md)（第一刀；§7 第二刀条目：plan_simple / explore 提示词与模板、子 session 等待形状 R2、noted task 分工 R5 由本单收尾；T2 call_behavior 不在本单）。
- [plan_simple TODO](./opendan-plan-simple-todo.md)（intent 契约 §3 不变；§1 "整个父 Plan 是一个逻辑 Turn"、"等待只挂起"、"执行完成与验收通过分开（review）"三条**作废**，以本单为准）。
- [llm_context 冻结 TODO](./llm-context-freeze-todo.md) 原则 8（loop 里不等待）。
- [xllm 视角 runtime review TODO](./xllm-agent-runtime-review-todo.md)（tool_policy，不相关但同期）。
- 记忆原则：Agent 工具简化 + 渐进式披露（执行方式由配置定：派发由宿主做，不让模型选"何时派"）；OS 父子进程语义（pause 不杀任务）。

## 8. 实施记录（2026-11-02，未提交）

按 §5 A → B → C → D 实施，§6 DV 见 §8.4。

### 8.1 落点

| 条目 | 落点 |
| --- | --- |
| A1–A7 数据模型与规则 | `agent_tool/src/todo_tools.rs`：`NewTodo.after: Option<Vec>`（缺省 = seq 最大的未取消 Todo）、`ready()` / `admit()`（pending ∧ after 全部 `completed`）、`settle_dependents()`（每次修改后重算 `blocked_by`）、`block_reason`、`update()` + `TodoPatch` / `UpdateOutcome` / `updates[]`（`after` 成环拒绝）、`mark_paused()` + `paused_at_ms / resumed_at_ms`、`summary()` / `TodoSummary::line()`、`view_line(&list)`（`after / blocked_by=T01:failed / started / paused / updates`）；删 `ReviewVerdict / ReviewState / TodoReview / reviews / SerialPick / CallRef / begin_serial / end_serial / refusals / DispatchRefusal / unconverged / ReportRef.run_id / TodoAttempt.caller / run_id`；`begin_dispatch / bind_dispatch / withdraw_dispatch(failure: Option)`；单测 7 项 |
| B1 调度 | `lib_opendan/src/todos.rs`：`schedule()`（按 seq 派所有就绪 Todo，`held` 即停；plan paused / stopping / finished 不派）、`dispatch()` → `Scheduled::{Dispatched, Held, Blocked}`（并发闸 = 宿主 `--max-children` 与 `policy.max_sub_sessions` 取小，挡住不写任何记录；`InvalidArgument` → blocked + `block_reason`）；`runner/todos.rs::sync` = 回写 + 调度（drive 开头、批次边界、`observe::at_boundary`）；CLI 的 `todo add / explore / update / retry / cancel` 之后同进程调度，输出 `scheduled` |
| B2 子 session 队列 | `SubSessionSpec.queued`（`api::create_sub_session` → `InputChannel::Queue`，等待策略不变）；`todos::child_spec` 置位；`runner/inputs.rs`：有 `origin.parent_session` 的 Session 把 `source.kind = todo` 的事件当 Active 输入 |
| B3 update | `xagent todo update`；`todos::post_update`（event `todo:<id>` / `updated`，summary ≤ 1 KiB，`data_ref = <plan>#<id>@<attempt>`，key 按 Todo rev 幂等）；`todos::expand_update`（取入批次时把 Todo 当前完整任务接在 summary 后，`drive.rs`）；`reports::validate` 在更新未取入前拒绝 `is_end`（`inputs::todo_update_queued`）；更新事件开出的 Turn 以更新为 task 标题（`live.rs::commit_input_batch`） |
| B4 pause / resume | `ControlCommand::Pause / Resume`；`SessionState.paused {at_ms, reason, by, in_run}`、`resume_run`；`SessionStatus.paused`；`needs_input()` 排除 paused；`hook.rs::boundary` 推理前遇 paused → 中断（原因 `session paused`）；`outcome.rs` 新 `FinishKind::Paused`（run 保留在检查点、Turn 开着、不报错）；`drive.rs`：paused 时不取输入、不开 Turn、不跑 continuation / bootstrap，`resume_run` 时从 `open_state_live_run` 续推理；`inputs.rs::apply_control` + `cascade_pause`（resume 只恢复因级联而暂停的子）；`turn_task.rs` 文案 `paused: <reason>`；`xagent ctl <sid> pause|resume`；`agent-session todo pause|resume|stop`；`xagent sessions` 与 OpenDAN WebUI（`Sessions.tsx` / `SessionDetail.tsx`）显示 paused |
| B5 plan 类 | 复用模板已有的 `[session.<class>] input = "queue"` 键（见偏差 1）；`[session.plan_simple] tick_secs = 600` |
| B6 模板变量 | `InputView` 项加 `is_child_end / is_child_needs_input / is_todo_update / is_user_msg`、`child_sid / todo_id`；`is_other_event` 改为排除这些；子 session 结束的 summary 带 Todo、attempt、结局、一句话与“它之后的 Todo 的去向”（`children.rs::summary_of` + `runner/todos.rs::bound_children`）；fixture `14_input_bus/rendering/vars.json` 重生成 |
| B7 背景 | plan_simple 改 `before_inference`；`InputBackground::AfterTools` 与 `observe.rs` 的 after_tools 分支删除 |
| B8 删 wait | `agent-session wait` 与 `create-worksession --wait` 返回说明（“结束作为输入回来”）；`pending_on_session` 删；`SessionTaskResolver` 保留（仍回答 `session:` id，已无产生方） |
| B9 只读 plan | behavior 键 `[capabilities] workspace = "read_only"`（`BehaviorCapabilities.workspace`）；`api::create_session` 把绑定（`workspace_binding.access`）收窄为 read_only、授权（`workspace`）不变，登记表条目记绑定的 access；`runtime::verify_workspace` 允许绑定比授权窄；`drive.rs::acquire_workspace_writer`：被收窄的绑定不取写者租约，冲突检查跳过 read_only 条目；授权本身是 read_only 的 Session 仍拒绝运行 |
| B10 结束 | `reports::validate` 删列表收敛与“子 session 未听取”两道门；结束提交里 `todos::stop_open`（在跑的 attempt 记 `stopped`）+ worklog `control_applied{todo_summary}` + 结束 Turn 的 task 结果前置摘要行；`after_run_end` 对非 Goal 的结束调 `drive::stop_children`（被 stop：全部子；正常结束：只停需要汇报的子），`host::stopped_from_above` 同步（父结束时停需要汇报的无队列子） |
| B11 默认与执行器 | `agent.toml [session.work] default_behavior = "plan_simple"`；删 `BehaviorEntry.todo` / `[entry] todo`、`ChildCall.todo` / `TodoBinding`、`runner/todos.rs` 的 `review_handover / settle / bind_serial / serial_returned / current / unconverged`、`hook.rs::review_decision` 覆写（llm_context 钩子本身保留）、`session.current_todo` 模板变量、`tests/plan_do_todo.rs`、`reports/` 报告副本 |
| B12 测试 | 新 `tests/plan_simple_async.rs`（6 项）、`tests/common/plan.rs`（Jarvis 包与宿主帮助函数）；`tests/plan_simple.rs` 改写为契约 + 崩溃 2 项（加子进程入口 1 项）；改 `xagent_cli.rs`（`--wait` 被拒后不带 `--wait` 创建）、`xagent_lib.rs`（`DispatchOptions` / `Scheduled`）、`opendan/tests/loader.rs`（无 plan / do、before_inference、read_only、work 默认 plan_simple）；schema 重生成（`behavior_config / behavior_entry / control_command / registry_entry / session_config / session_input / session_state / todo_list`） |
| C1–C6 | `plan_simple.toml` 重写（含 `on_input` 分支）、`work_simple.toml`（`is_todo_update` 分支、子 session 分支）、`explore.toml`（多 Turn 与更新）、`agent.toml`、`chat_route.toml`（§Work in progress 转发规则）、`analyze_intent.toml`（`--class plan_simple | --behavior work_simple`、`shape: plan | simple`、运行中 plan 的补充不是新请求）、`goal.toml`（去 `--wait`）、删 `plan.toml / do.toml`；手册 `opendan_agent_dev.md`（§Plans with Todos 重写、`[input]` / `[capabilities] workspace`）、`ui_core.md`（`post` 到 plan）、`xllm.md` |
| D1–D4 | `doc/opendan/Agent TodoList.md` 版本 4 重写；`OpenDAN Agent Session架构设计.md` 新 §6.8；`protocol/Session Input Protocol.md`（todo 事件、pause / resume、级联、去 wait）、`Agent State Protocol.md`（`status.paused`、`workspace` 能力键、`background`、`tick_secs`）、`Session Directory Protocol.md`（`paused / resume_run`、去 `call.todo / entry.todo`）；`doc/sdk/OpenDAN_Agent_Dev_Guide.md` §5.1；`xAgent.md`、`build-in agent-tool手册.md`、`lib_opendan/README.md`；DV 驱动 `test/plan_do_todo_dv/`（README、`main.py`、`task_simple.md`；删 PLAN/DO 模式与 `task.md`） |

### 8.2 与原文不同的决定

1. **B5 不加 `input_channel` 新键**：模板早有 `[session.<class>] input = "none" | "queue"`，直接用。plan 的队列只在 `--class plan_simple` 上（analyze_intent 用它创建 plan）；`[session.work]` 的缺省 behavior 是 plan_simple 但不开队列（Goal 建的 plan 不需要转发），其 tick 仍 30 s。
2. **B9 只读是绑定收窄，不是授权**：直接把 `workspace.access` 设 read_only 会让 Todo 子 session 继承只读（`workspace_binding.rs` 的“子继承不能扩大授权”测试守着这条），而授权本身为只读的 Session 按原规则仍不允许运行（本地 shell 无法隔离）。所以 behavior 只收窄 plan 的绑定，授权（读写）照常传给子；plan 的 shell 不受限，`write_file / edit_file` 本来只写工作目录，`copy --to workspace:` 被只读绑定拒绝。
3. **“没人回答”= 无队列 或 等待策略不是 allowed**（`outcome.rs::nobody_answers`）：带队列的 plan 与 Todo 子 session 仍是多 Turn Work（回复关 Turn、无事可等时纠正一次后失败）。只看等待策略会让无队列且策略为缺省 allowed 的会话改变行为（`report.rs` 用例），只看队列又会让 plan 变成“等用户”。
4. **work session 处于 Turn 中时，更新并入当前 Turn**：输入只在批次边界取入；它此时想 `report(is_end)` 会被拒绝（“更新在等你”），回复一句后经纠正把更新带进同一个 Turn（`plan_simple_async::an_update_reaches_the_running_work_session`）。Turn 之间到达时开新 Turn。TaskMgr 没有改名接口，所以没有 `retitle`：更新开出的 Turn 的 task 以更新为标题，并入时不改。
5. **attempt 的 `blocked` 不再表示“子在等回答”**：子需要输入时 attempt 仍是 `running`，plan 从 `is_child_needs_input` 输入得知；`blocked` 只表示前置未成功 / 派发失败。
6. **派发撤回不记录**：`refusals` 删除；并发闸挡住 = 不写任何东西（下个调度点再试），中断的派发撤回为 pending，派发失败 = `blocked + block_reason`。
7. **update 事件 summary 只有一行**：事件 summary 上限 1 KiB，完整任务正文由 work session 的宿主在取入时按 `data_ref` 读 plan 的列表补上（读不到时保持原样）。
8. **结束时停止的范围**：被 stop 的父停全部子（原规则）；正常结束的父只停需要汇报的子（`--report none` 的“发了就不管”的子继续跑）。原来父只有被 stop 时才级联，现在结束的门（未听取子）删了，所以补上这一条。
9. `todo dispatch` / `todo review` / `wait` 保留为返回说明的命令名（不是功能），便于模型旧习惯得到纠正。
10. 并发闸 `--max-children` 只在 `xagent` 宿主设置环境变量；Loader（opendan）宿主没有，仅受 `max_sub_sessions`（缺省 4）。

### 8.3 测试（本机，2026-11-02）

- `cargo test -p agent_tool --lib todo_tools`：7 passed。
- `cargo test -p libopendan --test plan_simple_async -- --test-threads=1`：6 passed（一口气建列表与依赖顺序 + 上游结束先于 plan 推理派下游 + 验证 Todo + 只读 plan / 读写子；上游失败 → blocked → retry 解锁 + 结束时在跑的被停；并发闸 held 与补派；用户消息 → `todo update` → 子取入更新、在跑时改 after 被拒；`todo pause/resume/stop` + retry、对不存在 Todo 的 pause 被拒；plan 暂停时输入积压、级联暂停子、恢复）。
- `cargo test -p libopendan --test plan_simple -- --test-threads=1`：3 passed（类与契约、只读绑定、`write_file` 写不进 workspace、注入前后崩溃恢复）。
- 全量 `cargo test --no-fail-fast -p libopendan -p agent_tool -p opendan -- --test-threads=1`：45 个测试二进制全部通过（第一轮暴露并修正：`report.rs` 无队列显式完成、`workspace_binding.rs` 只读授权、`xagent_cli.rs` 的 `--wait`、fixture f14、`opendan --test loader`）；`stop_children` 的最后一处改动后重跑 plan_simple(_async)、xagent_lib、runner_basic/more、goal_session、report_stop、crash 全部通过。
- OpenDAN WebUI `pnpm check`（tsc -b）通过。
- 未跑：`uv run buckyos-build.py`、Zone 侧（Loader 托管、MessageHub Turn task 卡片的 paused / 更新标题、chat_route 真实转发）——§6 第三条未做。

### 8.4 真实模型 DV（本机 CLIProxyAPI，gpt-6-sol，2026-11-02）

驱动 `test/plan_do_todo_dv/main.py all --variant normal|user-update|failure`（三例并行），证据 `/home/lzc/project/plan_do_todo_dv_runs/20261102T0140{03,06,09}-plan_simple-*`。

| 例 | 结果 | 用时 | tokens | 要点 |
| --- | --- | --- | --- | --- |
| normal | 32/32 | 364 s | 806 k | 第一个 Turn 建出 5 个 Todo（A、B 并行探索；C 验证 after A,B；D 写交付 after C；E 交付后的独立核验 after D）；每个下游在上游结束后 ≤ 10 ms 由宿主派出，早于 plan 的下一个输入批次；plan 6 个 Turn（建列表 + 5 个结束各一）；无 dispatch / wait / review / sleep；列表摘要 `5 todos: 5 completed` |
| user-update | 36/36 | 280 s | 696 k | B 运行中驱动 `xagent post` 用户变更；plan 下一个 Turn `todo update T02 --content … --accept …`（28 s 后）；B 的 work session 在想结束时被拒、经纠正批次（`#updated@4` + `nudge`）取入更新（又 25 s），报告含 `ACCEPT-B-EXTRA-9Z1`；plan 还主动 `todo update` 运行中的写交付 Todo，补充证据引用 |
| failure | 24/24 | 408 s | 1.0 M | 缺 `idempotency.py`；最终报告与 review.md 写明 B 不可判定、原样入口无法导入；plan 在 D 结束后追加核验 Todo（加入即派） |

- 基线（11-02 旧 plan_simple 40/40，309 s，534 k）不可直接比：本次 normal 多一个交付后核验 Todo（模型按“最后一条验证 Todo”自己加的），每个 Todo 一个 work session。
- 无 403 / invalid_prompt / 拒绝记录。
- DV 检查本身修正过两处（第一次 verify 31/32 与两处误判）：Turn 开出的输入批次在 worklog 里是 `turn_started`（不是 `input_batch`），调度检查改为两者都算，否则恒真；“可追溯”只要求被审查依据的调查（探索且带 A/B 标记），交付后的核验不可能出现在交付物里；调度检查跳过“前置结束后才建”的 Todo（加入即派）。三例在修正后的检查下全部通过（`verify` 重跑，未重跑模型）。

### 8.5 未做与风险

- **Zone 侧未验**（§6 第三条）：Loader 托管、MessageHub Turn task 卡片显示 paused / 更新标题、chat_route 真实转发用户消息到 plan——本机只做了 xagent 的 DV。`uv run buckyos-build.py` 未跑；容器内使用要 `./build_aios --local-test` 重建镜像。
- **work session 只在批次边界看到更新**：Turn 中途的更新要等它想结束（被拒）或这个 Turn 结束才取入，DV 里延迟约 25 s；长 Turn 会先做完旧任务再调整。可选的后续：把待取入的 Todo 更新放进 work session 的推理前背景（`before_inference`）。
- 拒绝结束 → 回复 → 纠正（`nudge`）这条路把更新带进同一个 Turn；若这个 Turn 已用过一次纠正，第二次无事可做的回复会以 `missing_end_report` 失败（与第一刀规则相同）。
- `SessionTaskResolver` 的 `session:` 分支已无产生方，留作后续清理。
- Goal 建的 plan（class `work`）没有队列、tick 30 s；只有对话建的 `--class plan_simple` 才能收转发与 600 s tick。
- 只读 plan 的 shell 仍能写 workspace（只靠提示词）；文件工具与 `copy` 已被宿主拒。

### 8.6 Zone DV 反馈修正：半订阅只在输入里使用（2026-11-02，未提交）

用户在重建的 DV 上用 Jarvis 跑“练练看”（plan `work-20261102-3`、UI `ui-20261102-2`）发现：

1. 输入（子 session 结束事件）之后又插一条独立的 `<<background_context>>` user 消息（`observe::at_boundary` 推理前注入）。
2. 同一结局两遍：子 session 的 `finished` 事件 + 背景里 `<todo status="completed">` 行；UI 里同样是 `work:<sid>: finished …` 背景行 + 同批 `finished` 事件（根源：UI 背景把本批的子 session 事件当成 `work:<sid>` 行的来源）。

用户给出的核心原则：**半订阅永远不能成为一条独立的 input，只能在 input 中被使用；input 的渲染模板不引用 background_context，半订阅对这个 session 就不可见。** Todo 只是创建 work session 的 helper，可以没有事件。

实施：
- 每个受控输入批次（所有 Session）算背景候选行：UI 为 chat 各行；其它 Session 为半订阅状态版本（`event:<key>`，用户时区事件也成行）；另有在跟的运行中 task 的新输出（`task:<id>`）。作为模板变量 `background` / `background_context`（原 `background_text` 改名）交给模板；内建模板（chat、`<session_input>`）都放在最前；`plan_simple` / `work_simple` 的 `on_input` 首行 `{{ background_context }}`。
- 渲染后只有输入文本里实际出现的行记为已显示（`background::settle`），未出现的状态版本不进 receipt `events`、留在 `pending_events`。记录统一在 `state.background_shown`（随 receipt），task 行记录值为 `<stdout>,<stderr>` 输出位置；调用转 task 后的推理边界只记位置（`observe::note_started`），不出消息。
- 删除：`<semi_subscription_snapshot>` 消息、`[prompt] semi_subscription_snapshot` 模板、`PART_SNAPSHOT`、`[input] background` 配置（`InputBackground`）、run.json `host.extra.background`、`HostMeta.observed / observed_epoch / task_output`、`observe::at_boundary`（工具结果之后不再注入背景）。推理前钩子只保留路由、TodoList 同步、记 task 位置。
- 子 session 的结束 / 需要输入 / 等决定只走事件：UI 背景不再从本批事件取 `work:<sid>` 行（只留活动视图里运行中的子 session），读报告提示挪进 UI 的一行事件摘要（`— read: agent-session read-session <sid> --report`）；Todo 绑定的子 session 与 Todo 本身都没有背景行。`chat_route` / `groupchat_route` / `plan_simple` / `work_simple` 提示词、`ui_core` / `opendan_agent_dev` 手册随之改。

文档：Session Input Protocol §4/§5/§6（新增 §6.6“背景是输入的素材”）/§7、Agent State Protocol、Session Directory Protocol、protocol README、Session 架构设计（实现状态段与 §15.5 表）、事件订阅、Agent TodoList、xAgent、LLM Context 设计、prompt_render_engine。

测试：新增 `background::a_line_counts_as_shown_only_when_the_input_contains_it`、`runner_more::semi_state_is_invisible_to_a_template_without_background`；`plan_simple` 崩溃用例改在承载 T01 结束的批次前后崩溃（`input_batch:after_input_checkpoint#3` / `after_state_commit#3`）；`plan_simple_async` 失败用例按结束事件判断；`work_simple_async` 第 3 例断言背景从不独立成消息；`runner_more` / `xagent_lib` / `fixtures` / `crash` / `input_tasks` 的快照断言改为单消息形状。

待定（用户架构 §6.3.1 原目标“工具返回后继续推理也观察，可随 function call result 一起装配”）：现在工具结果之后不观察；是否把背景装进 tool result 由用户定。子 session 的 `progress` 半订阅（report = progress）在它等输入时仍会与 `needs_input` 事件同批出现，未动。未做：Zone DV。
