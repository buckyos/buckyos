# Agent Memory Module — Memory Graph 核心组件规格 v2.10

> 本文档定义 Agent Memory 的 v2.10 契约：Agent-scoped singleton `memory_root`、Memory Graph 数据模型、写入语义、机械召回、遗忘、存储恢复与和 Agent Notebook 的职责边界。
> v2.10 相对 v2.9 的核心变化：
> 1. 时间模型改为**双时间**：`occurred_at`（实际/推断发生时间）是 Occasion 专属字段；`noticed_at`（被注意到的时间）是所有 item 的通用属性，遗忘沿 `noticed_at` 演化。
> 2. 原 `Event` 概念更名为 **Occasion**（可换成 Activity，只是一次全局替换）。
> 3. 恢复 v2.8 的 `set` / `remove` 平铺语义作为 `free` item（hint）的写入通路；`free` 不强制绑定 object。
> 4. 新增 `set-status` / `supersede` 写操作，让 status 变更也经 occasion log，curator 不直接改派生 state。
> 5. Memory 与 Notebook 改为**互斥召回源**：已进入 Notebook 的事实正文不再进入 Memory。
> 6. Memory 中所有 item 均为**非确定性推断（hint）**；`salience` item 仅用于审计，永不进入召回。

---

## 0. Core Contract

- `agent-memory` 是当前 Agent 唯一 Memory 的核心入口；每个 Agent 有且只有一个 Memory。
- `memory_root` 是该 Agent 唯一 Memory 的本地物理目录；所有持久化状态都在该目录内。
- Memory 保存 **Memory Graph**：occasions、对象、对象别名、观察、推论 item、对象关系和派生索引。
- Memory 的基础存储语义仍是 `key -> item`；Graph 语义不是替代 key-item，而是约束如何生成 canonical key 和 index key。
- Memory 中所有 item 都是**非确定性推断（hint）**，不是事实真理，不是原文摘要，不是知识库条目。
- 每个 item 必须有 `kind`；无法归类但值得保留的 item 默认 `kind = "free"`，通过平铺 `set` 语义写入。
- **时间是双维度的**：
  - `occurred_at`（实际或推断的发生时间，可为估算）**只属于 Occasion**。其它实体需要发生时间时引用其 `source_occasion`。
  - `noticed_at`（被注意到 / 被记录 / 被再次召回的时间）是**所有 item 的通用属性**，是**遗忘和召回衰减**的唯一时间依据。
  - `noticed_at` **不参与 LWW / 顺序语义**；LWW 仍以 occasion log 的 replay 顺序（`seq`）为准。
- 在线写入分两类：
  - **图操作（五类推断动作）**，必须挂在一个 Occasion 下：
    1. 新增对象，包括建立对象别名；
    2. 新增观察；
    3. 增强对象权重；
    4. 建立对象关系；
    5. 变更 item / object / observation 状态（supersede / dispute / stale / delete）。
  - **平铺操作**：`set` / `remove`，直接写 / 删 `free` item（hint），不要求 object 锚点。
- Memory 的召回以当前上下文识别出的 objects / aliases / ordered tags 为入口，做一到两跳的 mechanical contextual expansion，并结合 observation / free item 的全文兜底。
- **遗忘 = 不再被召回，不等于删除**。被遗忘的 item 仍留在存储中（本地空间足够长期保存），只是默认不进入 `load` 结果。
- Memory 不负责保存“用户明确要求记录的长期事实正文”；这类内容属于 Agent Notebook。**已进入 Notebook 的事实不再写入 Memory**（两者是互斥召回源）。
- Memory 不实现完整知识图谱推理，不做开放式 graph traversal，不把世界知识长期沉淀进本地。
- `.meta/occasions.jsonl` 是 append-only 审计日志，也是在线状态真相源。
- canonical key-item 与 `.meta/occasions.jsonl` 共同承载语义真相；index key、Graph state 文件与 `memory.sqlite` 都是派生缓存，可删除、重建。
- 同一 Agent 的唯一 `memory_root` 同时只允许一个写者；只读端允许并发，但必须容忍瞬时不一致。
- 跨语言互操作的边界是**统一 agent CLI**：digest、规范化、replay 都由 CLI 实现，调用方不需要自行复现 JSON canonical form。
- CLI 不构造 objects、aliases、tags，不翻译，不做语言检测；这些由上层 Agent / Session 合并器 / curator 提供。
- v2.10 仍使用与 Notebook 一致的 tag 词表和规范化规则；推荐 `primary_language = "en"`。

---

## 1. 范围与非目标

### 1.1 组件做什么

Agent Memory 为 Agent 提供跨 session 的结构化推论记忆能力，包括：

- 记录 occasion，并让发生时间语义都经由 occasion 表达；
- 识别和维护对象、对象别名、对象 salience / weight；
- 从 occasion 中保存可复用观察；
- 把观察提升为带证据、权重、置信度的 memory item；
- 保存无法结构化但值得记住的平铺 `free` item（hint）；
- 建立对象之间的关系 claim；
- 维护面向机械召回的 path / SQLite 派生索引；
- 沿 `noticed_at` 实现遗忘与召回衰减；
- 在 crash、索引损坏、文件残留等场景下提供确定性恢复规则。

### 1.2 组件不做什么

- 不保存聊天历史。
- 不替代 Agent Notebook。
- 不保存用户明确要求记录的长期事实全文。
- 不保存长文档、项目状态流水账、任务清单或知识库文章。
- 不实现提醒/待办调度系统。
- 不自动裁判 claim 是否为真理；所有 item 都是可修正的 hint。
- 不实现完整 ontology 或全局知识图谱。
- 不实现开放式 N 跳图遍历。
- 不维护当前 session tags、滑窗或上下文状态。
- 不保证 `load` 是事务快照；浮现式读取允许 best-effort。
- 不为多条 hint 提供跨 hint 的写入事务性；每条 hint 独立提交。
- 不支持同一 Agent 下多个命名 Memory，也不支持多个 Agent 直接共享一个 `memory_root`。

---

## 2. 与 Agent Notebook 的职责边界

Agent Notebook 和 Agent Memory 必须分离，且是**互斥召回源**：

| 模块 | 保存什么 | 写入来源 | 读取方式 |
|---|---|---|---|
| Notebook | 长期事实、用户明确要求记录的信息、偏好、项目状态、系统强约束 | 用户显式要求、项目状态、curator 整理 | notebook registry + tag list 过滤 + 时间倒序 |
| Memory | 观察到的推论（hint）、对象 salience、对象关系、可机械召回的 claim | Agent / curator 从 occasion 中推断 | objects / aliases / relation / ordered tags 机械展开 + 全文兜底 |

判断规则：

1. 用户说“记一下 X”或明确要求以后遵守，写 Notebook。
2. Agent 只是从上下文推断“X 未来可能影响行为”，写 Memory。
3. 一条信息需要长正文、稳定描述和人工可读整理，写 Notebook。
4. 一条信息需要参与对象、关系、权重、召回展开，写 Memory。
5. **同一事实不应同时进入 Notebook 与 Memory**：
   - 一旦事实正文进入 Notebook，它就由 Notebook 负责召回，Memory 不再复制该事实；
   - Memory 只在“需要参与图召回 / 关系推理”且该结构不属于 Notebook 表达时才写；
   - 如果 Memory 的某条推断依赖 Notebook 中的事实，它通过 `source_ref.notebook_id` 作为 evidence 指针引用，而不是复制正文。
6. 因为两者互斥，v2.10 不需要 Notebook→Memory 的同步/失效协议；不存在“Memory 复制了 Notebook 事实后变 stale”的问题。

示例：

| 输入 | 写哪里 |
|---|---|
| “以后回答我请用中文，简洁一点” | 写 Notebook `user/preferences`；Memory 不复制该偏好正文 |
| 用户多次提到 Bob 参与同一项目 | 写 Memory：对象 `Bob`、增强权重、`Bob -> works_on -> project` |
| “把这个项目决策记录下来” | 写项目 Notebook；Memory 至多写一条指向该 notebook 的关系/effect claim |
| 一段网页摘要 | 写 Notebook / KB；Memory 只在影响未来行为时写 claim，evidence 指向来源 |

