# OpenDAN Agent 如何观察世界

> 本文整理自已有草稿与补充口述信息，目标是说明 OpenDAN 如何把基于“浮现”的 Hints 体系贯穿起来，作为 Agent 跨 Session 获得全局信息、观察现实世界、并把相关信息编织进当前推理上下文的核心机制。

---

## 1. 问题背景：Agent 首先要能观察世界

如果把 Agent 的能力抽象成三件事：

1. **如何观察并了解现实世界**；
2. **如何基于观察进行推理**；
3. **如何通过工具或行动影响现实世界**；

那么“观察世界”是整个 Agent 架构中最基础的一环。

一个 Session 在启动时，本质上是一个线性的上下文窗口。除了系统提示词、工具定义和少量初始环境信息之外，Agent 并不能天然看到现实世界中已经发生的事情，也不能天然知道用户在其它 Session、Notebook、Memory、文件系统、日程、位置、项目目录、外部对象状态里留下了什么信息。

所以核心问题是：

> 在有限的线性上下文中，Agent 如何持续获得和当前任务有关的现实世界信息？

传统上，这个问题常常被归类到 RAG。但从 OpenDAN 的角度看，它更接近一个“观察系统”问题，而不仅仅是“检索增强生成”问题。

---

## 2. 两类传统方案及其问题

目前主流思路大致可以归纳成两类。

### 2.1 自动插入模式：机械式 RAG

第一类是自动插入模式。

每当用户输入一条消息，系统就在旁路中执行一次机械检索，例如：

- 全文搜索；
- 向量相似度搜索；
- 图数据库邻近节点搜索；
- 结构化索引匹配。

然后系统把召回出来的前若干个结果直接插入到用户输入前面，作为上下文的一部分交给 LLM。

这种方式的优点是“不容易漏查”。但问题也很明显：

1. **插入时机固定**：通常每次输入都查，每次输入都插。
2. **插入内容机械**：检索结果基于文本相似度或索引相关性，并不一定和当前推理目标真正相关。
3. **上下文污染严重**：用户可能只说了一句话，但系统在前面塞入大量文本块。
4. **逻辑关联弱**：LLM 在阅读上下文时，很难知道这些 block 为什么出现、和推理目标有什么关系。
5. **成本和密度高**：为了避免漏掉信息，只能高频触发和高密度插入。

这是一种“怕漏查”的工程路线。它能提高覆盖率，但代价是上下文窗口被大量背景文本占用，且这些背景文本并不天然符合 Agent 的推理流。

### 2.2 Agent Tool 模式：主动查询

第二类是 Agent Tool 模式。

系统不自动插入大量内容，而是把现实世界中的资源暴露成工具，例如：

- 文件系统查询工具；
- grep / glob；
- SQL 查询；
- RPC 接口；
- MCP 工具；
- 向量库或全文索引查询工具；
- DID-Object 读取接口。

这样 Agent 可以带着意图去查。如果不查询，工具只占用少量工具定义上下文。

这种方式比机械式 RAG 更符合 Agent 的推理逻辑，因为 Agent 大致知道自己为什么要查，以及查到的信息如何服务当前目标。

但它有一个认知层面的根本问题：

> Agent 不知道自己不知道什么。

如果 Agent 没有任何线索，它就不会主动查询某件事是否存在。

例如用户说：

> 我明天中午要和某某吃饭。

如果外部日程里有冲突，Agent 必须意识到“这可能和日程冲突有关”，然后主动查询日程。但如果系统没有给出任何线索，Agent 很可能只根据当前对话继续推理，而不会想到上下文之外还有一个相关事件。

所以 Agent Tool 模式的问题不是“查不到”，而是“想不到要查”。

---

## 3. OpenDAN 的核心思路：让相关信息先“浮现”为 Hint

OpenDAN 的设计不是简单选择自动插入或主动查询其中之一，而是在两者之间加入一层“浮现机制”。

所谓“浮现”，可以理解为一种模拟人类工作记忆的机制：

- 人在对话时，并不是把所有长期记忆全部读入脑中；
- 也不是完全等到自己明确知道要查什么才查；
- 很多时候，人脑中会先浮现出一些线索；
- 如果线索足够重要，人会顺着线索去打开笔记、文件、聊天记录或外部系统，确认完整事实。

OpenDAN 中，这个浮现机制的核心是：

> **Session Topic Tag + Hints + 主动查询 / 半订阅。**

基本流程是：

1. 当前 Session 中浮现出一组 Topic Tags。
2. 系统基于这些 Topic Tags 召回一组 Hints。
3. Hints 不要求包含完整事实，只要告诉 Agent“某件相关事情存在”。
4. Agent 后续可以基于这些 Hints 决定是否主动查询详细信息，或者订阅相关对象状态变化。

这解决了主动查询模式中的关键问题：

> Agent 不需要一开始就知道所有事实，但它需要知道“可能有这么一件相关的事”。

### 3.1 Notebook 与 Memory 的本质区分

主流 memory 方案常把两种不同的东西混成一个系统，导致两边都不到位。OpenDAN 把它们分开看待：

| 维度 | Notebook | Memory |
|---|---|---|
| 写入语义 | “请记下来”（显式） | “刚才发生了”（沉淀） |
| 内容形态 | 事实 / 规则 / 提醒 | 经历 / 感觉 / 线索 |
| 召回需求 | 精确匹配 / 必然召回 | 联想浮现 / 可能相关 |
| 写入者 | 用户主导（含 Agent 显式调用） | 系统主导 |
| Prompt 位置 | system prompt | 紧贴 user msg |
| 失败模式 | 漏掉 = 灾难 | 没浮现 = 可接受 |

`update_session_topic` **属于 Notebook 子系统的工具** —— 由 Agent 显式调用、写入精确的事实条目（本 Session 当前在谈什么）。但它写入的内容会被 **Memory 浮现层** 消费：未来 Session 在合适时机浮现“昨天讨论过 X (session: …)”时，topic 行就是浮现产物的来源。

简言之：

> **写入是 Notebook 语义，消费是 Memory 语义。**

这条工具是把“事实级 topic”和“启发式浮现”对接的桥。

### 3.2 浮现 = pointer + hint + 延迟具象化

整个浮现机制可以归纳为三个层次，复用同一个文件系统抽象（`session_dir` 是天然的锚）：

1. **“这里有东西”**：浮现产物只是 *awareness* —— 一个 Session 存在、它大致谈了什么；
2. **“它在哪”**：Session ID 作为锚点，文件系统路径作为统一 schema（不引入专门的 memory API）；
3. **“用的时候再展开”**：浮现阶段每条线索只占 20–50 token；模型判断要不要深入时再触发对文件级资产的“深度调用”。

`update_session_topic` 写入的 **topic 行**就是这一层的 “hint”，`session_id` 就是 pointer，`session_dir` 下的 history / artifacts 就是延迟具象化的目标。

---

## 4. 核心概念

### 4.1 Session

Session 是 Agent 当前正在进行的一段线性上下文。它可能是一次聊天、一次任务执行、一次代码修改、一次旅行计划制定，或一次跨工具的工作流。

Session 的本质特征是：

- 它是线性的；
- 它有上下文窗口限制；
- 它承载当前正在进行的推理；
- 它不能直接容纳所有长期记忆和全局状态。

### 4.2 Session Topic Tag

Session Topic Tag 是当前 Session 的“工作焦点标签”。

它不是长期记忆，也不是完整任务描述，而是类似人脑工作记忆中的当前关注点列表。

例如：

```text
OpenDAN
Agent Memory
Session Topic
RAG
旅行计划
Booking
NFL
49ers
```

这些 Tags 的作用是帮助系统判断：

- 当前 Session 正在围绕什么话题展开；
- 哪些 Memory / Notebook / DID-Object 可能相关；
- 是否需要触发 Hints 召回；
- 是否需要对某些环境状态进行半订阅。

### 4.3 Last Topic Title / Last Session Title

系统除了维护一组 Tags，还应维护一个更自然语言化的当前主题标题，例如：

