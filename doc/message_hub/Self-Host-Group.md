# Self-Host Group 组件需求

## 1. 背景与目标

本文基于 `Message Center.md` 和 `Contact Mgr.md` 的当前设计，说明 BuckyOS 在现有 MessageHub 框架下支持 **self-host group** 需要补齐的组件能力。

这里的 group 指 **MessageHub 中可收发消息的实体组**，不是 ContactMgr 里的普通联系人标签、权限集合或 UI 分类。判断标准是：这个 group 是否拥有自己的 DID，是否可以作为 `MsgObject` 的会话对象，是否可以在 MessageCenter 中拥有 `GROUP_INBOX`、成员订阅和群消息历史。

Self-host group 的含义是：

1. group 由当前用户的 Zone/OOD 托管，当前 Zone 是该 group 状态的真相源。
2. group 拥有独立 `Group DID`，在 Users & Agents / MessageHub 中是一级可管理实体，类似 agent。
3. group 的成员、角色、消息历史、投递策略和 read receipt 由 host Zone 维护。
4. 第一版不做多 host 共识，也不做多个 OOD/Zone 共同托管同一个 group 的协议。

### 1.1 设计目标

1. **复用现有消息模型**：继续使用不可变 `MsgObject`、可变 `MsgRecord`、`GROUP_INBOX`、per-reader `MsgReceiptObj`。
2. **复用 ContactMgr 的身份与路由能力**：group 成员仍是 DID；外部账号仍通过 `AccountBinding` 和 tunnel 路由。
3. **明确 self-host 权责**：host Zone 负责成员管理、写入校验、消息归档、成员投递和群信息展示。
4. **成员资格双向确认**：成员进入 `Active` 前必须有来自成员 DID 的有效签名，证明该用户愿意成为 group member。
5. **保持第一版简单**：单 host 权威模型，远端成员只是参与者；加入的外部 group 只作为 Contact/Joined Group 管理，不承担托管职责。

### 1.2 非目标

1. 不设计去中心化群共识协议。
2. 不要求群消息在所有成员 Zone 上形成一致的可写副本。
3. 不把普通联系人分组升级为 messageable group。
4. 不在本文定义完整 UI 交互细节，只列出 UI 需要读取或触发的后端能力。

---

## 2. 核心概念

### 2.1 Entity Group 与 Contact Collection 的区别

ContactMgr 当前有 `Contact.groups` / `tags`，这类 group 本质是本地联系人集合，用于分类、过滤或权限表达。它不拥有独立 DID，也不能作为消息的 `from/to` 主体。

Self-host group 是实体：

* 拥有 `group_did`
* 可被 MessageHub 打开为会话对象
* 可被 ContactMgr 识别为 `is_group_did(group_did) == true`
* 可被 MessageCenter 写入 `GROUP_INBOX`
* 有成员列表、角色、可见性、邀请策略和投递策略

### 2.2 Hosted Group 与 Joined Group

同一个 UI 中会看到两类实体组：

* **Hosted Group**：当前 Zone 是 host。用户可以管理成员、角色、邀请、禁言、消息保留策略等。
* **Joined Group**：当前用户只是成员。它在 ContactMgr 中表现为一个外部 group 联系人，MessageCenter 可以向它发消息，但本 Zone 不拥有群成员真相源。

当前文档只描述 Hosted Group 的组件需求。Joined Group 需要保存远端 group 的展示信息和路由入口，但不维护成员真相源。

### 2.3 群消息语义

与 MessageCenter 现有设计保持一致：

* 群消息的会话主体是 `group_did`。
* 群消息作者是 `source`。
* 规范化后的群消息建议使用：

```rust
MsgObject {
    from: group_did,
    source: author_did,
    to: member_dids,
    kind: "group_msg",
    ...
}
```

也就是说，UI 或 agent 可以提交 `from=author, to=[group]` 的发送意图，但进入群时间线前，MessageCenter/Group 管理组件应把它规范化为 `from=group, source=author` 的群消息对象。

### 2.4 成员资格双向确认

用户成为 group member 的核心条件不是管理员单方面把 DID 写入成员表，而是同时满足：

1. group host 侧有邀请、审批或加入授权。
2. member 侧产生了有效签名，明确同意加入该 `group_did`。

签名对象应至少绑定 `group_did`、`member_did`、目标角色、邀请或申请 id、过期时间和 nonce，避免被复用到其它 group 或其它角色。

