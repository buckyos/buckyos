
# MsgCenter / MsgTunnel Design Freeze TODO

> 状态（2026-07-14）：P0 / P1 / P2 / P4 文档冻结工作已完成（Issue #510 评论待发布，草稿见
> [issue-510-design-decision-draft.md](./issue-510-design-decision-draft.md)）。
> P3 是文档冻结后的实现迁移项，未动工。

## P0：固定共同架构定位

- [x] 在两份设计文档开头加入统一定位：

  > MessageCenter is not an IM server. It is a DID-native, store-and-forward personal messaging system—email upgraded for Personal Servers and Agents.

- [x] 明确 MessageCenter 运行在 Personal Server 上，主要服务一个用户、其设备和 Agent；设计优先级是可靠性、可恢复性、可审计性和语义清晰，不是中心化 IM 的极限吞吐。

- [x] 增加“非目标”：

  - 不把在线连接作为消息系统中心。
  - 不把 Session 作为消息真相源。
  - 不根据 ContactMgr、在线状态或最近活跃信道自动选路。
  - 不要求 UI 理解 Inbox、Delivery Queue 等内部实现。
  - 不把 typing、presence、streaming 状态混入可靠消息投递。

- [x] 固定五层模型：

  ```text
  MsgObject          不可变消息本体
  DeliveryEnvelope   一次确定投递的信封
  MailboxRecord      某个 owner 对消息的本地引用
  DeliveryRecord     投递队列、重试和结果
  SessionProjection  UI/Agent 的会话视图
  ```

- [x] 在两份文档中加入 Email → BuckyOS 概念映射表。

---

## P1：重写 Message Center 设计文档

目标文件：[Message Center.md](</Users/liuzhicong/project/buckyos/doc/message_hub/Message Center.md>)（已全文重写）

### 1. 重新定义数据模型

- [x] 明确 `MsgObject` 只保存不可变语义：

  - `from/to`
  - content
  - proof
  - topic/reply/correlation
  - object references

- [x] 把 `RouteInfo` 重新定义为 `DeliveryEnvelope/DeliverySnapshot`，明确它不是自动路由输入，而是确定性解析后的投递结果。

- [x] 将当前 `MsgRecord` 的三类状态在设计上拆开：

  ```text
  RecipientState: UNREAD / READING / READ / ARCHIVED / DELETED
  DeliveryState:  WAIT / SENDING / SENT / FAILED / DEAD
  SessionState:   typing / active / status_line
  ```

- [x] 决定并统一命名（已定案）：

  - `OUTBOX` → `SENT`
  - `TUNNEL_OUTBOX` → `DELIVERY_QUEUE`
  - `TunnelOutboxRecord` → `DeliveryRecord`

- [x] 明确 `SENT` 是发送历史，不代表最终投递成功。

- [x] 明确 `DELIVERY_QUEUE` 是内部 transport 队列，不允许 UI 当作会话历史读取。

### 2. 重写入站流程

- [x] 固定流程：

  ```text
  validate envelope
  → store immutable MsgObject
  → create recipient mailbox record
  → update session index
  → notify Agent/UI
  ```

- [x] 明确私聊、群聊、RequestBox 的 mailbox owner。

- [x] 明确外部平台入站的 `from` 保持 shadow endpoint DID；ContactMgr 关联不能改写消息来源。

- [x] 明确群消息：

  ```text
  from = actor shadow DID
  to   = group shadow/real DID
  ```

  回复群聊使用 group `to`，不能回复 actor `from`。

### 3. 重写发送流程

- [x] 删除所有“发送时查询 ContactMgr binding”的描述。

- [x] 固定：

  ```text
  msg.from → SenderRecord owner
  msg.to   → deterministic DeliveryEnvelope
  transport_did → DeliveryQueue owner/executor
  ```

- [x] `post_send()` 必须先验证所有目标，再写入 `SENT + DeliveryRecord`，避免当前“路由失败但 OUTBOX 已经是 SENT”的状态。

- [x] 明确多收件人语义。每个 `to` 生成独立 `DeliveryRecord` 和独立结果；一个目标失败不污染其他目标。

- [x] 增加两条确定投递分支：

  - shareable DID → MessageHub/native delivery
  - local shadow DID → MessageTunnel delivery

- [x] 明确任何解析失败都返回错误，禁止 default tunnel、default chat、last-active fallback。

### 4. 增加 Session Projection

- [x] 明确 Session 不是 MsgBox。

- [x] 定义：

  ```text
  SessionProjection(owner, session_id)
    = RecipientRecord
    + SenderRecord
    + aggregated DeliveryState
  ```

