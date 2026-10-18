# My Network 相关协议设计

- 2026-10-17：§5–§7、§9、§12、§13 按已实现的 msg-center 联系人模块改写：好友消息 intent 改为 `buckyos.friend`（取代 `bucky.social.friend.*`），`friend_state` 与 `access_level` 分离，分组统一为 tag，跨 Zone 走原生投递的自动路由与签名入站。当前行为以代码为准。

本文记录 My Network、MessageHub、ContactMgr、DID / binded Zone 在好友请求、联系人关系、第三方社交账号导入上的协议设计。

核心结论是：**不新增独立通讯协议**。My Network 相关协议应建立在现有 DID 解析、User binded Zone、MsgObject / SendMessage、MessageHub 和密码学签名设施之上。所谓“好友协议”本质上是一组特殊 Message Object 的 schema，以及各 Zone 本地 Social Graph 状态机的处理约定。

---

## 1. 基本结论

### 1.1 不设计新的好友通讯通道

好友请求不需要新的传输层协议。

模型是：

```text
A -> resolve B DID（名字服务）
  -> B 的 binded Zone
  -> 现有 post_send / MessageHub 原生投递
  -> 一条 kind = Operation、machine.intent = "buckyos.friend" 的 MsgObject
```

也就是说：

```text
buckyos.friend  action = request | accept | reject | cancel | remove
```

都只是特殊消息类型（§5）。

这些消息是否能送达、是否会进入 RequestBox、是否被 My Network 处理，最终取决于目标用户 Zone 的 msg-center 准入策略（§6.3）。

### 1.2 好友关系是本地状态，不是强一致全局状态

去中心化系统里，好友关系不应被设计成全局强一致状态。

例如：

```text
1. A 给 B 发好友请求。
2. B 可以接受、拒绝，也可以不处理。
3. B 接受后，A 本地显示为好友（friend_state = friend）。
4. B 之后仍然可以直接拒收 A 的消息，并且不通知 A。
5. A 的本地状态可能在一段时间内仍然显示 B 是好友。
```

因此协议只定义消息语义和本地状态机，不承诺双方状态永远一致。

### 1.3 好友关系不写入合约

好友关系不应进入链上合约。

链上或 BNS 层只负责：

```text
Owner DID -> DID Document / OwnerConfig / binded Zone / service endpoint
```

My Network 的关系数据应保存在用户自己的 Zone 中：

```text
Social Graph Store
Contact Store
Source Relation Store
Request Store
Block List
Collection Store
```

当前实现：联系人、好友状态、请求记录、屏蔽与分组都在 msg-center 的 ContactMgr（按 owner 分库，见 [Contact Mgr.md](<./Contact Mgr.md>)）；Source Relation Store 未实现。

这样可以保证：

- 关系图默认私有；
- 用户迁移 Zone 时可以导出 / 导入；
- 不把个人社交关系永久公开；
- 不把高频关系变更变成链上成本。

### 1.4 关系证明不属于 My Network 社交协议

“A 向 C 证明自己和 B 是好友”属于通用隐私证明 / 行为证明领域，不属于好友请求协议。

该类能力应与以下能力归为一类：

```text
安装证明
授权证明
身份绑定证明
行为证明
关系披露证明
信用背书
```

My Network 协议只维护 A/B 之间的关系状态。第三方可验证的关系披露必须另行设计，并应具备：

- 显式签名；
- 明确 audience；
- 明确 purpose；
- 很短有效期；
- nonce；
- 不可转授权；
- 与“信用背书”严格区分。

---

## 2. 目标与非目标

### 2.1 目标

本协议要解决：

1. DID 用户之间如何发送好友请求。
2. 接收方如何接受、拒绝或忽略请求。
3. My Network 如何维护本地 Social Graph。
4. MessageHub 如何根据关系状态进行消息分流。
5. 第三方社交账号、通讯录、导入联系人如何进入统一关系图。
6. 如何区分 Contact、Friend、Source Relation、Collection。

### 2.2 非目标

本协议不解决：

1. 新的消息传输协议。
2. 强一致的全球好友状态。
3. 公开好友列表。
4. 第三方可验证好友证明。
5. 信用背书。
6. 链上社交关系合约。
7. 第三方平台关系的反向管理协议。

---

## 3. 基于 User DID + Binded Zone 的模型

### 3.1 身份层

BuckyOS 中用户的长期身份是 Owner DID / User DID。

