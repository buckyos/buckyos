# Goal Session 改进 TODO

日期：2026-11-24 · 事实的行号取自 `1704812f8` · 状态：**已定稿（owner 2026-11-24）；同日实施，未提交，未部署；真实 Zone 的验收没有跑（§11）**

需求来源有三处：owner 2026-11-24 重新思考 Goal Session 的脑图（§2.1 原文）、当天按代码做的差距核对、几个实际场景里暴露的问题（§2.3）。

**实施顺序（owner 2026-11-24 已定，§2.4）：Plan / work_simple（进行中）→ 本文 → Agent 浏览器二期。** 本文的规划按这个顺序排：

- [Plan / work_simple TODO](jarvis-plan-work-simple-todo.md)（下称“Plan 单”）先落地。本文以它完成后的代码和提示词为基线，开工第一件事是重新核对行号（S0-0）。
- [Agent 浏览器二期 TODO](agent-browser-phase2-todo.md)（下称“二期单”）排在本文之后。它的 WG（Work 开发、Goal 运维）、GJ（Goal 作业）两组是通用的 Goal 能力，改由本文排期和验收（GJ 也在本文做，Q18 已定）。这两组的设计仍写在二期单 §4.2–§4.4，本文不重抄，只写合并关系（§4.10）。二期开工时这两组应该已经完成（§6 的出口）。
- [Goal Session 继续推进 TODO](opendan-goal-session-next-todo.md)（下称“旧单”）：本文接替它没做完的 F3、E1–E3、D4 里和提醒有关的部分，以及 §7.3、§10.7 的遗留。D3（workflow 计划后端）仍留在旧单。
- 现状见[架构设计](<../doc/opendan/OpenDAN Agent Session架构设计.md>) §15.7。

## 0. 怎么用这份文档

**给 owner。** 本文已定稿，现在没有等你回答的问题。先定的七条是你的答复，落实在 §1.3；我的四处理解你已确认；没有单独答复的各条按建议定案。Q17–Q19 是从二期单带过来的，结论在这里，二期单不用再答。实施中出现要你定的新问题，会写进 §11 再问你。

**给 CodeAgent。**

- §1 的问题都已定案：有答复的按答复和 §1.3 的落实，没有答复的按“建议”。“另一种做法”都没有采纳，不要再加回来。
- §2 是约束，不重新讨论。和代码现实冲突时停下来写进 §11，问 owner。
- §3 是查过的事实，标“没验”的要先验证。行号以基线为准，动手前核对。路径里 `lib_opendan/` 指 `src/frame/lib_opendan/src/`，`jarvis/` 指 `src/apps/jarvis_agent/agent/`。
- §4 是设计，名字和字段可以调整，调整后回写本文和协议文档。
- 做完一条，在 §11 记一行：改了什么、跑了什么、结果、没验什么。
- 遵守 [AGENTS.md](../AGENTS.md)：不做兼容实现；改动范围小；手册和代码同一变更内更新。
- **机制越少越好。** 每一处先看能不能用已有的命令、事件、提示词做到。本文新增的宿主机制只有五处，另有从二期单接手的两处（§4.9），多出来的要先问。
- 标着“见二期单”的任务，内容、落点和验证以二期单的对应条目为准；和本文任务合并做的，以 §4.10 的合并关系为准。
- 同机别的会话在改同一批文件（Plan 单进行中）。只动自己的文件，不要用 `git stash` 做基线。
- 不要提交，除非 owner 要求。

## 1. owner 决策（2026-11-24 已定稿）

定稿时 owner 的话：

> 四处理解都对，按这个版本定稿

“四处理解”指 §1.3 里标着“我的理解”的两处（Q2、Q18）和答复之后改过的两条建议（Q11、Q13）。下面保留各条问题的原文、建议和答复，作为裁决记录。

### 1.1 先定

**Q1 内置的 self improve 怎么合并？**

- 现状：两个内置 Goal，`memory_maintenance`（缺省开）和 `skill_crystallize`（缺省关），各有一份专用 behavior。没有“做工具”这项职责。
- **建议：合成一个 `self_improve`，分两步。** 它用标准的 `goal` behavior，每项职责是一份 goal skill，一份检查脚本按职责给出唤醒原因。
  - 第一步：把 `skill_crystallize` 改造成 `self_improve`，承担结晶手册和做工具。
  - 第二步：把 Memory 整理并进来，同时接上日程（Q12）。
- 分两步的理由：Memory 整理的质量依赖那份很长的专用提示词，搬成 goal skill 之后要用真实模型回放确认不退化；它还有宿主驱动的回执、清理和租约绑在 Goal 上（§3.1）。结晶和做工具没有这些负担。
- 另一种做法：一步全并；或只并结晶和做工具，Memory 整理一直单独。
- 影响：SI 全组。
- 答复：按建议合并

**Q2 Agent 自己做的工具，要不要你确认才给 Work 用？**

- 现状：2026-11-09 定了 skill 是纯文本，安装、删除、禁用只有 owner 能做；结晶 Goal 只能改写来源为 `local` / `imported` 的手册正文，其余写草稿。Work 靠手册发现命令，所以“工具做出来就能用”等于允许 self improve 自己新建一份 `local` 手册。
- **建议：允许。** self improve 可以往 `<agent_root>/tools/` 放命令，并新建来源为 `local` 的手册，做完通知你一次，你可以在 Skills 页禁用。改随包安装的手册、动 `scope` 往 `ui` 方向，仍走草稿。按“先能力再安全”记为已知风险（§9）。
- 另一种做法：新手册一律走现有的草稿流程，你在 Skills 页点安装后才生效。不用改宿主，但每个新工具都等你一次。
- 影响：SI3、SI4。
- 答复：允许，之前完成的owner系列工具其实就已经是一种“快捷方式”类的结晶了

**Q3 每次唤醒继承全部历史，还是只带上次的结论？**

- 现状：两样同时在。同一个 Session 的历史全部继承，走通用压缩；宿主每次另外重建 Goal 头，里面有上次的结论、距离、待办、问题和 Work。
- **建议：先量（S0-1），再定。** 历史占一次 `no_action` 成本两成以上，就把缺省改成“每次唤醒开新上下文，只带 Goal 头”，Goal 可以自己切回继承（和改检查脚本一样，不增加修订号），你也能在页面上改。不到两成就维持现状，不加这个开关。
- 两种情况下都不做 Goal 专用的压缩，继承历史时走标准压缩。旧单的 F3 就此关闭。这和你对 Plan 历史的裁决一致（Plan 单的 D5）。
- 影响：CX2。
- 答复：按建议

**Q4 新建 Goal 一律先过一个“建 Goal 的 Work”？**

- 现状：意图分析直接 `goal create --start`，对话立刻收到 `goal_created`。第一次 Activation 自己写检查脚本；它失败了 Goal 仍是 active，用户不知道。
- **建议：统一。** 意图分析遇到要新建 Goal 的请求，建一个 `work_simple` 的 Work（手册 `goal_setup`）。Work 把检查脚本写好并试跑，准备好交付要用的东西，最后建 Goal。成败就是 Work 的结局，结果自己回到对话。Goal 从第一天起只做运维。
- 这改了二期单 WG3 里“纯观察和提醒仍直接建 Goal”那一句。它需要 Work 能建 Goal、命令行能在创建时装检查脚本（WG1、WG2），这两条改在本文先做（Q17）。
- 另一种做法：只有“先建设后维护”的请求走 Work（二期单现在的写法），简单的 Goal 仍直接建。少一个 Work，但首次失败仍然没人知道。
- 影响：FX 全组、WG3。
- 答复：按需要，不固定。我想work是独立的，在work的任务要求里说明最后要建立 维护/持续改进的 goal session

**Q5 固定时间的提醒，改成优先写进日历？**

- 现状：2026-11-11 定了固定提醒由 Goal 的检查脚本发通知。2026-11-20 接入了 `owner.cal`，日程可以带 `reminders`，手册写着重复日程也会提醒。现在同一句“每天九点提醒我”有两条路，意图分析走的是建 Goal。
- **建议：到点就说、不需要判断的提醒写日历，不建 Goal。** 日历表达不了的规则（“工作日 15 点到 18 点每 50 分钟”这类可以拆成几条重复日程，拆不了的才算），以及要看了情况才决定说不说的提醒，仍用检查脚本。这样一个提醒不再占一个 Goal。
- 前提没验：日历提醒由谁在什么时候发、Agent 不在线时发不发、手机上收不收得到（S0-3）。验不过就维持现状。
- 影响：CA1。
- 答复：之前讨论过，一个提醒如果用户需要在日程里看到，就加。用户是否需要由Agent自行判断

**Q17 Work 凭什么能建立和改写 Goal？**（二期单 Q2，另一种做法的细节在那里）

- 现状：建 Goal 只有 owner 和 UI 会话能做；UI 只能修订自己建的 Goal；Work 一律拒绝。会话身份是自报的，这套权限只防误用。
- **建议（同二期单）：规则放宽到够用，不加授权字段。** 直接由 UI 会话创建、且不属于某个 Goal 的 Work，对 Goal 的权限等同它的父 UI。对端是 owner 的 UI 会话可以修订这个 Agent 的任何非内置 Goal。Plan 的 Todo、Goal 派的 Work 不行。身份自报的缺口这一期不补，记为已知风险。
- 另一种做法：创建 Work 时带结构化授权并堵住身份自报，要改会话协议、登记表和 kRPC 鉴权。
- 影响：WG1、WG2，FX 依赖它。
- 答复：按建议

**Q18 Goal 作业（二期单 GJ1–GJ6）在本文做，还是留到二期？**

