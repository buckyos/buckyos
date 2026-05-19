# update_session_topic 工具需求

> 状态：设计草案 v0.2（含浮现机制设计）

---

## 1. 背景：Notepad 与 Memory 的本质区分

主流 memory 方案把两种不同的东西混成一个系统，导致两边都不到位。我们要分开：

| 维度 | Notepad | Memory |
|---|---|---|
| 写入语义 | "请记下来"（显式） | "刚才发生了"（沉淀） |
| 内容形态 | 事实 / 规则 / 提醒 | 经历 / 感觉 / 线索 |
| 召回需求 | 精确匹配 / 必然召回 | 联想浮现 / 可能相关 |
| 写入者 | 用户主导（含 Agent 显式调用） | 系统主导 |
| Prompt 位置 | system prompt | 紧贴 user msg |
| 失败模式 | 漏掉 = 灾难 | 没浮现 = 可接受 |

`update_session_topic` **属于 Notepad 子系统的工具**——由 Agent 显式调用、写入精确的事实条目（本 session 当前在谈什么）。但它写入的内容会被 **Memory 浮现层** 消费：未来 session 在合适时机浮现"昨天讨论过 X (session: …)"时，**topic 行就是浮现产物的来源**。

简言之：**写入是 Notepad 语义，消费是 Memory 语义**。这条工具是把"事实级 topic"和"启发式浮现"对接的桥。

此外，`update_session_topic` 本身也是当前 session 内**浮现机制的唯一触发入口**（详见 §5），它同时承担"写入题眼"和"驱动浮现"两类职责。

---

## 2. 工程模型：浮现 = pointer + hint + 延迟具象化

参见 [session-history.md](session-history.md) 已经把 history 作为文件系统资产挂在 `{session_dir}/` 下。Memory 浮现机制复用同一个抽象：

1. **"这里有东西"**：浮现产物只是 *awareness*——一个 session 存在、它大致谈了什么
2. **"它在哪"**：Session ID 作为锚点，文件系统路径作为统一 schema（不引入专门的 memory API）
3. **"用的时候再展开"**：浮现阶段每条线索只占 20–50 token；模型判断要不要深入时再触发 read 文件级别的"深度调用"

`update_session_topic` 写入的 **topic 行** 就是"hint" 这一层；session_id 就是 pointer；session_dir 下的 history / artifacts 就是延迟具象化的目标。

---

## 3. 工具语义

### 3.1 接口

```
update_session_topic(
    topic: String,
    tags?: Vec<String>,
) -> UpdateSessionTopicResult
```

**入参**：
- **topic**：一行自然语言，描述本 session 当前正在谈的主题。约束：单行、≤ 120 字符、人类可读、对未来的"我"友好
- **tags**：可选，自由形态短标签，用于浮现层做粗筛（embedding/tag 召回）。不强 schema、不预定义词表

**返回值**（关键：携带浮现产物）：
```rust
struct UpdateSessionTopicResult {
    // Tag 集合更新结果（机械操作，必然存在）
    tag_set_diff: TagSetDiff {
        added: Vec<String>,
        removed: Vec<String>,
        current: Vec<TagEntry>,
    },

    // 召回结果（可能为空，参见 §5）
    recall: Option<RecallPayload>,

    // 召回状态（即便召回失败，工具仍 success）
    recall_status: RecallStatus,
}

enum RecallStatus {
    NotTriggered,           // 阀门未突破，未触发召回
    Mechanical { ms: u32 }, // 机械召回完成
    LLM { ms: u32 },        // LLM 召回完成
    Failed { reason: String }, // 召回失败，但 Tag 更新已成功
}
```

### 3.2 调用语义

- **覆盖写**（非 append）。一个 session 同一时刻只有一个 topic
- **幂等**：相同 topic 内容重复调用对 `topic.md` 是 no-op；但 tags 的变化仍会触发 Tag 集合更新与可能的召回
- **历史可追溯但不必暴露给 LLM**：底层把每次更新落到 `topic_log.jsonl`（运维 / 审计用），但工具对外只承诺"当前 topic = 最后一次写入"
- **同步语义**：工具调用同步等待 Tag 更新 + 召回（若触发）完成才返回 `success`。详见 §5.3

