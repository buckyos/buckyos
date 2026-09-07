# MessageHub 当前 UI Model Data

- 版本：v0.4，2026-09-07（真实 msg-center store 落地；mock 原型保持不变）
- 范围：MessageHub mock 原型与真实 store、共享的 Conversation 组件、CodeAssistant 长历史 seed，以及 Agent 主页入口。
- 实现依据：`src/frame/desktop/src/app/messagehub/`，本文件只描述已落地字段。权威服务设计见 [UI_DATAMODEL](../../src/frame/desktop/src/app/messagehub/UI_DATAMODEL.md)。
- §1–§7 描述 mock 原型；§8 描述真实 store 的实际字段来源与仍未接入的服务能力。

## 1. 数据流与入口

组件通过 `store/index.ts` 的 `useMessageHubStore()` 取得 `MessageHubStore` 实现：`VITE_MESSAGEHUB_USE_MOCK`
显式覆盖，否则跟随桌面的 `VITE_CP_USE_MOCK`。mock 模式下 `mock/data.ts` 提供 DID 化的 Entity / Session seed，
`mock/store.ts` 的 `MessageHubMockStore` 持有唯一可变业务状态；真实模式见 §8。
`MessageHubView` 持有当前选择与布局，不再维护自己的 Session 或 reader 数组副本。

```text
mock seed + IndexedDB 持久差量
  → MessageHubMockStore
    → Entity 聚合 / Session 排序 / 详情 / 能力投影
    → 按 viewer + owner + Session 缓存的原始 reader
      → ConversationProjection（Action 可见过滤）
        → materialized window → 原有虚拟列表与 renderer
```

- `/messagehub` 默认用户视角，默认选中 CodeAssistant 实体；`entityId` 使用 DID。
- 明确传 `mode=observe&ownerDid=...` 才进入 Agent 观察视角。允许的 mock owner 为 CodeAssistant 与 Users & Agents seed 中的 `did:bns:assistant.alice`。
- 桌面 `MessageHubAppPanel` 消费 `launch.payload = { kind: 'messagehub', entityId, context }`，通过 Zod 校验；无效启动参数进入拒绝态。
- `AgentDetailPage` 的“与 Agent 对话”进入用户 owner；“查看 Agent 的会话”进入 Agent owner。桌面通过 `openAppWindow` 传相同 context，独立页面通过 URL 传递。
- 退出观察在当前模块内回到用户 owner，桌面窗口保持打开。紧凑移动桌面内嵌时，Panel 复用 shell 的状态栏高度与安全区，避免详情按钮被系统栏遮挡。

## 2. Entity 与 Session

`types.ts` 的 `Entity.id` 是 DID，父实体与子实体有独立 DID。原有 type、name、avatar、statusText、isOnline、isPinned、isMuted、tags、children、childrenMode、childrenSections、drilldownDescription、source 继续服务列表与实体详情。

| Entity 字段 | 实际来源与语义 |
|---|---|
| `sessionCreation` | `{ policy: 'default' | 'allow' | 'deny', canCreate, unavailableReason? }`，按 owner / entity 策略与可用连接能力派生 |
| `unreadCount` | 当前 owner 下该实体全部 Session 未读之和，包含归档；父子实体分别计数 |
| `lastActiveAt` | 同 owner / entity 的 Session 有效消息活动时间最大值；零会话为 0 |
| `lastMessage` | 最后活动 Session 的消息摘要，可为空；其 timestamp 不参与排序 |
| `EntityDetail` 扩展 | 原有 bio、bindings、memberCount、note、createdAt；子实体无额外详情 seed 时仍可用 Entity 基础字段显示详情 |

实际 `Session` 使用下列字段，不再包含 `isActive`：