如果 member 与 group 在同一个 Zone，签名可以由本 Zone 的登录态、VerifyHub 或用户 DID 代理自动构造；但数据模型上仍应保存为可验证的 `GroupMemberProof`。这样同 Zone 自动加入和跨 Zone 手工确认使用同一套成员资格模型。

### 2.5 Subgroup

MessageHub 交互上需要支持 group 下的 subgroup。Subgroup 是父 group 内的小集合，用于更快地构建临时小组、项目小组或通知范围。

第一版 subgroup 不默认拥有独立 DID，它是 parent `group_did` 下的有名成员集合。subgroup 的成员必须已经是父 group 的 `Active` member，并且已经完成 DID 层面的双向确认。若某个 subgroup 需要脱离父 group 成为独立 messageable 实体，应升级为新的 self-host group，而不是在 subgroup 上临时发明第二套身份模型。

---

## 3. 组件边界

### 3.1 GroupMgr

Self-host group 需要一个明确的 group 管理能力。实现上可以是 ContactMgr 内部模块，也可以独立为 `GroupMgr` 系统服务；从职责上应独立描述，避免把联系人聚合逻辑和群权威状态混在一起。

GroupMgr 负责：

1. 创建 self-host group，生成/登记 `group_did`。
2. 保存 group 基本信息、成员列表、角色、设置和版本号。
3. 保存并验证成员加入签名，只有具备有效 `GroupMemberProof` 的成员才能进入 `Active`。
4. 校验成员是否允许发言、邀请、管理成员。
5. 为 MessageCenter 提供 `is_group_did()`、`get_group_subscribers()`、`check_group_send_permission()`。
6. 为 ContactMgr 提供 group 联系人视图和 joined/hosted 区分。
7. 管理 subgroup，保证 subgroup 成员是父 group 的有效成员。
8. 生成成员变更事件，供 MessageHub UI 和审计查看。

第一版建议 GroupMgr 的数据落在 MessageHub/ContactMgr 使用的 named store 中，不引入新数据库依赖。

### 3.2 ContactMgr

ContactMgr 继续负责身份和路由，但需要理解 group 是一种实体 DID。

新增或补齐能力：

1. `is_group_did(did) -> bool`：判断 DID 是否是已知 group。
2. `get_group_profile(group_did)`：返回 group 名称、头像、描述、host 信息、joined/hosted 状态。
3. `get_group_subscribers(group_did) -> Vec<DID>`：返回当前 Zone 内需要收到该群消息的 reader。对 self-host group，通常是本 Zone 内成员用户和已订阅 agent。
4. `plan_group_delivery(group_did, msg_obj, context)`：把群消息转换成面向成员或 tunnel 的投递计划。
5. `resolve_group_member_binding(member_did)`：对远端成员，继续复用 `AccountBinding` / tunnel 选路逻辑。
6. `check_group_access(group_did, actor_did, action)`：给 MessageCenter 和 UI 判断发言、邀请、踢人、修改配置等权限。

ContactMgr 不应把 self-host group 简化成 `Contact.groups` 字段。`Contact.groups` 只能表示某个联系人在本地通讯录里的分类。

### 3.3 MessageCenter

MessageCenter 是群消息进入系统后的唯一消息域真相源。

需要满足：

1. 支持 `GROUP_INBOX`，owner 为 `group_did`。
2. 支持 group message dispatch：校验 group 存在、作者权限、消息规范化、幂等写入。
3. 对每个本地 reader 创建 `INBOX` 记录或 read receipt 初始化，但不能复制 `MsgObject`。
4. 支持 per-reader `MsgReceiptObj`，群消息已读状态跟 reader 走，不跟 group 消息本体走。
5. 支持 host-side delivery plan：self-host group 收到群消息后，由 host 负责向远端成员投递。
6. 支持成员变更的系统消息或 event message，使群历史能解释成员加入、退出、改名、权限变化。
7. 支持把需要管理员确认的 group operation 发送到管理员个人 `INBOX`，而不只写入 group 事件流。
8. 支持把成员被移除、邀请被拒绝等面向个人的通知发送到该成员个人 `INBOX`。

关键约束：