```text
讨论 OpenDAN 中基于 Session Topic 的浮现式 Hints 召回机制
```

这个标题可以理解为当前 Session 的一句话摘要。它的意义是：

- 为 Tags 提供语义解释；
- 帮助后续 Session 级别召回；
- 作为历史 Session 的可索引标题；
- 给 Agent 一个更接近任务意图的当前工作描述。

### 4.4 Hint

Hint 是“线索”，不是完整上下文。

Hint 的目标不是把所有事实都塞进上下文，而是告诉 Agent：

> 有一个可能相关的对象、事实、记忆、笔记或 Memory 中沉淀出的历史 Session 线索存在。

一个好的 Hint 应该包含：

- 它指向什么对象；
- 为什么和当前 Topic 有关；
- 它大概是什么类型的信息；
- Agent 是否需要进一步查询；
- 如有必要，是否可以订阅其状态变化。

Hint 不应该默认携带大段原始文本。

#### 4.4.1 Agent Memory Hint Type

Agent Memory Hint Type 描述 Agent Memory 内部的 Hint 语义类型和召回层级，和全局 `Source` 不完全等价。

`Source` 表示 Hint 来自哪个系统，当前大的来源只有 Agent Memory、Notebook、DID-Object；`Agent Memory Hint Type` 表示 Agent Memory Recall 在当前 Topic 下应该用哪一类观察方式生成和淘汰 Hint。

Agent Memory 当前至少应区分五类 Hint：

1. **Session 原文类 Hint**

   Agent Memory 通过 Topic Tag 发现另一个相关 Session 时产生。它指向的是历史 Session 中的原文、topic、history 或 artifact，而不是对事实的再加工总结。

   这类 Hint 的价值是告诉 Agent：“之前有一段原始工作上下文可能相关，可以按需打开。”

2. **Event Hint**

   表示现在正在发生、最近发生、或 Agent 曾经关注过的事件。它属于 Agent Memory 的事件类信息。

   如果一个事件和当前 task 相关，Recall 应该把它作为 Hint 浮现出来，让 Agent 知道当前任务可能受某个进行中事件影响。

3. **Entity Observation Hint**

   当 Topic Tags 命中实体名称、别名、人名、对象名或其它可识别实体时产生。

   这类 Hint 表示系统应该对该实体做“对象观察”：它是谁、当前状态是什么、最近有什么变化、是否存在可读对象或可订阅状态。

4. **Entity Relation Hint**

   只有在当前 Topic 中识别出多个实体时才产生。

   如果系统知道这些实体之间存在关系，就应召回关系类线索，尤其是关系状态、态度、协作、冲突、依赖、所属、引用等信息。通常至少有一个实体是 Agent 自己或 Owner。

5. **Free Hint**

   不属于以上四类、但仍可能帮助 Agent 发现相关信息的自由线索。

   它可能来自弱关联记忆、模糊语义命中或其它 Agent Memory 内部启发式来源。

Agent Memory Hint Type 的作用是让 Memory Recall 能按类别分层查询、类内排序和类内淘汰，而不是让所有 Memory Hint 直接进入一个全局排序池。这样可以避免某一类高产候选挤掉其它类别中更关键但数量较少的线索。

### 4.5 DID-Object

DID-Object 是 OpenDAN 中对现实世界数字化状态的一种抽象。

从心智模型上看，可以认为：

> 数字世界中的所有状态，都可以通过一个巨型文件系统访问。

也就是说，某个对象可以用类似下面的结构定位：

```text
hostname / object_id / path
```

其中：

- `hostname` 表示对象所在的系统或命名空间；
- `object_id` 表示具体对象；
- `path` 表示对象中的某个状态、属性或字段。

Memory、Notebook、文件目录、项目状态、用户位置、日程、设备状态、外部服务状态，都可以被抽象为 DID-Object。历史 Session 经过 Self Improve 后进入 Agent Memory，不作为独立的大来源参与 Recall。

---

## 5. `update_session_topic`：浮现机制的标准工具入口

`update_session_topic` 是 Agent 观察世界机制中的一个关键工具。

它的职责不是直接回答用户问题，而是维护当前 Session 的 Topic 状态，并在必要时触发 Hints 召回或环境状态订阅。

从 Agent 角度看，它是一个标准工具：

```text
Agent 在发现当前任务主题发生变化、收敛或深化时，调用 update_session_topic。
```

典型输入可以包括：

- 当前 Topic Title；
- 一组 Topic Tags；
- 每个 Tag 的简短理由；
- 当前变化是否显著；
- 可选的召回策略参数。

示意：

```json
{
  "title": "讨论 OpenDAN 的 Session Topic 与 Hints 召回机制",
  "tags": [
    { "name": "OpenDAN", "reason": "当前讨论的系统架构" },
    { "name": "Session Topic", "reason": "当前机制核心" },
    { "name": "Hints", "reason": "用于浮现相关信息" },
    { "name": "Agent Memory", "reason": "召回来源之一" }
  ]
}
```

工程接口上，`update_session_topic` 至少应表达以下语义：

```rust
update_session_topic(
    title: String,
    tags: Vec<TopicTagInput>,
    recall_policy_override: Option<RecallPolicyOverride>,
) -> UpdateSessionTopicResult
```

其中：

- `title` 是当前 Session Topic Title，一行自然语言，单行、≤ 120 字符、人类可读、对未来的“我”友好。旧草案里的 `topic` 参数在本文统一命名为 `title`，语义不变；
- `tags` 是当前 Topic Tags。每个 Tag 必须包含 `name` 和 `reason`；`reason` 用于调试、后续解释和统一 Hint 呈现；
- `recall_policy_override` 是可选策略覆盖项，只能调整本次召回策略，不应改变 Topic / Tag 的存储语义。

典型输出可以包括：

- 更新后的 Topic Title；
- 当前仍然新鲜的 Tags；
- 被增强的 Tags；
- 被淘汰的 Tags；
- 立即召回的 Hints；
- LLM 旁路是否触发；
- 新创建的半订阅；
- 下一次上下文编织时需要关注的 Background Information。

工具返回值应保留一个最小可验证结构，方便 Agent、测试和日志统一理解：

```rust
struct UpdateSessionTopicResult {
    tag_set_diff: TagSetDiff,
    recall: Option<RecallPayload>,
    recall_status: RecallStatus,
}

struct TagSetDiff {
    added: Vec<String>,
    removed: Vec<String>,
    reinforced: Vec<String>,
    current: Vec<TagEntry>,
}

enum RecallStatus {
    NotTriggered,
    Mechanical { ms: u32 },
    LLM { ms: u32 },
    Failed { reason: String },
}
```

`tag_set_diff` 是工具的保底结果，机械更新成功后必然存在；`recall` 可能为空；`recall_status` 即使在召回失败时也必须明确返回，避免 Agent 把“未触发”和“触发失败”混为一谈。

但需要强调：

> `update_session_topic` 不一定每次都执行完整召回。它可以只是快速更新 Topic，然后返回。

### 5.1 调用时机：反例

期望模型在以下时机调用：用户首条消息已让主题清晰、话题发生显著漂移、Session 即将进入长尾。下列情况**不应触发**：

- 用户随手聊一句、和当前任务无关的边角问答；
- 模型自己内部的中间步骤；
- 当前 topic 的细微展开（不是漂移）。

主题更新本身是 Agent 的元行为，调用频率应远低于普通工具调用。工具描述（给 LLM 看的 system prompt 片段）必须明确：只在主题首次明确或显著漂移时调用、写给“未来的自己”而不是给用户看、不是会话总结、不要堆细节。

### 5.2 同步契约：永不 pending

`update_session_topic` 的工具结果遵守 `success | error` 协议，**永不返回 pending**。具体约束见 §10.3，这里先给出对 Agent 状态机的含义：

