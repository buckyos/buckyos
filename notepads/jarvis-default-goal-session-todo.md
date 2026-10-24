# Jarvis 默认 Goal Session：感知整理、认知维护与计划发现 TODO

日期：2026-10-23

状态：**P0 已实施（2026-10-23，未提交 git，DV 未验收）**，实施记录见 §11；P1、P2 待做。§0–§10 保留为方案与验收标准，与代码不一致处以 §11 为准。

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
8. Agent home page（OpenDAN WebUI）增加 Goals 管理入口：列出每个 Goal 的状态、最近 Activation、下次机会与 skip 原因，详情页按 epoch 看 Activation 并可跳到 Session；这是观察默认整理 Goal 是否在工作的主要手段，随 P0 一起交付（§2.1）。

以下新增字段、工具名、配置名均为实施建议；在 T01 冻结具体命名。默认阈值是首版可配置策略，不是对认知可靠性的科学标定。

实施按 §8.0 的三个切片推进：P0 先让默认 Goal 在真实 Jarvis 上零模型调用地播种、按 gate 运行并可靠提交认知；P1 做发现与确认；P2 才是 workflow 落地与 decision 预检。2026-10-23 review 补充的落点决定（存储位置、标记载体、lease 时机、gate 落点、schema 版本、时区来源、删除范围）分别写在各节的"**落点决定**"段，T01 只需核对命名，不再重新选型。

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
| 创建权限 | `GoalStore::create` 当前允许 Owner/UI，拒绝 Goal Session 创建其他 Goal。`GoalCreate.key` 已是创建方幂等键；`GoalCaller` 不带 `session` 即按 Owner 处理；`GoalCreatedBy.via` 注释已预留 `opendan`；`GoalCreate` 没有 `extensions` 字段 | 不能让整理 Goal 冒充 Owner；播种与标记写法见 §2 落点决定；需窄的“已授权候选落地”入口 |
| WebUI | [opendan/web](../src/frame/opendan/web/src/) 只有 Home 与 Sessions / Agent State / Loader 三个 tab；[service.rs](../src/frame/opendan/src/service.rs) 已对 owner 放行只读 `goals.list/get/revision/changes/state/epoch_state/activations/schedule` 和操作 `goals.activate/goals.post`，`goals.set_status/reopen/restart` 只对 Agent 自身的 App 开放 | Home 增加 Goals 区域与 Goal 详情页（§2.1）；owner 的 pause/resume/reopen/restart 需要新的受信操作入口 |
| decision | [aicc_client.rs](../src/kernel/buckyos-api/src/aicc_client.rs)、[aicc_decision.rs](../src/kernel/buckyos-api/src/aicc_decision.rs)、AICC 路由已有 `decision.evaluate` | OpenDAN/Jarvis 尚未调用；逻辑模型名存在不代表有可用 provider |

计划任务当前能力与阻塞见 §5.3。旧 `self_check.toml` 的业务分类可以参考，但它仍依赖 Notebook 和旧模板，不可直接作为新 Goal 的执行手册。

## 2. 默认 Goal 的身份、职责与生命周期

- 一个 Agent 一个内置目标，使用 Agent 身份 + builtin key 的确定性幂等键；不要用显示名 `Jarvis` 或标题判断唯一性。Loader 重启、升级、双重 reconcile 都不得重复创建。
- 目标定义：`lifecycle=maintain`；健康状态是“已到处理条件的感知被可靠处置，认知可追溯且按时复核，发现的事项有明确去处”。距离至少统计新材料、到期暂缓、清理失败、待复核认知、待确认候选、待落地动作及最老积压年龄。
- 增加显式 builtin 标记、策略版本、整理能力与 `records_perception=false`。这应是受信配置/宿主能力，模型不能通过改名字获取。

**落点决定（播种与标记）**

- 播种入口：Loader 启动 GoalHost 后、第一次 tick 前，用 `GoalCaller::owner(<Agent DID>)` 调用现有 `GoalStore::create`，`GoalCreate.key = "builtin:memory_maintenance"`（Agent 目录内唯一即可，不含显示名），`created_by.via = "opendan"`。同 key 返回同一 Goal，这就是幂等；不新增 kRPC。播种在宿主进程内直接走 facade，不经 `goals.create` kRPC，UI 调用方不能传 builtin 标记。
- 标记载体：`GoalDefinition.extensions["opendan"]`，内容建议为 `{"builtin":"memory_maintenance","policy_version":1,"capabilities":["memory_consolidation"],"records_perception":false}`。为此 `GoalCreate` 增加 `extensions: BTreeMap<String, Value>` 字段，`GoalStore::create` 只在 `CallerKind::Owner` 且 `caller.session` 为空时接受 `opendan` 键，UI/Goal/Work 调用方传入即拒绝；`GoalRevise` 不允许改 `opendan` 键（升级 `policy_version` 由宿主播种逻辑用同一 Owner 路径 revise）。
- Runner 识别：Goal Session 的 `SessionSpec.goal: GoalBinding` 已带 `goal_id`（[goal.rs](../src/frame/lib_opendan/src/goal.rs) `ensure_goal_session`）。drive 阶段按 `goal_id` 读取 `GoalDefinition.extensions.opendan`，得到 capabilities 与 `records_perception`，不把标记复制进 SessionConfig，避免两份真相。`ControlCommand::Perceive` 在 `records_perception=false` 时返回明确错误而不是静默丢弃。
- `extensions.opendan` 的 schema 作为 Goal 协议的一部分写进 protocol/schema 导出；WebUI 以 `builtin` 字段区分内置 Goal 的展示（§2.1）。

### 2.1 Agent Home Page 的 Goals 管理入口

用户要能在 Agent 的 home page 看到每个 Goal 的执行情况，这是观察默认整理 Goal 是否真的在工作的主要手段，与 T04 的 status 摘要一起交付。要求如下：

