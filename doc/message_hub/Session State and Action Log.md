# Session State 与 Action Log Message

- 版本：v0.1，2026-09-06
- 范围：数据层目标契约；本次只定义模型、权限、持久化与消息映射，不修改原型、Rust / TS 协议实现或数据库 DDL。
- 上游：[MessageHub PRD](../../product/message_hub/MessageHub_Web_UI_PRD.md)、[Message Center](<./Message Center.md>)。
- 关联：[UI DataModel](../../src/frame/desktop/src/app/messagehub/UI_DATAMODEL.md)、[Tunnel Design](<./Message Tunnel Design.md>)、[Self-Host-Group](<./Self-Host-Group.md>)。

## 1. 状态分类

Session 的持久业务状态分为整体状态和成员状态；已有临时运行态与个人展示偏好单独保留：

| 模型 | 内容举例 | 修改者 | 持久性 / Action Log |
|---|---|---|---|
| `SessionSharedState` | 会话共享标题、说明 | 有对应字段权限的成员 / 管理者 | 持久；实际变更产生日志 |
| `SessionMemberState` | 某成员在本会话的昵称 | 默认每位成员修改自己的可编辑字段；代改需授权 | 持久；共享可见字段实际变更产生日志 |
| `SessionRuntimeState` | typing、active、status_line、流式中间态 | 授权的成员 / Agent / 服务 | 有效期内可见，过期清除；不产生持久日志 |
| 个人展示偏好（`ui.*`） | 自定义显示标题、置顶、静音、草稿 | 当前用户 | 按原有偏好策略保存，不广播会话日志 |

“临时昵称”指仅对这个 Session 生效，不改实体全局名称，也不因刷新或离线而消失；
清除后回落到实体名称。成员可编辑自己的状态不等于可改其它成员，也不等于可修改角色、禁言或成员资格。
群成员资格 / 角色由 GroupMgr 或外部平台权威维护，不复制为可自由写入的 SessionMemberState 字段。

实体的全局名称、群成员等仍由各实体管理服务维护。修改 Session 标题不修改群名；
退出群与退出单个 Session 是不同动作。父子实体的状态与权限不因层级关系自动合并。

## 2. 身份与权威来源

`(ownerDid, sessionId)` 仍是读取本地消息历史的 `SessionRef`。共享状态需要稳定的逻辑会话引用：

```ts
type SessionStateRef = { authority_did: DID; session_key: string }
```

- `authority_did` 标识负责状态确认的实体 / 服务，`session_key` 是该权威下的稳定会话键。
  它不是展示标题，也不要求 Session 自身成为 DID 实体。
- 会话登记将本地 `SessionRef` 映射到 `SessionStateRef`；同一原生逻辑会话在不同 owner 的本地历史
  可以使用不同 Session ID，但引用相同权威状态。不得为每个观察者产生一份可独立修改的“共享标题”。
- 原生会话由创建 / 托管方明确状态权威；UI 和成员不能因为持有本地记录就成为权威。
  跨 Zone 写入向该权威请求，本地只保存确认后的副本。
- 外部会话的平台是业务权威，指定 tunnel 作为本 Zone 的可信来源代理，用其 `transport_did`
  标识代理权威，`session_key` 绑定具体实例、端点与远端上下文。不同 tunnel 不自动合并状态。
- `viewer` 是实际操作者，`owner` 是当前历史所属身份，`member_did` 是被修改状态的成员。
  三者不能混用；Agent 观察视角首期不允许任何状态写入。

## 3. 数据模型与修改契约

以下为逻辑类型，字段用消息域 snake_case；DID、CanonValue、MsgObject 复用现有基础类型。
这些尚不是已经存在的 RPC 返回类型。

```ts
interface StateVersion {
  revision: string
  updated_at_ms: number
}

interface SessionSharedState extends StateVersion {
  schema_version: 1
  session: SessionStateRef
  title?: string
  description?: string
  extensions: Record<string, CanonValue>
}

interface SessionMemberState extends StateVersion {
  schema_version: 1
  session: SessionStateRef
  member_did: DID
  nickname?: string
  extensions: Record<string, CanonValue>
}

type SessionStateTarget =
  | { scope: 'shared'; session: SessionStateRef }
  | { scope: 'member'; session: SessionStateRef; member_did: DID }

interface SessionStatePatch {
  target: SessionStateTarget
  expected_revision: string
  idempotency_key: string
  set: Record<string, CanonValue>
  unset: string[]
}
```

- 持久主键分别是 `(authority_did, session_key)` 与 `(authority_did, session_key, member_did)`。
  共享状态和每位成员状态分别维护 revision，避免不同成员改昵称互相覆盖。
