# OpenDAN Agent Message Chain

本文说明当前代码里 OpenDAN 如何接收 `msg-center` 的 `MsgObject`，如何转换成
LLMContext 使用的 `AiMessage`，以及 UI session 的 LLM round result 如何再构造成
`MsgObject`，最后经 `msg-center` 和 `tg_tunnel` 返回用户。

当前实现涉及的主要文件：

- `src/frame/msg_center/src/tg_tunnel.rs`
- `src/frame/msg_center/src/msg_center.rs`
- `src/frame/opendan/src/msg_center_pump.rs`
- `src/frame/opendan/src/agent.rs`
- `src/frame/opendan/src/session_model.rs`
- `src/frame/opendan/src/agent_session.rs`
- `src/frame/llm_context/src/msg_parser.rs`

## 消息模型

这条链路里有三层消息模型：

1. 外部平台消息，例如 Telegram update。
2. `ndn_lib::MsgObject` / `MsgRecordWithObject`，这是 msg-center 的存储、路由和
   tunnel 传输模型。
3. `buckyos_api::AiMessage`，这是 LLMContext 的 provider-neutral 输入/输出模型。

`MsgObject` 负责 envelope 和 MessageHub 语义：

- `from` / `to`：发送者和接收者 DID。
- `kind`：`Chat`、`GroupMsg`、`Event` 等。
- `thread`：用于 UI/session 聚合。
- `content.content`：文本主体。
- `content.refs`：CYFS named object 附件引用。
- `content.machine`：机器可读 payload。
- `meta`：tunnel 或上层系统补充的扩展信息。

`AiMessage` 负责 LLM 语义：

- `role`：`User`、`Assistant`、`System` 等。
- `content`：有序 `AiContent` block，例如 `Text`、`Image`、`Document`、
  `ToolUse`、`ToolResult`、`Thinking`、`ProviderState`。

`MsgObject` 和 `AiMessage` 的协议边界集中在
`src/frame/llm_context/src/msg_parser.rs`。

## Telegram 入站到 msg-center

`tg_tunnel.rs` 负责把 Telegram 消息转换为 `MsgObject`。

Grammers 网关入口是 `TgMessageConverter::tg_message_to_msg_object`；Bot API 网关有
对应的 `dispatch_incoming_message` 逻辑，最终都构造 `MsgObject` 并调用
`MsgCenterHandler::handle_dispatch`。

入站转换的关键行为：

- Telegram text 进入 `MsgContent.content`。
- Telegram media 会先存入 named-store，然后作为 `MsgContent.refs` 的
  `RefTarget::DataObj`。
- 图片、视频、音频、PDF 等 MIME 会映射到 `MsgContentFormat`。
- Telegram 的 chat id、message id、bot account、sender 等信息写入
  `msg.meta["telegram"]`。
- `msg.thread.topic` 设置为 `tg:<bot_account_id>:<chat_id>`。
- `IngressContext` 记录 tunnel DID、platform、chat id、source account、
  `contact_mgr_owner` 等路由上下文。

`msg_center.rs` 的 dispatch 逻辑会根据 `MsgObject.to`、群聊、contact manager
权限和 request box 策略，把消息写入目标 DID 的 `Inbox` / `GroupInbox` /
`RequestBox`。写入 record 时也会保存 ingress route，这个 route 后续用于让
OpenDAN 回复时尽量走回同一个 tunnel。

## OpenDAN 从 msg-center 拉取消息

OpenDAN 的 `msg_center_pump.rs` 订阅 msg-center 事件，并定期 sweep inbox，避免
kevent 丢失导致消息不被消费。

拉取逻辑：

1. 对 `Inbox`、`GroupInbox`、`RequestBox` 调用 `MsgCenterClient::take_next`。
2. `lock_on_take = true`，record 会从 `Unread` 进入 `Reading`。
3. `with_object = true`，返回值里直接带 `MsgObject`。
4. `deliver_record` 要求 record 中必须有 `MsgObject`。
5. 如果 `content.content`、`content.refs`、`content.machine` 都为空，则丢弃。
6. 调用 `llm_context::msg_object_to_ai_message(msg)` 转成完整 `AiMessage`。
7. 构造 `Inbound::Msg`，同时携带：
   - `record_id`
   - `from`
   - `from_did`
   - `from_name`
   - `tunnel_did`
   - `session_id`
   - `text`
   - `ai_message`

注意：`deliver_record` 不 ack msg-center。ack 发生在 `agent.rs` 的 dispatcher
把输入持久化进 session 之后。这样进程在投递到 session 前崩溃时，msg-center 的
lease recovery 仍可重放 `Reading` record。

