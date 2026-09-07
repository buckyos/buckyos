# MessageCenter 设计文档

- 2026-09-07：按当前源码校正 Session UI 接入边界；新增约束为目标契约，未在本次修改中实现。

> **MessageCenter is not an IM server. It is a DID-native, store-and-forward personal messaging system—email upgraded for Personal Servers and Agents.**

本文档与 [Message Tunnel Design.md](<./Message Tunnel Design.md>) 共同构成消息域的两份主设计文档，共享同一定位、同一五层模型和同一术语表。任何与本文冲突的旧描述（包括代码注释和历史文档）以本文为准。

## 1. 定位

MessageCenter 运行在用户的 Personal Server（Zone）上，主要服务**一个用户、这个用户的设备和 Agent**。它是消息域的真相源：所有消息先持久化为不可变对象，再按确定的规则投递、引用和展示。

设计优先级依次是：

1. **可靠性**：消息不丢、不重，投递失败可重试。
2. **可恢复性**：服务崩溃、重启、断网后能从持久状态完整恢复，不依赖任何在线连接。
3. **可审计性**：谁发了什么、投递到哪、结果如何，事后全部可追溯。
4. **语义清晰**：每个概念只有一个含义；UI、Agent、transport 各自只依赖自己那一层。

**不是**设计目标的：中心化 IM 的极限吞吐、在线撮合、全局一致的会话状态。单用户 Personal Server 的消息量级由真实的人和少量 Agent 决定，正确性永远优先于吞吐。

### 1.1 非目标

- **不把在线连接作为消息系统中心。** webhook、long polling、kevent、stream 都只是加速信号；真相永远在持久化的 mailbox 和 delivery queue 里，消费者必须能靠扫描补偿。
- **不把 Session 作为消息真相源。** Session 是 mailbox 的投影，可以随时丢弃重建；删除全部 session 索引不丢失任何消息。
- **不根据 ContactMgr、在线状态或最近活跃信道自动选路。** `msg.to` 写的是哪个 DID，就确定性地投递到哪；解析失败就是错误。
- **不要求 UI 理解 Inbox、Delivery Queue 等内部实现。** UI 只消费 Session API（见 §5）。
- **不把 typing、presence、streaming 中间态混入可靠消息投递。** 它们是易失的 SessionState，走独立通道，不产生 MailboxRecord。

### 1.2 五层模型

消息域的全部设计围绕五层展开。评审任何新需求时，第一个问题是"它落在哪一层"：

```text
MsgObject          不可变消息本体（内容寻址，全系统只存一份）
DeliveryEnvelope   一次确定投递的信封（post_send 解析产物，不是路由输入）
MailboxRecord      某个 owner 对消息的本地引用（INBOX / SENT / GROUP_INBOX / REQUEST_BOX）
DeliveryRecord     投递队列、重试和结果（DELIVERY_QUEUE，owner = 投递执行者）
SessionProjection  UI/Agent 的会话视图（由上面各层聚合派生，可重建）
```

层间依赖是单向的：上层可以引用下层，下层不知道上层存在。`MsgObject` 不知道自己被投递到哪；`DeliveryRecord` 不知道 UI 怎么展示；`SessionProjection` 不持有任何独立真相。

### 1.3 Email → BuckyOS 概念映射

MessageCenter 的心智模型是"为 Personal Server 和 Agent 升级过的 email"，而不是 IM：

| Email 世界 | BuckyOS 消息域 | 说明 |
|---|---|---|
| RFC 5322 message（信件本体） | `MsgObject` | 不可变、可签名、内容寻址 |
| Message-ID | `msg_id`（MsgObjectId） | canonical JSON hash |
| SMTP envelope（`RCPT TO`） | `DeliveryEnvelope` | 信封与信件分离；投递看信封不看信件 |
| 收件人地址 | shareable DID / local shadow endpoint DID | 分类见 Tunnel 文档 §3 |
| MX 解析 | DID → Zone 解析 | 确定性协议，不是"智能选路" |
| MTA 队列与重试 | `DeliveryRecord`（`DELIVERY_QUEUE`） | `WAIT → SENDING → SENT / FAILED / DEAD` |
| MTA / smarthost / gateway | MessageHub（原生）/ MessageTunnel（外部） | 两类 DeliveryExecutor |
| DSN / bounce | DeliveryReport → 更新 `DeliveryRecord` | 永不修改 `MsgObject` |
| IMAP mailbox + `\Seen` flag | `MailboxRecord` + `RecipientState` | 每个 owner 独立管理 |
| Sent 文件夹 | `SENT` mailbox | 发送历史 ≠ 投递成功 |
| MUA 的会话/线程视图 | `SessionProjection` | 客户端投影，可重建 |
| 邮件规则/过滤器 | 入站 policy（ContactMgr ACL → INBOX / REQUEST_BOX / DROP） | 只影响 mailbox 归属，不改消息 |

Email 没有做好而 BuckyOS 升级的部分：DID 原生身份与签名、群实体（group 自己有 mailbox）、Agent 作为一等收发方、投递状态对发送方可见（聚合进 Session 视图）。

---

## 2. 数据模型

### 2.1 MsgObject：不可变消息本体

定义见 `ndn_lib::MsgObject`。`MsgObject` 只保存**不可变语义**，一经创建永不修改（内容寻址，改一个字节就是另一条消息）：

- `from` / `to`：消息参与方 DID。入站消息 `from` 保持来源 endpoint DID 原样（见 §3.3）。
- `content`：`MsgContent`，含 `title/format/content/machine/refs`。大对象放对象存储，用 `refs` 引用。
- `proof`：来源签名/证明。
- `thread`：`topic / reply_to / correlation_id` 语义线索。
- `kind` / `created_at_ms` / `expires_at_ms` / `nonce` / `meta`。

**永远不属于 MsgObject 的**：已读状态、投递状态、重试信息、外部平台 message id、归档/删除标记、会话归类。
本地阅读状态属于 `MailboxRecord`；投递与重试属于 `DeliveryRecord`；回执由 `MsgReceiptObj` 单独表达。
会话级生命周期需另有 owner 范围元数据（§5.8）。这些变化均不修改原始 MsgObject。

