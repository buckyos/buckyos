# MsgObject 协议对象定义

## 1. 定义目标

`MsgObject` 是 BuckyOS 系统的通用消息抽象。它不是 Message Tunnel 的附属类型，也不只服务于 BuckyOS 内部 DID 消息；它应能表达内部消息、外部 IM 消息、系统事件、投递回执、Agent 流式输出和未来未知平台消息。

`MsgObject` 是需要长期存储和跨组件传输的协议级对象，因此必须满足：

1. **稳定编码**：使用 CYFS canonical JSON 形成稳定 `ObjectId`；可选字段缺省时省略，不能用 `null` 表达缺省。
2. **不可变**：一条消息事实生成后不修改。投递状态、已读状态、重试、删除、归档等属于 `MsgRecord`、`MsgReceiptObj` 或其它状态对象。
3. **字段精简**：同一事实只出现一次。作者只在 `from` 中表达；会话、群、组件和外部地址都作为 `to` 目标表达；平台细节放入扩展字段。
4. **内外兼容**：BuckyOS DID 是一种 endpoint，不是唯一 endpoint。外部账号、外部会话、邮件地址、Webhook 来源、未知平台对象都必须能被表达。
5. **可路由**：只看 `MsgObject` 本身，应能判断消息从哪里进入、逻辑目标在哪里、需要哪个组件消费。
6. **可流式**：支持 Agent / AI 的 token delta、工具调用片段、最终答案、错误结束等流式交互。

本文重新定义协议对象，不要求兼容旧 `MsgObject`。

## 2. 顶层结构

```rust
pub struct MsgObject {
    pub ver: u16,
    pub kind: MsgKind,
    pub from: MsgEndpoint,
    pub to: Vec<MsgTarget>,
    #[serde(skip_serializing_if = "Option::is_none", default)]
    pub via: Option<MsgTransport>,
    #[serde(skip_serializing_if = "Option::is_none", default)]
    pub thread: Option<MsgThread>,
    pub created_at_ms: u64,
    #[serde(skip_serializing_if = "Option::is_none", default)]
    pub expires_at_ms: Option<u64>,
    #[serde(skip_serializing_if = "Option::is_none", default)]
    pub nonce: Option<String>,
    pub content: MsgContent,
    #[serde(skip_serializing_if = "Option::is_none", default)]
    pub stream: Option<MsgStream>,
    #[serde(skip_serializing_if = "BTreeMap::is_empty", default)]
    pub ext: BTreeMap<String, serde_json::Value>,
    #[serde(skip_serializing_if = "Option::is_none", default)]
    pub proof: Option<String>,
}
```

### 2.1 字段说明

| 字段 | 必填 | 含义 | 应用场景 |
| --- | --- | --- | --- |
| `ver` | 是 | MsgObject 协议版本。当前为 `1`。 | 解码、升级、兼容性判断。 |
| `kind` | 是 | 消息事实类型。 | UI 渲染、Agent 分发、事件流区分。 |
| `from` | 是 | 消息的逻辑作者或触发者，只表达一次。 | 联系人策略、权限判断、作者展示、回复默认目标。 |
| `to` | 是 | 一个或多个逻辑目标。目标可以是 DID、外部会话、外部账号、组件或 URI。 | 消息投递、群消息、组件消费、多播。 |
| `via` | 否 | 消息进入或计划离开系统所经过的传输通道。 | 判断来自哪个 tunnel、应该交给哪个 tunnel 发送。 |
| `thread` | 否 | 会话、主题、回复、关联任务等轻量线索。 | UI 聚合、邮件 thread、群 topic、Agent session。 |
| `created_at_ms` | 是 | 消息事实产生时间，Unix timestamp 毫秒。 | 排序、归档、过期判断。 |
| `expires_at_ms` | 否 | 消息事实建议过期时间。 | 临时通知、typing、短期状态消息。 |
| `nonce` | 否 | 发送方生成的去重扰动值，字符串而非数字。 | 避免同一作者同一时间同一内容产生相同对象。 |
| `content` | 是 | 标准化内容。 | 文本、富文本 fallback、附件引用、机器可读数据。 |
| `stream` | 否 | 流式消息帧信息。 | AI token delta、工具调用进度、最终帧、错误帧。 |
| `ext` | 否 | 命名空间扩展。 | Message Tunnel、平台原始 payload、UI hint、实验字段。 |
| `proof` | 否 | 可验证签名或 proof。 | 跨 Zone 可信消息、审计、归档验证。 |