- **Home 的 Goals 区域**：与现有 `Overview / WorkspaceList / RuntimeList` 同级，列出该 Agent 的全部 Goal：标题、`lifecycle`、状态（active/paused/achieved/archived）、是否内置、最近一次 Activation 的时间与 `decision`、下一次机会（来自 `goals.schedule` 的 `every_secs/last_slot` 与 §6 gate 状态）、最近 skip 原因、待确认候选数、`last_error`。数据只用现有只读 kRPC `goals.list/state/schedule`，不新增汇总接口；数量多时按 active 优先、分页或折叠。
- **Goal 详情页**：新增 hash 路由 `#/goal/<goal_id>`（与 `#/session/<sid>` 同一机制，不加顶层 tab）。内容分块：定义与修订（`goals.get/revision/changes`）、Goal State（distance、pending、questions 及其通知状态、active_works、usage 与预算余量）、按 epoch 分组的 Activation 列表（`goals.activations`，每条显示 reason、outcome、decision、token/runs、关联 Session 链接）、内置整理 Goal 额外显示 §5.1 的候选列表与 §6 的 gate 摘要（last_checked/last_baseline/skip reason/pending_work 计数/最旧积压）。Activation 行链接到对应 Session 详情，复用 `SessionDetailPage`。
- **操作**：首期提供 `activate`（现有 `goals.activate`）和 pause / resume / archive / reopen / restart。后五个当前 `goals.set_status/reopen/restart` 只对 Agent 自身 App 放行，需要在 [service.rs](../src/frame/opendan/src/service.rs) 为 owner 增加受信操作（`is_operation` 加入 `goals.set_status/goals.reopen/goals.restart`，caller 为 `GoalCaller::owner(owner)`，`via` 记为 owner）。不提供 create/revise 表单：用户建 Goal 走 UI Session 的意图分析，改定义走 `agent-session goal revise`。
- **内置 Goal 的特殊展示**：标出“系统内置”，禁用 archive 按钮只留 pause/resume（archive 后重启不复活，用户容易误以为坏了），显示 `policy_version` 与配置开关状态。
- **i18n**：en/zh 两套文案走现有 `i18n.ts`；`model/types.ts` 的 `SessionKind` 随 T03 删除 `self_improve`（`self_check` 按 §8 T03 第 4 条暂留）。
- **验证**：`pnpm build` 与 `pnpm test:e2e`（mock 数据）覆盖列表、详情、activate；真实 Zone 验收见 §10。
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

**主语与范围规则（用户 2026-10-23 确认）**

- 认知的主语优先是三类主体：**Agent 自己**（它的习惯、能力边界、曾做过的选择）、**Agent 的 Owner / 服务对象**（偏好、要求、纠正）、**某个工作区**（项目约定、环境经验、工具用法）。这三者是 Agent 会主观改变、也只有通过记忆才能保持连续的东西。
- 其它对象（外部服务、文件内容、第三方的状态、日程、价格、在线状态）属于客观世界：Agent 需要时去观察，不靠记忆替代。这类内容若进入 Memory，只能作为“曾观察到什么、在哪里能再查到”的线索，`stability` 不高于 `contextual`，通常带 `valid_until` 或 `review_at`；整理时不得把一次观察提升为稳定事实。
- 落点：`default_subjects` 与候选 `scope` 的取值空间限定为 Agent DID、用户 principal、工作区 id 三类引用；整理模型给出的主语不属于这三类时，提交校验要求它要么挂在这三类主体之一的属性/关系上（如“工作区 A 里的工具 X 需要显式输出目录”，主语是工作区 A），要么作为低稳定度的世界观察线索，不能成为独立的全局事实。M-34 的“不为只被顺带提到的人建立印象”由此自然满足。
- fixture 相应补一组反例：对第三方对象的直接断言（如某网站的当前价格）应被整理为带有效期的观察线索或直接丢弃，不形成 `stable` 认知。

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

**落点决定（schema 版本与缺失字段）**

- `agent_memory` 的 `SCHEMA_VERSION` 从 `3.0` 升到 `4.0`，不走 `3.1`：TD-24 规则下次版本不一致只能只读挂载，整理 Goal 对旧根目录就无法提交，等于没用；主版本升级则明确拒绝旧根目录，DV 按 `start.py --all` 全新安装（见 [DV 可全新安装原则](../AGENTS.md)），不写迁移。
- `last_evidence_at` 对新建条目必填（由提交校验拒绝缺失）；`stability` 缺省 `contextual`；`review_at` 可选。读取投影遇到历史上缺 `last_evidence_at` 的条目（只会出现在测试 fixture 或 4.0 之前的 envelope 重放）时取 `first_occasion` 的时间，即按创建时间算龄，永不取 `noticed_at` 或当前时间。
- 字段加在 `ItemMeta`/`MemoryItem`，`UpsertRelationOp`/`PutItemOp` 的元数据一并带；文本与 `--json` 读取输出各加 `STABILITY`、`LAST_EVIDENCE`、`REVIEW_AT` 行；`FadeConfig` 改为按 `stability` 索引的三组 `days_per_point`。
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

**落点决定（工具注册、lease 时机、批次持久化）**

- 工具注册：Runner 当前只有 `report` 一个按名字拦截的宿主工具（[runner/tools.rs](../src/frame/lib_opendan/src/runner/tools.rs) `call_tool`），behaviors 没有 capability 概念。做法是 `SessionToolManager` 增加一组按名字拦截的宿主工具，仅当 drive 阶段从 `GoalDefinition.extensions.opendan.capabilities` 读到 `memory_consolidation` 时才把 `memory_consolidation_begin/commit` 加进工具清单并暴露 schema；其他 Session 调用同名工具按 unknown tool 处理。不扩展 behavior 文件格式，不新增 capability 配置层。
- lease 时机：沿用 [runner/drive.rs](../src/frame/lib_opendan/src/runner/drive.rs) 现有 kind lease 的位置，但判定条件从 `SessionKind::SelfImprove` 改为“Goal Session 且 capabilities 含 `memory_consolidation`”，lease 名 `memory_consolidation`（`CONSOLIDATION_LEASE` 同步改名，`Consolidator` 的 resource 校验随之变）。Busy 时沿用 `DriveResult::Busy`，Activation 尚未打开、不消耗预算；宿主在 Goal status 里记一条 `consolidation_busy` 并等下一个 tick。drive 结束即释放，天然满足“等待用户/休眠时不长持”。不在 begin 工具里再抢锁。
- 已知风险的处理：Memory TODO §8.5 记录“锁文件被删重建后 epoch 从 1 重计，提交被 `StaleLease` 拒绝”。本次不做自动修复：`commit` 返回 `StaleLease` 时 Activation 以 `failed` 结束，`last_error` 写明需要人工修复锁文件，status 摘要显露；不自动删锁、不自动 bump epoch。
- 批次持久化：`Batch` 已 `Serialize/Deserialize`，receipt 直接保存完整 `Batch`（约 20 条材料加 context，有 §4 第 4 点的上下文上限兜底），恢复时反序列化后原样传给 `Consolidator::commit`，不改 commit 签名，refs ⊂ batch 与 lease 校验照旧生效。
- 多批次：一个 Activation 内按批次顺序 begin → commit，每批一张 receipt；允许跨 run。`memory_consolidation_begin` 在本 Activation 已有 3 张 receipt 或存在未完成 receipt 时拒绝，返回原因让模型直接 report。
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

**落点决定（候选与 receipt 的存储、时区来源）**