### 3.3 调用时机（由 Agent 自主决定，不由系统强制）

期望模型在以下时机调用：

- 用户首条消息已让主题清晰 → 写入第一版 topic
- 话题发生**显著漂移** → 覆盖更新（不是每轮都改；细微展开不算漂移）
- session 即将进入长尾 / 接近结束 → 写入一份"最终主题"

**反例**（不应触发）：用户随手聊一句、和当前任务无关的边角问答、模型自己内部的中间步骤。

主题更新本身是 Agent 的元行为，调用频率应该远低于普通工具调用。

---

## 4. 存储与文件布局

复用 session_dir 作为锚，避免引入第二套 memory 数据库。

```
{session_dir}/
  .meta/
    topic.md           # 当前 topic（覆盖写）
    topic_log.jsonl    # 历次更新审计（append-only，可选）
    tag_set.json       # 当前 Tag 集合状态（含权重、时间戳、tier）
    subscriptions.json # 当前活跃的状态订阅（由 LLM 召回路径产生）
  round_history/       # 见 session-history.md
  ...
```

- `topic.md` 内容形态：
  ```markdown
  ---
  session_id: 2026-05-12-xxx
  updated_at: 2026-05-12T15:30:00Z
  tags: [llm-context, design]
  ---

  讨论 LLM Context 设计与"浮现"式 Memory 的工程实现
  ```
- `tag_set.json` 内容形态：
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
- 全局索引由浮现层另行维护（见 §6），**本工具不负责索引**。只承诺：写完 `topic.md` 后浮现层最终能看到

### 4.1 与 session_dir 的关系

- session_dir 路径由 AgentSession 决定，本工具不发明路径
- 工具实现从 session 上下文拿 session_dir，写到 `.meta/topic.md`
- 不允许跨 session 写他人的 topic（隔离边界 = session_id）

---

## 5. 浮现机制设计（核心新增章节）

### 5.1 三件事的解耦

整个浮现系统由三个**独立可演化**的子系统构成。`update_session_topic` 工具的实现必须遵守这一边界：

| 子系统 | 职责 | 何时执行 |
|---|---|---|
| **A. Topic / Tag 更新** | 维护 Session Tag 集合（增删 + 淘汰） | `update_session_topic` 调用时 |
| **B. 基于 Topic 的召回** | 以当前 Tag 为背景信息，从 Memory/Notepad 中检索条目 | **任意触发点**（当前仅 update_session_topic，但接口开放） |
| **C. 召回信息的呈现** | 决定召回结果以何种形式、在何时进入 LLM 上下文 | 立即返回 / 背景注入 / 状态订阅触发 |

> **关键约束**：三者通过定义良好的数据交换协议解耦。CodeAgent 实现时，B 子系统必须抽象为独立的 `RecallService`，**不得**把召回逻辑直接耦合到 `update_session_topic` 工具实现内部。

### 5.2 子系统 A：Tag 集合维护与淘汰（机械层）

#### 5.2.1 数据结构

```rust
struct TagSet {
    capacity: usize,           // 默认 8
    tags: Vec<TagEntry>,
}

struct TagEntry {
    name: String,
    weight: f32,               // reinforcement counter
    last_touched: DateTime,    // 用于时间衰减
    tier: TagTier,
}

enum TagTier {
    Pinned,      // 永不淘汰，仅由 LLM 召回路径升降级（v0.2 暂不启用，全部为 Transient）
    Active,      // 受保护层，仅在 transient 全空时才考虑淘汰
    Transient,   // 默认层，参与机械淘汰
}
```

#### 5.2.2 淘汰算法

1. 新 Tag 进入：
   - 若已存在同名 Tag → 权重 += 1.0，`last_touched` 更新为当前时间
   - 若不存在 → 以 `weight=1.0, tier=Transient` 加入
