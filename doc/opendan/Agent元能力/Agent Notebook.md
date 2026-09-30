# Agent Notebook 技术需求

> 面向：code agent / 工程实现
> 版本：v0.2 技术需求草案（与 Agent Memory v2.10 对齐）

---

## 0. 给 code agent 的执行摘要

实现一个 `Agent Notebook` 模块，用于保存 Agent 跨 session 的长期事实、偏好、项目状态和用户明确要求记录的信息。

这个模块不是 Memory，也不是聊天历史，也不是知识库。它的核心职责是：

1. 管理多本 Notebook；
2. 以 append-first 方式写入 Note Item；
3. 提供 registry，让 session 启动时只知道"有哪些本子"，而不是注入全文；
4. 提供 `read_notebook`，按 **tag list 过滤 + 时间倒序** 返回相关条目（tag 缺省即无过滤）；
5. 对已读且未变化的读取返回 `unchanged`，避免上下文重复污染；
6. `agent-notebook read` / `append` 在当前 session 内自动订阅相关 notebook/item title，并在跨 session 写入或更新后生成轻量 update hint；
7. 支持 System Notebook 的小规模强注入；
8. 支持条目的 stale / superseded / deleted 状态管理；
9. 给 self-improve / curator 提供整理入口，但不在本模块内实现 LLM 提取逻辑。

全文索引、语义检索、BM25、FTS、embedding 等都走和 Agent Memory 一致的系统，方便跨模块召回。Agent Notebook 模块只依赖一个 `ItemSearch` / `ItemRepository` 抽象，不自己建索引。

召回入口统一使用 **tag list**：

- tag 之间是并集过滤（任一命中即候选）；
- tag list 缺省、为空、或为 `["*"]` 都表示无 tag 过滤，返回该 notebook 全部 active items；
- tag 顺序不影响结果顺序，**tag 仅用于过滤、不参与排序**；
- 结果按 `updated_at DESC` 排序（详见 §5.3.4）；
- tag 词表、校验规则、规范化函数与 Memory v2 §8.3 完全一致。

---

## 1. 目标与非目标

### 1.1 目标

Agent Notebook 模块需要实现以下能力：

| 能力 | 说明 |
|---|---|
| Notebook registry | 列出 Agent 当前有哪些 notebooks、每本用途、条目数量、最近更新时间、版本号 |
| Note Item append | 向指定 notebook 追加一条长期事实或项目状态 |
| Note Item read by tags | 按 notebook + tag list 过滤（缺省=全量），按时间倒序返回；也支持 title / latest_n / item_ids 等精确读取 |
| Version / unchanged | 如果当前 session 已读过同一 (notebook, scope) 且 notebook 未变化，返回轻量 `unchanged` |
| Read cache | 记录 session 已读取过哪些 notebook、什么版本、什么读取范围（含 tag set 哈希） |
| Hint filtering | 已读取且未变化的 notebook 不再重复注入 topic hint |
| Cross-session update hint | 其他 session 更新相关 notebook 后，对曾读/写过或已自动订阅相关 notebook / title 的 session 提供轻量提示 |
| System Notebook | 极少数高置信、长期有效的事实可强注入 prompt |
| Status management | 支持 active、stale、superseded、deleted 等状态 |
| Item remarks | 支持基于 notebook item id 的备注 list / append / remove，可按备注类型过滤 |
| Curator API | 允许后台 self-improve 进行整理、淘汰、提升、降级 |
| Auditability | 每个条目保留来源、时间、actor、reason，便于追溯 |

### 1.2 非目标

以下内容不要在本模块里实现：

1. 不实现 Item 全文索引；
2. 不实现独立 BM25 / FTS5 / embedding index；
3. 不实现知识库检索；
4. 不实现提醒/待办调度系统；
5. 不实现 Memory 模块；
6. 不实现 LLM self-improve 逻辑本身；
7. 不在 session 启动时注入完整 notebook 内容；
8. 不做复杂自动事实裁判或全自动 last-write-wins 覆盖；
9. 不把 Workspace FS 或 Agent Root FS 当作 Notebook 的事实存储替代品；
10. 不接受自然语言 query 字符串作为召回输入；上层 LLM 应抽取 tag list 调用，缺省即无过滤。

### 1.3 与 Agent Memory 的边界（互斥召回源）

Agent Notebook 与 Agent Memory 是**互斥召回源**，对齐 Agent Memory v2.10 §2：

1. 用户明确要求记录的、需要长正文与稳定描述的**事实正文**只进 Notebook；
2. 同一事实**不应**把正文同时复制进 Memory；事实正文进入 Notebook 后即由 Notebook 负责召回；
3. Memory 只保存用于图召回 / 推理的结构化推断（hint）；当它的推断依赖某条 Notebook 事实时，通过 `source_ref.notebook_id` 作为 evidence 指针引用该 NotebookItem，**而不是复制正文**；
4. 因为互斥，两者之间不需要事实正文的同步 / 失效协议，也不会出现"Memory 复制了 Notebook 事实后变 stale"的问题；
5. 两者共享同一 tag 词表与规范化规则（§3.6）：目的不是让同一条事实在两处都召回，而是让上层用**同一组 ordered tags** 能分别命中各模块内的相关记录（Notebook 的事实正文 / Memory 的结构化线索）。

---

## 2. 核心概念

### 2.1 Notebook

Notebook 是一组长期事实条目的容器。它可以表示用户资料、用户偏好、项目状态、Agent 协作经验或系统级强约束。

推荐 notebook id：

```text
user/profile
user/preferences
projects/<project-name>
relationships
agent/operating-notes
system
```

Notebook 本身只提供组织、版本、统计和注入元信息。事实正文存储在 Note Item 中。

每个 Agent scope 必须有一个默认 notebook。当前 CLI 默认 notebook id 为 `user/actions`。上层 CLI / session 集成在未显式指定 notebook id 时，应先把请求解析到该 Agent 的默认 notebook，再调用底层 notebook 读写接口；底层结果中始终返回解析后的 concrete `notebook_id`。

### 2.2 Note Item

Note Item 是 Notebook 里的最小事实单元。它应实现为统一 Item 系统中的一种 item type，例如：

```text
item_type = "agent.notebook.note"
```

Note Item 的正文、title、tags 以及最小索引属性应能被统一 Item 索引系统处理。Notebook 模块不关心索引内部实现，只负责调用统一写入接口和查询接口。

### 2.3 System Notebook

System Notebook 是特殊 notebook：其中少量 active items 会被 prompt compiler 直接注入到高优先级上下文中。

硬性约束：

1. active system items 默认最多 10 条；
2. 只允许高置信、长期有效、来源明确的事实进入；
3. stale / superseded / deleted item 不得注入；
4. System Notebook 不是普通偏好记录区，不允许无限增长；
5. 推荐由 curator / self-improve 或人工确认流程提升，而不是在线 Agent 随意写入。

### 2.4 Session Read Cache

Session Read Cache 是当前 session 级状态，用来记录：

1. 读过哪本 notebook；
2. 读的是哪个 notebook version；
3. 读取范围是什么（mode + tag set / title / latest_n / item_ids）；
4. 返回过哪些 item；
5. 读取时间。

它的目的不是给用户看，而是避免 Agent 反复读取同样内容，使工具结果污染上下文。

### 2.5 Notebook Hint

Notebook Hint 是 prompt compiler 或 topic retrieval 注入给 Agent 的轻量提示。Hint 不是事实，只是"可能相关，建议读取"。

正确形式：

```text
A notebook may contain relevant information about this topic. Read user/preferences if needed.
```