> 注：`thread.tunnel_id` 是历史遗留字段，冻结设计中删除（transport 信息属于 DeliveryEnvelope 层，不属于消息语义）。`thread.topic` 是消息携带的**语义 hint**，与本地 `session_id` 的关系见 §5.4。

### 2.2 DeliveryEnvelope：一次确定投递的信封

历史上的 `RouteInfo` 承担了两个矛盾的角色：既像"路由输入"（给系统猜从哪发），又像"投递记录"。冻结设计将其重定义为 **DeliveryEnvelope**：`post_send` 对每个 `msg.to` 目标做确定性解析之后的**结果快照**，不是任何自动路由的输入。

```rust
/// 一次确定投递的信封。post_send 解析完成后创建，之后不再改变。
pub struct DeliveryEnvelope {
    pub msg_id: ObjId,          // 引用不可变消息本体
    pub target_did: DID,        // 本次投递的目标（msg.to 中的一个）
    pub transport_did: DID,     // 投递执行者：MessageHub 服务 DID 或某个 tunnel 实例 DID
    pub transport: TransportKind, // Native（MessageHub）/ Tunnel { platform, tunnel_instance_id }
    pub address: Option<DeliverySnapshot>, // 解析后的地址快照（平台 chat/address 等）
}
```

两条确定投递分支（也只有这两条）：

- `target_did` 是 **shareable DID**（如 `did:bns:bob`、`did:bns:telegram.bob`）→ **MessageHub 原生投递**：目标设计为确定性解析 DID、Zone 与语义接收点，发送 MsgObject canonical JSON。当前代码已支持本地 dispatch 及显式配置的跨 Zone CYFS 路由；未配置路由返回 `native-route-not-configured`，配置与投递快照不一致返回 `native-route-changed`。不能仅凭 native DID 假设可达；自动解析能力不作已实现承诺。接收与缓存结果见 §4.5。
- `target_did` 是 **local shadow endpoint DID**（`did:msgtunnel:*`）→ **MessageTunnel 投递**：从 DID 内嵌的 `tunnel_instance_id` 在注册表查出 tunnel 实例，平台地址由 DID 内嵌的 account 信息与 tunnel 配置确定。

任何解析失败（未注册的 tunnel 实例、无法解析的 DID、非法格式）都返回错误。**禁止 default tunnel、default chat、last-active fallback。**

### 2.3 MailboxRecord：owner 对消息的本地引用

```rust
/// 某个 owner 在某个 mailbox 中对一条 MsgObject 的引用与状态。
pub struct MailboxRecord {
    pub record_id: String,       // 可推导：hash(owner + box_kind + msg_id + variant)，天然幂等
    pub owner: DID,              // user / agent / group DID
    pub box_kind: MailboxKind,   // INBOX / SENT / GROUP_INBOX / REQUEST_BOX
    pub msg_id: ObjId,           // 指向不可变 MsgObject（只存引用，不复制内容）
    pub state: RecipientState,   // 见 §2.5
    pub session_id: String,      // 本地会话投影 key（见 §5.4）
    pub sort_key: u64,           // 排序，通常 = msg.created_at_ms
    pub tags: Vec<String>,
    pub created_at_ms: u64,
    pub updated_at_ms: u64,
}

pub enum MailboxKind {
    INBOX,        // 收件：owner 是收件人（user/agent）
    SENT,         // 发送历史：owner 是发送者。注意：SENT ≠ 投递成功，只表示"这条消息从我这里发出过"
    GROUP_INBOX,  // 群权威收件箱：owner 是 group DID
    REQUEST_BOX,  // 低信任消息暂存：owner 是收件人，待用户确认
}
```

同一条 `MsgObject` 可以被多个 `MailboxRecord`（和多个 `DeliveryRecord`）引用，但消息内容全系统只存一份。

命名冻结（旧名 → 新名）：

| 旧名 | 冻结名 | 语义 |
|---|---|---|
| `OUTBOX` | `SENT` | 发送历史 mailbox，**不代表最终投递成功** |
| `TUNNEL_OUTBOX` | `DELIVERY_QUEUE` | 内部 transport 队列，不是 mailbox |
| `TunnelOutboxRecord`（`TUNNEL_OUTBOX` 里的 `MsgRecord`） | `DeliveryRecord` | 投递队列条目 |
| `RouteInfo` | `DeliveryEnvelope` / `DeliverySnapshot` | 确定投递的结果快照 |
| `MsgRecord`（mailbox 语义部分） | `MailboxRecord` | owner 的本地消息引用 |

### 2.4 DeliveryRecord：投递队列、重试和结果

```rust
/// DELIVERY_QUEUE 中的一条投递任务。owner/executor 是 transport_did。
pub struct DeliveryRecord {
    pub delivery_id: String,        // 幂等键派生：hash(msg_id + target_did + transport_did)
    pub envelope: DeliveryEnvelope, // 创建后不变
    pub state: DeliveryState,       // 见 §2.5
    pub attempts: u32,
    pub next_retry_at_ms: Option<u64>,
    pub external_msg_id: Option<String>, // transport 接受后的外部/远端 id
    pub last_error: Option<DeliveryError>, // error_code / message / retryable / duplicate_risk
    pub created_at_ms: u64,
    pub updated_at_ms: u64,
}
```

**`DELIVERY_QUEUE` 是内部 transport 队列**，语义等价于 MTA 的发送队列：

- 消费者只有 delivery executor（MessageHub / 各 tunnel 实例），按 `transport_did` 分队列。
- **UI 不允许把它当作会话历史读取**。UI 需要的投递进度由 SessionProjection 聚合提供（§5.3）。
- 投递结果（成功、失败、重试、DEAD）只更新 `DeliveryRecord` 自身，不回写 `SENT` mailbox 的 `MailboxRecord`，更不改 `MsgObject`。

### 2.5 三类状态，三个状态机

历史实现把三类无关的状态塞进了同一个 `MsgState` 枚举，是多数语义混乱的根源。冻结设计拆为三个独立状态机，分别属于不同的层：