- 背景：二期单 Q1 建议新增 Goal 作业：属于一个 Goal、由宿主按时间表在 Agent 的私有 runtime 里以 Agent 身份运行、不推理的程序（设计在二期单 §4.2）。另一种做法是扩展 AIWS 的同步任务。你定过例行工作用脚本是通用规则。
- **建议：在本文做，放在最后一个里程碑；载体选 Goal 作业。** 理由有两条。一是按你定的顺序，二期一开工就要用 GJ1–GJ3。二是没有它，“Work 开发、Goal 运维”只有检查脚本一个载体，而检查脚本不能写文件、不能做事，证明通用性的 DV（WG7）跑不成。
- 另一种做法：留在二期单，本文只做 WG 和 GJ7，WG7 的 DV 和 V10 推到二期。本文少一个里程碑，二期多一段和重排站无关的工作。
- 这一条同时回答二期单 Q1。
- 影响：GJ 组、M6、V10。
- 答复：按建议，按Q4的结构，不搞特殊化

### 1.2 有建议值

这一组除 Q11、Q13 外没有单独的答复，随定稿按建议定案。

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

**Q11 Goal 太多怎么办？** 建议三个选项都不做成机制：不设硬上限，不加合并命令，不把所有 Goal 合成一个。靠四件事控制数量：固定提醒并入同一个提醒 Goal，一份检查脚本管多条；意图分析优先并入已有 Goal（已有）；活动 Goal 达到 16 个时，新建之前必须说明为什么不能并入（提示词）；Agent 后台总量封顶（已有）。Goal 之间的矛盾在新建和改写的时候查（提示词），不在每次唤醒时查。“全部合成一个”现在做不了：一个 Goal 只有一份检查脚本、一份预算、绑定一个 Workspace。影响 UX5。

这条建议在 Q5 答复之后改过。原来第一件事是“提醒改走日历”；按 Q5 的答复，提醒进不进日历由 Agent 判断，不进日历的仍然是 Goal，所以换成“并入同一个提醒 Goal”。
答复：改后的建议已确认（定稿时的“四处理解都对”）。

**Q12 “整理日程”指什么，什么时候做？** 我的理解：Memory 整理时发现的日程和待办，问过你之后写进 `owner.cal` / `owner.todo`；发现的长期职责，建议并入已有的 Goal。这就是旧单推后的 E1、E2，它等的日历和待办命令已经有了。建议放在 Q1 的第二步，方案沿用[默认 Goal 单](jarvis-default-goal-session-todo.md)的 T05，候选加一类“给已有的 Goal 增加一项职责”。影响 SI6、CA3。
答复：

**Q13 “写宏代码，是一种捷径”里的宏指什么？** 你在 Q2 的答复里给了例子：owner 系列工具就是一种“快捷方式”类的结晶。我据此改了理解：捷径是把已有的命令和接口包成一个短的、带约定参数的入口，省掉每次重新拼的 token，owner 系列（对象别名加本地动作脚本）是它的一种形态。它排在做工具的最前面。

建议第一期 self improve 产出的快捷方式用命令的形态（`<agent_root>/tools/` 下的脚本加一份 `local` 手册）。对象别名的形态推后（§10）：别名和本地动作脚本现在只从管理员安装的扩展包登记，Agent 对用户覆盖只读（[对象别名 TODO](opendan-object-local-extensions-todo.md) §4 第 1 条，实施后我没有复核），要先给 Agent 开一个自己的登记来源。影响 SI4。
答复：理解和建议已确认（定稿时的“四处理解都对”）。

**Q14 `llm_tool_carft` 的空壳删不删？** 它是“命令不存在时现场打造”的脚手架，只实现了拒绝分支，注入它的 shell 钩子没找到。架构 §9.5.1 已经改成离线处理。建议删，缺工具的信号改从 Work 的 worklog 里读（退出码 127）。影响 SI7。
答复：

**Q15 self improve 缺省开还是关，预算多少？** 现在结晶缺省关，因为看一次约 18 万 token。建议缺省开，用预算和门槛控制成本：周期预算 24 小时 60 万不变；检查脚本的门槛保守（新增的、用过手册的 Work 够多才看）；Skills 页上的结晶开关改成 self improve 的两个职责开关（结晶、做工具）。关着就一直没有数据，也就谈不上降本增效。影响 SI1、SI5。
答复：

**Q16 和另外两份 TODO 的先后。** 已定（2026-11-24，§2.4）：Plan 单 → 本文 → 二期单。

**Q19 运维和升级的几条通用规则**（二期单 Q4、Q5、Q15、Q19，随 WG4、WG5 在本文落地）。建议都同二期单：

- 请求升级走现有的通知和对话，不加机制：Goal 把问题、影响、建议和任务书草稿写成工作区里的一份文件，发一条带链接的通知；主人在对话里答应后，由意图分析建 Work（二期单 Q4）。
- 用户看到的东西、系统的权限和成本都不变，是运维；其余是升级（二期单 Q5）。
- 小改动不开 Work，主人所在的 UI 会话直接修订 Goal（二期单 Q15）。
- 架构 §8.3 的软件维护 Goal 保留现在的行为：维护目标里写明“允许自己派 Work 修改代码”才可以，默认不写（二期单 Q19）。

重排站专有的那一半（来源页上的显示、App 里的“升级”按钮、`site upgrade` 命令）留在二期单。
答复：

### 1.3 答复的落实（2026-11-24）

| 问题 | 你的答复 | 落实 |
| --- | --- | --- |
| Q1 | 按建议合并 | 一个 `self_improve`，分两步（§4.1）。 |
| Q2 | 允许，之前完成的owner系列工具其实就已经是一种“快捷方式”类的结晶了 | self improve 可以往 `tools/` 放命令、新建 `local` 手册，做完通知一次（§4.2、SI3）。“快捷方式”作为做工具的第一种产出写进 §4.2 和 Q13。我的理解（已确认）：这句话是在说明这类结晶已经有先例、值得做，没有要求第一期就能产出对象别名；所以第一期只做命令形态。 |
| Q3 | 按建议 | 先量（S0-1），历史占两成以上才加 `context: fresh \| carry`（§4.3、CX2）。 |
| Q4 | 按需要，不固定。我想work是独立的，在work的任务要求里说明最后要建立 维护/持续改进的 goal session | **没有采纳“一律先过 Work”。** 两条路都留：不需要先建设什么的，意图分析照旧直接建 Goal，`goal_created` 保留；需要先做出东西的，建一个普通的 Work，任务书里写明最后建立负责维护或持续改进的 Goal。走哪条由意图分析判断。没有“建 Goal 的 Work”这种专门的类型（§4.5、FX）。这就是二期单 WG3 原来的写法，WG3 不用改。 |
| Q5 | 之前讨论过，一个提醒如果用户需要在日程里看到，就加。用户是否需要由Agent自行判断 | **没有采纳“优先写日历”。** 判断依据只有一条：主人需不需要在日程里看到它。需要的写进日历（带 `reminders`，不再另建提醒）；不需要的照旧由 Goal 的检查脚本发。由 Agent 判断，不设规则（§4.6、CA1）。S0-3 不再决定做不做，只验证手册里的说法。 |
| Q17 | 按建议 | WG1 按二期单的规则做。 |
| Q18 | 按建议，按Q4的结构，不搞特殊化 | Goal 作业在本文做，放 M6。我的理解（已确认）：“按 Q4 的结构”是说作业没有专门的建立流程。它由普通的 Work 按任务书在收尾时和 Goal 一起装上，或由 Goal 自己装；V10 的 DV 也按这个结构跑。 |

答复带来的其他改动：

- Q4 之后，直接建的 Goal 第一次 Activation 失败时用户仍然不会马上知道，要等 FH2 的通知（连续 3 次）。记在 §9。
- Q4 之后，原来的手册 `goal_setup` 改名 `goal_handover`，内容是“Work 收尾时怎么建立或改写 Goal”，只给任务书里有这一步的 Work。
- Q5 之后，Q11 的第一条建议换了，Q13 按 Q2 的答复重写了。两条改后的内容都已确认。
- 没有单独答复的 Q6–Q10、Q12、Q14、Q15、Q19 随定稿按建议定案。其中 Q3 不需要再问：S0-1 量出来历史占两成以上就做 CX2，不到就不做。

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

脑图里带问号的几处当时没有裁决，对应 §1 的问题。之后 owner 要求“结合你的理解（认为值得做的部分）以及来自实际场景的新需求”写本文，取舍由整理者提出，列在 §1 和 §10，owner 逐条答复或随定稿确认。脑图和 §1 的结论不一致时以 §1 为准，例如脑图里“首次执行：一个标准的创建 goal 的 worksession”最后定的是按需要、不固定（Q4）。

### 2.2 之前定过、对本文有效的

| # | 内容 | 出处 |
| --- | --- | --- |
| E1 | 机制越少越好，优先用已有的事件和提示词 | Plan 单 D6、D9 |
| E2 | 先能力再安全，缺口记成已知风险 | 多处 |
| E3 | Goal 是珍贵的资源，不靠多建 Goal 分摊职责；内置配置只是初始状态，可以自我演化，不给内置 Goal 开特例 | 旧单 §8 |
| E4 | Goal 的早期运行为后面的重复交付准备工具；交付都是命令 | 旧单 §9 |
| E5 | 同一个 Workspace：Work 开发，Goal 运维；Work 可以改写 Goal 的维护目标；大问题请求用户开 Work 升级 | 架构 §7.10 |
| E6 | Goal 的周期性工作倾向用脚本完成 | 二期单 D1 |
| E7 | 四道门槛：硬保障、硬间隔、预算、检查；预算用量要让 Goal 看得见 | 架构 §12.1.1 |
| E8 | 检查脚本只读，不给 run 和写权限；不为“Agent 把脚本写坏”另加机制。Q8 改了一处：连续 3 次出错并入硬保障 | 旧单 §8、本文 Q8 |
| E9 | skill 是纯文本；安装、删除、禁用只有 owner 能做。Q2 放宽了一处：self improve 可以新建 `local` 手册 | 2026-11-09、本文 Q2 |
| E10 | Goal 找人靠通知，不在对话里开口；非 quiet 的通知跟随默认会话 | 架构 §15.7.2 第 5 条、聊天路由 TODO Q7 |
| E11 | Goal 建的 Work 缺省 `work_simple`。Q6 把提示词里的“一律”放宽成缺省，需要时可以派 `plan_simple` | 2026-11-02、本文 Q6 |
| E12 | 用户没有 PC，体验按手机评估 | 产品原则 |
| E13 | 默认处于开发阶段，不做兼容 | AGENTS.md |

