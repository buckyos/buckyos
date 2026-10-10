# Agent 消息桥接协议（MsgObject ↔ AiMessage）：当前设计与实现

> **文档定位**：描述 OpenDAN Agent 与 MessageCenter 之间消息协议的**当前实际设计**。每一节先写现在代码怎么做，再列出与早期规划的差异。尚未落地的规划和已知缺口统一放在 §7。
>
> **实现范围**：`msg_parser` 属于 `llm_context`，是共享的协议边界。OpenDAN 一侧（pump、分发、session 输入组装、出站）描述的是旧 opendan Runtime（`src/frame/opendan`）的代码，待下一阶段 opendan 按 libopendan 的 Session / Turn 抽象重构接入；libopendan 的输入批次与 Turn 规则见 [readme.md](readme.md)。
>
> **名词说明**：“Agent Message”不是一种独立的消息类型。系统里只有两个消息模型：`MsgObject`（投递与存储）和 `AiMessage`（推理，LLMContext 直接使用）。本文描述的是二者之间的转换规则，以及 OpenDAN 在转换两侧附加的路由、命令和附件策略。
>
> **核对基准**：2026-09-30，buckyos `d551cb7a`。代码位置以函数名为准，行号会漂移，所以不写行号。
>
> **相关文档**：

> - MessageCenter 与 Tunnel：[Message Center.md](<../message_hub/Message Center.md>)、[Message Tunnel Minimal Spec.md](<../message_hub/Message Tunnel Minimal Spec.md>)
> - Pending Input、Driver（旧 opendan Runtime）：[Agent Context Messages.md](<../opendan/Agent Context Messages.md>)、[Agent配置改进.md](../opendan/Agent配置改进.md)。前者的“Round”是“一条输入 → assistant 结束”的旧概念，不是现在的 Round
> - Round / Step / Turn 的当前定义：[readme.md](readme.md)

---

## 0. 两层模型与代码对照

消息有两种视角，它们由一个**只做形态转换**的边界层互转：

- **MessageCenter 层**：`MsgObject`，面向投递，不可变、可寻址。
- **Session History 层**：`AiMessage`，面向推理，provider 中立的 content-block 模型。

| 概念 | 代码 | 位置（`src/` 下） |
|---|---|---|
| 消息本体 | `ndn_lib::MsgObject` / `MsgContent` / `MsgObjKind` | 外部 crate `cyfs-ndn` 的 `ndn-lib` |
| 本地消息引用 | `MailboxRecord`（`MailboxKind`、`RecipientState`） | `kernel/buckyos-api/src/msg_center_client.rs` |
| 投递任务 | `DeliveryRecord`（`DeliveryState`），存于 `DELIVERY_QUEUE` | 同上；`frame/msg_center/src/msg_box_db.rs` |
| 入站元数据 | `IngressContext` | 同上 |
| 推理消息 | `AiMessage` / `AiContent` / `AiRole` | `kernel/buckyos-api/src/aicc_client.rs` |
| 协议边界 | `msg_parser`（不含任何 OpenDAN 策略） | `frame/llm_context/src/msg_parser.rs` |
| 入站 pump | `msg_center_pump::run` | `frame/opendan/src/msg_center_pump.rs` |
| 分发 | `AIAgent::dispatch_inbound` | `frame/opendan/src/agent.rs` |
| Session 输入队列 | `PendingInput` / `SessionMeta.pending_inputs` | `frame/opendan/src/session_model.rs` |
| 本次输入组装 | `compose_turn_message`、`render_on_wakeup_input_text` | `frame/opendan/src/agent_session.rs`、`prompt_env.rs` |
| 出站 | `AgentSession::post_outbound_message` | `frame/opendan/src/agent_session.rs` |
| 出站附件策略 | `WorkspaceAttachmentValidator`、`NamedStoreLocalLinkResolver` | `frame/opendan/src/attachment_policy.rs`、`attachment_resolver.rs` |
| 命令 | `BUILTIN_COMMANDS`、`run_command` | `frame/opendan/src/command_dispatcher.rs` |
| Tunnel | `DeliveryExecutor` trait、Telegram 实现 | `frame/msg_center/src/msg_tunnel.rs`、`tg_tunnel.rs` |

**职责划分**：`msg_parser` 只翻译协议形态。路由、授权、命令执行、回复策略都在调用方（OpenDAN）。路径校验和本地文件入库这类策略，由 OpenDAN 通过 `AttachmentValidator` / `LocalFileResolver` 两个 trait 注入。

---

## 1. 端到端链路（当前实现）