- [x] 增加推荐 API：

  ```text
  list_sessions(owner, cursor, limit)
  list_session(owner, session_id, cursor, limit, with_object)
  ```

- [x] UI 只调用一次 `list_session()`，不分别读取 Inbox、Sent、Delivery Queue。

- [x] 固定 `MsgObject.thread.topic` 与本地 `MailboxRecord.session_id` 的区别：

  - `thread.topic` 是消息携带的语义 hint。
  - `session_id` 是 Personal Server 的本地投影 key。
  - MessageCenter 可以建立映射，但不能修改 MsgObject。

### 5. 重写存储章节

- [x] 保留“不可变对象 + RDB reference/index”的总体模式。

- [x] 增加 Session 索引：

  ```text
  (owner, session_id, sort_key, record_id)
  ```

- [x] 增加 Delivery 索引：

  ```text
  (executor_did, delivery_state, next_retry_at)
  (msg_id, target_did)
  ```

- [x] 明确消息对象只存一份，多个 mailbox/delivery record 引用同一个 `msg_id`。

- [x] 描述本地事务边界、服务崩溃恢复和索引重建原则。

---

## P1：重写 Message Tunnel 设计文档

目标文件：[Message Tunnel Design.md](</Users/liuzhicong/project/buckyos/doc/message_hub/Message Tunnel Design.md>)（已全文重写）

### 1. 收窄 Tunnel 职责

- [x] 将 Tunnel 定义为 external transport adapter：

  ```text
  ingress producer + delivery executor
  ```

- [x] 明确 Tunnel 不负责：

  - Contact 选择
  - Agent Session 管理
  - 自动选择收件信道
  - 身份合并
  - UI 会话历史
  - 根据缺失字段猜测 chat/address

- [x] 将 MessageHub 从“典型 MessageTunnel”中分离出来：

  - MessageHub 是原生、跨 Zone transport。
  - MessageTunnel 是 Telegram、Email、Lark 等外部网络适配器。
  - 二者可以复用 `DeliveryExecutor` 接口，但语义不同。

### 2. 固定 DID 分类

- [x] 删除“Message Tunnel 二级 DID”术语，统一改成“local shadow endpoint DID”。

- [x] 文档固定两类 DID：

  ```text
  Shareable DID
    - did:bns:alice
    - did:bns:telegram.alice

  Local shadow endpoint DID
    - did:msgtunnel:12345.user.tg-main-tunnel
  ```

- [x] 明确 `did:bns:telegram.alice` 的实际信道映射发生在 Alice 的 Zone，不能公开指向一个远端不可解析的 shadow DID。

- [x] 明确 shadow DID：

  - 只在当前 MessageCenter/Zone 范围解析。
  - 可以作为消息 `from/to`。
  - 可以直接回复。
  - 不能发布到 Profile/DID Document。
  - 不能作为远端 Contact 或 MessageHub 目标。

- [x] 固定 `tunnel_instance_id`：

  - 本地唯一。
  - 稳定。
  - 不可复用。
  - 推荐以 `-tunnel` 结尾。
  - 重复注册必须失败，不能静默覆盖。

### 3. 删除旧自动路由描述

- [x] 删除以下发送模型：

  ```text
  contact DID + selector
  resolve_target(contact, "telegram")
  get_preferred_binding()
  last_active_at
  preferred_tunnel
  default tunnel
  ```

- [x] 将 `resolve_target()` 降级为 UI/联系人查看辅助，从核心发送 API 中移除。

- [x] 明确 ContactMgr 只负责：

  - shadow endpoint 与正式联系人关联
  - 展示
  - ACL
  - merge
  - reverse lookup

- [x] ContactMgr 关联结果不得自动成为发送目的地。

### 4. 固定 ingress/egress contract

- [x] 入站必须产生：

  ```text
  MsgObject
  exact source shadow DID
  exact recipient/group DID
  ingress delivery metadata
  external idempotency key
  ```

- [x] 出站 Tunnel 只能消费完整的 `DeliveryRecord`。

- [x] `DeliveryRecord` 缺少目标 address/chat 时必须失败；禁止从以下字段逐级猜测：

  ```text
  chat_id → extra → account_id → default_chat_id
  ```

- [x] `extra/meta` 只能保存平台扩展信息，不能成为核心路由依据。

- [x] 明确 DM、Group、Channel、Email address 的 endpoint 生成和回复规则。

### 5. 重写可靠性章节

- [x] 采用 Email 风格的投递状态机：

  ```text
  WAIT → SENDING → SENT
                 ↘ FAILED → WAIT
                          ↘ DEAD
  ```

- [x] 固定幂等键：

  ```text
  msg_id + target_did + executor_did
  ```