```text
RecipientState（属 MailboxRecord，owner 自己管理）:
  UNREAD → READING → READ；任意状态 → ARCHIVED / DELETED
  （SENT mailbox 无阅读语义，只使用 ARCHIVED / DELETED）

DeliveryState（属 DeliveryRecord，executor 驱动）:
  WAIT → SENDING → SENT
               ↘ FAILED → WAIT   （可重试，带退避）
                        ↘ DEAD   （不可重试或超次数，可诊断、可人工重投）

SessionRuntimeState（本文旧称 SessionState；易失，不落 mailbox/delivery）:
  typing / active / status_line 等 UI 会话状态，独立通道，随时可丢
```

这里的三类是既有消息处理状态机。新增的 Session 整体状态（`SessionSharedState`）和成员状态
（`SessionMemberState`）是持久业务数据，不属于上述易失通道，也不替代 RecipientState / DeliveryState。
字段、权限和 Action Log 契约见 §5.7。

三个状态机互不迁移、互不共享取值。"收件人已读"不影响 DeliveryState；"投递失败"不产生 RecipientState；typing 永远不产生记录。

`msg.update_record_state(record_id, READ)` 更新本地邮箱阅读状态，list_sessions 按记录的 UNREAD 计数。
`msg.set_read_state(group_id, msg_id, reader_did, ...)` 写的是回执，不更新 mailbox；当前回执只保存在
`MessageCenterState.receipts` 内存中，接口必填 group_id，尚无完整私聊回执、持久恢复及发布契约。
UI 不能用它代替清除自己的未读，不能将投递 accepted / delivered 或 Agent 处理中当成 reader 已读。

---

## 3. 入站流程

### 3.1 固定流程

所有入站消息（tunnel、MessageHub、系统内部）走同一条五步流水：

```text
validate envelope            校验来源、幂等键、格式；应用 ContactMgr 准入策略
→ store immutable MsgObject  内容寻址幂等写入，已存在即跳过
→ create recipient mailbox record   为每个本地收件方创建 MailboxRecord
→ update session index       计算/关联 session_id，更新 (owner, session_id) 索引
→ notify Agent/UI            发布变更通知（加速信号，失败不阻断写入）
```

```python
def dispatch(msg_obj, ingress_meta, idempotency_key):
    # 1. validate envelope
    ensure_valid(msg_obj)                       # from/to/kind/签名 格式校验
    if seen(idempotency_key):                   # 入站幂等（持久化，非内存）
        return already_dispatched()
    decision = contact_mgr.check_access(msg_obj.from, owner=local_recipient(msg_obj))
    if decision == Block:
        record_drop(idempotency_key); return rejected()

    # 2. store immutable MsgObject（幂等）
    named_store.put_if_absent(msg_obj.id, msg_obj)

    # 3+4. 事务内创建 mailbox record + session 索引
    with rdb.tx():
        for owner, box in mailbox_targets(msg_obj, decision):   # 见 3.2
            rec = put_mailbox_record(owner, box, msg_obj.id,
                                     state=UNREAD,
                                     session_id=derive_session_id(owner, msg_obj))
        mark_seen(idempotency_key)

    # 5. notify（尽力而为）
    notify_owners(...)
```

### 3.2 mailbox owner 规则

| 场景 | mailbox owner | box_kind |
|---|---|---|
| 私聊（`kind=Chat`），收件方是本 Zone 的 user/agent | `msg.to` 中的每个本地 DID | `INBOX` |
| 群聊（`kind=GroupMsg`） | group DID（`msg.to`） | `GROUP_INBOX`（群的权威 mailbox，唯一逻辑主线） |
| 低信任来源（ContactMgr 判为 Stranger 等） | 本地收件人 | `REQUEST_BOX` |

群消息只写一条 `GROUP_INBOX` 权威记录；订阅该群的本地 reader（agent/user）的"未读视图"是 per-reader 的投影记录或 read receipt（见 Self-Host-Group 文档），**不是把 MsgObject 复制多份群消息**。

当前实现为成员创建 INBOX 记录并附 `group:<did>` tag。`MailboxRecord.to` 在所有入站箱均为 owner，
SENT 中仅取原始 msg.to 的第一项。UI 必须从原始 `MsgObject.to` 或权威会话登记识别群与多目标集合，
不能用成员 INBOX 副本的 record.to 识别群，也不能因最后一条消息方向变化重新绑定会话。

### 3.3 入站身份：`from` 保持来源 endpoint DID

外部平台入站消息的 `from` **保持 shadow endpoint DID 原样**（如 `did:msgtunnel:12345.user.tg-main-tunnel`）。

ContactMgr 后续把这个 endpoint 关联到某个正式联系人，只影响**展示层**（UI 显示联系人名字与头像）与 ACL 判断；**不能改写消息来源**，不重写历史 `MsgObject`，也不重写 `MailboxRecord.session_id` 之外的任何字段。消息里记录的是"谁在哪个信道说的"，这是审计事实。

### 3.4 群消息的 from/to

```text
from = actor endpoint DID（外部平台成员是 shadow DID；原生成员是真实 DID）
to   = group shadow/real DID
```

**回复群聊使用 group `to`，不能回复 actor `from`。** 回复 `from` 等于绕开群、私聊那个成员——这必须是显式的用户动作，永远不是默认行为。

---

## 4. 发送流程

### 4.1 职责固定

```text
msg.from      → SENT mailbox 的 owner（SenderRecord）
msg.to        → 逐个解析为确定的 DeliveryEnvelope（解析规则见 §2.2）
transport_did → DELIVERY_QUEUE 的 owner / DeliveryRecord 的 executor
```

`post_send()` 里**没有任何 ContactMgr 查询**。"给 Bob 的 Telegram 发消息"的选择发生在构造 `MsgObject` 之前——由用户在 UI 点选、或 Agent 沿用会话中已有的 endpoint DID——`post_send` 收到的 `msg.to` 必须已经是确定 DID。

### 4.2 先验证，后写入

