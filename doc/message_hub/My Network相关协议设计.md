# My Network 相关协议设计

本文记录 My Network、MessageHub、ContactMgr、DID / binded Zone 在好友请求、联系人关系、第三方社交账号导入上的协议设计草稿。

核心结论是：**不新增独立通讯协议**。My Network 相关协议应建立在现有 DID 解析、User binded Zone、MsgObject / SendMessage、MessageHub 和密码学签名设施之上。所谓“好友协议”本质上是一组特殊 Message Object 的 schema，以及各 Zone 本地 Social Graph 状态机的处理约定。

---

## 1. 基本结论

### 1.1 不设计新的好友通讯通道

好友请求不需要新的传输层协议。

推荐模型是：

```text
A -> resolve B Owner DID
  -> resolve B binded Zone
  -> use existing SendMessage / MessageHub path
  -> send a typed social MsgObject
```

也就是说：

```text
bucky.social.friend.request
bucky.social.friend.accept
bucky.social.friend.reject
bucky.social.friend.cancel
bucky.social.friend.unfriend_notice
```

都只是特殊消息类型。

这些消息是否能送达、是否会进入 RequestBox、是否被 My Network 处理，最终取决于目标用户 Zone 的 MessageHub / My Network 策略。

### 1.2 好友关系是本地状态，不是强一致全局状态

去中心化系统里，好友关系不应被设计成全局强一致状态。

例如：

```text
1. A 给 B 发好友请求。
2. B 可以接受、拒绝，也可以不处理。
3. B 接受后，A 本地可以显示 mutual_friend。
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

本协议草稿要解决：

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

发送社交消息时，推荐流程为：

```text
Alice input Bob DID / BNS
  -> resolve Bob DID
  -> read Bob OwnerConfig / DID Document
  -> find Bob binded Zone
  -> find Bob MessageHub / Social message endpoint
  -> send social MsgObject
```

这个流程可复用现有 DID resolver、Zone resolver、MessageHub 和 Gateway。

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

### 4.3 DID Relation

DID Relation 表达 BuckyOS 原生关系状态。

推荐状态：

```text
none
local_contact
outgoing_pending
incoming_pending
mutual_friend
rejected
cancelled
unfriended
blocked
```

这个状态是本地的。

同一时刻，A 本地和 B 本地可以不一致。

---

## 5. Social MsgObject Schema

### 5.1 通用 Envelope

社交消息使用普通 MsgObject，但 content / meta 中声明机器可识别类型。

概念结构：

```json
{
  "kind": "bucky.social",
  "schema": "bucky.social.friend.request",
  "version": "0.1",
  "id": "uuid",
  "from_did": "did:bns:alice",
  "to_did": "did:bns:bob",
  "created_at": 1760000000,
  "expires_at": 1760600000,
  "body": {}
}
```

真正签名、对象 ID、from / to 的稳定性应遵守 MsgObject 既有规则。

### 5.2 Friend Request

```json
{
  "schema": "bucky.social.friend.request",
  "request_id": "uuid",
  "from_did": "did:bns:alice",
  "to_did": "did:bns:bob",
  "message": "Hi Bob",
  "profile_hint": {
    "display_name": "Alice",
    "avatar": "cyfs://..."
  },
  "requested_relation": "friend",
  "created_at": 1760000000,
  "expires_at": 1760600000
}
```

接收方语义：

- 可以放入 My Network Requests；
- 可以自动合并到已有 Contact；
- 可以静默忽略；
- 可以直接 Block；
- 不要求一定回复。

### 5.3 Friend Accept

```json
{
  "schema": "bucky.social.friend.accept",
  "request_id": "uuid",
  "from_did": "did:bns:bob",
  "to_did": "did:bns:alice",
  "accepted_relation": "friend",
  "created_at": 1760000100
}
```

接收方语义：

- A 收到 B 的 accept 后，可以把本地关系更新为 `mutual_friend`；
- A 可以把 B 加入 Contacts / Friends 基础视图；
- 这不保证 B 未来继续接收 A 的普通消息。

### 5.4 Friend Reject

```json
{
  "schema": "bucky.social.friend.reject",
  "request_id": "uuid",
  "from_did": "did:bns:bob",
  "to_did": "did:bns:alice",
  "reason_code": "not_now",
  "created_at": 1760000100
}
```

`reason_code` 应是机器可读的弱语义，不建议暴露过多拒绝原因。

### 5.5 Friend Cancel

```json
{
  "schema": "bucky.social.friend.cancel",
  "request_id": "uuid",
  "from_did": "did:bns:alice",
  "to_did": "did:bns:bob",
  "created_at": 1760000200
}
```

用于撤回尚未处理的好友请求。

### 5.6 Unfriend Notice

```json
{
  "schema": "bucky.social.friend.unfriend_notice",
  "relation_id": "local-or-shared-id",
  "from_did": "did:bns:bob",
  "to_did": "did:bns:alice",
  "created_at": 1760000300
}
```

该消息只是通知，不是强制协议。

B 可以选择通知 A，也可以不通知。A 收到后可以把本地状态降级为 `local_contact` 或 `unfriended`。

---

## 6. 本地状态机

### 6.1 发起方 A

```text
none
  -> local_contact
  -> outgoing_pending
  -> mutual_friend
  -> unfriended

