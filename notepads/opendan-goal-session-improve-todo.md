# Goal Session 改进 TODO

日期：2026-11-24 · 源码基线 `1704812f8` · 状态：**待 owner Review，未实施**

需求来源有三处：owner 2026-11-24 重新思考 Goal Session 的脑图（§2.1 原文）、当天按代码做的差距核对、几个实际场景里暴露的问题（§2.3）。

和其他文档的关系：

- [Goal Session 继续推进 TODO](opendan-goal-session-next-todo.md)（下称“旧单”）：本文接替它没做完的 F3、E1–E3、D4 里和提醒有关的部分，以及 §7.3、§10.7 的遗留。D3（workflow 计划后端）仍留在旧单。
- [Agent 浏览器二期 TODO](agent-browser-phase2-todo.md)（下称“二期单”）：它的 GJ（Goal 作业）、WG（Work 开发、Goal 运维）两组是通用的 Goal 能力，本文不重写，只列依赖和要改的地方（§4.5、Q4）。
- [Plan / work_simple TODO](jarvis-plan-work-simple-todo.md)：已裁决未实施，和本文改同一批 behavior 文本，先后见 Q16。
- 现状见[架构设计](<../doc/opendan/OpenDAN Agent Session架构设计.md>) §15.7。

## 0. 怎么用这份文档

**给 owner。** 要你定的都在 §1。Q1–Q5 没定之前不开始对应的组；其余没有答复的按建议做。请直接在“答复”后面写。

**给 CodeAgent。**

- §2 是约束，不重新讨论。和代码现实冲突时停下来写进 §11，问 owner。
- §3 是查过的事实，标“没验”的要先验证。行号以基线为准，动手前核对。路径里 `lib_opendan/` 指 `src/frame/lib_opendan/src/`，`jarvis/` 指 `src/apps/jarvis_agent/agent/`。
- §4 是设计，名字和字段可以调整，调整后回写本文和协议文档。
- 做完一条，在 §11 记一行：改了什么、跑了什么、结果、没验什么。
- 遵守 [AGENTS.md](../AGENTS.md)：不做兼容实现；改动范围小；手册和代码同一变更内更新。
- **机制越少越好。** 每一处先看能不能用已有的命令、事件、提示词做到。本文新增的宿主机制只有五处（§4.9），多出来的要先问。
- 不要提交，除非 owner 要求。

## 1. 待 owner 决策

### 1.1 先定

**Q1 内置的 self improve 怎么合并？**

- 现状：两个内置 Goal，`memory_maintenance`（缺省开）和 `skill_crystallize`（缺省关），各有一份专用 behavior。没有“做工具”这项职责。
- **建议：合成一个 `self_improve`，分两步。** 它用标准的 `goal` behavior，每项职责是一份 goal skill，一份检查脚本按职责给出唤醒原因。
  - 第一步：把 `skill_crystallize` 改造成 `self_improve`，承担结晶手册和做工具。
  - 第二步：把 Memory 整理并进来，同时接上日程（Q12）。
- 分两步的理由：Memory 整理的质量依赖那份很长的专用提示词，搬成 goal skill 之后要用真实模型回放确认不退化；它还有宿主驱动的回执、清理和租约绑在 Goal 上（§3.1）。结晶和做工具没有这些负担。
- 另一种做法：一步全并；或只并结晶和做工具，Memory 整理一直单独。
- 影响：SI 全组。
- 答复：

**Q2 Agent 自己做的工具，要不要你确认才给 Work 用？**

- 现状：2026-11-09 定了 skill 是纯文本，安装、删除、禁用只有 owner 能做；结晶 Goal 只能改写来源为 `local` / `imported` 的手册正文，其余写草稿。Work 靠手册发现命令，所以“工具做出来就能用”等于允许 self improve 自己新建一份 `local` 手册。
- **建议：允许。** self improve 可以往 `<agent_root>/tools/` 放命令，并新建来源为 `local` 的手册，做完通知你一次，你可以在 Skills 页禁用。改随包安装的手册、动 `scope` 往 `ui` 方向，仍走草稿。按“先能力再安全”记为已知风险（§9）。
- 另一种做法：新手册一律走现有的草稿流程，你在 Skills 页点安装后才生效。不用改宿主，但每个新工具都等你一次。
- 影响：SI3、SI4。
- 答复：

**Q3 每次唤醒继承全部历史，还是只带上次的结论？**

- 现状：两样同时在。同一个 Session 的历史全部继承，走通用压缩；宿主每次另外重建 Goal 头，里面有上次的结论、距离、待办、问题和 Work。
- **建议：先量（S0-1），再定。** 历史占一次 `no_action` 成本两成以上，就把缺省改成“每次唤醒开新上下文，只带 Goal 头”，Goal 可以自己切回继承（和改检查脚本一样，不增加修订号），你也能在页面上改。不到两成就维持现状，不加这个开关。
- 两种情况下都不做 Goal 专用的压缩，继承历史时走标准压缩。旧单的 F3 就此关闭。这和你对 Plan 历史的裁决一致（Plan / work_simple TODO 的 D5）。
- 影响：CX2。
- 答复：

**Q4 新建 Goal 一律先过一个“建 Goal 的 Work”？**

- 现状：意图分析直接 `goal create --start`，对话立刻收到 `goal_created`。第一次 Activation 自己写检查脚本；它失败了 Goal 仍是 active，用户不知道。
- **建议：统一。** 意图分析遇到要新建 Goal 的请求，建一个 `work_simple` 的 Work（手册 `goal_setup`）。Work 把检查脚本写好并试跑，准备好交付要用的东西，最后建 Goal。成败就是 Work 的结局，结果自己回到对话。Goal 从第一天起只做运维。
- 这改了二期单 WG3 里“纯观察和提醒仍直接建 Goal”那一句，并依赖它的 WG1、WG2（Work 能建 Goal、命令行能在创建时装检查脚本）。
- 另一种做法：只有“先建设后维护”的请求走 Work（二期单现在的写法），简单的 Goal 仍直接建。少一个 Work，但首次失败仍然没人知道。
- 影响：FX 全组。
- 答复：

**Q5 固定时间的提醒，改成优先写进日历？**

