# Jarvis 默认 Goal Session：感知整理、认知维护与计划发现 TODO

日期：2026-10-23

状态：待实现；本文只给出实现方案与验收标准，不表示下述新增能力已经存在。

读者：后续 codeagent。路径相对 `buckyos/`，实施前重新核对源码；不要把旧 TODO 的实施记录当作当前接口。

依据：

- [OpenDAN Agent Session 架构设计](<../doc/opendan/OpenDAN Agent Session架构设计.md>)，重点 §7、§8.1、§9.2、§11、§12。
- [Agent Memory 认知管理需求](<../doc/opendan/Agent Memory 认知管理需求.md>)，重点 M-15—M-17、M-20、M-30—M-34、§5.6—§5.9、附录 A/B。
- [Goal Session TODO](opendan-goal-session-todo.md) §9.2、[Memory 多 Session 模拟 TODO](agent-memory-multi-session-simulation-todo.md)。本文细化默认整理 Goal，并补充潜在 Goal、计划任务发现和可选 decision 预检；生产 workflow 接入仍是独立依赖。

## 0. 本次设计结论

1. Jarvis 默认启用一个 `maintain` 类型的内置 Goal，建议稳定标识为 `memory_maintenance`。它整理感知、维护认知的有效性，并发现值得进一步安排的 Goal / 计划候选；使用现有 Goal Session，不新增 SessionKind。材料清空只结束本次 Activation。
2. **认知、目标、计划分开持久化**：认知存 Memory；确认中的提案存 Goal 的结构化状态；正式目标存 GoalStore；正式日程存 workflow。不能把提醒、授权、执行状态仅存为一条可被漏召回的 Memory。
3. 典型认知使用现有主谓宾关系，加来源、范围、创建时间、稳定度和时间有效性。置信度、稳定度、重要性与有效期分别表达，不能混用一个分数。
4. 默认每 2 小时提供一次整理检查机会；有待处理感知或有效计划任务时，每 30 分钟检查。**检查不等于调用模型**：先确定性检查，再按需调用完整整理模型；可选 decision 只减少不必要的推理。
5. 本文把“感知和计划任务存在”解释为 **OR**：任一存在便进入活跃检查周期。感知指未处置或已到重评期限的材料；计划指仍有效的业务日程，或有新变化/已到复核点的候选。无变化的待确认提案不反复询问；远期日程可以触发廉价检查，但不因此每 30 分钟调用大模型。
6. 潜在意图先成为候选。已有明确、可追溯授权且参数齐全时可以落地；推断出的长期职责、模糊时刻或新增副作用需要用户确认。确认等待属于 Goal 状态，不创建等待用户的 Work。
7. 第一阶段复用 `LocalScheduleBackend`。真实提醒及定时 Work 的执行仍由既有业务计划能力承担；默认整理 Goal 不充当计时器、通用任务执行器或新的调度服务。

以下新增字段、工具名、配置名均为实施建议；在 T01 冻结具体命名。默认阈值是首版可配置策略，不是对认知可靠性的科学标定。

## 1. 源码核对：已有基础与实际缺口