- `revision` 是权威分配的不可由客户端猜测或改写的版本 token，更新使用相等比较，不能以客户端时间决定覆盖。
  读取不存在的成员覆盖状态时返回空字段与权威的初始版本 token，首次写入也做版本检查。
- `set` / `unset` 使用 schema 定义的字段名，不接受任意 JSON 路径；同字段不可同时 set 与 unset。
  缺省表示不修改，unset 表示清除覆盖，JSON null 仅在字段本身允许时表示值。
  标题 / 昵称 trim 后非空，暂沿用标题 64 字符上限，昵称同为 64；清除必须用 unset。
  description 上限 1024 字符；extensions 使用命名空间且必须有字段 schema、写权限和可见性定义，未知写入键拒绝。
- 整体状态修改按字段授权；成员默认可以修改自己的 nickname，不能通过 extensions 写入 role / membership 等权限字段。
  管理员代改、Agent 代操作均需独立授权，日志保留实际操作者。后端从认证上下文确定操作者，不能相信请求自报 DID。
- 状态编辑权限与“可发送普通消息”分开；解除 tunnel Composer 只读不授予改群名、改成员资料的权限。
  tunnel 还须声明对应状态写入能力；只支持读取状态时，修改请求明确返回不支持。
- 接受本地更新时原子检查权限、expected_revision 与字段约束，再提交新状态及对应日志登记。
  并发冲突不覆盖，返回当前版本供重读；无实际变化不递增版本、不产生变更日志；失败请求不产生成功日志。
  同一幂等键和相同请求返回原结果，换载荷复用同一键拒绝，不能重复生效或重复记日志。
- 外部状态写入先提交平台操作，只有可信 API 确认或平台事件确认生效后，才能更新已确认快照并产生日志。
  平台超时 / 接受但尚未确认时保留 pending，不能将乐观显示当成成功事实。
  pending 返回稳定 operation_id 供查询；重试关联同一操作，平台结果未知时先对账，不能重新制造一次修改。

当前状态快照用于读取最新值；Action Log 用于解释历史。普通客户端不能通过发送或重放一条日志修改状态。

## 4. Action Log Message

本文的 Action Message（即 Action Log Message）是“状态已发生变化”的不可变消息，
是标准 `MsgObject` 中可以识别的业务子类型，复用现有字段，不新增顶层消息大类：

```text
kind = 'event'
content.format = 'text/plain'
content.content = 可读摘要，例如：Alice 将会话标题更新为“发布准备”
content.machine.intent = 'buckyos.action_log'
content.machine.data = ActionLogData
```

标准实现 `cyfs-ndn/src/ndn-lib/src/msgobj.rs` 已有 `MsgObjKind::Event`、`MachineContent.intent`
与 `MachineContent.data`，Desktop 协议镜像也有对应字段。标准目前没有独立 `Action` 枚举，
也没有专用 Action Log payload / 判断函数；`buckyos.action_log` 是本设计新增的业务识别约定。
`Operation` 在现有 tunnel 设计中用于投票、审批卡片等可操作消息；本节记录已发生事件，使用 `Event`。

识别分两层：`kind=event + intent=buckyos.action_log` 识别整个 Action Message 类型，
`machine.data.action` 再区分 `entity.member_joined` / `session.title_changed` 等具体动作。
`content.format` 只描述摘要格式，不能用它、摘要文案或 `ui_item_kind=status` 识别 Action Message。

建议后续共用的分类函数（此处仅定义契约，尚未加入实现）：

```ts
function isActionMessage(message: MsgObject): boolean {
  return message.kind === 'event'
    && message.content.machine?.intent === 'buckyos.action_log'
}
```

该判断只识别消息类别。读取 actor / changes 等字段前仍须按 schema_version 校验载荷；
未知 action / 版本或非法字段使用可读摘要回落，不冒充已经通过完整 schema 校验，更不代表事件来源已经可信。

```ts
type FieldValue = { present: false } | { present: true; value: CanonValue }
type ActionTarget =
  | { kind: 'session'; session: SessionStateRef }
  | { kind: 'entity'; entity_did: DID }

interface ActionLogData {
  schema_version: 1
  event_id: string
  action: string
  target: ActionTarget
  actor_did?: DID
  subject_did?: DID
  occurred_at_ms: number
  changes: Array<{ field: string; before?: FieldValue; after?: FieldValue }>
  revision_before?: string
  revision_after?: string
  source: {
    kind: 'native' | 'tunnel'
    producer_did: DID
    source_event_id?: string
    source_event_obj_id?: ObjId
    operation_id?: string
    tunnel_instance_id?: string
    endpoint_did?: DID
    remote_context_id?: string
    source_revision?: string
  }
}
```

字段语义：

