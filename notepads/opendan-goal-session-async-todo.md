# Goal Session 对齐全异步模型与"Agent 主动找用户"TODO（agent-session 重构 Goal 刀）

日期：2026-11-02

状态：**待用户 review，未实施。已定：Q-G4（2026-11-02 用户：Goal 建的 Work 用 work_simple 就够了；Q-G4b 随之删除，Q-G5 收窄）；其余未裁决。** 用户 2026-11-02 要求先写 TODO 整体过一遍：核心是两刀重构之后 Goal Session 的设计意图有没有变化；最大的体验问题是"Agent 主动和用户发起会话"应该放在哪。Claude 对照 [llm_context 冻结 TODO](./llm-context-freeze-todo.md)、[简单 Work Session 全异步化](./opendan-simple-worksession-async-todo.md)（第一刀，§9 是实施记录）、[plan_simple 多 Turn 化](./opendan-plan-simple-async-todo.md)（第二刀，§8 是实施记录）、[Goal Session TODO](./opendan-goal-session-todo.md)（第一段，§11 是实施记录）、[Agent 主动给用户发"消息"的方法总结](../doc/opendan/Agent%20主动给用户发_消息_的方法总结.md)（下称《方法总结》）与当前源码（含工作区未提交改动）写成。

范围：Goal Session（class `goal`、`goal.toml`、`memory_maintenance.toml`）的运行时语义、它派生的 Work 的形状与预算、它与用户之间的通道。两刀都写明"goal 会话不动"，本单就是补这一刀。UI 会话只涉及 §4"由谁在对话里开口"这部分；UI 的中间回复、提问撤销仍归第三刀（第一刀 §7）。

路径相对 `buckyos/`；`lib_opendan/` 指 `src/frame/lib_opendan/src/`，`jarvis/` 指 `src/apps/jarvis_agent/agent/`。工作区有 MessageHub worksession UI 的未提交改动（`create-worksession --title` 必填等），实施前复核，不得覆盖。

## 0. 结论先行

1. **Goal 的主意图没变。** 它仍然负责长期观察、判断、按预算行动；Activation 是有预算的运行机会；Goal 不等 Work，Work 的结局由后续 Activation 吸收；需要人决定时声明 `questions`。新模型（loop 不等待、等待都是任务、结局作为输入回来）其实是把 Goal 一开始就采用的"靠输入驱动、不挂起"推广到了 Work。
2. **变了的是 Goal 周围的东西，而且是被默认值悄悄改掉的：**
   - Work 的形状：`[session.work]` 的缺省 behavior 从 PLAN/DO 变成了 `plan_simple`。Goal 原本要建的是"一件有界的事"，现在建出来的是"一个 plan 加若干 Todo 子会话"（§3 I6）。
   - Goal 的预算口径因此失效：孙会话不计入、Work 运行中计数恒为 0、plan 每 30 秒推理一次（§3 I7）。**Q-G4 已定：Goal 建的 Work 一律 work_simple**，孙会话与 plan tick 两条随之消失，只剩"运行中恒为 0"。
   - "Turn"在 Work 和 Goal 里的含义分叉了：Work 的 Turn 是一次进度，任何回复都会关闭它；Goal 的 Turn 是一次 Activation，只有 report、预算耗尽或 stop 才关闭。共享的提示文案是按 Work 写的（§3 I4）。
   - Work 现在也能长时间跟着自己发起的任务，"有界等待归 Work、长期等待归 Goal"的边界需要重新说清（§3 I2）。
3. **"Agent 主动找用户"今天只有 System Notify 一条路。** 对话里出现的是通知卡片的抄送；UI session 只在用户下一次说话时从背景行看到它，自己从不开口。对照之下，对话派出的 Work 结束时，UI session 会被事件唤醒，用自己的口吻转述结果；Goal 没有这条路。建议（Q-U1 选 B）：**对话里开口的永远是这个对话的 UI session。** Goal 声明"要告诉用户什么"，由 runtime 作为事件交给绑定对话的 UI session，形状与 Work 结束相同；通知只负责触达强度和需要答复的状态。§4 展开。