- 现状：2026-11-11 定了固定提醒由 Goal 的检查脚本发通知。2026-11-20 接入了 `owner.cal`，日程可以带 `reminders`，手册写着重复日程也会提醒。现在同一句“每天九点提醒我”有两条路，意图分析走的是建 Goal。
- **建议：到点就说、不需要判断的提醒写日历，不建 Goal。** 日历表达不了的规则（“工作日 15 点到 18 点每 50 分钟”这类可以拆成几条重复日程，拆不了的才算），以及要看了情况才决定说不说的提醒，仍用检查脚本。这样一个提醒不再占一个 Goal。
- 前提没验：日历提醒由谁在什么时候发、Agent 不在线时发不发、手机上收不收得到（S0-3）。验不过就维持现状。
- 影响：CA1。
- 答复：

### 1.2 有建议值

**Q6 模式 2（每次唤醒派子会话）怎么表达？** 建议不做成宿主的模式，写进提示词和一份 goal skill：轻的事自己做；要独立上下文的派一个 `work_simple`；要先探索再动手的派一个 `plan_simple`（它本来就是“探索 Todo + 执行 Todo”）。接受一次推进跨几次唤醒，不让 Activation 等子会话。需要把 2026-11-02“Goal 建的 Work 一律 `work_simple`”在提示词里放宽成“缺省 `work_simple`”，宿主本来就允许显式指定。影响 CX3。
答复：

**Q7 Goal 连续失败时，宿主要不要告诉你？** 建议要，只发通知、不自动休眠：连续 3 次 Activation 以失败、预算耗尽或没有有效结果结束时，以这个 Goal 的名义发一条通知，恢复之后再失败才再发。理由是 Activation 本身失败时，提示词里的求助规则没有机会执行。另一种做法是不加，只靠提示词和周期预算封顶。影响 FH2。
答复：

**Q8 检查脚本连续出错，要不要提前唤醒一次？** 2026-11-10 定的是不为脚本出错加机制，兜底只有硬保障（72 小时）；11-11 的 DV 里发现这对靠检查发通知的 Goal 偏长，当时留给你定。建议连续 3 次出错按硬保障到点处理，唤醒一次，改动是 `guarantee_due` 里多一个条件。另一种做法是维持裁决不动。影响 FH5。
答复：

**Q9 求助主人以外的人、在 HomeStation 上求助，这一期做不做？** 建议都推后（§10），这一期只把“求助主人”做扎实：Goal 提问时可以带选项，可以写明“这件事可以问谁”。推后的理由：Agent 主动联系第三人需要先定授权规则；HomeStation 上现在没有会去读求助帖的 Agent。影响 FH3、FH4。
答复：

**Q10 Goal 的对话视图放在哪，里面说的话怎么处理？** 建议放 Desktop 的 Agent 面板（用户在手机上，Agent 主页是另一个站点，手机宽度没验过）：Goal 列表，点开是对话形态，每次唤醒一组（原因、折叠的过程、结论）。输入框里发的话就是 `goal post`。用户的话不过硬间隔、仍过预算，否则发出去最长 15 分钟没反应。影响 UX1、UX2、UX4。
答复：

**Q11 Goal 太多怎么办？** 建议三个选项都不做成机制：不设硬上限，不加合并命令，不把所有 Goal 合成一个。靠四件事控制数量：提醒改走日历（Q5）；意图分析优先并入已有 Goal（已有）；活动 Goal 达到 16 个时，新建之前必须说明为什么不能并入（提示词）；Agent 后台总量封顶（已有）。Goal 之间的矛盾在新建和改写的时候查（提示词），不在每次唤醒时查。“全部合成一个”现在做不了：一个 Goal 只有一份检查脚本、一份预算、绑定一个 Workspace。影响 UX5。
答复：

**Q12 “整理日程”指什么，什么时候做？** 我的理解：Memory 整理时发现的日程和待办，问过你之后写进 `owner.cal` / `owner.todo`；发现的长期职责，建议并入已有的 Goal。这就是旧单推后的 E1、E2，它等的日历和待办命令已经有了。建议放在 Q1 的第二步，方案沿用[默认 Goal 单](jarvis-default-goal-session-todo.md)的 T05，候选加一类“给已有的 Goal 增加一项职责”。影响 SI6、CA3。
答复：

**Q13 “写宏代码，是一种捷径”里的宏指什么？** 我的理解：把已有命令组合成一个脚本，比从头开发工具便宜，所以排在做工具的最前面。不是 AIWorkspace 的宏。理解错了请指出。影响 SI4。
答复：

**Q14 `llm_tool_carft` 的空壳删不删？** 它是“命令不存在时现场打造”的脚手架，只实现了拒绝分支，注入它的 shell 钩子没找到。架构 §9.5.1 已经改成离线处理。建议删，缺工具的信号改从 Work 的 worklog 里读（退出码 127）。影响 SI7。
答复：

**Q15 self improve 缺省开还是关，预算多少？** 现在结晶缺省关，因为看一次约 18 万 token。建议缺省开，用预算和门槛控制成本：周期预算 24 小时 60 万不变；检查脚本的门槛保守（新增的、用过手册的 Work 够多才看）；Skills 页上的结晶开关改成 self improve 的两个职责开关（结晶、做工具）。关着就一直没有数据，也就谈不上降本增效。影响 SI1、SI5。
答复：

**Q16 和另外两份 TODO 的先后。** 建议：Plan / work_simple TODO 先落地（它改 `analyze_intent`、`goal`、`work_simple`、`plan_simple`）；二期单的 WG1、WG2 在本文的 FX 之前；二期单的 GJ7（定时输入不堆积）不等二期，随本文的 RT 一起做。其余互不影响。
答复：

## 2. 约束

### 2.1 owner 2026-11-24 的脑图（原文）

```text
- goal
    - 内置agent goal : self improve
      - 整理memory和日程
      - 结晶skill
      - 制作新的工具(用deno / python)
        - 写宏代码，是一种捷径
      - 目的：首先是能完成任务，然后是降本增效
    - 设计目标： 通过持续的运行接近或保持一个长期的目标
    - 定时唤醒，执行一次推理
      - 自我维护
        - 目的：根据历史记录，降本增效
        - 更新过滤器
        - 是否要切换模式
          - 模式1：每次唤醒固定几个turn模式，goal session的history就能看到每次唤醒后的全部操作
          - 模式2：sub session模式，每次唤醒都是构造2个work_simple(探索+todo)
          - 历史记录：每次唤醒继承历史，还是只集成最后一次的Report？
        - 长期失败处理
          - 定向求助:通过MessageHub求助主人或其它人
          - 非定向求助:通过HomeStation求助其它Agent
      - 判断现状，是否要推进目标
        - 创建worksesion
    - 用户goal
      - 自动创建的goal
      - 手工添加的goal
      - goal 太多怎么办？
        - 做硬Limit？
        - 合并简单的goal?
        - 全部合成一个？ 能处理goal之间的矛盾
    - 允许以chatsession模式查看？
    - 支持标准的llm_context压缩
    - 如何ask-user?
      - default ui session
    - 首次执行：一个标准的创建goal 的worksession, 根据报告决定工作成功还是失败？
```