| 范围 | 当前实现（2026-10-23） | 本次需要补齐 |
|---|---|---|
| Jarvis 启动 | [agent.toml](../src/apps/jarvis_agent/agent/agent.toml) 已启用 `[loader.goals]`；[loader.rs](../src/frame/opendan/src/loader.rs) 创建 `GoalHost` | 只托管已存在的 Goal，没有默认 Goal 的幂等播种 |
| Goal 生命周期 | [state/goals.rs](../src/frame/lib_opendan/src/state/goals.rs)、[goal.rs](../src/frame/lib_opendan/src/goal.rs)、[runner/goal.rs](../src/frame/lib_opendan/src/runner/goal.rs) 已有 Goal、epoch、Activation、预算、Work 结果吸收、状态写回 | 整理专用策略、工具、能力限制和结构化候选状态 |
| 调度 | [host/goals.rs](../src/frame/lib_opendan/src/host/goals.rs) 有 `ScheduleBackend`、`LocalScheduleBackend`、`AdmissionCheck`；周期按 UNIX 时间槽计算，有 pending fire 和稳定投递 key | `GoalScheduleSpec` 只有 `every_secs/min_interval_secs/start_on_create`；没有 2h/30m 自适应策略。准入链约定不调用 LLM |
| Memory 组件 | [memory/consolidation.rs](../src/frame/lib_opendan/src/memory/consolidation.rs) 已有 `pending_work`、`Consolidator::begin/commit/cleanup/sweep_expired` | 接入真实 Goal Runner；`pending_work` 只看感知，不含认知到期复核、业务计划或候选 |
| 旧整理链路 | [runner/outcome.rs](../src/frame/lib_opendan/src/runner/outcome.rs)、[state/cognition.rs](../src/frame/lib_opendan/src/state/cognition.rs) 仍在 SelfImprove 成功后推进旧感知游标；提交记录可为 `memory_ops: 0` | 不能以自然语言“已整理”或 Session 成功代替实际认知提交；接入新组件后删除对应旧路径 |
| 感知入口 | [runner/inputs.rs](../src/frame/lib_opendan/src/runner/inputs.rs) 的 `ControlCommand::Perceive` 仍构造简化记录；[memory/perception.rs](../src/frame/lib_opendan/src/memory/perception.rs) 已有来源、范围及幂等校验门面 | Runner/CLI 接线，宿主绑定真实来源与授权主体，不能把全部旧记录默认为用户明确表达 |
| 实际召回 | [runner/drive.rs](../src/frame/lib_opendan/src/runner/drive.rs) 和 [xagent_cli.rs](../src/frame/lib_opendan/src/xagent_cli.rs) 仍走旧 `recall_hints(tags)`，没有新门面的 Caller grants/scope/观察状态 | 新组件的纠正兜底、范围过滤和观察进度尚未在真实 Session 闭环；错误不能转换为空结果 |
| 认知模型 | [agent_memory/model.rs](../src/frame/agent_tool/src/agent_memory/model.rs) 的 `MemoryItem/ItemMeta/UpsertRelationOp` 支持主谓宾、scope、basis、explicit、confidence、revision、evidence、valid_until、review_when | 没有显式 stability；`noticed_at` 是当前 revision 时间，不能当创建时间或最近证据时间 |
| 时间退化 | [agent_memory/recall.rs](../src/frame/agent_tool/src/agent_memory/recall.rs) 的 `FadeConfig` 默认每 30 天扣 1 个召回分、最多 20 分，`explicit` 不淡出；到 `valid_until` 后默认不召回 | 按稳定度区分退化速度、区分修订与新证据、产生可确定的到期复核信号 |
| 写入口 | [agent_tool_cli_dev/src/lib.rs](../src/frame/agent_tool_cli_dev/src/lib.rs) 的 `run_agent_memory_blocking` 仍可直接走 `AgentMemory` 管理写入 | 普通 Session 不得绕过整理 lease 直接修改认知；只在 facade 收口还不够 |
| 用户确认 | [runner/goal.rs](../src/frame/lib_opendan/src/runner/goal.rs)、[notify.rs](../src/frame/lib_opendan/src/notify.rs) 已把 `questions` 发为通知，回答可触发后续 Activation | 当前主要是文本 question，尚无候选 ID/版本/授权范围绑定和确认后幂等落地 |
| 创建权限 | `GoalStore::create` 当前允许 Owner/UI，拒绝 Goal Session 创建其他 Goal | 不能让整理 Goal 冒充 Owner；需窄的宿主播种及“已授权候选落地”入口 |
| decision | [aicc_client.rs](../src/kernel/buckyos-api/src/aicc_client.rs)、[aicc_decision.rs](../src/kernel/buckyos-api/src/aicc_decision.rs)、AICC 路由已有 `decision.evaluate` | OpenDAN/Jarvis 尚未调用；逻辑模型名存在不代表有可用 provider |

计划任务当前能力与阻塞见 §5.3。旧 `self_check.toml` 的业务分类可以参考，但它仍依赖 Notebook 和旧模板，不可直接作为新 Goal 的执行手册。

## 2. 默认 Goal 的身份、职责与生命周期

- 一个 Agent 一个内置目标，使用 Agent 身份 + builtin key 的确定性幂等键；不要用显示名 `Jarvis` 或标题判断唯一性。Loader 重启、升级、双重 reconcile 都不得重复创建。
- 目标定义：`lifecycle=maintain`；健康状态是“已到处理条件的感知被可靠处置，认知可追溯且按时复核，发现的事项有明确去处”。距离至少统计新材料、到期暂缓、清理失败、待复核认知、待确认候选、待落地动作及最老积压年龄。
- 增加显式 builtin 标记、策略版本、整理能力与 `records_perception=false`。这应是受信配置/宿主能力，模型不能通过改名字获取。
- 建议使用独立 `behaviors/memory_maintenance.toml`，保留通用 `behaviors/goal.toml`。模板/Goal 默认覆盖设定小批次预算，不能直接继承目前通用 Goal 的 300000 token Activation 上限而无说明。
- 首次启用时播种定义并立即做一次廉价检查；有材料则整理，无材料则进入等待。现有 `start_on_create` 直接投 Created，且 Reopened/Restarted 也不走定时 gate，必须为 builtin 补检查入口，不能只配置 `start_on_create=true` 就宣称空材料零模型调用。之后按 §6 提供机会，Session 可以 idle unload，目标身份与状态长期保留。
- 用户 pause/archive/关闭默认职责后，重启不得重新激活或换 ID 重建；恢复必须使用现有 resume/reopen 语义。配置关闭时取消后台检查，不删除认知和用户确认过的业务日程。
- 模板升级记录版本，不覆盖用户改过的预算、范围、状态和计划；按需显式 revise/restart，遵守现有 behavior 冻结机制。新 epoch 接管候选及未完成提交，不重新提问或重复创建对象。
- 建议首版限额：每批约 20 条材料（复用 `BatchOptions`，同一来源表达不拆分）、每次最多 3 批、单 Activation 32000 token / 4 runs / 24 tool iterations，每 24h 统计窗口整理 200000 token、48 次完整 Activation。参数可调；超限保留积压并暴露原因，不能偷偷扩额。
- 当前 `PeriodBudgetCheck` 只约束定时准入，不能单靠它兑现以上总上限：内置整理 Goal 的所有模型入口（含 created/manual/answer）统一检查周期剩余量，并把本轮 cap 限制到剩余额度。事件照常可靠接收，预算不足时延期推理；恢复提交/发布/清理等机械工作不消耗模型额度，仍可在 lease 与退避约束下进行。

## 3. 典型认知结构与时间语义

### 3.1 在现有模型上扩充