## 3. 消息类型

```rust
pub enum MsgKind {
    Chat,
    Event,
    Command,
    Notice,
    Receipt,
    Stream,
    Unknown(String),
}
```

- `Chat`：普通对话消息。包括人、Agent、群、外部 IM 和 Email 的正文消息。
- `Event`：状态变化或平台事件，例如消息编辑、删除、成员加入、授权变化。
- `Command`：明确要求某组件执行的控制消息，例如 Agent 工具调用请求、系统操作请求。
- `Notice`：通知类消息，例如系统提示、任务进度、告警。
- `Receipt`：回执类消息，例如 delivered、read、failed、ack。
- `Stream`：流式交互帧。最终答案也可以使用 `Chat` 并带同一个 `stream_id`，但中间 delta 应使用 `Stream`。
- `Unknown(String)`：未来新增类型。旧系统应保留并按最低可用语义处理。

## 4. Endpoint 与 Target

### 4.1 MsgEndpoint

```rust
#[serde(tag = "type", rename_all = "snake_case")]
pub enum MsgEndpoint {
    Did {
        did: DID,
    },
    External {
        system: String,
        kind: String,
        id: String,
        #[serde(skip_serializing_if = "Option::is_none", default)]
        tenant_id: Option<String>,
        #[serde(skip_serializing_if = "Option::is_none", default)]
        display: Option<String>,
    },
    Component {
        service: String,
        #[serde(skip_serializing_if = "Option::is_none", default)]
        instance: Option<String>,
    },
    Uri {
        uri: String,
        #[serde(skip_serializing_if = "Option::is_none", default)]
        kind: Option<String>,
    },
    Anonymous {
        #[serde(skip_serializing_if = "Option::is_none", default)]
        label: Option<String>,
    },
}
```

`MsgEndpoint` 统一表达身份、地址、组件和外部对象：

- `Did`：BuckyOS 内部实体，可能是 Owner、Device、Agent、Zone、self-host group。
- `External`：外部系统对象，例如 Telegram user、Telegram chat、Lark open_id、Email address、Slack channel。`system` 使用稳定小写名字，如 `telegram`、`lark`、`email`；`kind` 使用平台语义，如 `user`、`bot`、`group`、`channel`、`message_thread`。
- `Component`：BuckyOS 内部消费组件，如 `msg_center`、`agent_runtime`、`message_tunnel`、`workflow`。
- `Uri`：无法或不应拆解为 DID / 外部账号的目标，例如 `mailto:`、`https:`、`cyfs:`。
- `Anonymous`：无法稳定识别但仍需保留“有一个来源/目标”的场景。

外部对象不应被强制映射为 DID。只有当需要长期授权、联系人合并、跨系统寻址或用户明确绑定时，才通过 ContactMgr / GroupMgr 建立 DID 映射。

### 4.2 MsgTarget

```rust
pub struct MsgTarget {
    pub endpoint: MsgEndpoint,
    #[serde(skip_serializing_if = "Option::is_none", default)]
    pub role: Option<MsgTargetRole>,
    #[serde(skip_serializing_if = "Vec::is_empty", default)]
    pub consumers: Vec<MsgConsumer>,
    #[serde(skip_serializing_if = "Option::is_none", default)]
    pub delivery: Option<MsgDeliveryHint>,
}

pub enum MsgTargetRole {
    To,
    Cc,
    Bcc,
    Conversation,
    Observer,
    Component,
    ReplyTarget,
    Unknown(String),
}

pub struct MsgConsumer {
    pub component: MsgEndpoint,
    #[serde(skip_serializing_if = "Option::is_none", default)]
    pub reason: Option<String>,
}

pub struct MsgDeliveryHint {
    pub transport: MsgTransport,
    #[serde(skip_serializing_if = "Option::is_none", default)]
    pub priority: Option<i32>,
    #[serde(skip_serializing_if = "Option::is_none", default)]
    pub mode: Option<String>,
}
```

`to` 不再只是 `Vec<DID>`，而是目标列表：

