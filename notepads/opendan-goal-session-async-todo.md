# Goal Session 对齐全异步模型与"Agent 主动找用户"TODO（agent-session 重构 Goal 刀）

日期：2026-11-02

状态：**待用户 review，未实施。已定：Q-G4（2026-11-02 用户：Goal 建的 Work 用 work_simple 就够了；Q-G4b 随之删除，Q-G5 收窄）。Q-U1 用户 11-02 给出方向：设一个 default chat session，Agent 的主动聊天都出现在这里，泛触达由通知带 URL 打开它（§4.3 B、§4.4、§4.5 已按此改写）。第四、五轮补了 §5 典型场景与归属（"维护 timer" 按用户澄清改为系统默认 Goal"日程管家"；Q-S1–Q-S10 待裁决）；其余未裁决。** 用户 2026-11-02 要求先写 TODO 整体过一遍：核心是两刀重构之后 Goal Session 的设计意图有没有变化；最大的体验问题是"Agent 主动和用户发起会话"应该放在哪。Claude 对照 [llm_context 冻结 TODO](./done/llm-context-freeze-todo.md)、[简单 Work Session 全异步化](./done/opendan-simple-worksession-async-todo.md)（第一刀，§9 是实施记录）、[plan_simple 多 Turn 化](./opendan-plan-simple-async-todo.md)（第二刀，§8 是实施记录）、[Goal Session TODO](./done/opendan-goal-session-todo.md)（第一段，§11 是实施记录）、[Agent 主动给用户发"消息"的方法总结](../doc/opendan/Agent%20主动给用户发_消息_的方法总结.md)（下称《方法总结》）与当前源码（含工作区未提交改动）写成。

范围：Goal Session（class `goal`、`goal.toml`、`memory_maintenance.toml`）的运行时语义、它派生的 Work 的形状与预算、它与用户之间的通道。两刀都写明"goal 会话不动"，本单就是补这一刀。UI 会话只涉及 §4"由谁在对话里开口"这部分；UI 的中间回复、提问撤销仍归第三刀（第一刀 §7）。

路径相对 `buckyos/`；`lib_opendan/` 指 `src/frame/lib_opendan/src/`，`jarvis/` 指 `src/apps/jarvis_agent/agent/`。工作区有 MessageHub worksession UI 的未提交改动（`create-worksession --title` 必填等），实施前复核，不得覆盖。

## 0. 结论先行

1. **Goal 的主意图没变。** 它仍然负责长期观察、判断、按预算行动；Activation 是有预算的运行机会；Goal 不等 Work，Work 的结局由后续 Activation 吸收；需要人决定时声明 `questions`。新模型（loop 不等待、等待都是任务、结局作为输入回来）其实是把 Goal 一开始就采用的"靠输入驱动、不挂起"推广到了 Work。
2. **变了的是 Goal 周围的东西，而且是被默认值悄悄改掉的：**
   - Work 的形状：`[session.work]` 的缺省 behavior 从 PLAN/DO 变成了 `plan_simple`。Goal 原本要建的是"一件有界的事"，现在建出来的是"一个 plan 加若干 Todo 子会话"（§3 I6）。
   - Goal 的预算口径因此失效：孙会话不计入、Work 运行中计数恒为 0、plan 每 30 秒推理一次（§3 I7）。**Q-G4 已定：Goal 建的 Work 一律 work_simple**，孙会话与 plan tick 两条随之消失，只剩"运行中恒为 0"。
   - "Turn"在 Work 和 Goal 里的含义分叉了：Work 的 Turn 是一次进度，任何回复都会关闭它；Goal 的 Turn 是一次 Activation，只有 report、预算耗尽或 stop 才关闭。共享的提示文案是按 Work 写的（§3 I4）。
   - Work 现在也能长时间跟着自己发起的任务，"有界等待归 Work、长期等待归 Goal"的边界需要重新说清（§3 I2）。
3. **"Agent 主动找用户"今天只有 System Notify 一条路。** 对话里出现的是通知卡片的抄送；UI session 只在用户下一次说话时从背景行看到它，自己从不开口。对照之下，对话派出的 Work 结束时，UI session 会被事件唤醒，用自己的口吻转述结果；Goal 没有这条路。
   - **用户 11-02 的方向**：通知特意不和聊天窗口放在一起，负责泛触达；但 Agent 一旦要"和用户说话"，就必须回答"在哪个上下文里说"。所以设一个 **default chat session**，Agent 的主动聊天都出现在这里，用户回话时看到的历史自然包含这条消息。BuckyOS 一定能用 URL 打开 chat session，所以消息发出后可以再发一条泛通知，带上 URL，点开就进这个对话。
   - **Claude 补充**：这句话必须由 default session 的 UI session 在一个由事件打开的 Turn 里写出，Goal 只提供素材。这样上下文里是"事件输入 → assistant"：不会出现两条 assistant 相连，UI 知道自己为什么这么说，也不冒充用户（§4.5）。§4 展开。
4. **典型场景（§5，用户第四轮）。**
   - timer 本身到点不需要推理，不是 Goal；"维护 timer"（按用户意图设置、取消、调整当天的实例）由系统默认 Goal"日程管家"每天凌晨跑一次 LLM 完成。LLM 只是增强：它没跑时 timer 照规则触发。
   - 到点需要判断的都是 Goal：例行任务、条件监视、智能提醒、各类目标。它们的区别只在 Goal 的定义和时间表；每个例行任务一个 Goal。
   - 系统默认 Goal 负责 Agent 自身的维护、用户全部 timer 的日常维护（日程管家），以及"发现 → 在 default chat 里确认 → 落地"；不执行用户的例行任务。
   - 缺口：timer 对象、按墙上时间与时区的 Goal 时间表、follow_up、意图分析的定时入口（今天返回 `unsupported`）。

## 1. 用户思路（2026-11-02 原话要点 → 本单条目）