```ts
interface Session {
  id: string
  ownerDid: string
  entityId: string
  binding: SessionBinding
  origin: 'manual' | 'connection' | 'remote_context' | 'unknown'
  lifecycle: 'active' | 'archived'
  type: 'chat' | 'task' | 'workspace'
  title: string
  source?: string
  createdAt: number
  lastActiveAt: number
  unreadCount: number
  lastMessage?: { senderName?: string; text: string; timestamp: number }
  shared: { title: string; description: string; updatedAt: number }
  members: Record<string, { nickname: string; updatedAt: number }>
}
```

`title` 是派生标题回落值；当前显示标题由 `sessionTitle()` 按“个人覆盖 → shared.title → title”计算。
创建标题只初始化 shared.title，不生成聊天消息，也不覆盖个人显示标题。

```ts
type SessionBinding =
  | { kind: 'native'; targetDid: string }
  | {
      kind: 'tunnel'
      tunnelInstanceId: string
      endpointDid: string
      remoteContextId?: string
      connectionName: string
      supportsMultipleSessions: boolean
      canCreateRemoteSession: boolean
      canSend: boolean
      connected: boolean
      revision?: number
    }
  | { kind: 'unknown' }
```

- 同一实体下的连接按实例与端点区分，同一实例多个远端上下文各有 Session。
- Alice seed 包含 Telegram Personal、Telegram Work 两个实例，Work 中含 General / Design 两个会话。
- 原生 Agent 可以直接创建。Person 默认禁止创建；显式允许仍需可用连接。多连接必须在表单中选择。
- 创建 tunnel Session 还要求 `supportsMultipleSessions && canCreateRemoteSession`；Personal seed 不满足此条件，Work 满足。
- `discoverConnection()` 按 owner、实例、端点和远端上下文幂等登记默认空 Session；重复发现和旧删除标记不会重新加载 seed。
- 删除保留连接来源，不删除联系人、父子实体或对端记录。连接失效后保留历史，禁止改用其它 tunnel 发送。

## 3. 消息活动时间、状态与权限

`sessionModel.ts` 集中实现活动类别判定、排序、标题、时间文案与有效权限。

| 事件 | lastActiveAt | 归档 |
|---|---|---|
| 创建空 Session | 初始化为创建时间 | active |
| 新 chat / group_msg | 与消息时间取 max | 恢复 active |
| 有正文或 output 引用的 deliver 结果 | 与消息时间取 max | 恢复 active |
| 迟到普通消息 | 不倒退；允许正常未读增长 | 恢复 active |
| 同一消息重放、投递 / 已读状态更新 | 不变 | 不变 |
| 共享 / 成员状态、对应 Action Log | 不变 | 不变 |
| typing / processing / active / statusLine | 不变 | 不变 |
| 个人标题、静音、归档 / 恢复、过滤、时钟 tick | 不变 | 仅显式生命周期动作改变 |

Session 排序为个人置顶优先、lastActiveAt 降序、稳定 ID 次序。Entity 使用其置顶和同口径聚合时间排序。
相对时间小于一分钟为 now / 刚刚，之后向下取整显示 m / h / d；未来差值按 0，未知值为 `—`。
相对时间订阅独立分钟时钟，不写业务 snapshot、不重建 reader。runtime 每秒检查到期，控制时钟注入可立即过期。

```ts
interface MessageHubContext {
  viewerDid: string
  ownerDid: string
  mode: 'self' | 'observe'
}
interface RuntimeState {
  memberDid: string
  status: 'typing' | 'processing' | 'active'
  statusLine?: string
  expiresAt: number
}
interface SessionAccess {
  mode: 'read_only' | 'read_write'
  canManage: boolean
  canEnableWrite: boolean
  canEditPresentation: boolean
  canEditSharedState: boolean
  canEditOwnMemberState: boolean
  readOnlyReason?: string
}
```

