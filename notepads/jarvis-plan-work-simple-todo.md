# Jarvis 长期计划与短任务执行 TODO

D1–D12 已裁决，落实方式已确认 · 2026-11-24（原稿 2026-11-18）· 对照实现 `1704812f8` · 未实施 · 1 项新增待你过目

目标是让 `plan_simple` 持续管理可跨 12 小时、可变更、可等待的目标，让 `work_simple` 作为干活的主体，独立完成一个已准备好的任务。任务可以是 Plan 的 Todo，也可以由会话或 Goal 直接交办。30 分钟是 Plan 拆单和回头看的参考，不是 Work 的时限。

各条目已按你的答复改写，原话和落实方式在“裁决记录”一节。现在没有待你回答的问题。“work_simple 与 explore”一节的第一项是按你对 `work_simple` 的定位新加的，请过目。

贯穿这一轮的原则是你在 D6、D9 里说的：尽力不要有硬机制，机制越少越好。落实时优先用已有的事件和提示词，“不做”一节列出了因此放弃的做法。

## 裁决记录

落实里有 6 处是我按理解写的：D3 的四处、D2 的墙钟、D9 的弱提醒和清单快照、D8 的不重复记，以及确定性失败的判定和用户打字回答时走 `notify answer`。第二轮你确认了：

> 待确认的都按你的想法来就好

这 6 处已并入下面各项的落实和后面的条目。

**D1 运行中的 Todo 还能不能改任务。**

> 按你的建议

落实：不冻结。`todo update` 原地改和 `todo stop` 后 `todo retry` 两条路都留，由 Plan 按变更大小选：细化或补充就原地改，交付目标变了就停掉重派，选法写进 Plan 的提示词。Todo 更新的送达提前到 Worker 的下一次推理前，和 inbox 在同一处实现。原稿建议的冻结不采用。

**D2 30 分钟是拆单目标还是硬上限。**

> 30分钟是给plan的建议，如果一个worksession 30分钟没有返回，那么plan就要考虑是否要做调整了。我想大部分情况下，除非是work卡死了，plan还是愿意等待的

落实：30 分钟只写在 Plan 的提示词里，是拆单的参考和回头看的时机。宿主不给 Work 加时限，11-02 的 Q7 不动，`work_simple` 的连续运行墙钟也保持缺省的 60 分钟。Plan 的 timer 第一次在 30 分钟时到，事件里列出每个在跑的 Todo 已经跑了多久和最近一句进度，由 Plan 判断是在推进还是卡死。在推进就继续等，卡死才停掉或重派。

**D3 Plan 怎样等外部条件。**

> Plan主要等的就是标准event + usermsg,event里除了task/todo的状态，也有timer. timer就是防止plan完全失去了推理机会引入的。  plan 运行了太久要不要失败也是Plan根据自己的理解决定，系统设置wall-clock也会非常大（比如48小时）

落实：不加等待登记，不加检查脚本。Plan 等的是 Todo 和 task 的状态、timer、用户消息。等外部条件时，Plan 说明在等什么然后等待，timer 每次给它一次推理机会，它用只读命令看条件是否满足，用户也可以直接告诉它。等了太久要不要失败由 Plan 自己判断。下面四处是我按理解写的，第二轮已确认：

- Plan 名下什么都没在跑时也可以等。提醒保留一次，之后的回复算等待，宿主不再以 `missing_end_report` 结束 Plan。
- timer 就是现有的 tick，改成退避：30 分钟起，间隔翻倍，封顶 4 小时，任何别的输入让间隔回到 30 分钟。名下什么都没在跑时也照常给。
- Plan 加寿命上限，从创建起算，缺省 48 小时，到点进收尾模式让 Plan 交终报。这是 11-02 的 Q7 当时没选的选项，现在只给 Plan 加。
- 重启按 D27、D28 不变：等待中的 Plan 会被 timer 带起推理，算正在工作，重启后停在异常中断，手工恢复后接着等。

**D4 等待期间还要不要 tick。**

> 按你的建议