- 单聊：`to=[{ endpoint: Did(agent_did), role: To }]`。
- self-host group：`from=Did(author_did)`，`to=[{ endpoint: Did(group_did), role: Conversation }]`。作者不再额外写 `source`。
- Telegram 群入站：`from=External{system:"telegram", kind:"user", id:"..."}`，`to=[{ endpoint: External{system:"telegram", kind:"group", id:"..."}, role: Conversation, consumers:[Component{service:"agent_runtime"}] }]`。
- 出站到外部平台：目标 endpoint 表达外部会话或外部账号，`delivery.transport` 指定应由哪个 tunnel 消费。
- 组件命令：目标 endpoint 可以是 `Component{service:"workflow"}`，`role=Component`。

这样系统只看 `MsgObject` 就能知道：

1. `from`：消息逻辑上来自谁。
2. `via`：消息通过哪个通道进入或计划离开。
3. `to[].endpoint`：应投递到哪个逻辑目标，可多个。
4. `to[].consumers`：应由哪个组件消费。
5. `to[].delivery`：需要哪个传输组件执行实际发送。

## 5. Transport

```rust
pub struct MsgTransport {
    pub kind: String,
    #[serde(skip_serializing_if = "Option::is_none", default)]
    pub id: Option<MsgEndpoint>,
    #[serde(skip_serializing_if = "Option::is_none", default)]
    pub account: Option<MsgEndpoint>,
    #[serde(skip_serializing_if = "Option::is_none", default)]
    pub direction: Option<MsgTransportDirection>,
}

pub enum MsgTransportDirection {
    Ingress,
    Egress,
    Internal,
    Unknown(String),
}
```

- `kind`：传输类型，例如 `message_tunnel`、`native_dispatch`、`local`、`webhook`。
- `id`：传输实例。Message Tunnel 场景下通常是 `Did(tunnel_did)`。
- `account`：传输账号。Telegram Bot、Lark Bot、Email account 等都放这里。
- `direction`：入站、出站或内部。

`via` 表示“这条消息事实经过的通道”，不是作者，也不是收件人。它避免把 tunnel 账号重复塞进 `from` 或 `to`。

## 6. Thread

```rust
pub struct MsgThread {
    #[serde(skip_serializing_if = "Option::is_none", default)]
    pub id: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none", default)]
    pub topic: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none", default)]
    pub reply_to: Option<ObjId>,
    #[serde(skip_serializing_if = "Option::is_none", default)]
    pub external_reply_to: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none", default)]
    pub correlation_id: Option<String>,
}
```

- `id`：稳定 thread/session key。可来自 Telegram chat、Lark chat、Email Message-ID thread、Agent session。
- `topic`：展示或分组 hint，例如群 topic 名称。
- `reply_to`：回复的 BuckyOS `MsgObjectId`。
- `external_reply_to`：还不能映射为 `MsgObjectId` 的外部消息引用，例如 Telegram `chat_id/message_id` 或 Email `Message-ID`。
- `correlation_id`：跨消息串联任务、Agent run、workflow run。

Thread 是聚合线索，不是投递目标。投递目标必须在 `to` 中表达。

## 7. Content

```rust
pub struct MsgContent {
    #[serde(skip_serializing_if = "Option::is_none", default)]
    pub title: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none", default)]
    pub fallback: Option<String>,
    #[serde(skip_serializing_if = "Vec::is_empty", default)]
    pub parts: Vec<MsgPart>,
}

pub struct MsgPart {
    #[serde(skip_serializing_if = "Option::is_none", default)]
    pub id: Option<String>,
    pub kind: MsgPartKind,
    #[serde(skip_serializing_if = "Option::is_none", default)]
    pub mime: Option<String>,
    pub data: MsgPartData,
    #[serde(skip_serializing_if = "BTreeMap::is_empty", default)]
    pub ext: BTreeMap<String, serde_json::Value>,
}

pub enum MsgPartKind {
    Text,
    RichText,
    Json,
    Attachment,
    Mention,
    Quote,
    Reaction,
    ToolCall,
    ToolResult,
    Unknown(String),
}

#[serde(tag = "type", rename_all = "snake_case")]
pub enum MsgPartData {
    Text { value: String },
    Json { value: serde_json::Value },
    Ref { value: MsgRef },
    Empty,
}

pub struct MsgRef {
    pub kind: String,
    pub value: String,
    #[serde(skip_serializing_if = "Option::is_none", default)]
    pub mime: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none", default)]
    pub name: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none", default)]
    pub size: Option<u64>,
}
```

