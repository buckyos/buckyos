# Agent Message 协议改进:开发指导(合并版)

> **文档定位**:本文档合并以下两份输入,形成面向下一阶段开发的执行指南:
> - 协议设计文档:**Agent Message 协议改进**(描述目标协议形态)
> - 现状实现文档:**OpenDAN Agent Message Chain**(描述当前代码实现)
>
> 每个改进项都标注**当前实现位置、目标形态、需要修改的代码、协议强度**,作为后续 sprint 拆分任务的依据。

---

## 0. 整体对照:协议设计 ↔ 现有实现

| 协议层概念 | 当前代码对应 | 主要文件 |
|---|---|---|
| **Message Center 层视角** | `MsgObject` / `MsgRecordWithObject` | `msg_center/src/msg_center.rs` |
| **Session History 层视角** | `AiMessage` / LLMContext accumulate | `llm_context/src/*`, `opendan/src/agent_session.rs` |
| **协议边界(两层互转)** | `msg_parser.rs` | `llm_context/src/msg_parser.rs` |
| **Session 类型枚举** | `MsgObjKind::{Chat, GroupMsg, Event, ...}`(已部分存在) | `ndn_lib` |
| **Tunnel 抽象** | `tg_tunnel.rs` + `TunnelOutbox` 通用机制 | `msg_center/src/tg_tunnel.rs` |
| **入站路由上下文** | `IngressContext` + record route | `msg_center.rs` |
| **出站路由偏好** | `SendContext.preferred_tunnel` + `session.peer_tunnel_did` | `agent_session.rs` |

**核心结论**:协议设计与现状实现的基础架构是**自洽的**——`MsgObject` ↔ `AiMessage` 的两层结构、`msg_parser.rs` 的转换边界、`from_did`/`tunnel_did` 在 pump 中的传递,这些都已经满足协议改进的前提条件。本次改动**不需要重构骨架,只需要在既有 hook 点补全功能**。

---

## 1. 入站消息:Background Environment 注入

### 1.1 现状

- **载体已存在**:`agent_session.rs::compose_turn_message` 里有 "environment preamble" 概念,作为本轮 user `AiMessage` 的第一个 text block。
- **形态尚未规范**:目前是裸文本拼接,无明确标签包裹。

### 1.2 目标协议

环境信息以 XML 标签块承载,语法形态:

```xml
<background_environment>
  <!-- 动态注入的环境信息:时间、定位、间隔事件摘要等 -->
</background_environment>
```

> **范围**:本节只定义注入**语法**——即环境信息以什么标签结构承载、如何在 `AiMessage` 内呈现。
>
> **不在本节讨论**:注入**时机**(哪一轮注入、是否进入历史 turn、prompt cache 影响)——由专门的"环境信息注入策略"文档讨论。

### 1.3 两层视角中的可见性

| 视角 | 是否可见 |
|---|---|
| `MsgObject`(Message Center 层) | **不包含**该标签——这是 dispatch 到 Session 之后才在 `compose_turn_message` 注入的 |
| `AiMessage`(Session History 层) | **包含**该标签,LLM 视角能看到 |

这是 §0 两层数据源原则的自然结果,无需额外存储改造。

### 1.4 开发任务

| 任务 | 文件 | 强度 |
|---|---|---|
| 在 `compose_turn_message` 中,把 environment preamble 包裹进 `<background_environment>...</background_environment>` 标签 | `agent_session.rs` | 软协议 |
| WebUI 在以 Session History 视角渲染时,识别并特殊渲染该标签(折叠 / 灰色弱化展示) | WebUI 代码 | 软协议 |

---

## 2. 出站消息:Attachment 标签处理

### 2.1 现状

`ai_message_to_msg_object_with_base` 已经实现核心转换逻辑:

- `AiContent::Image` / `Document` 的 `ResourceRef::NamedObject` → `MsgContent.refs::DataObj`(✅ 标准转换)
- `ResourceRef::Url` / `Base64` → 降级为文本里的 `<attachment ... />` marker(⚠️ 不可无损)
- LLM 文本中显式出现的 `<attachment obj_id="..." />` → 转换为 `MsgContent.refs`(✅ 已实现)

### 2.2 目标协议

#### 2.2.1 转换后原始 `<attachment>` 标签的去留

