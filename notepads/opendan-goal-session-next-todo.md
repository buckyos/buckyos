# Goal Session 继续推进 TODO

日期：2026-11-10

状态：**A 组已于 2026-11-10 实施并在本机 Zone 补验（未提交），记录见 §2 的 A 组和 §7；其余各组未实施。** B–G 的裁决状态见 §1 和 §3。

本单是 Goal 剩余工作的总入口：排顺序，标明依赖和裁决状态，并补上此前没有任何 TODO 承接的项。已有详细方案的不在这里重写，只给指针，实施时按原文的方案做：

- [Goal Session 对齐全异步模型 TODO](opendan-goal-session-async-todo.md)（下称“异步单”）：Q-G*、Q-U*、Q-S* 的选项与建议，§8 的实施清单。
- [Jarvis 默认 Goal Session TODO](jarvis-default-goal-session-todo.md)（下称“默认 Goal 单”）：P1 / P2 的 T05、T06，验收 V06、V10–V16。
- [Goal Session TODO](done/opendan-goal-session-todo.md)（下称“第一段”）：§9 的第二段输入、§11.5 的遗留。

现状不在本单重复，见[架构设计](<../doc/opendan/OpenDAN Agent Session架构设计.md>) §15.7（2026-11-10 按提交 `667468998` 核对的实现、与设计的差异和未实现项）。

读者：owner（裁决与排序）和后续实施的 code agent。路径相对 `buckyos/`；`lib_opendan/` 指 `src/frame/lib_opendan/src/`，`jarvis/` 指 `src/apps/jarvis_agent/agent/`。行号是 2026-11-10 的源码，实施前核对。

## 0. 结论先行

1. Goal 的框架已经完成并提交：Goal 对象、Activation、派生 Work、预算、休眠、两个内置 Goal、Goals 页面。剩下的工作分七组（A–G）。
2. 不需要裁决、现在就能做的只有 A 组：把 11-02 已裁决的“Goal 建的 Work 一律 `work_simple`”落地，补第一段的一处遗留，补真实 Zone 上的验证。
3. 对用户最可见的缺口有两个，互不依赖：
   - Goal 有话要说时只能发通知，对话里的 Agent 不知道也不开口（C 组）。
   - 没有按日历时间的运行机会，也没有提醒（D 组）。今天对 Agent 说“每天九点提醒我”得到的是 `unsupported`。
4. 建议顺序 A → B → C → D → E → F → G。B 排在 C、D 前面，因为它修的是预算口径和 Activation 的边界；C、D 会让 Goal 跑得更多，先把账算对。
5. 需要 owner 先定的：异步单 §7 里还没有裁决的问题（都有建议，§3.1），以及本单新增的 8 个问题（§3.2）。

## 1. 分组与依赖

| 组 | 内容 | 依赖 | 裁决状态 |
| --- | --- | --- | --- |
| A 收尾（已完成） | Q-G4 落地、第一段遗留、真实 Zone 补验 | 无 | 不需要 |
| B 运行语义 | Activation 墙钟上限、禁用 `ask_user`、运行中 Work 的用量、宿主缺省 behavior、Goal 头并入输入、残留任务 | 无 | Q-G1、G2、G4c、G5、G6、G7 待裁决 |
| C 主动找用户 | default chat session、`result.tell`、带 URL 的泛通知 | 无硬依赖；与 B5 改同一段输入装配，排在它后面 | 方向已定（Q-U1 选 B），Q-U2–U13 待裁决 |
| D 时间 | cron 与时区、follow_up、workflow 计划后端、timer 对象、定时意图、对象订阅 | D3、D4 依赖 workflow 的修复 | Q-S1、S3、S4、S7、S8 待裁决；N6 |
| E 内置 Goal | 整理 Goal 的 P1 / P2、日程管家、self_check 退役、结晶 Goal 的开关与验证 | E1 的确认通路用 C（没有 C 时先用通知）；E2、E3 依赖 D | Q-S6、S9、S10；N1、N8 |
| F 上下文与方法 | Goal Skill 运行时读取、内置 goal skill、Goal 压缩 | 无 | N2、N3 |
| G 治理 | 就绪性检查、页面（修订差异、数量提示、成本、编辑） | 无 | N4、N5 |
| 不排期 | XLLM 结晶、离线 Self Correct、子 Work 预算预留与 Agent 总预算、跨节点与多 Session | — | N7 |