落实：Plan 不再每 600 秒固定 tick。它的 timer 改成 D3 里的退避节奏，第一次在 30 分钟，也就是上一版说的“超过 30 分钟提醒一次、之后间隔翻倍”。Todo 的工作会话等命令时的 tick 从 30 秒起翻倍，封顶 10 分钟。

**D5 Plan 的历史怎样只留决策。**

> 实际上plan的历史记录里都是各种TODO的修改记录，很少触发context limit,所以按正常的处理压缩就好。反正session todolist和worksession的细节都是可以通过read去读取的

落实：不做。Plan 的历史照常重建和压缩，不加专用规则，也不限制 Plan 的工具。原稿的这一条目删除。

**D6 必要的独立验收靠提示词还是靠机制。**

> 按建议，我们的原则就是尽力不要有“硬机制”，影响扩展

落实：只改提示词。需要独立验收、结果又要被后继消费时，Plan 把验证 Todo 排在中间，消费者依赖验证 Todo。不加“等 Plan 接受才放行后继”的标记。

**D7 Plan 什么时候可以问用户。**

> 按你的建议

落实：放宽。答案会改变拆解或验收，而目标、workspace、报告和分析阶段的问答里都没有，Plan 就问并等待。依赖这个答案的 Todo 先不派，不依赖的照常跑。`work_simple` 不变。

**D8 计划里值得复用的事实由谁写入 Memory。**

> Plan写，worksession不写。其实从结果上，一个预期外的失败偶蛮适合plan在分析错误原因后把结论写入memory的（这通常是和workspace相关的memory)

落实：留在本单。Plan 用 `ctl perceive` 记下值得以后复用的结论，典型的是预期外的失败在分析出原因之后的结论。Plan 绑定了 workspace 时，宿主已经把这个 workspace 记成这条感知的主体，不需要 Plan 另外标。Todo 的工作会话不写，值得记的写进报告，由 Plan 判断。Plan 在终报里列出它记下了什么，UI 转述时不再记同样的内容。会话直接发起的 `work_simple` 没有 Plan，仍由 UI 在转述时记。上一版建议的移出本单不采用。

**D9 Todo 没写验收标准时宿主管不管。**

> 可选（弱提醒），还是之前的原则，机制越少越好。plan skill的设计说明不同领域的事务，验证的时机是不同的。 另外，worksession除了看到自己的TODO,还应该看到整个TODOLIST(能看到后面有安排的验证任务)，免得自己花太多token深度验证。

落实：验收标准可选。Plan 的提示词不再要求每个 work Todo 都带，改成按领域 plan skill 说的时机安排验证。弱提醒做成 `todo add` 输出里的一行，不拒绝，不影响派工。新增一项：Worker 的首条输入带上整张 TodoList，它能看到后面排了验证 Todo，自己只做验收标准要求的检查。清单是派工那一刻的快照，Todo 被更新时再给一次，之后的变化不主动推给 Worker。

**D10 短 Work 用尽上下文时怎样结束。**

> 按你的建议，毕竟把window用完说明已经花了很多token了，丢弃了挺浪费的

落实：压缩保留，Work 压缩后继续做。只改压缩三次仍放不下时的结局：Todo 的 attempt 以失败结束，原因标 `context_limit`，带上最后一次进度报告。它的会话和报告仍可读，Plan 重派时把引用交给新的 attempt，已做的部分不用从头再来。

**D11 explore 的只读做到哪一层。**

> 按建议

落实：这一期只修准入，写回靠提示词，记为已知风险。按会话收窄执行器写操作的硬边界不做。

**D12 调查方法放在哪里。**

> 按建议

落实：沿用。调查问题和证据标准由领域 plan skill 给，命令由现有 cmd skills 给，不单设 explore skill。

## 已修复

回归测试覆盖了一条只有一个前置项的依赖链。前置 Todo 的终报为 `failed` 或 `blocked` 时，后继启动 0 次。前置报告 `succeeded` 时，后继启动 1 次。已有的依赖调度、并行 explore 和领域 plan skill 继续作为下列事项的基础。[调度回归测试](../src/frame/lib_opendan/tests/plan_simple_async.rs) · [现有 plan skills](../src/apps/jarvis_agent/agent/plan_skills)