错误形式：

```text
User prefers X.
```

未读取 Notebook 之前，hint 不应被当成事实。

### 2.6 Notebook Subscription

Notebook Subscription 是 session 级弱订阅，用来把本 session 已经关注过的 notebook 或 note title 转换成后续的 `agent-note` / background hint。

它不是 `subscribe_event` 的 KEvent 订阅，也不要求向 LLM 暴露一个新的工具调用。实现上可以落在 session metadata、`session_reads`、或独立订阅表中，但语义必须满足：

1. 当 `agent-notebook read` 成功返回 `ok`，且调用方提供了 `session_id`，当前 session 自动订阅该 `notebook_id`；
2. 如果读取输入包含 `title`，或读取结果返回了具体 item title，当前 session 同时自动订阅这些 title；
3. 当 `agent-notebook append` 成功，且调用方提供了 `session_id`，当前 session 自动订阅写入的 `notebook_id + title`；
4. 自动订阅只表示"后续变化可能相关"，不得把 title 或 notebook 内容直接当事实注入；
5. 自动订阅命中只对其它 session 的后续变更触发 hint；同一个 `actor_session_id` 的写入默认不提示自己。

### 2.7 Tag List

所有召回入口的查询输入都是 tag list。tag 词表、校验、规范化与 Agent Memory v2.10 §8.3 完全一致；过滤语义参考 Memory v2.10 §8.4，但**不沿用 Memory 的 BM25/boost 排序**：

- tag 之间是并集过滤，任一命中即可成为候选；
- tag list 缺省、为空、或显式传 `["*"]` 一律视为"无 tag 过滤"，候选集为该 notebook 全部 active items；
- 单 tag 是 phrase（允许空格短语），不接受 FTS5 控制字符；
- tag 顺序对返回结果无影响，**不做 tag-position boost**；
- 排序统一按 `updated_at DESC`，见 §5.3.4；
- tag 校验规则见 §3.6。

CLI 不构造 tags，不翻译，不做语言检测；tags 由上层 Agent / Session 合并器提供。

---

## 3. 数据模型

以下为逻辑模型。实际实现可映射到现有数据库、Item Store、本地文件或对象存储，但字段语义必须保持。

### 3.1 Notebook

```ts
type NotebookKind = "normal" | "project" | "system" | "agent";
type NotebookStatus = "active" | "archived" | "deleted";

interface Notebook {
  id: string;                    // e.g. "user/preferences"

  kind: NotebookKind;
  title: string;
  description: string;
  status: NotebookStatus;

  entry_count: number;           // all non-deleted entries, or implementation-defined but documented
  active_entry_count: number;
  latest_item_id?: string;
  latest_title?: string;
  latest_updated_at?: string;    // ISO-8601

  revision: number;              // monotonically increasing per notebook
  version: string;               // opaque version string derived from revision/hash/ulid

  created_at: string;
  updated_at: string;
  archived_at?: string | null;
}
```

Requirements：

1. `id` 在当前 Agent Notebook DB 内唯一；owner / agent scope 由 Agent RootFS 路径提供，不在 SQL 表内重复存储；
2. `version` 每次 notebook 内容或状态变化都必须变化；
3. `version` 不能只依赖秒级时间戳；
4. `revision` 必须单调递增；
5. `entry_count`、`active_entry_count`、`latest_*` 必须在写入事务内同步更新。

### 3.2 NotebookItem

Note Item 需要作为统一 Item 系统的 Item 存在，同时 Notebook 模块可以保留一张关系/元数据表。

```ts
type NotebookItemStatus = "active" | "stale" | "superseded" | "deleted";
type Confidence = "low" | "medium" | "high";
type WriteReason =
  | "user_explicit"
  | "strong_rule"
  | "project_state"
  | "curator_extracted"
  | "curator_cleanup"
  | "manual_admin";

interface NotebookItem {
  item_id: string;               // Item Store id
  notebook_id: string;

  title: string;
  content: string;
  source_excerpt?: string;
  source_ref?: {
    type: "session_message" | "tool_result" | "file" | "manual" | "system";
    session_id?: string;
    message_id?: string;
    file_id?: string;
    uri?: string;
  };

  source_session_id?: string;
  write_reason: WriteReason;

  confidence: Confidence;
  status: NotebookItemStatus;

  valid_from?: string | null;     // date or ISO-8601
  valid_until?: string | null;    // date or ISO-8601

  tags: string[];                 // 用于召回的固有 tags；须符合 §3.6

  created_at: string;
  updated_at: string;

  item_revision: number;
  content_hash: string;
}
```

Requirements：

1. `title`、`content`、`created_at`、`status`、`write_reason` 必填；
2. `content` 应保留事实表述，避免只存关键词；
3. `source_excerpt` 推荐保留用户原话或关键片段，但 registry 不暴露；
4. 在线 Agent 写入时，`write_reason` 只允许 `user_explicit`、`strong_rule`、`project_state`；
5. `curator_extracted` 和 `curator_cleanup` 只允许后台 curator / self-improve 使用；
6. `deleted` 默认表示软删除；如有隐私合规要求，可额外实现 purge，但 purge 不在 MVP 必需范围；
7. `tags` 是召回信号，必须经过 §3.6 规范化；写入时若有非法 tag，直接拒绝。

### 3.2.1 NotebookItemRemark

NotebookItemRemark 是挂在 Note Item 上的备注。它不替代 item content，也不参与 tag 召回；语义上类似人在纸质笔记上用不同颜色的笔添加边注。

```ts
type NotebookItemRemarkStatus = "active" | "deleted";

interface NotebookItemRemark {
  remark_id: string;
  item_id: string;
  notebook_id: string;

  remark_type: string;           // e.g. "red", "blue", "warning", "verified"
  content: string;
  status: NotebookItemRemarkStatus;

  created_at: string;
  updated_at: string;
}
```

Requirements：

1. 备注必须通过 `item_id` 归属到一个已存在的 NotebookItem；
2. `remark_type` 和 `content` 必填，`remark_type` 经 trim / 空白归并后存储；
3. `list` 默认只返回 active remarks，可通过 `remark_type` 过滤；
4. `remove` 默认软删除 remark，不物理删除记录；
5. append / remove remark 必须更新 item 的 `updated_at` / `item_revision`，并更新 notebook version；
6. append / remove remark 必须写 NotebookEvent，event summary 不放完整 item content。

### 3.3 Supersede Edge

Notebook 不应通过覆盖旧内容来表达事实变化，而应通过状态和 edge 表达。

```ts
type NotebookItemEdgeType = "supersedes" | "related" | "conflicts_with";

interface NotebookItemEdge {
  from_item_id: string;
  to_item_id: string;
  edge_type: NotebookItemEdgeType;
  reason?: string;
  created_at: string;
  created_by?: string;
}
```

Requirements：

1. 新事实覆盖旧事实时，创建 `supersedes` edge；
2. 被覆盖的旧 item 可标记为 `superseded`；
3. 冲突不必阻止写入，可返回给 Agent 或 curator 处理；
4. 默认 read 不返回 `superseded`、`stale`、`deleted`，除非显式请求。

### 3.4 SessionNotebookRead

```ts
type ReadMode = "all_active" | "latest" | "title" | "tags" | "items";

interface SessionNotebookRead {
  session_id: string;
  notebook_id: string;

  read_scope_hash: string;       // 由 mode + 规范化输入计算得到
  read_scope: {
    mode: ReadMode;
    tags?: string[];             // 仅当 mode = "tags" 时填入；已规范化、已排序去重的 tag set
    title?: string;
    latest_n?: number;
    item_ids?: string[];
    include_status?: NotebookItemStatus[];
  };
  // 注：tags 缺省 / [] / ["*"] 都不会落到 mode = "tags"，而是落到 mode = "all_active"。

  notebook_version: string;
  notebook_revision: number;
  returned_item_ids: string[];
  max_bytes?: number;

  read_at: string;
}
```