* `MsgObject` 仍然不可变。
* 投递重试、外部 message id、tunnel 路由、失败状态仍写在 `MsgRecord.route` / `MsgRecord.delivery`。
* 群成员变更不能回改历史消息的 `to` 列表；历史消息只代表写入时的成员快照或投递计划。

### 3.4 Tunnel / Native Transport

Self-host group 需要两类投递路径：

1. **BuckyOS 原生 DID 路径**：成员也是 BuckyOS DID 时，优先通过 DID/Zone 路由投递到对方 MessageCenter。
2. **外部平台 tunnel 路径**：成员只有 Telegram/Email 等绑定时，通过 ContactMgr 选出的 tunnel 投递。

Tunnel 侧需求：

1. 能消费 `TUNNEL_OUTBOX` 中属于 group delivery 的记录。
2. 发送成功后通过 `report_delivery(record_id, result)` 回写外部消息 id 和状态。
3. 外部平台回流的群消息必须能映射到 `group_did` 和 `source`，不能只保留平台 chat id。
4. 同一外部平台群如果只是同步进来的 joined group，不应默认变成 self-host group。

### 3.5 VerifyHub / RBAC

Self-host group 是可管理实体，管理操作必须走系统身份和授权。

需要的权限动作：

* `group.create`
* `group.update_profile`
* `group.invite_member`
* `group.approve_member`
* `group.remove_member`
* `group.update_role`
* `group.manage_subgroup`
* `group.post_message`
* `group.read_history`
* `group.archive_or_delete`

第一版可以把权限映射到 group 内部角色，不必先做通用 RBAC Schema，但接口需要保留 action 维度，避免后续迁移困难。

### 3.6 MessageHub / Users & Agents UI

UI 需要区分：

* `isHostedBySelf = true`：显示为当前 Zone 托管，可管理。
* `isHostedBySelf = false`：显示为已加入 group，只能查看远端公开信息和本地会话设置。
* `canMessage = true`：能打开 MessageHub 会话。

UI 需要的后端能力：

1. 创建 self-host group。
2. 查看 group DID、host、成员数、成员列表、角色。
3. 邀请/移除成员。
4. 打开 group conversation。
5. 查看成员变更事件和群系统消息。
6. 对本地 reader 维护静音、置顶、归档等 per-user 设置。
7. 在个人 inbox 中处理需要管理员确认的加入申请、邀请确认和敏感变更。
8. 创建和管理 subgroup。

---

## 4. 数据模型需求

### 4.1 GroupDoc

`GroupDoc` 是 self-host group 的公开或半公开描述对象，类似 `ServiceDoc` / `DeviceDoc` 这类 Doc 对象，应该可验证、可缓存。

建议字段：

```rust
pub struct GroupDoc {
    pub group_did: DID,
    pub host_zone: DID,
    pub owner: DID,
    pub name: String,
    pub avatar: Option<String>,
    pub description: Option<String>,
    pub created_at_ms: u64,
    pub updated_at_ms: u64,
    pub profile_version: u64,
    pub policy: GroupPolicy,
    pub proof: Option<String>,
}
```

`GroupDoc` 不承载完整成员列表。成员列表变化频繁，应该进入 GroupMgr 的状态存储，并通过版本号、事件或摘要引用。

### 4.2 GroupMemberRecord

```rust
pub struct GroupMemberRecord {
    pub group_did: DID,
    pub member_did: DID,
    pub role: GroupRole,
    pub state: GroupMemberState,
    pub joined_at_ms: u64,
    pub updated_at_ms: u64,
    pub invited_by: Option<DID>,
    pub approved_by: Option<DID>,
    pub member_proof_id: Option<ObjId>,
    pub mute_until_ms: Option<u64>,
    pub delivery_preference: Option<GroupDeliveryPreference>,
}

pub enum GroupRole {
    Owner,
    Admin,
    Member,
    Guest,
}

pub enum GroupMemberState {
    Invited,
    PendingMemberSignature,
    PendingAdminApproval,
    Active,
    Muted,
    Left,
    Removed,
    Blocked,
}
```

成员记录是 group 的权威状态。ContactMgr 可把它投影成联系人视图，但不能以联系人视图反推权威成员表。

`Active` 状态必须满足 `member_proof_id` 指向的 `GroupMemberProof` 可验证。管理员邀请只能把成员推进到 `Invited` 或 `PendingMemberSignature`，不能直接把远端 DID 写成 `Active`。

### 4.3 GroupMemberProof

