# MessageHub 当前 UI Model Data 整理

- 文档版本：v0.1
- 文档类型：实现对齐文档
- 目标：整理当前 `MessageHub` 代码中实际使用到的 UI Model Data，而不是未来理想模型
- 范围：`src/app/messagehub` 及其直接依赖的 `codeassistant/mockHistory`

---

## 1. 文档目的

当前 `MessageHub` 的界面数据并不是一个单一对象，而是由几层数据共同组成：

1. 页面级实体数据：`Entity` / `Session` / `EntityDetail`
2. 会话消息数据：协议层 `MessageObject`
3. 输入区草稿数据：`ConversationComposerSubmitPayload` / `ComposerAttachmentItem`
4. 页面本地状态：`MessageHubView` 内部的 React state

这份文档只回答一个问题：

`MessageHub` 当前界面到底在读哪些数据字段。

---

## 2. 代码来源

核心定义和消费点主要来自以下文件：

- `src/app/messagehub/types.ts`
- `src/app/messagehub/protocol/msgobj.ts`
- `src/app/messagehub/mock/data.ts`
- `src/app/messagehub/MessageHubView.tsx`
- `src/app/messagehub/EntityList.tsx`
- `src/app/messagehub/SessionSidebar.tsx`
- `src/app/messagehub/ConversationView.tsx`
- `src/app/messagehub/EntityDetails.tsx`
- `src/app/messagehub/conversation/history/renderers.tsx`
- `src/app/messagehub/conversation/input/ConversationComposer.tsx`
- `src/app/messagehub/conversation/input/attachmentDraft.ts`

---

## 3. 顶层结论

### 3.1 当前实际存在的 4 层 UI Data

#### A. Entity List / Header / Details 使用的页面级数据

- `Entity`
- `Session`
- `EntityDetail`

#### B. Conversation History 使用的消息数据

- `MessageObject`
- 少量 `ui_*` 扩展字段

#### C. Composer 使用的草稿数据

- `ConversationComposerSubmitPayload`
- `ComposerAttachmentItem`

#### D. MessageHub 页面本地视图状态

- 当前保存在 `MessageHubView.tsx` 的多个 `useState`

### 3.2 当前并不存在单一的统一状态对象

虽然 `types.ts` 里定义了 `MessageHubState`，但当前代码没有真正使用这个接口作为页面的统一状态承载对象。

也就是说，当前实现更接近：

- 静态 UI 数据：`mockEntities` / `mockSessions` / `mockEntityDetails`
- 动态消息数据：`localReaders`
- 页面局部状态：多个分散的 React state

---

## 4. 页面级 UI Model

### 4.1 Entity

定义位置：`src/app/messagehub/types.ts`

```ts
export interface Entity {
  id: string
  type: 'person' | 'agent' | 'group' | 'service'
  name: string
  avatar?: string
  statusText?: string
  isOnline?: boolean
  isPinned?: boolean
  isMuted?: boolean
  unreadCount: number
  tags: string[]
  lastMessage?: {
    senderName?: string
    text: string
    timestamp: number
  }
  lastActiveAt: number
  children?: Entity[]
  childrenMode?: 'inline' | 'drilldown'
  childrenSections?: EntityChildrenSection[]
  drilldownDescription?: string
  source?: string
}
```

#### 当前实际被 UI 使用的字段

- `id`
  - 实体选择、树形查找、drilldown 路径、子项跳转都依赖它
- `type`
  - 决定头像图标、实体类型标签、Conversation header 图标
- `name`
  - 列表标题、详情标题、conversation header、breadcrumb
- `statusText`
  - 列表次级说明、drilldown 卡片状态、详情页状态、direct message 头部说明
- `isOnline`
  - 列表头像在线点、详情页在线状态
- `isPinned`
  - 列表 pin 图标、详情页 Pin / Unpin 动作文案
- `isMuted`
  - 列表 mute 图标、未读 badge 样式、详情页 Mute / Unmute 动作文案
- `unreadCount`
  - unread filter、列表 badge、子项 badge、drilldown unread 汇总
- `tags`
  - 详情页 tags 展示
- `lastMessage.senderName`
  - 列表摘要前缀
- `lastMessage.text`
  - 列表摘要、搜索匹配、drilldown 描述兜底
- `lastMessage.timestamp`
  - 列表时间、drilldown 行的时间信息
- `children`
  - inline children 和 drilldown children 的实际数据源