Requirements：

1. read cache key 必须包含 `session_id + notebook_id + read_scope_hash`；
2. 不同 mode 的读取范围不能简单视为相同；
3. 不同 tag set 的读取范围必须计算为不同 `read_scope_hash`；
4. `read_scope_hash` 计算前必须对 tags 做规范化（trim、归并空格、lowercase、按规范化串去重并排序），保证语义相同的输入得到相同 hash；tag list 的元素顺序不参与 hash 计算；
5. 如果之前读取的是 `all_active`，则可覆盖当前 notebook 同版本下的任意 tag scope（因为 all_active 已包含全集）；
6. 如果只是同 notebook 但 mode/tags 不同，不得盲目返回 `unchanged`；
7. read cache 可存在 session runtime store 中，也可持久化；MVP 只要求在 session 生命周期内可靠。
8. 对 CLI / session 集成而言，read cache 同时可作为 notebook_id 级自动订阅的实现依据；若需要 title 级订阅，应额外记录 read 输入或返回 item 的 title。

### 3.5 NotebookEvent

```ts
type NotebookEventType =
  | "notebook.created"
  | "notebook.updated"
  | "item.appended"
  | "item.status_changed"
  | "item.superseded"
  | "item.promoted_to_system"
  | "item.demoted_from_system"
  | "item.remark_appended"
  | "item.remark_removed";

interface NotebookEvent {
  event_id: string;
  event_type: NotebookEventType;

  notebook_id: string;
  item_id?: string;
  actor_session_id?: string;

  title?: string;
  summary?: string;              // short, safe, no full content
  tags?: string[];               // 受影响 item 的固有 tags，便于 hint topic match

  notebook_version: string;
  notebook_revision: number;

  created_at: string;
}
```

Requirements：

1. 每次 append/status/supersede/promote/demote 必须生成事件；
2. event summary 只放轻量摘要，不放完整 content；
3. event tags 是受影响 item 的 tags 副本，仅用于 cross-session hint 的 topic match；
4. cross-session update hint 只基于 event 元信息生成；
5. 事件要能根据 session watermark 去重。

### 3.6 Tag 规则（与 Agent Memory v2 §8.3 一致）

Notebook 模块对 tag 的校验和规范化必须与 Agent Memory v2.10 严格一致，以保证上层 session 用同一组 ordered tags 可以在 Memory 和 Notebook 中获得可预测的召回行为。

每个 tag 经 trim 后必须满足：

- UTF-8 字节长度 `2–32`；
- 只允许 ASCII 字母、数字、空格、连字符：`[A-Za-z0-9 -]`；
- 必须至少包含一个字母或数字；
- 不允许双引号、单引号、冒号、括号、星号、控制字符；
- 连续空格规范化为单个空格；
- 大小写不敏感（内部以 lowercase 比较和缓存）。

允许 phrase tag：

```text
phone case
calendar reminder
project state
```

规范化步骤（必须实现并暴露为同一函数）：

1. trim 首尾空白；
2. 内部连续空白合并为单空格；
3. lowercase；
4. 校验字符集、长度、至少一个字母数字字符；
5. 拒绝任何不符合规则的 tag，整次调用返回 `invalid_input`。

写入路径（`append_note` 的 `tags`、`tags` 字段持久化）和读取路径（`read_notebook` 的 `tags`、hint 的 `topic_tags`）必须使用同一个规范化实现。

---

## 4. 存储与统一 Item 集成

### 4.1 Source of Truth

推荐以统一 Item Store 作为 Note Item 正文的 source of truth：

```text
Item.type = "agent.notebook.note"
Item.title = NotebookItem.title
Item.body = NotebookItem.content
Item.tags = NotebookItem.tags           // 已规范化
Item metadata = notebook_id + status + timestamps
```

Notebook 模块负责 Notebook 容器、状态、版本、事件和 session read cache。

如果当前代码库还没有 Item Store 写入封装，也可以临时落地 JSONL / DB 表，但必须预留 Item 写入适配层；不要因此在 Notebook 模块内实现全文索引。

### 4.2 Item 索引约束

必须遵守：

1. Notebook 模块不创建 FTS 表；
2. Notebook 模块不创建倒排索引；
3. Notebook 模块不实现 BM25；
4. Notebook 模块不实现 embedding 生成与向量索引；
5. Note Item 写入后，通过统一 Item Store 的标准 hook / repository 接口进入统一索引；
6. 查询时调用统一 `ItemSearch` 能力，并加上 `item_type = "agent.notebook.note"`、`notebook_id` 等过滤条件；owner / agent scope 由 Agent RootFS 路径确定；
7. 召回输入是 ordered tag list，由 Notebook 模块原样下发给 `ItemSearchPort.searchItemsByTags`，由统一 Item 系统决定如何转换为 FTS query 或 phrase OR 查询。

建议定义端口，而不是具体实现：

```ts
interface ItemRepositoryPort {
  createItem(input: {
    item_type: "agent.notebook.note";
    title: string;
    body: string;
    tags: string[];                       // 已规范化
    metadata: Record<string, unknown>;
  }): Promise<{ item_id: string; content_hash: string }>;

  updateItemMetadata(item_id: string, metadataPatch: Record<string, unknown>): Promise<void>;

  getItems(item_ids: string[]): Promise<Array<{
    item_id: string;
    title: string;
    body: string;
    tags: string[];
    metadata: Record<string, unknown>;
  }>>;
}

interface ItemSearchPort {
  // 主召回接口：ordered tag list
  searchItemsByTags(input: {
    item_type: "agent.notebook.note";
    tags: string[];                       // ordered, 已规范化；空数组等价于全量候选
    filters?: Record<string, unknown>;    // 例如 { notebook_id, status }
    limit: number;
  }): Promise<Array<{
    item_id: string;
    score?: number;                       // 越小越相关；与 bm25_score 语义一致
    matched_tags?: string[];              // 命中的 tag（已规范化）
  }>>;
}
```

该接口只表达依赖，不要求 code agent 实现索引细节。Notebook 模块不暴露也不接受自然语言 query 字符串。

---

## 5. 工具 / 服务接口

接口可以实现为内部 TypeScript/Python service，也可以包装成 CLI。无论具体形态如何，语义必须一致。Notebook DB 由 Agent RootFS 路径隔离；接口输入不再包含 `owner_user_id`、`owner_agent_id`、`actor_kind`、`actor_id` 或 item 级 `metadata`。

### 5.1 list_notebooks

```ts
list_notebooks(input: {
  include_archived?: boolean;
}) -> {
  notebooks: Array<{
    id: string;
    kind: NotebookKind;
    title: string;
    description: string;
    entry_count: number;
    active_entry_count: number;
    latest_title?: string;
    latest_updated_at?: string;
    version: string;
  }>;
}
```

Requirements：

1. 只返回 registry 元信息；
2. 不返回 item content；
3. 默认不返回 archived/deleted notebooks；
4. 输出应适合 prompt compiler 生成 session 启动 registry block。

### 5.2 create_or_update_notebook

```ts
create_or_update_notebook(input: {
  notebook_id: string;
  kind?: NotebookKind;
  title?: string;
  description?: string;
}) -> {
  notebook: Notebook;
  created: boolean;
}
```

Requirements：