`GroupMemberProof` 是用户同意成为 group member 的签名对象。它可以是 NamedObject，也可以是 GroupMgr 状态中的可验证对象；关键是必须可审计、可重放验证、可绑定上下文。

```rust
pub struct GroupMemberProof {
    pub group_did: DID,
    pub member_did: DID,
    pub role: GroupRole,
    pub invite_id: Option<String>,
    pub request_id: Option<String>,
    pub nonce: String,
    pub issued_at_ms: u64,
    pub expires_at_ms: Option<u64>,
    pub signer: DID,
    pub proof: String,
}
```

同 Zone 用户的 proof 可以自动构造，但仍应写入同样的结构。自动构造的前提是当前登录态可以代表 `member_did` 完成签名或授权，不能由管理员替成员伪造。`signer` 必须是 `member_did`，或是 `member_did` 的 DID Document 中授权过的 key/agent。

### 4.4 GroupSettings

```rust
pub struct GroupSettings {
    pub group_did: DID,
    pub join_policy: JoinPolicy,
    pub post_policy: PostPolicy,
    pub history_visibility: HistoryVisibility,
    pub retention_policy: RetentionPolicy,
    pub default_delivery: GroupDeliveryPreference,
}
```

```rust
pub enum JoinPolicy {
    InviteOnly,
    RequestAndAdminApprove,
}

pub enum PostPolicy {
    AllMembers,
    AdminOnly,
    RoleBased(Vec<GroupRole>),
}
```

第一版建议最小支持：

* 只有 owner/admin 可邀请。
* 只有完成成员签名的 active member 可进入成员列表。
* `PostPolicy::AllMembers` 时 active member 可发言。
* `PostPolicy::AdminOnly` 时 group 是纯通知群，只有 owner/admin 可写入 `GROUP_INBOX`。
* 新成员默认只能看到加入后的消息。
* 历史保留策略先跟 MessageCenter 默认策略一致。

### 4.5 GroupSubgroup

```rust
pub struct GroupSubgroup {
    pub group_did: DID,
    pub subgroup_id: String,
    pub name: String,
    pub description: Option<String>,
    pub member_dids: Vec<DID>,
    pub created_by: DID,
    pub created_at_ms: u64,
    pub updated_at_ms: u64,
}
```

Subgroup 是父 group 内的轻量集合，不能包含非父 group 成员。它可用于 UI drilldown、定向通知、权限范围或 thread/topic 聚合。若 subgroup 需要独立入群确认、独立管理员、独立消息历史和独立 DID，应创建新的 self-host group。

### 4.6 GroupEvent

成员变更和配置变更需要可观察，建议写成轻量事件，并可选择同步成群系统消息。

```rust
pub struct GroupEvent {
    pub event_id: String,
    pub group_did: DID,
    pub actor: DID,
    pub event_type: GroupEventType,
    pub target: Option<DID>,
    pub created_at_ms: u64,
    pub detail: Map<String, String>,
}
```

典型事件：

* `GroupCreated`
* `MemberInvited`
* `MemberJoinRequested`
* `MemberProofAccepted`
* `MemberApprovalRequested`
* `MemberJoined`
* `MemberLeft`
* `MemberRemoved`
* `RoleChanged`
* `ProfileUpdated`
* `PolicyUpdated`
* `SubgroupCreated`
* `SubgroupUpdated`

---

## 5. 存储与索引需求

建议使用 named store / KV 索引，保持与 MessageCenter 的 `records/{record_id}`、`box/{owner}/{box_kind}/...` 思路一致。

```text
groups/{group_did}/doc -> GroupDoc
groups/{group_did}/settings -> GroupSettings
groups/{group_did}/members/{member_did} -> GroupMemberRecord
groups/{group_did}/member_proofs/{proof_id} -> GroupMemberProof
groups/{group_did}/members_by_state/{state}/{member_did} -> 1
groups/{group_did}/members_by_role/{role}/{member_did} -> 1
groups/{group_did}/subgroups/{subgroup_id} -> GroupSubgroup
groups/{group_did}/subgroups_by_member/{member_did}/{subgroup_id} -> 1
groups/{group_did}/events/{created_at_ms}/{event_id} -> GroupEvent
groups_by_host/{host_zone}/{group_did} -> 1
groups_by_member/{member_did}/{group_did} -> membership_summary
```