脑图里带问号的几处没有裁决，对应 §1 的问题。之后 owner 要求“结合你的理解（认为值得做的部分）以及来自实际场景的新需求”写本文，所以取舍是整理者做的，都列在 §1 和 §10。

### 2.2 之前定过、对本文有效的

| # | 内容 | 出处 |
| --- | --- | --- |
| E1 | 机制越少越好，优先用已有的事件和提示词 | Plan / work_simple TODO D6、D9 |
| E2 | 先能力再安全，缺口记成已知风险 | 多处 |
| E3 | Goal 是珍贵的资源，不靠多建 Goal 分摊职责；内置配置只是初始状态，可以自我演化，不给内置 Goal 开特例 | 旧单 §8 |
| E4 | Goal 的早期运行为后面的重复交付准备工具；交付都是命令 | 旧单 §9 |
| E5 | 同一个 Workspace：Work 开发，Goal 运维；Work 可以改写 Goal 的维护目标；大问题请求用户开 Work 升级 | 架构 §7.10 |
| E6 | Goal 的周期性工作倾向用脚本完成 | 二期单 D1 |
| E7 | 四道门槛：硬保障、硬间隔、预算、检查；预算用量要让 Goal 看得见 | 架构 §12.1.1 |
| E8 | 检查脚本只读，不给 run 和写权限；不为“Agent 把脚本写坏”另加机制（Q8 请求改一处） | 旧单 §8 |
| E9 | skill 是纯文本；安装、删除、禁用只有 owner 能做（Q2 请求放宽一处） | 2026-11-09 |
| E10 | Goal 找人靠通知，不在对话里开口；非 quiet 的通知跟随默认会话 | 架构 §15.7.2 第 5 条、聊天路由 TODO Q7 |
| E11 | Goal 建的 Work 缺省 `work_simple`（Q6 请求放宽提示词） | 2026-11-02 |
| E12 | 用户没有 PC，体验按手机评估 | 产品原则 |
| E13 | 默认处于开发阶段，不做兼容 | AGENTS.md |

### 2.3 实际场景里暴露的问题

| 场景 | 暴露的问题 | 本文的任务 |
| --- | --- | --- |
| 个人重排站（二期单） | 例行工作要脚本跑、不推理；建站是 Work，之后交给 Goal；大问题要请求升级 | 二期单 GJ、WG；本文 FX 把“先建后管”推广到所有 Goal |
| 同上 | Work 占着 Workspace 几小时后放行，Goal 连续打开多次 Activation | RT1（即二期单 GJ7） |
| 站立提醒（2026-11-11 DV） | 一个提醒占一个 Goal；首次 Activation 9.2 万 token；Agent 写的脚本只在整分钟命中，宿主延迟时提醒静默丢失；脚本写坏后 72 小时才被发现 | CA1、CA2、FH5 |
| 历次 DV 的成本 | 一次 `no_action` 2.5–7.8 万 token；结晶看一次 18.2 万；Goal 里跟长命令三次 Activation 37.8 万 | S0-1、CX1–CX3 |
| 真实 Zone 补验（旧单 §7.3） | Work 自己的任务结束事件出现在 Goal 的唤醒原因里；结晶开关关掉后 Goal 仍显示 active | RT2、SI5 |
| 日历和待办接入（2026-11-20） | 提醒有了两条路；“整理日程”等的前置已经具备 | CA1、CA3 |
| 默认会话（聊天路由） | Goal 的提问已经投到默认会话，但只有自由文本，对话里看不到 Goal 在做什么 | FH4、UX2 |

## 3. 事实

### 3.1 内置 Goal

- 播种在 [host/maintenance.rs](../src/frame/lib_opendan/src/host/maintenance.rs)：`memory_maintenance_create`（`:33`，每 30 分钟检查，单次 12 万 token / 4 个 run，周期 120 万，`max_works = 0`）和 `skill_crystallize_create`（`:82`，每天检查，单次 30 万，周期 60 万、最多 2 次 Activation，`max_works = 0`）。包内的检查脚本在 `jarvis/goal_checks/`，Goal 改过的不被升级覆盖（`sync_package_check`，`:120`）。
- behavior 在 [agent.toml](../src/apps/jarvis_agent/agent/agent.toml) `:125-137` 指定。两份专用 behavior 都禁止建 Work；Memory 整理还禁止发通知和跑 shell（只留 `agent-session goal check …`）。
- 内置 Goal 的身份和能力写在 `extensions.opendan`（[protocol/goal.rs](../src/frame/lib_opendan/src/protocol/goal.rs) `:74`），只有宿主的播种路径能写。`CAP_MEMORY_CONSOLIDATION`（`:65`）打开 `memory_consolidation_begin / commit` 两个宿主工具，并把 drive 绑到整理租约；`records_perception = false` 让它自己的运行不产生感知。整理 Goal 的每个时间槽还有一次不用模型的机械推进（[host/goals.rs](../src/frame/lib_opendan/src/host/goals.rs) `:1008`），单次预算被压到周期余量以内（[runner/goal.rs](../src/frame/lib_opendan/src/runner/goal.rs) `:869`）。
- `policy_version`（`protocol/goal.rs:68`）更高时宿主会修订已有内置 Goal 的标记，不动 owner 改过的预算、时间表和状态。
- 结晶开关在 `.meta/skills_state.json` 的 `crystallize`（kRPC `skills.crystallize_set`）。关掉后 Goal 仍显示 `active`。
- 整理发现的候选：类型已冻结（`MaintenanceCandidate`，`protocol/goal.rs:1020`），页面上有列表位置，发现、确认、落地都没实现。挂点是 `runner/consolidation.rs:854`（提交后发布候选）和问题的 `candidate_id`（`runner/goal.rs:1188`、`:1213`，现在恒为空）。

### 3.2 工具与手册

