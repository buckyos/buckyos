# Message Tunnel 定义

## 1. 定位

`Message Tunnel` 是外部消息/会话系统与 BuckyOS `MessageCenter` 之间的双向适配层。它把平台原生消息、会话事件和成员状态标准化为 BuckyOS 可处理的消息事件流，也把 `MessageCenter` 中待投递的标准消息转换为平台可执行的发送、会话操作或状态更新。

典型 Tunnel 包括：

- `Telegram Tunnel`
- `Lark Tunnel`
- `Email Tunnel`
- `MessageHub Tunnel`

完整的 `Message Tunnel` 定义的是最大能力集。具体平台 Tunnel 必须根据平台协议、机器人限制、账号形态和授权状态裁剪能力。

## 2. 职责边界

`Message Tunnel` 负责平台适配和可靠同步，不负责 Agent 的业务决策。

Tunnel 应负责：

- 连接外部平台，并维护平台账号、机器人账号或邮箱账号的运行状态。
- 接收外部平台的消息、会话事件和成员状态变化。
- 将平台原生对象转换为标准 `MsgObject`、`IngressContext`、`RouteInfo` 或后续扩展的会话事件对象。
- 将入站事件通过 `MessageCenter.dispatch()` 写入系统。
- 从 `TunnelOutbox` 获取待发送记录，并把标准消息转换为平台原生发送动作。
- 将发送结果通过 `MessageCenter.report_delivery()` 或等价接口回写。
- 维护平台消息 ID、会话 ID、账号 ID、附件 ID 与 BuckyOS DID / message id / record id 的映射。
- 支持重试、去重、补拉、状态同步和故障可观测。

Tunnel 不应负责：

- 决定 Agent 是否应该回复、沉默、调用工具或改变计划。
- 判断 Agent 是否有权跨会话发消息、发 email、加入群或读取外部内容。
- 存储 Agent 的长期记忆、任务状态或工具调用结果。
- 把某个平台的特殊交互模型泄漏为 Agent 必须理解的业务逻辑。

这些职责分别属于 `Agent Runtime`、权限/策略系统、工具系统、审计系统和 `MessageCenter`。

## 3. 核心链路

### 3.1 入站链路

外部平台到 Agent 的标准流程：

1. 外部平台产生消息或会话事件。
2. Tunnel 获取事件，解析平台身份、会话、消息、附件和平台元数据。
3. Tunnel 将平台身份映射到 BuckyOS DID 或临时联系人。
4. Tunnel 构造标准消息对象和 `IngressContext`。
5. Tunnel 调用 `MessageCenter.dispatch()`。
6. `MessageCenter` 根据联系人、权限、群订阅和会话规则写入对应 inbox、group inbox 或 request box。
7. Agent 通过自己的消息视图读取并处理。

### 3.2 出站链路

Agent 或系统到外部平台的标准流程：

1. Agent、用户或系统服务生成回复或通知。
2. 调用方通过 `MessageCenter.post_send()` 提交标准消息。
3. `MessageCenter` 根据接收方 DID、联系人绑定、优先通道和 `SendContext` 生成投递计划。
4. `MessageCenter` 写入发送方 outbox 和对应 Tunnel 的 `TunnelOutbox`。
5. Tunnel 从自己的 `TunnelOutbox` 获取 `WAIT` 记录，进入 `SENDING`。
6. Tunnel 将标准消息转换为平台原生发送请求。
7. Tunnel 将平台返回的外部消息 ID、成功时间、失败原因和重试建议回写到投递记录。
8. `MessageCenter` 发布消息视图变更，供 UI、Agent 或审计系统观察。

### 3.3 Agent 参与边界

Agent 只消费 `MessageCenter` 暴露的标准消息视图。除非 Agent 主动请求平台特定能力或诊断信息，否则它不应依赖消息来自 Telegram、Lark、Email 还是 MessageHub。

Agent 可以产生跨通道动作，例如“给某人发 email”或“向另一个会话发送消息”，但这些动作必须先经过权限/策略系统，再由 `MessageCenter` 选择对应 Tunnel 执行。

## 4. 对象模型

### 4.1 TunnelInstance

`TunnelInstance` 表示一个正在被系统管理的通道实例。

必要字段：

- `tunnel_did`：Tunnel 的系统身份。
- `platform`：平台类型，例如 `telegram`、`lark`、`email`、`messagehub`。
- `name`：可读名称。
- `supports_ingress`：是否支持入站。
- `supports_egress`：是否支持出站。
- `state`：运行状态，至少包括 `Registered`、`Starting`、`Running`、`Stopping`、`Stopped`、`Faulted`。
- `last_error`：最近一次运行错误。