1. 先写一个 TODO，整体过一下。→ 本单
2. 核心还是新的修改对 Goal Session 的设计意图有没有变化。→ §3
3. 最大的用户体验问题是"Agent 主动和用户发起会话"应该在哪。→ §4
4. （第二轮）notify 特意不和聊天窗口在一起，是为了泛触达。但 Agent 一旦有"和用户说话"的需求，就一定有"在哪个上下文里说"的问题。BuckyOS + OpenDAN 的目标是普通用户的 persona agent，而且已经做了大量设计让 chat session 比较轻，所以可以有一个 default chat session：Agent 的主动聊天在这里出现；用户回话时，看到的 message history 自然包含这条消息（这里可能有双 assistant message 的问题）。→ §4.3 B、§4.4 U2、§4.5
5. （第三轮）BuckyOS 的好处是一定能用 URL 打开 chat session：Agent 给用户发了消息后，还可以触发泛通知，通知里带 URL，点击就能打开这个 chat session。→ §4.4 U5、Q-U4
6. （第四轮）之前 goal session 的设计文档里有没有典型场景？理解了几个，觉得不够，要补充；区分哪些是系统自动的 default goal session 就能实现的，哪些是分析用户意图自动创建、或通过 UI 手工创建的。用户举了三个：维护 timer（触发时给用户一个通知）；维护定时 task，触发时启动一个 LLM 判定器，判定通过就构造 worksession 完成任务（比如定时读邮件）；根据用户设定的目标，主动做一些事情来逼近目标。→ §5
7. （第五轮）维护 timer 是指根据用户的意图设置和取消 timer。比如工作日上午 10 点的一个 timer，在有 LLM 的情况下，可以每天凌晨跑 LLM 判断今天的 timer 要不要设置（比如 Agent 已经知道用户今天请假了，就会自动取消这个 timer）。→ §5.2 场景 1、A6、Q-S8–Q-S10

同日上一轮 review 发现的 5 个具体问题（`--title`、plan_simple 预算漏算、task 文案、`ask_user`、残留任务开 Activation）并入 §3 与 §7。

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
| Goal 的时间表 | `GoalScheduleSpec` 只有 `every_secs`、`min_interval_secs`、`start_on_create`，时间槽按 UTC 对齐。没有按墙上时间与时区的日历规则，也没有一次性的"下次再看" | `protocol/goal.rs:277-289` |
| 定时请求的入口 | analyze_intent："something that has to run at a later time (scheduled): not supported yet → `unsupported`" | `analyze_intent.toml:36` |
| workflow | 有 `Once / Cron / RunEvery`、misfire、TaskMgr 持久化。执行器有 `workflow.send_message`（发消息给 `to`，要求非空 `text`），没有发通知的目标；`opendan.command` / `agent.delegate` 在 OpenDAN 侧没有执行链；创建计划没有幂等键 | `src/kernel/workflow/src/scheduled_task_manager.rs:1016,1207,1256`、Jarvis 默认 Goal TODO §5.3 |

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

**B. Goal 声明，default chat session 的 UI 开口（用户 11-02 方向，Claude 赞成）。** Goal 的 result 增加"要告诉用户的事"。Activation 关闭时，runtime 把它作为事件（来源 `goal:<goal_id>`，事件 `tell`，带 Goal、Activation、正文和级别）交给 **default chat session**（U2）。那里的 UI 开一个 Turn，用对话口吻说出来，这句话进 UI 自己的历史。用户接话自然落在同一个上下文里，需要改 Goal 时用已有的 `goal post`。这与 S6 的 Work 结束同形，只是来源从子会话换成了 Goal。消息发出后，runtime 再发一条泛通知，带上打开这条消息的 URL（U5）。
- 好处：Agent 主动找用户永远在同一个地方，用户好预测；对话只有一个声音，上下文完整；复用现有的"事件 → Turn → outbox"通路；Goal 不需要学会聊天；通知只管把人叫回来，内容不重复。
- 代价：每次开口多一次 UI 推理，成本和打扰频率要管；Goal 若是在别的对话里创建的，创建时的上下文不在 default 里（事件带 goal 引用，default 可以 `goal show`）；UI 正在一个 Turn 里时，消息要延后到下一个批次（实施前核对 UI 的取入规则）。

**C. Goal 自己写进对话**（给 Goal session 一个 outbound）。
- 代价：同一个对话有两个作者；用户的回复进了 UI session，UI 不知道 Goal 说过什么，这就是《方法总结》§12 的问题，而且更严重。**不建议。**

**D. 每个 Goal 一个对话。** MessageHub 把 Goal 当作一个会话实体，Goal 说的话是这个对话里的消息，用户在里面说话就等于 `goal post`。
- 好处：所有权干净，说话和接话的都是 Goal。
- 代价：Goal 的 Turn 是有预算的 Activation，不是聊天；对话数量膨胀；Jarvis 的人格被拆碎；S3 这种"在主对话里顺口问一句"做不到。可以作为 Goal 详情页的补充视图，不作为主路径。

### 4.4 B 的细节

- **U2 default chat session 是什么（用户 11-02 方向）。**
  - 每个 (Agent, owner) 一个，就是 MessageHub 里点开 Agent 默认进入的那个原生 1:1 对话。
  - Agent 初始化时就建好并绑定，不等用户先说话。今天 UI session 由对话的第一条消息按 route_key 创建，实施前核对这条路径。
  - 群聊里永远不主动开口。
  - 以后可以让用户把默认对话改到 Telegram 等 tunnel 对话；这是个人设置，跟着账号走。
  - Goal 在哪个对话里创建，不影响落点。事件带 goal 引用，default session 用 `goal show` 查。
  - 区分"回报"和"主动"：对话里派出的 Work 结束，仍在原对话转述（S6 不变），因为那是对请求的答复；Goal 的 tell 是 Agent 自己发起的，一律去 default。
