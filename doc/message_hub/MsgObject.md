# MsgObject 协议对象定义

## 1. 定义目标

`MsgObject` 是 BuckyOS 系统的通用消息抽象。它不是 Message Tunnel 的附属类型，也不只服务于 BuckyOS 内部 DID 消息；它应能表达内部消息、外部 IM 消息、系统事件、投递回执、Agent 流式输出和未来未知平台消息。

`MsgObject` 是需要长期存储和跨组件传输的协议级对象，因此必须满足：

1. **稳定编码**：使用 CYFS canonical JSON 形成稳定 `ObjectId`；可选字段缺省时省略，不能用 `null` 表达缺省。
2. **不可变**：一条消息事实生成后不修改。投递状态、已读状态、重试、删除、归档等属于 `MsgRecord`、`MsgReceiptObj` 或其它状态对象。
3. **字段精简**：同一事实只出现一次。作者只在 `from` 表达；会话、群、组件和外部地址都作为 `to` 目标表达；通道只在 `via` / `to[].delivery` 表达；平台差异放入扩展字段。
4. **内外兼容**：BuckyOS DID 是一种 endpoint，不是唯一 endpoint。外部账号、外部会话、邮件地址、Webhook 来源、未知平台对象都必须能被表达，不应被强制映射为 DID。
5. **可路由**：只看 `MsgObject` 本身，应能判断消息从哪里进入、逻辑目标在哪里、可能投递到哪些目标、应由哪个组件消费。
6. **可流式**：支持 Agent / AI 的 token delta、工具调用片段、快照、最终答案、错误结束等流式交互。

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
    #[serde(skip_serializing_if = "MsgSeq::is_none", default)]
    pub seq: MsgSeq,
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

#[serde(tag = "type", content = "value", rename_all = "snake_case")]
pub enum MsgSeq {
    U64(u64),
    Str(String),
    None,
}
```

### 2.1 字段说明

| 字段 | 必填 | 含义 | 应用场景 |
| --- | --- | --- | --- |
| `ver` | 是 | MsgObject 协议版本。当前为 `1`。 | 解码、升级、兼容性判断。 |
| `kind` | 是 | 消息事实类型。 | UI 渲染、Agent 分发、事件流区分。 |
| `from` | 是 | 消息的逻辑作者或触发者，只表达一次。 | 联系人策略、权限判断、作者展示、回复默认目标。 |
| `to` | 是 | 一个或多个逻辑目标。目标可以是 DID、外部会话、外部账号、组件或 URI。 | 消息投递、群消息、组件消费、多播。 |
| `via` | 否 | 消息进入系统或计划离开系统所经过的传输通道。 | 判断来自哪个 tunnel、应该交给哪个 tunnel 发送。 |
| `thread` | 否 | 会话、主题、回复、关联任务等轻量线索。 | UI 聚合、邮件 thread、群 topic、Agent session。 |
| `seq` | 否 | 消息在某个排序范围内的顺序号。`None` 不参与序列化。 | 外部平台消息顺序、Agent session 顺序、AI 流式帧重组。 |
| `created_at_ms` | 是 | 消息事实产生时间，Unix timestamp 毫秒。 | 归档、过期判断、缺少 `seq` 时的展示级兜底排序。 |
| `expires_at_ms` | 否 | 消息事实建议过期时间。 | 临时通知、typing、短期状态消息。 |
| `nonce` | 否 | 发送方生成的去重扰动值，字符串而非数字。 | 避免同一作者同一时间同一内容产生相同对象。 |
| `content` | 是 | 标准化内容。 | 文本、富文本 fallback、非结构化数据、机器可读数据。 |
| `stream` | 否 | 流式消息帧信息，不再包含序号。 | AI token delta、工具调用进度、最终帧、错误帧。 |
| `ext` | 否 | 命名空间扩展。 | Message Tunnel、平台原始 payload、UI hint、实验字段。 |
| `proof` | 否 | 可验证签名或 proof。 | 跨 Zone 可信消息、审计、归档验证。 |

### 2.2 顺序语义

`seq` 只在同一排序范围内有意义，不提供全局顺序。排序范围由业务语义决定，通常是 `to[].endpoint`、`thread.id`、`stream.stream_id`、平台 conversation 或 Agent session。

- `MsgSeq::U64` 用于平台提供单调数值、Agent Runtime 自增帧号、队列 offset。
- `MsgSeq::Str` 用于平台消息 ID、邮件 Message-ID、复合 cursor 等只能稳定按字符串比较或由 adapter 自定义排序的场景。
- `MsgSeq::None` 表示没有协议级顺序；序列化时省略 `seq` 字段。
- 同一排序范围内必须固定一种编码，不能一部分使用 `U64`、一部分使用 `Str`。
- `created_at_ms` 不是顺序字段，只能作为展示和恢复兜底。

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
- `Stream`：流式交互帧。中间 delta、工具调用片段、快照和错误帧都应使用该类型。
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

`to` 是逻辑目标列表，不只是收件人列表：

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

- `id`：稳定 thread/session key。可来自 Telegram topic、Lark thread、Email References、Agent session。
- `topic`：展示或分组 hint，例如群 topic 名称。
- `reply_to`：回复的 BuckyOS `MsgObjectId`。
- `external_reply_to`：还不能映射为 `MsgObjectId` 的外部消息引用，例如 Telegram `chat_id/message_id` 或 Email `Message-ID`。
- `correlation_id`：跨消息串联任务、Agent run、workflow run。

Thread 是聚合线索，不是投递目标。投递目标必须在 `to` 中表达。会话本身如果需要路由，应作为 `to[].endpoint` 表达。

## 7. Content 与非结构化数据

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
    Attachment { value: MsgAttachment },
    Empty,
}

pub struct MsgAttachment {
    #[serde(skip_serializing_if = "Option::is_none", default)]
    pub mime: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none", default)]
    pub name: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none", default)]
    pub size: Option<u64>,
    #[serde(skip_serializing_if = "Option::is_none", default)]
    pub digest: Option<String>,
    #[serde(skip_serializing_if = "BTreeMap::is_empty", default)]
    pub meta: BTreeMap<String, serde_json::Value>,
    pub body: MsgAttachmentBody,
}

#[serde(tag = "type", content = "value", rename_all = "snake_case")]
pub enum MsgAttachmentBody {
    Inline {
        encoding: String,
        value: String,
    },
    Url(String),
    ObjId(ObjId),
}
```