- Agent 拿到 `success` 时可以确信：要么没召回（`recall` 字段为 None），要么召回已完成（`recall` 字段含完整条目）；
- 即便召回失败（超时 / 旁路 LLM 报错），工具仍 `success`，因为 Tag 更新这一“保底语义”已经完成；
- **不存在“工具已返回但召回还在后台跑”的中间状态** —— Agent 侧不需要处理异步召回的事件回调。

### 5.3 调用语义补充

- **当前态覆盖写，调用历史 append**。一个 Session 同一时刻只有一个当前 topic；每次成功调用都会进入 session history，形成按时间线排列的 topic/tags 更新记录；
- **幂等**：相同 topic 内容重复调用对 `topic.md` 是 no-op；但 tags 的变化仍会触发 Tag 集合更新与可能的召回；
- **历史可追溯但不必暴露给 LLM**：底层把每次更新落到 `round_history` 的 `session_topic_updated` 事件，并同步保留 `topic_log.jsonl` 作为运维 / 审计用时间线；工具对外仍承诺“当前 topic = 最后一次写入”；
- **隔离边界**：不允许跨 Session 写他人的 topic（边界 = session_id）；
- **单行 ≤ 120 字符校验**：topic title 单行、人类可读、对未来的“我”友好。
- **缺失态可表达**：从未调用过该工具的 Session 不应生成空 `topic.md`；浮现层必须能容忍没有 topic 的 Session。

### 5.4 文件布局

复用 `session_dir` 作为锚，避免引入第二套 memory 数据库：

```
{session_dir}/
  .meta/
    topic.md           # 当前 topic（覆盖写）
    topic_log.jsonl    # 历次更新审计（append-only）
    tag_set.json       # 当前 Tag 集合状态（含权重、时间戳、tier）
    subscriptions.json # 当前活跃的状态订阅（由 LLM 召回路径产生）
  round_history/       # 见 session-history 设计；含 session_topic_updated 事件
  ...
```

`topic.md` 内容形态：

```markdown
---
session_id: 2026-05-12-xxx
updated_at: 2026-05-12T15:30:00Z
tags: [llm-context, design]
---

讨论 LLM Context 设计与“浮现”式 Memory 的工程实现
```

`tag_set.json` 内容形态：

```json
{
  "capacity": 8,
  "tags": [
    {
      "name": "llm-context",
      "weight": 3.2,
      "last_touched": "2026-05-12T15:30:00Z",
      "tier": "active"
    }
  ],
  "last_recall_turn": 12,
  "last_recall_at": "2026-05-12T15:25:00Z"
}
```

全局索引由浮现层另行维护，本工具不负责索引，只承诺：写完 `topic.md` 后浮现层最终能看到。

### 5.5 工具边界

`update_session_topic` 是 Notebook 语义的写入工具，不是一个新的 Memory 查询 API。

实现时应遵守以下边界：

- `session_dir` 路径由 AgentSession 决定，工具从当前 Session 上下文读取，不自行发明路径；
- 工具只允许写当前 Session 的 `.meta/topic.md`、`.meta/tag_set.json`、`.meta/topic_log.jsonl`、必要时写 `.meta/subscriptions.json`；
- 不允许跨 Session 写入其它 Session 的 Topic 或 Tag；
- 不引入新的 RPC、数据库或专用 Memory API；当前状态只通过文件系统投影表达；
- 不额外暴露 `read_topic` / `list_topics` 之类专用读接口。读取 Topic、枚举 Session、深度展开 history / artifacts 应走已有文件系统或 Session 资产读取能力；
- 工具本身始终可见，因为它负责维护当前 Session 题眼；Memory / Hint 相关工具是否可见由浮现层策略决定。

---

## 6. Session Topic Tag 的数量限制与机械淘汰

Session Topic Tag 是当前工作记忆，不是长期记忆。

因此它必须有数量上限。

例如可以设定：

```text
max_session_topic_tags = 8
```

这个值可以配置，但原则上不应无限增长。

如果 Tags 无限增长，Session Topic 会重新退化成长期上下文污染，导致：

- 当前工作焦点模糊；
- 主题漂移；
- 召回范围越来越大；
- 上下文编织越来越重；
- 当前 Session 失去“正在做什么”的表达能力。

### 6.1 Tag 权重

每个 Tag 都有一个动态权重。

权重主要由两个因素决定：

1. **反复被提到会增强权重**；
2. **随着时间流逝，权重自然衰减**。

可以抽象成：

```text
tag_score = mention_weight × time_decay
```

如果某个 Tag 在最近多轮对话中持续出现，它的权重会升高；如果一个 Tag 曾经重要但长时间没有再出现，它会自然降权。

参考实现（v0.2 默认）：

```text
score = weight * exp(-(now - last_touched) / TAU)
TAU = 30 分钟（可配置）
```

新 Tag 进入流程：

1. 若已存在同名 Tag → 权重 `+= 1.0`，`last_touched` 更新为当前时间；
2. 若不存在 → 以 `weight=1.0, tier=Transient` 加入；
3. 容量超限时：计算每个 transient Tag 的 score，淘汰得分最低者。

`tier` 字段（Pinned / Active / Transient）保留接口但 v0.2 全部置为 Transient；tier 升降级未来由 LLM 召回路径调用。

### 6.2 机械淘汰

当当前 Tags 数量达到上限，又有新的 Tag 进入时，系统必须淘汰旧 Tag。

淘汰逻辑应该是机械式的：

```text
淘汰当前 score 最低的 Tag
```

也就是综合考虑：

- 最近是否被提到；
- 被提到的频率；
- 当前权重；
- 时间衰减；
- 是否已经不再与当前 Topic Title 匹配。

这套淘汰逻辑不应每次都依赖 LLM 判断。LLM 可以帮助生成或解释 Tags，但 Tag 的冷热管理和容量淘汰应尽量确定、稳定、可调试。

### 6.3 工作记忆而非长期记忆

Tag 被淘汰不代表信息丢失。

它只表示：

> 这个话题不再属于当前 Session 的工作焦点。

真正长期有价值的信息应该进入：

- Memory；
- Notebook；
- DID-Object；
- 归档系统。

历史 Session 如果仍然有长期价值，应通过 Self Improve 沉淀到 Memory；Session 原文可以作为 Memory Hint 指向的读取目标存在。

Session Topic 只负责表达当前 Session 的短期注意力。

---

## 7. 召回机制：从 Topic 到 Hints

OpenDAN 的召回不是直接基于用户刚说的一句话，而是优先基于当前浮现出的 Session Topic。

这点非常关键。

传统 RAG 常常是：

```text
User Message -> Vector Search -> Insert Blocks -> LLM
```

OpenDAN 更接近：

```text
User Message / Agent Reasoning
  -> update_session_topic
  -> Session Topic Tags
  -> Recall Hints
  -> Agent decides query / subscribe / ignore
```

也就是说，召回依据从“单条输入消息”升级为“当前 Session 的主题状态”。

这会带来几个好处：

1. Topic 比单条输入更稳定；
2. Topic 更接近 Agent 的任务意图；
3. Hints 可以更短、更有解释性；
4. Agent 可以基于 Hint 主动追问或查询；
5. 系统不必每次都插入大段文本块。

### 7.1 Hints 的来源

Hints 可以来自多个系统：

- Memory：用户偏好、长期事实、稳定背景；
- Notebook：用户或 Agent 记录的结构化笔记；
- DID-Object：日程、位置、设备、应用状态、远程服务状态、文件系统对象等。

历史 Session 不作为独立大来源。它会通过 Self Improve 进入 Agent Memory，并在 Memory Recall 中作为 **Session 原文类 Hint** 浮现。

例如当前 Topic 出现 `NFL`，系统可能召回：

```text
Hint: 用户长期偏好 49ers。
Source: Memory
Reason: 当前 Session Topic 包含 NFL。
```

也可能召回：

```text
Hint: 之前有一个 Session 讨论过今年超级碗门票搜索。
Source: Memory
MemoryHintType: SessionRaw
Reason: 当前 Topic 包含 NFL / ticket / event planning。
```

注意，这些 Hint 不一定包含完整门票搜索结果。它们只需要让 Agent 知道：“这件事存在，可能值得查”。

