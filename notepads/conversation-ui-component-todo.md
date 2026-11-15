# 会话 UI 公共组件化 TODO

> 状态：立项，未实施；§5 的 Q1–Q9 待裁决。现状核实于 2026-11-15（buckyos `e1a44183e`、buckyos-websdk `03d2dde`），下文行号以这两个提交为准。
>
> 起因：Agent 浏览器（`e1a44183e`）要在自己的界面里显示和 Agent 的对话。系统里没有能嵌入的会话组件，于是它把 MessageHub 的会话接线复制了一份（[ThingConversation.tsx](../src/frame/desktop/src/app/agent-browser/conversation/ThingConversation.tsx)），这已经是会话 UI 的第四份拼装（§1.1）。
>
> 目标：会话 UI 只有一份实现，放在 `src/frame/desktop/src/components/conversation/`。Desktop 里的系统 App 直接用 React 组件；第三方 App 在自己的页面里嵌入 Zone 提供的系统页，系统页渲染的是同一个组件。
>
> 参照：
> - [Network 公共组件 UI DataModel](../src/frame/desktop/src/components/network/UI_DATAMODEL.md) §1：带数据层的公共组件，Desktop 内和第三方两种用法。
> - [系统对话框实现方案](../doc/desktop/BuckyOS%20系统对话框实现方案.md) §3、§8、§9：三条调用路径、内嵌系统页、信任模型。
> - websdk `src/app-frame.ts` 的 `buckyos.preview-embed`，Text Editor 已经在用。
> - [Agent Browser Protocol](../doc/opendan/protocol/Agent%20Browser%20Protocol.md) §1：会话和标签组怎么对应。

## 0. 范围

组件是**一个会话的正文**，包括：

- 置顶面板和自定义会话面板（`SessionPanel`）
- 历史（`ConversationHistoryPane`，以及各类消息的渲染、媒体、任务卡、工作会话卡）
- 运行状态行
- 阻塞条和 Turn 控制（`TurnControls`）
- 输入框（`ConversationComposer`）：草稿、附件、语音、表情、回复、编辑、回答 `ask_user`、@
- 拖放附件、会话内搜索（都是可选项）

下面这些不属于组件，由宿主自己画：会话头部（返回、会话列表按钮、实体名、⋮ 菜单）、实体列表、会话侧栏、Agent 面板、详情页、转发目标选择器。组件通过回调把这些动作交给宿主。宿主没给回调的动作就不出现，这是 `ConversationView` 现在已有的规则。

本 TODO 不做：

- 不改 msg-center 协议。Q5 的服务端筛选如果要做，单独立项。
- 不支持 Zone 以外的站点嵌入。
- 不改 MessageHub 自己的界面和行为。阶段一完成后，MessageHub 的 e2e 不改用例也要全部通过。

## 1. 现状

### 1.1 四份拼装