---

## 3. 核心概念

| 概念 | 定义 |
|---|---|
| `memory_root` | 当前 Agent 唯一 Memory 的本地根目录，包含 occasion log、graph state、派生索引与锁。 |
| Memory Graph | 由 occasions、objects、observations、items、relations 和 indexes 组成的轻量推论图。 |
| Occasion | 发生过的一次上下文事实或写入动作；唯一带 `occurred_at` 的实体，是图操作的提交单元。 |
| Object | 被 Memory 关注的实体，如 user、person、project、file、service、concept、agent。 |
| Alias | 指向 object 的别名、昵称、路径、DID、用户名或其它可识别名称。 |
| Observation | 从 occasion 中抽出的观察证据；可被多个 item / relation 复用。 |
| Memory Item | 带 weight、confidence、evidence 的 claim / inference（hint）。 |
| Relation | 一类 Memory Item，表达 object 之间的谓词关系。 |
| Free item | 平铺 `key -> content` 的逃生口 hint，经 `set` 写入，不强制 object 锚点。 |
| Weight | 值不值得召回、召回强度、salience，不等于可信度。 |
| Confidence | claim 有多可信，不等于重要性。推荐由信号出现次数推导（见 §8.5）。 |
| Evidence | 支持 claim 的 observation ids / source occasion / external ref。 |
| `occurred_at` | 实际或推断的发生时间，Occasion 专属。 |
| `noticed_at` | 被注意到 / 被记录 / 被再次召回的时间，所有 item 通用，驱动遗忘。 |
| Index path | 为机械召回建立的派生句柄，不是完整语义来源。 |
| Ordered tags | `load` 查询词列表，顺序表示优先级，与 Notebook 使用同一规范化规则。 |

### 3.1 key-item 仍是基础结构

v2.10 引入 Memory Graph 语义，但不要求把 Memory 实现成独立 graph database。实现层仍应把 Memory 理解为 `key -> item`：

```text
/occasion/occ_001                  -> occasion item
/object/obj_user                   -> object item
/observation/obs_001               -> observation item
/item/item_001                     -> memory item, kind = relation/free/...
/index/by_entity/obj_user/item_001 -> derived pointer
```

Rules：

1. canonical key 承载 item 语义；index key 只承载召回句柄。
2. Graph operation 是从语义动作到 canonical key / index key 的转换规则。
3. `kind` 是 item 的基础字段；不提供 kind 时默认 `free`。
4. `free` 是平铺逃生口（旧 `set` 语义），**不要求 object 锚点**；但仍必须有 content/claim、weight、confidence 和 write_reason（即 `--reason`）。
5. key 命名应服务召回和可检查性，不承担完整 ontology 表达。

通用来源引用：

```ts
interface SourceRef {
  type:
    | "session_message"
    | "tool_result"
    | "notebook_item"
    | "file"
    | "url"
    | "manual"
    | "system";
  session_id?: string;
  message_id?: string;
  tool_call_id?: string;
  notebook_id?: string;
  item_id?: string;
  uri?: string;
  digest?: string;
}
```

### 3.2 双时间模型

时间分两种，归属不同：

```ts
interface OccasionTime {
  occurred_at: string;            // UTC ISO-8601, 实际或推断的发生时间, 可为估算; Occasion 专属
  noticed_at: string;            // UTC ISO-8601, 被注意到 / 被记录的时间
}

// 所有可召回 item（object / observation / memory item / alias）通用:
interface NoticedTime {
  noticed_at: string;            // 被注意到 / 被记录 / 被再次召回刷新的时间; 驱动遗忘
}
```

Occasion 实体：

```ts
interface MemoryOccasion {
  occasion_id: string;
  seq: number;                    // replay order, monotonically increasing
  occurred_at: string;            // 实际/推断发生时间, 可估算; Occasion 专属
  noticed_at: string;             // 该 occasion 进入 Memory 的时间
  occasion_type:
    | "session.turn"
    | "user.statement"
    | "tool.result"
    | "notebook.item"
    | "curator.action"
    | "memory.write"
    | "external.signal";
  actor_session_id?: string;
  source_ref?: SourceRef;
  summary: string;
  tags?: string[];                // normalized, optional retrieval hint
  operations: GraphStateOperation[];
}
```

Rules：

1. `seq` 由写入端分配，按提交顺序单调递增。LWW、覆盖、权重增强、关系/状态更新都以 `seq` replay 顺序为准。
2. `occurred_at` 是 Occasion 专属字段；object / observation / item 不保存自己的 `occurred_at`，需要发生时间时引用 `source_occasion.occurred_at`。
3. `noticed_at` 是所有 item 的通用属性：
   - 创建时取所属 occasion 的 `noticed_at`；
   - 被 `reinforce` / `set-status` / curator 动作再次触及时更新（这些都经 occasion，可 replay）；
   - 被 `load` 成功召回时**可选地** best-effort 刷新（属于派生缓存语义，不要求持久、不影响 replay）。
4. `noticed_at` **只用于遗忘与召回衰减**，不参与 LWW 和顺序判定。
5. `occurred_at` 可与 `noticed_at` 不同：例如用户今天提到“上个月签的合同”，`occurred_at` 是上个月（推断），`noticed_at` 是今天。
6. occasion summary 只放轻量摘要，不放完整聊天记录或长正文。

### 3.3 Object

Object 是 Memory Graph 的召回锚点。

```ts
type MemoryObjectKind =
  | "user"
  | "person"
  | "agent"
  | "project"
  | "repo"
  | "file"
  | "service"
  | "concept"
  | "organization"
  | "place"
  | "custom";

type MemoryObjectStatus = "active" | "merged" | "deprecated" | "deleted";

interface MemoryObject {
  object_id: string;              // stable id, e.g. obj_...
  kind: MemoryObjectKind;
  canonical_name: string;
  aliases: ObjectAlias[];
  weight: number;                 // 0.0-1.0, salience / recall strength
  confidence: number;             // identity confidence
  evidence: string[];             // observation ids
  source_occasion: string;        // first occasion that introduced this object
  last_occasion: string;          // last occasion that changed this object
  noticed_at: string;             // 通用遗忘时间, 召回/强化时刷新
  status: MemoryObjectStatus;
  merged_into?: string;
}

interface ObjectAlias {
  alias: string;
  alias_type:
    | "name"
    | "nickname"
    | "username"
    | "email"
    | "did"
    | "path"
    | "url"
    | "repo"
    | "custom";
  confidence: number;
  evidence: string[];
  source_occasion: string;
  noticed_at: string;
  status: "active" | "deprecated" | "merged" | "deleted";
}
```

Rules：

1. `object_id` 是 Memory 内部稳定 id，不应直接使用用户可变名称。
2. 同一别名命中多个 object 时，不自动合并；返回 ambiguous，交由上层或 curator 处理。
3. 新增 alias 必须有 evidence。
4. 合并 object 时，旧 object 标记 `merged` 并写 `merged_into`；其 alias 同步置 `merged`，索引应指向新 object 或同时保留跳转。
5. `weight` 和 `confidence` 必须分开：重要但证据弱的对象可以高 weight、低 confidence。
6. `ObjectAlias.status` 与 `MemoryObject.status` 对齐，支持 `merged` / `deleted`。

### 3.4 Observation

Observation 是从 occasion 抽出的证据单位。它比 occasion 更小，但仍不是长期事实正文。

```ts
type ObservationKind =
  | "mention"
  | "explicit_statement"
  | "behavior_signal"
  | "tool_evidence"
  | "notebook_evidence"
  | "curator_note";

interface MemoryObservation {
  observation_id: string;         // obs_...
  kind: ObservationKind;
  source_occasion: string;
  entities: string[];             // object ids
  content: string;                // concise observation
  source_excerpt?: string;        // optional short original excerpt
  source_ref?: SourceRef;
  confidence: number;
  noticed_at: string;
  status: "active" | "superseded" | "disputed" | "deleted";
}
```