```text
User DID: did:bns:alice
Binded Zone: did:web:alice-zone.example
```

User DID 是社交关系的主身份。Zone 是当前托管这个用户数据与服务的运行位置。

因此好友关系应绑定到 User DID，而不是某个临时 Zone URL。

### 3.2 发现流程

发送社交消息时的流程（msg-center 实现）：

```text
Alice 输入 Bob 的 DID / BNS 名 / 域名
  -> contact.lookup：name_client 解析 Bob 的 DID Document（先按 Owner 文档）
  -> binded_zone_list 第一项 = Bob 的 Zone
  -> 原生投递到 cyfs://<zone>/kapi/msg-center/<bob did>/inbox（upstream https://<zone>）
  -> 好友消息 MsgObject
```

这个流程复用 name_client、Zone 网关和 msg-center 原生投递，见 [Message Center.md](<./Message Center.md>) §2.2、§4.5。

### 3.3 Zone 的职责

每个用户当前 binded Zone 负责：

- 接收发给该 User DID 的消息；
- 判断是否允许消息进入 Inbox / RequestBox / Drop；
- 识别 My Network 相关 MsgObject；
- 更新本地 Social Graph；
- 向对方发送 accept / reject / notice 等回复；
- 持久化联系人、请求、分组和来源关系。

---

## 4. 三层关系模型

My Network 不应只有一个 `friend` 字段。推荐拆成三层。

### 4.1 Identity Binding

Identity Binding 表达“这个联系人有哪些身份或账号”。

示例：

```text
canonical_did: did:bns:bob
bindings:
  - telegram:user:12345
  - facebook:user:9988
  - email_hash:...
  - phone_hash:...
```

Binding 不等于好友关系。它只是身份聚合、展示与反查（endpoint → 联系人）的基础；按冻结后的消息域设计，binding 不参与出站选路，发送目标始终是调用方显式给出的确定 DID。

### 4.2 Source Relation

Source Relation 表达“某个来源系统给出的关系边”。

例如：

```text
Alice --facebook_friend--> Bob
Alice --phone_contact--> Bob
Alice --telegram_contact--> Bob
Alice --csv_imported_contact--> Bob
```

这些关系来自外部系统或导入数据，不自动等价于 BuckyOS DID 好友。

建议字段：

```text
relation_id
owner_did
target_ref
source_type          # facebook / telegram / phonebook / csv / manual / agent
source_account       # alice 的第三方账号或导入批次
relation_type        # friend / contact / follower / member / unknown
managed_by           # buckyos / facebook / telegram / local_import
confidence
sync_status
created_at
updated_at
raw_ref
```

未实现。

### 4.3 DID Relation

DID Relation 表达 BuckyOS 原生关系状态，实现为联系人上的 `friend_state`：

```text
none
outgoing_pending
incoming_pending
friend
```

- “只是联系人”是有联系人记录且 `friend_state = none`；屏蔽是 `access_level = Block`（§7.1），不是 `friend_state` 的取值。
- rejected / cancelled / ignored / expired 是请求记录的结局（§5.4），不留在联系人上：请求结束后联系人回到 `none`。
- 单向关注不是 `friend_state`，用应用 tag 表达，如 HomeStation 的 `homestation.following`（§7.4）。

这个状态是本地的。

同一时刻，A 本地和 B 本地可以不一致。

---

## 5. 好友消息（`buckyos.friend`）

常量与类型在 `buckyos-api` 的 `msg_contact.rs`；websdk `msg_center_client.ts` 的 Relationships 一节是对应的 TS 定义。

### 5.1 消息形态

好友消息是普通 MsgObject，由 msg-center 代 owner 构造，经普通原生投递路径送出：

```text
MsgObject {
  kind: Operation,
  from: owner DID, to: [对方 DID],
  content: {
    format: text/plain,
    content: 可读文本（附言；没有附言时为 "Friend request"，控制消息为 "Friend request accepted" 等）,
    machine: {
      intent: "buckyos.friend",
      data: { action, request_id, message?, profile_hint?: { name?, avatar? }, expires_at_ms? }
    }
  }
}
```

签名、对象 ID、from / to 遵守 MsgObject 既有规则；跨 Zone 时由发送方 Zone 托管签名（Message Center.md §4.5）。

### 5.2 action

