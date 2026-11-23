# Jarvis 长期计划与短任务执行 TODO

待 Review · 2026-11-18 · 对照实现 `623b4d196`

目标是让 `plan_simple` 持续管理可跨 12 小时、可变更、可等待的目标，让 `work_simple` 在约 30 分钟内独立完成一个已准备好的 Todo。建议先审「Plan 怎样等」「Work 能否改题」「30 分钟从何时算」三项。它们直接决定一次任务应该继续、暂停还是结束。

**已修复**　回归测试覆盖了一条只有一个前置项的依赖链。前置 Todo 的终报为 `failed` 或 `blocked` 时，后继启动 0 次。前置报告 `succeeded` 时，后继启动 1 次。已有的依赖调度、并行 explore 和领域 plan skill 继续作为下列事项的基础。[调度回归测试](../src/frame/lib_opendan/tests/plan_simple_async.rs) · [现有 plan skills](../src/apps/jarvis_agent/agent/plan_skills)

## plan_simple

- [ ] **在父 UI 会话内完成需求澄清。** 目前 `ask_user` 发通知，能关联最近的 UI 祖先，父 UI 可以代答，TaskMgr 将答案送回提问的 Plan。普通会话消息另由 UI 转发给 Plan。Plan 提示主要在领域 skill 要求访谈或后期只差一个决定时提问，其他关键前提缺失时倾向于直接终报。建议把「答案会改变拆解或验收」作为澄清条件。

  **验收场景**　需求明确时直接建单。缺关键前提时，用户在原会话看到问题并回答，正确的 Plan 只消费一次答案，此前不派依赖该答案的 Todo。普通需求变更仍单独处理。[Plan 行为](../src/apps/jarvis_agent/agent/behaviors/plan_simple.toml) · [通知路由](../src/frame/lib_opendan/src/notify.rs) · [UI 代答测试](../src/frame/lib_opendan/tests/notify.rs)

- [ ] **支持等待外部条件超过 12 小时。** 目前 Plan 有持久输入队列，可以等待已跟踪的 task 或子会话。没有这两类工作时，仅回复「等文件」会收到一次 runner 提醒，再次空回复会以 `missing_end_report` 结束。提问默认 3 小时过期，可通过 `--expires` 调整。

  **待定**　外部等待由什么事件解除？等待到期如何处理？宿主重启后哪些等待自动恢复？

  **验收场景**　让没有运行中子任务的 Plan 等一份次日才交付的文件，期间重启宿主。无变化时不为报平安调用模型。文件到达后按当前计划推进一次，重复事件不重复派工。等待期间用户仍能修改或取消计划。[等待与提醒](../src/frame/lib_opendan/src/runner/ticks.rs) · [结束判定](../src/frame/lib_opendan/src/runner/outcome.rs) · [宿主重启](../src/frame/lib_opendan/src/runner/drive.rs) · [提问有效期](../src/tools/buckyos-agent/lib/ask.ts)

- [ ] **让 Plan 的决策历史只保留需求、Todo 变更和等待。** 目前 Plan 可直接读 workspace、子会话 worklog 和报告附件，定时 tick 也要求回复。workspace 绑定为只读，但仍开放通用 shell，哪些命令能用主要由提示词约束。通用历史会从 worklog 重建工具调用及其结果。建议把报告引用随 Todo 决策保存。领域手册、runtime 清单仍用于拆解和选环境，需要调查项目事实或复核成果时派 Todo。

  **待定**　是否在工具层限制 Plan 只能查计划元数据、收报告、改 Todo 和登记等待？

  **验收场景**　恢复一个多轮 Plan 时，模型能看出谁改了什么需求、哪份报告导致哪项调整、正在等什么。命令输出仍留在审计 worklog。重复 tick 不挤占决策历史。[Plan 行为](../src/apps/jarvis_agent/agent/behaviors/plan_simple.toml) · [历史重建](../src/frame/lib_opendan/src/runner/history.rs)

- [ ] **把必要的独立验收排在消费结果之前。** Worker 报 `succeeded` 后，调度器可在 Plan 下一次推理前启动后继。Plan 事后才发现证据不足、再补验证 Todo，已经启动的后继来不及拦住。

  **验收场景**　以「产出 → 独立验收 → 发布」建单，发布显式依赖验收。即使产出方报成功，只要验收失败，发布就启动 0 次。普通 Todo 可以用自己的证据验收。Plan 宣告目标完成时能逐项指出关键验收证据，并说明取消或未完成的事项。[Plan 验收规则](../src/apps/jarvis_agent/agent/behaviors/plan_simple.toml) · [调度时机](../src/frame/lib_opendan/src/runner/todos.rs)

