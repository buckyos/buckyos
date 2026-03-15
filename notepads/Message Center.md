
# MessageCenter 设计文档

## 1. 背景与目标

MessageCenter 是全系统的“消息域”核心服务，统一承接：

* **tunnel 入站**：Telegram/Slack/HTTP 等将外部事件转换为 MsgObject 后 `dispatch()` 写入系统。
* **Agent/人类/系统出站**：通过 `post_send()` 将待发送 MsgObject 进入发送流水，并由 tunnel 消费发送。
* **Box 抽象**：`get_inbox(did)` / `get_outbox(did|tunnel)` 提供统一队列/列表语义。
* **群聊 read receipt**：群聊消息的“已读/阅读中”状态跟随 *reader(agent)* 而不是跟随消息本体。

### 设计目标（必须满足）

1. **幂等**：同一 MsgObject（同一 MsgObjectId）重复 dispatch/post_send 不会造成重复的 inbox/outbox 记录（除非业务刻意允许）。
2. **可扩展**：可水平扩展（多 tunnel、多 agent、多 people)
3. **可观察**：提供事件/索引能力让 UI/TaskMgr/审计能“看见发生了什么”。
4. **存储模型正确**：MsgObject 不可变；状态、删除策略、重试等是“named_store”的职责。


---

## 2. 核心概念与数据模型

### 2.1 MsgObject（已完成）

定义参考ndn_lib::MsgObject

### 2.2 MsgRecord（关键：解决“消息副本/状态/删除策略”）

**MsgRecord 是“某个 owner 在某个 box 里对某条 MsgObject 的视图”**：

* 同一条 MsgObject 可以对应多个 MsgRecord：

  * sender 的 outbox 记录（历史/删除策略独立）
  * receiver 的 inbox 记录（已读状态独立）
  * group chat 中每个 reader 的 read receipt / 或每 reader 的 record

建议定义（伪结构）：

```rust
// 逻辑模型：可存 KV/对象存储/关系表
pub struct MsgRecord {
    pub record_id: String,      // 唯一，建议可推导：hash(owner + box + msg_id + variant)
    pub owner: DID,             // 这个记录属于谁（agent did / user did / tunnel did / group did）
    pub box_kind: BoxKind,      // INBOX / OUTBOX / GROUP_INBOX / TUNNEL_OUTBOX / ...
    pub msg_id: ObjId,    // 指向不可变 MsgObject
    pub state: MsgState,        // 记录状态（inbox/outbox 不同状态机）
    pub created_at_ms: u64,
    pub updated_at_ms: u64,


    pub route: Option<RouteInfo>,        // tunnel / platform / chat_id / ext ids
    pub delivery: Option<DeliveryInfo>,  // 重试次数、下一次重试时间、错误码等

    // --- 可选索引字段 ---
    pub thread_key: Option<String>,      // UI 聚合用（可从 msg.thread 派生缓存）
    pub sort_key: u64,                   // 排序（通常 = msg.created_at_ms 或 record.created_at_ms）
    pub tags: Vec<String>,               // archive/starred 等
}

pub enum BoxKind { INBOX, OUTBOX, GROUP_INBOX }

pub enum MsgState {
    // INBOX
    UNREAD, READING, READED,
    // OUTBOX
    WAIT, SENDING, SENT, FAILED, DEAD,
    // 通用
    DELETED, ARCHIVED,
}
```

> 为什么 route 放 MsgRecord 而不是 MsgObject：
> 因为 route 往往是“投递实现细节”，会随 tunnel、账号、重试而变化；而 MsgObjectId 要保持“语义稳定”。现有规则并不禁止把 route 放 meta，但把投递细节沉到 record 层通常更省返工。

---

### 2.3 ReadReceipt（群聊必备）

完整实现参考ndn-lib::MsgReceiptObj

群聊里消息“已读”跟随 reader（agent/user）走，因此 ReadReceipt 应该是独立对象（NamedObject）：

```rust
pub struct MsgReceiptObj {
    pub msg_id: ObjId,
    pub iss:DID,
    pub reader:DID,
    pub group_id:Option<DID>,
    pub at_ms: u64,
    pub status: ReceiptStatus,
    #[serde(skip_serializing_if = "Option::is_none", default)]
    pub reason: Option<String>,
}
pub enum ReceiptStatus {
    Accepted,
    Rejected,
    Quarantined,
}
```

索引建议：