- 存储位置：`GoalState` 增加 `maintenance: Option<MaintenanceState>` 字段（`protocol/goal.rs`），内含 `candidates`、`receipts`（§4.1）、gate 的检查水位摘要（§6）。它随 Session 的 `state.json.goal.state` 一起 commit，再按现有顺序 publish 到 `state/goals/<id>/state.json`，不建第二个文件、不另起写者。`GoalState::for_epoch(StateKeep::Facts)` 必须把 `maintenance` 列入保留项，否则 restart 会丢候选与未完成 receipt。
- 跨 epoch 恢复为什么够：`GoalHost::reconcile` 在前一 epoch 的 Session 未 `settled`（`needs_flush`）时不创建新 Session，所以新 epoch 通过 `prepare_epoch` 拿到的已发布 GoalState 一定包含最后一次 commit 的 `maintenance`。唯一例外是旧 Session 目录丢失（`lookup` 返回 None 直接 `end_epoch`），此时按已发布版本恢复，未发布的 receipt 视为未提交，Memory 侧用同一幂等键重放即可。
- 候选 ID 用 `cand-` + 稳定键的 hash；`questions` 投影只放 `status=needs_confirmation` 且未展示过的候选，`GoalQuestion` 增加可选 `candidate_id` 字段把两者关联，回答 Activation 据此找到候选。
- 时区来源按顺序取：感知记录 `anchors.intent_timezone` → `anchors.timezone`（[protocol/agent_state.rs](../src/frame/lib_opendan/src/protocol/agent_state.rs) `PerceptionAnchors`）→ 用户时区系统源（`protocol/config.rs` 的 `USER_TIMEZONE_SUBSCRIPTION`，`source = system:user_timezone`，Goal Session 读该源的当前值）→ 都没有则进 `missing_fields`，候选停在 `needs_confirmation`。解析后的绝对时刻与所用时区、来源三者一起存进候选。

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

**落点决定（gate 的位置、持久化与廉价化）**

- 位置：gate 放在宿主 [host/goals.rs](../src/frame/lib_opendan/src/host/goals.rs) 的 `reconcile` 中，作为内置 Goal 专用的一步，在 `post_activation` 之前对 `Created/Reopened/Restarted` 与 `Scheduled` 四种 reason 统一生效；不做成 `AdmissionCheck`（它只在定时 fire 链上，拦不住 start 事件），也不改 `LocalScheduleBackend`。普通 Goal 不经过这一步。
- 持久化：gate 水位（`last_checked_slot/last_baseline_slot/last_successful_consolidation_at/last_skip_reason/last_inputs_digest/backoff_until`）写在 `schedule.json` 新增的可选 `gate` 字段里，host 是 `schedule.json` 的既有写者，不新开文件；真正提交后的水位由 Session 写进 `GoalState.maintenance`，两者分别对应“检查过”和“整理过”。
- 槽位：`schedule.every_secs = 1800`；基线槽为 `slot % 4 == 0`（UTC 对齐的整 2 小时）。创建后的首次检查用 `Created` 事件触发，不等槽。
- 廉价化：`pending_work()` 要 replay Graph（TD-22 仍是全量），而 `tick` 对所有 Goal 串行持 `busy` 锁。gate 先用 A.9 的快速路径（`memory_snapshot` 版本向量，只看 occasions 与感知文件末行，不取写锁）比对 `last_inputs_digest`；版本未变且未到 `next_deadline`、无 `review_at` 到期、无候选变化、无未完成 receipt 时直接记 `no_change` 跳过，不调 `pending_work`。只有版本变了或到期才做完整确定性检查。到期认知复核与业务日程摘要各有独立查询，不假设 `pending_work` 覆盖。
- 出错：快速路径或 `pending_work` 返回错误时记 `unknown` 并有限退避（建议 5m→15m→30m 上限），不跳过也不进入整理；status 摘要显露错误与下次重试时间。

## 7. 可选 decision 快速前置判断

用户所说的 `decsion` 在当前代码中的逻辑模型名是 **`decision`**。它适合判断一小批非紧迫材料是否值得现在综合，不用于判断文件有没有变化，也不能代替来源、权限或调度校验。

- **首版默认 `off`**（2026-10-23 review 决定）：本节自己列出的限制（无 per-call 预算、无 cancel、费用未知、服务端 failover 不可控）在当前代码下都成立，`auto` 几乎必然落入关闭分支；因此 P0/P1 不实现本节，只保留配置开关位 `[session.goal.maintenance] decision = "off" | "auto"`，T06 全部内容在开关为 `auto` 时才需要。下面的 `auto` 设计保留作 T06 的依据。
- `auto` 策略：先本地确定性 gate；有非紧迫工作时，使用当前 requirements 做 `route.resolve(ApiType::Decision, logical_model="decision")`。成功后将 `selected_exact_model` 交给 `DecisionEvaluateRequest`，不能把逻辑名直接填入 `exact_model`。
- 输入仅传有界摘要：数量、最老材料时间、有限摘录、变化对象、上次真实提交、是否存在冲突/到期。建议最多 8 KiB，不发送全量历史。摘要和原始材料保持来源标记。
- 首版一题 Boolean `should_consolidate_now`。输出是 `DecisionAnswer::Boolean{id, probability_true, confidence?}`，不是自造的 `{run:true,reason:...}`；只处理 Succeeded，复用 `validate_answers` 校验完整性、题型和概率。OpenRouter 若填写 criteria，需同时给出 true/false 两侧。
- 建议保守阈值：仅 `probability_true <= 0.1` 且返回 `confidence >= 0.8` 时允许跳过非紧迫工作；其余（含 confidence 缺失）完整整理。概率未经校准，阈值是可调策略。§6 的最大拖延和紧迫项覆盖模型的否决。
- provider 未配置、空 inventory、能力不匹配、超时、限流、非法输出或上游 build 不受支持时，回到正常整理路径；不隐式改用普通聊天大模型做前置判断，不因预检坏掉而停掉整理。
- 在宿主独立异步任务中执行预检，不能把耗时网络调用塞进现有 `AdmissionCheck`/顺序 reconcile 阻塞其他 Goal。结果绑定输入版本、Goal revision/epoch 与 slot；消费前重新核验，过期结果不得压住新材料。完成或超时后通过同一次 scheduled-fire 入口重验状态/预算并投递，不借 Manual 绕过准入，也不等下一个 30m tick；已走 fallback 的请求，其晚到结果不得再次改变决策。**这个“scheduled-fire 回投入口”当前不存在**（host 只有 Manual 的 `goal_activate`），属于 T06 新增接口：建议为 `GoalHost` 增加 `pub(super) async fn resume_gate(&self, sup, goal_id, token)`，由预检任务完成时调用，内部重跑 §6 gate 并复用 `pending/settle` 的 fire key；T06 之前不定义。
- 建议一次机会最多 1 次可能计费的 provider attempt，本地等待上限 3 秒覆盖 route.resolve + evaluate 全过程；不能把“一次 RPC”当成“服务端只尝试一次”，需核对服务端 failover 策略。无法限制服务端尝试次数时关闭自动预检。预检有单独每日调用/费用限额并计入后台总预算，记录模型、请求键、usage/cost、耗时、失败和跳过原因。未知费用不能当免费；没有可接受的报价/硬预算控制时可关闭自动预检，正常整理仍工作。
- **现有接口限制必须处理**：`DecisionEvaluateRequest` 没有 per-call token/cost/timeout policy，evaluate 的 exact 路由不自动继承之前 route.resolve 的预算参数；resolve 不是预算预留。现有 adapter 也无 cancel，本地超时不保证远端停止或免计费。若要求硬费用上限，先补通调用链或禁用这一可选优化。
- 宿主预检不自动进入 `runner/goal.rs::usage_since` 的 Session `usage.jsonl`，必须单独持久记账再汇总。重复输入摘要可用缓存，但要重新核对最旧积压和最大拖延截止，false 结果不能跨时间窗口永久复用。重启不得盲目重复请求；in-flight 结果未知时回退主流程，不无限重发。幂等键不代表 provider 承诺不重复收费。
- 预检没有写权限：不处置感知、不刷新整理水位、不创建认知/Goal/日程、不发送确认通知。禁止通过预检响应直接执行任意命令。