**决议**:

- **默认行为:删除**——出站 `MsgObject.content.content` 中移除已被转换的 `<attachment>` marker,只保留结构化 refs。
- **Session 级可配置**:增加配置项 `preserve_attachment_tag_in_egress: bool`,允许特定 Session 保留原始 marker(调试 / 特殊场景)。
- **Session History 视角始终保留** AI 推理原文(含 marker),不受此配置影响。

#### 2.2.2 路径合法性检查(**安全强协议**)

`<attachment>` 的内容是 LLM 输出文本,**必须视为不可信输入**。出站转换层必须做以下校验:

| 引用形式 | 校验规则 |
|---|---|
| **Object ID**(`obj_id`) | 必须是当前 Agent 自身产生或在其访问范围内的对象。基于内容哈希天然不可伪造,但仍需校验 ACL |
| **本地路径** | 必须在该 Agent 的 **workspace 子目录**内,且不允许 `..` 路径穿越;符号链接需 resolve 后再次校验 |

校验失败时**拒绝转换**,在 Session History 中标记错误,并在出站消息中以普通文本说明(避免静默丢失)。

#### 2.2.3 长期方向:逐步弃用 path 模式

- Object ID 基于内容哈希,内容变更自动失效。
- path 模式在跨 host 场景下(Agent 在 host A、Tunnel 接收方在 host B)语义本身就不成立。
- 出站附件优先使用 `AiContent::Image` / `Document` + `ResourceRef::NamedObject`,文本 marker 作为兼容路径。

### 2.3 当前缺陷:`BudgetExhausted + partial output` 路径

现在 `BudgetExhausted` 走 `post_outbound_text`,**丢失了所有结构化 block**(包括 partial 输出里可能的图片、附件)。这违反了 §0 的两层视角原则——partial 输出也应该走完整的 `AiMessage → MsgObject` 转换。

**修复**:`BudgetExhausted` 分支应改为构造 `AiMessage`(role=Assistant)再走 `post_outbound_message`,而不是 `post_outbound_text`。

### 2.4 被丢弃的 block 类型

当前 `ai_message_to_msg_object_with_base` 会丢弃 `ToolUse`、`ToolResult`、`Thinking`、非 `buckyos.msg.machine` 的 `ProviderState`。这是**合理设计**——这些 block 是 Session History 内部状态,不应出 MsgObject 层。**不需修改**,但要在文档里明确这是 by-design。

### 2.5 开发任务

| 任务 | 文件 | 强度 |
|---|---|---|
| `<attachment>` 标签转换后默认删除,加 Session 级配置 | `msg_parser.rs` + Session config | 软协议 |
| Object ID 的 ACL 校验 | `msg_parser.rs` | **安全强协议** |
| 本地路径白名单校验(workspace 内、拒绝 `..`、resolve 符号链接) | `msg_parser.rs` | **安全强协议** |
| 校验失败时的标记与降级处理 | `msg_parser.rs` + `agent_session.rs` | 软协议 |
| `BudgetExhausted` 分支改走完整 `AiMessage` 转换 | `agent_session.rs` | 软协议(修复) |

---

## 3. 命令消息(Command Message)

### 3.1 现状

`msg_parser.rs` 中**已存在** `parse_msg_object` 函数包含 slash command 分支,但:

- `msg_center_pump.rs` 当前直接使用 `msg_object_to_ai_message`,**绕过了** slash command 解析。
- 因此 `/xxx` 形式的文本当前**全部进入 LLM 推理**,作为普通 user message。

### 3.2 目标协议

#### 3.2.1 严格白名单匹配

消息文本以 `/` 开头**且后续紧跟已注册命令名**的消息,识别为控制命令,不进入 LLM 推理。

**匹配规则**:`^/(<command_name>)(\s+<args>)?$`,其中 `<command_name>` 必须命中已注册命令表;否则当作普通文本进入推理。

这条规则消除了"伪命令误识别"风险——`/etc/nginx/conf 帮我看下` 这种用户文本天然安全,不会触发 `/clear`。

#### 3.2.2 命令表(初始集合)

| 命令 | 语义 |
|---|---|
| `/clear` | 清除当前 Session 的全部历史记录 |
| `/list` | 列出当前 Agent 上所有 Session |
| `/switch <session_id>` | 切换当前 Tunnel 绑定到指定 Session |
| `/help` | 列出当前 Agent 支持的所有命令 |