- [x] 区分：

  - submission accepted
  - transport accepted
  - remote delivered
  - remote read

- [x] Delivery failure 更新 `DeliveryRecord`，不修改 `MsgObject`。

- [x] UI 通过 SessionProjection 得到聚合状态，不直接读取 Delivery Queue。

---

## P2：更新 Minimal Spec 和关联文档

- [x] 让 [Message Tunnel Minimal Spec.md](</Users/liuzhicong/project/buckyos/doc/message_hub/Message Tunnel Minimal Spec.md>) 成为两份主设计文档的裁剪版，不再独立定义不同术语。

- [x] 更新 [Contact Mgr.md](</Users/liuzhicong/project/buckyos/doc/message_hub/Contact Mgr.md>)：

  - 移除 ContactMgr 自动选路职责。
  - 删除 `get_preferred_binding`（UI 改用 `list_endpoints` 点选）。
  - 删除 `did:bns:mc-*` 历史 shadow fallback。
  - 将 `last_active_at` 限定为 UI/merge 信息。

- [x] 更新 [Multi-Channel Message Requirements.md](</Users/liuzhicong/project/buckyos/doc/message_hub/Multi-Channel Message Requirements.md>)：

  - Session 改为 projection。
  - UI 只使用 Session API。
  - 投递状态来自 DeliveryRecord 聚合。

- [x] 更新 Self-Host Group：

  - `GROUP_INBOX` 是 group 的权威 mailbox。
  - 群投递使用 DeliveryRecord。
  - 群 Session 是 group mailbox 的投影。
  - （附带修正：§2.7 旧的 `from=group, source=author` 规范化改写模型 → 冻结的 `from=actor, to=group`，消息创建后不得改写。）

- [x] 确认 My Network 文档中的 User DID → Zone → MessageHub 流程与新设计一致（一致；仅修正 §4.1 “binding 是路由基础”一句措辞）。

- [ ] 在 Issue #510 写入最终 Design Decision，链接两份更新后的主文档。
  - 草稿已就绪：[issue-510-design-decision-draft.md](./issue-510-design-decision-draft.md)，确认后发布。

---

## P3：文档冻结后的实现 TODO

- [ ] 删除 Workflow 中 `"telegram" / "tg-main"` 隐式 `resolve_target`。

- [ ] 实现一级/shareable DID 的 MessageHub delivery plan。

- [ ] 删除 Telegram `default_chat_id` 路由 fallback。

- [ ] 修复 `tunnel_id` 与 `tunnel_did` 混用（冻结术语：`tunnel_instance_id` vs `transport_did`）。

- [ ] Tunnel registry 重复 ID 改为启动失败。

- [ ] 拆分 mailbox state 与 delivery state。

- [ ] 将 `OUTBOX` 重命名为 `SENT`。

- [ ] 将 `TUNNEL_OUTBOX` 泛化为 `DELIVERY_QUEUE`。

- [ ] 增加 `list_session/list_sessions` 及数据库索引。

- [ ] 聚合 DeliveryRecord 状态到 Session message view。

- [ ] 删除 `MsgObject.thread.tunnel_id`（在 cyfs-ndn 仓库 `ndn-lib/src/msgobj.rs`）。

- [ ] 删除旧群消息 `to` 为空时回退到 `from` 的兼容逻辑。

- [ ] Desktop MessageHub 从 mock reader 切换到 Session API。

---

## P4：设计验收（已逐条核对通过，2026-07-14）

- [x] 文档中不再出现“MessageCenter 根据联系人绑定自动选择 Tunnel”（仅存于“已废弃/禁止”列表中）。

- [x] 所有发送示例的 `to DID` 都能唯一确定投递类型。

- [x] `did:bns:bob` 永远不会自动生成 Telegram DeliveryRecord。

- [x] `did:bns:telegram.bob` 只由 Bob Zone 展开。

- [x] shadow DID 不能跨 Zone 作为目标使用。

- [x] 一个传统私聊 UI 只需要一次 `list_session()`。

- [x] UI 不读取 Delivery Queue。

- [x] 同一 `MsgObject` 可以被多个 MailboxRecord/DeliveryRecord 引用，但内容只存一份。

- [x] 服务重启、重复提交和投递重试不会产生不可控重复消息（幂等键 + 唯一约束 + SENDING 租约 sweep，见两文档可靠性/存储章节）。

- [x] 新贡献者只阅读两份主设计文档，就能准确说明：

  ```text
  MessageCenter ≠ IM Server
  MessageCenter = DID-native durable mailbox
  MessageHub/MessageTunnel = delivery transport
  Session = mailbox projection
  ```