| action | 方向 | data | 接收方处理 |
| --- | --- | --- | --- |
| `request` | A → B | `request_id`、`message?`（≤ 500 字符）、`profile_hint{name, avatar}`（取 A 自己库里的本人联系人，头像超过 2048 字符不带）、`expires_at_ms`（发出时 + 7 天） | 见 §6.3 |
| `accept` | B → A | `request_id` | A 有该 `request_id` 的 pending 出向请求 → 双方成为好友，请求 `accepted`；已 `accepted` 的重复 accept、已取消或过期的请求 → 忽略 |
| `reject` | B → A | `request_id` | 只在 B 选“拒绝并告知”时发。A 的 pending 出向请求 → `rejected`，联系人回到 `none` |
| `cancel` | A → B | `request_id` | B 的 pending 入向请求 → `cancelled`，联系人回到 `none` |
| `remove` | 任一方 | `request_id` 为空 | 尽力通知：对方不是系统关系时 `friend` → `none`，`Allow` 降为 `Stranger`，联系人记录保留 |

- `request_id`（UUID）是幂等键：接收方按 `(owner, peer, request_id)` 存记录，重复投递和 Gateway 重放都只命中已有记录（§6.3）。
- `accept` / `reject` / `cancel` / `remove` 是控制消息：只改状态并发联系人变更事件，双方都不写邮箱记录。`request` 在发送方留一条 SENT 记录，在接收方按普通准入写一条邮箱记录（§6.3），会话里因此能看到请求。
- 接收方在 dispatch 写邮箱之前识别 `buckyos.friend`（本地 dispatch 与 CYFS 接收共用同一入口），位置在发送方屏蔽检查之后。B 发回的 `accept` 因此不会被当成陌生人消息进 REQUEST_BOX。

### 5.3 RPC

浏览器没有用户私钥，好友消息只由 msg-center 构造：`msg.post_send` 收到 `buckyos.friend` 消息时返回 `friend-messages-use-contact-rpc`，客户端只能用下面的 RPC。

| 方法 | 参数 | 返回 |
| --- | --- | --- |
| `contact.send_friend_request` | `did`、`message?` | `FriendRequestResult { friend_state, request? }` |
| `contact.respond_friend_request` | `request_id`、`decision: accept \| reject \| ignore` | 同上 |
| `contact.cancel_friend_request` | `request_id` | 同上 |
| `contact.remove_friend` | `did`、`notify?`（默认 true） | 同上，`request` 为空 |
| `contact.list_friend_requests` | `direction?`、`state?`、`limit?`（默认 50，≤ 200）、`offset?` | `{ items: FriendRequest[] }`，按 `updated_at_ms` 倒序 |

- 都接受 `contact_mgr_owner`，省略时为调用用户本人。前四个要求对该 owner 的写权限，且 owner 由本 Zone 托管（否则 `owner-not-hosted`）；列表要求读权限。授权细节见 Contact Mgr.md §3.1。
- `decision`：`accept` 成为好友并回 `accept`；`reject` 回 `reject`；`ignore` 不发任何消息，对方继续显示等待确认，直到撤回或过期。
- 错误码：`cannot-befriend-self`、`peer-not-befriendable`（`did:msgtunnel`）、`message-too-long`、`peer-blocked`、`friend-request-cooldown`、`not-found`、`request-not-pending`、`request-expired`、`system-relation`。

### 5.4 请求记录

两侧在各自的 msg-center 里各存一份 `FriendRequest`（表 `friend_requests`，DDL 见 [Message Center Durable Data Schema.md](<./Message Center Durable Data Schema.md>)）：

| 字段 | 含义 |
| --- | --- |
| `request_id`、`peer_did`、`direction` | `direction` 为 `incoming` / `outgoing` |
| `state` | `pending` / `accepted` / `rejected` / `cancelled` / `expired` / `ignored` |
| `message`、`msg_id` | 附言；承载请求的 MsgObject（只用于展示，`accept` 不引用它） |
| `peer_name`、`peer_avatar` | 入向取对方的 `profile_hint`，出向取本地联系人资料 |
| `delivery` | 只用于出向：`pending` / `delivered` / `failed`，附 `reason` |
| `created_at_ms`、`expires_at_ms`、`updated_at_ms` | 有效期 7 天；入向请求不超过收到时 + 7 天 |

