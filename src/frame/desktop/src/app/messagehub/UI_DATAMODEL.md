# MessageHub UI DataModel

- 文档版本：v0.3（2026-09-06：补充共享 / 成员状态与 Action Log 数据契约）
- 文档类型：UI DataModel 设计文档（WebUI Dev Loop 阶段三产物）
- 模块位置：`src/frame/desktop/src/app/messagehub`
- 上游文档：
  - PRD：`product/message_hub/MessageHub_Web_UI_PRD.md` v0.3
  - 状态与日志：`doc/message_hub/Session State and Action Log.md` v0.1（目标契约，待实现）
  - 原型现状盘点：`product/message_hub/MessageHub_Current_UI_Model_Data.md` v0.1
  - 后端数据模型：`src/kernel/buckyos-api/src/msg_center_client.rs`、`src/frame/msg_center/src/`
- 下游用途：`integrate-ui-datamodel-with-backend`

---

## 1. Overview

### 1.1 本文档解决的问题

MessageHub 原型已经收敛（Entity List / Conversation View / Details 三层结构 + 多 Session 切换 + 消息投影 + Composer 草稿），
但原型的数据来自 `mock/data.ts`，其模型与 `msg-center` 的真实数据模型之间存在三处结构性错位：

1. **后端 Session API 没有统一的 Entity 聚合模型。** `msg-center` 的会话读取面只有 `SessionSummary`（会话摘要）与 `SessionMessageItem`（会话时间线），
   两者都在 owner 范围内以 `session_id` 为键。UI 的「实体 → 会话」两级结构必须由投影得到。
2. **当前 Session API 没有共享标题、置顶、静音等完整模型。** 个人展示属性走 `ui_session.*`；
   共享标题与成员会话昵称属于新增的持久业务状态，不能用个人显示标题或无约束 KV 代替（§3.3.5）。
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

- 协议镜像必须与真实后端一致；纯展示提示走 `ui_*` meta 或 `ui_session` KV。
  owner、稳定连接绑定、创建策略与授权能力属于业务契约，不能伪装为客户端 KV 权限开关。
  本次新增的目标字段与能力尚未实现，须先补齐后端契约再更新协议镜像，缺口见 §9.3。
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
| 其他任意字符串（topic / correlation_id） | 分支 1、2 | 取 `last_record.record`：`msg_kind === 'group_msg'` 时用 `record.to`；否则用 `record.from` 与 `record.to` 中非 owner 的一侧，再 `resolve_canonical_did` |
| 无 `last_record` 且不匹配上述任一形态 | — | 归入 `unassigned` 桶，见下 |

无法归属的 Session 不得丢弃：令 `entityId = null`，在实体列表旁的“未归类会话”分组呈现。
该分组是 UI 容器，不是 Entity，不分配伪 DID，不可作为发送目标；其会话仍可按 owner 与 Session ID 只读打开。
原 v0.1 的 `urn:buckyos:messagehub:unassigned` 不再冒充可寻址实体。
多目标消息无法唯一确定对端 / 群时也进入该分组，不能随意取一个收件人。

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
  /** = SessionSummary.updated_at_ms；目标空 Session 由登记时间提供初始值。 */
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

#### 3.3.5 整体状态与成员状态（仅数据定义，原型后续修改）

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
| 易失状态消息（typing / processing 等） | 不参与摘要，跳过取上一条非状态消息 |
| Action Log（持久 event） | 使用其可读摘要，不能按易失状态丢弃 |
| 无 `last_record.msg` | i18n `messagehub.preview.unavailable` |

#### 3.5.3 Action Log Message（目标数据契约）

Action Message（本文亦称 Action Log Message）是 MessageObject 的可识别业务子类型。
Session / 实体状态变更复用标准已有的 `MsgObject.kind = 'event'` 与
`content.machine.intent = 'buckyos.action_log'`，结构化字段见状态与日志主文档 §4。
`event` 与 `machine.intent/data` 已存在，`buckyos.action_log` 判别值及 payload 是本设计的新增约定；
标准没有独立 Action 枚举，当前代码也尚无 `isActionMessage` 或 Action 专用 renderer。
必须区分目标 Session / 实体、操作者 actor、受影响成员 subject，并保留事件 ID、变更值与可信来源。
例如共享标题更新、会话昵称更新、加入群、主动退群、被管理员移除分别有明确 action。