## 2. 任务

### A. 收尾（不需要裁决）

- [x] **A1 Goal 建的 Work 用 `work_simple`。**（已实施，见 §7） 2026-11-02 已裁决（异步单 Q-G4）。今天 `goal.toml` 不带 `--behavior`，取到的是 `[session.work] default_behavior = "plan_simple"`，建出来的是 plan 加 Todo 子会话，子会话不计入 Goal 的 Work 数。
  - 落点：`jarvis/behaviors/goal.toml:18` 的 `create-worksession` 示例加 `--behavior work_simple`。宿主侧的缺省（提示词漏写时不退回 plan）是 B4，等裁决。
  - 验收：`cargo test -p opendan`（behavior 能解析）、`cargo test -p libopendan --test goal_session`；DV 里 Goal 建出的 Work 冻结的 behavior 是 `work_simple`，名下没有 Todo 子会话。
  - 手册：核对 `opendan_agent_dev` 有没有描述 Goal 建 Work 的形状，有就同步。
- [x] **A2 第一段的遗留。**（已实施，见 §7）
  - Goal Session 因 Workspace 绑定失效而 failed 时不写 Activation 记录（`lib_opendan/runner/drive.rs:252` 的 `fail_workspace` 没有 Goal 分支）。先写用例复现：Goal 的 `scope.workspace` 失效后，Goal 页面上看不看得到原因，restart 能不能恢复；再决定是补一条 `failed` 的 Activation 记录，还是只更新 Goal State 的 `last_error`。
  - 不做的两条：Activation 预算不含 `max_tool_iterations`（每个 run 仍受 behavior 的上限，记为已知）；Goal Session 内的感知不带 `goal_ref`（已解决，`runner/consolidation.rs:1188`）。
- [x] **A3 真实 Zone 补验。**（四条都通过，发现的问题见 §7.3） 只验证不改代码，发现的问题另记。证据放 `notepads/evidence/goal-session-next/<date>/`，结束时归档测试 Goal。
  1. 容器形态（`./build_aios --local-test` 后 `uv run src/start.py --skip-update`）：整理 Goal 播种一次、无材料时不调用模型；等到一次由 30 分钟定时触发（不是手动 activate）的真实整理。
  2. WebUI 经 zone 网关登录：休眠、唤醒、归档、reopen、restart 都能做，内置 Goal 不能归档。
  3. 休眠语义：Activation 进行中置休眠，在下一个推理边界收尾；派生的 Work 随之暂停，唤醒后恢复；对 Goal Session 发 Session 级 stop 等于休眠。
  4. 把 `[session.goal.crystallize]` 打开跑一次结晶 Goal，看它对真实模型的表现（读了哪些 Session、有没有越权改写）。

### B. Goal 运行语义对齐全异步模型

方案在异步单 §3、§7，落点在它的 §8 A1–A5，验收在它的 E1。都有建议，等裁决。

- [ ] **B1 Activation 墙钟上限**（Q-G1，建议维持“跟到底”并加上限，缺省 1800 秒）。`ActivationBudget.max_wall_secs`（`lib_opendan/protocol/goal.rs:221`）、`runner/goal.rs::activation_exhausted`；跟进提示和 report 被拒的文案对 Goal 另写；`goal.toml` 补一句长命令的说明。
- [ ] **B2 Goal Session 里去掉 `ask_user`**（Q-G2，建议宿主层剔除并提示用 `result.questions`）。今天只靠 `goal.toml` 的一句话挡着，真被调用时 Activation 会挂到用户答复或过期。入口 `lib_opendan/baseline.rs:83`，按会话过滤的位置实施前核对。
- [ ] **B3 运行中 Work 的用量计入周期**（Q-G5，建议准入和 Goal 头读各 Work 的 `usage.jsonl`）。`lib_opendan/goal.rs` 的 `current_usage`、`work_admission`。
- [ ] **B4 宿主缺省 behavior**（Q-G4c，建议父是 Goal Session 且没给 `--behavior` 时取 `work_simple`，显式给别的照样允许）。`lib_opendan/api.rs::create_sub_session`，常量 `todos.rs::WORK_BEHAVIOR`。
- [ ] **B5 Goal 头并入输入模板，Work 结局只出现一次**（Q-G6）。今天 Goal 头是一条独立的 User 消息，同一个 Work 结局在原因、`<works>`、事件渲染三处出现。fixture `16_goal_session` 要重新生成。
- [ ] **B6 Activation 关闭后不再跟残留任务**（Q-G7）。今天不可取消的任务结束时会绕过准入链打开新的 Activation。