| 语义 | 字段/映射 | 约束 |
|---|---|---|
| 主谓宾 | `claim={type:"relation",subject,predicate,object}`，复用 `UpsertRelationOp` | 主/宾引用已有或本批创建的对象；规范化 predicate，保留方向。属性/经验可继续用已有 `attribute/observation_inference/free`，不强迫所有文本伪装成关系 |
| 自然语言解释 | 现有 statement/语义摘要与 `semantic_kind` | 表达适用条件，不能仅凭一个三元组省略否定、时间或例外 |
| 身份与修订 | `item_id + revision`，`expected_revision` | 相同语义、相同范围优先修订已有条目；冲突用显式替代/撤回关系，不按时间戳静默覆盖 |
| 创建时间 | `created_at` 读取投影，来自 `first_occasion` | 由宿主/日志确定，逻辑认知首次形成时间；后续修订不变，不要求模型填时钟 |
| 修订时间 | 现有 `noticed_at` | 表示这一 revision 何时生成，不能用它自动给事实“续期” |
| 最近独立证据时间 | 建议新增 `last_evidence_at` | 只在独立证据支持或重新验证当前结论时更新；反例触发复核，不给旧结论续期；召回、重述、整理摘要、改 tag/状态不更新 |
| 稳定度 | 建议新增 `stability=stable/contextual/volatile` | 表示预期随时间变化的速度；没有足够依据时取 contextual，不能因 confidence 高就设 stable |
| 置信度/重要性 | 现有 `confidence/weight` | 前者是证据支持程度，后者是召回重要性；都不是稳定度或权限 |
| 来源性质 | 现有 `basis/explicit/evidence/produced_by` | 区分用户明确表达、观测、推断；记下原始事件和整理模型/提示词版本；转述不算独立证据 |
| 范围与可见性 | 现有 `scope`、subjects、objects、exceptions | 默认最窄；主语对象和授权主体不同，不可由模型把局部内容扩大到全局 |
| 有效期与复核 | 现有 `valid_until/review_when`，建议新增结构化 `review_at` | `review_when` 自然语言只交给模型理解；需要准时检查的日期存正式字段，宿主不解析自由文本条件 |
| 状态 | 现有 status/replaces/replaced_by | 到期隐藏和召回淡出是派生视图；正式修订/撤回只经整理提交 |

示例（语义示意，非可直接提交的完整 JSON）：

```text
subject = obj_owner
predicate = prefers
object = obj_concise_release_report
scope = 当前用户 + 项目 A
basis = 用户明确表达；explicit = true
created_at = 首次整理时间；last_evidence_at = 原始表达被可靠观测的时间
stability = stable；valid_until = 无；review_when = [用户明确纠正]
evidence = [obs_* 证据观察引用] → MemoryObservation.source_ref → 原始消息事件
```

至少准备三种 fixture：明确偏好（stable）、特定项目/版本经验（contextual）、有明确结束时刻的短期背景（volatile + valid_until）。时间、在线状态等实时数据通常从世界状态读取，不为验证 volatile 而把所有瞬时数据都写进 Memory。

### 3.2 退化策略

首版沿用现有召回扣分算法，增加稳定度对应的配置，不引入新的存储引擎或复杂概率模型：

```text
age_days = max(0, now - last_evidence_at) / 86400
fade = min(age_days / days_per_point[stability], max_penalty)
建议初值：stable=365，contextual=30，volatile=1；max_penalty=20
```

- `explicit=true` 继续不做召回淡出，遵守 M-20；明确表达也可能有 `valid_until` 或被纠正，这两项仍有效。`stable` 不等于永久正确。
- 不按时间偷偷改写持久 `confidence`，不把“低优先级”变成“事实为假”，不以淡出为理由自动删除原始证据。
- 对旧材料/缺失证据时间的处理在 schema 变更时统一确定；本仓库默认允许 breaking change，不附带旧版本迁移工程。不能直接拿当前时间填补，从而把陈旧内容刷新成新证据。
- 到 `review_at` 只表示需要检查，复核缺证则保留未知/暂缓；到 `valid_until` 按现有规则不再默认召回。到期维护要有单独的确定性查询，不能假设 `pending_work()` 已覆盖。
- 用户纠正优先于既有推断；复用 Memory 组件已有“待复核/可能已被纠正”兜底，并按 T02 接入真实 Session，使纠正感知在正式提交前也可显露。不得用稳定度高来屏蔽纠正。

## 4. 一次 Activation 的整理流程与提交边界