- 出向请求的送达：本 Zone 的对方直接 dispatch，立即得到 `delivered` 或 `failed`（`rejected` 表示对方丢弃了它，§6.3）；他 Zone 的对方写一条原生 DeliveryRecord，读取时按它换算（SENT → `delivered`，DEAD / FAILED → `failed`，其余 `pending`，附最近一次错误）。UI 据此显示“等待送达”，投递完成前不能说请求已送达。
- 过期：msg-center 每 10 分钟清扫一次；`list_friend_requests` 先处理该 owner 的过期请求；`respond` / `cancel` 遇到过期请求返回 `request-expired`。过期请求置 `expired`，联系人从 pending 回到 `none`。
- 删除联系人不删除请求记录。

---

## 6. 本地状态机

### 6.1 发起方 A（A 库中 B 的 `friend_state`）

```text
none --send_friend_request--> outgoing_pending --收到 accept--> friend
outgoing_pending --cancel / 收到 reject / 过期--> none
friend --remove_friend / 收到 remove--> none
```

`send_friend_request` 依次判断：

1. 对象是自己、`did:msgtunnel`，或附言超长 → 参数错误；
2. A 屏蔽了 B → `peer-blocked`；
3. 已是好友 → 不发消息，直接返回 `friend`；
4. B 有发给 A 的 pending 请求 → 等同接受它；
5. A 已有发给 B 的 pending 请求 → 返回该请求，不重发；
6. A 上一次发给 B 的请求在 24 小时内被明确拒绝（收到过 `reject`）→ `friend-request-cooldown`；
7. 否则生成新的 `request_id`，联系人（不存在则以 `ManualCreate` 创建）置 `outgoing_pending`，存 pending 记录后发出 `request`。

### 6.2 接收方 B

```text
none --收到 request--> incoming_pending --accept--> friend
incoming_pending --reject / ignore / 收到 cancel / 过期--> none
```

`accept`：双方成为好友（§7.1）；B 若也有发给 A 的 pending 请求，一并置 `accepted`；回 `accept`。`reject`：回 `reject`。`ignore`：不回消息。

### 6.3 收到 `request` 时的处理

按顺序：

1. B 屏蔽了 A → 丢弃，不留记录（dispatch 在识别 intent 之前已按屏蔽丢弃）；
2. `request_id` 为空 → 丢弃；
3. 已有同一 `request_id` 的记录 → 幂等：仍 pending 时照常写邮箱（同一消息不会重复记录），否则当作控制消息；
4. `expires_at_ms` 已过 → 丢弃；
5. B 已把 A 当好友 → 不建记录，直接回 `accept`；
6. B 也有发给 A 的未过期 pending 请求（交叉请求）→ 双方成为好友：B 的请求与这条请求都记为 `accepted`，回 `accept`；
7. B 在 24 小时内拒绝或忽略过 A 的请求 → 丢弃（冷却）；
8. 否则 B 端联系人（不存在则以 `AutoInferred` 创建，用 `profile_hint` 补空缺的名称与头像）置 `incoming_pending`，存 pending 记录，再按普通准入写一条邮箱记录：陌生人进 REQUEST_BOX，`Allow` 或有效临时授权进 INBOX。

“丢弃”只是接收方不留记录，dispatch 仍报告该收件人被拒：本 Zone 发送方的请求显示 `failed`（`rejected`）；他 Zone 的接收方返回 403 `acl-denied`，发送方 DeliveryRecord 进入 DEAD。发送方因此能看出被屏蔽或处于冷却（已知问题）。

### 6.4 删除、屏蔽与好友关系

- `contact.remove_friend`：系统关系（§7.1）返回 `system-relation`；否则 `friend` → `none`、`Allow` → `Stranger`；原来是好友且 `notify` 为真时发 `remove`。pending 请求不受影响。
- `contact.remove_contact`：owner 由本 Zone 托管时先结清关系——结束好友并发 `remove`，撤回自己的 pending 请求（发 `cancel`），把对方的 pending 请求记为 `ignored`——再删除联系人记录与别名；历史消息与请求记录保留。系统关系拒绝删除。
- `contact.block_contact` 与 `contact.update_contact{access_level: block}`（带 `contact_mgr_owner` 且该 owner 由本 Zone 托管时）：撤回自己的 pending 请求（发 `cancel`）、忽略对方的，**不结束好友关系**，也不通知对方（§6.5）。此后对方的消息和请求都被丢弃。
- `contact.unblock_contact`：好友回到 `Allow`，否则回到 `Stranger`。

### 6.5 状态不一致是正常状态

例子：

```text
A: friend
B: friend，access_level = Block
```