当前代码中 `MsgTunnel` trait 已包含 `tunnel_did()`、`name()`、`platform()`、`supports_ingress()`、`supports_egress()`、`start()`、`stop()` 和 `send_record()`，可作为实现入口。

### 4.2 Binding

`Binding` 表示 BuckyOS 实体与外部平台账号或会话地址之间的绑定。

必要字段：

- `owner_did`：绑定所属实体，可以是用户、Agent、联系人或 group。
- `platform`：平台类型。
- `account_id`：平台账号 ID。
- `display_id`：用户可读地址，例如 username、邮箱地址、手机号或群名。
- `tunnel_id`：负责该绑定的 Tunnel。
- `context`：可选的平台会话、群、邮箱文件夹或租户上下文。
- `meta`：平台特定扩展字段。

Binding 是出站选路的基础，也是入站身份归并的依据。

### 4.3 Conversation

`Conversation` 表示一次可持续互动的会话上下文。

完整 Tunnel 应能表达：

- 1v1 会话。
- 多人会话。
- 群聊。
- 群聊中的 topic、thread、临时子会话或子群。
- 邮件 thread。
- 仅机器人参与的会话。

建议将群聊里的子群、议题线和邮件回复链统一抽象为 `Thread` 或 `SubConversation`，避免把平台差异直接暴露给 Agent。

### 4.4 Participant

`Participant` 表示会话参与者，可以是：

- 自然人。
- Agent。
- 系统账号。
- 外部联系人。
- 外部平台机器人。
- group 或组织实体。

参与者在系统内应尽量映射为 DID。无法确认身份时，可以先创建临时联系人或影子联系人，用于保留来源、显示名称和平台账号映射。

参与者身份本身不应直接决定消息是否可以进入 Agent 的会话上下文。尤其在群聊中，Agent 加入某个群通常意味着 owner 已允许 Agent 观察这个会话；群成员发言应先作为该会话的上下文事件进入系统，而不是因为成员尚未逐个验证就被统一降级为低信任消息。

信任判断应主要作用在“Agent 能对这条消息做什么”上，而不是简单作用在“这条消息能不能被 Agent 看到”上。未验证参与者的消息可以被 Agent 阅读、总结和普通回复；但是否能触发高权限工具、跨会话投递、长期配置变更或代表 owner 的外部动作，应交给权限/策略系统按动作单独判断。

### 4.5 Message

`Message` 是人类或 Agent 可阅读、处理、引用和投递的内容对象。

完整 Tunnel 应支持：

- 纯文本。
- 富文本。
- 表情、reaction、贴纸等符号消息。
- 图片、音频、视频、文件等附件引用。
- 引用、回复、转发。
- 编辑和撤回事件。
- 平台命令、按钮点击、表单回复等交互消息。

平台原始内容可以保留在 `extra` 或平台元数据中，但 Agent 的普通聊天逻辑应依赖标准化内容。

### 4.6 ConversationEvent

`ConversationEvent` 是会话状态变化，不应简单混同为聊天消息。

完整 Tunnel 应能表达：

- 会话创建、归档、关闭。
- 成员加入、退出、被邀请、被移除。
- 成员上线、下线。
- 禁言、解除禁言、屏蔽、解除屏蔽。
- 授权、撤销授权。
- 正在输入。
- 已发送、已送达、已读、阅读中。
- 消息编辑、删除、撤回。
- 会话名称、头像、公告、topic 变更。

当前实现可以先将部分事件降级为系统消息或 `extra` 元数据，但完整模型中应保留事件类型，避免后续无法区分“内容消息”和“状态变化”。

### 4.7 Mention 和特殊实体

`@`、频道引用、话题标签、命令、角色提醒和全员提醒不应只按字符处理。Tunnel 应把它们解析为结构化实体。

建议抽象：

```text
Mention {
  target_type: user | agent | group | role | all | platform_native
  target_did: optional DID
  platform_target_id: optional string
  raw_text: string
  range: optional text range
  extra: platform metadata
}
```

不同平台可以使用不同触发字符或完全不使用字符。标准模型应表达“提及了谁/什么”，而不是绑定在 `@` 字符上。

## 5. 投递语义

`Message Tunnel` 的投递语义定义为最终一致。

Tunnel 不承诺端到端强一致投递，不承诺 exactly-once，不承诺全局有序。它承诺在网络抖动、平台限流、进程重启、`MessageCenter` 短暂不可用等可恢复故障后，通过重试、去重、补拉和状态同步，使 BuckyOS 与外部会话系统在可接受时间内达到最终一致。