## 1. 用户思路（2026-11-02 原话要点 → 本单条目）

1. 先写一个 TODO，整体过一下。→ 本单
2. 核心还是新的修改对 Goal Session 的设计意图有没有变化。→ §3
3. 最大的用户体验问题是"Agent 主动和用户发起会话"应该在哪。→ §4

同日上一轮 review 发现的 5 个具体问题（`--title`、plan_simple 预算漏算、task 文案、`ask_user`、残留任务开 Activation）并入 §3 与 §6。

## 2. 现状（2026-11-02 源码核对；实施前复核）

| 主题 | 事实 | 位置 |
| --- | --- | --- |
| Goal 模板 | class `goal` = `Turns::Unbounded` + `WaitPolicy::EndTurn` + `InputChannel::Queue` + `ObserveScope::EventsAndActive` + 隐式订阅 `system:goal`。没有 `_task_watch_timer`；`multi_turn_work` 只认 `SessionKind::Work`，Goal 不在其中 | `lib_opendan/template.rs:207-230`、`runner/outcome.rs:318` |
| Activation 的结束 | `goal_turn_end` 把任何一种 Turn 结束都改成 Waiting，并清空 `wait_children`；只有 Goal 本身的 ending 会让会话结束 | `runner/outcome.rs:377-396` |
| Goal 不等 Work | `unsettled_children` 对 goal 直接返回空；Work 的结局作为 child 事件打开后续 Activation（reason `child`） | `runner/children.rs:189`、`goal.rs:152` |
| Activation 内的长命令 | shell 在调用内等 25 秒，超时转为进程内 task。模型回复但不带 report 时，`keep_while_running` 保住 run；由于 `turn_end` 为空，Activation 一直开着，直到 task 结局回来，在同一个 run 里接着跑。期间没有 tick。Activation 预算只有 tokens 和 runs，没有墙钟上限 | `runner/outcome.rs:996-1080`、`jarvis/agent.toml:91-93` |
| Activation 关闭后的残留任务 | `end_open_work` 只取消可取消的任务；不可取消的 `t-` 任务留在 `watched_tasks` 里。它们结束时以 reason `event` 打开新的 Activation，不经过 GoalHost 的准入（`min_interval` 等） | `runner/outcome.rs:1320-1356`、`goal.rs:158`、`host/goals.rs:244` |
| 跟进提示文案 | Session 宿主的 `TaskFollowup::Input` 说 "Reply with where things stand (that ends your turn) … the task's end starts the next one"；report 被拒时说 "reply to close this Turn meanwhile"。这两句对 goal 都不成立 | `src/frame/agent_tool/src/task_model.rs:195-238`、`runner/reports.rs:244-248` |
| `ask_user` | 所有会话的 baseline 都有这个工具（buckyos-agent CLI，底层是 `human.input/v1` 任务）。goal 里只靠 `goal.toml` 的 "do not use `ask_user` here" 和子 context 交接时的 "Do not ask the user." 挡住。真被调用的话，Activation 会挂到用户答复或过期（缺省 3 小时） | `lib_opendan/baseline.rs:79`、`runner/assembler.rs:486`、`jarvis/behaviors/goal.toml` |
| Goal 的 questions | 声明式：每次 report 整份替换，同一文本对应同一条通知，不再列出的就撤回。发出去的是 `NotifyKind::Input`，与 `ask_user` 底层是同一种 `human.input` 任务。答案由 GoalHost 每 30 秒轮询一次，变成 `system:goal` 的 `answer` 事件打开 Activation | `runner/goal.rs:900-1110`、`host/goals.rs:505-560` |
| Goal 建 Work | `goal.toml` 调 `create-worksession` 时既不带 `--behavior` 也不带 `--title`，于是走 class `work` 的缺省 behavior `plan_simple`（无队列、tick 30 秒）。工作区未提交的改动已经让 `--title` 必填，MessageHub worksession UI TODO 的 B2 要补 `goal.toml` | `goal.toml:18`、`jarvis/agent.toml:67-71`、`xagent_cli.rs:1267-1270` |
| `origin.goal` | 只打在 Goal Session 直接建的 Work 上；plan 派出的 Todo 子会话没有 | `lib_opendan/api.rs:918-976` |
| Goal 的预算 | period 的 `max_works` 只数带 `origin.goal` 的会话；`work_tokens` 只累加被吸收的 Work 自己的 `statistics.total_tokens`。statistics 只在 run 结束时累加，而多 Turn Work 的 run 一直保活，所以运行中恒为 0 | `goal.rs:296-360`、`runner/goal.rs:808-816`、`runner/outcome.rs:1505-1516` |
| Activation 的输入 | 第一批是两条独立的 User 消息：Goal 头（reasons / definition / state / works / recent_works / budget / result_format）和输入批次（事件渲染加内置模板的 background）。同一个 Work 结局会在 `<reason kind="child">`、`<works>`、事件渲染三处出现 | `runner/live.rs:1436-1445`、`runner/goal.rs:355-560`、`runner/assembler.rs:576` |
| Goal → 用户 | `goal.toml` 里有两种做法：`agent-session notify send`（提醒、发现）和 `result.questions`（要人决定）。通知的 `source.conversation` 由 `conversation_of` 沿 `parent_session` 或 `Goal.created_by.via = ui_session:<sid>` 找到 UI 会话的 outbound。msg-center 以 Agent 名义把通知抄送进那个对话，再往 Agent 收件箱写一条 `NotifyDelivered` 观察记录。UI session 通过 `_notify` 半订阅收到它，在下一个批次的背景里显示一行 `[notify <id>]`。UI session 不会因此开 Turn | `lib_opendan/notify.rs:255-293`、`src/frame/msg_center/src/notify.rs:1100-1195`、`lib_opendan/bridge/msg.rs:160-180`、`template.rs:169-187` |
| 不从对话创建的 Goal | 内置的 `memory_maintenance`、WebUI 或 kRPC `goals.create` 建的 Goal：`conversation_of` 返回 None，通知只进通知中心 | `lib_opendan/notify.rs:283-290` |
| Work → 用户（对照） | 对话派出的 Work 结束，是 UI session 的 child 事件 → UI 开一个 Turn → "read its report and relay the result"。也就是说，Work 结束时 Agent 已经会主动在对话里开口 | `jarvis/behaviors/chat_route.toml:23` |
| 承诺与实际 | UI 的目标文本写着 "when a work session or goal reports back, relay the result"，但 Goal 没有回报到 UI 的通路；analyze_intent 告诉用户的是 "the goal's notifications" | `template.rs:60`、`jarvis/behaviors/analyze_intent.toml:109` |
| 用户 → Goal | UI 判断后执行 `agent-session goal post <goal_id> --msg`（Goal 有队列，reason `message`）；回答通知用 `notify answer`（`UI_NOTIFY_RULE`） | `runner/assembler.rs:170`、`analyze_intent.toml:93` |