1. **读正式状态**：本 Goal 定义、上次提交回执、候选、开放问题、正式 Goal/日程及其变化；Memory 只提供认识线索，不提供授权或业务执行状态。
2. **独占整理**：Runner 在 Activation 执行期间持 Agent 级整理 lease，建议将 `CONSOLIDATION_LEASE` 从 `self_improve` 收敛为 `memory_consolidation`。不在等待用户或休眠时长持；失去 lease/epoch 时禁止写入。
3. **机械维护**：先恢复已提交结果和待重试清理；对超出暂缓窗口及 grace 的材料复用 `sweep_expired`，保留处置理由。仅清理重试不需要模型。
4. **取批次**：通过 `Consolidator::begin` 获得有界材料、关联暂缓上下文、graph seq 和认知引用。按原始来源去重；同一次表达不拆开。`Batch.context` 也必须有上下文预算，当前实现可能取回较多关联材料，需限制装配且不能处置未读项。
5. **综合判断**：区分长期偏好/经验、当前任务要求、外部事实缓存、猜测、回声、纠正；决定吸收、丢弃或暂缓。暂缓写明确的缺证与期限，不能每轮延长同一观察窗口。允许整批只丢弃/暂缓，不要求硬造认知。普通认知缺证优先暂缓或留低优先级澄清线索，不为每条推断主动通知；§5 的正式授权确认另行处理。
6. **并行产出候选**：同一材料可以支持认知，也可以暴露 Goal/日程意图；候选引用原始事件、已有正式对象和认知版本。候选属于业务提案，不塞进 `MemoryItem` 或 `pending: Vec<String>` 充当唯一存储。
7. **可靠提交**：经宿主工具调用 `Consolidator::commit`，把认知操作和感知 disposition 写成现有单个 envelope；校验来源、范围、引用、revision、批次成员、lease epoch、幂等键。仅 `report` 成功不构成整理成功。
8. **发布结果再清理**：候选可靠落盘后才允许清理其来源感知正文；正式对象创建/通知发送通过可恢复的幂等动作处理。用 `cleanup()` 保留感知身份、幂等键、处置和来源引用，失败独立重试。
9. **结束本轮**：报告真实提交次数、认知变更/处置数量、候选、未决项、预算及失败阶段。未处理材料留待后续机会；不等待派生 Work，不因空批次宣告 Goal achieved。

### 4.1 工具与崩溃恢复

- 推荐 Runner 内宿主工具 `memory_consolidation_begin/commit`，与 `report` 同层，由 Runner 持有 lease。不要让 shell 子进程重新抢锁或持模型自报的 lease epoch；工具通过 capability 限制只对内置整理 Goal 开放。
- `commit` 请求可同时携带认知提交计划和结构化候选，但 **Memory envelope、Goal State、workflow 不是一个跨库事务**。实现复用 Session 提交/发布模式：先在 Session 状态持久化 prepared receipt（稳定 operation ID、完整可重放 Memory plan、实际允许处置的 Batch refs、完整候选、digest 和阶段），再提交 Memory，再提交并发布候选，最后允许清理。摘要不足以恢复，禁止重调模型改写同一幂等键对应的计划。
- 每一步可重入：Memory 使用同一幂等键重放；候选按稳定 ID upsert；正式 Goal/计划按候选版本动作键创建；通知按候选问题键发送。崩溃后先查回执和已创建对象再重试，禁止仅因“上次没有收到响应”就再建一个。
- receipt 含 epoch/Activation/来源版本，但候选及业务动作的逻辑去重身份跨 epoch 保持。当前 `prepare_epoch` 仅迁移已发布 GoalState，不能假定它会自动带走旧 Session 的 receipt：新 epoch 在 begin/cleanup 前必须沿旧 epoch Session 引用恢复未完成 receipt，或读取事先可靠发布的恢复指针。覆盖 Memory 成功但候选/GoalState 未发布即 restart 的窗口；旧 epoch 不得继续写。
- `cleanup()` 会扫描所有终局处置材料，启动恢复、独立清理重试和 sweep 也必须遵守候选发布屏障；有未完成 receipt 时先恢复，不能由另一个机械清理入口提前删正文。
- `report` 请求结束时校验已打开批次的处理情况：有材料却无有效提交，应明确 `failed/needs_info` 或记录未完成原因，不能虚报 acted。已 commit 后报告失败也不得撤销处置或重复产生认知。
- 防回声覆盖写入和读取两端：整理 Goal 不写自动 `run_digest/task_outcome` 或模型自述感知；按 builtin goal 身份跨 epoch 过滤旧自身材料。当前只排除 `SelfImprove` kind 的逻辑必须替换。其他 Goal 的真实独立观察仍允许进入感知。
- 普通 UI/Work 继续只写感知；接 `Memory::record_perception` 时由宿主绑定主体、scope、source event、记忆策略及幂等键。来源未知的旧简化记录保留未知，不补造用户授权。

## 5. 潜在 Goal 与计划任务的发现、确认和落地

### 5.1 共用候选契约

建议在 Goal State 增加类型化的维护状态区，由 driver 写入并遵守现有 Session commit → Goal State publish 顺序；不要建第二个 Notebook。候选至少有：

```text
candidate_id, revision, kind(goal|schedule), semantic_key
title, objective/action, scope, source_refs, evidence_refs
intent_basis(explicit|inferred), authorization_ref?, missing_fields[]
proposed_goal? / proposed_schedule?, estimated_budget, expected_effects
status(proposed|needs_confirmation|approved|materializing|created|linked|rejected|expired|blocked)
question_id/notify_id, approved_revision?, approved_by?, approved_at?
linked_goal_id/schedule_id, operation_key, last_error, retry_at?
created_at, updated_at, review_at?, expires_at?
```