Action Log 是已确认变化的历史记录，按普通持久消息进入 Session API；
不映射成 `ui_item_kind='status'`，不使用 `ConversationStatusType` 代替业务动作。
现有原型可继续按内容 fallback，专用系统消息呈现后续再做。
普通消息提交或历史重放不触发状态写入；更新状态与发布日志由权威服务负责。

后续特殊展示与过滤都以 `kind === 'event' && content.machine?.intent === 'buckyos.action_log'`
作为类别判断，再校验 data 并按其中 action 选择呈现。不要匹配“加入群聊”等文本，也不能把所有 event 都当成 Action。
专用 renderer 位于通用文本 / 图片之前，未知动作保留摘要回落。

“显示 / 隐藏 Action Message”是纯 UI 偏好：在可见投影生成前筛选，不删除 reader 中的原始消息，
不改变后端历史、权威状态、未读数或日志发布。开关本身不触发标已读。
切换过滤时重建可见 entries、时间分隔与 totalCount，原始 messageIndex 与消息稳定 ID 保持不变。
不能只在 renderer 中返回 null，否则当前链会继续调用文本 fallback，或留下虚拟滚动空行。
本轮只定义分类与过滤边界，不修改原型、reader 或 renderer 实现。

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
| `InMemoryConversationMessageReader` | `memory:{viewerDid}:{ownerDid}:{sessionId}` | mock 数据、本地追加 |
| `IndexedDbConversationMessageReader` | `indexeddb:{db}:{ns}:{viewerDid}:{ownerDid}:{sessionId}` | 大历史本地缓存 |
| `SessionApiConversationMessageReader` | `msg-center:{viewerDid}:{ownerDid}:{sessionId}` | `msg.list_session` 真实数据 |

表中是目标 key，当前实现仍缺身份作用域。各部分需无歧义编码，不能直接按 DID 中的冒号切分。

`readerKey` 变化即视为换会话，投影全量重建；`totalCount` 增长视为追加，投影增量扩展。

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
| 只读 | 禁用输入、上传、发送与重试；显示原因，符合条件时显示「启用写入」 |
| 空草稿 | 发送按钮禁用 |
| 发送中 | 按钮 loading，输入框保持可编辑（允许连续发送） |
| 发送失败 | 消息气泡显示失败图标 + 重试入口；草稿不回填 |
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
| 全局 | 登录用户 owner 下全部唯一 Session 的未读和（含未归类、排除子实体重复计数），供 App 图标 badge |

静音（`isMuted`）**不减少**未读计数，只改变 badge 配色与提醒行为。这是原型已确立的口径，保持不变。
Agent 视角仅展示 Agent 自己的未读聚合，不加入用户 App badge；观察不调用 `msg.set_read_state`
或持久化 Agent 的 `ui.last_read_sort_key`。

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
| 同一联系人 + 两个同平台 tunnel + 同名 topic | 联系人聚合不合并 Session，发送端点不串线 | **缺失** |
| 单 tunnel 多上下文 / 不支持多上下文 | 远端上下文映射与创建能力区分 | **缺失** |
| 连接建立后的空会话 / 无连接实体 | 自动登记幂等、空会话重启可恢复、零会话非错误 | **缺失** |
| Agent 默认可创建 / Person 默认禁止 / 显式覆盖 | 创建策略不等于写入权限 | **缺失** |
| tunnel 默认只读 / 取消确认 / 确认写入 / 恢复只读 | 上传、快捷键、重试与发送门禁一致；刷新后重新确认 | **缺失** |
| 平台只读 / 连接失效 / 绑定未知 | 风险确认不能突破能力限制或自动切换 tunnel | **缺失** |
| 用户与 Agent owner 使用同名 Session ID | reader、选择、草稿、未读和迟到请求隔离 | **缺失** |
| Agent 只读观察 / 无查看权限 | 无发送、新建、已读与配置写入；无跨身份缓存回退 | **缺失** |
| 父子实体均有 Session / 未归类 Session | 子实体不重复计数，未归类历史可见且不可发送 | **缺失** |