内容规则：

- 小文本可直接放 `Text`。
- 富文本、卡片、平台 post 应提供 `fallback` 或一个 `Text` part，同时把原始结构放入 `RichText/Json` part 或 `ext`。
- 大附件不进入 `MsgObject` body，只放 `MsgRef`，例如 `kind="object_id"`、`kind="cyfs_uri"`、`kind="external_file_id"`、`kind="url"`。
- 无法理解的平台内容使用 `Unknown`，但必须保留 fallback 或 raw 引用。
- `content.fallback` 是最低可显示语义，Agent 或 UI 在不理解 parts 时可以使用它。

## 8. Stream

```rust
pub struct MsgStream {
    pub stream_id: String,
    pub seq: u64,
    pub stage: MsgStreamStage,
    #[serde(skip_serializing_if = "Option::is_none", default)]
    pub prev: Option<ObjId>,
    #[serde(skip_serializing_if = "Option::is_none", default)]
    pub final_msg: Option<ObjId>,
}

pub enum MsgStreamStage {
    Start,
    Delta,
    Snapshot,
    End,
    Error,
    Unknown(String),
}
```

AI 流式交互使用多条不可变 `MsgObject` 表达：

1. 第一帧：`kind=Stream`，`stage=Start`，声明 `stream_id`。
2. 中间帧：`stage=Delta`，`seq` 单调递增，`content.parts` 放增量文本、工具调用片段或状态。
3. 快照帧：`stage=Snapshot`，用于提供当前完整内容，便于断线恢复。
4. 结束帧：`stage=End`，可通过 `final_msg` 指向最终完整消息。
5. 错误帧：`stage=Error`，`content.parts` 放错误结构和可读 fallback。

流式帧仍是普通 `MsgObject`，可存储、可传输、可回放。消费者按 `stream_id + seq` 重建流。

## 9. 扩展字段规则

`ext` 是命名空间扩展，key 必须是稳定字符串：

- BuckyOS 官方扩展使用 `buckyos.<domain>`，例如 `buckyos.message_tunnel`。
- 平台扩展使用 `platform.<name>`，例如 `platform.telegram`。
- 应用扩展使用反向域名或应用 id，例如 `app.example.task`。

扩展规则：

1. 扩展字段只能追加，不能改变核心字段语义。
2. 不理解扩展的系统必须保留扩展或安全忽略，不能拒绝整条消息。
3. Secret 不得进入 `ext`。token、session、refresh token、私钥只能存 Secret Store。
4. 大型 raw payload 应保存为对象引用，并在扩展里放 `raw_ref`；小型 raw payload 可直接放 `raw`。
5. 若扩展字段影响路由，必须同时在核心字段中保留最低可用路由语义：`via`、`to`、`consumers`、`delivery` 不能只藏在扩展里。

## 10. Message Tunnel 扩展

Message Tunnel 场景使用 `ext["buckyos.message_tunnel"]`。该扩展用于无损保留外部平台语义，但核心路由仍必须出现在 `from`、`to`、`via` 和 `to[].delivery` 中。

```json
{
  "schema": 1,
  "platform": "telegram",
  "account_kind": "bot",
  "direction": "ingress",
  "external_message": {
    "conversation_id": "-100123",
    "message_id": "456",
    "thread_id": "789",
    "update_id": "998877",
    "sent_at_ms": 1710000000000
  },
  "conversation": {
    "id": "-100123",
    "kind": "group",
    "title": "BuckyOS Dev"
  },
  "actor": {
    "id": "12345",
    "kind": "user",
    "display": "Alice"
  },
  "reply_to": {
    "conversation_id": "-100123",
    "message_id": "455"
  },
  "mentions": [],
  "attachments": [],
  "idempotency_key": "ingress:telegram:bot_1:-100123:456",
  "checkpoint": {
    "source": "bot_1",
    "cursor": "998877"
  },
  "raw": {}
}
```

### 10.1 扩展字段定义