* `rr/{group}/{reader}/{msg_id} -> ReadReceiptId`
* 或 `rr/{msg_id}/{reader} -> ReadReceiptId`

---

## 3. 存储与索引（对象存储 + 轻索引）

底层基于对象存储，通过 MsgObjectId 可以全系统得到 MsgObject。基于这个前提，MessageCenter 需要额外维护：


1. `records/{record_id} -> MsgRecord`
2. `box_index/{owner}/{box_kind}/... -> record_id 列表或有序集合`

### 3.1 BoxIndex 的推荐 key 设计（便于分页/队列）

一个可直接实现的 KV key：

* `box/{owner}/{box_kind}/time/{sort_key}/{record_id} = 1`
* `box/{owner}/{box_kind}/state/{state}/{sort_key}/{record_id} = 1`（可选，加速“拉未读/待发送”）

这样能支持：

* “按时间分页”
* “按状态取队列”（比如 tunnel 取 WAIT）

---

## 4. MessageCenter 对外 API 设计

### 4.1 核心写入 API

#### `dispatch(msg_obj, ingress_ctx)`

用于**入站**：tunnel / 系统内部把消息投递到对应 inbox（或 group inbox）。

* 幂等：同 msg_id 重复 dispatch 不应重复生成 record（或需要按业务允许生成“重复记录”时，用不同 record_id variant）。

#### `post_send(msg_obj, send_ctx)`

用于**出站**：agent/人/系统产生消息，进入 outbox，并创建投递计划（由 tunnel 消费）。

> 在参考循环里已经有 dispatch/post_send 的最小版。
> 下面是“可生产落地版”的职责补全。

---

### 4.2 Box 访问 API

#### `get_inbox(owner_did) -> MsgBoxHandle`

* `get_next()`：取下一条未读（或按策略）
* `peek(n)`：预览
* `update_state(record_id, state)`：更新阅读状态
* `list_by_time(...)`：分页拉取历史

#### `get_outbox(owner_did) -> MsgBoxHandle`

* `put(msg_id)`：创建 outbox 记录（通常由 post_send 做）
* `list(...)`：UI 看历史

#### `get_tunnel_outbox(tunnel_did) -> MsgBoxHandle`

* tunnel 的 send_thread 从这里 `get_next()` 拉 WAIT/SENDING 进行发送。

---

## 5. 核心流程与伪代码



### 5.1 dispatch：入站投递（含群聊语义）

```python
def message_center.dispatch(msg_obj, ingress_ctx=None):
    """
    入站入口：tunnel/system -> MessageCenter

    msg_obj 约定：
      - 1:1：msg_obj.from = author DID, msg_obj.source = None
      - 群聊：msg_obj.from = group DID, msg_obj.source = author DID  
    """

    msg_id = msg_obj.id  # canonical_json_hash(MsgObject)
    # 1) 持久化 MsgObject（幂等：已存在则跳过）
    object_store.put_if_absent(f"objects/msg/{msg_id}", msg_obj)

    # 2) 计算“逻辑 sender”（用于 block/策略）
    sender = _logical_sender(msg_obj)  # 群聊取 source，否则取 from
    if contact_mgr.is_block(sender, context=ingress_ctx):
        return {"ok": False, "reason": "blocked"}

    # 3) 决定投递目标（单聊 inbox / 群聊 inbox）
    if _is_group_chat(msg_obj):
        group_id = msg_obj.from
        # 3.1 写入 group inbox（用于 UI / 归档 / 群维度历史）
        _put_record(
            owner=group_id,
            box_kind="GROUP_INBOX",
            msg_id=msg_id,
            route=_route_from_ingress(ingress_ctx, msg_obj),
            initial_state="UNREAD",  # 群 inbox 的 UNREAD 只是“群有新消息”，不代表某 agent 未读
        )

        # 3.2 为群内订阅的 agent 创建“未读视图”或 read receipt 初始化
        #     这里不要把 MsgObject 复制，只创建 per-agent 的“阅读状态对象/记录”
        readers = contact_mgr.get_group_subscribers(group_id)  # 哪些 agent 需要收到这个群的消息
        for agent_did in readers:
            # 可选A：建立 inbox 记录（每 agent 的“群消息收件箱”）
            _put_record(
                owner=agent_did,
                box_kind="INBOX",
                msg_id=msg_id,
                route=_route_from_ingress(ingress_ctx, msg_obj),
                initial_state="UNREAD",
                tags=["group:"+str(group_id)]
            )

            # 可选B：只初始化 ReadReceipt（更纯粹的群聊实现）
            # rr = ReadReceipt(msg_id=msg_id, reader=agent_did, group=group_id, state="UNREAD"?)
            # object_store.put_if_absent(rr.key, rr)

        _notify(group_id, msg_id)
        for agent_did in readers:
            _notify(agent_did, msg_id)

        return {"ok": True, "delivered": {"group": group_id, "agents": readers}}

    else:
        # 1:1 或多播：按 to 列表投递 inbox
        recipients = msg_obj.to or []  # 当前 MsgObject 有 to: Vec<DID>
        delivered = []
        for to_did in recipients:
            if contact_mgr.is_block(sender, target=to_did, context=ingress_ctx):
                continue
            _put_record(
                owner=to_did,
                box_kind="INBOX",
                msg_id=msg_id,
                route=_route_from_ingress(ingress_ctx, msg_obj),
                initial_state="UNREAD",
            )
            delivered.append(to_did)

        _notify_many(delivered, msg_id)
        return {"ok": True, "delivered": delivered}


def _logical_sender(msg_obj):
    # 群聊：source 是作者；否则 from 是作者
    return msg_obj.source if msg_obj.source is not None else msg_obj.from

def _is_group_chat(msg_obj):
    # 最简单判断：source 存在且 from 是 group DID
    return msg_obj.source is not None and contact_mgr.is_group_did(msg_obj.from)
```