### 2.3 实际场景里暴露的问题

| 场景 | 暴露的问题 | 本文的任务 |
| --- | --- | --- |
| 个人重排站（二期单） | 例行工作要脚本跑、不推理；建站是 Work，之后交给 Goal；大问题要请求升级 | WG、GJ（设计在二期单）；FX 让普通的 Work 在收尾时建立 Goal |
| 同上 | Work 占着 Workspace 几小时后放行，Goal 连续打开多次 Activation | RT1（即二期单 GJ7） |
| 站立提醒（2026-11-11 DV） | 一个提醒占一个 Goal；首次 Activation 9.2 万 token；Agent 写的脚本只在整分钟命中，宿主延迟时提醒静默丢失；脚本写坏后 72 小时才被发现 | CA1、CA2、FH5 |
| 历次 DV 的成本 | 一次 `no_action` 2.5–7.8 万 token；结晶看一次 18.2 万；Goal 里跟长命令三次 Activation 37.8 万 | S0-1、CX1–CX3 |
| 真实 Zone 补验（旧单 §7.3） | Work 自己的任务结束事件出现在 Goal 的唤醒原因里；结晶开关关掉后 Goal 仍显示 active | RT2、SI5 |
| 日历和待办接入（2026-11-20） | 提醒有了两条路；“整理日程”等的前置已经具备 | CA1、CA3 |
| 默认会话（聊天路由） | Goal 的提问已经投到默认会话，但只有自由文本，对话里看不到 Goal 在做什么 | FH4、UX2 |

### 2.4 实施顺序（owner 2026-11-24）

> 实施顺序上，plan-work-simple已经在开发了，随后我打算完成goal session的改造，最后再推进 agent-browser-phase2

对本文的含义（整理者的推论）：

1. **Plan 单先落地，本文不重复它做的事，在它的结果上改。** 和本文有关的几项：
   - 三个创建方（意图分析、Goal、Plan）共用一段“一个 work session 需要什么”的规则（`behaviors/work_request_rule.inc`，`goal.toml` 已经包含它）。本文里 Goal 派活、收尾时要建立 Goal 的 Work 的任务书都按它写，要回传的值放进报告的 `result`。
   - Work 等命令时的 tick 退避；`work_simple` 压缩三次仍放不下时以 `context_limit` 失败结束。Goal 派出去的 Work 因此不会无声地卡住，失败会作为结局回到 Goal（FH1 的计数用得上）。
   - Plan 能长时间等待，寿命上限 48 小时。Goal 派 `plan_simple`（Q6）时要知道它最多活这么久。
   - Plan 写 Memory、`work_simple` 不写。Goal 派的 `work_simple` 值得记的东西写在报告里，由 Goal 判断。
2. **二期排在本文之后，二期依赖的通用 Goal 能力在本文做完。** WG、GJ 两组都移到本文（Q18 已定）。本文的验收不能依赖重排站，用和浏览器无关的例子（V10）。
3. **本文的出口就是二期的入口。** 见 §6 末尾。

## 3. 事实

**2026-11-24 S0-0 核对：Plan 单已合入（`a5831ec7d`）。** 下面的行号随它变了，没有逐个改；说法变了的在各节末尾标注。

行号取自 `1704812f8`。Plan 单正在改 `jarvis/behaviors/{goal,analyze_intent,work_simple,plan_simple,explore}.toml`、`jarvis/agent.toml`，以及 `lib_opendan/` 的 `runner/{reports,drive,ticks,budget,outcome}.rs`、`xagent_cli.rs`、`runtime/mod.rs`、`protocol/config.rs` 等，它落地后这些文件的行号会变，个别说法也可能变（S0-0）。二期单 §3.3、§3.4 查过的事实对 WG、GJ 两组有效，不在这里重抄。

### 3.1 内置 Goal

- 播种在 [host/maintenance.rs](../src/frame/lib_opendan/src/host/maintenance.rs)：`memory_maintenance_create`（`:33`，每 30 分钟检查，单次 12 万 token / 4 个 run，周期 120 万，`max_works = 0`）和 `skill_crystallize_create`（`:82`，每天检查，单次 30 万，周期 60 万、最多 2 次 Activation，`max_works = 0`）。包内的检查脚本在 `jarvis/goal_checks/`，Goal 改过的不被升级覆盖（`sync_package_check`，`:120`）。
- behavior 在 [agent.toml](../src/apps/jarvis_agent/agent/agent.toml) `:125-137` 指定。两份专用 behavior 都禁止建 Work；Memory 整理还禁止发通知和跑 shell（只留 `agent-session goal check …`）。
- 内置 Goal 的身份和能力写在 `extensions.opendan`（[protocol/goal.rs](../src/frame/lib_opendan/src/protocol/goal.rs) `:74`），只有宿主的播种路径能写。`CAP_MEMORY_CONSOLIDATION`（`:65`）打开 `memory_consolidation_begin / commit` 两个宿主工具，并把 drive 绑到整理租约；`records_perception = false` 让它自己的运行不产生感知。整理 Goal 的每个时间槽还有一次不用模型的机械推进（[host/goals.rs](../src/frame/lib_opendan/src/host/goals.rs) `:1008`），单次预算被压到周期余量以内（[runner/goal.rs](../src/frame/lib_opendan/src/runner/goal.rs) `:869`）。
- `policy_version`（`protocol/goal.rs:68`）更高时宿主会修订已有内置 Goal 的标记，不动 owner 改过的预算、时间表和状态。
- 结晶开关在 `.meta/skills_state.json` 的 `crystallize`（kRPC `skills.crystallize_set`）。关掉后 Goal 仍显示 `active`。
- **（实施后）** 内置 Goal 只剩 `self_improve`，播种和合并在 `host/maintenance.rs`，检查在 `goal_checks/self_improve.ts`；专用 behavior 已删。
- 整理发现的候选：类型已冻结（`MaintenanceCandidate`，`protocol/goal.rs:1020`），页面上有列表位置，发现、确认、落地都没实现。挂点是 `runner/consolidation.rs:854`（提交后发布候选）和问题的 `candidate_id`（`runner/goal.rs:1188`、`:1213`，现在恒为空）。

### 3.2 工具与手册

- `<agent_root>/tools` 被认作 Agent 自己的命令目录（[baseline.rs](../src/frame/lib_opendan/src/baseline.rs) `:562`），解析手册声明的命令时排在最前（`runtime/mod.rs:1066`）。**S0-2（看代码）**：私有 runtime 的会话，目录存在时就在 `PATH` 上（`runtime/mod.rs` 的 `open_session_env` / `session_layers`）；节点 runtime 上没有。作业的 `PATH` 由宿主自己加。
- 现在没有任何东西往 `tools/` 里产出。`skill_crystallize.toml:52` 明令禁止结晶 Goal 写 `tools/`、`cmd_skills/`、`plan_skills/`、`goal_skills/`，它只能 `agent-session skill revise`（来源 `local` / `imported` 的正文）和写 `skills_drafts/`。
- skill 包里出现 `tools/` 或非文本文件会被拒绝（AGENTS.md 的 cmd_skills 一节）。
- [llm_tool_carft.rs](../src/frame/agent_tool/src/llm_tool_carft.rs) 只有骨架：流水线一律落到最后一步，退出码 127。
- 手册的使用记录在 `.meta/skills_state.json` 的 `usage`（哪些 Work 带了哪份手册、哪些没带），结晶据此读 Work 的 worklog 和 report。没有“缺工具”的专门记录。

### 3.3 Activation、历史与成本

- 一次 Activation 是一个 Turn。上限在 `agent.toml:106-109`：30 万 token、6 个 run、1800 秒墙钟；`goal.toml:56` 每个 run 最多 24 轮工具调用。
- Goal 头每次由宿主从 Goal 定义和 Goal State 重建（`runner/goal.rs:798 opening`、`:557 render_builtin`），历史里旧的 Goal 头只留一行（`:913`）。`<last_activation>` 带上次报告的正文，截到 2000 字符（`:1043`）。
- 历史走通用逻辑：`agent.toml:11` 写的是到窗口的 80% 开始压缩、压回 30%。会话级有 `history_budget_tokens`（[protocol/config.rs](../src/frame/lib_opendan/src/protocol/config.rs) `:495`），没有验它设成 0 时的行为（没用上：`fresh` 是装配历史时以本次 Activation 的起点为界，`runner/history.rs`）。
- 想要全新上下文只能 restart 开新 epoch（`StateKeep::Facts`，`protocol/goal.rs:388`）。
- Goal Session 不能改自己的定义、预算、硬间隔、硬保障和 behavior，只能改检查脚本和时间表、增删当前 Skills（`goal.toml:50`）。
- Goal 头里和成本有关的只有当期计数（`<this_period>`，`runner/goal.rs:516`）和 `no_action_in_a_row`（`:515`）。每次 Activation 的记录带用量（`GoalActivation.cost`，`protocol/goal.rs:648`），Goal 没有现成的命令去汇总它。
- 成本数字（来自历次 DV）：`no_action` 2.5–7.8 万 token；结晶看一次 18.2 万；提醒 Goal 的首次 Activation 9.2 万；Goal 里跟长命令三次 37.8 万。
- **S0-1（2026-11-24）**：
  - 已有的证据（2026-11-11 DV 那个测试 Goal，38 次连续 `no_action`）：平均 57,023 token，首次约 2.5 万，每多一次约长 2,166，历史平均占六成。超过两成，所以做 CX2，缺省 `fresh`。
  - 真实模型回放（`tests/goal_replay.rs::cost_of_an_activation`，qwen3.7-plus）：系统提示词约 1.85 万字符，Goal 头 2.6–3.1 千字符，首轮输入约 6.9 千 token；`no_action` 的 Activation `fresh` 稳定在约 1.45 万 token，`carry` 1.6 万到 2.8 万、逐次增长。
  - 没取本机 Zone 上的内置整理 Goal：本机 Zone 当天是 `waterflier3`，没有 Agent。