### 7.2 Agent Memory 按 Hint Type 分层召回

Agent Memory Recall 应该先按 Hint Type 分层查询候选 Hint，再分别做淘汰和保留。

推荐流程是：

```text
Session Topic Tags
  -> Session 原文类候选
  -> Event 候选
  -> Entity Observation 候选
  -> Entity Relation 候选
  -> Free Hint 候选
  -> 各类型内排序、去重、截断
  -> 跨类型合并、解释、最终预算截断
  -> Hints
```

各层的基本职责：

- **Session 原文类**：用 Tags 匹配 Agent Memory 中的历史 Session topic、title、artifact 摘要或 session index，返回可定位到原文上下文的 Hint；
- **Event**：从 Agent Memory 的事件索引里查询当前正在发生、最近发生、或曾经被 Agent 关注过的事件；
- **Entity Observation**：从 Tags 和已识别 aliases 中抽取实体，查询实体对象、状态、最近变化和可订阅字段；
- **Entity Relation**：在识别出多个实体后，查询实体之间的关系、态度、协作、冲突、依赖等关系类信息；
- **Free Hint**：在剩余预算内查询不属于上述类别的弱关联线索。

每一类都应有独立候选预算和保留预算。类内先按自身规则排序、去重、合并，再进入 Memory Hint 合并阶段。Memory Hint 合并阶段仍可以按解释性、相关性和紧迫性做二次排序，但不应完全抹掉类型预算，否则 Free Hint 或高频 Session 命中容易挤占 Event / Entity Relation 这类低频但关键的线索。

### 7.3 Attention Graph 与 Memory Hint Recall

基于 Tag 的 Memory Hint Recall 不应被理解成简单的全文检索。

Session Topic Tags 是当前工作焦点的压缩入口。Memory 召回要用这组 Tags 进入 Attention Graph，在受预算限制的一跳 / 二跳范围内找到可能相关的对象、事件、观察和关系，再返回短 Hint。

也就是说，召回链路更准确地说是：

```text
Session Topic Tags
  -> tag / phrase 候选
  -> Attention Graph 中的 Object / Event 命中
  -> 一跳关系展开
  -> 受预算限制的二跳展开
  -> 排序、去重、截断
  -> Memory Hints
```

这里的 Attention Graph 是 Owner-Centric 的：它不试图描述世界上最重要的对象，而是描述在 Agent 看来，哪些对象和事件正在与用户、当前 Session 和历史任务发生关系。

Memory Hint Recall 至少应区分四类信号：

- **Tag Match**：当前 Topic Tags 与 Memory item / observation / free hint 的 tag 或文本命中；
- **Object Attention**：命中的 item 是否关联高关注对象；
- **Event Attention**：命中的 item 是否属于高关注事件或事件子图；
- **Graph Distance**：命中结果距离当前 Topic / Object 是一跳还是二跳。

排序时，Attention Graph 不替代 tag，而是和 tag 一起参与评分：

```text
score =
  structural_match_boost
  + tag_position_boost
  + object_attention_boost
  + event_attention_boost
  + item.weight
  + item.confidence
  - recency_decay(noticed_at)
  - graph_distance_penalty
```

其中：

- `tag_position_boost` 只适用于 Memory 的 ordered tags；越靠前的 tag 命中权重越高；
- `object_attention_boost` 来自对象在 Attention Graph 中的当前权重；
- `event_attention_boost` 来自事件本身及其局部子图的热度；
- `graph_distance_penalty` 用来限制二跳扩散噪声；
- `noticed_at` 是遗忘与召回衰减的主要时间来源。

Memory 返回的仍然是 Hint，而不是事实正文：

```text
Hint:
User may prefer inspectable local memory systems over opaque embedding-only memory.

Source: Memory
Reason: matched tags "agent memory", "architecture"; linked object "Agent Memory".
Handle: item_001
```

Agent 拿到 Hint 后，可以选择忽略、读取完整 Memory item、追溯 evidence / source occasion，或者基于这个 Hint 主动询问用户。系统不应在第一阶段直接把大段 Memory / Notebook / DID-Object 正文塞入上下文；历史 Session 原文也只应通过 Memory 的 Session 原文类 Hint 按需展开。

需要特别注意 Memory 与 Notebook 的 tag 语义差异：

- Memory 使用 ordered tags；tag 顺序表达当前 Topic 的优先级，可参与排序；
- Notebook 的 tags 只做过滤，tag 顺序不影响结果排序；
- 两者共享 tag 规范化规则，但召回排序策略不能混用。

---

## 8. 机械召回与 LLM 旁路召回

从实现上，OpenDAN 可以同时保留两套召回机制：

1. 机械召回；
2. LLM 旁路召回。

### 8.1 机械召回

机械召回是低成本、确定性的。

它可以在 `update_session_topic` 调用后立即执行，例如：

- Tag 匹配；
- 全文检索；
- Memory Attention Graph 一跳 / 二跳展开；
- Notebook 标题匹配；
- Agent Memory 中的 Session 原文类匹配；
- 简单向量相似度搜索。

机械召回可以直接返回一组短 Hints。

它适合处理：

- 明确命中的长期记忆；
- 明确命中的 Notebook；
- Memory 中 Session 原文类 Hint 的标题级召回；
- 与当前 Tags 直接相关的对象列表；
- 与当前高关注对象 / 事件直接相关的 Memory Hints。

机械召回的原则是：

> 返回“可能相关的线索”，而不是把大段信息直接插入上下文。

### 8.2 LLM 旁路召回

LLM 旁路召回用于更复杂的语义判断。

它可以在 Topic 变化较剧烈、或者系统判断当前 Topic 可能需要更复杂背景理解时触发。

LLM 旁路可能判断：

- 哪些 Memory / Notebook / DID-Object 真正值得提示；
- 当前 Topic 是否意味着一个新的任务阶段；
- 是否应该恢复某个历史 Work Session；
- 是否应该关注某些动态环境状态；
- 是否应该创建半订阅；
- 是否需要在后续 User Message 前注入背景信息。

重要原则是：

> 环境感知与状态订阅，应该通过 LLM 旁路产生。

机械召回可以返回静态 Hints，但如果系统要开始关注“位置”“日程”“时间窗口”“项目目录变化”“某个外部对象状态”等动态环境，应该由 LLM 旁路判断其必要性。

---

## 9. LLM 旁路触发阀门

LLM 旁路有成本，因此不能每次 `update_session_topic` 都触发。

需要一个阀门机制。

核心可以由两个因素控制。

### 9.1 距离上一次旁路触发的距离

系统应记录上一次 LLM 旁路触发时间或轮次：

```text
last_llm_recall_at
```

如果上一次刚触发过，则短时间内不应重复触发。

这个距离可以按：

- 对话轮次；
- 时间间隔；
- Agent step 数；
- Topic 更新次数；

来计算。

### 9.2 Topic 改变的剧烈程度

另一个因素是 Topic Change Intensity。

如果 Session Topic 长时间稳定，但突然出现明显变化，就应提高触发概率。

典型信号包括：

- 一次加入多个新 Tags；
- 多个旧 Tags 被淘汰；
- Topic Title 语义方向明显变化；
- Tag 权重分布大幅变化；
- 用户从一个任务域跳到另一个任务域；
- 当前 Topic 命中了需要动态环境感知的领域，例如旅行、booking、日程安排、位置相关任务。

可以抽象成：

```text
should_trigger_llm_side_channel =
  distance_from_last_trigger > cooldown_threshold
  AND topic_change_intensity > change_threshold
```

实际实现中也可以是加权分数：

```text
trigger_score =
  α * distance_score
  + β * new_tag_score
  + γ * evicted_tag_score
  + δ * semantic_shift_score
  + ε * environment_sensitivity_score
```

当 `trigger_score` 超过阀门值时，触发 LLM 旁路。

参考默认（v0.2）：