## 8. 实施任务与依赖顺序

### 8.0 切片与顺序

T01–T07 合在一起等于 Memory S 阶段加 Goal 发现加 workflow 加 decision，不能一次交付。按下面三个切片推进，每个切片单独验收、单独提交；后一切片开始前，前一切片必须在真实 Jarvis 上跑通（§10）。

| 切片 | 内容 | 不包含 | 验收 |
|---|---|---|---|
| **P0 默认整理 Goal 跑通** | T01；T02 的感知入口接 `Memory::record_perception` 与认知新增字段（§3）；T03 全部；T04 全部；T07 中与之对应的文档与 Jarvis 模板；§2.1 的 Goals 列表与详情页（只读 + activate） | T02 的 recall/observe 接线（单独 TODO，见下）；候选与确认；workflow；decision | V01、V02、V04、V05、V07、V08、V09、V17 及 V03 的“形成实际 MemoryItem/版本、读取呈现主谓宾”部分；§10 的 DV 步骤 1–5 |
| **P1 发现与确认** | T05 中候选契约、去重、问题关联、批准版本、`needs_confirmation` 全流程与 UI 显式计划入口；§2.1 的候选展示与 owner 的 pause/resume/reopen/restart 操作 | 任何正式日程/Work 的自动落地（候选最高到 `approved`，落地动作一律 `blocked` 并说明缺 P2） | V06、V10、V11、V12、V13 中不依赖 workflow 的断言 |
| **P2 生产落地与可选优化** | T05 中 workflow 幂等合同、提醒/固定 workflow 落地、`opendan.command` + Dispatcher 接入；T06 | — | V14、V15、V16 与 V12/V13 其余断言 |

- T02 第二条（真实 Session 的 recall/read/observe 接线）是 Memory S 阶段的主体，与本 Goal 正交；从本文移出，另立 TODO 实施，P0 不等它。本文对它的唯一要求是 V03 中“另一 Session 的模型输入能收到认知与纠正”在该 TODO 完成后补验。
- 切片内部顺序：T01 → T02（P0 部分）→ T03 → T04 → §2.1 → T07。T03 先于 T04，使播种出来的 Goal 第一次运行就有真正的 begin/commit 工具，不出现“播种了但只能 report”的中间态。

### T01 — 冻结最小契约与 fixtures

- [x] 核对本 TODO 引用的源码，冻结 builtin key、配置、认知新增字段、候选/receipt schema、30m/2h slot 语义、默认预算（2026-10-23，命名见 §11.1）。
- [x] 用户已确认（2026-10-23）：(a) 感知 `Caller` 的 `default_subjects/grants` 最小规则：UI Session 取对话 peer 的 principal，Work 取 Origin 创建者，Goal 取 Goal owner，grants 为同一主体加 Agent 自身，跨主体可见性按 Memory 附录 A.3.7；(b) `agent_memory` schema 升 4.0 拒绝旧根目录（§3.2）。
- [x] 用户同时给出**主语原则**：认知的主语主要是三类——Agent 自己、Agent 的 Owner、某个工作区，因为只有这三者是 Agent 会主观改变的东西；其它对象基本属于客观世界，总是需要观察而不是记住。已回写为 §3.1 的“主语与范围”规则，并约束 §5.1 的候选 scope 与 T02 的 `default_subjects` 取值。

- [x] §3 三类认知的 fixture：`agent_memory/tests.rs::schema_4_time_semantics_follow_stability_and_evidence`（stable / contextual / volatile + explicit）、`review_at_is_a_deterministic_due_query`；§5 五类动作、四类触发和确认分支的感知 fixture 随 P1 准备（候选契约已冻结为类型，未有数据）。
- [x] schema/协议影响面已逐项改动（§11.1）：Memory Graph schema 4.0（replay、派生索引不变、文本 / JSON 读取加字段）、GoalState / GoalSchedule / GoalQuestion / ControlCommand / SessionKind、kRPC（删 `perception.cursor/backlog/read`，`goals.create/revise` 拒绝 `extensions.opendan`）、CLI（`ctl perceive` 新参数、`agent-memory` 写门）、schema 导出（`goal_opendan_extension` 新增，`perception_cursor` 删除）、WebUI 类型。



### T02 — 接通可信感知与认知模型

- [x] 将 Runner/CLI 的感知入口接入 Memory facade，补来源、主体、scope、anchors、cites 和幂等；自动 run 摘要仍按 Runtime 证据处理（`runner/consolidation.rs::perceive` + `xagent ctl perceive`；§11.1）。`Caller.default_subjects` 按 T01 已确认的规则由宿主绑定（UI：对话 peer；Work：Origin 创建者；Goal：Goal owner），并在有 Workspace 绑定时附上工作区 id；grants 为同一主体集合加 Agent 自身。模型不能自行扩大主体集合。
- [ ] （移出本文，见 §8.0）真实 Session 的 recall/read 和推理前 Memory 装配接 `Memory::query_topic/query/read` 与 `ObservationState` 观察路径，绑定可信 Caller grants/base scope，观察状态随 Session 持久化。只有模型输入可靠提交后才接受新 snapshot/read_set，失败显露 unavailable、不推进水位。由 Memory S 阶段 TODO 实施，不能只验证库级读取；本文 P0 不依赖它。
- [x] 在 `agent_memory/model.rs`、`state.rs`、`recall.rs` 及 `lib_opendan/memory` 中完成创建时间投影、稳定度、独立证据时间、到期复核与读取展示（schema 4.0）。
- [x] 收口普通 Session 的 `agent-memory` 写能力：Session shell（StableEnv）内写动词一律 `ReadOnly` 拒绝；管理写入只在非 Session 环境；整理写入只经 `Consolidator` 与当前 lease。