Rules：

1. Observation 必须引用 `source_occasion`。
2. Observation 至少绑定一个 object，除非它的用途是引入新 object。
3. `content` 应是可复用观察，不是原始 transcript。
4. `source_excerpt` 应短，只用于审计；长内容属于 Notebook / history / external source。
5. 后续 item / relation 的 `evidence` 应优先引用 observation，而不是直接引用 occasion。
6. `content` 进入召回全文兜底索引（见 §8.2）；非英文原文的检索质量交由 agent CLI / ItemSearch 处理，不强依赖 SQLite FTS5。

### 3.5 Memory Item

Memory item 是带 evidence 的 claim / inference（hint）。

```ts
type MemoryItemKind =
  | "object"
  | "attribute"
  | "relation"
  | "event_effect"
  | "salience"
  | "observation_inference"
  | "free";

type MemoryItemStatus = "active" | "superseded" | "disputed" | "stale" | "deleted";

interface MemoryItem {
  item_id: string;                // item_...
  kind: MemoryItemKind;
  entities: string[];             // object ids; free item 可为空
  claim: MemoryClaim;
  weight: number;                 // recall strength, 0.0-1.0
  confidence: number;             // belief strength, 0.0-1.0
  evidence: string[];             // observation ids
  source_occasion: string;
  noticed_at: string;             // 通用遗忘时间
  write_reason: string;
  status: MemoryItemStatus;
  replaces?: string[];
}
```

Recommended claim shapes：

```ts
type MemoryClaim =
  | {
      type: "object";
      object_id: string;
      statement: string;
    }
  | {
      type: "attribute";
      subject: string;
      attribute: string;
      value: string;
    }
  | {
      type: "relation";
      subject: string;
      predicate: string;
      object: string;
    }
  | {
      type: "event_effect";
      occasion_id: string;
      affected_objects: string[];
      effect: string;
    }
  | {
      type: "salience";
      object_id: string;
      reason: string;
      delta: number;
    }
  | {
      type: "free";
      key?: string;               // 平铺 set 的 key
      statement: string;          // 平铺 set 的 content
    };
```

Item kind 的写入通路：

| kind | 写入通路 |
|---|---|
| `relation` | §4.4 `upsert_relation` |
| `salience` | §4.3 `reinforce_object_weight` 自动派生（仅审计，见下） |
| `free` | §4.6 `set`（平铺逃生口） |
| `object` / `attribute` / `event_effect` / `observation_inference` | 由 curator / self-improve 通过内部 service API 以显式 item payload 写入；不在五类在线推断动作的快捷子命令里 |

Rules：

1. 每个 item 必须能回答 §5.3 的最小问题清单。
2. **所有 item 都是非确定性推断（hint）**，不要使用 `fact` / `truth` 命名。
3. `free` 是逃生口，不是垃圾桶；它可以没有 entities，但仍必须有 content/claim、weight、confidence、write_reason。
4. `weight` 表示是否值得想起；`confidence` 表示是否可信。
5. 低 confidence 但高 weight 的 item 可以存在，但召回输出必须暴露二者。
6. `salience` item **仅用于审计，永不进入普通召回**（见 §4.3 / §9.x）。
7. status 为 `superseded` / `disputed` / `stale` / `deleted` 的 item 默认不参与普通召回。
8. 被遗忘（`noticed_at` 衰减到阈值以下）的 item 默认不召回，但**不删除**。

### 3.6 Relation

Relation 是 `kind = "relation"` 的 Memory Item，同时需要生成关系索引。

```json
{
  "item_id": "item_001",
  "kind": "relation",
  "entities": ["obj_alice", "obj_bob"],
  "claim": {
    "type": "relation",
    "subject": "obj_alice",
    "predicate": "works_with",
    "object": "obj_bob"
  },
  "weight": 0.73,
  "confidence": 0.81,
  "evidence": ["obs_001"],
  "source_occasion": "occ_001",
  "noticed_at": "2026-06-03T10:00:00Z",
  "write_reason": "May affect future coordination suggestions.",
  "status": "active"
}
```

Predicate 规则：

1. 推荐 lowercase English snake_case，例如 `works_with`、`depends_on`、`prefers`、`conflicts_with`。
2. 不要求全局 ontology，但必须足够稳定以支持召回过滤。
3. 同一关系方向有意义时必须保留方向；无方向关系可在索引中生成双向 handle。
4. 关系变化不覆盖旧 item；新 item 可通过 `replaces` 或 `set-status` 标记旧 item。

---

## 4. 写入语义

Memory 的在线写入分为：**图操作（挂在 Occasion 下）** 与 **平铺操作（`set` / `remove`）**。每条 hint 独立提交，Memory 不提供跨 hint 的写入事务性，也不提供批量 `commit` CLI。

```ts
type MemoryWriteIntent =
  | AddOccasionOp
  | GraphStateOperation
  | FlatSetOp
  | FlatRemoveOp;

type GraphStateOperation =
  | UpsertObjectOp
  | AddObservationOp
  | ReinforceObjectWeightOp
  | UpsertRelationOp
  | SetStatusOp;
```

### 4.1 新增 occasion

新增 occasion 是图操作的外层提交单元。

```ts
interface AddOccasionOp {
  op: "add_occasion";
  occasion_id: string;
  occasion_type: MemoryOccasion["occasion_type"];
  summary: string;
  occurred_at?: string;           // 缺省取写入时刻; 可显式传推断发生时间
  source_ref?: SourceRef;
  tags?: string[];
}
```

Requirements：

1. 每次图操作写入必须生成或引用一个 occasion。
2. occasion 可以包含多个后续 operations，但这些 operations 共享同一个 `occurred_at` 时间来源。
3. occasion summary 必须简短、可审计，不写长正文。
4. occasion tags 仅用于辅助召回，不替代 object / relation 索引。
5. 不传 `occurred_at` 时取系统当前时间；显式传入用于表达“事后才推断出更早发生”的场景。

### 4.2 新增对象与对象别名

```ts
interface UpsertObjectOp {
  op: "upsert_object";
  object_id?: string;
  kind: MemoryObjectKind;
  canonical_name: string;
  aliases?: Array<{
    alias: string;
    alias_type: ObjectAlias["alias_type"];
    confidence: number;
  }>;
  evidence: string[];
  weight?: number;
  confidence: number;
  merge_into?: string;
}
```

Requirements：

1. 没有匹配 object 时创建新 object。
2. 命中明确 object 时可增加 alias 或提升 identity confidence。
3. alias 必须建立 `/index/by_alias/...`。
4. object / alias 写入必须引用 evidence；CLI 调用方需要先写入 observation 并使用返回的 observation id。
5. 当 alias 冲突时，不自动覆盖，必须返回冲突给 curator 或上层 Agent。

### 4.3 增强对象权重

```ts
interface ReinforceObjectWeightOp {
  op: "reinforce_object_weight";
  object_id: string;
  delta: number;                  // -1.0..1.0, implementation clamps final weight
  reason: string;
  evidence: string[];
}
```

Requirements：

1. 权重增强表示“未来更值得召回”，不是“更可信”。
2. 增强必须有 evidence 和 reason。
3. 最终 `object.weight` 必须 clamp 到 `0.0..1.0`；本次增强同时刷新该 object 的 `noticed_at`。
4. 负 delta 可用于降低噪声对象 salience，但不等于删除。
5. 每次增强派生一条 `kind = "salience"` 的 MemoryItem 作为**审计记录**：
   - salience item 的 `weight` / `confidence` 取被增强 object 的最新值，仅用于审计；
   - **salience item 永不进入 `load` 普通召回**（见 §9.x）。

### 4.4 建立对象关系

```ts
interface UpsertRelationOp {
  op: "upsert_relation";
  subject: string;
  predicate: string;
  object: string;
  weight: number;
  confidence: number;
  evidence: string[];
  write_reason: string;
  replaces?: string[];
}
```