- **U3 Goal 怎么声明。** 两种做法：result 新增字段 `tell: [{text, level?}]`（声明式，Activation 关闭时投递一次，与 questions 同一次提交）；或新命令 `agent-session goal tell`（Activation 进行中立即投递）。建议 result 字段：一次 Activation 最多开口一次，可以去重，不在 loop 里等，崩溃恢复时随 Activation 关闭的提交一起重做。
- **U4 投递方式。** 一种是 push：写进 default session 的队列。另一种是 pull：tell 落在 Goal State 里，default session 像 `poll_children` 那样查看"本 Agent 各 Goal 里还没投递的 tell"，然后生成事件。建议 pull：不往别的会话写东西，符合 §4.2 的第二条约束；投递记录随 UI 的 state 一起提交。多个 Goal 同时有 tell 时并进同一个批次，UI 一次说完。
- **U5 tell 与通知（用户 11-02 方向）。**
  - 内容只在 chat 里。通知只负责把人叫回来：一句摘要，加一个打开 default chat 并定位到这条消息的 URL。现成的深链是 `/messagehub?entityId=<agent>&sessionId=<sid>&messageId=<msg>`；`?notify=<id>` 也已经能解析到 {会话, 消息} 目标（`src/frame/desktop/src/app/messagehub/MessageHubRoute.tsx:54-58`）。
  - 通知不再抄送进这个对话，否则卡片和消息会重复。
  - 由 runtime 在 UI 消息投递成功、拿到 msg_id 之后发出，模型不调 `notify send`（Q-U7）。
  - 级别沿用 notify 的 `quiet | normal | timely | urgent`。quiet 不发通知，对话里的未读就够了；normal / timely / urgent 按级别选渠道。
  - 用户已经在看这个对话时，推送是噪音。可以让通知在消息被读到后自动 ack 或撤回（msg-center 有已读状态，实施前核对），或者延迟几秒再发、发前检查是否已读。
- **U6 questions 要不要也走 chat。** 同样的形状可以用于 questions：UI 在 default chat 里问出来，通知带 URL，用户在对话里答，UI 用 `goal post` 或 `notify answer` 交回。但 questions 需要状态（open / answered / withdrawn）、撤回和带选项的作答页面。建议先维持 input 通知，只把抄送的落点从 `created_by` 的对话改到 default chat（`notify.rs::conversation_of` 的回退），UI 观察并可代答；DV 之后再决定是否改成 chat 优先。
- **U7 UI 侧怎么写。** UI 的 `[input] on_input` 加一个 `source.kind == "goal"` 分支，大意是："你的 Goal <title> 想告诉用户：…（级别）。用对话口吻转达，不要说成是用户说的；需要用户决定的事不要替用户决定；用户的回应如果改变了 Goal，用 `goal post`。用户刚说过不要打扰、或者正在谈别的事时，可以这次不说（Q-U12）。"只改 `chat_route.toml`，群聊不收 tell。
- **U8 频率与成本。** 每个 Goal 每个周期设 `max_tells`；UI 这次推理的 token 记在 UI 名下，不进 Goal 的预算，打扰程度由 `max_tells` 控制。
- **U9 Work 完成要不要自动告诉用户。** 不自动。Goal 吸收结局后，自己判断要不要 tell：Goal 的 Work 是 Goal 的手段，不是用户提的请求。对话里直接派出的 Work 维持 S6 现状。
- **U10 内置整理 Goal。** 有了 default chat 就有了落点：P1 的候选确认（S3）可以在 default chat 里问。和 [Jarvis 默认 Goal TODO](./jarvis-default-goal-session-todo.md) 的 P1 一起定。
- **U11 事件打开的 Turn 不发占位。** Turn 超过 `placeholder_delay` 还没结束就会先发一个"处理中"气泡（`runner/outbound.rs:6-10`），事件打开的 Turn 也会这样，用户什么都没说就看到"正在处理"。规则：不是由用户消息打开的 Turn 不发占位，回复作为一条新消息发出。
- **U12 允许 UI 不说。** 事件打开的 Turn 可以没有回复（今天 chat_route 的结构化回复是否允许空 `reply`，实施前核对）。不说的原因写回 Goal State（tell 状态记为 `skipped` 并附原因），下一次 Activation 能看到。这是《方法总结》§12 反过来用：开口的一方要先看到用户此刻在做什么。
- **U13 压缩后保留标记。** chat session 很轻，用户几天后回复时历史可能已经压缩。事件正文要短，主要带引用（goal_id、epoch、seq）；压缩后的摘要里要保留"替 Goal X 说过…"和 goal id，否则用户一句"好的"就接不上了。实施前核对 UI 的压缩规则能不能按条目保留。

### 4.5 "双 assistant message"

用户 11-02 担心的问题，分两层看：

1. **上下文层（真正的问题）。** 如果把 Goal 写好的文本直接追加进 UI 的历史，上下文里会出现两条 assistant 相连，而且第二条不是 UI 生成的：用户问"你为什么这么说"，它答不上来；provider 对相邻同角色消息的处理也不统一。**解法**：主动消息必须是 default session 的一个 Turn 的产出，由事件触发：

   ```text
   … A1 | <<event goal:X tell>> Goal X 想告诉用户：… | A2
   ```

   交替不被破坏；事件是带来源的输入，不冒充用户（《方法总结》§8.2）；UI 知道自己为什么说了 A2。这和今天 Work 结束时 UI 转述结果的形状一样。

   省钱的变体：Goal 写好最终文本，runtime 直接记成"事件 + assistant 原文"，不推理。交替和诚实都保住了，但丢了两样东西：一是判断（用户一小时前在这个对话里说过别打扰，或者正在谈别的事），二是口吻。建议推理，成本用 `max_tells` 和批次合并控制（Q-U10）。