## plan_simple

- [ ] **需求澄清在原会话里完成。**（D7）

  **现状**　分析阶段在建 Plan 之前已经用 `ask_user` 问，最多 5 问，问答由分析写进 Plan 的首条消息。Plan 的 `ask_user` 发通知，关联最近的 UI 祖先，投到发起这项工作的会话。UI 可以代答，TaskMgr 把答案作为 task 结果送回提问的 Plan。普通会话消息另由 UI 用 `post` 转发给 Plan。Plan 现在只在领域 skill 要求访谈、或者后期只差一个决定时提问，其余缺前提时优先终报。

  **做法**　Plan 的提示词放宽提问条件：答案会改变拆解或验收、别处又查不到时就问并等待，依赖这个答案的 Todo 先不派。`chat_route` 写明：用户是在回答某张通知卡的问题时用 `notify answer`，其余关于这项工作的话才 `post`。

  **验收场景**　需求明确时直接建单。缺关键前提时，用户在原会话看到问题并回答，提问的 Plan 只收到一次答案，此前不派依赖该答案的 Todo，不依赖的照常跑。用户不点卡片、直接打字回答时，答案也只送达一次。普通需求变更仍走转发。

  **验证**　通知路由和代答有单测。提问时机、UI 选哪条路要真实模型 DV。[Plan 行为](../src/apps/jarvis_agent/agent/behaviors/plan_simple.toml) · [分析阶段的提问](../src/apps/jarvis_agent/agent/behaviors/analyze_intent.toml) · [会话转发规则](../src/apps/jarvis_agent/agent/behaviors/chat_route.toml) · [通知路由](../src/frame/lib_opendan/src/notify.rs) · [UI 代答测试](../src/frame/lib_opendan/tests/notify.rs)

- [ ] **让 Plan 能长时间等待。**（D2、D3、D4）

  **现状**　Plan 有持久输入队列，能等 Todo 的工作会话、task 和 `ask_user` 的回答。名下什么都没在跑时，纯文本回复会收到一次提醒，再次空回复以 `missing_end_report` 结束。tick 固定 600 秒一次，只在它跟着工作时才有，每次是一次推理，等一个 Todo 12 小时是 72 次。会话没有寿命上限。

  **做法**
  - timer 退避。Plan 在等待、30 分钟没有新输入时给一条 timer 事件，之后间隔翻倍，封顶 4 小时。任何别的输入让间隔回到 30 分钟。名下什么都没在跑时也照常给。
  - timer 事件列出每个在跑的 Todo 已经跑了多久、最近一句进度和它的时间。
  - 名下什么都没在跑时的纯文本回复：提醒保留一次，文案改成“目标达成或做不下去就终报，在等外部的东西就说明在等什么”。之后的回复算等待，不再结束 Plan。
  - Plan 的提示词：按约 30 分钟拆单。timer 到来时，在推进的 Todo 继续等，卡死的才停掉或重派。等外部条件时每次 timer 用只读命令查一次。等了太久要不要失败自己判断，在终报里说明。
  - `plan_simple` 的寿命上限缺省 48 小时，从创建起算，到点进收尾模式，只剩 `report`。还在跑的 Todo 和现在一样随 Plan 结束而停止。
  - 重启按 D27 不变。

  **验收场景**　用模拟时间。让没有运行中 Todo 的 Plan 等一份次日才交付的文件：24 小时内 timer 带来的推理不超过 8 次。文件到达后，下一次 timer 或用户一句话让计划推进一次，不重复派工。等待期间用户仍能修改或取消计划。期间重启宿主，Plan 停在异常中断，手工恢复后接着等。一个 Todo 跑满 3 小时，Plan 在第 30 分钟和第 1.5 小时各得到一次机会，两次都能看到它跑了多久和最近的进度。Plan 到 48 小时进收尾并终报。

  **验证**　宿主单测。Plan 能不能分清在推进和卡死，要真实模型 DV。[等待与 timer](../src/frame/lib_opendan/src/runner/ticks.rs) · [结束判定](../src/frame/lib_opendan/src/runner/outcome.rs) · [重启后的中断判定](../src/frame/lib_opendan/src/runner/drive.rs) · [收尾模式](../src/frame/lib_opendan/src/runner/budget.rs) · [会话类配置](../src/apps/jarvis_agent/agent/agent.toml)