1. 如果 notebook 不存在则创建；
2. 如果存在则只更新 title/description/kind 等容器元信息；
3. 更新 description/kind 应增加 notebook revision；
4. 不允许通过该接口覆盖 item 内容。

### 5.3 read_notebook

```ts
read_notebook(input: {
  session_id?: string;

  notebook_id: string;

  // 召回输入（互斥优先级：item_ids > title > tags > latest_n > 默认 all_active）
  tags?: string[];                        // tag list；缺省 / [] / ["*"] 表示无 tag 过滤
  title?: string;
  latest_n?: number;
  item_ids?: string[];

  since_version?: string;
  include_status?: NotebookItemStatus[];  // default: ["active"]
  include_superseded?: boolean;           // compatibility sugar

  max_items?: number;                     // default: 10
  max_bytes?: number;                     // default: 12000
  allow_unchanged?: boolean;              // default: true
}) -> NotebookReadResult
```

Result：

```ts
type NotebookReadResult =
  | {
      status: "ok";
      notebook_id: string;
      version: string;
      revision: number;
      read_scope_hash: string;
      matched_tags?: string[];            // 当 mode = "tags" 时给出本次请求中至少命中一次的 tag 集合
      entries: Array<{
        item_id: string;
        title: string;
        content: string;
        source_excerpt?: string;
        created_at: string;
        updated_at: string;
        source_session_id?: string;
        confidence: Confidence;
        status: NotebookItemStatus;
        valid_from?: string | null;
        valid_until?: string | null;
        tags: string[];
        matched_tags?: string[];          // 该 entry 命中的 tag 子集
      }>;
      truncated: boolean;
    }
  | {
      status: "unchanged";
      notebook_id: string;
      version: string;
      revision: number;
      read_scope_hash: string;
      instruction: string;
    };
```

#### 5.3.1 召回模式选择

| 条件 | 模式 | 说明 |
|---|---|---|
| 提供 `item_ids` | `items` | 精确按 id 取，绕过 tag 过滤；仍按 status 过滤。 |
| 提供 `title` | `title` | 先做规范化 title 精确匹配；可补充近似匹配。 |
| 提供 `tags`（非空且不为 `["*"]`） | `tags` | 调用 `ItemSearchPort.searchItemsByTags`，过滤 notebook/status，再按 §5.3.4 时间排序。 |
| 提供 `latest_n` | `latest` | 按 `updated_at` 倒序读取最近 N 条 active items。 |
| `tags` 缺省 / `[]` / `["*"]`，且未提供其他召回参数 | `all_active` | 不调用 ItemSearch，按 §5.3.4 时间排序取该 notebook 全部 active items；受 `max_items` / `max_bytes` 截断。 |

设计要点：

- **tags 缺省即无 tag 过滤**，等价于"读全部 active items"。这是默认行为；上层 LLM 没抽出 tag 时直接传空即可，不需要构造 `["*"]`。
- 互斥规则：若同时提供 `item_ids` 和其他召回参数，以 `item_ids` 为准并在 result 中提示 `truncated_inputs`，避免 LLM 误判。
- `latest_n` 与 `tags` 同时给出时，优先使用 `tags` 模式，再用 `latest_n` 作为 `max_items` 上限（取 `min(max_items, latest_n)`）。

#### 5.3.2 Tag 过滤行为

1. `tags` 在进入 ItemSearch 之前必须先做 §3.6 规范化；任一 tag 非法则整次调用返回 `invalid_tag`，不做静默丢弃；
2. 多个 tag 之间是并集过滤：任一命中即可成为候选；
3. tag 顺序对结果不产生任何影响（不做 boost）；
4. tags 必须以同一规范化形式既用于 ItemSearch，也用于命中判定（不要用未规范化的原始字符串子串匹配）；
5. 规范化后元素全部去重后为空（例如 `["*"]`、`[" "]` 在规范化时被剔除），按 `all_active` 处理。

#### 5.3.3 Unchanged 语义

1. 默认只返回 `active` items；
2. 若请求范围在当前 session、同 notebook version 下已被覆盖过，返回 `status: "unchanged"`；
3. 若 `since_version` 等于当前 notebook version，且 read scope 被缓存覆盖，也返回 `unchanged`；
4. 若 tags/title/latest_n 范围不同，除非之前读取过 `all_active`，否则不得只因为 notebook version 相同就返回 `unchanged`；
5. `unchanged` 返回必须非常短，不包含 entries；
6. 每次 `ok` 读取后必须更新 Session Read Cache，写入 `read_scope_hash`、`returned_item_ids`、`notebook_version`；
7. 返回内容必须受 `max_items` 和 `max_bytes` 限制。
8. 每次 `ok` 读取且存在 `session_id` 时，必须自动订阅该 `notebook_id`；若输入包含 `title` 或返回 entries 中包含 title，也应把这些 title 加入当前 session 的 Notebook Subscription。
9. `unchanged` 不需要新增 title 订阅；它继续沿用当前 session 之前的 read cache / subscription 状态。

#### 5.3.4 排序

**所有模式（`tags` / `latest` / `all_active` / `title` / `items`）的最终排序键统一为：**

```text
updated_at DESC,
created_at DESC,
item_id ASC
```

设计说明：

- Notebook 存的是长期事实，时间新鲜度比相关性更重要：用户最近更新过的事实更可能反映当前真相。
- tag 仅做候选集过滤；不像 Memory v2 那样做 tag-position boost。多个 tag 命中不会比单个 tag 命中排得更前。
- ItemSearch 返回的 `score`（若有）忽略，不参与排序。
- `confidence` 不参与召回排序；它只影响 System Notebook 提升、curator 决策等。
- `items` 模式按输入 `item_ids` 的顺序保留也可接受，由实现选择；但实现一旦定下，必须文档化并稳定。

时间字段约定：

- `updated_at` 是 NotebookItem 最后一次 content 或 status 变更时间；append 时 = `created_at`。
- 时间相同时（同毫秒并发写入），用 `item_id` ASC 作为稳定 tiebreaker。

#### 5.3.5 Unchanged instruction 推荐文案

```text
This notebook range has not changed since it was last read in this session. Use the earlier notebook content already present in the conversation history. Do not read it again unless the notebook changes or the user explicitly asks.
```

### 5.4 append_note

```ts
append_note(input: {
  session_id?: string;

  notebook_id: string;
  title: string;
  content: string;

  source_excerpt?: string;
  source_ref?: NotebookItem["source_ref"];
  source_session_id?: string;

  write_reason: WriteReason;

  valid_from?: string | null;
  valid_until?: string | null;
  confidence?: Confidence;               // default: medium
  tags?: string[];                        // 写入前按 §3.6 规范化；空数组允许

  detect_conflicts?: boolean;             // default: true
}) -> {
  status: "ok";
  item_id: string;
  notebook_id: string;
  version: string;
  revision: number;
  possible_conflicts: Array<{
    item_id: string;
    title: string;
    reason: "same_title" | "near_title" | "tag_overlap" | "active_overlap";
    matched_tags?: string[];
    status: NotebookItemStatus;
    updated_at: string;
  }>;
}
```

Requirements：

1. append-first：不得覆盖旧 item content；
2. 如果目标 notebook 不存在，必须在同一事务中自动创建 notebook，再写入 note；
3. 写入必须在事务中完成：创建 Item、创建 NotebookItem row、更新 notebook revision/version、创建 event；
4. `title` 和 `content` 不能为空；
5. `tags` 必须先经 §3.6 规范化再持久化；非法 tag 直接返回 `invalid_input`；
6. 返回 possible conflicts，但默认不阻止写入；
7. possible conflicts 候选来源：
   - 同 notebook、规范化 title 完全相同；
   - 同 notebook、title 近似（实现自定）；
   - 通过 `ItemSearchPort.searchItemsByTags` 用新 item 的 tags 召回的 top-N active item（`reason = "tag_overlap"`）；
   - 当前 active item 的 valid time 与新 item 重叠；