- 同源去重和语义去重分开：相同事件重放不能再建候选；不同来源提出同一目标时合并证据、查已有 Goal/日程，优先 linked。相同标题不等于同一业务事项。
- 稳定键纳入用户/Agent、范围、动作对象、规范化触发条件与收件人。批准后变更动作、范围、时间/时区、受益人或预算上限应生成新 revision，并核对原授权是否仍覆盖。
- 人的同意绑定候选 ID + revision + 具体动作与权限范围；不能拿一条泛化的“好的”批准整批建议。超时、未回复、忽略、拒绝都不等于同意。
- 复用已有 Goal question → notify → answer 唤醒链路，补稳定问题 ID 和候选关联。问题完整描述用途、频率、成本/副作用、停止方式及缺失参数；同一开放问题不重复发送，不因 report 的 questions 列表遗漏而误撤销。
- 回答前结束 Activation 并释放整理 lease；回答后先校验提案版本与当前现实状态，再落地。无通知服务时保持 `needs_confirmation`，在可用 UI 暴露，不自动批准。
- 当前 `GoalState.questions` 最多保留 16 条且 report 整表替换，超量可能移除并取消旧通知。候选需独立持久化、设保留策略并排队；仅将最多 16 条已展示的开放确认投影到 questions，回答/关闭后再取下一批。`Answer` 原因也包含 dismissed/expired/canceled，不能仅凭收到了这个事件就执行。
- 最小授权表：用户明确且参数完备的既有指令可以继续；纯推断的新增 Goal、模糊日程、扩大收件人/访问范围需确认。确认过的同一指令不因跨 Session 而反复询问。
- 不扩大 `GoalCaller` 为“所有 Goal 都能建 Goal”。默认播种用宿主受信入口；业务落地用窄的宿主候选执行入口或 Owner/UI 路径，检查授权记录并留下 `created_by` 和来源。

### 5.2 分类必须有两个维度

**动作类型决定谁执行，触发类型决定何时执行。** 每天重复不自动意味着 Goal；需要跨时间判断和调整才是 Goal。

| 动作类型 | 例子 | 归属与落地 |
|---|---|---|
| 固定通知 | “明天 9 点提醒带护照” | 无需推理；workflow 的一次性提醒，复用通知/消息能力 |
| 确定流程 | “每周备份指定目录” | 已有且已授权的 workflow/工具流程；参数、错误与结果通道明确，不由整理 Goal 临时发明执行 DSL |
| 有界 Agent Task | “每天 8 点汇总授权邮箱的新邮件” | 每次独立的 Task/Work；目标、输入、权限、完成条件、Workspace、预算和结果交付完整；到点重新检查可执行性 |
| 长期适应性目标 | “持续跟进项目 A 的风险，按变化调整安排” | 新建业务 Goal，复杂具体动作再派 Work；日程只是它的唤醒来源 |
| 未成熟想法 | “有空想学摄影” | 候选/观察或询问，不能擅自创建每日任务 |

| 触发类型 | 表达 | 必须保存/校验 |
|---|---|---|
| 一次性绝对时间 | 明天 9 点 | 原始事件时刻、用户时区、解析后的绝对时刻；不能相对整理时刻解释“明天” |
| 固定间隔 | 每 2 小时 | interval、起算点、停用时间、错过处理；不等于每天固定墙上时间 |
| 日历重复 | 每工作日 16 点 | cron/日历规则、明确时区、夏令时重复/缺失时刻策略；不把工作日默认为节假日规则 |
| 条件/事件 | 收到评审意见后再处理 | 对象/事件条件交业务 Goal 或现有订阅；未知发生时间不得伪造 cron，也不得建长期阻塞 Work |

用户明确提出准时提醒时，UI 应直接进入同一计划创建/确认流程，不能等默认 Goal 从 Memory 偶然发现；默认 Goal 的作用是补充发现和维护。**30 分钟检查频率不是提醒精度保证。**

计划还必须有 owner、来源候选/用户事件、执行目标、结果通道、幂等键、misfire、重试/失败策略、取消条件。未知时区、任务已过期或权限不足时保留候选并解释；不要擅自顺延到明天。用户显式取消后撤销后续 fire，已执行副作用单独报告；一次临时失败不能被解释成永久取消意图。

### 5.3 workflow 的真实接入边界

已核对 [scheduled_task_manager.rs](../src/kernel/workflow/src/scheduled_task_manager.rs)、[server.rs](../src/kernel/workflow/src/server.rs)、[workflow/main.rs](../src/kernel/workflow/src/main.rs)、[workflow_service.rs](../src/kernel/buckyos-api/src/workflow_service.rs)、[task_dispatcher.rs](../src/kernel/buckyos-api/src/task_dispatcher.rs)：

- 已有 `Once/Cron/RunEvery`、TaskMgr 持久化镜像、misfire 和并发策略。`workflow.send_message` 有实际执行器，`workflow.run` 有 orchestrator 路径，可作为首批正式落地目标，仍需验证权限和参数。
- `agent.delegate` / `opendan.command` 有协议/计划转换，**没有完整 OpenDAN 接收执行链**；不能仅因创建出 schedule/task 就声称定时 Work/Goal 激活可执行。未接通时候选标 `blocked`，不返回“已经安排成功”。
- 计划创建也缺幂等合同：`WorkflowCreateScheduledTaskReq` 没有 key，服务每次创建随机 UUID。需先补按 owner + 候选 revision/action operation key 的持久唯一约束，同键同请求返回原日程、同键异内容拒绝；单纯“先查再建”不能解决创建成功但回执丢失。补齐前自动落地日程也必须 blocked。
- 旧 TODO §9.1 提到的问题仍需处理：相关 fire subtask 绑定 workflow 自身；部分成功/跳过路径仍按旧 fire_time 推算下一次，RunOnce 停机后可能连续补发；客户端状态枚举和 task ID 类型与服务端不一致；`load_schedules` 未完整翻页；时区只有硬编码表。
- 后续接入遵循当前 TaskMgr2：Dispatcher 注册目标与 RunnerFunction，走 offer → bind → activate 和 fenced result；**不要照抄旧方案让 OpenDAN 按 runner_app_id 轮询/claim task**。参考 [Task Manager README](../src/kernel/task_manager/readme.md)。
- 生产 Goal 定时接入应新增受信 scheduled-fire 入口，统一走状态、频率、预算和本 TODO gate。现有 `goals.activate` 是 Manual 路径，绕过定时准入，不能直接当生产 timer 回调。
- 此生产接入可独立交付，不阻塞本地默认 Goal 的整理、发现、持久化和确认。引入完整 IANA 时区依赖或新通用组件前，遵守 AGENTS.md 的确认要求；可先复用仓库已有能力，未支持的时区显式拒绝。