原生用户会话可写。tunnel 默认只读；有能力时在 SessionDetails 明确确认历史不一致风险后启用写入。
这里的原生可写是当前 mock 规则：native 分支没有真实路由 / 群权限检查，且默认允许共享 / 自己成员状态编辑。
实际 `SessionAccess` 没有 canRead/canSend 字段；目标能力见 UI_DATAMODEL §3.3.4，不能把 mock 判断用于真实授权。
确认仅记录在视图内存，与当前绑定序列化值对应；连接 revision 变化、刷新和 owner 切换使旧确认失效。
发送动作在提交执行时重新检查权限和 binding，风险确认不授予共享或成员状态编辑权限。
所有输入、粘贴 / 拖拽文件、Enter 发送与失败重试共用可写 Composer 和 store 权限检查。

Agent 观察模式可看两种详情，不能创建、发送、归档 / 删除、改共享状态、昵称、Agent 草稿和个人标题等配置。
观察者可以修改自己的 Action 可见性。拒绝态不返回其它 owner 的 reader。未读聚合始终按 owner，观察未读不进入用户聚合。

当前 store 仅在有效普通入站消息追加时增加未读，没有真实 mailbox 已读 / receipt 写入流程。
因此上述 mock 时间和权限场景不能验证后端对所有 UNREAD 记录的统计，也不能验证跨端回执。

## 4. 持久状态、草稿和生命周期

`Snapshot` 实际保存以下字典，存放在 `messagehub-prototype-v1` IndexedDB 的 `state` object store 中。
修改通过串行异步事务提交，成功后一次性发布 snapshot；失败不改变已发布数据。组件只持有输入与请求进度。

| 字典 | 键 | 值 |
|---|---|---|
| `sessions` | JSON `[ownerDid, sessionId]` | Session |
| `deleted` | 同上 | 删除时间水位与已清理共享状态 / 摘要的来源元数据 |
| `withoutSeed` | 同上 | 删除后新消息重现也不再加载旧 seed |
| `messages` | 同上 | 本轮新增的原始 MsgObject 差量，包含 Action Log |
| `delivery` | 同上 | 消息 ID → delivery status 的展示覆盖，不改变活动时间 |
| `policies` | JSON `[ownerDid, entityDid]` | default / allow / deny |
| `preferences` | JSON `[viewerDid, ownerDid, sessionId]` | SessionPreferences |
| `drafts` | 同上 | 未发送正文 |
| `draftAttachments` | 同上 | `{ file: File, relativePath?: string }[]`，通过 IndexedDB 结构化克隆保存 |

```ts
interface SessionPreferences {
  title: string
  pinned: boolean
  muted: boolean
  showActions: boolean
}
```

个人偏好默认 `{ title: '', pinned: false, muted: false, showActions: true }`。
长 seed 继续使用 `buckyos-mock-message-history` 中的原有 IndexedDB reader；不复制进 localStorage。
当前 reader 将只读 seed 与本轮消息差量组合，按 viewer / owner / Session 缓存；标题、状态、时钟和偏好更新不会替换其原始历史。

归档保留历史、未读和文本 / 附件草稿。恢复沿用原活动时间。彻底删除移除本 owner 的记录、差量历史引用、投递覆盖、草稿与各 viewer 的 Session 偏好。
删除水位之前的历史消息和重复 seed 不能复活内容；连接仍可用时，水位之后的新普通消息可以重新建立可见 Session，但只能读到新历史。
这是本地 mock 生命周期语义，不是对远端数据的物理删除承诺。

## 5. 表单、详情与交互状态

输入使用 react-hook-form 与 Zod；schema 是输入约束的源码依据。

| Schema | 字段 / 约束 |
|---|---|
| `createSessionSchema` | entityId 非空；title trim 后最多 64 字符；connection 非空；提交时再检查策略与连接能力 |
| `sharedStateSchema` | title trim 后最多 64 字符；description trim 后最多 500 字符；store 拒绝额外字段 |
| `memberStateSchema` | nickname trim 后最多 64 字符；UI 只提交自己的昵称，store 拒绝角色等额外字段 |
| `presentationSchema` | title trim 后最多 64 字符，pinned / muted 为 boolean；不生成共享日志 |
| `policySchema` | EntityDetails 内 default / allow / deny 枚举 |
| `messageHubLaunchSchema` | kind=messagehub，entityId 为 DID 或 null，context DID 与 mode 必须有效 |

