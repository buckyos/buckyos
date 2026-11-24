# 在 App 里嵌入和 Agent 的对话

App 要让用户在自己的页面里和 Agent 对话时，嵌入 Zone 提供的系统会话页，不要自己连 msg-center 拼一套聊天界面。系统页渲染的就是 MessageHub 用的同一个会话组件（`src/frame/desktop/src/components/conversation/`），回答 Agent 的提问（`ask_user`）、停止 Turn、附件、语音、表情这些能力都跟着系统走。

设计和取舍见 [会话 UI 公共组件化](../../notepads/conversation-ui-component-todo.md) §2.3。

## 用法

SDK 在 `buckyos/app-frame` 里提供 `ConversationEmbed`（App 端）。它和 `PreviewEmbed` 一样，只负责一个 iframe：

```ts
import { ConversationEmbed } from 'buckyos/app-frame'

const systemOrigin = `${location.protocol}//${buckyos.getZoneHostName()}`
const iframe = document.querySelector('iframe#agent')!
const embed = new ConversationEmbed({
  systemOrigin,
  frame: () => iframe,
  init: { target: { kind: 'agent' }, theme: 'light', locale: navigator.language, placeholder: 'Ask about this document…' },
  onState: state => { /* 'loading' | 'ready' | 'error'；ready 之后带 sessionId */ },
  onTurn: mode => { /* 'idle' | 'running' | 'interrupted' | 'paused' | 'answer' */ },
  onError: code => { /* 'invalid_params' | 'not_found' | 'denied' */ },
})
iframe.src = embed.url

// 以用户的名义发一条消息；握手前调用会排队，握手后按顺序发出
embed.send({ text: 'Let’s talk about this document', attachments: [{ kind: 'cyfs-path', path: 'cyfs:///home/alice/notes.md' }] })

// 主题、语言跟着 App 变
embed.update({ theme: 'dark' })
// 卸载 iframe 时
embed.dispose()
```

`target` 有两种：

| target | 打开哪个会话 |
|---|---|
| `{ kind: 'agent', agentDid?, memoryMode? }` | 用户和这个 Agent 的、由本 App 建立的最近一个会话。还没有时，第一条消息（App 的 `send` 或用户在输入框里发的）会新建一个。`agentDid` 不填就用用户的默认 Agent（第一个就绪的 Agent）。`memoryMode` 是会话的记忆模式：`normal`（不填时新建的就是它）、`no_write`（这里说的内容和从这里创建的工作不写入长期记忆）、`no_read_write`（既不读也不写）。填了就只认这个模式的会话，没有就按这个模式新建；已有会话的模式不会改 |
| `{ kind: 'session', sessionId }` | 指定的会话，但只能是本 App 建立的；别的会话回 `denied` |

`attachments` 里，`object-id` 引用作为消息附件发出；`cyfs-path` 引用不指向具体对象，作为一行路径文字附在消息里，Agent 用自己的权限去读。

## 规则

- 系统页只接受本 Zone 内的嵌入方（开发用的 mock 运行时另外放行 `localhost`）。
- App 是谁由系统页根据嵌入方的 origin 推出：`<web host>.<zone>` 对应 AppDoc 里声明这个 web host 的 App，取它的 `app_id`。App 自己报的任何值都不采信。这个 id 写进会话的 `ui.source`，MessageHub 里也能按它认出这些会话。
- 消息里不带 token 和 Cookie；系统页不会把消息内容回传给 App，只告诉 App 会话在不在（`embed.state`）和 Agent 在做什么（`embed.turn`）。App 需要内容就用自己的会话去读（msg-center 把普通 App 的 token 当作用户本人，见下）。
- 已知限制：Zone 根域没有登录会话时，iframe 里会出现登录页；App 可以遮挡或覆盖这个 iframe。

## 协议（`buckyos.conversation-embed` v1）

握手和 `buckyos.preview-embed` 相同：iframe 地址是 `<systemOrigin>/embed/conversation#bce=<nonce>&po=<App origin>`，系统页发 `embed.hello { nonce }`，App 回 `embed.init`。之后的每条消息都校验来源窗口和 origin。

| type | 方向 | payload |
|---|---|---|
| `embed.init` | App → 系统页 | `{ nonce, target, theme, locale, placeholder? }` |
| `embed.update` | App → 系统页 | `{ theme?, locale? }` |
| `embed.send` | App → 系统页 | `{ text, attachments?: TransferableContentRef[] }` |
| `embed.state` | 系统页 → App | `{ status, sessionId?, error? }` |
| `embed.turn` | 系统页 → App | `{ mode }` |
| `embed.error` | 系统页 → App | `{ code }` |

系统端是 Desktop 的 `src/components/conversation/embed/ConversationEmbedApp.tsx`（`ConversationEmbedHost`），协议实现和单测在 websdk `src/app-frame.ts`、`tests/conversation_embed.test.ts`。

## 例子和测试

- Text Editor 的“和 Agent 讨论”：`src/apps/text_editor/src/ui/App.tsx` 的 `AgentPanel`，在编辑区旁边嵌入对话，并把当前文档作为附件发过去。
- Desktop 的 mock e2e：`src/frame/desktop/tests/e2e/pages/conversation-embed.spec.ts`，用一个只讲协议的假 App 页面完成发送、回答提问和停止 Turn。

## App token 能做什么（核实于 2026-11-15）

msg-center 的身份层把非 Agent 的 App token 当作 token 所属用户本人（`owner_session.rs` 的 `caller_mailbox_identity`），`post_send`、`msg.list_sessions`、`ui_session.*`（带 owner 和不带 owner 两种）都走这一层，所以用户登录过的 Web App 本来就能读写这个用户的会话。之后各入口还要过 RBAC（`authorize_resource`，资源 `obj://msg-center/owners/<owner>`），App token 在这一层的结果要在真实 Zone 上实测。