- [ ] **必要的独立验收排在消费结果之前。**（D6、D9）

  **现状**　Worker 报 `succeeded` 后，调度器在 Plan 下一次推理前启动后继。提示词第 4 步已要求预知需要独立验收时随清单加验证 Todo，没有说消费结果的 Todo 要依赖它。Plan 事后补的验证 Todo 拦不住已经启动的后继，只能 `todo stop`。提示词第 2 步要求每个 work Todo 都带验收标准。

  **做法**　只改提示词。第 4 步补一句：需要独立验收的结果，消费它的 Todo 依赖验证 Todo。第 2 步改成：验证排在什么时候按领域 plan skill 的说法，验收标准可选。11-10 起验证 Todo 只在需要时才加，这一点不变。

  **验收场景**　给一个含发布步骤的目标，Plan 建出的清单里发布依赖验收，产出方报成功而验收失败时，发布启动 0 次。只需要自证的 Todo 不多出验证 Todo。Plan 宣告目标完成时能逐项指出关键验收证据，并说明取消或未完成的事项。

  **验证**　依赖阻断已有单测。清单排得对不对要真实模型 DV，至少两类目标：改代码后发布、扫描或查询。[Plan 验收规则](../src/apps/jarvis_agent/agent/behaviors/plan_simple.toml) · [调度时机](../src/frame/lib_opendan/src/runner/todos.rs)

- [ ] **Plan 把值得复用的结论写入 Memory。**（D8）

  **现状**　结束会话不会自动写记忆。UI 在转述工作成果时用 `ctl perceive` 记录。机制上任何会话都能调 `ctl perceive`，Plan 的提示词没让它用，它的只读命令清单里也没有这一条。会话绑定了 workspace 时，宿主把 workspace 记成感知的主体。记忆模式由宿主把关：不写模式下 `ctl perceive` 不记录，会话创建的工作继承这个模式。

  **做法**　Plan 的提示词加一段：值得以后复用的结论用 `agent-session ctl perceive` 记下，写明依据的报告和适用范围。典型的是预期外的失败在分析出原因之后的结论。普通进度不记。命令清单加上这一条。Plan 在终报里列出它记下了什么。`work_simple` 和 `explore` 的提示词不提 `ctl perceive`，值得记的写进报告。`chat_route` 写明：转述 Plan 的报告时不再记它已经记过的内容。

  **验收场景**　一个 Todo 因为 workspace 的特殊情况意外失败，Plan 分析后记下一条结论，能追溯到那份报告，主体里有这个 workspace。UI 转述时不再记一遍。普通进度留在 Todo 记录里。无记忆模式的会话发起的计划不写。

  **验证**　记忆模式和主体绑定有单测。Plan 记什么、UI 是否重复记，要真实模型 DV。[Plan 行为](../src/apps/jarvis_agent/agent/behaviors/plan_simple.toml) · [UI 记忆规则](../src/apps/jarvis_agent/agent/behaviors/chat_route.toml) · [记忆写入入口](../src/frame/lib_opendan/src/runner/consolidation.rs) · [感知的主体](../src/frame/lib_opendan/src/memory/mod.rs)

## work_simple 与 explore

你在 11-24 给 `work_simple` 的定位：