Requirements：

1. subject / object 应是已存在 object。若引用解析失败，实现应拒绝该单条 op 并报告。
2. relation 必须生成 MemoryItem。
3. relation 必须建立 by_entity、by_pair、by_relation、by_predicate 索引。
4. relation 可通过 `replaces` 或后续 `set-status` 标记旧 relation 为 superseded，但不得静默覆盖旧 item。
5. confidence 低于实现阈值时，可写入 `disputed` / `stale` 状态，或拒绝写入。

### 4.5 变更状态（supersede / dispute / stale / delete）

curator 与 self-improve 通过此操作改 status，使状态变更也经 occasion log，可 replay；**不允许直接改派生 state 文件或 SQLite**。

```ts
interface SetStatusOp {
  op: "set_status";
  target_kind: "item" | "object" | "observation" | "alias";
  target_id: string;             // alias 用 "<object_id>:<alias>"
  status: "active" | "superseded" | "disputed" | "stale" | "deleted" | "merged" | "deprecated";
  reason: string;
  replaced_by?: string;          // 可选: 指向新 item/object
}
```

Requirements：

1. 状态变更刷新目标的 `noticed_at`。
2. 置 `deleted` 是逻辑删除（tombstone 语义），不物理移除 canonical item；遗忘亦然。
3. 状态非 `active` 的目标默认不进入普通召回。
4. 状态机非法跃迁（如 `deleted -> active`）由实现拒绝或要求 curator 显式覆盖。

### 4.6 平铺写 / 删 free item（`set` / `remove`）

`free` item 是旧 v2.8 `set` 语义的延续，是平铺 hint 的逃生口，**不要求 object 锚点**，也不要求挂在显式 Occasion 下（实现内部仍会生成一个 `occasion_type = "memory.write"` 的 occasion 承载该写入，用于时间与审计）。

```ts
interface FlatSetOp {
  op: "set";
  key: string;
  content: string;                // 非空 UTF-8
  reason: string;                 // 必填, 即 write_reason
  entities?: string[];            // 可选 object 锚点
  tags?: string[];
  weight?: number;                // 缺省由实现给默认值
  confidence?: number;
}

interface FlatRemoveOp {
  op: "remove";
  key: string;
  reason?: string;
}
```

Requirements：

1. `set` 写入 `kind = "free"` 的 item，key 即 canonical key 的 `free` 命名空间。
2. 同 key 重复 `set` 覆盖（LWW 按 occasion seq）；`remove` 写 tombstone（逻辑删除）。
3. `content` 必须非空、UTF-8、无 BOM；超大 content 由实现给出 warning / 上限策略。
4. free item 进入全文兜底召回（见 §8.2），可经 tags、entities（若提供）和全文命中。

---

## 5. Write Barrier：什么值得写入 Memory

上层 Agent / curator 只有在信息可能影响未来行为时才应写 Memory。

### 5.1 可以写

- 用户、联系人、项目、仓库、服务、概念等对象在未来可能反复出现；
- 某对象的别名、路径、DID、用户名等会帮助未来识别；
- 一条观察可以支持未来判断；
- 某对象的重要性因为反复出现或任务相关性增强；
- 两个对象之间存在合作、依赖、冲突、偏好、归属、使用等关系；
- 一条 occasion 导致了对未来行为有影响的状态变化；
- 无法归类但明显会影响未来建议的推论，可写 `free` item（`set`）。

### 5.2 不应写

- 单纯“用户刚刚提到了 X”；
- 当前消息中已经完整可见、没有长期价值的信息；
- 通用世界知识；
- 长文本摘要、网页摘要、项目流水账；
- 低 salience 的一次性 transient detail；
- 没有 evidence 的猜测（`free` item 例外地允许弱锚点，但仍需 reason）；
- **已经进入 Notebook 的用户明确长期事实正文**（互斥召回源）。

### 5.3 合法 item 的最小问题清单

每条 Memory 写入前应能回答（`free` item 可放宽 1）：

1. What object(s) is this about?
2. What is being inferred?
3. Why may it affect future behavior?
4. What observation supports it?
5. How confident is it?
6. How strong should recall be?
7. Does it strengthen, update, conflict with, or replace an existing item?

---

## 6. CLI / Service 契约

实现可以同时提供内部 service API 和 shell CLI。CLI 面向 Agent Tool，保持 subcommand + positional + flags 风格。

### 6.1 全局形态

```bash
agent-memory [--root <memory_root>] [--quiet] <verb> [...]
```

- `--root`：覆盖当前 Agent 唯一 Memory 的物理目录，仅用于开发、测试、迁移或恢复。
- `--quiet`：抑制非错误日志，不改变退出码。

退出码：

| 退出码 | 含义 |
|---:|---|
| `0` | 成功，包括幂等成功。 |
| `1` | 参数、校验或普通运行错误。 |
| `2` | 写者锁冲突或等待超时。 |
| `3` | 真相源损坏，无法自动修复或读取。 |
| `64–78` | 可选使用 `<sysexits.h>` 语义。 |

### 6.2 `init`

```bash
agent-memory [--root <memory_root>] init
```

初始化目录，创建 `.meta/`、`.meta/meta.json`、`.meta/occasions.jsonl`、`.meta/lock`、`graph/` 与 `memory.sqlite`。

Rules：

1. 幂等；已初始化时退出 `0`。
2. `schema_version` 写入 `"2.10"`。
3. `primary_language` 推荐 `"en"`。
4. 已初始化目录的 `schema_version.major`、`encoding` 不兼容时，拒绝写入。

### 6.3 `occasion`

```bash
agent-memory occasion add --type <occasion_type> --summary <summary> \
  [--occurred-at <iso8601>] [--source <source_ref>] [--tags <tag1,tag2>]
```

输出：

```text
OCCASION occ_...
SEQ 42
```

Rules：

1. 只新增 occasion，不带 graph operation 也允许（审计型）。
2. `--summary` 必填，长度建议 `<= 500` 字符。
3. 返回的 occasion id 可用于后续 object / observe / reinforce / relate / set-status。
4. 不传 `--occurred-at` 取当前时间。

### 6.4 `object`

```bash
agent-memory object upsert \
  --occasion <occasion_id> \
  --kind <kind> \
  --name <canonical_name> \
  [--object <object_id>] \
  [--alias <alias>] \
  [--alias-type <type>] \
  --evidence <obs_id,...> \
  [--weight <0..1>] \
  --confidence <0..1>
```

输出：

```text
OBJECT obj_...
STATUS created|updated|ambiguous
```

### 6.5 `observe`

```bash
agent-memory observe add \
  --occasion <occasion_id> \
  --kind <kind> \
  --entities <obj_id,...> \
  --confidence <0..1> \
  <content>
```

长 content 可从 stdin 读取。

### 6.6 `reinforce`

```bash
agent-memory object reinforce \
  --occasion <occasion_id> \
  --object <object_id> \
  --delta <number> \
  --evidence <obs_id,...> \
  --reason <reason>
```

### 6.7 `relate` / `set-status`

```bash
agent-memory relate \
  --occasion <occasion_id> \
  --subject <object_id> \
  --predicate <predicate> \
  --object <object_id> \
  --weight <0..1> \
  --confidence <0..1> \
  --evidence <obs_id,...> \
  --reason <reason>

agent-memory set-status \
  --occasion <occasion_id> \
  --target-kind item|object|observation|alias \
  --target <id> \
  --status <status> \
  --reason <reason> \
  [--replaced-by <id>]
```

### 6.8 `set` / `remove`（平铺 free item）

```bash
# 形态 A：短 content 经 argv
agent-memory set <key> <content> --reason <reason> [--entities <obj_id,...>] [--tags <t1,t2>]

# 形态 B：长 content 经 stdin
agent-memory set <key> --reason <reason>

agent-memory remove <key> [--reason <reason>]
```

消歧规则只看 positional 数量（与 v2.8 一致）：