字段说明：

| 字段 | 必填 | 含义 | 应用场景 |
| --- | --- | --- | --- |
| `MsgContent.title` | 否 | 消息标题或摘要标题。 | 邮件标题、通知标题、卡片标题。 |
| `MsgContent.fallback` | 否 | 最低可显示文本。 | 旧客户端、搜索索引、无法理解富内容时展示。 |
| `MsgContent.parts` | 否 | 有序内容片段。 | 文本加附件、富文本加 fallback、工具调用结果、多模态消息。 |
| `MsgPart.id` | 否 | part 内稳定 ID，只需在本消息内唯一。 | Tunnel 扩展通过 `part_id` 关联 mention、附件平台 ID、导入状态。 |
| `MsgPart.kind` | 是 | 内容片段类型。 | UI 渲染、Agent 输入解析、附件处理。 |
| `MsgPart.mime` | 否 | 片段 MIME hint。 | 文本格式、图片类型、文件类型判断。 |
| `MsgPart.data` | 是 | 片段数据。 | 文本、JSON、附件或空占位。 |
| `MsgPart.ext` | 否 | part 级扩展。 | 只影响该片段的平台 token、渲染 hint、未知结构。 |
| `MsgAttachment.mime` | 否 | 附件 MIME 类型。 | 下载、预览、能力裁剪。 |
| `MsgAttachment.name` | 否 | 附件文件名或展示名。 | UI 展示、保存文件名。 |
| `MsgAttachment.size` | 否 | 附件字节数。 | 限额判断、下载前提示。 |
| `MsgAttachment.digest` | 否 | 内容摘要，建议格式为 `sha256:<hex>`。 | 去重、完整性校验、对象导入校验。 |
| `MsgAttachment.meta` | 否 | 跨平台通用附件元数据。 | 图片宽高、音视频时长、页数、预览文本。 |
| `MsgAttachment.body` | 是 | 附件数据位置或内联数据。 | 小数据内联、云端 URL、本地 NDN 对象引用。 |

内容规则：