```python
def post_send(msg_obj, idempotency_key=None):
    ensure(msg_obj.to)                       # 至少一个目标
    ensure_local_sender(msg_obj.from)        # from 必须是本 Zone 的 user/agent/device

    # 阶段一：解析所有目标（纯读、无副作用）。任一失败 => 整体失败，不写任何记录。
    envelopes = []
    for target in msg_obj.to:
        env = resolve_delivery(msg_obj.id, target)   # §2.2 两条分支，无 fallback
        if env is None:
            raise NoDeliveryPath(target)             # 明确错误，调用方可见
        envelopes.append(env)

    # 阶段二：一个本地事务写入全部记录。
    named_store.put_if_absent(msg_obj.id, msg_obj)
    with rdb.tx():
        put_mailbox_record(owner=msg_obj.from, box=SENT, msg_id=msg_obj.id,
                           session_id=derive_session_id(msg_obj.from, msg_obj))
        for env in envelopes:
            put_delivery_record(env, state=WAIT)     # delivery_id 幂等，重复提交命中同一条

    # 阶段三：通知各 executor（加速信号）。
    notify([env.transport_did for env in envelopes])
    return {"msg_id": msg_obj.id, "deliveries": [env.summary() for env in envelopes]}
```

要点：

- **先验证所有目标，再写入 `SENT + DeliveryRecord`。** 杜绝历史实现中"路由失败但 OUTBOX 已经是 SENT"的脏状态：解析阶段失败时数据库里什么都没有。
- `SENT` 记录表示"这条消息从我这里发出过"，是发送历史；**不代表最终投递成功**。投递进度看 DeliveryRecord 聚合。
- 幂等：`msg_id` 内容寻址 + `record_id` / `delivery_id` 可推导，同一消息重复 `post_send` 不产生重复记录。

### 4.3 多收件人语义

**每个 `to` 生成独立的 `DeliveryRecord`，拥有独立的状态与结果。** 一个目标投递失败不污染其他目标：
给 3 个目标发消息，2 个 SENT、1 个 FAILED/DEAD 可同时存在。Session API 提供整体进度摘要，
但 UI 必须保留逐目标结果，不能将 partial_failed 解释为所有目标失败或整条重发。
创建期整体拒绝通过 `PostSendResult.ok=false/reason` 返回，RPC 成功不等于提交成功。
`ok=true` 只确认发送历史和投递队列创建；后续是否送达以 delivery 为准。
提交结果未知时重用原始对象与幂等键；人工重投需独立的目标范围、授权与去重契约，当前没有对应的 UI RPC。

### 4.4 投递执行与回报

executor（MessageHub / tunnel）通过 `get_next(transport_did, DELIVERY_QUEUE, WAIT, lock_on_take=true)` 以 CAS 抢占方式取任务（`WAIT → SENDING`），执行后调用：

```python
def report_delivery(delivery_id, result):
    rec = delivery_store.get(delivery_id)
    if result.ok:
        rec.external_msg_id = result.external_msg_id
        transition(rec, SENT)
    elif result.retryable and rec.attempts < MAX_RETRY:
        rec.last_error = result.error
        rec.next_retry_at_ms = backoff(rec.attempts)
        transition(rec, WAIT)           # FAILED → WAIT 由重试调度完成
    else:
        rec.last_error = result.error
        transition(rec, DEAD)           # 可诊断，支持人工重投
    emit_session_change(rec)            # 触发 SessionProjection 的聚合状态更新
```

处于 `SENDING` 超过租约时间的记录由定时 sweep 收回（→ `WAIT`，`attempts+1`，记录 duplicate risk），覆盖 executor 崩溃场景。

### 4.5 跨 Zone 原生投递与 Gateway 尽力缓存（2026-09-07 实现边界校正）

原生投递遵循 [CYFS dispatch 协议](<../../../cyfs-ndn/doc/CYFS Protocol/CYFS Protocol.md>)。Gateway 先执行 process-chain 安全过滤，再直接尝试 upstream；只有 upstream 失效才调用配置的 NamedInboxCacheServer。公网 VPS 上的 Gateway 可以因此在家庭 OOD 离线时暂存对象，并按配置后台转投。组件与实现入口见 [NamedInboxCacheServer 设计](../../../cyfs-gateway/doc/NamedInboxCacheServer设计.md)。

| 原生 dispatch 结果 | 发送方含义与处理 |
| --- | --- |
| 无响应 | 对方是否已处理未知；保存原对象，退避后按同一目标与 ObjectId 重试 |
| 明确拒绝 rejected | 按 reason/retryable 决定重试或失败；缓存满是可重试的暂时拒绝，不等同永久业务拒收 |
| 已经缓存 cached | 仅表示响应时写入接收侧缓存成功，之后仍可能丢失；本地 DeliveryRecord 继续未完成，发送方保存对象并负责重试 |
| 已经接收 accepted | 本次目标 upstream 已持久接收，或确认此前已经接收；只有此时原生投递才成功 |

`cached` 不移交可靠投递责任。缓存不是 MsgBox，不进入 Session 历史；它只存小对象与必要转投上下文，不 Pull 附件，不依赖 BuckyOS 运行。upstream 正常时不以前置缓存读写为条件，缓存故障不得影响正常转发。缓存满则本次缓存写入失败，不谎报 cached。

Gateway 排空按“取出但暂不删 → 投 upstream → accepted 后删除”执行；临时失败或无响应时尽力保留，永久拒绝可以清理。故障、过期清理或重建导致缓存丢失是允许的。发送方重试和 Gateway 排空可能重复或并发发生，接收适配必须按 `(target_zone, semantic_path, obj_id)` 幂等处理，并且只确认路径指定的本地接收点，不能因 MsgObject 带多个 to 就确认其他目标。

当前 `cyfs_dispatch.rs::delivery_report` 已把 accepted 映射为 ok=true，把 cached 映射为
ok=false、error_code=`cyfs-cached`、retryable=true、retry_after_ms=30000，交由既有投递重试机制处理。
因此 cached 仍待接收确认；UI 应保留该提示，不能提前标成 delivered/read，也不能将其显示为永久拒绝。
发送方的重试次数 / 截止时间仍由自身策略决定，达到上限记录失败，不能标记成功。

原生 executor 可以使用可选的 `GET cyfs://<zone>/<semantic_path>?dispatch-status=<ObjectId>` 查询减少重复传输，但查询不是基础投递的前置条件。查询不支持、失败或 unknown 时继续同对象 PUT；缓存中没有对象不证明已接收。只有查询得到有依据的 accepted 才结束本地投递。

