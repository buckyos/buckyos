# MessageHub UI DataModel

- 文档版本：v0.1
- 文档类型：UI DataModel 设计文档（WebUI Dev Loop 阶段三产物）
- 模块位置：`src/frame/desktop/src/app/messagehub`
- 上游文档：
  - PRD：`product/message_hub/MessageHub_Web_UI_PRD.md` v0.1
  - 原型现状盘点：`product/message_hub/MessageHub_Current_UI_Model_Data.md` v0.1
  - 后端数据模型：`src/kernel/buckyos-api/src/msg_center_client.rs`、`src/frame/msg_center/src/`
- 下游用途：`integrate-ui-datamodel-with-backend`

---

## 1. Overview

### 1.1 本文档解决的问题

MessageHub 原型已经收敛（Entity List / Conversation View / Details 三层结构 + 多 Session 切换 + 消息投影 + Composer 草稿），
但原型的数据来自 `mock/data.ts`，其模型与 `msg-center` 的真实数据模型之间存在三处结构性错位：

1. **后端没有 Entity 概念。** `msg-center` 的读取面只有 `SessionSummary`（会话摘要）与 `SessionMessageItem`（会话时间线），
   两者都以扁平的 `session_id` 为键。UI 的「实体 → 会话」两级结构必须由前端投影得到。
2. **后端 Session 没有标题、置顶、静音、标签。** `MailboxRecord` 只有 `session_id` / `sort_key` / `state` / `tags`。
   UI 需要的展示态属性只能落在 `ui_session.*` 这组 KV 接口上。
3. **原型的列表摘要与消息历史是两套并行数据。** `Entity.lastMessage` 与 `MessageObject[]` 各自独立，
   接后端后必须收敛为「会话摘要是消息时间线的派生投影」。

本文档定义的就是这三层之间的稳定边界：**协议层 → UI DataModel 层 → 组件层**。

### 1.2 分层约定

```text
协议层（Rust 镜像，禁止按 UI 需要改形）
  protocol/msgobj.ts        —— MsgObject / MsgContent / RefItem
  datamodel/sessionApi.ts   —— SessionSummary / SessionMessageItem / MailboxRecord / 分页游标
        │
        │  projection（本文档第 3 章定义的映射规则）
        ▼
UI DataModel 层（本文档定义，UI 需求驱动）
  Entity / EntitySession / EntityDetail / ConversationListItem / ComposerDraft / MessageHubViewState
        │
        ▼
组件层
  EntityList / SessionSidebar / ConversationView / ConversationHistoryPane / EntityDetails / ConversationComposer
```

约束：

- 协议层类型 **不得** 为了 UI 方便新增字段；UI 需要的提示信息一律走 `ui_*` 扁平 meta 或 `ui_session` KV。
- UI DataModel 层 **不得** 1:1 镜像 KRPC 结构。`SessionSummary` 与 `EntitySession` 不是同一个东西。
- 组件层 **不得** 直接调用 `datamodel/sessionApi.ts`，只消费 UI DataModel。

### 1.3 覆盖的视图

| 视图 | 组件 | 主数据 |
|------|------|--------|
| Panel A 实体列表 | `EntityList` | `Entity[]` + `EntityListQuery` |
| Panel A 下钻面板 | `EntityList` / `DrilldownPanel` | `Entity.children` + `Entity.childrenSections` |
| Session 侧栏 | `SessionSidebar` | `EntitySession[]` |
| Panel B 会话视图 | `ConversationView` / `ConversationHistoryPane` | `ConversationProjection` + `ConversationListItem[]` |
| Panel B 输入区 | `ConversationComposer` | `ComposerDraft` |
| Panel C 详情 | `EntityDetails` | `EntityDetail` |

---

## 2. 协议层参考（不属于 UI DataModel，但决定其边界）

只列出 UI 投影会读到的部分，完整定义见 `datamodel/sessionApi.ts` 与 `msg_center_client.rs`。

```ts
type MailboxKind = 'INBOX' | 'SENT' | 'GROUP_INBOX' | 'REQUEST_BOX'
type RecipientState = 'UNREAD' | 'READING' | 'READ' | 'ARCHIVED' | 'DELETED'
type SessionMessageDirection = 'in' | 'out'
type SessionDeliveryOverall = 'sending' | 'delivered' | 'partial_failed' | 'failed'

interface SessionSummary {
  session_id: string
  last_record?: MailboxRecordWithObject
  unread_count: number
  updated_at_ms: number
}

interface SessionMessageItem {
  record_id: string
  msg_id: string
  direction: SessionMessageDirection
  box_kind: MailboxKind
  sort_key: number
  from: DID
  to: DID
  recipient_state?: RecipientState   // 仅 inbound
  delivery?: SessionDeliveryView     // 仅 outbound
  msg?: MsgObject | null
}
```

### 2.1 `session_id` 的后端生成规则（决定 UI 的实体归属推导）

`msg_center.rs::derive_session_id` 的优先级：

1. `msg.thread.topic`
2. `msg.thread.correlation_id` / `meta.session_id` / `meta.owner_session_id` / 若干 payload 指针
3. 群消息：`msg.to[0]`（即 group DID）
4. 直接消息：`SENT` 箱用 `dm:{msg.to[0]}`，收件箱用 `dm:{msg.from}`

**这是 UI 能把扁平 session 归到实体下的唯一线索**，第 3.2 节的映射表直接建立在它之上。

### 2.2 后端未提供、必须由 UI 侧存储的属性

`MailboxRecord` 与 `SessionSummary` 都没有以下字段，UI 需要的它们只能落在 `ui_session.update_state` /
`ui_session.get_state` / `ui_session.list_state`（`UiSessionStateEntry { session_id, key, value, updated_at_ms }`）：

- 会话标题（用户重命名）
- 会话置顶 / 静音
- 会话本地标签
- 未发送草稿
- 客户端已读水位

第 4.4 节给出这组 KV 的保留 key 约定。

---

## 3. UI DataModel 定义

### 3.1 标识与基础类型