## 6. 2 小时 / 30 分钟触发与恢复语义

### 6.1 首版选择：固定机会 + 确定性 gate

复用 `LocalScheduleBackend` 的 **1800 秒时间槽**，内置整理策略决定是否给模型执行机会；空闲的基线检查使用 **7200 秒时间槽**。不每轮 revise `every_secs`，否则现实现会重设计划进度和 pending fire，并制造无意义的 Goal revision。

```text
每个 30m slot（或启动恢复）读取持久化摘要：
  M = pending_work()                 # new/due/waiting/retry_cleanup/next_deadline
  C = 到期认知复核摘要
  P = 有效业务日程 + 候选/计划变化摘要
  R = 未完成提交/发布/通知结果

  active = M.new > 0 OR M.due > 0 OR P.has_active_schedule OR P.actionable_change
  有 R / cleanup retry：按受控退避先恢复机械工作
  非启动、非 2h 基线 slot、且没有 active/C到期：跳过
  没有可供语义处理的材料或变化：记录 no_work，不调用任何模型
  有确定性紧迫项：进入完整整理，绕过 decision
  其余：可选 decision 预检；不可用则正常整理
```

- `waiting` 但尚未到期的感知、无变化的开放问题，不强迫每轮深度整理。`retry_cleanup` 只走机械清理。有效日程维持 30m 检查，但未变更且运行健康时不重复调用模型。
- `pending_work` 的 I/O 失败、Graph 无法读取、日程列表失败属于 unknown/error，不能当成 zero/no_work。保留输入和最后成功水位，有限退避并暴露状态。
- 2h/30m 是 UTC 时间槽的检查机会，**不是“上次 Activation 完成后再睡满两小时”**。创建可立即检查；故边界前创建可能很快碰到下一槽，仍受最小间隔限制。若产品要求严格相对计时，应另扩展持久 `next_check_at`，不可宣称现有 slot 已满足。
- 持久化 `last_checked_slot/last_baseline_slot`、输入版本摘要、`last_successful_consolidation_at`、最旧积压时间、preflight 结果、失败退避。检查/跳过时间和实际 commit 时间分开；decision skip 不刷新“已整理”水位。
- 确定性紧迫项包括：明确纠正/撤回、到期材料/复核、新收到的用户回答、待发布已提交结果；以及待处理材料已达 2 小时的最大拖延窗口。普通新材料可由 decision 暂缓，但不得无期限饿死；预算耗尽时保持积压并明确 blocked 原因，不绕过总预算。
- **同一时刻一个 Activation**：沿用 Session 串行所有权、输入幂等和 Agent 整理 lease。运行期间多个 tick 合并成待检查信号；完成后重读摘要，不能排队启动多个整理上下文。
- 重启补一轮最新机会，记录 missed，不为每个错过的半小时重复运行；时钟回拨不重放已处理 slot。将 gate 的检查提交与现有 pending fire 恢复语义配套设计。
- 手动 activate、用户答案、Work 结果沿现有事件链，不被周期 gate 或 decision 吞掉；仍受状态、权限和 Activation 预算约束。pause/archive 的事件按现有生命周期处理，不能因有新材料自动恢复。

## 7. 可选 decision 快速前置判断

用户所说的 `decsion` 在当前代码中的逻辑模型名是 **`decision`**。它适合判断一小批非紧迫材料是否值得现在综合，不用于判断文件有没有变化，也不能代替来源、权限或调度校验。