### T03 — 接通整理工具与可恢复提交

- [x] 按 §4 增加宿主工具、批次上下文上限、receipt、提交/候选发布/清理恢复；游标已删除，report 不再推进任何整理状态。
- [x] 整理 lease 按业务能力绑定，补跨 epoch 排除、自身感知禁写及丢锁拒绝（`StaleLease` → receipt `failed` + `last_error`，不自动修锁）。
- [x] 在默认 Goal 完整接通后删除对应 `create_self_improve_session/perception_window/旧 cursor` 写链、SelfImprove 模板及 Jarvis 两个旧 behavior；同步协议/CLI/WebUI 中的引用。保留有效测试场景并迁到 Goal，不以兼容分支维持双写。**本次删除的明确清单**：`SessionKind::SelfImprove` 与 `si-` 前缀、`api.rs::create_self_improve_session`、`drive.rs` 的 `self_improve` kind lease 分支（改为 §4.1 的能力判定）、`outcome.rs` 的 `perception_window` 与游标提交、`state/cognition.rs::commit_consolidation`、`state/perception.rs` 的 `.cursor.json/backlog/commit_cursor` 与 kRPC `perception.cursor/perception.backlog`、`[loader.self_improve]`（opendan `config.rs` 的 deny_unknown_fields 结构与 Jarvis `agent.toml` 同时删）、`tests/self_improve.rs`（幂等/防回声/锁场景迁入 `goal_session.rs` 或 `memory_sessions.rs`）、Jarvis `self_improve_signals.toml` / `self_improve_set_memory.toml`、WebUI `SessionKind` 中的 `self_improve`、`CONSOLIDATION_LEASE` 改名。
- [ ] **本次不删**：`SessionKind::SelfCheck`、`self_check.toml`、`[loader.self_check]`、`_self_check_timer`、Notebook（`cognition.notebook_append`、`agent_notebook.rs`、`agent-notebook` CLI）、`agent_attention_signal.rs`。它们属于 Goal TODO §9.3 与 Memory TD-09/TD-20 的退役项，目前没有独立 TODO；本文新增代码不得依赖它们，退役时再另立 TODO。

### T04 — 默认播种与自适应检查

- [x] 修改 `opendan` 配置/Loader、GoalHost 和 Jarvis 源模板，接稳定内置目标、独立 behavior、30m机会/2h基线、廉价摘要与恢复状态（`host/maintenance.rs`，`[session.goal.maintenance]`，`behaviors/memory_maintenance.toml`）。
- [x] pause/archive/disable、重启、多 Agent 隔离、升级（`policy_version` 只 revise 标记）、通用 Goal 不受影响：`tests/memory_maintenance.rs`、`tests/goal_session.rs` 全部通过。
- [x] status 摘要：`schedule.json.gate` 与 `GoalState.maintenance`，经 `goals.schedule/goals.state` 读出；Goal 头另有 `<memory_maintenance …>` 块。
- [ ] 按 §2.1 实现 WebUI：Home 的 Goals 区域、`#/goal/<goal_id>` 详情页、activate 操作（实施中，结果见 §11.4）。owner 的 pause/resume/reopen/restart 不需要新入口：`goals.set_status/reopen/restart` 经 Agent State 门面以 `GoalCaller{session: None}` = owner 侧放行，服务端 `authorize_agent_method` 的既有单测 `read_only_agent_writes_only_its_own_state` 已覆盖这三个方法。

### T05 — 发现、确认与正式对象落地

- [ ] 实现 §5 类型化候选、去重、已有对象关联、问题关联、批准版本和可重入动作。
- [ ] 复用通知回答链路，补窄的受信落地入口；新 Goal 定义包含 scope、lifecycle、健康/完成条件、预算、停止规则和来源。
- [ ] 先补 workflow 创建日程的持久幂等合同，再打通已可执行的提醒/固定 workflow；未接通的 Agent Task 明确 blocked。生产 `opendan.command/agent.delegate` + Dispatcher 接入作为单独子任务，以实际执行验收后才开放。
- [ ] UI 显式计划请求复用该流程，不能把必须执行的指令仅写成感知后等待默认整理。

### T06 — 可选 decision 优化

- [ ] T02—T05 在无 decision 模型时已能工作，再增加 §7 的异步预检、能力探测、阈值、持久缓存、退避、独立计账与硬预算边界，以及 §7 末尾的 `resume_gate` 回投入口。默认配置保持 `off`。
- [ ] 启用/禁用 decision 不改变整理语义、授权语义和最大拖延窗口，只改变成本与运行次数。

### T07 — 文档、手册与交付

- [x] 更新 Session/Memory 协议（Agent State Protocol §4/§5/§9.7、Lease Protocol、Session Directory Protocol）、schema 重生成、`Agent Memory 认知管理需求.md` 进度行、两份 README；cmd_skills 手册无需改（`grep agent-memory` 在 `cmd_skills/` 与 Dev Guide 中无命中）。
- [x] 修改 Jarvis `agent/` 源模板（`agent.toml`、`behaviors/memory_maintenance.toml`，删两份 self_improve behavior）。
- [x] 交付说明见 §11.5。

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
| V18 | WebUI Goals 入口 | Home 列出全部 Goal 与内置标记、最近 Activation、下次机会、skip 原因；详情页按 epoch 显示 Activation 并能跳到 Session；activate 立即触发并刷新；pause 后 gate 显示 paused；mock 与真实 Zone 各验一次 |

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

## 10. DV 验收做法

在已激活的本机 Zone 上验收，先 `uv run src/check.py` 确认服务状态。涉及容器内 OpenDAN 的改动要先 `./build_aios --local-test` 再 `uv run src/start.py --skip-update`；只验宿主侧逻辑可用 `./debug_jarvis.sh devtest --agent <agent_id>` 前台跑 Loader（它会构建 opendan/libopendan/agent_tool_cli_dev 并接管该 Agent 的容器端口）。Memory schema 升 4.0 后旧 Agent 根目录会被拒绝，按 `uv run src/start.py --all` 全新安装并重新用 Add Agent 向导建 Agent，不手改 `meta.json`。