## 3. 设计意图对照

| # | Goal 的原意图（出处） | 新模型下 | 结论 | Q |
| --- | --- | --- | --- | --- |
| I1 | Goal 负责长期观察、判断、渐进推进；它不持续运行，靠运行机会被拉起（《方法总结》§10.2、Goal TODO §0） | loop 不等待、结局作为输入（冻结原则 8、F18），与 Goal 的做法同构 | **不变** | — |
| I2 | Work 只做有界等待；长期等待（外部回复、某个日期、某个状态变化）归 Goal（《方法总结》§9.4、§10.1） | 多 Turn Work 能跟自己发起的任务任意久（保活 run + tick），`ask_user` 3 小时过期，Work 仍以 report 结束 | **需要重述**：Work 等的是"自己发起的任务"，Goal 等的是"世界、时间、人"。analyze_intent 已写 "Open-ended waiting or watching is never a work session"，`goal.toml` 与手册对齐这一句即可 | Q-G4 |
| I3 | Goal 不等 Work（Goal TODO §3） | 同上；`unsettled_children` 对 goal 返回空 | **不变** | — |
| I4 | 一次 Activation 就是一个 Turn，以 `report(result)` 结束；预算按 Activation 计 | Work 的 Turn 变成"一次进度"，回复即关；Goal 的 Turn 仍是 Activation，Activation 内的任务一直跟到结束 | **语义分叉**：行为本身没错，但共享文案是按 Work 写的，而且 Activation 没有墙钟上限 | Q-G1 |
| I5 | 要人决定时写 questions（声明式）→ 通知 → 答案打开后续 Activation；Goal 不用 `ask_user`（Goal TODO §3、Notify TODO） | `ask_user` 已一般化为 `human.input` 任务（F20），底层与 questions 是同一种通知；冻结 TODO §3.2.1 写的是"能不能问由工具白名单和 Goal 的 questions 回流决定" | **不变**，但白名单那一半没做；答案的回流有两套路径 | Q-G2、Q-G3 |
| I6 | Work 是"一件有界、可以独立完成的事，从不是整个目标"（`goal.toml`） | 缺省 behavior 变成 `plan_simple`：Goal 建出的是 plan，plan 再派 Todo | **被默认值改变**：Goal 本身已经是跨 Activation 的规划者，Goal → plan → Todo 成了两层规划。**Q-G4 已定：回到一件事一个 work_simple** | Q-G4 |
| I7 | Goal 的预算（period 的 tokens / works）覆盖它派生的全部工作（Goal TODO §3） | 孙会话没有 `origin.goal`、运行中计数为 0、plan 的 tick 是 30 秒 | **被破坏**；Q-G4 定后只剩运行中计数为 0 | Q-G5 |
| I8 | Goal 找用户靠 System Notify；UI 观察通知，并可以代用户回答（《方法总结》§11–§12、Notify TODO） | 半订阅只出现在输入里（11-02 原则），UI 要等下一个批次才看到 `[notify]` 行 | **意图不变，但体验缺口暴露出来了**，见 §4 | Q-U* |
| I9 | Goal 头是 Activation 的上下文（Goal TODO §3） | 11-02 原则：背景不能成为一条独立的 input；同一事实不能既是事件又是背景行 | **形式上冲突**：Goal 头是一条独立的 User 消息；Work 结局出现在三处 | Q-G6 |