> 其实work-simple是一个非常基础的设施：相比xllm ,agent worksession能使用更多的runtime,能力更强（是分布式的），因此opendan实际上定义了一个核心动作：创建simple worksession (干活的主体）
> - 清楚的任务目标，包括报告的格式
> - 正确的workspace 或 runtime
> - 合适的work_skills

创建方有三个：会话的意图分析、Goal、Plan（经 `todo add`）。会话直接发起的 `work_simple` 从 11-10 起承担扫描、查进程这类一个会话能做完的事，11-23 起有 inbox。下面各项写明对哪一种来源生效，没写的都适用。

- [ ] **三个创建方创建 simple work session 时给全三样东西。**（新增，按上面的定位核对出来的，待你过目）

  **现状**　三处提示词各写了一遍怎么创建，要求不完全一样。
  - 任务目标：三处都要求写目标和怎样算完成。准备好的标准详略不一，分析那一处最全：目标、范围、怎样检查、输入、依赖、runtime、workspace、可以改什么、手册。报告要包含什么，三处都没有让创建方说。`work_simple` 自己有一段固定的终报要求：结论、证据、交付物在哪、artifacts。`report` 有一个放结构化结果的 `result` 字段，没有哪个创建方用它。
  - workspace 和 runtime：三处都能指定，宿主在创建时检查 runtime 是否可用、只读授权、手册和 runtime 是否匹配。缺省不同：分析必须写 `--workspace`，Goal 可以不写，Todo 缺省继承 Plan 的。
  - work skills：三处已经共用同一段选择规则，宿主检查名字和环境。参数名不同，`create-worksession` 用 `--cmd-skill`，Todo 用 `--skill`。

  **做法**　只改提示词，不加参数。
  - 把“一个 simple work session 需要什么”写成一段共用的规则，和手册的选择规则一样由三个创建方包含。内容是：目标、范围、输入、可以改什么、怎样算完成、报告要包含什么，以及 runtime、workspace、work skills 的选法。创建方需要结构化结果时，写明要放进 `result` 的字段。三处各自的那几句删掉。
  - `work_simple` 和 `explore` 的终报要求加一句：任务写了报告要包含什么，就逐项给；没写，按现在的固定要求。

  **验收场景**　同一件事分别由会话、Goal、Plan 交办，Worker 收到的任务都说清了目标、怎样算完成和报告要包含什么，运行位置和手册选得一样。创建方要求了报告里的某几项时，终报逐项给出。

  **验证**　真实模型 DV。改完后确认每个 behavior 仍能解析。[分析创建工作](../src/apps/jarvis_agent/agent/behaviors/analyze_intent.toml) · [Goal 创建工作](../src/apps/jarvis_agent/agent/behaviors/goal.toml) · [Plan 的 Todo](../src/apps/jarvis_agent/agent/behaviors/plan_simple.toml) · [手册选择规则](../src/apps/jarvis_agent/agent/behaviors/skill_select_rule.inc) · [终报要求](../src/apps/jarvis_agent/agent/behaviors/work_simple.toml) · [report 的参数](../src/frame/lib_opendan/src/runner/reports.rs)

- [ ] **Worker 看得到整张 TodoList，验收标准可选。**（D9，只对 Plan 的 Todo）

  **现状**　首条输入已有 Todo ID、attempt、正文、已填写的验收标准、上游结果摘要、报告引用和产物引用。清单里别的 Todo 它看不到，只看得到自己依赖的上游。Plan 已选择或沿用默认 runtime、workspace，并可指定 cmd skills。`--accept` 可以不给。

  **做法**　首条输入附上整张清单的紧凑视图：每条的编号、标题、类型、状态和依赖，标出它自己。Todo 被更新时的输入再附一次。`work_simple` 和 `explore` 的提示词加一句：后面排了验证 Todo 时，自己只做验收标准要求的检查，不重复它会做的深度验证。`todo add` 对没有 `--accept` 的 work Todo 在输出里提醒一行，不拒绝，explore 不提醒。Plan 提示词里“工作会话看不到本会话的任何东西”的说法同步改。

  **验收场景**　仅凭首条任务和运行环境，Worker 能说明自己负责哪一部分、用什么输入、交付到哪里、怎样证明完成。后面排了验证 Todo 时，它的终报只给验收标准要求的证据，不重跑验证 Todo 会做的检查。后面没有验证 Todo 时照常自查。缺必要条件时能给 Plan 具体原因。只需基础命令的工作允许零 skill。

  **验证**　首条输入的内容和提醒有单测。Worker 是否少做了重复验证要真实模型 DV。[Todo 交接内容](../src/frame/agent_tool/src/todo_tools.rs) · [子会话创建](../src/frame/lib_opendan/src/todos.rs)

- [ ] **运行中的 Todo 更新及时送达。**（D1，只对 Plan 的 Todo）

  **现状**　`todo update` 把新正文和验收条件送给运行中的 Worker。更新要等 Worker 的当前 Turn 结束才读到，一路调工具不停的 Worker 要到提交终报时才看到，这时终报被拒绝。Plan 也可以 `todo stop` 后 `todo retry`。会话直接发起的 `work_simple` 已经在下一次推理前读补充。

  **做法**　Todo 更新在 Worker 的下一次推理前读入，和 inbox 用同一处检查点。Plan 的提示词写明何时原地改、何时停掉重派。命令结束事件、取消和停止控制不变。

  **验收场景**　Worker 连续调工具时收到更新，下一次推理就按新任务做，不用等到终报。执行中途用户改变交付目标，Plan 停掉旧 attempt 重派，旧 attempt 不记为成功。后台命令结束后，Worker 仍能收到结果、验证并提交一次终报。

  **验证**　宿主单测，要调整已有用例对送达时机的断言。Plan 选哪条路要真实模型 DV。[Work 行为](../src/apps/jarvis_agent/agent/behaviors/work_simple.toml) · [检查点](../src/frame/lib_opendan/src/runner/hook.rs) · [运行中更新测试](../src/frame/lib_opendan/tests/plan_simple_async.rs)

- [ ] **Work 等命令时的 tick 改成退避。**（D4）

  **现状**　Work 等自己启动的命令时每 30 秒 tick 一次，每次是一次推理，30 分钟的构建是 60 次。工具轮数按 Turn 补满，墙钟保护一段连续运行，缺省 60 分钟，等待和重启间隔不计。

  **做法**　tick 间隔从 30 秒起翻倍，封顶 10 分钟，任何别的输入让间隔回到 30 秒。和 Plan 的 timer 是同一处实现，只是起点和封顶不同。墙钟和工具轮数不动。

  **验收场景**　用模拟时间。等一条 30 分钟的命令，期间 tick 不超过 6 次。命令一结束，结果立刻作为输入到达。几条命令先后结束时，每次结束后的间隔都从 30 秒重新开始。

  **验证**　宿主单测，要调整已有的 tick 用例。[预算与恢复测试](../src/frame/lib_opendan/tests/work_simple_async.rs) · [等待与 timer](../src/frame/lib_opendan/src/runner/ticks.rs) · [现行墙钟设计](done/opendan-work-context-budget-fix-todo.md)

- [ ] **上下文放不下时把结局交还 Plan。**（D10）

  **现状**　多轮 Work 到窗口的 80% 时压缩到 30%，继续做。压缩三次仍放不下时，run 暂停，Turn 保持打开，等以后的驱动再试。按代码看，Plan 这时收不到任何输入，我没有实测。

  **做法**　压缩不变。Todo 的 attempt 压缩三次仍放不下时以失败结束，原因 `context_limit`，带最后一次进度报告。Plan 的提示词写明：这种失败重派时把上一次 attempt 的报告引用交给新的 attempt。会话直接发起的 `work_simple` 同样结束，结果回到会话。

  **验收场景**　让短 Work 的命令产生超出窗口的输出。能压缩时继续做完。放不下时 Plan 获得原因、已有证据和未完成项，新的 attempt 能读到上一次的报告。同样场景下长 Plan 仍能压缩续跑。

  **验证**　宿主单测。[上下文测试](../src/frame/lib_opendan/tests/context_limit.rs)

- [ ] **在同一次尝试内及时报告确定性失败。**

  **现状**　业务失败的终报已能结束 attempt，Plan 可以说明原因后显式重试。提示词没有约定 Worker 何时停止重跑失败的验证。连续 4 次工具出错由宿主结束，这管的是工具错误，不是验收不过。

  **做法**　`work_simple` 的提示词加一条：同一条验收检查，输入相同，同样的错误出现两次，中间没有改动也没有新证据，就以 `failed` 终报，写明失败原因和已验证的部分结果。

  **验收场景**　在同一输入上得到确定的验收错误时，Worker 不再重跑，提交失败原因和已验证的部分结果。Plan 补齐条件后可以发起新 attempt。短暂的 provider 错误沿用现有的有界重试。

  **验证**　真实模型 DV。[Work 行为](../src/apps/jarvis_agent/agent/behaviors/work_simple.toml) · [运行时重试](../src/frame/lib_opendan/src/runner/retry.rs)

- [ ] **修 explore 在只读 runtime 上的准入。**（D11、D12）

  **现状**　并行 explore 已可用，默认没有 workspace，可以在自己的目录写复现脚本和证据。`explore.toml` 没有声明 `workspace = "read_only"`，只读 runtime 的准入检查因此拒绝它，而 Plan 的提示词和准入代码的注释都说只读 runtime 可以跑 exploration。只读授权不含 Exec，准入修好后 explore 在那里只能读文件，跑不了命令。在可写的机器上，shell 能按绝对路径改调查对象，只有提示词里一句不要修改。

  **做法**　`explore.toml` 声明 `workspace = "read_only"`。在一个只读 runtime 上实测 explore 还能做什么，按结果改 Plan 提示词里“只读 runtime 可以跑 exploration”的说法。写回靠提示词，记为已知风险。调查方法沿用领域 plan skill 和现有 cmd skills。

  **验收场景**　指定只读 runtime 的 explore 能启动并读取。两个 explore 并行读同一份源材料，各自保存证据。

  **验证**　准入有单测。只读 runtime 上的实际能力要在真实 Zone 上看。[Explore 行为](../src/apps/jarvis_agent/agent/behaviors/explore.toml) · [runtime 准入](../src/frame/lib_opendan/src/api.rs) · [研究计划手册](../src/apps/jarvis_agent/agent/plan_skills/plan_research/SKILL.md)

## 不做

这些做法在裁决里没有采用，实施时不要再加回来。

- 冻结运行中的 Todo（D1）。
- 给 Work 加 30 分钟的时限或寿命上限（D2）。
- 等待登记、检查脚本、等待的到期时间（D3）。
- Plan 历史的专用重建规则，限制 Plan 只能用查询命令（D5）。
- “等 Plan 接受才放行后继”的标记或 review 命令（D6）。
- 强制 work Todo 带验收标准（D9）。
- 短 Work 不压缩（D10）。
- explore 的硬只读边界，这是已知风险：在可写的机器上 explore 能改调查对象（D11）。
- 单设 explore skill（D12）。

## 引用的旧裁决

| 裁决 | 内容 | 出处 |
| --- | --- | --- |
| Q7（11-02） | 墙钟只量一段连续运行；会话寿命不设缺省上限；触顶先收尾。本单只给 Plan 加 48 小时寿命上限（D3） | [墙钟设计](done/opendan-work-context-budget-fix-todo.md) §3.4 |
| D27（11-10） | 重启后都手工恢复，自动恢复留到下一期 | [Agent UI 升级](done/agent-ui-upgrade-todo.md) §2 |
| D28、D30 | 等自己发起的工作的 Work 算正在工作；在等用户回答的不算；Goal 照常激活 | 同上，§2.1 推定 13 |
| 11-10 | 一个会话能从头做到尾的事走 `work_simple`，不进 Plan | [分析分流](../src/apps/jarvis_agent/agent/behaviors/analyze_intent.toml) |
| 聊天路由 D6（11-23） | `work_simple` 可以有 inbox，补充在下一次推理前读入 | [聊天路由](jarvis-chat-route-todo.md) 裁决记录 |
| 聊天路由 D8、D10（11-23） | 记忆模式三选一，创建的工作继承；“什么值得写成感知”另立 TODO | 同上 |