| positional 数 | 行为 |
|---:|---|
| `2` | `<content>` 来自 argv；忽略 stdin。 |
| `1` | content 从 stdin 读取；stdin 是 tty 或 0 字节则退出 `1`。 |
| `0` 或 `>=3` | 退出 `1`。 |

Rules：

1. `set` 写 `free` item，不要求 object 锚点。
2. `remove` 写 tombstone，删除不存在的 key 退出 `0`。
3. `--reason` 必填且非空。

### 6.9 `load`

```bash
agent-memory load [--tags <tag1,tag2,...>] [--objects <obj_id,...>] [--aliases <name,...>] [--max-records N] [--max-bytes N]
```

行为：

1. tags 按 §8.3 校验和规范化。
2. aliases 先解析为 object candidates；歧义 alias 返回候选，不静默选择。
3. objects 触发 by_entity / by_pair / by_relation 机械展开。
4. tags / 全文触发 item + observation + free 候选。
5. 合并、去重、过滤无效项与已遗忘项。
6. 按 §8.5 排序和截断。

默认：

- `--max-records` 默认 `50`，`--max-bytes` 默认 `65536`。
- `--tags`、`--objects`、`--aliases` 均可选；三者全空时按 §8.5 兜底排序返回最值得召回的若干条（不再需要 `*` 占位）。

### 6.10 `get` / `list` / `verify` / `compact`

```bash
agent-memory get item <item_id>
agent-memory get object <object_id>
agent-memory list objects [--kind <kind>]
agent-memory verify [--repair]
agent-memory compact
```

Rules：

1. `get` 输出完整 JSON，不输出派生索引噪声。
2. `list objects` 默认只列 active object。
3. `verify` 检查 occasion log、graph state、index 一致性。
4. `compact` 归档 occasions、生成 state snapshot、重建派生索引。

---

## 7. 输出格式

### 7.1 `load` 默认文本格式

每条召回记录使用长度前缀，并以 `END` 结束：

```text
ITEM <item_id>
KIND <kind>
ENTITIES <comma-separated-object-ids>
WEIGHT <0..1>
CONFIDENCE <0..1>
SOURCE_OCCASION <occasion_id>
NOTICED_AT <iso8601>
EVIDENCE <comma-separated-observation-ids>
MATCHED <comma-separated-tags-or-index-handles>
SIZE <n>
TRUNCATED <0|1>
---
<exactly n bytes of UTF-8 claim summary>
END
```

Rules：

1. `SIZE` 是 claim summary 的 UTF-8 字节数。
2. claim summary 必须可读，但不能丢失结构化字段；完整 JSON 可通过 `get item` 读取。
3. `MATCHED` 可以包含 tag 或 index handle，例如 `entity:obj_a`、`pair:obj_a:obj_b`、`fts:observation`。
4. status 非 active、已遗忘、或 `kind = salience` 的 item 默认不输出，除非显式 debug / curator 模式。

### 7.2 JSON 输出

实现可提供 `--json`：

```json
{
  "items": [
    {
      "item_id": "item_001",
      "kind": "relation",
      "entities": ["obj_a", "obj_b"],
      "claim": { "type": "relation", "subject": "obj_a", "predicate": "works_with", "object": "obj_b" },
      "weight": 0.73,
      "confidence": 0.81,
      "evidence": ["obs_001"],
      "source_occasion": "occ_001",
      "noticed_at": "2026-06-03T10:00:00Z",
      "matched": ["pair:obj_a:obj_b"]
    }
  ],
  "ambiguous_aliases": []
}
```

---

## 8. 检索与索引

### 8.0 Graph 到 key 的转换

Graph 语义的实现重点是把高层推断动作稳定转换为 key-item 和 index key，而不是维护另一套 graph storage。

| Graph operation | canonical key | derived index keys |
|---|---|---|
| 新增 occasion | `/occasion/<occasion_id>` | `/index/by_occasion_type/<type>/<occasion_id>` |
| 新增对象 | `/object/<object_id>` | `/index/by_alias/<alias>/<object_id>`、`/index/by_kind/object/<object_id>` |
| 新增观察 | `/observation/<observation_id>` | `/index/by_entity/<object_id>/obs/<observation_id>`、全文 |
| 增强对象权重 | `/item/<item_id>` with `kind = "salience"` | 仅审计；不建召回索引 |
| 建立对象关系 | `/item/<item_id>` with `kind = "relation"` | `/index/by_pair/...`、`/index/by_relation/...`、`/index/by_predicate/...` |
| 平铺 set | `/item/<item_id>` with `kind = "free"`（key 命名空间） | `/index/by_entity/...`（若有 entities）、全文 |

Rules：

1. canonical key 对应的 item 是可审计、可读取、可备份的主记录。
2. derived index key 可以是文件、SQLite 行、内存缓存或统一 ItemSearch 索引项。
3. 删除 index key 不应造成语义丢失；删除 canonical key 或 occasion log 才会破坏真相源。
4. 同一个 item 可以有多个 derived index key，以服务 entity、pair、relation、predicate、tag、全文等不同召回入口。

### 8.1 Index path 是 retrieval handle

```text
/index/by_entity/<object_id>/item/<item_id>
/index/by_entity/<object_id>/obs/<observation_id>
/index/by_alias/<normalized_alias>/<object_id>
/index/by_pair/<object_a>/<object_b>/<item_id>
/index/by_kind/<kind>/<item_id>
/index/by_predicate/<predicate>/<item_id>
/index/by_relation/<subject>/<predicate>/<object>/<item_id>
/index/by_weight/<bucket>/<object_id>
```

Rules：

1. by_pair 对无方向查询应写规范化 pair key，也可额外写双向 key。
2. by_relation 必须保留方向。
3. by_alias 使用规范化别名，不保存原始大小写作为 key。
4. index path 可从 graph state 重建，不是真相源。

### 8.2 SQLite schema 与全文兜底

`memory.sqlite` 是派生缓存。实现必须能从 `.meta/occasions.jsonl` 和 state snapshot 重建它。

```sql
CREATE TABLE objects (
  object_id      TEXT PRIMARY KEY,
  kind           TEXT NOT NULL,
  canonical_name TEXT NOT NULL,
  weight         REAL NOT NULL,
  confidence     REAL NOT NULL,
  status         TEXT NOT NULL,
  source_occasion TEXT NOT NULL,
  last_occasion  TEXT NOT NULL,
  noticed_at     TEXT NOT NULL,
  merged_into    TEXT
);

CREATE TABLE aliases (
  alias_norm     TEXT NOT NULL,
  object_id      TEXT NOT NULL,
  alias_type     TEXT NOT NULL,
  confidence     REAL NOT NULL,
  status         TEXT NOT NULL,
  source_occasion TEXT NOT NULL,
  PRIMARY KEY(alias_norm, object_id)
);

CREATE TABLE observations (
  observation_id TEXT PRIMARY KEY,
  kind           TEXT NOT NULL,
  source_occasion TEXT NOT NULL,
  content        TEXT NOT NULL,
  confidence     REAL NOT NULL,
  noticed_at     TEXT NOT NULL,
  status         TEXT NOT NULL
);

CREATE TABLE items (
  item_id        TEXT PRIMARY KEY,
  kind           TEXT NOT NULL,
  claim_json     TEXT NOT NULL,
  weight         REAL NOT NULL,
  confidence     REAL NOT NULL,
  source_occasion TEXT NOT NULL,
  noticed_at     TEXT NOT NULL,
  status         TEXT NOT NULL
);

CREATE TABLE item_entities (
  item_id        TEXT NOT NULL,
  object_id      TEXT NOT NULL,
  PRIMARY KEY(item_id, object_id)
);

CREATE TABLE item_evidence (
  item_id        TEXT NOT NULL,
  observation_id TEXT NOT NULL,
  PRIMARY KEY(item_id, observation_id)
);

CREATE TABLE relations (
  item_id        TEXT PRIMARY KEY,
  subject        TEXT NOT NULL,
  predicate      TEXT NOT NULL,
  object         TEXT NOT NULL
);

CREATE VIRTUAL TABLE memory_fts USING fts5(
  ref_id UNINDEXED,            -- item_id 或 observation_id
  ref_type UNINDEXED,          -- "item" | "observation"
  object_text,
  predicate_text,
  claim_text,
  observation_text,            -- observation.content / free content 进入兜底
  tokenize = 'unicode61 remove_diacritics 2'
);
```