SessionProjection 可以展示“对方网关已暂存，等待接收”，仍属于未完成状态；原生 accepted 不代表远端用户已读、Agent 已处理或附件已下载。外部 Telegram/Email 等 tunnel 的 transport accepted 语义仍按 Message Tunnel Design §6.3 描述。

---

## 5. Session Projection

### 5.1 Session 不是 MsgBox

Session 时间线是**投影**：为 UI/Agent 把分散在多个 mailbox 和 delivery queue 里的记录聚合成"一个会话"的只读视图。它不持有独立的消息真相：

- 删除全部 session 索引，不丢任何消息，可从 MailboxRecord 全量重建。
- 单条消息阅读 / 记录状态更新落到对应的 `MailboxRecord` 上；会话级归档、恢复和删除另见 §5.8，不能与记录状态混用。
- Session 里"这条消息发送中/失败"的角标来自 DeliveryRecord 聚合，UI 不直接读 DELIVERY_QUEUE。

“只读投影”描述存储职责，不表示所有会话的 Composer 都只读；产品写入能力见 §5.6。
尚无消息的已创建会话与稳定连接绑定需要独立的持久登记元数据（§5.5），不能仅靠消息索引重建。

### 5.2 定义

```text
SessionProjection(owner, session_id)
  = RecipientRecord(owner 的 INBOX / GROUP_INBOX / REQUEST_BOX 中 session_id 匹配的记录)
  + SenderRecord(owner 的 SENT 中 session_id 匹配的记录)
  + aggregated DeliveryState（对每条出站消息，聚合其全部 DeliveryRecord）
按 sort_key 合并成单一时间线。
```

### 5.3 API

```text
msg.list_sessions(owner, cursor_updated_at_ms?, cursor_session_id?, limit?, with_object?)
  -> { items: [ { session_id, last_record?: { record, msg? }, unread_count, updated_at_ms } ],
       next_cursor_updated_at_ms?, next_cursor_session_id? }

msg.list_session(owner, session_id, cursor_sort_key?, cursor_record_id?, limit?, descending?, with_object?)
  -> { items: [ { record_id, msg_id, direction, box_kind, sort_key, from, to,
         recipient_state?,                  # 入站记录
         delivery?: {                       # 出站记录：聚合视图
            overall: sending | delivered | partial_failed | failed,
            per_target: [ { target_did, state, attempts, external_msg_id?, last_error? } ]
         },
         msg?                               # with_object=true 时附带 MsgObject
       } ], next_cursor_sort_key?, next_cursor_record_id? }
```

实际聚合规则按优先级为：存在 WAIT/SENDING → sending；否则全部 SENT → delivered；
否则部分 SENT、部分 FAILED/DEAD → partial_failed；否则 failed。没有 delivery 时不返回该字段。

一次 list_session 返回一页双向历史、本地阅读状态和投递视图，无需 UI 拼接 Inbox/Sent 或读取 Delivery Queue。
对端回执独立查询；长历史需分页。当前 list_sessions 没有独立 peer/group 字段、生命周期或最后投递摘要，
并按 MAX(mailbox.updated_at_ms) 排序；UI 需要的有效消息活动时间及同口径游标仍待补齐。
读取或修改旧消息状态不应推进产品 lastActiveAt，前端只重排一页不能修复跨页排序。

### 5.4 `thread.topic` 与 `session_id`

| | `MsgObject.thread.topic` | `MailboxRecord.session_id` |
|---|---|---|
| 归属层 | 消息本体（不可变） | 本地记录（每 owner 独立） |
| 语义 | 发送方携带的**语义 hint** | Personal Server 的**本地投影 key** |
| 由谁定 | 消息作者 | 收/发方本地的 MessageCenter |
| 可变性 | 永不可变 | 可由可信后端/Agent 重新归类 |

MessageCenter 可以在确定的 owner、对端与连接范围内建立 `thread.topic → session_id` 的映射，
但不能跨连接仅凭 topic 同名合并，也**不能修改 MsgObject**。同一条消息在不同 owner 的视图中可以有不同 `session_id`。
当前 `derive_session_id` 优先直接取 topic 的实现尚不满足这一隔离要求。

### 5.5 会话登记与连接隔离（2026-09-06 目标契约，待实现）

产品规则来自 [MessageHub PRD §6 / §9](../../product/message_hub/MessageHub_Web_UI_PRD.md)，
UI 目标字段见 [UI DataModel §3.3](../../src/frame/desktop/src/app/messagehub/UI_DATAMODEL.md)。

- 所有真实通信实体（包括 Agent、群与子群）均由 DID 标识，可进入 `MsgObject.from/to`。
  父子关系、联系人别名归一只影响组织与展示，不自动合并其 Session 或改变消息端点。
- 每条持久消息在每个 owner 的本地记录中必须有 Session 归属；不要求不同 owner 使用相同 Session ID。
  完整引用键为 `(owner, session_id)`，每个 owner 与同一对端可有多个 Session。
- 外部连接按 `(owner, tunnel_instance_id, peer/group endpoint DID)` 区分，建立连接即幂等登记至少一个默认 Session。
  一个 tunnel 实例可连接多个实体；不能为整个实例只建一个共用会话。
- tunnel 支持多个上下文时，再以稳定远端 thread / topic ID 建立 Session 映射；相同文本标题不是身份键。
  不同连接的同名 topic 不合并，双向消息必须回到同一绑定。原生连接也有默认 Session，Agent 可另行手工创建。
- 登记需保存 owner、对端 / 群、连接类型及确定目标、远端上下文关联、创建来源，
  并在首条消息前可持久恢复；列表应合并登记与消息摘要，空会话的历史为空、未读为 0。
  不允许通过伪造 MsgObject、`ui.title` 或可丢弃索引来代替登记。
- 只有实体资料、尚未建立连接或创建会话时允许零 Session。连接断开 / 移除不删除已有消息与登记，
  发送能力变为不可用；不得自动换 tunnel。