- 小文本直接放 `MsgPartData::Text`，`content.fallback` 保存最低可显示语义。
- 富文本、卡片、平台 post 应提供 `fallback` 或一个 `Text` part，同时把结构化内容放入 `Json` part 或 Message Tunnel 扩展。
- 图片、文件、语音、视频、贴纸和未知二进制内容统一使用 `MsgAttachment`。
- `Inline` 用于小型、体积受控且确实需要随消息一起存储的数据，`encoding` 例如 `utf-8`、`base64`。
- `Url` 用于已在云端或外部系统可下载的数据，URL 可以是 `http`、`https`、`ftp`、`cyfs` 等。
- `ObjId` 用于已导入本地 NDN / NamedObject 系统的数据。
- `mime`、`name`、`size`、`digest` 和 `meta` 只保存跨平台通用元信息；`meta` 可保存 `width`、`height`、`duration_ms`、`preview_text` 等不绑定平台的附件元数据。平台 file id、缩略图 token、上传/下载状态不放在 `MsgAttachment`，应放入 Message Tunnel 扩展并通过 `part.id` 关联。
- 无法理解的平台内容使用 `Unknown`，但必须保留 fallback 或 raw 引用。

## 8. Stream

```rust
pub struct MsgStream {
    pub stream_id: String,
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

1. 第一帧：`kind=Stream`，`stage=Start`，声明 `stream_id`，顶层 `seq=U64(0)` 或平台定义的起始序号。
2. 中间帧：`stage=Delta`，顶层 `seq` 单调递增，`content.parts` 放增量文本、工具调用片段或状态。
3. 快照帧：`stage=Snapshot`，用于提供当前完整内容，便于断线恢复。
4. 结束帧：`stage=End`，可通过 `final_msg` 指向最终完整消息。
5. 错误帧：`stage=Error`，`content.parts` 放错误结构和可读 fallback。

流式帧仍是普通 `MsgObject`，可存储、可传输、可回放。消费者按 `stream.stream_id + MsgObject.seq` 重建流。

## 9. 扩展字段规则

`ext` 是命名空间扩展，key 必须是稳定字符串：

- BuckyOS 官方扩展使用 `buckyos.<domain>`，例如 `buckyos.message_tunnel`。
- 平台扩展使用 `platform.<name>`，例如 `platform.telegram`。
- 应用扩展使用反向域名或应用 id，例如 `app.example.task`。

扩展规则：

1. 扩展字段只能追加，不能改变核心字段语义。
2. 不理解扩展的系统必须保留扩展或安全忽略，不能拒绝整条消息。
3. 已经由核心字段表达的信息不能重复出现在扩展中。`from`、`to`、`via`、`thread`、`seq`、`content` 是事实来源，扩展字段只能补充平台差异。
4. Secret 不得进入 `ext`。token、session、refresh token、私钥只能存 Secret Store。
5. 大型 raw payload 应保存为对象引用，并在扩展里放 `raw_ref`；小型 raw payload 可直接放 `raw`。
6. 若扩展字段影响路由，必须同时在核心字段中保留最低可用路由语义：`via`、`to`、`consumers`、`delivery` 不能只藏在扩展里。

## 10. Message Tunnel 扩展

Message Tunnel 场景使用 `ext["buckyos.message_tunnel"]`。该扩展用于无损保留外部平台语义，但核心路由必须只由 `from`、`to`、`via`、`thread`、`seq`、`to[].consumers` 和 `to[].delivery` 表达。

由于 `via` 已经表达 tunnel 类型、实例、账号和方向，扩展中不再重复 `platform`、`account_kind`、`direction`。由于 `from` 和 `to` 已经表达 actor 与 conversation，扩展中不再重复 actor id、conversation id 或 display；只保存核心字段无法表达的补充信息。

```json
{
  "schema": 1,
  "external_message": {
    "message_id": "456",
    "update_id": "998877",
    "sent_at_ms": 1710000000000,
    "edit_version": "2"
  },
  "external_thread": {
    "topic_id": "789",
    "references": ["<email-message-id@example.com>"]
  },
  "reply_ref": {
    "message_id": "455",
    "quote_offset": 0,
    "quote_length": 18
  },
  "mentions": [
    {
      "part_id": "p1",
      "token": "@bucky",
      "kind": "bot_mention",
      "resolved": { "type": "did", "did": "did:bns:jarvis" }
    }
  ],
  "attachments": [
    {
      "part_id": "file1",
      "platform_file_id": "AgACAgUAAxkBAA",
      "thumbnail_file_id": "thumb_1",
      "import_state": "imported"
    }
  ],
  "actor_profile": {
    "avatar_url": "https://example/avatar.png",
    "locale": "en"
  },
  "conversation_profile": {
    "member_count": 42,
    "permission_hint": "bot_can_reply"
  },
  "idempotency_key": "ingress:telegram:bucky_bot:-100123:456",
  "checkpoint": {
    "cursor": "998877"
  },
  "raw_ref": {
    "type": "obj_id",
    "value": "obj_xxx"
  }
}
```

### 10.1 扩展字段定义

| 字段 | 必填 | 含义 | 用法 |
| --- | --- | --- | --- |
| `schema` | 是 | Message Tunnel 扩展版本。当前为 `1`。 | 扩展内部升级。 |
| `external_message` | 否 | 平台消息对象中未进入核心字段的消息级信息。 | 去重、编辑/删除事件关联、回放、诊断。 |
| `external_thread` | 否 | 平台 thread/topic/email references 中无法放入 `thread` 的细节。 | 群 topic、邮件 References、平台 thread 参数还原。 |
| `reply_ref` | 否 | 平台回复目标中无法放入 `thread.external_reply_to` 的补充信息。 | 出站 reply、引用片段、quote range。 |
| `mentions` | 否 | 平台 mention token 列表和解析结果。 | 解析 Agent、@all、角色、未知 mention；通过 `part_id` 关联正文。 |
| `attachments` | 否 | 平台附件引用、缩略图、导入和复用状态。 | 入站附件保存、出站平台文件复用、失败降级；通过 `part_id` 关联 `MsgAttachment`。 |
| `actor_profile` | 否 | `from` 无法表达的外部 actor 补充资料。 | 展示 hint、联系人解析、审计；不得重复 actor id。 |
| `conversation_profile` | 否 | `to[].endpoint` 无法表达的外部会话补充资料。 | 成员数、平台权限、临时能力；不得重复 conversation id。 |
| `outbound_payload` | 否 | 出站平台特殊 payload。 | 卡片、按钮、模板消息等标准 `content` 无法完整表达的内容。 |
| `capability_overrides` | 否 | 针对本条消息的能力裁剪或降级要求。 | 禁用 markdown、要求静默发送、要求文本 fallback。 |
| `idempotency_key` | 否 | 入站或出站幂等键。 | 防重复 dispatch / send。 |
| `checkpoint` | 否 | 平台游标引用。 | 成功写入 MsgCenter 后提交 checkpoint。 |
| `raw` | 否 | 小型原始 payload。 | 无损诊断、未来重放。 |
| `raw_ref` | 否 | 大型或敏感 raw payload 的对象引用。 | 避免 MsgObject 过大；仍保持可追溯。 |

### 10.2 入站用法

Tunnel 收到外部消息时：

1. `from` 写外部 actor endpoint；如果已绑定 DID，可写 DID，但外部 actor 应通过 ContactMgr 映射记录追溯，不在扩展里重复 actor id。
2. `to` 写外部 conversation endpoint、解析出的 DID 目标或消费组件；如果目标 Agent 已确定，写入 `consumers=[Component{service:"agent_runtime"}]`。
3. `via.kind="message_tunnel"`，`via.id=Did(tunnel_did)`，`via.account=External{system, kind, id}`，`via.direction=Ingress`。
4. 平台顺序或可排序平台消息 ID 写入顶层 `seq`。
5. 平台 thread/topic/reply 的最低可聚合语义写入 `thread`。
6. 标准文本、附件、fallback 进入 `content`；非结构化数据使用 `MsgAttachment`。
7. 平台消息 ID、update ID、mention token、平台文件 ID、checkpoint、无法标准化字段进入 `ext["buckyos.message_tunnel"]`。
8. 幂等键至少包含 `platform + tunnel account + conversation_id + message_id/update_id`；其中 platform、account、conversation 来自 `via` 和 `to`，扩展只保存生成后的 `idempotency_key`。

这样 `MsgCenter.dispatch` 不依赖外部 `IngressContext` 也能从 `MsgObject` 判断来源、目标、投递范围和消费组件；`IngressContext` 仍可作为调用时的索引/权限上下文存在，但不是消息事实的唯一载体。

### 10.3 出站用法

系统要通过 tunnel 发外部消息时：

1. `from` 写 DID 作者，例如 Agent DID 或 Owner DID。
2. `to` 写外部 conversation / actor endpoint，或写 DID 并在 `delivery.transport` 中指定 tunnel。
3. `to[].delivery.transport.kind="message_tunnel"`，`transport.id=Did(tunnel_did)`。
4. 如果必须回复某条平台消息，在 `thread.external_reply_to` 和扩展 `reply_ref` 中保留平台引用；不要在扩展中重复 `thread.id`。
5. 若要发送平台卡片、按钮、特殊富文本，在 `content.parts` 放 fallback 和结构化 part，在扩展 `outbound_payload` 中放平台特殊结构。
6. 发送成功后生成 `Receipt` 或更新 `MsgRecord.delivery`，不要修改原始 `MsgObject`。

如果出站消息完全来自 `MsgObject` 标准字段，Tunnel 可直接按能力集转换；如果平台需要特殊结构，Tunnel 优先读取扩展字段，但不能要求所有上层都理解平台私有结构。

### 10.4 无损转换要求

Message Tunnel 与 `MsgObject` 的转换必须满足：

- 外部入站消息转为 `MsgObject` 后，至少能还原出平台发送 API 或回复 API 需要的 conversation、message、actor、thread、attachment、mention 和 raw 语义。
- 无法转换为 BuckyOS DID 的外部对象必须保留为 `External` endpoint 或扩展引用。
- 标准字段表达最低可用语义，扩展字段表达平台无损语义。
- 平台新增字段进入 `raw` / `raw_ref`，不能丢弃。
- 附件下载成功时保存 `MsgAttachmentBody::ObjId`；下载失败时保留 `MsgAttachmentBody::Url` 或平台 file id / mime / size。
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
  "seq": { "type": "u64", "value": 456 },
  "created_at_ms": 1710000000000,
  "content": {
    "fallback": "@bucky summarize this",
    "parts": [
      { "id": "p1", "kind": "Text", "mime": "text/plain", "data": { "type": "text", "value": "@bucky summarize this" } }
    ]
  },
  "ext": {
    "buckyos.message_tunnel": {
      "schema": 1,
      "external_message": {
        "message_id": "456",
        "update_id": "998877",
        "sent_at_ms": 1710000000000
      },
      "mentions": [
        { "part_id": "p1", "token": "@bucky", "kind": "bot_mention" }
      ],
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
  "seq": { "type": "u64", "value": 12 },
  "created_at_ms": 1710000000100,
  "content": {
    "parts": [
      { "kind": "Text", "mime": "text/plain", "data": { "type": "text", "value": "The backup " } }
    ]
  },
  "stream": {
    "stream_id": "run_abc/message_1",
    "stage": "Delta"
  }
}
```