### 5.1 基本语义

- 入站和出站都采用至少一次尝试投递。
- 重试可能导致重复事件，系统必须支持幂等。
- 同一 conversation/thread 内尽量保持平台顺序。
- 跨 conversation/thread 不保证顺序。
- 补拉历史、webhook 延迟、平台重发和断线恢复可能导致乱序。
- Agent 和 UI 必须能容忍重复、迟到和乱序事件。

### 5.2 幂等键

入站幂等键应优先由以下字段构造：

- `tunnel_did`
- `platform`
- `platform_account_id`
- `platform_conversation_id`
- `platform_message_id`
- `platform_event_id`
- `event_type`

出站幂等键应优先由以下字段构造：

- `record_id`
- `msg_id`
- `tunnel_did`
- `target_did`
- `route.chat_id`
- `send_intent_id` 或调用方提供的 idempotency key

`MessageCenter` 层应保证重复 `dispatch()` 或 `post_send()` 不产生重复的用户可见消息或重复投递任务。Tunnel 层应保证重复扫描 `TunnelOutbox` 时不会把同一投递记录当作新的业务意图。

### 5.3 状态收敛

Tunnel 应把投递状态收敛到明确状态：

- `WAIT`：等待发送。
- `SENDING`：正在发送或已被 Tunnel 获取。
- `SENT`：平台已接受或已成功投递。
- `FAILED`：可恢复失败，等待重试或人工处理。
- `DEAD`：不可恢复失败，不再自动重试。

可恢复失败包括网络超时、平台临时错误、限流、进程重启和短暂认证刷新失败。

不可恢复失败包括无权限、机器人被移除、会话不存在、目标账号不存在、附件永久失效、平台明确拒绝和配置缺失。

### 5.4 顺序性

Tunnel 不提供全局顺序。推荐顺序范围是：

- 每个 `platform_conversation_id` 或 `thread_id` 内维护平台 offset。
- 入站补拉时按平台原始顺序提交，但允许迟到事件覆盖旧状态。
- 出站发送时同一目标会话可以串行化，以避免用户可见顺序混乱。
- 跨会话和跨 Tunnel 不做顺序承诺。

如果平台本身不保证顺序，Tunnel 只能保留平台提供的时间戳、序号和接收时间，不能伪造强顺序。

## 6. 可靠性要求

完整 Tunnel 应具备以下恢复能力：

- 断线重连。
- webhook 与 polling 的 offset 持久化。
- 按 conversation/thread 补拉遗漏消息。
- 平台事件去重。
- 出站投递重试。
- 限流退避。
- 死信记录。
- 附件下载失败后的重试或终态失败。
- 进程重启后恢复未完成的 `SENDING` 记录。

`SENDING` 记录在进程崩溃后可能已经发送成功，也可能尚未发送。恢复时应优先通过外部消息 ID、平台查询或业务幂等键确认状态；无法确认时允许重试，但必须记录不确定性。

## 7. 附件与富内容

Tunnel 不应把大附件直接塞入普通消息内容。推荐模型：

- 入站附件转换为可访问的对象引用。
- 记录平台原始附件 ID、文件名、mime type、大小、过期时间和下载权限。
- 必要时由 Tunnel 或附件服务异步拉取并落入 BuckyOS 对象存储。
- 出站附件由 `MessageCenter` 或调用方提供对象引用，Tunnel 负责转换为平台上传动作。
- 附件失效、上传失败或权限不足必须形成可见投递结果。

富文本应尽量转换为标准结构，同时保留平台原文或平台结构，供需要高保真转发的 Tunnel 使用。

## 8. 权限与安全

Tunnel 是能力执行者，不是最终授权者。

权限应分层处理，不能只按消息作者是否可信来决定入站消息是否进入 Agent。完整系统至少应区分三类授权：

- 会话观察授权：Agent 是否可以读取某个会话、thread 或群的消息流。
- 会话发言授权：Agent 是否可以在该会话中发言、回复、mention 或执行平台会话动作。
- 高权限动作授权：Agent 是否可以调用工具、跨会话投递、发送 email、访问私有资料、修改配置或代表 owner 执行外部动作。

群聊场景下，owner 授权 Agent 加入或观察某个群后，群内普通消息默认应进入该群会话上下文。否则系统会把大多数群聊都变成低信任噪音，Agent 无法形成有效上下文。未验证群成员的发言可以影响普通对话理解，但不能自动获得更高的动作授权。

