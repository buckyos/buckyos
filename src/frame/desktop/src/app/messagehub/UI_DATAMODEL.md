# MessageHub UI DataModel

- 文档版本：v0.6（2026-09-07：真实接入 — 会话登记 / 生命周期 / 授权 / 对象访问落地，§9.4 记录执行结果）
- 文档类型：UI DataModel 设计文档（WebUI Dev Loop 阶段三产物）
- 模块位置：`src/frame/desktop/src/app/messagehub`
- 上游文档：
  - PRD：`product/message_hub/MessageHub_Web_UI_PRD.md` v0.3
  - 状态与日志：`doc/message_hub/Session State and Action Log.md` v0.1（目标契约，待实现）
  - 原型现状盘点：`product/message_hub/MessageHub_Current_UI_Model_Data.md` v0.3
  - 后端数据模型：`src/kernel/buckyos-api/src/msg_center_client.rs`、`src/frame/msg_center/src/`
- 下游用途：`integrate-ui-datamodel-with-backend`

---

## 1. Overview

### 1.1 本文档解决的问题

MessageHub 原型已经收敛（Entity List / Conversation View / Details 三层结构 + 多 Session 切换 + 消息投影 + Composer 草稿），
原型通过 `mock/store.ts` 统一管理 `mock/data.ts` seed、持久差量与历史 reader；其模型与 `msg-center` 的真实数据模型之间存在三处结构性错位：

1. **后端 Session API 没有统一的 Entity 聚合模型。** `msg-center` 的会话读取面只有 `SessionSummary`（会话摘要）与 `SessionMessageItem`（会话时间线），
   两者都在 owner 范围内以 `session_id` 为键。UI 的「实体 → 会话」两级结构必须由投影得到。
2. **当前 Session API 没有共享标题、置顶、静音等完整模型。** 个人展示属性走 `ui_session.*`；
   共享标题与成员会话昵称属于新增的持久业务状态，不能用个人显示标题或无约束 KV 代替（§3.3.5）。
3. **消息活动时间与属性更新时间必须分开。** mock 已将摘要、历史追加和活动排序统一到 store；
   接后端时仍不能将 `SessionSummary.updated_at_ms` 直接映射为消息活动时间。状态与 Action Log 不推进活动时间。

本文档定义的就是这三层之间的稳定边界：**协议层 → UI DataModel 层 → 组件层**。

2026-09-07 源码 Review 补充：上述结构可以保留，但集成还必须对齐邮箱记录与回执、
提交结果与逐目标投递、请求箱、对象附件及 Session 生命周期。本文分别标明原型现状、现有 RPC
和待实现契约；本次只修改文档，验收条件见 §9.4，不表示 mock 或后端已经补齐。

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
  EntityList / SessionSidebar / ConversationView / ConversationHistoryPane / EntityDetails / SessionDetails / ConversationComposer
```

约束：

- 协议镜像必须与真实后端一致；纯展示提示走 `ui_*` meta 或 `ui_session` KV。
  owner、稳定连接绑定、创建策略与授权能力属于业务契约，不能伪装为客户端 KV 权限开关。
  本次字段与交互已在 mock 中落地；真实能力仍须补齐后端契约再更新协议镜像，缺口见 §9.3。
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
| Panel C 实体详情 | `EntityDetails` | `EntityDetail` + owner / entity 创建策略 |
| Panel C 会话详情 | `SessionDetails` | `Session` + `SessionAccess` + `SessionPreferences` |
| 创建 / 归档 / 删除 | `SessionDialogs` | Zod 输入模型 + WindowDialogProvider |

### 1.4a 2026-09-06 已落地原型（本节时间口径优先）

2026-09-07 起组件通过 `store/` 的 `MessageHubStore` 接口取数据：mock 模式仍是 `mock/store.ts`，
真实模式是 `api/store.ts`（msg-center RPC）。本节描述的时间口径两种实现一致；真实模式的字段来源见
[当前 UI Model Data §8](../../../../../../product/message_hub/MessageHub_Current_UI_Model_Data.md)。

`mock/store.ts` 是 mock 原型的唯一可变业务数据源，组件通过 `useSyncExternalStore` 订阅。
本文后续 `EntitySession`、权威状态引用和 KRPC DTO 仍包含集成目标；当前实现精确字段见
[当前 UI Model Data](../../../../../../product/message_hub/MessageHub_Current_UI_Model_Data.md)。

| 范畴 | 当前实现 |
|---|---|
| 引用 | Session 以 JSON 编码 `(ownerDid, sessionId)` 寻址；reader、文本 / 文件草稿和 UI 偏好再加 viewerDid |
| 身份 | 默认 mock 登录用户；明确 `mode=observe` + ownerDid 才能观察允许的 Agent，拒绝时不回退用户缓存 |
| 连接 | native / tunnel / unknown 稳定 binding；tunnel 区分实例、端点和远端上下文；删除不移除连接 |
| 生命周期 | `active` / `archived`；删除保留时间水位与最小来源信息，移除本地历史引用、文本 / 文件草稿与偏好 |
| 持久化 | `messagehub-prototype-v1` IndexedDB 保存小型 Session 元数据和 mock 消息差量；长 seed 沿用现有 IndexedDB reader |
| 临时状态 | 带 member DID 与过期时间的运行态、绑定版本对应的写入确认均仅存在内存，刷新 / 切换 owner 清除 |
| 详情目标 | `detailsTarget: 'entity' | 'session' | null`，会话详情跟随当前有效 Session；移动详情覆盖 Conversation，保持其挂载 |
| 创建输入 | `createSessionSchema`: entityId、trim 后最多 64 字符 title、显式连接选择；当前只创建 chat |
| 编辑输入 | `sharedStateSchema`（title / description 64 / 500 字符）、`memberStateSchema`（nickname 64 字符）、`presentationSchema`（title / pinned / muted） |
| 操作状态 | 初始化 loading / error / retry；创建、编辑、管理和发送具有 pending / error，成功提交后才发布 store 结果 |
| Action 过滤 | `SessionPreferences.showActions` 默认 true；只能在 projection 过滤，不删除原始消息，不改业务状态 |

`lastActiveAt` 的唯一语义是最后有效消息活动时间：

- 新建空会话使用创建时间，reader 长度为 0。
- 普通 chat / group_msg，以及有正文或 output 引用的 deliver 结果，按 `max(旧时间, 消息时间)` 推进。
- typing / processing / active / statusLine、共享 / 成员状态、个人标题、Action Log、已读与投递变化均不推进。
- 归档 / 恢复 / 删除、偏好开关和分钟 tick 不推进；只有新的有效普通消息可自动解除归档。
- 重放已处理的消息 ID 不重复算活动；删除水位之前的消息不能复活会话或历史。
- 列表时间、排序和 Entity 聚合都使用此字段。摘要自己的 timestamp 不能反推活动时间。
- 相对时间为 now / 刚刚、整数 m / h / d；未知为 `—`，未来时间按零时间差处理。完整本地时间置于 title。

Action 类别严格使用 `kind === 'event' && content.machine?.intent === 'buckyos.action_log'`。
渲染先验证 schema_version，再读取 action / actor / subject。未知动作或版本回落为摘要，不执行载荷。
projection 保留 raw `messageIndex` 与 `messageCount`，重新生成可见 entries、日期和 totalCount；
全过滤显示“当前消息已被过滤”。过滤偏好按 viewer / owner / session 持久化，观察者只能修改自己的此项偏好。

原型开发环境通过 `window.__messageHubMock` 提供延迟 / 失败、时钟、runtime、共享 / 成员状态、
普通消息、投递、连接发现与连接失效事件注入。该入口只在 Vite DEV 暴露，不进入产品操作界面。
这些动作不是 KRPC，不代表真实服务完成了归档、物理删除、代理授权或远端线程创建。

### 1.4 产品不变量与术语

1. 真实 Entity 均以 DID 标识，可作为 `MsgObject.from: DID` 或 `to: DID[]`。
   子实体有自己的 DID；父子关系不合并消息、Session 或权限。仅表示上下文的 topic 是 Session，
   只有可独立寻址的 topic 才是子实体。
2. 实体间每条持久消息都必须有会话归属；准确地说，每个 owner 的本地消息记录归入一个 Session。
   同一消息可在不同 owner 视角下有不同 `session_id`，Session 的引用键是 `(ownerDid, sessionId)`。
3. 同一 owner 与同一对端可以有多个 Session。每条到该实体的 tunnel 连接至少对应一个独立 Session；
   同一连接支持多 Session 时再细分远端 thread / topic。不同实例、不同端点不能因联系人合并或 topic 同名而串会话。
4. 隧道会话通常默认只读；满足发送能力与授权后，显式确认外部软件历史风险才能启用当前 Session 写入。
5. 默认仅与 Agent 的 Session 可手工创建；其它由建立通信连接或发现远端上下文自动添加。
   “允许手工创建到该实体的会话”是 `(ownerDid, entityId)` 级配置，与发送权限分开。
6. 默认 owner 是登录用户；从 Agent 主页观察其会话时，owner 是 Agent，实际 viewer 仍是登录用户。
   首期 Agent 视角全部只读，包括已读状态与配置；未来代 Agent 写入需要独立授权。

本文的 tunnel 指后端外部平台 MessageTunnel。产品中的“通信连接”也包括原生 DID 通信；
后端同名的 MessageHub native transport 不是外部 tunnel。原生连接建立后也自动产生默认 Session，
但不套用外部软件历史风险确认。详见 PRD §6.7–§6.8、§9。

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

### 2.1 `session_id` 的当前后端生成规则

`msg_center.rs::derive_session_id` 的优先级：

1. `msg.thread.topic`
2. `msg.thread.correlation_id` / `meta.session_id` / `meta.owner_session_id` / 若干 payload 指针
3. 群消息：`msg.to[0]`（即 group DID）
4. 直接消息：`SENT` 箱用 `dm:{msg.to[0]}`，收件箱用 `dm:{msg.from}`

这是当前实现的推导规则，不是完整的目标会话身份契约。`thread.topic` 优先于其它上下文，
不同 tunnel 的同名 topic 可能碰撞；`MailboxRecord.session_id` 目前仍可为空。
目标要求后端保存稳定的 owner / 对端 / 连接 / 远端上下文绑定（§3.3.2），不得依赖标题或最近一条消息决定路由。
在绑定接口补齐前，§3.2.1 只作展示归类，不能据此确认空会话身份、开启写入或合并 Session。

### 2.2 后端未提供、必须由 UI 侧存储的属性

`MailboxRecord` 与 `SessionSummary` 都没有以下字段，UI 需要的它们只能落在 `ui_session.update_state` /
`ui_session.get_state` / `ui_session.list_state`（`UiSessionStateEntry { session_id, key, value, updated_at_ms }`）：

- 会话个人显示标题（仅当前用户的展示覆盖，不是共享标题）
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

export interface SessionRef {
  ownerDid: DID
  sessionId: SessionId
}

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

Entity 是 UI 聚合模型，由以下数据合并派生；新增会话登记 / 能力数据是待实现契约，不能从三路现有接口完整推断：

```text
contact.list_contacts   → Contact[]        身份、备注、绑定、访问级别
group.list_by_member    → GroupSummary[]   群名称、成员数、可发消息
msg.list_sessions       → SessionSummary[] 活跃度、未读、最近一条消息
会话登记 / 连接能力       → 待实现契约       空会话、稳定绑定、创建与发送能力
```

```ts
export interface Entity {
  /** 规范化 DID。经 contact.resolve_canonical_did 处理，别名 DID 不会产生重复实体。 */
  id: EntityId
  type: EntityType
  domain: EntityDomain
  /** 当前 owner 到该实体的手工创建策略与有效能力，见 3.3.3。 */
  sessionCreation: EntitySessionCreation
  /** 展示名。优先级：Contact.name → GroupSummary.name → MailboxRecord.from_name → DID 短写。 */
  name: string
  /** 头像 URL。当前渲染层仍按 type 生成图标，此字段为预留位。 */
  avatar?: string