- 本地 Session 与远端上下文的显式发送映射须在构造确定投递时完成。
  `session_id` 不能直接冒充外部 thread ID，ContactMgr 归一结果和 ingress 元数据也不能成为隐式选路依据。

登记是会话身份与连接关系的持久元数据，消息正文、收件状态与投递状态仍分别属于既有层。
实现时须同步扩展 durable schema、Session API 与 UI 协议镜像；本节不宣称当前已存在登记 / 创建 RPC。
当前 `MailboxRecord.session_id` 仍为可选，列表仅从消息索引读取，也需在实现阶段补齐上述约束。

### 5.6 查看视角、创建与写入（2026-09-06 目标契约，待实现）

- `viewer` 是实际登录 / 调用身份，`owner` 是被查看的会话所属实体。
  默认查看自己的 Session；从 Agent 主页进入时，经授权以 Agent 为 owner 读取其通信对象与历史。
  API 接受 owner 参数不等于调用者已经获得读取该 owner 的权限，服务端必须校验两者关系。
  用户 DID 取自 `users/{user_id}/profile.did`；前端通过 `user.get` 获取，服务端在验证 token 后读取同一档案，缺失时拒绝访问，不能由用户名拼接 `did:bns`。Agent 观察权限依据本地 Agent 注册文档和未删除状态判断，不能将 Zone 域下的所有 `did:web` 当成 Agent。
- Agent 观察首期只读：不发送、新建、修改已读或配置，不写 Agent 的草稿和 UI 状态。
  未来代 Agent 通信必须校验独立的发送授权，`MsgObject.from` 为 Agent，审计保留实际 viewer。
- 默认仅允许手工创建与 Agent 的 Session；其它会话由连接建立 / 远端上下文发现产生。
  支持 `(owner, entity)` 级“允许手工创建”配置，显式设置覆盖类型默认；
  在 tunnel 内创建还需多 Session 与远端创建能力，配置不能授予平台能力或发送权限。
- MessageHub UI 中外部 tunnel Session 通常默认只读；已有出站能力和发送权限时，
  用户确认“可能造成另一个软件中的会话历史记录错误或不一致”后，可对当前 Session 启用写入。
  此确认是 UI 的产品门槛，不授予服务端权限，也不限制已授权的 Agent / tunnel 正常收发。
  原生会话按授权正常发送，不套用外部 tunnel 风险确认。
- 传输方式与操作能力分开：native 也需确认路由与发言权限；GroupMgr 的按 action 授权可用于
  群历史、发言和管理能力。读取、发送、owner 本地生命周期、共享字段与成员字段编辑分别校验，
  本地托管或存在 mailbox 记录不授予管理权。能力未知时不能按 native 默认开放写入。
- Session UI 状态至少按 `(owner, session_id, key)` 存储与授权；浏览器缓存、草稿和选择态再按 viewer 隔离。
  Agent 的未读数不并入用户 badge，观察不自动标记 Agent 已读。

当前 `list_sessions/list_session` 已有 owner 参数，但对应 handler 未使用 `RPCContext`，
`ui_session` 接口也只有 Session ID 作用域。实现前需补齐 / 验证授权与隔离，不能只改前端传参。

### 5.7 Session 状态与 Action Log（2026-09-06 数据层目标契约，待实现）

主定义见 [Session State and Action Log.md](<./Session State and Action Log.md>)，本轮不修改原型或协议实现。

- 持久状态分为会话整体状态与每个成员自己的会话状态：整体字段按权限修改，成员默认可修改自己的昵称等字段。
  角色 / 群成员资格继续由原有权威管理，不能通过普通状态 patch 改写。
- 本地 `(owner, session_id)` 关联稳定权威会话引用；共享状态与成员状态分别带 revision，
  更新需要 expected_revision 与幂等键。旧 `ui_session` KV 不能承担共享状态、成员权限或日志真相。
- Session / 实体实际变化以 `kind=event`、`machine.intent=buckyos.action_log` 记录，
  区分 target、actor 与受影响成员 subject；入群、主动退出、被移除、会话标题修改有明确动作。
- 权威状态提交与待发布日志登记保持事务一致，跨 named store / mailbox / 服务的发布通过持久任务幂等恢复。
  同一 GroupEvent 或平台事件映射一次；失败、无变化和重复请求不生成新的成功日志。
- 外部平台仍是外部状态权威；平台修改确认前不能记录成功事实，回显需关联去重，旧事件不能回滚快照。
- Action Log 是历史事实，不是状态修改命令。普通 `post_send` 或入站 event 不能直接更新共享状态或群 ACL。
  日志可见范围受源状态权限约束；个人显示标题、草稿、typing 不产生共享 Action Log。

当前 GroupMgr 已把 GroupEvent 写入 group_events；它们尚未发布到 Session 消息时间线。
前端已有 mock Action renderer 和共享 / 成员编辑交互，不能将这些 UI 交互视为后端状态契约已实现。
Telegram 的 active / typing / status_line KV 已有消费方，但缺少统一成员 DID、有效期与 owner 隔离；
映射运行态时必须补齐可信来源和过期规则，不能据此推导实体在线状态。

### 5.8 owner 本地会话生命周期（2026-09-07 已实现）

实现：`owner_sessions(owner, session_id, lifecycle, registered, origin, peer_did, binding_json, title,
archived_at_ms, delete_watermark_sort_key, delete_watermark_record_id, deleted_at_ms, ...)`；
RPC `msg.create_session` / `msg.archive_session` / `msg.restore_session` / `msg.delete_session` / `msg.get_session_state`。
`list_sessions` 支持 `lifecycle` 过滤与 `order_by: activity`（按 chat / group_msg / deliver 记录的 `sort_key` 排序并给同口径游标），
摘要新增 `last_activity_ms` / `request_count` / `lifecycle` / `state`；`list_session` 与摘要都只统计删除水位之后的记录。
新普通消息提交后自动解除归档。`ui_session.*` 带 `owner` 时读写 `owner_ui_session_states`。
读取按 verify-hub 用户 token 校验：本人或 zone 托管的非用户身份（Agent）可读，写动作只允许本人；
服务 / 设备 token 与无 token 的进程内调用保持原行为。附件经 `GET /kapi/msg-center/objects/{obj_id}[/content]` 访问。