## 4. "Agent 主动和用户发起会话"放在哪

### 4.1 用户今天会看到什么

| 场景 | 路径 | 用户看到的 | UI session 知道的 |
| --- | --- | --- | --- |
| S1 Goal 发现值得说的事（进展、发现、建议） | `notify send`（notice） | 对话里一张通知卡（抄送），通知中心一条 | 用户下次说话时，背景里有一行 `[notify <id>] <title>`；它自己的历史里没有这句话 |
| S2 Goal 需要用户决定（questions） | input 通知 | 一张可以回答的卡片 | 同上；用户在对话里回答时，UI 按 `UI_NOTIFY_RULE` 代答 |
| S3 默认整理 Goal 的 P1 候选确认（"你每周五都让我…，要不要建成计划？"） | questions | 卡片 | 内置 Goal 没有对话，所以只进通知中心；而这件事本质上是一段可能要来回几句的对话 |
| S4 Goal 派出的 Work 完成 | Goal 吸收 | 什么都没有，除非 Goal 再发一条通知 | 不知道 |
| S5 提醒（《方法总结》§11） | 通知，按强度选渠道 | Telegram / App / 卡片 | 观察 |
| S6 对照：对话派出的 Work 完成 | child 事件 → UI 开 Turn | Jarvis 在对话里说"做完了，结果是…" | 这是它自己说的话，在它的历史里 |

缺口在 S1、S3、S4：Agent 有话要说时，对话里出现的是"系统通知"，而不是"Jarvis 在说话"；这句话也不在 UI 的上下文里，用户接话时 UI 只能靠背景行推测（这正是《方法总结》§12 说的难点）。S6 说明，runtime 已经具备"UI 被事件唤醒后开口"的通路。

### 4.2 已定的约束