2. **显示层。** 对话里连续两个 Jarvis 气泡，在 IM 里很正常。要处理的只有三件事：
   - 凭空出现"处理中"（U11）；
   - 和通知卡重复（U5：通知不抄送进对话）；
   - 和用户消息撞车：事件只在批次边界被取入，赶在推理前就和用户消息并成一条回复，否则下一个 Turn 再说。两种都可以接受。

## 5. 典型场景与归属（2026-11-02 第四轮）

已有的场景散在四处：

- 架构 §8：8.1 目标驱动的提醒、8.2 为他人提供持续服务、8.3 / 8.4 软件维护；
- 架构 §9：Agent 自身目标，包括 Memory 整理、经验结晶、XLLM 结晶、Self Correct；
- 《方法总结》§11：智能提醒（日程 Goal 安排一次性触发 → 到点 Work 判定 → 通知）；
- [Jarvis 默认 Goal TODO](./jarvis-default-goal-session-todo.md) §5.2："动作类型决定谁执行，触发类型决定何时执行；每天重复不自动意味着 Goal，需要跨时间判断和调整才是 Goal"。

这些材料都偏重"Goal 怎么想"，缺两样东西：一是按"谁创建、到点谁跑、怎么告诉用户"把场景排成一张表；二是把 timer 这类到点不需要推理的东西和 Goal 分开。

### 5.1 三个维度

- **谁创建**
  - **S 系统默认**：Agent 初始化时播种的内置 Goal。今天只有 `builtin:memory_maintenance`。
  - **D 默认 Goal 发现**：内置 Goal 从感知里发现潜在意图 → 形成候选 → 在 default chat 里确认 → 落地。这是 Jarvis 默认 Goal TODO 的 P1，未做。
  - **I 意图分析**：用户在对话里提出，由 `try_analyze_user_intent` 创建；参数完整的明确指令也可以由 chat 直接创建（Q-S7）。
  - **M 手工**：Agent Home 的 Goals 页、提醒与计划列表，或 kRPC。
- **到点谁跑**（决定成本）
  - **无 LLM**：机械执行，发通知或跑固定流程；
  - **Goal Activation**：带着跨次的状态和历史判断要不要做；要做的事轻就自己做，重就建 work_simple。
- **怎么告诉用户**
  - 通知：泛触达，URL 打开 default chat；
  - tell：在 default chat 里开口（§4）。

### 5.2 用户提的三个场景

#### 场景 1：维护 timer（按用户意图设置、取消、调整 timer；到点给用户一个通知）

用户 11-02 第五轮澄清："维护 timer"是指根据用户的意图设置和取消 timer。比如一个工作日上午 10 点的 timer：有 LLM 时，每天凌晨跑一次 LLM，判断今天这个 timer 要不要设；Agent 已经知道用户今天请假，就自动取消。

分成两层：

- **timer 本身：到点不需要推理。** 它是一个机械对象，规则可以是一次性的（明天 9 点）或日历重复（工作日 10 点）。要存原始时刻、用户时区和解析后的绝对时刻（Jarvis 默认 Goal TODO §5.2 的触发类型表）。到点发通知，级别由 timer 定义，URL 打开 default chat；通知抄送进 default chat 成为卡片，UI 观察它，用户回"推迟半小时""不用了"时由 UI 代为 snooze / cancel。到点没有 LLM，所以不存在 §4.5 的问题。
- **维护 timer 的那个 LLM：系统默认 Goal"日程管家"（A6）。** 每天在用户本地时间凌晨跑一次 Activation，检视今天要发生的所有 timer，结合 Agent 已知的情况（请假、出差换时区、节假日、今天已经做过）决定：今天这一次照常、跳过、改时间，还是先问用户。一个 Goal 管全部 timer 在这里是合适的：
  - 每天一次推理就覆盖所有提醒，比每个提醒一个 Goal 便宜得多；
  - "今天请假"这类判断本来就是全局的，一次看完所有提醒最自然；
  - Activation 很轻：读状态、改 timer、可能说一句话，不建 Work，也不需要各自不同的 cmd_skill。

  场景 2 里反对"一个 Goal 管全部例行任务"的那些理由（排队、上下文混杂、预算互相饿死、要的 skill 不同），在这里都不成立。

几条规则：

- **LLM 是增强，不是前提（Q-S8）。** 循环规则才是真相，日程管家只能改"今天这一次"。没有 LLM（模型不可用、预算用完、Agent 没在跑）时，timer 照规则触发。这对应用户说的"在有 LLM 的情况下"。另一种做法是日程管家每天把当天的 timer 实例化出来，不实例化就不响；这样日程管家一旦没跑，当天的提醒会静悄悄地全部丢掉，不建议。
- **判断依据是 timer 的用途和用户确认过的偏好，不是推断出来的假设**（架构 §8.1："不应擅自假定请假就不需要活动"）。timer 记录里要有 `purpose`，以及可选的、用户说过或确认过的跳过条件（比如"请假日不提醒"）。拿不准时就照常触发，或者在早上问一句。站会提醒遇到请假可以跳过；"提醒我活动十五分钟"遇到请假就不一定。
- **改了就说（Q-S9）。** 跳过或改时间之后，在 default chat 里用 quiet 级别的 tell 说一句，比如"今天你请假，10 点的站会提醒我先停了；要的话说一声"，让用户随时能撤回。一天里有多条改动就合成一句。
- **当天才知道的变化走对话。** 用户 9:30 才在对话里说"今天请假"，由 UI 当场处理（`timer list --today`、`timer skip`），不等第二天的检视。
- **检视时间。** 默认凌晨 3 点（用户本地时间）；最早的一个 timer 比这还早时，提前到它之前 1 小时。这依赖 Q-S3 的 cron + 时区。

谁创建 timer：

- I：用户明确说"提醒我…"时由 chat 直接建，缺时区或时间有歧义才问（Q-S7）；
- M：手工建；
- D：默认整理 Goal 从对话里发现"下周三要交税"，在 default chat 里问要不要提醒，确认后建；
- 日程管家自己不新建循环 timer，只改当天的实例。业务 Goal 需要的话也可以建 timer，这类 timer 带 goal 引用，Goal 归档时一起撤销。