- `childrenMode`
  - 控制子实体是 inline 还是 drilldown
- `childrenSections`
  - drilldown 分组展示
- `drilldownDescription`
  - drilldown 总览卡片描述

#### 当前已定义但基本未被 UI 使用的字段

- `avatar`
  - 当前头像全部使用类型图标生成，没有实际渲染图片头像
- `lastActiveAt`
  - 当前列表时间取的是 `lastMessage.timestamp`，没有读这个字段
- `source`
  - `Entity` 上已定义，但当前 MessageHub UI 没直接读

### 4.2 EntityChildrenSection

定义位置：`src/app/messagehub/types.ts`

```ts
export interface EntityChildrenSection {
  id: string
  title: string
  description?: string
  childIds: string[]
}
```

#### 当前实际被 UI 使用的字段

- `id`
  - section key
- `title`
  - drilldown section 标题
- `description`
  - drilldown section 描述
- `childIds`
  - 用于把 section 配置映射到真实 `children`

### 4.3 Session

定义位置：`src/app/messagehub/types.ts`

```ts
export interface Session {
  id: string
  entityId: string
  title: string
  type: 'chat' | 'task' | 'workspace'
  source?: string
  isActive: boolean
  lastActiveAt: number
  unreadCount: number
}
```

#### 当前实际被 UI 使用的字段

- `id`
  - session 选择、active session 查找、消息 reader 路由
- `title`
  - session list 标题、conversation header 次级标题
- `type`
  - session 图标类型，区分 `chat` / `task` / `workspace`
- `source`
  - 当前只用来识别 `telegram` 和 `linear` 的图标表现
- `unreadCount`
  - session list 未读数量

#### 当前已定义但基本未被 UI 使用的字段

- `entityId`
  - 数据结构里有，但 UI 逻辑没有直接读取
- `isActive`
  - UI 自己根据 `activeSessionId` 判断活跃态，没有使用这个字段
- `lastActiveAt`
  - 当前 session list 没展示时间

### 4.4 EntityDetail

定义位置：`src/app/messagehub/types.ts`

```ts
export interface EntityDetail extends Entity {
  bio?: string
  bindings?: AccountBinding[]
  memberCount?: number
  note?: string
  createdAt?: number
}
```

#### 当前实际被 UI 使用的字段

继承 `Entity` 后，详情页额外读到：

- `bio`
- `bindings`
- `memberCount`
- `note`

继承自 `Entity` 且在详情页中继续使用的字段：

- `name`
- `type`
- `isOnline`
- `statusText`
- `tags`
- `isMuted`
- `isPinned`

#### 当前已定义但基本未被 UI 使用的字段

- `createdAt`
  - 详情页没有展示创建时间

### 4.5 AccountBinding

定义位置：`src/app/messagehub/types.ts`

```ts
export interface AccountBinding {
  platform: string
  accountId: string
  displayId: string
}
```

#### 当前实际被 UI 使用的字段

- `platform`
  - 详情页账号来源标签
- `accountId`
  - 当前只用于 React key
- `displayId`
  - 详情页展示值

---

## 5. 会话消息 UI Model

### 5.1 当前消息区直接消费 `MessageObject`

定义位置：`src/app/messagehub/protocol/msgobj.ts`

这层不是传统意义上的“前端 ViewModel”。

当前实现刻意让 UI 直接消费协议对象 `MessageObject`，只通过少量辅助函数读取 `ui_*` 元数据，而没有再映射成独立的 `ConversationMessageVM`。

### 5.2 MessageObject 当前实际被 UI 使用的字段

```ts
export interface MsgObject {
  from: DID
  to: DID[]
  kind: MsgObjKind
  thread?: TopicThread
  workspace?: DID
  created_at_ms: number
  expires_at_ms?: number
  nonce?: number
  content: MsgContent
  proof?: string
  [key: string]: unknown
}
```

#### 当前实际被渲染层读取的协议字段

- `from`
  - 判断是否为自己发送的消息
- `kind`
  - 状态消息识别时会参与判断
- `created_at_ms`
  - 消息时间、稳定 key 兜底、时间分隔投影
- `content.format`
  - 选择文本渲染、图片引用渲染、fallback 渲染
- `content.content`
  - 文本正文、图片 caption、fallback 内容
- `content.refs`
  - 图片引用解析

#### 当前实际被 UI 读取的扩展 meta 字段

- `ui_message_id`
  - 消息稳定 id
- `ui_sender_name`
  - sender 展示名