| 字段 | 必填 | 含义 | 用法 |
| --- | --- | --- | --- |
| `schema` | 是 | Message Tunnel 扩展版本。当前为 `1`。 | 扩展内部升级。 |
| `platform` | 是 | 外部平台名。 | 选择平台 adapter、诊断、审计。 |
| `account_kind` | 否 | tunnel 账号形态：`bot`、`user`、`system`、`service`。 | 能力裁剪、审计。 |
| `direction` | 是 | `ingress` 或 `egress`。 | 区分入站封装和出站投递意图。 |
| `external_message` | 否 | 平台消息引用。 | 去重、回复、编辑/删除事件关联、回放。 |
| `conversation` | 否 | 平台会话引用。 | 回投来源会话、UI session、群/topic/thread 还原。 |
| `actor` | 否 | 平台 actor 引用。 | 展示、联系人解析、审计。 |
| `reply_to` | 否 | 平台回复目标。 | 出站 reply、入站引用还原。 |
| `mentions` | 否 | 平台 mention token 列表。 | 解析 Agent、@all、角色、未知 mention。 |
| `attachments` | 否 | 平台附件引用和导入结果。 | 入站附件保存、出站平台文件复用、失败降级。 |
| `idempotency_key` | 否 | 入站或出站幂等键。 | 防重复 dispatch / send。 |
| `checkpoint` | 否 | 平台游标引用。 | 成功写入 MsgCenter 后提交 checkpoint。 |
| `raw` | 否 | 小型原始 payload。 | 无损诊断、未来重放。 |
| `raw_ref` | 否 | 大型或敏感 raw payload 的对象引用。 | 避免 MsgObject 过大；仍保持可追溯。 |

### 10.2 入站用法

Tunnel 收到外部消息时：

1. `from` 写外部 actor endpoint；如果已绑定 DID，也可以写 DID，但外部 actor 仍应在扩展中保留。
2. `to` 写外部 conversation endpoint 或解析出的 DID 目标；如果目标 Agent 已确定，写入 `consumers=[Component{service:"agent_runtime"}]`。
3. `via.kind="message_tunnel"`，`via.id=Did(tunnel_did)`，`via.account=External{system, kind, id}`，`via.direction=Ingress`。
4. 标准文本、附件引用、fallback 进入 `content`。
5. 平台消息 ID、chat ID、thread、mention、raw payload 进入 `ext["buckyos.message_tunnel"]`。
6. 幂等键至少包含 `platform + tunnel account + conversation_id + message_id/update_id`。

这样 `MsgCenter.dispatch` 不依赖外部 `IngressContext` 也能从 `MsgObject` 判断来源、目标和消费组件；`IngressContext` 仍可作为调用时的索引/权限上下文存在，但不是消息事实的唯一载体。

### 10.3 出站用法

系统要通过 tunnel 发外部消息时：

1. `from` 写 DID 作者，例如 Agent DID 或 Owner DID。
2. `to` 写外部 conversation / actor endpoint，或写 DID 并在 `delivery.transport` 中指定 tunnel。
3. `to[].delivery.transport.kind="message_tunnel"`，`transport.id=Did(tunnel_did)`。
4. 如果必须回复某条平台消息，在 `thread.external_reply_to` 和扩展 `reply_to` 中保留平台引用。
5. 若要发送平台卡片、按钮、特殊富文本，在 `content.parts` 放 fallback 和结构化 part，在扩展中放平台原始 payload。
6. 发送成功后生成 `Receipt` 或更新 `MsgRecord.delivery`，不要修改原始 `MsgObject`。

如果出站消息完全来自 `MsgObject` 标准字段，Tunnel 可直接按能力集转换；如果平台需要特殊结构，Tunnel 优先读取扩展字段，但不能要求所有上层都理解平台私有结构。

### 10.4 无损转换要求

Message Tunnel 与 `MsgObject` 的转换必须满足：

- 外部入站消息转为 `MsgObject` 后，至少能还原出平台发送 API 或回复 API 需要的 conversation、message、actor、thread、attachment、mention 和 raw 语义。
- 无法转换为 BuckyOS DID 的外部对象必须保留为 `External` endpoint 或扩展引用。
- 标准字段表达最低可用语义，扩展字段表达平台无损语义。
- 平台新增字段进入 `raw` / `raw_ref`，不能丢弃。
- 附件下载成功时保存对象引用；下载失败时保留外部 file id / url / mime / size。
- 外部编辑、删除、reaction、read receipt 等不应改写原消息对象，应生成新的 `Event` 或 `Receipt` 消息并通过扩展关联原平台消息。