```ts
/** 规范化后的实体 DID。UI 内所有实体引用都用它，不再使用 mock 风格的短 id。 */
export type EntityId = DID

/** msg-center 的会话投影键，对 UI 不透明，禁止解析后用于业务判断（3.2 的归属推导除外）。 */
export type SessionId = string

export type EntityType = 'person' | 'agent' | 'group' | 'service'

/** PRD §6.5 / §6.6：域内实体可管理，域外实体只读 + 轻量标注。 */
export type EntityDomain = 'managed' | 'external'
```

`EntityType` 与 `EntityDomain` 的判定顺序（前端 `resolveEntityKind()`）：

| 条件 | type | domain |
|------|------|--------|
| DID 命中 `group.list_by_member` / `group.list_subgroups` 返回集 | `group` | `is_hosted_by_self ? 'managed' : 'external'` |
| DID method 为 `did:msgtunnel:*` 且 `account_type` 为 `group` / `channel` | `group` | `external` |
| DID method 为 `did:msgtunnel:*` 且 `account_type` 为 `user` / `addr` | `person` | `external` |
| DID 命中 zone 内 agent 注册（或 `Contact.tags` 含 `agent`） | `agent` | `managed` |
| DID 为系统服务（`msg-center` / `scheduler` / `task-mgr` 等） | `service` | `managed` |
| DID method 为 `did:bns:*` 且属于本 zone | `person` | `managed` |
| 其余 | `person` | `external` |

`domain` 决定 Panel C 的能力：`external` 只暴露备注 / 标签 / 只读来源信息；`managed` 才允许跳转管理页。

### 3.2 Entity

Entity 在后端没有对应实体，由三路数据合并派生：

```text
contact.list_contacts   → Contact[]        身份、备注、绑定、访问级别
group.list_by_member    → GroupSummary[]   群名称、成员数、可发消息
msg.list_sessions       → SessionSummary[] 活跃度、未读、最近一条消息
```

```ts
export interface Entity {
  /** 规范化 DID。经 contact.resolve_canonical_did 处理，别名 DID 不会产生重复实体。 */
  id: EntityId
  type: EntityType
  domain: EntityDomain
  /** 展示名。优先级：Contact.name → GroupSummary.name → MailboxRecord.from_name → DID 短写。 */
  name: string
  /** 头像 URL。当前渲染层仍按 type 生成图标，此字段为预留位。 */
  avatar?: string

  /** 单行状态说明，如 "online" / "12 members" / "last seen 2h ago"。由 presence 与实体类型派生。 */
  statusText?: string
  /** 在线态。仅 managed 实体可信；external 实体恒为 undefined。 */
  isOnline?: boolean

  /** 置顶。UI 侧属性，落在实体主 session 的 ui_session KV。 */
  isPinned?: boolean
  /** 静音。同上。静音仅影响提醒与 badge 配色，不影响 unreadCount 计数。 */
  isMuted?: boolean
  /** 该实体下所有可见 session 的 unread_count 之和。 */
  unreadCount: number

  /** 过滤标签。合并 Contact.tags 与 UI 本地标签，去重后按字典序。 */
  tags: string[]

  /** 最近一条消息摘要。由 lastActivitySessionId 对应 SessionSummary.last_record 派生，不独立存储。 */
  lastMessage?: MessagePreview
  /** max(session.updated_at_ms)。实体列表默认排序键。 */
  lastActiveAt: number
  /** 产生 lastMessage 的 session，点击实体时默认打开它。 */
  lastActivitySessionId?: SessionId

  /** 该实体下可见 session 数。等于 1 时 ConversationView 隐藏 Session 入口。 */
  sessionCount: number

  /** 子实体（PRD §6.2）。群的 subgroup / topic、服务下的房间。子实体本身也是 Entity。 */
  children?: Entity[]
  /** 子实体呈现方式：inline 原地展开（PRD §7.7 推荐），drilldown 替换列表。 */
  childrenMode?: EntityChildrenMode
  /** drilldown 面板的分组配置。 */
  childrenSections?: EntityChildrenSection[]
  /** drilldown 总览卡片描述。 */
  drilldownDescription?: string

  /** 协议来源标签集合，如 ['buckyos'] / ['telegram','email']。来自 AccountBinding.platform 与 ingress.platform。 */
  sources: string[]
}

export type EntityChildrenMode = 'inline' | 'drilldown'

export interface EntityChildrenSection {
  id: string
  title: string
  description?: string
  childIds: EntityId[]
}

export interface MessagePreview {
  /** 群会话展示发送者前缀；单聊为 undefined。 */
  senderName?: string
  /** 纯文本摘要。非文本消息由 3.5 节的 summarize 规则生成，禁止直接塞入 HTML/Markdown 原文。 */
  text: string
  timestamp: number
}
```

#### 3.2.1 session → entity 归属推导

`resolveEntityDid(summary: SessionSummary): EntityId | null`：

| `session_id` 形态 | 后端来源 | 实体 DID |
|---|---|---|
| `dm:<did>` | `derive_session_id` 分支 4 | `contact.resolve_canonical_did(<did>)` |
| 可在群集合中命中的 DID | 分支 3 | 该 group DID |
| 其他任意字符串（topic / correlation_id） | 分支 1、2 | 取 `last_record.record`：`msg_kind === 'group_msg'` 时用 `record.to`；否则用 `record.from` 与 `record.to` 中非 owner 的一侧，再 `resolve_canonical_did` |
| 无 `last_record` 且不匹配上述任一形态 | — | 归入 `unassigned` 桶，见下 |

无法归属的 session 不得丢弃：投影到一个内置的 `service` 型实体
`Entity { id: 'urn:buckyos:messagehub:unassigned', type: 'service', domain: 'managed' }`，
保证「后端有数据但 UI 看不见」不会静默发生。

#### 3.2.2 子实体来源

| 子实体场景 | 后端来源 | childrenMode |
|---|---|---|
| 群下的 subgroup / topic | `group.list_subgroups` | `inline` |
| 聚合型服务实体（如 Release Hub）下的房间 / agent / 系统 | 前端配置 + `childrenSections` | `drilldown` |

子实体 **不是** Session（PRD §9.4）：子实体持久存在于实体层级，Session 是实体下的上下文容器。

### 3.3 EntitySession