```text
Telegram update
  -> tg_tunnel: dispatch_incoming_message（grammers / Bot API 两条网关）
       MsgObject{from = 发送者 did:msgtunnel 影子 DID, to = agent DID 或群 DID}
       + IngressContext + 幂等键；thread.topic = "tg:<bot_account_id>:<chat_id>"
  -> msg_center.handle_dispatch -> dispatch_internal
       1v1：contact_mgr.check_access_permission 决定 INBOX / REQUEST_BOX / 丢弃
       群聊：写群 DID 名下的 GROUP_INBOX 记录，并扇出到各成员的 INBOX（tag "group:<gid>"）
       MailboxRecord{state = Unread, session_id = derive_session_id(..)}
       -> 发布 kevent "/msg_center/<owner>/<BOX>/..."
  -> OpenDAN msg_center_pump（kevent 加速 + 1s 超时全量扫描）
       list_mailboxes + get_next(box, [Unread], lock_on_take=true, with_object=true)
       （记录 Unread -> Reading）
  -> lower_inbound_message
       llm_context::parse_msg_object_structured(msg, BUILTIN_COMMANDS)
         ├─ 命中命令白名单 -> Inbound::Command
         │    -> agent.dispatch_inbound -> command_dispatcher::run_command
         │    -> dispatch_command_reply（直接 post_send 原始 MsgObject）-> ack
         └─ 普通消息 -> Inbound::Msg{.., session_id, group_id, ai_message}
  -> agent.dispatch_inbound
       event_type = group_id ? "msg.group" : "msg.chat" -> dispatch 规则选 session class
       解析 session_id -> get_or_create_session
       -> session.push_msg(PendingInput::Msg)（落盘 .meta/session.json 后返回）
       -> ack_msg_record（Reading -> Read）
  -> AgentSession worker（每个 session 一个串行 worker）
       drain pending -> on_wakeup hook
         ├─ behavior 配了 [prompt].on_wakeup：模板渲染为一条 user 文本
         │   （<background_environment> 在这里由模板注入）
         └─ 未配置：prepare_turn_messages_for_run -> compose_turn_message（od.msg/1）
       -> run_one_round -> build_or_resume
       -> LLMContextRequest.input（新上下文）或 append_turn_message_to_snapshot（续跑）
       -> LLMContext::run()
  -> handle_outcome
       Done：post_outbound_message(response.message)
       BudgetExhausted：partial 文本 -> AiMessage(Assistant) -> post_outbound_message
  -> post_outbound_message（仅 SessionKind::Ui）
       ai_message_to_msg_object_with_base_validated_async
         + WorkspaceAttachmentValidator（路径 / URL 校验）
         + NamedStoreLocalLinkResolver（本地文件 -> ObjId）
       -> msg_center.post_send(msg, None)
  -> msg-center post_send_internal：由目标 DID 决定路由
       SENT MailboxRecord + DeliveryRecord{state = Wait}，入 transport_did 名下的 DELIVERY_QUEUE
  -> msg-center pump_delivery_queue_once -> DeliveryExecutor::execute_delivery（TgTunnel）
       -> Telegram send -> report_delivery
```

---

## 2. 入站：MsgObject → 本次输入的 user 消息

### 2.1 Pump

`msg_center_pump` 是一个**纯 fetcher**：

- **订阅**：订阅 kevent `/msg_center/<owner>/{INBOX,GROUP_INBOX,REQUEST_BOX}/**`。kevent 只起加速作用；1 秒超时、reader 失效或未知事件时，都回退到三个 box 的全量扫描。
- **取记录**：`get_next(mailbox, box_kind, [Unread], lock_on_take=true, with_object=true)`，每个 box 每次拉取最多取 128 条。
- **不 ack**：ack 由 dispatcher 在 session 把输入持久化之后执行。进程在中途崩溃时，记录会停在 `Reading`，由 msg-center 的 lease 恢复机制重新投递。
- **`from_name`**：优先用 `MailboxRecord.from_name`，缺失时查 `ContactLookup.from_name`。这是一个带缓存的 `get_contact` 查询（命中缓存 300s，未命中缓存 60s）。
- **`tunnel_did`**：取自 `record.ingress.transport_did`，只用于诊断，不参与回复路由（见 §6）。
- **`group_id`**：当 `record.msg_kind == GroupMsg` 时，取 `msg.to[0]`；退路是 tag `group:<gid>`。
- **群消息来源**：Agent 实际是从**自己的 INBOX** 收到群消息的，因为 msg-center 已经把群消息扇出到各成员的 INBOX。`GROUP_INBOX` 记录的 owner 是群 DID。

### 2.2 形态转换：`parse_msg_object_structured`

pump 使用的是 `*_structured` 版本：

- 先做命令识别（§4）。
- 未命中命令时，调用 `msg_object_to_ai_message_structured`，生成一条 `role = User` 的 `AiMessage`。

| MsgObject 部分 | AiMessage block |
|---|---|
| `content.content`（trim 后非空） | `Text` |
| `refs` 中的 `DataObj` | 按 label / uri_hint / format 推断：图片走 `Image`，否则走 `Document`；`source = ResourceRef::NamedObject{obj_id}` |
| `refs` 中的 `ServiceDid` | `ProviderState{provider: "buckyos.msg.ref.service_did"}` |
| `content.machine` | `ProviderState{provider: "buckyos.msg.machine"}` |
| （structured 版本额外追加） | `ProviderState{provider: "buckyos.msg.metadata", value: {attachments, message_references}}` |

补充说明：

- `attachments` 为每个 `DataObj` 生成一条结构化描述：`index / kind / role / source.obj_id / mime / title / label / text_marker`。
- `message_references` 目前只取 `thread.reply_to`，记为 `relation = "reply_to"`。
- 如果正文只是 `[image]`、`[attachment]` 这类占位符，并且消息带附件，这个占位 `Text` 会被删掉。

### 2.3 本次输入的 user 消息组装

一次 wakeup 会 drain 当前全部 pending 输入，其中 `pull_msg` 等选项由 driver 配置决定。这批输入对应 libopendan 的一个输入批次（`<session_input hook="on_wakeup">`）：它是开启新 Turn 还是并入当前 Turn 由 Session 判断（没有打开的 Turn 才开新 Turn），不能按 user 消息条数或 wakeup 次数计 Turn。旧 Runtime 按 behavior 是否配置了 `[prompt].on_wakeup`，走两条路径之一：

- **配置了 `on_wakeup`**（例如 `chat_route`、`self_check`）：
  - 模板渲染结果整体替换为一条 `AiMessage::text(User, ..)`。
  - 模板变量 `{{input.text}}` 的取值规则：如果本批消息多于 1 条，或者消息带结构化元数据，取 `od.msg/1` JSON；否则取拼接后的纯文本。