#### 3.2.3 Tunnel 原生命令通道(推荐)

Telegram / Discord / Slack 等有原生 slash command UI 的 Tunnel,应当**优先走 Tunnel 原生通道**(BotFather 注册命令、Application Commands 等),而不是依赖消息体里的文本前缀。

两种入口在 Agent Runtime 入口的**命令处理器层统一收敛**——无论从哪个入口进来,执行的都是同一组 handler。

#### 3.2.4 转义

由于白名单匹配,**绝大多数情况下用户无需特殊处理**。只有当用户文本正好是某个命令名时,可以:
- 在 `/` 前加任意字符(命令解析器只在消息最开头匹配)
- 使用引号 / 反引号包裹(解析器不识别包裹形式)

### 3.3 命令处理点的协议归属

命令消息不变成 LLM 推理输入,但仍是协议层定义的消息。**实现上**:

- 命令解析发生在 `msg_center_pump.rs` → `Inbound` 构造之间,新增一种 `Inbound::Command { name, args, ... }` 变体。
- `agent.rs` 收到 `Inbound::Command` 不走 session 推理队列,直接路由到命令处理器。
- 命令执行结果走 `post_outbound_message` 回传(系统消息形态)。

### 3.4 开发任务

| 任务 | 文件 | 强度 |
|---|---|---|
| 恢复 `parse_msg_object` 调用路径,但改为白名单匹配 | `msg_center_pump.rs` + `msg_parser.rs` | **强协议** |
| 命令注册表 + dispatcher | `agent.rs` 新增模块 | **强协议** |
| `Inbound::Command` 变体 + 路由 | `agent.rs` / `session_model.rs` | **强协议** |
| `/clear` / `/list` / `/switch` / `/help` 初始命令实现 | `agent.rs` 新增模块 | 强协议 |
| Telegram 原生 BotFather 命令注册 + 适配层 | `tg_tunnel.rs` | 协议建议 |

---

## 4. Session 类型与群聊

### 4.1 现状

- `MsgObjKind` 已有 `Chat` 和 `GroupMsg` 两值。
- `msg_center.rs` 已根据 `to` 字段路由到 `Inbox` / `GroupInbox` / `RequestBox`。
- `msg_center_pump.rs` 从 `GroupInbox` take_next,但**之后的处理路径与单聊完全一致**——同样构造 `Inbound::Msg`、同样进入 session pending queue。

**关键 gap**:协议设计中关于群聊的所有规则(@ 触发推理、串行 + 排队、`from_did`/`relation` 标注、`from_user_did` 注入 Tool Call)**目前都没有实现**。

### 4.2 目标协议:Session 类型枚举

```rust
// 概念性定义,具体置于 session_model.rs 或共享类型库
pub enum SessionKind {
    OneToOne,
    Group,
    Channel,         // 预留:单向广播频道
    Custom(String),  // 后续扩展位
}
```

**约束**:

- Session 类型是 Session 本身的属性,**创建时确定,不允许中途变更**。
- Agent 装配到 Session 时,基于 `agent.allowed_session_kinds`(详见 §4.6)做准入判断。
- 协议字段进入 Session 创建协议(`AgentSession::ensure_for_*`)。

### 4.3 群聊触发推理:仅 @ 触发

| Session 类型 | 触发推理条件 |
|---|---|
| `OneToOne` | 每条 `Inbound::Msg` 都触发推理(当前行为) |
| `Group` | 仅当消息中**显式 @ 当前 Agent** 时才 dispatch 进 session 推理队列。其他消息只被 Message Center 收集存档 |

**实现位置**:`agent.rs` dispatcher 在路由 `Inbound::Msg` 时根据 Session kind 判断;未 @ 的群消息**不**调用 `enqueue_pending`,但仍 ack msg-center record(避免重新投递)。

> **@ 识别**:基于 Telegram 等 Tunnel 在 `meta["telegram"]` 中携带的 `mentions` 字段,或对 `MsgContent.content` 做 `@<bot_username>` 文本匹配。优先用 Tunnel 提供的结构化信息。

### 4.4 群聊推理时的 History 重建