完整系统中，以下行为必须经过权限/策略检查：

- Agent 读取某个会话的消息。
- Agent 加入群、退出群或邀请成员。
- Agent 主动向某人或某个外部会话发送消息。
- Agent 使用 Email、Lark、Telegram 等通道代表用户发言。
- Agent 访问附件、历史消息或联系人资料。
- Agent 跨平台转发消息。

Tunnel 应执行平台侧必要校验，并把平台拒绝结果回写；但 BuckyOS 内部是否允许执行该动作，应由策略系统在进入 `post_send()` 或会话操作前决定。

`RequestBox` 更适合处理会话建立、陌生人首次联系、邀请、授权请求或被策略隔离的消息，而不应作为群内未验证成员普通发言的默认归宿。对已经授权观察的群聊，普通发言应进入群会话视图；只有来源会话未授权、平台身份异常、命中屏蔽策略、包含危险操作请求或要求提升权限时，才进入 `RequestBox` 或被拒收。

因此，安全模型应避免把“低信任参与者”扩大成“低信任消息洪泛”。更合理的默认规则是：读上下文按会话授权，做动作按动作授权，敏感动作按 owner 确认或显式策略授权。

## 9. 可观测性

Agent 和 Tunnel 的行为必须可追踪，否则多 Agent 会话无法被理解和排查。

Tunnel 至少应记录：

- 入站事件接收时间、平台事件 ID、offset 和解析结果。
- 入站事件对应的 `msg_id`、`record_id` 和目标 DID。
- 出站投递记录的获取、发送请求、平台响应和最终状态。
- 重试次数、下一次重试时间、限流原因。
- 平台认证、连接、webhook、polling 和附件处理错误。
- 被策略拒绝、联系人屏蔽、无绑定、无权限等业务失败。

Agent 审计不属于 Tunnel 本身，但 Tunnel 日志应能与 Agent 决策日志通过 `msg_id`、`record_id`、`conversation_id` 或 trace id 关联。

## 10. 平台能力裁剪

完整能力集不要求每个平台都实现。

### 10.1 Telegram

Telegram Tunnel 通常支持聊天消息、群聊、附件、reply、mention、reaction 的部分能力。机器人账号可能受平台限制，例如无法读取所有历史、无法主动私聊未开始会话的用户、无法看到某些群成员状态。

Telegram Tunnel 应在能力矩阵中明确：

- 使用 Bot API 还是用户会话 API。
- 是否支持入站群消息。
- 是否支持主动出站私聊。
- 是否支持附件下载和上传。
- 是否支持编辑、撤回、已读、typing、reaction。

### 10.2 Lark

Lark Tunnel 通常需要处理租户、应用机器人、用户授权、群、thread、卡片消息和企业权限。

Lark Tunnel 应重点明确：

- 机器人能读取哪些群消息。
- 用户授权与机器人授权的边界。
- 卡片消息如何降级或结构化。
- 企业管理策略导致的发送失败如何回写。

### 10.3 Email

Email Tunnel 更接近异步 thread 投递，不是实时 IM。

Email Tunnel 应重点明确：

- `Conversation` 映射到邮件 thread。
- `Message` 映射到 MIME message。
- 收件人、抄送、密送与 DID 的关系。
- 附件上传下载策略。
- 已读、撤回、typing、成员在线等能力通常不支持。
- 投递状态只能表达本系统发送成功、SMTP 接受、退信或后续同步结果，不能承诺对方已读。

### 10.4 MessageHub

MessageHub Tunnel 是 BuckyOS 原生消息通道，理论上应最接近完整能力集。

它应优先支持：

- DID 原生身份。
- BuckyOS 原生 group。
- 原生 `MsgObject`。
- 附件对象引用。
- read receipt。
- group/subgroup/thread。
- Agent 与人类用户的同构参与。

## 11. 配置要求

每个 Tunnel 至少需要以下配置：

- `enabled`：是否启用。
- `tunnel_did`：Tunnel DID。
- `platform`：平台类型。
- `ingress_enabled`：是否接收入站事件。
- `egress_enabled`：是否允许出站投递。
- `gateway_mode`：平台接入方式，例如 webhook、polling、bot api、smtp/imap。
- `credentials_ref`：凭据引用，不应直接把敏感 token 写入普通文档。
- `bindings`：平台账号与 DID 的绑定关系。
- `retry_policy`：重试次数、退避、过期时间。
- `rate_limit_policy`：平台限流策略。
- `capabilities`：当前实例声明的能力矩阵。

