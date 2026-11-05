# Agent Memory：什么感知值得捕获、什么认知会被保留、权重怎么定 —— 场景推演 TODO

日期：2026-11-03

状态：**待 review**。§1 是源码与 demo 实况核对；§2 是三个问题的判定规则提案；§3 用 demo 真实材料推演；§4 待裁决；§5 实施任务。裁决前不改代码。

读者：用户 review；之后是 codeagent。路径相对 `buckyos/`，实施前重新核对源码。

依据：

- [Agent Memory 认知管理需求](<../doc/opendan/Agent Memory 认知管理需求.md>) §2.2–§2.3（信息去处、误判 E-01—E-17）、§5（不设统一阈值、§5.8 对照例子）、M-20（召回强度与淡出）、M-30—M-34。
- [Jarvis 默认 Goal Session TODO](jarvis-default-goal-session-todo.md) §3（认知结构、稳定度、退化）、§11（已实施落点）。
- demo.buckyos.io 实况（2026-11-03 读取；10 个 Agent，4 个启用整理 Goal）。

## 0. 为什么写这份

架子已经跑通：demo 上 4 个 Agent 的整理 Goal 在 10-27—10-30 之间做了 18 次提交，处理了 123 条感知，产出 4 条认知、11 条观察，没有乱记。但效果有三个问题：

1. **写成了流水账**。认知正文里带 UTC 时间、`work-20261029-8` 这类 session id、三段里程碑和一段免责声明（见 §3 S1）。召回时这些对模型没有意义，只占预算。
2. **该记的没记到**。yoshi 的用户澄清了自己的所在地，整理时被丢弃，理由是"no confirmed location"——因为 Runtime 自动记录的 `run_digest` 只有 Agent 的一句状态，没有用户原话；Agent 也没有 `ctl perceive`（§3 S4）。
3. **权重没有依据**。模型自由填 0.4–0.75，prompt 和 `rules` 里都没有任何指引；而 weight 在召回评分里的幅度和一个首位 tag 命中相当（§1.4），它是有实际效果的。

根因一致：prompt 只给了字段清单，没有例子，也没有"值不值得"的判定规则。本文把这三个问题各自定一套可执行的规则，再用 demo 的真实材料推演一遍，看规则能不能落地。

## 1. 源码与实况核对（2026-11-03）

### 1.1 感知从哪来

| 来源 | 代码 | 内容 | 现状 |
|---|---|---|---|
| Runtime 自动 `run_digest` | [state/perception.rs:313](../src/frame/lib_opendan/src/state/perception.rs) | 每次 run 结束写一条：`summary = one_line_status`（≤500 字）、`tags = topic.tags`（模型在 StructuredReply 的 `topic` 里给，≤8 个）、`objects = artifact`、无 subjects | demo 上 123 条材料几乎全是这一类 |
| Runtime 自动 `task_outcome` | [runner/outcome.rs:1593](../src/frame/lib_opendan/src/runner/outcome.rs) | Work 完成时附 `outcome` 与 `acceptance` | 同上 |
| Agent 主动 `ctl perceive` | [xagent_cli.rs:953](../src/frame/lib_opendan/src/xagent_cli.rs)、[runner/consolidation.rs:1115](../src/frame/lib_opendan/src/runner/consolidation.rs) | `kind=observation`，可带 `--tag/--object/--subject/--cite item@rev/--explicit/--intent/--suggest/--mention/--occurred-at/--source`；幂等键=输入 key；source 标 `actor_kind=Agent` | demo 感知文件里没有看到这一类记录 |
| 整理 Goal 自己 | `records_perception=false` | 不写（防回声） | 正常 |

Agent 什么时候会主动 perceive，只有两句提示：

- [chat_route.toml:29](../src/apps/jarvis_agent/agent/behaviors/chat_route.toml)："What you learn about your owner (facts, preferences, decisions, corrections) you record with `agent-session ctl perceive …`. Record facts, not instructions."
- [groupchat_route.toml:24](../src/apps/jarvis_agent/agent/behaviors/groupchat_route.toml)：只记 owner 本人的话，不记他人对 owner 或对自己的描述。

没有提到"关于自己的失误"和"工作区经验"，也没有说用户对交付的评价要记（E-03 需要的正是这个）。