这表示 A 本地仍认为 B 是好友，但 B 的 msg-center 会丢弃 A 的消息。

协议不要求 B 必须把 block 通知 A；`remove` 也只是尽力通知，对方 Zone 不可达或丢弃时双方状态就会不同。

---

## 7. 与 ContactMgr / MessageHub 的关系

### 7.1 `friend_state` 与 `access_level` 分离

Contact 上三样东西各管一件事：

| 字段 | 含义 | 谁改 |
| --- | --- | --- |
| `friend_state` | 好友关系（§4.3） | 好友请求流程；同 Zone 用户、Agent 与 owner 由 msg-center 的 Zone 同步每次置为 `friend` |
| `access_level` | 投递策略：`block` / `stranger` / `temporary` / `allow`。`allow` 原名 `Friend`，2026-10-17 改名，不兼容旧值 | 成为好友时若不是 Block 则设为 `Allow` 并清掉临时授权；解除好友时 `Allow` 降为 `Stranger`；用户也可以用 `update_contact` 单独调整（例如允许某个陌生人直接发消息），不影响 `friend_state` |
| `tags` | 分组（§7.4） | 按类别分别由 msg-center、所属应用、用户写 |

- 只有 `friend_state = friend` 能显示为双向好友；`access_level`、`is_verified` 都不代表好友关系。
- 带系统 tag（`zone_user` / `zone_agent`）的联系人是系统关系：`remove_friend`、`remove_contact` 返回 `system-relation`，收到的 `remove` 也不改变它。

### 7.2 MessageHub 的分流

msg-center 入站准入只看 `access_level`：

```text
Block                -> DROP
Allow                -> INBOX
Temporary（有效期内） -> INBOX
Stranger / 无记录     -> REQUEST_BOX
```

`buckyos.friend` 在写邮箱前单独处理（§6.3）：控制消息不进邮箱；新请求写一条记录，陌生人的请求因此出现在 REQUEST_BOX。My Network 的 Requests 把好友请求（`list_friend_requests`）和 REQUEST_BOX 里的普通消息分开列。MessageHub 按 intent 渲染好友请求卡尚未接入。

### 7.3 My Network 的职责

后台公共能力都在 msg-center 联系人模块：查找（`contact.lookup`）、关系状态、好友请求、tag 与写权限、联系人变更事件（Contact Mgr.md §3）。公共关系 UI 在 Desktop `src/frame/desktop/src/components/network/`，My Network、MessageHub、HomeStation 共用。My Network 是关系管理入口，负责：

- 展示 Contacts、Friends、Requests、Joined Groups；
- 管理 Collection（即 tag，§7.4）；
- 发起与处理好友请求，删除、解除好友、屏蔽；
- 合并第三方账号、管理 Source Relation（未实现）。

关系变化由 msg-center 按 owner 发 kevent `/msg_center/<owner>/CONTACTS/changed`（Contact Mgr.md §3.8）。订阅方（HomeStation、Desktop 公共组件的缓存）收到后重新读取，不另做同步。

### 7.4 Tag

分组一律用 `Contact.tags`（`Contact.groups` 已删除），Collection 就是 tag。tag 按名字分三类（`contact_tag_kind`）：

| 类别 | 规则 | 例 | 谁写 |
| --- | --- | --- | --- |
| 系统 | `zone_user`、`zone_agent` | 本 Zone 用户、我的 Agent | 只有 msg-center 的 Zone 同步 |
| 应用 | `<app>.<name>`：`app` 以小写字母开头、只含 `[a-z0-9_-]`，`name` 只含 `[a-z0-9_.-]` | `homestation.following`（我关注的人） | 只有服务 `system:<app>`，另需 RBAC `obj://msg-center/contact_tags/<app>` 的 `write` |
| 用户 | 其他任何文本（≤ 64 字符，不含换行） | “家人”“Team A” | 用户本人 |

应用之间约定公共 tag 就能对上：HomeStation 主动关注某人时写 `homestation.following`，My Network 直接显示这组人，即单向关系。well-known 表 `WELL_KNOWN_CONTACT_TAGS` 在 buckyos-api 与 websdk 各有一份，写明 key、类别、所属应用和用户能否编辑。写入用 `contact.update_tags`（Contact Mgr.md §3.5）。

---

## 8. 第三方关系导入

第三方社交账号、Master Hub / Message Tunnel、通讯录和文件导入应统一进入 Source Relation。