8. 不要为冲突检测实现独立全文索引；
9. 写入后必须发布 NotebookEvent，event.tags 为新 item 的规范化 tags；
10. 写入后应使相关 session 的旧 read cache 失效，或通过 version mismatch 自然失效；
11. 写入成功且存在 `session_id` 时，必须自动订阅当前 session 的 `notebook_id + title`，后续其它 session 对该 notebook 或相同/相近 title 的更新可触发 `agent-note` hint。

### 5.5 mark_note_status

```ts
mark_note_status(input: {
  session_id?: string;

  item_id: string;
  status: NotebookItemStatus;
  reason: string;
  superseded_by?: string;
  expected_item_revision?: number;
}) -> {
  status: "ok";
  item_id: string;
  notebook_id: string;
  notebook_version: string;
  notebook_revision: number;
}
```

Requirements：

1. 状态变化必须更新 item row；
2. 状态变化必须更新 notebook version；
3. 如果提供 `expected_item_revision`，不匹配则返回 conflict；
4. `superseded_by` 存在时应创建 `supersedes` edge；
5. 默认 read 不返回 stale/superseded/deleted；
6. `deleted` 默认软删除。

### 5.5.1 item remark management

```ts
list_item_remarks(input: {
  item_id: string;
  remark_type?: string;
}) -> NotebookItemRemark[]

append_item_remark(input: {
  session_id?: string;
  item_id: string;
  remark_type: string;
  content: string;
}) -> {
  status: "ok";
  remark_id: string;
  item_id: string;
  notebook_id: string;
  notebook_version: string;
  notebook_revision: number;
}

remove_item_remark(input: {
  session_id?: string;
  item_id: string;
  remark_id: string;
}) -> {
  status: "ok";
  remark_id: string;
  item_id: string;
  notebook_id: string;
  notebook_version: string;
  notebook_revision: number;
}
```

Requirements：

1. 三个接口都以 `item_id` 为入口；
2. list 支持按 `remark_type` 精确过滤；
3. append 不覆盖已有 remark；
4. remove 是软删除，默认 list 不返回已删除 remark；
5. append / remove 会改变 notebook version，使 read cache 和 cross-session hint 能通过 version / event 感知变化。

### 5.6 promote_to_system_notebook

```ts
promote_to_system_notebook(input: {
  item_id: string;
  reason: string;
  replace_item_id?: string;
}) -> {
  status: "ok" | "limit_exceeded";
  system_notebook_id: "system";
  active_system_count: number;
  version: string;
}
```

Requirements：

1. active System Notebook items 默认最多 10 条；
2. 如果超过限制且没有 `replace_item_id`，返回 `limit_exceeded`；
3. 被提升的 item 必须是 active、高置信、无 valid_until 或 valid_until 未过期；
4. 不能提升 deleted/stale/superseded item；
5. promote 需要生成 event；
6. prompt compiler 只注入 System Notebook active items 的短内容。

### 5.7 build_notebook_registry_context

```ts
build_notebook_registry_context(input: {
  max_notebooks?: number;
}) -> {
  block_type: "notebook_registry";
  text: string;
  notebooks: Array<{
    id: string;
    description: string;
    active_entry_count: number;
    latest_title?: string;
    latest_updated_at?: string;
    version: string;
  }>;
}
```

Requirements：

1. session 启动时使用；
2. 必须包含当前时间由 prompt compiler 统一注入，Notebook 模块不强行生成当前时间；
3. registry 只披露分类学和版本，不披露正文；
4. 文本中要提醒 Agent：需要事实时再读取，不要重复读取未变化 notebook。

### 5.8 build_system_notebook_context

```ts
build_system_notebook_context(input: {
  max_items?: number;             // default: 10
}) -> {
  block_type: "system_notebook";
  text: string;
  items: Array<{
    item_id: string;
    title: string;
    content: string;
    updated_at: string;
  }>;
}
```

Requirements：

1. 只返回 active system items；
2. 最多 10 条；
3. 不返回 source_excerpt；
4. 不返回低置信或过期 item；
5. 文本应短，不做长文展开。

### 5.9 build_notebook_hints

```ts
build_notebook_hints(input: {
  session_id: string;

  topic_tags?: string[];                  // tag list；由上层 session 合并器输出；缺省=不做 topic 过滤
  candidate_notebook_ids?: string[];
  max_hints?: number;                     // default: 3
}) -> {
  block_type: "notebook_hints";
  hints: Array<{
    notebook_id: string;
    reason: "topic_relevance" | "cross_session_update" | "near_title_update";
    title?: string;
    updated_at?: string;
    version: string;
    matched_tags?: string[];
    text: string;
  }>;
  suppressed: Array<{
    notebook_id: string;
    reason: "already_read_unchanged" | "too_many_hints" | "not_relevant";
  }>;
}
```

Requirements：

1. Hint 只能提示"可能相关"，不能断言事实；
2. `topic_tags` 必须先按 §3.6 规范化；非法 tag 直接返回 `invalid_tag`；
3. 如果当前 session 已读取相同 notebook 且当前 version 未变化，则抑制 hint；
4. topic 相关性判定：将 `topic_tags` 作为过滤器调用统一 `ItemSearchPort.searchItemsByTags`，加 `notebook_id` 过滤；候选按 `updated_at DESC` 排序后挑最新者作为 hint 的展示标题；
5. `topic_tags` 为空时，仅基于 cross-session update event 生成 hint（无 topic_relevance hint）；
6. 不实现自有全文索引；
7. 每轮最多返回 `max_hints` 条 hint；
8. cross-session update hint 只包含 notebook id、title、更新时间、version 与可选的 event.tags（同 §3.5），不包含正文；
9. `build_notebook_hints` 必须自动合并 Notebook Subscription 产生的结果与 topic relevance 结果：同一 notebook 去重，cross-session update 优先于普通 topic relevance；
10. 自动订阅产生的 hint 不依赖 `topic_tags`，因此 `topic_tags` 为空时仍可返回；若有 `topic_tags`，可把 event.tags 与 topic_tags 的交集放入 `matched_tags`；
11. `candidate_notebook_ids` 只限制 topic relevance 候选，不应屏蔽已经由当前 session 自动订阅命中的 cross-session update。

---

## 6. Prompt / Session 集成需求

### 6.1 Session 启动

Prompt compiler 在 session 启动时应注入：

1. 当前时间；
2. Notebook registry block；
3. System Notebook block；
4. 不注入普通 Notebook 全文。

示例文本：

```text
Available notebooks:
- user/preferences: stable or recurring user preferences, 18 active entries, last updated 2026-05-19, latest: "prefers structured design docs", version n_42
- projects/agent-notebook: durable facts and decisions about Agent Notebook, 9 active entries, last updated 2026-05-19, latest: "read cache returns unchanged", version n_7f

Use notebook contents only when relevant. Do not read a notebook repeatedly if it has not changed since the last read in this session.
```

### 6.2 用户消息到达

每次用户消息进入 LLM 之前，prompt compiler 或 background context builder 可调用 `build_notebook_hints`：