```text
distance_threshold = 5      # 距离上一次召回的对话轮次
change_threshold   = 0.5    # (added + removed) / tag_set.len()

# 阀门判定三态
if change_ratio >= change_threshold:
    return LLM           # 剧烈变化 → 走深度路径
elif turns_since_last_recall >= distance_threshold:
    return Mechanical    # 距离够 → 走轻量路径
else:
    return NotTriggered  # 阀门未突破，直接 success 返回
```

> 阀门策略必须可配置（通过 `RecallPolicy` 结构注入），不得硬编码常量。

### 9.3 旁路不是必然召回全文

即使触发了 LLM 旁路，也不意味着一定要读取大量数据。

LLM 旁路可能只产生：

- 一组更精确的 Hints；
- 一组待关注 DID-Objects；
- 一组半订阅；
- 一个“暂不召回”的决策；
- 一个对当前 Topic 的重写或合并建议。

---

## 10. 召回时机：立即召回与延迟召回

召回不一定发生在 `update_session_topic` 的同一次调用中。

系统可以有两类召回时机。

### 10.1 立即召回

当 Topic 变化较明显，或者机械检索命中明确对象时，`update_session_topic` 可以直接返回 Hints。

流程是：

```text
update_session_topic
  -> 机械式更新 Tag 权重
  -> 执行 Tag 淘汰
  -> 调用 RecallService
  -> 机械召回 Memory / Notebook / DID-Object Hints
  -> 返回 Hints
```

这种模式适合低成本召回，尤其适合：

- Memory 中的一句话事实；
- Notebook 中的标题级线索；
- Memory 中 Session 原文类的摘要级线索；
- 明确命中的对象引用。

### 10.2 延迟召回 / 半订阅召回

另一类召回并不立即返回完整 Hints，而是创建状态关注关系。

例如当前 Topic 涉及：

- 旅行计划；
- booking；
- 日程安排；
- 地点变化；
- 开发目录变化；
- 某个外部对象的实时状态。

LLM 旁路可能判断：

> 当前 Session 应该关注用户位置、日程或某个对象状态的变化。

于是系统不是立刻把所有信息读出来，而是创建一个半订阅。

后续当：

- 下一条 User Message 到达；
- 被订阅对象状态发生变化；
- Background Environment 编织阶段执行；

系统再把相关变化作为背景信息插入到 LLM 输入前。

示意：

```text
User Message 到达
  -> Context Weaver 检查当前 Session Topic
  -> 检查半订阅对象是否变化
  -> 生成 Background Information
  -> 插入到 User Message 前
  -> LLM 推理
```

这类似系统在用户输入前插入当前时间，但范围可以扩展到更多动态状态。

### 10.3 工程契约：互斥二选一 + 同步等待

> **机械召回与 LLM 召回是互斥的二选一关系**。一旦判定触发召回，`update_session_topic` 工具调用会**同步等待召回完成**才返回 `success`，召回结果作为工具返回值的一部分交付。

含义：

- 调用方（Agent）拿到 `success` 时可以确信：要么没召回（`recall` 字段为 None），要么召回已完成；
- **不存在“工具已返回但召回还在后台跑”的中间状态**；
- 简化 Agent 侧状态机：无需处理异步召回的事件回调；
- LLM 旁路超时（默认 10s）归入 `Failed` 状态，工具仍 `success`。

若未来需要异步语义，应另开新的召回入口（如 `OnUserMessageEnter` 钩子），而不是改变本工具的契约。

---

## 11. 半订阅机制：介于自动插入与主动查询之间

半订阅是 OpenDAN 观察世界机制中的一个重要设计。

它既不是每次都把对象完整内容塞进上下文，也不是完全等 Agent 主动查询。

它的基本思想是：

1. Topic 浮现后，系统识别出一组相关 DID-Objects；
2. Agent 或 LLM 旁路判断这些对象值得关注；
3. 系统订阅这些对象的某些状态变化；
4. 当变化发生时，以 Background Hint 的方式注入上下文；
5. Agent 再决定是否读取完整对象。

例如：

### 11.1 旅行计划

当前 Topic 包含：

```text
旅行计划
Booking
机票
酒店
地点
```

LLM 旁路可能创建订阅：

```text
订阅用户当前位置变化
订阅用户时区变化
订阅相关日程变化
订阅航班价格或 booking 状态变化
```

当用户位置变化时，系统可以在下一次输入前插入：

```text
Background Hint:
用户当前位置已从 A 城市变化到 B 城市。当前 Session Topic 涉及旅行计划和 booking，因此该位置变化可能影响推荐、时间换算或票务选择。
```

### 11.2 日程安排

用户说：

```text
我明天中午要和某某吃饭。
```

Topic 浮现：

```text
日程
午餐
某某
明天中午
```

系统可能半订阅或检查日历冲突。如果发现变化或冲突，注入：

```text
Background Hint:
明天中午用户日程中已有一个可能冲突的安排。当前 Session Topic 涉及约饭安排，建议确认时间冲突。
```

### 11.3 开发目录变化

当前 Topic 涉及某个代码项目。

系统可能订阅项目目录状态。

如果该目录在其它地方被修改，后续输入前可注入：

```text
Background Hint:
当前 Session 相关的开发目录在外部发生了变化。建议在继续修改前检查最新文件状态。
```

---

## 12. 召回结果如何显示与注入

召回结果不应该简单等同于“把一堆文本插入上下文”。

OpenDAN 中可以把召回结果分为几种显示层级。

### 12.1 工具返回级 Hints

这是 `update_session_topic` 的直接返回。

适合展示给 Agent，而不是直接展示给用户。

格式应短小、结构化、带理由：

```text
Hint:
- Type: Memory
- Summary: 用户偏好 49ers。
- Reason: 当前 Topic 包含 NFL。
- Action: 如需要讨论球队偏好，可主动查询该 Memory 详情。
```

### 12.2 Background Environment 注入

这是在下一次 LLM 输入前插入的背景信息。

它类似当前时间、地点等环境信息，但必须说明为什么出现。

示意：

```text
[Background Environment]
- 当前时间：2026-05-26 18:30 America/Los_Angeles
- 当前 Session Topic：旅行计划 / Booking / 地点
- 相关状态变化：用户位置发生变化，可能影响当前旅行计划。
```

这类信息不是用户主动说的，但它是当前推理环境的一部分。

### 12.3 Event Hint 注入

当半订阅对象发生变化时，可以插入事件级 Hint：

```text
[Background Event]
Object: calendar://user/main/events/2026-05-27-noon
Change: 发现明天中午存在已有安排
Reason: 当前 Session Topic 涉及午餐约会安排
Suggested Action: 在确认约饭前检查日程冲突
```

### 12.4 不直接暴露完整读取方法

在很多情况下，第一阶段 Hint 不应直接暴露完整读取方法。

原因是：

> 一旦 Agent 知道可以读取某个对象的完整路径，它可能会非常积极地查询，造成大量冗余读取。

因此可以采用分阶段披露：

1. 第一阶段只暴露对象存在和相关理由；
2. 如果状态变化发生，或 Agent 明确需要更多信息，再暴露读取方法；
3. 对于一些高频对象，只允许订阅，不允许主动全量读取；
4. 对于敏感对象，必须通过权限和策略控制。

这是一种工程上的频率控制。

---

## 13. Context Weaving：每次输入前都有机会基于 Topic 编织上下文

`update_session_topic` 不只是一个更新 Tags 的工具，它更像是 Context Weaving 的入口之一。

只要当前 Session Topic 仍然新鲜，那么每一次新的 LLM 输入前，系统都有机会基于它编织上下文。

流程可以是：

```text
New User Message
  -> 读取当前 Session Topic
  -> 检查 Topic Tags 是否仍然新鲜
  -> 检查机械 Hints 是否需要刷新
  -> 检查半订阅状态是否变化
  -> 生成 Background Information
  -> 与 User Message 一起输入 LLM
```

这意味着：

- 召回可以发生在 `update_session_topic` 当下；
- 也可以发生在下一条 User Message 到达前；
- 也可以由订阅对象状态变化触发；
- 还可以由 Context Weaver 按策略周期性检查。