```ts
export type SessionKind = 'chat' | 'task' | 'workspace'

export interface EntitySession {
  /** 后端 session_id 原值。 */
  id: SessionId
  /** 归属实体，由 3.2.1 推导得到。 */
  entityId: EntityId

  /** 展示标题，来源见 3.3.1。永远非空。 */
  title: string
  /** 标题来源，决定是否允许重命名与是否显示来源图标。 */
  titleSource: SessionTitleSource

  /** 会话类型，决定 Conversation View 的渲染形态（PRD §6.4 / §11）。当前仅 chat 有完整实现。 */
  kind: SessionKind
  /** 协议来源，如 'buckyos' / 'telegram' / 'linear'。驱动 SessionSidebar 的前导图标与配色。 */
  source?: string

  unreadCount: number
  /** = SessionSummary.updated_at_ms。Session 列表默认降序排序键。 */
  lastActiveAt: number
  /** 该会话最近一条消息摘要，供 Session 列表二行展示（当前原型未展示，字段为已定义可选项）。 */
  lastMessage?: MessagePreview

  /** 该会话最近一条出站消息的聚合投递状态。用于在 Session 列表上暴露发送失败。 */
  lastDelivery?: SessionDeliveryOverall

  isPinned?: boolean
  isMuted?: boolean
}

export type SessionTitleSource =
  /** 用户在 ui_session KV 里显式重命名过 */
  | 'user'
  /** 来自 msg.thread.topic */
  | 'topic'
  /** 来自协议来源，如 "Telegram" */
  | 'platform'
  /** dm: 前缀会话的默认标题 */
  | 'direct'
  /** 群会话默认标题（等同群名） */
  | 'group'
```

#### 3.3.1 title 派生优先级

1. `ui_session.get_state(session_id, 'ui.title')` → `titleSource: 'user'`
2. `last_record.msg.thread.topic` → `'topic'`
3. `session_id` 形如 `dm:*` → i18n `messagehub.session.direct`（"Direct Message"）→ `'direct'`
4. 群 session → 群名 → `'group'`
5. 其余 → 来源平台展示名 → `'platform'`

**原型中的 `isActive` 字段废弃。** 活跃态由 `MessageHubViewState.selectedSessionId` 单独判定，
不再作为数据字段下发——原型里它已经没有任何消费点。

### 3.4 EntityDetail

```ts
export interface EntityDetail extends Entity {
  /** 简介。Contact 无对应字段时取 GroupDoc.description。 */
  bio?: string
  /** 用户备注，可编辑，写回 contact.update_contact 的 ContactPatch.note。 */
  note?: string
  /** 协议账号绑定。 */
  bindings: AccountBinding[]
  /** 群成员数，仅 type === 'group' 时有值。来自 GroupSummary.member_count（GroupDoc 不含该字段）。 */
  memberCount?: number
  /** 访问级别，决定详情页的权限区块与「拉黑 / 临时授权」动作。 */
  accessLevel: AccessGroupLevel
  /** 联系人来源，用于区分自动推断出的影子联系人与用户手工创建的联系人。 */
  contactSource?: ContactSource
  /** 身份是否已验证。影响详情页的信任提示。 */
  isVerified: boolean
  createdAt?: number
}

export interface AccountBinding {
  platform: string
  accountId: string
  /** 面向用户展示的账号标识，如 '@alice_chen'。 */
  displayId: string
  /** 平台侧实体类型：user / group / channel / addr。空串表示非 tunnel 端点绑定。 */
  accountType?: string
  /** 该绑定投影出的影子端点 DID（did:msgtunnel:*）。回复外部平台消息时的目标。 */
  endpointDid?: DID
  lastActiveAt?: number
}

export type AccessGroupLevel = 'block' | 'stranger' | 'temporary' | 'friend'
export type ContactSource = 'manual_import' | 'manual_create' | 'auto_inferred' | 'shared'
```

相比原型：`bindings` 由可选改为必填（空数组表示无绑定，避免 `undefined` 与 `[]` 双态判断）；
新增 `accessLevel` / `isVerified` / `contactSource`，这三项是后端 `Contact` 已有、而 PRD §12.4/§15.1
的权限差异化要求必须消费的字段。

### 3.5 会话消息模型

**设计决定：Conversation 层继续直接消费协议对象 `MsgObject`，不引入独立的 `ConversationMessageVM`。**

理由：MessageHub 是聚合型 UI，需要兼容任意 IM 协议可承载的内容类型（PRD §8.7）。
再套一层 VM 会把「未识别内容类型」在映射阶段就丢掉，而直接消费协议对象可以让 fallback 渲染器拿到原始载荷。

代价是需要一个明确的 UI meta 契约：

```ts
/**
 * UI 提示元数据。Rust 侧把 MsgObject.meta 扁平化到顶层，因此这些 key 直接挂在 MsgObject 上。
 * 全部由前端投影层（sessionItemToMessageObject）写入，协议侧不产出、也不消费。
 */
export interface MessageUiMeta {
  /** 稳定消息 id。取 SessionMessageItem.record_id。 */
  ui_message_id?: string
  /** 所属会话，用于 reader key 推导与错误归因。 */
  ui_session_id?: SessionId
  /** 展示用发送者名。取 MailboxRecord.from_name，缺失时回落到 DID。 */
  ui_sender_name?: string
  /** 出站消息的投递状态。由 SessionDeliveryOverall 映射。 */
  ui_delivery_status?: MessageDeliveryStatus
  /** 标记该条目应渲染为状态 pill 而非消息气泡。 */
  ui_item_kind?: 'status'
  /** 状态类型。 */
  ui_status_type?: ConversationStatusType
}

export type MessageDeliveryStatus =
  | 'sending' | 'sent' | 'delivered' | 'read' | 'failed'

export type ConversationStatusType =
  | 'typing' | 'processing' | 'disconnected' | 'info'
```

`SessionDeliveryOverall → MessageDeliveryStatus` 映射（`sessionApiReader.ts` 已实现）：

| 后端 | UI | 说明 |
|---|---|---|
| `sending` | `sending` | 存在 WAIT / SENDING 目标 |
| `delivered` | `delivered` | 全部目标 SENT |
| `partial_failed` | `failed` | 部分目标 DEAD/FAILED，UI 统一显示失败并可展开 per_target |
| `failed` | `failed` | 全部目标 DEAD |
| 无 `delivery` 字段 | `undefined` | 入站消息不显示投递图标 |