1. 由 session 合并器输出当前 topic tags（缺省则不做 topic 过滤）；
2. 调用 `build_notebook_hints` 召回可能相关 notebook，并合并自动订阅产生的 cross-session update；
3. 过滤已读取且未变化的 notebook；
4. 将 NotebookHint 映射为 `agent-note` / background hint 注入；
5. 不直接注入事实正文。

Session 合并器是 tags 的唯一来源。Notebook / Memory 模块都不构造 tags、不翻译、不做语言检测。

### 6.3 Agent 读取 Notebook

Agent 只有在需要长期事实时才调用 `read_notebook`。读取结果返回后：

1. `status = ok`：把 entries 放入工具结果；
2. `status = unchanged`：只放短提示，让 Agent 回看当前上下文中之前的读取结果；
3. 更新 read cache；
4. 自动订阅该 notebook，以及本次明确读取或返回的 item title；
5. 后续 hint 需要根据 read cache 和 Notebook Subscription 抑制或触发。

Agent 应优先用 tags 召回（不知该传什么 tag 时直接不传，等价于读全部 active items）；只在已知具体 item 或 title 的场景下用 `item_ids` / `title`。

### 6.4 Agent 写入 Notebook

在线写入只处理强意图：

1. 用户明确说"记一下"；
2. 用户明确设定长期规则或偏好；
3. 当前任务产生明确需要跨 session 保留的项目状态；
4. 系统强规则要求记录。

在线 Agent 不应为了每个隐含事实都调用 `append_note`。隐含事实发现交给 self-improve / curator。

写入时应给出合理 tags：

1. tags 是固有召回信号，应能覆盖该 fact 在不同语境下可能出现的核心词；
2. 在线 Agent 默认从 content + title 抽取 2–6 个 tag；
3. tags 必须英语化，规则与 Memory v2 §8.3 / 本文 §3.6 一致；
4. 非英语原文可放在 source_excerpt，不应进入 tag。

写入成功后，当前 session 自动订阅 `notebook_id + title`。这个订阅只用于后续其它 session 更新同一 notebook 或相同/相近 title 时产生轻量 hint，不改变 append-first 写入语义。

---

## 7. Cross-session update hint

### 7.1 触发条件

当 session B 更新某个 notebook 后，如果 session A 满足任一条件，则 session A 下一轮可收到轻量 update hint：

1. session A 通过 `agent-notebook read` 自动订阅过该 `notebook_id`；
2. session A 通过 `agent-notebook append` 自动订阅过该 `notebook_id + title`；
3. session A 读过、写过、或自动订阅过相同或相近 title；
4. 更新 item 的 tags 与 session A 当前 topic tags 有交集（按规范化后比较）；
5. 更新事件可能与 session A 已读 item 冲突。

触发方必须是其它 session：如果更新事件的 `actor_session_id` 等于 session A，本轮默认不产生 hint。

### 7.2 Hint 内容

必须是元信息，不含完整正文：

```text
Notebook update since your last turn:
- projects/agent-notebook updated at 2026-05-19T14:12:03+08:00
- changed title: "read cache and unchanged responses"
- tags: read-cache, unchanged-response
- possible relevance: you previously read this notebook in this session
```

### 7.3 去重

需要维护 session event watermark：

```ts
interface SessionNotebookEventWatermark {
  session_id: string;
  last_seen_seq: number;
  last_seen_at: string;
}
```

Requirements：

1. 同一个 event 不应反复提示；
2. hint 被注入后即可更新 watermark，或在 prompt compiler 成功提交后更新；
3. 如果 session 与写入 session 相同，默认不提示自身写入事件；
4. hint 数量受 `max_hints` 限制。

---

## 8. 冲突、覆盖、过期

### 8.1 冲突检测

append_note 返回 possible conflicts，但不阻止 append。

冲突候选来源：

1. 同 notebook 下规范化 title 完全相同；
2. 同 notebook 下 title 近似；
3. 用新 item 的 tags 调用 `ItemSearchPort.searchItemsByTags` 召回 top-N active item（`reason = "tag_overlap"`，输出 `matched_tags`）；
4. 当前 active item 的 valid time 与新 item 重叠；
5. curator 显式标记 `conflicts_with`。

不要为冲突检测实现独立全文索引。

### 8.2 覆盖语义

不允许直接改写旧 content 来表达覆盖。正确方式：

1. append 新 item；
2. 创建 `supersedes` edge；
3. 将旧 item 标记为 `superseded`；
4. notebook version 增加；
5. 生成 event。

### 8.3 过期语义

如果 `valid_until` 已过期：

1. 默认 read 不返回；
2. registry 统计可不计入 active_entry_count；
3. curator 可定期标记为 stale；
4. 不强制硬删除。

### 8.4 Status 默认过滤

默认读取行为：

```text
include_status = ["active"]
exclude expired valid_until
```

显式历史读取可传：

```text
include_status = ["active", "stale", "superseded"]
```

---

## 9. CLI 形态建议

如果代码库倾向用 CLI 暴露 agent 工具，可以实现 `agent-notebook`。命令形态与 Agent Memory v2.10 保持一致风格：subcommand + positional + flags，CLI 不接受 JSON 入参。

Notebook selector 约定：

1. `read` / `append` 使用可选 `--bookid <notebook_id>` 选择 notebook，不再把 `notebook_id` 作为 positional 参数；
2. `read` / `append` 不传 `--bookid` 时，CLI 必须解析为当前 `owner_agent` 的默认 notebook；
3. `create-notebook` 是显式 notebook 管理命令，必须传 `--bookid <notebook_id>`，不使用默认 notebook 兜底；
4. CLI 在调用底层接口前必须得到 concrete `notebook_id`；底层 read cache、subscription、event、返回 JSON 都使用解析后的 id；
5. `status` / `promote` 这类以 `item_id` 定位既有条目的命令，不要求 `--bookid`，其 notebook 归属由 item 元数据确定。

Owner / session / actor selector 约定：Agent Notebook 是当前 Agent 专用的 durable notebook。CLI 从 AgentTool 最小环境契约和 Agent RootFS identity 推导当前 Agent appid、agent id 与 session id；命令行参数只表达业务语义，不需要传 `--owner-user`、`--session`、`--actor-kind`、`--write-reason`。

### 9.1 list

```bash
agent-notebook list \
  [--include-archived]
```

输出 JSON。

### 9.2 read

```bash
agent-notebook read \
  [--bookid <notebook_id>] \
  [--tags <tag1,tag2,tag3>] \
  [--title <title>] \
  [--latest 10] \
  [--items <item_id1,item_id2>] \
  [--since-version <version>] \
  [--max-items 10] \
  [--max-bytes 12000]
```

输出 `NotebookReadResult` JSON。

`--bookid` 是可选 notebook 选择器；不传时读取当前 `owner_agent` 的默认 notebook。`read` 不再要求 positional `notebook_id`。

`--tags` 是 tag list，逗号拆分；trim 后按 §3.6 规范化。**不传 `--tags` 即无过滤，返回该 notebook 全部 active items（按时间倒序）**；`--tags '*'` 语义等价于不传。

互斥优先级：`--items` > `--title` > `--tags` > `--latest` > 全部缺省（= all_active）。当多个同时给出时，CLI 按优先级取一个并在 stderr warning 提示忽略的参数。

当读取结果为 `ok` 时，CLI 应把当前 session 自动订阅到解析后的 `notebook_id`，并订阅本次读取输入或返回 entries 中涉及的 title。

### 9.3 append

短 content：

```bash
agent-notebook append <title> <content> \
  [--bookid <notebook_id>] \
  [--confidence high] \
  [--tags read-cache,unchanged-response] \
  [--valid-until 2026-12-25]
```