关键原则是：

> Topic 是持续存在的短期工作状态，而不是一次性检索 query。

---

## 14. 总体流程图

```mermaid
flowchart TD
    A[User Message / Agent Reasoning] --> B[Agent decides to call update_session_topic]
    B --> C[Update Topic Title and Tags]
    C --> D[Mechanical Tag Weight Update]
    D --> E[Mechanical Tag Eviction]

    E --> F{Immediate Mechanical Recall?}
    F -- yes --> G[Return Short Hints]
    F -- no --> H[Fast Return]

    E --> I{LLM Side-channel Gate}
    I -- not triggered --> H
    I -- triggered --> J[LLM Side-channel Recall]
    J --> K[Better Hints]
    J --> L[DID-Object Candidates]
    J --> M[Semi-subscriptions]

    G --> N[Agent May Query Details]
    K --> N
    L --> N
    M --> O[Subscription Manager]

    P[Next User Message] --> Q[Context Weaver]
    O --> Q
    C --> Q
    Q --> R[Background Environment / Event Hints]
    R --> S[LLM Input]
```

---

## 15. 推荐的内部模块划分

可以把整个系统拆成以下模块。

### 15.1 SessionTopicManager

负责：

- 接收 `update_session_topic` 调用；
- 更新 Topic Title；
- 合并或新增 Tags；
- 维护 Tag 权重；
- 执行容量限制和淘汰；
- 计算 Topic Change Intensity。

### 15.2 TopicTagStore

负责存储当前 Session 的 Topic 状态。

每个 Tag 可以包含：

```json
{
  "name": "Session Topic",
  "weight": 0.86,
  "first_seen_at": "...",
  "last_seen_at": "...",
  "mention_count": 5,
  "source": "agent_tool_call",
  "status": "active"
}
```

### 15.3 HintRecallEngine

负责机械召回。

输入：

- Topic Title；
- 当前 Tags；
- Tag 权重；
- 当前 Session 已识别出的 Objects / Aliases；
- Attention Graph Snapshot（对象权重、事件权重、关系距离）；
- Session ID；
- User / Agent profile；
- 检索预算。

输出：

- Memory Hints；
- Notebook Hints；
- DID-Object Hints；
- 去重后的短线索集合。

Agent Memory 产出的 `RecallItem` 必须同时携带 `memory_hint_type` 和 `source_system`。前者用于 Memory 内部策略分层、类内淘汰和上下文呈现；后者用于说明该 Hint 由 Agent Memory 产出。若 Hint 指向 Session 原文、实体对象或事件，还应额外携带目标 handle，供后续读取真实数据。

Memory Hints 的机械召回应通过独立的 MemoryRecallProvider 完成。HintRecallEngine 只负责编排，不直接实现 Memory 图展开。

MemoryRecallProvider 的职责：

- 使用 ordered tags 做 tag / phrase 候选召回；
- 使用 objects / aliases 做 entity、pair、relation 的机械展开；
- 结合 Attention Graph 的 object attention、event attention 和 graph distance 排序；
- 过滤 inactive、forgotten、salience 等不可召回项；
- 按 Session 原文类、Event、Entity Observation、Entity Relation、Free Hint 五类分别设置候选预算和保留预算；
- 对每类候选分别排序、去重、截断，再合并为最终 Memory Hints；
- 返回短 Hint、reason、matched tags、source handle 和必要的 debug metadata。

MemoryRecallProvider 不负责：

- 维护 Session Topic Tags；
- 写入或修改 Memory；
- 判断 Notebook 正文是否应进入上下文；
- 创建半订阅；
- 调用 LLM 做复杂语义筛选。

### 15.4 LLMSideChannel

负责复杂语义判断。

触发后，它可以判断：

- 当前 Topic 是否代表重要迁移；
- 哪些 Hints 真正值得保留；
- 是否需要创建半订阅；
- 是否需要关注环境状态；
- 是否需要延迟召回而不是立即召回。

### 15.5 SubscriptionManager

负责半订阅。

它维护：

- 订阅对象；
- 订阅字段；
- 订阅理由；
- 对应 Topic；
- TTL；
- 最近触发时间；
- 是否允许暴露读取方法。

订阅生命周期：

- **创建**：仅由 LLM 召回路径产出；
- **TTL**：默认与产生它的 Tag 绑定 —— 订阅记录中显式带 `bound_tags: Vec<String>`，**Tag 被淘汰，对应订阅自动失效**；
- **去重**：同一类订阅（如“地理位置”）多次注册时合并为一条，更新最新 Tag 关联；
- **退订**：Session 结束时全部清理；或 Tag tier 被降级至淘汰时清理。

订阅与 Tag 在数据结构上必须**显式关联**，以支持级联清理；这是实现层的硬约束，不可省。

### 15.6 ContextWeaver

负责在 LLM 输入前编织背景信息。

它读取：

- 当前 Session Topic；
- 仍然有效的 Hints；
- 半订阅事件；
- 当前时间、地点等环境信息；
- 策略预算。

然后生成简短、可解释的 Background Information。

### 15.7 DIDObjectSystem

负责把现实世界的数字状态抽象成对象系统。

它提供：

- 对象定位；
- 状态路径；
- 事件订阅；
- 可选读取；
- 权限控制；
- 对象到 Hint 的解释能力。

### 15.8 三子系统硬解耦：RecallService 抽象

整套机制由三个**独立可演化**的子系统构成：

| 子系统 | 职责 | 何时执行 |
|---|---|---|
| **A. Topic / Tag 更新** | 维护 Session Tag 集合（增删 + 淘汰） | `update_session_topic` 调用时 |
| **B. 基于 Topic 的召回** | 以当前 Tag 为背景信息，从 Memory / Notebook / DID-Object 中检索条目 | 任意触发点（当前仅 `update_session_topic`，但接口开放） |
| **C. 召回信息的呈现** | 决定召回结果以何种形式、在何时进入 LLM 上下文 | 立即返回 / 背景注入 / 状态订阅触发 |

> **关键约束**：三者必须通过定义良好的数据交换协议解耦。实现时，B 子系统必须抽象为独立的 `RecallService`，**不得**把召回逻辑直接耦合到 `update_session_topic` 工具实现内部。`update_session_topic` 是 `RecallService` 的**调用方，不是实现者**，两者必须在不同的模块 / crate 中。

参考 trait 定义（Rust 风格示意）：

```rust
trait RecallService {
    async fn recall(
        &self,
        tags: &TagSet,
        mode: RecallMode,
        policy: &RecallPolicy,
    ) -> RecallResult;
}

enum RecallMode {
    Mechanical,
    LLM,
    Auto,  // 由 policy 的阀门判定决定
}

enum RecallResult {
    NotTriggered,
    Recalled {
        items: Vec<RecallItem>,
        subscriptions: Vec<Subscription>,  // 仅 LLM 路径可能产出
    },
    Failed { reason: String },
}
```

未来可在其他位置接入新的召回入口（例：`OnUserMessageEnter`、长时间无活动、外部事件到达），复用同一个 `RecallService`。

---

## 16. 策略控制点

这套系统的价值很大程度上取决于策略可调。

### 16.1 Tag 数量上限

默认可以从 8 个左右开始。

数量过少会漏掉并行工作主题；数量过多会导致召回发散。

### 16.2 Tag 衰减速度

衰减速度决定 Topic 的“记忆长度”。

- 衰减太快：Session 容易丢失刚才讨论过的方向；
- 衰减太慢：旧 Topic 过久残留，导致召回污染。

### 16.3 LLM 旁路冷却时间

控制旁路成本。

- 冷却太短：成本高，频繁打扰；
- 冷却太长：Topic 迁移时反应迟钝。

### 16.4 Topic Change Intensity 阈值

决定什么时候认为话题发生剧烈变化。

可结合：

- 新 Tag 数量；
- 淘汰 Tag 数量；
- Title 语义变化；
- Tag 权重变化；
- 当前领域是否环境敏感。