`read` 状态不来自 `delivery`，来自 `msg.list_read_receipts` 的 `ReadReceiptState`，当前原型未接入，为预留态。

#### 3.5.1 渲染器可识别的内容类型

渲染器按顺序尝试，第一个返回非空的胜出（`renderers.tsx`）：

| 渲染器 | 触发条件 | 消费字段 |
|---|---|---|
| `renderImageMessage` | `content.refs` 中存在 `target.type === 'data_obj'` 且 `uri_hint` 为可识别图片 URL | `refs[].target.uri_hint`、`refs[].label`、`content.content`（caption） |
| `renderTextMessage` | `content.format` ∈ `text/plain` / `text/markdown` / `text/html` | `content.content` |
| `renderFallbackMessage` | 其余 | `content.format`、`content.content` |
| 状态 pill | `kind === 'notify'` 或 `ui_item_kind === 'status'` | `ui_status_type`、`content.content` |

已知限制（原型态，需在集成阶段决策）：`text/markdown` 与 `text/html` 当前按纯文本显示，未做富文本渲染。

#### 3.5.2 摘要生成规则（`MessagePreview.text`）

Entity / Session 列表的摘要必须由消息对象派生，不得由后端另发一份文本：

| 消息形态 | 摘要 |
|---|---|
| 文本类 | `content.content` 首行，截断至 120 字符 |
| 图片引用 | i18n `messagehub.preview.image` + `refs[0].label`（若有） |
| 其他 format | i18n `messagehub.preview.attachment` + `content.format` |
| 状态消息 | 不参与摘要，跳过取上一条非状态消息 |
| 无 `last_record.msg` | i18n `messagehub.preview.unavailable` |

### 3.6 会话时间线投影模型

已由原型收敛，保持不变（`conversation/history/types.ts`）：

```ts
export interface ConversationMessageReader {
  readonly readerKey: string
  readonly totalCount: number
  readRange(startIndex: number, count: number): Promise<readonly MessageObject[]>
}

export interface AppendableConversationMessageReader extends ConversationMessageReader {
  append(message: MessageObject): AppendableConversationMessageReader
}

export type ConversationListIndexEntry =
  | { kind: 'message'; key: string; messageIndex: number }
  | { kind: 'timestamp'; key: string; dateMs: number; anchorMessageIndex: number }
  | { kind: 'status'; key: string; status: ConversationStatusType; label: string
      anchorMessageIndex?: number; createdAtMs?: number }

export type ConversationListItem =
  | { kind: 'message'; key: string; index: number; messageIndex: number; data: MessageObject }
  | { kind: 'timestamp'; key: string; index: number; date: Date }
  | { kind: 'status'; key: string; index: number; status: ConversationStatusType
      label: string; createdAtMs?: number }

export interface ConversationProjection {
  readonly readerKey: string
  readonly messageCount: number
  readonly tailStatusCount: number
  readonly statusItemsSignature: string
  readonly lastMessage?: MessageObject
  readonly totalCount: number
  readonly entries: readonly ConversationListIndexEntry[]
}

export interface ConversationMaterializedWindow {
  startIndex: number
  endIndex: number
  items: readonly ConversationListItem[]
}
```

关键点：**index 空间是「投影后条目」而非「消息」**。时间分隔符与状态 pill 都占据 index 位，
所以 `ConversationListItem.index` 与 `messageIndex` 必须分开保存——虚拟滚动定位用前者，读消息用后者。

Reader 的三种实现：

| 实现 | readerKey | 用途 |
|---|---|---|
| `InMemoryConversationMessageReader` | `memory:{sessionId}` | mock 数据、本地追加 |
| `IndexedDbConversationMessageReader` | `indexeddb:{db}:{ns}:{sessionId}` | 大历史本地缓存 |
| `SessionApiConversationMessageReader` | `msg-center:{sessionId}` | `msg.list_session` 真实数据 |

`readerKey` 变化即视为换会话，投影全量重建；`totalCount` 增长视为追加，投影增量扩展。

---

## 4. 输入模型与校验

展示模型与输入模型分离。以下四处是 UI 的用户输入点，均以 Zod schema 作为唯一事实来源，
`react-hook-form` 的字段类型从 schema 推导。

### 4.1 实体列表查询

```ts
export const entityFilterSchema = z.enum([
  'all', 'unread', 'pinned', 'people', 'agents', 'groups',
])

export const entityListQuerySchema = z.object({
  filter: entityFilterSchema.default('all'),
  /** 搜索词。空串表示不过滤；trim 后长度上限 64。 */
  searchQuery: z.string().trim().max(64).default(''),
})

export type EntityFilter = z.infer<typeof entityFilterSchema>
export type EntityListQuery = z.infer<typeof entityListQuerySchema>
```

匹配语义（`EntityList.tsx` 已实现）：

- `filter` 判定：`all` 恒真；`unread` → `unreadCount > 0`；`pinned` → `isPinned`；
  `people` / `agents` / `groups` → `type` 相等。
- `searchQuery` 判定：大小写不敏感，匹配 `name` 或 `lastMessage.text`。
- 二者是 **与** 关系。
- 过滤只作用于顶层实体，子实体不参与顶层过滤（避免父项被过滤掉后子项悬空）。

### 4.2 Composer 草稿

```ts
export const composerAttachmentSchema = z.object({
  id: z.string().min(1),
  file: z.instanceof(File),
  /** 目录拖拽 / 目录选择时的相对路径。 */
  relativePath: z.string().max(1024).optional(),
  kind: z.enum(['image', 'file']),
  /** 图片走 URL.createObjectURL，需在卸载时 revoke。 */
  previewUrl: z.string().optional(),
})

export const composerDraftSchema = z.object({
  content: z.string().max(32_768).default(''),
  attachments: z.array(composerAttachmentSchema).max(64).default([]),
}).refine(
  (draft) => draft.content.trim().length > 0 || draft.attachments.length > 0,
  { message: 'messagehub.composer.emptyDraft' },
)

export type ComposerAttachmentItem = z.infer<typeof composerAttachmentSchema>
export type ComposerDraft = z.infer<typeof composerDraftSchema>
export type ConversationComposerSubmitPayload = ComposerDraft
```

约束说明：