- `(source.producer_did, event_id)` 稳定标识一次已确认事件，重试和广播不重新生成 ID；
  `msg_id` 仍按不可变 MsgObject 内容寻址，两者用途不同。
  tunnel 来源必须保留 tunnel_instance_id 与 endpoint_did，非默认上下文带 remote_context_id；
  source_event_id / source_revision 只在该来源连接内解释，不能跨端点去重或比较版本。
- `target` 是被修改的 Session / 实体；`actor_did` 是真实操作者；`subject_did` 是受影响成员。
  自己改昵称时 actor = subject，管理员移除成员时两者不同。未知外部操作者应省略并显示未知，不能猜成 bot / 群主。
- 原生消息 `from` 为发布已确认事件的权威实体 / 服务，操作者在 actor 中记录；不能伪造操作者签名。
  tunnel 入站仍遵循 exact source shadow DID：有平台 actor 则用 actor 端点，只有系统来源则用已映射的系统端点；
  找不到可信来源时走诊断 / REQUEST_BOX，不伪造 DID。source 由可信接入方校验 / 填入，单靠载荷不能自证可信。
- `to[]` 仍为真实接收实体 DID，群事件指向群 DID；SessionStateRef 不可作为 `to` 地址。
  会话绑定决定每个 owner 的本地 session_id，不能把所有日志塞进名为 `buckyos.action_log` 的共用 Session。
- `changes` 保存获准公开的变更字段。`before/after` 缺省表示不知道或未公开，`present:false` 表示确实没有该字段，
  与 `present:true,value:null` 区分。服务端计算原生 before/after，不接受客户端伪造。
  原生状态变更须带对应快照的前后 revision；外部平台缺少旧值或版本时允许省略，不能杜撰。
- `occurred_at_ms` 是有依据的发生时间；缺少平台时间时用接入确认时间。`MsgObject.created_at_ms` 是消息生成时间，
  二者不能代替远端事件序号决定状态新旧。展示摘要可本地化，业务判断不能解析摘要字符串。

### 4.1 动作与示例

| action | target | actor / subject | 示例 changes / 摘要 |
|---|---|---|---|
| `session.title_changed` | session | actor = 修改者；无 subject | `title: 讨论 → 发布准备`；“Alice 将会话标题更新为‘发布准备’” |
| `session.shared_state_changed` | session | actor = 修改者 | 说明等整体字段变化 |
| `session.member_state_changed` | session | actor = 操作者；subject = 成员 | `nickname: Alice → 值班 Alice`，仅影响这个 Session |
| `entity.member_joined` | 群 entity | actor = 执行动作的人（有依据时）；subject = 加入者 | “Bob 加入群聊” |
| `entity.member_left` | 群 entity | 自愿退出时 actor = subject | “Bob 退出群聊” |
| `entity.member_removed` | 群 entity | actor = 管理员；subject = 被移除者 | “Alice 将 Bob 移出群聊”，不能伪装成主动退出 |
| `entity.profile_changed` | entity | actor = 修改者 | 群名 / 实体全局名称变化，不改各 Session 标题 |

一次提交同时改多个共享字段时产生一个 `session.shared_state_changed`，不再为 title 重复发第二条；
仅修改 title 使用 `session.title_changed`。action 可扩展，未知 action 保留结构与摘要，禁止作为可执行指令。
实体级日志与 Session 日志用 target.kind 区分；实体日志按相关实体的默认会话绑定进入历史，
默认不复制到它的全部 Session。每条本地 MailboxRecord 仍只归属一个 Session；无绑定时走待归类流程，不能静默丢弃。

### 4.2 标题变更的结构化载荷示例

```json
{
  "schema_version": 1,
  "event_id": "event-42",
  "action": "session.title_changed",
  "target": { "kind": "session", "session": { "authority_did": "did:bns:team", "session_key": "release-thread" } },
  "actor_did": "did:bns:alice",
  "occurred_at_ms": 1788652800000,
  "changes": [{ "field": "title", "before": { "present": true, "value": "讨论" }, "after": { "present": true, "value": "发布准备" } }],
  "revision_before": "r7",
  "revision_after": "r8",
  "source": { "kind": "native", "producer_did": "did:bns:team", "operation_id": "op-42" }
}
```

### 4.3 UI 展示与隐藏边界（原型后续实现）

- 特殊展示和“隐藏 Action Message”使用同一个类别判断；其它 `event` 不自动属于 Action Message。
  具体动作可映射不同摘要 / 图标，保留完整 MsgObject 供详情查看。
- 隐藏是个人 UI 展示偏好，不写入 MsgObject / ActionLogData，不改权威状态、日志存储或后端消息列表。
  切换显示开关本身不触发删除、归档、标已读或重新广播；关闭隐藏后可恢复完整历史。
- 在 UI 的可见投影层过滤，保留原始 reader 与消息索引；按剩余可见消息重建时间分隔与可见条目数，
  避免空行或只有日期的分隔。具体组件与过滤开关在后续原型阶段实现。