MessageCenter 继续维护：

```text
box/{group_did}/GROUP_INBOX/time/{sort_key}/{record_id} -> 1
box/{reader_did}/INBOX/time/{sort_key}/{record_id} -> 1
rr/{group_did}/{reader_did}/{msg_id} -> MsgReceiptObjId
```

`groups_by_member` 是 ContactMgr / UI 快速列出 joined/hosted group 的索引，不是权限真相源。权限判断必须回到 `groups/{group_did}/members/{member_did}`。

---

## 6. 核心流程

### 6.1 创建 Self-Host Group

1. 用户在 Users & Agents 或 MessageHub 中发起创建。
2. VerifyHub/RBAC 校验当前用户是否允许创建 group。
3. GroupMgr 生成 `group_did`，写入 `GroupDoc`、`GroupSettings`。
4. GroupMgr 为 owner 构造或收集 `GroupMemberProof`。
5. GroupMgr 写入 owner 的 `GroupMemberRecord(role=Owner, state=Active, member_proof_id=...)`。
6. ContactMgr 建立 group entity 投影，使其出现在实体列表中。
7. MessageCenter 可选写入一条 `GroupCreated` 系统消息到 `GROUP_INBOX`。

### 6.2 邀请成员

1. actor 请求邀请 `member_did`。
2. GroupMgr 校验 actor 是否有 `group.invite_member`。
3. ContactMgr 解析 `member_did`，必要时创建 Shadow Contact 或读取绑定。
4. GroupMgr 写入 `GroupMemberRecord(state=Invited)` 和 `GroupEvent(MemberInvited)`。
5. MessageCenter 向 member 的个人 `INBOX` 发送邀请消息或系统通知。
6. member 接受邀请并提交 `GroupMemberProof`。
7. 如果 group 不需要管理员复核，GroupMgr 验证 proof 后直接把成员状态置为 `Active`，写入 `MemberJoined` 事件。
8. 如果 group 需要管理员确认，GroupMgr 把成员状态置为 `PendingAdminApproval`，MessageCenter 向 owner/admin 的个人 `INBOX` 发送待确认消息。
9. 管理员确认后，成员状态变为 `Active`，写入 `MemberJoined` 事件。
10. 如果 member 与 group 在同一个 Zone，步骤 5-6 可以由本 Zone 在用户授权下自动完成，但仍必须生成 `GroupMemberProof`。

### 6.3 加入申请

1. member 主动请求加入 `group_did`。
2. member 提交 `GroupMemberProof`，证明自己愿意加入。
3. GroupMgr 校验 proof 和 `JoinPolicy`。
4. 如果 `JoinPolicy=RequestAndAdminApprove`，GroupMgr 写入 `PendingAdminApproval`，MessageCenter 向 owner/admin 的个人 `INBOX` 发送确认请求。
5. 管理员批准后写入 `Active` 成员记录；拒绝时向申请人的个人 `INBOX` 发送结果通知。

### 6.4 发送群消息

1. author 在 MessageHub/Agent 中向 `group_did` 发送消息。
2. MessageCenter 调用 GroupMgr 校验 `group.post_message`，校验内容包括成员状态、成员签名和 `PostPolicy`。
3. MessageCenter 规范化 `MsgObject`：`from=group_did`，`source=author_did`，`kind=group_msg`。
4. MessageCenter 幂等保存 `MsgObject`。
5. MessageCenter 写入 group 的 `GROUP_INBOX` 记录。
6. MessageCenter 根据 GroupMgr/ContactMgr 返回的本地 subscribers 创建 reader `INBOX` 记录或 read receipt。
7. ContactMgr 生成远端成员 delivery plan。
8. MessageCenter 写入一个或多个 `TUNNEL_OUTBOX` 记录。
9. tunnel 投递后通过 `report_delivery()` 回写状态。

如果 `PostPolicy=AdminOnly`，普通 member 不能写入 group。这类 group 是通知群，管理员发送的消息仍按普通 group message 进入 `GROUP_INBOX` 和成员 inbox。

### 6.5 接收远端成员发来的群消息

1. tunnel 或 native transport 收到远端消息。
2. ContactMgr 解析外部身份到 `source_did`。
3. GroupMgr 根据 `group_did` 和 `source_did` 校验成员状态。
4. MessageCenter 规范化并写入 `GROUP_INBOX`。
5. 后续流程与本地发送群消息一致。