Rules：

1. SQLite 只是缓存；损坏时删除并重建。
2. FTS 索引 active、未遗忘 item 的可召回文本，并把 `observation.content` 与 `free` content 纳入 `observation_text` 作召回兜底——避免“observation 没被提升成 item 就彻底不可召回”。
3. `salience` item 不进 FTS、不进任何召回索引。
4. **FTS5 对非英文原文检索质量有限**；非英文 content 的语义召回交由 agent CLI / 统一 ItemSearch 处理，FTS 只作 ASCII / 英文兜底。Memory Graph 的 object / relation mechanical expansion 不依赖 FTS。
5. `claim_json` 必须能 round-trip 恢复 item claim。

### 8.3 tag 校验

Memory 和 Notebook 必须使用同一 tag 校验与规范化函数。

每个 tag 经 trim 后必须满足：

- UTF-8 字节长度 `2–32`；
- 只允许 ASCII 字母、数字、空格、连字符：`[A-Za-z0-9 -]`；
- 必须至少包含一个字母或数字；
- 不允许双引号、单引号、冒号、括号、星号、控制字符；
- 连续空格规范化为单个空格；
- 大小写不敏感，内部以 lowercase 比较和缓存。

允许 phrase tag（`phone case` / `calendar reminder` / `project state`）。拒绝更复杂的 FTS query 语法，避免 LLM 生成坏 query，也避免 MATCH 表达式注入。

### 8.4 机械召回流程

给定当前 session 抽取结果：

```text
objects = [obj_a, obj_b, obj_c]
tags = ["planning", "dependency"]
```

Memory `load` 应按以下流程召回：

1. 单点展开：`/index/by_entity/<obj>/item/*` 与 `/obs/*`。
2. 二元展开：`/index/by_pair/<obj_i>/<obj_j>/*`。
3. 强类型关系展开：`/index/by_relation/<obj_i>/*/<obj_j>/*` 双向。
4. kind / predicate 过滤：`relation`、`free`、当前 task 允许的 predicate whitelist；**排除 `salience`**。
5. tag / 全文候选：ordered tags phrase OR 查询，命中 `claim_text` 与 `observation_text`（含 free content）。
6. 合并、去重、过滤状态非 active 与已遗忘项。
7. 按 §8.5 排序输出。

这叫 mechanical contextual expansion，不是完整 graph traversal。

### 8.5 排序、遗忘与置信度来源

**置信度 / 权重的来源（推荐）：** 让 LLM 直接打 `confidence` 在跨 LLM-context 时漂移很大，不是好设计。推荐 `confidence` / `weight` **由 stage1 信号出现次数（mention / 重复观察次数 / 强化次数）推导**，而非 LLM 单点估值；LLM 只负责抽取信号，计数与归一化由实现完成。本节排序公式中的常量均为**实现可调策略**，先以能跑通为准。

**遗忘：** 召回衰减只依赖 `noticed_at`（每条 item 自带），不依赖 `occurred_at`，也不再依赖 `source_occasion` 的时间。`noticed_at` 越久未刷新，召回分越低；低于实现阈值即视为“已遗忘”，默认不召回，但**不删除**。被召回 / 被强化会刷新 `noticed_at`，于是越常想起越不易忘。

推荐分数（常量可调）：

```text
score =
  structural_match_boost        // object/pair/relation 命中 > 纯 tag/全文命中
  + tag_position_boost          // ordered tags 靠前命中加分
  + weight * Wd
  + confidence * Cd
  - recency_decay(noticed_at)   // 唯一的时间项, 仅此一处
```

tag 位置 boost（可调）：

| tag 位置 | 命中加分 |
|---:|---:|
| 0 | `+8` |
| 1 | `+4` |
| 2 | `+2` |
| >=3 | `+1` |

最终排序键：

```text
score DESC,
weight DESC,
confidence DESC,
item_id ASC
```

（时间只通过 `recency_decay(noticed_at)` 进入 `score`，排序键里不再单独出现 seq/时间，避免重复计权。）

tags 与 objects 全空时：

```text
recency_decay(noticed_at) ASC-as-penalty,
weight DESC,
confidence DESC,
item_id ASC
```

### 8.6 Alias 解析

1. 输入 alias 先做 trim、大小写归一、空白归并。
2. 命中一个 active object：加入 objects。
3. 命中多个 active objects：返回 ambiguous，不做静默选择。
4. 命中 merged object：跳转到 `merged_into`。
5. 未命中：不自动创建 object；创建对象是写入动作。

---

## 9. 真相源、存储布局与恢复

### 9.1 存储布局

```text
<memory_root>/
├── occasion/
├── object/
├── observation/
├── item/
├── index/
├── graph/
│   ├── objects.jsonl
│   ├── observations.jsonl
│   ├── items.jsonl
│   └── indexes/
├── memory.sqlite
└── .meta/
    ├── meta.json
    ├── occasions.jsonl
    ├── lock
    ├── state.jsonl
    └── archive/
        └── occasions_YYYYMMDD.jsonl
```

Rules：

1. `.meta/occasions.jsonl` 是唯一 append-only occasion truth，用于恢复顺序和发生时间语义。
2. `/occasion/`、`/object/`、`/observation/`、`/item/` 保存 canonical key-item 主记录；实现可根据 `kind` 决定是否把某类 item 铺成文件。
3. `/index/` 与 `graph/indexes/` 是派生 path index，可重建。
4. `graph/*.jsonl` 是 replay 后的 state snapshot，可重建。
5. `memory.sqlite` 是派生查询缓存，可重建。
6. `.meta/` 不参与普通 `load`。

### 9.1.1 扁平文件系统与 kind 策略

| kind / key class | 是否建议铺文件 | 说明 |
|---|---|---|
| `/occasion/<occasion_id>` | 必须或强烈建议 | occasion 是发生时间来源，需要稳定 replay。 |
| `/object/<object_id>` | 建议 | object 是召回锚点，人工检查价值高。 |
| `/observation/<observation_id>` | 建议 | evidence 可审计，便于 debug。 |
| `/item/<item_id>` kind = `relation` | 建议 | relation 是核心 claim，需要可读。 |
| `/item/<item_id>` kind = `free` | 建议 | free 更需要可审计，避免变成隐藏垃圾桶。 |
| `/item/<item_id>` kind = `salience` | 可选 | 纯审计，可只进 occasion log / SQLite。 |
| `/index/...` | 可选 | 派生召回句柄，可用空文件、pointer、SQLite 行或缓存表达。 |

Rules：

1. 不铺文件的 canonical item 必须能从 `.meta/occasions.jsonl` 或 state snapshot 完整重建。
2. 文件系统存在的 canonical item 不改变 occasion replay 语义；冲突时以 occasion log 顺序为准。
3. index 是否铺文件是实现选择，不得成为唯一真相源。
4. `kind` 决定 item 的 schema、索引展开方式和推荐落盘策略。

### 9.2 `.meta/meta.json`

```json
{
  "schema_version": "2.10",
  "primary_language": "en",
  "writer": { "lang": "rust", "impl": "agent-memory-rs", "version": "0.10.0" },
  "graph": {
    "occasion_log": ".meta/occasions.jsonl",
    "time_model": "dual:occurred_at+noticed_at",
    "state_snapshot": ".meta/state.jsonl"
  },
  "index": {
    "engine": "sqlite-fts5",
    "tokenizer": "unicode61 remove_diacritics 2",
    "mechanical_indexes": ["by_entity","by_alias","by_pair","by_kind","by_predicate","by_relation","by_weight"]
  },
  "compaction_strategy": "snapshot",
  "initialized_by_occasion": "occ_init"
}
```