同名标题允许重复，每次创建使用独立 UUID。取消不创建；失败保留表单值。提交具有进度与重复提交锁。
创建成功先登记空 Session / 空 reader，再选择并打开；发送第一条消息继续使用同一 ID。
取消一个已经提交的慢请求时，该登记可在原 context 完成，但迟到结果不能更改当前实体 / owner / Session 选择。

`MessageHubView` 选择和布局为 React state：selectedEntityId、selectedSessionId、filter、searchQuery、mobileView、showSessionSidebar、archived、detailsTarget、实体下钻路径、面板宽度与拖动状态。
`detailsTarget` 明确区分 entity / session；SessionDetails 始终使用当前有效选择，没有会话时入口禁用。
移动详情覆盖仍挂载的 Conversation，保留滚动与草稿。归档 / 删除当前会话后转入剩余有效会话或正常空态；处理其它行不切换当前会话。

| 场景 | 显示状态 |
|---|---|
| 首次读取 | loading；失败显示错误与 retry |
| 可写空会话 | 开始对话，Composer 可用 |
| 只读空会话 | 暂无消息及原因 |
| 无 Session | 尚无会话；保留 Sessions / 已归档 / 按能力创建入口 |
| 保存 / 删除 / 创建 / 发送 | pending，阻止重复请求；失败保留原数据 / 表单 / 草稿，可重试 |
| 保存成功 | 详情显示 Saved / 已保存；创建与管理关闭对话框并更新视图 |
| 权限拒绝 | 独立拒绝态，没有历史缓存回落 |

Session 行包含来源、截断标题、状态提示、未读、固定宽度时间与独立操作按钮；无右侧选中竖条，无嵌套 button。
hover、focus-within 和触屏均能访问处理入口。对话框有 Tab 焦点约束与取消焦点恢复，删除不作为默认回车动作。

## 6. Action Message 与历史投影

原始 `MessageObject` 仍使用现有 `protocol/msgobj.ts`，本轮未改 Rust 协议镜像。

类别判定只使用 `kind === 'event' && content.machine?.intent === 'buckyos.action_log'`，渲染与过滤共用同一函数。
读取 data 前检查 schema_version；支持 title / shared state / member nickname 变化、加入、主动离开和被移除；actor 和 subject 分开，未知操作者不猜测。
不支持的版本 / 动作回落为摘要，载荷不能执行。状态修改无变化或失败不生成日志；成功修改与日志在一个 mock snapshot 事务提交。

`ConversationProjection` 在原字段外增加 `showActions`。`messageCount` 始终为 raw reader 消息数，entries 内 `messageIndex` 始终为原始索引。
过滤只移除 Action entries，重新构建日期、可见 totalCount 和物化范围。普通 event 仍显示；全过滤显示专门提示。
增量追加沿用此规则，不产生虚拟空行或孤立日期。过滤切换尽量保持仍可见的消息锚点，恢复显示不会丢失消息。

## 7. mock 验证入口与后续集成

仅开发环境暴露 `window.__messageHubMock`：

- `configure({ delayMs, failNext, now })`：控制请求、单次失败与时钟。
- `injectRuntime(owner, session, state)`：只接受已知 member，带 expiresAt；不产生 Action Log。
- `injectState(owner, session, { shared?, member?, actorDid? })`：模拟远端共享 / 昵称更新与日志。
- `injectMessage(owner, session, msg)`、`injectDelivery(owner, session, messageId, status)`：普通内容、日志和投递边界。
- `discoverConnection(owner, entity, binding)`、`setConnection(owner, session, patch)`：连接发现、幂等、失效与 revision。
- `denyOwner(owner)`：验证拒绝态；不会回落到用户历史。

组件使用的 create、manage、updateState、updatePreferences、setPolicy、saveDraft、saveAttachments、send 全为 mock 方法，不冒充 KRPC。
真实集成仍需独立消息活动时间和跨页排序游标、空会话登记、权威状态版本和可靠日志、Session 级归档 / 删除、共享对象引用处理、平台能力与服务端代理授权。