- **未配置**（例如 `groupchat_route`）：
  - 走 `prepare_turn_messages_for_run` → `compose_turn_message`。
  - 同样的判断条件下，`prompt_env::render_ai_message_batch` 会生成一条 `od.msg/1` JSON 文本；否则按原 block 合并。
  - 事件批次另外追加为一条 user 文本。

`od.msg/1` 的形态如下（空字段省略；完整规则见 [Agent Enviroment.md](<../opendan/Agent Enviroment.md>)）：

```json
{"schema":"od.msg/1","messages":[
  {"text":"...","attachments":[{"index":0,"kind":"image","src":{"obj":"..."},"label":"a.png","mime":"image/png"}],
   "refs":["<reply_to obj_id>"]}
]}
```

由此带来两点实际效果：

- pump 转换出的消息总会带 `buckyos.msg.metadata`，所以**真实入站消息基本都以 `od.msg/1` JSON 文本进入 LLM**。附件以引用的形式出现，而不是原生的 `Image` / `Document` block。需要理解媒体时，由 LLM 通过工具（如 `llm_understand_media`）按 obj_id 读取。
- `od.msg/1` 的消息信封里**不含发送者信息**：没有 from、from_did、from_name、relation、timestamp。模板可以通过 `input.msgs[*]` 自行渲染 `from` / `from_did` / `tunnel_did`（没有 `from_name`），例如 `plan` 的 `on_behavior_step_ob` 写的是 `<message from={{msg.from}}>`。但 `chat_route` 和 `groupchat_route` 的输入路径都没有渲染发送者。

### 2.4 Background Environment

环境信息**由 behavior 的 `on_wakeup` 模板负责注入**，不在 `compose_turn_message` 里拼接。`chat_route.toml` 的写法：

```text
{% if session.background_hint_changed %}
<background_environment current_clock="{{runtime.clock_text}}">
{{ session.default_changed_background_hint_text }}
</background_environment>
{% endif %}
{{input.text}}
```

- **内容**：
  - `runtime.clock_text` 是本地时间，格式 `%Y-%m-%d %H:%M:%S %Z`。
  - 背景提示取自 `load_changed_background_hits`，来源包括 session 的 background events，以及 HintRecallEngine 按 topic tag 从 memory / notebook 召回的结果。
  - 只注入指纹发生变化的提示，非空提示的注入间隔至少 60s（`BACKGROUND_HINT_NON_EMPTY_INTERVAL_MS`）。
  - 渲染为 `- {text}` 列表。
  - 没有定位信息。
- **标签形态由各 behavior 自行决定**：
  - `chat_route`（agent loop）用 XML 标签 `<background_environment current_clock=..>`。
  - `self_check`（behavior loop）用 `<<background_environment>> … <</background_environment>>` 段落，时间单独写成 `Current Time:` 一行。
  - `groupchat_route` 没有 `on_wakeup`，**所以群聊 session 当前不注入环境信息**。
- **两层可见性**：`MsgObject` 层不含该标签；标签只存在于 Session History 的 user 消息中。
- **注入时机**：由 driver 的 `on_wakeup` 与 `load_background_hits` 配置决定。driver 配置里的 `inject_background_environment` 字段**目前不生效**：`run_one_round` / `build_or_resume` 接收了这个参数，但没有使用。
- **WebUI**：目前没有针对该标签的特殊渲染。

---

## 3. 出站：AiMessage → MsgObject

### 3.1 谁会发出站消息

`AgentSession::post_outbound_message` **只对 `SessionKind::Ui` 生效**；Work 类 session 通过 report 回传结果。出站信封的字段如下：

| 字段 | 值 |
|---|---|
| `from` | `agent.toml [identity].agent_did` |
| `to` | `[meta.peer_did]`，即本 session **最近一条入站消息的 `from_did`** |
| `kind` | `Chat` |
| `thread.topic` / `thread.correlation_id` | `session_id` |
| `meta` | `session_id`、`owner_session_id`、`turn_nonce`、`delivery_failure_notice`，以及转换时写入的 `llm_role`（通常是 `"assistant"`） |

有三种情况会直接跳过发送：缺少 `peer_did` 或 `agent_did`；两者相等（本地注入的消息）；转换结果为空。

调用 `post_outbound_message` 的地方：

- `Done` 结果：LLMContext run 每次以 `Done` 返回都会发（它与 Turn 完成的区别见 §3.7）。
- `BudgetExhausted` 的 partial 输出：`ContextOutput` 只有 `Text` / `Json` 两种，会被包成 `AiMessage::text(Assistant, ..)` 后发送。因为 outcome 类型本身不承载 block，partial 输出里不可能带图片或附件。
- `post_outbound_error`：把 i18n 文案 `response.failed` 包成 Assistant 文本后发送。

### 3.2 Block 映射

转换函数是 `ai_message_to_msg_object_with_base_validated_async`。另有同步和 permissive 版本，供测试和无策略场景使用。

| AiContent | MsgObject |
|---|---|
| `Text` | 逐行扫描 `<attachment>` 标签（§3.3）；其余文本保留。多个 Text 块之间用空行拼接 |
| `Image` / `Document` + `NamedObject` | 经 `validate_obj_id` 校验后，生成 `RefItem::DataObj{role: Input}` |
| `Image` / `Document` + `Url` | 经 `validate_url` 校验后，降级为文本 marker `<attachment kind=".." source="url" url=".." />` |
| `Image` / `Document` + `Base64` | 降级为带 `data_base64` 的文本 marker（不经过校验） |
| `ProviderState{provider: "buckyos.msg.machine"}` | 汇总进 `content.machine`（`intent = "buckyos.msg.machine"`，数据放在 `data.provider_state`） |
| `ToolUse` / `ToolResult` / `Thinking` / 其它 `ProviderState` | 丢弃。这是有意设计：它们属于 Session History 的内部状态 |