归档只改变会话在活动列表中的可见性，保留历史、逐记录阅读状态、未读计数和草稿；恢复沿用原会话与活动时间。
有效普通新消息可以解除归档，运行态与 Action Log 不解除归档。归档属于独立的持久 Session 元数据。

当前 RecipientState.ARCHIVED 会替换阅读状态，仍被 Session 查询纳入，且没有恢复到普通阅读状态的迁移。
DELETED 仅被 Session 查询过滤。它们不能代替上述会话级操作。

彻底删除的产品范围是当前 owner 的会话及本地历史引用、草稿和个人偏好；不删除其它 owner 的引用、联系人或连接。
需保留最小连接来源与删除水位，阻止旧消息重放复活；水位后的有效新消息按连接规则形成新的可见历史。
操作需有幂等结果、并发新消息边界、授权及重启恢复，不能用未完成的一串逐记录写入报告成功。
共享 MsgObject 及附件仍可能被其它 mailbox/delivery 引用，对象物理回收另按引用规则处理，不承诺擦除所有副本。

### 5.9 请求处理与 UI 同步边界

- REQUEST_BOX 已存在，UI 需保留每条记录的 box_kind 并提供请求入口与准入操作。
  同一 Session 可有请求与普通记录，不能从最后一条记录推导全部请求状态；筛选视图不重复增加未读。
- 联系人临时授权 / 拉黑改变当前 owner 的准入规则，查看消息不改变该规则。
  当前授权变化不迁移旧 REQUEST_BOX 记录；接受后的历史处理、处理状态及计数 / 分页需补显式契约。
- mailbox changed 在提交后发布，delivery changed 按 transport/executor 组织；事件可丢失，只作刷新信号。
  UI 需有权限的 owner/session 事件投影或受控轮询，通过 Session API 对账，不直接消费执行器队列。
- 历史 reader 需支持记录更新、删除、重新归类和断线补拉。仅按时间游标追加新消息无法发现旧记录变化，
  必须定向重读受影响记录 / 页面或提供变更游标；旧、新 Session 的摘要与未读均需刷新。
- Telegram 附件已经通过 obj_id 与 cyfs:// hint 引用 FileObject。UI 应解析有权限的对象访问地址，
  不依赖 HTTP URL 扩展名识别附件；无 hint、非图片及单附件失败均保留可读信息。

具体 UI 投影与验收见 [UI DataModel §3.5、§4.5–§4.6、§6、§9.4](../../src/frame/desktop/src/app/messagehub/UI_DATAMODEL.md)。

---

## 6. 存储与索引

### 6.1 总体模式

延续"**不可变对象 + RDB reference/index**"：

- `MsgObject` 存 named store（内容寻址，全系统一份）。**消息对象只存一份**，任意多个 `MailboxRecord` / `DeliveryRecord` 引用同一个 `msg_id`。
- `MailboxRecord`、`DeliveryRecord` 与全部索引存本地 RDB（当前为 SQLite）。
- SessionRuntimeState（typing 等）按易失语义保存；当前实现借用 `ui_session_states` RDB KV 不改变其易失语义。
  持久共享 / 成员状态与待发布 Action Log 另按 §5.7 的契约保存。

### 6.2 索引

```text
# mailbox 消费与列表（queue 式取件 + 分页）
(owner, box_kind, state, sort_key)          → record_id
(owner, box_kind, sort_key)                 → record_id

# Session 索引（list_session/list_sessions 的支撑）
(owner, session_id, sort_key, record_id)
(owner, updated_at)                          → session_id   # 会话列表排序

# Delivery 索引
(transport_did, delivery_state, next_retry_at)   # executor 取件与重试调度
(msg_id, target_did)                             # 唯一约束 + 出站聚合查询
```

`(msg_id, target_did)`（配合 `transport_did` 构成幂等键）上有唯一约束：重复 `post_send`、重复扫描、崩溃重放都收敛到同一条 `DeliveryRecord`。

### 6.3 事务边界

- `MsgObject` 写 named store 在 RDB 事务**之前**完成（幂等，重复写无害）。
- 一次 `dispatch`/`post_send` 的全部 record + 索引写入放在**单个本地 RDB 事务**里：不存在"SENT 写了、DeliveryRecord 没写"的中间态。
- 变更通知在事务提交**之后**发出，失败不回滚事务（通知是加速信号）。

### 6.3.1 `msg_idempotency`

`msg_idempotency` 是 `dispatch` / `post_send` 的持久幂等边界。主键是
`(scope, owner_scope, idempotency_key)`，其中 `scope` 取 `dispatch` 或
`post_send`。`owner_scope` 对 dispatch 使用稳定的接收方 owner（tunnel 入站为
`contact_mgr_owner`），对 post_send 使用消息 author；不同 owner 可以安全复用
同一个调用方幂等键。
每条记录包含：

- `state`：`pending` 或 `completed`。
- `msg_id`：已确定的内容寻址消息 id。
- `result_json`：`completed` 记录保存对应 RPC result 的 JSON；`pending` 记录为空。
- `retention_key`：清理分桶。外部入站按 platform + tunnel/bot account +
  chat/topic 会话分桶，不得使用单条消息的发送者账号；本地出站按发送者和 topic
  分桶。
- `expires_at_ms`：清理候选时间。

服务在同一个本地 RDB 事务内完成：占用幂等键为 `pending`，写入全部
mailbox / delivery 副作用，再把同一行更新为 `completed` 并写入
`result_json`。重复请求命中 `completed` 时直接返回 `result_json`，不重放
副作用；命中未过期 `pending` 时必须稍后重试，不能再次执行。

`expires_at_ms` 只作为物理清理依据，不作为幂等命中依据：只要 DB 中仍存在
`completed` 记录，重复请求就必须返回已保存的 `result_json`，不能因为
`expires_at_ms` 已过而重放副作用。DB 是唯一幂等命中源，不再维护内存
幂等缓存。30 天是最短保留窗口，不是到点立即失效的逻辑 TTL。