### C. Agent 主动找用户

方案在异步单 §4，落点在它的 §8 A6–A9 与 B 节，验收在它的 E2。owner 11-02 定了方向：设一个 default chat session，Goal 只声明要说什么，由那里的 UI Session 在事件打开的 Turn 里说出来，再发一条带 URL 的泛通知。细节 Q-U2–U13 待裁决。

- [ ] **C1 default chat session**（Q-U2）：每个 (Agent, owner) 一个 MessageHub 原生 1:1 对话，Agent 初始化时建好，登记表里能查到。
- [ ] **C2 声明与投递**（Q-U3、U5、U10）：`GoalResult.tell`、`GoalState.tells`（投递状态）、default session 轮询各 Goal 未投递的 tell，事件 key 保证幂等。
- [ ] **C3 泛通知**（Q-U4、U6）：消息投递成功后发通知，带打开这条消息的 URL，不抄送进对话；quiet 级别不发，消息已读后撤回。没有创建对话的 Goal（内置 Goal、页面或 kRPC 建的）回退到 default chat。
- [ ] **C4 事件打开的 Turn**（Q-U11–U13）：不发“处理中”占位；UI 可以不说并把原因回写；压缩后保留 Goal 引用。
- [ ] **C5 频率上限**（Q-U8）：`PeriodBudget.max_tells`。
- [ ] **C6 Jarvis 包**（Q-U7）：`goal.toml` 用 tell 取代 `notify send`；`chat_route.toml` 加 Goal 事件分支；`analyze_intent.toml:120` 把“the goal's notifications”改成与实际一致的说法。
- 验收时按手机宽度再走一遍：目标用户没有 PC，通知点开要直接落在这段对话里。

### D. 时间：日历时间表、follow_up、生产调度、timer

- [ ] **D1 cron 与时区**（Q-S3）。`GoalScheduleSpec`（`lib_opendan/protocol/goal.rs:284`）加 `cron` 与 `timezone`，与 `every_secs` 二选一；`LocalScheduleBackend`（`lib_opendan/host/goals.rs`）按墙上时间计槽；`goal create` 加对应参数。时区来源的顺序按默认 Goal 单 §5.1。不依赖 workflow，可以先做。需要时区库时先确认。
- [ ] **D2 follow_up**（Q-S3）。Goal 的结果加 `follow_up_at`，由宿主安排一次运行机会，不改定义。整理 Goal 的 gate 可以把 `pending.next_deadline` 接到同一机制。
- [ ] **D3 workflow 计划后端。** 生产定时换到 workflow 的计划任务，本地后端只留给 dev 和测试。
  - 先修 workflow 的四个问题（第一段 §9.1，是 10-11 的记录，workflow 在 11 月改过，实施前复核）：`opendan.command` 的 fire task 没有执行方；成功 fire 后按旧的 fire 时间算下一次，停机后会逐个补发；`buckyos-api/src/workflow_service.rs` 的类型与服务端不一致；`load_schedules` 只读第一页。
  - 再做 `WorkflowScheduleBackend`：`every_secs → RunEvery`、cron → `Cron`、follow_up → `Once`，目标 `opendan.command{goal.activate}`。
  - 改 workflow 按 `harness/Service Dev Loop.md`；完整的 IANA 时区要引入时区库，先确认。
- [ ] **D4 timer 对象**（Q-S1、S7、S8）。到点不需要推理的提醒，不是 Goal。方案在异步单 §5.2 场景 1 与 §8 D1：workflow 的 `Once` / `Cron` 加发通知的目标、按 owner 加 key 的幂等创建、单次发生的跳过与改动，以及建、列、取消的命令和手册。依赖 D3 里 workflow 的修复。
- [ ] **D5 意图分析的定时分支**（Q-S4）。`analyze_intent.toml:36` 今天一律 `unsupported`。改为：固定提醒建 timer；到点要判断的建 Goal 并带 cron。依赖 D1、D4。
- [ ] **D6 对象订阅驱动 Goal。** 设计 §7.8、§12.8：Goal 表达关注的对象与事件，由宿主绑定订阅并给运行机会（原因 `event` 已有）。声明放在哪待裁决（N6）。