### 16.5 Hint 数量预算

Hints 应该少而精。

推荐原则：

- 一次不要返回太多；
- 总预算之外，Agent Memory 还应按 Memory Hint Type 设置独立候选预算和保留预算；
- Memory Hint 每类先独立淘汰，再进入 Memory 内部合并；
- 优先返回解释性强的；
- 优先返回“存在性线索”；
- 避免直接插入大段内容；
- 对重复 Hints 做合并。

### 16.6 读取方法披露策略

为了避免 Agent 过度查询，可以分层披露能力：

1. 只告诉 Agent 有相关对象；
2. 允许订阅对象变化；
3. 在事件发生或任务需要时才暴露读取方法；
4. 对高成本对象设置查询预算；
5. 对敏感对象设置权限确认。

### 16.7 策略矩阵参考默认值

集中列出 v0.2 的可调维度及默认值，方便实现者一次性看清：

| 维度 | 选项 | 当前默认 |
|---|---|---|
| Tag 容量 | 整数 | 8 |
| Tag 分层 | pinned / active / transient | 全 transient（v0.2 简化） |
| 衰减常数 TAU | 时长 | 30 分钟 |
| 距离阀门 | turn 间隔 | 5 |
| 剧烈度阀门 | (add+remove)/total 比例 | 0.5 |
| 召回路径选择 | mechanical / llm / auto | auto |
| 召回入口 | `update_session_topic` | 仅此一个（接口开放） |
| Hint 总保留预算 | 整数 | 由 `RecallPolicy` 配置 |
| Agent Memory Hint Type 候选预算 | per-type 整数 | 由 `RecallPolicy` 配置 |
| Agent Memory Hint Type 保留预算 | per-type 整数 | 由 `RecallPolicy` 配置 |
| 呈现通道 | tool result / bg inject / sub trigger | 三通道并存 |
| LLM 召回超时 | 时长 | 10s |
| topic title 长度 | 字符上限 | 120 |

---

## 17. 与传统 RAG 的关键区别

OpenDAN 的 Hints 体系与传统 RAG 的区别可以总结如下。

| 维度 | 传统机械 RAG | OpenDAN 浮现式 Hints |
|---|---|---|
| 触发依据 | 用户当前输入 | Session Topic 状态 |
| 插入内容 | 文本块 | 线索、对象、事件、背景说明 |
| 插入时机 | 通常每次输入前 | Topic 更新、输入前编织、状态变化 |
| 是否需要 LLM 判断 | 通常不需要 | 可通过 LLM 旁路判断复杂情况 |
| Agent 是否理解来源 | 弱 | 强，Hint 带理由 |
| 上下文占用 | 高 | 低 |
| 对未知未知的处理 | 依赖召回命中 | 通过 Topic 浮现和 Hint 告知存在性 |
| 对动态环境的处理 | 不擅长 | 通过半订阅处理 |
| 后续行动 | 通常直接让 LLM 消化文本 | Agent 可查询、订阅或忽略 |

最核心的差异是：

> OpenDAN 不是把检索结果当作答案上下文，而是把召回结果当作 Agent 观察世界时浮现出的线索。

---

## 18. 示例：NFL 话题

用户开始讨论 NFL。

Agent 调用：

```text
update_session_topic(
  title = "讨论 NFL 相关内容",
  tags = ["NFL", "football", "sports"]
)
```

系统执行：

1. 更新 Tags；
2. 如果 Tags 已满，机械淘汰低权重旧 Tag；
3. 机械召回 Memory；
4. 命中用户偏好 49ers；
5. 命中 Memory 中的 Session 原文类线索：之前搜索过超级碗门票。

返回 Hints：

```text
Hint 1:
用户可能偏好 49ers。
Source: Memory
Reason: 当前 Topic 包含 NFL。

Hint 2:
曾有历史 Session 讨论今年超级碗门票搜索。
Source: Memory
MemoryHintType: SessionRaw
Reason: 当前 Topic 包含 NFL / tickets / event。
```

Agent 可以继续决定：

- 是否在回答中考虑用户偏好；
- 是否查询历史 Session 的完整内容；
- 是否询问用户是否继续之前的票务计划。

---

## 19. 示例：旅行和 Booking

用户开始规划旅行。

Topic Tags：

```text
旅行计划
Booking
机票
酒店
地点
时间
```

由于旅行和 booking 对环境状态敏感，LLM 旁路可能触发。

旁路判断：

- 当前任务可能依赖用户位置；
- 当前任务可能依赖用户时区；
- 当前任务可能依赖日程；
- 不需要立即读取全部位置历史；
- 应创建半订阅。

于是系统创建：

```text
Subscribe: user.location.current
Reason: 当前 Session Topic 涉及旅行计划和 booking。

Subscribe: user.calendar.upcoming
Reason: 当前 Session Topic 涉及出行安排，日程可能影响建议。
```

下一次用户输入前，Context Weaver 可以注入：

```text
[Background Environment]
当前 Session Topic 涉及旅行计划与 booking。
用户当前位置为旧金山湾区，时区为 America/Los_Angeles。
如涉及出发时间、航班日期或酒店入住时间，应考虑该时区。
```

如果位置变化，则注入事件 Hint，而不是每轮读取完整位置历史。

---

## 20. 示例：代码项目目录变化

当前 Session 在处理某个开发项目。

Topic Tags：

```text
OpenDAN
Agent Runtime
代码修改
Session Topic
```

系统召回一个 DID-Object：

```text
Object: project://opendan/runtime
Reason: 当前 Session 正在讨论 Agent Runtime 相关实现。
```

LLM 旁路可能不直接暴露读取整个目录的方法，而是创建订阅：

```text
Subscribe: project://opendan/runtime/** change events
```

如果目录在其它地方发生变化，系统在下一次输入前注入：

```text
[Background Event]
当前 Session 相关项目目录已在外部发生变化。
建议继续修改前检查最新状态，避免基于旧文件推理。
```

这样可以避免 Agent 没事就反复读取整个目录，同时仍然让它感知关键变化。

---

## 21. 设计原则总结

### 21.1 Topic 是工作记忆，不是长期记忆

Session Topic 只表达“当前正在做什么”。

它必须有上限、会衰减、会淘汰。

### 21.2 Hint 是线索，不是答案

Hint 的目标是告诉 Agent 某件事可能存在，而不是把完整事实塞进上下文。

### 21.3 召回应基于 Topic，而不是仅基于当前输入

Topic 比单条用户消息更稳定，也更符合 Agent 的任务意图。

### 21.4 机械逻辑负责稳定性，LLM 旁路负责语义判断

Tag 淘汰、权重衰减、基础召回应尽量机械、可预测。

复杂环境感知、状态订阅、语义迁移判断可以交给 LLM 旁路。

### 21.5 环境状态优先通过半订阅处理

对动态对象，不鼓励每次主动读取。

更好的方式是：

- 先发现对象；
- 再订阅变化；
- 变化发生时注入 Hint；
- 必要时再读取详情。

### 21.6 Context Weaving 是持续过程

`update_session_topic` 不只是一次工具调用。

只要 Topic 新鲜，每次 LLM 输入前都有机会基于 Topic 编织背景信息。

---

## 22. 系统边界与非目标

明确写出本机制**不做**什么、以及它和邻居系统的边界，是避免功能蔓延的关键。

### 22.1 与邻居系统的关系

- **与 session-history**：history 是真理源（事件级），topic 是题眼（语义级）。每次 `update_session_topic` 成功调用都会追加 `session_topic_updated` 历史事件；`topic.md` 是最后一次调用形成的当前态投影。
- **与 AgentNotebook**：AgentNotebook 是 Session 内的工作笔记；topic 是 Session 间被浮现的题眼。即便实现层把 topic 作为 AgentNotebook 的一个特化条目，**对外接口必须独立**，避免和普通笔记混淆。
- **与 auto memory**（跨 Session 的 user / feedback / project / reference 类长期记忆）：topic 不进 auto memory；topic 是“短期 Session 题眼”，由浮现机制时效性消费。
- **与 LLMContext**：本工具作为 standard tool 注册。Tool Result 遵循 `success | error | pending` 协议，但**永不返回 pending** —— 召回总是同步等待完成。即便召回失败也返回 `success` + `recall_status=Failed`，因为 Tag 更新的保底语义已完成。