转换后会写入 `meta["llm_role"] = role.as_str()`。

### 3.3 `<attachment>` 标签语法

标签必须**独占一行**，因为解析是按行进行的。支持两种形态：

```text
<attachment>/abs/or/relative/path.pdf</attachment>          # 主推形态：body 是本地路径
<attachment>cyfile:0123...</attachment>                     # body 能被 ObjId::new 解析，就当作 obj_id
<attachment kind="document" obj_id="..." title="report.pdf" />
<attachment kind="image" path="out/chart.png" />
<attachment kind="image" url="https://..." mime="image/png" />
```

`kind` 缺省为 `document`。body 的判定规则：不含 `/`、`\`、空白，并且能被 `ObjId::new` 解析，就视为 obj_id；否则视为路径。

### 3.4 校验（`AttachmentValidator`）

LLM 输出属于不可信输入。OpenDAN 在出站时注入 `WorkspaceAttachmentValidator`：

| 引用 | 规则 |
|---|---|
| 本地路径 | 一律拒绝 `..` 和盘符前缀。然后按 `[runtime] filesystem_policy` 处理：<br>`workspace`（默认）：路径（相对路径相对 workspace 根）经 canonicalize 解析符号链接后，必须仍在 `workspaces/<workspace_id>` 内；session 没有绑定 workspace 时一律拒绝。<br>`unrestricted`：允许宿主机上可读的绝对路径；相对路径仍然需要 workspace 作为锚点。Jarvis 使用的是 `unrestricted`。 |
| `obj_id` | 目前直接放行。obj_id 基于内容寻址，无法伪造；ACL 裁决交给 msg-center 和接收方 zone，这里只保留扩展口 |
| URL | 拒绝 `file:` scheme，防止把本地路径伪装成 URL 绕过路径校验 |

校验失败时不丢弃附件，而是在出站文本里保留 `<!-- attachment rejected: <reason> -->` 加原标签，接收方能看到 LLM 的意图，日志里也留有记录。

如果 `obj_id` 字符串本身不合法（`ObjId::new` 失败），会返回 `MsgParserError::InvalidObjId`。这会让整条消息转换失败，`post_outbound_message` 随即改发一条 `delivery_failure_notice`。

### 3.5 本地文件入库

通过校验的路径会交给 `NamedStoreLocalLinkResolver`，调用 `cacl_file_object` 注册到 NamedStore，得到 `ObjId` 后生成 `RefItem::DataObj`：

- 文件不小于 12 KiB（`3 × 4096`）时，用 **LocalLink** 模式，不复制字节。
- 更小的文件没有 qcid，直接存入 NamedStore。

这样，出站附件和外部上传的附件走的是同一条 `DataObj` 通道，Tunnel（如 Telegram）可以按统一方式上传。入库失败会降级为上面的 rejected 文本。

长期方向不变：优先使用 obj_id，path 只作为 LLM 的便利入口，由 runtime 立即把它物化成 obj_id。跨 host 场景下，出站消息里不会出现裸 path。

### 3.6 标签去留

- **默认删除**：已成功转换成 ref 的标签，会从 `MsgObject.content.content` 中移除。
- **保留开关**：agent 级配置 `[runtime] preserve_attachment_tag_in_egress = true` 可以保留原标签。注意这是 **agent 级**，不是 session 级。
- **不受开关影响的两类**：未转换的标签（只有 url 等）和 rejected 标签始终保留在文本中。
- **Session History 不受影响**：`MsgObject` 是另外构造的，assistant `AiMessage` 原文（含标签）会原样留在 LLM 历史中。

### 3.7 出站消息与 Step、report、Done、Turn 的边界

出站消息是 Agent 对外说的话，不是推进状态的单位。相关概念的区别如下（Round / Step / Turn 的定义见 [readme.md](readme.md)）：

| 概念 | 是什么 | 谁产生 / 谁判定 |
|---|---|---|
| 消息 | 入站 `MsgObject` 转成的 user `AiMessage`；出站的 assistant 文本或 `<sendmsg>` | `msg_parser` 转换；投递由 Session / msg-center 负责 |
| 决策（Step） | Behavior Loop 中一次 LLM 决策连同动作结果，记为一个 `StepRecord`；内部可以有多个 Round。function_call Loop 没有 Step | llm_context |
| 动作批次 | 一个 Step 的 `<actions>`，按序派发，带 action 的 Step 扣一次工具迭代额度。`<sendmsg>` 写在 `<actions>` 里，但 llm_context 只把它记进 `messages_sent` 并发出 `WorkEvent::MessageSent`，不负责投递（xllm 把 `sendmsg` 显式配置为 action 时除外） | llm_context |
| `<report>` | 更新 `LLMContextState.last_report`；缺省 / false 不投递、不结束，end=true 由宿主接受完成提交 | llm_context 写入；Session 决定如何上交 |
| behavior Done | `report_end=true` 或生效的 `next_behavior`（WAIT_USER_MSG / 跳转目标）使 `LLMContext::run()` 以 `Done` 返回；空决策纠错；function_call 模式下是最后一个没有 tool call 的 Round | llm_context 返回，含义由 Session 解释 |
| Turn 完成 | 一次逻辑输入得到结果 | 只由 Session 关闭（libopendan `finish_run`，worklog `turn_ended`） |

要点：

- 发出一条消息（旧 Runtime 的 `post_outbound_message`、Step 里的 `<sendmsg>`）本身不等于 Turn 完成；中间的 `<report>` 也不是。
- libopendan 的规则：交付了结果的 Done（最终回答 / report）使 Turn `completed`；`WAIT_USER_MSG` 只有在本 run 有 `<report>`（`last_report`）或最后一个 Step 有 `<sendmsg>` 时才完成 Turn，否则 Turn 保持打开，下一条输入并入；behavior 切换和 fork 返回让 Turn 保持打开；不可重试错误、预算耗尽、stop 分别以 `failed` / `budget_exhausted` / `stopped` 关闭 Turn。
- 旧 opendan Runtime 不按 Turn 收口：UI session 的每次 `Done` 都直接把最终 assistant message 发出站。按 Turn 收口出站待下一阶段 opendan 重构接入。

---

## 4. 命令消息

### 4.1 识别规则

识别由 `msg_parser::msg_object_control_command` 完成。一条消息同时满足以下条件，才会被识别为命令：

- `content.format` 为空或属于纯文本类：plain / markdown / html / css / xml。
- 没有 `refs`，也没有 `machine`。
- 正文以 `/` 开头。
- 第一个空白之前的 token 与已注册命令名**大小写不敏感地完全相等**。

命中后返回 `SystemControlCommand{command（小写化）, args（剩余部分 trim）}`；否则按普通消息进入推理。由于采用白名单，`/etc/nginx/conf 帮我看下` 这类文本是安全的。

**转义**：如果要把一个恰好等于命令名的文本发给 LLM，在 `/` 前加任意字符即可，因为匹配只从正文开头进行。

### 4.2 命令表（`BUILTIN_COMMANDS`）

| 命令 | 语义 |
|---|---|
| `/new` | 为当前 tunnel（`from`）新建 UI session 并绑定 |
| `/clean` | 解绑并**物理删除**当前 session，再新建一个并绑定；执行前不做确认 |
| `/stop` | 以 `InterruptMode::Graceful` 中断当前 session |
| `/cancel` | 以 `InterruptMode::Discard` 中断当前 session |
| `/info` | 显示当前 session 的状态；没有绑定时显示 agent 概况 |
| `/list` | 列出本 agent 的活跃 session |
| `/switch <session_id>` | 把当前 tunnel 绑定到指定 session |
| `/compress` | 手动压缩一次当前 session 的上下文 |
| `/help` | 列出命令 |

新增命令需要同时修改 `BUILTIN_COMMANDS` 和 `run_command` 里的 match。

### 4.3 执行路径

```text
Inbound::Command{record_id, from, from_did, tunnel_did, command, args}
  -> agent.dispatch_inbound -> command_dispatcher::run_command -> CommandOutcome{reply}
  -> AIAgent::dispatch_command_reply
       MsgObject{from = agent_did, to = [from_did], kind = Chat, format = TextPlain}
       meta: llm_role = "system", parse_mode = "Plain"
       msg_center.post_send(msg, None)
  -> ack_msg_record