## 11. 示例

### 11.1 内部 Agent 单聊

```json
{
  "ver": 1,
  "kind": "Chat",
  "from": { "type": "did", "did": "did:bns:agent_a" },
  "to": [
    {
      "endpoint": { "type": "did", "did": "did:bns:agent_b" },
      "role": "To",
      "consumers": [
        { "component": { "type": "component", "service": "agent_runtime" } }
      ]
    }
  ],
  "created_at_ms": 1710000000000,
  "content": {
    "fallback": "hello",
    "parts": [
      { "kind": "Text", "mime": "text/plain", "data": { "type": "text", "value": "hello" } }
    ]
  }
}
```

### 11.2 Telegram 群入站

```json
{
  "ver": 1,
  "kind": "Chat",
  "from": {
    "type": "external",
    "system": "telegram",
    "kind": "user",
    "id": "12345",
    "display": "Alice"
  },
  "to": [
    {
      "endpoint": {
        "type": "external",
        "system": "telegram",
        "kind": "group",
        "id": "-100123",
        "display": "BuckyOS Dev"
      },
      "role": "Conversation",
      "consumers": [
        { "component": { "type": "component", "service": "agent_runtime" }, "reason": "mentioned_bot" }
      ]
    }
  ],
  "via": {
    "kind": "message_tunnel",
    "id": { "type": "did", "did": "did:bns:tg_tunnel" },
    "account": { "type": "external", "system": "telegram", "kind": "bot", "id": "bucky_bot" },
    "direction": "Ingress"
  },
  "thread": { "id": "telegram:bucky_bot:-100123" },
  "created_at_ms": 1710000000000,
  "content": {
    "fallback": "@bucky summarize this",
    "parts": [
      { "kind": "Text", "mime": "text/plain", "data": { "type": "text", "value": "@bucky summarize this" } }
    ]
  },
  "ext": {
    "buckyos.message_tunnel": {
      "schema": 1,
      "platform": "telegram",
      "account_kind": "bot",
      "direction": "ingress",
      "external_message": {
        "conversation_id": "-100123",
        "message_id": "456",
        "update_id": "998877",
        "sent_at_ms": 1710000000000
      },
      "conversation": { "id": "-100123", "kind": "group", "title": "BuckyOS Dev" },
      "actor": { "id": "12345", "kind": "user", "display": "Alice" },
      "idempotency_key": "ingress:telegram:bucky_bot:-100123:456"
    }
  }
}
```

### 11.3 AI 流式 delta

```json
{
  "ver": 1,
  "kind": "Stream",
  "from": { "type": "did", "did": "did:bns:jarvis" },
  "to": [
    { "endpoint": { "type": "did", "did": "did:bns:owner" }, "role": "To" }
  ],
  "created_at_ms": 1710000000100,
  "content": {
    "parts": [
      { "kind": "Text", "mime": "text/plain", "data": { "type": "text", "value": "The backup " } }
    ]
  },
  "stream": {
    "stream_id": "run_abc/message_1",
    "seq": 12,
    "stage": "Delta"
  }
}
```

## 12. 与 MsgRecord / MsgReceiptObj 的边界

- `MsgObject` 保存消息事实。
- `MsgRecord` 保存某个 owner 在某个 box 中看到这条消息的状态、路由、重试、删除、归档和 UI 索引。
- `MsgReceiptObj` 保存阅读、接受、拒绝、隔离等 reader 维度回执。
- `DeliveryInfo` 保存外部投递结果，例如 external message id、重试次数、错误码。

不进入 `MsgObject` 的信息：

- 当前是否已读。
- 当前是否发送成功。
- 第几次重试。
- 某个用户是否删除或归档。
- 某个 tunnel 当前锁定发送。
- Secret、token、session。

## 13. 版本与扩展策略

- 初始协议版本：`ver=1`。
- 新字段必须可选，旧系统忽略后仍保留核心语义。
- 核心字段语义冻结：`ver`、`kind`、`from`、`to`、`via`、`created_at_ms`、`content`。
- 枚举新增项必须提供 `Unknown(String)` 或等价兼容能力。
- 不兼容变更必须提升 `ver`，不能在同一版本里改变字段含义。
- 扩展字段版本由扩展自身维护，例如 `ext["buckyos.message_tunnel"].schema`。