### E. 内置 Goal

- [ ] **E1 整理 Goal 的 P1：候选发现与确认。** 默认 Goal 单 T05 的前两条和 §2.1 的候选展示，验收 V06、V10–V13 中不依赖 workflow 的部分。
  - 挂点：`lib_opendan/runner/consolidation.rs:854`（提交后发布候选）；问题与候选的关联 `runner/goal.rs:921`（`candidate_id` 今天恒为空）。类型已冻结，页面已有候选列表的位置。
  - P1 内候选最高到 `approved`，落地动作一律 `blocked` 并说明缺 E2。
  - 确认通路：有 C 之后在 default chat 里问（Q-U9）；C 之前先用现有的 `questions` 通知。
- [ ] **E2 整理 Goal 的 P2：落地与 decision 预检。** T05 的后两条（依赖 D3、D4）和 T06（`lib_opendan/template.rs:76` 的 `decision = "auto"`）。验收 V14–V16。
- [ ] **E3 日程管家**（Q-S6、S8、S9、S10）。第二个内置 Goal，每天检视当天的 timer。方案在异步单 §5.2 与 §8 D6，依赖 D4。
- [ ] **E4 self_check 退役**（N1）。它今天不运行（`[loader.self_check]` 打开只会报“本版本不可用”），但类型、模板、behavior 都在，也没有独立的 TODO。
  - 删除：`SessionKind::SelfCheck`、内建模板 `self_check` 与 `_self_check_timer`（`lib_opendan/template.rs:213`）、sid 前缀 `sc-`、`[loader.self_check]`（`src/frame/opendan/src/config.rs:71`、`loader.rs:420`、`jarvis/agent.toml:41`）、`jarvis/behaviors/self_check.toml`、WebUI 的 `SessionKind`（`src/frame/opendan/web/src/model/types.ts:1`）。
  - Notebook 一并退役（Memory 需求 TD-09）：`agent_tool/src/agent_notebook.rs`、`lib_opendan/state/` 里的 `notebook_append`、`xagent_cli.rs` 与 `agent_tool_cli_dev` 的入口、`opendan/src/service.rs`。唯一的 behavior 调用方是 `self_check.toml`。
  - 文档：`doc/opendan/xAgent.md` 的模板表、架构设计 §15.3 与 §15.5、Session Control Protocol 里提到 self_check 的几处。
- [ ] **E5 结晶 Goal 的开关与验证**（N8）。今天只能改 `agent.toml [session.goal.crystallize]`，页面上没有开关。验证见 A3 第 4 条。

### F. Goal 的上下文与方法

- [ ] **F1 Goal Skill 运行时读取**（N2）。`goal_skills/` 能安装和列出，运行时没有任何路径读它；Goal 头的 `<current_skills>`（`lib_opendan/runner/goal.rs:424`）只列名字和一个自由文本的 `ref`。这是 Skills 单 §3.5 留给 Goal 的那一条：`ref` 指向 `goal_skills/<name>/SKILL.md` 时，让 Goal 能可靠地读到正文。同时让 `goal create` 与 `goal skills add` 能按名字从 `agent-session skill list --kind goal` 里选。
- [ ] **F2 内置 goal skill。** Jarvis 包今天没有 goal skill（`jarvis/goal_skills/` 目录不存在）。首批写哪几份跟着典型场景定（异步单 §5）：候选是软件项目维护与 Issue 跟踪、例行任务的判定、提醒策略。写法按 `doc/sdk/skill-dev-quickstart.md`；回归 `cargo test -p libopendan --lib skills`。
- [ ] **F3 Goal 的压缩策略**（N3）。今天恢复所需的内容都在每次重建的 Goal 头里，历史走通用压缩；连续的 `no_action` Activation 让历史逐轮增长，直到通用压缩触发（第一段 DV 量到每轮约 1k token，那之后历史重建改过，数字要重新量）。先量一个跑了一周的 Goal 的历史构成，再定方案。

### G. 治理与管理面