### 7.1 真实 API 适配（2026-09-07 起为 §8 的真实 store，原缺口表已作废）

`datamodel/sessionApi.ts` 现覆盖 `msg.*`（含会话登记 / 生命周期）、`ui_session.*`（owner 范围）、`contact.*`、`group.*`；
`conversation/history/sessionApiReader.ts` 已由 `api/reader.ts` 取代。

## 8. 真实 store（`api/store.ts`）实际字段来源

| UI 字段 | 来源 |
|---|---|
| `Entity`（含 `sources` / `domain` / `sessionCount` / `requestCount`） | `msg.list_sessions(lifecycle: all, order_by: activity, with_object)` 归属出的对端 + `contact.list_contacts`（owner 作用域与系统作用域合并）+ `group.list_by_member` + `agent.list`；zone 托管 `did:web:*.<zone>` 识别为 Agent；无法归属的会话进入 `messagehub:unassigned` 容器（非 DID，不可发送） |
| `Session.entityId` / `binding` / `origin` / `attributionEvidence` | `api/projection.ts::attributeSession`：登记 peer → `group:` tag / group_msg 目标 → `dm:` → 原始 msg 端点 → 记录字段；`did:msgtunnel:*` 解析为 tunnel 绑定（实例、端点、远端上下文），其余 native |
| `Session.lifecycle` / `lastActiveAt` / `unreadCount` / `requestCount` / `createdAt` | `SessionSummary.lifecycle` / `last_activity_ms` / `unread_count` / `request_count` / `state.created_at_ms` |
| `Session.title` / `shared.title` | 个人 `ui.title`（owner 范围）→ 登记标题 → `thread.topic` → 私聊 / 群名 / 连接名 → 未命名 |
| `Session.members` / 共享说明 | 无后端契约，保持空；详情中的共享 / 成员编辑禁用并说明 |
| `MessageObject.ui_record` | `SessionMessageItem` 的 owner / record_id / msg_id / direction / box_kind / sort_key / recipient_state / delivery；无对象时 `ui_unavailable` 占位 |
| 已读 | 可见入站 UNREAD 记录 → `msg.update_record_state(READ)`，成功后本地减少并重读摘要 |
| 发送 | `msg.post_send`，`to` 来自绑定，登记 / topic 会话带 `thread.topic`；附件经 NDM TUS 上传 + `put_object` 后写入 `refs[].target.data_obj` |
| 附件读取 | `GET /kapi/msg-center/objects/{obj_id}[/content]`（Bearer 会话 token），Blob URL 缓存 |
| 生命周期 | `msg.archive_session` / `msg.restore_session` / `msg.delete_session`（删除同时清本地草稿 / 偏好 / reader） |
| 创建 | `msg.create_session`（native 绑定；tunnel 创建因无能力声明始终 `platform_read_only`） |
| 偏好 | `ui.title` / `ui.pinned` / `ui.muted` → `ui_session.update_state(owner)`；`showActions`、草稿、创建策略为 viewer 本地 IndexedDB |
| 运行态 | `ui_session.list_state(session_id)`（无 owner 的旧 KV）：typing 30s、status_line 10min 内有效，成员固定为 owner |
| 权限 | 观察者全部只读；tunnel 默认只读 + 风险确认；群按 `group.check_access(group.post_message)`；native 由 `post_send` 校验 |
| 刷新 | 20s 轮询摘要与当前会话尾页；kevent `/msg_center/<owner>/box_*_<owner>/changed` 作为额外信号 |

仍未接入：批量已读 / 回执持久化、接受后请求记录迁移、共享 / 成员状态权威与 Action Log 发布、tunnel 能力声明与代发、
已加载尾页之外的旧记录变更游标、`ui_session` owner 范围批量读取。验收命令、场景和截图见
[原型交付说明](../../proposals/messagehub-ui-prototype/IMPLEMENTATION.md)。