  /** 单行状态说明，如 "online" / "12 members" / "last seen 2h ago"。由 presence 与实体类型派生。 */
  statusText?: string
  /** 在线态。仅 managed 实体可信；external 实体恒为 undefined。 */
  isOnline?: boolean

  /** 置顶。UI 侧属性，落在当前 owner 下实体主 session 的 ui_session KV；无 Session 时缺省 false。 */
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
  /** 最近活动的 session，点击实体时默认打开它；空会话允许没有 lastMessage。 */
  lastActivitySessionId?: SessionId

  /** 该实体下可见 session 数。等于 1 时 ConversationView 隐藏 Session 入口。 */
  sessionCount: number

  /** 子实体（PRD §6.2）。具有独立 DID 的 subgroup / topic、服务下的房间。 */
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

`resolveEntityDid(summary: SessionSummary, ownerDid: DID): EntityId | null`：

目标优先使用后端会话登记的对端 / 群 DID，再通过当前 owner 的联系人关系归一展示身份；
这不改变原始消息 DID、Session ID 或绑定的发送目标。当前没有登记时才使用下表：

| `session_id` 形态 | 后端来源 | 实体 DID |
|---|---|---|
| `dm:<did>` | `derive_session_id` 分支 4 | `contact.resolve_canonical_did(<did>)` |
| 可在群集合中命中的 DID | 分支 3 | 该 group DID |
| 其他任意字符串（topic / correlation_id） | 分支 1、2 | 群消息从 `last_record.msg.to` 识别群目标；普通单目标消息从原始 `msg.from/to` 识别相对 owner 的对端，再 `resolve_canonical_did`；对象或归属证据不足时进入未归类分组 |
| 无 `last_record` 且不匹配上述任一形态 | — | 归入 `unassigned` 桶，见下 |

无法归属的 Session 不得丢弃：令 `entityId = null`，在实体列表旁的“未归类会话”分组呈现。
该分组是 UI 容器，不是 Entity，不分配伪 DID，不可作为发送目标；其会话仍可按 owner 与 Session ID 只读打开。
原 v0.1 的 `urn:buckyos:messagehub:unassigned` 不再冒充可寻址实体。
多目标消息无法唯一确定对端 / 群时也进入该分组，不能随意取一个收件人。

`MailboxRecord.to` 是本地投递引用：所有入站箱均写 owner，`SENT` 只保存原始 `msg.to` 的第一项。
因此它不能恢复完整收件人集合；群消息复制到成员 `INBOX` 后也不能用它识别群。
当前后端还会通过 `group:<did>` tag 标注成员副本，但原始消息与标签只能提供展示归属证据，
不能代替稳定发送绑定。验收须覆盖同一 topic 会话最后一条消息由出站变入站，实体归属保持不变。

#### 3.2.2 子实体来源

| 子实体场景 | 后端来源 | childrenMode |
|---|---|---|
| 群下具有独立 DID 的 subgroup / topic | `group.list_subgroups` | `inline` |
| 聚合型服务实体（如 Release Hub）下的房间 / agent / 系统 | 前端配置 + `childrenSections` | `drilldown` |

子实体 **不是** Session（PRD §9.4）：子实体持久存在于实体层级，Session 是实体下的上下文容器。
父项自身会话与子项会话分别归属；子项即使在父项下展示，也只在未读全局聚合中计数一次。

### 3.3 EntitySession

```ts
export type SessionKind = 'chat' | 'task' | 'workspace'

export interface EntitySession {
  /** 后端 session_id 原值。 */
  id: SessionId
  /** 会话所属身份；与 id 组成完整引用键。 */
  ownerDid: DID
  /** 当前 owner 视角下的对端 / 群；null 表示未归类。 */
  entityId: EntityId | null
  /** 稳定连接绑定，不能用 source 或最近一条消息替代。 */
  binding: SessionBinding
  /** 会话创建来源，与 kind 和 source 分开。 */
  origin: 'manual' | 'connection' | 'remote_context' | 'unknown'
  /** owner 本地会话生命周期；不使用 RecipientState 表达。后端待实现。 */
  lifecycle: 'active' | 'archived'
  createdAt: number
  /** 当前 viewer / owner 视角下的有效访问能力，见 3.3.4。 */
  access: SessionAccess

  /** 展示标题，来源见 3.3.1。永远非空。 */
  title: string
  /** 标题来源，决定回落标题与来源图标；重命名能力由 access.canEditPresentation 决定。 */
  titleSource: SessionTitleSource

  /** 会话类型，决定 Conversation View 的渲染形态（PRD §6.4 / §11）。当前仅 chat 有完整实现。 */
  kind: SessionKind
  /** 协议来源，如 'buckyos' / 'telegram' / 'linear'。驱动 SessionSidebar 的前导图标与配色。 */
  source?: string

  unreadCount: number
  /** 最后有效消息活动时间；空会话初始化为创建时间。不得直接映射 SessionSummary.updated_at_ms。 */
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
  /** 来自权威 SessionSharedState.title */
  | 'shared'
  /** 来自 msg.thread.topic */
  | 'topic'
  /** 来自协议来源，如 "Telegram" */
  | 'platform'
  /** dm: 前缀会话的默认标题 */
  | 'direct'
  /** 群会话默认标题（等同群名） */
  | 'group'
  /** 无消息且无来源标题时的本地化占位标题 */
  | 'fallback'
```

#### 3.3.1 title 派生优先级

1. `ui_session.get_state(session_id, 'ui.title')` → `titleSource: 'user'`
2. 权威 `SessionSharedState.title` → `'shared'`
3. `last_record.msg.thread.topic` → `'topic'`
4. `session_id` 形如 `dm:*` → i18n `messagehub.session.direct`（"Direct Message"）→ `'direct'`
5. 群 session → 群名 → `'group'`
6. 有来源平台 → 平台展示名 → `'platform'`
7. 其余（含新建空会话）→ i18n `messagehub.session.untitled`（“未命名会话”）→ `'fallback'`

`ui.title` 只改本人的显示，不产生共享日志。修改共享标题必须调用权威状态修改能力，
产生 `session.title_changed` Action Log；不能改写 `thread.topic` 或历史消息标题来模拟状态更新。

**原型中的 `isActive` 字段废弃。** 活跃态由 `MessageHubViewState.selectedSessionId` 单独判定，
不再作为数据字段下发——原型里它已经没有任何消费点。

#### 3.3.2 Session 与连接绑定（目标契约，后端待补齐）

```ts
export type SessionBinding =
  | { kind: 'native'; targetDid: DID }
  | { kind: 'tunnel'; tunnelInstanceId: string; endpointDid: DID
      remoteContextId?: string; supportsMultipleSessions: boolean
      canCreateRemoteSession: boolean }
  | { kind: 'unknown' }
```

- 连接范围是 `(ownerDid, 对端端点, tunnelInstanceId)`，不是平台名称；实例可服务多个实体。
  `entityId` 是聚合展示身份，`endpointDid` 是此 Session 确定的外部发送目标；群会话指向群端点。
- 多 Session 连接再用稳定的远端上下文 ID 区分。同名标题 / topic 不能跨连接合并；
  `remoteContextId` 缺省仅表示该连接的默认上下文，未知映射必须标成 `unknown`。
- 联系人合并只改变实体展示归类，不重写 Session 绑定。新消息、出入站方向变化和重启均不得改变绑定。
- 建立连接即登记默认空 Session；远端上下文重复发现需幂等。手工创建也须先持久登记，
  第一条消息到达后沿用同一 Session ID。不能通过虚构消息或仅写 `ui.title` 来冒充会话创建。
- 连接移除不删除历史；禁止发送及自动换路。`source` / `AccountBinding` 仅供展示或显式选择，
  无法证明当前 Session 的稳定绑定时保持只读。

#### 3.3.3 实体级手工创建策略（目标契约）

```ts
export interface EntitySessionCreation {
  policy: 'default' | 'allow' | 'deny'
  canCreate: boolean
  unavailableReason?: string
}
```

策略按 `(ownerDid, entityId)` 保存，`default` 对 Agent 为允许、其它实体为禁止。
`canCreate` 是策略、当前视角、后端授权与所选连接能力共同计算的结果；多连接时在提交前明确选定连接，
并重新校验对应能力。在 tunnel 中新建还要求 `supportsMultipleSessions && canCreateRemoteSession`。
原生 Agent 会话不要求外部 tunnel 存在。配置允许不能绕过平台限制，不能解除已有 Session 的只读模式。
Agent 观察视角中 `canCreate = false`，也不能编辑此策略。

#### 3.3.4 会话读写能力

```ts
export interface SessionAccess {
  canRead: boolean
  /** 服务端授权与连接发送能力；不含页面内的 tunnel 风险确认。 */
  canSend: boolean
  /** 当前 owner 本地会话归档、恢复、删除能力。 */
  canManage: boolean
  mode: 'read_only' | 'read_write'
  canEnableWrite: boolean
  canEditPresentation: boolean
  canEditSharedState: boolean
  canEditOwnMemberState: boolean
  readOnlyReason?: 'agent_observer' | 'tunnel_default' | 'permission_denied'
    | 'transport_unavailable' | 'platform_read_only' | 'binding_unknown'
}
```

以上字段是有效能力投影，不是 UI 可随意写回的权限开关：

| 场景 | mode | canEnableWrite |
|---|---|---|
| 用户自己的原生 Session，目标明确且有发送权限 | `read_write` | `false` |
| tunnel 默认观察，绑定、出站能力与权限均满足 | `read_only` | `true` |
| tunnel 写入风险已确认，能力仍有效 | `read_write` | `false` |
| Agent 观察、权限不足、平台只读、连接失效或绑定未知 | `read_only` | `false` |

启用 tunnel 写入前提示：“从 MessageHub 写入可能造成另一个软件中的会话历史记录错误或不一致。”
取消不改变状态；确认仅对当前 `(viewerDid, ownerDid, sessionId)` 生效，并持续显示来源与写入状态。
首期确认只存在页面内存中，刷新、切换 owner 或绑定变化时清除；用户可恢复只读。
发送时重新校验能力。`canEditPresentation` 独立判断；用户自己的 tunnel 会话只读并不禁止本地重命名，
Agent 观察则连展示配置、已读、草稿都不能写入。后续代 Agent 发送另行定义授权，不复用 tunnel 风险确认。

`canEditSharedState` / `canEditOwnMemberState` 是独立的状态编辑能力，不由 Composer 模式推断；
它们只表示有可编辑字段，具体 patch 仍按字段权限校验。Agent 观察及能力未知时均为 false。

以上是目标字段，当前 mock `SessionAccess` 尚无 `canRead/canSend`，且自己的 native Session
会默认可写、可编辑共享和成员状态；该规则不能沿用到真实接入。native 也可能没有路由或没有群发言权限。
群能力可复用 `group.check_access` 的对应 action；`GroupSummary.can_message` 用于可通信性展示，
不能代替当前 actor 的操作授权。`is_hosted_by_self`、`Entity.domain` 和持有本地历史均不授予管理权。
原生目标无路由 / 路由变化、群只读、字段权限未知都须呈现具体原因；发送服务仍在提交时校验。
`canRead=false` 显示拒绝态；Agent 观察可经授权读取，但以上所有写入能力均为 false。

#### 3.3.5 整体状态与成员状态（后端目标；原型已模拟）

完整 schema、权限与持久化规则统一定义在
[Session State 与 Action Log](../../../../../../doc/message_hub/Session%20State%20and%20Action%20Log.md)。

- `SessionSharedState`：逻辑会话的共享标题 / 说明等，由有权限者修改。
- `SessionMemberState`：成员 DID 在指定会话中的昵称等，默认每位成员可改自己的允许字段。
  临时昵称只限本会话生效，但仍持久保存；不改全局实体名，不授予修改成员资格 / 角色的能力。
- 本地 `(ownerDid, sessionId)` 通过登记关联权威 `SessionStateRef`；同一逻辑会话的不同 owner
  视角读取同一份已确认状态，不能各自生成一份互相冲突的“共享状态”。
- `SessionRuntimeState`：typing / active / status_line 等可过期状态，与上述持久状态分开。
  Telegram 已有运行态同步，不表示共享 / 成员状态契约已经实现。
- `ui.*`：个人展示偏好。共享标题修改、成员昵称修改、个人显示标题修改是三种不同操作。

快照携带 revision，修改携带 expected_revision 与幂等键；只提交真实生效的变更并记录日志。
本节不要求添加组件、表单或原型交互，也不将目标类型提前写进当前协议镜像。

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

内容渲染继续消费完整 `MsgObject`，保留未知格式的原始载荷。同时，投影必须保留该 owner 的记录上下文，
不能把 `SessionMessageItem` 压缩成正文和一个投递图标。内容对象、邮箱记录、投递 / 回执有各自的标识与状态。
目标在本地 UI meta 中附带记录上下文，不改 Rust 协议；以下扩展尚未在 TS 实现中落地：

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
  /** 本地记录上下文；只存在展示副本，不能写回 MsgObject 或参与内容寻址。 */
  ui_record?: {
    ownerDid: DID
    recordId: string
    msgId: ObjId
    sessionId: SessionId
    direction: SessionMessageDirection
    boxKind: MailboxKind
    sortKey: number
    recipientState?: RecipientState
    delivery?: SessionDeliveryView
  }
  /** 展示用发送者名。取 MailboxRecord.from_name，缺失时回落到 DID。 */
  ui_sender_name?: string
  /** 便捷展示提示；完整投递事实保存在 ui_record.delivery。 */
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

`recordId` 用于更新阅读状态与定位本地条目，`msgId` 用于对象引用、回执查询及提交后的对账，
`direction` 用于 owner 视角的收发方向，不能仅用 `msg.from === owner` 推断；群成员副本尤其如此。
回执另按消息、群与 reader 身份保存，不覆写 recipientState 或 delivery。viewer 作用域由 reader / store context 提供。
`ui_sender_name` 仅在有可信名称时补充；当前 `SessionMessageItem` 没有 `from_name`，可从联系人投影或 DID 回落。
所有 `ui_*` 都是派生展示信息，发送时从输入与确定绑定构造新对象，不转发带 UI meta 的展示副本。

`SessionDeliveryOverall → MessageDeliveryStatus` 映射：

| 后端 | UI | 说明 |
|---|---|---|
| `sending` | `sending` | 存在 WAIT / SENDING 目标 |
| `delivered` | `delivered` | 全部目标 SENT |
| `partial_failed` | `failed`（当前图标） | 必须标明部分失败并可展开 per_target，不能丢掉已成功目标 |
| `failed` | `failed` | 无待投递 / 已成功目标，其余目标均为 FAILED/DEAD，以实际聚合实现为准 |
| 无 `delivery` 字段 | `undefined` | 入站消息不显示投递图标 |

当前 `sessionApiReader.ts` 只实现图标转换，丢弃了完整 delivery 与记录上下文；详情展开尚未实现。
投递详情须保留 `target_did/state/attempts/external_msg_id/last_error`，展示失败原因、可重试性及
`duplicate_risk`；不要从 `SENT` mailbox 或 RPC 成功推断已送达。
`read` 只来自明确 reader 的 `READED` 回执，不能把 READING、ACCEPTED 或 delivery.delivered 当成已读。
群聊展示各 reader 的回执，不把一个成员已读聚合成全员已读。当前回执服务必须传 `group_id`，
且仅存内存，尚无完整的私聊回执与重启恢复契约；这些场景保持未知，见 §6.5。

#### 3.5.1 渲染器可识别的内容类型

渲染器按顺序尝试，第一个返回非空的胜出（`renderers.tsx`）：

| 渲染器 | 触发条件 | 消费字段 |
|---|---|---|
| `renderImageMessage` | `content.refs` 中存在 `target.type === 'data_obj'` 且 `uri_hint` 为可识别图片 URL | `refs[].target.uri_hint`、`refs[].label`、`content.content`（caption） |
| `renderTextMessage` | `content.format` ∈ `text/plain` / `text/markdown` / `text/html` | `content.content` |
| `renderFallbackMessage` | 其余 | `content.format`、`content.content` |
| 状态 pill（原型现状） | `kind === 'notify'` 或 `ui_item_kind === 'status'` | 仅保留 status/content；真实持久通知的完整投影需修正，见下 |

已知限制：`text/markdown` 与 `text/html` 当前按纯文本显示；图片仅识别 HTTP(S) 图片 URL，
通用 fallback 只显示 format / content，没有通用附件入口。Telegram 实际附件已保存为 FileObject，
通过 `refs[].target.obj_id` 与 `cyfs://<obj_id>` 引用，因此当前渲染不能访问这些真实附件。

集成目标：

- 所有 `data_obj` 引用按 `obj_id` 经有权限的对象访问通道解析，`uri_hint` 只作提示；缺少 hint 也须可解析。
- 图片按解析后的 MIME / 对象元数据预览，文件提供名称、类型及下载入口；音视频不能预览时保留下载入口。
  不要求 URL 带文件扩展名，也不直接将 `cyfs://` 塞入浏览器 img src。
- 显示加载、不可用、无权限与重试状态；单个附件失败不隐藏正文或其它引用。
- refs 的 input/output/context 等角色保持原值；`service_did` 引用保留可识别的信息，不能伪装成文件。
- 对象解析与上传使用既有 named_store / content_mgr 能力，具体浏览器访问接口见 §9.3；未接通前不能把文件名摘要当成上传成功。

`kind=notify` 也不能一概作为 typing 等易失运行态。当前原型将它投影为只剩正文的状态 pill；
集成时只有明确的运行态来源进入独立状态通道，持久通知须保留记录上下文、内容与引用。

#### 3.5.2 摘要生成规则（`MessagePreview.text`）

Entity / Session 列表的摘要必须由消息对象派生，不得由后端另发一份文本：

| 消息形态 | 摘要 |
|---|---|
| 文本类 | `content.content` 首行，截断至 120 字符 |
| 图片引用 | i18n `messagehub.preview.image` + `refs[0].label`（若有） |
| 其他 format | i18n `messagehub.preview.attachment` + `content.format` |
| 易失状态消息（typing / processing 等） | 不参与摘要，跳过取上一条非状态消息 |
| Action Log（持久 event） | 使用其可读摘要，不能按易失状态丢弃 |
| 无 `last_record.msg` | i18n `messagehub.preview.unavailable` |

#### 3.5.3 Action Log Message（目标数据契约）

Action Message（本文亦称 Action Log Message）是 MessageObject 的可识别业务子类型。
Session / 实体状态变更复用标准已有的 `MsgObject.kind = 'event'` 与
`content.machine.intent = 'buckyos.action_log'`，结构化字段见状态与日志主文档 §4。
`event` 与 `machine.intent/data` 已存在，`buckyos.action_log` 判别值及 payload 是本设计的新增约定；
标准没有独立 Action 枚举；原型已在 `sessionModel.ts` 与 `renderers.tsx` 实现判别、专用渲染及未知版本回落。
必须区分目标 Session / 实体、操作者 actor、受影响成员 subject，并保留事件 ID、变更值与可信来源。
例如共享标题更新、会话昵称更新、加入群、主动退群、被管理员移除分别有明确 action。

Action Log 是已确认变化的历史记录，按普通持久消息进入 Session API；
不映射成 `ui_item_kind='status'`，不使用 `ConversationStatusType` 代替业务动作。
原型由 mock 生成日志；后端 `GroupMgr` 的事件目前存于 `group_events`，尚未发布成统一 Action Log 消息。
普通消息提交或历史重放不触发状态写入；更新状态与发布日志由权威服务负责。

后续特殊展示与过滤都以 `kind === 'event' && content.machine?.intent === 'buckyos.action_log'`
作为类别判断，再校验 data 并按其中 action 选择呈现。不要匹配“加入群聊”等文本，也不能把所有 event 都当成 Action。
专用 renderer 位于通用文本 / 图片之前，未知动作保留摘要回落。

“显示 / 隐藏 Action Message”是纯 UI 偏好：在可见投影生成前筛选，不删除 reader 中的原始消息，
不改变后端历史、权威状态、未读数或日志发布。开关本身不触发标已读。
切换过滤时重建可见 entries、时间分隔与 totalCount，原始 messageIndex 与消息稳定 ID 保持不变。
不能只在 renderer 中返回 null，否则当前链会继续调用文本 fallback，或留下虚拟滚动空行。
分类、过滤及 mock 事务已落地；真实状态提交、日志持久发布与 GroupEvent / 平台事件映射仍待实现。

### 3.6 会话时间线投影模型

当前原型接口如下（`conversation/history/types.ts`）；真实接入需补 §6.6 的记录更新与分页能力：

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
  readonly showActions: boolean
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
| `InMemoryConversationMessageReader` | `memory:{viewerDid}:{ownerDid}:{sessionId}` | mock 数据、本地追加 |
| `IndexedDbConversationMessageReader` | `indexeddb:{db}:{ns}:{viewerDid}:{ownerDid}:{sessionId}` | 大历史本地缓存 |
| `SessionApiConversationMessageReader` | `msg-center:{viewerDid}:{ownerDid}:{sessionId}` | `msg.list_session` 真实数据 |

表中是目标 key。当前 mock store 的 reader 已按 viewer / owner / Session 隔离，
旧 `SessionApiConversationMessageReader` 仍仅使用 `msg-center:{sessionId}`。各部分需无歧义编码，不能直接按 DID 中的冒号切分。

当前原型以 `readerKey` 变化触发重建、`totalCount` 增长触发追加。真实接入还必须通知同数量记录的状态更新、
删除和重新归类；仅有 append / totalCount 不能覆盖这些变化，扩展要求见 §6.6。

---

## 4. 输入模型与校验

展示模型与输入模型分离。以下四处是已有输入契约，以 Zod schema 作为校验事实来源，
`react-hook-form` 的字段类型从 schema 推导。新增创建与策略输入按 §3.3.3 的规则校验，
具体请求 schema 随 §9.3 的后端契约一起落地。

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
| 会话归属 | 当前原生无 topic 路径使用 `thread.correlation_id = session.id`；目标连接 / 远端上下文映射见下 |
| 发送者 | 首期仅自己的可写 Session：`from = context.sessionOwnerDid = context.viewerDid`；Agent 观察模式禁止构造出站消息 |
| 目标 | `to` 使用当前 Session 的稳定 `binding.targetDid` / `binding.endpointDid`；群指向群 DID，不能自动选择联系人其它绑定 |
| 幂等 | `msg.post_send` 的 `idempotency_key`，由 `{ownerDid}:{sessionId}:{clientNonce}` 无歧义编码生成，重发不产生重复消息 |

上表会话归属仅适用于当前原生无 topic 的构造路径：`thread.topic` 优先级更高，不能复制一个展示标题后
声称 `correlation_id` 保证归属。目标后端须将选中的本地 Session 与远端上下文稳定映射，
尤其 tunnel 回复不能把本地 Session ID 当成外部 thread ID；映射未明确时不开放发送（§9.3）。
提交前检查 `session.access.mode === 'read_write'`、当前身份与 Session owner 一致、绑定仍有效；
键盘快捷键、附件上传、失败重试与普通发送使用同一能力门禁。

发送后的乐观更新：立即用 `ui_delivery_status: 'sending'` 追加到 reader，
必须保留完整 `PostSendResult { ok, msg_id, deliveries, reason? }`。当前 `postSendMessage()` 返回 void 并忽略结果，
接入时须修正；RPC 正常返回仍可能是 `ok:false`，例如目标 tunnel 不存在或作者被拒绝。

- `ok:false`：显示提交失败与 reason，不伪造后端 SENT 记录；失败草稿 / 本地失败项可恢复。
- `ok:true`：只表示消息进入发送历史与投递队列。可显示“已提交”（现有 `sent` 提示），
  随后以 `SessionMessageItem.delivery` 更新发送中、部分失败、送达等状态。
- 保留 clientNonce、幂等键、原始 MsgObject、返回 msg_id 及 deliveries；用 owner + msg_id + 出站方向
  对账服务端记录，以 record_id 替换乐观项，避免提交回显产生双消息。
- 提交超时 / 结果未知时保持原始对象和幂等键重试，不重建 created_at_ms / nonce。
  后端自动投递重试由 executor 管理；复用 post_send 的幂等键不会重新启动 DEAD 任务。
  按目标人工重投目前没有对应 UI RPC，协议补齐前不提供会造成整条消息重复投递的通用“重发”。
- `cyfs-cached` 只表示网关暂存、仍待接收确认；保留该状态提示，不标记 delivered/read。

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
  /** 仅当前用户的个人显示标题覆盖。 */
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
- 修改个人显示标题 = 写 `ui.title`；清空 = 删除该 key，回落到 3.3.1 的派生标题，不发布 Action Log。
- 共享标题与成员昵称不能写入此 KV；它们使用 §3.3.5 的持久状态及授权修改契约。
- 当前 KV 只按 `session_id` 寻址，尚不满足多 owner 隔离。目标至少按 `(ownerDid, sessionId, key)`
  保存与授权；浏览器草稿 / 选择态 / 缓存另按 viewer 隔离。接口补齐前不能向其中写 Agent 的会话状态。
- `binding`、实体创建策略、授权能力不是此 KV 的展示键；tunnel 风险确认按 §3.3.4 仅保存在页面内存。

### 4.5 owner 本地 Session 生命周期（2026-09-07 已实现：`owner_sessions`）

沿用已落地 mock 的产品语义，生命周期独立于 `RecipientState`、共享标题及对端连接：

| 动作 | 目标效果 |
|---|---|
| 归档 | 移出活动列表，保留每条消息阅读状态、未读计数、历史与草稿 |
| 恢复 | 使用原 Session 引用、历史及活动时间，重新进入活动列表 |
| 彻底删除 | 移除当前 owner 的 Session 可见历史引用、草稿及相关个人偏好，不删除其它 owner 的引用或联系人 / 连接 |
| 新消息 | 有效普通消息可以解除归档；删除水位后的有效新消息可按连接规则形成新可见历史，旧消息重放不能复活已删内容 |

当前 `msg.update_record_state` 只处理单条记录。`ARCHIVED` 仍进入 list_sessions/list_session，
改写后不再计为 UNREAD，也没有恢复到普通阅读状态的迁移；`DELETED` 仅在会话查询中被过滤。
因此批量写这些状态既不能保持归档语义，也不能兑现会话级删除承诺。

已实现：`msg.archive_session` / `msg.restore_session` / `msg.delete_session` / `msg.get_session_state` 以 `owner_sessions`
持久保存生命周期与删除水位 `(sort_key, record_id)`，`msg.list_sessions` / `msg.list_session` 与未读、请求计数均在水位之后统计；
新 `chat` / `group_msg` / `deliver` 记录提交后自动解除归档。手工创建走 `msg.create_session`（registered 行，空历史可列出）。
后端需持久保存 owner 范围的生命周期、删除水位与最小连接来源，提供幂等的会话级操作及并发新消息处理。
删除的不可恢复范围是该 owner 的本地历史引用；不可变对象还可能被其它 mailbox/delivery 引用，
对象回收必须单独按共享引用处理，不能承诺清除所有物理副本。操作成功后再清理本地缓存，
失败不展示成功结果；能力未提供时禁用对应操作并说明原因。RPC 名称和存储方案在实现阶段定义。

### 4.6 消息请求与联系人准入

后端 `REQUEST_BOX` 已承载低信任来源消息。UI 需提供可发现的请求入口与来源提示，
按记录的 `box_kind` 标识请求，不把整个 Entity / Session 永久标为陌生人；同一会话可能包含历史请求与普通消息。
请求入口是已有记录的筛选视图，不制造第二份消息或伪 Entity。完整请求计数 / 分页需后端支持，不能只查最后一条记录。

展示当前 owner 的 Contact.access_level 与有上下文的临时授权到期信息。查看、标已读、回复、
允许后续投递是不同动作；查看请求不授予 friend / temporary 权限。
授权操作复用 `contact.grant_temporary_access` 或经校验的联系人准入修改，拉黑复用 `contact.block_contact`。
这些动作调整本 owner 的准入规则，不修改域外实体自身资料；Agent 观察禁用。

当前改变联系人准入并不会迁移旧 REQUEST_BOX 记录。接受后的历史迁移 / 保留、请求处理状态与幂等结果
须补显式服务契约；在此之前只准确反馈本次权限变更，不能清空请求列表或伪称历史已转入 INBOX。
群发言、成员管理等权限仍通过 GroupMgr 判断，不能从联系人 friend 或 native 连接推导。

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
| 空（无数据） | `status='success'` 且 `data.length === 0` | 按能力引导与 Agent 新建会话或建立连接；Agent 观察仅显示空态 |
| 空（过滤无结果） | 过滤后为空但 `data.length > 0` | `messagehub.noResults` + 清除过滤按钮 |
| 加载 | `status='loading'` | 8 条骨架行 |
| 错误 | `status='error'` | 错误文案 + 重试 |
| 进度 | `isLoadingMore` | 列表底部行内 spinner |

「空数据」与「过滤无结果」必须区分——原型当前只有后者。
未归类 Session 分组单独保留（§3.2.1）；有未归类历史时不能显示“没有任何会话”。

#### Session 侧栏 — `DataState<EntitySession[]>`

| 状态 | 呈现 |
|---|---|
| 正常 | session 列表，当前项右侧高亮条 |
| 空 | 实体尚无连接 / Session 时的正常状态；按 `sessionCreation.canCreate` 或连接能力提供入口，观察模式仅展示 |
| 加载 | 3 条骨架行 |
| 错误 | 行内错误条 + 重试，不阻塞 Conversation |
| 进度 | 无（session 数量有界，一次拉完） |

`sessionCount <= 1` 时整个侧栏入口隐藏。
可用的新建会话 / 建立连接入口不能随侧栏一起隐藏；零会话时 `selectedSessionId = null`。

#### Panel B 会话视图 — `ConversationProjection` + `DataState`

| 状态 | 触发 | 呈现 |
|---|---|---|
| 正常 | 投影非空 | 消息时间线 |
| 空 | `totalCount === 0` | 可写会话显示「开始对话」；只读会话显示「暂无消息」及只读原因 |
| 加载（首屏） | 投影未建立 | 消息区骨架 |
| 加载（窗口） | 窗口未覆盖当前视口 | `ListItemPlaceholder` 占位行，保持滚动高度稳定 |
| 加载（历史） | 上翻触顶 | 顶部 spinner，滚动位置锚定不跳 |
| 错误 | reader 抛错 | 全区错误态 + 重试 |
| 进度 | 出站消息 `ui_delivery_status = 'sending'` | 气泡内时钟图标 |
| 瞬时状态 | 可信运行态来源及有效期内的条目 | 居中状态 pill；持久 notify 保留消息与记录上下文，见 3.5.1 |

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
| 只读 | 禁用输入、上传、发送与重试；显示原因，符合条件时显示「启用写入」 |
| 空草稿 | 发送按钮禁用 |
| 发送中 | 按钮 loading，输入框保持可编辑（允许连续发送） |
| 提交失败 / 结果未知 | 保留可恢复输入与失败信息；未知结果使用同一对象和幂等键重试，见 4.2.1 |
| 投递失败 / 部分失败 | 消息显示逐目标进度与原因；自动重试由 executor 管理，人工重投待契约补齐 |
| 附件处理中 | 附件卡片显示进度；发送按钮在全部附件就绪前禁用 |
| 拖拽悬停 | 全区 drop overlay |

### 5.3 页面视图状态

`types.ts` 中现有的 `MessageHubState` 已落后于实现且无消费点，替换为：

```ts
export type MobileView = 'entity-list' | 'conversation' | 'details'

export interface MessageHubContext {
  viewerDid: DID
  sessionOwnerDid: DID
  mode: 'self' | 'agent_observer'
}

export interface MessageHubViewState {
  context: MessageHubContext
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
  detailsTarget: 'entity' | 'session' | null
  archived: boolean

  /* 布局态（仅桌面端有意义，应持久化到本地） */
  layout: MessageHubLayoutState
}

export interface MessageHubLayoutState {
  entityListWidth: number         // [280, 520]，默认 340
  sessionSidebarWidth: number     // [240, 520]，默认 280
  isEntityListCollapsed: boolean  // 折叠宽度 68
}
```

`self` 要求 viewer 与 owner 相等；`agent_observer` 由 Agent 主页入口加服务端查看授权建立。
选择态、实体归类、未读聚合、分页游标、reader、本地缓存与草稿均在当前 context 内计算。
切换 owner 时取消旧请求 / 订阅，清除当前选择与临时写入确认，忽略旧 context 的迟到响应；
可以恢复目标 context 自己的选择态，不能沿用上一个 owner 的同名 Session。
拒绝授权应显示无权查看，不能回退显示其它 owner 的缓存。

布局常量定义在 `layout.ts`，是 UI DataModel 的一部分（会被持久化），不是纯样式常量。

`isResizingEntityList` / `isResizingSessionSidebar` 属于拖拽过程中的瞬时交互状态，
**不进入** `MessageHubViewState`，留在组件内部 ref/state。

---

## 6. 分页与聚合

### 6.1 实体列表

- **数据来源**：当前 owner 的 `msg.list_sessions` 各页 + 有权限的联系人 / 群资料 + 待补齐的会话登记。
  普通会话入口以该 owner 的已有 Session 组织实体；联系人可作为创建选择项，父实体可作为层级导航容器。
- **分页策略**：cursor。游标为 `(next_cursor_updated_at_ms, next_cursor_session_id)` 二元组，
  两者任一为 `undefined` 即为末页。
- **页大小**：50（当前实现为 200，偏大，接后端时下调）。
- **排序**：`lastActiveAt` 降序；`isPinned` 的实体恒置顶，置顶组内部同样按 `lastActiveAt` 降序。
- **聚合**：
  - `Entity.unreadCount = Σ session.unreadCount`
  - `Entity.lastActiveAt = max(session.lastActiveAt)`；零 Session 时为 0，无 lastMessage
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
  当前后端加载对象失败会使整页 RPC 失败，应保留页面错误与重试；逐条不可用占位只有在服务返回该记录时才能构造，
  若需单个坏对象不影响整页，还要补服务端局部失败语义。
- **已知缺陷**：`createSessionApiReader()` 当前 `descending: false` 从头拉完整个会话。
  长会话必须改为「最新一页 + 向上增量」。
- **窗口物化**：当前 `DEFAULT_PAGE_SIZE = 32`，`INDEX_SCAN_PAGE_SIZE = 128`。
  原型一次性扫描建立索引；真实接入只对已加载历史建立索引，向前扩展时保留 record_id 锚点，
  不为了得到 totalCount 或虚拟列表索引预先扫描整段远端历史。
- **时间分隔符**：相邻消息间隔 `TIMESTAMP_GAP_MS = 30 min` 时插入一条 `timestamp` 条目。
- **状态条目**：`tail` 位置的状态 pill（typing / processing）不参与持久投影，
  由 `statusItemsSignature` 单独比对，避免每次状态变化触发全量重建。

### 6.3 Session 列表

一次性拉取，不分页。单实体 session 数预期 < 50。按个人置顶优先、`lastActiveAt` 降序、稳定 Session ID 升序排列。归档过滤先于活动列表排序。

上述一次性列表是 mock 假设。真实 Sidebar 消费按需加载的 Session 集合，不能为满足实体聚合而先拉完 owner 的全部会话。
跨页完整计数、置顶及活动排序应由同口径服务投影支撑。

### 6.4 未读聚合口径

| 层级 | 口径 |
|---|---|
| Session | `SessionSummary.unread_count`，后端按 `RecipientState = UNREAD` 计数 |
| Entity | Σ 其下 session 的 unread_count |
| 全局 | 登录用户 owner 下全部唯一 Session 的未读和（含未归类、排除子实体重复计数），供 App 图标 badge |

静音（`isMuted`）**不减少**未读计数，只改变 badge 配色与提醒行为。这是原型已确立的口径，保持不变。
请求箱和归档会话仍属于 owner 的唯一 Session 统计，筛选入口不重复贡献 badge。
不参与 lastActiveAt 的持久消息仍可能有 UNREAD 记录；不能用活动类别判定替代阅读状态统计。
当前 mock 只为有效普通入站消息增加未读，且尚无真实阅读状态更新，不能据此验证服务端未读口径。
Agent 视角仅展示 Agent 自己的未读聚合，不加入用户 App badge；观察不调用阅读状态更新、
`msg.set_read_state` 或持久化 Agent 的 `ui.last_read_sort_key`。

### 6.5 本地阅读状态与对端回执

- 本地已读：自己的会话处于可见且贴底状态时，对确实已展示的入站记录调用
  `msg.update_record_state(record_id, READ)`；SENT 不具有阅读语义。大批记录可补 owner 范围的批量接口，
  不能靠消费型 `msg.get_next` 推进 UI 历史或抢占 Agent 待处理消息。
- 未读数以服务端 `SessionSummary.unread_count` 为准；仅在成功写入后确认本地减少，失败或迟到新消息须重读对账。
  已归档记录的阅读更新与会话归档保留未读需按 §4.5 分离，不能强行执行现有非法状态迁移。
- `ui.last_read_sort_key` 仅是个人展示水位，不改变 mailbox 未读；同 sort_key 的记录还需 record_id
  才能无歧义定位阅读边界，批量接口应定义完整游标。
- 对端回执：`msg.set_read_state(group_id, msg_id, reader_did, status, ...)` 写 MsgReceiptObj，
  不修改 MailboxRecord；`msg.list_read_receipts` 查询该回执。当前实现只保存于内存，重启即丢失，
  没有完整的私聊回执及持久发布能力。不得为私聊伪造 group_id，也不得承诺跨端持久的已读回执。
- 回执读取 / 写入需校验 reader 与当前身份 / 群关系；Agent 观察不执行任何阅读或回执写入。

### 6.6 增量更新、事件与断线恢复

当前 API reader 仅全量读取一次再 append；它无法反映已加载消息投递变化、删除和会话重新归类。
接入时需在已加载集合上支持按 `(ownerDid, recordId)` upsert / remove，保留 msg_id 索引、分页边界和滚动锚点。
记录修改但总数不变也要触发投影 / 展示更新；重新归类需同时使旧、新 Session 摘要与历史失效。

`msg_center` 已在提交后发布 mailbox 与 delivery changed 事件，但它们是可丢失的刷新信号，
delivery 事件按 transport/executor 组织，没有现成的 viewer / owner / Session 订阅契约。
UI 通过有权限的 owner/session 事件投影或受控轮询触发 Session API 重读，不读取 DELIVERY_QUEUE。
只补拉末尾新消息不能发现旧记录更新 / 删除；需定向重读受影响记录 / 已加载页，或补有变更游标的接口。
首次进入、重新联网及恢复前台须对账摘要和已加载状态；重复事件按稳定记录键去重，缺失 / 乱序事件不得回滚权威状态。
切换 owner 取消订阅和旧请求，丢弃迟到结果。具体事件网关或轮询实现不预设新的 RPC 名称。

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
| `sessionCreation` | Extensible（目标） | owner / 实体级策略与有效能力，后端待补齐 |
| `isPinned` / `isMuted` | Frozen | 落在 ui_session KV |
| `isOnline` / `statusText` | Volatile | presence 模型尚未定稿（PRD §18.6） |
| `children` / `childrenMode` / `childrenSections` / `drilldownDescription` | Volatile | 子实体来源与 drilldown 配置仍在演进 |
| `avatar` | Volatile | 渲染层尚未接入真实头像 |

### EntitySession

| 字段 | 稳定性 | 说明 |
|---|---|---|
| `id` / `ownerDid` | Frozen（目标） | 后端 session_id 原值与所属身份共同构成引用 |
| `entityId` | Extensible | 归属关系，未归类时为 null |
| `binding` / `origin` / `access` | Extensible（目标） | 稳定连接、创建来源与有效读写能力，后端待补齐 |
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
| `content.title` | 未消费 | 协议已定义，当前通用渲染未读取 |
| `content.machine` | Extensible | Action renderer 已读取 intent/data，未知动作或版本保留摘要 |
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

## 8. Mock 数据契约与覆盖边界

当前 mock 字段和交互现状以 [当前 UI Model Data](../../../../../../product/message_hub/MessageHub_Current_UI_Model_Data.md)
及 [原型交付说明](../../../../../../proposals/messagehub-ui-prototype/IMPLEMENTATION.md) 为准。下面区分已经模拟的能力与真实接入缺口；
不再沿用 v0.3 中把空会话、失败注入和 Agent 观察全部列为缺失的旧盘点。

| 场景 | 原型现状 | 真实接入还需验证 |
|---|---|---|
| Entity、父子结构、多 Session、长历史 | 已有 seed、可变 store 与虚拟列表 | 原始群目标归属、真实 DID 归一和未归类会话 |
| 手工创建、空历史、首条消息 | 已有 mock 交互 | 持久登记、首条消息沿用 ID、远端上下文真实创建 |
| 归档、恢复、删除水位 | 已有 mock 交互 | 独立生命周期、阅读状态保留、并发消息和重启恢复 |
| 多 tunnel、同实例不同上下文 | 已有 Personal / Work 与 General / Design seed | 真实实例 / 端点绑定、同名 topic 不碰撞、发送不换路 |
| Agent 观察、拒绝、owner/viewer 缓存隔离 | 已有 mock 交互 | 服务端授权、旧 API reader 身份作用域及迟到响应 |
| tunnel 只读、确认、连接失效 | 已有 mock 交互 | native 路由、群 action 权限与字段编辑能力 |
| 共享 / 成员状态、Action 过滤 | 已有 mock 交互与 renderer | 权威 revision、可靠日志发布、GroupEvent / 平台回显对账 |
| runtime、消息 / 投递事件、时钟 | DEV 可注入 | 可信运行态与到期、真实事件 / 轮询及旧记录更新 |
| 初始化 / 操作失败 | 已有 delayMs / failNext 与重试 | RPC ok:false、结果未知、逐目标失败和 duplicate_risk |
| 文本 / 附件草稿 | 已有本地 File 持久化；发送仍拼摘要 | 对象上传、cyfs:// / 无 hint 引用、通用附件下载与局部失败 |
| 请求箱、本地已读、回执 | 未实现真实协议流程 | REQUEST_BOX 准入、mailbox 已读与回执分离、回执持久性 |
| 按需分页、非追加更新 | mock 可注入；旧 API reader 仍全量读取 | 大账号首屏、旧消息更新 / 删除 / 重新归类、断线补拉 |

原型行为覆盖不代表本表最后一列已完成；真实接入验收统一见 §9.4。

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

服务名：`msg-center`。`datamodel/sessionApi.ts` 仅封装了 3 个方法，当前 MessageHub 主视图使用 mock store，未调用这些真实 API。
以下现有 RPC 名称不表示目标能力已完整实现；新增契约单列在 §9.3。

### 9.1 读取路径

| UI DataModel | KRPC 方法 | 变换 |
|---|---|---|
| `Entity[]` | `msg.list_sessions` + `contact.list_contacts` + `group.list_by_member` | 目标三路合并；还需会话登记 / 能力，见 3.2 |
| `Entity.id` | `contact.resolve_canonical_did` | 别名 DID 归一 |
| `Entity.unreadCount` | `SessionSummary.unread_count` | Σ 聚合 |
| `Entity.lastMessage` | `SessionSummary.last_record.msg` | 见 3.5.2 摘要规则 |
| `Entity.lastActiveAt` | 独立的有效消息活动时间（后端待补齐） | max(session.lastActiveAt)，不含状态 / Action Log |
| `Entity.children`（群） | `group.list_subgroups` | `GroupSubgroup` → `Entity` |
| `EntitySession[]` | `msg.list_sessions(owner=context.sessionOwnerDid)` | 按 3.2.1 分组；空 Session、绑定与能力需补齐登记契约 |
| `EntitySession.title` | `ui_session.get_state('ui.title')` / 待实现共享状态读取 / `msg.thread.topic` | 见 3.3.1 |
| `EntitySession.isPinned/isMuted` | `ui_session.get_state` | KV 反序列化 + schema 校验 |
| `EntitySession.lifecycle` / `lastActiveAt` / `requestCount` | `msg.list_sessions(lifecycle: 'all', order_by: 'activity')` 的 `lifecycle` / `last_activity_ms` / `request_count` | 服务端按活动时间排序并给出同口径游标 |
| `MessageObject[]` | `msg.list_session`（当前 owner，`with_object: true`，最新页 + 向上分页） | `api/reader.ts::itemToMessage`，记录上下文置于 `ui_record` |
| 附件对象 / 内容 | `GET /kapi/msg-center/objects/{obj_id}[/content]`（Bearer 会话 token） | `api/objects.ts`，按 `obj_id` 访问，`uri_hint` 只作提示 |
| `ui_message_id` | `SessionMessageItem.record_id` | 直接 |
| `ui_delivery_status` | `SessionMessageItem.delivery.overall` | 枚举映射，见 3.5 |
| `ui_sender_name` | `MailboxRecord.from_name` | 缺失时回落 DID |
| `EntityDetail` | `contact.get_contact` / `group.get_doc` | `Contact` / `GroupDoc` → `EntityDetail` |
| `EntityDetail.memberCount` | `group.list_by_member` 的 `GroupSummary.member_count` | `group.get_doc` 不返回成员数 |
| `AccountBinding.endpointDid` | `Contact.bindings[].endpoint_did` | 直接 |
| `EntityDetail.accessLevel` | `Contact.access_level` | `SCREAMING`→`snake` 已由 serde 处理 |
| 请求记录及准入提示 | `msg.list_session` 的 box_kind / `msg.list_box_by_time` 的 REQUEST_BOX + `contact.get_contact` | 保留记录来源；请求处理状态与跨页计数待补齐 |
| 群操作能力 | `group.check_access` | 按 actor 和 action 查询，不由 native / 本地托管推断 |
| 回执详情 | `msg.list_read_receipts` | 独立于 mailbox 与 delivery；当前持久性限制见 6.5 |

### 9.2 写入路径

| UI 动作 | KRPC 方法 | 请求体 |
|---|---|---|
| 发送消息 | `msg.post_send` | `{ msg: MsgObject, idempotency_key }`，见 4.2.1 |
| 本地标记已读 | `msg.update_record_state` | `{ record_id, new_state: 'READ' }`，自己的已展示入站记录；批量水位待补，见 6.5 |
| 写入群消息回执 | `msg.set_read_state` | group_id / msg_id / reader_did / status；不清除邮箱未读，当前仅内存保存 |
| 单条记录状态变更 | `msg.update_record_state` | RecipientState；不能代替 Session 生命周期 |
| 会话归档 / 恢复 / 彻底删除 | `msg.archive_session` / `msg.restore_session` / `msg.delete_session` | `{ owner, session_id }`；保留阅读状态、删除水位和共享对象引用，见 4.5 |
| 手工创建空会话 | `msg.create_session` | `{ owner, peer_did, title?, binding?, session_id? }`；返回 `OwnerSessionState` |
| 个人显示标题 / 置顶 / 静音 | `ui_session.update_state` | `{ owner, session_id, key, value }`，带 `owner` 走 owner 范围表；草稿仅存 viewer 本地 |
| 编辑备注 / 标签 / 访问级别 | `contact.update_contact` | `ContactPatch` |
| 拉黑 | `contact.block_contact` | |
| 临时授权 | `contact.grant_temporary_access` | |
| 会话重新归类 | `msg.update_record_session` | 仅可信后端/Agent 使用，UI 暂不暴露 |

所有写动作都校验当前 context；Agent 观察首期禁用整张写入表，而不只是隐藏 Composer。

### 9.3 后端集成依赖与实现边界

2026-09-07 状态：#7、#10、#13、#15 已实现（会话登记、viewer → owner 授权、owner 范围 UI 状态、生命周期、活动排序与游标）；
#5 已实现对象访问与上传通道；#4、#6、#14、#16 前端已按现有 RPC 接入，仍缺本节列出的后端契约；
#1、#2、#3、#8、#9、#11、#12、#17 未实现，UI 以只读或按需读取呈现。

1. **实体列表没有单一接口。** 目前需要 UI 端做三路合并 + N 次 `resolve_canonical_did`。
   集成应控制首屏请求数，并提供完整的计数与分页排序；优先复用现有读取和批量能力。
   是否新增实体聚合端点由实现方案与性能验证决定，本次不预设 `msg.list_entities` 为现有 RPC。
2. **`SessionSummary` 缺 `last_delivery`。** 当前要拿到会话级投递失败，必须再调一次 `msg.list_session`。
   建议在 `SessionSummary` 上补一个聚合投递态。
3. **`ui_session` KV 无批量读取的按 owner 维度接口。** `ui_session.list_state` 是按 `session_id` 的，
   首屏 N 个会话就要 N 次调用。需要一个按 owner 批量拉取的形式。
4. **本地已读与回执。** 按 §6.5 修正调用；需补批量阅读边界、回执持久化 / 发布与私聊契约。
   当前 group_id 必填和内存 receipts 不能被解释为完整的跨端已读能力。
5. **附件访问与上传。** 接通对象 ID 到可授权下载 / 预览的通道，再接上传；必须覆盖 Telegram 的
   `cyfs://` 和无 uri_hint 的引用。4.2.1 的 named_store 引用构造须与 content_mgr 对齐。
6. **运行态与 presence。** Telegram 已读写 `ui_session` 的 active / typing / status_line，
   但缺成员 DID、有效期和完整 owner 隔离；不能直接映射为原型 RuntimeState 或 Entity.isOnline。
   需明确可信生产者、会话绑定及过期规则；无权威来源时显示未知，不能把断线缓存当在线。
7. **稳定会话登记与空会话。** 当前列表从 mailbox 索引派生，没有手工创建 / 连接创建的空 Session 契约。
   需支持幂等登记、重启恢复、首条消息沿用 ID，并显式返回 owner、对端、连接及远端上下文绑定。
   消息时间线继续由 mailbox 重建；登记元数据不能伪装成消息，也不能只藏在可重建的索引或 UI KV 中。
8. **跨 tunnel 隔离与发送上下文。** 当前 `derive_session_id` 直接优先使用 topic，未按连接隔离。
   需保证不同实例 / 端点 / 远端上下文独立，同一连接双向一致，且每条持久消息都有归属。
   本地 Session 选择与外部 thread 回复之间的显式映射尚无完整契约；不得把本地 ID 塞入远端 thread 字段。
9. **创建策略与写入能力。** 需提供 `(owner, entity)` 创建策略存储、tunnel 多会话 / 远端创建能力、
   有效发送与展示编辑权限的读取和校验。后端必须校验创建及发送，不能只依赖 UI 按钮状态。
10. **Agent 视角授权与状态隔离。** `list_sessions/list_session` 已有 owner 参数，
    但当前 `msg_center.rs` 对应 handler 未使用 `RPCContext`；不能据此认定已支持跨 owner 授权。
    需核实并补齐 viewer → owner 读取权限，首期禁用代理写入，并将 `ui_session` 持久键及接口补为 owner 范围。
    后续代发另行提供 Agent 发送授权与真实操作者审计，不能只改 `MsgObject.from`。
11. **共享 / 成员状态。** 需补权威会话引用、成员 DID 维度、revision、字段权限及幂等修改，
    与旧 `ui_session` 个人偏好 / tunnel 运行态分开。模型见状态与日志主文档 §2–§3。
12. **Action Log。** 复用 event / machine 载荷，补权威状态更新与日志登记的一致性、持久发布恢复，
    以及 GroupEvent / 外部事件的映射、确认、去重与乱序对账。GroupMgr 当前持久化 group_events，
    尚未将其发布为 Session 时间线中的统一日志；前端已有 mock Action renderer 不代表后端已完成。
13. **Session 生命周期。** 归档保留阅读状态、恢复和删除水位需独立持久化，
    不用 RecipientState.ARCHIVED/DELETED 代替；成功条件和共享对象处理见 §4.5。
14. **请求处理。** REQUEST_BOX 的入口可消费已有记录；接受后的历史处理、请求处理状态、
    owner 范围的计数与分页须补服务契约，联系人授权变化不会自动迁移旧记录。
15. **有效消息活动时间。** 当前 list_sessions 按 MAX(updated_at_ms) 排序，不能满足 §1.4a；
    需独立的 lastActiveAt 与同口径分页游标。未读仍按记录阅读状态统计，不能随活动过滤。
16. **增量刷新。** 补适合 owner/session 的授权事件投影或受控轮询与补拉；
    现有 executor delivery 事件不是 UI 订阅 API。记录状态、删除及重新归类须可对账，见 §6.6。
17. **投递诊断与人工重投。** 复用现有 per_target 与 DeliveryError；不丢弃提交结果。
    若开放人工重投，需另定按目标授权、幂等及 duplicate_risk 处理，不能用 post_send 整条重发替代。

本节列出真实接入依赖，不要求本次文档更新实现这些能力，也不把待实现能力当成新 RPC。
共同设计边界见 [Message Center §5](../../../../../../doc/message_hub/Message%20Center.md)。

### 9.4 接入验收条件（2026-09-07 执行记录）

验证载体：R = `test/test_msg_center/test_messagehub_sessions.ts`（真实 zone RPC，用户 token）；
E = `tests/e2e/real/messagehub.real.spec.ts`（真实 zone UI，1440 / 375）；U = `cargo test -p msg_center`；
D = `tests/datamodel/messagehub-projection.test.ts`（投影夹具）。

| 场景 | 必须满足的结果 | 状态 |
|---|---|---|
| 群 topic 会话交替出现成员 INBOX 副本与 SENT | 用原始群目标 / 权威绑定归属，实体不变成 owner；多目标不取第一人猜测 | D 通过（群 tag / group_msg 目标 / 多目标进入未归类）；真实 zone 无群数据，未在 R / E 覆盖 |
| 阅读自己的入站消息 / 查看 Agent 会话 | 前者成功后 mailbox 未读减少；回执写入不代替该更新；后者无阅读或回执写入 | E（观察无 Composer、Manage / 偏好禁用）、R（观察者的 update_record_state / archive / ui_state 被服务端拒绝）；本地已读经 `update_record_state` 后再重读摘要 |
| post_send 正常返回 ok:false / 返回超时 | 展示真实拒绝原因；结果未知重用原对象及幂等键，不产生双消息 | R（ok:false reason、同幂等键重放同 msg_id、删除后重放不复活）；超时路径按同幂等键重试，未在真实 zone 制造超时 |
| 多目标部分成功、后台重试、cyfs-cached | 展示逐目标事实与风险；暂存不算送达；不重新投递已成功目标 | D（partial_failed / duplicate_risk 投影）+ 渲染器展开逐目标；真实 zone 单目标，未覆盖多目标 |
| 归档 → 刷新 → 恢复，删除后并发新消息 / 旧消息重放 | 保留归档前阅读状态与活动时间；删除水位以前的本 owner 历史不复活，不影响其它 owner | U（含另一 owner 不受影响、水位后新消息重新可见）、R、E 通过 |
| 陌生人请求、临时授权到期、拉黑 | 请求来源可辨；查看不授权；接受后的旧请求处理按服务契约反馈 | E（REQUEST_BOX 横幅 / 逐条标记、观察者无准入按钮）；准入动作复用 contact RPC，接受后的旧记录迁移契约未实现（明确提示保留在请求箱） |
| native 无路由、群无发言权、共享字段无编辑权 | 对应能力关闭且有原因，不因 native 或本地托管开放按钮 | E（共享 / 成员编辑禁用并说明）；群走 `group.check_access`（真实 zone 无群，未覆盖）；native 无路由由 post_send 拒绝并展示原因（R 用未知 tunnel 验证） |
| Telegram 图片 / 文件、无 uri_hint、单附件不可用 | 按 obj_id 访问；保留文件入口与正文，错误局部展示 | E（上传 → `cyfile:` 引用 → 对象路由 401 / 200）；对象缺失时局部“附件不可用 + 重试”；真实 Telegram 媒体消息未出现在测试数据中 |
| 不同 owner 使用同名 sessionId | API reader、缓存、偏好、游标和迟到响应隔离；服务端拒绝越权读取 | U（owner UI 状态隔离、越权读写拒绝）、R（越权读取拒绝）；前端缓存按 `(viewer, owner, session)` 键 |
| 老消息投递变化、删除、重新归类，断线后恢复 | 按记录更新已加载列表、旧 / 新 Session 摘要，保持滚动锚点，无重复或陈旧条目 | 已实现按 record_id upsert / remove 与锚点保持（D 覆盖 upsert 语义）；轮询 / kevent 只对账最新页，更早记录的变更游标未实现 |
| 大量会话 / 长历史、状态修改引起 updated_at 变化 | 首屏按需分页，有效消息活动排序跨页一致，不先拉全量数据 | U（READ / event 不改活动顺序、limit=1 分页游标一致）；前端首页 50 条 + “加载更多”，历史最新 64 条 + 向上翻页 |
| Action / 运行态 / 持久 notify | GroupEvent 日志按权威来源去重；运行态到期；持久消息不因类型被丢弃或漏计未读 | U（event 记录不解除归档、仍计未读）；运行态 typing 30s / status_line 10min 到期（E 中显示 processing）；GroupEvent → Action Log 发布未实现 |

P1（记录语义、提交结果、会话生命周期、请求准入、授权边界）与附件通道、增量加载已执行；
多目标投递、群权限、Telegram 媒体、老记录变更游标与 GroupEvent 日志仍待真实数据或后端契约。

---

## 10. 与原型现状的差异清单

本文档相对 `MessageHub_Current_UI_Model_Data.md` 记录的实现现状，需要在集成阶段落地的改动：

| # | 改动 | 原因 |
|---|---|---|
| 1 | mock Entity.id 已 DID 化；接入真实联系人归一与群目标推导 | 群成员副本的 record.to 不是群 DID |
| 2 | `Entity.lastMessage` 由 session 摘要派生，不再独立 mock | 消除两套并行数据 |
| 3 | mock 已消费 `Entity.lastActiveAt` 作为排序键 | 真实后端仍按通用 `updated_at_ms` 排序，必须新增活动时间及分页游标 |
| 4 | `Entity.source: string` → `sources: string[]` | 一个实体可有多个平台绑定 |
| 5 | 新增 `Entity.domain` / `sessionCount` / `lastActivitySessionId` | PRD §12.4/§15.1 权限差异与默认会话选择 |
| 6 | mock 已删除 `isActive`，共享标题与个人覆盖分开；实际类型仍名 `Session` | `EntitySession` / `titleSource` / `lastDelivery` 为后端集成目标命名 |
| 7 | `EntityDetail.bindings` 改为必填，新增 `accessLevel` / `isVerified` / `contactSource` | 权限区块需要 |
| 8 | `AccountBinding` 新增 `endpointDid` / `accountType` | 回复外部平台消息的目标地址 |
| 9 | 附件从「拼进文本」改为写入 `content.refs` | 当前是纯 UI 演示态 |
| 10 | `MessageHubState` → `MessageHubViewState`，补齐 drilldown / 布局 / 展开态 | 旧接口无消费点且落后于实现 |
| 11 | `listAllSessions` / `createSessionApiReader` 改为按需分页 | 全量拉取在真实数据量下不可用 |
| 12 | 无 `msg` 的时间线条目由静默丢弃改为占位渲染 | 静默丢消息不可接受 |
| 13 | mock 已有延迟与失败注入；补真实 ok:false / 超时 / 权限失败测试 | mock 失败开关不能验证 RPC 业务结果 |
| 14 | 实体列表区分「无数据」与「过滤无结果」两种空态 | 当前只有后者 |
| 15 | 将 mock owner / binding / origin 接入权威契约；补未归类 UI 分组 | 当前 Session.entityId 必填，未知真实会话仍需可读 |
| 16 | 接入连接自动登记、Agent 手工创建与实体级策略 | 支持无消息的已创建 Session，区分实体存在与会话存在 |
| 17 | mock 已有 tunnel 只读及确认；接入 native / tunnel 真实能力 | 不能由传输方式推导发送和状态编辑权限 |
| 18 | mock 已有 Agent 观察隔离；补服务端授权和 API reader 作用域 | 后端与旧 API reader 仍不满足身份隔离 |
| 19 | 保留记录上下文和完整提交 / delivery / receipt 信息 | 支持已读、请求、逐目标诊断和状态刷新 |
| 20 | 接入独立 Session 生命周期与删除水位 | 单条记录状态不能实现归档 / 恢复 / 彻底删除 |
| 21 | 增加请求处理、对象附件访问及记录 upsert / remove | 覆盖后端已有消息类型与动态变化，验收见 9.4 |

---

## 11. 一句话总结

MessageHub 的 UI DataModel 是一个**三层投影模型**：协议层原样镜像 `msg-center` 的 `MsgObject` 与 session 投影；
UI DataModel 层把扁平的 `session_id` 空间还原成「实体 → 会话」两级结构，并把后端不存在的展示态属性
（标题、置顶、静音、草稿）统一收到 `ui_session` KV；组件层只消费 UI DataModel。
这一层投影还须对齐 owner 作用域、连接绑定、创建策略与读写能力（§3.3 / §5.3）；
这些业务关系由后端契约支撑，不能仅凭 Session ID 形态或 UI 展示 KV 推断。