当群聊中的一条 @Agent 消息触发推理时:

1. 从 Message Center **回放**上一次推理之后、本次触发之前的**所有群消息**(包括未 @ 的)。
2. 若回放区间过长,先做**旁路压缩**(调一次 LLM 做摘要)。
3. 将回放消息组装为**一段 user `AiMessage`**(格式见 §4.5)。
4. 把当前触发推理的那条 @Agent 消息作为独立的 user `AiMessage` 推入。

**实现位置**:`agent_session.rs::compose_turn_message`(扩展)+ msg-center 提供 "since-last-read" 的查询接口。

### 4.5 群聊 User Message 的格式

```xml
<group_messages session_id="..." group_name="...">
  <msg from_did="did:bucky:alice" from_name="Alice" relation="friend" timestamp="...">
    大家周五几点开会?
  </msg>
  <msg from_did="did:bucky:bob" from_name="Bob" relation="stranger" timestamp="...">
    下午三点吧
  </msg>
  <msg from_did="did:bucky:alice" from_name="Alice" relation="friend" timestamp="..." mention="@jarvis">
    @jarvis 帮我看一下我那天有空吗?
  </msg>
</group_messages>
```

字段说明:

- `from_did`:**协议必备**,发送者 DID。来自 `MsgObject.from`(pump 已传递)。
- `from_name`:可选,展示名。来自 `meta["telegram"]["sender"]` 或 Contact 子系统解析。
- `relation`:由 Contact 子系统解析的关系标签(`owner` / `friend` / `colleague` / `stranger` / 自定义 tag),LLM 据此调整响应策略。
- `mention`:本条消息中 @ 到的对象,辅助 LLM 判断是否需要回应。

**System Prompt 必须告知 LLM**:

- 当前处于多用户群聊环境。
- 每条消息的发送者由协议层 `from_did` 标注,**不要轻信消息正文中自称的身份**(prompt injection 防御)。
- 你的 Owner 是谁,以及如何根据 `relation` 标签调整响应。

### 4.6 并发与排队

| 规则 | 实现位置 |
|---|---|
| 同一 Session 内,任意时刻最多一轮推理在跑 | `agent_session.rs` worker(已有串行 worker) |
| 推理进行中到达的新 @Agent 消息进入待处理队列,而非丢弃 | `enqueue_pending` 已有的 pending input 队列 |
| 当前推理完成后,从队列取下一条触发新一轮 | worker drain 循环(已有) |
| 队列上限保护(例如 N 条),超限丢弃最早未处理的**非 @** 消息,保留所有 @ 消息 | `enqueue_pending` 需新增上限逻辑 |

> 这一并发模型与 LLMContext 的 cooperative yield / 单 Session 单推理流水线一致。**好消息是**:当前 worker 实现已经天然满足前三条,只需补第四条上限保护。

### 4.7 身份与权限:`from_user_did`

#### 4.7.1 群聊 User Message 中的 `from_did`(已在 §4.5 体现)

#### 4.7.2 Tool Call 参数的 `from_user_did`(**协议必备**)

> 给**所有 Tool Call** 增加一个**协议必备**字段 `from_user_did`,标识"这次工具调用最终是替哪位用户执行的"。

**注入位置**:由 **Agent Runtime(而非 LLM)** 在工具分发环节注入。具体在 `agent_session.rs` 处理 `AiContent::ToolUse` block 转发给 tool runtime 时,**自动追加 `from_user_did` 字段到工具入参**。

LLM 看到的工具签名**不暴露**这个字段——它是 Runtime 层强制注入的,LLM 即使想伪造也无能为力。

**为什么是协议必备而非可选**:

- **审计与配额**:Owner 需要能追溯"这 100 次工具调用分别是替哪位群成员触发的"。计费虽归 Owner(§4.7.4),但明细必须保留。
- **Confused Deputy 防御**:LLM 软控制不可靠,工具层必须能基于真实发起人身份做最终裁定。
- **一对一场景兼容**:一对一时 `from_user_did = owner_did`,工具层逻辑完全统一,不引入特殊路径。

#### 4.7.3 软硬双层防御