2. 容量超限时：
   - 计算每个 transient Tag 的得分：`score = weight * decay(now - last_touched)`
   - 淘汰得分最低者
   - 衰减函数：`decay(dt) = exp(-dt / TAU)`，`TAU` 暂定 30 分钟（可配置）
3. tier 升降级：v0.2 暂不实现，预留接口；未来由 LLM 召回路径调用

#### 5.2.3 关键性质

- **纯机械、纯同步、无 LLM 调用**
- `update_session_topic` 必然执行此步骤
- 这是工具的"保底语义"——即便后续召回路径全部跳过，Tag 状态也已被正确更新并持久化到 `tag_set.json`

### 5.3 子系统 B：召回机制

#### 5.3.1 召回判定（阀门）

Tag 更新完成后，进入召回判定阶段。判定结果三态之一：

| 判定结果 | 含义 |
|---|---|
| **NotTriggered** | 阀门未突破，工具直接 success 返回（仅含 Tag 更新结果） |
| **Mechanical** | 走文本/向量检索路径 |
| **LLM** | 走旁路 LLM 路径 |

**阀门维度**（纯机械计算，cheap signal）：

- **距离阀门**：`current_turn - last_recall_turn >= DISTANCE_THRESHOLD`（默认 5 轮）
- **剧烈度阀门**：`(added.len() + removed.len()) / tag_set.len() >= CHANGE_THRESHOLD`（默认 0.5）

判定逻辑（CodeAgent 实现参考）：
```
if change_ratio >= CHANGE_THRESHOLD:
    return LLM       # 剧烈变化 → 走深度路径
elif turns_since_last_recall >= DISTANCE_THRESHOLD:
    return Mechanical # 距离够 → 走轻量路径
else:
    return NotTriggered
```

> 阀门策略必须可配置（通过 `RecallPolicy` 结构注入），不得硬编码常量。

#### 5.3.2 核心约束：二选一 + 同步等待

> **机械召回与 LLM 召回是互斥的二选一关系**。一旦判定触发召回，`update_session_topic` 工具调用会**同步等待召回完成**才返回 `success`，召回结果作为工具返回值的一部分交付。

含义：
- 调用方（Agent）拿到 `success` 时可以确信：要么没召回（`recall` 字段为 None），要么召回已完成（`recall` 字段含完整条目）
- **不存在"工具已返回但召回还在后台跑"的中间状态**
- 简化 Agent 侧状态机：无需处理异步召回的事件回调

> **CodeAgent 实现注意**：即便 LLM 召回耗时数秒，也必须在工具调用内同步等待。若需要异步语义，应另开新的召回入口（见 §5.3.4），而不是改变本工具的契约。

#### 5.3.3 两条召回路径

| | 机械召回 | LLM 召回 |
|---|---|---|
| **实现** | 对 Memory/Notepad 索引做基于 Tag 的文本/向量检索 | 旁路 LLM 基于 Tag + 全局状态做语义召回 |
| **耗时预期** | <100ms | 1-5s |
| **能力** | 文本级匹配 | 语义级匹配 + 可声明状态订阅 |
| **副作用** | 无 | 可能注册环境状态订阅（见 §5.4） |
| **失败处理** | 失败则返回空结果 + Failed status | 失败/超时则返回空结果 + Failed status |

> **关键能力差**：**对环境的感知与订阅，只能通过 LLM 召回路径产生**。机械路径只看历史文本，不具备"对未来某种状态保持关注"的意图能力。

#### 5.3.4 召回入口的开放性

当前 `update_session_topic` 是召回的**唯一已实现入口**，但 RecallService 在接口设计上不耦合这一假设：

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

> **CodeAgent 实现要求**：`update_session_topic` 是 `RecallService` 的调用方，不是实现者。两者必须在不同的模块/crate 中。

### 5.4 子系统 C：召回信息的呈现