### 3.4 Work、唤醒与失败

- Goal 建 Work 不等它。Work 结束是一次事件，和用户转来的消息、问题的回答一样要过硬间隔（缺省 900 秒）和预算（[goal.rs](../src/frame/lib_opendan/src/goal.rs) `:256 gate`、`runner/goal.rs:202 hold_events`）。手动、epoch 开始、宿主放行的定时检查和硬保障除外。
- Activation 关闭后不再跟它启动的后台任务（`runner/goal.rs:998`）。
- `explore`、`work_simple` 两个 behavior 都在；`plan_simple` 的 Todo 就是用它们跑的（`agent.toml:60-72`）。Goal 显式指定 behavior 或 `--class plan_simple` 宿主不拦，只是提示词写着 `--behavior work_simple`（`goal.toml:29`）。
- Goal State 里和失败有关的：`consecutive_no_action`、`last_error`、`recent_works` 的 `outcome`（`protocol/goal.rs:822`）。Activation 的结局有 `completed / failed / budget_exhausted / stopped / needs_input`（`:570`），没有“连续几次失败”的计数。检查脚本连续出错数在 `schedule.check_errors`（`host/goals.rs:476`）。
- 检查脚本的 `ctx` 有 `now_ms`、`held`、`timezone`、`agent_root`（[goal_check.rs](../src/frame/lib_opendan/src/goal_check.rs) `:415`）。`now_ms` 是实际运行的时刻，不是时间槽的时刻；多个槽合并成一次时也一样。
- `agent-session goal check run --script` 要求 Goal 已经存在（[xagent_cli.rs](../src/frame/lib_opendan/src/xagent_cli.rs) `:2927`）。
- 旧单 §7.3 第 3 条（Work 的任务结束事件出现在 Goal 的唤醒原因里）：**S0-4（2026-11-24）查清**，是 Goal 自己记住的任务编号在 Activation 关闭后还被跟着，2026-11-11 的 H0 已经修掉；RT2 补回归用例，没有改代码。

### 3.5 找人

- Goal 的 `questions` 每条变成一条可回答的通知，同一文本只发一次（`runner/goal.rs:1314`）；回答打开一次 Activation（原因 `answer`）。只有自由文本。Goal Session 里没有 `ask_user`。
- `normal` 级的 Goal 通知投到默认会话（[msg_center/notify.rs](../src/frame/msg_center/src/notify.rs) `:1145`）。问题关闭时会告诉卡片出现过的那些对话的 UI Session（`lib_opendan/notify.rs:415 conversations_of`）。
- `agent-session` 的子命令里没有“给某个人发消息”（`xagent_cli.rs:3427-3450`）。`owner` 手册的 `xcall owner notify / ask` 只到主人，`owner.contacts find` 只查人。
- HomeStation：只有宿主的自动分享（[share.rs](../src/frame/lib_opendan/src/share.rs)，用户发起的 Work 结束后发一条经验帖）。会话没有发帖和读回复的命令。HomeStation 服务端有评论（`comments.rs`）。

### 3.6 创建与管理

- 建 Goal 只有 owner 和 UI Session 能做（[state/goals.rs](../src/frame/lib_opendan/src/state/goals.rs) `:618`）。Work 一律拒绝，这是 WG1 要改的。
- 意图分析的结果有 `goal_created`（[analyze_intent.toml](../src/apps/jarvis_agent/agent/behaviors/analyze_intent.toml) 的 `outcome` 枚举，宿主校验在 `runner/reports.rs:763`）。提醒分支写着“create a goal that sends it”。它已被要求优先并入已有 Goal（“goals are scarce: fold it into one that fits”）。
- 带检查脚本且不带 `--start` 的 Goal，epoch 开始先跑检查，不推理（`host/goals.rs:886`）。
- 页面：Agent 主页（`src/frame/opendan/web`）有 Goals 列表和详情，活动 Goal 超过 16 个时提示（`pages/Goals.tsx:117`），没有创建入口，owner 只能改预算和时间表。Goal Session 在 `SessionDetail.tsx` 里以 worklog 表格显示。
- Desktop 里只有 Agent 面板的活动 Goal 个数（`app/messagehub/agentpanel/model.ts:11`）。现成可复用的：`components/conversation/worklog/`（`agentServiceClient`、`session.worklog`）、`SessionConversation.tsx`、`worksessions/`。
- 日历：`owner` 手册写 `xcall owner.cal add` 的 `reminders`（开始前多少分钟）会让主人按时收到通知，重复日程也算，“不要在别处再排同一个提醒”。**S0-3（2026-11-24，只看了代码）**：AI 工作区服务每 60 秒扫一遍（`frame/aiworkspace/server/src/notices.rs`），重复日程逐次展开，经 msg-center 发，不依赖 Agent 进程，晚到不超过 1 小时，全天日程从 09:00 算起。真实 Zone 和手机没有验。

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

- 先后顺序写进 `self_improve_tools`：已有的命令或已装的包能做到（只需要写进手册）；做一个快捷方式；包管理器安装；开源实现在本机构建；自己写。脚本用 Deno 或 Python。
- 快捷方式（Q13）：把已有的命令和接口包成一个短的、带约定参数的入口，省掉每次重新拼的 token。owner 系列工具（`xcall owner.cal add` 这类）是人做的先例。第一期产出的是命令形态，不产出对象别名。
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
| 要先弄清楚再动手，或几件事并行 | 派一个 `plan_simple`，由它排探索和执行的 Todo（它最多活 48 小时） |
| 例行的、不需要判断的 | 写成检查脚本或 Goal 作业（GJ），不推理 |

- 派活的任务书按 Plan 单的 `work_request_rule.inc` 写，下一次唤醒要用的值让 Work 放进报告的 `result`。
- 派活之后本次 Activation 就结束，Work 的结局在后面的唤醒里读报告。不改“Activation 关闭就放手”，不让 Work 的结束绕过硬间隔。

### 4.4 失败信号与求助（Q7–Q9）

- **Goal 头加两个计数**：`activations_failed_in_a_row`（结局不是 `completed`，或没有有效结果）和 `works_failed_in_a_row`（最近吸收的 Work 连续不成功的个数），放在 `<state>` 里，为 0 时不显示。
- **提示词的升级顺序**（`goal.toml` 和 `goal_upkeep`）：同一种做法失败两次就换做法；换过仍失败，向主人求助（`questions`，写清试过什么、卡在哪、要主人做什么）；属于目标本身要变的，请求升级（WG5，Q19）；主人明确不处理的，降低检查频率并在 `pending` 里留着，不反复提。
- **宿主的通知**（Q7）：连续 3 次时发一条，key 为 `goal:<id>:failing`，计数回到 0 之后才可能再发。不休眠、不改状态。
- **检查脚本连续出错**（Q8）：`check_errors >= 3` 时 `guarantee_due` 返回真，之后按硬保障的现有路径走，Goal 头里已经有 `<check_errors>`。
- **提问带选项**：`questions` 的元素可以是字符串，也可以是 `{text, options: [{id, label}], allow_text}`，映射到通知已有的选项输入。

### 4.5 Goal 怎么来（Q4 的答复；先做 WG1、WG2）

Goal 有两条来路，走哪条由意图分析按请求判断，不固定：

| 请求 | 做法 | 结果 |
| --- | --- | --- |
| 不需要先做出什么东西：盯着某件事、到点判断、条件提醒 | 意图分析直接 `goal create`，和现在一样。Goal 在第一次 Activation 里写自己的检查脚本 | `goal_created` |
| 要先做出一个东西，以后一直维护或持续改进：状态页、重排站、一套脚本 | 建一个普通的 Work（`work_simple` 或 `plan_simple`）。任务书按 `work_request_rule.inc` 写，在“做完的标准”和“报告”里写明：最后建立负责维护或持续改进的 Goal，在报告的 `result` 里给出 `goal_id` | `work_created` |
| 属于已有 Goal 的事 | 照旧转给它 | `forwarded` |
| 对已有 Goal 的小改动（“以后别提醒周末”） | UI 直接 `goal revise`，不开 Work（Q19） | 用现有的 `forwarded`，或新增 `goal_revised`，选改动小的（二期单 WG3） |

第二条路的几点：

- Work 是独立的普通 Work，没有专门的类型，也不归属于它要建的 Goal。它和别的 Work 的区别只在任务书里多了收尾这一步。
- 收尾怎么做写在 work skill `goal_handover` 里，意图分析只给任务书里有这一步的 Work：先读 `goal list`，该并入已有 Goal 的就改写它，不新建；写检查脚本并试跑；装上例行要跑的脚本（Goal 作业）；`goal create` 带检查脚本、不带 `--start`，所以 Goal 的第一次运行机会是一次检查，不推理；把维护说明作为 goal skill 交给 Goal。
- 成败是 Work 的结局，结果自己回到对话。失败时不留半成品：Goal 已经建出来的，Work 归档它并在报告里说明。
- `plan_simple` 时，按 Q17 的规则只有 Plan 自己有建 Goal 的权限，它的 Todo 没有，所以由 Plan 在收尾时建。这一句是整理者的推论，WG1 实施时核对 Plan 能不能跑这条命令。

需要补的一处：`goal check run --script <file>` 在没有 Goal 时也能跑，`ctx` 里没有上次的 `state`，时间表和 Workspace 由参数给。