| 层 | 控制点 | 失效后果 |
|---|---|---|
| **软层**(System Prompt + `relation` 标注) | LLM 自觉拒绝不合规请求 | LLM 可能被 prompt injection 绕过 |
| **硬层**(`from_user_did` + 工具实现的权限检查) | 工具拒绝执行 | 即使 LLM 决定执行,工具也拒绝 |

两层不可互相替代。软层优化体验(拒绝时给出友好回复),硬层是安全底线。

#### 4.7.4 计费

- 群聊推理与工具调用产生的费用,**统一归属 Agent Owner**——无论触发的是 Owner 还是其朋友。
- Billing 子系统**必须保留 `from_user_did` 维度的明细**。

#### 4.7.5 Contact 子系统接口需求

协议层需要 Contact 子系统提供:

| 接口 | 用途 |
|---|---|
| `resolve_relation(did) -> RelationTag` | 给定 DID,返回相对 Owner 的关系标签,供 §4.5 消息格式标注使用 |
| `can_trigger_inference(did, agent_did) -> bool` | 判断该用户在该 Agent 的群聊 Session 中是否有权触发推理 |
| `redaction_policy(did, relation) -> RedactionSet` | (可选)返回该用户应被脱敏的隐私维度集合,供 System Prompt 使用 |

接口具体形态在 Contact 子系统设计文档中定义,本协议只约定消费方。

### 4.8 Agent 角色隔离的协议支持

§4.7 是软+硬双层防御,但真正的硬权限隔离需要**让不同能力面的 Agent 加入不同 Session**(例如全功能 Jarvis 不加入群组,只让能力面受限的"社交代理 Agent"加入群组)。**具体哪些 Agent 加入哪类 Session,是 Agent 装配 / 注册系统的设计问题,不在本协议范围**。

协议层只保证:

- `SessionKind` 是 Session 的属性。
- Agent 自身可声明 `allowed_session_kinds`(例如 `["OneToOne"]` / `["OneToOne", "Group"]`)。
- Session 装配阶段对此做检查;装配失败的 Agent 不会被路由到该 Session 的消息。

### 4.9 开发任务

| 任务 | 文件 | 强度 |
|---|---|---|
| `SessionKind` 枚举定义,集成到 `session_model.rs` 的 Session 元数据 | `session_model.rs` | **强协议** |
| `agent.rs` dispatcher 区分 OneToOne / Group,Group 仅 @ 触发推理 | `agent.rs` | **强协议** |
| @ 识别(优先 Tunnel 结构化,降级文本匹配) | `agent.rs` + `tg_tunnel.rs` | **强协议** |
| `<group_messages>` 格式注入到 `compose_turn_message` | `agent_session.rs` | **强协议** |
| msg-center 提供 "since-last-read" 群消息查询 | `msg_center.rs` | **强协议** |
| 旁路压缩(回放区间过长时) | `agent_session.rs` 新增辅助 | **强协议** |
| pending 队列上限保护 | `agent_session.rs::enqueue_pending` | 强协议 |
| 群聊 System Prompt 模板 | prompts 资源目录 | 强协议(提示词) |
| Tool Call 分发时注入 `from_user_did` | `agent_session.rs` Tool dispatch 点 | **强协议(安全)** |
| Contact 子系统接口接入 | `agent.rs` / `agent_session.rs` | **强协议** |
| Billing 子系统接入 `from_user_did` 明细 | Billing 子系统 | 强协议 |
| Agent 配置 `allowed_session_kinds` + 装配阶段校验 | Agent 装配模块 | **强协议** |

---

## 5. 路由与 Tunnel:与现有实现的契合

本节不引入新协议,仅说明协议改进与现有路由机制的契合点,避免后续开发踩坑。

### 5.1 入站路由保留(已存在,确认仍然成立)

- `IngressContext` 记录的 `tunnel_did` / `platform` / `chat_id` / `source_account` 在 record route 中保留——**群聊场景下这套机制无需改造**,因为群聊回复目标仍是 group ID,机制对称。
- `msg.thread.topic` 由 `buckyos_api::build_telegram_ui_session_id(bot_account_id, chat_id)` 生成,当前形态为 `"tg:<bot_account_id>:<chat_id>"`;在群聊场景下天然就是群聊话题,UI session 聚合行为正确。

### 5.2 出站偏好(已存在)