- 默认策略建议 `auto`：先本地确定性 gate；有非紧迫工作时，使用当前 requirements 做 `route.resolve(ApiType::Decision, logical_model="decision")`。成功后将 `selected_exact_model` 交给 `DecisionEvaluateRequest`，不能把逻辑名直接填入 `exact_model`。
- 输入仅传有界摘要：数量、最老材料时间、有限摘录、变化对象、上次真实提交、是否存在冲突/到期。建议最多 8 KiB，不发送全量历史。摘要和原始材料保持来源标记。
- 首版一题 Boolean `should_consolidate_now`。输出是 `DecisionAnswer::Boolean{id, probability_true, confidence?}`，不是自造的 `{run:true,reason:...}`；只处理 Succeeded，复用 `validate_answers` 校验完整性、题型和概率。OpenRouter 若填写 criteria，需同时给出 true/false 两侧。
- 建议保守阈值：仅 `probability_true <= 0.1` 且返回 `confidence >= 0.8` 时允许跳过非紧迫工作；其余（含 confidence 缺失）完整整理。概率未经校准，阈值是可调策略。§6 的最大拖延和紧迫项覆盖模型的否决。
- provider 未配置、空 inventory、能力不匹配、超时、限流、非法输出或上游 build 不受支持时，回到正常整理路径；不隐式改用普通聊天大模型做前置判断，不因预检坏掉而停掉整理。
- 在宿主独立异步任务中执行预检，不能把耗时网络调用塞进现有 `AdmissionCheck`/顺序 reconcile 阻塞其他 Goal。结果绑定输入版本、Goal revision/epoch 与 slot；消费前重新核验，过期结果不得压住新材料。完成或超时后通过同一次 scheduled-fire 入口重验状态/预算并投递，不借 Manual 绕过准入，也不等下一个 30m tick；已走 fallback 的请求，其晚到结果不得再次改变决策。
- 建议一次机会最多 1 次可能计费的 provider attempt，本地等待上限 3 秒覆盖 route.resolve + evaluate 全过程；不能把“一次 RPC”当成“服务端只尝试一次”，需核对服务端 failover 策略。无法限制服务端尝试次数时关闭自动预检。预检有单独每日调用/费用限额并计入后台总预算，记录模型、请求键、usage/cost、耗时、失败和跳过原因。未知费用不能当免费；没有可接受的报价/硬预算控制时可关闭自动预检，正常整理仍工作。
- **现有接口限制必须处理**：`DecisionEvaluateRequest` 没有 per-call token/cost/timeout policy，evaluate 的 exact 路由不自动继承之前 route.resolve 的预算参数；resolve 不是预算预留。现有 adapter 也无 cancel，本地超时不保证远端停止或免计费。若要求硬费用上限，先补通调用链或禁用这一可选优化。
- 宿主预检不自动进入 `runner/goal.rs::usage_since` 的 Session `usage.jsonl`，必须单独持久记账再汇总。重复输入摘要可用缓存，但要重新核对最旧积压和最大拖延截止，false 结果不能跨时间窗口永久复用。重启不得盲目重复请求；in-flight 结果未知时回退主流程，不无限重发。幂等键不代表 provider 承诺不重复收费。
- 预检没有写权限：不处置感知、不刷新整理水位、不创建认知/Goal/日程、不发送确认通知。禁止通过预检响应直接执行任意命令。

## 8. 实施任务与依赖顺序

### T01 — 冻结最小契约与 fixtures

- [ ] 核对本 TODO 引用的源码，冻结 builtin key、配置、认知新增字段、候选/receipt schema、30m/2h slot 语义、默认预算。
- [ ] 为 §3 三类认知、§5 五类动作、四类触发和确认分支准备原始感知 fixture；包含来源、范围和明确/推断的区别。
- [ ] 列出 schema/协议影响面：Memory replay/派生索引/文本与 JSON 读取、SessionState/GoalState/Registry、kRPC/CLI、schema 导出和 SDK 消费方。保持单一真相源，不引入依赖或通用新服务。

### T02 — 接通可信感知与认知模型

- [ ] 将 Runner/CLI 的感知入口接入 Memory facade，补来源、主体、scope、anchors、cites 和幂等；自动 run 摘要仍按 Runtime 证据处理。
- [ ] 将真实 Session 的 recall/read 和推理前 Memory 装配接 `Memory::query_topic/query/read` 与 `ObservationState` 观察路径，绑定可信 Caller grants/base scope，观察状态随 Session 持久化。只有模型输入可靠提交后才接受新 snapshot/read_set，失败显露 unavailable、不推进水位；与 Memory S 阶段接线任务合并实施，不能只验证库级读取。
- [ ] 在 `agent_memory/model.rs`、`state.rs`、`recall.rs` 及 `lib_opendan/memory` 中完成创建时间投影、稳定度、独立证据时间、到期复核与读取展示。
- [ ] 收口普通 Session 的 `agent-memory` 写能力；管理写入明确授权，整理写入必须经过 Consolidator 和当前 lease。

### T03 — 接通整理工具与可恢复提交

- [ ] 按 §4 增加宿主工具、批次上下文上限、receipt、提交/候选发布/清理恢复；旧 report 成功不能再推进整理游标。
- [ ] 整理 lease 按业务能力绑定，补跨 epoch 排除、自身感知禁写及丢锁拒绝。
- [ ] 在默认 Goal 完整接通后删除对应 `create_self_improve_session/perception_window/旧 cursor` 写链、SelfImprove 模板及 Jarvis 两个旧 behavior；同步协议/CLI/WebUI 中的引用。保留有效测试场景并迁到 Goal，不以兼容分支维持双写。
- [ ] `self_check`、Notebook 的全面退役、其他结晶 Goal 按各自 TODO 处理；新功能不再增加它们的依赖。

### T04 — 默认播种与自适应检查

- [ ] 修改 `opendan` 配置/Loader、GoalHost 和 Jarvis 源模板，接稳定内置目标、独立 behavior、30m机会/2h基线、廉价摘要与恢复状态。
- [ ] 保证 pause/archive/disable、用户修改、升级、重启、多 Agent 隔离和现有通用 Goal 的行为正确；不修改其他 Goal 的默认周期。
- [ ] 增加 status 摘要：最近检查/真实整理、下次机会、skip 原因、材料/候选计数、错误与预算；不依赖读完整 worklog 才知道为什么没运行。

### T05 — 发现、确认与正式对象落地

- [ ] 实现 §5 类型化候选、去重、已有对象关联、问题关联、批准版本和可重入动作。
- [ ] 复用通知回答链路，补窄的受信落地入口；新 Goal 定义包含 scope、lifecycle、健康/完成条件、预算、停止规则和来源。
- [ ] 先补 workflow 创建日程的持久幂等合同，再打通已可执行的提醒/固定 workflow；未接通的 Agent Task 明确 blocked。生产 `opendan.command/agent.delegate` + Dispatcher 接入作为单独子任务，以实际执行验收后才开放。
- [ ] UI 显式计划请求复用该流程，不能把必须执行的指令仅写成感知后等待默认整理。