### 4.6 提醒与日程（Q5、Q12）

- 意图分析的提醒分支只加一条判断，不加规则：主人需不需要在日程里看到这个提醒，由 Agent 自己判断。
  - 需要（会议、约会、截止日期这类本来就是日程的事）：写进日历，`xcall owner.cal add` 带 `reminders`，结果 `direct`。日历自己会提醒，不再另建 Goal，也不再往检查脚本里加。
  - 不需要（站起来活动、喝水这类习惯，或要看了情况才决定说不说的）：照旧由 Goal 的检查脚本到点发通知。已经有提醒 Goal 时并入它，不再新建（Q11）。
- 检查脚本的 `ctx` 加 `slot_ms`（这次检查对应的时间槽）和 `missed`（合并了几个槽）。`goal_check_script` 手册里按时刻发通知的例子改用 `slot_ms`，晚到窗口写成必须。
- 日程候选（第二步）沿用默认 Goal 单 T05：整理提交之后发布候选，一条候选一个带选项的问题，主人同意后由 self improve 调 `owner.cal` / `owner.todo` 落地；“长期职责”这类候选的落地是给已有的 Goal 增加一项职责（`goal post` 给那个 Goal，或请主人开 Work 改写），不新建 Goal。

### 4.7 让用户看得见（Q10、Q11）

- Desktop 的 Agent 面板：活动 Goal 的个数点开是列表（标题、状态、上次唤醒的结论、本周期用量、未回答的问题数）。
- Goal 会话视图：复用 `SessionConversation` 的外壳和 `worklog` 的数据源。每次 Activation 一组：唤醒原因、折叠的过程（工具调用）、结论（报告正文、决定、建了哪些 Work）。Goal 的提问和它发的通知按消息显示。只读加一个输入框。
- 输入框发的话调 `goals.post`，成为原因是 `feedback` 的输入。用户的话不过硬间隔，仍过预算和 Agent 总量；被预算拦下时视图里要说明什么时候恢复。
- “新建目标”不做表单：按钮打开和这个 Agent 的默认会话并预填一句话，走意图分析（§4.5）。
- 数量：意图分析和 `goal_handover` 的提示词加三条：固定提醒并入同一个提醒 Goal，改写它的目标后用 `goal post` 让它更新检查脚本；活动 Goal 达到 16 个时，新建之前必须说明为什么不能并入已有的；发现和已有 Goal 的目标矛盾时问主人，不自己选。并入的做法写进 `goal_handover`：改写目标 Goal 的定义和检查脚本，归档被并入的那个。

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

从二期单接手的两处，设计在二期单，不算本文新增：

6. Work 可以建立和改写 Goal（WG1，Q17）。
7. Goal 作业（GJ1–GJ6，Q18）；以及定时输入不堆积（GJ7）这一处修正。

### 4.10 从二期单接手的 WG、GJ

设计不重抄。下表是每条任务在本文里怎么做，以及留在二期单的部分。

| 二期单任务 | 在本文里 | 留在二期单的 |
| --- | --- | --- |
| WG1 权限 | 原样做（Q17） | 无 |
| WG2 命令行（`goal create / revise` 补字段、`--check-script`） | 原样做，和 FX3 同一处改 | 无 |
| WG3 意图分析 | 即 FX1，按二期单原样：先建设后维护的请求建 Work，任务书写明做完建立或改写哪个 Goal；纯观察和提醒仍直接建 Goal；小改动直接修订（Q4 的答复） | `site config set` 这类站点参数的小改动 |
| WG4 Goal 提示词（运维范围、先改工作区再安装、动手前看有没有主人的 Work） | 和 CX3、FH3 一起改 `goal.toml`，同一次提交 | 重排站的 goal skill `reading_site_ops`（MN2） |
| WG5 升级请求 | 只做通用的约定：升级说明文件、带 `--key --url` 的通知、主人在对话里答应后由意图分析建 Work（Q19） | `site upgrade open / close`、来源页状态、App 的升级界面（AP8） |
| WG6 文档 | 并入 DOC1 | 无 |
| WG7 非浏览器例子的 DV | 本文的 V10 | 无 |
| GJ1–GJ6 Goal 作业 | 原样做（Q18）；GJ4 改 `ctx` 时和 CA2 的 `slot_ms` 一起改，手册 `goal_check_script` 同一变更内更新 | 作业的具体命令 `site sync`、翻译的每日上限（ST4）、MN4 里其余手册 |
| GJ7 定时输入不堆积 | 即 RT1，放 M1 | 无 |

作业怎么到 Goal 手里不另设流程（Q18 的答复）：要先建设的，由 Work 按任务书在收尾时和 Goal 一起装上（§4.5）；运行中要加的，由 Goal 自己装。

## 5. 任务

### 5.0 S0：先量、先验

- [x] **S0-0 确认基线。** 入口条件：Plan 单已经合入。确认后重新核对 §3 的行号和说法（重点是 `goal.toml`、`analyze_intent.toml`、`agent.toml`、`runner/reports.rs`、`xagent_cli.rs`），把变了的改掉；读一遍 `work_request_rule.inc` 和改过的 `goal.toml`，确认 §4.3、§4.5 的写法和它不冲突。Plan 单没合入就不开始 M1 之后的任务。
- [x] **S0-1 量一次 Activation 的成本构成。** 取本机 Zone 上跑了两周的内置整理 Goal 和一个用户 Goal，从 Session 的 `usage.jsonl` 和 worklog 算：系统提示词加工具定义、Goal 头、历史、每轮工具往返各多少 token；`no_action` 平均几轮。结果写进 §3.3，据此定 Q3。
- [x] **S0-2 `tools/` 在不在 PATH 上。** 在 Agent 容器里往 `<agent_root>/tools` 放一个脚本，从 Work 的 shell 里直接按名字调。不在就记下要补的落点。
- [ ] **S0-3 日历提醒。**（看过代码，真实 Zone 没验） 真实 Zone 上建一条带 `reminders` 的重复日程：到点谁发、发到哪、Agent 容器停着时发不发、手机上有没有。结果写进 §3.6。它不再决定 CA1 做不做（Q5 已答）；手册里“重复日程也会提醒”的说法不成立时，记成 owner 工具的缺陷，CA1 的提示词里把这类提醒留给检查脚本。
- [x] **S0-4 Work 的任务事件。** 查旧单 §7.3 第 3 条的来源（`runner/children.rs`、`goal.rs:366 reason_of`）。
- [x] **S0-5 读 §1 的答复**，改 §4。先定的七条已落实（§1.3，2026-11-24）。Q6–Q16、Q19 以后有答复时再改。

### 5.1 SI：self improve

- [x] **SI1 播种。** `host/maintenance.rs`、`protocol/goal.rs` 的常量和 `OpendanGoalExtension`、`agent.toml` 的 `[session.goal.*]`；已有 Zone 的处理按 §4.1 末条。
  - 验证：`tests/goal_session.rs` 和 `memory_maintenance.rs` 里涉及结晶播种的用例改过来；升级后的 AgentRoot 上只有一个 self improve，没有加载失败的 Goal。
- [x] **SI2 goal skill 与检查脚本。** `self_improve_manuals`、`self_improve_tools`、`goal_checks/self_improve.ts`；删 `behaviors/skill_crystallize.toml` 和 `goal_checks/skill_crystallize.ts`。
  - 验证：`cargo test -p libopendan --lib skills`；检查脚本对三种材料（只有手册用量、只有 127、都没有）各跑一次；每个 `behaviors/*.toml` 仍能解析。
- [x] **SI3 新建 `local` 手册**（Q2）。`agent-session skill` 加一个子命令，限定 self improve 的 Goal Session 和它派的 Work；lint 同安装。
  - 验证：权限逐格测（别的 Goal、UI、普通 Work 都被拒）；新手册出现在 `skill list`，下一次意图分析能选到它。
- [x] **SI4 做工具的闭环。** 真实模型：造两个 Work 都因为缺同一个命令而绕路的材料，self improve 被检查唤醒，做出工具和手册，第三个同类 Work 用上了它。
- [x] **SI5 页面。** Skills 页的结晶开关改成两个职责开关；Goal 详情显示职责和各自最近一次的结论。
- [x] **SI6 第二步：并入 Memory 整理和日程候选**（Q1、Q12）。规则搬到 `self_improve_memory`，删 `behaviors/memory_maintenance.toml`；候选按 §4.6。
  - 验证：`memory_replay --ignored` 的真实模型回放不退化（和搬之前的结果逐条比）；`memory_maintenance.rs` 全过；整理回执页不变。
- [x] **SI7 删 `llm_tool_carft`**（Q14）。连同 `agent_tool/src/lib.rs`、`agent_tool_cli_dev/src/lib.rs` 里的 `command not found` 入口和文档里的说法。

### 5.2 CX：上下文与成本

- [x] **CX1 `goal stats`。** `xagent_cli.rs` 的 `cmd_goal`，kRPC 只读。验证：对一个有 50 条 Activation 记录的 fixture 核对每个数。
- [x] **CX2 `context: fresh | carry`**（Q3，S0-1 之后）。`GoalScheduleSpec`、`set_check` 那条路、`runner` 装配历史的地方、页面。
  - 验证：`fresh` 下第 N 次 Activation 的请求里没有第 N-1 次的工具往返；Activation 中途被杀后续完时本次的历史还在；切换不产生新修订。
- [x] **CX3 提示词与 `goal_upkeep`。** `goal.toml`：工作方式的表、指向 `goal_upkeep`、`--behavior work_simple` 改成缺省；`goal_skills/goal_upkeep/SKILL.md`。在 Plan 单改过的 `goal.toml` 上改，派活的写法沿用 `work_request_rule.inc`。`goal.toml` 这一批改动（CX3、FH3、WG4）放在同一次提交里。
  - 验证：真实模型上一个被故意设成每 5 分钟检查、每次都没事的 Goal，几次唤醒之内自己把检查收紧。