召回结果的"如何/何时进入 LLM 上下文"独立于"如何召回"。共三条呈现通道：

#### 5.4.1 通道一：作为工具返回值即时呈现

- **触发**：召回在 `update_session_topic` 调用内完成（机械或 LLM 均可）
- **呈现**：召回条目作为 `UpdateSessionTopicResult.recall` 返回，由 Agent 在下一轮 inference 时自然进入上下文
- **特点**：与工具调用同生命周期，由 Agent 自主决定如何使用

#### 5.4.2 通道二：背景信息注入（半订阅）

- **触发**：LLM 召回路径在产出条目的同时，声明了状态订阅
- **呈现**：在下一次 user message 入场前，作为 background info 拼接在用户输入之前
- **类比**：当前在用户输入前插入时间戳的做法
- **特点**：召回结果"持续存活"，跨多个 turn 影响上下文

#### 5.4.3 通道三：订阅状态变化触发注入

- **触发**：被订阅的环境状态发生变化（例：地理位置、外部事件）
- **呈现**：在状态变化时刻，主动在下一次 user message 前插入变化信息
- **典型场景**：session topic 涉及"旅行规划"或"订票"，地理位置变化即触发插入
- **特点**：把召回从"点事件"升级为"持续的背景过程"

#### 5.4.4 三通道共存

三条通道**不互斥**：一次 LLM 召回可以同时产出"立即返回的条目"（通道一）+ "注册的订阅"（通道二/三）。

> **核心意图**：只要 Session Topic 还"新鲜"，每一次新的 LLM input message 入场前，浮现系统都有机会基于 Topic 编织相关信息到上下文中。

### 5.5 订阅中心（Subscription Center）

通道二/三依赖一个**订阅中心**组件，负责：

- 存储所有活跃订阅（持久化到 `{session_dir}/.meta/subscriptions.json`）
- 监听外部状态变化（地理位置、时间窗口、外部事件等）
- 在 user message 入场前查询所有相关订阅并组装 background info

#### 5.5.1 订阅生命周期

- **创建**：仅由 LLM 召回路径产出
- **TTL**：默认与产生它的 Tag 绑定——Tag 被淘汰，对应订阅自动失效
- **去重**：同一类订阅（如"地理位置"）多次注册时合并为一条，更新最新 Tag 关联
- **退订**：Session 结束时全部清理；或 Tag tier 被降级至淘汰时清理

> **CodeAgent 实现注意**：订阅与 Tag 在数据结构上必须**显式关联**（订阅记录 `bound_tags: Vec<String>`），以支持级联清理。

### 5.6 整体执行流总览

```
┌─────────────────────────────────────────────────────────┐
│  Agent 调用 update_session_topic(topic, tags)           │
└─────────────────────────────────────────────────────────┘
                            │
                            ▼
┌─────────────────────────────────────────────────────────┐
│  [子系统 A] 写入 topic.md + Tag 更新与淘汰              │
│  （机械、同步、必然执行）                                │
└─────────────────────────────────────────────────────────┘
                            │
                            ▼
                  ┌──────────────────┐
                  │  阀门判定        │
                  └──────────────────┘
                            │
        ┌───────────────────┼───────────────────┐
        ▼                   ▼                   ▼
   [NotTriggered]       [Mechanical]         [LLM]
        │                   │                   │
        │                   │           ┌───────┴───────┐
        │                   │           ▼               ▼
        │                   │      [召回条目]    [状态订阅]
        │                   │           │               │
        │                   │           │               ▼
        │                   │           │      [订阅中心持久化]
        │                   │           │       subscriptions.json
        │                   │           │               │
        └───────────────────┴───────────┘               │
                            │                           │
                            ▼                           ▼
        ┌───────────────────────────────┐   ┌─────────────────────┐
        │  工具 success 返回            │   │  影响未来 turn 的   │
        │  含 tag_set_diff + recall     │   │  background info    │
        │  [通道一]                     │   │  [通道二/三]        │
        └───────────────────────────────┘   └─────────────────────┘
```