| 位置 | 怎么拼的 | 和 MessageHub 的差异 |
|---|---|---|
| MessageHub：[ConversationView.tsx](../src/frame/desktop/src/app/messagehub/ConversationView.tsx)（530 行），加上 [MessageHubView.tsx:487-534](../src/frame/desktop/src/app/messagehub/MessageHubView.tsx#L487-L534) 的 `toOutgoing` 和 `conversationProps` | 这是基准。数据来自 `MessageHubStore`；`ConversationView` 自己组装输入状态和前台 Turn（:177-189）、消息操作（:204-269）、阻塞条和输入框（:459-480） | — |
| Agent 浏览器：[ThingConversation.tsx](../src/frame/desktop/src/app/agent-browser/conversation/ThingConversation.tsx)（237 行） | 复制了输入状态、前台 Turn、阻塞条和消息操作。`toOutgoing`（:35）和 MessageHubView:487 逐字相同。`WindowDialogProvider`（:130）、`MessageMediaHost` 要自己包，`messagehub.css`（:11）要自己引 | 没有编辑（:107 写死 `edit: false`）、置顶、转发、消息详情、已读回执、拖放、搜索，也没有归档中的阻塞提示 |
| 访客页：[GuestRoute.tsx:223](../src/frame/desktop/src/app/messagehub/guest/GuestRoute.tsx#L223) 的 `GuestConversation` | 自己拼 HistoryPane 和 Composer；数据来自 guest hub，不是 `MessageHubStore` | 只有群会话，没有 Turn |
| CodeAssistant：[CodeAssistantAppPanel.tsx](../src/frame/desktop/src/app/codeassistant/CodeAssistantAppPanel.tsx) | 直接用 `ConversationView`，数据是 InMemory reader 加上 MessageHub 的 mock 数据 | 只在 mock 应用清单里（`src/mock/data.ts:201`），不接真实数据 |

结果是 MessageHub 改了会话行为，另外几份不会跟着变。比如 11-11 owner 定下"Turn 的停止、撤回、恢复只放在输入框上方"，现在这类改动得在 `ConversationView` 和 `ThingConversation` 各改一遍。

### 1.2 反向依赖

MessageHub 的 [AgentSessionPanel.tsx:8](../src/frame/desktop/src/app/messagehub/agentpanel/AgentSessionPanel.tsx#L8)（使用处在 :93）直接引用了 Agent 浏览器的 `AgentBrowserEntry`；Agent 浏览器反过来引用 MessageHub 十几个内部模块。目前还没有形成循环引用，但双方的内部路径都已经成了对方实际依赖的接口。

### 1.3 `messagehub/conversation/` 对 MessageHub 其他部分的依赖

`conversation/` 子树约 1.14 万行（含 1856 行的 `emojiData.ts`）。搬进 `components/` 之前，要先切断下面这些引用：

| 引用 | 用在哪 | 处理（§3） |
|---|---|---|
| `protocol/msgobj`（22 处）、`sessionModel`（8 处）、`friendNotice`、`groupModel` 的解析函数 | 渲染、搜索、任务卡 | 都是纯函数，随组件一起搬走（C1） |
| `store`：`useMessageHubClock`、`isMessageHubMock`、`store/outgoing`、`store/types` 的 `OutgoingUpload` | `MessageTask`；tasks、worklog、worksessions 判断是否用 mock；渲染上传中的气泡 | 时钟和上传状态改从 source 取；判断 mock 改用 `isMockRuntime`（C2） |
| `agentpanel/source`（停止、恢复）、`confirmDialog` | `TurnControls`、`WorkControls` | Agent 控制改成 source 的可选能力（C2） |
| `notify/NotifyCard`、`AgentGroupSettingsLink`、`relation` | `renderers.tsx` | 改为按消息类型注册的渲染插槽，由 MessageHub 注册（C3） |
| `SessionDialogs` 的 `DialogFocus` | `MediaSettingsDialog` | 下沉到组件里 |
| `mock/tasks`、`mock/worklog`、`mock/worksessions` | 各 source 的 mock 分支 | 随组件搬到 `components/conversation/mock/` |

`components/` 里已经有引用 `app/` 的先例：[saveToZone.ts](../src/frame/desktop/src/components/saveToZone.ts) 和 [preview/nfspProvider.ts](../src/frame/desktop/src/components/preview/nfspProvider.ts) 都引了 filebrowser。但会话组件如果再引回 MessageHub，就会形成新的循环。所以本 TODO 要求 `components/conversation/` 不引用 `app/messagehub/`，唯一的例外是 Q1 的注册点。

### 1.4 第三方 App 现在已经能用到什么

- **两个嵌入先例。** Preview 嵌入（`buckyos.preview-embed`，`app-frame.ts:269`）和选择器嵌入（`buckyos.sysdlg-embed`，`app-frame.ts:401`）都是"App 页面里框一个系统页"的做法：nonce 握手，系统页用自己在 Zone 根域的会话工作，消息里不带 token，只接受本 Zone 内的嵌入方（`originInZone`）。Desktop 在 [main.tsx:80](../src/frame/desktop/src/main.tsx#L80)、[:91](../src/frame/desktop/src/main.tsx#L91) 为这两个页面分出只渲染组件的路由。Text Editor 已经在用 Preview 嵌入（[App.tsx:34](../src/apps/text_editor/src/ui/App.tsx#L34)）。
- **msg-center 对 App token 的处理。** 非 Agent 的 App token 被当作 token 所属用户本人（[owner_session.rs:808](../src/frame/msg_center/src/owner_session.rs#L808)）。也就是说，用户登录过的 Web App 本来就能读写这个用户的会话，和系统对话框方案 §9 描述的 nfs_server 情况一样。msg-center 的其他入口是否都这样处理，还没有逐个核实（E7）。

## 2. 方案

### 2.1 分层

```text
msg-center / task-mgr / Agent Service
  └─ MessageHubStore            app/messagehub/store：单例，位置不动（同步、草稿、发送中的气泡只有一份）
      └─ ConversationSource     app/messagehub/conversationSource.ts：把 store + context + sessionId 收窄成组件需要的接口
          └─ components/conversation/
              ├─ ConversationSurface    会话正文（§0），只依赖 ConversationSource
              ├─ SessionConversation    已绑定 MessageHub store 的便捷组件，传 sessionId 就能用
              ├─ agentSessions.ts       Agent 会话的通用操作：默认会话、带来源标记新建、按来源列出
              └─ 各部件                 HistoryPane、Composer、TurnControls、SearchBar、MessageMediaHost …
宿主
  ├─ MessageHub     ConversationView = 头部 + ConversationSurface
  ├─ 系统 App       <SessionConversation …/>（Agent 浏览器，以及以后的其他 App）
  ├─ 访客页         ConversationSurface + guest source
  └─ 第三方 App     websdk ConversationEmbed → /embed/conversation 系统页 → SessionConversation
```

`ConversationSource` 的草案如下。实施时只收 `ConversationView` 和 `ThingConversation` 实际用到的部分，不新增能力：

```ts
interface ConversationSource {
  sessionId: string
  selfDid: DID
  subscribe(listener: () => void): () => void
  reader: ConversationMessageReader
  historyStatus: 'idle' | 'loading' | 'ready' | 'error'
  hasOlder: boolean
  loadOlder(): Promise<boolean>
  markRead(recordIds: string[]): void
  access: SessionAccess
  send(payload: OutgoingPayload): Promise<void>
  stageOutgoing(queued: OutgoingPayload[]): void
  resend?(message: MessageObject): Promise<void>
  remove?(message: MessageObject): Promise<void>
  draft: { text: string; attachments: ComposerAttachmentInput[]; saveText(value: string): Promise<void> | undefined; saveAttachments(list: ComposerAttachmentInput[]): Promise<void> | undefined }
  displayName(did: DID): string
  runtime: RuntimeState[]
  preferences: { showActions: boolean; pinned: PinnedMessage | null; setPinned?(pinned: PinnedMessage | null): Promise<void> }
  /** 对方是 viewer 自己的 Agent 时才有：Turn 控制、撤回输入、停止回答 */
  agent?: { did: DID; name: string; stop(sessionId: string, scope: 'turn'): Promise<void> }
  /** 群会话才有：@ 候选、编辑和撤回的时间窗、已读回执、群通知卡 */
  group?: GroupConversationRules
}
```

`ConversationSurface` 的 props 包括：`source`、`features`（搜索、拖放），以及宿主回调 `onOpenMessage`、`onForward`、`onOpenEntity`、`onOpenProfile`，再加上 `emptyLabel`、`placeholder`、`sendBlockedReason`、`focusMessageId`。

### 2.2 系统 App 的用法（Desktop 内）

```tsx
<SessionConversation sessionId={id} placeholder={…} emptyLabel={…} />
```

- 组件自带 `WindowDialogProvider`（外层已有就复用外层的）、`MessageMediaHost` 和样式，宿主不用再自己包一层。
- Agent 会话怎么建立：`agentSessions.ts` 提供 `ensureDefaultSession(agentDid)`、`createSourcedSession(agentDid, source, title)` 和 `isSourced(sessionId, source)`。这些就是 Agent 浏览器 [session.ts](../src/frame/desktop/src/app/agent-browser/conversation/session.ts) 和 [marks.ts](../src/frame/desktop/src/app/agent-browser/conversation/marks.ts) 里与浏览器无关的那部分。来源标记继续用 `ui.source`（已登记在 [UI_DATAMODEL.md:1375](../src/frame/desktop/src/app/messagehub/UI_DATAMODEL.md#L1375)），取值改为 App 的 id。浏览器特有的部分（`bind_default`、标签组换算、交接消息）留在 agent-browser。
- AI Workspace 独立页（`aiws.html`，在自己的源上）和 Desktop 是同一个构建。它能直接用这个组件还是得走嵌入，取决于它在自己源上的会话能不能访问 msg-center，这一点没有核实。

### 2.3 第三方 App 的用法（内嵌系统页）

做法和 Preview 嵌入一样：App 在自己的布局里放一个 iframe，指向 Zone 系统入口的 `/embed/conversation`。系统页只渲染 `SessionConversation`，不加载桌面外壳。无论 App 运行在桌面窗口里（App Frame）还是独立标签页里，用法都相同。会话是常驻面板而不是弹框，所以不需要系统对话框方案里那条"Shell 代画"的路径 B。

协议名 `buckyos.conversation-embed` v1，握手照搬 `buckyos.preview-embed`（URL 片段 `#bce=<nonce>&po=<origin>`，消息 `embed.hello` / `embed.init`）：

| type | 方向 | payload |
|---|---|---|
| `embed.init` | App → 系统页 | `{ nonce, target, theme, locale, placeholder? }`，`target` 见下 |
| `embed.update` | App → 系统页 | `{ theme?, locale? }` |
| `embed.send` | App → 系统页 | `{ text, attachments?: TransferableContentRef[] }`：以用户的名义发一条消息（Q3） |
| `embed.state` | 系统页 → App | `{ status: 'loading' \| 'ready' \| 'error', sessionId?, error? }` |
| `embed.turn` | 系统页 → App | `{ mode }`：取值同 `foregroundState` 和回答状态，App 可以据此调整自己的界面（Q4） |
| `embed.error` | 系统页 → App | `{ code }`：`invalid_params`、`not_found`、`denied` |

`target` 有两种：

- `{ kind: 'agent', agentDid? }`：和这个 Agent 的、由本 App 建立的会话。还没有时，第一次发消息时新建，并打上 `ui.source = <appid>`。`agentDid` 不填就用 owner 的默认 Agent。
- `{ kind: 'session', sessionId }`：打开指定的会话。是否允许由 Q2 决定。

规则：

1. 系统页只接受本 Zone 内的嵌入方。App 的 id 由系统页根据嵌入方的 origin 推出，不采信 App 自报的值；这个 id 就是 `ui.source` 的取值。origin 换成 App id 的规则要核实，现在 [SelectEmbedApp.tsx](../src/frame/desktop/src/sysdlg/select/SelectEmbedApp.tsx) 只显示 origin。
2. 消息里不带 token 和 Cookie，也不把消息内容回传给 App。App 需要内容就用自己的会话去读（§1.4）。
3. 已知限制和 sysdlg §8 相同：Zone 根域没有会话时，iframe 里会出现登录页；App 可以遮挡或覆盖这个 iframe。

websdk 这一侧，`ConversationEmbed`（App 端）和 `ConversationEmbedHost`（系统端）放在 `src/app-frame.ts`，和 `PreviewEmbed` 并列。Desktop 的 `main.tsx` 增加 `/embed/conversation` 路由，写法同 :80、:91 那两个。

## 3. TODO

### 3.1 阶段一：组件落地，MessageHub 改用（不改行为）

- [ ] **C1 搬家。** 把 `app/messagehub/conversation/` 整体移到 `components/conversation/`；会话要用到的纯模型（`protocol/msgobj`、`sessionModel` 里用到的函数、`friendNotice`、`groupModel` 的解析函数）一起搬过去，MessageHub 的其他文件改为从新位置引用。这一步只搬家、不改代码，单独提交，方便 review 和 bisect。
- [ ] **C2 切断对 store 和 agentpanel 的引用**（§1.3 第 2、3 行）。时钟、上传状态、Agent 的停止和恢复改从 `ConversationSource` 取；tasks、worklog、worksessions 判断 mock 改用 `isMockRuntime`，和 `components/network/backend.ts` 的做法一致。`confirmDialog` 如果和 MessageHub 无关，就一起下沉。
- [ ] **C3 渲染插槽。** `renderers.tsx` 对通知卡、群设置链接、好友关系的特化，改成按消息类型注册的渲染器，由 MessageHub 在启动时注册。其他宿主不注册时，这几类消息按普通消息显示。
- [ ] **C4 抽出 `ConversationSurface`。** 把 `ConversationView` 里头部以下的部分（§0 列的那些）挪进去，`ConversationView` 只剩头部加 Surface。两份 `toOutgoing` 合成一份。消息操作的组装（`ConversationView.tsx:204-269`）变成一个函数，按 source 的能力和宿主给的回调决定开哪些操作。
- [ ] **C5 `ConversationSource` 和 MessageHub 的适配。** 适配放在 `app/messagehub/conversationSource.ts`；MessageHubView 的 `conversationProps` 里只剩宿主相关的部分（打开详情、转发、打开实体、归档中的提示）。
- [ ] **C6 `SessionConversation`。** 已绑定 store 的便捷组件，按 Q1 取得 store；自带 provider 和样式（§2.2）。
- [ ] **C7 样式。** `messagehub.css` 里会话用到的类拆到 `components/conversation/conversation.css`，由组件自己引入。i18n 键见 Q6。
- [ ] **C8 依赖方向检查。** 在 `eslint.config.js` 里对 `components/conversation/**` 加 `no-restricted-imports`，禁止引用 `app/**`（Q1 的注册点除外），让 `pnpm lint` 来守这条规则。
- [ ] **C9 回归。** 先跑一遍基线并记下已经失败的用例；之后 `tests/e2e/pages/` 下的 `messagehub*.spec.ts`（18 个文件，含访客页）和 `agent-browser.conversation.spec.ts` 在不改用例的情况下通过。

### 3.2 阶段二：系统 App 改用

- [ ] **S1 Agent 浏览器。** `ThingConversation` 里的 `SessionConversation`（:46）换成组件，`ConversationSheet` 和 `ConversationPanel` 不用改。能力补齐的范围按 Q7。
- [ ] **S2 提取 `agentSessions.ts`**（§2.2）。agent-browser 的 `session.ts` 只留 `bind_default` 和交接消息；`marks.ts` 挪进通用模块，来源取值参数化。按来源列会话的开销见 Q5。
- [ ] **S3 访客页。** `GuestConversation` 改成 `ConversationSurface` 加 guest source。这一步顺带验证 `ConversationSource` 没有被做成 `MessageHubStore` 的形状。
- [ ] **S4 CodeAssistant**：按 Q8 处理。
- [ ] **S5 Agent 面板的入口插槽。** `AgentSessionPanel` 去掉对 agent-browser 的直接引用，改成一张注册表（比如 `registerAgentSessionEntry`），Agent 浏览器在应用登记时把 `AgentBrowserEntry` 注册进去。这样 MessageHub 不再需要知道有哪些 App 往 Agent 面板里放东西。

### 3.3 阶段三：第三方 App 嵌入

- [ ] **E1 系统页 `/embed/conversation`**（`ConversationEmbedApp`）。嵌入方校验同 `SelectEmbedApp` 的 `acceptEmbedder`，包括 mock 运行时下放行 localhost；origin 换成 App id 的规则先核实（§2.3 第 1 条）；`target` 的参数过 zod schema。
- [ ] **E2 websdk。** 实现 `ConversationEmbed` 和 `ConversationEmbedHost`，导出类型并补单测，测试写法参照 `tests/preview_embed.test.ts`。改动涉及 websdk 的 `app-frame` 导出，`buckyos-build.py` 会从源码重新构建 SDK。
- [ ] **E3 窄屏。** 系统页在 360px 宽度和软键盘弹出时都要能用，iframe 里的 `visualViewport` 在 iOS 上的表现要实测。按产品原则以手机为准评估。
- [ ] **E4 第一个真实调用方**，按 Q9。
- [ ] **E5 文档和手册。** 在 doc/sdk 下补一节嵌入会话的用法。[`buckyos_app_dev`](../src/apps/jarvis_agent/agent/cmd_skills/buckyos_app_dev/SKILL.md) 补一句：要和 Agent 对话就嵌入系统会话页，不要自己连 msg-center 拼界面。这属于 AGENTS 里的 cmd_skills 联动，要记录并验证。
- [ ] **E6 mock 运行时。** 系统页在 mock 下可用，补一个 e2e：用一个假的 App 页面嵌入它，完成发送、回答和停止。
- [ ] **E7 核实 §1.4 的前提。** 逐个检查 msg-center 的发送、会话列表、`ui_session.*` 对 App token 的处理。如果有入口不把 App 当作用户本人，就回头修改 §2.3 第 2 条和 Q2、Q3 的依据。

## 4. 不做和以后再做

- 不做 Shell 代画的会话面板（系统对话框方案的路径 B）。会话是常驻面板，App 自己的布局决定它放在哪里。
- 不把 `MessageHubStore` 搬出 `app/messagehub/`（Q1 选项 b）。
- 不通过嵌入协议向 App 提供消息内容或消息事件订阅。
- 本轮不改 i18n 键名（Q6）。

## 5. 待裁决

| # | 问题 | 选项 | 建议 |
|---|---|---|---|
| Q1 | `SessionConversation` 怎么拿到 MessageHub 的 store | a. 在 `components/conversation/backend.ts` 里动态 import `app/messagehub/store`，做法同 `components/network/backend.ts` 的注册模式，`preview/nfspProvider.ts` 引 filebrowser 是先例；b. 把 MessageHub 的数据层（store、api、mock、types，约 5 千行）搬到 `src/api/messagehub/`，组件和 MessageHub 都从那里引用 | a。改动最小；等出现第二个数据层消费者时再做 b |
| Q2 | 第三方 App 能打开哪些会话 | a. 只能打开自己建的（`ui.source` 等于自己的 appid）或者新建；b. 再加上用户和 Agent 的默认会话；c. 任意 `sessionId` | a。按 §1.4，App 本来就能读这些会话，所以这不是数据边界，而是"App 页面里显示用户的哪些对话"。选 a，App 页面里只会出现和它有关的对话，用户在 MessageHub 里也能按来源认出这些对话 |
| Q3 | 是否允许 `embed.send`，即 App 以用户的名义发消息 | 允许 / 不允许 | 允许。Agent 浏览器的交接（`afterHandover`）和"问 Agent"（`askAgent`）都需要这个能力；按"先能力再安全"，已知风险是 App 可以替用户说话，但它用自己的会话本来也做得到（§1.4）。气泡上要不要标注"经某 App 发送"，另议 |
| Q4 | 系统页向 App 回传哪些信息 | a. 只回传 `status`、`sessionId` 和 Turn 状态；b. 再加上消息摘要 | a |
| Q5 | 按来源列出会话的开销：现在每个会话都要调一次 `listUiSessionState`（最多同时 6 个，结果只缓存在内存里），通用化以后每个用到的 App 都会这样做 | a. 先沿用；b. 让 msg-center 的会话列表支持按 UI 状态键筛选（改协议，单独立项） | 阶段二先选 a；第三个使用方出现之前立项做 b |
| Q6 | i18n 键 | a. 保留 `messagehub.*`；b. 改成 `conversation.*` | a。一个键都不用动 |
| Q7 | Agent 浏览器里的会话要不要补齐 MessageHub 的全部能力（编辑、置顶、转发、详情、搜索、拖放） | 全部补齐 / 保持现在的子集 | 组件默认全开，宿主没给回调的操作（转发目标选择器、消息详情页）就不出现。Agent 浏览器的"详情"可以做成打开 MessageHub 里的对应消息 |
| Q8 | CodeAssistant（只在 mock 应用清单里，只接 mock 数据）怎么处理 | 删除 / 改用 `ConversationSurface` 加 InMemory source | 由 owner 决定。如果保留，它正好可以作为"不接 msg-center 的 source"的样例 |
| Q9 | 第一个第三方调用方 | Text Editor 的"和 Agent 讨论这篇文档"，或其他 | Text Editor。它已经用了 Preview 嵌入和系统选择器，接入成本最低 |

## 6. 验证

- **阶段一**：`pnpm check`、`pnpm lint`（含 C8）、`pnpm test:e2e`；跑 C9 列出的用例，先记下基线。
- **阶段二**：`agent-browser.conversation.spec.ts`、`agent-browser.mobile.spec.ts` 按新结构调整后通过；访客页用例不改也要通过。
- **阶段三**：websdk 嵌入协议的单测，`/embed/conversation` 的 mock e2e（E6）。
- **真实 Zone**：在 MessageHub、Agent 浏览器和 Text Editor 的嵌入里，各走一遍发送、回答 `ask_user`、停止 Turn、撤回输入。
- **手机**：Agent 浏览器的底部弹层，以及嵌入页在窄屏和软键盘弹出时的表现。