- [ ] **G1 创建 Work 的就绪性检查**（N4）。设计 §3.1、§14.5 要求 UI 和 Goal 建 Work 走同一套检查；今天两个入口都只有提示词约束。
- [ ] **G2 Goals 页面**（N5）。`src/frame/opendan/web/src/pages/{Goals,GoalDetail}.tsx`，e2e 在 `tests/e2e/goals.spec.ts`。
  - 修订前后的差异：两个修订的字段对比，数据已有（`goals.revision`）。
  - Goal 数量提示：活动 Goal 超过阈值时在 Goals 区域提示，阈值待定（设计 §13.4 的候选是 16）。
  - 成本：详情页按周期显示用量的变化，数据在 Activation 记录里。
  - 页面内编辑：owner 侧的 `goals.revise` 已经可调，页面今天不提供表单。

### 不排期

- XLLM 结晶 Goal（设计 §9.4）：面向企业客服，不作为个人 Agent 的默认 Goal。
- 离线 Self Correct Goal（设计 §9.5）：前置是工具意图记录，今天没有。旧的现场打造脚手架的去留见 N7。
- 子 Work 的预算预留、Agent 总预算、费用维度（设计 §12.2、§12.3）：等 B3 之后有了真实的用量数据再定。
- 跨节点运行与多 Session 协作（设计 §12.4）。

## 3. 待裁决

### 3.1 已有的问题（选项与理由在异步单 §7）

| 组 | 问题 | 建议 |
| --- | --- | --- |
| B | Q-G1 Activation 内的长任务 | 维持跟到底，加墙钟上限，缺省 1800 秒 |
| B | Q-G2 Goal 里的 `ask_user` | 宿主层剔除 |
| B | Q-G4c 宿主要不要强制 `work_simple` | 改缺省，不拒绝显式指定 |
| B | Q-G5 预算口径 | 运行中也计入 |
| B | Q-G6 Activation 的输入形状 | Goal 头并入输入模板 |
| B | Q-G7 残留任务 | Activation 关闭时放手 |
| C | Q-U2–U13 default chat、tell、通知、频率等 13 项 | 见异步单各行 |
| D | Q-S1 timer 放在哪 | workflow |
| D | Q-S3 Goal 的时间表 | cron 加时区、follow_up 都加 |
| D | Q-S4、S7 定时意图与 chat 直接建 timer | 按异步单 |
| E | Q-S6、S8、S9、S10 首批默认 Goal 与日程管家 | 按异步单 |

Q-G3（问题答案的回流）建议先不动，Q-S2（每个例行任务一个 Goal）、Q-S5（例行 Goal 用便宜模型）随 D、E 再定。

### 3.2 本单新增

| N | 问题 | 选项 | 建议 |
| --- | --- | --- | --- |
| N1 | self_check 什么时候删 | (a) 现在就删，Notebook 一并退役；(b) 等 D4、E3 承接了提醒再删 | **(a)**。它本来就不运行，留着只是让各处多一个要照顾的会话类型；提醒的缺口不因为它在而变小 |
| N2 | Goal Skill 的正文怎么进上下文 | (a) 正文进 Goal 头；(b) Goal 头只给能直接读的路径，由 Goal 按需读；(c) 像 work skill 那样在 Session 创建时冻结进提示词 | **(b)**。Goal 头每次 Activation 都重发，正文进去成本随 Skills 数量线性增长；(c) 与“Goal 运行中增删 Skills”冲突 |
| N3 | Goal 的压缩方案 | (a) 关闭的 Activation 在历史里折叠成它的结论，只保留最近几次的完整推理；(b) 维持通用压缩，只给压缩提示 | 量完数据再定；倾向 **(a)**，因为恢复所需内容已在 Goal 头里，历史的价值只是最近几次怎么做的 |
| N4 | 就绪性检查做到哪 | (a) 只靠提示词（现状）；(b) 宿主做机械检查：同一 Goal 下不重复建任务书相同且未结束的 Work、任务书不能等于 Goal 的 objective；(c) 建 Work 前过一次模型判定 | **(b)**。(c) 每次建 Work 多一次推理，不值 |
| N5 | 页面内编辑到什么程度 | (a) 不提供，改目标走对话；(b) 只开放预算和时间表；(c) 全部字段 | **(b)**。目标文本的修改需要澄清，适合对话；预算和时间表是调参，适合表单 |
| N6 | 对象订阅在哪声明 | (a) Goal 定义里加 `watch`，宿主绑定；(b) Goal Session 自己 `subscribe`，随 epoch 丢失 | **(a)**。定义是真相，restart 之后订阅还在 |
| N7 | 旧的现场打造脚手架（`agent_tool` 的 `llm_tool_carft`）| (a) 删除；(b) 留着等 Self Correct | **(a)**，如果确认运行时确实没有接入它。设计 §9.5.1 已经放弃现场打造 |
| N8 | 结晶 Goal 的开关放在哪 | (a) Skills 页；(b) Goals 页；(c) 维持改 `agent.toml` | **(a)**。它产出的草稿也在 Skills 页 |