兼容规则：

1. `schema_version.major` 不一致：拒绝写入。
2. `schema_version.minor` 不一致：可只读挂载；写入需实现明确支持。
3. `time_model` 不是 `"dual:occurred_at+noticed_at"`：v2.10 拒绝写入。
4. 不支持的 `compaction_strategy`：拒绝写入。

### 9.3 Occasion log envelope

`.meta/occasions.jsonl` 中每一行是完整 occasion envelope：

```json
{
  "schema_version": "2.10",
  "occasion_id": "occ_001",
  "seq": 1,
  "occurred_at": "2026-06-03T10:00:00Z",
  "noticed_at": "2026-06-03T10:00:00Z",
  "occasion_type": "session.turn",
  "actor_session_id": "s1",
  "source_ref": { "type": "session_message", "session_id": "s1", "message_id": "m9" },
  "summary": "User discussed Agent Memory design boundaries.",
  "tags": ["agent memory", "memory graph"],
  "operations": [
    {
      "op": "add_observation",
      "observation_id": "obs_001",
      "kind": "explicit_statement",
      "entities": ["obj_user", "obj_agent_memory"],
      "content": "User wants Memory separated from Notebook and modeled as graph-like inferred memory.",
      "confidence": 0.86
    }
  ],
  "digest": "blake3:abcd1234..."
}
```

Rules：

1. 单条 occasion 必须完整写入一行 JSON，不允许半行提交。
2. envelope 可以含空 operations（审计型 occasion，如 `memory.write` 承载平铺 set）。
3. `digest` 由 agent CLI 计算与校验，覆盖除 `digest` 外的内容。**digest 的规范化是 CLI 内部步骤**；跨语言互操作通过统一 CLI 完成，调用方无需自行复现 canonical JSON。
4. occasion log 写入使用 `O_APPEND` + `fsync`。

### 9.4 写入顺序

图操作的规范顺序：

1. 获取 `.meta/lock`。
2. 校验 occasion 与 operations；非法写入直接拒绝本次提交。
3. 分配 `seq`、`occasion_id` 和缺省 ids。
4. 在内存中 replay 当前 operations，得到新 graph state。
5. 追加 occasion envelope 到 `.meta/occasions.jsonl`，`O_APPEND` + `fsync`。
6. 更新 graph state snapshot / jsonl。
7. 更新 path indexes。
8. 更新 SQLite 派生缓存。
9. 释放锁。

失败语义：

- 步骤 5 成功后，写入视为已提交。
- 步骤 6–8 失败不影响真相源；后续 `verify --repair` / `compact` 可从 occasions 重建。
- 步骤 5 前失败，不得留下可见 graph state。
- `noticed_at` 的 load-time 刷新是派生缓存语义，不在此关键路径内，丢失不影响真相。

### 9.5 replay 规则

在线状态由以下内容 replay 得到：`.meta/state.jsonl`（若存在）→ `.meta/archive/*.jsonl`（按 compaction 元数据顺序）→ `.meta/occasions.jsonl`。

Rules：

1. replay 顺序由 `seq` 和归档顺序共同确定；冲突时报错，不静默修复。
2. object / alias / item 的最新状态按 occasion 顺序更新。
3. item status 按最后一次 `set_status` / `replaces` 生效。
4. `source_occasion` 引用不存在时，`verify` 必须报错。
5. evidence observation 引用不存在时，item 不得进入 active 召回。
6. `noticed_at` 的权威值取“创建 + 后续经 occasion 的刷新”；load-time best-effort 刷新不进入 replay。

### 9.6 `verify`

`agent-memory verify [--repair]` 检查：occasion log 行完整性、`seq` 单调性、occasion digest、operation schema、object / alias 冲突、evidence 引用完整性、`source_occasion` 引用完整性、graph state 可重建性、path index 可重建性、SQLite 一致性。

无 `--repair`：只报告。
有 `--repair`：可删除并重建 SQLite / `graph/*.jsonl` / `graph/indexes/`；不得静默改写 `.meta/occasions.jsonl`；不得静默合并 object / alias 冲突。

### 9.7 `compact`

推荐策略 `snapshot`：

1. 生成 `.meta/state.jsonl`，保存 replay 后每个 object / observation / item 的最新状态（含 `noticed_at`）；
2. 将旧 `.meta/occasions.jsonl` 移入 `.meta/archive/`；
3. 新建空 `.meta/occasions.jsonl` 接收增量；
4. 写 compaction manifest，记录归档顺序和最后 seq；
5. 重建 `memory.sqlite` 与 path indexes。

compact 不删除已遗忘 item（遗忘≠删除）；可选地为长期未召回的 item 标注但保留。

### 9.x salience 仅审计

`kind = "salience"` 的 item 是 `reinforce_object_weight` 的审计副产物：

1. 不进 FTS、不建召回 index、不进入 `load`。
2. 只能经 `get item` / debug / curator 模式读取。
3. 它记录“某次为什么调整了某 object 的 weight”，是审计轨迹，不是 hint。

---

## 10. 并发与可选 daemon

### 10.1 写者锁

- 写入端必须持有 `.meta/lock`（POSIX `flock` / Windows `LockFileEx`）。
- 默认锁等待 5 秒，超时退出 `2`。
- 只读端可不加锁，但遇到不一致时按 §9 判定。

### 10.2 可选 daemon

实现可提供 `agent-memory daemon` 作性能优化，由 daemon 持锁并转发子命令。约束：daemon 不改变 CLI 语义、不改变 `memory_root` 布局、崩溃后普通 CLI 必须能接管、daemon 是优化而非协议要求。

---

## 11. Prompt / Session 集成

Memory 的核心读取模式是 surfacing。上层 session 在每轮推理前识别 objects / aliases / ordered tags，并在合适的时候调用 `load`。

| 角色 | 做什么 | 不做什么 |
|---|---|---|
| LLM / prompt | 抽取候选对象、别名、tags、信号；决定是否写入推论。 | 不直接管理文件，不绕过 write barrier，不单点估 confidence（交计数）。 |
| Session 合并器 | 维护当前 session 的 objects / aliases / ordered tags。 | 不写 Memory，不决定长期真相。 |
| Memory 模块 | 解析 alias、机械召回、遗忘衰减、排序、截断、返回 item。 | 不构造 tags，不翻译，不维护 session。 |
| Curator / self-improve | 合并对象、降噪、`set-status` 清理冲突、提升/降级 item。 | 不把 Notebook 事实正文塞入 Memory，不直接改派生 state。 |

默认读取是 best-effort：不保证召回所有相关记忆、不保证强一致快照、不处理 token limit（CLI 只处理 byte limit）。token 预算由上层折算。

---

## 12. 非功能性要求

| 项目 | 要求 |
|---|---|
| 本地优先 | 只依赖本地文件系统、SQLite 3.34+ 与 FTS5；统一 ItemSearch 可作可选替代，尤其用于非英文语义召回。 |
| 写入可靠性 | occasion JSONL append 使用 `O_APPEND` + `fsync`；单写者锁防止半行交错。 |
| 双时间 | `occurred_at` 仅属 Occasion；`noticed_at` 是所有 item 通用属性且只驱动遗忘。 |
| 索引可重建 | `graph/indexes/` 与 `memory.sqlite` 任何时候都可从 occasions / state 重建。 |
| 跨语言互操作 | 边界是统一 agent CLI；digest / canonical form 不外泄给调用方，调用方只经 CLI 读写。 |
| tag 上限 | 每个 tag `2–32` UTF-8 字节。 |
| occasion summary 上限 | 建议 `<= 500` 字符；长正文应放 Notebook / history / external source。 |
| observation 上限 | 建议单条 `<= 2KB`；只放可复用观察。 |
| 可观测性 | write/load/verify/compact 应记录 occasion id、命中对象、命中 tags、错误摘要。 |
| 遗忘 | 沿 `noticed_at` 衰减，超阈值不召回但不删除；存储长期保留。 |

