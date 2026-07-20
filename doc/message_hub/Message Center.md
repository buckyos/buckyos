# MessageCenter 设计文档

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

**永远不属于 MsgObject 的**：已读状态、投递状态、重试信息、外部平台 message id、归档/删除标记、会话归类。这些全部落在 record 层。投递失败、重试、回执只更新 `DeliveryRecord`，`MsgObject` 与投递结果彻底解耦。

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

- `target_did` 是 **shareable DID**（如 `did:bns:bob`、`did:bns:telegram.bob`）→ **MessageHub 原生投递**：解析 DID → 找到目标 Zone → POST MsgObject。Zone 解析发生在每次投递尝试时（如同 email 发送时才查 MX），但走的是确定性解析协议，不是策略选路。
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

SessionState（易失，不落 mailbox/delivery）:
  typing / active / status_line 等 UI 会话状态，独立通道，随时可丢
```

三个状态机互不迁移、互不共享取值。"收件人已读"不影响 DeliveryState；"投递失败"不产生 RecipientState；typing 永远不产生记录。

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

**每个 `to` 生成独立的 `DeliveryRecord`，拥有独立的状态与结果。** 一个目标投递失败不污染其他目标：给 3 个人发消息，2 个 `SENT`、1 个 `FAILED` 是正常终态，UI 按目标分别展示。不存在"整条消息发送失败"这个聚合状态——只有创建期（阶段一）的整体失败和投递期的按目标结果。

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

---

## 5. Session Projection

### 5.1 Session 不是 MsgBox

Session 是**投影**：为 UI/Agent 把分散在多个 mailbox 和 delivery queue 里的记录聚合成"一个会话"的只读视图。它不持有独立真相：

- 删除全部 session 索引，不丢任何消息，可从 MailboxRecord 全量重建。
- 写操作（标已读、归档、删除）落到对应的 `MailboxRecord` 上，session 视图随之变化。
- Session 里"这条消息发送中/失败"的角标来自 DeliveryRecord 聚合，UI 不直接读 DELIVERY_QUEUE。

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
list_sessions(owner, cursor, limit)
  -> [ { session_id, peer/group 摘要, last_msg 摘要, unread_count, updated_at } ]

list_session(owner, session_id, cursor, limit, with_object)
  -> [ { record_id, msg_id, direction, sort_key,
         recipient_state?,                  # 入站记录
         delivery: {                        # 出站记录：聚合视图
            overall: sending | delivered | partial_failed | failed,
            per_target: [ { target_did, state, attempts, last_error? } ]
         },
         msg?                               # with_object=true 时附带 MsgObject
       } ]
```

聚合规则：全部 target `SENT` → `delivered`；存在 `WAIT/SENDING` → `sending`；部分 `DEAD/FAILED` → `partial_failed`；全部 `DEAD` → `failed`。

**一个传统私聊 UI 只需要调用一次 `list_session()`** 就能渲染完整会话（双向消息 + 已读状态 + 投递角标），不需要分别读取 Inbox、Sent、Delivery Queue——后两者对 UI 根本不可见。

### 5.4 `thread.topic` 与 `session_id`

| | `MsgObject.thread.topic` | `MailboxRecord.session_id` |
|---|---|---|
| 归属层 | 消息本体（不可变） | 本地记录（每 owner 独立） |
| 语义 | 发送方携带的**语义 hint** | Personal Server 的**本地投影 key** |
| 由谁定 | 消息作者 | 收/发方本地的 MessageCenter |
| 可变性 | 永不可变 | 可由可信后端/Agent 重新归类 |

MessageCenter 可以建立 `thread.topic → session_id` 的映射（同 topic 默认聚为同一会话），但**不能修改 MsgObject**。同一条消息在不同 owner 的视图中可以有不同 `session_id`。

---

## 6. 存储与索引

### 6.1 总体模式

延续"**不可变对象 + RDB reference/index**"：

- `MsgObject` 存 named store（内容寻址，全系统一份）。**消息对象只存一份**，任意多个 `MailboxRecord` / `DeliveryRecord` 引用同一个 `msg_id`。
- `MailboxRecord`、`DeliveryRecord` 与全部索引存本地 RDB（当前为 SQLite）。
- SessionState（typing 等）存内存/易失存储，不进 RDB。

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
set_session_state(owner, session_id, key, value)         # 易失 SessionState

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