outgoing_pending -> cancelled
outgoing_pending -> rejected
any -> blocked
```

### 6.2 接收方 B

```text
none
  -> incoming_pending
  -> mutual_friend

incoming_pending -> rejected
incoming_pending -> ignored
any -> blocked
```

### 6.3 状态不一致是正常状态

例子：

```text
A: mutual_friend
B: blocked
```

这表示 A 本地仍认为 B 是好友，但 B 的 MessageHub 会拒收 A 的消息。

协议不要求 B 必须把 block 通知 A。

---

## 7. 与 ContactMgr / MessageHub 的关系

### 7.1 ContactMgr 当前 Friend 语义需要收窄

当前 `AccessGroupLevel::Friend` 更接近 MessageHub 的本地准入策略：

```text
Friend = allow Inbox / allow notification
```

它不应被解释为协议层 mutual friend。

推荐后续拆分：

```text
DidRelation.mutual_friend
  -> policy engine
  -> AccessGroupLevel::Friend
```

也就是说，`AccessGroupLevel` 是投递策略结果，不是社交协议真相源。

### 7.2 MessageHub 的推荐分流

MessageHub 可以根据 Social Graph 推导：

```text
blocked -> DROP
mutual_friend -> INBOX
temporary -> INBOX
incoming friend request -> REQUEST_BOX / My Network Requests
stranger -> REQUEST_BOX / Spam
```

但这只是默认策略。用户和 Agent 可以继续覆盖。

### 7.3 My Network 的职责

My Network 负责：

- 展示 Contacts；
- 展示 Friends；
- 展示 Requests；
- 合并第三方账号；
- 管理 Source Relation；
- 管理 Collection；
- 将 DID social messages 转化为本地状态变更；
- 将状态变更反映给 MessageHub 和 Home Station。

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

不自动生成：

```text
DidRelation(mutual_friend)
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

但不能自动升级为 DID mutual friend。

手机号 / 邮箱发现 DID 也只是身份补全，不是好友确认。

---

## 9. 与 Home Station 的关系

Home Station 不应直接读取 ContactMgr 的 `AccessGroupLevel` 来判断社交关系。

推荐它消费 My Network / Social Graph 提供的关系视图：

```text
is_contact
is_mutual_friend
is_blocked
source_relations
profile_visibility
comment_policy
```

典型策略：

```text
public -> anyone
contacts_only -> local_contact or mutual_friend
friends_only -> mutual_friend
blocked -> deny
```

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

## 12. MVP 建议

### 12.1 第一阶段

实现范围：

1. 定义 `bucky.social.friend.*` MsgObject schema。
2. My Network 增加 Requests 视图。
3. Contact / Friend / Request 状态进入本地 Social Graph。
4. MessageHub 识别 friend request，默认进入 RequestBox。
5. Accept 后双方通过普通消息同步状态。
6. ContactMgr 的 Friend 语义收窄为本地投递策略。

### 12.2 第二阶段

实现范围：

1. 第三方 Source Relation Store。
2. Social Account / Message Tunnel 导入关系图。
3. Source Relation 与 DID Contact 合并。
4. 从第三方关系推荐发起 DID friend request。
5. Home Station 使用 Social Graph 判断访问策略。

### 12.3 暂不做

暂不做：

- 公开好友列表；
- 第三方可验证好友证明；
- 信用背书；
- 链上好友关系；
- 复杂共同好友推荐；
- 反向管理 Facebook / Telegram 好友关系。

---

## 13. 待确认问题

1. `bucky.social.friend.*` schema 放在 `buckyos-api` 还是 MessageHub 专属模块？
2. My Network 的 Social Graph Store 是否继续放在 MessageCenter，还是拆成独立 system service？
3. `friend.accept` 是否必须引用原始 request MsgObjectId？
4. `friend.unfriend_notice` 是否作为 best-effort notice，还是完全不设计？
5. 第三方 Source Relation 是否需要保留原始外部 relation id？
6. ContactMgr 中 `AccessGroupLevel::Friend` 是否改名，避免与 DID mutual friend 混淆？
7. DID 原生消息失败时，UI 是否应自动降级为“复制邀请链接 / 通过第三方发送邀请”？

---

## 14. 简短总结

My Network 的社交协议应保持轻量：

```text
现有 SendMessage 通道
+ 特殊 social MsgObject schema
+ 本地 Social Graph 状态机
+ 第三方 Source Relation
+ MessageHub 投递策略映射
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