## MsgObject 转 AiMessage

当前 OpenDAN 入站直接使用 `msg_object_to_ai_message`，不会走
`parse_msg_object` 的 slash command 分支。因此 `/xxx` 这样的纯文本消息当前会作为
普通 user message 送入 LLMContext。

转换规则在 `msg_parser.rs`：

- `MsgContent.content.trim()` 非空时，生成一个 `AiContent::Text`。
- 每个 `MsgContent.refs` 会按 target 转换：
  - `RefTarget::DataObj` 会变成 `AiContent::Image` 或 `AiContent::Document`。
  - 判断 image 的依据是 `MsgContentFormat`、ref label 或 `uri_hint` 是否像图片。
  - `ObjId` 保存在 `ResourceRef::NamedObject` 中。
  - `RefTarget::ServiceDid` 会变成 provider 为
    `buckyos.msg.ref.service_did` 的 `ProviderState`。
- `MsgContent.machine` 会变成 provider 为 `buckyos.msg.machine` 的
  `ProviderState`。
- 如果没有任何 block，会返回一个空文本 `AiMessage`。
- 默认 role 是 `AiRole::User`。

这个转换保留了 MsgObject 中的结构化附件和 machine payload；不再只取
`content.content` 文本。

## Inbound 到 Session

`agent.rs` 收到 `Inbound::Msg` 后：

1. 如果入站 record 已带 `session_id`，直接路由到该 session。
2. 否则按 sender host 解析或创建 UI session。
3. 构造 `PendingInput::Msg` 并调用 `AgentSession::enqueue_pending`。
4. `enqueue_pending` 把 pending input 写入 `.meta/session.json`。
5. 持久化完成后，dispatcher 才调用 `ack_msg_record(record_id)`。

`PendingInput::Msg` 当前包含必填 `ai_message: AiMessage`。开发阶段不做旧
session JSON 的兼容字段回填。

本地注入的文本入口，例如 `AgentSession::submit_text` 和 `forward_message`，会显式
构造 `AiMessage::text(AiRole::User, text)`。

## Session 组装 LLMContext 输入

`agent_session.rs` 的 worker drain pending inputs 时，会把消息分成三类：

- `PendingInput::Msg`：进入本轮 user 输入。
- 普通 `PendingInput::Event`：格式化为文本，再作为一个 user `AiMessage` 加入本轮。
- 匹配 `pending_task_calls` 的 task event：转换为 `Observation`，用于
  `ResumeFill::ToolResults` 恢复等待中的 LLMContext。

对 `PendingInput::Msg` 的处理：

- 使用 pending 中的 `AiMessage`，强制 role 为 `AiRole::User`。
- 非空文本、图片、文档、tool block、thinking、provider state 都算有效 payload。
- `from_did` 和 `tunnel_did` 会更新到 session meta，作为后续回复目标。
- `text` 仍用于 trigger preview 和 `current_origin_msg`，但 LLMContext 的真实输入
  来自 `ai_message`。

本轮输入最终通过 `compose_turn_message` 合成一个 user `AiMessage`：

- 如果有 environment preamble，它作为第一个 text block。
- 后续按顺序追加所有输入消息的 `AiContent` blocks。
- 相邻 text block 会用空行合并。
- 非文本 block 保持为独立 block，不会被压成字符串。

因此，一个带图片的 Telegram 消息会以 `Text + Image` 的结构进入 LLMContext，而不是
只留下图片 caption 或 `[attachment]`。

如果 session snapshot 当前存在 pending tool calls，裸 Msg/Event 不会直接开启新一轮；
worker 会继续等待 tool event，避免丢失 mid-run 状态。

## LLMContext Round Result 到 OpenDAN Reply

`AgentSession::run_one_round` 创建或恢复 `LLMContext` 后调用 `ctx.run()`。

当 outcome 是 `LLMContextOutcome::Done` 时：

1. 取 `response.message`，这是完整的 assistant `AiMessage`。
2. 调用 `post_outbound_message(&response.message)`。
3. 如果 `ContextOutput` 可提取文本，也通过 `reply_tx` 发给本地 reply collector。
   这个本地 collector 当前只负责 log，不负责 tunnel 发送。
4. 根据 behavior result 决定等待下一条用户消息、切换 behavior、结束独立进程等。

当 outcome 是 `BudgetExhausted` 且存在 partial output 时，当前只用
`post_outbound_text` 发送 partial 文本；这条路径不会保留结构化 block。