### 1.2 整理模型看到什么、被要求什么

- 材料视图 [material_view](../src/frame/lib_opendan/src/runner/consolidation.rs#L330)：`ref/at/kind/content/attributes/tags/scope|objects/suggested_kind/memory_intent/occurred_at/source{type,event_ref,session_id,actor_kind,goal_ref}/runtime_record/cites`。模型能分辨 Runtime 记录与 Agent 主动记录。
- `rules`（[consolidation.rs:558](../src/frame/lib_opendan/src/runner/consolidation.rs)）：处置格式、三类主语、`put_item`/`add_observation` 字段清单、修订规则。**没有** weight/confidence 的取值指引，没有 statement 写法，没有例子。
- prompt [memory_maintenance.toml](../src/apps/jarvis_agent/agent/behaviors/memory_maintenance.toml)：判定分类（持续偏好/任务要求/外部事实/猜测/回声/纠正）、三类主语、纠正优先、宁缺毋滥。同样没有例子。
- 提交校验：`weight`/`confidence` 必须在 0..=1（[state.rs:118](../src/frame/agent_tool/src/agent_memory/state.rs)）；`stability` 缺省 contextual；新建条目必带 `last_evidence_at`。对 statement 没有长度或内容校验。

### 1.3 认知被怎样想起（这是写法规则的依据）

- 推理前召回仍是旧路径 [drive.rs:1712](../src/frame/lib_opendan/src/runner/drive.rs)：`recall_hints(tags = state.topic.tags, max_hints = 5)`，只按 tag；新门面（Caller grants、`Budget{8,3,3}` 分池、纠正兜底）在 `lib_opendan/src/memory/` 已实现但 drive 未接，与 jarvis TODO §1 表"实际召回"一行一致。
- 候选门槛（[agent_memory/recall.rs:505](../src/frame/agent_tool/src/agent_memory/recall.rs)）：`structural + tag + fulltext` 全为 0 的条目直接不进候选。即 **tag 不命中就永远想不起来**，weight 再高也没用。
- 评分：`structural(实体 12 / 对象对 18 / 范围 12 / 一跳 6) + tag(8/4/2/1 按 tag 位置) + fulltext(每词 2，≤10) + weight×10 + confidence×6 − fade`；`fade = 距 last_evidence_at 天数 / {stable 365, contextual 30, volatile 1}`，上限 20；`explicit` 不淡出。
- **注入模型的那一行**（[background.rs:218](../src/frame/lib_opendan/src/runner/background.rs)）：`hint: <statement> (noticed <time>; about <object>)`。`kind` 存在但不渲染；`basis/confidence/weight/stability/scope` 都不渲染。`<hints>` XML（assembler.rs:429）也只有 `id/time/sentence`。

结论：以现状而言，**statement 是 Agent 唯一能看到的内容**。它必须自足，但也不能把本该在字段里的东西全写进去——字段不渲染是 Q1 要裁决的事。

### 1.4 weight 的实际作用有多大

同一 topic 下两条都命中首位 tag 的认知，weight 0.4 与 0.8 的差距是 4 分，等于一个次位 tag 的命中，或 contextual 条目 120 天的淡出。前 5 名之争里这是决定性的。但 weight 对"能否被触发"没有作用（§1.3 门槛）。所以：

- weight 决定"同题之下谁先被想起"，不决定"会不会被想起"——后者由 tag/对象决定，整理时要给对入口。
- confidence×6 的幅度更小（0.5 与 0.95 差 2.7 分），主要是显露用，排序作用弱。

### 1.5 demo 实况摘要

| Agent | 材料 | 丢弃/吸收/判重/暂缓 | 认知 | 观察 | 提交 |
|---|---|---|---|---|---|
| libin-jarvis | 77 | 65 / 7 / 5 / 0 | 3 | 7 | 10 |
| pinkcooper | 17 | 11 / 4 / 2 / 0 | 1 | 4 | 3 |
| victor-jarvis | 28 | 28 / 0 / 0 / 0 | 0 | 0 | 4 |
| yoshi | 1 | 1 / 0 / 0 / 0 | 0 | 0 | 1 |

- 4 条认知全是 `observation_inference` + `semantic_kind=experience` + contextual + 主语=Agent 自己；`entities` 全空；没有 object、relation、attribute；没有 volatile；没有 deferred；没有 explicit。
- 另 6 个 Agent（feifei、mandyhong、oxo-jarvis、xiaobai、xiaobai2、zzy/jarvis）用旧模板创建，`agent.toml` 无 maintenance 段、无 goal session，共 92 条感知从未整理。

## 2. 三个问题的判定规则（提案）

### 2.1 什么感知值得捕获

感知是"可能形成认知的原始材料 + 来源引用"。写入便宜、允许冗余、由整理把关（M-30），所以规则的重点不是"少写"，而是**别漏掉 Runtime 记不到的东西**。Runtime 只能看到 Agent 自己的一句状态；用户说了什么、用户怎么评价、Agent 自己哪里错了，只有 Agent 在对话里知道。

**Runtime 自动记录**：保持现状（`run_digest`/`task_outcome`）。它们是"经验"类认知的主要来源，且带 worklog 引用可追溯。不建议在 Runtime 层做过滤。

**Agent 主动 `ctl perceive` 的触发清单**（替换 chat_route 那一句，写进 ui_core 手册的 Memory 节）：

| # | 触发 | 怎么记 | 为什么 Runtime 记不到 |
|---|---|---|---|
| P1 | Owner 原话里的持续偏好、要求、边界 | 一句话原意 + `--subject <owner>`；用户说"记住 / 以后都 / 别再"时加 `--explicit` | run_digest 只有 Agent 的状态句 |
| P2 | Owner 对交付的评价：认可、否定、返工原因 | 一句话 + `--object <artifact>` | E-03：工具成功不等于用户认可；这是唯一的认可来源 |
| P3 | Owner 的纠正（否定了 Agent 之前的做法或之前的认知） | `--cite item_x@rev`，让整理走纠正路径 | 纠正要在正式提交前就能显露（§4.3） |
| P4 | Agent 自己的失误与教训：用户指出的，或自己发现的 | `--subject <agent did>`，写"我在什么情况下做错了什么" | 状态句里只有"已修复" |
| P5 | 工作区经验：环境、工具、项目约定，一次可追溯观察即可 | `--subject <workspace>` `--object <tool/project>` | 隐藏在工具输出里 |
| P6 | 用户自述的、关于自己的事实（所在地、时区、称呼、角色） | `--subject <owner>`，写"用户说 X"，不写"X 为真" | §3 S4 就是这类被漏掉的 |

**不写**（不是靠整理丢弃，而是 Agent 一开始就不记）：任务进度、等待状态、预算余量；问候、能力自述；一次性的任务参数（"这次用蓝色"，E-09）；群里他人对 owner 或对自己的评价；实时世界状态（天气、价格、在线）；密钥。provider 瞬时报错也不记，除非形成了"遇到 X 先怎么做"的处置经验（那就是 P5）。

**判定口诀**（放进手册）：*这句话一周后、换一个 session，还会影响我怎么做吗？会——记；不会——不记。*

### 2.2 什么认知会被保留

整理时每条候选认知过三问，缺一不可：

1. **自足**：把 statement 单独放进一个全新 session，Agent 读完知道"什么成立"或"该怎么做"吗？只记"发生过 X"而不说明它改变了什么的，不是认知（§3 S2）。
2. **主语**：是 Agent 自己 / Owner / 某工作区之一吗？不是的只能挂为三者的属性，或作 volatile 线索带 `valid_until`。
3. **来源**：有 observation 指向原始事件、且不是回声（E-05、E-15）吗？转述、Report 摘要、Goal digest 不增加证据。

通过三问之后，**写法规则**（现状缺的就是这个）：

| 规则 | 要求 | 反例（demo 原文） |
|---|---|---|
| W1 一条一事 | 一条认知只说一个结论 | S1 的 rev2 把三段里程碑 + 一段纠正 + 一段免责写在一句里 |
| W2 结论句、现在时 | 写"X 是 / 要 Y"，不写"某日某 session 报告了 X" | "work-20261029-8 reported … at 10:52 UTC" |
| W3 时间与 id 进 observation | statement 里不出现 session id、UTC 时间戳、事件引用；它们在 `evidence → observation.source_ref` | 同上 |
| W4 不确定性进字段 | `basis/confidence/stability/review_at` 表达证据性质；statement 里最多一个限定从句（"按报告"/"未独立核实"），不整段免责 | "These are report-level historical findings, not independent inspection by memory maintenance, proof of recipient reading, or evidence of current connectivity." |
| W5 可做的写成可做的 | 经验/纠正类优先写"在什么情况下我该怎么做"；事实类用 `attribute` | 同一教训写成"曾被标成功但没交付"不如"说完成前先核对交付证据" |
| W6 对象要挂 | 涉及具体系统、工具、项目的，`upsert_object` 并放进 `scope.objects`；这是召回的结构入口（+12/+18 分） | 4 条 `entities` 全空，ClawChat 从未成为对象 |
| W7 同类要连 | 同一主题的认知之间 `upsert_relation`（如 `relates_to`/`refines`），不是只在 `write_reason` 里提一句 | S1 两条教训只在 write_reason 里说"相关" |
| W8 tags 是入口 | 每条至少 2 个 tag：一个是主题词（用户会怎么提到它），一个是情境词（什么任务里该想起它） | 两条 `tags: []`；无 tag 则只能靠全文命中 |
| W9 用 owner 的语言 | 中文 owner 写中文 | 10-30 前英文，之后中文混用 |
| W10 长度 | statement ≤ 200 汉字 / 400 字符 | rev2 约 900 字符 |

**处置决策表**（材料类型 → 默认处置；与 demo 的实际处置对照）：

| 材料 | 默认处置 | demo 实况 |
|---|---|---|
| Owner 明确的持续要求/边界（P1，explicit） | absorbed → stable，explicit | 未出现（没人 perceive） |
| Owner 一次明确表达的偏好/事实（P1/P6） | absorbed → stable 或 contextual 的 attribute/relation | yoshi 地点被 discarded（S4） |
| Owner 对交付的评价（P2） | absorbed → owner 偏好 或 Agent 经验 | 未出现 |
| Agent 失误 + 纠正（P4） | absorbed → Agent experience/correction | S1 做到了，写法差 |
| 工作区/工具一次观察（P5） | absorbed → contextual，窄范围，`review_at` | 未出现 |
| 同一事件的第二份摘要（Report/UI 转述/Goal digest） | duplicate，挂到已有 cognition_refs | S1 判了 5 条 duplicate，正确 |
| 任务进度、等待、预算、问候、能力自述 | discarded | 105 条 discarded，大多正确 |
| 外部状态（provider 错误、天气、价格） | discarded；形成处置经验的除外 → volatile 线索 | S3 |
| "做过 X"的历史事件 | discarded；确需保留时为 Agent 的 attribute，weight ≤ 0.3 | S2 写成了 0.4 的 experience |
| 结论需要还没到的证据 | deferred + deadline + reevaluate_when | 18 次提交 0 次 deferred，偏硬 |

### 2.3 权重怎么定

现状是模型凭直觉填。提案：**weight 不是主观重要性评分，是按"谁说的 × 什么类型 × 是否明确"查表得到的初值，之后只由整理按规则修订。**

初值表（提交时按 `basis × semantic_kind × explicit` 查，写进 `rules.weight_guide` 和 prompt）：

| 认知 | weight 初值 | confidence 参考 | stability |
|---|---|---|---|
| Owner 明确要求/边界（explicit） | 0.9 | 0.95 | stable |
| Owner 一次明确表达的偏好 | 0.7 | 0.9 | stable |
| Owner 自述的关于自己的事实 | 0.6 | 0.9 | stable；地点/角色类加 `review_at` 半年 |
| Owner 对交付的评价形成的偏好 | 0.7 | 0.85 | contextual（限该类交付） |
| Agent 失误教训，用户指出 | 0.7 | 0.9 | stable |
| Agent 失误教训，自己发现 | 0.6 | 0.8 | contextual |
| 工作区约定，用户说的 | 0.7 | 0.9 | stable（限该工作区） |
| 工具/环境经验，一次观察 | 0.5 | 0.7 | contextual + `review_at` |
| 同上，独立复现后（修订） | +0.1，上限 0.8 | 0.85 | contextual |
| 第三方主张（文档、其他 Agent） | 0.4 | 0.5–0.7 | contextual |
| 历史事件（"做过 X"） | 0.3 | 0.9 | contextual |
| 外部状态线索（volatile） | 0.3 | 0.8 | volatile + `valid_until` |

修订规则：

- 新的**独立**证据支持（不同来源事件，非回声）：weight +0.1，`last_evidence_at` 更新；上限 0.9（explicit 的边界才到 0.9）。
- 反例或用户纠正：不降权，走 `set_status superseded` / `replaces`（§4.3）；"这次先不用"不是纠正（E-16）。
- 召回回声（B-44）、Report 转述、同事件再摘要：不动 weight，不动 `last_evidence_at`。
- confidence 与 weight 独立填，不互相推导；confidence 表达证据支持度（需求 §A 表："只参与排序，不证明可靠"）。

理由：

- §1.4 算过，weight 的幅度等于一个 tag 命中；查表能让"owner 明确说的"稳定压过"Agent 自己推断的"，这正是 M-20 "重要但证据弱的可以靠前、同时标明证据弱"的分工：weight 管位置，basis/confidence 管显露。
- 表里 owner 相关的整体高于 Agent 自己的，高于工作区的；这与三类主语的价值顺序一致——Agent 服务的是人。
- 不做"被实际用到"的派生强度（M-20 里的召回强度统计）：现在没有"用到"信号，B-44 又规定回声不计，先不引入。

落点：指引而非硬校验（Q3）。DV 看分布：一轮整理后 weight 的取值应集中在表内档位，不应出现 0.55 这类"折中值"。

## 3. 场景推演

每个场景按：材料 → 捕获判断 → 整理判断 → 认知写法 → 权重 → 召回入口 → Agent 实际看到的那一行。S1–S4 是 demo 真实材料，S5–S6 是补的典型场景。

### S1 libin-jarvis：ClawChat 接入（真实，77 条材料）

**材料**：10-29—10-30，UI session `ui-20261029-2` 里用户反复催问进展；若干 work session 的 `run_digest`/`task_outcome`（3 个标 succeeded 但没交付，第 8/9 个报"最小适配器完成""发出一条消息收到 ACK"）；10-30 Agent 在 UI 里自我纠正"不该让用户催"、并核对发现只做到单次发送。

**捕获判断**：Runtime 记录齐全。缺的是：用户催问时说的话（P1——"我要看到的是有证据的进展"）没有被 perceive；Agent 自己承认失误（P4）也只是落在 run_digest 的状态句里。如果 Agent 在当时 `ctl perceive "owner 要的进展汇报是可验证的变化或明确阻塞，不要等他催" --subject did:bns:libin`，basis 就是 `user_statement` 而不是从状态句推断。

**整理判断（按 §2.2）**：

| 原认知 | 三问 | 处置 |
|---|---|---|
| `item_agent_clawchat_completion_label_caution`（succeeded ≠ 验收） | 通过 | 保留，按 W2/W3 重写 |
| `item_agent_clawchat_reported_delivery_milestone` rev2（三段里程碑 + 纠正 + 免责） | 问 1 不过：读完不知道该怎么做 | 拆成一条工作区属性 + 一条 Agent 经验；里程碑进 observation |
| `item_agent_evidence_based_proactive_progress`（自我纠正） | 通过 | 保留，拆出 owner 偏好一条 |

**认知写法（应得）**：

```text
obj_clawchat                      upsert_object kind=system canonical_name=ClawChat

item_self_succeeded_is_not_accepted      experience · agent · stable · w0.7 c0.95 · basis tool_observation
  "Work session 标成 succeeded 只表示会话结束，不表示任务被验收。我在说"完成"之前，
   要读它的最终报告，核对任务要求的交付证据。"
  tags: [验收, work-session]

item_self_stall_detection                correction · agent · contextual · w0.7 c0.9 · basis user_statement（用户指出）
  "监督长任务时，我容易把"还在运行"当成进展。每次检查先问：有没有新的可验证产出？
   没有就按卡住处理，给用户结论。"
  tags: [监督, 进展汇报]        relation: refines → item_self_succeeded_is_not_accepted

item_self_integration_acceptance_scope   experience · agent · contextual · w0.6 c0.85
  "接入类任务的验收标准要在开工前写清：持续连接、收发、自动回复；不能用"发出去一条"代替。"
  tags: [验收, 接入]            scope.objects: [obj_clawchat]

item_ws_clawchat_stage                   attribute · workspace · contextual · w0.5 c0.85 · review_at +14d
  subject=<workspace> attribute=clawchat_integration_stage
  value="只做到单次发送并收到 ACK；没有常驻客户端、心跳、重连、持续收件和自动回复"
  scope.objects: [obj_clawchat]  tags: [clawchat, 接入]

item_owner_progress_report_style         preference · owner · stable · w0.7 c0.9 · basis user_statement
  "libin 要的进展汇报是"有可验证的变化"或"明确的阻塞"；"任务还在跑""又安排了恢复"不算进展，不要等他催。"
  tags: [汇报, 监督]
```

4 个里程碑、UTC 时间、session id 全进 `add_observation`（现有 7 条观察已经是这么写的，不用动）。

**权重**：按表——owner 偏好 0.7 / Agent 教训（用户指出）0.7 / 自己发现 0.6 / 工作区属性 0.5。原来的 0.55 和 0.75 没有依据。

**召回入口与效果**：下次任何带 `接入` 或 `验收` tag 的 work session，推理前会看到：

```
hint: Work session 标成 succeeded 只表示会话结束，不表示任务被验收。我在说"完成"之前，要读它的最终报告，核对任务要求的交付证据。 (noticed 2026-10-29)
hint: 接入类任务的验收标准要在开工前写清：持续连接、收发、自动回复；不能用"发出去一条"代替。 (noticed 2026-10-30)
```

对比现状 rev2 那一行是 900 字符的历史叙述。两条都在 5 条预算之内；`item_ws_clawchat_stage` 只有在 topic 带 `clawchat` 时才会出现，这是对的——它只对该工作区有用。

### S2 pinkcooper：Home Station 发帖（真实，17 条材料）

**材料**：10-27，用户让 Cooper 在 Home Station 发问候，再发一条中文自我介绍；两个 work session 的 `task_outcome` 报告发布成功；UI 里 Agent 说"我会发一条介绍"。

**捕获判断**：缺用户原话。用户要求"用中文发自我介绍、说明你是我的 Agent 和能做什么"——这是 P1（owner 对公开形象的要求），应 perceive。

**整理判断**：现有 `item_cooper_home_station_posts_20261027`（"10-27 发过两条帖，p-4634… 未改，不是定期发帖"）问 1 不过——下次任务读到它，不知道该做什么不同。整理时写它的 `write_reason` 是"区分已发布与将来的发布请求"，但这件事去查 Home Station 就知道（§2.2 "当前有效版本"由世界对象承载），不需要记忆。

**应得**：

- 若有用户原话：`item_owner_public_intro_style` preference · owner · stable · w0.7："Cooper 的公开自我介绍用中文，说明自己是 felica 的 Agent 和能帮什么。" tags [home-station, 自我介绍]。
- 没有原话时：discarded；或最多一条 Agent attribute `has_published_home_station_intro=true` w0.3——这种"做过 X"按 §2.2 表默认不写。

**结论**：这条应降为 0.3 或不写。demo 的 0.4 显示模型自己也觉得它不太重要，但没有规则告诉它"不重要就别写"。

### S3 xiaobai2：provider `auth_unavailable` 连串报错（真实，积压未整理）

**材料**：10-19，work session 连续几轮 `process_done: llm provider failed … auth_unavailable (providers=codex …)`，之后 UI 里是 "Hi! I'm 小白" 和能力介绍。

**捕获判断**：Runtime 自动记了。Agent 不应额外 perceive（外部状态）。

**整理判断**：主语不是三类之一；provider 当时坏了，现在早好了。两种结果都合规：

- discarded（默认）；
- 如果同一批里 Agent **做了处置**（换模型后成功），则 P5 工作区经验：`"codex-proxy 报 auth_unavailable 时先换 llm.chat 的备用模型重试，不要当成自己代码的问题去改。"` contextual · w0.5 · `review_at` +30d。

**结论**：这是"外部状态不记成事实，但处置经验可记"的标准例。问候和能力自述全部 discarded。demo 之所以积压，是旧模板没有整理 Goal（Q7），与规则无关。

### S4 yoshi：天气地点澄清（真实，1 条材料，最能说明问题）

**材料**：用户问天气，Agent 问在哪，用户答了城市；run_digest 的状态句是 Agent 的"已回答天气"。整理判 discarded："one-off weather-location clarification; no confirmed location"。

**捕获判断**：用户自述所在地是 P6（owner 事实），应当 `ctl perceive "用户说自己在 <城市>" --subject <owner>`。没有 perceive，整理只看到 Agent 的状态句，确实没有"confirmed location"可用——**整理的判断是对的，是捕获漏了**。

**应得**：`item_owner_location` attribute · owner · stable · w0.6 c0.9 · basis user_statement · `review_at` +180d：subject=<owner> attribute=location value=<城市>。tags [所在地, 天气, 时区]。下次问天气、安排时间，Agent 直接看到 `hint: yuko 说自己在 <城市>`。

**结论**：这是 §2.1 清单 P6 的来源，也是 DV 的第一个用例：同样的对话，加了触发清单后应产生这条认知。

### S5 用户纠正交付（典型，补）

**材料**：Work 交付一份 5 页报告；UI 里用户说"太长了，以后给我一页以内的"。两周后同一项目用户说"这次详细点"。

**捕获**：第一句 P1 + P2，`--explicit`（"以后"）；第二句是一次性调整（E-09），不 perceive，或 perceive 时 `--cite item_owner_report_length@1` 让整理判断。

**整理**：第一条 → `item_owner_report_length` preference · owner · stable · explicit · w0.9："<owner> 要的报告一页以内。" tags [报告, 长度]。第二条 → 若 perceive 了：discarded（一次性例外，E-16 不是纠正），`cognition_refs` 挂上原条目；不修订、不降权。

**召回**：explicit 不淡出；一年后仍以 0.9 排在"报告"主题前列（B-48）。

### S6 工具经验（已在需求 §5.8）

tool-x 输出目录的例子已经把"一次观察 → 窄范围检查线索""独立复现加强""反例分版本""用户约定与工具限制分两条"推演完整，本文不重复；按 §2.3 表它对应：一次观察 w0.5 → 复现后 w0.6；用户说"本项目产物统一放 workspace/build" w0.7 stable。

### 推演小结

| 场景 | 现状 | 按本文规则 | 差异来自 |
|---|---|---|---|
| S1 | 3 条流水账，entities 空 | 5 条 + 1 对象 + 1 关系 | 写法规则 W1–W8、权重表 |
| S2 | 1 条 0.4 的"做过 X" | 0 或 1 条 owner 偏好 | 三问之问 1、P1 捕获 |
| S3 | 积压 | 全丢或 1 条 volatile 经验 | Q7 + 处置表 |
| S4 | discarded | 1 条 owner 属性 | P6 捕获 |
| S5 | — | 1 条 explicit，不被一次性调整动摇 | P1 + E-09/E-16 |

规则覆盖了 demo 的全部实际情况；没有需要新增字段的场景，`attribute`/`relation`/`upsert_object`/`volatile`/`review_at` 都是现成的，只是没被用。

## 4. 待裁决

| # | 问题 | 建议 |
|---|---|---|
| Q1 | 注入行是否显露 `basis/confidence/stability`？现状只有 statement + 时间 | 加，短格式：`hint: <statement> (owner 说的 · 可信 0.9 · 2026-10-30)`；basis 用自然语言而不是枚举名。这样 W4 才能成立（免责从正文移到字段） |
| Q2 | drive 切到新召回门面（Caller/Budget/分池、纠正兜底）是独立大项，本文只记依赖 | 本单不做；本文写法规则在旧路径下已成立（§1.3） |
| Q3 | 权重初值表是硬校验还是指引 | 指引：写进 `rules.weight_guide` + prompt；commit 不拒绝；DV 统计分布 |
| Q4 | 捕获清单 P1–P6 放哪 | ui_core 手册 Memory 节写全表；chat_route/groupchat_route 各留一句引用 |
| Q5 | "做过 X"的历史事件认知是否允许 | 默认不写；确需时为 Agent attribute、w ≤ 0.3。"去查哪个对象"的线索用 `scope.objects`，不用正文 |
| Q6 | statement 语言 | 用 owner 的语言；prompt 一句话 |
| Q7 | 6 个旧模板 Agent 没有整理 Goal | 不在本单：按 [DV 可全新安装原则](../AGENTS.md) 由 demo 重建，或另起播种单 |
| Q8 | statement 长度与"含 session id/时间戳"的提交校验 | commit 做软校验：>400 字符、或匹配 `(ui|work|goal)-\d{8}-\d+` / ISO 时间戳 → 拒绝并提示"进 observation"；`item_id` 不受此限 |
| Q9 | S1 中 `item_ws_clawchat_stage` 这种"工作区当前阶段"要不要记——它也是"去查就知道" | 记，但 contextual + `review_at` 14 天；理由是它给了"下次从哪继续"的入口，而查 Workspace 并不能直接得到"阶段"这个判断 |

## 5. 实施任务（裁决后）

### T0 — prompt 与 rules 加例子和指引

- [memory_maintenance.toml](../src/apps/jarvis_agent/agent/behaviors/memory_maintenance.toml) 增加 `## Shape of a cognition`：三问；W1–W10；§3 S1 的两条正例（owner 偏好、Agent 经验）+ S4 的 attribute 例 + 一条反例（demo rev2 原文，标"不要这样写"）；权重表。
- [consolidation.rs](../src/frame/lib_opendan/src/runner/consolidation.rs) `rules` 增加 `shape`（三问 + W2/W3/W10 一句话）与 `weight_guide`（表的紧凑形式）。
- 验收：`cargo test -p libopendan --test memory_maintenance`；手动读一次 begin 返回。

### T1 — 捕获清单进手册与行为

- [ui_core.md](../src/apps/jarvis_agent/agent/cmd_skills/ui_core.md) Memory 节：P1–P6 表 + "不写"清单 + 口诀；`ctl perceive` 各参数的用法（`--explicit/--cite/--subject/--object`）。
- chat_route/groupchat_route：把那一句改为引用手册，并加"用户对交付的评价、你自己的失误也要记"。
- 验收：Q4 裁定的落点；读一次 Jarvis 的 UI session system prompt 确认出现。

### T2 — 注入行显露证据性质（Q1）

- [background.rs:218](../src/frame/lib_opendan/src/runner/background.rs)、[state/cognition.rs:86](../src/frame/lib_opendan/src/state/cognition.rs) 的 `Hint` 加 `basis/confidence/stability`（旧路径从 `RecallItem` 取），渲染短格式。
- 验收：background 单测；`cargo test -p libopendan -- --test-threads=1`。

### T3 — 提交软校验（Q8）

- `agent_memory` commit：statement 长度与 session-id/时间戳模式 → `invalid`，错误文案指引"进 add_observation"。
- 验收：`cargo test -p agent_tool --lib agent_memory` 加两条用例。

### T4 — DV（真实材料回放）

- 本机 `start.py --all` 后建 Agent，`debug_jarvis.sh devtest --agent <did> --no-build`（跑法见 jarvis TODO §11.6）。
- 用例：S4（问天气 → 答城市 → `goals.activate` → 应出 owner location attribute）；S5（两句话 → explicit 偏好 + 第二句 discarded 且 cognition_refs 挂上）；S1 的材料可从 demo 复制 `state/perception/*.jsonl` 到本机 Agent 目录回放（注意 seq 连续、lease 由本机 session 持有；需要先核对 `record_perception` 对外来文件的接受条件）。
- 检查：`items.jsonl` 里 weight 取值全部落在 §2.3 档位；statement 无 session id/时间戳；每条 ≥2 tags；涉及对象的有 `scope.objects`；注入行按 Q1 格式。

### T5 — 文档回写

- 需求文档附录 B 加 TD（写法规则、权重初值表、捕获清单）；jarvis TODO §11 加引用。
- 记忆：按 [agent-memory-component-impl] 的方式记落点。

## 6. 验证命令（`buckyos/src`）

```
cargo test -p agent_tool --lib agent_memory
cargo test -p libopendan --test memory_maintenance --test memory_sessions --test memory_component
cargo test -p libopendan -- --test-threads=1
cargo check --workspace --tests
```