## 4. 验证

在 `src/` 下，按改动范围选：

```bash
cargo test -p libopendan --test goal_session -- --test-threads=1
cargo test -p libopendan --test memory_maintenance -- --test-threads=1
cargo test -p libopendan --test session_control -- --test-threads=1
cargo test -p libopendan --test notify -- --test-threads=1
cargo test -p libopendan --test fixtures -- --test-threads=1
cargo test -p opendan -- --test-threads=1
# 改 workflow 时
cargo test -p workflow -- --test-threads=1
```

WebUI 在 `src/frame/opendan/web` 下跑 `pnpm check`、`pnpm lint`、`pnpm test:e2e`。

DV 用本机 Zone：只验宿主侧逻辑用 `./debug_jarvis.sh devtest --agent <agent_id>`；涉及容器内的 OpenDAN 或 WebUI 用 `./build_aios --local-test` 加 `uv run src/start.py --skip-update`。改了 Jarvis 的 behavior 要重新打包模板才会进到已部署的 Agent。mock 通过不算真实模型或真实 Zone 通过，没跑的写明没跑。

## 5. 实施时守住的边界

- 第一段 §10 的禁止项仍然有效：不用 TimerBridge 驱动 Goal，不在 Goal 内部做自我唤醒，不把 Goal 定义复制到 Goal 头之外当真相，不让 Activation 失败结束 Goal Session，不把 `no_action` 记成达成，不把“没观察到”记成“没有变化”。
- Goal 不自己写进对话（异步单 §4.3 的选项 C 已否决）。
- skill 是纯文本，只有 owner 能安装：内置 goal skill 随 Jarvis 包分发，Goal 自己不安装任何东西。
- 群消息、联系人策略不在本单范围；tell 只进 owner 的 default chat，不进群。
- 改 Goal 的协议字段（定义、结果、预算、时间表）时同步：`lib_opendan/protocol/goal.rs`、schema（`xagent schema doc/opendan/protocol/schema`）、fixture、Agent State Protocol §9、WebUI 的类型。默认允许 breaking change，不做旧字段兼容。
- 引入新依赖（时区库等）先确认。

## 6. 完成一项之后

- 在本单勾选，并在对应的原 TODO（异步单、默认 Goal 单）里记实施记录。
- 回写架构设计 §15.7：从“未实现”移到“已实现”，或改“与设计不同的地方”；涉及控制语义的回写 Session Control Protocol。
- 按 `AGENTS.md` 的 cmd_skills 联动表检查手册：Goal 建 Work 的方式、通知与 tell、定时相关的改动会影响 `opendan_agent_dev`、`ui_core`。

## 7. A 组实施记录（2026-11-10，未提交）

### 7.1 A1

- `jarvis/behaviors/goal.toml` 的 `create-worksession` 示例加了 `--behavior work_simple`，其余文字没有动。宿主侧不兜底（B4 待裁决）。
- 手册：`opendan_agent_dev`、`ui_core` 都没有描述 Goal 建 Work 的形状，不需要改。
- 验证：`cargo test -p opendan`（behavior 能解析）、`cargo test -p libopendan --test goal_session` 通过；本机 Zone 上真实模型建出的 Work 冻结的 behavior 是 `work_simple`，名下没有子会话。

### 7.2 A2

复现（`tests/goal_session.rs`，改之前的行为）：Goal 的 `scope.workspace` 目录被移走后，下一次运行机会让 Goal Session 以 `failed` 结束；Goal 仍是 `active`，Goal State 的 `last_error` 为空，没有 Activation 记录，详情页上看不到任何原因，只有 Loader 的日志和内存里的错误列表每个 tick 记一条“goal session … finished while the goal is active; restart the goal”。目录恢复后 restart 能恢复（新 epoch、新 Session）；目录没恢复就 restart，新 Session 建不出来，原因同样只在 Loader 的错误列表里。