### 11.4 图片附件

```json
{
  "ver": 1,
  "kind": "Chat",
  "from": { "type": "did", "did": "did:bns:owner" },
  "to": [
    { "endpoint": { "type": "did", "did": "did:bns:jarvis" }, "role": "To" }
  ],
  "created_at_ms": 1710000000200,
  "content": {
    "fallback": "[image] backup chart",
    "parts": [
      {
        "id": "img1",
        "kind": "Attachment",
        "mime": "image/png",
        "data": {
          "type": "attachment",
          "value": {
            "mime": "image/png",
            "name": "backup-chart.png",
            "size": 20480,
            "digest": "sha256:...",
            "meta": { "width": 1280, "height": 720 },
            "body": { "type": "obj_id", "value": "obj_xxx" }
          }
        }
      },
      { "kind": "Text", "mime": "text/plain", "data": { "type": "text", "value": "backup chart" } }
    ]
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
- 核心字段语义冻结：`ver`、`kind`、`from`、`to`、`via`、`thread`、`seq`、`created_at_ms`、`content`。
- 枚举新增项必须提供 `Unknown(String)` 或等价兼容能力。
- 不兼容变更必须提升 `ver`，不能在同一版本里改变字段含义。
- 扩展字段版本由扩展自身维护，例如 `ext["buckyos.message_tunnel"].schema`。