- [x] **CX4 关闭 F3。** 旧单和架构 §11.2、§11.3、§17.2 写明不做 Goal 专用压缩。

### 5.3 FH：失败与求助

- [x] **FH1 两个计数进 Goal State 和 Goal 头。** `protocol/goal.rs`、`runner/goal.rs:989 close_activation`、`render_builtin`。
- [x] **FH2 连续失败的通知**（Q7）。`host/goals.rs`，发送复用 `send_notify`。验证：第 3 次失败发一条；第 4 次不发；成功一次后再连续 3 次才再发。
- [x] **FH3 升级顺序。** `goal.toml`、`goal_upkeep`、`goal_delivery`。其中“请求升级”那一步的完整约定在 WG5（M2）补上，M1 先写成在 `questions` 里向主人提出。
- [x] **FH4 提问带选项。** `GoalResult` 的解析、`questions_to_user`（`runner/goal.rs:1314`）、Goal 头的 `<open_questions>`、Desktop 的通知卡片不用改（已有选项输入）。
- [x] **FH5 检查连续出错并入硬保障**（Q8）。`goal.rs:295 guarantee_due`。验证：出错 3 次后下一个槽唤醒一次，原因 `guarantee`，仍受硬间隔保护。

### 5.4 WG：Work 开发、Goal 运维（接手自二期单 §5.2，合并关系见 §4.10）

- [x] **WG1 权限**（Q17）。见二期单 WG1、§4.3。
- [x] **WG2 命令行。** 见二期单 WG2。
- [x] **WG3 意图分析。** 即 FX1。
- [x] **WG4 Goal 提示词。** 见二期单 WG4；和 CX3、FH3 同一次改。
- [x] **WG5 升级请求**（Q19）。只做通用约定：`goal.toml`、`analyze_intent.toml`、`ui_core`、`goal_delivery`。
- [x] **WG6 文档。** 并入 DOC1。
- [ ] **WG7 DV。** 见二期单 WG7，就是本文的 V10。

### 5.5 FX：Work 收尾时建立 Goal（在 WG1、WG2 之后）

- [x] **FX1 意图分析（即 WG3）。** `analyze_intent.toml`：加上 §4.5 的判断和第二条路的任务书写法；`goal_created` 和直接建 Goal 的写法不动。只改提示词，结果的 schema 和 `runner/reports.rs` 的校验不用改（小改动的结果选了新增 `goal_revised` 时除外）。
- [x] **FX2 work skill `goal_handover`。** Work 收尾时怎么建立或改写 Goal：指向 `goal_skills/goal_check_script`、`goal_delivery` 的正文路径，不复制；写明并入和查矛盾（§4.7）、装作业、失败不留半成品、报告的 `result` 里给 `goal_id`。重排站专有的交接说明仍在二期单的 MN1。
- [x] **FX3 `goal check run` 不要求已有 Goal。** `xagent_cli.rs:2925`、`goal_check.rs` 的 `run_for`。
- [x] **FX4 DV。**（用真实模型回放，没有在真实 Zone 上跑）真实模型四条：纯观察的请求仍直接建 Goal，没有多出 Work；先建设后维护的请求建了 Work，任务书里有收尾这一步，Work 结束时交出 Goal，Goal 由检查起步；Work 建到一半失败，对话里收到失败，没有残留的 Goal；应该并入已有 Goal 的请求没有新建。

### 5.6 CA：提醒与日程

- [x] **CA1 提醒进不进日历由 Agent 判断**（Q5 的答复）。`analyze_intent.toml` 的 Reminders 一节加 §4.6 的判断；`ui_core`、`owner` 手册里“不要在别处再排同一个提醒”的说法和它对齐；`goal_check_script` 的提醒一节不动。只改提示词和手册。
  - 验证：真实模型上各说一句会议类和习惯类的提醒，去向符合 V5；改完每个 `behaviors/*.toml` 仍能解析。
- [x] **CA2 `ctx.slot_ms` 与 `missed`。** `goal_check.rs:405`、`goal_check_runner.ts`、Agent State Protocol §9.4、手册例子。验证：宿主晚 3 分钟跑检查时，按 `slot_ms` 写的提醒仍然发出且只发一次。
- [x] **CA3 日程候选。** 并在 SI6 里做。

### 5.7 UX：Desktop

- [x] **UX1 Agent 面板的 Goal 列表。** `app/messagehub/agentpanel/`，数据 `goals.list`。
- [x] **UX2 Goal 会话视图。** 新组件放 `components/conversation/`，数据 `session.worklog` 加 `goals.activations`。
- [x] **UX3 新建目标入口。** 预填用 `hub.saveDraft`（二期单 §3.7 查过没有直接预填的接口）。
- [x] **UX4 用户的话不过硬间隔**（Q10）。`runner/goal.rs:202 hold_events`。验证：硬间隔内 `goal post` 立刻打开 Activation；预算用完时不打开，视图显示恢复时刻。
- [x] **UX5 数量与矛盾的提示词。** `analyze_intent.toml`、`goal_handover`：提醒并入同一个 Goal、16 个时先说明、矛盾时问主人（§4.7）。
- [x] **UX6 多语言与用例。** 10 种语言；`mock/backend.ts` 加 `goals.*`；手机视口的 e2e。

### 5.8 RT：运行中的问题

- [x] **RT1 定时输入不堆积。** 即二期单 GJ7：Goal Session 还有没消费的 `scheduled` 输入时，`host/goals.rs:989` 不再投新的，只累加 `missed`。验证：Work 占用工作区 3 小时，放行后只打开一次 Activation。
- [x] **RT2 Work 的任务事件不进 Goal 的原因。** S0-4 有结论后修。

### 5.9 GJ：Goal 作业（接手自二期单 §5.1，Q18）

- [x] **GJ1 存储与安装。** 见二期单 GJ1、§4.2。
- [x] **GJ2 运行器与调度。** 见二期单 GJ2。
- [x] **GJ3 命令、kRPC、权限。** 见二期单 GJ3；权限用 WG1 定下来的规则。
- [x] **GJ4 检查脚本与 Goal 头。** 见二期单 GJ4；和 CA2 改同一个 `ctx`，`goal_check_script` 手册同一变更内更新。
- [x] **GJ5 用量与预算。** 见二期单 GJ5。`goal stats`（CX1）把作业的用量单列一项。
- [x] **GJ6 Agent 主页的作业面板。** 见二期单 GJ6。Desktop 的 Goal 会话视图（UX2）在每次唤醒的那一组里显示相关作业的最近一次结果，不另做面板。

### 5.10 DOC

- [x] **DOC1 架构设计（含 WG6）。** §7.10（按 WG 的实现更新）、§8.3（补一句，Q19）、§9.1（改回一个 self improve，说明和“迁移为不同 Goal”的关系）、§9.3、§9.5、§11.2、§11.3、§12.1.1、§15.7、§17.2。
- [x] **DOC2 协议与手册。** Agent State Protocol §9（含 §9.5 权限表、作业）；`opendan_agent_dev`、`ui_core`、`skill_authoring`、`goal_check_script`、`goal_delivery`；AGENTS.md 的联动表（内置 Goal、goal skill、`goal stats`、Goal 作业）。
- [x] **DOC3 旧单。** 标注 F3、E1–E3、§7.3、§10.7 的去向。
- [x] **DOC4 二期单。** 标注 WG、GJ 已由本文完成并指向本文的实施记录；把这两组从它的 §5、§6 的里程碑和 §7 的 V10 里去掉；Q1、Q2、Q4、Q5、Q15、Q19、Q23 注明答复在本文的 Q16–Q19；它的 §3.3、§3.4 里被本文改掉的事实（权限、`goal_created`、`goal.toml` 的说法）更新。

## 6. 顺序

| 里程碑 | 内容 | 出口 |
| --- | --- | --- |
| M0 | 入口：Plan 单已合入。S0（含 S0-0）。先定的七条已答 | 基线核对完；成本构成有数，Q3 据此定下做不做 CX2；日历提醒验过 |
| M1 省钱、不丢 | CX1–CX4、FH1–FH5、WG4、CA2、RT1、RT2 | 一个空转的 Goal 会自己收紧；连续失败主人知道 |
| M2 先建后管 | WG1、WG2、FX（即 WG3）、WG5、CA1、UX5 | 要先建设的请求由 Work 在收尾时交出 Goal；提醒该进日历的进日历；升级请求能闭环 |
| M3 自我改进 | SI1–SI5、SI7 | self improve 做出第一个被 Work 用上的工具 |
| M4 看得见 | UX1–UX4、UX6 | 手机上能看一个 Goal 在做什么，并对它说话 |
| M5 并入 Memory | SI6、CA3 | 只剩一个内置 Goal |
| M6 例行工作与收口 | GJ1–GJ6、WG7、DOC1–DOC4、§7 验收 | V10 跑通；二期可以开工 |

- M0、M1、M2 按顺序做。M1 先做是因为后面每一组都会让 Goal 跑得更多，先把成本和失败管住。
- M2 完成后，M3（Rust：播种与 skills）、M4（Desktop）、M6 的 GJ（Rust：新模块）互不依赖，可以各派一个子 Agent 并行，Rust 子 Agent 各用自己的 `CARGO_TARGET_DIR`。串行做时按表里的顺序。
- M5 排在 M3 之后，等 self improve 跑稳了再把 Memory 整理搬进来。
- GJ 放最后：本文别的组都不依赖它，只有二期依赖。CX2 做不做要等 S0-1（Q3），不做时 M1 少这一条，别的不变。

**出口，也就是二期单的入口：**

- WG1–WG7、GJ1–GJ7 完成，V4、V10 在真实 Zone 上跑过。
- Agent State Protocol §9 已经包含权限表和作业，二期单 S0-4 不用再改这一份。
- 二期单已按 DOC4 回写。回写之后它的里程碑只剩重排站本身：M1 是 ST、TP、RH、AP1 / AP2 / AP6，M2 是 MN1 / MN3、AP3–AP5，M3 是 MN2、AP7 / AP8。