- 通知不进 Session Outbox（《方法总结》§5.1）。通知负责触达、强度和需要答复的状态，可以走对话以外的渠道。
- 不能把 A 的输出伪装成 B 的用户输入（§8.2）。交给 UI 的应该是带来源的事件，而不是 user 消息。报告机制也不等于允许会话之间往对方的 Inbox 塞聊天消息（§7.1）。
- 结局是事件，背景只出现在输入里（[11-02 原则](./opendan-plan-simple-async-todo.md) §8.6）。
- Agent 能否作出正确决策，取决于它能否观察到用户实际看到的东西（§12）。
- 一个外部对话绑定一个 UI session（§2.2）。推论：对话里的 Assistant 消息只能有一个作者，否则用户的回复会落进一个不认识那句话的上下文。

### 4.3 选项

**A. 维持现状。** Goal 只发通知，改进只做在 UI 的观察侧（例如把抄送的卡片也写进 UI 的历史）。
- 好处：不加机制，通道边界最清楚。
- 代价：永远是"系统卡片"的口吻；S3 这种需要来回几句的对话做不了；内置 Goal 没有对话可用。

**B. Goal 声明，UI 开口（建议）。** Goal 的 result 增加"要告诉用户的事"。Activation 关闭时，runtime 把它作为事件（来源 `goal:<goal_id>`，事件 `tell`，带 Goal、Activation、正文和级别）交给绑定对话的 UI session。UI 开一个 Turn，用对话口吻说出来，这句话进 UI 自己的历史。用户接话自然落在 UI 里，需要改 Goal 时用已有的 `goal post`。这与 S6 的 Work 结束同形，只是来源从子会话换成了 Goal。
- 好处：对话只有一个声音，上下文完整；复用现有的"事件 → Turn → outbox"通路；Goal 不需要学会聊天。
- 代价：每次开口多一次 UI 推理，成本和打扰频率要管；要定"绑定哪个对话"；UI 正在一个 Turn 里时，消息要延后到下一个批次（实施前核对 UI 的取入规则）。

**C. Goal 自己写进对话**（给 Goal session 一个 outbound）。
- 代价：同一个对话有两个作者；用户的回复进了 UI session，UI 不知道 Goal 说过什么，这就是《方法总结》§12 的问题，而且更严重。**不建议。**

**D. 每个 Goal 一个对话。** MessageHub 把 Goal 当作一个会话实体，Goal 说的话是这个对话里的消息，用户在里面说话就等于 `goal post`。
- 好处：所有权干净，说话和接话的都是 Goal。
- 代价：Goal 的 Turn 是有预算的 Activation，不是聊天；对话数量膨胀；Jarvis 的人格被拆碎；S3 这种"在主对话里顺口问一句"做不到。可以作为 Goal 详情页的补充视图，不作为主路径。

### 4.4 B 的细节（Q-U1 裁决为 B 后再定）