- [ ] **确定谁把计划中值得复用的事实写入 Memory。** 目前结束会话不会自动写入记忆，UI 已被要求在转述成果时用 `agent-session ctl perceive` 记录值得保留的内容。

  **待定**　由 Plan 在获得证据时记录，还是由父 UI 统一记录？没有父 UI 转述报告时，由谁记录？

  **验收场景**　用户确认一项长期约束后，记忆材料可追溯到原话或报告，重启和 UI 转述不会重复写入，普通进度留在 Todo 记录里。[UI 记忆规则](../src/apps/jarvis_agent/agent/behaviors/chat_route.toml) · [记忆写入入口](../src/frame/lib_opendan/src/runner/consolidation.rs)

## work_simple 与 explore

- [ ] **定义 Todo 达到什么条件才能派工。** 首条输入已有 Todo ID、attempt、正文、已填写的验收标准和上游结果摘要、报告引用及产物引用。Plan 已选择或沿用默认 runtime、workspace，并可指定 cmd skills。目前 `--accept` 可空，交付给谁、怎样使用也靠正文说明。

  **待定**　哪些工作必须显式给验收方法，哪些可以从目标直接判定？

  **验收场景**　仅凭首条任务及其运行环境，Worker 能说明自己负责哪一部分、用什么输入、交付到哪里、怎样证明完成。缺必要条件时能给 Plan 具体原因。只需基础命令的工作允许零 skill。[Todo 交接内容](../src/frame/agent_tool/src/todo_tools.rs) · [子会话创建](../src/frame/lib_opendan/src/todos.rs)

- [ ] **决定运行中的一次尝试是否冻结任务。** 当前 `todo update` 会把新正文和验收条件送入正在运行的 Worker，Worker 也可在少数情况下直接 `ask_user`。建议业务变更由 Plan 接收。若变更影响在途工作，由 Plan 停止旧 attempt、按新条件重派。保留命令结束事件、取消和停止控制。

  **验收场景**　执行中途用户改变交付目标时，旧结果无法按新要求记为成功。后台命令结束后，Worker 仍能收到结果、验证并提交一次终报。这涉及调整已有测试覆盖的行为。[Work 行为](../src/apps/jarvis_agent/agent/behaviors/work_simple.toml) · [运行中更新测试](../src/frame/lib_opendan/tests/plan_simple_async.rs)

- [ ] **给短 Work 定义 30 分钟时限和过期恢复。** 当前工具轮数按 Turn 补充，墙钟预算保护一段连续运行，跨 Turn 等待或重启间隔不计入同一连续段。

  **待定**　30 分钟是拆单目标还是硬上限？等命令返回和宿主停机的时间是否计入？建议每个 attempt 从首次启动计时，恢复时先检查是否过期。

  **验收场景**　用模拟时间覆盖限内续跑和过期恢复。按硬上限方案，超时后保留已有报告证据，向 Plan 标明原因，不再重做旧命令。[预算与恢复测试](../src/frame/lib_opendan/tests/work_simple_async.rs) · [现行墙钟设计](done/opendan-work-context-budget-fix-todo.md)

- [ ] **决定短 Work 用尽上下文时怎样结束。** 当前多轮 Work 会压缩历史并继续。建议短 Work 尽量使用原始上下文。

  **待定**　窗口将满时，是预留空间让 Worker 提交最后一次报告，还是直接由宿主记录容量不足并返回已有证据？

  **验收场景**　让短 Work 的命令产生超出窗口的输出。Plan 能获得原因、已有证据和未完成项。同样场景下长 Plan 仍能压缩续跑。[上下文测试](../src/frame/lib_opendan/tests/context_limit.rs)

- [ ] **在同一次尝试内及时报告确定性失败。** 业务失败终报已能结束 attempt，Plan 可说明原因后显式重试。还需约定 Worker 何时停止在同一次尝试里重跑失败的验证。

  **验收场景**　在同一输入上得到确定的验收错误，且没有新证据或修正时，Worker 提交失败原因和已验证的部分结果。Plan 补齐条件后可发起新 attempt。短暂 provider 错误沿用现有有界重试。[Work 行为](../src/apps/jarvis_agent/agent/behaviors/work_simple.toml) · [运行时重试](../src/frame/lib_opendan/src/runner/retry.rs)

- [ ] **补齐 explore 的只读边界，决定调查方法放在哪里。** 并行 explore 已可用，默认无 workspace，允许在自己的目录写复现脚本和证据。但其 behavior 没有声明 `workspace = "read_only"`，指定只读 runtime 时会被准入检查拒绝。在可写机器上，无 workspace 也不能阻止 shell 通过绝对路径修改调查对象。

  **验收场景**　两个 explore 可并行读取同一份源材料，各自保存证据。尝试写回源材料时被拒绝。

  **待定**　沿用领域 plan skill 描述调查问题和证据标准、用现有 cmd skills 提供命令，还是为多个领域共用的调查方法单设 explore skill？[Explore 行为](../src/apps/jarvis_agent/agent/behaviors/explore.toml) · [runtime 准入](../src/frame/lib_opendan/src/api.rs) · [研究计划手册](../src/apps/jarvis_agent/agent/plan_skills/plan_research/SKILL.md)