远端消息不能绕过 host 直接写本地成员的 inbox。Self-host group 的写入点必须收敛到 host Zone 的 MessageCenter。

### 6.6 管理员确认通知

当 group operation 需要管理员确认时，通知必须进入管理员的个人 `INBOX`，而不是只写入 `groups/{group_did}/events/...`。

这类通知建议用 `MsgObject.kind=operation` 或 `notify`，并在 `content.machine` 中包含：

* `operation_id`
* `group_did`
* `operation_type`
* `actor_did`
* `target_did`
* `expires_at_ms`
* approve/reject 所需的 callback 或 KRPC action

管理员确认后，GroupMgr 写入正式 `GroupEvent`，MessageCenter 再根据需要向申请人、目标成员或 group timeline 发送结果通知。

### 6.7 成员移除通知

1. 管理员移除 member。
2. GroupMgr 校验管理员权限，更新成员状态为 `Removed`，写入 `MemberRemoved` 事件。
3. MessageCenter 向被移除成员的个人 `INBOX` 发送移除通知，说明 group、操作者、时间和可展示原因。
4. MessageCenter 可选向 group 的 `GROUP_INBOX` 写入系统消息，具体是否公开展示由 group policy 决定。

成员移除通知是面向个人的结果通知，不应只写在 group 事件流里。被移除成员即使不再能读 group history，也应该能收到这条个人通知。

### 6.8 Subgroup 管理

1. 管理员或有权限的 member 创建 subgroup。
2. GroupMgr 校验 `group.manage_subgroup`。
3. GroupMgr 校验所有 subgroup 成员都是父 group 的 `Active` member，且都有有效 `GroupMemberProof`。
4. GroupMgr 写入 `GroupSubgroup` 和 `SubgroupCreated/SubgroupUpdated` 事件。
5. MessageHub 可用 subgroup 做 drilldown、定向通知或小集合选择。

### 6.9 阅读状态

1. reader 打开 group conversation。
2. MessageCenter 对该 reader 写入或更新独立阅读状态，可落在 reader 的 `MsgRecord.state`，也可以在 `MsgReceiptObj` 中扩展独立 `read_state` 字段。
3. read receipt 索引使用 `rr/{group}/{reader}/{msg_id}`。
4. 群消息本体和 group inbox record 不表达某个 reader 是否已读。
5. 现有 `ReceiptStatus::Accepted/Rejected/Quarantined` 表达接收结果，不应直接复用为 `READING/READED`。

---

## 7. 对外接口需求

### 7.1 GroupMgr API

```python
class GroupMgr:
    def create_group(self, owner_did, profile, settings=None) -> GroupDoc: pass
    def get_group_doc(self, group_did) -> GroupDoc: pass
    def update_group_profile(self, actor_did, group_did, patch) -> GroupDoc: pass

    def invite_member(self, actor_did, group_did, member_did, role="Member") -> GroupMemberRecord: pass
    def submit_member_proof(self, group_did, member_proof) -> GroupMemberRecord: pass
    def request_join(self, member_did, group_did, member_proof) -> GroupMemberRecord: pass
    def approve_member(self, actor_did, group_did, member_did) -> GroupMemberRecord: pass
    def reject_member(self, actor_did, group_did, member_did, reason=None) -> GroupMemberRecord: pass
    def accept_invite(self, member_did, group_did, invite_id=None) -> GroupMemberRecord: pass
    def remove_member(self, actor_did, group_did, member_did) -> GroupMemberRecord: pass
    def update_member_role(self, actor_did, group_did, member_did, role) -> GroupMemberRecord: pass
    def create_subgroup(self, actor_did, group_did, name, member_dids) -> GroupSubgroup: pass
    def update_subgroup(self, actor_did, group_did, subgroup_id, patch) -> GroupSubgroup: pass
    def list_subgroups(self, group_did) -> list[GroupSubgroup]: pass

    def is_group_did(self, did) -> bool: pass
    def check_group_access(self, group_did, actor_did, action) -> bool: pass
    def get_group_subscribers(self, group_did) -> list[DID]: pass
    def list_groups_by_member(self, member_did) -> list[GroupSummary]: pass
```

### 7.2 ContactMgr API 补充