- **U2 绑定哪个对话。** 先用 `Goal.created_by.via` 指的 UI session（与通知 `source.conversation` 走同一条链）。没有的话：(a) owner 最近活跃的 1:1 UI session；(b) MessageHub 里与 Agent 的缺省对话；(c) 不开口，只发通知。建议 (a)，找不到再退到 (c)。
- **U3 Goal 怎么声明。** 两种做法：result 新增字段 `tell: [{text, level?}]`（声明式，Activation 关闭时投递一次，与 questions 同一次提交）；或新命令 `agent-session goal tell`（Activation 进行中立即投递）。建议 result 字段：一次 Activation 最多开口一次，可以去重，不在 loop 里等，崩溃恢复时随 Activation 关闭的提交一起重做。
- **U4 投递方式。** 一种是 push：写进 UI 的队列。另一种是 pull：tell 落在 Goal State 里，UI 侧像 `poll_children` 那样查看"以我为 `created_by` 的 Goal 里还没投递的 tell"，然后生成事件。建议 pull：不往别的会话写东西，符合 §4.2 的第二条约束；投递记录随 UI 的 state 一起提交。
- **U5 级别与渠道。** `level` 沿用 notify 的 `quiet | normal | timely | urgent`。建议：有绑定对话且级别不高于 normal 时，只走 UI 事件；timely / urgent 时，UI 事件之外再发通知负责把人叫回来，但通知不再抄送进同一个对话，以免重复。也可以选"一律只走 UI"或"一律发通知，UI 再补一句"，见 Q-U4。
- **U6 questions 要不要也经 UI 开口。** questions 需要状态（open / answered / withdrawn）、撤回和跨渠道作答，用通知是对的。体验上可以让 UI 同时收到事件，用一句话问出来并引用通知 id（MessageHub 把这个引用渲染成可以回答的卡片），通知就不再抄送进该对话。建议先只做 tell，questions 维持通知，DV 之后再定。
- **U7 UI 侧怎么写。** UI 的 `[input] on_input` 加一个 `source.kind == "goal"` 分支，大意是："你的 Goal <title> 想告诉用户：…（级别）。用对话口吻转达，不要说成是用户说的；需要用户决定的事不要替用户决定；用户的回应如果改变了 Goal，用 `goal post`。"`chat_route.toml` 与 `groupchat_route.toml` 都要看。
- **U8 频率与成本。** 每个 Goal 每个周期设 `max_tells`；UI 这次推理的 token 记在 UI 名下，不进 Goal 的预算，打扰程度由 `max_tells` 控制。
- **U9 Work 完成要不要自动告诉用户。** 不自动。Goal 吸收结局后，自己判断要不要 tell：Goal 的 Work 是 Goal 的手段，不是用户提的请求。对话里直接派出的 Work 维持 S6 现状。
- **U10 内置整理 Goal。** 它没有对话，而 P1 的候选确认正是 S3 的典型。按 U2 (a) 找最近活跃的对话开口，或者维持 questions → 通知。和 [Jarvis 默认 Goal TODO](./jarvis-default-goal-session-todo.md) 的 P1 一起定。

## 5. 不需要裁决、先做的

- **G1** `goal.toml:18` 补 `--title`（不超过 20 个汉字或 40 个英文字符，用用户的语言）。这条要在 MessageHub worksession UI 的改动合入前或同时做，否则 Goal 建 Work 必然参数错误。它就是那份 TODO 的 B2，以那份为准，这里只是登记依赖。

## 6. 待裁决