### 22.2 与浮现层的对接面

浮现层是另一个子系统，本机制只产出它需要的原料。要保证：

1. **可枚举**：浮现器能列出所有有 topic 的 Session（`/{sessions_root}/*/.meta/topic.md`）；
2. **可粗筛**：`topic.md` 的 frontmatter（`updated_at`, `tags`）支持时间 / 标签维度的筛选；
3. **可定位**：`topic.md` 与 `session_id` 是 1:1 映射，浮现器拿到 hit 后能直接定位到 `session_dir` 做深度调用；
4. **当前 Session Tag 可读**：浮现层（含 `RecallService`）能读取 `tag_set.json`；
5. **订阅可读**：呈现层能读取 `subscriptions.json`。

“工具可见性也是被浮现的对象”（即一个无浮现内容的 Session 应当看不到任何 memory 相关 prompt 段）由浮现层自己实现；`update_session_topic` 本身**始终可见**（它是 Notebook 工具，不参与浮现）。

### 22.3 明确不做的事

记录已识别但**不在本机制范围内**的事情，避免将来误把功能塞进来：

- **不做 Session Summary**：那是另一种产物，篇幅更长、面向人类阅读；
- **不做自动 topic 抽取**：让模型自己判断什么时候写、写什么 —— 这是 Notebook 的语义；
- **不做跨 Session 的 topic 合并 / 去重**：浮现层的职责；
- **不做 embedding / 向量索引**：浮现层若需要，自己读 `topic.md` 建；
- **不实现浮现层的索引、检索、注入逻辑**：那些是另一个工单；
- **不实现 Tag 的 tier 升降级**（v0.2 全部 Transient，预留接口）；
- **不实现异步召回入口**：本工具同步语义；异步入口未来另开；
- **不在本工具内实现订阅状态监听器**：订阅中心是独立组件。

### 22.4 未来扩展（不在 v0.2 范围）

- **Tag tier 升降级**：由 LLM 召回路径决定，把高价值 Tag 提升到 Active / Pinned，避免被机械淘汰；
- **异步召回入口**：例如 `OnUserMessageEnter` 钩子，允许在不阻塞主路径的前提下做后台召回；
- **召回入口的策略路由**：不同入口注入不同的 `RecallPolicy`（关键路径上更保守、空闲时更激进）；
- **跨 Session 的浮现器**：本文档只覆盖 Session 内的 Tag 维护与召回触发；跨 Session 的 topic 浮现由独立工单。

---

## 23. 实现验收清单

本节给实现者使用，避免把架构原则误读成可选建议。

### 23.1 工具基础语义

- [ ] 工具名为 `update_session_topic`，参数至少包含 Topic Title 和 Topic Tags；
- [ ] 写入路径固定为 `{session_dir}/.meta/topic.md`，内容为 frontmatter + body 结构；
- [ ] `topic.md` 表达当前态，覆盖写；`round_history` / `topic_log.jsonl` 表达调用历史，append-only；
- [ ] 相同 Title 重复调用时，`topic.md` 不应无意义重写，但 Tags 仍要参与权重增强、淘汰和可能召回；
- [ ] Title 必须是单行、≤ 120 字符；
- [ ] 从未写入 Topic 的 Session 不生成空 `topic.md`，浮现层应容忍缺失；
- [ ] 工具描述必须明确：只在主题首次明确、显著漂移或进入长尾时调用；写给未来的 Agent；不是 Session Summary；
- [ ] 不引入新的 RPC / DB；不暴露专用 read / list API。

### 23.2 Tag 集合维护

- [ ] `tag_set.json` 可读取、更新和持久化；
- [ ] 同名 Tag 权重增强并更新 `last_touched`；
- [ ] 新 Tag 默认 `weight=1.0`，`tier=Transient`；
- [ ] 容量超限时按 `weight * exp(-(now - last_touched) / TAU)` 淘汰最低分 transient Tag；
- [ ] v0.2 保留 `tier` 字段，但全部按 Transient 处理；
- [ ] Tag 容量和 TAU 通过配置注入，不硬编码。

### 23.3 召回触发与交付

- [ ] 阀门判定使用距离阀门和剧烈度阀门；
- [ ] `change_ratio >= change_threshold` 映射为 LLM 召回；
- [ ] `turns_since_last_recall >= distance_threshold` 映射为 Mechanical 召回；
- [ ] 阀门未突破时返回 `NotTriggered`；
- [ ] 召回策略通过 `RecallPolicy` 注入，不硬编码；
- [ ] `update_session_topic` 只调用 `RecallService`，不得内嵌具体召回逻辑；
- [ ] 机械召回与 LLM 召回互斥二选一；
- [ ] 工具同步等待召回完成，永不返回 `pending`；
- [ ] LLM 召回默认 10s 超时，超时归入 `Failed`；
- [ ] 召回失败不影响 Tag 更新成功，工具仍返回 `success` + `recall_status=Failed`；
- [ ] `UpdateSessionTopicResult` 至少包含 `tag_set_diff`、`recall`、`recall_status`。

### 23.4 RecallService 与订阅

- [ ] 定义 `RecallService` trait，并通过 trait 注入到 `update_session_topic`；
- [ ] 提供机械召回实现，用于基于 Tag 的 Memory / Notebook / DID-Object 短 Hint 召回；
- [ ] 提供 LLM 旁路召回实现，用于语义筛选、动态环境判断和半订阅创建；
- [ ] 提供 mock RecallService 用于单元测试和工具集成测试；
- [ ] LLM 召回产出的订阅写入 `{session_dir}/.meta/subscriptions.json`；
- [ ] 订阅记录显式携带 `bound_tags`，Tag 淘汰时可级联清理；
- [ ] 同类订阅可去重合并，Session 结束时清理。

### 23.5 测试要点

- [ ] Tag 淘汰算法单元测试：容量满、同名累加、时间衰减、tier 预留字段；
- [ ] 阀门判定单元测试：距离阀门、剧烈度阀门、NotTriggered / Mechanical / LLM 三态；
- [ ] 工具调用集成测试：mock RecallService 返回 NotTriggered、Recalled、Failed；
- [ ] 幂等性测试：相同 Title 重复调用不重复写当前态，但仍处理 Tags；
- [ ] 召回失败 / 超时测试：工具返回 `success`，`recall_status=Failed`；
- [ ] 订阅写入和 Tag 级联清理测试；
- [ ] 缺失 `topic.md` 测试：浮现层枚举时能跳过无 Topic 的 Session。

---

## 24. 最终架构定位

OpenDAN 的观察世界机制可以概括为：

> 以 Session Topic 为当前工作记忆，以 Hints 作为相关信息的存在性线索，以 DID-Object 作为现实世界状态的统一抽象，以半订阅和 Context Weaving 控制动态状态注入，从而在机械 RAG 和 Agent 主动查询之间建立一套更符合 Agent 推理流的观察系统。

它要解决的不是简单“如何把更多资料塞给 LLM”，而是：

> 如何让 Agent 在有限上下文中，知道当前世界中有哪些事情可能和它正在做的任务有关。

这也是 OpenDAN Cross-session Memory / Notebook / DID-Object System 的核心连接点。

最终，Agent 的观察路径不再是单一的：

```text
输入 -> 检索 -> 塞上下文
```

而是：

```text
输入 / 推理
  -> Topic 浮现
  -> Hints 浮现
  -> 对象被感知
  -> 决定查询或订阅
  -> 状态变化通过 Background Environment 编织回来
  -> Agent 基于更完整的现实观察继续推理
```

这套机制让 Agent 既不会被机械 RAG 的上下文洪水淹没，也不会因为完全依赖主动查询而陷入“不知道自己不知道”的盲区。