---

### 5.2 post_send：出站排队（写 sender outbox + 写 tunnel outbox）

设计讨论里提到：tunnel 的 send_thread 从 `msg_center.get_outbox(self.tunnel_id)` 拉取发送。
因此建议把出站拆成两层：

* **Owner OUTBOX**：给 UI/历史/删除策略使用
* **Tunnel OUTBOX**：给投递使用（按 tunnel 分队列）

```python
def message_center.post_send(msg_obj, send_ctx=None):
    """
    出站入口：agent/user/system -> MessageCenter

    目标：
      1) 存 MsgObject（幂等）
      2) 写 sender OUTBOX record（用于历史/删除策略）
      3) 通过 ContactMgr 选路由（选 tunnel endpoint）
      4) 写入一个或多个 tunnel outbox record（用于实际发送）
    """
    msg_id = msg_obj.id
    object_store.put_if_absent(f"objects/msg/{msg_id}", msg_obj)

    author = _logical_sender_for_outbound(msg_obj)  # 群聊 msg.source 是作者；非群聊 msg.from 是作者
    if contact_mgr.is_block(author, context=send_ctx):
        return {"ok": False, "reason": "blocked_author"}

    # 1) 写 author 的 OUTBOX 记录（历史副本）
    _put_record(
        owner=author,
        box_kind="OUTBOX",
        msg_id=msg_id,
        route=None,              # 历史记录不一定绑定某个 tunnel
        initial_state="SENT",    # 注意：这里的 SENT 表示“已产生”，不是“外部平台已投递成功”
    )

    # 2) 选路由：对每个“投递目标”得到 endpoint 列表
    delivery_plans = contact_mgr.plan_delivery(msg_obj, context=send_ctx)
    # delivery_plans: List[ {tunnel_did, address, target_did, mode, priority} ]

    # 3) 将投递计划写入 tunnel outbox 队列（真正的 WAIT -> SENDING -> SENT）
    created = []
    for plan in delivery_plans:
        tunnel_id = plan["tunnel_did"]
        record_id = _put_record(
            owner=tunnel_id,
            box_kind="TUNNEL_OUTBOX",
            msg_id=msg_id,
            route=plan,               # 每条投递记录绑定 route（tunnel+address）
            initial_state="WAIT",
        )
        created.append({"tunnel": tunnel_id, "record_id": record_id})

    _notify_many([p["tunnel_did"] for p in delivery_plans], msg_id)
    return {"ok": True, "msg_id": msg_id, "deliveries": created}


def _logical_sender_for_outbound(msg_obj):
    # 已确认：群聊 from=group, source=author；因此出站作者取 source
    return msg_obj.source if msg_obj.source is not None else msg_obj.from
```

---

### 5.3 MsgBox：队列语义（给 agent / tunnel）

下面给一个 **“取 next + 置状态”** 的实现骨架（保证并发安全、避免多消费者抢同一条）。
（不使用系统内置的MsgQueue服务，是因为Message Center里的MsgObject的备份要求更高)