- 特殊 renderer 应在通用文本 / 图片 renderer 前识别此类型；不能用 renderer 返回 null 实现隐藏，
  否则当前 renderer 链还会继续尝试通用文本并重新显示。类型识别、是否可见与具体呈现是三个独立步骤。

## 5. 一致性、可见性与持久化

- 对原生状态权威，状态快照、版本更新、幂等结果与待发布事件在同一持久事务提交。
  named store 写对象、mailbox / delivery 扇出或跨服务发布不视为同一数据库事务：
  事务内保留稳定 event_id 与完整待发布载荷，后台幂等物化 MsgObject / 投递记录，失败可重试，重启可恢复。
  MsgObject 的生成时间、nonce、接收目标和内容须在首次物化前固定，重试沿用同一个 msg_id。
  外部平台操作不能纳入本地事务，使用 §3 的 pending / 确认流程。
- 状态快照和日志都属持久数据；Session 索引重建不丢状态和待发布日志。
  从历史日志重放仅由可信同步 / 修复流程执行，不能让任意入站 `event` 直接改状态或 ACL。
- 日志可见范围不超过相关状态 / 成员事件的可见范围。nickname 默认对会话可见成员共享；
  草稿、私人备注、个人标题等不进入共享 Action Log。扩展字段按自己的可见性过滤 before/after，
  不得为了审计把不可见旧值发送给新成员。接收范围由状态权威在变更时确定并保存，不由重试时的成员列表随意扩大。
- GroupMgr 已有 GroupEvent 作为群变更来源；按原 event_id / 对象引用确定性映射 Action Log，
  不重做一套入群逻辑、不把入群日志当作 GroupMemberProof，也不因映射再生成第二次业务事件。
- 同一远端变更的 API 确认与 webhook 回显，用 source_event_id / operation_id 在具体连接范围内关联去重。
  无法可靠关联时先对账，不以“标题和时间相近”猜测同一事件。乱序旧事件可以保留历史，但不能回滚最新快照；
  无可信远端版本时重新读取权威状态。来源只提供当前快照时不能虚构操作者与逐次历史变更。
- Action Log 是持久 `event`，进入普通 mailbox / Session 历史读取路径，沿用现有 event 的未读规则；
  它不是可随时清除的 status pill。后续原型可为其增加专用呈现，本次不要求渲染器或组件改动。

## 6. 现状与待落地项

| 当前仓库证据 | 已有能力 | 本设计仍需补齐 |
|---|---|---|
| `src/kernel/buckyos-api/src/msg_center_client.rs`：`UiSessionStateEntry` / `ui_session.*` | Session ID + key/value；active / typing / status_line 常量 | 权威引用、成员 DID、字段权限、revision、幂等状态修改 |
| `src/frame/msg_center/src/tg_tunnel.rs`：`TgUiSessionTracker` / `refresh_ui_sessions` | 活跃追踪、typing 与状态行同步 | 持久共享 / 成员状态读写、外部变更确认与 Action Log 映射；不能把现有运行态当作已完成实现 |
| `src/kernel/buckyos-api/src/group_mgr.rs`：`GroupMemberRecord` / `GroupEvent` | 成员权威数据及 joined / left / removed 等事件类型 | 群事件到统一 Action Log 的幂等发布与状态 / 事件提交一致性 |
| 现有 `MsgObject.kind=event` / `content.machine` | 容纳结构化事件 | `buckyos.action_log` payload 校验、来源校验与关联 |
| `src/frame/msg_center/src/msg_center.rs`：`is_group_message` | 当前只按 `kind=GroupMsg` 识别群消息 | 为发往群 DID 的 Action Log event 补齐群接收 / 分发与 Session 归类，不能改成普通聊天消息来绕过 |

实现阶段再确定 RPC 名称、DDL 与版本升级，并同步 Rust / TS 镜像。
本契约不新增 `MsgObjKind`，不将新状态放进旧 `ui_session` 无约束 KV，也不改变已经实现的原型。
GroupEvent 当前由群名、毫秒时间与事件类型构造 event_id；实现映射前还需保证同毫秒同类型的不同变化不会发生 ID 碰撞。

数据层验收至少覆盖：整体状态授权；成员只能改自己的允许字段；Agent 观察写入被拒；
共享标题与个人显示标题隔离；并发版本冲突；幂等重试 / 无变化不重复记日志；
入群、主动退出、被移除的 actor / subject 区分；外部未知操作者 / 旧值不伪造；
平台写入未确认不发成功日志；确认 / 回显去重及乱序对账；事务后发布失败与重启恢复；
私密字段不泄露；日志注入不能修改权威状态；群目标 event 正确分发；同毫秒不同事件 ID 不碰撞；typing 不进入持久历史。