| Q | 问题 | 选项 | 建议 |
| --- | --- | --- | --- |
| Q-G1 | Activation 内的长任务 | (a) 维持"Activation 跟到底"，加墙钟上限 `activation.max_wall_secs`，到点按 `BudgetExhausted` 关闭并取消可取消的任务，文案按 goal 改；(b) 对齐 Work：回复就关闭 Activation，task 结局打开下一个 Activation | **(a)，缺省 1800 秒**。Activation 是账本单位（每次一个 result），拆开会让 result 不完整；耗时长的事应当进 Work。`goal.toml` 补一句："超过 25 秒的命令会继续作为任务运行，结果在本 Activation 内回来；结束前不要 report；耗时的事交给 Work"。`TaskFollowup` 和 report 被拒的文案对 `SessionKind::Goal` 给另一句 |
| Q-G2 | `ask_user` 在 goal 里 | (a) Goal Session 的工具集去掉它，宿主层拒绝并提示 "use `result.questions`"；(b) 只靠提示词 | **(a)** |
| Q-G3 | questions 答案的回流 | (a) 维持 GoalHost 每 30 秒轮询 → `answer` 事件；(b) Goal session 直接跟这些 `human.input` 任务的 id，与 `ask_user` 走同一套任务跟进，结局打开 Activation（reason `answer`） | **(a) 先不动**。(b) 能删掉一段轮询，但要改 questions 跨 Activation、跨 epoch 的生命周期，放到后续 |
| Q-G4 | Goal 建的 Work 缺省是什么形状 | — | **已定（2026-11-02 用户）：一律 `--behavior work_simple`**，`goal.toml` 不再提 plan。理由（Claude 核对）：Goal 自己就是跨 Activation 的规划者，它做"下一步"的方式是 Work 结束 → child 事件开 Activation → 按结果建下一个 Work，每一步都重新判断；plan 的其余能力（转发用户消息要队列，Goal 建的 plan 没有队列；`todo update` 改运行中的 Work，Goal 可以 stop 再建）对 Goal 用不上；work_simple 的提示词不派子会话，Goal 只有一层子会话。原 Q-G4b（Goal 建的 plan 的 tick）随之删除 |
| Q-G4c | 宿主层要不要强制 | (a) 只改提示词；(b) Goal Session 调 `create-worksession` 时缺省 behavior 改为 work_simple（不带 `--behavior` 也是 work_simple），显式给别的 behavior 照样允许；(c) 拒绝 work_simple 以外的 behavior | **(b)**：提示词漏写时不会退回 plan，又不堵死特殊用法 |
| Q-G5 | 预算口径（Q-G4 定后收窄） | (a) 运行中也计入：准入和 Goal 头读各 Work 的 `usage.jsonl`，而不是 statistics；(b) 另加子树汇总，以防 Work 自己再派子会话 | **(a)**。(b) 等真出现 Goal 下的孙会话再做 |
| Q-G6 | Activation 的输入形状 | (a) Goal 头并入输入模板（`goal_activation` 作为模板变量，像 `background_context` 那样，只有一条 User 消息）；Work 结局只出现一次：事件负责结局，reason 只写 ref 不复述，`<works>` 只列还在跑的，已结束的由事件和 `<recent_works>` 承担；(b) 维持两条消息 | **(a)**，与 11-02 原则一致；fixture `16_goal_session` 重新生成 |
| Q-G7 | Activation 关闭后残留的任务 | (a) Activation 关闭时，Goal 不再跟这些任务：取消可取消的，其余放手，结局不打开 Activation；(b) 结局作为运行机会，经 GoalHost 准入（新 reason `task`） | **(a)**。Activation 里不该有跨 Activation 的任务，跨 Activation 的事就是 Work |
| Q-U1 | 谁在对话里开口（§4.3） | A / B / C / D | **B** |
| Q-U2 | 绑定哪个对话（§4.4 U2） | `created_by` → 最近活跃的 owner 1:1 → 只发通知 | 按左列顺序 |
| Q-U3 | 怎么声明（U3） | `result.tell` 或 `goal tell` 命令 | **`result.tell`** |
| Q-U4 | tell 与通知的关系（U5） | (a) 不高于 normal 只走 UI；timely / urgent 走 UI 加通知（通知不抄送同一对话）；(b) 一律只走 UI；(c) 一律发通知，UI 再补一句 | **(a)** |
| Q-U5 | 投递方式（U4） | push：写 UI 的队列；pull：UI 查 Goal State | **pull** |
| Q-U6 | questions 要不要经 UI 开口（U6） | 不变，或 UI 问出来并引用通知 | **先不变** |
| Q-U7 | `goal.toml` 里的 `notify send` | 保留，或并入 tell 的 `level` | **并入**。模型只说"告诉用户什么、有多急"，渠道由 runtime 决定（执行方式由配置决定）；`goal.toml` 删掉 `notify send` 的用法 |
| Q-U8 | 频率上限（U8） | 每周期 `max_tells` | **每周期 6 次**（缺省周期 1 天），0 表示不开口 |
| Q-U9 | 内置整理 Goal（U10） | 随 Jarvis 默认 Goal 的 P1 | 按 Q-U2 找最近活跃的对话 |

## 7. 实施清单（草案；裁决后回写）

### A. lib_opendan / agent_tool