### T06 — 可选 decision 优化

- [ ] T02—T05 在无 decision 模型时已能工作，再增加 §7 的异步预检、能力探测、阈值、持久缓存、退避、独立计账与硬预算边界。
- [ ] 启用/禁用 decision 不改变整理语义、授权语义和最大拖延窗口，只改变成本与运行次数。

### T07 — 文档、手册与交付

- [ ] 更新 Session/Memory 协议、schema/fixture 生成源和实现进度；同步 `opendan_agent_dev`，必要时 `ui_core`/`xllm` 手册，避免留下失效命令或旧 Notebook 依赖。
- [ ] 修改 Jarvis `agent/` 源模板，使用正常打包流程；不只改 `rootfs`/`dapp_dist` 生成副本。
- [ ] 交付说明列出真实可运行的动作类型、仍 blocked 的生产依赖、实测 provider/时区/环境和未验证项。

## 9. 验收标准

| 编号 | 场景 | 必须断言 |
|---|---|---|
| V01 | 首启、重启、并发 reconcile、两个 Agent | 每 Agent 只有一个 builtin Goal；同 epoch 一个 Session；无材料不调用模型 |
| V02 | pause/archive/disable 后重启，用户修改预算 | 不复活、不重建、不覆盖用户设置；恢复沿既有生命周期 |
| V03 | UI/Work 产生跨 Session 感知 | 形成实际 MemoryItem/版本；另一真实 Session 的模型输入能收到认知与纠正、越权内容不可见；读取呈现主谓宾/范围/证据；库级 get 或模型回答“已记录”都不算闭环 |
| V04 | 重放、截断批次、证据不足、纠正 | 不重复强化；未读材料不处置；暂缓到期再选；纠正不被高稳定度压制 |
| V05 | 虚拟时钟推进 | created_at 不变；改 tag 不刷新 last_evidence_at；三类稳定度差异；explicit 不淡出、valid_until 仍到期 |
| V06 | 2h/30m 机会、远期日程、等待确认 | OR 语义正确；无变化只廉价检查；不重复提问；计划按自己的时刻执行 |
| V07 | 同时 tick、长 Activation、停机、回拨 | 单整理者；错过槽合并；pending fire 可恢复；无自身 run_digest 导致的持续激活 |
| V08 | Memory/计划读取异常、预算耗尽、清理失败 | 不伪装为空；不丢材料；清理可无模型重试；预算阻塞可见且不扩大额度 |
| V09 | prepared/Memory commit/候选发布/cleanup 各阶段崩溃 | 无材料丢失、重复认知或重复候选；lease 旧 epoch 被拒；新 epoch 接管未完成发布 |
| V10 | 明确偏好、短期任务、模糊愿望、持续职责 | 认知/Task/候选/Goal 分类正确；不把单次要求推成永久偏好，不把愿望当授权 |
| V11 | approve/reject/ignore/timeout、提案改版、无通知服务 | 批准只覆盖对应版本；未回复不执行；不持 lease 等待；同问题不轰炸用户 |
| V12 | 创建成功但回执丢失、重复回答、restart | 正式 Goal/日程只创建一次；已有等价对象可 linked；拒绝旧授权和越权创建 |
| V13 | 绝对/间隔/日历/条件触发、时区、过期/取消 | 相对时间以原始表达为锚；未知/不支持不伪成功；确定提醒不依赖整理轮询；条件等待不滥用 Work |
| V14 | decision 不可用/超时/非法/true/灰区/false | 正确 fallback；强制项绕过否决；最大拖延到期仍处理；skip 不处置材料 |
| V15 | 预检重复 slot、摘要更新、费用未知/超额 | 有界输入；不阻塞其他 Goal；超时立即 fallback、晚到结果丢弃；缓存不越过最大拖延；限次/审计/预算生效；本地 timeout 不冒充远端 cancel |
| V16 | workflow 生产接入（独立依赖） | 真正送达并执行提醒/Work/Goal fire；TaskMgr2 授权/绑定/结果正确；RunOnce 不逐槽补发；分页/类型/时区问题有验证 |
| V17 | 不记忆策略、CLI 越权、无效来源 | DoNotRecord 内容不进入感知/认知/整理 receipt，正式 Task 修改仍送达；普通 Session 直接写认知被拒；来源未知不能提升为用户明确授权 |

在 `buckyos/src/` 下按实际修改范围执行以下现有入口，并新增覆盖上述场景的有意义测试：

```bash
cargo test -p agent_tool agent_memory -- --test-threads=1
cargo test -p libopendan --test memory_component -- --test-threads=1
cargo test -p libopendan --test memory_sessions -- --test-threads=1
cargo test -p libopendan --test goal_session -- --test-threads=1
cargo test -p libopendan --test notify -- --test-threads=1
cargo test -p libopendan --test xagent_cli -- --test-threads=1
cargo test -p opendan -- --test-threads=1
# 仅在接 decision / workflow 时追加
cargo test -p buckyos-api --test aicc_decision_test
cargo test -p aicc decision
cargo test -p workflow -- --test-threads=1
```

Mock/虚拟时钟用于确定性和崩溃测试；集成验收另外验证真实 Jarvis 默认启动、实际认知读取、通知回答及真正的计划执行。真实 decision provider 未配置时应证明 fallback 正常，不能把 mock 通过写成 provider 实测通过。

本次文档交付仅核对源码、文件路径与 Markdown 差异；未运行以上测试，未启用任何默认 Goal 或真实计划任务。