配置变更后，Tunnel 应支持重载或明确要求重启。凭据错误、绑定缺失、能力关闭时应进入可观测状态，而不是静默丢弃消息。

## 12. 能力矩阵

每个具体 Tunnel 都应声明能力矩阵，至少覆盖：

| 能力 | 说明 |
| --- | --- |
| ingress.message | 接收入站内容消息 |
| ingress.event | 接收入站会话事件 |
| egress.message | 发送出站内容消息 |
| egress.event | 执行出站会话操作 |
| conversation.1v1 | 1v1 会话 |
| conversation.group | 群聊 |
| conversation.thread | thread/topic/邮件回复链 |
| participant.human | 自然人参与者 |
| participant.agent | Agent 参与者 |
| content.text | 纯文本 |
| content.rich_text | 富文本 |
| content.attachment | 附件 |
| content.reaction | reaction/表情 |
| content.mention | 结构化 mention |
| state.read_receipt | 已读/阅读状态 |
| state.typing | 正在输入 |
| message.edit | 编辑消息 |
| message.delete | 删除或撤回消息 |
| delivery.retry | 可恢复重试 |
| delivery.report | 投递结果回写 |
| history.backfill | 历史补拉 |

能力矩阵是平台裁剪的依据，也是 UI 和 Agent Runtime 判断可用动作的依据。

## 13. 与现有实现的关系

当前仓库已有以下实现基础：

- `MessageCenter.dispatch()`：入站写入入口。
- `MessageCenter.post_send()`：出站排队入口。
- `BoxKind::TunnelOutbox`：Tunnel 出站队列。
- `MsgState::Wait/Sending/Sent/Failed/Dead`：出站投递状态。
- `IngressContext`：入站平台上下文。
- `SendContext`：出站投递偏好上下文。
- `RouteInfo`：投递路由信息。
- `DeliveryInfo` / `DeliveryReportResult`：投递结果与重试信息。
- `MsgTunnel` trait：Tunnel 实例的最小生命周期和出站发送接口。
- `MsgTunnelInstanceMgr`：Tunnel 注册、启动、停止和发送管理。

本文定义的是完整目标模型。实现时可以分阶段落地，但新增字段、协议或数据结构时必须同时检查：

- `src/kernel/buckyos-api/src/msg_center_client.rs`
- `src/frame/msg_center/`
- `doc/message_hub/`
- MessageHub / Users & Agents 前端类型
- `system-config` 中 Tunnel 配置与联系人 Binding

## 14. 最小可用实现

一个最小可用 Tunnel 至少应完成：

1. 注册 `tunnel_did`、`platform`、名称和 ingress/egress 能力。
2. 启动和停止生命周期可控。
3. 入站时把平台消息转换为标准 `MsgObject`，带上 `IngressContext`，调用 `dispatch()`。
4. 出站时从自己的 `TunnelOutbox` 获取 `WAIT` 记录，调用平台发送接口。
5. 成功时回写 `SENT`、`external_msg_id` 和 `delivered_at_ms`。
6. 可重试失败时回写 `FAILED`、错误原因和 `retry_after_ms`。
7. 不可恢复失败时回写 `DEAD` 和错误原因。
8. 使用平台消息 ID 和 record id 做幂等。
9. 记录足够日志用于排查入站、出站和投递失败。

## 15. 完整性检查清单

实现或评审一个 Tunnel 时，至少检查：

- 是否明确 ingress / egress 能力。
- 是否有 DID 与平台账号映射。
- 是否有 conversation/thread 映射。
- 是否有消息 ID / 事件 ID 幂等策略。
- 是否能处理重复、迟到和乱序事件。
- 是否有 offset、补拉或等价恢复机制。
- 是否区分内容消息和会话事件。
- 是否支持附件引用和附件失败终态。
- 是否能回写投递状态和外部消息 ID。
- 是否区分可重试失败和不可恢复失败。
- 是否能被 UI / Agent / 审计观察。
- 是否声明平台能力裁剪。
- 是否避免把平台特定语义泄漏给普通 Agent 逻辑。
- 是否经过权限/策略系统再执行跨会话、跨平台或代表用户的动作。

## 16. 关键原则

`Message Tunnel` 的核心原则是：

> Tunnel 表达并执行平台允许的会话能力；Agent 是否允许使用这些能力，由 BuckyOS 的身份、权限、策略和审计系统决定。

因此，一个完整 Tunnel 不等于一个全能 Agent。完整 Tunnel 是可靠、可恢复、可裁剪、可观察的平台能力适配器。