- `ui_delivery_status`
  - 发送状态图标
- `ui_session_id`
  - 用于 memory reader key 推导和消息归属
- `ui_item_kind`
  - 标记 status item
- `ui_status_type`
  - 标记 `typing` / `processing` / `disconnected` / `info`

### 5.3 MsgContent 当前实际使用情况

```ts
export interface MsgContent {
  title?: string
  format?: MsgContentFormat
  content: string
  machine?: MachineContent
  refs?: RefItem[]
}
```

#### 当前实际使用的字段

- `format`
- `content`
- `refs`

#### 当前已定义但未被 UI 使用的字段

- `title`
- `machine`

### 5.4 RefItem 当前实际使用情况

`refs` 当前只支持一个很窄的 UI 路径：图片引用。

#### 当前真正会被识别的结构

```ts
{
  role: 'input' | ...
  label?: string
  target: {
    type: 'data_obj'
    obj_id: string
    uri_hint?: string
  }
}
```

#### 当前实际被 UI 使用的字段

- `target.type`
  - 必须是 `data_obj`
- `target.uri_hint`
  - 必须是可识别的图片 URL
- `label`
  - 作为图片 alt 或链接文案

#### 当前未被 UI 使用的字段

- `role`
- `target.obj_id`
- `service_did` 型引用

### 5.5 当前渲染层实际支持的消息类型

#### 文本消息

- `content.format` 为：
  - `text/plain`
  - `text/markdown`
  - `text/html`

注意：

- 当前 markdown / html 只是按纯文本显示，没有做富文本渲染

#### 图片引用消息

- `content.refs` 中出现可识别图片 URL 时，会优先走图片渲染
- `content.content` 作为 caption

#### fallback 消息

- 其他格式统一走 fallback，展示 `format` 和原始 `content`

#### 状态消息

- `kind === 'notify'` 或 `ui_item_kind === 'status'`
- 展示为居中的状态 pill

---

## 6. Composer 草稿模型

### 6.1 发送 payload

定义位置：`src/app/messagehub/conversation/input/ConversationComposer.tsx`

```ts
export interface ConversationComposerSubmitPayload {
  attachments: ComposerAttachmentItem[]
  content: string
}
```

#### 当前实际语义

- `content`
  - 输入框中的文本内容，发送前会 `trim`
- `attachments`
  - 当前草稿中的文件或图片列表

### 6.2 附件项模型

定义位置：`src/app/messagehub/conversation/input/attachmentDraft.ts`

```ts
export interface ComposerAttachmentItem {
  id: string
  file: File
  relativePath?: string
  kind: 'image' | 'file'
  previewUrl?: string
}
```

#### 当前实际被 UI 使用的字段

- `id`
  - attachment card key、删除操作
- `file`
  - 文件名、大小、类型、预览图来源
- `relativePath`
  - 目录选择或拖拽目录时展示相对路径
- `kind`
  - 决定走图片卡片还是文件卡片
- `previewUrl`
  - 图片预览

### 6.3 当前草稿到消息内容的转换方式

当前 `MessageHubView` 并没有把附件真正编码进协议消息结构中。

发送时会把附件信息拼成一行 mock 文本：

- 无附件：只发送文本
- 有附件：把附件名称摘要拼进 `content`

这说明当前 composer 还停留在“UI 演示态”，未进入真实消息协议建模阶段。

---

## 7. 页面本地视图状态

### 7.1 `MessageHubState` 不是当前真实状态模型

定义位置：`src/app/messagehub/types.ts`

```ts
export interface MessageHubState {
  selectedEntityId: string | null
  selectedSessionId: string | null
  activeFilter: EntityFilter
  searchQuery: string
  mobileView: MobileView
  showSessionSidebar: boolean
  showDetails: boolean
}
```

当前代码没有实际引用这个接口。

### 7.2 当前 `MessageHubView` 真实维护的状态

定义位置：`src/app/messagehub/MessageHubView.tsx`

#### 业务状态

- `selectedEntityId`
- `selectedSessionId`
- `filter`
- `searchQuery`
- `mobileView`
- `showSessionSidebar`
- `showDetails`
- `entityListDrilldownPath`
- `localReaders`

#### 布局状态

- `entityListWidth`
- `sessionSidebarWidth`
- `isEntityListCollapsed`
- `isResizingEntityList`
- `isResizingSessionSidebar`

#### 结论

如果要定义“当前真实页面状态模型”，它至少应该包含：