### 5.7 策略矩阵（CodeAgent 应将这些参数暴露为配置项）

| 维度 | 选项 | 当前默认 |
|---|---|---|
| Tag 容量 | 整数 | 8 |
| Tag 分层 | pinned / active / transient | 全 transient（v0.2 简化） |
| 衰减常数 TAU | 时长 | 30 分钟 |
| 距离阀门 | turn 间隔 | 5 |
| 剧烈度阀门 | (add+remove)/total 比例 | 0.5 |
| 召回路径选择 | mechanical / llm / auto | auto |
| 召回入口 | update_session_topic | 仅此一个（接口开放） |
| 呈现通道 | tool result / bg inject / sub trigger | 三通道并存 |
| LLM 召回超时 | 时长 | 10s |

---

## 6. 与浮现层的对接（本工具不实现，但必须留好对接面）

浮现层是另一个子系统，本工具只产出它需要的原料。需要保证：

1. **可枚举**：浮现器能列出所有有 topic 的 session（`/{sessions_root}/*/.meta/topic.md`）
2. **可粗筛**：topic.md 的 frontmatter（updated_at, tags）支持时间/标签维度的筛选
3. **可定位**：topic.md 与 session_id 是 1:1 映射，浮现器拿到 hit 后能直接定位到 session_dir 做深度调用
4. **当前 session Tag 可读**：浮现层（含 RecallService）能读取 `tag_set.json`，作为机械召回的查询输入
5. **订阅可读**：呈现层（含通道二/三的注入逻辑）能读取 `subscriptions.json`

浮现层"工具可见性也是被浮现的对象"（即一个无浮现内容的 session 应当看不到任何 memory 相关 prompt 段）由浮现层自己实现；本工具**始终可见**（它是 Notepad 工具，不参与浮现）。

---

## 7. 与已有结构的关系

- **与 [session-history.md](session-history.md)**：history 是真理源（事件级），topic 是题眼（语义级）。history 不可写、只追加；topic 可覆盖。两者都挂在 session_dir 下，互不依赖
- **与 AgentNotebook**（[src/frame/agent_tool/src/agent_notebook.rs](src/frame/agent_tool/src/agent_notebook.rs)）：AgentNotebook 是 session 内的工作笔记；topic 是 session 间被浮现的题眼。如果实现层愿意把 topic 作为 AgentNotebook 的一个特化条目也可以，但**对外接口必须独立**，避免和普通笔记混淆
- **与 auto memory**（`~/.claude/.../memory/MEMORY.md`）：auto memory 存的是跨 session 的 user/feedback/project/reference 类长期记忆。topic 不进 auto memory；topic 是"短期 session 题眼"，由浮现机制时效性消费
- **与 LLMContext**：本工具作为 standard tool 注册到 LLMContext。其 Tool Result 遵循 `success | error | pending` 协议，但**永不返回 pending**——召回总是同步等待完成。即便召回失败也返回 `success` + `recall_status=Failed`，因为 Tag 更新这一"保底语义"已经完成（参见 §5.3.2）

---

## 8. 验收要点（给 CodeAgent 实现者）

### 8.1 工具基础语义

- [ ] 工具名 `update_session_topic`，参数 `topic` (+ 可选 `tags`)
- [ ] 写入路径固定为 `{session_dir}/.meta/topic.md`，frontmatter + body 结构
- [ ] 覆盖写、幂等（topic 不变时不重写 topic.md，但 Tag 仍处理）、单行 ≤ 120 字符校验
- [ ] 不写入即没有 topic.md，浮现层应能容忍缺失
- [ ] 工具描述（给 LLM 看的 system prompt 片段）必须明确：
  - 只在主题首次明确或显著漂移时调用
  - 写给"未来的自己"而不是给用户看
  - 不是会话总结，不要堆细节
- [ ] 不引入新的 RPC / DB；只走文件系统
- [ ] 不暴露 read / list API（读 topic 走通用文件工具即可，session_dir 路径已知）