长 content 走 stdin：

```bash
cat note.md | agent-notebook append <title> \
  [--bookid <notebook_id>] \
  --tags read-cache,unchanged-response \
  --stdin
```

`--bookid` 是可选 notebook 选择器；不传时写入当前 `owner_agent` 的默认 notebook。`append` 不再要求 positional `notebook_id`。

如果解析后的目标 notebook 不存在，append 成功路径必须先自动创建该 notebook，再写入 note；这保证 Agent 可以直接向默认 notebook 写入。

`--tags` 同样是逗号拆分的 ordered list；写入时强制规范化校验。

当 append 成功时，CLI 应把当前 session 自动订阅到解析后的 `notebook_id + title`。后续其它 session 修改同一 notebook 或相同/相近 title 时，`build_notebook_hints` 会把这类订阅命中合并到返回的 hint block。

### 9.4 create-notebook

```bash
agent-notebook create-notebook \
  --bookid <notebook_id> \
  [--kind normal|project|agent] \
  [--title <title>] \
  [--description <description>]
```

`--bookid` 是必填 notebook id；`create-notebook` 不再要求 positional `notebook_id`，也不使用默认 notebook 兜底。

### 9.5 status

```bash
agent-notebook status <item_id> stale \
  --reason "expired seasonal preference"
```

### 9.6 remarks

```bash
agent-notebook remarks list <item_id> \
  [--type <remark_type>]

agent-notebook remarks append <item_id> <remark_type> <content>

cat remark.md | agent-notebook remarks append <item_id> <remark_type> \
  --stdin

agent-notebook remarks remove <item_id> <remark_id>
```

`remarks list` 默认返回 active remarks；`--type` 只返回指定备注类型。`remarks append` 不覆盖已有 remark。`remarks remove` 是软删除。

### 9.7 registry-context

```bash
agent-notebook registry-context
```

### 9.8 hints

```bash
agent-notebook hints \
  --topic-tags agent-notebook,state-management,technical-requirement \
  --max-hints 3
```

CLI 入参不要求 JSON；输出建议统一 JSON，便于工具层消费。

---

## 10. 错误与返回码

### 10.1 结构化错误

```ts
interface NotebookError {
  status: "error";
  code:
    | "not_found"
    | "permission_denied"
    | "invalid_input"
    | "invalid_tag"
    | "version_conflict"
    | "limit_exceeded"
    | "item_search_unavailable"
    | "storage_error";
  message: string;
  details?: Record<string, unknown>;
}
```

`invalid_tag` 是 `invalid_input` 的子类语义，专指 §3.6 校验失败；details 应给出第一个失败的 tag 原始值。

### 10.2 unchanged 不是错误

`unchanged` 是正常返回，不应作为系统异常处理。它的目的是减少上下文污染。

如果工具框架支持"非致命错误提示"，可以映射成 warning；但默认建议 `exit code = 0`，JSON 中 `status = "unchanged"`。

### 10.3 Item Search 不可用

如果统一 Item Search 不可用：

1. 精确 title / latest / items 读取仍可工作；
2. tag 召回（`tags` 非空且不为 `*`）返回 `item_search_unavailable`；
3. 不得临时创建本地全文索引作为 fallback；
4. append_note 的 tag-overlap 冲突检测可降级为 same_title / near_title 检测，并在返回的 possible_conflicts 中省略 tag_overlap reason。

---

## 11. 权限与隐私

### 11.1 Owner scope

Notebook DB 位于当前 Agent RootFS 的 `notebook/` 下，owner / agent scope 由运行时路径和身份元信息确定，不在 notebook SQL 表中重复保存：

1. SQL schema 不包含 `owner_user_id` / `owner_agent_id` 列；
2. 查询不再按 owner 列过滤，而是依赖当前打开的 Agent Notebook DB；
3. session 必须有权限访问该 Agent RootFS；
4. 不同用户或 Agent 之间不得共享同一个 notebook DB 路径。

### 11.2 Registry 隐私

Registry 可以暴露 notebook 名称、描述、数量、最近标题，但不暴露：

1. item content；
2. source_excerpt；
3. source_ref 详细信息；
4. deleted item；
5. 低权限 session 不可见的 notebook。

### 11.3 删除

`deleted` 为软删除。未来如需隐私删除，可增加：

```ts
purge_note(item_id, reason)
```

MVP 不必实现 purge，但数据模型不要阻碍未来硬删除。

---

## 12. 事务与并发

### 12.1 写事务

`append_note` 必须在一个事务内完成：

1. 确认 notebook 存在或创建；
2. 写入 Item Store；
3. 写入 NotebookItem row；
4. 更新 Notebook revision/version/statistics；
5. 写入 NotebookEvent；
6. 提交。

如果任一步失败，整体回滚，避免 registry 与 item store 不一致。

如果 Item Store 无法参与同一事务，必须实现补偿或 outbox：

1. 先写 notebook pending record；
2. 写 Item Store；
3. finalize notebook record；
4. 失败时可重试或标记 repair_needed。

### 12.2 Version

推荐：

```text
version = "n_" + revision + "_" + short_hash(latest_event_id or notebook_state_hash)
```

Requirements：

1. 每次 mutation 后 version 必须变化；
2. read 不改变 version；
3. version 是 opaque string，调用方不得解析；
4. revision 可用于内部排序和并发检查。

### 12.3 并发写

多 session 并发 append：

1. 都应成功 append；
2. revision 按提交顺序递增；
3. 不因 title 相同而覆盖；
4. possible_conflicts 只作为提示；
5. event 顺序以提交顺序为准。

状态变更可选 optimistic lock：

```text
expected_item_revision
```

不匹配返回 `version_conflict`。

---

## 13. Self-improve / Curator 集成

本模块不实现 LLM 提取，但必须支持 curator 使用。

Curator 可做：

1. 从 session history 提取 durable facts；
2. 调用 `append_note` 写入（必须带 tags，规范化后存储）；
3. 调用 `mark_note_status` 标记 stale/superseded/deleted；
4. 调用 `promote_to_system_notebook` 提升少量强约束；
5. 创建 conflict/supersede edges；
6. 在 Memory 侧维护**指向** Notebook 事实的结构化线索（如 relation / event_effect claim，evidence 指向 `notebook_id`），但**不得把 Notebook 事实正文复制进 Memory**（互斥召回源，见 §1.3）；两者共用 ordered tag 词表。

Curator 写入要求：

1. `write_reason = "curator_extracted"` 或 `curator_cleanup`；
2. 必须带 source_ref 或 source_excerpt；
3. 必须给 confidence；
4. 对推断事实建议 confidence 不高于 medium，除非来源明确；
5. 必须给 tags。因为同一事实正文不在 Memory 与 Notebook 间重复，tags 对齐的目标**不是**让"同一条事实"两处都召回，而是让同一组 ordered tags 能分别命中各模块内的相关记录（Notebook 的事实正文 / Memory 的结构化线索）。

---

## 14. 测试需求

### 14.1 Unit Tests

必须覆盖：