- `content` 上限 32 KiB，超出时禁用发送并显示计数提示，不做静默截断。
- `attachments` 上限 64 项；附件按 `relativePath || file.name` 归一化后去重。
- 空草稿（无文本且无附件）不可提交，错误文案走 i18n key。

#### 4.2.1 草稿 → MsgObject 的构造规则

**原型现状是把附件名拼成一行 mock 文本塞进 `content.content`，这不是目标模型。** 目标构造规则：

```ts
function buildOutgoingMessage(draft: ComposerDraft, ctx: OutgoingContext): MsgObject
```

| 草稿部分 | 目标位置 |
|---|---|
| `content`（trim 后） | `content.content`，`content.format = 'text/plain'` |
| 每个附件 | 先上传到 named_store 得到 `obj_id`，再追加一项 `content.refs[]`：`{ role: 'input', target: { type: 'data_obj', obj_id, uri_hint }, label: relativePath ?? file.name }` |
| 会话归属 | `thread.correlation_id = session.id`（保证后端 `derive_session_id` 回到同一 session） |
| 目标 | `to = [resolveSendTargetDid(entity, session)]`，群会话为 group DID，外部平台会话为 `endpointDid` |
| 幂等 | `msg.post_send` 的 `idempotency_key`，由 `{sessionId}:{clientNonce}` 生成，重发不产生重复消息 |

发送后的乐观更新：立即用 `ui_delivery_status: 'sending'` 追加到 reader，
`msg.post_send` 返回后按 `PostSendResult.ok` 改写为 `sent` 或 `failed`。

### 4.3 实体备注编辑（Panel C）

```ts
export const entityNotePatchSchema = z.object({
  note: z.string().trim().max(280).optional(),
  tags: z.array(z.string().trim().min(1).max(24)).max(16).default([]),
  accessLevel: z.enum(['block', 'stranger', 'temporary', 'friend']).optional(),
})

export type EntityNotePatch = z.infer<typeof entityNotePatchSchema>
```

映射到 `contact.update_contact` 的 `ContactPatch`。`domain === 'external'` 的实体只允许提交 `note` 与 `tags`；
`accessLevel` 字段在 external 实体的表单里不渲染。

### 4.4 会话本地状态（`ui_session` KV）

后端 KV 是无 schema 的 `{ session_id, key, value: Value }`，UI 侧必须自行约定并校验：

```ts
export const uiSessionStateSchema = z.object({
  'ui.title': z.string().trim().min(1).max(64).optional(),
  'ui.pinned': z.boolean().optional(),
  'ui.muted': z.boolean().optional(),
  'ui.tags': z.array(z.string().trim().min(1).max(24)).max(16).optional(),
  /** 未发送草稿文本。附件不持久化。 */
  'ui.draft': z.string().max(32_768).optional(),
  /** 客户端已读水位（sort_key）。用于跨端对齐未读分割线。 */
  'ui.last_read_sort_key': z.number().int().nonnegative().optional(),
})

export type UiSessionStateKey = keyof z.infer<typeof uiSessionStateSchema>
```

规则：

- 读取到无法通过 schema 的值时 **丢弃该 key 并按缺省渲染**，不阻塞整个会话加载。
- `ui.*` 前缀为 MessageHub 保留，其他消费方（Agent Runtime 等）不得写入。
- 会话重命名 = 写 `ui.title`；清空 = 删除该 key，回落到 3.3.1 的派生标题。

---

## 5. 状态模型

### 5.1 通用状态容器

```ts
export type LoadingState = 'idle' | 'loading' | 'success' | 'error'

export interface DataState<T> {
  status: LoadingState
  data: T | null
  error: string | null
  /** 分页加载中（首屏已出、正在追加）。与 status='loading' 区分。 */
  isLoadingMore?: boolean
  /** 是否还有下一页。 */
  hasMore?: boolean
}
```

### 5.2 各视图的五态

#### Panel A 实体列表 — `DataState<Entity[]>`

| 状态 | 触发 | 呈现 |
|---|---|---|
| 正常 | `status='success'` 且过滤结果非空 | 实体列表 |
| 空（无数据） | `status='success'` 且 `data.length === 0` | 引导文案 + 「发起会话」入口 |
| 空（过滤无结果） | 过滤后为空但 `data.length > 0` | `messagehub.noResults` + 清除过滤按钮 |
| 加载 | `status='loading'` | 8 条骨架行 |
| 错误 | `status='error'` | 错误文案 + 重试 |
| 进度 | `isLoadingMore` | 列表底部行内 spinner |

「空数据」与「过滤无结果」必须区分——原型当前只有后者。

#### Session 侧栏 — `DataState<EntitySession[]>`

| 状态 | 呈现 |
|---|---|
| 正常 | session 列表，当前项右侧高亮条 |
| 空 | 不可能出现：每个实体至少一个默认 Session（PRD §9.3）。若真发生，按错误处理并上报 |
| 加载 | 3 条骨架行 |
| 错误 | 行内错误条 + 重试，不阻塞 Conversation |
| 进度 | 无（session 数量有界，一次拉完） |

`sessionCount <= 1` 时整个侧栏入口隐藏。

#### Panel B 会话视图 — `ConversationProjection` + `DataState`

| 状态 | 触发 | 呈现 |
|---|---|---|
| 正常 | 投影非空 | 消息时间线 |
| 空 | `totalCount === 0` | 「开始对话」占位，Composer 保持可用 |
| 加载（首屏） | 投影未建立 | 消息区骨架 |
| 加载（窗口） | 窗口未覆盖当前视口 | `ListItemPlaceholder` 占位行，保持滚动高度稳定 |
| 加载（历史） | 上翻触顶 | 顶部 spinner，滚动位置锚定不跳 |
| 错误 | reader 抛错 | 全区错误态 + 重试 |
| 进度 | 出站消息 `ui_delivery_status = 'sending'` | 气泡内时钟图标 |
| 瞬时状态 | `kind === 'notify'` 条目 | 居中状态 pill（typing / processing / disconnected / info） |

未选中实体时 Panel B 显示 `EmptyConversation` 占位，这是**视图空态**而非数据空态。

#### Panel C 详情 — `DataState<EntityDetail>`