- `<agent_root>/tools` 被认作 Agent 自己的命令目录（[baseline.rs](../src/frame/lib_opendan/src/baseline.rs) `:562`），解析手册声明的命令时排在最前（`runtime/mod.rs:1066`）。**没验**：它在不在会话 shell 的 `PATH` 上（S0-2）。
- 现在没有任何东西往 `tools/` 里产出。`skill_crystallize.toml:52` 明令禁止结晶 Goal 写 `tools/`、`cmd_skills/`、`plan_skills/`、`goal_skills/`，它只能 `agent-session skill revise`（来源 `local` / `imported` 的正文）和写 `skills_drafts/`。
- skill 包里出现 `tools/` 或非文本文件会被拒绝（AGENTS.md 的 cmd_skills 一节）。
- [llm_tool_carft.rs](../src/frame/agent_tool/src/llm_tool_carft.rs) 只有骨架：流水线一律落到最后一步，退出码 127。
- 手册的使用记录在 `.meta/skills_state.json` 的 `usage`（哪些 Work 带了哪份手册、哪些没带），结晶据此读 Work 的 worklog 和 report。没有“缺工具”的专门记录。

### 3.3 Activation、历史与成本

- 一次 Activation 是一个 Turn。上限在 `agent.toml:106-109`：30 万 token、6 个 run、1800 秒墙钟；`goal.toml:56` 每个 run 最多 24 轮工具调用。
- Goal 头每次由宿主从 Goal 定义和 Goal State 重建（`runner/goal.rs:798 opening`、`:557 render_builtin`），历史里旧的 Goal 头只留一行（`:913`）。`<last_activation>` 带上次报告的正文，截到 2000 字符（`:1043`）。
- 历史走通用逻辑：`agent.toml:11` 写的是到窗口的 80% 开始压缩、压回 30%。会话级有 `history_budget_tokens`（[protocol/config.rs](../src/frame/lib_opendan/src/protocol/config.rs) `:495`），**没验**它设成 0 时的行为。
- 想要全新上下文只能 restart 开新 epoch（`StateKeep::Facts`，`protocol/goal.rs:388`）。
- Goal Session 不能改自己的定义、预算、硬间隔、硬保障和 behavior，只能改检查脚本和时间表、增删当前 Skills（`goal.toml:50`）。
- Goal 头里和成本有关的只有当期计数（`<this_period>`，`runner/goal.rs:516`）和 `no_action_in_a_row`（`:515`）。每次 Activation 的记录带用量（`GoalActivation.cost`，`protocol/goal.rs:648`），Goal 没有现成的命令去汇总它。
- 成本数字（来自历次 DV）：`no_action` 2.5–7.8 万 token；结晶看一次 18.2 万；提醒 Goal 的首次 Activation 9.2 万；Goal 里跟长命令三次 37.8 万。**没量**：这些数字里系统提示词、Goal 头、历史、工具往返各占多少（S0-1）。

### 3.4 Work、唤醒与失败

- Goal 建 Work 不等它。Work 结束是一次事件，和用户转来的消息、问题的回答一样要过硬间隔（缺省 900 秒）和预算（[goal.rs](../src/frame/lib_opendan/src/goal.rs) `:256 gate`、`runner/goal.rs:202 hold_events`）。手动、epoch 开始、宿主放行的定时检查和硬保障除外。
- Activation 关闭后不再跟它启动的后台任务（`runner/goal.rs:998`）。
- `explore`、`work_simple` 两个 behavior 都在；`plan_simple` 的 Todo 就是用它们跑的（`agent.toml:60-72`）。Goal 显式指定 behavior 或 `--class plan_simple` 宿主不拦，只是提示词写着 `--behavior work_simple`（`goal.toml:29`）。
- Goal State 里和失败有关的：`consecutive_no_action`、`last_error`、`recent_works` 的 `outcome`（`protocol/goal.rs:822`）。Activation 的结局有 `completed / failed / budget_exhausted / stopped / needs_input`（`:570`），没有“连续几次失败”的计数。检查脚本连续出错数在 `schedule.check_errors`（`host/goals.rs:476`）。
- 检查脚本的 `ctx` 有 `now_ms`、`held`、`timezone`、`agent_root`（[goal_check.rs](../src/frame/lib_opendan/src/goal_check.rs) `:415`）。`now_ms` 是实际运行的时刻，不是时间槽的时刻；多个槽合并成一次时也一样。
- `agent-session goal check run --script` 要求 Goal 已经存在（[xagent_cli.rs](../src/frame/lib_opendan/src/xagent_cli.rs) `:2927`）。
- 旧单 §7.3 第 3 条（Work 的任务结束事件出现在 Goal 的唤醒原因里）来源一直没查。

### 3.5 找人

- Goal 的 `questions` 每条变成一条可回答的通知，同一文本只发一次（`runner/goal.rs:1314`）；回答打开一次 Activation（原因 `answer`）。只有自由文本。Goal Session 里没有 `ask_user`。
- `normal` 级的 Goal 通知投到默认会话（[msg_center/notify.rs](../src/frame/msg_center/src/notify.rs) `:1145`）。问题关闭时会告诉卡片出现过的那些对话的 UI Session（`lib_opendan/notify.rs:415 conversations_of`）。
- `agent-session` 的子命令里没有“给某个人发消息”（`xagent_cli.rs:3427-3450`）。`owner` 手册的 `xcall owner notify / ask` 只到主人，`owner.contacts find` 只查人。
- HomeStation：只有宿主的自动分享（[share.rs](../src/frame/lib_opendan/src/share.rs)，用户发起的 Work 结束后发一条经验帖）。会话没有发帖和读回复的命令。HomeStation 服务端有评论（`comments.rs`）。

### 3.6 创建与管理

- 建 Goal 只有 owner 和 UI Session 能做（[state/goals.rs](../src/frame/lib_opendan/src/state/goals.rs) `:618`）。Work 一律拒绝，这是二期单 WG1 要改的。
- 意图分析的结果有 `goal_created`（[analyze_intent.toml](../src/apps/jarvis_agent/agent/behaviors/analyze_intent.toml) 的 `outcome` 枚举，宿主校验在 `runner/reports.rs:763`）。提醒分支写着“create a goal that sends it”。它已被要求优先并入已有 Goal（“goals are scarce: fold it into one that fits”）。
- 带检查脚本且不带 `--start` 的 Goal，epoch 开始先跑检查，不推理（`host/goals.rs:886`）。
- 页面：Agent 主页（`src/frame/opendan/web`）有 Goals 列表和详情，活动 Goal 超过 16 个时提示（`pages/Goals.tsx:117`），没有创建入口，owner 只能改预算和时间表。Goal Session 在 `SessionDetail.tsx` 里以 worklog 表格显示。
- Desktop 里只有 Agent 面板的活动 Goal 个数（`app/messagehub/agentpanel/model.ts:11`）。现成可复用的：`components/conversation/worklog/`（`agentServiceClient`、`session.worklog`）、`SessionConversation.tsx`、`worksessions/`。
- 日历：`owner` 手册写 `xcall owner.cal add` 的 `reminders`（开始前多少分钟）会让主人按时收到通知，重复日程也算，“不要在别处再排同一个提醒”。**没验**（S0-3）。