---

## 附录 A：最小示例

### A.1 初始化

```bash
export AGENT_MEMORY_ROOT=/path/to/agent_memory_root
agent-memory init
```

### A.2 记录一次讨论 occasion 和观察

```bash
agent-memory occasion add \
  --type session.turn \
  --summary "User discussed Agent Memory and Notebook separation." \
  --tags "agent memory,memory graph"
```

输出：

```text
OCCASION occ_001
SEQ 1
```

```bash
agent-memory observe add \
  --occasion occ_001 \
  --kind explicit_statement \
  --entities obj_user,obj_agent_memory \
  --confidence 0.86 \
  "User wants Agent Memory to store inferred graph-like claims, not notebook-style long facts."
```

### A.3 新增对象和别名

```bash
agent-memory object upsert \
  --occasion occ_001 \
  --kind concept \
  --name "Agent Memory" \
  --alias "memory graph" \
  --alias-type name \
  --evidence obs_001 \
  --weight 0.72 \
  --confidence 0.84
```

### A.4 增强对象权重

```bash
agent-memory object reinforce \
  --occasion occ_001 \
  --object obj_agent_memory \
  --delta 0.12 \
  --evidence obs_001 \
  --reason "Repeated design focus in current task."
```

### A.5 建立对象关系

```bash
agent-memory relate \
  --occasion occ_001 \
  --subject obj_user \
  --predicate prefers \
  --object obj_inspectable_memory_systems \
  --weight 0.82 \
  --confidence 0.76 \
  --evidence obs_001 \
  --reason "May affect future architecture recommendations."
```

### A.6 平铺写一条 free hint

```bash
agent-memory set "user/tooling-preference" \
  "User prefers inspectable, greppable local tools over opaque services." \
  --reason "Recurring stance across sessions." \
  --tags "tooling,preference"
```

### A.7 召回

```bash
agent-memory load \
  --tags "agent memory,architecture" \
  --objects obj_user,obj_agent_memory \
  --max-records 10 \
  --max-bytes 8192
```

示例输出：

```text
ITEM item_001
KIND relation
ENTITIES obj_user,obj_inspectable_memory_systems
WEIGHT 0.82
CONFIDENCE 0.76
SOURCE_OCCASION occ_001
NOTICED_AT 2026-06-03T10:00:00Z
EVIDENCE obs_001
MATCHED entity:obj_user,tag:agent memory
SIZE 82
TRUNCATED 0
---
User likely prefers inspectable memory systems over opaque embedding-only memory.
END
```

---

## 附录 B：ADR 摘要

### B.1 为什么 Memory 保留 key-item 但不再是 key/content 仓库

纯 `key -> content` 仓库会自然滑向“长期事实库”和“聊天摘要库”，与 Notebook 重叠。Memory v2.10 保留 `key -> item`（key 对召回、grep、备份、人工检查友好），但 item 必须结构化（`kind`、claim/content、entities、evidence、weight、confidence、source_occasion、noticed_at）。其中 `free` item 通过平铺 `set` 保留了旧 v2.8 的简单写入手感，作为不强制 object 锚点的逃生口。

### B.2 为什么时间是双维度的

如果每个实体都带自己的多套时间，系统会出现多套 freshness 和 LWW 语义。v2.10 把时间拆成两类、归属不同：`occurred_at`（实际/推断发生时间）只属于 Occasion，保证 replay/审计/LWW 的确定性来源单一；`noticed_at`（被注意到的时间）是所有 item 的通用属性，**只驱动遗忘与召回衰减，不参与 LWW**。这样既保留了 v2.9 “单一时间真相”的好处，又让“越常想起越不易忘、久不想起自然淡出”成为每条 item 的一等属性。

### B.3 为什么分开 weight 和 confidence；为什么 confidence 走计数

`weight` 表示是否值得召回，`confidence` 表示推论是否可信。让 LLM 跨 context 直接打 `confidence` 漂移很大，因此推荐由 stage1 信号出现次数推导 confidence/weight，LLM 只负责抽信号。

### B.4 为什么 path 只是 retrieval handle / Graph 只是 graph-to-key 转换

Memory Graph 不是完整知识图谱。path index 的价值是稳定召回：按 entity、pair、relation、predicate 做一到两跳展开。Graph 语义只用来约束写入动作和索引展开，不引入独立 graph DB；新增对象/观察/关系最终都落成 canonical key-item + derived index key。

### B.5 为什么保留 `free` item / `set`

schema 太硬会迫使 LLM 扭曲信息；完全自由又会污染 Memory。`free` 是受约束的逃生口：可以没有 object 锚点（延续 v2.8 `set` 手感），但仍要 content、reason、weight、confidence，并进入全文兜底召回。

### B.6 为什么仍保留 tags / 加全文兜底

objects / relations 解决结构相关，tags 解决语义相似，全文（含 observation/free content）解决“没被提升成 item 也别彻底丢”。三者叠加缓解“读时没抽出对象就召不回”的盲区；非英文语义召回交由统一 ItemSearch，不强压在 FTS5 上。

### B.7 为什么 Memory 与 Notebook 互斥而不是同步

双写需要失效/同步协议，复杂且易 stale。v2.10 直接规定：事实正文进 Notebook 后不再进 Memory，Memory 只在需要图召回时引用 Notebook 作 evidence 指针。没有复制就没有同步问题。

### B.8 为什么 salience 仅审计

salience 是“为什么调权重”的轨迹，不是 hint。混进召回会噪声化。它只为 curator / debug / 审计存在。

### B.9 为什么不提供 commit CLI

Memory 全是非确定推断 hint，不存在“两条 hint 必须同时成立”的业务约束。批量 `commit` 会引入占位 id、部分成功、回滚语义和额外审计复杂度，容易把 Memory 误导成事务型知识库。v2.10 的 CLI 只提供单条图操作和平铺 `set` / `remove`，上层需要写多条 hint 时应显式多次调用。

---

## 附录 C：实现检查清单

- [ ] `schema_version` 升级到 `2.10`。
- [ ] `Event` 全量更名为 `Occasion`（含 `occasion_id` / `occasions.jsonl` / `source_occasion` 等）。
- [ ] 双时间落地：`occurred_at` 仅 Occasion；`noticed_at` 所有 item 通用且只驱动遗忘。
- [ ] `.meta/occasions.jsonl` 成为唯一 append-only occasion truth。
- [ ] Memory 保留 `key -> item` 基础结构，而不是改成独立 graph DB。
- [ ] Graph operation 能稳定转换为 canonical key-item 和 derived index key。
- [ ] 每个 item 都有 `kind`，缺省为 `free`；`free` 经 `set` 写入且不强制 object 锚点。
- [ ] 支持 `set` / `remove` 平铺语义。
- [ ] 支持新增对象/别名、新增观察、增强权重、建立关系、`set-status`。
- [ ] `set-status` 让状态变更经 occasion log，curator 不直接改派生 state。
- [ ] relation 写入生成 by_entity / by_pair / by_relation / by_predicate 索引。
- [ ] `salience` 仅审计，永不进入召回与 FTS。
- [ ] observation/free content 纳入全文兜底；非英文交 ItemSearch。
- [ ] confidence/weight 由信号计数推导，不由 LLM 单点估值。
- [ ] 遗忘沿 `noticed_at` 衰减，超阈值不召回但不删除。
- [ ] Memory 与 Notebook 互斥；已进 Notebook 的事实不再写 Memory。
- [ ] tag 校验与 Notebook 使用同一规范化规则。
- [ ] 不提供 `commit` CLI；多条 hint 由上层显式多次调用单条写入接口。
- [ ] 跨语言互操作只经统一 CLI；digest/canonical form 不外泄。
- [ ] `load` 三类入口均可选，无需 `*` 占位。
- [ ] `verify --repair` 可重建 graph state、path indexes 和 SQLite。
- [ ] `compact` 不改变 occasion replay 语义，不删除已遗忘 item。