| 状态 | 呈现 |
|---|---|
| 正常 | 详情面板 |
| 空 | 实体存在但无 detail：仅渲染从 `Entity` 继承的字段，不显示错误 |
| 加载 | 面板骨架 |
| 错误 | 面板内错误条 + 重试，不影响 Conversation |
| 进度 | 备注 / 标签保存中：按钮 loading + 字段禁用 |

#### Composer

| 状态 | 呈现 |
|---|---|
| 正常 | 可输入可发送 |
| 空草稿 | 发送按钮禁用 |
| 发送中 | 按钮 loading，输入框保持可编辑（允许连续发送） |
| 发送失败 | 消息气泡显示失败图标 + 重试入口；草稿不回填 |
| 附件处理中 | 附件卡片显示进度；发送按钮在全部附件就绪前禁用 |
| 拖拽悬停 | 全区 drop overlay |

### 5.3 页面视图状态

`types.ts` 中现有的 `MessageHubState` 已落后于实现且无消费点，替换为：

```ts
export type MobileView = 'entity-list' | 'conversation' | 'details'

export interface MessageHubViewState {
  /* 选择态 */
  selectedEntityId: EntityId | null
  selectedSessionId: SessionId | null

  /* 查询态 */
  query: EntityListQuery

  /* 导航态 */
  mobileView: MobileView
  /** 实体列表的 drilldown 路径，元素为 EntityId。空数组表示在顶层。 */
  entityListDrilldownPath: EntityId[]
  /** inline 展开的实体集合。 */
  expandedEntityIds: ReadonlySet<EntityId>

  /* 面板可见性 */
  showSessionSidebar: boolean
  showDetails: boolean

  /* 布局态（仅桌面端有意义，应持久化到本地） */
  layout: MessageHubLayoutState
}

export interface MessageHubLayoutState {
  entityListWidth: number         // [280, 520]，默认 340
  sessionSidebarWidth: number     // [240, 520]，默认 280
  isEntityListCollapsed: boolean  // 折叠宽度 68
}
```

布局常量定义在 `layout.ts`，是 UI DataModel 的一部分（会被持久化），不是纯样式常量。

`isResizingEntityList` / `isResizingSessionSidebar` 属于拖拽过程中的瞬时交互状态，
**不进入** `MessageHubViewState`，留在组件内部 ref/state。

---

## 6. 分页与聚合

### 6.1 实体列表

- **数据来源**：`msg.list_sessions` 的全部页 + `contact.list_contacts` + `group.list_by_member`。
- **分页策略**：cursor。游标为 `(next_cursor_updated_at_ms, next_cursor_session_id)` 二元组，
  两者任一为 `undefined` 即为末页。
- **页大小**：50（当前实现为 200，偏大，接后端时下调）。
- **排序**：`lastActiveAt` 降序；`isPinned` 的实体恒置顶，置顶组内部同样按 `lastActiveAt` 降序。
- **聚合**：
  - `Entity.unreadCount = Σ session.unreadCount`
  - `Entity.lastActiveAt = max(session.lastActiveAt)`
  - `Entity.lastMessage` 取 `lastActiveAt` 最大的那个 session 的 `last_record`
  - `Entity.sessionCount = 该实体下 session 数`
  - drilldown 面板额外聚合 `Σ children.unreadCount`
- **已知缺陷**：`listAllSessions()` 当前会一次性跟随游标拉完所有 session。
  接后端时必须改为按需分页 + 滚动加载，否则大账号首屏会被阻塞。

### 6.2 会话时间线

- **数据来源**：`msg.list_session`。
- **分页策略**：cursor。游标为 `(next_cursor_sort_key, next_cursor_record_id)`。
- **方向**：首屏 `descending: true` 取最新 64 条；上翻历史继续 `descending: true` 带游标；
  渲染前反转为时间升序。
- **`with_object`**：恒为 `true`。UI 不接受只拿到 record 再逐条 `msg.get_message`。
  无 `msg` 的条目当前被 `sessionItemToMessageObject` 跳过并 `console.warn`——这是数据缺陷信号，
  集成阶段应改为渲染一条「消息不可用」占位而非静默丢弃。
- **已知缺陷**：`createSessionApiReader()` 当前 `descending: false` 从头拉完整个会话。
  长会话必须改为「最新一页 + 向上增量」。
- **窗口物化**：`DEFAULT_PAGE_SIZE = 32`，`INDEX_SCAN_PAGE_SIZE = 128`。
  投影索引一次性扫描建立，条目内容按可视窗口懒加载。
- **时间分隔符**：相邻消息间隔 `TIMESTAMP_GAP_MS = 30 min` 时插入一条 `timestamp` 条目。
- **状态条目**：`tail` 位置的状态 pill（typing / processing）不参与持久投影，
  由 `statusItemsSignature` 单独比对，避免每次状态变化触发全量重建。

### 6.3 Session 列表

一次性拉取，不分页。单实体 session 数预期 < 50。按 `lastActiveAt` 降序，置顶优先。

### 6.4 未读聚合口径

| 层级 | 口径 |
|---|---|
| Session | `SessionSummary.unread_count`，后端按 `RecipientState = UNREAD` 计数 |
| Entity | Σ 其下 session 的 unread_count |
| 全局 | Σ 所有实体，供 App 图标 badge |

静音（`isMuted`）**不减少**未读计数，只改变 badge 配色与提醒行为。这是原型已确立的口径，保持不变。

---

## 7. 字段稳定性分级

- **Frozen**：前后端共同依赖，变更是高影响事件。
- **Extensible**：可演进，新增取值不影响现有消费者。
- **Volatile**：实现细节 / 原型态，集成阶段可能变。

### Entity

| 字段 | 稳定性 | 说明 |
|---|---|---|
| `id` | Frozen | 规范化 DID，实体主键 |
| `type` | Extensible | 可能新增实体类型（如 `device`） |
| `domain` | Frozen | 驱动权限差异，只有两值 |
| `name` | Frozen | 核心展示字段 |
| `unreadCount` | Frozen | 聚合口径见 6.4 |
| `lastActiveAt` | Frozen | 默认排序键 |
| `lastMessage` | Extensible | `MessagePreview` 可能增加富摘要字段 |
| `lastActivitySessionId` | Frozen | 决定点击实体后打开哪个会话 |
| `sessionCount` | Frozen | 决定 Session 入口是否出现 |
| `tags` | Extensible | 系统标签 + 用户标签合并 |
| `sources` | Extensible | 平台标识开放集合 |
| `isPinned` / `isMuted` | Frozen | 落在 ui_session KV |
| `isOnline` / `statusText` | Volatile | presence 模型尚未定稿（PRD §18.6） |
| `children` / `childrenMode` / `childrenSections` / `drilldownDescription` | Volatile | 子实体来源与 drilldown 配置仍在演进 |
| `avatar` | Volatile | 渲染层尚未接入真实头像 |