## 4. 设计

### 4.1 内置 Goal `self_improve`（Q1、Q15）

- 一个内置 Goal，生命周期 `maintain`，behavior 用标准的 `goal`。目标：让这个 Agent 先能完成任务，再降本增效。
- 职责各是一份 goal skill，放 `jarvis/goal_skills/`，写进 `initial_skills`。Goal 只在这次唤醒的原因指向某项职责时读那一份。

| 职责 | goal skill | 材料 | 产出 |
| --- | --- | --- | --- |
| 结晶手册 | `self_improve_manuals`（内容来自现在的 `skill_crystallize.toml`） | 手册使用记录、Work 的 worklog 和 report | 修订 `local` / `imported` 手册；其余写草稿并通知 |
| 做工具 | `self_improve_tools`（新） | 同上：退出码 127、反复出现的多步命令 | `tools/` 下的命令和它的手册（Q2） |
| 整理 Memory（第二步） | `self_improve_memory`（内容来自 `memory_maintenance.toml`） | 待处理的感知 | 认知变更 |
| 整理日程（第二步） | 同上的一节 | 整理中发现的日程、待办、长期职责 | 候选，问过主人后落地（Q12） |

- 现在写在两份 behavior 里的边界（不安装 skill、不改 UI 的手册集合等）移到 Goal 定义的 `constraints` 和对应的 goal skill。
- 检查脚本 `goal_checks/self_improve.ts`：读使用记录和职责开关，返回的 `reason` 指明职责和数量，例如 `manuals: 7 new work sessions used cmd skills`。一个职责关着，它的条件就不返回 `true`。
- 预算：单次 30 万，周期 60 万，`max_works` 放开（做工具需要派 Work）。第二步并入 Memory 时再加上它原来的额度，并保留“单次不超过周期余量”那条。
- 第二步并入 Memory 时保留 `CAP_MEMORY_CONSOLIDATION`、两个宿主工具、租约和回执，只把规则从系统提示词搬到 goal skill。`records_perception` 保持 `false`，否则它会整理自己产生的感知。
- 已有 Zone：优先用 `policy_version` 的修订路径把已有的内置结晶 Goal 改成 self improve。不能留下一个 behavior 已被删除、加载不了的 Goal。

### 4.2 Agent 自己做的工具（Q2、Q13、Q14）

- 先后顺序写进 `self_improve_tools`：已有的命令或已装的包能做到（只需要写进手册）；把已有命令组合成脚本（“宏”）；包管理器安装；开源实现在本机构建；自己写。脚本用 Deno 或 Python。
- 落点：命令放 `<agent_root>/tools/<name>`。每个命令配一份来源为 `local` 的手册，`commands` 里写这个命令名，正文写什么时候用、怎么用、怎么验。Work 靠这份手册发现它。
- 判断依据和结晶相同：至少两个 Work 显示同一个缺口。一次是事件，不是规律。
- 轻的当轮做完；要调试的派一个 Work（任务书写清输入输出和怎么验），下一次唤醒读报告后再登记手册。
- 做完通知主人一次（稳定的 `--key`），说明做了什么工具、依据是哪些 Work、在哪禁用。
- 宿主要补的只有一处：允许 self improve 新建 `local` 手册（现在只能修订已有的）。校验沿用安装时的 lint。

### 4.3 历史与工作方式（Q3、Q6）

- **历史继承**（S0-1 之后才定做不做）：Goal 定义的 `schedule` 里加 `context: fresh | carry`。`fresh` 时一次 Activation 的上下文是系统提示词、Goal 头和本次输入，Activation 内部的历史照常保留，之前的 Activation 不进上下文，worklog 照常完整。改它和改检查脚本同一条路（`set_check` 那类，记进 `changes.jsonl`，不增加修订号）。
- `fresh` 时 `<last_activation>` 的 2000 字符上限要看够不够；不够就把上次 `result` 的 `distance`、`pending` 之外的要点交给 Goal 自己写进 `pending`，不加大头部。
- **工作方式**不是字段，是 `goal.toml` 的一段话和一份 goal skill（`goal_upkeep`，见 §4.8）：

| 这次要做的事 | 做法 |
| --- | --- |
| 几步就能做完并当场核对 | 自己做 |
| 需要自己的上下文、一件事 | 派一个 `work_simple` |
| 要先弄清楚再动手，或几件事并行 | 派一个 `plan_simple`，由它排探索和执行的 Todo |
| 例行的、不需要判断的 | 写成检查脚本或 Goal 作业（二期单 GJ），不推理 |

- 派活之后本次 Activation 就结束，Work 的结局在后面的唤醒里读报告。不改“Activation 关闭就放手”，不让 Work 的结束绕过硬间隔。

### 4.4 失败信号与求助（Q7–Q9）

- **Goal 头加两个计数**：`activations_failed_in_a_row`（结局不是 `completed`，或没有有效结果）和 `works_failed_in_a_row`（最近吸收的 Work 连续不成功的个数），放在 `<state>` 里，为 0 时不显示。
- **提示词的升级顺序**（`goal.toml` 和 `goal_upkeep`）：同一种做法失败两次就换做法；换过仍失败，向主人求助（`questions`，写清试过什么、卡在哪、要主人做什么）；属于目标本身要变的，请求升级（二期单 §4.4）；主人明确不处理的，降低检查频率并在 `pending` 里留着，不反复提。
- **宿主的通知**（Q7）：连续 3 次时发一条，key 为 `goal:<id>:failing`，计数回到 0 之后才可能再发。不休眠、不改状态。
- **检查脚本连续出错**（Q8）：`check_errors >= 3` 时 `guarantee_due` 返回真，之后按硬保障的现有路径走，Goal 头里已经有 `<check_errors>`。
- **提问带选项**：`questions` 的元素可以是字符串，也可以是 `{text, options: [{id, label}], allow_text}`，映射到通知已有的选项输入。