现状：做不了。

- OpenDAN 没有 timer 对象；
- analyze_intent 对定时请求返回 `unsupported`；
- Goal 的调度只有 `every_secs`，没有按本地时间的凌晨检视；
- workflow 有 Once / Cron，但执行器只有 `workflow.send_message`（发消息，不是通知），没有幂等键（Jarvis 默认 Goal TODO §5.3），也不能单独跳过或改动一次发生（类似 iCalendar 的 EXDATE / RDATE）。

#### 场景 2：维护定时 task（到点启动 LLM 判定器，判定通过就建 Work，例如定时读邮件）

- **本质**：就是 Goal 的 Activation 循环（架构 §7.5）配上一个固定时间表。Activation 就是判定器：观察 → 判断 → `no_action` 或 `created_work`，Work 去做。它和场景 3 的区别只在于目标是一件固定的例行事，而不是一个要逼近的状态。
- **为什么用 Goal，而不是到点直接建 Work**：
  - **跨次状态**：上次读到哪封、哪些已经汇报过、对方回没回。Work 每次都是新的，这些信息只能塞进任务书或外部状态；Goal State 的 `pending` / `distance` 天然承载。
  - **判定要用工具**："有没有新邮件"要调一次命令才知道。纯 decision 模型（`ApiType::Decision`）不能调工具，只能做"给定摘要，现在要不要跑"的预检（Jarvis 默认 Goal TODO §7，默认关闭）。
  - **配套现成**：预算、pause / resume、Work 跟踪、Goals 页面、tell 到 default chat，都已经有了。
- **每个例行任务一个 Goal，还是一个默认 Goal 管全部**：建议每个一个（Q-S2）。一个 Goal 管全部有这些问题：
  - 同一个 Goal 一次只跑一个 Activation，慢的任务会拖住别的；
  - 上下文混着互不相干的任务；
  - 预算共用，一个失控就饿死其它；
  - 不能单独暂停某一个任务；
  - 各个任务要的 cmd_skill 不同。

  合并唯一的好处是能做"今天是假期，所有工作日任务都跳过"这类全局判断，而这可以由各个 Goal 读同一份用户状态（Memory、日历）解决。
- **成本**：每次 fire 一次 Activation。高频的例行事（比如每 5 分钟一次）可以调大间隔、给这类 Goal 用便宜模型，或打开 decision 预检（Q-S5）。
- **不需要判定的确定流程**（每周备份指定目录）：交给 workflow 的确定流程，无 LLM，只在流程已经存在且已授权时用（Jarvis 默认 Goal TODO §5.2）。
- **谁创建**：I（意图分析今天返回 `unsupported`，改为 `goal_created` 并带时间表）、M；D 可以提议，例如"你每周五都让我整理周报，要不要设成例行任务？"，这正是 S3。
- **现状**：一半能做。GoalHost 的 `every_secs` 能按固定间隔触发，Goal → work_simple 也是现成的。缺的是：
  - 按墙上时间与时区调度：`every_secs` 是 UTC 对齐的时间槽，表达不了"每天 8 点"；
  - 意图分析的入口；
  - tell。

#### 场景 3：根据用户设定的目标，主动做事逼近目标

- **本质**：达成型或维护型 Goal（架构 §7.3），这是 Goal 的本意。触发不只是时间：
  - Work 结束；
  - 用户在 default chat 里回话（经 UI `goal post`）；
  - 通知的答复；
  - 订阅的事件；
  - Goal 自己声明的"下次什么时候再看"（follow_up，架构 §7.8、Goal TODO §9.5，未做），由 runtime 安排。
- **例子**：
  - 减重到 130 斤（架构 §8.1，不给医学结论）；
  - 三个月学会一首曲子；
  - 为来访的朋友安排行程（§8.2，受益者不是主人）；
  - 推进一个 Issue 到合并（§8.3）；
  - 维护一个软件项目（§8.4）。
- **谁创建**：I（`goal_created`，就绪性按架构 §5.4）、M。D 只提议，不擅自建：推断出的长期职责必须经用户确认（Jarvis 默认 Goal TODO §5.1）。
- **现状**：能做（`goal create` + `--every` + 事件）。缺 tell（§4）、follow_up、按墙上时间的调度。

### 5.3 补充场景