### EntitySession

| 字段 | 稳定性 | 说明 |
|---|---|---|
| `id` | Frozen | 后端 session_id 原值 |
| `entityId` | Frozen | 归属关系 |
| `title` | Frozen | 永远非空 |
| `titleSource` | Extensible | 可能新增来源 |
| `kind` | Extensible | PRD §11 明确会扩展 Conversation Type |
| `unreadCount` / `lastActiveAt` | Frozen | |
| `source` | Extensible | 平台开放集合 |
| `lastDelivery` | Extensible | 映射自 `SessionDeliveryOverall` |
| `lastMessage` | Volatile | 当前 Session 列表未展示 |
| `isPinned` / `isMuted` | Extensible | |
| ~~`isActive`~~ | 废弃 | 由选择态判定，不再作为数据字段 |

### MessageObject / ui meta

| 字段 | 稳定性 | 说明 |
|---|---|---|
| `from` / `to` / `kind` / `created_at_ms` | Frozen | 协议字段 |
| `content.content` / `content.format` | Frozen | |
| `content.refs` | Frozen | 附件与图片的唯一承载位 |
| `thread.correlation_id` | Frozen | 出站消息的会话归属依据 |
| `ui_message_id` / `ui_session_id` | Frozen | |
| `ui_sender_name` | Extensible | |
| `ui_delivery_status` | Extensible | `read` 态待接 read_receipts |
| `ui_item_kind` / `ui_status_type` | Volatile | 状态消息规范未定稿（PRD §18.7） |
| `content.title` / `content.machine` | 未消费 | 协议已定义，UI 暂不读 |
| `workspace` / `expires_at_ms` / `nonce` / `proof` | 未消费 | |

### EntityDetail

| 字段 | 稳定性 |
|---|---|
| `bindings` | Frozen（含 `endpointDid`，回复外部平台必需） |
| `accessLevel` / `isVerified` | Frozen |
| `note` / `tags` | Frozen（可编辑） |
| `memberCount` | Extensible |
| `bio` / `contactSource` / `createdAt` | Extensible |

---

## 8. Mock 数据契约

`mock/data.ts` 需要覆盖以下场景，Playwright 才能跑完整流程。当前覆盖情况见「现状」列。

| 场景 | 目的 | 现状 |
|---|---|---|
| Agent 实体 + 3 个 session（chat / chat / task） | 多 Session 切换、Session 图标分化 | 已覆盖（`agent-coder`） |
| Person 实体 + 单 session | `sessionCount === 1` 时隐藏 Session 入口 | 已覆盖（`person-alice`） |
| Group 实体 + inline 子实体 | 原地展开（PRD §7.7） | 已覆盖（`group-team`） |
| Service 实体 + drilldown 子实体 + 分组 section | drilldown 面板与 `childrenSections` | 已覆盖（`service-release-hub`） |
| 零未读实体 | badge 不渲染 | 已覆盖（`person-bob`） |
| 静音实体 | 静音 badge 配色 | **缺失** |
| 外部平台绑定（telegram / email） | `AccountBinding` 与 external 域权限 | 部分（`person-alice` 有 bindings，缺 `endpointDid`） |
| 出站消息各投递态 | sending / sent / delivered / failed 图标 | 部分（缺 `failed`） |
| 状态消息 | typing / processing / disconnected / info pill | 已覆盖 |
| 图片引用消息 | `content.refs` 图片渲染 | **缺失** |
| 未知 format 消息 | fallback 渲染器 | **缺失** |
| 长会话（≥ 200 条） | 虚拟滚动、窗口物化、时间分隔符 | 已覆盖（`codeassistant/mockHistory`） |
| 空会话 | 空态占位 | **缺失** |
| 加载态 / 错误态 provider | 五态验证 | **缺失**（当前 mock 全是同步返回） |
| 实体数 ≥ 30 | 列表分页行为 | **缺失** |

补齐要求：mock provider 需支持 `delay(300~800ms)` 与可注入的失败开关，否则加载态与错误态无法被 Playwright 覆盖。

### 8.1 输入模型样例

`composerDraftSchema`：

| 类别 | 样例 | 预期 |
|---|---|---|
| 合法 | `{ content: 'ship it', attachments: [] }` | 通过 |
| 合法 | `{ content: '', attachments: [imageItem] }` | 通过（纯附件） |
| 非法 | `{ content: '   ', attachments: [] }` | `messagehub.composer.emptyDraft` |
| 非法 | `{ content: 'x'.repeat(32_769), attachments: [] }` | 长度上限错误 |
| 非法 | 65 个附件 | 数量上限错误 |
| 默认值 | `{}` | `{ content: '', attachments: [] }` 后被 refine 拒绝 |

`entityNotePatchSchema`：

| 类别 | 样例 | 预期 |
|---|---|---|
| 合法 | `{ note: '设计系统负责人', tags: ['work'] }` | 通过 |
| 非法 | `{ note: 'x'.repeat(281) }` | 长度上限错误 |
| 非法 | `{ tags: [''] }` | 标签不可为空串 |
| 编辑回填 | 从 `EntityDetail` 取 `{ note, tags, accessLevel }` | 表单 defaultValues |

---

## 9. KRPC 映射

服务名：`msg-center`。客户端封装：`datamodel/sessionApi.ts`（当前仅接了 3 个方法）。

### 9.1 读取路径