- 选择态
- 搜索与过滤态
- mobile / desktop 视图态
- drilldown 导航态
- 分栏布局态
- 会话 reader 数据态

也就是说，当前 `MessageHubState` 只能覆盖一部分，不足以代表真实页面状态。

---

## 8. 当前实际使用字段总表

### 8.1 Entity

#### 已使用

- `id`
- `type`
- `name`
- `statusText`
- `isOnline`
- `isPinned`
- `isMuted`
- `unreadCount`
- `tags`
- `lastMessage.senderName`
- `lastMessage.text`
- `lastMessage.timestamp`
- `children`
- `childrenMode`
- `childrenSections`
- `drilldownDescription`

#### 暂未使用

- `avatar`
- `lastActiveAt`
- `source`

### 8.2 Session

#### 已使用

- `id`
- `title`
- `type`
- `source`
- `unreadCount`

#### 暂未使用

- `entityId`
- `isActive`
- `lastActiveAt`

### 8.3 EntityDetail

#### 已使用

- `bio`
- `bindings`
- `memberCount`
- `note`
- 以及继承自 `Entity` 的常用展示字段

#### 暂未使用

- `createdAt`

### 8.4 MessageObject

#### 已使用

- `from`
- `kind`
- `created_at_ms`
- `content.format`
- `content.content`
- `content.refs`
- `ui_message_id`
- `ui_sender_name`
- `ui_delivery_status`
- `ui_session_id`
- `ui_item_kind`
- `ui_status_type`

#### 暂未使用

- `to`
- `thread`
- `workspace`
- `expires_at_ms`
- `nonce`
- `proof`
- `content.title`
- `content.machine`

### 8.5 ComposerAttachmentItem

#### 已使用

- `id`
- `file`
- `relativePath`
- `kind`
- `previewUrl`

---

## 9. 当前模型存在的几个实现特征

### 9.1 页面级数据和协议消息数据是分裂的

当前列表和详情使用 `Entity` / `Session` / `EntityDetail`。

消息区使用 `MessageObject`。

两者之间没有统一的中间层。例如：

- 列表摘要来自 `Entity.lastMessage`
- 会话详情来自 `MessageObject[]`

这意味着列表摘要和真实消息历史目前是两套并行数据，而不是单一来源派生。

### 9.2 `MessageHubState` 落后于真实实现

类型层面定义了一个较小的页面状态接口，但真实页面状态已经扩展到：

- drilldown 导航
- panel 宽度
- panel 折叠
- reader 缓存
- resize 交互状态

### 9.3 Composer 仍处于 UI 原型阶段

附件在输入区里已经有独立数据结构，但发送时仍被降级为文本摘要，没有进入协议层 `MessageObject.content` 的规范建模。

### 9.4 `Entity.avatar` 预留了，但 UI 还没有进入真实头像阶段

当前所有头像都由 `type` 生成图标和颜色，说明头像字段仍是未来扩展位。

---

## 10. 建议的后续整理方向

如果后面要继续收敛 `MessageHub` 的 UI DataModel，建议优先做这 3 件事：

### 10.1 区分“当前生效字段”和“预留字段”

可以把 `Entity` / `Session` / `EntityDetail` 分成：

- 当前必须字段
- 当前可选但已消费字段
- 预留未消费字段

这样后续接后端时更容易知道哪些字段是真的接口契约。

### 10.2 把页面状态模型补齐

如果要保留 `MessageHubState`，建议把以下内容补进去，或者改名为更准确的页面状态类型：

- `entityListDrilldownPath`
- `entityListWidth`
- `sessionSidebarWidth`
- `isEntityListCollapsed`
- `localReaders` 或其引用键

### 10.3 明确消息附件的协议建模

当前附件只是 composer 内部模型，还没有进入真实消息模型。

如果后面要做真实发送，至少需要明确：

- 图片是否进入 `content.refs`
- 文件是否进入 `refs`
- 文本与附件如何组合
- 本地预览对象与协议对象如何映射

---

## 11. 一句话总结

当前 `MessageHub` 的“UI Model Data”本质上是一个分层组合模型：

- 页面框架层用 `Entity / Session / EntityDetail`
- 会话内容层直接用协议 `MessageObject`
- 输入层单独维护 `ComposerAttachmentItem`
- 页面状态层分散在 `MessageHubView` 的 React state 中

它已经具备原型验证所需的数据结构，但还没有收敛成一个统一、严格的前端数据模型。