```

- 命令不进入 session 推理队列，也**不走 `post_outbound_message`**；回复是一条直接构造的 `MsgObject`，没有 `thread.topic`。
- 命令和回复都不进入 LLM history，因为这条路径完全绕开了 `AgentSession`，不需要额外的跳过逻辑。
- 当前 session 通过 `tunnel_to_ui_session[from]` 查找（`resolve_session_for_command`）。查不到时，依赖当前 session 的命令会返回错误文本，不会自动新建。

### 4.4 Tunnel 原生命令入口

- **Telegram**：Bot 启动时（grammers 与 Bot API 两条路径都会），通过 `setMyCommands` 注册 `TG_BUILTIN_COMMANDS`，共 8 条：new、clean、stop、cancel、info、list、switch、help。用户点选后仍然以 `/xxx` 文本入站，与手输命令走同一套解析和 handler。
- **Discord / Slack**：未实现。

---

## 5. Session 分类、群聊与身份

### 5.1 SessionKind 与 session class

`SessionKind` 描述的是 session 的**运行形态**，不区分单聊和群聊：

```rust
// frame/opendan/src/session_model.rs
pub enum SessionKind { Ui, Work, SelfCheck, SelfImprove }
```

单聊和群聊的区分放在 **session class** 这一层。入站消息按 `event_type` 匹配 `agent.toml` 的 dispatch 规则，选出 class。以 Jarvis 为例：

| event_type | class | kind | default_behavior | session_id_strategy |
|---|---|---|---|---|
| `msg.chat` | `ui` | `ui` | `chat_route` | `per_peer` |
| `msg.group` | `group` | `ui` | `groupchat_route` | `per_group` |
| `task_mgr.*` / `kvdoc.*` | `work` | `work` | `plan` | `per_event_session` |
| `timer.*` | `self_check` | `self_check` | `self_check` | `singleton` |

`SessionIdStrategy` 共四种：`PerPeer / PerGroup / PerEventSession / Singleton`。

**session_id 的解析顺序**（`resolve_msg_session_id`）：

1. 只有 `ui` kind 加 `per_peer` 的 class 才使用 tunnel 绑定。对这类 class，先查 `tunnel_to_ui_session[from]`，这个绑定由 `/new`、`/switch`、`/clean` 或首条消息建立。
2. 否则取 `MailboxRecord.session_id`。它由 msg-center 的 `derive_session_id` 生成，依次尝试：`thread.topic`（Telegram 为 `tg:<bot>:<chat_id>`）→ 消息中显式的 session id → 群 DID → `dm:<from>`。所以真实入站消息几乎总会带 session_id。
3. 再退一步，用 strategy evaluator 计算。实际上只有本地注入的消息才会走到这一步。
4. 最后兜底为新建 UI session。

**约束**：

- `kind` 在 session 创建时写入 `SessionMeta`，之后没有代码修改它，但代码里也没有强制它不可变。
- session class 不会持久化。
- session 的 driver 通过 `class_name_for_kind(kind)` 反查。因此 `group` class 的 session（kind 为 `ui`）实际使用的是 `[session.ui].driver`；目前两份配置恰好相同。

### 5.2 群聊现状

- **触发**：**每条群消息都会入队并触发推理**，目前没有 @ 过滤。
- **提示词**：`groupchat_route` 的 system prompt 声明了几件事：当前处于群聊 UI session；只听 owner 的指令；不要把每个参与者都当成 owner；需要 owner 权限时先请 owner 确认；不要把群聊闲谈转进 Work Session。工具白名单只有 `try_create_worksession` 和 `forward_msg`。
- **发送者身份**：LLM 看不到发送者。`groupchat_route` 没有 `on_wakeup`，走的是 `compose_turn_message` → `od.msg/1`，而这个信封不带发送者字段（§2.3），所以 LLM 分不清一条消息是谁发的。
- **历史**：没有回放“上次推理之后的群消息”，也没有旁路压缩。群消息本身就是逐条入队的 pending 输入；`pull_msg = "all"` 时，同一次 wakeup 会把积压的消息合成一个批次。

### 5.3 并发与排队

| 规则 | 实现 |
|---|---|
| 同一 session 同一时刻最多推进一个 run | 每个 session 一个 `run_worker` 串行 worker |
| 推理进行中到达的消息不丢弃 | 进入 `SessionMeta.pending_inputs`，持久化到 `.meta/session.json`，按 `dedup_key` 去重（`msg:<record_id>` 等） |
| 当前 run 返回后继续处理 | worker 的 drain 循环 |
| 队列上限 | `MAX_PENDING_INPUTS = 256`，由 `enforce_pending_queue_limit` 执行，**所有 session 都适用**。超限时按以下顺序淘汰：<br>① 最早的 `Event`<br>② 最早的、未 @ 本 agent 的 `Msg`<br>③ 最早的非 `Interrupt` 项（@ 消息也可能在这一步被淘汰）<br>`Interrupt` 永远不会被淘汰。 |

判断“是否 @ 本 agent”的方法：在消息文本中大小写不敏感地查找子串 `@<agent_name>`，其中 `agent_name` 是 `identity.display_name` 或目录名，去掉空白。它不是 Telegram 的 bot username。

### 5.4 `from_user_did`：工具调用的发起人

- **注入**：由 runtime 强制注入，LLM 无法控制。`OpendanToolAdapter::call_tool`（同步工具）、异步 PendingTool 分发，以及 fork 子上下文（`ForkSubContextInput.from_user_did`），都会**覆盖**写入 `args["from_user_did"]`；没有值时则删除该字段，防止 LLM 伪造。
- **不可见**：工具 schema 不暴露这个字段。注入发生在副本上，所以 history 里的 `ToolUse` 不含它；step 渲染时也会跳过它。
- **取值**：`current_from_user_did()` 返回 `meta.peer_did`，也就是**本 session 最近一条入站消息的 `from_did`**。对 Telegram 来说，这是发送者的 `did:msgtunnel:*` 影子 DID。它不是 owner DID，也不是“@ agent 的那个人”。Work / 自主 session 没有 peer，取值为 `None`。
- **消费方**：目前**没有任何工具读取**这个字段。
- **计费**：没有接入。AICC 用量记录中的 `user_id` 是 RPC 调用方身份；OpenDAN 调 AICC 时只传 `session_id` 和 `trace_id`。

---

## 6. 路由与 Tunnel

路由模型已经在 msg-center 升级中改为**由目标 DID 决定**。本节以 [Message Tunnel Minimal Spec.md](<../message_hub/Message Tunnel Minimal Spec.md>) 为准，这里只列出与 Agent 相关的部分。

### 6.1 DID 规则

```text
可共享 DID           did:bns:...                                   -> MessageHub 原生投递
本地影子端点 DID     did:msgtunnel:<encoded_account_id>.<account_type>.<tunnel_instance_id>
                                                                   -> 对应 tunnel 实例投递