### 8.1 导入不是原生好友

例如用户接入 Facebook 后：

```text
Alice.facebook has friend Bob.facebook
```

这只生成：

```text
SourceRelation(source_type=facebook, relation_type=friend)
```

不自动改变：

```text
friend_state
```

如果系统发现 Bob.facebook 绑定了 `did:bns:bob`，可以提示：

```text
你和 Bob 是 Facebook 好友，是否发送 BuckyOS 好友请求？
```

### 8.2 管理边界

第三方关系必须带 `managed_by`。

```text
managed_by = facebook
```

表示：

- BuckyOS 可以展示；
- BuckyOS 可以搜索；
- BuckyOS 可以用于推荐；
- BuckyOS 可以从本地隐藏或删除导入记录；
- BuckyOS 不应假装自己能解除 Facebook 好友关系。

### 8.3 通讯录导入

手机号、邮箱、CSV、手机通讯录导入也应生成 Source Relation。

它可以升级为 Contact：

```text
phonebook entry -> local_contact
```

但不能自动成为 DID 好友。当前 `contact.import_contacts` 的 `upgrade_to_friend` 只把准入设为 `Allow`，不改 `friend_state`。

手机号 / 邮箱发现 DID 也只是身份补全，不是好友确认。

---

## 9. 与 Home Station 的关系

Home Station 不用 `access_level` 判断社交关系（拉黑除外）。当前实现（`src/frame/homestation/src/contacts.rs`）从 Message Center 读：

```text
friend    = friend_state == friend
blocked   = access_level == Block
groups    = 用户 tag（受众“分组”与“不看”分组）
following = 带 homestation.following
```

- 好友默认互相关注：HomeStation 订阅 `/msg_center/*/CONTACTS/**`，事件到达时重新同步好友派生的关注，不再每小时轮询。
- 主动关注 / 取消关注时写入或移除 `homestation.following`；因好友关系产生的关注不打这个 tag。HomeStation 的 `sources` 表仍是关注的真相源，tag 是它写出的投影，启动时和每次联系人变更后按 `sources` 对账。
- 受众：

```text
public    -> anyone
followers -> 已关注者（好友视同关注者）
friends   -> friend_state == friend
group     -> 带该用户 tag 的联系人
dids      -> 指定名单
blocked   -> deny
```

`profile_visibility`、`comment_policy`、`source_relations` 未实现。细节见 HomeStation 协议与实现 §5.1、§5.5。

---

## 10. 隐私与反欺诈原则

### 10.1 默认私有

My Network 的关系图默认是私有数据。

默认不公开：

- 好友列表；
- 通讯录；
- 第三方导入关系；
- 手机号 / 邮箱 hash；
- incoming / outgoing requests。

### 10.2 好友关系不是信用背书

即使 A 和 B 是好友，也不能自动推出：

```text
B 推荐 A
B 担保 A
B 当前信任 A 的某个行为
B 同意 A 对 C 披露这段关系
```

这些都属于单独的证明 / 背书能力，不进入本文协议。

### 10.3 防垃圾请求

接收方 Zone 应能设置：

- 陌生人请求是否进入 RequestBox；
- 请求频率限制；
- 是否只允许共同 Group / invite code / known source 发起请求；
- 是否自动丢弃低信誉来源；
- block 后是否静默 drop。

当前实现只有固定规则，均不可配置：屏蔽后丢弃（发送方只看到投递失败）；拒绝或忽略后 24 小时冷却；附言 ≤ 500 字符；陌生人的请求进 REQUEST_BOX；无 Bearer 的跨 Zone 签名入站每个发送方每分钟 30 条，在验签前按声称的 `from` 扣除（Message Center.md §4.5）。其余记为已知风险。

---

## 11. 可参考的公开经验

### 11.1 ActivityPub

ActivityPub 的 Follow / Accept / Reject / Undo 模型值得参考。

可借鉴点：

- 关系请求是消息；
- 接收方可以接受或忽略；
- Undo 可以表达撤销；
- 没有要求全局强一致。

不直接照搬点：

- BuckyOS 以 DID 和 User binded Zone 为身份基础；
- 好友关系默认不应公开成 federation graph；
- BuckyOS 需要处理第三方账号绑定和 Message Tunnel。

### 11.2 DID / DIDComm

可借鉴点：

- DID Document / service endpoint 用于发现；
- 消息可以基于 DID 签名和加密；
- endpoint 可以随 binded Zone 迁移。