决定：两种都做，按失效发生的时机分。

- Activation 进行中失效：这次 Activation 记为 `failed`，`end_reason` 是失效原因，用量照常计入周期。
- 没有 Activation 在进行（两次运行机会之间）：不造一条空的 Activation 记录，只把原因写进 Goal State 的 `last_error`。
- 两种情况下 `last_error` 都带 `kind = workspace_binding_invalid`、原来的绑定和一句 `recovery`（修好 Workspace 后 restart）。详情页本来就显示 `last_error`，不用改页面。restart 后第一次 Activation 的 Goal 头带着这条错误，Activation 无错完成后清掉。
- Goal 的状态不变，Session 仍然以 `failed` 结束（Workspace 绑定失效对任何 Session 都是终局），恢复靠 restart。

落点：`lib_opendan/runner/goal.rs::fail_session`；`runner/drive.rs::fail_workspace` 改为拿 `Shared`，三个调用点（drive 开始时的检查、运行中发现、drive 返回的错误）都走它，之后立即 `flush` 把 Goal State 和 Activation 记录发布出去。drive 开始时的检查原来在 `Shared` 建好之前就返回，现在推迟到建好之后（不绑定 runtime、不打开输入队列）。

用例：`a_lost_workspace_is_told_by_the_goal_state_and_restart_recovers`、`a_workspace_lost_mid_activation_closes_it_as_failed`。回归：`goal_session`（28）、`workspace_binding`（9）、`fixtures`（19）、`memory_maintenance`（12）、`session_control`（8）、`cargo test -p opendan` 全部通过。

没有做的：

- 宿主每个 tick 仍然为这个 Goal 报一次“session finished while the goal is active”（日志和 Loader 的错误列表），没有去重。
- Workspace 没恢复就 restart 时，新 Session 建不出来的原因只在 Loader 的错误列表里，不在 Goal 页面上（带状态 restart 时旧的 `last_error` 还在，说的是同一件事）。
- 失效前已经投进队列的运行机会随 Session 结束而丢掉，不会转给 restart 之后的 Session。
- 真实 Zone 上没有验这一条（需要弄坏一个真实的 Workspace），只有集成测试。

### 7.3 A3

证据和逐条结论在 `notepads/evidence/goal-session-next/2026-11-10/README.md`。四条都通过：整理 Goal 播种一次、无材料不调用模型、3 次由定时触发的真实整理；WebUI 经网关登录后的休眠、唤醒、restart、归档、reopen，内置 Goal 不能归档；Activation 进行中置休眠在下一个推理边界收尾、Work 随之暂停并在唤醒后恢复、Session 级 stop 等于休眠；结晶 Goal 对真实模型跑了一次（读了 6 个 Work，`no_action`，没有改写任何手册）。

发现的问题，都没有处理：

1. 被停掉的 Activation 留下的任务结束时，唤醒后打开新的 Activation。就是 B6，真实 Zone 上确认了。
2. Work 的 `needs_input`（`work_simple` 的 Work 在等自己的后台任务时关闭 Turn）也给 Goal 一次运行机会，Goal 为此读一遍没有新内容的报告。没有 TODO 承接，建议并入 B6 或 B5 一起看：Goal 只该为 Work 的结束和真正要人回答的问题醒来。
3. Work 自己的任务结束事件（`task:local:shell:…:terminal`）出现在 Goal 的 Activation 原因里。来源没有查。
4. 成本：Goal 里每跟一条长命令就带着整段上下文推理一次，测试 Goal 三次 Activation 用了 37.8 万 token；结晶 Goal 一次 `no_action` 用了 18.2 万 token。前者和 B1 有关，后者和 E5 有关（打开开关之前要让 owner 知道一次的量级）。
5. 结晶 Goal 的开关关掉以后，Goal 仍显示为 `active`（gate 是 `disabled`），Session 留着。和 E5（N8）一起定页面上怎么显示。

没有验的：手机宽度与真机；结晶 Goal 的“修订手册”和“写草稿”路径（这次材料不够，没有触发）。

留在本机 Zone 的：两个已归档的测试 Goal、打开开关时播种的内置结晶 Goal（开关已还原）；Zone 运行的是包含 A1、A2 改动的构建。