其它 / 解析失败      post_send 报错；没有 default tunnel，也没有 fallback
```

- 1v1 消息：`kind = Chat`，`from` 为发送者的影子 DID，`to` 为 agent DID。
- 群消息：`kind = GroupMsg`，`from` 为发言者的影子 DID，`to` 为群 DID。按规范，**回复群应当写 `to = 群 DID`，而不是回给 `from`**（当前实现的偏差见 §7）。

### 6.2 入站元数据

- **字段**：`IngressContext{transport_did, platform, chat_id, source_account_id, context_id, contact_mgr_owner, extra}`。
- **用途**：它挂在 `MailboxRecord.ingress` 上，**只用于审计和辅助回复，不作为路由输入**。pump 把其中的 `transport_did` 作为 `tunnel_did` 传给 session，session 存为 `peer_tunnel_did`，但只用于诊断。
- **Telegram 平台 meta**：`meta["telegram"]` 包含 chat_dialog_id、chat_username、sender_id、sender_username、sender_name、bot_account_id、payload_kind、message_id、chat_type、chat_name 和 attachments。**没有 mentions 字段**。

### 6.3 出站

- **路由方式**：出站消息的 `to` 本身就是确定的端点 DID，所以 `post_send(msg, idempotency_key)` 不需要 tunnel 偏好或路由提示。`SendContext` 和 `preferred_tunnel` 已经删除。
- **入队**：msg-center 的 `build_delivery_envelope` 按目标 DID 区分：`did:msgtunnel:*` 生成 Tunnel 投递，并把 chat_id 写进 `DeliverySnapshot`；其它 DID 走 MessageHub 原生投递。然后生成 `DeliveryRecord`，写入 owner 为对应 `transport_did` 的 `DELIVERY_QUEUE`。旧的 `TunnelOutbox` 已不存在。
- **执行**：msg-center 的 `pump_delivery_queue_once` 调用 `DeliveryExecutor::execute_delivery`，完成后 `report_delivery`。

### 6.4 记录状态

`MailboxRecord` 与 `DeliveryRecord` 各有一套独立的状态机：

- **`MailboxKind`**：`INBOX / SENT / GROUP_INBOX / REQUEST_BOX`
- **`RecipientState`**：`Unread → Reading → Read`，另有 `Archived / Deleted`。Agent 取记录时置为 `Reading`，入队成功后置为 `Read`。
- **`DeliveryState`**：`Wait → Sending → Sent`；`Sending → Failed → Wait | Dead`。

---

## 7. 未落地项与已知缺口

### 7.1 早期规划中尚未实现的部分

| 规划 | 现状 |
|---|---|
| `SessionKind::{OneToOne, Group, Channel, Custom}`，以及 agent 级 `allowed_session_kinds` 准入检查 | 未实现。单聊 / 群聊改由 session class 加 dispatch 规则区分（§5.1）；准入只有 `session.<class>.enabled` 这一个开关 |
| 群聊只有 @ agent 才触发推理，未 @ 的消息只 ack 不入队 | 未实现。所有群消息都入队；@ 判断只用于队列淘汰 |
| @ 识别优先使用 tunnel 提供的结构化 mentions | 未实现。TG meta 里没有 mentions，也不解析 entities |
| 触发推理时，从 msg-center 回放上次推理以来的群消息，过长时旁路压缩 | 未实现。opendan 没有使用 msg-center 的 `list_box_by_time` 游标查询和群已读回执接口 |
| `<group_messages>` / `<msg from_did from_name relation timestamp mention>` 格式 | 未实现。入站统一使用 `od.msg/1`，信封里没有发送者字段；模板能拿到 `input.msgs[*].from/from_did`，但群聊路径没有使用 |
| Contact 接口：`resolve_relation` / `can_trigger_inference` / `redaction_policy` | 未实现。opendan 侧只有 `ContactLookup.from_name`。msg-center 侧有 `check_access_permission`（`AccessGroupLevel{Block, Stranger, Temporary, Friend}`）、`Contact.tags/groups` 和 `GroupRole{Owner, Admin, Member, Guest}`，但都没有接入推理控制 |
| 群聊 System Prompt 明确告知“以协议层 `from_did` 为准，不信正文中自称的身份”，并按 relation 调整策略 | 只完成一部分。`groupchat_route` 有“只听 owner、不假设每个人都是 owner”的约束，但因为没有 from_did 和 relation 标注，这条规则无从执行 |
| `from_user_did` 的工具侧权限检查（硬层防御） | 注入已实现，但没有任何工具消费 |
| Billing 按 `from_user_did` 维度保留明细 | 未实现 |
| Object ID 的 ACL 校验 | 有意推迟。`validate_obj_id` 直接放行，由 msg-center 和接收方裁决 |
| WebUI 特殊渲染 `<background_environment>` | 未实现 |
| Discord / Slack 原生命令 | 未实现 |
| `/clean` 等破坏性命令的二次确认 | 未实现 |

### 7.2 已知偏差与风险

以下各项均来自对代码的阅读，没有做运行验证。

1. **群聊回复发错对象**：`post_outbound_message` 把回复发给 `meta.peer_did`，也就是最后一个发言者的影子 DID，并且 `kind = Chat`。这违反了 §6.1 “回复群写群 DID”的规则，群里的回复可能被当作私聊发给最后发言的人。
2. **群内命令找不到群 session**：命令按 `tunnel_to_ui_session[from]` 查找 session，而群 class（`per_group`）从不写入这张绑定表。所以群里发 `/stop`、`/info` 等命令，只会作用于发送者自己绑定的私聊 session，或者直接报“未绑定”。
3. **`from_user_did` 语义偏差**：在群聊里，它取的是批次中最后一条消息的发送者，不一定是实际触发请求的人。
4. **`from_user_did` 可能导致工具调用失败**：`agent_tool` 里有若干参数结构带 `#[serde(deny_unknown_fields)]`，包括 `GrepArgs`、`GlobArgs`、`GetSessionArgs`、`BindExternalWorkspaceArgs`、`ListExternalWorkspacesArgs` 和 `agent_attention_signal` 的 Discover* 系列。当 session 有 `peer_did` 且这些工具以 function tool 方式调用时，注入的字段可能导致 `InvalidArgs`。
5. **`inject_background_environment` 不生效**（§2.4）。
6. **群聊 session 没有环境注入**：`groupchat_route` 没有配置 `on_wakeup`（§2.4）。
7. **群聊 session 用的是 `[session.ui].driver`**，而不是 `[session.group].driver`（§5.1）。
8. **Telegram 命令菜单缺 `/compress`**：`TG_BUILTIN_COMMANDS` 比 `BUILTIN_COMMANDS` 少了 `/compress`。另外，Telegram 群里常见的 `/cmd@botname` 写法不会被识别为命令，因为解析器不会剥离 `@botname`。
9. **BudgetExhausted 的 partial 输出只有文本**，结构化 block 无法随 partial 一起发出（§3.1）。
10. **过时注释**：
    - `command_dispatcher.rs` 中 `CommandOutcome` 的注释说回复“走和普通 LLM 回复相同的 Assistant 出站路径”，与 §4.3 不符。
    - `ai_runtime.rs` 中 `SessionDepsInput.from_user_did` 的注释说一对一时取值为 owner DID，与 §5.4 不符。
    - `agent_session.rs` 中说 user-input 段“仍由 legacy `compose_turn_message` 组装”的注释，已不能完整描述现状。