### 4.5 首次执行（Q4，依赖二期单 WG1、WG2）

1. 意图分析判断要新建 Goal 时，建一个 `work_simple` 的 Work，手册 `goal_setup`，结果 `work_created`。任务书写明目标、范围、怎么判断进展、限制，以及“做完建立 Goal”。
2. Work 做的事：读 `goal list` 确认不该并入已有 Goal、和已有 Goal 没有矛盾；写检查脚本并试跑；准备交付要用的东西（页面、脚本、Goal 作业）；`goal create`（带检查脚本，不带 `--start`）；在报告的 `result` 里写 `goal_id`。
3. 成败是 Work 的结局。失败时不留半成品：Goal 已经建出来的，Work 归档它并在报告里说明。
4. Goal 的第一次运行机会是一次检查，不推理。
5. 意图分析不再自己建 Goal：`goal_created` 从结果枚举里去掉，属于已有 Goal 的请求照旧 `forwarded`。小改动（“以后别提醒周末”）由 UI 直接 `goal revise`，不开 Work（二期单 Q15）。

需要补的一处：`goal check run --script <file>` 在没有 Goal 时也能跑，`ctx` 里没有上次的 `state`，时间表和 Workspace 由参数给。

### 4.6 提醒与日程（Q5、Q12）

- 意图分析的提醒分支：到点就说的写日历（`xcall owner.cal add`，带 `reminders`，重复规则拆成重复日程），结果 `direct`，报告里写清每个时刻。日历表达不了的才建 Goal。
- 检查脚本的 `ctx` 加 `slot_ms`（这次检查对应的时间槽）和 `missed`（合并了几个槽）。`goal_check_script` 手册里按时刻发通知的例子改用 `slot_ms`，晚到窗口写成必须。
- 日程候选（第二步）沿用默认 Goal 单 T05：整理提交之后发布候选，一条候选一个带选项的问题，主人同意后由 self improve 调 `owner.cal` / `owner.todo` 落地；“长期职责”这类候选的落地是给已有的 Goal 增加一项职责（`goal post` 给那个 Goal，或请主人开 Work 改写），不新建 Goal。

### 4.7 让用户看得见（Q10、Q11）

- Desktop 的 Agent 面板：活动 Goal 的个数点开是列表（标题、状态、上次唤醒的结论、本周期用量、未回答的问题数）。
- Goal 会话视图：复用 `SessionConversation` 的外壳和 `worklog` 的数据源。每次 Activation 一组：唤醒原因、折叠的过程（工具调用）、结论（报告正文、决定、建了哪些 Work）。Goal 的提问和它发的通知按消息显示。只读加一个输入框。
- 输入框发的话调 `goals.post`，成为原因是 `feedback` 的输入。用户的话不过硬间隔，仍过预算和 Agent 总量；被预算拦下时视图里要说明什么时候恢复。
- “新建目标”不做表单：按钮打开和这个 Agent 的默认会话并预填一句话，走意图分析（§4.5）。
- 数量：意图分析和 `goal_setup` 的提示词加两条：活动 Goal 达到 16 个时，新建之前必须说明为什么不能并入已有的；发现和已有 Goal 的目标矛盾时问主人，不自己选。并入的做法写进 `goal_setup`：改写目标 Goal 的定义和检查脚本，归档被并入的那个。

### 4.8 自我维护要的数据

- 新命令 `agent-session goal stats [--periods 7]`：按周期汇总 Activation 数、各决定的次数、token（自己和 Work 分开）、检查通过 / 不通过 / 出错的次数、平均每次 Activation 的 token。数据都在已有的 Activation 记录和 `schedule.json` 里，只读。
- goal skill `goal_upkeep`（新）：什么时候看 `goal stats`（`no_action_in_a_row` 高、预算吃紧、硬保障唤醒时）；怎么据此收紧检查、调稀时间表、把重复的判断写成脚本、换工作方式、切换历史继承；失败时的升级顺序（§4.4）。`goal.toml` 的“Keep yourself cheap”一段指向它。

### 4.9 本文新增的宿主机制

只有这五处，其余都是提示词、手册和已有命令的组合：

1. self improve 可以新建 `local` 手册（Q2）。
2. `context: fresh | carry`（Q3，S0-1 之后可能不做）。
3. 连续失败的计数和一条通知（Q7）；检查连续出错并入硬保障（Q8）。
4. 用户的话不过硬间隔（Q10）。
5. 检查的 `ctx.slot_ms`、提问带选项、`goal stats`、`goal check run` 不要求已有 Goal：四处小的接口补充。

## 5. 任务

### 5.0 S0：先量、先验

- [ ] **S0-1 量一次 Activation 的成本构成。** 取本机 Zone 上跑了两周的内置整理 Goal 和一个用户 Goal，从 Session 的 `usage.jsonl` 和 worklog 算：系统提示词加工具定义、Goal 头、历史、每轮工具往返各多少 token；`no_action` 平均几轮。结果写进 §3.3，据此定 Q3。
- [ ] **S0-2 `tools/` 在不在 PATH 上。** 在 Agent 容器里往 `<agent_root>/tools` 放一个脚本，从 Work 的 shell 里直接按名字调。不在就记下要补的落点。
- [ ] **S0-3 日历提醒。** 真实 Zone 上建一条带 `reminders` 的重复日程：到点谁发、发到哪、Agent 容器停着时发不发、手机上有没有。结果写进 §3.6，据此定 Q5。
- [ ] **S0-4 Work 的任务事件。** 查旧单 §7.3 第 3 条的来源（`runner/children.rs`、`goal.rs:366 reason_of`）。
- [ ] **S0-5 读 §1 的答复**，改 §4。

### 5.1 SI：self improve

- [ ] **SI1 播种。** `host/maintenance.rs`、`protocol/goal.rs` 的常量和 `OpendanGoalExtension`、`agent.toml` 的 `[session.goal.*]`；已有 Zone 的处理按 §4.1 末条。
  - 验证：`tests/goal_session.rs` 和 `memory_maintenance.rs` 里涉及结晶播种的用例改过来；升级后的 AgentRoot 上只有一个 self improve，没有加载失败的 Goal。
- [ ] **SI2 goal skill 与检查脚本。** `self_improve_manuals`、`self_improve_tools`、`goal_checks/self_improve.ts`；删 `behaviors/skill_crystallize.toml` 和 `goal_checks/skill_crystallize.ts`。
  - 验证：`cargo test -p libopendan --lib skills`；检查脚本对三种材料（只有手册用量、只有 127、都没有）各跑一次；每个 `behaviors/*.toml` 仍能解析。