```python
class MsgBoxHandle:
    def __init__(self, owner_did, box_kind):
        self.owner = owner_did
        self.box_kind = box_kind

    def get_next(self, state_filter=None):
        """
        从 box_index 按时间/优先级取一条符合状态的 record
        并用 CAS 抢占（避免多线程/多实例重复消费）
        """
        candidates = box_index.scan(owner=self.owner, box=self.box_kind, state=state_filter, limit=10)
        for record_id in candidates:
            rec = record_store.get(record_id)
            if state_filter and rec.state not in state_filter:
                continue

            # 并发抢占：WAIT->SENDING 或 UNREAD->READING
            new_state = _next_state_on_take(rec.state, self.box_kind)
            if new_state is None:
                continue

            ok = record_store.cas_update_state(record_id, expected=rec.state, new=new_state)
            if ok:
                msg_obj = object_store.get(f"objects/msg/{rec.msg_id}")
                return (record_id, rec, msg_obj)

        return None

    def update_state(self, record_id, new_state):
        rec = record_store.get(record_id)
        if not _state_transition_allowed(rec.state, new_state, self.box_kind):
            raise Exception(f"invalid transition: {rec.state}->{new_state}")
        record_store.update_state(record_id, new_state)
        box_index.update_state(self.owner, self.box_kind, record_id, new_state)

def _next_state_on_take(state, box_kind):
    if box_kind == "INBOX" and state == "UNREAD":
        return "READING"
    if box_kind == "TUNNEL_OUTBOX" and state == "WAIT":
        return "SENDING"
    return None
```

> 这与您参考循环的语义一致：agent/tunnel 取到后会将状态置 READING 或 SENDING。

---

### 5.4 Tunnel 发送回执落地（建议 MessageCenter 提供一个统一接口）

参考实现里 tunnel 发送完只做 `update_msg_state(msg_obj.id, SENDED)`。
生产系统通常还需要把外部 message_id/时间戳/错误码写入 record.delivery 或 record.route，避免重试导致重复发。

建议统一接口：

```python
def message_center.report_delivery(record_id, result):
    """
    tunnel 在 send_message 后回调：
      - success: external_msg_id / delivered_at_ms
      - failure: error_code / retry_after_ms
    """
    rec = record_store.get(record_id)
    assert rec.box_kind == "TUNNEL_OUTBOX"

    if result["ok"]:
        rec.delivery = {
            "external_msg_id": result.get("external_msg_id"),
            "delivered_at_ms": now_ms(),
            "attempts": rec.delivery.get("attempts", 0) + 1 if rec.delivery else 1,
        }
        record_store.update(rec)
        record_store.update_state(record_id, "SENT")
        box_index.update_state(rec.owner, rec.box_kind, record_id, "SENT")
    else:
        # 可重试/不可重试分类
        attempts = rec.delivery.get("attempts", 0) + 1 if rec.delivery else 1
        rec.delivery = {"attempts": attempts, "last_error": result["error"]}
        record_store.update(rec)

        if attempts >= MAX_RETRY:
            record_store.update_state(record_id, "DEAD")
        else:
            # 回到 WAIT，供下次 send_thread 再取
            record_store.update_state(record_id, "WAIT")
        box_index.update_state(rec.owner, rec.box_kind, record_id, record_store.get(record_id).state)
```

---

### 5.5 群聊 ReadReceipt 的写入点

在 Agent loop 中，文档提到群聊 reading 状态跟 agent id 走。
因此 MessageCenter 提供原子接口：

```python
def message_center.set_read_state(group_id, msg_id, reader_did, state):
    """
    群聊 read receipt：reader 维度
    """
    rr = ReadReceipt(msg_id=msg_id, reader=reader_did, group=group_id, state=state, at_ms=now_ms())
    rr_id = rr.id  # 同样可以 canonical hash
    object_store.put(f"objects/rr/{rr_id}", rr)

    rr_index.put(f"rr/{group_id}/{reader_did}/{msg_id}", rr_id)
    return rr_id
```

Agent 的处理流程示例（与文档一致，但把状态持久化）：

```python
def agent_on_group_msgs(agent_did, group_msgs):
    for msg in group_msgs:
        message_center.set_read_state(group_id=msg.from, msg_id=msg.id, reader_did=agent_did, state="READING")
        process(msg)
        message_center.set_read_state(group_id=msg.from, msg_id=msg.id, reader_did=agent_did, state="READED")
```

---