---

## 8. 开放问题

- **群聊的发送者身份如何呈现给 LLM**：是扩展 `od.msg/1` 的信封，增加 from、from_name、relation 等字段，还是另行定义群聊专用格式？这决定了 §7.1 群聊相关各项的落地方式。
- **群聊回复目标**：取群 DID（需要在 `SessionMeta` 中保存 `group_id`，目前 `PendingInput::Msg` 不携带它），还是按规则区分私聊回复与群回复？
- **Channel 类会话**：单向广播场景的协议细节（谁能发言、Agent 能否主动 push）尚未定义。
- **同一用户跨多个端点**：按 Message Center 规范，每条连接各自是独立 session，不跨连接合并。Agent 侧 `per_peer` 绑定按 `from` 区分，与此一致。如果需要“同一个人在 Telegram 和 WebUI 共享上下文”，要单独设计。

---

## 附录 A：术语表

| 术语 | 含义 |
|---|---|
| **MsgObject** | `ndn-lib` 定义的不可变消息对象，包含 envelope、content（正文、refs、machine）、thread 和 meta。`kind` 取值为 `Chat / GroupMsg / Deliver / Notify / Event / Operation` |
| **MailboxRecord** | owner 本地对某条消息的引用，归属某个 `MailboxKind`，带 `RecipientState` |
| **DeliveryRecord** | `DELIVERY_QUEUE` 中的一条投递任务，owner 是执行它的 `transport_did`，带 `DeliveryState` |
| **IngressContext** | 入站元数据，只用于审计，不参与路由 |
| **影子 DID** | `did:msgtunnel:*`，tunnel 端点在本 Zone 内的身份，自带路由信息 |
| **AiMessage / AiContent** | provider 中立的推理消息，由有序的 content block 组成，定义见 `kernel/buckyos-api/src/aicc_client.rs` |
| **od.msg/1** | OpenDAN 把一批入站消息渲染成 user 文本时使用的 JSON 格式 |
| **Inbound** | pump 交给 agent 主循环的项：`Msg / Command / Event` |
| **PendingInput** | session 持久化的待处理输入：`Msg / Event / Interrupt` |
| **SessionKind** | session 运行形态：`Ui / Work / SelfCheck / SelfImprove` |
| **Session class** | `agent.toml [session.<name>]`，决定 kind、默认 behavior、session_id 策略和 driver；由 dispatch 规则选定 |
| **Driver** | session 层对各 hook 点（on_init / on_wakeup / ...）拉取消息和事件的策略配置 |
| **from_user_did** | runtime 注入到工具参数中的发起人 DID，取值为 `meta.peer_did` |