物理删除采用分桶和全局两层容量水位。单个 `retention_key` 超过 3,000 行时，
按 `expires_at_ms` 从旧到新删除该 bucket 内已经过期的记录，并尽量降到 2,000
行；未过期记录不得删除，其它 bucket 不受分桶清理影响。全表超过 100,000 行
时启动全局清理，按相同顺序删除任意 bucket 的过期记录，逐轮降到 80,000 行。
全局清理每批最多删除 10,000 行，避免服务启动或长期停机后在一个事务中清空
大量记录。独立后台任务在服务启动时和之后每小时枚举并清理一次超限 bucket，
再执行全局清理；全局清理每批让出执行权，直到降到目标水位或某批没有删满。
清理不进入 dispatch/post_send 请求路径。30 天内的记录不受任一容量水位影响。

`result_json` 使用 schema version 8 下 RPC result 的 serde JSON 表示，不再
额外嵌套 envelope：

- `scope=dispatch`：对象字段为 `ok: bool`、`msg_id: ObjId string`，以及可选的
  `delivered_recipients: DID[]`、`dropped_recipients: DID[]`、
  `delivered_group: DID`、`delivered_agents: DID[]`、`reason: string`；空数组和
  `None` 字段序列化时省略。
- `scope=post_send`：对象字段为 `ok: bool`、`msg_id: ObjId string`、可选的
  `deliveries` 和 `reason`。每个 delivery 包含 `delivery_id`、`transport_did`、
  `target_did`、`transport`；`transport` 为 `{"kind":"native"}`，或
  `{"kind":"tunnel","platform":string,"tunnel_instance_id":string}`。

`scope`、`owner_scope`、主键语义、`state` 状态机及上述结果字段在 version 8 内冻结；未来增加
或改变结果字段必须提升 MessageCenter RDB schema version。当前共享 schema
version 为 8，存放在 scheduler 下发的 msg-center RDB instance spec 中。
beta2.2 尚处于 breaking-change 阶段，version 7 到 version 8 采用 no-compat
策略：旧实例数据必须重建，不提供表内迁移。

主要查询及索引：

- `(scope, owner_scope, idempotency_key)` 精确命中由主键支持。
- 超限 bucket 枚举和 bucket 行数统计由
  `idx_msg_idempotency_retention_expire(retention_key, expires_at_ms)` 支持。
- bucket 内按过期时间选择删除候选由同一索引支持；每小时的超限 bucket 枚举
  会扫描该索引，是受 sweep 间隔控制的维护成本，不进入每次读取路径。
- 全局清理的过期候选由 `idx_msg_idempotency_expire(expires_at_ms)` 支持，并受
  全局水位和单批 10,000 行上限约束；连续批次由后台清理任务驱动。

### 6.4 崩溃恢复

1. **DELIVERY_QUEUE 扫描**：启动时与定时 sweep 扫 `WAIT`（到期重试）与 `SENDING`（租约超时收回 → `WAIT`，标注 duplicate risk）。实时通知丢失不影响最终投递。
2. **入站幂等键持久化**：外部幂等键（`{platform}:{account}:{chat}:{external_message_id}`）落 RDB 带 TTL，重启后重复上报仍能去重。
3. **通知补偿**：所有订阅方（Agent pump、UI、executor）必须周期性扫描自己的 mailbox/queue，kevent/通知只是加速。

### 6.5 索引重建

- Session 索引、box 索引：可从 `MailboxRecord` 全表重建（投影）。
- `MailboxRecord` / `DeliveryRecord`：**权威状态**，不可从对象重建（阅读状态、投递结果只存在于 record），必须纳入备份。
- `MsgObject`：named store 自身的备份策略负责。

---

## 7. 对外 API 汇总

```text
# 写入
dispatch(msg_obj, ingress_meta, idempotency_key)   # 入站（tunnel/hub/系统）
post_send(msg_obj, idempotency_key)                # 出站（user/agent/系统）
report_delivery(delivery_id, result)               # executor 回报投递结果

# Session（UI/Agent 的唯一读取面）
list_sessions(owner, cursor, limit)
list_session(owner, session_id, cursor, limit, with_object)
update_record_state(owner, record_id, recipient_state)   # 已读/归档/删除
set_session_state(owner, session_id, key, value)         # 此处指易失 SessionRuntimeState，不是共享状态修改

# 队列（仅 executor / agent pump 使用，UI 不可见）
get_next(owner, box_or_queue, state_filter, lock_on_take)
```

群聊 read receipt（`MsgReceiptObj`，per-reader）见 Self-Host-Group 文档；其存储同样遵守"独立对象 + 索引"模式。

---

## 8. 现状对照（冻结名 → 当前代码）

P3 实现迁移已于 2026-07-14 完成：代码与本文档使用同一套命名，无旧名兼容层
（beta2.2 breaking change）。落点对照：

| 冻结概念 | 代码落点 |
|---|---|
| `MailboxRecord` / `MailboxKind`（INBOX/SENT/GROUP_INBOX/REQUEST_BOX） | `buckyos_api::msg_center_client`，存储表 `mailbox_records` |
| `DeliveryRecord` / `DeliveryEnvelope` / `DeliverySnapshot` | 同上，存储表 `delivery_records`（`DELIVERY_QUEUE`） |
| `RecipientState` / `DeliveryState` 两个状态机 | 两个独立枚举（`MsgState` 已删除；`READED` → `READ`） |
| `transport_did` vs `tunnel_instance_id` | `DeliveryEnvelope.transport_did`；registry key 为 `tunnel_instance_id` |
| `DeliveryExecutor` 接口 | `frame/msg_center/src/msg_tunnel.rs`；MessageHub 与 TgTunnel 共同实现 |
| MessageHub（原生投递） | `frame/msg_center/src/message_hub.rs`（本 Zone 目标本地 dispatch；跨 Zone hop 待实现，失败明确 DEAD） |
| `list_session/list_sessions` | `msg.list_session` / `msg.list_sessions` RPC + Session 索引；Desktop 集成代码已保留，MessageHub 页面当前使用 Mock DataModel，待专门集成测试通过后启用 |
| （已删除）`thread.tunnel_id` | 已从 `ndn_lib::TopicThread` 移除（cyfs-ndn beta2.2） |