- `session.peer_tunnel_did` 在群聊场景下指向**群所在的 tunnel**——回复时回到同一群,符合预期。

### 5.3 命令响应的出站路径

- 命令处理器产生的系统响应也走 `post_outbound_message`,但 `AiMessage.role` 应该是 `Assistant`(或新增 `System` 语义),`meta["llm_role"]` 标记便于 UI 区分。
- 命令响应**不应**进入 LLM history(它不是推理结果),`agent_session.rs` 需要在 LLMContext accumulate 时跳过。

---

## 6. 当前端到端链路(协议改进后的目标形态)

```text
Telegram update
  -> tg_tunnel: MsgObject + IngressContext
  -> msg_center.handle_dispatch
  -> Inbox / GroupInbox / RequestBox MsgRecord
  -> OpenDAN msg_center_pump.take_next(with_object=true)
  -> [新增] 命令白名单匹配
        ↓ (命中)
        -> Inbound::Command { name, args, ... }
        -> agent.rs 命令 dispatcher
        -> 命令处理器 → post_outbound_message
        ↓ (未命中)
  -> llm_context::msg_object_to_ai_message
  -> Inbound::Msg { from_did, tunnel_did, ai_message, ... }
  -> [新增] agent.rs dispatcher 根据 SessionKind 判断:
        - OneToOne: 每条入队
        - Group: 仅 @ 入队,其他 ack 后丢弃
  -> PendingInput::Msg
  -> AgentSession::compose_turn_message
        + <background_environment>...</background_environment>
        + (Group) <group_messages>...</group_messages> 回放
        + 当前触发消息
  -> LLMContextRequest.input
  -> LLMContext.run() → AiMessage(role=Assistant)
  -> [Tool Call 分发时] 自动注入 from_user_did
  -> post_outbound_message
  -> ai_message_to_msg_object_with_base
        + [新增] <attachment> 路径合法性校验
        + [新增] 默认删除原始 <attachment> marker
  -> msg_center.post_send(preferred_tunnel)
  -> TunnelOutbox record
  -> tg_tunnel.send_record
  -> Telegram send
```

---

## 7. 开发路线图建议

按依赖关系排序。**推荐路线**:

### Phase 1:安全底线(必须先做)

1. **§2.2.2 路径合法性校验**:`<attachment>` 路径白名单 + Object ID ACL。这是当前最大的安全敞口。
2. **§4.7.2 Tool Call `from_user_did` 注入**:即使群聊功能未上,一对一场景下这个字段就应该开始一致地传递,降低后续工具迁移成本。

### Phase 2:命令消息(独立改造,影响面小)

3. **§3 命令白名单 + dispatcher**:恢复 `parse_msg_object` 调用,改为白名单。`/clear` 作为首个落地命令。
4. Telegram 原生命令注册作为后续优化。

### Phase 3:群聊支持(主要工作量)

5. **§4.2 SessionKind 枚举落地**,Session 创建时携带。
6. **§4.3 @ 触发推理 + dispatcher 分流**。
7. **§4.4 群消息回放 + 旁路压缩**。
8. **§4.5 群聊消息格式** + 群聊 System Prompt 模板。
9. **§4.7.5 Contact 子系统接口**(可与群聊并行开发)。
10. **§4.8 `allowed_session_kinds`** 装配检查。

### Phase 4:体验优化与债务清理

11. **§1.4 `<background_environment>` 标签化**。
12. **§2.2.1 出站 `<attachment>` marker 默认删除**(配置开关)。
13. **§2.3 `BudgetExhausted + partial output`** 走完整 `AiMessage` 转换。
14. **§4.6 pending 队列上限保护**。

---

## 8. 开放问题与外部依赖

### 8.1 依赖其他文档讨论

- **Background Environment 注入时机**:本协议只定义注入语法,具体时机由专门的"环境信息注入策略"文档讨论。
- **外交官 Agent 等具体角色的能力面设计**:不在本协议范围,需另开设计文档。本协议只保证 §4.8 的 Session 装配机制足以支撑后续角色划分。
- **Contact 子系统接口形态**:§4.7.5 列出了 Message 层对 Contact 子系统的能力需求,具体接口在 Contact 子系统设计文档中定义。

### 8.2 仍待讨论