1. create notebook；
2. list registry 不含 content；
3. append note 增加 entry_count / active_entry_count；
4. append note 使 version 变化；
5. append note 拒绝非法 tag（长度、字符集、空格归一化）；
6. read latest 返回 active items；
7. read by tags 调用 `ItemSearchPort.searchItemsByTags`；
8. read by tags 不实现本地索引；
9. read 同 (notebook, scope_hash) 同 version 返回 unchanged；
10. read 不同 tag set 不误返回 unchanged；
11. read tag set 顺序不同但规范化后相同：scope_hash 相同；
12. read tag set 元素不同：scope_hash 不同；
13. read all_active 后同 version 下任意 tag scope 可返回 unchanged；
14. read 不传 tags = 读全部 active items（按 updated_at DESC）；
15. read 传 `["*"]` 与不传 tags 行为完全一致；
16. **所有模式返回的 entries 必须按 `updated_at DESC, created_at DESC, item_id ASC` 严格排序**，包括同时命中多 tag 的 entry 不会因此排到前面；
17. mark stale 后默认 read 不返回该 item；
18. supersede edge 创建后旧 item 默认不返回；
19. system notebook 超过 10 条返回 limit_exceeded；
20. cross-session event 生成且包含 event.tags；
21. hint 对已读未变化 notebook 被 suppress；
22. hint topic_tags 非法时返回 invalid_tag；
23. hint topic_tags 缺省时仍可基于 update event 生成 cross_session_update hint。

### 14.2 Integration Tests

必须覆盖：

1. session A read notebook，session B append note，session A 下轮收到 update hint；
2. session A read notebook 后未变化，多次 hint build 不重复提示；
3. append_note 返回 same_title conflict；
4. append_note 返回 tag_overlap conflict，且 matched_tags 非空；
5. Item Search unavailable 时 tag 召回返回结构化错误，latest / title / items 读取仍可工作；
6. Item Search unavailable 时 append_note 的 tag_overlap 检测降级，不抛错；
7. prompt registry block 不含正文；
8. System Notebook block 只含 active high-confidence unexpired items；
9. Memory v2 和 Notebook 用同一组 ordered tags 都能召回到对应记录（端到端 smoke）。

### 14.3 Regression Tests

至少添加以下回归测试：

1. `unchanged` 结果不得包含 entries；
2. stale item 不得进入 System Notebook context；
3. deleted item 不得被 tag 召回返回；
4. cross-session hint 不得包含 content；
5. append-first 不得覆盖旧 content；
6. 同 title 并发写必须产生两条 item，而不是最后写覆盖；
7. tag 规范化函数在写入路径和读取路径必须是同一份实现（同输入同输出）。

---

## 15. MVP 交付范围

### 15.1 必须交付

1. Notebook / NotebookItem / Event / ReadCache 数据模型；
2. Tag 规范化与校验函数（§3.6），写入路径与读取路径共用；
3. `list_notebooks`；
4. `create_or_update_notebook`；
5. `read_notebook`，含 tags/latest/title/items 四种召回模式；
6. `append_note`，含 tag_overlap 冲突检测；
7. `mark_note_status`；
8. `build_notebook_registry_context`；
9. `build_system_notebook_context`；
10. `build_notebook_hints` 的基础版（输入 topic_tags）；
11. Cross-session update event 和 watermark；
12. System Notebook 10 条上限；
13. 单元测试和集成测试；
14. 统一 Item Store / Item Search 适配层调用（`searchItemsByTags`）。

### 15.2 可暂缓

1. 复杂 conflict 自动合并；
2. LLM curator 具体 prompt 和调度；
3. 大规模权限模型；
4. hard purge；
5. 知识库引用展开；
6. 向量召回策略调优；
7. UI 管理界面；
8. 多语言 title normalization 高级算法。

---

## 16. 实现顺序建议

### Phase 1：Domain + Storage

1. 新增 Notebook、NotebookItem row、Event、ReadCache 存储；
2. 接入 ItemRepositoryPort；
3. 确保 append 是事务性；
4. 实现 version/revision；
5. 实现 §3.6 tag 规范化函数，作为 shared util。

### Phase 2：Core APIs

1. list/create；
2. append（含 tag 校验、event.tags）；
3. read latest/title/items；
4. mark status；
5. 单元测试。

### Phase 3：Item Search 集成

1. read by tags 调用 `ItemSearchPort.searchItemsByTags`；
2. append conflict 检测调用 `searchItemsByTags`；
3. Item Search unavailable 的错误路径与降级；
4. 明确不创建任何 Notebook 私有索引。

### Phase 4：Session Integration

1. read cache（含 read_scope_hash 计算）；
2. unchanged；
3. registry context；
4. system notebook context；
5. hint filtering（含 topic_tags）。

### Phase 5：Cross-session Events

1. event outbox；
2. session watermark；
3. update hints（含 event.tags 与 topic_tags 交集判定）；
4. 集成测试。

---

## 17. Code agent 注意事项

实现时请特别遵守：

1. 不要把 Notebook 做成聊天记录摘要表；
2. 不要把 Memory 和 Notebook 合并；
3. 不要在启动时返回全文；
4. 不要为 Notebook 单独建全文索引；
5. 不要用 last-write-wins 覆盖旧 note content；
6. 不要让 tag 召回绕过 owner scope；
7. 不要让 `unchanged` 对不同 tag set 误触发；
8. 不要让 system notebook 无限制增长；
9. 不要把 hint 文本写成事实断言；
10. 不要把 expired/stale/deleted item 注入 prompt；
11. 不要接受自然语言 query 字符串作为召回输入；接口语义是 tag list；
12. 不要在 Notebook 模块内私自构造 tags（翻译、抽取、扩展同义词都不是本模块职责）；那是 session 合并器和 curator 的活；
13. tag 规范化函数在所有路径必须是同一实现，禁止在某处临时小写、某处不小写；
14. **不要对 tag 命中数做排序加分**——Notebook 的排序键固定是 `updated_at DESC, created_at DESC, item_id ASC`，不沿用 Memory v2 的 tag-position boost；
15. **不要把 `tags` 缺省等同于"必须传 tag"** 的报错——空 tags 是合法输入，等价于读全部 active items。

---

## 18. 验收清单

交付时至少确认：

- [ ] `agent.notebook.note` 或等价 item type 已注册到统一 Item 系统；
- [ ] Note Item 写入后可被统一 Item Search 检索；
- [ ] Notebook 模块没有新增 FTS/BM25/embedding index 实现；
- [ ] `list_notebooks` 不返回正文；
- [ ] `read_notebook` 召回输入是 tag list（或 title/latest/items），不接受自由文本 query；缺省 tag = 读全部 active items；
- [ ] `read_notebook` 返回 entries 严格按 `updated_at DESC` 排序（验证多 tag 命中条目不被加权排前）；
- [ ] `read_notebook` 支持 `status: "unchanged"`；
- [ ] read cache 按 (notebook, scope_hash) 区分 mode 与 tag set；
- [ ] tag 规范化函数（§3.6）通过单测；写入路径与读取路径同一实现；
- [ ] append-first 行为通过测试；
- [ ] tag_overlap 冲突检测通过 `searchItemsByTags`；
- [ ] status 过滤通过测试；
- [ ] cross-session update hint 不含正文；
- [ ] System Notebook 上限通过测试；
- [ ] 所有操作按当前 Agent Notebook DB 路径隔离；
- [ ] tag 召回统一走 ItemSearchPort；
- [ ] ItemSearchPort 不可用时不会偷偷 fallback 到私有索引，但 latest/title/items 路径仍可工作。

---

## 19. 一句话技术定义

Agent Notebook 是一个基于统一 Item 存储与检索系统、与 Agent Memory v2 共享 tag 词表与规范化规则的长期事实层：召回入口接受 tag list 作为过滤器（缺省即无过滤），结果统一按时间倒序返回；Notebook 模块负责事实条目的组织、版本、读写语义、上下文 hint、cross-session 同步和状态整理；Item 系统负责条目正文的统一索引与检索；Prompt compiler 负责只在合适时机注入 registry、system items 和轻量 hints，而不是注入整本 Notebook。