| # | 场景 | 例子 | 创建 | 到点谁跑 | 告诉用户 | 现状 | 出处 |
| --- | --- | --- | --- | --- | --- | --- | --- |
| A1 | Memory 整理 | 感知 → 认知 | S | Goal Activation；宿主 gate 先挡掉无事可做的时间槽 | 一般不说 | P0 已实施 | 架构 §9.2 |
| A2 | 意图与计划发现 | 对话里说过"下周三交税""想减肥" → 提议 timer / 例行任务 / Goal | S（并入 A1 的 P1） | Goal Activation | 在 default chat 里问，确认后落地 | 未做 | Jarvis 默认 Goal TODO §5 |
| A3 | 经验结晶 | 修正、清理、提炼 cmd_skill | S | Goal Activation，复杂的建 Work | 一般不说；删手册前可以说 | 未做 | 架构 §9.3 |
| A4 | Self Correct / 工具补齐 | 汇总 `command not found` 一类信号，补工具或明确不做 | S | 同上 | 补了新能力时说一句 | 未做 | 架构 §9.5 |
| A5 | Agent 健康巡检（新增建议） | runtime 不可用、AICC 额度将尽、例行任务连续失败、通知发不出去 | S | 宿主确定性检查，异常时才开 Activation | default chat 说明，必要时加通知 | 未做；self_check 退役后没有接手者 | Goal TODO §9.3 |
| A6 | 日程管家：维护用户的全部 timer | 工作日 10 点的站会提醒，今天请假就跳过；出差换了时区就改时间 | S | 每天用户本地凌晨一次 Goal Activation，只改当天的实例 | 改了就在 default chat 里用 quiet tell 说一句 | 未做 | 场景 1、架构 §8.1、《方法总结》§11 |
| B1 | 提醒（timer） | 明天 9 点带护照；工作日 10 点站会；每周一交周报 | I / M / D / 业务 Goal | 无 LLM；当天的实例由 A6 维护 | 通知 + default chat 卡片 | 做不了 | 场景 1 |
| B2 | 固定流程 | 每周备份某个目录 | I / M | 无 LLM（workflow） | 失败时通知 | workflow 侧部分有 | Jarvis 默认 Goal TODO §5.2 |
| B3 | 定时有界任务 | 每天 8 点汇总新邮件；每周五生成周报草稿 | I / M / D | Goal Activation 判定 + Work | tell | 一半 | 场景 2 |
| B4 | 条件监视 | 机票低于 X 告诉我；网站挂了告诉我；PR 合并了告诉我 | I / M | Goal Activation（定时轮询或事件订阅）；满足时 tell，达成型可以 `achieved` | tell，紧急的加通知 | 一半 | 架构 §12.7；analyze_intent "Open-ended waiting or watching is never a work session" |
| B5 | 智能提醒 | 工作日 16 点提醒活动，但看假期、看当天是否已经动过 | I / M 建 timer（带 `purpose` 与跳过条件） | 并入 A6：B1 timer + 日程管家当天的判断，不再单独建 Goal | timer 的通知；跳过时 quiet tell | 做不了 | 架构 §8.1、《方法总结》§11 |
| C1 | 个人达成目标 | 减重、学一首曲子 | I / M | Goal（定期 + 用户回话） | tell：进展与提问 | 能做，缺 tell | 架构 §8.1 |
| C2 | 为他人提供持续服务 | 朋友来访的接待 | I / M | Goal；受益者对话里的信息经授权进入 | 对主人 tell，对朋友经朋友自己的对话 | 部分能做，跨用户授权未做 | 架构 §8.2 |
| C3 | 推进一件事到结束 | 某 Issue 到合并；一份申请到获批 | I / M | Goal（事件 + 定期），具体改动建 Work | tell | 能做 | 架构 §8.3 |
| C4 | 持续职责 | 维护一个软件项目；照看家里的 NAS | I / M | Goal，Skills 里写分类与优先级规则 | tell | 能做 | 架构 §8.4 |
| C5 | 等外部回复 | 发邮件请人确认，之后跟进 | I（由 Work 或对话转出） | Goal 观察收据关联的回复 | tell | 做不了：`post_message` 收据未做 | 《方法总结》§9.4、§10 |

XLLM 结晶（架构 §9.4）面向企业客服，不作为个人 Agent 的默认 Goal。

### 5.4 由此得出的划分

- **系统默认 Goal 负责 Agent 自身的维护与发现（A1–A5），以及用户全部 timer 的日常维护（A6 日程管家）**，不执行用户的例行任务。A2 是"系统默认"与"用户创建"之间的桥：它负责提议，用户在 default chat 里确认，落地成 B / C 类对象。
- **timer 不是 Goal**，是一个到点无 LLM 的对象（B1、B2）。它可以由 chat、意图分析、手工、A2 或业务 Goal 创建；当天的实例由 A6 维护，A6 没跑时 timer 照规则触发。
- **timer 的维护只用一个 Goal，例行任务每个一个 Goal。** 前者每天一次、动作轻、判断全局；后者每次要用工具判定、带各自的跨次状态与 skill（§5.2 场景 2）。
- **只要到点需要按情况做事，就是 Goal**：例行任务（B3）、条件监视（B4）、各类目标（C*）。它们的差别只在 Goal 的定义（lifecycle、`achieved_when` / `healthy_state`）、时间表和 Skills，不需要新的 Session 类型。
- **Goal 调度因此缺三样**：按墙上时间与时区的日历时间表（cron）、Goal 自己声明的 follow_up、B1 要用的 timer 对象（还要能单独改一次发生）。见 Q-S1–Q-S10。

## 6. 不需要裁决、先做的

- **G1** `goal.toml:18` 补 `--title`（不超过 20 个汉字或 40 个英文字符，用用户的语言）。这条要在 MessageHub worksession UI 的改动合入前或同时做，否则 Goal 建 Work 必然参数错误。它就是那份 TODO 的 B2，以那份为准，这里只是登记依赖。