- **群聊 System Prompt 模板**:如何在 System Prompt 中表达 relation 标签的语义、引导 LLM 在拒绝场景下给出合适回复——属于提示词工程,可在原型期通过迭代敲定。
- **`/clear` 等破坏性命令的二次确认机制**:是否需要在执行前发一条"确认?"消息——取决于产品体验取舍。
- **Channel 类 Session 的具体规则**:本协议预留了 `Channel` 枚举位,但单向广播场景的协议细节(谁能发、谁能 @、Agent 是否被允许主动 push)需另文定义。
- **多 Tunnel 并发场景**:同一 Owner 同时在 Telegram 和 WebUI 在线,消息分发与回复路由策略——当前 `preferred_tunnel` 是单选,需要评估是否需要 fan-out。

### 8.3 已落定的决议

- 命令前缀:`/`,严格白名单匹配。
- Attachment 转换默认删除原始标签,Session 级可配置保留。
- `from_user_did` 是 Tool Call 协议**必备**字段。
- 群聊计费归属 Owner,但 Billing 保留 `from_user_did` 维度明细。
- 群聊推理串行执行,新触发进入待处理队列。
- Session 类型创建时确定,不可中途变更。

---

## 附录 A:术语表

| 术语 | 含义 |
|---|---|
| **MsgObject** | msg-center 的标准消息对象(envelope + content + refs + machine + meta) |
| **MsgRecord** | msg-center 中存储的消息记录,带状态(Unread / Reading / Read / Sent / Wait) |
| **AiMessage** | LLMContext 使用的 provider-neutral 消息模型,含有序 `AiContent` block |
| **AiContent** | AiMessage 的内容 block:Text / Image / Document / ToolUse / ToolResult / Thinking / ProviderState |
| **Inbox / GroupInbox / RequestBox** | msg-center 按目标 DID 维护的消息容器,分别对应私聊 / 群聊 / 陌生人请求 |
| **TunnelOutbox** | 待 Tunnel 发送的消息记录容器 |
| **IngressContext** | 入站消息携带的路由上下文(tunnel DID / platform / chat ID 等) |
| **Session** | 一次连续的 Agent 对话会话,有唯一 ID |
| **SessionKind** | Session 类型(OneToOne / Group / Channel / ...),创建时确定 |
| **LLMotor / LLMContext** | LLM 推理引擎模块,负责 accumulate 推理历史、管理 turn 结构 |
| **PendingInput** | Session worker 待处理输入队列项(Msg / Event) |
| **DID** | Decentralized Identifier,BuckyOS 的去中心化身份标识 |
| **Owner** | 一个 Agent 的归属用户,持有该 Agent 的全部能力授权与计费责任 |
| **relation** | 由 Contact 子系统解析的"用户相对 Owner 的关系标签" |

---

## 附录 B:协议改动文件影响矩阵

| 改动 | `msg_parser.rs` | `msg_center_pump.rs` | `agent.rs` | `agent_session.rs` | `session_model.rs` | `msg_center.rs` | `tg_tunnel.rs` | 新模块 |
|---|---|---|---|---|---|---|---|---|
| §1 background_environment 标签 | | | | ✏️ | | | | |
| §2.2.1 attachment 默认删除 | ✏️ | | | | ✏️ | | | |
| §2.2.2 路径合法性校验 | ✏️ | | | | | | | |
| §2.3 BudgetExhausted 修复 | | | | ✏️ | | | | |
| §3 命令白名单 + dispatcher | ✏️ | ✏️ | ✏️ | | ✏️ | | (+) | (命令处理器) |
| §4.2 SessionKind | | | | | ✏️ | | | |
| §4.3 群聊 @ 触发 | | | ✏️ | | | | (+) | |
| §4.4 群消息回放 + 压缩 | | | | ✏️ | | ✏️ | | |
| §4.5 group_messages 格式 | | | | ✏️ | | | | |
| §4.6 队列上限保护 | | | | ✏️ | | | | |
| §4.7.2 from_user_did 注入 | | | | ✏️ | | | | |
| §4.7.5 Contact 接入 | | | ✏️ | ✏️ | | | | (Contact 客户端) |
| §4.8 allowed_session_kinds | | | ✏️ | | ✏️ | | | (Agent 装配) |

✏️ = 修改;(+) = 小幅适配;空 = 不影响。