- [ ] **SI3 新建 `local` 手册**（Q2）。`agent-session skill` 加一个子命令，限定 self improve 的 Goal Session 和它派的 Work；lint 同安装。
  - 验证：权限逐格测（别的 Goal、UI、普通 Work 都被拒）；新手册出现在 `skill list`，下一次意图分析能选到它。
- [ ] **SI4 做工具的闭环。** 真实模型：造两个 Work 都因为缺同一个命令而绕路的材料，self improve 被检查唤醒，做出工具和手册，第三个同类 Work 用上了它。
- [ ] **SI5 页面。** Skills 页的结晶开关改成两个职责开关；Goal 详情显示职责和各自最近一次的结论。
- [ ] **SI6 第二步：并入 Memory 整理和日程候选**（Q1、Q12）。规则搬到 `self_improve_memory`，删 `behaviors/memory_maintenance.toml`；候选按 §4.6。
  - 验证：`memory_replay --ignored` 的真实模型回放不退化（和搬之前的结果逐条比）；`memory_maintenance.rs` 全过；整理回执页不变。
- [ ] **SI7 删 `llm_tool_carft`**（Q14）。连同 `agent_tool/src/lib.rs`、`agent_tool_cli_dev/src/lib.rs` 里的 `command not found` 入口和文档里的说法。

### 5.2 CX：上下文与成本

- [ ] **CX1 `goal stats`。** `xagent_cli.rs` 的 `cmd_goal`，kRPC 只读。验证：对一个有 50 条 Activation 记录的 fixture 核对每个数。
- [ ] **CX2 `context: fresh | carry`**（Q3，S0-1 之后）。`GoalScheduleSpec`、`set_check` 那条路、`runner` 装配历史的地方、页面。
  - 验证：`fresh` 下第 N 次 Activation 的请求里没有第 N-1 次的工具往返；Activation 中途被杀后续完时本次的历史还在；切换不产生新修订。
- [ ] **CX3 提示词与 `goal_upkeep`。** `goal.toml`：工作方式的表、指向 `goal_upkeep`、`--behavior work_simple` 改成缺省；`goal_skills/goal_upkeep/SKILL.md`。在 Plan / work_simple TODO 之后改（Q16）。
  - 验证：真实模型上一个被故意设成每 5 分钟检查、每次都没事的 Goal，几次唤醒之内自己把检查收紧。
- [ ] **CX4 关闭 F3。** 旧单和架构 §11.2、§11.3、§17.2 写明不做 Goal 专用压缩。

### 5.3 FH：失败与求助

- [ ] **FH1 两个计数进 Goal State 和 Goal 头。** `protocol/goal.rs`、`runner/goal.rs:989 close_activation`、`render_builtin`。
- [ ] **FH2 连续失败的通知**（Q7）。`host/goals.rs`，发送复用 `send_notify`。验证：第 3 次失败发一条；第 4 次不发；成功一次后再连续 3 次才再发。
- [ ] **FH3 升级顺序。** `goal.toml`、`goal_upkeep`、`goal_delivery`。
- [ ] **FH4 提问带选项。** `GoalResult` 的解析、`questions_to_user`（`runner/goal.rs:1314`）、Goal 头的 `<open_questions>`、Desktop 的通知卡片不用改（已有选项输入）。
- [ ] **FH5 检查连续出错并入硬保障**（Q8）。`goal.rs:295 guarantee_due`。验证：出错 3 次后下一个槽唤醒一次，原因 `guarantee`，仍受硬间隔保护。

### 5.4 FX：首次执行

- [ ] **FX1 意图分析。** `analyze_intent.toml`：新建 Goal 改成建 Work；去掉 `goal_created`；`runner/reports.rs:763`、`tests/notify.rs` 等跟着改。
- [ ] **FX2 work skill `goal_setup`。** 指向 `goal_skills/goal_check_script`、`goal_delivery` 的正文路径，不复制；写明并入和查矛盾（§4.7）、失败不留半成品。
- [ ] **FX3 `goal check run` 不要求已有 Goal。** `xagent_cli.rs:2925`、`goal_check.rs` 的 `run_for`。
- [ ] **FX4 DV。** 真实模型三条：一个要判断的监控目标建成并由检查起步；一个建到一半失败，对话里收到失败、没有残留的 Goal；一个应该并入已有 Goal 的请求没有新建。

### 5.5 CA：提醒与日程

- [ ] **CA1 提醒走日历**（Q5，S0-3 之后）。`analyze_intent.toml` 的 Reminders 一节、`goal_check_script` 的对应一节、`ui_core`。
- [ ] **CA2 `ctx.slot_ms` 与 `missed`。** `goal_check.rs:405`、`goal_check_runner.ts`、Agent State Protocol §9.4、手册例子。验证：宿主晚 3 分钟跑检查时，按 `slot_ms` 写的提醒仍然发出且只发一次。
- [ ] **CA3 日程候选。** 并在 SI6 里做。

### 5.6 UX：Desktop

- [ ] **UX1 Agent 面板的 Goal 列表。** `app/messagehub/agentpanel/`，数据 `goals.list`。
- [ ] **UX2 Goal 会话视图。** 新组件放 `components/conversation/`，数据 `session.worklog` 加 `goals.activations`。
- [ ] **UX3 新建目标入口。** 预填用 `hub.saveDraft`（二期单 §3.7 查过没有直接预填的接口）。
- [ ] **UX4 用户的话不过硬间隔**（Q10）。`runner/goal.rs:202 hold_events`。验证：硬间隔内 `goal post` 立刻打开 Activation；预算用完时不打开，视图显示恢复时刻。
- [ ] **UX5 数量与矛盾的提示词。** `analyze_intent.toml`、`goal_setup`。
- [ ] **UX6 多语言与用例。** 10 种语言；`mock/backend.ts` 加 `goals.*`；手机视口的 e2e。

### 5.7 RT：运行中的问题

- [ ] **RT1 定时输入不堆积。** 即二期单 GJ7，随本文先做。
- [ ] **RT2 Work 的任务事件不进 Goal 的原因。** S0-4 有结论后修。

### 5.8 DOC