不直接照搬点：

- 不需要为了好友请求引入完整 DIDComm runtime；
- 可以先使用 BuckyOS 现有 MsgObject / SendMessage。

### 11.3 WebFinger / acct URI

可借鉴点：

- 传统账号到身份文档的发现方式；
- 有助于未来从 email、domain、社交账号发现 DID。

不直接照搬点：

- WebFinger 只是发现，不解决好友关系状态。

---

## 12. 实施阶段

### 12.1 第一阶段（2026-10-17 后端已实现）

1. `buckyos.friend` 的 schema 与类型（buckyos-api `msg_contact.rs`、websdk）。
2. Contact `friend_state`、`friend_requests` 表与五个好友 RPC。
3. 好友请求按准入进 REQUEST_BOX；accept 等作为控制消息同步双方状态。
4. `AccessGroupLevel::Friend` 改名 `Allow`，只表示投递策略。
5. `contact.lookup`、`update_tags`、`remove_contact`、`unblock_contact`，联系人变更事件；`Contact.groups` 并入 tags。
6. 跨 Zone 原生投递的自动路由、无 Bearer 签名入站与 Zone 托管签名（Message Center.md §2.2、§4.5）。

测试（`src/frame/msg_center/src/test_contact_network.rs`）覆盖一个 Zone 内的请求、接受、忽略、取消、交叉请求、解除、屏蔽后请求、冷却与重放，以及两个 msg-center 实例模拟的两个 Zone 之间的请求往返与私聊。真实跨 Zone 未在 DV 验证（DV 只有一个 Zone）。

### 12.2 第二阶段

实现范围：

1. 第三方 Source Relation Store。
2. Social Account / Message Tunnel 导入关系图。
3. Source Relation 与 DID Contact 合并。
4. 从第三方关系推荐发起 DID friend request。
5. Home Station 使用更完整的关系视图（profile_visibility、comment_policy）判断访问策略。

### 12.3 暂不做

暂不做：

- 公开好友列表；
- 网络目录 / 全局搜索（查找只覆盖自己的关系、本 Zone 与名字服务）；
- 第三方可验证好友证明；
- 信用背书；
- 链上好友关系；
- 复杂共同好友推荐；
- 反向管理 Facebook / Telegram 好友关系。

---

## 13. 已确认问题（2026-10-17）

1. **schema 放在哪**：`buckyos-api` 的 `msg_contact.rs`（intent、action、请求类型、lookup 与 tag 规则），websdk `msg_center_client.ts` 有对应 TS 定义；不放 MessageHub 专属模块。
2. **Social Graph Store**：继续放在 msg-center 的 ContactMgr（按 owner 分库），不拆独立 system service。
3. **accept 是否引用原始 request MsgObjectId**：不引用。所有动作只带 `request_id`，双方按 `(peer, request_id)` 对上；记录里的 `msg_id` 只用于展示。
4. **unfriend notice**：做成尽力通知的 `remove` 动作（`remove_friend` 默认通知），收到方 `friend` → `none`，**保留联系人记录**。
5. **Source Relation 是否保留外部 relation id**：未定，Source Relation 尚未实现。
6. **`AccessGroupLevel::Friend` 是否改名**：已改为 `Allow`（线上值 `allow`），与 `friend_state` 同时引入，不做兼容；存量联系人在加载时迁移（Contact Mgr.md §3.9）。
7. **DID 原生消息失败时 UI 是否降级**：是。msg-center 如实给出原因（lookup 的 `reachability`、请求的 `delivery`），Desktop 加好友对话框（`components/network/AddFriendDialog.tsx`）在不可达或送达失败时提供“复制我的分享链接”；“通过第三方发送邀请”未做。

---

## 14. 简短总结

My Network 的社交协议应保持轻量：

```text
现有 SendMessage 通道
+ buckyos.friend 特殊 MsgObject
+ 本地 Social Graph 状态机（friend_state + 请求记录）
+ 第三方 Source Relation
+ MessageHub 投递策略映射（access_level）
```

它不应变成：

```text
新的通讯协议
强一致好友系统
链上关系合约
公开关系证明系统
信用背书系统
```

这样设计可以最大化复用当前 BuckyOS 的 DID、binded Zone、MessageHub、ContactMgr 和 Message Tunnel 基础设施，同时为 My Network 提供清晰的产品和协议边界。