## 7. 验收

| # | 场景 | 通过条件 | 怎么验 |
| --- | --- | --- | --- |
| V1 | 一个没事可做的 Goal 跑三天 | 第三天的 token 明显少于第一天；它自己改过检查脚本或时间表；没有人提醒它 | 真实 Zone + 真实模型 |
| V2 | 一个一直失败的 Goal | 主人收到一条看得懂的求助或失败通知，不是每次都发；Goal 没有重复同一种做法超过两次 | 人为让它的 Work 必然失败 |
| V3 | 检查脚本被写坏 | 3 个槽之后 Goal 被唤醒并修好；固定时刻的通知最多漏这几个槽 | 集成测试 + 真实 Zone |
| V4 | 两种请求各说一句 | “帮我盯着某件事”：直接建了 Goal，没有多出 Work。“做一个某某，以后一直维护”：建了 Work，任务书里写着最后建立 Goal；Work 结束时对话里有结论和 Goal；Goal 起步是一次检查、没有推理；Work 失败时没有残留的 Goal | 真实 Zone + 真实模型 |
| V5 | 两种提醒各说一句 | “明天下午三点提醒我开评审会”：日历里有这一条并带提醒，没有新的 Goal，到点手机收到。“工作日每 50 分钟提醒我站起来”：没有进日历，并入了提醒 Goal 的检查脚本，到点发通知、不推理 | 真机 |
| V6 | 同一个缺口出现两次 | self improve 做出工具和手册并通知主人；第三次同类 Work 用了它，轮数减少 | 真实模型 |
| V7 | 手机上看一个 Goal | 能看到每次唤醒为什么醒、做了什么、结论是什么；发一句话，一分钟内有回应 | 真机 |
| V8 | 第 17 个 Goal | 意图分析先说明为什么不能并入，或直接并入了已有的 | 真实模型 |
| V9 | Memory 整理并入之后 | 回放结果不退化；只有一个内置 Goal；整理回执照常 | `memory_replay --ignored` |
| V10 | 和浏览器无关的“先建后管”（二期单 WG7） | 对 Agent 说“做一个每小时检查我的几个网站是否可用的状态页，以后一直维护”：Work 建成并交出 Goal 和作业；例行检查不推理；人为制造故障后 Goal 被唤醒修好；改需求时主人收到升级请求，同意后由 Work 升级 | 真实 Zone + 真实模型 |

## 8. 验证命令与 DV 注意

- Rust（`src/`）：`cargo test -p libopendan -- --test-threads=1`（`plan_simple_async`、`work_simple_async` 共用进程环境变量，并行会假失败）；`--test goal_session`、`--test memory_maintenance`、`--lib skills`、`--test xagent_cli skills_are`；`cargo test -p opendan`。协议 schema 文档用 `target/debug/xagent schema <dir>` 重新生成。
- 真实模型回放：`memory_replay --ignored`（SI6 前后各跑一次存档）。
- Desktop（`src/frame/desktop`）：`pnpm check`、`pnpm lint`、相关 e2e、`desktop-i18n.spec.ts`。Agent 主页（`src/frame/opendan/web`）：`pnpm check`、`tests/e2e/goals.spec.ts`。
- 部署：改 behavior、goal skill、检查脚本要重新打 Jarvis 包；容器内的 Rust 改动要 `./build_aios --local-test` 再 `uv run src/start.py --skip-update`。
- 真实 Zone 的 Goal DV 做法见旧单 §10 和它的证据目录。**测试 Goal 由脚本在拿到结果后自动归档**，不要留着过夜（2026-11-11 一个短硬保障的测试 Goal 烧了约 220 万 token）。
- 手册的描述会影响意图分流，改完先用便宜的消息试分流。
- 开工前用 `git status` 确认 Plan 单的改动已经提交。它没提交时动 `goal.toml`、`analyze_intent.toml`、`runner/reports.rs` 会和另一个会话互相覆盖。
- WG、GJ 的验证命令和 DV 注意见二期单 §8 里和这两组有关的部分。

## 9. 已知风险

- 本文接手 WG、GJ 之后范围变大，多了一个子系统（Goal 作业，Q18 已定在本文做）。它排在最后，前面的里程碑不依赖它。
- 会话身份自报（Q17）：Agent 绕过 Goal 的权限在技术上可行，靠提示词约束。
- 作业以 Agent 身份运行、没有沙箱（Q18）：程序由 Agent 写，被诱导写出恶意程序的风险和它平时写代码相同。
- 直接建的 Goal 第一次 Activation 失败时，用户不会马上知道（Q4 保留了直接建）：对话里已经收到 `goal_created`，要等连续 3 次失败的通知（FH2）。第一次 Activation 的成本也照旧，提醒 Goal 实测 9.2 万 token。
- 走哪条路、提醒进不进日历都交给 Agent 判断（Q4、Q5），判断错了没有机制兜底，靠 V4、V5 的真实模型 DV 看分流准不准。

- Agent 自己做的工具不经主人确认就给 Work 用（Q2）：工具由 Agent 写，被诱导写出有害命令的风险和它平时写代码相同；事后通知，主人可以禁用。
- `fresh` 上下文会丢掉 Goal 没写进报告的东西。Goal 头和报告的质量决定了连续性，要在 V1 里看有没有反复重查同一件事。
- self improve 缺省打开后每天最多 60 万 token（Q15）。门槛写松了会空转，V1 的方法同样适用于它。
- 用户的话不过硬间隔（Q10）：一直说话可以连续唤醒，由周期预算封顶。
- Memory 整理的规则从系统提示词搬到按需读的 goal skill 后，模型可能不读就动手。SI6 的回放是唯一的保证，不过就不并。
- 定时仍只在 Agent 进程在线时触发（旧单 D3 未做）。

## 10. 不在本文

- **求助主人以外的人。** 要先定：以谁的名义发、能联系谁、对方的回复怎么回到 Goal（现在只会进 Agent 和那个人的 UI Session）。最小的做法是加一条发消息的命令并由 UI 转 `goal post`，等授权规则定了再做。
- **在 HomeStation 上求助其他 Agent。** 要先有读的一方：现在没有 Agent 会去看别人的求助帖。最小的做法是会话可用的发帖命令加检查脚本读评论，等有第二个会读的 Agent 再做。
- **self improve 产出对象别名形态的快捷方式**（Q13）。要先给 Agent 开一个自己的别名和本地动作脚本的登记来源，现在只有管理员安装的扩展包和用户覆盖两种。
- **新建 Goal 一律先过 Work、固定提醒一律写日历。** Q4、Q5 没有采纳，不要再加回来。
- **在一次唤醒里等子会话跑完**（模式 2 的严格形态）。
- **Goal 数量的硬上限、合并命令、所有 Goal 合成一个。**
- **Goal 专用的压缩策略。**
- 个人重排站本身：二期单的 ST、TP、RH、AP、MN、RB、BR、EV、CL 各组，以及 WG、GJ 里重排站专有的部分（§4.10 右列）。
- workflow 计划后端（旧单 D3）、XLLM 结晶、子 Work 的预算预留、跨节点运行。

## 11. 实施记录

代码全部在 2026-11-24 写完，**没有提交、没有部署**。真实模型样例用本机 Zone 的 AICC（当天所有别名都路由到 `qwen3.7-plus`，比以前回放用的模型弱），会话、宿主、检查、作业在测试进程里跑，不碰 Zone 的数据。**真实 Zone 上的验收（V1、V5、V7、V10 的 Zone 部分）没有跑**：本机 Zone 当天是 owner 的 `waterflier3`，没有 Agent，也没有 aios 镜像。