### 8.2 Tag 集合维护（子系统 A）

- [ ] `tag_set.json` 的读取、更新、持久化
- [ ] 实现 §5.2.2 的淘汰算法（权重 × 时间衰减）
- [ ] tier 字段保留但 v0.2 全部置为 Transient
- [ ] 容量、TAU 通过配置注入，不硬编码

### 8.3 召回触发（子系统 B 的调用方）

- [ ] 实现 §5.3.1 的阀门判定（距离 + 剧烈度）
- [ ] 调用 `RecallService::recall()`，**不内嵌召回逻辑**
- [ ] `RecallMode` 由阀门判定结果映射（剧烈→LLM，距离→Mechanical，否则 NotTriggered）
- [ ] 同步等待召回结果，遵守 §5.3.2 契约
- [ ] LLM 召回设置超时（默认 10s），超时归入 Failed 状态
- [ ] 召回失败不影响 Tag 更新；工具仍返回 `success`

### 8.4 召回结果交付（子系统 C 通道一）

- [ ] `UpdateSessionTopicResult` 含完整字段（tag_set_diff / recall / recall_status）
- [ ] 召回条目按 §5.4.1 通过工具返回值交付
- [ ] LLM 召回产出的订阅写入 `subscriptions.json`，与产生它的 Tag 关联

### 8.5 RecallService 抽象

- [ ] 定义 `RecallService` trait（§5.3.4）
- [ ] 实现 `MechanicalRecallService`（基于 Tag 的文本/向量检索）
- [ ] 实现 `LLMRecallService`（旁路 LLM 调用）
- [ ] 提供 `RecallService` 的 mock 实现用于单元测试
- [ ] **`update_session_topic` 工具不得直接依赖具体 RecallService 实现**，必须通过 trait 注入

### 8.6 订阅中心（可与本工具独立实现，但需对接）

- [ ] `subscriptions.json` 的读写
- [ ] 订阅与 Tag 的级联关系（Tag 淘汰 → 订阅清理）
- [ ] 订阅去重逻辑

### 8.7 测试要点

- [ ] Tag 淘汰算法的单元测试（含边界：容量满、同名累加、衰减）
- [ ] 阀门判定的单元测试（各种 turn 距离 / 变化比例组合）
- [ ] 工具调用的集成测试：mock RecallService 返回不同结果，验证 `UpdateSessionTopicResult` 正确性
- [ ] 召回失败/超时场景下工具仍 `success` 的测试
- [ ] 幂等性测试：相同 topic 重复调用

---

## 9. 非目标（明确不做）

- 不做 session summary（那是另一种产物，篇幅更长、面向人类阅读）
- 不做自动 topic 抽取（让模型自己判断什么时候写、写什么——这是 Notepad 的语义）
- 不做跨 session 的 topic 合并 / 去重（浮现层的职责）
- 不做 embedding / 向量索引（浮现层若需要，自己读 topic.md 建）
- 不实现浮现层的索引、检索、注入逻辑——那些是另一个工单
- 不实现 Tag 的 tier 升降级（v0.2 全部 Transient，预留接口）
- 不实现异步召回入口（本工具同步语义；异步入口未来另开）
- 不在本工具内实现订阅状态监听器（订阅中心是独立组件）

---

## 10. 未来扩展（不在 v0.2 范围）

记录已识别但暂不实现的扩展点，避免现在过度设计：

- **Tag tier 升降级**：由 LLM 召回路径决定，把高价值 Tag 提升到 Active/Pinned，避免被机械淘汰
- **异步召回入口**：例如 `OnUserMessageEnter` 钩子，允许在不阻塞主路径的前提下做后台召回
- **召回入口的策略路由**：不同入口注入不同的 `RecallPolicy`（例如关键路径上更保守、空闲时更激进）
- **跨 session 的浮现器**：本文档只覆盖 session 内的 Tag 维护与召回触发；跨 session 的 topic 浮现由独立工单