```python
class ContactMgr:
    def get_group_profile(self, group_did) -> GroupProfile: pass
    def plan_group_delivery(self, group_did, msg_obj, context=None) -> list[DeliveryPlan]: pass
    def resolve_group_member_binding(self, member_did) -> list[AccountBinding]: pass
```

### 7.3 MessageCenter API 补充

```python
class MessageCenter:
    def dispatch_group(self, group_did, msg_obj, ingress_ctx=None): pass
    def post_group_send(self, group_did, author_did, content, send_ctx=None): pass
    def post_group_admin_request(self, group_did, admin_dids, operation) -> list[MsgRecord]: pass
    def post_group_member_notice(self, group_did, member_did, notice) -> MsgRecord: pass
    def get_group_inbox(self, group_did) -> MsgBoxHandle: pass
    def set_group_read_state(self, group_did, msg_id, reader_did, state): pass
```

这些 API 可以复用已有 `dispatch()` / `post_send()`，但建议在接口层保留 group 语义，避免调用方手工拼错 `from/source/to/kind`。

---

## 8. 安全与一致性要求

1. **写入权限必须由 host 校验**：self-host group 的消息写入、成员变更、配置变更都必须经过 host Zone。
2. **Active 成员必须有签名**：没有有效 `GroupMemberProof` 的 DID 不能成为 `Active` member，也不能被 subgroup 引用。
3. **同 Zone 自动签名仍需留痕**：自动构造成员签名只是交互优化，不是跳过双向确认。
4. **管理员确认进入个人 inbox**：需要管理员批准的操作必须进入 owner/admin 的个人 `INBOX`，不能只依赖 group 事件流。
5. **移除结果进入个人 inbox**：成员被移除时，必须向该成员个人 `INBOX` 发送合理通知。
6. **成员快照不可回改**：历史消息的投递范围按写入时成员状态决定；成员退出不删除既有历史。
7. **作者与 group 分离**：`from=group_did` 表示群时间线主体，`source=author_did` 表示真实作者。
8. **外部账号不等于成员身份**：Telegram/Email 账号必须先通过 ContactMgr 解析到 DID，再参与 group 权限判断。
9. **删除策略分层**：删除 group、退出 group、删除本地会话历史是不同操作；不能用一个 delete 标志表达。
10. **幂等写入**：同一个 `MsgObjectId` 重复 dispatch/post_send 不应产生重复 group inbox 或 reader inbox 记录。
11. **审计可见**：成员和权限变化必须有事件记录，至少 host owner 可查看。

---

## 9. 第一版落地范围

### 9.1 必须实现

1. 创建 self-host group，生成并保存 `GroupDoc`。
2. 保存成员表和 `GroupMemberProof`，支持 owner/admin/member 三类角色。
3. MessageCenter 支持 `from=group, source=author` 的群消息规范化。
4. MessageCenter 写入 `GROUP_INBOX` 和本地 reader inbox/read receipt。
5. ContactMgr 能识别 group DID，并为群成员规划投递。
6. MessageCenter 能把管理员待确认操作和成员移除通知写入个人 `INBOX`。
7. 支持 `PostPolicy::AdminOnly` 的纯通知群。
8. 支持 parent group 内的 subgroup。
9. UI 能展示 hosted/joined、member count、DID、owner、messageable。
10. tunnel 投递结果能回写到 `TUNNEL_OUTBOX` record。

### 9.2 可以后置

1. 多 host / federation 共识。
2. 多级复杂入群审批流。
3. 群公告、群文件、群内 topic/channel。
4. 全量历史同步到远端成员 Zone。
5. group 级别复杂 RBAC Schema。
6. 外部平台群到 self-host group 的自动迁移。
7. subgroup 升级为独立 Group DID 的自动迁移。

---

## 10. 与现有文档的联动点

需要在实现时同步检查：

1. `Message Center.md`：群消息 `dispatch/post_send` 的伪代码应与最终 `from/source/to` 规范一致。
2. `Contact Mgr.md`：`Contact.groups` 要明确为联系人集合，不等同于 messageable group。
3. MessageHub UI model：如果当前 mock 仍使用 `from=author, to=[group]` 表达群消息，需要在协议层或 data adapter 中迁移到 `from=group, source=author`。
4. `ndn-lib::MsgObject`：确认 `source` 字段和 `kind=group_msg` 的序列化字段已经存在；如果协议对象缺字段，必须先改协议和前后端镜像类型。