1. **播种与零模型调用**：Loader 启动后用 kRPC `goals.list` 应看到唯一一个 `extensions.opendan.builtin = "memory_maintenance"` 的 Goal；`goals.schedule` 的 `gate` 显示 `no_work`；Agent Session 的 `usage.jsonl` 没有新增记录。重启 Loader 两次再查，Goal 数不变、`goals.changes` 没有新的 `created`。
2. **注入材料**：在 MessageHub 与 Jarvis 对话产生一条带明确偏好的消息，再在该 UI Session 内让模型执行 `agent-session ctl perceive`（或由 `xagent_cli` 直接调用 `perception.append` 的测试路径）。`goals.schedule.gate` 在下一个 tick 变为 `active`，到 30m 槽或手动 `goals.activate` 后 `goals.activations` 出现 `decision=acted` 的记录，`GoalState.maintenance.receipts` 有已完成的 receipt。
3. **认知可读**：容器内运行 `agent-memory load --tag <...>`（管理读取）或经 `agent-session recall`，输出含 `REVISION/BASIS/SCOPE/STABILITY/LAST_EVIDENCE` 行；原感知在 `<agent_root>/state/perception/<sid>.jsonl` 中正文已替换为处置标记。模型回答“已记住”不算通过。
4. **gate 行为**：无新材料时连续两个 30m 槽 `gate.last_skip_reason = no_change` 且无 Activation；`pause` 后 gate 显示 `paused`；`resume` 后下一槽恢复检查。
5. **WebUI**：打开 Agent home page，Goals 区域出现内置 Goal 与其最近 Activation、下次机会；点进详情页看到 Activation 按 epoch 分组并能跳到 Session 详情；点 activate 后列表刷新。
6. **P1 起**：对话中说出“明天 9 点提醒我带护照”，详情页候选列表出现 `kind=schedule, status=needs_confirmation` 的候选，通知托盘收到带候选 ID 的确认问题；回答后候选变 `approved`，P2 之前落地动作显示 `blocked` 并注明原因。
7. **崩溃恢复**：用 `crate::fault::point` 现有机制在 `goal:before_session_create` 与新增的 receipt 阶段故障点上注入，重启后无重复认知、无重复候选、无丢材料。

证据按仓库惯例放 `notepads/evidence/jarvis-default-goal-session/<date>/`，记录命令、kRPC 返回与关键日志行。

## 11. 实施记录（2026-10-23，P0）

### 11.1 冻结的命名与落点

| 项 | 冻结值 / 落点 |
|---|---|
| builtin key / 标记 | `GoalCreate.key = "builtin:memory_maintenance"`；`extensions.opendan = {builtin: "memory_maintenance", policy_version: 1, capabilities: ["memory_consolidation"], records_perception: false}`（`protocol/goal.rs` `OpendanGoalExtension`，schema `goal_opendan_extension`）。`GoalCreate.extensions` / `GoalRevise.extensions` 新增；`FsGoals::create/revise` 只对 `CallerKind::Owner` 接受 `opendan` 键，kRPC `goals.create/revise` 带它一律 `forbidden`。 |
| 配置 | `agent.toml [session.goal.maintenance] enabled / decision ("off" \| "auto") / behavior`（`template.rs` `MaintenanceDefaults`；库默认 `enabled = false`，Jarvis 模板 `enabled = true`、`behavior = "memory_maintenance"`、`decision = "off"`）。`[loader.self_improve]` 删除；`loader.status.modules` 增 `memory_maintenance`。 |
| 认知新增字段 | `agent_memory` schema `4.0`：`ItemMeta/MemoryItem.{stability: stable\|contextual\|volatile（缺省 contextual）, last_evidence_at（新建整理认知必填，修订不给则保留）, review_at}`；`created_at` = `GraphView::item_created_at`（`first_occasion` 的时间，不存）；`FadeConfig{stable/contextual/volatile_days_per_point = 365/30/1, max_penalty 20}` 以 `last_evidence_at` 或创建时间为锚；`GraphView::due_reviews / next_review_at`；`load` 文本与 `RecallItem/LoadItem` 增 `CREATED_AT / STABILITY / LAST_EVIDENCE / REVIEW_AT`。旧主版本根目录拒绝打开（DV 全新安装）。 |
| 候选 / receipt | `GoalState.maintenance: Option<MaintenanceState>{receipts[], candidates[], last_successful_consolidation_at_ms, consolidations, operations, dispositions, abandoned_batches, last_error}`；`ConsolidationReceipt{operation_id "mc-<24hex>", epoch, activation, batch_id, graph_seq, refs[], batch(完整 Batch JSON), plan?(完整 ConsolidationCommit), stage: prepared\|memory_committed\|published\|completed\|failed, memory_occasion?, operations, dispositions, cleanup_pending, …}`；`MaintenanceCandidate` 按 §5.1 字段冻结（P1 填）；`GoalQuestion.candidate_id`；`StateKeep::Facts` 保留 `maintenance`。完成的 receipt 保留 32 条，一次 Activation 最多 3 批。 |
| gate 水位 | `GoalSchedule.gate: Option<MaintenanceGate>`（`status`、`last_checked_slot/last_baseline_slot/last_checked_at_ms/last_full_check_at_ms`、`last_skip_reason`、`last_inputs_digest`、`pending{new,due,waiting,retry_cleanup,next_deadline,oldest_at_ms}`、`due_reviews`、`next_review_at`、`unfinished_receipts`、`last_run_at_ms/last_run_slot`、`backoff_until_ms/backoff_level`、`last_error`）。 |
| slot 语义 | `schedule.every_secs = 1800`，基线槽 `slot % 4 == 0`；gate 对 `created/reopened/restarted` 与每个定时 fire 生效（`host/maintenance.rs::decide`，在 `GoalHost::reconcile` 内，不是 `AdmissionCheck`）；错误退避 5m → 15m → 30m。 |
| 默认预算 | activation 120000 tokens / 4 runs；period 86400 s / 1200000 tokens / 48 activations / `max_works = 0`（§2 建议的 32000 / 200000 在 DV 实测中不够：一轮重发整个上下文约 9k tokens，begin → commit → report 至少三轮，首次 DV 四轮 41k 即耗尽；见 §11.6）；`min_interval_secs = 60`；Activation 的 token 上限取 `min(activation 上限, 周期剩余)`（`runner/goal.rs::opening`）。 |
| lease | `CONSOLIDATION_LEASE = "memory_consolidation"`（`.locks/memory_consolidation.lease`），`drive_inner` 按 `GoalCaps.consolidation` 持有（`runner/consolidation.rs::load_caps`），Busy 不开 Activation。 |
| 工具 | `memory_consolidation_begin {limit?}` / `memory_consolidation_commit {operation_id, summary, operations[], dispositions[], tags?, abandon?, reason?}`：`SessionToolManager::call_tool` 按名字拦截（`tools.rs`），stub spec 只在有能力时进 `x.host_tools` 与 `tools.tools`（`live.rs`）。`report` 的 `acted/created_work` 在有未提交批次时被拒（`reports.rs` → `consolidation::check_final_report`）。 |
| 感知入口 | `ControlCommand::Perceive{kind, summary, tags, objects, subjects, cites, source, occurred_at, suggested_kind, memory_intent, mentions}`；`xagent ctl perceive <text> [--kind] [--tag]... [--object]... [--subject]... [--cite]... [--source <input key>] [--occurred-at] [--suggest] [--intent \| --explicit] [--mention]...`；驱动者经 `Memory::record_perception`（`runner/consolidation.rs::perceive`）：seq 由 `state.perception_seq` 分配，幂等键 = 控制输入 key，主体 UI=`channels.outbound.to`、Work=`session.owner`、Goal=Goal owner（都没有时 Agent DID），Workspace 绑定附 `workspace:<id>`，grants 另含 Agent；来源 = `--source` 的输入 key 或该控制输入 id（`actor_kind = agent`）；`records_perception = false` 的 Goal Session 拒绝（`input_rejected`），其 `run_digest/task_outcome` 也不写。 |
| 删除 | `SessionKind::SelfImprove`、`si-` 前缀、`api::create_self_improve_session`、`drive.rs` 的 self_improve lease 分支、`outcome.rs` 的 `perception_window*` 与游标提交、`state/cognition.rs::commit_consolidation` 与 `ConsolidationBatch`、`state/perception.rs` 的 `.cursor.json/Backlog/BacklogItem/cursor/backlog/read/commit_cursor`、kRPC `perception.cursor/backlog/read`、`PerceptionCursor` 协议类型与 schema、`[loader.self_improve]`、`tests/self_improve.rs`（场景迁入 `tests/memory_maintenance.rs`）、Jarvis `self_improve_signals.toml` / `self_improve_set_memory.toml`、assembler 的 `<perceptions>` 块。未删（按 T03）：`SelfCheck`、Notebook、attention signal。 |