- A1（Q-G1）`ActivationBudget` 加 `max_wall_secs`；`goal::activation_exhausted` 判断墙钟；`TaskFollowup` 增加 goal 口径的文案（或由 libopendan 按 SessionKind 选择）；`reports.rs` 拒绝文案按 goal 区分。
- A2（Q-G2）Goal Session 的工具集剔除 `ask_user`，落点要看 baseline 在哪里按会话过滤，实施前核对。
- A3（Q-G5）`current_usage` 与 `work_admission` 读运行中 Work 的 `usage.jsonl`；`close_activation` 吸收时仍用 statistics（那时 run 已结束）。
- A3b（Q-G4c）`api.rs` 建子会话时，父是 Goal Session 且未给 `--behavior` → behavior 取 `work_simple`。
- A4（Q-G6）Goal 头作为输入模板变量；删掉独立的那条 User 消息；Work 结局去重。
- A5（Q-G7）Goal Activation 关闭时清理 `watched_tasks`（`commit_run_end` 中 goal 分支）。
- A6（Q-U1/U3/U5）`GoalResult.tell`（`reports.rs:297-312` 校验）、`GoalState.tells`（投递状态）；UI 侧的 poll（与 `poll_children` 同款），找"`created_by` 指向我的 Goal"或 U2 的回退；事件 key 形如 `goal:<id>:tell:<epoch>:<seq>`，保证幂等。
- A7（Q-U4）timely / urgent 级别的 tell 同时发通知，不抄送同一个对话（`source.conversation` 置空，或新增"不抄送"选项）。
- A8（Q-U8）`PeriodBudget.max_tells`。
- A9 schema 用 `xagent schema doc/opendan/protocol/schema` 重新生成，fixtures 重新生成（只保留真正变化的文件）。

### B. Jarvis 包

- B1 `goal.toml`：G1；Q-G4（`create-worksession` 示例写 `--behavior work_simple`，删"plan"的说法）；Q-G1 的长任务那句；Q-U3 的 tell；按裁决删去 `notify send` 与 `ask_user` 的说法。
- B2 `chat_route.toml` / `groupchat_route.toml`：`goal` 事件分支（U7）；`template.rs:60` 与 `:63` 的 UI 目标文本与实际对齐。
- B3 `analyze_intent.toml:109`："what the user will see next"里 Goal 的部分改为"Goal 会在这个对话里告诉你…"。
- B4 `agent.toml`：`[session.goal.budget]` 新字段。
- B5 `memory_maintenance.toml`：不动，除非 Q-U9 改变。

### C. 文档

- C1 [Goal Session TODO](./opendan-goal-session-todo.md) §11 之后加一个指向本单的指针。
- C2 《方法总结》§11–§12 补"Goal 开口经 UI"一段；`Session Input Protocol` 补 `goal` 事件；手册 `opendan_agent_dev.md` 的 Goal 段落。

### D. 测试与 DV

- D1 `tests/goal_session.rs`：Activation 墙钟上限；残留任务不开 Activation；运行中 Work 的 token 计入准入；Goal 不带 `--behavior` 建出 work_simple；tell 投递与幂等（崩溃点放在 `goal:after_activation_close`）；UI poll。
- D2 DV（本机 DV Zone，`./debug_jarvis.sh devtest --agent xiaobai.test.buckyos.io`）：在对话里建一个 Goal → Activation 产出 tell → MessageHub 里 Jarvis 用对话口吻说出来 → 用户回复 → UI `goal post` → 下一次 Activation 的 reason 是 `message`；Goal 建出 work_simple，且运行中 period tokens 随之增长。DV 结束后归档测试 Goal。

## 8. 关联

- [llm_context 冻结 TODO](./llm-context-freeze-todo.md)：原则 8、F18、F20、§3.2.1
- [简单 Work Session 全异步化](./opendan-simple-worksession-async-todo.md)：§3.3、§7（第三刀 UI）
- [plan_simple 多 Turn 化](./opendan-plan-simple-async-todo.md)：§8.5（Goal 建的 plan tick 30 秒）、§8.6（半订阅只在输入里）
- [MessageHub worksession UI TODO](./messagehub-worksession-ui-todo.md)：Q3 `--title` 必填、Q9 Goal UI 不做、B2
- [Goal Session TODO](./opendan-goal-session-todo.md)、[Jarvis 默认 Goal TODO](./jarvis-default-goal-session-todo.md)（P1 候选确认）、[Agent 通知 TODO](./opendan-agent-notify-todo.md)
- [Agent 主动给用户发"消息"的方法总结](../doc/opendan/Agent%20主动给用户发_消息_的方法总结.md)：§5、§7.1、§8.2、§10–§12