## 7. 待裁决

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
| Q-U1 | 谁在对话里开口（§4.3） | A / B / C / D | **B，落点是 default chat session**（用户 11-02 方向，待最终确认） |
| Q-U2 | default chat session 的定义（U2） | (a) 每个 (Agent, owner) 一个 MessageHub 原生 1:1 对话，Agent 初始化时建好；(b) owner 最近活跃的 1:1 对话 | **(a)**：固定落点才好预测；以后允许用户在设置里把默认改到某个 tunnel 对话 |
| Q-U3 | 怎么声明（U3） | `result.tell` 或 `goal tell` 命令 | **`result.tell`** |
| Q-U4 | tell 与通知的关系（U5） | 用户方向：chat 发消息 + 泛通知带 URL 打开这条消息，通知不抄送进对话。待定两点：(a) quiet 级别不发通知；(b) 消息已读后通知自动 ack / 撤回 | **(a)、(b) 都做** |
| Q-U5 | 投递方式（U4） | push：写 default session 的队列；pull：default session 查 Goal State | **pull** |
| Q-U6 | questions 要不要走 chat（U6） | (a) 维持 input 通知，抄送落点改到 default chat；(b) chat 优先：UI 问，通知带 URL，UI 把答案交回 | **先 (a)**，DV 后再定 |
| Q-U7 | `goal.toml` 里的 `notify send` | 保留，或并入 tell 的 `level` | **并入**。模型只说"告诉用户什么、有多急"，渠道由 runtime 决定（执行方式由配置决定）；`goal.toml` 删掉 `notify send` 的用法 |
| Q-U8 | 频率上限（U8） | 每周期 `max_tells` | **每周期 6 次**（缺省周期 1 天），0 表示不开口 |
| Q-U9 | 内置整理 Goal（U10） | 随 Jarvis 默认 Goal 的 P1 | 落点就是 default chat |
| Q-U10 | 主动消息由谁写（§4.5） | (a) default session 的 UI 推理写出；(b) Goal 写原文，runtime 记成"事件 + assistant 原文"，不推理 | **(a)** |
| Q-U11 | 事件打开的 Turn 的占位（U11） | 不发占位，或照旧 | **不发** |
| Q-U12 | UI 能不能不说（U12） | 能，原因写回 Goal State；或必须说 | **能** |
| Q-U13 | 压缩后的标记（U13） | 保留 goal 引用，或不处理 | **保留** |
| Q-S1 | timer 放在哪（§5.2 场景 1） | (a) workflow 的 Once / Cron，新增"发通知"目标，补幂等键与时区；(b) OpenDAN 本地 timer，由 Loader 宿主触发（像 GoalHost 的 `LocalScheduleBackend`）；(c) 每个 timer 一个 Goal | **(a)**：到点不需要 Agent 在线，workflow 已有 misfire 与 TaskMgr 持久化。(b) 只能在 Agent 进程在线时触发；(c) 太贵。前置是修 Jarvis 默认 Goal TODO §5.3 列出的 workflow 问题 |
| Q-S2 | 例行任务（B3）的形态 | (a) 每个一个 Goal；(b) 一个默认 Goal 管全部；(c) workflow 到点直接建 Work | **(a)**，理由见 §5.2 场景 2 |
| Q-S3 | Goal 的时间表 | `GoalScheduleSpec` 加 `cron` + `timezone`（与 `every_secs` 二选一）；Goal 的 result 加 `follow_up_at`（"下次什么时候再看"，由 runtime 安排一次运行机会，不修改定义） | **两个都加**；时区来源沿用 Jarvis 默认 Goal TODO §5.1 的顺序 |
| Q-S4 | 意图分析遇到定时请求 | 今天一律 `unsupported`。改为：固定提醒 → 建 timer（新 outcome `timer_created`）；到点要判断的 → 建 Goal 并带 cron；确定流程 → 交给 workflow（流程已存在时） | **按左列** |
| Q-S5 | 例行 Goal 的成本档 | (a) Goal 定义加可选 `model`，例行任务用便宜模型；(b) 打开 decision 预检；(c) 只靠调大间隔 | **先 (a)**，decision 预检维持关闭 |
| Q-S6 | 首批默认 Goal | A1（已有）+ A2（并入 A1 的 P1）；A3、A4 第二批；A5 待定 | **按左列**；A5 先确认要不要 |
| Q-S7 | chat 能否直接建 timer | 用户明确说"提醒我…"时 chat 直接建，或一律经意图分析 | **直接建**：参数完整的明确指令不需要探索；缺时区或时间有歧义才问 |
| Q-S8 | 日程管家怎么改 timer（场景 1） | (a) 循环规则是真相，日程管家只能跳过 / 改动当天的一次发生，没跑就照规则响；(b) 日程管家每天把当天的 timer 实例化，不实例化就不响 | **(a)**：LLM 是增强不是前提；(b) 在日程管家没跑的那天会静悄悄丢掉全部提醒。需要 workflow 支持单次发生的跳过 / 改动 |
| Q-S9 | 日程管家能否自动跳过 | (a) 有 `purpose` 或用户确认过的跳过条件就自动跳过，并用 quiet tell 告知；拿不准就照常或早上问一句；(b) 一律先问 | **(a)**：跳过一次提醒风险低，用户随时可以撤回；架构 §8.1 要求依据的是用户确认过的偏好，不是推断 |
| Q-S10 | 日程管家是否也检视例行任务（B3）当天的 fire | 是：例如请假日跳过"每天汇总邮件"；否：B3 的 Activation 自己判断 | **否（首版）**：B3 的 Activation 本来就是判定器，能看到同样的用户状态；让一个 Goal 改另一个 Goal 的时间表还要新增权限 |

## 8. 实施清单（草案；裁决后回写）

### A. lib_opendan / agent_tool

- A1（Q-G1）`ActivationBudget` 加 `max_wall_secs`；`goal::activation_exhausted` 判断墙钟；`TaskFollowup` 增加 goal 口径的文案（或由 libopendan 按 SessionKind 选择）；`reports.rs` 拒绝文案按 goal 区分。
- A2（Q-G2）Goal Session 的工具集剔除 `ask_user`，落点要看 baseline 在哪里按会话过滤，实施前核对。
- A3（Q-G5）`current_usage` 与 `work_admission` 读运行中 Work 的 `usage.jsonl`；`close_activation` 吸收时仍用 statistics（那时 run 已结束）。
- A3b（Q-G4c）`api.rs` 建子会话时，父是 Goal Session 且未给 `--behavior` → behavior 取 `work_simple`。
- A4（Q-G6）Goal 头作为输入模板变量；删掉独立的那条 User 消息；Work 结局去重。
- A5（Q-G7）Goal Activation 关闭时清理 `watched_tasks`（`commit_run_end` 中 goal 分支）。
- A6（Q-U1/U3/U5）`GoalResult.tell`（`reports.rs:297-312` 校验）、`GoalState.tells`（投递状态：pending / delivered / skipped + 原因）；default session 的 poll（与 `poll_children` 同款），看本 Agent 各 Goal 未投递的 tell；事件 key 形如 `goal:<id>:tell:<epoch>:<seq>`，保证幂等。
- A6b（Q-U2）default chat session：Agent 初始化时建好并绑定 MessageHub 原生 1:1 对话；登记表里能查到"这是 default"。落点（control_panel 建 Agent 时，还是 opendan Loader 启动时 ensure）实施前核对 UI session 的创建路径再定。
- A7（Q-U4）UI 消息投递成功后由 runtime 发泛通知：`url` 指向 `/messagehub?entityId=…&sessionId=…&messageId=…`，`source.conversation` 置空（不抄送）；quiet 不发；消息已读后通知自动 ack / 撤回（msg-center 侧，实施前核对已读状态的来源）。
- A7b（Q-U6）`notify.rs::conversation_of` 的回退改为 default chat（`created_by` 找不到对话时）。
- A7c（Q-U11）`runner/outbound.rs`：不是由用户消息打开的 Turn 不发占位。
- A7d（Q-U12/U13）事件 Turn 允许空回复并回写 `skipped`；UI 压缩保留 tell 标记。
- A8（Q-U8）`PeriodBudget.max_tells`。
- A9 schema 用 `xagent schema doc/opendan/protocol/schema` 重新生成，fixtures 重新生成（只保留真正变化的文件）。