### 11.2 与 §0–§7 不同或补充的决定

- `related_view`：批次里没有 `scope` 的材料（Runtime 记录）按“关于 Agent 自己”匹配已知认知（`Scope{subjects:[agent_did]}`），否则模型看不到要修订的条目，只能重复创建。
- `begin` 返回 `empty: true` 时不建 receipt；模型以 `no_action` 结束。`commit` 的计划被 Memory 拒绝（Invalid/Conflict/NotFound）时清掉 receipt 的 plan 让模型改后重交同一 `operation_id`；`abandon: true` 把 receipt 记 `completed` + `last_error = "abandoned: …"` 并计 `abandoned_batches`。
- 机械维护 `maintain()` 在每次持 lease 的 drive 开始运行：`prepared` 无 plan 的 receipt 直接丢弃（材料未处置，下次重读）；其余按阶段推进；`sweep_expired(24h)` 与 `cleanup()`。gate 的 `mechanical` 结果只 `drive` Session（不投运行机会），由它完成。
- 宿主侧 `consolidation_busy` 没有单独落到 Goal status：Busy 由 drive 返回、Supervisor 的 hosted status 记录，下一 tick 重试。
- 库默认 `maintenance.enabled = false`（测试与非 Jarvis 模板不自动多出 Goal），Jarvis 模板显式打开。`GoalDefaults.maintenance` 在 `defaults(agent_root, "goal")` 每次 tick 读取，无需重启即可关闭。
- 两个使用同一 Agent DID 的目录会得到同一个 goal id（确定性 key）；隔离靠 Agent 目录，不靠 id。
- §2.1 的 owner 操作入口：复用门面写路径（见 T04 勾选说明），`service.rs` 未加新方法。

### 11.3 验证（2026-10-23，`buckyos/src`）

| 命令 | 结果 |
|---|---|
| `cargo test -p agent_tool --lib agent_memory` | 28 passed（含 2 个新用例） |
| `cargo test -p agent_tool_cli_dev agent_memory` | 7 passed（写门：Session 内 `set` 被拒、`list` 可读） |
| `cargo test -p libopendan --test memory_maintenance -- --test-threads=1` | 10 passed（播种幂等 / 零模型 / 两 Agent；材料经宿主工具整理、正文清理、第二批修订同一条目；lease Busy；自身感知拒写；未提交批次报告被拒；pause/resume；`consolidation:after_begin / after_plan / after_memory_commit` 三个故障点子进程 abort 后恢复：一条 envelope、一条认知、无材料丢失） |
| `cargo test -p libopendan -- --test-threads=1` | 全部通过（含 goal_session 25、memory_component 25、memory_sessions 11、runner_more、xagent_cli、fixtures） |
| `cargo test -p opendan -- --test-threads=1` | 全部通过（loader 24 + 1 ignored，config 15） |
| `xagent schema doc/opendan/protocol/schema` | 重生成（新增 `goal_opendan_extension`，删除 `perception_cursor`） |

### 11.4 WebUI（§2.1）

落点 `src/frame/opendan/web/src`：`pages/Goals.tsx`（Home 的 Goals 区域：标题、lifecycle、状态、内置徽标、最近 Activation 时间与 decision、下次机会 `(last_slot+1)*every_secs` + 内置 Goal 的 gate 状态、最近 skip 原因、待确认候选数、`last_error`；active 优先，超过 8 条折叠）、`pages/GoalDetail.tsx`（`#/goal/<goal_id>`：定义与修订下拉、Goal State、内置 Goal 的 Maintenance 块（gate + 簿记 + receipts + candidates）、按 epoch 分组的 Activation 与 Session 链接、变更列表、操作 Activate / Pause / Resume / Archive / Restart / Reopen，内置 Goal 禁用 Archive 并显示 `policy_version` 与开关状态）、`goals.ts`、`lib.ts`、`App.tsx` 路由、`model/{types,datamodel,krpc,mock}.ts`（11 个 goals 方法，mock 夹具含内置 Goal / 普通 Goal / 已归档 Goal，操作改变夹具状态）、`i18n.ts` en/zh、`AgentState.tsx` 的 Perception 块改为一行说明（`perception.cursor/backlog/read` 已删）、`SessionKind` 删 `self_improve`。e2e：`tests/e2e/goals.spec.ts` 7 条。验证：`pnpm build` 通过、`pnpm lint` 0 问题、`pnpm test:e2e` 25 passed（含既有 18 条）；真实 Zone 未验。