共享 / 成员状态与 Action Log 的数据层验收见状态与日志主文档 §6；本次不要求补原型 mock 或 Playwright 流程。

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
以下现有 RPC 名称不表示目标能力已完整实现；新增契约单列在 §9.3。

### 9.1 读取路径

| UI DataModel | KRPC 方法 | 变换 |
|---|---|---|
| `Entity[]` | `msg.list_sessions` + `contact.list_contacts` + `group.list_by_member` | 当前三路合并；目标还需会话登记 / 能力，见 3.2 |
| `Entity.id` | `contact.resolve_canonical_did` | 别名 DID 归一 |
| `Entity.unreadCount` | `SessionSummary.unread_count` | Σ 聚合 |
| `Entity.lastMessage` | `SessionSummary.last_record.msg` | 见 3.5.2 摘要规则 |
| `Entity.lastActiveAt` | `SessionSummary.updated_at_ms` | max 聚合 |
| `Entity.children`（群） | `group.list_subgroups` | `GroupSubgroup` → `Entity` |
| `EntitySession[]` | `msg.list_sessions(owner=context.sessionOwnerDid)` | 按 3.2.1 分组；空 Session、绑定与能力需补齐登记契约 |
| `EntitySession.title` | `ui_session.get_state('ui.title')` / 待实现共享状态读取 / `msg.thread.topic` | 见 3.3.1 |
| `EntitySession.isPinned/isMuted` | `ui_session.get_state` | KV 反序列化 + schema 校验 |
| `MessageObject[]` | `msg.list_session`（当前 owner，`with_object: true`） | `sessionItemToMessageObject`，方向相对 owner |
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
| 标记已读 | `msg.set_read_state` | 仅自己的会话进入且窗口贴底时触发；Agent 观察禁用 |
| 单条状态变更（归档 / 删除） | `msg.update_record_state` | `RecipientState` |
| 个人显示标题 / 置顶 / 静音 / 草稿 | `ui_session.update_state` | `{ session_id, key, value }`，key 见 4.4；不修改共享状态 |
| 编辑备注 / 标签 / 访问级别 | `contact.update_contact` | `ContactPatch` |
| 拉黑 | `contact.block_contact` | |
| 临时授权 | `contact.grant_temporary_access` | |
| 会话重新归类 | `msg.update_record_session` | 仅可信后端/Agent 使用，UI 暂不暴露 |

所有写动作都校验当前 context；Agent 观察首期禁用整张写入表，而不只是隐藏 Composer。

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
    以及 GroupEvent / 外部事件的映射、确认、去重与乱序对账。当前尚未实现统一日志协议。

以上 7–12 为产品规则落地所必需的后端工作，不是已经存在的新 RPC。
共同设计边界见 [Message Center §5.5–§5.6](../../../../../../doc/message_hub/Message%20Center.md#55-会话登记与连接隔离2026-09-06-目标契约待实现)。

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
| 15 | Session 补 owner、稳定连接绑定、来源与能力；替换伪实体 unassigned 为 UI 分组 | 防止会话 / 端点串线，保持真实实体均为 DID |
| 16 | 接入连接自动登记、Agent 手工创建与实体级策略 | 支持无消息的已创建 Session，区分实体存在与会话存在 |
| 17 | 加入 tunnel 默认只读、风险确认及统一发送门禁 | 保持外部软件历史一致性提示与权限边界 |
| 18 | 增加 Agent 观察 context、身份隔离及禁止已读 / 配置写入 | 查看 Agent 自己拥有的 Session，避免模拟发送或改变其状态 |

---

## 11. 一句话总结

MessageHub 的 UI DataModel 是一个**三层投影模型**：协议层原样镜像 `msg-center` 的 `MsgObject` 与 session 投影；
UI DataModel 层把扁平的 `session_id` 空间还原成「实体 → 会话」两级结构，并把后端不存在的展示态属性
（标题、置顶、静音、草稿）统一收到 `ui_session` KV；组件层只消费 UI DataModel。
这一层投影还须对齐 owner 作用域、连接绑定、创建策略与读写能力（§3.3 / §5.3）；
这些业务关系由后端契约支撑，不能仅凭 Session ID 形态或 UI 展示 KV 推断。