## 附录 B：功能 → 代码入口

| 功能 | 入口 |
|---|---|
| 入站拉取 | `msg_center_pump::{run, drain_box, deliver_record, lower_inbound_message}` |
| MsgObject → AiMessage | `msg_parser::{parse_msg_object_structured, msg_object_to_ai_message_structured}` |
| 命令识别 | `msg_parser::msg_object_control_command` |
| 命令执行与回复 | `command_dispatcher::run_command`、`AIAgent::dispatch_command_reply` |
| 分发与 session 解析 | `AIAgent::{dispatch_inbound, route_msg, resolve_msg_session_id, get_or_create_session}` |
| 队列与上限 | `AgentSession::enqueue_pending`、`enforce_pending_queue_limit` |
| 本次输入组装 | `AgentSession::render_on_wakeup_input_text`、`compose_turn_message`、`prompt_env::render_ai_message_batch` |
| 环境信息 | behavior `[prompt].on_wakeup`、`AgentSession::load_changed_background_hits` |
| `from_user_did` | `AgentSession::current_from_user_did`、`OpendanToolAdapter::call_tool` |
| AiMessage → MsgObject | `msg_parser::ai_message_to_msg_object_with_base_validated_async` |
| 出站附件策略 | `attachment_policy::WorkspaceAttachmentValidator`、`attachment_resolver::NamedStoreLocalLinkResolver` |
| 出站发送 | `AgentSession::{post_outbound_message, post_outbound_error}` |
| 出站配置 | `agent.toml [runtime] preserve_attachment_tag_in_egress / filesystem_policy` |
| TG 入站与命令菜单 | `tg_tunnel::{dispatch_incoming_message, register_tg_builtin_commands}` |
| msg-center 分发与投递 | `msg_center::{dispatch_internal, derive_session_id, post_send_internal, build_delivery_envelope}`、`main::pump_delivery_queue_once` |

### H4 最终交付投影

Session 将接受的 report、稳定产物引用和可选 result 保存为 `ReportSubmission`，工具来源关联 run / call_id，XML 来源关联 run / step_index。只有最终提交生成 `ReportDelivery` worklog 和一条机械生成的 assistant 消息，不增加推理 Round；阶段报告不产生交付消息。原始调用 / 回执保留，重建模型历史时同一最终交付不重复展示。子 context 的最终报告只返回调用方，不关闭父 Session。