### 11.5 交付说明与未验证项

- 真实可运行（单元 / 集成测试 + §11.6 的宿主侧 DV）：默认 Goal 的播种、gate、Activation 内的 `begin/commit`、receipt 恢复、`ctl perceive` 写入、`agent-memory` 写门、schema 4.0 字段与淡出、WebUI Goals 页。
- **未验证**：容器形态（`./build_aios --local-test` 后的 aios 镜像里的 opendan / WebUI）——本次 DV 用 `debug_jarvis.sh` 宿主进程验证，容器仍是改动前镜像；WebUI 经 zone 网关登录后的 `goals.set_status/reopen/restart` 权限；`goals.activate` 以外的真实 30 分钟定时 Activation 只观察到 gate 决策（见 §11.6 步骤 4），未等到一次由定时 fire 触发的真实整理。
- 仍 blocked / 未做：P1（候选发现、确认、`questions` 候选关联的运行逻辑；类型已冻结）、P2（workflow 幂等合同与落地、`opendan.command`、decision 预检 `auto`、`resume_gate`）；T02 第二条（recall/observe 接线）另立 TODO；TD-16 `backfilled`；Memory TODO §8.5 的 Notebook / attention signal 退役。
### 11.6 DV 记录（2026-10-23，`xiaobai.test.buckyos.io`，`./debug_jarvis.sh devtest --agent xiaobai.test.buckyos.io`，kRPC `http://127.0.0.1:10048/kapi/opendan`）

证据：`notepads/evidence/jarvis-default-goal-session/2026-10-23/`（schedule.json、goal_state.json、activations.jsonl、memory_occasions.jsonl、perception_ui_after_cleanup.jsonl、agent_memory_load.txt、goal_session_tool_calls.txt、webui_home.png、webui_goal.png）。

1. **播种与零模型调用**：全新 Zone（`start.py --all` 后用 `test/test_opendan/agent_target.ts --create xiaobai` 建 Agent），Loader 启动即 `goals: memory maintenance goal g-k3a2b568b895bac7d2b8a3911 (policy 1)`；`goals.list` 恰一个 `extensions.opendan.builtin = memory_maintenance`；`goals.schedule.gate.status = no_work`（`last_inputs_digest = g0/p0`，`last_full_check_at_ms` 已填）；Goal Session 目录无 `usage.jsonl`。Loader 重启两次后仍 1 个 Goal，`goals.changes` 只有一条 `created`（后来一条 `revised` 是我手动改预算）。`loader.status.modules` 含 `memory_maintenance: enabled running`。
2. **注入材料**：`test_agent_loader.ts` 发一句“请记住：以后给我的发布报告要尽量简短，只列要点”；Jarvis 的 UI Session 自己调用了 `ctl perceive`（观察 `ui-…:1`，scope.subjects `did:bns:devtest`，来源 `q#2`、`actor_kind = agent`）并留下 `run_digest`（`ui-…:2`）。`goals.activate` 后：
   - 第 1 次 Activation（原 32000 token 预算）：模型两次猜错字段（disposition 用 `ref/status`，observation kind 写 `observation`），每轮重发约 9k tokens 的上下文，4 轮 41k 后 `budget_exhausted / unknown`；receipt 停在 `prepared`（无 plan），下一次 drive 的 `maintain()` 丢弃并计 `abandoned_batches = 1`，材料保留。
   - 据此把工具 args_schema 写成精确的闭合枚举（`commit_args_schema()`），begin 结果的 `rules` 给出完整字段骨架，默认预算改为 120000 / 1200000（§11.1）；DV 上用 `goals.revise`（owner 侧，rev 2）改预算。
   - 第 2 次 Activation：`begin` → `commit`（一次被拒：`claim.type preference` ≠ kind `attribute`；schema 描述已补“claim.type 必须等于 kind”）→ 修正后 `committed`（`occ_0000000000000001`，2 operations、2 dispositions，cleanup cleared 2）→ 再 `begin` 得 `empty: true` → `report acted`；5 轮 61.7k tokens。`GoalState.maintenance`：`consolidations 1 / operations 2 / dispositions 2 / abandoned_batches 1`，receipt `completed`。
3. **认知可读**：`agent_tool agent-memory --root <agent_root>/memory load 发布报告` 输出 `ITEM item_pref_brief_release_reports … STABILITY stable / LAST_EVIDENCE 2026-10-24T02:03:20Z / CREATED_AT 2026-10-24T02:10:07Z / BASIS inference / SCOPE s=did:bns:devtest`，`produced_by.goal_run = goal:g-…:1:2`，`prompt_version = memory_maintenance`；`state/perception/ui-….jsonl` 两条正文均为空、`cleared.outcome = absorbed / discarded`。
   - 修掉的缺陷：整理 Goal 的 Session 仍被 `finish_run` 预留 `perception_seq`、由 `catch_up` 回填 `run_digest`（出现 `goal-….jsonl`）；现在 `records_perception = false` 时既不预留也不回填。
4. **gate 行为**：整理完成后的下一个槽（无新材料）为 `no_work`；`goals.set_status paused` 后 19:30 的槽 996005 被准入链跳过，`schedule.skipped = [{slot 996005, reason paused}]`，`gate.status = paused`，无 Activation；`set_status active` 后 20:00 的槽 996006：Memory 版本向量已变（`g0/p0 → g1/p2`，因为有过一次整理提交）所以做了完整检查，结果 `no_work`，仍无新 Activation（共 2 次，都是手动的），整理 Goal 的 Session 没有再出现感知文件。`no_change` 快速路径这次没观察到（每个槽都有版本变化或状态变化），由 `tests/memory_maintenance.rs` 的用例覆盖。证据 `schedule_after_slots.json`。
5. **WebUI**：Playwright 打开 `http://127.0.0.1:10048/`（`data: krpc`）：Home 的 Goals 区域有内置徽标、`Memory maintenance`、最近 decision `acted`；详情页 `#/goal/g-…` 按 epoch 分组、4 个 Session 链接、Maintenance 块（gate + receipts）、Archive 禁用、`Next opportunity 19:30:00 (gate no_work)`（截图在证据目录）。控制台一条 `405`：页面某个 POST 打到静态目录处理器（`dir_server.rs:693 Method not allowed: POST`），是 debug 形态无登录时 SDK 的请求，与本次改动无关。

- 已知风险：`StaleLease`（锁文件被删重建）只记 `failed` 与 `last_error`，需人工处理；`pending_work` 仍全量 replay（TD-22）；`related_view` 只按 scope 重叠选已知认知，上限 24 条；行为提示词未经真实模型调优。