### B. Jarvis 包

- B1 `goal.toml`：G1；Q-G4（`create-worksession` 示例写 `--behavior work_simple`，删"plan"的说法）；Q-G1 的长任务那句；Q-U3 的 tell；按裁决删去 `notify send` 与 `ask_user` 的说法。
- B2 `chat_route.toml`：`goal` 事件分支（U7，含可以不说）；`template.rs:60` 与 `:63` 的 UI 目标文本与实际对齐（群聊不收 tell）。
- B3 `analyze_intent.toml:109`："what the user will see next"里 Goal 的部分改为"Goal 有话要说时，会在和 Agent 的默认对话里告诉你"。
- B4 `agent.toml`：`[session.goal.budget]` 新字段。
- B5 `memory_maintenance.toml`：不动，除非 Q-U9 改变。

### C. 文档

- C1 [Goal Session TODO](./done/opendan-goal-session-todo.md) §11 之后加一个指向本单的指针。
- C2 《方法总结》补一节"default chat session：Agent 主动说话的落点"（§4.4 U2、U5，§4.5），§11–§12 的通知路径随之改写；`Session Input Protocol` 补 `goal` 事件；手册 `opendan_agent_dev.md` 的 Goal 段落。

### D. 场景相关（Q-S 裁决后再细化）

- D1（Q-S1/S8）timer：workflow 新增发通知的目标（`url` 指向 default chat）、按 owner + key 的幂等创建、时区、单次发生的跳过 / 改动；timer 记录带 `purpose` 与可选的跳过条件；`agent-session timer add|list|cancel|snooze`（或 websdk `buckyos` CLI 的同名命令，按"Agent 工具优先 TS + websdk"的原则落位）；对应的 cmd_skill 手册。
- D2（Q-S3）`GoalScheduleSpec.cron/timezone`，调度后端按墙上时间计算时间槽；`GoalResult.follow_up_at` 与对应的运行机会。
- D3（Q-S4/S7）`analyze_intent.toml` 的定时分支与 `timer_created` outcome（schema 加字段）；`chat_route.toml` 直接建 timer 的规则。
- D4（Q-S5）Goal 定义加可选 `model`，`goal create --model`。
- D5（Q-S6）A2 随 Jarvis 默认 Goal TODO 的 P1 落地，落地目标包括 timer / 例行 Goal / 目标 Goal。
- D6（Q-S8/S9）日程管家：作为第二个内置 Goal 播种（与 `builtin:memory_maintenance` 同一套播种机制，key 如 `builtin:timer_keeper`）；`behaviors/timer_keeper.toml`；宿主 gate 只在当天有 timer 发生时才开 Activation（没有 timer 的日子零推理）；检视结果用 quiet tell 告知。

### E. 测试与 DV

- E1 `tests/goal_session.rs`：Activation 墙钟上限；残留任务不开 Activation；运行中 Work 的 token 计入准入；Goal 不带 `--behavior` 建出 work_simple；tell 投递与幂等（崩溃点放在 `goal:after_activation_close`）；UI poll。
- E2 DV（本机 DV Zone，`./debug_jarvis.sh devtest --agent xiaobai.test.buckyos.io`）：在另一个对话里建一个 Goal → Activation 产出 tell → Jarvis 在 default chat 里用对话口吻说出来（没有"处理中"占位、没有通知卡）→ 通知中心收到一条泛通知，点开 URL 定位到这条消息 → 用户回复 → UI `goal post` → 下一次 Activation 的 reason 是 `message`；用户先说"今天别打扰"再触发 tell，UI 不说并回写 `skipped`；Goal 建出 work_simple，且运行中 period tokens 随之增长。DV 结束后归档测试 Goal。

## 9. 关联

- [llm_context 冻结 TODO](./done/llm-context-freeze-todo.md)：原则 8、F18、F20、§3.2.1
- [简单 Work Session 全异步化](./done/opendan-simple-worksession-async-todo.md)：§3.3、§7（第三刀 UI）
- [plan_simple 多 Turn 化](./opendan-plan-simple-async-todo.md)：§8.5（Goal 建的 plan tick 30 秒）、§8.6（半订阅只在输入里）
- [MessageHub worksession UI TODO](./done/messagehub-worksession-ui-todo.md)：Q3 `--title` 必填、Q9 Goal UI 不做、B2
- [Goal Session TODO](./done/opendan-goal-session-todo.md)、[Jarvis 默认 Goal TODO](./jarvis-default-goal-session-todo.md)（P1 候选确认）、[Agent 通知 TODO](./done/opendan-agent-notify-todo.md)
- [Agent 主动给用户发"消息"的方法总结](../doc/opendan/Agent%20主动给用户发_消息_的方法总结.md)：§5、§7.1、§8.2、§10–§12