- [ ] **DOC1 架构设计。** §9.1（改回一个 self improve，说明和“迁移为不同 Goal”的关系）、§9.3、§9.5、§11.2、§11.3、§12.1.1、§15.7、§17.2。
- [ ] **DOC2 协议与手册。** Agent State Protocol §9；`opendan_agent_dev`、`ui_core`、`skill_authoring`、`goal_check_script`、`goal_delivery`；AGENTS.md 的联动表（内置 Goal、goal skill、`goal stats`）。
- [ ] **DOC3 旧单。** 标注 F3、E1–E3、§7.3、§10.7 的去向。

## 6. 顺序

| 里程碑 | 内容 | 出口 |
| --- | --- | --- |
| M0 | S0；Q1–Q5 定稿 | 成本构成、日历提醒的实测结论 |
| M1 省钱、不丢 | CX1–CX4、FH1–FH5、CA2、RT1、RT2 | 一个空转的 Goal 会自己收紧；连续失败主人知道 |
| M2 先建后管 | 二期单 WG1、WG2，FX、CA1、UX5 | 新建 Goal 的成败回到对话；提醒不占 Goal |
| M3 自我改进 | SI1–SI5、SI7 | self improve 做出第一个被 Work 用上的工具 |
| M4 看得见 | UX1–UX4、UX6 | 手机上能看一个 Goal 在做什么，并对它说话 |
| M5 并入 Memory | SI6、CA3、DOC | 只剩一个内置 Goal |

M1 的 Rust 部分、M4 的 Desktop 部分可以并行；M2 等 Plan / work_simple TODO 落地（Q16）。

## 7. 验收

| # | 场景 | 通过条件 | 怎么验 |
| --- | --- | --- | --- |
| V1 | 一个没事可做的 Goal 跑三天 | 第三天的 token 明显少于第一天；它自己改过检查脚本或时间表；没有人提醒它 | 真实 Zone + 真实模型 |
| V2 | 一个一直失败的 Goal | 主人收到一条看得懂的求助或失败通知，不是每次都发；Goal 没有重复同一种做法超过两次 | 人为让它的 Work 必然失败 |
| V3 | 检查脚本被写坏 | 3 个槽之后 Goal 被唤醒并修好；固定时刻的通知最多漏这几个槽 | 集成测试 + 真实 Zone |
| V4 | 对 Agent 说“帮我盯着某件事” | 出现一个 Work；它结束时对话里有结论；Goal 起步没有推理；建不成时没有残留 | 真实 Zone |
| V5 | 对 Agent 说“每天九点提醒我” | 日历里多一条带提醒的日程；没有新的 Goal；到点手机收到 | 真机 |
| V6 | 同一个缺口出现两次 | self improve 做出工具和手册并通知主人；第三次同类 Work 用了它，轮数减少 | 真实模型 |
| V7 | 手机上看一个 Goal | 能看到每次唤醒为什么醒、做了什么、结论是什么；发一句话，一分钟内有回应 | 真机 |
| V8 | 第 17 个 Goal | 意图分析先说明为什么不能并入，或直接并入了已有的 | 真实模型 |
| V9 | Memory 整理并入之后 | 回放结果不退化；只有一个内置 Goal；整理回执照常 | `memory_replay --ignored` |

## 8. 验证命令与 DV 注意

- Rust（`src/`）：`cargo test -p libopendan -- --test-threads=1`（`plan_simple_async`、`work_simple_async` 共用进程环境变量，并行会假失败）；`--test goal_session`、`--test memory_maintenance`、`--lib skills`、`--test xagent_cli skills_are`；`cargo test -p opendan`。协议 schema 文档用 `target/debug/xagent schema <dir>` 重新生成。
- 真实模型回放：`memory_replay --ignored`（SI6 前后各跑一次存档）。
- Desktop（`src/frame/desktop`）：`pnpm check`、`pnpm lint`、相关 e2e、`desktop-i18n.spec.ts`。Agent 主页（`src/frame/opendan/web`）：`pnpm check`、`tests/e2e/goals.spec.ts`。
- 部署：改 behavior、goal skill、检查脚本要重新打 Jarvis 包；容器内的 Rust 改动要 `./build_aios --local-test` 再 `uv run src/start.py --skip-update`。
- 真实 Zone 的 Goal DV 做法见旧单 §10 和它的证据目录。**测试 Goal 由脚本在拿到结果后自动归档**，不要留着过夜（2026-11-11 一个短硬保障的测试 Goal 烧了约 220 万 token）。
- 手册的描述会影响意图分流，改完先用便宜的消息试分流。

## 9. 已知风险

- Agent 自己做的工具不经主人确认就给 Work 用（Q2）：工具由 Agent 写，被诱导写出有害命令的风险和它平时写代码相同；事后通知，主人可以禁用。
- `fresh` 上下文会丢掉 Goal 没写进报告的东西。Goal 头和报告的质量决定了连续性，要在 V1 里看有没有反复重查同一件事。
- self improve 缺省打开后每天最多 60 万 token（Q15）。门槛写松了会空转，V1 的方法同样适用于它。
- 新建 Goal 多一个 Work（Q4）：一次意图分析加一个 `work_simple` 的固定开销。提醒改走日历之后，剩下的都是需要判断的 Goal，这个开销换来的是检查脚本经过试跑、成败回到对话。
- 用户的话不过硬间隔（Q10）：一直说话可以连续唤醒，由周期预算封顶。
- Memory 整理的规则从系统提示词搬到按需读的 goal skill 后，模型可能不读就动手。SI6 的回放是唯一的保证，不过就不并。
- 定时仍只在 Agent 进程在线时触发（旧单 D3 未做）。

## 10. 不在本文

- **求助主人以外的人。** 要先定：以谁的名义发、能联系谁、对方的回复怎么回到 Goal（现在只会进 Agent 和那个人的 UI Session）。最小的做法是加一条发消息的命令并由 UI 转 `goal post`，等授权规则定了再做。
- **在 HomeStation 上求助其他 Agent。** 要先有读的一方：现在没有 Agent 会去看别人的求助帖。最小的做法是会话可用的发帖命令加检查脚本读评论，等有第二个会读的 Agent 再做。
- **在一次唤醒里等子会话跑完**（模式 2 的严格形态）。
- **Goal 数量的硬上限、合并命令、所有 Goal 合成一个。**
- **Goal 专用的压缩策略。**
- workflow 计划后端（旧单 D3）、Goal 作业与 Work 改写 Goal 的实现（二期单 GJ、WG）、XLLM 结晶、子 Work 的预算预留、跨节点运行。

## 11. 实施记录

| 日期 | 任务 | 记录 |
| --- | --- | --- |