只有 `Ui` session 会通过 msg-center 对外回复；`Work` session 不走
`post_outbound_message`，其结果由工作流自身处理。

## AiMessage 转 MsgObject

`post_outbound_message` 会先构造一个 base `MsgObject`：

- `from`：agent DID。
- `to`：当前 session 记录的 peer DID。
- `kind`：`MsgObjKind::Chat`。
- `created_at_ms`：当前时间。
- `thread.topic` 和 `thread.correlation_id`：session id。
- `meta["session_id"]` 和 `meta["owner_session_id"]`：session id。

然后调用：

```rust
llm_context::ai_message_to_msg_object_with_base(message, msg)
```

`with_base` 的含义是保留调用方提供的 envelope，并替换 `base.content`。它本身不按
base 做额外过滤。

实际转换规则：

- `AiContent::Text` 写入 `MsgContent.content`。
- 多个文本段 trim 后用空行合并。
- 文本中的 `<attachment ... />` marker 如果带合法 `obj_id`，会转换为
  `MsgContent.refs`。
- `AiContent::Image` / `AiContent::Document`：
  - `ResourceRef::NamedObject` 转成 `RefItem::DataObj`。
  - `ResourceRef::Url` 或 `ResourceRef::Base64` 无法无损表示成 CYFS ref，会降级成
    文本里的 `<attachment ... />` marker。
- provider 为 `buckyos.msg.machine` 的 `ProviderState` 转成 `MsgContent.machine`。
- `ToolUse`、`ToolResult`、`Thinking`、其它 provider 的 `ProviderState` 会被丢弃。
- 写入 `meta["llm_role"]`，记录原始 `AiMessage.role`。

如果转换后的 `MsgObject` 没有文本、refs 和 machine payload，OpenDAN 会跳过发送。

## OpenDAN 通过 msg-center 发送

`post_outbound_message` 调用 `MsgCenterClient::post_send`，并传入 `SendContext`：

- `contact_mgr_owner = agent_did`
- `preferred_tunnel = session.peer_tunnel_did`

`peer_tunnel_did` 来自入站 record 的 route。这样 Telegram 进来的消息，回复时会优先
走回同一个 Telegram tunnel。

`msg_center.rs` 的 `post_send_internal` 会：

1. 保存 `MsgObject` named object。
2. 给 agent 自己创建一个 `Outbox` record，状态是 `Sent`。
3. 对每个目标 DID 生成 delivery plan。
4. 给目标 tunnel 创建 `TunnelOutbox` record，状态是 `Wait`。
5. route 中包含 tunnel DID、target DID、platform、account、chat id 等发送信息。

## tg_tunnel 出站到 Telegram

`TgTunnel::send_record` 处理 `TunnelOutbox` record：

1. 加载 record 对应的 `MsgObject`。
2. 用 `MsgObject.from` 找到绑定的 Telegram bot account。
3. `build_egress_envelope` 从 record route 解析 chat id：
   - 优先 `route.chat_id`
   - 其次 `route.extra["route"]["chat_id"]`
   - 其次从 `route.account_id` 解析
   - 最后使用 bot binding 的 `default_chat_id`
4. `TgMessageConverter::msg_object_to_tg_content` 提取 Telegram 可发送内容：
   - `MsgContent.content` 作为 text。
   - 如果 content 为空，尝试从 `meta["msg_payload"]["text"]` 取 fallback text。
   - 附件来自 `meta["telegram"]["attachments"]` 和 `MsgContent.refs` 的合并结果。
5. gateway 的 `send` 实现把 text 和附件发给 Telegram。

目前 `tg_tunnel` 的出站转换仍是基础实现：文本和 named object 附件是主路径；更完整的
Telegram 消息模型转换，例如 reply chain、entity、button 等仍是 TODO。

## 当前端到端链路

```text
Telegram update
  -> tg_tunnel: MsgObject + IngressContext
  -> msg_center.handle_dispatch
  -> Inbox / GroupInbox / RequestBox MsgRecord
  -> OpenDAN msg_center_pump.take_next(with_object = true)
  -> llm_context::msg_object_to_ai_message
  -> Inbound::Msg
  -> PendingInput::Msg { text, ai_message, route peer info }
  -> AgentSession::compose_turn_message
  -> LLMContextRequest.input
  -> LLMContextOutcome::Done { response.message }
  -> AgentSession::post_outbound_message
  -> llm_context::ai_message_to_msg_object_with_base
  -> msg_center.post_send(preferred_tunnel)
  -> TunnelOutbox record
  -> tg_tunnel.send_record
  -> Telegram send
```