| 日期 | 任务 | 记录 |
| --- | --- | --- |
| 11-24 | S0 | 见 §3 各节的标注。S0-1：历史占六成，做 CX2。S0-2、S0-4 看代码定。S0-3 只看了代码。 |
| 11-24 | CX1 | `goal_stats.rs`、kRPC `goals.stats`（读）、`agent-session goal stats`。每周期：Activation 数、决定 / 结果 / 原因分布、token（自己 / Work / 作业）、检查与拦截；闸门计数保留之前 14 个周期（`schedule.json.past_stats`）。用例 `goal_stats_add_up`（50 条记录逐数核对）。 |
| 11-24 | CX2 | `schedule.context`，缺省 `fresh`；改它走 `set_check`，不加修订。`runner/history.rs` 以本次 Activation 的起点为界。用例：`fresh` 看不到上次的工具往返；中途被杀后本次历史还在；切换不产生修订。回放：`no_action` 稳定在约 1.45 万 token（`carry` 1.6–2.8 万且增长）。 |
| 11-24 | CX3、FH3、WG4 | `goal.toml` 重写：唤醒与成本、运维与升级的分界、工作方式的表、失败的升级顺序、作业；新 `goal_skills/goal_upkeep`。回放 `an_idle_goal_tightens_its_check`、`a_broken_check_is_repaired`、`failing_work_ends_in_a_question` 通过。 |
| 11-24 | CX4 | 旧单 F3 关闭；架构 §11.2、§11.3、§17.2 写明不做 Goal 专用压缩。 |
| 11-24 | FH1、FH2 | `activations_failed_in_a_row`（failed、budget_exhausted、没有有效结果；stopped 不计）、`works_failed_in_a_row`、`failing_since_ms`；连续 3 次通知一次，key `goal:<id>:failing:<since_ms>`，一段失败一次。 |
| 11-24 | FH4 | `questions` 可带 `options`（≤8）与 `allow_text`，映射到通知的选项。回放 `a_question_offers_choices_and_its_answer_is_followed` 通过。 |
| 11-24 | FH5 | 检查连续出错 3 次按硬保障唤醒一次（`guarantee_posted_for_errors`）。 |
| 11-24 | WG1、WG2 | `state/goals.rs::CallerKind`：owner、UI（对端是 owner 的可改任何非内置 Goal）、Goal、Work（UI 会话直接创建、不属于某个 Goal 的等同父 UI）。`created_by.work`。`goal create / revise` 命令行字段补齐，创建时可装检查。修订里新增的 `initial_skills` 在下次 Activation 并入当前 Skills。另修：`goal` 派 `--class plan_simple` 原来会被悄悄换成 `work_simple`。 |
| 11-24 | WG3 = FX1、CA1、UX5 | `analyze_intent.toml`：先做再管 / 直接建 Goal / 并入 / 升级 / 小改动（结果用 `forwarded`，没有新 outcome）、提醒进不进日历、16 个、矛盾。回放 `watching_something…`、`made_then…`、`a_habit…`、`an_appointment…`、`a_small_change…`、`an_agreed_upgrade…`、`a_seventeenth…` 通过；16 个那条第一次把它读成了硬上限，改了写法后通过。 |
| 11-24 | WG5 | 升级请求：`goal_delivery` 一节，要点也写进 `goal.toml`（模型不去读 skill 时只发通知、不写文件）。 |
| 11-24 | FX2、FX3 | work skill `goal_handover`；没有 Goal 也能 `goal check run`（`--every / --cron / --workspace / --path`）。 |
| 11-24 | CA2 | `ctx.slot_ms`、`missed`；`goal_check_script` 的例子按槽写，晚到窗口写成必须。 |
| 11-24 | RT1 | `schedule.json.waiting`：前一次没被消费不再投，槽并进 `merged`。 |
| 11-24 | RT2 | 不用改：H0 已修，补用例 `a_task_the_goal_only_reads_about_is_not_followed`。 |
| 11-24 | SI1–SI3、SI5、SI7 | 内置 Goal `self_improve`（`host/maintenance.rs`），goal skill 三份加 `goal_upkeep`，检查 `goal_checks/self_improve.ts`，`agent-session skill add <name> --file --reason`（只给 self_improve 的 Goal Session 和它派的 Work），职责开关 `skills.self_improve_set`，Skills 页与 Goal 详情；`llm_tool_carft` 与 `__command_not_found__` 删除。 |
| 11-24 | SI6、CA3 | 整理并入，内置 Goal 只剩一个；专用 behavior 都删。已有的 Zone：保留整理 Goal 的对象（带 receipt），改写成 `self_improve` 并 restart，检查脚本一律换成包里的（旧脚本留在 `checks/`），结晶 Goal 归档；owner 改过的 token 预算和间隔保留。候选用带选项的 `questions`。真实模型回放的对比见下。 |
| 11-24 | UX1–UX4、UX6 | Desktop Agent 面板：目标列表、目标会话视图、新建目标；用户的话不过硬间隔。10 种语言。e2e 15、datamodel 13 通过（mock）。真机和真实服务没有验。 |
| 11-24 | GJ1–GJ6 | `goal_jobs.rs`、`goals.job_*`、`agent-session goal job …`、`<jobs>`、`ctx.jobs`、`xllm` 的用量与预算、Agent 主页的作业面板。用例 `tests/goal_jobs.rs` 12 个（含宿主死掉后记 `killed` 并补跑、休眠结束在跑的作业、预算用完模型调用被拒）。 |
| 11-24 | DOC1–DOC4、WG6 | 架构（§7.9、§7.10、§8.3、§9.1、§9.3、§9.5、§11.2、§11.3、§12.1.1、§15.3、§15.7、§17.2）、Agent State Protocol §9（含 §9.5 权限表、§9.7–§9.9）与 schema、开发指南、两个 README、AGENTS.md 联动表、`opendan_agent_dev`；旧单、二期单标注。 |

**SI6 的回放对比**（`memory_replay --ignored replay_with_the_real_model`，14 个样例，各跑一次）：

| | 机械检查 | token 合计 | 说明 |
| --- | --- | --- | --- |
| 搬之前（专用 behavior，规则在系统提示词里） | 13/14，`third_party_advice` 没有处置材料 | 68.7 万 | 每个样例约 3 轮 |
| 搬之后，规则原样进 skill | 11/14（用英文写结论、多建一条认知、提交格式被拒多次） | 114.7 万 | 退化，没有采用 |
| 搬之后，skill 加一份完整的提交样例和“提交前再读一遍”的清单 | 14/14 | 116.1 万 | 采用。逐条读过：认知、依据、范围、语言和搬之前同级或更好（`s1` 的依据从 inference 变成 tool_observation，`s4` 的常住地不再是三元组写法） |

token 约为 1.7 倍：每次要先 `cat` 一遍约 2.2 万字符的 skill（`fresh` 之下每次 Activation 一遍），多 1–2 轮。`s5_exception` 有一次去翻了原因没点名的职责（19.6 万 token），随后在目标和 skill 里写明只做原因点名的职责，没有再量全套。

**SI4、V10 的回放**：

- SI4 / V6（`a_missing_command_becomes_a_tool`）：两个真实 Work 先跑 `ledgersum` 失败再各自写脚本 → 检查点名 → Goal 派一个 Work 写出命令和手册（`skill add`）→ 第二次 Activation 试一次、记进 skills、通知主人 → 第三个请求经意图分析选中这份手册，Work 用上了它。token 18.9 万 + 9.5 万。三个 Work 的工具调用数 15、8、9，轮数没有明显减少（样例太小）。前四次没过，暴露并改掉的：`which` 探测不算 command not found；Goal 自己写程序会用完单次预算（改成由 Work 写，单次预算 30 万改 50 万）；意图分析不先看 `skill list` 就建 Work（`work_request_rule.inc` 写明先看）。
- V10 的前半（`a_job_runs_what_was_built_and_its_failure_wakes_the_goal`）：Work 写程序、装作业、建 Goal；三小时例行没有推理；改掉输入文件后作业连续失败两次，检查叫醒 Goal，Goal 修好并重新安装作业（版本 2）。前几次没过，暴露并改掉的：修了工作区的源但没重新安装作业；把整个工作区当程序目录装上去（读的是装进去的旧数据）。另加单测 `a_job_that_keeps_failing_wakes_its_goal`。V10 的后半（请求升级、同意后由 Work 升级）分别由 `more_than_operation_is_asked_for`、`an_agreed_upgrade_becomes_work` 覆盖，没有连成一条。

**测试。** `cargo test -p libopendan -- --test-threads=1`：649 过，6 个失败和 HEAD 上一样（`fixtures` 的 f04 / f06 / f07 / f12 / f15“Session tool paths changed”，`memory_replay a_jarvis_conversation_is_told_what_to_perceive`）。`-p opendan`：过；`browser::screen_tests::what_pages_show…` 在整套里偶发失败一次，单独跑过。`-p agent_tool -p agent_tool_cli_dev`：只有本机没装 `rg` 的 4 个 grep 用例失败，和以前一样。Agent 主页：check、lint、build、e2e 69/69。Desktop：tsc、改动文件 eslint、新 e2e 15/15、datamodel 13；`pnpm lint` 另有 5 个不在改动文件里的错误，4 个 messagehub 用例在 HEAD 上同样失败。

**真实模型还没稳的。** 弱模型上同一条样例不是每次都过，都过了至少一次：`failed_work_leaves_no_goal` 有一次模型自己造了一个同名命令冒充（`goal_handover` 补了“替身不算做成”之后通过）；提示词改动后没有把每一条都重跑多次。`plan_replay a_goal_gives_everything…` 在这个模型上因措辞不过（改动前后一样）。

**没有验的。**

- V1（跑三天）、V5（手机收到日历提醒）、V7（真机）、V10 的真实 Zone、S0-3 的真实 Zone、SI6 在已有 Zone 上的合并（只有集成测试）。
- `skill add`、作业、职责开关在 Agent 容器里的真实运行；作业里的 `xllm` 用量计入预算（单测加集成测试，没有在容器里跑）。
- 页面只跑了 mock；非中英文的翻译没有母语者看过。

**和本文设计不同的地方（请 owner 过目）。**

1. §4.6 写“候选沿用默认 Goal 单 T05”。没有实现宿主的类型化候选（`MaintenanceCandidate` 仍没有写者，页面位置保留），用带选项的 `questions` 加 skill 里的规则（每个发现一个问题、先查现有日程 / 待办 / Goal、每次至多两个新问题）。理由是 §4.9 没有列这处机制。去重靠问题文本，没有稳定 key 和批准版本。
2. 二期单 GJ3 写“只读 Agent 不能写”作业。实现和 `set_check`、`sync.put` 一致：只读 Agent 可以改自己的 Goal 的作业，不能改别的 Agent 的。作业以它自己的身份运行，权限没有变大。要改成完全不能写只需在 `opendan/service.rs` 拦 `goals.job_*`。
3. 作业的用量文件每个 Goal 一份（`jobs/usage.jsonl`），不是每个作业一份；命令不经过 shell。
4. self_improve 的预算：Q15 的结晶加做工具部分是每天 60 万，并入整理后合成 24 小时 180 万、单次 50 万 / 6 个 run、48 次，Work 数不限（单次 30 万时，弱模型自己写工具会在通知和报告之前用完）；每 30 分钟检查（手册和工具的材料每 6 小时收集一次）。
5. 有问题的检查项：Agent 用 `which` 探测命令时不会留下 command not found，检查脚本补了这一类（任务文本里点名的命令才算）。
6. 报告的 `result` 写成 JSON 文本时按对象解析（`reports.rs::plain_result`），否则弱模型的每次 Activation 都算失败。
7. 回放暴露的一次事故：样例里的模型在测试机上装了一条用户 crontab（当时是唯一一行），已用 `crontab -r` 删掉，现在没有 crontab；回放环境给 `crontab / at / sudo / systemctl / docker / ssh …` 放了只报错的桩（`tests/common/replay.rs`）。