| UI DataModel | KRPC 方法 | 变换 |
|---|---|---|
| `Entity[]` | `msg.list_sessions` + `contact.list_contacts` + `group.list_by_member` | 三路合并，见 3.2 |
| `Entity.id` | `contact.resolve_canonical_did` | 别名 DID 归一 |
| `Entity.unreadCount` | `SessionSummary.unread_count` | Σ 聚合 |
| `Entity.lastMessage` | `SessionSummary.last_record.msg` | 见 3.5.2 摘要规则 |
| `Entity.lastActiveAt` | `SessionSummary.updated_at_ms` | max 聚合 |
| `Entity.children`（群） | `group.list_subgroups` | `GroupSubgroup` → `Entity` |
| `EntitySession[]` | `msg.list_sessions` | 按 3.2.1 分组 |
| `EntitySession.title` | `ui_session.get_state('ui.title')` / `msg.thread.topic` | 见 3.3.1 |
| `EntitySession.isPinned/isMuted` | `ui_session.get_state` | KV 反序列化 + schema 校验 |
| `MessageObject[]` | `msg.list_session`（`with_object: true`） | `sessionItemToMessageObject` |
| `ui_message_id` | `SessionMessageItem.record_id` | 直接 |
| `ui_delivery_status` | `SessionMessageItem.delivery.overall` | 枚举映射，见 3.5 |
| `ui_sender_name` | `MailboxRecord.from_name` | 缺失时回落 DID |
| `EntityDetail` | `contact.get_contact` / `group.get_doc` | `Contact` / `GroupDoc` → `EntityDetail` |
| `EntityDetail.memberCount` | `group.list_by_member` 的 `GroupSummary.member_count` | `group.get_doc` 不返回成员数 |
| `AccountBinding.endpointDid` | `Contact.bindings[].endpoint_did` | 直接 |
| `EntityDetail.accessLevel` | `Contact.access_level` | `SCREAMING`→`snake` 已由 serde 处理 |

### 9.2 写入路径

| UI 动作 | KRPC 方法 | 请求体 |
|---|---|---|
| 发送消息 | `msg.post_send` | `{ msg: MsgObject, idempotency_key }`，见 4.2.1 |
| 标记已读 | `msg.set_read_state` | 进入会话且窗口贴底时触发 |
| 单条状态变更（归档 / 删除） | `msg.update_record_state` | `RecipientState` |
| 会话重命名 / 置顶 / 静音 / 草稿 | `ui_session.update_state` | `{ session_id, key, value }`，key 见 4.4 |
| 编辑备注 / 标签 / 访问级别 | `contact.update_contact` | `ContactPatch` |
| 拉黑 | `contact.block_contact` | |
| 临时授权 | `contact.grant_temporary_access` | |
| 会话重新归类 | `msg.update_record_session` | 仅可信后端/Agent 使用，UI 暂不暴露 |

### 9.3 需要后端确认的点

1. **实体列表没有单一接口。** 目前需要 UI 端做三路合并 + N 次 `resolve_canonical_did`。
   是否值得在 `msg-center` 增加一个 `msg.list_entities` 投影接口，由后端完成归属推导？
   这直接决定首屏请求数是 O(1) 还是 O(sessions)。
2. **`SessionSummary` 缺 `last_delivery`。** 当前要拿到会话级投递失败，必须再调一次 `msg.list_session`。
   建议在 `SessionSummary` 上补一个聚合投递态。
3. **`ui_session` KV 无批量读取的按 owner 维度接口。** `ui_session.list_state` 是按 `session_id` 的，
   首屏 N 个会话就要 N 次调用。需要一个按 owner 批量拉取的形式。
4. **已读回执未接入。** `MessageDeliveryStatus.read` 需要 `msg.list_read_receipts`，
   拉取时机与频率未定。
5. **附件上传通道未定。** 4.2.1 假设「先写 named_store 拿 obj_id 再引用」，需与 content_mgr 对齐。
6. **presence 无来源。** `Entity.isOnline` / `statusText` 在后端没有任何字段支撑，
   对应 PRD §18.6 的 attention 模型待决项。

---

## 10. 与原型现状的差异清单

本文档相对 `MessageHub_Current_UI_Model_Data.md` 记录的实现现状，需要在集成阶段落地的改动：

| # | 改动 | 原因 |
|---|---|---|
| 1 | `Entity.id` 从 mock 短 id 改为规范化 DID | 后端一切以 DID 寻址 |
| 2 | `Entity.lastMessage` 由 session 摘要派生，不再独立 mock | 消除两套并行数据 |
| 3 | `Entity.lastActiveAt` 启用为排序键（当前未消费） | 后端按 `updated_at_ms` 排序 |
| 4 | `Entity.source: string` → `sources: string[]` | 一个实体可有多个平台绑定 |
| 5 | 新增 `Entity.domain` / `sessionCount` / `lastActivitySessionId` | PRD §12.4/§15.1 权限差异与默认会话选择 |
| 6 | `Session` 更名 `EntitySession`，删除 `isActive`，新增 `titleSource` / `lastDelivery` | `isActive` 无消费点；标题在后端不存在 |
| 7 | `EntityDetail.bindings` 改为必填，新增 `accessLevel` / `isVerified` / `contactSource` | 权限区块需要 |
| 8 | `AccountBinding` 新增 `endpointDid` / `accountType` | 回复外部平台消息的目标地址 |
| 9 | 附件从「拼进文本」改为写入 `content.refs` | 当前是纯 UI 演示态 |
| 10 | `MessageHubState` → `MessageHubViewState`，补齐 drilldown / 布局 / 展开态 | 旧接口无消费点且落后于实现 |
| 11 | `listAllSessions` / `createSessionApiReader` 改为按需分页 | 全量拉取在真实数据量下不可用 |
| 12 | 无 `msg` 的时间线条目由静默丢弃改为占位渲染 | 静默丢消息不可接受 |
| 13 | mock provider 增加延迟与失败注入 | 加载态 / 错误态当前无法被测试覆盖 |
| 14 | 实体列表区分「无数据」与「过滤无结果」两种空态 | 当前只有后者 |

---

## 11. 一句话总结

MessageHub 的 UI DataModel 是一个**三层投影模型**：协议层原样镜像 `msg-center` 的 `MsgObject` 与 session 投影；
UI DataModel 层把扁平的 `session_id` 空间还原成「实体 → 会话」两级结构，并把后端不存在的展示态属性
（标题、置顶、静音、草稿）统一收到 `ui_session` KV；组件层只消费 UI DataModel。
这一层投影规则（3.2.1 的归属推导、3.3.1 的标题派生、3.5 的 ui meta 契约）就是本文档的核心契约，
也是接后端时唯一需要严格对齐的部分。
